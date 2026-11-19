//! Zone filling: each copper zone's area on each of its layers, keeping clear of other nets,
//! holes, the board edge, keepouts and higher-priority zones, with thermal reliefs, the minimum
//! width and island removal. Zones of one priority fill in parallel.

use crate::board::{
    Board, Connect, Geom, LayerId, LayerMask, MAX_ERROR, MM, NetId, Nm, Pad, Pt, Smoothing, Zone, ZoneFill, arc_points, segments, union,
};
use crate::drc::{RBox, RTree, overlaps, pad_box};
use crate::font;
use crate::history::Patch;
use crate::polygon::{Contour, Shape, fracture, grow, ip, normalize, outline, overlay, path, rounded};
use crate::save::{KICAD6, new_id};
use crate::threads::par_map;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::{Overlay, ShapeType};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::core::point_location::IntPointContainment;
use i_overlay::core::solver::Solver;
use i_overlay::i_float::int::point::IntPoint;
use i_overlay::i_shape::int::area::Area;
use std::cmp::Reverse;

mod refill;

/// How far a thermal spoke reaches past its relief gap, to join the fill.
const OVERLAP: f64 = 3.0 * MAX_ERROR;

impl Board {
    /// Fills every copper zone again from its outline and the copper around it; returns how
    /// many zones' fills changed.
    // shortcut: hatched zones, and zones on no copper layer, keep the fill they had; fill them
    // when a board needs it.
    pub fn fill_zones(&mut self) -> usize {
        let (new, old, mut changed) = (fill(self, None, &[]), std::mem::take(&mut self.fills), 0);
        for (z, new) in self.zones.iter_mut().zip(new) {
            let (start, was) = (self.fills.len(), &old[z.fills.clone()]);
            match new {
                Some(new) => {
                    changed += usize::from(new[..] != *was);
                    self.fills.extend(new);
                }
                None => self.fills.extend_from_slice(was),
            }
            z.fills = start..self.fills.len();
        }
        changed
    }

    /// Adds a copper zone for `net` (0: none) on `layer` inside `corners`, with KiCad's
    /// settings for a new zone (the board's zone clearance), and fills it alone, keeping out of
    /// the other zones' fills (`keeps_out`) as they are; returns its index.
    pub fn add_zone(&mut self, corners: Vec<Pt>, layer: LayerId, net: NetId) -> usize {
        let (i, start) = (self.zones.len(), self.fills.len());
        // Below every zone's id, it ranks last (as a refill, also KiCad's, would fill it); KiCad
        // 4–5 ids take 32 bits.
        let id = match self.zones.iter().map(|z| z.id).min() {
            Some(least) => least.saturating_sub(1),
            None => new_id((&corners, layer, net)) & if self.version < KICAD6 { u32::MAX.into() } else { u128::MAX },
        };
        self.zones.push(Zone {
            outline: vec![corners],
            layers: 1 << layer,
            net,
            priority: 0,
            id,
            keepout: false,
            name: String::new(),
            clearance: self.rules.zone_clearance,
            min_width: MM / 4,
            connect: Connect::Thermal,
            thermal_gap: MM / 2,
            spoke_width: MM / 2,
            island_min: i64::MAX,
            smoothing: Smoothing::None,
            hatched: false,
            // KiCad 5 draws every fill with an outline (`save-zones.md`).
            outlined: self.version < KICAD6,
            fills: start..start,
            src: None,
        });
        let fills = fill(self, Some(&one(self, i)), &[]).pop().flatten().unwrap_or_default();
        self.fills.extend(fills);
        self.zones[i].fills.end = self.fills.len();
        i
    }

    /// Puts `z`, zone `i` edited (its outline or settings), in its place, filled alone as a new
    /// zone is (`add_zone`); returns the patch that undoes it.
    pub fn set_zone(&mut self, i: usize, z: Zone) -> Patch {
        let (net, old) = (z.net, std::mem::replace(&mut self.zones[i], z));
        let kept = |f: &ZoneFill| ZoneFill { net, ..f.clone() };
        let fills = fill(self, Some(&one(self, i)), &[]).swap_remove(i);
        let fills = fills.unwrap_or_else(|| self.fills[old.fills.clone()].iter().map(kept).collect());
        let (at, len) = (old.fills.start, old.fills.len());
        let z = Zone { fills: at..at + fills.len(), ..std::mem::replace(&mut self.zones[i], old) };
        self.apply(Patch::zone(i, z, at..at + len, fills))
    }
}

/// The box around `pts`; `None` when there are none.
fn bounds(pts: &[Pt]) -> Option<(Pt, Pt)> {
    pts.iter().map(|&p| (p, p)).reduce(union)
}

/// Zone `i` alone, as a mask of the board's zones' layers.
fn one(b: &Board, i: usize) -> Vec<LayerMask> {
    (0..b.zones.len()).map(|z| if z == i { LayerMask::MAX } else { 0 }).collect()
}

