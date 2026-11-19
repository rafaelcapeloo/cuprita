//! A new KiCad project: from nothing (its `.kicad_pro`, root `.kicad_sch` and `.kicad_pcb` as
//! KiCad 10 writes them: a title, the copper layers and the board's outline), or as a copy of one
//! of KiCad's templates; and KiCad's templates and demo projects, found with nothing to set up.

use crate::library::kicad_dirs;
use crate::save::uuid;
use crate::tr::trf;
use std::path::{Path, PathBuf};

/// The three files of a new project `name`: a board of `copper` copper layers (`LAYERS`'s: an
/// even number, as KiCad's boards have) whose outline is a `width` × `height` mm rectangle, and
/// an empty root sheet. Each is its file name and text; `seed` makes the root sheet's id (the
/// project file names it).
pub fn files(name: &str, copper: u8, (width, height): (f64, f64), seed: u64) -> [(String, String); 3] {
    let root = uuid(("root sheet", name, seed));
    let pro = format!(
        r#"{{
  "board": {{ "design_settings": {{ "defaults": {{}}, "rules": {{}} }} }},
  "meta": {{ "filename": {file}, "version": 3 }},
  "net_settings": {{
    "classes": [
      {{ "name": "Default", "clearance": 0.2, "track_width": 0.2, "via_diameter": 0.6, "via_drill": 0.3, "priority": 2147483647 }}
    ],
    "meta": {{ "version": 4 }}
  }},
  "sheets": [ [ "{root}", "Root" ] ]
}}
"#,
        file = crate::json::json_str(&format!("{name}.kicad_pro")),
    );
    let title = sexpr_str(name);
    let sch = format!(
        "(kicad_sch\n\t(version 20260306)\n\t(generator \"cuprita\")\n\t(generator_version \"{v}\")\n\t(uuid \"{root}\")\n\t(paper \"A4\")\n\t(title_block\n\t\t(title {title})\n\t)\n\t(lib_symbols)\n\t(sheet_instances\n\t\t(path \"/\"\n\t\t\t(page \"1\")\n\t\t)\n\t)\n\t(embedded_fonts no)\n)\n",
        v = env!("CARGO_PKG_VERSION"),
    );
    // KiCad 10's layer numbers: front copper 0, back 2, inner n 2n + 2; the others as it writes them.
    let inner = copper.clamp(*LAYERS.start(), *LAYERS.end()) / 2 * 2 - 2;
    let mut layers = String::from("\t\t(0 \"F.Cu\" signal)\n");
    for n in 1..=inner {
        layers += &format!("\t\t({} \"In{n}.Cu\" signal)\n", 2 * n + 2);
    }
    layers += "\t\t(2 \"B.Cu\" signal)\n";
    for (id, name, user) in [
        (9, "F.Adhes", "F.Adhesive"),
        (11, "B.Adhes", "B.Adhesive"),
        (13, "F.Paste", ""),
        (15, "B.Paste", ""),
        (5, "F.SilkS", "F.Silkscreen"),
        (7, "B.SilkS", "B.Silkscreen"),
        (1, "F.Mask", ""),
        (3, "B.Mask", ""),
        (17, "Dwgs.User", "User.Drawings"),
        (19, "Cmts.User", "User.Comments"),
        (25, "Edge.Cuts", ""),
        (27, "Margin", ""),
        (31, "F.CrtYd", "F.Courtyard"),
        (29, "B.CrtYd", "B.Courtyard"),
        (35, "F.Fab", ""),
        (33, "B.Fab", ""),
    ] {
        let user = if user.is_empty() { String::new() } else { format!(" \"{user}\"") };
        layers += &format!("\t\t({id} \"{name}\" user{user})\n");
    }
    // The outline's top left corner where KiCad users put boards on the A4 page.
    let (x0, y0) = (100.0, 60.0);
    let outline = format!(
        "\t(gr_rect\n\t\t(start {x0} {y0})\n\t\t(end {} {})\n\t\t(stroke\n\t\t\t(width 0.05)\n\t\t\t(type default)\n\t\t)\n\t\t(fill no)\n\t\t(layer \"Edge.Cuts\")\n\t\t(uuid \"{}\")\n\t)\n",
        x0 + width,
        y0 + height,
        uuid(("outline", name, seed)),
    );
    let pcb = format!(
        "(kicad_pcb\n\t(version 20260206)\n\t(generator \"cuprita\")\n\t(generator_version \"{v}\")\n\t(general\n\t\t(thickness 1.6)\n\t\t(legacy_teardrops no)\n\t)\n\t(paper \"A4\")\n\t(title_block\n\t\t(title {title})\n\t)\n\t(layers\n{layers}\t)\n\t(setup\n\t\t(pad_to_mask_clearance 0)\n\t\t(allow_soldermask_bridges_in_footprints no)\n\t)\n\t(net 0 \"\")\n{outline}\t(embedded_fonts no)\n)\n",
        v = env!("CARGO_PKG_VERSION"),
    );
    [(format!("{name}.kicad_pro"), pro), (format!("{name}.kicad_sch"), sch), (format!("{name}.kicad_pcb"), pcb)]
}

