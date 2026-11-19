//! Symbols: their library copies in a schematic (`lib_symbols`) and their placed units, drawn as
//! KiCad draws them (pin names and numbers, fields upright).

use super::net::{self, LabelKind, PinKind};
use super::{Graphic, Ink, LINE, Legacy, Schematic, Symbol, graphic, instance, text};
use crate::board::{Geom, Nm, Pt, Text, shape_box, union};
use crate::sexpr::{Cursor, Result, unescape};
use crate::tr::tr;
use std::collections::HashMap;

/// A symbol as its library defines it, in its own frame (Y down).
pub(super) struct LibSymbol {
    items: Vec<Item>,
    /// How far pin names sit inside the body from the pin's end; 0: above the pin, numbers below.
    name_offset: Nm,
    names: bool,
    numbers: bool,
    units: u32,
    /// A power symbol's: where the net its value names joins.
    power: Option<LabelKind>,
}

/// A graphic, text or pin of `unit` (0: of every unit) in body `style` (0: both).
struct Item {
    unit: u32,
    style: u32,
    kind: Kind,
}

enum Kind {
    Graphic(Graphic),
    Text(Text),
    Pin(Pin),
}

struct Pin {
    /// Where a wire connects.
    at: Pt,
    /// Unit step from `at` along the pin to the body.
    dir: (Nm, Nm),
    length: Nm,
    look: Look,
    /// Its name's text and style (`~` or empty: none shown).
    name: Text,
    number: Option<Text>,
    /// Its number as written, also when hidden: a placed symbol selects an alternate by it.
    num: String,
    kind: PinKind,
    /// Not drawn, but connected all the same.
    hidden: bool,
    /// Its alternate functions: name, type and look.
    alternates: Vec<(String, PinKind, Look)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Line,
    Inverted,
    Clock,
    InvertedClock,
}

/// A symbol's turn and mirror: sheet = `at` + M · symbol frame.
pub(super) type Turn = [[Nm; 2]; 2];

pub(super) fn apply(m: Turn, (x, y): (Nm, Nm)) -> (Nm, Nm) {
    (m[0][0] * x + m[0][1] * y, m[1][0] * x + m[1][1] * y)
}

impl LibSymbol {
    /// Reads a library symbol's list after its name.
    pub(super) fn parse(c: &mut Cursor) -> Result<LibSymbol> {
        let mut s = LibSymbol { items: vec![], name_offset: 508_000, names: true, numbers: true, units: 1, power: None };
        s.read(c, 0, 0)?;
        Ok(s)
    }

    /// Reads a symbol's (or one unit's) items, of `unit` in body `style`.
    fn read(&mut self, c: &mut Cursor, unit: u32, style: u32) -> Result<()> {
        loop {
            while c.opt_atom()?.is_some() {}
            let Some(head) = c.child()? else { return Ok(()) };
            let kind = match head {
                "symbol" => {
                    // `NAME_unit_style`
                    let name = c.atom()?;
                    let mut parts = name.rsplit('_').map(|n| n.parse().unwrap_or(0));
                    let (style, unit) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
                    self.units = self.units.max(unit);
                    self.read(c, unit, style)?;
                    None
                }
                "pin_names" | "pin_numbers" => {
                    let mut hide = false;
                    loop {
                        // KiCad <= 8 writes `hide` as a bare flag.
                        while let Some(flag) = c.opt_atom()? {
                            hide |= flag == "hide";
                        }
                        let Some(h) = c.child()? else { break };
                        match h {
                            "offset" => self.name_offset = c.nm()?,
                            "hide" => hide = c.atom()? == "yes",
                            _ => {}
                        }
                        c.close()?;
                    }
                    *(if head == "pin_names" { &mut self.names } else { &mut self.numbers }) = !hide;
                    None
                }
                // `(power)` (KiCad <= 9: global), `(power global)` or `(power local)` (KiCad 10).
                "power" => {
                    self.power = Some(if c.opt_atom()? == Some("local") { LabelKind::Local } else { LabelKind::Global });
                    None
                }
                "polyline" | "rectangle" | "circle" | "arc" | "bezier" => {
                    let mut g = graphic(c, head)?;
                    g.geom.map(flip);
                    Some(Kind::Graphic(g))
                }
                "text" => {
                    let raw = c.atom()?;
                    let mut t = text(c, raw, Ink::Body, |_, _| Ok(()))?;
                    // Library text angles are in tenths of a degree.
                    (t.at, t.angle) = (flip(t.at), t.angle / 10.0);
                    (!t.text.is_empty()).then_some(Kind::Text(t))
                }
                "pin" => Some(Kind::Pin(pin(c)?)),
                _ => None,
            };
            if let Some(kind) = kind {
                self.items.push(Item { unit, style, kind });
            }
            c.close()?;
        }
    }
}

