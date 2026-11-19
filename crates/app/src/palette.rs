//! The palette (Ctrl+K, Ctrl+F): every command, every part and net of the view shown, and the
//! projects to open (the recent ones; on Home, KiCad's examples too), by a few letters of its
//! name; ↑↓ choose, Enter runs the command, shows the part or net or opens the project, Esc
//! closes it.

use crate::cmd::Cmd;
use crate::shell::Bench;
use crate::tr::{n, t, tr, trf};
use crate::{App, icon, theme};
use eframe::egui::{self, RichText, vec2};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// How many results the palette lists at most.
const SHOWN: usize = 12;

/// The palette's field.
const FIELD: &str = "palette";

/// The palette, open or not, with what is typed and the result chosen.
#[derive(Default)]
pub(crate) struct Palette {
    pub(crate) open: bool,
    query: String,
    pick: usize,
}

/// A result: a command, a part (its reference and value), a net, or a project to open and
/// what it is (a recent one, an example).
#[derive(Clone, Debug, PartialEq)]
enum Hit {
    Cmd(Cmd),
    Part(String, String),
    Net(String),
    Project(PathBuf, &'static str),
}

/// How well `text` matches `query`, best first (lower is better): a start, a word's start, a
/// part of it, else its letters in order; `None` when it doesn't match. Case is ignored.
fn score(query: &str, text: &str) -> Option<u32> {
    let (q, t) = (query.trim().to_lowercase(), text.to_lowercase());
    if q.is_empty() {
        return Some(3);
    }
    if t.starts_with(&q) {
        return Some(0);
    }
    if t.match_indices(&q).any(|(i, _)| t[..i].ends_with(|c: char| !c.is_alphanumeric())) {
        return Some(1);
    }
    if t.contains(&q) {
        return Some(2);
    }
    let mut letters = t.chars();
    q.chars().filter(|c| !c.is_whitespace()).all(|c| letters.any(|l| l == c)).then_some(4)
}

impl App {
    /// Opens the palette, empty and ready to type in, or closes it.
    pub(crate) fn toggle_palette(&mut self, ctx: &egui::Context) {
        self.palette = Palette { open: !self.palette.open, ..Default::default() };
        ctx.request_repaint();
    }

    /// What the palette lists for what is typed: commands that can run now, then the parts and
    /// nets of the view shown; best matches first.
    fn hits(&self) -> Vec<Hit> {
        let q = self.palette.query.as_str();
        let mut hits: Vec<(u32, usize, Hit)> = vec![];
        for (k, cmd) in Cmd::ALL.into_iter().enumerate().filter(|&(_, c)| self.can(c) && c != Cmd::Find) {
            // In the language shown, and in English too, as KiCad's docs and tutorials name it.
            let english = cmd.about_in(cfg!(target_arch = "wasm32")).name;
            if let Some(s) = [cmd.about().name, english].into_iter().filter_map(|name| score(q, name)).min() {
                hits.push((s, k, Hit::Cmd(cmd)));
            }
        }
        // Parts and nets come only once something is typed: they can be thousands.
        if !q.trim().is_empty() {
            let mut parts = vec![];
            if let (true, Some(sch)) = (self.sch_view(), &self.sch) {
                let refs: BTreeSet<(&str, &str)> = sch
                    .sheets
                    .project
                    .pages
                    .iter()
                    .flat_map(|p| &p.sheet.symbols)
                    .map(|s| (s.designator.as_str(), s.value.as_str()))
                    .collect();
                parts.extend(refs.into_iter().map(|(r, v)| (r.to_string(), v.to_string())));
            } else if self.scene.is_some() {
                parts.extend(self.board.footprints.iter().map(|f| (f.reference.clone(), f.value.clone())));
                for (k, net) in self.board.nets.iter().enumerate().skip(1) {
                    if let Some(s) = score(q, net).filter(|&s| s < 4) {
                        hits.push((s + 1, 1000 + k, Hit::Net(net.clone())));
                    }
                }
            }
            for (k, (r, v)) in parts.into_iter().enumerate() {
                if let Some(s) = score(q, &r).filter(|&s| s < 4) {
                    hits.push((s, 500 + k, Hit::Part(r, v)));
                }
            }
            let examples = if self.bench == Bench::Home { self.home.examples() } else { &[] };
            let projects =
                self.home.recent.iter().map(|p| (p, t!("recent project"))).chain(examples.iter().map(|p| (p, t!("KiCad example"))));
            for (k, (path, what)) in projects.enumerate() {
                let name = path.file_stem().unwrap_or_default().to_string_lossy();
                if let Some(s) = score(q, &name).filter(|&s| s < 4) {
                    hits.push((s, 2000 + k, Hit::Project(path.clone(), what)));
                }
            }
        }
        hits.sort_by_key(|(s, k, _)| (*s, *k));
        hits.into_iter().take(SHOWN).map(|(_, _, h)| h).collect()
    }

