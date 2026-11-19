//! Help (F1, or the ? on Home): the guide (`docs.rs`), and every command with its keys and what
//! it does, by where it works, filtered as you type; the walk-through to a first board and First
//! steps; what Cuprita opens.

use crate::cmd::Cmd::{self, *};
use crate::tr::{t, tr, trf};
use crate::{App, icon, theme};
use eframe::egui::{self, Align, Layout, RichText};

/// Every command once, by where it works, in the order Help lists them.
const GROUPS: [(&str, &[Cmd]); 6] = [
    (t!("Anywhere"), &[Find, Help, Settings, Home, New, Open, Save, Undo, Redo, Units, Ask, Tour, Guide]),
    (t!("Workbenches"), &[Schematic, Board, ThreeD, Make, Update, Fab, Pdf, Share]),
    (t!("Board and schematic"), &[Select, Add, ImportPart, NewPart, Checks, Turn, Delete, Fit]),
    (
        t!("Board"),
        &[
            Route, Pair, Zone, Outline, Cutout, Line, Text, Measure, Fill, TurnBack, FlipPart, TidyTexts, Run, NudgeLeft, NudgeRight,
            NudgeUp, NudgeDown, Flip, GridNext, GridBack, Changes, Heat, Bom,
        ],
    ),
    (t!("While routing"), &[TopLayer, BottomLayer, NextLayer, PrevLayer, WiderTrack, NarrowerTrack, SwapBend, Shove, BlindVias]),
    (
        t!("Schematic"),
        &[Wire, Bus, BusEntry, Label, GlobalLabel, HierLabel, Sheet, NoConnect, MirrorX, MirrorY, Copy, Paste, Duplicate, Simulate],
    ),
];

/// The filter field's id.
const FIELD: &str = "help filter";

/// Help while it is open: what is typed in its field, and the guide's page shown (else the
/// commands).
#[derive(Default)]
pub(crate) struct Help {
    filter: String,
    page: Option<&'static str>,
}

impl Help {
    /// Help as it opens: the guide on Home, where one starts, else the commands and their keys.
    pub(crate) fn new(home: bool) -> Self {
        Help { filter: String::new(), page: home.then_some(crate::docs::INDEX) }
    }
}

/// Whether `cmd`, its keys shown as `keys`, has every word of `filter` in its name, keys or what
/// it does.
fn matches(cmd: Cmd, keys: &str, filter: &str) -> bool {
    let about = cmd.about();
    // In the language shown, and in English too.
    let english = cmd.about_in(cfg!(target_arch = "wasm32"));
    let text = format!("{} {keys} {} {} {}", about.name, about.hint, english.name, english.hint).to_lowercase();
    filter.to_lowercase().split_whitespace().all(|w| text.contains(w))
}