/// Each zone's new fill, `None` for the zones that keep theirs; with `only`, the zones on the
/// layers it marks fill alone, keeping out of the others' fills they keep out of (`keeps_out`)
/// as they are. With `changes` (where copper changed, on which layers) too, only those near
/// them on their layers and the zones keeping out of those fills, each near what changed and
/// where those fills did (all of it, where that is much of it: `Ctx::window`).
fn fill(b: &Board, only: Option<&[LayerMask]>, changes: &[(LayerMask, (Pt, Pt))]) -> Vec<Option<Vec<ZoneFill>>> {
    let allowed = |z: usize, l: usize| {
        let zone = &b.zones[z];
        !zone.keepout && !zone.hatched && zone.layers & b.copper & 1 << l != 0 && only.is_none_or(|o| o[z] & 1 << l != 0)
    };
    let pads = pad_clearance(b);
    let changed = |z: usize, l: usize| {
        let at = b.zones[z].outline.first().and_then(|o| bounds(o)).map(|o| pad_box(o, reach(b, z, pads)));
        changes.is_empty() || changes.iter().any(|&(on, c)| on & 1 << l != 0 && at.is_some_and(|at| overlaps(at, c)))
    };
    let all = (0..b.zones.len()).flat_map(|z| (0..128).map(move |l| (z, l)));
    let mut jobs: Vec<(usize, LayerId)> = all.filter(|&(z, l)| allowed(z, l) && changed(z, l)).map(|(z, l)| (z, l as LayerId)).collect();
    if jobs.is_empty() {
        return (0..b.zones.len()).map(|_| None).collect();
    }
    let cx = Ctx::new(b);
    let boxes: Vec<_> = cx.areas.iter().map(|a| shapes_box(a).map(pt_box)).collect();
    let near = |z: usize, y: usize| {
        let by = cx.apart(z, y);
        boxes[z].zip(boxes[y]).is_some_and(|(a, c)| overlaps(pad_box(a, by as Nm + 1), c))
            && !overlay(&grow(&cx.areas[y], by), &cx.areas[z], OverlayRule::Intersect).is_empty()
    };
    // Near what changed, the zones keeping out of one filled, near it, may change too.
    let mut k = 0;
    while k < jobs.len() && !changes.is_empty() {
        let (x, l) = jobs[k];
        let next: Vec<_> = (0..b.zones.len())
            .filter(|&y| allowed(y, l as usize) && keeps_out(b, y, x) && !jobs.contains(&(y, l)) && near(y, x))
            .map(|y| (y, l))
            .collect();
        jobs.extend(next);
        k += 1;
    }
    // A zone keeps out of some fills near it (`keeps_out`), so it fills after them; as late as
    // the zones filling after it allow, so the zones nothing waits for (the big planes) fill
    // last, together.
    jobs.sort_by_key(|&(z, l)| (Reverse(rank(b, z)), l));
    let (mut after, mut wave) = (vec![vec![]; jobs.len()], vec![0; jobs.len()]);
    for i in (0..jobs.len()).rev() {
        let (y, l) = jobs[i];
        for k in (i + 1..jobs.len()).filter(|&k| jobs[k].1 == l && keeps_out(b, jobs[k].0, y) && near(jobs[k].0, y)) {
            after[k].push(i);
            wave[i] = wave[i].max(wave[k] + 1);
        }
    }
    let job = |z: usize, l: LayerId| jobs.iter().position(|&j| j == (z, l));
    // With `only`, the fills of the others near a zone it fills that keeps out of them, as they are.
    let mut kept = vec![];
    for zone in (0..b.zones.len()).filter(|_| only.is_some()) {
        for f in b.fills[b.zones[zone].fills.clone()].iter().filter(|f| job(zone, f.layer).is_none()) {
            let fill_box = bounds(&f.pts);
            let near = |o: usize| boxes[o].zip(fill_box).is_some_and(|(a, c)| overlaps(pad_box(a, cx.apart(o, zone) as Nm + 1), c));
            if jobs.iter().any(|&(o, l)| l == f.layer && keeps_out(b, o, zone) && near(o)) {
                kept.push(Done::kept(zone, f));
            }
        }
    }
    // Within a wave, the biggest first, so the threads finish together.
    let size = |z: usize| boxes[z].map_or(0, |(lo, hi)| (hi.x as i64 - lo.x as i64) * (hi.y as i64 - lo.y as i64));
    let mut order: Vec<usize> = (0..jobs.len()).collect();
    order.sort_by_key(|&i| (Reverse(wave[i]), Reverse(size(jobs[i].0)), jobs[i]));
    let mut done: Vec<Option<Done>> = jobs.iter().map(|_| None).collect();
    for w in order.chunk_by(|&i, &k| wave[i] == wave[k]) {
        // Where each fills: near what changed and where the fills it keeps out of did, filled
        // before it; nowhere (it keeps its fill) when none of those reaches it.
        let at: Vec<Option<Vec<(Pt, Pt)>>> = w
            .iter()
            .map(|&i| {
                let (z, l) = jobs[i];
                let reach = boxes[z].map(|zb| pad_box(zb, cx.reach(z)))?;
                let moved = after[i]
                    .iter()
                    .filter_map(|&k| done[k].as_ref())
                    .flat_map(|d| d.moved.iter().map(|&m| pad_box(m, cx.apart(z, d.zone) as Nm)));
                let here = changes.iter().filter(|&&(on, _)| on & 1 << l != 0).map(|&(_, c)| c).chain(moved);
                Some(here.filter(|&c| overlaps(c, reach)).collect())
            })
            .collect();
        let filled = par_map(&w.iter().zip(at).collect::<Vec<_>>(), |(i, at)| {
            let (z, l) = jobs[**i];
            let old: Vec<&ZoneFill> = b.fills[b.zones[z].fills.clone()].iter().filter(|f| f.layer == l).collect();
            let near = (!changes.is_empty()).then(|| at.as_deref().unwrap_or_default());
            if near.is_some_and(|at| at.is_empty()) {
                let shapes = old.iter().flat_map(|f| Done::kept(z, f).shapes).collect();
                return Done { zone: z, layer: l, shapes, fills: old.iter().map(|f| f.pts.clone()).collect(), moved: vec![] };
            }
            let kept = kept.iter().filter(|d| keeps_out(b, z, d.zone));
            let above: Vec<&Done> = kept.chain(after[**i].iter().filter_map(|&k| done[k].as_ref())).collect();
            let window = near.and_then(|at| cx.window(z, l, at));
            let windowed = window.and_then(|w| cx.refill_layer(z, l, &above, w, old.iter().copied()).map(|f| (f, Some(w.0))));
            let ((shapes, fills), window) = windowed.unwrap_or_else(|| (cx.fill_layer(z, l, &above), None));
            // Where it changed: its window, else the fills it gained or lost.
            let moved = match window {
                _ if fills.iter().eq(old.iter().map(|f| &f.pts)) => vec![],
                Some(w) => vec![w],
                None => {
                    let gone = old.iter().map(|f| &f.pts).filter(|o| !fills.contains(o));
                    gone.chain(fills.iter().filter(|f| !old.iter().any(|o| o.pts == **f))).filter_map(|f| bounds(f)).collect()
                }
            };
            Done { zone: z, layer: l, shapes, fills, moved }
        });
        for (&i, d) in w.iter().zip(filled) {
            done[i] = Some(d);
        }
    }
    // Each zone filled: its layers in order, those it didn't fill as they were.
    (0..b.zones.len())
        .map(|z| {
            let zone = &b.zones[z];
            let layers = (0..128).filter(|&l| zone.layers & 1 << l != 0).map(|l| l as LayerId);
            if !layers.clone().any(|l| job(z, l).is_some()) {
                return None;
            }
            let mut fills = vec![];
            for l in layers {
                match job(z, l).and_then(|i| done[i].take()) {
                    Some(d) => fills.extend(d.fills.into_iter().map(|pts| ZoneFill { pts, layer: l, net: zone.net })),
                    None => fills.extend(b.fills[zone.fills.clone()].iter().filter(|f| f.layer == l).cloned()),
                }
            }
            Some(fills)
        })
        .collect()
}

/// One zone filled on one layer: its areas, each as KiCad saves it, and where it changed.
struct Done {
    zone: usize,
    layer: LayerId,
    shapes: Vec<Shape>,
    fills: Vec<Vec<Pt>>,
    moved: Vec<(Pt, Pt)>,
}

impl Done {
    /// Zone `zone`'s fill `f` as it is, which others keep out of: as saved, its holes joined to
    /// its outline (every use clips it to a window first).
    fn kept(zone: usize, f: &ZoneFill) -> Self {
        Done { zone, layer: f.layer, shapes: vec![vec![f.pts.iter().map(|&p| ip(p)).collect()]], fills: vec![], moved: vec![] }
    }
}

/// What every zone fill shares.
struct Ctx<'a> {
    b: &'a Board,
    /// Each zone's outline, smoothed and kept inside the board (keepouts: as drawn).
    areas: Vec<Vec<Shape>>,
    obs: Vec<Ob>,
    tree: RTree,
    /// The largest clearance of a pad's own, or of a net class.
    pad_clearance: Nm,
}

/// Copper or a hole a zone keeps clear of; its own net's copper joins the zone instead.
struct Ob {
    rb: RBox,
    layers: LayerMask,
    net: NetId,
    kind: Kind,
    /// Where the copper is, for telling which islands touch it.
    at: [Pt; 2],
    /// Its own clearance (a pad's, also for its hole; its net's class's), when larger than the board's.
    clearance: Nm,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Tracks, vias, graphics and text on copper.
    Copper,
    Pad(usize),
    /// A filled polygon on copper: `Board::shapes[i]`.
    Poly(usize),
    /// A hole without copper.
    Hole,
    Edge,
}

