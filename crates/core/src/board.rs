//! Board model, geometry helpers and lookups by name and place. Parsing lives in `parse`, net
//! analysis in `analysis`, board comparison in `diff`.

use crate::drc::{Item, RBox};
use crate::polygon::{inside, ip, joined};
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, TAU};
use std::ops::Range;

/// Integer nanometres, like KiCad.
pub type Nm = i32;
pub(crate) const MM: Nm = 1_000_000;
/// 1 oz copper and a 1.6 mm board: KiCad's defaults when the stackup leaves them out.
pub(crate) const DEFAULT_CU: Nm = 35_000;
pub(crate) const DEFAULT_BOARD: Nm = 1_600_000;
/// Index into `Board::layers`.
pub type LayerId = u8;
/// Index into `Board::nets`; 0 means "no net".
pub type NetId = u32;
/// Bit `i` set = on layer `i`.
pub type LayerMask = u128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pt {
    pub x: Nm,
    pub y: Nm,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Geom {
    Segment { a: Pt, b: Pt },
    Arc { start: Pt, mid: Pt, end: Pt },
    Circle { center: Pt, radius: Nm },
    Poly(Vec<Pt>),
}

/// A graphic item (board or footprint), in board coordinates.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Shape {
    pub geom: Geom,
    pub width: Nm,
    pub filled: bool,
    pub layer: LayerId,
}

/// Straight track, or an arc track when `mid` is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Track {
    pub a: Pt,
    pub b: Pt,
    pub mid: Option<Pt>,
    pub width: Nm,
    pub layer: LayerId,
    pub net: NetId,
    /// Locked in KiCad: a shove never moves it.
    pub locked: bool,
}

impl Track {
    /// Centerline length in nanometres (arc length when `mid` is set).
    pub fn len(&self) -> f64 {
        let dist = |p: Pt, q: Pt| ((q.x - p.x) as f64).hypot((q.y - p.y) as f64);
        match self.mid {
            Some(m) => arc_center(self.a, m, self.b).map_or_else(|| dist(self.a, m) + dist(m, self.b), |(_, _, r, _, s)| r * s.abs()),
            None => dist(self.a, self.b),
        }
    }
}

/// Copper layer in top-to-bottom stack order, with the dielectric below it (`0` on the bottom layer).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CuLayer {
    pub layer: LayerId,
    pub thickness: Nm,
    pub dielectric: Nm,
    pub epsilon_r: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Via {
    pub at: Pt,
    pub size: Nm,
    pub drill: Nm,
    /// Every copper layer the via spans.
    pub layers: LayerMask,
    pub net: NetId,
    /// Locked in KiCad: a shove never moves it.
    pub locked: bool,
}

/// How far a via goes: through the board, from an outer layer to an inner one, or between
/// inner layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Through,
    Blind,
    Buried,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PadShape {
    Circle,
    Rect,
    Oval,
    /// Corner radius as a ratio of the smaller side.
    RoundRect(f32),
    Trapezoid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    pub number: String,
    pub footprint: u32,
    pub at: Pt,
    /// Degrees, absolute: KiCad stores pad angles with the footprint rotation included.
    pub angle: f64,
    pub size: (Nm, Nm),
    /// For a custom pad, its anchor (`Rect` or `Circle`); `custom` is all its copper.
    pub shape: PadShape,
    /// Hole size; oval when the sides differ.
    pub drill: Option<(Nm, Nm)>,
    pub layers: LayerMask,
    pub net: NetId,
    /// Its own (else its footprint's) clearance and zone connection, overriding the board's.
    pub clearance: Option<Nm>,
    pub zone_connect: Option<Connect>,
    /// Angle of the first of a thermal relief's four spokes from the pad's x axis, in degrees.
    pub spoke_angle: f64,
    /// A custom pad's copper (or paste or mask opening), its anchor and primitives joined, in its
    /// own frame (like `size`): outlines with the holes inside them (even-odd), curves within
    /// `MAX_ERROR`. Empty for other pads.
    pub custom: Vec<Vec<Pt>>,
    /// Its own (else its footprint's) solder mask margin, paste margin and paste ratio of its
    /// size, overriding the board's (`Rules`).
    pub mask_margin: Option<Nm>,
    pub paste_margin: Option<Nm>,
    pub paste_ratio: Option<f64>,
}

impl Pad {
    /// `custom` on the board.
    pub fn copper(&self) -> Vec<Vec<Pt>> {
        self.custom.iter().map(|c| c.iter().map(|&q| place(q, self.at, self.angle)).collect()).collect()
    }

    /// Whether `p` is on the pad (a custom pad's copper, else its rectangle) or within `tol` of it.
    pub fn holds(&self, p: Pt, tol: Nm) -> bool {
        let ((s, c), tol) = (self.angle.to_radians().sin_cos(), tol as f64);
        let (dx, dy) = (p.x as f64 - self.at.x as f64, p.y as f64 - self.at.y as f64);
        // The pad's x axis runs along (cos, -sin) on the Y-down board, its y axis along (sin, cos).
        let (u, v) = (dx * c - dy * s, dx * s + dy * c);
        if !self.custom.is_empty() {
            return RBox::disc(Pt { x: u.round() as Nm, y: v.round() as Nm }, 0).dist_poly(&self.custom) as f64 <= tol;
        }
        u.abs() <= self.size.0 as f64 / 2.0 + tol && v.abs() <= self.size.1 as f64 / 2.0 + tol
    }
}

/// Design rules in nanometres, initialized to KiCad's defaults and overridden by the board/project.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    pub clearance: Nm,
    pub track_width: Nm,
    pub via_diameter: Nm,
    pub hole_diameter: Nm,
    pub annular_width: Nm,
    pub hole_to_hole: Nm,
    /// Between a hole and copper of another net (zone fills keep it from holes without copper).
    pub hole_clearance: Nm,
    pub edge_clearance: Nm,
    /// The default net class's track width and via diameter and drill, which new tracks and vias get.
    pub class_track_width: Nm,
    pub class_via_diameter: Nm,
    pub class_via_drill: Nm,
    /// Its differential pairs' track width and the gap between their two tracks.
    pub class_pair_width: Nm,
    pub class_pair_gap: Nm,
    /// The clearance new zones get.
    pub zone_clearance: Nm,
    /// Pads' solder mask openings grow by `mask_margin`; their paste openings by `paste_margin`
    /// and `paste_ratio` of their size (a pad or its footprint may say otherwise).
    pub mask_margin: Nm,
    pub paste_margin: Nm,
    pub paste_ratio: f64,
    /// Whether the solder mask covers vias on the front and on the back.
    pub tented: (bool, bool),
    /// The narrowest web of solder mask a fab keeps between two openings (0: any), how far bare
    /// copper of another net keeps from an opening, and whether a part's own pads may share one.
    pub mask_min_width: Nm,
    pub mask_to_copper: Nm,
    pub mask_bridges_in_parts: bool,
    /// What silkscreen keeps from other silkscreen (0: they may not touch).
    pub silk_clearance: Nm,
    /// What a line or text drawn on the silkscreen, and on any other layer, gets (the project's
    /// defaults): the line's width, the text's glyph width and height, and its pen.
    pub silk_ink: (Nm, (Nm, Nm), Nm),
    pub other_ink: (Nm, (Nm, Nm), Nm),
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            clearance: 200_000,
            track_width: 200_000,
            via_diameter: 500_000,
            hole_diameter: 300_000,
            annular_width: 100_000,
            hole_to_hole: 250_000,
            hole_clearance: 250_000,
            edge_clearance: 500_000,
            class_track_width: 250_000,
            class_via_diameter: 800_000,
            class_via_drill: 400_000,
            class_pair_width: 200_000,
            class_pair_gap: 250_000,
            zone_clearance: 500_000,
            mask_margin: 0,
            paste_margin: 0,
            paste_ratio: 0.0,
            tented: (true, true),
            mask_min_width: 0,
            mask_to_copper: 5_000,
            mask_bridges_in_parts: false,
            silk_clearance: 0,
            // What a fab prints: lines of 0.15 mm, letters 1 mm high.
            silk_ink: (150_000, (MM, MM), 150_000),
            other_ink: (100_000, (MM, MM), 150_000),
        }
    }
}

