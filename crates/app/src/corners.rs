//! The picked zone's corners: a handle on each, and on the middle of each edge long enough. A
//! drag from a corner moves it (onto the grid or the copper under the pointer, as zone corners
//! snap; Ctrl frees it), and dropped on a corner beside it, it goes; a drag from an edge's middle
//! adds a corner there; Delete with the pointer on a corner takes it away. Each is an undo step,
//! the zone refilled alone (`Board::set_zone`); `canvas.rs` grabs, drops and draws them.

use crate::tr::{t, tr};
use crate::{App, pick_name, theme};
use cuprita_core::board::{Nm, Pt, Zone};
use cuprita_core::history::Pick;
use eframe::egui;
use std::cmp::Ordering;
use web_time::Instant; // std's panics in the browser

/// A handle of a zone's outline: corner `k` of ring `r` (0 the outline, then the cutouts), or
/// the middle of the edge from it to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Handle {
    Corner(usize, usize),
    Edge(usize, usize),
}

/// How far from a handle the pointer takes it, in points.
const REACH: f32 = 6.0;

fn mid(a: Pt, b: Pt) -> Pt {
    Pt { x: a.x + (b.x - a.x) / 2, y: a.y + (b.y - a.y) / 2 }
}

fn apart(a: Pt, b: Pt) -> f64 {
    (b.x as f64 - a.x as f64).hypot(b.y as f64 - a.y as f64)
}

/// Each edge of zone `z`: its ring, its first corner's index and its ends.
fn edges(z: &Zone) -> impl Iterator<Item = (usize, usize, Pt, Pt)> + '_ {
    z.outline.iter().enumerate().flat_map(|(r, ring)| (0..ring.len()).map(move |k| (r, k, ring[k], ring[(k + 1) % ring.len()])))
}

/// Whether an edge from `a` to `b` shows a handle in its middle: four reaches long or more, so
/// it stays clear of the corners'.
fn midway(a: Pt, b: Pt, reach: Nm) -> bool {
    apart(a, b) >= 4.0 * reach as f64
}

/// What an outline edit did, from the corners before and after.
fn outline_change(old: &[Vec<Pt>], new: &[Vec<Pt>]) -> &'static str {
    let corners = |o: &[Vec<Pt>]| o.iter().map(Vec::len).sum::<usize>();
    match (new.len().cmp(&old.len()), corners(new).cmp(&corners(old))) {
        (Ordering::Greater, _) => t!("cutout added"),
        (Ordering::Less, _) => t!("cutout removed"),
        (_, Ordering::Greater) => t!("corner added"),
        (_, Ordering::Less) => t!("corner removed"),
        _ => t!("corner moved"),
    }
}

/// Zone `z`'s handle within `reach` of `at`: a corner first, else an edge's middle.
pub(crate) fn handle_at(z: &Zone, at: Pt, reach: Nm) -> Option<Handle> {
    let near = |p: Pt| apart(p, at) <= reach as f64;
    edges(z)
        .find(|e| near(e.2))
        .map(|(r, k, ..)| Handle::Corner(r, k))
        .or_else(|| edges(z).find(|&(_, _, a, b)| midway(a, b, reach) && near(mid(a, b))).map(|(r, k, ..)| Handle::Edge(r, k)))
}

impl App {
    /// The handles' reach in board nanometres at the view's zoom.
    fn reach(&self) -> Nm {
        (REACH * 1000.0 * self.um_per_pt) as Nm
    }

    /// The picked zone and its handle at board point `at`.
    pub(crate) fn handle(&self, at: Pt) -> Option<(usize, Handle)> {
        let Some(Pick::Zone(i)) = self.selected else { return None };
        handle_at(&self.board.zones[i], at, self.reach()).map(|h| (i, h))
    }

    /// The end of a drag from the picked zone's handle at `from` to `to`: the corner moved
    /// there, or a corner added there; one undo step.
    pub(crate) fn drop_handle(&mut self, from: Pt, to: Pt) {
        let Some((i, h)) = self.handle(from) else { return };
        let (t, mut z) = (Instant::now(), self.board.zones[i].clone());
        let done = match h {
            Handle::Corner(r, k) if z.outline[r][k] == to => return,
            Handle::Corner(r, k) => z.move_corner(r, k, to),
            Handle::Edge(r, k) => z.add_corner(r, k, to),
        };
        self.zone_edited(i, z, done, t, None);
    }

    /// Delete with the pointer on a corner of the picked zone: takes the corner away (a cutout
    /// left with two goes); false when the pointer is on none.
    pub(crate) fn delete_corner(&mut self) -> bool {
        let Some((i, Handle::Corner(r, k))) = self.pointer.and_then(|p| self.handle(p)) else { return false };
        let (t, mut z) = (Instant::now(), self.board.zones[i].clone());
        let done = z.remove_corner(r, k);
        self.zone_edited(i, z, done, t, None);
        true
    }

