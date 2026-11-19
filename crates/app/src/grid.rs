//! The board's grid: its steps (N, Shift+N), shown as faint dots over the board and in the
//! status bar; the points that dragged parts, tool clicks and routes snap to (Ctrl frees them).

use crate::cmd::Cmd;
use crate::tr::{tr, trf};
use crate::{App, helpers, theme, zone};
use cuprita_core::board::{Board, Geom, LayerId, Nm, Pt};
use cuprita_core::drc::Item;
use cuprita_core::history::Pick;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, pos2, vec2};

/// The grid steps N goes through, a short list of KiCad's: metric, then imperial.
pub(crate) const STEPS: [(Nm, &str); 10] = [
    (1_000_000, "1 mm"),
    (500_000, "0.5 mm"),
    (250_000, "0.25 mm"),
    (100_000, "0.1 mm"),
    (50_000, "0.05 mm"),
    (2_540_000, "100 mil"),
    (1_270_000, "50 mil"),
    (635_000, "25 mil"),
    (254_000, "10 mil"),
    (127_000, "5 mil"),
];

/// The step a board opens with, 0.25 mm: fine enough to route and place parts, coarse enough
/// to line them up.
pub(crate) const DEFAULT: usize = 2;

/// Dots never come closer than this many points (KiCad's least grid spacing).
const SPACING: f32 = 10.0;

/// The dots' colour over the board: a faint white, quiet on the dark canvas, still seen on copper.
pub(crate) const ON_BOARD: Color32 = Color32::from_rgba_premultiplied(36, 36, 36, 36);

impl App {
    /// The board's grid: a point it goes through (its origin) and its step.
    pub(crate) fn board_grid(&self) -> (Pt, Nm) {
        (self.board.grid_origin, STEPS[self.grid].0)
    }

    /// The board's grid point nearest `p`.
    pub(crate) fn on_grid(&self, p: Pt) -> Pt {
        snap(p, self.board_grid())
    }

    /// Where a click of the measure, zone or outline tool at `at` puts its point: on the centre
    /// of the pad or via, or the end of the track, under the pointer (with which, for the ring
    /// naming it); else on the grid (`free`, Ctrl held: where it is); a zone's or outline's next
    /// corner at a multiple of 45° from the last, whole grid steps along, unless `any` (Shift).
    /// The zone tool's first click, which takes a net, is the pointer's place.
    pub(crate) fn aim(&self, at: Pt, tol: Nm, any: bool, free: bool) -> (Pt, Option<Item>) {
        if self.zoning && !self.outlining && !self.cutting && !self.lining && self.zone.is_none() {
            return (at, None);
        }
        if let Some((p, item)) = copper_point(&self.board, at, tol, &self.shown_copper()) {
            return (p, Some(item));
        }
        let point = match self.zone.as_ref().filter(|z| !z.rect).and_then(|z| z.corners.last()) {
            Some(&last) if !any => helpers::lock45(last, at, if free { 0 } else { STEPS[self.grid].0 }),
            _ if free => zone::um(at),
            _ => self.on_grid(at),
        };
        (point, None)
    }

    /// Steps to the next grid of `STEPS` (N) or, `back`, the one before (Shift+N), going round.
    pub(crate) fn step_grid(&mut self, back: bool) {
        self.grid = (self.grid + if back { STEPS.len() - 1 } else { 1 }) % STEPS.len();
        self.status = trf("Grid {0}", &[&STEPS[self.grid].1]);
    }

    /// The grid's step in the status bar, beside the pointer's place; a click steps it as N does.
    pub(crate) fn grid_label(&mut self, ui: &mut egui::Ui) {
        let sch = self.sch_view();
        let step = if sch { "50 mil" } else { STEPS[self.grid].1 };
        let text = RichText::new(trf("Grid {0}", &[&step])).monospace().color(theme::QUIET);
        let label = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
        let tip = if sch {
            tr("The schematic's connection grid, KiCad's")
        } else {
            tr(
                "N steps the grid, Shift+N back. Dragged parts, the tools' points and tracks snap to it, or to the copper under the pointer; hold Ctrl to place off it",
            )
        };
        if label.on_hover_text(tip).clicked() && !sch {
            self.run(ui.ctx(), Cmd::GridNext);
        }
    }
}

/// Of the parts' anchors `others`, the nearest on the same vertical line as `at`, and on the same
/// horizontal one: where a part dragged to `at` lines up with others.
pub(crate) fn lined_up(at: Pt, others: impl Iterator<Item = Pt>) -> [Option<Pt>; 2] {
    let mut lined = [None, None];
    for o in others.filter(|&o| o != at) {
        for (axis, along) in [(0, o.x == at.x), (1, o.y == at.y)] {
            let far = |p: Pt| if axis == 0 { (p.y - at.y).abs() } else { (p.x - at.x).abs() };
            if along && lined[axis].is_none_or(|p| far(o) < far(p)) {
                lined[axis] = Some(o);
            }
        }
    }
    lined
}

