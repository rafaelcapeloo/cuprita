//! STEP (ISO 10303-21) models, as KiCad's libraries ship parts' 3D shapes: their faces as
//! coloured triangles in millimetres, Z up (each face laid out by `face`).

mod face;

use crate::tr::{tr, trf};
use face::{Surface, triangulate};
use std::collections::HashMap;
use std::f64::consts::TAU;

/// A model's faces as triangles: positions in mm (Z up), with a normal and an sRGB colour per
/// vertex.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub pos: Vec<[f32; 3]>,
    pub normal: Vec<[f32; 3]>,
    pub color: Vec<[u8; 4]>,
    pub indices: Vec<u32>,
}

/// Reads a STEP file's shapes (solids and shells, placed as its assemblies say) into one mesh.
/// Faces it cannot triangulate are left out; an error says what is wrong and where.
pub fn read(src: &[u8]) -> Result<Mesh, String> {
    let d = Data::parse(src)?;
    let m = Model::new(&d);
    let mut out = Mesh::default();
    let mut done: HashMap<u32, Mesh> = HashMap::new();
    for (item, xf) in m.placed() {
        let mesh = done.entry(item).or_insert_with(|| {
            let mut mesh = Mesh::default();
            m.item(item, &mut mesh, None);
            mesh
        });
        let base = out.pos.len() as u32;
        let f = |v: [f32; 3], w: f64| std::array::from_fn(|r| (0..3).map(|c| xf[r][c] * v[c] as f64).sum::<f64>() + xf[r][3] * w);
        out.pos.extend(mesh.pos.iter().map(|&p| f(p, 1.0).map(|v| (v * m.mm) as f32)));
        out.normal.extend(mesh.normal.iter().map(|&n| f(n, 0.0).map(|v| v as f32)));
        out.color.extend(&mesh.color);
        out.indices.extend(mesh.indices.iter().map(|i| i + base));
    }
    Ok(out)
}

type V = [f64; 3];
/// Rows of a 3 × 4 matrix: a placement.
type Xf = [[f64; 4]; 3];

const IDENTITY: Xf = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
/// How far a chord may stray from its curve or surface, mm.
const TOL: f64 = 0.01;
const GREY: [u8; 4] = [190, 190, 190, 255];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: V) -> V {
    let n = dot(a, a).sqrt();
    if n > 0.0 { a.map(|v| v / n) } else { [0.0, 0.0, 1.0] }
}

/// `a * x + b * y + c * z` about `o`.
fn comb(o: V, f: &Frame, [a, b, c]: V) -> V {
    std::array::from_fn(|k| o[k] + a * f.x[k] + b * f.y[k] + c * f.z[k])
}

fn compose(a: &Xf, b: &Xf) -> Xf {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum::<f64>() + if c == 3 { a[r][3] } else { 0.0 }))
}

/// An axis placement: origin and unit axes.
#[derive(Clone, Copy, Debug)]
struct Frame {
    o: V,
    x: V,
    y: V,
    z: V,
}

impl Frame {
    fn local(&self, p: V) -> V {
        let d = sub(p, self.o);
        [dot(d, self.x), dot(d, self.y), dot(d, self.z)]
    }

    /// From this frame's coordinates to the file's.
    fn xf(&self) -> Xf {
        std::array::from_fn(|r| [self.x[r], self.y[r], self.z[r], self.o[r]])
    }

    /// From the file's coordinates to this frame's.
    fn inverse(&self) -> Xf {
        let rows = [self.x, self.y, self.z];
        std::array::from_fn(|r| [rows[r][0], rows[r][1], rows[r][2], -dot(rows[r], self.o)])
    }
}

/// A B-spline curve: its degree, control points, weights and knots (multiplicities spelled out).
struct Spline {
    degree: usize,
    pts: Vec<V>,
    weights: Vec<f64>,
    knots: Vec<f64>,
}

impl Spline {
    fn range(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.pts.len()])
    }

    /// The point at `t`, by de Boor's algorithm in homogeneous coordinates.
    fn at(&self, t: f64) -> V {
        let (p, n) = (self.degree, self.pts.len());
        let k = (p..n).rev().find(|&k| self.knots[k] <= t).unwrap_or(p);
        let mut d: Vec<[f64; 4]> = (0..=p)
            .map(|j| {
                let (q, w) = (self.pts[j + k - p], self.weights[j + k - p]);
                [q[0] * w, q[1] * w, q[2] * w, w]
            })
            .collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = j + k - p;
                let span = self.knots[i + p - r + 1] - self.knots[i];
                let a = if span > 0.0 { (t - self.knots[i]) / span } else { 0.0 };
                d[j] = std::array::from_fn(|c| (1.0 - a) * d[j - 1][c] + a * d[j][c]);
            }
        }
        let h = d[p];
        [h[0] / h[3], h[1] / h[3], h[2] / h[3]]
    }

    /// The parameter of the point nearest `p`, from `n + 1` samples, then narrowed down.
    fn nearest(&self, p: V, n: usize) -> f64 {
        let (t0, t1) = self.range();
        let step = (t1 - t0) / n as f64;
        let dist = |t: f64| {
            let d = sub(self.at(t), p);
            dot(d, d)
        };
        let best = (0..=n).map(|i| t0 + step * i as f64).min_by(|a, b| dist(*a).total_cmp(&dist(*b))).unwrap_or(t0);
        let (mut lo, mut hi) = ((best - step).max(t0), (best + step).min(t1));
        for _ in 0..30 {
            let (a, b) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
            if dist(a) < dist(b) { hi = b } else { lo = a }
        }
        (lo + hi) / 2.0
    }
}

