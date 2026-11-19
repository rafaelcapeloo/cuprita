//! Push and shove, as KiCad's Shove mode: other nets' tracks and vias make way for a track being
//! routed. The stretch of a track in its way moves aside by what the clearance needs, joined to
//! the rest at 45°; a via in its way moves aside with its tracks' ends; what they push moves in
//! turn, up to a limit. Pads, holes, the board edge, arcs, locked tracks and vias and the routed
//! net's own copper never move: where only moving them would make room, the shove fails.

use crate::board::{Board, LayerId, LayerMask, NetId, Nm, Pt, Rules, Track, Via, square, union};
use crate::drc::{Item, RBox, RTree, SLACK, arc_pts, cu_label, edges, overlaps, pad_box, track_box};
use crate::history::Patch;
use crate::route::{MAX_SETTLED, Router};
use crate::tr::{t, tr, trf};
use std::collections::{HashMap, VecDeque};
use std::ops::Range;

/// Tracks and vias a shove may push (each push counts) before it gives up.
const LIMIT: usize = 64;
/// Tracks a run is followed each way from the one in the way: farther joints stay put, so moving
/// a long meander costs what moving a short track does.
const RUN: usize = 32;
/// Room kept past the clearance (nm), for corners rounded to whole nanometres.
const MARGIN: f64 = 5.0;
/// How deep (nm) copper must reach into the room another keeps to be in its way: less is
/// rounding, or copper just at the clearance.
const DEPTH: f64 = 2.0 * MARGIN;
/// Items in the shove's index: the index times 4 plus one of these.
const TRACK: u32 = 0;
const VIA: u32 = 1;
const PAD: u32 = 2;
const EDGE: u32 = 3;

/// What a shove moves: each run of tracks (their indices, in order along it) with its corners
/// now, and each via with where it goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shoved {
    pub runs: Vec<(Vec<usize>, Vec<Pt>)>,
    pub vias: Vec<(usize, Pt)>,
}

/// Why a shove can't make room: what can't move, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct Stuck {
    pub at: Pt,
    pub what: String,
}

/// Moves other nets' tracks and vias out of the way of a route of one net. Build one per route,
/// and again once other nets' copper changes.
pub struct Shover {
    net: NetId,
    /// What it keeps: the clearances from copper, the edge and holes, the hole spacing; and
    /// from copper of another net, each net its class's clearance (`Board::clearances`).
    rules: Rules,
    clearances: Vec<Nm>,
    /// Every track, via and pad, and the pieces of the board edge (`edges`), by their boxes.
    tree: RTree,
    edges: Vec<RBox>,
}

/// A piece of copper that pushes: a track's segment, or a via's disc (`a == b`, `hole` its
/// drill's radius); `r` is its half width or radius.
#[derive(Clone, Copy, Debug)]
struct Cu {
    a: Pt,
    b: Pt,
    r: Nm,
    hole: Nm,
    layers: LayerMask,
    net: NetId,
}

impl Cu {
    fn track(t: &Track) -> Self {
        Cu { a: t.a, b: t.b, r: t.width / 2, hole: 0, layers: 1 << t.layer, net: t.net }
    }

    fn via(v: &Via) -> Self {
        Cu { a: v.at, b: v.at, r: v.size / 2, hole: v.drill / 2, layers: v.layers, net: v.net }
    }

    fn shape(&self) -> RBox {
        RBox::segment(self.a, self.b, self.r)
    }

    /// Its box, `by` wider each way.
    fn reach(&self, by: Nm) -> (Pt, Pt) {
        let (lo, hi) =
            (Pt { x: self.a.x.min(self.b.x), y: self.a.y.min(self.b.y) }, Pt { x: self.a.x.max(self.b.x), y: self.a.y.max(self.b.y) });
        pad_box((lo, hi), self.r + by)
    }
}

/// An item being shoved: a run of tracks joined end to end (indices in order, corners as read,
/// when last pushing and now), or a via (where it was, was when last pushing, and is).
enum Moved {
    Run { tracks: Vec<usize>, was: Vec<Pt>, seen: Vec<Pt>, pts: Vec<Pt>, proto: Track },
    Via { i: usize, was: Pt, seen: Pt, at: Pt },
}

impl Moved {
    /// Whether some of its copper may lie in box `reach`.
    fn near(&self, b: &Board, reach: (Pt, Pt)) -> bool {
        let (pts, r) = match self {
            Moved::Run { pts, proto, .. } => (&pts[..], proto.width / 2),
            Moved::Via { i, at, .. } => (std::slice::from_ref(at), b.vias[*i].size / 2),
        };
        let lo = pts.iter().fold(pts[0], |l, p| Pt { x: l.x.min(p.x), y: l.y.min(p.y) });
        let hi = pts.iter().fold(pts[0], |h, p| Pt { x: h.x.max(p.x), y: h.y.max(p.y) });
        overlaps(pad_box((lo, hi), r), reach)
    }

    /// Its copper as it is now.
    fn copper(&self, b: &Board) -> Vec<Cu> {
        match self {
            Moved::Run { pts, proto, .. } => pts.windows(2).map(|w| Cu::track(&Track { a: w[0], b: w[1], ..*proto })).collect(),
            Moved::Via { i, at, .. } => vec![Cu { a: *at, b: *at, ..Cu::via(&b.vias[*i]) }],
        }
    }

    /// Its copper that moved since it last pushed (a run's segments off those it had, a via),
    /// which now pushes.
    fn fresh(&mut self, b: &Board) -> Vec<Cu> {
        let mut cu = self.copper(b);
        match self {
            Moved::Run { seen, pts, .. } => {
                cu = cu.drain(span(seen, pts)).collect();
                *seen = pts.clone();
            }
            Moved::Via { seen, at, .. } => {
                cu.retain(|_| seen != at);
                *seen = *at;
            }
        }
        cu
    }

    /// Makes all it moved push again.
    fn forget(&mut self) {
        match self {
            Moved::Run { was, seen, .. } => *seen = was.clone(),
            Moved::Via { was, seen, .. } => *seen = *was,
        }
    }
}

/// The items a shove moves, and which of them each track and via is in.
#[derive(Default)]
struct State {
    moved: Vec<Moved>,
    owner: HashMap<Item, usize>,
}

impl State {
    fn shoved(&self) -> Shoved {
        let mut s = Shoved::default();
        for m in &self.moved {
            match m {
                Moved::Run { tracks, was, pts, .. } if pts != was => s.runs.push((tracks.clone(), pts.clone())),
                Moved::Via { i, was, at, .. } if at != was => s.vias.push((*i, *at)),
                _ => {}
            }
        }
        s
    }
}

