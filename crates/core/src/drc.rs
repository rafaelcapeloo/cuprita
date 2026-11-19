//! Design rule checking (DRC) and 2D R-tree spatial index.

use crate::board::{Board, Geom, LayerMask, NetId, Nm, Pad, PadShape, Pt, Track, arc_center, place, shape_box, union};
use crate::history::Patch;
use crate::polygon::{Shape, grow, outline, overlay, path};
use crate::tr::{t, tr, trf};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::point::IntPoint;
use std::f64::consts::{PI, TAU};

mod silk;

const FANOUT: usize = 16;
/// How far copper or holes may fall short of a clearance and still pass, as in KiCad (found with
/// `kicad-cli`): copper drawn at the limit measures a little under it once rounded to nanometres.
pub(crate) const SLACK: Nm = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub kind: &'static str,
    pub at: Pt,
    pub actual: Nm,
    pub limit: Nm,
    pub what: String,
    pub fix: String,
    /// The one or two items it is about.
    pub items: [Option<Item>; 2],
    /// Only a warning, as its kind's severity says (KiCad's, or the project's own): the board
    /// can be made as it is, but look.
    pub warning: bool,
}

impl Violation {
    /// What its kind is called over a list of them, in the language shown (KiCad's name for a
    /// kind this doesn't know).
    pub fn kind_name(&self) -> &'static str {
        tr(match self.kind {
            "clearance" => t!("Copper too close"),
            "copper_edge_clearance" => t!("Copper too near the edge"),
            "track_width" => t!("Tracks too narrow"),
            "via_diameter" => t!("Vias too small"),
            "drill_out_of_range" => t!("Holes too small"),
            "annular_width" => t!("Rings too thin"),
            "hole_to_hole" => t!("Holes too close"),
            "hole_clearance" => t!("Copper too near a hole"),
            "solder_mask_bridge" => t!("Solder mask bridges"),
            "courtyards_overlap" => t!("Parts overlapping"),
            "track_dangling" => t!("Track ends on nothing"),
            "via_dangling" => t!("Vias joined on one layer"),
            other => silk::kind_name(other).unwrap_or(other),
        })
    }
}

/// The violations as the project wants them told: each a warning or an error as its kind's
/// severity says there (else as KiCad starts it: loose ends are warnings), those of a kind the
/// project ignores left out.
fn judged(b: &Board, mut vs: Vec<Violation>) -> Vec<Violation> {
    let set = |kind: &str| b.severities.iter().find(|(k, _)| k == kind).map(|&(_, how)| how);
    vs.retain_mut(|v| match set(v.kind).unwrap_or(Some(DANGLING.contains(&v.kind) || silk::KINDS.contains(&v.kind))) {
        Some(warning) => {
            v.warning = warning;
            true
        }
        None => false,
    });
    vs
}

/// A board item: an index into `Board::tracks`, `Board::vias` or `Board::pads`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    Track(usize),
    Via(usize),
    Pad(usize),
}

/// Flat Sort-Tile-Recursive (STR) 2D R-tree over `(lo, hi)` nanometre bounding boxes.
pub struct RTree {
    items: Vec<((Pt, Pt), u32)>,
    blocks: Vec<(Pt, Pt)>,
    top: Vec<(Pt, Pt)>,
}

impl RTree {
    pub fn new(mut items: Vec<((Pt, Pt), u32)>) -> Self {
        items.sort_unstable_by_key(|(b, _)| b.0.x);
        let slice = ((items.len() / FANOUT).isqrt().max(1) * FANOUT).max(FANOUT);
        for s in items.chunks_mut(slice) {
            s.sort_unstable_by_key(|(b, _)| b.0.y);
        }
        let blocks: Vec<_> = items.chunks(FANOUT).map(|c| c.iter().map(|&(b, _)| b).reduce(union).unwrap()).collect();
        let top = blocks.chunks(FANOUT).map(|c| c.iter().copied().reduce(union).unwrap()).collect();
        Self { items, blocks, top }
    }

    pub fn query(&self, q: (Pt, Pt), mut f: impl FnMut(u32)) {
        for (ti, _) in self.top.iter().enumerate().filter(|&(_, &b)| overlaps(b, q)) {
            let b_end = ((ti + 1) * FANOUT).min(self.blocks.len());
            for bi in (ti * FANOUT..b_end).filter(|&i| overlaps(self.blocks[i], q)) {
                let i_end = ((bi + 1) * FANOUT).min(self.items.len());
                for &(ib, id) in &self.items[bi * FANOUT..i_end] {
                    if overlaps(ib, q) {
                        f(id);
                    }
                }
            }
        }
    }
}

/// Oriented rectangle core `(center, u, hx, hy)` dilated by radius `r`: represents discs,
/// capsules, segments, rectangles, ovals and rounded rectangles with one distance primitive.
#[derive(Clone, Copy)]
pub(crate) struct RBox {
    pub(crate) cx: f64,
    pub(crate) cy: f64,
    pub(crate) ux: f64,
    pub(crate) uy: f64,
    pub(crate) hx: f64,
    pub(crate) hy: f64,
    pub(crate) r: f64,
    pub(crate) bounds: (Pt, Pt),
}

impl RBox {
    pub(crate) fn disc(at: Pt, r: Nm) -> Self {
        Self { cx: at.x as f64, cy: at.y as f64, ux: 1.0, uy: 0.0, hx: 0.0, hy: 0.0, r: r as f64, bounds: pad_box((at, at), r) }
    }

    pub(crate) fn segment(a: Pt, b: Pt, r: Nm) -> Self {
        let (dx, dy) = (b.x as f64 - a.x as f64, b.y as f64 - a.y as f64);
        let len = dx.hypot(dy);
        let (ux, uy) = if len > 0.0 { (dx / len, dy / len) } else { (1.0, 0.0) };
        let lo = Pt { x: a.x.min(b.x), y: a.y.min(b.y) };
        let hi = Pt { x: a.x.max(b.x), y: a.y.max(b.y) };
        Self {
            cx: (a.x as f64 + b.x as f64) * 0.5,
            cy: (a.y as f64 + b.y as f64) * 0.5,
            ux,
            uy,
            hx: len * 0.5,
            hy: 0.0,
            r: r as f64,
            bounds: pad_box((lo, hi), r),
        }
    }

    pub(crate) fn boxed(at: Pt, size: (Nm, Nm), deg: f64, r: Nm) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        let (ux, uy) = (c, -s);
        let (hx, hy) = ((size.0 / 2 - r).max(0) as f64, (size.1 / 2 - r).max(0) as f64);
        let rx = (hx * ux.abs() + hy * uy.abs()).ceil() as Nm + r;
        let ry = (hx * uy.abs() + hy * ux.abs()).ceil() as Nm + r;
        let bounds = (Pt { x: at.x - rx, y: at.y - ry }, Pt { x: at.x + rx, y: at.y + ry });
        Self { cx: at.x as f64, cy: at.y as f64, ux, uy, hx, hy, r: r as f64, bounds }
    }

    /// A pad's shape; for a custom pad, the box around its copper (`Pad::copper`), turned with it.
    pub(crate) fn pad(p: &Pad) -> Self {
        let mut pts = p.custom.iter().flatten();
        if let Some(&first) = pts.next() {
            let (lo, hi) = pts
                .fold((first, first), |(lo, hi), q| (Pt { x: lo.x.min(q.x), y: lo.y.min(q.y) }, Pt { x: hi.x.max(q.x), y: hi.y.max(q.y) }));
            return Self::boxed(place(mid(lo, hi), p.at, p.angle), (hi.x - lo.x, hi.y - lo.y), p.angle, 0);
        }
        let (w, h) = if p.shape == PadShape::Circle { (p.size.0, p.size.0) } else { p.size };
        let r = match p.shape {
            PadShape::Circle | PadShape::Oval => w.min(h) / 2,
            PadShape::RoundRect(k) => (k as f64 * w.min(h) as f64).round() as Nm,
            _ => 0,
        };
        Self::boxed(p.at, (w, h), p.angle, r)
    }

    pub(crate) fn core_pt(&self, px: f64, py: f64) -> f64 {
        let (dx, dy) = (px - self.cx, py - self.cy);
        let lx = (dx * self.ux + dy * self.uy).abs() - self.hx;
        let ly = (-dx * self.uy + dy * self.ux).abs() - self.hy;
        lx.max(0.0).hypot(ly.max(0.0))
    }

    fn corners(&self) -> [(f64, f64); 4] {
        let (xx, xy, yx, yy) = (self.hx * self.ux, self.hx * self.uy, -self.hy * self.uy, self.hy * self.ux);
        [
            (self.cx + xx + yx, self.cy + xy + yy),
            (self.cx + xx - yx, self.cy + xy - yy),
            (self.cx - xx + yx, self.cy - xy + yy),
            (self.cx - xx - yx, self.cy - xy - yy),
        ]
    }

    pub(crate) fn dist(&self, o: &Self) -> Nm {
        let core = if self.hx == 0.0 && self.hy == 0.0 {
            o.core_pt(self.cx, self.cy)
        } else if o.hx == 0.0 && o.hy == 0.0 {
            self.core_pt(o.cx, o.cy)
        } else {
            let (dx, dy) = (o.cx - self.cx, o.cy - self.cy);
            let (c, s) = ((self.ux * o.ux + self.uy * o.uy).abs(), (self.ux * o.uy - self.uy * o.ux).abs());
            let hit = (dx * self.ux + dy * self.uy).abs() <= self.hx + o.hx * c + o.hy * s
                && (-dx * self.uy + dy * self.ux).abs() <= self.hy + o.hx * s + o.hy * c
                && (dx * o.ux + dy * o.uy).abs() <= o.hx + self.hx * c + self.hy * s
                && (-dx * o.uy + dy * o.ux).abs() <= o.hy + self.hx * s + self.hy * c;
            if hit {
                0.0
            } else {
                let d1 = self.corners().into_iter().map(|(x, y)| o.core_pt(x, y)).fold(f64::INFINITY, f64::min);
                let d2 = o.corners().into_iter().map(|(x, y)| self.core_pt(x, y)).fold(f64::INFINITY, f64::min);
                d1.min(d2)
            }
        };
        (core - self.r - o.r).max(0.0).round() as Nm
    }

    /// Whether it and `o` overlap, or come nearer than `keeps`: as KiCad collides two shapes,
    /// which only touching doesn't (two parts' silkscreen lines side by side).
    pub(crate) fn collides(&self, o: &Self, keeps: Nm) -> bool {
        // Their middles nearer than both half widths and `keeps`, by more than rounding.
        let (middles, reach) = (Self { r: 0.0, ..*self }.dist(&Self { r: 0.0, ..*o }) as f64, self.r + o.r + keeps as f64);
        middles + 0.5 < reach || (middles == 0.0 && reach == 0.0)
    }

    /// Distance to polygons with their holes inside them (even-odd); 0 when its centre is inside.
    pub(crate) fn dist_poly(&self, poly: &[Vec<Pt>]) -> Nm {
        let edges = || poly.iter().flat_map(|c| c.iter().zip(c.iter().cycle().skip(1)));
        let (x, y) = (self.cx, self.cy);
        let crosses = |(a, b): &(&Pt, &Pt)| {
            let ((ax, ay), (bx, by)) = ((a.x as f64, a.y as f64), (b.x as f64, b.y as f64));
            (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay)
        };
        if edges().filter(crosses).count() % 2 == 1 {
            return 0;
        }
        edges().map(|(a, b)| RBox::segment(*a, *b, 0).dist(self)).min().unwrap_or(Nm::MAX)
    }
}

