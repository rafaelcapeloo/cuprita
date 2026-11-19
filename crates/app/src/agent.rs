//! AI agents' session (MCP stdio server `--mcp`, JSON CLI `--json`): a board opened headless and
//! the app's commands on it.

mod mcp;
mod parts;
mod render;
mod rules;
mod sheet;

pub(crate) use mcp::functions;

use crate::edit::{NO_LIBRARIES, add_from_library, flipped};
use crate::fab::{fab_files, write_files};
use crate::file::save_file;
use crate::sheet_edit::Sheets;
use crate::{Found, find_target, load_base, pick_name};
use cuprita_core::board::{Board, Connect, Footprint, LayerId, Nm, Pt, Zone};
use cuprita_core::diff::Diff;
use cuprita_core::drc::Item;
use cuprita_core::history::{History, Patch, Pick};
use cuprita_core::json::{Val, json_str, list};
use cuprita_core::library::{self, LibPart};
use cuprita_core::models::Models;
use cuprita_core::new;
use cuprita_core::pair::PairRouter;
use cuprita_core::route::{Router, tracks};
use cuprita_core::shove::{Shoved, Shover};
use cuprita_render::{Scene, build, rebuild};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Session {
    path: Option<PathBuf>,
    name: String,
    board: Board,
    /// The text `board` was read from, which `save` writes the edits into.
    src: String,
    scene: Option<Scene>,
    /// The edits to undo and redo, on the board and on the schematic `sch`.
    edits: Edits,
    /// The project's schematic, read when an edit first needs it, with its edits.
    sch: Option<Sheets>,
    /// KiCad's footprints and symbols, read when `add` first needs them.
    parts: Option<Vec<LibPart>>,
    symbols: Option<Vec<LibPart>>,
    /// The footprints' 3D models, read when a 3D render first needs them.
    models: Models,
}

/// The board's edits, and the order of all the session's edits to undo and redo: on the board
/// (`false`) or the schematic (`true`).
#[derive(Default)]
struct Edits {
    board: History,
    done: Vec<bool>,
    undone: Vec<bool>,
}

impl Edits {
    /// Keeps an edit of board `b`, undone by `undo`, with the zones whose fill it changes
    /// refilled (`Board::refill`), as the app keeps one.
    fn push(&mut self, b: &mut Board, undo: Patch, what: String) {
        self.board.push(b.refill(undo).0, what);
        self.did(false);
    }

    /// An edit was made to the schematic (`true`) or the board: the ones undone can't be redone.
    fn did(&mut self, sheets: bool) {
        self.done.push(sheets);
        self.undone.clear();
    }

    /// What the next undo (or redo) changes: the schematic (`true`) or the board.
    fn step(&mut self, redo: bool) -> Option<bool> {
        let (from, to) = if redo { (&mut self.undone, &mut self.done) } else { (&mut self.done, &mut self.undone) };
        let sheets = from.pop()?;
        to.push(sheets);
        Some(sheets)
    }

    /// Forgets the edits to the schematic (`true`), or the board's: another one was read.
    fn forget(&mut self, sheets: bool) {
        if !sheets {
            self.board = History::default();
        }
        self.done.retain(|&s| s != sheets);
        self.undone.retain(|&s| s != sheets);
    }
}

/// What `--help` prints: how to run Cuprita, and every command agents have.
pub fn usage() -> String {
    format!(
        "Cuprita: a PCB and schematic CAD that opens and saves KiCad files.\n\n\
         cuprita [project.kicad_pro | board.kicad_pcb | sheet.kicad_sch]   open the window\n\
         cuprita --mcp [board]          an MCP server on stdin and stdout (JSON-RPC 2.0, one message a line)\n\
         cuprita --json [board] '{{\"cmd\":\"query\"}}'…   run commands, one JSON reply a line; with none, read them from stdin\n\
         cuprita --serve project [--port 8080] [--bind 127.0.0.1]   a web page of its schematic, board and 3D view, to look at\n\
         cuprita --screenshot out.png board   save a picture of the window, then exit\n\
         cuprita --bench board          print frames a second\n\n\
         Commands (MCP tools; the JSON CLI's \"cmd\", their arguments beside it): {}.\n\
         MCP's tools/list says what each takes. A Cuprita window with the project open shows what an agent saves.",
        mcp::names().join(", ")
    )
}

pub fn run(mcp: bool, file: Option<PathBuf>, cmds: Vec<String>) {
    let mut s = Session::default();
    if let Some(p) = file
        && let Err(e) = s.open(&crate::opens(&p))
    {
        eprintln!("{e}");
    }
    let mut out = std::io::stdout().lock();
    if mcp {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if let Some(resp) = s.mcp_line(&line) {
                let _ = writeln!(out, "{resp}");
                let _ = out.flush();
            }
        }
    } else if !cmds.is_empty() {
        for c in &cmds {
            let _ = writeln!(out, "{}", s.cli_line(c));
        }
    } else {
        // A person at a terminal is told what is being waited for.
        if std::io::stdin().is_terminal() {
            eprintln!(
                "Cuprita reads commands from here, a JSON object a line ({{\"cmd\":\"query\"}}); Ctrl+D ends. cuprita --help lists them."
            );
        }
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                let _ = writeln!(out, "{}", s.cli_line(&line));
                let _ = out.flush();
            }
        }
    }
}

impl Session {
    /// A session on `board`, a copy of the app's with its file's `path` and `name` (`None`: the
    /// board at `path` as its file has it, if there is one), and on `sch`, the project's schematic
    /// as the app edits it (`None`: read from its files when needed): the chat's (`chat.rs`).
    pub(crate) fn of(path: Option<PathBuf>, name: String, board: Option<Board>, sch: Option<Sheets>) -> Session {
        let Some(board) = board else {
            // The path finds the schematic; a board that can't be read is as none.
            let mut s = Session { path: path.clone(), sch, ..Session::default() };
            if let Some(p) = path.filter(|p| p.exists()) {
                let _ = s.open(&p);
            }
            return s;
        };
        Session { path, name, scene: Some(build(&board)), board, sch, ..Session::default() }
    }

