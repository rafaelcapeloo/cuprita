//! Items picked on a board, together: those a box takes, where they lie, and moving, turning,
//! flipping and deleting them as one edit, its patch undoing it all (`history.rs`).

use crate::board::{Board, Footprint, Geom, LayerId, LayerMask, Nm, Pt, Track, arc_points, crosses, nearest, rotate, union, upright};
use crate::history::{Patch, Pick};
use crate::polygon::inside;
use crate::{font, polygon};

impl Board {
    /// The items a box from `a` to `b` holds whole, or (`touching`) also those it touches, as
    /// KiCad's selection box takes them drawn left to right, or right to left: parts by their
    /// box, the tracks, vias and zones on the copper `layers` by their lines, round and outline,
    /// the board's own texts and drawings on the layers `shown` by their boxes and lines.
    pub fn items_in(&self, a: Pt, b: Pt, touching: bool, layers: &[LayerId], shown: LayerMask) -> Vec<Pick> {
        let area = (Pt { x: a.x.min(b.x), y: a.y.min(b.y) }, Pt { x: a.x.max(b.x), y: a.y.max(b.y) });
        let holds = |p: &Pt| (area.0.x..=area.1.x).contains(&p.x) && (area.0.y..=area.1.y).contains(&p.y);
        let meets = |(lo, hi): (Pt, Pt)| lo.x <= area.1.x && hi.x >= area.0.x && lo.y <= area.1.y && hi.y >= area.0.y;
        // A path's points all in it, or one of its stretches across it.
        let takes = |pts: &[Pt], closed: bool| match touching {
            false => pts.iter().all(holds),
            true => stretches(pts, closed).any(|(p, q)| crosses(area, p, q)),
        };
        let on = |m: LayerMask| layers.iter().any(|&l| m & 1 << l != 0);
        let parts = (0..self.footprints.len()).filter(|&i| {
            let (lo, hi) = self.footprints[i].bounds;
            if touching { meets((lo, hi)) } else { holds(&lo) && holds(&hi) }
        });
        let tracks = (0..self.tracks.len()).filter(|&i| on(1 << self.tracks[i].layer) && takes(&path(&self.tracks[i]), false));
        let vias = (0..self.vias.len()).filter(|&i| {
            let (v, r) = (&self.vias[i], self.vias[i].size / 2);
            let round = (Pt { x: v.at.x - r, y: v.at.y - r }, Pt { x: v.at.x + r, y: v.at.y + r });
            on(v.layers) && if touching { meets(round) } else { holds(&round.0) && holds(&round.1) }
        });
        let zones = (0..self.zones.len()).filter(|&i| {
            let z = &self.zones[i];
            let rings = z.outline.iter().filter(|r| !r.is_empty());
            on(z.layers) && if touching { rings.clone().any(|r| takes(r, true)) } else { rings.clone().all(|r| takes(r, false)) }
        });
        let (shapes, texts) = if shown == 0 { Default::default() } else { self.own() };
        let texts = texts.into_iter().filter(|&i| {
            let (lo, hi) = font::bounds(&self.texts[i]);
            shown >> self.texts[i].layer & 1 != 0 && if touching { meets((lo, hi)) } else { holds(&lo) && holds(&hi) }
        });
        let shapes =
            shapes.into_iter().filter(|&i| shown >> self.shapes[i].layer & 1 != 0 && takes(&polygon::path(&self.shapes[i].geom), false));
        let picks = parts.map(Pick::Footprint).chain(tracks.map(Pick::Track)).chain(vias.map(Pick::Via));
        picks.chain(zones.map(Pick::Zone)).chain(texts.map(Pick::Text)).chain(shapes.map(Pick::Shape)).collect()
    }

    /// The board's own text or drawing at `p` (within `tol`) on the layers `shown`: the text
    /// whose box holds it, else the drawing whose line it is on (a filled one, inside it too).
    pub fn drawing_at(&self, p: Pt, tol: Nm, shown: LayerMask) -> Option<Pick> {
        let (shapes, texts) = self.own();
        let text = texts.into_iter().find(|&i| shown >> self.texts[i].layer & 1 != 0 && self.hits(Pick::Text(i), p, tol));
        let shape = || shapes.into_iter().find(|&i| shown >> self.shapes[i].layer & 1 != 0 && self.hits(Pick::Shape(i), p, tol));
        text.map(Pick::Text).or_else(|| shape().map(Pick::Shape))
    }

