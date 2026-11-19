//! A project's nets: what its sheets' wires, labels and pins join, through sheet pins, buses and
//! global names, as KiCad 10 joins them (`crates/core/nets.md`; checked against its netlist
//! export in `tests/corpus.rs`).

use super::{Item, Project, Schematic};
use crate::board::{Nm, Pt};
use crate::tr::t;
use std::collections::HashMap;

/// A pin's electrical type, in KiCad's order: the rows and columns of its pin conflict map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PinKind {
    Input,
    Output,
    Bidirectional,
    TriState,
    Passive,
    Free,
    Unspecified,
    PowerIn,
    PowerOut,
    OpenCollector,
    OpenEmitter,
    NoConnect,
}

/// Each kind by the name KiCad's files give it.
const KINDS: [(&str, PinKind); 12] = [
    ("input", PinKind::Input),
    ("output", PinKind::Output),
    ("bidirectional", PinKind::Bidirectional),
    ("tri_state", PinKind::TriState),
    ("passive", PinKind::Passive),
    ("free", PinKind::Free),
    ("unspecified", PinKind::Unspecified),
    ("power_in", PinKind::PowerIn),
    ("power_out", PinKind::PowerOut),
    ("open_collector", PinKind::OpenCollector),
    ("open_emitter", PinKind::OpenEmitter),
    ("no_connect", PinKind::NoConnect),
];

impl PinKind {
    pub(super) fn parse(s: &str) -> PinKind {
        KINDS.iter().find(|k| k.0 == s).map_or(PinKind::Unspecified, |k| k.1)
    }

    /// Its name in KiCad's files.
    fn name(self) -> &'static str {
        KINDS.iter().find(|k| k.1 == self).map_or("unspecified", |k| k.0)
    }
}

/// A shown pin of a placed symbol, as agents wire it: where a wire joins it, and the way it faces
/// from its symbol's body (`left`, `right`, `up`, `down`: where a wire leaves it).
#[derive(Debug, PartialEq)]
pub struct PinEnd {
    pub number: String,
    pub name: String,
    /// KiCad's electrical type: `input`, `power_in`, `passive`…
    pub kind: &'static str,
    pub at: Pt,
    pub facing: &'static str,
}

/// Where a label's name joins: its sheet's names (`Local`), every sheet's (`Global`), or its
/// sheet's and, through the sheet pin of that name, the sheet above's (`Hier`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    Local,
    Global,
    Hier,
}

impl LabelKind {
    /// What a label of this kind is called: `Label`, `Global label`, `Hierarchical label`.
    pub fn name(self) -> &'static str {
        match self {
            LabelKind::Local => t!("Label"),
            LabelKind::Global => t!("Global label"),
            LabelKind::Hier => t!("Hierarchical label"),
        }
    }
}

/// A net label, where it joins a wire or bus; its name as written (KiCad's escapes kept), a
/// global or hierarchical one's shape (`input`…); where its list is in the sheet's text and the
/// box it is drawn in.
#[derive(Clone, Debug)]
pub(super) struct Label {
    pub(super) name: String,
    pub(super) at: Pt,
    pub(super) kind: LabelKind,
    pub(super) shape: String,
    pub(super) span: std::ops::Range<usize>,
    pub(super) bounds: (Pt, Pt),
}

/// A net where it is on one page (`Project::net_items`): its wires, labels and power symbols
/// there, and the points of its pins and sheet pins (not hidden pins, which show nothing).
#[derive(Clone, Debug, PartialEq)]
pub struct NetOnPage {
    pub page: usize,
    pub items: Vec<Item>,
    pub pins: Vec<Pt>,
}

/// A placed symbol's pin, where a wire meets it.
#[derive(Clone, Debug)]
pub(super) struct Pin {
    pub(super) at: Pt,
    /// A step from `at` away from its symbol's body.
    pub(super) out: (Nm, Nm),
    /// Its symbol: an index into `Schematic::symbols`.
    pub(super) symbol: usize,
    pub(super) number: String,
    /// Its name, or the alternate function its symbol picks.
    pub(super) name: String,
    /// Its name in the library, whichever alternate it shows: KiCad names nets after it.
    pub(super) base: String,
    pub(super) kind: PinKind,
    pub(super) hidden: bool,
}

/// What a sheet's items connect, in sheet coordinates.
#[derive(Clone, Debug, Default)]
pub(super) struct Wiring {
    pub(super) wires: Vec<(Pt, Pt)>,
    pub(super) buses: Vec<(Pt, Pt)>,
    /// Bus entries' two ends, the one on a bus either way round.
    pub(super) entries: Vec<(Pt, Pt)>,
    pub(super) junctions: Vec<Pt>,
    pub(super) no_connects: Vec<Pt>,
    pub(super) labels: Vec<Label>,
    pub(super) pins: Vec<Pin>,
}

/// A sheet's connection points, numbered, joined into groups as KiCad joins them: ends meet the
/// ends and pins at their point; labels, junctions and bus entries join a wire or bus anywhere
/// along it. Points on buses are apart from points on wires: only bus entries reach both.
pub(super) struct Joined {
    /// Each point's place, and its group: the smallest point in it.
    pub(super) at: Vec<Pt>,
    pub(super) group: Vec<u32>,
    /// How many items have their connection point at each point.
    pub(super) hits: Vec<u32>,
    /// The points of each wire's ends, and of each bus entry's wire and bus end.
    pub(super) wires: Vec<[u32; 2]>,
    pub(super) entries: Vec<[u32; 2]>,
    pub(super) no_connects: Vec<u32>,
    /// Each label's point, and whether it names a bus.
    pub(super) labels: Vec<(u32, bool)>,
    /// Each pin's point; none for a no-connect pin, which joins nothing.
    pub(super) pins: Vec<Option<u32>>,
    /// Each sheet's pins' points, and whether each is on a bus.
    pub(super) sheet_pins: Vec<Vec<(u32, bool)>>,
}

fn find(up: &mut [u32], mut i: u32) -> u32 {
    while up[i as usize] != i {
        let next = up[up[i as usize] as usize];
        up[i as usize] = next;
        i = next;
    }
    i
}

fn join(up: &mut [u32], a: u32, b: u32) {
    let (a, b) = (find(up, a), find(up, b));
    up[a.max(b) as usize] = a.min(b);
}

