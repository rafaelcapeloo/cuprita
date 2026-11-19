//! Board files: open one (offering its autosave), save the edits (Ctrl+S), autosave them to
//! KiCad's `_autosave-<file>` once the board rests, and ask before unsaved edits are lost; other
//! files and links open in the system's apps.

use crate::tr::{n, t, tr, trf};
use crate::{App, theme};
use cuprita_core::history::History;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use web_time::Instant; // std's panics in the browser

/// How long the board rests after a change before it is autosaved.
const AUTOSAVE_AFTER: Duration = Duration::from_secs(5);

/// A question waiting for the user's answer.
pub(crate) enum Ask {
    /// The board has unsaved edits: save or drop them, then do this (open a board, close).
    Unsaved(Box<dyn FnOnce(&mut App)>),
    /// The board file has an autosave this old and newer than it: restore it or open the file.
    Restore(PathBuf, Duration),
    /// Sheets of the schematic of root sheet `root` have autosaves newer than them, the newest
    /// this old: restore them or open the files, showing the page of file `at`.
    RestoreSheets { root: PathBuf, at: PathBuf, files: Vec<String>, age: Duration },
}

impl App {
    /// Whether the board or its schematic has edits its files lack.
    pub(crate) fn unsaved(&self) -> bool {
        self.history.unsaved() || self.sch.as_ref().is_some_and(|s| s.sheets.history.unsaved())
    }

    /// Does `then` (opening a board, closing), first asking to save or drop unsaved edits.
    pub(crate) fn leave(&mut self, then: impl FnOnce(&mut App) + 'static) {
        if self.unsaved() {
            self.ask = Some(Ask::Unsaved(Box::new(then)));
        } else {
            then(self);
        }
    }

