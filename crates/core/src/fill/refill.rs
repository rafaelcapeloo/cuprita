//! Zone fills after an edit (`Board::refill`): the zones whose fill it can change filled again,
//! only near what changed where that is a small part of a zone, in the edit's undo step.

use super::{Ctx, Done, Kind, OVERLAP, bounds, fill, keep, pt_box, shapes_box};
use crate::board::{Board, LayerId, LayerMask, MAX_ERROR, Nm, Pt, Zone, ZoneFill, arc_points, union};
use crate::drc::{RBox, overlaps, pad_box};
use crate::font;
use crate::history::Patch;
use crate::polygon::{Shape, fracture, ip, overlay, path};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::{Overlay, ShapeType};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::point::IntPoint;

impl Board {
    /// Fills again the copper zones whose fill an edit can change, given `undo`, the patch that
    /// undoes it, as `fill_zones` fills them: those on the layers of the copper it put in,
    /// changed or took away, near it (all, when it changed the board's edge), and those keeping
    /// out of their fills; not the zones the edit put in or changed, which keep the fills it gave
    /// them. Where that is a small part of a zone, only its fill near the change is made again
    /// (`Ctx::window`). Returns `undo` undoing the refill too, and how many zones' fills changed.
    pub fn refill(&mut self, undo: Patch) -> (Patch, usize) {
        let changed = changed_copper(self, &undo);
        let mut only = vec![LayerMask::MAX; self.zones.len()];
        undo.zones_put(self).for_each(|z| only[z] = 0);
        let filled = if changed.is_empty() { vec![] } else { fill(self, Some(&only), &changed) };
        let refilled = filled.into_iter().enumerate().filter_map(|(z, f)| Some(z).zip(f)).filter_map(|(z, fills)| {
            let range = self.zones[z].fills.clone();
            (fills[..] != self.fills[range.clone()]).then(|| {
                let zone = Zone { fills: range.start..range.start + fills.len(), ..self.zones[z].clone() };
                Patch::zone(z, zone, range, fills)
            })
        });
        let refilled: Vec<Patch> = refilled.collect();
        let n = refilled.len();
        (self.apply(refilled.into_iter().collect()).then(undo), n)
    }
}

/// Where the copper an edit put in, changed or took away is and was, on which copper layers;
/// everywhere, when it changed the board's edge.
fn changed_copper(b: &Board, undo: &Patch) -> Vec<(LayerMask, (Pt, Pt))> {
    if undo.edge(b) {
        return vec![(b.copper, (Pt { x: Nm::MIN, y: Nm::MIN }, Pt { x: Nm::MAX, y: Nm::MAX }))];
    }
    let c = undo.changed(b);
    let mut out = vec![];
    for t in c.tracks {
        let path = t.mid.map_or_else(|| vec![t.a, t.b], |m| arc_points(t.a, m, t.b));
        out.extend(bounds(&path).map(|at| (1 << t.layer, pad_box(at, t.width / 2 + MAX_ERROR as Nm))));
    }
    out.extend(c.vias.iter().map(|v| (v.layers & b.copper, pad_box((v.at, v.at), v.size / 2))));
    // A hole keeps the zones on the layers its pad isn't on clear of it.
    out.extend(c.pads.iter().map(|p| (if p.drill.is_some() { b.copper } else { p.layers & b.copper }, RBox::pad(p).bounds)));
    for s in c.shapes.iter().filter(|s| b.copper & 1 << s.layer != 0) {
        out.extend(bounds(&path(&s.geom)).map(|at| (1 << s.layer, pad_box(at, s.width / 2 + MAX_ERROR as Nm))));
    }
    for t in c.texts.iter().filter(|t| b.copper & 1 << t.layer != 0) {
        let mut at: Option<(Pt, Pt)> = None;
        font::strokes(t, |pts| at = pts.iter().map(|&p| (p, p)).chain(at).reduce(union));
        out.extend(at.map(|at| (1 << t.layer, pad_box(at, t.thickness / 2))));
    }
    out
}

