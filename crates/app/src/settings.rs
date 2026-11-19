//! Settings (Ctrl+, as KiCad's Preferences, or the gear on Home): the units lengths show in, the
//! language, and the user's own Digi-Key key, kept in Cuprita's data folder for the user alone
//! (the key in the environment wins). A panel over the window, as Help.

use crate::tr::{self, tr, trf};
use crate::{App, icon, theme, units};
use eframe::egui::{self, Align, Layout, RichText};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// What a lookup says without the user's key.
#[cfg(not(target_arch = "wasm32"))]
const NO_KEY: &str = crate::tr::t!(
    "Digi-Key looks part numbers up with your own API key, free: make an app with Product Information V4 \
    at developer.digikey.com, then paste its Client ID and Secret in Settings (Ctrl+,)"
);

/// The user's Digi-Key key: the environment's, else the one Settings keep; else how to get one
/// (in English: `tr` it to show it).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn digikey_key() -> Result<cuprita_core::digikey::Key, &'static str> {
    let kept = || Some(read_key(&key_file()?)).filter(|(id, secret)| !id.is_empty() && !secret.is_empty());
    cuprita_core::digikey::env_key().or_else(kept).ok_or(NO_KEY)
}

/// What Settings keep, and the key's fields as typed (the browser has no Digi-Key: it can't run
/// curl).
#[derive(Default)]
pub(crate) struct Settings {
    /// Whether the panel is open.
    pub(crate) open: bool,
    #[cfg(not(target_arch = "wasm32"))]
    id: String,
    #[cfg(not(target_arch = "wasm32"))]
    secret: String,
    /// Why the key typed last couldn't be kept.
    #[cfg(not(target_arch = "wasm32"))]
    error: Option<String>,
}

/// The file the key is kept in: Cuprita's data folder's `digikey`, the Client ID then the Secret.
#[cfg(not(target_arch = "wasm32"))]
fn key_file() -> Option<std::path::PathBuf> {
    crate::recent::dir().map(|d| d.join("digikey"))
}

/// The key kept in `file`, both its lines as typed (blank when none is kept).
#[cfg(not(target_arch = "wasm32"))]
fn read_key(file: &Path) -> (String, String) {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let mut lines = text.lines().map(|l| l.trim().to_string());
    (lines.next().unwrap_or_default(), lines.next().unwrap_or_default())
}

