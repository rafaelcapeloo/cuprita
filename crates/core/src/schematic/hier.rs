//! A project's hierarchy edited: sheets added, and the sheet pins a hierarchical label added
//! below the root brings to the sheet boxes showing its sheet.

use super::edit::{GRID, Layout, Patch, on_grid, reference, xy};
use super::symbol::below_root;
use super::{LabelKind, Project, Schematic, Sheet};
use crate::board::{Nm, Pt};
use crate::save::uuid;
use crate::sexpr::{Cursor, Result, escape, unescape};
use crate::tr::{t, tr, trf};
use crate::units::nm_to_mm;
use std::collections::{BTreeMap, HashMap, HashSet};

impl Project {
    /// Adds a box from corner `a` to `b` (on the grid) to page `page`, showing sheet file `file`
    /// (beside the root) as sheet `name`, as KiCad 10 writes one: its fields, its id, a page
    /// number for each place the page's sheet is used (in the box; a KiCad 6 root lists them).
    /// It shows the project's text of `file`, else `existing` (the file as read: KiCad's "use an
    /// existing sheet"), else a new sheet, empty, as KiCad 10 writes one, with the root's paper
    /// and title block. Returns the patch that undoes it.
    pub fn add_sheet(
        &mut self,
        page: usize,
        name: &str,
        file: &str,
        a: Pt,
        b: Pt,
        existing: Option<String>,
    ) -> std::result::Result<Patch, String> {
        let (name, file) = (name.trim(), file.trim());
        if name.is_empty() || name.contains(['\n', '\r']) {
            return Err(trf("sheet {0}: give its name, one line of text (Power, USB)", &[&format!("{name:?}")]));
        }
        if file.len() <= ".kicad_sch".len() || !file.ends_with(".kicad_sch") || file.contains(['/', '\\', '\n', '\r']) {
            return Err(trf(
                "file {0}: give a file name ending in .kicad_sch, beside the root sheet (power.kicad_sch)",
                &[&format!("{file:?}")],
            ));
        }
        let (a, b) = (on_grid(a), on_grid(b));
        let (lo, hi) = (Pt { x: a.x.min(b.x), y: a.y.min(b.y) }, Pt { x: a.x.max(b.x), y: a.y.max(b.y) });
        if lo.x == hi.x || lo.y == hi.y {
            return Err(tr("a sheet's box needs a width and a height: drag across").into());
        }
        let (pfile, src) = self.page_text(page)?;
        if self.pages[page].sheet.sheets.iter().any(|s| s.name == name) {
            return Err(trf("a sheet named {0} is on this sheet already: give another name", &[&name]));
        }
        let mut up = Some(page);
        while let Some(k) = up {
            if self.pages[k].file == file {
                return Err(trf("{0} is this sheet or one above it, so it would show itself: give another file", &[&file]));
            }
            up = self.pages[k].parent;
        }
        let here = Layout::read(src).map_err(|e| format!("{pfile}: {}", e.msg))?;
        let root = self.text(&self.root).ok_or_else(|| trf("{0} isn't read", &[&self.root]))?;
        let top = Layout::read(root).map_err(|e| format!("{}: {}", self.root, e.msg))?;
        // The page numbers after the project's highest, one for each place the page's sheet is used.
        let numbers = self.texts.values().filter_map(|t| t.as_deref().ok()).flat_map(|t| t.split("(page \"").skip(1));
        let mut n = numbers.filter_map(|s| s.split('"').next()?.parse::<u32>().ok()).max().unwrap_or(1);
        let places: Vec<(&str, u32)> = (self.pages.iter().filter(|p| p.file == pfile))
            .map(|p| {
                n += 1;
                (p.sheet.path.as_str(), n)
            })
            .collect();
        let id = uuid((pfile, name, file, lo, hi, src.len()));
        let (kicad6, project) = (top.instances.is_some(), escape(self.root.trim_end_matches(".kicad_sch")));
        let instances = match kicad6 {
            true => String::new(),
            false => {
                let paths: String =
                    places.iter().map(|(path, n)| format!("\n\t\t\t\t(path \"{path}\"\n\t\t\t\t\t(page \"{n}\")\n\t\t\t\t)")).collect();
                format!("\n\t\t(instances\n\t\t\t(project \"{project}\"{paths}\n\t\t\t)\n\t\t)")
            }
        };
        // Its fields above and below it, where KiCad 10 places them (measured).
        let size = self.pages[page].sheet.style.text_size;
        let field = |key: &str, value: &str, y: Nm, justify: &str| {
            let s = nm_to_mm(size);
            format!(
                "\n\t\t(property \"{key}\" \"{}\"\n\t\t\t(at {} {} 0)\n\t\t\t(show_name no)\n\t\t\t(do_not_autoplace no)\n\t\t\t(effects\n\t\t\t\t(font\n\t\t\t\t\t(size {s} {s})\n\t\t\t\t)\n\t\t\t\t(justify {justify})\n\t\t\t)\n\t\t)",
                escape(value),
                nm_to_mm(lo.x),
                nm_to_mm(y)
            )
        };
        let fields = field("Sheetname", name, lo.y - 76_600 - size / 2, "left bottom")
            + &field("Sheetfile", file, hi.y + 76_600 + size * 2 / 5, "left top");
        // A sheet shown again: the pins of its hierarchical labels, as KiCad's sync gives them.
        let parsed;
        let shown = match self.pages.iter().find(|p| p.file == file) {
            Some(p) => Some(&p.sheet),
            None => {
                let text = self.text(file).filter(|t| !t.is_empty()).or(existing.as_deref());
                parsed = text.and_then(|t| Schematic::parse(t, &self.pro).ok());
                parsed.as_ref()
            }
        };
        let mut boxed = Sheet { name: name.into(), file: file.into(), bounds: (lo, hi), uuid: id.clone(), pins: vec![], numbers: vec![] };
        let mut pins = String::new();
        for l in shown.iter().flat_map(|s| &s.wiring.labels).filter(|l| l.kind == LabelKind::Hier) {
            let Some((at, angle)) = pin_place(&boxed, l.shape == "input").filter(|_| !boxed.pins.iter().any(|q| q.0 == l.name)) else {
                continue;
            };
            pins += &pin_text(&l.name, &l.shape, at, angle, size, &uuid((&id, &l.name)));
            boxed.pins.push((l.name.clone(), at));
        }
        let text = format!(
            "\n\t(sheet\n\t\t(at {})\n\t\t(size {} {})\n\t\t(exclude_from_sim no)\n\t\t(in_bom yes)\n\t\t(on_board yes)\n\t\t(dnp no)\n\t\t(fields_autoplaced yes)\n\t\t(stroke\n\t\t\t(width 0.1524)\n\t\t\t(type solid)\n\t\t)\n\t\t(fill\n\t\t\t(color 0 0 0 0)\n\t\t)\n\t\t(uuid \"{id}\"){fields}{pins}{instances}\n\t)",
            xy(lo),
            nm_to_mm(hi.x - lo.x),
            nm_to_mm(hi.y - lo.y)
        );
        // What goes where, by file and place in it.
        let mut inserts = Inserts::new();
        inserts.insert((pfile.to_string(), here.end), text);
        let root_lists = match (top.instances, top.pages) {
            (Some(symbols), Some(pages)) => Some((symbols, pages)),
            (None, _) => None,
            _ => return Err(trf("{0}: a KiCad 6 root sheet without sheet_instances", &[&self.root])),
        };
        if let Some((_, at)) = root_lists {
            for (path, n) in &places {
                add(&mut inserts, &self.root, at, format!("\n    (path \"{}/{id}\" (page \"{n}\"))", below_root(path)));
            }
        }
        // A sheet shown again: its parts numbered and its boxes paged in each new place.
        let prefixes: Vec<String> = places.iter().map(|(path, _)| format!("{path}/{id}")).collect();
        self.number_places(file, &prefixes, &mut n, root_lists, &project, &mut inserts)?;
        let mut patch = Patch::default();
        for ((f, at), text) in inserts {
            match patch.0.iter_mut().find(|(g, _)| *g == f) {
                Some((_, edits)) => edits.push((at..at, text)),
                None => patch.0.push((f, vec![(at..at, text)])),
            }
        }
        // A new sheet, empty: as KiCad 10 writes one, with the root's paper and title block.
        let new = format!(
            "(kicad_sch\n\t(version 20260306)\n\t(generator \"cuprita\")\n\t(generator_version \"{}\")\n\t(uuid \"{}\")\n\t{}{}\n\t(lib_symbols)\n)\n",
            env!("CARGO_PKG_VERSION"),
            uuid((file, &id)),
            top.paper.map_or("(paper \"A4\")", |r| &root[r]),
            top.title.map_or(String::new(), |r| format!("\n\t{}", &root[r]))
        );
        let file = file.to_string();
        if !self.texts.get(&file).is_some_and(|t| t.as_ref().is_ok_and(|t| !t.is_empty())) {
            match existing {
                Some(text) => drop(self.texts.insert(file, Ok(text))),
                None => {
                    self.texts.insert(file.clone(), Ok(String::new()));
                    patch.0.push((file, vec![(0..0, new)]));
                }
            }
        }
        self.apply_readable(patch, t!("the sheet wasn't added, as it would leave a sheet unreadable: {0}"))
    }

