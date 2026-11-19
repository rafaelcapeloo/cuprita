//! KiCad schematics (`.kicad_sch`, KiCad 6–10): a sheet as KiCad draws it, in sheet coordinates
//! (nm, Y down): its lines, curves and text, each in an `Ink`; its symbols and the sheets in it.
//! KiCad 5's `.sch` isn't read: KiCad 6 or later converts it.

mod autoplace;
mod bus;
mod clip;
mod edit;
mod erc;
mod fields;
mod frame;
mod hier;
mod label;
mod net;
mod page;
mod pick;
mod placed;
mod project;
mod spice;
mod symbol;

use crate::board::{Geom, LayerId, Nm, Pt, Shape, Text};
use crate::json::Val;
use crate::parse::{effects, pts_into};
use crate::sexpr::{Cursor, Error, Result, unescape};
use crate::tr::tr;
use net::Wiring;
use std::collections::HashMap;
use symbol::LibSymbol;

pub use edit::{Patch, SHAPES, on_grid};
pub use erc::Violation;
pub use fields::PartFields;
pub use frame::Frame;
pub use net::{LabelKind, NetOnPage, PinEnd};
pub use page::{LINES, PAPERS, PageSetup};
pub use pick::Item;
pub use project::{Page, Project};
pub use spice::Deck;

/// KiCad 6's references by symbol path (`/<sheet uuid>…/<symbol uuid>`): its root sheet's
/// `symbol_instances`, a reference and unit for each placement of a symbol.
type Legacy = HashMap<String, (String, u32)>;

/// What KiCad's schematic colours tell apart, back to front: the `layer` of a drawing's shapes
/// and texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// The drawing sheet: the frame and title block (`Project::frame`).
    Frame,
    BodyFill,
    Notes,
    Sheet,
    Body,
    Wire,
    Bus,
    NoConnect,
    Label,
    GlobalLabel,
    HierLabel,
    SheetName,
    SheetFile,
    SheetPin,
    PinName,
    PinNumber,
    Reference,
    Value,
    Field,
}

impl Ink {
    pub const ALL: [Ink; 19] = [
        Ink::Frame,
        Ink::BodyFill,
        Ink::Notes,
        Ink::Sheet,
        Ink::Body,
        Ink::Wire,
        Ink::Bus,
        Ink::NoConnect,
        Ink::Label,
        Ink::GlobalLabel,
        Ink::HierLabel,
        Ink::SheetName,
        Ink::SheetFile,
        Ink::SheetPin,
        Ink::PinName,
        Ink::PinNumber,
        Ink::Reference,
        Ink::Value,
        Ink::Field,
    ];

    /// Its colour, `0xRRGGBB`: KiCad's default theme's.
    pub fn color(self) -> u32 {
        match self {
            Ink::BodyFill => 0xFFFFC2,
            Ink::Notes => 0x0000C2,
            Ink::Wire => 0x009600,
            Ink::Bus | Ink::NoConnect => 0x000084,
            Ink::Label => 0x0F0F0F,
            Ink::HierLabel | Ink::SheetFile => 0x725600,
            Ink::SheetName | Ink::SheetPin | Ink::PinName | Ink::Reference | Ink::Value => 0x006464,
            Ink::PinNumber => 0xA90000,
            Ink::Field => 0x840084,
            Ink::Frame | Ink::Sheet | Ink::Body | Ink::GlobalLabel => 0x840000,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Ink::Frame => tr("Drawing sheet"),
            Ink::BodyFill => tr("Symbol fills"),
            Ink::Notes => tr("Notes"),
            Ink::Sheet => tr("Sheets"),
            Ink::Body => tr("Symbols"),
            Ink::Wire => tr("Wires"),
            Ink::Bus => tr("Buses"),
            Ink::NoConnect => tr("No connects"),
            Ink::Label => tr("Labels"),
            Ink::GlobalLabel => tr("Global labels"),
            Ink::HierLabel => tr("Hierarchical labels"),
            Ink::SheetName => tr("Sheet names"),
            Ink::SheetFile => tr("Sheet files"),
            Ink::SheetPin => tr("Sheet pins"),
            Ink::PinName => tr("Pin names"),
            Ink::PinNumber => tr("Pin numbers"),
            Ink::Reference => tr("References"),
            Ink::Value => tr("Values"),
            Ink::Field => tr("Fields"),
        }
    }
}

