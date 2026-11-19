//! Electrical rules (ERC), in plain language: each problem says what is wrong, where, and how to
//! fix it. The checks KiCad 10 makes of what a schematic connects (`kicad-cli sch erc`, compared
//! in `tests/corpus.rs`), with the project's severities and pin conflict map.

use super::net::{self, LabelKind, Nets, PinKind};
use super::{Page, Project};
use crate::board::Pt;
use crate::json::Val;
use crate::tr::{t, tr, trf};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Display;

/// A broken electrical rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Violation {
    /// KiCad's name for the rule broken (`pin_not_connected`).
    pub kind: &'static str,
    /// An error, else a warning.
    pub error: bool,
    /// Where: an index into `Project::pages`, and a point on that page.
    pub page: usize,
    pub at: Pt,
    pub what: String,
    pub fix: String,
}

impl Violation {
    /// What its kind is called over a list of them, in the language shown (KiCad's name for a
    /// kind this doesn't know).
    pub fn kind_name(&self) -> &'static str {
        tr(match self.kind {
            "pin_not_connected" => t!("Pins not connected"),
            "pin_not_driven" => t!("Inputs nothing drives"),
            "power_pin_not_driven" => t!("Power pins nothing drives"),
            "pin_to_pin" => t!("Pins in conflict"),
            "no_connect_connected" => t!("No-connect pins wired"),
            "no_connect_dangling" => t!("No-connect flags on no pin"),
            "label_dangling" => t!("Labels on nothing"),
            "isolated_pin_label" => t!("Labels on one pin alone"),
            "same_local_global_label" => t!("Local and global labels of one name"),
            "similar_labels" => t!("Names alike but in case"),
            "wire_dangling" => t!("Wires on nothing"),
            "unconnected_wire_endpoint" => t!("Wire ends on nothing"),
            "hier_label_mismatch" => t!("Sheet pins and labels without each other"),
            "multiple_net_names" => t!("Nets of two names"),
            "net_not_bus_member" => t!("Nets not of their bus"),
            "duplicate_reference" => t!("References used twice"),
            "unannotated" => t!("Parts not numbered"),
            other => other,
        })
    }
}

/// The checks made and whether KiCad calls each an error (else a warning) unless the project
/// says otherwise; `pin_to_pin` takes its severity from the pin conflict map.
const CHECKS: [(&str, bool); 17] = [
    ("pin_not_connected", true),
    ("pin_not_driven", true),
    ("power_pin_not_driven", true),
    ("pin_to_pin", true),
    ("no_connect_connected", false),
    ("no_connect_dangling", false),
    ("label_dangling", true),
    ("isolated_pin_label", false),
    ("same_local_global_label", false),
    ("similar_labels", false),
    ("wire_dangling", true),
    ("unconnected_wire_endpoint", false),
    ("hier_label_mismatch", true),
    ("multiple_net_names", false),
    ("net_not_bus_member", false),
    ("duplicate_reference", true),
    ("unannotated", true),
];

/// KiCad's pin conflict map: 0 fine, 1 a warning, 2 an error, by `PinKind` (input, output,
/// bidirectional, tri-state, passive, free, unspecified, power in, power out, open collector,
/// open emitter, no-connect).
const PIN_MAP: [[u8; 12]; 12] = [
    [0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 2],
    [0, 2, 0, 1, 0, 0, 1, 0, 2, 2, 2, 2],
    [0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 2],
    [0, 1, 0, 0, 0, 0, 1, 1, 2, 1, 1, 2],
    [0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 2],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2],
    [1, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 2],
    [0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 2],
    [0, 2, 1, 2, 0, 0, 1, 0, 2, 2, 2, 2],
    [0, 2, 0, 1, 0, 0, 1, 0, 2, 0, 0, 2],
    [0, 2, 1, 1, 0, 0, 1, 0, 2, 0, 0, 2],
    [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
];

/// A project's ERC settings (`.kicad_pro`): each check's severity and the pin conflict map.
#[derive(Clone, Debug)]
pub(super) struct Settings {
    /// `Some(error)` for each check made; ignored ones aren't here.
    severity: HashMap<&'static str, bool>,
    map: [[u8; 12]; 12],
}

impl Settings {
    pub(super) fn of(pro: Option<&Val>) -> Settings {
        let erc = pro.and_then(|p| p.get("erc"));
        let set = erc.and_then(|e| e.get("rule_severities"));
        let severity = (CHECKS.iter())
            .filter_map(|&(kind, error)| match set.and_then(|s| s.str(kind)) {
                Some("ignore") => None,
                Some(s) => Some((kind, s == "error")),
                None => Some((kind, error)),
            })
            .collect();
        let mut map = PIN_MAP;
        if let Some(rows) = erc.and_then(|e| e.arr("pin_map")).filter(|r| r.len() == 12) {
            for (row, from) in map.iter_mut().zip(rows) {
                if let Val::Arr(cells) = from {
                    for (cell, v) in row.iter_mut().zip(cells) {
                        if let Val::Num(n) = v {
                            *cell = *n as u8;
                        }
                    }
                }
            }
        }
        Settings { severity, map }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings::of(None)
    }
}

