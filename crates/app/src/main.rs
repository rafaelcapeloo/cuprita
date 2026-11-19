//! Cuprita app for the desktop and the browser (`web.rs`): open a KiCad board (argument or
//! drag-and-drop), pan with drag, zoom with the wheel, click an item to highlight its net and
//! select it or its part (drag the part to move it, R turns it, Delete deletes what is
//! selected), add parts from KiCad's libraries with A, route tracks with X (V adds a via), fill
//! zones with B, find a part or net with Ctrl+F, flip with F, see it in 3D with Alt+3, measure
//! with M, check DRC with D, visual diff with G, DC IR-drop heatmap with I, tick off the BOM with
//! Shift+B (Esc clears), ask a local model about the project with ?, undo with Ctrl+Z, redo with
//! Ctrl+Y or Ctrl+Shift+Z, save with Ctrl+S (autosaved meanwhile; closing asks first).
//! `cuprita [--bench] [--screenshot out.png] [--diff [base]] [--ir [net]] [--mcp] [--json] [--serve [--port n] [--bind addr]] [board.kicad_pcb] ['{"cmd":...}' ...]`
//! `--bench` redraws continuously without vsync and prints fps; `--screenshot` saves the first
//! frame with the board (else Home) and exits; `--mcp` and `--json` run the headless agent
//! interface; `--serve` shows the project on a web page, to look at (`serve.rs`).

// On Windows, a window with no console beside it; the modes that print find one (`console`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
mod agent;
mod canvas;
#[cfg(not(target_arch = "wasm32"))]
mod chat;
mod chrome;
#[cfg(not(target_arch = "wasm32"))]
mod classes;
mod cmd;
mod corners;
mod dock;
mod docs;
mod drawing;
mod edit;
mod fab;
mod file;
mod grid;
mod guide;
mod help;
mod helpers;
mod home;
mod icon;
mod inspect;
#[cfg(not(target_arch = "wasm32"))]
mod lookup;
mod make;
mod motion;
#[cfg(not(target_arch = "wasm32"))]
mod newpart;
mod pair;
mod palette;
mod panels;
mod parts;
mod recent;
mod route;
#[cfg(not(target_arch = "wasm32"))]
mod serve;
mod settings;
#[cfg(not(target_arch = "wasm32"))]
mod share;
mod sheet;
mod sheet_edit;
mod shell;
#[cfg(not(target_arch = "wasm32"))]
mod sim;
mod theme;
mod three;
mod tr;
mod units;
mod watch;
#[cfg(target_arch = "wasm32")]
mod web;
mod zone;
mod zone_settings;

use crate::tr::{n, t, tr, trf};
use cuprita_core::analysis::IrDrop;
use cuprita_core::board::{Board, LayerId, LayerMask, NetId, Pt};
use cuprita_core::diff::Diff;
use cuprita_core::drc::{self, Violation};
use cuprita_core::history::{History, Pick};
use cuprita_core::library::LibPart;
use cuprita_core::models::Models;
use cuprita_render::{Orbit, Renderer, Scene};
use eframe::egui;
use eframe::egui_wgpu::RenderState;
use shell::Bench;
use std::path::{Path, PathBuf};
use web_time::Instant; // std's panics in the browser

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let (mut bench, mut mcp, mut json, mut screenshot, mut diff, mut ir, mut file, mut cmds) =
        (false, false, false, None, None, None, None, vec![]);
    let (mut serve, mut port, mut bind) = (false, 8080, String::from("127.0.0.1"));
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--help" | "-h" => {
                println!("{}", agent::usage());
                return Ok(());
            }
            // Not for a user: a simulation's run inside KiCad's Flatpak (`ngspice::inside`).
            #[cfg(not(target_arch = "wasm32"))]
            "--ngspice" => {
                cuprita_core::ngspice::helper();
                return Ok(());
            }
            "--bench" => bench = true,
            "--mcp" => mcp = true,
            "--json" => json = true,
            "--serve" => serve = true,
            "--port" => port = args.next().and_then(|p| p.parse().ok()).unwrap_or(port),
            "--bind" => bind = args.next().unwrap_or(bind),
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--diff" => {
                let next = args.next_if(|s| !s.starts_with("--") && !s.trim_start().starts_with('{')).unwrap_or_default();
                diff = Some(next);
            }
            "--ir" => {
                let next = args
                    .next_if(|s| !s.starts_with("--") && !s.trim_start().starts_with('{') && !Path::new(s).is_file())
                    .unwrap_or_default();
                ir = Some(next);
            }
            s if s.trim_start().starts_with('{') => cmds.push(a),
            _ => file = Some(PathBuf::from(a)),
        }
    }
    if file.is_none()
        && let Some(d) = diff.as_mut().filter(|d| !d.is_empty())
    {
        file = Some(PathBuf::from(std::mem::take(d)));
    }
    #[cfg(windows)]
    if mcp || json || bench || serve || !cmds.is_empty() {
        console();
    }
    if serve {
        let shown = file.ok_or("Which project? cuprita --serve project.kicad_pro".to_string()).and_then(|f| serve::run(&f, &bind, port));
        if let Err(why) = shown {
            eprintln!("{why}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mcp || json || !cmds.is_empty() {
        agent::run(mcp, file, cmds);
        return Ok(());
    }
    let viewport = egui::ViewportBuilder::default()
        .with_title("Cuprita")
        // A Flatpak's own id names its launcher there.
        .with_app_id(std::env::var("FLATPAK_ID").unwrap_or_else(|_| "cuprita".into()))
        .with_icon(theme::window_icon())
        .with_decorations(!chrome::own())
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([720.0, 480.0]);
    let mut options = eframe::NativeOptions { renderer: eframe::Renderer::Wgpu, viewport, ..Default::default() };
    if bench {
        options.wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
    }
    eframe::run_native("Cuprita", options, Box::new(move |cc| Ok(Box::new(App::new(cc, file, bench, screenshot, diff, ir)))))
}

/// A Windows window app starts without a console: what it prints goes to the console it was
/// started from, unless its output was handed over already (a pipe, as an MCP client's).
#[cfg(windows)]
fn console() {
    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> isize;
        fn AttachConsole(process: u32) -> i32;
    }
    // SAFETY: plain calls without pointers: -11 as u32 is STD_OUTPUT_HANDLE, u32::MAX is
    // ATTACH_PARENT_PROCESS. Without a console to attach to, nothing changes.
    unsafe {
        if matches!(GetStdHandle(-11i32 as u32), 0 | -1) {
            AttachConsole(u32::MAX);
        }
    }
}

