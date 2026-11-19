//! Laid tracks edited: the run a track is part of (KiCad's U), their width set, a segment dragged
//! with its neighbours following. A run is the segments of a net joined end to end on a layer,
//! between pads, vias and branches.

use crate::board::{Board, LayerId, Nm, Pt, Track, nearest};
use crate::history::{Patch, Pick};
use crate::tr::tr;

/// What a drag makes: the tracks changed (by index) with their new shape, and those added.
pub type Dragged = (Vec<(usize, Track)>, Vec<Track>);

impl Board {
    /// The run track `i` is part of: it and the segments of its net joined end to end on its
    /// layer, up to a pad, a via, a branch (three ends or more meeting, or an end on another
    /// track's middle) or a loose end; `i` first, the others as they are reached.
    pub fn run(&self, i: usize) -> Vec<usize> {
        let t = &self.tracks[i];
        let ours = |k: usize| self.tracks[k].layer == t.layer && self.tracks[k].net == t.net;
        let mut run = vec![i];
        let mut todo = vec![(i, t.a), (i, t.b)];
        while let Some((from, end)) = todo.pop() {
            if self.joins_more(end, t.layer) {
                continue;
            }
            let at: Vec<usize> =
                (0..self.tracks.len()).filter(|&k| ours(k) && [self.tracks[k].a, self.tracks[k].b].contains(&end)).collect();
            let middle = |k: usize| ours(k) && !at.contains(&k) && nearest(self.tracks[k].a, self.tracks[k].b, end) == end;
            // Exactly this segment and one more end here, and none passes through.
            let [a, b] = at[..] else { continue };
            if (0..self.tracks.len()).any(middle) {
                continue;
            }
            let next = if a == from { b } else { a };
            if !run.contains(&next) {
                run.push(next);
                let n = &self.tracks[next];
                todo.push((next, if n.a == end { n.b } else { n.a }));
            }
        }
        run
    }

    /// Whether a pad or via on `layer` takes in `p`, where a run ends.
    fn joins_more(&self, p: Pt, layer: LayerId) -> bool {
        let d = |q: Pt| (q.x as f64 - p.x as f64).hypot(q.y as f64 - p.y as f64);
        self.pads.iter().any(|q| q.layers >> layer & 1 != 0 && q.holds(p, 0))
            || self.vias.iter().any(|v| v.layers >> layer & 1 != 0 && d(v.at) <= (v.size / 2) as f64)
    }

    /// Gives tracks `ts` the width `width`. Returns the patch that undoes it.
    pub fn set_widths(&mut self, ts: &[usize], width: Nm) -> Patch {
        let undo = ts.iter().map(|&i| Patch::item(self, Pick::Track(i))).collect();
        for &i in ts {
            self.tracks[i].width = width;
        }
        undo
    }

    /// What dragging track `i` by `by` makes, as KiCad's drag does: only the part of `by` across it
    /// counts; each end slides along its neighbour, which keeps its direction, to where it meets
    /// the track's new line; an end that can't (on a pad, a via, a branch, loose, or with a
    /// neighbour along it or curved) gets a new stretch at 45° to the new line. The tracks changed
    /// with their new shape, and those added; or why not: an arc, or dragged so far that a
    /// neighbour or the track itself would turn round.
    pub fn dragged(&self, i: usize, by: Pt) -> Result<Dragged, String> {
        let t = &self.tracks[i];
        if t.mid.is_some() {
            return Err(tr("an arc can't be dragged yet: delete it and route it again").into());
        }
        let v = |p: Pt| (p.x as f64, p.y as f64);
        let at = |(x, y): (f64, f64)| Pt { x: x.round() as Nm, y: y.round() as Nm };
        let ((ax, ay), (bx, by_)) = (v(t.a), v(t.b));
        let length = (bx - ax).hypot(by_ - ay);
        let (u, n) = (((bx - ax) / length, (by_ - ay) / length), (-(by_ - ay) / length, (bx - ax) / length));
        let across = by.x as f64 * n.0 + by.y as f64 * n.1;
        if length == 0.0 || across.abs() < 1.0 {
            return Ok((vec![], vec![]));
        }
        let cross = |p: (f64, f64), q: (f64, f64)| p.0 * q.1 - p.1 * q.0;
        let (mut changed, mut added, mut ends) = (vec![], vec![], [(ax, ay), (bx, by_)]);
        for (side, end) in [t.a, t.b].into_iter().enumerate() {
            let moved = (v(end).0 + n.0 * across, v(end).1 + n.1 * across);
            match self.slider(i, end) {
                Some(k) => {
                    // Where its line, from its far end through this one, meets the new line.
                    let nb = &self.tracks[k];
                    let far = v(if nb.a == end { nb.b } else { nb.a });
                    let dir = (v(end).0 - far.0, v(end).1 - far.1);
                    let s = cross((moved.0 - far.0, moved.1 - far.1), u) / cross(dir, u);
                    if s <= 0.0 {
                        return Err(tr("dragged that far, the track beside it would turn round: drag it less").into());
                    }
                    let joint = (far.0 + s * dir.0, far.1 + s * dir.1);
                    changed.push((k, if nb.a == end { Track { a: at(joint), ..*nb } } else { Track { b: at(joint), ..*nb } }));
                    ends[side] = joint;
                }
                // An end on nothing goes along.
                None if self.loose(i, end) => ends[side] = moved,
                None => {
                    // A 45° stretch from where the end stays to the new line, toward the middle.
                    let toward = if side == 0 { 1.0 } else { -1.0 } * across.abs();
                    let joint = (moved.0 + u.0 * toward, moved.1 + u.1 * toward);
                    added.push(Track { a: end, b: at(joint), mid: None, ..*t });
                    ends[side] = joint;
                }
            }
        }
        // The track keeps its direction between its new ends, else it was dragged too far.
        let ([a, b], (x, y)) = (ends, (ends[1].0 - ends[0].0, ends[1].1 - ends[0].1));
        if x * u.0 + y * u.1 <= 0.0 {
            return Err(tr("dragged that far, the track would turn round: drag it less").into());
        }
        changed.push((i, Track { a: at(a), b: at(b), ..*t }));
        Ok((changed, added))
    }

