//! Every command a user can give, once: its name, icon, keys and what it does. Keys and buttons
//! run them all through `App::run`, so a command acts the same however it is given.

use crate::dock::{Dock, Tab};
use crate::shell::Bench;
use crate::tr::{t, tr};
use crate::{App, UPDATE_HINT, icon, picks_name, theme};
use cuprita_core::history::Pick;
use cuprita_core::schematic::LabelKind;
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Cmd {
    Home,
    New,
    Open,
    Save,
    Undo,
    Redo,
    Find,
    Fab,
    Share,
    Pdf,
    ImportPart,
    NewPart,
    Schematic,
    Board,
    ThreeD,
    Make,
    Update,
    Flip,
    Measure,
    Route,
    Zone,
    Outline,
    Line,
    Text,
    Add,
    Fill,
    Checks,
    Changes,
    Heat,
    Bom,
    Ask,
    Wire,
    Label,
    GlobalLabel,
    NoConnect,
    Simulate,
    Turn,
    TurnBack,
    Delete,
    Fit,
    Guide,
    TopLayer,
    BottomLayer,
    NextLayer,
    PrevLayer,
    WiderTrack,
    NarrowerTrack,
    SwapBend,
    GridNext,
    GridBack,
    NudgeLeft,
    NudgeRight,
    NudgeUp,
    NudgeDown,
    FlipPart,
    TidyTexts,
    Units,
    Tour,
    Cutout,
    Select,
    HierLabel,
    Sheet,
    Bus,
    BusEntry,
    MirrorX,
    MirrorY,
    Copy,
    Paste,
    Duplicate,
    Run,
    Shove,
    BlindVias,
    Pair,
    Help,
    Settings,
}

use Cmd::*;

/// A command's name, icon, keys (the first is the one shown) and what it does.
pub(crate) struct About {
    pub(crate) name: &'static str,
    pub(crate) icon: &'static str,
    keys: &'static [(Modifiers, Key)],
    pub(crate) hint: &'static str,
}

const NONE: Modifiers = Modifiers::NONE;
const CTRL: Modifiers = Modifiers::COMMAND;
const SHIFT: Modifiers = Modifiers::SHIFT;
const CTRL_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

impl Cmd {
    pub(crate) const ALL: [Cmd; 75] = [
        Home,
        New,
        Open,
        Save,
        Undo,
        Redo,
        Find,
        Fab,
        Share,
        Pdf,
        ImportPart,
        NewPart,
        Schematic,
        Board,
        ThreeD,
        Make,
        Update,
        Flip,
        Measure,
        Route,
        Zone,
        Outline,
        Line,
        Text,
        Add,
        Fill,
        Checks,
        Changes,
        Heat,
        Bom,
        Ask,
        Wire,
        Label,
        GlobalLabel,
        NoConnect,
        Simulate,
        Turn,
        TurnBack,
        Delete,
        Fit,
        Guide,
        TopLayer,
        BottomLayer,
        NextLayer,
        PrevLayer,
        WiderTrack,
        NarrowerTrack,
        SwapBend,
        GridNext,
        GridBack,
        NudgeLeft,
        NudgeRight,
        NudgeUp,
        NudgeDown,
        FlipPart,
        TidyTexts,
        Units,
        Tour,
        Cutout,
        Select,
        HierLabel,
        Sheet,
        Bus,
        BusEntry,
        MirrorX,
        MirrorY,
        Copy,
        Paste,
        Duplicate,
        Run,
        Shove,
        BlindVias,
        Pair,
        Help,
        Settings,
    ];

    /// Whether it only looks, changing nothing of the project: what a served view offers
    /// (`App::viewer`).
    pub(crate) fn looks(self) -> bool {
        let layers = matches!(self, TopLayer | BottomLayer | NextLayer | PrevLayer | GridNext | GridBack);
        layers
            || matches!(self, Find | Help | Settings | Units | Checks | Schematic | Board | ThreeD | Flip | Fit | Select | Measure | Heat)
    }

    /// Its name, icon, keys and hint, in the language shown.
    pub(crate) fn about(self) -> About {
        let about = self.about_in(cfg!(target_arch = "wasm32"));
        About { name: tr(about.name), hint: tr(about.hint), ..about }
    }

