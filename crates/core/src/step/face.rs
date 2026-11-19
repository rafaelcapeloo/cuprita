//! A face of a B-rep model laid out flat in its surface's own coordinates (angle and height on a
//! cylinder), triangulated there and lifted back, so it follows its surface and shares its
//! edges' points with its neighbours.

use super::{Frame, Mesh, TOL, V, comb, cross, dot, sub, unit};
use lyon_tessellation::math::point;
use lyon_tessellation::path::Polygon;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};
use std::collections::HashMap;
use std::f64::consts::TAU;

/// The surfaces faces lie on, with coordinates that follow them: angle about the axis and
/// height (cylinder, cone), longitude and latitude (sphere), angles round the axis and the tube
/// (torus). Other surfaces (B-spline, swept) are flattened onto the plane their face fits best.
#[derive(Clone, Copy, Debug)]
pub(super) enum Surface {
    Plane(Frame),
    Cylinder(Frame, f64),
    /// Radius at the frame's origin, and the tangent of its half angle.
    Cone(Frame, f64, f64),
    Sphere(Frame, f64),
    /// Distance from the axis to the tube's centre, and the tube's radius.
    Torus(Frame, f64, f64),
}

impl Surface {
    fn frame(&self) -> &Frame {
        match self {
            Surface::Plane(f) | Surface::Cylinder(f, _) | Surface::Cone(f, ..) | Surface::Sphere(f, _) | Surface::Torus(f, ..) => f,
        }
    }

    /// Its coordinates of `p`, which lies on it; the angle is NaN on the axis (as near as files
    /// write numbers), where any will do.
    fn uv(&self, p: V) -> [f64; 2] {
        let l = self.frame().local(p);
        let rho = l[0].hypot(l[1]);
        let angle = if rho > 1e-6 * self.scale() { l[1].atan2(l[0]) } else { f64::NAN };
        match *self {
            Surface::Plane(_) => [l[0], l[1]],
            Surface::Cylinder(..) | Surface::Cone(..) => [angle, l[2]],
            Surface::Sphere(_, r) => [angle, (l[2] / r).clamp(-1.0, 1.0).asin()],
            Surface::Torus(_, big, _) => [angle, l[2].atan2(rho - big)],
        }
    }

    /// The point at coordinates `uv`, and the surface's normal there.
    fn at(&self, [u, v]: [f64; 2]) -> (V, V) {
        let f = self.frame();
        let (s, c) = u.sin_cos();
        match *self {
            Surface::Plane(_) => (comb(f.o, f, [u, v, 0.0]), f.z),
            Surface::Cylinder(_, r) => (comb(f.o, f, [r * c, r * s, v]), comb([0.0; 3], f, [c, s, 0.0])),
            Surface::Cone(_, r, tan) => {
                let rho = r + v * tan;
                (comb(f.o, f, [rho * c, rho * s, v]), unit(comb([0.0; 3], f, [c, s, -tan])))
            }
            Surface::Sphere(_, r) => {
                let n = comb([0.0; 3], f, [v.cos() * c, v.cos() * s, v.sin()]);
                (std::array::from_fn(|k| f.o[k] + r * n[k]), n)
            }
            Surface::Torus(_, big, small) => {
                let n = comb([0.0; 3], f, [v.cos() * c, v.cos() * s, v.sin()]);
                (comb(f.o, f, [(big + small * v.cos()) * c, (big + small * v.cos()) * s, small * v.sin()]), n)
            }
        }
    }

    /// Whether its coordinates go round: the angle, and the torus's second angle.
    fn periodic(&self) -> [bool; 2] {
        match self {
            Surface::Plane(_) => [false, false],
            Surface::Torus(..) => [true, true],
            _ => [true, false],
        }
    }

    /// About how far one unit of its first coordinate goes, so that both are alike in length.
    fn scale(&self) -> f64 {
        match *self {
            Surface::Plane(_) => 1.0,
            Surface::Cylinder(_, r) | Surface::Cone(_, r, _) | Surface::Sphere(_, r) => r.abs().max(1e-3),
            Surface::Torus(_, big, small) => (big + small).abs().max(1e-3),
        }
    }
}

/// Wrapped into half a turn either way.
fn wrap(d: f64) -> f64 {
    d - TAU * (d / TAU).round()
}

/// A point of a face's boundary: its surface coordinates and where it is.
type P = ([f64; 2], V);