/// Whether `p` lies on the segment from `a` to `b`.
pub(super) fn on(p: Pt, a: Pt, b: Pt) -> bool {
    let cross = (b.x - a.x) as i64 * (p.y - a.y) as i64 - (b.y - a.y) as i64 * (p.x - a.x) as i64;
    cross == 0 && (a.x.min(b.x)..=a.x.max(b.x)).contains(&p.x) && (a.y.min(b.y)..=a.y.max(b.y)).contains(&p.y)
}

impl Schematic {
    /// The shown pins of symbol `i` (an index into `symbols`).
    pub fn pin_ends(&self, i: usize) -> Vec<PinEnd> {
        let facing = |(x, y): (Nm, Nm)| match (x.signum(), y.signum()) {
            (-1, _) => "left",
            (1, _) => "right",
            (_, -1) => "up",
            _ => "down",
        };
        let pins = self.wiring.pins.iter().filter(|q| q.symbol == i && !q.hidden);
        pins.map(|q| PinEnd { number: q.number.clone(), name: q.name.clone(), kind: q.kind.name(), at: q.at, facing: facing(q.out) })
            .collect()
    }

    /// Whether a wire ending at `p` joins something there: a pin, a wire, a bus entry, a label
    /// or a sheet pin.
    pub fn joins(&self, p: Pt) -> bool {
        self.wired(p, false) || self.wiring.labels.iter().any(|l| l.at == p)
    }

    /// Whether net label `i` (in the sheet's order) names anything: a pin at its point, a wire,
    /// bus or bus entry along it, a sheet pin.
    pub fn names(&self, i: usize) -> bool {
        self.wired(self.wiring.labels[i].at, true)
    }

    /// Whether a pin or sheet pin is at `p`, or a wire, bus entry or (`buses`) bus along it.
    fn wired(&self, p: Pt, buses: bool) -> bool {
        let w = &self.wiring;
        let lines = w.wires.iter().chain(&w.entries).chain(if buses { &w.buses[..] } else { &[] });
        w.pins.iter().any(|q| q.at == p && !q.hidden)
            || lines.into_iter().any(|&(a, b)| on(p, a, b))
            || self.sheets.iter().any(|s| s.pins.iter().any(|q| q.1 == p))
    }
}

fn joined(s: &Schematic) -> Joined {
    let w = &s.wiring;
    let (mut ids, mut at, mut up, mut hits) = (HashMap::new(), vec![], vec![], vec![]);
    let mut point = |p: Pt, bus: bool| {
        let id = *ids.entry((p, bus)).or_insert_with(|| {
            at.push(p);
            hits.push(0);
            up.push(up.len() as u32);
            up.len() as u32 - 1
        });
        hits[id as usize] += 1;
        id
    };
    let on_bus = |p: Pt| w.buses.iter().any(|&(a, b)| on(p, a, b));
    let wires: Vec<[u32; 2]> = w.wires.iter().map(|&(a, b)| [point(a, false), point(b, false)]).collect();
    let buses: Vec<[u32; 2]> = w.buses.iter().map(|&(a, b)| [point(a, true), point(b, true)]).collect();
    // Points that join a wire or bus anywhere along it: whether on a bus, where, which.
    let mut along = vec![];
    let entries = (w.entries.iter())
        .map(|&(a, b)| {
            let (bus_end, wire_end) = if on_bus(a) || !on_bus(b) { (a, b) } else { (b, a) };
            let bus = point(bus_end, true);
            along.push((true, bus_end, bus));
            [point(wire_end, false), bus]
        })
        .collect();
    for &p in &w.junctions {
        let bus = on_bus(p);
        along.push((bus, p, point(p, bus)));
    }
    let labels = (w.labels.iter())
        .map(|l| {
            let bus = on_bus(l.at);
            let id = point(l.at, bus);
            along.push((bus, l.at, id));
            (id, bus)
        })
        .collect();
    let pins = w.pins.iter().map(|p| (p.kind != PinKind::NoConnect).then(|| point(p.at, false))).collect();
    let no_connects = w.no_connects.iter().map(|&p| point(p, false)).collect();
    let sheet_pins = (s.sheets.iter())
        .map(|sh| {
            let on = |&(_, p): &(String, Pt)| {
                let bus = on_bus(p);
                (point(p, bus), bus)
            };
            sh.pins.iter().map(on).collect()
        })
        .collect();
    for ends in wires.iter().chain(&buses) {
        join(&mut up, ends[0], ends[1]);
    }
    along.sort_unstable_by_key(|&(bus, p, _)| (bus, p.x, p.y));
    for (bus, segments, ends) in [(false, &w.wires, &wires), (true, &w.buses, &buses)] {
        for (&(a, b), end) in segments.iter().zip(ends) {
            let first = along.partition_point(|&(l, p, _)| (l, p.x) < (bus, a.x.min(b.x)));
            for &(_, p, id) in along[first..].iter().take_while(|&&(l, p, _)| l == bus && p.x <= a.x.max(b.x)) {
                if on(p, a, b) {
                    join(&mut up, end[0], id);
                }
            }
        }
    }
    let group = (0..up.len() as u32).map(|i| find(&mut up, i)).collect();
    Joined { at, group, hits, wires, entries, no_connects, labels, pins, sheet_pins }
}

/// The scope of the names every sheet shares (global labels, power), beside each page's.
const GLOBAL: usize = usize::MAX;

/// A project's nets: each page's points joined across its sheets, buses and global names.
pub(super) struct Nets {
    /// Each page's points, joined on its sheet.
    pub(super) pages: Vec<Joined>,
    /// Where each page's points start in `net`.
    base: Vec<usize>,
    net: Vec<u32>,
    /// Each net's name as KiCad's netlist has it (`/Sheet/LABEL`, `GLOBAL`); empty when no label
    /// names it.
    pub(super) names: Vec<String>,
}

impl Nets {
    /// The net of a page's point.
    pub(super) fn of(&self, page: usize, point: u32) -> usize {
        self.net[self.base[page] + point as usize] as usize
    }
}

