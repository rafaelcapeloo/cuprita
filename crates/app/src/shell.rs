//! The window around the canvas: the workbenches (Schematic, Board, 3D, Make) in the header
//! bar, the tool dock and the tool's hint floating on the canvas, and the status bar.

use crate::cmd::Cmd;
use crate::dock::{Dock, Tab};
use crate::tr::{n, t, tr, trf};
use crate::{App, chrome, drawing, icon, net_text, pick_text, picks_name, route, ruler_text, theme, three, units, zone};
use eframe::egui::{self, Align, Align2, Layout, RichText, UiBuilder, pos2, vec2};

/// Home, then the workbenches in the order of the work: one project, seen four ways.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Bench {
    Home,
    Schematic,
    Board,
    ThreeD,
    Make,
}

/// Each workbench's tab and the command that shows it.
const BENCHES: [(Bench, Cmd); 4] =
    [(Bench::Schematic, Cmd::Schematic), (Bench::Board, Cmd::Board), (Bench::ThreeD, Cmd::ThreeD), (Bench::Make, Cmd::Make)];

/// How wide a workbench tab is.
const TAB: f32 = 116.0;

/// Whether the window is as narrow as a phone's page (a served view opened by its QR code): the
/// docks fold away and the header and status bar keep to what fits, the canvas the rest.
pub(crate) fn narrow(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < 700.0
}

impl App {
    pub(crate) fn sch_view(&self) -> bool {
        self.bench == Bench::Schematic
    }

    /// Shows workbench `to`. The schematic and the board keep their own place and zoom on the
    /// canvas (`on_sheet` says whose it has), 3D draws the board; Home and Make draw no canvas.
    pub(crate) fn go(&mut self, to: Bench) {
        if self.bench == to {
            return;
        }
        self.arrive(false);
        if to == Bench::Home {
            self.keep_standing();
            #[cfg(not(target_arch = "wasm32"))]
            self.home.look_again();
        }
        let sheet = match to {
            Bench::Schematic => Some(true),
            Bench::Board | Bench::ThreeD => Some(false),
            Bench::Home | Bench::Make => None,
        };
        if let Some(sheet) = sheet.filter(|&s| s != self.on_sheet) {
            self.toggle_sch();
            // There, or it can't switch yet (a board to read, a question to answer first).
            if self.on_sheet != sheet || to == Bench::Schematic {
                return;
            }
        }
        match to {
            Bench::ThreeD => self.toggle_3d(),
            _ => {
                self.bench = to;
                self.sync_gpu();
                if to == Bench::Make {
                    self.load_models();
                }
            }
        }
    }

    /// The canvas shows another workbench, or (`fresh`) something new there: it fades in
    /// (`motion.rs`), a view on its way stops, and what is new shows whole at once.
    pub(crate) fn arrive(&mut self, fresh: bool) {
        (self.arrived, self.glide, self.fitted, self.fresh) = (web_time::Instant::now(), None, None, self.fresh || fresh);
    }

    /// The canvas `rect` fading in since it last arrived: a veil of its ground, thinning.
    pub(crate) fn fade_in(&self, ui: &egui::Ui, rect: egui::Rect) {
        let t = crate::motion::eased(ui.ctx(), self.arrived, crate::motion::QUICK);
        if t < 1.0 {
            ui.painter().rect_filled(rect, 0.0, theme::CANVAS.gamma_multiply(1.0 - t));
        }
    }

    /// The project's name: its board's, else its schematic's.
    pub(crate) fn project(&self) -> String {
        let file = self.path.clone().or_else(|| self.sch.as_ref().map(|s| s.sheets.root.clone()));
        file.and_then(|f| Some(f.file_stem()?.to_string_lossy().into_owned())).unwrap_or_else(|| "Cuprita".into())
    }

