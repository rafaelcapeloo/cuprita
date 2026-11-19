//! Gerber X2: a board layer as the image a fab works from. RS-274X with X2 file attributes, in
//! mm with 6 decimals (the board's nanometres exactly), Y up; pads flash apertures, zone fills
//! are regions, tracks and graphics draw with round apertures, arcs as arcs.

use crate::board::{Board, Geom, LayerId, Nm, Pad, PadShape, Pt, Shape, arc_center, rotate};
use crate::drc::RBox;
use crate::font;
use crate::polygon::{outlines, rounded};
use std::collections::HashMap;
use std::fmt::Write;

/// What a flash leaves, or the round pen of a draw.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Aperture {
    Circle(Nm),
    Rect(Nm, Nm),
    Oval(Nm, Nm),
    /// Contours around the flash point, on board axes: each outline (`true`), then the holes in it.
    Outline(Vec<(bool, Vec<Pt>)>),
}

struct Gerber {
    out: String,
    apertures: HashMap<Aperture, usize>,
    current: Option<usize>,
    /// Where the pen is, so a draw going on from there needs no move.
    at: Option<Pt>,
}

impl Board {
    /// Layer `l` as a Gerber X2 file whose X2 file function is `function` (`Copper,L1,Top`,
    /// `Soldermask,Bot`, `Paste,Top`, `Legend,Top`, `Profile,NP`), which also decides what shows
    /// besides the layer's fills, graphics, texts and tracks: the pads on it, mask openings and
    /// paste grown by their margins; vias on copper, and on the mask of a side that leaves them open.
    pub(crate) fn gerber(&self, l: LayerId, function: &str) -> String {
        let (r, top) = (&self.rules, function.ends_with("Top"));
        let (mask, paste) = (function.starts_with("Soldermask"), function.starts_with("Paste"));
        let polarity = if mask { "Negative" } else { "Positive" };
        let mut g = Gerber { out: String::new(), apertures: HashMap::new(), current: None, at: None };
        let _ = write!(g.out, "%TF.GenerationSoftware,Cuprita,Cuprita,{}*%\n%TF.SameCoordinates,Original*%\n", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(g.out, "%TF.FileFunction,{function}*%");
        if !function.starts_with("Profile") {
            let _ = writeln!(g.out, "%TF.FilePolarity,{polarity}*%");
        }
        g.out += "%FSLAX46Y46*%\n%MOMM*%\n%LPD*%\nG01*\nG75*\n";
        for f in self.fills.iter().filter(|f| f.layer == l) {
            g.region(&f.pts);
        }
        for s in self.shapes.iter().filter(|s| s.layer == l) {
            g.shape(s);
        }
        for t in self.texts.iter().filter(|t| t.layer == l) {
            font::strokes(t, |pts| g.line(t.thickness, pts));
        }
        for t in self.tracks.iter().filter(|t| t.layer == l) {
            match t.mid {
                Some(m) => g.arc(t.width, t.a, m, t.b),
                None => g.line(t.width, &[t.a, t.b]),
            }
        }
        // Vias: on copper, and as mask openings on a side whose outer layer they reach, untented.
        let (outer, tented) = if top { (self.stackup.first(), r.tented.0) } else { (self.stackup.last(), r.tented.1) };
        let vias = if function.starts_with("Copper") {
            Some((l, 0))
        } else if mask && !tented {
            outer.map(|c| (c.layer, r.mask_margin))
        } else {
            None
        };
        if let Some((cu, grow)) = vias {
            for v in self.vias.iter().filter(|v| v.layers & 1 << cu != 0) {
                g.flash(Aperture::Circle(v.size + 2 * grow), v.at);
            }
        }
        for p in self.pads.iter().filter(|p| p.layers & 1 << l != 0) {
            let size = if p.shape == PadShape::Circle { (p.size.0, p.size.0) } else { p.size };
            // A pad without copper or hole is an opening itself: no margins, as in KiCad.
            let aperture = p.layers & self.copper == 0 && p.drill.is_none();
            let grow = if aperture {
                (0, 0)
            } else if mask {
                let m = p.mask_margin.unwrap_or(r.mask_margin);
                (m, m)
            } else if paste {
                let (m, k) = (p.paste_margin.unwrap_or(r.paste_margin), p.paste_ratio.unwrap_or(r.paste_ratio));
                (m + (k * size.0 as f64).round() as Nm, m + (k * size.1 as f64).round() as Nm)
            } else {
                (0, 0)
            };
            if let Some(a) = pad_aperture(p, size, grow) {
                g.flash(a, p.at);
            }
        }
        g.out += "M02*\n";
        g.out
    }
}

/// Pad `p` (of `size`) grown by `gx` and `gy` on its own axes (shrunk when negative), as KiCad
/// plots mask openings and paste (found by probing KiCad 10): an even margin grows corners by
/// it, rounding a rectangle's; margins that differ (paste ratios) keep a round rectangle's corner
/// ratio and round a rectangle by `gx`. `None` when nothing is left.
// shortcut: chamfered corners and trapezoids plot as their rectangle (the model reads neither;
// one corpus pad has a chamfer); read them when boards need their exact copper.
fn pad_aperture(p: &Pad, size: (Nm, Nm), (gx, gy): (Nm, Nm)) -> Option<Aperture> {
    if !p.custom.is_empty() {
        // shortcut: a custom pad grows by its smaller margin (paste ratios differ per axis); grow
        // each axis when a board needs it.
        let turned: Vec<Vec<Pt>> = p.custom.iter().map(|c| c.iter().map(|&q| rotate(q, p.angle)).collect()).collect();
        let shape = outlines(&turned, gx.min(gy));
        return (!shape.is_empty()).then_some(Aperture::Outline(shape));
    }
    let (w, h) = (size.0 + 2 * gx, size.1 + 2 * gy);
    if w <= 0 || h <= 0 {
        return None;
    }
    let r = match p.shape {
        PadShape::Circle => return Some(Aperture::Circle(w)),
        PadShape::Oval => w.min(h) / 2,
        PadShape::RoundRect(k) if gx == gy => (k as f64 * size.0.min(size.1) as f64).round() as Nm + gx,
        PadShape::RoundRect(k) => (k as f64 * w.min(h) as f64).round() as Nm,
        PadShape::Rect | PadShape::Trapezoid => gx,
    }
    .clamp(0, w.min(h) / 2);
    let turn = p.angle.rem_euclid(180.0);
    if turn == 0.0 || turn == 90.0 {
        let (w, h) = if turn == 0.0 { (w, h) } else { (h, w) };
        if r == 0 {
            return Some(Aperture::Rect(w, h));
        }
        if r == w.min(h) / 2 {
            return Some(if w == h { Aperture::Circle(w) } else { Aperture::Oval(w, h) });
        }
    }
    let outline = rounded(&RBox::boxed(Pt::default(), (w, h), p.angle, r), 0.0);
    Some(Aperture::Outline(vec![(true, outline.iter().map(|q| Pt { x: q.x, y: q.y }).collect())]))
}

impl Gerber {
    /// Makes `a` the current aperture, defining it on first use.
    fn select(&mut self, a: Aperture) {
        let n = self.apertures.len() + 10;
        let out = &mut self.out;
        let d = *self.apertures.entry(a).or_insert_with_key(|a| {
            let mm = |v: Nm| v as f64 / 1e6;
            let _ = match a {
                Aperture::Circle(d) => writeln!(out, "%ADD{n}C,{:.6}*%", mm(*d)),
                Aperture::Rect(w, h) => writeln!(out, "%ADD{n}R,{:.6}X{:.6}*%", mm(*w), mm(*h)),
                Aperture::Oval(w, h) => writeln!(out, "%ADD{n}O,{:.6}X{:.6}*%", mm(*w), mm(*h)),
                // A macro of outline primitives, holes clearing what the outlines before them cover.
                Aperture::Outline(contours) => {
                    let _ = write!(out, "%AMShape{n}*");
                    for (solid, c) in contours {
                        let _ = write!(out, "\n4,{},{}", *solid as u8, c.len());
                        for (i, q) in c.iter().chain(&c[..1]).enumerate() {
                            let _ = write!(out, ",{}{:.6},{:.6}", if i % 8 == 7 { "\n" } else { "" }, mm(q.x), mm(-q.y));
                        }
                        out.push_str(",0*");
                    }
                    writeln!(out, "%\n%ADD{n}Shape{n}*%")
                }
            };
            n
        });
        if self.current != Some(d) {
            let _ = writeln!(self.out, "D{d}*");
            self.current = Some(d);
        }
    }

