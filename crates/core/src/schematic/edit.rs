//! Editing a project's sheets as their text: patches that undo themselves, a library symbol, a
//! wire or a net label added to a sheet. Unedited text stays byte for byte.

use super::Project;
use super::net::{LabelKind, on};
use super::symbol::{apply, below_root, turn};
use crate::board::{Nm, Pt};
use crate::save::{Edit, uuid};
use crate::sexpr::{Cursor, Result, escape, unescape};
use crate::tr::{t, tr, trf};
use crate::units::nm_to_mm;
use std::ops::Range;

/// Changes to a project's sheet files: in each file, byte ranges and the text replacing them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patch(pub(super) Vec<(String, Vec<Edit>)>);

/// KiCad's connection grid (50 mil), where symbols go so that their pins meet wires.
// shortcut: a project's own `connection_grid_size` isn't read; read it when one differs.
pub(super) const GRID: Nm = 1_270_000;
/// Between a part's units in their row, the room a net's name takes on a pin facing the gap.
// shortcut: a name of 14 letters; a longer one still reaches the next unit, to be moved.
const ROOM: Nm = 12 * GRID;

/// The shapes of global and hierarchical labels and sheet pins, as KiCad writes them: which way
/// their signal goes.
pub const SHAPES: [&str; 5] = ["input", "output", "bidirectional", "tri_state", "passive"];

/// The point of KiCad's connection grid nearest `p`.
pub fn on_grid(p: Pt) -> Pt {
    let snap = |v: Nm| ((v as f64 / GRID as f64).round() as i64 * GRID as i64) as Nm;
    Pt { x: snap(p.x), y: snap(p.y) }
}

impl crate::history::Edited for Project {
    type Patch = Patch;
    fn apply(&mut self, p: Patch) -> Patch {
        Project::apply(self, p)
    }
}

impl Patch {
    /// The sheet files it changes.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(f, _)| f.as_str())
    }
}

impl Project {
    /// The text of sheet file `file`, with the edits made to it.
    pub fn text(&self, file: &str) -> Option<&str> {
        self.texts.get(file)?.as_deref().ok()
    }

    /// Makes a patch's changes and reads the sheets again; returns the patch that undoes them.
    pub fn apply(&mut self, p: Patch) -> Patch {
        let mut back = Patch::default();
        for (file, mut edits) in p.0 {
            let Some(Ok(text)) = self.texts.get_mut(&file) else { continue };
            edits.sort_by_key(|(r, _)| r.start);
            let mut shift = 0;
            let undo = (edits.iter())
                .map(|(r, new)| {
                    let start = r.start.wrapping_add_signed(shift);
                    shift += new.len() as isize - r.len() as isize;
                    (start..start + new.len(), text[r.clone()].to_string())
                })
                .collect();
            *text = crate::save::apply(text, edits);
            back.0.push((file, undo));
        }
        self.reread(&back.files().collect::<Vec<_>>());
        back
    }

