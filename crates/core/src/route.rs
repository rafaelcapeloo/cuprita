//! Routing: octilinear track paths around other nets' copper, holes and the board edge, keeping
//! the design rule clearances, or across the tracks and vias a shove moves aside (`shove.rs`).

use crate::board::{Board, CuLayer, LayerId, LayerMask, MM, NetId, Nm, Pt, Span, Track, Via, nearest, square, union};
use crate::drc::{self, Item, RBox, RTree, Violation, arc_pts, edges, pad_box};
use crate::history::Patch;
use crate::shove::Shoved;
use std::ops::Range;

/// Grid moves, 45° apart.
const DIRS: [(i32, i32); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
/// Search cost of a straight and a diagonal step.
const STEP: [u32; 2] = [100, 141];
/// Extra cost of going on straight, turning 45° and turning 90°, so paths keep few, gentle
/// corners; sharper turns are never taken.
const TURN: [u32; 3] = [0, 300, 1000];
/// Search states a path may settle before it stops at the closest point it reached.
pub(crate) const MAX_SETTLED: usize = 200_000;
/// Grid cells along a tile side; a tile's blocked cells are found when a search first needs them.
const TILE: i32 = 64;
/// Buckets of open search states; must exceed the most a step can add to the estimated total.
const RING: usize = 2048;
const _: () = assert!(2 * STEP[1] + TURN[2] + 1 < RING as u32);
/// The track widths a route steps through (mm) on a board whose project lists none.
const WIDTHS: [f64; 12] = [0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.8, 1.0, 1.5, 2.0];

/// Where a route starts: a point on a pad, via or track of `net`, routed on `layer` with
/// `width` tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Start {
    pub at: Pt,
    pub net: NetId,
    pub layer: LayerId,
    pub width: Nm,
}

/// Other nets' copper, a hole or the edge: shape, a custom pad's copper (its shape then is its
/// box), the distance from them a track centre must keep, the box that distance reaches, and
/// whether a shove can move it (an unlocked straight track or via: `shove.rs`).
type Obstacle = (RBox, Vec<Vec<Pt>>, f64, (Pt, Pt), bool);

/// Finds paths for tracks of one net and width on one copper layer, on a grid through the
/// route's start. Build one per route and layer; it keeps what it learns about the board, so
/// the other nets must not change while it is in use.
pub struct Router {
    step: Nm,
    /// Board point of cell (0, 0), the grid size in cells and its width in tiles.
    origin: Pt,
    cells: (i32, i32),
    tiles_x: i32,
    obstacles: Vec<Obstacle>,
    tree: RTree,
    /// Two bits per cell, set when a track centred there would be too close to an obstacle: to
    /// any (plane 0), to one a shove can't move (plane 1).
    tiles: Vec<Option<Box<[[u64; TILE as usize]; 2]>>>,
    /// Where a path may end on the net's own copper on the layer: its pads (centre and shape),
    /// and its via centres and track ends (with their radius).
    pads: Vec<(Pt, RBox)>,
    ends: Vec<(Pt, Nm)>,
    /// Where the way is clear, whether a bend's diagonal comes first (else last): of two paths as
    /// short, the one bending this way round.
    pub diagonal_first: bool,
}

impl Router {
    /// A router for `width` tracks of `net` on `layer`, with a grid point on `start`.
    pub fn new(b: &Board, net: NetId, layer: LayerId, width: Nm, start: Pt) -> Self {
        Self::on_grid(b, net, layer, width, start, (start, 0))
    }

    /// A router as `new`'s whose grid takes in every point of the grid through `at` every
    /// `step` (the user's): its own step is then the coarsest whole fraction of `step` fine
    /// enough (from 10 µm; without one, as `new`'s), so paths aimed at those points end on them.
    pub fn on_grid(b: &Board, net: NetId, layer: LayerId, width: Nm, start: Pt, grid: (Pt, Nm)) -> Self {
        Self::build(b, [net, net], layer, width, start, grid, 0)
    }

    /// A router for the middle line of a differential pair of `nets`, `width` across both its
    /// tracks and their gap, on `layer`: the copper of either net is its own. Its grid has a
    /// point on `start` and is no finer than `step`, so the line's corners lie far enough apart
    /// for the two tracks to follow them.
    pub(crate) fn pair(b: &Board, nets: [NetId; 2], layer: LayerId, width: Nm, start: Pt, step: Nm) -> Self {
        Self::build(b, nets, layer, width, start, (start, 0), step)
    }

