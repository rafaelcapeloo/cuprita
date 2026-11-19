//! Polygon booleans and offsets on board points (i_overlay's integer engine on our nm): the
//! board's area from its edge lines, rounded boxes, fills as KiCad saves them, custom pads' copper.

use crate::board::{Geom, MAX_ERROR, Nm, Pad, PadShape, Pt, arc_points, segments};
use crate::drc::RBox;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::{Overlay, ShapeType};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::angle::Angle;
use i_overlay::i_float::int::point::IntPoint;
use i_overlay::mesh::int::arc::ArcOptions;
use i_overlay::mesh::int::outline::offset::IntOutlineOffset;
use i_overlay::mesh::int::stroke::offset::IntStrokeOffset;
use i_overlay::mesh::int::style::{IntLineCap, IntLineJoin, IntOutlineStyle, IntStrokeStyle};
use std::f64::consts::FRAC_PI_2;

pub(crate) type Contour = Vec<IntPoint>;
/// An outline and the holes in it.
pub(crate) type Shape = Vec<Contour>;

// shortcut: i_overlay's i32 engine takes coordinates within ±1.07 m (KiCad's reach ±2.1 m);
// shift boards by their centre if one ever lies further out.
pub(crate) fn ip(p: Pt) -> IntPoint {
    IntPoint::new(p.x, p.y)
}

/// The area inside an odd number of `contours` (outlines with their cutouts), as shapes.
pub(crate) fn normalize(contours: Vec<Contour>) -> Vec<Shape> {
    let mut o = Overlay::new(contours.len());
    contours.iter().for_each(|c| o.add_contour(c, ShapeType::Subject));
    o.overlay(OverlayRule::Subject, FillRule::EvenOdd)
}

pub(crate) fn overlay(a: &[Shape], b: &[Shape], rule: OverlayRule) -> Vec<Shape> {
    let mut o = Overlay::new(a.len() + b.len());
    a.iter().flatten().for_each(|c| o.add_contour(c, ShapeType::Subject));
    b.iter().flatten().for_each(|c| o.add_contour(c, ShapeType::Clip));
    o.overlay(rule, FillRule::NonZero)
}

/// The shapes grown by `by` (shrunk when negative), with round corners.
pub(crate) fn grow(shapes: &[Shape], by: f64) -> Vec<Shape> {
    shapes.outline(&IntOutlineStyle::new(by.round() as Nm).line_join(IntLineJoin::Round(round(by)))).unwrap_or_default()
}

/// Arcs of radius `r` drawn within `MAX_ERROR`.
fn round(r: f64) -> ArcOptions {
    let step = Angle::from_radians(2.0 * (1.0 - MAX_ERROR / r.abs().max(MAX_ERROR)).acos()).unwrap_or(ArcOptions::MAX_STEP);
    ArcOptions { max_step: step, ..ArcOptions::default() }
}

/// A custom pad's copper in its own frame: its anchor and primitives `prims` joined (see `Pad::custom`).
pub(crate) fn custom_copper(p: &Pad, prims: &[crate::board::Shape]) -> Vec<Vec<Pt>> {
    let o = Pt::default();
    let anchor = if p.shape == PadShape::Circle { RBox::disc(o, p.size.0 / 2) } else { RBox::boxed(o, p.size, 0.0, 0) };
    let mut parts = vec![rounded(&anchor, 0.0)];
    for s in prims {
        match &s.geom {
            Geom::Poly(pts) if s.filled => {
                let poly = normalize(vec![pts.iter().map(|&q| ip(q)).collect()]);
                parts.extend(if s.width > 0 { grow(&poly, (s.width / 2) as f64) } else { poly }.into_iter().flatten());
            }
            Geom::Circle { center, radius } if s.filled => parts.push(rounded(&RBox::disc(*center, radius + s.width / 2), 0.0)),
            // Lines, arcs and outlines, one stroke each (a closed one joins round where its ends
            // meet); the chords of curves widened to cover them.
            g => {
                let w = s.width + if matches!(g, Geom::Arc { .. } | Geom::Circle { .. }) { 2 * MAX_ERROR as Nm } else { 0 };
                let r = round(w as f64 / 2.0);
                let style =
                    IntStrokeStyle::new(w).start_cap(IntLineCap::Round(r)).end_cap(IntLineCap::Round(r)).line_join(IntLineJoin::Round(r));
                let line: Contour = path(g).into_iter().map(ip).collect();
                parts.extend(line.stroke(&style, false).unwrap_or_default().into_iter().flatten());
            }
        }
    }
    joined(parts)
}

