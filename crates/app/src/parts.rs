//! Parts from elsewhere: a part's KiCad files from a vendor or a site (or a zip of them) dropped
//! on the window go into the open project's own library (`cuprita_core::parts`), where the Parts
//! tab offers them with KiCad's, the project's first.

use crate::App;
#[cfg(not(target_arch = "wasm32"))]
use crate::{
    dock::Tab,
    tr::{n, tr, trf},
};
use cuprita_core::library::{self, LibPart};
#[cfg(not(target_arch = "wasm32"))]
use cuprita_core::parts::Imported;
#[cfg(not(target_arch = "wasm32"))]
use eframe::egui;
use std::path::{Path, PathBuf};

/// Sites that give a part's KiCad files (a free account each): their names and searches for a part
/// number, which the Parts tab links to for a part KiCad's libraries lack.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SITES: [(&str, &str); 3] = [
    ("SnapMagic", "https://www.snapeda.com/search/?q="),
    ("Ultra Librarian", "https://app.ultralibrarian.com/search?queryText="),
    ("SamacSys", "https://componentsearchengine.com/search?term="),
];

/// The user's downloads folder, where a site's zip lands: XDG's (`~/.config/user-dirs.dirs`, named
/// in the user's language), else `Downloads` in the home folder.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn downloads() -> Option<PathBuf> {
    let home = std::env::home_dir()?;
    let named = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).ok().and_then(|t| {
        let line = t.lines().find(|l| l.starts_with("XDG_DOWNLOAD_DIR="))?;
        Some(PathBuf::from(line.split_once('=')?.1.trim_matches('"').replace("$HOME", &home.to_string_lossy())))
    });
    named.or_else(|| Some(home.join("Downloads"))).filter(|d| d.is_dir())
}