    /// Its name, icon, keys and hint in English, in the browser (`web`) or on the desktop.
    pub(crate) fn about_in(self, web: bool) -> About {
        let (name, icon, keys, hint): (_, _, &'static [(Modifiers, Key)], _) = match self {
            Home => ("Home", icon::HOME, &[], "New, open and recent projects, and KiCad's examples"),
            New => (
                "New project",
                icon::NEW,
                &[(CTRL, Key::N)],
                "A new project: a blank board of the size you want, or one of KiCad's templates",
            ),
            Open => ("Open…", icon::OPEN, &[(CTRL, Key::O)], "Opens a KiCad project: its .kicad_pro, .kicad_pcb or .kicad_sch"),
            Save if web => ("Save", icon::SAVE, &[(CTRL, Key::S)], "Downloads the board with its edits"),
            Save => (
                "Save",
                icon::SAVE,
                &[(CTRL, Key::S)],
                "Writes the edits into the board and sheet files; each previous version stays as <file>-bak",
            ),
            Undo => ("Undo", icon::UNDO, &[(CTRL, Key::Z)], "Takes back the last edit"),
            Redo => ("Redo", icon::REDO, &[(CTRL, Key::Y), (CTRL_SHIFT, Key::Z)], "Puts back the last edit undone"),
            Find => (
                "Find or run…",
                icon::SEARCH,
                &[(CTRL, Key::K), (CTRL, Key::F)],
                "Every command, part and net by a few letters of its name: Enter runs the command or shows the part or net",
            ),
            Fab if web => (
                "Fab files",
                icon::EXPORT,
                &[(CTRL, Key::P)],
                "Fills the zones, then downloads <board>-gerbers.zip (Gerbers and drills), -bom.csv and -positions.csv, which JLCPCB, PCBWay and OSH Park take as they are",
            ),
            Fab => (
                "Fab files",
                icon::EXPORT,
                &[(CTRL, Key::P)],
                "Fills the zones, then writes <board>-gerbers.zip (Gerbers and drills), -bom.csv and -positions.csv next to the board, which JLCPCB, PCBWay and OSH Park take as they are",
            ),
            Share => (
                "Share",
                icon::SHARE,
                &[],
                "Shows the project on a web page a phone or another computer opens by its QR code or address, to look at; again hides the code",
            ),
            ImportPart => (
                "Add part files…",
                icon::OPEN,
                &[],
                "Adds a part's KiCad files, or the zip SnapMagic, Ultra Librarian or SamacSys give, to the project's own library, which KiCad reads too",
            ),
            NewPart => (
                "New part…",
                icon::NEW_PART,
                &[],
                "Makes a part of your own: its symbol from its pins, its footprint from a package's numbers or the libraries', into the project's own library, which KiCad reads too",
            ),
            Pdf => (
                "Schematic PDF",
                icon::FILE,
                &[(CTRL_SHIFT, Key::P)],
                "Writes every sheet into one PDF beside the project, as KiCad numbers them, each with a frame and title block, and opens it",
            ),
            Schematic => (
                "Schematic",
                icon::SCHEMATIC,
                &[(CTRL, Key::E), (CTRL, Key::Num1)],
                "The project's schematic, read from the .kicad_sch beside the board; the part selected stays selected",
            ),
            Board => ("Board", icon::BOARD, &[(CTRL, Key::E), (CTRL, Key::Num2)], "The board; the part selected stays selected"),
            ThreeD => (
                "3D",
                icon::THREE_D,
                &[(Modifiers::ALT, Key::Num3), (CTRL, Key::Num3)],
                "The board in 3D: drag turns it, right drag slides it, the wheel zooms, F turns it over",
            ),
            Make => ("Make", icon::MAKE, &[(CTRL, Key::Num4)], "What the fab needs: the checks left, the fab files, assembling by hand"),
            Update => ("Update board", icon::UPDATE, &[(NONE, Key::F8)], UPDATE_HINT),
            Flip => ("Flip view", icon::FLIP, &[(NONE, Key::F)], "Shows the board from the back (F, unless only parts are picked)"),
            Measure => (
                "Measure",
                icon::MEASURE,
                &[(NONE, Key::M)],
                "Click two points: the distance between them in mm and mil, and along each axis",
            ),
            Route => {
                ("Route", icon::ROUTE, &[(NONE, Key::X)], "Draws a track from a pad, via or track, around other nets with the clearances")
            }
            Zone => {
                ("Zone", icon::ZONE, &[(NONE, Key::Z)], "Draws a copper zone on the net and layer of the pad, via or track clicked first")
            }
            Outline => (
                "Outline",
                icon::OUTLINE,
                &[(NONE, Key::O)],
                "Draws the board's outline on Edge.Cuts: drag a rectangle, or click its corners",
            ),
            Line => (
                "Line",
                icon::LINE,
                &[(NONE, Key::L)],
                "Draws lines on the silkscreen, or the layer chosen: click their corners, or drag a rectangle",
            ),
            Text => (
                "Text",
                icon::TEXT,
                &[(SHIFT, Key::T)],
                "Writes on the silkscreen, or the layer chosen: type the words, Enter puts them in the middle of the view, picked to drag where they go",
            ),
            Add => (
                "Add part",
                icon::PART,
                &[(NONE, Key::A)],
                "Adds a part from KiCad's libraries: a footprint on the board, a symbol in the schematic",
            ),
            Fill => ("Fill zones", icon::FILL, &[(NONE, Key::B)], "Fills every copper zone again, as KiCad's Fill All Zones"),
            Checks => (
                "Checks",
                icon::CHECKS,
                &[(NONE, Key::D)],
                "The board's design rule checks, the schematic's electrical rule checks, in plain words: hover one for its fix, click to see it",
            ),
            Changes => {
                ("Changes", icon::CHANGES, &[(NONE, Key::G)], "What changed since the board's last commit in git: added green, removed red")
            }
            Heat => (
                "IR drop",
                icon::HEAT,
                &[(NONE, Key::I)],
                "A DC IR-drop heatmap of the net selected (else the main power net) at a test current",
            ),
            Bom => (
                "BOM checklist",
                icon::BOM,
                &[(SHIFT, Key::B)],
                "Ticks off the parts as you place them by hand: a line marks its parts on the board",
            ),
            Ask => (
                "Ask",
                icon::ASK,
                &[(NONE, Key::Questionmark)],
                "Ask a local model (Ollama) about this project: it looks up its parts, nets, DRC, ERC, simulation, files and datasheets",
            ),
            Wire => ("Wire", icon::WIRE, &[(NONE, Key::W)], "Draws wires: a click starts one, each click after adds to it"),
            Label => ("Label", icon::LABEL, &[(NONE, Key::L)], "Names a net: type its name, then click a pin or wire"),
            GlobalLabel => (
                "Global label",
                icon::GLOBAL_LABEL,
                &[(CTRL, Key::L)],
                "Names a net on every sheet: type its name, then click a pin or wire",
            ),
            NoConnect => (
                "No connect",
                icon::NO_CONNECT,
                &[(NONE, Key::Q)],
                "Flags a pin left unconnected on purpose, so ERC lets it be: a click flags a pin, a click on a flag takes it away",
            ),
            Simulate => (
                "Simulate",
                icon::SIMULATE,
                &[(NONE, Key::S)],
                "Simulates the schematic in ngspice as KiCad's simulator does: its parts' models, directives and workbook; S again stops it",
            ),
            Turn => (
                "Turn",
                icon::TURN,
                &[(NONE, Key::R)],
                "Turns what is picked (several about their middle), or the symbol being placed, 90° counter-clockwise",
            ),
            TurnBack => ("Turn back", icon::TURN, &[(SHIFT, Key::R)], "Turns what is picked 90° clockwise"),
            Delete => (
                "Delete",
                icon::DELETE,
                &[(NONE, Key::Delete), (NONE, Key::Backspace)],
                "Deletes what is picked: parts, tracks, vias, zones, texts and drawings; in the schematic, symbols, wires and labels",
            ),
            Fit => (
                "Zoom to fit",
                icon::FIT,
                &[(NONE, Key::Home)],
                "Shows the whole board with its parts (those beside it too), sheet or 3D view",
            ),
            Guide => (
                "First steps",
                icon::PASSES,
                &[],
                "The way from an empty schematic to the fab's files, each step ticked off as the project gets there",
            ),
            TopLayer => ("Top layer", icon::LAYERS, &[(NONE, Key::PageUp)], LAYER_HINT),
            BottomLayer => ("Bottom layer", icon::LAYERS, &[(NONE, Key::PageDown)], LAYER_HINT),
            NextLayer => ("Next layer", icon::LAYERS, &[(NONE, Key::Plus)], LAYER_HINT),
            PrevLayer => ("Previous layer", icon::LAYERS, &[(NONE, Key::Minus)], LAYER_HINT),
            WiderTrack => (
                "Wider track",
                icon::ROUTE,
                &[(NONE, Key::W)],
                "Steps the track being routed to the next wider of the project's track widths",
            ),
            NarrowerTrack => (
                "Narrower track",
                icon::ROUTE,
                &[(SHIFT, Key::W)],
                "Steps the track being routed to the next narrower of the project's track widths, never below the rules' minimum",
            ),
            SwapBend => (
                "Swap bend",
                icon::ROUTE,
                &[(NONE, Key::Slash)],
                "Bends the track being routed the other way round: where the way is clear, its diagonal comes first or last; turns the corner of the wire being drawn the other way",
            ),
            GridNext => (
                "Next grid",
                icon::GRID,
                &[(NONE, Key::N)],
                "The board's next grid step: 1, 0.5, 0.25, 0.1, 0.05 mm, then 100, 50, 25, 10, 5 mil, and round again",
            ),
            GridBack => ("Previous grid", icon::GRID, &[(SHIFT, Key::N)], "The board's grid step before, the other way round the list"),
            NudgeLeft => ("Nudge left", icon::PART, &[(NONE, Key::ArrowLeft)], "Moves what is picked one grid step left"),
            NudgeRight => ("Nudge right", icon::PART, &[(NONE, Key::ArrowRight)], "Moves what is picked one grid step right"),
            NudgeUp => ("Nudge up", icon::PART, &[(NONE, Key::ArrowUp)], "Moves what is picked one grid step up"),
            NudgeDown => ("Nudge down", icon::PART, &[(NONE, Key::ArrowDown)], "Moves what is picked one grid step down"),
            FlipPart => (
                "Flip part",
                icon::FLIP,
                &[(NONE, Key::F)],
                "Puts the parts picked on the other side of the board, mirrored together, as KiCad's Flip does",
            ),
            TidyTexts => (
                "Tidy texts",
                icon::TIDY,
                &[(NONE, Key::T)],
                "Puts each part's reference where it reads: upright, 1 mm high, just outside the part, off pads, other silkscreen and the board's edge, else hidden; and its value inside the part on the Fab layer. The parts picked, else all. On a sheet, each symbol's reference and value beside it, clear of its pins, wires and labels",
            ),
            Units => (
                "Units: mm or mil",
                icon::MEASURE,
                &[(CTRL, Key::U)],
                "Shows lengths in mils, or in millimetres again; files keep KiCad's millimetres",
            ),
            Tour => (
                "First board",
                icon::PASSES,
                &[],
                "A walk-through: light an LED from a coin cell, step by step from the schematic to the fab's files",
            ),
            Cutout => ("Cutout", icon::CUTOUT, &[(SHIFT, Key::C)], "Cuts a hole in the selected zone: click the hole's corners inside it"),
            // Esc does it too, step by step, in the canvas (a key here would take Esc from the tools).
            Select => ("Select", icon::SELECT, &[], "Stops the tool on, the work in hand with it, to pick and move things again (Esc)"),
            Run => (
                "Pick the run",
                icon::ROUTE,
                &[(NONE, Key::U)],
                "Picks the whole run of the tracks picked: their segments end to end, up to pads, vias and branches (KiCad's U)",
            ),
            HierLabel => (
                "Hierarchical label",
                icon::HIER_LABEL,
                &[(NONE, Key::H)],
                "On a sheet below the root, joins a net to the sheet above through the pin of its name, which the sheet's box gets: type its name, then click a pin or wire",
            ),
            MirrorX => (
                "Mirror",
                icon::FLIP,
                &[(NONE, Key::X)],
                "Mirrors what is picked left to right, in place as KiCad's Mirror Horizontally: a pin that takes another's place takes its wires",
            ),
            MirrorY => (
                "Mirror up and down",
                icon::FLIP_UP,
                &[(NONE, Key::Y)],
                "Mirrors what is picked top to bottom, in place as KiCad's Mirror Vertically",
            ),
            Copy => ("Copy", icon::COPY, &[(CTRL, Key::C)], "Copies what is picked, for Paste here or in KiCad"),
            Paste => (
                "Paste",
                icon::PASTE,
                &[(CTRL, Key::V)],
                "Pastes what was copied, here or in KiCad: it follows the pointer until a click puts it down, its parts numbered on",
            ),
            Duplicate => {
                ("Duplicate", icon::DUPLICATE, &[(CTRL, Key::D)], "A copy of what is picked follows the pointer until a click puts it down")
            }
            Sheet => (
                "Sheet",
                icon::SHEET,
                &[(SHIFT, Key::S)],
                "Adds a sheet below this one: drag its box, then give its name and file (a new one, or one of the project's to show again); a double click opens it",
            ),
            Bus => (
                "Bus",
                icon::BUS,
                &[(NONE, Key::B)],
                "Draws buses as W draws wires; a label names its members (D[0..7], {SDA SCL}) and bus entries (Z) join wires to it",
            ),
            BusEntry => (
                "Bus entry",
                icon::BUS_ENTRY,
                &[(NONE, Key::Z)],
                "Joins a wire to a bus at 45°: one follows the pointer on the grid, R turns it, each click puts one down",
            ),
            Shove => (
                "Shove or walk round",
                icon::ROUTE,
                &[(NONE, Key::P)],
                "While routing, other nets' tracks and vias make way for the track (KiCad's Shove), or it walks round them",
            ),
            BlindVias => (
                "Blind vias",
                icon::VIA,
                &[(SHIFT, Key::V)],
                "While routing, a via joins only the layers the track goes between (blind or buried: ask the fab what they cost), or goes through the board",
            ),
            Pair => (
                "Route pair",
                icon::PAIR,
                &[(NONE, Key::Num6)],
                "Routes a differential pair: from a pad of one of its nets (D+ and D-, CLK_P and CLK_N), its two tracks side by side, as wide and as far apart as their net class says",
            ),
            Help => (
                "Help",
                icon::HELP,
                &[(NONE, Key::F1)],
                "Every command with its keys and what it does, by where it works; the walk-through to a first board",
            ),
            Settings if web => ("Settings", icon::SETTINGS, &[(CTRL, Key::Comma)], "The units lengths show in, and the language"),
            Settings => (
                "Settings",
                icon::SETTINGS,
                &[(CTRL, Key::Comma)],
                "The units lengths show in, the language, and your own Digi-Key key, which finds parts by number",
            ),
        };
        About { name, icon, keys, hint }
    }
}