/// The point of the grid through `origin` every `step` nearest `p`.
pub(crate) fn snap(p: Pt, (origin, step): (Pt, Nm)) -> Pt {
    let near = |v: Nm, o: Nm| (o as i64 + ((v as i64 - o as i64) as f64 / step as f64).round() as i64 * step as i64) as Nm;
    Pt { x: near(p.x, origin.x), y: near(p.y, origin.y) }
}

/// The point a click at `at` joins on the copper `layers` in view (top first) within `tol`: the
/// centre of the pad or via, or the end of the track, `Board::copper_at` finds there.
fn copper_point(b: &Board, at: Pt, tol: Nm, layers: &[LayerId]) -> Option<(Pt, Item)> {
    let item = b.copper_at(at, tol, layers)?;
    let near = |q: &Pt, r: Nm| (q.x as f64 - at.x as f64).hypot(q.y as f64 - at.y as f64) <= (r + tol) as f64;
    let point = match item {
        Item::Pad(i) => b.pads[i].at,
        Item::Via(i) => b.vias[i].at,
        Item::Track(i) => [b.tracks[i].a, b.tracks[i].b].into_iter().find(|e| near(e, b.tracks[i].width / 2))?,
    };
    Some((point, item))
}

/// How far what is picked, dragged from `from` to `to` by item `held`, moves: so that the pad
/// grabbed (within `tol`), else the part's anchor, the track's end or zone's corner nearest
/// `from`, or the via, lands on `grid`; `free` (Ctrl held) as the pointer moved, in whole µm.
pub(crate) fn drag_by(b: &Board, grid: (Pt, Nm), held: Pick, (from, to): (Pt, Pt), tol: Nm, free: bool) -> Pt {
    let by = Pt { x: to.x - from.x, y: to.y - from.y };
    if free {
        return zone::um(by);
    }
    let nearest = |pts: Vec<Pt>| pts.into_iter().min_by_key(|p| (p.x as i64 - from.x as i64).pow(2) + (p.y as i64 - from.y as i64).pow(2));
    let point = match held {
        Pick::Footprint(i) => {
            let f = &b.footprints[i];
            b.pads[f.pads.clone()].iter().find(|p| p.holds(from, tol)).map_or(f.at, |p| p.at)
        }
        Pick::Track(i) => nearest(vec![b.tracks[i].a, b.tracks[i].b]).unwrap_or(from),
        Pick::Via(i) => b.vias[i].at,
        Pick::Zone(i) => nearest(b.zones[i].outline.concat()).unwrap_or(from),
        Pick::Text(i) => b.texts[i].at,
        // A line's nearest end or a polygon's nearest corner; a circle's centre.
        Pick::Shape(i) => match &b.shapes[i].geom {
            Geom::Segment { a, b } => nearest(vec![*a, *b]).unwrap_or(from),
            Geom::Arc { start, end, .. } => nearest(vec![*start, *end]).unwrap_or(from),
            Geom::Circle { center, .. } => *center,
            Geom::Poly(pts) => nearest(pts.clone()).unwrap_or(from),
        },
    };
    let to = snap(Pt { x: point.x + by.x, y: point.y + by.y }, grid);
    Pt { x: to.x - point.x, y: to.y - point.y }
}

/// Dots on the grid `(origin, step)` over the canvas showing the board box with opposite
/// corners `view` (`screen` maps board points onto it), as `lines` places them; one mesh.
pub(crate) fn dots(p: &egui::Painter, view: [Pt; 2], (origin, step): (Pt, Nm), color: Color32, screen: impl Fn(Pt) -> Pos2) {
    let gap = (screen(Pt { x: origin.x.saturating_add(step), ..origin }).x - screen(origin).x).abs();
    let (xs, ys) = lines(view, (origin, step), gap);
    let xs: Vec<f32> = xs.into_iter().map(|x| screen(Pt { x, ..origin }).x).collect();
    let ys: Vec<f32> = ys.into_iter().map(|y| screen(Pt { y, ..origin }).y).collect();
    // Square dots on whole pixels, so they stay crisp as the view pans.
    let ppp = p.ctx().pixels_per_point();
    let side = (1.5 * ppp).round().max(1.0);
    let edge = |v: f32| (v * ppp - side / 2.0).round() / ppp;
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices(4 * xs.len() * ys.len());
    mesh.reserve_triangles(2 * xs.len() * ys.len());
    for &y in &ys {
        for &x in &xs {
            mesh.add_colored_rect(Rect::from_min_size(pos2(edge(x), edge(y)), vec2(side, side) / ppp), color);
        }
    }
    p.add(mesh);
}

