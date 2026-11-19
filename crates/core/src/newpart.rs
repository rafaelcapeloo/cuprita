//! A new part from a few facts: its symbol drawn from its pins (a box sized to their names, as
//! KiCad 10 writes a symbol), its footprint one of the libraries' or made from a package
//! (`newpart/footprint.rs`); both go into the project's own library as a vendor's part does
//! (`parts::import`).

use crate::board::{Nm, Pt, Text};
use crate::parts::Imported;
use crate::sexpr::escape;
use crate::tr::{tr, trf};
use crate::units::nm_to_mm;
use std::path::Path;

mod footprint;
pub use footprint::{Kind, PRESETS, Package};

/// KiCad's electrical types of a pin, as its files name them.
pub const TYPES: [&str; 12] = [
    "input",
    "output",
    "bidirectional",
    "tri_state",
    "passive",
    "free",
    "unspecified",
    "power_in",
    "power_out",
    "open_collector",
    "open_emitter",
    "no_connect",
];

/// The side of the symbol's box a pin sticks out of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// A pin of the symbol, and the footprint's pad of its number.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub number: String,
    pub name: String,
    /// One of `TYPES`.
    pub kind: &'static str,
    pub side: Side,
}

/// What the part's footprint is.
#[derive(Clone, Debug, PartialEq)]
pub enum Footprint {
    /// One of the libraries', as `lib:name`.
    Library(String),
    /// One made from a package, with a pad for each pin.
    Made(Package),
}

/// A part to make: its symbol's name (and value), reference prefix (`U`), what it is, its
/// datasheet, pins and footprint.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub reference: String,
    pub description: String,
    pub datasheet: String,
    pub pins: Vec<Pin>,
    pub footprint: Footprint,
}

/// Pins `1` to `n`, unnamed and passive, the first half on the left, the rest on the right.
pub fn pins(n: usize) -> Vec<Pin> {
    let side = |i| if i < n.div_ceil(2) { Side::Left } else { Side::Right };
    (0..n).map(|i| Pin { number: (i + 1).to_string(), name: String::new(), kind: "passive", side: side(i) }).collect()
}

/// Puts `part` into the own library of project `project` in folder `dir`, as `parts::import`
/// puts a vendor's: its symbol, linked to its footprint, and the footprint made for it.
pub fn create(dir: &Path, project: &str, part: &Part) -> Result<Imported, String> {
    crate::parts::import(dir, project, part.files()?)
}

/// Pins are 100 mil apart, their ends on the 50 mil grid, 100 mil long.
const PITCH: Nm = 2_540_000;
const GRID: Nm = 1_270_000;
const LENGTH: Nm = 2_540_000;
/// KiCad's text size, and how far inside the box a pin's name starts.
const TEXT: Nm = 1_270_000;
const OFFSET: Nm = 508_000;

/// A symbol laid out: its box (lowest corner, highest; library Y up), and each pin's end (where
/// wires join it) by `Part::pins`' order; Reference and Value go above and below it.
#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    pub body: (Pt, Pt),
    pub pins: Vec<Pt>,
    pub reference: Pt,
    pub value: Pt,
}