    /// `on_grid`'s router, for a track of either of `nets` (two of a pair, else one twice), its
    /// own grid's step at least `coarse`.
    fn build(b: &Board, nets: [NetId; 2], layer: LayerId, width: Nm, start: Pt, (at, step): (Pt, Nm), coarse: Nm) -> Self {
        let (r, on) = (&b.rules, 1 << layer);
        let (other, own) = (|n: NetId| n == 0 || !nets.contains(&n), |n: NetId| n != 0 && nets.contains(&n));
        // What the net keeps from copper of another: the larger of their classes' clearances.
        let clearances = b.clearances();
        let keeps = |n: NetId| clearances.get(n as usize).copied().unwrap_or(r.clearance);
        let own_keeps = keeps(nets[0]).max(keeps(nets[1]));
        let from = |n: NetId| own_keeps.max(keeps(n));
        // Fine enough to find the gaps the rules leave between copper.
        let fine = ((width + own_keeps) / 8).clamp(10_000, 50_000).max(coarse);
        let (at, step) = match (1.max((step + fine - 1) / fine)..=step / 10_000).find(|k| step % k == 0) {
            Some(k) => (at, step / k),
            None => (start, fine),
        };
        // A diagonal step between two clear cell centres passes up to step² / (4 · reach) closer
        // to a corner than its ends do: keep that much more, and a little for rounding.
        let margin = (step as f64).powi(2) / (2.0 * width as f64 + 4.0 * own_keeps as f64) + 10.0;
        let mut obstacles = vec![];
        let mut add = |shape: RBox, copper: Vec<Vec<Pt>>, clearance: Nm, movable: bool| {
            let keep = shape.r + width as f64 / 2.0 + clearance as f64 + margin;
            obstacles.push((shape, copper, keep, pad_box(shape.bounds, (keep - shape.r).ceil() as Nm), movable));
        };
        for t in b.tracks.iter().filter(|t| t.layer == layer && other(t.net)) {
            match t.mid {
                Some(m) => {
                    arc_pts(t.a, m, t.b).windows(2).for_each(|w| add(RBox::segment(w[0], w[1], t.width / 2), vec![], from(t.net), false))
                }
                None => add(RBox::segment(t.a, t.b, t.width / 2), vec![], from(t.net), !t.locked),
            }
        }
        // Holes keep the hole clearance from other nets' copper (and from any where their pad has none).
        for v in b.vias.iter().filter(|v| v.layers & on != 0 && other(v.net)) {
            add(RBox::disc(v.at, v.size / 2), vec![], from(v.net), !v.locked);
            add(RBox::disc(v.at, v.drill / 2), vec![], r.hole_clearance, !v.locked);
        }
        for p in &b.pads {
            if p.layers & on != 0 && other(p.net) {
                add(RBox::pad(p), p.copper(), from(p.net), false);
            }
            if let Some((dw, dh)) = p.drill.filter(|_| other(p.net) || p.layers & on == 0) {
                add(RBox::boxed(p.at, (dw, dh), p.angle, dw.min(dh) / 2), vec![], r.hole_clearance, false);
            }
        }
        let outline = edges(b);
        let board = outline.iter().map(|e| e.bounds).reduce(union);
        outline.into_iter().for_each(|e| add(e, vec![], r.edge_clearance, false));
        let pads: Vec<_> = b.pads.iter().filter(|p| p.layers & on != 0 && own(p.net)).map(|p| (p.at, RBox::pad(p))).collect();
        let tracks = b.tracks.iter().filter(|t| t.layer == layer && own(t.net));
        let mut ends: Vec<(Pt, Nm)> = tracks.flat_map(|t| [(t.a, t.width / 2), (t.b, t.width / 2)]).collect();
        ends.extend(b.vias.iter().filter(|v| v.layers & on != 0 && own(v.net)).map(|v| (v.at, v.size / 2)));
        // The grid covers the board outline (or, with none, the copper and room around it) and
        // `start`, and has a cell centre on `at`.
        let (lo, hi) = match board {
            Some(outline) => union(outline, square(start, 0)),
            None => {
                let boxes = obstacles.iter().map(|o| o.3).chain(pads.iter().map(|p| p.1.bounds));
                pad_box(boxes.fold(square(start, 0), union), 10 * MM)
            }
        };
        let s = step as i64;
        let first = |lo: Nm, at: Nm| at as i64 - (at as i64 - lo as i64 + s - 1) / s * s;
        let origin = Pt { x: first(lo.x, at.x) as Nm, y: first(lo.y, at.y) as Nm };
        let cells = (((hi.x as i64 - origin.x as i64) / s + 1) as i32, ((hi.y as i64 - origin.y as i64) / s + 1) as i32);
        let tiles_x = (cells.0 + TILE - 1) / TILE;
        let tiles = vec![None; (tiles_x * ((cells.1 + TILE - 1) / TILE)) as usize];
        let tree = RTree::new(obstacles.iter().enumerate().map(|(i, o)| (o.3, i as u32)).collect());
        Self { step, origin, cells, tiles_x, obstacles, tree, tiles, pads, ends, diagonal_first: false }
    }

    /// Where a path aimed at `p` should end: the centre of the net's pad there, or its via or
    /// track end within `tol`; elsewhere the nearest grid point.
    pub fn snap(&self, p: Pt, tol: Nm) -> Pt {
        let d = |q: &Pt| (q.x as f64 - p.x as f64).hypot(q.y as f64 - p.y as f64);
        let pads = self.pads.iter().filter(|(_, s)| s.core_pt(p.x as f64, p.y as f64) - s.r <= tol as f64).map(|&(q, _)| q);
        let ends = self.ends.iter().filter(|&(q, r)| d(q) <= (r + tol) as f64).map(|&(q, _)| q);
        pads.chain(ends).min_by(|a, b| d(a).total_cmp(&d(b))).unwrap_or_else(|| self.pt(self.cell(p)))
    }

    /// The grid point nearest `p`, and the grid's step.
    pub(crate) fn on_cell(&self, p: Pt) -> Pt {
        self.pt(self.cell(p))
    }

    pub(crate) fn step(&self) -> Nm {
        self.step
    }

    /// Whether a track ending at `p` connects to the net: `p` is one of its pad or via centres or
    /// track ends on the layer.
    pub fn connects(&self, p: Pt) -> bool {
        self.pads.iter().any(|q| q.0 == p) || self.ends.iter().any(|e| e.0 == p)
    }

    /// The corners of a track path from `from` (a grid point) towards `to` (from `snap`): it ends
    /// on `to`, or as close to it as other copper, holes and the edge let it get. Only its ends
    /// may be closer to them than the clearance: where it starts, and where it `connects`.
    pub fn path(&mut self, from: Pt, to: Pt) -> Vec<Pt> {
        self.search(from, to, false, MAX_SETTLED).0
    }