impl Shover {
    /// A shover for routes of `net` on `b`.
    pub fn new(b: &Board, net: NetId) -> Self {
        let edges = edges(b);
        let hole =
            |p: &crate::board::Pad| p.drill.map_or(RBox::pad(p).bounds, |d| union(RBox::pad(p).bounds, hole_box(p.at, d, p.angle).bounds));
        let tracks = b.tracks.iter().enumerate().map(|(i, t)| (track_box(t).bounds, (i as u32) << 2 | TRACK));
        let vias = b.vias.iter().enumerate().map(|(i, v)| (square(v.at, v.size / 2), (i as u32) << 2 | VIA));
        let pads = b.pads.iter().enumerate().map(|(i, p)| (hole(p), (i as u32) << 2 | PAD));
        let edge = edges.iter().enumerate().map(|(i, e)| (e.bounds, (i as u32) << 2 | EDGE));
        let tree = RTree::new(tracks.chain(vias).chain(pads).chain(edge).collect());
        Shover { net, rules: b.rules, clearances: b.clearances(), tree, edges }
    }

    /// A route step that shoves: `r`'s path from `from` towards `to` (a track `width` wide on
    /// `layer`) through what a shove moves, and the shove; where that can't make room, the path
    /// round it all instead, and why. Both searches together settle 3/4 of the states one may
    /// (the first one half), as the first reaches farther than a walk round would.
    pub fn step(&self, b: &Board, r: &mut Router, (from, to): (Pt, Pt), width: Nm, layer: LayerId) -> (Vec<Pt>, Result<Shoved, Stuck>) {
        let (path, settled) = r.search(from, to, true, MAX_SETTLED / 2);
        match self.shove(b, &path, width, layer, None) {
            Ok(s) => (path, Ok(s)),
            Err(e) => (r.search(from, to, false, (MAX_SETTLED * 3 / 4).saturating_sub(settled)).0, Err(e)),
        }
    }

