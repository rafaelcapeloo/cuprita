//! Footprints (KiCad <= 5 `module`s): their pads, fields and 3D models.

use super::{Parser, expand};
use crate::board::{Connect, Footprint, Model, Nm, Pad, PadShape, Pt, Src, part_box, place};
use crate::polygon::custom_copper;
use crate::sexpr::{Cursor, Result, unescape};
use crate::tr::tr;
use crate::units::mm_to_nm;

impl<'a> Parser<'a> {
    pub(crate) fn footprint(&mut self, c: &mut Cursor<'a>) -> Result<()> {
        let lib_id = unescape(c.opt_atom()?.unwrap_or_default());
        let (fp_id, pads0, shapes0, texts0) = (self.b.footprints.len() as u32, self.b.pads.len(), self.b.shapes.len(), self.b.texts.len());
        let (mut at, mut deg, mut reference, mut value, mut fitted, mut placed, mut back) = (Pt::default(), 0.0, "", "", true, true, false);
        let mut mask_bridges = false;
        let (mut clearance, mut zone_connect, mut mask, mut paste, mut ratio, mut fields) = (None, None, None, None, None, vec![]);
        // Its `fp_text`s and `property`s so far, hidden ones too: where saving finds each.
        let (mut models, mut texts) = (vec![], 0);
        while let Some(head) = c.child()? {
            match head {
                "at" => (at, deg) = (c.point()?, c.opt_angle()?),
                // KiCad <= 5 wrote copper layers by the user's names (`C4B`): any below the top is the back.
                "layer" => {
                    let name = c.atom()?;
                    back = name == "B.Cu" || self.layer_ids.get(name).and_then(|l| self.cu_depth.get(l)).is_some_and(|&d| d > 0);
                }
                "clearance" => clearance = Some(c.nm()?),
                "zone_connect" => zone_connect = connect(c)?,
                "solder_mask_margin" => mask = Some(c.nm()?),
                "solder_paste_margin" => paste = Some(c.nm()?),
                "solder_paste_ratio" | "solder_paste_margin_ratio" => ratio = Some(c.f64()?),
                "attr" => {
                    while let Some(a) = c.opt_atom()? {
                        fitted &= !matches!(a, "dnp" | "exclude_from_bom" | "virtual");
                        placed &= !matches!(a, "dnp" | "exclude_from_pos_files" | "virtual");
                        mask_bridges |= a == "allow_soldermask_bridges";
                    }
                }
                "pad" => self.pad(c, fp_id)?,
                "model" => models.extend(model(c)?),
                "fp_line" | "fp_arc" | "fp_circle" | "fp_rect" | "fp_poly" => self.shape(c, head, false)?,
                "fp_text" | "property" => {
                    let (shown, hidden) = (self.b.texts.len(), self.hidden.len());
                    let named = self.text(c, head)?;
                    if let Some(t) = self.b.texts.get_mut(shown).or(self.hidden.get_mut(hidden)) {
                        t.src = Some(texts);
                    }
                    texts += 1;
                    match named {
                        ("reference" | "Reference", s) => reference = s,
                        ("value" | "Value", s) => value = s,
                        (name, s) if head == "property" => fields.push((unescape(name), unescape(s))),
                        _ => {}
                    }
                }
                _ => {}
            }
            c.close()?;
        }
        // Children are stored footprint-local; pad and text angles are already absolute.
        for p in &mut self.b.pads[pads0..] {
            p.at = place(p.at, at, deg);
            p.clearance = p.clearance.or(clearance);
            p.zone_connect = p.zone_connect.or(zone_connect);
            (p.mask_margin, p.paste_margin, p.paste_ratio) = (p.mask_margin.or(mask), p.paste_margin.or(paste), p.paste_ratio.or(ratio));
        }
        let mut f = Footprint {
            reference: unescape(reference),
            value: unescape(value),
            lib_id,
            fitted,
            placed,
            mask_bridges,
            fields,
            back,
            at,
            angle: deg.rem_euclid(360.0),
            bounds: (at, at),
            pads: pads0..self.b.pads.len(),
            shapes: shapes0..self.b.shapes.len(),
            texts: texts0..self.b.texts.len(),
            hidden: std::mem::take(&mut self.hidden),
            src: Src::File(fp_id as usize),
            models,
        };
        for s in &mut self.b.shapes[shapes0..] {
            s.geom.map(|p| place(p, at, deg));
        }
        f.bounds = part_box(&self.b.pads[pads0..], &self.b.shapes[shapes0..], at);
        let (reference, value) = (f.reference.clone(), f.value.clone());
        for t in self.b.texts[texts0..].iter_mut().chain(&mut f.hidden) {
            t.at = place(t.at, at, deg);
            if t.text.contains("${") {
                let field = |name: &str| match name {
                    "REFERENCE" => Some(reference.as_str()),
                    "VALUE" => Some(value.as_str()),
                    _ => None,
                };
                t.text = expand(&t.text, field);
            }
        }
        self.b.footprints.push(f);
        Ok(())
    }

