//! Placed items moved, turned and deleted, alone or together, as edits of their sheets' text that
//! undo themselves: symbols, wires, net labels and no-connect flags. What joins a moved item stays joined, and the
//! edit joins nothing by the way: a wire with an end on a moving point (a symbol's pin, a wire's
//! end, a junction or label along a moved wire) keeps its part up to the first thing joining it
//! there (a junction, a label), and from there a new stretch reaches the point's new place; where
//! something stays on the old place (another symbol's or a sheet's pin, a bus entry, a wire passing
//! with a junction, two wires' ends), a new wire joins it. Each new stretch is straight, else bent
//! once either way, else slanting: the first that meets no point where something joins (refused
//! when none does). No-connect flags go along, labels and junctions where nothing stays, and a
//! moved point that now needs a junction gets one. A label moved itself names what it lands on.

use super::edit::{Patch, items_end, junction_text, needs_junction, on_grid, wire_text, xy};
use super::net::on;
use super::symbol::turn;
use super::{Item, Project, Schematic};
use crate::board::Pt;
use crate::save::{At, Edit, angle_edit, read_at, uuid};
use crate::sexpr::{Cursor, Result, unescape};
use crate::tr::{t, tr, trf};
use std::collections::{HashMap, HashSet};
use std::ops::Range;

impl Project {
    /// Moves `items` of page `page` by `by`, taken to the grid, with what joins them. Returns the
    /// patch that undoes it, and the items as the page has them now.
    pub fn move_items(&mut self, page: usize, items: &[Item], by: Pt) -> std::result::Result<(Patch, Vec<Item>), String> {
        let by = on_grid(by);
        self.reshape(page, items, move |p| Pt { x: p.x + by.x, y: p.y + by.y }, How::Turn(0.0))
    }

    /// Mirrors `items` of page `page` with what joins them, left to right (KiCad's Mirror
    /// Horizontally) or, `up_down`, top to bottom: about what `turn_items` turns them about. A
    /// symbol takes KiCad's `(mirror y)` or `(mirror x)`, or loses it, or turns half round where
    /// it had the other; its fields go to the other side of it. A label reads away from its
    /// point still. Returns the patch that undoes it, and the items as the page has them now.
    pub fn mirror_items(&mut self, page: usize, items: &[Item], up_down: bool) -> std::result::Result<(Patch, Vec<Item>), String> {
        let o = self.pivot(page, items)?;
        let to = move |p: Pt| if up_down { Pt { x: p.x, y: 2 * o.y - p.y } } else { Pt { x: 2 * o.x - p.x, y: p.y } };
        self.reshape(page, items, to, How::Mirror(up_down))
    }

    /// Turns `items` of page `page` a quarter turn counterclockwise with what joins them: a symbol
    /// alone about its origin, a label alone about its point, else about the grid point nearest
    /// their middle. A symbol's fields turn with it, their angles staying in its frame (as KiCad
    /// keeps them); a label's text keeps reading away from its point. Returns the patch that undoes
    /// it, and the items as the page has them now.
    pub fn turn_items(&mut self, page: usize, items: &[Item]) -> std::result::Result<(Patch, Vec<Item>), String> {
        let o = self.pivot(page, items)?;
        // Y points down the sheet: counterclockwise takes (dx, dy) to (dy, -dx).
        self.reshape(page, items, move |p| Pt { x: o.x + p.y - o.y, y: o.y - (p.x - o.x) }, How::Turn(90.0))
    }

    /// Deletes `items` of page `page`: a symbol's part with its every unit on every sheet (a sheet
    /// used twice once, and a KiCad 6 root's list of its places), wires, labels and no-connect
    /// flags. What was wired
    /// to them stays; junctions where they were that no longer join three or more go too, as KiCad
    /// cleans them up. Returns the patch that undoes it.
    pub fn delete_items(&mut self, page: usize, items: &[Item]) -> std::result::Result<Patch, String> {
        let (file, src) = self.page_text(page)?;
        let sheet = &self.pages[page].sheet;
        let (items, w) = (sheet.check(items, page)?, &sheet.wiring);
        let entries = entries(src).map_err(|e| format!("{file}: {}", e.msg))?;
        let mut parts: Vec<&str> =
            items.iter().filter_map(|&it| if let Item::Symbol(i) = it { Some(&*sheet.symbols[i].designator) } else { None }).collect();
        parts.sort_unstable();
        parts.dedup();
        let gone = |symbol: usize| parts.contains(&&*sheet.symbols[symbol].designator);
        // With the space before it, as KiCad writes each item on its own line.
        let line = |span: &Range<usize>| src[..span.start].trim_end().len()..span.end;
        let (mut edits, mut wires, mut at) = (vec![], vec![], vec![]);
        for &it in &items {
            match it {
                Item::Wire(i) => {
                    // shortcut: a wire written as a path of several points goes whole; KiCad writes two.
                    let e = wire(&entries, i).ok_or_else(|| trf("{0}: wire {1} isn't in its text", &[&file, &i]))?;
                    let (a, b) = w.wires[i];
                    edits.push((line(&e.span), String::new()));
                    wires.push(i);
                    at.extend(w.junctions.iter().copied().filter(|&p| on(p, a, b)));
                }
                Item::Label(i) => edits.push((line(&w.labels[i].span), String::new())),
                Item::NoConnect(i) => {
                    let e =
                        flag(&entries, w.no_connects[i]).ok_or_else(|| trf("{0}: no-connect flag {1} isn't in its text", &[&file, &i]))?;
                    edits.push((line(&e.span), String::new()));
                }
                Item::Symbol(_) => {}
            }
        }
        // The junctions on them or on the parts' pins that no longer join three or more.
        at.extend(w.pins.iter().filter(|q| gone(q.symbol)).map(|q| q.at));
        let left: Vec<(Pt, Pt)> = (0..w.wires.len()).filter(|i| !wires.contains(i)).map(|i| w.wires[i]).collect();
        let ends = sheet.ends(|i, p| (!gone(i)).then_some(p));
        for e in entries.iter().filter(|e| e.head == "junction") {
            if let [(p, _)] = &e.points[..]
                && at.contains(p)
                && !needs_junction(*p, &left, &ends)
            {
                edits.push((line(&e.span), String::new()));
            }
        }
        edits.sort_by_key(|e| e.0.start);
        edits.dedup_by_key(|e| e.0.start);
        let mut files = vec![(file.to_string(), edits)];
        for part in parts {
            self.part_edits(part, &mut files)?;
        }
        Ok(self.apply(Patch(files)))
    }

