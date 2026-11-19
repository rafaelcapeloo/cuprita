//! The projects opened lately, and a picture of each board, kept in Cuprita's folder of the
//! user's data: `$XDG_DATA_HOME/cuprita` (`~/.local/share/cuprita`), `~/Library/Application
//! Support/Cuprita` on macOS, `%APPDATA%\Cuprita` on Windows.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

/// How many projects the list keeps.
const KEPT: usize = 12;

/// Cuprita's folder of the user's data.
pub(crate) fn dir() -> Option<PathBuf> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        var("APPDATA").map(|d| d.join("Cuprita"))
    } else if cfg!(target_os = "macos") {
        std::env::home_dir().map(|h| h.join("Library/Application Support/Cuprita"))
    } else {
        var("XDG_DATA_HOME").or_else(|| std::env::home_dir().map(|h| h.join(".local/share"))).map(|d| d.join("cuprita"))
    }
}

/// A choice kept for the next run as `name` (the units, the language): a file in the data
/// folder, or the browser's storage.
pub(crate) fn kept(name: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    return dir().and_then(|d| std::fs::read_to_string(d.join(name)).ok()).map(|s| s.trim().to_string());
    #[cfg(target_arch = "wasm32")]
    return web_sys::window()?.local_storage().ok()??.get_item(&format!("cuprita-{name}")).ok()?;
}

/// Keeps `value` as `name` for the next run; a failure to keep it only forgets it.
pub(crate) fn keep(name: &str, value: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(d) = dir() {
        let _ = std::fs::create_dir_all(&d).and_then(|()| std::fs::write(d.join(name), value));
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(&format!("cuprita-{name}"), value);
    }
}

/// The projects opened lately, newest first: those whose files are still there.
pub(crate) fn read() -> Vec<PathBuf> {
    let Some(text) = dir().and_then(|d| std::fs::read_to_string(d.join("recent")).ok()) else { return vec![] };
    text.lines().map(PathBuf::from).filter(|p| p.is_file()).take(KEPT).collect()
}

/// Puts `path` first in `list` and keeps the list; a failure to write it is no one's business.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn remember(list: &mut Vec<PathBuf>, path: &Path) {
    put_first(list, path);
    keep_list(list);
}

/// Takes `path` off `list` and keeps the list.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn forget(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    keep_list(list);
}

#[cfg(not(target_arch = "wasm32"))]
fn keep_list(list: &[PathBuf]) {
    if let Some(d) = dir() {
        let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
        let _ = std::fs::create_dir_all(&d).and_then(|()| std::fs::write(d.join("recent"), text));
    }
}

/// Where the project of the board at `path` stood when it was last open, kept beside its
/// picture: what (`route`) and how many, worded where it shows, in the language shown then.
pub(crate) fn standing(path: &Path) -> Option<(String, usize)> {
    let text = std::fs::read_to_string(thumb(path)?.with_extension("txt")).ok()?;
    let (what, n) = text.trim_end().split_once(' ')?;
    Some((what.to_string(), n.parse().ok()?))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn keep_standing(path: &Path, what: &str, n: usize) {
    if let Some(file) = thumb(path).map(|t| t.with_extension("txt")) {
        let _ = std::fs::create_dir_all(file.parent().unwrap_or(Path::new("."))).and_then(|()| std::fs::write(file, format!("{what} {n}")));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn put_first(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(KEPT);
}

/// Where the picture of the board at `path` is kept.
pub(crate) fn thumb(path: &Path) -> Option<PathBuf> {
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    Some(dir()?.join("thumbs").join(format!("{:016x}.png", h.finish())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_first_once_each() {
        let mut list: Vec<PathBuf> = (0..KEPT).map(|n| PathBuf::from(format!("/{n}.kicad_pcb"))).collect();
        put_first(&mut list, Path::new("/3.kicad_pcb"));
        put_first(&mut list, Path::new("/new.kicad_pcb"));
        assert_eq!(list[..3], ["/new.kicad_pcb", "/3.kicad_pcb", "/0.kicad_pcb"].map(PathBuf::from));
        assert_eq!(list.len(), KEPT);
        assert_ne!(thumb(Path::new("/a")), thumb(Path::new("/b")));
    }
}
