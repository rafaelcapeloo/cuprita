//! Stroke text laid out like KiCad, drawn with the Hershey Roman Simplex glyphs.
//!
//! `rowmans.jhf` comes from https://github.com/kamalmostafa/hershey-fonts (commit 1356bf2).
//! Its distribution terms require these acknowledgements to travel with it:
//! - The Hershey Fonts were originally created by Dr. A. V. Hershey while working at the
//!   U. S. National Bureau of Standards.
//! - The format of the Font data in this distribution was originally created by
//!   James Hurt, Cognition, Inc., 900 Technology Park Drive, Billerica, MA 01821
//!   (mit-eddie!ci-dandelion!hurt).
//!
//! The layout constants were measured from KiCad 10 Gerber plots. Hershey advances match
//! KiCad's text widths within 0.5%; a few glyphs are shaped slightly differently.

use crate::board::{Nm, Pt, Text};
use std::sync::OnceLock;

/// Hershey units per text height: capitals span y = -12 (top) to 9 (baseline).
const UNITS: f64 = 21.0;

struct Glyph {
    advance: i8,
    /// Polylines in Hershey units from the cell's left edge and the baseline (y down).
    strokes: Vec<Vec<[i8; 2]>>,
}

/// Glyphs for ASCII 32..=127.
fn glyphs() -> &'static [Glyph] {
    static GLYPHS: OnceLock<Vec<Glyph>> = OnceLock::new();
    GLYPHS.get_or_init(|| {
        // Per line: glyph number (5 chars), pair count (3), then coordinate pairs as letters with
        // 'R' = 0. The first pair is the cell's left and right edge; " R" lifts the pen.
        let glyph = |line: &str| {
            let b = line.as_bytes();
            let v = |i: usize| (b[i] as i32 - b'R' as i32) as i8;
            let mut strokes = vec![vec![]];
            for i in (10..b.len()).step_by(2) {
                match b[i] {
                    b' ' => strokes.push(vec![]),
                    _ => strokes.last_mut().unwrap().push([v(i) - v(8), v(i + 1) - 9]),
                }
            }
            Glyph { advance: v(9) - v(8), strokes }
        };
        include_str!("rowmans.jhf").lines().map(glyph).collect()
    })
}

fn glyph(ch: char) -> &'static Glyph {
    let all = glyphs();
    // shortcut: characters outside ASCII draw as '?'; add glyphs when boards need µ, Ω, ±, °.
    all.get((ch as usize).wrapping_sub(32)).unwrap_or(&all[b'?' as usize - 32])
}

/// A line's characters without KiCad's markup, each with whether an overbar runs over it:
/// `~{…}` draws one; `_{…}` and `^{…}` (subscript, superscript) show inline.
// shortcut: subscripts and superscripts keep the line's size and baseline; shrink and shift
// them when a board or sheet needs it.
fn marked(line: &str) -> Vec<(char, bool)> {
    let (mut out, mut open) = (vec![], vec![]);
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, '~' | '_' | '^') && chars.peek() == Some(&'{') {
            chars.next();
            open.push(ch == '~');
        } else if ch == '}' && !open.is_empty() {
            open.pop();
        } else {
            out.push((ch, open.contains(&true)));
        }
    }
    out
}

/// How far a line of text of glyph width `w` advances.
fn advance(line: &[(char, bool)], w: f64) -> f64 {
    line.iter().map(|&(ch, _)| glyph(ch).advance as f64 * w / UNITS).sum()
}

/// The advance of `t`'s widest line, in nm.
pub fn width(t: &Text) -> f64 {
    t.text.split('\n').map(|l| advance(&marked(l), t.size.0 as f64)).fold(0.0, f64::max)
}