impl<'a> Ctx<'a> {
    fn new(b: &'a Board) -> Self {
        let mut obs = vec![];
        // Copper keeps its net's class's clearance, as a pad its own: the larger of them.
        let clearances = b.clearances();
        let mut ob = |rb: RBox, layers, net: NetId, kind, at, own: Nm| {
            let class = if kind == Kind::Hole { 0 } else { clearances.get(net as usize).copied().unwrap_or(0) };
            obs.push(Ob { rb, layers, net, kind, at, clearance: own.max(class) })
        };
        for t in &b.tracks {
            match t.mid {
                None => ob(RBox::segment(t.a, t.b, t.width / 2), 1 << t.layer, t.net, Kind::Copper, [t.a, t.b], 0),
                // The arc's chords lie inside it: wider by the error, they cover it.
                Some(m) => {
                    for w in arc_points(t.a, m, t.b).windows(2) {
                        ob(RBox::segment(w[0], w[1], t.width / 2 + MAX_ERROR as Nm), 1 << t.layer, t.net, Kind::Copper, [w[0], w[1]], 0);
                    }
                }
            }
        }
        // shortcut: vias and pads keep their ring on every layer, also where KiCad removes unused
        // ones (`remove_unused_layers`): fills stay clear but smaller; drop those rings if it matters.
        for v in &b.vias {
            ob(RBox::disc(v.at, v.size / 2), v.layers & b.copper, v.net, Kind::Copper, [v.at; 2], 0);
        }
        for (i, p) in b.pads.iter().enumerate() {
            let own = p.clearance.unwrap_or(0);
            if p.layers & b.copper != 0 {
                // A custom pad counts as its box in its own frame, as in KiCad: its spokes start at
                // the box's centre and cross it. Its copper keeps the clearances.
                ob(RBox::pad(p), p.layers & b.copper, p.net, Kind::Pad(i), [p.at; 2], own);
            }
            // Where the hole has no copper around it.
            if let (Some((dw, dh)), bare @ 1..) = (p.drill, b.copper & !p.layers) {
                ob(RBox::boxed(p.at, (dw, dh), p.angle, dw.min(dh) / 2), bare, 0, Kind::Hole, [p.at; 2], own);
            }
        }
        let edge = b.layers.iter().position(|l| l == "Edge.Cuts");
        let mut edges = vec![];
        for (i, s) in b.shapes.iter().enumerate() {
            let (cu, w) = (b.copper & 1 << s.layer, s.width / 2);
            if Some(s.layer as usize) == edge {
                edges.push((path(&s.geom), matches!(s.geom, Geom::Arc { .. } | Geom::Circle { .. })));
            } else if cu != 0 {
                match &s.geom {
                    Geom::Poly(pts) if s.filled => {
                        let bounds = pts.iter().fold((pts[0], pts[0]), |(lo, hi), &p| {
                            (Pt { x: lo.x.min(p.x), y: lo.y.min(p.y) }, Pt { x: hi.x.max(p.x), y: hi.y.max(p.y) })
                        });
                        let mut rb = RBox::disc(pts[0], 0);
                        rb.bounds = pad_box(bounds, w);
                        ob(rb, cu, 0, Kind::Poly(i), [pts[0]; 2], 0);
                    }
                    Geom::Circle { center, radius } if s.filled => {
                        ob(RBox::disc(*center, radius + w), cu, 0, Kind::Copper, [*center; 2], 0)
                    }
                    g => {
                        let curved = !matches!(g, Geom::Segment { .. } | Geom::Poly(_));
                        let w = w + if curved { MAX_ERROR as Nm } else { 0 };
                        for p in path(g).windows(2) {
                            ob(RBox::segment(p[0], p[1], w), cu, 0, Kind::Copper, [p[0], p[1]], 0);
                        }
                    }
                }
            }
        }
        for t in b.texts.iter().filter(|t| b.copper & 1 << t.layer != 0) {
            font::strokes(t, |pts| {
                for p in pts.windows(2) {
                    ob(RBox::segment(p[0], p[1], t.thickness / 2), 1 << t.layer, 0, Kind::Copper, [p[0], p[1]], 0);
                }
            });
        }
        for (p, curved) in &edges {
            for s in p.windows(2) {
                ob(RBox::segment(s[0], s[1], if *curved { MAX_ERROR as Nm } else { 0 }), b.copper, 0, Kind::Edge, [s[0], s[1]], 0);
            }
        }
        let board = outline(edges.into_iter().map(|e| e.0).collect());
        let areas = b
            .zones
            .iter()
            .map(|z| {
                let area = smooth(normalize(z.outline.iter().map(|c| c.iter().map(|&p| ip(p)).collect()).collect()), z.smoothing);
                match &board {
                    Some(board) if !z.keepout => overlay(&area, board, OverlayRule::Intersect),
                    _ => area,
                }
            })
            .collect();
        let tree = RTree::new(obs.iter().enumerate().map(|(i, o)| (o.rb.bounds, i as u32)).collect());
        Ctx { b, areas, obs, tree, pad_clearance: pad_clearance(b) }
    }

    /// How far zone `z`'s area, shrunk by half its minimum width, keeps from zone `y`'s fill.
    fn apart(&self, z: usize, y: usize) -> f64 {
        let (zz, yy, rules) = (&self.b.zones[z], &self.b.zones[y], &self.b.rules);
        let r = (zz.min_width / 2 - 1).max(0) as f64;
        let class = |z: &Zone| self.b.class(z.net).map_or(0, |c| c.clearance);
        let keeps = zz.clearance.max(rules.clearance).max(yy.clearance).max(class(zz)).max(class(yy));
        if zz.net != 0 && zz.net == yy.net { r } else { keeps as f64 + r + MAX_ERROR }
    }

    fn reach(&self, zi: usize) -> Nm {
        reach(self.b, zi, self.pad_clearance)
    }

    /// Zone `zi`'s fill on layer `l`, keeping out of the fills `above` (`keeps_out`).
    fn fill_layer(&self, zi: usize, l: LayerId, above: &[&Done]) -> (Vec<Shape>, Vec<Vec<Pt>>) {
        let Some(zone_box) = shapes_box(&self.areas[zi]) else { return (vec![], vec![]) };
        let (mut shapes, marks) = self.filled(zi, l, above, pt_box(zone_box), &self.areas[zi]);
        let touches = self.touches(zi, l, &shapes, &marks, above);
        keep(&self.b.zones[zi], &mut shapes, &touches, touches.contains(&true));
        let fills = shapes.iter().map(fracture).collect();
        (shapes, fills)
    }