    /// The open board's file, and the board.
    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The open project's folder, whose own libraries `add` finds parts in too.
    pub(super) fn folder(&self) -> Option<PathBuf> {
        self.path.as_deref().or_else(|| self.sch.as_ref().map(|s| s.root.as_path()))?.parent().map(Path::to_path_buf)
    }

    pub(crate) fn board(&self) -> &Board {
        &self.board
    }

    pub fn open(&mut self, path: &Path) -> Result<String, String> {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let src = std::fs::read_to_string(path).map_err(|e| format!("{name}: {e}"))?;
        let mut board = Board::parse(&src).map_err(|e| {
            let (line, col) = e.line_col(&src);
            format!("{name}:{line}:{col}: {}", e.msg)
        })?;
        if let Ok(pro) = std::fs::read_to_string(path.with_extension("kicad_pro")) {
            board.apply_pro(&pro);
        }
        let scene = build(&board);
        (self.path, self.name, self.board, self.src, self.scene) = (Some(path.to_path_buf()), name, board, src, Some(scene));
        (self.models, self.parts, self.symbols) = (Models::default(), None, None);
        self.edits.forget(false);
        // A schematic of another project goes, with its edits.
        if self.sch.as_ref().is_some_and(|s| s.root != path.with_extension("kicad_sch")) {
            self.sch = None;
            self.edits.forget(true);
        }
        self.summary()
    }

    /// Starts a project in a new folder, blank or as a copy of one of KiCad's templates, and
    /// opens its board.
    fn new_project(&mut self, args: &Val) -> Result<String, String> {
        let (dir, name) = (Path::new(args.str("dir").ok_or("missing dir")?), args.str("name").ok_or("missing name")?);
        let board = match args.str("template") {
            Some(t) => {
                let all = new::templates();
                let names = || all.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ");
                let found =
                    all.iter().find(|x| x.name.eq_ignore_ascii_case(t)).ok_or_else(|| format!("no template {t}; KiCad's: {}", names()))?;
                new::copy_template(found, dir, name)?
            }
            None => {
                let size = (args.num("width_mm").unwrap_or(100.0), args.num("height_mm").unwrap_or(80.0));
                new::write(dir, name, args.num("copper").unwrap_or(2.0) as u8, size, new::seed())?
            }
        };
        self.open(&board)
    }

    /// Fills every zone again, an undo step when a fill changed; returns how many changed.
    fn fill(&mut self) -> usize {
        let undo = Patch::fills(&self.board);
        let n = self.board.fill_zones();
        if n > 0 {
            self.edits.push(&mut self.board, undo, format!("{n} zone{} filled", if n == 1 { "" } else { "s" }));
            redraw(&mut self.scene, &self.board);
        }
        n
    }

    fn summary(&self) -> Result<String, String> {
        let scene = self.scene.as_ref().ok_or("no board open")?;
        let layers = list(self.board.layers.iter().map(|l| json_str(l)));
        Ok(format!(
            r#"{{"board":{},"bounds_mm":{},"layers":{layers},"footprints":{},"pads":{},"tracks":{},"vias":{},"zones":{},"nets":{}}}"#,
            json_str(&self.name),
            mm_box(scene, scene.bounds),
            self.board.footprints.len(),
            self.board.pads.len(),
            self.board.tracks.len(),
            self.board.vias.len(),
            self.board.zones.len(),
            self.board.nets.len().saturating_sub(1),
        ))
    }

