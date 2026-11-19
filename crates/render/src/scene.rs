//! Turns a `Board` into GPU-ready primitives, grouped into back-to-front draw slots.

use bytemuck::{Pod, Zeroable};
use cuprita_core::analysis::IrDrop;
use cuprita_core::board::{Board, Geom, LayerId, NetId, Nm, Pad, PadShape, Pt, Shape, Text, Via, ZoneFill, arc_center};
use cuprita_core::diff::Diff;
use cuprita_core::font;
use cuprita_core::schematic::{Ink, Project};
use lyon_tessellation::math::point;
use lyon_tessellation::path::Polygon;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};
use std::collections::HashMap;
use std::f64::consts::TAU;
use std::ops::Range;

/// A rotated rounded rectangle drawn by a signed-distance shader. Tracks are capsules
/// (radius = half height); pads, vias and holes are boxes or circles.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Inst {
    pub center: [f32; 2],
    pub half: [f32; 2],
    /// Direction of the local x axis.
    pub rot: [f32; 2],
    pub radius: f32,
    pub slot: u32,
    pub net: u32,
}

impl Inst {
    /// Distance from `p` to the shape, negative inside: the shader's SDF without its 1 px minimum.
    fn dist(&self, p: [f32; 2]) -> f32 {
        let ([dx, dy], [c, s]) = ([p[0] - self.center[0], p[1] - self.center[1]], self.rot);
        let local = [dx * c + dy * s, dy * c - dx * s];
        let q = [0, 1].map(|k| local[k].abs() - self.half[k] + self.radius);
        q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0) - self.radius
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vert {
    pub pos: [f32; 2],
    /// Its colour slot; `| ZONE` on a zone fill's, which draws quieter than tracks and pads.
    pub slot: u32,
    pub net: u32,
}

/// Marks a zone fill's vertex (`Vert::slot`).
pub const ZONE: u32 = 256;

/// The instances and triangle indices of one slot (a board layer, or the via/hole overlay).
#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    pub slot: usize,
    pub insts: Range<u32>,
    /// Zone fill triangles, drawn first.
    pub fills: Range<u32>,
    pub indices: Range<u32>,
}

/// Board geometry in micrometres relative to `origin`, ready for upload.
pub struct Scene {
    pub origin: Pt,
    /// Min x, min y, max x, max y of the board outline (of everything when there is none).
    pub bounds: [f32; 4],
    pub insts: Vec<Inst>,
    /// Zone fills' vertices and indices come first, so that `rebuild` can keep them.
    pub verts: Vec<Vert>,
    pub indices: Vec<u32>,
    /// Back to front; the first `stackup` entries are board layers that reverse when flipped.
    pub draws: Vec<Draw>,
    pub stackup: usize,
    /// Per slot: board layers in file order, then vias and holes.
    pub layers: Vec<Layer>,
    pub tess_failures: usize,
    zones: Zones,
}

/// The zone fill triangles at the start of `Scene::verts` and `Scene::indices`.
struct Zones {
    verts: usize,
    indices: usize,
    failures: usize,
    /// The board's fills they show (not set in diff and heatmap scenes), and where each one's
    /// vertices and indices are (`None`: it failed to tessellate): `rebuild` keeps them all
    /// while the board's fills stay these, else those of the fills still on the board.
    of: Option<Vec<ZoneFill>>,
    tris: Vec<Option<(Range<u32>, Range<u32>)>>,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    /// sRGB.
    pub color: [f32; 4],
    /// Shown when a board opens.
    pub visible: bool,
}

pub const BACKGROUND: [f32; 4] = rgb(0x0B0E12);
pub const HEAT_SLOT: u32 = 188;
pub const HEAT_STEPS: u32 = 64;
pub const ADDED_SLOT: u32 = 254;
pub const REMOVED_SLOT: u32 = 255;
pub const ADDED_COLOR: [f32; 4] = [0x2E as f32 / 255.0, 0xCC as f32 / 255.0, 0x71 as f32 / 255.0, 0.85];
pub const REMOVED_COLOR: [f32; 4] = [0xFF as f32 / 255.0, 0x52 as f32 / 255.0, 0x52 as f32 / 255.0, 0.85];

/// Thermal colour ramp (`0.0` cool blue → `0.5` emerald → `1.0` hot red).
pub fn heat_color(t: f32) -> [f32; 4] {
    const STOPS: [[f32; 3]; 5] = [[0.15, 0.55, 1.0], [0.0, 0.85, 0.85], [0.20, 0.90, 0.35], [1.0, 0.75, 0.10], [1.0, 0.22, 0.22]];
    let s = (t.clamp(0.0, 1.0) * (STOPS.len() - 1) as f32).min(3.9999);
    let (i, f) = (s as usize, s.fract());
    let [a, b] = [STOPS[i], STOPS[i + 1]];
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, 1.0]
}

pub fn build(board: &Board) -> Scene {
    let mut s = Builder::new(board);
    s.add(board, None, |l| l as usize, None);
    let mut scene = s.finish(board, None);
    scene.zones.of = Some(board.fills.clone());
    scene
}