    fn pad(&mut self, c: &mut Cursor<'a>, footprint: u32) -> Result<()> {
        let number = unescape(c.atom()?);
        let hole = c.atom()?;
        let kind = c.atom()?;
        let shape = match kind {
            // A custom pad's anchor, unless its options say otherwise.
            "circle" | "custom" => PadShape::Circle,
            "oval" => PadShape::Oval,
            "roundrect" => PadShape::RoundRect(0.25),
            "trapezoid" => PadShape::Trapezoid,
            _ => PadShape::Rect,
        };
        let mut pad = Pad {
            number,
            footprint,
            at: Pt::default(),
            angle: 0.0,
            size: (0, 0),
            shape,
            drill: None,
            layers: 0,
            net: 0,
            clearance: None,
            zone_connect: None,
            spoke_angle: 0.0,
            custom: vec![],
            mask_margin: None,
            paste_margin: None,
            paste_ratio: None,
        };
        let mut spoke_angle = None;
        while let Some(head) = c.child()? {
            match head {
                "at" => (pad.at, pad.angle) = (c.point()?, c.opt_angle()?),
                "size" => pad.size = (c.nm()?, c.nm()?),
                "drill" => pad.drill = drill(c)?,
                "layers" => pad.layers = self.layer_list(c)?,
                "roundrect_rratio" if matches!(pad.shape, PadShape::RoundRect(_)) => pad.shape = PadShape::RoundRect(c.f64()? as f32),
                "net" => pad.net = self.net(c)?,
                "clearance" => pad.clearance = Some(c.nm()?),
                "zone_connect" => pad.zone_connect = connect(c)?,
                "solder_mask_margin" => pad.mask_margin = Some(c.nm()?),
                "solder_paste_margin" => pad.paste_margin = Some(c.nm()?),
                "solder_paste_margin_ratio" | "solder_paste_ratio" => pad.paste_ratio = Some(c.f64()?),
                // shortcut: a pad's own thermal gap and spoke width aren't read (no corpus board
                // sets them); the zone's apply. Read them when a board does.
                "thermal_bridge_angle" => spoke_angle = Some(c.f64()?),
                // shortcut: `(clearance convexhull)` keeps the outline's clearance, not its convex
                // hull's (no corpus board uses it); add the hull when a board does.
                "options" => {
                    while let Some(head) = c.child()? {
                        if head == "anchor" {
                            pad.shape = if c.atom()? == "rect" { PadShape::Rect } else { PadShape::Circle };
                        }
                        c.close()?;
                    }
                }
                "primitives" => {
                    while let Some(kind) = c.child()? {
                        // shortcut: Bézier curves (`gr_curve`) are left out; read them when a board has one.
                        if matches!(kind, "gr_line" | "gr_arc" | "gr_circle" | "gr_rect" | "gr_poly") {
                            self.shape(c, kind, true)?;
                        }
                        c.close()?;
                    }
                }
                _ => {}
            }
            c.close()?;
        }
        // Round pads get 45° spokes; KiCad <= 6 gave custom pads 90°, also on a round anchor.
        let round = pad.shape == PadShape::Circle && (kind != "custom" || self.b.version >= 20221018);
        pad.spoke_angle = spoke_angle.unwrap_or(if round { 45.0 } else { 90.0 });
        // KiCad keeps plated holes' pads on every copper layer, whatever they list.
        match hole {
            "thru_hole" => pad.layers |= self.b.copper,
            "np_thru_hole" => pad.layers &= !self.b.copper,
            _ => {}
        }
        let prims = std::mem::take(&mut self.prims);
        if !prims.is_empty() {
            pad.custom = custom_copper(&pad, &prims);
        }
        self.b.pads.push(pad);
        Ok(())
    }
}

