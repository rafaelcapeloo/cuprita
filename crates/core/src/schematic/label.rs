//! Net labels (local, global, hierarchical) and hierarchical sheets with their pins, drawn as
//! KiCad draws them: label outlines and text offsets measured from KiCad 10 plots.

use super::net::{Label, LabelKind};
use super::{Ink, LINE, Schematic, Sheet, stroke, text, upright};
use crate::board::{Geom, Nm, Pt, Text};
use crate::font;
use crate::sexpr::{Cursor, Result, unescape};

/// A label's or sheet pin's shape: which way its signal goes.
#[derive(Clone, Copy, PartialEq)]
enum Flow {
    Input,
    Output,
    Both,
    Passive,
}

fn flow(s: &str) -> Flow {
    match s {
        "input" => Flow::Input,
        "output" => Flow::Output,
        "bidirectional" | "tri_state" => Flow::Both,
        _ => Flow::Passive,
    }
}

/// Reads a `label`, `global_label` or `hierarchical_label` and draws it.
pub(super) fn label(c: &mut Cursor, kind: &str, s: &mut Schematic) -> Result<()> {
    let raw = c.atom()?;
    let mut keyword = "";
    let (ink, scope) = match kind {
        "global_label" => (Ink::GlobalLabel, LabelKind::Global),
        "hierarchical_label" => (Ink::HierLabel, LabelKind::Hier),
        _ => (Ink::Label, LabelKind::Local),
    };
    // shortcut: a global label's inter-sheet references (a property KiCad fills in) aren't shown.
    let t = text(c, raw, ink, |c, head| {
        if head == "shape" {
            keyword = c.atom()?;
        }
        Ok(())
    })?;
    let shape = flow(keyword);
    if t.text.is_empty() {
        return Ok(());
    }
    let (at, shapes, texts) = (t.at, s.shapes.len(), s.texts.len());
    match ink {
        Ink::Label => s.texts.push(local(t, s.style.text_offset)),
        Ink::GlobalLabel => global(s, t, shape),
        _ => flag(s, t, shape, ink),
    }
    // The box it is drawn in: its point, outline and text.
    let text: Vec<Pt> = s.texts[texts..].iter().flat_map(|t| <[Pt; 2]>::from(font::bounds(t))).collect();
    let outline = s.shapes[shapes..].iter().flat_map(|o| if let Geom::Poly(p) = &o.geom { &p[..] } else { &[] });
    let grow = |(lo, hi): (Pt, Pt), p: &Pt| (Pt { x: lo.x.min(p.x), y: lo.y.min(p.y) }, Pt { x: hi.x.max(p.x), y: hi.y.max(p.y) });
    let bounds = outline.chain(&text).fold((at, at), grow);
    s.wiring.labels.push(Label { name: unescape(raw), at, kind: scope, shape: keyword.into(), span: 0..0, bounds });
    Ok(())
}

/// A local label's text: above the wire (left of a vertical one), reading away from its point.
fn local(mut t: Text, ratio: f64) -> Text {
    let h = t.size.1;
    // Measured from KiCad 10 plots: the project's text offset ratio and an eighth of the height.
    let off = (ratio * h as f64).round() as Nm + h / 8;
    let (vertical, back) = (t.angle.rem_euclid(180.0) == 90.0, matches!(t.angle.rem_euclid(360.0) as i32, 180 | 270));
    t.at = if vertical { Pt { x: t.at.x - off, y: t.at.y } } else { Pt { x: t.at.x, y: t.at.y - off } };
    t.justify = (if back { 1 } else { -1 }, 1);
    upright(t)
}

/// Unit step along a label from its point, by its angle.
fn along(t: &Text) -> (Nm, Nm) {
    match t.angle.rem_euclid(360.0) as i32 {
        90 => (0, -1),
        180 => (-1, 0),
        270 => (0, 1),
        _ => (1, 0),
    }
}

/// `pts` in a label's frame (x along it from its point `o`, y across) on the sheet.
fn outline(o: Pt, u: (Nm, Nm), pts: &[(Nm, Nm)]) -> Geom {
    Geom::Poly(pts.iter().map(|&(x, y)| Pt { x: o.x + u.0 * x - u.1 * y, y: o.y + u.1 * x + u.0 * y }).collect())
}

/// `t` reading away from its point `o` along `u`, starting `at` along, centred across and moved
/// `down` towards its glyphs' feet.
fn beside(mut t: Text, o: Pt, u: (Nm, Nm), at: Nm, down: Nm) -> Text {
    let vertical = u.0 == 0;
    t.at = Pt { x: o.x + u.0 * at + if vertical { down } else { 0 }, y: o.y + u.1 * at + if vertical { 0 } else { down } };
    t.angle = if vertical { 90.0 } else { 0.0 };
    // Text reads +X or up the sheet: it starts at its anchor going that way, else ends there.
    t.justify = (if u == (1, 0) || u == (0, -1) { -1 } else { 1 }, 0);
    t
}