    /// The project's name, a menu: its folder, another project (Open, the recent ones), Home.
    fn project_menu(&mut self, ui: &mut egui::Ui) {
        let here = self.path.clone().or_else(|| self.sch.as_ref().map(|s| s.sheets.root.clone()));
        let name = RichText::new(format!("{}  {}", self.project(), icon::UNFOLD)).family(theme::strong()).color(theme::TEXT);
        let (mut open, mut folder, mut run) = (None, None, None);
        ui.menu_button(name, |ui| {
            if let Some(dir) = here.as_ref().and_then(|p| p.parent()).filter(|_| cfg!(not(target_arch = "wasm32")))
                && ui.button(format!("{}  {}", icon::OPEN, tr("Show its folder"))).clicked()
            {
                folder = Some(dir.display().to_string());
            }
            for cmd in [Cmd::Open, Cmd::Home].into_iter().filter(|&c| self.can(c)) {
                if ui.button(format!("{}  {}", cmd.about().icon, cmd.about().name)).clicked() {
                    run = Some(cmd);
                }
            }
            let others: Vec<&std::path::PathBuf> = self.home.recent.iter().filter(|p| Some(*p) != here.as_ref()).take(6).collect();
            if !others.is_empty() {
                ui.separator();
                ui.label(RichText::new(tr("Recent")).small().color(theme::QUIET));
                for p in others {
                    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                    if ui.button(format!("{}  {stem}", icon::BOARD)).on_hover_text(p.display().to_string()).clicked() {
                        open = Some(p.clone());
                    }
                }
            }
        });
        if let Some(dir) = folder {
            self.open_url(ui.ctx(), &dir);
        }
        if let Some(cmd) = run {
            self.run(ui.ctx(), cmd);
        }
        if let Some(p) = open {
            self.open_project(p);
        }
    }