struct CuItem {
    item: Item,
    at: Pt,
    layers: LayerMask,
    net: NetId,
    shape: RBox,
}

/// Checks every item on the board.
pub fn check(b: &Board) -> Vec<Violation> {
    let cu = copper(b);
    // The loose ends and the silkscreen of a large board go meanwhile, each on a core of its own.
    #[cfg(not(target_arch = "wasm32"))]
    if cu.0.len() > 8_000 {
        return std::thread::scope(|s| {
            let (ends, ink) = (s.spawn(|| dangling(b, None, &cu)), s.spawn(|| silk::check(b, None)));
            let mut out = run(b, None, &cu);
            out.extend(courtyards(b));
            out.extend(ends.join().expect("the loose ends' check panicked"));
            out.extend(ink.join().expect("the silkscreen's check panicked"));
            judged(b, out)
        });
    }
    let mut out = run(b, None, &cu);
    out.extend(courtyards(b));
    out.extend(dangling(b, None, &cu));
    out.extend(silk::check(b, None));
    judged(b, out)
}

/// The violations that involve any of `items`.
pub fn check_items(b: &Board, items: &[Item]) -> Vec<Violation> {
    let mut items = items.to_vec();
    items.sort_unstable();
    judged(b, run(b, Some(&items), &copper(b)))
}

/// Brings `old`, the board's violations before a change, up to date after it, given `undo`, the
/// patch that undoes the change: the violations of the items it left alone follow them to their
/// new indices, and only the items it put in or changed are checked again (all, after a new
/// outline).
pub fn recheck(b: &Board, mut old: Vec<Violation>, undo: &Patch) -> Vec<Violation> {
    // A new outline, or a part drawing some of it moved, moves every item's edge clearance.
    if undo.edge(b) {
        return check(b);
    }
    // Courtyards are the parts', which items don't name: all checked again.
    old.retain(|v| v.kind != COURTYARDS);
    // An end dangles by what its neighbours do: those found before are looked at again, with
    // every end where the change put copper, took it away or filled a zone anew.
    let c = undo.changed(b);
    let was =
        |lo: Pt, hi: Pt, r: Nm| pad_box((Pt { x: lo.x.min(hi.x), y: lo.y.min(hi.y) }, Pt { x: lo.x.max(hi.x), y: lo.y.max(hi.y) }), r);
    let mut near: Vec<(Pt, Pt)> = old.iter().filter(|v| DANGLING.contains(&v.kind)).map(|v| (v.at, v.at)).collect();
    near.extend(c.tracks.iter().map(|t| track_box(t).bounds));
    near.extend(c.vias.iter().map(|v| was(v.at, v.at, v.size / 2)));
    near.extend(c.pads.iter().map(|p| RBox::pad(p).bounds));
    near.extend(undo.filled(b).iter().filter_map(|f| f.pts.iter().map(|&p| (p, p)).reduce(union)));
    old.retain(|v| !DANGLING.contains(&v.kind));
    // Ink is looked at again where the change put or took some, or what it may lie over: a
    // part's pads, graphics and texts, a via where vias have openings; a track's nowhere.
    let mut inked: Vec<(Pt, Pt)> = c.pads.iter().map(|p| RBox::pad(p).bounds).collect();
    inked.extend(c.shapes.iter().map(|s| shape_box(s)));
    inked.extend(c.texts.iter().map(|t| pad_box(crate::font::bounds(t), t.thickness)));
    inked.extend(c.vias.iter().filter(|_| b.rules.tented != (true, true)).map(|v| was(v.at, v.at, v.size / 2)));
    old.retain(|v| !(silk::KINDS.contains(&v.kind) && inked.iter().any(|&n| overlaps(n, (v.at, v.at)))));
    old.retain_mut(|v| v.items.iter_mut().flatten().all(|i| undo.moved(*i).map(|m| *i = m).is_some()));
    let cu = copper(b);
    old.extend(run(b, Some(&undo.items(b)), &cu));
    old.extend(courtyards(b));
    old.extend(dangling(b, Some(&near), &cu));
    if !inked.is_empty() {
        old.extend(silk::check(b, Some(&inked)));
    }
    judged(b, old)
}

/// KiCad's warnings of copper that joins nothing: found for the whole board, not by an item.
const DANGLING: [&str; 2] = ["track_dangling", "via_dangling"];

/// Tracks with an end on no copper of their net, and vias their net's copper meets on one
/// layer alone or not at all (KiCad's `track_dangling` and `via_dangling`, as its connectivity
/// finds them): of every track and via, or with `near` of those in its boxes. An end is joined
/// by a pad, via, track or zone fill of its net within half its track's width; a track with
/// both ends on one pad, via or fill doesn't dangle, and one with both on one track is joined
/// at the nearer end only.
fn dangling(b: &Board, near: Option<&[(Pt, Pt)]>, (cu, tree): &(Vec<CuItem>, RTree)) -> Vec<Violation> {
    let mut looked = vec![near.is_none(); cu.len()];
    for &at in near.into_iter().flatten() {
        tree.query(at, |k| looked[k as usize] = true);
    }
    // A net's fills on a layer, asked only of an end or a via no copper settles. A whole check
    // asks one fill thousands of times (a ground plane's stitching vias): its edges by rows then.
    let mut fills: std::collections::HashMap<(NetId, u8), Vec<Fill>> = std::collections::HashMap::new();
    for f in &b.fills {
        if let Some(bounds) = f.pts.iter().map(|&p| (p, p)).reduce(union) {
            fills.entry((f.net, f.layer)).or_default().push((bounds, &f.pts, Default::default(), Default::default()));
        }
    }
    let fill_near = |net: NetId, layer: u8, at: &RBox, r: Nm| {
        let in_reach = |(bounds, pts, rows, asked): &Fill| {
            if !overlaps(pad_box(*bounds, r), at.bounds) {
                return false;
            }
            // After a change few ask, and a fill's rows aren't worth making: until several do.
            asked.set(asked.get().saturating_add(1));
            match near.is_none() || asked.get() > 4 {
                true => rows.get_or_init(|| Rows::new(pts, *bounds)).near((at.cx, at.cy), at.r + r as f64),
                false => at.dist_poly(std::slice::from_ref(*pts)) <= r,
            }
        };
        fills.get(&(net, layer)).is_some_and(|of| of.iter().any(in_reach))
    };
    let mut out = vec![];
    for (k, a) in cu.iter().enumerate().filter(|&(k, _)| looked[k]) {
        match a.item {
            Item::Track(i) => {
                let (t, r) = (&b.tracks[i], (b.tracks[i].width + 1) / 2);
                let ends = [RBox::disc(t.a, 0), RBox::disc(t.b, 0)];
                // How many of its net's items each end is on; `None` once it can't dangle.
                let mut on = Some([0, 0]);
                tree.query(pad_box(a.shape.bounds, r), |j| {
                    let c = &cu[j as usize];
                    let Some(counts) = on.as_mut().filter(|_| j as usize != k && c.net == t.net && c.layers & a.layers != 0) else {
                        return;
                    };
                    let copper = Copper::of(b, c);
                    match ends.each_ref().map(|e| copper.dist(e)) {
                        [da, db] if da <= r && db <= r => match c.item {
                            // The end nearer one of the other track's ends, as KiCad picks it.
                            Item::Track(o) => {
                                let o = &b.tracks[o];
                                let to = |e: Pt| {
                                    [o.a, o.b]
                                        .map(|q| (q.x as f64 - e.x as f64).hypot(q.y as f64 - e.y as f64))
                                        .into_iter()
                                        .fold(f64::MAX, f64::min)
                                };
                                counts[(to(t.a) >= to(t.b)) as usize] += 1
                            }
                            _ => on = None,
                        },
                        [da, db] => (counts[0], counts[1]) = (counts[0] + (da <= r) as u32, counts[1] + (db <= r) as u32),
                    }
                });
                let Some(counts) = on else { continue };
                let joined = [0, 1].map(|e| counts[e] > 0 || fill_near(t.net, t.layer, &ends[e], r));
                if joined != [true, true] {
                    let (label, at) = (cu_label(b, a.item), if joined[0] { t.b } else { t.a });
                    let what = trf("{0}: an end joins nothing", &[&label]);
                    let fix = tr("Route it on to a pad, via or track of its net, or delete it").to_string();
                    out.push(viol("track_dangling", [Some(a.item), None], at, 0, 0, what, fix));
                }
            }
            Item::Via(i) => {
                let v = &b.vias[i];
                // The layers its net's copper meets it on: an item's is its topmost, as KiCad counts.
                let top = |mask: LayerMask| b.stackup.iter().map(|c| c.layer).find(|&l| mask & 1 << l != 0);
                let mut layers: Vec<u8> = vec![];
                tree.query(a.shape.bounds, |j| {
                    let c = &cu[j as usize];
                    if j as usize != k && c.net == v.net && c.layers & a.layers != 0 && Copper::of(b, c).dist(&a.shape) <= 0 {
                        layers.extend(top(c.layers));
                    }
                });
                if layers.iter().all(|&l| l == layers[0]) {
                    let on_fill = b.stackup.iter().map(|c| c.layer).filter(|&l| a.layers & 1 << l != 0 && fill_near(v.net, l, &a.shape, 0));
                    layers.extend(on_fill);
                }
                let alone = layers.is_empty() && v.net != 0;
                if alone || (!layers.is_empty() && layers.iter().all(|&l| l == layers[0])) {
                    let label = cu_label(b, a.item);
                    let what = match alone {
                        true => trf("{0}: joins nothing", &[&label]),
                        false => trf("{0}: joined on {1} alone", &[&label, &b.layers[layers[0] as usize]]),
                    };
                    let fix = tr("Route on from it on another layer, or delete it").to_string();
                    out.push(viol("via_dangling", [Some(a.item), None], v.at, 0, 0, what, fix));
                }
            }
            Item::Pad(_) => {}
        }
    }
    out
}

/// A zone fill: its box, its outline and, once asked for, its edges by rows; how often it was asked.
type Fill<'a> = ((Pt, Pt), &'a Vec<Pt>, std::cell::OnceCell<Rows<'a>>, std::cell::Cell<u8>);

/// A zone fill's edges by the rows of the board they cross: whether a point is in the fill or
/// near its rim, looking at one row's edges and not the fill's tens of thousands.
struct Rows<'a> {
    pts: &'a [Pt],
    y0: i64,
    step: i64,
    rows: Vec<Vec<u32>>,
}

impl<'a> Rows<'a> {
    fn new(pts: &'a [Pt], (lo, hi): (Pt, Pt)) -> Self {
        let n = (pts.len() / 8).clamp(1, 4096);
        let (y0, step) = (lo.y as i64, ((hi.y as i64 - lo.y as i64) / n as i64 + 1).max(1));
        let mut rows = vec![vec![]; n];
        let row = |y: Nm| ((y as i64 - y0) / step).clamp(0, n as i64 - 1) as usize;
        for (i, (a, c)) in pts.iter().zip(pts.iter().cycle().skip(1)).enumerate() {
            (row(a.y.min(c.y))..=row(a.y.max(c.y))).for_each(|r| rows[r].push(i as u32));
        }
        Rows { pts, y0, step, rows }
    }

