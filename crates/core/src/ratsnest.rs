//! What is left to route: each net's copper in islands (pads, tracks, vias and zone fills that
//! touch on a copper layer they share), and the shortest lines that would join its islands.

use crate::board::{Board, LayerMask, NetId, Pt};
use crate::drc::{Item, RBox, RTree, arc_pts};
use crate::history::Patch;
use crate::tr::trf;
use std::collections::BTreeMap;

/// A connection still to draw, on net `net`, between the nearest pad centres, via centres or
/// track ends of two islands of its copper: `a` of item `ends[0]`, `b` of `ends[1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub net: NetId,
    pub a: Pt,
    pub b: Pt,
    pub ends: [Item; 2],
}

impl Link {
    /// Its length in mm.
    pub fn mm(&self) -> f64 {
        ((self.b.x - self.a.x) as f64).hypot((self.b.y - self.a.y) as f64) / 1e6
    }

    /// What it joins, named as design rule checks name them: `GND: C1 pad 2 – Via`.
    pub fn what(&self, b: &Board) -> String {
        let name = |i: Item| match i {
            Item::Pad(k) => trf("{0} pad {1}{2}", &[&b.footprints[b.pads[k].footprint as usize].reference, &b.pads[k].number, &""]),
            Item::Via(_) => trf("Via{0}", &[&""]),
            Item::Track(k) => trf("Track{0} on {1}", &[&"", &b.layers[b.tracks[k].layer as usize]]),
        };
        format!("{}: {} – {}", b.nets[self.net as usize], name(self.ends[0]), name(self.ends[1]))
    }
}

/// A piece of copper: its net, layers, and the points a ratsnest line may start from.
struct Cu {
    net: NetId,
    layers: LayerMask,
    item: Item,
    anchors: Vec<Pt>,
}

impl Board {
    /// The ratsnest: for each net, the lines of a minimum spanning tree joining its islands of
    /// copper, longest last; empty when every net is joined.
    pub fn ratsnest(&self) -> Vec<Link> {
        sorted(self.links(|_| true))
    }