/// What a name on a net takes precedence by, as KiCad picks a net's name: its kind (global
/// label, power symbol, hidden power pin, local label, hierarchical label, sheet pin), then the
/// sheet highest up, then the alphabet.
type Rank = (u8, usize, String);

/// A name on a bus: by a point on it, its rank, the scope of its members' names, the name as
/// written and its page.
type BusName<'a> = (u32, Rank, usize, &'a str, usize);

/// A name a sheet gives the net at one of its points: its rank's kind, the name as written,
/// whether every sheet shares it (else only its sheet) and whether it names a bus.
pub(super) struct Name<'a> {
    pub(super) point: u32,
    pub(super) rank: u8,
    pub(super) name: &'a str,
    pub(super) global: bool,
    pub(super) bus: bool,
}

/// The names on a sheet's nets: its labels', its power symbols' values and its parts' hidden
/// power inputs' (KiCad joins those to the global net of their name).
pub(super) fn names<'a>(s: &'a Schematic, j: &Joined) -> Vec<Name<'a>> {
    let labels = s.wiring.labels.iter().zip(&j.labels).map(|(l, &(point, bus))| {
        let rank = match l.kind {
            LabelKind::Global => 0,
            LabelKind::Local => 3,
            LabelKind::Hier => 4,
        };
        Name { point, rank, name: &l.name, global: l.kind == LabelKind::Global, bus }
    });
    let pins = s.wiring.pins.iter().zip(&j.pins).filter_map(|(p, &point)| {
        let (rank, name) = match &s.symbols[p.symbol].power {
            // A power symbol's input names its net by its value; a flag's output doesn't.
            Some((kind, value)) if p.kind == PinKind::PowerIn => (if *kind == LabelKind::Global { 1 } else { 3 }, value.as_str()),
            None if p.hidden && p.kind == PinKind::PowerIn => (2, p.name.as_str()),
            _ => return None,
        };
        Some(Name { point: point?, rank, name, global: rank < 3, bus: false })
    });
    labels.chain(pins).collect()
}

impl Project {
    /// Its nets: each page's points joined into the nets KiCad's netlist has.
    pub(super) fn nets(&self) -> Nets {
        let pages: Vec<Joined> = self.pages.iter().map(|p| joined(&p.sheet)).collect();
        let base: Vec<usize> = pages.iter().scan(0, |n, j| Some(std::mem::replace(n, *n + j.group.len()))).collect();
        // Each page's points, in its sheet's groups.
        let mut up: Vec<u32> = pages.iter().zip(&base).flat_map(|(j, &b)| j.group.iter().map(move |&g| b as u32 + g)).collect();
        let points = up.len() as u32;
        let mut keys: HashMap<(usize, String), u32> = HashMap::new();
        let mut key = |up: &mut Vec<u32>, scope: usize, name: &str| {
            *keys.entry((scope, name.to_string())).or_insert_with(|| {
                up.push(up.len() as u32);
                up.len() as u32 - 1
            })
        };
        // Names that could name each net, by a point on it, and the names on buses.
        let (mut candidates, mut buses): (Vec<(u32, Rank)>, Vec<BusName>) = (vec![], vec![]);
        for (i, page) in self.pages.iter().enumerate() {
            let (j, s, depth) = (&pages[i], &page.sheet, page.path.len());
            let at = |point: u32| (base[i] + point as usize) as u32;
            let local = |name: &str| format!("/{}", page.path.iter().map(|p| format!("{}/", step(p))).collect::<String>()) + name;
            for n in names(s, j) {
                let (g, k) = (at(n.point), key(&mut up, i, n.name));
                join(&mut up, g, k);
                if n.global {
                    let k = key(&mut up, GLOBAL, n.name);
                    join(&mut up, g, k);
                }
                let rank = (n.rank, depth, if n.global { n.name.to_string() } else { local(n.name) });
                if n.bus {
                    buses.push((g, rank, if n.global { GLOBAL } else { i }, n.name, i));
                } else {
                    candidates.push((g, rank));
                }
            }
            for ((sheet, points), child) in s.sheets.iter().zip(&j.sheet_pins).zip(&page.children) {
                let Some(child) = *child else { continue };
                // A sheet pin joins the hierarchical label of its name in its sheet; without one,
                // nothing there, and it names its net here.
                let labels = &self.pages[child].sheet.wiring.labels;
                for ((name, _), &(point, bus)) in sheet.pins.iter().zip(points) {
                    let g = at(point);
                    if !labels.iter().any(|l| l.kind == LabelKind::Hier && l.name == *name) {
                        candidates.extend((!bus).then(|| (g, (5, depth, local(name)))));
                        continue;
                    }
                    let k = key(&mut up, child, name);
                    join(&mut up, g, k);
                    let rank = (5, depth, format!("{}{name}", local(&format!("{}/", step(&sheet.name)))));
                    if bus {
                        buses.push((g, rank, child, name, i));
                    } else {
                        candidates.push((g, rank));
                    }
                }
            }
        }
        // A bus's members join the same members of each other name on it (KiCad picks the first
        // as it ranks net names); a global bus label's also join its sheet's names.
        let mut on_bus: HashMap<u32, Vec<(Rank, usize, &str)>> = HashMap::new();
        for (g, rank, scope, name, page) in buses {
            if scope == GLOBAL {
                for (_, m) in members(name, &self.aliases) {
                    let (a, b) = (key(&mut up, GLOBAL, &m), key(&mut up, page, &m));
                    join(&mut up, a, b);
                }
            }
            on_bus.entry(find(&mut up, g)).or_default().push((rank, scope, name));
        }
        for mut bus in on_bus.into_values() {
            bus.sort_unstable();
            let ((_, scope, driver), rest) = bus.split_first().expect("a bus has a name");
            let ours = members(driver, &self.aliases);
            for &(_, other, name) in rest {
                for (a, b) in matched(driver, &ours, name, &members(name, &self.aliases)) {
                    let (a, b) = (key(&mut up, *scope, a), key(&mut up, other, b));
                    join(&mut up, a, b);
                }
            }
        }
        let (mut net, mut count) = (vec![u32::MAX; up.len()], 0);
        let net_of: Vec<u32> = (0..points)
            .map(|p| {
                let root = find(&mut up, p) as usize;
                if net[root] == u32::MAX {
                    net[root] = count;
                    count += 1;
                }
                net[root]
            })
            .collect();
        let mut best: Vec<Option<Rank>> = vec![None; count as usize];
        for (g, rank) in candidates {
            let n = &mut best[net_of[g as usize] as usize];
            if n.as_ref().is_none_or(|b| rank < *b) {
                *n = Some(rank);
            }
        }
        let names = best.into_iter().map(|b| b.map(|r| r.2).unwrap_or_default()).collect();
        Nets { pages, base, net: net_of, names }
    }