    /// The header bar: the mark, the project with save, undo and redo; the workbenches in the
    /// middle; find, the checks and Ask on the right.
    pub(crate) fn header(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::new().fill(theme::GROUND).inner_margin(egui::Margin::symmetric(10, 0));
        egui::Panel::top("header").exact_size(46.0).frame(frame).show(ui, |ui| {
            let full = ui.max_rect();
            if chrome::own() {
                chrome::title_bar(ui.ctx(), &ui.interact(full, egui::Id::unique("title bar"), egui::Sense::click_and_drag()));
            }
            let row = |layout| UiBuilder::new().max_rect(full).layout(layout);
            let left = ui.scope_builder(row(Layout::left_to_right(Align::Center)), |ui| {
                let (mark, home) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::click());
                let color = if home.hovered() { theme::tint(theme::COPPER, theme::TEXT, 0.25) } else { theme::COPPER };
                theme::mark(
                    ui.painter(),
                    mark.shrink(2.0),
                    color,
                    crate::motion::eased(ui.ctx(), self.started, 2.0 * crate::motion::ARRIVE),
                );
                if !self.viewer && home.on_hover_text(tr("Home: new, open and recent projects")).clicked() {
                    self.run(ui.ctx(), Cmd::Home);
                }
                ui.add_space(4.0);
                // A served view is of its one project, as it is.
                if self.viewer || self.scene.is_none() && self.sch.is_none() {
                    ui.label(RichText::new(self.project()).family(theme::strong()).color(theme::TEXT));
                    return ui.min_rect().right();
                }
                self.project_menu(ui);
                if self.unsaved() {
                    let (dot, hover) = ui.allocate_exact_size(vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.5, theme::COPPER);
                    hover.on_hover_text(tr("Edits not saved yet (autosaved meanwhile)"));
                }
                ui.add_space(6.0);
                for cmd in [Cmd::Save, Cmd::Undo, Cmd::Redo] {
                    self.icon_cmd(ui, cmd, 30.0, false);
                }
                ui.min_rect().right()
            });
            let right = ui.scope_builder(row(Layout::right_to_left(Align::Center)), |ui| {
                if chrome::own() {
                    chrome::buttons(ui);
                    ui.add_space(10.0);
                }
                if self.can(Cmd::Ask) {
                    self.icon_cmd(ui, Cmd::Ask, 30.0, false);
                }
                if self.bench == Bench::Home {
                    self.icon_cmd(ui, Cmd::Settings, 30.0, false);
                    self.icon_cmd(ui, Cmd::Help, 30.0, false);
                }
                // Share: lit while the project is on its web page; its panel opens under it.
                #[cfg(not(target_arch = "wasm32"))]
                if self.can(Cmd::Share) {
                    let button = theme::tool(ui, icon::SHARE, 30.0, self.on(Cmd::Share), true, "");
                    ui.data_mut(|d| d.insert_temp(egui::Id::unique(crate::share::BUTTON), button.rect));
                    if button.on_hover_text(self.cmd_tip(ui.ctx(), Cmd::Share)).clicked() {
                        self.run(ui.ctx(), Cmd::Share);
                    }
                }
                if self.viewer {
                    let words = if narrow(ui.ctx()) { String::new() } else { format!("  {}", tr("View only")) };
                    let text = RichText::new(format!("{}{words}", icon::SHOWN)).color(theme::SIGNAL).family(theme::strong());
                    let pill = egui::Button::new(text).fill(theme::tint(theme::GROUND, theme::SIGNAL, 0.13)).stroke(egui::Stroke::NONE);
                    ui.add(pill.corner_radius(8).sense(egui::Sense::hover()))
                        .on_hover_text(tr("This page shows the project as its files are, and follows them: nothing here changes them"));
                } else if self.scene.is_some() || self.sch.is_some() {
                    self.checks_pill(ui);
                }
                self.palette_button(ui);
                ui.min_rect().left()
            });
            // The workbenches are a project's: none before one is open.
            if self.scene.is_none() && self.sch.is_none() {
                return;
            }
            // Make is where a board is exported and ordered: not a served view's.
            let benches: Vec<(Bench, Cmd)> = BENCHES.into_iter().filter(|b| !self.viewer || b.0 != Bench::Make).collect();
            // On a narrow window a tab is its icon.
            let small = narrow(ui.ctx());
            let tab_width = if small { 46.0 } else { TAB };
            let width = benches.len() as f32 * tab_width + 8.0;
            let x = (full.center().x - width / 2.0).min(right.inner - 12.0 - width).max(left.inner + 12.0);
            let tabs = egui::Rect::from_min_size(pos2(x, full.center().y - 19.0), vec2(width, 38.0));
            ui.scope_builder(UiBuilder::new().max_rect(tabs), |ui| {
                let frame = egui::Frame::new().fill(theme::CANVAS).stroke((1.0, theme::LINE)).corner_radius(10).inner_margin(3);
                frame.show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.horizontal(|ui| {
                        let mut shown = None;
                        for (bench, cmd) in benches {
                            let about = cmd.about();
                            let tab = theme::tab(ui, self.bench == bench, about.icon, if small { "" } else { about.name }, tab_width);
                            shown = shown.or((self.bench == bench).then_some(tab.rect));
                            if tab.on_hover_text(self.cmd_tip(ui.ctx(), cmd)).clicked() {
                                self.run(ui.ctx(), cmd);
                            }
                        }
                        // The copper line under the workbench shown glides to it from the last one.
                        if let Some(tab) = shown {
                            let x = crate::motion::toward(ui.ctx(), egui::Id::unique("workbench line"), tab.center().x);
                            let line = egui::Rangef::new(x - tab_width / 2.0 + 12.0, x + tab_width / 2.0 - 12.0);
                            ui.painter().hline(line, tab.bottom() - 1.0, egui::Stroke::new(2.0, theme::COPPER));
                        }
                    });
                });
            });
        });
    }

    /// The palette's button, drawn as the field it opens: Find or run…, and its key.
    fn palette_button(&mut self, ui: &mut egui::Ui) {
        if narrow(ui.ctx()) {
            return self.icon_cmd(ui, Cmd::Find, 30.0, false);
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(240.0, 30.0), egui::Sense::click());
        let p = ui.painter();
        let line = if resp.hovered() || self.palette.open { theme::COPPER.gamma_multiply(0.7) } else { theme::LINE };
        p.rect(rect, 8.0, theme::PANEL, (1.0, line), egui::StrokeKind::Inside);
        let y = rect.center().y;
        p.text(
            egui::pos2(rect.left() + 18.0, y),
            Align2::CENTER_CENTER,
            crate::icon::SEARCH,
            egui::FontId::proportional(14.0),
            theme::QUIET,
        );
        p.text(egui::pos2(rect.left() + 32.0, y), Align2::LEFT_CENTER, tr("Find or run…"), egui::FontId::proportional(12.5), theme::QUIET);
        let key = Cmd::Find.about().keys().next().map(|k| crate::cmd::key_text(ui.ctx(), &k)).unwrap_or_default();
        p.text(egui::pos2(rect.right() - 10.0, y), Align2::RIGHT_CENTER, key, egui::FontId::monospace(11.0), theme::QUIET);
        if resp.on_hover_text(self.cmd_tip(ui.ctx(), Cmd::Find)).clicked() {
            self.run(ui.ctx(), Cmd::Find);
        }
    }

    /// How far the project is from being ordered: Make's checks left, or ready; a click shows
    /// Make.
    fn checks_pill(&mut self, ui: &mut egui::Ui) {
        if self.scene.is_none() && self.sch.is_none() {
            return;
        }
        let checks = self.checks();
        let fails = checks.iter().any(|c| c.color == theme::FAIL);
        let left: Vec<String> = checks.into_iter().filter(|c| c.color != theme::PASS).map(|c| c.title).collect();
        let (text, color) = match left.len() {
            0 => (tr("Ready to make").to_string(), theme::PASS),
            n => (trf("{0} to fix", &[&n]), if fails { theme::FAIL } else { theme::WARN }),
        };
        let text = RichText::new(format!("{}  {text}", if left.is_empty() { crate::icon::PASSES } else { crate::icon::NEEDS }))
            .color(color)
            .family(theme::strong());
        let fill = theme::tint(theme::GROUND, color, if self.bench == Bench::Make { 0.28 } else { 0.13 });
        let pill = ui.add(egui::Button::new(text).fill(fill).stroke(egui::Stroke::NONE).corner_radius(8));
        let tip = if left.is_empty() {
            tr("Every check before ordering passes: Make has the fab files").into()
        } else {
            trf("Before ordering: {0}", &[&left.join("; ")])
        };
        if pill.on_hover_text(tip).clicked() {
            self.go(Bench::Make);
        }
    }

    /// The checks of the view shown, counted, in the status bar: the board's design rules, else
    /// the schematic's electrical rules; a click lists them.
    fn rules_count(&mut self, ui: &mut egui::Ui) {
        let (text, color) = match &self.sch {
            Some(s) if self.sch_view() => {
                let errors = s.violations.iter().filter(|v| v.error).count();
                match (errors, s.violations.len() - errors) {
                    (0, 0) => ("✓ ERC".into(), theme::PASS),
                    (0, w) => (format!("ERC {}", crate::count(w, n!("warning"))), theme::WARN),
                    (e, _) => (format!("ERC {}", crate::count(e, n!("error"))), theme::FAIL),
                }
            }
            _ if self.scene.is_none() || self.bench != Bench::Board => return,
            // Rules broken, in red; KiCad's warnings alone (a loose end), as ERC's, in amber.
            _ => match (self.broken(), self.violations.len()) {
                (0, 0) => ("✓ DRC".into(), theme::PASS),
                (0, w) => (format!("DRC {}", crate::count(w, n!("warning"))), theme::WARN),
                (e, _) => (format!("DRC {e}"), theme::FAIL),
            },
        };
        let count = ui.add(egui::Label::new(RichText::new(text).color(color)).sense(egui::Sense::click()));
        if count.on_hover_text(self.cmd_tip(ui.ctx(), Cmd::Checks)).clicked() {
            self.right = Dock { tab: Tab::Checks, open: true };
        }
    }

    /// The tool dock: the workbench's tools, floating at the bottom of the canvas `rect`.
    pub(crate) fn tools(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let groups: &[&[Cmd]] = match self.bench {
            Bench::Board => &[
                &[Cmd::Select, Cmd::Add, Cmd::Route, Cmd::Pair, Cmd::Zone, Cmd::Outline, Cmd::Line, Cmd::Text, Cmd::Measure],
                &[Cmd::Fill, Cmd::Heat, Cmd::Changes, Cmd::Bom],
                &[Cmd::Fit, Cmd::Flip],
            ],
            Bench::Schematic => &[
                &[
                    Cmd::Select,
                    Cmd::Add,
                    Cmd::Wire,
                    Cmd::Bus,
                    Cmd::BusEntry,
                    Cmd::Label,
                    Cmd::GlobalLabel,
                    Cmd::HierLabel,
                    Cmd::NoConnect,
                    Cmd::Sheet,
                ],
                &[Cmd::Simulate, Cmd::Update],
                &[Cmd::Fit],
            ],
            _ => return,
        };
        // A served view has the tools that look.
        let shown = |group: &&[Cmd]| group.iter().copied().filter(|c| !self.viewer || c.looks()).collect::<Vec<_>>();
        let groups: Vec<Vec<Cmd>> = groups.iter().map(shown).filter(|g| !g.is_empty()).collect();
        let area = egui::Area::new(egui::Id::unique("dock")).order(egui::Order::Foreground).fade_in(false);
        area.fixed_pos(rect.center_bottom() - vec2(0.0, 16.0)).pivot(Align2::CENTER_BOTTOM).show(ctx, |ui| {
            floating().corner_radius(14).inner_margin(5).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (k, group) in groups.iter().enumerate() {
                        if k > 0 {
                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(4.0);
                        }
                        for &cmd in group {
                            self.icon_cmd(ui, cmd, 40.0, true);
                        }
                    }
                });
            });
        });
    }

    /// What the tool on does and its keys, floating at the top of the canvas `rect`; the label
    /// tool's name field, the sheet tool's once its box is drawn.
    pub(crate) fn tool_hint(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        // A phone's page has the canvas for itself: its hints speak of a mouse and keys.
        if self.viewer && narrow(ctx) {
            return;
        }
        let sch = self.sch.as_ref().filter(|_| self.sch_view());
        let label = [Cmd::Label, Cmd::GlobalLabel, Cmd::HierLabel].into_iter().find(|&c| self.on(c));
        let (cmd, hint) = match (self.bench, sch) {
            (Bench::Schematic, Some(_)) if label.is_some() => (label.unwrap_or(Cmd::Label), String::new()),
            (Bench::Schematic, Some(s)) if s.sheeting.as_ref().is_some_and(|s| s.drawn) => (Cmd::Sheet, String::new()),
            (Bench::Schematic, Some(s)) if s.sheeting.is_some() => (Cmd::Sheet, s.status.clone()),
            // What is being placed names its own tool: a label isn't Add part's.
            (Bench::Schematic, Some(crate::sheet::Sch { placing: Some(placing), status, .. })) => (placing.cmd(), status.clone()),
            (Bench::Schematic, Some(s)) if s.wire.is_some() => (if s.bus { Cmd::Bus } else { Cmd::Wire }, s.status.clone()),
            (Bench::Schematic, Some(s)) if s.flagging => (Cmd::NoConnect, s.status.clone()),
            (Bench::Board, _) if self.routing => (Cmd::Route, tr(route::HINT).into()),
            (Bench::Board, _) if self.pairing => (Cmd::Pair, tr(crate::pair::HINT).into()),
            (Bench::Board, _) if self.writing.is_some() => (Cmd::Text, String::new()),
            (Bench::Board, _) if self.zoning && self.lining => (Cmd::Line, tr(drawing::LINE_HINT).into()),
            (Bench::Board, _) if self.zoning && self.outlining => (Cmd::Outline, tr(zone::OUTLINE_HINT).into()),
            (Bench::Board, _) if self.zoning && self.cutting => (Cmd::Cutout, tr(zone::CUTOUT_HINT).into()),
            (Bench::Board, _) if self.zoning => (Cmd::Zone, tr(zone::HINT).into()),
            (Bench::Board, _) if self.measuring => (Cmd::Measure, tr("Click two points · Esc clears").into()),
            (Bench::ThreeD, _) => (Cmd::ThreeD, tr(three::HINT).into()),
            _ => return,
        };
        // A board tool's hint names its keys in a line; hovered, it says them all in full.
        let more = match cmd {
            Cmd::Route => route::KEYS,
            Cmd::Pair => crate::pair::KEYS,
            Cmd::Outline => zone::OUTLINE_KEYS,
            Cmd::Line => drawing::LINE_KEYS,
            Cmd::Cutout => zone::CUTOUT_KEYS,
            Cmd::Zone => zone::KEYS,
            _ => "",
        };
        // In the canvas left of First steps while it shows there, as wide as that room allows.
        let guide = self.guide.is_some() && matches!(self.bench, Bench::Schematic | Bench::Board);
        let room = if guide { rect.with_max_x(rect.max.x - crate::guide::TAKES) } else { rect };
        let area = egui::Area::new(egui::Id::unique("tool hint")).order(egui::Order::Foreground).fade_in(false);
        area.fixed_pos(room.center_top() + vec2(0.0, 12.0)).pivot(Align2::CENTER_TOP).show(ctx, |ui| {
            floating().corner_radius(18).inner_margin(egui::Margin::symmetric(14, 7)).show(ui, |ui| {
                ui.set_max_width(room.width() - 40.0);
                ui.horizontal(|ui| {
                    let about = cmd.about();
                    ui.label(RichText::new(about.icon).color(theme::COPPER));
                    ui.label(RichText::new(about.name).family(theme::strong()).color(theme::TEXT));
                    let hint = hint.strip_prefix(&format!("{}: ", about.name)).unwrap_or(&hint);
                    // The Line tool's layer, chosen before its lines are drawn.
                    if let (Cmd::Line, Some(layer)) = (cmd, self.ink_layer())
                        && let Some(to) = self.ink_choice(ui, "line layer", layer)
                    {
                        (self.ink, self.zone) = (Some(to), None);
                    }
                    if hint.is_empty() && cmd == Cmd::Sheet {
                        self.sheet_field(ui);
                    } else if hint.is_empty() && cmd == Cmd::Text {
                        self.text_field(ui);
                    } else if hint.is_empty() {
                        self.label_field(ui);
                    } else {
                        let said = ui.add(egui::Label::new(RichText::new(hint).color(theme::TEXT_2)).wrap());
                        if !more.is_empty() {
                            said.on_hover_text(tr(more));
                        }
                    }
                });
            });
        });
    }

    /// What to do first on an empty sheet or board, in the middle of the canvas `rect`.
    pub(crate) fn empty_hint(&self, ctx: &egui::Context, rect: egui::Rect) {
        let sch = self.sch.as_ref().is_some_and(|s| s.wire.is_some() || s.placing.is_some());
        let tool = self.routing || self.pairing || self.zoning || self.measuring || self.writing.is_some() || sch;
        let (title, what) = match self.bench {
            Bench::Schematic if self.sch.as_ref().is_some_and(|s| s.sheets.project.pages.iter().all(|p| p.sheet.symbols.is_empty())) => (
                tr("An empty sheet"),
                tr("Press A to add a part: a resistor, an LM358, a USB-C connector, or a part number. W wires, L names a net."),
            ),
            Bench::Board if self.board.footprints.is_empty() => (
                tr("An empty board"),
                tr("Press F8 to bring in the schematic's parts, or A to add one: mounting holes, test points, a logo."),
            ),
            _ => return,
        };
        // First steps says it too; what is picked (a text just written) shows under it.
        if tool || self.adding() || self.guide.is_some() || self.bench == Bench::Board && !self.picks().is_empty() {
            return;
        }
        let area = egui::Area::new(egui::Id::unique("empty hint")).order(egui::Order::Foreground).fade_in(false).interactable(false);
        area.fixed_pos(rect.center()).pivot(Align2::CENTER_CENTER).show(ctx, |ui| {
            floating().corner_radius(14).inner_margin(egui::Margin::symmetric(22, 16)).show(ui, |ui| {
                ui.set_max_width(360.0);
                ui.label(RichText::new(title).family(theme::strong()).size(16.0).color(theme::TEXT));
                ui.label(RichText::new(what).color(theme::TEXT_2));
            });
        });
    }

    /// The status bar: where the pointer is, then what was picked or measured, or the news.
    pub(crate) fn status_bar(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::new().fill(theme::GROUND).inner_margin(egui::Margin::symmetric(10, 0));
        egui::Panel::bottom("status").exact_size(26.0).frame(frame).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let wide = !narrow(ui.ctx());
                if wide && matches!(self.bench, Bench::Board | Bench::Schematic) {
                    // Where the pointer is (blank off the canvas, so what follows stays put), then
                    // the units, which a click switches.
                    let at = self.pointer.map_or(" ".repeat(22), |p| format!("X {:>8}  Y {:>8}", units::fixed(p.x), units::fixed(p.y)));
                    ui.label(RichText::new(at).monospace().color(theme::TEXT_2));
                    let name = RichText::new(units::name()).monospace().color(theme::TEXT_2);
                    let tip = if units::mils() {
                        tr("Lengths in mils: a click (or Ctrl+U) shows mm")
                    } else {
                        tr("Lengths in mm: a click (or Ctrl+U) shows mils")
                    };
                    if ui.add(egui::Label::new(name).sense(egui::Sense::click())).on_hover_text(tip).clicked() {
                        self.run(ui.ctx(), Cmd::Units);
                    }
                    self.grid_label(ui);
                    if let Some(name) = &self.snapped {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            ui.label(RichText::new(icon::SNAP).color(theme::SIGNAL));
                            ui.label(RichText::new(trf("snapped to {0}", &[name])).color(theme::TEXT_2));
                        });
                    }
                }
                if let (true, Bench::Board, Some(layer)) = (wide, self.bench, self.scene.as_ref().and_then(|s| s.layers.get(self.active as usize))) {
                    let active = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (swatch, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().rect_filled(swatch, 2.0, crate::color32(layer.color));
                        ui.label(RichText::new(&layer.name).monospace().color(theme::TEXT_2));
                    });
                    active.response.on_hover_text(tr("The active copper layer, which tracks go on: PgUp, PgDn, + and − switch it"));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.benchmark {
                        ui.label(RichText::new(format!("{} fps", self.fps)).monospace());
                    }
                    let saved = ui.label(RichText::new(if self.viewer { String::new() } else { self.saved_text() }).color(theme::QUIET));
                    if self.unsaved() {
                        saved.on_hover_text(tr(
                            "Edits are autosaved 5 s after the last one, beside each file as _autosave-…; Ctrl+S writes them into the files",
                        ));
                    }
                    let left = self.ratsnest.len();
                    if left > 0 && self.scene.is_some() && !self.sch_view() {
                        let text = RichText::new(trf("{0} to route", &[&left])).color(theme::WARN);
                        let count = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                        if count.on_hover_text(tr("Connections still to route: Checks lists them")).clicked() {
                            self.right = Dock { tab: Tab::Checks, open: true };
                        }
                    }
                    self.rules_count(ui);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(self.news()).truncate());
                    });
                });
            });
        });
    }

    /// Keeps what the canvas's pointer snaps to for the status bar, which draws before the
    /// canvas: a change shows on the next frame, asked for at once.
    pub(crate) fn snap_to(&mut self, ctx: &egui::Context, snapped: Option<String>) {
        if snapped != self.snapped {
            self.snapped = snapped;
            ctx.request_repaint();
        }
    }

    /// What the status bar says: the ruler's reading, the route or zone being drawn, the part,
    /// track, via or zone picked, or how many of each (until news comes), the net lit, else the
    /// last news.
    fn news(&self) -> RichText {
        let strong = |s: String| RichText::new(s).color(theme::TEXT);
        if let (Bench::Schematic, Some(s)) = (self.bench, &self.sch) {
            // While a tool is on, its hint says it on the canvas.
            let tool = s.wire.is_some() || s.naming.is_some() || s.placing.is_some() || s.sheeting.is_some();
            return RichText::new(if tool { "" } else { &s.status }).color(theme::TEXT_2);
        }
        if self.bench == Bench::ThreeD {
            return RichText::new(self.models_text()).color(theme::TEXT_2);
        }
        if let Some((a, b, _)) = self.ruler {
            strong(ruler_text(a, b))
        } else if self.routing && self.route.is_some() {
            strong(self.route_text())
        } else if self.pairing && self.status.is_empty() {
            strong(self.pair_text())
        } else if self.zoning && self.zone.is_some() {
            strong(self.zone_text())
        } else if let Some(pick) = self.selected {
            strong(pick_text(&self.board, pick))
        } else if !self.picked.is_empty() && self.status.is_empty() {
            let keys = if self.can(Cmd::FlipPart) {
                t!("{0}  (drag moves, R turns, F flips, Del deletes)")
            } else {
                t!("{0}  (drag moves, R turns, Del deletes)")
            };
            strong(trf(keys, &[&picks_name(&self.board, &self.picked)]))
        } else if !self.picked.is_empty() {
            RichText::new(&self.status).color(theme::TEXT_2)
        } else if self.net != 0 {
            strong(net_text(&self.board, self.net))
        } else {
            RichText::new(&self.status).color(theme::TEXT_2)
        }
    }
}

/// The frame of what floats on the canvas: raised, with a hairline and a shadow.
pub(crate) fn floating() -> egui::Frame {
    egui::Frame::new().fill(theme::PANEL).stroke((1.0, egui::Color32::from_rgb(0x2C, 0x33, 0x3D))).shadow(egui::Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: egui::Color32::from_black_alpha(120),
    })
}
