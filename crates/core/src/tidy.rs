//! Tidy texts: each part's silkscreen reference put where it reads (upright at KiCad's size, just
//! outside its courtyard, clear of exposed copper, other silkscreen and the board's edge) and its
//! value inside its body on the Fab layer; a text with no room is hidden (`crates/core/tidy.md`).

use crate::board::{Board, Footprint, LayerId, MM, Nm, Pt, Text, shape_box, union};
use crate::drc::{RBox, RTree, edges, mid, overlaps, pad_box};
use crate::font;
use crate::history::Patch;
use crate::polygon::{inside, loops, path};
use std::collections::{HashMap, HashSet};

/// KiCad's silkscreen text, JLCPCB's smallest: 1 mm high, a 0.15 mm pen.
const SIZE: Nm = MM;
const PEN: Nm = 150_000;
/// How far a reference keeps from solder mask openings (JLCPCB's silkscreen to pad clearance),
/// from other silkscreen and its part's courtyard, and inside the board's edge.
const MASK: Nm = 150_000;
const GAP: Nm = 100_000;
const EDGE: Nm = 250_000;
/// The steps a reference slides along its part and away from it (`RINGS` of them), and how much
/// farther it goes to stay level.
const STEP: Nm = 250_000;
const RINGS: Nm = 3;
const LEVEL: i64 = MM as i64;
/// A Fab text's pen, of its height (KiCad's), and the smallest a value shrinks to.
const FAB_PEN: f64 = 0.15;
const FAB_MIN: Nm = 100_000;
/// KiCad's line pitch and a line's box, in heights (`font.rs`).
const PITCH: f64 = 1.61;
const LINE: f64 = 1.17;

/// What `Board::tidy_texts` did: the references it placed and hid, the values it put in their
/// parts and hid, and the parts whose texts changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tidied {
    pub placed: usize,
    pub hidden: usize,
    pub values: usize,
    pub values_hidden: usize,
    pub parts: usize,
}

impl Board {
    /// Tidies the texts of `parts` (every part when none is given), as one edit: each one's
    /// silkscreen reference upright (0° or 90°) at KiCad's size (1 mm, a 0.15 mm pen; a bigger
    /// one keeps its own), at the first spot round its courtyard (else its pads' box) clear of
    /// solder mask openings, other silkscreen and the board's edge, else hidden; its value on the
    /// Fab layer upright in the middle of its body, below its reference there, shrunk to fit,
    /// else hidden. Returns the patch that undoes it, and what it did.
    pub fn tidy_texts(&mut self, parts: &[usize]) -> (Patch, Tidied) {
        let mut parts = if parts.is_empty() { (0..self.footprints.len()).collect() } else { parts.to_vec() };
        parts.sort_unstable();
        parts.dedup();
        // Each part's texts as they go: `None` for one hidden.
        let mut texts: Vec<Vec<Option<Text>>> =
            parts.iter().map(|&i| self.texts[self.footprints[i].texts.clone()].iter().cloned().map(Some).collect()).collect();
        let mut done = Tidied::default();
        self.references(&parts, &mut texts, &mut done);
        for (&i, texts) in parts.iter().zip(&mut texts) {
            self.value(&self.footprints[i], texts, &mut done);
        }
        let (undo, changed) = self.set_texts(&parts, texts);
        (undo, Tidied { parts: changed, ..done })
    }

    /// Puts `texts` in place of the texts of `parts` (in order, each once), a `None` hiding the
    /// one it stands for (saving writes it hidden). Returns the patch that undoes it and how many
    /// parts changed.
    pub(crate) fn set_texts(&mut self, parts: &[usize], texts: Vec<Vec<Option<Text>>>) -> (Patch, usize) {
        // Each part whose texts changed, its texts where they will be: after those the parts
        // before it lost.
        let (mut patches, mut grew) = (vec![], 0isize);
        for (&i, new) in parts.iter().zip(texts) {
            let f = &self.footprints[i];
            let old = &self.texts[f.texts.clone()];
            if new.iter().zip(old).all(|(n, o)| n.as_ref() == Some(o)) {
                continue;
            }
            let hid = new.iter().zip(old).filter(|(n, _)| n.is_none()).map(|(_, o)| o);
            let hidden = f.hidden.iter().chain(hid).cloned().collect();
            let kept: Vec<Text> = new.into_iter().flatten().collect();
            let start = f.texts.start.wrapping_add_signed(grew);
            grew += kept.len() as isize - old.len() as isize;
            patches.push(Patch::texts(self, i, Footprint { texts: start..start + kept.len(), hidden, ..f.clone() }, kept));
        }
        let changed = patches.len();
        (self.apply(patches.into_iter().collect()), changed)
    }