/// Keeps `id` and `secret` in `file`, which only the user can read; both blank, none is kept.
#[cfg(not(target_arch = "wasm32"))]
fn keep_key(file: &Path, id: &str, secret: &str) -> std::io::Result<()> {
    if id.trim().is_empty() && secret.trim().is_empty() {
        return match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut f = options.open(file)?;
    // The mode makes a new file private; one kept before is made so too.
    #[cfg(unix)]
    std::fs::set_permissions(file, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    std::io::Write::write_all(&mut f, format!("{}\n{}\n", id.trim(), secret.trim()).as_bytes())
}

impl Settings {
    /// Settings as last kept.
    pub(crate) fn read() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((id, secret)) = key_file().map(|f| read_key(&f)) {
            return Settings { id, secret, ..Settings::default() };
        }
        Settings::default()
    }
}

impl App {
    /// Settings while open: a panel over the window; Esc, Ctrl+, or a click outside closes it.
    pub(crate) fn settings(&mut self, ctx: &egui::Context) {
        if !self.settings.open {
            return;
        }
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut repage = false;
        let screen = ctx.content_rect();
        let width = 600.0_f32.min(screen.width() - 32.0);
        let area = egui::Area::new(egui::Id::unique("settings")).order(egui::Order::Foreground).fade_in(false);
        let shown = area.fixed_pos(egui::pos2(screen.center().x, screen.top() + 56.0)).pivot(egui::Align2::CENTER_TOP).show(ctx, |ui| {
            let frame = egui::Frame::popup(ui.style()).fill(theme::PANEL).corner_radius(14).inner_margin(egui::Margin::same(18));
            frame.show(ui, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::SETTINGS).size(20.0).color(theme::COPPER));
                    ui.label(RichText::new(tr("Settings")).size(17.0).family(theme::strong()).color(theme::TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let x = ui.add(egui::Button::new(RichText::new(icon::CLOSE).size(16.0)).frame(false));
                        close |= x.on_hover_text(tr("Close (Esc)")).clicked();
                    });
                });
                ui.add_space(14.0);
                let note =
                    |ui: &mut egui::Ui, text: &str| ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::QUIET)).wrap());
                egui::Grid::new(egui::Id::unique("settings rows")).num_columns(2).spacing([28.0, 18.0]).show(ui, |ui| {
                    let name = |ui: &mut egui::Ui, text: &str| {
                        ui.vertical(|ui| ui.add(egui::Label::new(RichText::new(text).family(theme::strong()).color(theme::TEXT)).extend()))
                    };
                    name(ui, tr("Units"));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            for (mils, name) in [(false, tr("Millimetres")), (true, tr("Mils"))] {
                                if ui.selectable_label(units::mils() == mils, name).clicked() && units::mils() != mils {
                                    units::toggle();
                                }
                            }
                        });
                        note(ui, tr("Every length shown and typed (Ctrl+U switches them); files keep KiCad's millimetres."));
                    });
                    ui.end_row();
                    name(ui, tr("Motion"));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            for (reduced, name) in [(false, tr("Full")), (true, tr("Reduced"))] {
                                if ui.selectable_label(crate::motion::reduced() == reduced, name).clicked() {
                                    crate::motion::set(ui.ctx(), reduced);
                                }
                            }
                        });
                        note(ui, tr("The view glides to what you find, and the 3D view tilts up and turns on when let go; Reduced shows each change at once."));
                    });
                    ui.end_row();
                    name(ui, tr("Sheets"));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            for (dark, name) in [(true, tr("Dark")), (false, tr("Light"))] {
                                if ui.selectable_label(theme::dark_sheet() == dark, name).clicked() && theme::dark_sheet() != dark {
                                    theme::set_dark_sheet(dark);
                                    repage = true;
                                }
                            }
                        });
                        note(ui, tr("The schematic's page in the window: dark as the board's, or light as KiCad's paper. A PDF is always light."));
                    });
                    ui.end_row();
                    name(ui, tr("Language"));
                    ui.vertical(|ui| {
                        let (chosen, system) = (tr::chosen().map(|l| l.tag), tr::system_language().name);
                        let as_system = trf("As the system: {0}", &[&system]);
                        let mut pick = chosen;
                        let shown = tr::LANGUAGES.iter().find(|l| Some(l.tag) == chosen).map_or(as_system.clone(), |l| l.name.to_string());
                        egui::ComboBox::from_id_salt("language").selected_text(shown).width(260.0).show_ui(ui, |ui| {
                            ui.selectable_value(&mut pick, None, as_system);
                            for l in &tr::LANGUAGES {
                                ui.selectable_value(&mut pick, Some(l.tag), l.name);
                            }
                        });
                        if pick != chosen {
                            tr::choose(tr::LANGUAGES.iter().find(|l| Some(l.tag) == pick));
                        }
                        note(ui, tr("The whole window, its checks and its guide; layer and field names stay as KiCad writes them (F.Cu, Value)."));
                    });
                    ui.end_row();
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        name(ui, tr("Digi-Key"));
                        let get = ui.vertical(|ui| {
                            if cuprita_core::digikey::env_key().is_some() {
                                note(
                                    ui,
                                    tr("Your key is in DIGIKEY_CLIENT_ID and DIGIKEY_CLIENT_SECRET: Add part finds parts by number with it."),
                                );
                                return false;
                            }
                            let s = &mut self.settings;
                            let id = ui.add(egui::TextEdit::singleline(&mut s.id).hint_text(tr("Client ID")).desired_width(360.0));
                            let secret =
                                egui::TextEdit::singleline(&mut s.secret).hint_text(tr("Client Secret")).password(true).desired_width(360.0);
                            let secret = ui.add(secret);
                            if id.changed() || secret.changed() {
                                let file = key_file().ok_or(tr("Couldn't keep the key: Cuprita has no data folder (set HOME)").to_string());
                                let kept = |f: std::path::PathBuf| {
                                    keep_key(&f, &s.id, &s.secret).map_err(|e| trf("Couldn't keep the key in {0}: {1}", &[&f.display(), &e]))
                                };
                                s.error = file.and_then(kept).err();
                            }
                            note(
                                ui,
                                tr("Your own key, free, finds a part by its number in Add part: make an app with Product Information V4 \
                                 at developer.digikey.com, then paste its Client ID and Secret here. Kept for you alone."),
                            );
                            if let Some(e) = &s.error {
                                ui.label(RichText::new(e).color(theme::FAIL));
                            }
                            ui.link("developer.digikey.com").clicked()
                        });
                        ui.end_row();
                        if get.inner {
                            self.open_url(ui.ctx(), "https://developer.digikey.com");
                        }
                        if let Some(dir) = crate::recent::dir() {
                            name(ui, tr("Kept in"));
                            let show = ui.horizontal(|ui| {
                                let tip = tr("Opens the folder: the recent projects, their pictures, these settings");
                                let shown = ui.button(tr("Show")).on_hover_text(tip).clicked();
                                // The home folder as ~, the rest cut short to fit.
                                let home = std::env::home_dir().filter(|h| dir.starts_with(h));
                                let path = home
                                    .map_or(dir.display().to_string(), |h| format!("~/{}", dir.strip_prefix(h).unwrap_or(&dir).display()));
                                ui.add(egui::Label::new(RichText::new(path).monospace().color(theme::TEXT_2)).truncate());
                                shown
                            });
                            ui.end_row();
                            if show.inner {
                                self.open_url(ui.ctx(), &dir.display().to_string());
                            }
                        }
                    }
                });
            });
        });
        close |= ctx.input(|i| i.pointer.any_pressed()) && !shown.response.contains_pointer();
        if close {
            self.settings.open = false;
        }
        // The sheet shown takes its page's new colours at once.
        if repage {
            self.redraw_sch();
            self.sync_gpu();
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_key_for_the_user_alone_and_forgets_it_blank() {
        let dir = std::env::temp_dir().join(format!("cuprita-settings-{}", std::process::id()));
        let file = dir.join("digikey");
        keep_key(&file, " id1 ", "secret1\n").unwrap();
        assert_eq!(read_key(&file), ("id1".to_string(), "secret1".to_string()));
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&file).unwrap().permissions()) & 0o777, 0o600);
        keep_key(&file, "", " ").unwrap();
        assert!(!file.exists());
        assert_eq!(read_key(&file), (String::new(), String::new()));
        keep_key(&file, "", "").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
