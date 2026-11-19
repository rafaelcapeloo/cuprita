//! A project's own part libraries: those its `sym-lib-table` and `fp-lib-table` list, as KiCad
//! reads a project's tables, and parts put into them from the files a vendor or a site gives
//! (KiCad's own formats, or a zip of them), so Cuprita and KiCad both find them.

use crate::library::{self, LibPart};
use crate::sexpr::{Cursor, escape, unescape};
use crate::tr::{tr, trf};
use std::path::{Path, PathBuf};

/// The folder, in the project's, that parts are put into.
const FOLDER: &str = "libraries";

/// The libraries the table `file` (`sym-lib-table` or `fp-lib-table`) of project folder `dir`
/// lists, each one's name and where it is (`${KIPRJMOD}` the folder); KiCad's own (other
/// variables) are left out, as `library` finds them.
pub fn table(dir: &Path, file: &str) -> Vec<(String, PathBuf)> {
    let Ok(text) = std::fs::read_to_string(dir.join(file)) else { return vec![] };
    let mut libs = vec![];
    let mut read = || -> crate::sexpr::Result<()> {
        let c = &mut Cursor::new(&text);
        c.open(if file.starts_with("sym") { "sym_lib_table" } else { "fp_lib_table" })?;
        while let Some(head) = c.child()? {
            if head == "lib" {
                let (mut name, mut uri) = (String::new(), String::new());
                while let Some(head) = c.child()? {
                    match head {
                        "name" => name = unescape(c.atom()?),
                        "uri" => uri = unescape(c.atom()?),
                        _ => {}
                    }
                    c.close()?;
                }
                let path = uri.replace("${KIPRJMOD}", &dir.to_string_lossy());
                if !path.contains("${") {
                    libs.push((name, PathBuf::from(path)));
                }
            }
            c.close()?;
        }
        Ok(())
    };
    // A broken table lists what comes before the break.
    let _ = read();
    libs
}

/// The footprints of the project's own libraries in folder `dir`, as `library::footprints` lists
/// KiCad's.
pub fn footprints(dir: &Path) -> Vec<LibPart> {
    library::footprints_of(table(dir, "fp-lib-table"))
}

/// The symbols of the project's own libraries in folder `dir`, as `library::symbols` lists KiCad's.
pub fn symbols(dir: &Path) -> Vec<LibPart> {
    library::symbols_of(table(dir, "sym-lib-table"))
}

/// What `import` put into a project's libraries: symbols and footprints as `lib:name`, models by
/// the path a footprint names them by (`${KIPRJMOD}/…`), and the files it left out (not KiCad's
/// formats).
#[derive(Debug, Default, PartialEq)]
pub struct Imported {
    pub symbols: Vec<String>,
    pub footprints: Vec<String>,
    pub models: Vec<String>,
    pub skipped: Vec<String>,
}