/// A key as its keycap reads: `Ctrl+K`, `F1`, `/`, `?`, `+`, `Shift+W`, `Left` (egui names
/// punctuation, `Slash`, off a Mac).
pub(crate) fn key_text(ctx: &egui::Context, k: &KeyboardShortcut) -> String {
    let (text, cap) = (ctx.format_shortcut(k), k.logical_key.symbol_or_name());
    match text.strip_suffix(k.logical_key.name()) {
        Some(mods) if cap.chars().count() == 1 && !cap.starts_with(['⏴', '⏵', '⏶', '⏷']) => format!("{mods}{cap}"),
        _ => text,
    }
}

/// What the layer commands do.
const LAYER_HINT: &str = "Makes it the active copper layer, drawn on top; while routing, the track goes on there from a via";

impl About {
    /// Its keys, the one to show first.
    pub(crate) fn keys(&self) -> impl Iterator<Item = KeyboardShortcut> + '_ {
        self.keys.iter().map(|&(m, k)| KeyboardShortcut::new(m, k))
    }
}

impl App {
    /// Whether a tool is on: routing, a zone, an outline, a cutout or lines, a text, measuring, the
    /// Parts tab, or in the schematic a wire, label, no-connect flag or a symbol being placed.
    pub(crate) fn tool_on(&self) -> bool {
        let sch = self.sch.as_ref().filter(|_| self.sch_view());
        self.adding()
            || sch.is_some_and(|s| s.wire.is_some() || s.naming.is_some() || s.placing.is_some() || s.flagging)
            || sch.is_none() && (self.routing || self.zoning || self.measuring || self.pairing || self.writing.is_some())
    }

