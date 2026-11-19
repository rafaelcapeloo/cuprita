//! KiCad libraries: the footprints and symbols KiCad installs, found without any setup; a symbol
//! as a sheet keeps its copy, and a footprint added to a board.

use crate::board::{Board, Footprint, Pad, Pt, Src};
use crate::parse::Parser;
use crate::save::{KICAD6, KICAD8, apply, uuid};
use crate::sexpr::{Cursor, Lexer, Result, Tok, escape, unescape};
use crate::threads::par_map;
use crate::tr::{tr, trf};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// A footprint or symbol in a KiCad library.
#[derive(Clone, Debug, PartialEq)]
pub struct LibPart {
    pub lib: String,
    pub name: String,
    /// Its file: a footprint's `.kicad_mod`, or the library's `.kicad_sym` holding a symbol.
    pub path: PathBuf,
    /// What a symbol is: its description and keywords (a footprint's are not read).
    pub about: String,
}

impl LibPart {
    /// `lib:name`, as a board or sheet links a part to its library.
    pub fn id(&self) -> String {
        format!("{}:{}", self.lib, self.name)
    }
}

/// The footprints of the libraries KiCad installs (`.pretty` folders, named after their library):
/// in the folders `KICAD*_FOOTPRINT_DIR` name, then where KiCad's packages put them.
// shortcut: libraries only the user's global `fp-lib-table` lists are left out (a project's own
// are `parts::footprints`); read it when someone keeps libraries there.
pub fn footprints() -> Vec<LibPart> {
    footprints_in(&kicad_dirs("footprints", "_FOOTPRINT_DIR"))
}

/// The symbols of the libraries KiCad installs (`.kicad_sym` files, named after their library),
/// found where `footprints` are, with what each is; the files are read in parallel.
// shortcut: as for footprints, libraries only the user's global `sym-lib-table` lists are left out.
pub fn symbols() -> Vec<LibPart> {
    symbols_in(&kicad_dirs("symbols", "_SYMBOL_DIR"))
}

/// How many footprint and symbol libraries `footprints` and `symbols` would read: a look at
/// KiCad's folders that reads none of them.
pub fn found() -> (usize, usize) {
    let count = |kind, var, ext| libraries(&kicad_dirs(kind, var), ext).len();
    (count("footprints", "_FOOTPRINT_DIR", "pretty"), count("symbols", "_SYMBOL_DIR", "kicad_sym"))
}

/// KiCad's folders of `kind` (`footprints`, `symbols`, `3dmodels`): those the `KICAD*{var}`
/// variables name, then where KiCad's packages put them (Linux, as a Flatpak sandbox sees the
/// system's under /run/host too; Flatpak, Snap, macOS, Windows, newest first), then the copy of
/// its symbols and footprints Cuprita comes with (`own_dirs`): a KiCad installed goes first.
pub(crate) fn kicad_dirs(kind: &str, var: &str) -> Vec<PathBuf> {
    // A browser has no files to look in, and reading the environment panics there.
    if cfg!(target_arch = "wasm32") {
        return vec![];
    }
    let named = std::env::vars_os().filter(|(k, _)| k.to_str().is_some_and(|k| k.starts_with("KICAD") && k.ends_with(var)));
    let mut roots: Vec<PathBuf> = named.map(|(_, dir)| dir.into()).collect();
    let (arch, home) = (std::env::consts::ARCH, PathBuf::from(std::env::var_os("HOME").unwrap_or_default()));
    let flatpak = format!("flatpak/runtime/org.kicad.kicad.Library/{arch}/stable/active/files/{kind}");
    roots.extend([
        Path::new("/usr/share/kicad").join(kind),
        Path::new("/usr/local/share/kicad").join(kind),
        Path::new("/run/host/usr/share/kicad").join(kind),
        Path::new("/run/host/usr/local/share/kicad").join(kind),
        Path::new("/var/lib").join(&flatpak),
        home.join(".local/share").join(&flatpak),
        Path::new("/snap/kicad/current/usr/share/kicad").join(kind),
        Path::new("/Applications/KiCad/KiCad.app/Contents/SharedSupport").join(kind),
    ]);
    if let Some(programs) = std::env::var_os("ProgramFiles") {
        let mut versions: Vec<PathBuf> = folders(&Path::new(&programs).join("KiCad")).collect();
        let number = |p: &PathBuf| p.file_name().and_then(|n| n.to_str()?.parse::<f64>().ok()).unwrap_or(0.0);
        versions.sort_by(|a, b| number(b).total_cmp(&number(a)));
        roots.extend(versions.into_iter().map(|v| v.join("share/kicad").join(kind)));
    }
    if let Some(bin) = std::env::current_exe().ok().as_deref().and_then(Path::parent) {
        roots.extend(own_dirs(bin, kind));
    }
    roots
}

/// Where KiCad's libraries of `kind` that Cuprita comes with (`packaging/libraries.sh`) lie
/// beside the app in `bin`: a package's or `install.sh`'s `<prefix>/share/cuprita/kicad`, a Mac
/// app's `Resources`, beside Windows' `.exe`, and `target/libraries` for a build run from the source.
fn own_dirs(bin: &Path, kind: &str) -> [PathBuf; 4] {
    ["../share/cuprita/kicad", "../Resources/kicad", "kicad", "../libraries"].map(|own| bin.join(own).join(kind))
}

fn folders(dir: &Path) -> impl Iterator<Item = PathBuf> + use<> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path())
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The libraries in `roots`: what has extension `ext` (`.pretty` folders, `.kicad_sym` files),
/// named after it; a library found again further on is left out.
fn libraries(roots: &[PathBuf], ext: &str) -> Vec<(String, PathBuf)> {
    let mut libs: Vec<(String, PathBuf)> = vec![];
    for path in roots.iter().flat_map(|r| folders(r)).filter(|p| p.extension().is_some_and(|e| e == ext)) {
        let lib = stem(&path);
        if !libs.iter().any(|(l, _)| *l == lib) {
            libs.push((lib, path));
        }
    }
    libs
}

