//! A footprint made from a package's numbers, as KiCad 10 writes one (`.kicad_mod`): its pads
//! laid out and numbered as its family does; the courtyard 0.25 mm round the pads and body, on a
//! 0.01 mm grid (KiCad's libraries' rule); the body on the fab layer, pin 1's corner cut; its
//! outline on the silkscreen kept 0.2 mm off the pads, pin 1 marked; the reference on the
//! silkscreen, the value and `${REFERENCE}` on the fab layer.

use crate::board::{Nm, Pt};
use crate::tr::{t, tr, trf};
use crate::units::nm_to_mm;

/// How a package's pads are laid out and numbered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// SMD pads in two rows, 1 at the top of the left one, on round counter-clockwise (SOIC,
    /// SSOP, TSSOP, DFN).
    Dual,
    /// SMD pads on four sides, as many on each, numbered as `Dual` (QFN, QFP).
    Quad,
    /// Through-hole pins in one or two rows, 1 at the top left, numbered across the rows (pin
    /// headers: 1 2 / 3 4).
    Header,
    /// Through-hole pins in two rows, numbered as `Dual` (DIP).
    Dip,
}

/// A package's numbers, which its footprint is made from.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    /// The family it is named after: `SOIC`, `QFN`, `PinHeader`…
    pub family: &'static str,
    pub kind: Kind,
    /// Pad centre to centre along a row.
    pub pitch: Nm,
    /// Pad centre to centre across: between the rows, or opposite sides (`Quad`).
    pub span: Nm,
    /// A pad's length (out from the body; a through-hole pad's across the rows) and width.
    pub pad: (Nm, Nm),
    /// A through-hole pin's drill.
    pub drill: Nm,
    /// The body's width (across the rows) and length (along them).
    pub body: (Nm, Nm),
    /// An SMD package's exposed pad under its body: width and length; numbered after the others.
    pub exposed: Option<(Nm, Nm)>,
    /// Its rows of pads: 2, a quad's 4 sides, a pin header's 1 or 2 (the only one that changes).
    pub rows: usize,
}