    /// Moves other nets' tracks and vias out of the way of the track `path` (`width` wide, on
    /// `layer`) and of `via` at its end: what moves, or why that can't make room.
    pub fn shove(&self, b: &Board, path: &[Pt], width: Nm, layer: LayerId, via: Option<&Via>) -> Result<Shoved, Stuck> {
        let segments = path.windows(2).filter(|w| w[0] != w[1]);
        let mut head: Vec<Cu> =
            segments.map(|w| Cu { a: w[0], b: w[1], r: width / 2, hole: 0, layers: 1 << layer, net: self.net }).collect();
        head.extend(via.map(Cu::via));
        let (mut st, mut queue, mut pushes) = (State::default(), VecDeque::from([None]), 0);
        // The route pushes first, then each item it moved, with what moved. Each gives way to the
        // route and the items moved before it, and pushes those after: none pushes back.
        'next: while let Some(me) = queue.pop_front() {
            let pushing = me.map_or_else(|| head.clone(), |m: usize| st.moved[m].fresh(b));
            for s in &pushing {
                while let Some(item) = self.hit(b, &st, s, me)? {
                    pushes += 1;
                    if pushes > LIMIT {
                        return Err(Stuck { at: s.a, what: trf("more than {0} tracks and vias would move", &[&LIMIT]) });
                    }
                    let m = self.mover(b, &mut st, item);
                    // In the way of one moved before it, `me` goes round that, and pushes again.
                    let back = me.filter(|&p| m < p);
                    for moved in self.push(b, &mut st, back.unwrap_or(m), &head)? {
                        if !queue.contains(&Some(moved)) {
                            queue.push_back(Some(moved));
                        }
                    }
                    if let Some(p) = back {
                        st.moved[p].forget();
                        continue 'next;
                    }
                }
            }
        }
        Ok(st.shoved())
    }

    /// What item `m` gives way to: the route's copper and that of the items moved before it,
    /// near it.
    fn walls(&self, b: &Board, st: &State, head: &[Cu], m: usize) -> Vec<Cu> {
        let reach = self.reach();
        let mine = st.moved[m].copper(b).iter().map(|c| c.reach(reach)).reduce(union).expect("an item has copper");
        let before = st.moved[..m].iter().filter(|k| k.near(b, mine)).flat_map(|k| k.copper(b));
        head.iter().copied().chain(before).filter(|w| overlaps(w.reach(0), mine)).collect()
    }

    /// The first item copper `s` (of item `me`, `None` for the route) comes too close to, as an
    /// item moved (`Err` for one that can't move); a via before a track, as a via takes its
    /// tracks' ends along.
    fn hit(&self, b: &Board, st: &State, s: &Cu, me: Option<usize>) -> Result<Option<Item>, Stuck> {
        let reach = s.reach(self.reach());
        let (mut via, mut track, mut stuck) = (None, None, None);
        self.tree.query(reach, |id| {
            let (kind, i) = (id & 3, (id >> 2) as usize);
            let item = if kind == VIA { Item::Via(i) } else { Item::Track(i) };
            if stuck.is_some() || kind <= VIA && st.owner.contains_key(&item) || !self.too_close(b, s, kind, i) {
                return;
            }
            match (kind, self.fixed(b, kind, i)) {
                (TRACK, None) => track = track.or(Some(item)),
                (VIA, None) => via = via.or(Some(item)),
                _ => stuck = Some(self.stuck(b, s, kind, i)),
            }
        });
        if let Some(stuck) = stuck {
            return Err(stuck);
        }
        let mut moved = st.moved.iter().enumerate().filter(|&(m, k)| Some(m) != me && k.near(b, reach));
        let near = |c: &Cu| overlaps(c.reach(0), reach) && self.clash(s, c);
        let moved = moved.find(|(_, m)| m.copper(b).iter().any(near)).map(|(_, item)| match item {
            Moved::Via { i, .. } => Item::Via(*i),
            Moved::Run { tracks, .. } => Item::Track(tracks[0]),
        });
        Ok(via.or(moved.filter(|i| matches!(i, Item::Via(_)))).or(track).or(moved))
    }

    /// The item for board item `item`, made when it first moves: a via, or the run of tracks
    /// joined end to end that a track is in.
    fn mover(&self, b: &Board, st: &mut State, item: Item) -> usize {
        if let Some(&m) = st.owner.get(&item) {
            return m;
        }
        let m = st.moved.len();
        match item {
            Item::Via(i) => st.moved.push(Moved::Via { i, was: b.vias[i].at, seen: b.vias[i].at, at: b.vias[i].at }),
            _ => {
                let Item::Track(i) = item else { unreachable!("pads never move") };
                let t = b.tracks[i];
                let ahead = self.follow(b, st, i, t.b);
                // A run in a loop came round to its first track's other end.
                let back = if ahead.last().map(|e| e.1) == Some(t.a) { vec![] } else { self.follow(b, st, i, t.a) };
                let tracks: Vec<usize> = back.iter().rev().map(|e| e.0).chain([i]).chain(ahead.iter().map(|e| e.0)).collect();
                let pts: Vec<Pt> = back.iter().rev().map(|e| e.1).chain([t.a, t.b]).chain(ahead.iter().map(|e| e.1)).collect();
                st.owner.extend(tracks.iter().map(|&k| (Item::Track(k), m)));
                st.moved.push(Moved::Run { tracks, was: pts.clone(), seen: pts.clone(), pts, proto: t });
                return m;
            }
        }
        st.owner.insert(item, m);
        m
    }

    /// The tracks on from track `i`'s end `at`, each with its far end, while each joint has just
    /// two tracks of the net on the layer, both free to move, as wide and in no other item moved,
    /// and no via or pad of the net; at most `RUN` of them.
    fn follow(&self, b: &Board, st: &State, i: usize, at: Pt) -> Vec<(usize, Pt)> {
        let (t0, mut out, mut last, mut at) = (b.tracks[i], vec![], i, at);
        while out.len() < RUN {
            let (mut next, mut joint) = (vec![], false);
            self.tree.query(square(at, 0), |id| {
                let k = (id >> 2) as usize;
                match id & 3 {
                    TRACK => {
                        let t = &b.tracks[k];
                        if k != last && t.layer == t0.layer && t.net == t0.net && (t.a == at || t.b == at) {
                            next.push(k);
                        }
                    }
                    VIA => joint |= b.vias[k].at == at && b.vias[k].net == t0.net && b.vias[k].layers & 1 << t0.layer != 0,
                    PAD => joint |= b.pads[k].net == t0.net && b.pads[k].layers & 1 << t0.layer != 0 && b.pads[k].holds(at, 0),
                    _ => {}
                }
            });
            let (&[k], false) = (&next[..], joint) else { break };
            let t = &b.tracks[k];
            // Back at the first track, round a loop.
            if k == i || t.width != t0.width || st.owner.contains_key(&Item::Track(k)) || self.fixed(b, TRACK, k).is_some() {
                break;
            }
            at = if t.a == at { t.b } else { t.a };
            out.push((k, at));
            last = k;
        }
        out
    }

    /// Moves item `m` out of the way of what it gives way to (`walls`; `head` is the route's
    /// copper): a run round it, a via off it with its tracks' ends. Returns the items that moved.
    fn push(&self, b: &Board, st: &mut State, m: usize, head: &[Cu]) -> Result<Vec<usize>, Stuck> {
        let walls = self.walls(b, st, head, m);
        let Moved::Via { i, at: from, .. } = st.moved[m] else {
            self.reroute(b, st, m, &walls, head)?;
            return Ok(vec![m]);
        };
        let v = &b.vias[i];
        let hulls: Vec<Vec<P>> =
            walls.iter().filter_map(|w| self.keep(w, &Cu::via(v)).map(|r| hull(w.a, w.b, r as f64 + MARGIN))).collect();
        let mut to = (from.x as f64, from.y as f64);
        for _ in 0..8 {
            match hulls.iter().find(|h| depth(h, to) > DEPTH) {
                Some(h) => to = out_of(h, to),
                None => break,
            }
        }
        let to = Pt { x: to.0.round() as Nm, y: to.1.round() as Nm };
        let label = || cu_label(b, Item::Via(i));
        if hulls.iter().any(|h| depth(h, (to.x as f64, to.y as f64)) > DEPTH) {
            return Err(Stuck { at: from, what: trf("{0} has no room to move", &[&label()]) });
        }
        if let Some(stuck) = self.blocked(b, &Cu { a: to, b: to, ..Cu::via(v) }) {
            return Err(stuck);
        }
        if let Moved::Via { at, .. } = &mut st.moved[m] {
            *at = to;
        }
        // Its tracks' ends go with it.
        let mut ends = vec![];
        self.tree.query(square(from, 0), |id| {
            let k = (id >> 2) as usize;
            if id & 3 == TRACK && b.tracks[k].net == v.net && (b.tracks[k].a == from || b.tracks[k].b == from) {
                ends.push(k);
            }
        });
        let mut moved = vec![m];
        for k in ends {
            if let Some(why) = self.fixed(b, TRACK, k) {
                let track = trf(why, &[&cu_label(b, Item::Track(k))]);
                return Err(Stuck { at: from, what: trf("{0} can't move: {1}", &[&label(), &track]) });
            }
            let r = self.mover(b, st, Item::Track(k));
            if let Moved::Run { pts, .. } = &mut st.moved[r] {
                drag(pts, from, to);
            }
            let walls = self.walls(b, st, head, r);
            self.reroute(b, st, r, &walls, head)?;
            moved.push(r);
        }
        Ok(moved)
    }

    /// Leads run `m` round `walls` it comes too close to, the shorter way that keeps clear of
    /// what can't move and of all it gives way to: the route's copper (`head`) and the items moved
    /// before it, far from where it was too (round one, it mustn't run into another).
    fn reroute(&self, b: &Board, st: &mut State, m: usize, walls: &[Cu], head: &[Cu]) -> Result<(), Stuck> {
        let others: Vec<Cu> = head.iter().copied().chain(st.moved[..m].iter().flat_map(|k| k.copper(b))).collect();
        let Moved::Run { pts, proto, tracks, .. } = &mut st.moved[m] else { return Ok(()) };
        let (mine, label) = (Cu::track(proto), cu_label(b, Item::Track(tracks[0])));
        let hulls: Vec<Vec<P>> = walls.iter().filter_map(|w| self.keep(w, &mine).map(|r| hull(w.a, w.b, r as f64 + MARGIN))).collect();
        let before = std::mem::take(pts);
        // Each way round with what stops it, the shorter first: what can't move, or what it gives
        // way to that the way round runs into.
        let blocker = |c: &[Pt]| {
            let new = span(&before, c);
            c[new.start..new.end + 1].windows(2).map(|w| Cu::track(&Track { a: w[0], b: w[1], ..*proto })).find_map(|c| {
                let into = || others.iter().any(|w| self.clash(&c, w));
                self.blocked(b, &c).or_else(|| into().then(|| Stuck { at: c.a, what: trf("{0} has no room to go round", &[&label]) }))
            })
        };
        let length = |c: &[Pt]| c.windows(2).map(|w| sub(w[1], w[0]).0.hypot(sub(w[1], w[0]).1)).sum::<f64>();
        let mut ways: Vec<(Vec<Pt>, Option<Stuck>)> = [true, false]
            .into_iter()
            .filter_map(|ccw| wrap(&before, &hulls, ccw))
            .map(|c| {
                let stuck = blocker(&c);
                (c, stuck)
            })
            .collect();
        ways.sort_by(|x, y| length(&x.0).total_cmp(&length(&y.0)));
        if let Some(i) = ways.iter().position(|w| w.1.is_none()) {
            *pts = ways.swap_remove(i).0;
            return Ok(());
        }
        let at = before[before.len() / 2];
        *pts = before;
        Err(ways.into_iter().find_map(|w| w.1).unwrap_or(Stuck { at, what: trf("{0} has no room to go round", &[&label]) }))
    }

    /// What can't move that copper `s` comes too close to.
    fn blocked(&self, b: &Board, s: &Cu) -> Option<Stuck> {
        let mut stuck = None;
        self.tree.query(s.reach(self.reach()), |id| {
            let (kind, i) = (id & 3, (id >> 2) as usize);
            if stuck.is_none() && self.too_close(b, s, kind, i) && self.fixed(b, kind, i).is_some() {
                stuck = Some(self.stuck(b, s, kind, i));
            }
        });
        stuck
    }

    /// Whether copper `s` comes too close to the board's track, via, pad or edge piece `i`
    /// (`kind`), as DRC measures: by its clearance, the edge clearance, the hole spacing.
    fn too_close(&self, b: &Board, s: &Cu, kind: u32, i: usize) -> bool {
        let (shape, near) = (s.shape(), |d: Nm, need: Nm| d < need - SLACK);
        match kind {
            TRACK => match b.tracks[i].mid {
                Some(m) => {
                    let t = &b.tracks[i];
                    arc_pts(t.a, m, t.b).windows(2).any(|w| self.clash(s, &Cu::track(&Track { a: w[0], b: w[1], mid: None, ..*t })))
                }
                None => self.clash(s, &Cu::track(&b.tracks[i])),
            },
            VIA => self.clash(s, &Cu::via(&b.vias[i])),
            PAD => {
                let (p, r, apart) = (&b.pads[i], &self.rules, apart(s.net, b.pads[i].net));
                let to = |o: &RBox| if p.custom.is_empty() { RBox::pad(p).dist(o) } else { o.dist_poly(&p.copper()) };
                // Its copper keeps the clearance from other nets' copper on its layers, the hole
                // clearance from their holes; its hole the hole clearance from their copper, the
                // hole spacing from holes.
                let hole = (s.hole > 0).then(|| RBox::disc(s.a, s.hole));
                let copper = apart
                    && s.layers & p.layers != 0
                    && (near(to(&shape), self.gap(s.net, p.net)) || hole.is_some_and(|h| near(to(&h), r.hole_clearance)));
                copper
                    || p.drill.is_some_and(|d| {
                        let drill = hole_box(p.at, d, p.angle);
                        apart && near(drill.dist(&shape), r.hole_clearance) || hole.is_some_and(|h| near(drill.dist(&h), r.hole_to_hole))
                    })
            }
            _ => near(self.edges[i].dist(&shape), self.rules.edge_clearance),
        }
    }

    /// Whether copper `a` and `c` come closer than they keep (`keep`).
    fn clash(&self, a: &Cu, c: &Cu) -> bool {
        self.keep(a, c).is_some_and(|keep| RBox::segment(a.a, a.b, 0).dist(&RBox::segment(c.a, c.b, 0)) < keep - SLACK)
    }

    /// How far apart the centre lines of copper `w` and `c` keep, if anything keeps them: their
    /// copper the clearance, of nets apart on a layer of both; a hole the hole clearance from the
    /// other's copper of another net, on every layer; two holes the hole spacing.
    fn keep(&self, w: &Cu, c: &Cu) -> Option<Nm> {
        let (r, apart) = (&self.rules, apart(w.net, c.net));
        let copper = (apart && w.layers & c.layers != 0).then(|| w.r + self.gap(w.net, c.net) + c.r);
        let hole = |h: &Cu, o: &Cu| (apart && h.hole > 0).then(|| h.hole + r.hole_clearance + o.r);
        let holes = (w.hole > 0 && c.hole > 0).then(|| w.hole + r.hole_to_hole + c.hole);
        copper.max(hole(w, c)).max(hole(c, w)).max(holes)
    }

    /// The gap copper of nets `a` and `c` keeps: the larger of their classes' clearances.
    fn gap(&self, a: NetId, c: NetId) -> Nm {
        let keeps = |n: NetId| self.clearances.get(n as usize).copied().unwrap_or(self.rules.clearance);
        keeps(a).max(keeps(c))
    }

    /// How far around copper the rules reach.
    fn reach(&self) -> Nm {
        let (r, widest) = (&self.rules, self.clearances.iter().copied().max().unwrap_or(0));
        r.clearance.max(widest).max(r.edge_clearance).max(r.hole_clearance).max(r.hole_to_hole)
    }

    /// What can't move of the board's track, via, pad or edge piece `i` (`kind`) that copper `s`
    /// meets, and why.
    fn stuck(&self, b: &Board, s: &Cu, kind: u32, i: usize) -> Stuck {
        let why = self.fixed(b, kind, i).unwrap_or_default();
        let (at, what) = match kind {
            TRACK => (b.tracks[i].a, trf(why, &[&cu_label(b, Item::Track(i))])),
            VIA => (b.vias[i].at, trf(why, &[&cu_label(b, Item::Via(i))])),
            PAD => (b.pads[i].at, trf(why, &[&cu_label(b, Item::Pad(i))])),
            _ => (s.a, tr("The board edge is in the way").into()),
        };
        Stuck { at, what }
    }

    /// Why the board's track, via, pad or edge piece `i` (`kind`) can't move, `{0}` for the item
    /// (`trf` fills it); `None` when it can.
    fn fixed(&self, b: &Board, kind: u32, i: usize) -> Option<&'static str> {
        let (net, locked) = match kind {
            TRACK => (b.tracks[i].net, b.tracks[i].locked),
            VIA => (b.vias[i].net, b.vias[i].locked),
            _ => return Some(t!("{0} is in the way")),
        };
        Some(match () {
            _ if locked => t!("{0} is locked"),
            _ if kind == TRACK && b.tracks[i].mid.is_some() => t!("{0} is an arc"),
            _ if net != 0 && net == self.net => t!("{0} is the routed net's"),
            _ if self.joined(b, kind, i) => t!("{0} would leave its net's copper it touches"),
            _ => return None,
        })
    }

    /// Whether track or via `i` (`kind`) touches copper of its net other than where tracks end on
    /// it as a run does (a via crossed, a pad under a via): moved, it would leave it.
    fn joined(&self, b: &Board, kind: u32, i: usize) -> bool {
        let me = if kind == TRACK { Cu::track(&b.tracks[i]) } else { Cu::via(&b.vias[i]) };
        let (ends, shape, mut joined) = ([me.a, me.b], me.shape(), false);
        let ends_on = |p: &dyn Fn(Pt) -> bool| ends.iter().any(|&e| p(e));
        self.tree.query(me.reach(0), |id| {
            let (k, j) = (id & 3, (id >> 2) as usize);
            joined |= (k, j) != (kind, i)
                && match k {
                    TRACK => {
                        let t = &b.tracks[j];
                        let own = t.net == me.net && me.layers & 1 << t.layer != 0;
                        own && !ends_on(&|e| e == t.a || e == t.b) && Cu::track(t).shape().dist(&shape) == 0
                    }
                    VIA => {
                        let v = &b.vias[j];
                        let own = v.net == me.net && me.layers & v.layers != 0;
                        own && (kind == VIA || !ends.contains(&v.at)) && Cu::via(v).shape().dist(&shape) == 0
                    }
                    PAD => {
                        let p = &b.pads[j];
                        let own = p.net == me.net && me.layers & p.layers != 0 && (kind == VIA || !ends_on(&|e| p.holds(e, 0)));
                        own && if p.custom.is_empty() { RBox::pad(p).dist(&shape) == 0 } else { shape.dist_poly(&p.copper()) == 0 }
                    }
                    _ => false,
                };
        });
        joined
    }
}