    /// Select (the tool dock's arrow): the tool on stops, as Esc stops it, with the work in hand.
    pub(crate) fn select_tool(&mut self) {
        (self.route, self.zone, self.ruler, self.writing, self.pair) = (None, None, None, None, None);
        (self.routing, self.zoning, self.measuring, self.pairing) = (false, false, false, false);
        if self.adding() {
            self.left.tab = Tab::Layers;
        }
        self.stop_tool();
    }

    /// Whether `cmd` can run now: else its button is grey and its keys do nothing.
    pub(crate) fn can(&self, cmd: Cmd) -> bool {
        if self.viewer && !cmd.looks() {
            return false;
        }
        let desktop = cfg!(not(target_arch = "wasm32"));
        // The board's side is Board, 3D and Make: a canvas tool given in 3D or Make shows the board.
        let board = self.scene.is_some() && matches!(self.bench, Bench::Board | Bench::ThreeD | Bench::Make);
        let sch = self.sch_view() && self.sch.is_some();
        let canvas = board && self.bench == Bench::Board;
        let overlay = self.show_diff || self.show_ir;
        let picked = !self.picks().is_empty() && !overlay;
        let parts = picked && self.picks().iter().all(|p| matches!(p, Pick::Footprint(_)));
        let (undo, redo) = match &self.sch {
            Some(s) if self.sch_view() => s.sheets.history.peek(),
            _ => self.history.peek(),
        };
        match cmd {
            Home => self.bench != Bench::Home,
            New | Open => desktop,
            Save => self.history.unsaved() || self.sch.as_ref().is_some_and(|s| s.sheets.history.unsaved()),
            Undo => undo.is_some(),
            Redo => redo.is_some(),
            Find | Help | Settings => true,
            Add | Checks => board || sch,
            Fab | ThreeD => self.scene.is_some(),
            Share => desktop && (self.path.is_some() || self.sch.is_some()),
            Pdf | ImportPart | NewPart => desktop && (self.sch.is_some() || self.path.is_some()),
            Schematic => (desktop || self.viewer) && !self.sch_view() && (self.path.is_some() || self.sch.is_some()),
            Board => self.bench != Bench::Board && (self.scene.is_some() || self.sch_view()),
            Make => self.bench != Bench::Make && (self.scene.is_some() || self.sch.is_some()),
            Update => desktop && (self.sch_view() || self.path.is_some()),
            Flip => canvas || board && self.bench == Bench::ThreeD,
            GridNext | GridBack => canvas,
            Measure | Route | Pair | Zone | Outline | Line | Text | Heat | Bom => board,
            Changes => desktop && board,
            Fill => board && !self.board.zones.is_empty(),
            Cutout => canvas && matches!(self.selected, Some(Pick::Zone(_))) && !overlay,
            Ask => desktop,
            Wire | Label | GlobalLabel | NoConnect | Sheet | Bus | BusEntry | Paste => sch,
            MirrorX | MirrorY | Copy | Duplicate => sch && self.sch.as_ref().is_some_and(|s| !s.picked.is_empty() && s.drag.is_none()),
            HierLabel => sch && self.sch.as_ref().is_some_and(|s| s.sheets.project.pages[s.page].parent.is_some()),
            Simulate => desktop && sch,
            Turn => canvas && picked || sch && self.sch.as_ref().is_some_and(|s| s.placing.is_some() || !s.picked.is_empty()),
            TurnBack => canvas && picked,
            FlipPart => canvas && parts,
            TidyTexts => match &self.sch {
                Some(s) if sch => !s.sheets.project.pages[s.page].sheet.symbols.is_empty(),
                _ => canvas && !overlay && !self.board.footprints.is_empty(),
            },
            Units => true,
            Tour => desktop || self.scene.is_some() || self.sch.is_some(),
            Select => canvas || sch,
            Run => canvas && self.picks().iter().any(|p| matches!(p, Pick::Track(_))),
            Shove => canvas && self.routing,
            BlindVias => canvas && self.routing && self.board.stackup.len() > 2,
            Fit => canvas || sch || self.bench == Bench::ThreeD && self.scene.is_some(),
            Guide => self.scene.is_some() || self.sch.is_some(),
            Delete => {
                canvas && picked && self.route.is_none() && self.zone.is_none()
                    || sch && self.sch.as_ref().is_some_and(|s| !s.picked.is_empty() && s.placing.is_none() && s.wire.is_none())
            }
            TopLayer | BottomLayer | NextLayer | PrevLayer => canvas,
            WiderTrack | NarrowerTrack => canvas && self.route.is_some(),
            SwapBend => canvas && self.route.is_some() || sch && self.sch.as_ref().is_some_and(|s| s.wire.is_some()),
            // Arrows move what is picked unless something else listens to them.
            NudgeLeft | NudgeRight | NudgeUp | NudgeDown => {
                canvas && picked && !(self.routing || self.zoning || self.measuring || self.show_bom || self.palette.open || self.adding())
            }
        }
    }