/// A package the wizard starts from: its name, and its numbers for a count of pads.
pub type Preset = (&'static str, fn(usize) -> Package);

/// The packages the wizard starts from (KiCad's libraries' numbers for the common sizes).
pub const PRESETS: [Preset; 8] = [
    ("SOIC", |n| dual("SOIC", 1_270_000, 4_950_000, (1_950_000, 600_000), (3_900_000, steps(n / 2, 1_270_000) - 180_000), None)),
    ("TSSOP", |n| dual("TSSOP", 650_000, 5_725_000, (1_475_000, 400_000), (4_400_000, steps(n / 2, 650_000) + 450_000), None)),
    ("DFN", |n| {
        let length = steps(n / 2, 650_000) + 400_000;
        dual("DFN", 650_000, 3_100_000, (650_000, 350_000), (3_000_000, length), Some((1_550_000, length - 600_000)))
    }),
    ("QFN", |n| {
        let side = steps(n / 4, 500_000) + 1_000_000;
        quad("QFN", side - 75_000, (825_000, 250_000), side, Some((side - 1_300_000, side - 1_300_000)))
    }),
    ("LQFP", |n| {
        // Its bodies come in these sizes.
        let least = steps(n / 4, 500_000) + 1_000_000;
        let side = [5, 7, 10, 12, 14, 20, 24, 28].map(|mm| mm * 1_000_000).into_iter().find(|&s| s >= least).unwrap_or(least);
        quad("LQFP", side + 1_325_000, (1_475_000, 300_000), side, None)
    }),
    (t!("Pin header"), |n| tht("PinHeader", Kind::Header, 0, (1_700_000, 1_700_000), 1_000_000, (2_540_000, steps(n, 2_540_000)), 1)),
    (t!("Pin header, 2 rows"), |n| {
        tht("PinHeader", Kind::Header, 2_540_000, (1_700_000, 1_700_000), 1_000_000, (5_080_000, steps(n / 2, 2_540_000)), 2)
    }),
    ("DIP", |n| tht("DIP", Kind::Dip, 7_620_000, (1_600_000, 1_600_000), 800_000, (6_350_000, steps(n / 2, 2_540_000)), 2)),
];

fn steps(n: usize, step: Nm) -> Nm {
    n as Nm * step
}

fn dual(family: &'static str, pitch: Nm, span: Nm, pad: (Nm, Nm), body: (Nm, Nm), exposed: Option<(Nm, Nm)>) -> Package {
    Package { family, kind: Kind::Dual, pitch, span, pad, drill: 0, body, exposed, rows: 2 }
}

fn quad(family: &'static str, span: Nm, pad: (Nm, Nm), side: Nm, exposed: Option<(Nm, Nm)>) -> Package {
    Package { family, kind: Kind::Quad, pitch: 500_000, span, pad, drill: 0, body: (side, side), exposed, rows: 4 }
}

fn tht(family: &'static str, kind: Kind, span: Nm, pad: (Nm, Nm), drill: Nm, body: (Nm, Nm), rows: usize) -> Package {
    Package { family, kind, pitch: 2_540_000, span, pad, drill, body, exposed: None, rows }
}

/// The least room between two pads (a fab's usual 4 mil), and between a pad and the silkscreen
/// line's middle (KiCad's 0.2 mm, and half the line).
const GAP: Nm = 100_000;
const SILK_GAP: Nm = 260_000;
/// The courtyard's margin, the grid it is on, and the line widths, as KiCad's libraries.
const COURTYARD: Nm = 250_000;
const SILK: Nm = 120_000;

/// A pad: its number, centre, size (x, y), and the way out from the body to it (none for one
/// under the body).
struct Land {
    number: String,
    at: Pt,
    size: (Nm, Nm),
    out: (Nm, Nm),
}

impl Land {
    fn lo(&self) -> Pt {
        Pt { x: self.at.x - self.size.0 / 2, y: self.at.y - self.size.1 / 2 }
    }
    fn hi(&self) -> Pt {
        Pt { x: self.at.x + self.size.0 / 2, y: self.at.y + self.size.1 / 2 }
    }
}

impl Package {
    /// Whether its pads go through the board.
    pub fn through(&self) -> bool {
        matches!(self.kind, Kind::Header | Kind::Dip)
    }

    /// Its exposed pad: an SMD package's only.
    pub(super) fn exposed_pad(&self) -> Option<(Nm, Nm)> {
        self.exposed.filter(|_| !self.through())
    }

    /// Its footprint for `count` pads round it (and the exposed pad, numbered after them): its
    /// name and its `.kicad_mod` text, or why it can't be made.
    pub fn footprint(&self, count: usize) -> Result<(String, String), String> {
        let lands = self.lands(count)?;
        let name = self.name(count);
        Ok((name.clone(), self.text(&name, count, &lands)))
    }

    /// Its pads for `count` round it, checked: sizes there, as many on each row, apart.
    fn lands(&self, count: usize) -> Result<Vec<Land>, String> {
        let sizes = [
            (self.pitch, t!("The pitch")),
            (self.pad.0, t!("The pad's length")),
            (self.pad.1, t!("The pad's width")),
            (self.body.0, t!("The body's width")),
            (self.body.1, t!("The body's length")),
        ];
        let span = (self.kind != Kind::Header || self.rows == 2).then_some((self.span, t!("The span")));
        let drill = self.through().then_some((self.drill, t!("The drill")));
        let exposed = self.exposed_pad().map(|e| [(e.0, t!("The exposed pad's width")), (e.1, t!("The exposed pad's length"))]);
        for (size, what) in sizes.into_iter().chain(span).chain(drill).chain(exposed.into_iter().flatten()) {
            if size <= 0 {
                return Err(trf("{0} must be more than 0", &[&tr(what)]));
            }
        }
        let rows = match self.kind {
            Kind::Quad => 4,
            Kind::Header => self.rows.clamp(1, 2),
            _ => 2,
        };
        if count == 0 || !count.is_multiple_of(rows) {
            // The pins the part has: those round it, and the exposed pad's.
            let pins = count + self.exposed_pad().is_some() as usize;
            let why = match (rows, self.exposed_pad().is_some()) {
                (4, false) => t!("A {0} has as many pads on each side: give it 4, 8, 12, 16… pins (now {1})"),
                (4, true) => t!("A {0} has as many pads on each side, and its exposed pad: give it 5, 9, 13, 17… pins (now {1})"),
                (2, false) => t!("A {0} has two rows of pads: give it an even number of pins (now {1})"),
                (2, true) => t!("A {0} has two rows of pads and its exposed pad: give it an odd number of pins (now {1})"),
                _ => t!("Give the part its pins: type how many, then Fill"),
            };
            return Err(trf(why, &[&self.family, &pins]));
        }
        if self.through() && self.drill + 2 * GAP > self.pad.0.min(self.pad.1) {
            let least = nm_to_mm(self.drill + 2 * GAP);
            return Err(trf("The drill leaves too little copper round it: make the pad at least {0} mm", &[&least]));
        }
        let (n, p, half) = (count / rows, self.pitch, self.span / 2);
        // Along a row from its first pad, centred on 0.
        let along = |k: usize| -((n as Nm - 1) * p) / 2 + k as Nm * p;
        let (across, down) = ((self.pad.0, self.pad.1), (self.pad.1, self.pad.0));
        let land = |k: usize, at: Pt, size: (Nm, Nm), out: (Nm, Nm)| Land { number: (k + 1).to_string(), at, size, out };
        let mut lands: Vec<Land> = match self.kind {
            Kind::Dual | Kind::Quad => (0..count)
                .map(|i| {
                    let (k, side) = (i % n, i / n);
                    // Down the left, along the bottom, up the right, back along the top.
                    let (at, size, out) = match (side, rows) {
                        (0, _) => (Pt { x: -half, y: along(k) }, across, (-1, 0)),
                        (1, 2) => (Pt { x: half, y: -along(k) }, across, (1, 0)),
                        (1, _) => (Pt { x: along(k), y: half }, down, (0, 1)),
                        (2, _) => (Pt { x: half, y: -along(k) }, across, (1, 0)),
                        _ => (Pt { x: -along(k), y: -half }, down, (0, -1)),
                    };
                    land(i, at, size, out)
                })
                .collect(),
            // Pin 1 at the origin, as KiCad's libraries put a through-hole part's.
            Kind::Header => {
                (0..count).map(|i| land(i, Pt { x: (i % rows) as Nm * self.span, y: (i / rows) as Nm * p }, across, (0, 0))).collect()
            }
            Kind::Dip => (0..count)
                .map(|i| {
                    let (k, side) = (i % n, i / n);
                    let at = if side == 0 { Pt { x: 0, y: k as Nm * p } } else { Pt { x: self.span, y: (n - 1 - k) as Nm * p } };
                    land(i, at, across, (0, 0))
                })
                .collect(),
        };
        if let Some(size) = self.exposed_pad() {
            lands.push(land(count, Pt::default(), size, (0, 0)));
        }
        for (i, a) in lands.iter().enumerate() {
            for b in &lands[..i] {
                let gap = |a0: Nm, a1: Nm, b0: Nm, b1: Nm| (b0 - a1).max(a0 - b1) as f64;
                let (dx, dy) = (gap(a.lo().x, a.hi().x, b.lo().x, b.hi().x), gap(a.lo().y, a.hi().y, b.lo().y, b.hi().y));
                let apart = if dx > 0.0 && dy > 0.0 { dx.hypot(dy) } else { dx.max(dy) };
                if apart < GAP as f64 {
                    let what = if apart <= 0.0 {
                        t!("Pads {0} and {1} overlap: make the pads smaller, or the pitch or span bigger")
                    } else {
                        t!("Pads {0} and {1} are closer than 0.1 mm: make the pads smaller, or the pitch or span bigger")
                    };
                    return Err(trf(what, &[&b.number, &a.number]));
                }
            }
        }
        Ok(lands)
    }

    /// Its name as KiCad's libraries name a package: `SOIC-8_3.9x4.9mm_P1.27mm`,
    /// `QFN-16-1EP_3x3mm_P0.5mm_EP1.7x1.7mm`, `PinHeader_2x05_P2.54mm`, `DIP-8_W7.62mm`.
    pub fn name(&self, count: usize) -> String {
        let mm = nm_to_mm;
        match (self.kind, self.exposed) {
            (Kind::Header, _) => {
                format!("{}_{}x{:02}_P{}mm", self.family, self.rows.clamp(1, 2), count / self.rows.clamp(1, 2), mm(self.pitch))
            }
            (Kind::Dip, _) => format!("{}-{count}_W{}mm", self.family, mm(self.span)),
            (_, ep) => {
                let (b, p) = (self.body, mm(self.pitch));
                let (one, size) = ep.map_or((String::new(), String::new()), |(w, l)| ("-1EP".into(), format!("_EP{}x{}mm", mm(w), mm(l))));
                format!("{}-{count}{one}_{}x{}mm_P{p}mm{size}", self.family, mm(b.0), mm(b.1))
            }
        }
    }

    /// Its `.kicad_mod` text, named `name`, for pads `lands` (`count` round it).
    fn text(&self, name: &str, count: usize, lands: &[Land]) -> String {
        let through = self.through();
        // The body, centred on the pads round it (pin 1 is the origin for a through-hole part).
        let (lo, hi) = lands[..count].iter().fold((lands[0].at, lands[0].at), |(lo, hi), l| {
            (Pt { x: lo.x.min(l.at.x), y: lo.y.min(l.at.y) }, Pt { x: hi.x.max(l.at.x), y: hi.y.max(l.at.y) })
        });
        let centre = Pt { x: (lo.x + hi.x) / 2, y: (lo.y + hi.y) / 2 };
        let (bw, bl) = (self.body.0 / 2, self.body.1 / 2);
        let body = (Pt { x: centre.x - bw, y: centre.y - bl }, Pt { x: centre.x + bw, y: centre.y + bl });
        // The courtyard round pads and body, out to a 0.01 mm grid.
        let (mut clo, mut chi) = body;
        for l in lands {
            (clo, chi) = (Pt { x: clo.x.min(l.lo().x), y: clo.y.min(l.lo().y) }, Pt { x: chi.x.max(l.hi().x), y: chi.y.max(l.hi().y) });
        }
        let (down, up) = (|v: Nm| v.div_euclid(10_000) * 10_000, |v: Nm| -(-v).div_euclid(10_000) * 10_000);
        let court =
            (Pt { x: down(clo.x - COURTYARD), y: down(clo.y - COURTYARD) }, Pt { x: up(chi.x + COURTYARD), y: up(chi.y + COURTYARD) });
        let (outline, lines) = silk(body, lands, through);
        // Pin 1's mark: a triangle pointing at it from beyond its outer end, past the silkscreen.
        let one = &lands[0];
        let beyond = if outline.0.x < one.at.x { outline.0.x - SILK_GAP } else { Nm::MAX };
        let tip = Pt { x: (one.lo().x - SILK_GAP).min(beyond), y: one.at.y };
        let mark = [tip, Pt { x: tip.x - 330_000, y: tip.y - 240_000 }, Pt { x: tip.x - 330_000, y: tip.y + 240_000 }];
        let cut = (self.body.0.min(self.body.1) / 4).min(1_000_000);
        let fab = [
            Pt { x: body.0.x + cut, y: body.0.y },
            Pt { x: body.1.x, y: body.0.y },
            body.1,
            Pt { x: body.0.x, y: body.1.y },
            Pt { x: body.0.x, y: body.0.y + cut },
        ];
        let size = (self.body.0.min(self.body.1) / 4).clamp(250_000, 1_000_000) / 10_000 * 10_000;
        let turn = if self.body.1 > self.body.0 { 90 } else { 0 };

        let xy = |p: Pt| format!("{} {}", nm_to_mm(p.x), nm_to_mm(p.y));
        let stroke = |w: Nm| format!("\t\t(stroke\n\t\t\t(width {})\n\t\t\t(type solid)\n\t\t)\n", nm_to_mm(w));
        let font = |s: Nm| {
            format!(
                "\t\t(effects\n\t\t\t(font\n\t\t\t\t(size {0} {0})\n\t\t\t\t(thickness {1})\n\t\t\t)\n\t\t)\n",
                nm_to_mm(s),
                nm_to_mm(s * 15 / 100)
            )
        };
        let poly = |pts: &[Pt], w: Nm, fill: &str, layer: &str| {
            let pts: String = pts.iter().map(|&p| format!("\t\t\t(xy {})\n", xy(p))).collect();
            format!("\t(fp_poly\n\t\t(pts\n{pts}\t\t)\n{}\t\t(fill {fill})\n\t\t(layer \"{layer}\")\n\t)\n", stroke(w))
        };
        let pads = if lands.len() > count { format!("{count} pads and an exposed pad") } else { format!("{count} pads") };
        let mut out = format!(
            "(footprint \"{name}\"\n\t(version 20260206)\n\t(generator \"cuprita\")\n\t(generator_version \"{}\")\n\t(layer \"F.Cu\")\n\t(descr \"{}, {pads}, {} mm pitch (Cuprita's part wizard)\")\n\t(tags \"{}\")\n",
            env!("CARGO_PKG_VERSION"),
            self.family,
            nm_to_mm(self.pitch),
            self.family
        );
        let middle = (court.0.x + court.1.x) / 2;
        for (field, value, y, layer) in
            [("Reference", "REF**", court.0.y - 700_000, "F.SilkS"), ("Value", name, court.1.y + 700_000, "F.Fab")]
        {
            out += &format!(
                "\t(property \"{field}\" \"{value}\"\n\t\t(at {} 0)\n\t\t(layer \"{layer}\")\n{}\t)\n",
                xy(Pt { x: middle, y }),
                font(1_000_000)
            );
        }
        out += &format!("\t(attr {})\n\t(duplicate_pad_numbers_are_jumpers no)\n", if through { "through_hole" } else { "smd" });
        for (a, b) in lines {
            out += &format!("\t(fp_line\n\t\t(start {})\n\t\t(end {})\n{}\t\t(layer \"F.SilkS\")\n\t)\n", xy(a), xy(b), stroke(SILK));
        }
        out += &poly(&mark, SILK, "yes", "F.SilkS");
        out += &format!(
            "\t(fp_rect\n\t\t(start {})\n\t\t(end {})\n{}\t\t(fill no)\n\t\t(layer \"F.CrtYd\")\n\t)\n",
            xy(court.0),
            xy(court.1),
            stroke(50_000)
        );
        out += &poly(&fab, 100_000, "no", "F.Fab");
        out += &format!("\t(fp_text user \"${{REFERENCE}}\"\n\t\t(at {} {turn})\n\t\t(layer \"F.Fab\")\n{}\t)\n", xy(centre), font(size));
        for (i, l) in lands.iter().enumerate() {
            let size = format!("(size {} {})", nm_to_mm(l.size.0), nm_to_mm(l.size.1));
            out += &if through {
                let shape = if i == 0 {
                    "rect"
                } else if l.size.0 == l.size.1 {
                    "circle"
                } else {
                    "oval"
                };
                format!(
                    "\t(pad \"{}\" thru_hole {shape}\n\t\t(at {})\n\t\t{size}\n\t\t(drill {})\n\t\t(layers \"*.Cu\" \"*.Mask\")\n\t\t(remove_unused_layers no)\n\t)\n",
                    l.number,
                    xy(l.at),
                    nm_to_mm(self.drill)
                )
            } else if i < count {
                // Corners rounded a quarter of the pad's width, at most 0.25 mm, as KiCad's libraries.
                let ratio = (250_000.0 / l.size.0.min(l.size.1) as f64).min(0.25);
                format!(
                    "\t(pad \"{}\" smd roundrect\n\t\t(at {})\n\t\t{size}\n\t\t(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")\n\t\t(roundrect_rratio {})\n\t)\n",
                    l.number,
                    xy(l.at),
                    trim(format!("{ratio:.6}"))
                )
            } else {
                // The exposed pad sinks heat into the zones it joins solid; its paste in a grid of
                // squares 80% of their cells (64% of it: IPC-7525's 50–80%).
                let cells = |s: Nm| ((s as f64 / 1e6).round() as Nm).max(1);
                let (nx, ny) = (cells(l.size.0), cells(l.size.1));
                let (cw, ch) = (l.size.0 / nx, l.size.1 / ny);
                let mut pad = format!(
                    "\t(pad \"{}\" smd rect\n\t\t(at {})\n\t\t{size}\n\t\t(property pad_prop_heatsink)\n\t\t(layers \"F.Cu\" \"F.Mask\")\n\t\t(zone_connect 2)\n\t)\n",
                    l.number,
                    xy(l.at)
                );
                for (x, y) in (0..nx).flat_map(|x| (0..ny).map(move |y| (x, y))) {
                    let at = Pt { x: l.lo().x + cw * x + cw / 2, y: l.lo().y + ch * y + ch / 2 };
                    pad += &format!(
                        "\t(pad \"\" smd roundrect\n\t\t(at {})\n\t\t(size {} {})\n\t\t(layers \"F.Paste\")\n\t\t(roundrect_rratio 0.25)\n\t)\n",
                        xy(at),
                        nm_to_mm(cw * 8 / 10),
                        nm_to_mm(ch * 8 / 10)
                    );
                }
                pad
            };
        }
        out + "\t(embedded_fonts no)\n)\n"
    }
}

