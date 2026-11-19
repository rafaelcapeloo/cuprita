//! The board's outline: a new one drawn on Edge.Cuts in place of the outer loops there, keeping
//! the cutouts inside them.

use crate::board::{Board, Geom, LayerId, Nm, Pt, Shape};
use crate::history::Patch;
use crate::polygon::{inside, loops, outline, path};
use crate::tr::{tr, trf};

/// KiCad's width for Edge.Cuts lines.
const WIDTH: Nm = 50_000;

impl Board {
    /// The corners of the box round the board's outline (its Edge.Cuts lines, its parts' too)
    /// when they all join into closed loops: `None` without one, as a fab can't cut it then.
    pub fn outline_box(&self) -> Option<(Pt, Pt)> {
        let edge = self.layers.iter().position(|l| l == "Edge.Cuts")?;
        let paths: Vec<Vec<Pt>> = self.shapes.iter().filter(|s| s.layer as usize == edge).map(|s| path(&s.geom)).collect();
        let pts = || paths.iter().flatten();
        let (x0, x1) = (pts().map(|p| p.x).min()?, pts().map(|p| p.x).max()?);
        let (y0, y1) = (pts().map(|p| p.y).min()?, pts().map(|p| p.y).max()?);
        outline(paths.clone()).map(|_| (Pt { x: x0, y: y0 }, Pt { x: x1, y: y1 }))
    }

    /// The width and height of `outline_box`.
    pub fn outline_size(&self) -> Option<(Nm, Nm)> {
        self.outline_box().map(|(a, b)| (b.x - a.x, b.y - a.y))
    }

    /// Draws the closed outline through `corners` on Edge.Cuts in place of the board's outer
    /// outlines there, keeping the cutouts inside them (lines that close no loop go too); returns
    /// the patch that undoes it. A part drawing part of the outer outline can't be redrawn here.
    pub fn set_outline(&mut self, corners: &[Pt]) -> Result<Patch, String> {
        let edge =
            self.layers.iter().position(|l| l == "Edge.Cuts").ok_or_else(|| tr("the board has no Edge.Cuts layer: add it in KiCad"))?;
        if corners.len() < 3 {
            return Err(trf("an outline needs 3 corners or more, not {0}", &[&corners.len()]));
        }
        let mut parts = vec![false; self.shapes.len()];
        for f in &self.footprints {
            parts[f.shapes.clone()].fill(true);
        }
        let lines: Vec<usize> = (0..self.shapes.len()).filter(|&i| self.shapes[i].layer as usize == edge).collect();
        let cutout = cutouts(&lines.iter().map(|&i| path(&self.shapes[i].geom)).collect::<Vec<_>>());
        let gone: Vec<usize> = lines.iter().zip(cutout).filter(|(_, c)| !c).map(|(&i, _)| i).collect();
        if let Some(f) = self.footprints.iter().find(|f| gone.iter().any(|i| f.shapes.contains(i))) {
            return Err(trf(
                "{0} draws part of the board's outline on Edge.Cuts: draw the new outline to meet it in KiCad",
                &[&f.reference],
            ));
        }
        // One splice from the first line gone to the last: KiCad writes a board's own drawings
        // together, after its parts.
        let range = match (gone.first(), gone.last()) {
            (Some(&a), Some(&b)) => a..b + 1,
            _ => self.shapes.len()..self.shapes.len(),
        };
        if parts[range.clone()].contains(&true) {
            return Err(tr("the board's outline lines lie apart in its file, between parts' drawings: redraw it in KiCad").into());
        }
        let kept = range.clone().filter(|i| gone.binary_search(i).is_err()).map(|i| self.shapes[i].clone());
        let new = corners.iter().zip(corners.iter().cycle().skip(1)).map(|(&a, &b)| Shape {
            geom: Geom::Segment { a, b },
            width: WIDTH,
            filled: false,
            layer: edge as LayerId,
        });
        Ok(self.apply(Patch::shapes(range, kept.chain(new).collect())))
    }
}