    /// Zone `zi`'s fill on layer `l` in `area` (its own, or what of it a window holds, in the
    /// box `at`) before islands go, keeping out of the fills `above`, and the points in it of
    /// its net's copper.
    fn filled(&self, zi: usize, l: LayerId, above: &[&Done], at: (Pt, Pt), area: &[Shape]) -> (Vec<Shape>, Vec<IntPoint>) {
        let (b, z, on) = (self.b, &self.b.zones[zi], 1 << l);
        // Every knockout grows by half the minimum width (`r`) as the area shrinks by it; growing
        // what is left by `r` then drops the copper narrower than the minimum width (a channel
        // exactly as wide stays, as in KiCad).
        let (r, gap, rules) = ((z.min_width / 2 - 1).max(0) as f64, z.thermal_gap as f64, &b.rules);
        // How far the fill keeps from `o`: the zone's clearance, its net's class's or `o`'s, the largest.
        let class = b.class(z.net).map_or(0, |c| c.clearance);
        let clearance = |o: &Ob| match o.kind {
            Kind::Edge => rules.edge_clearance as f64,
            Kind::Hole => rules.hole_clearance.max(o.clearance) as f64,
            _ => z.clearance.max(rules.clearance).max(class).max(o.clearance) as f64,
        };
        let (mut cuts, mut reliefs): (Vec<Contour>, Vec<Contour>) = (vec![], vec![]);
        let (mut marks, mut thermal, reach) = (vec![], vec![], self.reach(zi));
        self.tree.query(pad_box(at, reach), |i| {
            let o = &self.obs[i as usize];
            if o.layers & on == 0 {
                return;
            }
            let same = z.net != 0 && o.net == z.net;
            match o.kind {
                Kind::Poly(s) => cuts.extend(grow_poly(&b.shapes[s], clearance(o) + r)),
                Kind::Edge | Kind::Hole => cuts.push(rounded(&o.rb, clearance(o) + r)),
                Kind::Copper if !same => cuts.push(rounded(&o.rb, clearance(o) + r)),
                Kind::Pad(p) if !same => cuts.extend(pad_cut(&o.rb, &b.pads[p], clearance(o) + r)),
                Kind::Copper => marks.extend(o.at.map(ip)),
                Kind::Pad(p) => match (b.pads[p].zone_connect.unwrap_or(z.connect), b.pads[p].drill) {
                    (Connect::Solid, _) | (Connect::ThruHoleThermal, None) => marks.push(ip(o.at[0])),
                    (Connect::None, _) => cuts.extend(pad_cut(&o.rb, &b.pads[p], clearance(o) + r)),
                    _ => {
                        reliefs.extend(pad_cut(&o.rb, &b.pads[p], gap + r));
                        thermal.push((i as usize, p));
                    }
                },
            }
        });
        let near = |shapes: &[Shape], by: Nm| {
            let (lo, hi) = pad_box(at, by);
            shapes_box(shapes).is_some_and(|s| overlaps(pt_box(s), (lo, hi))).then_some((lo, hi))
        };
        for d in above.iter().filter(|d| d.layer == l && keeps_out(b, zi, d.zone)) {
            let c = self.apart(zi, d.zone);
            // A higher fill only matters near this zone: clipped to that first, it grows fast.
            let Some((lo, hi)) = near(&d.shapes, c as Nm + 1) else { continue };
            let window = [vec![vec![ip(lo), IntPoint::new(hi.x, lo.y), ip(hi), IntPoint::new(lo.x, hi.y)]]];
            cuts.extend(grow(&overlay(&d.shapes, &window, OverlayRule::Intersect), c).into_iter().flatten());
        }
        for (k, _) in
            b.zones.iter().enumerate().filter(|&(k, keep)| keep.keepout && keep.layers & on != 0 && near(&self.areas[k], reach).is_some())
        {
            cuts.extend(grow(&self.areas[k], r + MAX_ERROR).into_iter().flatten());
        }
        let area = if r > 0.0 { grow(area, -r) } else { area.to_vec() };
        let cut = |shapes: &[Shape], cuts: &[Contour]| {
            // The fragment solver: 2× faster on planes crossed by many track knockouts.
            let mut o = Overlay::new_custom(shapes.len() + cuts.len(), Default::default(), Solver::FRAG);
            shapes.iter().flatten().for_each(|c| o.add_contour(c, ShapeType::Subject));
            cuts.iter().for_each(|c| o.add_contour(c, ShapeType::Clip));
            o.overlay(OverlayRule::Difference, FillRule::NonZero)
        };
        // A thermal spoke stays where the fill would reach its end without the relief gaps, as
        // KiCad keeps them (the spokes of crowded pads join up), unless it ends on another pad.
        let unrelieved = cut(&area, &cuts);
        let left = if reliefs.is_empty() { unrelieved.clone() } else { cut(&unrelieved, &reliefs) };
        let mut shapes = if r > 0.0 { grow(&left, r) } else { left };

        // Thermal spokes: from the pad (or its hole) across its gap, keeping clear of other nets.
        let mut spokes = vec![];
        for &(i, p) in &thermal {
            for (spoke, outside, end) in self.spokes(zi, i, p) {
                // Only the part off the pad needs room: on it, it is the pad's copper.
                let mut clear = true;
                self.tree.query(pad_box(outside.bounds, reach), |j| {
                    let o = &self.obs[j as usize];
                    let same = z.net != 0 && o.net == z.net && o.kind != Kind::Hole;
                    // shortcut: a custom pad counts as its box here; use its outline if a spoke ends near one.
                    let on_pad = matches!(o.kind, Kind::Pad(_)) && j as usize != i && depth(&o.rb, end) < -MAX_ERROR;
                    let dist = || match o.kind {
                        Kind::Pad(q) if !b.pads[q].custom.is_empty() => outside.dist_poly(&b.pads[q].copper()),
                        _ => outside.dist(&o.rb),
                    };
                    // shortcut: spokes don't check filled copper polygons; check them if one ever crowds a pad.
                    clear &= o.layers & on == 0
                        || (same && !on_pad)
                        || matches!(o.kind, Kind::Poly(_))
                        || (!same && dist() as f64 >= clearance(o));
                });
                if clear {
                    spokes.push((rounded(&spoke, 0.0), ip(end)));
                }
            }
        }
        let reached = unrelieved.contains_points(&spokes.iter().map(|s| s.1).collect::<Vec<_>>());
        let spokes: Vec<_> = spokes.into_iter().zip(reached).filter(|s| s.1).map(|s| s.0).collect();
        if !spokes.is_empty() {
            let mut o = Overlay::new(shapes.len() + spokes.len());
            shapes.iter().flatten().for_each(|c| o.add_contour(c, ShapeType::Subject));
            spokes.iter().for_each(|s| o.add_contour(&s.0, ShapeType::Clip));
            shapes = o.overlay(OverlayRule::Union, FillRule::NonZero);
            marks.extend(spokes.iter().map(|s| s.1));
        }
        (shapes, marks)
    }

