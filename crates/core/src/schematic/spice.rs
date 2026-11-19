//! A project's SPICE netlist for ngspice, as KiCad 10's simulator writes it: each part's model
//! from its `Sim.*` fields, else from its reference and value (R, C, L, V, I); the sheets'
//! directives; the workbook's analyses (`crates/core/spice.md`; checked against `kicad-cli sch
//! export netlist --format spice` with skill `spice-check`).

use super::Project;
use crate::json::Val;
use crate::library::kicad_dirs;
use crate::tr::{t, tr, trf};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A project's simulation: the netlist ngspice reads, and how it reads model libraries.
#[derive(Debug, PartialEq)]
pub struct Deck {
    pub text: String,
    /// ngspice's `ngbehavior` for the project's compatibility mode (`.kicad_pro`): `ltpsa`
    /// (LTspice and PSpice, KiCad's default), `psa`, `lta`, `hsa`, `spice3`; empty: the user's own
    /// `.spiceinit`.
    pub behavior: &'static str,
    /// The project's folder, where code models read their files (`d_state`'s state file).
    pub dir: PathBuf,
    /// KiCad's names for the vectors ngspice makes of the nets and parts' currents: `/out` is
    /// `V(/out)`, `net-_d1-a_` `V(Net-(D1-A))`, `v1#branch` `I(V1)`.
    pub signals: HashMap<String, String>,
    /// The signals the workbook plots (`V(/out)`), every tab's.
    pub traces: Vec<String>,
}

/// A part as the deck sees it: its first unit's fields and value, and its pins' numbers.
struct Part<'a> {
    reference: &'a str,
    value: &'a str,
    sim: &'a [(String, String)],
    on_sim: bool,
    unit: u32,
    pins: Vec<&'a str>,
}

/// A model library: where it is, and its `.model`s and `.subckt`s by name (lowercase).
struct Lib {
    path: PathBuf,
    models: HashMap<String, Model>,
}

#[derive(Clone)]
enum Model {
    /// A subcircuit's ports.
    Subckt(Vec<String>),
    /// A device model's type (`d`, `npn`, `vdmos`) and its statement after its name.
    Device(String, String),
}

/// Parameters ngspice takes on a device's line rather than in its model.
const INSTANCE: [&str; 23] = [
    "area", "areab", "areac", "m", "pj", "off", "ic", "temp", "dtemp", "lm", "wm", "lp", "wp", "l", "w", "ad", "as", "pd", "ps", "nrd",
    "nrs", "nf", "thermal",
];

/// KiCad's MOSFET model types and ngspice's levels for them (MOS1, its first, has none).
const MOS: [(&str, &str); 15] = [
    ("MOS2", "2"),
    ("MOS3", "3"),
    ("BSIM1", "4"),
    ("BSIM2", "5"),
    ("MOS6", "6"),
    ("BSIM3", "8"),
    ("MOS9", "9"),
    ("B4SOI", "10"),
    ("BSIM4", "14"),
    ("B3SOIFD", "55"),
    ("B3SOIDD", "56"),
    ("B3SOIPD", "57"),
    ("HISIM2", "68"),
    ("HISIMHV1", "73 version=1.2.4"),
    ("HISIMHV2", "73 version=2.2.0"),
];

const NO_MODEL: &str =
    t!("no simulation model: give it one in KiCad (Symbol Properties, Simulation Model) or tick Exclude from simulation");

impl Project {
    /// The project's SPICE netlist, its model libraries and workbook found from `dir`, its
    /// folder; an error names the parts that can't be simulated and how to make them so.
    pub fn spice(&self, dir: &Path) -> Result<Deck, String> {
        // ngspice reads the deck's files from wherever Cuprita runs.
        let dir = &std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let pro = Val::parse(&self.pro);
        let ngspice = pro.as_ref().and_then(|p| p.at(&["schematic", "ngspice"]));
        // KiCad's compatibility modes, in its order; LTspice and PSpice without one.
        let behavior = match ngspice.and_then(|n| n.num("model_mode")).unwrap_or(4.0) as u8 {
            0 => "",
            1 => "spice3",
            2 => "psa",
            3 => "lta",
            5 => "hsa",
            _ => "ltpsa",
        };
        let workbook = ngspice.and_then(|n| n.str("workbook_filename")).filter(|f| !f.is_empty());
        let (analyses, traces) =
            workbook.and_then(|f| std::fs::read_to_string(dir.join(f)).ok()).map_or_else(Default::default, |w| tabs(&w));
        let (mut nodes, mut signals) = (HashMap::new(), HashMap::new());
        for (net, pins) in self.netlist() {
            let n = node(&net);
            signals.insert(n.to_lowercase(), format!("V({net})"));
            nodes.extend(pins.into_iter().map(|pin| (pin, n.clone())));
        }
        let mut w = Writer { dir, nodes, libs: HashMap::new(), includes: vec![], models: vec![] };
        let (mut lines, mut errors) = (vec![], vec![]);
        for p in self.parts() {
            match w.element(&p) {
                Ok(Some(line)) => {
                    let element = line.split(' ').next().unwrap_or_default().to_lowercase();
                    signals.insert(format!("{element}#branch"), format!("I({})", p.reference));
                    lines.push(line);
                }
                Ok(None) => {}
                Err(e) => errors.push((p.reference, e)),
            }
        }
        if !errors.is_empty() {
            return Err(grouped(errors));
        }
        let mut text = String::from(".title KiCad schematic\n");
        for path in &w.includes {
            text += &format!(".include \"{}\"\n", path.display());
        }
        for line in w.models.iter().chain(&analyses) {
            text += line;
            text.push('\n');
        }
        let mut directives: Vec<&str> = vec![];
        for d in self.pages.iter().flat_map(|p| &p.sheet.directives) {
            // A sheet used twice says them twice.
            if !directives.contains(&d.as_str()) {
                directives.push(d);
            }
        }
        for d in directives {
            // The workbook's analyses replace the sheets', and may hold some of their lines too.
            let kept = |l: &&str| !(is_analysis(l) && !analyses.is_empty() || analyses.iter().any(|a| a == l.trim_end()));
            for line in d.lines().filter(kept) {
                text += &w.absolute(line);
                text.push('\n');
            }
        }
        for line in lines {
            text += &line;
            text.push('\n');
        }
        text += ".end\n";
        harmless(&text, tr("the schematic's SPICE texts"), dir, 0)?;
        Ok(Deck { text, behavior, dir: dir.to_path_buf(), signals, traces })
    }