/// What a group of items wired together on a sheet holds.
#[derive(Clone, Default)]
struct Group {
    pins: u32,
    labels: u32,
    sheet_pins: u32,
    no_connects: u32,
}

impl Project {
    /// Its electrical rule violations, page by page: errors first. A problem inside a sheet used
    /// more than once is told once, on its first page (KiCad tells some once per use).
    // shortcut: no check needs the symbol libraries yet (library symbol and footprint links: KiCad
    // tells each symbol whose library it lacks, thousands on the corpus projects); nor are grid,
    // bus syntax, netclass, unit, ground-pin and simulation checks made: add them when users ask.
    pub fn erc(&self) -> Vec<Violation> {
        let nets = self.nets();
        let mut e = Erc { p: self, nets: &nets, seen: HashSet::new(), named: HashSet::new(), strays: HashSet::new(), out: vec![] };
        e.pins();
        e.labels();
        e.wires();
        e.sheets();
        e.parts();
        let mut out = e.out;
        out.sort_by_key(|v| (v.page, !v.error, v.kind, v.at.x, v.at.y));
        out
    }
}

struct Erc<'a> {
    p: &'a Project,
    nets: &'a Nets,
    /// Problems told: file, kind and place; and the nets told of having two names or of not
    /// being a member of their bus, once each.
    seen: HashSet<(&'a str, &'static str, Pt)>,
    named: HashSet<usize>,
    strays: HashSet<usize>,
    out: Vec<Violation>,
}

impl<'a> Erc<'a> {
    /// Tells a problem, as its check's severity says (`error` for `pin_to_pin`), once per place.
    fn tell(&mut self, kind: &'static str, error: Option<bool>, page: usize, at: Pt, what: String, fix: String) {
        let Some(&severity) = self.p.erc.severity.get(kind) else { return };
        if self.seen.insert((&self.p.pages[page].file, kind, at)) {
            let mut chars = what.chars();
            let what = chars.next().map_or(String::new(), |c| c.to_uppercase().chain(chars).collect());
            self.out.push(Violation { kind, error: error.unwrap_or(severity), page, at, what, fix });
        }
    }

    /// What each group of a page's wired items holds, by point.
    fn groups(&self, page: usize) -> Vec<Group> {
        let j = &self.nets.pages[page];
        let (mut g, at) = (vec![Group::default(); j.group.len()], |point: u32| j.group[point as usize] as usize);
        j.pins.iter().flatten().for_each(|&p| g[at(p)].pins += 1);
        j.labels.iter().filter(|l| !l.1).for_each(|&(p, _)| g[at(p)].labels += 1);
        j.sheet_pins.iter().flatten().filter(|s| !s.1).for_each(|&(p, _)| g[at(p)].sheet_pins += 1);
        j.no_connects.iter().for_each(|&p| g[at(p)].no_connects += 1);
        g
    }

    /// `U3 pin 5 (EN)`: a pin by its part, number and name; a power symbol's by its net.
    fn pin(&self, page: usize, i: usize) -> String {
        let (s, p) = (&self.p.pages[page].sheet, &self.p.pages[page].sheet.wiring.pins[i]);
        let sym = &s.symbols[p.symbol];
        match &sym.power {
            Some((_, value)) => trf("power symbol {0}", &[value]),
            None if p.name.is_empty() || p.name == "~" => trf("{0} pin {1}", &[&sym.designator, &p.number]),
            None => trf("{0} pin {1} ({2})", &[&sym.designator, &p.number, &p.name]),
        }
    }

    /// ` (NAME)` for a named net, else nothing.
    fn net_name(&self, net: usize) -> String {
        let name = &self.nets.names[net];
        if name.is_empty() { String::new() } else { format!(" ({name})") }
    }