/// The footprints of the `.pretty` libraries in `roots`, by library and name.
fn footprints_in(roots: &[PathBuf]) -> Vec<LibPart> {
    footprints_of(libraries(roots, "pretty"))
}

/// The footprints of `.pretty` libraries `libs` (each its name and folder), by library and name.
pub(crate) fn footprints_of(libs: Vec<(String, PathBuf)>) -> Vec<LibPart> {
    let mut all: Vec<LibPart> = libs
        .into_iter()
        .flat_map(|(lib, dir)| {
            let files = folders(&dir).filter(|f| f.extension().is_some_and(|e| e == "kicad_mod"));
            files.map(move |path| LibPart { lib: lib.clone(), name: stem(&path), path, about: String::new() })
        })
        .collect();
    all.sort_by(|a, b| (&a.lib, &a.name).cmp(&(&b.lib, &b.name)));
    all
}

/// The symbols of the `.kicad_sym` libraries in `roots`, by library and name.
fn symbols_in(roots: &[PathBuf]) -> Vec<LibPart> {
    symbols_of(libraries(roots, "kicad_sym"))
}

/// The symbols of `.kicad_sym` libraries `libs` (each its name and file), by library and name.
pub(crate) fn symbols_of(libs: Vec<(String, PathBuf)>) -> Vec<LibPart> {
    let read = par_map(&libs, |(lib, path)| {
        // A library that can't be read lists nothing, a broken one what comes before the break.
        let names = names(&std::fs::read_to_string(path).unwrap_or_default());
        names.into_iter().map(|(name, about)| LibPart { lib: lib.clone(), name, path: path.clone(), about }).collect::<Vec<_>>()
    });
    let mut all: Vec<LibPart> = read.into_iter().flatten().collect();
    all.sort_by(|a, b| (&a.lib, &a.name).cmp(&(&b.lib, &b.name)));
    all
}

/// The symbols a `.kicad_sym` library's text holds, up to an error in it: each one's name and
/// what it is (its description and keywords).
fn names(text: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut read = || -> Result<()> {
        let c = &mut Cursor::new(text);
        c.open("kicad_symbol_lib")?;
        while let Some(head) = c.child()? {
            if head == "symbol" {
                let name = unescape(c.atom()?);
                let mut about = vec![];
                while let Some(head) = c.child()? {
                    // KiCad 6 and 7 call the description `ki_description`.
                    if head == "property" && matches!(c.atom()?, "Description" | "ki_description" | "ki_keywords") {
                        about.push(unescape(c.atom()?));
                    }
                    c.close()?;
                }
                out.push((name, about.join(" ")));
            }
            c.close()?;
        }
        Ok(())
    };
    let _ = read();
    out
}

/// The parts whose `lib:name`, or what they are, hold every word of `query` (any case), the
/// closest first: the one named or linked as the query, then those KiCad names for it as a part
/// number (`named`), then those whose names hold every word, then whose `lib:name`, then those
/// more of whose words start one (`R 0402`: `R_0402` before `C_0402_1005Metric`), then shorter
/// names.
pub fn search<'a>(all: &'a [LibPart], query: &str) -> Vec<&'a LibPart> {
    let (q, words): (&str, Vec<String>) = (query.trim(), query.split_whitespace().map(str::to_lowercase).collect());
    let holds = |s: &str| words.iter().all(|w| s.contains(w.as_str()));
    let own = if words.len() == 1 { named(all, q) } else { vec![] };
    let mut hits: Vec<((bool, bool, bool), &LibPart)> = (all.iter())
        .filter_map(|p| {
            let id = p.id().to_lowercase();
            let named = (own.iter().any(|&o| std::ptr::eq(o, p)), holds(&id[p.lib.len() + 1..]), holds(&id));
            let about = || p.about.to_lowercase();
            let found = named.0 || named.2 || words.iter().all(|w| id.contains(w.as_str()) || about().contains(w.as_str()));
            found.then_some((named, p))
        })
        .collect();
    // How many of the query's words start a word of a name: at its start, or after a sign.
    let starts = |name: &str| {
        let (name, word) = (name.to_lowercase(), |i: usize, name: &str| i == 0 || !name.as_bytes()[i - 1].is_ascii_alphanumeric());
        words.iter().filter(|w| name.match_indices(w.as_str()).any(|(i, _)| word(i, &name))).count()
    };
    hits.sort_by_cached_key(|((own, name, id), p)| {
        let exact = p.name.eq_ignore_ascii_case(q) || p.id().eq_ignore_ascii_case(q);
        (!exact, !own, !name, !id, std::cmp::Reverse(starts(&p.name)), p.name.len())
    });
    hits.into_iter().map(|(_, p)| p).collect()
}

/// The parts KiCad names for part number `mpn`: the longest names it starts with, as KiCad names
/// a part's symbol: the ordering code's package and reel letters left off (`LM358` for LM358DR),
/// `x` for any one character (`STM32F103C8Tx` for STM32F103C8T6), signs aside (`MCP2515-xSO` for
/// MCP2515-I/SO); never a number cut short (not `LM358` for LM3580) nor a name without one (`R`).
pub fn named<'a>(all: &'a [LibPart], mpn: &str) -> Vec<&'a LibPart> {
    let found: Vec<(usize, &LibPart)> = all.iter().filter_map(|p| Some((starts(mpn, &p.name)?, p))).collect();
    let most = found.iter().map(|f| f.0).max();
    found.into_iter().filter(|f| Some(f.0) == most).map(|f| f.1).collect()
}