struct App {
    gpu: RenderState,
    status: String,
    path: Option<PathBuf>,
    /// File name of the board, which a browser download keeps.
    name: String,
    board: Board,
    /// The text `board` was read from, which saving writes the edits into.
    src: String,
    /// The edits to undo and redo, and whether the board has any `src` lacks.
    history: History,
    /// When the board last changed, until it is autosaved.
    changed: Option<Instant>,
    /// When the last autosave of the board or its sheets was written, while edits stay unsaved.
    autosaved: Option<Instant>,
    /// How far the board is behind its schematic (`make.rs`).
    behind: make::Behind,
    /// The board whose schematic was looked for since it opened (`App::early_sch`).
    sought: Option<PathBuf>,
    /// What was last copied on a sheet (Ctrl+C), for Paste: any project's, as KiCad's clipboard text.
    clip: Option<String>,
    /// The question waiting for an answer; `closing` once it let the window close.
    ask: Option<file::Ask>,
    closing: bool,
    /// How `board` is drawn, kept for picking; the layers panel toggles `layers[i].visible`.
    scene: Option<Scene>,
    /// Highlighted net, 0 for none.
    net: u32,
    /// The palette (Ctrl+K): every command, part and net by name.
    palette: palette::Palette,
    /// Help (F1) while it is open.
    help: Option<help::Help>,
    settings: settings::Settings,
    /// Scene boxes around the footprints, DRC violation or diff change found last, marked on the canvas.
    found: Vec<[f32; 4]>,
    /// What a click or Find selected: a part (a drag moves it, R turns it), a track, a via or a
    /// zone (a drag on one of its handles moves that corner); Delete deletes it. `grab` is the
    /// scene point where a drag of what is picked, or of the selected zone's handle, started, with
    /// the item grabbed there.
    selected: Option<Pick>,
    grab: Option<([f32; 2], Pick)>,
    /// A setting being typed in Inspect: whose (a zone's, the tracks' picked: the first), the
    /// setting and the text.
    typing: Option<(Pick, String, String)>,
    /// The net class open in Inspect's list (`classes.rs`), and the value of it being typed.
    #[cfg(not(target_arch = "wasm32"))]
    class_open: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    class_typing: Option<(String, String)>,
    /// Why the last change asked of an item picked wasn't made (a setting typed in Inspect, a
    /// zone's corner dragged): Inspect says it while that item stays picked.
    refused: Option<(Pick, String)>,
    /// What is picked when it is several items (Shift+click, a box from `boxing`, a board
    /// point): `selected` is then `None`, and when it is set `picked` means nothing (`picks`).
    picked: Vec<Pick>,
    boxing: Option<Pt>,
    /// Route tool (`X`): a click on a pad, via or track starts a track; `route` is the one being drawn.
    routing: bool,
    route: Option<route::Route>,
    /// Whether other nets' tracks and vias make way for the tracks routed (KiCad's Shove), else
    /// they walk round them; P switches it, for the next tracks too.
    shove: bool,
    /// The Pair tool (`6`): a differential pair's two tracks routed side by side; `pair` is the
    /// one being drawn (`pair.rs`).
    pairing: bool,
    pair: Option<pair::PairRoute>,
    /// Whether a via dropped while routing joins only the layers the track goes between (blind
    /// or buried), else goes through the board; Shift+V switches it.
    blind: bool,
    /// The copper layer tracks are routed on, drawn above the other copper and picked first: the
    /// top copper when a board opens; PgUp, PgDn, + and - (KiCad's keys) or the Layers dock switch it.
    active: LayerId,
    /// Zone tool (`Z`): a click on a pad, via or track takes its net and layer, then clicks add
    /// corners; `zone` is the one being drawn.
    zoning: bool,
    zone: Option<zone::Draft>,
    /// With `zoning`: the Outline tool (`O`), drawing the board's outline instead of a zone, the
    /// Cutout tool (Shift+C), a hole in the selected zone, or the Line tool (`L`), lines on `ink`.
    outlining: bool,
    cutting: bool,
    lining: bool,
    /// The Text tool (Shift+T): the words being typed. The layer the Line and Text tools draw
    /// on, once chosen: else the silkscreen of the side shown (`drawing.rs`).
    writing: Option<String>,
    ink: Option<LayerId>,
    /// The docks beside the canvas (`dock.rs`): the tab each shows, and whether it is open.
    left: dock::Dock,
    right: dock::Dock,
    /// Add part (`A`, the Parts tab): the parts of KiCad's libraries (`parts`, read when it first
    /// shows; in the schematic, `symbols`) holding the words of `part_query`; `part_pick` is the
    /// match Enter adds. In the schematic, a part number is also looked up at Digi-Key (`lookup`).
    parts: Option<Vec<LibPart>>,
    symbols: Option<Vec<LibPart>>,
    part_query: String,
    part_pick: usize,
    #[cfg(not(target_arch = "wasm32"))]
    lookup: Option<lookup::Lookup>,
    /// New part… (the Parts tab, the palette): the part being made, while its window is open.
    #[cfg(not(target_arch = "wasm32"))]
    wizard: Option<newpart::Wizard>,
    /// Back-side view: mirrors X and reverses the board layer stackup.
    flipped: bool,
    /// The 3D workbench (Alt+3): the board seen from `orbit` (fitted when it opens), with
    /// the footprints' 3D `models` read so far and those being read (`loading`).
    orbit: Option<Orbit>,
    models: Models,
    loading: Option<std::thread::JoinHandle<Models>>,
    /// The files shown, which another program may write (`watch.rs`).
    watch: watch::Watch,
    /// The project's schematic, read when first shown in the Schematic workbench (Ctrl+E).
    sch: Option<sheet::Sch>,
    /// The workbench shown: Home, the schematic, the board in 2D or 3D, or what the fab needs;
    /// `on_sheet` when the canvas's place and zoom (`center`, `um_per_pt`) are the schematic's.
    bench: Bench,
    on_sheet: bool,
    /// Home's projects, pictures and the new project being made.
    home: home::Home,
    /// The chat about the project with a local model (`?`).
    #[cfg(not(target_arch = "wasm32"))]
    chat: chat::Chat,
    /// Ruler tool (`M`): first click sets start, second locks end (`(start, end, locked)` in µm).
    measuring: bool,
    ruler: Option<([f32; 2], [f32; 2], bool)>,
    /// DRC violations for `board`, marked on the canvas while the Checks tab shows (`D`).
    violations: Vec<Violation>,
    /// The connections still to route (`Board::ratsnest`), found again after each edit.
    ratsnest: Vec<cuprita_core::ratsnest::Link>,
    /// Visual diff (`(base, diff, scene)`); `show_diff` (`G`) toggles the panel and diff view.
    diff: Option<(String, Diff, Scene)>,
    show_diff: bool,
    /// DC IR-drop heatmap (`(result, scene)`), test current (A), optional source pad and `show_ir` (`I`).
    ir: Option<(IrDrop, Scene)>,
    ir_current: f64,
    ir_source: Option<String>,
    show_ir: bool,
    /// Bench checklist (`Shift+B`): `Board::bom` lines, which are ticked off and the selected one.
    bom: Vec<Vec<usize>>,
    ticked: Vec<bool>,
    bom_line: Option<usize>,
    show_bom: bool,
    /// Where the pointer is on the board or sheet shown, while it is over the canvas.
    pointer: Option<Pt>,
    /// What the point a click takes there snaps to (a pad, via, track or pin), named.
    snapped: Option<String>,
    /// The board's grid step: `grid::STEPS[grid]` (N steps it).
    grid: usize,
    /// Scene micrometres at the centre of the canvas.
    center: [f32; 2],
    um_per_pt: f32,
    /// Scene box to show in the canvas on the next frame.
    fit: Option<[f32; 4]>,
    /// The view on its way to a box asked for (`motion.rs`): its centre and scale where it came
    /// from and where it goes, and since when. `fresh`: the canvas shows something new (a board or
    /// sheet read, another page), which the next box asked for shows at once.
    glide: Option<(motion::Place, motion::Place, Instant)>,
    fresh: bool,
    /// The box the view was last fitted to and the canvas's size then, until the hand moves the
    /// view or it shows something else: fitted again when the canvas changes size.
    fitted: Option<([f32; 4], [f32; 2])>,
    /// When the workbench shown, or what it shows, last changed: the canvas fades in from then.
    arrived: Instant,
    /// What was last marked as found, and since when: a ring runs out from it as it is found.
    pinged: (Vec<[f32; 4]>, Instant),
    /// The 3D view on its way (tilting up as it arrives, turning over): the orbit it comes from
    /// and since when; and how fast it still turns once let go, in points a second.
    turning: Option<(Orbit, Instant)>,
    spin: [f32; 2],
    /// When the app started: the mark draws itself.
    started: Instant,
    /// First steps, shown open or folded, and whether as the walk-through (`guide.rs`).
    guide: Option<bool>,
    tour: bool,
    /// The documents of the board's folder the Project tab lists, and when they were read.
    #[cfg(not(target_arch = "wasm32"))]
    docs: Option<(PathBuf, Instant, Vec<PathBuf>)>,
    /// A served view (`cuprita --serve`, `web.rs`): the project shows to look at, the commands that
    /// only look run (`Cmd::looks`), and nothing edits it.
    viewer: bool,
    /// The project shared on its web page from this window (`share.rs`), while it is.
    #[cfg(not(target_arch = "wasm32"))]
    share: Option<share::Share>,
    /// `--bench`: redraws without a pause, counting frames.
    benchmark: bool,
    /// Frames counted since `window` started; `fps` is the last full second's count.
    frames: u32,
    window: Instant,
    fps: u32,
    screenshot: Option<PathBuf>,
}