/// KiCad's default line width (6 mil): wires, lines and text pens without one of their own.
const LINE: Nm = 152_400;
const BUS: Nm = 304_800;
const JUNCTION: Nm = 914_400;
/// Half the no-connect cross.
const CROSS: Nm = 609_600;
/// KiCad's A4 page, in mils.
const A4: (Nm, Nm) = (11693 * 25_400, 8268 * 25_400);

/// One schematic file: a sheet.
#[derive(Clone, Debug, Default)]
pub struct Schematic {
    /// The page's width and height.
    pub page: (Nm, Nm),
    pub symbols: Vec<Symbol>,
    pub sheets: Vec<Sheet>,
    /// Lines, curves and fills; `layer` is their `Ink`.
    pub shapes: Vec<Shape>,
    /// Shown text; `layer` is its `Ink`.
    pub texts: Vec<Text>,
    /// What its title block says of the drawing (`pdf.rs` plots it).
    pub title: TitleBlock,
    /// What its items connect.
    wiring: Wiring,
    /// Its texts that are SPICE directives (`.tran 1u 1m`), in its order.
    directives: Vec<String>,
    style: Style,
    /// Where in its project's hierarchy it is drawn: `/<root uuid>/<sheet uuid>…`, KiCad's
    /// instance path, whose references its symbols show.
    path: String,
    /// A KiCad 6 root sheet's `symbol_instances`.
    legacy: Legacy,
    /// A root sheet's page numbers by instance path, from its `sheet_instances`: KiCad 6 lists
    /// every sheet's there, KiCad 7 on only its own.
    numbers: Vec<(String, String)>,
}

/// A sheet's title block, as its file gives it (KiCad's `title_block`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitleBlock {
    pub title: String,
    pub date: String,
    pub rev: String,
    pub company: String,
    /// Its comments (KiCad's 1 to 9) in order, the empty ones left out.
    pub comments: Vec<String>,
}

/// A project's drawing settings (`.kicad_pro`) that place labels: its text offset ratio and
/// label size ratio, and the size of new text; KiCad's defaults without them.
#[derive(Clone, Copy, Debug)]
struct Style {
    text_offset: f64,
    label_size: f64,
    text_size: Nm,
}

impl Style {
    /// The settings of a project's `.kicad_pro`, KiCad's defaults where it has none.
    fn of(pro: Option<&Val>) -> Style {
        let num = |k: &str, default| pro.and_then(|p| p.at(&["schematic", "drawing"])?.num(k)).filter(|&v| v >= 0.0).unwrap_or(default);
        let text_size = (num("default_text_size", 50.0) * 25_400.0).round() as Nm;
        Style { text_offset: num("text_offset_ratio", 0.15), label_size: num("label_size_ratio", 0.375), text_size }
    }
}

impl Default for Style {
    fn default() -> Self {
        Style::of(None)
    }
}

/// A placed symbol (one unit of a part).
#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    /// As shown: with the unit's letter when the part has several (`U1A`).
    pub reference: String,
    /// Box around its body and pins.
    pub bounds: (Pt, Pt),
    /// Its reference without the unit's letter (`U1`): its part's, as the board's footprint has it.
    pub designator: String,
    /// Its part's value and footprint (`Resistor_SMD:R_0603_1608Metric`; empty when none is chosen).
    pub value: String,
    pub footprint: String,
    /// What links a footprint to it (the footprint's `path`): its sheets' ids below the root,
    /// then its own (`/…/…`).
    pub link: String,
    /// Its library symbol (`Device:R`), and where its `(symbol …)` list is in the sheet's text.
    pub lib_id: String,
    pub span: std::ops::Range<usize>,
    /// A power symbol's net (its value) and where that name joins.
    power: Option<(LabelKind, String)>,
    /// Whether KiCad's Exclude from board is unticked (`(on_board yes)`); `Symbol::on_board`
    /// also leaves out power symbols.
    on_board: bool,
    /// Whether KiCad's Exclude from bill of materials is unticked (`(in_bom yes)`), and Do not
    /// populate ticked (`(dnp yes)`): its footprint's attributes follow them.
    pub in_bom: bool,
    pub dnp: bool,
    /// Its unit: 1 for a part of one.
    unit: u32,
    /// Its simulation model's fields by their names after `Sim.` (`Device`, `Params`…), KiCad 6's
    /// by theirs (`Spice_Model`), and whether KiCad's Exclude from simulation is unticked.
    sim: Vec<(String, String)>,
    on_sim: bool,
}

