//! Route tool (`X`): click a pad, via or track to start a track, which follows the pointer while
//! other nets' tracks and vias make way for it (P: it walks round them instead). A click fixes it
//! up to the pointer, with what made way; another active layer (PgUp, PgDn, +, -) drops a via
//! there and goes on on that layer, V on the other side; W and Shift+W step its width, `/` swaps
//! its bend; Enter or a double click finishes it, Esc drops the part not fixed yet.

use crate::tr::{n, t, tr, trf};
use crate::{App, count, units};
use cuprita_core::board::{Board, LayerId, Nm, Pt};
use cuprita_core::route::{Router, Start, tracks};
use cuprita_core::shove::{Shoved, Shover, Stuck};
use eframe::egui;
use web_time::Instant; // std's panics in the browser

/// A track being routed: its net, width, layer and last fixed point (`start.at`), the router
/// and shover for that layer with its cells on the board's `grid`, where the pointer aims, the
/// path there and what makes way for it (or why nothing could); `note` says why the last change
/// asked for didn't happen, until the track moves on.
pub(crate) struct Route {
    pub(crate) start: Start,
    router: Router,
    shover: Shover,
    grid: (Pt, Nm),
    aim: Pt,
    path: Vec<Pt>,
    shoved: Shoved,
    stuck: Option<Stuck>,
    note: String,
}

impl Route {
    /// A route from `start` on the board's `grid`, its bends' diagonal first or last (`/` swaps it).
    fn new(board: &Board, start: Start, grid: (Pt, Nm), diagonal_first: bool) -> Self {
        let mut router = Router::on_grid(board, start.net, start.layer, start.width, start.at, grid);
        router.diagonal_first = diagonal_first;
        let (shover, path) = (Shover::new(board, start.net), vec![start.at]);
        Self { start, router, shover, grid, aim: start.at, path, shoved: Shoved::default(), stuck: None, note: String::new() }
    }

    /// The path to the aim again: with `shove`, through other nets' tracks and vias that make way
    /// for it, round everything where they can't; else round everything.
    fn find(&mut self, board: &Board, shove: bool) {
        let (s, ends) = (self.start, (self.start.at, self.aim));
        (self.path, self.shoved, self.stuck) = match shove {
            true => match self.shover.step(board, &mut self.router, ends, s.width, s.layer) {
                (path, Ok(shoved)) => (path, shoved, None),
                (path, Err(stuck)) => (path, Shoved::default(), Some(stuck)),
            },
            false => (self.router.path(ends.0, ends.1), Shoved::default(), None),
        };
    }

    /// Where the pointer aims and whether a click there joins the net's copper; the path's length
    /// in mm and its last segment.
    pub(crate) fn aim(&self) -> (Pt, bool, f64, (Pt, Pt)) {
        let length = self.path.windows(2).map(|w| crate::helpers::span(w[0], w[1]).0).sum();
        let last = match self.path[..] {
            [.., a, b] => (a, b),
            _ => (self.start.at, self.aim),
        };
        (self.aim, self.aim != self.start.at && self.router.connects(self.aim), length, last)
    }
}

impl App {
    /// A route tool click at `at`: fixes the track being routed up to the pointer, or starts one
    /// on the pad, via or track there, on its layer, which becomes the active one.
    pub(crate) fn route_click(&mut self, at: Pt, tol: Nm) {
        if self.route.is_some() {
            return self.route_fix(false);
        }
        match self.board.route_start(at, tol, &self.shown_copper()) {
            Some(s) => {
                (self.net, self.active, self.status) = (s.net, s.layer, String::new());
                self.route = Some(Route::new(&self.board, s, self.board_grid(), false));
            }
            None => self.status = tr("Click a pad, via or track to start a track there").into(),
        }
    }