    /// Its nets as KiCad's netlist export lists them: each net's name and the parts' pins on it,
    /// by reference and number; power symbols left out. A net nothing names gets the name KiCad
    /// makes from a pin on it (`Net-(R1-Pad2)`, `unconnected-(U1-NC-Pad5)`).
    pub fn netlist(&self) -> Vec<(String, Vec<(String, String)>)> {
        let mut out = self.named(&self.nets());
        out.retain(|(_, nodes)| !nodes.is_empty());
        out
    }

    /// The name of the net `item` of page `page` is on, as `netlist` names it: a wire's or a
    /// label's; none for a symbol, a bus, or a net that has neither a name nor a part's pin.
    pub fn net_of(&self, page: usize, item: Item) -> Option<String> {
        let nets = self.nets();
        let j = nets.pages.get(page)?;
        let point = match item {
            Item::Wire(i) => j.wires.get(i)?[0],
            Item::Label(i) if !j.labels.get(i)?.1 => j.labels[i].0,
            _ => return None,
        };
        Some(self.named(&nets).swap_remove(nets.of(page, point)).0).filter(|n| !n.is_empty())
    }

    /// Where net `name` (as `netlist` names it) is on the sheets: on each page it reaches.
    pub fn net_items(&self, name: &str) -> Vec<NetOnPage> {
        let nets = self.nets();
        let Some(n) = self.named(&nets).iter().position(|(m, _)| m == name) else { return vec![] };
        let mut out = vec![];
        for (i, (j, page)) in nets.pages.iter().zip(&self.pages).enumerate() {
            let (s, on) = (&page.sheet, |p: &u32| nets.of(i, *p) == n);
            let pins: Vec<&Pin> = s.wiring.pins.iter().zip(&j.pins).filter(|(_, p)| p.as_ref().is_some_and(on)).map(|q| q.0).collect();
            let wires = (0..j.wires.len()).filter(|&k| on(&j.wires[k][0])).map(Item::Wire);
            let labels = (0..j.labels.len()).filter(|&k| !j.labels[k].1 && on(&j.labels[k].0)).map(Item::Label);
            let power = pins.iter().filter(|q| s.symbols[q.symbol].power.is_some()).map(|q| Item::Symbol(q.symbol));
            let items: Vec<Item> = wires.chain(labels).chain(power).collect();
            let sheet_pins = s.sheets.iter().zip(&j.sheet_pins).flat_map(|(sh, at)| sh.pins.iter().zip(at));
            let sheet_pins = sheet_pins.filter(|(_, (p, bus))| !bus && on(p)).map(|((_, at), _)| *at);
            let pins: Vec<Pt> = pins.iter().filter(|q| !q.hidden).map(|q| q.at).chain(sheet_pins).collect();
            if !items.is_empty() || !pins.is_empty() {
                out.push(NetOnPage { page: i, items, pins });
            }
        }
        out
    }

    /// The nets of `nets` by index, named as `netlist` names them, with the parts' pins on each;
    /// after them, each no-connect pin's net, which it is alone on.
    fn named(&self, nets: &Nets) -> Vec<(String, Vec<(String, String)>)> {
        let mut out: Vec<(String, Vec<(String, String)>)> = nets.names.iter().map(|n| (n.clone(), vec![])).collect();
        // Each net's pin to name it after, its pins other than parts' (power symbols') and
        // whether a no-connect flag is on it.
        let mut by: Vec<(Option<Namer>, usize, bool)> = vec![(None, 0, false); out.len()];
        for (i, page) in self.pages.iter().enumerate() {
            let (s, j) = (&page.sheet, &nets.pages[i]);
            for &point in &j.no_connects {
                by[nets.of(i, point)].2 = true;
            }
            let mut names: HashMap<(usize, &str), u32> = HashMap::new();
            for p in &s.wiring.pins {
                *names.entry((p.symbol, p.base.as_str())).or_default() += 1;
            }
            for (p, point) in s.wiring.pins.iter().zip(&j.pins) {
                // Power symbols and parts without a reference yet (`R?`) have no footprint.
                let sym = &s.symbols[p.symbol];
                let (part, net) = (&sym.designator, point.map(|point| nets.of(i, point)));
                if part.starts_with('#') || part.ends_with('?') {
                    if let Some(n) = net {
                        by[n].1 += 1;
                    }
                    continue;
                }
                let node = (part.clone(), p.number.clone());
                // A name names a net when no other pin of its unit has it, and isn't its number.
                // shortcut: `~` is no name in every sheet, but a name in KiCad 10's (they write
                // none as ""); tell them apart by the sheet's version when a sheet names a pin `~`.
                let name = if p.base.is_empty() || p.base == "~" || p.base == p.number { "" } else { &p.base };
                // A name goes with its unit (`U1A-SDA`), a number with its part (`U1-Pad5`).
                let unit = if name.is_empty() { part } else { &sym.reference };
                let pin = (name.is_empty() || names[&(p.symbol, p.base.as_str())] > 1, unit.as_str(), name, p.number.as_str());
                match net {
                    Some(n) => {
                        out[n].1.push(node);
                        let first = &mut by[n].0;
                        if first.is_none_or(|f| pin < f) {
                            *first = Some(pin);
                        }
                    }
                    None => out.push((named_after(pin, true), vec![node])),
                }
            }
        }
        for (i, (name, nodes)) in out.iter_mut().enumerate() {
            nodes.sort_unstable();
            nodes.dedup();
            if let Some(&(Some(pin), others, flagged)) = by.get(i).filter(|_| name.is_empty()) {
                *name = named_after(pin, flagged || nodes.len() + others == 1);
            }
        }
        out
    }
}