/// `text` as a URL's query value: letters, digits and `-_.~` as they are, the rest percent-encoded.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn query_value(text: &str) -> String {
    let keep = |b: u8| b.is_ascii_alphanumeric() || b"-_.~".contains(&b);
    text.bytes().map(|b| if keep(b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The project's own footprints in `folder`, then KiCad's: what the Parts tab and a footprint's
/// `lib:name` are looked up in.
pub(crate) fn footprints(folder: Option<&Path>) -> Vec<LibPart> {
    let mut all = folder.map(cuprita_core::parts::footprints).unwrap_or_default();
    all.extend(library::footprints());
    all
}

/// The project's own symbols in `folder`, then KiCad's.
pub(crate) fn symbols(folder: Option<&Path>) -> Vec<LibPart> {
    let mut all = folder.map(cuprita_core::parts::symbols).unwrap_or_default();
    all.extend(library::symbols());
    all
}

/// Whether files dropped (`paths`) are a part's, for the open project's libraries, rather than a
/// project to open.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn are_parts(paths: &[PathBuf]) -> bool {
    let part = |p: &PathBuf| {
        p.extension()
            .is_some_and(|e| ["zip", "kicad_sym", "kicad_mod", "step", "stp", "wrl"].contains(&e.to_string_lossy().to_lowercase().as_str()))
    };
    !paths.is_empty() && paths.iter().all(part)
}

impl App {
    /// The open project's folder: its board's, else its schematic's root's.
    pub(crate) fn folder(&self) -> Option<PathBuf> {
        let file = self.path.as_deref().or_else(|| self.sch.as_ref().map(|s| s.sheets.root.as_path()))?;
        file.parent().map(Path::to_path_buf)
    }

    /// Puts part files `paths` into the open project's own library, then shows the part in the
    /// Parts tab, its name typed, for Enter to add.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn import_parts(&mut self, ctx: &egui::Context, paths: &[PathBuf]) {
        let read = |p: &PathBuf| std::fs::read(p).map(|b| (p.file_name().unwrap_or_default().to_string_lossy().into_owned(), b));
        let files = paths.iter().map(|p| read(p).map_err(|e| format!("{}: {e}", p.display()))).collect::<Result<Vec<_>, _>>();
        // The status bar says why it failed.
        let _ = self.put_parts(ctx, |folder, name| cuprita_core::parts::import(folder, name, files?));
    }

    /// Puts a part into the open project's own library with `put` (given its folder and name),
    /// then shows the part in the Parts tab, its name typed, for Enter to add; else says why not
    /// in the status bar, and returns it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn put_parts(
        &mut self,
        ctx: &egui::Context,
        put: impl FnOnce(&Path, &str) -> Result<Imported, String>,
    ) -> Result<(), String> {
        let Some(folder) = self.folder() else {
            let e = tr("Open a project first: a part dropped goes into its own library");
            self.tell(e.into());
            return Err(e.into());
        };
        let name = self.project();
        let done = match put(&folder, &name) {
            Ok(done) => done,
            Err(e) => {
                self.tell(trf("The part wasn't added: {0}", &[&e]));
                return Err(e);
            }
        };
        (self.parts, self.symbols) = (None, None);
        // A model alone, with a part picked on the board, is that part's.
        if let (Some(model), true, Some(crate::Pick::Footprint(i))) =
            (done.models.first(), done.symbols.is_empty() && done.footprints.is_empty() && !self.sch_view(), self.selected)
        {
            self.give_model(i, model);
            return Ok(());
        }
        let came: Vec<String> =
            [(done.symbols.len(), n!("symbol")), (done.footprints.len(), n!("footprint")), (done.models.len(), n!("3D model"))]
                .into_iter()
                .filter(|(n, _)| *n > 0)
                .map(|(n, what)| crate::count(n, what))
                .collect();
        let mut said = trf("Added to {0}'s own library: {1}; KiCad finds them too", &[&name, &came.join(", ")]);
        if !done.skipped.is_empty() {
            said += &trf(" ({0} left out: not a KiCad part's)", &[&crate::count(done.skipped.len(), n!("file"))]);
        }
        self.tell(said);
        // The Parts tab typed with its `lib:name`, which finds it first.
        if let Some(query) = if self.sch_view() { done.symbols.first() } else { done.footprints.first() } {
            self.part_query = query.clone();
            self.left.tab = Tab::Parts;
            self.left.open = true;
            self.open_parts(ctx);
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl App {
    /// Puts 3D model `file` (a vendor's STEP; a WRL KiCad shows) into the open project's
    /// library and gives it to footprint `i`.
    pub(crate) fn attach_model(&mut self, ctx: &egui::Context, i: usize, file: &Path) {
        let file = file.to_path_buf();
        let keep = self.selected;
        self.selected = Some(crate::Pick::Footprint(i));
        self.import_parts(ctx, std::slice::from_ref(&file));
        self.selected = keep;
    }

    /// Gives footprint `i` the 3D model at `path` (as footprints name it) in place of its own, as
    /// one undo step, and has the 3D view read it.
    fn give_model(&mut self, i: usize, path: &str) {
        let Some(f) = self.board.footprints.get(i) else { return };
        let reference = f.reference.clone();
        let model = cuprita_core::board::Model { file: path.to_string(), offset: [0.0; 3], scale: [1.0; 3], rotate: [0.0; 3] };
        let undo = self.board.set_models(i, vec![model]);
        self.edited(undo, web_time::Instant::now(), trf("{0}'s 3D model", &[&reference]));
        self.load_models();
        let file = path.rsplit('/').next().unwrap_or(path);
        self.tell(trf("{0} shows {1} in 3D (Alt+3); Ctrl+S saves it", &[&reference, &file]));
    }
}

impl App {
    /// `text` in the status bar of the view shown: the schematic's or the board's.
    #[cfg(not(target_arch = "wasm32"))]
    fn tell(&mut self, text: String) {
        match (self.sch_view(), &mut self.sch) {
            (true, Some(sch)) => sch.status = text,
            _ => self.status = text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_a_parts_files_from_a_project() {
        let paths = |names: &[&str]| names.iter().map(PathBuf::from).collect::<Vec<_>>();
        assert!(are_parts(&paths(&["LM358.zip"])) && are_parts(&paths(&["U1.kicad_sym", "SOIC-8.kicad_mod", "SOIC-8.STEP"])));
        assert!(!are_parts(&paths(&["glow.kicad_pro"])) && !are_parts(&paths(&["glow.kicad_pcb", "LM358.zip"])) && !are_parts(&[]));
        assert_eq!(query_value("LM358DR/NOPB #1+"), "LM358DR%2FNOPB%20%231%2B");
    }
}