/// `parts` joined into outlines and the holes in them, as i_overlay lays them out (a custom
/// pad's copper, also mirrored by a flip).
pub(crate) fn joined(parts: Vec<Contour>) -> Vec<Vec<Pt>> {
    let joined = overlay(&[parts], &[], OverlayRule::Subject);
    joined.into_iter().flatten().map(|c| c.into_iter().map(|q| Pt { x: q.x, y: q.y }).collect()).collect()
}

/// `rb` grown by `by`, its rounded corners drawn with segments tangent to them, so it never
/// falls inside the true shape. The tangents lie at multiples of one step for a given radius,
/// so shapes meeting at a corner (track ends) share its points and booleans stay fast.
pub(crate) fn rounded(rb: &RBox, by: f64) -> Contour {
    let rad = rb.r + by;
    let step = FRAC_PI_2 / if rad > 0.0 { segments(rad, FRAC_PI_2) } else { 1 } as f64;
    let (ux, uy) = (rb.ux, rb.uy);
    let mut out: Contour = Vec::with_capacity(4 * segments(rad.max(1.0), FRAC_PI_2) + 8);
    for (q, (sx, sy)) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)].into_iter().enumerate() {
        // Corner of the core box (x along u, y along (-uy, ux)), its arc facing a0..a1.
        let (lx, ly) = (sx * rb.hx, sy * rb.hy);
        let (cx, cy) = (rb.cx + lx * ux - ly * uy, rb.cy + lx * uy + ly * ux);
        let a0 = uy.atan2(ux) + q as f64 * FRAC_PI_2;
        let inner = ((a0 / step).floor() as i64 + 1..).map(|k| k as f64 * step).take_while(|&a| a < a0 + FRAC_PI_2 - 1e-9);
        let tangents: Vec<f64> = std::iter::once(a0).chain(inner).chain([a0 + FRAC_PI_2]).collect();
        for w in tangents.windows(2) {
            // Where the tangents at w[0] and w[1] meet.
            let (m, k) = ((w[0] + w[1]) / 2.0, rad / ((w[1] - w[0]) / 2.0).cos());
            let p = IntPoint::new((cx + k * m.cos()).round() as Nm, (cy + k * m.sin()).round() as Nm);
            if out.last() != Some(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// A graphic's line as points; circles and polygons come back closed.
pub(crate) fn path(g: &Geom) -> Vec<Pt> {
    match g {
        Geom::Segment { a, b } => vec![*a, *b],
        Geom::Arc { start, mid, end } => arc_points(*start, *mid, *end),
        Geom::Circle { center: c, radius: r } => {
            let at = |dx: Nm, dy: Nm| Pt { x: c.x + dx, y: c.y + dy };
            let mut v = arc_points(at(*r, 0), at(0, *r), at(-r, 0));
            v.extend(arc_points(at(-r, 0), at(0, -r), at(*r, 0)).into_iter().skip(1));
            v
        }
        Geom::Poly(pts) => pts.iter().chain(pts.first()).copied().collect(),
    }
}

/// The board's area from its Edge.Cuts lines (outlines less cutouts), when they all join up
/// into closed loops.
pub(crate) fn outline(paths: Vec<Vec<Pt>>) -> Option<Vec<Shape>> {
    let loops = loops(&paths);
    let joined = loops.iter().map(|(members, _)| members.len()).sum::<usize>();
    (!loops.is_empty() && joined == paths.iter().filter(|p| !p.is_empty()).count())
        .then(|| normalize(loops.into_iter().map(|(_, pts)| pts.into_iter().map(ip).collect()).collect()))
}

/// Edge.Cuts `paths` (each a line's points) joined end to end into closed loops: each loop's
/// corners and the paths in it. Paths that close no loop are left out.
pub(crate) fn loops(paths: &[Vec<Pt>]) -> Vec<(Vec<usize>, Vec<Pt>)> {
    const JOIN: i64 = 10_000;
    let near = |a: Pt, b: Pt| (a.x as i64 - b.x as i64).abs() <= JOIN && (a.y as i64 - b.y as i64).abs() <= JOIN;
    let (mut open, mut out): (Vec<usize>, _) = ((0..paths.len()).filter(|&k| !paths[k].is_empty()).collect(), vec![]);
    'next: while let Some(k) = open.pop() {
        let (mut members, mut p) = (vec![k], paths[k].clone());
        while !near(p[0], p[p.len() - 1]) || p.len() < 3 {
            let end = p[p.len() - 1];
            let Some(i) = open.iter().position(|&q| near(paths[q][0], end) || near(paths[q][paths[q].len() - 1], end)) else {
                continue 'next;
            };
            let q = open.swap_remove(i);
            let mut next = paths[q].clone();
            if !near(next[0], end) {
                next.reverse();
            }
            p.extend(next.into_iter().skip(1));
            members.push(q);
        }
        p.pop();
        out.push((members, p));
    }
    out
}

/// Whether `p` is inside the loop through `ring` (even-odd: its edges crossing the ray from `p`
/// to the right), as a fill saved with its holes joined to its outline is.
pub(crate) fn inside(p: Pt, ring: &[Pt]) -> bool {
    let (x, y) = (p.x as f64, p.y as f64);
    let crosses = |(a, b): (&Pt, &Pt)| {
        let ((ax, ay), (bx, by)) = ((a.x as f64, a.y as f64), (b.x as f64, b.y as f64));
        (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay)
    };
    ring.iter().zip(ring.iter().cycle().skip(1)).filter(|&e| crosses(e)).count() % 2 == 1
}

/// The area inside an odd number of `contours`, grown by `by` (shrunk when negative) with round
/// corners: each outline (`true`), then the holes in it.
pub(crate) fn outlines(contours: &[Vec<Pt>], by: Nm) -> Vec<(bool, Vec<Pt>)> {
    let shapes = normalize(contours.iter().map(|c| c.iter().map(|&p| ip(p)).collect()).collect());
    let shapes = if by == 0 { shapes } else { grow(&shapes, by as f64) };
    let pt = |q: &IntPoint| Pt { x: q.x, y: q.y };
    shapes.iter().flat_map(|s| s.iter().enumerate().map(move |(i, c)| (i == 0, c.iter().map(pt).collect()))).collect()
}

/// A fill as KiCad saves it, grown by `by` (shrunk when negative), each part a contour: what
/// KiCad draws of a fill it saved to draw with an outline `2 * by` wide, and back.
pub(crate) fn grown(fill: &[Pt], by: Nm) -> Vec<Vec<Pt>> {
    grow(&normalize(vec![fill.iter().map(|&p| ip(p)).collect()]), by as f64).iter().map(fracture).collect()
}

/// One contour for a shape with holes, as KiCad saves fills: each hole joins the outline (or a
/// hole joined before it) by a zero-width cut from its leftmost corner straight to the left.
pub(crate) fn fracture(shape: &Shape) -> Vec<Pt> {
    let pt = |p: &IntPoint| Pt { x: p.x, y: p.y };
    if shape.len() == 1 {
        return shape[0].iter().map(pt).collect();
    }
    // Every contour as a ring of `next` links; joining a hole splices its ring in.
    let (mut pts, mut next) = (vec![], vec![]);
    let mut rings = vec![];
    for c in shape {
        let start = pts.len();
        pts.extend(c.iter().map(pt));
        next.extend((0..c.len()).map(|i| start + (i + 1) % c.len()));
        rings.push(start..pts.len());
    }
    // Edges by the start of the rows they cross, to find the one a ray to the left hits first.
    let (lo, hi) = (pts.iter().map(|p| p.y).min().unwrap() as i64, pts.iter().map(|p| p.y).max().unwrap() as i64);
    let rows = (pts.len() / 8).clamp(1, 1 << 16);
    let row_h = (hi - lo) / rows as i64 + 1;
    let row = |y: Nm| ((y as i64 - lo) / row_h) as usize;
    let mut table: Vec<Vec<usize>> = vec![vec![]; rows];
    let span = |pts: &[Pt], a: usize, b: usize| row(pts[a].y.min(pts[b].y))..=row(pts[a].y.max(pts[b].y));
    let add = |table: &mut Vec<Vec<usize>>, pts: &[Pt], next: &[usize], v: usize| {
        if pts[v].y != pts[next[v]].y {
            span(pts, v, next[v]).for_each(|r| table[r].push(v));
        }
    };
    for v in rings[0].clone() {
        add(&mut table, &pts, &next, v);
    }
    let mut holes: Vec<usize> = rings[1..].iter().map(|r| r.clone().min_by_key(|&v| (pts[v].x, pts[v].y)).unwrap()).collect();
    holes.sort_by_key(|&v| (pts[v].x, pts[v].y));
    for h in holes {
        let (hx, y) = (pts[h].x as f64, pts[h].y);
        // The nearest edge crossing the hole's row to its left (half-open in y, so a ray through
        // a corner hits one of its two edges).
        let hit = table[row(y)].iter().filter_map(|&v| {
            let (a, b) = (pts[v], pts[next[v]]);
            if y < a.y.min(b.y) || y >= a.y.max(b.y) {
                return None;
            }
            let x = a.x as f64 + (y - a.y) as f64 * (b.x - a.x) as f64 / (b.y - a.y) as f64;
            (x <= hx).then_some((x, v))
        });
        let Some((x, v)) = hit.max_by(|a, b| a.0.total_cmp(&b.0)) else { continue };
        let ring = rings.iter().find(|r| r.contains(&h)).unwrap().clone();
        let before = if h == ring.start { ring.end - 1 } else { h - 1 };
        let (w, cut) = (next[v], Pt { x: x.round() as Nm, y });
        span(&pts, v, w).for_each(|r| table[r].retain(|&e| e != v));
        // v → cut → h … before → h' → cut' → w
        let (c1, h2, c2) = (pts.len(), pts.len() + 1, pts.len() + 2);
        pts.extend([cut, pts[h], cut]);
        next.extend([h, c2, w]);
        (next[v], next[before]) = (c1, h2);
        for e in ring.chain([v, c2]) {
            add(&mut table, &pts, &next, e);
        }
    }
    let mut out = vec![pts[0]];
    let mut v = next[0];
    while v != 0 {
        out.push(pts[v]);
        v = next[v];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use i_overlay::core::point_location::IntPointContainment;
    use i_overlay::i_shape::int::area::Area;

    #[test]
    fn fracture_joins_holes_to_the_outline() {
        let sq = |x: Nm, y: Nm, s: Nm, ccw: bool| {
            let v = vec![IntPoint::new(x, y), IntPoint::new(x + s, y), IntPoint::new(x + s, y + s), IntPoint::new(x, y + s)];
            if ccw { v } else { v.into_iter().rev().collect() }
        };
        let shape = vec![sq(0, 0, 100, true), sq(20, 20, 10, false), sq(60, 50, 20, false), sq(20, 60, 10, false)];
        let one: Contour = fracture(&shape).into_iter().map(ip).collect();
        assert_eq!((one.area_two().unsigned_abs(), shape.area_two().unsigned_abs()), (2 * (10_000 - 100 - 400 - 100), 2 * 9_400));
        let probe = [IntPoint::new(25, 25), IntPoint::new(70, 60), IntPoint::new(25, 65), IntPoint::new(45, 45), IntPoint::new(5, 95)];
        assert_eq!(normalize(vec![one]).contains_points(&probe), [false, false, false, true, true]);
    }
}