    /// After an edit of zone `i` into `z` that `done` says took: puts it in place, refilled
    /// alone, an undo step saying `what` it did (its outline's change, by default); else the
    /// status says why not.
    pub(crate) fn zone_edited(&mut self, i: usize, z: Zone, done: Result<(), &str>, t: Instant, what: Option<String>) {
        if let Err(e) = done {
            self.refused = Some((Pick::Zone(i), e.into()));
            return self.status = e.into();
        }
        let what = what.unwrap_or_else(|| tr(outline_change(&self.board.zones[i].outline, &z.outline)).into());
        let what = format!("{}: {what}", pick_name(&self.board, Pick::Zone(i)));
        let undo = self.board.set_zone(i, z);
        self.edited(undo, t, what);
    }

    /// Zone `i`'s outline and its handles: corners as squares, edges' middles as dots, the one
    /// under `pointer` lit; while one is dragged (`drag`: from, to), the outline as it will be,
    /// over the one it was.
    pub(crate) fn draw_handles(
        &self,
        ui: &egui::Ui,
        i: usize,
        drag: Option<(Pt, Pt)>,
        pointer: Option<Pt>,
        screen: impl Fn(Pt) -> egui::Pos2,
    ) {
        let (z, reach, cyan, p) = (&self.board.zones[i], self.reach(), theme::SIGNAL, ui.painter());
        let mut outline = z.outline.clone();
        let dragged = drag.and_then(|(from, to)| handle_at(z, from, reach).map(|h| (h, to)));
        match dragged {
            Some((Handle::Corner(r, k), to)) => outline[r][k] = to,
            Some((Handle::Edge(r, k), to)) => outline[r].insert(k + 1, to),
            None => {}
        }
        let hot = pointer.filter(|_| dragged.is_none()).and_then(|q| handle_at(z, q, reach));
        if hot.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if dragged.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            for ring in &z.outline {
                let pts: Vec<_> = ring.iter().chain(&ring[..1]).map(|&q| screen(q)).collect();
                p.extend(egui::Shape::dashed_line(&pts, (1.5, cyan.gamma_multiply(0.7)), 5.0, 4.0));
            }
        }
        for (r, ring) in outline.iter().enumerate() {
            p.add(egui::Shape::closed_line(ring.iter().map(|&q| screen(q)).collect(), (2.0, cyan)));
            for (k, &a) in ring.iter().enumerate() {
                let b = ring[(k + 1) % ring.len()];
                if dragged.is_none() && midway(a, b, reach) {
                    let fill = if hot == Some(Handle::Edge(r, k)) { cyan } else { theme::GROUND };
                    p.circle(screen(mid(a, b)), 3.5, fill, (1.5, cyan));
                }
                let fill = if hot == Some(Handle::Corner(r, k)) { cyan } else { theme::GROUND };
                p.rect(egui::Rect::from_center_size(screen(a), egui::vec2(8.0, 8.0)), 1.0, fill, (1.5, cyan), egui::StrokeKind::Middle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cuprita_core::board::Board;

    #[test]
    fn finds_corners_first_then_the_middles_of_long_edges() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal)) (zone (net 0) (layer "F.Cu")
              (polygon (pts (xy 0 0) (xy 10 0) (xy 10 1) (xy 0 1))) (polygon (pts (xy 2 0.2) (xy 4 0.2) (xy 4 0.8)))))"#,
        )
        .unwrap();
        let (z, at) = (&b.zones[0], |x: f64, y: f64| Pt { x: (x * 1e6) as i32, y: (y * 1e6) as i32 });
        let reach = 300_000;
        assert_eq!(handle_at(z, at(10.1, 0.1), reach), Some(Handle::Corner(0, 1)));
        assert_eq!(handle_at(z, at(5.0, 0.15), reach), Some(Handle::Edge(0, 0)));
        assert_eq!(handle_at(z, at(3.9, 0.25), reach), Some(Handle::Corner(1, 1)));
        // A 1 mm edge is too short for a middle handle at this reach; nothing is near the centre.
        assert_eq!((handle_at(z, at(10.0, 0.5), reach), handle_at(z, at(7.0, 0.5), reach)), (None, None));
        assert_eq!(handle_at(z, at(3.0, 0.2), reach), Some(Handle::Edge(1, 0)));
    }

    #[test]
    fn names_what_an_outline_edit_did() {
        let ring = |n: usize| vec![Pt::default(); n];
        let did = |old: &[Vec<Pt>], new: &[Vec<Pt>]| outline_change(old, new);
        assert_eq!(
            [did(&[ring(4)], &[ring(4), ring(3)]), did(&[ring(4), ring(3)], &[ring(4)]), did(&[ring(4)], &[ring(5)])],
            ["cutout added", "cutout removed", "corner added"]
        );
        assert_eq!([did(&[ring(4)], &[ring(3)]), did(&[ring(4)], &[ring(4)])], ["corner removed", "corner moved"]);
    }
}