/// `build` after an edit of the board `old` shows, keeping `old`'s layer visibility and, while the
/// origin stays, the triangles of the zone fills it drew that the board still has: most of a big
/// board's build.
// shortcut: everything else is built again (6 ms on kria-k26); rebuild only the edited items when
// a board needs it.
pub fn rebuild(board: &Board, mut old: Scene) -> Scene {
    let mut s = Builder::new(board);
    let same = old.origin == s.origin;
    let keep = same && old.zones.of.as_ref() == Some(&board.fills);
    s.zones = false;
    s.add(board, None, |l| l as usize, None);
    if !keep {
        let drawn: HashMap<&ZoneFill, usize> = old.zones.of.iter().flatten().enumerate().filter(|_| same).map(|(i, f)| (f, i)).collect();
        for f in &board.fills {
            let slot = f.layer as usize;
            let tris = match drawn.get(f).and_then(|&i| old.zones.tris[i].clone()) {
                Some(tris) => Some(s.copy_fill(slot, &old, tris)),
                None => s.fill(slot, f.net, &[&f.pts], true),
            };
            s.drawn.push(tris.map(|(v, x)| (slot, v, x)));
        }
    }
    let visible: Vec<bool> = old.layers.iter().map(|l| l.visible).collect();
    let of = if keep { old.zones.of.take() } else { Some(board.fills.clone()) };
    let mut scene = s.finish(board, keep.then_some(old));
    scene.zones.of = of;
    for (l, v) in scene.layers.iter_mut().zip(visible) {
        l.visible = v;
    }
    scene
}

/// DC IR-drop heatmap scene: background items have `net = 0` (dimmed when `View::net = 1`), while
/// items on `ir.net` use `HEAT_SLOT..` thermal colours and `net = 1` (drawn on top).
pub fn build_ir(board: &Board, ir: &IrDrop) -> Scene {
    let mut s = Builder::new(board);
    s.add(board, Some(0), |l| l as usize, None);
    let slot = |mv: f64| {
        let t = if ir.max_drop_mv > 1e-9 { (mv / ir.max_drop_mv).clamp(0.0, 1.0) as f32 } else { 0.0 };
        HEAT_SLOT + (t * (HEAT_STEPS - 1) as f32).round() as u32
    };
    for (i, t) in board.tracks.iter().enumerate().filter(|(_, t)| t.net == ir.net) {
        let (va, vb) = ir.track_drops.get(i).copied().unwrap_or_default();
        let pts = match t.mid {
            Some(m) => arc_pts(t.a, m, t.b),
            None => {
                let n = if ir.max_drop_mv > 1e-9 { (((va - vb).abs() / ir.max_drop_mv) * 16.0).ceil() as usize } else { 1 }.clamp(1, 16);
                (0..=n)
                    .map(|k| Pt {
                        x: t.a.x + ((t.b.x - t.a.x) as i64 * k as i64 / n as i64) as Nm,
                        y: t.a.y + ((t.b.y - t.a.y) as i64 * k as i64 / n as i64) as Nm,
                    })
                    .collect()
            }
        };
        let m = pts.len().saturating_sub(1);
        for (k, w) in pts.windows(2).enumerate() {
            let f = if m > 1 { k as f64 / (m - 1) as f64 } else { 0.5 };
            s.color = Some(slot(va + (vb - va) * f));
            s.capsule(t.layer as usize, 1, w[0], w[1], t.width);
        }
    }
    for (i, v) in board.vias.iter().enumerate().filter(|(_, v)| v.net == ir.net) {
        s.color = Some(slot(ir.via_drops.get(i).copied().unwrap_or_default()));
        s.via(1, v);
    }
    for (i, p) in board.pads.iter().enumerate().filter(|(_, p)| p.net == ir.net) {
        s.color = Some(slot(ir.pad_drops.get(i).copied().unwrap_or_default()));
        s.pad(1, p, |l| l as usize);
    }
    for z in board.fills.iter().filter(|z| z.net == ir.net) {
        let mark = s.slots[z.layer as usize].verts.len();
        s.color = Some(HEAT_SLOT);
        s.fill(z.layer as usize, 1, &[&z.pts], false);
        let on_layer = ir.nodes.iter().any(|&(l, _, _)| l == z.layer);
        for v in &mut s.slots[z.layer as usize].verts[mark..] {
            let (mut num, mut den) = (0.0, 0.0);
            for &(_, pt, mv) in ir.nodes.iter().filter(|&&(l, _, _)| !on_layer || l == z.layer) {
                let [nx, ny] = to_um(pt, s.origin);
                let w = 1.0 / ((v.pos[0] - nx).powi(2) + (v.pos[1] - ny).powi(2) + 1e4) as f64;
                num += w * mv;
                den += w;
            }
            if den > 0.0 {
                v.slot = slot(num / den);
            }
        }
    }
    s.finish(board, None)
}

/// Visual diff scene: unchanged items have `net = 0` (dimmed when `View::net = 1`), while removed
/// (`REMOVED_SLOT`, red) and added (`ADDED_SLOT`, green) items have `net = 1` (drawn on top).
pub fn build_diff(old: &Board, new: &Board, diff: &Diff) -> Scene {
    let mut s = Builder::new(new);
    let n = new.layers.len();
    let map_old = |l: LayerId| (diff.layer_map[l as usize] as usize).min(n.saturating_sub(1));
    s.add(
        new,
        Some(0),
        |l| l as usize,
        Some((&diff.added_tracks, &diff.added_vias, &diff.added_pads, &diff.added_shapes, &diff.added_zones, &diff.added_texts, false)),
    );
    if n > 0 || !old.vias.is_empty() {
        s.color = Some(REMOVED_SLOT);
        s.add(
            old,
            Some(1),
            map_old,
            Some((
                &diff.removed_tracks,
                &diff.removed_vias,
                &diff.removed_pads,
                &diff.removed_shapes,
                &diff.removed_zones,
                &diff.removed_texts,
                true,
            )),
        );
    }
    s.color = Some(ADDED_SLOT);
    s.add(
        new,
        Some(1),
        |l| l as usize,
        Some((&diff.added_tracks, &diff.added_vias, &diff.added_pads, &diff.added_shapes, &diff.added_zones, &diff.added_texts, true)),
    );
    s.finish(new, None)
}

