//! Fields placed beside their symbol, as KiCad's Autoplace fields does: the visible ones stacked
//! and reading across the sheet, where they meet nothing else: on a side without pins (right,
//! top, bottom, left), else past the pins on the top or bottom, else on the right or left, each
//! nearest first, a step further out at a time; else on the first of those sides. Each goes on the
//! grid point nearest its place.

use super::edit::{Patch, on_grid};
use super::symbol::{shown, turn};
use super::{Item, Project, Schematic};
use crate::board::{Nm, Pt, Text};
use crate::font;
use crate::save::Edit;
use crate::sexpr::{Cursor, Result, unescape};
use crate::tr::t;
use crate::units::nm_to_mm;
use std::ops::Range;

/// From the body (its pins' ends) to the fields, and between their lines.
const GAP: Nm = 1_270_000;
/// Right, top, bottom and left: the order sides without pins are taken in.
const SIDES: [(Nm, Nm); 4] = [(1, 0), (0, -1), (0, 1), (-1, 0)];
/// How many steps of `GAP` further out a stack may go to find room.
const STEPS: Nm = 8;

impl Project {
    /// Places the visible fields of page `page`'s symbols beside them; power symbols' stay, as
    /// do fields marked Do not autoplace. Returns the patch that undoes it (none when nothing
    /// moves) and how many symbols' fields moved.
    pub fn tidy_fields(&mut self, page: usize) -> std::result::Result<(Patch, usize), String> {
        let (file, src) = self.page_text(page)?;
        let sheet = &self.pages[page].sheet;
        let (mut edits, mut placed, mut moved) = (vec![], vec![], 0);
        for (i, s) in sheet.symbols.iter().enumerate().filter(|(_, s)| s.power.is_none()) {
            let to = beside(sheet, src, i, sheet, Some(Item::Symbol(i)), &mut placed).map_err(|e| format!("{file}: {e}"))?;
            // Only what changes: fields already in place stay as written.
            let at = |(r, text): Edit| (r.start + s.span.start..r.end + s.span.start, text);
            let changed: Vec<Edit> = to.into_iter().map(at).filter(|(r, text)| src[r.clone()] != *text).collect();
            moved += usize::from(!changed.is_empty());
            edits.extend(changed);
        }
        if edits.is_empty() {
            return Ok((Patch::default(), 0));
        }
        let patch = Patch(vec![(file.to_string(), edits)]);
        Ok((self.apply_readable(patch, t!("the fields weren't placed, as it would leave a sheet unreadable: {0}"))?, moved))
    }
}

