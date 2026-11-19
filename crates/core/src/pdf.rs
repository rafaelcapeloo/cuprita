//! PDF plots of a project's schematic: a page per sheet, on its paper with a frame and a title
//! block, in KiCad's colours. Lines, curves and fills are vectors and text is the strokes it is
//! drawn with, as on screen (so it can't be searched); PDF 1.4, its streams deflated.

use crate::board::{Geom, Nm, Pt, Shape, Text, arc_center};
use crate::font;
use crate::schematic::{Ink, Project};
use flate2::{Compression, write::ZlibEncoder};
use std::f64::consts::{FRAC_PI_2, TAU};
use std::fmt::Write as _;
use std::io::Write as _;

/// PDF points (1/72 in) in a nanometre.
const POINT: f64 = 72.0 / 25.4e6;

impl Project {
    /// Its schematic as a PDF titled `title`: a page per sheet, in KiCad's order (by page
    /// number, the numbers first).
    pub fn pdf(&self, title: &str) -> Vec<u8> {
        let mut order: Vec<(String, usize)> = (0..self.pages.len()).map(|i| (self.number(i), i)).collect();
        order.sort_by_key(|(n, i)| (n.parse::<u64>().unwrap_or(u64::MAX), n.clone(), *i));
        let pages = order.iter().map(|(_, i)| {
            let (s, frame) = (&self.pages[*i].sheet, self.frame(*i));
            let mut c = Canvas::new(s.page.1);
            // Back to front, as the window draws them.
            for (k, ink) in Ink::ALL.iter().enumerate() {
                for sh in s.shapes.iter().chain(&frame.shapes).filter(|sh| sh.layer as usize == k) {
                    c.ink(ink.color());
                    c.shape(sh);
                }
                for t in s.texts.iter().chain(&frame.texts).filter(|t| t.layer as usize == k) {
                    c.ink(ink.color());
                    c.text(t);
                }
            }
            (s.page, c.out)
        });
        document(title, pages.collect())
    }
}

/// A page's drawing operators, in PDF points with Y up.
struct Canvas {
    out: String,
    /// The page's height: a sheet's Y runs down.
    height: Nm,
    /// The colour and line width set last.
    color: Option<u32>,
    width: Option<Nm>,
}

impl Canvas {
    fn new(height: Nm) -> Canvas {
        // Round ends and corners, as the window's strokes.
        Canvas { out: "1 J 1 j\n".into(), height, color: None, width: None }
    }

    /// Point `(x, y)` of the sheet (nm), as an operand.
    fn at(&mut self, (x, y): (f64, f64)) {
        let _ = write!(self.out, "{} {} ", num(x * POINT), num((self.height as f64 - y) * POINT));
    }

    fn ink(&mut self, rgb: u32) {
        if self.color != Some(rgb) {
            let c = |shift: u32| num(f64::from(rgb >> shift & 0xFF) / 255.0);
            let (r, g, b) = (c(16), c(8), c(0));
            let _ = writeln!(self.out, "{r} {g} {b} RG {r} {g} {b} rg");
            self.color = Some(rgb);
        }
    }

    fn pen(&mut self, width: Nm) {
        if self.width != Some(width) {
            let _ = writeln!(self.out, "{} w", num(width as f64 * POINT));
            self.width = Some(width);
        }
    }

    /// A polyline through `pts`, as a new piece of the path.
    fn line(&mut self, pts: &[Pt]) {
        for (k, p) in pts.iter().enumerate() {
            self.at((p.x as f64, p.y as f64));
            self.out += if k == 0 { "m " } else { "l " };
        }
    }

    /// From where the path is, `sweep` radians round centre `c` at radius `r` from angle `from`:
    /// a Bézier a quarter turn at most.
    fn arc(&mut self, c: (f64, f64), r: f64, from: f64, sweep: f64) {
        let n = (sweep.abs() / FRAC_PI_2).ceil().max(1.0);
        let (step, mut a) = (sweep / n, from);
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        for _ in 0..n as usize {
            let b = a + step;
            let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
            self.at((c.0 + r * (ca - k * sa), c.1 + r * (sa + k * ca)));
            self.at((c.0 + r * (cb + k * sb), c.1 + r * (sb - k * cb)));
            self.at((c.0 + r * cb, c.1 + r * sb));
            self.out += "c ";
            a = b;
        }
    }

