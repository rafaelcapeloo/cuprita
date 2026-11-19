//! The pointer's helpers on the canvases. While a tool draws: a crosshair at the point a click
//! takes, a ring on the copper or pin it would join (a diamond on the grid else) with its name,
//! and a readout by the pointer. With no tool: a glow on what a click would pick; the box a
//! Shift+drag draws to pick what it takes.

use crate::tr::{tr, trf};
use crate::{App, theme, units};
use cuprita_core::board::{Board, Nm, Pt};
use cuprita_core::drc::Item;
use cuprita_core::history::Pick;
use eframe::egui::{self, Color32, Pos2, Stroke, vec2};

/// The helpers' colours on a canvas: dark ones on the schematic's paper, light on the board.
#[derive(Clone, Copy)]
pub(crate) struct Ink {
    cross: Color32,
    mark: Color32,
}

pub(crate) const ON_BOARD: Ink = Ink { cross: Color32::WHITE, mark: theme::SIGNAL };
const ON_PAPER: Ink = Ink { cross: Color32::from_rgb(0x20, 0x24, 0x2A), mark: Color32::from_rgb(0x0A, 0x8C, 0xB0) };

/// The helpers' colours on a sheet: the board's on a dark page, dark ones on light paper.
fn on_sheet() -> Ink {
    if theme::dark_sheet() { ON_BOARD } else { ON_PAPER }
}

/// A small crosshair with an open centre at `at`, and faint lines across `rect`.
pub(crate) fn crosshair(p: &egui::Painter, rect: egui::Rect, at: Pos2, ink: Ink) {
    let faint = Stroke::new(1.0, ink.cross.gamma_multiply(0.08));
    p.hline(rect.x_range(), at.y, faint);
    p.vline(at.x, rect.y_range(), faint);
    let stroke = Stroke::new(1.5, ink.cross.gamma_multiply(0.9));
    for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
        p.line_segment([at + d * 4.0, at + d * 12.0], stroke);
    }
}

/// A ring running out from the `marker` of what was just found, `t` of its way (0 to 1), thinning
/// as it goes: where to look, said once.
pub(crate) fn ping(p: &egui::Painter, marker: egui::Rect, t: f32) {
    if t < 1.0 {
        let ring = marker.expand(36.0 * t);
        p.rect_stroke(ring, 4.0 + 10.0 * t, (2.0, theme::SIGNAL.gamma_multiply(1.0 - t)), egui::StrokeKind::Outside);
    }
}

/// The box a Shift+drag draws from `from` to `to` (on screen) to pick what it takes: solid while
/// drawn left to right (it takes what it holds), dashed right to left (what it touches).
pub(crate) fn pick_box(p: &egui::Painter, from: Pos2, to: Pos2) {
    let r = egui::Rect::from_two_pos(from, to);
    p.rect_filled(r, 0.0, theme::SIGNAL.gamma_multiply(0.08));
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    if to.x < from.x {
        p.extend(egui::Shape::dashed_line(&corners, (1.0, theme::SIGNAL), 6.0, 4.0));
    } else {
        p.rect_stroke(r, 0.0, (1.0, theme::SIGNAL), egui::StrokeKind::Inside);
    }
}

/// A ring on what a click at `at` joins, with its name.
pub(crate) fn ring(p: &egui::Painter, at: Pos2, name: &str, ink: Ink) {
    p.circle_stroke(at, 10.0, Stroke::new(2.0, ink.mark));
    p.circle_filled(at, 2.0, ink.mark);
    if !name.is_empty() {
        tag(p, at + vec2(14.0, -26.0), &[name.to_string()], ink.mark, false);
    }
}

/// A diamond on the grid point a click takes.
pub(crate) fn diamond(p: &egui::Painter, at: Pos2, ink: Ink) {
    let pts = [vec2(0.0, -5.0), vec2(5.0, 0.0), vec2(0.0, 5.0), vec2(-5.0, 0.0)].map(|d| at + d).to_vec();
    p.add(egui::Shape::closed_line(pts, Stroke::new(1.5, ink.mark.gamma_multiply(0.85))));
}

/// A dashed guide on past `to`, the way the edge from `from` runs (on screen): where a track or
/// zone edge locked to 45° goes on.
pub(crate) fn guide(p: &egui::Painter, from: Pos2, to: Pos2, ink: Ink) {
    let way = to - from;
    if way.length() >= 1.0 {
        p.extend(egui::Shape::dashed_line(&[to, to + way.normalized() * 120.0], (1.5, ink.mark.gamma_multiply(0.65)), 8.0, 6.0));
    }
}