    /// Undoes the session's last edit, on the board or the schematic, or redoes the last undone.
    fn undo(&mut self, redo: bool) -> Result<String, String> {
        let what = match self.edits.step(redo) {
            None => None,
            Some(true) => self.sch.as_mut().and_then(|s| s.undo(redo)),
            Some(false) => {
                let (b, h) = (&mut self.board, &mut self.edits.board);
                let what = if redo { h.redo(b) } else { h.undo(b) }.map(|(w, _)| w.to_string());
                if what.is_some() {
                    redraw(&mut self.scene, b);
                }
                what
            }
        };
        let (cmd, done) = if redo { ("redo", "redone") } else { ("undo", "undone") };
        Ok(format!(r#"{{"{done}":{}}}"#, json_str(&what.ok_or_else(|| format!("nothing to {cmd}"))?)))
    }

    /// Writes the board with its edits over its file, and the schematic's edited sheets over
    /// theirs, each previous version kept as `<file>-bak` first.
    fn save(&mut self) -> Result<String, String> {
        let sheets = match &mut self.sch {
            Some(s) if s.history.unsaved() => s.save()?,
            _ => vec![],
        };
        let (path, bak) = match self.path.clone() {
            Some(path) => {
                let text = self.board.write(&self.src).map_err(|e| format!("{}: {e}", self.name))?;
                let bak = save_file(&path, text.as_bytes())?;
                self.edits.board.mark_saved();
                (json_str(&path.display().to_string()), bak.map_or_else(|| "null".into(), |b| json_str(&b.display().to_string())))
            }
            None if sheets.is_empty() => return Err("nothing to save: open a board, or edit its schematic".into()),
            None => ("null".into(), "null".into()),
        };
        Ok(format!(r#"{{"saved":{path},"backup":{bak},"sheets":{}}}"#, list(sheets.iter().map(|f| json_str(f)))))
    }

    pub fn exec(&mut self, cmd: &str, args: &Val) -> Result<String, String> {
        match cmd {
            "open" => return self.open(Path::new(args.str("path").ok_or("missing path")?)),
            "new" => return self.new_project(args),
            "erc" => return self.erc(args.str("path")),
            "pdf" => return self.pdf(args.str("path")),
            "simulate" => {
                let wanted = args
                    .arr("signals")
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|s| if let Val::Str(s) = s { Some(s.as_str()) } else { None });
                return self.simulate(args.str("path"), &wanted.collect::<Vec<_>>());
            }
            "update" => return self.update(),
            "add" if args.get("label").is_some() || args.get("hier_label").is_some() => return self.add_label(args),
            "add" if args.get("no_connect").is_some() => return self.add_no_connect(args),
            "add" if args.get("bus_entry").is_some() => return self.add_bus_entry(args),
            "add" if args.get("symbol").is_some() || args.get("part").is_some() => return self.add_symbol(args),
            "wire" | "bus" => return self.add_wire(args, cmd == "bus"),
            "connect" => return self.connect(args),
            "sheet" => return self.add_sheet(args),
            "field" => return self.field(args),
            "import" => return self.import(args),
            "library" => return self.library(args),
            "new_part" => return self.new_part(args),
            "part_text" => return self.part_text(args),
            "rules" => return self.rules(args),
            "netclass" => return self.netclass(args),
            "undo" | "redo" => return self.undo(cmd == "redo"),
            "save" => return self.save(),
            _ => {}
        }
        let scene = self.scene.as_ref().ok_or("no board open")?;
        match cmd {
            "query" => {
                if let Some([Val::Num(x), Val::Num(y)]) = args.arr("at") {
                    let at = scene.um(mm_pt(*x, *y));
                    let tol = args.num("tol_mm").unwrap_or(0.1) as f32 * 1000.0;
                    let visible: Vec<bool> = scene.layers.iter().map(|l| l.visible).collect();
                    let net = scene.net_at(at, tol, &visible, args.bool("flip").unwrap_or(false), None);
                    return Ok(format!(
                        r#"{{"at_mm":[{x:.3},{y:.3}],"net":{net},"name":{},{}}}"#,
                        json_str(&self.board.nets[net as usize]),
                        net_fields(&self.board, net),
                    ));
                }
                let q = args.str("filter").unwrap_or("").to_ascii_lowercase();
                match args.str("what").unwrap_or("summary") {
                    "summary" => self.summary(),
                    "layers" => {
                        let items = scene.layers.iter().map(|l| format!(r#"{{"name":{},"visible":{}}}"#, json_str(&l.name), l.visible));
                        Ok(format!(r#"{{"layers":{}}}"#, list(items)))
                    }
                    "nets" => {
                        let items = (1..self.board.nets.len())
                            .filter(|&i| self.board.nets[i].to_ascii_lowercase().contains(&q))
                            .map(|i| format!(r#"{{"id":{i},"name":{}}}"#, json_str(&self.board.nets[i])));
                        Ok(format!(r#"{{"nets":{}}}"#, list(items)))
                    }
                    "footprints" => {
                        let items = self.board.footprints.iter().filter(|f| f.reference.to_ascii_lowercase().contains(&q));
                        Ok(format!(r#"{{"footprints":{}}}"#, list(items.map(|f| footprint_json(scene, f)))))
                    }
                    "bom" => {
                        let fp = &self.board.footprints;
                        let hit = |i: &usize| {
                            [&fp[*i].reference, &fp[*i].value, &fp[*i].lib_id].iter().any(|s| s.to_ascii_lowercase().contains(&q))
                        };
                        let items = self.board.bom().into_iter().filter(|line| line.iter().any(hit)).map(|line| {
                            let refs = list(line.iter().map(|&i| json_str(&fp[i].reference)));
                            let f = &fp[line[0]];
                            let [maker, mpn, lcsc] = f.part_numbers();
                            let numbers =
                                [("manufacturer", maker), ("mpn", mpn), ("lcsc", lcsc)].into_iter().filter(|(_, v)| !v.is_empty());
                            let numbers: String = numbers.map(|(k, v)| format!(r#","{k}":{}"#, json_str(v))).collect();
                            format!(
                                r#"{{"value":{},"footprint":{}{numbers},"references":{refs}}}"#,
                                json_str(&f.value),
                                json_str(&f.lib_id)
                            )
                        });
                        Ok(format!(r#"{{"bom":{}}}"#, list(items)))
                    }
                    w => Err(format!("unknown query: {w}")),
                }
            }
            "find" => {
                let q = args.str("query").ok_or("missing query")?;
                match find_target(&self.board, scene, q).ok_or_else(|| format!("not found: {q}"))? {
                    Found::Footprint { index, bounds, .. } => {
                        let f = &self.board.footprints[index];
                        // Empty fields, and KiCad's own (`ki_fp_filters`), say nothing of the part.
                        let fields = f.fields.iter().filter(|(k, v)| !v.is_empty() && !k.starts_with("ki_"));
                        let fields: Vec<String> = fields.map(|(k, v)| format!("{}:{}", json_str(k), json_str(v))).collect();
                        // Every pad where a route starts or ends: its net (none: null) and copper layers.
                        let b = &self.board;
                        let copper = |m: u128| {
                            list(
                                b.layers
                                    .iter()
                                    .enumerate()
                                    .filter(|(i, l)| m >> i & 1 == 1 && l.ends_with(".Cu"))
                                    .map(|(_, l)| json_str(l)),
                            )
                        };
                        let pads = b.pads[f.pads.clone()].iter().map(|p| {
                            let net = if p.net == 0 { "null".into() } else { json_str(&b.nets[p.net as usize]) };
                            let (x, y) = (p.at.x as f64 / 1e6, p.at.y as f64 / 1e6);
                            format!(r#"{{"number":{},"net":{net},"at_mm":[{x},{y}],"layers":{}}}"#, json_str(&p.number), copper(p.layers))
                        });
                        Ok(format!(
                            r#"{{"kind":"footprint","reference":{},"value":{},"footprint":{},"fields":{{{}}},"back":{},"bounds_mm":{},"pads":{}}}"#,
                            json_str(&f.reference),
                            json_str(&f.value),
                            json_str(&f.lib_id),
                            fields.join(","),
                            f.back,
                            mm_box(scene, bounds),
                            list(pads),
                        ))
                    }
                    Found::Net { net, bounds, .. } => {
                        let b = bounds.map_or_else(|| "null".into(), |b| mm_box(scene, b));
                        let (fp, pads) = (&self.board.footprints, self.board.pads.iter().filter(|p| p.net == net));
                        let mut nodes: Vec<String> =
                            pads.map(|p| json_str(&format!("{} pin {}", fp[p.footprint as usize].reference, p.number))).collect();
                        nodes.dedup();
                        Ok(format!(
                            r#"{{"kind":"net","id":{net},"name":{},"bounds_mm":{b},{},"nodes":[{}]}}"#,
                            json_str(&self.board.nets[net as usize]),
                            net_fields(&self.board, net),
                            nodes.join(","),
                        ))
                    }
                }
            }
            "place" => {
                let q = args.str("reference").ok_or("missing reference")?;
                let fps = &self.board.footprints;
                let i = fps.iter().position(|f| f.reference.eq_ignore_ascii_case(q)).ok_or_else(|| format!("not found: {q}"))?;
                let at = match args.arr("at") {
                    None => fps[i].at,
                    Some([Val::Num(x), Val::Num(y)]) if x.is_finite() && y.is_finite() => mm_pt(*x, *y),
                    Some(_) => return Err("at must be [x_mm, y_mm]".into()),
                };
                let angle = match args.get("angle") {
                    None => None,
                    Some(Val::Num(a)) if a.is_finite() => Some(*a),
                    Some(_) => return Err("angle must be a number of degrees".into()),
                };
                let flip = match args.str("side") {
                    None => false,
                    Some("front" | "top") => fps[i].back,
                    Some("back" | "bottom") => !fps[i].back,
                    Some(side) => return Err(format!("side {side}: give front or back")),
                };
                let undo = Patch::footprint(&self.board, i);
                if flip {
                    self.board.flip_footprint(i);
                }
                let f = &self.board.footprints[i];
                let (angle, what) = (angle.unwrap_or(f.angle), if flip { flipped(f) } else { format!("{} placed", f.reference) });
                self.board.place_footprint(i, at, angle);
                self.edits.push(&mut self.board, undo, what);
                Ok(footprint_json(redraw(&mut self.scene, &self.board), &self.board.footprints[i]))
            }
            "add" => {
                let q = args.str("footprint").ok_or("give footprint as Library:Name, or words that find one (MountingHole M3)")?;
                let at = match args.arr("at") {
                    Some([Val::Num(x), Val::Num(y)]) if x.is_finite() && y.is_finite() => mm_pt(*x, *y),
                    _ => return Err("at must be [x_mm, y_mm]".into()),
                };
                let angle = match args.get("angle") {
                    None => 0.0,
                    Some(Val::Num(a)) if a.is_finite() => *a,
                    Some(_) => return Err("angle must be a number of degrees".into()),
                };
                let folder = self.folder();
                let parts = self.parts.get_or_insert_with(|| crate::parts::footprints(folder.as_deref()));
                let part = pick(parts, q, ("part", "Library:Name (MountingHole M3)"), NO_LIBRARIES)?;
                let undo = Patch::added(&self.board);
                let i = add_from_library(&mut self.board, &part, at, angle)?;
                let what = format!("{} added", self.board.footprints[i].reference);
                self.edits.push(&mut self.board, undo, what);
                Ok(footprint_json(redraw(&mut self.scene, &self.board), &self.board.footprints[i]))
            }
            "route" => {
                let pt = |k: &str| match args.arr(k) {
                    Some([Val::Num(x), Val::Num(y)]) if x.is_finite() && y.is_finite() => Ok(mm_pt(*x, *y)),
                    _ => Err(format!("{k} must be [x_mm, y_mm]")),
                };
                let (from, to, b) = (pt("from")?, pt("to")?, &mut self.board);
                let layers = copper_layers(b, args)?;
                // Within 0.05 mm, as a pointer picks.
                let mut s = b.route_start(from, 50_000, &layers).ok_or("from must be on a pad, via or track")?;
                match args.get("width_mm") {
                    None => {}
                    Some(Val::Num(w)) if w.is_finite() && *w > 0.0 => s.width = (w * 1e6).round() as Nm,
                    Some(_) => return Err("width_mm must be a positive number".into()),
                }
                let mut r = Router::new(b, s.net, s.layer, s.width, s.at);
                let aim = r.snap(to, 50_000);
                // With shove, other nets' tracks and vias make way where they can, else it walks round.
                let shover = (args.bool("shove") == Some(true)).then(|| Shover::new(b, s.net));
                let (path, step) = match &shover {
                    Some(sh) => sh.step(b, &mut r, (s.at, aim), s.width, s.layer),
                    None => (r.path(s.at, aim), Ok(Shoved::default())),
                };
                let (end, stuck) = (*path.last().expect("a path holds its start"), step.as_ref().err().map(|e| e.what.clone()));
                // Through the board, or with `via_to` only down (or up) to that copper layer.
                let via_to = match args.str("via_to") {
                    Some(name) => {
                        let l = b.layers.iter().position(|l| l.eq_ignore_ascii_case(name)).filter(|&l| b.copper & 1 << l != 0);
                        Some(l.ok_or_else(|| format!("no copper layer {name} for via_to; see query layers"))? as LayerId)
                    }
                    None => None,
                };
                let via = match (via_to, args.bool("via") == Some(true)) {
                    (Some(to), _) => Some(b.via_between(end, s.net, s.layer, to)),
                    (None, true) => Some(b.via(end, s.net)),
                    (None, false) => None,
                };
                // The via makes its room too: where it has none, nothing changes.
                let shoved = match (&shover, via) {
                    (Some(sh), Some(v)) => sh.shove(b, &path, s.width, s.layer, Some(&v)).map_err(|e| e.what),
                    _ => Ok(step.unwrap_or_default()),
                };
                let no_room = |why: String| format!("no room for a via at the end: {why}");
                let shoved = shoved.map_err(no_room)?;
                let (n, moved) = (path.windows(2).filter(|w| w[0] != w[1]).count(), shoved.runs.len() + shoved.vias.len());
                let undo = b
                    .add_route(&shoved, tracks(&path, s.width, s.layer, s.net), via)
                    .map_err(|v| no_room(format!("{}. {}", v.what, v.fix)))?;
                let what = if via.is_some() { "Via added".into() } else { format!("{n} track{} added", if n == 1 { "" } else { "s" }) };
                if n > 0 || via.is_some() {
                    self.edits.push(b, undo, if moved == 0 { what } else { format!("{what}, {moved} moved aside") });
                }
                // What it added: its tracks, after the board's last, and its via.
                let (t, v) = (b.tracks.len(), b.vias.len());
                let changed: Vec<Item> = (t - n..t).map(Item::Track).chain(via.map(|_| Item::Via(v - 1))).collect();
                let violations = cuprita_core::drc::check_items(b, &changed).len();
                redraw(&mut self.scene, b);
                let points = list(path.iter().map(|p| format!("[{},{}]", p.x as f64 / 1e6, p.y as f64 / 1e6)));
                let stuck = stuck.map_or_else(String::new, |why| format!(r#","walked_round":{}"#, json_str(&why)));
                Ok(format!(
                    r#"{{"net":{},"layer":{},"width_mm":{},"points_mm":{points},"reached":{},"connected":{},"via":{},"moved":{moved}{stuck},"violations":{violations}}}"#,
                    json_str(&b.nets[s.net as usize]),
                    json_str(&b.layers[s.layer as usize]),
                    s.width as f64 / 1e6,
                    end == aim,
                    end == aim && end != s.at && r.connects(end),
                    via.is_some(),
                ))
            }
            "delete" => {
                let b = &self.board;
                let pick = match (args.str("reference"), args.arr("at")) {
                    (Some(q), _) => {
                        let i = b
                            .footprints
                            .iter()
                            .position(|f| f.reference.eq_ignore_ascii_case(q))
                            .ok_or_else(|| format!("not found: {q}"))?;
                        Pick::Footprint(i)
                    }
                    // Within 0.05 mm, as a pointer picks: copper on top, else a text or drawing of
                    // the board's own, else a zone.
                    (None, Some([Val::Num(x), Val::Num(y)])) if x.is_finite() && y.is_finite() => {
                        let (at, layers) = (mm_pt(*x, *y), copper_layers(b, args)?);
                        match b.copper_at(at, 50_000, &layers) {
                            Some(Item::Track(i)) => Pick::Track(i),
                            Some(Item::Via(i)) => Pick::Via(i),
                            Some(Item::Pad(i)) => {
                                let r = &b.footprints[b.pads[i].footprint as usize].reference;
                                return Err(format!("[{x}, {y}] is on a pad of {r}: to delete the part, give its reference"));
                            }
                            None => (b.drawing_at(at, 50_000, u128::MAX))
                                .or_else(|| b.zone_at(at, 50_000, &layers).map(Pick::Zone))
                                .ok_or_else(|| {
                                    format!("no track, via, text, drawing or zone at [{x}, {y}]; query at says what is there")
                                })?,
                        }
                    }
                    _ => return Err("give reference (a part) or at [x_mm, y_mm] (a track, via, text, drawing or zone)".into()),
                };
                let what = pick_name(b, pick);
                let undo = self.board.apply(Patch::delete(&self.board, pick));
                self.edits.push(&mut self.board, undo, format!("{what} deleted"));
                redraw(&mut self.scene, &self.board);
                Ok(format!(r#"{{"deleted":{}}}"#, json_str(&what)))
            }
            "zone" => {
                let b = &mut self.board;
                let (Some(q), Some(_)) = (args.str("net"), args.str("layer")) else {
                    return Err("give the zone's net and layer by name (query nets, query layers)".into());
                };
                let net = b.find_net(q).ok_or_else(|| format!("no net {q}; see query nets"))?;
                let layer = copper_layers(b, args)?[0];
                let corners = mm_pts(args, "corners").filter(|c| c.len() >= 3).ok_or("corners must be [[x_mm, y_mm], …], three or more")?;
                let connect = match args.str("pads") {
                    None | Some("thermal") => None,
                    Some("solid") => Some(Connect::Solid),
                    Some("thru_hole_thermal") => Some(Connect::ThruHoleThermal),
                    Some("none") => Some(Connect::None),
                    Some(p) => return Err(format!("pads {p}: give thermal, solid, thru_hole_thermal or none")),
                };
                let (undo, what) = (Patch::added(b), format!("Zone ({}) on {}", b.nets[net as usize], b.layers[layer as usize]));
                let i = b.add_zone(corners, layer, net);
                if let Some(connect) = connect {
                    // One undo step: taking the zone back takes its settings.
                    b.set_zone(i, Zone { connect, ..b.zones[i].clone() });
                }
                self.edits.push(b, undo, format!("{what} added"));
                redraw(&mut self.scene, b);
                Ok(format!(r#"{{"added":{},"filled_areas":{}}}"#, json_str(&what), b.zones[i].fills.len()))
            }
            "outline" => {
                let corners = mm_pts(args, "corners").filter(|c| c.len() >= 3).ok_or("corners must be [[x_mm, y_mm], …], three or more")?;
                let undo = self.board.set_outline(&corners)?;
                self.edits.push(&mut self.board, undo, "Outline drawn".into());
                let (a, b) = self.board.outline_box().ok_or("the new outline doesn't close")?;
                redraw(&mut self.scene, &self.board);
                let mm = |v: Nm| v as f64 / 1e6;
                Ok(format!(r#"{{"outline_mm":[{},{},{},{}]}}"#, mm(a.x), mm(a.y), mm(b.x), mm(b.y)))
            }
            "text" => {
                let b = &self.board;
                let words = args.str("text").map(str::trim).filter(|w| !w.is_empty()).ok_or("give text, the words to write")?;
                let Some([Val::Num(x), Val::Num(y)]) =
                    args.arr("at").filter(|a| a.iter().all(|v| matches!(v, Val::Num(n) if n.is_finite())))
                else {
                    return Err("at must be [x_mm, y_mm]".into());
                };
                let mut t = b.new_text(words, mm_pt(*x, *y), drawing_layer(b, args)?);
                // A height sets the letters' width and pen with it, as KiCad's own do: 15% of it.
                match args.num("height") {
                    Some(h) if h > 0.0 && h.is_finite() => {
                        let h = (h * 1e6).round() as Nm;
                        (t.size, t.thickness) = ((h, h), h * 3 / 20);
                    }
                    Some(_) => return Err("height must be more than 0 mm".into()),
                    None => {}
                }
                t.angle = args.num("angle").filter(|a| a.is_finite()).unwrap_or(0.0).rem_euclid(360.0);
                let (pick, undo) = self.board.add_text(t);
                let (what, (lo, hi)) = (pick_name(&self.board, pick), self.board.extent(&[pick]).unwrap_or_default());
                self.edits.push(&mut self.board, undo, format!("{what} added"));
                redraw(&mut self.scene, &self.board);
                let mm = |v: Nm| v as f64 / 1e6;
                Ok(format!(r#"{{"added":{},"box_mm":[{},{},{},{}]}}"#, json_str(&what), mm(lo.x), mm(lo.y), mm(hi.x), mm(hi.y)))
            }
            "line" => {
                let b = &self.board;
                let points = mm_pts(args, "points").filter(|p| p.len() >= 2).ok_or("points must be [[x_mm, y_mm], …], two or more")?;
                let layer = drawing_layer(b, args)?;
                let width = match args.num("width") {
                    Some(w) if w > 0.0 && w.is_finite() => (w * 1e6).round() as Nm,
                    Some(_) => return Err("width must be more than 0 mm".into()),
                    None => b.ink(layer).0,
                };
                let (lines, undo) = self.board.add_lines(&points, matches!(args.get("closed"), Some(Val::Bool(true))), layer, width);
                let (n, name) = (lines.len(), self.board.layers[layer as usize].clone());
                self.edits.push(&mut self.board, undo, format!("{n} line{} on {name} drawn", if n == 1 { "" } else { "s" }));
                redraw(&mut self.scene, &self.board);
                Ok(format!(r#"{{"lines":{n},"layer":{},"width_mm":{}}}"#, json_str(&name), width as f64 / 1e6))
            }
            "pair" => {
                let b = &self.board;
                let pt = |key: &str| match args.arr(key) {
                    Some([Val::Num(x), Val::Num(y)]) if x.is_finite() && y.is_finite() => Ok(mm_pt(*x, *y)),
                    _ => Err(format!("{key} must be [x_mm, y_mm]")),
                };
                let (start, end, layers) = (pt("from")?, pt("to")?, copper_layers(b, args)?);
                let through = if args.get("through").is_some() {
                    mm_pts(args, "through").ok_or("through must be [[x_mm, y_mm], …], the corners to lead the pair by")?
                } else {
                    vec![]
                };
                let from = b.pair_ends(start, 50_000, &layers)?;
                let mut r = PairRouter::new(b, from, *through.first().unwrap_or(&end));
                for p in through {
                    let laid = r.to(b, p, 50_000);
                    if !r.fix(&laid) {
                        let why = laid.refused.unwrap_or_else(|| "it gets no nearer".into());
                        return Err(format!("the pair can't be led by [{}, {}]: {why}", p.x as f64 / 1e6, p.y as f64 / 1e6));
                    }
                }
                let laid = r.to(b, end, 50_000);
                let undo = self.board.add_pair(&from, &laid)?;
                let (b, [p, n]) = (&self.board, laid.lengths().map(|l| l / 1e6));
                let nets = from.pair.nets.map(|net| b.nets[net as usize].clone());
                self.edits.push(&mut self.board, undo, format!("Pair {} and {} routed", nets[0], nets[1]));
                redraw(&mut self.scene, &self.board);
                Ok(format!(
                    r#"{{"pair":[{},{}],"layer":{},"width_mm":{},"gap_mm":{},"lengths_mm":[{p:.4},{n:.4}],"skew_mm":{:.4},"reached":{}}}"#,
                    json_str(&nets[0]),
                    json_str(&nets[1]),
                    json_str(&self.board.layers[from.layer as usize]),
                    from.pair.width as f64 / 1e6,
                    from.pair.gap as f64 / 1e6,
                    p - n,
                    laid.to.is_some(),
                ))
            }
            "tune" => {
                let b = &self.board;
                let Some([Val::Num(x), Val::Num(y)]) =
                    args.arr("at").filter(|a| a.iter().all(|v| matches!(v, Val::Num(n) if n.is_finite())))
                else {
                    return Err("at must be [x_mm, y_mm], a point of the track to tune".into());
                };
                let Some(Item::Track(i)) = b.copper_at(mm_pt(*x, *y), 50_000, &copper_layers(b, args)?) else {
                    return Err(format!("no track at [{x}, {y}]; query at says what is there"));
                };
                let net = b.tracks[i].net;
                // As long as asked, or as the other side of its differential pair.
                let target = match (args.num("length"), args.bool("match") == Some(true)) {
                    (Some(mm), _) if mm > 0.0 && mm.is_finite() => mm * 1e6,
                    (None, true) => {
                        let pair = b
                            .pair(net)
                            .ok_or_else(|| format!("{} is no side of a differential pair: give length", b.nets[net as usize]))?;
                        b.net_length(pair.nets[usize::from(pair.nets[0] == net)])
                    }
                    _ => return Err("give length, the net's in mm, or match: true for its differential pair's other side".into()),
                };
                let done = self.board.tune(i, target)?;
                let (name, length, turns, n) = (self.board.nets[net as usize].clone(), done.length / 1e6, done.turns, done.tracks.len());
                self.edits.push(&mut self.board, done.undo, format!("{name} tuned to {length:.3} mm"));
                redraw(&mut self.scene, &self.board);
                Ok(format!(r#"{{"net":{},"length_mm":{length:.4},"turns":{turns},"tracks":{n}}}"#, json_str(&name)))
            }
            "tidy" => {
                let fps = &self.board.footprints;
                let part = |r: &Val| match r {
                    Val::Str(q) => fps.iter().position(|f| f.reference.eq_ignore_ascii_case(q)).ok_or_else(|| format!("not found: {q}")),
                    _ => Err("references must be the parts' references, as [\"R1\", \"U3\"]".to_string()),
                };
                let parts = args.arr("references").unwrap_or_default().iter().map(part).collect::<Result<Vec<_>, _>>()?;
                let (undo, d) = self.board.tidy_texts(&parts);
                if d.parts > 0 {
                    self.edits.push(&mut self.board, undo, format!("Texts of {} tidied", d.parts));
                    redraw(&mut self.scene, &self.board);
                }
                Ok(format!(
                    r#"{{"placed":{},"hidden":{},"values":{},"values_hidden":{},"parts_changed":{}}}"#,
                    d.placed, d.hidden, d.values, d.values_hidden, d.parts
                ))
            }
            "fill" => Ok(format!(r#"{{"zones_changed":{}}}"#, self.fill())),
            "fab" => {
                let path = self.path.clone().ok_or("no board file")?;
                let n = self.fill();
                let stem = self.name.strip_suffix(".kicad_pcb").unwrap_or(&self.name);
                let (written, kept) = write_files(&path, &fab_files(&self.board, stem))?;
                let paths = |v: Vec<PathBuf>| list(v.iter().map(|p| json_str(&p.display().to_string())));
                Ok(format!(r#"{{"files":{},"zones_changed":{n},"backups":{}}}"#, paths(written), paths(kept)))
            }
            "drc" => {
                let kind = args.str("kind");
                let vs: Vec<_> = cuprita_core::drc::check(&self.board).into_iter().filter(|v| kind.is_none_or(|k| v.kind == k)).collect();
                let items = vs.iter().map(|v| {
                    format!(
                        r#"{{"kind":{},"severity":"{}","at_mm":[{:.3},{:.3}],"actual_mm":{:.4},"limit_mm":{:.4},"what":{},"fix":{}}}"#,
                        json_str(v.kind),
                        if v.warning { "warning" } else { "error" },
                        v.at.x as f64 / 1e6,
                        v.at.y as f64 / 1e6,
                        v.actual as f64 / 1e6,
                        v.limit as f64 / 1e6,
                        json_str(&v.what),
                        json_str(&v.fix),
                    )
                });
                // The connections still to route, as KiCad's DRC lists its unconnected items.
                let links = if kind.is_none_or(|k| k == "unconnected_items") { self.board.ratsnest() } else { vec![] };
                let mm = |p: Pt| format!("[{:.3},{:.3}]", p.x as f64 / 1e6, p.y as f64 / 1e6);
                let links = links.iter().map(|l| {
                    let what = json_str(&l.what(&self.board));
                    format!(r#"{{"what":{what},"from_mm":{},"to_mm":{},"length_mm":{:.3}}}"#, mm(l.a), mm(l.b), l.mm())
                });
                let errors = vs.iter().filter(|v| !v.warning).count();
                Ok(format!(r#"{{"count":{},"errors":{errors},"violations":{},"unconnected":{}}}"#, vs.len(), list(items), list(links)))
            }
            "diff" => {
                let (base, old) = load_base(self.path.as_deref(), args.str("base"))?;
                let d = Diff::between(&old, &self.board);
                let items = d.changes.iter().map(|c| {
                    let ([x0, y0], [x1, y1]) = (scene.um(c.bounds.0), scene.um(c.bounds.1));
                    format!(
                        r#"{{"kind":{},"item":{},"what":{},"bounds_mm":{}}}"#,
                        json_str(c.kind.as_str()),
                        json_str(c.item),
                        json_str(&c.what),
                        mm_box(scene, [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]),
                    )
                });
                Ok(format!(r#"{{"base":{},"count":{},"changes":{}}}"#, json_str(&base), d.changes.len(), list(items)))
            }
            "ir_drop" | "ir" => {
                let net = match args.str("net") {
                    Some(q) => self.board.find_net(q).ok_or_else(|| format!("not found: {q}"))?,
                    None => self.board.default_ir_net(),
                };
                if net == 0 {
                    return Err("no nets on board".into());
                }
                let ir = self.board.ir_drop(net, args.num("current_a").unwrap_or(1.0), args.str("source"), args.str("sink"));
                let pads = ir.pads.iter().map(|p| {
                    format!(
                        r#"{{"reference":{},"pad":{},"at_mm":[{:.3},{:.3}],"drop_mv":{:.3},"resistance_mohm":{:.3}}}"#,
                        json_str(&p.reference),
                        json_str(&p.number),
                        p.at.x as f64 / 1e6,
                        p.at.y as f64 / 1e6,
                        p.drop_mv,
                        p.resistance_mohm,
                    )
                });
                Ok(format!(
                    r#"{{"net":{net},"name":{},"current_a":{:.3},"source":{},"sink":{},"drop_mv":{:.3},"resistance_mohm":{:.3},"power_mw":{:.3},"max_j_a_mm2":{:.2},"pads":{}}}"#,
                    json_str(&self.board.nets[net as usize]),
                    ir.current_a,
                    json_str(&ir.source),
                    json_str(&ir.sink),
                    ir.max_drop_mv,
                    ir.resistance_mohm,
                    ir.power_mw,
                    ir.max_j_a_mm2,
                    list(pads),
                ))
            }
            "render" => self.render(args),
            _ => Err(format!("unknown command: {cmd}")),
        }
    }
}

/// The library part `q` names (`Library:Name`, or its name), else the only one its words find;
/// `kind` says what parts are (`part`, `symbol`) and what words find one, `missing` what to do
/// when no library was found.
fn pick(all: &[LibPart], q: &str, (kind, words): (&str, &str), missing: &str) -> Result<LibPart, String> {
    let hits = library::search(all, q);
    match hits.first() {
        Some(&f) if hits.len() == 1 || f.id().eq_ignore_ascii_case(q) || f.name.eq_ignore_ascii_case(q) => Ok(f.clone()),
        Some(_) => {
            let some: Vec<_> = hits.iter().take(8).map(|f| f.id()).collect();
            Err(format!("{} {kind}s match {q}: {}…; give one as Library:Name", hits.len(), some.join(", ")))
        }
        None if all.is_empty() => Err(missing.into()),
        None => Err(format!("no KiCad library {kind} matches {q}; give words of {words}")),
    }
}

/// Draws `board` again after an edit, keeping what `rebuild` can of the old scene.
fn redraw<'a>(scene: &'a mut Option<Scene>, board: &Board) -> &'a Scene {
    let new = match scene.take() {
        Some(old) => rebuild(board, old),
        None => build(board),
    };
    scene.insert(new)
}

/// Reference, placement (anchor, rotation, side) and box of a footprint.
fn footprint_json(scene: &Scene, f: &Footprint) -> String {
    let ([x0, y0], [x1, y1]) = (scene.um(f.bounds.0), scene.um(f.bounds.1));
    format!(
        r#"{{"reference":{},"at_mm":[{},{}],"angle":{},"back":{},"bounds_mm":{}}}"#,
        json_str(&f.reference),
        f.at.x as f64 / 1e6,
        f.at.y as f64 / 1e6,
        f.angle,
        f.back,
        mm_box(scene, [x0, y0, x1, y1])
    )
}

fn net_fields(board: &Board, net: u32) -> String {
    let i = board.inspect_net(net);
    format!(
        r#""length_mm":{:.3},"width_mm":[{:.3},{:.3}],"current_a":{:.2},"impedance_ohm":[{:.1},{:.1}],"tracks":{},"vias":{},"pads":{},"zones":{}"#,
        i.length_mm,
        i.width.0 as f64 / 1e6,
        i.width.1 as f64 / 1e6,
        i.current_a,
        i.impedance_ohm.0,
        i.impedance_ohm.1,
        i.tracks,
        i.vias,
        i.pads,
        i.zones,
    )
}

/// The copper layer `layer` names, or every copper layer from the top.
fn copper_layers(b: &Board, args: &Val) -> Result<Vec<LayerId>, String> {
    let Some(name) = args.str("layer") else { return Ok(b.stackup.iter().map(|c| c.layer).collect()) };
    let l = b.layers.iter().position(|l| l.eq_ignore_ascii_case(name)).filter(|&l| b.copper & 1 << l != 0);
    Ok(vec![l.ok_or_else(|| format!("no copper layer {name}; see query layers"))? as LayerId])
}

/// The layer `layer` names for a text or lines, the front silkscreen without one.
fn drawing_layer(b: &Board, args: &Val) -> Result<LayerId, String> {
    let name = args.str("layer").unwrap_or("F.SilkS");
    let found = b.drawing_layers().into_iter().find(|&l| b.layers[l as usize].eq_ignore_ascii_case(name));
    found.ok_or_else(|| format!("no layer {name} to draw on: texts and lines go on any layer of query layers but copper, Edge.Cuts (outline's), Margin and the courtyards"))
}

/// The points `[[x_mm, y_mm], …]` of `args`' `key`; none when one isn't two numbers.
fn mm_pts(args: &Val, key: &str) -> Option<Vec<Pt>> {
    (args.arr(key).unwrap_or_default().iter())
        .map(|p| match p {
            Val::Arr(xy) => match xy.as_slice() {
                [Val::Num(x), Val::Num(y)] if x.is_finite() && y.is_finite() => Some(mm_pt(*x, *y)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn mm_pt(x: f64, y: f64) -> Pt {
    Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
}

fn mm_box(scene: &Scene, [x0, y0, x1, y1]: [f32; 4]) -> String {
    let mm = |u: f32, o: Nm| u as f64 / 1000.0 + o as f64 / 1e6;
    format!("[{:.3},{:.3},{:.3},{:.3}]", mm(x0, scene.origin.x), mm(y0, scene.origin.y), mm(x1, scene.origin.x), mm(y1, scene.origin.y))
}

#[cfg(test)]
pub(crate) mod tests;