impl Shoved {
    /// What makes the shove on `b`, with `tracks` and `via` added after the board's last: each run's
    /// tracks now in place of its first, its others taken away; each via where it goes.
    pub(crate) fn patch(&self, b: &Board, tracks: Vec<Track>, via: Option<Via>) -> Patch {
        let mut t: Vec<(usize, Vec<Track>)> = vec![];
        for (old, pts) in &self.runs {
            let first = old.iter().copied().min().expect("a run holds a track");
            t.push((first, pts.windows(2).map(|w| Track { a: w[0], b: w[1], ..b.tracks[first] }).collect()));
            t.extend(old.iter().filter(|&&i| i != first).map(|&i| (i, vec![])));
        }
        t.sort_by_key(|e| e.0);
        t.push((b.tracks.len(), tracks));
        let mut v: Vec<(usize, Via)> = self.vias.iter().map(|&(i, at)| (i, Via { at, ..b.vias[i] })).collect();
        v.sort_by_key(|e| e.0);
        v.extend(via.map(|via| (b.vias.len(), via)));
        Patch::replace(t, v)
    }
}

/// Whether copper of nets `a` and `c` keeps the clearance from each other (no net is apart from all).
fn apart(a: NetId, c: NetId) -> bool {
    a == 0 || a != c
}