/// How many copper layers a new board may have: KiCad's 2 to 32, an even number.
pub const LAYERS: std::ops::RangeInclusive<u8> = 2..=32;

/// A seed for a new project's ids: the time, so two projects never share them.
pub fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

/// `s` as a KiCad string: quoted, its quotes and backslashes escaped.
fn sexpr_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// One of KiCad's templates: its folder, its project's name in it, and what its `meta/info.html`
/// calls it.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    pub dir: PathBuf,
    pub name: String,
    pub title: String,
}

impl Template {
    /// Its board, to show it.
    pub fn board(&self) -> PathBuf {
        self.dir.join(format!("{}.kicad_pcb", self.name))
    }
}

/// KiCad's project templates that have a board, by title: in the folders `KICAD*_TEMPLATE_DIR`
/// name, then where KiCad's packages put them.
pub fn templates() -> Vec<Template> {
    // KiCad_* are the library's own drawing aids, not boards to start from.
    let mut all: Vec<Template> = projects(&kicad_dirs("template", "_TEMPLATE_DIR"), 1)
        .into_iter()
        .filter(|(dir, name)| {
            !dir.file_name().is_some_and(|f| f.to_string_lossy().starts_with("KiCad_")) && dir.join(format!("{name}.kicad_pcb")).is_file()
        })
        .map(|(dir, name)| {
            let info = std::fs::read_to_string(dir.join("meta/info.html")).unwrap_or_default();
            let title = info.split_once("<title>").and_then(|(_, t)| t.split_once("</title>")).map(|(t, _)| t.trim().to_string());
            Template { title: title.filter(|t| !t.is_empty()).unwrap_or_else(|| name.replace('_', " ")), dir, name }
        })
        .collect();
    all.sort_by(|a, b| a.title.cmp(&b.title));
    all.dedup_by(|a, b| a.name == b.name);
    all
}

/// KiCad's demo projects (their `.kicad_pro`), by name: in its `demos` folders, two deep.
pub fn examples() -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> =
        projects(&kicad_dirs("demos", "_DEMOS_DIR"), 2).into_iter().map(|(dir, name)| dir.join(format!("{name}.kicad_pro"))).collect();
    all.sort_by_key(|p| p.file_stem().map(|s| s.to_ascii_lowercase()));
    all.dedup_by(|a, b| a.file_name() == b.file_name());
    all
}

