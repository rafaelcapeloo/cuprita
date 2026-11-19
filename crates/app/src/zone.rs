//! Zone tool (`Z`): click a pad, via or track to take its net and layer, then the zone's
//! corners, each at a multiple of 45° from the last unless Shift is held; Enter or a double
//! click closes the zone and fills it, Backspace drops the last corner, Esc drops the zone.
//! Outline tool (`O`, the zone tool `outlining`): the same corners from the first click, or a
//! drag's rectangle, drawn on Edge.Cuts as the board's new outline (`Board::set_outline`).
//! Cutout tool (Shift+C, the zone tool `cutting`): the same corners from the first click, inside
//! the selected zone, cut out of it (`Zone::add_cutout`).
//! Line tool (`L`, the zone tool `lining`): the same corners or rectangle, drawn as lines on the
//! layer the drawing tools are on (`drawing.rs`), left open unless they end where they start.

use crate::tr::{n, t, tr, trf};
use crate::{App, count};
use cuprita_core::board::{LayerId, NetId, Nm, Pt};
use cuprita_core::history::{Patch, Pick};
use eframe::egui;
use web_time::Instant; // std's panics in the browser

/// A zone being drawn: the net and layer of the copper clicked first, and its corners; or the
/// board's outline (on Edge.Cuts), a rectangle from its first corner while `rect` (a drag); or
/// a cutout, on the selected zone's net and first layer.
pub(crate) struct Draft {
    net: NetId,
    layer: LayerId,
    pub(crate) corners: Vec<Pt>,
    pub(crate) rect: bool,
}

/// Whole micrometres: as fine as a pointer gets, without float noise in the file.
pub(crate) fn um(p: Pt) -> Pt {
    let um = |v: Nm| (v as f64 / 1000.0).round() as Nm * 1000;
    Pt { x: um(p.x), y: um(p.y) }
}

/// The rectangle with corners `a` and `b` across it.
pub(crate) fn rectangle(a: Pt, b: Pt) -> Vec<Pt> {
    vec![a, Pt { x: b.x, y: a.y }, b, Pt { x: a.x, y: b.y }]
}

impl App {
    /// A zone tool click at `at` (`App::aim`'s): a corner of the zone being drawn, or, to start
    /// one, the pad, via or track whose net and layer it takes.
    pub(crate) fn zone_click(&mut self, at: Pt, tol: Nm) {
        if let Some(z) = &mut self.zone {
            return z.corners.push(at);
        }
        if self.outlining || self.lining {
            return self.zone = self.outline_draft(at, false);
        }
        if self.cutting {
            return self.zone = self.cutout_draft(at);
        }
        match self.board.route_start(at, tol, &self.shown_copper()) {
            Some(s) => {
                let draft = Draft { net: s.net, layer: s.layer, corners: vec![], rect: false };
                (self.net, self.zone, self.status) = (s.net, Some(draft), String::new());
            }
            None => self.status = tr("Click a pad, via or track: the zone takes its net and layer").into(),
        }
    }

    /// The board's outline drawn from `at`, a rectangle while `rect`; on Edge.Cuts. With the Line
    /// tool, lines on the layer it draws on.
    pub(crate) fn outline_draft(&mut self, at: Pt, rect: bool) -> Option<Draft> {
        if self.lining {
            let Some(layer) = self.ink_layer() else {
                self.status = tr("The board has no layer to draw on: add a silkscreen layer in KiCad").into();
                return None;
            };
            return Some(Draft { net: 0, layer, corners: vec![at], rect });
        }
        let Some(layer) = self.board.layers.iter().position(|l| l == "Edge.Cuts") else {
            self.status = tr("The board has no Edge.Cuts layer to draw its outline on: add it in KiCad").into();
            return None;
        };
        Some(Draft { net: 0, layer: layer as LayerId, corners: vec![at], rect })
    }

    /// A cutout of the selected zone drawn from `at`.
    fn cutout_draft(&mut self, at: Pt) -> Option<Draft> {
        let Some(Pick::Zone(i)) = self.selected else {
            self.status = tr("Select the zone to cut first: click its fill or outline").into();
            return None;
        };
        let z = &self.board.zones[i];
        Some(Draft { net: z.net, layer: z.layers.trailing_zeros() as LayerId, corners: vec![at], rect: false })
    }

    /// Closes the zone being drawn (Enter or a double click): adds it to the board, filled; a
    /// cutout, cut out of the selected zone (refused, it stays drawn to fix).
    pub(crate) fn zone_close(&mut self) {
        // Lines end at two corners; a rectangle dragged closes back on its first.
        if let Some(z) = self.zone.take_if(|z| self.lining && z.corners.len() >= 2) {
            let (t, width) = (Instant::now(), self.board.ink(z.layer).0);
            let (picks, undo) = self.board.add_lines(&z.corners, false, z.layer, width);
            let what = trf("{0} on {1} drawn", &[&count(picks.len(), n!("line")), &self.board.layers[z.layer as usize]]);
            (self.selected, self.picked) = crate::canvas::one_or_more(picks);
            return self.edited(undo, t, what);
        }
        let Some(z) = self.zone.take_if(|z| z.corners.len() >= 3) else {
            if self.zone.is_some() {
                self.status = if self.lining {
                    tr("A line needs two corners or more")
                } else if self.outlining {
                    tr("An outline needs three corners or more")
                } else if self.cutting {
                    tr("A cutout needs three corners or more")
                } else {
                    tr("A zone needs three corners or more")
                }
                .into();
            }
            return;
        };
        if self.outlining {
            let t = Instant::now();
            return match self.board.set_outline(&z.corners) {
                Ok(undo) => {
                    self.zoning = false;
                    self.edited(undo, t, tr("Board outline drawn").into());
                }
                Err(e) => self.status = e,
            };
        }
        if self.cutting {
            let Some(Pick::Zone(i)) = self.selected else {
                return self.status = tr("Select the zone to cut first: click its fill or outline").into();
            };
            let (t, mut cut) = (Instant::now(), self.board.zones[i].clone());
            let done = cut.add_cutout(z.corners.clone());
            if done.is_err() {
                self.zone = Some(z);
            }
            return self.zone_edited(i, cut, done, t, None);
        }
        let (t, undo, what) = (Instant::now(), Patch::added(&self.board), self.zone_name(&z, true));
        self.board.add_zone(z.corners, z.layer, z.net);
        self.edited(undo, t, what);
    }