    /// Each pin connected, driven, and of a type its net's other pins can share.
    fn pins(&mut self) {
        let (p, nets) = (self.p, self.nets);
        let pin = |(pg, k): (usize, usize)| &p.pages[pg].sheet.wiring.pins[k];
        // A hidden power input of a part, which joins the net of its name.
        let implicit = |q| pin(q).hidden && pin(q).kind == PinKind::PowerIn && p.pages[q.0].sheet.symbols[pin(q).symbol].power.is_none();
        // Each net's pins, and whether a no-connect flag is on it.
        let (mut on, mut flagged) = (vec![vec![]; nets.names.len()], vec![false; nets.names.len()]);
        for (i, j) in nets.pages.iter().enumerate() {
            j.pins.iter().enumerate().filter_map(|(k, pt)| Some((nets.of(i, (*pt)?), (i, k)))).for_each(|(net, q)| on[net].push(q));
            j.no_connects.iter().for_each(|&f| flagged[nets.of(i, f)] = true);
        }
        for (i, page) in p.pages.iter().enumerate() {
            let (j, w, groups) = (&nets.pages[i], &page.sheet.wiring, self.groups(i));
            for (k, (q, point)) in w.pins.iter().zip(&j.pins).enumerate() {
                let Some(point) = *point else {
                    // A no-connect pin joins nothing: anything on its end is a mistake.
                    let wired = w.wires.iter().any(|&(a, b)| a == q.at || b == q.at);
                    if wired || w.pins.iter().enumerate().any(|(o, r)| o != k && r.at == q.at) || w.labels.iter().any(|l| l.at == q.at) {
                        let what = trf("{0} is a not-connected pin in its symbol, but something connects to it.", &[&self.pin(i, k)]);
                        let fix = tr("Leave the pin open, or, if it does connect, fix its type in the symbol.");
                        self.tell("no_connect_connected", None, i, q.at, what, fix.into());
                    }
                    continue;
                };
                let g = &groups[j.group[point as usize] as usize];
                let net = nets.of(i, point);
                // A hidden power input is connected when its net has more than those (as KiCad).
                let joined = implicit((i, k)) && on[net].iter().any(|&r| r != (i, k) && !implicit(r));
                if g.pins > 1 || g.labels > 0 || g.sheet_pins > 0 || g.no_connects > 0 || joined || q.kind == PinKind::Free {
                    continue;
                }
                let (what, fix) = if q.hidden {
                    (
                        trf("{0}, hidden in its symbol, connects to nothing.", &[&self.pin(i, k)]),
                        tr("Show the hidden pins and wire it, or flag it no-connect."),
                    )
                } else {
                    (
                        trf("{0} connects to nothing.", &[&self.pin(i, k)]),
                        tr("Wire it to its net, or put a no-connect flag on its end if it stays unused."),
                    )
                };
                self.tell("pin_not_connected", None, i, q.at, what, fix.into());
            }
            for (sheet, points) in page.sheet.sheets.iter().zip(&j.sheet_pins) {
                for ((name, at), &(point, _)) in sheet.pins.iter().zip(points) {
                    let g = &groups[j.group[point as usize] as usize];
                    if g.pins == 0 && g.labels == 0 && g.sheet_pins == 1 {
                        let what = trf("Sheet pin {0} of sheet {1} connects to nothing here.", &[name, &sheet.name]);
                        let fix = tr("Wire it on this sheet, or delete the pin if nothing needs it.");
                        self.tell("pin_not_connected", None, i, *at, what, fix.into());
                    }
                }
            }
        }
        for (net, pins) in on.iter().enumerate() {
            let kind = |q: (usize, usize)| pin(q).kind;
            if !flagged[net] {
                use PinKind::*;
                let drives = pins.iter().any(|&q| matches!(kind(q), Output | Bidirectional | TriState | Passive | PowerIn | PowerOut));
                if let (Some(&q), false) = (pins.iter().find(|&&q| kind(q) == Input), drives) {
                    let what = trf("{0} is an input, but nothing on its net{1} drives it.", &[&self.pin(q.0, q.1), &self.net_name(net)]);
                    let fix = tr("Connect an output to it, or a pull-up or pull-down resistor; if the pin is unused, flag it no-connect.");
                    self.tell("pin_not_driven", None, q.0, pin(q).at, what, fix.into());
                }
                let powered = pins.iter().any(|&q| kind(q) == PowerOut);
                if let (Some(&q), false) = (pins.iter().find(|&&q| kind(q) == PowerIn), powered) {
                    let what = trf(
                        "{0} takes power, but no pin on its net{1} is a power output to give it.",
                        &[&self.pin(q.0, q.1), &self.net_name(net)],
                    );
                    let fix = tr(
                        "If the power comes from a connector, a battery or a part whose pins aren't power outputs, put a PWR_FLAG on the net; \
                         otherwise connect the supply.",
                    );
                    self.tell("power_pin_not_driven", None, q.0, pin(q).at, what, fix.into());
                }
            }
            // Each pin against the earlier pin it conflicts with worst.
            let mut first: [Option<(usize, usize)>; 12] = [None; 12];
            for &q in pins {
                let b = kind(q) as usize;
                let worst = (0..12).filter_map(|a| Some((p.erc.map[a][b], first[a]?))).max_by_key(|&(m, _)| m);
                if let Some((m @ 1.., r)) = worst {
                    let (what, fix) = self.conflict(r, q, net);
                    self.tell("pin_to_pin", Some(m == 2), q.0, pin(q).at, what, fix);
                }
                first[b].get_or_insert(q);
            }
        }
    }

    /// Why two pins' types shouldn't share a net, and what to do.
    fn conflict(&self, a: (usize, usize), b: (usize, usize), net: usize) -> (String, String) {
        use PinKind::*;
        let kind = |(pg, k): (usize, usize)| self.p.pages[pg].sheet.wiring.pins[k].kind;
        let (ka, kb) = (kind(a), kind(b));
        let drives = |k| matches!(k, Output | PowerOut | TriState | OpenCollector | OpenEmitter | Bidirectional);
        let both: [&dyn Display; 5] = [&self.pin(a.0, a.1), &word(ka), &self.pin(b.0, b.1), &word(kb), &self.net_name(net)];
        let (what, fix) = if ka == Unspecified || kb == Unspecified {
            (
                trf("{0}, {1}, and {2}, {3}, share a net{4}: a pin of no type can't be checked.", &both),
                tr("Set the pin's electrical type in its symbol."),
            )
        } else if drives(ka) && drives(kb) {
            (
                trf("{0}, {1}, and {2}, {3}, drive one net{4} against each other.", &both),
                tr("Leave one output on the net; if a pin is really an input, fix its type in the symbol."),
            )
        } else {
            (
                trf("{0}, {1}, and {2}, {3}, shouldn't share a net{4}: one can't supply the other.", &both),
                tr("Supply the pin from a pin of the right type, or fix the types in the symbols."),
            )
        };
        (what, fix.into())
    }

