//! Updating a board from its schematic, as KiCad's Update PCB from Schematic (F8) does: the
//! parts the board lacks are added, those on it take their symbols' footprints, values and
//! fields, and the parts' pads take the nets the schematic puts their pins on
//! (`crates/core/update.md`).

use crate::board::{Board, Footprint, MM, NetId, Nm, Pad, Pt, Src, shape_box, union};
use crate::bom::ref_key;
use crate::history::Patch;
use crate::save::{KICAD6, KICAD8};
use crate::schematic::{Project, Symbol};
use crate::tr::trf;
use std::collections::{HashMap, HashSet};

/// The schematic's parts by reference: each one's first unit, with the file of its sheet.
type Parts<'a> = HashMap<&'a str, (&'a Symbol, &'a str)>;

/// What `Board::update` changed, and what it couldn't.
#[derive(Debug, Default, PartialEq)]
pub struct Updated {
    /// Parts added, in rows below the board.
    pub parts: usize,
    /// Parts swapped for the footprint their symbol names.
    pub footprints: usize,
    /// Parts given their symbol's value.
    pub values: usize,
    /// Fields (MPN, LCSC…) parts took from their symbols, changed or new.
    pub fields: usize,
    /// Nets renamed, their tracks, vias and zones with them.
    pub renamed: usize,
    /// Pads put on another net (but the parts added).
    pub moved: usize,
    /// Nets new to the board.
    pub added: usize,
    /// The schematic's parts the board still lacks or couldn't swap, each with why and what to do.
    pub missing: Vec<String>,
}

impl Board {
    /// Updates the board from `project`'s schematic: puts the parts whose symbol names another
    /// footprint as that one, adds the parts it lacks (`footprint` gives the `.kicad_mod` text of
    /// a `Library:Footprint`), gives them their symbols' values and fields, then puts the pads of
    /// its parts on the nets their pins are on (`Project::netlist`), matched by reference and
    /// number; a pad without a pin gets no net. A net takes the name most of its pads go to, its
    /// tracks, vias and zones with it, unless a net with more pads for it or one that keeps its
    /// own name has it. Returns the patch that undoes it (`None` when nothing changed) and what
    /// changed.
    pub fn update(&mut self, project: &Project, footprint: impl Fn(&str) -> Result<String, String>) -> (Option<Patch>, Updated) {
        // Parts not numbered yet (`R?`) have no footprint to match; KiCad links a footprint to
        // any unit of its part. Each with its sheet's file.
        let mut parts: Parts = HashMap::new();
        for p in &project.pages {
            for s in p.sheet.symbols.iter().filter(|s| s.on_board() && !s.designator.ends_with('?')) {
                parts.entry(s.designator.as_str()).or_insert((s, p.file.as_str()));
            }
        }
        // The parts whose symbol names another footprint: the undo keeps the graphics from the
        // first of them on.
        let other =
            |f: &Footprint| parts.get(f.reference.as_str()).is_some_and(|(s, _)| !s.footprint.is_empty() && s.footprint != f.lib_id);
        let swaps: Vec<usize> = (0..self.footprints.len()).filter(|&i| other(&self.footprints[i])).collect();
        let from = swaps.iter().map(|&i| self.footprints[i].shapes.start).min().unwrap_or(self.shapes.len());
        let (undo, mut done, had) = (Patch::update(self, from), Updated::default(), self.footprints.len());
        let mut fresh = self.swap_parts(&swaps, &parts, &footprint, &mut done);
        self.add_parts(&parts, &footprint, &mut done);
        fresh.extend(had as u32..self.footprints.len() as u32);
        self.take_fields(project, &parts, &mut done);
        self.take_nets(project, &parts, &fresh, &mut done);
        let changed = done.parts + done.footprints + done.values + done.fields + done.renamed + done.moved + done.added > 0;
        (changed.then_some(undo), done)
    }