/// Unwraps coordinate `k` along a loop, so no step goes half a turn; returns the turns it makes.
fn unwrap(l: &mut [P], k: usize) -> i64 {
    let mut last: Option<f64> = None;
    for p in l.iter_mut().filter(|p| !p.0[k].is_nan()) {
        if let Some(q) = last {
            p.0[k] = q + wrap(p.0[k] - q);
        }
        last = Some(p.0[k]);
    }
    match (l.iter().find(|p| !p.0[k].is_nan()), last) {
        (Some(f), Some(q)) => ((q - f.0[k] + wrap(f.0[k] - q)) / TAU).round() as i64,
        _ => 0,
    }
}

/// Triangulates a face bounded by `loops` (`outer` the outer one) on `surface` (`None`: flat,
/// on the plane they fit best) into `mesh`; `mm`: millimetres per unit.
pub(super) fn triangulate(
    surface: Option<Surface>,
    loops: &[Vec<V>],
    outer: usize,
    mm: f64,
    color: [u8; 4],
    mesh: &mut Mesh,
) -> Option<()> {
    let flat = surface.is_none();
    let surface = surface.unwrap_or_else(|| Surface::Plane(best_plane(loops)));
    let periodic = surface.periodic();
    let mut rings: Vec<Vec<P>> = loops.iter().map(|l| l.iter().map(|&p| (surface.uv(p), p)).collect()).collect();
    let mut winding = vec![];
    for (i, l) in rings.iter_mut().enumerate() {
        let turns = [0, 1].map(|k| if periodic[k] { unwrap(l, k) } else { 0 });
        // On the axis (a sphere's pole, a cone's apex) any angle will do: there the loop turns back.
        if let Some(s) = l.iter().position(|p| p.0[0].is_nan()) {
            for p in &mut l[s..] {
                p.0[0] -= turns[0] as f64 * TAU;
            }
            let n = l.len();
            let angle = |j: usize| (1..n).map(|d| l[(j + d) % n].0[0]).find(|u| !u.is_nan());
            let back = |j: usize| (1..n).map(|d| l[(j + n - d) % n].0[0]).find(|u| !u.is_nan());
            *l = (0..n)
                .flat_map(|j| match (l[j].0[0].is_nan(), back(j), angle(j)) {
                    (true, Some(b), Some(a)) => vec![([b, l[j].0[1]], l[j].1), ([a, l[j].0[1]], l[j].1)],
                    _ => vec![l[j]],
                })
                .collect();
            if n == 1 {
                continue;
            }
        } else if let Some(k) = (0..2).find(|&k| turns[k] != 0) {
            winding.push((i, k));
        }
    }
    let mut polys: Vec<Vec<P>> = vec![];
    match winding[..] {
        [] => polys.extend(rings.iter().filter(|l| l.len() > 2).cloned()),
        // Two loops round the axis bound a band; one and a point (a cone's apex), a cap.
        [(a, k), (b, k2)] if k == k2 => polys.push(band(&rings[a], &rings[b], k)),
        [(a, 0)] => {
            let apex = rings.iter().find(|l| l.len() == 1)?;
            let mut line = rings[a][0];
            line.0[1] = apex[0].0[1];
            line.1 = apex[0].1;
            polys.push(band(&rings[a], &[line], 0));
        }
        _ => return None,
    }
    let main = if winding.is_empty() { outer.min(polys.len().saturating_sub(1)) } else { 0 };
    for (i, l) in rings.iter().enumerate() {
        if !winding.is_empty() && l.len() > 2 && !winding.iter().any(|w| w.0 == i) {
            polys.push(l.clone());
        }
    }
    // Holes found where the outer loop is, a whole turn away.
    let Some(range) = polys.get(main).map(|l| span(l)) else { return Some(()) };
    for (_, l) in polys.iter_mut().enumerate().filter(|(i, _)| *i != main) {
        let mid = span(l);
        for k in (0..2).filter(|&k| periodic[k]) {
            let shift = TAU * ((range[k] - mid[k]) / TAU).round();
            for p in l.iter_mut() {
                p.0[k] += shift;
            }
        }
    }
    // Lyon sweeps along y: put the angle there, so triangles span little of it.
    let scale = surface.scale();
    let key = |u: f64, v: f64| point(v as f32, (u * scale) as f32);
    let mut at: HashMap<(u32, u32), V> = HashMap::new();
    let pts: Vec<Vec<_>> = (polys.iter())
        .map(|l| {
            l.iter()
                .map(|&([u, v], p)| {
                    let q = key(u, v);
                    at.insert((q.x.to_bits(), q.y.to_bits()), p);
                    q
                })
                .collect()
        })
        .collect();
    let shapes: Vec<_> = pts.iter().map(|r| Polygon { points: r, closed: true }).collect();
    let mut tris: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut sink = BuffersBuilder::new(&mut tris, |v: FillVertex| v.position().to_array());
    FillTessellator::new().tessellate(shapes.iter().flat_map(Polygon::path_events), &FillOptions::DEFAULT, &mut sink).ok()?;
    let mut uv: Vec<[f64; 2]> = tris.vertices.iter().map(|&[x, y]| [y as f64 / scale, x as f64]).collect();
    let mut pos: Vec<V> = (tris.vertices.iter().zip(&uv))
        .map(|(q, &c)| at.get(&(q[0].to_bits(), q[1].to_bits())).copied().unwrap_or_else(|| surface.at(c).0))
        .collect();
    let mut faces: Vec<[u32; 3]> = tris.indices.as_chunks::<3>().0.to_vec();
    if !matches!(surface, Surface::Plane(_)) {
        delaunay(&uv, scale, &mut faces);
    }
    // Cylinders and cones run straight along their height: only spheres and tori bulge between
    // their edges.
    if matches!(surface, Surface::Sphere(..) | Surface::Torus(..)) {
        refine(&surface, &mut uv, &mut pos, &mut faces, mm);
    }
    let base = mesh.pos.len() as u32;
    let normal = |c: [f64; 2]| if flat { surface.frame().z } else { surface.at(c).1 };
    mesh.pos.extend(pos.iter().map(|p| p.map(|v| v as f32)));
    mesh.normal.extend(uv.iter().map(|&c| normal(c).map(|v| v as f32)));
    mesh.color.extend(std::iter::repeat_n(color, pos.len()));
    mesh.indices.extend(faces.iter().flatten().map(|i| i + base));
    Some(())
}

