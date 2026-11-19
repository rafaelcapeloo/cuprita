//! The schematic view (Ctrl+E, KiCad's key for the schematic, switches it with the board): the
//! project's sheets read from its root down (`crates/core/schematic.md`). Drag pans, the wheel
//! zooms, a double click opens a sheet, Backspace or the path goes back up; a click picks a
//! symbol, wire or label (Shift+click or a Shift+drag box several), Find a part, which Ctrl+E shows
//! selected on the board, as it shows the board's here (cross-probing); D lists the electrical rule
//! violations (`crates/core/erc.md`), a click shows one.
//! F8 updates the board from the schematic (`crates/core/update.md`); S simulates it (`sim.rs`).

use crate::canvas::{Paint, zoom_at};
use crate::edit::footprint_text;
use crate::file::{Ask, autosave_path};
use crate::sheet_edit::{Drag, LABEL_FIELD, Naming, NewSheet, Placing, Sheets};
use crate::shell::Bench;
use crate::theme::{self, FAIL as ERROR, WARN as WARNING};
use crate::tr::{n, tr, trf};
use crate::{App, Found, around, count, find_target, um_box};
use cuprita_core::board::{Nm, Pt};
use cuprita_core::history::Pick;
use cuprita_core::schematic::{Ink, Item, LabelKind, NetOnPage, Page, Project, SHAPES, Symbol, Violation};
use cuprita_core::update::Updated;
use cuprita_render::{Scene, View, build_sch};
use eframe::egui::{self, RichText};
use eframe::egui_wgpu;
use std::path::{Path, PathBuf};
use web_time::Instant;

/// A project's schematic (its sheets, as edited) and the page shown.
pub(crate) struct Sch {
    pub(crate) sheets: Sheets,
    pub(crate) page: usize,
    pub(crate) scene: Scene,
    /// Scene boxes around the ERC violation clicked, marked on the page shown.
    pub(crate) found: Vec<[f32; 4]>,
    /// What is picked on the page shown (a click, Shift+click, a box, or Find: its part's
    /// symbols), marked there; a drag moving it, and a box being drawn (Shift+drag) from a corner.
    pub(crate) picked: Vec<Item>,
    pub(crate) drag: Option<Drag>,
    pub(crate) boxing: Option<Pt>,
    /// The net lit (a click on a wire or label, or the board's, Ctrl+E): its name, and where it
    /// is on each page it reaches.
    pub(crate) lit: Option<(String, Vec<NetOnPage>)>,
    /// The board's centre and zoom while the schematic shows, and back.
    pub(crate) other: Option<([f32; 2], f32)>,
    /// What the schematic's toolbar says: what was read, what went wrong.
    pub(crate) status: String,
    /// The project's electrical rule violations, errors first.
    pub(crate) violations: Vec<Violation>,
    /// The symbol or label being placed (A, L); when the sheets last changed, until autosaved.
    pub(crate) placing: Option<Placing>,
    pub(crate) changed: Option<Instant>,
    /// The wire tool (W): on, with where the wire being drawn goes on from once one is started.
    pub(crate) wire: Option<Option<Pt>>,
    /// The wire tool draws buses (B).
    pub(crate) bus: bool,
    /// The wire's corner turned the other way (`/`): the shorter leg first.
    pub(crate) swap_bend: bool,
    /// The label tool (L, Ctrl+L global, H hierarchical): on, with the label until Enter starts
    /// placing it: the name typed, its kind and shape.
    pub(crate) naming: Option<Naming>,
    /// The no-connect tool (Q) is on.
    pub(crate) flagging: bool,
    /// The sheet tool (Shift+S): the box being drawn, then its name and file.
    pub(crate) sheeting: Option<NewSheet>,
    /// The selected part's fields, read when first shown after an edit (`redraw_sch` forgets
    /// them), with the field being typed in and the footprint chooser's words while it is open.
    pub(crate) fields: Option<(String, cuprita_core::schematic::PartFields)>,
    pub(crate) typing: Option<(String, String)>,
    /// The page shown's settings as its file has them, read once an edit or another page (Inspect's Page).
    pub(crate) setup: Option<((usize, Option<Instant>), cuprita_core::schematic::PageSetup)>,
    pub(crate) choosing: Option<String>,
    /// Its simulation (S): running, or what it gave.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) sim: Option<crate::sim::Simulation>,
}

/// The root sheet of the project a `.kicad_sch` file is part of: the one named after its
/// folder's `.kicad_pro`, else the file itself.
pub(crate) fn root_of(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let pro =
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "kicad_pro"));
    pro.map(|p| p.with_extension("kicad_sch")).filter(|r| r.exists()).unwrap_or_else(|| path.to_path_buf())
}