    /// Drops the last corner of the zone being drawn (Backspace).
    pub(crate) fn zone_back(&mut self) {
        if let Some(z) = &mut self.zone {
            z.corners.pop();
        }
    }

    /// The zone being drawn, through its corners and the pointer, in a light tint of its
    /// layer's colour.
    pub(crate) fn draw_zone(&self, ui: &egui::Ui, pointer: Option<Pt>, screen: impl Fn(Pt) -> egui::Pos2) {
        let (Some(z), Some(scene)) = (&self.zone, &self.scene) else { return };
        let color = crate::color32(scene.layers[z.layer as usize].color).lerp_to_gamma(egui::Color32::WHITE, 0.4);
        let corners = if z.rect { rectangle(z.corners[0], pointer.unwrap_or(z.corners[0])) } else { z.corners.clone() };
        let pts: Vec<_> = corners.iter().copied().chain(pointer.filter(|_| !z.rect)).map(screen).collect();
        for &p in &pts[..corners.len()] {
            ui.painter().circle_filled(p, 3.0, color);
        }
        // Lines stay open, but a rectangle.
        let open = self.lining && !z.rect;
        ui.painter().add(if open { egui::Shape::line(pts, (2.0, color)) } else { egui::Shape::closed_line(pts, (2.0, color)) });
    }

    /// The zone being drawn (net, layer, corners) or how to start one, for the status bar.
    pub(crate) fn zone_text(&self) -> String {
        match &self.zone {
            Some(z) if self.lining => {
                trf("Line on {0}: {1}", &[&self.board.layers[z.layer as usize], &count(z.corners.len(), n!("corner"))])
            }
            Some(z) if self.outlining => trf("Outline: {0}", &[&count(if z.rect { 4 } else { z.corners.len() }, n!("corner"))]),
            Some(z) if self.cutting => trf("Cutout: {0}", &[&count(z.corners.len(), n!("corner"))]),
            Some(z) => trf("{0}: {1}", &[&self.zone_name(z, false), &count(z.corners.len(), n!("corner"))]),
            None if self.lining => tr("Line: click its corners, or drag a rectangle").into(),
            None if self.outlining => tr("Outline: drag a rectangle, or click its corners").into(),
            None if self.cutting => tr("Cutout: click its corners inside the zone").into(),
            None => tr("Zone: click a pad, via or track of its net").into(),
        }
    }

    /// `Zone (GND) on F.Cu`, as DRC names tracks; with `added`, that it was added.
    fn zone_name(&self, z: &Draft, added: bool) -> String {
        let (net, layer) = (&self.board.nets[z.net as usize], &self.board.layers[z.layer as usize]);
        match (z.net, added) {
            (0, false) => trf("Zone on {0}", &[layer]),
            (0, true) => trf("Zone on {0} added", &[layer]),
            (_, false) => trf("Zone ({0}) on {1}", &[net, layer]),
            (_, true) => trf("Zone ({0}) on {1} added", &[net, layer]),
        }
    }
}

/// The outline tool's keys and clicks.
pub(crate) const OUTLINE_HINT: &str =
    t!("Drag a rectangle or click its corners · Enter closes it · Backspace undoes a corner · Esc drops it");
/// Every key of it, in full: the hint's hover.
pub(crate) const OUTLINE_KEYS: &str = t!(
    "Drag a rectangle, or click the outline's corners, on the grid (Ctrl frees them) · Enter or double click closes it as the board's outline · Backspace drops the last corner · Esc drops it, again leaves the tool"
);

/// The cutout tool's keys and clicks.
pub(crate) const CUTOUT_HINT: &str = t!("Click its corners inside the selected zone · Enter cuts it out · Esc drops it");
/// Every key of it, in full: the hint's hover.
pub(crate) const CUTOUT_KEYS: &str = t!(
    "Click the cutout's corners inside the selected zone, on the grid (Ctrl frees them) · Enter or double click cuts it out · Backspace drops the last corner · Esc drops it, again leaves the tool"
);

/// The zone tool's keys and clicks.
pub(crate) const HINT: &str = t!("Click a pad or track: the zone takes its net · then its corners · Enter fills it · Esc drops it");
/// Every key of it, in full: the hint's hover.
pub(crate) const KEYS: &str = t!(
    "Click a pad, via or track: the zone takes its net and layer · then click its corners, on the grid (Ctrl frees them) · Enter or double click closes and fills it · Backspace drops the last corner · Esc drops the zone, again leaves the tool"
);
