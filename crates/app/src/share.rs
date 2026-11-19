//! Share (the header's button, Ctrl+K › Share): the project on a web page served from this
//! window (`serve.rs`), its address as a QR code for a phone or another computer on the network
//! to open, to look at. Desktop only: a page has no network to serve on.

use crate::tr::tr;
use crate::{App, icon, serve, theme};
use eframe::egui::{self, Align, Color32, Layout, RichText, vec2};
use std::path::PathBuf;

/// Where the header's Share button is, kept by the frame for the panel to open under it.
pub(crate) const BUTTON: &str = "share button";

/// The project being shared: its file, the server showing it (else why it can't), whether the
/// panel is open, and the QR code of its address, drawn once.
pub(crate) struct Share {
    file: PathBuf,
    server: Result<serve::Server, String>,
    open: bool,
    code: Option<egui::TextureHandle>,
}

/// The QR code of `text`, a pixel a module: dark on white inside the quiet zone a camera needs
/// (4 modules), whatever the window's colours. None for a text too long for a code.
fn qr(text: &str) -> Option<egui::ColorImage> {
    let code = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium).ok()?;
    let (n, quiet) = (code.size(), 4);
    let mut image = egui::ColorImage::filled([(n + 2 * quiet) as usize; 2], Color32::WHITE);
    for (x, y) in (0..n).flat_map(|y| (0..n).map(move |x| (x, y))).filter(|&(x, y)| code.get_module(x, y)) {
        image[((x + quiet) as usize, (y + quiet) as usize)] = Color32::BLACK;
    }
    Some(image)
}

impl App {
    /// The file of the project shown: its board's, else its root sheet's.
    fn project_file(&self) -> Option<PathBuf> {
        self.path.clone().or_else(|| self.sch.as_ref().map(|s| s.sheets.root.clone()))
    }

    /// Whether the project is being shown on its web page.
    pub(crate) fn sharing(&self) -> bool {
        self.share.as_ref().is_some_and(|s| s.server.is_ok())
    }

    /// Share: starts showing the project on its web page, on every address of this computer (a
    /// phone on its network reaches it) and port 8080 or a free one, and opens the panel with its
    /// address; while shared, shows or hides the panel. Stop, or another project, ends it.
    pub(crate) fn share(&mut self) {
        let Some(file) = self.project_file() else { return };
        match &mut self.share {
            Some(share) if share.file == file && share.server.is_ok() => share.open = !share.open,
            _ => self.share = Some(Share { server: serve::start(&file, "0.0.0.0", 8080, true), file, open: true, code: None }),
        }
    }