/// A net class of the project besides Default, whose values `Rules` holds (`.kicad_pro`
/// `net_settings`): what its nets' copper keeps from others', and what their new tracks, vias
/// and differential pairs take. A value the class leaves out is Default's.
#[derive(Clone, Debug, PartialEq)]
pub struct NetClass {
    pub name: String,
    /// Never below the board's minimum.
    pub clearance: Nm,
    pub track_width: Nm,
    pub via_diameter: Nm,
    pub via_drill: Nm,
    /// A differential pair's track width and the gap between its two tracks.
    pub pair_width: Nm,
    pub pair_gap: Nm,
}

/// Whether `name` is as `pattern` says: itself, or with `*` for any run and `?` for any one.
// shortcut: KiCad also reads a pattern as a regular expression; match those when a project has one.
pub fn matches(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    // The classic two-pointer walk: on a mismatch, the last `*` takes one more letter.
    let (mut i, mut j, mut star) = (0, 0, None);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            (i, j) = (i + 1, j + 1);
        } else if i < p.len() && p[i] == '*' {
            (star, i) = (Some((i, j)), i + 1);
        } else if let Some((si, sj)) = star {
            (i, j, star) = (si + 1, sj + 1, Some((si, sj + 1)));
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == '*')
}

/// A placed footprint, for finding it by reference, listing it in the BOM and moving it.
#[derive(Clone, Debug, PartialEq)]
pub struct Footprint {
    pub reference: String,
    pub value: String,
    /// Library link as written, e.g. `Resistor_SMD:R_0603_1608Metric`.
    pub lib_id: String,
    /// False for parts left off the assembled board: DNP, excluded from the BOM or KiCad 5 `virtual`.
    pub fitted: bool,
    /// False for parts a pick-and-place machine leaves alone: DNP, excluded from position files
    /// or KiCad 5 `virtual`.
    pub placed: bool,
    /// Its pads may share a solder mask opening (KiCad's Allow solder mask bridges).
    pub mask_bridges: bool,
    /// Its other fields (`MPN`, `LCSC`, …): name and value, as written.
    pub fields: Vec<(String, String)>,
    /// Placed on the bottom side (`B.Cu`).
    pub back: bool,
    /// Anchor, and rotation in degrees in [0°, 360°).
    pub at: Pt,
    pub angle: f64,
    /// Corners of the box around its pads and anchor.
    pub bounds: (Pt, Pt),
    /// Its items in `Board::pads`, `Board::shapes` and `Board::texts`.
    pub pads: Range<usize>,
    pub shapes: Range<usize>,
    pub texts: Range<usize>,
    /// Its texts not shown, the file's and those an edit hid (`Board::tidy_texts`, Delete), where
    /// they would be: saving writes them hidden, `Board::show_text` shows one again.
    pub hidden: Vec<Text>,
    /// Where saving finds its text.
    pub src: Src,
    /// Its shown 3D models.
    pub models: Vec<Model>,
}

/// A footprint's 3D model: its file as written (`${KICAD10_3DMODEL_DIR}/…`), then how it sits on
/// the footprint: offset in mm (Y up, Z out of the board's side), scale, and turns in degrees
/// about X, Y and Z.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub file: String,
    pub offset: [f64; 3],
    pub scale: [f64; 3],
    pub rotate: [f64; 3],
}

/// Where a footprint's KiCad text is.
#[derive(Clone, Debug, PartialEq)]
pub enum Src {
    /// Its place among the footprints of the text the board was read from, which saving edits.
    File(usize),
    /// Its text at the origin, for one added from a library (`Board::add_footprint`); saving
    /// writes it placed, after the file's footprints.
    Library(String),
    /// The file's footprint `.0` as the library footprint of text `.1` (as `Library` keeps it):
    /// F8 swapped it (`update.rs`). Saving writes the file's with the library's pads, graphics
    /// and models in place of its own, its fields and texts kept.
    Swapped(usize, String),
}

impl Footprint {
    /// Whether `p` is in its box or within `tol` of it.
    pub fn holds(&self, p: Pt, tol: Nm) -> bool {
        let ((lo, hi), tol) = (self.bounds, tol as i64);
        let near = |v: Nm, lo: Nm, hi: Nm| (lo as i64 - tol..=hi as i64 + tol).contains(&(v as i64));
        near(p.x, lo.x, hi.x) && near(p.y, lo.y, hi.y)
    }
}

/// How a zone joins the pads of its net; KiCad numbers them in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connect {
    None,
    Thermal,
    Solid,
    /// Thermal reliefs on through-hole pads, solid on the others.
    ThruHoleThermal,
}

/// What a zone does to its outline's convex corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Smoothing {
    None,
    Chamfer(Nm),
    Fillet(Nm),
}