    /// Zone `zi`'s four thermal spokes for pad `p` (`self.obs[i]`): each bar from the pad's
    /// centre (or its hole's edge) across its gap, its part off the pad, and its end.
    fn spokes(&self, zi: usize, i: usize, p: usize) -> impl Iterator<Item = (RBox, RBox, Pt)> + '_ {
        let (z, pad, rb) = (&self.b.zones[zi], &self.b.pads[p], &self.obs[i].rb);
        let (gap, width) = (z.thermal_gap as f64, z.spoke_width as f64);
        // shortcut: a custom pad's hole counts as centred on its box (no corpus board has a
        // thru-hole custom pad); find where its spokes leave an offset hole when one does.
        let hole = pad.drill.map(|(dw, dh)| RBox::boxed(pad.at, (dw, dh), pad.angle, dw.min(dh) / 2));
        (0..4).map(move |k| {
            let a = (pad.angle + pad.spoke_angle + 90.0 * k as f64).to_radians();
            let d = (a.cos(), -a.sin());
            let (from, to) = (hole.as_ref().map_or(0.0, |h| exit(h, d, 0.0)), exit(rb, d, gap) + OVERLAP);
            let along = |t: f64| Pt { x: (rb.cx + d.0 * t).round() as Nm, y: (rb.cy + d.1 * t).round() as Nm };
            let bar =
                |from: f64| RBox::boxed(along((from + to) / 2.0), ((to - from).round() as Nm, width.round() as Nm), a.to_degrees(), 0);
            (bar(from), bar(exit(rb, d, 0.0)), along(to - OVERLAP))
        })
    }

    /// Whether each of `islands` of zone `zi` on layer `l` touches copper of its net: holds one
    /// of `marks`, a pad, via or track of the net reaches into it, or it meets the fill of a zone
    /// of the net it fills after (in `above`).
    fn touches(&self, zi: usize, l: LayerId, islands: &[Shape], marks: &[IntPoint], above: &[&Done]) -> Vec<bool> {
        let (b, z, on) = (self.b, &self.b.zones[zi], 1 << l);
        let mut touches: Vec<bool> = islands
            .iter()
            .map(|s| {
                let Some((lo, hi)) = shapes_box(std::slice::from_ref(s)) else { return false };
                let near: Vec<_> = marks.iter().copied().filter(|p| (lo.x..=hi.x).contains(&p.x) && (lo.y..=hi.y).contains(&p.y)).collect();
                s.contains_points(&near).contains(&true) || self.reaches(s, pt_box((lo, hi)), z.net, on)
            })
            .collect();
        let ours = above.iter().filter(|d| d.layer == l && z.net != 0 && b.zones[d.zone].net == z.net);
        if touches.contains(&false) && ours.clone().next().is_some() {
            // The other islands, grown by the drawing error, meet those fills all at once.
            let grown: Vec<_> =
                islands.iter().zip(&touches).map(|(s, &t)| if t { vec![] } else { grow(std::slice::from_ref(s), MAX_ERROR) }).collect();
            let mut o = Overlay::new(grown.len());
            grown.iter().flatten().flatten().for_each(|c| o.add_contour(c, ShapeType::Subject));
            ours.flat_map(|d| d.shapes.iter().flatten()).for_each(|c| o.add_contour(c, ShapeType::Clip));
            let met = o.overlay(OverlayRule::Intersect, FillRule::NonZero);
            let near = |g: &[Shape], m: &Shape| {
                shapes_box(g).zip(shapes_box(std::slice::from_ref(m))).is_some_and(|(a, c)| overlaps(pt_box(a), pt_box(c)))
            };
            for (t, g) in touches.iter_mut().zip(&grown) {
                *t = *t || met.iter().any(|m| near(g, m) && !overlay(g, std::slice::from_ref(m), OverlayRule::Intersect).is_empty());
            }
        }
        touches
    }

    /// Whether a pad, via or track of `net` on the `on` layers reaches into `island` (in `bounds`).
    fn reaches(&self, island: &Shape, bounds: (Pt, Pt), net: NetId, on: LayerMask) -> bool {
        let poly: Vec<Vec<Pt>> = island.iter().map(|c| c.iter().map(|p| Pt { x: p.x, y: p.y }).collect()).collect();
        let mut hit = false;
        self.tree.query(bounds, |i| {
            let o = &self.obs[i as usize];
            let ours = net != 0 && o.net == net && o.layers & on != 0 && matches!(o.kind, Kind::Copper | Kind::Pad(_));
            hit = hit || (ours && o.rb.dist_poly(&poly) == 0);
        });
        hit
    }
}

/// The largest clearance of a pad's own, or of a net class.
fn pad_clearance(b: &Board) -> Nm {
    b.pads.iter().filter_map(|p| p.clearance).chain(b.classes.iter().map(|c| c.clearance)).max().unwrap_or(0)
}

/// How far from zone `z` copper changes its fill: its clearances (`pads`: the largest of a
/// pad's own), thermal gap and minimum width.
fn reach(b: &Board, z: usize, pads: Nm) -> Nm {
    let (z, rules) = (&b.zones[z], &b.rules);
    let reach = [z.clearance, rules.clearance, pads, rules.edge_clearance, rules.hole_clearance, z.thermal_gap];
    reach.into_iter().max().unwrap_or_default() + z.min_width + MAX_ERROR as Nm
}

/// Islands touching no copper of zone `z`'s net go (by its rule), when one (`any`) does.
fn keep(z: &Zone, islands: &mut Vec<Shape>, touches: &[bool], any: bool) {
    if z.island_min > 0 && any {
        let mut keep = touches.iter();
        islands.retain(|s| *keep.next().unwrap() || s.area_two().unsigned_abs() / 2 >= z.island_min as u64);
    }
}

/// A zone's place in the fill order: priority, then id (the greater first, as in KiCad 10), then
/// the first in the file.
fn rank(b: &Board, z: usize) -> (u32, u128, Reverse<usize>) {
    (b.zones[z].priority, b.zones[z].id, Reverse(z))
}

/// Whether zone `z` keeps out of zone `y`'s fill: `y` ranks above it and has another net (or `z`
/// has none), or `y` has the same net and a higher priority (KiCad 10 leaves the overlap to it).
fn keeps_out(b: &Board, z: usize, y: usize) -> bool {
    let (zz, yy) = (&b.zones[z], &b.zones[y]);
    rank(b, y) > rank(b, z) && (zz.net == 0 || yy.net != zz.net || yy.priority > zz.priority)
}

fn pt_box((lo, hi): (IntPoint, IntPoint)) -> (Pt, Pt) {
    (Pt { x: lo.x, y: lo.y }, Pt { x: hi.x, y: hi.y })
}

/// Box around the shapes' outlines; `None` when there are none.
fn shapes_box(shapes: &[Shape]) -> Option<(IntPoint, IntPoint)> {
    let mut pts = shapes.iter().filter_map(|s| s.first()).flatten();
    let first = *pts.next()?;
    Some(pts.fold((first, first), |(lo, hi), p| (IntPoint::new(lo.x.min(p.x), lo.y.min(p.y)), IntPoint::new(hi.x.max(p.x), hi.y.max(p.y)))))
}

/// Pad `p`'s copper grown by `by`: `rb`, or a custom pad's outline.
fn pad_cut(rb: &RBox, p: &Pad, by: f64) -> Vec<Contour> {
    if p.custom.is_empty() {
        return vec![rounded(rb, by)];
    }
    let copper = p.copper().into_iter().map(|c| c.into_iter().map(ip).collect()).collect();
    grow(&normalize(copper), by + MAX_ERROR).into_iter().flatten().collect()
}

/// A filled polygon on copper, grown by its stroke and `by`.
fn grow_poly(s: &crate::board::Shape, by: f64) -> Vec<Contour> {
    let Geom::Poly(pts) = &s.geom else { return vec![] };
    grow(&normalize(vec![pts.iter().map(|&p| ip(p)).collect()]), by + (s.width / 2) as f64 + MAX_ERROR).into_iter().flatten().collect()
}