    /// Adds library symbol `text` (as `library::symbol` gives it) of library `lib` to page `page`,
    /// its origin on the grid point nearest `at`, turned `angle` degrees (0, 90, 180 or 270), its
    /// other units in a row after it: the sheet's `lib_symbols` gets its copy unless it has one
    /// so named, and it takes the next free reference of its kind (`R5`, `#PWR012`) in each place
    /// the sheet is used. Fields are valued as `set` says: those the library's lacks (`MPN`)
    /// hidden, as copies of its Footprint field; the visible ones go beside the body
    /// (`autoplace`), a power symbol's where the library puts them. Returns the patch that undoes
    /// it, and its reference here.
    // shortcut: its first body style goes in, as its library's KiCad version writes it, also into
    // older sheets (only that KiCad or later reads them then); write older forms when users ask.
    pub fn add_symbol(
        &mut self,
        page: usize,
        lib: &str,
        text: &str,
        set: &[(&str, String)],
        at: Pt,
        angle: f64,
    ) -> std::result::Result<(Patch, String), String> {
        quarter(angle, t!("angle {0}: a symbol turns by 0, 90, 180 or 270 degrees"))?;
        let (file, src) = self.page_text(page)?;
        let Some(Ok(root)) = self.texts.get(&self.root) else { return Err(trf("{0} isn't read", &[&self.root])) };
        let sym = LibText::read(text).map_err(|e| trf("not a library symbol: {0}", &[&e.msg]))?;
        let field = |name: &str| sym.fields.iter().find(|f| f.name == name);
        let prefix = field("Reference").ok_or(tr("the library symbol has no Reference field"))?.text.clone();
        let new: Vec<_> = set.iter().filter(|(name, _)| field(name).is_none()).collect();
        let copied = if new.is_empty() { None } else { Some(field("Footprint").ok_or(tr("the library symbol has no Footprint field"))?) };
        // KiCad 6 and 7 number fields (`(id 2)`): those added take the next numbers.
        let ids = sym.fields.iter().filter_map(|f| field_id(&text[f.value.end..f.span.end])).max();
        let (sheet, top) = (Layout::read(src).map_err(|e| format!("{file}: {}", e.msg))?, Layout::read(root).map_err(|e| e.msg)?);

        // Its reference in each place the sheet is used, numbered on from the project's highest.
        let places: Vec<usize> = (0..self.pages.len()).filter(|&i| self.pages[i].file == file).collect();
        let used = self.pages.iter().flat_map(|p| &p.sheet.symbols).filter_map(|s| s.designator.strip_prefix(&prefix)?.parse().ok());
        let mut n: u32 = used.max().unwrap_or(0);
        let refs: Vec<String> = (places.iter())
            .map(|_| {
                n += 1;
                reference(&prefix, n)
            })
            .collect();

        let m = turn(angle, "");
        let lib_id = format!("\"{lib}:{}\"", text[sym.name.clone()].trim_matches('"'));
        let flags: String = sym.flags.iter().map(|f| format!("\n\t\t{}", &text[f.clone()])).collect();
        let project = self.root.trim_end_matches(".kicad_sch");
        // A unit placed at `at`, with its id: its fields (not the library's own, keywords and
        // footprint filters) placed with it, its references (KiCad 7+; KiCad 6 keeps them in the
        // root sheet). Its pins' ids are left to KiCad, which writes them when it saves.
        let place = |unit: u32, at: Pt| {
            let id = uuid((&file, lib, &refs, unit, at.x, at.y, src.len()));
            let mut fields = String::new();
            // A field's place, at what its text from `from` on holds.
            let moved = |f: &Field, from: usize| {
                let (x, y) = apply(m, (f.pos.0.x, -f.pos.0.y));
                let to = format!("(at {} {} {})", nm_to_mm(at.x + x), nm_to_mm(at.y + y), f.pos.1);
                f.at.as_ref().map(|span| (span.start - from..span.end - from, to))
            };
            for f in sym.fields.iter().filter(|f| !f.name.starts_with("ki_")) {
                let mut edits: Vec<Edit> = moved(f, f.span.start).into_iter().collect();
                let value = if f.name == "Reference" { Some(&refs[0]) } else { set.iter().find(|s| s.0 == f.name).map(|s| &s.1) };
                if let Some(v) = value {
                    edits.push((f.value.start - f.span.start..f.value.end - f.span.start, format!("\"{}\"", escape(v))));
                }
                fields += "\n\t\t";
                fields += &crate::save::apply(&text[f.span.clone()], edits);
            }
            for (k, (name, value)) in new.iter().enumerate() {
                let f = copied.expect("fields to add have a field to copy");
                let rest = &text[f.value.end..f.span.end];
                let mut edits: Vec<Edit> = moved(f, f.value.end).into_iter().collect();
                if let (Some(last), Some(own)) = (ids, field_id(rest)) {
                    let i = rest.find(&format!("(id {own})")).expect("id read from it");
                    edits.push((i..i + format!("(id {own})").len(), format!("(id {})", last + 1 + k as u32)));
                }
                fields += &format!("\n\t\t(property \"{}\" \"{}\"{}", escape(name), escape(value), crate::save::apply(rest, edits));
            }
            let instances = match top.instances {
                Some(_) => String::new(),
                None => {
                    let paths = places.iter().zip(&refs).map(|(&i, r)| {
                        let path = &self.pages[i].sheet.path;
                        format!("\n\t\t\t\t(path \"{path}\"\n\t\t\t\t\t(reference \"{r}\")\n\t\t\t\t\t(unit {unit})\n\t\t\t\t)")
                    });
                    format!("\n\t\t(instances\n\t\t\t(project \"{project}\"{}\n\t\t\t)\n\t\t)", paths.collect::<String>())
                }
            };
            let (x, y, a) = (nm_to_mm(at.x), nm_to_mm(at.y), angle.rem_euclid(360.0));
            let text = format!(
                "\n\t(symbol\n\t\t(lib_id {lib_id})\n\t\t(at {x} {y} {a})\n\t\t(unit {unit}){flags}\n\t\t(uuid \"{id}\"){fields}{instances}\n\t)"
            );
            (id, text)
        };
        // Its library copy, named as the symbol links it.
        let copy = format!("(symbol {lib_id}{}", &text[sym.name.end..]);
        // Every unit, so that none is missed (an op amp's power pins): in a row from `at`, each a
        // grid step past the last, as far as their bodies and pins reach, and past the names of
        // the nets on the pins that face each other across the gap (`ROOM` a side that has them).
        let mut ats = vec![on_grid(at)];
        if sym.units > 1 {
            let probe =
                format!("(kicad_sch (lib_symbols {copy}) {})", (1..=sym.units).map(|u| place(u, Pt::default()).1).collect::<String>());
            let drawn = super::Schematic::parse(&probe, "").map_err(|e| trf("not a library symbol: {0}", &[&e.msg]))?;
            let faces = |i: usize, way: &str| drawn.pin_ends(i).iter().any(|p| p.facing == way);
            for (k, (b, last)) in drawn.symbols[1..].iter().zip(&drawn.symbols).enumerate() {
                let room = ROOM * (faces(k, "right") as Nm + faces(k + 1, "left") as Nm);
                let x = ats.last().unwrap().x + last.bounds.1.x + 2 * GRID + room - b.bounds.0.x;
                ats.push(on_grid(Pt { x, y: ats[0].y }));
            }
        }
        let units: Vec<(String, String)> = ats.iter().enumerate().map(|(k, &at)| place(k as u32 + 1, at)).collect();
        // Their fields beside them, clear of the sheet's items and texts and of each other.
        let probe = format!("(kicad_sch (lib_symbols {copy}){})", units.iter().map(|u| u.1.as_str()).collect::<String>());
        let drawn = super::Schematic::parse(&probe, "").map_err(|e| trf("not a library symbol: {0}", &[&e.msg]))?;
        let here = &self.pages[page].sheet;
        let mut placed: Vec<(Pt, Pt)> = here.texts.iter().map(crate::font::bounds).chain(drawn.symbols.iter().map(|s| s.bounds)).collect();
        let units = (units.into_iter().enumerate())
            .map(|(k, (id, text))| {
                let edits = match drawn.symbols[k].power {
                    Some(_) => vec![],
                    None => super::autoplace::beside(&drawn, &probe, k, here, None, &mut placed)?,
                };
                let from = text.find('(').unwrap_or(0);
                Ok((id, format!("{}{}", &text[..from], crate::save::apply(&text[from..], edits))))
            })
            .collect::<std::result::Result<Vec<_>, String>>()?;

        let mut edits: Vec<Edit> = sheet.copy(&lib_id, &copy).into_iter().collect();
        edits.push((sheet.end..sheet.end, units.iter().map(|u| u.1.as_str()).collect()));
        let mut patch = Patch(vec![(file.to_string(), edits)]);
        if let Some(end) = top.instances {
            let set = |name: &str| set.iter().find(|s| s.0 == name).map(|s| format!("\"{}\"", escape(&s.1)));
            let value = |name| set(name).unwrap_or_else(|| field(name).map_or("\"\"".into(), |f| text[f.value.clone()].to_string()));
            let (value, footprint) = (value("Value"), value("Footprint"));
            let mut paths = String::new();
            for ((&i, r), (unit, (id, _))) in places.iter().zip(&refs).flat_map(|p| (1..).zip(&units).map(move |u| (p, u))) {
                let path = format!("{}/{id}", below_root(&self.pages[i].sheet.path));
                let entry = format!("(reference \"{r}\") (unit {unit}) (value {value}) (footprint {footprint})");
                paths += &format!("\n    (path \"{path}\"\n      {entry}\n    )");
            }
            match patch.0.iter_mut().find(|(f, _)| *f == self.root) {
                Some((_, edits)) => edits.push((end..end, paths)),
                None => patch.0.push((self.root.clone(), vec![(end..end, paths)])),
            }
        }
        let undo = self.apply_readable(patch, t!("the symbol wasn't added, as it would leave a sheet unreadable: {0}"))?;
        let here = places.iter().position(|&i| i == page).expect("the page is a place of its file");
        Ok((undo, refs[here].clone()))
    }

    /// Adds a wire along `pts` (sheet nm, each taken to its nearest grid point) to page `page`: a
    /// segment from each point to the next, and a junction where a wire's end or a pin lies on a
    /// wire's middle, or three ends meet, as KiCad's editor adds them (a wire's T joins only at
    /// a junction). Returns the patch that undoes it.
    pub fn add_wire(&mut self, page: usize, pts: &[Pt]) -> std::result::Result<Patch, String> {
        self.add_line(page, pts, false)
    }