    /// Its parts (not power symbols) in the order the sheets show them first, each with its first
    /// unit's fields, as KiCad keeps a part's model there.
    fn parts(&self) -> Vec<Part<'_>> {
        let (mut parts, mut at): (Vec<Part>, HashMap<&str, usize>) = (vec![], HashMap::new());
        for s in self.pages.iter().map(|p| &p.sheet) {
            for sym in s.symbols.iter().filter(|s| !s.designator.starts_with('#')) {
                let part =
                    Part { reference: &sym.designator, value: &sym.value, sim: &sym.sim, on_sim: sym.on_sim, unit: sym.unit, pins: vec![] };
                match at.get(sym.designator.as_str()) {
                    Some(&i) if parts[i].unit > sym.unit => parts[i] = Part { pins: std::mem::take(&mut parts[i].pins), ..part },
                    Some(_) => {}
                    None => {
                        at.insert(&sym.designator, parts.len());
                        parts.push(part);
                    }
                }
            }
            for pin in &s.wiring.pins {
                if let Some(&i) = at.get(s.symbols[pin.symbol].designator.as_str()) {
                    parts[i].pins.push(&pin.number);
                }
            }
        }
        for p in &mut parts {
            p.pins.sort_by_key(|n| (n.len(), *n));
            p.pins.dedup();
        }
        parts
    }
}

/// What the deck is built from: the parts' pins' nodes, the model libraries read and included,
/// the models it defines.
struct Writer<'a> {
    dir: &'a Path,
    nodes: HashMap<(String, String), String>,
    libs: HashMap<String, Result<Lib, String>>,
    includes: Vec<PathBuf>,
    models: Vec<String>,
}