    /// Labels on nets with other pins than one, and names unlike others but in case.
    fn labels(&mut self) {
        let (p, nets) = (self.p, self.nets);
        let mut pins = vec![0u32; nets.names.len()];
        for (i, j) in nets.pages.iter().enumerate() {
            j.pins.iter().flatten().for_each(|&pt| pins[nets.of(i, pt)] += 1);
        }
        // A sheet used more than once: its labels on its first page (as KiCad).
        let mut files = HashSet::new();
        for (i, page) in p.pages.iter().enumerate().filter(|(_, page)| files.insert(page.file.as_str())) {
            let (j, w) = (&nets.pages[i], &page.sheet.wiring);
            // Each name's spellings on this page, and the names its global labels have.
            let mut spellings: HashMap<String, Vec<&str>> = HashMap::new();
            let globals: HashSet<&str> = labels(page, LabelKind::Global).map(|l| l.name.as_str()).collect();
            for (l, &(point, bus)) in w.labels.iter().zip(&j.labels) {
                let seen = spellings.entry(l.name.to_lowercase()).or_default();
                if !seen.contains(&l.name.as_str()) {
                    if let Some(first) = seen.first() {
                        let what = trf("Labels {0} and {1} differ only in case, so they name two nets.", &[first, &l.name]);
                        self.tell("similar_labels", None, i, l.at, what, tr("Spell them alike if they are one net, or rename one.").into());
                    }
                    seen.push(&l.name);
                }
                if l.kind == LabelKind::Local && globals.contains(l.name.as_str()) {
                    let what = trf(
                        "Label {0} is local and a global label {0} is on this sheet: here they join, but labels {0} on other sheets don't.",
                        &[&l.name],
                    );
                    self.tell("same_local_global_label", None, i, l.at, what, tr("Make the local label global, or rename it.").into());
                }
                let label = || format!("{} {}", tr(l.kind.name()), l.name);
                match (bus, pins[nets.of(i, point)]) {
                    (false, 0) => {
                        let (what, fix) =
                            (trf("{0} connects to no pin.", &[&label()]), tr("Put it on a wire or pin end of its net, or delete it."));
                        self.tell("label_dangling", None, i, l.at, what, fix.into());
                    }
                    (false, 1) => {
                        let what = trf("{0} reaches only one pin: no other pin is on its net.", &[&label()]);
                        let fix = tr("Check that the label at the other end has the same name, case too, or remove this one.");
                        self.tell("isolated_pin_label", None, i, l.at, what, fix.into());
                    }
                    _ => {}
                }
            }
        }
    }

    /// Wires and bus entries that connect something at each end, and no-connect flags on
    /// unconnected pins; then each page's names and bus members.
    fn wires(&mut self) {
        let (p, nets) = (self.p, self.nets);
        for (i, page) in p.pages.iter().enumerate() {
            let (j, w, groups) = (&nets.pages[i], &page.sheet.wiring, self.groups(i));
            let group = |point: u32| &groups[j.group[point as usize] as usize];
            // Wires by their two ends, bus entries by their wire's end; what is told of one on
            // nothing, and of an end on nothing.
            let wire = (tr("A wire connects to nothing: no pin or label is on it."), tr("A wire's end connects to nothing."));
            let entry = (tr("A bus entry connects to nothing: no pin or label is on it."), tr("A bus entry's end connects to nothing."));
            let ends = j.wires.iter().map(|e| (wire, &e[..])).chain(j.entries.iter().map(|e| (entry, &e[..1])));
            let mut told = HashSet::new();
            for ((alone, end_alone), ends) in ends {
                let g = group(ends[0]);
                if g.pins == 0 && g.labels == 0 && g.sheet_pins == 0 {
                    if told.insert(j.group[ends[0] as usize]) {
                        self.tell("wire_dangling", None, i, j.at[ends[0] as usize], alone.into(), tr("Connect it, or delete it.").into());
                    }
                    continue;
                }
                // A no-connect pin, which joins nothing, still ends a wire.
                let pin_there = |e: u32| w.pins.iter().any(|q| q.kind == PinKind::NoConnect && q.at == j.at[e as usize]);
                for &end in ends.iter().filter(|&&e| j.hits[e as usize] == 1 && !pin_there(e)) {
                    let fix = tr("End it on a pin, a wire's end or a junction: a wire ending on another's middle needs a junction there.");
                    self.tell("unconnected_wire_endpoint", None, i, j.at[end as usize], end_alone.into(), fix.into());
                }
            }
            for (&at, &point) in w.no_connects.iter().zip(&j.no_connects) {
                let (g, here) = (group(point), j.group[point as usize]);
                // The flag marks the pins at one place (stacked pins too); pins at two connect.
                let mut places =
                    w.pins.iter().zip(&j.pins).filter(|(_, pt)| pt.is_some_and(|pt| j.group[pt as usize] == here)).map(|(q, _)| q.at);
                let apart = places.next().is_some_and(|first| places.any(|p| p != first));
                if g.pins == 0 && g.labels == 0 && !w.pins.iter().any(|q| q.kind == PinKind::NoConnect && q.at == at) {
                    let (what, fix) =
                        (tr("A no-connect flag isn't on any pin."), tr("Move it onto the end of the pin it marks, or delete it."));
                    self.tell("no_connect_dangling", None, i, at, what.into(), fix.into());
                } else if apart || g.pins > 0 && g.labels + g.sheet_pins > 0 {
                    let what = match w.pins.iter().position(|q| q.at == at && q.kind != PinKind::NoConnect) {
                        Some(k) => trf("{0} has a no-connect flag, but connects to other pins.", &[&self.pin(i, k)]),
                        None => tr("A no-connect flag is on a wire that connects pins.").into(),
                    };
                    let fix = tr("Remove the flag if the pin is meant to connect, else what connects to it.");
                    self.tell("no_connect_connected", None, i, at, what, fix.into());
                }
            }
            self.names(i);
            self.bus_members(i);
        }
    }