/// The folders below `roots`, down to `depth`, that hold a `.kicad_pro`, with its name.
fn projects(roots: &[PathBuf], depth: usize) -> Vec<(PathBuf, String)> {
    let mut found = vec![];
    let mut dirs: Vec<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    while let Some((dir, d)) = dirs.pop() {
        for path in std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()) {
            if path.extension().is_some_and(|e| e == "kicad_pro") && d > 0 {
                found.push((dir.clone(), path.file_stem().unwrap_or_default().to_string_lossy().into_owned()));
            } else if path.is_dir() && d < depth && path.file_name().is_some_and(|n| n != "meta") {
                dirs.push((path, d + 1));
            }
        }
    }
    found
}

/// Copies template `t` into the new folder `dir` as project `name`: every file but its
/// `meta/`, those of the template's name renamed, and the texts that name the project or its root
/// sheet following. Fails, writing nothing, when `dir` exists and isn't empty.
pub fn copy_template(t: &Template, dir: &Path, name: &str) -> Result<PathBuf, String> {
    check(dir, name)?;
    let mut stack = vec![t.dir.clone()];
    while let Some(from) = stack.pop() {
        let to = dir.join(from.strip_prefix(&t.dir).map_err(|e| e.to_string())?);
        std::fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
        for entry in std::fs::read_dir(&from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "meta") {
                    stack.push(path);
                }
                continue;
            }
            let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let renamed = match file.strip_prefix(&t.name).filter(|rest| rest.starts_with('.')) {
                Some(rest) => format!("{name}{rest}"),
                None => file,
            };
            let out = to.join(&renamed);
            let kicad = ["kicad_pro", "kicad_sch", "kicad_pcb"].iter().any(|e| renamed.ends_with(&format!(".{e}")));
            let done = match std::fs::read_to_string(&path) {
                Ok(text) if kicad => std::fs::write(&out, rename(&text, &t.name, name)),
                _ => std::fs::copy(&path, &out).map(|_| ()),
            };
            done.map_err(|e| format!("{}: {e}", out.display()))?;
        }
    }
    Ok(dir.join(format!("{name}.kicad_pcb")))
}

/// A template's text with its project `old` named `new`: its root sheet's file, its sheets'
/// project instances and its project file's name.
fn rename(text: &str, old: &str, new: &str) -> String {
    text.replace(&format!("\"{old}.kicad_sch\""), &format!("\"{new}.kicad_sch\""))
        .replace(&format!("(project \"{old}\""), &format!("(project \"{new}\""))
        .replace(&format!("\"{old}.kicad_pro\""), &format!("\"{new}.kicad_pro\""))
}