/// How many characters of part number `mpn` the name matches when it starts it, as `named` takes it.
fn starts(mpn: &str, name: &str) -> Option<usize> {
    let (mut rest, mut n, mut last) = (mpn.chars().peekable(), 0, ' ');
    for c in name.chars().filter(char::is_ascii_alphanumeric) {
        while rest.next_if(|m| !m.is_ascii_alphanumeric()).is_some() {}
        last = rest.next().filter(|m| c == 'x' || m.eq_ignore_ascii_case(&c))?;
        n += 1;
    }
    let cut = last.is_ascii_digit() && rest.peek().is_some_and(char::is_ascii_digit);
    (name.contains(|c: char| c.is_ascii_digit()) && !cut).then_some(n)
}

/// Symbol `name` of a `.kicad_sym` library's `text` as a sheet keeps its copy: one that extends
/// another holds that one's units, pins and flags under its own name, with its own fields (and
/// those of the other's it lacks), as KiCad copies it.
pub fn symbol(text: &str, name: &str) -> std::result::Result<String, String> {
    let spans = spans(text).map_err(|e| {
        let (line, col) = e.line_col(text);
        format!("{line}:{col}: {}", e.msg)
    })?;
    flat(text, &spans, name, 0)
}

/// The value of field `name` (`Footprint`, `ki_fp_filters`) of a symbol's text as `symbol` gives it.
pub fn field(text: &str, name: &str) -> Option<String> {
    let (_, lists) = lists(text).ok()?;
    let (_, _, span) = lists.into_iter().find(|l| l.0 == "property" && l.1 == name)?;
    let c = &mut Cursor::new(&text[span]);
    c.open("property").and_then(|()| c.atom()).and_then(|_| c.atom()).ok().map(unescape)
}

/// Whether footprint `fp` fits a symbol's footprint filter (`SOIC*3.9x4.9mm*`, `LED_SMD:*`), as
/// KiCad matches them: `*` any characters, `?` any one, any case; its `lib:name` when the filter
/// names a library, else its name.
pub fn fits(filter: &str, fp: &LibPart) -> bool {
    fn glob(p: &[u8], t: &[u8]) -> bool {
        // On a miss the last `*` takes one more character: a filter of many `*` (a symbol's
        // words, from anyone) costs its length times the name's, not every way to split it.
        let (mut i, mut j, mut star) = (0, 0, None);
        while j < t.len() {
            match (p.get(i), star) {
                (Some(b'*'), _) => (i, star) = (i + 1, Some((i, j))),
                (Some(&c), _) if c == b'?' || c.eq_ignore_ascii_case(&t[j]) => (i, j) = (i + 1, j + 1),
                (_, Some((at, from))) => (i, j, star) = (at + 1, from + 1, Some((at, from + 1))),
                (_, None) => return false,
            }
        }
        p[i..].iter().all(|&c| c == b'*')
    }
    let name = if filter.contains(':') { fp.id() } else { fp.name.clone() };
    glob(filter.as_bytes(), name.as_bytes())
}

/// Where each symbol of a library's text is, by name, and the one it extends.
type Spans = HashMap<String, (Range<usize>, Option<String>)>;

/// The `Spans` of a library's text.
fn spans(text: &str) -> Result<Spans> {
    Ok(listed(text)?.1.into_iter().map(|(name, span, extends)| (name, (span, extends))).collect())
}

/// A library's text read: its version (0 when it names none), and its symbols in its order, each
/// one's name, where it is (its `(` to its `)`) and the one it extends.
pub(crate) type Listed = (u32, Vec<(String, Range<usize>, Option<String>)>);

/// The `Listed` of a library's text.
pub(crate) fn listed(text: &str) -> Result<Listed> {
    let (c, mut out, mut version) = (&mut Cursor::new(text), vec![], 0);
    c.open("kicad_symbol_lib")?;
    while let Some(head) = c.child()? {
        if head == "version" {
            version = c.atom()?.parse().unwrap_or(0);
            c.close()?;
        } else if head == "symbol" {
            // The list starts at its `(`, just before its head.
            let (start, name, mut extends) = (c.span().start - 1, unescape(c.atom()?), None);
            while let Some(head) = c.child()? {
                if head == "extends" {
                    extends = Some(unescape(c.atom()?));
                }
                c.close()?;
            }
            c.close()?;
            out.push((name, start..c.span().end, extends));
        } else {
            c.close()?;
        }
    }
    Ok((version, out))
}

/// Symbol `name`'s text, flattened onto the one it extends (`depth` steps down already).
fn flat(text: &str, spans: &Spans, name: &str, depth: u32) -> std::result::Result<String, String> {
    let (span, extends) = spans.get(name).ok_or_else(|| trf("the library has no symbol {0}", &[&name]))?;
    let own = &text[span.clone()];
    let Some(parent) = extends else { return Ok(own.to_string()) };
    if depth > 8 {
        return Err(trf("{0} extends itself, through {1}; fix the library in KiCad's symbol editor", &[&name, parent]));
    }
    if !spans.contains_key(parent) {
        return Err(trf("{0} extends {1}, which the library lacks", &[&name, parent]));
    }
    let base = flat(text, spans, parent, depth + 1)?;
    let bad = |e: crate::sexpr::Error| format!("{name}: {}", e.msg);
    let ((raw, mine), (head, theirs)) = (lists(own).map_err(bad)?, lists(&base).map_err(bad)?);
    let raw = &own[raw];
    // Each list goes before the space the base writes them after, and the base's end closes it.
    let gap = theirs.first().map_or(" ", |l| &base[head.end..l.2.start]);
    let mut out = format!("(symbol {raw}");
    let mut put = |t: &str| {
        out += gap;
        out += t;
    };
    let last_field = theirs.iter().rposition(|l| l.0 == "property");
    let new = |l: &&List| !theirs.iter().any(|b| b.0 == l.0 && b.1 == l.1);
    for (i, (kind, key, s)) in theirs.iter().enumerate() {
        let t = &base[s.clone()];
        let mut parts = key.rsplitn(3, '_');
        match (*kind, parts.next(), parts.next(), parts.next()) {
            // A unit, `NAME_unit_style`, takes the derived symbol's name.
            ("symbol", Some(style), Some(unit), Some(_)) => {
                let c = &mut Cursor::new(t);
                c.open("symbol").and_then(|()| c.atom()).map_err(bad)?;
                let (n, name) = (c.span(), raw.trim_matches('"'));
                put(&format!("{}\"{name}_{unit}_{style}\"{}", &t[..n.start], &t[n.end..]));
            }
            // Its own fields and flags replace the base's.
            _ => put(mine.iter().find(|l| l.0 == *kind && l.1 == *key).map_or(t, |l| &own[l.2.clone()])),
        }
        if Some(i) == last_field {
            mine.iter().filter(|l| l.0 == "property").filter(new).for_each(|l| put(&own[l.2.clone()]));
        }
    }
    let rest = mine.iter().filter(|l| l.0 != "extends" && (l.0 != "property" || last_field.is_none()));
    rest.filter(new).for_each(|l| put(&own[l.2.clone()]));
    Ok(out + &base[theirs.last().map_or(head.end, |l| l.2.end)..])
}