    /// Items wired together under two or more names: their net (or bus) takes one, as KiCad ranks
    /// them.
    fn names(&mut self, i: usize) {
        let (s, j, nets) = (&self.p.pages[i].sheet, &self.nets.pages[i], self.nets);
        // By group, in order: a net of two groups is told at the first's, the same each time.
        let mut names: BTreeMap<u32, Vec<(u8, &str, u32, bool)>> = BTreeMap::new();
        for n in net::names(s, j) {
            names.entry(j.group[n.point as usize]).or_default().push((n.rank, n.name, n.point, n.bus));
        }
        for (group, mut on) in names {
            on.sort_unstable();
            let mut distinct: Vec<&str> = on.iter().map(|n| n.1).collect();
            distinct.dedup();
            if distinct.len() > 1 && self.named.insert(nets.of(i, group)) {
                let names: [&dyn Display; 3] = [&distinct.len(), &list(&distinct), &on[0].1];
                let what = match on[0].3 {
                    true => trf("One bus has {0} names, {1}; it takes {2}.", &names),
                    false => trf("One net has {0} names, {1}; it takes {2}.", &names),
                };
                let fix = tr("If the names are one net, keep one; if they are two nets, they are shorted here: separate them.");
                self.tell("multiple_net_names", None, i, j.at[on[0].2 as usize], what, fix.into());
            }
        }
    }

    /// Wires that enter a bus through a bus entry under a name that isn't one of its members.
    fn bus_members(&mut self, i: usize) {
        let (s, j, nets) = (&self.p.pages[i].sheet, &self.nets.pages[i], self.nets);
        // Each bus's name, by group, as KiCad picks one: of its labels' and sheet pins' on this
        // page, by kind (global, local, hierarchical label, sheet pin), then the alphabet.
        let rank = |k: LabelKind| [LabelKind::Global, LabelKind::Local, LabelKind::Hier].iter().position(|&o| o == k).unwrap_or(0);
        let labels = s.wiring.labels.iter().zip(&j.labels).map(|(l, &(pt, bus))| (rank(l.kind), l.name.as_str(), pt, bus));
        let pins = s
            .sheets
            .iter()
            .zip(&j.sheet_pins)
            .flat_map(|(sh, pins)| sh.pins.iter().zip(pins).map(|((n, _), &(pt, bus))| (3, n.as_str(), pt, bus)));
        let mut buses: HashMap<u32, (usize, &str)> = HashMap::new();
        let mut names: HashMap<u32, Vec<&str>> = HashMap::new();
        for (rank, name, point, _) in labels.clone().chain(pins).filter(|n| n.3) {
            let first = buses.entry(j.group[point as usize]).or_insert((rank, name));
            *first = (*first).min((rank, name));
        }
        for (_, name, point, _) in labels.filter(|n| !n.3) {
            names.entry(j.group[point as usize]).or_default().push(name);
        }
        // A net named on a sheet (not a global or power net) that isn't a member; once each. Each
        // bus's members are worked out once: a page's entries mostly enter one or two.
        let mut members: HashMap<&str, HashSet<String>> = HashMap::new();
        for &[wire, bus] in &j.entries {
            let (Some(&(_, bus)), Some(on)) = (buses.get(&j.group[bus as usize]), names.get(&j.group[wire as usize])) else { continue };
            let net = nets.of(i, wire);
            if !nets.names[net].starts_with('/') || self.strays.contains(&net) {
                continue;
            }
            let members =
                members.entry(bus).or_insert_with(|| net::members(bus, &self.p.aliases).into_iter().map(|(_, net)| net).collect());
            if !on.iter().any(|n| members.contains(*n)) {
                self.strays.insert(net);
                let what = trf("Net {0} enters bus {1} through a bus entry but isn't one of its members.", &[&on[0], &bus]);
                let fix = tr("Name the wire after a member of the bus, or remove the entry.");
                self.tell("net_not_bus_member", None, i, j.at[wire as usize], what, fix.into());
            }
        }
    }

