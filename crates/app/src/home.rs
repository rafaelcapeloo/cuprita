//! Home, where every project starts: a new one (blank, or a copy of one of KiCad's templates),
//! opening one (the system's dialog, or a drop), the projects opened lately with a picture of
//! each board, and KiCad's demo projects to explore.

use crate::cmd::Cmd;
use crate::shell::Bench;
use crate::tr::{t, tr, trf};
use crate::{App, icon, recent, theme};
use cuprita_core::new::{self, Template};
use cuprita_render::Renderer;
use eframe::egui::{self, RichText, vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

/// How wide a project's card is, and its picture.
const CARD: f32 = 230.0;
const PICTURE: [f32; 2] = [CARD, 140.0];

/// What Home keeps between frames.
#[derive(Default)]
pub(crate) struct Home {
    /// The projects opened lately, newest first (`recent.rs`).
    pub(crate) recent: Vec<PathBuf>,
    /// The pictures shown, by the board each shows; `None` when it has none.
    pictures: HashMap<PathBuf, Option<egui::TextureHandle>>,
    /// Where each project stood when it was last open (`recent::standing`).
    standings: HashMap<PathBuf, Option<(String, usize)>>,
    /// What was last found on this computer, and the look under way (`look_around`).
    #[cfg(not(target_arch = "wasm32"))]
    found: (Option<Found>, Option<std::thread::JoinHandle<Found>>),
    /// KiCad's templates and demo projects, found when first wanted.
    templates: Option<Vec<Template>>,
    examples: Option<Vec<PathBuf>>,
    /// The new project being made: the New project dialog is open.
    new: Option<Draft>,
    /// The system's Open, folder or part files dialog, open: its answer to come (the files or
    /// folder picked).
    asking: Option<(Ask, Receiver<Option<Vec<PathBuf>>>)>,
}

/// What this computer has for adding parts, simulating and asking: KiCad's footprint and
/// symbol libraries, ngspice (else what to do about it), and the models Ollama has (its error
/// when it doesn't answer).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Found {
    libraries: (usize, usize),
    ngspice: Result<(), String>,
    models: Result<usize, String>,
}

