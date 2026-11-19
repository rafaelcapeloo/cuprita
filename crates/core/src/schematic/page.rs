//! A sheet's page settings, as KiCad's Page Settings sets them in the sheet's text: the paper it
//! is drawn on, lying or standing, and the lines of its title block.

use super::Project;
use super::edit::{Layout, Patch};
use crate::sexpr::{Cursor, Result, escape, unescape};
use crate::tr::{t, tr, trf};

/// The papers KiCad names, smallest first, then the American ones.
pub const PAPERS: [&str; 14] = ["A5", "A4", "A3", "A2", "A1", "A0", "A", "B", "C", "D", "E", "USLetter", "USLegal", "USLedger"];

/// The title block's lines, in the order KiCad writes them.
pub const LINES: [&str; 8] = ["title", "date", "rev", "company", "comment 1", "comment 2", "comment 3", "comment 4"];

/// A sheet's page as its file says: its paper's name (`User` for a size of its own), whether it
/// stands (portrait), and its title block's lines (`LINES`' names; a comment by its number).
#[derive(Debug, Default, PartialEq)]
pub struct PageSetup {
    pub paper: String,
    pub portrait: bool,
    pub lines: Vec<(String, String)>,
}

impl Project {
    /// Page `page`'s paper and title block, as its sheet's text has them.
    pub fn page_setup(&self, page: usize) -> std::result::Result<PageSetup, String> {
        let (file, src) = self.page_text(page)?;
        let layout = Layout::read(src).map_err(|e| format!("{file}: {}", e.msg))?;
        let mut setup = PageSetup { paper: "A4".into(), ..PageSetup::default() };
        if let Some(paper) = &layout.paper {
            let c = &mut Cursor::new(&src[paper.clone()]);
            let named = (|| -> Result<(String, bool)> {
                c.open("paper")?;
                let name = unescape(c.atom()?);
                let mut portrait = false;
                while let Some(more) = c.opt_atom()? {
                    portrait |= more == "portrait";
                }
                Ok((name, portrait))
            })();
            (setup.paper, setup.portrait) = named.map_err(|e| format!("{file}: {}", e.msg))?;
        }
        if let Some(title) = &layout.title {
            setup.lines = lines(&src[title.clone()]).map_err(|e| format!("{file}: {}", e.msg))?;
        }
        Ok(setup)
    }

    /// Puts page `page` on paper `name` (one of `PAPERS`), standing with `portrait`. Returns the
    /// patch that undoes it.
    pub fn set_paper(&mut self, page: usize, name: &str, portrait: bool) -> std::result::Result<Patch, String> {
        if !PAPERS.contains(&name) {
            return Err(trf("no paper {0}: KiCad's are {1}", &[&name, &PAPERS.join(", ")]));
        }
        let (file, src) = self.page_text(page)?;
        let layout = Layout::read(src).map_err(|e| format!("{file}: {}", e.msg))?;
        let paper = format!("(paper \"{name}\"{})", if portrait { " portrait" } else { "" });
        let edit = match &layout.paper {
            Some(old) => (old.clone(), paper),
            None => (layout.head..layout.head, format!("\n\t{paper}")),
        };
        let patch = Patch(vec![(file.to_string(), vec![edit])]);
        self.apply_readable(patch, t!("the paper wasn't set, as it would leave a sheet unreadable: {0}"))
    }

    /// Sets line `line` (one of `LINES`, or a comment up to 9) of page `page`'s title block to
    /// `value`; an empty one takes the line away, and the block with its last. Returns the patch
    /// that undoes it.
    pub fn set_title_line(&mut self, page: usize, line: &str, value: &str) -> std::result::Result<Patch, String> {
        let comment = line.strip_prefix("comment ").and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=9).contains(n));
        if !LINES[..4].contains(&line) && comment.is_none() {
            return Err(trf("no line {0} in a title block: it has {1}", &[&line, &LINES.join(", ")]));
        }
        let (file, src) = self.page_text(page)?;
        let layout = Layout::read(src).map_err(|e| format!("{file}: {}", e.msg))?;
        let mut all = match &layout.title {
            Some(title) => lines(&src[title.clone()]).map_err(|e| format!("{file}: {}", e.msg))?,
            None => vec![],
        };
        all.retain(|(name, _)| name != line);
        if !value.trim().is_empty() {
            all.push((line.to_string(), value.trim().to_string()));
        }
        // As KiCad orders them: title, date, rev, company, then the comments by number.
        let rank = |name: &str| {
            LINES[..4]
                .iter()
                .position(|l| *l == name)
                .unwrap_or_else(|| 4 + name.trim_start_matches("comment ").parse::<usize>().unwrap_or(0))
        };
        all.sort_by_key(|(name, _)| rank(name));
        let written: String = all.iter().map(|(name, text)| format!("\n\t\t({name} \"{}\")", escape(text))).collect();
        let block = format!("(title_block{written}\n\t)");
        let edit = match (&layout.title, all.is_empty()) {
            (Some(old), true) => (src[..old.start].trim_end().len()..old.end, String::new()),
            (Some(old), false) => (old.clone(), block),
            (None, true) => return Err(tr("the title block has no such line to take away").into()),
            (None, false) => {
                let after = layout.paper.as_ref().map_or(layout.head, |p| p.end);
                (after..after, format!("\n\t{block}"))
            }
        };
        let patch = Patch(vec![(file.to_string(), vec![edit])]);
        self.apply_readable(patch, t!("the title block wasn't set, as it would leave a sheet unreadable: {0}"))
    }
}

