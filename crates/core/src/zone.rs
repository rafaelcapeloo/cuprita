//! A zone's outline edited: a corner moved, added on an edge or taken away, a cutout added. Each
//! edit keeps the outline simple, as KiCad wants it: no edge crossing or folding back on another,
//! every cutout inside the outline and apart from the others; else it changes nothing and says why.
//! `Board::set_zone` (`fill.rs`) puts the edited zone in place, filled.

use crate::board::{Pt, Zone};
use crate::drc::RBox;
use crate::polygon::inside;
use crate::tr::{t, tr};

/// Why a corner can't go where it was put, and what to do.
const CROSSES: &str = t!("That would make the zone's outline cross itself or a cutout: put the corner where its edges stay clear");
/// Why the outline keeps its corners.
const THREE: &str = t!("A zone keeps three corners or more: Delete with the pointer off its corners deletes the zone");
/// Why a cutout can't go where it was drawn, and what to do.
const CUTOUT: &str = t!("A cutout goes inside its zone, clear of the outline and the other cutouts: draw it there");

impl Zone {
    /// Moves corner `k` of ring `r` (0 the outline, then the cutouts) to `to`; dropped on a
    /// corner beside it, it goes (`remove_corner`).
    pub fn move_corner(&mut self, r: usize, k: usize, to: Pt) -> Result<(), &'static str> {
        let n = self.outline[r].len();
        if [k + n - 1, k + 1].iter().any(|&j| self.outline[r][j % n] == to) {
            return self.remove_corner(r, k);
        }
        self.edited(r, &[k + n - 1, k], CROSSES, |o| o[r][k] = to)
    }

    /// Adds a corner at `at` on the edge from corner `k` of ring `r` to the next.
    pub fn add_corner(&mut self, r: usize, k: usize, at: Pt) -> Result<(), &'static str> {
        self.edited(r, &[k, k + 1], CROSSES, |o| o[r].insert(k + 1, at))
    }

    /// Takes corner `k` of ring `r` away: the outline keeps three or more, a cutout left with
    /// two goes.
    pub fn remove_corner(&mut self, r: usize, k: usize) -> Result<(), &'static str> {
        match self.outline[r].len() {
            ..=3 if r == 0 => Err(tr(THREE)),
            ..=3 => {
                self.outline.remove(r);
                Ok(())
            }
            // The edge that joins the corners beside it.
            n => self.edited(r, &[k + n - 2], CROSSES, |o| {
                o[r].remove(k);
            }),
        }
    }

    /// Adds a cutout through `corners`: a hole in the zone, inside its outline, clear of it and
    /// of the other cutouts.
    pub fn add_cutout(&mut self, corners: Vec<Pt>) -> Result<(), &'static str> {
        let (r, edges) = (self.outline.len(), (0..corners.len()).collect::<Vec<_>>());
        self.edited(r, &edges, CUTOUT, |o| o.push(corners))
    }

    /// Makes `edit` to a copy of the outline and keeps it when the edges `edges` of ring `r`
    /// (edge `k` runs from corner `k` to the next, round the ring) cross or fold back on no other
    /// edge and the cutouts lie inside the outline, apart; else it says `why`.
    fn edited(&mut self, r: usize, edges: &[usize], why: &'static str, edit: impl FnOnce(&mut Vec<Vec<Pt>>)) -> Result<(), &'static str> {
        let mut o = self.outline.clone();
        edit(&mut o);
        let edge = |s: usize, j: usize| (o[s][j], o[s][(j + 1) % o[s].len()]);
        let on = |(a, b): (Pt, Pt), p: Pt| RBox::segment(a, b, 0).dist(&RBox::disc(p, 0)) == 0;
        let n = o[r].len();
        for k in edges.iter().map(|k| k % n) {
            let e = edge(r, k);
            for (s, j) in o.iter().enumerate().flat_map(|(s, ring)| (0..ring.len()).map(move |j| (s, j))).filter(|&sj| sj != (r, k)) {
                let f = edge(s, j);
                // Edges meeting at a corner may only touch there.
                let shared = if s != r {
                    None
                } else if j == (k + 1) % n {
                    Some(e.1)
                } else if k == (j + 1) % n {
                    Some(e.0)
                } else {
                    None
                };
                let touch = match shared {
                    Some(v) => [f.0, f.1].iter().any(|&p| p != v && on(e, p)) || [e.0, e.1].iter().any(|&p| p != v && on(f, p)),
                    None => RBox::segment(e.0, e.1, 0).dist(&RBox::segment(f.0, f.1, 0)) == 0,
                };
                if e.0 == e.1 || touch {
                    return Err(tr(why));
                }
            }
        }
        // With no edge crossing, a ring's first corner tells which side of another it lies on.
        let cutouts = || o.iter().enumerate().skip(1);
        let astray = match r {
            0 => cutouts().any(|(_, c)| !inside(c[0], &o[0])),
            _ => !inside(o[r][0], &o[0]) || cutouts().any(|(s, c)| s != r && (inside(o[r][0], c) || inside(c[0], &o[r]))),
        };
        if astray {
            return Err(tr(why));
        }
        self.outline = o;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;

    fn pt(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6) as i32, y: (y * 1e6) as i32 }
    }

    fn square(x: f64, y: f64, side: f64) -> Vec<Pt> {
        vec![pt(x, y), pt(x + side, y), pt(x + side, y + side), pt(x, y + side)]
    }

    /// A 10 mm square zone with a 2 mm square cutout at (2, 2).
    fn zone() -> Zone {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal)) (zone (net 0) (layer "F.Cu")
              (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10))) (polygon (pts (xy 2 2) (xy 4 2) (xy 4 4) (xy 2 4)))))"#,
        )
        .unwrap();
        b.zones[0].clone()
    }

    #[test]
    fn moves_adds_and_removes_corners_keeping_the_outline_simple() {
        let mut z = zone();
        z.move_corner(0, 2, pt(12.0, 12.0)).unwrap();
        assert_eq!(z.outline[0], [pt(0.0, 0.0), pt(10.0, 0.0), pt(12.0, 12.0), pt(0.0, 10.0)]);
        // Across the far edge, into the cutout, or so far in that the outline leaves it out: refused,
        // nothing changed.
        for (k, to) in [(0, pt(5.0, 12.0)), (0, pt(3.0, 3.0)), (2, pt(1.0, 1.0))] {
            let mut moved = z.clone();
            assert_eq!((moved.move_corner(0, k, to), &moved), (Err(CROSSES), &z), "{to:?}");
        }
        // A corner on the middle of an edge, then dragged out: a fifth corner.
        z.add_corner(0, 0, pt(5.0, 0.0)).unwrap();
        z.move_corner(0, 1, pt(5.0, -2.0)).unwrap();
        assert_eq!(z.outline[0], [pt(0.0, 0.0), pt(5.0, -2.0), pt(10.0, 0.0), pt(12.0, 12.0), pt(0.0, 10.0)]);
        // Folding an edge back along the one beside it is refused; dropped on a neighbour, it goes.
        assert_eq!(z.clone().move_corner(0, 1, pt(15.0, 0.0)), Err(CROSSES));
        z.move_corner(0, 1, pt(10.0, 0.0)).unwrap();
        assert_eq!(z.outline[0], [pt(0.0, 0.0), pt(10.0, 0.0), pt(12.0, 12.0), pt(0.0, 10.0)]);
        // Three corners stay; a cutout left with two goes.
        z.remove_corner(0, 2).unwrap();
        assert_eq!((z.remove_corner(0, 0), z.outline[0].len()), (Err(THREE), 3));
        z.remove_corner(1, 0).unwrap();
        assert_eq!(z.outline[1], [pt(4.0, 2.0), pt(4.0, 4.0), pt(2.0, 4.0)]);
        z.remove_corner(1, 2).unwrap();
        assert_eq!(z.outline.len(), 1);
    }

    #[test]
    fn adds_cutouts_inside_the_outline_apart() {
        let mut z = zone();
        z.add_cutout(square(6.0, 6.0, 1.0)).unwrap();
        assert_eq!(z.outline[2], square(6.0, 6.0, 1.0));
        // Outside, across the edge, across a cutout, round one, inside one or on one: refused.
        let inner = vec![pt(2.5, 2.5), pt(3.5, 2.5), pt(3.5, 3.5)];
        for c in
            [square(20.0, 20.0, 1.0), square(-1.0, 5.0, 2.0), square(3.0, 3.0, 2.0), square(1.0, 1.0, 4.0), inner, square(6.0, 6.0, 1.0)]
        {
            let mut cut = z.clone();
            assert_eq!((cut.add_cutout(c.clone()), &cut), (Err(CUTOUT), &z), "{c:?}");
        }
        // A cutout's corner moved out of the outline: refused; within it, moved.
        assert_eq!(z.clone().move_corner(2, 1, pt(11.0, 6.0)), Err(CROSSES));
        z.move_corner(2, 1, pt(8.0, 5.0)).unwrap();
        assert_eq!(z.outline[2][1], pt(8.0, 5.0));
    }
}