impl Writer<'_> {
    /// A part's line (`R1 /in /out 1k`), or none when it's excluded from simulation.
    fn element(&mut self, p: &Part) -> Result<Option<String>, String> {
        let field = |k: &str| p.sim.iter().find(|(n, _)| n == k).map_or("", |(_, v)| v.as_str());
        if !p.on_sim || field("Enable") == "0" {
            return Ok(None);
        }
        if p.reference.ends_with('?') {
            return Err(tr("not annotated: number the parts (KiCad's Annotate Schematic)").into());
        }
        let params = params(field("Params"));
        let param = |k: &str| params.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|&(_, v)| v);
        let (device, kind, lib, name) = (field("Device"), field("Type"), field("Library"), field("Name"));
        let first = p.reference.chars().next().unwrap_or(' ').to_ascii_uppercase();
        // The element's letter, its model's pins and how many it needs, and what follows them.
        let (letter, pins, needed, tail): (char, Vec<String>, usize, String) = match device {
            // shortcut: KiCad 6's `Spice_` fields aren't read; read them when a KiCad 6 project needs it.
            "" if p.sim.iter().any(|(n, _)| n.starts_with("Spice_")) => {
                return Err(tr("its simulation model is KiCad 6's: open and save the sheet in KiCad 7 or later, which converts it").into());
            }
            // shortcut: IBIS, XSPICE and transmission line models aren't written; write them when a
            // project needs one.
            "IBIS" | "KIBIS" | "XSPICE" | "TLINE" => {
                return Err(trf(
                    "its {0} model isn't simulated in Cuprita yet: give it a SPICE or library model, or tick Exclude from simulation",
                    &[&device],
                ));
            }
            "SPICE" => {
                if let Some(lib) = param("lib").filter(|l| !l.is_empty()) {
                    self.lib(lib)?;
                }
                let letter = param("type").and_then(|t| t.chars().next()).unwrap_or(first);
                let pins = (1..=p.pins.len()).map(|n| n.to_string()).collect();
                (letter.to_ascii_uppercase(), pins, p.pins.len(), param("model").unwrap_or_default().into())
            }
            _ if !lib.is_empty() => self.library(p, device, lib, name, &params)?,
            "" => match first {
                'R' | 'C' | 'L' if p.pins.len() == 2 => (first, two(), 2, number(p.value)),
                'V' | 'I' if p.pins.len() == 2 => (first, two(), 2, format!("DC {}", dc(None, p.value))),
                _ => return Err(tr(NO_MODEL).into()),
            },
            "R" | "C" | "L" if matches!(kind, "" | "=") => {
                let key = device.to_ascii_lowercase();
                let value = param(&key).map_or_else(|| number(p.value), number);
                let rest = params.iter().filter(|(n, _)| !n.eq_ignore_ascii_case(&key)).map(|(n, v)| format!(" {n}={}", number(v)));
                (first_of(device), two(), 2, value + &rest.collect::<String>())
            }
            "V" | "I" if kind == "=" => {
                let expr = param(device).ok_or(tr("a behavioral source needs its expression: set it in its Sim.Params"))?;
                ('B', two(), 2, format!("{device}={expr}"))
            }
            "V" | "I" => (first_of(device), two(), 2, source(kind, &param, p.value)?),
            "SW" if kind == "V" => {
                // KiCad's names for ngspice's threshold and hysteresis.
                let renamed = params.iter().map(|&(n, v)| {
                    (
                        match n {
                            "thr" => "vt",
                            "his" => "vh",
                            n => n,
                        },
                        v,
                    )
                });
                self.models.push(format!(
                    ".model __{} SW{}",
                    p.reference,
                    renamed.map(|(n, v)| format!(" {n}={}", number(v))).collect::<String>()
                ));
                let pins = ["no+", "no-", "ctrl+", "ctrl-"].map(String::from).to_vec();
                ('S', pins, 4, format!("__{}", p.reference))
            }
            _ => {
                let (model, pins) = builtin(device, kind).ok_or_else(|| {
                    trf(
                        "its model, KiCad's {0} {1}, isn't simulated in Cuprita yet: give it a SPICE or library model, or tick Exclude from simulation",
                        &[&device, &kind],
                    )
                })?;
                let (inst, model_params) = split(&params);
                self.models.push(format!(".model __{} {model}{model_params}", p.reference));
                (pins.0, pins.1.iter().map(|s| s.to_string()).collect(), pins.2, format!("__{}{inst}", p.reference))
            }
        };
        let nodes = self.pins(p, &pins, needed, field("Pins"))?;
        let name =
            if p.reference.to_ascii_uppercase().starts_with(letter) { p.reference.to_string() } else { format!("{letter}{}", p.reference) };
        Ok(Some(format!("{name} {} {tail}", nodes.join(" "))))
    }

    /// A part's model from library `file`: a subcircuit (`XU1 … OPA1641 gain=2`) or a device
    /// model, a copy of it (`D1.1N4148`) when the part sets some of its parameters.
    fn library(
        &mut self,
        p: &Part,
        device: &str,
        file: &str,
        name: &str,
        params: &[(&str, &str)],
    ) -> Result<(char, Vec<String>, usize, String), String> {
        let lib = self.lib(file)?;
        let (path, model) = (lib.path.display().to_string(), lib.models.get(&name.to_lowercase()).cloned());
        let model = model.ok_or_else(|| trf("its model {0} isn't in {1}: correct its Sim.Name or Sim.Library", &[&name, &path]))?;
        match model {
            Model::Subckt(ports) => {
                let given = params.iter().map(|(n, v)| format!(" {n}={}", number(v))).collect::<String>();
                Ok(('X', ports.clone(), ports.len(), format!("{name}{given}")))
            }
            Model::Device(kind, text) => {
                let (letter, pins, needed) = pins(&kind)
                    .ok_or_else(|| trf("its model {0} in {1} is a {2}, which Cuprita doesn't simulate yet", &[&name, &path, &kind]))?;
                if device == "SUBCKT" {
                    return Err(trf("its model {0} in {1} is a .model, not a .subckt: set its Sim.Device to the device", &[&name, &path]));
                }
                let (inst, model_params) = split(params);
                let model = if model_params.is_empty() {
                    name.to_string()
                } else {
                    let copy = format!("{}.{name}", p.reference);
                    self.models.push(format!(".model {copy} {text}{model_params}"));
                    copy
                };
                Ok((letter, pins.iter().map(|s| s.to_string()).collect(), needed, model + &inst))
            }
        }
    }

    /// The part's nodes in its model's pin order: `Sim.Pins` (`1=K 2=A`: symbol pin = model pin,
    /// by name or position) places them, its other pins go to the model pins left, in order; a
    /// model pin it needs that nothing reaches gets a node of its own (`NC-Q1-0`), as in KiCad.
    fn pins(&self, p: &Part, model: &[String], needed: usize, map: &str) -> Result<Vec<String>, String> {
        let mut at: Vec<Option<&str>> = vec![None; model.len()];
        for (sym, pin) in map.split_whitespace().filter_map(|m| m.split_once('=')) {
            let i = model.iter().position(|m| m.eq_ignore_ascii_case(pin)).or_else(|| pin.parse::<usize>().ok()?.checked_sub(1));
            if let Some(slot) = i.and_then(|i| at.get_mut(i)) {
                *slot = p.pins.iter().copied().find(|&n| n == sym);
            }
        }
        let placed: Vec<&str> = at.iter().flatten().copied().collect();
        let mut free = p.pins.iter().copied().filter(|n| !placed.contains(n));
        let (mut out, mut unreached) = (vec![], 0);
        for (i, slot) in at.into_iter().enumerate() {
            match slot.or_else(|| free.next()) {
                Some(pin) => {
                    let node = self.nodes.get(&(p.reference.to_string(), pin.to_string()));
                    out.push(node.cloned().ok_or_else(|| trf("its pin {0} isn't on a net", &[&pin]))?);
                }
                None if i < needed => {
                    out.push(format!("NC-{}-{unreached}", p.reference));
                    unreached += 1;
                }
                None => break,
            }
        }
        Ok(out)
    }

    /// Model library `file` (`Sim.Library`), read once; the deck includes it.
    fn lib(&mut self, file: &str) -> Result<&Lib, String> {
        if !self.libs.contains_key(file) {
            let lib = find(file, self.dir).and_then(|path| {
                let text = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(Lib { models: models(&String::from_utf8_lossy(&text)), path })
            });
            if let Ok(lib) = &lib {
                self.includes.push(lib.path.clone());
            }
            self.libs.insert(file.to_string(), lib);
        }
        self.libs[file].as_ref().map_err(String::clone)
    }

    /// A directive's line with its relative `.include` or `.lib` file in the project's folder, as
    /// KiCad's simulator reads it (ngspice would look in Cuprita's).
    fn absolute(&self, line: &str) -> String {
        let mut words = line.splitn(2, char::is_whitespace);
        let (kw, rest) = (words.next().unwrap_or_default(), words.next().unwrap_or_default().trim());
        if !matches!(kw.to_ascii_lowercase().as_str(), ".include" | ".inc" | ".lib") || rest.is_empty() {
            return line.to_string();
        }
        let (file, section) = match rest.strip_prefix('"').and_then(|r| r.split_once('"')) {
            Some((file, section)) => (file, section),
            None => rest.split_at(rest.find(char::is_whitespace).unwrap_or(rest.len())),
        };
        format!("{kw} \"{}\"{section}", self.dir.join(file).display())
    }
}