    /// `add_wire`, or (`bus`) a bus, its junctions where a bus's end lies on a bus's middle or
    /// three bus ends meet.
    pub(super) fn add_line(&mut self, page: usize, pts: &[Pt], bus: bool) -> std::result::Result<Patch, String> {
        let mut pts: Vec<Pt> = pts.iter().map(|&p| on_grid(p)).collect();
        pts.dedup();
        if pts.len() < 2 {
            let few = match bus {
                true => tr("a bus needs two points at least one grid step (50 mil) apart"),
                false => tr("a wire needs two points at least one grid step (50 mil) apart"),
            };
            return Err(few.into());
        }
        let (file, src) = self.page_text(page)?;
        let end = items_end(src).map_err(|e| format!("{file}: {}", e.msg))?;
        let (s, new): (_, Vec<(Pt, Pt)>) = (&self.pages[page].sheet, pts.windows(2).map(|p| (p[0], p[1])).collect());
        let old = if bus { &s.wiring.buses } else { &s.wiring.wires };
        let lines: Vec<(Pt, Pt)> = old.iter().chain(&new).copied().collect();
        // Pins and bus entries join a wire's middle at a junction; nothing a bus's.
        let pins = if bus { vec![] } else { s.ends(|_, p| Some(p)) };
        let inside = |p: Pt, &(a, b): &(Pt, Pt)| p != a && p != b && on(p, a, b);
        // Where one may be needed: the line's points, and the ends and pins along it.
        let along = old.iter().flat_map(|&(a, b)| [a, b]).chain(pins.iter().copied());
        let mut junctions: Vec<Pt> = pts.iter().copied().chain(along.filter(|&p| new.iter().any(|w| inside(p, w)))).collect();
        junctions.sort_by_key(|p| (p.x, p.y));
        junctions.dedup();
        junctions.retain(|&p| !s.wiring.junctions.contains(&p) && needs_junction(p, &lines, &pins));
        let id = |k: usize| uuid((file, &pts, src.len(), k, bus));
        let line = |a, b, id: &str| if bus { wire_text(a, b, id).replacen("(wire", "(bus", 1) } else { wire_text(a, b, id) };
        let mut text: String = new.iter().enumerate().map(|(k, &(a, b))| line(a, b, &id(k))).collect();
        for (k, &p) in junctions.iter().enumerate() {
            text += &junction_text(p, &id(new.len() + k));
        }
        let patch = Patch(vec![(file.to_string(), vec![(end..end, text)])]);
        let refusal = match bus {
            true => t!("the bus wasn't added, as it would leave a sheet unreadable: {0}"),
            false => t!("the wire wasn't added, as it would leave a sheet unreadable: {0}"),
        };
        self.apply_readable(patch, refusal)
    }

    /// Adds a net label `name` (trimmed) of `kind` to page `page` at the grid point nearest `at`,
    /// reading `angle` degrees (0, 90, 180 or 270) from it, in the project's text size, as KiCad 10
    /// writes one: a local label joins the labels of its name on its sheet, a global one those on
    /// every sheet, a hierarchical one (below the root) the net on the pin of its name of each
    /// sheet box showing its sheet, which gets one (`sheet_pins`). A global or hierarchical label
    /// has `shape`, one of `SHAPES`. It joins a wire or bus anywhere along it, a pin at its point.
    /// Returns the patch that undoes it.
    pub fn add_label(
        &mut self,
        page: usize,
        name: &str,
        kind: LabelKind,
        shape: &str,
        at: Pt,
        angle: f64,
    ) -> std::result::Result<Patch, String> {
        self.add_labels(page, &[(name, at, angle)], kind, shape)
    }

    /// Adds labels `(name, at, angle)` of `kind` and `shape` to page `page`, each as `add_label`
    /// adds one, in one edit: returns the patch that undoes them all.
    // shortcut: a global label's inter-sheet references field is left out: KiCad adds and places
    // it, as it does when it shows them.
    pub fn add_labels(
        &mut self,
        page: usize,
        labels: &[(&str, Pt, f64)],
        kind: LabelKind,
        shape: &str,
    ) -> std::result::Result<Patch, String> {
        if kind != LabelKind::Local && !SHAPES.contains(&shape) {
            return Err(trf("shape {0}: a label's shape is input, output, bidirectional, tri_state or passive", &[&format!("{shape:?}")]));
        }
        let (file, src) = self.page_text(page)?;
        let end = Layout::read(src).map_err(|e| format!("{file}: {}", e.msg))?.end;
        let size = nm_to_mm(self.pages[page].sheet.style.text_size);
        // A global or hierarchical label's box is centred on its point; a local label's text sits on it.
        let (head, foot) = match kind {
            LabelKind::Local => ("label", " bottom"),
            LabelKind::Global => ("global_label", ""),
            LabelKind::Hier => ("hierarchical_label", ""),
        };
        let shaped = if kind == LabelKind::Local { String::new() } else { format!("\n\t\t(shape {shape})") };
        let mut text = String::new();
        for &(name, at, angle) in labels {
            let name = name.trim();
            if name.is_empty() || name.contains(['\n', '\r']) {
                return Err(trf("label {0}: give the net's name, one line of text (SDA, ~{RESET})", &[&format!("{name:?}")]));
            }
            quarter(angle, t!("angle {0}: a label turns by 0, 90, 180 or 270 degrees"))?;
            let (at, a) = (on_grid(at), angle.rem_euclid(360.0));
            let (x, y, id) = (nm_to_mm(at.x), nm_to_mm(at.y), uuid((file, name, at, src.len())));
            // Upright text reading away from its point: from it at 0° and 90°, up to it at 180° and 270°.
            let side = if a < 180.0 { "left" } else { "right" };
            let quoted = escape(name);
            text += &format!(
                "\n\t({head} \"{quoted}\"{shaped}\n\t\t(at {x} {y} {a})\n\t\t(effects\n\t\t\t(font\n\t\t\t\t(size {size} {size})\n\t\t\t)\n\t\t\t(justify {side}{foot})\n\t\t)\n\t\t(uuid \"{id}\")\n\t)"
            );
        }
        let mut patch = Patch(vec![(file.to_string(), vec![(end..end, text)])]);
        if kind == LabelKind::Hier {
            let mut names: Vec<&str> = labels.iter().map(|l| l.0.trim()).collect();
            names.dedup();
            for name in names {
                self.sheet_pins(page, name, shape, &mut patch)?;
            }
        }
        self.apply_readable(patch, t!("the label wasn't added, as it would leave a sheet unreadable: {0}"))
    }

    /// Adds a no-connect flag to page `page` on the pin ending at the grid point nearest `at`, as
    /// KiCad 10 writes one: the pin is left unconnected on purpose, which ERC then accepts. Returns
    /// the patch that undoes it.
    pub fn add_no_connect(&mut self, page: usize, at: Pt) -> std::result::Result<Patch, String> {
        let (file, src) = self.page_text(page)?;
        let (at, s) = (on_grid(at), &self.pages[page].sheet);
        let Some((reference, number)) = s.pin_at(at) else {
            return Err(trf("no pin ends at {0} mm: a no-connect flag goes on a pin's end", &[&xy(at)]));
        };
        if s.wiring.no_connects.contains(&at) {
            return Err(trf("{0} pin {1} has a no-connect flag already", &[&reference, &number]));
        }
        let end = items_end(src).map_err(|e| format!("{file}: {}", e.msg))?;
        let text = format!("\n\t(no_connect\n\t\t(at {})\n\t\t(uuid \"{}\")\n\t)", xy(at), uuid((file, at, src.len(), "no_connect")));
        let patch = Patch(vec![(file.to_string(), vec![(end..end, text)])]);
        self.apply_readable(patch, t!("the no-connect flag wasn't added, as it would leave a sheet unreadable: {0}"))
    }

    /// What a click of the no-connect tool does at `at` on page `page`: flags the pin ending at
    /// its grid point, or takes away the flag there. Returns the patch that undoes it, and
    /// whether a flag was added.
    pub fn toggle_no_connect(&mut self, page: usize, at: Pt) -> std::result::Result<(Patch, bool), String> {
        let flags = self.pages.get(page).map_or(&[][..], |p| &p.sheet.wiring.no_connects);
        match flags.iter().position(|&q| q == on_grid(at)) {
            Some(i) => self.delete_items(page, &[super::Item::NoConnect(i)]).map(|undo| (undo, false)),
            None => self.add_no_connect(page, at).map(|undo| (undo, true)),
        }
    }