/// A hierarchical sheet: a box standing for the schematic file it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub name: String,
    /// Its schematic file, relative to this one's folder.
    pub file: String,
    pub bounds: (Pt, Pt),
    /// Its id, the last step of its sheet's instance path.
    uuid: String,
    /// Its pins' names and where wires meet them.
    pins: Vec<(String, Pt)>,
    /// Its page's number by the instance path of the sheet it is on (KiCad 7's `instances`).
    numbers: Vec<(String, String)>,
}

impl Schematic {
    /// Reads a `.kicad_sch` file's text, drawn with the settings of its project's `.kicad_pro`
    /// text (`""` for none).
    // shortcut: a project's own pin mark size and default line widths aren't read (KiCad's
    // defaults are drawn); read them when a project changes them.
    pub fn parse(src: &str, pro: &str) -> Result<Schematic> {
        Schematic::parse_at(src, Style::of(Val::parse(pro).as_ref()), "", &Legacy::new())
    }

    /// `parse` for the sheet at instance path `path` of its project (`""`: a root sheet, which
    /// its own id names), showing that instance's references (KiCad 6's from `legacy`).
    fn parse_at(src: &str, style: Style, path: &str, legacy: &Legacy) -> Result<Schematic> {
        if src.starts_with("EESchema") {
            let msg = tr("a KiCad 5 schematic: open it in KiCad 6 or later, which converts it, then open the .kicad_sch");
            return Err(Error { offset: 0, msg });
        }
        let mut c = Cursor::new(src);
        c.open("kicad_sch")?;
        let mut s = Schematic { page: A4, style, path: path.into(), ..Schematic::default() };
        let mut lib = HashMap::new();
        while let Some(head) = c.child()? {
            match head {
                // KiCad writes it before any symbol.
                "uuid" if s.path.is_empty() => s.path = format!("/{}", c.atom()?),
                "symbol_instances" => {
                    while let Some(head) = c.child()? {
                        if head == "path" {
                            let key = c.atom()?;
                            s.legacy.insert(key.into(), instance(&mut c)?);
                        }
                        c.close()?;
                    }
                }
                "sheet_instances" => numbers(&mut c, &mut s.numbers)?,
                "paper" => s.page = paper(&mut c)?,
                "title_block" => s.title = title_block(&mut c)?,
                "lib_symbols" => {
                    while let Some(head) = c.child()? {
                        if head == "symbol" {
                            let name = c.atom()?;
                            lib.insert(name, LibSymbol::parse(&mut c)?);
                        }
                        c.close()?;
                    }
                }
                "symbol" => {
                    // Its `(`, just before the head read.
                    let start = src[..c.span().start].rfind('(').unwrap_or(0);
                    symbol::place(&mut c, &lib, &mut s, legacy)?;
                    c.close()?;
                    if let Some(placed) = s.symbols.last_mut() {
                        placed.span = start..c.span().end;
                    }
                    continue;
                }
                "wire" | "bus" | "bus_entry" => s.wire(&mut c, head)?,
                "junction" => {
                    let (mut at, mut d) = (Pt::default(), 0);
                    while let Some(head) = c.child()? {
                        match head {
                            "at" => at = c.point()?,
                            "diameter" => d = c.nm()?,
                            _ => {}
                        }
                        c.close()?;
                    }
                    let radius = if d > 0 { d } else { JUNCTION } / 2;
                    s.shape(Geom::Circle { center: at, radius }, 0, true, Ink::Wire);
                    s.wiring.junctions.push(at);
                }
                "no_connect" => {
                    let at = at(&mut c)?.0;
                    s.wiring.no_connects.push(at);
                    for (dx, dy) in [(CROSS, CROSS), (CROSS, -CROSS)] {
                        let (a, b) = (Pt { x: at.x - dx, y: at.y - dy }, Pt { x: at.x + dx, y: at.y + dy });
                        s.shape(Geom::Segment { a, b }, LINE, false, Ink::NoConnect);
                    }
                }
                "label" | "global_label" | "hierarchical_label" => {
                    let (start, n) = (src[..c.span().start].rfind('(').unwrap_or(0), s.wiring.labels.len());
                    label::label(&mut c, head, &mut s)?;
                    c.close()?;
                    if let Some(l) = s.wiring.labels.get_mut(n) {
                        l.span = start..c.span().end;
                    }
                    continue;
                }
                "sheet" => label::sheet(&mut c, &mut s)?,
                "text" => {
                    let (raw, mut on_sim) = (c.atom()?, true);
                    let mut t = text(&mut c, raw, Ink::Notes, |c, head| {
                        if head == "exclude_from_sim" {
                            on_sim = c.atom()? != "yes";
                        }
                        Ok(())
                    })?;
                    s.directive(&t.text, on_sim);
                    // KiCad 10 shows notes 0.25 mm up the sheet, whichever way they read (measured).
                    t.at.y -= 250_000;
                    if !t.text.is_empty() {
                        s.texts.push(upright(t));
                    }
                }
                "text_box" => s.text_box(&mut c)?,
                "polyline" | "rectangle" | "circle" | "arc" | "bezier" => {
                    let g = graphic(&mut c, head)?;
                    s.draw(&g, Ink::Notes, Ink::BodyFill);
                }
                // Rule areas (KiCad 9) are polylines drawn as notes.
                "rule_area" => {
                    while let Some(head) = c.child()? {
                        if head == "polyline" {
                            let g = graphic(&mut c, head)?;
                            s.draw(&g, Ink::Notes, Ink::BodyFill);
                        }
                        c.close()?;
                    }
                }
                // shortcut: images, tables and net class flags aren't drawn; draw them when sheets need them.
                _ => {}
            }
            c.close()?;
        }
        Ok(s)
    }