/// Page `i` of project `p` as a scene: its paper, then each `Ink` in KiCad's default colours,
/// its drawing sheet first.
pub fn build_sch(p: &Project, i: usize) -> Scene {
    let (sch, frame) = (&p.pages[i].sheet, p.frame(i));
    let page = (Pt::default(), Pt { x: sch.page.0, y: sch.page.1 });
    let mut s = Builder::with(page, Ink::ALL.len() + 1);
    let (lo, hi) = page;
    s.fill(0, 0, &[[lo, Pt { x: hi.x, y: lo.y }, hi, Pt { x: lo.x, y: hi.y }]], false);
    for sh in frame.shapes.iter().chain(&sch.shapes) {
        s.shape(sh.layer as usize + 1, 0, sh);
    }
    for t in frame.texts.iter().chain(&sch.texts) {
        s.text(t.layer as usize + 1, 0, t);
    }
    let inks = Ink::ALL.iter().enumerate().map(|(r, &i)| (i.name().to_string(), 701 + r as i32, rgb(i.color()), true));
    s.finish_layers(std::iter::once((cuprita_core::tr::tr("Page").to_string(), 700, rgb(0xF5F4EF), true)).chain(inks).collect(), None)
}

/// Slots in back-to-front draw order; `flip` reverses the front/inner/back stackup while keeping
/// outlines, vias and holes on top, and the `active` copper slot goes just above the other copper
/// (the slots `copper` names, together in the stackup).
pub fn draw_order(
    draws: &[Draw],
    stackup: usize,
    flip: bool,
    active: Option<usize>,
    copper: impl Fn(usize) -> bool,
) -> impl DoubleEndedIterator<Item = &Draw> {
    let (s, o) = draws.split_at(stackup);
    let mut order: Vec<&Draw> = if flip { s.iter().rev().collect() } else { s.iter().collect() };
    if let (Some(a), Some(top)) =
        (order.iter().position(|d| Some(d.slot) == active && copper(d.slot)), order.iter().rposition(|d| copper(d.slot)))
    {
        let d = order.remove(a);
        order.insert(top, d);
    }
    order.into_iter().chain(o)
}

/// What copper a layer named `name` is, as the shader dims it: 1 an outer layer, 2 an inner one
/// (`In1.Cu`…), else 0.
pub(crate) fn copper(name: &str) -> u32 {
    match name {
        "F.Cu" | "B.Cu" => 1,
        n if n.strip_prefix("In").and_then(|k| k.strip_suffix(".Cu")).is_some_and(|k| k.parse::<u32>().is_ok()) => 2,
        _ => 0,
    }
}

impl Scene {
    /// Board nanometres to scene micrometres.
    pub fn um(&self, p: Pt) -> [f32; 2] {
        to_um(p, self.origin)
    }

    /// Scene micrometres to board nanometres.
    pub fn nm(&self, [x, y]: [f32; 2]) -> Pt {
        let d = |v: f32, o: Nm| (o as f64 + v as f64 * 1000.0).round() as Nm;
        Pt { x: d(x, self.origin.x), y: d(y, self.origin.y) }
    }

    /// Net of the topmost visible item with a net within `tol` of `p` (scene µm), or 0, with the
    /// view flipped or not and the `active` copper slot on top, as `draw_order` stacks them.
    // shortcut: scans every drawn shape (a few ms on the largest board), use the R-tree once
    // selection or DRC needs many queries.
    pub fn net_at(&self, p: [f32; 2], tol: f32, visible: &[bool], flip: bool, active: Option<usize>) -> u32 {
        let r = |r: &Range<u32>| r.start as usize..r.end as usize;
        let order = draw_order(&self.draws, self.stackup, flip, active, |s| copper(&self.layers[s].name) != 0);
        for d in order.rev().filter(|d| visible.get(d.slot) != Some(&false)) {
            // A slot draws its instances over its triangles.
            if let Some(i) = self.insts[r(&d.insts)].iter().rev().find(|i| i.net != 0 && i.dist(p) <= tol) {
                return i.net;
            }
            for t in [&d.indices, &d.fills].into_iter().flat_map(|x| self.indices[r(x)].as_chunks::<3>().0.iter().rev()) {
                let [a, b, c] = t.map(|k| self.verts[k as usize]);
                if a.net != 0 && in_triangle(p, a.pos, b.pos, c.pos) {
                    return a.net;
                }
            }
        }
        0
    }
}

const fn rgb(hex: u32) -> [f32; 4] {
    [((hex >> 16) & 0xFF) as f32 / 255.0, ((hex >> 8) & 0xFF) as f32 / 255.0, (hex & 0xFF) as f32 / 255.0, 1.0]
}

/// Draw rank, colour and default visibility of a layer.
fn style(name: &str) -> (i32, [f32; 4], bool) {
    const INNER: [u32; 6] = [0x4CAF7A, 0xD9A43F, 0x9C6FD6, 0x3FB7B0, 0xD96FA8, 0x8FA83F];
    let (rank, hex) = match name {
        "F.Cu" => (500, 0xE05A47),
        "B.Cu" => (200, 0x3F7FD9),
        "F.SilkS" => (620, 0xEEEEEE),
        "B.SilkS" => (100, 0xB0A0D8),
        "F.Fab" => (610, 0x9AA0A6),
        "B.Fab" => (110, 0x6B7280),
        "F.CrtYd" => (600, 0xD04FD0),
        "B.CrtYd" => (120, 0x6F6FD0),
        "F.Paste" => (510, 0xBFA58A),
        "B.Paste" => (190, 0x8A9BBF),
        "F.Mask" => (520, 0xC94F8F),
        "B.Mask" => (180, 0x3FA88F),
        "F.Adhes" => (530, 0xA8873F),
        "B.Adhes" => (170, 0x6F5FA8),
        "Edge.Cuts" => (800, 0xF2D649),
        n => match n.strip_prefix("In").and_then(|k| k.strip_suffix(".Cu")).and_then(|k| k.parse::<i32>().ok()) {
            Some(k) => (400 - k, INNER[(k.max(1) - 1) as usize % INNER.len()]),
            None => (700, 0x7A8899),
        },
    };
    let mut color = rgb(hex);
    // See-through solder mask keeps the copper under its openings visible.
    if name.ends_with(".Mask") {
        color[3] = 0.45;
    }
    // Paste, mask and glue matter only for fabrication, and the Fab layers' values and outlines
    // to an assembly drawing: they start hidden.
    (rank, color, ![".Paste", ".Mask", ".Adhes", ".Fab"].iter().any(|s| name.ends_with(s)))
}