    /// The ratsnest's lines of the nets `keep` keeps, in no set order.
    fn links(&self, keep: impl Fn(NetId) -> bool + Sync) -> Vec<Link> {
        let mut cu: Vec<Cu> = vec![];
        // Each shape, with the piece it belongs to, in an R-tree of their boxes.
        let mut shapes: Vec<(usize, RBox)> = vec![];
        let mine = |net: NetId, layers: LayerMask| net != 0 && layers & self.copper != 0 && keep(net);
        let mut add = |net, layers: LayerMask, item, anchors, s: Vec<RBox>| {
            shapes.extend(s.into_iter().map(|s| (cu.len(), s)));
            cu.push(Cu { net, layers: layers & self.copper, item, anchors });
        };
        for (k, p) in self.pads.iter().enumerate().filter(|(_, p)| mine(p.net, p.layers)) {
            add(p.net, p.layers, Item::Pad(k), vec![p.at], vec![RBox::pad(p)]);
        }
        for (k, v) in self.vias.iter().enumerate().filter(|(_, v)| mine(v.net, v.layers)) {
            add(v.net, v.layers, Item::Via(k), vec![v.at], vec![RBox::disc(v.at, v.size / 2)]);
        }
        for (k, t) in self.tracks.iter().enumerate().filter(|(_, t)| mine(t.net, 1 << t.layer)) {
            let pts = t.mid.map_or_else(|| vec![t.a, t.b], |m| arc_pts(t.a, m, t.b));
            let segments = pts.windows(2).map(|w| RBox::segment(w[0], w[1], t.width / 2)).collect();
            add(t.net, 1 << t.layer, Item::Track(k), vec![t.a, t.b], segments);
        }
        let tree = RTree::new(shapes.iter().enumerate().map(|(k, (_, s))| (s.bounds, k as u32)).collect());
        let mut uf: Vec<usize> = (0..cu.len() + self.fills.len()).collect();
        fn find(uf: &mut [usize], mut x: usize) -> usize {
            while uf[x] != x {
                uf[x] = uf[uf[x]];
                x = uf[x];
            }
            x
        }
        // An island's root is its first piece, so islands keep their order whichever nets are joined.
        let join = |uf: &mut Vec<usize>, a: usize, b: usize| {
            let (ra, rb) = (find(uf, a), find(uf, b));
            uf[ra.max(rb)] = ra.min(rb);
        };
        for &(i, s) in &shapes {
            tree.query(s.bounds, |k| {
                let (o, t) = shapes[k as usize];
                if o > i && cu[o].net == cu[i].net && cu[o].layers & cu[i].layers != 0 && s.dist(&t) == 0 {
                    join(&mut uf, i, o);
                }
            });
        }
        // A zone fill joins the copper of its net it touches on its layer, and the fills of its
        // net's other zones there: where one of its edges near them reaches them, or their centre
        // (a fill's first corner) is inside it. Its edges are in an R-tree, so a fill of thousands
        // of corners is asked only near them. A zone's own fills never meet: KiCad joins them.
        let boxes: Vec<(Pt, Pt)> = self.fills.iter().map(|f| bounds(&f.pts)).collect();
        let mut zone = vec![None; self.fills.len()];
        for (i, z) in self.zones.iter().enumerate() {
            zone[z.fills.clone()].fill(Some(i));
        }
        let fills: Vec<usize> =
            (0..self.fills.len()).filter(|&z| self.fills[z].net != 0 && keep(self.fills[z].net) && self.fills[z].pts.len() > 2).collect();
        let touched = crate::threads::par_map(&fills, |&z| {
            let (fill, (lo, hi)) = (&self.fills[z], boxes[z]);
            let ring = |k: usize| (fill.pts[k], fill.pts[(k + 1) % fill.pts.len()]);
            let edges = RTree::new((0..fill.pts.len()).map(|k| (bounds(&[ring(k).0, ring(k).1]), k as u32)).collect());
            let touches = |s: &RBox| {
                let mut hit = false;
                edges.query(s.bounds, |e| hit |= RBox::segment(ring(e as usize).0, ring(e as usize).1, 0).dist(s) == 0);
                hit
            };
            // Even-odd: edges crossing the ray from (x, y) to the right.
            let inside = |x: f64, y: f64| {
                let mut crossings = 0;
                edges.query((Pt { x: x as i32, y: y as i32 }, Pt { x: hi.x, y: y as i32 }), |e| {
                    let (a, b) = ring(e as usize);
                    let ((ax, ay), (bx, by)) = ((a.x as f64, a.y as f64), (b.x as f64, b.y as f64));
                    if (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay) {
                        crossings += 1;
                    }
                });
                crossings % 2 == 1
            };
            let mut joined = vec![];
            tree.query((lo, hi), |k| {
                let (o, t) = shapes[k as usize];
                if cu[o].net == fill.net && cu[o].layers & 1 << fill.layer != 0 && (touches(&t) || inside(t.cx, t.cy)) {
                    joined.push(o);
                }
            });
            for &y in &fills {
                let (other, (olo, ohi)) = (&self.fills[y], boxes[y]);
                let near = olo.x <= hi.x && lo.x <= ohi.x && olo.y <= hi.y && lo.y <= ohi.y;
                let apart = zone[y].is_none() || zone[y] != zone[z];
                if y != z && other.net == fill.net && other.layer == fill.layer && near && apart {
                    // Edges meet edges once a pair: asked of the bigger fill, the smaller's edges.
                    let smaller = (other.pts.len(), y) < (fill.pts.len(), z);
                    let edge = |k: usize| RBox::segment(other.pts[k], other.pts[(k + 1) % other.pts.len()], 0);
                    if inside(other.pts[0].x as f64, other.pts[0].y as f64) || smaller && (0..other.pts.len()).any(|k| touches(&edge(k))) {
                        joined.push(cu.len() + y);
                    }
                }
            }
            joined
        });
        for (&z, joined) in fills.iter().zip(touched) {
            for o in joined {
                join(&mut uf, cu.len() + z, o);
            }
        }
        // In a set order, so the same board gives the same lines.
        let mut nets: BTreeMap<NetId, BTreeMap<usize, Vec<(Pt, Item)>>> = BTreeMap::new();
        for (i, c) in cu.iter().enumerate() {
            let root = find(&mut uf, i);
            nets.entry(c.net).or_default().entry(root).or_default().extend(c.anchors.iter().map(|&p| (p, c.item)));
        }
        let mut links: Vec<Link> = vec![];
        for (net, islands) in nets.into_iter().filter(|(_, i)| i.len() > 1) {
            let islands: Vec<Vec<(Pt, Item)>> = islands.into_values().collect();
            links.extend(spanning(&islands).into_iter().map(|((a, i), (b, j))| Link { net, a, b, ends: [i, j] }));
        }
        links
    }
}