impl App {
    /// Help while it is open (`App::help`): a panel over the window; Esc, F1 or a click outside
    /// closes it.
    pub(crate) fn help(&mut self, ctx: &egui::Context) {
        let Some(mut help) = self.help.take() else { return };
        let (mut run, mut close) = (None, ctx.input(|i| i.key_pressed(egui::Key::Escape)));
        let screen = ctx.content_rect();
        let width = 720.0_f32.min(screen.width() - 32.0);
        let area = egui::Area::new(egui::Id::unique("help")).order(egui::Order::Foreground).fade_in(false);
        let shown = area.fixed_pos(egui::pos2(screen.center().x, screen.top() + 56.0)).pivot(egui::Align2::CENTER_TOP).show(ctx, |ui| {
            let frame = egui::Frame::popup(ui.style()).fill(theme::PANEL).corner_radius(14).inner_margin(egui::Margin::same(18));
            frame.show(ui, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::HELP).size(20.0).color(theme::COPPER));
                    ui.label(RichText::new(tr("Help")).size(17.0).family(theme::strong()).color(theme::TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let x = ui.add(egui::Button::new(RichText::new(icon::CLOSE).size(16.0)).frame(false));
                        close |= x.on_hover_text(tr("Close (Esc)")).clicked();
                    });
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for cmd in [Tour, Guide] {
                        let about = cmd.about();
                        let button = egui::Button::new(format!("{}  {}", about.icon, about.name)).min_size(egui::vec2(0.0, 30.0));
                        if ui.add_enabled(self.can(cmd), button).on_hover_text(about.hint).clicked() {
                            run = Some(cmd);
                        }
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let guide = theme::dock_tab(ui, help.page.is_some(), icon::GUIDE, tr("Guide"), None);
                    if guide.clicked() && help.page.is_none() {
                        help.page = Some(crate::docs::INDEX);
                    }
                    if theme::dock_tab(ui, help.page.is_none(), icon::KEYS, tr("Commands and keys"), None).clicked() {
                        help.page = None;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::SEARCH).size(16.0).color(theme::QUIET));
                    let field = egui::TextEdit::singleline(&mut help.filter)
                        .id(egui::Id::unique(FIELD))
                        .hint_text(tr("Find a command: a word of what it does, or its key"))
                        .desired_width(f32::INFINITY);
                    // Typing filters the commands: asked before the field shows, as egui drops a
                    // focus given to a field in a new floating area.
                    ui.memory_mut(|m| m.request_focus(egui::Id::unique(FIELD)));
                    if ui.add(field).changed() {
                        help.page = None;
                    }
                });
                ui.add_space(6.0);
                ui.separator();
                let height = (screen.height() - 300.0).max(160.0);
                if let Some(page) = help.page {
                    let text = crate::docs::pages().iter().find(|p| p.0 == page).map_or("", |p| p.1);
                    egui::ScrollArea::vertical().id_salt(page).max_height(height).auto_shrink([false, true]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        if page != crate::docs::INDEX && ui.link(tr("‹ Guide")).clicked() {
                            help.page = Some(crate::docs::INDEX);
                        }
                        // Clear of the scroll bar.
                        let page = egui::Frame::NONE.inner_margin(egui::Margin { right: 14, ..Default::default() });
                        if let Some(to) = page.show(ui, |ui| crate::docs::show(ui, text)).inner {
                            help.page = Some(to);
                        }
                    });
                } else {
                let filter = &help.filter;
                egui::ScrollArea::vertical().max_height(height).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let mut any = false;
                    for (group, cmds) in GROUPS {
                        let keys = |c: Cmd| c.about().keys().map(|k| crate::cmd::key_text(ctx, &k)).collect::<Vec<_>>().join(tr(" or "));
                        let rows: Vec<(Cmd, String)> = cmds.iter().map(|&c| (c, keys(c))).filter(|(c, k)| matches(*c, k, filter)).collect();
                        if rows.is_empty() {
                            continue;
                        }
                        any = true;
                        ui.add_space(10.0);
                        ui.label(RichText::new(tr(group)).size(12.0).family(theme::strong()).color(theme::QUIET));
                        for (cmd, keys) in rows {
                            let about = cmd.about();
                            ui.add_space(4.0);
                            let row = ui.horizontal(|ui| {
                                ui.add_sized([20.0, 18.0], egui::Label::new(RichText::new(about.icon).size(15.0).color(theme::QUIET)));
                                ui.label(RichText::new(about.name).color(theme::TEXT));
                            });
                            // Clear of the scroll bar.
                            let (right, y) = (ui.max_rect().right() - 14.0, row.response.rect.center().y);
                            let font = egui::FontId::monospace(12.0);
                            ui.painter().text(egui::pos2(right, y), egui::Align2::RIGHT_CENTER, keys, font, theme::TEXT_2);
                            egui::Frame::NONE.inner_margin(egui::Margin { left: 28, right: 14, ..Default::default() }).show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(about.hint).size(12.0).color(theme::QUIET)).wrap());
                            });
                        }
                    }
                    if !any {
                        ui.add_space(10.0);
                        ui.label(RichText::new(trf("No command has “{0}”", &[&filter.trim()])).color(theme::QUIET));
                    }
                });
                }
                ui.separator();
                let about = trf(
                    "Cuprita {0} · GPL-3.0-or-later · opens KiCad 4–10 boards and 6–10 schematics, and saves them as they were but for your edits",
                    &[&env!("CARGO_PKG_VERSION")],
                );
                ui.add(egui::Label::new(RichText::new(about).size(11.5).color(theme::QUIET)).wrap());
            });
        });
        // A click outside closes it, as it does the palette.
        close |= ctx.input(|i| i.pointer.any_pressed()) && !shown.response.contains_pointer();
        if !close && run.is_none() {
            self.help = Some(help);
        }
        if let Some(cmd) = run {
            self.run(ctx, cmd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_every_command_once() {
        let listed: Vec<Cmd> = GROUPS.iter().flat_map(|(_, cmds)| cmds.iter().copied()).collect();
        for c in Cmd::ALL {
            assert_eq!(listed.iter().filter(|&&l| l == c).count(), 1, "{c:?}");
        }
        assert_eq!(listed.len(), Cmd::ALL.len());
    }

    #[test]
    fn filters_by_every_word_of_name_keys_and_hint() {
        assert!(matches(Route, "X", "route"));
        assert!(matches(Route, "X", "x"));
        assert!(matches(Route, "X", "track CLEARANCES"));
        assert!(!matches(Route, "X", "track wire"));
        assert!(matches(Save, "Ctrl+S", "ctrl+s"));
        assert!(matches(Save, "Ctrl+S", "  "));
    }
}