impl Part {
    /// Why it can't be made, if it can't: a name, a reference prefix, pins of their own numbers,
    /// and a footprint (a made one's pads those pins' numbers).
    pub fn check(&self) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(tr("Give the part a name, as LM358: its symbol's name and its value").into());
        }
        if name.contains(char::is_whitespace) {
            return Err(tr("A part's name can't hold spaces in KiCad's libraries: use - or _").into());
        }
        if let Some(c) = name.chars().find(|c| matches!(c, ':' | '/' | '\\' | '"')) {
            return Err(trf("A part's name can't hold \"{0}\" in KiCad's libraries: use - or _", &[&c]));
        }
        let prefix = self.reference.trim();
        if prefix.is_empty() || prefix.chars().any(|c| c.is_ascii_digit() || c.is_whitespace()) {
            return Err(tr("The reference prefix is letters, as U, J, D or Q: KiCad numbers the parts after it").into());
        }
        if self.pins.is_empty() {
            return Err(tr("Give the part its pins: type how many, then Fill").into());
        }
        for (i, pin) in self.pins.iter().enumerate() {
            if pin.number.trim().is_empty() {
                return Err(trf("The pin on row {0} has no number: give it one, as {0} or A1", &[&(i + 1)]));
            }
            if let Some(j) = self.pins[..i].iter().position(|p| p.number.trim() == pin.number.trim()) {
                return Err(trf("Pin number {0} is on rows {1} and {2}: give each pin its own", &[&pin.number.trim(), &(j + 1), &(i + 1)]));
            }
        }
        match &self.footprint {
            Footprint::Library(id) if !id.contains(':') => Err(tr("Choose the part's footprint, or make one from a package").into()),
            Footprint::Library(_) => Ok(()),
            Footprint::Made(package) => {
                package.footprint(self.pads())?;
                // Its pads are numbered 1 to n, as `1`, not `01`.
                let n = self.pins.len();
                let pad = |s: &str| s.parse::<usize>().ok().filter(|k| (1..=n).contains(k) && k.to_string() == s);
                match self.pins.iter().find(|p| pad(p.number.trim()).is_none()) {
                    Some(pin) => Err(trf("Pin {0} has no pad: the footprint's are numbered 1 to {1}", &[&pin.number.trim(), &n])),
                    None => Ok(()),
                }
            }
        }
    }

    /// How many pads a made footprint has round it: a pad for each pin, the last one the exposed
    /// pad when it has one.
    pub fn pads(&self) -> usize {
        match &self.footprint {
            Footprint::Made(p) if p.exposed_pad().is_some() => self.pins.len().saturating_sub(1),
            _ => self.pins.len(),
        }
    }

    /// Its symbol laid out: the pins 100 mil apart down the left and right sides and across the
    /// top and bottom, in their order, from the top or the left; the box as big as the pins and
    /// their names inside it need, its edges on the 50 mil grid, centred on the origin.
    pub fn symbol(&self) -> Symbol {
        let on = |side| self.pins.iter().enumerate().filter(move |(_, p)| p.side == side).map(|(i, _)| i).collect::<Vec<_>>();
        let (left, right, top, bottom) = (on(Side::Left), on(Side::Right), on(Side::Top), on(Side::Bottom));
        let names = |pins: &[usize]| pins.iter().map(|&i| name_width(&self.pins[i].name)).max().unwrap_or(0);
        // Room a side's names take inside the box: from its edge to their far end, and a gap.
        let room = |pins: &[usize]| match names(pins) {
            0 => 0,
            w => w + OFFSET,
        };
        let rows = left.len().max(right.len()) as Nm;
        let cols = top.len().max(bottom.len()) as Nm;
        // Top and bottom pins' names run along them, inside the box above and below the side pins.
        let margin = |pins: &[usize]| up((room(pins) + GRID).max(PITCH), GRID);
        let (above, below) = (margin(&top), margin(&bottom));
        let height = (rows - 1).max(0) * PITCH + above + below;
        let width = (room(&left) + room(&right) + PITCH).max((cols + 1) * PITCH).max(2 * PITCH);
        // Its edges on the grid, half on each side of the origin.
        let (x, top_y) = (up((width + 1) / 2, GRID), up(height / 2, GRID));
        let bottom_y = top_y - height;
        let mut pins = vec![Pt::default(); self.pins.len()];
        for (k, &i) in left.iter().enumerate() {
            pins[i] = Pt { x: -x - LENGTH, y: top_y - above - k as Nm * PITCH };
        }
        for (k, &i) in right.iter().enumerate() {
            pins[i] = Pt { x: x + LENGTH, y: top_y - above - k as Nm * PITCH };
        }
        let across = |n: usize, k: usize| -(n as Nm - 1) * PITCH / 2 + k as Nm * PITCH;
        for (k, &i) in top.iter().enumerate() {
            pins[i] = Pt { x: across(top.len(), k), y: top_y + LENGTH };
        }
        for (k, &i) in bottom.iter().enumerate() {
            pins[i] = Pt { x: across(bottom.len(), k), y: bottom_y - LENGTH };
        }
        let past = |pins: &[usize]| if pins.is_empty() { GRID } else { LENGTH + GRID };
        Symbol {
            body: (Pt { x: -x, y: bottom_y }, Pt { x, y: top_y }),
            pins,
            reference: Pt { x: -x, y: top_y + past(&top) },
            value: Pt { x: -x, y: bottom_y - past(&bottom) },
        }
    }

    /// Its symbol library and footprint files, as `parts::import` takes them, or why it can't be
    /// made: the symbol's Footprint names the footprint, which a made one's goes in with.
    pub fn files(&self) -> Result<Vec<(String, Vec<u8>)>, String> {
        self.check()?;
        let made = match &self.footprint {
            Footprint::Made(package) => Some(package.footprint(self.pads())?),
            Footprint::Library(_) => None,
        };
        let made = made.map(|(name, text)| (format!("{name}.kicad_mod"), text.into_bytes()));
        Ok([(format!("{}.kicad_sym", self.name.trim()), self.library().into_bytes())].into_iter().chain(made).collect())
    }

    /// Its symbol, in a library as KiCad 10 writes one, its Footprint field naming its footprint
    /// (made, by its name; as `parts::import` lists it once it goes in with the symbol).
    pub fn library(&self) -> String {
        let footprint = match &self.footprint {
            Footprint::Library(id) => id.clone(),
            Footprint::Made(package) => package.name(self.pads()),
        };
        let (s, name) = (self.symbol(), self.name.trim());
        let xy = |p: Pt| format!("{} {}", nm_to_mm(p.x), nm_to_mm(p.y));
        let below = |k: Nm| Pt { x: s.value.x, y: s.value.y - k * PITCH };
        let fields = [
            ("Reference", self.reference.trim(), s.reference, false),
            ("Value", name, s.value, false),
            ("Footprint", footprint.as_str(), below(1), true),
            ("Datasheet", self.datasheet.trim(), below(2), true),
            ("Description", self.description.trim(), below(3), true),
        ];
        let mut out = format!(
            "\t(symbol \"{}\"\n\t\t(exclude_from_sim no)\n\t\t(in_bom yes)\n\t\t(on_board yes)\n\t\t(in_pos_files yes)\n\t\t(duplicate_pin_numbers_are_jumpers no)\n",
            escape(name)
        );
        for (field, value, at, hidden) in fields {
            let hide = if hidden { "\t\t\t(hide yes)\n" } else { "" };
            out += &format!(
                "\t\t(property \"{field}\" \"{}\"\n\t\t\t(at {} 0)\n\t\t\t(show_name no)\n\t\t\t(do_not_autoplace no)\n{hide}\t\t\t(effects\n\t\t\t\t(font\n\t\t\t\t\t(size 1.27 1.27)\n\t\t\t\t)\n\t\t\t\t(justify left)\n\t\t\t)\n\t\t)\n",
                escape(value),
                xy(at)
            );
        }
        let (lo, hi) = s.body;
        out += &format!(
            "\t\t(symbol \"{0}_0_1\"\n\t\t\t(rectangle\n\t\t\t\t(start {1})\n\t\t\t\t(end {2})\n\t\t\t\t(stroke\n\t\t\t\t\t(width 0.254)\n\t\t\t\t\t(type default)\n\t\t\t\t)\n\t\t\t\t(fill\n\t\t\t\t\t(type background)\n\t\t\t\t)\n\t\t\t)\n\t\t)\n\t\t(symbol \"{0}_1_1\"\n",
            escape(name),
            xy(Pt { x: lo.x, y: hi.y }),
            xy(Pt { x: hi.x, y: lo.y })
        );
        let font = "\t\t\t\t\t(effects\n\t\t\t\t\t\t(font\n\t\t\t\t\t\t\t(size 1.27 1.27)\n\t\t\t\t\t\t)\n\t\t\t\t\t)\n";
        for (pin, at) in self.pins.iter().zip(&s.pins) {
            // Pointing into the box: right from the left side, down from the top.
            let angle = match pin.side {
                Side::Left => 0,
                Side::Right => 180,
                Side::Top => 270,
                Side::Bottom => 90,
            };
            out += &format!(
                "\t\t\t(pin {} line\n\t\t\t\t(at {} {angle})\n\t\t\t\t(length 2.54)\n\t\t\t\t(name \"{}\"\n{font}\t\t\t\t)\n\t\t\t\t(number \"{}\"\n{font}\t\t\t\t)\n\t\t\t)\n",
                pin.kind,
                xy(*at),
                escape(pin.name.trim()),
                escape(pin.number.trim())
            );
        }
        let v = env!("CARGO_PKG_VERSION");
        format!(
            "(kicad_symbol_lib\n\t(version 20251024)\n\t(generator \"cuprita\")\n\t(generator_version \"{v}\")\n{out}\t\t)\n\t\t(embedded_fonts no)\n\t)\n)\n"
        )
    }
}

