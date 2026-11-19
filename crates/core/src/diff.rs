//! Visual and structural comparison of two boards, matched by layer, net and reference names.

use crate::board::{Board, Footprint, Geom, LayerId, LayerMask, MM, NetId, Nm, Pad, PadShape, Pt, Shape, Text, Track, Via, square, union};
use crate::tr::{tr, trf};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Changed => "changed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub item: &'static str,
    pub what: String,
    pub bounds: (Pt, Pt),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub layer_map: Vec<LayerId>,
    pub removed_tracks: Vec<bool>,
    pub added_tracks: Vec<bool>,
    pub removed_vias: Vec<bool>,
    pub added_vias: Vec<bool>,
    pub removed_pads: Vec<bool>,
    pub added_pads: Vec<bool>,
    pub removed_shapes: Vec<bool>,
    pub added_shapes: Vec<bool>,
    pub removed_zones: Vec<bool>,
    pub added_zones: Vec<bool>,
    pub removed_texts: Vec<bool>,
    pub added_texts: Vec<bool>,
    pub changes: Vec<Change>,
}

impl Diff {
    /// Visual and structural differences from `old` to `new`, matched by layer, net and reference names.
    pub fn between<'a>(old: &'a Board, new: &'a Board) -> Diff {
        let layer_map: Vec<LayerId> = old
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| new.layers.iter().position(|nl| nl == l).unwrap_or(new.layers.len() + i) as LayerId)
            .collect();
        let map_mask = |mut m: LayerMask| {
            let mut out = 0;
            while m != 0 {
                let b = m.trailing_zeros() as usize;
                if let Some(&nl) = layer_map.get(b)
                    && (nl as u32) < LayerMask::BITS
                {
                    out |= 1 << nl;
                }
                m &= m - 1;
            }
            out
        };
        let net_ids: HashMap<&str, NetId> = new.nets.iter().enumerate().map(|(i, n)| (n.as_str(), i as NetId)).collect();
        let net_map: Vec<NetId> = old
            .nets
            .iter()
            .enumerate()
            .map(|(i, n)| if i == 0 { 0 } else { net_ids.get(n.as_str()).copied().unwrap_or((new.nets.len() + i) as NetId) })
            .collect();
        let (same_l, same_n) = (old.layers == new.layers, old.nets == new.nets);
        let (removed_tracks, added_tracks) = if same_l && same_n {
            diff_keys(&old.tracks, &new.tracks)
        } else {
            let ot: Vec<_> =
                old.tracks.iter().map(|t| Track { layer: layer_map[t.layer as usize], net: net_map[t.net as usize], ..*t }).collect();
            diff_keys(&ot, &new.tracks)
        };
        let (removed_vias, added_vias) = if same_l && same_n {
            diff_keys(&old.vias, &new.vias)
        } else {
            let ov: Vec<_> = old.vias.iter().map(|v| Via { layers: map_mask(v.layers), net: net_map[v.net as usize], ..*v }).collect();
            diff_keys(&ov, &new.vias)
        };
        let pad_key = |b: &'a Board, p: &'a Pad, layers: LayerMask, net: NetId| {
            let r = b.footprints.get(p.footprint as usize).map_or("", |f| f.reference.as_str());
            let sh = match p.shape {
                PadShape::Circle => (0, 0),
                PadShape::Rect => (1, 0),
                PadShape::Oval => (2, 0),
                PadShape::RoundRect(k) => (3, (k * 1e4).round() as i32),
                PadShape::Trapezoid => (4, 0),
            };
            (r, p.number.as_str(), p.at, (p.angle.rem_euclid(360.0) * 1e4).round() as i64, p.size, sh, &p.custom, p.drill, layers, net)
        };
        let op: Vec<_> = old.pads.iter().map(|p| pad_key(old, p, map_mask(p.layers), net_map[p.net as usize])).collect();
        let np: Vec<_> = new.pads.iter().map(|p| pad_key(new, p, p.layers, p.net)).collect();
        let (removed_pads, added_pads) = diff_keys(&op, &np);
        let (removed_shapes, added_shapes) = if same_l {
            diff_keys(&old.shapes, &new.shapes)
        } else {
            let os: Vec<_> = old.shapes.iter().map(|s| (&s.geom, s.width, s.filled, layer_map[s.layer as usize])).collect();
            let ns: Vec<_> = new.shapes.iter().map(|s| (&s.geom, s.width, s.filled, s.layer)).collect();
            diff_keys(&os, &ns)
        };
        let (removed_zones, added_zones) = if same_l && same_n {
            diff_keys(&old.fills, &new.fills)
        } else {
            let oz: Vec<_> = old.fills.iter().map(|z| (&z.pts, layer_map[z.layer as usize], net_map[z.net as usize])).collect();
            let nz: Vec<_> = new.fills.iter().map(|z| (&z.pts, z.layer, z.net)).collect();
            diff_keys(&oz, &nz)
        };
        let tk = |t: &'a Text, l: LayerId| {
            (t.text.as_str(), t.at, (t.angle.rem_euclid(360.0) * 1e4).round() as i64, t.size, t.thickness, l, t.justify, t.mirror)
        };
        let ot: Vec<_> = old.texts.iter().map(|t| tk(t, layer_map[t.layer as usize])).collect();
        let nt: Vec<_> = new.texts.iter().map(|t| tk(t, t.layer)).collect();
        let (removed_texts, added_texts) = diff_keys(&ot, &nt);

        let fp_dirty = |b: &Board, mask: &[bool]| {
            let mut dirty = vec![false; b.footprints.len()];
            for (p, &m) in b.pads.iter().zip(mask) {
                if m && let Some(d) = dirty.get_mut(p.footprint as usize) {
                    *d = true;
                }
            }
            dirty
        };
        let (old_dirty, new_dirty) = (fp_dirty(old, &removed_pads), fp_dirty(new, &added_pads));
        let fp_keys = |fps: &'a [Footprint]| {
            let mut seen = HashMap::<&'a str, u32>::new();
            fps.iter()
                .enumerate()
                .map(|(i, f)| {
                    let occ = seen.entry(f.reference.as_str()).or_default();
                    let k = (f.reference.as_str(), *occ);
                    *occ += 1;
                    (k, i)
                })
                .collect::<Vec<_>>()
        };
        let (old_fk, new_fk) = (fp_keys(&old.footprints), fp_keys(&new.footprints));
        let old_map: HashMap<_, _> = old_fk.iter().copied().collect();
        let new_map: HashMap<_, _> = new_fk.iter().copied().collect();
        let (mut changes, mut fp_boxes) = (vec![], vec![]);
        let fp_name = |r: &'a str| if r.is_empty() { tr("<unnamed>") } else { r };
        for &(k, ni) in &new_fk {
            let nf = &new.footprints[ni];
            match old_map.get(&k) {
                None => {
                    fp_boxes.push(nf.bounds);
                    changes.push(Change {
                        kind: ChangeKind::Added,
                        item: "footprint",
                        what: trf("+ footprint {0}", &[&fp_name(&nf.reference)]),
                        bounds: nf.bounds,
                    });
                }
                Some(&oi) if old_dirty[oi] || new_dirty[ni] || old.footprints[oi].bounds != nf.bounds => {
                    let of = &old.footprints[oi];
                    fp_boxes.extend([of.bounds, nf.bounds]);
                    let mid = |(a, b): (Pt, Pt)| ((a.x as i64 + b.x as i64) / 2, (a.y as i64 + b.y as i64) / 2);
                    let ((ox, oy), (nx, ny)) = (mid(of.bounds), mid(nf.bounds));
                    let moved = ((nx - ox) as f64).hypot((ny - oy) as f64);
                    let name = fp_name(&nf.reference);
                    let what = match moved >= 1_000.0 {
                        true => trf("~ footprint {0} (moved {1} mm)", &[&name, &format!("{:.2}", moved / 1e6)]),
                        false => trf("~ footprint {0} (pads changed)", &[&name]),
                    };
                    changes.push(Change { kind: ChangeKind::Changed, item: "footprint", what, bounds: union(of.bounds, nf.bounds) });
                }
                _ => {}
            }
        }
        for &(k, oi) in &old_fk {
            if !new_map.contains_key(&k) {
                let of = &old.footprints[oi];
                fp_boxes.push(of.bounds);
                changes.push(Change {
                    kind: ChangeKind::Removed,
                    item: "footprint",
                    what: trf("- footprint {0}", &[&fp_name(&of.reference)]),
                    bounds: of.bounds,
                });
            }
        }
        for (b, mask, kind, sign) in [(new, &added_tracks, ChangeKind::Added, "+"), (old, &removed_tracks, ChangeKind::Removed, "-")] {
            for (t, &m) in b.tracks.iter().zip(mask) {
                if m {
                    let bounds = [t.a, t.b].into_iter().chain(t.mid).map(|p| square(p, t.width / 2)).reduce(union).unwrap();
                    let (n, l) = (&b.nets[t.net as usize], &b.layers[t.layer as usize]);
                    let what = if n.is_empty() { trf("{0} track ({1})", &[&sign, l]) } else { trf("{0} track {1} ({2})", &[&sign, n, l]) };
                    changes.push(Change { kind, item: "track", what, bounds });
                }
            }
        }
        for (b, mask, kind, sign) in [(new, &added_vias, ChangeKind::Added, "+"), (old, &removed_vias, ChangeKind::Removed, "-")] {
            for (v, &m) in b.vias.iter().zip(mask) {
                if m {
                    let n = &b.nets[v.net as usize];
                    let what = if n.is_empty() { trf("{0} via", &[&sign]) } else { trf("{0} via {1}", &[&sign, n]) };
                    changes.push(Change { kind, item: "via", what, bounds: square(v.at, v.size / 2) });
                }
            }
        }
        for (b, mask, kind, sign) in [(new, &added_zones, ChangeKind::Added, "+"), (old, &removed_zones, ChangeKind::Removed, "-")] {
            for (z, &m) in b.fills.iter().zip(mask) {
                if m && let Some(bounds) = z.pts.iter().map(|&p| square(p, 0)).reduce(union) {
                    let (n, l) = (&b.nets[z.net as usize], &b.layers[z.layer as usize]);
                    let what = if n.is_empty() { trf("{0} zone ({1})", &[&sign, l]) } else { trf("{0} zone {1} ({2})", &[&sign, n, l]) };
                    changes.push(Change { kind, item: "zone", what, bounds });
                }
            }
        }
        let in_fp = |(lo, hi): (Pt, Pt)| {
            let p = Pt { x: ((lo.x as i64 + hi.x as i64) / 2) as Nm, y: ((lo.y as i64 + hi.y as i64) / 2) as Nm };
            fp_boxes.iter().any(|&(a, b)| p.x >= a.x - 3 * MM && p.x <= b.x + 3 * MM && p.y >= a.y - 3 * MM && p.y <= b.y + 3 * MM)
        };
        for (b, mask, kind, sign) in [(new, &added_shapes, ChangeKind::Added, "+"), (old, &removed_shapes, ChangeKind::Removed, "-")] {
            for (s, &m) in b.shapes.iter().zip(mask) {
                if m {
                    let bounds = shape_bounds(s);
                    let l = &b.layers[s.layer as usize];
                    if l == "Edge.Cuts" || !in_fp(bounds) {
                        changes.push(Change { kind, item: "shape", what: trf("{0} shape ({1})", &[&sign, l]), bounds });
                    }
                }
            }
        }
        for (b, mask, kind, sign) in [(new, &added_texts, ChangeKind::Added, "+"), (old, &removed_texts, ChangeKind::Removed, "-")] {
            for (t, &m) in b.texts.iter().zip(mask) {
                let bounds = square(t.at, t.size.0.max(t.size.1));
                if m && !in_fp(bounds) {
                    changes.push(Change {
                        kind,
                        item: "text",
                        what: trf("{0} text {1} ({2})", &[&sign, &format!("{:?}", t.text), &b.layers[t.layer as usize]]),
                        bounds,
                    });
                }
            }
        }
        Diff {
            layer_map,
            removed_tracks,
            added_tracks,
            removed_vias,
            added_vias,
            removed_pads,
            added_pads,
            removed_shapes,
            added_shapes,
            removed_zones,
            added_zones,
            removed_texts,
            added_texts,
            changes,
        }
    }
}

