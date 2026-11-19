//! Undo and redo. An edit is kept as the items it replaced, so undoing it costs what it
//! touched, not a copy of the board (kria-k26 has 259k shapes to copy for one part move).

use crate::board::{Board, Footprint, NetId, Pad, Shape, Text, Track, Via, Zone, ZoneFill};
use crate::drc::Item;
use std::collections::BTreeSet;
use std::ops::Range;

/// Items to put in one list of a board: `items` replace the `len` from `at` on (all of them
/// when `len` runs past the end). Of a splice undoing a change, `was` is where `at` was before it.
struct Splice<T> {
    at: usize,
    len: usize,
    items: Vec<T>,
    was: usize,
}

impl<T> Splice<T> {
    /// Puts `items` in place of the `len` from `at` on, in a list as it is now.
    fn new(at: usize, len: usize, items: Vec<T>) -> Self {
        Splice { at, len, items, was: at }
    }

    /// Of the splice that undoes a change: where the items the change put in are, in a list of `n`.
    fn put(&self, n: usize) -> Range<usize> {
        self.at..self.at.saturating_add(self.len).min(n)
    }
}

impl<T: Clone> Splice<T> {
    /// Takes away the items from `at` on.
    fn tail(at: usize) -> Self {
        Splice::new(at, usize::MAX, vec![])
    }

    /// Takes away the items in `range`.
    fn cut(range: Range<usize>) -> Self {
        Splice::new(range.start, range.len(), vec![])
    }

    /// Puts back the items now in `range`.
    fn copy(list: &[T], range: Range<usize>) -> Self {
        Splice::new(range.start, range.len(), list[range].to_vec())
    }

    /// Puts back the whole list as it is now, however many items it holds by then.
    fn all(list: &[T]) -> Self {
        Self::rest(list, 0)
    }

    /// Puts back the items from `at` on as they are now, however many the list holds by then.
    fn rest(list: &[T], at: usize) -> Self {
        Splice::new(at, usize::MAX, list[at..].to_vec())
    }

    /// Puts the items in and returns the splice that puts back what they replaced.
    fn apply(self, list: &mut Vec<T>) -> Self {
        let (end, len) = (self.at.saturating_add(self.len).min(list.len()), self.items.len());
        Splice::new(self.at, len, list.splice(self.at..end, self.items).collect())
    }
}

/// How many items the change a splice undoes put in, less those it took away.
fn grown<T>(s: &Splice<T>) -> isize {
    s.len as isize - s.items.len() as isize
}

/// Whether any of the splices puts in more or fewer items than it takes away.
fn resized<T>(splices: &[Splice<T>]) -> bool {
    splices.iter().any(|s| s.len != s.items.len())
}

/// Makes the splices' changes to `list` (in order and apart, each where the list has its items
/// now) and returns the splices that undo them.
fn splice<T: Clone>(list: &mut Vec<T>, splices: Vec<Splice<T>>) -> Vec<Splice<T>> {
    // One alone, or each putting in as many as it takes away, changes the list where it is; else
    // the list is built again once (taking many items away one by one moves the rest each time).
    if splices.len() < 2 || !resized(&splices) {
        return splices.into_iter().map(|s| s.apply(list)).collect();
    }
    let (mut old, mut done, mut back) = (std::mem::take(list).into_iter(), 0, Vec::with_capacity(splices.len()));
    for s in splices {
        list.extend(old.by_ref().take(s.at.saturating_sub(done)));
        let gone: Vec<T> = old.by_ref().take(s.len).collect();
        done = s.at.max(done) + gone.len();
        back.push(Splice { at: list.len(), len: s.items.len(), items: gone, was: s.at });
        list.extend(s.items);
    }
    list.extend(old);
    back
}

/// Of the splices that undo a change to a list: where item `i` of the list before the change is
/// after it, `None` when the change took it away or put another in its place.
fn moved<T>(undo: &[Splice<T>], i: usize) -> Option<usize> {
    // The last splice from where `i` was on or before it: past its end, `i` is as far from it.
    let Some(s) = undo.partition_point(|s| s.was <= i).checked_sub(1).map(|k| &undo[k]) else { return Some(i) };
    let end = s.was + s.items.len();
    (i >= end).then(|| (i - end).saturating_add(s.at.saturating_add(s.len)))
}

/// Of the splices that undo a change to a list: where the place before item `i` of the list
/// before the change is after it, from before to after what the change put in right there (or
/// in place of what it took away around it): an item there follows it all, a part without
/// items keeps to its own side of it.
fn span<T>(undo: &[Splice<T>], i: usize) -> (usize, usize) {
    let k = undo.partition_point(|s| s.was < i);
    let a = match k.checked_sub(1).map(|k| &undo[k]) {
        None => i,
        Some(s) if i < s.was + s.items.len() => return (s.at, s.at + s.len),
        Some(s) => s.at + s.len + i - s.was - s.items.len(),
    };
    (a, a + undo[k..].iter().take_while(|s| s.was == i && s.items.is_empty()).map(|s| s.len).sum::<usize>())
}

/// Of the splices that undo a change to `list`: the items they put back and those the change
/// put in, but those it left as they were: one to one where it kept their number, else past the
/// ends they share.
fn changed<'a, T: PartialEq>(splices: &'a [Splice<T>], list: &'a [T]) -> Vec<&'a T> {
    let mut out = vec![];
    for s in splices {
        let (was, now) = (&s.items[..], &list[s.put(list.len())]);
        if was.len() == now.len() {
            out.extend(was.iter().zip(now).filter(|(a, b)| a != b).flat_map(|(a, b)| [a, b]));
        } else {
            let head = was.iter().zip(now).take_while(|(a, b)| a == b).count();
            let tail = was[head..].iter().rev().zip(now[head..].iter().rev()).take_while(|(a, b)| a == b).count();
            out.extend(was[head..was.len() - tail].iter().chain(&now[head..now.len() - tail]));
        }
    }
    out
}