impl App {
    fn new(
        cc: &eframe::CreationContext,
        file: Option<PathBuf>,
        benchmark: bool,
        screenshot: Option<PathBuf>,
        diff: Option<String>,
        ir: Option<String>,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        units::read();
        tr::read();
        motion::read(&cc.egui_ctx, benchmark || screenshot.is_some());
        let gpu = cc.wgpu_render_state.clone().expect("eframe runs with the wgpu renderer");
        let renderer = Renderer::new(&gpu.device, gpu.target_format);
        gpu.renderer.write().callback_resources.insert(renderer);
        let status = String::new();
        let mut app = Self {
            gpu,
            status,
            path: None,
            name: String::new(),
            board: Board::default(),
            src: String::new(),
            history: History::default(),
            changed: None,
            autosaved: None,
            behind: make::Behind::default(),
            sought: None,
            clip: None,
            ask: None,
            closing: false,
            scene: None,
            net: 0,
            palette: palette::Palette::default(),
            help: None,
            settings: settings::Settings::read(),
            found: Vec::new(),
            selected: None,
            grab: None,
            typing: None,
            #[cfg(not(target_arch = "wasm32"))]
            class_open: None,
            #[cfg(not(target_arch = "wasm32"))]
            class_typing: None,
            refused: None,
            picked: Vec::new(),
            boxing: None,
            routing: false,
            route: None,
            shove: true,
            blind: false,
            pairing: false,
            pair: None,
            active: 0,
            zoning: false,
            zone: None,
            outlining: false,
            cutting: false,
            lining: false,
            writing: None,
            ink: None,
            left: dock::Dock { tab: dock::Tab::Layers, open: true },
            right: dock::Dock { tab: dock::Tab::Inspect, open: true },
            parts: None,
            symbols: None,
            part_query: String::new(),
            part_pick: 0,
            #[cfg(not(target_arch = "wasm32"))]
            lookup: None,
            #[cfg(not(target_arch = "wasm32"))]
            wizard: None,
            flipped: false,
            orbit: None,
            models: Models::default(),
            loading: None,
            watch: watch::Watch::default(),
            sch: None,
            bench: Bench::Home,
            on_sheet: false,
            home: home::Home::new(),
            #[cfg(not(target_arch = "wasm32"))]
            chat: chat::Chat::default(),
            measuring: false,
            ruler: None,
            violations: Vec::new(),
            ratsnest: Vec::new(),
            diff: None,
            show_diff: false,
            ir: None,
            ir_current: 1.0,
            ir_source: None,
            show_ir: false,
            bom: Vec::new(),
            ticked: Vec::new(),
            bom_line: None,
            show_bom: false,
            pointer: None,
            snapped: None,
            grid: grid::DEFAULT,
            center: [0.0; 2],
            um_per_pt: 1.0,
            fit: None,
            glide: None,
            fresh: true,
            fitted: None,
            arrived: Instant::now(),
            pinged: (Vec::new(), Instant::now()),
            turning: None,
            spin: [0.0; 2],
            started: Instant::now(),
            guide: None,
            tour: false,
            #[cfg(not(target_arch = "wasm32"))]
            docs: None,
            viewer: false,
            #[cfg(not(target_arch = "wasm32"))]
            share: None,
            benchmark,
            frames: 0,
            window: Instant::now(),
            fps: 0,
            screenshot,
        };
        if let Some(f) = file {
            app.open(&opens(&f));
            if let Some(d) = diff {
                app.toggle_diff((!d.is_empty()).then_some(d.as_str()));
            }
            if let Some(n) = ir {
                app.toggle_ir((!n.is_empty()).then_some(n.as_str()));
            }
        }
        app
    }

