//! A project's schematic: its root sheet and every sheet under it, a page for each place a sheet
//! is used, drawn with that place's references.

use super::{Legacy, Schematic, Sheet, Style, erc};
use crate::json::Val;
use crate::parse::expand;
use crate::tr::trf;
use std::collections::HashMap;

/// A project's schematic, read from its root sheet down.
#[derive(Clone, Debug, Default)]
pub struct Project {
    /// The root first, then each sheet's pages after its parent's; a sheet used twice is two.
    pub pages: Vec<Page>,
    /// The sheets that couldn't be read: which, where, and what went wrong.
    pub errors: Vec<String>,
    /// The project's bus aliases (`.kicad_pro`): each one's members.
    // shortcut: KiCad 6 to 8 kept bus aliases in the sheets (`bus_alias`), which aren't read (no
    // corpus project has one); read them when a project does.
    pub(super) aliases: HashMap<String, Vec<String>>,
    /// The project's text variables (`.kicad_pro`), which texts and title blocks name as `${NAME}`.
    vars: HashMap<String, String>,
    pub(super) erc: erc::Settings,
    /// Its root sheet's file, its `.kicad_pro` text and how that draws labels, and KiCad 6's
    /// references from the root, to read its sheets again after an edit.
    pub(super) root: String,
    pub(super) pro: String,
    style: Style,
    legacy: Legacy,
    /// The text of each sheet file read (or why it couldn't be), by the name sheets give it.
    pub(super) texts: Texts,
}

pub(super) type Texts = HashMap<String, Result<String, String>>;

/// A sheet at one place in the hierarchy.
#[derive(Clone, Debug)]
pub struct Page {
    /// Sheet names from the root down; empty for the root.
    pub path: Vec<String>,
    /// Its file, as the sheet above names it.
    pub file: String,
    pub sheet: Schematic,
    pub parent: Option<usize>,
    /// The page each of `sheet.sheets` opens; `None` when it couldn't be read.
    pub children: Vec<Option<usize>>,
}

impl Project {
    /// Reads the root sheet `root` and the sheets under it, drawn with the project's
    /// `.kicad_pro` text `pro`; `read` gives a file's text by the name a sheet gives it.
    pub fn load(root: &str, pro: &str, read: impl Fn(&str) -> Result<String, String>) -> Result<Project, String> {
        let mut texts = Texts::new();
        let p = Project::read(root, pro, &mut texts, read)?;
        Ok(Project { texts, ..p })
    }

    /// Page `i`'s number as KiCad numbers it: its sheet box's (KiCad 7 on), else the root's list
    /// of them (KiCad 6), else its place in the project.
    pub fn number(&self, i: usize) -> String {
        let (page, root) = (&self.pages[i], &self.pages[0].sheet);
        let boxed = page.parent.and_then(|up| {
            let parent = &self.pages[up];
            let k = parent.children.iter().position(|&c| c == Some(i))?;
            parent.sheet.sheets[k].numbers.iter().find(|(path, _)| *path == parent.sheet.path)
        });
        // KiCad 6 names a sheet's instance from the root down, without the root's id.
        let trim = |path: &str| path.trim_end_matches('/').to_string();
        let path = if i == 0 { String::new() } else { trim(page.sheet.path.strip_prefix(&root.path).unwrap_or(&page.sheet.path)) };
        let listed = || root.numbers.iter().find(|(p, _)| trim(p) == path);
        boxed.or_else(listed).map_or_else(|| (i + 1).to_string(), |(_, n)| n.clone())
    }