    /// The part's text (its reference, its value) whose box holds `p` (within `tol`) on the
    /// layers `shown`: picked alone, it moves, turns and is sized apart from its part.
    pub fn part_text_at(&self, p: Pt, tol: Nm, shown: LayerMask) -> Option<Pick> {
        let parts = self.footprints.iter().flat_map(|f| f.texts.clone());
        parts.filter(|&i| shown >> self.texts[i].layer & 1 != 0).map(Pick::Text).find(|&t| self.hits(t, p, tol))
    }

    /// The part text `i` is one of (its reference, its value), `None` for one of the board's own.
    pub fn text_part(&self, i: usize) -> Option<usize> {
        // Parts' texts lie in the parts' order.
        let f = self.footprints.partition_point(|f| f.texts.end <= i);
        self.footprints.get(f).is_some_and(|f| f.texts.contains(&i)).then_some(f)
    }

    /// The line that marks a picked text or drawing: round the text's box, along the drawing.
    pub fn mark(&self, pick: Pick) -> Vec<Pt> {
        match pick {
            Pick::Text(i) => {
                let (lo, hi) = font::bounds(&self.texts[i]);
                vec![lo, Pt { x: hi.x, y: lo.y }, hi, Pt { x: lo.x, y: hi.y }, lo]
            }
            Pick::Shape(i) => polygon::path(&self.shapes[i].geom),
            _ => vec![],
        }
    }

    /// The box around the picked items: parts' boxes, tracks' and zones' corners, vias' centres;
    /// `None` when none is picked.
    pub fn extent(&self, picks: &[Pick]) -> Option<(Pt, Pt)> {
        let corners = |&pick: &Pick| match pick {
            Pick::Footprint(i) => vec![self.footprints[i].bounds.0, self.footprints[i].bounds.1],
            Pick::Track(i) => path(&self.tracks[i]),
            Pick::Via(i) => vec![self.vias[i].at],
            Pick::Zone(i) => self.zones[i].outline.concat(),
            Pick::Text(i) => <[Pt; 2]>::from(font::bounds(&self.texts[i])).to_vec(),
            Pick::Shape(i) => polygon::path(&self.shapes[i].geom),
        };
        picks.iter().flat_map(corners).map(|p| (p, p)).reduce(union)
    }

    /// Whether `p` (within `tol`) is on the picked item: in a part's box, on a track or via,
    /// inside a zone's outline (not a cutout) or on it.
    pub fn hits(&self, pick: Pick, p: Pt, tol: Nm) -> bool {
        let d = |q: Pt| (q.x as f64 - p.x as f64).hypot(q.y as f64 - p.y as f64);
        let near = |pts: &[Pt], closed: bool, reach: Nm| stretches(pts, closed).any(|(a, b)| d(nearest(a, b, p)) <= (reach + tol) as f64);
        match pick {
            Pick::Footprint(i) => self.footprints[i].holds(p, tol),
            Pick::Track(i) => near(&path(&self.tracks[i]), false, self.tracks[i].width / 2),
            Pick::Via(i) => d(self.vias[i].at) <= (self.vias[i].size / 2 + tol) as f64,
            Pick::Zone(i) => {
                let rings = &self.zones[i].outline;
                rings.iter().filter(|r| inside(p, r)).count() % 2 == 1 || rings.iter().any(|r| near(r, true, 0))
            }
            Pick::Text(i) => {
                let (lo, hi) = font::bounds(&self.texts[i]);
                (lo.x - tol..=hi.x + tol).contains(&p.x) && (lo.y - tol..=hi.y + tol).contains(&p.y)
            }
            Pick::Shape(i) => {
                let (s, path) = (&self.shapes[i], polygon::path(&self.shapes[i].geom));
                near(&path, false, s.width / 2)
                    || s.filled && !matches!(s.geom, Geom::Segment { .. } | Geom::Arc { .. }) && inside(p, &path)
            }
        }
    }