    /// Adds the edits deleting part `designator` from every sheet (a sheet used twice once) and
    /// from a KiCad 6 root's list of its places to `files`.
    fn part_edits(&self, designator: &str, files: &mut Vec<(String, Vec<Edit>)>) -> std::result::Result<(), String> {
        let mut push = |file: &str, edit: Edit| match files.iter_mut().find(|(f, _)| f == file) {
            Some((_, edits)) => edits.push(edit),
            None => files.push((file.to_string(), vec![edit])),
        };
        let (mut done, mut links) = (HashSet::new(), vec![]);
        for page in &self.pages {
            let Some(text) = self.text(&page.file) else { continue };
            for s in page.sheet.symbols.iter().filter(|s| s.designator == designator) {
                links.push(s.link.clone());
                if done.insert((page.file.clone(), s.span.start)) {
                    push(&page.file, (text[..s.span.start].trim_end().len()..s.span.end, String::new()));
                }
            }
        }
        if let Some(root) = self.text(&self.root) {
            for span in instances(root, &links).map_err(|e| format!("{}: {}", self.root, e.msg))? {
                push(&self.root.clone(), (root[..span.start].trim_end().len()..span.end, String::new()));
            }
        }
        Ok(())
    }

    /// What `turn_items` turns `items` about.
    fn pivot(&self, page: usize, items: &[Item]) -> std::result::Result<Pt, String> {
        let (file, src) = self.page_text(page)?;
        let sheet = &self.pages[page].sheet;
        match sheet.check(items, page)?[..] {
            [Item::Symbol(i)] => {
                let s = &sheet.symbols[i];
                let p = placed(&src[s.span.clone()]).map_err(|e| format!("{file}: {}", e.msg))?;
                p.at.map(|a| a.at).ok_or_else(|| trf("{0} has no place in {1}", &[&s.reference, &file]))
            }
            [Item::Label(i)] => Ok(sheet.wiring.labels[i].at),
            ref items => {
                let pts: Vec<Pt> = items.iter().flat_map(|&it| <[Pt; 2]>::from(sheet.extent(it))).collect();
                let mid = |v: fn(&Pt) -> i32| (pts.iter().map(v).min().unwrap_or(0) + pts.iter().map(v).max().unwrap_or(0)) / 2;
                Ok(on_grid(Pt { x: mid(|p| p.x), y: mid(|p| p.y) }))
            }
        }
    }

    /// Takes every place of `items` of page `page` (symbols' and labels' own and their fields',
    /// wires' ends) and the points they join on to `to(place)`, symbols and labels turned or
    /// mirrored as `how` says; returns the patch that undoes it, and the items as the page has
    /// them now.
    fn reshape(&mut self, page: usize, items: &[Item], to: impl Fn(Pt) -> Pt, how: How) -> std::result::Result<(Patch, Vec<Item>), String> {
        let (file, src) = self.page_text(page)?;
        let sheet = &self.pages[page].sheet;
        let (items, w) = (sheet.check(items, page)?, &sheet.wiring);
        let (mut edits, mut pins, mut moving) = (vec![], vec![], Moving::default());
        let read = |span: &Range<usize>| placed(&src[span.clone()]).map_err(|e| format!("{file}: {}", e.msg));
        let entries = entries(src).map_err(|e| format!("{file}: {}", e.msg))?;
        let lost = |it: Item| trf("{0} has no place in {1}", &[&sheet.describe(it), &file]);
        // A mirror is made in place, as KiCad's: a pin that takes the place of another of what is
        // mirrored takes what is wired there (a diode turned round, an amplifier's inputs changed
        // over), and only a pin going elsewhere draws its wires after it.
        let symbols: HashSet<usize> = items.iter().filter_map(|&it| if let Item::Symbol(i) = it { Some(i) } else { None }).collect();
        let places: HashSet<Pt> =
            w.pins.iter().filter(|q| matches!(how, How::Mirror(_)) && symbols.contains(&q.symbol)).map(|q| q.at).collect();
        for &it in &items {
            match it {
                Item::Symbol(i) => {
                    let s = &sheet.symbols[i];
                    let p = read(&s.span)?;
                    let flipped = p.mirror.is_some();
                    let symbol = match how {
                        // A mirrored symbol turns the other way as its angle grows: it takes a quarter less.
                        How::Turn(turn) => p.edits(s.span.start, if flipped { -turn } else { turn }, &to),
                        How::Mirror(up_down) => p.mirrored(&src[s.span.clone()], s.span.start, up_down, &to),
                    };
                    edits.extend(symbol.ok_or_else(|| lost(it))?);
                    pins.extend(w.pins.iter().filter(|q| q.symbol == i && !places.contains(&to(q.at))).map(|q| (q.at, to(q.at))));
                    moving.symbols.insert(i);
                }
                Item::Wire(i) => {
                    // Its ends, and the junctions and labels along it, which go with it.
                    let (a, b) = w.wires[i];
                    let along = w.junctions.iter().copied().chain(w.labels.iter().map(|l| l.at)).filter(|&p| on(p, a, b));
                    pins.extend([a, b].into_iter().chain(along).map(|p| (p, to(p))));
                }
                Item::Label(i) => {
                    let (l, p) = (&w.labels[i], read(&w.labels[i].span)?);
                    // A label mirrored reads the other way along the axis it flips over: half a turn.
                    let turn = match (how, p.at.as_ref().map(|a| a.deg.rem_euclid(180.0) == 0.0)) {
                        (How::Turn(turn), _) => turn,
                        (How::Mirror(up_down), Some(level)) if level != up_down => 180.0,
                        _ => 0.0,
                    };
                    edits.extend(p.edits(l.span.start, turn, &to).ok_or_else(|| lost(it))?);
                    moving.labels.insert(l.span.start);
                    moving.placed.push((l.at, to(l.at)));
                }
                // A flag goes alone: what is on its point stays.
                Item::NoConnect(i) => {
                    let at = w.no_connects[i];
                    let e = flag(&entries, at).ok_or_else(|| lost(it))?;
                    edits.push((e.points[0].1.clone(), xy(to(at))));
                    moving.labels.insert(e.span.start);
                }
            }
        }
        pins.sort_by_key(|&(p, _)| (p.x, p.y));
        pins.dedup();
        edits.extend(rewire(src, sheet, &pins, &moving, file, how)?);
        let ends: Vec<Option<(Pt, Pt)>> = items.iter().map(|&it| if let Item::Wire(i) = it { Some(w.wires[i]) } else { None }).collect();
        let patch = Patch(vec![(file.to_string(), edits)]);
        let undo = self.apply_readable(patch, t!("the edit wasn't added, as it would leave a sheet unreadable: {0}"))?;
        // Wires found again by their ends: one before them that went shifts them.
        let now = &self.pages[page].sheet.wiring.wires;
        let found = |ends: Option<(Pt, Pt)>| ends.and_then(|(a, b)| now.iter().position(|&l| l == (to(a), to(b))));
        Ok((undo, items.iter().zip(ends).map(|(&it, e)| found(e).map_or(it, Item::Wire)).collect()))
    }
}