/// Page `i` of `project` as the window shows it: KiCad's colours on its light paper, or
/// (`theme::dark_sheet`) the same inks made for a dark page, which the eye rests on beside the
/// board's canvas. A PDF is always KiCad's (`Ink::color`).
pub(crate) fn page_scene(project: &Project, i: usize) -> Scene {
    let mut scene = build_sch(project, i);
    if theme::dark_sheet() {
        let dark = |ink: Ink| match ink {
            Ink::Frame => 0x566170,
            Ink::BodyFill => 0x232830,
            Ink::Notes => 0x7FB2FF,
            Ink::Sheet => 0xD28C6E,
            Ink::Body => 0xE2857A,
            Ink::Wire => 0x55C97E,
            Ink::Bus | Ink::NoConnect => 0x7C97FF,
            Ink::Label => 0xE8EBEF,
            Ink::GlobalLabel => 0xF09A86,
            Ink::HierLabel | Ink::SheetFile => 0xD9B65C,
            Ink::SheetName | Ink::SheetPin | Ink::PinName | Ink::Reference | Ink::Value => 0x56CBCB,
            Ink::PinNumber => 0xEE8A8A,
            Ink::Field => 0xCF94E2,
        };
        // The page first, then a slot an ink (`build_sch`).
        let colors = std::iter::once(0x151A20).chain(Ink::ALL.map(dark));
        for (layer, c) in scene.layers.iter_mut().zip(colors) {
            layer.color = [(c >> 16 & 255) as f32 / 255.0, (c >> 8 & 255) as f32 / 255.0, (c & 255) as f32 / 255.0, 1.0];
        }
    }
    scene
}

/// Reads the schematic of the project whose root sheet is `root`, with its `.kicad_pro`; its
/// sheet files `autosaved` from their autosaves, and then those made since the last save.
pub(crate) fn read_project(root: &Path, autosaved: &[String]) -> Result<Project, String> {
    let (dir, name) = (root.parent().unwrap_or(Path::new(".")), root.file_name().unwrap_or_default().to_string_lossy());
    let (read_text, there) = (crate::file::read_text, |p: &Path| crate::file::written(p).is_some());
    let pro = read_text(&root.with_extension("kicad_pro")).unwrap_or_default();
    let read = |f: &str| {
        let (file, auto) = (dir.join(f), autosave_path(&dir.join(f)));
        let made = !autosaved.is_empty() && !there(&file) && there(&auto);
        let path = if made || autosaved.iter().any(|a| a == f) { auto } else { file };
        read_text(&path)
            .map_err(|e| trf("{0} not read from {1}: {2}", &[&path.file_name().unwrap_or_default().display(), &dir.display(), &e]))
    };
    Project::load(&name, &pro, read)
}

impl App {
    /// Ctrl+E: the project's schematic in place of the board, or the board again. Each is read
    /// the first time: the schematic from the root sheet beside the board, the board beside it.
    pub(crate) fn toggle_sch(&mut self) {
        self.stop_tool();
        self.arrive(false);
        if self.on_sheet {
            if self.open_board(App::probe) {
                return;
            }
        } else if !self.board_sch() {
            return;
        }
        self.on_sheet = !self.on_sheet;
        self.bench = if self.on_sheet { Bench::Schematic } else { Bench::Board };
        // Each view keeps its own centre and zoom; the schematic's first shows its whole page.
        if let Some(sch) = &mut self.sch {
            let here = (self.center, self.um_per_pt);
            match sch.other.replace(here) {
                Some((center, zoom)) => (self.center, self.um_per_pt) = (center, zoom),
                None => self.fit = Some(sch.scene.bounds),
            }
        }
        self.sync_gpu();
        self.probe();
    }

    /// From the schematic of a board that isn't open: opens it in place of the schematic (first
    /// asking about the open board's unsaved edits), then does `then`; `false` when it is open
    /// already.
    fn open_board(&mut self, then: fn(&mut App)) -> bool {
        let Some(board) = self.sch.as_ref().map(|s| s.sheets.root.with_extension("kicad_pcb")) else { return true };
        if self.path.as_deref() == Some(&board) {
            return false;
        }
        if crate::file::written(&board).is_none() {
            let why = trf("No board beside the schematic: {0} isn't there", &[&board.display()]);
            self.sch.iter_mut().for_each(|s| s.status = why.clone());
            return true;
        }
        let here = (self.center, self.um_per_pt);
        let go = move |app: &mut App| {
            // The board opens fitted; Ctrl+E brings the schematic back as it was.
            app.sch.iter_mut().for_each(|s| s.other = Some(here));
            (app.bench, app.on_sheet) = (Bench::Board, false);
            app.open(&board);
            // Not while asking whether to restore its autosave: once it opens, the key again.
            if app.path.as_deref() == Some(board.as_path()) {
                then(app);
            }
        };
        // The schematic's edits stay, as it is this board's: only the open board's would go.
        if self.history.unsaved() {
            self.leave(go)
        } else {
            go(self)
        }
        true
    }