/// Board outline extent, or of all copper items when there is no outline.
fn extent(b: &Board) -> (Pt, Pt) {
    let mut pts: Vec<Pt> = vec![];
    for sh in b.shapes.iter().filter(|sh| b.layers[sh.layer as usize] == "Edge.Cuts") {
        match &sh.geom {
            Geom::Segment { a, b } => pts.extend([*a, *b]),
            Geom::Arc { start, mid, end } => pts.extend(arc_pts(*start, *mid, *end)),
            Geom::Circle { center: c, radius: r } => pts.extend([Pt { x: c.x - r, y: c.y - r }, Pt { x: c.x + r, y: c.y + r }]),
            Geom::Poly(p) => pts.extend(p),
        }
    }
    if pts.is_empty() {
        pts.extend(b.tracks.iter().flat_map(|t| [t.a, t.b]).chain(b.pads.iter().map(|p| p.at)).chain(b.vias.iter().map(|v| v.at)));
    }
    let lo = pts.iter().fold(Pt { x: Nm::MAX, y: Nm::MAX }, |m, p| Pt { x: m.x.min(p.x), y: m.y.min(p.y) });
    let hi = pts.iter().fold(Pt { x: Nm::MIN, y: Nm::MIN }, |m, p| Pt { x: m.x.max(p.x), y: m.y.max(p.y) });
    if pts.is_empty() { (Pt::default(), Pt::default()) } else { (lo, hi) }
}

/// Points along an arc with at most ~2 µm deviation; ends are exact so tracks join without gaps.
fn arc_pts(s: Pt, m: Pt, e: Pt) -> Vec<Pt> {
    let Some((cx, cy, r, a0, sweep)) = arc_center(s, m, e) else { return vec![s, m, e] };
    let n = segments(r, sweep.abs());
    let mut v: Vec<Pt> = (0..=n).map(|i| on_circle(cx, cy, r, a0 + sweep * i as f64 / n as f64)).collect();
    (v[0], v[n]) = (s, e);
    v
}

fn circle_pts(c: Pt, r: Nm) -> Vec<Pt> {
    let n = segments(r as f64, TAU).max(8);
    (0..n).map(|i| on_circle(c.x as f64, c.y as f64, r as f64, TAU * i as f64 / n as f64)).collect()
}

fn on_circle(cx: f64, cy: f64, r: f64, a: f64) -> Pt {
    Pt { x: (cx + r * a.cos()).round() as Nm, y: (cy + r * a.sin()).round() as Nm }
}

/// Board nanometres to micrometres from `origin`.
fn to_um(p: Pt, origin: Pt) -> [f32; 2] {
    let d = |v: Nm, o: Nm| (v as i64 - o as i64) as f32 / 1000.0;
    [d(p.x, origin.x), d(p.y, origin.y)]
}

/// Whether `p` is inside or on triangle `abc`, in either winding.
fn in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let side = |u: [f32; 2], v: [f32; 2]| (v[0] - u[0]) * (p[1] - u[1]) - (v[1] - u[1]) * (p[0] - u[0]);
    let s = [side(a, b), side(b, c), side(c, a)];
    s.iter().all(|&x| x >= 0.0) || s.iter().all(|&x| x <= 0.0)
}

fn segments(r: f64, sweep: f64) -> usize {
    const TOL: f64 = 2_000.0; // nm
    let step = if r > TOL { 2.0 * (1.0 - TOL / r).acos() } else { sweep };
    ((sweep / step).ceil() as usize).clamp(1, 256)
}

#[derive(Clone, Default)]
struct Slot {
    insts: Vec<Inst>,
    verts: Vec<Vert>,
    indices: Vec<u32>,
    /// Zone fill triangles, which go first in the scene.
    fills: (Vec<Vert>, Vec<u32>),
}

type DiffMask<'a> = (&'a [bool], &'a [bool], &'a [bool], &'a [bool], &'a [bool], &'a [bool], bool);

struct Builder {
    origin: Pt,
    ext: (Pt, Pt),
    slots: Vec<Slot>,
    color: Option<u32>,
    tess: FillTessellator,
    failures: usize,
    zone_failures: usize,
    /// Whether `add` draws zone fills (`rebuild` draws them itself).
    zones: bool,
    /// Each zone fill drawn, in the board's order.
    drawn: Vec<Drawn>,
}

/// A zone fill drawn: its slot and its triangles' place in the slot's fills (vertices,
/// indices); `None` where it failed to tessellate.
type Drawn = Option<(usize, Range<u32>, Range<u32>)>;

impl Builder {
    fn new(board: &Board) -> Self {
        Self::with(extent(board), board.layers.len() + 2)
    }