/// A copper zone, or a rule area that keeps zone copper out, in board coordinates.
// shortcut: zones inside footprints (keepouts of a few antenna parts) are not read; read them,
// moving with their part, when a board needs them.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    /// Corner lists: the outline, and the cutouts inside it.
    pub outline: Vec<Vec<Pt>>,
    pub layers: LayerMask,
    pub net: NetId,
    /// Zones fill from the highest priority down; lower ones keep clear of higher ones' fill.
    pub priority: u32,
    /// Its `uuid` (KiCad 4–5: `tstamp`) as a number, 0 when it has none: of one priority, the
    /// greater fills first, as in KiCad 10.
    pub id: u128,
    /// A rule area keeping zone copper out (`copperpour not_allowed`); it has no fill.
    pub keepout: bool,
    /// Its name (KiCad 6 on; `(name …)`), escaped as written; empty for none.
    pub name: String,
    pub clearance: Nm,
    pub min_width: Nm,
    pub connect: Connect,
    pub thermal_gap: Nm,
    pub spoke_width: Nm,
    /// Islands touching no copper of the net go when smaller than this (nm²): `i64::MAX` all, 0 none.
    pub island_min: i64,
    pub smoothing: Smoothing,
    pub hatched: bool,
    /// KiCad draws its fills with an outline as wide as the minimum width (KiCad 5's way): the
    /// file holds them shrunk by half of it, `fills` as drawn.
    pub outlined: bool,
    /// Its filled areas in `Board::fills`.
    pub fills: Range<usize>,
    /// Its place among the zones of the text the board was read from, which saving edits;
    /// `None` for one added (`Board::add_zone`).
    pub src: Option<usize>,
}

/// One filled area of a copper zone as KiCad draws it, holes joined to its outline by cuts as
/// KiCad saves fills.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ZoneFill {
    pub pts: Vec<Pt>,
    pub layer: LayerId,
    pub net: NetId,
}

/// A shown text (board or footprint), in board coordinates; `font::strokes` lays it out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Text {
    /// Escapes resolved, `${VAR}`s filled in from footprint fields, board properties and the
    /// title block (unknown ones stay as written); `\n` separates lines.
    pub text: String,
    pub at: Pt,
    /// Degrees, absolute; locked footprint text is already turned upright, as KiCad shows it.
    pub angle: f64,
    /// Locked footprint text, which stays upright when its footprint turns.
    pub upright: bool,
    /// Glyph width and height.
    pub size: (Nm, Nm),
    pub thickness: Nm,
    pub layer: LayerId,
    /// Horizontal and vertical: -1 left/top, 0 centre, 1 right/bottom.
    pub justify: (i8, i8),
    pub mirror: bool,
    /// In a TrueType face or knocked out of a block: drawn here in KiCad's stroke font, so
    /// only roughly where and as wide as KiCad draws it.
    pub rough: bool,
    /// A part's: its place among the part's `fp_text`s and `property`s (hidden ones counted) in
    /// the text read, where saving finds it; the board's own: its place among the `gr_text`s
    /// shown there. `None` for one added since.
    pub src: Option<usize>,
    /// Its words were typed here, not read: saving writes them over the file's.
    pub typed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Board {
    pub version: u32,
    pub layers: Vec<String>,
    pub copper: LayerMask,
    pub stackup: Vec<CuLayer>,
    /// Board thickness, and the solder mask and silkscreen colours its stackup names (`"Black"`,
    /// `"#RRGGBBAA"`), front then back.
    pub thickness: Nm,
    pub mask_colors: [Option<String>; 2],
    pub silk_colors: [Option<String>; 2],
    pub rules: Rules,
    /// The project's net classes besides Default, and which nets each takes: a net's name or a
    /// pattern of names (`*` any run of letters, `?` any one) with its class's place in
    /// `classes`, in the project's order; a net is of the first that fits it, as in KiCad.
    pub classes: Vec<NetClass>,
    pub class_nets: Vec<(String, usize)>,
    /// The checks whose severity the project sets (`.kicad_pro` `rule_severities`), by KiCad's
    /// names for them: whether one is only a warning, or (`None`) isn't made at all.
    pub severities: Vec<(String, Option<bool>)>,
    /// The project's own track widths (`.kicad_pro` `track_widths`), which a route steps through.
    pub track_widths: Vec<Nm>,
    /// A point the grid goes through: KiCad's grid origin (Place → Grid Origin), else (0, 0).
    pub grid_origin: Pt,
    pub nets: Vec<String>,
    /// Each net's name in the text the board was read from, which saving finds it by: an edit
    /// that renames a net keeps its id.
    pub file_nets: Vec<String>,
    pub tracks: Vec<Track>,
    pub vias: Vec<Via>,
    pub pads: Vec<Pad>,
    pub footprints: Vec<Footprint>,
    pub shapes: Vec<Shape>,
    pub zones: Vec<Zone>,
    pub fills: Vec<ZoneFill>,
    pub texts: Vec<Text>,
}

impl Board {
    /// The class of net `net`, unless Default's: that of the first name or pattern that fits it.
    pub fn class(&self, net: NetId) -> Option<&NetClass> {
        let name = self.nets.get(net as usize).filter(|n| !n.is_empty())?;
        let fits = |p: &String| p == name || (p.contains(['*', '?']) && matches(p, name));
        self.class_nets.iter().find(|(p, _)| fits(p)).and_then(|&(_, c)| self.classes.get(c))
    }

    /// Each net's class by the net's id, as `class` finds it (its place in `classes`; `None`:
    /// Default), all at once: names by a table and the few patterns each tried, a large board's
    /// thousands of nets in microseconds.
    pub fn net_classes(&self) -> Vec<Option<usize>> {
        let mut out = vec![None; self.nets.len().max(1)];
        let (mut named, mut patterns) = (HashMap::new(), vec![]);
        for (i, (p, class)) in self.class_nets.iter().enumerate() {
            match p.contains(['*', '?']) {
                true => patterns.push((i, p, *class)),
                false => drop(named.entry(p.as_str()).or_insert((i, *class))),
            }
        }
        for (name, of) in self.nets.iter().zip(&mut out).skip(1).filter(|_| !self.class_nets.is_empty()) {
            let pattern = patterns.iter().find(|(_, p, _)| matches(p, name)).map(|&(i, _, class)| (i, class));
            let first = [named.get(name.as_str()).copied(), pattern].into_iter().flatten().min_by_key(|&(i, _)| i);
            *of = first.map(|(_, class)| class).filter(|&c| c < self.classes.len());
        }
        out
    }

    /// What copper of each net keeps from other nets', by the net's id: its class's clearance.
    /// Two nets keep the larger of theirs (KiCad's rule); copper without a net, Default's.
    pub fn clearances(&self) -> Vec<Nm> {
        self.net_classes().into_iter().map(|c| c.map_or(self.rules.clearance, |c| self.classes[c].clearance)).collect()
    }

    /// The width new tracks of `net` take: its class's, never below the board's minimum.
    pub fn track_width(&self, net: NetId) -> Nm {
        self.class(net).map_or(self.rules.class_track_width, |c| c.track_width).max(self.rules.track_width)
    }

    /// The net named `q`, ignoring case; failing that, the first whose last path part is `q`
    /// (`scl` finds `/Sheet/SCL`).
    pub fn find_net(&self, q: &str) -> Option<NetId> {
        let last = |n: &String| n.rsplit('/').next().unwrap_or(n).eq_ignore_ascii_case(q);
        let i = self.nets.iter().position(|n| n.eq_ignore_ascii_case(q)).or_else(|| self.nets.iter().position(last))?;
        (i > 0).then_some(i as NetId)
    }