    /// `load`, with the files `texts` holds (read with `read` and added when it lacks one); the
    /// project returned holds no texts.
    // shortcut: sheet files are named relative to the root's folder; KiCad also finds them
    // relative to the sheet above. Look there too when a project needs it.
    pub(super) fn read(root: &str, pro: &str, texts: &mut Texts, read: impl Fn(&str) -> Result<String, String>) -> Result<Project, String> {
        let json = Val::parse(pro);
        let style = Style::of(json.as_ref());
        let aliases = aliases(json.as_ref());
        let vars = json.as_ref().map(crate::parse::project_vars).unwrap_or_default();
        let parse = |src: &str, file: &str, path: &str, legacy: &Legacy| {
            Schematic::parse_at(src, style, path, legacy).map_err(|e| {
                let (line, col) = e.line_col(src);
                format!("{file}:{line}:{col}: {}", e.msg)
            })
        };
        let mut sheet = parse(text(texts, &read, root)?, root, "", &Legacy::new())?;
        let legacy = std::mem::take(&mut sheet.legacy);
        let root_page = Page { path: vec![], file: root.into(), sheet, parent: None, children: vec![] };
        let (erc, (root, pro)) = (erc::Settings::of(json.as_ref()), (root.into(), pro.into()));
        let mut p = Project { pages: vec![root_page], aliases, vars, erc, root, pro, style, ..Project::default() };
        let mut i = 0;
        while i < p.pages.len() {
            for s in p.pages[i].sheet.sheets.clone() {
                let path = format!("{}/{}", p.pages[i].sheet.path, s.uuid);
                let mut up = Some(i);
                let mut inside_itself = false;
                while let Some(k) = up {
                    inside_itself |= p.pages[k].file == s.file;
                    up = p.pages[k].parent;
                }
                let read = if inside_itself {
                    Err(trf("{0} holds itself, so it would never end; point the sheet at another file", &[&s.file]))
                } else if p.pages.len() >= 4096 {
                    // Thirty files each holding the next twice would be a thousand million pages.
                    Err(trf("{0}: the project shows more than 4096 sheets; Cuprita reads no more", &[&s.file]))
                } else {
                    text(texts, &read, &s.file).and_then(|src| parse(src, &s.file, &path, &legacy))
                };
                let child = match read {
                    Ok(sheet) => {
                        let path = p.pages[i].path.iter().cloned().chain([s.name.clone()]).collect();
                        p.pages.push(Page { path, file: s.file.clone(), sheet, parent: Some(i), children: vec![] });
                        Some(p.pages.len() - 1)
                    }
                    Err(e) => {
                        // The sheet's name in quotes, which the app finds its box's error by.
                        p.errors.push(trf("sheet \"{0}\" in {1}: {2}", &[&s.name, &p.pages[i].file, &e]));
                        None
                    }
                };
                p.pages[i].children.push(child);
            }
            i += 1;
        }
        (0..p.pages.len()).for_each(|i| p.fill(i));
        Ok(Project { legacy, ..p })
    }

    /// What `${name}` reads on page `i`, as KiCad fills it in: the sheet's title block (`TITLE`,
    /// `ISSUE_DATE`, `REVISION`, `COMPANY`, each with the project's variables it names in
    /// turn), its place (`#` of `##`, `SHEETNAME`, `SHEETPATH`, `FILENAME`, `PROJECTNAME`), else
    /// the project's variable.
    // shortcut: a symbol's own (`${VALUE}` in its fields), other parts' (`${R1:VALUE}`), the title
    // block's `COMMENT1`… and `CURRENT_DATE` stay as written; fill them in when a project shows one.
    pub(super) fn var(&self, i: usize, name: &str) -> Option<String> {
        let (page, title) = (&self.pages[i], &self.pages[i].sheet.title);
        let stem = |file: &str| file.rsplit('/').next().unwrap_or(file).trim_end_matches(".kicad_sch").to_string();
        let block = match name {
            "TITLE" => Some(&title.title),
            "ISSUE_DATE" => Some(&title.date),
            "REVISION" => Some(&title.rev),
            "COMPANY" => Some(&title.company),
            _ => None,
        };
        if let Some(text) = block {
            return Some(expand(text, |name| self.vars.get(name)));
        }
        match name {
            "#" => Some(self.number(i)),
            "##" => Some(self.pages.len().to_string()),
            "SHEETNAME" => Some(page.path.last().cloned().unwrap_or_else(|| "Root".into())),
            "SHEETPATH" => Some(page.path.iter().fold("/".to_string(), |path, name| path + name + "/")),
            "FILENAME" => Some(page.file.rsplit('/').next().unwrap_or(&page.file).to_string()),
            "PROJECTNAME" => Some(stem(&self.root)),
            _ => self.vars.get(name).cloned(),
        }
    }

    /// Fills in the `${VARIABLES}` page `i`'s texts name (`var`); those none knows stay as written.
    fn fill(&mut self, i: usize) {
        let named = |t: &&mut crate::board::Text| t.text.contains("${");
        let mut texts = std::mem::take(&mut self.pages[i].sheet.texts);
        for t in texts.iter_mut().filter(named) {
            t.text = expand(&t.text, |name| self.var(i, name));
        }
        self.pages[i].sheet.texts = texts;
    }

    /// Reads the pages of sheet files `files` again from the texts the project holds, after an
    /// edit to them; the whole project when one no longer reads or its sheet boxes open other
    /// pages (when the root sheet no longer reads, it keeps its pages and tells why in `errors`).
    pub(super) fn reread(&mut self, files: &[&str]) {
        let none = Legacy::new();
        // Pages go from the root down: a KiCad 6 root's references are read first.
        let edited: Vec<usize> = (0..self.pages.len()).filter(|&i| files.contains(&self.pages[i].file.as_str())).collect();
        // A box's place and pins don't change the pages under it.
        let same = |a: &[Sheet], b: &[Sheet]| {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (&a.name, &a.file, &a.uuid) == (&b.name, &b.file, &b.uuid))
        };
        for i in edited {
            let (page, path) = (&self.pages[i], if i == 0 { "" } else { &self.pages[i].sheet.path });
            let src = self.texts.get(&page.file).and_then(|t| t.as_deref().ok()).unwrap_or_default();
            match Schematic::parse_at(src, self.style, path, if i == 0 { &none } else { &self.legacy }) {
                Ok(mut s) if same(&s.sheets, &page.sheet.sheets) => {
                    if i == 0 {
                        self.legacy = std::mem::take(&mut s.legacy);
                    }
                    self.pages[i].sheet = s;
                    self.fill(i);
                }
                _ => return self.read_all(),
            }
        }
    }

    /// Reads the whole project again from the texts it holds.
    fn read_all(&mut self) {
        let mut texts = std::mem::take(&mut self.texts);
        match Project::read(&self.root, &self.pro, &mut texts, |f| Err(trf("{0} isn't read", &[&f]))) {
            Ok(p) => *self = Project { texts, ..p },
            Err(e) => (self.texts, self.errors) = (texts, vec![e]),
        }
    }
}