    /// The copper layers shown in the layers panel, as they show from the top: the active layer,
    /// then the others from the side in view. A click picks on them in this order.
    pub(crate) fn shown_copper(&self) -> Vec<LayerId> {
        let shown = |l: LayerId| self.scene.as_ref().is_some_and(|s| s.layers.get(l as usize).is_some_and(|layer| layer.visible));
        let mut layers: Vec<LayerId> = self.board.stackup.iter().map(|c| c.layer).filter(|&l| shown(l)).collect();
        if self.flipped {
            layers.reverse();
        }
        if let Some(i) = layers.iter().position(|&l| l == self.active) {
            layers[..=i].rotate_right(1);
        }
        layers
    }

    /// The layers shown, copper or not: those a click or a box takes the board's own texts and
    /// drawings on.
    pub(crate) fn shown_layers(&self) -> LayerMask {
        let layers = self.scene.iter().flat_map(|s| s.layers.iter().enumerate());
        layers.filter(|(_, l)| l.visible).fold(0, |m, (i, _)| m | 1u128.checked_shl(i as u32).unwrap_or(0))
    }

    fn sync_gpu(&self) {
        let active = if self.sch_view() {
            self.sch.as_ref().map(|s| &s.scene)
        } else if self.show_diff {
            self.diff.as_ref().map(|(_, _, s)| s)
        } else if self.show_ir {
            self.ir.as_ref().map(|(_, s)| s)
        } else {
            self.scene.as_ref()
        };
        if let (Some(s), Some(r)) = (active, self.gpu.renderer.write().callback_resources.get_mut::<Renderer>()) {
            r.upload(&self.gpu.device, &self.gpu.queue, s);
            // Make shows the board in 3D too.
            if matches!(self.bench, Bench::ThreeD | Bench::Make) {
                r.upload3d(&self.gpu.device, &self.gpu.queue, &self.board, s, &self.models);
            }
        }
    }

    /// Shows the board in `src` with the design rules of the `.kicad_pro` text `pro`; `path` is
    /// the file it came from, which diff and saving need. `false` when `src` isn't a board.
    fn load(&mut self, path: Option<&Path>, name: &str, src: String, pro: Option<&str>) -> bool {
        let t = Instant::now();
        let mut board = match Board::parse(&src) {
            Ok(b) => b,
            Err(e) => {
                let (line, col) = e.line_col(&src);
                self.status = format!("{name}:{line}:{col}: {}", e.msg);
                return false;
            }
        };
        if let Some(pro) = pro {
            board.apply_pro(pro);
        }
        let parse = t.elapsed();
        let t = Instant::now();
        let scene = cuprita_render::build(&board);
        let build = t.elapsed();
        let t = Instant::now();
        let violations = drc::check(&board);
        let ratsnest = board.ratsnest();
        let drc_time = t.elapsed();
        let t = Instant::now();
        if let Some(r) = self.gpu.renderer.write().callback_resources.get_mut::<Renderer>() {
            r.upload(&self.gpu.device, &self.gpu.queue, &scene);
            if self.bench == Bench::ThreeD {
                r.upload3d(&self.gpu.device, &self.gpu.queue, &board, &scene, &Models::default());
            }
        }
        let upload = t.elapsed();
        let took = [parse, build, drc_time, upload].map(|t| format!("{t:.0?}"));
        self.status = trf(
            "{0} — parse {1}, scene {2}, drc {3}, upload {4} — {5}, {6}, {7}",
            &[
                &name,
                &took[0],
                &took[1],
                &took[2],
                &took[3],
                &count(board.tracks.len(), n!("track")),
                &count(board.vias.len(), n!("via")),
                &count(board.pads.len(), n!("pad")),
            ],
        );
        // A conversation about another project goes.
        #[cfg(not(target_arch = "wasm32"))]
        if self.path.as_deref() != path {
            self.chat.forget();
        }
        (self.path, self.name, self.src, self.history, self.changed) =
            (path.map(Path::to_path_buf), name.to_string(), src, History::default(), None);
        (self.diff, self.show_diff, self.ir, self.ir_source, self.show_ir) = (None, false, None, None, false);
        (self.bom, self.bom_line, self.selected, self.grab, self.route) = (board.bom(), None, None, None, None);
        (self.picked, self.boxing) = (Vec::new(), None);
        self.active = board.stackup.first().map_or(0, |c| c.layer);
        (self.fit, self.orbit, self.net, self.found, self.ruler, self.violations, self.board, self.scene) =
            (Some(whole(&board, &scene)), None, 0, Vec::new(), None, violations, board, Some(scene));
        self.ticked = self.kept_ticks();
        self.ratsnest = ratsnest;
        (self.models, self.loading) = (Models::default(), None);
        if self.bench == Bench::ThreeD {
            self.load_models();
        }
        // Another project's own parts are read again when the Parts tab wants them.
        (self.parts, self.symbols) = (None, None);
        // A schematic of another project goes; the board shows.
        if self.sch.as_ref().is_some_and(|s| Some(s.sheets.root.with_extension("kicad_pcb")) != self.path) {
            self.sch = None;
        }
        (self.bench, self.on_sheet) = (Bench::Board, false);
        self.arrive(true);
        // Home remembers what the user opens, not what a --screenshot or --bench run does.
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some(p), Some(s), None, false) = (path, &self.scene, &self.screenshot, self.benchmark) {
            recent::remember(&mut self.home.recent, p);
            let (bounds, visible) = (s.bounds, home::picture_layers(s.layers.iter().map(|l| &l.name)));
            self.keep_picture(p, bounds, &visible);
            self.keep_standing();
        }
        true
    }

    fn toggle_diff(&mut self, base: Option<&str>) {
        if self.scene.is_none() {
            return;
        }
        if !self.show_diff || base.is_some() {
            if self.diff.is_none() || base.is_some() {
                match load_base(self.path.as_deref(), base) {
                    Ok((label, old)) => {
                        let d = Diff::between(&old, &self.board);
                        let s = cuprita_render::build_diff(&old, &self.board, &d);
                        self.diff = Some((label, d, s));
                    }
                    Err(e) => return self.status = e,
                }
            }
            (self.show_diff, self.show_ir) = (true, false);
            self.right = dock::Dock { tab: dock::Tab::Changes, open: true };
        } else {
            self.show_diff = false;
        }
        self.sync_gpu();
    }

    fn update_ir(&mut self, rebuild_scene: bool) {
        if self.net == 0 {
            self.net = self.board.default_ir_net();
        }
        if self.net == 0 {
            return;
        }
        let res = self.board.ir_drop(self.net, self.ir_current, self.ir_source.as_deref(), None);
        let scene = match (rebuild_scene, self.ir.take()) {
            (false, Some((_, s))) => s,
            _ => cuprita_render::build_ir(&self.board, &res),
        };
        self.ir = Some((res, scene));
        if rebuild_scene {
            self.sync_gpu();
        }
    }

    fn toggle_ir(&mut self, net: Option<&str>) {
        if self.scene.is_none() {
            return;
        }
        if let Some(q) = net.filter(|q| !q.is_empty())
            && let Some(id) = self.board.find_net(q)
        {
            self.net = id;
        }
        if !self.show_ir || net.is_some() {
            (self.show_ir, self.show_diff, self.ir_source) = (true, false, None);
            self.right = dock::Dock { tab: dock::Tab::Heat, open: true };
            self.update_ir(true);
        } else {
            self.show_ir = false;
            self.sync_gpu();
        }
    }

    /// Zooms to the footprint or net `q` names: a footprint gets a marker, a net the highlight.
    pub(crate) fn find(&mut self, q: &str) {
        let Some(scene) = &self.scene else { return };
        match find_target(&self.board, scene, q) {
            Some(Found::Footprint { index, bounds, view, .. }) => {
                (self.found, self.fit, self.selected) = (vec![bounds], Some(view), Some(Pick::Footprint(index)))
            }
            Some(Found::Net { net, view, .. }) => {
                (self.net, self.found, self.fit) = (net, Vec::new(), view);
                if self.show_ir {
                    self.ir_source = None;
                    self.update_ir(true);
                }
            }
            None => self.status = trf("Nothing on the board is called {0}", &[&q]),
        }
    }
}