    /// Places the silkscreen reference of each of `parts` (whose texts are `texts`), the
    /// smallest parts first: they have the fewest spots.
    fn references(&self, parts: &[usize], texts: &mut [Vec<Option<Text>>], done: &mut Tidied) {
        let silk = |t: &Text| self.layers[t.layer as usize].ends_with(".SilkS");
        // Each reference: its part's place in `parts`, its own among the part's texts, the part's box.
        let mut refs = vec![];
        for (k, &i) in parts.iter().enumerate() {
            let f = &self.footprints[i];
            let shows = |t: &Option<Text>| t.as_ref().is_some_and(|t| silk(t) && t.text == f.reference && !t.text.is_empty());
            if let Some(j) = texts[k].iter().position(shows) {
                refs.push((k, j, self.drawn(f, |l| l.ends_with(".CrtYd")).unwrap_or(f.bounds)));
            }
        }
        let area = |(lo, hi): (Pt, Pt)| (hi.x as i64 - lo.x as i64) * (hi.y as i64 - lo.y as i64);
        refs.sort_by_key(|&(k, _, frame)| (area(frame), k));
        // Where the references are now is no obstacle: they move.
        let moving: HashSet<usize> = refs.iter().map(|&(k, j, _)| self.footprints[parts[k]].texts.start + j).collect();
        let mut rooms: HashMap<LayerId, Room> = HashMap::new();
        for (k, j, frame) in refs {
            let Some(t) = texts[k][j].take() else { continue };
            let room = rooms.entry(t.layer).or_insert_with(|| Room::new(self, t.layer, &moving));
            texts[k][j] = room.spot(t, frame, parts[k]);
            match texts[k][j] {
                Some(_) => done.placed += 1,
                None => done.hidden += 1,
            }
        }
    }

    /// Puts part `f`'s value on its Fab layer (`texts` are the part's) upright in the middle of
    /// its body, below the part's reference shown there, which goes above it, both shrunk to fit
    /// as KiCad pitches lines; a value with no room is hidden.
    fn value(&self, f: &Footprint, texts: &mut [Option<Text>], done: &mut Tidied) {
        let fab = |t: &Text| self.layers[t.layer as usize].ends_with(".Fab");
        let Some(v) = texts.iter().position(|t| t.as_ref().is_some_and(|t| fab(t) && t.text == f.value && !t.text.is_empty())) else {
            return;
        };
        let layer = texts[v].as_ref().map_or(0, |t| t.layer);
        let body = self.drawn(f, |l| *l == self.layers[layer as usize]).or_else(|| self.drawn(f, |l| l.ends_with(".CrtYd")));
        let body = body.unwrap_or(f.bounds);
        let shown =
            |j: usize| texts[j].as_ref().is_some_and(|t| t.layer == layer && t.text == f.reference && overlaps(font::bounds(t), body));
        let lines: Vec<usize> = (0..texts.len()).find(|&j| j != v && shown(j)).into_iter().chain([v]).collect();
        let (w, h) = (body.1.x - body.0.x, body.1.y - body.0.y);
        let (angle, long, short) = if h > w { (90.0, h, w) } else { (0.0, w, h) };
        // A tenth of the body's short side stays clear on each side; along it go the widest
        // line's advance and pen, across it the lines at KiCad's pitch.
        let room = |len: Nm| (len - short / 5) as f64;
        let line = |j: usize| texts[j].as_ref().expect("a line is a text shown");
        let wide = lines.iter().map(|&j| font::width(line(j)) / line(j).size.1.max(1) as f64 + FAB_PEN).fold(0.0, f64::max);
        let n = lines.len() as f64;
        let fit = (room(long) / wide).min(room(short) / ((n - 1.0) * PITCH + LINE + FAB_PEN));
        let own = lines.iter().map(|&j| line(j).size.1).max().unwrap_or(SIZE) as f64;
        // To 10 µm, as a person would size it.
        let size = (fit.min(own).max(0.0) as Nm) / 10_000 * 10_000;
        if size < FAB_MIN {
            texts[v] = None;
            done.values_hidden += 1;
            return;
        }
        let (centre, s) = (mid(body.0, body.1), size as f64);
        // The first line goes up: up the board when level, to its left when upright.
        let up = if angle == 0.0 { (0.0, -1.0) } else { (-1.0, 0.0) };
        for (m, &j) in lines.iter().enumerate() {
            let Some(t) = texts[j].as_mut() else { continue };
            let shift = ((n - 1.0) / 2.0 - m as f64) * PITCH * s;
            let width = (s * t.size.0 as f64 / t.size.1.max(1) as f64).round() as Nm;
            let laid = Text { at: Pt::default(), angle, size: (width, size), thickness: (s * FAB_PEN).round() as Nm, ..t.clone() };
            let (a, z) = font::bounds(&laid);
            let c = Pt { x: centre.x + (up.0 * shift).round() as Nm, y: centre.y + (up.1 * shift).round() as Nm };
            *t = Text { at: Pt { x: c.x - mid(a, z).x, y: c.y - mid(a, z).y }, ..laid };
        }
        done.values += 1;
    }