fn two() -> Vec<String> {
    vec!["+".into(), "-".into()]
}

fn first_of(device: &str) -> char {
    device.chars().next().unwrap_or('X')
}

/// A device's SPICE letter, its pins in its order (KiCad's names for them) and how many of them
/// it needs.
type Device = (char, &'static [&'static str], usize);

/// A device model type's (`.model`'s, lowercase) device.
fn pins(kind: &str) -> Option<Device> {
    Some(match kind {
        "d" => ('D', &["A", "K"], 2),
        "npn" | "pnp" => ('Q', &["C", "B", "E", "S"], 3),
        "njf" | "pjf" => ('J', &["D", "G", "S"], 3),
        "nmos" | "pmos" => ('M', &["D", "G", "S", "B"], 4),
        "vdmos" => ('M', &["D", "G", "S"], 3),
        "nmf" | "pmf" => ('Z', &["D", "G", "S"], 3),
        _ => return None,
    })
}

/// A KiCad built-in model (`Sim.Device`, `Sim.Type`) as ngspice's `.model` names it, and its
/// device's letter and pins.
fn builtin(device: &str, kind: &str) -> Option<(String, Device)> {
    let base = match device {
        "NJFET" => "NJF",
        "PJFET" => "PJF",
        "NMES" => "NMF",
        "PMES" => "PMF",
        "D" | "NPN" | "PNP" | "NMOS" | "PMOS" => device,
        _ => return None,
    };
    let level = match (base, kind) {
        ("D", _) | (_, "") | ("NPN" | "PNP", "GUMMELPOON") | ("NJF" | "PJF", "SHICHMANHODGES") => "",
        ("NMF" | "PMF", "STATZ") | ("NMOS" | "PMOS", "MOS1") => "",
        ("NPN" | "PNP", "VBIC") => "4",
        ("NJF" | "PJF" | "NMF" | "PMF", "PARKERSKELLERN" | "YTTERDAL") => "2",
        ("NMF" | "PMF", "HFET1") => "5",
        ("NMF" | "PMF", "HFET2") => "6",
        ("NMOS" | "PMOS", "VDMOS") => return Some((format!("VDMOS {}CHAN", &device[..1]), pins("vdmos")?)),
        ("NMOS" | "PMOS", t) => MOS.iter().find(|(m, _)| *m == t)?.1,
        _ => return None,
    };
    let model = if level.is_empty() { base.to_string() } else { format!("{base} level={level}") };
    Some((model, pins(&base.to_ascii_lowercase())?))
}

/// Parameters as a device's line and its model take them: ` area=2 m=3`, ` rs=1 cjo=10p`.
fn split(params: &[(&str, &str)]) -> (String, String) {
    let (mut inst, mut model) = (String::new(), String::new());
    for (n, v) in params {
        let to = if INSTANCE.contains(&n.to_ascii_lowercase().as_str()) { &mut inst } else { &mut model };
        *to += &format!(" {n}={}", number(v));
    }
    (inst, model)
}

/// A V or I source's text after its nodes (`DC 0 SIN( 0 1 1k ) AC 1`), from its KiCad model
/// type, its parameters and, for DC, its value: each type's parameters in ngspice's order,
/// written up to the last one given (noise and random sources whole, as KiCad writes them), the
/// missing ones before it 0.
fn source<'p>(kind: &str, param: &impl Fn(&str) -> Option<&'p str>, value: &str) -> Result<String, String> {
    let call = |f: &str, slots: &[&str]| {
        let last = if f.starts_with("TR") { slots.len() } else { slots.iter().rposition(|s| param(s).is_some()).map_or(0, |i| i + 1) };
        // A slot is a parameter's name or a number of its own (`TRRANDOM`'s type).
        let arg = |s: &&str| if s.starts_with(|c: char| c.is_ascii_digit()) { s.to_string() } else { param(s).map_or("0".into(), number) };
        format!("{f}( {} )", slots[..last].iter().map(arg).collect::<Vec<_>>().join(" "))
    };
    let out = match kind {
        "" | "DC" => format!("DC {}", dc(param("dc"), value)),
        "SIN" => format!("DC {} {}", dc(param("dc"), value), call("SIN", &["dc", "ampl", "f", "td", "theta", "phase"])),
        "PULSE" => call("PULSE", &["y1", "y2", "td", "tr", "tf", "tw", "per", "np"]),
        "EXP" => call("EXP", &["y1", "y2", "td1", "tau1", "td2", "tau2"]),
        "AM" => call("AM", &["vo", "vmo", "vma", "fm", "fc", "td", "phasem", "phasec"]),
        "SFFM" => call("SFFM", &["vo", "va", "fm", "mdi", "fc", "phasem", "phasec"]),
        "PWL" => format!("PWL( {} )", param("pwl").unwrap_or_default().split_whitespace().map(number).collect::<Vec<_>>().join(" ")),
        "WHITENOISE" => call("TRNOISE", &["rms", "dt", "0", "0", "0", "0", "0"]),
        "PINKNOISE" => call("TRNOISE", &["0", "dt", "slope", "rms", "0", "0", "0"]),
        "BURSTNOISE" => call("TRNOISE", &["0", "0", "0", "0", "ampl", "tcapt", "temit"]),
        "RANDUNIFORM" => call("TRRANDOM", &["1", "ts", "td", "range", "offset"]),
        "RANDNORMAL" => call("TRRANDOM", &["2", "ts", "td", "stddev", "mean"]),
        "RANDEXP" => call("TRRANDOM", &["3", "ts", "td", "mean", "offset"]),
        "RANDPOISSON" => call("TRRANDOM", &["4", "ts", "td", "lambda", "offset"]),
        _ => return Err(trf("its source type {0} isn't simulated in Cuprita yet: pick another in its Simulation Model", &[&kind])),
    };
    let ac = param("ac").map(|ac| format!(" AC {}{}", number(ac), param("ph").map(|ph| format!(" {}", number(ph))).unwrap_or_default()));
    Ok(out + &ac.unwrap_or_default())
}