    /// Whether the point (`x`, `y`) is in the fill or within `reach` of its rim: in, by the
    /// edges it has to its right an odd number of times (a fill's holes hang on its rim by a
    /// cut walked both ways, which counts twice).
    fn near(&self, (x, y): (f64, f64), reach: f64) -> bool {
        let row = |y: f64| ((y as i64 - self.y0) / self.step).clamp(0, self.rows.len() as i64 - 1) as usize;
        let edge = |i: u32| {
            let (a, c) = (self.pts[i as usize], self.pts[(i as usize + 1) % self.pts.len()]);
            ((a.x as f64, a.y as f64), (c.x as f64, c.y as f64))
        };
        let right = |i: &&u32| {
            let ((ax, ay), (bx, by)) = edge(**i);
            (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay)
        };
        let close = |&i: &u32| {
            let ((ax, ay), (bx, by)) = edge(i);
            let (dx, dy) = (bx - ax, by - ay);
            let along = if dx == 0.0 && dy == 0.0 { 0.0 } else { (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0) };
            (x - ax - along * dx).hypot(y - ay - along * dy) <= reach
        };
        self.rows[row(y)].iter().filter(right).count() % 2 == 1 || (row(y - reach)..=row(y + reach)).any(|r| self.rows[r].iter().any(close))
    }
}

/// The board's copper that the rules are checked between, its tracks, vias and pads on copper,
/// and a tree of their boxes: made once a check.
fn copper(b: &Board) -> (Vec<CuItem>, RTree) {
    let mut cu = Vec::with_capacity(b.tracks.len() + b.vias.len() + b.pads.len());
    for (i, t) in b.tracks.iter().enumerate() {
        cu.push(CuItem { item: Item::Track(i), at: mid(t.a, t.b), layers: 1 << t.layer, net: t.net, shape: track_box(t) });
    }
    for (i, v) in b.vias.iter().enumerate() {
        let layers = v.layers & b.copper;
        if layers != 0 {
            cu.push(CuItem { item: Item::Via(i), at: v.at, layers, net: v.net, shape: RBox::disc(v.at, v.size / 2) });
        }
    }
    for (i, p) in b.pads.iter().enumerate() {
        let layers = p.layers & b.copper;
        if layers != 0 {
            cu.push(CuItem { item: Item::Pad(i), at: p.at, layers, net: p.net, shape: RBox::pad(p) });
        }
    }
    let tree = RTree::new(cu.iter().enumerate().map(|(i, c)| (c.shape.bounds, i as u32)).collect());
    (cu, tree)
}

const COURTYARDS: &str = "courtyards_overlap";

/// How deep courtyards may overlap and still pass, as KiCad lets them: parts placed touching
/// overlap a little once their corners are rounded (7.5 µm on glasgow-revD1, which KiCad passes;
/// 20 µm it reports, on kria-k26).
const TOUCHING: f64 = 10_000.0;

/// Parts whose courtyards overlap on their side of the board (KiCad's courtyards_overlap): the
/// room each needs to be assembled, which another takes. Courtyards that only touch pass; a part
/// whose courtyard lines don't close is left out.
fn courtyards(b: &Board) -> Vec<Violation> {
    let mut out = vec![];
    for name in ["F.CrtYd", "B.CrtYd"] {
        let Some(layer) = b.layers.iter().position(|l| l == name) else { continue };
        let yards: Vec<(usize, Vec<Shape>, (Pt, Pt))> = (b.footprints.iter().enumerate())
            .filter_map(|(i, f)| {
                let lines: Vec<_> = b.shapes[f.shapes.clone()].iter().filter(|s| s.layer as usize == layer).collect();
                let area = grow(&outline(lines.iter().map(|s| path(&s.geom)).collect())?, -TOUCHING / 2.0);
                Some((i, area, lines.into_iter().map(shape_box).reduce(union)?))
            })
            .collect();
        let tree = RTree::new(yards.iter().enumerate().map(|(k, y)| (y.2, k as u32)).collect());
        for (k, (i, a, ab)) in yards.iter().enumerate() {
            tree.query(*ab, |m| {
                let (j, c, _) = &yards[m as usize];
                if (m as usize) <= k {
                    return;
                }
                let both = overlay(a, c, OverlayRule::Intersect);
                let Some(at) = both.iter().flatten().flatten().copied().reduce(|p, q| IntPoint::new(p.x.min(q.x), p.y.min(q.y))) else {
                    return;
                };
                let (f, g) = (&b.footprints[*i].reference, &b.footprints[*j].reference);
                let (what, fix) = (
                    trf("Courtyards of {0} and {1} overlap on {2}", &[f, g, &name]),
                    trf("Move {0} and {1} apart until their courtyards clear", &[f, g]),
                );
                out.push(viol(COURTYARDS, [None, None], Pt { x: at.x, y: at.y }, 0, 0, what, fix));
            });
        }
    }
    out
}

