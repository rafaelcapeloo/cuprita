//! A page's drawing sheet, as KiCad's default one: the frame, its zones numbered along the top
//! and bottom and lettered down the sides, and the title block in its corner. The window and the
//! PDF both draw it, on `Ink::Frame`.

use super::{Ink, Project, TitleBlock};
use crate::board::{Geom, LayerId, Nm, Pt, Shape, Text, shape_box};
use crate::font;
use crate::tr::trf;

/// A title block row: its height, its text's size, and its cells (each its width and text).
type Row = (Nm, Nm, Vec<(Nm, String)>);

const MM: Nm = 1_000_000;
const LINE: Nm = 150_000;

/// A drawing sheet's lines and texts.
#[derive(Default)]
pub struct Frame {
    pub shapes: Vec<Shape>,
    pub texts: Vec<Text>,
}

impl Project {
    /// Page `i`'s drawing sheet: the frame round its paper and the title block, which names the
    /// sheet's path (`/Power/`), file and page (`3/11`).
    pub fn frame(&self, i: usize) -> Frame {
        let page = &self.pages[i];
        let path: String = page.path.iter().map(|name| format!("{name}/")).collect();
        let number = format!("{}/{}", self.number(i), self.pages.len());
        let s = &page.sheet;
        let ((w, h), (m, band)) = (s.page, (10 * MM, 2 * MM));
        let mut f = Frame::default();
        for lo in [m, m + band] {
            let (hi_x, hi_y) = (w - lo, h - lo);
            let corners = vec![Pt { x: lo, y: lo }, Pt { x: hi_x, y: lo }, Pt { x: hi_x, y: hi_y }, Pt { x: lo, y: hi_y }];
            f.shapes.push(Shape { geom: Geom::Poly(corners), width: LINE, filled: false, layer: Ink::Frame as LayerId });
        }
        // Zones about 50 mm long, ticked and named in the band on both sides.
        for across in [true, false] {
            let (length, other) = if across { (w - 2 * m, h) } else { (h - 2 * m, w) };
            let zones = (length as f64 / 50e6).round().max(1.0) as Nm;
            for k in 0..zones {
                // In i64: a zone's end, in nm, may pass i32 before it's divided.
                let end = |k: Nm| m + (i64::from(length) * i64::from(k) / i64::from(zones)) as Nm;
                let (from, to) = (end(k), end(k + 1));
                let name = if across { (k + 1).to_string() } else { char::from(b'A' + (k % 26) as u8).to_string() };
                for (side, tick) in [(m + band / 2, m), (other - m - band / 2, other - m - band)] {
                    let at = |along: Nm, off: Nm| if across { Pt { x: along, y: off } } else { Pt { x: off, y: along } };
                    f.say(&name, at((from + to) / 2, side), 1_300_000, (0, 0), to - from);
                    if k + 1 < zones {
                        f.rule(&[at(to, tick), at(to, tick + band)]);
                    }
                }
            }
        }
        // The title block's fields with the `${VARIABLES}` they name filled in (`Project::var`).
        let said = |text: &str| crate::parse::expand(text, |name| self.var(i, name));
        let t = &s.title;
        let title = TitleBlock {
            title: said(&t.title),
            date: said(&t.date),
            rev: said(&t.rev),
            company: said(&t.company),
            comments: t.comments.iter().map(|c| said(c)).collect(),
        };
        f.title_block(&title, &format!("/{path}"), &page.file, &number, Pt { x: w - m - band, y: h - m - band });
        f
    }
}

