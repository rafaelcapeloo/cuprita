//! The schematic's commands: `add` of a symbol or label, `wire`, `erc` and `simulate`, on the
//! project's sheets as this session edits them.

use super::{Session, footprint_json, mm_pt, mm_pts, pick, redraw};
use crate::edit::footprint_text;
use crate::sheet::{read_project, root_of, updated};
use crate::sheet_edit::{NO_SYMBOLS, Sheets, file_for};
use cuprita_core::board::Pt;
use cuprita_core::digikey;
use cuprita_core::json::{Val, json_str, list};
use cuprita_core::library;
use cuprita_core::ngspice::{self, Plot};
use cuprita_core::schematic::{Deck, LabelKind, PinEnd, Project, on_grid};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

impl Session {
    /// The root sheet of the project of the `.kicad_sch` at `path`, else of the one beside the
    /// open board.
    fn sch_root(&self, path: Option<&str>) -> Result<PathBuf, String> {
        let root = match (path, &self.path) {
            (Some(p), _) => root_of(Path::new(p)),
            (None, Some(board)) => board.with_extension("kicad_sch"),
            (None, None) => return Err("no board open: give path, a .kicad_sch of the project".into()),
        };
        if !root.exists() {
            return Err(format!("no schematic: {} isn't there; give path, a .kicad_sch of the project", root.display()));
        }
        Ok(root)
    }

    /// The schematic to edit, as `sch_root` finds it: the one with this session's unsaved edits,
    /// else read from its files (as KiCad may have changed them); another project's only once the
    /// one before has no unsaved edits.
    fn sheets(&mut self, path: Option<&str>) -> Result<&mut Sheets, String> {
        let root = self.sch_root(path)?;
        match &self.sch {
            Some(s) if s.history.unsaved() && s.root == root => {}
            Some(s) if s.history.unsaved() => {
                return Err(format!("{} has unsaved edits: save them, or undo them, first", s.root.display()));
            }
            _ => {
                self.sch = Some(Sheets::read(&root, &[])?);
                self.edits.forget(true);
            }
        }
        Ok(self.sch.as_mut().expect("read above"))
    }

    /// Updates the open board from its project's schematic, as KiCad's Update PCB from Schematic:
    /// the parts it lacks added below it, every part's pads on the schematic's nets.
    pub(super) fn update(&mut self) -> Result<String, String> {
        if self.scene.is_none() {
            return Err("no board open: open the board to update from its schematic".into());
        }
        self.sheets(None)?;
        let folder = self.folder();
        let (sch, parts) =
            (self.sch.as_ref().expect("read above"), self.parts.get_or_insert_with(|| crate::parts::footprints(folder.as_deref())));
        let (undo, done) = self.board.update(&sch.project, |id| footprint_text(parts, id));
        if let Some(undo) = undo {
            self.edits.push(&mut self.board, undo, updated(&done));
        }
        let scene = redraw(&mut self.scene, &self.board);
        let added = list(self.board.footprints[self.board.footprints.len() - done.parts..].iter().map(|f| footprint_json(scene, f)));
        Ok(format!(
            r#"{{"parts_added":{added},"footprints_changed":{},"values_changed":{},"fields_changed":{},"nets_renamed":{},"nets_added":{},"pads_moved":{},"not_added":{}}}"#,
            done.footprints,
            done.values,
            done.fields,
            done.renamed,
            done.added,
            done.moved,
            list(done.missing.iter().map(|m| json_str(m)))
        ))
    }