/// A source's DC value: its `dc` parameter, else its value as written (`5`, `3.3V`, `SINE(0 1 1k)`).
fn dc(param: Option<&str>, value: &str) -> String {
    match param {
        Some(dc) => number(dc),
        // A text variable (`${SIM.PARAMS}`) is no value.
        None if value.starts_with("${") => "0".into(),
        None => number(value),
    }
}

/// `Sim.Params` (`dc=0 ampl=1 pwl="0 0 1m 1" r='V(a)*2'`): each name and value, double quotes
/// taken off (single ones mark an expression, which ngspice reads with them).
fn params(s: &str) -> Vec<(&str, &str)> {
    let (mut out, mut rest) = (vec![], s);
    while let Some((name, after)) = rest.split_once('=') {
        let after = after.trim_start();
        let (value, next) = match after.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let close = after[1..].find(q).map_or(after.len(), |i| i + 1);
                let end = (close + 1).min(after.len());
                (if q == '"' { &after[1..close] } else { &after[..end] }, &after[end..])
            }
            _ => after.split_once(char::is_whitespace).unwrap_or((after, "")),
        };
        out.push((name.split_whitespace().last().unwrap_or_default(), value));
        rest = next;
    }
    out
}

/// A library's `.model`s and `.subckt`s by name (lowercase): its statements, a line and the
/// `+` lines that go on with it, `*` lines left out, in any case.
fn models(text: &str) -> HashMap<String, Model> {
    let mut statements: Vec<String> = vec![];
    for line in text.lines().map(str::trim) {
        match (line.strip_prefix('+'), statements.last_mut()) {
            (Some(more), Some(s)) => *s += &format!(" {more}"),
            _ if !line.starts_with('*') => statements.push(line.to_string()),
            _ => {}
        }
    }
    let mut out = HashMap::new();
    for s in statements {
        let mut words = s.split_whitespace();
        let (Some(kw), Some(name)) = (words.next(), words.next()) else { continue };
        let model = match kw.to_ascii_lowercase().as_str() {
            ".subckt" => {
                Model::Subckt(words.take_while(|w| !w.eq_ignore_ascii_case("params:") && !w.contains('=')).map(String::from).collect())
            }
            ".model" => {
                let after = kw.len() + s[kw.len()..].find(name).unwrap_or_default() + name.len();
                let text = s[after..].trim().to_string();
                let kind = text.split(|c: char| c == '(' || c.is_whitespace()).next().unwrap_or_default().to_ascii_lowercase();
                Model::Device(kind, text)
            }
            _ => continue,
        };
        out.entry(name.to_lowercase()).or_insert(model);
    }
    out
}

/// Where model library `file` is: a relative file and `${KIPRJMOD}` in the project's folder,
/// `${KICAD*_SYMBOL_DIR}` in KiCad's symbol libraries' folder (which ships
/// `Simulation_SPICE.sp`), another `${VAR}` from the environment.
fn find(file: &str, dir: &Path) -> Result<PathBuf, String> {
    let Some((var, rest)) = file.strip_prefix("${").and_then(|f| f.split_once('}')) else {
        let path = dir.join(file);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(trf("its model library {0} isn't there: correct its Sim.Library", &[&path.display()]))
        };
    };
    let rest = rest.trim_start_matches(['/', '\\']);
    let roots = match std::env::var_os(var) {
        Some(root) => vec![PathBuf::from(root)],
        None if var == "KIPRJMOD" => vec![dir.to_path_buf()],
        None if var.starts_with("KICAD") && var.ends_with("_SYMBOL_DIR") => kicad_dirs("symbols", "_SYMBOL_DIR"),
        None => return Err(trf("its model library {0}: {1} is not set; set it to the library's folder", &[&file, &var])),
    };
    roots
        .iter()
        .map(|r| r.join(rest))
        .find(|p| p.is_file())
        .ok_or_else(|| trf("its model library {0} isn't installed: install KiCad's symbol libraries", &[&file]))
}

/// A KiCad workbook's (`.wbk`) analyses, each tab's first command as KiCad runs it, and the
/// signals its tabs plot.
fn tabs(wbk: &str) -> (Vec<String>, Vec<String>) {
    let Some(tabs) = Val::parse(wbk).and_then(|w| w.arr("tabs").map(<[Val]>::to_vec)) else { return Default::default() };
    let (mut analyses, mut traces): (Vec<String>, Vec<String>) = Default::default();
    let add = |to: &mut Vec<String>, s: &str| {
        if !to.iter().any(|o| o == s) {
            to.push(s.to_string());
        }
    };
    for t in &tabs {
        if let Some(Val::Str(c)) = t.arr("commands").and_then(|c| c.first()) {
            c.lines().map(str::trim_end).filter(|l| !l.is_empty()).for_each(|l| add(&mut analyses, l));
        }
        for trace in t.arr("traces").unwrap_or_default() {
            trace.str("signal").into_iter().for_each(|s| add(&mut traces, s));
        }
    }
    (analyses, traces)
}

/// Whether a directive's line starts an analysis, which the workbook's replace.
fn is_analysis(line: &str) -> bool {
    let kw = line.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    matches!(kw.as_str(), ".op" | ".tran" | ".ac" | ".dc" | ".noise" | ".tf" | ".pz" | ".sens" | ".disto" | ".sp")
}

/// Whether a sheet's text is a SPICE directive, as KiCad takes them: one of its lines starts
/// with a `.` or couples two inductors (`K1 L1 L2 0.98`).
pub(super) fn directive(text: &str) -> bool {
    text.lines().any(|l| {
        let w: Vec<&str> = l.split_whitespace().collect();
        l.starts_with('.') || w.len() >= 4 && w[0].starts_with(['K', 'k']) && w[1].starts_with(['L', 'l']) && w[2].starts_with(['L', 'l'])
    })
}