    /// A builder of `slots` draw slots for a scene spanning `ext`.
    fn with(ext: (Pt, Pt), slots: usize) -> Self {
        let origin = Pt { x: ((ext.0.x as i64 + ext.1.x as i64) / 2) as Nm, y: ((ext.0.y as i64 + ext.1.y as i64) / 2) as Nm };
        let slots = vec![Slot::default(); slots];
        let tess = FillTessellator::new();
        Self { origin, ext, slots, color: None, tess, failures: 0, zone_failures: 0, zones: true, drawn: vec![] }
    }

    fn via(&mut self, net: NetId, v: &Via) {
        let (via, hole) = (self.slots.len() - 2, self.slots.len() - 1);
        self.boxed(via, net, v.at, (v.size, v.size), 0.0, v.size / 2);
        if self.color != Some(REMOVED_SLOT) {
            self.hole(hole, net, v.at, (v.drill, v.drill), 0.0);
        }
    }

    fn pad(&mut self, net: NetId, p: &Pad, map: impl Fn(LayerId) -> usize) {
        let (w, h) = if p.shape == PadShape::Circle { (p.size.0, p.size.0) } else { p.size };
        let r = match p.shape {
            PadShape::Circle | PadShape::Oval => w.min(h) / 2,
            PadShape::RoundRect(k) => (k as f64 * w.min(h) as f64) as Nm,
            // shortcut: trapezoids draw as their bounding rectangle; draw them exactly when a board needs it.
            _ => 0,
        };
        let copper = p.copper();
        let mut m = p.layers;
        while m != 0 {
            let l = map(m.trailing_zeros() as LayerId);
            if copper.is_empty() {
                self.boxed(l, net, p.at, (w, h), p.angle, r);
            } else {
                self.fill(l, net, &copper, false);
            }
            m &= m - 1;
        }
        if let (Some((dw, dh)), true) = (p.drill, self.color != Some(REMOVED_SLOT)) {
            self.hole(self.slots.len() - 1, net, p.at, (dw, dh), p.angle);
        }
    }

    fn add(&mut self, board: &Board, force_net: Option<NetId>, map: impl Fn(LayerId) -> usize, mask: Option<DiffMask<'_>>) {
        let net = |n: NetId| force_net.unwrap_or(n);
        for (_, t) in board.tracks.iter().enumerate().filter(|&(i, _)| mask.is_none_or(|m| m.0[i] == m.6)) {
            match t.mid {
                None => self.capsule(map(t.layer), net(t.net), t.a, t.b, t.width),
                Some(m) => self.polyline(map(t.layer), net(t.net), &arc_pts(t.a, m, t.b), t.width, false),
            }
        }
        for (_, v) in board.vias.iter().enumerate().filter(|&(i, _)| mask.is_none_or(|m| m.1[i] == m.6)) {
            self.via(net(v.net), v);
        }
        for (_, p) in board.pads.iter().enumerate().filter(|&(i, _)| mask.is_none_or(|m| m.2[i] == m.6)) {
            self.pad(net(p.net), p, &map);
        }
        for (_, sh) in board.shapes.iter().enumerate().filter(|&(i, _)| mask.is_none_or(|m| m.3[i] == m.6)) {
            self.shape(map(sh.layer), net(0), sh);
        }
        let zones = self.zones;
        for (_, z) in board.fills.iter().enumerate().filter(|&(i, _)| zones && mask.is_none_or(|m| m.4[i] == m.6)) {
            let slot = map(z.layer);
            let tris = self.fill(slot, net(z.net), &[&z.pts], true);
            self.drawn.push(tris.map(|(v, x)| (slot, v, x)));
        }
        for (_, t) in board.texts.iter().enumerate().filter(|&(i, _)| mask.is_none_or(|m| m.5[i] == m.6)) {
            self.text(map(t.layer), net(0), t);
        }
    }

    fn shape(&mut self, l: usize, n: NetId, sh: &Shape) {
        match &sh.geom {
            Geom::Segment { a, b } => self.capsule(l, n, *a, *b, sh.width),
            Geom::Arc { start, mid, end } => self.polyline(l, n, &arc_pts(*start, *mid, *end), sh.width, false),
            Geom::Circle { center, radius } if sh.filled => {
                let d = 2 * radius + sh.width;
                self.boxed(l, n, *center, (d, d), 0.0, d / 2)
            }
            Geom::Circle { center, radius } => self.polyline(l, n, &circle_pts(*center, *radius), sh.width, true),
            Geom::Poly(pts) => {
                if sh.filled {
                    self.fill(l, n, &[pts], false);
                }
                if sh.width > 0 || !sh.filled {
                    self.polyline(l, n, pts, sh.width, true);
                }
            }
        }
    }

    fn text(&mut self, l: usize, n: NetId, t: &Text) {
        font::strokes(t, |pts| self.polyline(l, n, pts, t.thickness, false));
    }

    /// The scene, its zone fill triangles first: `kept`'s, else the ones `add` made.
    fn finish(self, board: &Board, kept: Option<Scene>) -> Scene {
        // A layer named as copper that the board's table lacks holds what KiCad hides: it starts hidden.
        let styles = board.layers.iter().enumerate().map(|(i, l)| {
            let (rank, color, shown) = style(l);
            (l.clone(), rank, color, shown && (board.copper >> i & 1 != 0 || !l.ends_with(".Cu")))
        });
        let extra = [("Vias", 900, rgb(0xC7CCD1)), ("Holes", 1000, BACKGROUND)].map(|(n, r, c)| (n.to_string(), r, c, true));
        self.finish_layers(styles.chain(extra).collect(), kept)
    }