/// A `(title_block …)`'s lines: `title`, `date`, `rev`, `company`, `comment N`, each with its text.
fn lines(block: &str) -> Result<Vec<(String, String)>> {
    let (c, mut out) = (&mut Cursor::new(block), vec![]);
    c.open("title_block")?;
    while let Some(head) = c.child()? {
        let name = if head == "comment" { format!("comment {}", c.atom()?) } else { head.to_string() };
        out.push((name, unescape(c.opt_atom()?.unwrap_or_default())));
        c.close()?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(sheet: &str) -> Project {
        let sheet = sheet.to_string();
        Project::load("a.kicad_sch", "", |f| if f == "a.kicad_sch" { Ok(sheet.clone()) } else { Err(f.into()) }).unwrap()
    }

    #[test]
    fn sets_a_sheets_paper_and_title_block() {
        let src = "(kicad_sch\n\t(version 20250114)\n\t(generator \"eeschema\")\n\t(uuid \"r\")\n\t(paper \"A4\")\n\t(lib_symbols)\n)";
        let mut p = project(src);
        let text = |p: &Project| p.text("a.kicad_sch").unwrap().to_string();
        assert_eq!(p.page_setup(0).unwrap(), PageSetup { paper: "A4".into(), portrait: false, lines: vec![] });
        // A3 standing: the page is as wide as A3 is high.
        let paper = p.set_paper(0, "A3", true).unwrap();
        assert!(text(&p).contains("\n\t(paper \"A3\" portrait)\n\t(lib_symbols)"), "{}", text(&p));
        assert_eq!((p.pages[0].sheet.page.0, p.page_setup(0).unwrap().portrait), (11693 * 25_400, true));
        // The title block comes after the paper, its lines in KiCad's order however they are set.
        p.set_title_line(0, "rev", "B").unwrap();
        p.set_title_line(0, "comment 2", "second").unwrap();
        p.set_title_line(0, "title", "An \"amp\"").unwrap();
        let block = "\n\t(paper \"A3\" portrait)\n\t(title_block\n\t\t(title \"An \\\"amp\\\"\")\n\t\t(rev \"B\")\n\t\t(comment 2 \"second\")\n\t)\n\t(lib_symbols)";
        assert!(text(&p).contains(block), "{}", text(&p));
        let t = &p.pages[0].sheet.title;
        assert_eq!((t.title.as_str(), t.rev.as_str(), &t.comments[..]), ("An \"amp\"", "B", &["second".to_string()][..]));
        assert_eq!(p.page_setup(0).unwrap().lines.len(), 3);
        // A line emptied goes, and the block with its last; undone, the sheet is as it was.
        let rev = p.set_title_line(0, "rev", " ").unwrap();
        assert!(!text(&p).contains("(rev"));
        p.set_title_line(0, "title", "").unwrap();
        p.set_title_line(0, "comment 2", "").unwrap();
        assert!(text(&p).contains("\n\t(paper \"A3\" portrait)\n\t(lib_symbols)"), "{}", text(&p));
        p.apply(paper);
        assert_eq!(text(&p), src);
        drop(rev);
        // A sheet without a paper gets one after its head; a paper or a line KiCad has none of is refused.
        let mut bare = project("(kicad_sch (version 20211123) (uuid \"r\") (lib_symbols))");
        bare.set_paper(0, "USLetter", false).unwrap();
        bare.set_title_line(0, "company", "Cuprita").unwrap();
        let t = text(&bare);
        assert!(t.contains("(uuid \"r\")\n\t(paper \"USLetter\")\n\t(title_block\n\t\t(company \"Cuprita\")\n\t) (lib_symbols)"), "{t}");
        assert!(bare.set_paper(0, "A7", false).unwrap_err().starts_with("no paper A7"));
        assert!(bare.set_title_line(0, "author", "x").unwrap_err().starts_with("no line author"));
    }
}
