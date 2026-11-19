//! The Make workbench: what an order still needs, each check with what does it; the fab files;
//! the board in the numbers a fab quotes by and where to order it; the board as the fab makes it,
//! in 3D; assembling by hand. Its cards sit three to a row when the window is wide.

use crate::cmd::Cmd;
use crate::dock::{Dock, Tab};
use crate::shell::Bench;
use crate::tr::{n, t, tr, trf};
use crate::units::{len, num};
use crate::{App, count, icon, theme};
use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, RichText, vec2};
use web_time::Instant;

/// How wide the page grows, and the room between its cards.
const PAGE: f32 = 1376.0;
const GAP: f32 = 20.0;

/// Fabs that take the fab files as they are: name, where an order starts, what they make from
/// which files.
const FABS: [(&str, &str, &str); 3] = [
    ("JLCPCB", "https://cart.jlcpcb.com/quote", t!("Boards and assembly: gerbers, BOM, positions")),
    ("PCBWay", "https://www.pcbway.com/QuickOrderOnline.aspx", t!("Boards and assembly: gerbers, BOM, positions")),
    ("OSH Park", "https://oshpark.com/", t!("Boards: the gerbers zip")),
];

/// A check before ordering: its colour (`PASS`, else how much it matters), what it says and
/// lists, the action that fixes it (and the part it shows there), and what to do, for the
/// page's summary.
pub(crate) struct Check {
    pub(crate) color: Color32,
    pub(crate) title: String,
    detail: String,
    act: (&'static str, Cmd),
    part: Option<String>,
    todo: String,
}

/// How far the board is behind its schematic, as F8 would bring it: worked out on a copy of the
/// board once the edits rest a second (`App::refresh_behind`).
#[derive(Default)]
pub(crate) struct Behind {
    /// How many changes of each kind (`n!` forms); `None` without a schematic.
    kinds: Option<[(usize, &'static str); 5]>,
    /// The times of the edits it was worked out after (the board's, the schematic's).
    seen: Option<(Option<Instant>, Option<Instant>)>,
}

impl Behind {
    /// How many changes, and what they are in the language shown ("2 values, 1 field"); `None`
    /// without a schematic.
    pub(crate) fn changes(&self) -> Option<(usize, String)> {
        let kinds = self.kinds?;
        let what: Vec<String> = kinds.iter().filter(|(n, _)| *n > 0).map(|&(n, kind)| count(n, kind)).collect();
        Some((kinds.iter().map(|(n, _)| n).sum(), what.join(", ")))
    }
}

/// The page's cards.
#[derive(Clone, Copy, Hash, Debug)]
enum Card {
    Checks,
    Files,
    Order,
    Look,
    Assemble,
}

impl App {
    /// The Make workbench: what is left and Export, then the cards.
    pub(crate) fn make_page(&mut self, ui: &mut egui::Ui) {
        if self.sch.is_none() && self.path.is_some() {
            self.board_sch(); // its checks count too
        }
        self.poll_models(ui.ctx());
        let checks = self.checks();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let width = (ui.available_width() - 64.0).min(PAGE);
            ui.add_space(28.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(width);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    self.summary(ui, &checks);
                    ui.add_space(24.0);
                    let (wide, third) = (width >= 960.0, (width - 2.0 * GAP) / 3.0);
                    self.cards(ui, &checks, wide, &[(Card::Checks, third), (Card::Files, third), (Card::Order, third)]);
                    ui.add_space(GAP);
                    self.cards(ui, &checks, wide, &[(Card::Look, 2.0 * third + GAP), (Card::Assemble, third)]);
                });
            });
            ui.add_space(32.0);
        });
    }

    /// Works out, once the edits rest a second, how far the board is behind its schematic: F8
    /// run on a copy (no library read: a part to add counts as missing). Renamed nets don't
    /// count: KiCad 7 on names a project's nets anew, the same connections.
    pub(crate) fn refresh_behind(&mut self, ctx: &egui::Context) {
        let (Some(sch), Some(_)) = (&self.sch, &self.scene) else { return self.behind = Behind::default() };
        let key = (self.changed, sch.changed);
        if self.behind.seen == Some(key) {
            return;
        }
        if let Some(last) = [key.0, key.1].into_iter().flatten().max() {
            let wait = std::time::Duration::from_secs(1).saturating_sub(last.elapsed());
            if !wait.is_zero() {
                return ctx.request_repaint_after(wait);
            }
        }
        if ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let (_, done) = self.board.clone().update(&sch.sheets.project, |_| Err(String::new()));
        let kinds = [
            (done.missing.len() + done.parts, n!("part to add|parts to add")),
            (done.footprints, n!("footprint")),
            (done.values, n!("value")),
            (done.fields, n!("field")),
            (done.moved, n!("pad on another net|pads on another net")),
        ];
        self.behind = Behind { kinds: Some(kinds), seen: Some(key) };
    }

    /// How many design rules the board breaks: its violations but KiCad's warnings (a loose
    /// end, silkscreen over a pad), which Checks lists in amber and no count holds against it.
    pub(crate) fn broken(&self) -> usize {
        self.violations.iter().filter(|v| !v.warning).count()
    }

    /// What an order still needs, in the order the work goes: the schematic, the outline, the
    /// routing, the rules, the zones, the part numbers.
    pub(crate) fn checks(&self) -> Vec<Check> {
        let b = &self.board;
        let pass = |ok: bool, need| if ok { theme::PASS } else { need };
        let mut out = vec![];
        if let Some(s) = &self.sch {
            let e = s.violations.iter().filter(|v| v.error).count();
            out.push(Check {
                color: pass(e == 0, theme::FAIL),
                title: if e == 0 { tr("Schematic checks pass").into() } else { trf("{0} in the schematic", &[&count(e, n!("error"))]) },
                detail: format!("{} · {}", count(e, n!("error")), count(s.violations.len() - e, n!("warning"))),
                act: (tr("Show"), Cmd::Schematic),
                part: None,
                todo: trf("fix the schematic's {0}", &[&count(e, n!("error"))]),
            });
        }
        if let Some((n, what)) = self.behind.changes() {
            out.push(Check {
                color: pass(n == 0, theme::WARN),
                title: if n == 0 {
                    tr("Board matches the schematic").into()
                } else {
                    trf("The board is {0} behind the schematic", &[&count(n, n!("change"))])
                },
                detail: if n == 0 { count(b.footprints.len(), n!("part")) } else { what },
                act: (tr("Update"), Cmd::Update),
                part: None,
                todo: tr("update the board from the schematic").into(),
            });
        }
        // An empty board has nothing to make (when the schematic has parts, its check above says F8).
        if b.footprints.is_empty() && self.behind.changes().is_none_or(|(n, _)| n == 0) {
            out.push(Check {
                color: theme::FAIL,
                title: tr("No parts on the board yet").into(),
                detail: tr("A adds them here; from the schematic, F8 brings them over").into(),
                act: (tr("Add"), Cmd::Add),
                part: None,
                todo: tr("put parts on the board").into(),
            });
        }
        let size = b.outline_size();
        out.push(Check {
            color: pass(size.is_some(), theme::FAIL),
            title: if size.is_some() { tr("Board outline closed") } else { tr("No closed board outline") }.into(),
            detail: size.map_or(tr("The fab can't cut the board without one").into(), |(w, h)| format!("{} × {}", num(w), len(h))),
            act: (tr("Draw"), Cmd::Outline),
            part: None,
            todo: tr("draw the board's outline").into(),
        });
        let left = self.ratsnest.len();
        out.push(Check {
            color: pass(left == 0, theme::FAIL),
            title: if left == 0 {
                tr("Every connection routed").into()
            } else {
                count(left, n!("connection not routed|connections not routed"))
            },
            detail: match left {
                0 => count(b.nets.len().saturating_sub(1), n!("net")),
                _ => few(self.ratsnest.iter().map(|l| b.nets[l.net as usize].as_str())),
            },
            act: (tr("Show"), Cmd::Checks),
            part: None,
            todo: if left == 1 { tr("route the last connection").into() } else { trf("route the last {0} connections", &[&left]) },
        });
        // Rules broken; KiCad's warnings (a loose end) don't keep a board from being made.
        let drc = self.broken();
        let kinds: Vec<String> = self.violations.iter().filter(|v| !v.warning).map(|v| rule_name(v.kind)).collect();
        out.push(Check {
            color: pass(drc == 0, theme::FAIL),
            title: if drc == 0 { tr("Design rules pass").into() } else { trf("{0} with the design rules", &[&count(drc, n!("problem"))]) },
            detail: match drc {
                0 => trf("Clearance {0}, tracks from {1}", &[&len(b.rules.clearance), &len(b.rules.track_width)]),
                _ => few(kinds.iter().map(String::as_str)),
            },
            act: (tr("Show"), Cmd::Checks),
            part: None,
            todo: trf("fix {0}", &[&count(drc, n!("design rule problem"))]),
        });
        let zones = b.zones.iter().filter(|z| !z.keepout).count();
        if zones > 0 {
            let unfilled = b.zones.iter().filter(|z| !z.keepout && z.fills.is_empty()).count();
            out.push(Check {
                color: pass(unfilled == 0, theme::FAIL),
                title: if unfilled == 0 { tr("Zones filled").into() } else { count(unfilled, n!("zone not filled|zones not filled")) },
                detail: count(zones, n!("zone")),
                act: (tr("Fill"), Cmd::Fill),
                part: None,
                todo: tr("fill the zones").into(),
            });
        }
        // A line of the BOM is a part to buy: assembly needs its MPN or LCSC number.
        let fp = &b.footprints;
        let bare: Vec<&str> = self
            .bom
            .iter()
            .filter(|l| fp[l[0]].part_numbers()[1..].iter().all(|n| n.is_empty()))
            .map(|l| fp[l[0]].reference.as_str())
            .collect();
        // Part numbers are typed in the schematic, on the part: show the first one lacking one.
        let (act, part) = match (&self.sch, bare.first()) {
            (Some(_), Some(r)) => ((tr("Fill in"), Cmd::Schematic), Some(r.to_string())),
            _ => ((tr("Show"), Cmd::Bom), None),
        };
        if !self.bom.is_empty() {
            out.push(Check {
                color: pass(bare.is_empty(), theme::WARN),
                title: match bare.len() {
                    0 => tr("Every part has a part number").into(),
                    n => trf("{0} without a part number", &[&count(n, n!("part"))]),
                },
                detail: match bare.len() {
                    0 => trf("{0} on the BOM", &[&count(self.bom.len(), n!("line"))]),
                    _ => trf("The assembler can't buy {0}", &[&few(bare.iter().copied())]),
                },
                act,
                part,
                todo: trf("give {0} a part number", &[&count(bare.len(), n!("part"))]),
            });
        }
        out
    }

    /// The checks passed as a ring, what is left in a sentence, and Export.
    fn summary(&mut self, ui: &mut egui::Ui, checks: &[Check]) {
        let left: Vec<&Check> = checks.iter().filter(|c| c.color != theme::PASS).collect();
        let (title, what) = match left.len() {
            0 => (tr("Ready to order"), tr("Every check passes: export the fab files, then order from a fab below.").to_string()),
            n => {
                let things = [
                    t!("One thing before you order"),
                    t!("Two things before you order"),
                    t!("Three things before you order"),
                    t!("Four things before you order"),
                    t!("Five things before you order"),
                    t!("Six things before you order"),
                ];
                let then = if self.board.outline_size().is_some() { tr(" You can export now; the fab makes what is drawn.") } else { "" };
                (tr(things[(n - 1).min(5)]), format!("{}.{then}", sentence(left.iter().map(|c| c.todo.as_str()))))
            }
        };
        ui.horizontal(|ui| {
            ring(ui, checks.len() - left.len(), checks.len(), 64.0);
            ui.add_space(12.0);
            // Export first, from the right: the sentence takes the room it leaves, in any language.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let key = Cmd::Fab.about().keys().next().map(|k| crate::cmd::key_text(ui.ctx(), &k)).unwrap_or_default();
                let text = RichText::new(format!("{}  {}   {key}", icon::EXPORT, tr("Export fab files")))
                    .color(theme::ON_COPPER)
                    .family(theme::strong());
                let button = egui::Button::new(text).fill(theme::COPPER).corner_radius(10).min_size(vec2(0.0, 42.0));
                if ui.add_enabled(self.can(Cmd::Fab), button).on_hover_text(self.cmd_tip(ui.ctx(), Cmd::Fab)).clicked() {
                    self.run(ui.ctx(), Cmd::Fab);
                }
                ui.add_space(24.0);
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 64.0), Layout::top_down(Align::Min), |ui| {
                    ui.label(RichText::new(title).family(theme::strong()).size(24.0).color(theme::TEXT));
                    ui.add(egui::Label::new(RichText::new(what).color(theme::TEXT_2)).wrap());
                });
            });
        });
    }

    /// A row of cards this wide side by side (else one above the other), each as tall as the
    /// tallest one's content was last frame.
    fn cards(&mut self, ui: &mut egui::Ui, checks: &[Check], wide: bool, row: &[(Card, f32)]) {
        let id = ui.scope_id().with(("make row", row[0].0));
        let tall: f32 = if wide { ui.data(|d| d.get_temp(id)).unwrap_or(0.0) } else { 0.0 };
        let mut most = 0.0f32;
        if wide {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for &(card, width) in row {
                    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                        most = most.max(self.card(ui, checks, card, tall));
                    });
                }
            });
        } else {
            for (k, &(card, _)) in row.iter().enumerate() {
                if k > 0 {
                    ui.add_space(GAP);
                }
                self.card(ui, checks, card, 0.0);
            }
        }
        ui.data_mut(|d| d.insert_temp(id, most));
    }

    /// A card titled for what it holds, at least `tall` high; returns its content's height.
    fn card(&mut self, ui: &mut egui::Ui, checks: &[Check], card: Card, tall: f32) -> f32 {
        let title = match card {
            Card::Checks => tr("Before you order"),
            Card::Files => tr("Files for the fab"),
            Card::Order => tr("Order"),
            Card::Look => tr("As the fab makes it"),
            Card::Assemble => tr("Assemble by hand"),
        };
        let frame = egui::Frame::new().fill(theme::PANEL).stroke((1.0, theme::LINE)).corner_radius(14).inner_margin(20);
        frame
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(tall);
                let top = ui.cursor().top();
                ui.label(RichText::new(title.to_uppercase()).size(11.0).family(theme::strong()).color(theme::QUIET));
                ui.add_space(6.0);
                match card {
                    Card::Checks => self.make_checks(ui, checks),
                    Card::Files => self.make_files(ui),
                    Card::Order => self.make_order(ui),
                    Card::Look => self.make_look(ui),
                    Card::Assemble => self.make_assemble(ui),
                }
                ui.cursor().top() - top
            })
            .inner
    }

    /// Each check: passed or what it needs, with what it lists, and the action that fixes it.
    fn make_checks(&mut self, ui: &mut egui::Ui, checks: &[Check]) {
        for (k, c) in checks.iter().enumerate() {
            if k > 0 {
                ui.separator();
            }
            let pass = c.color == theme::PASS;
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(if pass { icon::PASSES } else { icon::NEEDS }).color(c.color).size(18.0));
                ui.allocate_ui_with_layout(vec2(ui.available_width() - 48.0, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.add(egui::Label::new(RichText::new(&c.title).family(theme::strong()).color(theme::TEXT)).wrap());
                    ui.add(egui::Label::new(RichText::new(&c.detail).size(12.0).color(theme::QUIET)).wrap());
                });
                if !pass {
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if ui.link(RichText::new(c.act.0).family(theme::strong())).clicked() {
                            self.run(ui.ctx(), c.act.1);
                            if let Some(part) = &c.part {
                                self.find_sch(part);
                            } else if matches!(c.act.1, Cmd::Checks | Cmd::Schematic) {
                                self.right = Dock { tab: Tab::Checks, open: true };
                            }
                        }
                    });
                }
            });
        }
    }

    /// The fab files: what each holds, its size once written, the folder they go to.
    fn make_files(&mut self, ui: &mut egui::Ui) {
        let (b, stem) = (&self.board, self.name.trim_end_matches(".kicad_pcb"));
        let dir = self.path.as_ref().and_then(|p| p.parent()).map(|d| d.to_path_buf());
        let back = b.footprints.iter().filter(|f| f.back).count();
        let sides = match back {
            0 => tr("top only"),
            n if n == b.footprints.len() => tr("bottom only"),
            _ => tr("both sides"),
        };
        let mut files = vec![
            (icon::LAYERS, "-gerbers.zip", tr("Gerbers and drills"), count(b.copper.count_ones() as usize, n!("copper layer"))),
            (icon::BOM, "-bom.csv", tr("Bill of materials"), count(self.bom.len(), n!("line"))),
            (icon::ROUTE, "-positions.csv", tr("Placement"), format!("{}, {sides}", count(b.footprints.len(), n!("part")))),
        ];
        // The schematic's PDF (Ctrl+Shift+P), which the fab doesn't want but whoever assembles does.
        if let Some(sch) = self.sch.as_ref().filter(|_| cfg!(not(target_arch = "wasm32"))) {
            files.push((icon::FILE, ".pdf", tr("Schematic PDF"), count(sch.sheets.project.pages.len(), n!("sheet"))));
        }
        let mut write_pdf = false;
        for (k, (glyph, end, what, holds)) in files.into_iter().enumerate() {
            if k > 0 {
                ui.separator();
            }
            let size = dir.as_ref().and_then(|d| std::fs::metadata(d.join(format!("{stem}{end}"))).ok()).map(|m| m.len());
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph).color(theme::QUIET).size(16.0));
                // The size first, from the right: what the file holds shortens to the room left.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    match size {
                        None if end == ".pdf" => write_pdf = ui.link(RichText::new(tr("Write it")).family(theme::strong())).clicked(),
                        _ => {
                            let size = size.map_or(tr("not written yet").into(), |n| format!("{} KB", n.div_ceil(1024)));
                            ui.label(RichText::new(size).monospace().size(12.0).color(theme::QUIET));
                        }
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.label(RichText::new(what).color(theme::TEXT)).on_hover_text(format!("{stem}{end}"));
                        ui.add(egui::Label::new(RichText::new(format!("· {holds}")).color(theme::QUIET)).truncate());
                    });
                });
            });
        }
        if write_pdf {
            self.run(ui.ctx(), Cmd::Pdf);
        }
        ui.add_space(10.0);
        match dir {
            Some(dir) => {
                egui::Frame::new().fill(theme::GROUND).stroke((1.0, theme::LINE)).corner_radius(10).inner_margin(10).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::OPEN).color(theme::QUIET));
                        let show = ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let show = ui.link(RichText::new(tr("Show")).family(theme::strong())).clicked();
                            ui.add(egui::Label::new(RichText::new(dir.display().to_string()).monospace().size(12.0)).truncate());
                            show
                        });
                        if show.inner {
                            self.open_url(ui.ctx(), &dir.display().to_string());
                        }
                    });
                });
            }
            None if cfg!(target_arch = "wasm32") => _ = ui.label(RichText::new(tr("The browser downloads them.")).color(theme::QUIET)),
            None => _ = ui.label(RichText::new(tr("Next to the board, once the project has one.")).color(theme::QUIET)),
        }
        ui.add_space(8.0);
        let note = tr("Gerber X2 and Excellon, as KiCad writes them. Export fills the zones first and keeps files already there as -bak.");
        ui.add(egui::Label::new(RichText::new(note).size(12.0).color(theme::QUIET)).wrap());
    }

    /// The board in the numbers a fab's quote form asks for, and the fabs to order from.
    fn make_order(&mut self, ui: &mut egui::Ui) {
        let (b, row) = (&self.board, crate::inspect::row);
        let size = b.outline_size().map_or(tr("no closed outline").into(), |(w, h)| format!("{} × {}", num(w), len(h)));
        row(ui, tr("Size"), size);
        row(ui, tr("Copper layers"), b.copper.count_ones().to_string());
        row(ui, tr("Thickness"), len(b.thickness));
        if let Some(w) = b.tracks.iter().map(|t| t.width).min() {
            row(ui, tr("Smallest track / gap"), format!("{} / {}", num(w), len(b.rules.clearance)));
        }
        let drills = b.vias.iter().map(|v| v.drill).chain(b.pads.iter().filter_map(|p| p.drill.map(|(w, h)| w.min(h))));
        if let Some(d) = drills.min() {
            row(ui, tr("Smallest hole"), len(d));
        }
        // Plated holes' pads are on every copper layer, unplated ones on none.
        let unplated = b.pads.iter().filter(|p| p.drill.is_some() && p.layers & b.copper == 0).count();
        let plated = b.vias.len() + b.pads.iter().filter(|p| p.drill.is_some()).count() - unplated;
        row(ui, tr("Holes"), trf("{0} plated, {1} not", &[&plated, &unplated]));
        let back = b.footprints.iter().filter(|f| f.back).count();
        row(ui, tr("Parts"), trf("{0} on top, {1} below", &[&(b.footprints.len() - back), &back]));
        ui.add_space(10.0);
        ui.label(RichText::new(tr("Open a fab's order page and drop the zip:")).size(12.0).color(theme::QUIET));
        ui.add_space(4.0);
        let mut open = None;
        ui.columns(2, |cols| {
            for (k, (fab, url, what)) in FABS.into_iter().enumerate() {
                if fab_tile(&mut cols[k % 2], fab, url, tr(what)) {
                    open = Some(url);
                }
            }
        });
        if let Some(url) = open {
            self.open_url(ui.ctx(), url);
        }
    }

    /// The board in 3D, as the fab makes it: a click opens the 3D view.
    fn make_look(&mut self, ui: &mut egui::Ui) {
        if self.scene.is_none() {
            ui.label(RichText::new(tr("The board shows here once the project has one.")).color(theme::QUIET));
            return;
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 260.0), egui::Sense::click());
        self.picture3d(ui, rect);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.go(Bench::ThreeD);
        }
        let named = |c: &Option<String>, default: &str| match c.as_deref() {
            None => default.to_string(),
            Some(c) if c.starts_with('#') => tr("its own colour of").into(),
            Some(c) => c.to_lowercase(),
        };
        let b = &self.board;
        let look =
            trf("{0} solder mask, {1} silkscreen", &[&named(&b.mask_colors[0], tr("green")), &named(&b.silk_colors[0], tr("white"))]);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(look).color(theme::TEXT_2));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .link(RichText::new(tr("Open in 3D")).family(theme::strong()))
                    .on_hover_text(self.cmd_tip(ui.ctx(), Cmd::ThreeD))
                    .clicked()
                {
                    self.go(Bench::ThreeD);
                }
            });
        });
    }

    /// The first lines of the bench BOM, ticked as they are placed, and the checklist.
    fn make_assemble(&mut self, ui: &mut egui::Ui) {
        let fp = &self.board.footprints;
        let done = self.ticked.iter().filter(|&&t| t).count();
        ui.label(RichText::new(trf("{0} of {1} placed", &[&done, &count(self.bom.len(), n!("line"))])).color(theme::TEXT_2));
        ui.add_space(4.0);
        for (line, &ticked) in self.bom.iter().zip(&self.ticked).take(5) {
            let refs = few(line.iter().map(|&i| fp[i].reference.as_str()));
            let text = RichText::new(format!("{}× {} · {refs}", line.len(), fp[line[0]].value));
            ui.horizontal(|ui| {
                ui.label(RichText::new(if ticked { icon::PASSES } else { " " }).color(theme::PASS));
                ui.add(
                    egui::Label::new(if ticked { text.strikethrough().color(theme::QUIET) } else { text.color(theme::TEXT) }).truncate(),
                );
            });
        }
        if self.bom.len() > 5 {
            ui.label(RichText::new(trf("and {0}", &[&count(self.bom.len() - 5, n!("more line"))])).color(theme::QUIET));
        }
        ui.add_space(8.0);
        if ui.button(format!("{}  {}", icon::BOM, tr("BOM checklist"))).on_hover_text(self.cmd_tip(ui.ctx(), Cmd::Bom)).clicked() {
            self.run(ui.ctx(), Cmd::Bom);
        }
        ui.label(RichText::new(tr("A line lights its parts on the board; Space ticks it off.")).size(12.0).color(theme::QUIET));
    }
}