/// Segments along an arc of radius `r` (mm) sweeping `sweep` radians: chords within `TOL`, and
/// at least 12 a turn.
fn arc_steps(r: f64, sweep: f64) -> usize {
    let step = if r > TOL { (2.0 * (1.0 - TOL / r).acos()).min(TAU / 12.0) } else { TAU / 12.0 };
    ((sweep.abs() / step).ceil() as usize).clamp(1, 96)
}

/// A file's geometry: its entities, and its length unit in mm.
struct Model<'a, 'd> {
    d: &'d Data<'a>,
    mm: f64,
    /// Its plane angle unit in radians.
    rad: f64,
    /// Items' colours (solids', shells' and faces'), from their styles.
    colors: HashMap<u32, [u8; 4]>,
    /// How many items deep it reads (`within`).
    deep: std::cell::Cell<u32>,
}

impl<'a, 'd> Model<'a, 'd> {
    fn new(d: &'d Data<'a>) -> Self {
        let (mut mm, mut rad) = (1.0, 1.0);
        for e in d.0.values().filter(|e| e.kind.is_empty()) {
            let part = |k: &[u8]| e.args.iter().find_map(|a| if let Arg::Typed(n, v) = a { (*n == k).then_some(v) } else { None });
            let si = part(b"SI_UNIT");
            if part(b"LENGTH_UNIT").is_some() {
                if let Some(v) = si {
                    mm = match &v[..] {
                        [Arg::Enum(b"MILLI"), ..] => 1.0,
                        [Arg::Enum(b"CENTI"), ..] => 10.0,
                        [Arg::Enum(b"MICRO"), ..] => 1e-3,
                        _ => 1000.0,
                    };
                } else if let Some([Arg::Str(name), ..]) = part(b"CONVERSION_BASED_UNIT").map(|v| &v[..]) {
                    mm = if name.eq_ignore_ascii_case(b"FOOT") { 304.8 } else { 25.4 };
                }
            }
            if part(b"PLANE_ANGLE_UNIT").is_some() && si.is_none() {
                rad = TAU / 360.0;
            }
        }
        let mut colors = HashMap::new();
        for e in d.0.values().filter(|e| matches!(e.kind, b"STYLED_ITEM" | b"OVER_RIDING_STYLED_ITEM")) {
            let color = e.args.get(1).and_then(|s| s.list().iter().find_map(|a| color(d, a.id()?, 8)));
            if let (Some(c), Some(item)) = (color, e.args.get(2).and_then(Arg::id)) {
                colors.insert(item, c);
            }
        }
        Model { d, mm, rad, colors, deep: std::cell::Cell::new(0) }
    }

    fn point(&self, id: u32) -> Option<V> {
        let c = self.d.args(id, b"CARTESIAN_POINT")?.get(1)?.list();
        Some([c.first()?.num()?, c.get(1)?.num()?, c.get(2).and_then(Arg::num).unwrap_or(0.0)])
    }

    fn dir(&self, id: u32) -> Option<V> {
        let c = self.d.args(id, b"DIRECTION")?.get(1)?.list();
        Some(unit([c.first()?.num()?, c.get(1)?.num()?, c.get(2).and_then(Arg::num).unwrap_or(0.0)]))
    }

    /// An `AXIS2_PLACEMENT_3D`: its axis (Z) and reference direction (X) are optional.
    fn frame(&self, id: u32) -> Option<Frame> {
        let a = self.d.args(id, b"AXIS2_PLACEMENT_3D")?;
        let o = self.point(a.get(1)?.id()?)?;
        let z = a.get(2).and_then(Arg::id).and_then(|i| self.dir(i)).unwrap_or([0.0, 0.0, 1.0]);
        let r = a.get(3).and_then(Arg::id).and_then(|i| self.dir(i)).unwrap_or(if z[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        });
        let x = unit(sub(r, z.map(|v| v * dot(r, z))));
        Some(Frame { o, x, y: cross(z, x), z })
    }

    fn vertex(&self, id: u32) -> Option<V> {
        self.point(self.d.args(id, b"VERTEX_POINT")?.get(1)?.id()?)
    }