    /// Opens `url` (or a folder) in the system's browser (file manager); in the browser build,
    /// in a new tab, so the board stays open.
    pub(crate) fn open_url(&mut self, ctx: &egui::Context, url: &str) {
        #[cfg(target_arch = "wasm32")]
        ctx.open_url(egui::OpenUrl::new_tab(url));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            if let Err((cmd, e)) = shell_open(url) {
                self.status = trf("Couldn't open {0} ({1}: {2}): open it yourself", &[&url, &cmd, &e]);
            }
        }
    }

    /// Opens a board file with the design rules of its sibling `.kicad_pro`, if there is one;
    /// with an autosave newer than the file, first asks whether to restore it. A `.kicad_sch`
    /// opens its project's schematic.
    pub(crate) fn open(&mut self, path: &Path) {
        if path.extension().is_some_and(|e| e == "kicad_sch") {
            return self.open_sch(path);
        }
        match autosave_age(path) {
            Some(age) => self.ask = Some(Ask::Restore(path.to_path_buf(), age)),
            None => self.read(path, false),
        }
    }

    /// Opens the board file at `path`, or with `restore` its autosave, whose edits the file lacks.
    fn read(&mut self, path: &Path, restore: bool) {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let file = if restore { autosave_path(path) } else { path.to_path_buf() };
        match read_text(&file) {
            Ok(src) => {
                let pro = read_text(&path.with_extension("kicad_pro")).ok();
                if self.load(Some(path), &name, src, pro.as_deref()) && restore {
                    self.history = History::restored();
                    self.status = trf("Restored the unsaved edits of {0}; Ctrl+S saves them", &[&name]);
                }
            }
            Err(e) => self.status = format!("{}: {e}", file.display()),
        }
    }

    /// Saves the board with its edits (Ctrl+S): over its file, the previous version kept as
    /// `<file>-bak` first; in the browser, as a download.
    pub(crate) fn save(&mut self) {
        if !self.history.unsaved() {
            return self.status = tr("No edits to save").into();
        }
        let t = Instant::now();
        let saved = self.board.write(&self.src).map_err(|e| e.to_string()).and_then(|text| {
            #[cfg(target_arch = "wasm32")]
            let bak = crate::web::download(&[(&self.name, text.as_bytes())]).map(|()| None::<PathBuf>)?;
            #[cfg(not(target_arch = "wasm32"))]
            let bak = save_file(self.path.as_deref().ok_or(tr("no board file"))?, text.as_bytes())?;
            Ok(bak)
        });
        match saved {
            Ok(bak) => {
                self.history.mark_saved();
                self.drop_autosave();
                self.seen_files();
                let saved = if bak.is_some() { t!("Saved {0} in {1}; the previous version is {0}-bak") } else { t!("Saved {0} in {1}") };
                self.status = trf(saved, &[&self.name, &format!("{:.0?}", t.elapsed())]);
                self.keep_standing();
            }
            Err(e) => self.status = trf("{0} not saved: {1}", &[&self.name, &e]),
        }
    }

    /// Autosaves the board, and the schematic's edited sheets, once each has rested
    /// `AUTOSAVE_AFTER` since its last change, not while the pointer is held (kria-k26 takes
    /// ~0.1 s) or a symbol is being placed; back as saved, the autosave goes instead.
    pub(crate) fn autosave(&mut self, ctx: &egui::Context) {
        if let Some(sch) = &mut self.sch
            && let Some(changed) = sch.changed
        {
            let wait = AUTOSAVE_AFTER.saturating_sub(changed.elapsed());
            if !wait.is_zero() {
                ctx.request_repaint_after(wait);
            } else if !ctx.input(|i| i.pointer.any_down()) && sch.placing.is_none() {
                sch.changed = None;
                match sch.sheets.autosave() {
                    Ok(_) => self.autosaved = Some(Instant::now()),
                    Err(e) => sch.status = trf("Autosave failed: {0}; Ctrl+S saves the sheets", &[&e]),
                }
            }
        }
        let (Some(changed), Some(auto)) = (self.changed, self.path.as_deref().map(autosave_path)) else { return };
        let wait = AUTOSAVE_AFTER.saturating_sub(changed.elapsed());
        if !wait.is_zero() {
            return ctx.request_repaint_after(wait);
        }
        if ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        if !self.history.unsaved() {
            return self.drop_autosave();
        }
        self.changed = None;
        match self.board.write(&self.src).map_err(|e| e.to_string()).and_then(|text| write_whole(&auto, &text)) {
            Ok(()) => self.autosaved = Some(Instant::now()),
            Err(e) => self.status = trf("Autosave failed: {0}; Ctrl+S saves the board", &[&e]),
        }
    }

    /// Whether the edits are saved, as the status bar says: all in the files; all in the
    /// autosaves, and when; or some in neither yet.
    pub(crate) fn saved_text(&self) -> String {
        let pending = self.changed.is_some() || self.sch.as_ref().is_some_and(|s| s.changed.is_some());
        match (self.unsaved(), self.autosaved) {
            (false, _) => tr("Saved").into(),
            (true, Some(at)) if !pending => trf("Autosaved {0}", &[&ago(at.elapsed())]),
            (true, _) => tr("Edits not saved").into(),
        }
    }

    /// Forgets the board's autosave. One left behind is older than its board, so never offered.
    fn drop_autosave(&mut self) {
        self.changed = None;
        if let Some(p) = &self.path {
            let _ = std::fs::remove_file(autosave_path(p));
        }
    }

    /// Shows the question waiting for an answer (Esc cancels); the board takes no input meanwhile.
    pub(crate) fn ask_dialog(&mut self, ctx: &egui::Context) {
        let (text, choices) = match &self.ask {
            None => return,
            Some(Ask::Unsaved(_)) => {
                let what = match (self.history.unsaved(), self.sch.as_ref().is_some_and(|s| s.sheets.history.unsaved())) {
                    (true, true) => t!("{0} and its schematic have unsaved edits. Save them first?"),
                    (false, true) => t!("The schematic has unsaved edits. Save them first?"),
                    _ => t!("{0} has unsaved edits. Save them first?"),
                };
                (trf(what, &[&self.name]), [tr("Save"), tr("Don't save"), tr("Cancel")])
            }
            Some(Ask::Restore(path, age)) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let text = trf("{0} has edits that were never saved, autosaved {1}. Restore them?", &[&name, &ago(*age)]);
                (text, [tr("Restore"), tr("Open the saved board"), tr("Cancel")])
            }
            Some(Ask::RestoreSheets { files, age, .. }) => {
                let have = if files.len() == 1 {
                    t!("{0} has edits that were never saved, autosaved {1}. Restore them?")
                } else {
                    t!("{0} have edits that were never saved, autosaved {1}. Restore them?")
                };
                (trf(have, &[&files.join(", "), &ago(*age)]), [tr("Restore"), tr("Open the saved schematic"), tr("Cancel")])
            }
        };
        // A dialog as New project's: the question beside its sign, the answers at its right, the
        // one that keeps the work in copper and last, where the hand ends.
        let frame = egui::Frame::popup(&ctx.global_style()).fill(theme::PANEL).corner_radius(14).inner_margin(22);
        let modal = egui::Modal::new(egui::Id::unique("ask")).frame(frame).show(ctx, |ui| {
            ui.set_width(460.0_f32.min(ctx.content_rect().width() - 80.0));
            ui.horizontal_top(|ui| {
                ui.label(egui::RichText::new(crate::icon::NEEDS).size(22.0).color(theme::WARN));
                ui.add(egui::Label::new(egui::RichText::new(text).size(14.0).color(theme::TEXT)).wrap());
            });
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let first = ui.add(theme::primary(choices[0]));
                let buttons = [first, ui.button(choices[1]), ui.button(choices[2])];
                if ui.memory(|m| m.focused().is_none()) {
                    buttons[0].request_focus(); // Enter or Space picks it
                }
                buttons.iter().position(egui::Response::clicked)
            })
            .inner
        });
        let pick = modal.inner.or(modal.should_close().then_some(2));
        ctx.input_mut(|i| i.events.clear());
        match (pick, self.ask.take_if(|_| pick.is_some())) {
            (Some(0), Some(Ask::Unsaved(then))) => {
                if self.history.unsaved() {
                    self.save();
                }
                if self.sch.as_ref().is_some_and(|s| s.sheets.history.unsaved()) {
                    self.save_sch();
                }
                if !self.unsaved() {
                    then(self);
                }
            }
            (Some(1), Some(Ask::Unsaved(then))) => {
                self.drop_autosave();
                if let Some(sch) = &self.sch {
                    sch.sheets.drop_autosaves();
                }
                then(self);
            }
            (Some(p @ (0 | 1)), Some(Ask::Restore(path, _))) => self.read(&path, p == 0),
            (Some(p @ (0 | 1)), Some(Ask::RestoreSheets { root, at, files, .. })) => {
                self.show_sch(&root, &at, Some(if p == 0 { &files } else { &[] }));
            }
            _ => {}
        }
    }
}

