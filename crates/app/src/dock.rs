//! The docks beside the canvas, one tab shown in each: the project, its layers and the parts to
//! add on the left; what is picked, the checks, Ask, and the results of a heatmap, the changes
//! or the BOM checklist (while they are on) on the right. A dock folds to a rail of its tabs.

use crate::cmd::Cmd;
use crate::shell::Bench;
use crate::tr::{n, tr, trf};
use crate::{App, color32, icon, theme};
use cuprita_core::board::{Board, LayerId};
use cuprita_render::Layer;
use eframe::egui::{self, RichText, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tab {
    Project,
    Layers,
    Parts,
    Inspect,
    Checks,
    Ask,
    Heat,
    Changes,
    Bom,
}

impl Tab {
    fn about(self) -> (&'static str, &'static str) {
        match self {
            Tab::Project => (tr("Project"), icon::OPEN),
            Tab::Layers => (tr("Layers"), icon::LAYERS),
            Tab::Parts => (tr("Parts"), icon::PART),
            Tab::Inspect => (tr("Inspect"), icon::INSPECT),
            Tab::Checks => (tr("Checks"), icon::CHECKS),
            Tab::Ask => (tr("Ask"), icon::ASK),
            Tab::Heat => (tr("IR drop"), icon::HEAT),
            Tab::Changes => (tr("Changes"), icon::CHANGES),
            Tab::Bom => (tr("BOM"), icon::BOM),
        }
    }
}

/// A dock: the tab it shows and whether it is open, else folded to a rail.
pub(crate) struct Dock {
    pub(crate) tab: Tab,
    pub(crate) open: bool,
}

impl Dock {
    /// Whether it shows `tab`.
    pub(crate) fn shows(&self, tab: Tab) -> bool {
        self.open && self.tab == tab
    }

    /// Shows `tab`, or, when it does, `back` instead.
    pub(crate) fn toggle(&mut self, tab: Tab, back: Tab) {
        (self.tab, self.open) = (if self.shows(tab) { back } else { tab }, true);
    }
}

impl App {
    /// The docks, before the canvas takes the space left; and what runs in the background for
    /// them (a part number's lookup, the chat's answer).
    pub(crate) fn docks(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.poll_lookup(ui.ctx());
            self.chat_poll();
        }
        let canvas = matches!(self.bench, Bench::Schematic | Bench::Board | Bench::ThreeD);
        let mut right = vec![Tab::Inspect, Tab::Checks];
        if cfg!(not(target_arch = "wasm32")) {
            right.push(Tab::Ask);
        }
        for (on, tab) in [(self.show_ir, Tab::Heat), (self.show_diff, Tab::Changes), (self.show_bom, Tab::Bom)] {
            if on && !self.sch_view() {
                right.push(tab);
            }
        }
        // Home and Make are pages: only Ask comes along. A narrow window (a phone's page) keeps
        // the right dock away while folded: the canvas has little enough.
        let room = self.right.open || !crate::shell::narrow(ui.ctx());
        if room && (canvas || self.right.shows(Tab::Ask)) {
            self.dock(ui, true, if canvas { &right } else { &[Tab::Ask] });
        }
        if canvas {
            self.dock(ui, false, if self.viewer { &[Tab::Project, Tab::Layers] } else { &[Tab::Project, Tab::Layers, Tab::Parts] });
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.sch_view() {
            self.sim_panel(ui);
        }
    }

    fn dock(&mut self, ui: &mut egui::Ui, right: bool, tabs: &[Tab]) {
        let dock = if right { &mut self.right } else { &mut self.left };
        if !tabs.contains(&dock.tab) {
            dock.tab = tabs[0];
        }
        let (open, shown) = (dock.open, dock.tab);
        let checks = self.checks_count();
        let panel = if right { egui::Panel::right("right dock") } else { egui::Panel::left("left dock") };
        let frame = egui::Frame::new().fill(theme::PANEL);
        let mut pick = None;
        if !open {
            // Folded: a rail of the tabs' icons.
            panel.exact_size(44.0).resizable(false).frame(frame.inner_margin(egui::Margin::symmetric(4, 8))).show(ui, |ui| {
                for &tab in tabs {
                    let (name, icon) = tab.about();
                    if theme::tool(ui, icon, 36.0, false, true, "").on_hover_text(name).clicked() {
                        pick = Some(tab);
                    }
                }
            });
        } else {
            let size = if right { 310.0 } else { 250.0 };
            panel.resizable(true).default_size(size).size_range(210.0..=520.0).frame(frame).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 6, top: 0, bottom: 0 }).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        for &tab in tabs {
                            let (name, icon) = tab.about();
                            let count = if tab == Tab::Checks { checks } else { None };
                            if theme::dock_tab(ui, tab == shown, icon, name, count).clicked() {
                                pick = Some(tab);
                            }
                        }
                        // At the right, else after the tabs: the dock widens to hold longer names.
                        ui.add_space((ui.available_width() - 26.0).max(0.0));
                        if theme::tool(ui, icon::FOLD, 26.0, false, true, "").on_hover_text(tr("Fold the dock")).clicked() {
                            if right { &mut self.right } else { &mut self.left }.open = false;
                        }
                    });
                });
                let line = ui.max_rect().x_range();
                ui.painter().hline(line, ui.cursor().top(), (1.0, theme::LINE));
                egui::Frame::new().inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match shown {
                        Tab::Project => self.project_tab(ui),
                        Tab::Layers => self.layers_tab(ui),
                        Tab::Parts => self.parts_tab(ui),
                        // A part's many fields, a class's values: past the window's foot, it scrolls.
                        Tab::Inspect => drop(egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.inspect_tab(ui))),
                        Tab::Checks if self.sch_view() => self.erc_tab(ui),
                        Tab::Checks => self.drc_tab(ui),
                        #[cfg(not(target_arch = "wasm32"))]
                        Tab::Ask => self.ask_tab(ui),
                        Tab::Heat => self.heat_tab(ui),
                        Tab::Changes => self.changes_tab(ui),
                        Tab::Bom => self.bom_tab(ui),
                        #[cfg(target_arch = "wasm32")]
                        Tab::Ask => {}
                    }
                });
            });
        }
        if let Some(tab) = pick {
            let dock = if right { &mut self.right } else { &mut self.left };
            (dock.tab, dock.open) = (tab, true);
            match tab {
                Tab::Parts => self.open_parts(ui.ctx()),
                #[cfg(not(target_arch = "wasm32"))]
                Tab::Ask => crate::focus(ui.ctx(), crate::chat::FIELD, ""),
                _ => {}
            }
        }
    }

    /// What Checks lists, counted for its tab: ERC's violations in the schematic, DRC's and the
    /// connections to route on the board; red with an error among them, else amber; none, none.
    fn checks_count(&self) -> Option<(usize, egui::Color32)> {
        let (n, errors) = match &self.sch {
            Some(s) if self.sch_view() => (s.violations.len(), s.violations.iter().any(|v| v.error)),
            _ => (self.violations.len() + self.ratsnest.len(), self.broken() > 0),
        };
        (n > 0).then_some((n, if errors { theme::FAIL } else { theme::WARN }))
    }

    /// The project: its sheets (a click shows one; New sheet adds one), its board, and the fab
    /// files written so far.
    fn project_tab(&mut self, ui: &mut egui::Ui) {
        let (mut go, mut new_sheet) = (None, false);
        if let Some(sch) = &self.sch {
            heading(ui, tr("Schematic"));
            // Pages in the order of the hierarchy, each below its parent.
            let pages = &sch.sheets.project.pages;
            let mut order = vec![(0, 0)];
            let mut k = 0;
            while k < order.len() {
                let (page, depth) = order[k];
                let children = pages.iter().enumerate().filter(|(_, p)| p.parent == Some(page)).map(|(c, _)| (c, depth + 1));
                order.splice(k + 1..k + 1, children);
                k += 1;
            }
            for (page, depth) in order.into_iter().filter(|&(p, _)| p < pages.len()) {
                let p = &pages[page];
                let name = p.path.last().cloned().unwrap_or_else(|| p.file.trim_end_matches(".kicad_sch").to_string());
                let shown = self.sch_view() && sch.page == page;
                let text = RichText::new(format!("{}  {name}", if depth == 0 { icon::SCHEMATIC } else { icon::FILE }));
                // Its parts (a part's units once), as the board will hold them.
                let parts: std::collections::HashSet<&str> =
                    p.sheet.symbols.iter().filter(|s| s.on_board()).map(|s| s.designator.as_str()).collect();
                ui.horizontal(|ui| {
                    ui.add_space(depth as f32 * 14.0);
                    if ui.add(egui::Button::selectable(shown, text)).on_hover_text(&p.file).clicked() {
                        go = Some(page);
                    }
                    if !parts.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(crate::count(parts.len(), n!("part"))).size(11.5).color(theme::QUIET));
                        });
                    }
                });
            }
            if !self.viewer {
                let new = format!("{}  {}", icon::SHEET, tr("New sheet"));
                let tip = tr("Adds a sheet below the one shown, its box where there is room: type its name, then Enter");
                new_sheet = ui.link(new).on_hover_text(tip).clicked();
            }
            ui.add_space(8.0);
        } else if let Some(root) = self.path.as_ref().map(|p| p.with_extension("kicad_sch")).filter(|r| crate::file::written(r).is_some()) {
            // The board's schematic, not read yet: a click reads it and shows its root.
            heading(ui, tr("Schematic"));
            let name = root.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let text = format!("{}  {name}", icon::SCHEMATIC);
            if ui.add(egui::Button::selectable(false, text)).on_hover_text(root.display().to_string()).clicked() {
                go = Some(0);
            }
            ui.add_space(8.0);
        }
        if let Some(path) = self.path.clone() {
            heading(ui, tr("Board"));
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let shown = !self.sch_view();
            if ui
                .add(egui::Button::selectable(shown, format!("{}  {name}", icon::BOARD)))
                .on_hover_text(path.display().to_string())
                .clicked()
            {
                self.go(Bench::Board);
            }
            // What F8 would bring over from the schematic.
            if let Some((n @ 1.., what)) = self.behind.changes().filter(|_| !self.viewer) {
                let behind = trf("{0} behind the schematic: {1}", &[&crate::count(n, n!("change")), &what]);
                ui.label(RichText::new(behind).color(theme::WARN));
                let update = format!("{}  {}", icon::UPDATE, tr("Update the board (F8)"));
                if ui.link(update).on_hover_text(self.cmd_tip(ui.ctx(), Cmd::Update)).clicked() {
                    self.run(ui.ctx(), Cmd::Update);
                }
            }
            ui.add_space(8.0);
            heading(ui, tr("Fab files"));
            let stem = path.with_extension("");
            let mut any = false;
            for end in ["-gerbers.zip", "-bom.csv", "-positions.csv"] {
                let file = std::path::PathBuf::from(format!("{}{end}", stem.display()));
                if let Ok(meta) = std::fs::metadata(&file) {
                    any = true;
                    let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{}  {name}", icon::FILE)).color(theme::TEXT_2));
                        ui.label(RichText::new(format!("{} KB", meta.len().div_ceil(1024))).monospace().color(theme::QUIET));
                    });
                }
            }
            if !any {
                ui.label(RichText::new(tr("None yet: Make writes them (Ctrl+P)")).color(theme::QUIET));
            }
        }
        self.files_list(ui);
        if let Some(page) = go {
            self.go(Bench::Schematic);
            if self.sch_view() {
                self.show_page(page);
            }
        }
        if new_sheet {
            self.go(Bench::Schematic);
            self.new_sheet();
        }
    }

    /// The project's documents (README first) and the datasheets its parts link to, each opened
    /// in the system's app by a click.
    fn files_list(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        let docs = self.documents();
        #[cfg(target_arch = "wasm32")]
        let docs: Vec<std::path::PathBuf> = Vec::new();
        let mut sheets: Vec<(String, String)> = vec![];
        for f in &self.board.footprints {
            let link = f.fields.iter().find(|(k, _)| k.eq_ignore_ascii_case("datasheet")).map(|(_, v)| v);
            if let Some(url) = link.filter(|u| u.starts_with("http://") || u.starts_with("https://"))
                && !sheets.iter().any(|(u, _)| u == url)
            {
                sheets.push((url.clone(), format!("{} · {}", f.reference, f.value)));
            }
        }
        if docs.is_empty() && sheets.is_empty() {
            return;
        }
        ui.add_space(8.0);
        heading(ui, tr("Files"));
        let mut open = None;
        for doc in docs.iter().take(8) {
            let name = doc.file_name().unwrap_or_default().to_string_lossy();
            if ui.add(egui::Button::selectable(false, format!("{}  {name}", icon::FILE))).on_hover_text(doc.display().to_string()).clicked()
            {
                open = Some(doc.display().to_string());
            }
        }
        if docs.len() > 8 {
            ui.label(RichText::new(trf("and {0} more", &[&(docs.len() - 8)])).color(theme::QUIET));
        }
        if !sheets.is_empty() {
            egui::CollapsingHeader::new(trf("Datasheets · {0}", &[&sheets.len()])).show(ui, |ui| {
                for (url, part) in &sheets {
                    if ui.link(part).on_hover_text(url).clicked() {
                        open = Some(url.clone());
                    }
                }
            });
        }
        if let Some(what) = open {
            self.open_url(ui.ctx(), &what);
        }
    }

    /// The documents in the board's folder, README first: read again at most every 10 s while
    /// they show (git lists the folder's files).
    #[cfg(not(target_arch = "wasm32"))]
    fn documents(&mut self) -> Vec<std::path::PathBuf> {
        let Some(dir) = self.path.as_ref().and_then(|p| p.parent()).map(|d| d.to_path_buf()) else { return Vec::new() };
        if self.docs.as_ref().is_none_or(|(d, at, _)| *d != dir || at.elapsed().as_secs() >= 10) {
            let readme = |p: &std::path::Path| p.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("readme"));
            let doc = |p: &std::path::PathBuf| {
                let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                // By what it is, a README too: the system opens it, and would run a `README.bat`.
                readme(p) && ext.is_empty() || matches!(ext.as_str(), "md" | "txt" | "pdf" | "rst" | "adoc")
            };
            let mut docs: Vec<_> = crate::chat::files::files(&dir).into_iter().filter(doc).map(|p| dir.join(p)).collect();
            docs.sort_by_key(|p| (!readme(p), p.components().count(), p.clone()));
            self.docs = Some((dir, web_time::Instant::now(), docs));
        }
        self.docs.as_ref().map(|(_, _, d)| d.clone()).unwrap_or_default()
    }

    /// Every layer of the board (the schematic's inks), its colour, and an eye that shows or hides
    /// it; on the board, ready-made sets of them, and a click on a copper layer makes it active.
    fn layers_tab(&mut self, ui: &mut egui::Ui) {
        let board = !self.sch_view();
        let (copper, active) = (self.board.copper, self.active as usize);
        // An inner copper layer is named by its plane: the net of its biggest zone.
        let planes: Vec<(usize, String)> = match self.board.stackup.as_slice() {
            [_, inner @ .., _] if board => {
                inner.iter().filter_map(|c| Some((c.layer as usize, plane(&self.board, c.layer as usize)?.to_string()))).collect()
            }
            _ => vec![],
        };
        let mut activate = None;
        let Some(scene) = self.shown_scene() else { return };
        if board {
            ui.horizontal_wrapped(|ui| {
                type Shown = fn(&str, bool) -> bool;
                let sets: [(&str, Shown); 4] = [
                    (tr("All"), |_, _| true),
                    (tr("Copper"), |name, copper| copper || matches!(name, "Edge.Cuts" | "Vias" | "Holes")),
                    (tr("Front"), |name, _| name.starts_with("F.") || matches!(name, "Edge.Cuts" | "Vias" | "Holes")),
                    (tr("Back"), |name, _| name.starts_with("B.") || matches!(name, "Edge.Cuts" | "Vias" | "Holes")),
                ];
                for (name, shown) in sets {
                    if ui.add(egui::Button::new(name).corner_radius(12)).clicked() {
                        for (l, layer) in scene.layers.iter_mut().enumerate() {
                            layer.visible = shown(&layer.name, l < 128 && copper & 1 << l != 0);
                        }
                    }
                }
            });
            ui.add_space(6.0);
        }
        let is_copper = |l: usize| board && l < 128 && copper & 1 << l != 0;
        let mut row = |ui: &mut egui::Ui, l: usize, layer: &mut Layer| {
            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), egui::Sense::click());
            // A copper layer's row makes it active, its eye shows or hides it; another's row is its eye.
            let cu = is_copper(l);
            let eye =
                if cu { ui.interact(row.with_min_x(row.max.x - 30.0), resp.id.with("eye"), egui::Sense::click()) } else { resp.clone() };
            let p = ui.painter();
            if resp.hovered() || eye.hovered() || cu && l == active {
                p.rect_filled(row, 6.0, theme::RAISED);
            }
            if cu && l == active {
                p.rect_filled(egui::Rect::from_center_size(row.left_center() + vec2(1.5, 0.0), vec2(3.0, 14.0)), 1.5, theme::COPPER);
            }
            let swatch = egui::Rect::from_center_size(row.left_center() + vec2(14.0, 0.0), vec2(12.0, 12.0));
            p.rect_filled(swatch, 3.0, color32(layer.color).gamma_multiply(if layer.visible { 1.0 } else { 0.35 }));
            let color = match (layer.visible, cu && l == active) {
                (false, _) => theme::QUIET,
                (true, true) => theme::TEXT,
                (true, false) => theme::TEXT_2,
            };
            // The renderer's vias and holes in the language shown; the board's layers by KiCad's names.
            let name = match layer.name.as_str() {
                "Vias" => tr("Vias"),
                "Holes" => tr("Holes"),
                name => name,
            };
            let name =
                p.text(row.left_center() + vec2(30.0, 0.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.0), color);
            // The active layer says so, before its eye.
            let tag = cu && l == active;
            if tag {
                let at = row.right_center() - vec2(32.0, 0.0);
                p.text(at, egui::Align2::RIGHT_CENTER, tr("ACTIVE"), egui::FontId::new(10.5, theme::strong()), theme::COPPER);
            }
            if let Some((_, net)) = planes.iter().find(|(p, _)| *p == l) {
                // A sheet's net by its own name (`/Power/VOUT` is VOUT), clear of the eye (and tag).
                let net = if net.starts_with('/') { net.rsplit('/').next().unwrap_or(net) } else { net };
                let at = egui::pos2(name.right() + 6.0, row.center().y);
                let clear = p.with_clip_rect(row.with_max_x(row.max.x - if tag { 84.0 } else { 30.0 }));
                clear.text(at, egui::Align2::LEFT_CENTER, format!("· {net}"), egui::FontId::proportional(12.0), theme::QUIET);
            }
            let icon = if layer.visible { icon::SHOWN } else { icon::HIDDEN };
            p.text(row.right_center() - vec2(10.0, 0.0), egui::Align2::RIGHT_CENTER, icon, egui::FontId::proportional(15.0), color);
            if eye.on_hover_text(if layer.visible { tr("Click hides it") } else { tr("Click shows it") }).clicked() {
                layer.visible = !layer.visible;
            } else if cu && resp.on_hover_text(tr("Click makes it the active layer: tracks go on it, drawn on top")).clicked() {
                activate = Some(l as LayerId);
            }
        };
        // On the board, hidden layers seldom shown wait under "more", below a line after the copper.
        let seldom = |l: usize, layer: &Layer| {
            board
                && !is_copper(l)
                && !layer.visible
                && !matches!(layer.name.as_str(), "F.SilkS" | "B.SilkS" | "Edge.Cuts" | "Vias" | "Holes")
        };
        if board {
            // How the active layer goes, at the dock's foot.
            egui::Panel::bottom("layers hint").frame(egui::Frame::new()).show(ui, |ui| {
                ui.add_space(8.0);
                let hint = tr("A click on a copper layer makes it active; PgUp, PgDn, + and − step through them");
                ui.add(egui::Label::new(RichText::new(hint).size(11.5).color(theme::QUIET)).wrap());
            });
        }
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let (shown, more): (Vec<usize>, Vec<usize>) = (0..scene.layers.len()).partition(|&l| !seldom(l, &scene.layers[l]));
            for (k, &l) in shown.iter().enumerate() {
                if k > 0 && is_copper(shown[k - 1]) && !is_copper(l) {
                    ui.separator();
                }
                row(ui, l, &mut scene.layers[l]);
            }
            if !more.is_empty() {
                let title = RichText::new(trf("{0} more, hidden", &[&more.len()])).color(theme::QUIET);
                egui::CollapsingHeader::new(title).id_salt("more layers").show(ui, |ui| {
                    for l in more {
                        row(ui, l, &mut scene.layers[l]);
                    }
                });
            }
        });
        if let Some(layer) = activate {
            self.switch_layer(layer);
        }
    }
}

/// The net of the biggest zone on copper layer `layer` (by its outline's box): an inner layer's
/// plane (`GND`); none without a zone there.
fn plane(b: &Board, layer: usize) -> Option<&str> {
    let area = |z: &&cuprita_core::board::Zone| {
        let pts = z.outline.first().map_or(&[][..], Vec::as_slice);
        let (x, y) = (pts.iter().map(|p| p.x as i64), pts.iter().map(|p| p.y as i64));
        (x.clone().max().unwrap_or(0) - x.min().unwrap_or(0)) * (y.clone().max().unwrap_or(0) - y.min().unwrap_or(0))
    };
    let biggest = b.zones.iter().filter(|z| !z.keepout && z.net != 0 && z.layers & 1 << layer != 0).max_by_key(area)?;
    b.nets.get(biggest.net as usize).map(String::as_str)
}

/// A small heading above a group in a dock.
pub(crate) fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).size(11.0).family(theme::strong()).color(theme::QUIET));
}