/// `t`'s text as KiCad lays it out (without its final line break), and the height of its box:
/// lines 1.61 heights apart, 1.17 heights per line.
fn lines(t: &Text) -> (&str, f64) {
    let text = t.text.strip_suffix('\n').unwrap_or(&t.text);
    (text, (1.17 + (text.split('\n').count() - 1) as f64 * 1.61) * t.size.1 as f64)
}

/// Takes a point `lx` along `t`'s lines from its anchor and `ly` down them to board coordinates.
fn frame(t: &Text) -> impl Fn(f64, f64) -> Pt + '_ {
    let (s, c) = t.angle.to_radians().sin_cos();
    move |lx, ly| {
        let lx = if t.mirror { -lx } else { lx };
        Pt { x: t.at.x + (lx * c + ly * s).round() as Nm, y: t.at.y + (ly * c - lx * s).round() as Nm }
    }
}

/// The box around `t`'s lines as `strokes` lays them out: its widest advance by its height.
pub fn bounds(t: &Text) -> (Pt, Pt) {
    let (w, h) = (width(t), lines(t).1);
    let (x, y) = (-(1.0 + t.justify.0 as f64) / 2.0 * w, -(1.0 + t.justify.1 as f64) / 2.0 * h);
    let place = frame(t);
    let c = [(x, y), (x + w, y + h), (x, y + h), (x + w, y)].map(|(lx, ly)| place(lx, ly));
    let lo = Pt { x: c.iter().map(|p| p.x).min().unwrap(), y: c.iter().map(|p| p.y).min().unwrap() };
    (lo, Pt { x: c.iter().map(|p| p.x).max().unwrap(), y: c.iter().map(|p| p.y).max().unwrap() })
}