/// Violations of the `only` items (sorted), or of every item when `None`.
fn run(b: &Board, only: Option<&[Item]>, (cu, cu_tree): &(Vec<CuItem>, RTree)) -> Vec<Violation> {
    let dirty = |i: Item| only.is_none_or(|c| c.binary_search(&i).is_ok());
    // Each pair is checked once: from its first item, or from its changed one.
    let pair = |i: Item, j: Item| dirty(i) && (i < j || !dirty(j));
    let r = &b.rules;
    let mut out = vec![];
    for (i, t) in b.tracks.iter().enumerate().filter(|&(i, _)| dirty(Item::Track(i))) {
        if t.width < r.track_width {
            let l = &b.layers[t.layer as usize];
            out.push(viol(
                "track_width",
                [Some(Item::Track(i)), None],
                mid(t.a, t.b),
                t.width,
                r.track_width,
                trf("Track {0}on {1}: width {2} < {3}", &[&net_tag(b, t.net), l, &mm(t.width), &mm(r.track_width)]),
                trf("Widen track on {0} to at least {1}", &[l, &mm(r.track_width)]),
            ));
        }
    }
    for (i, v) in b.vias.iter().enumerate().filter(|&(i, _)| dirty(Item::Via(i))) {
        let label = cu_label(b, Item::Via(i));
        if v.size < r.via_diameter {
            out.push(viol(
                "via_diameter",
                [Some(Item::Via(i)), None],
                v.at,
                v.size,
                r.via_diameter,
                trf("{0}: diameter {1} < {2}", &[&label, &mm(v.size), &mm(r.via_diameter)]),
                trf("Increase via diameter to at least {0}", &[&mm(r.via_diameter)]),
            ));
        }
        if v.drill > 0 && v.drill < r.hole_diameter {
            out.push(viol(
                "drill_out_of_range",
                [Some(Item::Via(i)), None],
                v.at,
                v.drill,
                r.hole_diameter,
                trf("{0}: drill {1} < {2}", &[&label, &mm(v.drill), &mm(r.hole_diameter)]),
                trf("Increase via drill to at least {0}", &[&mm(r.hole_diameter)]),
            ));
        }
        let ann = (v.size - v.drill) / 2;
        if v.drill > 0 && ann < r.annular_width {
            out.push(viol(
                "annular_width",
                [Some(Item::Via(i)), None],
                v.at,
                ann,
                r.annular_width,
                trf("{0}: annular ring {1} < {2}", &[&label, &mm(ann), &mm(r.annular_width)]),
                trf("Increase via size or shrink drill so ring >= {0}", &[&mm(r.annular_width)]),
            ));
        }
    }
    for (i, p) in b.pads.iter().enumerate().filter(|&(i, _)| dirty(Item::Pad(i))) {
        let Some((dw, dh)) = p.drill.filter(|_| (p.layers & b.copper) != 0) else { continue };
        let d = dw.min(dh);
        if d < r.hole_diameter {
            out.push(viol(
                "drill_out_of_range",
                [Some(Item::Pad(i)), None],
                p.at,
                d,
                r.hole_diameter,
                trf("{0}: drill {1} < {2}", &[&pad_label(b, p), &mm(d), &mm(r.hole_diameter)]),
                trf("Increase pad drill to at least {0}", &[&mm(r.hole_diameter)]),
            ));
        }
        // shortcut: a custom pad's ring is its anchor's (no corpus board drills one); measure it to
        // `Pad::copper` when a board does.
        let ann = ((p.size.0 - dw) / 2).min((p.size.1 - dh) / 2);
        if ann < r.annular_width {
            out.push(viol(
                "annular_width",
                [Some(Item::Pad(i)), None],
                p.at,
                ann,
                r.annular_width,
                trf("{0}: annular ring {1} < {2}", &[&pad_label(b, p), &mm(ann), &mm(r.annular_width)]),
                trf("Enlarge pad copper or shrink drill so ring >= {0}", &[&mm(r.annular_width)]),
            ));
        }
    }

    // Hole-to-hole mechanical spacing across all vias and drilled pads.
    let mut holes: Vec<(Item, Pt, RBox)> = Vec::new();
    for (i, v) in b.vias.iter().enumerate().filter(|(_, v)| v.drill > 0) {
        holes.push((Item::Via(i), v.at, RBox::disc(v.at, v.drill / 2)));
    }
    for (i, p) in b.pads.iter().enumerate() {
        if let Some((dw, dh)) = p.drill {
            holes.push((Item::Pad(i), p.at, RBox::boxed(p.at, (dw, dh), p.angle, dw.min(dh) / 2)));
        }
    }
    let hole_tree = RTree::new(holes.iter().enumerate().map(|(i, h)| (h.2.bounds, i as u32)).collect());
    for (i, h) in holes.iter().enumerate().filter(|(_, h)| dirty(h.0)) {
        hole_tree.query(pad_box(h.2.bounds, r.hole_to_hole), |j| {
            let j = j as usize;
            if !pair(h.0, holes[j].0) {
                return;
            }
            // Blind and buried vias are drilled before the board is pressed: two that share no
            // layer never meet (as KiCad takes them).
            if let (Item::Via(x), Item::Via(y)) = (h.0, holes[j].0)
                && b.vias[x].layers & b.vias[y].layers & b.copper == 0
            {
                return;
            }
            // Reported in item order, as a full check finds it.
            let ((ia, at_a, ha), (ib, at_b, hb)) = (holes[i.min(j)], holes[i.max(j)]);
            let d = ha.dist(&hb);
            if d < r.hole_to_hole - SLACK && at_a != at_b {
                let (la, lb) = (cu_label(b, ia), cu_label(b, ib));
                out.push(viol(
                    "hole_to_hole",
                    [Some(ia), Some(ib)],
                    mid(at_a, at_b),
                    d,
                    r.hole_to_hole,
                    trf("{0} to {1}: hole clearance {2} < {3}", &[&la, &lb, &mm(d), &mm(r.hole_to_hole)]),
                    trf("Separate drilled holes by at least {0}", &[&mm(r.hole_to_hole)]),
                ));
            }
        });
    }

    let edges = edges(b);
    if !edges.is_empty() {
        let edge_tree = RTree::new(edges.iter().enumerate().map(|(i, e)| (e.bounds, i as u32)).collect());
        for item in cu.iter().filter(|c| dirty(c.item)) {
            let (mut best, copper) = (Nm::MAX, Copper::of(b, item));
            edge_tree.query(pad_box(item.shape.bounds, r.edge_clearance), |ei| best = best.min(copper.dist(&edges[ei as usize])));
            if best < r.edge_clearance - SLACK {
                let label = cu_label(b, item.item);
                out.push(viol(
                    "copper_edge_clearance",
                    [Some(item.item), None],
                    item.at,
                    best,
                    r.edge_clearance,
                    trf("{0}: edge clearance {1} < {2}", &[&label, &mm(best), &mm(r.edge_clearance)]),
                    trf("Move {0} at least {1} from Edge.Cuts", &[&label, &mm(r.edge_clearance)]),
                ));
            }
        }
    }

    // Two nets keep the larger of their classes' clearances; the widest any keeps is how far to look.
    let clearances = b.clearances();
    let (keeps, reach) =
        (|n: NetId| clearances.get(n as usize).copied().unwrap_or(r.clearance), clearances.iter().copied().fold(r.clearance, Nm::max));
    // A via's or a pad's hole, which copper of another net keeps the hole clearance from
    // (KiCad's hole_clearance), where their copper keeps its own: a thin ring, a pad drilled off centre.
    let hole = |c: &CuItem| match c.item {
        Item::Via(k) => (b.vias[k].drill > 0).then(|| RBox::disc(b.vias[k].at, b.vias[k].drill / 2)),
        Item::Pad(k) => b.pads[k].drill.map(|(w, h)| RBox::boxed(b.pads[k].at, (w, h), b.pads[k].angle, w.min(h) / 2)),
        Item::Track(_) => None,
    };
    let near_hole = |x: &CuItem, of: &CuItem, d: Nm| {
        // A mounting hole's pad has no number: its part names it.
        let lo = match of.item {
            Item::Pad(k) if b.pads[k].number.is_empty() => b.footprints[b.pads[k].footprint as usize].reference.clone(),
            other => cu_label(b, other),
        };
        let what = trf("{0} to the hole of {1}: clearance {2} < {3}", &[&cu_label(b, x.item), &lo, &mm(d), &mm(r.hole_clearance)]);
        let fix = trf("Move it at least {0} from the hole", &[&mm(r.hole_clearance)]);
        viol("hole_clearance", [Some(x.item), Some(of.item)], mid(x.at, of.at), d, r.hole_clearance, what, fix)
    };
    // An unplated hole has no copper: every net's keeps the hole clearance from it.
    for (k, p) in b.pads.iter().enumerate().filter(|(_, p)| p.layers & b.copper == 0) {
        let Some(h) = p.drill.map(|(w, h)| RBox::boxed(p.at, (w, h), p.angle, w.min(h) / 2)) else { continue };
        let of = CuItem { item: Item::Pad(k), at: p.at, layers: 0, net: 0, shape: h };
        cu_tree.query(pad_box(h.bounds, r.hole_clearance), |j| {
            let a = &cu[j as usize];
            let d = Copper::of(b, a).dist(&h);
            if (dirty(a.item) || dirty(of.item)) && d < r.hole_clearance - SLACK {
                out.push(near_hole(a, &of, d));
            }
        });
    }
    // Solder mask openings (KiCad's solder_mask_bridge), on the front and on the back: a pad's
    // where it lists the mask, grown by its margin; a via's where vias aren't tented. Two of
    // nets apart with no mask left between them, or bare copper of another net in one, are a
    // bridge of solder waiting: one opening over both.
    // shortcut: openings drawn on the mask layers, and pads of a net tie, aren't looked at; nor
    // zone copper an opening lies over (a fill keeps further from a pad than a mask margin). An
    // unplated hole in a pad's opening is no bridge here (KiCad calls it one when the hole's pad
    // lists copper layers): hole_clearance says it.
    let outer = [b.stackup.first(), b.stackup.last()].map(|c| c.map_or(0, |c| 1 << c.layer));
    let masks = ["F.Mask", "B.Mask"].map(|name| b.layers.iter().position(|l| l == name).map_or(0, |l| 1 << l));
    let tented = [r.tented.0, r.tented.1];
    let opening = |c: &CuItem, side: usize| match c.item {
        Item::Pad(k) => (b.pads[k].layers & masks[side] != 0).then(|| b.pads[k].mask_margin.unwrap_or(r.mask_margin)),
        Item::Via(_) => (!tented[side]).then_some(r.mask_margin),
        Item::Track(_) => None,
    };
    let widest_margin = b.pads.iter().filter_map(|p| p.mask_margin).fold(r.mask_margin, Nm::max).max(0);
    let mask_reach = r.mask_min_width.max(r.mask_to_copper) + 2 * widest_margin;
    let same_part = |a: Item, c: Item| match (a, c) {
        (Item::Pad(i), Item::Pad(j)) if b.pads[i].footprint == b.pads[j].footprint => {
            r.mask_bridges_in_parts || b.footprints[b.pads[i].footprint as usize].mask_bridges
        }
        _ => false,
    };
    // On each side they share: `a`'s and `c`'s openings there (`None`: copper under the mask).
    let bridge = |a: &CuItem, c: &CuItem, open: [[Option<Nm>; 2]; 2], d: Nm| -> [Option<Violation>; 2] {
        let side = |s: usize| {
            // Two openings keep the fab's narrowest web between them; bare copper, its own gap from one.
            let need = match (open[0][s], open[1][s]) {
                (Some(ea), Some(ec)) => r.mask_min_width + ea + ec,
                (Some(e), None) | (None, Some(e)) => r.mask_to_copper + e,
                (None, None) => return None,
            };
            // As KiCad collides them: nearer than the mask needs, or touching.
            (a.layers & c.layers & outer[s] != 0 && (d < need || d <= 0)).then(|| {
                let (la, lc, face) = (cu_label(b, a.item), cu_label(b, c.item), tr(if s == 0 { t!("front") } else { t!("back") }));
                let what = match (open[0][s], open[1][s]) {
                    (Some(_), Some(_)) => trf("{0} and {1}: no solder mask between their openings ({2})", &[&la, &lc, &face]),
                    (Some(_), None) => trf("{0} is bare in the solder mask opening of {1} ({2})", &[&lc, &la, &face]),
                    _ => trf("{0} is bare in the solder mask opening of {1} ({2})", &[&la, &lc, &face]),
                };
                let fix = tr("Move them apart, or give the pad a smaller solder mask margin").to_string();
                viol("solder_mask_bridge", [Some(a.item), Some(c.item)], mid(a.at, c.at), d, need, what, fix)
            })
        };
        [side(0), side(1)]
    };
    // What item `i` and those after it (or unchanged) do to each other.
    let from = |i: usize, out: &mut Vec<Violation>| {
        let a = &cu[i];
        cu_tree.query(pad_box(a.shape.bounds, reach.max(r.hole_clearance).max(mask_reach)), |j| {
            let j = j as usize;
            let c = &cu[j];
            if !pair(a.item, c.item) || (a.layers & c.layers) == 0 || (a.net != 0 && a.net == c.net) || same_unconnected(b, a.item, c.item)
            {
                return;
            }
            let (a, c) = (&cu[i.min(j)], &cu[i.max(j)]);
            let (d, need) = (Copper::of(b, a).dist_to(&Copper::of(b, c)), keeps(a.net).max(keeps(c.net)));
            // Few pairs are near enough for the widest of margins to join them: the rest go by.
            if (d < mask_reach || d <= 0) && !same_part(a.item, c.item) {
                out.extend(bridge(a, c, [a, c].map(|x| [0, 1].map(|s| opening(x, s))), d).into_iter().flatten());
            }
            if d < need - SLACK {
                let (la, lc) = (cu_label(b, a.item), cu_label(b, c.item));
                // Copper of two nets that touches is a short: said as one, not as a gap of 0.
                let what = match d <= 0 {
                    true => trf("{0} and {1} touch: a short between their nets", &[&la, &lc]),
                    false => trf("{0} to {1}: clearance {2} < {3}", &[&la, &lc, &mm(d), &mm(need)]),
                };
                let fix = trf("Separate items by at least {0}", &[&mm(need)]);
                out.push(viol("clearance", [Some(a.item), Some(c.item)], mid(a.at, c.at), d, need, what, fix));
            // A hole lies in its own copper: only copper nearer that than the hole clearance can be too near the hole.
            } else if d < r.hole_clearance - SLACK
                && let Some((x, of, d)) = [(a, c), (c, a)].into_iter().find_map(|(x, of)| {
                    // One said of a pair: its copper too close, else a hole.
                    let d = Copper::of(b, x).dist(&hole(of)?);
                    (d < r.hole_clearance - SLACK).then_some((x, of, d))
                })
            {
                out.push(near_hole(x, of, d));
            }
        });
    };
    let changed: Vec<usize> = (0..cu.len()).filter(|&i| dirty(cu[i].item)).collect();
    // A large board checked whole takes its cores, each a run of the items, their findings in
    // the items' order (but in the browser, which has one core).
    #[cfg(not(target_arch = "wasm32"))]
    if only.is_none() && changed.len() > 8_000 {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
        let found: Vec<Vec<Violation>> = std::thread::scope(|s| {
            let runs = changed.chunks(changed.len().div_ceil(cores)).map(|run| {
                s.spawn(|| {
                    let mut out = vec![];
                    run.iter().for_each(|&i| from(i, &mut out));
                    out
                })
            });
            runs.collect::<Vec<_>>().into_iter().map(|run| run.join().expect("the clearance check panicked")).collect()
        });
        out.extend(found.into_iter().flatten());
        return out;
    }
    changed.iter().for_each(|&i| from(i, &mut out));
    out
}

/// The board outline (`Edge.Cuts`) as zero-width segments; arcs and circles become 32-segment polylines.
pub(crate) fn edges(b: &Board) -> Vec<RBox> {
    let Some(edge_layer) = b.layers.iter().position(|l| l == "Edge.Cuts") else { return vec![] };
    let mut edges = Vec::new();
    for sh in b.shapes.iter().filter(|s| s.layer as usize == edge_layer) {
        match &sh.geom {
            Geom::Segment { a, b } => edges.push(RBox::segment(*a, *b, 0)),
            Geom::Arc { start, mid, end } => edges.extend(arc_pts(*start, *mid, *end).windows(2).map(|w| RBox::segment(w[0], w[1], 0))),
            Geom::Circle { center, radius } => {
                let pts: Vec<Pt> = (0..=32)
                    .map(|i| {
                        let a = TAU * i as f64 / 32.0;
                        Pt {
                            x: center.x + (*radius as f64 * a.cos()).round() as Nm,
                            y: center.y + (*radius as f64 * a.sin()).round() as Nm,
                        }
                    })
                    .collect();
                edges.extend(pts.windows(2).map(|w| RBox::segment(w[0], w[1], 0)));
            }
            // Outline polygons are closed, like KiCad draws them.
            Geom::Poly(pts) => edges.extend(pts.iter().zip(pts.iter().cycle().skip(1)).map(|(a, b)| RBox::segment(*a, *b, 0))),
        }
    }
    // An unplated slot is milled as the outline is, so its rim is an edge too, as in KiCad (a
    // round hole is drilled: it keeps the hole clearance).
    let slots = b.pads.iter().filter(|p| p.layers & b.copper == 0).filter_map(|p| Some((p, p.drill.filter(|(w, h)| w != h)?)));
    edges.extend(slots.map(|(p, (w, h))| RBox::boxed(p.at, (w, h), p.angle, w.min(h) / 2)));
    edges
}

fn viol(kind: &'static str, items: [Option<Item>; 2], at: Pt, actual: Nm, limit: Nm, what: String, fix: String) -> Violation {
    Violation { kind, at, actual, limit, what, fix, items, warning: false }
}

