//! Saving a part put on the other side (`Board::flip_footprint`): its text mirrored as KiCad 10's
//! Flip writes it, top to bottom about the part's anchor (`crates/core/save.md`).

use super::{Edit, KICAD8, angle_edit, read_at, xy};
use crate::board::{Board, Pt, place};
use crate::sexpr::{Cursor, Result};

/// A footprint's text (in `src`, of file `version`) flipped about its anchor `from`, as the file
/// has it, then turned `turn` degrees and moved to `to`.
pub(super) struct Flip<'a> {
    pub(super) b: &'a Board,
    pub(super) version: u32,
    pub(super) src: &'a str,
    pub(super) from: Pt,
    pub(super) to: Pt,
    pub(super) turn: f64,
}

/// What a list's points and angles belong to.
#[derive(Clone, Copy, PartialEq)]
enum Item {
    Pad,
    Text,
    /// Its corners are in board coordinates.
    Zone,
    /// Its own layer, a graphic.
    Other,
}

impl Flip<'_> {
    /// The edits flipping the footprint whose children `c` reads next; its `at` is the caller's.
    // shortcut: text boxes, dimensions and barcodes in a footprint stay as written; flip them too
    // when boards have them. KiCad flips no 3D model.
    pub(super) fn part(&self, c: &mut Cursor, edits: &mut Vec<Edit>) -> Result<()> {
        while let Some(head) = c.child()? {
            match head {
                "pad" => self.list(c, head, "", Item::Pad, edits)?,
                "fp_text" | "property" => self.text(c, edits)?,
                "zone" => self.list(c, head, "", Item::Zone, edits)?,
                "layer" | "fp_line" | "fp_arc" | "fp_circle" | "fp_rect" | "fp_poly" | "fp_curve" => {
                    self.list(c, head, "", Item::Other, edits)?
                }
                _ => {}
            }
            c.close()?;
        }
        Ok(())
    }

    /// The edits flipping the text `c` is in: mirrored when its layer goes to the other side, as
    /// KiCad mirrors none on a layer of neither side (`Dwgs.User`).
    fn text(&self, c: &mut Cursor, edits: &mut Vec<Edit>) -> Result<()> {
        let (mut sided, mut mirror) = (false, None);
        while let Some(head) = c.child()? {
            match head {
                "layer" => {
                    let n = edits.len();
                    self.list(c, head, "", Item::Text, edits)?;
                    sided = edits.len() > n;
                }
                "effects" => mirror = Some(self.mirror(c)?),
                _ => self.list(c, head, "", Item::Text, edits)?,
            }
            c.close()?;
        }
        edits.extend(mirror.filter(|_| sided));
        Ok(())
    }

    /// The edits flipping list `head` (in list `parent`) of `item` that `c` is in, and its lists.
    fn list(&self, c: &mut Cursor, head: &str, parent: &str, item: Item, edits: &mut Vec<Edit>) -> Result<()> {
        match head {
            "layer" | "layers" => {
                while let Some(name) = c.opt_atom()? {
                    if let Some(other) = self.b.flipped_name(name) {
                        let quoted = c.span().len() > name.len();
                        edits.push((c.span(), if quoted { format!("\"{other}\"") } else { other.to_string() }));
                    }
                }
            }
            // A chamfered corner at the top goes to the bottom.
            "chamfer" => {
                while let Some(corner) = c.opt_atom()? {
                    let other = match corner.split_once('_') {
                        Some(("top", side)) => format!("bottom_{side}"),
                        Some(("bottom", side)) => format!("top_{side}"),
                        _ => continue,
                    };
                    edits.push((c.span(), other));
                }
            }
            // A KiCad 5 arc's sweep turns the other way.
            "angle" if matches!(parent, "fp_arc" | "gr_arc") => {
                c.atom()?;
                edits.push((c.span(), negated(&self.src[c.span()])));
            }
            // Pad and text angles are absolute: a pad's is negated, a text reads backwards along
            // 180° less its angle (KiCad writes it as is, turned ones in [0°, 360°)). KiCad leaves
            // a 0° out, but for texts from KiCad 8 on.
            "at" if matches!(item, Item::Pad | Item::Text) => {
                let a = read_at(c)?;
                edits.push((a.y.clone(), negated(&self.src[a.y.clone()])));
                let deg = if item == Item::Pad { 360.0 - a.deg } else { 180.0 - a.deg } + self.turn;
                let deg = if item == Item::Pad || self.turn != 0.0 { deg.rem_euclid(360.0) } else { deg };
                let zero = (deg * 1e6).round() == 0.0 && (item == Item::Pad || self.version < KICAD8);
                edits.extend(if zero && !a.angle.is_empty() {
                    Some((a.y.end..a.angle.end, String::new()))
                } else {
                    angle_edit(a.angle, deg)
                });
            }
            "xy" | "start" | "mid" | "end" if item == Item::Zone => {
                let a = read_at(c)?;
                let local = Pt { x: a.at.x.saturating_sub(self.from.x), y: self.from.y.saturating_sub(a.at.y) };
                edits.push((a.xy, xy(place(local, self.to, self.turn))));
            }
            // Local points, a hole's offset, a trapezoid's slant: mirrored top to bottom.
            "xy" | "start" | "mid" | "end" | "center" | "offset" | "rect_delta" => {
                let a = read_at(c)?;
                edits.push((a.y.clone(), negated(&self.src[a.y])));
            }
            _ => {}
        }
        while let Some(child) = c.child()? {
            self.list(c, child, head, item, edits)?;
            c.close()?;
        }
        Ok(())
    }

    /// The edit toggling `mirror` in the `(effects …)` list `c` is in, as KiCad mirrors a flipped
    /// part's texts keeping their justification: `(justify left bottom)` gains it, a `(justify
    /// mirror)` goes, with the space before it, and comes back after the font, spaced as it is.
    fn mirror(&self, c: &mut Cursor) -> Result<Edit> {
        let (mut after, mut space) = (c.span().end, " ");
        loop {
            while c.opt_atom()?.is_some() {}
            let Some(head) = c.child()? else { break };
            let open = self.src[..c.span().start].rfind('(').unwrap_or_default();
            let gap = self.src[..open].trim_end().len();
            match head {
                "font" => {
                    c.close()?;
                    (after, space) = (c.span().end, &self.src[gap..open]);
                }
                "justify" => {
                    let (mut last, mut words, mut mirror) = (c.span().end, 0, None);
                    while let Some(word) = c.opt_atom()? {
                        if word == "mirror" {
                            mirror = Some(last..c.span().end);
                        }
                        (last, words) = (c.span().end, words + 1);
                    }
                    c.close()?;
                    return Ok(match mirror {
                        Some(_) if words == 1 => (gap..c.span().end, String::new()),
                        Some(word) => (word, String::new()),
                        None => (last..last, " mirror".into()),
                    });
                }
                _ => c.close()?,
            }
        }
        Ok((after..after, format!("{space}(justify mirror)")))
    }
}