/// Calls `f` with every stroke of `t` as a polyline in board coordinates.
pub fn strokes(t: &Text, mut f: impl FnMut(&[Pt])) {
    let (w, h, pen) = (t.size.0 as f64, t.size.1 as f64, t.thickness as f64);
    let (sx, sy) = (w / UNITS, h / UNITS);
    // KiCad's text box has a pen-dependent margin.
    let ((text, height), pitch, pad) = (lines(t), 1.61 * h, 0.6579 * pen);
    let mut base = h * 22.0 / UNITS - 0.052 * pen - (1.0 + t.justify.1 as f64) / 2.0 * height;
    let place = frame(t);
    let mut pts = vec![];
    for line in text.split('\n') {
        let line = marked(line);
        let width = advance(&line, w);
        let mut x = pad - (1.0 + t.justify.0 as f64) / 2.0 * (width + 2.0 * pad);
        // An overbar runs 1.23 heights and a stroke unit over the baseline, a tenth of the height
        // in from its ends (measured from KiCad 10 plots).
        let (bar_y, mut bar) = (base - h * (1.23 + 1.0 / UNITS), None);
        for (i, &(ch, over)) in line.iter().enumerate() {
            bar = bar.or(over.then_some(x));
            let g = glyph(ch);
            for stroke in g.strokes.iter().filter(|s| !s.is_empty()) {
                pts.clear();
                pts.extend(stroke.iter().map(|&[gx, gy]| place(x + gx as f64 * sx, base + gy as f64 * sy)));
                f(&pts);
            }
            x += g.advance as f64 * sx;
            if let Some(x0) = bar.filter(|_| !line.get(i + 1).is_some_and(|n| n.1)) {
                f(&[place(x0 + h / 10.0, bar_y), place(x - h / 10.0, bar_y)]);
                bar = None;
            }
        }
        base += pitch;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MM: Nm = 1_000_000;

    fn text(s: &str, at: Pt, angle: f64, justify: (i8, i8), mirror: bool) -> Text {
        Text { text: s.into(), at, angle, size: (10 * MM, 10 * MM), thickness: MM, justify, mirror, ..Text::default() }
    }

    fn all_pts(t: &Text) -> Vec<Pt> {
        let mut v = vec![];
        strokes(t, |p| v.extend_from_slice(p));
        v
    }

    #[test]
    fn glyph_table_covers_ascii() {
        let g = glyphs();
        assert_eq!(g.len(), 96);
        assert_eq!((g[b'H' as usize - 32].advance, g[b'H' as usize - 32].strokes.len()), (22, 3));
    }

    #[test]
    fn layout_matches_kicad_plots() {
        // KiCad 10 put this text's cap top at y = 18.724 mm and its baseline at 28.724 mm.
        let p = all_pts(&text("H", Pt { x: 10 * MM, y: 30 * MM }, 0.0, (-1, 1), false));
        let (top, bottom) = (p.iter().map(|p| p.y).min().unwrap(), p.iter().map(|p| p.y).max().unwrap());
        assert!((top - 18_724_000).abs() < 1000 && (bottom - 28_724_000).abs() < 1000, "{top} {bottom}");
        // Left edge: pen margin 0.658 mm, then the H's stem 4 units into its cell.
        assert_eq!(p.iter().map(|p| p.x).min(), Some(10 * MM + 657_900 + 1_904_762));
        // Two centred lines: KiCad's baselines were 3.4755 mm above and 12.6239 mm below the anchor.
        // "I" is one stroke from the cap top down to the baseline.
        let p = all_pts(&text("I\nI", Pt::default(), 0.0, (0, 0), false));
        let (b0, b1) = (p[1].y, p[3].y);
        assert!((b0 + 3_475_530).abs() < 1000 && (b1 - 12_623_910).abs() < 1000, "{b0} {b1}");
    }

    #[test]
    fn bounds_hold_the_strokes() {
        for (angle, justify) in [(0.0, (-1, 1)), (90.0, (1, 0)), (270.0, (0, -1))] {
            let t = text("HE\nI", Pt { x: 5 * MM, y: 7 * MM }, angle, justify, false);
            let ((lo, hi), pen) = (bounds(&t), t.thickness);
            let inside = |p: &Pt| (lo.x - pen..=hi.x + pen).contains(&p.x) && (lo.y - pen..=hi.y + pen).contains(&p.y);
            assert!(all_pts(&t).iter().all(inside), "{angle} {justify:?}: {lo:?} {hi:?}");
        }
    }

    #[test]
    fn overbars_markup_like_kicad() {
        // KiCad 10 drew the note "~{HE}H" (2.54 mm, left bottom at 195, 270) with its overbar
        // from x 195.3542 to 199.8053 at y 266.186, over "HE" alone; notes sit 0.25 mm up there.
        let t = Text {
            text: "~{HE}H".into(),
            at: Pt { x: 195 * MM, y: 270 * MM },
            size: (2_540_000, 2_540_000),
            thickness: 152_400,
            justify: (-1, 1),
            ..Text::default()
        };
        let mut lines = vec![];
        strokes(&t, |p| lines.push(p.to_vec()));
        let bar = lines.iter().find(|p| p.len() == 2 && p[0].y == p[1].y && p[0].y < 267 * MM).unwrap();
        assert!(
            (bar[0].x - 195_354_200).abs() < 1000 && (bar[1].x - 199_805_300).abs() < 1000 && (bar[0].y - 266_436_000).abs() < 1000,
            "{bar:?}"
        );
        assert_eq!(marked("a_{1}^{2}~{b}"), [('a', false), ('1', false), ('2', false), ('b', true)]);
        assert_eq!(width(&t), width(&Text { text: "HEH".into(), ..t.clone() }));
    }

    #[test]
    fn mirror_then_rotate() {
        // Mirrored, the text grows towards -x; turned 90°, the text's up points to -x on the
        // board and its -x to +y, as KiCad plots back-side text.
        let p = all_pts(&text("I", Pt::default(), 90.0, (-1, 0), true));
        let (top, bottom) = (p[0], p[1]);
        assert!(top.x < bottom.x && top.y == bottom.y && bottom.y > 0, "{p:?}");
    }
}