/// `(model "file" (offset (xyz X Y Z)) (scale (xyz …)) (rotate (xyz …)))`, `None` when hidden.
/// KiCad <= 5 wrote the offset as `at`, in inches.
pub(crate) fn model(c: &mut Cursor) -> Result<Option<Model>> {
    let mut m = Model { file: unescape(c.atom()?), offset: [0.0; 3], scale: [1.0; 3], rotate: [0.0; 3] };
    let mut hidden = false;
    loop {
        // KiCad 6 and 7 write `hide` as a bare flag.
        while let Some(flag) = c.opt_atom()? {
            hidden |= flag == "hide";
        }
        let Some(head) = c.child()? else { return Ok((!hidden).then_some(m)) };
        let xyz = |c: &mut Cursor| -> Result<[f64; 3]> {
            let mut v = [0.0; 3];
            if c.child()? == Some("xyz") {
                for x in &mut v {
                    *x = c.f64()?;
                }
                c.close()?;
            }
            Ok(v)
        };
        match head {
            "offset" => m.offset = xyz(c)?,
            "at" => m.offset = xyz(c)?.map(|inch| inch * 25.4),
            "scale" => m.scale = xyz(c)?,
            "rotate" => m.rotate = xyz(c)?,
            "hide" => hidden = c.atom()? == "yes",
            _ => {}
        }
        c.close()?;
    }
}

/// `(zone_connect 2)`: KiCad's number for a pad's zone connection.
fn connect(c: &mut Cursor) -> Result<Option<Connect>> {
    let all = [Connect::None, Connect::Thermal, Connect::Solid, Connect::ThruHoleThermal];
    Ok(c.atom()?.parse().ok().and_then(|i: usize| all.get(i).copied()))
}