/// The splices `later`, undoing a change to a list, and `earlier`, undoing the change before it
/// to other items of it, as one list of splices made at once: `earlier`'s move by what `later`'s
/// before them grew.
fn after<T>(later: Vec<Splice<T>>, earlier: Vec<Splice<T>>) -> Vec<Splice<T>> {
    let (mut out, mut later, mut grew) = (Vec::with_capacity(later.len() + earlier.len()), later.into_iter().peekable(), 0);
    for mut e in earlier {
        while let Some(s) = later.next_if(|s| s.was + s.items.len() <= e.at) {
            grew += grown(&s);
            out.push(s);
        }
        debug_assert!(later.peek().is_none_or(|s| e.at.saturating_add(e.len) <= s.was), "changes to the same items");
        e.at = e.at.wrapping_add_signed(grew);
        out.push(e);
    }
    out.extend(later);
    out
}

/// A part, track, via or zone, or one of the board's own texts or drawings (`Board::own`): what
/// a click selects and `Patch::delete` takes away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pick {
    Footprint(usize),
    Track(usize),
    Via(usize),
    Zone(usize),
    Text(usize),
    Shape(usize),
}

/// What undoes an edit: the items it replaced, list by list. A list's splices lie in order and
/// apart, each where the list has its items before any is made: they are made at once. A zone
/// whose fills it puts in or takes away goes with them, its `fills` as many.
#[derive(Default)]
pub struct Patch {
    nets: Vec<Splice<String>>,
    tracks: Vec<Splice<Track>>,
    vias: Vec<Splice<Via>>,
    footprints: Vec<Splice<Footprint>>,
    pads: Vec<Splice<Pad>>,
    shapes: Vec<Splice<Shape>>,
    texts: Vec<Splice<Text>>,
    zones: Vec<Splice<Zone>>,
    fills: Vec<Splice<ZoneFill>>,
}

impl Patch {
    /// Puts footprint `i` back as it is now, with its pads, graphics and texts.
    pub fn footprint(b: &Board, i: usize) -> Self {
        let f = &b.footprints[i];
        Patch {
            footprints: vec![Splice::copy(&b.footprints, i..i + 1)],
            pads: vec![Splice::copy(&b.pads, f.pads.clone())],
            shapes: vec![Splice::copy(&b.shapes, f.shapes.clone())],
            texts: vec![Splice::copy(&b.texts, f.texts.clone())],
            ..Patch::default()
        }
    }

    /// Puts the picked part (with its pads, graphics and texts), track, via or zone (with its
    /// fills) back as it is now.
    pub fn item(b: &Board, pick: Pick) -> Self {
        match pick {
            Pick::Footprint(i) => Patch::footprint(b, i),
            Pick::Track(i) => Patch { tracks: vec![Splice::copy(&b.tracks, i..i + 1)], ..Patch::default() },
            Pick::Via(i) => Patch { vias: vec![Splice::copy(&b.vias, i..i + 1)], ..Patch::default() },
            Pick::Zone(i) => Patch {
                zones: vec![Splice::copy(&b.zones, i..i + 1)],
                fills: vec![Splice::copy(&b.fills, b.zones[i].fills.clone())],
                ..Patch::default()
            },
            Pick::Text(i) => Patch { texts: vec![Splice::copy(&b.texts, i..i + 1)], ..Patch::default() },
            Pick::Shape(i) => Patch { shapes: vec![Splice::copy(&b.shapes, i..i + 1)], ..Patch::default() },
        }
    }

    /// Puts each `(i, tracks)`'s tracks in place of track `i` (none takes it away) and each
    /// `(i, via)` in place of via `i`, once applied; `i` the list's length adds them after its
    /// last. Each list in order.
    pub fn replace(tracks: Vec<(usize, Vec<Track>)>, vias: Vec<(usize, Via)>) -> Self {
        Patch {
            tracks: tracks.into_iter().map(|(at, items)| Splice::new(at, 1, items)).collect(),
            vias: vias.into_iter().map(|(at, via)| Splice::new(at, 1, vec![via])).collect(),
            ..Patch::default()
        }
    }

    /// Puts back every net, part (its value and fields), pad and text, and the graphics from
    /// `shapes` on, as they are now: what undoes `Board::update` (the parts it swaps own graphics
    /// from there; those it adds, after).
    pub fn update(b: &Board, shapes: usize) -> Self {
        Patch {
            nets: vec![Splice::all(&b.nets)],
            pads: vec![Splice::all(&b.pads)],
            footprints: vec![Splice::all(&b.footprints)],
            shapes: vec![Splice::rest(&b.shapes, shapes)],
            texts: vec![Splice::all(&b.texts)],
            ..Patch::default()
        }
    }

    /// Puts part `f` with `pads`, `shapes` and `texts` (its ranges where they will be) in place of
    /// part `i` and its own, once applied.
    pub(crate) fn part(b: &Board, i: usize, f: Footprint, pads: Vec<Pad>, shapes: Vec<Shape>, texts: Vec<Text>) -> Self {
        let old = &b.footprints[i];
        Patch {
            footprints: vec![Splice::new(i, 1, vec![f])],
            pads: vec![Splice::new(old.pads.start, old.pads.len(), pads)],
            shapes: vec![Splice::new(old.shapes.start, old.shapes.len(), shapes)],
            texts: vec![Splice::new(old.texts.start, old.texts.len(), texts)],
            ..Patch::default()
        }
    }