    /// The scene of slots named, ranked back to front, coloured and shown as `layers` says.
    fn finish_layers(self, layers: Vec<(String, i32, [f32; 4], bool)>, kept: Option<Scene>) -> Scene {
        let mut order: Vec<usize> = (0..self.slots.len()).collect();
        order.sort_by_key(|&i| layers[i].1);
        let ([lx, ly], [hx, hy]) = (self.p(self.ext.0), self.p(self.ext.1));
        let (mut fills, mut insts, mut verts, mut indices) = (vec![0..0; self.slots.len()], vec![], vec![], vec![]);
        let (failures, tris) = match kept {
            Some(old) => {
                for d in old.draws.iter().filter(|d| !d.fills.is_empty()) {
                    fills[d.slot] = d.fills.clone();
                }
                (insts, verts, indices) = (old.insts, old.verts, old.indices);
                insts.clear();
                verts.truncate(old.zones.verts);
                indices.truncate(old.zones.indices);
                (old.zones.failures, old.zones.tris)
            }
            None => {
                let mut base = vec![(0, 0); self.slots.len()];
                for &i in order.iter().filter(|&&i| !self.slots[i].fills.1.is_empty()) {
                    let ((v, x), vb, x0) = (&self.slots[i].fills, verts.len() as u32, indices.len() as u32);
                    verts.extend(v);
                    indices.extend(x.iter().map(|k| k + vb));
                    (fills[i], base[i]) = (x0..indices.len() as u32, (vb, x0));
                }
                let at = |(slot, v, x): &(usize, Range<u32>, Range<u32>)| {
                    let (vb, xb) = base[*slot];
                    (v.start + vb..v.end + vb, x.start + xb..x.end + xb)
                };
                (self.zone_failures, self.drawn.iter().map(|d| d.as_ref().map(at)).collect())
            }
        };
        let mut scene = Scene {
            origin: self.origin,
            bounds: [lx, ly, hx, hy],
            insts,
            zones: Zones { verts: verts.len(), indices: indices.len(), failures, of: None, tris },
            verts,
            indices,
            draws: vec![],
            stackup: 0,
            layers: layers.iter().map(|(name, _, color, visible)| Layer { name: name.clone(), color: *color, visible: *visible }).collect(),
            tess_failures: self.failures + failures,
        };
        for i in order {
            let (sl, fills) = (&self.slots[i], fills[i].clone());
            let (i0, x0, base) = (scene.insts.len() as u32, scene.indices.len() as u32, scene.verts.len() as u32);
            scene.insts.extend(&sl.insts);
            scene.verts.extend(&sl.verts);
            scene.indices.extend(sl.indices.iter().map(|k| k + base));
            if scene.insts.len() as u32 > i0 || scene.indices.len() as u32 > x0 || !fills.is_empty() {
                if layers[i].1 < 700 {
                    scene.stackup += 1;
                }
                let (insts, indices) = (i0..scene.insts.len() as u32, x0..scene.indices.len() as u32);
                scene.draws.push(Draw { slot: i, insts, fills, indices });
            }
        }
        scene
    }

    fn p(&self, p: Pt) -> [f32; 2] {
        to_um(p, self.origin)
    }

    fn push(&mut self, slot: usize, inst: Inst) {
        self.slots[slot].insts.push(inst);
    }

    fn hole(&mut self, slot: usize, net: NetId, at: Pt, size: (Nm, Nm), deg: f64) {
        let prev = self.color.take();
        self.boxed(slot, net, at, size, deg, size.0.min(size.1) / 2);
        self.color = prev;
    }

    fn capsule(&mut self, slot: usize, net: NetId, a: Pt, b: Pt, width: Nm) {
        let ([ax, ay], [bx, by]) = (self.p(a), self.p(b));
        let (dx, dy, hw) = (bx - ax, by - ay, width as f32 / 2000.0);
        let len = dx.hypot(dy);
        let rot = if len > 0.0 { [dx / len, dy / len] } else { [1.0, 0.0] };
        let color = self.color.unwrap_or(slot as u32);
        self.push(slot, Inst { center: [(ax + bx) / 2.0, (ay + by) / 2.0], half: [len / 2.0 + hw, hw], rot, radius: hw, slot: color, net });
    }

    fn polyline(&mut self, slot: usize, net: NetId, pts: &[Pt], width: Nm, closed: bool) {
        for w in pts.windows(2) {
            self.capsule(slot, net, w[0], w[1], width);
        }
        if let (true, Some(&a), Some(&b)) = (closed && pts.len() > 2, pts.last(), pts.first()) {
            self.capsule(slot, net, a, b, width);
        }
    }

    /// A box of `size` centred on `at`, turned by KiCad degrees, with rounded corners.
    fn boxed(&mut self, slot: usize, net: NetId, at: Pt, size: (Nm, Nm), deg: f64, radius: Nm) {
        let (s, c) = deg.to_radians().sin_cos();
        let half = [size.0 as f32 / 2000.0, size.1 as f32 / 2000.0];
        // KiCad turns counter-clockwise on a Y-down screen, so local x maps to (cos, -sin).
        let rot = [c as f32, -s as f32];
        let color = self.color.unwrap_or(slot as u32);
        self.push(slot, Inst { center: self.p(at), half, rot, radius: radius as f32 / 1000.0, slot: color, net });
    }