    /// Page `page`'s sheet file and its text.
    pub(super) fn page_text(&self, page: usize) -> std::result::Result<(&str, &str), String> {
        let file = &self.pages.get(page).ok_or_else(|| trf("no page {0}: the project has {1}", &[&page, &self.pages.len()]))?.file;
        let Some(Ok(src)) = self.texts.get(file) else { return Err(trf("{0} isn't read", &[file])) };
        Ok((file, src))
    }

    /// Makes a patch, unless it would leave a sheet unreadable (a bug here: then `refusal` says
    /// what wasn't added, its `{0}` where): the patch that undoes it.
    pub(super) fn apply_readable(&mut self, patch: Patch, refusal: &'static str) -> std::result::Result<Patch, String> {
        let before = self.errors.clone();
        let undo = self.apply(patch);
        if let Some(e) = self.errors.iter().find(|e| !before.contains(e)).cloned() {
            self.apply(undo);
            return Err(trf(refusal, &[&e]));
        }
        Ok(undo)
    }
}

/// Reference `n` of kind `prefix`: KiCad numbers power symbols and flags `#PWR01`, `#PWR02`…
pub(super) fn reference(prefix: &str, n: u32) -> String {
    if prefix.starts_with('#') { format!("{prefix}0{n}") } else { format!("{prefix}{n}") }
}

/// Whether something can turn `angle` degrees: 0, 90, 180 or 270, else `refusal` (`{0}` the angle).
pub(super) fn quarter(angle: f64, refusal: &'static str) -> std::result::Result<(), String> {
    if angle.rem_euclid(90.0) == 0.0 { Ok(()) } else { Err(trf(refusal, &[&angle])) }
}

/// What adding a library symbol takes from its text.
struct LibText {
    /// Its name as written, quoted.
    name: Range<usize>,
    fields: Vec<Field>,
    /// Where its flags are: `(in_bom yes)`, …
    flags: Vec<Range<usize>>,
    units: u32,
}

/// A library symbol's field: its name, where it and its value (quoted) and `(at …)` are, its
/// value's text, and its place (library Y up) and angle.
struct Field {
    name: String,
    span: Range<usize>,
    value: Range<usize>,
    text: String,
    at: Option<Range<usize>>,
    pos: (Pt, f64),
}

impl LibText {
    fn read(t: &str) -> Result<LibText> {
        let c = &mut Cursor::new(t);
        c.open("symbol")?;
        c.atom()?;
        let mut s = LibText { name: c.span(), fields: vec![], flags: vec![], units: 1 };
        while let Some(head) = c.child()? {
            // The list starts at its `(`, just before its head.
            let start = c.span().start - 1;
            if head == "property" {
                let (name, text) = (unescape(c.atom()?), unescape(c.atom()?));
                let (value, mut at, mut pos) = (c.span(), None, (Pt::default(), 0.0));
                while let Some(head) = c.child()? {
                    let from = c.span().start - 1;
                    let read = if head == "at" { Some((c.point()?, c.opt_angle()?)) } else { None };
                    c.close()?;
                    if let Some(p) = read {
                        (at, pos) = (Some(from..c.span().end), p);
                    }
                }
                c.close()?;
                s.fields.push(Field { name, span: start..c.span().end, value, text, at, pos });
            } else {
                // A unit's graphics and pins: `NAME_unit_style`.
                if head == "symbol" {
                    let unit = c.atom()?.rsplit('_').nth(1).and_then(|u| u.parse().ok());
                    s.units = s.units.max(unit.unwrap_or(1));
                }
                c.close()?;
                if matches!(head, "exclude_from_sim" | "in_bom" | "on_board" | "in_pos_files") {
                    s.flags.push(start..c.span().end);
                }
            }
        }
        Ok(s)
    }
}

/// The number a KiCad 6 or 7 field's text after its value gives it, `(id 2)`.
pub(super) fn field_id(field: &str) -> Option<u32> {
    let at = field.find("(id ")? + 4;
    field[at..].split(')').next()?.trim().parse().ok()
}

/// Where things are in a sheet's text.
/// A point as sheets write one, in mm.
pub(super) fn xy(p: Pt) -> String {
    format!("{} {}", nm_to_mm(p.x), nm_to_mm(p.y))
}

/// A wire from `a` to `b` with id `id`, as KiCad 10 writes one.
pub(super) fn wire_text(a: Pt, b: Pt, id: &str) -> String {
    let (a, b) = (xy(a), xy(b));
    format!(
        "\n\t(wire\n\t\t(pts\n\t\t\t(xy {a}) (xy {b})\n\t\t)\n\t\t(stroke\n\t\t\t(width 0)\n\t\t\t(type default)\n\t\t)\n\t\t(uuid \"{id}\")\n\t)"
    )
}

/// A junction at `p` with id `id`, as KiCad 10 writes one.
pub(super) fn junction_text(p: Pt, id: &str) -> String {
    format!("\n\t(junction\n\t\t(at {})\n\t\t(diameter 0)\n\t\t(color 0 0 0 0)\n\t\t(uuid \"{id}\")\n\t)", xy(p))
}

/// Whether a junction at `p` joins what meets there, as KiCad's editor places them: a wire's end
/// or one of `ends` (pins, sheet pins, bus entries' ends) on a wire's middle, or three ends.
pub(super) fn needs_junction(p: Pt, wires: &[(Pt, Pt)], ends: &[Pt]) -> bool {
    let at = wires.iter().map(|&(a, b)| (a == p) as usize + (b == p) as usize).sum::<usize>() + ends.iter().filter(|&&q| q == p).count();
    let middles = wires.iter().filter(|&&(a, b)| p != a && p != b && on(p, a, b)).count();
    at > 0 && at + 2 * middles >= 3
}

impl super::Schematic {
    /// The reference and number of the pin ending at `p` (`U3A`, `5`); hidden pins aren't there.
    pub fn pin_at(&self, p: Pt) -> Option<(&str, &str)> {
        let q = self.wiring.pins.iter().find(|q| q.at == p && !q.hidden)?;
        Some((&self.symbols[q.symbol].reference, &q.number))
    }

    /// Where wires end on the sheet's other items: its pins but hidden ones, each where `pin(its
    /// symbol, its place)` puts it (`None`: gone), its sheets' pins and its bus entries' ends.
    pub(super) fn ends(&self, pin: impl Fn(usize, Pt) -> Option<Pt>) -> Vec<Pt> {
        let w = &self.wiring;
        let sheet_pins = self.sheets.iter().flat_map(|s| s.pins.iter().map(|p| p.1));
        let entries = w.entries.iter().flat_map(|&(a, b)| [a, b]);
        w.pins.iter().filter(|q| !q.hidden).filter_map(|q| pin(q.symbol, q.at)).chain(sheet_pins).chain(entries).collect()
    }
}

/// Where a sheet's new items go: after its last item, before its instance lists and fonts.
pub(super) fn items_end(src: &str) -> Result<usize> {
    Ok(Layout::read(src)?.end)
}

pub(super) struct Layout {
    /// Its `lib_symbols` list, and the copies in it: name, where each is.
    lib: Option<Range<usize>>,
    pub(super) copies: Vec<(String, Range<usize>)>,
    /// The end of its last item, before its instance lists and fonts.
    pub(super) end: usize,
    /// Where a KiCad 6 root's `symbol_instances` takes another, and its `sheet_instances` another
    /// page.
    pub(super) instances: Option<usize>,
    pub(super) pages: Option<usize>,
    /// Its `paper` and `title_block` lists, and where its head ends (its version, its
    /// generator, its id): where a paper goes when it has none.
    pub(super) paper: Option<Range<usize>>,
    pub(super) title: Option<Range<usize>>,
    pub(super) head: usize,
}