    /// Writes `X…Y…D0n*` for `p` (Y up).
    fn op(&mut self, p: Pt, d: u8) {
        let _ = writeln!(self.out, "X{}Y{}D0{d}*", p.x, -(p.y as i64));
        self.at = Some(p);
    }

    fn move_to(&mut self, p: Pt) {
        if self.at != Some(p) {
            self.op(p, 2);
        }
    }

    fn flash(&mut self, a: Aperture, at: Pt) {
        self.select(a);
        self.op(at, 3);
    }

    /// A line through `pts` with a round pen `width` wide.
    fn line(&mut self, width: Nm, pts: &[Pt]) {
        let Some((&first, rest)) = pts.split_first() else { return };
        self.select(Aperture::Circle(width));
        self.move_to(first);
        for &p in rest {
            self.op(p, 1);
        }
    }

    /// The arc from `a` through `m` to `b`; a line when they are in line.
    fn arc(&mut self, width: Nm, a: Pt, m: Pt, b: Pt) {
        let Some((cx, cy, _, _, sweep)) = arc_center(a, m, b) else { return self.line(width, &[a, b]) };
        self.select(Aperture::Circle(width));
        self.move_to(a);
        // A sweep of growing angle on the Y-down board turns clockwise as seen, KiCad's G02.
        let (i, j) = ((cx - a.x as f64).round() as i64, (a.y as f64 - cy).round() as i64);
        let _ = writeln!(self.out, "G0{}*", if sweep > 0.0 { 2 } else { 3 });
        let _ = writeln!(self.out, "X{}Y{}I{i}J{j}D01*\nG01*", b.x, -(b.y as i64));
        self.at = Some(b);
    }