#[cfg(not(target_arch = "wasm32"))]
fn look_around() -> Found {
    Found {
        libraries: cuprita_core::library::found(),
        ngspice: cuprita_core::ngspice::found(),
        models: cuprita_core::ollama::Ollama::default().models().map(|m| m.0.len()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Home {
    /// Looks around this computer again (Home shows anew: Ollama may have started), keeping
    /// what was found meanwhile.
    pub(crate) fn look_again(&mut self) {
        if self.found.1.is_none() {
            self.found.1 = Some(std::thread::spawn(look_around));
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Ask {
    Open,
    Folder,
    Parts,
    /// A 3D model for footprint `.0`.
    #[cfg(not(target_arch = "wasm32"))]
    Model(usize),
}

/// A new project in the making.
struct Draft {
    name: String,
    /// The folder its own folder goes in.
    parent: PathBuf,
    /// The template it copies, else a blank board of `copper` layers, `size` mm.
    template: Option<usize>,
    copper: u8,
    size: (f64, f64),
    error: String,
    /// Puts the keyboard in the name field once the dialog shows.
    focus: bool,
    /// The project starts the first-board walk-through (Cmd::Tour from Home).
    tour: bool,
}

impl Home {
    /// KiCad's example projects, once Home has looked for them.
    pub(crate) fn examples(&self) -> &[PathBuf] {
        self.examples.as_deref().unwrap_or_default()
    }

    /// Home with the projects opened lately.
    pub(crate) fn new() -> Self {
        Home { recent: recent::read(), ..Default::default() }
    }
}

/// The New project dialog's name field.
const NAME_FIELD: &str = "new project name";

/// Where new projects go: `~/Documents/Cuprita`, else `~/Cuprita`.
fn projects_dir() -> PathBuf {
    let home = std::env::home_dir().unwrap_or_default();
    let docs = home.join("Documents");
    if docs.is_dir() { docs.join("Cuprita") } else { home.join("Cuprita") }
}

/// The name of the first project folder `parent` lacks: `stem` (`board`), else `board-2`, …
fn free_name(parent: &Path, stem: &str) -> String {
    (1..).map(|n| if n == 1 { stem.to_string() } else { format!("{stem}-{n}") }).find(|n| !parent.join(n).exists()).unwrap_or_default()
}

/// The layers a picture of a board shows: copper outside, the front's silkscreen, the outline.
pub(crate) fn picture_layers(names: impl Iterator<Item = impl AsRef<str>>) -> Vec<bool> {
    names.map(|n| matches!(n.as_ref(), "F.Cu" | "B.Cu" | "F.SilkS" | "Edge.Cuts" | "Vias" | "Holes")).collect()
}

impl App {
    /// Shows Home: a project opened from it, or made there, goes to its workbenches.
    pub(crate) fn home_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.add_space(36.0);
            let width = ui.available_width().min(1180.0);
            let margin = (ui.available_width() - width) / 2.0;
            ui.horizontal_top(|ui| {
                ui.add_space(margin.max(24.0));
                ui.vertical(|ui| {
                    ui.set_width(300.0);
                    self.home_start(ui);
                });
                ui.add_space(44.0);
                ui.vertical(|ui| {
                    ui.set_width((width - 344.0).max(CARD));
                    self.home_recent(ui);
                });
            });
            ui.add_space(36.0);
        });
        if self.home.new.is_some() {
            self.new_dialog(ui.ctx());
        }
    }

    /// The left column: what to do first.
    fn home_start(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr("Start a board")).family(theme::strong()).size(26.0).color(theme::TEXT));
        ui.add_space(14.0);
        if cfg!(not(target_arch = "wasm32")) {
            let start = [
                (Cmd::New, tr("Blank, or from a board template")),
                (Cmd::Open, tr("A .kicad_pro, .kicad_pcb or .kicad_sch")),
                (Cmd::Tour, tr("An LED lit by a coin cell, step by step")),
            ];
            for (cmd, what) in start {
                let primary = cmd == Cmd::New;
                if action(ui, cmd, what, primary).on_hover_text(self.cmd_tip(ui.ctx(), cmd)).clicked() {
                    self.run(ui.ctx(), cmd);
                }
                ui.add_space(8.0);
            }
        }
        ui.add_space(8.0);
        egui::Frame::new().stroke((1.5, theme::HOVER)).corner_radius(14).inner_margin(20).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(icon::OPEN).size(26.0).color(theme::QUIET));
                ui.label(RichText::new(tr("Drop a KiCad project anywhere")).color(theme::TEXT_2));
                ui.label(
                    RichText::new(tr("It opens as it is. Nothing is converted; saving keeps every byte you didn't change."))
                        .small()
                        .color(theme::QUIET),
                );
            });
        });
        #[cfg(not(target_arch = "wasm32"))]
        self.found_list(ui);
        ui.add_space(18.0);
        ui.label(RichText::new(tr("TIP")).size(11.0).family(theme::strong()).color(theme::QUIET));
        ui.add(egui::Label::new(RichText::new(tip()).color(theme::TEXT_2)).wrap());
    }

    /// What was found on this computer for adding parts, simulating and Ask, each with what to
    /// do when it is missing; looked for on a thread, as Ollama may take a moment to answer.
    #[cfg(not(target_arch = "wasm32"))]
    fn found_list(&mut self, ui: &mut egui::Ui) {
        let (last, look) = &mut self.home.found;
        if look.as_ref().is_some_and(|l| l.is_finished())
            && let Some(Ok(found)) = look.take().map(|l| l.join())
        {
            *last = Some(found);
        }
        if last.is_none() && look.is_none() {
            *look = Some(std::thread::spawn(look_around));
        }
        if look.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        let Some(f) = last else { return };
        let (fp, sym) = f.libraries;
        let rows = [
            match fp + sym {
                0 => (false, tr("No KiCad libraries: install KiCad to add parts from them").to_string()),
                _ => (true, trf("KiCad's {0} footprint and {1} symbol libraries", &[&fp, &sym])),
            },
            match &f.ngspice {
                Ok(()) => (true, tr("ngspice, to simulate").into()),
                Err(e) => (false, e.lines().next().unwrap_or_default().into()),
            },
            match &f.models {
                Ok(0) => (false, tr("Ollama runs, with no model that looks things up: ollama pull gemma4").into()),
                Ok(n) => (true, trf("Ollama, to Ask: {0}", &[&crate::count(*n, crate::tr::n!("model"))])),
                Err(_) => (false, tr("Ollama isn't running: Ask needs it").into()),
            },
        ];
        let ollama = f.models.is_err();
        ui.add_space(18.0);
        ui.label(RichText::new(tr("FOUND ON THIS COMPUTER")).size(11.0).family(theme::strong()).color(theme::QUIET));
        let mut get = false;
        for (k, (ok, text)) in rows.into_iter().enumerate() {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, if ok { theme::PASS } else { theme::WARN });
                ui.add(egui::Label::new(RichText::new(text).color(theme::TEXT_2)).wrap());
                get |= k == 2 && ollama && ui.link("ollama.com").clicked();
            });
        }
        if get {
            self.open_url(ui.ctx(), "https://ollama.com/download");
        }
    }

    /// The right column: the projects opened lately, then KiCad's demos.
    fn home_recent(&mut self, ui: &mut egui::Ui) {
        let mut open = None;
        let (mut folder, mut forget) = (None, None);
        if !self.home.recent.is_empty() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Recent")).family(theme::strong()).size(17.0).color(theme::TEXT));
                ui.label(RichText::new(tr("Right-click one to show its folder or forget it")).color(theme::QUIET));
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(16.0, 16.0);
                for path in self.home.recent.clone() {
                    let picture = self.picture(ui.ctx(), &path, false);
                    let standing = self.home.standings.entry(path.clone()).or_insert_with(|| recent::standing(&path));
                    let standing = standing.as_ref().and_then(|(what, n)| said(what, *n));
                    let card = project_card(ui, &path, picture.as_ref(), standing);
                    if card.clicked() {
                        open = Some(path.clone());
                    }
                    card.context_menu(|ui| {
                        if ui.button(format!("{}  {}", icon::OPEN, tr("Show its folder"))).clicked() {
                            folder = path.parent().map(Path::to_path_buf);
                        }
                        if ui.button(format!("{}  {}", icon::CLOSE, tr("Forget it"))).clicked() {
                            forget = Some(path.clone());
                        }
                    });
                }
            });
            ui.add_space(22.0);
        }
        if let Some(dir) = folder {
            self.open_url(ui.ctx(), &dir.display().to_string());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = forget {
            recent::forget(&mut self.home.recent, &path);
        }
        let examples = self.home.examples.get_or_insert_with(new::examples).clone();
        if !examples.is_empty() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Examples")).family(theme::strong()).size(17.0).color(theme::TEXT));
                ui.label(RichText::new(tr("KiCad's demo projects, as KiCad installed them")).color(theme::QUIET));
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for pro in examples {
                    let name = pro.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                    if ui.add(egui::Button::new(name).corner_radius(8)).on_hover_text(pro.display().to_string()).clicked() {
                        open = crate::dropped_path(&[pro.as_path()]);
                    }
                }
            });
        } else if self.home.recent.is_empty() && cfg!(not(target_arch = "wasm32")) {
            ui.label(RichText::new(tr("Projects you open show here, with a picture of their board.")).color(theme::QUIET));
        }
        if let Some(path) = open {
            self.open_project(path);
        }
    }

    /// Opens the project of `path` (first asking about unsaved edits), or shows it if it is open.
    pub(crate) fn open_project(&mut self, path: PathBuf) {
        let sch = self.sch.as_ref().map(|s| &s.sheets.root);
        if self.path.as_ref() == Some(&path) || sch == Some(&path) {
            let to = if self.path.as_ref() == Some(&path) { Bench::Board } else { Bench::Schematic };
            return self.go(to);
        }
        self.leave(move |app| app.open(&path));
    }

    /// Ctrl+N: Home, with the New project dialog; `tour`, for the first-board walk-through.
    pub(crate) fn start_new(&mut self, ctx: &egui::Context, tour: bool) {
        self.go(Bench::Home);
        let parent = projects_dir();
        let name = free_name(&parent, if tour { "first-board" } else { "board" });
        // The name offered is selected: typing one's own replaces it (it once made `boardamp`).
        let id = egui::Id::unique(NAME_FIELD);
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(&name)));
        state.store(ctx, id);
        let draft = Draft { name, parent, template: None, copper: 2, size: (100.0, 80.0), error: String::new(), focus: true, tour };
        self.home.new = Some(draft);
        ctx.request_repaint();
    }

    /// Ctrl+O: the system's dialog to open a project, asked on a thread of its own so the
    /// window keeps drawing.
    pub(crate) fn ask_open(&mut self) {
        self.ask_system(Ask::Open);
    }

    /// The system's dialog to pick a part's KiCad files (a vendor's zip), from the downloads
    /// folder, for the open project's libraries (`parts.rs`).
    pub(crate) fn ask_parts(&mut self) {
        self.ask_system(Ask::Parts);
    }

    /// The system's dialog to pick a 3D model (a vendor's STEP) for footprint `i`, from the
    /// downloads folder.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ask_model(&mut self, i: usize) {
        self.ask_system(Ask::Model(i));
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn ask_system(&mut self, ask: Ask) {
        if self.home.asking.is_some() {
            return;
        }
        let (tell, answer) = std::sync::mpsc::channel();
        let project = || self.home.new.as_ref().map(|d| d.parent.clone()).or_else(|| self.path.as_ref()?.parent().map(Path::to_path_buf));
        let start = if matches!(ask, Ask::Parts | Ask::Model(_)) { crate::parts::downloads().or_else(project) } else { project() };
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new();
            if let Some(dir) = start.filter(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            let picked = match ask {
                Ask::Open => dialog
                    .set_title(tr("Open a KiCad project"))
                    .add_filter(tr("KiCad project"), &["kicad_pro", "kicad_pcb", "kicad_sch"])
                    .pick_file()
                    .map(|f| vec![f]),
                Ask::Folder => dialog.set_title(tr("Where the new project goes")).pick_folder().map(|f| vec![f]),
                Ask::Parts => dialog
                    .set_title(tr("A part's KiCad files"))
                    .add_filter(tr("KiCad part files or their zip"), &["zip", "kicad_sym", "kicad_mod", "step", "stp", "wrl"])
                    .pick_files(),
                Ask::Model(_) => dialog
                    .set_title(tr("A part's 3D model"))
                    .add_filter(tr("3D models (STEP, WRL)"), &["step", "stp", "wrl"])
                    .pick_file()
                    .map(|f| vec![f]),
            };
            let _ = tell.send(picked);
        });
        self.home.asking = Some((ask, answer));
    }

    #[cfg(target_arch = "wasm32")]
    fn ask_system(&mut self, _: Ask) {}

    /// The system dialog's answer, once given, whatever shows.
    pub(crate) fn home_answers(&mut self, ctx: &egui::Context) {
        let Some((ask, answer)) = &self.home.asking else { return };
        let Ok(picked) = answer.try_recv() else {
            return ctx.request_repaint_after(std::time::Duration::from_millis(100));
        };
        let ask = *ask;
        self.home.asking = None;
        match (ask, picked.unwrap_or_default()) {
            (Ask::Open, picked) => {
                if let Some(path) = crate::dropped_path(&picked.iter().map(PathBuf::as_path).collect::<Vec<_>>()) {
                    self.open_project(path);
                }
            }
            (Ask::Folder, picked) => {
                if let (Some(d), Some(dir)) = (&mut self.home.new, picked.into_iter().next()) {
                    d.parent = dir;
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            (Ask::Parts, picked) if !picked.is_empty() => self.import_parts(ctx, &picked),
            #[cfg(not(target_arch = "wasm32"))]
            (Ask::Model(i), picked) if !picked.is_empty() => self.attach_model(ctx, i, &picked[0]),
            _ => {}
        }
    }

    /// The New project dialog: a name, where it goes, and what it starts from.
    fn new_dialog(&mut self, ctx: &egui::Context) {
        let templates = self.home.templates.get_or_insert_with(new::templates).clone();
        let frame = egui::Frame::popup(&ctx.global_style()).fill(theme::PANEL).corner_radius(16).inner_margin(24);
        let modal = egui::Modal::new(egui::Id::unique("new project")).frame(frame).show(ctx, |ui| {
            ui.set_width(820.0_f32.min(ctx.content_rect().width() - 80.0));
            let mut create = false;
            ui.label(RichText::new(tr("New project")).family(theme::strong()).size(20.0).color(theme::TEXT));
            ui.add_space(12.0);
            let Some(d) = &mut self.home.new else { return false };
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Name")).color(theme::QUIET));
                let field = egui::TextEdit::singleline(&mut d.name).id(egui::Id::unique(NAME_FIELD)).desired_width(220.0);
                let field = ui.add(field.font(egui::TextStyle::Body));
                if std::mem::take(&mut d.focus) {
                    field.request_focus();
                }
                create |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if field.changed() {
                    d.error.clear();
                }
                ui.add_space(12.0);
                ui.label(RichText::new(tr("Folder")).color(theme::QUIET));
                let shown = d.parent.join(&d.name);
                ui.add(egui::Label::new(RichText::new(format!("{}/", shown.display())).monospace().color(theme::TEXT_2)).truncate());
            });
            let mut change = false;
            ui.horizontal(|ui| {
                ui.add_space(ui.available_width() - 90.0);
                change = ui.button(tr("Change…")).on_hover_text(tr("Pick the folder the project's folder goes in")).clicked();
            });
            ui.add_space(6.0);
            ui.label(RichText::new(tr("START FROM")).size(11.0).family(theme::strong()).color(theme::QUIET));
            ui.add_space(4.0);
            let mut pick = None;
            egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
                    if template_card(ui, d.template.is_none(), None, tr("Blank board"), tr("Any size, 2 to 16 layers")).clicked() {
                        pick = Some(None);
                    }
                    for (k, t) in templates.iter().enumerate() {
                        let picture = self.home.pictures.get(&t.board()).cloned().flatten();
                        let card = template_card(ui, d.template == Some(k), picture.as_ref(), &t.title, &t.name);
                        if card.clicked() {
                            pick = Some(Some(k));
                        }
                    }
                });
            });
            if let Some(p) = pick {
                (d.template, d.error) = (p, String::new());
            }
            if d.template.is_none() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Copper layers")).color(theme::QUIET));
                    for n in [2, 4, 6, 8, 10, 12, 16] {
                        if ui.add(egui::Button::selectable(d.copper == n, n.to_string()).min_size(vec2(32.0, 0.0))).clicked() {
                            d.copper = n;
                        }
                    }
                    ui.add_space(16.0);
                    ui.label(RichText::new(tr("Size")).color(theme::QUIET));
                    ui.add(egui::DragValue::new(&mut d.size.0).range(5.0..=1000.0).suffix(" mm").speed(1.0));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut d.size.1).range(5.0..=1000.0).suffix(" mm").speed(1.0));
                });
            }
            if !d.error.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(&d.error).color(theme::FAIL));
            }
            ui.add_space(14.0);
            let mut cancel = false;
            ui.horizontal(|ui| {
                let files = trf("Writes {0}.kicad_pro, .kicad_sch and .kicad_pcb, which KiCad opens too.", &[&d.name]);
                ui.label(RichText::new(files).small().color(theme::QUIET));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    create |= ui.add(theme::primary(tr("Create project"))).on_hover_text("Enter").clicked();
                    cancel = ui.button(tr("Cancel")).clicked();
                });
            });
            if change {
                self.ask_system(Ask::Folder);
            }
            if create {
                self.create_project();
            }
            cancel
        });
        if modal.inner || modal.should_close() {
            self.home.new = None;
        }
        // Pictures of the templates, one a frame, drawn on the canvas's renderer while Home shows
        // no canvas (a project made goes to its own).
        let open = self.home.new.is_some() && self.bench == Bench::Home;
        if let Some(t) = templates.iter().find(|t| open && !self.home.pictures.contains_key(&t.board())) {
            let board = t.board();
            self.picture(ctx, &board, true);
            ctx.request_repaint();
        }
    }

    /// Writes the new project, then shows its schematic.
    fn create_project(&mut self) {
        let Some(d) = &self.home.new else { return };
        let (dir, tour) = (d.parent.join(d.name.trim()), d.tour);
        let made = match d.template.and_then(|k| self.home.templates.as_ref()?.get(k)) {
            Some(t) => new::copy_template(t, &dir, &d.name),
            None => new::write(&dir, &d.name, d.copper, d.size, new::seed()),
        };
        match made {
            Ok(board) => {
                self.home.new = None;
                self.leave(move |app| {
                    app.open(&board);
                    app.go(Bench::Schematic);
                    (app.guide, app.tour) = (Some(true), tour);
                });
            }
            Err(e) => self.home.new.iter_mut().for_each(|d| d.error = e.clone()),
        }
    }

    /// The picture of the board at `board`: kept from when it was last opened, or, with `draw`,
    /// drawn now on the canvas's renderer (which then holds that board).
    fn picture(&mut self, ctx: &egui::Context, board: &Path, draw: bool) -> Option<egui::TextureHandle> {
        if let Some(p) = self.home.pictures.get(board) {
            return p.clone();
        }
        let image = if draw { self.draw_picture(board) } else { recent::thumb(board).and_then(|f| read_png(&f)) };
        let texture = image.map(|i| ctx.load_texture(board.display().to_string(), i, egui::TextureOptions::LINEAR));
        self.home.pictures.insert(board.to_path_buf(), texture.clone());
        texture
    }

    /// Reads and draws the board at `path`, as a project's card shows it.
    fn draw_picture(&mut self, path: &Path) -> Option<egui::ColorImage> {
        let board = cuprita_core::board::Board::parse(&std::fs::read_to_string(path).ok()?).ok()?;
        let scene = cuprita_render::build(&board);
        let mut renderer = self.gpu.renderer.write();
        let r = renderer.callback_resources.get_mut::<Renderer>()?;
        r.upload(&self.gpu.device, &self.gpu.queue, &scene);
        let visible = picture_layers(scene.layers.iter().map(|l| &l.name));
        let (w, h, rgba) = r.shot(&self.gpu.device, &self.gpu.queue, scene.bounds, (PICTURE[0] * 2.0) as u32, &visible, 0, false);
        Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba))
    }

    /// Where the open project stands, for its card on Home (`said`): what, and how many.
    #[cfg(not(target_arch = "wasm32"))]
    fn standing(&self) -> (&'static str, usize) {
        if self.board.footprints.is_empty() {
            ("empty", 0)
        } else if self.broken() > 0 {
            ("drc", self.broken())
        } else if !self.ratsnest.is_empty() {
            ("route", self.ratsnest.len())
        } else if self.board.outline_size().is_none() {
            ("outline", 0)
        } else {
            ("ready", 0)
        }
    }

    /// Keeps where the open board's project stands, for its card on Home: on opening it,
    /// saving it and going Home.
    pub(crate) fn keep_standing(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some(p), None, false) = (self.path.clone(), &self.screenshot, self.benchmark) {
            let (what, n) = self.standing();
            recent::keep_standing(&p, what, n);
            self.home.standings.remove(&p);
        }
    }

    /// Keeps a picture of the board just opened for Home's card, from the scene uploaded.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn keep_picture(&mut self, path: &Path, bounds: [f32; 4], visible: &[bool]) {
        let Some(file) = recent::thumb(path) else { return };
        let renderer = self.gpu.renderer.read();
        let Some(r) = renderer.callback_resources.get::<Renderer>() else { return };
        let (w, h, rgba) = r.shot(&self.gpu.device, &self.gpu.queue, bounds, (PICTURE[0] * 2.0) as u32, visible, 0, false);
        self.home.pictures.remove(path);
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")));
            let _ = crate::save_png(&file, w, h, &rgba);
        });
    }
}