    /// Adds to `inserts` what the pages under a box showing sheet `file` need in each new place
    /// whose path is in `prefixes`, as KiCad's annotation gives them: each part a reference of
    /// its own, numbered on from the project's highest of its kind, and each sheet box a page
    /// from `n`; in the items (KiCad 7+), else in the root's lists at `root_lists` (KiCad 6's
    /// `symbol_instances` and `sheet_instances`). None when the project doesn't show `file` yet.
    fn number_places(
        &self,
        file: &str,
        prefixes: &[String],
        n: &mut u32,
        root_lists: Option<(usize, usize)>,
        project: &str,
        inserts: &mut Inserts,
    ) -> std::result::Result<(), String> {
        let Some(q) = self.pages.iter().position(|p| p.file == file) else { return Ok(()) };
        let mut under = vec![q];
        let mut i = 0;
        while i < under.len() {
            under.extend(self.pages[under[i]].children.iter().flatten());
            i += 1;
        }
        let mut last: HashMap<&str, u32> = HashMap::new();
        for (kind, k) in self.pages.iter().flat_map(|p| &p.sheet.symbols).filter_map(|s| numbered(&s.designator)) {
            let highest = last.entry(kind).or_default();
            *highest = (*highest).max(k);
        }
        let (base, mut slots) = (self.pages[q].sheet.path.len(), HashMap::new());
        for prefix in prefixes {
            let mut renamed: HashMap<&str, String> = HashMap::new();
            for page in under.iter().map(|&d| &self.pages[d]) {
                let path = format!("{prefix}{}", &page.sheet.path[base..]);
                if !slots.contains_key(&page.file) {
                    let src = self.text(&page.file).ok_or_else(|| trf("{0} isn't read", &[&page.file]))?;
                    slots.insert(&page.file, item_slots(src, project).map_err(|e| format!("{}: {}", page.file, e.msg))?);
                }
                let here = &slots[&page.file];
                // A part's (`list` 0) or a box's (1) entry: KiCad 6's in the root, else its own.
                let mut put = |id: &str, list: usize, kicad6: String, own: String| {
                    match root_lists {
                        Some(at) => add(inserts, &self.root, [at.0, at.1][list], kicad6),
                        None => {
                            let slot = *here.get(id).ok_or_else(|| trf("{0}: item {1} isn't in its text", &[&page.file, &id]))?;
                            let (at, text) = slot.wrap(project, &own);
                            add(inserts, &page.file, at, text);
                        }
                    }
                    Ok::<(), String>(())
                };
                for s in &page.sheet.symbols {
                    let r = renamed.entry(&s.designator).or_insert_with(|| match numbered(&s.designator) {
                        Some((kind, _)) => {
                            let k = last.entry(kind).or_default();
                            *k += 1;
                            reference(kind, *k)
                        }
                        None => s.designator.clone(),
                    });
                    let id = s.link.rsplit('/').next().unwrap_or_default();
                    let kicad6 = format!(
                        "\n    (path \"{}/{id}\"\n      (reference \"{r}\") (unit {}) (value \"{}\") (footprint \"{}\")\n    )",
                        below_root(&path),
                        s.unit,
                        escape(&s.value),
                        escape(&s.footprint)
                    );
                    put(id, 0, kicad6, format!("(path \"{path}\"\n\t\t\t\t\t(reference \"{r}\")\n\t\t\t\t\t(unit {})\n\t\t\t\t)", s.unit))?;
                }
                for b in &page.sheet.sheets {
                    *n += 1;
                    let kicad6 = format!("\n    (path \"{}/{}\" (page \"{n}\"))", below_root(&path), b.uuid);
                    put(&b.uuid, 1, kicad6, format!("(path \"{path}\"\n\t\t\t\t\t(page \"{n}\")\n\t\t\t\t)"))?;
                }
            }
        }
        Ok(())
    }