    /// `path`'s search, across the tracks and vias a shove moves when `through`, settling at most
    /// `settle` states; with how many it settled.
    pub(crate) fn search(&mut self, from: Pt, to: Pt, through: bool, settle: usize) -> (Vec<Pt>, usize) {
        let own = self.connects(to);
        let (start, mut goal) = (self.cell(from), self.cell(to));
        let mut exact = true;
        if !own && !self.free(goal, through) {
            // Aim for the clear cell nearest `to` on the way back to the start.
            let n = (goal.0 - start.0).abs().max((goal.1 - start.1).abs());
            let back = |k: i32| (goal.0 + (start.0 - goal.0) * k / n, goal.1 + (start.1 - goal.1) * k / n);
            goal = (1..=n).map(back).find(|&c| self.free(c, through)).unwrap_or(goal);
            exact = false;
        }
        // A* over (cell, heading) states, so that turns can cost extra; heading 8 is the start's.
        let h = |(x, y): (i32, i32)| {
            let (dx, dy) = ((x - goal.0).unsigned_abs(), (y - goal.1).unsigned_abs());
            STEP[0] * dx.max(dy) + (STEP[1] - STEP[0]) * dx.min(dy)
        };
        let mut marks = vec![None; self.tiles.len()];
        // Open states in a ring of buckets by estimated total cost, which never falls and rises
        // by less than RING from one settled state to the states it opens.
        let mut open = vec![Vec::new(); RING];
        let (mut at, mut queued) = (h(start) as usize, 1);
        open[at % RING].push((0, start, 8u8));
        let (mut closest, mut settled, mut reached) = ((h(start), 0, start, 8u8), 0, goal == start);
        while queued > 0 {
            while open[at % RING].is_empty() {
                at += 1;
            }
            let (cost, c, d) = open[at % RING].pop().expect("not empty");
            queued -= 1;
            // A state found again at a lower cost after this entry was queued.
            if d < 8 && *mark(&mut marks, self.tiles_x, c, d) >> 4 <= cost {
                continue;
            }
            closest = closest.min((h(c), cost, c, d));
            settled += 1;
            if c == goal || settled > settle {
                reached = c == goal;
                break;
            }
            for nd in 0..8u8 {
                let turn = if d == 8 { 0 } else { (nd + 8 - d) % 8 };
                let turn = turn.min(8 - turn) as usize;
                let n = (c.0 + DIRS[nd as usize].0, c.1 + DIRS[nd as usize].1);
                if turn > 2 || !(self.free(n, through) || (own && n == goal)) {
                    continue;
                }
                // A 45° bend the other way round costs a little more (a diagonal heading is odd).
                let other_way = turn == 1 && (d % 2 == 1) != self.diagonal_first;
                let next = cost + STEP[nd as usize % 2] + TURN[turn] + other_way as u32;
                let m = mark(&mut marks, self.tiles_x, n, nd);
                if *m == 0 || next < (*m >> 4) - 1 {
                    *m = (next + 1) << 4 | d as u32;
                    open[(next + h(n)) as usize % RING].push((next, n, nd));
                    queued += 1;
                }
            }
        }
        // Walk back to the start, keeping only the cells where the heading changes; with no step
        // taken, straight to `to` when it is in the start's cell, else nowhere (not onto the cell).
        let (_, _, mut c, mut d) = closest;
        if d == 8 {
            return (if reached && exact && to != from { vec![from, to] } else { vec![from] }, settled);
        }
        let mut pts = vec![self.pt(c)];
        if reached && exact && to != pts[0] {
            pts.insert(0, to);
        }
        while d != 8 {
            let before = (*mark(&mut marks, self.tiles_x, c, d) & 15) as u8;
            c = (c.0 - DIRS[d as usize].0, c.1 - DIRS[d as usize].1);
            if before != d {
                pts.push(self.pt(c));
            }
            d = before;
        }
        if pts.last() != Some(&from) {
            pts.push(from);
        }
        pts.reverse();
        (pts, settled)
    }

    fn pt(&self, (x, y): (i32, i32)) -> Pt {
        let at = |o: Nm, i: i32| (o as i64 + i as i64 * self.step as i64) as Nm;
        Pt { x: at(self.origin.x, x), y: at(self.origin.y, y) }
    }

    fn cell(&self, p: Pt) -> (i32, i32) {
        let at = |v: Nm, o: Nm| ((v as i64 - o as i64) as f64 / self.step as f64).round() as i32;
        (at(p.x, self.origin.x), at(p.y, self.origin.y))
    }

    /// Whether a track centred on cell `c` keeps clear of every obstacle, or `through` those a
    /// shove moves, of the others (off the grid it doesn't).
    fn free(&mut self, (x, y): (i32, i32), through: bool) -> bool {
        if !(0..self.cells.0).contains(&x) || !(0..self.cells.1).contains(&y) {
            return false;
        }
        let t = ((y / TILE) * self.tiles_x + x / TILE) as usize;
        if self.tiles[t].is_none() {
            self.tiles[t] = Some(self.raster(x / TILE * TILE, y / TILE * TILE));
        }
        self.tiles[t].as_ref().is_some_and(|tile| tile[through as usize][(y % TILE) as usize] >> (x % TILE) & 1 == 0)
    }

    /// The blocked cells of the tile whose first cell is (x0, y0): by any obstacle, by those a
    /// shove can't move.
    fn raster(&self, x0: i32, y0: i32) -> Box<[[u64; TILE as usize]; 2]> {
        let mut bits = Box::new([[0u64; TILE as usize]; 2]);
        let (x1, y1) = ((x0 + TILE).min(self.cells.0) - 1, (y0 + TILE).min(self.cells.1) - 1);
        // The tile's cells whose centres lie between lo and hi.
        let span = |lo: Nm, hi: Nm, o: Nm, c0: i32, c1: i32| {
            let at = |v: Nm| (v as i64 - o as i64) as f64 / self.step as f64;
            (at(lo).ceil() as i32).max(c0)..=(at(hi).floor() as i32).min(c1)
        };
        self.tree.query((self.pt((x0, y0)), self.pt((x1, y1))), |i| {
            let (shape, copper, keep, (lo, hi), movable) = &self.obstacles[i as usize];
            for y in span(lo.y, hi.y, self.origin.y, y0, y1) {
                let row = (y - y0) as usize;
                if !copper.is_empty() {
                    // A custom pad's copper (it never moves), by the stretches of the row within
                    // reach of it: a row costs what its outline does, not each cell.
                    let (o, step) = (self.origin.x as f64, self.step as f64);
                    for (a, b) in near_row(copper, self.pt((x0, y)).y as f64, *keep) {
                        let (first, last) = ((((a - o) / step).floor() as i32 + 1).max(x0), (((b - o) / step).ceil() as i32 - 1).min(x1));
                        if first <= last {
                            let run = u64::MAX >> (63 - (last - first)) << (first - x0);
                            (bits[0][row], bits[1][row]) = (bits[0][row] | run, bits[1][row] | run);
                        }
                    }
                    continue;
                }
                for x in span(lo.x, hi.x, self.origin.x, x0, x1) {
                    let bit = 1 << (x - x0);
                    // Blocked already as far as this obstacle could block it.
                    if bits[1][row] & bit != 0 || *movable && bits[0][row] & bit != 0 {
                        continue;
                    }
                    let p = self.pt((x, y));
                    if shape.core_pt(p.x as f64, p.y as f64) < *keep {
                        bits[0][row] |= bit;
                        if !movable {
                            bits[1][row] |= bit;
                        }
                    }
                }
            }
        });
        bits
    }
}

