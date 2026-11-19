//! Differential pairs: two nets that carry one signal, found by their names as KiCad finds them,
//! with the width and gap their class routes them with (`crates/core/pair.md`).

use crate::board::{Board, LayerId, MM, NetId, Nm, Pt, Track};
use crate::drc::{self, Item};
use crate::history::Patch;
use crate::route::Router;
use crate::tr::{tr, trf};

/// A differential pair: its positive and its negative net, and the width of its tracks and the
/// gap between them as its class asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    pub nets: [NetId; 2],
    pub width: Nm,
    pub gap: Nm,
}

/// The name of the net that pairs with net `name`, as KiCad pairs them: its last character
/// before any digits and underscores swapped, `+` and `-`, `P` and `N` (`USB_D+` and `USB_D-`,
/// `CLK_P` and `CLK_N`, `TX2P_1` and `TX2N_1`); and whether `name` is the positive side.
pub fn other_side(name: &str) -> Option<(String, bool)> {
    let tail = name.bytes().rev().take_while(|c| c.is_ascii_digit() || *c == b'_').count();
    let at = name.len().checked_sub(tail + 1)?;
    let (to, positive) = match name.as_bytes()[at] {
        b'+' => ('-', true),
        b'-' => ('+', false),
        b'P' => ('N', true),
        b'N' => ('P', false),
        _ => return None,
    };
    Some((format!("{}{to}{}", &name[..at], &name[at + 1..]), positive))
}

impl Board {
    /// The differential pair `net` is a side of: the board has the net its name pairs it with.
    /// Its width and gap are its positive net's class's; the gap no less than the clearance its
    /// nets keep from each other, which DRC holds them to.
    pub fn pair(&self, net: NetId) -> Option<Pair> {
        let (other, positive) = other_side(self.nets.get(net as usize).filter(|_| net != 0)?)?;
        let partner = self.nets.iter().position(|n| *n == other)? as NetId;
        let nets = if positive { [net, partner] } else { [partner, net] };
        let (r, class) = (&self.rules, |n: NetId| self.class(n));
        let (width, gap) = class(nets[0]).map_or((r.class_pair_width, r.class_pair_gap), |c| (c.pair_width, c.pair_gap));
        let keeps = nets.map(|n| class(n).map_or(r.clearance, |c| c.clearance));
        Some(Pair { nets, width, gap: gap.max(keeps[0]).max(keeps[1]) })
    }
}

/// How far from the copper clicked the other side of its pair is looked for.
const NEAR: Nm = 5 * MM;

/// Where a pair's route starts or ends: copper of its two nets side by side on a layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ends {
    pub pair: Pair,
    pub layer: LayerId,
    /// A pad's or via's centre, or a track's end, of the positive net, then of the negative.
    pub at: [Pt; 2],
    /// How far their copper reaches from those points.
    pub reach: Nm,
}

/// What a pair's route would lay down: its middle line; the corners of its positive and of its
/// negative track, from the ends it starts at; the ends it arrives at, if it does; and why it
/// can't be laid so, if it can't.
#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    pub middle: Vec<Pt>,
    pub sides: [Vec<Pt>; 2],
    pub to: Option<Ends>,
    pub refused: Option<String>,
}

impl Laid {
    /// How long its positive and its negative track are, in nm.
    pub fn lengths(&self) -> [f64; 2] {
        [0, 1].map(|k| self.sides[k].windows(2).map(|w| dist(w[0], w[1])).sum())
    }
}

fn dist(a: Pt, b: Pt) -> f64 {
    ((b.x - a.x) as f64).hypot((b.y - a.y) as f64)
}

fn middle(at: [Pt; 2]) -> Pt {
    Pt { x: ((at[0].x as i64 + at[1].x as i64) / 2) as Nm, y: ((at[0].y as i64 + at[1].y as i64) / 2) as Nm }
}

/// The step from between `ends`' two points square out of the line through them, on the side
/// `toward` lies, past their copper: where the pair's tracks run side by side from.
fn out_of(ends: &Ends, toward: Pt) -> Pt {
    let (m, [p, n]) = (middle(ends.at), ends.at);
    let (dx, dy, len) = ((n.x - p.x) as f64, (n.y - p.y) as f64, dist(p, n).max(1.0));
    let (ox, oy) = (-dy / len, dx / len);
    let side = if (toward.x - m.x) as f64 * ox + (toward.y - m.y) as f64 * oy < 0.0 { -1.0 } else { 1.0 };
    let t = (ends.reach + ends.pair.width) as f64 * side;
    Pt { x: m.x + (ox * t).round() as Nm, y: m.y + (oy * t).round() as Nm }
}