/// A list in a symbol: its head, key (a property's or unit's name) and where it is.
type List<'a> = (&'a str, String, Range<usize>);

/// A symbol's text read as where its name is, and its lists.
fn lists(t: &str) -> Result<(Range<usize>, Vec<List<'_>>)> {
    let c = &mut Cursor::new(t);
    c.open("symbol")?;
    c.atom()?;
    let (name, mut out) = (c.span(), vec![]);
    while let Some(head) = c.child()? {
        let start = c.span().start - 1;
        let key = if matches!(head, "property" | "symbol") { unescape(c.atom()?) } else { String::new() };
        c.close()?;
        out.push((head, key, start..c.span().end));
    }
    Ok((name, out))
}

impl Board {
    /// Adds the footprint of `text`, a `.kicad_mod` file of library `lib`, at `at` turned `angle`
    /// degrees, on the front, with the next free reference of its kind (`H3` for a third mounting
    /// hole, `TP`, `FID`, `LOGO`, else `REF`); returns its index. Saving writes it (`Src::Library`).
    // shortcut: it goes in as its library's KiCad version wrote it, also into boards of an older
    // version (only KiCad of the library's version or later reads them); write it as the board's
    // version when someone keeps an older KiCad.
    pub fn add_footprint(&mut self, text: &str, lib: &str, at: Pt, angle: f64) -> Result<usize> {
        let prefix = match lib {
            "MountingHole" => "H",
            "TestPoint" => "TP",
            "Fiducial" => "FID",
            "Symbol" => "LOGO",
            _ => "REF",
        };
        let n = self.footprints.iter().filter_map(|f| f.reference.strip_prefix(prefix)?.parse::<u32>().ok()).max().unwrap_or(0);
        self.add_part(text, lib, &format!("{prefix}{}", n + 1), None, at, angle)
    }

    /// Adds a footprint as `add_footprint` does, named `reference`; a schematic's part's with
    /// its value and link to its symbol (`part`: `Symbol::{value, link}`).
    pub(crate) fn add_part(
        &mut self,
        text: &str,
        lib: &str,
        reference: &str,
        part: Option<(&str, &str)>,
        at: Pt,
        angle: f64,
    ) -> Result<usize> {
        let (read, text) = self.library_part(text, lib, reference, part)?;
        let (i, f) = (self.footprints.len(), read.footprints.into_iter().next().expect("a footprint was read"));
        let (pads, shapes, texts) = (self.pads.len(), self.shapes.len(), self.texts.len());
        // A library part has no nets yet. The layers it brings are the board's too (not copper:
        // only the board's layer table makes copper).
        self.pads.extend(read.pads.into_iter().map(|p| Pad { footprint: i as u32, net: 0, ..p }));
        self.shapes.extend(read.shapes);
        self.texts.extend(read.texts);
        self.layers = read.layers;
        let ranges = (pads..self.pads.len(), shapes..self.shapes.len(), texts..self.texts.len());
        self.footprints.push(Footprint { pads: ranges.0, shapes: ranges.1, texts: ranges.2, src: Src::Library(text), ..f });
        self.place_footprint(i, at, angle);
        Ok(i)
    }

    /// Footprint `text` of library `lib` as part `reference` (a schematic's part's with its value
    /// and link: `part`), read alone at the origin into a board of this one's layers, with the
    /// text saving keeps of it (`Src::Library`); an error points into `text`.
    pub(crate) fn library_part(&self, text: &str, lib: &str, reference: &str, part: Option<(&str, &str)>) -> Result<(Board, String)> {
        // Read into a board of this one's layers (`*.Cu` is its copper): first as written, so an
        // error points into `text`.
        let read = |text: &str| {
            let (layers, stackup) = (self.layers.clone(), self.stackup.clone());
            let blank = Board { version: self.version, layers, copper: self.copper, stackup, ..Board::default() };
            let (mut p, c) = (Parser::onto(blank), &mut Cursor::new(text));
            match c.child()? {
                Some("footprint" | "module") => p.footprint(c).map(|()| p.b),
                _ => Err(c.err(tr("not a KiCad footprint (a .kicad_mod file)"))),
            }
        };
        read(text)?;
        let text = at_origin(text, lib, reference, part, self.version, uuid((lib, reference, self.footprints.len(), self.pads.len())))?;
        Ok((read(&text)?, text))
    }
}