    /// Puts `texts` in place of part `i`'s and `f` (its texts where they will be) in place of the
    /// part, once applied; its pads and graphics stay (each part put in has a splice of each).
    pub(crate) fn texts(b: &Board, i: usize, f: Footprint, texts: Vec<Text>) -> Self {
        let old = &b.footprints[i];
        Patch {
            footprints: vec![Splice::new(i, 1, vec![f])],
            pads: vec![Splice::new(old.pads.start, 0, vec![])],
            shapes: vec![Splice::new(old.shapes.start, 0, vec![])],
            texts: vec![Splice::new(old.texts.start, old.texts.len(), texts)],
            ..Patch::default()
        }
    }

    /// Puts back every zone's fill as it is now.
    pub fn fills(b: &Board) -> Self {
        Patch { zones: vec![Splice::all(&b.zones)], fills: vec![Splice::all(&b.fills)], ..Patch::default() }
    }

    /// Puts `z` in place of zone `i`, and `fills` (its range in `z` as long) in place of the
    /// board's fills in `range`, its own, once applied.
    pub(crate) fn zone(i: usize, z: Zone, range: Range<usize>, fills: Vec<ZoneFill>) -> Self {
        Patch { zones: vec![Splice::new(i, 1, vec![z])], fills: vec![Splice::new(range.start, range.len(), fills)], ..Patch::default() }
    }

    /// Takes away the parts (with their pads, graphics and texts), tracks, vias and zones (with
    /// their fills) added from now on.
    pub fn added(b: &Board) -> Self {
        Patch {
            nets: vec![],
            tracks: vec![Splice::tail(b.tracks.len())],
            vias: vec![Splice::tail(b.vias.len())],
            footprints: vec![Splice::tail(b.footprints.len())],
            pads: vec![Splice::tail(b.pads.len())],
            shapes: vec![Splice::tail(b.shapes.len())],
            texts: vec![Splice::tail(b.texts.len())],
            zones: vec![Splice::tail(b.zones.len())],
            fills: vec![Splice::tail(b.fills.len())],
        }
    }

    /// Puts `items` in place of the board's graphics in `range`, once applied.
    pub(crate) fn shapes(range: Range<usize>, items: Vec<Shape>) -> Self {
        Patch { shapes: vec![Splice::new(range.start, range.len(), items)], ..Patch::default() }
    }

    /// Of the patch that undoes a change to `b`: whether the change drew, moved or took away a
    /// line on Edge.Cuts, the board's or a part's (a connector's notch): the edge every item keeps clear of.
    pub(crate) fn edge(&self, b: &Board) -> bool {
        let Some(edge) = b.layers.iter().position(|l| l == "Edge.Cuts") else { return false };
        self.shapes.iter().any(|s| s.items.iter().chain(&b.shapes[s.put(b.shapes.len())]).any(|x| x.layer as usize == edge))
    }

    /// Takes away the picked part (with its pads, graphics and texts), track, via or zone (with
    /// its fills), once applied; several, the patches of each collected into one.
    pub fn delete(b: &Board, pick: Pick) -> Self {
        match pick {
            Pick::Footprint(i) => {
                let f = &b.footprints[i];
                Patch {
                    footprints: vec![Splice::cut(i..i + 1)],
                    pads: vec![Splice::cut(f.pads.clone())],
                    shapes: vec![Splice::cut(f.shapes.clone())],
                    texts: vec![Splice::cut(f.texts.clone())],
                    ..Patch::default()
                }
            }
            Pick::Track(i) => Patch { tracks: vec![Splice::cut(i..i + 1)], ..Patch::default() },
            Pick::Via(i) => Patch { vias: vec![Splice::cut(i..i + 1)], ..Patch::default() },
            Pick::Zone(i) => {
                Patch { zones: vec![Splice::cut(i..i + 1)], fills: vec![Splice::cut(b.zones[i].fills.clone())], ..Patch::default() }
            }
            Pick::Text(i) => Patch { texts: vec![Splice::cut(i..i + 1)], ..Patch::default() },
            Pick::Shape(i) => Patch { shapes: vec![Splice::cut(i..i + 1)], ..Patch::default() },
        }
    }

    /// Of the patch that undoes a change: where a track, via or pad before the change is after
    /// it, `None` when the change took it away or put another in its place.
    pub(crate) fn moved(&self, item: Item) -> Option<Item> {
        match item {
            Item::Track(i) => moved(&self.tracks, i).map(Item::Track),
            Item::Via(i) => moved(&self.vias, i).map(Item::Via),
            Item::Pad(i) => moved(&self.pads, i).map(Item::Pad),
        }
    }

    /// Of the patch that undoes a change to `b`: the tracks, vias and pads the change put in or
    /// changed, in order.
    pub(crate) fn items(&self, b: &Board) -> Vec<Item> {
        let tracks = self.tracks.iter().flat_map(|s| s.put(b.tracks.len()).map(Item::Track));
        let vias = self.vias.iter().flat_map(|s| s.put(b.vias.len()).map(Item::Via));
        tracks.chain(vias).chain(self.pads.iter().flat_map(|s| s.put(b.pads.len()).map(Item::Pad))).collect()
    }