    /// Adds to `patch` a pin named `name` of `shape` on each sheet box showing page `page`'s sheet
    /// that has none of that name, on its edge (`pin_place`), as KiCad 10 writes one: what a
    /// hierarchical label added there joins the sheet above through.
    pub(super) fn sheet_pins(&self, page: usize, name: &str, shape: &str, patch: &mut Patch) -> std::result::Result<(), String> {
        let file = &self.pages[page].file;
        if self.pages[page].parent.is_none() {
            return Err(trf(
                "hierarchical label {0}: the root sheet has no sheet above it to join; put it on a sheet below, or use a global label",
                &[&name],
            ));
        }
        let size = self.pages[page].sheet.style.text_size;
        // Each sheet file once: a sheet used twice is one text.
        let mut seen = HashSet::new();
        for p in self.pages.iter().filter(|p| seen.insert(&p.file)) {
            let boxes: Vec<&Sheet> = p.sheet.sheets.iter().filter(|s| s.file == *file && !s.pins.iter().any(|q| q.0 == name)).collect();
            if boxes.is_empty() {
                continue;
            }
            let src = self.text(&p.file).ok_or_else(|| trf("{0} isn't read", &[&p.file]))?;
            let ends = pin_ends(src).map_err(|e| format!("{}: {}", p.file, e.msg))?;
            let mut edits = vec![];
            for s in boxes {
                let end = *ends.get(s.uuid.as_str()).ok_or_else(|| trf("{0}: sheet \"{1}\" isn't in its text", &[&p.file, &s.name]))?;
                let Some((at, angle)) = pin_place(s, shape == "input") else {
                    return Err(trf(
                        "sheet \"{0}\" in {1} has no room on its sides for pin {2}: draw its box taller, or delete a pin it no longer needs",
                        &[&s.name, &p.file, &name],
                    ));
                };
                edits.push((end..end, pin_text(name, shape, at, angle, size, &uuid((&p.file, &s.uuid, name, at)))));
            }
            patch.0.push((p.file.clone(), edits));
        }
        Ok(())
    }
}