    /// Points the track being routed at `at`: at its net's pad, via or track end there, else at
    /// the grid point (`free`, Ctrl held: the router's own, finer); the path follows when the
    /// aim changes. After N steps the grid, the router's cells go onto the new one first.
    pub(crate) fn route_to(&mut self, at: Pt, tol: Nm, free: bool) {
        let grid = self.board_grid();
        let Some(r) = &mut self.route else { return };
        if r.grid != grid {
            *r = Route::new(&self.board, r.start, grid, r.router.diagonal_first);
        }
        let aim = r.router.snap(at, tol);
        let aim = if free || r.router.connects(aim) { aim } else { crate::grid::snap(at, grid) };
        if aim != r.aim {
            r.aim = aim;
            r.find(&self.board, self.shove);
        }
    }

    /// Adds the path up to the pointer to the board, with what made way for it. The track ends
    /// there when `finish` is set or the path reached its own net's pad, via or track end; else it
    /// goes on from there, among the other nets as they are now.
    pub(crate) fn route_fix(&mut self, finish: bool) {
        let Some(r) = &mut self.route else { return };
        let (t, s, end) = (Instant::now(), r.start, *r.path.last().expect("a path holds its start"));
        let new = tracks(&r.path, s.width, s.layer, s.net);
        let (what, moved) = (fixed(new.len(), false, &r.shoved), r.shoved != Shoved::default());
        let Ok(undo) = self.board.add_route(&r.shoved, new, None) else { unreachable!("only a via is refused") };
        if finish || (end == r.aim && end != s.at && r.router.connects(end)) {
            self.route = None;
        } else if moved {
            // Other nets moved: the router learns them again.
            *r = Route::new(&self.board, Start { at: end, ..s }, r.grid, r.router.diagonal_first);
        } else {
            // The new tracks are the route's own, which other nets keep clear of.
            (r.start.at, r.path, r.stuck, r.note) = (end, vec![end], None, String::new());
            r.shover = Shover::new(&self.board, s.net);
        }
        if let Some(what) = what {
            self.edited(undo, t, what);
        }
    }

    /// V: the track goes on on the other outer layer (from an inner one, the top) from a via.
    pub(crate) fn route_via(&mut self) {
        let Some(r) = &self.route else { return };
        let (top, bottom) = (self.board.stackup.first().map(|c| c.layer), self.board.stackup.last().map(|c| c.layer));
        if let Some(layer) = if Some(r.start.layer) == top { bottom } else { top } {
            self.switch_layer(layer);
        }
    }

    /// Makes copper `layer` the active one, shown. While a track is routed, it is fixed up to the
    /// pointer, with what made way for it and for a via dropped there (none where a pad or via
    /// already joins the layers), and goes on on `layer` from there; where the via has no room,
    /// it says why and nothing changes.
    pub(crate) fn switch_layer(&mut self, layer: LayerId) {
        if let Some(l) = self.scene.as_mut().and_then(|s| s.layers.get_mut(layer as usize)) {
            l.visible = true;
        }
        let Some(r) = self.route.as_ref().filter(|r| r.start.layer != layer) else { return self.active = layer };
        let (t, s, at, bend) = (Instant::now(), r.start, *r.path.last().expect("a path holds its start"), r.router.diagonal_first);
        // Through the board, or (Shift+V) only the layers it joins.
        let via = (!self.board.joins_layers(at, s.net, s.layer, layer))
            .then(|| if self.blind { self.board.via_between(at, s.net, s.layer, layer) } else { self.board.via(at, s.net) });
        let new = tracks(&r.path, s.width, s.layer, s.net);
        // What makes way for the via too, when shoving.
        let shoved = match via {
            Some(v) if self.shove => r.shover.shove(&self.board, &r.path, s.width, s.layer, Some(&v)).map_err(|e| e.what),
            _ => Ok(r.shoved.clone()),
        };
        let what = shoved.as_ref().ok().and_then(|shoved| fixed(new.len(), via.is_some(), shoved));
        match shoved.and_then(|shoved| self.board.add_route(&shoved, new, via).map_err(|v| format!("{}. {}", v.what, v.fix))) {
            Ok(undo) => {
                (self.active, self.route) = (layer, Some(Route::new(&self.board, Start { at, layer, ..s }, self.board_grid(), bend)));
                if let Some(what) = what {
                    self.edited(undo, t, what);
                }
            }
            Err(why) => {
                if let Some(r) = &mut self.route {
                    r.note = trf("No room for a via here: {0}", &[&why]);
                }
            }
        }
    }