    /// Keeps a text or text box as a SPICE directive when it is one and isn't excluded from simulation.
    fn directive(&mut self, text: &str, on_sim: bool) {
        if on_sim && spice::directive(text) {
            self.directives.push(text.to_string());
        }
    }

    fn shape(&mut self, geom: Geom, width: Nm, filled: bool, ink: Ink) {
        self.shapes.push(Shape { geom, width, filled, layer: ink as LayerId });
    }

    /// A wire, bus (`(pts …)`) or bus entry (`(at …) (size …)`), drawn as KiCad does.
    fn wire(&mut self, c: &mut Cursor, kind: &str) -> Result<()> {
        let (mut pts, mut width, mut size) = (vec![], 0, Pt::default());
        while let Some(head) = c.child()? {
            match head {
                "pts" => pts_into(c, &mut pts)?,
                "at" => pts.push(c.point()?),
                "size" => size = c.point()?,
                "stroke" => width = stroke(c)?,
                _ => {}
            }
            c.close()?;
        }
        if kind == "bus_entry" {
            let &[at] = &pts[..] else { return Err(c.err(tr("bus entry without one (at)"))) };
            pts.push(Pt { x: at.x + size.x, y: at.y + size.y });
        }
        let (ink, default) = if kind == "bus" { (Ink::Bus, BUS) } else { (Ink::Wire, LINE) };
        let w = &mut self.wiring;
        let into = match kind {
            "bus" => &mut w.buses,
            "bus_entry" => &mut w.entries,
            _ => &mut w.wires,
        };
        into.extend(pts.windows(2).map(|p| (p[0], p[1])));
        for w in pts.windows(2) {
            self.shape(Geom::Segment { a: w[0], b: w[1] }, if width > 0 { width } else { default }, false, ink);
        }
        Ok(())
    }

    /// A graphic as KiCad draws it: its fill in `fill`, its outline in `ink`.
    fn draw(&mut self, g: &Graphic, ink: Ink, fill: Ink) {
        if g.fill != Fill::None && g.fill != Fill::Color {
            let area = match &g.geom {
                Geom::Arc { start, mid, end } => Geom::Poly(crate::board::arc_points(*start, *mid, *end)),
                other => other.clone(),
            };
            let fill = if g.fill == Fill::Outline { ink } else { fill };
            self.shape(area, 0, true, fill);
        }
        if g.width < 0 {
            return;
        }
        let width = if g.width > 0 { g.width } else { LINE };
        match &g.geom {
            // An open line: one segment per pair, so the renderer doesn't close it.
            Geom::Poly(pts) if !g.closed => {
                for w in pts.windows(2) {
                    self.shape(Geom::Segment { a: w[0], b: w[1] }, width, false, ink);
                }
            }
            geom => self.shape(geom.clone(), width, false, ink),
        }
    }