    /// Reads the schematic beside the open board, unless it is read; `false` when it can't (yet),
    /// the status saying why.
    pub(crate) fn board_sch(&mut self) -> bool {
        if self.sch.as_ref().is_some_and(|s| Some(s.sheets.root.with_extension("kicad_pcb")) == self.path) {
            return true;
        }
        let Some(root) = self.path.as_ref().map(|p| p.with_extension("kicad_sch")) else {
            self.status = tr("Open a board or a .kicad_sch first").into();
            return false;
        };
        if crate::file::written(&root).is_none() {
            self.status = trf("No schematic beside the board: {0} isn't there", &[&root.display()]);
            return false;
        }
        // Read, the board keeps its view.
        let fit = self.fit;
        let read = self.load_sch(&root, &root, None);
        self.fit = fit;
        read
    }

    /// Reads the schematic beside the board once the board has shown a second and no button is
    /// held, for its checks to count before Make is opened (the header's pill): once a board,
    /// saying nothing when there is none, and not one with autosaved sheets, whose question
    /// waits for the schematic to be shown.
    pub(crate) fn early_sch(&mut self, ctx: &egui::Context) {
        if self.sch.is_some() || self.path.is_none() || self.sought == self.path {
            return;
        }
        let wait = std::time::Duration::from_secs(1).saturating_sub(self.arrived.elapsed());
        if !wait.is_zero() {
            return ctx.request_repaint_after(wait);
        }
        if self.ask.is_some() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        self.sought = self.path.clone();
        let there = self.path.as_ref().is_some_and(|p| crate::file::written(&p.with_extension("kicad_sch")).is_some());
        if there && !self.board_sch() && matches!(self.ask, Some(crate::file::Ask::RestoreSheets { .. })) {
            self.ask = None;
        }
    }

    /// F8, KiCad's Update PCB from Schematic: the board takes the parts it lacks and the nets of
    /// its project's schematic, edits not saved yet too (`crates/core/update.md`), and shows,
    /// the parts added in view.
    pub(crate) fn update_board(&mut self) {
        self.stop_tool();
        if self.sch_view() {
            if self.open_board(App::update_board) {
                return;
            }
            self.toggle_sch();
        } else if !self.board_sch() {
            return;
        }
        let folder = self.folder();
        let (t, Some(sch)) = (Instant::now(), &self.sch) else { return };
        let parts = self.parts.get_or_insert_with(|| crate::parts::footprints(folder.as_deref()));
        let (undo, done) = self.board.update(&sch.sheets.project, |id| footprint_text(parts, id));
        let missing = match &done.missing[..] {
            [] => String::new(),
            [why] => format!(" — {why}"),
            [why, more @ ..] => format!(" — {why}; {}", count(more.len(), n!("more part left as they were|more parts left as they were"))),
        };
        let Some(undo) = undo else {
            return self.status = tr("Updated from the schematic: nothing changed").to_string() + &missing;
        };
        self.edited(undo, t, updated(&done));
        self.status += &missing;
        // The board and, below it, the parts added.
        if let (true, Some(scene)) = (done.parts > 0, &self.scene) {
            self.fit = Some(crate::whole(&self.board, scene));
        }
    }

    /// Cross-probing: the view shown selects and shows the part the other view selects, and
    /// lights the net it lights (showing it when no part is shown), unless it already does; the
    /// status says when it has no such part or net.
    fn probe(&mut self) {
        let Some(sch) = &self.sch else { return };
        // Not while the schematic's own board is still to open.
        if self.path != Some(sch.sheets.root.with_extension("kicad_pcb")) {
            return;
        }
        let on_board = match self.selected {
            Some(Pick::Footprint(i)) => Some(self.board.footprints[i].reference.clone()),
            _ => None,
        };
        let footprinted =
            |r: &str| sch.sheets.project.pages.iter().flat_map(|p| &p.sheet.symbols).any(|s| s.designator == r && s.on_board());
        let shown = match (self.sch_view(), on_board, sch.part().map(String::from)) {
            (_, a, b) if a == b => false,
            (true, Some(r), _) => {
                let found = self.find_sch(&r);
                if !found {
                    self.sch.iter_mut().for_each(|s| s.status = trf("{0} isn't in the schematic", &[&r]));
                }
                found
            }
            // Power symbols and parts kept off the board (a shield's cover) have nothing there.
            (false, _, Some(r)) if footprinted(&r) => {
                let Some(scene) = &self.scene else { return };
                match find_target(&self.board, scene, &r) {
                    Some(Found::Footprint { index, view, .. }) => (self.selected, self.fit) = (Some(Pick::Footprint(index)), Some(view)),
                    _ => self.status = trf("{0} isn't on the board: Update from schematic (F8) adds it", &[&r]),
                }
                true
            }
            _ => false,
        };
        // Nets by name: the board's come from the schematic's (F8).
        let Some(sch) = &self.sch else { return };
        let board = self.board.nets.get(self.net as usize).filter(|_| self.net != 0).cloned();
        let sheets = sch.lit.as_ref().map(|l| l.0.clone());
        match (self.sch_view(), board, sheets) {
            (_, a, b) if a == b => {}
            (true, Some(name), _) => self.light_net(Some(name), !shown),
            (false, _, Some(name)) => match self.board.nets.iter().position(|n| *n == name) {
                Some(net) => {
                    self.net = net as u32;
                    if !shown {
                        let Some(scene) = &self.scene else { return };
                        (self.selected, self.fit) = (None, self.board.net_bounds(self.net).map(|b| around(um_box(scene, b), 1.0)));
                    }
                }
                None => self.status = trf("{0} isn't on the board: Update from schematic (F8) adds it", &[&name]),
            },
            _ => {}
        }
    }