/// Writes the files of a new project `name` (`files`) into the new folder `dir`, and returns its
/// board. Fails, writing nothing, when `dir` exists and isn't empty, or a board can't have
/// `copper` layers.
pub fn write(dir: &Path, name: &str, copper: u8, size: (f64, f64), seed: u64) -> Result<PathBuf, String> {
    check(dir, name)?;
    if !LAYERS.contains(&copper) || !copper.is_multiple_of(2) {
        return Err(trf("A board has an even number of copper layers, 2 to 32: not {0}", &[&copper]));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (file, text) in files(name, copper, size, seed) {
        std::fs::write(dir.join(&file), text).map_err(|e| format!("{}: {e}", dir.join(&file).display()))?;
    }
    Ok(dir.join(format!("{name}.kicad_pcb")))
}

/// A project's name makes its file names, and it never writes over a user's files: a new
/// project needs a new or empty folder.
fn check(dir: &Path, name: &str) -> Result<(), String> {
    let bad = |c: char| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control();
    if name.trim().is_empty() || name.trim() != name || name.starts_with('.') || name.contains(bad) {
        return Err(trf("\"{0}\" can't name files: use letters, digits, spaces, - or _", &[&name]));
    }
    if std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some()) {
        return Err(trf("{0} already holds files: pick a new folder, or another name", &[&dir.display()]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::schematic::Project;

    #[test]
    fn writes_a_project_cuprita_and_kicad_read() {
        for (copper, inner) in [(2, 0), (4, 2), (6, 4), (8, 6), (32, 30)] {
            let [(pro_name, pro), (sch_name, sch), (pcb_name, pcb)] = files("Blinky \"v1\"", copper, (50.0, 30.0), 7);
            assert_eq!(
                [pro_name.as_str(), sch_name.as_str(), pcb_name.as_str()],
                ["Blinky \"v1\".kicad_pro", "Blinky \"v1\".kicad_sch", "Blinky \"v1\".kicad_pcb"]
            );
            let board = Board::parse(&pcb).unwrap();
            assert_eq!(board.copper.count_ones(), 2 + inner);
            let body = board.body();
            let xs = body.iter().flatten().map(|p| p.x);
            let ys = body.iter().flatten().map(|p| p.y);
            let span = |v: Vec<i32>| v.iter().max().unwrap() - v.iter().min().unwrap();
            assert_eq!((span(xs.collect()), span(ys.collect())), (50_000_000, 30_000_000));
            assert!(crate::json::Val::parse(&pro).is_some());
            let read = |f: &str| if f == sch_name { Ok(sch.clone()) } else { Err(format!("no {f}")) };
            let project = Project::load(&sch_name, &pro, read).unwrap();
            assert_eq!(project.pages.len(), 1);
        }
    }

    #[test]
    fn copies_a_template_under_its_new_name() {
        let tmp = std::env::temp_dir().join(format!("cuprita-template-{}", std::process::id()));
        let (from, to) = (tmp.join("Shield"), tmp.join("weather"));
        std::fs::create_dir_all(from.join("meta")).unwrap();
        std::fs::create_dir_all(from.join("Holes.pretty")).unwrap();
        std::fs::write(from.join("meta/info.html"), "<title>Shield</title>").unwrap();
        std::fs::write(from.join("Shield.kicad_pro"), r#"{"meta": {"filename": "Shield.kicad_pro"}}"#).unwrap();
        std::fs::write(from.join("Shield.kicad_sch"), r#"(kicad_sch (instances (project "Shield" (path "/x"))))"#).unwrap();
        std::fs::write(from.join("Shield.kicad_pcb"), r#"(kicad_pcb (property "Sheetfile" "Shield.kicad_sch"))"#).unwrap();
        std::fs::write(from.join("Holes.pretty/M3.kicad_mod"), "(footprint \"M3\")").unwrap();
        let t = Template { dir: from, name: "Shield".into(), title: "Shield".into() };
        assert_eq!(copy_template(&t, &to, "weather"), Ok(to.join("weather.kicad_pcb")));
        let read = |f: &str| std::fs::read_to_string(to.join(f)).unwrap();
        assert_eq!(read("weather.kicad_pro"), r#"{"meta": {"filename": "weather.kicad_pro"}}"#);
        assert_eq!(read("weather.kicad_sch"), r#"(kicad_sch (instances (project "weather" (path "/x"))))"#);
        assert_eq!(read("weather.kicad_pcb"), r#"(kicad_pcb (property "Sheetfile" "weather.kicad_sch"))"#);
        assert_eq!(read("Holes.pretty/M3.kicad_mod"), "(footprint \"M3\")");
        assert!(!to.join("meta").exists());
        // Never over a folder holding files.
        assert!(copy_template(&t, &to, "weather").unwrap_err().contains("already holds files"));
        assert!(write(&to, "x", 2, (10.0, 10.0), 1).is_err());
        // Nor with a name that isn't a file's.
        for bad in ["", " a", "a/b", "..", "a:b"] {
            assert!(write(&tmp.join("bad"), bad, 2, (10.0, 10.0), 1).unwrap_err().contains("can't name files"), "{bad:?}");
        }
        // Nor a board of layers KiCad's can't have: it says so, where it once made two.
        for copper in [0, 5, 34] {
            let e = write(&tmp.join("layers"), "a", copper, (10.0, 10.0), 1).unwrap_err();
            assert!(e.contains("2 to 32") && !tmp.join("layers").exists(), "{copper}: {e}");
        }
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