/// The middle of a loop's coordinates.
fn span(l: &[P]) -> [f64; 2] {
    let (lo, hi) = l.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| {
        ([lo[0].min(p.0[0]), lo[1].min(p.0[1])], [hi[0].max(p.0[0]), hi[1].max(p.0[1])])
    });
    [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]
}

/// The band between two loops winding round coordinate `k`: each spelled out over one whole turn
/// from where it starts, one there and the other back.
fn band(a: &[P], b: &[P], k: usize) -> Vec<P> {
    let turn = |l: &[P]| {
        let mut l = l.to_vec();
        if l.len() > 1 && l[l.len() - 1].0[k] < l[0].0[k] {
            l.reverse();
        }
        let mut end = l[0];
        end.0[k] += TAU;
        l.push(end);
        l
    };
    let (a, mut b) = (turn(a), turn(b));
    let shift = TAU * ((a[0].0[k] - b[0].0[k]) / TAU).round();
    for p in &mut b {
        p.0[k] += shift;
    }
    a.into_iter().chain(b.into_iter().rev()).collect()
}

/// The plane through the loops' points they fit best (Newell's normal).
fn best_plane(loops: &[Vec<V>]) -> Frame {
    let (mut n, mut c, mut count) = ([0.0; 3], [0.0; 3], 0.0);
    for l in loops {
        for (i, &p) in l.iter().enumerate() {
            let q = l[(i + 1) % l.len()];
            n = [n[0] + (p[1] - q[1]) * (p[2] + q[2]), n[1] + (p[2] - q[2]) * (p[0] + q[0]), n[2] + (p[0] - q[0]) * (p[1] + q[1])];
            c = std::array::from_fn(|k| c[k] + p[k]);
            count += 1.0;
        }
    }
    let z = unit(n);
    let x = unit(cross(if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }, z));
    Frame { o: c.map(|v| v / f64::max(count, 1.0)), x, y: cross(z, x), z }
}