/// A PNG file's pixels.
fn read_png(path: &Path) -> Option<egui::ColorImage> {
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let mut png = png::Decoder::new(file).read_info().ok()?;
    let mut rgba = vec![0; png.output_buffer_size()?];
    let info = png.next_frame(&mut rgba).ok()?;
    (info.color_type == png::ColorType::Rgba)
        .then(|| egui::ColorImage::from_rgba_unmultiplied([info.width as usize, info.height as usize], &rgba))
}

/// One of Home's big buttons: an icon, a name and what it does; copper when `primary`.
fn action(ui: &mut egui::Ui, cmd: Cmd, what: &str, primary: bool) -> egui::Response {
    let about = cmd.about();
    let (fill, text, quiet) = if primary {
        (theme::COPPER, theme::ON_COPPER, theme::ON_COPPER.gamma_multiply(0.75))
    } else {
        (theme::PANEL, theme::TEXT, theme::QUIET)
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), egui::Sense::click());
    let p = ui.painter();
    let fill = if resp.hovered() { theme::tint(fill, theme::TEXT, 0.06) } else { fill };
    p.rect(rect, 12.0, fill, (1.0, if primary { fill } else { theme::LINE }), egui::StrokeKind::Inside);
    let tile = egui::Rect::from_center_size(rect.left_center() + vec2(34.0, 0.0), vec2(40.0, 40.0));
    p.rect_filled(tile, 10.0, if primary { theme::ON_COPPER.gamma_multiply(0.12) } else { theme::RAISED });
    p.text(tile.center(), egui::Align2::CENTER_CENTER, about.icon, egui::FontId::proportional(20.0), text);
    let x = tile.right() + 14.0;
    // Its words stay on the card, however narrow the column.
    let p = p.with_clip_rect(rect.shrink(4.0));
    p.text(egui::pos2(x, rect.center().y - 9.0), egui::Align2::LEFT_CENTER, about.name, egui::FontId::new(14.0, theme::strong()), text);
    p.text(egui::pos2(x, rect.center().y + 10.0), egui::Align2::LEFT_CENTER, what, egui::FontId::proportional(12.0), quiet);
    if let Some(k) = about.keys().next() {
        let at = egui::pos2(rect.right() - 14.0, rect.center().y - 9.0);
        p.text(at, egui::Align2::RIGHT_CENTER, crate::cmd::key_text(ui.ctx(), &k), egui::FontId::monospace(11.0), quiet);
    }
    resp
}

