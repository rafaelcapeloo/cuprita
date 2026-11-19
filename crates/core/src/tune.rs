//! Length tuning: a straight track given a meander, so its net is as long as asked: as the other
//! nets of a bus, or as the other side of a differential pair (`crates/core/pair.md`).

use crate::board::{Board, MM, NetId, Nm, Pt, Track};
use crate::drc::{self, Item};
use crate::history::Patch;
use crate::tr::{tr, trf};
use crate::units::nm_to_mm;
use std::f64::consts::SQRT_2;
use std::ops::Range;

/// What a tuning did: the patch that undoes it, the tracks in the place of the one tuned, how
/// many turns its meander takes, and how long (nm) the net's tracks are now.
pub struct Tuned {
    pub undo: Patch,
    pub tracks: Range<usize>,
    pub turns: usize,
    pub length: f64,
}

impl Board {
    /// How long `net`'s tracks are together, in nm.
    pub fn net_length(&self, net: NetId) -> f64 {
        self.tracks.iter().filter(|t| t.net == net).map(Track::len).sum()
    }

    /// Makes the tracks of track `i`'s net `target` nm long together, by a meander in the place
    /// of that straight track: turns to one side, their legs three widths apart (or the net's
    /// clearance, when that is more) and as tall as it takes, up to 1 mm (four widths of a wide
    /// track), their corners cut at 45°. On the side where it keeps the design rules, else the
    /// other; or why not: the net is long enough, the track too short for the turns, or no room.
    pub fn tune(&mut self, i: usize, target: f64) -> Result<Tuned, String> {
        let t = *self.tracks.get(i).ok_or_else(|| tr("no such track"))?;
        if t.mid.is_some() {
            return Err(tr("An arc can't take a meander: pick a straight track").into());
        }
        let (now, len, w) = (self.net_length(t.net), t.len(), t.width as f64);
        let keeps = self.class(t.net).map_or(self.rules.clearance, |c| c.clearance) as f64;
        // Legs `s` apart, corners cut by `c`: what a turn's four corners take off its length.
        let s = (3.0 * w).max(w + keeps);
        let (c, lost) = (s / 4.0, s * (2.0 - SQRT_2));
        let low = w.max(2.0 * c);
        let high = (MM as f64).max(4.0 * w).max(2.0 * low);
        let (more, mm) = (target - now, |nm: f64| format!("{} mm", nm_to_mm(nm.round() as Nm)));
        if more < 2.0 * low - lost {
            return Err(match more <= 0.0 {
                true => trf("Its net is {0} long already: a meander only adds length", &[&mm(now)]),
                false => trf("Its net needs {0} more, less than the smallest turn adds ({1})", &[&mm(more), &mm(2.0 * low - lost)]),
            });
        }
        // Turns start a leg's spacing in from each end and lie as far apart.
        let (turns, room) = ((more / (2.0 * high - lost)).ceil() as usize, ((len - s) / (2.0 * s)).floor().max(0.0) as usize);
        if turns > room {
            let most = room as f64 * (2.0 * high - lost);
            return Err(trf(
                "This track has room for {0} of the {1} turns it would take ({2} more at most): pick a longer one, or tune another of its net too",
                &[&room, &turns, &mm(most)],
            ));
        }
        let a = (more / turns as f64 + lost) / 2.0;
        let (ux, uy) = ((t.b.x - t.a.x) as f64 / len, (t.b.y - t.a.y) as f64 / len);
        let mut why = String::new();
        for side in [1.0, -1.0] {
            // Along the track and out to the side, from its start.
            let at = |x: f64, y: f64| Pt {
                x: (t.a.x as f64 + ux * x - uy * y * side).round() as Nm,
                y: (t.a.y as f64 + uy * x + ux * y * side).round() as Nm,
            };
            let mut pts = vec![t.a];
            for k in 0..turns {
                let x = s + 2.0 * s * k as f64;
                pts.extend(
                    [(x - c, 0.0), (x, c), (x, a - c), (x + c, a), (x + s - c, a), (x + s, a - c), (x + s, c), (x + s + c, 0.0)]
                        .map(|(x, y)| at(x, y)),
                );
            }
            pts.push(t.b);
            let new: Vec<Track> = pts.windows(2).filter(|p| p[0] != p[1]).map(|p| Track { a: p[0], b: p[1], ..t }).collect();
            let tracks = i..i + new.len();
            let undo = self.apply(Patch::replace(vec![(i, new)], vec![]));
            let items: Vec<Item> = tracks.clone().map(Item::Track).collect();
            match drc::check_items(self, &items).into_iter().find(|v| !v.warning) {
                None => return Ok(Tuned { undo, tracks, turns, length: self.net_length(t.net) }),
                Some(v) => {
                    self.apply(undo);
                    why = v.what;
                }
            }
        }
        Err(trf("No room for the meander on either side of this track: {0}", &[&why]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20 mm track of net A on the top, between two pads; `more`: other items.
    fn board(more: &str) -> Board {
        Board::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "A") (net 2 "B")
              (footprint "TP" (layer "F.Cu") (at 10 10) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
              (footprint "TP" (layer "F.Cu") (at 30 10) (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
              (segment (start 10 10) (end 30 10) (width 0.2) (layer "F.Cu") (net 1)) {more})"#
        ))
        .unwrap()
    }

    fn mm(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    #[test]
    fn a_meander_makes_a_net_as_long_as_asked() {
        let (orig, mut b) = (board(""), board(""));
        assert_eq!(b.net_length(1), 20e6);
        // 3.3 mm more: a turn 1 mm tall adds 1.649, so three turns, each 0.726 mm tall.
        let done = b.tune(0, 23.3e6).unwrap();
        assert_eq!((done.turns, done.tracks.clone(), b.tracks.len()), (3, 0..25, 25));
        assert!((done.length - 23.3e6).abs() < 20.0 && (b.net_length(1) - 23.3e6).abs() < 20.0, "{}", done.length);
        // It starts and ends where the track did, each stretch going on from the last, on the
        // side it tries first; no rule broken.
        let t = &b.tracks;
        assert_eq!((t[0].a, t[t.len() - 1].b), (mm(10.0, 10.0), mm(30.0, 10.0)));
        assert!(t.windows(2).all(|w| w[0].b == w[1].a) && t.iter().all(|x| (x.width, x.net, x.layer) == (200_000, 1, 0)));
        let far = t.iter().map(|x| x.a.y).max().unwrap();
        assert!(t.iter().all(|x| x.a.y >= 10_000_000) && (10_725_000..10_726_000).contains(&far), "{far}");
        assert!(drc::check(&b).iter().all(|v| v.warning));
        b.apply(done.undo);
        assert_eq!(b.tracks, orig.tracks);
        // A short stretch more takes one low turn.
        let done = b.tune(0, 20.5e6).unwrap();
        assert!((done.turns, (done.length - 20.5e6).abs() < 20.0) == (1, true), "{}", done.length);
    }

    #[test]
    fn a_meander_keeps_the_rules_or_says_why_it_cant() {
        // Another net's track 0.6 mm to the side it tries first: it turns to the other side.
        let above = r#"(segment (start 10 10.6) (end 30 10.6) (width 0.2) (layer "F.Cu") (net 2))"#;
        let below = r#"(segment (start 10 9.4) (end 30 9.4) (width 0.2) (layer "F.Cu") (net 2))"#;
        let mut b = board(above);
        b.tune(0, 22e6).unwrap();
        assert!(b.tracks.iter().filter(|t| t.net == 1).all(|t| t.a.y <= 10_000_000 && t.b.y <= 10_000_000));
        // Tracks on both sides: no room, and nothing changed.
        let mut b = board(&format!("{above} {below}"));
        let (before, why) = (b.tracks.clone(), b.tune(0, 22e6).err().unwrap());
        assert!(
            why.starts_with("No room for the meander on either side of this track: Track (A) on F.Cu and Track (B) on F.Cu touch"),
            "{why}"
        );
        assert_eq!(b.tracks, before);
        // Long enough already; too little to add; more than the track has room for; an arc.
        let mut b = board(r#"(arc (start 40 10) (mid 45 12) (end 50 10) (width 0.2) (layer "F.Cu") (net 1))"#);
        let total = b.net_length(1);
        let why = |b: &mut Board, i, to| b.tune(i, to).err().unwrap();
        assert!(why(&mut b, 0, total).contains("long already"));
        assert!(
            why(&mut b, 0, total + 0.1e6).contains("needs 0.1 mm more, less than the smallest turn adds (0.248528 mm)"),
            "{}",
            why(&mut b, 0, total + 0.1e6)
        );
        assert!(why(&mut b, 0, total + 40e6).contains("room for 16 of the 25 turns"), "{}", why(&mut b, 0, total + 40e6));
        assert_eq!(why(&mut b, 1, total + 5e6), "An arc can't take a meander: pick a straight track");
    }
}