/// Library coordinates have Y up.
fn flip(p: Pt) -> Pt {
    Pt { x: p.x, y: -p.y }
}

/// `(pin TYPE LOOK (at x y angle) (length l) (name "n" (effects …)) (number "1" (effects …)))`.
// shortcut: the IEEE looks (input_low, output_low, clock_low, edge_clock_high, non_logic) draw as
// a plain line; draw them when sheets use them (61 of KiCad 10's 540,000 library pins).
fn pin(c: &mut Cursor) -> Result<Pin> {
    let kind = PinKind::parse(c.atom()?);
    let look = look_of(c.atom()?);
    let mut p = Pin {
        at: Pt::default(),
        dir: (1, 0),
        length: 0,
        look,
        name: Text::default(),
        number: None,
        num: String::new(),
        kind,
        hidden: false,
        alternates: vec![],
    };
    loop {
        while let Some(flag) = c.opt_atom()? {
            p.hidden |= flag == "hide";
        }
        let Some(head) = c.child()? else { break };
        match head {
            "at" => {
                p.at = flip(c.point()?);
                p.dir = match c.opt_angle()?.rem_euclid(360.0) as i32 {
                    90 => (0, -1),
                    180 => (-1, 0),
                    270 => (0, 1),
                    _ => (1, 0),
                };
            }
            "length" => p.length = c.nm()?,
            "hide" => p.hidden = c.atom()? == "yes",
            "name" => {
                let raw = c.atom()?;
                p.name = text(c, raw, Ink::PinName, |_, _| Ok(()))?;
            }
            "number" => {
                let raw = c.atom()?;
                p.num = unescape(raw);
                p.number = Some(text(c, raw, Ink::PinNumber, |_, _| Ok(()))?).filter(|t| !t.text.is_empty());
            }
            // `(alternate "NAME" type look)`
            "alternate" => {
                let name = unescape(c.atom()?);
                let kind = PinKind::parse(c.atom()?);
                p.alternates.push((name, kind, look_of(c.atom()?)));
            }
            _ => {}
        }
        c.close()?;
    }
    Ok(p)
}

fn look_of(s: &str) -> Look {
    match s {
        "inverted" => Look::Inverted,
        "clock" => Look::Clock,
        "inverted_clock" => Look::InvertedClock,
        _ => Look::Line,
    }
}