    /// Moves the picked items by `by`: parts with their pads, graphics and texts, tracks, vias,
    /// zones with their fills, the board's own texts and drawings, a part's text alone (it stays
    /// there as its part moves on). Returns the patch that undoes it.
    pub fn move_items(&mut self, picks: &[Pick], by: Pt) -> Patch {
        self.transform(picks, 0.0, |p| Pt { x: p.x.saturating_add(by.x), y: p.y.saturating_add(by.y) })
    }

    /// Turns the picked items `deg` degrees about `about`, counter-clockwise on screen as KiCad
    /// turns (exactly, a right angle), parts' angles along. Returns the patch that undoes it.
    pub fn turn_items(&mut self, picks: &[Pick], about: Pt, deg: f64) -> Patch {
        self.transform(picks, deg, |p| {
            let r = rotate(Pt { x: p.x - about.x, y: p.y - about.y }, deg);
            Pt { x: about.x + r.x, y: about.y + r.y }
        })
    }

    /// Puts the picked parts on the other side as KiCad's Flip does, top to bottom about the line
    /// through `about` (their middle), so together they mirror; other picks stay. Returns the patch
    /// that undoes it.
    pub fn flip_items(&mut self, picks: &[Pick], about: Pt) -> Patch {
        let parts: Vec<usize> =
            self.picked(picks).into_iter().filter_map(|p| if let Pick::Footprint(i) = p { Some(i) } else { None }).collect();
        let undo = parts.iter().map(|&i| Patch::footprint(self, i)).collect();
        for i in parts {
            self.flip_footprint(i);
            let f = &self.footprints[i];
            self.place_footprint(i, Pt { x: f.at.x, y: (2 * about.y as i64 - f.at.y as i64) as Nm }, f.angle);
        }
        undo
    }

    /// Deletes the picked items: parts with their pads, graphics and texts, tracks, vias, zones
    /// with their fills; a part's text picked alone is hidden (its part keeps it: saved hidden,
    /// as Tidy texts hides one). Returns the patch that undoes it.
    pub fn delete_items(&mut self, picks: &[Pick]) -> Patch {
        let part_text = |p: &Pick| if let Pick::Text(i) = *p { self.text_part(i).map(|f| (f, i)) } else { None };
        let (hide, mut rest): (Vec<Pick>, Vec<Pick>) = self.picked(picks).into_iter().partition(|p| part_text(p).is_some());
        let hide: Vec<(usize, usize)> = hide.iter().filter_map(part_text).collect();
        let mut parts: Vec<usize> = hide.iter().map(|&(f, _)| f).collect();
        parts.dedup();
        let kept =
            |f: &usize| self.footprints[*f].texts.clone().map(|i| (!hide.contains(&(*f, i))).then(|| self.texts[i].clone())).collect();
        let hidden = self.set_texts(&parts, parts.iter().map(kept).collect()).0;
        // The rest, the board's own texts where they are with those gone.
        for p in &mut rest {
            if let Pick::Text(i) = p {
                *i -= hide.iter().filter(|&&(_, h)| h < *i).count();
            }
        }
        let gone = rest.into_iter().map(|p| Patch::delete(self, p)).collect();
        self.apply(gone).then(hidden)
    }

    /// The picks in order, each once; a part's text picked with its part goes as the part does.
    fn picked(&self, picks: &[Pick]) -> Vec<Pick> {
        let mut picks = picks.to_vec();
        picks.sort_unstable();
        picks.dedup();
        let with_part =
            |p: &Pick| matches!(*p, Pick::Text(i) if self.text_part(i).is_some_and(|f| picks.binary_search(&Pick::Footprint(f)).is_ok()));
        picks.iter().filter(|p| !with_part(p)).copied().collect()
    }

    /// Shows hidden text `k` of part `part` again (`Footprint::hidden`: the file's and those an
    /// edit hid), where it would be, after the part's other texts. Returns its pick and the patch
    /// that undoes it.
    pub fn show_text(&mut self, part: usize, k: usize) -> (Pick, Patch) {
        let f = &self.footprints[part];
        let (mut hidden, mut texts) = (f.hidden.clone(), self.texts[f.texts.clone()].to_vec());
        texts.push(hidden.remove(k));
        let shown = f.texts.start..f.texts.end + 1;
        let patch = Patch::texts(self, part, Footprint { texts: shown.clone(), hidden, ..f.clone() }, texts);
        (Pick::Text(shown.end - 1), self.apply(patch))
    }