#[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "only the desktop agent reads every field"))]
pub(crate) enum Found {
    Footprint { index: usize, bounds: [f32; 4], view: [f32; 4] },
    Net { net: u32, bounds: Option<[f32; 4]>, view: Option<[f32; 4]> },
}

/// The add part panel's search field.
pub(crate) const ADD_FIELD: &str = "add part";

/// Puts the keyboard in text field `field`, which key `letter` opened: its letter doesn't land there.
pub(crate) fn focus(ctx: &egui::Context, field: &str, letter: &str) {
    ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t.eq_ignore_ascii_case(letter))));
    ctx.memory_mut(|m| m.request_focus(egui::Id::unique(field)));
}

pub(crate) fn find_target(board: &Board, scene: &Scene, q: &str) -> Option<Found> {
    let q = q.trim();
    if q.is_empty() {
        return None;
    }
    if let Some(index) = board.footprints.iter().position(|f| f.reference.eq_ignore_ascii_case(q)) {
        let b = um_box(scene, board.footprints[index].bounds);
        Some(Found::Footprint { index, bounds: b, view: around(b, 3.0) })
    } else if let Some(net) = board.find_net(q).or_else(|| pin_net(board, q)) {
        let b = board.net_bounds(net).map(|b| um_box(scene, b));
        Some(Found::Net { net, bounds: b, view: b.map(|b| around(b, 1.0)) })
    } else {
        None
    }
}

/// The net of the part's pin `q` names: `U17 pin 15`, `U17.15`, `U17-15`.
fn pin_net(board: &Board, q: &str) -> Option<NetId> {
    let (part, pin) = match q.to_ascii_lowercase().find(" pin ") {
        Some(i) => (&q[..i], &q[i + 5..]),
        None => q.split_once(['.', '-', ':', ' '])?,
    };
    let f = board.footprints.iter().find(|f| f.reference.eq_ignore_ascii_case(part.trim()))?;
    let p = board.pads[f.pads.clone()].iter().find(|p| p.number.eq_ignore_ascii_case(pin.trim()))?;
    (p.net != 0).then_some(p.net)
}