/// Of run `pts`, once `was`: the segments that moved, between those it keeps from either end (the
/// same, or cut shorter along the old one).
fn span(was: &[Pt], pts: &[Pt]) -> Range<usize> {
    // Segments of `a` kept from its start, once `b`.
    let kept = |a: &[Pt], b: &[Pt]| {
        let (n, mut k) = (a.len().min(b.len()), 0);
        while k + 1 < n && a[k] == b[k] && a[k + 1] == b[k + 1] {
            k += 1;
        }
        k + usize::from(k + 1 < n && a[k] == b[k] && on(b[k], b[k + 1], a[k + 1]))
    };
    let rev = |v: &[Pt]| v.iter().rev().copied().collect::<Vec<_>>();
    let (n, lo) = (pts.len() - 1, kept(pts, was).min(pts.len() - 1));
    lo..(n - kept(&rev(pts), &rev(was)).min(n)).max(lo)
}

/// Whether `p` lies on the segment from `a` to `b`, exactly.
fn on(a: Pt, b: Pt, p: Pt) -> bool {
    let ((ux, uy), (vx, vy)) = ((b.x as i64 - a.x as i64, b.y as i64 - a.y as i64), (p.x as i64 - a.x as i64, p.y as i64 - a.y as i64));
    ux * vy == uy * vx && (0..=ux * ux + uy * uy).contains(&(ux * vx + uy * vy))
}

/// A pad's hole.
fn hole_box(at: Pt, (w, h): (Nm, Nm), deg: f64) -> RBox {
    RBox::boxed(at, (w, h), deg, w.min(h) / 2)
}

/// Run `pts` with its end at `from` moved to `to`: its last segment there keeps its line, joined
/// to the new end at 45° where it has room, else goes straight to it.
fn drag(pts: &mut Vec<Pt>, from: Pt, to: Pt) {
    if pts[0] != from {
        pts.reverse();
        drag(pts, from, to);
        return pts.reverse();
    }
    let (d, (dx, dy)) = (sub(pts[1], from), ((to.x - from.x) as f64, (to.y - from.y) as f64));
    let len = d.0.hypot(d.1);
    let u = (d.0 / len, d.1 / len);
    let (along, side) = (dx * u.0 + dy * u.1, dy * u.0 - dx * u.1);
    let k = along + side.abs();
    pts[0] = to;
    if side.abs() >= 1.0 && k < len - 1.0 {
        pts.insert(1, Pt { x: (from.x as f64 + u.0 * k).round() as Nm, y: (from.y as f64 + u.1 * k).round() as Nm });
    }
}

/// A point in nm, unrounded.
type P = (f64, f64);

fn sub(a: Pt, b: Pt) -> P {
    (a.x as f64 - b.x as f64, a.y as f64 - b.y as f64)
}

/// The room a segment from `a` to `b` keeps around it, `r` from its centre line: the convex
/// hull of an octagon about each end, its sides on whole nanometres (counter-clockwise).
fn hull(a: Pt, b: Pt, r: f64) -> Vec<P> {
    let (r, k) = (r.ceil(), (r * (std::f64::consts::SQRT_2 - 1.0)).ceil());
    let octagon = [(r, k), (k, r), (-k, r), (-r, k), (-r, -k), (-k, -r), (k, -r), (r, -k)];
    let mut pts: Vec<P> = [a, b].iter().flat_map(|p| octagon.map(|(x, y)| (p.x as f64 + x, p.y as f64 + y))).collect();
    pts.sort_by(|p, q| p.partial_cmp(q).expect("whole numbers"));
    pts.dedup();
    let turn = |o: P, a: P, b: P| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut h: Vec<P> = vec![];
    for half in [pts.clone(), pts.into_iter().rev().collect()] {
        let start = h.len();
        for p in half {
            while h.len() >= start + 2 && turn(h[h.len() - 2], h[h.len() - 1], p) <= 0.0 {
                h.pop();
            }
            h.push(p);
        }
        h.pop();
    }
    h
}

