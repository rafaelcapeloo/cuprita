//! The 3D view (Alt+3): drag turns the board, a right or middle drag slides it, the wheel zooms
//! and F turns it over. It arrives tilting up from the plan, turns over in a sweep and turns on
//! a moment when let go (`motion.rs`). The renderer draws it into its own target before egui's
//! pass (`Paint3d`).

use crate::canvas::wheel_notches;
use crate::shell::Bench;
use crate::tr::{t, tr, trf};
use crate::{App, motion};
use cuprita_core::models;
use cuprita_render::{Orbit, Renderer};
use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use web_time::Instant;

/// Let go faster than this, in points a second, the board turns on, slowing by e in this many
/// seconds: a flick carries, a careful turn stays where it is put.
const SPIN_FROM: f32 = 300.0;
const SPIN_FOR: f32 = 0.09;

/// How to move about in the 3D view.
pub(crate) const HINT: &str =
    t!("Drag turns the board · right or middle drag slides it · wheel zooms · F turns it over · Esc: back to the board");

impl App {
    /// Shows the board in 3D, fitted to the canvas, or back in 2D.
    pub(crate) fn toggle_3d(&mut self) {
        let three = self.bench != Bench::ThreeD && self.scene.is_some();
        self.arrive(false);
        (self.bench, self.orbit) = (if three { Bench::ThreeD } else { Bench::Board }, None);
        if three {
            // The 2D tools and views stop; their keys bring the 2D view back.
            (self.show_diff, self.show_ir, self.measuring, self.routing, self.zoning) = (false, false, false, false, false);
            self.sync_gpu();
            self.load_models();
        }
    }