    /// The box round part `f`'s graphics on the layers `on` takes (by name), `None` without one.
    pub(crate) fn drawn(&self, f: &Footprint, on: impl Fn(&String) -> bool) -> Option<(Pt, Pt)> {
        self.shapes[f.shapes.clone()].iter().filter(|s| on(&self.layers[s.layer as usize])).map(shape_box).reduce(union)
    }
}

/// What a reference on one silkscreen layer keeps clear of, each with the gap it keeps: solder
/// mask openings (pads', untented vias', drawn ones), the layer's graphics and texts and the
/// board's edge; the references placed there; and the outside of the board. Where it can, it
/// keeps off the courtyards of the parts on that side (`yards`, by part): under them once made.
struct Room {
    tree: RTree,
    things: Vec<(Thing, Nm)>,
    placed: Vec<RBox>,
    yards: RTree,
    /// The board's outline, its loops; none when its Edge.Cuts lines don't all close.
    outline: Vec<Vec<Pt>>,
}

/// A stroke or box to keep clear of, or a filled area.
enum Thing {
    Shape(RBox),
    Area(Vec<Vec<Pt>>),
}

/// A spot a reference may take: its cost (how far from its part, how turned; its side), its
/// box's centre and size, its anchor from that centre, its angle.
struct Spot {
    cost: (i64, usize),
    at: Pt,
    size: (Nm, Nm),
    off: Pt,
    angle: f64,
}