impl Schematic {
    /// `items` once each, when the sheet has them all; else which one it lacks.
    fn check(&self, items: &[Item], page: usize) -> std::result::Result<Vec<Item>, String> {
        let w = &self.wiring;
        let has = |it: &Item| match *it {
            Item::Symbol(i) => i < self.symbols.len(),
            Item::Wire(i) => i < w.wires.len(),
            Item::Label(i) => i < w.labels.len(),
            Item::NoConnect(i) => i < w.no_connects.len(),
        };
        match items.iter().find(|it| !has(it)) {
            Some(it) => Err(trf("page {0} has no {1}", &[&page, &format!("{it:?}")])),
            None if items.is_empty() => Err(tr("no items given: pick a symbol, wire, label or no-connect flag").into()),
            None => {
                let mut items = items.to_vec();
                items.sort_unstable();
                items.dedup();
                Ok(items)
            }
        }
    }
}

/// What moves itself besides the points `rewire` takes: symbols (by index), labels and no-connect
/// flags (where their text starts), and where those labels are and go.
#[derive(Default)]
struct Moving {
    symbols: HashSet<usize>,
    labels: HashSet<usize>,
    placed: Vec<(Pt, Pt)>,
}

/// How `reshape` turns what it takes elsewhere: by degrees (none: a move), or mirrored left to
/// right or (`true`) top to bottom.
#[derive(Clone, Copy)]
enum How {
    Turn(f64),
    Mirror(bool),
}

/// What a placed symbol's or label's text places: its own `(at X Y [A])` and where that list
/// ends, its fields', its `(mirror x)` or `(mirror y)` (the list and which), and where its
/// justification says `left` or `right`.
#[derive(Default)]
struct Placed {
    at: Option<At>,
    at_end: usize,
    fields: Vec<At>,
    mirror: Option<(Range<usize>, bool)>,
    side: Option<Range<usize>>,
}

impl Placed {
    /// The edits taking its places (its text at `start` in the sheet's) to `to(place)`, turned
    /// `turn` degrees; a label's text reading away from its point: from it at 0° and 90°, towards
    /// it at 180° and 270°. None without a place of its own.
    fn edits(self, start: usize, turn: f64, to: &impl Fn(Pt) -> Pt) -> Option<Vec<Edit>> {
        let (own, shift) = (self.at?, |r: Range<usize>| start + r.start..start + r.end);
        let deg = (own.deg + turn).rem_euclid(360.0);
        let mut edits: Vec<Edit> = angle_edit(shift(own.angle.clone()), deg).filter(|_| turn != 0.0).into_iter().collect();
        edits.extend(self.side.filter(|_| turn != 0.0).map(|s| (shift(s), (if deg < 180.0 { "left" } else { "right" }).into())));
        edits.extend([own].into_iter().chain(self.fields).map(|a| (shift(a.xy), xy(to(a.at)))));
        Some(edits)
    }

    /// The edits mirroring a symbol (`text`, at `start` in the sheet's) left to right or
    /// (`up_down`) top to bottom, its places taken to `to(place)`: the angle and mirror that
    /// make its frame the mirror of what it was, as KiCad would write them (`symbol::turn`).
    fn mirrored(self, text: &str, start: usize, up_down: bool, to: &impl Fn(Pt) -> Pt) -> Option<Vec<Edit>> {
        let (own, shift) = (self.at?, |r: Range<usize>| start + r.start..start + r.end);
        let kind = |up_down: Option<bool>| up_down.map_or("", |x| if x { "x" } else { "y" });
        let (was, flip) = (turn(own.deg, kind(self.mirror.as_ref().map(|m| m.1))), turn(0.0, kind(Some(up_down))));
        let want = [0, 1].map(|i| [0, 1].map(|j| flip[i][0] * was[0][j] + flip[i][1] * was[1][j]));
        // Its angle kept where a mirror taken, given or changed does it; else half a turn more.
        let ways = [0.0, 180.0, 90.0, 270.0]
            .into_iter()
            .flat_map(|more| [None, Some(false), Some(true)].map(|m| ((own.deg + more).rem_euclid(360.0), m)));
        let (deg, mirror) = ways.into_iter().find(|&(deg, m)| turn(deg, kind(m)) == want)?;
        let mut edits: Vec<Edit> =
            angle_edit(shift(own.angle.clone()), deg).filter(|_| deg != own.deg.rem_euclid(360.0)).into_iter().collect();
        match (&self.mirror, mirror) {
            (Some((list, x)), Some(now)) if *x != now => edits.push((shift(list.end - 2..list.end - 1), kind(mirror).into())),
            // With the space before it, as each list is on its own line (or after a space).
            (Some((list, _)), None) => edits.push((shift(text[..list.start].trim_end().len()..list.end), String::new())),
            (None, Some(_)) => {
                // After its place, set off as the list after that is.
                let gap = &text[self.at_end..self.at_end + text[self.at_end..].find('(').unwrap_or(0)];
                edits.push((shift(self.at_end..self.at_end), format!("{gap}(mirror {})", kind(mirror))));
            }
            _ => {}
        }
        edits.extend([own].into_iter().chain(self.fields).map(|a| (shift(a.xy), xy(to(a.at)))));
        Some(edits)
    }
}

/// Reads a placed symbol's or label's text: `(symbol …)`, `(label "SDA" …)`.
fn placed(text: &str) -> Result<Placed> {
    let (c, mut p) = (&mut Cursor::new(text), Placed::default());
    c.tok()?;
    c.atom()?;
    while let Some(head) = c.child()? {
        let list = text[..c.span().start].rfind('(').unwrap_or(0);
        match head {
            "at" => p.at = Some(read_at(c)?),
            "mirror" => {
                let up_down = c.atom()? == "x";
                c.close()?;
                p.mirror = Some((list..c.span().end, up_down));
                continue;
            }
            "property" | "effects" => {
                while let Some(head) = c.child()? {
                    if head == "at" {
                        p.fields.push(read_at(c)?);
                    }
                    while head == "justify"
                        && let Some(a) = c.opt_atom()?
                    {
                        p.side = p.side.take().or(matches!(a, "left" | "right").then(|| c.span()));
                    }
                    c.close()?;
                }
            }
            _ => {}
        }
        c.close()?;
        if head == "at" {
            p.at_end = c.span().end;
        }
    }
    Ok(p)
}

/// A wire with an end on a moving point (`pin`): where that end's X Y are, its far end, the first
/// point joining it from the pin (its far end when none does), and where the whole wire is.
struct Held {
    pin: Pt,
    end: Range<usize>,
    far: Pt,
    near: Pt,
    whole: Range<usize>,
}