    /// Box around every track, via, pad and zone fill on `net`; `None` when it has none.
    pub fn net_bounds(&self, net: NetId) -> Option<(Pt, Pt)> {
        let tracks = self.tracks.iter().filter(|t| t.net == net);
        let tracks = tracks.flat_map(|t| [t.a, t.b].into_iter().chain(t.mid).map(move |p| square(p, t.width / 2)));
        let vias = self.vias.iter().filter(|v| v.net == net).map(|v| square(v.at, v.size / 2));
        let pads = self.pads.iter().filter(|p| p.net == net).map(|p| square(p.at, reach(p)));
        let fills = self.fills.iter().filter(|z| z.net == net).flat_map(|z| z.pts.iter().map(|&p| square(p, 0)));
        tracks.chain(vias).chain(pads).chain(fills).reduce(union)
    }

    /// The footprint whose box holds `p` (within `tol`): the smallest, preferring the side in
    /// view (`back` when the view is flipped).
    pub fn footprint_at(&self, p: Pt, tol: Nm, back: bool) -> Option<usize> {
        let area = |(lo, hi): (Pt, Pt)| (hi.x as i64 - lo.x as i64) * (hi.y as i64 - lo.y as i64);
        let hits = self.footprints.iter().enumerate().filter(|(_, f)| f.holds(p, tol));
        hits.min_by_key(|(_, f)| (f.back != back, area(f.bounds))).map(|(i, _)| i)
    }

    /// The pad, via or track under `p` (within `tol`) that shows on top of the copper `layers`,
    /// listed as they show, top first: the one on the first layer, and on one layer a pad before
    /// a via before a track, the nearest first.
    // shortcut: an arc track is picked only near its ends; pick its middle when arcs get edited.
    pub fn copper_at(&self, p: Pt, tol: Nm, layers: &[LayerId]) -> Option<Item> {
        let depth = |m: LayerMask| layers.iter().position(|&l| m & 1 << l != 0);
        let d = |q: Pt| (q.x as f64 - p.x as f64).hypot(q.y as f64 - p.y as f64);
        let pads = self.pads.iter().enumerate().filter(|(_, q)| q.holds(p, tol)).map(|(i, q)| (q.layers, 0, d(q.at), Item::Pad(i)));
        let vias = self.vias.iter().enumerate().filter(|(_, v)| d(v.at) <= (v.size / 2 + tol) as f64);
        let vias = vias.map(|(i, v)| (v.layers, 1, d(v.at), Item::Via(i)));
        let tracks = self.tracks.iter().enumerate().filter_map(|(i, t)| {
            let on = if t.mid.is_none() { d(nearest(t.a, t.b, p)) } else { d(t.a).min(d(t.b)) };
            (on <= (t.width / 2 + tol) as f64).then_some((1 << t.layer, 2, on, Item::Track(i)))
        });
        let hits = pads.chain(vias).chain(tracks).filter_map(|(m, kind, dist, item)| Some(((depth(m)?, kind), dist, item)));
        hits.min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))).map(|h| h.2)
    }

    /// `copper_at`, as a pointer picks: what a zone's fill on a layer above hides is no more there
    /// to pick than to see (a back track under an inner plane), until its layer shows on top.
    pub fn copper_shown_at(&self, p: Pt, tol: Nm, layers: &[LayerId]) -> Option<Item> {
        let item = self.copper_at(p, tol, layers)?;
        let on = match item {
            Item::Pad(i) => self.pads[i].layers,
            Item::Via(i) => self.vias[i].layers,
            Item::Track(i) => 1 << self.tracks[i].layer,
        };
        let mut above = layers.iter().take_while(|&&l| on & 1 << l == 0);
        let hidden = above.any(|&l| self.fills.iter().any(|f| f.layer == l && inside(p, &f.pts)));
        (!hidden).then_some(item)
    }

    /// The zone a click at `p` picks on the copper `layers`, listed as they show, top first (as
    /// for `copper_at`): on the first layer one is on, one whose outline passes within `tol` of
    /// `p`, else the highest priority one whose fill there holds `p` (a rule area has none: its
    /// outline only).
    pub fn zone_at(&self, p: Pt, tol: Nm, layers: &[LayerId]) -> Option<usize> {
        let near = |(&a, &b): (&Pt, &Pt)| {
            let q = nearest(a, b, p);
            (q.x as f64 - p.x as f64).hypot(q.y as f64 - p.y as f64) <= tol as f64
        };
        let edge = |i: &usize| self.zones[*i].outline.iter().any(|c| c.iter().zip(c.iter().cycle().skip(1)).any(near));
        layers.iter().find_map(|&l| {
            let on = || (0..self.zones.len()).filter(move |&i| self.zones[i].layers & 1 << l != 0);
            let filled = |i: &usize| self.fills[self.zones[*i].fills.clone()].iter().any(|f| f.layer == l && inside(p, &f.pts));
            on().find(edge).or_else(|| on().filter(filled).max_by_key(|&i| self.zones[i].priority))
        })
    }

    /// The area zone `z`'s fills cover in mm², its layers together.
    pub fn fill_area(&self, z: usize) -> f64 {
        // The shoelace formula, from a fill's first point (smaller products); a fill's holes join
        // its outline by cuts, so they count against it.
        let area = |pts: &[Pt]| {
            let d = |p: Pt| (p.x as f64 - pts[0].x as f64, p.y as f64 - pts[0].y as f64);
            pts.iter().zip(pts.iter().cycle().skip(1)).map(|(&a, &b)| d(a).0 * d(b).1 - d(b).0 * d(a).1).sum::<f64>().abs() / 2e12
        };
        self.fills[self.zones[z].fills.clone()].iter().map(|f| area(&f.pts)).sum()
    }

    /// Moves footprint `i` to `at`, turned to `angle` degrees, with its pads, graphics and texts.
    pub fn place_footprint(&mut self, i: usize, at: Pt, angle: f64) {
        let f = &mut self.footprints[i];
        let (from, turn) = (f.at, angle - f.angle);
        let to = |p: Pt| place(Pt { x: p.x.saturating_sub(from.x), y: p.y.saturating_sub(from.y) }, at, turn);
        for p in &mut self.pads[f.pads.clone()] {
            (p.at, p.angle) = (to(p.at), (p.angle + turn).rem_euclid(360.0));
        }
        for s in &mut self.shapes[f.shapes.clone()] {
            s.geom.map(to);
        }
        for t in self.texts[f.texts.clone()].iter_mut().chain(&mut f.hidden) {
            t.at = to(t.at);
            t.angle = if t.upright { upright(t.angle + turn) } else { (t.angle + turn).rem_euclid(360.0) };
        }
        (f.at, f.angle, f.bounds) = (at, angle.rem_euclid(360.0), part_box(&self.pads[f.pads.clone()], &self.shapes[f.shapes.clone()], at));
    }

    /// Puts footprint `i` on the other side as KiCad's Flip does (top to bottom about its
    /// anchor): its pads, graphics and texts mirrored onto the other side's layers, its angle and
    /// its pads' negated, its texts mirrored at `180° - angle`; its 3D models stay as written.
    pub fn flip_footprint(&mut self, i: usize) {
        // A counterpart the board lacks (`B.Fab` of a KiCad 4 board) comes in, as reading it would.
        let mut to = vec![];
        for l in 0..self.layers.len() as LayerId {
            to.push(match self.flipped_layer(l) {
                Ok(t) => t,
                Err(_) if self.layers.len() >= LayerMask::BITS as usize => l,
                Err(name) => {
                    self.layers.push(name);
                    (self.layers.len() - 1) as LayerId
                }
            });
        }
        let mask = |m: LayerMask| (0..to.len()).filter(|&l| m >> l & 1 != 0).fold(0, |o, l| o | 1 << to[l]);
        let f = &mut self.footprints[i];
        let y = f.at.y as i64;
        let mirror = |p: Pt| Pt { x: p.x, y: (2 * y - p.y as i64) as Nm };
        for p in &mut self.pads[f.pads.clone()] {
            (p.at, p.angle, p.layers) = (mirror(p.at), (360.0 - p.angle).rem_euclid(360.0), mask(p.layers));
            if !p.custom.is_empty() {
                p.custom = joined(p.custom.iter().map(|c| c.iter().map(|q| ip(Pt { x: q.x, y: -q.y })).collect()).collect());
            }
        }
        for s in &mut self.shapes[f.shapes.clone()] {
            s.geom.map(mirror);
            s.layer = to[s.layer as usize];
        }
        for t in self.texts[f.texts.clone()].iter_mut().chain(&mut f.hidden) {
            // A mirrored text reads backwards along its angle: 180° - angle keeps it on its line.
            // One on a layer of neither side (`Dwgs.User`) is turned so, not mirrored.
            let (angle, layer) = (180.0 - t.angle, to[t.layer as usize]);
            (t.at, t.mirror, t.layer) = (mirror(t.at), t.mirror != (layer != t.layer), layer);
            t.angle = if t.upright { upright(angle) } else { angle };
        }
        (f.back, f.angle) = (!f.back, (360.0 - f.angle).rem_euclid(360.0));
        f.bounds = part_box(&self.pads[f.pads.clone()], &self.shapes[f.shapes.clone()], f.at);
    }

    /// The layer an item on layer `l` goes to when its part flips: front and back swapped
    /// (`F.SilkS`, `B.SilkS`), copper in reverse stack order, any other (`Edge.Cuts`) its own;
    /// `Err` names a counterpart the board lacks.
    pub(crate) fn flipped_layer(&self, l: LayerId) -> Result<LayerId, String> {
        if let Some(k) = self.stackup.iter().position(|c| c.layer == l) {
            return Ok(self.stackup[self.stackup.len() - 1 - k].layer);
        }
        let other = match self.layers[l as usize].split_once('.') {
            Some(("F", rest)) => format!("B.{rest}"),
            Some(("B", rest)) => format!("F.{rest}"),
            _ => return Ok(l),
        };
        self.layers.iter().position(|n| *n == other).map(|o| o as LayerId).ok_or(other)
    }

    /// The name of `flipped_layer` of the layer named `name`; `None` when that is its own
    /// (`Edge.Cuts`, `*.Cu`) or the board lacks it.
    pub(crate) fn flipped_name(&self, name: &str) -> Option<&str> {
        let l = self.layers.iter().position(|n| n == name)? as LayerId;
        let other = self.flipped_layer(l).ok().filter(|&o| o != l)?;
        Some(&self.layers[other as usize])
    }

    /// The board's own drawings and texts (not its parts'), by their places in `shapes` and
    /// `texts`: what its Edge.Cuts, silkscreen and notes hold, picked and edited one by one.
    pub fn own(&self) -> (Vec<usize>, Vec<usize>) {
        let (mut shapes, mut texts) = (vec![true; self.shapes.len()], vec![true; self.texts.len()]);
        for f in &self.footprints {
            shapes[f.shapes.clone()].fill(false);
            texts[f.texts.clone()].fill(false);
        }
        let own = |own: Vec<bool>| (0..own.len()).filter(|&i| own[i]).collect();
        (own(shapes), own(texts))
    }

    /// Whether a part whose `(layer …)` is `name` sits on the back: `B.Cu`, or (KiCad ≤ 5 named
    /// copper as users did, `C4B`) any copper below the top.
    pub(crate) fn on_back(&self, name: &str) -> bool {
        let l = self.layers.iter().position(|n| n == name);
        name == "B.Cu" || l.is_some_and(|l| self.stackup.iter().position(|c| c.layer as usize == l).is_some_and(|k| k > 0))
    }
}