/// A net's name as a SPICE node, as KiCad writes it: `(`, `)` and spaces as `_`, `~{…}` (an
/// overbar) as `_…`, `_{…}` and `^{…}` (sub- and superscript) as what they hold, `{slash}` as `/`.
pub(super) fn node(net: &str) -> String {
    let (mut out, mut open) = (String::with_capacity(net.len()), 0);
    let net = net.replace("{slash}", "/");
    let mut chars = net.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '~' | '_' | '^' if chars.peek() == Some(&'{') => {
                chars.next();
                open += 1;
                if c == '~' {
                    out.push('_');
                }
            }
            '}' if open > 0 => open -= 1,
            '(' | ')' | ' ' => out.push('_'),
            c => out.push(c),
        }
    }
    out
}

/// A value in KiCad's notation (`4k7`, `1M`, `10µF`, `2,2 n`, `4R7`) as SPICE reads the same
/// number (`4.7k`, `1Meg`, `10u`, `2.2n`, `4.7`): SPICE takes `M` for milli, stops at a number's
/// first letter and knows no `µ`, `,` or peta and exa. Anything else is kept as written.
pub(super) fn number(v: &str) -> String {
    let digits = |s: &str| s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let s = v.trim();
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", s.strip_prefix('+').unwrap_or(s)),
    };
    let int = &s[..digits(s)];
    let (mut frac, mut rest) = (None, &s[int.len()..]);
    if let Some(r) = rest.strip_prefix(['.', ',']) {
        frac = Some(&r[..digits(r)]);
        rest = &r[digits(r)..];
    }
    if int.is_empty() && frac.is_none_or(str::is_empty) {
        return v.to_string();
    }
    let dot = |f: &str| if f.is_empty() { String::new() } else { format!(".{f}") };
    // An exponent SPICE reads as it is.
    if let Some(e) = rest.strip_prefix(['e', 'E']) {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        return if !e.is_empty() && digits(e) == e.len() { format!("{sign}{int}{}{rest}", dot(frac.unwrap_or(""))) } else { v.to_string() };
    }
    let rest = rest.trim_start();
    let (scale, after) = match rest.chars().next() {
        Some(c) if "afpnuµmkKMGTPERr".contains(c) => (Some(c), &rest[c.len_utf8()..]),
        _ => (None, rest),
    };
    // RKM: the prefix is the decimal point (`4k7`, `4R7`).
    let (frac, after) = match (frac, scale) {
        (None, Some(_)) => (&after[..digits(after)], &after[digits(after)..]),
        (f, _) => (f.unwrap_or(""), after),
    };
    if !matches!(after.trim_start(), "" | "F" | "H" | "Ω") {
        return v.to_string();
    }
    let scale = match scale {
        Some('µ') => "u",
        Some('K') => "k",
        Some('M') => "Meg",
        Some('P') => "e15",
        Some('E') => "e18",
        Some('R' | 'r') | None => "",
        // a f p n u m k G T: SPICE's own.
        Some(_) => &rest[..1],
    };
    format!("{sign}{int}{}{scale}", dot(frac))
}

/// The parts that can't be simulated, those with the same problem together: `U2, Y1: no
/// simulation model: …`.
fn grouped(errors: Vec<(&str, String)>) -> String {
    let mut by: Vec<(String, Vec<&str>)> = vec![];
    for (part, e) in errors {
        match by.iter_mut().find(|(m, _)| *m == e) {
            Some((_, parts)) => parts.push(part),
            None => by.push((e, vec![part])),
        }
    }
    let line = |(e, parts): (String, Vec<&str>)| {
        let named = parts[..parts.len().min(8)].join(", ");
        let named = if parts.len() > 8 { trf("{0} and {1} more", &[&named, &(parts.len() - 8)]) } else { named };
        // Through the catalogs: French sets a space before the colon.
        trf("{0}: {1}", &[&named, &e])
    };
    by.into_iter().map(line).collect::<Vec<_>>().join("\n")
}

/// What a `.control` block may run from a schematic or a model file: ngspice's commands that
/// set, run, measure and show. ngspice runs such a block as it reads the deck, and its `shell`,
/// `source`, `write` and the like reach the user's programs and files.
const COMMANDS: &[&str] = &[
    "ac",
    "alter",
    "altermod",
    "alterparam",
    "break",
    "compose",
    "continue",
    "dc",
    "define",
    "destroy",
    "display",
    "disto",
    "dowhile",
    "echo",
    "else",
    "elseif",
    "end",
    "fft",
    "foreach",
    "if",
    "let",
    "linearize",
    "listing",
    "meas",
    "measure",
    "noise",
    "op",
    "option",
    "options",
    "plot",
    "print",
    "pz",
    "repeat",
    "reset",
    "resume",
    "run",
    "rusage",
    "save",
    "sens",
    "set",
    "setplot",
    "setscale",
    "settype",
    "show",
    "showmod",
    "status",
    "step",
    "stop",
    "tf",
    "tran",
    "undefine",
    "unlet",
    "unset",
    "version",
    "while",
];