/// Texts to put into sheet files, by file and place, in the order given at each place.
type Inserts = BTreeMap<(String, usize), String>;

fn add(inserts: &mut Inserts, file: &str, at: usize, text: String) {
    *inserts.entry((file.to_string(), at)).or_default() += &text;
}

/// Where a place's entry goes in a symbol or sheet box: after the last path of its instances for
/// the project, else at the end of its `instances` (in a list for the project), else at its
/// end (in an `instances` list).
#[derive(Clone, Copy)]
enum Slot {
    Path(usize),
    Project(usize),
    Instances(usize),
}

impl Slot {
    /// Where entry `path` goes and its text, in the lists it lacks, as KiCad 10 writes them.
    fn wrap(self, project: &str, path: &str) -> (usize, String) {
        match self {
            Slot::Path(at) => (at, format!("\n\t\t\t\t{path}")),
            Slot::Project(at) => (at, format!("\n\t\t\t(project \"{project}\"\n\t\t\t\t{path}\n\t\t\t)")),
            Slot::Instances(at) => (at, format!("\n\t\t(instances\n\t\t\t(project \"{project}\"\n\t\t\t\t{path}\n\t\t\t)\n\t\t)")),
        }
    }
}

/// Each symbol's and sheet box's `Slot` in the sheet text `src`, by its id, for `project`.
fn item_slots<'a>(src: &'a str, project: &str) -> Result<HashMap<&'a str, Slot>> {
    let (c, mut slots) = (&mut Cursor::new(src), HashMap::new());
    c.open("kicad_sch")?;
    while let Some(head) = c.child()? {
        let (mut id, mut slot, mut last) = (None, None, c.span().end);
        while matches!(head, "symbol" | "sheet")
            && let Some(h) = c.child()?
        {
            match h {
                "uuid" => id = Some(c.atom()?),
                "instances" => {
                    let mut at = Slot::Project(c.span().end);
                    while c.child()?.is_some() {
                        let (name, mut end) = (unescape(c.atom()?), c.span().end);
                        while c.child()?.is_some() {
                            c.close()?;
                            end = c.span().end;
                        }
                        c.close()?;
                        if name == project {
                            at = Slot::Path(end);
                        } else if !matches!(at, Slot::Path(_)) {
                            at = Slot::Project(c.span().end);
                        }
                    }
                    slot = Some(at);
                }
                _ => {}
            }
            c.close()?;
            last = c.span().end;
        }
        c.close()?;
        if let Some(id) = id {
            slots.insert(id, slot.unwrap_or(Slot::Instances(last)));
        }
    }
    Ok(slots)
}

/// A reference's kind and number: `R12` is (`R`, 12), `#PWR012` (`#PWR`, 12); none for `R?`.
fn numbered(r: &str) -> Option<(&str, u32)> {
    let (kind, number) = r.split_at(r.trim_end_matches(|c: char| c.is_ascii_digit()).len());
    (!kind.is_empty()).then_some((kind, number.parse().ok()?))
}

/// A sheet pin named `name` of `shape` at `at`, turned `angle` (0° on a box's right side, 180°
/// on its left), its text `size` high, as KiCad 10 writes one.
fn pin_text(name: &str, shape: &str, at: Pt, angle: f64, size: Nm, id: &str) -> String {
    let (s, side) = (nm_to_mm(size), if angle == 0.0 { "right" } else { "left" });
    format!(
        "\n\t\t(pin \"{}\" {shape}\n\t\t\t(at {} {angle})\n\t\t\t(uuid \"{id}\")\n\t\t\t(effects\n\t\t\t\t(font\n\t\t\t\t\t(size {s} {s})\n\t\t\t\t)\n\t\t\t\t(justify {side})\n\t\t\t)\n\t\t)",
        escape(name),
        xy(at)
    )
}