/// KiCad's autosave of a board or sheet file: `_autosave-<file>` next to it, which KiCad offers too.
pub(crate) fn autosave_path(path: &Path) -> PathBuf {
    path.with_file_name(format!("_autosave-{}", path.file_name().unwrap_or_default().to_string_lossy()))
}

/// How old the autosave of the file at `path` is, if it is newer than the file.
pub(crate) fn autosave_age(path: &Path) -> Option<Duration> {
    let auto = written(&autosave_path(path))?;
    written(path).is_none_or(|board| auto > board).then(|| SystemTime::now().duration_since(auto).unwrap_or_default())
}

/// The text of a file of the project shown: the disk's, or in the browser what the page fetched
/// from the server showing the project (`web.rs`), which has no disk to read.
pub(crate) fn read_text(path: &Path) -> std::io::Result<String> {
    #[cfg(target_arch = "wasm32")]
    return crate::web::read_text(path);
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::read_to_string(path)
}

/// Hands `target` (a link, a file, a folder) to the system to open, as a double click would; else
/// the opener and what it said. Never through a command line: a link is a file's words (a part's
/// Datasheet field), and `cmd /C start` runs what follows an `&` in one.
#[cfg(all(not(target_arch = "wasm32"), not(windows)))]
fn shell_open(target: &str) -> Result<(), (&'static str, String)> {
    let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    // Waited for on a thread of its own, so it doesn't linger as a zombie.
    let opened = std::process::Command::new(cmd).arg(target).spawn().map(|mut opener| drop(std::thread::spawn(move || opener.wait())));
    opened.map_err(|e| (cmd, e.to_string()))
}

#[cfg(windows)]
fn shell_open(target: &str) -> Result<(), (&'static str, String)> {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(window: isize, verb: *const u16, file: *const u16, args: *const u16, dir: *const u16, show: i32) -> isize;
    }
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(target));
    // SAFETY: two null-terminated UTF-16 strings that outlive the call; the other pointers may
    // be null; 1 is SW_SHOWNORMAL. Above 32 it opened.
    let code = unsafe { ShellExecuteW(0, verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
    if code > 32 { Ok(()) } else { Err(("ShellExecute", code.to_string())) }
}

/// git in `dir`, to read a project's own repository: set to run none of the programs its
/// `.git/config` may name (a file system monitor, a signature checker), as a project's folder
/// comes from someone else, `.git` and all.
pub(crate) fn git(dir: &Path) -> std::process::Command {
    let mut git = std::process::Command::new("git");
    git.args(["-c", "core.fsmonitor=false", "-c", "log.showSignature=false", "-C"]).arg(dir);
    git
}

/// Refuses to write `path`, a file of Cuprita's own beside the user's (a backup, an autosave),
/// through a link someone put there in its name: it would write over the file the link names.
fn own_file(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            Err(trf("{0} is a link to another file, which Cuprita won't write through: remove it", &[&path.display()]))
        }
        _ => Ok(()),
    }
}