    /// Of the patch that undoes a change to `b`: the nets of the tracks, vias, pads and zone
    /// fills the change put in, changed or took away; `None` when it changed the nets.
    pub(crate) fn nets(&self, b: &Board) -> Option<BTreeSet<NetId>> {
        if !self.nets.is_empty() {
            return None;
        }
        let now = self.items(b).into_iter().map(|i| match i {
            Item::Track(k) => b.tracks[k].net,
            Item::Via(k) => b.vias[k].net,
            Item::Pad(k) => b.pads[k].net,
        });
        let fills = self.fills.iter().flat_map(|s| b.fills[s.put(b.fills.len())].iter().chain(&s.items).map(|f| f.net));
        let tracks = self.tracks.iter().flat_map(|s| s.items.iter().map(|t| t.net));
        let vias = self.vias.iter().flat_map(|s| s.items.iter().map(|v| v.net));
        let pads = self.pads.iter().flat_map(|s| s.items.iter().map(|p| p.net));
        Some(now.chain(fills).chain(tracks).chain(vias).chain(pads).collect())
    }

    /// Of the patch that undoes a change to `b`: the zone fills the change put in or took away.
    pub(crate) fn filled<'a>(&'a self, b: &'a Board) -> Vec<&'a ZoneFill> {
        self.fills.iter().flat_map(|s| b.fills[s.put(b.fills.len())].iter().chain(&s.items)).collect()
    }

    /// Of the patch that undoes a change to `b`: the tracks, vias, pads, graphics and texts it
    /// put in, changed or took away, as they are and as they were.
    pub(crate) fn changed<'a>(&'a self, b: &'a Board) -> Changed<'a> {
        Changed {
            tracks: changed(&self.tracks, &b.tracks),
            vias: changed(&self.vias, &b.vias),
            pads: changed(&self.pads, &b.pads),
            shapes: changed(&self.shapes, &b.shapes),
            texts: changed(&self.texts, &b.texts),
        }
    }

    /// Of the patch that undoes a change to `b`: the zones the change put in or changed.
    pub(crate) fn zones_put(&self, b: &Board) -> impl Iterator<Item = usize> {
        self.zones.iter().flat_map(|s| s.put(b.zones.len()))
    }

    /// One patch undoing the change this one undoes, then the change before it, which `earlier`
    /// undoes and which changed other items.
    pub(crate) fn then(self, earlier: Patch) -> Patch {
        Patch {
            nets: after(self.nets, earlier.nets),
            tracks: after(self.tracks, earlier.tracks),
            vias: after(self.vias, earlier.vias),
            footprints: after(self.footprints, earlier.footprints),
            pads: after(self.pads, earlier.pads),
            shapes: after(self.shapes, earlier.shapes),
            texts: after(self.texts, earlier.texts),
            zones: after(self.zones, earlier.zones),
            fills: after(self.fills, earlier.fills),
        }
    }
}

/// What a change put in, changed or took away (`Patch::changed`).
pub(crate) struct Changed<'a> {
    pub tracks: Vec<&'a Track>,
    pub vias: Vec<&'a Via>,
    pub pads: Vec<&'a Pad>,
    pub shapes: Vec<&'a Shape>,
    pub texts: Vec<&'a Text>,
}

/// Patches made from one board, as one: each puts back or takes away items apart from the
/// others' (other picks), and they are made at once.
impl FromIterator<Patch> for Patch {
    fn from_iter<I: IntoIterator<Item = Patch>>(patches: I) -> Self {
        let mut all = Patch::default();
        for p in patches {
            all.nets.extend(p.nets);
            all.tracks.extend(p.tracks);
            all.vias.extend(p.vias);
            all.footprints.extend(p.footprints);
            all.pads.extend(p.pads);
            all.shapes.extend(p.shapes);
            all.texts.extend(p.texts);
            all.zones.extend(p.zones);
            all.fills.extend(p.fills);
        }
        // In order; of two at one place, the empty one first (a part without pads, then the next part's).
        fn order<T>(list: &mut [Splice<T>]) {
            list.sort_by_key(|s| (s.at, s.len));
            debug_assert!(list.windows(2).all(|w| w[0].at.saturating_add(w[0].len) <= w[1].at), "patches of the same items");
        }
        order(&mut all.nets);
        order(&mut all.tracks);
        order(&mut all.vias);
        order(&mut all.footprints);
        order(&mut all.pads);
        order(&mut all.shapes);
        order(&mut all.texts);
        order(&mut all.zones);
        order(&mut all.fills);
        all
    }
}

impl Board {
    /// Makes the patch's changes and returns the patch that undoes them.
    pub fn apply(&mut self, p: Patch) -> Patch {
        let back = Patch {
            nets: splice(&mut self.nets, p.nets),
            tracks: splice(&mut self.tracks, p.tracks),
            vias: splice(&mut self.vias, p.vias),
            footprints: splice(&mut self.footprints, p.footprints),
            pads: splice(&mut self.pads, p.pads),
            shapes: splice(&mut self.shapes, p.shapes),
            texts: splice(&mut self.texts, p.texts),
            zones: splice(&mut self.zones, p.zones),
            fills: splice(&mut self.fills, p.fills),
        };
        // Items put in or taken away move those after them in their lists: the parts the patch
        // didn't put in (those it did have their items where they are) follow their own, the
        // board's own texts and drawings among them or not. A part without items keeps its place
        // after the items of the parts before it.
        if [resized(&back.pads), resized(&back.shapes), resized(&back.texts)].contains(&true) {
            let n = self.footprints.len();
            let (mut put, mut ends) = (back.footprints.iter().flat_map(|s| s.put(n)).peekable(), [0; 3]);
            for (k, g) in self.footprints.iter_mut().enumerate() {
                if put.next_if_eq(&k).is_none() {
                    let to = |r: &Range<usize>, (a, b): (usize, usize), end: usize| {
                        let at = if r.is_empty() { end.clamp(a, b) } else { b };
                        at..at + r.len()
                    };
                    g.pads = to(&g.pads, span(&back.pads, g.pads.start), ends[0]);
                    g.shapes = to(&g.shapes, span(&back.shapes, g.shapes.start), ends[1]);
                    g.texts = to(&g.texts, span(&back.texts, g.texts.start), ends[2]);
                }
                ends = [ends[0].max(g.pads.end), ends[1].max(g.shapes.end), ends[2].max(g.texts.end)];
            }
        }
        // Parts put in or taken away renumber the pads of the parts after them: those the patch
        // didn't put in, by what the splices before them grew.
        if resized(&back.footprints) {
            let renumber =
                |pads: &mut [Pad], d: isize| pads.iter_mut().for_each(|q| q.footprint = q.footprint.wrapping_add_signed(d as i32));
            let (mut d, mut next) = (0, 0);
            for (f, p) in back.footprints.iter().zip(&back.pads) {
                renumber(&mut self.pads[next..p.at], d);
                (d, next) = (d + grown(f), p.at + p.len);
            }
            renumber(&mut self.pads[next..], d);
        }
        // Each zone's fills follow the zone before's: zones put in or taken away with their fills,
        // or given more or fewer fills, move the fills of the zones after them.
        if resized(&back.zones) || resized(&back.fills) {
            let mut at = 0;
            for z in &mut self.zones {
                z.fills = at..at + z.fills.len();
                at = z.fills.end;
            }
            debug_assert_eq!(at, self.fills.len(), "fills outside their zones");
        }
        back
    }
}