/// Of Edge.Cuts `paths`: whether each is a cutout's, its loop inside an odd number of others.
fn cutouts(paths: &[Vec<Pt>]) -> Vec<bool> {
    let mut out = vec![false; paths.len()];
    let loops = loops(paths);
    for (k, (members, pts)) in loops.iter().enumerate() {
        let around = loops.iter().enumerate().filter(|&(o, (_, ring))| o != k && inside(pts[0], ring)).count();
        if around % 2 == 1 {
            members.iter().for_each(|&m| out[m] = true);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::MM;

    #[test]
    fn draws_an_outline_in_place_of_the_outer_one_keeping_cutouts() {
        // As KiCad writes a board: parts, then its own drawings; here a part after them too.
        let src = r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user) (44 "Edge.Cuts" user))
          (footprint "R" (layer "F.Cu") (at 5 5) (fp_line (start 0 0) (end 1 0) (layer "F.SilkS") (width 0.1)))
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts") (width 0.05))
          (gr_line (start 1 1) (end 2 2) (layer "F.SilkS") (width 0.1))
          (gr_circle (center 10 5) (end 11 5) (layer "Edge.Cuts") (width 0.05))
          (footprint "C" (layer "F.Cu") (at 15 5) (fp_line (start 0 0) (end 1 1) (layer "F.SilkS") (width 0.1))))"#;
        let mut b = Board::parse(src).unwrap();
        let before = b.clone();
        let corners = [(-1, -1), (30, -1), (30, 12), (-1, 12)].map(|(x, y)| Pt { x: x * MM, y: y * MM });
        let undo = b.set_outline(&corners).unwrap();
        let edge: Vec<_> = b.shapes.iter().filter(|s| b.layers[s.layer as usize] == "Edge.Cuts").map(|s| s.geom.clone()).collect();
        // The rectangle went for four lines; the circle, a cutout, stayed.
        assert_eq!(edge.len(), 5);
        assert_eq!(
            (&edge[0], &edge[3]),
            (&Geom::Segment { a: corners[0], b: corners[1] }, &Geom::Segment { a: corners[3], b: corners[0] })
        );
        assert!(matches!(edge[4], Geom::Circle { .. }));
        // The part after them keeps its line.
        let line = |b: &Board| b.shapes[b.footprints[1].shapes.clone()].to_vec();
        assert_eq!(line(&b), line(&before));
        b.apply(undo);
        assert_eq!((&b.shapes, line(&b)), (&before.shapes, line(&before)));
        assert!(b.set_outline(&corners[..2]).err().is_some_and(|e| e.contains("3 corners")));
        // Its size, the outline closed; none for an open one, or none at all.
        assert_eq!(b.outline_size(), Some((20 * MM, 10 * MM)));
        assert_eq!(b.outline_box(), Some((Pt { x: 0, y: 0 }, Pt { x: 20 * MM, y: 10 * MM })));
        let edge = b.layers.iter().position(|l| l == "Edge.Cuts").unwrap() as LayerId;
        let line = Shape { geom: Geom::Segment { a: corners[0], b: corners[1] }, width: WIDTH, filled: false, layer: edge };
        b.shapes.push(line);
        assert_eq!((b.outline_size(), Board::default().outline_size()), (None, None));
        // A part drawing part of the outline (an edge connector's tongue) keeps it; one drawing a
        // slot inside it doesn't stop a new one.
        let line = |x0, y0, x1, y1| format!("(fp_line (start {x0} {y0}) (end {x1} {y1}) (layer \"Edge.Cuts\") (width 0.05))");
        let tongue = format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (44 "Edge.Cuts" user))
              (footprint "J" (layer "F.Cu") (at 0 0) (property "Reference" "J1") {} {} {})
              (gr_line (start 0 0) (end 20 0) (layer "Edge.Cuts") (width 0.05)) (gr_line (start 20 0) (end 20 10) (layer "Edge.Cuts") (width 0.05))
              (gr_line (start 20 10) (end 0 10) (layer "Edge.Cuts") (width 0.05)) (gr_line (start 0 10) (end 0 8) (layer "Edge.Cuts") (width 0.05))
              (gr_line (start 0 2) (end 0 0) (layer "Edge.Cuts") (width 0.05)))"#,
            line(0, 8, -3, 8),
            line(-3, 8, -3, 2),
            line(-3, 2, 0, 2)
        );
        let err = Board::parse(&tongue).unwrap().set_outline(&corners).err();
        assert!(err.is_some_and(|e| e.starts_with("J1 draws part of the board's outline")));
        let slot = src.replace(
            "(fp_line (start 0 0) (end 1 1) (layer \"F.SilkS\") (width 0.1))",
            &format!("{} {} {}", line(0, 0, 1, 0), line(1, 0, 1, 1), line(1, 1, 0, 0)),
        );
        assert!(Board::parse(&slot).unwrap().set_outline(&corners).is_ok());
    }
}