    /// The one track that slides along with track `i`'s end `end` when `i` is dragged: of its net
    /// and layer, ending there, straight and not along `i`; none where a pad, via, branch or
    /// middle of another track is.
    fn slider(&self, i: usize, end: Pt) -> Option<usize> {
        let t = &self.tracks[i];
        if self.joins_more(end, t.layer) {
            return None;
        }
        let ours = |k: usize| k != i && self.tracks[k].layer == t.layer && self.tracks[k].net == t.net;
        let at: Vec<usize> = (0..self.tracks.len()).filter(|&k| ours(k) && [self.tracks[k].a, self.tracks[k].b].contains(&end)).collect();
        let through =
            (0..self.tracks.len()).any(|k| ours(k) && !at.contains(&k) && nearest(self.tracks[k].a, self.tracks[k].b, end) == end);
        let [k] = at[..] else { return None };
        let nb = &self.tracks[k];
        let (d1, d2) = ((t.b.x - t.a.x) as f64, (t.b.y - t.a.y) as f64);
        let (e1, e2) = ((nb.b.x - nb.a.x) as f64, (nb.b.y - nb.a.y) as f64);
        let along = (d1 * e2 - d2 * e1).abs() < 1e-6 * d1.hypot(d2) * e1.hypot(e2);
        (!through && nb.mid.is_none() && !along).then_some(k)
    }

    /// Whether track `i`'s end `end` is on nothing: no pad, via or other track of its net there.
    fn loose(&self, i: usize, end: Pt) -> bool {
        let t = &self.tracks[i];
        let touches = |(k, o): (usize, &Track)| k != i && o.layer == t.layer && o.net == t.net && nearest(o.a, o.b, end) == end;
        !self.joins_more(end, t.layer) && !self.tracks.iter().enumerate().any(touches)
    }