    /// Lights net `name` on the sheets (`None`: none), the status saying which sheets it reaches;
    /// with `show`, shows it: on the page shown, else the first it reaches, zoomed to it.
    pub(crate) fn light_net(&mut self, name: Option<String>, show: bool) {
        let Some(sch) = &mut self.sch else { return };
        let Some(name) = name else { return sch.lit = None };
        let (on, pages) = (sch.sheets.project.net_items(&name), &sch.sheets.project.pages);
        let names: Vec<String> = on.iter().map(|o| page_name(&pages[o.page])).collect();
        sch.status = match names.len() {
            0 => trf("{0} isn't in the schematic", &[&name]),
            n => trf("Net {0}, on {1}: {2}", &[&name, &count(n, n!("sheet")), &names.join(", ")]),
        };
        let page = on.iter().find(|o| o.page == sch.page).or(on.first()).map(|o| o.page);
        sch.lit = Some((name, on));
        let (true, Some(page)) = (show, page) else { return };
        if page != sch.page {
            self.show_page(page);
        }
        let Some(sch) = &self.sch else { return };
        let Some(here) = sch.lit.iter().flat_map(|l| &l.1).find(|o| o.page == page) else { return };
        let sheet = &sch.sheets.project.pages[page].sheet;
        let corners = here.items.iter().flat_map(|&it| <[Pt; 2]>::from(sheet.extent(it))).chain(here.pins.iter().copied());
        let grow = |(lo, hi): (Pt, Pt), p: Pt| (Pt { x: lo.x.min(p.x), y: lo.y.min(p.y) }, Pt { x: hi.x.max(p.x), y: hi.y.max(p.y) });
        if let Some(first) = corners.clone().next() {
            self.fit = Some(around(um_box(&sch.scene, corners.fold((first, first), grow)), 1.2));
        }
    }

    /// Opens a `.kicad_sch` file: its project's schematic, read from the root sheet (named after
    /// the folder's `.kicad_pro`, else the file itself), at the file's first page.
    pub(crate) fn open_sch(&mut self, path: &Path) {
        self.show_sch(&root_of(path), path, None);
    }

    /// Reads the schematic of root sheet `root` (as `load_sch` does) and shows the first page of
    /// file `at` in place of the board.
    pub(crate) fn show_sch(&mut self, root: &Path, at: &Path, restored: Option<&[String]>) {
        // The board's view, which Ctrl+E goes back to.
        let board = if self.on_sheet { self.sch.as_ref().and_then(|s| s.other) } else { Some((self.center, self.um_per_pt)) };
        if self.load_sch(root, at, restored) {
            self.sch.iter_mut().for_each(|s| s.other = board);
            (self.bench, self.on_sheet) = (Bench::Schematic, true);
            self.sync_gpu();
        }
    }

    /// Reads the schematic of root sheet `root`, the sheets `restored` from their autosaves
    /// (`None`: first asking whether to, when some are newer than their files), at the first
    /// page of file `at`; `false` when it doesn't (yet), the status saying why.
    pub(crate) fn load_sch(&mut self, root: &Path, at: &Path, restored: Option<&[String]>) -> bool {
        let t = Instant::now();
        let sheets = match Sheets::read(root, restored.unwrap_or_default()) {
            Ok(s) => s,
            Err(e) => {
                self.status = e;
                return false;
            }
        };
        if let (None, Some((files, age))) = (restored, sheets.autosaved()) {
            self.ask = Some(Ask::RestoreSheets { root: root.to_path_buf(), at: at.to_path_buf(), files, age });
            return false;
        }
        let (read, project) = (t.elapsed(), &sheets.project);
        let violations = project.erc();
        let errors = violations.iter().filter(|v| v.error).count();
        let file = at.file_name().unwrap_or_default().to_string_lossy();
        let page = project.pages.iter().position(|p| p.file == file).unwrap_or(0);
        let name = root.file_name().unwrap_or_default().to_string_lossy();
        let (pages, checked) = (count(project.pages.len(), n!("sheet")), format!("{:.0?}", t.elapsed() - read));
        let mut status = trf("{0} — {1} read in {2}, checked in {3}", &[&name, &pages, &format!("{read:.0?}"), &checked]);
        status += &format!(" — ERC: {}, {}", count(errors, n!("error")), count(violations.len() - errors, n!("warning")));
        if let Some(e) = project.errors.first() {
            status += &format!(" — {}: {e}", count(project.errors.len(), n!("not read|not read")));
        }
        let scene = page_scene(project, page);
        self.fit = Some(scene.bounds);
        // Another project's own parts are read again when the Parts tab wants them.
        (self.parts, self.symbols) = (None, None);
        self.sch = Some(Sch {
            sheets,
            page,
            scene,
            found: Vec::new(),
            picked: Vec::new(),
            drag: None,
            boxing: None,
            lit: None,
            other: None,
            status,
            violations,
            placing: None,
            changed: None,
            wire: None,
            bus: false,
            swap_bend: false,
            naming: None,
            flagging: false,
            sheeting: None,
            fields: None,
            typing: None,
            setup: None,
            choosing: None,
            #[cfg(not(target_arch = "wasm32"))]
            sim: None,
        });
        true
    }