/// The silkscreen outline: the body's edges 0.11 mm out (the line just outside it), as lines
/// kept off the pads: an SMD part's cut where pads (and their leads, in to the middle) come
/// near, the pieces at the corners kept; a through-hole part's edge moved in past the pads it
/// would cross. Its box, and its lines.
fn silk(body: (Pt, Pt), lands: &[Land], through: bool) -> ((Pt, Pt), Vec<(Pt, Pt)>) {
    let out = 110_000;
    let (mut lo, mut hi) = (Pt { x: body.0.x - out, y: body.0.y - out }, Pt { x: body.1.x + out, y: body.1.y + out });
    // Where a pad keeps the silkscreen off: its box grown by the gap, an SMD pad's in to the middle.
    let keep = |l: &Land| {
        let (mut a, mut b) = (l.lo(), l.hi());
        if !through {
            match l.out {
                (-1, _) => b.x = b.x.max(0),
                (1, _) => a.x = a.x.min(0),
                (_, -1) => b.y = b.y.max(0),
                (_, 1) => a.y = a.y.min(0),
                _ => {}
            }
        }
        (Pt { x: a.x - SILK_GAP, y: a.y - SILK_GAP }, Pt { x: b.x + SILK_GAP, y: b.y + SILK_GAP })
    };
    let keeps: Vec<(Pt, Pt)> = lands.iter().filter(|l| !l.number.is_empty()).map(keep).collect();
    if through {
        // Each edge in past the pads on it, as far as the middle.
        let (mid_x, mid_y) = ((lo.x + hi.x) / 2, (lo.y + hi.y) / 2);
        for _ in 0..lands.len() {
            let on_x = |x: Nm| keeps.iter().filter(move |k| k.0.x < x && x < k.1.x && k.0.y < hi.y && lo.y < k.1.y);
            let on_y = |y: Nm| keeps.iter().filter(move |k| k.0.y < y && y < k.1.y && k.0.x < hi.x && lo.x < k.1.x);
            let moved = (
                on_x(lo.x).map(|k| k.1.x).max(),
                on_x(hi.x).map(|k| k.0.x).min(),
                on_y(lo.y).map(|k| k.1.y).max(),
                on_y(hi.y).map(|k| k.0.y).min(),
            );
            if moved == (None, None, None, None) {
                break;
            }
            lo = Pt { x: moved.0.unwrap_or(lo.x).min(mid_x), y: moved.2.unwrap_or(lo.y).min(mid_y) };
            hi = Pt { x: moved.1.unwrap_or(hi.x).max(mid_x), y: moved.3.unwrap_or(hi.y).max(mid_y) };
        }
    }
    let corners = [lo, Pt { x: hi.x, y: lo.y }, hi, Pt { x: lo.x, y: hi.y }];
    let mut lines = vec![];
    for k in 0..4 {
        let (a, b) = (corners[k], corners[(k + 1) % 4]);
        // The edge as a span of `t` from a (0) to b (1); what the pads keep off it, as spans.
        let along = |p: Pt| if a.x == b.x { (p.y - a.y) as f64 / (b.y - a.y) as f64 } else { (p.x - a.x) as f64 / (b.x - a.x) as f64 };
        let mut cuts: Vec<(f64, f64)> = keeps
            .iter()
            .filter(|(p, q)| if a.x == b.x { p.x < a.x && a.x < q.x } else { p.y < a.y && a.y < q.y })
            .map(|&(p, q)| {
                let (s, t) = (along(p), along(q));
                (s.min(t), s.max(t))
            })
            .filter(|(s, t)| *t > 0.0 && *s < 1.0)
            .collect();
        cuts.sort_by(|x, y| x.0.total_cmp(&y.0));
        // Only the pieces from a corner to the first cut, and from the last to the other corner.
        let first = cuts.first().map_or(1.0, |c| c.0);
        let last = cuts.iter().map(|c| c.1).fold(0.0, f64::max);
        let at = |t: f64| Pt { x: a.x + ((b.x - a.x) as f64 * t).round() as Nm, y: a.y + ((b.y - a.y) as f64 * t).round() as Nm };
        if cuts.is_empty() {
            lines.push((a, b));
            continue;
        }
        if first > 0.0 {
            lines.push((a, at(first)));
        }
        if last < 1.0 {
            lines.push((at(last), b));
        }
    }
    ((lo, hi), lines.into_iter().filter(|(a, b)| a != b).collect())
}