/// A project's standing (`App::standing`) as its card says it: `pass`, `warn`, `fail` or `info`,
/// and in words, in the language shown; none for one kept otherwise (an older Cuprita's).
fn said(what: &str, n: usize) -> Option<(&'static str, String)> {
    Some(match what {
        "empty" => ("info", tr("No parts on the board yet").into()),
        "drc" => ("fail", crate::count(n, crate::tr::n!("DRC problem"))),
        "route" => ("warn", trf("{0} to route", &[&n])),
        "outline" => ("warn", tr("No board outline").into()),
        "ready" => ("pass", tr("Ready to make").into()),
        _ => return None,
    })
}

/// A recent project's card: its board's picture, its name, its folder and when it was edited.
fn project_card(ui: &mut egui::Ui, path: &Path, picture: Option<&egui::TextureHandle>, standing: Option<(&str, String)>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(CARD, PICTURE[1] + 66.0), egui::Sense::click());
    let p = ui.painter();
    let line = if resp.hovered() { theme::HOVER } else { theme::LINE };
    p.rect(rect, 14.0, theme::PANEL, (1.0, line), egui::StrokeKind::Inside);
    let top = egui::Rect::from_min_size(rect.min, PICTURE.into());
    p.rect_filled(top.shrink(1.0), egui::CornerRadius { nw: 13, ne: 13, sw: 0, se: 0 }, theme::CANVAS);
    match picture {
        Some(t) => {
            let size = t.size_vec2() * (top.shrink(14.0).size() / t.size_vec2()).min_elem();
            p.image(
                t.id(),
                egui::Rect::from_center_size(top.center(), size),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            p.text(top.center(), egui::Align2::CENTER_CENTER, icon::SCHEMATIC, egui::FontId::proportional(30.0), theme::QUIET);
        }
    }
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    let folder = path.parent().map(|d| d.display().to_string()).unwrap_or_default();
    let home = std::env::home_dir().map(|h| h.display().to_string()).unwrap_or_default();
    let folder = match folder.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => folder,
    };
    // A long folder keeps its end, where the project's own name is.
    let n = folder.chars().count();
    let folder = if n > 32 { format!("…{}", folder.chars().skip(n - 31).collect::<String>()) } else { folder };
    let edited = std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(crate::file::ago);
    let x = rect.left() + 14.0;
    p.text(egui::pos2(x, top.bottom() + 18.0), egui::Align2::LEFT_CENTER, name, egui::FontId::new(14.0, theme::strong()), theme::TEXT);
    p.text(egui::pos2(x, top.bottom() + 36.0), egui::Align2::LEFT_CENTER, folder, egui::FontId::monospace(11.0), theme::QUIET);
    let mut room = rect.right() - 14.0 - x;
    if let Some(edited) = edited {
        let at = egui::pos2(rect.right() - 14.0, rect.bottom() - 14.0);
        room = p.text(at, egui::Align2::RIGHT_CENTER, edited, egui::FontId::proportional(11.5), theme::QUIET).left() - 8.0 - x;
    }
    let mut hover = path.display().to_string();
    // Where it stood when last open, as a pill in the room left of when it was edited.
    if let Some((kind, words)) = standing {
        hover = format!("{hover}\n{words}");
        let color = match kind {
            "pass" => theme::PASS,
            "warn" => theme::WARN,
            "fail" => theme::FAIL,
            _ => theme::TEXT_2,
        };
        let words = if kind == "pass" { format!("✓ {words}") } else { words };
        let mut job = egui::text::LayoutJob::simple_singleline(words, egui::FontId::new(11.5, theme::strong()), color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(room - 16.0);
        let galley = p.layout_job(job);
        let pill =
            egui::Rect::from_min_size(egui::pos2(x, rect.bottom() - 14.0 - galley.size().y / 2.0 - 3.0), galley.size() + vec2(16.0, 6.0));
        p.rect_filled(pill, 999.0, theme::tint(theme::PANEL, color, 0.14));
        p.galley(pill.min + vec2(8.0, 3.0), galley, color);
    }
    resp.on_hover_text(hover)
}