/// A pin KiCad can name a net after: whether its name is shared or none (then it comes after
/// those with their own), its unit's reference (`U1A`; its part's when it has no name), name
/// (empty: none) and number.
type Namer<'a> = (bool, &'a str, &'a str, &'a str);

/// The name KiCad gives a net nothing names, after the first of its pins as `Namer` orders them:
/// `unconnected-(…)` for a pin alone or flagged no-connect.
fn named_after((shared, unit, name, number): Namer, unconnected: bool) -> String {
    let name = name.replace('/', "{slash}");
    let pad = match (name.is_empty(), shared || unconnected) {
        (true, _) => format!("Pad{number}"),
        (false, true) => format!("{name}-Pad{number}"),
        (false, false) => name,
    };
    format!("{}-({unit}-{pad})", if unconnected { "unconnected" } else { "Net" })
}

/// A sheet's name as a step of a net's path: trimmed, its `/` written `{slash}`, as KiCad writes it.
fn step(sheet: &str) -> String {
    sheet.trim().replace('/', "{slash}")
}

/// The nets a bus name stands for: each member's name within the bus and its net's name.
/// `D[0..7]` is D0 to D7; a group `P{A B[0..1]}` is A, B0 and B1, named P.A, P.B0, P.B1 (without
/// a prefix just A…); a group's member or a whole name may be a bus alias of the project.
pub(super) fn members(name: &str, aliases: &HashMap<String, Vec<String>>) -> Vec<(String, String)> {
    let one = |m: &str| -> Vec<String> {
        match (vector(m), aliases.get(m)) {
            (Some((prefix, lo, hi)), _) => (lo..=hi).map(|i| format!("{prefix}{i}")).collect(),
            (None, Some(names)) => names.iter().flat_map(|n| members(n, &HashMap::new())).map(|(m, _)| m).collect(),
            (None, None) => vec![m.to_string()],
        }
    };
    let Some((prefix, inner)) = group(name) else {
        return one(name).into_iter().map(|m| (m.clone(), m)).collect();
    };
    let net = |m: String| if prefix.is_empty() { (m.clone(), m) } else { (m.clone(), format!("{prefix}.{m}")) };
    inner.split([' ', ',']).filter(|m| !m.is_empty()).flat_map(one).map(net).collect()
}

/// `PREFIX[a..b]`: its prefix and its range, low to high either way written.
fn vector(name: &str) -> Option<(&str, u32, u32)> {
    let (prefix, range) = name.strip_suffix(']')?.rsplit_once('[')?;
    let (a, b) = range.split_once("..")?;
    let (a, b) = (a.parse::<u32>().ok()?, b.parse::<u32>().ok()?);
    Some((prefix, a.min(b), a.max(b)))
}

/// `PREFIX{members}`: its prefix and members. Braces after `_`, `^` and `~` are KiCad's
/// subscript, superscript and overbar marks, not a group.
fn group(name: &str) -> Option<(&str, &str)> {
    let inner = name.strip_suffix('}')?;
    let (mut depth, mut open) = (0, None);
    for (i, ch) in inner.char_indices().rev() {
        match ch {
            '}' => depth += 1,
            '{' if depth == 0 => {
                open = Some(i);
                break;
            }
            '{' => depth -= 1,
            _ => {}
        }
    }
    let open = open?;
    let prefix = &inner[..open];
    (!prefix.ends_with(['_', '^', '~'])).then(|| (prefix, &inner[open + 1..]))
}