/// Where sheet box `s` takes a new pin, and its angle: on its left side (180°) for an input,
/// else its right (0°), the other side when that one is full; at the first grid point down from
/// its top a pin's spacing (100 mil) from the others on that side.
// shortcut: KiCad hands the pin to the pointer to place along the box; its top and bottom edges
// aren't used, and a box whose sides are full doesn't grow. Do both when users ask.
fn pin_place(s: &Sheet, input: bool) -> Option<(Pt, f64)> {
    let (lo, hi) = s.bounds;
    let sides = if input { [(lo.x, 180.0), (hi.x, 0.0)] } else { [(hi.x, 0.0), (lo.x, 180.0)] };
    sides.into_iter().find_map(|(x, angle)| {
        let free = |y: &Nm| !s.pins.iter().any(|q| q.1.x == x && (q.1.y - y).abs() < 2 * GRID);
        let top = on_grid(Pt { x, y: lo.y }).y;
        let mut ys = (1..).map(|k| top + k * 2 * GRID).take_while(|&y| y < hi.y);
        ys.find(free).map(|y| (Pt { x, y }, angle))
    })
}

/// Where each sheet box of the sheet text `src` takes a new pin, by the box's id: after its last
/// pin or field.
fn pin_ends(src: &str) -> Result<HashMap<&str, usize>> {
    let (c, mut ends) = (&mut Cursor::new(src), HashMap::new());
    c.open("kicad_sch")?;
    while let Some(head) = c.child()? {
        let (mut id, mut end) = (None, None);
        while head == "sheet"
            && let Some(h) = c.child()?
        {
            if h == "uuid" {
                id = Some(c.atom()?);
            }
            c.close()?;
            if matches!(h, "property" | "pin") {
                end = Some(c.span().end);
            }
        }
        c.close()?;
        if let (Some(id), Some(end)) = (id, end) {
            ends.insert(id, end);
        }
    }
    Ok(ends)
}

#[cfg(test)]
mod tests {
    use super::super::net::tests::{project, r};
    use super::super::{LabelKind, Project};
    use crate::board::{Nm, Pt};
    use std::collections::HashMap;

    fn at(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    /// A box at (101.6, `y`), 20.32 mm wide and 10.16 tall: three pins a side, showing
    /// `sub.kicad_sch` as `name`.
    fn sheet(name: &str, uuid: &str, y: f64) -> String {
        format!(
            r#"(sheet (at 101.6 {y}) (size 20.32 10.16) (uuid "{uuid}") (property "Sheetname" "{name}" (at 101.6 {y} 0))
              (property "Sheetfile" "sub.kicad_sch" (at 101.6 {y} 0))
              (instances (project "p" (path "/root.kicad_sch" (page "2")))))"#
        )
    }

    #[test]
    fn adds_hierarchical_labels_with_the_sheet_pins_they_join() {
        // R1 wired to where the box's first pin on its right goes (2.54 down from its top).
        let root = [sheet("Sub", "s1", 0.0), r("R1", 124.46, 2.54), "(wire (pts (xy 121.92 2.54) (xy 124.46 2.54)))".into()];
        let mut p = project(&[("root.kicad_sch", root.concat()), ("sub.kicad_sch", r("R2", 10.16, 10.16))], "");
        let before = (p.text("root.kicad_sch").unwrap().to_string(), p.text("sub.kicad_sch").unwrap().to_string());
        // A bidirectional label on R2's pin 1: the box gets its pin on the right, joining R1 to R2.
        let mut undo = vec![p.add_label(1, "SDA", LabelKind::Hier, "bidirectional", at(10.2, 10.1), 180.0).unwrap()];
        let font = "(effects\n\t\t\t(font\n\t\t\t\t(size 1.27 1.27)\n\t\t\t)\n\t\t\t(justify right)\n\t\t)";
        let label =
            format!("\n\t(hierarchical_label \"SDA\"\n\t\t(shape bidirectional)\n\t\t(at 10.16 10.16 180)\n\t\t{font}\n\t\t(uuid \"");
        assert!(p.text("sub.kicad_sch").unwrap().contains(&label), "{}", p.text("sub.kicad_sch").unwrap());
        let root = p.text("root.kicad_sch").unwrap();
        let pin = "(at 101.6 0 0))\n\t\t(pin \"SDA\" bidirectional\n\t\t\t(at 121.92 2.54 0)\n\t\t\t(uuid \"";
        let effects = "\n\t\t\t(effects\n\t\t\t\t(font\n\t\t\t\t\t(size 1.27 1.27)\n\t\t\t\t)\n\t\t\t\t(justify right)\n\t\t\t)\n\t\t)\n              (instances";
        assert!(root.contains(pin) && root.contains(effects), "{root}");
        let pins = vec![("R1".to_string(), "1".to_string()), ("R2".to_string(), "1".to_string())];
        assert!(p.netlist().contains(&("/Sub/SDA".to_string(), pins)), "{:?}", p.netlist());
        // An input on the left; the next on the right a step lower; a name the box has gets none.
        undo.push(p.add_label(1, "CLK", LabelKind::Hier, "input", at(30.0, 30.0), 0.0).unwrap());
        undo.push(p.add_label(1, "SCL", LabelKind::Hier, "output", at(30.0, 40.0), 0.0).unwrap());
        undo.push(p.add_label(1, "SDA", LabelKind::Hier, "bidirectional", at(30.0, 50.0), 0.0).unwrap());
        let pins: Vec<_> = p.pages[0].sheet.sheets[0].pins.iter().map(|(n, at)| (n.as_str(), *at)).collect();
        assert_eq!(pins, [("SDA", at(121.92, 2.54)), ("CLK", at(101.6, 2.54)), ("SCL", at(121.92, 5.08))]);
        // Three pins a side, then no room.
        for name in ["A", "B", "C"] {
            undo.push(p.add_label(1, name, LabelKind::Hier, "output", at(30.0, 60.0), 0.0).unwrap());
        }
        let full = p.add_label(1, "D", LabelKind::Hier, "output", at(30.0, 60.0), 0.0).unwrap_err();
        assert!(full.starts_with("sheet \"Sub\" in root.kicad_sch has no room on its sides for pin D"), "{full}");
        // Mistakes say what is wrong; undone, both files are as read.
        let root_label = p.add_label(0, "X", LabelKind::Hier, "input", at(0.0, 0.0), 0.0).unwrap_err();
        assert!(root_label.contains("the root sheet has no sheet above it"), "{root_label}");
        assert!(p.add_label(1, "X", LabelKind::Hier, "in", at(0.0, 0.0), 0.0).unwrap_err().starts_with("shape \"in\""));
        for u in undo.into_iter().rev() {
            p.apply(u);
        }
        assert_eq!((p.text("root.kicad_sch").unwrap(), p.text("sub.kicad_sch").unwrap()), (before.0.as_str(), before.1.as_str()));
    }