/// The stretches (x from, to) of the row at `y` nearer than `keep` to `copper` (outlines with the
/// holes inside them, even-odd): where the row runs inside it, and near each of its sides.
fn near_row(copper: &[Vec<Pt>], y: f64, keep: f64) -> Vec<(f64, f64)> {
    let sides = || {
        copper
            .iter()
            .flat_map(|c| c.iter().zip(c.iter().cycle().skip(1)))
            .map(|(p, q)| ((p.x as f64, p.y as f64), (q.x as f64, q.y as f64)))
    };
    let mut inside: Vec<f64> =
        sides().filter(|(p, q)| (p.1 > y) != (q.1 > y)).map(|(p, q)| p.0 + (y - p.1) * (q.0 - p.0) / (q.1 - p.1)).collect();
    inside.sort_by(f64::total_cmp);
    let mut out: Vec<(f64, f64)> = inside.as_chunks::<2>().0.iter().map(|&[a, b]| (a, b)).collect();
    for (p, q) in sides() {
        // Near a side: near either end, or near its line where the nearest point lies between them.
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for (cx, cy) in [p, q] {
            if (y - cy).abs() < keep {
                let w = (keep * keep - (y - cy) * (y - cy)).sqrt();
                (lo, hi) = (lo.min(cx - w), hi.max(cx + w));
            }
        }
        let ((dx, dy), dy0) = ((q.0 - p.0, q.1 - p.1), y - p.1);
        let l2 = dx * dx + dy * dy;
        // Near the line, then between the ends (the nearest point's place along the side, from
        // 0 to l2, runs with x unless the side is upright).
        let (a, b) = match (dy != 0.0, dy0.abs() < keep) {
            (true, _) => (p.0 + dx * dy0 / dy - (keep * l2.sqrt() / dy).abs(), p.0 + dx * dy0 / dy + (keep * l2.sqrt() / dy).abs()),
            (false, true) => (f64::NEG_INFINITY, f64::INFINITY),
            _ => (1.0, 0.0),
        };
        let (a, b) = if dx != 0.0 {
            let (s, t) = (p.0 - dy0 * dy / dx, p.0 + (l2 - dy0 * dy) / dx);
            (a.max(s.min(t)), b.min(s.max(t)))
        } else if l2 > 0.0 && (0.0..=l2).contains(&(dy0 * dy)) {
            (a, b)
        } else {
            (1.0, 0.0)
        };
        if a < b {
            (lo, hi) = (lo.min(a), hi.max(b));
        }
        if lo < hi {
            out.push((lo, hi));
        }
    }
    out
}

impl Board {
    /// Where a route clicked at `p` (within `tol`) starts, on the copper `layers` in view listed
    /// top first (the active layer first): on the pad, via or track `copper_at` finds there. A
    /// pad or via starts at its centre, on the first of `layers` it has copper on (a through-hole
    /// pad's or a via's: the active layer); a track at its end or nearest point, on its layer.
    /// New tracks get their net's class's width, or the width of the track they start on.
    pub fn route_start(&self, p: Pt, tol: Nm, layers: &[LayerId]) -> Option<Start> {
        let layer = |mask: LayerMask| layers.iter().copied().find(|&l| mask & 1 << l != 0);
        let start = |at: Pt, net: NetId, mask: LayerMask| Some(Start { at, net, layer: layer(mask)?, width: self.track_width(net) });
        match self.copper_at(p, tol, layers)? {
            Item::Pad(i) => start(self.pads[i].at, self.pads[i].net, self.pads[i].layers),
            Item::Via(i) => start(self.vias[i].at, self.vias[i].net, self.vias[i].layers),
            Item::Track(i) => {
                let (t, d) = (&self.tracks[i], |q: Pt| (q.x as f64 - p.x as f64).hypot(q.y as f64 - p.y as f64));
                let end = if d(t.a) <= d(t.b) { t.a } else { t.b };
                let at = if t.mid.is_some() || d(end) <= tol as f64 { end } else { nearest(t.a, t.b, p) };
                Some(Start { at, net: t.net, layer: t.layer, width: t.width.max(self.rules.track_width) })
            }
        }
    }

    /// A through via of `net` at `at`, sized by its net's class.
    pub fn via(&self, at: Pt, net: NetId) -> Via {
        let (size, drill) =
            self.class(net).map_or((self.rules.class_via_diameter, self.rules.class_via_drill), |c| (c.via_diameter, c.via_drill));
        Via { at, size, drill, layers: self.copper, net, locked: false }
    }

    /// A via of `net` at `at` through the copper layers from `a` to `b` and those between them
    /// only: on a board with more, blind or buried. Sized by its net's class.
    pub fn via_between(&self, at: Pt, net: NetId, a: LayerId, b: LayerId) -> Via {
        let depth = |l: LayerId| self.stackup.iter().position(|c| c.layer == l);
        let layers = match (depth(a), depth(b)) {
            (Some(i), Some(j)) => self.stackup[i.min(j)..=i.max(j)].iter().fold(0, |m: LayerMask, c| m | 1 << c.layer),
            _ => self.copper,
        };
        Via { layers, ..self.via(at, net) }
    }

