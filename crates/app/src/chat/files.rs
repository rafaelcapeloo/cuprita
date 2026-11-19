//! The chat's tools for the project's repo (`files`, `read`): its files listed, and read a page at
//! a time, PDFs with poppler's pdftotext, the datasheets its parts link to downloaded once with
//! curl. Paths stay inside the repo and links are the parts': nothing a file says can make the
//! model send anything elsewhere.

use cuprita_core::board::Board;
use cuprita_core::json::{Val, json_str, list};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The tools, as Ollama's functions, to add to the agents' commands.
pub(super) const TOOLS: &str = r#"{"type":"function","function":{"name":"files","description":"List the files of the project's repo (its git repository, else its folder): their paths and sizes. The board and schematic are there too: the other tools read those.","parameters":{"type":"object","properties":{}}}},
{"type":"function","function":{"name":"read","description":"Read a file of the project's repo by its path (as files gives it), or a part's datasheet by its link (its Datasheet, as find gives it), a page at a time: a PDF's pages, a text file's 100 lines. find: words to look for (absolute maximum, pin 5): the page holding most of them, and which others hold them; page: a page by its number (default 1).","parameters":{"type":"object","properties":{"path":{"type":"string"},"find":{"type":"string"},"page":{"type":"integer"}},"required":["path"]}}}"#;
/// The most files `files` lists.
const LISTED: usize = 300;
/// A text file's lines on a page.
const LINES: usize = 100;

/// Runs tool `name` (`files` or `read`) for the project of the board file `board_path` (whose
/// parts' links `read` may download).
pub(super) fn run(name: &str, args: &Val, board_path: Option<&Path>, board: &Board) -> Result<String, String> {
    let dir = board_path.and_then(Path::parent).ok_or("no project is open")?;
    let root = repo(dir);
    if name == "files" {
        let all = files(&root);
        let size = |p: &PathBuf| std::fs::metadata(root.join(p)).map_or(0, |m| m.len());
        let shown = all.iter().take(LISTED).map(|p| format!(r#"{{"path":{},"bytes":{}}}"#, json_str(&p.to_string_lossy()), size(p)));
        return Ok(format!(r#"{{"files":{},"more":{}}}"#, list(shown), all.len().saturating_sub(LISTED)));
    }
    let at = args.str("path").ok_or("give path: a file files lists, or a datasheet link find gives")?;
    let text = if at.starts_with("http://") || at.starts_with("https://") {
        if !board.footprints.iter().any(|f| f.fields.iter().any(|(_, v)| v == at)) {
            return Err(format!("no part links to {at}: read the links find gives (Datasheet)"));
        }
        pdf(&download(at)?)?
    } else {
        let path = inside(&root, &at.replace("${KIPRJMOD}", &dir.to_string_lossy()))?;
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            pdf(&path)?
        } else {
            std::fs::read_to_string(&path).map_err(|e| format!("{at} isn't read as text: {e}"))?
        }
    };
    Ok(page(&text, args.str("find").unwrap_or_default(), args.num("page")))
}

/// The git repository folder `dir` is in, else `dir`.
fn repo(dir: &Path) -> PathBuf {
    match crate::file::git(dir).args(["rev-parse", "--show-toplevel"]).output() {
        Ok(o) if o.status.success() => PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()),
        _ => dir.to_path_buf(),
    }
}

/// The files of repo `root`, by their paths in it: git's (but those it ignores), else the
/// folder's but hidden ones, KiCad's backups and autosaves.
pub(crate) fn files(root: &Path) -> Vec<PathBuf> {
    let git = crate::file::git(root).args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"]).output();
    if let Ok(o) = &git
        && o.status.success()
    {
        return o.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()).map(|p| PathBuf::from(String::from_utf8_lossy(p).as_ref())).collect();
    }
    let (mut out, mut dirs) = (vec![], vec![PathBuf::new()]);
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(root.join(&d)).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(['.', '~']) || name.starts_with("_autosave-") || name.ends_with("-backups") {
                continue;
            }
            let path = d.join(&name);
            if e.file_type().is_ok_and(|t| t.is_dir()) { dirs.push(path) } else { out.push(path) }
        }
    }
    out.sort();
    out
}

/// The file `at` names (relative to `root`, or absolute), if it is inside `root`.
fn inside(root: &Path, at: &str) -> Result<PathBuf, String> {
    let path = root.join(at).canonicalize().map_err(|e| format!("{at}: {e}; give a path files lists"))?;
    match root.canonicalize() {
        Ok(r) if path.starts_with(&r) => Ok(path),
        _ => Err(format!("{at} isn't in the project's repo: give a path files lists")),
    }
}