/// A fab's tile: its name, what it makes, a sign that it opens outside; `true` when clicked.
fn fab_tile(ui: &mut egui::Ui, fab: &str, url: &str, what: &str) -> bool {
    let frame =
        egui::Frame::new().fill(theme::GROUND).stroke((1.0, theme::LINE)).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 10));
    let tile = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        // The sign first, from the right: what the fab makes wraps in the room it leaves.
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.label(RichText::new(icon::EXTERNAL).color(theme::QUIET));
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 0.0), Layout::top_down(Align::Min), |ui| {
                ui.label(RichText::new(fab).family(theme::strong()).color(theme::TEXT));
                ui.add(egui::Label::new(RichText::new(what).size(11.5).color(theme::QUIET)).wrap());
            });
        });
    });
    let resp = tile.response.interact(egui::Sense::click()).on_hover_text(url).on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.hovered() {
        ui.painter().rect_stroke(resp.rect, 10.0, (1.0, theme::QUIET), egui::StrokeKind::Inside);
    }
    ui.add_space(8.0);
    resp.clicked()
}

/// `done` of `of` passed, as a ring `size` across round the count.
pub(crate) fn ring(ui: &mut egui::Ui, done: usize, of: usize, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    let (c, r, w, p) = (rect.center(), 0.42 * size, 0.11 * size, ui.painter());
    p.circle_stroke(c, r, (w, theme::RAISED));
    let color = if done == of { theme::PASS } else { theme::COPPER };
    let turn = std::f32::consts::TAU * done as f32 / of.max(1) as f32;
    let at = |a: f32| c + r * vec2((a - std::f32::consts::FRAC_PI_2).cos(), (a - std::f32::consts::FRAC_PI_2).sin());
    if done > 0 {
        p.add(egui::Shape::line((0..=64).map(|i| at(turn * i as f32 / 64.0)).collect(), (w, color)));
        // Round ends, as a stroke of a pen.
        p.circle_filled(at(0.0), w / 2.0, color);
        p.circle_filled(at(turn), w / 2.0, color);
    }
    p.text(c, Align2::CENTER_CENTER, format!("{done}/{of}"), FontId::new((0.23 * size).max(10.0), theme::strong()), theme::TEXT);
}