/// What the sheet (`src` its text) holds on points moving as `pins` says (before, after), besides
/// what moves itself (`moving`): the edits rewiring it, as the module says.
fn rewire(
    src: &str,
    sheet: &Schematic,
    pins: &[(Pt, Pt)],
    moving: &Moving,
    file: &str,
    how: How,
) -> std::result::Result<Vec<Edit>, String> {
    let (w, moved): (_, HashMap<Pt, Pt>) = (&sheet.wiring, pins.iter().copied().collect());
    // Only bus entries and bus labels join a bus: nothing that joins wires goes onto one.
    let on_bus = |p: Pt| w.buses.iter().any(|&(a, b)| on(p, a, b));
    if let Some(&(_, q)) = pins.iter().chain(&moving.placed).find(|&&(p, q)| on_bus(q) && !on_bus(p)) {
        return Err(trf("{0} mm is on a bus, which only bus entries and bus labels join: move it elsewhere", &[&xy(q)]));
    }
    let sheet_pins: Vec<Pt> = sheet.sheets.iter().flat_map(|sh| sh.pins.iter().map(|p| p.1)).collect();
    let inside = |p: Pt, (a, b): (Pt, Pt)| p != a && p != b && on(p, a, b);
    let whole = |(a, b): (Pt, Pt)| moved.contains_key(&a) && moved.contains_key(&b);
    // Where something joins, as the sheet is.
    let mut joins: Vec<Pt> = w.junctions.iter().copied().chain(w.labels.iter().map(|l| l.at)).collect();
    joins.extend(w.wires.iter().flat_map(|&(a, b)| [a, b]).chain(w.entries.iter().flat_map(|&(a, b)| [a, b])));
    joins.extend(w.pins.iter().map(|q| q.at).chain(sheet_pins.iter().copied()));
    // What stays on a moving point's old place: a pin of a symbol staying or of a sheet, a bus
    // entry, a wire passing with a junction, or two wires ending there that don't move whole.
    let stays = |p: Pt| {
        w.pins.iter().any(|q| q.at == p && !moving.symbols.contains(&q.symbol))
            || sheet_pins.contains(&p)
            || w.entries.iter().any(|&(a, b)| a == p || b == p)
            || w.junctions.contains(&p) && w.wires.iter().any(|&l| inside(p, l) && !whole(l))
            || w.wires.iter().filter(|&&(a, b)| (a == p || b == p) && !whole((a, b))).count() > 1
    };
    let entries = entries(src).map_err(|e| e.msg)?;
    let mut edits: Vec<Edit> = vec![];
    // Wires with both ends moving go along whole; one with an end on a moving point where nothing
    // stays is held; the rest stay, and so does what new stretches must bend clear of.
    let (mut held, mut wires, mut going) = (vec![], vec![], vec![]);
    for e in entries.iter().filter(|e| e.head == "wire") {
        let [(a, sa), (b, sb)] = &e.points[..] else {
            wires.extend(e.points.windows(2).map(|s| (s[0].0, s[1].0)));
            continue;
        };
        let end = [(*a, sa, *b), (*b, sb, *a)].into_iter().find(|(p, ..)| moved.get(p).is_some_and(|q| q != p));
        match (moved.get(a), moved.get(b), end) {
            (Some(&ta), Some(&tb), _) => {
                edits.extend([(sa.clone(), xy(ta)), (sb.clone(), xy(tb))]);
                going.push((ta, tb));
            }
            (.., Some((pin, span, far))) if !stays(pin) => {
                let d2 = |p: Pt| (p.x as i64 - pin.x as i64).pow(2) + (p.y as i64 - pin.y as i64).pow(2);
                // The first point joining it from the pin: a junction, or a label staying there.
                let labels = w.labels.iter().filter(|l| !moving.labels.contains(&l.span.start)).map(|l| l.at);
                let near = w.junctions.iter().copied().chain(labels).filter(|&p| inside(p, (pin, far))).min_by_key(|&p| d2(p));
                held.push(Held { pin, end: span.clone(), far, near: near.unwrap_or(far), whole: e.span.clone() });
            }
            _ => wires.push((*a, *b)),
        }
    }
    // Wires lying along each other look joined but join only where an end meets: a wire going
    // along may not lie along one that stays.
    if let Some(&(a, _)) = going.iter().find(|&&m| wires.iter().any(|&k| along(m, k))) {
        return Err(trf("the wire from {0} mm would lie along another wire: move it elsewhere", &[&xy(a)]));
    }
    wires.extend(going);
    held.sort_by_key(|h| (h.pin.x, h.pin.y, h.far.x, h.far.y));
    wires.extend(held.iter().filter(|h| h.near != h.far).map(|h| (h.near, h.far)));
    // Where something joins once it has moved, to keep new stretches clear of: the moving points'
    // old places hold only what stays there.
    let mut points: Vec<Pt> = joins.iter().copied().filter(|p| !moved.contains_key(p) || stays(*p)).collect();
    points.extend(pins.iter().map(|p| p.1).chain(moving.placed.iter().map(|p| p.1)));
    // New stretches from where wires hold on, then from what stays on old places.
    let starts = held.iter().map(|h| (h.near, moved[&h.pin]));
    let starts: Vec<(Pt, Pt)> = starts.chain(pins.iter().copied().filter(|&(p, q)| p != q && stays(p))).collect();
    let (mut text, mut along) = (String::new(), vec![]);
    for (k, &(from, to)) in starts.iter().enumerate() {
        // What stays on an old place may be joined to the new one by a wire already.
        if k >= held.len() && wires.iter().any(|&(a, b)| on(from, a, b) && on(to, a, b)) {
            along.push(from);
            continue;
        }
        let path = clear_path(from, to, &points, &wires, &w.buses)
            .ok_or_else(|| trf("a wire can't reach {0} mm there without touching other wiring: move it elsewhere", &[&xy(to)]))?;
        let mut rest = &path[..];
        if let Some(h) = held.get(k) {
            // The held wire becomes the stretch's first part when it held on nowhere between;
            // it goes when the point lands on its far end.
            match (h.near == h.far, path.get(1)) {
                (true, None) => edits.push((src[..h.whole.start].trim_end().len()..h.whole.end, String::new())),
                (true, Some(&first)) => {
                    edits.push((h.end.clone(), xy(first)));
                    rest = &path[1..];
                }
                (false, _) => edits.push((h.end.clone(), xy(h.near))),
            }
        }
        for (j, seg) in rest.windows(2).enumerate() {
            text += &wire_text(seg[0], seg[1], &uuid((file, src.len(), from.x, from.y, k, j)));
        }
        points.extend(path.iter().copied());
        wires.extend(path.windows(2).map(|s| (s[0], s[1])));
    }
    // A moving point's junction goes along where nothing stays; where one is needed now (a wire's
    // end or a pin on a wire's middle, three ends), at a new place or an old one a wire joins it
    // to, it is added, and one no longer needed where something stays goes, as KiCad's editor does.
    let junctions: Vec<Pt> = w.junctions.iter().map(|&p| moved.get(&p).copied().filter(|_| !stays(p)).unwrap_or(p)).collect();
    // A pin mirrored onto another's place isn't among those moved: a pin is there as before.
    let ends = sheet.ends(|i, p| Some(moved.get(&p).copied().filter(|_| moving.symbols.contains(&i)).unwrap_or(p)));
    let needed = |p: Pt| needs_junction(p, &wires, &ends);
    for q in pins.iter().map(|p| p.1).chain(along).filter(|&q| !junctions.contains(&q) && needed(q)) {
        text += &junction_text(q, &uuid((file, src.len(), q.x, q.y)));
    }
    if !text.is_empty() {
        let end = items_end(src).map_err(|e| e.msg)?;
        edits.push((end..end, text));
    }
    // No-connect flags go along; labels and junctions too where nothing stays.
    for e in entries.iter().filter(|e| e.head != "wire" && !moving.labels.contains(&e.span.start)) {
        let [(p, span)] = &e.points[..] else { continue };
        match moved.get(p).filter(|&to| to != p) {
            Some(&to) if e.head == "no_connect" || !stays(*p) => {
                // A label going with a pin to a symbol's other side turns to read away from it still.
                let turned = match (how, e.head.ends_with("label")) {
                    (How::Mirror(up_down), true) => placed(&src[e.span.clone()]).ok().and_then(|l| {
                        let (level, by) = (l.at.as_ref()?.deg.rem_euclid(180.0) == 0.0, (to.x - p.x, to.y - p.y));
                        (level != up_down).then(|| l.edits(e.span.start, 180.0, &|q: Pt| Pt { x: q.x + by.0, y: q.y + by.1 })).flatten()
                    }),
                    _ => None,
                };
                match turned {
                    Some(turned) => edits.extend(turned),
                    None => edits.push((span.clone(), xy(to))),
                }
            }
            Some(_) if e.head == "junction" && !needed(*p) => edits.push((src[..e.span.start].trim_end().len()..e.span.end, String::new())),
            _ => {}
        }
    }
    Ok(edits)
}