/// Refuses deck `text` (`what`, for the error) when a `.control` block of it, or of a file it
/// includes (relative ones from `dir`), runs anything but `COMMANDS`, redirects to a file or
/// substitutes a program's output: a schematic from someone else must not run programs.
fn harmless(text: &str, what: &str, dir: &Path, depth: usize) -> Result<(), String> {
    let mut control = false;
    for line in text.lines().map(str::trim) {
        let lower = line.to_ascii_lowercase();
        let mut words = lower.split_whitespace();
        match words.next() {
            Some(".control") => control = true,
            Some(".endc") => control = false,
            Some(".include" | ".inc" | ".lib") if !control && depth < 8 => {
                let rest = line.split_once(char::is_whitespace).map_or("", |(_, r)| r.trim());
                let file =
                    rest.strip_prefix('"').and_then(|r| r.split_once('"')).map_or(rest.split_whitespace().next().unwrap_or(""), |(f, _)| f);
                let path = dir.join(file);
                if let Ok(bytes) = std::fs::read(&path) {
                    harmless(&String::from_utf8_lossy(&bytes), &path.display().to_string(), path.parent().unwrap_or(dir), depth + 1)?;
                }
            }
            Some(_) if control && !line.starts_with(['*', '+']) => {
                let run = lower.split(';').filter_map(|c| c.split_whitespace().next());
                if line.contains(['`', '>']) || run.clone().any(|c| !COMMANDS.contains(&c)) {
                    return Err(trf(
                        "{0}: a .control block runs \"{1}\", which can reach files and programs: Cuprita runs no such line; take it out to simulate",
                        &[&what, &line],
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::net::tests::{part, project, wire};

    #[test]
    fn a_control_block_runs_only_what_reads_sets_and_measures() {
        let dir = std::env::temp_dir().join(format!("cuprita-control-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // KiCad's own demos' blocks pass; a comment, a continuation and an analysis too.
        let demo = ".option abstol=100n\n.control\nset controlswait\nif $?sharedmode\n  version -s ; rusage time\nelse\n  run\n  plot v(/vdd)\n+ v(/vss)\nend\n* power\nmeas tran p rms R1:power from=250m to=300m\n.endc\n.tran 1u 1m\n";
        assert_eq!(harmless(demo, "t", &dir, 0), Ok(()));
        // A program run, a file written, a program's output taken, a script read: each refused, with its line.
        for line in ["shell touch owned", "print v(1) > out.txt", "echo `id`", "source x.sp", "write out.raw", "set a=1 ; SHELL id"] {
            let e = harmless(&format!(".control\n{line}\n.endc\n"), "the sheet", &dir, 0).unwrap_err();
            assert!(e.starts_with("the sheet: a .control block runs") && e.contains(line), "{e}");
        }
        // Outside a block the words are a part's or a model's; in a file the deck includes, they count.
        assert_eq!(harmless("Xshell 1 2 shell\n.model shell d\n", "t", &dir, 0), Ok(()));
        std::fs::write(dir.join("m.lib"), ".model d1 d\n.include \"n.lib\"\n").unwrap();
        std::fs::write(dir.join("n.lib"), "* models\n.CONTROL\n  Shell id\n.endc\n").unwrap();
        let e = harmless(".lib m.lib\n.include \"missing.lib\"\n", "t", &dir, 0).unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(e.contains("n.lib: a .control block runs \"Shell id\""), "{e}");
    }

    #[test]
    fn writes_kicads_values_as_spice_reads_them() {
        // KiCad 10's conversions (`kicad-cli sch export netlist --format spice`), or the same numbers.
        for (kicad, spice) in [
            ("1M", "1Meg"),
            ("4k7", "4.7k"),
            ("10R", "10"),
            ("4R7", "4.7"),
            ("1M5", "1.5Meg"),
            ("1.5uF", "1.5u"),
            ("10µF", "10u"),
            ("5mΩ", "5m"),
            ("1uH", "1u"),
            ("2.2 k", "2.2k"),
            ("1,5k", "1.5k"),
            ("1K", "1k"),
            ("1P", "1e15"),
            ("+5", "5"),
            ("1e3", "1e3"),
            ("1F", "1"),
            ("10a", "10a"),
            // Not KiCad's notation: as written, as KiCad writes it.
            ("100uF;16V", "100uF;16V"),
            ("1meg", "1meg"),
            ("47k 1%", "47k 1%"),
            ("5V", "5V"),
            ("R47", "R47"),
            ("", ""),
        ] {
            assert_eq!(number(kicad), spice, "{kicad}");
        }
    }

    #[test]
    fn names_nodes_and_takes_directives_as_kicad_does() {
        // KiCad 10's nodes for glasgow-revD1's nets.
        for (net, spice) in [
            ("/ADC/ADC_~{RST}", "/ADC/ADC__RST"),
            ("/NAFE_{~{CS}}", "/NAFE_CS"),
            ("/FPGA/RAM + LVDS/VIO_{AUX}", "/FPGA/RAM_+_LVDS/VIOAUX"),
            ("Net-(U2-PD3{slash}FD11)", "Net-_U2-PD3/FD11_"),
        ] {
            assert_eq!(node(net), spice);
        }
        assert!(["note\n.ac dec 10 1 1k", ".op", "K1 L1 L2 0.5", "k2 l1 l3 0.1"].into_iter().all(directive));
        assert!(![" .op", "Keep this short", "K1 a b 1", "*.dc V1 0 1 0.1"].into_iter().any(directive));
        let p = params(r#"dc=0 pwl="0 0 1m 1" r='V(a) * 2' type="V""#);
        assert_eq!(p, [("dc", "0"), ("pwl", "0 0 1m 1"), ("r", "'V(a) * 2'"), ("type", "V")]);
    }

    #[test]
    fn writes_sources_as_kicad_does() {
        // KiCad 10's lines for its v_i_sources and subsheets demos' sources.
        for (kind, given, value, kicad) in [
            ("SIN", "dc=0 ampl=1 f=100M td=1n theta=10G", "VSIN", "DC 0 SIN( 0 1 100Meg 1n 10G )"),
            ("PULSE", "y1=-1 y2=1 td=2n tr=30n tf=2n tw=50n per=100n", "", "PULSE( -1 1 2n 30n 2n 50n 100n )"),
            ("WHITENOISE", "rms=100m dt=500p ac=0 ph=0", "", "TRNOISE( 100m 500p 0 0 0 0 0 ) AC 0 0"),
            ("RANDUNIFORM", "ts=100ns td=10ns range=1", "", "TRRANDOM( 1 100ns 10ns 1 0 )"),
            ("DC", "", "2.345", "DC 2.345"),
            ("", "", "SINE(0 1.5 1k 0 0 0 0)", "DC SINE(0 1.5 1k 0 0 0 0)"),
        ] {
            let p = params(given);
            let param = |k: &str| p.iter().find(|(n, _)| *n == k).map(|&(_, v)| v);
            assert_eq!(source(kind, &param, value).unwrap(), kicad);
        }
    }

    #[test]
    fn writes_a_projects_parts_as_kicad_does() {
        let dir = std::env::temp_dir().join(format!("cuprita-spice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m.lib"), "* models\n.MODEL DX d(is=1e-14 n=2)\n.subckt AMP out\n+ in PARAMS: g=1\n.ends\n").unwrap();
        let wbk = r#"{"tabs": [{"commands": [".tran 1u 10m", ".kicad adjustpaths", ".save all"], "traces": [{"signal": "V(/out)"}]}]}"#;
        std::fs::write(dir.join("t.wbk"), wbk).unwrap();
        // A two-pin part (pins 5 mm apart from its first at x, y) with simulation fields.
        let two = |r: &str, value: &str, x: f64, y: f64, fields: &[(&str, &str)]| {
            let f: String = fields.iter().map(|(k, v)| format!(r#" (property "Sim.{k}" "{v}" (at 0 0 0))"#)).collect();
            part("R", r, value, x, y).replace(r#" (property "Value""#, &format!(r#"{f} (property "Value""#))
        };
        let items = [
            two("V1", "x", 10.0, 10.0, &[("Device", "V"), ("Type", "SIN"), ("Params", "dc=0 ampl=1 f=1k ac=1"), ("Pins", "1=+ 2=-")]),
            wire(5.0, 10.0, 10.0, Some(("label", "in"))),
            part("PWR", "#PWR1", "GND", 15.0, 10.0),
            two("R1", "4k7", 10.0, 20.0, &[]),
            wire(5.0, 10.0, 20.0, Some(("label", "in"))),
            wire(15.0, 20.0, 20.0, Some(("label", "out"))),
            two("D1", "1N4148", 10.0, 30.0, &[("Library", "m.lib"), ("Name", "dx"), ("Pins", "1=K 2=A")]),
            part("PWR", "#PWR2", "GND", 10.0, 30.0),
            wire(15.0, 20.0, 30.0, Some(("label", "out"))),
            two("U1", "AMP", 10.0, 40.0, &[("Device", "SUBCKT"), ("Library", "m.lib"), ("Name", "AMP"), ("Pins", "1=in 2=out"), ("Params", "g=2")]),
            wire(5.0, 10.0, 40.0, Some(("label", "in"))),
            wire(15.0, 20.0, 40.0, Some(("label", "out"))),
            // A built-in model: its line's parameters and its model's apart, a pin it needs and lacks.
            part("DUP", "Q1", "", 10.0, 50.0).replace(
                r#" (property "Value""#,
                r#" (property "Sim.Device" "NMOS" (at 0 0 0)) (property "Sim.Type" "MOS2" (at 0 0 0)) (property "Sim.Params" "w=10u vto=1" (at 0 0 0)) (property "Value""#,
            ),
            two("R2", "1k", 10.0, 70.0, &[]).replace("(unit 1)", "(unit 1) (exclude_from_sim yes)"),
            r#"(text ".tran 1 2\n.options reltol=1e-4" (at 0 0 0))"#.into(),
        ];
        let pro = r#"{"schematic": {"ngspice": {"model_mode": 2, "workbook_filename": "t.wbk"}}}"#;
        let deck = project(&[("root.kicad_sch", items.concat())], pro).spice(&dir).unwrap();
        let lib = dir.join("m.lib").display().to_string();
        let expected = [
            ".title KiCad schematic",
            &format!(".include \"{lib}\""),
            ".model __Q1 NMOS level=2 vto=1",
            ".tran 1u 10m",
            ".options reltol=1e-4",
            "V1 /in GND DC 0 SIN( 0 1 1k ) AC 1",
            "R1 /in /out 4.7k",
            "D1 /out GND dx",
            "XU1 /out /in AMP g=2",
            "MQ1 unconnected-_Q1-SH-Pad1_ unconnected-_Q1-SH-Pad2_ unconnected-_Q1-X-Pad3_ NC-Q1-0 __Q1 w=10u",
            ".end",
        ];
        assert_eq!(deck.text.lines().collect::<Vec<_>>(), expected);
        assert_eq!((deck.behavior, deck.dir.as_path()), ("psa", dir.as_path()));
        assert_eq!((deck.signals["/out"].as_str(), deck.signals["xu1#branch"].as_str()), ("V(/out)", "I(U1)"));
        assert_eq!(deck.traces, ["V(/out)"]);
        // Parts without a model say so together, with what to do.
        let kicad6 = part("R", "Q5", "x", 10.0, 40.0)
            .replace(r#" (property "Value""#, r#" (property "Spice_Primitive" "Q" (at 0 0 0)) (property "Value""#);
        let items = [
            two("U9", "x", 10.0, 10.0, &[]),
            two("U8", "x", 10.0, 20.0, &[]),
            two("D1", "x", 10.0, 30.0, &[("Library", "none.lib")]),
            kicad6,
        ];
        let e = project(&[("root.kicad_sch", items.concat())], "").spice(&dir).unwrap_err();
        assert!(e.starts_with("U9, U8: no simulation model: give it one in KiCad"), "{e}");
        assert!(e.contains("D1: its model library") && e.contains("none.lib isn't there: correct its Sim.Library"), "{e}");
        assert!(e.contains("Q5: its simulation model is KiCad 6's: open and save the sheet in KiCad 7 or later"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