    /// Adds the KiCad library symbol `symbol` names, or Digi-Key's part numbered `part` with its
    /// fields (`Part::kicad`), to a sheet of the project's schematic.
    pub(super) fn add_symbol(&mut self, args: &Val) -> Result<String, String> {
        let (at, angle) = at_angle(args)?;
        let folder = self.folder();
        let symbols = self.symbols.get_or_insert_with(|| crate::parts::symbols(folder.as_deref()));
        let (part, text, fields) = match args.str("part") {
            Some(mpn) => digikey::lookup(mpn, &crate::settings::digikey_key()?)?
                .kicad(symbols, self.parts.get_or_insert_with(|| crate::parts::footprints(folder.as_deref())))?,
            None => {
                let q = args.str("symbol").unwrap_or_default();
                let part = pick(symbols, q, ("symbol", "Library:Name or of what it is (LM358, regulator 3.3)"), NO_SYMBOLS)?;
                let read = std::fs::read_to_string(&part.path).map_err(|e| e.to_string());
                let text = read.and_then(|t| library::symbol(&t, &part.name)).map_err(|e| format!("{}: {e}", part.path.display()))?;
                (part, text, vec![])
            }
        };
        let mut fields = fields;
        if let Some(Val::Obj(set)) = args.get("fields") {
            for (name, value) in set {
                let Val::Str(value) = value else { return Err(format!("fields: {name}'s value must be text")) };
                fields.retain(|f| f.0 != name);
                fields.push((name.as_str(), value.clone()));
            }
        }
        let s = self.sheets(args.str("path"))?;
        let page = page(s, args.str("sheet"))?;
        let (undo, reference) = s.project.add_symbol(page, &part.lib, &text, &fields, at, angle)?;
        s.push(undo, format!("{reference} added"));
        let (page, at) = (&s.project.pages[page], on_grid(at));
        let unit = |r: &str| r.strip_prefix(reference.as_str()).is_some_and(|u| u.chars().all(|c| c.is_ascii_uppercase()));
        let set: Vec<String> = fields.iter().map(|(k, v)| format!("{}:{}", json_str(k), json_str(v))).collect();
        let set = if set.is_empty() { String::new() } else { format!(r#","fields":{{{}}}"#, set.join(",")) };
        let added = format!(
            r#"{{"reference":{},"symbol":{},"sheet":{},"at_mm":[{},{}],"angle":{},"units":{}{set},"pins":{}}}"#,
            json_str(&reference),
            json_str(&part.id()),
            json_str(&format!("/{}", page.path.join("/"))),
            at.x as f64 / 1e6,
            at.y as f64 / 1e6,
            angle.rem_euclid(360.0),
            page.sheet.symbols.iter().filter(|u| unit(&u.reference)).count(),
            pins_json(&s.project, &reference),
        );
        self.edits.did(true);
        Ok(added)
    }

    /// Adds a net label named `label`, `global` or not, or a hierarchical label named `hier_label`
    /// (its sheet's boxes get its pin), of `shape`, to a sheet of the project's schematic.
    pub(super) fn add_label(&mut self, args: &Val) -> Result<String, String> {
        let global = if args.bool("global").unwrap_or(false) { LabelKind::Global } else { LabelKind::Local };
        let (name, kind) = match (args.str("label"), args.str("hier_label")) {
            (Some(name), _) => (name, global),
            (None, Some(name)) => (name, LabelKind::Hier),
            _ => return Err("label must be the net's name (SDA)".into()),
        };
        let shape = args.str("shape").unwrap_or(if kind == LabelKind::Hier { "bidirectional" } else { "input" });
        let (at, angle) = at_angle(args)?;
        let s = self.sheets(args.str("path"))?;
        let page = page(s, args.str("sheet"))?;
        let undo = s.project.add_label(page, name, kind, shape, at, angle)?;
        let name = name.trim();
        s.push(undo, format!("{} {name} added", kind.name()));
        let (sheet, at) = (&s.project.pages[page].path, on_grid(at));
        let added = format!(
            r#"{{"label":{},"sheet":{},"at_mm":[{},{}],"angle":{}}}"#,
            json_str(name),
            json_str(&format!("/{}", sheet.join("/"))),
            at.x as f64 / 1e6,
            at.y as f64 / 1e6,
            angle.rem_euclid(360.0),
        );
        self.edits.did(true);
        Ok(added)
    }

    /// Joins pins `pins` (`U1:VIN`, `C3:1`: a part's reference and a pin's number, else every pin
    /// of that name) into net `net`: a label of its name on each pin's end, reading away from its
    /// symbol, as one edit on each sheet.
    pub(super) fn connect(&mut self, args: &Val) -> Result<String, String> {
        let net = args.str("net").ok_or("give net: the name of the net the pins join (+3V3, SDA)")?;
        let kind = if args.bool("global").unwrap_or(false) { LabelKind::Global } else { LabelKind::Local };
        let pins: Vec<&str> =
            args.arr("pins").unwrap_or_default().iter().filter_map(|v| if let Val::Str(s) = v { Some(s.as_str()) } else { None }).collect();
        if pins.is_empty() {
            return Err(r#"give pins: ["U1:VIN", "C3:1"], each a part's reference and a pin's number or name"#.into());
        }
        let s = self.sheets(args.str("path"))?;
        let mut on: std::collections::BTreeMap<usize, Vec<(Pt, f64)>> = Default::default();
        let mut done = vec![];
        for want in pins {
            let (part, pin) = want.split_once(':').ok_or_else(|| format!("pin {want}: give it as reference:pin (U1:VIN, C3:1)"))?;
            let pages = s.project.pages.iter().enumerate();
            let units = pages.flat_map(|(k, page)| {
                page.sheet.symbols.iter().enumerate().filter(|(_, u)| u.designator == part).map(move |(i, _)| (k, i))
            });
            let ends: Vec<_> = units.flat_map(|(k, i)| s.project.pages[k].sheet.pin_ends(i).into_iter().map(move |p| (k, p))).collect();
            if ends.is_empty() {
                return Err(format!("no part {part} on the sheets"));
            }
            let by_number: Vec<_> = ends.iter().filter(|(_, p)| p.number == pin).collect();
            let found = if by_number.is_empty() { ends.iter().filter(|(_, p)| p.name == pin).collect() } else { by_number };
            if found.is_empty() {
                let named =
                    |p: &PinEnd| if p.name.is_empty() || p.name == "~" { p.number.clone() } else { format!("{} ({})", p.number, p.name) };
                let some: Vec<String> = ends.iter().take(12).map(|(_, p)| named(p)).collect();
                let more = if ends.len() > some.len() { "…" } else { "" };
                return Err(format!("{part} has no pin {pin}: its pins are {}{more}", some.join(", ")));
            }
            for (k, p) in found {
                let angle = match p.facing {
                    "left" => 180.0,
                    "up" => 90.0,
                    "down" => 270.0,
                    _ => 0.0,
                };
                on.entry(*k).or_default().push((p.at, angle));
                done.push(format!(
                    r#"{{"pin":{},"at_mm":[{},{}]}}"#,
                    json_str(&format!("{part}:{}", p.number)),
                    p.at.x as f64 / 1e6,
                    p.at.y as f64 / 1e6
                ));
            }
        }
        let sheets = on.len();
        for (page, labels) in on {
            let labels: Vec<_> = labels.into_iter().map(|(at, angle)| (net, at, angle)).collect();
            let undo = s.project.add_labels(page, &labels, kind, "passive")?;
            s.push(undo, format!("{net} connected"));
        }
        (0..sheets).for_each(|_| self.edits.did(true));
        Ok(format!(r#"{{"net":{},"labeled":{}}}"#, json_str(net.trim()), list(done.into_iter())))
    }

    /// Flags the pin ending at `at` on a sheet of the project's schematic as left unconnected.
    pub(super) fn add_no_connect(&mut self, args: &Val) -> Result<String, String> {
        let (at, _) = at_angle(args)?;
        let s = self.sheets(args.str("path"))?;
        let page = page(s, args.str("sheet"))?;
        let undo = s.project.add_no_connect(page, at)?;
        s.push(undo, "No-connect flag added".into());
        let (p, at) = (&s.project.pages[page], on_grid(at));
        let pin = p.sheet.pin_at(at).map(|(r, n)| format!("{r} pin {n}")).unwrap_or_default();
        let (sheet, x, y) = (json_str(&format!("/{}", p.path.join("/"))), at.x as f64 / 1e6, at.y as f64 / 1e6);
        self.edits.did(true);
        Ok(format!(r#"{{"no_connect":{},"sheet":{sheet},"at_mm":[{x},{y}]}}"#, json_str(&pin)))
    }

    /// Adds a wire through `points` (two or more) to a sheet of the project's schematic.
    pub(super) fn add_wire(&mut self, args: &Val, bus: bool) -> Result<String, String> {
        let mut pts = mm_pts(args, "points").filter(|p| p.len() >= 2).ok_or("points must be [[x_mm, y_mm], …], two or more")?;
        let s = self.sheets(args.str("path"))?;
        let page = page(s, args.str("sheet"))?;
        let undo = if bus { s.project.add_bus(page, &pts)? } else { s.project.add_wire(page, &pts)? };
        s.push(undo, if bus { "Bus added" } else { "Wire added" }.into());
        self.edits.did(true);
        pts = pts.into_iter().map(on_grid).collect();
        pts.dedup();
        let pts = pts.iter().map(|p| format!("[{},{}]", p.x as f64 / 1e6, p.y as f64 / 1e6));
        let sheet = &self.sch.as_ref().expect("edited above").project.pages[page].path;
        let what = if bus { "bus" } else { "wire" };
        Ok(format!(r#"{{"{what}":{},"sheet":{}}}"#, list(pts), json_str(&format!("/{}", sheet.join("/")))))
    }

    /// Adds a bus entry from `at` on a sheet of the project's schematic, turned `angle`.
    pub(super) fn add_bus_entry(&mut self, args: &Val) -> Result<String, String> {
        let (at, angle) = at_angle(args)?;
        let s = self.sheets(args.str("path"))?;
        let page = page(s, args.str("sheet"))?;
        let undo = s.project.add_bus_entry(page, at, angle)?;
        s.push(undo, "Bus entry added".into());
        self.edits.did(true);
        let (p, at) = (&self.sch.as_ref().expect("edited above").project.pages[page], on_grid(at));
        let (sheet, x, y) = (json_str(&format!("/{}", p.path.join("/"))), at.x as f64 / 1e6, at.y as f64 / 1e6);
        Ok(format!(r#"{{"bus_entry":[{x},{y}],"sheet":{sheet},"angle":{}}}"#, angle.rem_euclid(360.0)))
    }

    /// Adds a sheet named `name` to a sheet of the project's schematic, its box at `at` [x, y] mm
    /// (its top left corner) of `size` [width, height] mm, showing `file` (default the name's): a
    /// new sheet, or the project's or folder's file of that name.
    pub(super) fn add_sheet(&mut self, args: &Val) -> Result<String, String> {
        let name = args.str("name").ok_or("name must be the sheet's name (Power)")?;
        let file = args.str("file").map_or_else(|| file_for(name), String::from);
        let (at, _) = at_angle(args)?;
        let size = match args.arr("size") {
            Some([Val::Num(w), Val::Num(h)]) if w.is_finite() && h.is_finite() => mm_pt(*w, *h),
            _ => return Err("size must be [width_mm, height_mm]".into()),
        };
        let s = self.sheets(args.str("path"))?;
        let page = page(s, args.str("sheet"))?;
        let path = s.root.with_file_name(file.trim());
        let existing = if path.exists() {
            Some(std::fs::read_to_string(&path).map_err(|e| format!("{} not read: {e}", path.display()))?)
        } else {
            None
        };
        let undo = s.project.add_sheet(page, name, &file, at, Pt { x: at.x + size.x, y: at.y + size.y }, existing)?;
        s.push(undo, format!("Sheet {} added", name.trim()));
        self.edits.did(true);
        let (from, to) = (on_grid(at), on_grid(Pt { x: at.x + size.x, y: at.y + size.y }));
        let p = &self.sch.as_ref().expect("edited above").project.pages[page];
        Ok(format!(
            r#"{{"sheet":{},"file":{},"at_mm":[{},{}],"size_mm":[{},{}]}}"#,
            json_str(&format!("/{}", p.path.iter().map(String::as_str).chain([name.trim()]).collect::<Vec<_>>().join("/"))),
            json_str(file.trim()),
            from.x.min(to.x) as f64 / 1e6,
            from.y.min(to.y) as f64 / 1e6,
            (to.x - from.x).abs() as f64 / 1e6,
            (to.y - from.y).abs() as f64 / 1e6,
        ))
    }

    /// Sets field `name` (Value, Footprint, MPN…) of part `reference` to `value` on its every unit
    /// and sheet; with no `value`, says the part's fields and its footprint filters.
    pub(super) fn field(&mut self, args: &Val) -> Result<String, String> {
        let part = args.str("reference").ok_or("missing reference")?;
        let s = self.sheets(args.str("path"))?;
        if let (Some(name), Some(value)) = (args.str("name"), args.str("value")) {
            let undo = s.project.set_field(part, name, value)?;
            s.push(undo, format!("{part} {name} {value}"));
            self.edits.did(true);
        }
        let s = self.sch.as_ref().expect("read above");
        let f = s.project.fields(part).ok_or_else(|| format!("no part {part} on the sheets"))?;
        let fields = f.fields.iter().map(|(k, v)| format!("{}:{}", json_str(k), json_str(v)));
        Ok(format!(
            r#"{{"reference":{},"fields":{{{}}},"footprint_filters":{},"pins":{}}}"#,
            json_str(part),
            fields.collect::<Vec<_>>().join(","),
            json_str(&f.filters),
            pins_json(&s.project, part)
        ))
    }

    /// The schematic of a project, as `sch_root` finds it, with the session's unsaved edits to it
    /// if any, given to `f` with its root sheet.
    fn project<T>(&self, path: Option<&str>, f: impl FnOnce(&Path, &Project) -> Result<T, String>) -> Result<T, String> {
        let root = self.sch_root(path)?;
        match &self.sch {
            Some(s) if s.root == root && s.history.unsaved() => f(&root, &s.project),
            _ => f(&root, &read_project(&root, &[])?),
        }
    }

    /// The electrical rule violations of the schematic of a project, as `project` finds it.
    pub(super) fn erc(&self, path: Option<&str>) -> Result<String, String> {
        self.project(path, erc)
    }

    /// The schematic of a project, as `project` finds it, written as one PDF, `<root>.pdf` beside
    /// its root sheet (the one there kept as `-bak`).
    pub(super) fn pdf(&self, path: Option<&str>) -> Result<String, String> {
        self.project(path, |root, p| {
            let file = root.with_extension("pdf");
            let bak = crate::file::save_file(&file, &p.pdf(&root.file_stem().unwrap_or_default().to_string_lossy()))?;
            let bak = bak.map_or("null".into(), |b| json_str(&b.display().to_string()));
            Ok(format!(r#"{{"file":{},"sheets":{},"backup":{bak}}}"#, json_str(&file.display().to_string()), p.pages.len()))
        })
    }

    /// The schematic of a project, as `project` finds it, simulated in ngspice: each analysis,
    /// with the values of the `signals` named.
    pub(super) fn simulate(&self, path: Option<&str>, signals: &[&str]) -> Result<String, String> {
        self.project(path, |root, p| {
            let deck = p.spice(root.parent().unwrap_or(Path::new(".")))?;
            let sim = ngspice::simulate(&deck, &AtomicBool::new(false))?;
            let analyses = list(sim.plots.iter().map(|plot| analysis(plot, &deck, signals)));
            let log = list(sim.log.iter().map(|l| json_str(l)));
            Ok(format!(r#"{{"schematic":{},"analyses":{analyses},"log":{log}}}"#, json_str(&root.display().to_string())))
        })
    }
}

/// A project's electrical rule violations as agents read them.
fn erc(root: &Path, p: &Project) -> Result<String, String> {
    let vs = p.erc();
    let errors = vs.iter().filter(|v| v.error).count();
    let items = vs.iter().map(|v| {
        format!(
            r#"{{"kind":{},"error":{},"sheet":{},"at_mm":[{:.3},{:.3}],"what":{},"fix":{}}}"#,
            json_str(v.kind),
            v.error,
            json_str(&format!("/{}", p.pages[v.page].path.join("/"))),
            v.at.x as f64 / 1e6,
            v.at.y as f64 / 1e6,
            json_str(&v.what),
            json_str(&v.fix),
        )
    });
    let root = json_str(&root.display().to_string());
    Ok(format!(
        r#"{{"schematic":{root},"sheets":{},"errors":{errors},"warnings":{},"violations":{}}}"#,
        p.pages.len(),
        vs.len() - errors,
        list(items)
    ))
}

/// An analysis as agents read it: its sweep (`x`; an operating point has none) and each signal's
/// KiCad name, unit and range (an operating point's value; AC's magnitudes), with the values of
/// those `wanted` (`[x, y]`, AC's `[f, magnitude, phase°]`; at most 500, evenly picked).
fn analysis(plot: &Plot, deck: &Deck, wanted: &[&str]) -> String {
    let num = |v: f64| if v.is_finite() { v.to_string() } else { "null".into() };
    let (sweep, ys) = match plot.vectors.split_first() {
        Some((x, ys)) if !plot.name.starts_with("op") => (Some(x), ys),
        _ => (None, &plot.vectors[..]),
    };
    let x = sweep.map_or(String::new(), |x| {
        let (from, to) = (x.re.first().copied().unwrap_or(0.0), x.re.last().copied().unwrap_or(0.0));
        format!(
            r#","x":{{"name":{},"unit":{},"from":{},"to":{},"points":{}}}"#,
            json_str(&x.name),
            json_str(x.unit),
            num(from),
            num(to),
            x.re.len()
        )
    });
    let signals = ys.iter().map(|y| {
        let name = deck.signals.get(&y.name).cloned().unwrap_or_else(|| y.name.clone());
        let head = format!(r#""name":{},"unit":{}"#, json_str(&name), json_str(y.unit));
        let Some(x) = sweep else { return format!(r#"{{{head},"value":{}}}"#, num(y.re.first().copied().unwrap_or(f64::NAN))) };
        let size: Vec<f64> = if y.im.is_empty() { y.re.clone() } else { y.re.iter().zip(&y.im).map(|(re, im)| re.hypot(*im)).collect() };
        let (lo, hi) = size.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        let values = match wanted.iter().any(|w| name == *w || name == format!("V({w})") || name == format!("I({w})")) {
            false => String::new(),
            true => {
                let at = |i: usize| match y.im.get(i) {
                    Some(im) => format!("[{},{},{}]", num(x.re[i]), num(size[i]), num(im.atan2(y.re[i]).to_degrees())),
                    None => format!("[{},{}]", num(x.re[i]), num(y.re[i])),
                };
                format!(r#","values":{}"#, list((0..size.len().min(x.re.len())).step_by(size.len().div_ceil(500).max(1)).map(at)))
            }
        };
        format!(r#"{{{head},"min":{},"max":{},"last":{}{values}}}"#, num(lo), num(hi), num(size.last().copied().unwrap_or(f64::NAN)))
    });
    format!(r#"{{"analysis":{}{x},"signals":{}}}"#, json_str(&plot.name), list(signals))
}

/// Where `add` puts a symbol or label: `at` [x_mm, y_mm], turned `angle` degrees (default 0).
fn at_angle(args: &Val) -> Result<(Pt, f64), String> {
    let at = match args.arr("at") {
        Some([Val::Num(x), Val::Num(y)]) if x.is_finite() && y.is_finite() => mm_pt(*x, *y),
        _ => return Err("at must be [x_mm, y_mm]".into()),
    };
    match args.get("angle") {
        None => Ok((at, 0.0)),
        Some(Val::Num(a)) if a.is_finite() => Ok((at, *a)),
        Some(_) => Err("angle must be a number of degrees".into()),
    }
}

/// The page of sheet `q` (its path of sheet names, `/Power`, or its file), the root's without one.
fn page(s: &Sheets, q: Option<&str>) -> Result<usize, String> {
    let Some(q) = q else { return Ok(0) };
    (s.project.pages.iter().position(|p| format!("/{}", p.path.join("/")) == q || p.file == q))
        .ok_or_else(|| format!("no sheet {q}: give its path of sheet names (/Power) or its file (power.kicad_sch)"))
}

/// Part `designator`'s shown pins, each unit's on its sheet, as agents wire them: number, name,
/// KiCad's electrical type, the point a wire or label joins and the way it faces; with its
/// unit's reference for a part of several units.
fn pins_json(p: &Project, designator: &str) -> String {
    let units: Vec<_> =
        p.pages.iter().flat_map(|page| page.sheet.symbols.iter().enumerate().map(move |(i, s)| (&page.sheet, i, s))).collect();
    let units: Vec<_> = units.into_iter().filter(|(_, _, s)| s.designator == designator).collect();
    let pins = units.iter().flat_map(|&(sheet, i, s)| {
        let unit = if units.len() > 1 { format!(r#","unit":{}"#, json_str(&s.reference)) } else { String::new() };
        sheet.pin_ends(i).into_iter().map(move |p| {
            format!(
                r#"{{"number":{},"name":{},"type":"{}","at_mm":[{},{}],"facing":"{}"{unit}}}"#,
                json_str(&p.number),
                json_str(&p.name),
                p.kind,
                p.at.x as f64 / 1e6,
                p.at.y as f64 / 1e6,
                p.facing
            )
        })
    });
    list(pins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cuprita_core::library::LibPart;

    #[test]
    fn cli_checks_the_schematic() {
        let dir = std::env::temp_dir().join(format!("cuprita-erc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // An input that connects to nothing: not connected, and not driven.
        let lib = r#"(lib_symbols (symbol "T:IN" (symbol "IN_1_1" (pin input line (at 0 0 0) (length 1) (name "EN") (number "1")))))"#;
        let part = r#"(symbol (lib_id "T:IN") (at 10 10 0) (unit 1) (property "Reference" "U1" (at 0 0 0)))"#;
        std::fs::write(dir.join("b.kicad_sch"), format!(r#"(kicad_sch (version 20260306) (uuid "r") {lib} {part})"#)).unwrap();
        let mut s = Session::default();
        let err = |s: &mut Session, c: &str| Val::parse(&s.cli_line(c)).unwrap().str("error").map(String::from);
        assert_eq!(err(&mut s, r#"{"cmd":"erc"}"#).as_deref(), Some("no board open: give path, a .kicad_sch of the project"));
        let out =
            Val::parse(&s.cli_line(&format!(r#"{{"cmd":"erc","path":{}}}"#, json_str(&dir.join("b.kicad_sch").display().to_string()))))
                .unwrap();
        assert_eq!((out.num("sheets"), out.num("errors"), out.num("warnings")), (Some(1.0), Some(2.0), Some(0.0)));
        let v = &out.arr("violations").unwrap()[1];
        assert_eq!(
            (v.str("kind"), v.str("sheet"), v.arr("at_mm").map(|a| a.to_vec())),
            (Some("pin_not_driven"), Some("/"), Some(vec![Val::Num(10.0), Val::Num(10.0)]))
        );
        assert_eq!(v.str("what"), Some("U1 pin 1 (EN) is an input, but nothing on its net drives it."));
        // With a board open, its schematic beside it.
        std::fs::write(dir.join("b.kicad_pcb"), "(kicad_pcb)").unwrap();
        s.open(&dir.join("b.kicad_pcb")).unwrap();
        assert_eq!(Val::parse(&s.cli_line(r#"{"cmd":"erc"}"#)).unwrap().num("errors"), Some(2.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_simulates_the_schematic() {
        let dir = std::env::temp_dir().join(format!("cuprita-simulate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A 2 V source over a 3k and 1k divider, pin to pin; MID labels its middle, GND its foot.
        let lib = r#"(lib_symbols (symbol "Device:R" (symbol "R_1_1" (pin passive line (at -2.54 0 0) (length 1.27) (name "") (number "1"))
            (pin passive line (at 2.54 0 180) (length 1.27) (name "") (number "2")))))"#;
        let part = |r: &str, x: f64, value: &str| {
            format!(
                r#"(symbol (lib_id "Device:R") (at {x} 10 0) (unit 1) (property "Reference" "{r}" (at 0 0 0)) (property "Value" "{value}" (at 0 0 0)))"#
            )
        };
        let wiring = r#"(wire (pts (xy 22.7 10) (xy 22.7 15))) (wire (pts (xy 22.7 15) (xy 7.46 15))) (wire (pts (xy 7.46 15) (xy 7.46 10)))
            (wire (pts (xy 17.62 10) (xy 17.62 5))) (label "MID" (at 17.62 5 0))
            (wire (pts (xy 12.54 10) (xy 12.54 5))) (global_label "GND" (shape input) (at 12.54 5 0))
            (text ".op\n.tran 0.1m 1m" (at 0 30 0))"#;
        let parts = [part("V1", 10.0, "2"), part("R1", 15.08, "3k"), part("R2", 20.16, "1k")].concat();
        std::fs::write(dir.join("d.kicad_sch"), format!(r#"(kicad_sch (version 20260306) (uuid "r") {lib} {parts} {wiring})"#)).unwrap();
        let mut s = Session::default();
        let path = json_str(&dir.join("d.kicad_sch").display().to_string());
        let out = Val::parse(&s.cli_line(&format!(r#"{{"cmd":"simulate","path":{path},"signals":["/MID"]}}"#))).unwrap();
        if let (Some(e), false) = (out.str("error"), ngspice::loads()) {
            let _ = std::fs::remove_dir_all(&dir);
            return eprintln!("{e}");
        }
        let analyses = out.arr("analyses").unwrap_or_else(|| panic!("{}", out.to_json()));
        let signal = |a: &Val, name: &str| a.arr("signals").unwrap().iter().find(|s| s.str("name") == Some(name)).cloned().unwrap();
        // The operating point: the divider's 1.5 V, 0.5 mA through the source; then the transient.
        assert_eq!(analyses[0].str("analysis"), Some("op1"));
        assert_eq!(signal(&analyses[0], "V(/MID)").num("value"), Some(1.5));
        assert!((signal(&analyses[0], "I(V1)").num("value").unwrap() + 0.5e-3).abs() < 1e-12);
        let x = analyses[1].get("x").unwrap();
        assert_eq!((x.str("name"), x.str("unit"), x.num("to")), (Some("time"), Some("s"), Some(1e-3)));
        let mid = signal(&analyses[1], "V(/MID)");
        assert_eq!((mid.num("min"), mid.num("max"), mid.num("last")), (Some(1.5), Some(1.5), Some(1.5)));
        assert!(mid.arr("values").is_some_and(|v| v.len() > 2) && signal(&analyses[1], "V(Net-(R2-Pad2))").get("values").is_none());
        // A part without a model says so.
        std::fs::write(dir.join("d.kicad_sch"), format!(r#"(kicad_sch (version 20260306) (uuid "r") {lib} {})"#, part("U1", 10.0, "x")))
            .unwrap();
        let e = Val::parse(&s.cli_line(&format!(r#"{{"cmd":"simulate","path":{path}}}"#))).unwrap();
        assert!(e.str("error").is_some_and(|e| e.starts_with("U1: no simulation model")), "{}", e.to_json());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_updates_the_board_from_its_schematic() {
        let dir = std::env::temp_dir().join(format!("cuprita-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // R1 has a footprint of the library, R2 none; the board has neither.
        let lib = r#"(lib_symbols (symbol "Device:R" (symbol "R_1_1" (pin passive line (at -2.54 0 0) (length 1.27) (name "") (number "1"))
            (pin passive line (at 2.54 0 180) (length 1.27) (name "") (number "2")))))"#;
        let r = |r: &str, x: u32, fp: &str| {
            format!(
                r#"(symbol (lib_id "Device:R") (at {x} 10 0) (unit 1) (uuid "{r}") (property "Reference" "{r}" (at 0 0 0)) (property "Footprint" "{fp}" (at 0 0 0)))"#
            )
        };
        let sch = format!(r#"(kicad_sch (version 20260306) (uuid "root") {lib} {} {})"#, r("R1", 10, "Test:R"), r("R2", 30, ""));
        std::fs::write(dir.join("b.kicad_sch"), sch).unwrap();
        std::fs::write(dir.join("b.kicad_pcb"), r#"(kicad_pcb (version 20240108) (layers (0 "F.Cu" signal)) (net 0 ""))"#).unwrap();
        let fp = r#"(footprint "R" (layer "F.Cu") (property "Reference" "REF**" (at 0 -1 0)) (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu"))
            (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))"#;
        std::fs::write(dir.join("R.kicad_mod"), fp).unwrap();
        let wide = fp.replace("\"R\"", "\"R_Wide\"").replace("(at -1 0)", "(at -2 0)").replace("(at 1 0)", "(at 2 0)");
        std::fs::write(dir.join("R_Wide.kicad_mod"), wide).unwrap();
        let part = |name: &str| LibPart {
            lib: "Test".into(),
            name: name.into(),
            path: dir.join(format!("{name}.kicad_mod")),
            about: String::new(),
        };
        let mut s = Session::default();
        let cmd = |s: &mut Session, c: &str| s.cli_line(&format!(r#"{{"cmd":"{c}"}}"#));
        assert_eq!(cmd(&mut s, "update"), r#"{"error":"no board open: open the board to update from its schematic"}"#);
        // Opening a board reads its project's libraries anew: the test's go in after.
        s.open(&dir.join("b.kicad_pcb")).unwrap();
        s.parts = Some(vec![part("R"), part("R_Wide")]);
        let out = Val::parse(&cmd(&mut s, "update")).unwrap();
        let added = out.arr("parts_added").unwrap();
        assert_eq!((added.len(), added[0].str("reference"), out.num("nets_added")), (1, Some("R1"), Some(2.0)));
        let why = "R2 has no footprint: set its Footprint field (KiCad's Assign Footprints)";
        assert_eq!(out.arr("not_added").map(|w| w.to_vec()), Some(vec![Val::Str(why.into())]));
        // One undo step; saved, the board holds R1 linked to its symbol, its pads on their nets.
        assert_eq!(cmd(&mut s, "undo"), r#"{"undone":"Updated from the schematic: 1 part added below the board, 2 nets added"}"#);
        cmd(&mut s, "redo");
        cmd(&mut s, "save");
        let text = std::fs::read_to_string(dir.join("b.kicad_pcb")).unwrap();
        assert!(text.contains(r#"(path "/R1")"#) && text.contains(r#"(net 2 "unconnected-(R1-Pad2)")"#), "{text}");
        // A value and a part number set in the schematic go onto R1.
        s.cli_line(r#"{"cmd":"field","reference":"R1","name":"Value","value":"4k7"}"#);
        s.cli_line(r#"{"cmd":"field","reference":"R1","name":"MPN","value":"RC0603FR-074K7L"}"#);
        let out = Val::parse(&cmd(&mut s, "update")).unwrap();
        assert_eq!((out.num("values_changed"), out.num("fields_changed")), (Some(1.0), Some(1.0)));
        assert_eq!(cmd(&mut s, "undo"), r#"{"undone":"Updated from the schematic: 1 value changed, 1 field changed"}"#);
        // Another footprint named for R1: it comes in its place, its pads 4 mm apart.
        s.cli_line(r#"{"cmd":"field","reference":"R1","name":"Footprint","value":"Test:R_Wide"}"#);
        let out = Val::parse(&cmd(&mut s, "update")).unwrap();
        assert_eq!((out.num("footprints_changed"), out.num("pads_moved")), (Some(1.0), Some(0.0)));
        let r1 = Val::parse(&s.cli_line(r#"{"cmd":"find","query":"R1"}"#)).unwrap();
        assert_eq!(r1.str("footprint"), Some("Test:R_Wide"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_adds_symbols_wires_and_labels_to_the_schematic_undoes_and_saves_them() {
        let dir = std::env::temp_dir().join(format!("cuprita-sym-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sch = r#"(kicad_sch (version 20260306) (uuid "r") (lib_symbols))"#;
        std::fs::write(dir.join("b.kicad_sch"), sch).unwrap();
        std::fs::write(dir.join("b.kicad_pcb"), r#"(kicad_pcb (version 20240108) (footprint "R" (at 10 20) (property "Reference" "R1")))"#)
            .unwrap();
        let lib = r#"(kicad_symbol_lib (symbol "R" (property "Reference" "R" (at 2.032 0 90)) (symbol "R_1_1"
              (pin passive line (at 0 3.81 270) (length 1.27) (name "") (number "1"))
              (pin passive line (at 0 -3.81 90) (length 1.27) (name "") (number "2")))))"#;
        std::fs::write(dir.join("Device.kicad_sym"), lib).unwrap();
        let part =
            |name: &str| LibPart { lib: "Device".into(), name: name.into(), path: dir.join("Device.kicad_sym"), about: "Resistor".into() };
        let mut s = Session::default();
        s.open(&dir.join("b.kicad_pcb")).unwrap();
        s.symbols = Some(vec![part("R"), part("R_Small")]);
        let add = |s: &mut Session, args: &str| s.cli_line(&format!(r#"{{"cmd":"add","symbol":{args}}}"#));
        // Its origin on the grid; the next free reference of the schematic, not the board's.
        assert_eq!(
            add(&mut s, r#""r","at":[25.5,25.3],"angle":90"#),
            concat!(
                r#"{"reference":"R1","symbol":"Device:R","sheet":"/","at_mm":[25.4,25.4],"angle":90,"units":1,"pins":["#,
                r#"{"number":"1","name":"","type":"passive","at_mm":[21.59,25.4],"facing":"left"},"#,
                r#"{"number":"2","name":"","type":"passive","at_mm":[29.21,25.4],"facing":"right"}]}"#
            )
        );
        for (args, error) in [
            (r#""resistor","at":[0,0]"#, "2 symbols match resistor: Device:R, Device:R_Small…; give one as Library:Name"),
            (
                r#""LM7805","at":[0,0]"#,
                "no KiCad library symbol matches LM7805; give words of Library:Name or of what it is (LM358, regulator 3.3)",
            ),
            (r#""R","at":[0,0],"sheet":"/Power""#, "no sheet /Power: give its path of sheet names (/Power) or its file (power.kicad_sch)"),
            (r#""R","at":[0,0],"angle":45"#, "angle 45: a symbol turns by 0, 90, 180 or 270 degrees"),
        ] {
            assert_eq!(add(&mut s, args), format!(r#"{{"error":{}}}"#, json_str(error)));
        }
        // ERC checks it as edited: its two pins connect to nothing.
        let erc = |s: &mut Session| Val::parse(&s.cli_line(r#"{"cmd":"erc"}"#)).unwrap().arr("violations").unwrap().len();
        let cmd_undo = |s: &mut Session| s.cli_line(r#"{"cmd":"undo"}"#);
        assert_eq!(erc(&mut s), 2);
        // A no-connect flag on a pin lets it be; undone, the pin connects to nothing again.
        let flag = |s: &mut Session, at: &str| s.cli_line(&format!(r#"{{"cmd":"add","no_connect":true,"at":{at}}}"#));
        assert_eq!(flag(&mut s, "[21.6,25.4]"), r#"{"no_connect":"R1 pin 1","sheet":"/","at_mm":[21.59,25.4]}"#);
        assert_eq!(erc(&mut s), 1);
        let error = r#"{"error":"no pin ends at 0 0 mm: a no-connect flag goes on a pin's end"}"#;
        assert_eq!(
            (flag(&mut s, "[0,0]"), s.cli_line(r#"{"cmd":"undo"}"#)),
            (error.into(), r#"{"undone":"No-connect flag added"}"#.into())
        );
        assert_eq!(erc(&mut s), 2);
        // Connected by name, both pins get a label reading away from the part: joined, as one edit.
        let connect = |s: &mut Session, args: &str| s.cli_line(&format!(r#"{{"cmd":"connect",{args}}}"#));
        assert_eq!(
            connect(&mut s, r#""net":"LOOP","pins":["R1:1","R1:2"]"#),
            r#"{"net":"LOOP","labeled":[{"pin":"R1:1","at_mm":[21.59,25.4]},{"pin":"R1:2","at_mm":[29.21,25.4]}]}"#
        );
        assert_eq!((erc(&mut s), cmd_undo(&mut s), erc(&mut s)), (0, r#"{"undone":"LOOP connected"}"#.into(), 2));
        for (args, error) in [
            (r#""net":"X","pins":[]"#, r#"give pins: ["U1:VIN", "C3:1"], each a part's reference and a pin's number or name"#),
            (r#""net":"X","pins":["R1"]"#, "pin R1: give it as reference:pin (U1:VIN, C3:1)"),
            (r#""net":"X","pins":["U9:1"]"#, "no part U9 on the sheets"),
            (r#""net":"X","pins":["R1:3"]"#, "R1 has no pin 3: its pins are 1, 2"),
        ] {
            assert_eq!(connect(&mut s, args), format!(r#"{{"error":{}}}"#, json_str(error)));
        }
        // A wire round it from pin to pin (erc says where they are), on the grid: both connected.
        let wire = |s: &mut Session, args: &str| s.cli_line(&format!(r#"{{"cmd":"wire",{args}}}"#));
        assert_eq!(
            wire(&mut s, r#""points":[[21.6,25.4],[21.59,20.3],[21.59,20.32],[29.2,20.32],[29.21,25.4]]"#),
            r#"{"wire":[[21.59,25.4],[21.59,20.32],[29.21,20.32],[29.21,25.4]],"sheet":"/"}"#
        );
        assert_eq!(erc(&mut s), 0);
        // Its PDF, with the edits: one sheet, beside the root.
        let pdf = Val::parse(&s.cli_line(r#"{"cmd":"pdf"}"#)).unwrap();
        assert_eq!((pdf.num("sheets"), pdf.str("file")), (Some(1.0), Some(dir.join("b.pdf").to_str().unwrap())));
        assert!(std::fs::read(dir.join("b.pdf")).unwrap().starts_with(b"%PDF-1.4"));
        for (args, error) in [
            (r#""points":[[0,0]]"#, "points must be [[x_mm, y_mm], …], two or more"),
            (r#""points":[[0,0],[0.1,0]]"#, "a wire needs two points at least one grid step (50 mil) apart"),
            (
                r#""points":[[0,0],[5,0]],"sheet":"/Power""#,
                "no sheet /Power: give its path of sheet names (/Power) or its file (power.kicad_sch)",
            ),
        ] {
            assert_eq!(wire(&mut s, args), format!(r#"{{"error":{}}}"#, json_str(error)));
        }
        // A global label on the wire names its net: its name trimmed, on the grid.
        let label = |s: &mut Session, args: &str| s.cli_line(&format!(r#"{{"cmd":"add","label":{args}}}"#));
        assert_eq!(
            label(&mut s, r#"" SDA ","global":true,"at":[25.4,20.3],"angle":180"#),
            r#"{"label":"SDA","sheet":"/","at_mm":[25.4,20.32],"angle":180}"#
        );
        for (args, error) in [
            (r#"" ","at":[0,0]"#, r#"label "": give the net's name, one line of text (SDA, ~{RESET})"#),
            (r#"5,"at":[0,0]"#, "label must be the net's name (SDA)"),
            (r#""SDA","at":[0,0],"angle":45"#, "angle 45: a label turns by 0, 90, 180 or 270 degrees"),
        ] {
            assert_eq!(label(&mut s, args), format!(r#"{{"error":{}}}"#, json_str(error)));
        }
        assert_eq!(erc(&mut s), 0);
        // Undo and redo go back and forth through the session's edits, board or schematic.
        s.cli_line(r#"{"cmd":"place","reference":"R1","at":[12.5,20]}"#);
        let cmd = |s: &mut Session, c: &str| s.cli_line(&format!(r#"{{"cmd":"{c}"}}"#));
        let steps = ["R1 placed", "Global label SDA added", "Wire added", "R1 added"];
        assert_eq!(steps.map(|_| cmd(&mut s, "undo")), steps.map(|w| format!(r#"{{"undone":"{w}"}}"#)));
        assert_eq!((erc(&mut s), cmd(&mut s, "undo")), (0, r#"{"error":"nothing to undo"}"#.into()));
        let mut redone = steps.map(|_| cmd(&mut s, "redo"));
        redone.reverse();
        assert_eq!(redone, steps.map(|w| format!(r#"{{"redone":"{w}"}}"#)));
        // Saved, the sheet holds it, its previous version kept.
        let saved = Val::parse(&cmd(&mut s, "save")).unwrap();
        assert_eq!(saved.arr("sheets").map(|f| f.to_vec()), Some(vec![Val::Str("b.kicad_sch".into())]));
        let text = std::fs::read_to_string(dir.join("b.kicad_sch")).unwrap();
        assert!(text.contains("(lib_id \"Device:R\")\n\t\t(at 25.4 25.4 90)") && text.contains("(xy 21.59 20.32) (xy 29.21 20.32)"));
        assert!(text.contains("(global_label \"SDA\"\n\t\t(shape input)\n\t\t(at 25.4 20.32 180)"));
        assert_eq!(std::fs::read_to_string(dir.join("b.kicad_sch-bak")).unwrap(), sch);
        // Nothing left to save, the next edit starts from the file, as KiCad may have changed it.
        std::fs::write(dir.join("b.kicad_sch"), text.replace("\"R1\"", "\"R7\"")).unwrap();
        assert!(add(&mut s, r#""R","at":[50,50]"#).starts_with(r#"{"reference":"R8""#));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_adds_hierarchical_labels_with_their_sheet_pins() {
        let dir = std::env::temp_dir().join(format!("cuprita-hier-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sheet = r#"(sheet (at 101.6 0) (size 20.32 10.16) (uuid "s1") (property "Sheetname" "Sub" (at 101.6 0 0))
            (property "Sheetfile" "sub.kicad_sch" (at 101.6 10.16 0)))"#;
        std::fs::write(dir.join("b.kicad_sch"), format!(r#"(kicad_sch (version 20260306) (uuid "r") (lib_symbols) {sheet})"#)).unwrap();
        std::fs::write(dir.join("sub.kicad_sch"), r#"(kicad_sch (version 20260306) (uuid "u") (lib_symbols))"#).unwrap();
        let mut s = Session::default();
        let path = json_str(&dir.join("b.kicad_sch").display().to_string());
        let add = |s: &mut Session, args: &str| s.cli_line(&format!(r#"{{"cmd":"add","path":{path},{args}}}"#));
        let added = add(&mut s, r#""hier_label":"EN","shape":"input","sheet":"/Sub","at":[10,10]"#);
        assert_eq!(added, r#"{"label":"EN","sheet":"/Sub","at_mm":[10.16,10.16],"angle":0}"#);
        assert!(add(&mut s, r#""hier_label":"EN","at":[0,0]"#).contains("the root sheet has no sheet above it"));
        // Saved, the sheet has the label and its box in the root the pin, on its left as an input.
        let saved = Val::parse(&s.cli_line(r#"{"cmd":"save"}"#)).unwrap();
        let both = vec![Val::Str("b.kicad_sch".into()), Val::Str("sub.kicad_sch".into())];
        assert_eq!(saved.arr("sheets").map(|f| f.to_vec()), Some(both));
        let (root, sub) =
            (std::fs::read_to_string(dir.join("b.kicad_sch")).unwrap(), std::fs::read_to_string(dir.join("sub.kicad_sch")).unwrap());
        assert!(
            root.contains("(pin \"EN\" input\n\t\t\t(at 101.6 2.54 180)") && sub.contains("(hierarchical_label \"EN\"\n\t\t(shape input)")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_adds_sheets_and_saves_their_files() {
        let dir = std::env::temp_dir().join(format!("cuprita-sheets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = "(kicad_sch\n\t(version 20260306)\n\t(uuid \"r\")\n\t(paper \"A3\")\n\t(lib_symbols)\n\t(sheet_instances\n\t\t(path \"/\"\n\t\t\t(page \"1\")\n\t\t)\n\t)\n)";
        std::fs::write(dir.join("b.kicad_sch"), root).unwrap();
        let (mut s, path) = (Session::default(), json_str(&dir.join("b.kicad_sch").display().to_string()));
        let run = |s: &mut Session, cmd: &str, args: &str| s.cli_line(&format!(r#"{{"cmd":"{cmd}","path":{path}{args}}}"#));
        // Its file named after it, its box on the grid.
        let added = run(&mut s, "sheet", r#","name":"Power","at":[25.4,20.3],"size":[25.4,20.32]"#);
        assert_eq!(added, r#"{"sheet":"/Power","file":"power.kicad_sch","at_mm":[25.4,20.32],"size_mm":[25.4,20.32]}"#);
        assert_eq!(run(&mut s, "sheet", r#","name":"Power","at":[0,0]"#), r#"{"error":"size must be [width_mm, height_mm]"}"#);
        // Undone, nothing is saved, no file made; redone and given a label, saving writes both.
        run(&mut s, "undo", "");
        assert!(run(&mut s, "save", "").contains("nothing to save") && !dir.join("power.kicad_sch").exists());
        run(&mut s, "redo", "");
        run(&mut s, "add", r#","hier_label":"EN","sheet":"/Power","at":[10.16,10.16]"#);
        let saved = Val::parse(&run(&mut s, "save", "")).unwrap();
        let both = vec![Val::Str("b.kicad_sch".into()), Val::Str("power.kicad_sch".into())];
        assert_eq!(saved.arr("sheets").map(|f| f.to_vec()), Some(both));
        let new = std::fs::read_to_string(dir.join("power.kicad_sch")).unwrap();
        assert!(
            new.starts_with("(kicad_sch\n\t(version 20260306)")
                && new.contains("(paper \"A3\")")
                && new.contains("(hierarchical_label \"EN\"")
        );
        // The same file again, shown as it is: the project has three pages.
        run(&mut s, "sheet", r#","name":"Power 2","file":"power.kicad_sch","at":[60.96,20.32],"size":[20.32,20.32]"#);
        assert_eq!(Val::parse(&run(&mut s, "erc", "")).unwrap().num("sheets"), Some(3.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_draws_buses_and_entries() {
        let dir = std::env::temp_dir().join(format!("cuprita-bus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.kicad_sch"), r#"(kicad_sch (version 20260306) (uuid "r") (lib_symbols))"#).unwrap();
        let (mut s, path) = (Session::default(), json_str(&dir.join("b.kicad_sch").display().to_string()));
        let run = |s: &mut Session, cmd: &str, args: &str| s.cli_line(&format!(r#"{{"cmd":"{cmd}","path":{path}{args}}}"#));
        assert_eq!(run(&mut s, "bus", r#","points":[[20.3,0],[20.32,30.5]]"#), r#"{"bus":[[20.32,0],[20.32,30.48]],"sheet":"/"}"#);
        assert_eq!(
            run(&mut s, "add", r#","bus_entry":true,"at":[17.78,10.16],"angle":90"#),
            r#"{"bus_entry":[17.78,10.16],"sheet":"/","angle":90}"#
        );
        assert!(run(&mut s, "add", r#","bus_entry":true,"at":[0,0],"angle":45"#).contains("a bus entry turns by 0, 90, 180 or 270"));
        // Undone, the entry goes first.
        assert_eq!(run(&mut s, "undo", ""), r#"{"undone":"Bus entry added"}"#);
        assert_eq!(run(&mut s, "undo", ""), r#"{"undone":"Bus added"}"#);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