/// Puts `files` (by name: `.kicad_sym` symbol libraries, `.kicad_mod` footprints, `.step`,
/// `.stp`, `.wrl` 3D models, and zips of them) into the own library of project `name` in folder
/// `dir`, under `libraries/`: the symbols into `<name>.kicad_sym`, the footprints into
/// `<name>.pretty`, the models into `<name>.3dshapes`, listed in the project's tables (each file
/// kept as `-bak` before it changes). A footprint's model, and a symbol's footprint, point at what
/// came with them. A part that is there already stays; another of its name goes in as `_2`.
pub fn import(dir: &Path, name: &str, files: Vec<(String, Vec<u8>)>) -> Result<Imported, String> {
    let mut all = vec![];
    for (file, bytes) in files {
        if ext(&file) == "zip" {
            all.extend(crate::zip::read(&bytes).map_err(|e| format!("{file}: {e}"))?);
        } else {
            all.push((file, bytes));
        }
    }
    let base = |f: &str| Path::new(f).file_name().map_or(f.to_string(), |n| n.to_string_lossy().into_owned());
    let of = |kinds: &[&str]| all.iter().filter(|(f, _)| kinds.contains(&ext(f).as_str())).map(|(f, b)| (base(f), b)).collect::<Vec<_>>();
    let (symbols, mut footprints, models) = (of(&["kicad_sym"]), of(&["kicad_mod"]), of(&["step", "stp", "wrl"]));
    // A vendor's zip may hold a footprint twice, as KiCad 6 on writes it and as KiCad 5 did
    // (`module`): the newer goes in, once.
    footprints.sort_by_key(|(_, b)| String::from_utf8_lossy(b).trim_start().starts_with("(module"));
    let mut seen = std::collections::HashSet::new();
    footprints.retain(|(f, _)| seen.insert(f.clone()));
    let mut done = Imported { skipped: all.iter().map(|(f, _)| base(f)).filter(|f| !known(f)).collect(), ..Imported::default() };
    if symbols.is_empty() && footprints.is_empty() && models.is_empty() {
        return Err(tr("nothing to add: give a part's KiCad files (.kicad_sym, .kicad_mod, .step) or a zip of them").into());
    }
    // A symbol library KiCad couldn't read stops it before anything goes in.
    for (file, bytes) in &symbols {
        let text = String::from_utf8_lossy(bytes);
        library::listed(&text)
            .map_err(|e| trf("{0} isn't a symbol library KiCad reads ({1}); download it again", &[file, &at(&text, e)]))?;
    }
    let folder = dir.join(FOLDER);
    let shapes = folder.join(format!("{name}.3dshapes"));
    let mut placed_models = vec![];
    for (file, bytes) in &models {
        let put = put(&shapes, file, |_| bytes.to_vec())?;
        let path = format!("${{KIPRJMOD}}/{FOLDER}/{name}.3dshapes/{put}");
        placed_models.push((file.to_lowercase(), path.clone()));
        done.models.push(path);
    }
    let mut placed_footprints = vec![];
    if !footprints.is_empty() {
        let lib = list(dir, "fp-lib-table", name, &format!("${{KIPRJMOD}}/{FOLDER}/{name}.pretty"))?;
        for (file, bytes) in &footprints {
            let text = String::from_utf8_lossy(bytes);
            let only = (footprints.len() == 1 && placed_models.len() == 1).then(|| placed_models[0].1.as_str());
            let text = remodel(&text, &placed_models, only);
            let stem = file.trim_end_matches(".kicad_mod");
            // KiCad names a footprint after its file: one put in as `_2` is named so inside too.
            let put = put(&folder.join(format!("{name}.pretty")), file, |new| rename(&text, stem, new).into_bytes())?;
            let new = put.trim_end_matches(".kicad_mod").to_string();
            placed_footprints.push((stem.to_string(), format!("{lib}:{new}")));
            done.footprints.push(format!("{lib}:{new}"));
        }
    }
    if !symbols.is_empty() {
        let lib = list(dir, "sym-lib-table", name, &format!("${{KIPRJMOD}}/{FOLDER}/{name}.kicad_sym"))?;
        let path = folder.join(format!("{name}.kicad_sym"));
        let had = std::fs::read_to_string(&path).ok();
        let mut text = had.clone().unwrap_or_else(|| {
            format!(
                "(kicad_symbol_lib\n\t(version 20251024)\n\t(generator \"cuprita\")\n\t(generator_version \"{}\")\n)\n",
                env!("CARGO_PKG_VERSION")
            )
        });
        library::listed(&text).map_err(|e| {
            trf(
                "{0} isn't a symbol library KiCad reads ({1}): fix or remove it, then add the part again",
                &[&path.display(), &at(&text, e)],
            )
        })?;
        for (_, bytes) in &symbols {
            let only = (placed_footprints.len() == 1).then(|| placed_footprints[0].1.as_str());
            // An empty one (an older file's `~` too) gets the lone footprint.
            let theirs = relink(&String::from_utf8_lossy(bytes), |fp| {
                let bare = fp.rsplit(':').next().unwrap_or(fp);
                let found = placed_footprints.iter().find(|(stem, _)| stem == bare).map(|(_, id)| id.as_str());
                found.or(only.filter(|_| matches!(fp, "" | "~")))
            });
            done.symbols.extend(merge(&mut text, &theirs).into_iter().map(|n| format!("{lib}:{n}")));
        }
        if had.as_ref() != Some(&text) {
            let write = |path: &Path, text: &str| std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()));
            std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            if let Some(had) = &had {
                write(&folder.join(format!("{name}.kicad_sym-bak")), had)?;
            }
            write(&path, &text)?;
        }
    }
    Ok(done)
}