    /// Each sheet pin with its hierarchical label inside the sheet, and back.
    fn sheets(&mut self) {
        let p = self.p;
        for (i, page) in p.pages.iter().enumerate() {
            for (sheet, &child) in page.sheet.sheets.iter().zip(&page.children) {
                let Some(c) = child else { continue };
                let inside: HashSet<&str> = labels(&p.pages[c], LabelKind::Hier).map(|l| l.name.as_str()).collect();
                let pins: HashSet<&str> = sheet.pins.iter().map(|(n, _)| n.as_str()).collect();
                for (name, at) in sheet.pins.iter().filter(|(n, _)| !inside.contains(n.as_str())) {
                    let what =
                        trf("Sheet pin {0} of sheet {1} has no hierarchical label {0} inside {2}.", &[name, &sheet.name, &sheet.file]);
                    let fix = tr("Add the label inside the sheet, its name exactly the pin's, or delete the pin.");
                    self.tell("hier_label_mismatch", None, i, *at, what, fix.into());
                }
                for l in labels(&p.pages[c], LabelKind::Hier).filter(|l| !pins.contains(l.name.as_str())) {
                    let what = match page.path.is_empty() {
                        true => trf("Hierarchical label {0} has no sheet pin {0} on this sheet's box in the root sheet.", &[&l.name]),
                        false => trf(
                            "Hierarchical label {0} has no sheet pin {0} on this sheet's box in sheet {1}.",
                            &[&l.name, &page.path.join("/")],
                        ),
                    };
                    let fix = tr("Add the pin to the sheet's box (KiCad: Sync Sheet Pins), or delete the label.");
                    self.tell("hier_label_mismatch", None, c, l.at, what, fix.into());
                }
            }
        }
    }

    /// Each part numbered, and only once.
    fn parts(&mut self) {
        let p = self.p;
        let mut first: HashMap<&str, usize> = HashMap::new();
        for (i, page) in p.pages.iter().enumerate() {
            for sym in page.sheet.symbols.iter().filter(|s| !s.designator.starts_with('#')) {
                let at = Pt { x: (sym.bounds.0.x + sym.bounds.1.x) / 2, y: (sym.bounds.0.y + sym.bounds.1.y) / 2 };
                if sym.designator.ends_with('?') {
                    let what = trf("{0} has no number yet.", &[&sym.designator]);
                    self.tell("unannotated", None, i, at, what, tr("Annotate the schematic (KiCad: Tools → Annotate Schematic).").into());
                } else if let Some(&other) = first.get(sym.reference.as_str()) {
                    let (name, path) = (&sym.reference, &p.pages[other].path);
                    let what = match (other == i, path.is_empty()) {
                        (true, _) => trf("Two parts are named {0}: this one and one on this sheet.", &[name]),
                        (false, true) => trf("Two parts are named {0}: this one and one on the root sheet.", &[name]),
                        (false, false) => trf("Two parts are named {0}: this one and one on sheet {1}.", &[name, &path.join("/")]),
                    };
                    self.tell("duplicate_reference", None, i, at, what, tr("Annotate the schematic again, or rename one.").into());
                } else {
                    first.insert(&sym.reference, i);
                }
            }
        }
    }
}

/// A page's labels of one kind.
fn labels(page: &Page, kind: LabelKind) -> impl Iterator<Item = &net::Label> {
    page.sheet.wiring.labels.iter().filter(move |l| l.kind == kind)
}

/// `A`, `A and B`, `A, B and C`.
fn list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => trf("{0} and {1}", &[&rest.join(", "), last]),
    }
}

