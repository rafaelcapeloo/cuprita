//! DRC's silkscreen checks, as KiCad lists them (warnings there): ink over a solder mask
//! opening, which the fab clips off (`silk_over_copper`); ink over other ink (`silk_overlap`);
//! ink across the board's edge (`silk_edge_clearance`). Each graphic and each text on F.SilkS
//! or B.SilkS is one thing printed, by its strokes at their width and its filled areas.

use super::{Copper, Item, RBox, RTree, Violation, arc_pts, cu_label, edges, pad_box, pad_label, viol};
use crate::board::{Board, Geom, Nm, Pt, Shape, shape_box, union};
use crate::tr::{t, tr, trf};
use std::collections::HashSet;
use std::f64::consts::TAU;

pub(super) const KINDS: [&str; 3] = ["silk_over_copper", "silk_overlap", "silk_edge_clearance"];

/// How deep a curve must lie over something to be told: KiCad's arcs and circles are chords
/// true to half its 5 µm either way, so one that touches a line by less isn't found there.
const SKIN: Nm = 3_000;

/// A thing printed: a graphic or a text.
struct Ink {
    /// Front (0) or back (1).
    side: usize,
    /// A graphic's part (`Some(None)`: the board's own); none for a text. Two graphics of one
    /// part are drawn to meet (a body's outline), and aren't held against each other.
    drawn_by: Option<Option<usize>>,
    /// How deep a text must lie over something to be told (0: a graphic).
    // shortcut: a text is drawn in a stroke font near KiCad's, not it (a twentieth of its height
    // wider or narrower): that much slack, until KiCad's own glyphs are drawn.
    skin: Nm,
    /// The box round all of it.
    bounds: (Pt, Pt),
    what: String,
}

/// Whether ink `a` lies over `c` (ink, copper or an edge) deeper than `skin`, or nearer than `keeps`.
fn lies_over(a: &Copper, c: &Copper, keeps: Nm, skin: Nm) -> bool {
    match (a, c) {
        (Copper::Shape(x), Copper::Shape(y)) => x.collides(y, keeps - skin),
        // An area's rim isn't measured into: its stroke thinned by the skin still on it.
        (Copper::Shape(x), area) | (area, Copper::Shape(x)) => {
            let d = area.dist(&RBox { r: (x.r - skin as f64).max(0.0), ..*x });
            d <= 0 || d < keeps
        }
        _ => a.collides(c, keeps),
    }
}