/// The readout beside the pointer at `at`: its lines, in mono, below and right of it (`left`: of
/// a guide running down to the right, so it doesn't hide it).
pub(crate) fn readout(p: &egui::Painter, at: Pos2, lines: &[String], left: bool) {
    tag(p, at + vec2(if left { -18.0 } else { 18.0 }, 16.0), lines, theme::TEXT, left);
}

/// Lines of mono text on a dark plate, its top left at `at` (`right`: its top right); the first
/// line in `color`.
fn tag(p: &egui::Painter, at: Pos2, lines: &[String], color: Color32, right: bool) {
    let font = egui::FontId::monospace(11.5);
    let galleys: Vec<_> = lines
        .iter()
        .enumerate()
        .map(|(k, l)| p.layout_no_wrap(l.clone(), font.clone(), if k == 0 { color } else { theme::TEXT_2 }))
        .collect();
    let (w, h) = (galleys.iter().map(|g| g.size().x).fold(0.0, f32::max), galleys.iter().map(|g| g.size().y).sum::<f32>());
    let at = if right { at - vec2(w, 0.0) } else { at };
    let plate = egui::Rect::from_min_size(at, vec2(w, h)).expand2(vec2(8.0, 5.0));
    p.rect(plate, 6.0, theme::GROUND.gamma_multiply(0.94), (1.0, theme::LINE), egui::StrokeKind::Inside);
    let mut y = at.y;
    for g in galleys {
        let height = g.size().y;
        p.galley(egui::pos2(at.x, y), g, color);
        y += height;
    }
}

/// `b` moved onto the nearest of the eight lines through `a` at a multiple of 45°, a whole
/// number of `step`s from `a` along each axis it runs along (0: any distance).
pub(crate) fn lock45(a: Pt, b: Pt, step: Nm) -> Pt {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let turn = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
    let (s, c) = turn.sin_cos();
    // The steps along the line: each moves (ux, uy) steps, √2 of them long on a diagonal.
    let ((ux, uy), step) = ((c.round() as i64, s.round() as i64), step.max(1) as i64);
    let n = ((dx * c + dy * s) / (step as f64 * (ux as f64).hypot(uy as f64))).round() as i64;
    Pt { x: (a.x as i64 + n * ux * step) as Nm, y: (a.y as i64 + n * uy * step) as Nm }
}

/// A segment's length in mm and its angle in degrees, counter-clockwise as KiCad's (Y down).
pub(crate) fn span(a: Pt, b: Pt) -> (f64, f64) {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    // + 0.0: a level segment reads 0°, not -0°.
    (dx.hypot(dy) / 1e6, (-dy).atan2(dx).to_degrees().rem_euclid(360.0) + 0.0)
}

/// What a click on `item` takes: `U3 pin 7 · SDA`, `Via · GND`, `Track · GND`.
pub(crate) fn copper_name(b: &Board, item: Item) -> String {
    let net = |n: u32| b.nets.get(n as usize).filter(|s| !s.is_empty()).map_or(String::new(), |s| format!(" · {s}"));
    match item {
        Item::Pad(i) => {
            let pad = &b.pads[i];
            trf("{0} pin {1}{2}", &[&b.footprints[pad.footprint as usize].reference, &pad.number, &net(pad.net)])
        }
        Item::Via(i) => trf("Via{0}", &[&net(b.vias[i].net)]),
        Item::Track(i) => trf("Track{0}", &[&net(b.tracks[i].net)]),
    }
}