/// A global label: its text in a box, pointed at the ends its signal goes through.
fn global(s: &mut Schematic, t: Text, shape: Flow) {
    let (o, u, h) = (t.at, along(&t), t.size.1 as f64);
    // Measured from KiCad 10 plots: a margin of the label size ratio around the text's box (its
    // advance and 0.175 heights), an eighth-height pen; arrows as long as half the box is high.
    let (m, pen) = (s.style.label_size * h, h / 8.0);
    let half = h / 2.0 + m;
    let (y, x) = ((half + pen).round() as Nm, (font::width(&t) + 0.175 * h + 2.0 * m + pen).round() as Nm);
    let a = half.round() as Nm;
    let (start, end) = match shape {
        Flow::Input => (a, 0),
        Flow::Output => (0, a),
        Flow::Both => (a, a),
        Flow::Passive => (0, 0),
    };
    let far = start + x;
    s.shape(outline(o, u, &[(0, 0), (start, y), (far, y), (far + end, 0), (far, -y), (start, -y)]), LINE, false, Ink::GlobalLabel);
    // Past the arrow by three quarters of the height, and a fourteenth of the height below the
    // middle (measured).
    let lead = if start > 0 { 0.75 * h } else { 0.0 };
    s.texts.push(beside(t, o, u, (lead + m).round() as Nm, (h / 14.0).round() as Nm));
}

/// A hierarchical label or sheet pin: a flag one text height long, then its text.
fn flag(s: &mut Schematic, t: Text, shape: Flow, ink: Ink) {
    let (o, u, h) = (t.at, along(&t), t.size.1);
    let half = h / 2;
    let pts: &[(Nm, Nm)] = match shape {
        Flow::Input => &[(0, 0), (half, half), (h, half), (h, -half), (half, -half)],
        Flow::Output => &[(h, 0), (half, -half), (0, -half), (0, half), (half, half)],
        Flow::Both => &[(0, 0), (half, -half), (h, 0), (half, half)],
        Flow::Passive => &[(0, -half), (h, -half), (h, half), (0, half)],
    };
    s.shape(outline(o, u, pts), LINE, false, ink);
    let off = h + (s.style.text_offset * h as f64).round() as Nm;
    s.texts.push(beside(t, o, u, off, 0));
}

/// Reads a hierarchical sheet and draws its box, name, file and pins.
pub(super) fn sheet(c: &mut Cursor, s: &mut Schematic) -> Result<()> {
    let (mut at, mut size, mut width) = (Pt::default(), Pt::default(), 0);
    let (mut name, mut file, mut uuid, mut pins, mut numbers) = (String::new(), String::new(), String::new(), vec![], vec![]);
    while let Some(head) = c.child()? {
        match head {
            "at" => at = c.point()?,
            "uuid" => uuid = c.atom()?.into(),
            "size" => size = c.point()?,
            "stroke" => width = stroke(c)?,
            "property" => {
                let (key, raw) = (c.atom()?, c.atom()?);
                // KiCad 6 names them "Sheet name" and "Sheet file".
                let (ink, shown) = match key {
                    "Sheetname" | "Sheet name" => (Ink::SheetName, raw.to_string()),
                    "Sheetfile" | "Sheet file" => (Ink::SheetFile, format!("File: {raw}")),
                    _ => (Ink::Field, raw.to_string()),
                };
                match ink {
                    Ink::SheetName => name = unescape(raw),
                    Ink::SheetFile => file = unescape(raw),
                    _ => {}
                }
                let t = text(c, &shown, ink, |_, _| Ok(()))?;
                if !t.text.is_empty() {
                    s.texts.push(upright(t));
                }
            }
            "instances" => {
                while let Some(head) = c.child()? {
                    if head == "project" {
                        c.atom()?;
                        super::numbers(c, &mut numbers)?;
                    }
                    c.close()?;
                }
            }
            "pin" => {
                let raw = c.atom()?;
                // From the sheet's side: an input pin's flag points into the sheet, as a
                // hierarchical output's does, and its text reads inward.
                let shape = match flow(c.atom()?) {
                    Flow::Input => Flow::Output,
                    Flow::Output => Flow::Input,
                    other => other,
                };
                let mut t = text(c, raw, Ink::SheetPin, |_, _| Ok(()))?;
                pins.push((unescape(raw), t.at));
                t.angle += 180.0;
                flag(s, t, shape, Ink::SheetPin);
            }
            _ => {}
        }
        c.close()?;
    }
    let corner = Pt { x: at.x + size.x, y: at.y + size.y };
    let rect = vec![at, Pt { x: corner.x, y: at.y }, corner, Pt { x: at.x, y: corner.y }];
    s.shape(Geom::Poly(rect), if width > 0 { width } else { LINE }, false, Ink::Sheet);
    s.sheets.push(Sheet { name, file, bounds: (at, corner), uuid, pins, numbers });
    Ok(())
}