/// Where the dots go over the board box with opposite corners `view`, x then y: every grid
/// line, or every 2, 4, 10, 20, 40…, so that they never come closer than `SPACING` when one
/// step shows `gap` points long.
fn lines(view: [Pt; 2], (origin, step): (Pt, Nm), gap: f32) -> (Vec<Nm>, Vec<Nm>) {
    let Some(k) = (0..9).flat_map(|e| [1, 2, 4].map(|m| m * 10i64.pow(e))).find(|&k| gap * k as f32 >= SPACING) else {
        return (vec![], vec![]);
    };
    let s = step as i64 * k;
    // The multiples of `s` from `o` between `a` and `b`.
    let along = |a: Nm, b: Nm, o: Nm| {
        let (lo, hi) = (a.min(b) as i64 - o as i64, a.max(b) as i64 - o as i64);
        (-(-lo).div_euclid(s)..=hi.div_euclid(s)).map(|i| (o as i64 + i * s) as Nm).collect()
    };
    (along(view[0].x, view[1].x, origin.x), along(view[0].y, view[1].y, origin.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    #[test]
    fn lines_a_dragged_part_up_with_the_nearest_others() {
        let pt = |x, y| Pt { x, y };
        let others = [pt(0, 5), pt(0, -2), pt(7, 9), pt(3, 9), pt(4, 4), pt(1, 1)];
        // At (0, 9): (0, 5) above it is nearer than (0, -2); (3, 9) nearer than (7, 9) along.
        assert_eq!(lined_up(pt(0, 9), others.into_iter()), [Some(pt(0, 5)), Some(pt(3, 9))]);
        assert_eq!(lined_up(pt(4, 4), others.into_iter()), [None, None]);
    }

    #[test]
    fn snaps_to_the_grid_copper_or_the_pad_grabbed() {
        assert_eq!(snap(mm(-0.13, 0.37), (mm(0.1, 0.0), 250_000)), mm(-0.15, 0.25));
        assert_eq!(snap(mm(25.39, -1.2), (Pt::default(), 1_270_000)), mm(25.4, -1.27));
        // R1 at (10.03, 20.01) with pads 1.1 mm left and 1 mm right of it; a track and a via.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A")
              (footprint "R" (layer "F.Cu") (at 10.03 20.01)
                (pad "1" smd rect (at -1.1 0) (size 1 1) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))
              (segment (start 0 0) (end 5 0) (width 0.25) (layer "F.Cu") (net 1))
              (via (at 5 5) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1)))"#,
        )
        .unwrap();
        let copper: Vec<LayerId> = (0..128).filter(|&l| b.copper & 1 << l != 0).collect();
        let point = |x, y| copper_point(&b, mm(x, y), 0, &copper);
        assert_eq!(point(9.2, 20.2), Some((mm(8.93, 20.01), Item::Pad(0))));
        assert_eq!(point(5.1, 5.2), Some((mm(5.0, 5.0), Item::Via(0))));
        assert_eq!(point(4.95, 0.05), Some((mm(5.0, 0.0), Item::Track(0))));
        // A track's middle, or nothing: the grid's point.
        assert_eq!((point(2.5, 0.05), point(7.0, 7.0)), (None, None));
        // Dragged 1 mm right and 0.3 mm down by pad 1, it lands on the 0.25 mm grid; by its body,
        // the anchor does; with Ctrl, as the pointer moved, in whole micrometres. By the track, its
        // end nearest the pointer does; by the via, its centre.
        let grid = (Pt::default(), 250_000);
        let drag = |held, from: Pt, free| drag_by(&b, grid, held, (from, Pt { x: from.x + 1_000_400, y: from.y + 300_000 }), 0, free);
        assert_eq!(drag(Pick::Footprint(0), mm(9.0, 20.0), false), mm(1.07, 0.24));
        assert_eq!(drag(Pick::Footprint(0), mm(10.03, 20.01), false), mm(0.97, 0.24));
        assert_eq!(drag(Pick::Footprint(0), mm(9.0, 20.0), true), mm(1.0, 0.3));
        assert_eq!(drag(Pick::Track(0), mm(1.07, 0.01), false), mm(1.0, 0.25));
        assert_eq!(drag(Pick::Via(0), mm(5.13, 5.2), false), mm(1.0, 0.25));
    }

    #[test]
    fn dots_every_grid_line_or_every_few_never_a_carpet() {
        let pt = |x: Nm, y: Nm| Pt { x, y };
        // A view 1.1 x 0.5 mm, its corners either way round, on a 0.25 mm grid through (0.1, 0).
        let (view, grid) = ([pt(1_100_000, 0), pt(0, 500_000)], (pt(100_000, 0), 250_000));
        let every = (vec![100_000, 350_000, 600_000, 850_000, 1_100_000], vec![0, 250_000, 500_000]);
        assert_eq!(lines(view, grid, 25.0), every);
        assert_eq!(lines(view, grid, SPACING), every);
        // Steps 5 points long: every second line; 2.5 points: every fourth; then every tenth.
        assert_eq!(lines(view, grid, 5.0), (vec![100_000, 600_000, 1_100_000], vec![0, 500_000]));
        assert_eq!(lines(view, grid, 2.5), (vec![100_000, 1_100_000], vec![0]));
        assert_eq!(lines(view, (pt(-100_000, 0), 250_000), 1.0), (vec![], vec![0]));
        // A mil step from an origin far off the view.
        let mil = lines([pt(-2_540_000, -1), pt(2_540_000, 1)], (pt(-1_270_000_000, 0), 1_270_000), 12.0);
        assert_eq!(mil, (vec![-2_540_000, -1_270_000, 0, 1_270_000, 2_540_000], vec![0]));
    }
}