/// Reads a placed symbol and draws the unit it shows from its library copy in `lib`, with the
/// reference and unit of the sheet's instance: its own (KiCad 7+), else KiCad 6's in `legacy`.
pub(super) fn place(c: &mut Cursor, lib: &HashMap<&str, LibSymbol>, s: &mut Schematic, legacy: &Legacy) -> Result<()> {
    let (mut lib_id, mut lib_name, mut at, mut angle, mut mirror) = ("", None, Pt::default(), 0.0, "");
    let (mut unit, mut style, mut fields, mut uuid, mut placed, mut on_board) = (1, 1, vec![], "", None, true);
    let (mut on_sim, mut in_bom, mut dnp) = (true, true, false);
    // The alternate function a pin (by number) shows.
    let mut alternates = HashMap::new();
    while let Some(head) = c.child()? {
        match head {
            "lib_id" => lib_id = c.atom()?,
            "lib_name" => lib_name = Some(c.atom()?),
            "at" => (at, angle) = (c.point()?, c.opt_angle()?),
            "mirror" => mirror = c.atom()?,
            "unit" => unit = c.atom()?.parse().unwrap_or(1),
            "convert" | "body_style" => style = c.atom()?.parse().unwrap_or(1),
            "uuid" => uuid = c.atom()?,
            "on_board" => on_board = c.atom()? != "no",
            "in_bom" => in_bom = c.atom()? != "no",
            "dnp" => dnp = c.atom()? == "yes",
            "exclude_from_sim" => on_sim = c.atom()? != "yes",
            // `(pin "5" (uuid …) (alternate "SPI1_SCK"))`
            "pin" => {
                let num = c.atom()?;
                while let Some(head) = c.child()? {
                    if head == "alternate" {
                        alternates.insert(num, c.atom()?);
                    }
                    c.close()?;
                }
            }
            // `(instances (project "name" (path "/<root>/<sheet>…" (reference "R1") (unit 1))))`
            "instances" => {
                while c.child()?.is_some() {
                    while let Some(head) = c.child()? {
                        if head == "path" {
                            let path = c.atom()?;
                            let at = instance(c)?;
                            placed = placed.or((path == s.path).then_some(at));
                        }
                        c.close()?;
                    }
                    c.close()?;
                }
            }
            "property" => {
                let (name, raw) = (c.atom()?, c.atom()?);
                let ink = match name {
                    "Reference" => Ink::Reference,
                    "Value" => Ink::Value,
                    _ => Ink::Field,
                };
                let mut show_name = false;
                let t = text(c, raw, ink, |c, head| {
                    show_name |= head == "show_name" && c.opt_atom()? != Some("no");
                    Ok(())
                })?;
                fields.push((name, raw, t, show_name));
            }
            _ => {}
        }
        c.close()?;
    }
    let Some(sym) = lib.get(lib_name.unwrap_or(lib_id)) else {
        return Err(c.err(tr("a symbol's library copy is missing from lib_symbols: open and save the sheet in KiCad, which restores it")));
    };
    // KiCad 6 names a placement by its sheets' ids below the root and its own.
    let placed = placed.or_else(|| legacy.get(&format!("{}/{uuid}", below_root(&s.path))).cloned());
    if let Some((_, u)) = placed {
        unit = u;
    }
    let m = turn(angle, mirror);
    let to = |p: Pt| {
        let (x, y) = apply(m, (p.x, p.y));
        Pt { x: at.x + x, y: at.y + y }
    };
    let (first_shape, mut bounds) = (s.shapes.len(), (at, at));
    for item in sym.items.iter().filter(|i| (i.unit == 0 || i.unit == unit) && (i.style == 0 || i.style == style)) {
        match &item.kind {
            Kind::Graphic(g) => {
                let mut g = g.clone();
                g.geom.map(to);
                s.draw(&g, Ink::Body, Ink::BodyFill);
            }
            Kind::Text(t) => s.texts.push(shown(Text { at: to(t.at), ..t.clone() }, m)),
            Kind::Pin(p) => {
                let (o, d) = (to(p.at), apply(m, p.dir));
                let alternate = alternates.get(p.num.as_str()).and_then(|&a| p.alternates.iter().find(|(name, ..)| name == &unescape(a)));
                let (name, kind) = alternate.map_or((&p.name.text, p.kind), |a| (&a.0, a.1));
                let symbol = s.symbols.len();
                let (number, name, base, hidden) = (p.num.clone(), name.clone(), p.name.text.clone(), p.hidden);
                s.wiring.pins.push(net::Pin { at: o, out: (-d.0, -d.1), symbol, number, name, base, kind, hidden });
                if !p.hidden {
                    pin_marks(s, o, d, p.length, alternate.map_or(p.look, |a| a.2));
                    pin_texts(s, sym, p, alternate.map(|a| a.0.as_str()), o, d);
                    bounds = union(bounds, (o, o));
                }
            }
        }
    }
    bounds = s.shapes[first_shape..].iter().map(shape_box).fold(bounds, union);
    let (mut reference, mut designator, mut power, mut value, mut footprint) =
        (String::new(), String::new(), None, String::new(), String::new());
    let mut sim = vec![];
    for (name, raw, t, show_name) in fields {
        // KiCad 6 kept a part's model in `Spice_` fields.
        if let Some(key) = name.strip_prefix("Sim.").or(name.starts_with("Spice_").then_some(name)) {
            sim.push((key.to_string(), unescape(raw)));
        }
        // KiCad 5's `~` for an empty field still shows nothing.
        let mut shown_text = Some(t).filter(|t| !t.text.is_empty() && t.text != "~").map(|t| shown(t, m));
        if name == "Reference" {
            designator = placed.as_ref().map_or_else(|| unescape(raw), |p| p.0.clone());
            reference = designator.clone() + &unit_letter(unit, sym.units);
            if let Some(t) = &mut shown_text {
                t.text = reference.clone();
            }
        }
        match name {
            "Value" => (value, power) = (unescape(raw), sym.power.map(|kind| (kind, unescape(raw)))),
            "Footprint" => footprint = unescape(raw),
            _ => {}
        }
        if let Some(mut t) = shown_text {
            if show_name {
                t.text = format!("{name}: {}", t.text);
            }
            s.texts.push(t);
        }
    }
    let link = format!("{}/{uuid}", below_root(&s.path));
    let (lib_id, span) = (lib_id.to_string(), 0..0); // the span is the caller's, who closes the list
    s.symbols.push(Symbol {
        reference,
        bounds,
        designator,
        value,
        footprint,
        link,
        lib_id,
        span,
        power,
        on_board,
        in_bom,
        dnp,
        unit,
        sim,
        on_sim,
    });
    Ok(())
}