    /// Starts reading, on other threads, the board's 3D models not read yet. A browser has no
    /// files to read them from: a served view asks its server for them (`web::fetch_models`).
    pub(crate) fn load_models(&mut self) {
        let missing: Vec<String> = models::files(&self.board).into_iter().filter(|f| !self.models.contains_key(f)).collect();
        if missing.is_empty() || self.loading.is_some() {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if self.viewer {
            crate::web::fetch_models(missing);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = self.path.as_deref().and_then(std::path::Path::parent).map(std::path::Path::to_path_buf);
            self.loading = Some(std::thread::spawn(move || models::load(&missing, dir.as_deref())));
        }
    }

    /// How the 3D view's models are coming along, and why the first one missing is.
    pub(crate) fn models_text(&self) -> String {
        let files = models::files(&self.board);
        let errors: Vec<&str> = files.iter().filter_map(|f| self.models.get(f)?.as_ref().err()).map(String::as_str).collect();
        let shown = files.iter().filter(|f| matches!(self.models.get(*f), Some(Ok(_)))).count();
        match (self.loading.is_some(), files.len(), errors.first()) {
            (true, n, _) => trf("Reading {0} 3D models…", &[&n]),
            (_, 0, _) => tr("No 3D models").into(),
            (_, n, None) => trf("{0} of {1} 3D models", &[&shown, &n]),
            (_, n, Some(why)) => trf("{0} of {1} 3D models · {2} missing, first {3}", &[&shown, &n, &errors.len(), why]),
        }
    }

    /// Takes in the models read in the background once they all are, and shows them; until then
    /// the view comes back for them.
    pub(crate) fn poll_models(&mut self, ctx: &egui::Context) {
        if self.loading.as_ref().is_some_and(|h| h.is_finished()) {
            if let Some(Ok(read)) = self.loading.take().map(|h| h.join()) {
                self.models.extend(read);
            }
            self.sync_gpu();
        } else if self.loading.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    /// The board in 3D in `rect`, fitted, from the south and above (Make's picture of it).
    pub(crate) fn picture3d(&self, ui: &egui::Ui, rect: egui::Rect) {
        if let Some(scene) = &self.scene {
            paint3d(ui, rect, Orbit::fit(scene.bounds, [rect.width(), rect.height()]));
        }
    }

    pub(crate) fn canvas3d(&mut self, ui: &mut egui::Ui) {
        self.pointer = None;
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        self.poll_models(ui.ctx());
        let Some(scene) = &self.scene else { return };
        let fitted = self.orbit.is_none();
        let o = self.orbit.get_or_insert_with(|| Orbit::fit(scene.bounds, [rect.width(), rect.height()]));
        // Flipping turns the board over left to right, as in 2D; so does dragging it past edge-on.
        if self.flipped != (o.pitch < 0.0) {
            let from = *o;
            (o.yaw, o.pitch) = (o.yaw + 180.0, -o.pitch);
            self.turning = Some((from, Instant::now()));
        }
        // It arrives tilting up from the plan the board is drawn in.
        if fitted {
            self.turning = Some((Orbit { pitch: 90f32.copysign(o.pitch), ..*o }, Instant::now()));
        }
        let d = resp.drag_delta();
        if let Some(touch) = ui.input(|i| i.multi_touch()).filter(|t| rect.contains(t.center_pos)) {
            // Two fingers slide the board along and pinch it nearer; one turns it, as the mouse.
            o.pan(touch.translation_delta.x, touch.translation_delta.y, rect.height());
            o.zoom(touch.zoom_delta.ln() / 1.25f32.ln());
            self.spin = [0.0; 2];
        } else if resp.dragged_by(egui::PointerButton::Primary) {
            o.turn(d.x, d.y);
            self.spin = [0.0; 2];
        } else if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
            o.pan(d.x, d.y, rect.height());
        }
        // Let go while it turns, it turns on a moment, slowing.
        if resp.drag_stopped_by(egui::PointerButton::Primary) && !motion::reduced() {
            let v = ui.input(|i| i.pointer.velocity());
            self.spin = if v.length() > SPIN_FROM { [v.x, v.y] } else { [0.0; 2] };
        }
        if self.spin != [0.0; 2] {
            let dt = ui.input(|i| i.stable_dt).min(0.05);
            o.turn(self.spin[0] * dt, self.spin[1] * dt);
            self.spin = self.spin.map(|v| v * (-dt / SPIN_FOR).exp());
            if self.spin[0].hypot(self.spin[1]) < SPIN_FROM / 10.0 {
                self.spin = [0.0; 2];
            }
            ui.ctx().request_repaint();
        }
        if resp.hovered() {
            o.zoom(wheel_notches(ui));
        }
        if o.pitch != 0.0 {
            self.flipped = o.pitch < 0.0;
        }
        // On its way, the view is between the orbit it came from and the one it goes to.
        let shown = match self.turning {
            Some((from, start)) => {
                let t = motion::eased(ui.ctx(), start, motion::ARRIVE);
                if t >= 1.0 {
                    self.turning = None;
                }
                let between = |a: f32, b: f32| motion::lerp(a, b, t);
                Orbit { yaw: between(from.yaw, o.yaw), pitch: between(from.pitch, o.pitch), dist: between(from.dist, o.dist), ..*o }
            }
            None => *o,
        };
        paint3d(ui, rect, shown);
        self.fade_in(ui, rect);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.go(Bench::Board);
        }
    }
}

/// Draws the board in 3D, seen from `orbit`, into `rect`.
fn paint3d(ui: &egui::Ui, rect: egui::Rect, orbit: Orbit) {
    let ppp = ui.ctx().pixels_per_point();
    let size = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, Paint3d(orbit, size)));
}

/// Renders the 3D view from the orbit, this many pixels, then shows it in the canvas.
struct Paint3d(Orbit, [u32; 2]);

impl egui_wgpu::CallbackTrait for Paint3d {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(r) = resources.get_mut::<Renderer>() {
            r.render3d(device, queue, encoder, self.1, &self.0);
        }
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        if let Some(r) = resources.get::<Renderer>() {
            r.draw3d(pass);
        }
    }
}