/// The PDF at `url`, downloaded once, into Cuprita's data folder (the system's temporary one is
/// everyone's: another user could put a file there in a datasheet's name).
fn download(url: &str) -> Result<PathBuf, String> {
    let dir = crate::recent::dir().unwrap_or_else(std::env::temp_dir).join("datasheets");
    let mut h = DefaultHasher::new();
    url.hash(&mut h);
    let file = dir.join(format!("{:016x}.pdf", h.finish()));
    if file.exists() {
        return Ok(file);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = file.with_extension("part");
    let curl = Command::new("curl")
        .args(["-fsSL", "--globoff", "--proto", "=http,https", "--max-filesize", "100M", "--max-time", "60", "-o"])
        .arg(&part)
        .arg(url)
        .output();
    let curl = curl.map_err(|_| "downloading a datasheet takes curl: install it")?;
    let pdf = curl.status.success() && std::fs::read(&part).is_ok_and(|b| b.starts_with(b"%PDF"));
    if !pdf {
        let _ = std::fs::remove_file(&part);
        let why = String::from_utf8_lossy(&curl.stderr).trim().to_string();
        let why = if why.is_empty() { "a web page, not a PDF".into() } else { why };
        return Err(format!("{url} wasn't downloaded ({why}): the user can open it in a browser"));
    }
    std::fs::rename(&part, &file).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(file)
}

/// The text of the PDF at `path`, its pages apart by form feeds.
// shortcut: poppler's pdftotext reads PDFs, which Windows and macOS lack until it is installed;
// read them in Cuprita when that keeps users from datasheets.
fn pdf(path: &Path) -> Result<String, String> {
    let out = Command::new("pdftotext").args(["-layout", "-enc", "UTF-8"]).arg(path).arg("-").output().map_err(|_| {
        "reading PDFs takes poppler's pdftotext: install poppler-utils (Linux), poppler (macOS: brew install poppler; Windows: its release)"
    })?;
    if !out.status.success() {
        return Err(format!("{} isn't read as a PDF", path.display()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Page `page` of `text` (a PDF's, apart by form feeds, else `LINES` lines), or the one holding
/// most of the words `find`, saying which others hold them.
fn page(text: &str, find: &str, page: Option<f64>) -> String {
    let pages: Vec<String> = if text.contains('\u{c}') {
        text.trim_end_matches('\u{c}').split('\u{c}').map(String::from).collect()
    } else {
        text.lines().collect::<Vec<_>>().chunks(LINES).map(|c| c.join("\n")).collect()
    };
    let n = pages.len().max(1);
    let words: Vec<String> = find.split_whitespace().map(str::to_lowercase).collect();
    // The pages holding the most of the words first, then the most often.
    let score = |p: &String| {
        let p = p.to_lowercase();
        words.iter().map(|w| p.matches(w.as_str()).count()).fold(0, |s, c| s + if c > 0 { 1000 + c.min(100) } else { 0 })
    };
    let mut ranked: Vec<(usize, usize)> = pages.iter().map(score).enumerate().filter(|&(_, s)| s > 0).collect();
    ranked.sort_by_key(|&(i, s)| (std::cmp::Reverse(s), i));
    let (i, note) = match (page, ranked.first()) {
        (Some(k), _) => ((k.max(1.0) as usize).min(n) - 1, String::new()),
        _ if words.is_empty() => (0, String::new()),
        (None, None) => (0, format!("\n(no page holds {find})")),
        (None, Some(&(i, _))) => {
            let also: Vec<String> = ranked[1..].iter().take(10).map(|(k, _)| (k + 1).to_string()).collect();
            (i, if also.is_empty() { String::new() } else { format!("\n(also on pages {})", also.join(", ")) })
        }
    };
    let shown = pages.get(i).map_or(String::new(), |p| p.lines().map(str::trim_end).collect::<Vec<_>>().join("\n"));
    format!("page {} of {n}:\n{}{note}", i + 1, shown.trim_matches('\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_reads_the_repo_by_pages() {
        let (dir, outside) = (std::env::temp_dir().join(format!("cuprita-files-{}", std::process::id())), "cuprita-files-outside");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::write(dir.with_file_name(outside), "").unwrap();
        for f in ["b.kicad_pcb", "docs/notes.md", ".git-hidden/x", "b-backups/b.zip", "_autosave-b.kicad_pcb"] {
            std::fs::create_dir_all(dir.join(f).parent().unwrap()).unwrap();
            std::fs::write(dir.join(f), "").unwrap();
        }
        let notes: Vec<String> =
            (1..=250).map(|i| if i == 180 { "The input takes 5 V at most.".into() } else { format!("line {i}") }).collect();
        std::fs::write(dir.join("docs/notes.md"), notes.join("\n")).unwrap();
        let (board, b) = (dir.join("b.kicad_pcb"), Board::default());
        let run = |name: &str, args: &str| run(name, &Val::parse(args).unwrap(), Some(&board), &b);
        // Not a git repository (the temporary folder isn't in one): the folder's files.
        let listed = Val::parse(&run("files", "{}").unwrap()).unwrap();
        let paths: Vec<_> = listed.arr("files").unwrap().iter().filter_map(|f| f.str("path")).collect();
        assert_eq!(paths, ["b.kicad_pcb", "docs/notes.md"]);
        assert_eq!(run("read", r#"{"path":"docs/notes.md"}"#).unwrap().lines().take(2).collect::<Vec<_>>(), ["page 1 of 3:", "line 1"]);
        let found = run("read", r#"{"path":"docs/notes.md","find":"input"}"#).unwrap();
        assert!(found.starts_with("page 2 of 3:\nline 101") && found.contains("5 V at most"), "{found}");
        assert!(run("read", r#"{"path":"docs/notes.md","page":9}"#).unwrap().starts_with("page 3 of 3:\nline 201"));
        assert!(run("read", r#"{"path":"docs/notes.md","find":"current"}"#).unwrap().ends_with("(no page holds current)"));
        // Nothing outside the repo, and no link a part doesn't hold.
        assert!(run("read", &format!(r#"{{"path":"../{outside}"}}"#)).unwrap_err().contains("isn't in the project's repo"));
        assert!(run("read", r#"{"path":"https://example.com/x.pdf"}"#).unwrap_err().starts_with("no part links to"));
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_file(dir.with_file_name(outside)).unwrap();
    }

    #[test]
    fn finds_the_pages_that_hold_the_words() {
        let text = "Features\u{c}Pinout: pin 5 is EN\u{c}Absolute maximum ratings: VIN 6 V\u{c}Maximum VIN again\u{c}";
        assert_eq!(page(text, "absolute maximum", None), "page 3 of 4:\nAbsolute maximum ratings: VIN 6 V\n(also on pages 4)");
        assert_eq!(page(text, "", Some(2.0)), "page 2 of 4:\nPinout: pin 5 is EN");
    }
}