/// `s` without trailing zeros: `0.25`, `0.208333`.
fn trim(s: String) -> String {
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;

    /// Preset `k`'s footprint for `n` pads, as Cuprita's board parser reads it: its name, the
    /// board, and its text.
    fn made(k: usize, n: usize) -> (String, Board, String) {
        let (name, text) = (PRESETS[k].1)(n).footprint(n).unwrap();
        let mut board = Board::default();
        board.add_footprint(&text, "p", Pt::default(), 0.0).unwrap();
        (name, board, text)
    }

    fn mm(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    /// The pads' numbers and centres.
    fn pads(b: &Board) -> Vec<(&str, Pt)> {
        b.pads.iter().map(|p| (p.number.as_str(), p.at)).collect()
    }

    /// The lines on `layer`, their ends (the lower first).
    fn lines(b: &Board, layer: &str) -> Vec<(Pt, Pt)> {
        let on = b.layers.iter().position(|l| l == layer).unwrap() as u8;
        let ends = |a: Pt, b: Pt| if (a.x, a.y) <= (b.x, b.y) { (a, b) } else { (b, a) };
        let segments = b.shapes.iter().filter(|s| s.layer == on);
        segments.filter_map(|s| if let crate::board::Geom::Segment { a, b } = s.geom { Some(ends(a, b)) } else { None }).collect()
    }

    #[test]
    fn makes_a_soic_as_kicads_library_has_it() {
        let (name, b, text) = made(0, 8);
        assert_eq!(name, "SOIC-8_3.9x4.9mm_P1.27mm");
        let kicad = [
            (-2.475, -1.905),
            (-2.475, -0.635),
            (-2.475, 0.635),
            (-2.475, 1.905),
            (2.475, 1.905),
            (2.475, 0.635),
            (2.475, -0.635),
            (2.475, -1.905),
        ];
        let numbers = ["1", "2", "3", "4", "5", "6", "7", "8"];
        assert_eq!(pads(&b), numbers.into_iter().zip(kicad.map(|(x, y)| mm(x, y))).collect::<Vec<_>>());
        assert_eq!(b.pads[0].size, (1_950_000, 600_000));
        // KiCad's silkscreen: the body's ends whole, its sides just past the pads.
        let silk = lines(&b, "F.SilkS");
        for line in [(mm(-2.06, -2.56), mm(2.06, -2.56)), (mm(2.06, -2.56), mm(2.06, -2.465)), (mm(-2.06, 2.465), mm(-2.06, 2.56))] {
            assert!(silk.contains(&line), "{line:?} in {silk:?}");
        }
        assert_eq!(silk.len(), 6);
        assert!(text.contains("\t(fp_rect\n\t\t(start -3.7 -2.7)\n\t\t(end 3.7 2.7)\n"), "{text}");
        assert!(text.contains("\t(property \"Reference\" \"REF**\"\n\t\t(at 0 -3.4 0)\n\t\t(layer \"F.SilkS\")\n"));
        assert!(text.contains("(fp_text user \"${REFERENCE}\"\n\t\t(at 0 0 90)\n\t\t(layer \"F.Fab\")"));
        assert!(text.contains("\t\t(pts\n\t\t\t(xy -0.975 -2.45)\n\t\t\t(xy 1.95 -2.45)\n\t\t\t(xy 1.95 2.45)\n\t\t\t(xy -1.95 2.45)\n\t\t\t(xy -1.95 -1.475)\n"));
        assert!(text.starts_with("(footprint \"SOIC-8_3.9x4.9mm_P1.27mm\"\n\t(version 20260206)\n\t(generator \"cuprita\")\n"));
    }

    #[test]
    fn makes_a_qfn_with_its_exposed_pad_as_kicads_library_has_it() {
        let (name, b, text) = made(3, 16);
        assert_eq!(name, "QFN-16-1EP_3x3mm_P0.5mm_EP1.7x1.7mm");
        let at = |n: &str| b.pads.iter().find(|p| p.number == n).map(|p| (p.at, p.size)).unwrap();
        assert_eq!(at("1"), (mm(-1.4625, -0.75), (825_000, 250_000)));
        assert_eq!(at("5"), (mm(-0.75, 1.4625), (250_000, 825_000)));
        assert_eq!((at("9").0, at("13").0, at("16").0), (mm(1.4625, 0.75), mm(0.75, -1.4625), mm(-0.75, -1.4625)));
        assert_eq!(at("17"), (Pt::default(), (1_700_000, 1_700_000)));
        // Its paste in four squares; the silkscreen only at the corners.
        assert_eq!(b.pads.iter().filter(|p| p.number.is_empty()).count(), 4);
        let silk = lines(&b, "F.SilkS");
        assert!(silk.len() == 8 && silk.contains(&(mm(-1.61, -1.61), mm(-1.135, -1.61))), "{silk:?}");
        assert!(text.contains("(pts\n\t\t\t(xy -2.135 -0.75)\n\t\t\t(xy -2.465 -0.99)\n\t\t\t(xy -2.465 -0.51)\n"), "{text}");
        assert!(text.contains("(start -2.13 -2.13)\n\t\t(end 2.13 2.13)"));
    }

    #[test]
    fn makes_headers_and_dips_from_pin_1() {
        let (name, b, _) = made(6, 10);
        assert_eq!(name, "PinHeader_2x05_P2.54mm");
        assert_eq!(pads(&b)[..3], [("1", Pt::default()), ("2", mm(2.54, 0.0)), ("3", mm(0.0, 2.54))]);
        assert_eq!(b.pads[0].shape, crate::board::PadShape::Rect);
        assert_eq!(b.pads[1].drill, Some((1_000_000, 1_000_000)));
        let (name, b, _) = made(7, 8);
        assert_eq!(name, "DIP-8_W7.62mm");
        assert_eq!(pads(&b)[3..5], [("4", mm(0.0, 7.62)), ("5", mm(7.62, 7.62))]);
        // The silkscreen between the rows, clear of the pads.
        assert!(lines(&b, "F.SilkS").contains(&(mm(1.06, -1.38), mm(6.56, -1.38))));
        let (name, ..) = made(5, 3);
        assert_eq!(name, "PinHeader_1x03_P2.54mm");
    }

    #[test]
    fn every_preset_makes_a_footprint_cuprita_reads() {
        for (k, (preset, _)) in PRESETS.iter().enumerate() {
            for n in [4, 8, 16, 48] {
                let (_, b, _) = made(k, n);
                let exposed = (PRESETS[k].1)(n).exposed.is_some() as usize;
                assert_eq!(b.pads.iter().filter(|p| !p.number.is_empty()).count(), n + exposed, "{preset} {n}");
                for layer in ["F.SilkS", "F.CrtYd", "F.Fab"] {
                    assert!(b.shapes.iter().any(|s| b.layers[s.layer as usize] == layer), "{preset} {n}: {layer}");
                }
            }
        }
    }

    #[test]
    fn says_why_a_package_cant_be_made() {
        let soic = (PRESETS[0].1)(8);
        let fails = |p: Package, n: usize| p.footprint(n).unwrap_err();
        assert_eq!(
            fails(Package { pad: (1_950_000, 1_200_000), ..soic.clone() }, 8),
            "Pads 1 and 2 are closer than 0.1 mm: make the pads smaller, or the pitch or span bigger"
        );
        assert_eq!(
            fails(Package { span: 1_000_000, ..soic.clone() }, 8),
            "Pads 4 and 5 overlap: make the pads smaller, or the pitch or span bigger"
        );
        assert_eq!(fails(soic.clone(), 7), "A SOIC has two rows of pads: give it an even number of pins (now 7)");
        assert_eq!(fails(Package { pitch: 0, ..soic }, 8), "The pitch must be more than 0");
        let dip = (PRESETS[7].1)(8);
        assert_eq!(
            fails(Package { drill: 1_500_000, ..dip }, 8),
            "The drill leaves too little copper round it: make the pad at least 1.7 mm"
        );
        let qfn = (PRESETS[3].1)(16);
        assert_eq!(
            fails(Package { exposed: Some((2_600_000, 2_600_000)), ..qfn }, 16),
            "Pads 1 and 17 overlap: make the pads smaller, or the pitch or span bigger"
        );
    }
}