    fn shape(&mut self, sh: &Shape) {
        // Filled, outlined at its width, or both, as the window draws it; the pen set before
        // the path, which takes nothing else until it is painted.
        let closed = matches!(sh.geom, Geom::Circle { .. } | Geom::Poly(_));
        let outlined = !closed || sh.width > 0 || !sh.filled;
        if outlined {
            self.pen(sh.width);
        }
        match &sh.geom {
            Geom::Segment { a, b } => self.line(&[*a, *b]),
            Geom::Arc { start, mid, end } => {
                self.line(&[*start]);
                match arc_center(*start, *mid, *end) {
                    Some((cx, cy, r, from, sweep)) => self.arc((cx, cy), r, from, sweep),
                    None => self.line(&[*end]),
                }
            }
            Geom::Circle { center, radius } => {
                let (c, r) = ((center.x as f64, center.y as f64), *radius as f64);
                self.at((c.0 + r, c.1));
                self.out += "m ";
                self.arc(c, r, 0.0, TAU);
            }
            Geom::Poly(pts) => {
                self.line(pts);
                self.out += "h ";
            }
        }
        self.out += match (closed && sh.filled, outlined) {
            (true, true) => "B\n",
            (true, false) => "f\n",
            _ => "S\n",
        };
    }

    fn text(&mut self, t: &Text) {
        let mut strokes = vec![];
        font::strokes(t, |pts| strokes.push(pts.to_vec()));
        if !strokes.is_empty() {
            self.pen(t.thickness);
            strokes.iter().for_each(|pts| self.line(pts));
            self.out += "S\n";
        }
    }
}

