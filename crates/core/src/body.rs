//! The board's body, as the 3D view shows it: its area less its holes.

use crate::board::{Board, Pt, Span, square, union};
use crate::drc::RBox;
use crate::polygon::{outline, overlay, path, rounded};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::point::IntPoint;

impl Board {
    /// The area inside its Edge.Cuts outlines (else the box around its copper) less cutouts and
    /// holes: contours, the holes inside their outlines (even-odd).
    pub fn body(&self) -> Vec<Vec<Pt>> {
        let edge = self.layers.iter().position(|l| l == "Edge.Cuts");
        let lines = self.shapes.iter().filter(|s| Some(s.layer as usize) == edge).map(|s| path(&s.geom)).collect();
        let area = outline(lines).or_else(|| {
            let pads = self.pads.iter().map(|p| square(p.at, p.size.0.max(p.size.1) / 2));
            let tracks = self.tracks.iter().flat_map(|t| [square(t.a, t.width / 2), square(t.b, t.width / 2)]);
            let (lo, hi) = pads.chain(tracks).chain(self.vias.iter().map(|v| square(v.at, v.size / 2))).reduce(union)?;
            let corners = [(lo.x, lo.y), (hi.x, lo.y), (hi.x, hi.y), (lo.x, hi.y)];
            Some(vec![vec![corners.map(|(x, y)| IntPoint::new(x, y)).to_vec()]])
        });
        let pads = self.pads.iter().filter_map(|p| p.drill.map(|(w, h)| RBox::boxed(p.at, (w, h), p.angle, w.min(h) / 2)));
        // A blind or buried via's hole stops inside the board.
        let vias = self.vias.iter().filter(|v| self.span(v) == Span::Through).map(|v| RBox::disc(v.at, v.drill / 2));
        let holes: Vec<_> = pads.chain(vias).map(|h| vec![rounded(&h, 0.0)]).collect();
        let pt = |q: &IntPoint| Pt { x: q.x, y: q.y };
        overlay(&area.unwrap_or_default(), &holes, OverlayRule::Difference).iter().flatten().map(|c| c.iter().map(pt).collect()).collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::board::{Board, MM};

    #[test]
    fn body_is_the_outline_less_cutouts_and_holes() {
        // A 20 × 10 mm board with a 2 mm square cutout, a via, a slot and a pad half off its edge.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts") (width 0.05))
              (gr_rect (start 2 2) (end 4 4) (layer "Edge.Cuts") (width 0.05))
              (via (at 10 5) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu"))
              (footprint "J" (layer "F.Cu") (at 15 5) (pad "1" thru_hole oval (at 0 0 90) (size 3 1.6) (drill oval 2 1) (layers "*.Cu")))
              (footprint "H" (layer "F.Cu") (at 20 5) (pad "" np_thru_hole circle (at 0 0) (size 2 2) (drill 2) (layers "*.Cu"))))"#,
        )
        .unwrap();
        let body = b.body();
        // The outline, now notched at the right, and three holes inside it.
        assert_eq!(body.len(), 4);
        let area: f64 =
            body.iter().map(|c| shoelace(c).abs()).sum::<f64>() - 2.0 * body[1..].iter().map(|c| shoelace(c).abs()).sum::<f64>();
        // 200 mm² less the cutout, the via, the 2 × 1 mm slot and half the 2 mm hole; holes are cut
        // a little wide (`rounded`).
        let pi = std::f64::consts::PI;
        let expect = 200.0 - 4.0 - pi * 0.15 * 0.15 - (1.0 + pi * 0.25) - pi / 2.0;
        assert!((area - expect).abs() < 0.02, "{area} vs {expect}");
        // No outline: the box around the copper.
        let bare = Board::parse(r#"(kicad_pcb (via (at 1 1) (size 1 1) (drill 0.5) (layers "F.Cu" "B.Cu")))"#).unwrap();
        assert_eq!(bare.body().iter().map(|c| c.len()).collect::<Vec<_>>()[..1], [4]);
        assert_eq!(bare.body()[0].iter().map(|p| p.x).max(), Some(3 * MM / 2));
    }

    fn shoelace(c: &[crate::board::Pt]) -> f64 {
        let mm = |v: i32| v as f64 / MM as f64;
        (0..c.len()).map(|i| (c[i], c[(i + 1) % c.len()])).map(|(p, q)| mm(p.x) * mm(q.y) - mm(q.x) * mm(p.y)).sum::<f64>() / 2.0
    }
}