/// Whether segment `p`–`q` meets the box `lo`–`hi` (what a box drawn right to left takes, on a
/// board or a sheet): their boxes meet, and the box has corners on both sides of its line, or on it.
pub(crate) fn crosses((lo, hi): (Pt, Pt), p: Pt, q: Pt) -> bool {
    let meets = p.x.min(q.x) <= hi.x && p.x.max(q.x) >= lo.x && p.y.min(q.y) <= hi.y && p.y.max(q.y) >= lo.y;
    let (dx, dy) = (q.x as i64 - p.x as i64, q.y as i64 - p.y as i64);
    let side = |c: Pt| (dx * (c.y as i64 - p.y as i64) - dy * (c.x as i64 - p.x as i64)).signum();
    let sides = [lo, Pt { x: hi.x, y: lo.y }, hi, Pt { x: lo.x, y: hi.y }].map(side);
    meets && sides.iter().any(|&s| s >= 0) && sides.iter().any(|&s| s <= 0)
}

/// The point of segment `a`–`b` nearest `p`.
pub(crate) fn nearest(a: Pt, b: Pt, p: Pt) -> Pt {
    let (dx, dy) = (b.x as f64 - a.x as f64, b.y as f64 - a.y as f64);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.x as f64 - a.x as f64) * dx + (p.y as f64 - a.y as f64) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    Pt { x: a.x + (t * dx).round() as Nm, y: a.y + (t * dy).round() as Nm }
}

/// Box of half-size `r` around `p`.
pub(crate) fn square(p: Pt, r: Nm) -> (Pt, Pt) {
    (Pt { x: p.x.saturating_sub(r), y: p.y.saturating_sub(r) }, Pt { x: p.x.saturating_add(r), y: p.y.saturating_add(r) })
}

