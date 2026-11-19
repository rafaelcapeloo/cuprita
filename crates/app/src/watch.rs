//! The window follows its files: a board or schematic another program saves (an agent's `save`,
//! KiCad) shows again as saved, the view kept, unless edits here not saved would be lost.

use crate::App;
use crate::tr::trf;
use eframe::egui;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use web_time::Instant;

/// How often the files are looked at.
const EVERY: Duration = Duration::from_secs(1);

/// The files shown, each with when it was written as last seen; when to look again.
#[derive(Default)]
pub(crate) struct Watch {
    files: Vec<(PathBuf, Option<SystemTime>)>,
    next: Option<Instant>,
}

impl Watch {
    /// Of `files`, the ones shown now, those written since last seen (none when they are other
    /// files than then: just read); each is seen as it is now.
    fn changed(&mut self, files: Vec<PathBuf>) -> Vec<PathBuf> {
        let now: Vec<_> = files.into_iter().map(|p| (crate::file::written(&p), p)).map(|(t, p)| (p, t)).collect();
        let same = now.len() == self.files.len() && now.iter().zip(&self.files).all(|(a, b)| a.0 == b.0);
        let changed = now.iter().zip(&self.files).filter(|(a, b)| same && a.1 != b.1).map(|(a, _)| a.0.clone()).collect();
        self.files = now;
        changed
    }
}

impl App {
    /// The board's file, then the schematic's sheet files.
    fn shown_files(&self) -> Vec<PathBuf> {
        let sheets = self.sch.iter().flat_map(|s| s.sheets.project.pages.iter().map(|p| s.sheets.path(&p.file)));
        let sheets: std::collections::BTreeSet<PathBuf> = sheets.collect();
        self.path.iter().cloned().chain(sheets).collect()
    }

    /// The files are as the window knows them: it just wrote them.
    pub(crate) fn seen_files(&mut self) {
        self.watch.changed(self.shown_files());
    }

    /// Once a second, shows again the board or the schematic whose files another program wrote;
    /// with edits here not saved, says so instead.
    pub(crate) fn follow_files(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.watch.next.is_none_or(|t| now >= t) {
            self.watch.next = Some(now + EVERY);
            let changed = self.watch.changed(self.shown_files());
            let name = |p: &PathBuf| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let kept = |files: &str| {
                trf("{0} was saved by another program; your edits here stay (Ctrl+S writes over it, keeping it as -bak)", &[&files])
            };
            let (board, sheets): (Vec<_>, Vec<_>) = changed.iter().partition(|p| Some(*p) == self.path.as_ref());
            if let Some(p) = board.first() {
                if self.history.unsaved() {
                    self.status = kept(&name(p));
                } else {
                    self.reload_board(&name(p));
                }
            }
            let files = sheets.iter().map(|p| name(p)).collect::<Vec<_>>().join(", ");
            match &mut self.sch {
                Some(sch) if !sheets.is_empty() && sch.sheets.history.unsaved() => sch.status = kept(&files),
                Some(_) if !sheets.is_empty() => self.reload_sheets(&files),
                _ => {}
            }
        }
        // While a project shows, the next look comes by itself: one opened since the last look
        // (a served view's, within its first second) isn't among the files seen yet.
        if let Some(t) = self.watch.next.filter(|_| self.path.is_some() || self.sch.is_some()) {
            ctx.request_repaint_after(t.saturating_duration_since(now));
        }
    }

    /// Reads the board's file `name` again, the view kept.
    fn reload_board(&mut self, name: &str) {
        let Some(path) = self.path.clone() else { return };
        let Ok(src) = crate::file::read_text(&path) else { return };
        let pro = crate::file::read_text(&path.with_extension("kicad_pro")).ok();
        // Loading shows the whole board, on the board: the view stays as it was instead.
        let (bench, on_sheet, fit, orbit) = (self.bench, self.on_sheet, self.fit.take(), self.orbit.take());
        let loaded = self.load(Some(&path), &self.name.clone(), src, pro.as_deref());
        (self.bench, self.on_sheet, self.fit, self.orbit) = (bench, on_sheet, fit, orbit);
        if loaded {
            self.status = trf("{0} was saved by another program: shown as it is now", &[&name]);
        }
        self.sync_gpu();
    }

    /// Reads the schematic again, its sheet files `files` changed, at the page shown, the view kept.
    fn reload_sheets(&mut self, files: &str) {
        let Some(sch) = &self.sch else { return };
        let (root, at, other) = (sch.sheets.root.clone(), sch.sheets.path(&sch.sheets.project.pages[sch.page].file), sch.other);
        let fit = self.fit.take();
        let loaded = self.load_sch(&root, &at, Some(&[]));
        self.fit = fit;
        if let (true, Some(sch)) = (loaded, &mut self.sch) {
            sch.other = other;
            sch.status = trf("{0} was saved by another program: shown as it is now", &[&files]);
        }
        self.sync_gpu();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_files_written_since_last_seen() {
        let dir = std::env::temp_dir().join(format!("cuprita-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.kicad_pcb"), dir.join("a.kicad_sch"));
        let write = |p: &std::path::Path, secs| {
            std::fs::write(p, "(x)").unwrap();
            std::fs::File::options().write(true).open(p).unwrap().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
        };
        write(&a, 10);
        write(&b, 10);
        let mut w = Watch::default();
        // Files read are as seen; one written since is changed, once.
        assert!(w.changed(vec![a.clone(), b.clone()]).is_empty());
        write(&b, 20);
        assert_eq!(w.changed(vec![a.clone(), b.clone()]), std::slice::from_ref(&b));
        assert!(w.changed(vec![a.clone(), b.clone()]).is_empty());
        // Other files shown (another board, a sheet added) are just read.
        write(&a, 30);
        assert!(w.changed(vec![a.clone()]).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(w.changed(vec![a]), [dir.join("a.kicad_pcb")]);
    }
}