    /// A filled area inside `pts`.
    fn region(&mut self, pts: &[Pt]) {
        let Some(&first) = pts.first() else { return };
        self.out += "G36*\n";
        self.op(first, 2);
        for &p in pts[1..].iter().chain([&first]) {
            self.op(p, 1);
        }
        self.out += "G37*\n";
    }

    fn shape(&mut self, s: &Shape) {
        match &s.geom {
            Geom::Segment { a, b } => self.line(s.width, &[*a, *b]),
            Geom::Arc { start, mid, end } => self.arc(s.width, *start, *mid, *end),
            Geom::Circle { center, radius } if s.filled => self.flash(Aperture::Circle(2 * radius + s.width), *center),
            &Geom::Circle { center: c, radius: r } => {
                // A whole turn: the arc's end is its start.
                let start = Pt { x: c.x + r, y: c.y };
                self.select(Aperture::Circle(s.width));
                self.move_to(start);
                let _ = writeln!(self.out, "G02*\nX{}Y{}I{}J0D01*\nG01*", start.x, -(start.y as i64), -r);
            }
            Geom::Poly(pts) => {
                if s.filled {
                    self.region(pts);
                }
                if !s.filled || s.width > 0 {
                    let closed: Vec<Pt> = pts.iter().chain(pts.first()).copied().collect();
                    self.line(s.width, &closed);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::board::Board;

    #[test]
    fn plots_layers_as_kicad_does() {
        let b = Board::parse(
            r#"(kicad_pcb (version 20240108)
              (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (35 "F.Paste" user) (39 "F.Mask" user) (44 "Edge.Cuts" user))
              (setup (pad_to_mask_clearance 0.05) (pad_to_paste_clearance_ratio -0.1) (pcbplotparams (viasonmask true)))
              (net 0 "") (net 1 "GND")
              (footprint "U" (layer "F.Cu") (at 10 10)
                (pad "1" smd rect (at 0 0) (size 2 1) (layers "F.Cu" "F.Paste" "F.Mask") (net 1 "GND"))
                (pad "2" smd roundrect (at 4 0 30) (size 2 1) (roundrect_rratio 0.25) (layers "F.Cu" "F.Mask") (solder_mask_margin 0))
                (pad "" smd circle (at 8 0) (size 0.8 0.8) (layers "F.Mask"))
                (pad "3" thru_hole oval (at 12 0 90) (size 2 1.4) (drill oval 1.2 0.6) (layers "B.Cu")))
              (segment (start 0 0) (end 5 0) (width 0.25) (layer "F.Cu") (net 1))
              (arc (start 11 20) (mid 10 19) (end 9 20) (width 0.2) (layer "F.Cu") (net 1))
              (via (at 5 0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1))
              (zone (net 1) (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy 30 0) (xy 31 0) (xy 31 1))))
              (gr_circle (center 40 0) (end 41 0) (stroke (width 0.1)) (fill none) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        let has = |text: &str, parts: &[&str]| parts.iter().filter(|p| !text.contains(*p)).map(|p| p.to_string()).collect::<Vec<_>>();
        // Copper: Y up, mm with 6 decimals (nm); the plated hole's pad is on F.Cu too, turned 90°;
        // the arc turns counter-clockwise as seen (G03) about I, J from its start.
        let cu = b.gerber(0, "Copper,L1,Top");
        let missing = has(
            &cu,
            &[
                "%TF.FileFunction,Copper,L1,Top*%\n%TF.FilePolarity,Positive*%\n%FSLAX46Y46*%\n%MOMM*%",
                "R,2.000000X1.000000*%",
                "X10000000Y-10000000D03*",
                "%AMShape",
                "O,1.400000X2.000000*%",
                "X22000000Y-10000000D03*",
                "X0Y0D02*\nX5000000Y0D01*",
                "G03*\nX9000000Y-20000000I-1000000J0D01*\nG01*",
                "C,0.600000*%",
                "X5000000Y0D03*",
                "G36*\nX30000000Y0D02*\nX31000000Y0D01*\nX31000000Y-1000000D01*\nX30000000Y0D01*\nG37*",
                "M02*\n",
            ],
        );
        assert_eq!(missing, Vec::<String>::new());
        // Mask: openings grown by the board's margin (a rectangle's corners rounded), not a pad's
        // own 0 nor a pad without copper or hole; vias open, as the plot settings ask.
        let mask = b.gerber(3, "Soldermask,Top");
        let missing =
            has(&mask, &["%TF.FilePolarity,Negative*%", "C,0.800000*%", "X18000000Y-10000000D03*", "C,0.700000*%", "X5000000Y0D03*"]);
        assert_eq!((missing, mask.matches("%AMShape").count(), mask.contains("R,")), (vec![], 2, false));
        // Paste: shrunk by the board's ratio of each side.
        let paste = b.gerber(2, "Paste,Top");
        assert_eq!((paste.contains("R,1.600000X0.800000*%"), paste.contains("D03*"), paste.matches("D03*").count()), (true, true, 1));
        // Outline: no polarity; a circle is a whole turn.
        let edge = b.gerber(4, "Profile,NP");
        assert_eq!(
            has(&edge, &["%TF.FileFunction,Profile,NP*%\n%FSLAX46Y46*%", "X41000000Y0D02*\nG02*\nX41000000Y0I-1000000J0D01*"]),
            Vec::<String>::new()
        );
        assert!(!edge.contains("FilePolarity"));
    }
}
