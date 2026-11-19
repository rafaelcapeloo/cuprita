//! Editing: add a part from a library; move, turn, flip and delete what is picked; fill zones,
//! undo and redo. Every edit keeps the patch that undoes it, then redraws and re-checks what it
//! changed.

use crate::grid::snap;
use crate::tr::{n, t, tr, trf};
use crate::{App, count, picks_name};
use cuprita_core::board::{Board, Footprint, Nm, Pt};
use cuprita_core::drc::{self, Violation};
use cuprita_core::history::{Patch, Pick};
use cuprita_core::library::LibPart;
use cuprita_core::ratsnest::{self, Link};
use cuprita_core::tidy::Tidied;
use cuprita_render::Scene;
use std::collections::HashSet;
use web_time::Instant; // std's panics in the browser

impl App {
    /// Adds library part `part` on the grid point in the middle of the view (A, then Enter or a
    /// click in the list), selected so a drag moves it and R turns it; the other tools stop.
    pub(crate) fn add_part(&mut self, part: &LibPart) {
        let Some(scene) = &self.scene else { return };
        let (t, at, undo) = (Instant::now(), self.on_grid(scene.nm(self.center)), Patch::added(&self.board));
        let ticked = ticked_lines(&self.board.footprints, &self.bom, &self.ticked);
        match add_from_library(&mut self.board, part, at, 0.0) {
            Ok(i) => {
                (self.measuring, self.routing, self.zoning, self.route, self.zone) = (false, false, false, None, None);
                (self.selected, self.picked) = (Some(Pick::Footprint(i)), Vec::new());
                self.retick(ticked);
                self.edited(undo, t, trf("{0} added", &[&self.board.footprints[i].reference]));
            }
            Err(e) => self.status = e,
        }
    }

    /// Moves what is picked one grid step along `(dx, dy)` as seen (the arrow keys): the board
    /// seen from the back runs the other way.
    pub(crate) fn nudge(&mut self, (dx, dy): (i32, i32)) {
        let step = self.board_grid().1;
        let dx = if self.flipped { -dx } else { dx };
        self.move_picks(Pt { x: dx * step, y: dy * step });
    }

    /// Moves what is picked by `by` (a drag, the arrow keys) as one edit, then redraws and
    /// re-checks the board. A track being routed stops there: its router knew the board as it was.
    pub(crate) fn move_picks(&mut self, by: Pt) {
        let (t, picks) = (Instant::now(), self.picks().to_vec());
        let undo = self.board.move_items(&picks, by);
        self.route = None;
        self.edited(undo, t, trf("{0} moved", &[&picks_name(&self.board, &picks)]));
    }

    /// Turns what is picked a quarter turn, counter-clockwise or (Shift+R) not, about `middle`.
    pub(crate) fn turn_picks(&mut self, ccw: bool) {
        let (t, picks) = (Instant::now(), self.picks().to_vec());
        let Some(about) = middle(&self.board, &picks, self.board_grid()) else { return };
        let undo = self.board.turn_items(&picks, about, if ccw { 90.0 } else { -90.0 });
        self.route = None;
        self.edited(undo, t, trf("{0} turned", &[&picks_name(&self.board, &picks)]));
    }

    /// Drags track `i` by `by`, its neighbours following (`Board::drag_segment`); says why not.
    pub(crate) fn drag_track(&mut self, i: usize, by: Pt) {
        let t = Instant::now();
        match self.board.drag_segment(i, by) {
            Ok(undo) => {
                self.route = None;
                self.edited(undo, t, trf("{0} dragged", &[&crate::pick_name(&self.board, Pick::Track(i))]));
            }
            Err(why) => self.status = why,
        }
    }

    /// Makes track `i`'s net `target` nm long by a meander on that track (`Board::tune`), picked
    /// after; refused, Inspect says why beside the track.
    pub(crate) fn tune(&mut self, i: usize, target: f64) {
        let (t, name) = (Instant::now(), crate::pick_name(&self.board, Pick::Track(i)));
        match self.board.tune(i, target) {
            Ok(done) => {
                (self.selected, self.picked) = crate::canvas::one_or_more(done.tracks.clone().map(Pick::Track).collect());
                self.route = None;
                let turns = count(done.turns, n!("turn"));
                self.edited(done.undo, t, trf("{0} tuned: its net {1} long, by {2}", &[&name, &crate::units::len(done.length), &turns]));
            }
            Err(why) => self.refused = Some((Pick::Track(i), why)),
        }
    }