    /// The points along edge curve `id`, from its start vertex to its end.
    fn edge(&self, id: u32) -> Option<Vec<V>> {
        let a = self.d.args(id, b"EDGE_CURVE")?;
        let (p0, p1) = (self.vertex(a.get(1)?.id()?)?, self.vertex(a.get(2)?.id()?)?);
        let sense = !a.get(4).is_some_and(|s| s.is(b"F"));
        let mut pts = a.get(3).and_then(Arg::id).and_then(|c| self.curve(c, p0, p1, sense)).unwrap_or_default();
        if pts.len() < 2 {
            pts = vec![p0, p1];
        }
        let last = pts.len() - 1;
        (pts[0], pts[last]) = (p0, p1);
        Some(pts)
    }

    /// Points along curve `id` from `p0` to `p1`, which lie on it; `sense`: in its direction.
    fn curve(&self, id: u32, p0: V, p1: V, sense: bool) -> Option<Vec<V>> {
        let e = self.d.get(id)?;
        let a = &e.args;
        match e.kind {
            b"LINE" => Some(vec![p0, p1]),
            b"CIRCLE" | b"ELLIPSE" => {
                let f = self.frame(a.get(1)?.id()?)?;
                let r1 = a.get(2)?.num()?;
                let r2 = if e.kind == b"CIRCLE" { r1 } else { a.get(3)?.num()? };
                let t = |p: V| {
                    let l = f.local(p);
                    (l[1] / r2).atan2(l[0] / r1)
                };
                let (t0, t1) = (t(p0), t(p1));
                let mut sweep = if sense { (t1 - t0).rem_euclid(TAU) } else { -(t0 - t1).rem_euclid(TAU) };
                // A closed curve starts and ends at one vertex.
                if sweep.abs() < 1e-9 {
                    sweep = if sense { TAU } else { -TAU };
                }
                let n = arc_steps(r1.max(r2) * self.mm, sweep);
                Some((0..=n).map(|i| t0 + sweep * i as f64 / n as f64).map(|t| comb(f.o, &f, [r1 * t.cos(), r2 * t.sin(), 0.0])).collect())
            }
            b"SURFACE_CURVE" | b"SEAM_CURVE" | b"BOUNDED_SURFACE_CURVE" | b"TRIMMED_CURVE" => {
                self.within(|| self.curve(a.get(1)?.id()?, p0, p1, sense))?
            }
            b"POLYLINE" => {
                let pts: Vec<V> = a.get(1)?.list().iter().filter_map(|p| self.point(p.id()?)).collect();
                let near = |q: V| {
                    (0..pts.len()).min_by(|&i, &j| dot(sub(pts[i], q), sub(pts[i], q)).total_cmp(&dot(sub(pts[j], q), sub(pts[j], q))))
                };
                let (i, j) = (near(p0)?, near(p1)?);
                Some(if i <= j { pts[i..=j].to_vec() } else { pts[j..=i].iter().rev().copied().collect() })
            }
            _ => {
                let s = self.spline(id)?;
                let n = (s.pts.len() * 4).clamp(8, 200);
                let (lo, hi) = s.range();
                let (closed, t0) = (dot(sub(p0, p1), sub(p0, p1)) < 1e-18, s.nearest(p0, n));
                let t1 = if closed { if sense { hi } else { lo } } else { s.nearest(p1, n) };
                let t0 = if closed { if sense { lo } else { hi } } else { t0 };
                // A closed curve's edge may run across its ends.
                let mut ts = vec![];
                let mut run = |a: f64, b: f64| {
                    let k = ((b - a).abs() / (hi - lo) * n as f64).ceil().max(1.0) as usize;
                    ts.extend((0..=k).map(|i| a + (b - a) * i as f64 / k as f64));
                };
                match (sense, t0 <= t1) {
                    (true, true) | (false, false) => run(t0, t1),
                    (true, false) => {
                        run(t0, hi);
                        run(lo, t1);
                    }
                    (false, true) => {
                        run(t0, lo);
                        run(hi, t1);
                    }
                }
                Some(ts.into_iter().map(|t| s.at(t)).collect())
            }
        }
    }

    /// A B-spline curve, plain or rational (a complex entity).
    fn spline(&self, id: u32) -> Option<Spline> {
        let (degree, pts, mults, knots) = match self.d.args(id, b"B_SPLINE_CURVE_WITH_KNOTS")? {
            a if a.len() >= 8 => (a[1].num()?, a[2].list(), a[6].list(), a[7].list()),
            k => {
                let b = self.d.args(id, b"B_SPLINE_CURVE")?;
                (b.first()?.num()?, b.get(1)?.list(), k.first()?.list(), k.get(1)?.list())
            }
        };
        let pts: Vec<V> = pts.iter().map(|p| self.point(p.id()?)).collect::<Option<_>>()?;
        let weights = match self.d.args(id, b"RATIONAL_B_SPLINE_CURVE") {
            Some(w) => w.first()?.list().iter().map(Arg::num).collect::<Option<Vec<_>>>()?,
            None => vec![1.0; pts.len()],
        };
        let knots: Vec<f64> = knots
            .iter()
            .zip(mults)
            .flat_map(|(k, m)| std::iter::repeat_n(k.num(), (m.num().unwrap_or(0.0) as usize).min(1024)))
            .collect::<Option<_>>()?;
        let degree = degree as usize;
        let fits = degree >= 1 && pts.len() > degree && knots.len() == pts.len() + degree + 1 && weights.len() == pts.len();
        (fits && knots[degree] < knots[pts.len()]).then_some(Spline { degree, pts, weights, knots })
    }