/// How far `p` lies inside convex hull `h` (negative outside), and the side nearest it.
fn inside(h: &[P], p: P) -> (f64, usize) {
    (0..h.len()).map(|i| (dist(h[i], h[(i + 1) % h.len()], p), i)).fold((f64::INFINITY, 0), |m, d| if d.0 < m.0 { d } else { m })
}

fn depth(h: &[P], p: P) -> f64 {
    inside(h, p).0
}

/// How far `p` lies left of the line from `a` to `b` (inside a counter-clockwise hull side).
fn dist(a: P, b: P, p: P) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    (dx * (p.1 - a.1) - dy * (p.0 - a.0)) / dx.hypot(dy)
}

/// The point of hull `h`'s sides nearest `p` inside it.
fn out_of(h: &[P], p: P) -> P {
    let (d, i) = inside(h, p);
    let (a, b) = (h[i], h[(i + 1) % h.len()]);
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    // Inward is left of the side: (-dy, dx).
    (p.0 + (b.1 - a.1) / len * d, p.1 - (b.0 - a.0) / len * d)
}

/// Where the segment from `a` to `b` runs inside hull `h` (deeper than rounding): from `t0` to
/// `t1` along it, in through side `enter` and out through side `leave` (`None` for an end inside).
fn clip(a: P, b: P, h: &[P]) -> Option<(f64, Option<usize>, f64, Option<usize>)> {
    // Off the hull's box, quickly.
    let fold = |f: fn(f64, f64) -> f64, k: fn(&P) -> f64| h.iter().map(k).fold(k(&h[0]), f);
    let ((x0, x1), (y0, y1)) = ((fold(f64::min, |p| p.0), fold(f64::max, |p| p.0)), (fold(f64::min, |p| p.1), fold(f64::max, |p| p.1)));
    if a.0.max(b.0) < x0 || a.0.min(b.0) > x1 || a.1.max(b.1) < y0 || a.1.min(b.1) > y1 {
        return None;
    }
    let (mut t0, mut t1, mut enter, mut leave) = (0.0, 1.0, None, None);
    for i in 0..h.len() {
        let (da, db) = (dist(h[i], h[(i + 1) % h.len()], a), dist(h[i], h[(i + 1) % h.len()], b));
        if da <= 0.0 && db <= 0.0 {
            return None;
        }
        let t = da / (da - db);
        if da < 0.0 && t > t0 {
            (t0, enter) = (t, Some(i));
        } else if db < 0.0 && t < t1 {
            (t1, leave) = (t, Some(i));
        }
    }
    let mid = lerp(a, b, (t0 + t1) / 2.0);
    (t0 < t1 && depth(h, mid) > DEPTH).then_some((t0, enter, t1, leave))
}

fn lerp(a: P, b: P, t: f64) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Run `pts` led round each of `hulls` it enters, counter-clockwise round them or clockwise:
/// `None` when one holds an end of it, or it doesn't settle.
fn wrap(pts: &[Pt], hulls: &[Vec<P>], ccw: bool) -> Option<Vec<Pt>> {
    let mut line: Vec<P> = pts.iter().map(|p| (p.x as f64, p.y as f64)).collect();
    for _ in 0..4 * hulls.len() + 4 {
        let Some(h) = hulls.iter().find(|h| line.windows(2).any(|w| clip(w[0], w[1], h).is_some())) else {
            return Some(tidy(&line));
        };
        line = walk(&line, h, ccw)?;
    }
    None
}

/// Line `line` led round hull `h` from where it first goes in to where it last comes out, along
/// the hull's sides, with 45° bends where it leaves its course and comes back to it.
fn walk(line: &[P], h: &[P], ccw: bool) -> Option<Vec<P>> {
    let n = h.len();
    let cuts: Vec<_> = (0..line.len() - 1).filter_map(|k| clip(line[k], line[k + 1], h).map(|c| (k, c))).collect();
    let (&(k0, (t0, enter, ..)), &(k1, (.., t1, leave))) = (cuts.first()?, cuts.last()?);
    // An end of the line within rounding of a side goes round from there; deeper, it can't.
    let on = |p: P| Some(inside(h, p)).filter(|d| d.0 <= DEPTH).map(|d| d.1);
    let (e0, e1) = (enter.or_else(|| on(line[k0]))?, leave.or_else(|| on(line[k1 + 1]))?);
    let (from, to) = (lerp(line[k0], line[k0 + 1], t0), lerp(line[k1], line[k1 + 1], t1));
    // Out on the side it came in through: ahead of where it came in, or behind.
    let ahead = {
        let (a, b) = (h[e0], h[(e0 + 1) % n]);
        (to.0 - from.0) * (b.0 - a.0) + (to.1 - from.1) * (b.1 - a.1) > 0.0
    };
    let corners: Vec<P> = if ccw {
        let m = match (e1 + n - e0) % n {
            0 if ahead => 0,
            0 => n,
            m => m,
        };
        (1..=m).map(|j| h[(e0 + j) % n]).collect()
    } else {
        let m = match (e0 + n - e1) % n {
            0 if !ahead => 0,
            0 => n,
            m => m,
        };
        (0..m).map(|j| h[(e0 + n - j) % n]).collect()
    };
    let mut out = line[..=k0].to_vec();
    out.push(from);
    let (at, n) = (out.len() - 1, out.len() + corners.len() + line.len() - k1);
    out.extend(corners);
    out.push(to);
    out.extend(&line[k1 + 1..]);
    // Where it leaves its course and where it comes back, at 45° where it can (the way back found
    // on the line reversed).
    let back: Vec<P> = out.iter().rev().copied().collect();
    let (leave, rejoin) = (ramp(&out, at, n - 1 - (line.len() - k1 - 1)), ramp(&back, line.len() - k1 - 1, n - 1 - at));
    let rejoin = rejoin.map(|(j, q, r)| (n - 1 - j, n - q, r)).filter(|&(j, ..)| leave.is_none_or(|(i, ..)| i <= j));
    let tail = match rejoin {
        Some((j, q, r)) => [&out[..=j], &[r], &out[q..]].concat(),
        None => out,
    };
    Some(match leave {
        Some((j, q, r)) => [&tail[..q], &[r], &tail[j..]].concat(),
        None => tail,
    })
}