/// Where in `text` error `e` is, and what it is: `12:5: unexpected end of file`.
fn at(text: &str, e: crate::sexpr::Error) -> String {
    let (line, col) = e.line_col(text);
    format!("{line}:{col}: {}", e.msg)
}

/// Puts the symbols of library text `theirs` into library text `lib` (KiCad 10's; both read
/// already), each after the one it extends and as KiCad 10 reads what `theirs`' version meant:
/// one `lib` holds already (the same but for spaces) stays as it is, another of its name goes in
/// as `<name>_2` (or 3…), its units and the symbols extending it renamed with it. Returns the
/// names they are in `lib` by.
// shortcut: a library newer than KiCad 10's goes in under KiCad 10's version; take up its version
// once a KiCad after 10 writes one.
fn merge(lib: &mut String, theirs: &str) -> Vec<String> {
    let (version, listed) = library::listed(theirs).unwrap_or_default();
    let (mut named, mut out) = (std::collections::HashMap::new(), vec![]);
    let mut left: Vec<usize> = (0..listed.len()).collect();
    while !left.is_empty() {
        // One that extends a symbol still to come waits for it (round, they come as they are).
        let ready = |&i: &usize| listed[i].2.as_ref().is_none_or(|p| named.contains_key(p) || !listed.iter().any(|s| s.0 == *p));
        let (own, span, extends) = &listed[left.remove(left.iter().position(ready).unwrap_or(0))];
        let parent = extends.as_ref().and_then(|p| named.get(p)).cloned();
        let there = library::listed(lib).unwrap_or_default().1;
        for k in 1.. {
            let name = if k == 1 { own.clone() } else { format!("{own}_{k}") };
            let new = upgraded(&theirs[span.clone()], version, &name, parent.as_deref());
            match there.iter().find(|s| s.0 == name) {
                Some((_, at, _)) if lib[at.clone()].split_whitespace().eq(new.split_whitespace()) => {}
                Some(_) => continue,
                None => {
                    let end = lib.rfind(')').unwrap_or(lib.len());
                    *lib = format!("{}\n\t{new}\n{}", lib[..end].trim_end(), &lib[end..]);
                }
            }
            out.push(name.clone());
            named.insert(own.clone(), name);
            break;
        }
    }
    out
}