    fn surface(&self, id: u32) -> Option<Surface> {
        let e = self.d.get(id)?;
        let a = &e.args;
        let f = || self.frame(a.get(1)?.id()?);
        let num = |i: usize| a.get(i).and_then(Arg::num);
        Some(match e.kind {
            b"PLANE" => Surface::Plane(f()?),
            b"CYLINDRICAL_SURFACE" => Surface::Cylinder(f()?, num(2)?),
            b"CONICAL_SURFACE" => Surface::Cone(f()?, num(2)?, (num(3)? * self.rad).tan()),
            b"SPHERICAL_SURFACE" => Surface::Sphere(f()?, num(2)?),
            b"TOROIDAL_SURFACE" => Surface::Torus(f()?, num(2)?, num(3)?),
            _ => return None,
        })
    }
}

/// The colour a style (`PRESENTATION_STYLE_ASSIGNMENT` …) leads to, looking `depth` steps deep.
fn color(d: &Data, id: u32, depth: u32) -> Option<[u8; 4]> {
    let e = d.get(id)?;
    let byte = |a: &Arg| a.num().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    match e.kind {
        b"COLOUR_RGB" => Some([byte(e.args.get(1)?)?, byte(e.args.get(2)?)?, byte(e.args.get(3)?)?, 255]),
        b"DRAUGHTING_PRE_DEFINED_COLOUR" => {
            const NAMES: [(&[u8], [u8; 4]); 8] = [
                (b"red", [255, 0, 0, 255]),
                (b"green", [0, 255, 0, 255]),
                (b"blue", [0, 0, 255, 255]),
                (b"yellow", [255, 255, 0, 255]),
                (b"magenta", [255, 0, 255, 255]),
                (b"cyan", [0, 255, 255, 255]),
                (b"black", [0, 0, 0, 255]),
                (b"white", [255, 255, 255, 255]),
            ];
            let Some(Arg::Str(name)) = e.args.first() else { return None };
            NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, c)| c)
        }
        _ if depth > 0 => {
            let refs = e.args.iter().flat_map(|a| std::iter::once(a).chain(a.list()));
            refs.filter_map(Arg::id).find_map(|i| color(d, i, depth - 1))
        }
        _ => None,
    }
}

/// A value in an entity's parameter list.
#[derive(Clone, Debug, PartialEq)]
enum Arg<'a> {
    Ref(u32),
    Num(f64),
    Str(&'a [u8]),
    Enum(&'a [u8]),
    List(Vec<Arg<'a>>),
    /// `NAME(…)`: a typed value, or one part of a complex entity.
    Typed(&'a [u8], Vec<Arg<'a>>),
    /// `$` or `*`.
    Unset,
}

impl<'a> Arg<'a> {
    fn id(&self) -> Option<u32> {
        if let Arg::Ref(i) = self { Some(*i) } else { None }
    }

    fn num(&self) -> Option<f64> {
        match self {
            Arg::Num(v) => Some(*v),
            Arg::Typed(_, v) => v.first()?.num(),
            _ => None,
        }
    }

    fn list(&self) -> &[Arg<'a>] {
        if let Arg::List(v) = self { v } else { &[] }
    }

    fn is(&self, e: &[u8]) -> bool {
        matches!(self, Arg::Enum(v) if *v == e)
    }
}

/// An entity: its type and parameters; a complex entity (`(A(…) B(…))`) has the type `b""` and
/// its parts as `Typed` parameters.
#[derive(Debug)]
struct Entity<'a> {
    kind: &'a [u8],
    args: Vec<Arg<'a>>,
}

/// The entities of a file's DATA section, by number.
struct Data<'a>(HashMap<u32, Entity<'a>>);

impl<'a> Data<'a> {
    fn parse(src: &'a [u8]) -> Result<Self, String> {
        let mut p = Parser { s: src, i: 0, depth: 0 };
        let start = find(src, b"DATA;").ok_or_else(|| tr("not a STEP file: no DATA section"))?;
        p.i = start + 5;
        let mut out = HashMap::new();
        loop {
            p.space();
            match p.peek() {
                Some(b'#') => {
                    p.i += 1;
                    let id = p.int().ok_or_else(|| trf("line {0}: expected an entity number", &[&p.line()]))?;
                    p.space();
                    p.expect(b'=')?;
                    p.space();
                    let e = if p.peek() == Some(b'(') {
                        p.i += 1;
                        let mut parts = vec![];
                        loop {
                            p.space();
                            if p.peek() == Some(b')') {
                                p.i += 1;
                                break;
                            }
                            let name = p.word();
                            p.space();
                            parts.push(Arg::Typed(name, p.list()?));
                        }
                        Entity { kind: b"", args: parts }
                    } else {
                        let kind = p.word();
                        p.space();
                        Entity { kind, args: p.list()? }
                    };
                    p.space();
                    p.expect(b';')?;
                    out.insert(id, e);
                }
                Some(_) if src[p.i..].starts_with(b"ENDSEC") => return Ok(Data(out)),
                _ => return Err(trf("line {0}: expected an entity", &[&p.line()])),
            }
        }
    }