impl Project {
    /// Where on page `i` a box `size` fits, clear by 2.54 mm of what the sheet draws and of the
    /// title block, inside the frame: its top left corner on the grid, the first row by row from
    /// the top left. None when the page has no room.
    pub fn free_spot(&self, i: usize, size: (Nm, Nm)) -> Option<Pt> {
        const CELL: Nm = 2_540_000;
        let s = &self.pages[i].sheet;
        let (cols, rows) = ((s.page.0 / CELL) as usize, (s.page.1 / CELL) as usize);
        let mut taken = vec![false; cols * rows];
        let mut take = |(lo, hi): (Pt, Pt)| {
            let cell = |v: Nm, n: usize| (v.max(0) / CELL).min(n as Nm - 1) as usize;
            for y in cell(lo.y - CELL, rows)..=cell(hi.y + CELL, rows) {
                taken[y * cols + cell(lo.x - CELL, cols)..=y * cols + cell(hi.x + CELL, cols)].fill(true);
            }
        };
        let frame = self.frame(i);
        // The title block's lines, its borders round the page left out.
        let lines = frame.shapes.iter().filter(|sh| matches!(sh.geom, Geom::Segment { .. }));
        s.shapes.iter().chain(lines).for_each(|sh| take(shape_box(sh)));
        s.texts.iter().chain(&frame.texts).for_each(|t| take(font::bounds(t)));
        let (w, h) = (((size.0 + CELL - 1) / CELL) as usize, ((size.1 + CELL - 1) / CELL) as usize);
        // Inside the frame's inner line (12 mm in) by a cell.
        let first = (12 * MM / CELL) as usize + 2;
        let free = |x: usize, y: usize| (y..y + h).all(|y| !taken[y * cols + x..y * cols + x + w].contains(&true));
        (first..rows.saturating_sub(first + h))
            .flat_map(|y| (first..cols.saturating_sub(first + w)).map(move |x| (x, y)))
            .find(|&(x, y)| free(x, y))
            .map(|(x, y)| Pt { x: x as Nm * CELL, y: y as Nm * CELL })
    }
}

impl Frame {
    /// A line through `pts`.
    fn rule(&mut self, pts: &[Pt]) {
        for ab in pts.windows(2) {
            let geom = Geom::Segment { a: ab[0], b: ab[1] };
            self.shapes.push(Shape { geom, width: LINE, filled: false, layer: Ink::Frame as LayerId });
        }
    }

    /// `text` `size` high at `at` as `justify` places it (`Text`'s), narrowed to fit `room`.
    fn say(&mut self, text: &str, at: Pt, size: Nm, justify: (i8, i8), room: Nm) {
        let layer = Ink::Frame as LayerId;
        let mut t = Text { text: text.into(), at, size: (size, size), thickness: size / 8, justify, layer, ..Text::default() };
        let wide = font::width(&t);
        if wide > room as f64 {
            t.size.0 = (size as f64 * room as f64 / wide) as Nm;
        }
        self.texts.push(t);
    }