/// Symbol `text` of a library of `version` as a KiCad 10 library holds it, named `name`, its
/// units (`<name>_1_1`) after it, extending `parent` when it extends a symbol. Before KiCad 10, a
/// field, pin name or pin number `~` was empty (KiCad 10 shows it); before 20210606, `~` toggled
/// an overbar (`~{…}` after).
fn upgraded(text: &str, version: u32, name: &str, parent: Option<&str>) -> String {
    let fix = |s: String, number: bool| match s.as_str() {
        "~" if version < 20251024 => String::new(),
        _ if version < 20210606 && !number => overbar(&s),
        _ => s,
    };
    // Each value that changes: where it is, and what it becomes.
    let mut edits = vec![];
    let mut read = || -> crate::sexpr::Result<()> {
        let c = &mut Cursor::new(text);
        let mut edit = |c: &Cursor, was: &str, new: String| {
            if unescape(was) != new {
                edits.push((c.span(), new));
            }
        };
        c.open("symbol")?;
        let old = c.atom()?;
        edit(c, old, name.to_string());
        while let Some(head) = c.child()? {
            match (head, parent) {
                ("property", _) => {
                    c.atom()?;
                    let value = c.atom()?;
                    edit(c, value, fix(unescape(value), false));
                }
                ("symbol", _) => {
                    let unit = c.atom()?;
                    if let Some(rest) = unescape(unit).strip_prefix(&unescape(old)) {
                        edit(c, unit, format!("{name}{rest}"));
                    }
                    while let Some(head) = c.child()? {
                        match head {
                            "text" => {
                                let t = c.atom()?;
                                edit(c, t, fix(unescape(t), false));
                            }
                            "pin" => {
                                while let Some(head) = c.child()? {
                                    if matches!(head, "name" | "number") {
                                        let value = c.atom()?;
                                        edit(c, value, fix(unescape(value), head == "number"));
                                    }
                                    c.close()?;
                                }
                            }
                            _ => {}
                        }
                        c.close()?;
                    }
                }
                ("extends", Some(parent)) => {
                    let was = c.atom()?;
                    edit(c, was, parent.to_string());
                }
                _ => {}
            }
            c.close()?;
        }
        Ok(())
    };
    // `merge` reads it whole first.
    let _ = read();
    let mut out = text.to_string();
    for (span, new) in edits.into_iter().rev() {
        out.replace_range(span, &format!("\"{}\"", escape(&new)));
    }
    out
}

/// Text in the overbar marks before KiCad 6 (`~RESET`, `~CS~/W`, `~~` a tilde) in KiCad 6's
/// (`~{RESET}`, `~{CS}/W`), as KiCad reads a library older than 20210606.
fn overbar(s: &str) -> String {
    let (mut out, mut open, mut chars) = (String::new(), false, s.chars().peekable());
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('~', Some('~')) => _ = chars.next(),
            ('~', Some('{')) => {}
            ('~', _) => {
                out += if open { "}" } else { "~{" };
                open = !open;
                continue;
            }
            _ => {}
        }
        out.push(c);
    }
    if open {
        out.push('}');
    }
    out
}

/// A file's extension, in small letters.
fn ext(file: &str) -> String {
    Path::new(file).extension().map_or(String::new(), |e| e.to_string_lossy().to_lowercase())
}

/// Whether `import` takes file `f` (by its kind).
fn known(f: &str) -> bool {
    ["kicad_sym", "kicad_mod", "step", "stp", "wrl", "zip"].contains(&ext(f).as_str())
}

/// Writes `file` in folder `dir` (made when missing), its bytes as `bytes` makes them for its
/// stem, unless it holds them already; another file of that name stays, and this one goes in as
/// `<stem>_2` (or 3…). Returns the name used.
fn put(dir: &Path, file: &str, bytes: impl Fn(&str) -> Vec<u8>) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (stem, dot) = file.rsplit_once('.').map_or((file, String::new()), |(s, e)| (s, format!(".{e}")));
    for k in 1.. {
        let new = if k == 1 { stem.to_string() } else { format!("{stem}_{k}") };
        let (path, data) = (dir.join(format!("{new}{dot}")), bytes(&new));
        match std::fs::read(&path) {
            Ok(there) if there == data => return Ok(format!("{new}{dot}")),
            Ok(_) => continue,
            Err(_) => return std::fs::write(&path, data).map(|()| format!("{new}{dot}")).map_err(|e| format!("{}: {e}", path.display())),
        }
    }
    unreachable!("a free name is found")
}