impl Symbol {
    /// Whether a footprint stands for it on the board: it isn't a power symbol or flag
    /// (`#PWR01`), nor kept off the board (KiCad's Exclude from board).
    pub fn on_board(&self) -> bool {
        self.on_board && !self.designator.starts_with('#')
    }
}

impl Schematic {
    /// The symbol whose box holds `p`, or is within `tol` of it: the smallest.
    pub fn symbol_at(&self, p: Pt, tol: Nm) -> Option<&Symbol> {
        let near = |v: Nm, lo: Nm, hi: Nm| (lo as i64 - tol as i64..=hi as i64 + tol as i64).contains(&(v as i64));
        let area = |(lo, hi): (Pt, Pt)| (hi.x as i64 - lo.x as i64) * (hi.y as i64 - lo.y as i64);
        let hits = self.symbols.iter().filter(|s| near(p.x, s.bounds.0.x, s.bounds.1.x) && near(p.y, s.bounds.0.y, s.bounds.1.y));
        hits.min_by_key(|s| area(s.bounds))
    }
}

/// An instance path below the root sheet, as KiCad 6 names placements: `/<sheet uuid>…`, empty
/// for the root's own.
pub(super) fn below_root(path: &str) -> &str {
    path.get(1..).and_then(|p| p.find('/')).map_or("", |i| &path[i + 1..])
}

/// KiCad's unit letters: none for a one-unit part, then A to Z, AA, AB, ….
fn unit_letter(unit: u32, units: u32) -> String {
    if units <= 1 {
        return String::new();
    }
    let mut n = unit.max(1);
    let mut out = vec![];
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.iter().rev().map(|&b| b as char).collect()
}

/// The symbol frame's turn by `angle` degrees (counter-clockwise on the sheet), then its
/// mirror: `x` flips Y, `y` flips X.
pub(super) fn turn(angle: f64, mirror: &str) -> Turn {
    let r = match angle.rem_euclid(360.0) as i32 {
        90 => [[0, 1], [-1, 0]],
        180 => [[-1, 0], [0, -1]],
        270 => [[0, -1], [1, 0]],
        _ => [[1, 0], [0, 1]],
    };
    let f = match mirror {
        "x" => [[1, 0], [0, -1]],
        "y" => [[-1, 0], [0, 1]],
        _ => return r,
    };
    [0, 1].map(|i| [0, 1].map(|j| f[i][0] * r[0][j] + f[i][1] * r[1][j]))
}