    /// The title block, its bottom right corner at `corner`: comments, company, title, then the
    /// sheet's path, revision and page, its file and date.
    fn title_block(&mut self, t: &TitleBlock, path: &str, file: &str, page: &str, corner: Pt) {
        let width = (110 * MM).min(corner.x - 12 * MM);
        let part = |mm: Nm| (i64::from(width) * i64::from(mm) / 110) as Nm;
        // Rows from the bottom.
        let rev = if t.rev.is_empty() { String::new() } else { trf("Rev. {0}", &[&t.rev]) };
        let mut rows: Vec<Row> = vec![
            (6 * MM, 1_500_000, vec![(part(60), file.into()), (part(50), t.date.clone())]),
            (6 * MM, 1_500_000, vec![(part(60), path.into()), (part(20), rev), (part(30), page.into())]),
            (9 * MM, 3 * MM, vec![(width, t.title.clone())]),
        ];
        rows.extend((!t.company.is_empty()).then(|| (6 * MM, 2 * MM, vec![(width, t.company.clone())])));
        rows.extend(t.comments.iter().map(|c| (5 * MM, 1_500_000, vec![(width, c.clone())])));
        let top = corner.y - rows.iter().map(|r| r.0).sum::<Nm>();
        let left = corner.x - width;
        self.rule(&[Pt { x: left, y: corner.y }, Pt { x: left, y: top }, Pt { x: corner.x, y: top }]);
        let mut y = corner.y;
        for (height, size, cells) in rows {
            let mut x = left;
            for (k, (cell, text)) in cells.iter().enumerate() {
                if k > 0 {
                    self.rule(&[Pt { x, y }, Pt { x, y: y - height }]);
                }
                if !text.is_empty() {
                    let pad = 1_500_000;
                    self.say(text, Pt { x: x + pad, y: y - height / 2 }, size, (-1, 0), cell - 2 * pad);
                }
                x += cell;
            }
            y -= height;
            self.rule(&[Pt { x: left, y }, Pt { x: corner.x, y }]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_each_page_its_path_file_and_number_and_finds_room_on_it() {
        let root = r#"(kicad_sch (version 20250114) (uuid "r") (paper "A4") (title_block (title "Glow"))
              (sheet (at 50 50) (size 20 10) (stroke (width 0)) (fill (color 0 0 0 0)) (uuid "s")
                (property "Sheetname" "Power" (at 50 49 0) (effects (font (size 1.27 1.27))))
                (property "Sheetfile" "power.kicad_sch" (at 50 61 0) (effects (font (size 1.27 1.27))))))"#;
        let read = |f: &str| Ok(if f == "power.kicad_sch" { r#"(kicad_sch (version 20250114) (paper "A4"))"# } else { root }.into());
        let p = Project::load("glow.kicad_sch", "", read).unwrap();
        let says = |i: usize, want: &[&str]| {
            let texts: Vec<String> = p.frame(i).texts.into_iter().map(|t| t.text).collect();
            assert!(want.iter().all(|w| texts.iter().any(|t| t == w)), "{texts:?}");
        };
        // A4's six zones across and four down.
        says(0, &["/", "glow.kicad_sch", "Glow", "1/2", "6", "D"]);
        says(1, &["/Power/", "power.kicad_sch", "2/2"]);
        // Room for a box: in the top left corner, else right of the sheet box (50 to 70 mm).
        let mm = |v: f64| (v * 1e6) as Nm;
        assert_eq!(p.free_spot(1, (mm(30.0), mm(20.0))), Some(Pt { x: mm(15.24), y: mm(15.24) }));
        let spot = p.free_spot(0, (mm(40.0), mm(40.0))).unwrap();
        assert!(spot.y == mm(15.24) && spot.x > mm(72.0), "{spot:?}");
        assert_eq!(p.free_spot(0, (mm(280.0), mm(20.0))), None);
    }

    #[test]
    fn fills_in_the_variables_title_blocks_and_texts_name() {
        // As hackrf-one's: the title block names the project's variables, a note the sheet's own.
        let root = r#"(kicad_sch (version 20250114) (uuid "r") (paper "A4")
              (title_block (title "${TITLE}") (date "${DATE}") (rev "${VERSION}") (company "${NONE} Lab"))
              (text "${TITLE} rev ${REVISION}, sheet ${#} of ${##}: ${SHEETNAME} in ${FILENAME} of ${PROJECTNAME}, ${MISSING}"
                (at 20 20 0) (effects (font (size 1.27 1.27))))
              (sheet (at 50 50) (size 20 10) (uuid "s") (property "Sheetname" "Power" (at 50 49 0))
                (property "Sheetfile" "power.kicad_sch" (at 50 61 0))))"#;
        let power = r#"(kicad_sch (version 20250114) (paper "A4") (title_block (title "Supplies"))
              (text "${SHEETPATH} ${TITLE} ${#}/${##}" (at 20 20 0) (effects (font (size 1.27 1.27)))))"#;
        let pro = r#"{"text_variables": {"TITLE": "HackRF One", "DATE": "2023-12-09", "VERSION": "10"}}"#;
        let read = |f: &str| Ok(if f == "power.kicad_sch" { power } else { root }.into());
        let p = Project::load("glow.kicad_sch", pro, read).unwrap();
        let framed: Vec<String> = p.frame(0).texts.into_iter().map(|t| t.text).collect();
        for want in ["HackRF One", "2023-12-09", "Rev. 10", "${NONE} Lab"] {
            assert!(framed.iter().any(|t| t == want), "{want}: {framed:?}");
        }
        let said = |i: usize| p.pages[i].sheet.texts.iter().map(|t| t.text.as_str()).find(|t| t.contains("of") || t.contains("/Power/"));
        assert_eq!(said(0), Some("HackRF One rev 10, sheet 1 of 2: Root in glow.kicad_sch of glow, ${MISSING}"));
        // A sheet's own title comes before the project's variable of that name.
        assert_eq!(said(1), Some("/Power/ Supplies 2/2"));
    }
}