    /// Whether `cmd`'s tool, view or panel is on.
    pub(crate) fn on(&self, cmd: Cmd) -> bool {
        let sch = self.sch.as_ref();
        match cmd {
            Schematic => self.bench == Bench::Schematic,
            Board => self.bench == Bench::Board,
            ThreeD => self.bench == Bench::ThreeD,
            Make => self.bench == Bench::Make,
            #[cfg(not(target_arch = "wasm32"))]
            Share => self.sharing(),
            Guide => self.guide.is_some(),
            Units => crate::units::mils(),
            Tour => self.tour,
            Select => !self.tool_on(),
            Shove => self.shove,
            BlindVias => self.blind,
            Help => self.help.is_some(),
            Settings => self.settings.open,
            Flip => self.flipped,
            Measure => self.measuring,
            Route => self.routing,
            Pair => self.pairing,
            Zone => self.zoning && !self.outlining && !self.cutting && !self.lining,
            Outline => self.zoning && self.outlining,
            Cutout => self.zoning && self.cutting,
            Line => self.zoning && self.lining,
            Text => self.writing.is_some(),
            Add => self.adding(),
            Checks => self.right.shows(Tab::Checks),
            Changes => self.show_diff,
            Heat => self.show_ir,
            Bom => self.show_bom,
            #[cfg(not(target_arch = "wasm32"))]
            Ask => self.right.shows(Tab::Ask),
            Wire => sch.is_some_and(|s| s.wire.is_some() && !s.bus),
            Bus => sch.is_some_and(|s| s.wire.is_some() && s.bus),
            BusEntry => sch.is_some_and(|s| s.placing.as_ref().is_some_and(|p| p.entry())),
            Label => sch.is_some_and(|s| matches!(s.naming, Some((_, LabelKind::Local, _)))),
            GlobalLabel => sch.is_some_and(|s| matches!(s.naming, Some((_, LabelKind::Global, _)))),
            HierLabel => sch.is_some_and(|s| matches!(s.naming, Some((_, LabelKind::Hier, _)))),
            Sheet => sch.is_some_and(|s| s.sheeting.is_some()),
            NoConnect => sch.is_some_and(|s| s.flagging),
            #[cfg(not(target_arch = "wasm32"))]
            Simulate => sch.is_some_and(|s| matches!(s.sim, Some(crate::sim::Simulation::Running { .. }))),
            _ => false,
        }
    }