/// Whether segments `s` and `t` lie along each other for more than a point.
fn along(s: (Pt, Pt), t: (Pt, Pt)) -> bool {
    let shared: Vec<Pt> =
        [s.0, s.1].into_iter().filter(|&p| on(p, t.0, t.1)).chain([t.0, t.1].into_iter().filter(|&p| on(p, s.0, s.1))).collect();
    shared.iter().any(|&p| p != shared[0])
}

/// The first of the ways from `from` to `to` (straight, bent once either way, slanting) whose
/// corners and lines meet none of `points` but at its ends nor lie along a line of `wires` or
/// `buses`, its bend on none of them; just `from` when it is `to`.
fn clear_path(from: Pt, to: Pt, points: &[Pt], wires: &[(Pt, Pt)], buses: &[(Pt, Pt)]) -> Option<Vec<Pt>> {
    if from == to {
        return Some(vec![from]);
    }
    let (h, v) = (Pt { x: to.x, y: from.y }, Pt { x: from.x, y: to.y });
    let straight = from.x == to.x || from.y == to.y;
    let ways = [
        straight.then(|| vec![from, to]),
        (!straight).then(|| vec![from, h, to]),
        (!straight).then(|| vec![from, v, to]),
        Some(vec![from, to]),
    ];
    let lines = || wires.iter().chain(buses);
    ways.into_iter().flatten().find(|way| {
        let clear =
            |s: &[Pt]| !points.iter().any(|&p| p != s[0] && p != s[1] && on(p, s[0], s[1])) && !lines().any(|&l| along((s[0], s[1]), l));
        let bend = way.len() < 3 || !points.contains(&way[1]) && !lines().any(|&(a, b)| on(way[1], a, b));
        way.windows(2).all(clear) && bend
    })
}

/// A sheet's wire, junction, no-connect flag or label, in its text: its head, where it is, and its
/// points, each with where its X Y are.
pub(super) struct Entry<'a> {
    pub(super) head: &'a str,
    pub(super) span: Range<usize>,
    pub(super) points: Vec<(Pt, Range<usize>)>,
}

/// The sheet's (`src` its text) wires, junctions, no-connect flags and labels.
pub(super) fn entries(src: &str) -> Result<Vec<Entry<'_>>> {
    let (c, mut out) = (&mut Cursor::new(src), vec![]);
    c.open("kicad_sch")?;
    while let Some(head) = c.child()? {
        let start = src[..c.span().start].rfind('(').unwrap_or(0);
        let label = matches!(head, "label" | "global_label" | "hierarchical_label");
        if label {
            c.atom()?;
        }
        if label || matches!(head, "wire" | "junction" | "no_connect") {
            let mut points = vec![];
            while let Some(child) = c.child()? {
                if child == "at" {
                    let a = read_at(c)?;
                    points.push((a.at, a.xy));
                } else if child == "pts" {
                    while let Some(point) = c.child()? {
                        if point == "xy" {
                            let a = read_at(c)?;
                            points.push((a.at, a.xy));
                        }
                        c.close()?;
                    }
                }
                c.close()?;
            }
            c.close()?;
            out.push(Entry { head, span: start..c.span().end, points });
            continue;
        }
        c.close()?;
    }
    Ok(out)
}

/// The text of the sheet's wire `i` (in `Schematic`'s order): the `(wire …)` holding it.
pub(super) fn wire<'a>(entries: &'a [Entry<'a>], i: usize) -> Option<&'a Entry<'a>> {
    let mut n = 0;
    entries.iter().filter(|e| e.head == "wire").find(|e| {
        n += e.points.len().saturating_sub(1);
        n > i
    })
}

/// The no-connect flag at `at`.
pub(super) fn flag<'a>(entries: &'a [Entry<'a>], at: Pt) -> Option<&'a Entry<'a>> {
    entries.iter().find(|e| e.head == "no_connect" && e.points.first().is_some_and(|p| p.0 == at))
}