    /// Drags track `i` by `by` as `dragged` says. Returns the patch that undoes it.
    pub fn drag_segment(&mut self, i: usize, by: Pt) -> Result<Patch, String> {
        let (changed, added) = self.dragged(i, by)?;
        let undo = std::iter::once(Patch::added(self)).chain(changed.iter().map(|&(k, _)| Patch::item(self, Pick::Track(k)))).collect();
        for (k, t) in changed {
            self.tracks[k] = t;
        }
        self.tracks.extend(added);
        Ok(undo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(items: &str) -> Board {
        Board::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "B")
              (footprint "R" (at 0 0) (property "Reference" "R1")
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 20 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
              {items})"#
        ))
        .unwrap()
    }

    fn seg(a: (i32, i32), b: (i32, i32), layer: &str, net: u32) -> String {
        format!("(segment (start {} {}) (end {} {}) (width 0.25) (layer \"{layer}\") (net {net}))", a.0, a.1, b.0, b.1)
    }

    #[test]
    fn a_run_goes_end_to_end_up_to_pads_vias_and_branches() {
        // Pad 1 to pad 2 in three segments (0, 1, 2), then a spur off the far pad (3); a segment of
        // net B (4) and one on B.Cu (5) touch the run but aren't of it.
        let items = [
            seg((0, 0), (5, 0), "F.Cu", 1),
            seg((5, 0), (10, 5), "F.Cu", 1),
            seg((10, 5), (20, 0), "F.Cu", 1),
            seg((20, 0), (25, 0), "F.Cu", 1),
            seg((5, 0), (5, 9), "F.Cu", 2),
            seg((10, 5), (10, 9), "B.Cu", 1),
        ]
        .concat();
        let b = board(&items);
        let mut run = b.run(1);
        run.sort();
        assert_eq!(run, [0, 1, 2]);
        assert_eq!(b.run(3), [3]);
        // A third segment of the net meeting at (5, 0) makes it a branch: the run stops there.
        let b = board(&(items + &seg((5, 0), (5, -9), "F.Cu", 1)));
        let mut run = b.run(2);
        run.sort();
        assert_eq!(run, [1, 2]);
        // So does a via, and an end on another segment's middle.
        let via = "(via (at 10 5) (size 0.8) (drill 0.4) (layers \"F.Cu\" \"B.Cu\") (net 1))";
        let b = board(
            &[seg((0, 0), (5, 0), "F.Cu", 1), seg((5, 0), (10, 5), "F.Cu", 1), seg((10, 5), (20, 0), "F.Cu", 1), via.into()].concat(),
        );
        assert_eq!(b.run(0), [0, 1]);
        let b = board(&[seg((0, 0), (10, 0), "F.Cu", 1), seg((10, 0), (20, 0), "F.Cu", 1), seg((5, 0), (5, 5), "F.Cu", 1)].concat());
        assert_eq!(b.run(2), [2]);
    }

    fn ends(b: &Board) -> Vec<((i32, i32), (i32, i32))> {
        let mm = |p: Pt| (p.x / 1_000_000, p.y / 1_000_000);
        b.tracks.iter().map(|t| (mm(t.a), mm(t.b))).collect()
    }

    #[test]
    fn a_dragged_segment_slides_along_its_neighbours() {
        // A jog from pad 1 to pad 2: up at 45°, across, down at 45°; the middle dragged 2 mm down.
        let items = [seg((0, 0), (5, 5), "F.Cu", 1), seg((5, 5), (15, 5), "F.Cu", 1), seg((15, 5), (20, 0), "F.Cu", 1)].concat();
        let mut b = board(&items);
        let before = ends(&b);
        let undo = b.drag_segment(1, Pt { x: 300_000, y: 2_000_000 }).unwrap();
        assert_eq!(ends(&b), [((0, 0), (7, 7)), ((7, 7), (13, 7)), ((13, 7), (20, 0))]);
        b.apply(undo);
        assert_eq!(ends(&b), before);
        // Dragged up past the pads' line, the neighbours would turn round: it says so.
        let Err(why) = b.drag_segment(1, Pt { x: 0, y: -6_000_000 }) else { panic!("dragged past its neighbours") };
        assert!(why.contains("would turn round"), "{why}");
        assert_eq!(ends(&b), before);
    }

    #[test]
    fn an_end_on_a_pad_gets_a_45_degree_stretch_and_a_loose_end_goes_along() {
        // From pad 1 out to nothing; dragged 3 mm down.
        let mut b = board(&seg((0, 0), (10, 0), "F.Cu", 1));
        let undo = b.drag_segment(0, Pt { x: 0, y: 3_000_000 }).unwrap();
        assert_eq!(ends(&b), [((3, 3), (10, 3)), ((0, 0), (3, 3))]);
        b.apply(undo);
        assert_eq!(ends(&b), [((0, 0), (10, 0))]);
    }

    #[test]
    fn sets_widths_as_one_undoable_step_and_saves_them() {
        let items = [seg((0, 0), (5, 0), "F.Cu", 1), seg((5, 0), (20, 0), "F.Cu", 1)].concat();
        let mut b = board(&items);
        let undo = b.set_widths(&[0, 1], 500_000);
        assert!(b.tracks.iter().all(|t| t.width == 500_000));
        // Saved, the file reads back with them so wide.
        let src = format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "B")
              (footprint "R" (at 0 0) (property "Reference" "R1")
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 20 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
              {items})"#
        );
        let back = Board::parse(&b.write(&src).unwrap()).unwrap();
        assert!(back.tracks.len() == 2 && back.tracks.iter().all(|t| t.width == 500_000));
        b.apply(undo);
        assert!(b.tracks.iter().all(|t| t.width == 250_000));
    }
}