    /// How far `v` goes through the board; a microvia reads as blind.
    pub fn span(&self, v: &Via) -> Span {
        let on = |c: Option<&CuLayer>| c.is_some_and(|c| v.layers >> c.layer & 1 != 0);
        match (on(self.stackup.first()), on(self.stackup.last())) {
            _ if v.layers & self.copper == self.copper => Span::Through,
            (true, true) => Span::Through,
            (false, false) => Span::Buried,
            _ => Span::Blind,
        }
    }

    /// Adds `via` at `at` if it keeps the design rules; else returns the first rule it would break.
    pub fn add_via(&mut self, at: Pt, net: NetId) -> Result<usize, Violation> {
        self.add_route(&Shoved::default(), vec![], Some(self.via(at, net))).map(|_| self.vias.len() - 1)
    }

    /// Adds a route's `tracks` and `via` after the board's last, with what `shoved` moves out of
    /// their way, and returns the patch that undoes it all; a via that breaks a design rule is
    /// refused, nothing changed, with the first rule it breaks.
    pub fn add_route(&mut self, shoved: &Shoved, tracks: Vec<Track>, via: Option<Via>) -> Result<Patch, Violation> {
        let undo = self.apply(shoved.patch(self, tracks, via));
        match via.and_then(|_| drc::check_items(self, &[Item::Via(self.vias.len() - 1)]).into_iter().find(|v| !v.warning)) {
            Some(v) => {
                self.apply(undo);
                Err(v)
            }
            None => Ok(undo),
        }
    }

    /// Whether a pad or via of `net` centred on `at` has copper on both layers `a` and `b`, so a
    /// track goes on from one to the other there without a new via.
    pub fn joins_layers(&self, at: Pt, net: NetId, a: LayerId, b: LayerId) -> bool {
        let both: LayerMask = 1 << a | 1 << b;
        let joins = |q: Pt, n: NetId, m: LayerMask| q == at && n == net && m & both == both;
        self.pads.iter().any(|p| joins(p.at, p.net, p.layers)) || self.vias.iter().any(|v| joins(v.at, v.net, v.layers))
    }

    /// The next wider (or narrower) track width than `width` a route steps to: of the net
    /// classes' and the project's own (`track_widths`), else common ones; none below the rules'
    /// minimum. `None` past the last.
    pub fn step_width(&self, width: Nm, wider: bool) -> Option<Nm> {
        let common = WIDTHS.iter().map(|mm| (mm * 1e6).round() as Nm);
        let own: Vec<Nm> = if self.track_widths.is_empty() { common.collect() } else { self.track_widths.clone() };
        let classes = self.classes.iter().map(|c| c.track_width).chain([self.rules.class_track_width]);
        let widths = own.into_iter().chain(classes).filter(|&w| w >= self.rules.track_width);
        if wider { widths.filter(|&w| w > width).min() } else { widths.filter(|&w| w < width).max() }
    }

    /// Adds tracks along the corners of `path` and returns their indices.
    pub fn add_tracks(&mut self, path: &[Pt], width: Nm, layer: LayerId, net: NetId) -> Range<usize> {
        let first = self.tracks.len();
        self.tracks.extend(tracks(path, width, layer, net));
        first..self.tracks.len()
    }
}

/// The tracks along the corners of `path`.
pub fn tracks(path: &[Pt], width: Nm, layer: LayerId, net: NetId) -> Vec<Track> {
    path.windows(2).filter(|w| w[0] != w[1]).map(|w| Track { a: w[0], b: w[1], mid: None, width, layer, net, locked: false }).collect()
}