/// Box around a graphic, its pen included.
pub(crate) fn shape_box(s: &Shape) -> (Pt, Pt) {
    let pts: Vec<Pt> = match &s.geom {
        Geom::Segment { a, b } => vec![*a, *b],
        Geom::Arc { start, mid, end } => arc_points(*start, *mid, *end),
        Geom::Circle { center: c, radius: r } => vec![Pt { x: c.x - r, y: c.y - r }, Pt { x: c.x + r, y: c.y + r }],
        Geom::Poly(pts) => pts.clone(),
    };
    let (lo, hi) = pts.iter().map(|&p| (p, p)).reduce(union).unwrap_or_default();
    let w = s.width / 2;
    (Pt { x: lo.x - w, y: lo.y - w }, Pt { x: hi.x + w, y: hi.y + w })
}

/// Smallest box holding both.
pub(crate) fn union(a: (Pt, Pt), b: (Pt, Pt)) -> (Pt, Pt) {
    (Pt { x: a.0.x.min(b.0.x), y: a.0.y.min(b.0.y) }, Pt { x: a.1.x.max(b.1.x), y: a.1.y.max(b.1.y) })
}

/// Half the pad's diagonal, or as far as a custom pad's copper goes: covers it at any angle.
pub(crate) fn reach(p: &Pad) -> Nm {
    let far = p.custom.iter().flatten().map(|q| (q.x as f64).hypot(q.y as f64));
    far.fold((p.size.0 as f64).hypot(p.size.1 as f64) / 2.0, f64::max).round() as Nm
}

/// Corners of the box around a part's pads and anchor; a part without pads (a logo) boxes its
/// graphics, so a click anywhere on them finds it.
pub(crate) fn part_box(pads: &[Pad], shapes: &[Shape], anchor: Pt) -> (Pt, Pt) {
    if !pads.is_empty() {
        return pads.iter().map(|p| square(p.at, reach(p))).fold(square(anchor, 0), union);
    }
    let corners = |s: &Shape| match &s.geom {
        Geom::Segment { a, b } => vec![*a, *b],
        Geom::Arc { start, mid, end } => arc_points(*start, *mid, *end),
        Geom::Circle { center: c, radius: r } => vec![Pt { x: c.x - r, y: c.y - r }, Pt { x: c.x + r, y: c.y + r }],
        Geom::Poly(pts) => pts.clone(),
    };
    shapes.iter().flat_map(|s| corners(s).into_iter().map(|p| square(p, s.width / 2))).fold(square(anchor, 0), union)
}

/// KiCad keeps locked footprint text readable: angles outside (-90°, 90°] turn half a turn.
pub(crate) fn upright(deg: f64) -> f64 {
    90.0 - (90.0 - deg).rem_euclid(180.0)
}

/// Rotates like KiCad: positive degrees turn counter-clockwise on screen (Y points down).
pub fn rotate(p: Pt, deg: f64) -> Pt {
    let d = deg.rem_euclid(360.0);
    // Exact for right angles, which are most footprint rotations.
    let (s, c) = if d == 0.0 {
        return p;
    } else if d == 90.0 {
        (1.0, 0.0)
    } else if d == 180.0 {
        (0.0, -1.0)
    } else if d == 270.0 {
        (-1.0, 0.0)
    } else {
        d.to_radians().sin_cos()
    };
    let (x, y) = (p.x as f64, p.y as f64);
    Pt { x: (x * c + y * s).round() as Nm, y: (y * c - x * s).round() as Nm }
}

/// Centre, radius, start angle and signed sweep (radians) of the circular arc through three
/// points; `None` when they are collinear.
pub fn arc_center(a: Pt, m: Pt, b: Pt) -> Option<(f64, f64, f64, f64, f64)> {
    // Work relative to `a` so the squares stay small enough for f64.
    let (mx, my) = ((m.x - a.x) as f64, (m.y - a.y) as f64);
    let (bx, by) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let d = 2.0 * (mx * by - my * bx);
    if d.abs() < 1.0 {
        return None;
    }
    let (m2, b2) = (mx * mx + my * my, bx * bx + by * by);
    let (ux, uy) = ((by * m2 - my * b2) / d, (mx * b2 - bx * m2) / d);
    let (cx, cy) = (a.x as f64 + ux, a.y as f64 + uy);
    let ang = |p: Pt| (p.y as f64 - cy).atan2(p.x as f64 - cx);
    let a0 = ang(a);
    let ccw = (ang(b) - a0).rem_euclid(TAU);
    let sweep = if (ang(m) - a0).rem_euclid(TAU) <= ccw { ccw } else { ccw - TAU };
    Some((cx, cy, ux.hypot(uy), a0, sweep))
}

/// How far a curve's straight pieces may stray from it (KiCad's default).
pub(crate) const MAX_ERROR: f64 = 5_000.0;

/// Straight pieces for `sweep` radians of a circle of radius `r`, short enough that pieces
/// touching it (or with their corners on it) stay within `MAX_ERROR` of it.
pub(crate) fn segments(r: f64, sweep: f64) -> usize {
    let step = if r <= MAX_ERROR { FRAC_PI_2 } else { 2.0 * (r / (r + MAX_ERROR)).acos() };
    (sweep.abs() / step).ceil().max(1.0) as usize
}

/// Points along the arc through `s`, `m` and `e`, its ends exact.
pub fn arc_points(s: Pt, m: Pt, e: Pt) -> Vec<Pt> {
    let Some((cx, cy, r, a0, sweep)) = arc_center(s, m, e) else { return vec![s, m, e] };
    let n = segments(r, sweep);
    let mut v: Vec<Pt> = (0..=n)
        .map(|i| {
            let a = a0 + sweep * i as f64 / n as f64;
            Pt { x: (cx + r * a.cos()).round() as Nm, y: (cy + r * a.sin()).round() as Nm }
        })
        .collect();
    (v[0], v[n]) = (s, e);
    v
}

pub(crate) fn place(p: Pt, at: Pt, deg: f64) -> Pt {
    let r = rotate(p, deg);
    Pt { x: r.x.saturating_add(at.x), y: r.y.saturating_add(at.y) }
}