impl Ctx<'_> {
    /// Where zone `zi`'s fill on layer `l` can change as the copper or fills in the boxes `at`
    /// (near it) do: the box to fill again, holding what they reach (its clearances, minimum
    /// width, and the spokes of its net's pads they reach, whole), and the larger box to work
    /// that out in, holding all that decides the fill in the first (the spokes reaching into
    /// it, whole). `None` when they reach much of the zone, which then fills whole.
    pub(super) fn window(&self, zi: usize, l: LayerId, at: &[(Pt, Pt)]) -> Option<((Pt, Pt), (Pt, Pt))> {
        let (z, zone_box) = (&self.b.zones[zi], shapes_box(&self.areas[zi]).map(pt_box)?);
        let (reach, spoke) = (self.reach(zi), z.thermal_gap + z.spoke_width + 2 * OVERLAP as Nm);
        let changed = at.iter().copied().reduce(union)?;
        // The box `w` with the spokes of the zone's net's pads that come within `by` of it,
        // grown by `add`.
        let spokes = |w: (Pt, Pt), by: Nm, add: Nm| {
            let mut out = w;
            self.tree.query(pad_box(w, by + spoke), |i| {
                let o = &self.obs[i as usize];
                let Kind::Pad(p) = o.kind else { return };
                if z.net != 0 && o.net == z.net && o.layers & 1 << l != 0 {
                    let near = self.spokes(zi, i as usize, p).filter(|s| overlaps(pad_box(s.0.bounds, by), w));
                    out = near.fold(out, |out, s| union(out, pad_box(s.0.bounds, add)));
                }
            });
            out
        };
        let w = union(pad_box(changed, reach + spoke), spokes(changed, reach, 0));
        let wc = union(pad_box(w, reach), spokes(w, 0, reach));
        let size = |(lo, hi): (Pt, Pt)| (hi.x as f64 - lo.x as f64) * (hi.y as f64 - lo.y as f64);
        (size(wc) * 4.0 < size(zone_box)).then_some((w, wc))
    }

    /// Zone `zi`'s fill on layer `l` made again in the box `w` alone, worked out in `wc`
    /// (`window`), keeping out of the fills `above`: its fills now, `old`, lose what of them is
    /// in the box, which the new fill takes; those not meeting it stay as they are. `None` when
    /// the box can't tell: an island it joins to the net may go on out of it (as one that went
    /// before), or none of the zone's islands touches the net (so those that went stay).
    pub(super) fn refill_layer<'f>(
        &self,
        zi: usize,
        l: LayerId,
        above: &[&Done],
        (w, wc): ((Pt, Pt), (Pt, Pt)),
        old: impl Iterator<Item = &'f ZoneFill>,
    ) -> Option<(Vec<Shape>, Vec<Vec<Pt>>)> {
        let rect = |(lo, hi): (Pt, Pt)| vec![vec![ip(lo), IntPoint::new(hi.x, lo.y), ip(hi), IntPoint::new(lo.x, hi.y)]];
        let area = overlay(&self.areas[zi], &[rect(wc)], OverlayRule::Intersect);
        let (filled, mut marks) = self.filled(zi, l, above, wc, &area);
        let old: Vec<&ZoneFill> = old.collect();
        let meets = |f: &ZoneFill| bounds(&f.pts).is_some_and(|at| overlaps(at, w));
        // As saved, holes join the outline by cuts: even-odd reads them back.
        let mut o = Overlay::new(old.len() + 1);
        for f in old.iter().filter(|f| meets(f)) {
            o.add_contour(&f.pts.iter().map(|&p| ip(p)).collect::<Vec<_>>(), ShapeType::Subject);
        }
        o.add_contour(&rect(w)[0], ShapeType::Clip);
        let out = o.overlay(OverlayRule::Difference, FillRule::EvenOdd);
        let mut islands = overlay(&out, &overlay(&filled, &[rect(w)], OverlayRule::Intersect), OverlayRule::Union);
        // The points of the net's copper out of the window too (its islands go on out of it).
        let (net, at) = (self.b.zones[zi].net, shapes_box(&islands).map(pt_box));
        self.tree.query(at.unwrap_or(wc), |i| {
            let o = &self.obs[i as usize];
            if net != 0 && o.net == net && o.layers & 1 << l != 0 && matches!(o.kind, Kind::Copper | Kind::Pad(_)) {
                marks.extend(o.at.map(ip));
            }
        });
        let touches = self.touches(zi, l, &islands, &marks, above);
        // An island in the box, reaching its edge: it may go on out of it.
        let held = |s: &Shape| {
            shapes_box(std::slice::from_ref(s)).map(pt_box).is_some_and(|(lo, hi)| {
                let inside = w.0.x <= lo.x && w.0.y <= lo.y && hi.x <= w.1.x && hi.y <= w.1.y;
                inside && (lo.x == w.0.x || lo.y == w.0.y || hi.x == w.1.x || hi.y == w.1.y)
            })
        };
        let stays = old.iter().any(|f| !meets(f));
        if !(stays || touches.contains(&true)) || islands.iter().zip(&touches).any(|(s, &t)| t && held(s)) {
            return None;
        }
        // shortcut: islands out of the box stay as they were, even where none touched the net
        // and one in it now does; fill the zone whole (B) if that ever matters.
        keep(&self.b.zones[zi], &mut islands, &touches, true);
        let (first, kept) = (old.iter().take_while(|f| !meets(f)).count(), old.iter().filter(|f| !meets(f)));
        let mut fills: Vec<Vec<Pt>> = kept.clone().map(|f| f.pts.clone()).collect();
        fills.splice(first..first, islands.iter().map(fracture));
        islands.extend(kept.map(|f| vec![f.pts.iter().map(|&p| ip(p)).collect()]));
        Some((islands, fills))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{HEAD, inside};
    use super::*;
    use crate::board::MM;
    use crate::history::Pick;

    /// The board read back from its text (KiCad 10's: fills as filled) with its zones filled,
    /// and that text.
    fn saved(items: &str) -> (Board, String) {
        let text = format!("{HEAD} {items})").replace("20221018", "20260206");
        let mut b = Board::parse(&text).unwrap();
        b.fill_zones();
        let src = b.write(&text).unwrap();
        (Board::parse(&src).unwrap(), src)
    }

    #[test]
    fn refills_the_zones_an_edit_changes_as_a_refill_of_all_and_undoes_them() {
        let (mut b, src) = saved(
            r#"(gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts") (stroke (width 0.1)))
              (segment (start 2 5) (end 12 5) (width 0.25) (layer "F.Cu") (net 2))
              (zone (net 1) (net_name "GND") (layer "F.Cu") (connect_pads (clearance 0.3)) (min_thickness 0.25)
                (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4)) (polygon (pts (xy 20 2) (xy 38 2) (xy 38 28) (xy 20 28))))
              (zone (net 1) (net_name "GND") (layer "B.Cu") (connect_pads (clearance 0.3)) (min_thickness 0.25)
                (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4)) (polygon (pts (xy 2 2) (xy 38 2) (xy 38 28) (xy 2 28))))"#,
        );
        let orig = b.clone();
        let pt = |x, y| Pt { x: x * MM, y: y * MM };
        // Moved on F.Cu, but 6 mm from the F.Cu zone: no zone refills.
        let moved = b.move_items(&[Pick::Track(0)], pt(0, 10));
        let (undo, n) = b.refill(moved);
        assert_eq!((n, &b.fills), (0, &orig.fills));
        b.apply(undo);
        // Across the F.Cu zone: it keeps 0.3 mm from the track, as a refill of all leaves it.
        let moved = b.move_items(&[Pick::Track(0)], pt(20, 10));
        let (undo, n) = b.refill(moved);
        assert_eq!((n, inside(&b, 25.0, 15.4), inside(&b, 25.0, 15.5), b.clone().fill_zones()), (1, false, true, 0));
        assert_eq!(&b.fills[b.zones[1].fills.clone()], &orig.fills[orig.zones[1].fills.clone()]);
        let refilled = (b.tracks.clone(), b.zones.clone(), b.fills.clone());
        // One step: undone, the board is as read, saved byte for byte; redone, refilled again.
        let redo = b.apply(undo);
        assert_eq!((&b.tracks, &b.zones, &b.fills, b.write(&src).unwrap()), (&orig.tracks, &orig.zones, &orig.fills, src.clone()));
        b.apply(redo);
        assert_eq!((b.tracks.clone(), b.zones.clone(), b.fills.clone()), refilled);
    }

    #[test]
    fn refills_the_zones_keeping_out_of_a_fill_that_changed_far_from_the_edit() {
        // A GND track cuts the SIG strip: its end beyond, where no SIG copper is, goes; the small
        // GND zone there, 10 mm from the track, fills it.
        let (mut b, _) = saved(
            r#"(gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts") (stroke (width 0.1)))
              (footprint "P" (layer "F.Cu") (at 6 12) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 2 "SIG")))
              (segment (start 20 8) (end 20 16) (width 0.5) (layer "F.Cu") (net 1))
              (zone (net 2) (net_name "SIG") (layer "F.Cu") (priority 1) (connect_pads yes (clearance 0.3)) (min_thickness 0.25)
                (fill yes) (polygon (pts (xy 5 10) (xy 35 10) (xy 35 14) (xy 5 14))))
              (zone (net 1) (net_name "GND") (layer "F.Cu") (connect_pads yes (clearance 0.3)) (min_thickness 0.25)
                (fill yes) (polygon (pts (xy 30 8) (xy 38 8) (xy 38 16) (xy 30 16))))"#,
        );
        assert!(inside(&b, 32.0, 12.0));
        // The track moved out of the strip: the strip reaches its end, and the GND zone keeps out of it.
        let moved = b.move_items(&[Pick::Track(0)], Pt { x: 0, y: 10 * MM });
        let (_, n) = b.refill(moved);
        assert_eq!((n, inside(&b, 32.0, 12.0), b.clone().fill_zones()), (2, true, 0));
        assert_eq!(b.fills[b.zones[0].fills.clone()].len(), 1);
    }

    #[test]
    fn refills_after_edits_to_zones_too_each_as_its_edit_left_it() {
        // A SIG zone, deleted with the SIG track that cuts the GND zone before it in two: its
        // fill goes back between the GND zone's, one fewer once refilled, and the B.Cu zone's.
        let (mut b, src) = saved(
            r#"(gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts") (stroke (width 0.1)))
              (footprint "P" (layer "F.Cu") (at 12 15) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "GND")))
              (footprint "Q" (layer "F.Cu") (at 36 15) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "GND")))
              (segment (start 24 0) (end 24 30) (width 0.25) (layer "F.Cu") (net 2))
              (zone (net 1) (net_name "GND") (layer "F.Cu") (connect_pads (clearance 0.3)) (min_thickness 0.25)
                (fill yes) (polygon (pts (xy 10 2) (xy 38 2) (xy 38 28) (xy 10 28))))
              (zone (net 2) (net_name "SIG") (layer "F.Cu") (connect_pads (clearance 0.3)) (min_thickness 0.25)
                (fill yes) (polygon (pts (xy 2 2) (xy 8 2) (xy 8 8) (xy 2 8))))
              (zone (net 2) (net_name "SIG") (layer "B.Cu") (connect_pads (clearance 0.3)) (min_thickness 0.25)
                (fill yes) (polygon (pts (xy 2 20) (xy 8 20) (xy 8 28) (xy 2 28))))"#,
        );
        let orig = b.clone();
        assert_eq!((orig.zones[0].fills.len(), orig.fills.len()), (2, 4));
        let deleted = b.delete_items(&[Pick::Zone(1), Pick::Track(0)]);
        let (undo, n) = b.refill(deleted);
        assert_eq!((n, b.zones.len(), b.fills.len(), b.zones[0].fills.clone(), b.clone().fill_zones()), (1, 2, 2, 0..1, 0));
        let refilled = (b.zones.clone(), b.fills.clone());
        let redo = b.apply(undo);
        assert_eq!((&b.tracks, &b.zones, &b.fills, b.write(&src).unwrap()), (&orig.tracks, &orig.zones, &orig.fills, src.clone()));
        b.apply(redo);
        assert_eq!((b.zones.clone(), b.fills.clone(), b.tracks.len()), (refilled.0, refilled.1, 0));
    }
}