/// `(drill 0.8)`, `(drill oval 1 2)`, optionally followed by `(offset ...)`.
fn drill(c: &mut Cursor) -> Result<Option<(Nm, Nm)>> {
    let (w, h) = match c.opt_atom()? {
        None => return Ok(None),
        Some("oval") => {
            let w = c.atom()?;
            (w, c.opt_atom()?.unwrap_or(w))
        }
        Some(d) => (d, d),
    };
    match (mm_to_nm(w), mm_to_nm(h)) {
        (Some(w), Some(h)) => Ok(Some((w, h)).filter(|&(w, _)| w > 0)),
        _ => Err(c.err(tr("bad drill"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;

    #[test]
    fn pads_take_their_footprints_settings() {
        let b = Board::parse(
            r#"(kicad_pcb (version 20260206) (layers (0 "F.Cu" signal) (2 "In1.Cu" signal) (31 "B.Cu" signal))
              (footprint "U" (layer "F.Cu") (at 0 0) (clearance 0.3) (zone_connect 2) (solder_mask_margin 0.05) (solder_paste_ratio -0.1)
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (zone_connect 0) (thermal_bridge_angle 30)
                  (solder_mask_margin 0.02) (solder_paste_margin -0.03) (solder_paste_margin_ratio 0))
                (pad "2" thru_hole circle (at 2 0) (size 1 1) (drill 0.5) (layers "*.Cu") (clearance 0.1))))"#,
        )
        .unwrap();
        // A pad's own setting wins over its footprint's; circle pads put their spokes at 45°.
        let pads: Vec<_> = b.pads.iter().map(|p| (p.clearance, p.zone_connect, p.spoke_angle)).collect();
        assert_eq!(pads, [(Some(300_000), Some(Connect::None), 30.0), (Some(100_000), Some(Connect::Solid), 45.0)]);
        let margins: Vec<_> = b.pads.iter().map(|p| (p.mask_margin, p.paste_margin, p.paste_ratio)).collect();
        assert_eq!(margins, [(Some(20_000), Some(-30_000), Some(0.0)), (Some(50_000), None, Some(-0.1))]);
    }

    #[test]
    fn finds_the_back_by_its_copper_also_when_renamed() {
        // KiCad <= 5 wrote the names users gave copper layers.
        let b = Board::parse(
            "(kicad_pcb (version 4) (layers (0 C1F signal) (1 C2 signal hide) (31 C4B signal) (37 F.SilkS user))
              (module TP (layer C4B) (at 1 2)) (module R (layer C1F) (at 0 0)))",
        )
        .unwrap();
        assert_eq!((b.footprints[0].back, b.footprints[1].back), (true, false));
    }

    #[test]
    fn puts_plated_holes_on_every_copper_layer() {
        // As KiCad does, whatever the pad lists (this spacer's ring lists only B.Cu); unplated
        // holes have no copper.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (31 "B.Cu" signal) (38 "B.Mask" user))
              (footprint "SP" (layer "B.Cu") (at 0 0)
                (pad "1" thru_hole circle (at 0 0) (size 5.3 5.3) (drill 3) (layers "B.Cu" "B.Mask"))
                (pad "" np_thru_hole circle (at 9 0) (size 3 3) (drill 3) (layers "*.Cu" "*.Mask"))))"#,
        )
        .unwrap();
        assert_eq!((b.pads[0].layers, b.pads[1].layers), (0b1111, 0b1000));
    }

    #[test]
    fn parses_custom_pads_in_their_own_frame() {
        let b = Board::parse(
            "(kicad_pcb (version 20171130) (layers (0 F.Cu signal))
              (module K (layer F.Cu) (at 10 10 90)
                (pad 1 smd custom (at 1 0 90) (size 1 1) (layers F.Cu) (options (clearance outline) (anchor circle))
                  (primitives (gr_poly (pts (xy 2 -1) (xy 4 -1) (xy 4 1) (xy 2 1)) (width 0))
                    (gr_circle (center 0 4) (end 1 4) (width 0)) (gr_circle (center 0 -4) (end 1 -4) (width 0.2))
                    (gr_line (start -2 0) (end -4 0) (width 0.3)) (gr_curve (pts (xy 0 0) (xy 1 1) (xy 2 1) (xy 3 0)) (width 0.1)))))
              (module R (layer F.Cu) (at 0 0) (pad 1 smd custom (at 0 0) (size 1 2) (layers F.Cu) (thermal_bridge_angle 30))))",
        )
        .unwrap();
        // As KiCad 10 reads them: a round anchor unless the options say rect; on a KiCad <= 6
        // custom pad 90° spokes, also on a round anchor.
        let (p, plain) = (&b.pads[0], &b.pads[1]);
        assert_eq!(
            (p.shape, p.spoke_angle, plain.shape, plain.spoke_angle, plain.custom.len()),
            (PadShape::Circle, 90.0, PadShape::Circle, 30.0, 0)
        );
        // As KiCad 10 reads KiCad 5 primitives: the polygon and the zero-width circle filled, the
        // other circle a ring. Round anchor π/4, square 4, disc π, ring 0.4π, line 0.6 + 0.0225π;
        // curves are drawn outside them.
        let area: f64 = p
            .custom
            .iter()
            .map(|c| c.iter().zip(c.iter().cycle().skip(1)).map(|(a, b)| a.x as f64 * b.y as f64 - b.x as f64 * a.y as f64).sum::<f64>())
            .sum();
        let over = area.abs() / 2e12 - (std::f64::consts::PI * (0.25 + 1.0 + 0.4 + 0.0225) + 4.6);
        assert!((0.0..0.1).contains(&over), "{over} mm² over");
        // On the board: the pad's (1, 0) and its primitives turned 90° counter-clockwise, at (10, 10).
        let pts: Vec<Pt> = p.copper().into_iter().flatten().collect();
        let (lo, hi) = pts
            .iter()
            .fold((pts[0], pts[0]), |(l, h), q| (Pt { x: l.x.min(q.x), y: l.y.min(q.y) }, Pt { x: h.x.max(q.x), y: h.y.max(q.y) }));
        let mm = |p: Pt| ((p.x as f64 / 1e4).round() / 100.0, (p.y as f64 / 1e4).round() / 100.0);
        assert_eq!((mm(p.at), mm(lo), mm(hi)), ((10.0, 9.0), (4.9, 5.0), (15.0, 13.15)));
    }
}