    /// Triangles of polygons `rings` with their holes inside them (even-odd); `zone` ones go with
    /// the zone fills. Returns where their vertices and indices went in the slot's lists, `None`
    /// when they failed to tessellate.
    fn fill(&mut self, slot: usize, net: NetId, rings: &[impl AsRef<[Pt]>], zone: bool) -> Option<(Range<u32>, Range<u32>)> {
        let rings = rings.iter().map(AsRef::as_ref).filter(|r| r.len() > 2);
        let rings: Vec<Vec<_>> = rings.map(|r| r.iter().map(|&q| self.p(q)).map(|[x, y]| point(x, y)).collect()).collect();
        let polys: Vec<_> = rings.iter().map(|r| Polygon { points: r, closed: true }).collect();
        let mut out: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
        let mut sink = BuffersBuilder::new(&mut out, |v: FillVertex| v.position().to_array());
        if self.tess.tessellate(polys.iter().flat_map(Polygon::path_events), &FillOptions::DEFAULT, &mut sink).is_err() {
            *(if zone { &mut self.zone_failures } else { &mut self.failures }) += 1;
            return None;
        }
        let color = self.color.unwrap_or(slot as u32 | if zone { ZONE } else { 0 });
        let sl = &mut self.slots[slot];
        let (verts, indices) = if zone { (&mut sl.fills.0, &mut sl.fills.1) } else { (&mut sl.verts, &mut sl.indices) };
        let (base, x0) = (verts.len() as u32, indices.len() as u32);
        verts.extend(out.vertices.iter().map(|&pos| Vert { pos, slot: color, net }));
        indices.extend(out.indices.iter().map(|i| i + base));
        Some((base..verts.len() as u32, x0..indices.len() as u32))
    }