impl Geom {
    pub(crate) fn map(&mut self, f: impl Fn(Pt) -> Pt) {
        match self {
            Geom::Segment { a, b } => (*a, *b) = (f(*a), f(*b)),
            Geom::Arc { start, mid, end } => (*start, *mid, *end) = (f(*start), f(*mid), f(*end)),
            Geom::Circle { center, .. } => *center = f(*center),
            Geom::Poly(pts) => pts.iter_mut().for_each(|p| *p = f(*p)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn finds_footprints_and_nets_by_name() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal))
              (net 0 "") (net 1 "/Sheet/SCL") (net 2 "GND")
              (footprint "Resistor_SMD:R_0603" (layer "F.Cu") (at 10 20) (attr smd)
                (property "Reference" "R7" (at 0 -1) (layer "F.SilkS")) (property "Value" "4.7k")
                (pad "1" smd rect (at -1 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "/Sheet/SCL"))
                (pad "2" smd rect (at 1 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "GND")))
              (module C (layer F.Cu) (at 0 0) (attr virtual) (fp_text reference C3 (at 0 0) (layer F.SilkS)) (fp_text value 1u))
              (segment (start 11 20) (end 15 20) (width 0.2) (layer "F.Cu") (net 1))
              (via (at 15 22) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1)))"#,
        )
        .unwrap();
        let mm = |x: f64, y: f64| Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm };
        // The pads are 0.6 x 0.8 mm, so each one is covered 0.5 mm (half its diagonal) around.
        let (r7, c3) = (&b.footprints[0], &b.footprints[1]);
        assert_eq!((r7.reference.as_str(), r7.value.as_str(), r7.lib_id.as_str(), r7.fitted), ("R7", "4.7k", "Resistor_SMD:R_0603", true));
        assert_eq!((c3.reference.as_str(), c3.value.as_str(), c3.lib_id.as_str(), c3.fitted), ("C3", "1u", "C", false));
        assert_eq!((r7.bounds, c3.bounds), ((mm(8.5, 19.5), mm(11.5, 20.5)), (mm(0.0, 0.0), mm(0.0, 0.0))));
        let found = ["scl", "/sheet/SCL", "Gnd", "Sheet", ""].map(|q| b.find_net(q));
        assert_eq!(found, [Some(1), Some(1), Some(2), None, None]);
        assert_eq!((b.net_bounds(1), b.net_bounds(3)), (Some((mm(8.5, 19.5), mm(15.3, 22.3))), None));
    }

    #[test]
    fn picks_the_smallest_part_on_the_side_in_view() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
              (footprint "J" (layer "F.Cu") (at 10 20) (pad "1" smd rect (at 0 0) (size 4 4) (layers "F.Cu")))
              (footprint "R" (layer "F.Cu") (at 11 20) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))
              (footprint "C" (layer "B.Cu") (at 11 20) (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "B.Cu")))
              (footprint "LOGO" (layer "F.Cu") (at 30 20 90) (fp_poly (pts (xy -2 -1) (xy 2 -1) (xy 2 1)) (layer "F.SilkS"))))"#,
        )
        .unwrap();
        let at = |x: f64, y, back| b.footprint_at(Pt { x: (x * 1e6) as Nm, y: (y * 1e6) as Nm }, MM / 100, back);
        // At (11, 20) the smaller front part wins from the front and the back part when flipped; a
        // logo, without pads, is found on its graphics, turned with it.
        assert_eq!(
            [at(11.0, 20.0, false), at(11.0, 20.0, true), at(9.0, 20.0, false), at(30.9, 21.9, false), at(31.5, 20.0, false)],
            [Some(1), Some(2), Some(0), Some(3), None]
        );
        assert!(b.footprints[2].back && !b.footprints[1].back);
    }

    #[test]
    fn picks_the_copper_that_shows_on_top() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "B")
              (footprint "C" (layer "B.Cu") (at 5 0) (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu") (net 2 "B")))
              (footprint "J" (layer "F.Cu") (at 0 0) (pad "1" thru_hole circle (at 0 0) (size 1.5 1.5) (drill 0.8) (layers "*.Cu") (net 1 "A")))
              (segment (start 0 0) (end 10 0) (width 0.25) (layer "F.Cu") (net 1))
              (via (at 10 0) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1)))"#,
        )
        .unwrap();
        let at = |x: f64, y: f64, layers: &[LayerId]| b.copper_at(Pt { x: (x * 1e6) as Nm, y: (y * 1e6) as Nm }, MM / 20, layers);
        let back = b.layers.iter().position(|l| l == "B.Cu").unwrap() as LayerId;
        let (front, flipped) = ([0, back], [back, 0]);
        // The front track hides the back pad under it, unless B.Cu shows on top (the view flipped,
        // or B.Cu active) or F.Cu is hidden.
        assert_eq!(
            [at(5.0, 0.0, &front), at(5.0, 0.0, &flipped), at(5.0, 0.0, &[back])],
            [Some(Item::Track(0)), Some(Item::Pad(0)), Some(Item::Pad(0))]
        );
        // Where a track ends on a pad or via, that is picked; off the track, the pad beside it.
        assert_eq!(
            [at(0.0, 0.0, &front), at(10.0, 0.0, &front), at(5.0, 0.3, &front), at(5.0, 3.0, &front)],
            [Some(Item::Pad(1)), Some(Item::Via(0)), Some(Item::Pad(0)), None]
        );
    }

    #[test]
    fn a_pointer_picks_no_copper_a_plane_above_hides() {
        // A back track under an inner plane, which a hole and a via reach through.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "GND")
              (segment (start 0 5) (end 20 5) (width 0.25) (layer "B.Cu") (net 1))
              (via (at 8 5) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1))
              (zone (net 2) (layer "In1.Cu") (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))
                (filled_polygon (layer "In1.Cu") (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))))"#,
        )
        .unwrap();
        let layer = |name: &str| b.layers.iter().position(|l| l == name).unwrap() as LayerId;
        let (front, inner, back) = (layer("F.Cu"), layer("In1.Cu"), layer("B.Cu"));
        let at = |x: f64, layers: &[LayerId]| b.copper_shown_at(Pt { x: (x * 1e6) as Nm, y: 5 * MM }, MM / 20, layers);
        // Under the plane it isn't there to pick, as it isn't to see; past it, or with the plane
        // hidden, or with its own layer on top (active, or the view flipped), it is.
        assert_eq!(b.copper_at(Pt { x: 5 * MM, y: 5 * MM }, MM / 20, &[front, inner, back]), Some(Item::Track(0)));
        assert_eq!(
            [at(5.0, &[front, inner, back]), at(15.0, &[front, inner, back]), at(5.0, &[front, back]), at(5.0, &[back, front, inner])],
            [None, Some(Item::Track(0)), Some(Item::Track(0)), Some(Item::Track(0))]
        );
        // A via shows on every layer it spans.
        assert_eq!(at(8.0, &[front, inner, back]), Some(Item::Via(0)));
    }

    #[test]
    fn picks_zones_by_their_fill_or_outline_from_the_side_in_view() {
        // Front: A's zone filled but for a hole, a priority 2 zone of B filled inside it, a keepout
        // with no fill; back: A's zone across the board.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "B")
              (zone (net 1) (layer "F.Cu") (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))
                (filled_polygon (layer "F.Cu") (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10) (xy 0 5) (xy 2 5) (xy 2 7) (xy 4 7) (xy 4 5) (xy 0 5))))
              (zone (net 2) (layer "F.Cu") (priority 2) (polygon (pts (xy 6 6) (xy 9 6) (xy 9 9)))
                (filled_polygon (layer "F.Cu") (pts (xy 6 6) (xy 9 6) (xy 9 9))))
              (zone (net 0) (layer "F.Cu") (keepout (copperpour not_allowed)) (polygon (pts (xy 20 0) (xy 30 0) (xy 30 10) (xy 20 10))))
              (zone (net 1) (layer "B.Cu") (polygon (pts (xy 0 0) (xy 30 0) (xy 30 10)))
                (filled_polygon (layer "B.Cu") (pts (xy 0 0) (xy 30 0) (xy 30 10)))))"#,
        )
        .unwrap();
        let at = |x: f64, y: f64, layers: &[LayerId]| b.zone_at(Pt { x: (x * 1e6) as Nm, y: (y * 1e6) as Nm }, MM / 20, layers);
        // The copper layers in view, top first: from the front, from the back, the back alone.
        let back = b.layers.iter().position(|l| l == "B.Cu").unwrap() as LayerId;
        let (all, flipped, bottom) = ([0, back], [back, 0], [back]);
        // Fills on top first, the higher priority where two hold the point; none in A's hole; a
        // keepout by its outline only (inside it, the back's fill under it).
        assert_eq!(
            [at(1.0, 2.0, &all), at(8.5, 6.5, &all), at(3.0, 6.0, &all), at(25.0, 5.0, &all), at(20.03, 5.0, &all)],
            [Some(0), Some(1), None, Some(3), Some(2)]
        );
        // Where both sides have one: the front's, unless seen from the back or the front hidden.
        assert_eq!(
            [at(8.0, 2.0, &all), at(8.0, 2.0, &flipped), at(8.0, 2.0, &bottom), at(15.0, 0.02, &all)],
            [Some(0), Some(3), Some(3), Some(3)]
        );
        // Their filled areas, holes taken out: 100 - 4, 4.5, none, 150 mm².
        assert_eq!((0..4).map(|z| b.fill_area(z)).collect::<Vec<_>>(), [96.0, 4.5, 0.0, 150.0]);
    }

    #[test]
    fn places_footprints_with_their_pads_graphics_and_texts() {
        let src = r#"(kicad_pcb (layers (0 "F.Cu" signal) (37 "F.SilkS" user))
              (gr_line (start 0 0) (end 1 0) (stroke (width 0.1)) (layer "F.SilkS"))
              (footprint "R" (layer "F.Cu") (at 10 20)
                (property "Reference" "R1" (at 0 -1 0) (layer "F.SilkS"))
                (fp_text user "U" (at 0 1 0 unlocked) (layer "F.SilkS"))
                (fp_line (start -1 0) (end 1 0) (stroke (width 0.1)) (layer "F.SilkS"))
                (pad "1" smd rect (at 1 0) (size 1 0.5) (layers "F.Cu"))
                (pad "2" smd rect (at -1 0 -90) (size 1 0.5) (layers "F.Cu"))))"#;
        let (orig, mut b) = (Board::parse(src).unwrap(), Board::parse(src).unwrap());
        let f = &b.footprints[0];
        assert_eq!(
            (f.at, f.angle, f.pads.clone(), f.shapes.clone(), f.texts.clone()),
            (Pt { x: 10 * MM, y: 20 * MM }, 0.0, 0..2, 1..2, 0..2)
        );
        b.place_footprint(0, Pt { x: 20 * MM, y: 30 * MM }, 90.0);
        // Local (1, 0) turned 90° counter-clockwise on screen is (0, -1).
        let pt = |x, y| Pt { x: x * MM, y: y * MM };
        assert_eq!((b.pads[0].at, b.pads[0].angle), (pt(20, 29), 90.0));
        // The 1 x 0.5 mm pad now stands upright.
        let near = |x, y, tol| b.pads[0].holds(Pt { x: 20 * MM + x, y: 29 * MM + y }, tol);
        assert_eq!([near(0, 400_000, 0), near(400_000, 0, 0), near(400_000, 0, 200_000)], [true, false, true]);
        assert_eq!(
            (b.shapes[0].geom.clone(), b.shapes[1].geom.clone()),
            (orig.shapes[0].geom.clone(), Geom::Segment { a: pt(20, 31), b: pt(20, 29) })
        );
        assert_eq!([(b.texts[0].at, b.texts[0].angle), (b.texts[1].at, b.texts[1].angle)], [(pt(19, 30), 90.0), (pt(21, 30), 90.0)]);
        // Another half turn: the locked reference stays readable, the unlocked text doesn't.
        b.place_footprint(0, pt(20, 30), 270.0);
        let bounds = (Pt { x: 19_440_983, y: 28_440_983 }, Pt { x: 20_559_017, y: 31_559_017 });
        assert_eq!((b.texts[0].angle, b.texts[1].angle, b.footprints[0].bounds), (90.0, 270.0, bounds));
        // Back where it was: nothing changed.
        b.place_footprint(0, pt(10, 20), 0.0);
        assert_eq!((&b.footprints, crate::diff::Diff::between(&orig, &b).changes), (&orig.footprints, vec![]));
    }

    #[test]
    fn custom_pads_are_clicked_and_boxed_by_all_their_copper() {
        // An anchor with an arm along its top, turned 90°: the arm runs up, x 9.5 to 9.9, y 9.5 to 7.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal)) (net 0 "") (net 1 "A")
              (footprint "X" (layer "F.Cu") (at 10 10 90) (pad "1" smd custom (at 0 0 90) (size 1 1) (layers "F.Cu") (net 1 "A")
                (options (clearance outline) (anchor rect)) (primitives (gr_poly (pts (xy 0.5 -0.5) (xy 3 -0.5) (xy 3 -0.1) (xy 0.5 -0.1)) (width 0) (fill yes))))))"#,
        )
        .unwrap();
        let pt = |x: f64, y: f64| Pt { x: (x * 1e6) as Nm, y: (y * 1e6) as Nm };
        let holds = |x, y, tol| b.pads[0].holds(pt(x, y), tol);
        // On the arm, 0.1 mm beside it, on the anchor; beside it within 0.2 mm.
        assert_eq!([holds(9.7, 7.5, 0), holds(9.4, 7.5, 0), holds(10.0, 10.0, 0), holds(9.4, 7.5, 200_000)], [true, false, true, true]);
        assert_eq!(b.copper_at(pt(9.7, 7.5), 0, &[0]), Some(Item::Pad(0)));
        // The part's box takes in the arm's end.
        assert_eq!(b.footprint_at(pt(9.7, 7.1), 0, false), Some(0));
    }

    #[test]
    fn arc_center_finds_the_circle() {
        let (x, y, r, _, sweep) = arc_center(Pt { x: -MM, y: 0 }, Pt { x: 0, y: MM }, Pt { x: MM, y: 0 }).unwrap();
        assert!(x.abs() < 1e-6 && y.abs() < 1e-6 && (r - MM as f64).abs() < 1e-6 && (sweep.abs() - PI).abs() < 1e-6);
        assert_eq!(arc_center(Pt::default(), Pt { x: 1, y: 1 }, Pt { x: 2, y: 2 }), None);
    }
}
