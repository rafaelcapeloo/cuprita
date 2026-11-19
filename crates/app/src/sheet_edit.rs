//! Editing the schematic: `Sheets`, a project's schematic as the app or an agent edits it (undo,
//! save, autosave); in the app, add a symbol from KiCad's libraries (A: it follows the pointer on
//! the grid, R turns it, a click puts it down, Esc stops), draw wires (W: each click adds one to
//! the pointer, a click on a pin or wire ends it), name nets (L, Ctrl+L global: type a label's
//! name, then place it as a symbol), drag, turn (R) and delete (Delete) what is picked, undo and
//! redo (Ctrl+Z, Ctrl+Y), and save the edited sheets (Ctrl+S, each previous version kept as
//! `<file>-bak`).

use crate::file::{autosave_age, autosave_path, save_file, write_whole};
use crate::sheet::{Sch, read_project};
use crate::tr::{n, t, tr, trf};
use crate::{App, color32, count, focus};
use cuprita_core::board::Pt;
use cuprita_core::history::History;
use cuprita_core::library::{self, LibPart};
use cuprita_core::schematic::{Ink, Item, LabelKind, Patch, Project, on_grid};
use eframe::egui;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use web_time::Instant; // std's panics in the browser

/// What to do when no KiCad symbol library was found.
pub(crate) const NO_SYMBOLS: &str = t!("no KiCad symbol libraries found: install KiCad, or set KICAD10_SYMBOL_DIR to its symbols folder");

/// A project's schematic as it is edited, in the app or by agents: read from root sheet `root`,
/// with its edits to undo and redo and the sheet files they touched since it was saved.
#[derive(Clone)]
pub(crate) struct Sheets {
    pub(crate) root: PathBuf,
    pub(crate) project: Project,
    pub(crate) history: History<Patch>,
    edited: BTreeSet<String>,
}

impl Sheets {
    /// Reads the schematic of the project whose root sheet is `root`, its sheet files `restored`
    /// from their autosaves (`_autosave-<file>`), whose edits the files lack.
    pub(crate) fn read(root: &Path, restored: &[String]) -> Result<Sheets, String> {
        let project = read_project(root, restored)?;
        let history = if restored.is_empty() { History::default() } else { History::restored() };
        let mut s = Sheets { root: root.to_path_buf(), project, history, edited: restored.iter().cloned().collect() };
        // Sheets made since the last save came from their autosaves too.
        let new: Vec<String> =
            s.project.pages.iter().map(|p| p.file.clone()).filter(|f| crate::file::written(&s.path(f)).is_none()).collect();
        s.edited.extend(new);
        Ok(s)
    }

    pub(crate) fn path(&self, file: &str) -> PathBuf {
        self.root.parent().unwrap_or(Path::new(".")).join(file)
    }

    /// The sheet files with an autosave newer than them, and how old the newest is.
    pub(crate) fn autosaved(&self) -> Option<(Vec<String>, Duration)> {
        let files: BTreeSet<&str> = self.project.pages.iter().map(|p| p.file.as_str()).collect();
        let ages: Vec<(String, Duration)> = files.into_iter().filter_map(|f| Some((f.to_string(), autosave_age(&self.path(f))?))).collect();
        let newest = ages.iter().map(|a| a.1).min()?;
        Some((ages.into_iter().map(|a| a.0).collect(), newest))
    }

    /// Writes each edited sheet to its autosave; back as saved, their autosaves go instead.
    pub(crate) fn autosave(&self) -> Result<(), String> {
        if !self.history.unsaved() {
            self.drop_autosaves();
            return Ok(());
        }
        let texts = self.edited.iter().filter_map(|f| Some((f, self.project.text(f).filter(|t| !t.is_empty())?)));
        texts.into_iter().try_for_each(|(f, text)| write_whole(&autosave_path(&self.path(f)), text))
    }

    /// Takes the edited sheets' autosaves away.
    pub(crate) fn drop_autosaves(&self) {
        for f in &self.edited {
            let _ = std::fs::remove_file(autosave_path(&self.path(f)));
        }
    }

    /// Keeps an edit made: `undo` undoes it.
    pub(crate) fn push(&mut self, undo: Patch, what: String) {
        self.edited.extend(undo.files().map(String::from));
        self.history.push(undo, what);
    }

    /// Undoes the last edit, or redoes the last one undone: what it did, `None` when there is none.
    pub(crate) fn undo(&mut self, redo: bool) -> Option<String> {
        let h = &mut self.history;
        let (what, back) = if redo { h.redo(&mut self.project) } else { h.undo(&mut self.project) }?;
        self.edited.extend(back.files().map(String::from));
        Some(what.to_string())
    }

    /// Writes the edited sheets over their files, each previous version kept as `<file>-bak`
    /// first: the files written, or what went wrong (those not written stay edited).
    pub(crate) fn save(&mut self) -> Result<Vec<String>, String> {
        let (mut saved, mut failed) = (vec![], None);
        for file in std::mem::take(&mut self.edited) {
            // An edit changes only sheets that were read, or made: one made, then undone, is empty.
            let Some(text) = self.project.text(&file).filter(|t| !t.is_empty()) else { continue };
            match save_file(&self.path(&file), text.as_bytes()) {
                Ok(_) => {
                    let _ = std::fs::remove_file(autosave_path(&self.path(&file)));
                    saved.push(file);
                }
                Err(e) => {
                    failed.get_or_insert(e);
                    self.edited.insert(file);
                }
            }
        }
        match failed {
            None => {
                self.history.mark_saved();
                Ok(saved)
            }
            Some(e) => Err(e),
        }
    }
}