/// Where a KiCad 6 root's `symbol_instances` lists the places `links`.
fn instances(root: &str, links: &[String]) -> Result<Vec<std::ops::Range<usize>>> {
    let (c, mut out) = (&mut Cursor::new(root), vec![]);
    c.open("kicad_sch")?;
    while let Some(head) = c.child()? {
        if head == "symbol_instances" {
            while let Some(head) = c.child()? {
                let start = root[..c.span().start].rfind('(').unwrap_or(0);
                let mine = head == "path" && c.atom().is_ok_and(|path| links.iter().any(|l| *l == unescape(path)));
                c.close()?;
                if mine {
                    out.push(start..c.span().end);
                }
            }
        }
        c.close()?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Nm;

    const LIB: &str = r#"(lib_symbols (symbol "Device:R" (property "Reference" "R" (at 2.032 0 90)) (property "Value" "R" (at 0 0 90))
        (symbol "R_1_1" (pin passive line (at 0 3.81 270) (length 1.27) (name "") (number "1"))
          (pin passive line (at 0 -3.81 90) (length 1.27) (name "") (number "2")))))"#;

    fn project(sheet: &str) -> Project {
        let sheet = sheet.to_string();
        Project::load("a.kicad_sch", "", |f| if f == "a.kicad_sch" { Ok(sheet.clone()) } else { Err(f.into()) }).unwrap()
    }

    fn at(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    fn text(p: &Project) -> String {
        p.text("a.kicad_sch").unwrap().to_string()
    }

    fn wired(t: &str, a: &str, b: &str) -> bool {
        t.contains(&format!("(xy {a}) (xy {b})"))
    }

    /// Resistors standing at (10.16, 10.16) and below it, R2's pin 1 on R1's pin 2: R1's pin 1
    /// wired right to a label `A`, R2's pin 2 flagged not connected.
    fn sheet(roots: &str) -> String {
        format!(
            "(kicad_sch (version 20211123) (uuid \"r\") {LIB}
  (symbol (lib_id \"Device:R\") (at 10.16 10.16 0) (unit 1) (uuid \"u1\")
    (property \"Reference\" \"R1\" (at 12.192 10.16 90)) (property \"Value\" \"10k\" (at 10.16 10.16 90)))
  (symbol (lib_id \"Device:R\") (at 10.16 17.78 0) (unit 1) (uuid \"u2\") (property \"Reference\" \"R2\" (at 12.192 17.78 90)))
  (wire (pts (xy 10.16 6.35) (xy 20.32 6.35)) (uuid \"w1\"))
  (label \"A\" (at 20.32 6.35 0) (uuid \"l\"))
  (no_connect (at 10.16 21.59) (uuid \"n\")){roots}
)"
        )
    }

    #[test]
    fn moves_and_turns_a_symbol_with_what_is_on_its_pins() {
        let src = sheet("");
        let mut p = project(&src);
        let nets = p.netlist();
        let pins = |p: &Project| p.pages[0].sheet.wiring.pins.iter().map(|q| q.at).collect::<Vec<_>>();
        let r1 = [Item::Symbol(0)];
        // A step and a half right and one down: taken to the grid, 2.54 mm each way. Its wire
        // keeps level, a new stretch bending down to its pin; R2's pin stays, a wire bent once
        // joining it to R1's.
        let (moved, now) = p.move_items(0, &r1, at(3.0, 2.0)).unwrap();
        let t = text(&p);
        assert!(now == r1 && t.contains("(at 12.7 12.7 0)") && t.contains("\"R1\" (at 14.732 12.7 90)"), "{t}");
        assert!(wired(&t, "12.7 6.35", "20.32 6.35") && wired(&t, "12.7 6.35", "12.7 8.89"), "{t}");
        assert!(wired(&t, "10.16 13.97", "12.7 13.97") && wired(&t, "12.7 13.97", "12.7 16.51"), "{t}");
        assert_eq!((&pins(&p)[..2], p.netlist()), (&[at(12.7, 8.89), at(12.7, 16.51)][..], nets.clone()));
        // A quarter turn: lying, pin 1 left; its reference above it, still upright in its frame.
        let (turned, _) = p.turn_items(0, &r1).unwrap();
        let t = text(&p);
        assert!(t.contains("(at 12.7 12.7 90)") && t.contains("\"R1\" (at 12.7 10.668 90)"), "{t}");
        assert!(wired(&t, "12.7 6.35", "8.89 6.35") && wired(&t, "8.89 6.35", "8.89 12.7"), "{t}");
        assert!(wired(&t, "12.7 13.97", "16.51 13.97") && wired(&t, "16.51 13.97", "16.51 12.7"), "{t}");
        assert_eq!((&pins(&p)[..2], p.netlist()), (&[at(8.89, 12.7), at(16.51, 12.7)][..], nets));
        // A wire with a label along it keeps its part from the label on; a new stretch reaches
        // the pin from there.
        let along = sheet("").replace("(label \"A\" (at 20.32 6.35 0)", "(label \"A\" (at 15.24 6.35 0)");
        let mut q = project(&along);
        let nets = q.netlist();
        q.move_items(0, &r1, at(2.54, 2.54)).unwrap();
        let t = text(&q);
        assert!(
            wired(&t, "15.24 6.35", "20.32 6.35") && wired(&t, "15.24 6.35", "12.7 6.35") && wired(&t, "12.7 6.35", "12.7 8.89"),
            "{t}"
        );
        assert_eq!(q.netlist(), nets);
        // Mirrored (pin 1 below), it turns counterclockwise all the same, its angle a quarter less.
        let mirrored = sheet("").replace("(at 10.16 10.16 0) (unit 1)", "(at 10.16 10.16 0) (mirror x) (unit 1)");
        let mut q = project(&mirrored);
        let nets = q.netlist();
        q.turn_items(0, &r1).unwrap();
        assert!(text(&q).contains("(at 10.16 10.16 270) (mirror x)"));
        assert_eq!((&pins(&q)[..2], q.netlist()), (&[at(13.97, 10.16), at(6.35, 10.16)][..], nets));
        // A label on a pin alone goes with it, as does a no-connect flag.
        let label = sheet("").replace(
            "(wire (pts (xy 10.16 6.35) (xy 20.32 6.35)) (uuid \"w1\"))\n  (label \"A\" (at 20.32 6.35 0)",
            "(label \"A\" (at 10.16 6.35 0)",
        );
        let mut q = project(&label);
        let nets = q.netlist();
        q.move_items(0, &[Item::Symbol(1)], at(2.54, 0.0)).unwrap();
        // R1's pin 2 lands where the wire to R2's went: that wire goes.
        q.move_items(0, &r1, at(2.54, 0.0)).unwrap();
        let t = text(&q);
        assert!(t.contains("(label \"A\" (at 12.7 6.35 0)") && t.contains("(no_connect (at 12.7 21.59)"), "{t}");
        assert!(!t.contains("(wire") && q.netlist() == nets, "{t}");
        p.apply(turned);
        p.apply(moved);
        assert_eq!(text(&p), src);
    }

    #[test]
    fn mirrors_symbols_labels_and_wires_with_what_joins_them() {
        let src = sheet("");
        let mut p = project(&src);
        let nets = p.netlist();
        let pins = |p: &Project| p.pages[0].sheet.wiring.pins.iter().map(|q| q.at).collect::<Vec<_>>();
        let (r1, before) = ([Item::Symbol(0)], pins(&p));
        // Left to right about its origin: KiCad's (mirror y), its reference to its other side; its
        // pins, on its axis, stay. Again, and the sheet is as it was, byte for byte.
        p.mirror_items(0, &r1, false).unwrap();
        let t = text(&p);
        assert!(t.contains("(at 10.16 10.16 0) (mirror y) (unit 1)") && t.contains("\"R1\" (at 8.128 10.16 90)"), "{t}");
        assert_eq!((pins(&p), p.netlist()), (before.clone(), nets.clone()));
        p.mirror_items(0, &r1, false).unwrap();
        assert_eq!(text(&p), src);
        // Top to bottom, its pins change places, in place as KiCad mirrors: each takes what was
        // wired where it lands (the wire to A now on pin 2, R2 on pin 1), and no wire moves.
        let (flipped, _) = p.mirror_items(0, &r1, true).unwrap();
        let t = text(&p);
        assert!(t.contains("(at 10.16 10.16 0) (mirror x) (unit 1)") && wired(&t, "10.16 6.35", "20.32 6.35"), "{t}");
        let swapped = p.netlist();
        let on =
            |net: &str| swapped.iter().find(|n| n.0 == net).map(|n| n.1.iter().map(|(r, pin)| format!("{r}:{pin}")).collect::<Vec<_>>());
        assert_eq!((&pins(&p)[..2], on("/A"), swapped.len()), (&[before[1], before[0]][..], Some(vec!["R1:2".to_string()]), 3));
        // Mirrored the other way too, it is half a turn round with no mirror, as KiCad writes it.
        p.mirror_items(0, &r1, false).unwrap();
        assert!(text(&p).contains("(at 10.16 10.16 180) (unit 1)"), "{}", text(&p));
        assert_eq!((&pins(&p)[..2], p.netlist()), (&[before[1], before[0]][..], swapped));
        // A label on a pin that goes to its symbol's other side goes with it, and reads away from
        // it still: a test point's, left of it, is right of it once mirrored left to right.
        let point =
            "(kicad_sch (version 20211123) (uuid \"r\") (lib_symbols (symbol \"Device:TP\" (property \"Reference\" \"TP\" (at 0 2.54 0))
            (symbol \"TP_1_1\" (pin passive line (at -2.54 0 0) (length 2.54) (name \"\") (number \"1\")))))
  (symbol (lib_id \"Device:TP\") (at 10.16 10.16 0) (unit 1) (uuid \"t\") (property \"Reference\" \"TP1\" (at 10.16 7.62 0)))
  (label \"A\" (at 7.62 10.16 180) (uuid \"l\"))
)";
        let mut q = project(point);
        let named = q.netlist();
        q.mirror_items(0, &r1, false).unwrap();
        assert!(text(&q).contains("(label \"A\" (at 12.7 10.16 0)") && q.netlist() == named, "{}", text(&q));
        // A label alone reads the other way when mirrored along its line, and as it did across it.
        let mut q = project(&src);
        let label = [Item::Label(0)];
        q.mirror_items(0, &label, true).unwrap();
        assert_eq!(text(&q), src);
        q.mirror_items(0, &label, false).unwrap();
        assert!(text(&q).contains("(label \"A\" (at 20.32 6.35 180)") && q.netlist() == nets, "{}", text(&q));
        // Several together, about their middle on the grid: the wire and its label go to the
        // other side of R1, which R2 stays wired to.
        let mut q = project(&src);
        let all = [Item::Symbol(0), Item::Wire(0), Item::Label(0)];
        let (undo, now) = q.mirror_items(0, &all, false).unwrap();
        let (t, w) = (text(&q), &q.pages[0].sheet.wiring);
        assert_eq!((now.len(), q.netlist()), (3, nets));
        assert!(w.labels[0].at.x < w.pins[0].at.x && t.contains("(mirror y)"), "{t}");
        q.apply(undo);
        assert_eq!(text(&q), src);
        p.apply(flipped);
    }

    #[test]
    fn deletes_a_part_and_a_kicad6_roots_places_of_it() {
        let roots = "\n  (symbol_instances (path \"/u1\" (reference \"R1\") (unit 1) (value \"10k\") (footprint \"\")))";
        let src = sheet(roots);
        let mut p = project(&src);
        let undo = p.delete_items(0, &[Item::Symbol(0)]).unwrap();
        let r1 = &src[src.find("\n  (symbol (lib_id").unwrap()..src.find("\n  (symbol (lib_id \"Device:R\") (at 10.16 17.78").unwrap()];
        assert_eq!(text(&p), src.replace(r1, "").replace(roots, "\n  (symbol_instances)"));
        assert_eq!(p.pages[0].sheet.symbols.iter().map(|s| s.designator.as_str()).collect::<Vec<_>>(), ["R2"]);
        p.apply(undo);
        assert_eq!(text(&p), src);
        assert!(p.delete_items(0, &[Item::Symbol(9)]).unwrap_err().contains("no Symbol(9)"));
        // A part picked by two of its units with another between them goes once.
        let three = sheet("").replace(
            "\n  (wire",
            "\n  (symbol (lib_id \"Device:R\") (at 30 30 0) (unit 1) (uuid \"u3\") (property \"Reference\" \"R1\" (at 0 0 0)))\n  (wire",
        );
        let mut p = project(&three);
        let undo = p.delete_items(0, &[Item::Symbol(2), Item::Symbol(0), Item::Symbol(1)]).unwrap();
        assert!(p.pages[0].sheet.symbols.is_empty() && text(&p).contains("(wire"), "{}", text(&p));
        p.apply(undo);
        assert_eq!(text(&p), three);
    }

    #[test]
    fn picks_moves_and_deletes_a_no_connect_flag() {
        let src = sheet("");
        let mut p = project(&src);
        // Its cross is picked before R2, whose pin it is on.
        let flag = p.pages[0].sheet.item_at(at(10.5, 21.2), 0);
        assert_eq!(flag, Some(Item::NoConnect(0)));
        // Moved alone: R2 and its pin stay.
        let (undo, items) = p.move_items(0, &[Item::NoConnect(0)], at(2.54, 0.0)).unwrap();
        assert!(text(&p).contains("(no_connect (at 12.7 21.59)") && !text(&p).contains("(wire (pts (xy 10.16 21.59)"), "{}", text(&p));
        assert_eq!((items, p.pages[0].sheet.symbols[1].bounds), (vec![Item::NoConnect(0)], project(&src).pages[0].sheet.symbols[1].bounds));
        p.apply(undo);
        let undo = p.delete_items(0, &[Item::NoConnect(0)]).unwrap();
        assert_eq!(text(&p), src.replace("\n  (no_connect (at 10.16 21.59) (uuid \"n\"))", ""));
        p.apply(undo);
        assert_eq!(text(&p), src);
    }

    /// R1 standing at (10.16, 10.16), its pin 1 wired right (`a`) to a corner, down from there
    /// (`b`) past a T to the right (`c`, a junction, the global label `G` at its end) and the
    /// label `M`; a wire `d` apart, the label `D` at its end.
    fn wiring() -> String {
        let label = |name: &str, x: f64, y: f64| {
            format!("(label \"{name}\" (at {x} {y} 0) (effects (font (size 1.27 1.27)) (justify left bottom)) (uuid \"{name}\"))")
        };
        format!(
            "(kicad_sch (version 20250114) (uuid \"r\") {LIB}
  (symbol (lib_id \"Device:R\") (at 10.16 10.16 0) (unit 1) (uuid \"u1\") (property \"Reference\" \"R1\" (at 12.192 10.16 90)))
  (wire (pts (xy 10.16 6.35) (xy 20.32 6.35)) (uuid \"a\"))
  (wire (pts (xy 20.32 6.35) (xy 20.32 16.51)) (uuid \"b\"))
  (wire (pts (xy 20.32 11.43) (xy 30.48 11.43)) (uuid \"c\"))
  (junction (at 20.32 11.43) (diameter 0) (color 0 0 0 0) (uuid \"j\"))
  {}
  (global_label \"G\" (shape input) (at 30.48 11.43 0) (effects (font (size 1.27 1.27)) (justify left)) (uuid \"g\")
    (property \"Intersheetrefs\" \"${{INTERSHEET_REFS}}\" (at 36.83 11.43 0) (effects (font (size 1.27 1.27)) (justify left))))
  (wire (pts (xy 40.64 10.16) (xy 50.8 10.16)) (uuid \"d\"))
  {}
)",
            label("M", 20.32, 13.97),
            label("D", 50.8, 10.16)
        )
    }

    #[test]
    fn moves_a_wire_with_what_joins_it() {
        let src = wiring();
        let mut p = project(&src);
        let nets = p.netlist();
        // `b` a step right: `a` stretches to it, `c` shortens, the T's junction and the label
        // along it go with it.
        let (undo, now) = p.move_items(0, &[Item::Wire(1)], at(2.54, 0.0)).unwrap();
        let t = text(&p);
        assert!(wired(&t, "10.16 6.35", "22.86 6.35") && wired(&t, "22.86 6.35", "22.86 16.51") && wired(&t, "22.86 11.43", "30.48 11.43"));
        assert!(t.contains("(junction (at 22.86 11.43)") && t.contains("(label \"M\" (at 22.86 13.97 0)"), "{t}");
        assert_eq!((now, p.netlist()), (vec![Item::Wire(1)], nets.clone()));
        p.apply(undo);
        assert_eq!(text(&p), src);
        // Along itself, down a step: `a` stays and a stretch reaches it, `c` bends to follow the T.
        p.move_items(0, &[Item::Wire(1)], at(0.0, 2.54)).unwrap();
        let t = text(&p);
        assert!(wired(&t, "10.16 6.35", "20.32 6.35") && wired(&t, "20.32 6.35", "20.32 8.89"), "{t}");
        assert!(t.contains("(junction (at 20.32 13.97)") && p.netlist() == nets, "{t}");
        // `c` up a step: its T slides along `b`, which joins the old place to the new: a junction
        // there, none left where it was.
        let mut p = project(&src);
        p.move_items(0, &[Item::Wire(2)], at(0.0, -1.27)).unwrap();
        let t = text(&p);
        assert!(
            wired(&t, "20.32 10.16", "30.48 10.16") && t.contains("(at 30.48 10.16 0)") && t.contains("(junction\n\t\t(at 20.32 10.16)")
        );
        assert!(!t.contains("(junction (at") && !t.contains("(xy 20.32 11.43)") && p.netlist() == nets, "{t}");
        // `d` turned about its middle, its label with it; then moved so its end lands on `a`'s
        // middle, which joins them: a junction goes there, as KiCad's editor puts one.
        let mut p = project(&src);
        let (_, now) = p.turn_items(0, &[Item::Wire(3)]).unwrap();
        let t = text(&p);
        assert!(wired(&t, "45.72 15.24", "45.72 5.08") && t.contains("(label \"D\" (at 45.72 5.08 0)"), "{t}");
        p.move_items(0, &now, at(-30.48, 1.27)).unwrap();
        let t = text(&p);
        assert!(wired(&t, "15.24 16.51", "15.24 6.35") && t.contains("(junction\n\t\t(at 15.24 6.35)"), "{t}");
    }

    #[test]
    fn deletes_wires_and_the_junctions_they_leave() {
        let src = wiring();
        let mut p = project(&src);
        // `c` goes, and the T's junction with it: `b` passes there alone.
        let undo = p.delete_items(0, &[Item::Wire(2)]).unwrap();
        let (c, j) = (
            "\n  (wire (pts (xy 20.32 11.43) (xy 30.48 11.43)) (uuid \"c\"))",
            "\n  (junction (at 20.32 11.43) (diameter 0) (color 0 0 0 0) (uuid \"j\"))",
        );
        assert_eq!(text(&p), src.replace(c, "").replace(j, ""));
        p.apply(undo);
        assert_eq!(text(&p), src);
        // `b` goes: the junction joins `c`'s end alone, and goes too; `M` stays, naming nothing.
        p.delete_items(0, &[Item::Wire(1)]).unwrap();
        let t = text(&p);
        assert!(!t.contains("(uuid \"b\")") && !t.contains("(junction") && t.contains("(label \"M\""), "{t}");
        assert!(!p.pages[0].sheet.names(0));
    }

    #[test]
    fn moves_turns_and_deletes_labels() {
        let src = wiring();
        let mut p = project(&src);
        let m = [Item::Label(0)];
        assert!(p.pages[0].sheet.names(0));
        // Onto R1's pin 2: it names that pin now, and `b` nothing.
        p.move_items(0, &m, at(-10.16, 0.0)).unwrap();
        assert!(text(&p).contains("(label \"M\" (at 10.16 13.97 0)") && p.pages[0].sheet.names(0));
        // Off everything: it names nothing.
        p.move_items(0, &m, at(0.0, 12.7)).unwrap();
        assert!(!p.pages[0].sheet.names(0));
        // Turned twice: it reads up from its point, then left towards it.
        let mut p = project(&src);
        p.turn_items(0, &m).unwrap();
        assert!(text(&p).contains("(label \"M\" (at 20.32 13.97 90) (effects (font (size 1.27 1.27)) (justify left bottom))"));
        p.turn_items(0, &m).unwrap();
        assert!(text(&p).contains("(label \"M\" (at 20.32 13.97 180) (effects (font (size 1.27 1.27)) (justify right bottom))"));
        // A global label's references go with it.
        let g = [Item::Label(1)];
        let (undo, _) = p.move_items(0, &g, at(2.54, 0.0)).unwrap();
        let t = text(&p);
        assert!(t.contains("(at 33.02 11.43 0)") && t.contains("(at 39.37 11.43 0)"), "{t}");
        p.apply(undo);
        // Deleted, undone byte for byte.
        let undo = p.delete_items(0, &g).unwrap();
        assert!(!text(&p).contains("global_label") && p.pages[0].sheet.describe(Item::Label(1)) == "Label D");
        p.apply(undo);
        assert!(text(&p).contains("global_label") && p.pages[0].sheet.describe(Item::Label(1)) == "Global label G");
    }
}