    /// W, Shift+W: the track being routed steps to the next wider (or narrower) track width
    /// (`Board::step_width`), its path found again for it; past the last, it says so.
    pub(crate) fn route_width(&mut self, wider: bool) {
        let Some(r) = &mut self.route else { return };
        let (width, min) = (units::len(r.start.width), units::len(self.board.rules.track_width));
        r.note = match self.board.step_width(r.start.width, wider) {
            Some(width) => {
                let aim = r.aim;
                *r = Route::new(&self.board, Start { width, ..r.start }, r.grid, r.router.diagonal_first);
                r.aim = aim;
                return r.find(&self.board, self.shove);
            }
            None if wider => trf("{0} is the widest track width: KiCad's Board Setup, Pre-defined Sizes adds wider ones", &[&width]),
            None => trf("{0} is the narrowest track width; the rules' minimum is {1}", &[&width, &min]),
        };
    }

    /// `/`: the track being routed bends the other way round, its diagonal first or last where
    /// the way is clear (`Router::diagonal_first`).
    pub(crate) fn swap_bend(&mut self) {
        if let Some(r) = &mut self.route {
            r.router.diagonal_first = !r.router.diagonal_first;
            r.find(&self.board, self.shove);
        }
    }

    /// P: other nets' tracks and vias make way for the tracks routed, or they walk round them
    /// (for this track and the next ones).
    pub(crate) fn toggle_shove(&mut self) {
        self.shove = !self.shove;
        if let Some(r) = &mut self.route {
            r.find(&self.board, self.shove);
        }
    }

    /// The track being routed, in a light tint of its layer's colour, with a red ring where the
    /// pointer aims when other copper or the edge keeps it from getting there; what makes way for
    /// it, where it goes in its own colour (dimmed where it was), and a red ring on what can't.
    pub(crate) fn draw_route(&self, ui: &egui::Ui, screen: impl Fn(Pt) -> egui::Pos2) {
        let (Some(r), Some(scene)) = (&self.route, &self.scene) else { return };
        let (b, p, px) = (&self.board, ui.painter(), |nm: Nm| (nm as f32 / 1000.0 / self.um_per_pt).max(1.0));
        let colour = |slot: usize| crate::color32(scene.layers[slot].color);
        let gone = crate::theme::CANVAS.gamma_multiply(0.75);
        // A line as copper draws: round joins and ends.
        let copper = |pts: &[egui::Pos2], width: f32, c: egui::Color32| {
            p.add(egui::Shape::line(pts.to_vec(), (width, c)));
            for &q in pts {
                p.circle_filled(q, width / 2.0, c);
            }
        };
        for (old, pts) in &r.shoved.runs {
            let t = &b.tracks[old[0]];
            for o in old.iter().map(|&k| &b.tracks[k]) {
                copper(&[screen(o.a), screen(o.b)], px(t.width), gone);
            }
            copper(&pts.iter().map(|&q| screen(q)).collect::<Vec<_>>(), px(t.width), colour(t.layer as usize));
        }
        for &(i, at) in &r.shoved.vias {
            let v = &b.vias[i];
            p.circle_filled(screen(v.at), px(v.size) / 2.0, gone);
            p.circle_filled(screen(at), px(v.size) / 2.0, colour(scene.layers.len() - 2));
            p.circle_filled(screen(at), px(v.drill) / 2.0, colour(scene.layers.len() - 1));
        }
        if let Some(stuck) = &r.stuck {
            p.circle_stroke(screen(stuck.at), 6.0, (2.0, crate::theme::FAIL));
        }
        let color = colour(r.start.layer as usize).lerp_to_gamma(egui::Color32::WHITE, 0.4);
        let width = px(r.start.width);
        let pts: Vec<_> = r.path.iter().map(|&p| screen(p)).collect();
        // The room it keeps from other nets' copper: a soft band its clearance wide each side,
        // past both ends (one translucent stroke, so the band doesn't darken where it overlaps).
        let keeps = b.class(r.start.net).map_or(b.rules.clearance, |c| c.clearance);
        let clearance = keeps as f32 / 1000.0 / self.um_per_pt;
        let reach = width / 2.0 + clearance;
        let mut band = pts.clone();
        if let ([a, b, ..], [.., y, z]) = (&pts[..], &pts[..]) {
            let past = |from: egui::Pos2, to: egui::Pos2| if from == to { to } else { to + (to - from).normalized() * reach };
            (band[0], *band.last_mut().expect("two points")) = (past(*b, *a), past(*y, *z));
        }
        p.add(egui::Shape::line(band, (2.0 * reach, color.gamma_multiply(0.16))));
        copper(&pts, width, color);
        if r.path.last() != Some(&r.aim) {
            // Where other copper or the edge keeps it out: how much room it needs.
            let at = screen(r.aim);
            p.circle_stroke(at, 6.0, (2.0, crate::theme::FAIL));
            let needs = trf("needs {0}", &[&units::len(keeps)]);
            p.text(at + egui::vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, needs, egui::FontId::proportional(12.0), crate::theme::FAIL);
        }
    }