/// Text of a symbol (in its frame's angle and justification) as KiCad shows it once the symbol
/// turns by `m`: upright, horizontal or vertical, its justification following its box.
pub(super) fn shown(mut t: Text, m: Turn) -> Text {
    let (s, c) = t.angle.to_radians().sin_cos();
    let along = apply(m, (c.round() as Nm, -s.round() as Nm));
    let down = apply(m, (s.round() as Nm, c.round() as Nm));
    // Shown reading along +X, or up the sheet (-Y), with its glyphs' down +Y or +X.
    let vertical = along.0 == 0;
    let (u, v) = if vertical { ((0, -1), (1, 0)) } else { ((1, 0), (0, 1)) };
    let dot = |a: (Nm, Nm), b: (Nm, Nm)| (a.0 * b.0 + a.1 * b.1) as i8;
    t.justify = (t.justify.0 * dot(along, u), t.justify.1 * dot(down, v));
    t.angle = if vertical { 90.0 } else { 0.0 };
    t
}

/// A pin's line and its inverted bubble or clock mark, from where it connects (`o`) along `d`.
fn pin_marks(s: &mut Schematic, o: Pt, d: (Nm, Nm), length: Nm, look: Look) {
    let at = |t: Nm, n: Nm| Pt { x: o.x + d.0 * t - d.1 * n, y: o.y + d.1 * t + d.0 * n };
    // KiCad's default pin mark size (25 mil): the bubble's radius, half the clock's width.
    const MARK: Nm = 635_000;
    let bubble = matches!(look, Look::Inverted | Look::InvertedClock);
    let line_end = if bubble { (length - 2 * MARK).max(0) } else { length };
    if length > 0 {
        s.shape(Geom::Segment { a: o, b: at(line_end, 0) }, LINE, false, Ink::Body);
    }
    if bubble {
        s.shape(Geom::Circle { center: at(length - MARK, 0), radius: MARK }, LINE, false, Ink::Body);
    }
    if matches!(look, Look::Clock | Look::InvertedClock) {
        for side in [-MARK, MARK] {
            s.shape(Geom::Segment { a: at(length, side), b: at(length + 2 * MARK, 0) }, LINE, false, Ink::Body);
        }
    }
}