    /// Does `cmd`, if it can run now.
    pub(crate) fn run(&mut self, ctx: &egui::Context, cmd: Cmd) {
        if !self.can(cmd) {
            return;
        }
        if matches!(cmd, Measure | Route | Pair | Zone | Outline | Line | Text | Add | Checks | Changes | Heat | Bom)
            && self.bench != Bench::Board
            && !self.sch_view()
        {
            self.go(Bench::Board);
        }
        match cmd {
            Home => self.go(Bench::Home),
            New => self.start_new(ctx, false),
            Open => self.ask_open(),
            Save => {
                if self.history.unsaved() {
                    self.save();
                }
                if self.sch.as_ref().is_some_and(|s| s.sheets.history.unsaved()) {
                    self.save_sch();
                }
            }
            Undo | Redo if self.sch_view() => self.undo_sch(cmd == Redo),
            Undo | Redo => self.undo(cmd == Redo),
            Find => self.toggle_palette(ctx),
            Fab => self.fab(),
            #[cfg(not(target_arch = "wasm32"))]
            Pdf => self.pdf(ctx),
            #[cfg(target_arch = "wasm32")]
            Pdf => {}
            ImportPart => self.ask_parts(),
            #[cfg(not(target_arch = "wasm32"))]
            NewPart => self.new_part(),
            Schematic => self.go(Bench::Schematic),
            Board => self.go(Bench::Board),
            ThreeD => self.go(if self.bench == Bench::ThreeD { Bench::Board } else { Bench::ThreeD }),
            Make => self.go(Bench::Make),
            Update => self.update_board(),
            Flip => {
                self.flipped = !self.flipped;
                // F flips the parts picked; with other items picked too it flips the view, saying why.
                let others: Vec<Pick> = self.picked.iter().copied().filter(|p| !matches!(p, Pick::Footprint(_))).collect();
                if !others.is_empty() {
                    let others = picks_name(&self.board, &others);
                    self.status = format!("F puts parts on the other side, and {others} are picked too: it flipped the view");
                }
            }
            // One canvas tool at a time: the others stop, with the work in their hands.
            Measure | Route | Pair | Zone | Outline | Cutout | Line => {
                let on = !self.on(cmd);
                self.select_tool();
                (self.measuring, self.routing, self.pairing) = (on && cmd == Measure, on && cmd == Route, on && cmd == Pair);
                self.zoning = on && matches!(cmd, Zone | Outline | Cutout | Line);
                (self.outlining, self.cutting, self.lining) = (cmd == Outline, cmd == Cutout, cmd == Line);
            }
            Text => self.text_tool(ctx),
            Add => self.toggle_add(ctx),
            Fill => self.fill(),
            Checks => self.right.toggle(Tab::Checks, Tab::Inspect),
            Changes => self.toggle_diff(None),
            Heat => self.toggle_ir(None),
            Bom => {
                self.show_bom = !self.show_bom;
                if self.show_bom {
                    self.right = Dock { tab: Tab::Bom, open: true };
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Ask => self.toggle_chat(ctx),
            Wire | Bus => self.wire_tool(cmd == Bus),
            BusEntry => self.place_entry(),
            Label => self.label_tool(ctx, LabelKind::Local),
            GlobalLabel => self.label_tool(ctx, LabelKind::Global),
            HierLabel => self.label_tool(ctx, LabelKind::Hier),
            MirrorX | MirrorY => self.mirror_picked(cmd == MirrorY),
            Copy => self.copy_picked(ctx),
            Paste => self.paste_clip(None),
            Duplicate => self.duplicate_picked(),
            Sheet => self.sheet_tool(),
            NoConnect => self.no_connect_tool(),
            #[cfg(not(target_arch = "wasm32"))]
            Simulate => self.simulate(ctx),
            Turn | TurnBack if self.sch.as_ref().is_some_and(|s| s.placing.is_some()) && self.sch_view() => self.turn_placing(),
            Turn | TurnBack if self.sch_view() => self.turn_picked(),
            Turn | TurnBack => self.turn_picks(cmd == Turn),
            Delete if self.sch_view() => self.delete_picked(),
            Delete => self.delete(),
            Fit if self.bench == Bench::ThreeD => self.orbit = None,
            Fit if self.sch_view() => self.fit = self.sch.as_ref().map(|s| s.scene.bounds),
            Fit => self.fit = self.scene.as_ref().map(|s| crate::whole(&self.board, s)),
            Guide => {
                self.guide = if self.guide.is_some() { None } else { Some(true) };
                if self.guide.is_some() && !matches!(self.bench, Bench::Schematic | Bench::Board) {
                    self.go(if self.sch.is_some() { Bench::Schematic } else { Bench::Board });
                }
            }
            TopLayer | BottomLayer | NextLayer | PrevLayer => {
                // Next goes down the stack, from the bottom round to the top.
                let stack: Vec<_> = self.board.stackup.iter().map(|c| c.layer).collect();
                let at = stack.iter().position(|&l| l == self.active).unwrap_or(0);
                let to = match cmd {
                    TopLayer => stack.first(),
                    BottomLayer => stack.last(),
                    NextLayer => stack.iter().cycle().nth(at + 1),
                    _ => stack.iter().rev().cycle().nth(stack.len() - at),
                };
                if let Some(&layer) = to {
                    self.switch_layer(layer);
                }
            }
            WiderTrack | NarrowerTrack => self.route_width(cmd == WiderTrack),
            SwapBend if self.sch_view() => self.sch.iter_mut().for_each(|s| s.swap_bend = !s.swap_bend),
            SwapBend => self.swap_bend(),
            GridNext | GridBack => self.step_grid(cmd == GridBack),
            NudgeLeft => self.nudge((-1, 0)),
            NudgeRight => self.nudge((1, 0)),
            NudgeUp => self.nudge((0, -1)),
            NudgeDown => self.nudge((0, 1)),
            FlipPart => self.flip_picks(),
            TidyTexts if self.sch_view() => self.tidy_fields(),
            TidyTexts => self.tidy_texts(),
            Units => crate::units::toggle(),
            Tour => self.start_tour(ctx),
            Select => self.select_tool(),
            Run => self.pick_runs(),
            Shove => self.toggle_shove(),
            BlindVias => {
                self.blind = !self.blind;
                let says = if self.blind {
                    t!("Vias join only the layers a track goes between: blind or buried")
                } else {
                    t!("Vias go through the board")
                };
                self.status = tr(says).into();
            }
            Help => {
                let open = self.help.is_none().then(|| crate::help::Help::new(self.bench == Bench::Home));
                (self.help, self.settings.open) = (open, false);
            }
            Settings => (self.settings.open, self.help) = (!self.settings.open, None),
            #[cfg(not(target_arch = "wasm32"))]
            Share => self.share(),
            #[cfg(target_arch = "wasm32")]
            Ask | Simulate | NewPart | Share => {}
        }
    }

    /// Runs the commands whose keys were pressed. While a field is typed in, only keys with Ctrl
    /// (or Cmd) or F-keys count, but for the text's own Ctrl+Z, Y, A, C, V and X.
    pub(crate) fn keys(&mut self, ctx: &egui::Context) {
        // The New part window has the keys while it is open.
        #[cfg(not(target_arch = "wasm32"))]
        if self.wizard.is_some() {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        let mut keys: Vec<(Cmd, KeyboardShortcut)> =
            Cmd::ALL.iter().flat_map(|&c| c.about().keys().map(move |k| (c, k)).collect::<Vec<_>>()).collect();
        // Shift+B before B: a key with more modifiers is matched first, as egui ignores extra ones.
        keys.sort_by_key(|(_, k)| std::cmp::Reverse(k.modifiers.command as u8 + k.modifiers.shift as u8 + k.modifiers.alt as u8));
        let text = [Key::Z, Key::Y, Key::A, Key::C, Key::V, Key::X];
        for (cmd, k) in keys {
            let f_key = matches!(
                k.logical_key,
                Key::F1 | Key::F2 | Key::F3 | Key::F4 | Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::F11 | Key::F12
            );
            let while_typing = f_key || k.modifiers.command && !text.contains(&k.logical_key);
            // F flips the parts picked, else the view (KiCad's key for both).
            let shadowed = cmd == Flip && self.can(FlipPart);
            if (!typing || while_typing) && self.can(cmd) && !shadowed && ctx.input_mut(|i| i.consume_shortcut(&k)) {
                self.run(ctx, cmd);
            }
        }
    }

    /// `cmd` as an icon button `size` across: lit while it is on, grey while it can't run;
    /// hovering it names it, its key and what it does. With `badge`, a one-letter key shows in
    /// a corner.
    pub(crate) fn icon_cmd(&mut self, ui: &mut egui::Ui, cmd: Cmd, size: f32, badge: bool) {
        let about = cmd.about();
        let key = about.keys().next().map(|k| key_text(ui.ctx(), &k)).filter(|k| badge && k.chars().count() == 1);
        let button = theme::tool(ui, about.icon, size, self.on(cmd), self.can(cmd), &key.unwrap_or_default());
        if button.on_hover_text(self.cmd_tip(ui.ctx(), cmd)).clicked() {
            self.run(ui.ctx(), cmd);
        }
    }

    /// `cmd`'s name, its key, and what it does now: undo and redo name their step.
    pub(crate) fn cmd_tip(&self, ctx: &egui::Context, cmd: Cmd) -> String {
        let about = cmd.about();
        let key = about.keys().next().map(|k| format!(" ({})", key_text(ctx, &k))).unwrap_or_default();
        let peek = if self.sch_view() { self.sch.as_ref().map(|s| s.sheets.history.peek()) } else { Some(self.history.peek()) };
        let what = match (cmd, peek) {
            (Undo, Some((Some(what), _))) | (Redo, Some((_, Some(what)))) => what,
            _ => about.hint,
        };
        format!("{}{key}: {what}", about.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_runs_one_command_where_it_applies() {
        let mut seen: Vec<(KeyboardShortcut, Cmd)> = vec![];
        for c in Cmd::ALL {
            let about = c.about();
            assert!(!about.name.is_empty() && !about.hint.is_empty(), "{c:?}");
            for k in about.keys() {
                // Ctrl+E switches between schematic and board; W draws a wire in the schematic and
                // widens a track on the board, B a bus and fills the zones, Z a bus entry and a
                // zone, X routes on the board and mirrors on a sheet, L draws lines on the board
                // and labels on a sheet: they never run in the same view; F flips the parts
                // picked, else the view.
                if let Some((_, other)) = seen.iter().find(|(s, _)| *s == k) {
                    assert!(
                        matches!(
                            (other, c),
                            (Schematic, Board)
                                | (Wire, WiderTrack)
                                | (Flip, FlipPart)
                                | (Fill, Bus)
                                | (Zone, BusEntry)
                                | (Route, MirrorX)
                                | (Line, Label)
                        ),
                        "{c:?} and {other:?} share a key"
                    );
                }
                seen.push((k, c));
            }
        }
    }

    #[test]
    fn keys_read_as_their_keycaps() {
        let (ctx, key) = (egui::Context::default(), |m, k| KeyboardShortcut::new(m, k));
        let text = |m, k| key_text(&ctx, &key(m, k));
        assert_eq!(text(NONE, Key::Slash), "/");
        assert_eq!(text(NONE, Key::Questionmark), "?");
        assert_eq!(text(NONE, Key::Plus), "+");
        assert_eq!(text(CTRL, Key::K), "Ctrl+K");
        assert_eq!(text(SHIFT, Key::W), "Shift+W");
        assert_eq!(text(NONE, Key::F1), "F1");
        assert_eq!(text(NONE, Key::ArrowLeft), "Left");
    }

    #[test]
    fn the_browser_leaves_ctrl_keys_to_cuprita() {
        // index.html cancels the browser's action for its own keys, eframe for O, P and S; Z, Y and
        // N don't touch the page (Chrome keeps Ctrl+N, a new window; New is desktop only); C and V
        // are the browser's to take: its copy and paste are what reach Cuprita.
        let page = include_str!("../index.html");
        let own = page.split("const own = \"").nth(1).and_then(|s| s.split('"').next()).expect("index.html lists its keys");
        for c in Cmd::ALL {
            for k in c.about().keys().filter(|k| k.modifiers.command) {
                let key = k.logical_key.symbol_or_name().to_lowercase();
                assert!("opszyncv".contains(&key) || own.contains(&key), "{c:?}: Ctrl+{key} isn't one of index.html's own keys");
            }
        }
    }
}