/// The label tool's name field.
pub(crate) const LABEL_FIELD: &str = "label name";

/// What the tools say while on: their command's name, then `: ` and what to do (`tool_hint` shows
/// the name apart, so a translation starts with the name's own).
const WIRE: &str = t!("Wire: click to click · / turns its corner, Shift goes straight · a pin, a wire or Enter ends it · Esc stops");
const NO_CONNECT: &str =
    t!("No connect: a click flags the pin there as left unconnected on purpose, a click on a flag takes it away; Esc stops");
const BUS: &str = t!("Bus: click to click · a bus, an entry, a label or Enter ends it · Esc stops");
const ENTRY: &str = t!("Bus entry: a click puts one down, R turns it, Esc stops");
const SHEET: &str = t!("Sheet: drag its box on the sheet; Esc stops");

/// A sheet being added (Shift+S): its box's corners as dragged (on the grid), then, drawn, its
/// name and file as typed (the file following the name until it is typed).
pub(crate) struct NewSheet {
    pub(crate) corners: Option<(Pt, Pt)>,
    pub(crate) drawn: bool,
    name: String,
    file: String,
    typed: bool,
    /// Whether its name field takes the keys when it next shows.
    focus: bool,
}

/// A sheet's file named after it, as words joined by `_`: `RAM + LVDS` → `ram_lvds.kicad_sch`.
pub(crate) fn file_for(name: &str) -> String {
    let words: Vec<String> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    format!("{}.kicad_sch", if words.is_empty() { "sheet".into() } else { words.join("_") })
}

impl Sch {
    /// The part picked: when what is picked is its symbols, it alone.
    pub(crate) fn part(&self) -> Option<&str> {
        let symbols = &self.sheets.project.pages[self.page].sheet.symbols;
        let mut parts = self.picked.iter().map(|&it| if let Item::Symbol(i) = it { Some(&*symbols[i].designator) } else { None });
        let first = parts.next()??;
        parts.all(|p| p == Some(first)).then_some(first)
    }

    /// What is picked, in words: its part (`U3`), the item (`Wire`, `Label SDA`), or how many of
    /// each (`3 symbols, 4 wires`).
    pub(crate) fn what(&self) -> String {
        let sheet = &self.sheets.project.pages[self.page].sheet;
        if let Some(part) = self.part() {
            return part.into();
        }
        if let [one] = self.picked[..] {
            return sheet.describe(one);
        }
        let n = |kind: fn(&Item) -> bool| self.picked.iter().filter(|it| kind(it)).count();
        let kinds = [
            (n(|it| matches!(it, Item::Symbol(_))), n!("symbol")),
            (n(|it| matches!(it, Item::Wire(_))), n!("wire")),
            (n(|it| matches!(it, Item::Label(_))), n!("label")),
            (n(|it| matches!(it, Item::NoConnect(_))), n!("no-connect flag")),
        ];
        kinds.iter().filter(|(k, _)| *k > 0).map(|&(k, one)| count(k, one)).collect::<Vec<_>>().join(", ")
    }

    /// What is picked as it shows: moved while dragged.
    pub(crate) fn marked(&self) -> &[Item] {
        self.drag.as_ref().and_then(|d| d.shown.as_ref()).map_or(&self.picked, |s| &s.1)
    }

    /// A picked label that names nothing where it shows, and what to do.
    fn alone(&self) -> Option<String> {
        let sheet = &self.sheets.project.pages[self.page].sheet;
        let label = self.marked().iter().find(|&&it| matches!(it, Item::Label(i) if !sheet.names(i)))?;
        Some(trf("{0} names nothing there: put it on a pin or a wire", &[&sheet.describe(*label)]))
    }

    /// Keeps an edit made (`undo` undoes it, `what` says what it did) and checks the project again.
    fn edited(&mut self, undo: Patch, what: String) {
        let t = Instant::now();
        self.sheets.push(undo, what.clone());
        self.changed = Some(Instant::now());
        self.violations = self.sheets.project.erc();
        self.status = trf("{0}, checked in {1}", &[&what, &format!("{:.0?}", t.elapsed())]);
    }

    /// The reference of the symbol in hand: shown at the pointer, its page's last, not put down.
    pub(crate) fn in_hand(&self) -> Option<&str> {
        let Placing { item: Placeable::Symbol(..), shown: Some(_), .. } = self.placing.as_ref()? else { return None };
        self.sheets.project.pages[self.page].sheet.symbols.last().map(|s| s.designator.as_str())
    }
}

/// The wire from `a` to `b` along the grid: bent once, the longer way first.
fn wire_path(a: Pt, b: Pt, swap: bool, straight: bool) -> [Pt; 3] {
    let longer_across = (b.x - a.x).abs() >= (b.y - a.y).abs();
    let corner = match (straight, longer_across != swap) {
        (true, _) => a,
        (false, true) => Pt { x: b.x, y: a.y },
        (false, false) => Pt { x: a.x, y: b.y },
    };
    [a, corner, b]
}

/// A library symbol or net label being placed: it shows at the pointer until a click puts it down.
pub(crate) struct Placing {
    item: Placeable,
    angle: f64,
    /// Where it shows now, the patch that takes it away again and the edit it makes (`R5 added`,
    /// `Label SDA added`).
    shown: Option<(Pt, Patch, String)>,
    /// A clip's items as the page has them while it shows: picked once it is put down.
    pasted: Vec<Item>,
}