    /// The Share panel while open, under its button: the address as a QR code and in letters
    /// (Copy, Open), what the page shows, and Stop. Esc, its × or a click elsewhere closes it; the
    /// page stays up until Stop.
    pub(crate) fn share_panel(&mut self, ctx: &egui::Context) {
        if self.share.as_ref().is_some_and(|s| Some(&s.file) != self.project_file().as_ref()) {
            self.share = None;
        }
        let unsaved = self.unsaved();
        let Some(share) = self.share.as_mut().filter(|s| s.open) else { return };
        let button = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(BUTTON))).unwrap_or(egui::Rect::NOTHING);
        let (mut close, mut stop, mut open) = (ctx.input(|i| i.key_pressed(egui::Key::Escape)), false, None);
        let area = egui::Area::new(egui::Id::unique("share")).order(egui::Order::Foreground).fade_in(false);
        let shown = area.fixed_pos(button.center_bottom() + vec2(0.0, 10.0)).pivot(egui::Align2::CENTER_TOP).show(ctx, |ui| {
            let frame = egui::Frame::popup(ui.style()).fill(theme::PANEL).corner_radius(14).inner_margin(egui::Margin::same(18));
            frame.show(ui, |ui| {
                ui.set_width(292.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::SHARE).size(20.0).color(theme::COPPER));
                    ui.label(RichText::new(tr("Share")).size(17.0).family(theme::strong()).color(theme::TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let x = ui.add(egui::Button::new(RichText::new(icon::CLOSE).size(16.0)).frame(false));
                        close |= x.on_hover_text(tr("Close (Esc)")).clicked();
                    });
                });
                ui.add_space(10.0);
                let note = |ui: &mut egui::Ui, text: &str, color| ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).wrap());
                let server = match &share.server {
                    Ok(server) => server,
                    Err(why) => return drop(note(ui, why.lines().next().unwrap_or_default(), theme::WARN)),
                };
                let code = share.code.get_or_insert_with(|| {
                    let image = qr(&server.url).unwrap_or_else(|| egui::ColorImage::filled([1, 1], Color32::WHITE));
                    ctx.load_texture("share code", image, egui::TextureOptions::NEAREST)
                });
                ui.vertical_centered(|ui| ui.add(egui::Image::new(&*code).fit_to_exact_size(vec2(232.0, 232.0)).corner_radius(10)));
                ui.add_space(10.0);
                ui.add(egui::Label::new(RichText::new(&server.url).monospace().color(theme::TEXT)).wrap());
                ui.horizontal(|ui| {
                    if ui.button(format!("{}  {}", icon::COPY, tr("Copy"))).on_hover_text(tr("Copies the address")).clicked() {
                        ctx.copy_text(server.url.clone());
                    }
                    let here = ui.button(format!("{}  {}", icon::EXTERNAL, tr("Open"))).on_hover_text(tr("Opens the page in this computer's browser"));
                    if here.clicked() {
                        open = Some(server.url.clone());
                    }
                });
                ui.add_space(8.0);
                note(
                    ui,
                    tr("A phone or another computer on this network opens the page by this code or address: the schematic, the board and its 3D view, to look at. Nothing there changes the project."),
                    theme::QUIET,
                );
                if !server.reached {
                    note(ui, tr("This computer is on no network: only it can open the address."), theme::WARN);
                }
                if unsaved {
                    note(ui, tr("The page shows the files as saved: Ctrl+S takes your edits there."), theme::WARN);
                }
                ui.add_space(10.0);
                stop = ui.button(tr("Stop sharing")).on_hover_text(tr("Takes the page down: its address opens nothing after")).clicked();
            });
        });
        // A click elsewhere closes it; one on its button is the button's to take.
        let outside = |p: egui::Pos2| !shown.response.rect.contains(p) && !button.contains(p);
        close |= ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(outside));
        if let Some(url) = open {
            self.open_url(ctx, &url);
        }
        match (stop, close, &mut self.share) {
            (true, ..) => self.share = None,
            (_, true, Some(share)) => share.open = false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_an_address_as_a_code_a_camera_reads() {
        // An address with its key: a version 4 code (33 modules) at medium correction, in its quiet zone.
        let image = qr("http://192.168.100.123:8080/0123456789abcdef/").unwrap();
        assert_eq!(image.size, [41, 41]);
        let dark = |x: usize, y: usize| image[(x, y)] == Color32::BLACK;
        // The quiet zone is white all round; the three finder patterns sit in their corners, a
        // dark ring round a white one round a dark core, and none in the fourth.
        assert!((0..41).all(|k| !dark(k, 3) && !dark(3, k) && !dark(k, 37) && !dark(37, k)));
        for (x, y) in [(4, 4), (30, 4), (4, 30)] {
            assert!(dark(x, y) && !dark(x + 1, y + 1) && dark(x + 3, y + 3) && dark(x + 6, y + 6), "{x} {y}");
        }
        assert!(!(dark(30, 30) && !dark(31, 31) && dark(33, 33) && dark(36, 36)));
        // Nothing fits a code past 2,331 bytes at this correction.
        assert!(qr(&"x".repeat(3000)).is_none());
    }
}