    /// Shows page `i` of the schematic, whole.
    pub(crate) fn show_page(&mut self, i: usize) {
        self.stop_tool();
        let Some(sch) = &mut self.sch else { return };
        (sch.page, sch.scene) = (i, page_scene(&sch.sheets.project, i));
        (self.fit, sch.found, sch.picked) = (Some(sch.scene.bounds), Vec::new(), Vec::new());
        self.sync_gpu();
    }

    /// The label tool's field: the net's name, Enter starts placing it; whether a net label is
    /// global, and a global or hierarchical one's shape.
    pub(crate) fn label_field(&mut self, ui: &mut egui::Ui) {
        let Some((name, kind, shape)) = self.sch.as_mut().and_then(|s| s.naming.as_mut()) else { return };
        let field =
            egui::TextEdit::singleline(name).id(egui::Id::unique(LABEL_FIELD)).hint_text(tr("Net name, then Enter")).desired_width(150.0);
        let field = ui.add(field);
        // The hint's first showing only sizes it, which drops the focus the tool asked for: ask
        // again once it shows.
        let refocus = egui::Id::unique((LABEL_FIELD, "again"));
        if ui.is_sizing_pass() {
            ui.data_mut(|d| d.insert_temp(refocus, true));
        } else if ui.data_mut(|d| d.remove_temp::<bool>(refocus)).is_some() {
            field.request_focus();
        }
        let mut changed = false;
        if *kind != LabelKind::Hier {
            let mut global = *kind == LabelKind::Global;
            let tick = ui
                .checkbox(&mut global, tr("Global"))
                .on_hover_text(tr("Joins the labels of its name on every sheet, not only on this one"));
            if tick.changed() {
                (*kind, changed) = (if global { LabelKind::Global } else { LabelKind::Local }, true);
            }
        }
        if *kind != LabelKind::Local {
            let named = |s: &str| match s {
                "input" => tr("Input"),
                "output" => tr("Output"),
                "bidirectional" => tr("Bidirectional"),
                "tri_state" => tr("Tri-state"),
                _ => tr("Passive"),
            };
            let combo = egui::ComboBox::from_id_salt("label shape")
                .selected_text(named(shape))
                .show_ui(ui, |ui| SHAPES.iter().any(|&s| ui.selectable_value(shape, s, named(s)).clicked()));
            changed |= combo.inner == Some(true);
            combo.response.on_hover_text(tr("Its shape: which way its signal goes (KiCad's)"));
        }
        // Back to the name, so Enter places it.
        if changed {
            field.request_focus();
        }
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let label = (name.trim().to_string(), *kind, *shape);
            if label.0.is_empty() { field.request_focus() } else { self.pick_label(label) }
        }
    }

    /// The path of the page shown, from the root sheet, floating at the top left of the canvas
    /// `rect`: a click goes to a page, Backspace up one.
    pub(crate) fn sheet_path(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let Some(sch) = &self.sch else { return };
        let mut chain = vec![sch.page];
        while let Some(up) = sch.sheets.project.pages[*chain.last().unwrap()].parent {
            chain.push(up);
        }
        let mut go = (!ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Backspace)))
            .then_some(sch.sheets.project.pages[sch.page].parent)
            .flatten();
        let area = egui::Area::new(egui::Id::unique("sheet path")).order(egui::Order::Foreground).fade_in(false);
        area.fixed_pos(rect.left_top() + egui::vec2(12.0, 12.0)).show(ctx, |ui| {
            let frame = egui::Frame::new()
                .fill(theme::PANEL)
                .stroke((1.0, theme::LINE))
                .corner_radius(9)
                .inner_margin(egui::Margin::symmetric(6, 3));
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (n, &k) in chain.iter().rev().enumerate() {
                        if n > 0 {
                            ui.label(RichText::new(crate::icon::RIGHT).color(theme::QUIET).size(10.0));
                        }
                        let page = &sch.sheets.project.pages[k];
                        let color = if k == sch.page { theme::TEXT } else { theme::TEXT_2 };
                        let name = RichText::new(page_name(page)).color(color);
                        if ui.add(egui::Button::selectable(false, name)).on_hover_text(&page.file).clicked() {
                            go = Some(k);
                        }
                    }
                });
            });
        });
        if let Some(k) = go {
            self.show_page(k);
        }
    }

    /// The electrical rule violations (`D`), by page, errors in red: hover shows the fix, a click
    /// shows its page and marks the place.
    pub(crate) fn erc_tab(&mut self, ui: &mut egui::Ui) {
        let Some(sch) = &self.sch else { return };
        let mut pick = None;
        if sch.violations.is_empty() {
            ui.label(RichText::new(format!("{}  {}", crate::icon::CHECKS, tr("Every electrical rule passes."))).color(theme::PASS));
        }
        let rows: Vec<crate::panels::Check> = (sch.violations.iter())
            .map(|v| {
                let sheet = sch.sheets.project.pages[v.page].path.join("/");
                let text = if sheet.is_empty() { v.what.clone() } else { trf("{0}: {1}", &[&sheet, &v.what]) };
                (v.kind_name(), text, &v.fix[..], if v.error { ERROR } else { WARNING })
            })
            .collect();
        pick = pick.or(crate::panels::check_rows(ui, &rows));
        if let Some(k) = pick {
            let (page, at) = (sch.violations[k].page, sch.violations[k].at);
            self.show_page(page);
            let Some(sch) = &mut self.sch else { return };
            let [x, y] = sch.scene.um(at);
            let b = [x - 1500.0, y - 1500.0, x + 1500.0, y + 1500.0];
            (sch.found, self.fit) = (vec![b], Some(around(b, 8.0)));
        }
    }

    /// Selects the part whose reference is `q` (`U3`, or a unit of it: `U3A`) and shows the page
    /// of its first symbol, zoomed to it; `false` when no page has it.
    pub(crate) fn find_sch(&mut self, q: &str) -> bool {
        let Some(sch) = &self.sch else { return false };
        let is = |s: &&Symbol| !q.is_empty() && (s.designator.eq_ignore_ascii_case(q) || s.reference.eq_ignore_ascii_case(q));
        let hit = sch.sheets.project.pages.iter().enumerate().find_map(|(i, p)| Some((i, p.sheet.symbols.iter().find(is)?)));
        let Some((page, s)) = hit else { return false };
        let (part, bounds) = (s.designator.clone(), s.bounds);
        self.show_page(page);
        let Some(sch) = &mut self.sch else { return false };
        let symbols = &sch.sheets.project.pages[page].sheet.symbols;
        sch.picked = (0..symbols.len()).filter(|&i| symbols[i].designator == part).map(Item::Symbol).collect();
        self.fit = Some(around(um_box(&sch.scene, bounds), 3.0));
        true
    }

    /// The schematic's canvas: drag pans, the wheel zooms, a click picks the symbol, wire or label
    /// under the pointer (Shift+click adds or takes it away; Shift+drag on empty paper draws a box,
    /// which takes what it holds drawn left to right, what it touches right to left), a drag
    /// starting on a picked item moves what is picked (shown where it goes), a double click opens
    /// the sheet there, Esc clears the picks and the marks; a symbol or label being placed follows
    /// the pointer until a click puts it down (Esc stops); W starts a wire at the pointer, each
    /// click adds one, Enter or Esc ends it (Esc again stops the tool).
    pub(crate) fn canvas_sch(&mut self, ui: &mut egui::Ui) {
        let checks = self.right.shows(crate::dock::Tab::Checks);
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        self.settle_view(ui.ctx(), rect, resp.dragged() || crate::canvas::wheel_notches(ui) != 0.0 || ui.input(|i| i.any_touches()));
        let Some(sch) = &self.sch else { return };
        let to_um = |pos: egui::Pos2, center: [f32; 2], scale: f32| {
            let off = pos - rect.center();
            [center[0] + off.x * scale, center[1] + off.y * scale]
        };
        let (mut open, placing, wire, flagging, tol) =
            (None, sch.placing.is_some(), sch.wire, sch.flagging, (3000.0 * self.um_per_pt) as Nm);
        let (sheeting, framing) = (sch.sheeting.is_some(), sch.sheeting.as_ref().is_some_and(|s| !s.drawn));
        let (at, shift) = (|pos| sch.scene.nm(to_um(pos, self.center, self.um_per_pt)), ui.input(|i| i.modifiers.shift));
        // A drag that starts on a picked item moves what is picked; one with Shift on empty paper
        // draws a box; with the sheet tool, the new sheet's box; other drags pan.
        let pressed = resp.drag_started_by(egui::PointerButton::Primary).then(|| ui.input(|i| i.pointer.press_origin())).flatten();
        let start = pressed
            .filter(|_| !placing && wire.is_none() && !sheeting)
            .map(|pos| (at(pos), sch.sheets.project.pages[sch.page].sheet.item_at(at(pos), tol)));
        let grab = start.filter(|(_, hit)| !self.viewer && hit.is_some_and(|it| sch.picked.contains(&it))).map(|s| s.0);
        let boxing = start.filter(|(_, hit)| shift && hit.is_none()).map(|s| s.0);
        let (frame, ended) =
            (pressed.filter(|_| framing).map(at), resp.drag_stopped().then(|| ui.input(|i| i.pointer.latest_pos())).flatten().map(at));
        if grab.is_none()
            && boxing.is_none()
            && sch.drag.is_none()
            && sch.boxing.is_none()
            && !framing
            && ui.input(|i| i.multi_touch().is_none())
        {
            let d = resp.drag_delta();
            self.center = [self.center[0] - d.x * self.um_per_pt, self.center[1] - d.y * self.um_per_pt];
        }
        // Under the pointer, or between two fingers on the canvas.
        let touched = ui.input(|i| i.multi_touch()).map(|t| t.center_pos).filter(|p| rect.contains(*p));
        if let Some(pos) = resp.hover_pos().or(touched) {
            zoom_at(ui, pos, (&mut self.center, &mut self.um_per_pt), to_um);
        }
        let Some(sch) = &self.sch else { return };
        let pointer = resp.hover_pos().map(|pos| sch.scene.nm(to_um(pos, self.center, self.um_per_pt)));
        self.pointer = pointer;
        if let (true, false, None, Some(pos)) = (resp.double_clicked() && !sheeting, placing, wire, resp.interact_pointer_pos()) {
            let at = sch.scene.nm(to_um(pos, self.center, self.um_per_pt));
            let page = &sch.sheets.project.pages[sch.page];
            let inside = |(lo, hi): (cuprita_core::board::Pt, cuprita_core::board::Pt)| {
                (lo.x..=hi.x).contains(&at.x) && (lo.y..=hi.y).contains(&at.y)
            };
            if let Some(k) = page.sheet.sheets.iter().position(|s| inside(s.bounds)) {
                open = Some(page.children[k].ok_or_else(|| {
                    let s = &page.sheet.sheets[k];
                    sch.sheets.project.errors.iter().find(|e| e.contains(&format!("\"{}\"", s.name))).cloned().unwrap_or_default()
                }));
            }
        }
        let typing = ui.ctx().egui_wants_keyboard_input();
        let key = |k| !typing && ui.input(|i| i.key_pressed(k));
        // Ctrl+C, Ctrl+X and Ctrl+V come as the clipboard's own events (in the browser, the page's).
        if !typing && !self.viewer {
            let clipboard =
                ui.input(|i| i.events.iter().find(|e| matches!(e, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))).cloned());
            match clipboard {
                Some(egui::Event::Copy) => self.copy_picked(ui.ctx()),
                Some(egui::Event::Cut) => {
                    self.copy_picked(ui.ctx());
                    self.delete_picked();
                }
                Some(egui::Event::Paste(text)) => self.paste_clip(Some(text)),
                _ => {}
            }
        }
        let (drawing, double) = (matches!(wire, Some(Some(_))), resp.double_clicked());
        if drawing && (double || key(egui::Key::Escape) || key(egui::Key::Enter)) {
            self.sch.iter_mut().for_each(|s| s.wire = Some(None));
        } else if key(egui::Key::Escape) {
            self.sch.iter_mut().for_each(|s| (s.found, s.picked, s.lit) = (Vec::new(), Vec::new(), None));
            self.stop_tool();
        } else if let (true, Some(at)) = (placing, pointer) {
            self.show_placing(at, false);
            if resp.clicked() {
                self.put_down();
            }
        } else if let (Some(_), true, false, Some(at)) = (wire, resp.clicked(), double, pointer) {
            self.wire_click(at, shift);
        } else if let (true, true, Some(at)) = (flagging, resp.clicked(), pointer) {
            self.flag_pin(at);
        } else if let (None, true, Some(at), Some(sch)) = (wire, resp.clicked() && !sheeting, pointer, &mut self.sch) {
            let hit = sch.sheets.project.pages[sch.page].sheet.item_at(at, tol);
            match (hit, sch.picked.iter().position(|&p| Some(p) == hit)) {
                (_, Some(k)) if shift => drop(sch.picked.remove(k)),
                (Some(it), None) if shift => sch.picked.push(it),
                _ if shift => {}
                // A wire or label lights its net; a symbol or the paper none.
                _ => {
                    sch.picked = hit.into_iter().collect();
                    let net = hit.and_then(|it| sch.sheets.project.net_of(sch.page, it));
                    self.light_net(net, false);
                }
            }
        }
        if let Some(sch) = &mut self.sch {
            sch.drag = grab.map(Drag::from).or(sch.drag.take());
            sch.boxing = boxing.or(sch.boxing);
        }
        if let Some(at) = pointer.filter(|_| self.sch.as_ref().is_some_and(|s| s.drag.is_some())) {
            self.drag_to(at);
        }
        if let Some(to) = ended {
            self.drop_drag();
            self.pick_box(to);
        }
        if framing {
            self.drag_sheet(frame, pointer, ended.is_some());
        }
        let Some(sch) = &self.sch else { return };
        let visible: Vec<bool> = sch.scene.layers.iter().map(|l| l.visible).collect();
        let ppp = ui.ctx().pixels_per_point();
        let view = View::new(self.center, self.um_per_pt / ppp, [rect.width() * ppp, rect.height() * ppp]);
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, Paint(view, visible)));
        let screen = |[x, y]: [f32; 2]| rect.center() + egui::vec2(x - self.center[0], y - self.center[1]) / self.um_per_pt;
        let (page, p) = (&sch.sheets.project.pages[sch.page], ui.painter());
        let (glow, width) = (theme::SIGNAL.gamma_multiply(0.4), 152.4 / self.um_per_pt + 6.0);
        // The net lit, where it shows: a glow along its wires, over its labels and power symbols,
        // on its pins.
        if let Some(here) = sch.lit.iter().flat_map(|l| &l.1).find(|o| o.page == sch.page) {
            for &it in &here.items {
                let (a, b) = page.sheet.extent(it);
                let (a, b) = (screen(sch.scene.um(a)), screen(sch.scene.um(b)));
                match it {
                    Item::Wire(_) => p.line_segment([a, b], (width, glow)),
                    _ => p.rect_filled(egui::Rect::from_two_pos(a, b).expand(2.0), 2.0, glow),
                };
            }
            for &q in &here.pins {
                p.circle_filled(screen(sch.scene.um(q)), 4.0, glow);
            }
        }
        // What is picked, where it shows: a glow along a wire, a box round anything else.
        for &it in sch.marked() {
            let (a, b) = page.sheet.extent(it);
            match it {
                Item::Wire(_) => {
                    p.line_segment([screen(sch.scene.um(a)), screen(sch.scene.um(b))], (width, glow));
                }
                _ => {
                    let [x0, y0, x1, y1] = um_box(&sch.scene, (a, b));
                    let marker = egui::Rect::from_two_pos(screen([x0, y0]), screen([x1, y1])).expand(3.0);
                    p.rect_stroke(marker, 2.0, (2.0, theme::SIGNAL), egui::StrokeKind::Outside);
                }
            }
        }
        let ping = crate::motion::eased(ui.ctx(), self.pinged.1, crate::motion::ARRIVE);
        for [x0, y0, x1, y1] in sch.found.iter().copied() {
            let marker = egui::Rect::from_two_pos(screen([x0, y0]), screen([x1, y1])).expand(3.0);
            p.rect_stroke(marker, 2.0, (2.0, theme::SIGNAL), egui::StrokeKind::Outside);
            crate::helpers::ping(p, marker, ping);
        }
        // A new sheet's box.
        if let Some((a, b)) = sch.sheeting.as_ref().and_then(|s| s.corners) {
            let r = egui::Rect::from_two_pos(screen(sch.scene.um(a)), screen(sch.scene.um(b)));
            p.rect_stroke(r, 0.0, (1.5, theme::SIGNAL), egui::StrokeKind::Middle);
        }
        // The box being drawn: solid when it takes what it holds, dashed what it touches.
        if let (Some(from), Some(to)) = (sch.boxing, pointer) {
            crate::helpers::pick_box(p, screen(sch.scene.um(from)), screen(sch.scene.um(to)));
        }
        for v in sch.violations.iter().filter(|v| checks && v.page == sch.page) {
            p.circle_stroke(screen(sch.scene.um(v.at)), 6.0, (2.0, if v.error { ERROR } else { WARNING }));
        }
        if let Some(at) = pointer {
            self.draw_wire(ui, at, shift, |p| screen(sch.scene.um(p)));
        }
        let snapped = self.sheet_helpers(ui, rect, pointer, |p| screen(sch.scene.um(p)));
        self.snap_to(ui.ctx(), snapped);
        self.fade_in(ui, rect);
        match (open, &mut self.sch) {
            (Some(Ok(k)), _) => self.show_page(k),
            (Some(Err(why)), Some(sch)) => sch.status = why,
            _ => {}
        }
    }

    /// The scene the canvas shows: the schematic's page or the board's.
    pub(crate) fn shown_scene(&mut self) -> Option<&mut Scene> {
        if self.sch_view() { self.sch.as_mut().map(|s| &mut s.scene) } else { self.scene.as_mut() }
    }
}

/// A page's name: its sheet's, the root's file's.
fn page_name(page: &Page) -> String {
    page.path.last().cloned().unwrap_or_else(|| page.file.trim_end_matches(".kicad_sch").to_string())
}

/// What an update did, as its undo step says it: `Updated from the schematic: 2 parts added below
/// the board, 1 footprint changed, 2 values changed, 3 nets renamed`.
pub(crate) fn updated(d: &Updated) -> String {
    let counts = [
        (d.parts, n!("part added below the board|parts added below the board")),
        (d.footprints, n!("footprint changed|footprints changed")),
        (d.values, n!("value changed|values changed")),
        (d.fields, n!("field changed|fields changed")),
        (d.renamed, n!("net renamed|nets renamed")),
        (d.added, n!("net added|nets added")),
        (d.moved, n!("pad moved|pads moved")),
    ];
    let said: Vec<String> = counts.iter().filter(|(n, _)| *n > 0).map(|&(n, what)| count(n, what)).collect();
    trf("Updated from the schematic: {0}", &[&said.join(", ")])
}