/// A pin's name (or the `alternate` function the placed symbol selects) and number, placed as
/// KiCad places them.
fn pin_texts(s: &mut Schematic, sym: &LibSymbol, p: &Pin, alternate: Option<&str>, o: Pt, d: (Nm, Nm)) {
    let vertical = d.0 == 0;
    // Along the pin and to its "above" side (left of a vertical pin), in sheet coordinates.
    let at = |t: Nm, n: Nm| if vertical { Pt { x: o.x - n, y: o.y + d.1 * t } } else { Pt { x: o.x + d.0 * t, y: o.y - n } };
    let mid = p.length / 2;
    // A pin's texts sit 8 mil and half the pens off its line (measured from KiCad 10 plots).
    let gap = |t: &Text| 203_200 + (LINE + t.thickness) / 2;
    let set = |t: &Text, at: Pt, justify: (i8, i8)| Text { at, angle: if vertical { 90.0 } else { 0.0 }, justify, ..t.clone() };
    // Reading direction: +X, or up for a vertical pin; `inward` is +1 when the body lies that way.
    let inward = if vertical { -d.1 } else { d.0 } as i8;
    let name = alternate.map(|a| Text { text: a.into(), ..p.name.clone() }).or_else(|| Some(p.name.clone()));
    let name = name.filter(|t| sym.names && !t.text.is_empty() && t.text != "~");
    let (name, number) = (name.as_ref(), p.number.as_ref().filter(|_| sym.numbers));
    if sym.name_offset > 0 {
        if let Some(t) = name {
            s.texts.push(set(t, at(p.length + sym.name_offset, 0), (-inward, 0)));
        }
        if let Some(t) = number {
            s.texts.push(set(t, at(mid, gap(t)), (0, 1)));
        }
    } else {
        if let Some(t) = name {
            s.texts.push(set(t, at(mid, gap(t)), (0, 1)));
        }
        if let Some(t) = number {
            let (side, j) = if name.is_some() { (-gap(t), -1) } else { (gap(t), 1) };
            s.texts.push(set(t, at(mid, side), (0, j)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::MM;
    use crate::font;

    /// Whether `s` shows `text` centred at `at` (mm) within `tol`, as KiCad 10 plotted it: Hershey
    /// glyphs reach a stroke unit further each side than KiCad's, so centres compare.
    fn shows(s: &Schematic, text: &str, at: (f64, f64), tol: f64) -> bool {
        s.texts.iter().filter(|t| t.text == text).any(|t| {
            let (mut lo, mut hi) = (Pt { x: Nm::MAX, y: Nm::MAX }, Pt { x: Nm::MIN, y: Nm::MIN });
            font::strokes(t, |pts| pts.iter().for_each(|&p| (lo, hi) = union((lo, hi), (p, p))));
            ((lo.x + hi.x) as f64 / 2e6 - at.0).abs() < tol && ((lo.y + hi.y) as f64 / 2e6 - at.1).abs() < tol
        })
    }

    #[test]
    fn places_pins_and_fields_as_kicad_does() {
        let field = |x, y| format!(r#"(property "Reference" "HRH" (at {x} {y} 0) (effects (font (size 1.27 1.27)) (justify left)))"#);
        let s = Schematic::parse(
            &format!(
                r#"(kicad_sch (version 20250114) (lib_symbols
                  (symbol "T:P1" (pin_names (offset 0.508))
                    (symbol "P1_0_1" (rectangle (start -5.08 5.08) (end 5.08 -5.08) (stroke (width 0.254)) (fill (type background))))
                    (symbol "P1_1_1"
                      (pin input line (at -7.62 2.54 0) (length 2.54) (name "HL" (effects (font (size 1.27 1.27)))) (number "HHL" (effects (font (size 1.27 1.27)))))
                      (pin input line (at -2.54 7.62 270) (length 2.54) (name "HT" (effects (font (size 1.27 1.27)))) (number "HHT" (effects (font (size 1.27 1.27)))))
                      (pin input clock (at 2.54 7.62 270) (length 2.54) (name "~" (effects (font (size 1.27 1.27)))) (number "1" (effects (font (size 1.27 1.27)) (hide yes))))
                      (pin input inverted (at 7.62 -2.54 180) (length 2.54) (name "~" (effects (font (size 1.27 1.27)))) (number "2" (effects (font (size 1.27 1.27)) (hide yes))))))
                  (symbol "T:X" (symbol "X_1_1"))
                  (symbol "T:Y" (symbol "Y_1_1") (symbol "Y_2_1")))
                  (symbol (lib_id "T:P1") (at 40 50 0) (unit 1))
                  (symbol (lib_id "T:X") (at 70 25 90) (unit 1) {})
                  (symbol (lib_id "T:X") (at 250 25 90) (mirror x) (unit 1) {})
                  (symbol (lib_id "T:X") (at 295 25 180) (mirror x) (unit 1) {})
                  (symbol (lib_id "T:Y") (at 0 0 0) (unit 2) (property "Reference" "U1" (at 0 0 0)) (property "Value" "~" (at 0 0 0))))"#,
                field(75, 17),
                field(255, 17),
                field(300, 17)
            ),
            "",
        )
        .unwrap();
        // Pin names inside the body, numbers above their lines (left of vertical ones).
        for (text, at) in
            [("HL", (36.79815, 47.4045)), ("HHL", (33.7407, 46.3059)), ("HT", (37.4045, 46.5865)), ("HHT", (36.3059, 43.55925))]
        {
            assert!(shows(&s, text, at, 1e-3), "{text}");
        }
        // A clock's mark, an inverted pin's bubble.
        let mm = |x: f64, y: f64| Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm };
        let has = |g: Geom| s.shapes.iter().any(|sh| sh.geom == g);
        assert!(
            has(Geom::Segment { a: mm(43.175, 44.92), b: mm(42.54, 46.19) })
                && has(Geom::Segment { a: mm(41.905, 44.92), b: mm(42.54, 46.19) })
        );
        assert!(has(Geom::Circle { center: mm(45.715, 52.54), radius: 635_000 }));
        // Fields stay upright as their symbol turns and mirrors (KiCad's R sits 11 µm off ours);
        // the second of two units is B.
        for at in [(74.9445, 14.9233), (254.9445, 19.0767), (297.9232, 16.9445)] {
            assert!(shows(&s, "HRH", at, 0.02), "{at:?}");
        }
        assert_eq!(s.symbols.last().map(|s| s.reference.as_str()), Some("U1B"));
        assert!(s.texts.iter().all(|t| t.text != "~"));
    }

    #[test]
    fn shows_the_alternate_function_a_pin_selects() {
        let s = Schematic::parse(
            r#"(kicad_sch (version 20250114) (lib_symbols (symbol "T:A" (symbol "A_1_1"
                (pin bidirectional line (at 0 0 0) (length 2.54) (name "PA5" (effects (font (size 1.27 1.27))))
                  (number "1" (effects (font (size 1.27 1.27)))) (alternate "SPI1_SCK" bidirectional clock)))))
              (symbol (lib_id "T:A") (at 0 0 0) (unit 1) (pin "1" (uuid "p") (alternate "SPI1_SCK"))))"#,
            "",
        )
        .unwrap();
        // Its name and look, as KiCad shows them: a clock's two strokes besides the pin's line.
        let names: Vec<_> = s.texts.iter().filter(|t| t.layer == Ink::PinName as u8).map(|t| t.text.as_str()).collect();
        assert_eq!((names, s.shapes.len()), (vec!["SPI1_SCK"], 3));
    }

    #[test]
    fn knows_the_parts_kept_off_the_board() {
        let s = Schematic::parse(
            r##"(kicad_sch (version 20250114) (lib_symbols (symbol "T:X" (symbol "X_1_1")))
              (symbol (lib_id "T:X") (at 0 0 0) (unit 1) (on_board no) (property "Reference" "J3" (at 0 0 0)))
              (symbol (lib_id "T:X") (at 0 0 0) (unit 1) (on_board yes) (property "Reference" "#PWR01" (at 0 0 0)))
              (symbol (lib_id "T:X") (at 0 0 0) (unit 1) (on_board yes) (property "Reference" "R1" (at 0 0 0)))
              (symbol (lib_id "T:X") (at 0 0 0) (unit 1) (property "Reference" "R2" (at 0 0 0))))"##,
            "",
        )
        .unwrap();
        assert_eq!(s.symbols.iter().map(Symbol::on_board).collect::<Vec<_>>(), [false, false, true, true]);
    }

    #[test]
    fn picks_the_smallest_symbol_under_a_point() {
        let sym = |r: &str, (x0, y0), (x1, y1)| {
            let bounds = (Pt { x: x0, y: y0 }, Pt { x: x1, y: y1 });
            let (value, footprint, link) = (String::new(), String::new(), String::new());
            let (unit, sim, on_sim) = (1, vec![], true);
            Symbol {
                reference: r.into(),
                bounds,
                designator: r.into(),
                value,
                footprint,
                link,
                lib_id: String::new(),
                span: 0..0,
                power: None,
                on_board: true,
                in_bom: true,
                dnp: false,
                unit,
                sim,
                on_sim,
            }
        };
        // A resistor on the end of an IC's pin, inside the IC's box.
        let s = Schematic { symbols: vec![sym("U1", (0, 0), (100, 100)), sym("R1", (90, 40), (130, 60))], ..Schematic::default() };
        let at = |x, y| s.symbol_at(Pt { x, y }, 5).map(|s| s.reference.as_str());
        assert_eq!([at(95, 50), at(50, 50), at(134, 50), at(136, 50)], [Some("R1"), Some("U1"), Some("R1"), None]);
    }

    #[test]
    fn turns_then_mirrors_like_kicad() {
        // A point right of the anchor, turned 90° counter-clockwise, points up the sheet (-Y).
        assert_eq!(apply(turn(90.0, ""), (MM, 0)), (0, -MM));
        // Mirrored about X after the turn, it points down.
        assert_eq!(apply(turn(90.0, "x"), (MM, 0)), (0, MM));
        assert_eq!(apply(turn(0.0, "y"), (MM, 0)), (-MM, 0));
        assert_eq!(unit_letter(2, 2) + &unit_letter(27, 30) + &unit_letter(1, 1), "BAA");
    }
}