/// Nanometres as millimetres to `places` decimals.
/// The first four of `names`, each once, a dot between: "DA3 · CLKOUT · VAA · SDA …".
/// A design rule check's kind (`drc::Violation::kind`) as people read it.
fn rule_name(kind: &str) -> String {
    let name = match kind {
        "clearance" => t!("clearance"),
        "copper_edge_clearance" => t!("copper edge clearance"),
        "track_width" => t!("track width"),
        "via_diameter" => t!("via diameter"),
        "drill_out_of_range" => t!("drill out of range"),
        "annular_width" => t!("annular width"),
        "hole_to_hole" => t!("hole to hole"),
        other => return other.replace('_', " "),
    };
    tr(name).into()
}

fn few<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let mut seen: Vec<&str> = vec![];
    for n in names {
        if !seen.contains(&n) {
            if seen.len() == 4 {
                return seen.join(" · ") + " …";
            }
            seen.push(n);
        }
    }
    seen.join(" · ")
}

/// `parts` as one sentence: "Route the last 3 connections, fix 2 problems and fill the zones".
fn sentence<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    let parts: Vec<&str> = parts.collect();
    let text = match parts.split_last() {
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => trf("{0} and {1}", &[&rest.join(", "), last]),
        None => String::new(),
    };
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_what_is_left_in_a_sentence() {
        assert_eq!(sentence(["route the last connection"].into_iter()), "Route the last connection");
        let left = ["route the last 3 connections", "fix 2 design rule problems", "fill the zones"];
        assert_eq!(sentence(left.into_iter()), "Route the last 3 connections, fix 2 design rule problems and fill the zones");
        assert_eq!(few(["DA3", "DA3", "SDA"].into_iter()), "DA3 · SDA");
        assert_eq!(few(["A", "B", "C", "D", "E"].into_iter()), "A · B · C · D …");
        assert_eq!((count(1, n!("zone")), count(2, n!("zone"))), ("1 zone".to_string(), "2 zones".to_string()));
    }

    #[test]
    fn says_how_far_behind_its_schematic_the_board_is() {
        let behind = Behind { kinds: Some([(2, "value"), (0, "footprint"), (1, "field"), (0, "part"), (0, "pad")]), seen: None };
        assert_eq!(behind.changes(), Some((3, "2 values, 1 field".to_string())));
        assert_eq!(Behind::default().changes(), None);
    }
}