/// The picked items being dragged: where the drag started, the grid step it last reached, and
/// the move shown there (the patch taking it back, the items as moved); none where they can't go.
pub(crate) struct Drag {
    from: Pt,
    by: Pt,
    shown: Option<(Patch, Vec<Item>)>,
}

impl From<Pt> for Drag {
    fn from(from: Pt) -> Drag {
        Drag { from, by: Pt::default(), shown: None }
    }
}

/// What is being placed: a library symbol (its library, its text as sheets copy it, and the
/// fields it takes: a part's at Digi-Key), a net label or a bus entry.
enum Placeable {
    Symbol(String, String, Vec<(&'static str, String)>),
    Label(Naming),
    Entry,
    /// What was copied (Paste, Duplicate), as `Project::copy_items` gives it.
    Clip(String),
}

impl Placing {
    /// Whether it is a bus entry.
    pub(crate) fn entry(&self) -> bool {
        matches!(self.item, Placeable::Entry)
    }

    /// The command placing it: a symbol's, a label's by its kind, a bus entry's.
    pub(crate) fn cmd(&self) -> crate::cmd::Cmd {
        use crate::cmd::Cmd;
        match &self.item {
            Placeable::Symbol(..) => Cmd::Add,
            Placeable::Label((_, LabelKind::Local, _)) => Cmd::Label,
            Placeable::Label((_, LabelKind::Global, _)) => Cmd::GlobalLabel,
            Placeable::Label((_, LabelKind::Hier, _)) => Cmd::HierLabel,
            Placeable::Entry => Cmd::BusEntry,
            Placeable::Clip(_) => Cmd::Paste,
        }
    }
}

/// A net label: its name, its kind and, a global or hierarchical one's, its shape (`SHAPES`).
pub(crate) type Naming = (String, LabelKind, &'static str);

impl App {
    /// Starts placing library symbol `part` (A, then Enter or a click in the list).
    pub(crate) fn pick_symbol(&mut self, part: &LibPart) {
        let read = std::fs::read_to_string(&part.path).map_err(|e| e.to_string());
        match read.and_then(|t| library::symbol(&t, &part.name)) {
            Ok(text) => self.place_symbol(part, text, vec![], part.id()),
            Err(e) => self.sch.iter_mut().for_each(|sch| sch.status = format!("{}: {e}", part.path.display())),
        }
    }

    /// Starts placing symbol `text` of `part`'s library with `fields` set, the status saying `what`.
    pub(crate) fn place_symbol(&mut self, part: &LibPart, text: String, fields: Vec<(&'static str, String)>, what: String) {
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        sch.placing = Some(Placing { item: Placeable::Symbol(part.lib.clone(), text, fields), angle: 0.0, shown: None, pasted: vec![] });
        sch.status = trf("{0}: a click puts it down, R turns it, Esc stops", &[&what]);
    }

    /// Turns the label tool on for a label of `kind` (L, Ctrl+L global, H hierarchical, or the
    /// buttons: a field to type its name in, Enter starts placing it), or off.
    pub(crate) fn label_tool(&mut self, ctx: &egui::Context, kind: LabelKind) {
        let on = self.sch.as_ref().is_some_and(|s| s.naming.as_ref().is_some_and(|n| n.1 == kind));
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        if !on {
            // KiCad's shape for a new hierarchical label; most global labels are inputs.
            let shape = if kind == LabelKind::Hier { "bidirectional" } else { "input" };
            (sch.naming, sch.status) = (Some((String::new(), kind, shape)), tr("Label: type the net's name, then Enter; Esc stops").into());
            focus(ctx, LABEL_FIELD, if kind == LabelKind::Hier { "h" } else { "l" });
        }
    }

    /// Starts placing a net label (Enter in the label tool's field).
    pub(crate) fn pick_label(&mut self, label: Naming) {
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        sch.status = trf("{0}: a click puts it down, R turns it, Esc stops", &[&format!("{} {}", tr(label.1.name()), label.0)]);
        sch.placing = Some(Placing { item: Placeable::Label(label), angle: 0.0, shown: None, pasted: vec![] });
    }

    /// Shows the symbol or label being placed at the grid point nearest `at` (sheet nm), when it
    /// isn't there yet or `again` (turned).
    pub(crate) fn show_placing(&mut self, at: Pt, again: bool) {
        let Some(sch) = &mut self.sch else { return };
        let (Some(p), at) = (&mut sch.placing, on_grid(at)) else { return };
        if !again && p.shown.as_ref().is_some_and(|s| s.0 == at) {
            return;
        }
        if let Some((_, undo, _)) = p.shown.take() {
            sch.sheets.project.apply(undo);
        }
        let project = &mut sch.sheets.project;
        let added = match &p.item {
            Placeable::Symbol(lib, text, fields) => {
                project.add_symbol(sch.page, lib, text, fields, at, p.angle).map(|(undo, what)| (undo, trf("{0} added", &[&what])))
            }
            Placeable::Label((name, kind, shape)) => project
                .add_label(sch.page, name, *kind, shape, at, p.angle)
                .map(|undo| (undo, trf("{0} added", &[&format!("{} {name}", tr(kind.name()))]))),
            Placeable::Entry => project.add_bus_entry(sch.page, at, p.angle).map(|undo| (undo, tr("Bus entry added").into())),
            Placeable::Clip(clip) => project.paste(sch.page, clip, at).map(|(undo, items)| {
                let what = trf("{0} pasted", &[&count(items.len(), n!("item"))]);
                p.pasted = items;
                (undo, what)
            }),
        };
        match added {
            Ok((undo, what)) => p.shown = Some((at, undo, what)),
            Err(e) => (sch.placing, sch.status) = (None, e),
        }
        self.redraw_sch();
    }

    /// Turns the symbol or label being placed 90° counter-clockwise (R).
    pub(crate) fn turn_placing(&mut self) {
        let Some(p) = self.sch.as_mut().and_then(|s| s.placing.as_mut()) else { return };
        p.angle = (p.angle + 90.0) % 360.0;
        if let Some(at) = p.shown.as_ref().map(|s| s.0) {
            self.show_placing(at, true);
        }
    }

    /// Puts the symbol, label or bus entry being placed down where it shows (a click): an edit to
    /// undo; another bus entry follows the pointer.
    pub(crate) fn put_down(&mut self) {
        let Some(sch) = &mut self.sch else { return };
        let Some(Placing { item, angle, shown: Some((_, undo, what)), pasted }) = sch.placing.take() else { return };
        sch.edited(undo, what);
        match item {
            Placeable::Entry => sch.placing = Some(Placing { item, angle, shown: None, pasted }),
            // What was pasted is picked: to move on, turn or mirror.
            Placeable::Clip(_) => sch.picked = pasted,
            _ => {}
        }
    }

    /// Starts placing bus entries (Z): one follows the pointer on the grid, R turns it, each
    /// click puts one down.
    pub(crate) fn place_entry(&mut self) {
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        (sch.placing, sch.status) = (Some(Placing { item: Placeable::Entry, angle: 0.0, shown: None, pasted: vec![] }), tr(ENTRY).into());
    }

    /// Turns the wire tool on for wires or (`bus`) buses, or off when it is on (W, B or the
    /// buttons): the next click starts one, never the pointer's place when the key went down.
    pub(crate) fn wire_tool(&mut self, bus: bool) {
        let on = self.sch.as_ref().is_some_and(|s| s.wire.is_some() && s.bus == bus);
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        if on {
            sch.status.clear();
        } else {
            (sch.wire, sch.bus, sch.status) = (Some(None), bus, tr(if bus { BUS } else { WIRE }).into());
        }
    }

    /// A click with the wire tool at `at`: starts a wire (or bus) there, or adds one to it
    /// (`wire_path`) and goes on from there; it ends where it joins something, or on a second
    /// click.
    pub(crate) fn wire_click(&mut self, at: Pt, straight: bool) {
        let Some(sch) = &mut self.sch else { return };
        let (Some(from), at) = (sch.wire, on_grid(at)) else { return };
        let start = match from {
            None => return sch.wire = Some(Some(at)),
            // A second click where it is: the wire ends.
            Some(s) if s == at => return sch.wire = Some(None),
            Some(s) => s,
        };
        let sheet = &sch.sheets.project.pages[sch.page].sheet;
        let ends = if sch.bus { sheet.joins_bus(at) } else { sheet.joins(at) };
        let (project, path) = (&mut sch.sheets.project, wire_path(start, at, sch.swap_bend, straight));
        let added = if sch.bus { project.add_bus(sch.page, &path) } else { project.add_wire(sch.page, &path) };
        match added {
            Ok(undo) => {
                sch.edited(undo, if sch.bus { tr("Bus added") } else { tr("Wire added") }.into());
                sch.wire = Some((!ends).then_some(at));
                self.redraw_sch();
            }
            Err(e) => sch.status = e,
        }
    }

    /// Turns the no-connect tool on, or off when it is on (Q or the button): clicks flag, the
    /// key alone flags nothing.
    pub(crate) fn no_connect_tool(&mut self) {
        let on = self.sch.as_ref().is_some_and(|s| s.flagging);
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        if on {
            sch.status.clear();
        } else {
            (sch.flagging, sch.status) = (true, tr(NO_CONNECT).into());
        }
    }

    /// A click with the no-connect tool at `at`: flags the pin ending at its grid point, or takes
    /// the flag there away.
    pub(crate) fn flag_pin(&mut self, at: Pt) {
        let Some(sch) = &mut self.sch else { return };
        match sch.sheets.project.toggle_no_connect(sch.page, at) {
            Ok((undo, added)) => {
                sch.edited(undo, tr(if added { t!("No-connect flag added") } else { t!("No-connect flag removed") }).into());
                self.redraw_sch();
            }
            Err(e) => sch.status = e,
        }
    }

    /// Tidy texts (T) on a sheet: every symbol's fields on the page shown beside it
    /// (`Project::tidy_fields`), as one edit.
    pub(crate) fn tidy_fields(&mut self) {
        let Some(sch) = &mut self.sch else { return };
        match sch.sheets.project.tidy_fields(sch.page) {
            Ok((_, 0)) => sch.status = tr("The texts are tidy already: nothing moved").into(),
            Ok((undo, n)) => {
                sch.edited(undo, trf("Texts of {0} tidied", &[&count(n, n!("symbol"))]));
                self.redraw_sch();
            }
            Err(e) => sch.status = e,
        }
    }

    /// Turns the sheet tool on (Shift+S, or the button: drag a box, then name it), or off.
    pub(crate) fn sheet_tool(&mut self) {
        let on = self.sch.as_ref().is_some_and(|s| s.sheeting.is_some());
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        if !on {
            let s = NewSheet { corners: None, drawn: false, name: String::new(), file: String::new(), typed: false, focus: false };
            (sch.sheeting, sch.status) = (Some(s), tr(SHEET).into());
        }
    }

    /// Starts a sheet below the page shown, its box where there is room on it: its name typed,
    /// Enter adds it (the Project tab's New sheet).
    pub(crate) fn new_sheet(&mut self) {
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        let size = (30_480_000, 20_320_000);
        let Some(a) = sch.sheets.project.free_spot(sch.page, size) else {
            return sch.status = tr("No room for a sheet's box on this sheet: drag one where you want it (Shift+S)").into();
        };
        let corners = Some((a, Pt { x: a.x + size.0, y: a.y + size.1 }));
        sch.sheeting = Some(NewSheet { corners, drawn: true, name: String::new(), file: String::new(), typed: false, focus: true });
        sch.status = tr("Sheet: its name and file, then Enter").into();
    }

    /// The sheet tool's drag: from the box's first corner `from` (where it started) to `to`, the
    /// pointer; let go (`done`), the box is drawn and its name and file asked, unless it is flat.
    pub(crate) fn drag_sheet(&mut self, from: Option<Pt>, to: Option<Pt>, done: bool) {
        let Some(sch) = &mut self.sch else { return };
        let Some(s) = &mut sch.sheeting else { return };
        if let Some(p) = from {
            s.corners = Some((on_grid(p), on_grid(p)));
        }
        if let (Some(c), Some(p)) = (&mut s.corners, to) {
            c.1 = on_grid(p);
        }
        match s.corners {
            _ if !done => {}
            Some((a, b)) if a.x != b.x && a.y != b.y => {
                (s.drawn, s.focus, sch.status) = (true, true, tr("Sheet: its name and file, then Enter").into());
            }
            _ => (s.corners, sch.status) = (None, tr("Sheet: drag across, so its box has a width and a height; Esc stops").into()),
        }
    }

    /// The sheet tool's fields, its box drawn: its name and file (the name's until typed); Enter
    /// or Add adds it.
    pub(crate) fn sheet_field(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.sch.as_mut().and_then(|s| s.sheeting.as_mut()) else { return };
        let name = ui.add(egui::TextEdit::singleline(&mut s.name).hint_text(tr("Name")).desired_width(120.0));
        if std::mem::take(&mut s.focus) {
            name.request_focus();
        }
        if name.changed() && !s.typed {
            s.file = file_for(&s.name);
        }
        let file = ui.add(egui::TextEdit::singleline(&mut s.file).hint_text(tr("name.kicad_sch")).desired_width(160.0));
        s.typed |= file.changed();
        let file =
            file.on_hover_text(tr("Its file, beside the root sheet: a new one, or one of the project's (or this folder's) to show again"));
        let enter = (name.lost_focus() || file.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button(tr("Add")).clicked() || enter {
            self.add_sheet();
        }
    }

    /// Adds the sheet drawn, with its name and file: a new sheet, or the file of that name (the
    /// project's, else the folder's) shown again; an edit to undo, the page drawn again.
    pub(crate) fn add_sheet(&mut self) {
        let Some(sch) = &mut self.sch else { return };
        let Some(NewSheet { corners: Some((a, b)), drawn: true, name, file, .. }) = &sch.sheeting else { return };
        let (a, b, name, file) = (*a, *b, name.trim().to_string(), file.trim().to_string());
        let path = sch.sheets.path(&file);
        let existing = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return sch.status = trf("{0} not read: {1}", &[&path.display(), &e]),
        };
        match sch.sheets.project.add_sheet(sch.page, &name, &file, a, b, existing) {
            Ok(undo) => {
                sch.sheeting = None;
                sch.edited(undo, trf("Sheet {0} ({1}) added", &[&name, &file]));
                sch.status = trf("{0}; a double click opens it", &[&sch.status]);
            }
            Err(e) => return sch.status = e,
        }
        self.redraw_sch();
    }

    /// Draws the wire (or bus) the next click adds, from where it goes on from to the pointer's
    /// grid point `at` (a ring there shows it).
    pub(crate) fn draw_wire(&self, ui: &egui::Ui, at: Pt, straight: bool, screen: impl Fn(Pt) -> egui::Pos2) {
        let Some(sch) = &self.sch else { return };
        let Some(wire) = sch.wire else { return };
        let (ink, width) = if sch.bus { (Ink::Bus, 4.0) } else { (Ink::Wire, 2.0) };
        let color = color32(sch.scene.layers[ink as usize + 1].color);
        let at = on_grid(at);
        if let Some(from) = wire {
            ui.painter().add(egui::Shape::line(wire_path(from, at, sch.swap_bend, straight).map(&screen).to_vec(), (width, color)));
        }
    }

    /// Stops placing a symbol or label (the one shown goes), a drag (what it moved goes back), a
    /// box and the wire and label tools: Esc, another page or tool, undo, the board.
    pub(crate) fn stop_tool(&mut self) {
        let Some(sch) = &mut self.sch else { return };
        (sch.wire, sch.naming, sch.boxing, sch.flagging, sch.sheeting) = (None, None, None, false, None);
        let (placed, dragged) = (sch.placing.take().and_then(|p| p.shown).map(|s| s.1), sch.drag.take());
        if dragged.is_some() {
            sch.status.clear();
        }
        if let Some(undo) = placed.or(dragged.and_then(|d| d.shown).map(|s| s.0)) {
            sch.sheets.project.apply(undo);
            self.redraw_sch();
        }
    }

    /// Undoes the schematic's last edit, or redoes the last one undone (Ctrl+Z, Ctrl+Y). While a
    /// symbol or label is being placed or something dragged, undo takes it back, as Esc does.
    pub(crate) fn undo_sch(&mut self, redo: bool) {
        if !redo && self.sch.as_ref().is_some_and(|s| s.placing.is_some() || s.drag.is_some()) {
            return self.stop_tool();
        }
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        let t = Instant::now();
        sch.picked.clear();
        let Some(what) = sch.sheets.undo(redo) else {
            return sch.status = if redo { tr("Nothing to redo") } else { tr("Nothing to undo") }.into();
        };
        sch.changed = Some(Instant::now());
        let what = if redo { trf("Redone: {0}", &[&what]) } else { trf("Undone: {0}", &[&what]) };
        sch.violations = sch.sheets.project.erc();
        sch.status = trf("{0}, checked in {1}", &[&what, &format!("{:.0?}", t.elapsed())]);
        self.redraw_sch();
    }

    /// Saves the edited sheets (Ctrl+S): over their files, each previous version kept as
    /// `<file>-bak` first.
    pub(crate) fn save_sch(&mut self) {
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        if !sch.sheets.history.unsaved() {
            return sch.status = tr("No edits to save").into();
        }
        sch.status = match sch.sheets.save() {
            Ok(saved) => trf("Saved {0}; each previous version is <file>-bak", &[&saved.join(", ")]),
            Err(e) => trf("Not saved: {0}", &[&e]),
        };
        // Saved from the board's question too, which shows the board's status.
        self.status = sch.status.clone();
        self.seen_files();
    }

    /// Shows what is picked moved by the drag to `at` (sheet nm), on the grid: the move shown
    /// before taken back, then made again from where it was, what joins it following
    /// (`crates/core/schematic-move.md`); where it can't go, the status says why.
    pub(crate) fn drag_to(&mut self, at: Pt) {
        let Some(sch) = &mut self.sch else { return };
        let Some(d) = &mut sch.drag else { return };
        let by = on_grid(Pt { x: at.x - d.from.x, y: at.y - d.from.y });
        if by == d.by {
            return;
        }
        d.by = by;
        if let Some((undo, _)) = d.shown.take() {
            sch.sheets.project.apply(undo);
        }
        if by != Pt::default() {
            match sch.sheets.project.move_items(sch.page, &sch.picked, by) {
                Ok(moved) => d.shown = Some(moved),
                Err(e) => sch.status = e,
            }
            if d.shown.is_some() {
                sch.status = sch.alone().unwrap_or_else(|| trf("{0}: release to put it down, Esc leaves it", &[&sch.what()]));
            }
        }
        self.redraw_sch();
    }

    /// Ends a drag: what it moved stays where it shows, an edit to undo, still picked; the status
    /// says when a label there names nothing.
    pub(crate) fn drop_drag(&mut self) {
        let Some(sch) = &mut self.sch else { return };
        let Some(d) = sch.drag.take() else { return };
        let Some((undo, moved)) = d.shown else {
            // Not moved, or where it can't go (the status says why).
            return if d.by == Pt::default() {
                sch.status.clear()
            };
        };
        let what = trf("{0} moved", &[&sch.what()]);
        sch.picked = moved;
        sch.edited(undo, what);
        if let Some(alone) = sch.alone() {
            sch.status = format!("{}; {alone}", sch.status);
        }
    }

    /// Turns what is picked a quarter turn counterclockwise (R), what joins it following: an edit
    /// to undo, the page drawn again.
    pub(crate) fn turn_picked(&mut self) {
        let Some(sch) = self.sch.as_mut().filter(|s| !s.picked.is_empty() && s.drag.is_none()) else { return };
        let what = trf("{0} turned", &[&sch.what()]);
        match sch.sheets.project.turn_items(sch.page, &sch.picked) {
            Ok((undo, turned)) => {
                sch.picked = turned;
                sch.edited(undo, what);
            }
            Err(e) => return sch.status = e,
        }
        self.redraw_sch();
    }

    /// Inspect with nothing picked on a sheet: its page, as KiCad's Page Settings: the paper,
    /// lying or standing, and the title block's lines typed in place, each an edit to undo.
    pub(crate) fn page_settings(&mut self, ui: &mut egui::Ui) {
        use cuprita_core::schematic::{LINES, PAPERS};
        let Some(sch) = self.sch.as_mut().filter(|_| !self.viewer) else { return };
        let key = (sch.page, sch.changed);
        if sch.setup.as_ref().is_none_or(|s| s.0 != key) {
            sch.setup = sch.sheets.project.page_setup(sch.page).ok().map(|setup| (key, setup));
        }
        let Some((_, setup)) = &sch.setup else { return };
        ui.add_space(12.0);
        crate::dock::heading(ui, tr("Page"));
        let (mut paper, mut portrait, mut line) = (setup.paper.clone(), setup.portrait, None);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("paper").selected_text(&paper).show_ui(ui, |ui| {
                for name in PAPERS {
                    ui.selectable_value(&mut paper, name.to_string(), name);
                }
            });
            ui.checkbox(&mut portrait, tr("Standing")).on_hover_text(tr("Portrait: its short side across"));
        });
        let names = [t!("Title"), t!("Date"), t!("Revision"), t!("Company")];
        for (k, name) in LINES.iter().enumerate() {
            let label = names.get(k).map_or_else(|| trf("Comment {0}", &[&(k - 3)]), |n| tr(n).to_string());
            let value = setup.lines.iter().find(|l| l.0 == *name).map_or("", |l| l.1.as_str());
            if let Some(typed) = crate::inspect::field_row(ui, &label, value, &mut sch.typing) {
                line = Some((*name, label, typed));
            }
        }
        let hint = tr("They fill the title block in the sheet's corner, as the PDF prints it; ${VARIABLES} of the project are filled in.");
        ui.label(egui::RichText::new(hint).small().color(crate::theme::QUIET));
        let project = &mut sch.sheets.project;
        let done = if (&paper, portrait) != (&setup.paper, setup.portrait) {
            Some(project.set_paper(sch.page, &paper, portrait).map(|undo| (undo, trf("Page on {0}", &[&paper]))))
        } else {
            line.map(|(name, label, typed)| project.set_title_line(sch.page, name, &typed).map(|undo| (undo, trf("{0} set", &[&label]))))
        };
        match done {
            Some(Ok((undo, what))) => {
                sch.edited(undo, what);
                self.redraw_sch();
                // Another paper is another size of page: shown whole.
                self.fit = self.sch.as_ref().map(|s| s.scene.bounds);
            }
            Some(Err(e)) => sch.status = e,
            None => {}
        }
    }

    /// Mirrors what is picked left to right (X) or top to bottom (Y), in place: an edit to undo,
    /// the page drawn again.
    pub(crate) fn mirror_picked(&mut self, up_down: bool) {
        let Some(sch) = self.sch.as_mut().filter(|s| !s.picked.is_empty() && s.drag.is_none()) else { return };
        let what = trf("{0} mirrored", &[&sch.what()]);
        match sch.sheets.project.mirror_items(sch.page, &sch.picked, up_down) {
            Ok((undo, mirrored)) => {
                sch.picked = mirrored;
                sch.edited(undo, what);
            }
            Err(e) => return sch.status = e,
        }
        self.redraw_sch();
    }

    /// Copies what is picked (Ctrl+C): kept here for Paste, and on the clipboard, where KiCad
    /// finds it too.
    pub(crate) fn copy_picked(&mut self, ctx: &egui::Context) {
        let Some(sch) = self.sch.as_mut().filter(|s| !s.picked.is_empty()) else { return };
        match sch.sheets.project.copy_items(sch.page, &sch.picked) {
            Ok(clip) => {
                sch.status = trf("{0} copied: Ctrl+V pastes it, here or in KiCad", &[&sch.what()]);
                ctx.copy_text(clip.clone());
                self.clip = Some(clip);
            }
            Err(e) => sch.status = e,
        }
    }

    /// Starts pasting (Ctrl+V): `text` from the clipboard when it is a schematic's clip (ours,
    /// or what KiCad copied), else what was last copied here; it follows the pointer until a
    /// click puts it down.
    pub(crate) fn paste_clip(&mut self, text: Option<String>) {
        let clip = text.filter(|t| t.trim_start().starts_with('(')).or_else(|| self.clip.clone());
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        match clip {
            Some(clip) => {
                sch.picked.clear();
                sch.placing = Some(Placing { item: Placeable::Clip(clip), angle: 0.0, shown: None, pasted: vec![] });
                sch.status = tr("Paste: a click puts it down, Esc drops it").into();
            }
            None => sch.status = tr("Nothing to paste: pick something and copy it (Ctrl+C) first").into(),
        }
    }

    /// Duplicates what is picked (Ctrl+D): a copy follows the pointer until a click puts it
    /// down; the clipboard keeps what it had.
    pub(crate) fn duplicate_picked(&mut self) {
        let Some(sch) = self.sch.as_ref().filter(|s| !s.picked.is_empty()) else { return };
        match sch.sheets.project.copy_items(sch.page, &sch.picked) {
            Ok(clip) => {
                let kept = self.clip.replace(clip);
                self.paste_clip(None);
                self.clip = kept;
            }
            Err(e) => self.sch.iter_mut().for_each(|s| s.status = e.clone()),
        }
    }

    /// Deletes what is picked (Delete): symbols' parts with every unit, wires, labels: an edit to
    /// undo, the page drawn again.
    pub(crate) fn delete_picked(&mut self) {
        let Some(sch) = self.sch.as_mut().filter(|s| !s.picked.is_empty() && s.drag.is_none()) else { return };
        let what = trf("{0} deleted", &[&sch.what()]);
        match sch.sheets.project.delete_items(sch.page, &std::mem::take(&mut sch.picked)) {
            Ok(undo) => sch.edited(undo, what),
            Err(e) => return sch.status = e,
        }
        self.redraw_sch();
    }

    /// Ends a box (Shift+drag on empty paper) at `to`: what it takes is picked too.
    pub(crate) fn pick_box(&mut self, to: Pt) {
        let Some(sch) = &mut self.sch else { return };
        let Some(from) = sch.boxing.take() else { return };
        let taken = sch.sheets.project.pages[sch.page].sheet.items_in(from, to, to.x < from.x);
        let new: Vec<Item> = taken.into_iter().filter(|it| !sch.picked.contains(it)).collect();
        sch.picked.extend(new);
    }

    /// Sets field `name` of the schematic's part `part` to `value` on its every unit: an edit to
    /// undo, the page drawn again.
    pub(crate) fn set_field(&mut self, part: &str, name: &str, value: &str) {
        let Some(sch) = &mut self.sch else { return };
        match sch.sheets.project.set_field(part, name, value) {
            Ok(undo) => sch.edited(undo, trf("{0} {1} set to {2}", &[&part, &name, &value])),
            Err(e) => return sch.status = e,
        }
        self.redraw_sch();
    }

    /// Draws the page shown again after an edit, its inks shown or hidden as they were, the net
    /// lit where it is now.
    pub(crate) fn redraw_sch(&mut self) {
        let Some(sch) = &mut self.sch else { return };
        sch.fields = None;
        if let Some((name, on)) = &mut sch.lit {
            *on = sch.sheets.project.net_items(name);
        }
        let mut scene = crate::sheet::page_scene(&sch.sheets.project, sch.page);
        for (new, old) in scene.layers.iter_mut().zip(&sch.scene.layers) {
            new.visible = old.visible;
        }
        sch.scene = scene;
        self.sync_gpu();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn wires_bend_once_the_longer_way_first() {
        let (a, pt) = (Pt { x: 0, y: 0 }, |x, y| Pt { x, y });
        assert_eq!(wire_path(a, pt(5, -2), false, false), [a, pt(5, 0), pt(5, -2)]);
        assert_eq!(wire_path(a, pt(-2, 5), false, false), [a, pt(0, 5), pt(-2, 5)]);
        // `/` turns the corner the other way; Shift goes straight there, at any angle.
        assert_eq!(wire_path(a, pt(5, -2), true, false), [a, pt(0, -2), pt(5, -2)]);
        assert_eq!(wire_path(a, pt(5, -2), false, true), [a, a, pt(5, -2)]);
    }

    #[test]
    fn autosaves_edited_sheets_and_restores_them() {
        let dir = std::env::temp_dir().join(format!("cuprita-sheets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (root, auto) = (dir.join("b.kicad_sch"), dir.join("_autosave-b.kicad_sch"));
        let src = r#"(kicad_sch (version 20260306) (uuid "r") (lib_symbols))"#;
        std::fs::write(&root, src).unwrap();
        // Written a while ago, so its autosave is newer.
        let ago = SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options().write(true).open(&root).unwrap().set_modified(ago).unwrap();
        let r = r#"(symbol "R" (property "Reference" "R" (at 0 0 0)) (symbol "R_1_1" (pin passive line (at 0 0 0) (length 1) (name "") (number "1"))))"#;
        let mut s = Sheets::read(&root, &[]).unwrap();
        assert_eq!(s.autosaved(), None);
        let (undo, _) = s.project.add_symbol(0, "Device", r, &[], Pt::default(), 0.0).unwrap();
        s.push(undo, "R1 added".into());
        s.autosave().unwrap();
        assert_eq!(
            (std::fs::read_to_string(&auto).unwrap().as_str(), std::fs::read_to_string(&root).unwrap().as_str()),
            (s.project.text("b.kicad_sch").unwrap(), src)
        );
        // Read again, the sheet's autosave is offered; restored, its edit is back, not yet saved.
        let (files, _) = Sheets::read(&root, &[]).unwrap().autosaved().unwrap();
        assert_eq!(files, ["b.kicad_sch"]);
        let mut back = Sheets::read(&root, &files).unwrap();
        assert!(back.history.unsaved() && back.project.pages[0].sheet.symbols.len() == 1);
        // Saved: the file has it, the autosave is gone.
        assert_eq!(back.save(), Ok(vec!["b.kicad_sch".to_string()]));
        assert!(!auto.exists() && !back.history.unsaved() && std::fs::read_to_string(&root).unwrap().contains("(lib_id \"Device:R\")"));
        // Undone back to the file as read, its autosave goes instead of being written.
        s.autosave().unwrap();
        assert!(auto.exists());
        s.undo(false);
        s.autosave().unwrap();
        assert!(!auto.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_sheet_is_autosaved_restored_and_saved() {
        let dir = std::env::temp_dir().join(format!("cuprita-new-sheet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (root, new) = (dir.join("b.kicad_sch"), dir.join("power.kicad_sch"));
        std::fs::write(&root, r#"(kicad_sch (version 20260306) (uuid "r") (lib_symbols))"#).unwrap();
        std::fs::File::options().write(true).open(&root).unwrap().set_modified(SystemTime::now() - Duration::from_secs(60)).unwrap();
        let mut s = Sheets::read(&root, &[]).unwrap();
        let undo = s.project.add_sheet(0, "Power", "power.kicad_sch", Pt::default(), Pt { x: 25_400_000, y: 25_400_000 }, None).unwrap();
        s.push(undo, "Sheet Power added".into());
        // Autosaved, both files; the new one only there.
        s.autosave().unwrap();
        assert!(autosave_path(&new).exists() && !new.exists());
        // Restored with the root's autosave, its sheet comes too, and saving writes it.
        let (files, _) = Sheets::read(&root, &[]).unwrap().autosaved().unwrap();
        let mut back = Sheets::read(&root, &files).unwrap();
        assert_eq!(back.project.pages.len(), 2);
        assert_eq!(back.save(), Ok(vec!["b.kicad_sch".to_string(), "power.kicad_sch".to_string()]));
        assert!(std::fs::read_to_string(&new).unwrap().starts_with("(kicad_sch\n\t(version 20260306)"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