    /// Puts each part of `swaps` as the footprint its symbol names, from KiCad's libraries, in its
    /// place: the same anchor, angle and side (flipped onto the back), reference, value, fields,
    /// link, attributes and texts as placed (KiCad users place their references); the library's
    /// pads, graphics and 3D models. A part added from a library this session becomes the new one
    /// whole. Tells why of those it can't; returns the parts swapped.
    fn swap_parts(
        &mut self,
        swaps: &[usize],
        parts: &Parts,
        footprint: &impl Fn(&str) -> Result<String, String>,
        done: &mut Updated,
    ) -> HashSet<u32> {
        let mut swapped = HashSet::new();
        for &i in swaps {
            let (old, (s, _)) = (&self.footprints[i], parts[self.footprints[i].reference.as_str()]);
            let read = footprint_of(s, footprint).and_then(|(text, lib)| {
                let read = self.library_part(&text, lib, &old.reference, Some((&old.value, &s.link)));
                read.map_err(|e| trf("{0}'s footprint {1} doesn't read: {2}", &[&s.designator, &s.footprint, &e.msg]))
            });
            let (mut part, text) = match read {
                Ok(read) => read,
                Err(why) => {
                    done.missing.push(why);
                    continue;
                }
            };
            if old.back {
                part.flip_footprint(0);
            }
            part.place_footprint(0, old.at, old.angle);
            let new = &part.footprints[0];
            let (texts, f) = match &old.src {
                Src::Library(_) => (part.texts.clone(), Footprint { src: Src::Library(text), ..new.clone() }),
                Src::File(k) | Src::Swapped(k, _) => {
                    // KiCad 8 keeps the footprint's name as a field too.
                    let fields = old.fields.iter().map(|(k, v)| (k.clone(), if k == "Footprint" { new.lib_id.clone() } else { v.clone() }));
                    let (lib_id, bounds, models) = (new.lib_id.clone(), new.bounds, new.models.clone());
                    let f = Footprint { lib_id, bounds, models, fields: fields.collect(), src: Src::Swapped(*k, text), ..old.clone() };
                    (self.texts[old.texts.clone()].to_vec(), f)
                }
            };
            let (p, g, t) = (old.pads.start, old.shapes.start, old.texts.start);
            let f = Footprint { pads: p..p + part.pads.len(), shapes: g..g + part.shapes.len(), texts: t..t + texts.len(), ..f };
            let pads = part.pads.iter().map(|q| Pad { footprint: i as u32, net: 0, ..q.clone() }).collect();
            self.apply(Patch::part(self, i, f, pads, part.shapes, texts));
            (self.layers, done.footprints) = (part.layers, done.footprints + 1);
            swapped.insert(i as u32);
        }
        swapped
    }