fn diff_keys<T: Eq + std::hash::Hash>(old: &[T], new: &[T]) -> (Vec<bool>, Vec<bool>) {
    let p = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let s = old[p..].iter().rev().zip(new[p..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (om, nm) = (&old[p..old.len() - s], &new[p..new.len() - s]);
    let (mut removed, mut added) = (vec![false; old.len()], vec![false; new.len()]);
    if !om.is_empty() || !nm.is_empty() {
        let mut keep = HashMap::<&T, usize>::new();
        for x in om {
            *keep.entry(x).or_default() += 1;
        }
        for (i, x) in nm.iter().enumerate() {
            match keep.get_mut(x) {
                Some(n) if *n > 0 => *n -= 1,
                _ => added[p + i] = true,
            }
        }
        for (i, x) in om.iter().enumerate().rev() {
            if let Some(n) = keep.get_mut(x)
                && *n > 0
            {
                *n -= 1;
                removed[p + i] = true;
            }
        }
    }
    (removed, added)
}

fn shape_bounds(sh: &Shape) -> (Pt, Pt) {
    let r = sh.width / 2;
    match &sh.geom {
        Geom::Segment { a, b } => union(square(*a, r), square(*b, r)),
        Geom::Arc { start, mid, end } => union(union(square(*start, r), square(*mid, r)), square(*end, r)),
        Geom::Circle { center, radius } => square(*center, radius + r),
        Geom::Poly(pts) => pts.iter().map(|&p| square(p, r)).reduce(union).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_footprints_tracks_vias_and_remapped_nets() {
        let old = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user))
              (net 0 "") (net 1 "GND") (net 2 "VCC")
              (footprint "R" (layer "F.Cu") (at 10 20)
                (property "Reference" "R1" (at 0 0) (layer "F.SilkS"))
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "GND")))
              (footprint "R" (layer "F.Cu") (at 30 20)
                (property "Reference" "R2" (at 0 0) (layer "F.SilkS"))
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 2 "VCC")))
              (segment (start 0 0) (end 5 0) (width 0.2) (layer "F.Cu") (net 1))
              (segment (start 5 0) (end 10 0) (width 0.2) (layer "F.Cu") (net 2)))"#,
        )
        .unwrap();
        // Net codes swapped (VCC=1, GND=2), R1 moved by 2 mm, R2 removed, R3 added, second track replaced by a via.
        let new = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user))
              (net 0 "") (net 1 "VCC") (net 2 "GND")
              (footprint "R" (layer "F.Cu") (at 12 20)
                (property "Reference" "R1" (at 0 0) (layer "F.SilkS"))
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 2 "GND")))
              (footprint "R" (layer "F.Cu") (at 40 20)
                (property "Reference" "R3" (at 0 0) (layer "F.SilkS"))
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "VCC")))
              (segment (start 0 0) (end 5 0) (width 0.2) (layer "F.Cu") (net 2))
              (via (at 5 0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 2)))"#,
        )
        .unwrap();
        let d = Diff::between(&old, &new);
        // The first GND track is unchanged despite its net code changing from 1 to 2.
        assert_eq!((&d.removed_tracks[..], &d.added_tracks[..]), (&[false, true][..], &[false][..]));
        assert_eq!((&d.removed_vias[..], &d.added_vias[..]), (&[][..], &[true][..]));
        let whats: Vec<_> = d.changes.iter().map(|c| c.what.as_str()).collect();
        assert_eq!(whats, ["~ footprint R1 (moved 2.00 mm)", "+ footprint R3", "- footprint R2", "- track VCC (F.Cu)", "+ via GND"]);
    }
}