/// The edits to the text of symbol `i` of `drawn` (in `src`), from where it starts, placing its
/// visible fields beside it clear of `sheet`'s items but `me` and of the boxes `placed`, which
/// gets theirs.
pub(super) fn beside(
    drawn: &Schematic,
    src: &str,
    i: usize,
    sheet: &Schematic,
    me: Option<Item>,
    placed: &mut Vec<(Pt, Pt)>,
) -> std::result::Result<Vec<Edit>, String> {
    let s = &drawn.symbols[i];
    let (angle, mirror, fields) = read(&src[s.span.clone()]).map_err(|e| e.msg.to_string())?;
    let m = turn(angle, mirror);
    // Each field shown, as it reads on the sheet.
    let shows: Vec<(&Field, Text)> = (fields.iter())
        .filter(|f| !f.hidden && !f.fixed && !f.value.is_empty() && f.value != "~")
        .map(|f| {
            let value = if f.name == "Reference" { s.reference.clone() } else { f.value.clone() };
            let text = if f.show_name { format!("{}: {value}", f.name) } else { value };
            (f, Text { text, size: (f.size, f.size), ..Text::default() })
        })
        .collect();
    let Some(h) = shows.iter().map(|(f, _)| f.size).max() else { return Ok(vec![]) };
    let (w, n) = (shows.iter().map(|(_, t)| font::width(t) as Nm).max().unwrap_or(0), shows.len() as Nm);
    let tall = (n - 1) * (h + GAP) + h;
    let ((lo, hi), pins) = (s.bounds, &drawn.wiring.pins);
    let c = Pt { x: (lo.x + hi.x) / 2, y: (lo.y + hi.y) / 2 };
    // The stack's box on side `d`, `off` further out.
    let stack = |d: (Nm, Nm), off: Nm| {
        let g = GAP + off;
        match d {
            (1, 0) => (Pt { x: hi.x + g, y: c.y - tall / 2 }, Pt { x: hi.x + g + w, y: c.y + tall / 2 }),
            (-1, 0) => (Pt { x: lo.x - g - w, y: c.y - tall / 2 }, Pt { x: lo.x - g, y: c.y + tall / 2 }),
            (0, -1) => (Pt { x: c.x - w / 2, y: lo.y - g - tall }, Pt { x: c.x + w / 2, y: lo.y - g }),
            _ => (Pt { x: c.x - w / 2, y: hi.y + g }, Pt { x: c.x + w / 2, y: hi.y + g + tall }),
        }
    };
    let meets = |a: (Pt, Pt), b: (Pt, Pt)| a.0.x <= b.1.x && a.1.x >= b.0.x && a.0.y <= b.1.y && a.1.y >= b.0.y;
    let clear = |b: (Pt, Pt)| !placed.iter().any(|&p| meets(p, b)) && sheet.items_in(b.0, b.1, true).iter().all(|&it| Some(it) == me);
    let has_pins = |d: &(Nm, Nm)| pins.iter().any(|q| q.symbol == i && !q.hidden && (q.out.0.signum(), q.out.1.signum()) == *d);
    let with = |sides: [(Nm, Nm); 2]| sides.into_iter().filter(has_pins).collect::<Vec<_>>();
    let groups = [SIDES.into_iter().filter(|d| !has_pins(d)).collect(), with([(0, -1), (0, 1)]), with([(1, 0), (-1, 0)])];
    let mut tries = groups.iter().flat_map(|g| (0..=STEPS).flat_map(move |k| g.iter().map(move |&d| (d, k * GAP))));
    let first = groups.iter().flatten().next().copied().unwrap_or((0, -1));
    let (side, off) = tries.find(|&(d, off)| clear(stack(d, off))).unwrap_or((first, 0));
    let (box_lo, box_hi) = stack(side, off);
    placed.push((box_lo, box_hi));
    let mut edits = vec![];
    for (k, (f, _)) in shows.iter().enumerate() {
        let y = box_lo.y + h / 2 + k as Nm * (h + GAP);
        let (x, want) = match side.0 {
            1 => (box_lo.x, (-1, 0)),
            -1 => (box_hi.x, (1, 0)),
            _ => (c.x, (0, 0)),
        };
        // Its angle and justification in the symbol's frame that show it so.
        let framed = [0.0, 90.0].into_iter().flat_map(|a| (-1..=1).flat_map(move |jx| (-1..=1).map(move |jy| (a, (jx, jy)))));
        let (a, j) = framed
            .into_iter()
            .find(|&(angle, justify)| {
                let t = shown(Text { angle, justify, ..Text::default() }, m);
                t.angle == 0.0 && t.justify == want
            })
            .unwrap_or((0.0, want));
        let at = on_grid(Pt { x, y });
        edits.push((f.at.clone(), format!("(at {} {} {a})", nm_to_mm(at.x), nm_to_mm(at.y))));
        let words: Vec<&str> = [(j.0, "left", "right"), (j.1, "top", "bottom")]
            .into_iter()
            .filter_map(|(v, less, more)| (v != 0).then_some(if v < 0 { less } else { more }))
            .collect();
        let justify = (!words.is_empty()).then(|| format!("(justify {})", words.join(" ")));
        match (&f.justify, justify) {
            (Some((_, span)), Some(new)) => edits.push((span.clone(), new)),
            (Some((space, span)), None) => edits.push((*space..span.end, String::new())),
            (None, Some(new)) => match f.effects_end {
                Some(end) => edits.push((end..end, format!(" {new}"))),
                None => edits.push((f.end..f.end, format!(" (effects {new})"))),
            },
            (None, None) => {}
        }
    }
    edits.sort_by_key(|e| e.0.start);
    Ok(edits)
}

/// A placed symbol's field, as its text has it (places from where the symbol's text starts).
struct Field {
    name: String,
    value: String,
    size: Nm,
    hidden: bool,
    show_name: bool,
    /// Marked Do not autoplace.
    fixed: bool,
    /// Its `(at …)`, its `(justify …)` (and where the space before it starts), where its
    /// `(effects …)` and it end (their `)`).
    at: Range<usize>,
    justify: Option<(usize, Range<usize>)>,
    effects_end: Option<usize>,
    end: usize,
}

/// A placed symbol's angle, mirror and fields, from its text `src` (its `(symbol …)` list).
fn read(src: &str) -> Result<(f64, &str, Vec<Field>)> {
    let c = &mut Cursor::new(src);
    c.open("symbol")?;
    let (mut angle, mut mirror, mut fields) = (0.0, "", vec![]);
    while let Some(head) = c.child()? {
        match head {
            "at" => {
                c.point()?;
                angle = c.opt_angle()?;
            }
            "mirror" => mirror = c.atom()?,
            "property" => {
                let mut f = field(c, src)?;
                c.close()?;
                f.end = c.span().end - 1;
                fields.push(f);
                continue;
            }
            _ => {}
        }
        c.close()?;
    }
    Ok((angle, mirror, fields))
}