/// A number as written, negated: `-0.735` and `0.735` swap, `0` stays.
fn negated(n: &str) -> String {
    match n.strip_prefix('-') {
        Some(positive) => positive.to_string(),
        None if n.parse::<f64>().is_ok_and(|v| v == 0.0) => n.to_string(),
        None => format!("-{n}"),
    }
}

#[cfg(test)]
mod tests {
    use crate::board::Board;

    /// `b` flipped twice saves as read, and so does its text saved flipped, read and flipped back.
    fn flips_back(b: &mut Board, src: &str, i: usize) {
        let out = b.write(src).unwrap();
        b.flip_footprint(i);
        assert_eq!(b.write(src).unwrap(), src);
        let mut back = Board::parse(&out).unwrap();
        back.flip_footprint(i);
        assert_eq!(back.write(&out).unwrap(), src);
    }

    #[test]
    fn writes_a_flipped_part_as_kicad_10_flips_it() {
        // glasgow-revD1's D80, shortened; KiCad 10's Flip writes it as `want` (skill kicad-check).
        let src = "(kicad_pcb\n\t(version 20260206)\n\t(layers\n\t\t(0 \"F.Cu\" signal)\n\t\t(2 \"B.Cu\" signal)\n\t\t(1 \"F.Mask\" user)
\t\t(3 \"B.Mask\" user)\n\t\t(5 \"F.SilkS\" user \"F.Silkscreen\")\n\t\t(7 \"B.SilkS\" user \"B.Silkscreen\")\n\t)
\t(footprint \"LED_SMD:LED_0603_1608Metric\"\n\t\t(layer \"F.Cu\")\n\t\t(at 204.8 90.2 90)\n\t\t(property \"Reference\" \"D80\"
\t\t\t(at 0 -1.43 90)\n\t\t\t(layer \"F.SilkS\")\n\t\t\t(effects\n\t\t\t\t(font\n\t\t\t\t\t(size 1 1)\n\t\t\t\t)\n\t\t\t)\n\t\t)
\t\t(fp_line\n\t\t\t(start 0.8 -0.735)\n\t\t\t(end -1.485 -0.735)\n\t\t\t(layer \"F.SilkS\")\n\t\t)
\t\t(pad \"1\" smd roundrect\n\t\t\t(at -0.7875 0 90)\n\t\t\t(size 0.875 0.95)\n\t\t\t(layers \"F.Cu\" \"F.Mask\")\n\t\t)\n\t)\n)";
        let want = src
            .replace("(layer \"F.Cu\")\n\t\t(at 204.8 90.2 90)", "(layer \"B.Cu\")\n\t\t(at 204.8 90.2 -90)")
            .replace("(at 0 -1.43 90)\n\t\t\t(layer \"F.SilkS\")", "(at 0 1.43 90)\n\t\t\t(layer \"B.SilkS\")")
            .replace("(size 1 1)\n\t\t\t\t)", "(size 1 1)\n\t\t\t\t)\n\t\t\t\t(justify mirror)")
            .replace(
                "(start 0.8 -0.735)\n\t\t\t(end -1.485 -0.735)\n\t\t\t(layer \"F.SilkS\")",
                "(start 0.8 0.735)\n\t\t\t(end -1.485 0.735)\n\t\t\t(layer \"B.SilkS\")",
            )
            .replace("(at -0.7875 0 90)", "(at -0.7875 0 270)")
            .replace("(layers \"F.Cu\" \"F.Mask\")", "(layers \"B.Cu\" \"B.Mask\")");
        let mut b = Board::parse(src).unwrap();
        b.flip_footprint(0);
        assert_eq!(b.write(src).unwrap(), want);
        let back = Board::parse(&want).unwrap();
        assert_eq!((&back.footprints, &back.pads, &back.shapes, &back.texts), (&b.footprints, &b.pads, &b.shapes, &b.texts));
        flips_back(&mut b, src, 0);
    }

    #[test]
    fn flips_every_kind_of_item_as_kicad_does() {
        // A KiCad 7 part on a 4-layer board: a text on a layer of neither side, justified text, a
        // KiCad 5 arc, a chamfered pad on inner copper, a custom pad with a hole off its middle,
        // a trapezoid, a keepout (board coordinates); then turned, after flipping.
        let src = r#"(kicad_pcb (version 20221018) (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (31 "B.Cu" signal)
    (36 "B.SilkS" user) (37 "F.SilkS" user) (38 "B.Mask" user) (39 "F.Mask" user) (40 "Dwgs.User" user))
  (footprint "U" (layer "F.Cu") (at 10 20)
    (fp_text reference "U1" (at 0 -2) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15)) (justify left bottom)))
    (fp_text user "edge" (at 1 2 30) (layer "Dwgs.User") (effects (font (size 1 1) (thickness 0.15))))
    (fp_arc (start 0 0) (end 1 0) (angle 90) (layer "F.SilkS") (width 0.12))
    (pad "1" smd roundrect (at -1 1 90) (size 1 1) (layers "In1.Cu" "F.Mask") (roundrect_rratio 0) (chamfer_ratio 0.2) (chamfer bottom_left))
    (pad "2" thru_hole custom (at 1 1) (size 1 1) (drill 0.5 (offset 0 0.1)) (layers "*.Cu" "*.Mask")
      (primitives (gr_poly (pts (xy 0 0) (xy 1 -1) (xy 1 0)) (width 0))))
    (pad "3" smd trapezoid (at 0 -1) (size 1 1) (rect_delta 0 0.2) (layers "F.Cu"))
    (zone (layers "F.Cu" "In2.Cu") (polygon (pts (xy 9 19) (xy 11 19) (xy 11 22))))))"#;
        let mut b = Board::parse(src).unwrap();
        b.flip_footprint(0);
        let out = b.write(src).unwrap();
        for part in [
            r#"(fp_text reference "U1" (at 0 2 180) (layer "B.SilkS") (effects (font (size 1 1) (thickness 0.15)) (justify left bottom mirror)))"#,
            r#"(fp_text user "edge" (at 1 -2 150) (layer "Dwgs.User") (effects (font (size 1 1) (thickness 0.15))))"#,
            r#"(fp_arc (start 0 0) (end 1 0) (angle -90) (layer "B.SilkS") (width 0.12))"#,
            r#"(at -1 -1 270) (size 1 1) (layers "In2.Cu" "B.Mask") (roundrect_rratio 0) (chamfer_ratio 0.2) (chamfer top_left)"#,
            r#"(drill 0.5 (offset 0 -0.1)) (layers "*.Cu" "*.Mask")
      (primitives (gr_poly (pts (xy 0 0) (xy 1 1) (xy 1 0)) (width 0))))"#,
            r#"(rect_delta 0 -0.2) (layers "B.Cu")"#,
            r#"(zone (layers "B.Cu" "In1.Cu") (polygon (pts (xy 9 21) (xy 11 21) (xy 11 18))))"#,
        ] {
            assert!(out.contains(part), "{part}\n{out}");
        }
        let back = Board::parse(&out).unwrap();
        assert_eq!((&back.footprints, &back.pads, &back.shapes, &back.texts), (&b.footprints, &b.pads, &b.shapes, &b.texts));
        flips_back(&mut b, src, 0);
        // Flipped and turned a quarter: the turn adds to its flipped angles, the keepout turns along.
        b.flip_footprint(0);
        b.place_footprint(0, b.footprints[0].at, 90.0);
        let out = b.write(src).unwrap();
        assert!(out.contains("(at 10 20 90)") && out.contains("(at 0 2 270)") && out.contains("(xy 11 21) (xy 11 19) (xy 8 19)"), "{out}");
        let back = Board::parse(&out).unwrap();
        assert_eq!((&back.pads, &back.shapes), (&b.pads, &b.shapes));
    }
}