/// What edits change and patches undo: a board, or a project's schematic.
pub trait Edited {
    type Patch;
    /// Makes the patch's changes and returns the patch that undoes them.
    fn apply(&mut self, p: Self::Patch) -> Self::Patch;
}

impl Edited for Board {
    type Patch = Patch;
    fn apply(&mut self, p: Patch) -> Patch {
        Board::apply(self, p)
    }
}

/// The edits to undo and redo, each with what it did ("R1 placed"): of a board, or with
/// `History<schematic::Patch>` of a schematic.
#[derive(Clone)]
pub struct History<P = Patch> {
    undo: Vec<(P, String)>,
    redo: Vec<(P, String)>,
    /// How many edits the saved file had; `None` once no undo or redo leads back to it.
    saved: Option<usize>,
}

impl<P> Default for History<P> {
    fn default() -> Self {
        History { undo: vec![], redo: vec![], saved: Some(0) }
    }
}

impl<P> History<P> {
    /// The history of a file read from an autosave: its edits aren't in the file until saved.
    pub fn restored() -> Self {
        History { saved: None, ..Self::default() }
    }

    /// Keeps an edit, given the patch that undoes it; the edits undone before can't be redone.
    pub fn push(&mut self, undo: P, what: String) {
        if self.saved.is_some_and(|n| n > self.undo.len()) {
            self.saved = None;
        }
        self.undo.push((undo, what));
        self.redo.clear();
    }

    /// Undoes the last edit: what it did and the patch that redoes it (for `drc::recheck`), or
    /// `None` when there is none.
    pub fn undo(&mut self, d: &mut impl Edited<Patch = P>) -> Option<(&str, &P)> {
        step(&mut self.undo, &mut self.redo, d)
    }

    /// Redoes the last edit undone, like `undo`.
    pub fn redo(&mut self, d: &mut impl Edited<Patch = P>) -> Option<(&str, &P)> {
        step(&mut self.redo, &mut self.undo, d)
    }

    /// What `undo` and `redo` would do next.
    pub fn peek(&self) -> (Option<&str>, Option<&str>) {
        (self.undo.last().map(|(_, w)| w.as_str()), self.redo.last().map(|(_, w)| w.as_str()))
    }

    /// The file is saved as it is now.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.undo.len());
    }

    /// Whether there are edits the file lacks.
    pub fn unsaved(&self) -> bool {
        self.saved != Some(self.undo.len())
    }
}