    /// Adds the footprints of the `parts` the board lacks, named, valued and linked as their
    /// symbols, in rows below the board, from its left: a sheet's parts together, as they go
    /// together on the board, each clear of the next's courtyard, its anchor on a 0.1 mm grid;
    /// tells why of those it can't.
    fn add_parts(&mut self, parts: &Parts, footprint: &impl Fn(&str) -> Result<String, String>, done: &mut Updated) {
        const GRID: Nm = MM / 10;
        let on_board: HashSet<&str> = self.footprints.iter().map(|f| f.reference.as_str()).collect();
        /// Where a part's sheet is used: its file, then its place below the root (a sheet used twice).
        fn sheet<'a>(&(s, file): &(&'a Symbol, &'a str)) -> (&'a str, &'a str) {
            (file, s.link.rsplit_once('/').map_or("", |(sheets, _)| sheets))
        }
        let mut lacking: Vec<(&Symbol, &str)> =
            parts.values().copied().filter(|(s, _)| !on_board.contains(s.designator.as_str())).collect();
        lacking.sort_unstable_by_key(|part| (sheet(part), ref_key(&part.0.designator)));
        let edge = self.layers.iter().position(|l| l == "Edge.Cuts");
        let outline = self.shapes.iter().filter(|s| Some(s.layer as usize) == edge).map(shape_box).reduce(union);
        let (lo, hi) = outline.or_else(|| self.footprints.iter().map(|f| f.bounds).reduce(union)).unwrap_or_default();
        let (mut x, mut y, mut row, mut last) = (lo.x, hi.y.saturating_add(5 * MM), 0, None);
        for part in lacking {
            let s = part.0;
            let added = footprint_of(s, footprint).and_then(|(text, lib)| {
                let text = flagged(&text, s.in_bom, s.dnp);
                let i = self.add_part(&text, lib, &s.designator, Some((&s.value, &s.link)), Pt::default(), 0.0);
                i.map_err(|e| trf("{0}'s footprint {1} doesn't read: {2}", &[&s.designator, &s.footprint, &e.msg]))
            });
            match added {
                Ok(i) => {
                    // Its room: its pads and its courtyard, which may reach past them (a connector's).
                    let f = &self.footprints[i];
                    let (a, b) = self.drawn(f, |l| l.ends_with(".CrtYd")).map_or(f.bounds, |yard| union(f.bounds, yard));
                    // A row as wide as the board, its parts 1 mm apart; another sheet's start one, 3 mm down.
                    let next = last.is_some_and(|l| l != sheet(&part));
                    if x > lo.x && (next || x + (b.x - a.x) > hi.x) {
                        (x, y, row) = (lo.x, y + row + if next { 3 * MM } else { MM }, 0);
                    }
                    let up = |v: Nm| v.div_euclid(GRID) * GRID + if v.rem_euclid(GRID) == 0 { 0 } else { GRID };
                    let at = Pt { x: up(x - a.x), y: up(y - a.y) };
                    self.place_footprint(i, at, 0.0);
                    (x, row, last, done.parts) = (at.x + b.x + MM, row.max(at.y + b.y - y), Some(sheet(&part)), done.parts + 1);
                }
                Err(why) => done.missing.push(why),
            }
        }
    }

    /// Gives the board's `parts` their symbols' values and fields, as each KiCad version keeps
    /// fields on its boards (as the corpus boards it updated have them): none in KiCad 5; a
    /// symbol's own fields from KiCad 6, its Datasheet too from KiCad 8. KiCad 6 and 7 write an
    /// empty field `~`. The part's texts showing a value that changed show the new one.
    // shortcut: a text showing a field among other words (`${REFERENCE}: ${VALUE}`) shows the old
    // value until the board is read again; keep texts' templates when boards need it. A field
    // added (saved hidden) is among the part's hidden texts, to show, once the board is read again.
    fn take_fields(&mut self, project: &Project, parts: &Parts, done: &mut Updated) {
        let (version, empty) = (self.version, |v: &str| if v == "~" { String::new() } else { v.to_string() });
        let carried = |name: &str| {
            !matches!(name, "Reference" | "Value" | "Footprint") && !name.starts_with("ki_") && (name != "Datasheet" || version >= KICAD8)
        };
        for i in 0..self.footprints.len() {
            let Some(&(s, file)) = parts.get(self.footprints[i].reference.as_str()) else { continue };
            let theirs = if version < KICAD6 { vec![] } else { project.symbol_fields(file, s) };
            let f = &mut self.footprints[i];
            // Each text a value changed from, and what it changed to.
            let mut changed: Vec<(String, String)> = vec![];
            if empty(&f.value) != empty(&s.value) {
                changed.push((std::mem::replace(&mut f.value, empty(&s.value)), empty(&s.value)));
                done.values += 1;
            }
            for (name, value) in theirs.into_iter().filter(|(name, _)| carried(name)) {
                let value = empty(&value);
                match f.fields.iter_mut().find(|(k, _)| *k == name) {
                    Some((_, v)) if empty(v) == value => continue,
                    Some((_, v)) => changed.push((std::mem::replace(v, value.clone()), value)),
                    None => f.fields.push((name, value)),
                }
                done.fields += 1;
            }
            for t in self.texts[f.texts.clone()].iter_mut().chain(&mut f.hidden) {
                if let Some((_, new)) = changed.iter().find(|(old, _)| *old == t.text) {
                    t.text = new.clone();
                }
            }
        }
    }

    /// Puts the pads of the board's `parts` on the nets the schematic puts their pins on, as
    /// `update` says; those of parts but the `fresh` ones (added or swapped) count as moved.
    fn take_nets(&mut self, project: &Project, parts: &Parts, fresh: &HashSet<u32>, done: &mut Updated) {
        let netlist = project.netlist();
        let pins: HashMap<(&str, &str), &str> =
            netlist.iter().flat_map(|(net, pins)| pins.iter().map(move |(r, n)| ((r.as_str(), n.as_str()), net.as_str()))).collect();
        // Each pad of a part the schematic has, with the name of the net it goes on ("": none).
        let mut to: Vec<(usize, String)> = vec![];
        for f in self.footprints.iter().filter(|f| parts.contains_key(f.reference.as_str())) {
            let net = |i: usize| pins.get(&(f.reference.as_str(), self.pads[i].number.as_str())).copied().unwrap_or_default();
            to.extend(f.pads.clone().map(|i| (i, net(i).to_string())));
        }
        self.apart(&mut to);
        // A net takes the name most of its pads go to, a name the net with most pads for it (then
        // the lower id), while no net keeping its own has it.
        let mut votes: HashMap<(NetId, &str), usize> = HashMap::new();
        for (i, net) in to.iter().filter(|(i, net)| self.pads[*i].net != 0 && !net.is_empty()) {
            *votes.entry((self.pads[*i].net, net.as_str())).or_default() += 1;
        }
        let mut votes: Vec<_> = votes.into_iter().collect();
        votes.sort_unstable_by(|(a, n), (b, m)| m.cmp(n).then(a.cmp(b)));
        let (mut renamed, mut taken) = (HashMap::new(), HashSet::new());
        for ((id, net), _) in votes {
            if !renamed.contains_key(&id) && taken.insert(net) {
                renamed.insert(id, net);
            }
        }
        renamed.retain(|&id, net| self.nets[id as usize] != *net);
        loop {
            let kept: HashSet<&str> =
                self.nets.iter().enumerate().filter(|(id, _)| !renamed.contains_key(&(*id as NetId))).map(|(_, n)| n.as_str()).collect();
            let n = renamed.len();
            renamed.retain(|_, net| !kept.contains(net));
            if renamed.len() == n {
                break;
            }
        }
        done.renamed = renamed.len();
        for (id, net) in renamed {
            self.nets[id as usize] = net.to_string();
        }
        let mut ids: HashMap<String, NetId> = HashMap::new();
        for (id, n) in self.nets.iter().enumerate() {
            ids.entry(n.clone()).or_insert(id as NetId);
        }
        for (i, net) in to {
            let id = *ids.entry(net).or_insert_with_key(|net| {
                self.nets.push(net.clone());
                done.added += 1;
                (self.nets.len() - 1) as NetId
            });
            if self.pads[i].net != id {
                self.pads[i].net = id;
                done.moved += usize::from(!fresh.contains(&self.pads[i].footprint));
            }
        }
    }

    /// KiCad 8 gives each pad of an `unconnected-(…)` net a net of its own, the first its name,
    /// the others `…_1`, `…_2` (KiCad 7 one each pin): a pad on one of them keeps it, the others
    /// take free ones in turn.
    fn apart(&self, to: &mut [(usize, String)]) {
        let mut on: HashMap<String, Vec<usize>> = HashMap::new();
        for (k, (_, net)) in to.iter().enumerate().filter(|(_, (_, net))| net.starts_with("unconnected-(")) {
            on.entry(net.clone()).or_default().push(k);
        }
        for (net, pads) in on.into_iter().filter(|(_, pads)| pads.len() > 1) {
            // Each pad's place in `to`, with the name of the net it is on.
            let pads: Vec<(usize, &str)> = pads.into_iter().map(|k| (k, self.nets[self.pads[to[k].0].net as usize].as_str())).collect();
            let ours = |name: &str| {
                name.strip_prefix(net.as_str())
                    .is_some_and(|s| s.is_empty() || s.strip_prefix('_').is_some_and(|n| n.parse::<u32>().is_ok()))
            };
            let used: HashSet<&str> = pads.iter().map(|&(_, now)| now).filter(|now| ours(now)).collect();
            let mut free = (0..).map(|n| if n == 0 { net.clone() } else { format!("{net}_{n}") }).filter(|n| !used.contains(n.as_str()));
            for (k, now) in pads {
                to[k].1 = if ours(now) { now.to_string() } else { free.next().expect("names without end") };
            }
        }
    }
}

