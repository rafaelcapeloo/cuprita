//! Pair tool (`6`): routes a differential pair on the active side of its pads. A click on a pad,
//! via or track end of one of its nets takes that and its other net's beside it; the two tracks
//! follow the pointer side by side, as wide and as far apart as their class says, round other
//! nets' copper (`cuprita_core::pair`); a click fixes a corner, on the pair's other pads it ends
//! there; Enter ends it in the open, Backspace takes a corner back, Esc drops it.

use crate::tr::{t, tr, trf};
use crate::{App, units};
use cuprita_core::board::{Nm, Pt};
use cuprita_core::pair::{Laid, PairRouter};
use eframe::egui;
use web_time::Instant; // std's panics in the browser

/// A pair's route being drawn: its router, and what it would lay as far as the pointer.
pub(crate) struct PairRoute {
    router: PairRouter,
    laid: Option<Laid>,
}

/// The pair tool's keys and clicks.
pub(crate) const HINT: &str =
    t!("Click a pad of a differential pair · then its corners · its other pads end it · Enter ends it here · Esc drops it");
/// Every key of it, in full: the hint's hover.
pub(crate) const KEYS: &str = t!(
    "Click a pad, via or track end of a differential pair (nets named as D+ and D-, or CLK_P and CLK_N): its two tracks follow the pointer side by side, as wide and as far apart as their net class says, round other nets · a click fixes a corner, on the pair's other pads ends it there · Enter or double click ends it at the pointer · Backspace takes the last corner back · Esc drops it, again leaves the tool"
);

impl App {
    /// A pair tool click at `at`: starts a route on the pair's ends there (saying why not), fixes
    /// a corner of the one being drawn, or, on the pair's other ends, lays it.
    pub(crate) fn pair_click(&mut self, at: Pt, tol: Nm) {
        let Some(r) = &mut self.pair else {
            return match self.board.pair_ends(at, tol, &self.shown_copper()) {
                Ok(from) => {
                    (self.active, self.net, self.status) = (from.layer, from.pair.nets[0], String::new());
                    self.pair = Some(PairRoute { router: PairRouter::new(&self.board, from, at), laid: None });
                }
                Err(why) => self.status = why,
            };
        };
        match r.laid.clone() {
            Some(laid) if laid.to.is_some() => self.pair_finish(),
            Some(laid) if !r.router.fix(&laid) => self.status = laid.refused.unwrap_or_default(),
            _ => {}
        }
    }

    /// The pair being routed follows the pointer to `at`.
    pub(crate) fn pair_to(&mut self, at: Pt, tol: Nm) {
        if let Some(r) = &mut self.pair {
            r.laid = Some(r.router.to(&self.board, at, tol));
        }
    }

    /// Lays the pair as drawn (Enter, a double click, a click on its other pads): its tracks, one
    /// undo step; refused, it says why and stays drawn.
    pub(crate) fn pair_finish(&mut self) {
        let Some(PairRoute { router, laid: Some(laid) }) = self.pair.take() else { return };
        let (t, nets) = (Instant::now(), router.pair().nets.map(|n| self.board.nets[n as usize].clone()));
        match self.board.add_pair(&router.from, &laid) {
            Ok(undo) => {
                let [p, n] = laid.lengths().map(units::len);
                self.edited(undo, t, trf("Pair {0} and {1} routed: {2} and {3} long", &[&nets[0], &nets[1], &p, &n]));
            }
            Err(why) => (self.status, self.pair) = (why, Some(PairRoute { router, laid: Some(laid) })),
        }
    }

    /// Takes back the last corner of the pair being routed (Backspace); with none, the route.
    pub(crate) fn pair_back(&mut self) {
        if self.pair.as_mut().is_some_and(|r| !r.router.back()) {
            self.pair = None;
        }
    }

    /// The pair being routed: its two tracks as wide as they would be, in a light tint of their
    /// layer's colour, red where it can't be laid so; its middle line faint.
    pub(crate) fn draw_pair(&self, ui: &egui::Ui, screen: impl Fn(Pt) -> egui::Pos2) {
        let (Some(PairRoute { router, laid: Some(laid) }), Some(scene)) = (&self.pair, &self.scene) else { return };
        let tint = crate::color32(scene.layers[router.from.layer as usize].color).lerp_to_gamma(egui::Color32::WHITE, 0.4);
        let colour = if laid.refused.is_some() { crate::theme::FAIL } else { tint };
        let width = (router.pair().width as f32 / 1000.0 / self.um_per_pt).max(1.0);
        let line = |pts: &[Pt]| pts.iter().map(|&p| screen(p)).collect::<Vec<_>>();
        ui.painter().add(egui::Shape::line(line(&laid.middle), (1.0, colour.gamma_multiply(0.35))));
        for side in &laid.sides {
            ui.painter().add(egui::Shape::line(line(side), (width, colour.gamma_multiply(0.85))));
        }
    }

    /// The pair being routed (its nets, width and gap, how long each side is, why it can't be
    /// laid) or how to start one, for the status bar.
    pub(crate) fn pair_text(&self) -> String {
        let Some(r) = &self.pair else { return tr("Pair: click a pad, via or track end of a differential pair").into() };
        let (pair, b) = (r.router.pair(), &self.board);
        let nets = pair.nets.map(|n| b.nets[n as usize].as_str());
        let sizes = trf("{0} wide, {1} apart", &[&units::len(pair.width), &units::len(pair.gap)]);
        match &r.laid {
            Some(Laid { refused: Some(why), .. }) => why.clone(),
            Some(laid) => {
                let [p, n] = laid.lengths().map(units::len);
                trf("Pair {0} and {1}: {2} · {3} and {4} long", &[&nets[0], &nets[1], &sizes, &p, &n])
            }
            None => trf("Pair {0} and {1}: {2}", &[&nets[0], &nets[1], &sizes]),
        }
    }
}