impl Room {
    /// What a reference on silkscreen `layer` keeps clear of, but the texts `moving` (by their
    /// place in `Board::texts`): the references being placed.
    fn new(b: &Board, layer: LayerId, moving: &HashSet<usize>) -> Self {
        let front = !b.layers[layer as usize].starts_with("B.");
        let mask = b.layers.iter().position(|l| l == if front { "F.Mask" } else { "B.Mask" });
        let mut things = vec![];
        for p in b.pads.iter().filter(|p| mask.is_some_and(|m| p.layers & 1 << m != 0)) {
            let margin = p.mask_margin.unwrap_or(b.rules.mask_margin);
            things.push((Thing::Shape(RBox::pad(p)), (MASK + margin).max(0)));
        }
        if !(if front { b.rules.tented.0 } else { b.rules.tented.1 }) {
            things.extend(b.vias.iter().map(|v| (Thing::Shape(RBox::disc(v.at, v.size / 2)), MASK)));
        }
        for s in b.shapes.iter().filter(|s| s.layer == layer || Some(s.layer as usize) == mask) {
            let (pts, gap) = (path(&s.geom), if s.layer == layer { GAP } else { MASK });
            if s.filled && pts.len() > 2 {
                things.push((Thing::Area(vec![pts.clone()]), gap));
            }
            things.extend(pts.windows(2).map(|w| (Thing::Shape(RBox::segment(w[0], w[1], s.width / 2)), gap)));
        }
        let texts = b.texts.iter().enumerate().filter(|&(i, t)| t.layer == layer && !moving.contains(&i));
        things.extend(texts.map(|(_, t)| (Thing::Shape(boxed(pad_box(font::bounds(t), t.thickness / 2))), GAP)));
        things.extend(edges(b).into_iter().map(|e| (Thing::Shape(e), EDGE)));
        let bounds = |thing: &Thing| match thing {
            Thing::Shape(s) => s.bounds,
            Thing::Area(a) => a[0].iter().map(|&p| (p, p)).reduce(union).unwrap_or_default(),
        };
        let tree = RTree::new(things.iter().enumerate().map(|(i, (t, gap))| (pad_box(bounds(t), *gap), i as u32)).collect());
        // The board's Edge.Cuts lines joined into loops, as `Board::outline_box` takes them.
        let edge = b.layers.iter().position(|l| l == "Edge.Cuts");
        let paths: Vec<Vec<Pt>> = b.shapes.iter().filter(|s| Some(s.layer as usize) == edge).map(|s| path(&s.geom)).collect();
        let joined = loops(&paths);
        let closed = joined.iter().map(|(lines, _)| lines.len()).sum::<usize>() == paths.len();
        let outline = if closed { joined.into_iter().map(|(_, pts)| pts).collect() } else { vec![] };
        let yard = b.layers.iter().position(|l| l == if front { "F.CrtYd" } else { "B.CrtYd" });
        let yards = b.footprints.iter().enumerate().filter_map(|(i, f)| {
            let on = b.shapes[f.shapes.clone()].iter().filter(|s| Some(s.layer as usize) == yard);
            on.map(shape_box).reduce(union).map(|y| (y, i as u32))
        });
        Room { tree, things, placed: vec![], yards: RTree::new(yards.collect()), outline }
    }

    /// Reference `t` of part `part` at its first spot round `frame` (the part's box) clear of all
    /// the room keeps clear of: the nearest the part, level before upright, off other parts
    /// before over them; `None` when none is.
    fn spot(&mut self, t: Text, frame: (Pt, Pt), part: usize) -> Option<Text> {
        let t = Text { size: (t.size.0.max(SIZE), t.size.1.max(SIZE)), thickness: t.thickness.max(PEN), ..t };
        let centre = mid(frame.0, frame.1);
        let mut spots = vec![];
        for (ring, angle) in (0..RINGS).flat_map(|ring| [(ring, 0.0), (ring, 90.0)]) {
            let (lo, hi) = pad_box(frame, GAP + ring * STEP);
            let (a, z) = pad_box(font::bounds(&Text { at: Pt::default(), angle, ..t.clone() }), t.thickness / 2);
            let (w, h, off) = (z.x - a.x, z.y - a.y, mid(a, z));
            // Its box touching each side from outside, the way it slides along it, how far.
            let sides = [
                (Pt { x: centre.x, y: lo.y - h / 2 }, (1, 0), hi.x - lo.x + w),
                (Pt { x: centre.x, y: hi.y + h / 2 }, (1, 0), hi.x - lo.x + w),
                (Pt { x: lo.x - w / 2, y: centre.y }, (0, 1), hi.y - lo.y + h),
                (Pt { x: hi.x + w / 2, y: centre.y }, (0, 1), hi.y - lo.y + h),
            ];
            for (side, (c, (dx, dy), span)) in sides.into_iter().enumerate() {
                let n = span / 2 / STEP;
                for k in -n..=n {
                    let c = Pt { x: c.x + dx * k * STEP, y: c.y + dy * k * STEP };
                    let far = ((c.x - centre.x) as f64).hypot((c.y - centre.y) as f64) as i64;
                    spots.push(Spot { cost: (far + if angle == 0.0 { 0 } else { LEVEL }, side), at: c, size: (w, h), off, angle });
                }
            }
        }
        spots.sort_by_key(|s| s.cost);
        // A part off the board (beside it, to be placed) has its reference beside it too.
        let (lo, hi) = frame;
        let on = [lo, hi, Pt { x: lo.x, y: hi.y }, Pt { x: hi.x, y: lo.y }, centre].into_iter().any(|p| self.on_board(p));
        let fits = |s: &&Spot, over: bool| self.under(s.at, s.size, part) == over && self.free(s.at, s.size, on);
        let s = spots.iter().find(|s| fits(s, false)).or_else(|| spots.iter().find(|s| fits(s, true)))?;
        self.placed.push(RBox::boxed(s.at, s.size, 0.0, 0));
        Some(Text { at: Pt { x: s.at.x - s.off.x, y: s.at.y - s.off.y }, angle: s.angle, ..t })
    }