/// Where `line`, turning at `i` onto a hull's sides (up to `end`) by more than 45° from its way
/// in, can leave its course at 45° instead: the first side on within 45° of the way in (from
/// `line[j]`), carried back to meet the segment ending at `line[q]` at `r`.
fn ramp(line: &[P], i: usize, end: usize) -> Option<(usize, usize, P)> {
    let way = |a: P, b: P| (b.0 - a.0, b.1 - a.1);
    let within = |u: P, v: P| u.0 * v.0 + u.1 * v.1 >= 0.7 * u.0.hypot(u.1) * v.0.hypot(v.1);
    let into = way(*line.get(i.checked_sub(1)?)?, line[i]);
    if i + 1 >= line.len() || within(into, way(line[i], line[i + 1])) {
        return None;
    }
    let j = (i + 1..end.min(line.len() - 1)).find(|&j| within(into, way(line[j], line[j + 1])))?;
    let (o, d) = (line[j], way(line[j], line[j + 1]));
    for q in (1..=i).rev() {
        let (a, b) = (line[q - 1], line[q]);
        let e = way(a, b);
        let den = d.0 * e.1 - d.1 * e.0;
        if den.abs() < 1e-9 {
            continue;
        }
        // o + s·d = a + t·e: where the side's line crosses this segment, behind `o`.
        let (s, t) = (((a.0 - o.0) * e.1 - (a.1 - o.1) * e.0) / den, ((a.0 - o.0) * d.1 - (a.1 - o.1) * d.0) / den);
        if (0.0..=1.0).contains(&t) && s <= 0.0 {
            return within(e, d).then(|| (j, q, lerp(a, b, t)));
        }
    }
    None
}