/// The search mark of cell `c` with heading `d`, `(cost + 1) << 4 | heading before` (0: not
/// reached yet); a tile's marks are allocated when first used.
fn mark(marks: &mut [Option<Box<[u32]>>], tiles_x: i32, (x, y): (i32, i32), d: u8) -> &mut u32 {
    let tile = marks[((y / TILE) * tiles_x + x / TILE) as usize].get_or_insert_with(|| vec![0; (TILE * TILE * 8) as usize].into());
    &mut tile[((y % TILE * TILE + x % TILE) * 8 + d as i32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Net A pads at (3, 5) and (17, 5) mm with a 2 x 4 mm net B pad between them, on a 20 x 10 mm board.
    const BOARD: &str = r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
      (net 0 "") (net 1 "A") (net 2 "B")
      (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts") (width 0.05))
      (footprint "T" (layer "F.Cu") (at 3 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
      (footprint "U" (layer "F.Cu") (at 17 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
      (footprint "R" (layer "F.Cu") (at 10 5) (pad "1" smd rect (at 0 0) (size 2 4) (layers "F.Cu") (net 2 "B"))))"#;

    fn mm(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    #[test]
    fn walks_around_other_nets_to_the_pad() {
        let mut b = Board::parse(BOARD).unwrap();
        let s = b.route_start(mm(3.2, 5.1), 0, &[0, 1]).unwrap();
        assert_eq!(s, Start { at: mm(3.0, 5.0), net: 1, layer: 0, width: 250_000 });
        let mut r = Router::new(&b, s.net, s.layer, s.width, s.at);
        let to = r.snap(mm(16.8, 5.3), 0);
        let path = r.path(s.at, to);
        // Up and over (or down and under) the B pad, its 0.2 mm clearance and half the track
        // width away on the 0.05 mm grid, then down to the pad centre.
        let over = [mm(3.0, 5.0), mm(5.35, 2.65), mm(14.65, 2.65), mm(17.0, 5.0)];
        let under = over.map(|p| Pt { x: p.x, y: 10 * MM - p.y });
        assert!(path == over || path == under, "{path:?}");
        let new: Vec<_> = b.add_tracks(&path, s.width, s.layer, s.net).map(Item::Track).collect();
        assert_eq!((new.len(), drc::check_items(&b, &new)), (3, vec![]));
    }

    #[test]
    fn ends_on_its_own_pad_even_where_other_copper_crowds_it() {
        // A net B track 0.3 mm from U1's centre: a track may end there, but not pass by.
        let track = r#"(net 2 "B"))) (segment (start 17.4 3) (end 17.4 7) (width 0.2) (layer "F.Cu") (net 2)))"#;
        let b = Board::parse(&BOARD.replace(r#"(net 2 "B"))))"#, track)).unwrap();
        assert_eq!(b.tracks.len(), 1);
        let mut r = Router::new(&b, 1, 0, 250_000, mm(3.0, 5.0));
        let path = r.path(mm(3.0, 5.0), r.snap(mm(17.0, 5.0), 0));
        assert_eq!(path.last(), Some(&mm(17.0, 5.0)));
        // Aimed just past the B track instead, it stops where it keeps the clearance.
        let past = r.snap(mm(17.65, 5.0), 0);
        assert_eq!(r.path(mm(3.0, 5.0), past).last(), Some(&mm(16.95, 5.0)));
    }

    #[test]
    fn keeps_clear_of_custom_pads_copper_not_their_box() {
        // Net B's custom pad is an L: a bar over x 9..11, y 3..7, and a foot to x = 12.5 along
        // its bottom. U1 (net A) sits in the corner of the L's box.
        let custom = r#"smd custom (at 0 0) (size 0.2 0.2) (layers "F.Cu") (net 2 "B") (options (anchor rect))
          (primitives (gr_poly (pts (xy -1 -2) (xy 1 -2) (xy 1 1.5) (xy 2.5 1.5) (xy 2.5 2) (xy -1 2)) (width 0) (fill yes)))"#;
        let src = BOARD.replace("(at 17 5)", "(at 12 4)").replace(r#"smd rect (at 0 0) (size 2 4) (layers "F.Cu") (net 2 "B")"#, custom);
        let mut b = Board::parse(&src).unwrap();
        let mut r = Router::new(&b, 1, 0, 250_000, mm(3.0, 5.0));
        let path = r.path(mm(3.0, 5.0), r.snap(mm(12.0, 4.0), 0));
        assert_eq!(path.last(), Some(&mm(12.0, 4.0)));
        let new: Vec<_> = b.add_tracks(&path, 250_000, 0, 1).map(Item::Track).collect();
        assert_eq!(drc::check_items(&b, &new), vec![]);
        // Aimed inside the bar, it stops on the way back, as for any pad.
        let mut r = Router::new(&b, 1, 0, 250_000, mm(3.0, 5.0));
        assert_eq!(r.path(mm(3.0, 5.0), r.snap(mm(10.0, 3.5), 0)), [mm(3.0, 5.0), mm(7.4, 5.0), mm(8.65, 3.75)]);
    }

    #[test]
    fn custom_copper_rows_are_what_each_point_measures() {
        // An L with a square hole in its bar; within 0.3 mm of it, off the grid of the samples.
        let ring = |pts: &[(f64, f64)]| pts.iter().map(|&(x, y)| mm(x, y)).collect::<Vec<_>>();
        let hole = ring(&[(0.5, 1.0), (1.5, 1.0), (1.5, 2.0), (0.5, 2.0)]);
        let copper = vec![ring(&[(0.0, 0.0), (2.0, 0.0), (2.0, 4.0), (5.0, 4.0), (5.0, 5.0), (0.0, 5.0)]), hole];
        let keep = 300_000.0;
        for y in (-40..=60).map(|i| i * 97_000 + 13) {
            let rows = near_row(&copper, y as f64, keep);
            for x in (-40..=60).map(|i| i * 91_000 + 7) {
                let d = RBox::disc(Pt { x, y }, 0).dist_poly(&copper) as f64;
                if (d - keep).abs() > 2.0 {
                    assert_eq!(rows.iter().any(|&(a, b)| a < x as f64 && (x as f64) < b), d < keep, "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn starts_on_the_layer_on_top_where_the_copper_reaches_it() {
        // Net A: a pad through the board at (3, 5), a pad on B.Cu (layer 1) at (17, 5) and a via
        // at (10, 8); net B's via at (10, 2).
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "B")
              (footprint "J" (layer "F.Cu") (at 3 5) (pad "1" thru_hole circle (at 0 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu") (net 1 "A")))
              (footprint "U" (layer "B.Cu") (at 17 5) (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu") (net 1 "A")))
              (via (at 10 8) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1))
              (via (at 10 2) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 2)))"#,
        )
        .unwrap();
        let layer = |x, y, layers: &[LayerId]| b.route_start(mm(x, y), 0, layers).map(|s| s.layer);
        // The pad through the board and the via start on the layer on top (the active one), the
        // B.Cu pad on its own.
        let [front, back] = [[0, 1], [1, 0]];
        assert_eq!([layer(3.0, 5.0, &front), layer(3.0, 5.0, &back), layer(10.0, 8.0, &back)], [Some(0), Some(1), Some(1)]);
        assert_eq!([layer(17.0, 5.0, &front), layer(17.0, 5.0, &[0])], [Some(1), None]);
        // A track changes layer on J's pad and net A's via without a new via, nowhere else.
        let joins = |x, y| b.joins_layers(mm(x, y), 1, 0, 1);
        assert_eq!(
            [joins(3.0, 5.0), joins(10.0, 8.0), joins(17.0, 5.0), joins(10.0, 2.0), joins(3.1, 5.0)],
            [true, true, false, false, false]
        );
    }

    #[test]
    fn bends_with_the_diagonal_last_or_first() {
        let b = Board::parse(BOARD).unwrap();
        let mut r = Router::new(&b, 1, 0, 250_000, mm(3.0, 5.0));
        // 5 mm across and 2 mm down, clear of the B pad: the diagonal last, else first.
        assert_eq!(r.path(mm(3.0, 5.0), mm(8.0, 7.0)), [mm(3.0, 5.0), mm(6.0, 5.0), mm(8.0, 7.0)]);
        r.diagonal_first = true;
        assert_eq!(r.path(mm(3.0, 5.0), mm(8.0, 7.0)), [mm(3.0, 5.0), mm(5.0, 7.0), mm(8.0, 7.0)]);
        // Around the B pad it walks as before.
        let path = r.path(mm(3.0, 5.0), r.snap(mm(17.0, 5.0), 0));
        assert!(path[1] == mm(5.35, 2.65) || path[1] == mm(5.35, 7.35), "{path:?}");
        assert_eq!((path.len(), path.last()), (4, Some(&mm(17.0, 5.0))));
    }

    #[test]
    fn a_nets_class_sets_its_tracks_width_and_how_far_they_keep() {
        let mut b = Board::parse(BOARD).unwrap();
        b.apply_pro(
            r#"{"net_settings": {"classes": [{"name": "Default", "clearance": 0.2, "track_width": 0.25},
              {"name": "Power", "clearance": 0.4, "track_width": 0.5}], "netclass_patterns": [{"netclass": "Power", "pattern": "A"}]}}"#,
        );
        // A's route starts 0.5 mm wide, and passes the B pad by its class's 0.4 mm and half its
        // width: 0.65 mm, so 0.7 mm on the 0.05 mm grid, where Default's 0.25 mm track kept
        // 0.325 mm (`walks_around_other_nets_to_the_pad`).
        let s = b.route_start(mm(3.2, 5.1), 0, &[0, 1]).unwrap();
        assert_eq!(s, Start { at: mm(3.0, 5.0), net: 1, layer: 0, width: 500_000 });
        let mut r = Router::new(&b, s.net, s.layer, s.width, s.at);
        let to = r.snap(mm(16.8, 5.3), 0);
        let path = r.path(s.at, to);
        let over = [mm(3.0, 5.0), mm(5.7, 2.3), mm(14.3, 2.3), mm(17.0, 5.0)];
        let under = over.map(|p| Pt { x: p.x, y: 10 * MM - p.y });
        assert!(path == over || path == under, "{path:?}");
        let new: Vec<_> = b.add_tracks(&path, s.width, s.layer, s.net).map(Item::Track).collect();
        assert_eq!((new.len(), drc::check_items(&b, &new)), (3, vec![]));
    }

    #[test]
    fn steps_through_the_project_track_widths_above_the_minimum() {
        let mut b = Board::parse(BOARD).unwrap();
        let step = |b: &Board, mm: f64, wider| b.step_width((mm * 1e6) as Nm, wider).map(|w| w as f64 / 1e6);
        // Common widths without the project's, the net class's 0.25 mm among them; none below
        // the 0.2 mm minimum.
        assert_eq!(
            [step(&b, 0.25, true), step(&b, 0.25, false), step(&b, 0.2, false), step(&b, 2.0, true)],
            [Some(0.3), Some(0.2), None, None]
        );
        // The project's, KiCad's 0 for "the net class's" left out; a track's own width steps too.
        b.apply_pro(r#"{"board": {"design_settings": {"track_widths": [0.0, 0.1, 0.4, 1.0], "rules": {"min_track_width": 0.15}}}}"#);
        assert_eq!(b.track_widths, [100_000, 400_000, 1_000_000]);
        assert_eq!(
            [step(&b, 0.25, true), step(&b, 0.4, false), step(&b, 0.25, false), step(&b, 1.0, true), step(&b, 0.33, false)],
            [Some(0.4), Some(0.25), None, None, Some(0.25)]
        );
    }

    #[test]
    fn ends_on_the_users_grid_points_square_to_it() {
        let mut b = Board::parse(BOARD).unwrap();
        // A 10 mil grid through (0.1, 0) mm: the pad at (3, 5) is off it, the aim on it.
        let (at, step, aim) = (mm(0.1, 0.0), 254_000, mm(0.1 + 20.0 * 0.254, 4.0 * 0.254));
        let mut r = Router::on_grid(&b, 1, 0, 250_000, mm(3.0, 5.0), (at, step));
        let path = r.path(mm(3.0, 5.0), aim);
        let on_cells = |p: &Pt| (p.x - at.x) % 31_750 == 0 && (p.y - at.y) % 31_750 == 0;
        let square = |w: &[Pt]| w[0].x == w[1].x || w[0].y == w[1].y || (w[1].x - w[0].x).abs() == (w[1].y - w[0].y).abs();
        // It starts on the pad's centre, steps onto the router's cells (a 31.75 µm fraction of the
        // grid's) inside the pad, and goes on at multiples of 45° to the aim.
        assert_eq!((path[0], path.last()), (mm(3.0, 5.0), Some(&aim)));
        assert!(path[1..].iter().all(on_cells) && path[1..].windows(2).all(square), "{path:?}");
        assert!((path[1].x - path[0].x).abs().max((path[1].y - path[0].y).abs()) <= 31_750 / 2, "{path:?}");
        // Aimed into the B pad from where no step towards it keeps clear: nowhere, not even its cell.
        assert_eq!(r.path(mm(8.68, 5.0), mm(10.0, 5.0)), [mm(8.68, 5.0)]);
        let new: Vec<_> = b.add_tracks(&path, 250_000, 0, 1).map(Item::Track).collect();
        assert_eq!(drc::check_items(&b, &new), vec![]);
        // Without a grid, or one no step from 10 µm divides, its cells go through the start.
        assert_eq!(r.snap(mm(5.02, 5.0), 0), mm(0.1 + 155.0 * 0.03175, 0.0 + 157.0 * 0.03175));
        for grid in [(at, 0), (at, 5_000)] {
            let r = Router::on_grid(&b, 1, 0, 250_000, mm(3.0, 5.0), grid);
            assert_eq!(r.snap(mm(5.02, 5.0), 0), mm(5.0, 5.0));
        }
    }

    #[test]
    fn adds_a_via_only_where_it_keeps_the_rules() {
        let mut b = Board::parse(BOARD).unwrap();
        // A 0.8 mm via 0.1 mm from the B pad is too close; 1 mm away it is fine.
        let e = b.add_via(mm(8.5, 5.0), 1).unwrap_err();
        assert_eq!((e.kind, e.actual, b.vias.len()), ("clearance", 100_000, 0));
        assert_eq!(b.add_via(mm(7.6, 5.0), 1), Ok(0));
        assert_eq!((b.vias[0].size, b.vias[0].drill, b.vias[0].layers), (800_000, 400_000, b.copper));
    }

    #[test]
    fn a_via_between_layers_stops_inside_the_board_and_saves_as_one_that_does() {
        // Six layers of copper; a track of A on the top, one of B on the fourth layer.
        let board = |version: u32| {
            format!(
                r#"(kicad_pcb (version {version}) (generator "pcbnew")
	(layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (8 "In3.Cu" signal) (10 "In4.Cu" signal) (2 "B.Cu" signal))
	(net 0 "") (net 1 "A") (net 2 "B")
	(segment (start 0 0) (end 5 0) (width 0.25) (layer "F.Cu") (net 1))
	(segment (start 0 5) (end 5 5) (width 0.25) (layer "In3.Cu") (net 2))
)
"#
            )
        };
        let (ten, eight) = (board(20260206), board(20240108));
        let mut b = Board::parse(&ten).unwrap();
        let layer = |b: &Board, name: &str| b.layers.iter().position(|l| l == name).unwrap() as LayerId;
        let (top, in1, in2, in4, bottom) =
            (layer(&b, "F.Cu"), layer(&b, "In1.Cu"), layer(&b, "In2.Cu"), layer(&b, "In4.Cu"), layer(&b, "B.Cu"));
        // From the top to the second inner layer: three layers, blind; inner to inner, buried;
        // top to bottom, or a layer the board lacks, through.
        let blind = b.via_between(mm(5.0, 0.0), 1, in2, top);
        let buried = b.via_between(mm(5.0, 5.0), 2, in2, in4);
        assert_eq!((blind.layers.count_ones(), buried.layers.count_ones()), (3, 3));
        assert_eq!((b.span(&blind), b.span(&buried), blind.size, blind.drill), (Span::Blind, Span::Buried, 800_000, 400_000));
        assert_eq!(b.span(&b.via_between(mm(0.0, 0.0), 1, top, bottom)), Span::Through);
        assert_eq!(b.via_between(mm(0.0, 0.0), 1, top, 99).layers, b.copper);
        assert_eq!(blind.layers & 1 << in1, 1 << in1);
        b.vias.extend([blind, buried, b.via(mm(9.0, 9.0), 1)]);
        // Each is written as its KiCad calls it, and reads back through the same layers.
        let out = b.write(&ten).unwrap();
        assert!(out.contains("(via blind\n\t\t(at 5 0)") && out.contains("(layers \"F.Cu\" \"In2.Cu\")"), "{out}");
        assert!(out.contains("(via buried\n\t\t(at 5 5)") && out.contains("(layers \"In2.Cu\" \"In4.Cu\")"), "{out}");
        assert!(out.contains("(via\n\t\t(at 9 9)") && out.contains("(layers \"F.Cu\" \"B.Cu\")"), "{out}");
        assert_eq!(Board::parse(&out).unwrap().vias, b.vias);
        // Before KiCad 10 a buried via is a blind one.
        let mut b = Board::parse(&eight).unwrap();
        b.vias.push(buried);
        let out = b.write(&eight).unwrap();
        assert!(out.contains("(via blind\n\t\t(at 5 5)") && !out.contains("buried"), "{out}");
        assert_eq!(Board::parse(&out).unwrap().vias, b.vias);
    }

    #[test]
    fn crosses_what_a_shove_moves_and_walks_round_the_rest() {
        // A net B track across the way from (3, 5) to (8, 5), up from the board's bottom edge.
        let track = r#"(net 2 "B"))) (segment (start 6 4) (end 6 10) (width 0.25) (layer "F.Cu") (net 2)))"#;
        let b = Board::parse(&BOARD.replace(r#"(net 2 "B"))))"#, track)).unwrap();
        let mut r = Router::new(&b, 1, 0, 250_000, mm(3.0, 5.0));
        let around = r.path(mm(3.0, 5.0), mm(8.0, 5.0));
        assert!(around.len() > 2, "{around:?}");
        let through = |r: &mut Router, to: Pt| r.search(mm(3.0, 5.0), to, true, MAX_SETTLED).0;
        assert_eq!(through(&mut r, mm(8.0, 5.0)), [mm(3.0, 5.0), mm(8.0, 5.0)]);
        // Pads and locked tracks it walks round either way.
        let b = Board::parse(&BOARD.replace(r#"(net 2 "B"))))"#, &track.replace("(net 2)))", "(locked yes) (net 2)))"))).unwrap();
        let mut r = Router::new(&b, 1, 0, 250_000, mm(3.0, 5.0));
        assert_eq!(through(&mut r, mm(8.0, 5.0)), around);
        let to = r.snap(mm(17.0, 5.0), 0);
        assert_eq!(through(&mut r, to).len(), 4);
    }

    #[test]
    fn stops_short_of_other_copper_and_the_edge() {
        let b = Board::parse(BOARD).unwrap();
        let mut r = Router::new(&b, 1, 0, 250_000, mm(3.0, 5.0));
        // Onto the B pad, then off the board.
        for (aim, end) in [(mm(10.0, 5.0), mm(8.65, 5.0)), (mm(3.0, -2.0), mm(3.0, 0.65))] {
            let to = r.snap(aim, 0);
            assert_eq!(r.path(mm(3.0, 5.0), to), [mm(3.0, 5.0), end]);
        }
    }
}