/// `v` with at most two decimals (a point's 1/100 is 3.5 µm), no trailing zeros: `12.5`, `0`.
fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// A PDF of `pages` (each its size and drawing operators), titled `title`.
fn document(title: &str, pages: Vec<((Nm, Nm), String)>) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    // Where each object starts: 1 the catalog, 2 the pages, 3 the document's info, then each
    // page and its drawing.
    let mut at = vec![];
    let mut object = |out: &mut Vec<u8>, body: &[u8], stream: Option<&[u8]>| {
        at.push(out.len());
        out.extend(format!("{} 0 obj\n", at.len()).as_bytes());
        out.extend(body);
        if let Some(data) = stream {
            out.extend(b"\nstream\n");
            out.extend(data);
            out.extend(b"\nendstream");
        }
        out.extend(b"\nendobj\n");
    };
    let kids: Vec<String> = (0..pages.len()).map(|k| format!("{} 0 R", 4 + 2 * k)).collect();
    object(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>", None);
    object(&mut out, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).as_bytes(), None);
    // Its title in UTF-16, so any language's letters show.
    let title: String = std::iter::once(0xFEFF).chain(title.encode_utf16()).map(|u| format!("{u:04X}")).collect();
    object(&mut out, format!("<< /Title <{title}> /Creator (Cuprita) /Producer (Cuprita) >>").as_bytes(), None);
    for (k, ((w, h), ops)) in pages.iter().enumerate() {
        let size = format!("[0 0 {} {}]", num(*w as f64 * POINT), num(*h as f64 * POINT));
        let page = format!("<< /Type /Page /Parent 2 0 R /MediaBox {size} /Resources << >> /Contents {} 0 R >>", 5 + 2 * k);
        object(&mut out, page.as_bytes(), None);
        // Level 3: as small as the default within 3%, in half the time.
        let mut z = ZlibEncoder::new(vec![], Compression::new(3));
        let data = z.write_all(ops.as_bytes()).and_then(|()| z.finish()).expect("deflating into memory");
        object(&mut out, format!("<< /Length {} /Filter /FlateDecode >>", data.len()).as_bytes(), Some(&data));
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", at.len() + 1).as_bytes());
    for o in &at {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n", at.len() + 1).as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn plots_each_sheet_on_its_paper_with_its_title_block() {
        let root = r#"(kicad_sch (version 20250114) (uuid "r") (paper "A3") (title_block (title "Glow") (rev "B"))
              (wire (pts (xy 10 10) (xy 20 10)) (stroke (width 0)))
              (sheet (at 50 50) (size 20 10) (stroke (width 0)) (fill (color 0 0 0 0)) (uuid "s")
                (property "Sheetname" "Power" (at 50 49 0) (effects (font (size 1.27 1.27))))
                (property "Sheetfile" "power.kicad_sch" (at 50 61 0) (effects (font (size 1.27 1.27))))
                (instances (project "glow" (path "/r" (page "7"))))))"#;
        let power = r#"(kicad_sch (version 20250114) (paper "A4"))"#;
        let read = |root: &'static str| move |f: &str| Ok(if f == "power.kicad_sch" { power } else { root }.to_string());
        let p = Project::load("glow.kicad_sch", "", read(root)).unwrap();
        // Page numbers as KiCad 7 on keeps them, and as KiCad 6 did, all in the root's list.
        assert_eq!((p.number(0), p.number(1)), ("1".into(), "7".into()));
        let kicad6 = root
            .replace("(instances (project \"glow\" (path \"/r\" (page \"7\"))))", "")
            .replace("(paper \"A3\")", "(paper \"A3\") (sheet_instances (path \"/\" (page \"1\")) (path \"/s\" (page \"3\")))");
        let six = Project::load("glow.kicad_sch", "", read(Box::leak(kicad6.into_boxed_str()))).unwrap();
        assert_eq!((six.number(0), six.number(1)), ("1".into(), "3".into()));
        let pdf = p.pdf("Glöw");
        let find = |from: usize, what: &str| pdf[from..].windows(what.len()).position(|w| w == what.as_bytes()).map(|k| from + k);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.4\n") && text.ends_with("%%EOF\n"));
        // A3 then A4, landscape, in points.
        assert!(text.contains("/MediaBox [0 0 1190.52 841.9]") && text.contains("/MediaBox [0 0 841.9 595.3]"));
        assert!(text.contains("/Count 2") && text.contains("/Title <FEFF0047006C00F60077>"));
        // Each object where the cross-reference table says it starts.
        let xref = find(0, "startxref\n").unwrap() + 10;
        let xref: usize = String::from_utf8_lossy(&pdf[xref..]).lines().next().unwrap().parse().unwrap();
        let table = String::from_utf8_lossy(&pdf[xref..]).into_owned();
        for (k, entry) in table.lines().skip(3).take(7).enumerate() {
            let at: usize = entry[..10].parse().unwrap();
            assert!(pdf[at..].starts_with(format!("{} 0 obj", k + 1).as_bytes()), "object {}", k + 1);
        }
        // The root's drawing: the wire in KiCad's green, its sheet in its own colour, the frame.
        let start = find(0, "stream\n").unwrap() + 7;
        let mut ops = String::new();
        flate2::read::ZlibDecoder::new(&pdf[start..find(start, "\nendstream").unwrap()]).read_to_string(&mut ops).unwrap();
        let wire = "0 0.59 0 RG 0 0.59 0 rg\n28.35 813.55 m 56.69 813.55 l S\n";
        assert!(ops.contains(wire) && ops.contains("0.52 0 0 RG"), "{ops}");
        // On A3, the last zone's tick (7/8 along) and the title block's first cell (60 mm wide).
        assert!(ops.contains("1020.45 813.55 m 1020.45 807.88 l S") && ops.contains("1014.77 34.02 m 1014.77 51.02 l S"));
        // Every point drawn on the page.
        let mut numbers = vec![];
        for token in ops.split_whitespace() {
            match token {
                "m" | "l" | "c" => {
                    for xy in numbers.chunks(2) {
                        assert!((0.0..=1190.52).contains(&xy[0]) && (0.0..=841.9).contains(&xy[1]), "{xy:?} off the page");
                    }
                    numbers.clear();
                }
                t => match t.parse::<f64>() {
                    Ok(v) => numbers.push(v),
                    Err(_) => numbers.clear(),
                },
            }
        }
    }
}