impl Layout {
    /// Library copy `text`, of the symbol linked as `lib_id` (quoted), put in the sheet's
    /// `lib_symbols` in name order, one level further in than in its library; none when the
    /// sheet has one so named.
    pub(super) fn copy(&self, lib_id: &str, text: &str) -> Option<Edit> {
        let text = text.replace('\n', "\n\t");
        let name = |n: &str| format!("\"{n}\"");
        let Some(list) = &self.lib else { return Some((self.end..self.end, format!("\n\t(lib_symbols\n\t\t{text}\n\t)"))) };
        if self.copies.iter().any(|(n, _)| name(n) == lib_id) {
            return None;
        }
        Some(match (self.copies.iter().find(|(n, _)| name(n).as_str() > lib_id), self.copies.last()) {
            (Some((_, after)), _) => (after.start..after.start, format!("{text}\n\t\t")),
            (None, Some((_, last))) => (last.end..last.end, format!("\n\t\t{text}")),
            (None, None) => (list.end - 1..list.end - 1, format!("\n\t\t{text}\n\t")),
        })
    }

    pub(super) fn read(t: &str) -> Result<Layout> {
        let c = &mut Cursor::new(t);
        c.open("kicad_sch")?;
        let mut l = Layout {
            lib: None,
            copies: vec![],
            end: c.span().end,
            instances: None,
            pages: None,
            paper: None,
            title: None,
            head: c.span().end,
        };
        while let Some(head) = c.child()? {
            let start = c.span().start - 1;
            let (mut names, mut last) = (vec![], c.span().end);
            if matches!(head, "lib_symbols" | "symbol_instances" | "sheet_instances") {
                while let Some(h) = c.child()? {
                    let from = c.span().start - 1;
                    let name = if h == "symbol" { unescape(c.atom()?) } else { String::new() };
                    c.close()?;
                    last = c.span().end;
                    names.push((name, from..last));
                }
            }
            c.close()?;
            match head {
                "symbol_instances" => l.instances = Some(last),
                "sheet_instances" => l.pages = Some(last),
                "embedded_fonts" => {}
                _ => l.end = c.span().end,
            }
            match head {
                "version" | "generator" | "generator_version" | "uuid" => l.head = c.span().end,
                "lib_symbols" => (l.lib, l.copies) = (Some(start..c.span().end), names),
                "paper" => l.paper = Some(start..c.span().end),
                "title_block" => l.title = Some(start..c.span().end),
                _ => {}
            }
        }
        Ok(l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// KiCad 10's `Device:R`, trimmed, as `library::symbol` gives it.
    const R: &str = "(symbol \"R\"
		(pin_numbers
			(hide yes)
		)
		(exclude_from_sim no)
		(in_bom yes)
		(on_board yes)
		(property \"Reference\" \"R\"
			(at 2.032 0 90)
			(effects
				(font
					(size 1.27 1.27)
				)
			)
		)
		(property \"Value\" \"R\"
			(at 0 0 90)
		)
		(property \"Footprint\" \"\"
			(at -1.778 0 90)
			(hide yes)
		)
		(property \"ki_keywords\" \"R res resistor\"
			(at 0 0 0)
			(hide yes)
		)
		(symbol \"R_0_1\"
			(rectangle
				(start -1.016 -2.54)
				(end 1.016 2.54)
			)
		)
		(symbol \"R_1_1\"
			(pin passive line
				(at 0 3.81 270)
				(length 1.27)
				(name \"\")
				(number \"1\")
			)
			(pin passive line
				(at 0 -3.81 90)
				(length 1.27)
				(name \"\")
				(number \"2\")
			)
		)
		(embedded_fonts no)
	)";

    /// KiCad 10's `power:GND`, trimmed.
    const GND: &str = "(symbol \"GND\"
		(power global)
		(property \"Reference\" \"#PWR\"
			(at 0 -6.35 0)
			(hide yes)
		)
		(property \"Value\" \"GND\"
			(at 0 -3.81 0)
		)
		(symbol \"GND_1_1\"
			(pin power_in line
				(at 0 0 270)
				(length 0)
				(name \"\")
				(number \"1\")
			)
		)
	)";

    fn project(files: &[(&str, &str)]) -> Project {
        let files: HashMap<_, _> = files.iter().map(|&(n, t)| (n.to_string(), t.to_string())).collect();
        Project::load("root.kicad_sch", "", |f| files.get(f).cloned().ok_or(format!("{f}: not found"))).unwrap()
    }

    fn text<'a>(p: &'a Project, file: &str) -> &'a str {
        p.text(file).unwrap()
    }

    const MM: Nm = 1_000_000;

    #[test]
    fn adds_a_library_symbol_with_the_next_free_reference_and_undoes_it() {
        let root = format!(
            "(kicad_sch\n\t(version 20250114)\n\t(uuid \"r0\")\n\t(lib_symbols\n\t\t{}\n\t)\n\t(symbol (lib_id \"Device:R\") (at 10.16 10.16 0) (unit 1) (uuid \"a\")
		(property \"Reference\" \"R4\" (at 0 0 0)))\n\t(sheet_instances\n\t\t(path \"/\" (page \"1\"))\n\t)\n\t(embedded_fonts no)\n)",
            R.replacen("\"R\"", "\"Device:R\"", 1).replace('\n', "\n\t")
        );
        let mut p = project(&[("root.kicad_sch", &root)]);
        // Turned 90°, near (50.8, 50.8): on the grid there, its copy in the sheet already.
        let (undo, r) = p.add_symbol(0, "Device", R, &[], Pt { x: 50_900_000, y: 50_700_000 }, 90.0).unwrap();
        let t = text(&p, "root.kicad_sch").to_string();
        assert_eq!((r.as_str(), t.matches("(symbol \"Device:R\"").count()), ("R5", 1));
        let new = p.pages[0].sheet.symbols.last().unwrap();
        assert_eq!((new.reference.as_str(), new.designator.as_str()), ("R5", "R5"));
        // Its pins across the sheet now; its fields above it, reading across: their angle in its
        // frame is 90°.
        let pins: Vec<_> = p.pages[0].sheet.wiring.pins.iter().filter(|q| q.symbol == 1).map(|q| (q.number.as_str(), q.at)).collect();
        let at = |x: f64, y: f64| Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm };
        assert_eq!(pins, [("1", at(46.99, 50.8)), ("2", at(54.61, 50.8))]);
        assert!(t.contains("\t\t(property \"Reference\" \"R5\"\n\t\t\t(at 50.8 45.72 90)\n\t\t\t(effects"), "{t}");
        assert!(t.contains("\t\t(property \"Value\" \"R\"\n\t\t\t(at 50.8 48.26 90)"), "{t}");
        assert!(t.contains("(lib_id \"Device:R\")\n\t\t(at 50.8 50.8 90)\n\t\t(unit 1)\n\t\t(exclude_from_sim no)\n\t\t(in_bom yes)"));
        let placed = &t[t.find("(at 50.8 50.8 90)").unwrap()..];
        assert!(
            !placed.contains("ki_keywords") && placed.contains("(project \"root\"\n\t\t\t\t(path \"/r0\"\n\t\t\t\t\t(reference \"R5\")")
        );
        // Before the instance lists; nets and checks see it.
        assert!(t.find("(reference \"R5\")").unwrap() < t.find("(sheet_instances").unwrap());
        assert!(p.netlist().iter().any(|(_, pins)| pins.contains(&("R5".into(), "2".into()))));
        assert_eq!(p.erc().iter().filter(|v| v.kind == "pin_not_connected").count(), 4);
        // Undone, the text is as read; redone, it is back.
        let redo = p.apply(undo);
        assert_eq!((text(&p, "root.kicad_sch"), p.pages[0].sheet.symbols.len()), (root.as_str(), 1));
        p.apply(redo);
        assert_eq!(text(&p, "root.kicad_sch"), t);
        // A power symbol: `#PWR01`, its library copy put in the sheet in name order.
        let (_, r) = p.add_symbol(0, "power", GND, &[], Pt { x: 50 * MM, y: 60 * MM }, 0.0).unwrap();
        let t = text(&p, "root.kicad_sch");
        assert_eq!(r, "#PWR01");
        assert!(t.find("(symbol \"Device:R\"").unwrap() < t.find("(symbol \"power:GND\"\n\t\t\t(power global)").unwrap());
        assert!(t.contains("\t\t\t(symbol \"GND_1_1\"\n\t\t\t\t(pin power_in line"));
        assert_eq!(p.pages[0].sheet.symbols.last().unwrap().power, Some((super::super::LabelKind::Global, "GND".into())));
        // Mistakes say what is wrong.
        assert!(p.add_symbol(0, "Device", R, &[], Pt::default(), 45.0).unwrap_err().contains("0, 90, 180 or 270"));
        assert!(p.add_symbol(3, "Device", R, &[], Pt::default(), 0.0).unwrap_err().starts_with("no page 3"));
        assert!(p.add_symbol(0, "Device", "(kicad_sch)", &[], Pt::default(), 0.0).unwrap_err().starts_with("not a library symbol"));
    }

    #[test]
    fn places_every_unit_in_a_row() {
        let unit = |u: u32, plus: u32, out: u32| {
            format!(
                "(symbol \"OPA_{u}_1\" (polyline (pts (xy -5.08 5.08) (xy 5.08 0) (xy -5.08 -5.08)))
                  (pin input line (at -7.62 2.54 0) (length 2.54) (name \"+\") (number \"{plus}\"))
                  (pin output line (at 7.62 0 180) (length 2.54) (name \"~\") (number \"{out}\")))"
            )
        };
        let opa = format!(
            "(symbol \"OPA\" (property \"Reference\" \"U\" (at 0 5.08 0)) (property \"Value\" \"OPA\" (at 0 -5.08 0)) {} {}
              (symbol \"OPA_3_1\" (pin power_in line (at -2.54 7.62 270) (length 3.81) (name \"V+\") (number \"8\"))
                (pin power_in line (at -2.54 -7.62 90) (length 3.81) (name \"V-\") (number \"4\"))))",
            unit(1, 3, 1),
            unit(2, 5, 7)
        );
        let mut p = project(&[("root.kicad_sch", "(kicad_sch (version 20250114) (uuid \"r\") (lib_symbols))")]);
        let (_, r) = p.add_symbol(0, "Amp", &opa, &[], Pt { x: 25_400_000, y: 25_400_000 }, 0.0).unwrap();
        let refs: Vec<_> = p.pages[0].sheet.symbols.iter().map(|s| s.reference.as_str()).collect();
        assert_eq!((r.as_str(), refs), ("U1", vec!["U1A", "U1B", "U1C"]));
        // Each past the last's pins by a grid step and room for the names on the pins facing each
        // other (an output and an input: 2 x 15.24 mm; then an output and the power unit's side:
        // 15.24 mm). The power unit too, so its pins are in the netlist.
        let t = text(&p, "root.kicad_sch");
        assert!(
            ["(at 25.4 25.4 0)\n\t\t(unit 1)", "(at 73.66 25.4 0)\n\t\t(unit 2)", "(at 101.6 25.4 0)\n\t\t(unit 3)"]
                .iter()
                .all(|u| t.contains(u)),
            "{t}"
        );
        let pins: Vec<_> = p.netlist().into_iter().flat_map(|(_, pins)| pins).filter(|(r, _)| r == "U1").map(|(_, n)| n).collect();
        assert_eq!(pins.len(), 6);
        assert!(["1", "3", "4", "5", "7", "8"].iter().all(|n| pins.contains(&n.to_string())));
    }

    #[test]
    fn adds_wires_with_the_junctions_that_join_them() {
        let root = "(kicad_sch (version 20250114) (uuid \"r\") (lib_symbols) (wire (pts (xy 0 0) (xy 50.8 0))))";
        let mut p = project(&[("root.kicad_sch", root)]);
        let at = |x: f64, y: f64| Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm };
        // Two resistors standing apart: their pins 1 at y 8.89, pins 2 at y 16.51.
        p.add_symbol(0, "Device", R, &[], at(12.7, 12.7), 0.0).unwrap();
        p.add_symbol(0, "Device", R, &[], at(38.1, 12.7), 0.0).unwrap();
        let before = text(&p, "root.kicad_sch").to_string();
        let junctions = |p: &Project| p.pages[0].sheet.wiring.junctions.clone();
        let mut undo = vec![];
        let mut wire = |p: &mut Project, pts: &[Pt]| undo.push(p.add_wire(0, pts).unwrap());
        // Each pin 1 up to the wire's middle, one by a corner (near the grid): junctions at the Ts only.
        wire(&mut p, &[at(12.7, 8.89), at(5.1, 8.9), at(5.08, 0.0)]);
        wire(&mut p, &[at(38.1, 8.89), at(38.1, 0.0)]);
        assert_eq!(junctions(&p), [at(5.08, 0.0), at(38.1, 0.0)]);
        let sheet = &p.pages[0].sheet;
        assert!(sheet.joins(at(12.7, 16.51)) && sheet.joins(at(30.48, 0.0)) && !sheet.joins(at(30.48, 8.89)));
        let t = text(&p, "root.kicad_sch");
        assert!(t.contains("\n\t(wire\n\t\t(pts\n\t\t\t(xy 12.7 8.89) (xy 5.08 8.89)\n\t\t)\n\t\t(stroke\n\t\t\t(width 0)"), "{t}");
        assert!(t.contains("\n\t(junction\n\t\t(at 5.08 0)\n\t\t(diameter 0)\n\t\t(color 0 0 0 0)\n\t\t(uuid \""));
        let pins = |n: &str| [("R1".to_string(), n.to_string()), ("R2".to_string(), n.to_string())];
        assert!(p.netlist().iter().any(|(_, on)| on[..] == pins("1")));
        // Across the wire: none. Through both pins 2, and over the wire's end: one at each.
        wire(&mut p, &[at(25.4, -5.08), at(25.4, 5.08)]);
        wire(&mut p, &[at(5.08, 16.51), at(45.72, 16.51)]);
        wire(&mut p, &[at(0.0, -5.08), at(0.0, 5.08)]);
        assert_eq!(junctions(&p)[2..], [at(12.7, 16.51), at(38.1, 16.51), at(0.0, 0.0)]);
        assert!(p.netlist().iter().any(|(_, on)| on[..] == pins("2")));
        // A point alone isn't a wire; undone, the sheet is as it was.
        assert!(p.add_wire(0, &[at(1.0, 1.0), at(1.2, 1.2)]).unwrap_err().contains("two points"));
        assert!(p.add_wire(2, &[at(0.0, 0.0), at(5.08, 0.0)]).unwrap_err().starts_with("no page 2"));
        for u in undo.into_iter().rev() {
            p.apply(u);
        }
        assert_eq!((text(&p, "root.kicad_sch"), junctions(&p).len()), (before.as_str(), 0));
    }

    #[test]
    fn adds_labels_that_name_the_nets_they_lie_on() {
        let root = "(kicad_sch (version 20250114) (uuid \"r\") (lib_symbols))";
        let mut p = project(&[("root.kicad_sch", root)]);
        let at = |x: f64, y: f64| Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm };
        // Two resistors standing apart, their pins 1 at y 8.89; R2's wired up and along.
        p.add_symbol(0, "Device", R, &[], at(12.7, 12.7), 0.0).unwrap();
        p.add_symbol(0, "Device", R, &[], at(38.1, 12.7), 0.0).unwrap();
        p.add_wire(0, &[at(38.1, 8.89), at(38.1, 2.54), at(50.8, 2.54)]).unwrap();
        let before = text(&p, "root.kicad_sch").to_string();
        // One on R1's pin (near the grid, the name trimmed), one turned on the middle of R2's wire.
        let mut undo = vec![p.add_label(0, " SDA ", LabelKind::Local, "", at(12.8, 8.8), 0.0).unwrap()];
        undo.push(p.add_label(0, "SDA", LabelKind::Local, "", at(45.72, 2.54), 180.0).unwrap());
        let t = text(&p, "root.kicad_sch");
        let font = "\t\t(effects\n\t\t\t(font\n\t\t\t\t(size 1.27 1.27)\n\t\t\t)\n\t\t\t(justify";
        assert!(t.contains(&format!("\n\t(label \"SDA\"\n\t\t(at 12.7 8.89 0)\n{font} left bottom)\n\t\t)\n\t\t(uuid \"")), "{t}");
        assert!(t.contains(&format!("(at 45.72 2.54 180)\n{font} right bottom)")));
        // They join the pins they lie on, by name.
        let pins = [("R1".to_string(), "1".to_string()), ("R2".to_string(), "1".to_string())];
        assert!(p.netlist().iter().any(|(name, on)| name == "/SDA" && on[..] == pins), "{:?}", p.netlist());
        assert_eq!(p.pages[0].sheet.texts.iter().filter(|t| t.text == "SDA").count(), 2);
        // Quotes and backslashes escaped, read back as typed; one on nothing ends a wire there.
        undo.push(p.add_label(0, r#"A"B\C"#, LabelKind::Local, "", at(60.96, 2.54), 90.0).unwrap());
        assert!(text(&p, "root.kicad_sch").contains(r#"(label "A\"B\\C""#));
        assert_eq!(p.pages[0].sheet.wiring.labels.last().unwrap().name, r#"A"B\C"#);
        assert!(p.pages[0].sheet.joins(at(60.96, 2.54)) && !p.pages[0].sheet.joins(at(60.96, 5.08)));
        // Mistakes say what is wrong; undone, the sheet is as it was.
        assert!(p.add_label(0, " ", LabelKind::Local, "", Pt::default(), 0.0).unwrap_err().starts_with("label \"\": give the net's name"));
        assert!(p.add_label(0, "A\nB", LabelKind::Local, "", Pt::default(), 0.0).unwrap_err().contains("one line"));
        assert!(
            p.add_label(0, "SDA", LabelKind::Local, "", Pt::default(), 45.0).unwrap_err().contains("a label turns by 0, 90, 180 or 270")
        );
        assert!(p.add_label(1, "SDA", LabelKind::Local, "", Pt::default(), 0.0).unwrap_err().starts_with("no page 1"));
        for u in undo.into_iter().rev() {
            p.apply(u);
        }
        assert_eq!(text(&p, "root.kicad_sch"), before);
        // In the project's text size.
        let pro = r#"{"schematic":{"drawing":{"default_text_size":60.0}}}"#;
        let mut p = Project::load("root.kicad_sch", pro, |_| Ok(root.into())).unwrap();
        p.add_label(0, "SCL", LabelKind::Local, "", Pt::default(), 270.0).unwrap();
        assert!(text(&p, "root.kicad_sch").contains("(at 0 0 270)\n\t\t(effects\n\t\t\t(font\n\t\t\t\t(size 1.524 1.524)"));
    }

    #[test]
    fn flags_pins_left_unconnected() {
        let root = "(kicad_sch (version 20250114) (uuid \"r\") (lib_symbols))";
        let mut p = project(&[("root.kicad_sch", root)]);
        let at = |x: f64, y: f64| Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm };
        // A resistor alone: its pins (1 at y 8.89, 2 at 16.51) connect to nothing.
        p.add_symbol(0, "Device", R, &[], at(12.7, 12.7), 0.0).unwrap();
        let before = text(&p, "root.kicad_sch").to_string();
        let open = |p: &Project| p.erc().iter().filter(|v| v.kind == "pin_not_connected").map(|v| v.at).collect::<Vec<_>>();
        assert_eq!(open(&p).len(), 2);
        // A flag near pin 1 goes on it, as KiCad 10 writes one, and ERC lets that pin be.
        let undo = p.add_no_connect(0, at(12.8, 8.8)).unwrap();
        let t = text(&p, "root.kicad_sch");
        assert!(t.contains("\n\t(no_connect\n\t\t(at 12.7 8.89)\n\t\t(uuid \""), "{t}");
        assert_eq!(open(&p), [at(12.7, 16.51)]);
        // Mistakes say what is wrong; undone, the sheet is as it was.
        assert_eq!(p.add_no_connect(0, at(12.7, 8.89)).unwrap_err(), "R1 pin 1 has a no-connect flag already");
        // The tool's click takes the flag there away; undone, it is back.
        let flagged = text(&p, "root.kicad_sch").to_string();
        let (back, added) = p.toggle_no_connect(0, at(12.8, 8.8)).unwrap();
        assert!(!added && !text(&p, "root.kicad_sch").contains("(no_connect") && open(&p).len() == 2);
        p.apply(back);
        assert_eq!(text(&p, "root.kicad_sch"), flagged);
        assert!(p.add_no_connect(0, at(20.32, 8.89)).unwrap_err().starts_with("no pin ends at 20.32 8.89 mm"));
        assert!(p.add_no_connect(1, at(12.7, 8.89)).unwrap_err().starts_with("no page 1"));
        p.apply(undo);
        assert_eq!(text(&p, "root.kicad_sch"), before);
        assert!(p.toggle_no_connect(0, at(12.7, 16.51)).unwrap().1 && open(&p) == [at(12.7, 8.89)]);
    }

    #[test]
    fn adds_global_labels_that_join_their_name_on_every_sheet() {
        let root = format!("(kicad_sch (version 20250114) (uuid \"r\") (lib_symbols) {})", sheet("s1", "A", "a.kicad_sch"));
        let mut p = project(&[("root.kicad_sch", &root), ("a.kicad_sch", "(kicad_sch (version 20250114) (uuid \"a\") (lib_symbols))")]);
        let at = |x: f64, y: f64| Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm };
        // A resistor on each sheet, a global label CLK on its pin 1 (at y 8.89), turned on the second.
        for (page, angle) in [(0, 0.0), (1, 180.0)] {
            p.add_symbol(page, "Device", R, &[], at(12.7, 12.7), 0.0).unwrap();
            p.add_label(page, "CLK", LabelKind::Global, "input", at(12.7, 8.89), angle).unwrap();
        }
        let font = "\t\t(effects\n\t\t\t(font\n\t\t\t\t(size 1.27 1.27)\n\t\t\t)\n\t\t\t(justify";
        let label = format!("\n\t(global_label \"CLK\"\n\t\t(shape input)\n\t\t(at 12.7 8.89 0)\n{font} left)\n\t\t)\n\t\t(uuid \"");
        assert!(text(&p, "root.kicad_sch").contains(&label), "{}", text(&p, "root.kicad_sch"));
        assert!(text(&p, "a.kicad_sch").contains(&format!("(shape input)\n\t\t(at 12.7 8.89 180)\n{font} right)\n")));
        // One net across the sheets, its name without a sheet's path.
        let pins = [("R1".to_string(), "1".to_string()), ("R2".to_string(), "1".to_string())];
        assert!(p.netlist().iter().any(|(name, on)| name == "CLK" && on[..] == pins), "{:?}", p.netlist());
    }

    #[test]
    fn reads_the_whole_project_again_when_an_edit_changes_its_sheets() {
        let root = format!("(kicad_sch (version 20250114) (uuid \"r\") (lib_symbols) {})", sheet("s1", "A", "a.kicad_sch"));
        let mut p = project(&[("root.kicad_sch", &root), ("a.kicad_sch", "(kicad_sch (uuid \"a\"))")]);
        // Another box showing the sheet: its page is read.
        let end = root.len() - 1;
        let undo = p.apply(Patch(vec![("root.kicad_sch".into(), vec![(end..end, sheet("s2", "B", "a.kicad_sch"))])]));
        assert_eq!((p.pages.len(), p.pages[0].children.clone()), (3, vec![Some(1), Some(2)]));
        p.apply(undo);
        assert_eq!(p.pages.len(), 2);
        // The root broken: the pages stay, and the error says where.
        p.apply(Patch(vec![("root.kicad_sch".into(), vec![(end..end, "(".into())])]));
        assert_eq!((p.pages.len(), p.errors.len()), (2, 1));
        assert!(p.errors[0].starts_with("root.kicad_sch:1:"), "{:?}", p.errors);
    }

    #[test]
    fn sets_fields_and_adds_those_the_library_lacks_hidden() {
        let set =
            [("Value", "10k".to_string()), ("Footprint", "R:R_0603".into()), ("MPN", "RC0603".into()), ("Manufacturer", "A \"B\"".into())];
        let mut p = project(&[("root.kicad_sch", "(kicad_sch (version 20250114) (uuid \"r\") (lib_symbols))")]);
        p.add_symbol(0, "Device", R, &set, Pt::default(), 0.0).unwrap();
        let t = text(&p, "root.kicad_sch");
        let placed = &t[t.find("(lib_id").unwrap()..];
        assert!(
            placed.contains("(property \"Value\" \"10k\"")
                && placed.contains("(property \"Footprint\" \"R:R_0603\"\n\t\t\t(at -1.778 0 90)")
        );
        assert!(placed.contains("(property \"MPN\" \"RC0603\"\n\t\t\t(at -1.778 0 90)\n\t\t\t(hide yes)\n\t\t)"), "{placed}");
        assert!(placed.contains("(property \"Manufacturer\" \"A \\\"B\\\"\""));
        // The sheet's library copy stays the library's.
        let copy = &t[..t.find("(lib_id").unwrap()];
        assert!(copy.contains("(property \"Value\" \"R\"") && !copy.contains("MPN"));
        // KiCad 6 and 7 number fields: those added take the next numbers; KiCad 6's root lists the values set.
        let r7 = R
            .replace("(property \"Footprint\" \"\"", "(property \"Footprint\" \"\" (id 2)")
            .replace("\"R res resistor\"", "\"R (id 9) res\" (id 4)");
        let root = "(kicad_sch (version 20211123) (uuid \"r\") (lib_symbols)\n  (symbol_instances\n  )\n)";
        let mut p = project(&[("root.kicad_sch", root)]);
        p.add_symbol(0, "Device", &r7, &set, Pt::default(), 0.0).unwrap();
        let t = text(&p, "root.kicad_sch");
        assert!(
            t.contains("(property \"MPN\" \"RC0603\" (id 5)") && t.contains("(property \"Manufacturer\" \"A \\\"B\\\"\" (id 6)"),
            "{t}"
        );
        assert!(t.contains("(reference \"R1\") (unit 1) (value \"10k\") (footprint \"R:R_0603\")"));
    }

    fn sheet(uuid: &str, name: &str, file: &str) -> String {
        format!(r#"(sheet (at 0 0) (size 10 10) (uuid "{uuid}") (property "Sheetname" "{name}") (property "Sheetfile" "{file}"))"#)
    }

    #[test]
    fn numbers_each_place_a_sheet_is_used() {
        // KiCad 7+: the references in the symbol, one per place.
        let root = format!(
            "(kicad_sch (version 20230121) (uuid \"r\") (lib_symbols) {} {})",
            sheet("s1", "A", "a.kicad_sch"),
            sheet("s2", "B", "a.kicad_sch")
        );
        let mut p = project(&[("root.kicad_sch", &root), ("a.kicad_sch", "(kicad_sch (version 20230121) (uuid \"a\") (lib_symbols))")]);
        let (_, r) = p.add_symbol(2, "Device", R, &[], Pt::default(), 0.0).unwrap();
        let refs: Vec<_> = p.pages.iter().map(|g| g.sheet.symbols.iter().map(|s| s.reference.clone()).collect::<Vec<_>>()).collect();
        assert_eq!((r.as_str(), refs), ("R2", vec![vec![], vec!["R1".to_string()], vec!["R2".to_string()]]));
        assert_eq!(text(&p, "root.kicad_sch"), root);
        assert!(text(&p, "a.kicad_sch").contains("(lib_symbols\n\t\t(symbol \"Device:R\"\n\t\t\t(pin_numbers"));
        // KiCad 6: the root lists them, by the places' sheet ids below it.
        let root = format!(
            "(kicad_sch (version 20211123) (uuid \"r\") (lib_symbols) {} {}\n  (symbol_instances\n    (path \"/x/u\" (reference \"R7\") (unit 1))\n  )\n)",
            sheet("s1", "A", "a.kicad_sch"),
            sheet("s2", "B", "a.kicad_sch")
        );
        let mut p = project(&[("root.kicad_sch", &root), ("a.kicad_sch", "(kicad_sch (version 20211123) (uuid \"a\"))")]);
        let (undo, r) = p.add_symbol(1, "Device", R, &[], Pt::default(), 0.0).unwrap();
        let refs: Vec<_> = p.pages[1..].iter().map(|g| g.sheet.symbols[0].reference.clone()).collect();
        assert_eq!((r.as_str(), refs), ("R1", vec!["R1".to_string(), "R2".to_string()]));
        let t = text(&p, "a.kicad_sch");
        let id = &t[t.find("\t\t(uuid \"").unwrap() + 9..][..36];
        assert!(!t.contains("(instances") && t.contains("(lib_symbols\n\t\t(symbol \"Device:R\""), "{t}");
        let top = text(&p, "root.kicad_sch");
        let entry = format!("(path \"/s2/{id}\"\n      (reference \"R2\") (unit 1) (value \"R\") (footprint \"\")\n    )\n  )\n)");
        assert!(top.ends_with(&entry), "{top}");
        p.apply(undo);
        assert_eq!(text(&p, "root.kicad_sch"), root);
    }
}