/// Lists library `lib` at `uri` in table `file` of project folder `dir`, made as KiCad 10 writes
/// one when there is none, else kept as `<file>-bak` first. A library already listed at `uri`
/// keeps its name; another of the name `lib` makes it `lib_2`. Returns the name it is listed by.
fn list(dir: &Path, file: &str, lib: &str, uri: &str) -> Result<String, String> {
    let path = dir.join(file);
    let resolved = PathBuf::from(uri.replace("${KIPRJMOD}", &dir.to_string_lossy()));
    let listed = table(dir, file);
    if let Some((name, _)) = listed.iter().find(|(_, p)| *p == resolved) {
        return Ok(name.clone());
    }
    let name = (1..).map(|k| if k == 1 { lib.to_string() } else { format!("{lib}_{k}") }).find(|n| !listed.iter().any(|(l, _)| l == n));
    let name = name.expect("a free name is found");
    let entry = format!("\t(lib (name \"{}\") (type \"KiCad\") (uri \"{}\") (options \"\") (descr \"\"))\n", escape(&name), escape(uri));
    let write = |text: String| std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()));
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let end = text
                .rfind(')')
                .ok_or_else(|| trf("{0} isn't a library table: fix or remove it, then add the part again", &[&path.display()]))?;
            std::fs::write(dir.join(format!("{file}-bak")), &text).map_err(|e| format!("{file}-bak: {e}"))?;
            let head = text[..end].trim_end();
            write(format!("{head}\n{entry}{}", &text[end..]))?;
        }
        Err(_) => write(format!("({}\n\t(version 7)\n{entry})\n", file.replace('-', "_")))?,
    }
    Ok(name)
}

/// Footprint `text` with each model that came along (`models`: file in small letters, and the
/// path it went in at) pointed at; with none of its own, `only`, the one model that came with it.
fn remodel(text: &str, models: &[(String, String)], only: Option<&str>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut any = false;
    while let Some(at) = rest.find("(model \"") {
        any = true;
        let start = at + "(model \"".len();
        let Some(len) = quoted(&rest[start..]) else { break };
        let old = unescape(&rest[start..start + len]);
        let file = old.rsplit(['/', '\\']).next().unwrap_or(&old).to_lowercase();
        let new = models.iter().find(|(f, _)| *f == file).map_or(old.clone(), |(_, p)| p.clone());
        out += &rest[..start];
        out += &escape(&new);
        rest = &rest[start + len..];
    }
    out += rest;
    match (any, only, out.rfind(')')) {
        (false, Some(model), Some(end)) => {
            let block = format!(
                "\t(model \"{}\"\n\t\t(offset\n\t\t\t(xyz 0 0 0)\n\t\t)\n\t\t(scale\n\t\t\t(xyz 1 1 1)\n\t\t)\n\t\t(rotate\n\t\t\t(xyz 0 0 0)\n\t\t)\n\t)\n",
                escape(model)
            );
            format!("{}\n{block}{}", out[..end].trim_end(), &out[end..])
        }
        _ => out,
    }
}

/// Footprint `text` named `new` where it named itself `old` (KiCad 6 on: `footprint`, KiCad 5:
/// `module`), as KiCad names a footprint after its file.
fn rename(text: &str, old: &str, new: &str) -> String {
    let mut out = text.to_string();
    if new == old {
        return out;
    }
    for head in ["(footprint \"", "(module \""] {
        if let Some(at) = out.find(&format!("{head}{}\"", escape(old))) {
            out.replace_range(at + head.len()..at + head.len() + escape(old).len(), &escape(new));
            return out;
        }
    }
    for head in ["(footprint ", "(module "] {
        if let Some(at) = out.find(&format!("{head}{old} ")) {
            out.replace_range(at + head.len()..at + head.len() + old.len(), &format!("\"{}\"", escape(new)));
            return out;
        }
    }
    out
}

/// Symbol library `text` with each symbol's `Footprint` set to what `link` gives for it (given
/// what it holds); one `link` gives nothing for stays.
fn relink<'a>(text: &str, link: impl Fn(&str) -> Option<&'a str>) -> String {
    const HEAD: &str = "(property \"Footprint\" \"";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(HEAD) {
        let start = at + HEAD.len();
        let Some(len) = quoted(&rest[start..]) else { break };
        let old = unescape(&rest[start..start + len]);
        out += &rest[..start];
        out += &link(&old).map_or_else(|| rest[start..start + len].to_string(), escape);
        rest = &rest[start + len..];
    }
    out + rest
}