/// What a pin of a type is, in plain words.
fn word(k: PinKind) -> &'static str {
    match k {
        PinKind::Input => tr("an input"),
        PinKind::Output => tr("an output"),
        PinKind::Bidirectional => tr("a bidirectional pin"),
        PinKind::TriState => tr("a tri-state output"),
        PinKind::Passive => tr("a passive pin"),
        PinKind::Free => tr("a free pin"),
        PinKind::Unspecified => tr("a pin of no stated type"),
        PinKind::PowerIn => tr("a power input"),
        PinKind::PowerOut => tr("a power output"),
        PinKind::OpenCollector => tr("an open-collector output"),
        PinKind::OpenEmitter => tr("an open-emitter output"),
        PinKind::NoConnect => tr("a not-connected pin"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_every_kind_it_checks() {
        // A check added is named for the list's headings: KiCad's key isn't shown.
        for (kind, error) in CHECKS {
            let v = Violation { kind, error, page: 0, at: Pt::default(), what: String::new(), fix: String::new() };
            assert!(v.kind_name() != kind && !v.kind_name().contains('_'), "{kind}");
        }
    }
    use crate::schematic::net::tests::{part, project, r, wire};

    fn mm(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6) as i32, y: (y * 1e6) as i32 }
    }

    /// Its violations: kind, error, page and place.
    fn found(p: &Project) -> Vec<(&'static str, bool, usize, Pt)> {
        p.erc().into_iter().map(|v| (v.kind, v.error, v.page, v.at)).collect()
    }

    #[test]
    fn checks_a_sheet_as_kicad_does() {
        let nc = |x: f64, y: f64| format!("(no_connect (at {x} {y}))");
        let items = [
            // An open pin (one with a no-connect flag is fine, a free pin too).
            r("R1", 10.0, 10.0),
            wire(15.0, 20.0, 10.0, None),
            r("R2", 20.0, 10.0),
            nc(25.0, 10.0),
            part("FREE", "U9", "", 40.0, 10.0),
            // Two inputs that nothing drives; two power inputs nothing powers; a power flag does.
            part("IN", "U1", "", 10.0, 20.0),
            wire(10.0, 20.0, 20.0, None),
            part("IN", "U2", "", 20.0, 20.0),
            part("PI", "U3", "", 10.0, 30.0),
            wire(10.0, 20.0, 30.0, None),
            part("PI", "U4", "", 20.0, 30.0),
            part("PI", "U5", "", 10.0, 40.0),
            wire(10.0, 20.0, 40.0, None),
            part("FLAG", "#FLG1", "PWR_FLAG", 20.0, 40.0),
            // Two outputs on one net.
            part("OUT", "U6", "", 10.0, 50.0),
            wire(10.0, 20.0, 50.0, None),
            part("OUT", "U7", "", 20.0, 50.0),
            // A flag on nothing; one on a pin wired to another.
            nc(60.0, 10.0),
            r("R5", 40.0, 20.0),
            wire(45.0, 50.0, 20.0, None),
            r("R6", 50.0, 20.0),
            nc(40.0, 20.0) + &nc(45.0, 20.0) + &nc(55.0, 20.0),
            // A label alone; one with a single pin.
            r#"(label "DANG" (at 60 30 0))"#.into(),
            wire(35.0, 40.0, 40.0, Some(("label", "ISO"))),
            r("R7", 40.0, 40.0),
            nc(45.0, 40.0),
            // A wire alone; a wire ending on another's middle without a junction.
            wire(60.0, 70.0, 50.0, None),
            r("R8", 40.0, 60.0),
            nc(40.0, 60.0),
            wire(45.0, 55.0, 60.0, None),
            r("R9", 55.0, 60.0),
            nc(60.0, 60.0),
            "(wire (pts (xy 50 65) (xy 50 60)))".into(),
            r("R10", 50.0, 65.0),
            nc(55.0, 65.0),
            // Two names on one net.
            r("R11", 40.0, 80.0),
            wire(45.0, 55.0, 80.0, None),
            r#"(label "NB" (at 52 80 0)) (label "NA" (at 48 80 0))"#.into(),
            r("R12", 55.0, 80.0),
            nc(40.0, 80.0) + &nc(60.0, 80.0),
            // Two parts of one name, one without a number.
            part("LOGO", "LOGO1", "", 100.0, 10.0),
            part("LOGO", "LOGO1", "", 100.0, 20.0),
            part("LOGO", "LOGO?", "", 100.0, 30.0),
        ];
        let p = project(&[("root.kicad_sch", items.concat())], "");
        assert_eq!(
            found(&p),
            [
                ("duplicate_reference", true, 0, mm(100.0, 20.0)),
                ("label_dangling", true, 0, mm(60.0, 30.0)),
                ("pin_not_connected", true, 0, mm(10.0, 10.0)),
                ("pin_not_connected", true, 0, mm(50.0, 65.0)),
                ("pin_not_driven", true, 0, mm(10.0, 20.0)),
                ("pin_to_pin", true, 0, mm(20.0, 50.0)),
                ("power_pin_not_driven", true, 0, mm(10.0, 30.0)),
                ("unannotated", true, 0, mm(100.0, 30.0)),
                ("wire_dangling", true, 0, mm(60.0, 50.0)),
                ("isolated_pin_label", false, 0, mm(35.0, 40.0)),
                ("multiple_net_names", false, 0, mm(48.0, 80.0)),
                ("no_connect_connected", false, 0, mm(45.0, 20.0)),
                ("no_connect_dangling", false, 0, mm(60.0, 10.0)),
                ("unconnected_wire_endpoint", false, 0, mm(50.0, 60.0)),
            ]
        );
        // In plain words: what, and how to fix it.
        let v = p.erc();
        let text = |kind| v.iter().find(|v| v.kind == kind).map(|v| format!("{} {}", v.what, v.fix)).unwrap();
        assert_eq!(
            text("power_pin_not_driven"),
            "U3 pin 1 (VDD) takes power, but no pin on its net is a power output to give it. If the power comes from a connector, a \
             battery or a part whose pins aren't power outputs, put a PWR_FLAG on the net; otherwise connect the supply."
        );
        assert_eq!(
            text("pin_to_pin"),
            "U6 pin 1 (Q), an output, and U7 pin 1 (Q), an output, drive one net against each other. Leave one output on the net; \
             if a pin is really an input, fix its type in the symbol."
        );
        assert_eq!(text("multiple_net_names").split(". ").next(), Some("One net has 2 names, NA and NB; it takes NA"));
        // The project's severities: an ignored check isn't made, another made a warning.
        let pro = r#"{"erc": {"rule_severities": {"pin_to_pin": "ignore", "pin_not_connected": "warning"}}}"#;
        let p = project(&[("root.kicad_sch", items.concat())], pro);
        let v = found(&p);
        assert!(!v.iter().any(|v| v.0 == "pin_to_pin") && v.iter().filter(|v| v.0 == "pin_not_connected").all(|v| !v.1));
    }

    #[test]
    fn checks_sheets_buses_and_names() {
        let nc = |x: f64, y: f64| format!("(no_connect (at {x} {y}))");
        let sheet = r#"(sheet (at 100 0) (size 20 30) (uuid "s") (property "Sheetname" "Sub" (at 0 0 0))
          (property "Sheetfile" "sub.kicad_sch" (at 0 0 0)) (pin "A" input (at 100 10 180)) (pin "B" input (at 100 20 180)))"#;
        let root = [
            sheet.to_string(),
            r("R1", 85.0, 10.0),
            wire(90.0, 100.0, 10.0, None),
            nc(85.0, 10.0),
            r("R2", 85.0, 20.0),
            wire(90.0, 100.0, 20.0, None),
            nc(85.0, 20.0),
        ];
        let sub = [
            // A, inside as outside; C without a sheet pin (alone with one pin); B missing.
            wire(10.0, 20.0, 10.0, Some(("hierarchical_label", "A"))),
            r("R10", 20.0, 10.0),
            nc(25.0, 10.0),
            wire(10.0, 20.0, 20.0, Some(("hierarchical_label", "C"))),
            r("R11", 20.0, 20.0),
            nc(25.0, 20.0),
            // A global and a local label G: one net here, not elsewhere.
            wire(10.0, 20.0, 30.0, Some(("global_label", "G"))),
            r("R12", 20.0, 30.0),
            wire(10.0, 20.0, 40.0, Some(("label", "G"))),
            r("R13", 20.0, 40.0),
            nc(25.0, 30.0) + &nc(25.0, 40.0),
            // abc and ABC: two nets.
            wire(10.0, 20.0, 50.0, Some(("label", "abc"))),
            r("R14", 20.0, 50.0),
            wire(25.0, 35.0, 50.0, None),
            r#"(label "abc" (at 35 50 0))"#.into(),
            wire(10.0, 20.0, 60.0, Some(("label", "ABC"))),
            r("R15", 20.0, 60.0),
            wire(25.0, 35.0, 60.0, None),
            r#"(label "ABC" (at 35 60 0))"#.into(),
            // X enters bus D[0..1] by an entry, but isn't a member.
            r#"(bus (pts (xy 40 70) (xy 40 90))) (label "D[0..1]" (at 40 80 0)) (bus_entry (at 40 75) (size 2.5 2.5))"#.into(),
            wire(42.5, 50.0, 77.5, Some(("label", "X"))),
            r("R16", 50.0, 77.5),
            wire(55.0, 60.0, 77.5, None),
            r#"(label "X" (at 60 77.5 0))"#.into(),
        ];
        let p = project(&[("root.kicad_sch", root.concat()), ("sub.kicad_sch", sub.concat())], "");
        assert_eq!(
            found(&p),
            [
                ("hier_label_mismatch", true, 0, mm(100.0, 20.0)),
                ("hier_label_mismatch", true, 1, mm(10.0, 20.0)),
                ("isolated_pin_label", false, 1, mm(10.0, 20.0)),
                ("net_not_bus_member", false, 1, mm(42.5, 77.5)),
                ("same_local_global_label", false, 1, mm(10.0, 40.0)),
                ("similar_labels", false, 1, mm(10.0, 60.0)),
            ]
        );
    }

    #[test]
    fn names_the_sheet_a_reference_was_used_on_first() {
        // LOGO1 on the root sheet, and again on the sheet below it.
        let sheet = r#"(sheet (at 100 0) (size 20 30) (uuid "s") (property "Sheetname" "Sub" (at 0 0 0))
          (property "Sheetfile" "sub.kicad_sch" (at 0 0 0)))"#;
        let logo = part("LOGO", "LOGO1", "", 10.0, 10.0);
        let p = project(&[("root.kicad_sch", sheet.to_string() + &logo), ("sub.kicad_sch", logo)], "");
        let twice: Vec<_> = p.erc().into_iter().filter(|v| v.kind == "duplicate_reference").map(|v| (v.page, v.what)).collect();
        assert_eq!(twice, [(1, "Two parts are named LOGO1: this one and one on the root sheet.".to_string())]);
    }

    #[test]
    fn tells_a_net_of_two_names_at_the_same_place_each_time() {
        // Two wires apart, each named NA and NB: one net, told once, at the first wire's NA.
        let named = |y: f64| wire(10.0, 20.0, y, Some(("label", "NA"))) + &format!(r#"(label "NB" (at 20 {y} 0))"#);
        let p = project(&[("root.kicad_sch", named(10.0) + &named(20.0))], "");
        for _ in 0..16 {
            let told: Vec<_> = p.erc().into_iter().filter(|v| v.kind == "multiple_net_names").map(|v| v.at).collect();
            assert_eq!(told, [mm(10.0, 10.0)]);
        }
    }
}