/// A file's text from `texts`, read with `read` the first time.
fn text<'t>(texts: &'t mut Texts, read: &impl Fn(&str) -> Result<String, String>, file: &str) -> Result<&'t str, String> {
    texts.entry(file.to_string()).or_insert_with(|| read(file)).as_deref().map_err(String::clone)
}

/// A `.kicad_pro`'s bus aliases (`schematic.bus_aliases`): each one's members.
fn aliases(pro: Option<&Val>) -> HashMap<String, Vec<String>> {
    let Some(Val::Obj(all)) = pro.and_then(|p| p.at(&["schematic", "bus_aliases"])) else { return HashMap::new() };
    let names = |members: &Val| match members {
        Val::Arr(m) => m.iter().filter_map(|m| if let Val::Str(m) = m { Some(m.clone()) } else { None }).collect(),
        _ => vec![],
    };
    all.iter().map(|(name, members)| (name.clone(), names(members))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project of files `(name, text)`; a missing one fails to read.
    fn project(files: &[(&str, &str)]) -> Project {
        let files: HashMap<_, _> = files.iter().map(|&(n, t)| (n.to_string(), t.to_string())).collect();
        Project::load("root.kicad_sch", "", |f| files.get(f).cloned().ok_or(format!("{f}: not found"))).unwrap()
    }

    fn sheet(uuid: &str, name: &str, file: &str) -> String {
        format!(r#"(sheet (at 0 0) (size 10 10) (uuid "{uuid}") (property "Sheetname" "{name}") (property "Sheetfile" "{file}"))"#)
    }

    const LIB: &str = r#"(lib_symbols (symbol "T:U" (symbol "U_1_1") (symbol "U_2_1")))"#;

    #[test]
    fn draws_each_place_a_sheet_is_used_with_its_references() {
        // KiCad 7+: each symbol lists its references by instance path.
        let a = format!(
            r#"(kicad_sch (uuid "a") {LIB} (symbol (lib_id "T:U") (at 0 0 0) (unit 1) (uuid "u")
              (property "Reference" "U?" (at 0 0 0))
              (instances (project "p" (path "/r/s1" (reference "U1") (unit 1)) (path "/r/s2" (reference "U7") (unit 2))))))"#
        );
        let root = format!(
            r#"(kicad_sch (uuid "r") {} {} {})"#,
            sheet("s1", "A", "a.kicad_sch"),
            sheet("s2", "B", "a.kicad_sch"),
            sheet("s3", "C", "c.kicad_sch")
        );
        let p = project(&[("root.kicad_sch", &root), ("a.kicad_sch", &a)]);
        let refs: Vec<_> =
            p.pages.iter().map(|g| (g.path.join("/"), g.sheet.symbols.iter().map(|s| s.reference.clone()).collect::<Vec<_>>())).collect();
        assert_eq!(refs, [("".into(), vec![]), ("A".into(), vec!["U1A".to_string()]), ("B".into(), vec!["U7B".to_string()])]);
        assert_eq!((p.pages[0].children.clone(), p.pages[2].parent), (vec![Some(1), Some(2), None], Some(0)));
        assert_eq!(p.errors, ["sheet \"C\" in root.kicad_sch: c.kicad_sch: not found"]);
        // KiCad 6: the root lists them, by the sheets' ids below it and the symbol's.
        let b = format!(
            r#"(kicad_sch (uuid "b") {LIB} (symbol (lib_id "T:U") (at 0 0 0) (unit 1) (uuid "u") (property "Reference" "U?" (at 0 0 0))) {})"#,
            sheet("s9", "Loop", "b.kicad_sch")
        );
        let root = format!(
            r#"(kicad_sch (uuid "r") {} (symbol_instances (path "/s1/u" (reference "U3") (unit 2))))"#,
            sheet("s1", "B", "b.kicad_sch")
        );
        let p = project(&[("root.kicad_sch", &root), ("b.kicad_sch", &b)]);
        assert_eq!(p.pages[1].sheet.symbols[0].reference, "U3B");
        assert!(p.errors[0].contains("b.kicad_sch holds itself"), "{:?}", p.errors);
    }
}