/// A template's card in the New project dialog, lit when `picked`.
fn template_card(ui: &mut egui::Ui, picked: bool, picture: Option<&egui::TextureHandle>, title: &str, what: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(180.0, 150.0), egui::Sense::click());
    let p = ui.painter();
    let line = if picked {
        theme::COPPER
    } else if resp.hovered() {
        theme::HOVER
    } else {
        theme::LINE
    };
    p.rect(rect, 12.0, theme::GROUND, (if picked { 2.0 } else { 1.0 }, line), egui::StrokeKind::Inside);
    let top = egui::Rect::from_min_size(rect.min, vec2(rect.width(), 100.0)).shrink(10.0);
    match picture {
        Some(t) => {
            let size = t.size_vec2() * (top.size() / t.size_vec2()).min_elem();
            p.image(
                t.id(),
                egui::Rect::from_center_size(top.center(), size),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            let outline = egui::Rect::from_center_size(top.center(), vec2(110.0, 70.0));
            p.rect_stroke(outline, 6.0, (1.5, egui::Color32::from_rgb(0xF2, 0xD6, 0x49)), egui::StrokeKind::Inside);
        }
    }
    let x = rect.left() + 12.0;
    let mut job = egui::text::LayoutJob::single_section(
        title.into(),
        egui::TextFormat::simple(egui::FontId::new(12.5, theme::strong()), theme::TEXT),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 24.0);
    p.galley(egui::pos2(x, rect.top() + 106.0), ui.fonts_mut(|f| f.layout_job(job)), theme::TEXT);
    p.text(egui::pos2(x, rect.bottom() - 12.0), egui::Align2::LEFT_CENTER, what, egui::FontId::proportional(11.0), theme::QUIET);
    resp
}

/// Today's tip: a key worth knowing.
fn tip() -> &'static str {
    const TIPS: [&str; 8] = [
        t!("Ctrl+E jumps between a part's symbol and its footprint."),
        t!("Ctrl+K finds any command, part or net by a few of its letters."),
        t!("Home fits the board or the sheet in view again."),
        t!("F8 brings the schematic's changes onto the board."),
        t!("D lists the design rule problems; hover one for its fix."),
        t!("? asks a model on this computer about the project; it only looks."),
        t!("Shift+B ticks the parts off as you place them by hand."),
        t!("Drop a .kicad_pro on the window to open its board."),
    ];
    let day = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400);
    tr(TIPS[day as usize % TIPS.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_words_what_was_kept_and_skips_what_it_cannot() {
        assert_eq!(said("drc", 3), Some(("fail", "3 DRC problems".to_string())));
        assert_eq!(said("route", 1), Some(("warn", "1 to route".to_string())));
        assert_eq!(said("pass", 0), None);
    }
}