/// `line` on whole nanometres, without repeated points or corners that go straight on or back.
fn tidy(line: &[P]) -> Vec<Pt> {
    let mut out: Vec<Pt> = vec![];
    for p in line.iter().map(|p| Pt { x: p.0.round() as Nm, y: p.1.round() as Nm }) {
        if out.last() == Some(&p) {
            continue;
        }
        if let [.., a, b] = out[..] {
            let ((ux, uy), (vx, vy)) =
                ((b.x as i64 - a.x as i64, b.y as i64 - a.y as i64), (p.x as i64 - b.x as i64, p.y as i64 - b.y as i64));
            if ux as i128 * vy as i128 == uy as i128 * vx as i128 {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drc;

    /// A board from (-60, 0) to (90, 50) mm, nets A to D (1 to 4) and `items`; net A's pad at
    /// (15, 4) starts the routes.
    fn board(items: &str) -> Board {
        let src = format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (net 0 "") (net 1 "A") (net 2 "B") (net 3 "C") (net 4 "D")
              (gr_rect (start -60 0) (end 90 50) (layer "Edge.Cuts") (width 0.05))
              (footprint "P" (layer "F.Cu") (at 15 4) (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "A"))) {items})"#
        );
        Board::parse(&src).unwrap()
    }

    /// A track of `net` on F.Cu through `pts` (mm).
    fn run(net: u32, pts: &[(f64, f64)]) -> String {
        let seg = |w: &[(f64, f64)]| {
            format!("(segment (start {} {}) (end {} {}) (width 0.25) (layer \"F.Cu\") (net {net}))", w[0].0, w[0].1, w[1].0, w[1].1)
        };
        pts.windows(2).map(seg).collect()
    }

    fn mm(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    /// Net A's route down from its pad to `to`, 0.25 mm wide on F.Cu, and the shove for it.
    fn shove(b: &Board, to: Pt) -> (Vec<Pt>, Result<Shoved, Stuck>) {
        let path = vec![mm(15.0, 4.0), to];
        let s = Shover::new(b, 1).shove(b, &path, 250_000, 0, None);
        (path, s)
    }

    /// `b` with the shove and the route made, the patch that undoes them and the violations of
    /// every track and via.
    fn made(b: &Board, path: &[Pt], s: &Shoved, via: Option<Via>) -> (Board, Patch, Vec<drc::Violation>) {
        let mut done = b.clone();
        let undo = done.add_route(s, crate::route::tracks(path, 250_000, 0, 1), via).unwrap();
        let items: Vec<Item> = (0..done.tracks.len()).map(Item::Track).chain((0..done.vias.len()).map(Item::Via)).collect();
        let violations = drc::check_items(&done, &items);
        (done, undo, violations)
    }

    /// Whether every segment runs at a multiple of 45°, within rounding.
    fn octilinear(pts: &[Pt]) -> bool {
        pts.windows(2).all(|w| {
            let (dx, dy) = ((w[1].x - w[0].x).abs(), (w[1].y - w[0].y).abs());
            dx == 0 || dy == 0 || (dx - dy).abs() <= 1
        })
    }

    #[test]
    fn a_track_in_the_way_moves_aside_by_the_clearance_at_45_degrees() {
        // Net B from (2, 10) to (28, 10); the route stops 0.2 mm above it.
        let b = board(&run(2, &[(2.0, 10.0), (28.0, 10.0)]));
        let (path, s) = shove(&b, mm(15.0, 9.8));
        let s = s.unwrap();
        assert_eq!((s.runs.len(), &s.runs[0].0, s.vias.len()), (1, &vec![0], 0));
        // Its ends stay; the stretch in the way dips to keep 0.2 mm from the route's end (half
        // both widths and 5 nm more from its centre line), joined to the rest at 45°.
        let pts = &s.runs[0].1;
        assert_eq!((pts[0], pts[pts.len() - 1], pts.len()), (mm(2.0, 10.0), mm(28.0, 10.0), 6), "{pts:?}");
        assert_eq!(pts.iter().map(|p| p.y).max(), Some(10_250_005));
        assert!(octilinear(pts), "{pts:?}");
        assert_eq!(made(&b, &path, &s, None).2, vec![]);
        // Of a class that keeps 0.5 mm, it dips 0.3 mm more, and the rules pass by that clearance.
        let mut b = b;
        b.apply_pro(r#"{"net_settings": {"classes": [{"name": "Far", "clearance": 0.5}], "netclass_patterns": [{"netclass": "Far", "pattern": "B"}]}}"#);
        let (path, s) = shove(&b, mm(15.0, 9.8));
        let s = s.unwrap();
        assert_eq!(s.runs[0].1.iter().map(|p| p.y).max(), Some(10_550_005));
        assert_eq!(made(&b, &path, &s, None).2, vec![]);
    }

    #[test]
    fn a_track_crossed_goes_round_the_routes_end() {
        let b = board(&run(2, &[(2.0, 10.0), (28.0, 10.0)]));
        let (path, s) = shove(&b, mm(15.0, 14.0));
        let s = s.unwrap();
        let pts = &s.runs[0].1;
        assert_eq!(pts.iter().map(|p| p.y).max(), Some(14_450_005), "{pts:?}");
        assert!(octilinear(pts), "{pts:?}");
        assert_eq!(made(&b, &path, &s, None).2, vec![]);
    }

    #[test]
    fn what_a_moved_track_comes_too_close_to_moves_in_turn() {
        // Net C runs 0.5 mm below net B (0.25 mm apart), from ends below.
        let c = run(3, &[(5.0, 12.0), (5.0, 10.5), (25.0, 10.5), (25.0, 12.0)]);
        let b = board(&format!("{}{c}", run(2, &[(2.0, 10.0), (28.0, 10.0)])));
        let (path, s) = shove(&b, mm(15.0, 9.8));
        let s = s.unwrap();
        assert_eq!(s.runs.iter().map(|r| r.0.clone()).collect::<Vec<_>>(), [vec![0], vec![1, 2, 3]]);
        assert!(s.runs.iter().all(|r| octilinear(&r.1)));
        let c = &s.runs[1].1;
        assert_eq!((c[0], c[c.len() - 1]), (mm(5.0, 12.0), mm(25.0, 12.0)));
        assert_eq!(made(&b, &path, &s, None).2, vec![]);
    }

    #[test]
    fn a_via_in_the_way_moves_with_its_tracks_end() {
        // Net C's via under the route's end, its track going right then down.
        let via = r#"(via (at 15 10.3) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 3))"#;
        let b = board(&format!("{via}{}", run(3, &[(15.0, 10.3), (20.0, 10.3), (20.0, 14.0)])));
        let (path, s) = shove(&b, mm(15.0, 9.6));
        let s = s.unwrap();
        // Down by what the clearance needs; its track's end along, with a 45° jog back onto its line.
        let at = Pt { x: 15_000_000, y: 9_600_000 + 725_005 };
        assert_eq!(s.vias, [(0, at)]);
        let pts = &s.runs[0].1;
        assert_eq!(&pts[..3], [at, Pt { x: 15_025_005, y: 10_300_000 }, mm(20.0, 10.3)], "{pts:?}");
        assert_eq!(made(&b, &path, &s, None).2, vec![]);
    }

    #[test]
    fn a_route_ending_on_a_new_via_pushes_with_it() {
        // Net B on B.Cu under the via the route drops at its end.
        let b = board(&run(2, &[(2.0, 10.0), (28.0, 10.0)]).replace("F.Cu", "B.Cu"));
        let via = Via { at: mm(15.0, 10.2), size: 800_000, drill: 400_000, layers: b.copper, net: 1, locked: false };
        let path = [mm(15.0, 4.0), via.at];
        let s = Shover::new(&b, 1).shove(&b, &path, 250_000, 0, Some(&via)).unwrap();
        assert_eq!(s.runs.len(), 1);
        assert_eq!(made(&b, &path, &s, Some(via)).2, vec![]);
    }

    #[test]
    fn what_cant_move_stops_it() {
        let b = board(&run(2, &[(2.0, 10.0), (28.0, 10.0)]).replace("(net 2)", "(locked yes) (net 2)"));
        assert_eq!(shove(&b, mm(15.0, 9.8)).1.unwrap_err().what, "Track (B) on F.Cu is locked");
        // A pad of net D just under the track: it has no room to move down, nor up round the route.
        let d = r#"(footprint "D" (layer "F.Cu") (at 15 10.8) (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 4 "D")))"#;
        let b = board(&format!("{}{d}", run(2, &[(2.0, 10.0), (28.0, 10.0)])));
        let e = shove(&b, mm(15.0, 9.8)).1.unwrap_err();
        assert!(e.what.ends_with("pad 1 (D) is in the way"), "{e:?}");
        // Net B's track crosses one of its vias (joined there, not by its ends): moved, it would
        // leave it.
        let via = r#"(via (at 20 10) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 2))"#;
        let b = board(&format!("{}{via}", run(2, &[(2.0, 10.0), (28.0, 10.0)])));
        assert_eq!(shove(&b, mm(15.0, 9.8)).1.unwrap_err().what, "Track (B) on F.Cu would leave its net's copper it touches");
        // Net B along the board's bottom edge, 0.575 mm from it: no room past it.
        let b = board(&run(2, &[(2.0, 49.3), (28.0, 49.3)]));
        assert_eq!(shove(&b, mm(15.0, 49.0)).1.unwrap_err().what, "The board edge is in the way");
    }

    #[test]
    fn a_track_pushed_round_the_route_keeps_clear_of_all_of_it() {
        // Net B under a bent route's end, net C locked just under B: B can't go down, and up
        // round the route's end it would cross the route's first stretch, far from where B was:
        // no room, so the step walks round.
        let c = run(3, &[(2.0, 15.6), (28.0, 15.6)]).replace("(net 3)", "(locked yes) (net 3)");
        let b = board(&format!("{}{c}", run(2, &[(2.0, 15.0), (28.0, 15.0)])));
        let path = [mm(15.0, 4.0), mm(15.0, 10.2), mm(19.6, 14.8)];
        let e = Shover::new(&b, 1).shove(&b, &path, 250_000, 0, None).unwrap_err();
        assert_eq!(e.what, "Track (C) on F.Cu is locked");
    }

    #[test]
    fn stops_once_too_many_would_move() {
        // Tracks 0.45 mm apart (0.2 mm between them), long enough for the bumps nested in them:
        // each pushes the next. 20 make room, each clear of the others; 66 are too many.
        let y = |i: u32| 10.0 + 0.45 * i as f64;
        let runs = |n: u32| board(&(0..n).map(|i| run(2 + i % 3, &[(-55.0, y(i)), (85.0, y(i))])).collect::<String>());
        let b = runs(20);
        let (path, s) = shove(&b, mm(15.0, 9.8));
        let s = s.unwrap();
        assert_eq!((s.runs.len(), made(&b, &path, &s, None).2), (20, vec![]));
        assert!(s.runs.iter().all(|r| octilinear(&r.1)));
        let e = shove(&runs(66), mm(15.0, 9.8)).1.unwrap_err();
        assert_eq!(e.what, "more than 64 tracks and vias would move");
    }

    #[test]
    fn undone_the_board_is_as_before() {
        // Net C's via (its track going right, then up) pushed down into net B's track.
        let via = r#"(via (at 15 10.3) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 3))"#;
        let c = run(3, &[(15.0, 10.3), (20.0, 10.3), (20.0, 8.0)]);
        let b = board(&format!("{}{via}{c}", run(2, &[(2.0, 11.0), (28.0, 11.0)])));
        let (path, s) = shove(&b, mm(15.0, 9.6));
        let s = s.unwrap();
        let (mut done, undo, violations) = made(&b, &path, &s, None);
        assert_eq!((violations, s.runs.len(), s.vias.len()), (vec![], 2, 1));
        assert_ne!(done.tracks, b.tracks);
        done.apply(undo);
        assert_eq!((done.tracks, done.vias), (b.tracks, b.vias));
    }
}