/// Loads the comparison base board from a file path or from git (`HEAD`, or the previous commit
/// that touched `board_path` when the working tree matches `HEAD`).
pub(crate) fn load_base(board_path: Option<&Path>, base: Option<&str>) -> Result<(String, Board), String> {
    let parse = |name: String, src: &str| {
        Board::parse(src).map(|b| (name.clone(), b)).map_err(|e| {
            let (line, col) = e.line_col(src);
            format!("{name}:{line}:{col}: {}", e.msg)
        })
    };
    if let Some(b) = base.filter(|b| Path::new(b).is_file()) {
        let p = Path::new(b);
        let name = p.file_name().map_or_else(|| b.to_string(), |n| n.to_string_lossy().into_owned());
        let src = std::fs::read_to_string(p).map_err(|e| format!("{name}: {e}"))?;
        return parse(name, &src);
    }
    let path = board_path.ok_or(tr("no board file"))?;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let file = path.file_name().ok_or(tr("bad file name"))?.to_string_lossy();
    let git = |rev: &str| -> Result<String, String> {
        // A revision is an agent's words too (`diff`'s base): never read as one of git's options.
        let out = crate::file::git(dir)
            .args(["show", "--end-of-options"])
            .arg(format!("{rev}:./{file}"))
            .output()
            .map_err(|e| format!("git: {e}"))?;
        if out.status.success() {
            String::from_utf8(out.stdout).map_err(|e| e.to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    };
    if let Some(rev) = base.filter(|b| !b.is_empty()) {
        return parse(rev.to_string(), &git(rev)?);
    }
    let head = git("HEAD")?;
    if std::fs::read_to_string(path).is_ok_and(|cur| cur == head)
        && let Ok(out) = crate::file::git(dir).args(["log", "-n1", "--skip=1", "--format=%h", "--"]).arg(format!("./{file}")).output()
        && let rev = String::from_utf8_lossy(&out.stdout).trim()
        && !rev.is_empty()
        && let Ok(prev) = git(rev)
    {
        return parse(rev.to_string(), &prev);
    }
    parse("HEAD".into(), &head)
}

/// Of the files dropped on the desktop, the one to open: the `.kicad_pcb`, else what a dropped
/// `.kicad_pro` `opens`, else the first file (a `.kicad_sch` opens the schematic; opening
/// anything else says what is wrong).
fn dropped_path(paths: &[&Path]) -> Option<PathBuf> {
    let find = |ext: &str| paths.iter().find(|p| p.extension().is_some_and(|e| e == ext));
    find("kicad_pcb").or_else(|| find("kicad_pro")).or(paths.first()).map(|p| opens(p))
}

/// The file opening `path` reads: a `.kicad_pro` stands for the board next to it (its schematic
/// without one); any other file for itself.
pub(crate) fn opens(path: &Path) -> PathBuf {
    if path.extension().is_none_or(|e| e != "kicad_pro") {
        return path.to_path_buf();
    }
    let (board, sheet) = (path.with_extension("kicad_pcb"), path.with_extension("kicad_sch"));
    if !board.exists() && sheet.exists() { sheet } else { board }
}

/// Name and contents of each file dropped in the browser.
#[cfg(any(test, target_arch = "wasm32"))]
type Files = Vec<(String, Result<Vec<u8>, String>)>;

/// Of the files dropped in the browser, the board to open (name, text) and the design rules of a
/// `.kicad_pro` dropped along with it; `Err` says what to drop instead.
#[cfg(any(test, target_arch = "wasm32"))]
fn dropped_board(files: Files) -> Result<(String, String, Option<String>), String> {
    let (pro, mut boards): (Vec<_>, Vec<_>) = files.into_iter().partition(|(name, _)| name.ends_with(".kicad_pro"));
    let pro = pro.into_iter().find_map(|(_, bytes)| String::from_utf8(bytes.ok()?).ok());
    // Of a whole project dropped at once, open the board.
    boards.sort_by_key(|(name, _)| !name.ends_with(".kicad_pcb"));
    let (name, bytes) = boards.into_iter().next().ok_or(tr("Drop the .kicad_pcb file and its .kicad_pro together"))?;
    match bytes.and_then(|b| String::from_utf8(b).map_err(|_| tr("not a text file; drop a .kicad_pcb board").into())) {
        Ok(text) => Ok((name, text, pro)),
        Err(e) => Err(format!("{name}: {e}")),
    }
}

/// `n` of a thing, in the language shown: "1 zone", "3 zones" (`forms` marked `n!`).
pub(crate) use cuprita_core::tr::count;

/// Distance and axis deltas between two scene-micrometre points.
fn ruler_text(a: [f32; 2], b: [f32; 2]) -> String {
    let (dx, dy) = (((b[0] - a[0]).abs() * 1000.0) as f64, ((b[1] - a[1]).abs() * 1000.0) as f64);
    format!("{}  dx {}  dy {}", units::len(dx.hypot(dy)), units::num(dx), units::num(dy))
}

/// What a selected zone's corners and keys do.
const ZONE_KEYS: &str = t!("drag a corner or an edge's middle, or inside to move it, R turns, Shift+C cuts out, Del deletes");

/// The selected part (reference, value, anchor in mm, rotation and the bottom), track (width),
/// via (size and drill) or zone (clearance and priority), with how to change it.
fn pick_text(b: &Board, pick: Pick) -> String {
    let name = pick_name(b, pick);
    match pick {
        Pick::Footprint(i) => {
            let f = &b.footprints[i];
            let (x, y, deg) = (units::num(f.at.x), units::len(f.at.y), (f.angle * 10.0).round() / 10.0);
            let text = if f.back {
                t!("{0} {1}  {2}, {3}  {4}°  bottom  (drag moves, R turns, F flips, Del deletes)")
            } else {
                t!("{0} {1}  {2}, {3}  {4}°  (drag moves, R turns, F flips, Del deletes)")
            };
            trf(text, &[&name, &f.value, &x, &y, &deg])
        }
        Pick::Track(i) => trf("{0}  {1} wide  (drag moves, R turns, Del deletes)", &[&name, &units::len(b.tracks[i].width)]),
        Pick::Via(i) => {
            let v = &b.vias[i];
            trf("{0}  {1}, drill {2}  (drag moves, Del deletes)", &[&name, &units::len(v.size), &units::len(v.drill)])
        }
        Pick::Zone(i) if b.zones[i].keepout => trf("{0}  keeps zone copper out  ({1})", &[&name, &tr(ZONE_KEYS)]),
        Pick::Zone(i) => {
            let z = &b.zones[i];
            trf("{0}  clearance {1}, priority {2}  ({3})", &[&name, &units::len(z.clearance), &z.priority, &tr(ZONE_KEYS)])
        }
        Pick::Text(i) if b.text_part(i).is_some() => {
            trf("{0}  {1} high  (drag moves, R turns, Del hides)", &[&name, &units::len(b.texts[i].size.1)])
        }
        Pick::Text(i) => trf("{0}  {1} high  (drag moves, R turns, Del deletes)", &[&name, &units::len(b.texts[i].size.1)]),
        Pick::Shape(i) => trf("{0}  {1} wide  (drag moves, R turns, Del deletes)", &[&name, &units::len(b.shapes[i].width)]),
    }
}

/// A part's reference, or a track, via or zone with its net (and layers), as DRC names them:
/// `Track (GND) on F.Cu`, `Via (GND)`, `Zone (GND) on F.Cu and B.Cu`, `Rule area on 4 layers`.
pub(crate) fn pick_name(b: &Board, pick: Pick) -> String {
    let net = |n: NetId| if n == 0 { String::new() } else { format!(" ({})", b.nets[n as usize]) };
    match pick {
        Pick::Footprint(i) => b.footprints[i].reference.clone(),
        Pick::Track(i) => trf("Track{0} on {1}", &[&net(b.tracks[i].net), &b.layers[b.tracks[i].layer as usize]]),
        Pick::Via(i) => trf("Via{0}", &[&net(b.vias[i].net)]),
        Pick::Zone(i) => {
            let z = &b.zones[i];
            let on: Vec<&str> = (0..b.layers.len()).filter(|&l| z.layers >> l & 1 != 0).map(|l| b.layers[l].as_str()).collect();
            let on = match on[..] {
                [a, b] => trf("{0} and {1}", &[&a, &b]),
                [_, _, _, ..] => count(on.len(), n!("layer")),
                _ => on.concat(),
            };
            trf(if z.keepout { t!("Rule area{0} on {1}") } else { t!("Zone{0} on {1}") }, &[&net(z.net), &on])
        }
        Pick::Text(i) => {
            let t = &b.texts[i];
            // Its first words: a text can run on for lines.
            let words: String = t.text.lines().next().unwrap_or_default().chars().take(24).collect();
            match b.text_part(i) {
                Some(f) => trf("Text \"{0}\" of {1}", &[&words, &b.footprints[f].reference]),
                None => trf("Text \"{0}\" on {1}", &[&words, &b.layers[t.layer as usize]]),
            }
        }
        Pick::Shape(i) => trf("{0} on {1}", &[&tr(drawing::kind(&b.shapes[i])), &b.layers[b.shapes[i].layer as usize]]),
    }
}

/// What is picked, in words: the one item (`pick_name`), or how many of each kind there are:
/// `3 parts, 12 tracks, 2 vias, 1 zone`.
pub(crate) fn picks_name(b: &Board, picks: &[Pick]) -> String {
    if let [one] = picks {
        return pick_name(b, *one);
    }
    let n = |kind: fn(&Pick) -> bool| picks.iter().filter(|&p| kind(p)).count();
    let kinds = [
        (n(|p| matches!(p, Pick::Footprint(_))), n!("part")),
        (n(|p| matches!(p, Pick::Track(_))), n!("track")),
        (n(|p| matches!(p, Pick::Via(_))), n!("via")),
        (n(|p| matches!(p, Pick::Zone(_))), n!("zone")),
        (n(|p| matches!(p, Pick::Text(_))), n!("text")),
        (n(|p| matches!(p, Pick::Shape(_))), n!("drawing")),
    ];
    kinds.iter().filter(|(k, _)| *k > 0).map(|&(k, one)| count(k, one)).collect::<Vec<_>>().join(", ")
}

/// Net inspector summary: total track length, width, IPC-2152 current, impedance and item counts.
fn net_text(board: &Board, net: u32) -> String {
    let name = board.nets.get(net as usize).map_or("", String::as_str);
    let i = board.inspect_net(net);
    let mut s = trf("net {0}  {1}", &[&name, &units::len(i.length_mm * 1e6)]);
    if i.width.1 > 0 {
        let (w0, w1) = i.width;
        let w = if w0 == w1 { units::len(w0) } else { format!("{}–{}", units::num(w0), units::len(w1)) };
        s += &format!("  {}  {:.2} A", trf("w {0}", &[&w]), i.current_a);
    }
    if i.impedance_ohm.1 > 0.0 {
        let (z0, z1) = i.impedance_ohm;
        if (z1 - z0).abs() < 0.05 {
            s += &format!("  {z0:.1} Ω");
        } else {
            s += &format!("  {z0:.1}–{z1:.1} Ω");
        }
    }
    s + &format!("  ({}, {}, {})", count(i.pads, n!("pad")), count(i.tracks, n!("track")), count(i.vias, n!("via")))
}

/// What Update from schematic (F8) does, as its buttons say.
pub(crate) const UPDATE_HINT: &str = "Adds the parts of the schematic the board lacks, below it, and puts every part's pads on the schematic's nets, as KiCad's Update PCB from Schematic";

/// Board box to scene micrometres, as `[x0, y0, x1, y1]`.
pub(crate) fn um_box(scene: &Scene, (lo, hi): (Pt, Pt)) -> [f32; 4] {
    let ([x0, y0], [x1, y1]) = (scene.um(lo), scene.um(hi));
    [x0, y0, x1, y1]
}

/// The board's outline grown by every part's box, in scene µm: what the view fits, parts F8 put
/// below the board included (KiCad's Zoom to Fit takes in every item too).
pub(crate) fn whole(board: &Board, scene: &Scene) -> [f32; 4] {
    let grown = |[a, b, c, d]: [f32; 4], [e, f, g, h]: [f32; 4]| [a.min(e), b.min(f), c.max(g), d.max(h)];
    board.footprints.iter().map(|f| um_box(scene, f.bounds)).fold(scene.bounds, grown)
}

/// A square around the centre of `b`, `k` times its size and at least 5 mm across.
pub(crate) fn around([x0, y0, x1, y1]: [f32; 4], k: f32) -> [f32; 4] {
    let (cx, cy, half) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, ((x1 - x0).max(y1 - y0) * k / 2.0).max(2500.0));
    [cx - half, cy - half, cx + half, cy + half]
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        if self.benchmark && self.window.elapsed().as_secs_f32() >= 1.0 {
            (self.fps, self.frames, self.window) = (self.frames, 0, Instant::now());
            println!("{} fps", self.fps);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
            if parts::are_parts(&dropped) {
                self.import_parts(ui.ctx(), &dropped);
            } else if let Some(path) = dropped_path(&dropped.iter().map(PathBuf::as_path).collect::<Vec<_>>()) {
                self.leave(move |app| app.open(&path));
            }
        }
        #[cfg(target_arch = "wasm32")]
        web::poll(self, ui.ctx());
        // Closing with unsaved edits asks first; in the browser, the page does (`web.rs`).
        if ui.input(|i| i.viewport().close_requested()) && self.unsaved() && !self.closing {
            let ctx = ui.ctx().clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.leave(move |app| {
                app.closing = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            });
        }
        self.ask_dialog(ui.ctx());
        self.home_answers(ui.ctx());
        self.autosave(ui.ctx());
        self.follow_files(ui.ctx());
        self.early_sch(ui.ctx());
        self.refresh_behind(ui.ctx());
        #[cfg(target_arch = "wasm32")]
        web::UNSAVED.set(self.unsaved());
        self.keys(ui.ctx());
        if !self.measuring {
            self.ruler = None;
        }
        if !self.routing {
            self.route = None;
        }
        if !self.zoning {
            self.zone = None;
        }
        self.header(ui);
        self.status_bar(ui);
        self.palette(ui.ctx());
        self.help(ui.ctx());
        self.settings(ui.ctx());
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.wizard(ui.ctx());
            self.share_panel(ui.ctx());
        }
        self.docks(ui);
        let page = matches!(self.bench, Bench::Home | Bench::Make);
        egui::CentralPanel::no_frame().frame(egui::Frame::NONE.fill(if page { theme::GROUND } else { theme::CANVAS })).show(ui, |ui| {
            let rect = ui.max_rect();
            match self.bench {
                Bench::Home => self.home_page(ui),
                Bench::Schematic => self.canvas_sch(ui),
                Bench::Board => self.canvas(ui),
                Bench::ThreeD => self.canvas3d(ui),
                Bench::Make => self.make_page(ui),
            }
            if !page {
                self.tools(ui.ctx(), rect);
                self.tool_hint(ui.ctx(), rect);
                if !self.viewer {
                    self.empty_hint(ui.ctx(), rect);
                }
                self.guide(ui.ctx(), rect);
            }
            if self.sch_view() {
                self.sheet_path(ui.ctx(), rect);
            }
        });
        if chrome::own() {
            chrome::edges(ui.ctx());
        }
        if self.benchmark {
            ui.ctx().request_repaint();
        }
        if let (Some(path), true) = (&self.screenshot, self.scene.is_some() || self.sch_view() || self.bench == Bench::Home) {
            let (path, ctx) = (path.clone(), ui.ctx().clone());
            self.screenshot = None;
            ui.ctx().request_screenshot(move |img| {
                if let Err(e) = save_png(&path, img.size[0] as u32, img.size[1] as u32, img.as_raw()) {
                    eprintln!("{}: {e}", path.display());
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint();
            });
            ui.ctx().request_repaint();
        }
    }
}