    /// Whether a box `size` round `c` lies over the courtyard of another part than `part`.
    fn under(&self, c: Pt, size: (Nm, Nm), part: usize) -> bool {
        let mut over = false;
        self.yards.query(RBox::boxed(c, size, 0.0, 0).bounds, |i| over |= i as usize != part);
        over
    }

    /// Whether `p` is on the board: inside an odd number of its outline's loops, anywhere when the
    /// outline doesn't close.
    fn on_board(&self, p: Pt) -> bool {
        self.outline.is_empty() || self.outline.iter().filter(|l| inside(p, l)).count() % 2 == 1
    }

    /// Whether a box `size` round `c` keeps clear of all the room keeps clear of, and (`on`) lies
    /// on the board.
    fn free(&self, c: Pt, size: (Nm, Nm), on: bool) -> bool {
        let (b, mut clear) = (RBox::boxed(c, size, 0.0, 0), true);
        self.tree.query(b.bounds, |i| {
            if clear {
                let (thing, gap) = &self.things[i as usize];
                clear = match thing {
                    Thing::Shape(s) => s.dist(&b) >= *gap,
                    Thing::Area(a) => b.dist_poly(a) >= *gap,
                };
            }
        });
        // No edge crosses the box: its centre tells whether it is on the board.
        let near = pad_box(b.bounds, GAP);
        clear && self.placed.iter().all(|p| !overlaps(p.bounds, near) || p.dist(&b) >= GAP) && (!on || self.on_board(c))
    }
}