    /// The palette, when open, floating at the top of the window.
    pub(crate) fn palette(&mut self, ctx: &egui::Context) {
        if !self.palette.open {
            return;
        }
        let hits = self.hits();
        let key = |k| ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
        let step = key(egui::Key::ArrowDown) as isize - key(egui::Key::ArrowUp) as isize;
        self.palette.pick = self.palette.pick.saturating_add_signed(step).min(hits.len().saturating_sub(1));
        let (mut run, mut close) = (None, ctx.input(|i| i.key_pressed(egui::Key::Escape)));
        let screen = ctx.content_rect();
        let width = 620.0_f32.min(screen.width() - 32.0);
        let area = egui::Area::new(egui::Id::unique("palette")).order(egui::Order::Foreground).fade_in(false);
        let shown = area.fixed_pos(egui::pos2(screen.center().x, screen.top() + 64.0)).pivot(egui::Align2::CENTER_TOP).show(ctx, |ui| {
            let frame = egui::Frame::popup(ui.style()).fill(theme::PANEL).corner_radius(16).inner_margin(egui::Margin::same(8));
            frame.show(ui, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(icon::SEARCH).size(18.0).color(theme::QUIET));
                    let hint = if self.scene.is_some() || self.sch.is_some() {
                        tr("A command, a part or a net…")
                    } else {
                        tr("A command or a project…")
                    };
                    let field = egui::TextEdit::singleline(&mut self.palette.query)
                        .id(egui::Id::unique(FIELD))
                        .hint_text(hint)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::proportional(16.0))
                        .desired_width(f32::INFINITY);
                    // What is typed while it is open goes to it: asked before the field shows, as egui
                    // drops a focus given to a field in a new floating area.
                    ui.memory_mut(|m| m.request_focus(egui::Id::unique(FIELD)));
                    let field = ui.add(field);
                    if field.changed() {
                        self.palette.pick = 0;
                    }
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        run = hits.get(self.palette.pick).cloned();
                    }
                });
                ui.add_space(4.0);
                ui.separator();
                if hits.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(trf("Nothing is called “{0}”", &[&self.palette.query.trim()])).color(theme::QUIET));
                }
                for (k, hit) in hits.iter().enumerate() {
                    let (icon, name, what, keys) = match hit {
                        Hit::Cmd(c) => {
                            let about = c.about();
                            let keys = about.keys().next().map(|k| crate::cmd::key_text(ui.ctx(), &k)).unwrap_or_default();
                            (about.icon, about.name.to_string(), about.hint.to_string(), keys)
                        }
                        Hit::Part(r, v) => (icon::PART, r.clone(), trf("part · {0}", &[v]), String::new()),
                        // The word for one net, of its counted forms.
                        Hit::Net(name) => {
                            (icon::ROUTE, name.clone(), tr(n!("net")).split('|').next().unwrap_or_default().into(), String::new())
                        }
                        Hit::Project(p, what) => {
                            let name = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                            let folder = p.parent().map_or(String::new(), |d| d.display().to_string());
                            (icon::OPEN, name, format!("{} · {folder}", tr(what)), String::new())
                        }
                    };
                    let row = self.palette_row(ui, k == self.palette.pick, icon, &name, &what, &keys);
                    if row.clicked() {
                        run = Some(hit.clone());
                    } else if step != 0 && k == self.palette.pick {
                        row.scroll_to_me(None);
                    }
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    let tip = tr("↑↓ choose · Enter runs or shows · Esc closes");
                    ui.label(RichText::new(tip).small().color(theme::QUIET));
                });
            });
        });
        // A click elsewhere closes it, as a menu does.
        close |= ctx.input(|i| i.pointer.any_pressed()) && !shown.response.contains_pointer() && !shown.response.hovered();
        if close || run.is_some() {
            self.palette.open = false;
        }
        match run {
            Some(Hit::Cmd(c)) => self.run(ctx, c),
            Some(Hit::Part(r, _)) if self.sch_view() => {
                self.find_sch(&r);
            }
            Some(Hit::Part(name, _) | Hit::Net(name)) => self.find(&name),
            Some(Hit::Project(path, _)) => self.open_project(crate::opens(&path)),
            None => {}
        }
    }

    /// A palette row: an icon, a name, what it is, and its key on the right; lit when `picked`.
    fn palette_row(&self, ui: &mut egui::Ui, picked: bool, icon: &str, name: &str, what: &str, keys: &str) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), egui::Sense::click());
        let p = ui.painter();
        if picked || resp.hovered() {
            p.rect_filled(rect, 9.0, if picked { theme::tint(theme::PANEL, theme::COPPER, 0.16) } else { theme::RAISED });
        }
        let y = rect.center().y;
        p.text(
            egui::pos2(rect.left() + 20.0, y),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(17.0),
            if picked { theme::COPPER } else { theme::QUIET },
        );
        let name_at = egui::pos2(rect.left() + 40.0, y);
        let shown = p.text(name_at, egui::Align2::LEFT_CENTER, name, egui::FontId::new(13.5, theme::strong()), theme::TEXT);
        let right = if keys.is_empty() {
            rect.right() - 12.0
        } else {
            let k =
                p.text(egui::pos2(rect.right() - 12.0, y), egui::Align2::RIGHT_CENTER, keys, egui::FontId::monospace(11.5), theme::QUIET);
            k.left() - 12.0
        };
        let mut job =
            egui::text::LayoutJob::single_section(what.into(), egui::TextFormat::simple(egui::FontId::proportional(12.5), theme::QUIET));
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - shown.right() - 12.0).max(0.0));
        let what = ui.fonts_mut(|f| f.layout_job(job));
        p.galley(egui::pos2(shown.right() + 12.0, y - what.size().y / 2.0), what, theme::QUIET);
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_a_start_over_a_word_over_a_part_over_letters() {
        assert_eq!(score("rou", "Route"), Some(0));
        assert_eq!(score("lab", "Global label"), Some(1));
        assert_eq!(score("ute", "Route"), Some(2));
        assert_eq!(score("fz", "Fill zones"), Some(4));
        assert_eq!(score("zz", "Route"), None);
        assert_eq!(score("  ", "Route"), Some(3));
        assert_eq!(score("u2", "U24"), Some(0));
    }
}