    /// U: the runs of the tracks picked join what is picked (`Board::run`).
    pub(crate) fn pick_runs(&mut self) {
        let mut all = self.picks().to_vec();
        for &p in self.picks() {
            if let Pick::Track(i) = p {
                all.extend(self.board.run(i).into_iter().map(Pick::Track));
            }
        }
        all.sort_unstable();
        all.dedup();
        self.status = trf("Picked: {0}", &[&picks_name(&self.board, &all)]);
        (self.selected, self.picked) = crate::canvas::one_or_more(all);
    }

    /// Puts the parts picked on the other side, mirrored as KiCad's Flip does (F): together about
    /// the line through their `middle`, so they mirror as one.
    pub(crate) fn flip_picks(&mut self) {
        let (t, picks) = (Instant::now(), self.picks().to_vec());
        let Some(about) = middle(&self.board, &picks, self.board_grid()) else { return };
        let undo = self.board.flip_items(&picks, about);
        self.route = None;
        let what = match picks[..] {
            [Pick::Footprint(i)] => flipped(&self.board.footprints[i]),
            _ => trf("{0} flipped", &[&picks_name(&self.board, &picks)]),
        };
        self.edited(undo, t, what);
        let back = if picks.len() == 1 { t!("{0}. Ctrl+Z puts it back") } else { t!("{0}. Ctrl+Z puts them back") };
        self.status = trf(back, &[&self.status]);
    }

    /// Tidies the texts of the parts picked, else of every part (T): their references where they
    /// read on the silkscreen, their values inside them on the Fab layer (`Board::tidy_texts`),
    /// as one edit.
    pub(crate) fn tidy_texts(&mut self) {
        let t = Instant::now();
        let parts: Vec<usize> = self.picks().iter().filter_map(|p| if let Pick::Footprint(i) = p { Some(*i) } else { None }).collect();
        let (undo, done) = self.board.tidy_texts(&parts);
        if done.parts == 0 {
            return self.status = tr("The texts are tidy already: nothing moved").into();
        }
        self.edited(undo, t, tidied(&done));
    }

    /// Deletes what is picked (Delete or Backspace; on a zone's corner, that corner): parts with
    /// their pads, graphics and texts, tracks, vias, zones with their fills, as one edit; then
    /// redraws and re-checks the board.
    pub(crate) fn delete(&mut self) {
        if self.delete_corner() {
            return;
        }
        let picks = self.picks().to_vec();
        if picks.is_empty() {
            return;
        }
        // A part keeps its texts: one picked alone is hidden.
        let hides = picks.iter().all(|p| matches!(*p, Pick::Text(i) if self.board.text_part(i).is_some()));
        let what = trf(if hides { t!("{0} hidden") } else { t!("{0} deleted") }, &[&picks_name(&self.board, &picks)]);
        let t = Instant::now();
        let ticked = ticked_lines(&self.board.footprints, &self.bom, &self.ticked);
        let undo = self.board.delete_items(&picks);
        (self.selected, self.picked, self.route, self.grab) = (None, Vec::new(), None, None);
        self.retick(ticked);
        self.edited(undo, t, what);
    }

    /// Shows hidden text `k` of part `part` again (Inspect's Hidden texts), picked to drag into
    /// place; its layer too, when that was hidden.
    pub(crate) fn show_text(&mut self, part: usize, k: usize) {
        let (t, (pick, undo)) = (Instant::now(), self.board.show_text(part, k));
        let Pick::Text(i) = pick else { return };
        let on = &self.board.layers[self.board.texts[i].layer as usize];
        if let Some(l) = self.scene.as_mut().and_then(|s| s.layers.iter_mut().find(|l| l.name == *on)) {
            l.visible = true;
        }
        (self.selected, self.picked) = (Some(pick), Vec::new());
        self.edited(undo, t, trf("{0} shown", &[&crate::pick_name(&self.board, pick)]));
    }

    /// Fills every zone again (B), as KiCad's Fill All Zones; an edit when a fill changed.
    pub(crate) fn fill(&mut self) {
        let (t, undo) = (Instant::now(), Patch::fills(&self.board));
        match self.board.fill_zones() {
            0 => self.status = trf("Zones filled in {0}: no fill changed", &[&format!("{:.0?}", t.elapsed())]),
            k => self.edited(undo, t, count(k, n!("zone filled|zones filled"))),
        }
    }