/// A piece of ink or copper: a stroke at its width or a filled area, with its box and whether
/// it is a chord of a curve.
type Piece = (Copper<'static>, (Pt, Pt), bool);

fn stroke(a: Pt, b: Pt, r: Nm, curved: bool) -> Piece {
    let s = RBox::segment(a, b, r);
    (Copper::Shape(s), s.bounds, curved)
}

/// The pieces of graphic `s`: its lines, its arcs and circles as chords, its area when filled.
fn pieces(s: &Shape) -> Vec<Piece> {
    let (r, mut out) = (s.width / 2, vec![]);
    let mut line = |pts: &[Pt], curved: bool| out.extend(pts.windows(2).map(|w| stroke(w[0], w[1], r, curved)));
    match &s.geom {
        Geom::Segment { a, b } => line(&[*a, *b], false),
        Geom::Arc { start, mid, end } => line(&arc_pts(*start, *mid, *end), true),
        Geom::Circle { center, radius } if s.filled => {
            let disc = RBox::disc(*center, radius + r);
            out.push((Copper::Shape(disc), disc.bounds, false));
        }
        Geom::Circle { center, radius } => {
            let at = |k: usize| (TAU * k as f64 / 64.0).sin_cos();
            let round =
                |k| Pt { x: center.x + (*radius as f64 * at(k).1).round() as Nm, y: center.y + (*radius as f64 * at(k).0).round() as Nm };
            line(&(0..=64).map(round).collect::<Vec<_>>(), true);
        }
        Geom::Poly(pts) => {
            if !s.filled || r > 0 {
                line(&pts.iter().chain(pts.first()).copied().collect::<Vec<_>>(), false);
            }
            if let (true, Some(bounds)) = (s.filled, pts.iter().map(|&p| (p, p)).reduce(union)) {
                out.push((Copper::Outline(vec![pts.clone()]), bounds, false));
            }
        }
    }
    out
}

/// The silkscreen's violations, of each kind the project doesn't ignore: all of them, or with
/// `near` those in its boxes (where a change put or took ink or pads). Each is told where the
/// boxes of its two things meet, so that one is in a box of `near` whenever either thing is
/// changed: the others stay as they were found.
pub(super) fn check(b: &Board, near: Option<&[(Pt, Pt)]>) -> Vec<Violation> {
    let told = |kind: &str| !b.severities.iter().any(|(k, how)| k == kind && how.is_none());
    let (over, overlap, edge) = (told(KINDS[0]), told(KINDS[1]), told(KINDS[2]));
    let layer = |name: &str| b.layers.iter().position(|l| l == name);
    let silk = [layer("F.SilkS"), layer("B.SilkS")];
    let side = |l: u8| silk.iter().position(|&s| s == Some(l as usize));
    if !(over || overlap || edge) || silk == [None, None] {
        return vec![];
    }
    // A part's hundreds of pads are as many boxes: found by a tree of them.
    let boxes = near.map(|boxes| RTree::new(boxes.iter().enumerate().map(|(k, &n)| (n, k as u32)).collect()));
    let looked = |at: (Pt, Pt)| {
        let mut any = boxes.is_none();
        boxes.iter().for_each(|tree| tree.query(at, |_| any = true));
        any
    };
    let meet =
        |(a, z): (Pt, Pt), (c, y): (Pt, Pt)| super::mid(Pt { x: a.x.max(c.x), y: a.y.max(c.y) }, Pt { x: z.x.min(y.x), y: z.y.min(y.y) });
    // Every piece of the ink looked at, with whose it is.
    let (mut inks, mut ink): (Vec<Ink>, Vec<(usize, Piece)>) = (vec![], vec![]);
    let mut part = vec![None; b.shapes.len()];
    for (i, f) in b.footprints.iter().enumerate() {
        part[f.shapes.clone()].fill(Some(i));
    }
    for (i, s) in b.shapes.iter().enumerate() {
        let (Some(side), bounds) = (side(s.layer), shape_box(s)) else { continue };
        if looked(bounds) {
            let what = part[i].map_or(tr("A silkscreen graphic").to_string(), |f| trf("{0}'s silkscreen", &[&b.footprints[f].reference]));
            ink.extend(pieces(s).into_iter().map(|p| (inks.len(), p)));
            inks.push(Ink { side, drawn_by: Some(part[i]), skin: 0, bounds, what });
        }
    }
    // shortcut: a text in a TrueType face or knocked out is drawn here only roughly (the stroke
    // font): left out, lest it be said to cross what it doesn't; read its outlines when a board needs it.
    for t in b.texts.iter().filter(|t| !t.rough) {
        let (Some(side), bounds) = (side(t.layer), pad_box(crate::font::bounds(t), t.thickness)) else { continue };
        if looked(bounds) {
            let said = t.text.lines().next().unwrap_or_default();
            crate::font::strokes(t, |pts| ink.extend(pts.windows(2).map(|w| (inks.len(), stroke(w[0], w[1], t.thickness / 2, false)))));
            inks.push(Ink { side, drawn_by: None, skin: t.size.1 / 20, bounds, what: trf("Text {0}", &[&format!("{said:?}")]) });
        }
    }
    let tree = RTree::new(ink.iter().enumerate().map(|(k, (_, p))| (p.1, k as u32)).collect());
    let skin = |curved: bool| if curved { SKIN } else { 0 };
    let mut out = vec![];
    let mut tell = |kind, item, at: Pt, keeps: Nm, what: String, fix: &'static str| {
        if looked((at, at)) {
            out.push(viol(kind, [item, None], at, 0, keeps, what, tr(fix).to_string()));
        }
    };

    // Ink over an opening in the mask: a pad's that lists the mask there, a via's where vias
    // aren't tented. The fab prints none of it: a reference half gone, a pin 1 mark lost.
    // shortcut: openings drawn on the mask layers themselves aren't looked at.
    if over {
        let masks = [layer("F.Mask"), layer("B.Mask")].map(|l| l.map_or(0, |l| 1 << l));
        let (outer, tented) =
            ([b.stackup.first(), b.stackup.last()].map(|c| c.map_or(0, |c| 1 << c.layer)), [b.rules.tented.0, b.rules.tented.1]);
        let pads = b
            .pads
            .iter()
            .enumerate()
            .filter(|(_, p)| p.layers & (masks[0] | masks[1]) != 0)
            .map(|(i, p)| (Item::Pad(i), RBox::pad(p).bounds, [0, 1].map(|s| p.layers & masks[s] != 0)));
        let vias = b
            .vias
            .iter()
            .enumerate()
            .filter(|_| tented != [true, true])
            .map(|(i, v)| (Item::Via(i), pad_box((v.at, v.at), v.size / 2), [0, 1].map(|s| !tented[s] && v.layers & outer[s] != 0)));
        for (item, bounds, open) in pads.chain(vias).filter(|o| looked(o.1)) {
            let copper = match item {
                Item::Pad(i) if !b.pads[i].custom.is_empty() => Copper::Outline(b.pads[i].copper()),
                Item::Pad(i) => Copper::Shape(RBox::pad(&b.pads[i])),
                Item::Via(i) => Copper::Shape(RBox::disc(b.vias[i].at, b.vias[i].size / 2)),
                Item::Track(_) => continue,
            };
            let mut clipped: Vec<usize> = vec![];
            tree.query(bounds, |k| {
                let (i, (piece, _, curved)) = &ink[k as usize];
                if open[inks[*i].side] && !clipped.contains(i) && lies_over(piece, &copper, 0, inks[*i].skin.max(skin(*curved))) {
                    clipped.push(*i);
                    let name = if let Item::Pad(p) = item { pad_label(b, &b.pads[p]) } else { cu_label(b, item) };
                    let what = trf("{0} is printed over {1}, where there is no mask to print on", &[&inks[*i].what, &name]);
                    tell(
                        KINDS[0],
                        Some(item),
                        meet(inks[*i].bounds, bounds),
                        0,
                        what,
                        t!("Move the text or the part: the fab leaves ink off pads"),
                    );
                }
            });
        }
    }

    // Ink over other ink, nearer than the silkscreen's clearance (none: touching).
    if overlap {
        let (keeps, mut found) = (b.rules.silk_clearance, HashSet::new());
        for (k, (i, (piece, at, curved))) in ink.iter().enumerate() {
            tree.query(pad_box(*at, keeps), |j| {
                let (o, (other, _, round)) = &ink[j as usize];
                let (a, c) = (&inks[*i], &inks[*o]);
                // Each pair once, from its first; a part's own graphics, or the board's, meet as drawn.
                if j as usize <= k
                    || o == i
                    || a.side != c.side
                    || (a.drawn_by.is_some() && a.drawn_by == c.drawn_by)
                    || found.contains(&(*i, *o))
                {
                    return;
                }
                if lies_over(piece, other, keeps, a.skin.max(c.skin).max(skin(*curved || *round))) {
                    found.insert((*i, *o));
                    let what = trf("{0} and {1} are printed over each other", &[&a.what, &c.what]);
                    tell(KINDS[1], None, meet(a.bounds, c.bounds), keeps, what, t!("Move one of them: where they cross neither reads"));
                }
            });
        }
    }

    // Ink across the board's edge, where the fab cuts it.
    if edge {
        let (keeps, mut cut) = (b.rules.silk_clearance, HashSet::new());
        for e in edges(b) {
            tree.query(pad_box(e.bounds, keeps), |k| {
                let (i, (piece, _, curved)) = &ink[k as usize];
                if !cut.contains(i) && lies_over(piece, &Copper::Shape(e), keeps, inks[*i].skin.max(skin(*curved))) {
                    cut.insert(*i);
                    let (a, what) = (&inks[*i], trf("{0} is printed across the board's edge", &[&inks[*i].what]));
                    tell(KINDS[2], None, super::mid(a.bounds.0, a.bounds.1), keeps, what, t!("Move it inside the board: the edge cuts it"));
                }
            });
        }
    }
    out
}

/// What names a kind of these over a list of them.
pub(super) fn kind_name(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "silk_over_copper" => t!("Silkscreen over pads"),
        "silk_overlap" => t!("Silkscreen over silkscreen"),
        "silk_edge_clearance" => t!("Silkscreen across the edge"),
        _ => return None,
    })
}