/// A `(property …)` just entered, up to its end.
fn field(c: &mut Cursor, src: &str) -> Result<Field> {
    let (name, value) = (c.atom()?.to_string(), unescape(c.atom()?));
    let mut f = Field {
        name,
        value,
        size: 1_270_000,
        hidden: false,
        show_name: false,
        fixed: false,
        at: 0..0,
        justify: None,
        effects_end: None,
        end: 0,
    };
    // Where the list just entered starts: its `(`.
    let start = |c: &Cursor| src[..c.span().start].rfind('(').unwrap_or(0);
    loop {
        // KiCad 6 writes `hide` as a bare flag.
        while let Some(flag) = c.opt_atom()? {
            f.hidden |= flag == "hide";
        }
        let Some(head) = c.child()? else { break };
        let from = start(c);
        match head {
            "at" => {
                c.close()?;
                f.at = from..c.span().end;
                continue;
            }
            "effects" => {
                loop {
                    while let Some(flag) = c.opt_atom()? {
                        f.hidden |= flag == "hide";
                    }
                    let Some(head) = c.child()? else { break };
                    let from = start(c);
                    match head {
                        "font" => {
                            while let Some(head) = c.child()? {
                                if head == "size" {
                                    f.size = c.nm()?;
                                }
                                c.close()?;
                            }
                        }
                        "justify" => {
                            c.close()?;
                            f.justify = Some((src[..from].trim_end().len(), from..c.span().end));
                            continue;
                        }
                        "hide" => f.hidden = c.atom()? == "yes",
                        _ => {}
                    }
                    c.close()?;
                }
                c.close()?;
                f.effects_end = Some(c.span().end - 1);
                continue;
            }
            "hide" => f.hidden = c.atom()? == "yes",
            "show_name" => f.show_name = c.opt_atom()? != Some("no"),
            "do_not_autoplace" => f.fixed = c.opt_atom()? != Some("no"),
            _ => {}
        }
        c.close()?;
    }
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::LayerId;
    use crate::schematic::Ink;

    #[test]
    fn stacks_fields_on_a_side_without_pins_reading_across() {
        let src = r#"(kicad_sch (version 20211123) (uuid "r")
  (lib_symbols (symbol "Device:R" (property "Reference" "R" (at 2.032 0 90)) (property "Value" "R" (at 0 0 90))
    (symbol "R_1_1" (pin passive line (at 0 3.81 270) (length 1.27) (name "") (number "1"))
      (pin passive line (at 0 -3.81 90) (length 1.27) (name "") (number "2")))))
  (symbol (lib_id "Device:R") (at 10.16 10.16 0) (unit 1) (uuid "u1")
    (property "Reference" "R1" (at 12.192 10.16 90) (effects (font (size 1.27 1.27))))
    (property "Value" "10k" (at 10.16 10.16 90) (effects (font (size 1.27 1.27)))))
  (symbol (lib_id "Device:R") (at 30.48 10.16 90) (unit 1) (uuid "u2")
    (property "Reference" "R2" (at 30 8 0) (effects (font (size 1.27 1.27)) (justify left)))
    (property "Value" "1k" (at 30 12 0) (effects (font (size 1.27 1.27)) hide)))
  (label "X" (at 30.48 8.89 0) (effects (font (size 1.27 1.27))) (uuid "x"))
)"#;
        let mut p = Project::load("a.kicad_sch", "", |_| Ok(src.to_string())).unwrap();
        let (undo, moved) = p.tidy_fields(0).unwrap();
        let text = p.text("a.kicad_sch").unwrap().to_string();
        // Again, nothing moves.
        assert_eq!((moved, p.clone().tidy_fields(0).unwrap().1), (2, 0));
        // R1 stands: right of it, top down. R2 lies, pins left and right, label X above it:
        // below it, centred, its hidden value left.
        assert!(text.contains(r#""R1" (at 11.43 8.89 0) (effects (font (size 1.27 1.27)) (justify left))"#), "{text}");
        assert!(text.contains(r#""10k" (at 11.43 11.43 0) (effects (font (size 1.27 1.27)) (justify left))"#), "{text}");
        assert!(text.contains(r#""R2" (at 30.48 12.7 90) (effects (font (size 1.27 1.27))))"#), "{text}");
        assert!(text.contains(r#""1k" (at 30 12 0) (effects (font (size 1.27 1.27)) hide)"#), "{text}");
        // Both read across the sheet, as placed.
        let refs: Vec<_> = p.pages[0].sheet.texts.iter().filter(|t| t.layer == Ink::Reference as LayerId).collect();
        let at = |t: &Text| (t.at, t.angle, t.justify);
        let pt = |x: f64, y: f64| Pt { x: (x * 1e6) as Nm, y: (y * 1e6) as Nm };
        assert_eq!(refs.iter().map(|t| at(t)).collect::<Vec<_>>(), [(pt(11.43, 8.89), 0.0, (-1, 0)), (pt(30.48, 12.7), 0.0, (0, 0))]);
        p.apply(undo);
        assert_eq!(p.text("a.kicad_sch").unwrap(), src);
    }
}