    fn get(&self, id: u32) -> Option<&Entity<'a>> {
        self.0.get(&id)
    }

    /// The parameters of entity `id` if it is a `kind`, or has a `kind` part.
    fn args(&self, id: u32, kind: &[u8]) -> Option<&[Arg<'a>]> {
        let e = self.get(id)?;
        if e.kind == kind {
            return Some(&e.args);
        }
        e.args.iter().find_map(|a| match a {
            Arg::Typed(k, v) if *k == kind => Some(&v[..]),
            _ => None,
        })
    }
}

fn find(s: &[u8], what: &[u8]) -> Option<usize> {
    s.windows(what.len()).position(|w| w == what)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    /// How many lists deep it reads: each is a call, so a file may nest only so far.
    depth: u32,
}

impl<'a> Parser<'a> {
    /// The line it reads, from 1.
    fn line(&self) -> usize {
        self.s[..self.i.min(self.s.len())].iter().filter(|&&c| c == b'\n').count() + 1
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// Skips white space and comments.
    fn space(&mut self) {
        loop {
            while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                self.i += 1;
            }
            if !self.s[self.i..].starts_with(b"/*") {
                return;
            }
            self.i = find(&self.s[self.i..], b"*/").map_or(self.s.len(), |k| self.i + k + 2);
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() != Some(c) {
            return Err(trf("line {0}: expected '{1}'", &[&self.line(), &(c as char)]));
        }
        self.i += 1;
        Ok(())
    }

    fn word(&mut self) -> &'a [u8] {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
            self.i += 1;
        }
        &self.s[start..self.i]
    }

    fn int(&mut self) -> Option<u32> {
        let w = self.word();
        std::str::from_utf8(w).ok()?.parse().ok()
    }

    /// `( value, … )`.
    fn list(&mut self) -> Result<Vec<Arg<'a>>, String> {
        self.expect(b'(')?;
        self.depth += 1;
        if self.depth > 64 {
            return Err(trf("line {0}: lists nested deeper than a STEP file's go", &[&self.line()]));
        }
        let mut out = vec![];
        loop {
            self.space();
            match self.peek() {
                Some(b')') => {
                    (self.i, self.depth) = (self.i + 1, self.depth - 1);
                    return Ok(out);
                }
                Some(b',') => self.i += 1,
                _ => out.push(self.value()?),
            }
        }
    }

    fn value(&mut self) -> Result<Arg<'a>, String> {
        let c = self.peek().ok_or_else(|| trf("line {0}: expected a value", &[&self.line()]))?;
        Ok(match c {
            b'#' => {
                self.i += 1;
                Arg::Ref(self.int().ok_or_else(|| trf("line {0}: expected an entity number", &[&self.line()]))?)
            }
            b'\'' => {
                self.i += 1;
                let start = self.i;
                // `''` is a quote inside the string.
                while self.i < self.s.len() && !(self.s[self.i] == b'\'' && self.s.get(self.i + 1) != Some(&b'\'')) {
                    self.i += if self.s[self.i] == b'\'' { 2 } else { 1 };
                }
                if self.i >= self.s.len() {
                    return Err(trf("line {0}: a text without its closing quote: the file is cut short", &[&self.line()]));
                }
                self.i += 1;
                Arg::Str(&self.s[start..self.i - 1])
            }
            b'.' if self.s.get(self.i + 1).is_some_and(|c| c.is_ascii_alphabetic()) => {
                self.i += 1;
                let w = self.word();
                self.expect(b'.')?;
                Arg::Enum(w)
            }
            b'(' => Arg::List(self.list()?),
            b'$' | b'*' => {
                self.i += 1;
                Arg::Unset
            }
            c if c.is_ascii_alphabetic() => {
                let name = self.word();
                self.space();
                Arg::Typed(name, self.list()?)
            }
            _ => {
                let start = self.i;
                while self.peek().is_some_and(|c| c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'+' | b'e' | b'E')) {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
                Arg::Num(text.parse().map_err(|_| trf("line {0}: expected a number", &[&self.line()]))?)
            }
        })
    }
}