    /// Takes every picked item's points through `to` (a part's anchor, then turned `turn` degrees
    /// with its own). Returns the patch that undoes it.
    fn transform(&mut self, picks: &[Pick], turn: f64, to: impl Fn(Pt) -> Pt) -> Patch {
        let picks = self.picked(picks);
        let undo = picks.iter().map(|&p| Patch::item(self, p)).collect();
        for pick in picks {
            match pick {
                Pick::Footprint(i) => {
                    let f = &self.footprints[i];
                    self.place_footprint(i, to(f.at), f.angle + turn);
                }
                Pick::Track(i) => {
                    let t = &mut self.tracks[i];
                    (t.a, t.b, t.mid) = (to(t.a), to(t.b), t.mid.map(&to));
                }
                Pick::Via(i) => self.vias[i].at = to(self.vias[i].at),
                Pick::Zone(i) => {
                    let z = &mut self.zones[i];
                    z.outline.iter_mut().flatten().for_each(|p| *p = to(*p));
                    self.fills[z.fills.clone()].iter_mut().flat_map(|f| &mut f.pts).for_each(|p| *p = to(*p));
                }
                Pick::Text(i) => {
                    let t = &mut self.texts[i];
                    t.at = to(t.at);
                    if turn != 0.0 {
                        // A part's locked text stays upright, as when its part turns.
                        t.angle = if t.upright { upright(t.angle + turn) } else { (t.angle + turn).rem_euclid(360.0) };
                    }
                }
                Pick::Shape(i) => match &mut self.shapes[i].geom {
                    Geom::Segment { a, b } => (*a, *b) = (to(*a), to(*b)),
                    Geom::Arc { start, mid, end } => (*start, *mid, *end) = (to(*start), to(*mid), to(*end)),
                    Geom::Circle { center, .. } => *center = to(*center),
                    Geom::Poly(pts) => pts.iter_mut().for_each(|p| *p = to(*p)),
                },
            }
        }
        undo
    }
}

/// A track's centreline: its ends, an arc's points along it.
fn path(t: &Track) -> Vec<Pt> {
    t.mid.map_or_else(|| vec![t.a, t.b], |m| arc_points(t.a, m, t.b))
}