    /// The track being routed (net, layer, width, whether other nets make way, and why the last
    /// change asked for didn't happen) or how to start one, for the status bar.
    pub(crate) fn route_text(&self) -> String {
        let Some(r) = &self.route else { return tr("Route: click a pad, via or track").into() };
        let (s, b) = (r.start, &self.board);
        let net = b.nets.get(s.net as usize).map_or("", String::as_str);
        let how = match (&r.stuck, self.shove) {
            (_, false) => tr("walks round other nets (P shoves)").to_string(),
            (Some(stuck), true) => trf("walks round: {0}", &[&stuck.what]),
            (None, true) => tr("other nets make way (P walks round)").to_string(),
        };
        let note = if r.note.is_empty() { String::new() } else { format!(" · {}", r.note) };
        let routing = trf("Routing {0} on {1}, {2}", &[&net, &b.layers[s.layer as usize], &units::len(s.width)]);
        format!("{routing} · {how}{note}")
    }
}

/// What fixing `n` tracks and a `via` with what `shoved` moves aside says; `None` for nothing.
fn fixed(n: usize, via: bool, shoved: &Shoved) -> Option<String> {
    let added = match (via, n) {
        (true, _) => tr("Via added").to_string(),
        (false, 0) => return None,
        _ => count(n, n!("track added|tracks added")),
    };
    Some(match shoved.runs.len() + shoved.vias.len() {
        0 => added,
        moved => format!("{added}, {}", count(moved, n!("moved aside|moved aside"))),
    })
}

/// The route tool's keys and clicks.
pub(crate) const HINT: &str = t!("Click fixes the track · V via · W width · / bend · P shove · Enter finishes · Esc drops it");
/// Every key of it, in full: the hint's hover.
pub(crate) const KEYS: &str = t!(
    "Click fixes the track up to the pointer · PgUp, PgDn, + and − go on on another layer from a via, V on the other side; Shift+V keeps vias to the layers they join (blind or buried) · W and Shift+W step its width, / swaps its bend, P walks round other nets or shoves them · Enter or double click finishes · Esc drops the unfixed part, again leaves the tool"
);