impl Model<'_, '_> {
    /// The shapes to draw (solids, shells, faces) and where each goes: those of the
    /// representations joined (by plain relationships) to one no assembly places, then those its
    /// assemblies and mapped items place in it, down the tree.
    fn placed(&self) -> Vec<(u32, Xf)> {
        let d = self.d;
        let reps: HashMap<u32, Vec<u32>> = (d.0.iter())
            .filter(|(_, e)| e.kind.ends_with(b"REPRESENTATION") && matches!(e.args.get(1), Some(Arg::List(_))))
            .map(|(&id, e)| (id, e.args[1].list().iter().filter_map(Arg::id).collect()))
            .collect();
        let mut same: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut parts: HashMap<u32, Vec<(u32, Xf)>> = HashMap::new();
        let mut placed = vec![];
        for &id in d.0.keys() {
            let Some([_, _, Arg::Ref(r1), Arg::Ref(r2), ..]) =
                d.args(id, b"REPRESENTATION_RELATIONSHIP").or_else(|| d.args(id, b"SHAPE_REPRESENTATION_RELATIONSHIP"))
            else {
                continue;
            };
            // Assemblies place the first representation in the second: from item 1 to item 2.
            let moved = d.args(id, b"REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION").and_then(|t| {
                let t = d.args(t.first()?.id()?, b"ITEM_DEFINED_TRANSFORMATION")?;
                Some(compose(&self.frame(t.get(3)?.id()?)?.xf(), &self.frame(t.get(2)?.id()?)?.inverse()))
            });
            match moved {
                Some(xf) => {
                    parts.entry(*r2).or_default().push((*r1, xf));
                    placed.push(*r1);
                }
                None => {
                    same.entry(*r1).or_default().push(*r2);
                    same.entry(*r2).or_default().push(*r1);
                }
            }
        }
        // Representations joined without a move, as groups.
        let group = |start: u32| {
            let mut g = vec![start];
            let mut i = 0;
            while i < g.len() {
                for &r in same.get(&g[i]).into_iter().flatten() {
                    if !g.contains(&r) {
                        g.push(r);
                    }
                }
                i += 1;
            }
            g
        };
        let mut roots: Vec<u32> = reps.keys().copied().collect();
        roots.sort_unstable();
        let (mut seen, mut todo) = (vec![], vec![]);
        for root in roots.into_iter().filter(|r| !seen.contains(r)).collect::<Vec<_>>() {
            let g = group(root);
            seen.extend(&g);
            if !g.iter().any(|r| placed.contains(r)) {
                todo.push((g, IDENTITY, 0));
            }
        }
        let mut out = vec![];
        while let Some((g, xf, depth)) = todo.pop() {
            for &item in g.iter().flat_map(|r| reps.get(r)).flatten() {
                match d.get(item).map(|e| (e.kind, &e.args[..])) {
                    // A copy of another representation, moved from its origin to the target.
                    Some((b"MAPPED_ITEM", a)) if depth < 16 => {
                        let map = a.get(1).and_then(Arg::id).and_then(|m| d.args(m, b"REPRESENTATION_MAP"));
                        let origin = map.and_then(|m| self.frame(m.first()?.id()?));
                        let target = a.get(2).and_then(Arg::id).and_then(|i| self.frame(i));
                        if let (Some(rep), Some(origin), Some(target)) = (map.and_then(|m| m.get(1)?.id()), origin, target) {
                            todo.push((group(rep), compose(&xf, &compose(&target.xf(), &origin.inverse())), depth + 1));
                        }
                    }
                    Some((b"MANIFOLD_SOLID_BREP" | b"BREP_WITH_VOIDS" | b"FACETED_BREP" | b"SHELL_BASED_SURFACE_MODEL", _))
                        if !out.contains(&(item, xf)) =>
                    {
                        out.push((item, xf));
                    }
                    _ => {}
                }
            }
            for &(rep, moved) in g.iter().flat_map(|r| parts.get(r)).flatten().filter(|_| depth < 16) {
                todo.push((group(rep), compose(&xf, &moved), depth + 1));
            }
        }
        if out.is_empty() {
            // No representations: every solid as it is.
            let solids =
                d.0.iter().filter(|(_, e)| matches!(e.kind, b"MANIFOLD_SOLID_BREP" | b"BREP_WITH_VOIDS" | b"SHELL_BASED_SURFACE_MODEL"));
            out.extend(solids.map(|(&id, _)| (id, IDENTITY)));
        }
        out
    }

    /// Triangles of solid, shell or face `id` (file units) into `mesh`, in its colour, else its
    /// solid's or shell's (`color`), else grey.
    fn item(&self, id: u32, mesh: &mut Mesh, color: Option<[u8; 4]>) {
        self.within(|| self.item_in(id, mesh, color));
    }