    /// After an edit that started at `t` and is undone by `undo`: refills the zones whose fill it
    /// changes (`Board::refill`, one undo step with it), re-checks what it changed, redraws the
    /// board and keeps the edit for undo.
    pub(crate) fn edited(&mut self, undo: Patch, t: Instant, what: String) {
        self.refused = None;
        let (undo, n) = self.board.refill(undo);
        let (old, vs, rs) = (self.scene.take(), std::mem::take(&mut self.violations), std::mem::take(&mut self.ratsnest));
        let origin = old.as_ref().map(|s| s.origin);
        let (vs, rs, scene) = rechecked(&self.board, &undo, vs, rs, old);
        (self.violations, self.ratsnest) = (vs, rs);
        let shown = match n {
            0 => what.clone(),
            n => format!("{what}, {}", count(n, crate::tr::n!("zone refilled|zones refilled"))),
        };
        self.redraw(t, &shown, scene, origin);
        self.history.push(undo, what);
    }

    /// Undoes the last edit, or redoes the last one undone (Ctrl+Z, Ctrl+Y). A track being
    /// routed or a part being dragged stops first.
    pub(crate) fn undo(&mut self, redo: bool) {
        let (t, ticked) = (Instant::now(), ticked_lines(&self.board.footprints, &self.bom, &self.ticked));
        let (done, b) = (if redo { t!("Redone: {0}") } else { t!("Undone: {0}") }, &mut self.board);
        let Some((what, back)) = (if redo { self.history.redo(b) } else { self.history.undo(b) }) else {
            return self.status = tr(if redo { t!("Nothing to redo") } else { t!("Nothing to undo") }).into();
        };
        let (old, vs, rs) = (self.scene.take(), std::mem::take(&mut self.violations), std::mem::take(&mut self.ratsnest));
        let origin = old.as_ref().map(|s| s.origin);
        let (vs, rs, scene) = rechecked(&self.board, back, vs, rs, old);
        (self.violations, self.ratsnest) = (vs, rs);
        let what = trf(done, &[&what]);
        // A part, track or via that came or went moved the ones after it: nothing stays picked.
        (self.route, self.grab, self.selected, self.picked, self.boxing) = (None, None, None, Vec::new(), None);
        self.retick(ticked);
        self.redraw(t, &what, scene, origin);
    }

    /// After parts came or went, lines the bench BOM up with them again, ticking the lines that
    /// list what a `ticked` line did.
    fn retick(&mut self, ticked: HashSet<LineKey>) {
        let bom = self.board.bom();
        if bom != self.bom {
            (self.ticked, self.bom, self.bom_line) = (ticks(&self.board.footprints, &bom, &ticked), bom, None);
        }
    }

    /// After a change to the board that started at `t`, re-checked and drawn again as `scene`
    /// (`rechecked`; the scene before had its `origin`): shows the board, says `what` was done
    /// and starts the wait to autosave it.
    fn redraw(&mut self, t: Instant, what: &str, scene: Scene, origin: Option<Pt>) {
        self.changed = Some(Instant::now());
        // Without an outline the origin follows the parts: keep the view on the same spot.
        if let Some(o) = origin {
            let [dx, dy] = scene.um(o);
            (self.center, self.fitted) = ([self.center[0] + dx, self.center[1] + dy], None);
        }
        // A diff or heatmap of the old board would mislead: back to the board view.
        (self.scene, self.diff, self.show_diff, self.ir, self.show_ir, self.found) = (Some(scene), None, false, None, false, Vec::new());
        self.sync_gpu();
        self.status = trf("{0}, redrawn and checked in {1}", &[&what, &format!("{:.0?}", t.elapsed())]);
    }
}