/// How long the quoted text `s` starts with is, up to its closing quote (escapes skipped).
fn quoted(s: &str) -> Option<usize> {
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => _ = chars.next(),
            '"' => return Some(i),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYM: &str = "(kicad_symbol_lib (version 20241209) (generator \"vendor\")\n  (symbol \"LM358\" (property \"Reference\" \"U\") (property \"Footprint\" \"SnapEDA:SOIC127P600X175-8N\" (at 0 0 0)) (property \"Description\" \"Dual op-amp\"))\n  (symbol \"LM358B\" (extends \"LM358\") (property \"Footprint\" \"\"))\n)\n";
    const FP: &str = "(footprint \"SOIC127P600X175-8N\" (version 20240108) (layer \"F.Cu\")\n  (pad \"1\" smd rect (at -2.5 -1.9) (size 1.5 0.6) (layers \"F.Cu\"))\n  (model \"${KIPRJMOD}/LM358DR.STEP\" (offset (xyz 0 0 0)))\n)\n";

    /// A fresh project folder for a test.
    fn project(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cuprita-parts-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn imports_a_vendors_part_into_the_projects_libraries_kicad_reads() {
        let dir = project("vendor");
        // The project already lists a library of its own, which stays.
        std::fs::write(dir.join("fp-lib-table"), "(fp_lib_table\n\t(version 7)\n\t(lib (name \"Mine\") (type \"KiCad\") (uri \"${KIPRJMOD}/mine.pretty\") (options \"\") (descr \"\"))\n)\n").unwrap();
        let files = vec![
            ("LM358/KiCad/LM358.kicad_sym".to_string(), SYM.as_bytes().to_vec()),
            ("LM358/KiCad/SOIC127P600X175-8N.kicad_mod".into(), FP.as_bytes().to_vec()),
            ("LM358/3D/LM358DR.STEP".into(), b"ISO-10303-21;".to_vec()),
            ("LM358/README.txt".into(), b"hi".to_vec()),
        ];
        let done = import(&dir, "glow", files.clone()).unwrap();
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(done.symbols, ids(&["glow:LM358", "glow:LM358B"]));
        assert_eq!(
            (done.footprints, done.models, done.skipped),
            (ids(&["glow:SOIC127P600X175-8N"]), ids(&["${KIPRJMOD}/libraries/glow.3dshapes/LM358DR.STEP"]), ids(&["README.txt"]))
        );
        // The symbols go into the project's own library, made as KiCad 10 writes one.
        let lib = std::fs::read_to_string(dir.join("libraries/glow.kicad_sym")).unwrap();
        assert!(lib.starts_with("(kicad_symbol_lib\n\t(version 20251024)\n\t(generator \"cuprita\")\n"), "{lib}");
        // The symbol points at the footprint as the project lists it; an empty one at the only one.
        assert_eq!(lib.matches("(property \"Footprint\" \"glow:SOIC127P600X175-8N\"").count(), 2);
        let fp = std::fs::read_to_string(dir.join("libraries/glow.pretty/SOIC127P600X175-8N.kicad_mod")).unwrap();
        assert!(fp.contains("(model \"${KIPRJMOD}/libraries/glow.3dshapes/LM358DR.STEP\""));
        // Both tables list the libraries, the one there before kept (and backed up).
        let names = |file| table(&dir, file).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!((names("fp-lib-table"), names("sym-lib-table")), (ids(&["Mine", "glow"]), ids(&["glow"])));
        assert!(dir.join("fp-lib-table-bak").exists() && !dir.join("sym-lib-table-bak").exists());
        assert_eq!(symbols(&dir).iter().map(LibPart::id).collect::<Vec<_>>(), ids(&["glow:LM358", "glow:LM358B"]));
        assert_eq!(footprints(&dir)[0].id(), "glow:SOIC127P600X175-8N");
        // The same part again changes nothing.
        assert_eq!(import(&dir, "glow", files.clone()).unwrap().symbols, ids(&["glow:LM358", "glow:LM358B"]));
        assert!(!dir.join("libraries/glow.kicad_sym-bak").exists());
        // The same files again change nothing; another part of the same names goes in beside it.
        assert_eq!(import(&dir, "glow", files).unwrap().footprints, ids(&["glow:SOIC127P600X175-8N"]));
        let other = vec![("SOIC127P600X175-8N.kicad_mod".to_string(), FP.replace("-2.5", "-2.6").into_bytes())];
        assert_eq!(import(&dir, "glow", other.clone()).unwrap().footprints, ids(&["glow:SOIC127P600X175-8N_2"]));
        assert_eq!(import(&dir, "glow", other).unwrap().footprints, ids(&["glow:SOIC127P600X175-8N_2"]));
        let renamed = std::fs::read_to_string(dir.join("libraries/glow.pretty/SOIC127P600X175-8N_2.kicad_mod")).unwrap();
        assert!(renamed.starts_with("(footprint \"SOIC127P600X175-8N_2\""));
        assert_eq!(table(&dir, "fp-lib-table").len(), 2);
        let none = import(&dir, "glow", vec![("notes.pdf".into(), vec![])]).unwrap_err();
        assert_eq!(none, "nothing to add: give a part's KiCad files (.kicad_sym, .kicad_mod, .step) or a zip of them");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn merges_symbols_into_the_projects_library_renaming_another_of_a_name() {
        let dir = project("merge");
        // A library the project had stays as it was, listed.
        std::fs::write(dir.join("sym-lib-table"), "(sym_lib_table\n\t(version 7)\n\t(lib (name \"old\") (type \"KiCad\") (uri \"${KIPRJMOD}/old.kicad_sym\") (options \"\") (descr \"\"))\n)\n").unwrap();
        let unit = |name: &str, pin: &str| {
            format!(
                "(symbol \"{name}\" (property \"Reference\" \"U\") (symbol \"{name}_1_1\" (pin input line (at 0 0 0) (length 2.54) (name \"{pin}\") (number \"1\"))))"
            )
        };
        let library = |symbols: &[String]| format!("(kicad_symbol_lib (version 20211014)\n  {}\n)\n", symbols.join("\n  "));
        let first = library(&[unit("OPA", "IN")]);
        assert_eq!(import(&dir, "p", vec![("a.kicad_sym".into(), first.into_bytes())]).unwrap().symbols, ["p:OPA"]);
        // Another OPA, with a symbol extending it written before it, as KiCad orders them.
        let derived = "(symbol \"OPA2\" (extends \"OPA\") (property \"Value\" \"OPA2\"))".to_string();
        let second = library(&[derived, unit("OPA", "INP")]);
        let done = import(&dir, "p", vec![("b.kicad_sym".into(), second.clone().into_bytes())]).unwrap();
        assert_eq!(done.symbols, ["p:OPA_2", "p:OPA2"]);
        let lib = std::fs::read_to_string(dir.join("libraries/p.kicad_sym")).unwrap();
        for held in ["(symbol \"OPA\" ", "(symbol \"OPA_1_1\"", "(symbol \"OPA_2\" ", "(symbol \"OPA_2_1_1\"", "(extends \"OPA_2\")"] {
            assert!(lib.contains(held), "{held} in {lib}");
        }
        assert!(std::fs::read_to_string(dir.join("libraries/p.kicad_sym-bak")).unwrap().contains("(symbol \"OPA\" "));
        // Cuprita reads them as KiCad does: OPA2 is OPA_2's units under its own name.
        let opa2 = library::symbol(&lib, "OPA2").unwrap();
        assert!(opa2.contains("(symbol \"OPA2_1_1\"") && opa2.contains("(name \"INP\")"), "{opa2}");
        // Given again, all of it is there already.
        assert_eq!(import(&dir, "p", vec![("b.kicad_sym".into(), second.into_bytes())]).unwrap().symbols, ["p:OPA_2", "p:OPA2"]);
        assert_eq!(std::fs::read_to_string(dir.join("libraries/p.kicad_sym")).unwrap(), lib);
        let names = table(&dir, "sym-lib-table").into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names, ["old", "p"]);
        // Only what came in is listed: no footprint library for symbols alone.
        assert!(!dir.join("fp-lib-table").exists());
        let broken = import(&dir, "p", vec![("c.kicad_sym".into(), b"(kicad_symbol_lib (symbol \"X\"".to_vec())]).unwrap_err();
        assert_eq!(broken, "c.kicad_sym isn't a symbol library KiCad reads (1:30: unexpected end of file); download it again");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_an_older_librarys_marks_as_kicad_did() {
        // As KiCad 10 upgrades a library of 20201005: `~` is empty, and toggled overbars.
        let old = ["~A", "~A~B", "A~B~C", "~~", "A~~B", "~A~~B~", "~{A}", "A~", "x~y~z~w", "~A/B~C"];
        let new = ["~{A}", "~{A}B", "A~{B}C", "~", "A~B", "~{A~B}", "~{A}", "A~{}", "x~{y}z~{w}", "~{A/B}C"];
        assert_eq!(old.map(overbar), new);
        let pin = |name: &str, number: &str| format!("(pin input line (at 0 0 0) (name \"{name}\") (number \"{number}\"))");
        let symbol = |value: &str, pins: [String; 2]| {
            format!("(symbol \"P\" (property \"Value\" \"{value}\") (symbol \"P_1_1\" (text \"~B~\" (at 0 0 0)) {}))", pins.join(" "))
        };
        let text = symbol("~", [pin("~", "1"), pin("~RST", "~")]);
        assert_eq!(upgraded(&text, 20201005, "P", None), symbol("", [pin("", "1"), pin("~{RST}", "")]).replace("~B~", "~{B}"));
        assert_eq!(upgraded(&text, 20241209, "P", None), symbol("", [pin("", "1"), pin("~RST", "")]));
        // KiCad 10's `~` is a tilde.
        assert_eq!(upgraded(&text, 20251024, "Q", None), text.replace("\"P", "\"Q"));
    }

    #[test]
    fn takes_the_newer_of_a_footprint_given_twice() {
        let dir = project("twice");
        let old = "(module SOIC-8 (layer F.Cu)\n  (pad 1 smd rect (at -2.5 -1.9) (size 1.5 0.6) (layers F.Cu))\n)\n";
        let new = "(footprint \"SOIC-8\" (version 20240108) (layer \"F.Cu\")\n)\n";
        let files = vec![
            ("KiCad/SOIC-8.kicad_mod".to_string(), old.as_bytes().to_vec()),
            ("KiCADv6/SOIC-8.kicad_mod".into(), new.as_bytes().to_vec()),
        ];
        assert_eq!(import(&dir, "p", files).unwrap().footprints, ["p:SOIC-8"]);
        assert_eq!(std::fs::read_to_string(dir.join("libraries/p.pretty/SOIC-8.kicad_mod")).unwrap(), new);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gives_a_lone_footprint_the_model_that_came_with_it() {
        let bare = "(footprint \"X\" (layer \"F.Cu\")\n)\n";
        let text = remodel(
            bare,
            &[("x.step".into(), "${KIPRJMOD}/libraries/p.3dshapes/X.step".into())],
            Some("${KIPRJMOD}/libraries/p.3dshapes/X.step"),
        );
        assert!(
            text.contains("(layer \"F.Cu\")\n\t(model \"${KIPRJMOD}/libraries/p.3dshapes/X.step\"\n") && text.ends_with("\t)\n)\n"),
            "{text}"
        );
        assert_eq!(rename("(module SOIC-8 (layer F.Cu))", "SOIC-8", "SOIC-8_2"), "(module \"SOIC-8_2\" (layer F.Cu))");
        assert_eq!(quoted("a\\\"b\" rest"), Some(4));
    }
}