    /// What `f` gives, `None` once items hold items deeper than a model's do: one that holds
    /// itself (a shell its own shell, a curve its own curve) would never end.
    fn within<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        if self.deep.get() >= 32 {
            return None;
        }
        self.deep.set(self.deep.get() + 1);
        let out = f();
        self.deep.set(self.deep.get() - 1);
        Some(out)
    }

    fn item_in(&self, id: u32, mesh: &mut Mesh, color: Option<[u8; 4]>) {
        let Some(e) = self.d.get(id) else { return };
        let color = self.colors.get(&id).copied().or(color);
        let ids = |a: Option<&Arg>| a.map(|a| a.list().iter().filter_map(Arg::id).collect::<Vec<_>>()).unwrap_or_default();
        match e.kind {
            b"MANIFOLD_SOLID_BREP" | b"FACETED_BREP" => {
                if let Some(shell) = e.args.get(1).and_then(Arg::id) {
                    self.item(shell, mesh, color);
                }
            }
            b"BREP_WITH_VOIDS" => {
                for shell in e.args.get(1).and_then(Arg::id).into_iter().chain(ids(e.args.get(2))) {
                    self.item(shell, mesh, color);
                }
            }
            b"SHELL_BASED_SURFACE_MODEL" | b"CLOSED_SHELL" | b"OPEN_SHELL" => {
                for i in ids(e.args.get(1)) {
                    self.item(i, mesh, color);
                }
            }
            // shortcut: faces it can't lay out flat (4 of the 2.6 million in KiCad 10's library:
            // B-spline-bounded and pole-touching patches) are left out; cut them along their
            // seam when a part shows a hole.
            b"ADVANCED_FACE" | b"FACE_SURFACE" => {
                let _ = self.face(&e.args, mesh, color.unwrap_or(GREY));
            }
            _ => {}
        }
    }

    /// A face (`ADVANCED_FACE` parameters: bounds, surface) into `mesh`; `None` when it can't.
    fn face(&self, a: &[Arg], mesh: &mut Mesh, color: [u8; 4]) -> Option<()> {
        let (mut loops, mut outer) = (vec![], 0);
        for b in a.get(1)?.list() {
            let bound = self.d.get(b.id()?)?;
            if bound.kind == b"FACE_OUTER_BOUND" {
                outer = loops.len();
            }
            loops.push(self.ring(bound.args.get(1)?.id()?)?);
        }
        let surface = a.get(2).and_then(Arg::id).and_then(|s| self.surface(s));
        triangulate(surface, &loops, outer, self.mm, color, mesh)
    }

    /// The points around a loop: its edges' (each in the loop's direction) or polygon's; a
    /// vertex loop's one point.
    fn ring(&self, id: u32) -> Option<Vec<V>> {
        let e = self.d.get(id)?;
        match e.kind {
            b"EDGE_LOOP" => {
                let mut pts = vec![];
                for oe in e.args.get(1)?.list() {
                    let o = self.d.args(oe.id()?, b"ORIENTED_EDGE")?;
                    let mut p = self.edge(o.get(3)?.id()?)?;
                    if o.get(4).is_some_and(|s| s.is(b"F")) {
                        p.reverse();
                    }
                    // The next edge starts there.
                    p.pop();
                    pts.extend(p);
                }
                Some(pts)
            }
            b"POLY_LOOP" => e.args.get(1)?.list().iter().map(|p| self.point(p.id()?)).collect(),
            b"VERTEX_LOOP" => Some(vec![self.vertex(e.args.get(1)?.id()?)?]),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pin: a cylinder 1 mm round and 2 mm tall with flat ends, orange, which an assembly moves
    /// 10 mm along X.
    const PIN: &str = "ISO-10303-21;
HEADER; FILE_SCHEMA(('AUTOMOTIVE_DESIGN')); ENDSEC;
DATA;
#1 = CARTESIAN_POINT('',(0.,0.,0.));
#2 = DIRECTION('',(0.,0.,1.));
#3 = DIRECTION('',(1.,0.,0.));
#4 = AXIS2_PLACEMENT_3D('',#1,#2,#3);
#5 = CYLINDRICAL_SURFACE('',#4,1.);
#6 = CARTESIAN_POINT('',(1.,0.,0.));
#7 = CARTESIAN_POINT('',(1.,0.,2.));
#8 = VERTEX_POINT('',#6);
#9 = VERTEX_POINT('',#7);
#10 = CIRCLE('',#4,1.);
#11 = CARTESIAN_POINT('',(0.,0.,2.));
#12 = AXIS2_PLACEMENT_3D('',#11,#2,#3);
#13 = CIRCLE('',#12,1.);
#14 = EDGE_CURVE('',#8,#8,#10,.T.);
#15 = EDGE_CURVE('',#9,#9,#13,.T.);
#17 = VECTOR('',#2,1.);
#18 = LINE('',#6,#17);
#19 = EDGE_CURVE('',#8,#9,#18,.T.);
#20 = ORIENTED_EDGE('',*,*,#14,.T.);
#21 = ORIENTED_EDGE('',*,*,#19,.T.);
#22 = ORIENTED_EDGE('',*,*,#15,.F.);
#23 = ORIENTED_EDGE('',*,*,#19,.F.);
#24 = EDGE_LOOP('',(#20,#21,#22,#23));
#25 = FACE_OUTER_BOUND('',#24,.T.);
#26 = ADVANCED_FACE('',(#25),#5,.T.);
/* the ends */
#27 = PLANE('',#4);
#28 = ORIENTED_EDGE('',*,*,#14,.F.);
#29 = EDGE_LOOP('',(#28));
#30 = FACE_OUTER_BOUND('',#29,.T.);
#31 = ADVANCED_FACE('',(#30),#27,.F.);
#32 = PLANE('',#12);
#33 = ORIENTED_EDGE('',*,*,#15,.T.);
#34 = EDGE_LOOP('',(#33));
#35 = FACE_OUTER_BOUND('',#34,.T.);
#36 = ADVANCED_FACE('',(#35),#32,.T.);
#37 = CLOSED_SHELL('',(#26,#31,#36));
#38 = MANIFOLD_SOLID_BREP('pin',#37);
#39 = ADVANCED_BREP_SHAPE_REPRESENTATION('',(#4,#38),#100);
#40 = SHAPE_REPRESENTATION('',(#4),#100);
#41 = SHAPE_REPRESENTATION_RELATIONSHIP('','',#40,#39);
#50 = SHAPE_REPRESENTATION('',(#4,#51),#100);
#51 = AXIS2_PLACEMENT_3D('',#52,#2,#3);
#52 = CARTESIAN_POINT('',(10.,0.,0.));
#53 = ITEM_DEFINED_TRANSFORMATION('','',#4,#51);
#54 = ( REPRESENTATION_RELATIONSHIP('','',#40,#50) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#53)
  SHAPE_REPRESENTATION_RELATIONSHIP() );
#60 = COLOUR_RGB('it''s orange',1.,0.5,0.);
#61 = FILL_AREA_STYLE_COLOUR('',#60);
#62 = FILL_AREA_STYLE('',(#61));
#63 = SURFACE_STYLE_FILL_AREA(#62);
#64 = SURFACE_SIDE_STYLE('',(#63));
#65 = SURFACE_STYLE_USAGE(.BOTH.,#64);
#66 = PRESENTATION_STYLE_ASSIGNMENT((#65));
#67 = STYLED_ITEM('color',(#66),#38);
#100 = ( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#101,#102)) REPRESENTATION_CONTEXT('','') );
#101 = ( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );
#102 = ( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) );
ENDSEC;
END-ISO-10303-21;";

    #[test]
    fn reads_a_coloured_cylinder_moved_by_its_assembly_without_cracks() {
        let m = read(PIN.as_bytes()).unwrap();
        assert!(m.indices.len() >= 3 * 40, "{} triangles", m.indices.len() / 3);
        assert!(m.color.iter().all(|&c| c == [255, 128, 0, 255]));
        // On the cylinder round x = 10 or on its ends, every point.
        for p in &m.pos {
            let r = (p[0] - 10.0).hypot(p[1]);
            assert!(r <= 1.0 + 1e-4 && (0.0..=2.0).contains(&p[2]) && (r > 1.0 - 1e-4 || p[2] == 0.0 || p[2] == 2.0), "{p:?}");
        }
        // Each edge joins two triangles: faces meet without a gap.
        let mut edges: HashMap<([u32; 3], [u32; 3]), u32> = HashMap::new();
        for t in m.indices.as_chunks::<3>().0 {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (a, b) = (m.pos[a as usize].map(f32::to_bits), m.pos[b as usize].map(f32::to_bits));
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        assert!(edges.values().all(|&n| n == 2), "{:?}", edges.values().filter(|&&n| n != 2).count());
    }

    #[test]
    fn says_where_a_file_is_broken() {
        assert_eq!(read(b"no step").unwrap_err(), "not a STEP file: no DATA section");
        let broken = PIN.replace("#10 = CIRCLE('',#4,1.);", "#10 = CIRCLE('',#4,1.)");
        assert_eq!(read(broken.as_bytes()).unwrap_err(), "line 14: expected ';'");
    }

    #[test]
    fn a_file_made_to_break_the_reader_is_refused_or_read_as_far_as_it_goes() {
        // Lists inside lists without end, and a text cut short: an error, with its line.
        let deep = format!("DATA;\n#1 = A({});\nENDSEC;", "(".repeat(100_000));
        assert_eq!(read(deep.as_bytes()).unwrap_err(), "line 2: lists nested deeper than a STEP file's go");
        assert!(read(b"DATA;\n#1 = A('cut").unwrap_err().starts_with("line 2: a text without its closing quote"));
        // A shell that is its own shell, a curve its own curve, a spline of fewer points than
        // its degree with a count no file has: read, as nothing to draw.
        let own = PIN.replace("#10 = CIRCLE('',#4,1.);", "#10 = SURFACE_CURVE('',#10,(),.PCURVE_S1.);");
        assert!(read(own.as_bytes()).is_ok());
        let looped = "DATA;\n#1 = CLOSED_SHELL('',(#1));\n#2 = MANIFOLD_SOLID_BREP('',#1);\nENDSEC;";
        assert_eq!(read(looped.as_bytes()).map(|m| m.pos.len()), Ok(0));
        let spline = PIN.replace(
            "#10 = CIRCLE('',#4,1.);",
            "#10 = B_SPLINE_CURVE_WITH_KNOTS('',3,(#1,#1),.UNSPECIFIED.,.F.,.F.,(1e18,2),(1.,0.),.UNSPECIFIED.);",
        );
        assert!(read(spline.as_bytes()).is_ok());
    }
}