/// The board re-checked and drawn again after a change `patch` undoes: its DRC violations and
/// ratsnest from `vs` and `rs` (`drc::recheck`, `ratsnest::recheck`), its scene from `old`; at
/// once, each on its own core (but in the browser, which has one).
fn rechecked(b: &Board, patch: &Patch, vs: Vec<Violation>, rs: Vec<Link>, old: Option<Scene>) -> (Vec<Violation>, Vec<Link>, Scene) {
    let scene = |old: Option<Scene>| old.map_or_else(|| cuprita_render::build(b), |old| cuprita_render::rebuild(b, old));
    #[cfg(target_arch = "wasm32")]
    return (drc::recheck(b, vs, patch), ratsnest::recheck(b, rs, patch), scene(old));
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::scope(|s| {
        let (vs, rs) = (s.spawn(|| drc::recheck(b, vs, patch)), s.spawn(|| ratsnest::recheck(b, rs, patch)));
        let scene = scene(old);
        (vs.join().expect("the DRC re-check panicked"), rs.join().expect("the ratsnest re-check panicked"), scene)
    })
}

/// Where what is picked turns and flips about: a part or text alone's anchor (where it stays),
/// else the grid point nearest the middle of their box, so what was on the grid stays on it.
fn middle(b: &Board, picks: &[Pick], grid: (Pt, Nm)) -> Option<Pt> {
    match picks {
        [Pick::Footprint(i)] => return Some(b.footprints[*i].at),
        [Pick::Text(i)] => return Some(b.texts[*i].at),
        _ => {}
    }
    let (lo, hi) = b.extent(picks)?;
    Some(snap(Pt { x: lo.x / 2 + hi.x / 2, y: lo.y / 2 + hi.y / 2 }, grid))
}

/// What flipping part `f` did, as its undo step says it: `U3 flipped to the bottom`.
pub(crate) fn flipped(f: &Footprint) -> String {
    trf(if f.back { t!("{0} flipped to the bottom") } else { t!("{0} flipped to the top") }, &[&f.reference])
}

/// What tidying texts did, as its undo step says it: `Texts tidied: 17 references placed, 4
/// hidden with no room; 21 values in their parts`.
fn tidied(d: &Tidied) -> String {
    let hidden = |n: usize, forms: &'static str| if n == 0 { String::new() } else { format!(", {}", count(n, forms)) };
    let refs = count(d.placed, n!("reference placed|references placed")) + &hidden(d.hidden, n!("hidden with no room|hidden with no room"));
    let values =
        count(d.values, n!("value in its part|values in their parts")) + &hidden(d.values_hidden, n!("value hidden|values hidden"));
    trf("Texts tidied: {0}; {1}", &[&refs, &values])
}

/// What to do when no KiCad library was found.
pub(crate) const NO_LIBRARIES: &str =
    t!("no KiCad footprint libraries found: install KiCad, or set KICAD10_FOOTPRINT_DIR to its footprints folder");

/// The `.kicad_mod` text of footprint `id` (`Library:Name`) of KiCad's libraries, `parts`.
pub(crate) fn footprint_text(parts: &[LibPart], id: &str) -> Result<String, String> {
    let (lib, name) = id.split_once(':').unwrap_or_default();
    let part = parts.iter().find(|p| p.lib == lib && p.name == name);
    let part = part
        .ok_or_else(|| trf("its footprint {0} isn't in KiCad's footprint libraries: set its Footprint field to one of theirs", &[&id]))?;
    std::fs::read_to_string(&part.path).map_err(|e| format!("{}: {e}", part.path.display()))
}

/// Reads library part `part` and adds it to `board` at `at`, turned `angle` degrees; an error
/// names the file and where in it.
pub(crate) fn add_from_library(board: &mut Board, part: &LibPart, at: Pt, angle: f64) -> Result<usize, String> {
    let text = std::fs::read_to_string(&part.path).map_err(|e| format!("{}: {e}", part.path.display()))?;
    board.add_footprint(&text, &part.lib, at, angle).map_err(|e| {
        let (line, col) = e.line_col(&text);
        format!("{}:{line}:{col}: {}", part.path.display(), e.msg)
    })
}

impl App {
    /// Keeps the bench BOM's ticks, by what their lines list, for when the board opens again:
    /// beside its picture in Cuprita's data folder (`recent.rs`).
    pub(crate) fn keep_ticks(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(file) = self.path.as_deref().and_then(crate::recent::thumb).map(|t| t.with_extension("ticks")) {
            let text = ticks_text(&ticked_lines(&self.board.footprints, &self.bom, &self.ticked));
            let _ = std::fs::create_dir_all(file.parent().unwrap_or(std::path::Path::new("."))).and_then(|()| std::fs::write(file, text));
        }
    }