    /// The zone fill triangles `old` holds at `verts` and `indices`, copied to slot `slot`'s
    /// fills; returns where they went there.
    fn copy_fill(&mut self, slot: usize, old: &Scene, (verts, indices): (Range<u32>, Range<u32>)) -> (Range<u32>, Range<u32>) {
        let (v, x) = &mut self.slots[slot].fills;
        let (base, x0) = (v.len() as u32, x.len() as u32);
        v.extend_from_slice(&old.verts[verts.start as usize..verts.end as usize]);
        x.extend(old.indices[indices.start as usize..indices.end as usize].iter().map(|k| k - verts.start + base));
        (base..v.len() as u32, x0..x.len() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_scene_tags_unchanged_removed_and_added_items() {
        let old = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal)) (net 0 "") (net 1 "GND")
              (segment (start 0 0) (end 2 0) (width 0.5) (layer "F.Cu") (net 1))
              (segment (start 2 0) (end 4 0) (width 0.5) (layer "F.Cu") (net 1)))"#,
        )
        .unwrap();
        let new = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal)) (net 0 "") (net 1 "GND")
              (segment (start 0 0) (end 2 0) (width 0.5) (layer "F.Cu") (net 1))
              (segment (start 2 0) (end 2 2) (width 0.5) (layer "F.Cu") (net 1)))"#,
        )
        .unwrap();
        let s = build_diff(&old, &new, &Diff::between(&old, &new));
        let tags: Vec<_> = s.insts.iter().map(|i| (i.slot, i.net)).collect();
        assert_eq!(tags, [(0, 0), (REMOVED_SLOT, 1), (ADDED_SLOT, 1)]);
        let ir = build_ir(&new, &new.ir_drop(1, 1.0, None, None));
        let heat: Vec<_> = ir.insts.iter().filter(|i| i.net == 1).map(|i| i.slot).collect();
        assert!(heat.first().copied() == Some(HEAT_SLOT) && heat.last().copied() == Some(HEAT_SLOT + HEAT_STEPS - 1));
    }

    #[test]
    fn track_becomes_one_capsule_and_fab_layers_start_hidden() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "F.Mask" user))
              (segment (start 0 0) (end 2 0) (width 0.5) (layer "F.Cu"))
              (segment (start 0 0) (end 2 0) (width 0.5) (layer "F.Mask"))
              (gr_line (start 0 0) (end 2 0) (layer "In5.Cu") (stroke (width 0.1)))
              (gr_line (start 0 0) (end 2 0) (layer "F.Fab") (stroke (width 0.1))))"#,
        )
        .unwrap();
        let s = build(&b);
        let track = Inst { center: [0.0, 0.0], half: [1250.0, 250.0], rot: [1.0, 0.0], radius: 250.0, slot: 0, net: 0 };
        assert_eq!(s.insts.iter().find(|i| i.slot == 0), Some(&track));
        // The mask copy is built so the layers panel can show it, but it starts hidden, as the
        // Fab layer does; so does a copper layer the board's table lacks, as KiCad hides it.
        assert_eq!((s.insts.len(), s.draws.iter().map(|d| d.slot).collect::<Vec<_>>()), (4, vec![2, 0, 1, 3]));
        let layers: Vec<_> = s.layers.iter().map(|l| (l.name.as_str(), l.visible)).collect();
        assert_eq!(layers, [("F.Cu", true), ("F.Mask", false), ("In5.Cu", false), ("F.Fab", false), ("Vias", true), ("Holes", true)]);
        assert_eq!(s.bounds, [-1000.0, 0.0, 1000.0, 0.0]);
    }

    #[test]
    fn rebuild_keeps_the_triangles_of_the_zone_fills_still_there_while_the_origin_stays() {
        let mut b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "GND")
              (segment (start 0 0) (end 2 0) (width 0.5) (layer "F.Cu") (net 1))
              (zone (net 1) (layer "B.Cu") (filled_polygon (pts (xy -1 -1) (xy 3 -1) (xy 3 1) (xy -1 1))))
              (zone (net 1) (layer "F.Cu") (filled_polygon (pts (xy 0 0.2) (xy 1 0.2) (xy 1 0.8)))))"#,
        )
        .unwrap();
        // The scene `build` makes, with the triangles of the fills `kept` kept (a mark on each
        // stays) and the others made again.
        fn rebuilt(b: &Board, mut s: Scene, kept: [bool; 2]) -> Scene {
            for (i, t) in s.zones.tris.clone().into_iter().enumerate() {
                s.verts[t.unwrap().0.start as usize].net = 7 + i as u32;
            }
            let mut s = rebuild(b, s);
            assert_eq!([7, 8].map(|m| s.verts.iter().any(|v| v.net == m)), kept);
            s.verts.iter_mut().filter(|v| v.net >= 7).for_each(|v| v.net = 1);
            let new = build(b);
            assert_eq!((&s.insts, &s.verts, &s.indices, &s.draws), (&new.insts, &new.verts, &new.indices, &new.draws));
            assert_eq!(s.zones.tris, new.zones.tris);
            s
        }
        let mut s = build(&b);
        s.layers[1].visible = false;
        b.tracks[0].width = 300_000;
        let s = rebuilt(&b, s, [true, true]);
        // Without an outline the origin follows the track; then one fill changes, alone made again.
        b.tracks[0].b.x = 4_000_000;
        let s = rebuilt(&b, s, [false, false]);
        b.fills[0].pts[0].x = -2_000_000;
        let s = rebuilt(&b, s, [false, true]);
        assert!(!s.layers[1].visible);
    }

    #[test]
    fn custom_pads_draw_all_their_copper() {
        // An anchor with an arm along its top, turned 90°: the arm runs up, x 9.5 to 9.9, y 9.5 to 7.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (35 "F.Paste" user)) (net 0 "") (net 1 "A")
              (footprint "X" (layer "F.Cu") (at 10 10 90) (pad "1" smd custom (at 0 0 90) (size 1 1) (layers "F.Cu" "F.Paste") (net 1 "A")
                (options (clearance outline) (anchor rect)) (primitives (gr_poly (pts (xy 0.5 -0.5) (xy 3 -0.5) (xy 3 -0.1) (xy 0.5 -0.1)) (width 0) (fill yes)))))
              (footprint "Y" (layer "F.Cu") (at 20 10) (pad "1" smd custom (at 0 0) (size 1 1) (layers "F.Paste") (net 1 "A")
                (primitives (gr_poly (pts (xy 0.5 -0.5) (xy 3 -0.5) (xy 3 -0.1) (xy 0.5 -0.1)) (width 0) (fill yes))))))"#,
        )
        .unwrap();
        let s = build(&b);
        // Its whole shape on F.Cu and F.Paste, also for the paste-only pad.
        let slots: Vec<_> = s.draws.iter().map(|d| (d.slot, d.insts.len(), !d.indices.is_empty())).collect();
        assert_eq!(slots, [(0, 0, true), (2, 0, true)]);
        let net = |x: f32, y: f32| s.net_at(s.um(Pt { x: (x * 1e6) as Nm, y: (y * 1e6) as Nm }), 1.0, &[true; 5], false, None);
        // On the arm, beside it, on the anchor.
        assert_eq!([net(9.7, 7.5), net(9.0, 7.5), net(10.0, 10.0)], [1, 0, 1]);
    }

    #[test]
    fn picks_the_net_of_the_topmost_visible_item() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user))
              (net 0 "") (net 1 "A") (net 2 "GND")
              (segment (start 0 0) (end 2 0) (width 0.5) (layer "F.Cu") (net 1))
              (zone (net 2) (layer "B.Cu") (filled_polygon (pts (xy -1 -1) (xy 3 -1) (xy 3 1) (xy -1 1))))
              (gr_line (start 1 -1) (end 1 1) (width 0.1) (layer "F.SilkS")))"#,
        )
        .unwrap();
        let s = build(&b);
        // In scene µm the track runs from (-1000, 0) to (1000, 0), 500 wide, over the zone; the
        // silk line (no net) crosses both at x = 0.
        let all = [true; 5];
        let pick = |x, y, tol, visible: &[bool], flip| s.net_at([x, y], tol, visible, flip, None);
        assert_eq!([pick(500.0, 0.0, 1.0, &all, false), pick(0.0, 0.0, 1.0, &all, false), pick(500.0, 600.0, 1.0, &all, false)], [1, 1, 2]);
        // Near the track's edge within tolerance; off the board.
        assert_eq!([pick(500.0, 300.0, 100.0, &all, false), pick(5000.0, 0.0, 1.0, &all, false)], [1, 0]);
        // Hidden layers can't be picked; flipping puts B.Cu above F.Cu, and so does B.Cu active,
        // under the front's silkscreen.
        assert_eq!([pick(500.0, 0.0, 1.0, &[false, true, true, true, true], false), pick(500.0, 0.0, 1.0, &all, true)], [2, 2]);
        assert_eq!(s.net_at([500.0, 0.0], 1.0, &all, false, Some(1)), 2);
        let slots = |flip, active| draw_order(&s.draws, s.stackup, flip, active, |l| copper(&s.layers[l].name) != 0).map(|d| d.slot);
        assert_eq!(
            [(false, None), (true, None), (false, Some(1)), (true, Some(0)), (false, Some(2))]
                .map(|(f, a)| slots(f, a).collect::<Vec<_>>()),
            [vec![1, 0, 2], vec![2, 0, 1], vec![0, 1, 2], vec![2, 1, 0], vec![1, 0, 2]]
        );
        assert_eq!(["F.Cu", "In12.Cu", "B.Cu", "F.SilkS", "Inner"].map(copper), [1, 2, 1, 0, 0]);
    }

    #[test]
    fn arc_points_follow_the_mid_point() {
        let mm = 1_000_000;
        let pts = arc_pts(Pt { x: mm, y: 0 }, Pt { x: 0, y: mm }, Pt { x: -mm, y: 0 });
        assert_eq!((pts[0], *pts.last().unwrap()), (Pt { x: mm, y: 0 }, Pt { x: -mm, y: 0 }));
        // Every point is on the side of the mid point (y > 0 here), within tolerance of the circle.
        assert!(pts.iter().all(|p| p.y >= 0 && ((p.x as f64).hypot(p.y as f64) - mm as f64).abs() < 1.0));
    }
}