/// Brings `old`, the board's ratsnest before a change, up to date after it, given `undo`, the
/// patch that undoes the change: only the nets of the copper and zone fills it put in, changed
/// or took away are joined again (every net, when it changed the nets); the others' lines follow
/// their items to their new indices.
pub fn recheck(b: &Board, mut old: Vec<Link>, undo: &Patch) -> Vec<Link> {
    let Some(nets) = undo.nets(b) else { return b.ratsnest() };
    old.retain_mut(|l| !nets.contains(&l.net) && l.ends.iter_mut().all(|i| undo.moved(*i).map(|m| *i = m).is_some()));
    old.extend(b.links(|n| nets.contains(&n)));
    sorted(old)
}

/// The lines shortest first, in a set order, so the same board gives the same list.
fn sorted(mut links: Vec<Link>) -> Vec<Link> {
    let key = |l: &Link| (l.net, l.a.x, l.a.y, l.b.x, l.b.y);
    links.sort_by(|x, y| x.mm().total_cmp(&y.mm()).then(key(x).cmp(&key(y))));
    links
}

/// The box around `pts`.
fn bounds(pts: &[Pt]) -> (Pt, Pt) {
    let first = (Pt { x: i32::MAX, y: i32::MAX }, Pt { x: i32::MIN, y: i32::MIN });
    pts.iter().fold(first, |(lo, hi), p| (Pt { x: lo.x.min(p.x), y: lo.y.min(p.y) }, Pt { x: hi.x.max(p.x), y: hi.y.max(p.y) }))
}