/// Signed distance from `p` to the edge of `rb`, negative inside.
fn depth(rb: &RBox, p: Pt) -> f64 {
    let (dx, dy) = (p.x as f64 - rb.cx, p.y as f64 - rb.cy);
    let (lx, ly) = ((dx * rb.ux + dy * rb.uy).abs() - rb.hx, (dy * rb.ux - dx * rb.uy).abs() - rb.hy);
    lx.max(0.0).hypot(ly.max(0.0)) + lx.max(ly).min(0.0) - rb.r
}

/// How far from `rb`'s centre a ray along unit `d` leaves `rb` grown by `by`.
fn exit(rb: &RBox, d: (f64, f64), by: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, rb.hx + rb.hy + rb.r + by + 1.0);
    for _ in 0..40 {
        let t = (lo + hi) / 2.0;
        if rb.core_pt(rb.cx + d.0 * t, rb.cy + d.1 * t) - rb.r < by { lo = t } else { hi = t }
    }
    hi
}

/// Chamfers or rounds the convex corners of the shapes.
fn smooth(shapes: Vec<Shape>, how: Smoothing) -> Vec<Shape> {
    let (Smoothing::Chamfer(size) | Smoothing::Fillet(size)) = how else { return shapes };
    if size <= 0 {
        return shapes;
    }
    let fillet = matches!(how, Smoothing::Fillet(_));
    let mut out = shapes;
    for c in out.iter_mut().flatten() {
        let n = c.len();
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            let (p, q, s) = (c[(i + n - 1) % n], c[i], c[(i + 1) % n]);
            let (ax, ay, bx, by) = ((p.x - q.x) as f64, (p.y - q.y) as f64, (s.x - q.x) as f64, (s.y - q.y) as f64);
            let (la, lb) = (ax.hypot(ay), bx.hypot(by));
            // Convex for the area: the outline turns left there (outlines and holes alike).
            let turn = (q.x - p.x) as f64 * by - (q.y - p.y) as f64 * bx;
            if turn <= 0.0 || la == 0.0 || lb == 0.0 {
                v.push(q);
                continue;
            }
            let (ua, ub) = ((ax / la, ay / la), (bx / lb, by / lb));
            let half = (ua.0 * ub.0 + ua.1 * ub.1).clamp(-1.0, 1.0).acos() / 2.0;
            // How far along each side the smoothing starts: the size, or the fillet's tangent point.
            let t = if fillet { size as f64 / half.tan() } else { size as f64 }.min(la / 2.0).min(lb / 2.0);
            let at = |u: (f64, f64), t: f64| IntPoint::new((q.x as f64 + u.0 * t).round() as Nm, (q.y as f64 + u.1 * t).round() as Nm);
            if !fillet {
                v.extend([at(ua, t), at(ub, t)]);
                continue;
            }
            // The arc from the tangent point on side a to the one on side b, around its centre.
            let rad = t * half.tan();
            let bis = ((ua.0 + ub.0) / 2.0, (ua.1 + ub.1) / 2.0);
            let bl = bis.0.hypot(bis.1);
            let (cx, cy) = (q.x as f64 + bis.0 / bl * rad / half.sin(), q.y as f64 + bis.1 / bl * rad / half.sin());
            let (s0, s1) = (at(ua, t), at(ub, t));
            let (a0, a1) = (((s0.y as f64) - cy).atan2(s0.x as f64 - cx), ((s1.y as f64) - cy).atan2(s1.x as f64 - cx));
            let sweep = (a1 - a0 + std::f64::consts::PI).rem_euclid(2.0 * std::f64::consts::PI) - std::f64::consts::PI;
            let m = segments(rad, sweep);
            v.extend((0..=m).map(|j| {
                let a = a0 + sweep * j as f64 / m as f64;
                IntPoint::new((cx + rad * a.cos()).round() as Nm, (cy + rad * a.sin()).round() as Nm)
            }));
        }
        *c = v;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const HEAD: &str = r#"(kicad_pcb (version 20221018) (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user) (44 "Edge.Cuts" user))
      (net 0 "") (net 1 "GND") (net 2 "SIG")"#;

    fn filled(items: &str) -> Board {
        let mut b = Board::parse(&format!("{HEAD} {items})")).unwrap();
        b.fill_zones();
        b
    }

    /// Whether (x, y) mm is in a fill on layer 0 (each fill's cuts resolved into holes first).
    pub(super) fn inside(b: &Board, x: f64, y: f64) -> bool {
        let p = IntPoint::new((x * 1e6).round() as Nm, (y * 1e6).round() as Nm);
        b.fills.iter().filter(|f| f.layer == 0).any(|f| normalize(vec![f.pts.iter().map(|&q| ip(q)).collect()]).contains_points(&[p])[0])
    }

    fn area(fills: &[ZoneFill]) -> f64 {
        fills.iter().map(|f| f.pts.iter().map(|&q| ip(q)).collect::<Contour>().area_two().unsigned_abs() as f64 / 2e12).sum()
    }

    #[test]
    fn fills_like_kicad_around_other_nets_with_thermal_spokes() {
        let b = filled(
            r#"(gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts") (stroke (width 0.1)))
              (footprint "T" (layer "F.Cu") (at 10 10)
                (pad "1" smd rect (at 0 0) (size 2 1) (layers "F.Cu") (net 1 "GND"))
                (pad "2" thru_hole circle (at 4 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu") (net 1 "GND"))
                (pad "3" smd rect (at 8 0) (size 2 1) (layers "F.Cu") (net 2 "SIG")))
              (segment (start 5 20) (end 15 20) (width 0.25) (layer "F.Cu") (net 2))
              (via (at 20 20) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 2))
              (zone (net 1) (net_name "GND") (layer "F.Cu") (connect_pads (clearance 0.3)) (min_thickness 0.25)
                (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4)) (polygon (pts (xy 2 2) (xy 28 2) (xy 28 28) (xy 2 28))))"#,
        );
        // KiCad 10 fills this board with 653.454 mm².
        assert_eq!((b.fills.len(), b.zones[0].fills.clone()), (1, 0..1));
        assert!((area(&b.fills) - 653.454).abs() < 0.02, "{}", area(&b.fills));
        // 0.3 mm around the other net's pad, track and via.
        let around = [(18.0, 10.79), (18.0, 10.81), (10.0, 20.4), (10.0, 20.44), (20.69, 20.0), (20.71, 20.0)];
        assert_eq!(around.map(|(x, y)| inside(&b, x, y)), [false, true, false, true, false, true]);
        // The SMD pad's spokes cross it; the round pad's run at 45° from its hole's edge.
        let spokes = [(10.0, 10.0), (10.6, 10.3), (14.0, 10.0), (14.7, 9.3), (15.0, 10.0), (14.3, 10.3)];
        assert_eq!(spokes.map(|(x, y)| inside(&b, x, y)), [true, false, false, true, false, true]);
    }

    #[test]
    fn keeps_a_net_classs_clearance_from_its_copper() {
        let board = format!(
            r#"{HEAD} (gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts") (stroke (width 0.1)))
              (segment (start 5 20) (end 15 20) (width 0.25) (layer "F.Cu") (net 2))
              (zone (net 1) (net_name "GND") (layer "F.Cu") (connect_pads (clearance 0.3)) (min_thickness 0.25)
                (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4)) (polygon (pts (xy 2 2) (xy 28 2) (xy 28 28) (xy 2 28)))))"#
        );
        let mut b = Board::parse(&board).unwrap();
        let pro = |class: &str| {
            format!(
                r#"{{"net_settings": {{"classes": [{{"name": "Default", "clearance": 0.2}}, {{"name": "HV", "clearance": 0.8}}],
                  "netclass_patterns": [{{"netclass": "HV", "pattern": "{class}"}}]}}}}"#
            )
        };
        // The zone's own 0.3 mm from the track's edge (0.125 mm from its middle); then 0.8 mm,
        // the track's net's class's, and as far when the class is the zone's net's.
        let near = |b: &Board| [20.4, 20.44, 20.9, 20.95].map(|y| inside(b, 10.0, y));
        b.fill_zones();
        assert_eq!(near(&b), [false, true, true, true]);
        for class in ["SIG", "GND"] {
            b.apply_pro(&pro(class));
            b.fill_zones();
            assert_eq!(near(&b), [false, false, false, true], "{class}");
        }
    }

    #[test]
    fn keeps_off_the_edge_narrow_necks_islands_holes_and_higher_zones() {
        let b = filled(
            r#"(gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts") (stroke (width 0.1)))
              (footprint "H" (layer "F.Cu") (at 10 5) (pad "" np_thru_hole circle (at 0 0) (size 1 1) (drill 1) (layers "*.Cu")))
              (footprint "T" (layer "F.Cu") (at 10 20) (pad "1" smd rect (at 0 0) (size 2 1) (layers "F.Cu") (net 1 "GND")))
              (segment (start 11.6 15) (end 11.6 25) (width 0.25) (layer "F.Cu") (net 2))
              (segment (start 20 10) (end 30 10) (width 0.25) (layer "F.Cu") (net 2))
              (segment (start 20 10.8) (end 30 10.8) (width 0.25) (layer "F.Cu") (net 2))
              (segment (start 25 15) (end 35 15) (width 0.25) (layer "F.Cu") (net 2))
              (segment (start 35 15) (end 35 25) (width 0.25) (layer "F.Cu") (net 2))
              (segment (start 35 25) (end 25 25) (width 0.25) (layer "F.Cu") (net 2))
              (segment (start 25 25) (end 25 15) (width 0.25) (layer "F.Cu") (net 2))
              (zone (net 2) (net_name "SIG") (layer "F.Cu") (priority 1) (connect_pads (clearance 0.3)) (min_thickness 0.25)
                (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4)) (polygon (pts (xy 2 25) (xy 8 25) (xy 8 28) (xy 2 28))))
              (zone (net 1) (net_name "GND") (layer "F.Cu") (connect_pads (clearance 0.1)) (min_thickness 0.25)
                (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4)) (polygon (pts (xy -5 -5) (xy 45 -5) (xy 45 35) (xy -5 35))))"#,
        );
        // KiCad 10: 17.985 mm² of SIG and 965.892 mm² of GND.
        assert!((area(&b.fills) - 17.985 - 965.892).abs() < 0.2, "{}", area(&b.fills));
        let at = |pts: &[(f64, f64)]| pts.iter().map(|&(x, y)| inside(&b, x, y)).collect::<Vec<_>>();
        // 0.5 mm from the edge; 0.25 mm around the hole; the net class's 0.2 mm beats the zone's 0.1.
        assert_eq!(
            at(&[(0.49, 15.0), (0.51, 15.0), (10.74, 5.0), (10.76, 5.0), (11.28, 22.0), (11.27, 22.0)]),
            [false, true, false, true, false, true]
        );
        // A 0.15 mm channel is narrower than the 0.25 mm minimum; nothing inside the SIG ring
        // connects to GND; the pad's spoke toward the SIG track goes, the others stay.
        assert_eq!(at(&[(25.0, 10.4), (30.0, 20.0), (11.0, 20.0), (9.0, 20.0), (10.0, 19.0)]), [false, false, false, true, true]);
        // GND keeps 0.3 mm (the SIG zone's clearance) from the SIG zone's fill.
        assert_eq!(at(&[(5.0, 24.69), (5.0, 24.71), (5.0, 26.0)]), [true, false, true]);
    }

    #[test]
    fn keeps_clear_of_custom_pads_and_relieves_them_from_their_box() {
        let b = filled(
            r#"(gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts") (stroke (width 0.1)))
              (footprint "C" (layer "F.Cu") (at 15 15 30)
                (pad "1" smd custom (at 0 0 30) (size 1 1) (layers "F.Cu") (net 1 "GND") (options (clearance outline) (anchor rect))
                  (primitives (gr_poly (pts (xy 0 -0.5) (xy 3 -0.5) (xy 3 2.5) (xy 2 2.5) (xy 2 0.5) (xy 0 0.5)) (width 0) (fill yes))))
                (pad "2" smd custom (at 0 6 30) (size 1 1) (layers "F.Cu") (net 2 "SIG") (options (clearance outline) (anchor circle))
                  (primitives (gr_line (start 0 0) (end 4 0) (width 0.4))))
                (pad "3" smd custom (at 0 -6 30) (size 1 1) (layers "F.Cu") (net 1 "GND") (options (clearance outline) (anchor circle))
                  (primitives (gr_circle (center 0 0) (end 2 0) (width 0.4) (fill no)))))
              (zone (net 1) (net_name "GND") (layer "F.Cu") (connect_pads (clearance 0.3)) (min_thickness 0.25)
                (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4)) (polygon (pts (xy 2 2) (xy 28 2) (xy 28 28) (xy 2 28))))"#,
        );
        // KiCad 10 fills this board with 642.202 mm².
        assert!((area(&b.fills) - 642.202).abs() < 0.05, "{}", area(&b.fills));
        // Points in the footprint, turned 30° with it, as KiCad 10 fills them.
        let (s, c) = 30f64.to_radians().sin_cos();
        let at = |pts: &[(f64, f64)]| pts.iter().map(|&(x, y)| inside(&b, 15.0 + x * c + y * s, 15.0 - x * s + y * c)).collect::<Vec<_>>();
        // 0.3 mm around the SIG pad's line.
        assert_eq!(at(&[(2.0, 6.49), (2.0, 6.51)]), [false, true]);
        // The L pad's spokes run from its box's centre (1.25, 1) across its 0.5 mm gap, even the
        // one whose way misses its copper.
        let l = at(&[(3.25, 1.0), (3.25, 1.5), (1.25, -0.75), (1.6, -0.75), (0.5, 0.9), (0.5, 0.7)]);
        assert_eq!(l, [true, false, true, false, true, false]);
        // The ring pad's spokes, at 45° for its round anchor, join the fill inside the ring.
        let k = std::f64::consts::FRAC_1_SQRT_2;
        let ring = at(&[(1.15, -6.0), (1.55, -6.0), (1.55 * k, -6.0 - 1.55 * k), (2.45, -6.0), (2.45 * k, -6.0 - 2.45 * k)]);
        assert_eq!(ring, [true, false, true, false, true]);
    }

    #[test]
    fn fills_a_new_zone_alone_with_kicads_settings() {
        // A SIG zone of higher priority whose fill is stale: a 3 mm square in its corner.
        let items = r#"(gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts") (stroke (width 0.1)))
              (footprint "T" (layer "F.Cu") (at 10 10)
                (pad "1" smd rect (at 0 0) (size 2 1) (layers "F.Cu") (net 1 "GND"))
                (pad "2" smd rect (at 10 0) (size 2 1) (layers "F.Cu") (net 2 "SIG")))
              (zone (net 2) (net_name "SIG") (layer "F.Cu") (priority 1) (connect_pads (clearance 0.3)) (min_thickness 0.25)
                (filled_areas_thickness no) (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4))
                (polygon (pts (xy 2 20) (xy 8 20) (xy 8 28) (xy 2 28)))
                (filled_polygon (layer "F.Cu") (pts (xy 3 21) (xy 6 21) (xy 6 24) (xy 3 24))))"#;
        let mut b = Board::parse(&format!("{HEAD} {items})")).unwrap();
        let pt = |x, y| Pt { x: x * MM, y: y * MM };
        let corners = vec![pt(1, 1), pt(29, 1), pt(29, 29), pt(1, 29)];
        let before = b.fills.clone();
        let i = b.add_zone(corners.clone(), 0, 1);
        let z = &b.zones[i];
        assert_eq!(
            (z.priority, z.connect, z.clearance, z.min_width, z.thermal_gap, z.spoke_width),
            (0, Connect::Thermal, MM / 2, MM / 4, MM / 2, MM / 2)
        );
        assert_eq!((z.island_min, z.outlined, &b.fills[..1], z.fills.clone()), (i64::MAX, false, &before[..], 1..2));
        let at = |pts: &[(f64, f64)]| pts.iter().map(|&(x, y)| inside(&b, x, y)).collect::<Vec<_>>();
        // 0.5 mm from the SIG pad and the SIG fill as it is (not the zone's outline); relieved GND pad.
        assert_eq!(at(&[(21.45, 10.0), (21.55, 10.0), (6.45, 22.0), (6.55, 22.0), (7.5, 26.0)]), [false, true, false, true, true]);
        assert_eq!(at(&[(11.25, 10.0), (11.25, 10.4)]), [true, false]);
        // With every fill up to date, the zone filled alone is what a refill of all gives.
        let mut b = filled(items);
        b.add_zone(corners, 0, 1);
        assert_eq!(b.fill_zones(), 0);
    }

    #[test]
    fn fills_zones_of_one_priority_the_greater_id_first() {
        let zone = |net, name, id, x0, x1| {
            format!(
                r#"(zone (net {net}) (net_name "{name}") (layer "F.Cu") (tstamp {id}) (connect_pads (clearance 0.5)) (min_thickness 0.25)
                  (fill yes (island_removal_mode 1)) (polygon (pts (xy {x0} 5) (xy {x1} 5) (xy {x1} 25) (xy {x0} 25))))"#
            )
        };
        let (low, high) = ("29ccb441-0479-4f07-8c20-b90754f4e2a4", "7cb510eb-00dd-42b4-8e9c-736e44d3def8");
        // KiCad 10: the greater id fills its outline, the other keeps 0.5 mm from it.
        for ((gnd, sig), want) in [((high, low), [299.9845, 289.8732]), ((low, high), [189.8732, 399.9845])] {
            let edge = r#"(gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts") (stroke (width 0.1)))"#;
            let mut b = filled(&format!("{edge} {} {}", zone(1, "GND", gnd, 5, 20), zone(2, "SIG", sig, 15, 35)));
            let got: Vec<_> = b.zones.iter().map(|z| area(&b.fills[z.fills.clone()])).collect();
            assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.02), "{got:?}");
            // A new zone ranks last: it fills around the GND fill as a refill of all leaves it.
            let pt = |x, y| Pt { x: x * MM, y: y * MM };
            b.add_zone(vec![pt(1, 1), pt(39, 1), pt(39, 29), pt(1, 29)], 0, 2);
            assert_eq!(b.fill_zones(), 0);
        }
    }

    #[test]
    fn keeps_islands_their_nets_copper_reaches_into() {
        // A GND island inside a SIG ring: KiCad 10 keeps it when a GND pad, or a via leading to
        // one, reaches into it from the ring's clearance, their centres outside it, or when it
        // touches another GND zone's fill.
        let ring = r#"(gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts") (stroke (width 0.1)))
              (segment (start 25 15) (end 35 15) (width 0.25) (layer "F.Cu") (net 2))
              (segment (start 35 15) (end 35 25) (width 0.25) (layer "F.Cu") (net 2))
              (segment (start 35 25) (end 25 25) (width 0.25) (layer "F.Cu") (net 2))
              (segment (start 25 25) (end 25 15) (width 0.25) (layer "F.Cu") (net 2))
              (footprint "T" (layer "F.Cu") (at 10 20) (pad "1" smd rect (at 0 0) (size 2 1) (layers "F.Cu") (net 1 "GND")))
              (zone (net 1) (net_name "GND") (layer "F.Cu") (connect_pads yes (clearance 0.2)) (min_thickness 0.25)
                (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4) (island_removal_mode 0))
                (polygon (pts (xy 2 2) (xy 38 2) (xy 38 28) (xy 2 28))))"#;
        let pad = r#"(footprint "P" (layer "F.Cu") (at 25.3 20) (pad "1" smd rect (at 0 0) (size 0.3 2) (layers "F.Cu") (net 1 "GND")))"#;
        let via = r#"(via (at 25.3 20) (size 0.35) (drill 0.2) (layers "F.Cu" "B.Cu") (net 1))
              (segment (start 25.3 20) (end 10 5) (width 0.25) (layer "B.Cu") (net 1))
              (footprint "B" (layer "B.Cu") (at 10 5) (pad "1" smd rect (at 0 0) (size 2 1) (layers "B.Cu") (net 1 "GND")))"#;
        // A GND zone of higher priority with a pad in it: the island leaves it its 4 mm² and joins it.
        let zone = r#"(zone (net 1) (net_name "GND") (layer "F.Cu") (priority 1) (connect_pads yes (clearance 0.2)) (min_thickness 0.25)
                (fill yes (island_removal_mode 1)) (polygon (pts (xy 29 19) (xy 31 19) (xy 31 21) (xy 29 21))))
              (footprint "Q" (layer "F.Cu") (at 30 20) (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "GND")))"#;
        // KiCad 10: 910.018 mm² with the island, 822.630 without.
        for (extra, kept, want) in [("", false, 822.630), (pad, true, 910.018), (via, true, 910.018), (zone, true, 910.018)] {
            let b = filled(&format!("{ring} {extra}"));
            assert!((area(&b.fills) - want).abs() < 0.05 && inside(&b, 30.0, 20.0) == kept, "{extra}: {}", area(&b.fills));
        }
    }

    #[test]
    fn keeps_the_fill_of_zones_on_no_copper_layer() {
        let b = filled(
            r#"(zone (net 0) (net_name "") (layer "F.SilkS") (min_thickness 0.25) (fill yes)
                (polygon (pts (xy 0 0) (xy 5 0) (xy 5 5))) (filled_polygon (layer "F.SilkS") (pts (xy 0 0) (xy 5 0) (xy 5 5))))"#,
        );
        assert_eq!((b.fills.len(), b.zones[0].fills.clone()), (1, 0..1));
    }
}