/// How far a pin's name runs in KiCad's font at its size.
fn name_width(name: &str) -> Nm {
    let text = Text { text: name.trim().to_string(), size: (TEXT, TEXT), ..Text::default() };
    crate::font::width(&text).ceil() as Nm
}

/// `v` (0 or more) up to a multiple of `step`.
fn up(v: Nm, step: Nm) -> Nm {
    (v + step - 1).div_euclid(step) * step
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::Project;

    /// An 8-pin part: inputs on the left, outputs on the right, power above and below.
    fn part(footprint: Footprint) -> Part {
        let mut pins = pins(8);
        let named = [("IN+", "input"), ("IN-", "input"), ("~{SHDN}", "input"), ("VCC", "power_in")];
        let more = [("OUT", "output"), ("REF", "passive"), ("NC", "no_connect"), ("GND", "power_in")];
        for (pin, (name, kind)) in pins.iter_mut().zip(named.into_iter().chain(more)) {
            (pin.name, pin.kind) = (name.into(), kind);
        }
        (pins[3].side, pins[7].side) = (Side::Top, Side::Bottom);
        let text = |s: &str| s.to_string();
        Part {
            name: text("AMP1"),
            reference: text("U"),
            description: text("An amplifier"),
            datasheet: text("https://x.org/a.pdf"),
            pins,
            footprint,
        }
    }

    #[test]
    fn lays_its_symbol_out_on_the_grid_round_its_pins_names() {
        let p = part(Footprint::Library("Package_SO:SOIC-8_3.9x4.9mm_P1.27mm".into()));
        let s = p.symbol();
        let (lo, hi) = s.body;
        for at in s.pins.iter().chain([&lo, &hi]) {
            assert!(at.x % GRID == 0 && at.y % GRID == 0, "{at:?} off the 50 mil grid");
        }
        // Left and right pins from the top down, 100 mil apart, 100 mil out of the box.
        let left: Vec<Pt> = (0..3).map(|k| Pt { x: lo.x - LENGTH, y: s.pins[0].y - k * PITCH }).collect();
        assert_eq!(s.pins[..3], left[..]);
        assert_eq!((s.pins[4].x, s.pins[4].y, s.pins[5].y), (hi.x + LENGTH, s.pins[0].y, s.pins[0].y - PITCH));
        assert_eq!((s.pins[3], s.pins[7]), (Pt { x: 0, y: hi.y + LENGTH }, Pt { x: 0, y: lo.y - LENGTH }));
        // The names fit inside, apart; VCC's runs down from the top above the side pins' names.
        let (inside, names) = (hi.x - lo.x - 2 * OFFSET, name_width("~{SHDN}") + name_width("OUT"));
        assert!(inside >= names + PITCH && hi.y - s.pins[0].y >= name_width("VCC") + OFFSET + GRID, "{s:?}");
        assert!(s.reference.y > hi.y + LENGTH && s.value.y < lo.y - LENGTH);
        // Two pins: a small box.
        let two = Part { pins: pins(2), ..p };
        assert_eq!(two.symbol().body, (Pt { x: -PITCH, y: -PITCH }, Pt { x: PITCH, y: PITCH }));
    }

    #[test]
    fn makes_a_part_kicad_and_cuprita_read_into_the_projects_library() {
        let dir = std::env::temp_dir().join(format!("cuprita-newpart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let made = || part(Footprint::Made((PRESETS[0].1)(8)));
        let done = create(&dir, "p", &made()).unwrap();
        let fp = "p:SOIC-8_3.9x4.9mm_P1.27mm";
        assert_eq!((done.symbols, done.footprints), (vec!["p:AMP1".to_string()], vec![fp.to_string()]));
        // The project lists them, the symbol linked to the footprint as listed.
        let symbols = crate::parts::symbols(&dir);
        let lib = std::fs::read_to_string(&symbols[0].path).unwrap();
        let symbol = crate::library::symbol(&lib, "AMP1").unwrap();
        assert_eq!(crate::library::field(&symbol, "Footprint").as_deref(), Some(fp));
        assert_eq!((symbols[0].about.as_str(), crate::parts::footprints(&dir)[0].id()), ("An amplifier", fp.to_string()));
        // Placed on a sheet, its pins are where the layout puts them, each on a net of its own.
        let [_, (root, sch), _] = crate::new::files("p", 2, (50.0, 50.0), 1);
        let mut project = Project::load(&root, "", |_| Ok(sch.clone())).unwrap();
        let at = Pt { x: 50_800_000, y: 50_800_000 };
        project.add_symbol(0, "p", &symbol, &[], at, 0.0).unwrap();
        let sheet = &project.pages[0].sheet;
        for (pin, end) in made().pins.iter().zip(made().symbol().pins) {
            // The library's Y is up, the sheet's down.
            assert_eq!(sheet.pin_at(Pt { x: at.x + end.x, y: at.y - end.y }), Some(("U1", pin.number.as_str())));
        }
        assert_eq!(project.erc().iter().filter(|v| v.kind == "pin_not_connected").count(), 7);
        // Made again, it is there already.
        assert_eq!(create(&dir, "p", &made()).unwrap().symbols, ["p:AMP1"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn says_why_a_part_cant_be_made() {
        let p = part(Footprint::Made((PRESETS[0].1)(8)));
        let fails = |change: &dyn Fn(&mut Part)| {
            let mut p = p.clone();
            change(&mut p);
            p.files().unwrap_err()
        };
        assert_eq!(fails(&|p| p.name = " ".into()), "Give the part a name, as LM358: its symbol's name and its value");
        assert_eq!(fails(&|p| p.name = "AMP 1".into()), "A part's name can't hold spaces in KiCad's libraries: use - or _");
        assert_eq!(fails(&|p| p.name = "A:B".into()), "A part's name can't hold \":\" in KiCad's libraries: use - or _");
        assert!(fails(&|p| p.reference = "U1".into()).starts_with("The reference prefix is letters"));
        assert_eq!(fails(&|p| p.pins.clear()), "Give the part its pins: type how many, then Fill");
        assert_eq!(fails(&|p| p.pins[6].number = " ".into()), "The pin on row 7 has no number: give it one, as 7 or A1");
        assert_eq!(fails(&|p| p.pins[6].number = "2".into()), "Pin number 2 is on rows 2 and 7: give each pin its own");
        assert_eq!(fails(&|p| p.pins[6].number = "9".into()), "Pin 9 has no pad: the footprint's are numbered 1 to 8");
        assert_eq!(fails(&|p| p.footprint = Footprint::Library(String::new())), "Choose the part's footprint, or make one from a package");
        let quad = |p: &mut Part| p.footprint = Footprint::Made((PRESETS[3].1)(8));
        assert_eq!(fails(&quad), "A QFN has as many pads on each side, and its exposed pad: give it 5, 9, 13, 17… pins (now 8)");
        // Its exposed pad is the last pin: 8 pins round it and the pad make 9.
        let mut nine = p.clone();
        quad(&mut nine);
        nine.pins.push(Pin { number: "9".into(), name: "EP".into(), kind: "passive", side: Side::Bottom });
        assert!(nine.files().is_ok(), "{:?}", nine.files());
    }
}