impl App {
    /// The board canvas's helpers over `rect`: `pointer` is the board point under it, `tol` its
    /// reach in nm, `screen` maps board points to the screen. Returns the name of the copper the
    /// point a click takes snaps to.
    pub(crate) fn board_helpers(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        pointer: Option<Pt>,
        tol: Nm,
        screen: impl Fn(Pt) -> Pos2,
    ) -> Option<String> {
        let at = pointer?;
        let (b, p) = (&self.board, ui.painter());
        if !(self.routing || self.zoning || self.measuring || self.pairing) {
            self.hover_glow(p, at, tol, &screen);
            return None;
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        let (shift, free) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        let layers = self.shown_copper();
        let start = || b.route_start(at, tol, &layers);
        let name = |q: Pt| b.copper_at(q, tol, &layers).map(|i| copper_name(b, i));
        // `way`: the edge being drawn, which a guide carries on past its end.
        let (mut point, mut lines, mut snapped, mut way) = (screen(at), vec![], None, None);
        if let (true, Some(r)) = (self.routing, &self.route) {
            let (aim, joins, length, last) = r.aim();
            (point, way) = (screen(aim), Some((screen(last.0), screen(last.1))));
            if joins {
                snapped = name(aim);
                ring(p, point, snapped.as_deref().unwrap_or_default(), ON_BOARD)
            } else if !free {
                diamond(p, point, ON_BOARD)
            }
            let (_, angle) = span(last.0, last.1);
            lines.push(format!("{} · {angle:.0}°", units::len(length * 1e6)));
            let how = if self.shove { tr("shoves") } else { tr("walks round") };
            lines.push(format!("{} · {} · {how}", b.layers[r.start.layer as usize], units::len(r.start.width)));
        } else if self.routing || self.zoning && !self.outlining && !self.cutting && !self.lining && self.zone.is_none() {
            // The copper a track or zone starts on.
            if let Some(s) = start() {
                (point, snapped) = (screen(s.at), name(s.at));
                ring(p, point, snapped.as_deref().unwrap_or_default(), ON_BOARD);
            }
        } else {
            // A point of the measure, zone or outline tool: on the copper ringed, else the grid.
            let (aim, on) = self.aim(at, tol, shift, free);
            point = screen(aim);
            snapped = on.map(|item| copper_name(b, item));
            match &snapped {
                Some(name) => ring(p, point, name, ON_BOARD),
                None if !free => diamond(p, point, ON_BOARD),
                None => {}
            }
            let zone = self.zone.as_ref().filter(|_| self.zoning);
            if let Some(z) = zone.filter(|z| z.rect) {
                let (w, h) = ((aim.x - z.corners[0].x).abs(), (aim.y - z.corners[0].y).abs());
                lines.push(format!("{} × {}", units::num(w), units::len(h)));
            } else if let Some(&last) = zone.and_then(|z| z.corners.last()) {
                way = Some((screen(last), point));
                let (len, angle) = span(last, aim);
                lines.push(format!("{} · {angle:.0}°", units::len(len * 1e6)));
                if on.is_none() {
                    lines.push(if shift { tr("any angle") } else { tr("45° · Shift frees") }.into());
                }
            }
        }
        if let Some((a, b)) = way {
            guide(p, a, b, ON_BOARD);
        }
        crosshair(p, rect, point, ON_BOARD);
        if !lines.is_empty() {
            readout(p, point, &lines, way.is_some_and(|(a, b)| b.x > a.x && b.y > a.y));
        }
        snapped
    }

    /// The schematic canvas's helpers over `rect`: with the wire or no-connect tool or a symbol
    /// being placed, a crosshair on the grid point a click takes, a ring where a wire would join
    /// something or the pin a flag would go on (a pin named; a diamond else), and the wire's
    /// length; with no tool, the symbol a click would select. Returns the pin the point snaps to.
    pub(crate) fn sheet_helpers(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        pointer: Option<Pt>,
        screen: impl Fn(Pt) -> Pos2,
    ) -> Option<String> {
        let (Some(at), Some(sch)) = (pointer, &self.sch) else { return None };
        let (p, sheet) = (ui.painter(), &sch.sheets.project.pages[sch.page].sheet);
        if sch.wire.is_none() && sch.placing.is_none() && !sch.flagging {
            // What a click would pick, unless it is picked: a wire glows, anything else a box.
            let tol = (3000.0 * self.um_per_pt) as Nm;
            let hit = sheet.item_at(at, tol).filter(|it| !sch.picked.contains(it) && sch.drag.is_none() && sch.boxing.is_none());
            let glow = on_sheet().mark.gamma_multiply(0.5);
            if let Some(it) = hit {
                let (a, b) = sheet.extent(it);
                if let cuprita_core::schematic::Item::Wire(_) = it {
                    p.line_segment([screen(a), screen(b)], (152.4 / self.um_per_pt + 4.0, glow));
                } else {
                    p.rect_stroke(egui::Rect::from_two_pos(screen(a), screen(b)).expand(3.0), 3.0, (1.5, glow), egui::StrokeKind::Outside);
                }
            }
            return None;
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        let grid = cuprita_core::schematic::on_grid(at);
        let (point, pin) = (screen(grid), sheet.pin_at(grid).map(|(reference, number)| trf("{0} pin {1}", &[&reference, &number])));
        if let Some(wire) = sch.wire {
            let joins = if sch.bus { sheet.joins_bus(grid) } else { sheet.joins(grid) };
            if joins {
                ring(p, point, pin.as_deref().filter(|_| !sch.bus).unwrap_or_default(), on_sheet())
            } else {
                diamond(p, point, on_sheet())
            }
            if let Some(from) = wire {
                let (dx, dy) = ((grid.x - from.x).abs() as f64, (grid.y - from.y).abs() as f64);
                let straight = ui.input(|i| i.modifiers.shift);
                readout(p, point, &[units::len(if straight { dx.hypot(dy) } else { dx + dy })], false);
            }
        }
        if sch.flagging {
            match &pin {
                Some(name) => ring(p, point, name, on_sheet()),
                None => diamond(p, point, on_sheet()),
            }
        }
        crosshair(p, rect, point, on_sheet());
        pin.filter(|_| sch.flagging || sch.wire.is_some() && !sch.bus)
    }

    /// What a click at `at` would select, lit before the click: a part's box, a track or a via.
    fn hover_glow(&self, p: &egui::Painter, at: Pt, tol: Nm, screen: &impl Fn(Pt) -> Pos2) {
        if self.grab.is_some() || self.boxing.is_some() || self.scene.is_none() {
            return;
        }
        let (b, glow) = (&self.board, theme::SIGNAL.gamma_multiply(0.45));
        let pick = self.pick_at(at, tol);
        if pick.is_none_or(|p| self.picks().contains(&p)) {
            return;
        }
        match pick {
            Some(Pick::Zone(i)) => {
                for ring in &b.zones[i].outline {
                    p.add(egui::Shape::closed_line(ring.iter().map(|&q| screen(q)).collect(), (1.5, glow)));
                }
            }
            Some(Pick::Footprint(i)) => {
                let (lo, hi) = b.footprints[i].bounds;
                p.rect_stroke(egui::Rect::from_two_pos(screen(lo), screen(hi)).expand(3.0), 3.0, (1.5, glow), egui::StrokeKind::Outside);
            }
            Some(Pick::Track(i)) => {
                let t = &b.tracks[i];
                let pts = [t.a].into_iter().chain(t.mid).chain([t.b]).map(screen).collect();
                p.add(egui::Shape::line(pts, (t.width as f32 / 1000.0 / self.um_per_pt + 4.0, theme::SIGNAL.gamma_multiply(0.22))));
            }
            Some(Pick::Via(i)) => {
                let v = &b.vias[i];
                p.circle_stroke(screen(v.at), v.size as f32 / 2000.0 / self.um_per_pt + 3.0, (1.5, glow));
            }
            Some(pick @ (Pick::Text(_) | Pick::Shape(_))) => drop(p.add(drawing_mark(b, pick, self.um_per_pt, glow, screen))),
            None => {}
        }
    }
}

/// The mark of a text or drawing of the board's own, picked or under the pointer: a box round
/// the text, a glow along the drawing's line (`screen` maps board points, `um_per_pt` its scale).
pub(crate) fn drawing_mark(b: &Board, pick: Pick, um_per_pt: f32, color: Color32, screen: impl Fn(Pt) -> Pos2) -> egui::Shape {
    let pts = b.mark(pick).into_iter().map(screen).collect();
    match pick {
        Pick::Shape(i) => egui::Shape::line(pts, (b.shapes[i].width as f32 / 1000.0 / um_per_pt + 6.0, color.gamma_multiply(0.4))),
        _ => egui::Shape::line(pts, (2.0, color)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_to_the_nearest_45_degrees() {
        let o = Pt { x: 0, y: 0 };
        assert_eq!(lock45(o, Pt { x: 10_000, y: 1_000 }, 0), Pt { x: 10_000, y: 0 });
        assert_eq!(lock45(o, Pt { x: 1_000, y: -9_000 }, 0), Pt { x: 0, y: -9_000 });
        // A diagonal keeps the pointer's distance along it.
        assert_eq!(lock45(o, Pt { x: 10_000, y: 9_000 }, 0), Pt { x: 9_500, y: 9_500 });
        assert_eq!(lock45(Pt { x: 5, y: 5 }, Pt { x: 5, y: 5 }, 0), Pt { x: 5, y: 5 });
        // On a grid, whole steps along each axis: from a grid point, onto grid points.
        let a = Pt { x: 100_000, y: 0 };
        assert_eq!(lock45(a, Pt { x: 1_220_000, y: 130_000 }, 250_000), Pt { x: 1_100_000, y: 0 });
        assert_eq!(lock45(a, Pt { x: 700_000, y: -640_000 }, 250_000), Pt { x: 600_000, y: -500_000 });
        assert_eq!(lock45(a, Pt { x: 160_000, y: -1_000_000 }, 254_000), Pt { x: 100_000, y: -1_016_000 });
    }

    #[test]
    fn measures_a_segment_as_kicad_turns() {
        let (len, angle) = span(Pt { x: 0, y: 0 }, Pt { x: 3_000_000, y: -3_000_000 });
        assert_eq!((format!("{len:.3}"), format!("{angle:.0}")), ("4.243".into(), "45".into()));
        assert_eq!(span(Pt { x: 0, y: 0 }, Pt { x: 0, y: 2_000_000 }).1.round(), 270.0);
        assert_eq!(format!("{:.0}", span(Pt { x: 5, y: 0 }, Pt { x: 9, y: 0 }).1), "0");
    }
}