/// The stretches between the points, and back to the first when `closed`.
fn stretches(pts: &[Pt], closed: bool) -> impl Iterator<Item = (Pt, Pt)> + '_ {
    let back = pts.last().zip(pts.first()).filter(|_| closed && pts.len() > 2).map(|(&a, &b)| (a, b));
    pts.windows(2).map(|w| (w[0], w[1])).chain(back)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, Pad, Text};
    use crate::history::History;
    use crate::{drc, ratsnest};

    /// R1 and C1 joined by a track; a track, via and track from R1 on to the left and down; a
    /// track past the corner of a box round the parts; a zone below the parts.
    const BOARD: &str = r#"(kicad_pcb (version 20240108) (generator "pcbnew")
	(layers (0 "F.Cu" signal) (31 "B.Cu" signal) (36 "B.SilkS" user) (37 "F.SilkS" user))
	(net 0 "") (net 1 "A") (net 2 "B")
	(footprint "R" (layer "F.Cu") (at 10 10)
		(property "Reference" "R1" (at 0 -1 0) (layer "F.SilkS"))
		(pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net 1 "A"))
		(pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 2 "B")))
	(footprint "C" (layer "F.Cu") (at 20 10)
		(property "Reference" "C1" (at 0 -1 0) (layer "F.SilkS"))
		(pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net 2 "B"))
		(pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
	(segment (start 11 10) (end 19 10) (width 0.25) (layer "F.Cu") (net 2))
	(segment (start 9 10) (end 5 10) (width 0.25) (layer "F.Cu") (net 1))
	(segment (start 5 10) (end 5 30) (width 0.25) (layer "B.Cu") (net 1))
	(segment (start 20 30) (end 30 18) (width 0.25) (layer "F.Cu") (net 2))
	(via (at 5 10) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1))
	(zone (net 1) (net_name "A") (layer "F.Cu") (hatch edge 0.5) (connect_pads (clearance 0.5)) (min_thickness 0.25) (filled_areas_thickness no)
		(polygon (pts (xy 8 15) (xy 22 15) (xy 22 20) (xy 8 20)))
		(filled_polygon (layer "F.Cu") (pts (xy 8.5 15.5) (xy 21.5 15.5) (xy 21.5 19.5) (xy 8.5 19.5))))
)"#;

    fn pt(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    /// The board's parts, pads and texts, and the rest as text.
    fn everything(b: &Board) -> (Vec<Footprint>, Vec<Pad>, Vec<Text>, String) {
        (b.footprints.clone(), b.pads.clone(), b.texts.clone(), format!("{:?}", (&b.tracks, &b.vias, &b.zones, &b.fills)))
    }

    fn sorted(mut v: Vec<drc::Violation>) -> Vec<drc::Violation> {
        v.sort_unstable_by_key(|v| (v.kind, v.items));
        v
    }

    #[test]
    fn a_box_takes_what_it_holds_drawn_left_to_right_what_it_touches_right_to_left() {
        let b = Board::parse(BOARD).unwrap();
        let both: Vec<LayerId> = b.stackup.iter().map(|c| c.layer).collect();
        let back = [both[1]];
        let (a, z) = (pt(7.0, 8.0), pt(23.0, 21.0));
        // The parts, the track between them and the zone lie in it; the track to the via crosses
        // its edge; the one past its corner, whose box meets it, doesn't cross it.
        let held = [Pick::Footprint(0), Pick::Footprint(1), Pick::Track(0)];
        assert_eq!(b.items_in(a, z, false, &both, 0), [&held[..], &[Pick::Zone(0)]].concat());
        assert_eq!(b.items_in(z, a, true, &both, 0), [&held[..], &[Pick::Track(1), Pick::Zone(0)]].concat());
        // Copper hidden is never taken; a box round the via takes it, or it and the tracks it ends.
        assert_eq!(b.items_in(a, z, true, &back, 0), [Pick::Footprint(0), Pick::Footprint(1)]);
        let (lo, hi) = (pt(4.5, 9.5), pt(5.5, 10.5));
        assert_eq!(
            (b.items_in(lo, hi, false, &both, 0), b.items_in(lo, hi, true, &both, 0)),
            (vec![Pick::Via(0)], vec![Pick::Track(1), Pick::Track(2), Pick::Via(0)])
        );
        // What they span, and where a press takes one up.
        assert_eq!((b.extent(&[Pick::Track(1), Pick::Zone(0)]), b.extent(&[])), (Some((pt(5.0, 10.0), pt(22.0, 20.0))), None));
        let hits = |pick, x, y| b.hits(pick, pt(x, y), 50_000);
        assert!(hits(Pick::Zone(0), 15.0, 17.0) && hits(Pick::Zone(0), 22.04, 17.0) && !hits(Pick::Zone(0), 23.0, 17.0));
        assert!(hits(Pick::Track(1), 7.0, 10.15) && !hits(Pick::Track(1), 7.0, 10.3) && hits(Pick::Via(0), 5.4, 10.0));
    }

    #[test]
    fn turns_about_a_grid_point_exactly_and_back_one_step_at_a_time() {
        let (orig, mut b, mut h) = (Board::parse(BOARD).unwrap(), Board::parse(BOARD).unwrap(), History::default());
        let copper: Vec<LayerId> = b.stackup.iter().map(|c| c.layer).collect();
        let all = b.items_in(pt(0.0, 0.0), pt(40.0, 40.0), false, &copper, 0);
        assert_eq!(all.len(), 8);
        let (mut vs, mut rs) = (drc::check(&b), b.ratsnest());
        for k in 1..=4 {
            let undo = b.turn_items(&all, pt(15.0, 15.0), 90.0);
            // Each quarter turn re-checks what it moved: as a full check finds it.
            vs = drc::recheck(&b, vs, &undo);
            rs = ratsnest::recheck(&b, rs, &undo);
            assert_eq!((sorted(vs.clone()), &rs), (sorted(drc::check(&b)), &b.ratsnest()), "turn {k}");
            h.push(undo, format!("turn {k}"));
            if k == 1 {
                // R1 from (10, 10) to (10, 20), turned; the track between the parts upright.
                let (r1, t) = (&b.footprints[0], &b.tracks[0]);
                assert_eq!(
                    (r1.at, r1.angle, b.pads[0].at, t.a, t.b),
                    (pt(10.0, 20.0), 90.0, pt(10.0, 21.0), pt(10.0, 19.0), pt(10.0, 11.0))
                );
                assert_eq!(b.zones[0].outline[0][0], pt(15.0, 22.0));
            }
        }
        // Four quarter turns: the board as it was; each turn one undo step.
        assert_eq!(everything(&b), everything(&orig));
        for _ in 0..3 {
            h.undo(&mut b);
        }
        assert_eq!((b.footprints[0].at, b.footprints[0].angle), (pt(10.0, 20.0), 90.0));
        h.undo(&mut b);
        assert_eq!(everything(&b), everything(&orig));
    }

    #[test]
    fn flips_parts_together_about_their_middle() {
        let (orig, mut b) = (Board::parse(BOARD).unwrap(), Board::parse(BOARD).unwrap());
        // The track and via stay; the parts, each flipped as KiCad's Flip, mirror about y = 12.
        let undo = b.flip_items(&[Pick::Footprint(1), Pick::Track(0), Pick::Footprint(0), Pick::Via(0)], pt(15.0, 12.0));
        let side = |b: &Board| b.footprints.iter().map(|f| (f.at, f.back)).collect::<Vec<_>>();
        assert_eq!(side(&b), [(pt(10.0, 14.0), true), (pt(20.0, 14.0), true)]);
        assert_eq!((&b.tracks, &b.vias, b.layers[b.texts[0].layer as usize].as_str()), (&orig.tracks, &orig.vias, "B.SilkS"));
        let redo = b.apply(undo);
        assert_eq!(everything(&b), everything(&orig));
        b.apply(redo);
        b.flip_items(&[Pick::Footprint(0), Pick::Footprint(1)], pt(15.0, 12.0));
        assert_eq!(everything(&b), everything(&orig));
    }

    #[test]
    fn saves_what_moved_turned_and_went_together() {
        let mut b = Board::parse(BOARD).unwrap();
        let picks = [Pick::Footprint(1), Pick::Track(0), Pick::Track(3), Pick::Via(0), Pick::Zone(0)];
        let moved = b.move_items(&picks, pt(1.0, -2.0));
        let turned = b.turn_items(&picks, pt(20.0, 10.0), 90.0);
        let out = b.write(BOARD).unwrap();
        // Read back, every item is where the board has it: the zone's outline written in place of
        // its own, its fill too; the tracks and via moved after the file's last track.
        let back = Board::parse(&out).unwrap();
        let tracks = |b: &Board| {
            let mut t = b.tracks.clone();
            t.sort_by_key(|t| (t.a.x, t.a.y, t.b.x, t.b.y));
            t
        };
        assert_eq!((&back.footprints[1].at, back.footprints[1].angle, &back.pads), (&b.footprints[1].at, b.footprints[1].angle, &b.pads));
        assert_eq!((tracks(&back), &back.vias, &back.zones[0].outline, &back.fills), (tracks(&b), &b.vias, &b.zones[0].outline, &b.fills));
        assert!(out.contains("\t\t(polygon\n\t\t\t(pts\n\t\t\t\t(xy 23 21) (xy 23 7) (xy 28 7) (xy 28 21)\n\t\t\t)\n\t\t)"), "{out}");
        // Deleted, they are gone from the text; undone, it all writes as it was read.
        let deleted = b.delete_items(&picks);
        let back = Board::parse(&b.write(BOARD).unwrap()).unwrap();
        assert_eq!((back.footprints.len(), back.tracks.len(), back.vias.len(), back.zones.len()), (1, 2, 0, 0));
        for undo in [deleted, turned, moved] {
            b.apply(undo);
        }
        assert_eq!(b.write(BOARD).unwrap(), BOARD);
    }
}