/// The shortest lines joining every one of `islands` (each a set of points, with the item each
/// is of): a minimum spanning tree over them (Prim's), each line between the nearest points of
/// two islands.
fn spanning(islands: &[Vec<(Pt, Item)>]) -> Vec<((Pt, Item), (Pt, Item))> {
    let n = islands.len();
    let d2 = |a: Pt, b: Pt| ((a.x - b.x) as i64).pow(2) + ((a.y - b.y) as i64).pow(2);
    let far = (i64::MAX, (Pt::default(), Item::Pad(0)), (Pt::default(), Item::Pad(0)));
    let nearest = |a: &[(Pt, Item)], b: &[(Pt, Item)]| {
        a.iter().flat_map(|&p| b.iter().map(move |&q| (d2(p.0, q.0), p, q))).min_by_key(|c| c.0).unwrap_or(far)
    };
    let mut joined = vec![false; n];
    let mut best: Vec<_> = (0..n).map(|i| if i == 0 { far } else { nearest(&islands[0], &islands[i]) }).collect();
    let mut out = vec![];
    if n > 0 {
        joined[0] = true;
    }
    for _ in 1..n {
        let Some(k) = (0..n).filter(|&i| !joined[i]).min_by_key(|&i| best[i].0) else { break };
        joined[k] = true;
        out.push((best[k].1, best[k].2));
        for i in (0..n).filter(|&i| !joined[i]) {
            let c = nearest(&islands[k], &islands[i]);
            if c.0 < best[i].0 {
                best[i] = c;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(items: &str) -> Board {
        Board::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "B")
              (footprint "R" (at 0 0) (property "Reference" "R1")
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 10 0) (size 1 1) (layers "F.Cu") (net 1 "A"))
                (pad "3" smd rect (at 10 10) (size 1 1) (layers "F.Cu") (net 1 "A"))
                (pad "4" smd rect (at 0 10) (size 1 1) (layers "B.Cu") (net 2 "B")))
              {items})"#
        ))
        .unwrap()
    }

    #[test]
    fn joins_a_nets_islands_by_the_shortest_lines() {
        let b = board("");
        let links = b.ratsnest();
        // Net A's three pads: two lines of 10 mm, not the 14 mm diagonal; net B's lone pad none.
        assert_eq!(links.len(), 2);
        assert!(links.iter().all(|l| l.net == 1 && (l.mm() - 10.0).abs() < 1e-9));
        assert_eq!(links[0].what(&b), "A: R1 pad 1 – R1 pad 2");
    }

    #[test]
    fn tracks_vias_and_zones_join_copper() {
        let track = r#"(segment (start 0 0) (end 10 0) (width 0.25) (layer "F.Cu") (net 1))"#;
        assert_eq!(board(track).ratsnest().len(), 1);
        // On the other layer, a track joins nothing and is an island of its own, as KiCad's
        // ratsnest has it; a via down and back up joins it.
        let below = r#"(segment (start 10 0) (end 10 10) (width 0.25) (layer "B.Cu") (net 1))"#;
        assert_eq!(board(&format!("{track} {below}")).ratsnest().len(), 2);
        let vias = r#"(via (at 10 0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1)) (via (at 10 10) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1))"#;
        assert_eq!(board(&format!("{track} {below} {vias}")).ratsnest(), vec![]);
        // A filled zone touching every pad of the net joins them.
        let zone = r#"(zone (net 1) (net_name "A") (layer "F.Cu") (hatch edge 0.5) (connect_pads yes (clearance 0)) (min_thickness 0.25)
            (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5)) (polygon (pts (xy -1 -1) (xy 11 -1) (xy 11 11) (xy -1 11)))
            (filled_polygon (layer "F.Cu") (pts (xy -1 -1) (xy 11 -1) (xy 11 11) (xy -1 11))))"#;
        assert_eq!(board(zone).ratsnest(), vec![]);
        // Two fills meeting at an edge, each on its own pads, join them.
        let fill = |x0, x1| {
            format!(
                r#"(zone (net 1) (net_name "A") (layer "F.Cu") (hatch edge 0.5) (polygon (pts (xy {x0} -1) (xy {x1} -1) (xy {x1} 11) (xy {x0} 11)))
            (filled_polygon (layer "F.Cu") (pts (xy {x0} -1) (xy {x1} -1) (xy {x1} 11) (xy {x0} 11))))"#
            )
        };
        assert_eq!(board(&fill(-1, 5)).ratsnest().len(), 2);
        assert_eq!(board(&format!("{} {}", fill(-1, 5), fill(5, 11))).ratsnest(), vec![]);
    }

    #[test]
    fn joins_a_net_alone_as_among_all() {
        // Net A: a pad with a track up and one left, and a track apart; net B's vias between the
        // two tracks' left ends change the box tree, so the order A's pieces join in, when every
        // net is joined, from when A is alone (as after an edit of its track apart).
        let vias: String =
            (0..200).map(|k| format!(r#"(via (at -3 {}) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 2))"#, 50 + 2 * k)).collect();
        let b = Board::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "B")
              (footprint "R" (at 0 0) (property "Reference" "R1") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
              (segment (start 0 0) (end 0 -5) (width 0.25) (layer "F.Cu") (net 1))
              (segment (start 20 20) (end 25 20) (width 0.25) (layer "F.Cu") (net 1))
              (segment (start 0 0) (end -5 0) (width 0.25) (layer "F.Cu") (net 1)) {vias})"#
        ))
        .unwrap();
        let all = b.ratsnest();
        assert_eq!(all.iter().find(|l| l.net == 1).map(|l| l.ends), Some([Item::Pad(0), Item::Track(1)]));
        assert!(recheck(&b, all.clone(), &Patch::item(&b, crate::history::Pick::Track(1))) == all);
    }

    #[test]
    fn an_edit_joins_again_only_the_nets_it_touched() {
        let mut b = board("");
        let before = b.ratsnest();
        // A track from pad 1 to pad 2 of net A leaves one line of it.
        let undo = Patch::added(&b);
        b.tracks.push(crate::board::Track {
            a: Pt { x: 0, y: 0 },
            b: Pt { x: 10_000_000, y: 0 },
            mid: None,
            width: 250_000,
            layer: 0,
            net: 1,
            locked: false,
        });
        let after = recheck(&b, before.clone(), &undo);
        assert_eq!((after.len(), &after), (1, &b.ratsnest()));
        // Undone, the lines are as before.
        let redo = b.apply(undo);
        assert_eq!(recheck(&b, after, &redo), before);
        // Deleting net B's track moves net A's after it: its line follows it.
        let tracks = r#"(segment (start 0 10) (end 5 10) (width 0.25) (layer "B.Cu") (net 2))
            (segment (start 10 10) (end 20 10) (width 0.25) (layer "B.Cu") (net 1))"#;
        let mut b = board(tracks);
        let before = b.ratsnest();
        assert!(before.iter().any(|l| l.ends.contains(&Item::Track(1))));
        let undo = b.apply(Patch::delete(&b, crate::history::Pick::Track(0)));
        assert_eq!(recheck(&b, before, &undo), b.ratsnest());
    }
}