    /// A text box: its border and its text, placed in it by its justification.
    fn text_box(&mut self, c: &mut Cursor) -> Result<()> {
        let raw = c.atom()?;
        let (mut size, mut margins, mut width, mut on_sim) = (Pt::default(), None, 0, true);
        let mut t = text(c, raw, Ink::Notes, |c, head| {
            match head {
                "size" => size = c.point()?,
                "margins" => margins = Some(c.nm()?),
                "stroke" => width = stroke(c)?,
                "exclude_from_sim" => on_sim = c.atom()? != "yes",
                _ => {}
            }
            Ok(())
        })?;
        self.directive(&t.text, on_sim);
        let at = t.at;
        let corners =
            vec![at, Pt { x: at.x + size.x, y: at.y }, Pt { x: at.x + size.x, y: at.y + size.y }, Pt { x: at.x, y: at.y + size.y }];
        if width >= 0 {
            self.shape(Geom::Poly(corners), if width > 0 { width } else { LINE }, false, Ink::Notes);
        }
        let m = margins.unwrap_or(t.size.1 / 2);
        let place = |j: i8, lo: Nm, len: Nm| lo + m + (len - 2 * m) * (j as Nm + 1) / 2;
        t.at = Pt { x: place(t.justify.0, at.x, size.x), y: place(t.justify.1, at.y, size.y) };
        if !t.text.is_empty() {
            self.texts.push(t);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fill {
    None,
    /// In the outline's colour.
    Outline,
    /// In the symbol body's background colour.
    Background,
    /// shortcut: in a colour of its own, which isn't drawn; draw it when a sheet needs it.
    Color,
}

/// A graphic item: polyline, rectangle, circle, arc or Bézier curve; width -1: no outline.
#[derive(Clone, Debug)]
struct Graphic {
    geom: Geom,
    /// Whether a `Geom::Poly` is a closed outline (a rectangle) rather than an open line.
    closed: bool,
    width: Nm,
    fill: Fill,
}

/// Reads a graphic item; `head` is its kind.
fn graphic(c: &mut Cursor, head: &str) -> Result<Graphic> {
    let (mut pts, mut start, mut mid, mut end, mut center, mut radius) = (vec![], None, None, None, None, 0);
    let (mut width, mut fill) = (0, Fill::None);
    while let Some(h) = c.child()? {
        match h {
            "pts" => pts_into(c, &mut pts)?,
            "start" => start = Some(c.point()?),
            "mid" => mid = Some(c.point()?),
            "end" => end = Some(c.point()?),
            "center" => center = Some(c.point()?),
            "radius" => radius = c.nm()?,
            "stroke" => width = stroke(c)?,
            "fill" => {
                while let Some(h) = c.child()? {
                    if h == "type" {
                        fill = match c.atom()? {
                            "outline" => Fill::Outline,
                            "background" => Fill::Background,
                            "color" => Fill::Color,
                            _ => Fill::None,
                        };
                    }
                    c.close()?;
                }
            }
            _ => {}
        }
        c.close()?;
    }
    let bad = || c.err(tr("incomplete graphic"));
    let (geom, closed) = match (head, start, end) {
        ("polyline", ..) if pts.len() >= 2 => (Geom::Poly(pts), false),
        ("bezier", ..) if pts.len() == 4 => (Geom::Poly(bezier(&pts)), false),
        ("rectangle", Some(a), Some(b)) => (Geom::Poly(vec![a, Pt { x: b.x, y: a.y }, b, Pt { x: a.x, y: b.y }]), true),
        ("circle", ..) => (Geom::Circle { center: center.ok_or_else(bad)?, radius }, true),
        ("arc", Some(start), Some(end)) => (Geom::Arc { start, mid: mid.ok_or_else(bad)?, end }, false),
        _ => return Err(bad()),
    };
    Ok(Graphic { geom, closed, width, fill })
}

/// Points along a cubic Bézier curve.
// shortcut: 16 even steps, not within a tolerance; refine if a big curve looks angular.
fn bezier(p: &[Pt]) -> Vec<Pt> {
    let f = |a: Nm, b: Nm, c: Nm, d: Nm, t: f64| {
        let u = 1.0 - t;
        (u * u * u * a as f64 + 3.0 * u * u * t * b as f64 + 3.0 * u * t * t * c as f64 + t * t * t * d as f64).round() as Nm
    };
    (0..=16)
        .map(|i| i as f64 / 16.0)
        .map(|t| Pt { x: f(p[0].x, p[1].x, p[2].x, p[3].x, t), y: f(p[0].y, p[1].y, p[2].y, p[3].y, t) })
        .collect()
}

/// An instance's `(reference "R1") (unit 1)`, the rest of its list read.
fn instance(c: &mut Cursor) -> Result<(String, u32)> {
    let (mut reference, mut unit) = (String::new(), 1);
    while let Some(head) = c.child()? {
        match head {
            "reference" => reference = unescape(c.atom()?),
            "unit" => unit = c.atom()?.parse().unwrap_or(1),
            _ => {}
        }
        c.close()?;
    }
    Ok((reference, unit))
}

/// `(stroke (width w) (type t))`: its width (0: the default; dashes draw solid).
// shortcut: dashed and dotted lines and their own colours draw solid in their ink's colour.
fn stroke(c: &mut Cursor) -> Result<Nm> {
    let mut width = 0;
    while let Some(head) = c.child()? {
        if head == "width" {
            width = c.nm()?;
        }
        c.close()?;
    }
    Ok(width)
}

/// The `(at x y angle)` among a list's children, the rest skipped.
fn at(c: &mut Cursor) -> Result<(Pt, f64)> {
    let mut out = (Pt::default(), 0.0);
    while let Some(head) = c.child()? {
        if head == "at" {
            out = (c.point()?, c.opt_angle()?);
        }
        c.close()?;
    }
    Ok(out)
}

/// A text item's `(at …)` and `(effects …)`, its other children passed to `other`; hidden text
/// comes back empty. Its pen, when it has none of its own, is KiCad's: a fifth of its height
/// when bold, else the default line width, on all but pins at most a quarter of its height
/// (measured from KiCad 10 plots).
fn text<'a>(c: &mut Cursor<'a>, raw: &str, ink: Ink, mut other: impl FnMut(&mut Cursor<'a>, &'a str) -> Result<()>) -> Result<Text> {
    let mut t = Text { text: unescape(raw), size: (1_270_000, 1_270_000), layer: ink as LayerId, ..Text::default() };
    let (mut hidden, mut bold) = (false, false);
    loop {
        // KiCad 6 writes `hide` as a bare flag.
        while let Some(flag) = c.opt_atom()? {
            hidden |= flag == "hide";
        }
        let Some(head) = c.child()? else { break };
        match head {
            "at" => (t.at, t.angle) = (c.point()?, c.opt_angle()?),
            "effects" => bold = effects(c, &mut t, &mut hidden)?,
            "hide" => hidden = c.atom()? == "yes",
            _ => other(c, head)?,
        }
        c.close()?;
    }
    if t.thickness <= 0 {
        let h = t.size.1;
        t.thickness = if bold {
            h / 5
        } else if matches!(ink, Ink::PinName | Ink::PinNumber) {
            LINE
        } else {
            LINE.min(h / 4)
        };
    }
    if hidden {
        t.text.clear();
    }
    Ok(t)
}

/// Text as KiCad shows it on a sheet: readable, so 180° reads as 0° and 270° as 90°, its
/// justification as written.
fn upright(mut t: Text) -> Text {
    t.angle = t.angle.rem_euclid(180.0);
    t
}

/// The page numbers in a list of `(path "…" (page "…"))`, by path, onto `into`.
fn numbers(c: &mut Cursor, into: &mut Vec<(String, String)>) -> Result<()> {
    while let Some(head) = c.child()? {
        if head == "path" {
            let path = c.atom()?;
            while let Some(head) = c.child()? {
                if head == "page" {
                    into.push((path.into(), unescape(c.atom()?)));
                }
                c.close()?;
            }
        }
        c.close()?;
    }
    Ok(())
}

/// `(title_block (title "…") (date "…") (rev "…") (company "…") (comment 1 "…") …)`.
fn title_block(c: &mut Cursor) -> Result<TitleBlock> {
    let (mut t, mut comments) = (TitleBlock::default(), vec![]);
    while let Some(head) = c.child()? {
        match head {
            "title" => t.title = unescape(c.atom()?),
            "date" => t.date = unescape(c.atom()?),
            "rev" => t.rev = unescape(c.atom()?),
            "company" => t.company = unescape(c.atom()?),
            "comment" => comments.push((c.atom()?.parse::<u32>().unwrap_or(0), unescape(c.atom()?))),
            _ => {}
        }
        c.close()?;
    }
    comments.sort();
    t.comments = comments.into_iter().map(|(_, text)| text).filter(|text| !text.is_empty()).collect();
    Ok(t)
}

/// `(paper "A4" [portrait])` or `(paper "User" W H)`: the page's width and height, KiCad's in
/// mils (A4 is 297.0022 mm wide).
fn paper(c: &mut Cursor) -> Result<(Nm, Nm)> {
    let mil = |w: Nm, h: Nm| (w * 25_400, h * 25_400);
    let (w, h) = match c.atom()? {
        "User" => return Ok((c.nm()?, c.nm()?)),
        "A5" => mil(8268, 5846),
        "A3" => mil(16535, 11693),
        "A2" => mil(23386, 16535),
        "A1" => mil(33110, 23386),
        "A0" => mil(46811, 33110),
        "A" | "USLetter" => mil(11000, 8500),
        "B" | "USLedger" => mil(17000, 11000),
        "C" => mil(22000, 17000),
        "D" => mil(34000, 22000),
        "E" => mil(44000, 34000),
        "USLegal" => mil(14000, 8500),
        _ => A4,
    };
    Ok(if c.opt_atom()? == Some("portrait") { (h, w) } else { (w, h) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::union;
    use crate::font;

    /// The centre of a text's strokes, in mm.
    fn centre(t: &Text) -> (f64, f64) {
        let (mut lo, mut hi) = (Pt { x: Nm::MAX, y: Nm::MAX }, Pt { x: Nm::MIN, y: Nm::MIN });
        font::strokes(t, |pts| pts.iter().for_each(|&p| (lo, hi) = union((lo, hi), (p, p))));
        ((lo.x as f64 + hi.x as f64) / 2e6, (lo.y as f64 + hi.y as f64) / 2e6)
    }

    /// Whether `s` shows `text` centred at `at` (mm) within a micrometre, as KiCad 10 plotted it.
    /// Hershey glyphs reach a stroke unit further each side than KiCad's, so centres compare.
    fn shows(s: &Schematic, text: &str, at: (f64, f64)) -> bool {
        s.texts.iter().filter(|t| t.text == text).any(|t| {
            let c = centre(t);
            (c.0 - at.0).abs() < 1e-3 && (c.1 - at.1).abs() < 1e-3
        })
    }

    #[test]
    fn reads_wires_junctions_sheets_the_page_and_its_title_block() {
        let s = Schematic::parse(
            r#"(kicad_sch (version 20211123) (paper "A3" portrait)
              (title_block (title "Glow \"2\"") (date "2026-10-22") (rev "B") (company "Lab")
                (comment 2 "second") (comment 1 "first") (comment 3 ""))
              (wire (pts (xy 0 0) (xy 10 0)) (stroke (width 0) (type solid)))
              (bus (pts (xy 0 5) (xy 10 5)) (stroke (width 0)))
              (bus_entry (at 10 5) (size 2.54 2.54) (stroke (width 0)))
              (junction (at 10 0) (diameter 0) (color 0 0 0 0))
              (no_connect (at 20 0))
              (sheet (at 50 50) (size 20 10) (fields_autoplaced) (stroke (width 0)) (fill (color 0 0 0 0))
                (property "Sheet name" "Power" (id 0) (at 50 49 0) (effects (font (size 1.27 1.27)) (justify left bottom)))
                (property "Sheet file" "power.kicad_sch" (id 1) (at 50 61 0) (effects (font (size 1.27 1.27)) (justify left top)))
                (pin "EN" input (at 50 55 180) (effects (font (size 1.27 1.27)) (justify left)))))"#,
            "",
        )
        .unwrap();
        assert_eq!(s.page, (297_002_200, 419_989_000));
        let comments = vec!["first".to_string(), "second".into()];
        let title = TitleBlock { title: "Glow \"2\"".into(), date: "2026-10-22".into(), rev: "B".into(), company: "Lab".into(), comments };
        assert_eq!(s.title, title);
        let of = |ink: Ink| {
            s.shapes.iter().filter(|sh| sh.layer == ink as LayerId).map(|sh| (&sh.geom, sh.width, sh.filled)).collect::<Vec<_>>()
        };
        let pt = |x: f64, y: f64| Pt { x: (x * 1e6) as Nm, y: (y * 1e6) as Nm };
        let wires = of(Ink::Wire);
        assert_eq!(wires[0], (&Geom::Segment { a: pt(0.0, 0.0), b: pt(10.0, 0.0) }, LINE, false));
        assert_eq!(wires[1], (&Geom::Segment { a: pt(10.0, 5.0), b: pt(12.54, 7.54) }, LINE, false));
        assert_eq!(wires[2], (&Geom::Circle { center: pt(10.0, 0.0), radius: JUNCTION / 2 }, 0, true));
        assert_eq!(of(Ink::Bus), [(&Geom::Segment { a: pt(0.0, 5.0), b: pt(10.0, 5.0) }, BUS, false)]);
        assert_eq!(of(Ink::NoConnect).len(), 2);
        let (name, file, bounds) = ("Power".into(), "power.kicad_sch".into(), (pt(50.0, 50.0), pt(70.0, 60.0)));
        let sheet = Sheet { name, file, bounds, uuid: String::new(), pins: vec![("EN".into(), pt(50.0, 55.0))], numbers: vec![] };
        assert_eq!(s.sheets, [sheet]);
        let texts: Vec<_> = s.texts.iter().map(|t| (Ink::ALL[t.layer as usize], t.text.as_str())).collect();
        assert_eq!(texts, [(Ink::SheetName, "Power"), (Ink::SheetFile, "File: power.kicad_sch"), (Ink::SheetPin, "EN")]);
        // KiCad 5's schematics: KiCad converts them.
        let e = Schematic::parse("EESchema Schematic File Version 4\n", "").unwrap_err();
        assert!(e.msg.contains("KiCad 6 or later"), "{e}");
    }

    #[test]
    fn places_labels_and_notes_as_kicad_does() {
        let s = Schematic::parse(
            r#"(kicad_sch (version 20250114) (paper "A2")
              (label "HEH00L" (at 30 30 0) (effects (font (size 1.27 1.27)) (justify left bottom)))
              (label "HEH03L" (at 180 30 270) (effects (font (size 1.27 1.27)) (justify right bottom)))
              (global_label "HEH000G" (shape input) (at 30 70 0) (effects (font (size 1.27 1.27)) (justify left)))
              (global_label "HEH021G" (shape passive) (at 130 100 180) (effects (font (size 1.27 1.27)) (justify right)))
              (hierarchical_label "HEH00H" (shape input) (at 30 140 0) (effects (font (size 1.27 1.27)) (justify left)))
              (hierarchical_label "HEH12H" (shape input) (at 330 140 180) (effects (font (size 2.54 2.54)) (justify right)))
              (text "HEH10T" (at 230 180 0) (effects (font (size 2.54 2.54)) (justify left bottom)))
              (text "HEH11B" (at 280 240 90) (effects (font (size 2.54 2.54) (bold yes)) (justify left bottom))))"#,
            "",
        )
        .unwrap();
        // Centres of KiCad 10's plot of these, with its default text offset and label size ratios.
        let kicad = [
            ("HEH00L", (33.8194, 28.8522)),
            ("HEH03L", (178.8522, 33.638)),
            ("HEH000G", (35.9134, 70.0353)),
            ("HEH021G", (125.0994, 100.0353)),
            ("HEH00H", (35.34035, 139.9445)),
            ("HEH12H", (319.41915, 139.897)),
            ("HEH10T", (237.47825, 178.1611)),
            ("HEH11B", (278.3926, 231.85625)),
        ];
        for (text, at) in kicad {
            assert!(shows(&s, text, at), "{text}: {:?}", s.texts.iter().find(|t| t.text == text).map(centre));
        }
        // The input label's outline, its point at the wire, and the hierarchical label's flag.
        let corners = |ink: Ink| match &s.shapes.iter().find(|sh| sh.layer == ink as LayerId).unwrap().geom {
            Geom::Poly(pts) => pts.iter().map(|p| (p.x as f64 / 1e6, p.y as f64 / 1e6)).collect::<Vec<_>>(),
            g => panic!("{g:?}"),
        };
        let near = |a: &[(f64, f64)], b: &[(f64, f64)]| {
            a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p.0 - q.0).abs() < 1e-3 && (p.1 - q.1).abs() < 1e-3)
        };
        let global = [(30.0, 70.0), (31.1113, 71.2704), (41.1539, 71.2704), (41.1539, 70.0), (41.1539, 68.7296), (31.1113, 68.7296)];
        assert!(near(&corners(Ink::GlobalLabel), &global), "{:?}", corners(Ink::GlobalLabel));
        assert!(near(&corners(Ink::HierLabel), &[(30.0, 140.0), (30.635, 140.635), (31.27, 140.635), (31.27, 139.365), (30.635, 139.365)]));
    }
}