fn same_unconnected(b: &Board, a: Item, c: Item) -> bool {
    match (a, c) {
        (Item::Pad(i), Item::Pad(j)) => {
            let (pa, pb) = (&b.pads[i], &b.pads[j]);
            pa.footprint == pb.footprint && !pa.number.is_empty() && pa.number == pb.number
        }
        (Item::Track(i), Item::Track(j)) => {
            let (ta, tb) = (&b.tracks[i], &b.tracks[j]);
            ta.net == 0 && tb.net == 0 && (ta.a == tb.a || ta.a == tb.b || ta.b == tb.a || ta.b == tb.b)
        }
        _ => false,
    }
}

pub(crate) fn track_box(t: &Track) -> RBox {
    let Some(m) = t.mid else { return RBox::segment(t.a, t.b, t.width / 2) };
    let pts = arc_pts(t.a, m, t.b);
    let bounds = pts.iter().map(|&p| (p, p)).reduce(union).unwrap();
    RBox { cx: m.x as f64, cy: m.y as f64, ux: 1.0, uy: 0.0, hx: 0.0, hy: 0.0, r: 0.0, bounds: pad_box(bounds, t.width / 2) }
}

/// A circle's centre, radius, start angle and signed sweep (radians), as `arc_center` gives them.
type Arc = (f64, f64, f64, f64, f64);

/// An item's copper, ready to measure: its shape, an arc track and its circle, or a custom
/// pad's outlines (`Pad::copper`).
enum Copper<'a> {
    Shape(RBox),
    Arc(&'a Track, Arc),
    Outline(Vec<Vec<Pt>>),
}

impl<'a> Copper<'a> {
    fn of(b: &'a Board, a: &CuItem) -> Self {
        if let Item::Track(i) = a.item
            && let Some(m) = b.tracks[i].mid
        {
            let t = &b.tracks[i];
            return arc_center(t.a, m, t.b).map_or(Copper::Shape(RBox::segment(t.a, t.b, t.width / 2)), |c| Copper::Arc(t, c));
        }
        if let Item::Pad(i) = a.item
            && !b.pads[i].custom.is_empty()
        {
            return Copper::Outline(b.pads[i].copper());
        }
        Copper::Shape(a.shape)
    }

    /// Its pieces as boxes: an arc's chords, an outline's edges, else its shape.
    // shortcut: chords stray from the arc up to 1.2 µm on corpus boards, past `SLACK` (more on
    // bigger arcs); measure arcs to boxes exactly if that flags copper routed at the limit.
    fn boxes(&self) -> Vec<RBox> {
        match self {
            Copper::Shape(s) => vec![*s],
            Copper::Arc(t, _) => {
                let chords = arc_pts(t.a, t.mid.expect("an arc track"), t.b);
                chords.windows(2).map(|w| RBox::segment(w[0], w[1], t.width / 2)).collect()
            }
            Copper::Outline(p) => {
                p.iter().flat_map(|c| c.iter().zip(c.iter().cycle().skip(1))).map(|(a, b)| RBox::segment(*a, *b, 0)).collect()
            }
        }
    }

    /// Distance to box `o`.
    fn dist(&self, o: &RBox) -> Nm {
        match self {
            Copper::Shape(s) => s.dist(o),
            Copper::Outline(p) => o.dist_poly(p),
            Copper::Arc(..) => self.boxes().iter().map(|c| c.dist(o)).min().unwrap_or(Nm::MAX),
        }
    }

    /// Whether it and `o` overlap or come nearer than `keeps`, as KiCad collides them (`RBox::collides`).
    fn collides(&self, o: &Copper, keeps: Nm) -> bool {
        match (self, o) {
            (Copper::Shape(a), Copper::Shape(c)) => a.collides(c, keeps),
            _ => {
                let d = self.dist_to(o);
                d <= 0 || d < keeps
            }
        }
    }

    /// Distance to other copper.
    fn dist_to(&self, o: &Copper) -> Nm {
        match (self, o) {
            (Copper::Arc(t, p), Copper::Arc(u, q)) => (arcs_apart(*p, *q) - (t.width + u.width) as f64 / 2.0).max(0.0).round() as Nm,
            (_, Copper::Shape(s)) => self.dist(s),
            (Copper::Shape(s), _) => o.dist(s),
            // Each one's pieces against the other: both ways, as an outline's edges miss what lies inside it.
            _ => o.boxes().iter().map(|e| self.dist(e)).chain(self.boxes().iter().map(|e| o.dist(e))).min().unwrap_or(Nm::MAX),
        }
    }
}

/// Distance between two arcs: their nearest points are ends, on the line through both centres,
/// or where they cross.
fn arcs_apart(p: Arc, q: Arc) -> f64 {
    let at = |(cx, cy, r, ..): Arc, t: f64| (cx + r * t.cos(), cy + r * t.sin());
    // Whether the arc passes direction `t` from its centre.
    let on = |(.., a0, sweep): Arc, t: f64| ((t - a0) * sweep.signum()).rem_euclid(TAU) <= sweep.abs();
    let apart = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
    // From a point straight across to the arc, else to its nearer end.
    let to = |e: (f64, f64), a: Arc| {
        let (dx, dy) = (e.0 - a.0, e.1 - a.1);
        if on(a, dy.atan2(dx)) { (dx.hypot(dy) - a.2).abs() } else { apart(e, at(a, a.3)).min(apart(e, at(a, a.3 + a.4))) }
    };
    let ends = |a: Arc| [at(a, a.3), at(a, a.3 + a.4)];
    let mut d = ends(p).map(|e| to(e, q)).into_iter().chain(ends(q).map(|e| to(e, p))).fold(f64::INFINITY, f64::min);
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let k = dx.hypot(dy);
    if k > 0.0 {
        let t = dy.atan2(dx);
        for (tp, tq) in [(t, t), (t, t + PI), (t + PI, t), (t + PI, t + PI)] {
            if on(p, tp) && on(q, tq) {
                d = d.min(apart(at(p, tp), at(q, tq)));
            }
        }
        // Where the circles cross, `x` along the centre line from p's centre and `h` to a side.
        if (p.2 - q.2).abs() <= k && k <= p.2 + q.2 {
            let x = (k * k + p.2 * p.2 - q.2 * q.2) / (2.0 * k);
            let h = (p.2 * p.2 - x * x).max(0.0).sqrt();
            for side in [-h, h] {
                let c = (p.0 + (x * dx - side * dy) / k, p.1 + (x * dy + side * dx) / k);
                if on(p, (c.1 - p.1).atan2(c.0 - p.0)) && on(q, (c.1 - q.1).atan2(c.0 - q.0)) {
                    return 0.0;
                }
            }
        }
    }
    d
}

pub(crate) fn arc_pts(s: Pt, m: Pt, e: Pt) -> Vec<Pt> {
    let Some((cx, cy, r, a0, sweep)) = arc_center(s, m, e) else { return vec![s, m, e] };
    let mut v: Vec<Pt> = (0..=32)
        .map(|i| {
            let a = a0 + sweep * i as f64 / 32.0;
            Pt { x: (cx + r * a.cos()).round() as Nm, y: (cy + r * a.sin()).round() as Nm }
        })
        .collect();
    (v[0], v[16], v[32]) = (s, m, e);
    v
}

fn pad_label(b: &Board, p: &Pad) -> String {
    // A pad without a number (a mounting hole's, a shield's) is its part's pad.
    let number = if p.number.is_empty() { String::new() } else { format!(" {}", p.number) };
    trf("{0} pad{1}{2}", &[&b.footprints[p.footprint as usize].reference, &number, &net_suffix(b, p.net)])
}

pub(crate) fn cu_label(b: &Board, k: Item) -> String {
    match k {
        Item::Track(i) => {
            let t = &b.tracks[i];
            trf("Track {0}on {1}", &[&net_tag(b, t.net), &b.layers[t.layer as usize]])
        }
        Item::Via(i) => trf("Via{0}", &[&net_suffix(b, b.vias[i].net)]),
        Item::Pad(i) => pad_label(b, &b.pads[i]),
    }
}

fn net_tag(b: &Board, net: NetId) -> String {
    if net == 0 { String::new() } else { format!("({}) ", b.nets[net as usize]) }
}

/// ` (GND)` after an item's name, or nothing off any net.
fn net_suffix(b: &Board, net: NetId) -> String {
    if net == 0 { String::new() } else { format!(" ({})", b.nets[net as usize]) }
}

fn mm(v: Nm) -> String {
    format!("{:.3} mm", v as f64 / 1e6)
}

pub(crate) fn mid(a: Pt, b: Pt) -> Pt {
    Pt { x: ((a.x as i64 + b.x as i64) / 2) as Nm, y: ((a.y as i64 + b.y as i64) / 2) as Nm }
}

pub(crate) fn pad_box((lo, hi): (Pt, Pt), r: Nm) -> (Pt, Pt) {
    (Pt { x: lo.x.saturating_sub(r), y: lo.y.saturating_sub(r) }, Pt { x: hi.x.saturating_add(r), y: hi.y.saturating_add(r) })
}