/// Applies the last patch of `from` and keeps the one that reverts it on `to`.
fn step<'a, P>(from: &mut Vec<(P, String)>, to: &'a mut Vec<(P, String)>, d: &mut impl Edited<Patch = P>) -> Option<(&'a str, &'a P)> {
    let (p, what) = from.pop()?;
    to.push((d.apply(p), what));
    to.last().map(|(p, what)| (what.as_str(), p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{MM, Pt, Src};

    fn parts(b: &Board) -> (Vec<Footprint>, Vec<Pad>, Vec<Shape>, Vec<Text>) {
        (b.footprints.clone(), b.pads.clone(), b.shapes.clone(), b.texts.clone())
    }

    fn copper(b: &Board) -> (Vec<Track>, Vec<Via>) {
        (b.tracks.clone(), b.vias.clone())
    }

    /// What an undo or redo did and the items it put in or changed.
    fn step(done: Option<(&str, &Patch)>, b: &Board) -> Option<(String, Vec<Item>)> {
        done.map(|(what, p)| (what.to_string(), p.items(b)))
    }

    #[test]
    fn undoes_and_redoes_placements_and_added_copper() {
        let src = r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user)) (net 0 "") (net 1 "A")
              (footprint "R" (layer "F.Cu") (at 10 20) (property "Reference" "R1" (at 0 -1 0) (layer "F.SilkS"))
                (fp_line (start -1 0) (end 1 0) (stroke (width 0.1)) (layer "F.SilkS"))
                (pad "1" smd rect (at 1 0) (size 1 0.5) (layers "F.Cu") (net 1 "A")))
              (segment (start 0 0) (end 1 0) (width 0.25) (layer "F.Cu") (net 1)))"#;
        let (orig, mut b, mut h) = (Board::parse(src).unwrap(), Board::parse(src).unwrap(), History::default());
        assert_eq!((h.peek(), h.unsaved()), ((None, None), false));
        h.push(Patch::footprint(&b, 0), "R1 placed".into());
        b.place_footprint(0, Pt { x: 20 * MM, y: 30 * MM }, 90.0);
        let placed = parts(&b);
        let undo = Patch::added(&b);
        let route = b.add_tracks(&[Pt { x: MM, y: 0 }, Pt { x: 2 * MM, y: 0 }, Pt { x: 2 * MM, y: MM }], 250_000, 0, 1);
        let via = b.add_via(Pt { x: 2 * MM, y: MM }, 1).unwrap();
        // The patch made before the route knows what it added.
        let added = vec![Item::Track(1), Item::Track(2), Item::Via(0)];
        assert_eq!((undo.items(&b), undo.moved(Item::Track(0))), (added.clone(), Some(Item::Track(0))));
        h.push(undo, "via added".into());
        let routed = copper(&b);
        assert_eq!((route, via, h.peek(), h.unsaved()), (1..3, 0, (Some("via added"), None), true));
        // Undoing the route takes the tracks and via away and puts nothing in.
        assert_eq!(step(h.undo(&mut b), &b), Some(("via added".into(), vec![])));
        assert_eq!((copper(&b), h.peek()), (copper(&orig), (Some("R1 placed"), Some("via added"))));
        // Undoing the move puts the part, its pad, line and text back.
        assert_eq!(step(h.undo(&mut b), &b), Some(("R1 placed".into(), vec![Item::Pad(0)])));
        assert_eq!((parts(&b), step(h.undo(&mut b), &b), h.peek()), (parts(&orig), None, (None, Some("R1 placed"))));
        assert_eq!(step(h.redo(&mut b), &b), Some(("R1 placed".into(), vec![Item::Pad(0)])));
        assert_eq!((step(h.redo(&mut b), &b), &parts(&b), copper(&b)), (Some(("via added".into(), added)), &placed, routed));
        assert_eq!(step(h.redo(&mut b), &b), None);
    }

    #[test]
    fn deletes_parts_tracks_and_vias_and_puts_them_back() {
        let src = r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user)) (net 0 "") (net 1 "A")
              (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "R1" (at 0 -1 0) (layer "F.SilkS"))
                (pad "1" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
              (gr_line (start 0 5) (end 20 5) (stroke (width 0.1)) (layer "F.SilkS"))
              (footprint "C" (layer "F.Cu") (at 10 0) (property "Reference" "C1" (at 0 -1 0) (layer "F.SilkS"))
                (fp_line (start -1 0) (end 1 0) (stroke (width 0.1)) (layer "F.SilkS"))
                (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net 1 "A")) (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))
              (footprint "D" (layer "F.Cu") (at 20 0) (property "Reference" "D1" (at 0 -1 0) (layer "F.SilkS"))
                (fp_line (start -1 0) (end 1 0) (stroke (width 0.1)) (layer "F.SilkS"))
                (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
              (segment (start 1 0) (end 9 0) (width 0.25) (layer "F.Cu") (net 1))
              (segment (start 11 0) (end 19 0) (width 0.25) (layer "F.Cu") (net 1))
              (via (at 5 5) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1)))"#;
        let (orig, mut b, mut h) = (Board::parse(src).unwrap(), Board::parse(src).unwrap(), History::default());
        // C1 goes with its pads, line and text: D1's items move up and its pad is D1's again.
        let undo = b.apply(Patch::delete(&b, Pick::Footprint(1)));
        let d = &b.footprints[1];
        assert_eq!((b.footprints.len(), d.reference.as_str(), &d.src), (2, "D1", &Src::File(2)));
        assert_eq!((d.pads.clone(), d.shapes.clone(), d.texts.clone(), b.pads[1].footprint), (1..2, 1..2, 1..2, 1));
        assert_eq!((b.pads[1].at, b.shapes[0].geom.clone()), (orig.pads[3].at, orig.shapes[0].geom.clone()));
        let moved = [Item::Pad(0), Item::Pad(1), Item::Pad(3), Item::Track(1)].map(|i| undo.moved(i));
        assert_eq!((moved, undo.items(&b)), ([Some(Item::Pad(0)), None, Some(Item::Pad(1)), Some(Item::Track(1))], vec![]));
        h.push(undo, "C1 deleted".into());
        let undo = b.apply(Patch::delete(&b, Pick::Track(0)));
        assert_eq!((b.tracks[0], undo.moved(Item::Track(0)), undo.moved(Item::Track(1))), (orig.tracks[1], None, Some(Item::Track(0))));
        h.push(undo, "Track deleted".into());
        let undo = b.apply(Patch::delete(&b, Pick::Via(0)));
        h.push(undo, "Via deleted".into());
        let deleted = (parts(&b), copper(&b));
        assert_eq!((b.tracks.len(), b.vias.len()), (1, 0));
        // Undone, each comes back where it was and is what the undo put in.
        assert_eq!(step(h.undo(&mut b), &b), Some(("Via deleted".into(), vec![Item::Via(0)])));
        let (what, redo) = h.undo(&mut b).unwrap();
        assert_eq!((what, redo.items(&b), redo.moved(Item::Track(0))), ("Track deleted", vec![Item::Track(0)], Some(Item::Track(1))));
        assert_eq!(step(h.undo(&mut b), &b), Some(("C1 deleted".into(), vec![Item::Pad(1), Item::Pad(2)])));
        assert_eq!((parts(&b), copper(&b)), (parts(&orig), copper(&orig)));
        for _ in 0..3 {
            h.redo(&mut b);
        }
        assert_eq!((parts(&b), copper(&b)), deleted);
    }

    /// Parts R1, LOGO1 (no pads), C1 and D1; tracks of nets A, A, B, B; vias of A, B, A; a zone
    /// of A, one of B with two fills, another of A.
    const ITEMS: &str = r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user)) (net 0 "") (net 1 "A") (net 2 "B")
          (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "R1" (at 0 -1 0) (layer "F.SilkS"))
            (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net 1 "A")) (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 2 "B")))
          (footprint "Logo" (layer "F.Cu") (at 5 0) (property "Reference" "LOGO1" (at 0 -1 0) (layer "F.SilkS"))
            (fp_line (start -1 0) (end 1 0) (stroke (width 0.1)) (layer "F.SilkS")))
          (footprint "C" (layer "F.Cu") (at 10 0) (property "Reference" "C1" (at 0 -1 0) (layer "F.SilkS"))
            (fp_line (start -1 0) (end 1 0) (stroke (width 0.1)) (layer "F.SilkS"))
            (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net 1 "A")) (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))
          (footprint "D" (layer "F.Cu") (at 20 0) (property "Reference" "D1" (at 0 -1 0) (layer "F.SilkS"))
            (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net 2 "B")))
          (segment (start 1 0) (end 9 0) (width 0.25) (layer "F.Cu") (net 1))
          (segment (start 0 5) (end 9 5) (width 0.25) (layer "B.Cu") (net 1))
          (segment (start 11 0) (end 19 0) (width 0.25) (layer "F.Cu") (net 2))
          (segment (start 11 5) (end 19 5) (width 0.25) (layer "B.Cu") (net 2))
          (via (at 5 5) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1))
          (via (at 6 5) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 2))
          (via (at 7 5) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1))
          (zone (net 1) (layer "F.Cu") (polygon (pts (xy 0 10) (xy 5 10) (xy 5 15))) (filled_polygon (layer "F.Cu") (pts (xy 0 10) (xy 5 10) (xy 5 15))))
          (zone (net 2) (layer "F.Cu") (polygon (pts (xy 10 10) (xy 15 10) (xy 15 15)))
            (filled_polygon (layer "F.Cu") (pts (xy 10 10) (xy 12 10) (xy 12 12))) (filled_polygon (layer "F.Cu") (pts (xy 13 10) (xy 15 10) (xy 15 15))))
          (zone (net 1) (layer "B.Cu") (polygon (pts (xy 20 10) (xy 25 10) (xy 25 15))) (filled_polygon (layer "B.Cu") (pts (xy 20 10) (xy 25 10) (xy 25 15)))))"#;

    /// A part's reference with its own pads (where), graphics and texts.
    type Own = (String, Vec<Pt>, Vec<Shape>, Vec<Text>);

    /// Each part's own items, found by its ranges.
    fn own(b: &Board) -> Vec<Own> {
        let pads = |f: &Footprint| b.pads[f.pads.clone()].iter().map(|p| p.at).collect();
        b.footprints
            .iter()
            .map(|f| (f.reference.clone(), pads(f), b.shapes[f.shapes.clone()].to_vec(), b.texts[f.texts.clone()].to_vec()))
            .collect()
    }

    #[test]
    fn deletes_items_apart_as_one_edit_and_puts_them_back() {
        let (orig, mut b, mut h) = (Board::parse(ITEMS).unwrap(), Board::parse(ITEMS).unwrap(), History::default());
        // LOGO1 and C1, the first and third tracks, the middle via and the outer zones, at once.
        let picks = [Pick::Zone(2), Pick::Footprint(1), Pick::Track(0), Pick::Footprint(2), Pick::Via(1), Pick::Track(2), Pick::Zone(0)];
        let undo = b.apply(picks.iter().map(|&p| Patch::delete(&b, p)).collect());
        // R1 and D1 keep their own items, D1's pad is D1's; the rest of the copper moves up.
        let kept = |k: &[usize]| own(&orig).into_iter().enumerate().filter(|(i, _)| k.contains(i)).map(|(_, o)| o).collect::<Vec<_>>();
        assert_eq!((own(&b), b.pads.iter().map(|p| p.footprint).collect::<Vec<_>>()), (kept(&[0, 3]), vec![0, 0, 1]));
        assert_eq!(copper(&b), (vec![orig.tracks[1], orig.tracks[3]], vec![orig.vias[0], orig.vias[2]]));
        let z = &b.zones[..];
        assert_eq!((z.len(), &b.fills[z[0].fills.clone()], &z[0].outline), (1, &orig.fills[1..3], &orig.zones[1].outline));
        // Where the items before it are after it, none for those it took away.
        let moved = [Item::Pad(1), Item::Pad(2), Item::Pad(4), Item::Track(1), Item::Track(2), Item::Track(3), Item::Via(1), Item::Via(2)];
        let to = [Some(Item::Pad(1)), None, Some(Item::Pad(2)), Some(Item::Track(0)), None, Some(Item::Track(1)), None, Some(Item::Via(1))];
        assert_eq!(moved.map(|i| undo.moved(i)), to);
        assert_eq!((undo.items(&b), undo.nets(&b)), (vec![], Some([0, 1, 2].into())));
        h.push(undo, "deleted".into());
        let deleted = (parts(&b), copper(&b), b.zones.clone(), b.fills.clone());
        // Undone, every item is where it was, and what the undo put in; redone, gone again.
        let (_, redo) = h.undo(&mut b).unwrap();
        let back = [Item::Track(0), Item::Track(2), Item::Via(1), Item::Pad(2), Item::Pad(3)];
        assert_eq!(
            (redo.items(&b), redo.moved(Item::Track(1)), redo.moved(Item::Pad(2))),
            (back.to_vec(), Some(Item::Track(3)), Some(Item::Pad(4)))
        );
        assert_eq!((parts(&b), copper(&b), &b.zones, &b.fills), (parts(&orig), copper(&orig), &orig.zones, &orig.fills));
        h.redo(&mut b);
        assert_eq!((parts(&b), copper(&b), b.zones.clone(), b.fills.clone()), deleted);
    }

    #[test]
    fn puts_back_items_apart_changed_as_one_edit() {
        let (orig, mut b) = (Board::parse(ITEMS).unwrap(), Board::parse(ITEMS).unwrap());
        // C1, the second track, the first via and the zone with two fills move 1 mm right.
        let picks = [Pick::Footprint(2), Pick::Track(1), Pick::Via(0), Pick::Zone(1)];
        let undo: Patch = picks.iter().map(|&p| Patch::item(&b, p)).collect();
        let right = |p: &mut Pt| p.x += MM;
        let at = b.footprints[2].at;
        b.place_footprint(2, Pt { x: at.x + MM, ..at }, 0.0);
        let t = &mut b.tracks[1];
        right(&mut t.a);
        right(&mut t.b);
        right(&mut b.vias[0].at);
        b.zones[1].outline.iter_mut().flatten().for_each(right);
        b.fills[1..3].iter_mut().flat_map(|f| &mut f.pts).for_each(right);
        // What it changed, for the re-checks: the track, the via and C1's pads; the nets of all
        // of them and of the fills.
        assert_eq!(
            (undo.items(&b), undo.nets(&b)),
            (vec![Item::Track(1), Item::Via(0), Item::Pad(2), Item::Pad(3)], Some([0, 1, 2].into()))
        );
        assert_eq!(
            (undo.moved(Item::Track(0)), undo.moved(Item::Track(1)), undo.moved(Item::Pad(4))),
            (Some(Item::Track(0)), None, Some(Item::Pad(4)))
        );
        let moved = (parts(&b), copper(&b), b.zones.clone(), b.fills.clone());
        let redo = b.apply(undo);
        assert_eq!((parts(&b), copper(&b), &b.zones, &b.fills), (parts(&orig), copper(&orig), &orig.zones, &orig.fills));
        b.apply(redo);
        assert_eq!((parts(&b), copper(&b), b.zones.clone(), b.fills.clone()), moved);
    }

    #[test]
    fn undoes_and_redoes_zone_fills() {
        // A zone with a stale fill and one never filled: filled, they hold a fill each.
        let src = r#"(kicad_pcb (version 20260206) (layers (0 "F.Cu" signal)) (net 0 "") (net 1 "A")
              (zone (net 1) (layer "F.Cu") (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))
                (filled_polygon (layer "F.Cu") (pts (xy 1 1) (xy 2 1) (xy 2 2))))
              (zone (net 1) (layer "F.Cu") (polygon (pts (xy 20 0) (xy 30 0) (xy 30 10)))))"#;
        let (orig, mut b, mut h) = (Board::parse(src).unwrap(), Board::parse(src).unwrap(), History::default());
        let undo = Patch::fills(&b);
        assert_eq!(b.fill_zones(), 2);
        let filled = (b.zones.clone(), b.fills.clone());
        assert_eq!((filled.1.len(), filled.0[1].fills.clone()), (2, 1..2));
        h.push(undo, "2 zones filled".into());
        assert_eq!(step(h.undo(&mut b), &b), Some(("2 zones filled".into(), vec![])));
        assert_eq!((&b.zones, &b.fills), (&orig.zones, &orig.fills));
        h.redo(&mut b);
        assert_eq!((b.zones.clone(), b.fills.clone()), filled);
        // Filled again, no zone's fill changes.
        assert_eq!(b.fill_zones(), 0);
        // A zone added goes with its fill, and comes back.
        let (filled, undo) = ((b.zones.clone(), b.fills.clone()), Patch::added(&b));
        let corners = vec![Pt { x: 0, y: 20 * MM }, Pt { x: 10 * MM, y: 20 * MM }, Pt { x: 0, y: 30 * MM }];
        let i = b.add_zone(corners, 0, 1);
        let added = (b.zones.clone(), b.fills.clone());
        assert_eq!((i, added.0[i].fills.clone()), (2, 2..3));
        h.push(undo, "Zone (A) on F.Cu added".into());
        assert_eq!(step(h.undo(&mut b), &b), Some(("Zone (A) on F.Cu added".into(), vec![])));
        assert_eq!((b.zones.clone(), b.fills.clone()), filled);
        h.redo(&mut b);
        assert_eq!((b.zones.clone(), b.fills.clone()), added);
    }

    #[test]
    fn knows_when_the_board_is_as_saved() {
        let (mut b, mut h) = (Board::default(), History::default());
        let edit = |h: &mut History, b: &Board| h.push(Patch::added(b), "edit".into());
        edit(&mut h, &b);
        h.mark_saved();
        assert!(!h.unsaved());
        assert!(h.undo(&mut b).is_some() && h.unsaved());
        assert!(h.redo(&mut b).is_some() && !h.unsaved());
        // A new edit after an undo: the saved board is gone for good.
        h.undo(&mut b);
        edit(&mut h, &b);
        assert!(h.unsaved() && h.undo(&mut b).is_some() && h.unsaved());
        // A restored autosave differs from its file until it is saved, whatever is undone.
        let mut h = History::restored();
        assert!(h.unsaved() && h.peek() == (None, None));
        edit(&mut h, &b);
        assert!(h.undo(&mut b).is_some() && h.unsaved());
        h.mark_saved();
        assert!(!h.unsaved());
    }
}
