//! Motion (`BRAND.md`): a change shows as a move the eye can follow, then is over. No frame is
//! drawn for motion once it ends, and Settings' Reduced motion ends each move as it starts.

use crate::recent;
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use web_time::Instant;

/// How long a move takes, in seconds: what answers the hand (a tool taken, a tab, a workbench
/// shown), the view going somewhere (Find, Fit, a check shown), and what arrives (the 3D view
/// tilting up, a ping round what was found).
pub(crate) const QUICK: f32 = 0.12;
pub(crate) const GLIDE: f32 = 0.22;
pub(crate) const ARRIVE: f32 = 0.45;

/// Where a 2D view is: the scene point at its centre, and its scale.
pub(crate) type Place = ([f32; 2], f32);

static REDUCED: AtomicBool = AtomicBool::new(false);

/// Whether motion is reduced (Settings).
pub(crate) fn reduced() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

/// Takes the choice kept (`motion`) when the app starts; `still`: a run that pictures or times a
/// frame (`--screenshot`, `--bench`) shows it as it ends up.
pub(crate) fn read(ctx: &egui::Context, still: bool) {
    apply(ctx, still || recent::kept("motion").as_deref() == Some("reduced"));
}

/// Reduces motion or not, and keeps the choice for the next run.
pub(crate) fn set(ctx: &egui::Context, reduced: bool) {
    apply(ctx, reduced);
    recent::keep("motion", if reduced { "reduced" } else { "full" });
}

/// Ours, and egui's own moves too (its popups, its scroll bars).
fn apply(ctx: &egui::Context, reduced: bool) {
    REDUCED.store(reduced, Ordering::Relaxed);
    ctx.all_styles_mut(|s| s.animation_time = if reduced { 0.0 } else { QUICK });
}

/// How far a move `secs` long that started at `start` is, eased out: 0 to 1, and 1 once over
/// (from its start with motion reduced). While it runs it asks for the next frame.
pub(crate) fn eased(ctx: &egui::Context, start: Instant, secs: f32) -> f32 {
    let t = if reduced() { 1.0 } else { (start.elapsed().as_secs_f32() / secs).min(1.0) };
    if t < 1.0 {
        ctx.request_repaint();
    }
    1.0 - (1.0 - t).powi(3)
}

/// How far `on` is taken, 0 to 1: a quick move from where it was (a tool's wash, a hover).
pub(crate) fn on(ctx: &egui::Context, id: egui::Id, on: bool) -> f32 {
    ctx.animate_bool_with_time(id, on, if reduced() { 0.0 } else { QUICK })
}

/// `x` on its way to where it is now, a glide from where it was (a tab's underline).
pub(crate) fn toward(ctx: &egui::Context, id: egui::Id, x: f32) -> f32 {
    ctx.animate_value_with_time(id, x, if reduced() { 0.0 } else { GLIDE })
}

pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_move_eases_out_and_ends() {
        let ctx = egui::Context::default();
        let ago = |secs: f32| Instant::now() - Duration::from_secs_f32(secs);
        // Fast at first, slowing into its end: past half way at a third of its time.
        let third = eased(&ctx, ago(GLIDE / 3.0), GLIDE);
        assert!((0.6..0.8).contains(&third), "{third}");
        assert_eq!(eased(&ctx, ago(GLIDE * 2.0), GLIDE), 1.0);
        // Reduced, it is over as it starts.
        apply(&ctx, true);
        assert_eq!((eased(&ctx, Instant::now(), ARRIVE), ctx.global_style().animation_time), (1.0, 0.0));
        apply(&ctx, false);
        assert_eq!(lerp(2.0, 4.0, 0.25), 2.5);
    }
}