/// `line` without the points that repeat the one before, or lie on the way from it to the next.
fn distinct(line: &[Pt]) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(line.len());
    for &p in line {
        if out.last() == Some(&p) {
            continue;
        }
        // The point before it goes when it bends the line by nothing.
        if let [.., a, b] = out[..] {
            let (ux, uy, vx, vy) = ((b.x - a.x) as f64, (b.y - a.y) as f64, (p.x - b.x) as f64, (p.y - b.y) as f64);
            if (ux * vy - uy * vx).abs() <= 1e-6 * ux.hypot(uy) * vx.hypot(vy) && ux * vx + uy * vy > 0.0 {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

/// The way each stretch of `line` goes, a unit long.
fn ways(line: &[Pt]) -> Vec<(f64, f64)> {
    line.windows(2).map(|w| ((w[1].x - w[0].x) as f64 / dist(w[0], w[1]), (w[1].y - w[0].y) as f64 / dist(w[0], w[1]))).collect()
}

/// Takes out of `line` each stretch too short for the corners two tracks `d` to its sides turn
/// at its ends (two bends the same way, close together), its neighbours going on to where their
/// lines meet: one corner for the two.
fn relax(line: &mut Vec<Pt>, d: f64) {
    for _ in 0..line.len() {
        let u = ways(line);
        let cross = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;
        let dot = |a: (f64, f64), b: (f64, f64)| a.0 * b.0 + a.1 * b.1;
        // What a corner between ways `a` and `b` takes of each stretch on its inner side.
        let takes = |a: (f64, f64), b: (f64, f64)| d * ((1.0 - dot(a, b)).max(0.0) / (1.0 + dot(a, b)).max(1e-9)).sqrt();
        let short = (1..u.len().saturating_sub(1)).find(|&i| {
            let (before, after) = (cross(u[i - 1], u[i]), cross(u[i], u[i + 1]));
            before * after > 0.0 && dist(line[i], line[i + 1]) < takes(u[i - 1], u[i]) + takes(u[i], u[i + 1]) + 1000.0
        });
        let Some(i) = short else { return };
        // Where the stretch before, carried on, meets the one after, carried back.
        let (a, b, den) = (line[i], line[i + 1], cross(u[i - 1], u[i + 1]));
        if den.abs() < 1e-6 {
            return;
        }
        let t = ((b.x - a.x) as f64 * u[i + 1].1 - (b.y - a.y) as f64 * u[i + 1].0) / den;
        let meet = Pt { x: a.x + (u[i - 1].0 * t).round() as Nm, y: a.y + (u[i - 1].1 * t).round() as Nm };
        line.splice(i..i + 2, [meet]);
        *line = distinct(line);
    }
}

/// The lines `d` to each side of `line`, their corners where the sides of two stretches meet;
/// `None` where it turns back more than a right angle, or a stretch is too short for its corners.
fn beside(line: &[Pt], d: f64) -> Option<[Vec<Pt>; 2]> {
    let u = ways(line);
    let mut out = [Vec::with_capacity(line.len()), Vec::with_capacity(line.len())];
    for (i, p) in line.iter().enumerate() {
        // The step to the side at this point: square to its one stretch, or to where two meet.
        let (a, b) = (u[i.saturating_sub(1)], u[i.min(u.len() - 1)]);
        let (turn, (sx, sy)) = (a.0 * b.0 + a.1 * b.1, (a.1 + b.1, -a.0 - b.0));
        if turn < -0.05 {
            return None;
        }
        for (side, sign) in [1.0, -1.0].into_iter().enumerate() {
            let k = sign * d / (1.0 + turn);
            out[side].push(Pt { x: p.x + (sx * k).round() as Nm, y: p.y + (sy * k).round() as Nm });
        }
    }
    // Each side goes the way its stretch of the line does.
    let along =
        |side: &[Pt]| side.windows(2).zip(&u).all(|(w, u)| (w[1].x - w[0].x) as f64 * u.0 + (w[1].y - w[0].y) as f64 * u.1 > 1000.0);
    (along(&out[0]) && along(&out[1])).then_some(out)
}

impl Board {
    /// A differential pair's ends at `at` (within `tol`, on the copper `layers`): the pad, via
    /// or track end there, as a route starts on one, and the nearest of its pair's other net on
    /// that layer, within 5 mm; or why there are none.
    pub fn pair_ends(&self, at: Pt, tol: Nm, layers: &[LayerId]) -> Result<Ends, String> {
        let s = self.route_start(at, tol, layers).ok_or_else(|| tr("Click a pad, via or track end of a differential pair").to_string())?;
        let name = &self.nets[s.net as usize];
        let pair = self.pair(s.net).ok_or_else(|| match other_side(name) {
            Some((other, _)) => trf("{0} is no side of a differential pair: the board has no net {1}", &[name, &other]),
            None => trf("{0} is no side of a differential pair: their nets are named as D+ and D-, or CLK_P and CLK_N", &[name]),
        })?;
        let other = pair.nets[usize::from(pair.nets[0] == s.net)];
        // A net's copper on the layer a pair may end on: where, and how far it reaches from there.
        let copper = |net: NetId| {
            let on = 1 << s.layer;
            let pads = self.pads.iter().filter(move |p| p.net == net && p.layers & on != 0).map(|p| (p.at, p.size.0.max(p.size.1) / 2));
            let vias = self.vias.iter().filter(move |v| v.net == net && v.layers & on != 0).map(|v| (v.at, v.size / 2));
            let ends = self.tracks.iter().filter(move |t| t.net == net && t.layer == s.layer);
            pads.chain(vias).chain(ends.flat_map(|t| [(t.a, t.width / 2), (t.b, t.width / 2)]))
        };
        let near = copper(other).filter(|&(q, _)| dist(q, s.at) <= NEAR as f64).min_by(|a, b| dist(a.0, s.at).total_cmp(&dist(b.0, s.at)));
        let (to, reach) = near.ok_or_else(|| {
            trf(
                "No pad, via or track end of {0} within 5 mm of this one on {1}",
                &[&self.nets[other as usize], &self.layers[s.layer as usize]],
            )
        })?;
        let own = copper(s.net).filter(|&(q, _)| q == s.at).map(|(_, r)| r).max().unwrap_or(s.width / 2);
        let at = if pair.nets[0] == s.net { [s.at, to] } else { [to, s.at] };
        Ok(Ends { pair, layer: s.layer, at, reach: reach.max(own) })
    }

    /// Adds the tracks of a pair's route from `from` as `laid`, after the board's last; returns
    /// the patch that undoes it. Refused, nothing changed, when it can't be laid so or its
    /// tracks would break a design rule: with why.
    pub fn add_pair(&mut self, from: &Ends, laid: &Laid) -> Result<Patch, String> {
        if let Some(why) = &laid.refused {
            return Err(why.clone());
        }
        let (pair, layer) = (from.pair, from.layer);
        let track = |net: NetId, w: &[Pt]| Track { a: w[0], b: w[1], mid: None, width: pair.width, layer, net, locked: false };
        let (undo, first) = (Patch::added(self), self.tracks.len());
        for (side, net) in laid.sides.iter().zip(pair.nets) {
            self.tracks.extend(side.windows(2).filter(|w| w[0] != w[1]).map(|w| track(net, w)));
        }
        let items: Vec<Item> = (first..self.tracks.len()).map(Item::Track).collect();
        match drc::check_items(self, &items).into_iter().find(|v| !v.warning) {
            _ if items.is_empty() => Err(tr("Nothing to lay yet: lead the pair away from its pads").into()),
            Some(v) => {
                self.apply(undo);
                Err(format!("{}. {}", v.what, v.fix))
            }
            None => Ok(undo),
        }
    }
}

/// A differential pair's route being drawn on a layer: its middle line searched as one track as
/// wide as both of its tracks and their gap, the tracks then laid to each side of it.
pub struct PairRouter {
    router: Router,
    pub from: Ends,
    /// The middle line fixed so far: from between the ends it starts at, square out of them, then
    /// through each point clicked; and as it was before each click.
    line: Vec<Pt>,
    fixed: Vec<Vec<Pt>>,
}

impl PairRouter {
    /// A route of the pair that starts at `from`, leaving on the side `toward` lies.
    pub fn new(b: &Board, from: Ends, toward: Pt) -> Self {
        let (pair, first) = (from.pair, out_of(&from, toward));
        let across = 2 * pair.width + pair.gap;
        // Corners no closer than the tracks beside the line need to turn them.
        let router = Router::pair(b, pair.nets, from.layer, across, first, (pair.width + pair.gap) * 3 / 10);
        PairRouter { router, from, line: vec![middle(from.at), first], fixed: vec![] }
    }

    /// The nets it routes, the positive first, with their tracks' width and gap.
    pub fn pair(&self) -> Pair {
        self.from.pair
    }

    /// The route as far as `p`: round other nets' copper to the grid point nearest it, or, with
    /// ends of the pair under it (within `tol`), into those; as far as it gets when the way is shut.
    pub fn to(&mut self, b: &Board, p: Pt, tol: Nm) -> Laid {
        // Until a click fixes a corner it leaves its ends on the side the pointer is.
        if self.fixed.is_empty() && out_of(&self.from, p) != self.line[1] {
            *self = PairRouter::new(b, self.from, p);
        }
        let (from, last) = (self.from, *self.line.last().expect("a line starts with two points"));
        let there = b.pair_ends(p, tol, &[from.layer]).ok().filter(|e| e.pair.nets == from.pair.nets && e.at != from.at);
        // Into ends: to the grid point a step short of where the line runs square into them, then
        // sideways at 45° onto that line (the grid needn't hold it), and in between them along it.
        let landing = there.map(|e| {
            let (m, out) = (middle(e.at), out_of(&e, last));
            let (t, step) = (dist(m, out).max(1.0), self.router.step() as f64);
            let (ex, ey) = ((out.x - m.x) as f64 / t, (out.y - m.y) as f64 / t);
            let at = |s: f64| Pt { x: m.x + (ex * s).round() as Nm, y: m.y + (ey * s).round() as Nm };
            let g = self.router.on_cell(at(t + step));
            let (along, across) = ((g.x - m.x) as f64 * ex + (g.y - m.y) as f64 * ey, (g.x - m.x) as f64 * ey - (g.y - m.y) as f64 * ex);
            (g, [at(along - across.abs()), m])
        });
        let aim = landing.map_or_else(|| self.router.on_cell(p), |l| l.0);
        let path = self.router.path(last, aim);
        let to = there.filter(|_| path.last() == Some(&aim));
        let land = landing.filter(|_| to.is_some()).map(|l| l.1);
        let mut line: Vec<Pt> = self.line.iter().chain(&path[1..]).copied().chain(land.into_iter().flatten()).collect();
        line = distinct(&line);
        let d = (from.pair.width + from.pair.gap) as f64 / 2.0;
        relax(&mut line, d);
        let Some([a, b_side]) = (line.len() > 1).then(|| beside(&line, d)).flatten() else {
            let refused = Some(tr("The pair can't turn back that tightly: lead it round in a wider curve").to_string());
            return Laid { middle: line, sides: [vec![], vec![]], to, refused };
        };
        // The side that starts nearer the positive end is the positive track's.
        let [mut pos, mut neg] = if dist(a[0], from.at[0]) <= dist(b_side[0], from.at[0]) { [a, b_side] } else { [b_side, a] };
        let mut refused = None;
        if let Some(e) = to {
            let (p_end, n_end) = (pos[pos.len() - 1], neg[neg.len() - 1]);
            if dist(p_end, e.at[0]) + dist(n_end, e.at[1]) > dist(p_end, e.at[1]) + dist(n_end, e.at[0]) {
                let nets = from.pair.nets.map(|n| b.nets[n as usize].as_str());
                refused =
                    Some(trf("The pair arrives crossed: {0} would end on {1}. Bring it in from the other side", &[&nets[0], &nets[1]]));
            }
            pos.push(e.at[0]);
            neg.push(e.at[1]);
        }
        pos.insert(0, from.at[0]);
        neg.insert(0, from.at[1]);
        Laid { middle: line, sides: [distinct(&pos), distinct(&neg)], to, refused }
    }

    /// Fixes the middle line as `laid` has it (a click short of the pair's other ends): the
    /// route goes on from its end. Whether it did: not one refused, or that got nowhere.
    pub fn fix(&mut self, laid: &Laid) -> bool {
        let on = laid.refused.is_none() && laid.to.is_none() && laid.middle.last() != self.line.last();
        if on {
            self.fixed.push(std::mem::replace(&mut self.line, laid.middle.clone()));
        }
        on
    }

    /// Takes back the last click; whether there was one.
    pub fn back(&mut self) -> bool {
        self.fixed.pop().map(|line| self.line = line).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_nets_by_their_names_as_kicad_does() {
        let other = other_side;
        assert_eq!(other("USB_D+"), Some(("USB_D-".into(), true)));
        assert_eq!(other("/usb/CLK_N"), Some(("/usb/CLK_P".into(), false)));
        assert_eq!(other("TX2P_1"), Some(("TX2N_1".into(), true)));
        assert_eq!(other("LVDS-3"), Some(("LVDS+3".into(), false)));
        assert_eq!((other("GND"), other("RESET_1"), other(""), other("42")), (None, None, None, None));
        // A net is one side of a pair when the board has the other; the gap never under the clearance.
        let mut b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
              (net 0 "") (net 1 "D-") (net 2 "D+") (net 3 "EN") (net 4 "CLK_P"))"#,
        )
        .unwrap();
        assert_eq!(b.pair(1), Some(Pair { nets: [2, 1], width: 200_000, gap: 250_000 }));
        assert_eq!((b.pair(2), b.pair(3), b.pair(4), b.pair(0)), (b.pair(1), None, None, None));
        b.apply_pro(
            r#"{"net_settings": {"classes": [{"name": "Default", "clearance": 0.2},
                {"name": "USB", "clearance": 0.3, "diff_pair_width": 0.18, "diff_pair_gap": 0.15}],
                "netclass_patterns": [{"netclass": "USB", "pattern": "D?"}]}}"#,
        );
        assert_eq!(b.pair(2), Some(Pair { nets: [2, 1], width: 180_000, gap: 300_000 }));
    }

    /// A connector's D+ and D- pads 1 mm apart on the left, a chip's 0.5 mm apart 20 mm to the
    /// right and 6 mm down, a pad of GND between them in the way; `swap`: the chip's pads the
    /// other way round.
    fn board(swap: bool) -> Board {
        let (p, n) = if swap { ("2 \"D-\"", "1 \"D+\"") } else { ("1 \"D+\"", "2 \"D-\"") };
        Board::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (net 0 "") (net 1 "D+") (net 2 "D-") (net 3 "GND")
              (footprint "J" (layer "F.Cu") (at 10 10)
                (pad "1" smd rect (at 0 -0.5) (size 1.2 0.6) (layers "F.Cu") (net 1 "D+"))
                (pad "2" smd rect (at 0 0.5) (size 1.2 0.6) (layers "F.Cu") (net 2 "D-")))
              (footprint "U" (layer "F.Cu") (at 30 16)
                (pad "1" smd rect (at 0 -0.25) (size 0.8 0.3) (layers "F.Cu") (net {p}))
                (pad "2" smd rect (at 0 0.25) (size 0.8 0.3) (layers "F.Cu") (net {n})))
              (footprint "TP" (layer "F.Cu") (at 20 10) (pad "1" smd rect (at 0 0) (size 2 2) (layers "F.Cu") (net 3 "GND")))
              (gr_rect (start 0 0) (end 40 30) (width 0.1) (layer "Edge.Cuts")))"#
        ))
        .unwrap()
    }

    fn mm(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    #[test]
    fn finds_a_pairs_ends_side_by_side() {
        let b = board(false);
        // Clicked on either pad: both, the positive first, as far as the larger reaches.
        let ends = b.pair_ends(mm(10.2, 10.5), 50_000, &[0]).unwrap();
        assert_eq!((ends.at, ends.reach, ends.layer, ends.pair.nets), ([mm(10.0, 9.5), mm(10.0, 10.5)], 600_000, 0, [1, 2]));
        assert_eq!(b.pair_ends(mm(10.0, 9.5), 50_000, &[0]), Ok(ends));
        let why = |at: Pt, layers: &[LayerId]| b.pair_ends(at, 50_000, layers).err().unwrap();
        assert_eq!(
            why(mm(20.0, 10.0), &[0]),
            "GND is no side of a differential pair: their nets are named as D+ and D-, or CLK_P and CLK_N"
        );
        assert_eq!(why(mm(5.0, 5.0), &[0]), "Click a pad, via or track end of a differential pair");
        assert_eq!(why(mm(10.0, 9.5), &[1]), "Click a pad, via or track end of a differential pair");
    }

    #[test]
    fn routes_a_pair_side_by_side_round_other_copper_into_its_other_ends() {
        let mut b = board(false);
        let from = b.pair_ends(mm(10.0, 9.5), 50_000, &[0]).unwrap();
        let mut r = PairRouter::new(&b, from, mm(30.0, 16.0));
        // In the open it follows the pointer; a click fixes it there, Backspace takes that back.
        let open = r.to(&b, mm(15.0, 14.0), 50_000);
        assert!(open.to.is_none() && open.refused.is_none() && open.middle[0] == mm(10.0, 10.0), "{open:?}");
        assert!((r.fix(&open), r.back(), r.back(), r.fix(&open)) == (true, true, false, true));
        // Onto the chip's pads it arrives between them, each track on its own pad.
        let laid = r.to(&b, mm(30.0, 16.2), 50_000);
        assert_eq!((laid.to.map(|e| e.at), &laid.refused), (Some([mm(30.0, 15.75), mm(30.0, 16.25)]), &None));
        let [pos, neg] = &laid.sides;
        assert_eq!(
            (pos[0], pos[pos.len() - 1], neg[0], neg[neg.len() - 1]),
            (mm(10.0, 9.5), mm(30.0, 15.75), mm(10.0, 10.5), mm(30.0, 16.25))
        );
        // The two run as long as each other within a corner's worth, and nothing fixes after arriving.
        let [lp, ln] = laid.lengths();
        assert!((lp - ln).abs() < 1.5e6 && lp > 20e6 && !r.fix(&laid), "{lp} {ln}");
        let undo = b.add_pair(&from, &laid).unwrap();
        // Laid, they join each net's pads, keep every rule, and the gap between them is the pair's
        // wherever they run side by side.
        assert!(b.ratsnest().is_empty() && drc::check(&b).iter().all(|v| v.warning), "{:?}", drc::check(&b));
        let (pair, tracks) = (from.pair, &b.tracks);
        assert!(tracks.iter().all(|t| t.width == pair.width && t.layer == 0));
        let apart = |t: &Track, o: &Track| {
            let mid = Pt { x: (t.a.x + t.b.x) / 2, y: (t.a.y + t.b.y) / 2 };
            dist(mid, crate::board::nearest(o.a, o.b, mid))
        };
        let (side, other): (Vec<&Track>, Vec<&Track>) = tracks.iter().partition(|t| t.net == 1);
        let gaps: Vec<f64> = side.iter().map(|t| other.iter().map(|o| apart(t, o)).fold(f64::MAX, f64::min) - pair.width as f64).collect();
        assert!(gaps.iter().all(|g| *g > pair.gap as f64 - 2000.0), "{gaps:?}");
        assert!(gaps.iter().filter(|g| (**g - pair.gap as f64).abs() < 2000.0).count() >= gaps.len() - 3, "{gaps:?}");
        b.apply(undo);
        assert!(b.tracks.is_empty());
    }

    #[test]
    fn a_pair_that_would_arrive_crossed_or_break_a_rule_is_not_laid() {
        // The chip's pads the other way round: D+ would end on D-.
        let mut b = board(true);
        let from = b.pair_ends(mm(10.0, 9.5), 50_000, &[0]).unwrap();
        let mut r = PairRouter::new(&b, from, mm(30.0, 16.0));
        let laid = r.to(&b, mm(30.0, 16.2), 50_000);
        assert_eq!(laid.refused.as_deref(), Some("The pair arrives crossed: D+ would end on D-. Bring it in from the other side"));
        assert_eq!((b.add_pair(&from, &laid).err(), b.tracks.len()), (laid.refused.clone(), 0));
        // Nothing yet to lay at its own pads.
        let mut r = PairRouter::new(&b, from, mm(30.0, 16.0));
        let none = Laid { middle: vec![], sides: [vec![], vec![]], to: None, refused: None };
        assert_eq!(b.add_pair(&from, &none).err().as_deref(), Some("Nothing to lay yet: lead the pair away from its pads"));
        assert!(r.to(&b, mm(12.0, 10.0), 50_000).refused.is_none());
    }
}