/// sRGB floats to egui's colour.
fn color32(c: [f32; 4]) -> egui::Color32 {
    let [r, g, b, a] = c.map(|c| (c * 255.0) as u8);
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

pub(crate) fn save_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut png = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path)?), w, h);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(rgba)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_ruler_in_mm_and_mil() {
        assert_eq!(ruler_text([1000.0, 2000.0], [4000.0, -2000.0]), "5 mm  dx 3  dy 4");
        assert_eq!(ruler_text([0.0, 0.0], [2540.0, 0.0]), "2.54 mm  dx 2.54  dy 0");
    }

    #[test]
    fn opens_the_dropped_board_with_its_rules() {
        let file = |name: &str, text: &str| (name.to_string(), Ok(text.as_bytes().to_vec()));
        let rules = r#"{"rules": {"min_track_width": 0.1}}"#;
        let project = vec![file("a.kicad_pro", rules), file("a.kicad_sch", "(kicad_sch)"), file("a.kicad_pcb", "(kicad_pcb)")];
        assert_eq!(dropped_board(project), Ok(("a.kicad_pcb".into(), "(kicad_pcb)".into(), Some(rules.into()))));
        assert_eq!(dropped_board(vec![file("a.kicad_pro", rules)]), Err("Drop the .kicad_pcb file and its .kicad_pro together".into()));
        let zip = vec![("b.zip".to_string(), Ok(vec![0xFF]))];
        assert_eq!(dropped_board(zip), Err("b.zip: not a text file; drop a .kicad_pcb board".into()));
    }

    #[test]
    fn opens_the_board_of_a_desktop_drop() {
        let drop = |names: &[&str]| dropped_path(&names.iter().map(Path::new).collect::<Vec<_>>());
        assert_eq!(drop(&["/p/a.kicad_sch", "/p/a.kicad_pcb", "/p/a.kicad_pro"]), Some("/p/a.kicad_pcb".into()));
        // A project file stands for the board next to it, also given alone (on the command line).
        assert_eq!(drop(&["/p/a.kicad_pro", "/p/a.kicad_sch"]), Some("/p/a.kicad_pcb".into()));
        assert_eq!(
            (opens(Path::new("/p/a.kicad_pro")), opens(Path::new("/p/a.kicad_sch"))),
            ("/p/a.kicad_pcb".into(), "/p/a.kicad_sch".into())
        );
        assert_eq!((drop(&["/p/notes.txt"]), drop(&[])), (Some("/p/notes.txt".into()), None));
        // A schematic alone opens as one.
        assert_eq!(drop(&["/p/a.kicad_sch"]), Some("/p/a.kicad_sch".into()));
    }

    #[test]
    fn formats_the_selected_part_track_via_or_zone() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "GND")
              (footprint "R" (layer "B.Cu") (at 10.5 -2 33.33) (property "Reference" "R7") (property "Value" "4k7"))
              (segment (start 0 0) (end 1 0) (width 0.25) (layer "B.Cu") (net 0))
              (via (at 5 5) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1))
              (zone (net 1) (layers "F.Cu" "B.Cu") (priority 2) (connect_pads (clearance 0.3)) (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))
              (zone (net 0) (layers "*.Cu") (keepout (copperpour not_allowed)) (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1)))))"#,
        )
        .unwrap();
        assert_eq!(
            [Pick::Footprint(0), Pick::Track(0), Pick::Via(0), Pick::Zone(0), Pick::Zone(1)].map(|p| pick_text(&b, p)),
            [
                "R7 4k7  10.5, -2 mm  33.3°  bottom  (drag moves, R turns, F flips, Del deletes)",
                "Track on B.Cu  0.25 mm wide  (drag moves, R turns, Del deletes)",
                "Via (GND)  0.8 mm, drill 0.4 mm  (drag moves, Del deletes)",
                "Zone (GND) on F.Cu and B.Cu  clearance 0.3 mm, priority 2  (drag a corner or an edge's middle, or inside to move it, R turns, Shift+C cuts out, Del deletes)",
                "Rule area on 4 layers  keeps zone copper out  (drag a corner or an edge's middle, or inside to move it, R turns, Shift+C cuts out, Del deletes)"
            ]
        );
        // Several picked: how many of each; one: what it is.
        let all = [Pick::Footprint(0), Pick::Track(0), Pick::Via(0), Pick::Zone(0), Pick::Zone(1)];
        let said = [&all[..], &all[1..2], &all[3..]].map(|p| picks_name(&b, p));
        assert_eq!(said, ["1 part, 1 track, 1 via, 2 zones", "Track on B.Cu", "2 zones"]);
    }

    #[test]
    fn formats_net_inspector_summary() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
              (setup (stackup
                (layer "F.Cu" (type "copper") (thickness 0.035))
                (layer "dielectric 1" (type "core") (thickness 0.2104) (epsilon_r 4.6))
                (layer "B.Cu" (type "copper") (thickness 0.035))))
              (net 0 "") (net 1 "RF")
              (segment (start 0 0) (end 2.54 0) (width 0.29337) (layer "F.Cu") (net 1)))"#,
        )
        .unwrap();
        assert_eq!(net_text(&b, 1), "net RF  2.54 mm  w 0.293 mm  1.01 A  54.1 Ω  (0 pads, 1 track, 0 vias)");
    }
}