pub(crate) fn overlaps(a: (Pt, Pt), b: (Pt, Pt)) -> bool {
    a.0.x <= b.1.x && b.0.x <= a.1.x && a.0.y <= b.1.y && b.0.y <= a.1.y
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rules broken, without the warnings of loose ends: these boards' copper joins nothing.
    fn check(b: &Board) -> Vec<Violation> {
        super::check(b).into_iter().filter(|v| !v.warning).collect()
    }

    #[test]
    fn warns_of_track_ends_and_vias_that_join_nothing() {
        // A pad of net A at (5, 5), 1 mm square, and a GND fill from x = 20 on.
        let board = |items: &str| {
            Board::parse(&format!(
                r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "GND")
                  (footprint "T" (layer "F.Cu") (at 5 5) (property "Reference" "T1") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
                  (zone (net 2) (net_name "GND") (layer "F.Cu") (polygon (pts (xy 20 0) (xy 30 0) (xy 30 10) (xy 20 10)))
                    (filled_polygon (layer "F.Cu") (pts (xy 20 0) (xy 30 0) (xy 30 10) (xy 20 10))))
                  {items})"#
            ))
            .unwrap()
        };
        let seg = |a: (f64, f64), z: (f64, f64), layer: &str, net: u32| {
            format!("(segment (start {} {}) (end {} {}) (width 0.2) (layer \"{layer}\") (net {net}))", a.0, a.1, z.0, z.1)
        };
        let via = |x: f64, y: f64, net: u32| format!("(via (at {x} {y}) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net {net}))");
        let found = |items: &str| super::check(&board(items)).into_iter().map(|v| (v.kind, v.at.x, v.at.y, v.what)).collect::<Vec<_>>();
        // From the pad to nowhere: its far end dangles. On to a via and away on the back: nothing does,
        // but the back track's own far end.
        let loose = found(&seg((5.0, 5.0), (10.0, 5.0), "F.Cu", 1));
        assert_eq!(loose, [("track_dangling", 10_000_000, 5_000_000, "Track (A) on F.Cu: an end joins nothing".to_string())]);
        let on = [seg((5.0, 5.0), (10.0, 5.0), "F.Cu", 1), via(10.0, 5.0, 1), seg((10.0, 5.0), (10.0, 8.0), "B.Cu", 1)].concat();
        assert_eq!(found(&on), [("track_dangling", 10_000_000, 8_000_000, "Track (A) on B.Cu: an end joins nothing".to_string())]);
        // A via its net meets on one layer, or not at all; one without a net alone is no one's.
        let vias = [seg((5.0, 5.0), (10.0, 5.0), "F.Cu", 1), via(10.0, 5.0, 1), via(15.0, 8.0, 1), via(15.0, 2.0, 0)].concat();
        let said: Vec<_> = found(&vias).into_iter().map(|v| v.3).collect();
        assert_eq!(said, ["Via (A): joined on F.Cu alone", "Via (A): joins nothing"]);
        // An end in its net's fill is joined, and a track wholly in the fill or on one pad doesn't dangle;
        // within half its width of the pad's edge counts (0.6 mm from its middle), further doesn't.
        let joined =
            [seg((25.0, 5.0), (27.0, 5.0), "F.Cu", 2), seg((5.2, 5.0), (5.3, 5.2), "F.Cu", 1), seg((19.0, 2.0), (25.0, 2.0), "F.Cu", 2)]
                .concat();
        assert_eq!(found(&joined), [("track_dangling", 19_000_000, 2_000_000, "Track (GND) on F.Cu: an end joins nothing".to_string())]);
        let near = |x: f64| {
            found(&[seg((x, 5.0), (8.0, 5.0), "F.Cu", 1), via(8.0, 5.0, 1), seg((8.0, 5.0), (5.0, 5.4), "B.Cu", 1)].concat()).len()
        };
        assert_eq!((near(5.6), near(5.61)), (1, 2));
        // A short track with both ends on another: joined at the end nearer one of its ends, as KiCad.
        let short =
            [seg((5.0, 5.0), (9.0, 5.0), "F.Cu", 1), seg((9.0, 5.0), (9.05, 5.0), "F.Cu", 1), seg((9.05, 5.0), (5.0, 5.0), "F.Cu", 1)]
                .concat();
        assert_eq!(found(&short), []);
        // They are warnings, re-found after a change near them: the loose end goes once a track joins it.
        let mut b = board(&seg((5.0, 5.0), (10.0, 5.0), "F.Cu", 1));
        let before = super::check(&b);
        assert!(before.len() == 1 && before[0].warning && check(&b).is_empty());
        let undo = b
            .add_route(
                &crate::shove::Shoved::default(),
                crate::route::tracks(&[Pt { x: 10_000_000, y: 5_000_000 }, Pt { x: 5_000_000, y: 5_400_000 }], 200_000, 0, 1),
                None,
            )
            .unwrap();
        assert_eq!(recheck(&b, before.clone(), &undo), []);
        let redo = b.apply(undo);
        assert_eq!(recheck(&b, vec![], &redo), before);
    }

    #[test]
    fn finds_a_point_in_a_fill_by_its_rows_as_by_all_its_edges() {
        // A square with a square hole hung on its rim by a cut, as KiCad writes a fill; many
        // points along its sides, for several rows.
        let side = |a: (i32, i32), z: (i32, i32)| (0..50).map(move |k| Pt { x: a.0 + (z.0 - a.0) / 50 * k, y: a.1 + (z.1 - a.1) / 50 * k });
        let mm = |x: i32, y: i32| (x * MM, y * MM);
        let rim = side(mm(0, 0), mm(20, 0))
            .chain(side(mm(20, 0), mm(20, 20)))
            .chain(side(mm(20, 20), mm(0, 20)))
            .chain(side(mm(0, 20), mm(0, 0)));
        let hole = [mm(0, 0), mm(5, 5), mm(5, 15), mm(15, 15), mm(15, 5), mm(5, 5)].map(|(x, y)| Pt { x, y });
        let pts: Vec<Pt> = rim.chain(hole).collect();
        let bounds = pts.iter().map(|&p| (p, p)).reduce(union).unwrap();
        let rows = Rows::new(&pts, bounds);
        assert!(rows.rows.len() > 20);
        // A lattice of points over and around it, alone and with a reach: the same answers.
        for (x, y) in (-3..26).flat_map(|x| (-3..26).map(move |y| (x * MM + 1234, y * MM + 777))) {
            for reach in [0, 600_000] {
                let (at, all) = (RBox::disc(Pt { x, y }, 0), std::slice::from_ref(&pts));
                assert_eq!(rows.near((x as f64, y as f64), reach as f64), at.dist_poly(all) <= reach, "{x} {y} {reach}");
            }
        }
    }

    #[test]
    fn copper_keeps_clear_of_other_nets_holes_and_of_unplated_slots() {
        // A via of B, 0.7 mm over a 0.5 mm hole; a round unplated hole and an unplated slot, 1 mm
        // across; a track of A passing each, `via` and `holes` mm from their middles.
        let board = |via: f64, holes: f64| {
            Board::parse(&format!(
                r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user)) (net 0 "") (net 1 "A") (net 2 "B")
                  (via (at 5 5) (size 0.7) (drill 0.5) (layers "F.Cu" "B.Cu") (net 2))
                  (segment (start 3 {0}) (end 7 {0}) (width 0.2) (layer "B.Cu") (net 1))
                  (footprint "H" (layer "F.Cu") (at 15 5) (property "Reference" "H1")
                    (pad "" np_thru_hole circle (at 0 0) (size 1 1) (drill 1) (layers "*.Cu" "*.Mask"))
                    (pad "" np_thru_hole oval (at 10 0) (size 2 1) (drill oval 2 1) (layers "*.Cu" "*.Mask")))
                  (segment (start 13 {1}) (end 17 {1}) (width 0.2) (layer "F.Cu") (net 1))
                  (segment (start 23 {1}) (end 27 {1}) (width 0.2) (layer "F.Cu") (net 1)))"#,
                5.0 + via,
                5.0 + holes
            ))
            .unwrap()
        };
        let found = |b: &Board| check(b).into_iter().map(|v| (v.kind, v.actual, v.what)).collect::<Vec<_>>();
        // 0.5 mm from the slot's rim, the edge clearance, and so from the round hole: all pass.
        assert_eq!(found(&board(1.1, 1.1)), []);
        // 0.2 mm from them: under the hole clearance (0.25 mm) of both, and of the edge clearance
        // of the slot, which is milled as the outline is.
        assert_eq!(
            found(&board(1.1, 0.8)),
            [
                ("copper_edge_clearance", 200_000, "Track (A) on F.Cu: edge clearance 0.200 mm < 0.500 mm".to_string()),
                ("hole_clearance", 200_000, "Track (A) on F.Cu to the hole of H1: clearance 0.200 mm < 0.250 mm".to_string()),
                ("hole_clearance", 200_000, "Track (A) on F.Cu to the hole of H1: clearance 0.200 mm < 0.250 mm".to_string()),
            ]
        );
        // The via's ring is 0.1 mm: with a clearance as small, copper may pass 0.1 mm from the ring
        // and still be 0.2 mm from the hole, too near it. Nearer, the copper's clearance says it alone.
        let mut b = board(0.55, 1.1);
        assert_eq!(found(&b), [("clearance", 100_000, "Track (A) on B.Cu to Via (B): clearance 0.100 mm < 0.200 mm".to_string())]);
        b.apply_pro(r#"{"board": {"design_settings": {"rules": {"min_clearance": 0.1}}}, "net_settings": {"classes": [{"name": "Default", "clearance": 0.1}]}}"#);
        assert_eq!(
            found(&b),
            [("hole_clearance", 200_000, "Track (A) on B.Cu to the hole of Via (B): clearance 0.200 mm < 0.250 mm".to_string())]
        );
        // Of the via's net, the track may: the hole is its own. Re-checked alone, the same is found.
        assert_eq!(check_items(&b, &[Item::Track(0)]).len(), 1);
        b.tracks[0].net = 2;
        assert_eq!(found(&b), []);
    }

    #[test]
    fn finds_solder_mask_openings_that_bridge_two_nets() {
        // Pads 1 mm square with the board's 0.1 mm mask margin, `gap` apart: of two parts, of one
        // part, with holes (so on both sides), of no net; a track of B `off` a pad of A's edge.
        let board = |setup: &str, gap: f64, off: f64| {
            let pad = |n: u32, x: f64, kind: &str, net: &str| {
                let (layers, drill) = if kind == "thru_hole" { (r#""*.Cu" "*.Mask""#, "(drill 0.5)") } else { (r#""F.Cu" "F.Mask""#, "") };
                format!(r#"(pad "{n}" {kind} rect (at {x} 0) (size 1 1) {drill} (layers {layers}) {net})"#)
            };
            let part = |r: &str, x: f64, y: f64, pads: &str| {
                format!(r#"(footprint "T" (layer "F.Cu") (at {x} {y}) (property "Reference" "{r}") {pads})"#)
            };
            let step = 1.0 + gap;
            let parts = [
                part("P1", 10.0, 10.0, &pad(1, 0.0, "smd", r#"(net 1 "A")"#)),
                part("P2", 10.0 + step, 10.0, &pad(1, 0.0, "smd", r#"(net 2 "B")"#)),
                part("P3", 10.0, 15.0, &[pad(1, 0.0, "smd", r#"(net 1 "A")"#), pad(2, step, "smd", r#"(net 2 "B")"#)].concat()),
                part("P4", 10.0, 20.0, &pad(1, 0.0, "thru_hole", r#"(net 1 "A")"#)),
                part("P5", 10.0 + step, 20.0, &pad(1, 0.0, "thru_hole", r#"(net 2 "B")"#)),
                part("P6", 10.0, 25.0, &pad(1, 0.0, "smd", "")),
                part("P7", 10.0 + step, 25.0, &pad(1, 0.0, "smd", "")),
                part("P8", 10.0, 30.0, &pad(1, 0.0, "smd", r#"(net 1 "A")"#)),
            ];
            let src = format!(
                r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (38 "B.Mask" user) (39 "F.Mask" user))
                  (setup (pad_to_mask_clearance 0.1) {setup}) (net 0 "") (net 1 "A") (net 2 "B") {}
                  (segment (start 8 {y}) (end 12 {y}) (width 0.2) (layer "F.Cu") (net 2)))"#,
                parts.concat(),
                y = 30.6 + off
            );
            Board::parse(&src).unwrap()
        };
        let bridges = |b: &Board| check(b).into_iter().filter(|v| v.kind == "solder_mask_bridge").map(|v| v.what).collect::<Vec<_>>();
        // 0.25 mm apart, 0.05 mm of mask is left between openings, and the track is clear of its pad's.
        assert_eq!(bridges(&board("", 0.25, 0.15)), Vec::<String>::new());
        // 0.15 mm apart the openings join; the track 0.05 mm from the pad lies bare in its opening.
        let found = bridges(&board("", 0.15, 0.05));
        assert_eq!(
            found,
            [
                "Track (B) on F.Cu is bare in the solder mask opening of P8 pad 1 (A) (front)",
                "P1 pad 1 (A) and P2 pad 1 (B): no solder mask between their openings (front)",
                "P3 pad 1 (A) and P3 pad 2 (B): no solder mask between their openings (front)",
                "P4 pad 1 (A) and P5 pad 1 (B): no solder mask between their openings (front)",
                "P4 pad 1 (A) and P5 pad 1 (B): no solder mask between their openings (back)",
                "P6 pad 1 and P7 pad 1: no solder mask between their openings (front)",
            ]
        );
        // A board that lets a part's own pads share an opening says nothing of P3's; a fab's
        // narrowest web of mask counts between openings (0.25 mm apart less 0.2 leaves 0.05 mm).
        assert_eq!(bridges(&board("(allow_soldermask_bridges_in_footprints yes)", 0.15, 0.05)).len(), 5);
        assert_eq!(bridges(&board("(solder_mask_min_width 0.1)", 0.25, 0.15)).len(), 5);
        // What a board with a wide margin breaks is said of the pads, kept as they move.
        let mut b = board("", 0.15, 0.3);
        let (vs, undo) = (super::check(&b), Patch::footprint(&b, 1));
        b.place_footprint(1, Pt { x: 12 * MM, y: 10 * MM }, 0.0);
        let after = recheck(&b, vs, &undo);
        assert_eq!((after.iter().filter(|v| v.kind == "solder_mask_bridge").count(), sorted(after) == sorted(super::check(&b))), (4, true));
    }

    #[test]
    fn finds_silkscreen_over_pads_other_silkscreen_and_the_edge() {
        // Two parts, each a pad with the mask open and two crossing lines of silkscreen (its own
        // may cross); P2 sits `dx` right of P1. A line of the board's own, and a text.
        let board = |dx: f64, extra: &str| {
            let part = |r: &str, x: f64| {
                format!(
                    r#"(footprint "T" (layer "F.Cu") (at {x} 10) (property "Reference" "{r}" (at 0 -3) (layer "F.Fab"))
                      (fp_line (start -1 -1) (end 1 1) (layer "F.SilkS") (stroke (width 0.12)))
                      (fp_line (start -1 1) (end 1 -1) (layer "F.SilkS") (stroke (width 0.12)))
                      (pad "1" smd rect (at 0 2) (size 1 1) (layers "F.Cu" "F.Mask")))"#
                )
            };
            let src = format!(
                r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user) (39 "F.Mask" user) (44 "Edge.Cuts" user) (49 "F.Fab" user))
                  (net 0 "") (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts") (stroke (width 0.05)))
                  {} {} {extra})"#,
                part("P1", 10.0),
                part("P2", 10.0 + dx)
            );
            Board::parse(&src).unwrap()
        };
        let found = |b: &Board| {
            super::check(b).into_iter().filter(|v| silk::KINDS.contains(&v.kind)).map(|v| (v.kind, v.warning, v.what)).collect::<Vec<_>>()
        };
        // Apart, nothing: a part's own lines may cross. Their ends 0.12 mm apart only touch (KiCad says nothing either).
        assert_eq!(found(&board(5.0, "")), []);
        assert_eq!(found(&board(2.12, "(gr_line (start 5 5) (end 5 8) (layer \"F.SilkS\") (stroke (width 0.12)))")), []);
        // Nearer, P2's lines cross P1's; a board's line over P1's pad, and out past the edge.
        let over = "(gr_line (start 10 12) (end 10 22) (layer \"F.SilkS\") (stroke (width 0.12)))";
        assert_eq!(
            found(&board(1.5, over)),
            [
                ("silk_over_copper", true, "A silkscreen graphic is printed over P1 pad 1, where there is no mask to print on".to_string()),
                ("silk_overlap", true, "P1's silkscreen and P2's silkscreen are printed over each other".to_string()),
                ("silk_overlap", true, "P1's silkscreen and P2's silkscreen are printed over each other".to_string()),
                ("silk_edge_clearance", true, "A silkscreen graphic is printed across the board's edge".to_string()),
            ]
        );
        // A text over a pad; one in a TrueType face is only roughly where KiCad draws it, and left out.
        let text =
            |face: &str| format!(r#"(gr_text "PAD" (at 15 12) (layer "F.SilkS") (effects (font {face} (size 1 1) (thickness 0.15))))"#);
        let said: Vec<_> = found(&board(5.0, &text(""))).into_iter().map(|v| v.2).collect();
        assert_eq!(said, ["Text \"PAD\" is printed over P2 pad 1, where there is no mask to print on"]);
        assert_eq!(found(&board(5.0, &text(r#"(face "Lato")"#))), []);
        // A project that ignores a kind isn't told it, and one may make another an error.
        let mut b = board(1.5, over);
        b.apply_pro(r#"{"board": {"design_settings": {"rule_severities": {"silk_overlap": "ignore", "silk_over_copper": "error"}}}}"#);
        let kinds: Vec<_> = found(&b).into_iter().map(|v| (v.0, v.1)).collect();
        assert_eq!(kinds, [("silk_over_copper", false), ("silk_edge_clearance", true)]);
        // After a part moves, what is found near where it was and is equals a whole check.
        let mut b = board(1.5, over);
        let (vs, undo) = (super::check(&b), Patch::footprint(&b, 1));
        b.place_footprint(1, Pt { x: 10 * MM, y: 11 * MM }, 0.0);
        let after = recheck(&b, vs, &undo);
        assert_eq!(sorted(after.clone()), sorted(super::check(&b)));
        assert!(
            after.iter().filter(|v| v.kind == "silk_overlap").count() >= 2
                && after.iter().any(|v| v.what.starts_with("P2's silkscreen is printed over P1 pad 1"))
        );
    }

    fn sorted(mut v: Vec<Violation>) -> Vec<Violation> {
        v.sort_unstable_by(|a, b| (a.kind, a.items, &a.what, a.at.x, a.at.y).cmp(&(b.kind, b.items, &b.what, b.at.x, b.at.y)));
        v
    }

    #[test]
    fn finds_parts_whose_courtyards_overlap() {
        let part = |r: &str, x: f64, layer: &str| {
            format!(
                r#"(footprint "C" (layer "F.Cu") (at {x} 0) (property "Reference" "{r}" (at 0 0 0) (layer "F.SilkS"))
                  (fp_rect (start -1 -0.5) (end 1 0.5) (stroke (width 0.05) (type default)) (fill no) (layer "{layer}")))"#
            )
        };
        let layers = r#"(layers (0 "F.Cu" signal) (31 "B.Cu" signal) (38 "B.CrtYd" user) (39 "F.CrtYd" user))"#;
        // C2's courtyard over C1's; C3's only touches C2's; C4's is on the back.
        let parts = [part("C1", 0.0, "F.CrtYd"), part("C2", 1.5, "F.CrtYd"), part("C3", 3.5, "F.CrtYd"), part("C4", 0.0, "B.CrtYd")];
        let b = Board::parse(&format!("(kicad_pcb {layers} {})", parts.concat())).unwrap();
        let found: Vec<String> = check(&b).into_iter().filter(|v| v.kind == COURTYARDS).map(|v| v.what).collect();
        assert_eq!(found, ["Courtyards of C1 and C2 overlap on F.CrtYd"]);
    }
    use crate::board::MM;
    use crate::history::{History, Pick};

    const BOARD: &str = r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (net 0 "") (net 1 "A") (net 2 "B")
              (gr_line (start 0 0) (end 20 0) (layer "Edge.Cuts") (width 0.05))
              (segment (start 5 5) (end 10 5) (width 0.15) (layer "F.Cu") (net 1))
              (segment (start 5 5.25) (end 10 5.25) (width 0.2) (layer "F.Cu") (net 2))
              (via (at 15 0.4) (size 0.4) (drill 0.25) (layers "F.Cu" "B.Cu") (net 1))
              (footprint "U" (layer "F.Cu") (at 10 10)
                (property "Reference" "U1" (at 0 0) (layer "F.SilkS"))
                (pad "1" thru_hole circle (at 0 0) (size 0.8 0.8) (drill 0.4) (layers "*.Cu") (net 1))
                (pad "2" thru_hole circle (at 0.55 0) (size 0.8 0.8) (drill 0.4) (layers "*.Cu") (net 1))))"#;

    #[test]
    fn checks_all_rules_and_project_overrides() {
        let mut b = Board::parse(BOARD).unwrap();
        let kinds: Vec<_> = check(&b).into_iter().map(|v| (v.kind, v.actual, v.limit)).collect();
        assert_eq!(
            kinds,
            [
                ("track_width", 150_000, 200_000),
                ("via_diameter", 400_000, 500_000),
                ("drill_out_of_range", 250_000, 300_000),
                ("annular_width", 75_000, 100_000),
                ("hole_to_hole", 150_000, 250_000),
                ("copper_edge_clearance", 200_000, 500_000),
                ("clearance", 75_000, 200_000),
            ]
        );
        b.apply_pro(
            r#"{"board": {"design_settings": {"rules": {"min_track_width": 0.1, "min_via_diameter": 0.4, "min_through_hole_diameter": 0.2,
              "min_via_annular_width": 0.05, "min_hole_to_hole": 0.1, "min_copper_edge_clearance": 0.1, "min_clearance": 0.05}}}}"#,
        );
        assert!(check(&b).is_empty());
    }

    #[test]
    fn two_nets_keep_the_larger_of_their_classes_clearances() {
        // Tracks 0.3 mm apart, A to B and C to D; a pad of E 0.45 mm from B's track.
        let mut b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
              (net 0 "") (net 1 "A") (net 2 "HV_B") (net 3 "C") (net 4 "D") (net 5 "E")
              (segment (start 5 5) (end 10 5) (width 0.2) (layer "F.Cu") (net 1))
              (segment (start 5 5.5) (end 10 5.5) (width 0.2) (layer "F.Cu") (net 2))
              (segment (start 5 8) (end 10 8) (width 0.2) (layer "F.Cu") (net 3))
              (segment (start 5 8.5) (end 10 8.5) (width 0.2) (layer "F.Cu") (net 4))
              (footprint "T" (layer "F.Cu") (at 7 6.55) (property "Reference" "T1")
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 5 "E"))))"#,
        )
        .unwrap();
        assert_eq!(check(&b), []);
        // HV_B's class keeps 0.5 mm: from A's track and E's pad, each of Default's 0.2 mm.
        b.apply_pro(
            r#"{"net_settings": {"classes": [{"name": "Default", "clearance": 0.2}, {"name": "HV", "clearance": 0.5}],
              "netclass_patterns": [{"netclass": "HV", "pattern": "HV_*"}]}}"#,
        );
        let found: Vec<_> = check(&b).into_iter().map(|v| (v.kind, v.actual, v.limit, v.what)).collect();
        assert_eq!(
            found,
            [
                ("clearance", 300_000, 500_000, "Track (A) on F.Cu to Track (HV_B) on F.Cu: clearance 0.300 mm < 0.500 mm".to_string()),
                ("clearance", 450_000, 500_000, "Track (HV_B) on F.Cu to T1 pad 1 (E): clearance 0.450 mm < 0.500 mm".to_string()),
            ]
        );
        // Checked again for the track alone, the same two.
        assert_eq!(check_items(&b, &[Item::Track(1)]).len(), 2);
    }

    #[test]
    fn lets_clearances_fall_short_by_half_a_micron_as_kicad() {
        // Pairs of tracks, vias by the edge and holes: 500 nm short of the rule, then 501 nm.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (net 0 "") (net 1 "A") (net 2 "B") (net 3 "C") (net 4 "D") (net 5 "H")
              (gr_line (start 0 0) (end 20 0) (layer "Edge.Cuts") (width 0.05))
              (segment (start 5 5) (end 10 5) (width 0.2) (layer "F.Cu") (net 1))
              (segment (start 5 5.3995) (end 10 5.3995) (width 0.2) (layer "F.Cu") (net 2))
              (segment (start 5 8) (end 10 8) (width 0.2) (layer "F.Cu") (net 3))
              (segment (start 5 8.399499) (end 10 8.399499) (width 0.2) (layer "F.Cu") (net 4))
              (via (at 15 0.7995) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1))
              (via (at 17 0.799499) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 3))
              (via (at 5 3) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 5))
              (via (at 5.5495 3) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 5))
              (via (at 8 3) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 5))
              (via (at 8.549499 3) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 5)))"#,
        )
        .unwrap();
        let found: Vec<_> = check(&b).into_iter().map(|v| (v.kind, v.actual)).collect();
        assert_eq!(found, [("hole_to_hole", 249_499), ("copper_edge_clearance", 499_499), ("clearance", 199_499)]);
    }

    #[test]
    fn holes_of_vias_that_share_no_layer_never_meet() {
        // Two vias of one net 0.35 mm apart, their 0.3 mm holes 0.05 mm from each other: through
        // the board both, they are too close; one above the other in the stack, they never meet.
        let board = |a: &str, c: &str| {
            Board::parse(&format!(
                r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A")
                  (via {a} (at 5 5) (size 0.6) (drill 0.3) (net 1))
                  (via {c} (at 5.35 5) (size 0.6) (drill 0.3) (net 1)))"#
            ))
            .unwrap()
        };
        let kinds = |b: &Board| check(b).into_iter().filter(|v| !v.warning).map(|v| v.kind).collect::<Vec<_>>();
        let (through, top, low) = (r#"(layers "F.Cu" "B.Cu")"#, r#"blind (layers "F.Cu" "In1.Cu")"#, r#"blind (layers "In2.Cu" "B.Cu")"#);
        assert_eq!(kinds(&board(through, through)), ["hole_to_hole"]);
        assert_eq!(kinds(&board(top, through)), ["hole_to_hole"]);
        assert!(kinds(&board(top, low)).is_empty());
    }

    #[test]
    fn calls_touching_copper_of_two_nets_a_short() {
        // A's track ends on B's via; another of A passes 0.1 mm from it.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "B")
              (via (at 5 5) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 2))
              (segment (start 0 5) (end 5 5) (width 0.2) (layer "F.Cu") (net 1))
              (segment (start 0 5.5) (end 10 5.5) (width 0.2) (layer "B.Cu") (net 1)))"#,
        )
        .unwrap();
        let said: Vec<_> = check(&b).into_iter().map(|v| (v.kind, v.what)).collect();
        assert_eq!(
            said,
            [
                ("clearance", "Track (A) on F.Cu and Via (B) touch: a short between their nets".to_string()),
                ("clearance", "Track (A) on B.Cu to Via (B): clearance 0.100 mm < 0.200 mm".to_string()),
            ]
        );
    }

    #[test]
    fn checks_every_side_of_a_closed_outline() {
        // The via is 0.2 mm from the rectangle's last side, the one that closes the polygon.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts") (width 0.05))
              (via (at 0.5 5) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 0)))"#,
        )
        .unwrap();
        let found: Vec<_> = check(&b).into_iter().map(|v| (v.kind, v.actual)).collect();
        assert_eq!(found, [("copper_edge_clearance", 200_000)]);
    }

    #[test]
    fn measures_arc_tracks_to_arc_tracks() {
        // Half circles 2 and 2.35 mm around (10, 10), 0.2 mm wide: 0.15 mm of copper apart.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "B")
              (arc (start 8 10) (mid 10 8) (end 12 10) (width 0.2) (layer "F.Cu") (net 1))
              (arc (start 7.65 10) (mid 10 7.65) (end 12.35 10) (width 0.2) (layer "F.Cu") (net 2)))"#,
        )
        .unwrap();
        let found: Vec<_> = check(&b).into_iter().map(|v| (v.kind, v.actual)).collect();
        assert_eq!(found, [("clearance", 150_000)]);
    }

    #[test]
    fn measures_arcs_apart_exactly() {
        let arc = |s: (f64, f64), m: (f64, f64), e: (f64, f64)| {
            let p = |(x, y): (f64, f64)| Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm };
            arc_center(p(s), p(m), p(e)).unwrap()
        };
        let um = |d: f64| (d / 1e3).round();
        let (c, s) = (3f64.sqrt() / 2.0, 0.5f64.sqrt());
        // Unit circles bulging towards each other, nearest inside both; crossing; on one circle,
        // nearest at their ends.
        let facing = arcs_apart(arc((c, -0.5), (1.0, 0.0), (c, 0.5)), arc((3.0 - c, 0.5), (2.0, 0.0), (3.0 - c, -0.5)));
        let crossing = arcs_apart(arc((1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)), arc((2.0, 0.0), (1.0, -1.0), (0.0, 0.0)));
        let ends = arcs_apart(arc((1.0, 0.0), (s, s), (0.0, 1.0)), arc((-1.0, 0.0), (-s, -s), (0.0, -1.0)));
        assert_eq!([um(facing), um(crossing), um(ends)], [1000.0, 0.0, 1414.0]);
    }

    #[test]
    fn measures_custom_pads_to_all_their_copper() {
        // Custom pads with a 1 mm anchor and an arm along its top, making an L. A (net A): a
        // track 0.15 mm under its arm, in the L's corner, a via on the arm, the edge 0.3 mm from
        // its end. C and D: arms 0.12 mm apart, each in the other's corner. A small custom pad
        // (F) inside a big one (E). The anchors are clear of the rest.
        let custom = |at: &str, net: &str, anchor: f64, [x0, y0, x1, y1]: [f64; 4]| {
            format!(
                r#"(footprint "X" (layer "F.Cu") (at {at}) (pad "1" smd custom (at 0 0) (size {anchor} {anchor}) (layers "F.Cu") (net {net})
                  (options (clearance outline) (anchor rect)) (primitives (gr_poly (pts (xy {x0} {y0}) (xy {x1} {y0}) (xy {x1} {y1}) (xy {x0} {y1})) (width 0) (fill yes)))))"#
            )
        };
        let b = Board::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (net 0 "") (net 1 "A") (net 2 "B") (net 3 "C") (net 4 "D") (net 5 "E") (net 6 "F")
              (gr_line (start 13.3 9) (end 13.3 11) (layer "Edge.Cuts") (width 0.05))
              (segment (start 11 10.15) (end 12.6 10.15) (width 0.2) (layer "F.Cu") (net 2))
              (via (at 12 9.7) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 2))
              {} {} {} {} {})"#,
            custom("10 10", r#"1 "A""#, 1.0, [0.5, -0.5, 3.0, -0.1]),
            custom("30 10", r#"3 "C""#, 1.0, [0.5, -0.5, 3.0, -0.1]),
            custom("36 10.52", r#"4 "D""#, 1.0, [-3.5, -0.5, -0.5, -0.1]),
            custom("50 10", r#"5 "E""#, 1.0, [-2.0, -2.0, 2.0, 2.0]),
            custom("51 10", r#"6 "F""#, 0.4, [-0.3, -0.3, 0.3, 0.3]),
        ))
        .unwrap();
        let mut found: Vec<_> = check(&b).into_iter().map(|v| (v.kind, v.actual)).collect();
        found.sort_unstable();
        assert_eq!(
            found,
            [("clearance", 0), ("clearance", 0), ("clearance", 120_000), ("clearance", 150_000), ("copper_edge_clearance", 300_000)]
        );
    }

    #[test]
    fn measures_to_polygons_with_holes() {
        // A 10 mm square with a 4 mm square hole in its middle.
        let sq = |at: Nm, side: Nm| {
            vec![Pt { x: at, y: at }, Pt { x: at + side, y: at }, Pt { x: at + side, y: at + side }, Pt { x: at, y: at + side }]
        };
        let poly = [sq(0, 10 * MM), sq(3 * MM, 4 * MM)];
        let disc = |x: Nm, y: Nm, r: Nm| RBox::disc(Pt { x: x * MM, y: y * MM }, r * MM).dist_poly(&poly) / MM;
        // Outside, on it, in the hole, touching it; a segment across the hole.
        assert_eq!([disc(15, 5, 2), disc(1, 1, 0), disc(5, 5, 1), disc(12, 5, 2)], [3, 0, 1, 0]);
        assert_eq!(RBox::segment(Pt { x: -5 * MM, y: 5 * MM }, Pt { x: 20 * MM, y: 5 * MM }, 0).dist_poly(&poly), 0);
    }

    #[test]
    fn rechecks_every_items_edge_when_a_part_moves_the_outline() {
        // J1 draws a notch of the outline on Edge.Cuts; a track runs 0.2 mm from it.
        let src = r#"(kicad_pcb (layers (0 "F.Cu" signal) (44 "Edge.Cuts" user)) (net 0 "") (net 1 "A")
              (gr_line (start 0 0) (end 20 0) (layer "Edge.Cuts") (width 0.05))
              (footprint "J" (layer "F.Cu") (at 10 5) (fp_line (start -2 0) (end 2 0) (layer "Edge.Cuts") (width 0.05)))
              (segment (start 8 5.3) (end 12 5.3) (width 0.2) (layer "F.Cu") (net 1)))"#;
        let mut b = Board::parse(src).unwrap();
        let vs = super::check(&b);
        assert_eq!(vs.iter().map(|v| v.kind).collect::<Vec<_>>(), ["copper_edge_clearance", "track_dangling"]);
        // J1 moved away takes the notch with it: the track keeps clear of every edge.
        let undo = Patch::footprint(&b, 0);
        b.place_footprint(0, Pt { x: 10 * MM, y: 15 * MM }, 0.0);
        assert_eq!(recheck(&b, vs, &undo), super::check(&b));
    }

    #[test]
    fn rechecks_only_what_a_change_changed() {
        let (mut b, mut h) = (Board::parse(BOARD).unwrap(), History::default());
        let sorted = |mut v: Vec<Violation>| {
            v.sort_unstable_by_key(|v| (v.kind, v.items));
            v
        };
        // U1 moves next to the via and the edge, a new track crosses net B's, then the first
        // track and the via (one edit, moving the others up) and U1 go; after each edit and each
        // undo the list is what a full check finds, and it changed.
        let place: &dyn Fn(&mut Board) -> Patch = &|b| {
            let undo = Patch::footprint(b, 0);
            b.place_footprint(0, Pt { x: 15_300_000, y: 600_000 }, 0.0);
            undo
        };
        let route: &dyn Fn(&mut Board) -> Patch = &|b| {
            let undo = Patch::added(b);
            b.add_tracks(&[Pt { x: 7 * MM, y: 4 * MM }, Pt { x: 7 * MM, y: 6 * MM }], 200_000, 0, 1);
            undo
        };
        let mut vs = super::check(&b);
        let track_and_via: &dyn Fn(&mut Board) -> Patch =
            &|b| b.apply([Pick::Track(0), Pick::Via(0)].map(|p| Patch::delete(b, p)).into_iter().collect());
        for edit in [place, route, track_and_via, &|b| b.apply(Patch::delete(b, Pick::Footprint(0)))] {
            let (before, undo) = (sorted(vs.clone()), edit(&mut b));
            vs = sorted(recheck(&b, vs, &undo));
            assert_eq!(vs, sorted(super::check(&b)));
            assert_ne!(vs, before);
            h.push(undo, String::new());
        }
        while let Some((_, redo)) = h.undo(&mut b) {
            let before = vs.clone();
            vs = sorted(recheck(&b, vs, redo));
            assert_eq!(vs, sorted(super::check(&b)));
            assert_ne!(vs, before);
        }
    }
}