    /// The ticks kept for the bench BOM of the board open (none in the browser).
    pub(crate) fn kept_ticks(&self) -> Vec<bool> {
        let file = self.path.as_deref().and_then(crate::recent::thumb).map(|t| t.with_extension("ticks"));
        let text = file.and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        ticks(&self.board.footprints, &self.bom, &parse_ticks(&text))
    }
}

/// What a bench BOM line lists: its parts' value, footprint and part numbers, which name the
/// line after parts come or go.
type LineKey = (String, String, [String; 3]);

/// Ticked lines as kept: a line each, its key's parts apart by tabs.
#[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "the browser keeps no ticks"))]
fn ticks_text(ticked: &HashSet<LineKey>) -> String {
    let mut lines: Vec<String> = ticked.iter().map(|(v, lib, [m, p, l])| format!("{v}\t{lib}\t{m}\t{p}\t{l}\n")).collect();
    lines.sort();
    lines.concat()
}

fn parse_ticks(text: &str) -> HashSet<LineKey> {
    let key = |l: &str| match l.split('\t').collect::<Vec<_>>()[..] {
        [v, lib, m, p, n] => Some((v.into(), lib.into(), [m.into(), p.into(), n.into()])),
        _ => None,
    };
    text.lines().filter_map(key).collect()
}

fn line_key(fp: &[Footprint], line: &[usize]) -> LineKey {
    let f = &fp[line[0]];
    (f.value.clone(), f.lib_id.clone(), f.part_numbers().map(String::from))
}

/// What the ticked lines of a bench BOM list.
fn ticked_lines(fp: &[Footprint], bom: &[Vec<usize>], ticked: &[bool]) -> HashSet<LineKey> {
    bom.iter().zip(ticked).filter(|&(_, &done)| done).map(|(line, _)| line_key(fp, line)).collect()
}

/// The ticks of `bom`'s lines: those that list what a line in `ticked` did.
fn ticks(fp: &[Footprint], bom: &[Vec<usize>], ticked: &HashSet<LineKey>) -> Vec<bool> {
    bom.iter().map(|line| ticked.contains(&line_key(fp, line))).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_ticked_lines_as_text() {
        let ticked: super::HashSet<super::LineKey> =
            [("10k".into(), "Resistor_SMD:R_0603".into(), ["".into(), "RC0603".into(), "C25804".into()])].into();
        assert_eq!(super::parse_ticks(&super::ticks_text(&ticked)), ticked);
        assert!(super::parse_ticks("not a line").is_empty());
    }

    use super::*;

    #[test]
    fn turns_a_part_about_its_anchor_and_more_about_the_grid_point_in_their_middle() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "")
              (footprint "R" (at 10.03 20.01) (property "Reference" "R1") (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu")))
              (segment (start 0 0) (end 5.3 0) (width 0.25) (layer "F.Cu"))
              (via (at 5 5.1) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu")))"#,
        )
        .unwrap();
        let (grid, mm) = ((Pt::default(), 250_000), |x: f64, y: f64| Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm });
        // R1 alone stays where it is, off the grid; the track and via about (2.65, 2.55) on it.
        assert_eq!(middle(&b, &[Pick::Footprint(0)], grid), Some(mm(10.03, 20.01)));
        assert_eq!(middle(&b, &[Pick::Track(0), Pick::Via(0)], grid), Some(mm(2.75, 2.5)));
        assert_eq!(middle(&b, &[], grid), None);
    }

    #[test]
    fn bench_ticks_stay_on_their_lines_when_parts_go() {
        let part = |r: &str, v: &str| format!(r#"(footprint "R" (at 0 0) (property "Reference" "{r}") (property "Value" "{v}"))"#);
        let mut b = Board::parse(&format!("(kicad_pcb {} {} {})", part("R1", "10k"), part("C1", "1u"), part("R2", "10k"))).unwrap();
        let bom = b.bom();
        // The 10k line is ticked; R1 goes, so C1 and R2 move up a place.
        let ticked = ticked_lines(&b.footprints, &bom, &[false, true]);
        assert_eq!(bom, [vec![1], vec![0, 2]]);
        b.apply(Patch::delete(&b, Pick::Footprint(0)));
        assert_eq!((b.bom(), ticks(&b.footprints, &b.bom(), &ticked)), (vec![vec![0], vec![1]], vec![false, true]));
    }
}