    #[test]
    fn a_sheet_used_twice_gets_its_pin_on_both_boxes() {
        let two = format!("{} {}", sheet("A", "s1", 0.0), sheet("B", "s2", 50.8));
        let mut p: Project = project(&[("root.kicad_sch", two), ("sub.kicad_sch", r("R1", 10.16, 10.16))], "");
        p.add_label(2, "EN", LabelKind::Hier, "input", at(10.16, 10.16), 0.0).unwrap();
        let pins: Vec<_> = p.pages[0].sheet.sheets.iter().map(|s| s.pins.clone()).collect();
        assert_eq!(pins, [vec![("EN".to_string(), at(101.6, 2.54))], vec![("EN".to_string(), at(101.6, 53.34))]]);
        // Labelled once in the sheet, its file edited once.
        assert_eq!(p.text("sub.kicad_sch").unwrap().matches("hierarchical_label").count(), 1);
    }

    #[test]
    fn adds_sheets_new_or_existing_with_their_pages() {
        let root = "(kicad_sch\n\t(version 20260306)\n\t(uuid \"r\")\n\t(paper \"A3\")\n\t(title_block\n\t\t(title \"Glow\")\n\t)\n\t(lib_symbols)\n\t(sheet_instances\n\t\t(path \"/\"\n\t\t\t(page \"1\")\n\t\t)\n\t)\n\t(embedded_fonts no)\n)";
        let files = |f: &str| if f == "root.kicad_sch" { Ok(root.to_string()) } else { Err(format!("{f}: none")) };
        let mut p = Project::load("root.kicad_sch", "", files).unwrap();
        // A new file, its box from corner to corner (on the grid), its fields above and below it.
        p.add_sheet(0, "Power", "power.kicad_sch", at(50.9, 40.5), at(25.3, 20.4), None).unwrap();
        let t = p.text("root.kicad_sch").unwrap();
        let fields = "\n\t\t(property \"Sheetname\" \"Power\"\n\t\t\t(at 25.4 19.6084 0)\n\t\t\t(show_name no)\n\t\t\t(do_not_autoplace no)\n\t\t\t(effects\n\t\t\t\t(font\n\t\t\t\t\t(size 1.27 1.27)\n\t\t\t\t)\n\t\t\t\t(justify left bottom)\n\t\t\t)\n\t\t)\n\t\t(property \"Sheetfile\" \"power.kicad_sch\"\n\t\t\t(at 25.4 41.2246 0)";
        assert!(
            t.contains("\n\t(sheet\n\t\t(at 25.4 20.32)\n\t\t(size 25.4 20.32)\n\t\t(exclude_from_sim no)") && t.contains(fields),
            "{t}"
        );
        assert!(t.contains("\n\t\t(instances\n\t\t\t(project \"root\"\n\t\t\t\t(path \"/r\"\n\t\t\t\t\t(page \"2\")\n\t\t\t\t)\n\t\t\t)\n\t\t)\n\t)\n\t(sheet_instances"));
        let new = p.text("power.kicad_sch").unwrap();
        assert!(
            new.starts_with("(kicad_sch\n\t(version 20260306)\n\t(generator \"cuprita\")")
                && new.ends_with("\n\t(paper \"A3\")\n\t(title_block\n\t\t(title \"Glow\")\n\t)\n\t(lib_symbols)\n)\n"),
            "{new}"
        );
        assert_eq!((p.pages.len(), p.pages[1].path.clone(), p.pages[1].file.as_str()), (2, vec!["Power".to_string()], "power.kicad_sch"));
        // In it, another: its page numbered on, its path the sheet's place.
        p.add_sheet(1, "Rails", "rails.kicad_sch", at(10.16, 10.16), at(20.32, 20.32), None).unwrap();
        let path = format!("(path \"/r/{}\"\n\t\t\t\t\t(page \"3\")", p.pages[0].sheet.sheets[0].uuid);
        assert!(p.text("power.kicad_sch").unwrap().contains(&path) && p.pages.len() == 3, "{}", p.text("power.kicad_sch").unwrap());
        // An existing file: shown as it is, the box with a pin for each of its hierarchical labels
        // (as KiCad's sync gives them); the project's own text before the one given.
        let used = "(kicad_sch (version 20260306) (uuid \"u\") (lib_symbols) (hierarchical_label \"EN\" (shape input) (at 10 10 0))
          (hierarchical_label \"OUT\" (shape output) (at 10 20 0)) (hierarchical_label \"EN\" (shape input) (at 10 30 0)))";
        p.add_sheet(0, "Old", "old.kicad_sch", at(60.96, 20.32), at(80.0, 30.0), Some(used.into())).unwrap();
        p.add_sheet(0, "Power 2", "power.kicad_sch", at(60.96, 40.64), at(80.0, 50.0), Some("(kicad_sch)".into())).unwrap();
        assert_eq!((p.text("old.kicad_sch"), p.pages.len()), (Some(used), 6));
        let pins: Vec<_> = p.pages[0].sheet.sheets[1].pins.iter().map(|(n, at)| (n.as_str(), *at)).collect();
        assert_eq!(pins, [("EN", at(60.96, 22.86)), ("OUT", at(80.01, 22.86))]);
        // Mistakes say what is wrong.
        for (name, file, b, error) in [
            ("", "x.kicad_sch", at(5.08, 5.08), "sheet \"\": give its name"),
            ("X", "x.sch", at(5.08, 5.08), "file \"x.sch\": give a file name ending in .kicad_sch"),
            ("X", "a/x.kicad_sch", at(5.08, 5.08), "file \"a/x.kicad_sch\""),
            ("X", "x.kicad_sch", at(0.5, 5.08), "a sheet's box needs a width and a height"),
            ("Power", "x.kicad_sch", at(5.08, 5.08), "a sheet named Power is on this sheet already"),
            ("X", "root.kicad_sch", at(5.08, 5.08), "root.kicad_sch is this sheet or one above it"),
        ] {
            let e = p.add_sheet(0, name, file, at(0.0, 0.0), b, None).unwrap_err();
            assert!(e.starts_with(error), "{e}");
        }
        // The first undone: its box goes, and with it the pages under it; the new file is empty.
        let mut q = Project::load("root.kicad_sch", "", files).unwrap();
        let undo = q.add_sheet(0, "Power", "power.kicad_sch", at(50.9, 40.5), at(25.3, 20.4), None).unwrap();
        q.apply(undo);
        assert_eq!((q.text("root.kicad_sch"), q.text("power.kicad_sch"), q.pages.len()), (Some(root), Some(""), 1));
    }

    #[test]
    fn a_kicad_6_root_lists_the_new_sheets_pages() {
        let root = "(kicad_sch (version 20211123) (uuid r)\n  (lib_symbols)\n  (sheet_instances\n    (path \"/\" (page \"1\"))\n  )\n  (symbol_instances\n  )\n)";
        let mut p = Project::load("root.kicad_sch", "", |_| Ok(root.to_string())).unwrap();
        p.add_sheet(0, "Power", "power.kicad_sch", at(25.4, 20.32), at(50.8, 40.64), None).unwrap();
        let t = p.text("root.kicad_sch").unwrap();
        let id = &p.pages[0].sheet.sheets[0].uuid;
        assert!(
            t.contains(&format!("(path \"/\" (page \"1\"))\n    (path \"/{id}\" (page \"2\"))\n  )")) && !t.contains("(instances"),
            "{t}"
        );
        assert_eq!(p.pages.len(), 2);
    }

    /// A resistor `reference` with id `id`, placed as `places` (path, reference) say.
    fn part(id: &str, reference: &str, places: &[(&str, &str)]) -> String {
        let paths: String = places.iter().map(|(path, r)| format!(r#"(path "{path}" (reference "{r}") (unit 1))"#)).collect();
        format!(
            r#"(symbol (lib_id "T:R") (at 10 10 0) (unit 1) (uuid "{id}") (property "Reference" "{reference}" (at 0 0 0)) (property "Value" "1k" (at 0 0 0))
              (instances (project "root" {paths})))"#
        )
    }

    const LIB: &str = r#"(lib_symbols (symbol "T:R" (symbol "R_1_1" (pin passive line (at -2.54 0 0) (length 1) (name "") (number "1"))
        (pin passive line (at 2.54 0 180) (length 1) (name "") (number "2")))))"#;

    #[test]
    fn a_sheet_shown_again_numbers_its_parts_and_pages_there() {
        let boxed = |id: &str, name: &str, file: &str, place: &str, page: u32| {
            format!(
                r#"(sheet (at 50 0) (size 10 10) (uuid "{id}") (property "Sheetname" "{name}" (at 0 0 0)) (property "Sheetfile" "{file}" (at 0 0 0))
                  (instances (project "root" (path "{place}" (page "{page}")))))"#
            )
        };
        let root = format!(
            "(kicad_sch (version 20260306) (uuid \"r\") (lib_symbols) {} (sheet_instances (path \"/\" (page \"1\"))))",
            boxed("s1", "Sub", "sub.kicad_sch", "/r", 2)
        );
        let sub = format!(
            "(kicad_sch (version 20260306) (uuid \"s\") {LIB} {} {})",
            part("u1", "R1", &[("/r/s1", "R1")]),
            boxed("d1", "Deep", "deep.kicad_sch", "/r/s1", 3)
        );
        let deep = format!("(kicad_sch (version 20260306) (uuid \"d\") {LIB} {})", part("u2", "R2", &[("/r/s1/d1", "R2")]));
        let files: HashMap<&str, String> = [("root.kicad_sch", root), ("sub.kicad_sch", sub), ("deep.kicad_sch", deep)].into();
        let mut p = Project::load("root.kicad_sch", "", |f| files.get(f).cloned().ok_or(format!("{f}: none"))).unwrap();
        // Shown again: there, its part and its sheet's part take the next references, its sheet the next page.
        let undo = p.add_sheet(0, "Again", "sub.kicad_sch", at(0.0, 50.8), at(20.32, 60.96), None).unwrap();
        let refs: Vec<_> =
            p.pages.iter().map(|g| (g.path.join("/"), g.sheet.symbols.iter().map(|s| s.reference.clone()).collect::<Vec<_>>())).collect();
        let names = |a: &str, r: &str| (a.to_string(), vec![r.to_string()]);
        assert_eq!(refs[1..], [names("Sub", "R1"), names("Again", "R3"), names("Sub/Deep", "R2"), names("Again/Deep", "R4")]);
        let id = &p.pages[0].sheet.sheets[1].uuid;
        let sub = p.text("sub.kicad_sch").unwrap();
        assert!(
            sub.contains(&format!(
                "(reference \"R1\") (unit 1))\n\t\t\t\t(path \"/r/{id}\"\n\t\t\t\t\t(reference \"R3\")\n\t\t\t\t\t(unit 1)\n\t\t\t\t))"
            )),
            "{sub}"
        );
        assert!(sub.contains(&format!("(page \"3\"))\n\t\t\t\t(path \"/r/{id}\"\n\t\t\t\t\t(page \"5\")\n\t\t\t\t)))")), "{sub}");
        assert!(p.erc().iter().all(|v| v.kind != "duplicate_reference"));
        // Undone, every file is as read.
        p.apply(undo);
        assert!(["root.kicad_sch", "sub.kicad_sch", "deep.kicad_sch"].iter().all(|f| p.text(f) == files.get(f).map(String::as_str)));
        // KiCad 6: the root lists the place's reference.
        let r6 = "(kicad_sch (version 20211123) (uuid r)\n  (lib_symbols)\n  (sheet (at 50 0) (size 10 10) (uuid s1) (property \"Sheet name\" \"Sub\" (at 0 0 0)) (property \"Sheet file\" \"sub.kicad_sch\" (at 0 0 0)))\n  (sheet_instances\n    (path \"/\" (page \"1\"))\n    (path \"/s1\" (page \"2\"))\n  )\n  (symbol_instances\n    (path \"/s1/u1\" (reference \"R1\") (unit 1))\n  )\n)";
        let s6 = format!("(kicad_sch (version 20211123) (uuid s) {LIB} {})", part("u1", "R?", &[]));
        let mut p = Project::load("root.kicad_sch", "", |f| Ok(if f == "sub.kicad_sch" { s6.clone() } else { r6.to_string() })).unwrap();
        p.add_sheet(0, "Again", "sub.kicad_sch", at(0.0, 50.8), at(20.32, 60.96), None).unwrap();
        let id = &p.pages[0].sheet.sheets[1].uuid;
        let entry = format!("(path \"/{id}/u1\"\n      (reference \"R2\") (unit 1) (value \"1k\") (footprint \"\")\n    )\n  )\n)");
        assert!(
            p.text("root.kicad_sch").unwrap().ends_with(&entry) && p.pages[2].sheet.symbols[0].reference == "R2",
            "{}",
            p.text("root.kicad_sch").unwrap()
        );
    }
}