/// Footprint `text` (read before) from library `lib` as a board holds it at the origin: linked as
/// `lib:name`, named `reference`, with `id` (by the board's KiCad `version`) and `(at 0 0)` after
/// its layer, without the library file's version and generator; a schematic's part's with its
/// value and `(path …)` to its symbol (`part`).
fn at_origin(text: &str, lib: &str, reference: &str, part: Option<(&str, &str)>, version: u32, id: String) -> Result<String> {
    let c = &mut Cursor::new(text);
    c.child()?;
    let name = c.atom()?;
    let mut edits = vec![(c.span(), format!("\"{lib}:{name}\""))];
    let (mut end, mut layer, mut placed) = (c.span().end, None, false);
    while let Some(head) = c.child()? {
        // The list starts at its `(`, just before its head; the text before it parts it from the last.
        let start = c.span().start - 1;
        match head {
            "property" | "fp_text" => match (c.atom()?, part) {
                ("Reference" | "reference", _) => {
                    c.atom()?;
                    edits.push((c.span(), format!("\"{}\"", escape(reference))));
                }
                ("Value" | "value", Some((value, _))) => {
                    c.atom()?;
                    edits.push((c.span(), format!("\"{}\"", escape(value))));
                }
                _ => {}
            },
            "at" => placed = true,
            _ => {}
        }
        c.close()?;
        match head {
            "version" | "generator" | "generator_version" => edits.push((end..c.span().end, String::new())),
            "layer" => layer = Some((&text[end..start], c.span().end)),
            _ => {}
        }
        end = c.span().end;
    }
    let (gap, after) = layer.ok_or_else(|| c.err(tr("footprint without a layer")))?;
    // Its pads, texts and graphics get ids of their own, as KiCad gives each item of a footprint
    // it places: the library's would be those of every part made from it.
    let (mut lex, mut last, mut k) = (Lexer::new(text), [Tok::Close; 2], 0);
    while let Some((tok, start, end)) = lex.next_tok()? {
        if let ([Tok::Open, Tok::Atom("uuid" | "tstamp")], Tok::Atom(_) | Tok::Str(_)) = (last, tok) {
            let new = uuid((&id, k));
            edits.push((start..end, if matches!(tok, Tok::Str(_)) { format!("\"{new}\"") } else { new }));
            k += 1;
        }
        last = [last[1], tok];
    }
    let id = match version {
        KICAD8.. => format!("{gap}(uuid \"{id}\")"),
        KICAD6.. => format!("{gap}(tstamp {id})"),
        _ => String::new(),
    };
    let at = if placed { String::new() } else { format!("{gap}(at 0 0)") };
    let link = part.map_or(String::new(), |(_, link)| format!("{gap}(path \"{}\")", escape(link)));
    edits.push((after..after, format!("{id}{at}{link}")));
    Ok(apply(text, edits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{MM, Text};
    use crate::history::Patch;

    /// KiCad 10's `MountingHole:MountingHole_3.2mm_M3_Pad_Via`, two of its pads.
    const HOLE: &str = r#"(footprint "MountingHole_3.2mm_M3_Pad_Via"
	(version 20260206)
	(generator "kicad-footprint-generator")
	(layer "F.Cu")
	(descr "Mounting Hole 3.2mm, M3")
	(property "Reference" "REF**"
		(at 0 -4.15 0)
		(layer "F.SilkS")
		(effects
			(font
				(size 1 1)
				(thickness 0.15)
			)
		)
	)
	(property "Value" "MountingHole_3.2mm_M3_Pad_Via"
		(at 0 4.15 0)
		(layer "F.Fab")
		(effects
			(font
				(size 1 1)
				(thickness 0.15)
			)
		)
	)
	(attr exclude_from_pos_files exclude_from_bom)
	(fp_circle
		(center 0 0)
		(end 3.45 0)
		(stroke
			(width 0.05)
			(type solid)
		)
		(fill no)
		(layer "F.CrtYd")
	)
	(pad "1" thru_hole circle
		(at -2.4 0)
		(size 0.8 0.8)
		(drill 0.5)
		(layers "*.Cu" "*.Mask")
	)
	(pad "1" thru_hole circle
		(at 0 0)
		(size 6.4 6.4)
		(drill 3.2)
		(layers "*.Cu" "*.Mask")
	)
	(embedded_fonts no)
)"#;

    #[test]
    fn adds_a_library_footprint_with_the_next_free_reference() {
        let mut b = Board::parse(
            r#"(kicad_pcb (version 20260206) (layers (0 "F.Cu" signal) (2 "In1.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user))
              (footprint "MountingHole:MountingHole_2.2mm_M2" (layer "F.Cu") (at 1 1) (property "Reference" "H2")))"#,
        )
        .unwrap();
        let i = b.add_footprint(HOLE, "MountingHole", Pt { x: 10 * MM, y: 20 * MM }, 90.0).unwrap();
        let f = &b.footprints[i];
        assert_eq!((f.reference.as_str(), f.lib_id.as_str(), f.fitted), ("H3", "MountingHole:MountingHole_3.2mm_M3_Pad_Via", false));
        // Turned 90° counter-clockwise about its anchor; its plated holes on every copper layer.
        let pad = &b.pads[f.pads.start];
        assert_eq!((pad.at, pad.angle, pad.footprint, pad.layers & b.copper), (Pt { x: 10 * MM, y: 22_400_000 }, 90.0, 1, b.copper));
        assert_eq!(
            b.texts[f.texts.start],
            Text { text: "H3".into(), at: Pt { x: 5_850_000, y: 20 * MM }, angle: 90.0, ..b.texts[f.texts.start].clone() }
        );
        // The layers it brought are the board's now.
        assert_eq!((b.layers.len(), &b.layers[4..]), (6, &["F.Fab".to_string(), "F.CrtYd".to_string()][..]));
        // A file that isn't a footprint, or a bad one, changes nothing; the error points into it.
        let before = (b.footprints.len(), b.pads.len(), b.layers.len());
        assert!(b.add_footprint("(kicad_pcb)", "X", Pt::default(), 0.0).is_err());
        let bad = HOLE.replace("(size 0.8 0.8)", "(size 0.8 x)");
        let e = b.add_footprint(&bad, "MountingHole", Pt::default(), 0.0).unwrap_err();
        assert_eq!((e.msg, e.line_col(&bad)), ("bad length", (bad.lines().position(|l| l.contains(" x)")).unwrap() + 1, 13)));
        assert_eq!(before, (b.footprints.len(), b.pads.len(), b.layers.len()));
        // Undone, it goes with its pads, graphics and texts; redone, it is back.
        let parts = |b: &Board| (b.footprints.clone(), b.pads.clone(), b.shapes.clone(), b.texts.clone());
        let (undo, before) = (Patch::added(&b), parts(&b));
        let i = b.add_footprint(HOLE, "MountingHole", Pt::default(), 0.0).unwrap();
        let redo = b.apply(undo);
        assert!(parts(&b) == before);
        b.apply(redo);
        assert_eq!((b.footprints[i].reference.as_str(), b.footprints.len(), b.pads.len()), ("H4", 3, 4));
    }

    #[test]
    fn keeps_the_boards_copper_layers() {
        let src = r#"(kicad_pcb (version 20260206) (layers (0 "F.Cu" signal) (2 "In1.Cu" signal) (31 "B.Cu" signal)))"#;
        let mut b = Board::parse(src).unwrap();
        // As KiCad's layer markers write on every inner layer.
        let part = r#"(footprint "LayerMarker" (layer "F.Cu")
              (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1) (layers "F.Cu" "In1.Cu" "In5.Cu" "User.1"))
              (fp_line (start 0 0) (end 1 0) (layer "In5.Cu") (stroke (width 0.1))))"#;
        let (copper, i) = (b.copper, b.add_footprint(part, "Symbol", Pt::default(), 0.0).unwrap());
        let pad = b.pads[b.footprints[i].pads.start].layers;
        let names: Vec<_> = (0..b.layers.len()).filter(|&l| pad >> l & 1 != 0).map(|l| b.layers[l].as_str()).collect();
        // Its plated hole puts it on all the board's copper, as in KiCad.
        assert_eq!(
            (b.copper, names, &b.layers[3..]),
            (copper, vec!["F.Cu", "In1.Cu", "B.Cu", "In5.Cu", "User.1"], &["In5.Cu".into(), "User.1".into()][..])
        );
        // Read back, the board's layer table still makes its copper.
        let back = Board::parse(&b.write(src).unwrap()).unwrap();
        assert_eq!((back.copper, back.footprints.len()), (copper, 1));
    }

    #[test]
    fn holds_the_library_text_at_the_origin_as_the_boards_version_writes_ids() {
        let text = |version| at_origin(HOLE, "MountingHole", "H1", None, version, "ID".into()).unwrap();
        let head = |t: &str| t.lines().take(4).collect::<Vec<_>>().join("|");
        assert_eq!(
            head(&text(20260206)),
            "(footprint \"MountingHole:MountingHole_3.2mm_M3_Pad_Via\"|\t(layer \"F.Cu\")|\t(uuid \"ID\")|\t(at 0 0)"
        );
        assert_eq!(
            head(&text(20221018)),
            "(footprint \"MountingHole:MountingHole_3.2mm_M3_Pad_Via\"|\t(layer \"F.Cu\")|\t(tstamp ID)|\t(at 0 0)"
        );
        assert!(
            text(20171130).contains("(layer \"F.Cu\")\n\t(at 0 0)\n\t(descr") && text(20171130).contains("(property \"Reference\" \"H1\"")
        );
        // Its items get ids of their own, each another: two parts from one footprint share none.
        let lib = "(footprint \"R\" (layer \"F.Cu\")\n\t(property \"Reference\" \"REF**\" (at 0 0) (layer \"F.SilkS\") (uuid \"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\"))\n\t(fp_line (start 0 0) (end 1 0) (layer \"F.SilkS\") (uuid \"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\"))\n\t(pad \"1\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\") (tstamp cccccccc-cccc-4ccc-8ccc-cccccccccccc)))";
        let placed = |reference: &str, id: &str| at_origin(lib, "Device", reference, None, 20260206, id.into()).unwrap();
        let ids = |t: &str| {
            let quoted = t.match_indices("(uuid \"").map(|(at, _)| t[at + 7..].split('"').next().unwrap().to_string());
            let bare = t.match_indices("(tstamp ").map(|(at, _)| t[at + 8..].split(')').next().unwrap().to_string());
            quoted.chain(bare).collect::<Vec<_>>()
        };
        let (one, other) = (ids(&placed("R1", "ID1")), ids(&placed("R2", "ID2")));
        assert_eq!((one.len(), &one[0], &other[0]), (4, &"ID1".to_string(), &"ID2".to_string()));
        assert!(one.iter().chain(&other).collect::<std::collections::HashSet<_>>().len() == 8, "{one:?} {other:?}");
        assert!(one.iter().all(|id| !lib.contains(id.as_str())) && placed("R1", "ID1") == placed("R1", "ID1"));
        // KiCad 5's own format: its reference text.
        let old = "(module Fiducial_1mm (layer F.Cu) (tedit 5A0F774F)\n  (fp_text reference REF** (at 0 -2) (layer F.SilkS)))";
        assert_eq!(
            at_origin(old, "Fiducial", "FID1", None, 20171130, String::new()).unwrap(),
            "(module \"Fiducial:Fiducial_1mm\" (layer F.Cu) (at 0 0) (tedit 5A0F774F)\n  (fp_text reference \"FID1\" (at 0 -2) (layer F.SilkS)))"
        );
    }

    #[test]
    fn finds_library_footprints_by_words_the_closest_first() {
        let dir = std::env::temp_dir().join(format!("cuprita-libs-{}", std::process::id()));
        for (lib, name) in [
            ("MountingHole", "MountingHole_3.2mm_M3_Pad"),
            ("MountingHole", "MountingHole_3.2mm_M3"),
            ("Fiducial", "Fiducial_1mm"),
            ("Capacitor_SMD", "C_0402_1005Metric"),
            ("Resistor_SMD", "R_0402_1005Metric"),
        ] {
            std::fs::create_dir_all(dir.join(format!("{lib}.pretty"))).unwrap();
            std::fs::write(dir.join(format!("{lib}.pretty/{name}.kicad_mod")), "").unwrap();
        }
        // The same library further on, and a folder that isn't one, are left out.
        std::fs::create_dir_all(dir.join("more/Fiducial.pretty")).unwrap();
        std::fs::write(dir.join("more/Fiducial.pretty/Other.kicad_mod"), "").unwrap();
        let all = footprints_in(&[dir.clone(), dir.join("more")]);
        std::fs::remove_dir_all(&dir).unwrap();
        let ids = |q| search(&all, q).iter().map(|f| f.id()).collect::<Vec<_>>();
        assert_eq!(all.len(), 5);
        assert_eq!(ids("m3"), ["MountingHole:MountingHole_3.2mm_M3", "MountingHole:MountingHole_3.2mm_M3_Pad"]);
        // A one-letter word finds the name it starts, not one that only holds it (Metric's r).
        assert_eq!(ids("r 0402"), ["Resistor_SMD:R_0402_1005Metric", "Capacitor_SMD:C_0402_1005Metric"]);
        assert_eq!(ids("C 0402")[0], "Capacitor_SMD:C_0402_1005Metric");
        assert_eq!(ids("PAD hole"), ["MountingHole:MountingHole_3.2mm_M3_Pad"]);
        assert_eq!(ids("fiducial:fiducial_1MM"), ["Fiducial:Fiducial_1mm"]);
        assert_eq!(search(&all, "").len(), 5);
    }

    #[test]
    fn finds_the_libraries_cuprita_comes_with_beside_the_app_after_kicads() {
        // As a package lays them out, and with a KiCad installed that has one of them too.
        let dir = std::env::temp_dir().join(format!("cuprita-own-{}", std::process::id()));
        let (own, kicad) = (dir.join("usr/share/cuprita/kicad/footprints"), dir.join("kicad"));
        std::fs::create_dir_all(dir.join("usr/bin")).unwrap();
        for lib in [own.join("Fiducial.pretty"), own.join("Package_SO.pretty"), kicad.join("Fiducial.pretty")] {
            std::fs::create_dir_all(&lib).unwrap();
            std::fs::write(lib.join("A.kicad_mod"), "").unwrap();
        }
        let roots: Vec<PathBuf> = [kicad.clone()].into_iter().chain(own_dirs(&dir.join("usr/bin"), "footprints")).collect();
        let all = footprints_in(&roots);
        std::fs::remove_dir_all(&dir).unwrap();
        let from = |lib: &str| all.iter().find(|f| f.lib == lib).map(|f| f.path.starts_with(&kicad));
        assert_eq!((all.len(), from("Fiducial"), from("Package_SO")), (2, Some(true), Some(false)));
    }

    /// A library of KiCad 10's form: a regulator, one that extends it, one that extends that.
    const REGULATORS: &str = r#"(kicad_symbol_lib
	(version 20251024)
	(generator "kicad_symbol_editor")
	(symbol "AP2112K-1.8"
		(pin_names
			(offset 0.254)
		)
		(exclude_from_sim no)
		(in_bom yes)
		(on_board yes)
		(property "Reference" "U"
			(at -5.08 5.715 0)
		)
		(property "Value" "AP2112K-1.8"
			(at 0 5.715 0)
		)
		(property "Description" "600mA low dropout linear regulator, fixed 1.8V output"
			(at 0 0 0)
			(hide yes)
		)
		(property "ki_keywords" "linear regulator ldo"
			(at 0 0 0)
			(hide yes)
		)
		(property "Sim.Enable" "0"
			(at 0 0 0)
			(hide yes)
		)
		(symbol "AP2112K-1.8_0_1"
			(rectangle
				(start -5.08 4.445)
				(end 5.08 -5.08)
			)
		)
		(symbol "AP2112K-1.8_1_1"
			(pin power_in line
				(at -7.62 2.54 0)
				(length 2.54)
				(name "VIN")
				(number "1")
			)
		)
		(embedded_fonts no)
	)
	(symbol "AP2112K-3.3"
		(extends "AP2112K-1.8")
		(property "Reference" "U"
			(at -5.08 5.715 0)
		)
		(property "Value" "AP2112K-3.3"
			(at 0 5.715 0)
		)
		(property "Description" "600mA low dropout linear regulator, fixed 3.3V output"
			(at 0 0 0)
			(hide yes)
		)
		(in_bom no)
		(embedded_fonts no)
	)
	(symbol "AP2112K-3.3_Alt"
		(extends "AP2112K-3.3")
		(property "MPN" "AP2112K-3.3TRG1"
			(at 0 0 0)
		)
		(property "Value" "AP2112K-3.3_Alt"
			(at 0 5.715 0)
		)
	)
	(symbol "Loop_A"
		(extends "Loop_B")
	)
	(symbol "Loop_B"
		(extends "Loop_A")
	)
	(symbol "Orphan"
		(extends "Gone")
	)
)"#;

    #[test]
    fn lists_library_symbols_with_what_they_are() {
        let dir = std::env::temp_dir().join(format!("cuprita-syms-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("more")).unwrap();
        std::fs::write(dir.join("Regulator_Linear.kicad_sym"), REGULATORS).unwrap();
        // KiCad 7's description, a library cut short, the same library further on: left out.
        let old = "(kicad_symbol_lib (version 20220914) (symbol \"R\" (property \"ki_description\" \"Resistor\")))";
        std::fs::write(dir.join("Device.kicad_sym"), old).unwrap();
        std::fs::write(dir.join("Cut.kicad_sym"), "(kicad_symbol_lib (symbol \"A\") (symbol \"B\" (property").unwrap();
        std::fs::write(dir.join("more/Device.kicad_sym"), old.replace('R', "C")).unwrap();
        let all = symbols_in(&[dir.clone(), dir.join("more")]);
        std::fs::remove_dir_all(&dir).unwrap();
        let ids: Vec<_> = all.iter().map(|s| (s.id(), s.about.as_str())).collect();
        assert_eq!(
            ids[..4],
            [
                ("Cut:A".into(), ""),
                ("Device:R".into(), "Resistor"),
                ("Regulator_Linear:AP2112K-1.8".into(), "600mA low dropout linear regulator, fixed 1.8V output linear regulator ldo"),
                ("Regulator_Linear:AP2112K-3.3".into(), "600mA low dropout linear regulator, fixed 3.3V output"),
            ]
        );
        assert_eq!(all.len(), 8);
        // Found by name first, then by what they are; words may be split between the two.
        let ids = |q| search(&all, q).iter().map(|f| f.id()).collect::<Vec<_>>();
        assert_eq!(ids("resistor"), ["Device:R"]);
        assert_eq!(ids("3.3 regulator")[..2], ["Regulator_Linear:AP2112K-3.3".to_string(), "Regulator_Linear:AP2112K-3.3_Alt".into()]);
        assert_eq!(ids("ldo"), ["Regulator_Linear:AP2112K-1.8"]);
        assert_eq!(ids("r")[0], "Device:R");
    }

    #[test]
    fn finds_the_symbol_kicad_names_for_a_part_number() {
        let names = ["LM358", "LM358_DFN", "LM2904", "STM32F103C8Tx", "STM32F103C_8-B_Tx", "MCP2515-xSO", "AMS1117", "AMS1117-3.3", "R"];
        let part = |n: &str| LibPart { lib: "L".into(), name: n.into(), path: PathBuf::new(), about: String::new() };
        let mut all: Vec<_> = names.map(part).into();
        all[2].about = "Dual op amp, as LM358DR".into();
        let found = |q| search(&all, q).iter().map(|p| p.name.as_str()).collect::<Vec<_>>();
        // The ordering code's package and reel letters left off, `x` for any one character, signs
        // aside; before the parts that hold the words.
        assert_eq!(found("LM358DR"), ["LM358", "LM2904"]);
        assert_eq!(found("stm32f103c8t6"), ["STM32F103C8Tx"]);
        assert_eq!(found("MCP2515-I/SO"), ["MCP2515-xSO"]);
        // The longest name only; never a number cut short, nor a name without one.
        assert_eq!(found("AMS1117-3.3TRG"), ["AMS1117-3.3"]);
        assert!(found("LM3580").is_empty());
        assert!(found("RC0603FR-0710KL").is_empty());
        assert_eq!(found("LM358 DR"), ["LM2904"]);
    }

    #[test]
    fn matches_footprints_to_a_symbols_filters_as_kicad_does() {
        let fp = LibPart { lib: "LED_SMD".into(), name: "LED_0603_1608Metric".into(), path: PathBuf::new(), about: String::new() };
        let fits = |filter| fits(filter, &fp);
        assert!(fits("LED*") && fits("led_smd:*") && fits("LED_????_1608*") && fits("*1608Metric"));
        assert!(!fits("LED_THT:*") && !fits("LED_0603") && !fits("R_*") && !fits("LED_???_1608*"));
        assert_eq!(field("(symbol \"R\" (property \"Footprint\" \"R:R_0603\" (at 0 0 0)))", "Footprint").as_deref(), Some("R:R_0603"));
    }

    #[test]
    fn flattens_a_symbol_onto_the_one_it_extends() {
        let s = symbol(REGULATORS, "AP2112K-3.3_Alt").unwrap();
        let (name, lists) = lists(&s).unwrap();
        let heads: Vec<_> = lists.iter().map(|(h, k, _)| format!("{h} {k}")).collect();
        // The fields and flags of the ones it extends, but those it has of its own (its value, its
        // parent's description and `in_bom`), then its other fields; their units under its name.
        assert_eq!(
            (&s[name], heads),
            (
                "\"AP2112K-3.3_Alt\"",
                [
                    "pin_names ",
                    "exclude_from_sim ",
                    "in_bom ",
                    "on_board ",
                    "property Reference",
                    "property Value",
                    "property Description",
                    "property ki_keywords",
                    "property Sim.Enable",
                    "property MPN",
                    "symbol AP2112K-3.3_Alt_0_1",
                    "symbol AP2112K-3.3_Alt_1_1",
                    "embedded_fonts ",
                ]
                .map(String::from)
                .to_vec()
            )
        );
        assert!(
            s.contains("\n\t\t(in_bom no)\n\t\t(on_board yes)")
                && s.contains("fixed 3.3V output")
                && s.ends_with(")\n\t\t(embedded_fonts no)\n\t)")
        );
        assert!(s.contains("(symbol \"AP2112K-3.3_Alt_1_1\"\n\t\t\t(pin power_in line"));
        // One that extends nothing is as written.
        assert!(symbol(REGULATORS, "AP2112K-1.8").unwrap().starts_with("(symbol \"AP2112K-1.8\"\n\t\t(pin_names"));
        assert_eq!(symbol(REGULATORS, "LM7805").unwrap_err(), "the library has no symbol LM7805");
        assert_eq!(symbol(REGULATORS, "Orphan").unwrap_err(), "Orphan extends Gone, which the library lacks");
        assert!(symbol(REGULATORS, "Loop_A").unwrap_err().contains("extends itself"));
        assert_eq!(symbol("(kicad_symbol_lib\n(symbol \"A\" (", "A").unwrap_err(), "2:14: unexpected end of file");
    }
}