/// The box `lo`–`hi` as a shape.
fn boxed((lo, hi): (Pt, Pt)) -> RBox {
    RBox::boxed(mid(lo, hi), (hi.x - lo.x, hi.y - lo.y), 0.0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::rotate;
    use crate::history::Pick;
    use crate::units::nm_to_mm;

    /// A KiCad 10 text of a part, as KiCad 10 writes it: its head, place, layer and font.
    fn text(head: &str, at: &str, layer: &str, (size, pen): (&str, &str)) -> String {
        let font = format!("(effects\n\t\t\t\t(font\n\t\t\t\t\t(size {size})\n\t\t\t\t\t(thickness {pen})\n\t\t\t\t)\n\t\t\t)");
        format!("\n\t\t({head}\n\t\t\t(at {at})\n\t\t\t(layer \"{layer}\")\n\t\t\t{font}\n\t\t)")
    }

    /// A KiCad 10 board `w` × `h` mm with an 0603 capacitor C1 at (10, 5) as KiCad's library has
    /// it, its reference on the silkscreen (here over its pads, at 0.8 mm), its value below it
    /// and its reference in its body on the Fab layer; and the same resistor R1 at `r` mm.
    fn board(w: f64, h: f64, r: (f64, f64)) -> String {
        let part = |lib: &str, (x, y): (f64, f64), reference: &str, value: &str| {
            let yard = "(fp_rect (start -1.48 -0.73) (end 1.48 0.73) (stroke (width 0.05) (type solid)) (fill no) (layer \"F.CrtYd\"))";
            let body = "(fp_rect (start -0.8 -0.4) (end 0.8 0.4) (stroke (width 0.1) (type solid)) (fill no) (layer \"F.Fab\"))";
            let pad = |n: &str, x: &str| {
                format!(
                    "(pad \"{n}\" smd roundrect (at {x} 0) (size 0.9 0.95) (layers \"F.Cu\" \"F.Mask\" \"F.Paste\") (roundrect_rratio 0.25))"
                )
            };
            format!(
                "\n\t(footprint \"{lib}\"\n\t\t(layer \"F.Cu\")\n\t\t(at {x} {y}){}{}{}\n\t\t{yard}\n\t\t{body}\n\t\t{}\n\t\t{}\n\t)",
                text(&format!("property \"Reference\" \"{reference}\""), "0 0 0", "F.SilkS", ("0.8 0.8", "0.1")),
                text(&format!("property \"Value\" \"{value}\""), "0 1.43 0", "F.Fab", ("1 1", "0.15")),
                text("fp_text user \"${REFERENCE}\"", "0 0 0", "F.Fab", ("0.4 0.4", "0.06")),
                pad("1", "-0.775"),
                pad("2", "0.775"),
            )
        };
        let layers = ["(0 \"F.Cu\" signal)", "(2 \"B.Cu\" signal)", "(1 \"F.Mask\" user)", "(5 \"F.SilkS\" user \"F.Silkscreen\")"];
        let more = ["(25 \"Edge.Cuts\" user)", "(31 \"F.CrtYd\" user \"F.Courtyard\")", "(35 \"F.Fab\" user)"];
        let edge = format!("(gr_rect (start 0 0) (end {w} {h}) (stroke (width 0.05) (type default)) (fill no) (layer \"Edge.Cuts\"))");
        format!(
            "(kicad_pcb\n\t(version 20260206)\n\t(layers\n\t\t{}\n\t)\n\t{edge}{}{}\n)\n",
            layers.iter().chain(&more).copied().collect::<Vec<_>>().join("\n\t\t"),
            part("Capacitor_SMD:C_0603_1608Metric", (10.0, 5.0), "C1", "4.7u"),
            part("Resistor_SMD:R_0603_1608Metric", r, "R1", "10k"),
        )
    }

    /// The box round text `t` with its pen.
    fn text_box(t: &Text) -> (Pt, Pt) {
        pad_box(font::bounds(t), t.thickness / 2)
    }

    #[test]
    fn places_a_reference_upright_at_kicads_size_just_outside_its_part() {
        let src = board(20.0, 10.0, (16.0, 5.0));
        let mut b = Board::parse(&src).unwrap();
        let (undo, done) = b.tidy_texts(&[]);
        assert_eq!(done, Tidied { placed: 2, hidden: 0, values: 2, values_hidden: 0, parts: 2 });
        let c1 = &b.texts[b.footprints[0].texts.start];
        assert_eq!((c1.text.as_str(), c1.angle, c1.size, c1.thickness), ("C1", 0.0, (MM, MM), PEN));
        // Level, above the courtyard (its top at 5 - 0.73 mm, half its pen out), a gap from it.
        let (lo, hi) = text_box(c1);
        assert!(((lo.x + hi.x) / 2 - 10 * MM).abs() <= 1 && (hi.y - (5 * MM - 730_000 - 25_000 - GAP)).abs() <= 1, "{lo:?} {hi:?}");
        // Clear of every mask opening, by JLCPCB's silkscreen to pad clearance.
        assert!(b.pads.iter().all(|p| RBox::pad(p).dist(&boxed((lo, hi))) >= MASK));
        // Again, nothing moves; undone, the board saves as read.
        assert_eq!(b.tidy_texts(&[]).1.parts, 0);
        b.apply(undo);
        assert_eq!(b.write(&src).unwrap(), src);
    }

    #[test]
    fn tidies_only_the_parts_picked_keeping_off_their_neighbours() {
        // R1 right above C1: C1's reference goes below C1, R1's stays.
        let src = board(20.0, 10.0, (10.0, 2.5));
        let mut b = Board::parse(&src).unwrap();
        let r1 = b.texts[b.footprints[1].texts.clone()].to_vec();
        let (_, done) = b.tidy_texts(&[0]);
        assert_eq!((done.placed, done.parts, &b.texts[b.footprints[1].texts.clone()]), (1, 1, &r1[..]));
        let (lo, _) = text_box(&b.texts[b.footprints[0].texts.start]);
        assert!((lo.y - (5 * MM + 755_000 + GAP)).abs() <= 1, "below C1: {lo:?}");
    }

    #[test]
    fn a_part_off_the_board_has_its_reference_beside_it() {
        // R1 waits beside the board to be placed (as F8 adds parts): its reference reads by it.
        let src = board(20.0, 10.0, (30.0, 5.0));
        let mut b = Board::parse(&src).unwrap();
        let (_, done) = b.tidy_texts(&[1]);
        let (lo, hi) = text_box(&b.texts[b.footprints[1].texts.start]);
        assert_eq!(done.placed, 1);
        assert!(lo.x > 20 * MM && (hi.y - (5 * MM - 755_000 - GAP)).abs() <= 1, "{lo:?} {hi:?}");
    }

    #[test]
    fn hides_a_reference_with_no_room_and_saves_it_hidden() {
        // The board's edge hugs C1: no room for its reference.
        let src = board(11.7, 6.1, (30.0, 5.0)).replacen("(start 0 0)", "(start 8.3 3.9)", 1);
        let mut b = Board::parse(&src).unwrap();
        let (undo, done) = b.tidy_texts(&[0]);
        assert_eq!((done.placed, done.hidden, b.footprints[0].hidden[0].src), (0, 1, Some(0)));
        let out = b.write(&src).unwrap();
        let hidden = "(property \"Reference\" \"C1\"\n\t\t\t(at 0 0 0)\n\t\t\t(layer \"F.SilkS\")\n\t\t\t(hide yes)\n\t\t\t(effects";
        assert!(out.contains(hidden), "{out}");
        // Read back, it is hidden, the other texts as tidied.
        let back = Board::parse(&out).unwrap();
        assert_eq!(back.texts, b.texts);
        b.apply(undo);
        assert_eq!(b.write(&src).unwrap(), src);
    }

    #[test]
    fn puts_a_value_in_its_body_below_the_reference_there_shrunk_to_fit() {
        let src = board(20.0, 10.0, (16.0, 5.0));
        let mut b = Board::parse(&src).unwrap();
        b.tidy_texts(&[0]);
        let (value, fab) = (&b.texts[1], &b.texts[2]);
        assert_eq!((value.text.as_str(), fab.text.as_str()), ("4.7u", "C1"));
        // The body (1.6 × 0.8 mm, a 0.1 mm pen) holds both lines at KiCad's pitch at 0.24 mm.
        let body = (Pt { x: 10 * MM - 850_000, y: 5 * MM - 450_000 }, Pt { x: 10 * MM + 850_000, y: 5 * MM + 450_000 });
        for t in [value, fab] {
            let (lo, hi) = text_box(t);
            assert!(lo.x >= body.0.x && lo.y >= body.0.y && hi.x <= body.1.x && hi.y <= body.1.y, "{:?} in the body", t.text);
            assert_eq!((t.angle, t.size, t.thickness), (0.0, (240_000, 240_000), 36_000));
        }
        assert!(text_box(fab).1.y <= text_box(value).0.y, "the reference above the value");
        // A body too small for its value: the value is hidden, the reference there stays.
        let tiny = src.replacen("(start -0.8 -0.4) (end 0.8 0.4)", "(start -0.2 -0.1) (end 0.2 0.1)", 1);
        let mut b = Board::parse(&tiny).unwrap();
        let fab = b.texts[2].clone();
        let (_, done) = b.tidy_texts(&[0]);
        assert_eq!((done.values, done.values_hidden, &b.texts[1]), (0, 1, &fab));
    }

    #[test]
    fn saves_the_texts_tidied_as_kicad_writes_them() {
        let src = board(20.0, 10.0, (16.0, 5.0));
        let mut b = Board::parse(&src).unwrap();
        b.tidy_texts(&[0]);
        // Only C1's texts' places, sizes and pens change, in place: its reference, value and
        // reference on the Fab layer, the first three texts.
        let local = |t: &Text| format!("{} {} 0", nm_to_mm(t.at.x - 10 * MM), nm_to_mm(t.at.y - 5 * MM));
        let (c1, value, fab) = (local(&b.texts[0]), local(&b.texts[1]), local(&b.texts[2]));
        let was = |at: &str, layer: &str, font: (&str, &str)| text("", at, layer, font).split_off(4);
        let want = src
            .replacen(&was("0 0 0", "F.SilkS", ("0.8 0.8", "0.1")), &was(&c1, "F.SilkS", ("1 1", "0.15")), 1)
            .replacen(&was("0 1.43 0", "F.Fab", ("1 1", "0.15")), &was(&value, "F.Fab", ("0.24 0.24", "0.036")), 1)
            .replacen(&was("0 0 0", "F.Fab", ("0.4 0.4", "0.06")), &was(&fab, "F.Fab", ("0.24 0.24", "0.036")), 1);
        assert_eq!(b.write(&src).unwrap(), want);
    }

    #[test]
    fn saves_a_kicad_6_reference_hidden_and_a_level_one_without_its_angle() {
        // KiCad 6 writes texts on one line, a 0° angle left out and `hide` as a flag. R2 is on a
        // board of its own, too small for its reference.
        let src =
            "(kicad_pcb (version 20211014) (layers (0 \"F.Cu\" signal) (38 \"F.Mask\" user) (37 \"F.SilkS\" user) (44 \"Edge.Cuts\" user))
  (gr_rect (start 0 0) (end 20 10) (layer \"Edge.Cuts\") (width 0.05) (fill none))
  (gr_rect (start 29.5 4.5) (end 30.5 5.5) (layer \"Edge.Cuts\") (width 0.05) (fill none))
  (footprint \"R\" (layer \"F.Cu\") (at 5 5 90)
    (fp_text reference \"R1\" (at 0 0 90) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))))
    (pad \"1\" smd rect (at 0 -0.5 90) (size 0.6 0.6) (layers \"F.Cu\" \"F.Mask\")))
  (footprint \"R\" (layer \"F.Cu\") (at 30 5 90)
    (fp_text reference \"R2\" (at 0 0 90) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))))
    (pad \"1\" smd rect (at 0 0 90) (size 0.6 0.6) (layers \"F.Cu\" \"F.Mask\"))))";
        let mut b = Board::parse(src).unwrap();
        let (_, done) = b.tidy_texts(&[]);
        assert_eq!((done.placed, done.hidden), (1, 1));
        let out = b.write(src).unwrap();
        let r1 = &b.texts[0];
        let local = rotate(Pt { x: r1.at.x - 5 * MM, y: r1.at.y - 5 * MM }, -90.0);
        assert_eq!(r1.angle, 0.0);
        let level = format!("(fp_text reference \"R1\" (at {} {}) (layer \"F.SilkS\") (effects", nm_to_mm(local.x), nm_to_mm(local.y));
        assert!(out.contains(&level) && out.contains("(fp_text reference \"R2\" (at 0 0 90) (layer \"F.SilkS\") hide (effects"), "{out}");
    }

    #[test]
    fn a_tidied_part_turned_flipped_and_moved_saves_as_the_board_has_it() {
        let src = board(20.0, 10.0, (16.0, 5.0));
        let mut b = Board::parse(&src).unwrap();
        let (picks, about) = ([Pick::Footprint(0), Pick::Footprint(1)], Pt { x: 12 * MM, y: 4 * MM });
        let mut undo = vec![b.tidy_texts(&[]).0, b.turn_items(&picks, about, 90.0)];
        undo.extend([b.flip_items(&picks, about), b.move_items(&picks, Pt { x: MM / 3, y: -MM / 7 })]);
        let back = Board::parse(&b.write(&src).unwrap()).unwrap();
        assert_eq!(back.texts.len(), b.texts.len());
        for (t, u) in b.texts.iter().zip(&back.texts) {
            let off = (t.angle - u.angle).rem_euclid(360.0);
            assert!((t.at.x - u.at.x).abs() <= 2 && (t.at.y - u.at.y).abs() <= 2 && off.min(360.0 - off) < 1e-6, "{t:?}, read {u:?}");
            // Layers met first on a flipped part (B.SilkS here) come in another order.
            let (l, m) = (&b.layers[t.layer as usize], &back.layers[u.layer as usize]);
            assert_eq!((t.size, t.thickness, l, t.mirror), (u.size, u.thickness, m, u.mirror));
        }
        undo.into_iter().rev().for_each(|p| _ = b.apply(p));
        assert_eq!(b.write(&src).unwrap(), src);
    }
}