/// The `.kicad_mod` text of the footprint symbol `s` names (`footprint` reads `Library:Name`s)
/// with its library, or why there is none and what to do.
fn footprint_of<'a>(s: &'a Symbol, footprint: &impl Fn(&str) -> Result<String, String>) -> Result<(String, &'a str), String> {
    let (r, fp) = (&s.designator, &s.footprint);
    match fp.split_once(':') {
        _ if fp.is_empty() => Err(trf("{0} has no footprint: set its Footprint field (KiCad's Assign Footprints)", &[r])),
        None => Err(trf("{0}'s footprint {1} names no library: write it Library:Footprint", &[r, fp])),
        Some((lib, _)) => footprint(fp).map(|text| (text, lib)).map_err(|e| format!("{r}: {e}")),
    }
}

/// Footprint `text` with the attributes KiCad's F8 gives a symbol's part: excluded from the bill
/// of materials unless the symbol is in it (`in_bom`), not populated when it isn't (`dnp`).
// shortcut: parts already on the board keep their attributes; follow the symbols there too when
// boards need it.
fn flagged(text: &str, in_bom: bool, dnp: bool) -> String {
    let wanted = [("exclude_from_bom", !in_bom), ("dnp", dnp)];
    let add = wanted.iter().filter(|w| w.1).map(|w| w.0);
    if let Some(at) = text.find("(attr ").or_else(|| text.find("(attr)")) {
        let end = at + text[at..].find(')').unwrap_or_default();
        let kept = text[at + 5..end].split_whitespace().filter(|w| !wanted.iter().any(|f| f.0 == *w));
        let words: Vec<&str> = kept.chain(add).collect();
        return format!("{}(attr {}){}", &text[..at], words.join(" "), &text[end + 1..]);
    }
    let words: Vec<&str> = add.collect();
    // None yet: after the footprint's layer.
    match text.find("(layer ").map(|at| at + text[at..].find(')').unwrap_or_default() + 1) {
        Some(end) if !words.is_empty() => format!("{} (attr {}){}", &text[..end], words.join(" "), &text[end..]),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project of one sheet: resistors (pins 2.54 mm left and right of their anchor), `items`.
    fn project(items: &str) -> Project {
        let sheet = format!(
            r#"(kicad_sch (version 20250114) (uuid "root") (lib_symbols (symbol "Device:R" (symbol "R_1_1"
              (pin passive line (at -2.54 0 0) (length 1.27) (name "") (number "1"))
              (pin passive line (at 2.54 0 180) (length 1.27) (name "") (number "2"))))) {items})"#
        );
        Project::load("t.kicad_sch", "", |_| Ok(sheet.clone())).unwrap()
    }

    fn r(reference: &str, x: f64, y: f64) -> String {
        format!(r#"(symbol (lib_id "Device:R") (at {x} {y} 0) (unit 1) (property "Reference" "{reference}" (at 0 0 0)))"#)
    }

    /// A board's text with resistors `(reference, [(pad, net)])`, its nets numbered as up to KiCad 9.
    fn board(nets: &[&str], parts: &[(&str, &[(&str, u32)])], rest: &str) -> String {
        let nets: String = nets.iter().enumerate().map(|(i, n)| format!(r#"(net {i} "{n}")"#)).collect();
        let parts: String = (parts.iter())
            .map(|(r, pads)| {
                let pads: String = pads
                    .iter()
                    .map(|(n, net)| format!(r#"(pad "{n}" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net {net}))"#))
                    .collect();
                format!(r#"(footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "{r}" (at 0 0 0) (layer "F.Cu")) {pads})"#)
            })
            .collect();
        format!(r#"(kicad_pcb (version 20240108) (layers (0 "F.Cu" signal)) {nets} {parts} {rest})"#)
    }

    /// A library with one footprint, `Test:R`.
    fn library(id: &str) -> Result<String, String> {
        let pads = r#"(pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu")) (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu"))"#;
        let fp = r#"(footprint "R" (layer "F.Cu") (property "Reference" "REF**" (at 0 -1 0)) (property "Value" "R" (at 0 1 0))"#;
        (id == "Test:R").then(|| format!("{fp} {pads})")).ok_or(format!("{id} isn't in KiCad's libraries"))
    }

    #[test]
    fn gives_a_part_its_symbols_bom_and_dnp_flags() {
        let tp = r#"(footprint "TP" (layer "F.Cu") (attr exclude_from_pos_files exclude_from_bom) (pad "1" smd circle))"#;
        assert_eq!(flagged(tp, true, false), r#"(footprint "TP" (layer "F.Cu") (attr exclude_from_pos_files) (pad "1" smd circle))"#);
        assert_eq!(
            flagged(tp, false, true),
            r#"(footprint "TP" (layer "F.Cu") (attr exclude_from_pos_files exclude_from_bom dnp) (pad "1" smd circle))"#
        );
        let bare = r#"(footprint "R" (layer "F.Cu") (pad "1" smd rect))"#;
        assert_eq!(flagged(bare, true, false), bare);
        assert_eq!(flagged(bare, false, false), r#"(footprint "R" (layer "F.Cu") (attr exclude_from_bom) (pad "1" smd rect))"#);
    }

    #[test]
    fn puts_new_parts_clear_of_each_other_on_the_grid_a_sheet_together() {
        // Connectors whose courtyard reaches 3 mm past their pads: J2 on the root, J1 and J3 on sheet A.
        let j = |name: &str| {
            format!(
                r#"(symbol (lib_id "Device:R") (at 50 10 0) (unit 1) (uuid "u{name}") (property "Reference" "{name}" (at 0 0 0))
                  (property "Footprint" "Test:J" (at 0 0 0)))"#
            )
        };
        let lib =
            r#"(lib_symbols (symbol "Device:R" (symbol "R_1_1" (pin passive line (at -2.54 0 0) (length 1.27) (name "") (number "1")))))"#;
        let root = format!(
            r#"(kicad_sch (version 20250114) (uuid "root") {lib} {} (sheet (at 100 100) (size 10 10) (uuid "s1")
              (property "Sheetname" "A" (at 0 0 0)) (property "Sheetfile" "a.kicad_sch" (at 0 0 0))))"#,
            j("J2")
        );
        let a = format!(r#"(kicad_sch (version 20250114) (uuid "a") {lib} {} {})"#, j("J1"), j("J3"));
        let p = Project::load("t.kicad_sch", "", |f| Ok(if f == "a.kicad_sch" { a.clone() } else { root.clone() })).unwrap();
        let yard = r#"(fp_rect (start -4.03 -3) (end 4.03 3) (stroke (width 0.05) (type solid)) (fill no) (layer "F.CrtYd"))"#;
        let fields = r#"(property "Reference" "REF**" (at 0 -1 0)) (property "Value" "J" (at 0 1 0))"#;
        let part = format!(r#"(footprint "J" (layer "F.Cu") {fields} (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu")) {yard})"#);
        let outline = r#"(gr_rect (start 0 0) (end 30.033 20) (stroke (width 0.05) (type solid)) (fill no) (layer "Edge.Cuts"))"#;
        let layers = r#"(layers (0 "F.Cu" signal) (25 "Edge.Cuts" user) (31 "F.CrtYd" user))"#;
        let mut b = Board::parse(&format!("(kicad_pcb (version 20240108) {layers} {outline})")).unwrap();
        let (_, done) = b.update(&p, |_| Ok(part.clone()));
        assert_eq!((done.parts, &done.missing[..]), (3, &[][..]));
        let at = |name: &str| b.footprints.iter().find(|f| f.reference == name).unwrap().at;
        let (j1, j2, j3) = (at("J1"), at("J2"), at("J3"));
        // Sheet A's two in the first row, their 8.11 mm courtyards 1 mm apart; the root's below
        // them, 3 mm clear; every anchor on the 0.1 mm grid.
        assert!(j1.y == j3.y && j3.x - j1.x >= 9_110_000 && j3.x - j1.x < 9_310_000, "{j1:?} {j3:?}");
        assert!(j2.x == j1.x && j2.y - j1.y >= 9_050_000 && j2.y - j1.y < 9_250_000, "{j1:?} {j2:?}");
        assert!([j1, j2, j3].iter().all(|p| p.x % 100_000 == 0 && p.y % 100_000 == 0), "{j1:?} {j2:?} {j3:?}");
        assert!(crate::drc::check(&b).iter().all(|v| v.kind != "courtyards_overlap"));
    }

    fn pad_nets(b: &Board) -> Vec<&str> {
        b.pads.iter().map(|p| b.nets[p.net as usize].as_str()).collect()
    }

    #[test]
    fn adds_the_parts_and_takes_the_nets_renaming_those_whose_pads_all_go() {
        // R1.1 and R2.1 on a net labelled SDA; R1.2 and R2.2 on one no label names; R3, alone
        // and not on the board yet, has a footprint, R4 none.
        let r3 = r#"(symbol (lib_id "Device:R") (at 50 10 0) (unit 1) (uuid "u3") (property "Reference" "R3" (at 0 0 0))
          (property "Value" "10k" (at 0 0 0)) (property "Footprint" "Test:R" (at 0 0 0)))"#;
        let p = project(
            &[
                r("R1", 10.0, 10.0),
                r("R2", 30.0, 10.0),
                "(wire (pts (xy 7.46 10) (xy 7.46 20))) (wire (pts (xy 7.46 20) (xy 27.46 20))) (wire (pts (xy 27.46 20) (xy 27.46 10)))"
                    .into(),
                r#"(label "SDA" (at 15 20 0))"#.into(),
                "(wire (pts (xy 12.54 10) (xy 12.54 5))) (wire (pts (xy 12.54 5) (xy 32.54 5))) (wire (pts (xy 32.54 5) (xy 32.54 10)))"
                    .into(),
                r3.into(),
                r("R4", 70.0, 10.0),
            ]
            .concat(),
        );
        // A, with a track on it, goes whole to SDA; B and C join; R2's pad 3 has no pin.
        let track = r#"(segment (start 0 0) (end 1 0) (width 0.25) (layer "F.Cu") (net 1))"#;
        let src = board(&["", "A", "B", "C"], &[("R1", &[("1", 1), ("2", 2)]), ("R2", &[("1", 1), ("2", 3), ("3", 3)])], track);
        let mut b = Board::parse(&src).unwrap();
        let before = (b.nets.clone(), b.pads.clone(), b.footprints.clone());
        let (undo, done) = b.update(&p, library);
        let missing = vec!["R4 has no footprint: set its Footprint field (KiCad's Assign Footprints)".to_string()];
        assert_eq!(done, Updated { parts: 1, renamed: 2, moved: 2, added: 2, missing, ..Updated::default() });
        let r3 = &b.footprints[2];
        assert_eq!(pad_nets(&b), ["/SDA", "Net-(R1-Pad2)", "/SDA", "Net-(R1-Pad2)", "", "unconnected-(R3-Pad1)", "unconnected-(R3-Pad2)"]);
        assert_eq!((b.nets[b.tracks[0].net as usize].as_str(), b.nets[3].as_str()), ("/SDA", "C"));
        // R3 below the parts, its box's corner under theirs, 5 mm down, as far on as puts its anchor
        // on the 0.1 mm grid; saved, it keeps its nets and links to its symbol.
        assert_eq!((r3.reference.as_str(), r3.value.as_str(), r3.lib_id.as_str()), ("R3", "10k", "Test:R"));
        let (lo, hi) = b.footprints[0].bounds;
        let (dx, dy) = (r3.bounds.0.x - lo.x, r3.bounds.0.y - hi.y - 5 * MM);
        assert!((0..MM / 10).contains(&dx) && (0..MM / 10).contains(&dy), "{:?}", r3.bounds);
        assert_eq!((r3.at.x % (MM / 10), r3.at.y % (MM / 10)), (0, 0));
        let out = b.write(&src).unwrap();
        assert!(out.contains(r#"(path "/u3")"#), "{out}");
        assert_eq!(pad_nets(&Board::parse(&out).unwrap()), pad_nets(&b));
        // Again: nothing to do. Undone: as it was.
        assert!(b.update(&p, library).0.is_none());
        b.apply(undo.unwrap());
        assert_eq!((b.nets, b.pads, b.footprints), before);
    }

    #[test]
    fn keeps_the_pads_of_a_no_connect_net_apart() {
        // R5.1, flagged no-connect, wired to R6.1: KiCad gives each its own net.
        let p = project(
            &[r("R5", 10.0, 10.0), r("R6", 10.0, 20.0), "(wire (pts (xy 7.46 10) (xy 7.46 20))) (no_connect (at 7.46 10))".into()].concat(),
        );
        let mut b = Board::parse(&board(&["", "unconnected-(R5-Pad1)_1"], &[("R5", &[("1", 1)]), ("R6", &[("1", 0)])], "")).unwrap();
        let (_, done) = b.update(&p, library);
        assert_eq!((done.renamed, done.moved, done.added), (0, 1, 1));
        assert_eq!(pad_nets(&b), ["unconnected-(R5-Pad1)_1", "unconnected-(R5-Pad1)"]);
    }

    #[test]
    fn carries_values_and_fields_as_each_kicad_version_keeps_them() {
        // R1's symbol says 4k7, a part number and an LCSC number; the board's R1 is a 10k with
        // another part number. KiCad 7 keeps no Datasheet on parts, KiCad 8 on does; KiCad 10 adds
        // a field hidden on the part's side's Fab layer, at its anchor, turned with it.
        let p = project(
            r#"(symbol (lib_id "Device:R") (at 10 10 0) (unit 1) (property "Reference" "R1" (at 0 0 0)) (property "Value" "4k7" (at 0 0 0))
              (property "Datasheet" "r.pdf" (at 0 0 0)) (property "MPN" "RC0603FR-074K7L" (at 0 0 0)) (property "LCSC" "C23" (at 0 0 0)))"#,
        );
        let kicad7 = r#"(kicad_pcb (version 20221018) (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (49 "F.Fab" user))
  (footprint "R" (layer "F.Cu") (at 5 5)
    (property "MPN" "RC0603FR-0710KL")
    (property "Sheetfile" "t.kicad_sch")
    (fp_text reference "R1" (at 0 -1) (layer "F.Fab"))
    (fp_text value "10k" (at 0 1) (layer "F.Fab"))))"#;
        let mut b = Board::parse(kicad7).unwrap();
        let (undo, done) = b.update(&p, library);
        assert_eq!((done.values, done.fields, b.texts[1].text.as_str()), (1, 2, "4k7"));
        let out = b.write(kicad7).unwrap();
        let want = kicad7.replace("RC0603FR-0710KL", "RC0603FR-074K7L").replace("\"10k\"", "\"4k7\"").replace(
            "(property \"Sheetfile\" \"t.kicad_sch\")",
            "(property \"Sheetfile\" \"t.kicad_sch\")\n    (property \"LCSC\" \"C23\")",
        );
        assert_eq!(out, want);
        assert_eq!(Board::parse(&out).unwrap().footprints, b.footprints);
        b.apply(undo.unwrap());
        assert_eq!(b.write(kicad7).unwrap(), kicad7);
        let kicad10 = "(kicad_pcb\n\t(version 20260206)\n\t(layers\n\t\t(0 \"F.Cu\" signal)\n\t\t(2 \"B.Cu\" signal)\n\t\t(51 \"B.Fab\" user)\n\t)
\t(footprint \"R\"\n\t\t(layer \"B.Cu\")\n\t\t(at 5 5 90)\n\t\t(property \"Reference\" \"R1\"\n\t\t\t(at 0 -1 90)\n\t\t\t(layer \"B.Fab\")\n\t\t)
\t\t(property \"Value\" \"4k7\"\n\t\t\t(at 0 1 90)\n\t\t\t(layer \"B.Fab\")\n\t\t)\n\t)\n)";
        let mut b = Board::parse(kicad10).unwrap();
        let (_, done) = b.update(&p, library);
        assert_eq!((done.values, done.fields), (0, 3));
        let out = b.write(kicad10).unwrap();
        let lcsc = "\n\t\t(property \"LCSC\" \"C23\"\n\t\t\t(at 0 0 90)\n\t\t\t(unlocked yes)\n\t\t\t(layer \"B.Fab\")\n\t\t\t(hide yes)";
        assert!(out.contains("(property \"Datasheet\" \"r.pdf\"") && out.contains(lcsc), "{out}");
        assert!(out.contains("(thickness 0.15)\n\t\t\t\t)\n\t\t\t\t(justify mirror)\n\t\t\t)\n\t\t)\n\t)\n)"), "{out}");
        // Read back, the fields added are among its hidden texts, where the part is.
        let mut read = Board::parse(&out).unwrap();
        let hidden: Vec<(String, Pt, bool)> = read.footprints[0].hidden.drain(..).map(|t| (t.text, t.at, t.mirror)).collect();
        assert_eq!(hidden, ["r.pdf", "RC0603FR-074K7L", "C23"].map(|t| (t.to_string(), Pt { x: 5 * MM, y: 5 * MM }, true)));
        assert_eq!(read.footprints, b.footprints);
    }

    #[test]
    fn swaps_a_footprint_its_symbol_changed_keeping_the_parts_place_side_and_texts() {
        // R1, on the back and turned, was an 0603; its symbol now names an 0805, whose pads are
        // further apart. R2's names one KiCad's libraries lack: it stays, told.
        let p = project(
            &[
                r#"(symbol (lib_id "Device:R") (at 10 10 0) (unit 1) (uuid "u1") (property "Reference" "R1" (at 0 0 0))
                  (property "Value" "10k" (at 0 0 0)) (property "Footprint" "Test:R_0805" (at 0 0 0)))"#,
                r#"(symbol (lib_id "Device:R") (at 30 10 0) (unit 1) (property "Reference" "R2" (at 0 0 0))
                  (property "Footprint" "Test:R_9999" (at 0 0 0)))"#,
                "(wire (pts (xy 7.46 10) (xy 7.46 20))) (wire (pts (xy 7.46 20) (xy 27.46 20))) (wire (pts (xy 27.46 20) (xy 27.46 10)))",
            ]
            .concat(),
        );
        let src = "(kicad_pcb\n\t(version 20260206)\n\t(layers\n\t\t(0 \"F.Cu\" signal)\n\t\t(2 \"B.Cu\" signal)\n\t\t(5 \"F.SilkS\" user)\n\t\t(7 \"B.SilkS\" user)\n\t)
\t(footprint \"Test:R_0603\"\n\t\t(layer \"B.Cu\")\n\t\t(at 10 20 90)\n\t\t(property \"Reference\" \"R1\"\n\t\t\t(at 2 -3 90)\n\t\t\t(layer \"B.SilkS\")
\t\t)\n\t\t(property \"Value\" \"10k\"\n\t\t\t(at 0 1 90)\n\t\t\t(layer \"B.SilkS\")\n\t\t)\n\t\t(property \"MPN\" \"RC0603\")\n\t\t(path \"/u1\")\n\t\t(attr smd dnp)
\t\t(fp_line\n\t\t\t(start -1 0)\n\t\t\t(end 1 0)\n\t\t\t(layer \"B.SilkS\")\n\t\t)\n\t\t(pad \"1\" smd rect\n\t\t\t(at -0.8 0 270)\n\t\t\t(size 0.8 0.9)\n\t\t\t(layers \"B.Cu\")\n\t\t)
\t\t(pad \"2\" smd rect\n\t\t\t(at 0.8 0 270)\n\t\t\t(size 0.8 0.9)\n\t\t\t(layers \"B.Cu\")\n\t\t)\n\t)
\t(footprint \"Test:R_1206\"\n\t\t(layer \"F.Cu\")\n\t\t(at 30 20)\n\t\t(property \"Reference\" \"R2\"\n\t\t\t(at 0 -2 0)\n\t\t\t(layer \"F.SilkS\")\n\t\t)\n\t)\n)";
        let r0805 = "(footprint \"R_0805\"\n\t(version 20260206)\n\t(layer \"F.Cu\")\n\t(property \"Reference\" \"REF**\"\n\t\t(at 0 -1.5 0)\n\t\t(layer \"F.SilkS\")\n\t)
\t(attr smd)\n\t(fp_line\n\t\t(start -1.5 -1)\n\t\t(end 1.5 -1)\n\t\t(layer \"F.SilkS\")\n\t)\n\t(pad \"1\" smd rect\n\t\t(at -1 0)\n\t\t(size 1 1.2)\n\t\t(layers \"F.Cu\")\n\t)
\t(pad \"2\" smd rect\n\t\t(at 1 0)\n\t\t(size 1 1.2)\n\t\t(layers \"F.Cu\")\n\t)\n\t(model \"${KICAD10_3DMODEL_DIR}/R_0805.step\")\n)";
        let library = |id: &str| if id == "Test:R_0805" { Ok(r0805.to_string()) } else { Err(format!("{id} isn't in KiCad's libraries")) };
        let mut b = Board::parse(src).unwrap();
        let (undo, done) = b.update(&p, library);
        assert_eq!((done.footprints, done.missing), (1, vec!["R2: Test:R_9999 isn't in KiCad's libraries".to_string()]));
        // In R1's place, turned and on the back as it was, its texts where the user put them; the
        // 0805's pads, 2 mm apart, now at y 21 and 19, on B.Cu and on R1's nets.
        let f = &b.footprints[0];
        assert_eq!(
            (f.lib_id.as_str(), f.at, f.angle, f.back, f.fitted, f.models[0].file.as_str()),
            ("Test:R_0805", Pt { x: 10 * MM, y: 20 * MM }, 90.0, true, false, "${KICAD10_3DMODEL_DIR}/R_0805.step")
        );
        let pads: Vec<(Pt, &str)> = b.pads[f.pads.clone()].iter().map(|q| (q.at, b.nets[q.net as usize].as_str())).collect();
        assert_eq!(pads, [(Pt { x: 10 * MM, y: 21 * MM }, "Net-(R1-Pad1)"), (Pt { x: 10 * MM, y: 19 * MM }, "unconnected-(R1-Pad2)")]);
        assert_eq!((b.texts[0].at, b.footprints[1].lib_id.as_str()), (Pt { x: 7 * MM, y: 18 * MM }, "Test:R_1206"));
        // Saved: R1's own lists, the 0805's in place of the 0603's, flipped to the back.
        let out = b.write(src).unwrap();
        for part in [
            "(footprint \"Test:R_0805\"",
            "(at 2 -3 90)",
            "(property \"MPN\" \"RC0603\")",
            "(attr smd dnp)",
            "(at -1 0 90)",
            "(layers \"B.Cu\")",
        ] {
            assert!(out.contains(part), "{part}\n{out}");
        }
        assert!(!out.contains("R_0603") && !out.contains("REF**") && !out.contains("(attr smd)\n"), "{out}");
        let back = Board::parse(&out).unwrap();
        assert_eq!((&back.footprints[0].lib_id, &back.pads, &back.shapes, &back.texts), (&f.lib_id, &b.pads, &b.shapes, &b.texts));
        b.apply(undo.unwrap());
        assert_eq!(b.write(src).unwrap(), src);
    }
}