/// When the file at `path` was last written, as `read_text` reads it; `None` when it isn't there.
pub(crate) fn written(path: &Path) -> Option<SystemTime> {
    #[cfg(target_arch = "wasm32")]
    return crate::web::written(path);
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Writes `text` to `path` through a `.part` file, so a crash while writing leaves the last
/// autosave whole.
pub(crate) fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    let part = path.with_extension("part");
    own_file(&part)?;
    own_file(path)?;
    std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, path)).map_err(|e| trf("can't write {0}: {1}", &[&path.display(), &e]))
}

/// `d` ago, in words.
pub(crate) fn ago(d: Duration) -> String {
    match d.as_secs() / 60 {
        0 => tr("just now").into(),
        m @ 1..60 => trf("{0} min ago", &[&m]),
        m @ 60..1440 => trf("{0} h ago", &[&(m / 60)]),
        m => trf("{0} ago", &[&crate::count(m as usize / 1440, n!("day"))]),
    }
}

/// Writes `data` over the file at `path` after copying it to `<file>-bak` (KiCad's name for a
/// backup); returns the copy, `None` when there was no file (or it is gone since it was opened).
pub(crate) fn save_file(path: &Path, data: &[u8]) -> Result<Option<PathBuf>, String> {
    let bak = path.with_file_name(format!("{}-bak", path.file_name().ok_or(tr("no file name"))?.to_string_lossy()));
    own_file(&bak)?;
    let bak = match std::fs::copy(path, &bak) {
        Ok(_) => Some(bak),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(trf("can't back up {0} as {1}: {2}; nothing was written", &[&path.display(), &bak.display(), &e])),
    };
    std::fs::write(path, data).map_err(|e| match &bak {
        Some(b) => trf("can't write {0}: {1}; the previous version is {2}", &[&path.display(), &e, &b.display()]),
        None => trf("can't write {0}: {1}", &[&path.display(), &e]),
    })?;
    Ok(bak)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn writes_no_backup_or_autosave_through_a_link_in_its_name() {
        let dir = std::env::temp_dir().join(format!("cuprita-links-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (board, secret) = (dir.join("a.kicad_pcb"), dir.join("secret"));
        std::fs::write(&board, "old").unwrap();
        std::fs::write(&secret, "keys").unwrap();
        // A project that came with its backup's and its autosave's names pointing elsewhere.
        std::os::unix::fs::symlink(&secret, dir.join("a.kicad_pcb-bak")).unwrap();
        std::os::unix::fs::symlink(&secret, dir.join("_autosave-a.part")).unwrap();
        let (saved, autosaved) = (save_file(&board, b"new"), write_whole(&dir.join("_autosave-a.kicad_pcb"), "new"));
        let read = |p: &Path| std::fs::read_to_string(p).unwrap();
        let (kept, still) = (read(&secret), read(&board));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(saved.unwrap_err().contains("a.kicad_pcb-bak is a link") && autosaved.unwrap_err().contains("_autosave-a.part is a link"));
        assert_eq!((kept.as_str(), still.as_str()), ("keys", "old"));
    }

    #[test]
    fn offers_an_autosave_only_while_newer_than_its_board() {
        let dir = std::env::temp_dir().join(format!("cuprita-autosave-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (board, auto) = (dir.join("a.kicad_pcb"), dir.join("_autosave-a.kicad_pcb"));
        assert_eq!(autosave_path(&board), auto);
        let at = |p: &Path, secs: u64| {
            let t = SystemTime::now() - Duration::from_secs(secs);
            std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
        };
        std::fs::write(&board, "(kicad_pcb)").unwrap();
        assert_eq!(autosave_age(&board), None);
        write_whole(&auto, "(kicad_pcb (edited))").unwrap();
        assert_eq!(
            (std::fs::read_to_string(&auto).unwrap().as_str(), dir.join("_autosave-a.part").exists()),
            ("(kicad_pcb (edited))", false)
        );
        at(&board, 600);
        at(&auto, 300);
        assert_eq!(autosave_age(&board).map(ago), Some("5 min ago".into()));
        // Saved since (by Cuprita or KiCad): the autosave is stale.
        at(&board, 0);
        assert_eq!(autosave_age(&board), None);
        // The board file is gone: its autosave is all there is.
        std::fs::remove_file(&board).unwrap();
        assert!(autosave_age(&board).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn says_how_long_ago() {
        let ago = |s: u64| ago(Duration::from_secs(s));
        assert_eq!(
            [ago(59), ago(60), ago(3599), ago(3600), ago(86400 * 3)],
            ["just now", "1 min ago", "59 min ago", "1 h ago", "3 days ago"]
        );
    }
}