/// Flips inner edges until each is Delaunay (its two opposite angles sum to at most half a turn)
/// in `uv`, its first coordinate times `k`: lyon fans flat slivers along straight runs of boundary,
/// which become even strips.
fn delaunay(uv: &[[f64; 2]], k: f64, tris: &mut [[u32; 3]]) {
    let at = |i: u32| [uv[i as usize][0] * k, uv[i as usize][1]];
    let angle = |o: u32, a: u32, b: u32| {
        let ([ox, oy], [ax, ay], [bx, by]) = (at(o), at(a), at(b));
        let (u, v) = ([ax - ox, ay - oy], [bx - ox, by - oy]);
        (u[0] * v[1] - u[1] * v[0]).abs().atan2(u[0] * v[0] + u[1] * v[1])
    };
    let key = |a: u32, b: u32| (a.min(b), a.max(b));
    let mut sides: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (t, tri) in tris.iter().enumerate() {
        for i in 0..3 {
            sides.entry(key(tri[i], tri[(i + 1) % 3])).or_default().push(t);
        }
    }
    let mut todo: Vec<(u32, u32)> = sides.keys().copied().collect();
    // Each flip makes the smallest angles larger, so this ends; the cap only guards against
    // rounding going round in circles.
    let mut flips = 0;
    while let Some(e) = todo.pop().filter(|_| flips < 8 * tris.len()) {
        let Some(&[t1, t2]) = sides.get(&e).map(Vec::as_slice) else { continue };
        // Triangle t1 runs p, q, r round; s is across p-q in t2.
        let i = (0..3).find(|&i| key(tris[t1][i], tris[t1][(i + 1) % 3]) == e).unwrap_or(0);
        let (p, q, r) = (tris[t1][i], tris[t1][(i + 1) % 3], tris[t1][(i + 2) % 3]);
        let s = tris[t2].into_iter().find(|&v| v != p && v != q).unwrap_or(r);
        if angle(r, p, q) + angle(s, p, q) <= std::f64::consts::PI + 1e-9 {
            continue;
        }
        (tris[t1], tris[t2]) = ([p, s, r], [s, q, r]);
        flips += 1;
        sides.remove(&e);
        sides.insert(key(r, s), vec![t1, t2]);
        for (edge, from, to) in [((q, r), t1, t2), ((p, s), t2, t1)] {
            if let Some(ts) = sides.get_mut(&key(edge.0, edge.1)) {
                ts.iter_mut().filter(|t| **t == from).for_each(|t| *t = to);
            }
        }
        todo.extend([key(p, s), key(s, q), key(q, r), key(r, p)]);
    }
}

/// Splits triangles' inner edges at their middle while it lies more than `TOL` off the surface.
fn refine(surface: &Surface, uv: &mut Vec<[f64; 2]>, pos: &mut Vec<V>, tris: &mut Vec<[u32; 3]>, mm: f64) {
    for _ in 0..4 {
        let mut uses: HashMap<(u32, u32), u8> = HashMap::new();
        for t in tris.iter() {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *uses.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let mut mids: HashMap<(u32, u32), Option<u32>> = HashMap::new();
        let mut mid = |a: u32, b: u32, uv: &mut Vec<[f64; 2]>, pos: &mut Vec<V>| {
            let key = (a.min(b), a.max(b));
            *mids.entry(key).or_insert_with(|| {
                // Boundary edges stay as their curves made them, matching the next face's.
                if uses[&key] < 2 {
                    return None;
                }
                let (a, b) = (a as usize, b as usize);
                let m = [(uv[a][0] + uv[b][0]) / 2.0, (uv[a][1] + uv[b][1]) / 2.0];
                let p = surface.at(m).0;
                let off = sub(p, std::array::from_fn(|k| (pos[a][k] + pos[b][k]) / 2.0));
                (dot(off, off).sqrt() * mm > TOL).then(|| {
                    uv.push(m);
                    pos.push(p);
                    pos.len() as u32 - 1
                })
            })
        };
        let mut out = Vec::with_capacity(tris.len() * 2);
        for &[a, b, c] in tris.iter() {
            let m = [mid(a, b, uv, pos), mid(b, c, uv, pos), mid(c, a, uv, pos)];
            // Turned so the split edges come first: edge i joins corners i and i + 1.
            let v = [a, b, c];
            match m.iter().filter(|x| x.is_some()).count() {
                0 => out.push(v),
                1 => {
                    let i = m.iter().position(Option::is_some).unwrap_or(0);
                    let (p, q, r, s) = (v[i], v[(i + 1) % 3], v[(i + 2) % 3], m[i].unwrap_or(0));
                    out.extend([[p, s, r], [s, q, r]]);
                }
                2 => {
                    let i = m.iter().position(Option::is_none).unwrap_or(0);
                    // Corners after the whole edge i: p, q, r with edges q-r and r-p split.
                    let (p, q, r) = (v[(i + 1) % 3], v[(i + 2) % 3], v[i]);
                    let (s, t) = (m[(i + 1) % 3].unwrap_or(0), m[(i + 2) % 3].unwrap_or(0));
                    out.extend([[s, q, t], [p, s, t], [p, t, r]]);
                }
                _ => {
                    let [ab, bc, ca] = m.map(|x| x.unwrap_or(0));
                    out.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
                }
            }
        }
        let done = out.len() == tris.len();
        *tris = out;
        if done {
            return;
        }
    }
}