/// The net names of two buses' members that join: vectors member by member, otherwise by the
/// members' names within the buses.
fn matched<'a>(a: &str, am: &'a [(String, String)], b: &str, bm: &'a [(String, String)]) -> Vec<(&'a str, &'a str)> {
    if vector(a).is_some() && vector(b).is_some() {
        return am.iter().zip(bm).map(|(x, y)| (x.1.as_str(), y.1.as_str())).collect();
    }
    let by_name: HashMap<&str, &str> = bm.iter().map(|(m, n)| (m.as_str(), n.as_str())).collect();
    am.iter().filter_map(|(m, n)| Some((n.as_str(), *by_name.get(m.as_str())?))).collect()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Library symbols: R (passive pins 1 and 2, 2.5 mm left and right of its anchor), a power
    /// symbol (hidden input VCC, named by its value), parts with a hidden power input (VCC, VPP),
    /// parts with one pin at their anchor (no-connect, input, output, power input, free), a power
    /// flag (a power output), a logo, which has no pins, and parts with pins 5 mm apart down from
    /// their anchor: two named SH and an X; one named 1, its number, and one A/B; Z and Y; two
    /// units, Q1 (pin 9) with an unnamed pin 3, and Q2 (pin 10).
    const LIB: &str = r#"(lib_symbols
      (symbol "T:R" (symbol "R_1_1" (pin passive line (at -2.5 0 0) (length 1) (name "") (number "1"))
        (pin passive line (at 2.5 0 180) (length 1) (name "") (number "2"))))
      (symbol "T:PWR" (power) (symbol "PWR_1_1" (pin power_in line (at 0 0 90) (length 0) hide (name "VCC") (number "1"))))
      (symbol "T:HP" (symbol "HP_1_1" (pin power_in line (at 0 0 0) (length 1) hide (name "VCC") (number "1"))))
      (symbol "T:HP2" (symbol "HP2_1_1" (pin power_in line (at 0 0 0) (length 1) hide (name "VPP") (number "1"))))
      (symbol "T:NC" (symbol "NC_1_1" (pin no_connect line (at 0 0 0) (length 1) (name "NC") (number "1"))))
      (symbol "T:IN" (symbol "IN_1_1" (pin input line (at 0 0 0) (length 1) (name "EN") (number "1"))))
      (symbol "T:OUT" (symbol "OUT_1_1" (pin output line (at 0 0 0) (length 1) (name "Q") (number "1"))))
      (symbol "T:PI" (symbol "PI_1_1" (pin power_in line (at 0 0 0) (length 1) (name "VDD") (number "1"))))
      (symbol "T:FREE" (symbol "FREE_1_1" (pin free line (at 0 0 0) (length 1) (name "F") (number "1"))))
      (symbol "T:FLAG" (power) (symbol "FLAG_1_1" (pin power_out line (at 0 0 0) (length 0) (name "pwr") (number "1"))))
      (symbol "T:LOGO")
      (symbol "T:DUP" (symbol "DUP_1_1" (pin passive line (at 0 0 0) (length 1) (name "SH") (number "1"))
        (pin passive line (at 0 -5 0) (length 1) (name "SH") (number "2")) (pin input line (at 0 -10 0) (length 1) (name "X") (number "3"))))
      (symbol "T:ONE" (symbol "ONE_1_1" (pin passive line (at 0 0 0) (length 1) (name "1") (number "1"))
        (pin passive line (at 0 -5 0) (length 1) (name "A/B") (number "2"))))
      (symbol "T:ZY" (symbol "ZY_1_1" (pin passive line (at 0 0 0) (length 1) (name "Z") (number "1"))
        (pin passive line (at 0 -5 0) (length 1) (name "Y") (number "2"))))
      (symbol "T:MU" (symbol "MU_1_1" (pin passive line (at 0 0 0) (length 1) (name "Q1") (number "9"))
        (pin passive line (at 0 -5 0) (length 1) (name "") (number "3")))
        (symbol "MU_2_1" (pin passive line (at 0 0 0) (length 1) (name "Q2") (number "10")))))"#;

    /// A symbol of `lib` whose pin 1 is at (x, y): R's pin 2 is 5 mm right of it.
    pub(in super::super) fn part(lib: &str, reference: &str, value: &str, x: f64, y: f64) -> String {
        let at = if lib == "R" { x + 2.5 } else { x };
        format!(
            r#"(symbol (lib_id "T:{lib}") (at {at} {y} 0) (unit 1) (property "Reference" "{reference}" (at 0 0 0)) (property "Value" "{value}" (at 0 0 0)))"#
        )
    }

    pub(in super::super) fn r(reference: &str, x: f64, y: f64) -> String {
        part("R", reference, "10k", x, y)
    }

    /// A wire from (x0, y) to (x1, y), the label `kind` named `name` at its start.
    pub(in super::super) fn wire(x0: f64, x1: f64, y: f64, label: Option<(&str, &str)>) -> String {
        let label = label.map_or(String::new(), |(kind, name)| format!(r#"({kind} "{name}" (at {x0} {y} 0))"#));
        format!("(wire (pts (xy {x0} {y}) (xy {x1} {y}))) {label}")
    }

    /// A project of `(file, items)` sheets, the root first, read with `pro`.
    pub(in super::super) fn project(files: &[(&str, String)], pro: &str) -> Project {
        let texts: HashMap<String, String> = files
            .iter()
            .map(|(f, items)| (f.to_string(), format!(r#"(kicad_sch (version 20260306) (uuid "{f}") {LIB} {items})"#)))
            .collect();
        Project::load(files[0].0, pro, |f| texts.get(f).cloned().ok_or(format!("{f}: missing"))).unwrap()
    }

    /// Its nets but pins alone, as `name: pins`.
    fn nets(files: &[(&str, String)], pro: &str) -> Vec<String> {
        let p = project(files, pro);
        let mut out: Vec<String> = (p.netlist().into_iter())
            .filter(|(name, pins)| !name.starts_with("unconnected-(") || pins.len() > 1)
            .map(|(name, pins)| format!("{name}: {}", pins.iter().map(|(r, n)| format!("{r}-{n}")).collect::<Vec<_>>().join(" ")))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn joins_wires_as_kicad_does() {
        let items = [
            // A wire ending on another's middle, without a junction, isn't joined to it; nor is
            // a pin on a wire's middle.
            r("R1", 10.0, 10.0),
            wire(15.0, 30.0, 10.0, None),
            "(wire (pts (xy 20 10) (xy 20 20)))".into(),
            r("R2", 20.0, 20.0),
            r("R3", 10.0, 30.0),
            "(wire (pts (xy 15 25) (xy 15 35)))".into(),
            r("R4", 15.0, 35.0),
            // Crossing wires join at a junction; a label joins a wire anywhere along it.
            r("R5", 35.0, 10.0),
            wire(40.0, 60.0, 10.0, None),
            "(wire (pts (xy 50 0) (xy 50 20))) (junction (at 50 10))".into(),
            r("R6", 50.0, 20.0),
            r("R7", 35.0, 30.0),
            wire(40.0, 60.0, 30.0, None),
            r#"(label "LA" (at 50 30 0))"#.into(),
            wire(70.0, 80.0, 30.0, Some(("label", "LA"))),
            r("R8", 80.0, 30.0),
            // Pins meet pins; a no-connect pin joins nothing.
            r("R9", 10.0, 50.0),
            r("R10", 15.0, 50.0),
            part("NC", "U1", "", 30.0, 60.0),
            wire(25.0, 30.0, 60.0, None),
            r("R11", 20.0, 60.0),
        ];
        assert_eq!(
            nets(&[("root.kicad_sch", items.concat())], ""),
            ["/LA: R7-2 R8-1", "Net-(R10-Pad1): R10-1 R9-2", "Net-(R5-Pad2): R5-2 R6-1"]
        );
    }

    #[test]
    fn names_nets_after_pins_as_kicad_does() {
        let items = [
            // A pin with a name of its own first, then by reference, name and number, as text.
            r("R1", 10.0, 10.0),
            wire(15.0, 30.0, 10.0, None),
            part("IN", "U1", "", 30.0, 10.0),
            r("R9", 10.0, 100.0),
            wire(15.0, 20.0, 100.0, None),
            r("R10", 20.0, 100.0),
            // A name two pins have takes the pad's number; a name that is the number names nothing.
            part("DUP", "U2", "", 10.0, 20.0),
            "(wire (pts (xy 10 20) (xy 10 25)))".into(),
            part("DUP", "U3", "", 10.0, 40.0),
            wire(10.0, 20.0, 40.0, None),
            r("V1", 20.0, 40.0),
            part("ONE", "W1", "", 10.0, 60.0),
            wire(10.0, 20.0, 60.0, None),
            r("V2", 20.0, 60.0),
            part("ONE", "W2", "", 10.0, 80.0),
            wire(10.0, 20.0, 85.0, None),
            r("V3", 20.0, 85.0),
            // A name goes with its unit, a number with its part.
            part("MU", "U4", "", 10.0, 120.0),
            wire(10.0, 30.0, 120.0, None),
            r#"(symbol (lib_id "T:MU") (at 30 120 0) (unit 2) (property "Reference" "U4" (at 0 0 0)) (property "Value" "" (at 0 0 0)))"#
                .into(),
            // Unconnected: a pin alone (a power flag's counts), flagged no-connect or of that type.
            part("FLAG", "#FLG1", "PWR_FLAG", 10.0, 140.0),
            r("R30", 10.0, 140.0),
            r("R20", 10.0, 160.0),
            "(no_connect (at 10 160)) (wire (pts (xy 10 160) (xy 10 170)))".into(),
            r("R21", 10.0, 170.0),
            part("NC", "U5", "", 10.0, 180.0),
            part("ZY", "U6", "", 10.0, 200.0),
            "(wire (pts (xy 10 200) (xy 10 205)))".into(),
        ];
        let p = project(&[("root.kicad_sch", items.concat())], "");
        let nets: Vec<String> = (p.netlist().into_iter())
            .map(|(name, pins)| format!("{name}: {}", pins.iter().map(|(r, n)| format!("{r}-{n}")).collect::<Vec<_>>().join(" ")))
            .collect();
        // KiCad 10's names for these nets (`kicad-cli sch export netlist`).
        for kicad in [
            "Net-(U1-EN): R1-2 U1-1",
            "Net-(R10-Pad1): R10-1 R9-2",
            "Net-(U2-SH-Pad1): U2-1 U2-2",
            "Net-(U3-SH-Pad1): U3-1 V1-1",
            "Net-(V2-Pad1): V2-1 W1-1",
            "Net-(W2-A{slash}B): V3-1 W2-2",
            "Net-(U4A-Q1): U4-10 U4-9",
            "unconnected-(U4-Pad3): U4-3",
            "unconnected-(U2-X-Pad3): U2-3",
            "Net-(R30-Pad1): R30-1",
            "unconnected-(R30-Pad2): R30-2",
            "unconnected-(R20-Pad1): R20-1 R21-1",
            "unconnected-(U5-NC-Pad1): U5-1",
            "Net-(U6-Y): U6-1 U6-2",
        ] {
            assert!(nets.iter().any(|n| n == kicad), "{kicad} isn't in {nets:?}");
        }
    }

    #[test]
    fn names_nets_and_joins_sheets_as_kicad_does() {
        let sheet = r#"(sheet (at 100 0) (size 20 40) (uuid "s") (property "Sheetname" "Sub" (at 0 0 0))
          (property "Sheetfile" "sub.kicad_sch" (at 0 0 0)) (pin "S" input (at 100 10 180)) (pin "T" input (at 100 20 180))
          (pin "U" input (at 100 30 180)))"#;
        let root = [
            // A power symbol names its net by its value; hidden power inputs join the net of
            // their name, here and on every sheet; a global label outranks a power symbol, a
            // hidden power pin a label.
            part("PWR", "#PWR1", "VDD", 10.0, 10.0),
            wire(10.0, 20.0, 10.0, None),
            r("R1", 20.0, 10.0),
            part("PWR", "#PWR2", "VCC", 10.0, 20.0),
            wire(10.0, 20.0, 20.0, None),
            r("R2", 20.0, 20.0),
            part("HP", "U1", "", 50.0, 50.0),
            part("PWR", "#PWR3", "GP", 10.0, 30.0),
            wire(10.0, 20.0, 30.0, None),
            r#"(global_label "G1" (at 15 30 0))"#.into(),
            r("R3", 20.0, 30.0),
            part("HP2", "U2", "", 10.0, 40.0),
            wire(10.0, 20.0, 40.0, None),
            r#"(label "LL" (at 15 40 0))"#.into(),
            r("R4", 20.0, 40.0),
            wire(10.0, 20.0, 70.0, Some(("label", "VPP"))),
            r("R8", 20.0, 70.0),
            // Of two labels, the first in the alphabet.
            r("R5", 5.0, 60.0),
            wire(10.0, 20.0, 60.0, Some(("label", "ZZ"))),
            r#"(label "AA" (at 20 60 0))"#.into(),
            // Into the sheet: a label here outranks one there; there, a hierarchical label
            // outranks the sheet pin.
            sheet.into(),
            r("R6", 85.0, 10.0),
            wire(90.0, 100.0, 10.0, Some(("label", "ZROOT"))),
            r("R7", 85.0, 20.0),
            wire(90.0, 100.0, 20.0, None),
            // A sheet pin with no hierarchical label of its name there joins nothing there, and
            // names its net here.
            r("R9", 85.0, 30.0),
            wire(90.0, 100.0, 30.0, None),
        ];
        let sub = [
            wire(10.0, 20.0, 10.0, Some(("hierarchical_label", "S"))),
            r#"(label "ACHILD" (at 15 10 0))"#.into(),
            r("R10", 20.0, 10.0),
            wire(10.0, 20.0, 20.0, Some(("hierarchical_label", "T"))),
            r("R11", 20.0, 20.0),
            wire(10.0, 20.0, 30.0, Some(("global_label", "G1"))),
            r("R12", 20.0, 30.0),
            wire(10.0, 20.0, 40.0, Some(("label", "AA"))),
            r("R13", 20.0, 40.0),
            wire(10.0, 20.0, 50.0, Some(("label", "U"))),
            r("R14", 20.0, 50.0),
        ];
        assert_eq!(
            nets(&[("root.kicad_sch", root.concat()), ("sub.kicad_sch", sub.concat())], ""),
            [
                "/AA: R5-2",
                "/Sub/AA: R13-1",
                "/Sub/T: R11-1 R7-2",
                "/Sub/U: R14-1",
                "/U: R9-2",
                "/ZROOT: R10-1 R6-2",
                "G1: R12-1 R3-1",
                "VCC: R2-1 U1-1",
                "VDD: R1-1",
                "VPP: R4-1 R8-1 U2-1"
            ]
        );
    }

    #[test]
    fn finds_a_net_on_every_sheet_it_reaches() {
        let sheet = r#"(sheet (at 100 0) (size 20 40) (uuid "s") (property "Sheetname" "Sub" (at 0 0 0))
          (property "Sheetfile" "sub.kicad_sch" (at 0 0 0)) (pin "S" input (at 100 10 180)))"#;
        let root = [
            sheet.into(),
            r("R6", 85.0, 10.0),
            wire(90.0, 100.0, 10.0, Some(("label", "ZROOT"))),
            part("PWR", "#PWR1", "VDD", 10.0, 30.0),
            wire(10.0, 20.0, 30.0, None),
            r("R1", 20.0, 30.0),
            wire(40.0, 50.0, 50.0, None),
        ];
        let sub = [wire(10.0, 20.0, 10.0, Some(("hierarchical_label", "S"))), r("R10", 20.0, 10.0)];
        let p = project(&[("root.kicad_sch", root.concat()), ("sub.kicad_sch", sub.concat())], "");
        // A wire's or label's net, by the name the netlist gives it; none for a symbol, or a wire
        // that nothing names and no pin is on.
        assert_eq!((p.net_of(0, Item::Wire(0)), p.net_of(1, Item::Label(0))), (Some("/ZROOT".into()), Some("/ZROOT".into())));
        assert_eq!(
            (p.net_of(0, Item::Wire(1)), p.net_of(0, Item::Wire(2)), p.net_of(0, Item::Symbol(1))),
            (Some("VDD".into()), None, None)
        );
        // Where it is on each sheet: wires, labels, power symbols; pins and sheet pins, not hidden ones.
        let pt = |x: i32, y: i32| Pt { x: x * 1_000_000, y: y * 1_000_000 };
        let on = |page, items: &[Item], pins: &[Pt]| NetOnPage { page, items: items.to_vec(), pins: pins.to_vec() };
        let here = [Item::Wire(0), Item::Label(0)];
        assert_eq!(p.net_items("/ZROOT"), [on(0, &here, &[pt(90, 10), pt(100, 10)]), on(1, &here, &[pt(20, 10)])]);
        assert_eq!(p.net_items("VDD"), [on(0, &[Item::Wire(1), Item::Symbol(1)], &[pt(20, 30)])]);
        assert_eq!((p.net_items("unconnected-(R1-Pad2)"), p.net_items("/Sub/S")), (vec![on(0, &[], &[pt(25, 30)])], vec![]));
    }

    #[test]
    fn joins_buses_member_by_member() {
        // Vectors member by member (a range either way round), groups by member name, a group of
        // the project's bus alias.
        let pins = r#"(pin "B[0..1]" bidirectional (at 100 10 180)) (pin "{X Y}" bidirectional (at 100 30 180))
          (pin "{AL}" bidirectional (at 100 50 180))"#;
        let sheet = format!(
            r#"(sheet (at 100 0) (size 20 60) (uuid "s") (property "Sheetname" "Sub" (at 0 0 0)) (property "Sheetfile" "sub.kicad_sch" (at 0 0 0)) {pins})"#
        );
        let bus = |x0: f64, x1: f64, y: f64, kind: &str, name: &str| {
            format!(r#"(bus (pts (xy {x0} {y}) (xy {x1} {y}))) ({kind} "{name}" (at {} {y} 0))"#, (x0 + x1) / 2.0)
        };
        let root = [
            sheet,
            bus(80.0, 100.0, 10.0, "label", "D[3..2]"),
            wire(60.0, 70.0, 10.0, Some(("label", "D2"))),
            r("R1", 70.0, 10.0),
            wire(60.0, 70.0, 20.0, Some(("label", "D3"))),
            r("R2", 70.0, 20.0),
            bus(80.0, 100.0, 30.0, "label", "P{X Y}"),
            wire(60.0, 70.0, 30.0, Some(("label", "P.Y"))),
            r("R3", 70.0, 30.0),
            bus(80.0, 100.0, 50.0, "label", "{AL}"),
            wire(60.0, 70.0, 50.0, Some(("label", "M2"))),
            r("R4", 70.0, 50.0),
        ];
        let sub = [
            bus(0.0, 20.0, 10.0, "hierarchical_label", "B[0..1]"),
            wire(30.0, 40.0, 10.0, Some(("label", "B0"))),
            r("R10", 40.0, 10.0),
            wire(30.0, 40.0, 20.0, Some(("label", "B1"))),
            r("R11", 40.0, 20.0),
            bus(0.0, 20.0, 30.0, "hierarchical_label", "{X Y}"),
            wire(30.0, 40.0, 30.0, Some(("label", "Y"))),
            r("R12", 40.0, 30.0),
            bus(0.0, 20.0, 50.0, "hierarchical_label", "{AL}"),
            wire(30.0, 40.0, 50.0, Some(("label", "M2"))),
            r("R13", 40.0, 50.0),
        ];
        // The sheet pins sit on the root's buses' ends, the hierarchical labels at the middle of
        // the sheet's.
        let sub = sub
            .concat()
            .replace(r#"(at 10 10 0)"#, r#"(at 0 10 0)"#)
            .replace("(at 10 30 0)", "(at 0 30 0)")
            .replace("(at 10 50 0)", "(at 0 50 0)");
        let pro = r#"{"schematic": {"bus_aliases": {"AL": ["M1", "M2"]}}}"#;
        assert_eq!(
            nets(&[("root.kicad_sch", root.concat()), ("sub.kicad_sch", sub)], pro),
            ["/D2: R1-1 R10-1", "/D3: R11-1 R2-1", "/M2: R13-1 R4-1", "/P.Y: R12-1 R3-1"]
        );
    }

    #[test]
    fn reads_bus_names_as_kicad_does() {
        let aliases = HashMap::from([("AL".to_string(), vec!["M1".to_string(), "Q[0..1]".to_string()])]);
        let nets = |name| members(name, &aliases).into_iter().map(|(_, net)| net).collect::<Vec<_>>().join(" ");
        assert_eq!(
            [nets("D[3..1]"), nets("A{DIR[0..1] CLK}"), nets("{AL}"), nets("X{AL}")],
            ["D1 D2 D3", "A.DIR0 A.DIR1 A.CLK", "M1 Q0 Q1", "X.M1 X.Q0 X.Q1"]
        );
        // KiCad's subscripts and overbars aren't groups.
        assert_eq!([nets("V_{IO}"), nets("~{RESET}")], ["V_{IO}", "~{RESET}"]);
    }
}
