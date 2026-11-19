//! `.kicad_pcb` parsing (KiCad 4 through 10) into a `Board`, and design rules from `.kicad_pro`.

pub(crate) mod footprint;
mod zone;

use crate::board::{
    Board, CuLayer, DEFAULT_BOARD, DEFAULT_CU, Geom, LayerId, LayerMask, MM, NetClass, NetId, Nm, Pt, Shape, Text, Track, Via, arc_points,
    place, upright,
};
use crate::json::Val;
use crate::sexpr::{Cursor, Result, Tok, unescape};
use crate::tr::tr;
use std::collections::HashMap;

const DEFAULT_ER: f64 = 4.5;
/// From this file version on, KiCad draws a zone's fills with an outline only if the zone says
/// `(filled_areas_thickness yes)`; before it, unless it says `no` (found with KiCad 10).
pub(crate) const THIN_FILLS: u32 = 20250210;

impl Board {
    pub fn parse(src: &str) -> Result<Board> {
        let mut p = Parser::new();
        let c = &mut Cursor::new(src);
        c.open("kicad_pcb")?;
        while let Some(head) = c.child()? {
            match head {
                "version" => p.b.version = c.atom()?.parse().map_err(|_| c.err(tr("bad version")))?,
                "general" => {
                    while let Some(k) = c.child()? {
                        if k == "thickness" {
                            set_rule(&mut p.b.thickness, c.nm()?);
                        }
                        c.close()?;
                    }
                }
                "layers" => p.layers(c)?,
                "setup" => {
                    while let Some(k) = c.child()? {
                        let r = &mut p.b.rules;
                        match k {
                            "stackup" => p.stackup(c)?,
                            "trace_clearance" => set_rule(&mut r.clearance, c.nm()?),
                            "trace_min" => set_rule(&mut r.track_width, c.nm()?),
                            "via_min_size" => set_rule(&mut r.via_diameter, c.nm()?),
                            "via_min_drill" => set_rule(&mut r.hole_diameter, c.nm()?),
                            "via_min_annulus" => set_rule(&mut r.annular_width, c.nm()?),
                            "hole_to_hole_min" => set_rule(&mut r.hole_to_hole, c.nm()?),
                            "copper_edge_clearance" => set_rule(&mut r.edge_clearance, c.nm()?),
                            "zone_clearance" => set_rule(&mut r.zone_clearance, c.nm()?),
                            "pad_to_mask_clearance" => r.mask_margin = c.nm()?,
                            "solder_mask_min_width" => r.mask_min_width = c.nm()?,
                            "allow_soldermask_bridges_in_footprints" => r.mask_bridges_in_parts = c.atom()? == "yes",
                            "pad_to_paste_clearance" => r.paste_margin = c.nm()?,
                            "pad_to_paste_clearance_ratio" => r.paste_ratio = c.f64()?,
                            "tenting" => r.tented = tenting(c)?,
                            "grid_origin" => p.b.grid_origin = c.point()?,
                            // KiCad <= 8 kept via tenting in the plot settings.
                            "pcbplotparams" => {
                                while let Some(k) = c.child()? {
                                    if k == "viasonmask" {
                                        let open = matches!(c.atom()?, "true" | "yes");
                                        r.tented = (!open, !open);
                                    }
                                    c.close()?;
                                }
                            }
                            _ => {}
                        }
                        c.close()?;
                    }
                }
                "net_class" => {
                    let is_default = c.atom()? == "Default";
                    while let Some(k) = c.child()? {
                        let r = &mut p.b.rules;
                        match (is_default, k) {
                            (true, "clearance") => set_rule(&mut r.clearance, c.nm()?),
                            (true, "trace_width") => set_rule(&mut r.class_track_width, c.nm()?),
                            (true, "via_dia") => set_rule(&mut r.class_via_diameter, c.nm()?),
                            (true, "via_drill") => set_rule(&mut r.class_via_drill, c.nm()?),
                            _ => {}
                        }
                        c.close()?;
                    }
                }
                "net" => {
                    p.net(c)?;
                }
                "footprint" | "module" => p.footprint(c)?,
                "segment" | "arc" => p.track(c)?,
                "via" => p.via(c)?,
                "zone" => p.zone(c)?,
                "gr_line" | "gr_arc" | "gr_circle" | "gr_rect" | "gr_poly" => p.shape(c, head, false)?,
                "gr_text" => {
                    p.own_text(c)?;
                }
                // Text variables: board properties win over title block fields.
                "property" => {
                    let name = c.atom()?.to_string();
                    p.vars.insert(name, unescape(c.opt_atom()?.unwrap_or_default()));
                }
                "title_block" => {
                    while let Some(field) = c.child()? {
                        let name = match field {
                            "title" => "TITLE".into(),
                            "date" => "ISSUE_DATE".into(),
                            "rev" => "REVISION".into(),
                            "company" => "COMPANY".into(),
                            "comment" => format!("COMMENT{}", c.atom()?),
                            _ => String::new(),
                        };
                        if let (false, Some(v)) = (name.is_empty(), c.opt_atom()?) {
                            p.vars.entry(name).or_insert(unescape(v));
                        }
                        c.close()?;
                    }
                }
                _ => {}
            }
            c.close()?;
        }
        c.close()?;
        if p.b.stackup.is_empty() && !p.cu_depth.is_empty() {
            let mut order: Vec<_> = p.cu_depth.iter().map(|(&id, &d)| (d, id)).collect();
            order.sort_unstable();
            let gaps = order.len().saturating_sub(1) as Nm;
            let diel = if gaps > 0 { (p.b.thickness - order.len() as Nm * DEFAULT_CU).max(100_000) / gaps } else { 0 };
            p.b.stackup = order
                .iter()
                .enumerate()
                .map(|(i, &(_, layer))| CuLayer {
                    layer,
                    thickness: DEFAULT_CU,
                    dielectric: if i + 1 < order.len() { diel } else { 0 },
                    epsilon_r: DEFAULT_ER,
                })
                .collect();
        }
        for t in p.b.texts.iter_mut().filter(|t| t.text.contains("${")) {
            t.text = expand(&t.text, |name| p.vars.get(name).map(String::as_str));
        }
        p.b.file_nets = p.b.nets.clone();
        Ok(p.b)
    }

    /// Applies the design rules and the default net class of a sibling `.kicad_pro` JSON file.
    pub fn apply_pro(&mut self, pro: &str) {
        let Some(pro) = Val::parse(pro) else { return };
        let (settings, classes) = (pro.at(&["board", "design_settings"]), pro.get("net_settings").and_then(|n| n.arr("classes")));
        let (rules, zones) = (settings.and_then(|s| s.get("rules")), settings.and_then(|s| s.at(&["defaults", "zones"])));
        let class = classes.and_then(|c| c.iter().find(|c| c.str("name") == Some("Default")));
        let nm = |from: Option<&Val>, k: &str| from.and_then(|v| v.num(k)).map(|mm| (mm * 1e6).round() as Nm).filter(|&v| v > 0);
        let r = &mut self.rules;
        let min_clearance = nm(rules, "min_clearance");
        if let Some(v) = nm(class, "clearance").or(min_clearance) {
            r.clearance = v.max(min_clearance.unwrap_or(0));
        }
        // A minimum the project sets to 0 is none (as KiCad takes it), not this one's default.
        for (k, dst) in [
            ("min_track_width", &mut r.track_width),
            ("min_via_diameter", &mut r.via_diameter),
            ("min_through_hole_diameter", &mut r.hole_diameter),
            ("min_via_annular_width", &mut r.annular_width),
            ("min_hole_to_hole", &mut r.hole_to_hole),
            ("min_hole_clearance", &mut r.hole_clearance),
            ("min_copper_edge_clearance", &mut r.edge_clearance),
            ("solder_mask_to_copper_clearance", &mut r.mask_to_copper),
            ("min_silk_clearance", &mut r.silk_clearance),
        ] {
            if let Some(mm) = rules.and_then(|v| v.num(k)).filter(|&mm| mm >= 0.0) {
                *dst = (mm * 1e6).round() as Nm;
            }
        }
        for (from, k, dst) in [
            (class, "track_width", &mut r.class_track_width),
            (class, "via_diameter", &mut r.class_via_diameter),
            (class, "via_drill", &mut r.class_via_drill),
            (class, "diff_pair_width", &mut r.class_pair_width),
            (class, "diff_pair_gap", &mut r.class_pair_gap),
            (zones, "min_clearance", &mut r.zone_clearance),
        ] {
            if let Some(v) = nm(from, k) {
                *dst = v;
            }
        }
        // What the project draws new lines and texts with, on the silkscreen and elsewhere.
        let defaults = settings.and_then(|s| s.get("defaults"));
        for (kind, ink) in [("silk", &mut r.silk_ink), ("other", &mut r.other_ink)] {
            let of = |k: &str, or: Nm| nm(defaults, &format!("{kind}_{k}")).unwrap_or(or);
            *ink = (of("line_width", ink.0), (of("text_size_h", ink.1.0), of("text_size_v", ink.1.1)), of("text_thickness", ink.2));
        }
        // The other classes, each value it leaves out Default's; then the nets each takes, the
        // first that fits a net winning as in KiCad: those KiCad 6 lists in the class, those the
        // schematic's labels assign, then the project's patterns.
        let (r, mut listed) = (self.rules, vec![]);
        self.classes = (classes.unwrap_or_default().iter())
            .filter_map(|c| {
                let name = c.str("name").filter(|&n| n != "Default")?.to_string();
                let of = |k: &str, default: Nm| nm(Some(c), k).unwrap_or(default);
                listed.extend(
                    c.arr("nets")
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|n| if let Val::Str(n) = n { Some((n.as_str(), name.clone())) } else { None }),
                );
                Some(NetClass {
                    name,
                    clearance: of("clearance", r.clearance).max(min_clearance.unwrap_or(0)),
                    track_width: of("track_width", r.class_track_width),
                    via_diameter: of("via_diameter", r.class_via_diameter),
                    via_drill: of("via_drill", r.class_via_drill),
                    pair_width: of("diff_pair_width", r.class_pair_width),
                    pair_gap: of("diff_pair_gap", r.class_pair_gap),
                })
            })
            .collect();
        let net_settings = pro.get("net_settings");
        // shortcut: a net KiCad 9 gives several classes takes the first here, not their values
        // merged by priority; merge them when a project sets classes that way.
        let assigned = match net_settings.and_then(|n| n.get("netclass_assignments")) {
            Some(Val::Obj(nets)) => nets.as_slice(),
            _ => &[],
        };
        let assigned = assigned.iter().filter_map(|(net, class)| match class {
            Val::Str(class) => Some((net.as_str(), class.clone())),
            Val::Arr(of) => of.iter().find_map(|c| if let Val::Str(c) = c { Some((net.as_str(), c.clone())) } else { None }),
            _ => None,
        });
        let patterns = net_settings.and_then(|n| n.arr("netclass_patterns")).unwrap_or_default();
        let patterns = patterns.iter().filter_map(|p| Some((p.str("pattern")?, p.str("netclass")?.to_string())));
        let class = |name: &str| self.classes.iter().position(|c| c.name == name);
        self.class_nets =
            listed.into_iter().chain(assigned).chain(patterns).filter_map(|(net, name)| Some((net.to_string(), class(&name)?))).collect();
        // Each check as the project wants it told: an error, a warning, or not at all.
        self.severities = match settings.and_then(|s| s.get("rule_severities")) {
            Some(Val::Obj(of)) => (of.iter())
                .filter_map(|(kind, how)| match how {
                    Val::Str(how) if how == "ignore" => Some((kind.clone(), None)),
                    Val::Str(how) => Some((kind.clone(), Some(how == "warning"))),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        };
        // KiCad's first entry, 0, stands for the net class's width.
        let widths = settings.and_then(|s| s.arr("track_widths")).unwrap_or_default();
        let width = |w: &Val| match w {
            Val::Num(mm) if *mm > 0.0 => Some((mm * 1e6).round() as Nm),
            _ => None,
        };
        self.track_widths = widths.iter().filter_map(width).collect();
        // The project's text variables, in the texts the board's own names left as written.
        let vars = project_vars(&pro);
        for t in self.texts.iter_mut().filter(|t| !vars.is_empty() && t.text.contains("${")) {
            t.text = expand(&t.text, |name| vars.get(name));
        }
    }
}

/// A `.kicad_pro`'s text variables (KiCad's Project Setup: Text Variables), each name's text.
pub(crate) fn project_vars(pro: &Val) -> HashMap<String, String> {
    let Some(Val::Obj(vars)) = pro.get("text_variables") else { return HashMap::new() };
    vars.iter().filter_map(|(name, v)| if let Val::Str(v) = v { Some((name.clone(), v.clone())) } else { None }).collect()
}

/// Where KiCad's name puts a copper layer in the stack: `F.Cu` 0, `In1.Cu` 1…, `B.Cu` 31.
fn copper_depth(name: &str) -> Option<u32> {
    match name {
        "F.Cu" => Some(0),
        "B.Cu" => Some(31),
        _ => name.strip_prefix("In").and_then(|s| s.strip_suffix(".Cu")).and_then(|s| s.parse().ok()),
    }
}

/// Replaces each `${NAME}` that `var` knows; KiCad shows unknown ones as written.
pub(crate) fn expand<V: AsRef<str>>(s: &str, var: impl Fn(&str) -> Option<V>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some((before, after)) = rest.split_once("${") {
        let Some((name, tail)) = after.split_once('}') else { break };
        out += before;
        match var(name) {
            Some(v) => out += v.as_ref(),
            None => out += &rest[before.len()..rest.len() - tail.len()],
        }
        rest = tail;
    }
    out + rest
}

fn set_rule(dst: &mut Nm, v: Nm) {
    if v > 0 {
        *dst = v;
    }
}

/// Reads a file's items into `b`; saving reads tracks, vias and zones with it too.
#[derive(Default)]
pub(crate) struct Parser<'a> {
    pub(crate) b: Board,
    layer_ids: HashMap<String, LayerId>,
    cu_depth: HashMap<LayerId, u32>,
    masks: HashMap<&'a str, LayerMask>,
    /// KiCad 9 and before: net numbers, and the net each stands for.
    pub(crate) net_codes: HashMap<u32, NetId>,
    net_names: HashMap<&'a str, NetId>,
    vars: HashMap<String, String>,
    /// The primitives of the custom pad being read.
    prims: Vec<Shape>,
    /// Whether the layer table was read: from then on, only it makes a layer copper.
    tabled: bool,
    /// How many of the board's own texts were stored.
    own_texts: usize,
    /// The hidden texts of the footprint being read, which it keeps to show again.
    pub(crate) hidden: Vec<Text>,
}

impl<'a> Parser<'a> {
    pub(crate) fn new() -> Self {
        Parser { b: Board { nets: vec![String::new()], thickness: DEFAULT_BOARD, ..Board::default() }, ..Parser::default() }
    }

    /// A parser adding to `b`, which knows its layers: `*.Cu` stands for its copper.
    pub(crate) fn onto(b: Board) -> Self {
        let layer_ids = b.layers.iter().enumerate().map(|(i, n)| (n.clone(), i as LayerId)).collect();
        Parser { b, layer_ids, tabled: true, ..Parser::new() }
    }

    /// The `(layers …)` list: every layer, and which are copper.
    pub(crate) fn layers(&mut self, c: &mut Cursor<'a>) -> Result<()> {
        while c.child()?.is_some() {
            let name = c.atom()?;
            let kind = c.opt_atom()?;
            let id = self.layer(c, name)?;
            if matches!(kind, Some("signal" | "power" | "mixed" | "jumper")) {
                let d = self.cu_depth.get(&id).copied().unwrap_or(self.cu_depth.len() as u32);
                self.cu_depth.insert(id, d);
                self.b.copper |= 1 << id;
            }
            c.close()?;
        }
        self.tabled = true;
        Ok(())
    }

    fn stackup(&mut self, c: &mut Cursor<'a>) -> Result<()> {
        while let Some(k) = c.child()? {
            if k == "layer" {
                let name = c.atom()?;
                let (mut kind, mut thick, mut er_sum, mut er_n, mut color) = ("", 0, 0.0, 0u32, None);
                stackup_layer(c, &mut kind, &mut thick, &mut er_sum, &mut er_n, &mut color)?;
                let color = color.map(unescape);
                match kind {
                    "Top Solder Mask" => self.b.mask_colors[0] = color,
                    "Bottom Solder Mask" => self.b.mask_colors[1] = color,
                    "Top Silk Screen" => self.b.silk_colors[0] = color,
                    "Bottom Silk Screen" => self.b.silk_colors[1] = color,
                    _ => {}
                }
                if kind == "copper" {
                    let layer = self.layer(c, name)?;
                    self.b.stackup.push(CuLayer {
                        layer,
                        thickness: if thick > 0 { thick } else { DEFAULT_CU },
                        dielectric: 0,
                        epsilon_r: DEFAULT_ER,
                    });
                } else if matches!(kind, "core" | "prepreg")
                    && thick > 0
                    && let Some(cu) = self.b.stackup.last_mut()
                {
                    let er = if er_n > 0 { er_sum / er_n as f64 } else { DEFAULT_ER };
                    let total = cu.dielectric + thick;
                    cu.epsilon_r = (cu.epsilon_r * cu.dielectric as f64 + er * thick as f64) / total as f64;
                    cu.dielectric = total;
                }
            }
            c.close()?;
        }
        Ok(())
    }
    fn layer(&mut self, c: &Cursor, name: &'a str) -> Result<LayerId> {
        if let Some(&id) = self.layer_ids.get(name) {
            return Ok(id);
        }
        if self.b.layers.len() >= LayerMask::BITS as usize {
            return Err(c.err(tr("too many layers")));
        }
        let id = self.b.layers.len() as LayerId;
        self.b.layers.push(name.to_string());
        self.layer_ids.insert(name.to_string(), id);
        if let (Some(d), false) = (copper_depth(name), self.tabled) {
            self.cu_depth.insert(id, d);
            self.b.copper |= 1 << id;
        }
        Ok(id)
    }

    fn layer_at(&mut self, c: &mut Cursor<'a>) -> Result<LayerId> {
        let name = c.atom()?;
        self.layer(c, name)
    }

    /// Layer names in a pad/via list, including wildcards like `*.Cu` and `F&B.Cu`.
    fn layer_mask(&mut self, c: &Cursor, name: &'a str) -> Result<LayerMask> {
        if let Some(&m) = self.masks.get(name) {
            return Ok(m);
        }
        let m = if name == "*.Cu" {
            self.b.copper
        } else if let Some(suffix) = name.strip_prefix('*') {
            let on = self.b.layers.iter().enumerate().filter(|(_, l)| l.ends_with(suffix));
            on.fold(0, |m, (i, _)| m | 1 << i)
        } else if let Some(suffix) = name.strip_prefix("F&B") {
            let (f, b) = (format!("F{suffix}"), format!("B{suffix}"));
            let ids = self.layer_ids.get(f.as_str()).into_iter().chain(self.layer_ids.get(b.as_str()));
            ids.fold(0, |m, &i| m | 1 << i)
        } else {
            1 << self.layer(c, name)?
        };
        self.masks.insert(name, m);
        Ok(m)
    }

    fn layer_list(&mut self, c: &mut Cursor<'a>) -> Result<LayerMask> {
        let mut m = 0;
        while let Some(name) = c.opt_atom()? {
            m |= self.layer_mask(c, name)?;
        }
        Ok(m)
    }

    fn net_by_name(&mut self, name: &'a str) -> NetId {
        if name.is_empty() {
            return 0;
        }
        *self.net_names.entry(name).or_insert_with(|| {
            self.b.nets.push(name.to_string());
            (self.b.nets.len() - 1) as NetId
        })
    }

    /// `(net 5)`, `(net 5 "GND")` (declarations, KiCad <= 9) or `(net "GND")` (KiCad 10).
    pub(crate) fn net(&mut self, c: &mut Cursor<'a>) -> Result<NetId> {
        match c.tok()? {
            Tok::Str(name) => Ok(self.net_by_name(name)),
            Tok::Atom(code) => {
                let code: u32 = code.parse().map_err(|_| c.err(tr("bad net code")))?;
                let name = c.opt_atom()?;
                if let Some(&id) = self.net_codes.get(&code) {
                    return Ok(id);
                }
                let id = name.map_or(0, |n| self.net_by_name(n));
                self.net_codes.insert(code, id);
                Ok(id)
            }
            _ => Err(c.err(tr("bad net"))),
        }
    }

    /// The board's own `gr_text "T" …`: stored when it is shown, with its place among those
    /// shown, where saving finds it; returns whether it was.
    pub(crate) fn own_text(&mut self, c: &mut Cursor<'a>) -> Result<bool> {
        let n = self.b.texts.len();
        self.text(c, "gr_text")?;
        let Some(t) = self.b.texts.get_mut(n) else { return Ok(false) };
        t.src = Some(self.own_texts);
        self.own_texts += 1;
        Ok(true)
    }

    /// `gr_text "T" …`, `fp_text KIND "T" …` or `property "NAME" "T" …`: stores the text when it
    /// is shown (a footprint's hidden one in `hidden`) and returns the kind or name with the raw value.
    fn text(&mut self, c: &mut Cursor<'a>, item: &str) -> Result<(&'a str, &'a str)> {
        let kind = if item == "gr_text" { "" } else { c.atom()? };
        let mut raw = c.atom()?;
        // KiCad 6 and 7 wrote a locked board text `(gr_text locked "T" …)`.
        if item == "gr_text"
            && raw == "locked"
            && let Some(value) = c.opt_atom()?
        {
            raw = value;
        }
        let mut t = Text { size: (MM, MM), ..Text::default() };
        let (mut layer, mut hidden, mut unlocked) = (None, false, false);
        loop {
            // KiCad <= 7 writes `hide` as a bare flag.
            while let Some(flag) = c.opt_atom()? {
                hidden |= flag == "hide";
            }
            let Some(head) = c.child()? else { break };
            match head {
                "at" => {
                    t.at = c.point()?;
                    while let Some(a) = c.opt_atom()? {
                        match a.parse() {
                            Ok(deg) => t.angle = deg,
                            Err(_) => unlocked |= a == "unlocked",
                        }
                    }
                }
                "layer" => {
                    layer = Some(self.layer_at(c)?);
                    t.rough |= c.opt_atom()? == Some("knockout");
                }
                "hide" => hidden = c.atom()? == "yes",
                "unlocked" => unlocked = c.atom()? == "yes",
                "effects" => _ = effects(c, &mut t, &mut hidden)?,
                _ => {}
            }
            c.close()?;
        }
        // KiCad 6/7 footprint properties without a layer are plain fields, never drawn.
        if let (Some(layer), true) = (layer, !hidden || item != "gr_text") {
            t.upright = item != "gr_text" && !unlocked;
            if t.upright {
                t.angle = upright(t.angle);
            }
            if t.thickness == 0 {
                t.thickness = t.size.0 / 8; // KiCad's default pen, measured from its plots
            }
            // KiCad <= 5 wrote %R and %V for the reference and value.
            let shown = match raw {
                "%R" => "${REFERENCE}",
                "%V" => "${VALUE}",
                raw => raw,
            };
            let texts = if hidden { &mut self.hidden } else { &mut self.b.texts };
            texts.push(Text { text: unescape(shown), layer, ..t });
        }
        Ok((kind, raw))
    }

    pub(crate) fn track(&mut self, c: &mut Cursor<'a>) -> Result<()> {
        let (mut a, mut b, mut mid, mut width, mut layer, mut net) = (None, None, None, 0, None, 0);
        let mut locked = locked(c)?;
        while let Some(head) = c.child()? {
            match head {
                "start" => a = Some(c.point()?),
                "end" => b = Some(c.point()?),
                "mid" => mid = Some(c.point()?),
                "width" => width = c.nm()?,
                "layer" => layer = Some(self.layer_at(c)?),
                "net" => net = self.net(c)?,
                "locked" => locked = c.opt_atom()? != Some("no"),
                _ => {}
            }
            c.close()?;
        }
        let (Some(a), Some(b), Some(layer)) = (a, b, layer) else { return Err(c.err(tr("incomplete track"))) };
        self.b.tracks.push(Track { a, b, mid, width, layer, net, locked });
        Ok(())
    }

    pub(crate) fn via(&mut self, c: &mut Cursor<'a>) -> Result<()> {
        let mut v = Via { at: Pt::default(), size: 0, drill: 0, layers: 0, net: 0, locked: locked(c)? };
        let mut ends = vec![];
        while let Some(head) = c.child()? {
            match head {
                "at" => v.at = c.point()?,
                "size" => v.size = c.nm()?,
                "drill" => v.drill = c.nm()?,
                "layers" => {
                    while let Some(name) = c.opt_atom()? {
                        ends.push(self.layer(c, name)?);
                    }
                }
                "net" => v.net = self.net(c)?,
                "locked" => v.locked = c.opt_atom()? != Some("no"),
                _ => {}
            }
            c.close()?;
        }
        // Span every copper layer between the two named ones, in stack order.
        let span: Vec<u32> = ends.iter().filter_map(|l| self.cu_depth.get(l).copied()).collect();
        let (lo, hi) = (span.iter().min().copied().unwrap_or(0), span.iter().max().copied().unwrap_or(31));
        for (&i, &d) in &self.cu_depth {
            if (lo..=hi).contains(&d) {
                v.layers |= 1 << i;
            }
        }
        self.b.vias.push(v);
        Ok(())
    }

    /// A `gr_*` or `fp_*` graphic, or (`prim`) a custom pad's primitive, on the pad's layers.
    pub(crate) fn shape(&mut self, c: &mut Cursor<'a>, kind: &str, prim: bool) -> Result<()> {
        let (mut start, mut mid, mut end, mut center, mut angle) = (None, None, None, None, None);
        let (mut pts, mut width, mut filled, mut layer) = (vec![], 0, None, None);
        while let Some(head) = c.child()? {
            match head {
                "start" => start = Some(c.point()?),
                "mid" => mid = Some(c.point()?),
                "end" => end = Some(c.point()?),
                "center" => center = Some(c.point()?),
                "angle" => angle = Some(c.f64()?),
                "pts" => pts_into(c, &mut pts)?,
                "width" => width = c.nm()?,
                "stroke" => {
                    while let Some(head) = c.child()? {
                        if head == "width" {
                            width = c.nm()?;
                        }
                        c.close()?;
                    }
                }
                "fill" => filled = Some(matches!(c.atom()?, "solid" | "yes")),
                "layer" => layer = Some(self.layer_at(c)?),
                _ => {}
            }
            c.close()?;
        }
        let bad = || c.err(tr("incomplete graphic"));
        let kind = kind.split_once('_').map_or(kind, |(_, k)| k);
        let geom = match (kind, start, end) {
            ("line", Some(a), Some(b)) => Geom::Segment { a, b },
            ("arc", Some(start), Some(end)) => match (mid, angle) {
                (Some(mid), _) => Geom::Arc { start, mid, end },
                // KiCad <= 5: start is the centre, end is the start point, angle is the sweep.
                (None, Some(sweep)) => {
                    let turn = |deg| place(Pt { x: end.x - start.x, y: end.y - start.y }, start, deg);
                    Geom::Arc { start: end, mid: turn(-sweep / 2.0), end: turn(-sweep) }
                }
                _ => return Err(bad()),
            },
            ("circle", _, Some(e)) => {
                let ctr = center.or(start).ok_or_else(bad)?;
                let r = ((e.x - ctr.x) as f64).hypot((e.y - ctr.y) as f64).round() as Nm;
                Geom::Circle { center: ctr, radius: r }
            }
            ("rect", Some(a), Some(b)) => Geom::Poly(vec![a, Pt { x: b.x, y: a.y }, b, Pt { x: a.x, y: b.y }]),
            ("poly", _, _) if pts.len() >= 2 => Geom::Poly(pts),
            _ => return Err(bad()),
        };
        // KiCad <= 5 wrote no fill: polygons are filled, and a pad's zero-width circles.
        let filled = filled.unwrap_or(kind == "poly" || prim && width == 0);
        if prim {
            self.prims.push(Shape { geom, width, filled, layer: 0 });
        } else {
            self.b.shapes.push(Shape { geom, width, filled, layer: layer.ok_or_else(bad)? });
        }
        Ok(())
    }
}

fn stackup_layer<'a>(
    c: &mut Cursor<'a>,
    kind: &mut &'a str,
    thick: &mut Nm,
    er_sum: &mut f64,
    er_n: &mut u32,
    color: &mut Option<&'a str>,
) -> Result<()> {
    while let Some(h) = c.child()? {
        match h {
            "type" => *kind = c.atom()?,
            "color" => *color = Some(c.atom()?),
            "thickness" => *thick += c.nm()?,
            "epsilon_r" => {
                *er_sum += c.f64()?;
                *er_n += 1;
            }
            "sublayer" => stackup_layer(c, kind, thick, er_sum, er_n, color)?,
            _ => {}
        }
        c.close()?;
    }
    Ok(())
}

/// `(effects (font (size H W) (thickness T) (bold yes)) (justify left top mirror) hide)`;
/// returns whether the text is bold.
/// shortcut: bold, italic, TrueType faces and knockout draw as plain stroke text; add them
/// when boards need them (KiCad 7+ saves TrueType outlines in `render_cache`).
pub(crate) fn effects(c: &mut Cursor, t: &mut Text, hidden: &mut bool) -> Result<bool> {
    let mut bold = false;
    loop {
        while let Some(flag) = c.opt_atom()? {
            *hidden |= flag == "hide";
        }
        let Some(head) = c.child()? else { return Ok(bold) };
        match head {
            "font" => loop {
                // KiCad 6 writes `bold` as a bare flag.
                while let Some(flag) = c.opt_atom()? {
                    bold |= flag == "bold";
                }
                let Some(head) = c.child()? else { break };
                match head {
                    // Height comes first in the file.
                    "size" => (t.size.1, t.size.0) = (c.nm()?, c.nm()?),
                    "thickness" => t.thickness = c.nm()?,
                    "bold" => bold = c.atom()? == "yes",
                    "face" => t.rough = true,
                    _ => {}
                }
                c.close()?;
            },
            "justify" => {
                while let Some(a) = c.opt_atom()? {
                    match a {
                        "left" => t.justify.0 = -1,
                        "right" => t.justify.0 = 1,
                        "top" => t.justify.1 = -1,
                        "bottom" => t.justify.1 = 1,
                        "mirror" => t.mirror = true,
                        _ => {}
                    }
                }
            }
            "hide" => *hidden = c.atom()? == "yes",
            _ => {}
        }
        c.close()?;
    }
}

/// `(tenting front back)` (KiCad 9) or `(tenting (front yes) (back no))` (KiCad 10): whether the
/// mask covers vias on the front and on the back.
// shortcut: a via's own tenting (KiCad 9+) isn't read (no corpus board sets one); read it when a board does.
fn tenting(c: &mut Cursor) -> Result<(bool, bool)> {
    let mut t = (false, false);
    loop {
        while let Some(side) = c.opt_atom()? {
            match side {
                "front" => t.0 = true,
                "back" => t.1 = true,
                _ => {}
            }
        }
        let Some(side) = c.child()? else { return Ok(t) };
        let on = c.atom()? == "yes";
        match side {
            "front" => t.0 = on,
            "back" => t.1 = on,
            _ => {}
        }
        c.close()?;
    }
}

/// KiCad 6–7's lock on a track or via: `locked` right after its head (KiCad 8 on: `(locked yes)`).
fn locked(c: &mut Cursor) -> Result<bool> {
    Ok(c.opt_atom()? == Some("locked"))
}

/// Reads `(xy x y)` points; an `(arc (start) (mid) (end))` among them becomes points along it.
pub(crate) fn pts_into(c: &mut Cursor, out: &mut Vec<Pt>) -> Result<()> {
    while let Some(head) = c.child()? {
        match head {
            "xy" => out.push(c.point()?),
            "arc" => {
                let mut ends = vec![];
                while let Some(head) = c.child()? {
                    if matches!(head, "start" | "mid" | "end") {
                        ends.push(c.point()?);
                    }
                    c.close()?;
                }
                let [s, m, e] = ends[..] else { return Err(c.err(tr("incomplete arc"))) };
                let skip = (out.last() == Some(&s)) as usize;
                out.extend(arc_points(s, m, e).into_iter().skip(skip));
            }
            _ => {}
        }
        c.close()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Model, PadShape};

    #[test]
    fn reads_the_projects_net_classes_and_which_nets_each_takes() {
        let mut b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
              (net 0 "") (net 1 "+5V") (net 2 "/USB/D+") (net 3 "/USB/D-") (net 4 "SDA") (net 5 "VBUS") (net 6 "/eth/TX") (net 7 "SCL"))"#,
        )
        .unwrap();
        assert_eq!((b.class(1), b.clearances(), b.track_width(1)), (None, vec![200_000; 8], 250_000));
        b.apply_pro(
            r#"{"board": {"design_settings": {"rules": {"min_clearance": 0.15}}}, "net_settings": {"classes": [
                {"name": "Default", "clearance": 0.2, "track_width": 0.25, "via_diameter": 0.8, "via_drill": 0.4},
                {"name": "Power", "clearance": 0.1, "track_width": 0.5, "via_diameter": 1.0, "via_drill": 0.5, "nets": ["VBUS"]},
                {"name": "USB", "clearance": 0.3, "diff_pair_width": 0.2, "diff_pair_gap": 0.15}],
              "netclass_assignments": {"SDA": ["USB", "Power"], "/eth/TX": "Power"},
              "netclass_patterns": [{"netclass": "Power", "pattern": "+5V"}, {"netclass": "USB", "pattern": "/USB/D?"},
                {"netclass": "Power", "pattern": "/USB/*"}, {"netclass": "Gone", "pattern": "*"}]}}"#,
        );
        // A class's values, those it leaves out Default's; its clearance never below the board's minimum.
        let class = |name: &str, c, w, via, (pair, gap)| NetClass {
            name: name.into(),
            clearance: c,
            track_width: w,
            via_diameter: via,
            via_drill: via / 2,
            pair_width: pair,
            pair_gap: gap,
        };
        let (power, usb) =
            (class("Power", 150_000, 500_000, 1_000_000, (200_000, 250_000)), class("USB", 300_000, 250_000, 800_000, (200_000, 150_000)));
        assert_eq!(b.classes, [power, usb]);
        // KiCad 6's list in the class, the schematic's assignments (the first of several), then
        // the patterns in their order, the first that fits winning; a class that isn't, left out.
        let nets: Vec<_> = b.class_nets.iter().map(|(n, c)| (n.as_str(), *c)).collect();
        assert_eq!(nets, [("VBUS", 0), ("SDA", 1), ("/eth/TX", 0), ("+5V", 0), ("/USB/D?", 1), ("/USB/*", 0)]);
        let of = |net| b.class(net).map(|c| c.name.as_str());
        assert_eq!(
            (1..8).map(of).collect::<Vec<_>>(),
            [Some("Power"), Some("USB"), Some("USB"), Some("USB"), Some("Power"), Some("Power"), None]
        );
        assert_eq!(b.clearances(), [200_000, 150_000, 300_000, 300_000, 300_000, 150_000, 150_000, 200_000]);
        // New tracks and vias take their net's class's sizes; a route steps through the classes' widths.
        assert_eq!((b.track_width(1), b.track_width(2), b.track_width(7)), (500_000, 250_000, 250_000));
        let via = |net| (b.via(Pt::default(), net).size, b.via(Pt::default(), net).drill);
        assert_eq!((via(5), via(7)), ((1_000_000, 500_000), (800_000, 400_000)));
        assert_eq!(b.step_width(400_000, true), Some(500_000));
        // Patterns: `*` any run of letters, `?` any one.
        let fits =
            [("/USB/D?", "/USB/D+"), ("*unconnected-(U22*", "unconnected-(U22-Pad3)"), ("a*b*c", "aXbbYc"), ("a*b", "ab"), ("*", "")];
        let fails = [("/Ethernet/*", "/Eth"), ("a?b", "ab"), ("a*b", "abc"), ("SDA", "SDA1")];
        assert!(fits.iter().all(|(p, n)| crate::board::matches(p, n)) && !fails.iter().any(|(p, n)| crate::board::matches(p, n)));
        // Another project's classes take the last one's place.
        b.apply_pro("{}");
        assert_eq!((b.classes.len(), b.class_nets.len()), (0, 0));
    }

    #[test]
    fn parses_kicad8_items_in_board_coordinates() {
        let b = Board::parse(
            r#"(kicad_pcb (version 20240108) (generator "pcbnew")
              (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (net 0 "") (net 1 "GND")
              (footprint "R" (layer "F.Cu") (at 10 20 -270)
                (fp_line (start 0 0) (end 1 0) (stroke (width 0.1) (type solid)) (layer "F.Cu"))
                (pad "1" smd roundrect (at 1 0 90) (size 1 0.5) (layers "F.Cu" "F.Mask")
                  (roundrect_rratio 0.2) (net 1 "GND"))
                (pad "2" thru_hole circle (at 0 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu" "*.Mask"))
                (fp_text user "In5" (at 0 0) (layer "In5.Cu") (effects (font (size 1 1)))))
              (gr_arc (start 0 0) (mid 1 1) (end 2 0) (stroke (width 0.05)) (layer "Edge.Cuts"))
              (segment (start 0 0) (end 5 0) (width 0.25) (layer "F.Cu") (net 1))
              (arc (start 0 0) (mid 1 1) (end 2 0) (width 0.2) (locked yes) (layer "B.Cu") (net "GND"))
              (via (at 5 0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1))
              (via locked (at 6 0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1))
              (zone (net 1) (layer "F.Cu") (filled_polygon (layer "B.Cu") (pts (xy 0 0) (xy 1 0) (xy 1 1)))))"#,
        )
        .unwrap();
        assert_eq!(b.layers, ["F.Cu", "B.Cu", "Edge.Cuts", "F.Mask", "In5.Cu"]);
        // Only the table makes copper, not a layer named so that it lacks (KiCad's layer markers
        // write on every inner layer).
        assert_eq!(b.copper, 0b11);
        assert_eq!(b.nets, ["", "GND"]);
        // -270° reads as 90°: local (1, 0) turned 90° counter-clockwise on screen is (0, -1),
        // then moved to (10, 20).
        assert_eq!((b.pads[0].at, b.footprints[0].angle), (Pt { x: 10 * MM, y: 19 * MM }, 90.0));
        assert_eq!((b.pads[0].angle, b.pads[0].shape, b.pads[0].layers, b.pads[0].net), (90.0, PadShape::RoundRect(0.2), 0b1001, 1));
        assert_eq!((b.pads[1].drill, b.pads[1].layers), (Some((800_000, 800_000)), 0b1011));
        assert_eq!(b.shapes[0].geom, Geom::Segment { a: Pt { x: 10 * MM, y: 20 * MM }, b: Pt { x: 10 * MM, y: 19 * MM } });
        assert!(matches!(b.shapes[1].geom, Geom::Arc { .. }));
        assert_eq!((b.tracks[0].mid, b.tracks[1].net, b.tracks[1].layer), (None, 1, 1));
        assert_eq!((b.vias[0].layers, b.vias[0].drill), (0b11, 300_000));
        // Locked as KiCad 8 on writes it, and as KiCad 6 and 7 do.
        assert_eq!([b.tracks[0].locked, b.tracks[1].locked, b.vias[0].locked, b.vias[1].locked], [false, true, false, true]);
        assert_eq!((b.fills[0].layer, b.fills[0].net, b.fills[0].pts.len()), (1, 1, 3));
    }

    #[test]
    fn reads_3d_models_thickness_and_stackup_colors() {
        // KiCad 5 offsets models in inches (`at`); 6 and 7 hide one with a bare flag, 8 on with `(hide yes)`.
        let b = Board::parse(
            r#"(kicad_pcb (general (thickness 0.8)) (layers (0 "F.Cu" signal))
              (setup (stackup (layer "F.SilkS" (type "Top Silk Screen") (color "Yellow"))
                (layer "F.Mask" (type "Top Solder Mask") (color "Black") (thickness 0.01)) (layer "F.Cu" (type "copper"))))
              (module R (layer F.Cu) (at 0 0) (model ${KISYS3DMOD}/R.wrl (at (xyz 0 0.1 0)) (scale (xyz 1 1 1)) (rotate (xyz 0 0 0))))
              (footprint "C" (layer "F.Cu") (at 0 0)
                (model "${KICAD10_3DMODEL_DIR}/C.step" (offset (xyz 0 0 0.2)) (scale (xyz 1 2 1)) (rotate (xyz -90 0 90)))
                (model "${KIPRJMOD}/C.step" hide) (model "x.step" (hide yes))))"#,
        )
        .unwrap();
        let models: Vec<_> = b.footprints.iter().flat_map(|f| &f.models).collect();
        let r = Model { file: "${KISYS3DMOD}/R.wrl".into(), offset: [0.0, 2.54, 0.0], scale: [1.0; 3], rotate: [0.0; 3] };
        let c = Model {
            file: "${KICAD10_3DMODEL_DIR}/C.step".into(),
            offset: [0.0, 0.0, 0.2],
            scale: [1.0, 2.0, 1.0],
            rotate: [-90.0, 0.0, 90.0],
        };
        assert_eq!(models, [&r, &c]);
        assert_eq!((b.thickness, Board::parse("(kicad_pcb)").unwrap().thickness), (800_000, DEFAULT_BOARD));
        assert_eq!((&b.mask_colors, &b.silk_colors), (&[Some("Black".into()), None], &[Some("Yellow".into()), None]));
    }

    #[test]
    fn reads_design_rules_and_the_default_net_class() {
        let mut b = Board::parse(
            r#"(kicad_pcb (version 20171130) (setup (trace_min 0.15) (zone_clearance 0.254) (pad_to_mask_clearance 0.051)
              (pad_to_paste_clearance -0.02) (pad_to_paste_clearance_ratio -0.12) (grid_origin 115.3 -80.7) (pcbplotparams (viasonmask true)))
              (net_class Default "This is the default net class." (clearance 0.18) (trace_width 0.3) (via_dia 0.7) (via_drill 0.35) (add_net GND))
              (net_class Power "" (clearance 0.5) (trace_width 1)))"#,
        )
        .unwrap();
        let rules = |b: &Board| {
            let r = b.rules;
            (r.track_width, r.clearance, r.class_track_width, r.class_via_diameter, r.class_via_drill, r.zone_clearance)
        };
        assert_eq!(rules(&b), (150_000, 180_000, 300_000, 700_000, 350_000, 254_000));
        assert_eq!(b.grid_origin, Pt { x: 115_300_000, y: -80_700_000 });
        // Masks and paste; KiCad <= 8 left vias open in the plot settings, KiCad 9 and 10 in the setup.
        assert_eq!(
            (b.rules.mask_margin, b.rules.paste_margin, b.rules.paste_ratio, b.rules.tented),
            (51_000, -20_000, -0.12, (false, false))
        );
        let tented = |setup: &str| Board::parse(&format!("(kicad_pcb (setup {setup}))")).unwrap().rules.tented;
        assert_eq!(
            [
                tented(""),
                tented("(tenting front)"),
                tented("(tenting (front no) (back yes))"),
                tented("(pcbplotparams (viasonmask false))")
            ],
            [(true, true), (true, false), (false, true), (true, true)]
        );
        // Laid out as KiCad writes it: severities and zone defaults reuse the rules' keys; Default is one class of several.
        b.apply_pro(
            r#"{"board": {"design_settings": {"defaults": {"zones": {"min_clearance": 0.5}},
              "rule_severities": {"clearance": "error", "track_width": "error"},
              "rules": {"min_clearance": 0.1, "min_track_width": 0.12}}},
              "net_settings": {"classes": [
                {"clearance": 0.127, "name": "Default", "track_width": 0.2, "via_diameter": 0.45, "via_drill": 0.25},
                {"clearance": 0.3, "name": "Power", "track_width": 1.0}]}}"#,
        );
        assert_eq!(rules(&b), (120_000, 127_000, 200_000, 450_000, 250_000, 500_000));
        // A minimum set to 0 is none (hackrf-one's edge clearance), not the 0.5 mm it starts as;
        // a class's 0 is no size, and stays as it was.
        assert_eq!(b.rules.edge_clearance, 500_000);
        b.apply_pro(r#"{"board": {"design_settings": {"rules": {"min_copper_edge_clearance": 0.0}}}, "net_settings": {"classes": [{"name": "Default", "track_width": 0.0}]}}"#);
        assert_eq!((b.rules.edge_clearance, b.rules.class_track_width), (0, 200_000));
    }

    #[test]
    fn parses_kicad5_modules_and_legacy_arcs() {
        let b = Board::parse(
            "(kicad_pcb (version 20171130) (host pcbnew 5.1.6)
              (layers (0 F.Cu signal) (31 B.Cu signal) (44 Edge.Cuts user))
              (net 0 \"\") (net 1 GND)
              (module R (layer F.Cu) (at 0 0) (pad 1 smd rect (at 0 0) (size 1 1) (layers F.Cu) (net 1 GND)))
              (gr_arc (start 0 0) (end 1 0) (angle 90) (layer Edge.Cuts) (width 0.1))
              (gr_poly (pts (xy 0 0) (xy 1 0) (xy 1 1)) (layer F.Cu) (width 0))
              (gr_circle (center 0 0) (end 1 0) (layer F.Cu) (width 0.1)))",
        )
        .unwrap();
        assert_eq!((b.pads[0].net, b.nets[1].as_str()), (1, "GND"));
        let s = Pt { x: MM, y: 0 };
        assert_eq!(b.shapes[0].geom, Geom::Arc { start: s, mid: Pt { x: 707_107, y: 707_107 }, end: Pt { x: 0, y: MM } });
        // KiCad 5 wrote no fill: its polygons are filled, its circles not.
        assert_eq!((b.shapes[1].filled, b.shapes[2].filled), (true, false));
    }

    #[test]
    fn parses_shown_texts_with_kicad_upright_and_variable_rules() {
        let b = Board::parse(
            r#"(kicad_pcb (version 20240108) (title_block (rev "1.3") (comment 2 "x")) (property "VERSION" "10")
              (layers (0 "F.Cu" signal) (37 "F.SilkS" user) (49 "F.Fab" user))
              (gr_text "A\"B\nC" (at 1 2 180) (layer "F.SilkS")
                (effects (font (size 2 1.5) (thickness 0.3)) (justify right top mirror)))
              (footprint "R" (layer "F.Cu") (at 10 20 180)
                (property "Reference" "R1" (at 0 -1 180) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
                (property "Value" "10k" (at 0 1 0) (layer "F.Fab") (hide yes) (effects (font (size 1 1))))
                (property "Sheetfile" "a.kicad_sch")
                (fp_text user "${REFERENCE}=${VALUE}" (at 0 0 180 unlocked) (layer "F.Fab") (effects (font (size 1 2))))
                (fp_text user "hidden" (at 0 0) (layer "F.Fab") hide (effects (font (size 1 1))))
                (fp_text user %R (at 0 0) (layer F.Fab) (effects (font (size 1 1)))))
              (gr_text "r${VERSION} ${REVISION} ${COMMENT2} ${NOPE}" (at 0 0) (layer "F.SilkS")))"#,
        )
        .unwrap();
        // The project's variables fill in what the board's own left: its own win.
        let mut named = b.clone();
        named.apply_pro(r#"{"text_variables": {"NOPE": "yes", "VERSION": "11"}}"#);
        assert_eq!(named.texts[4].text, "r10 1.3 x yes");
        let t: Vec<_> = b.texts.iter().map(|t| (t.text.as_str(), t.at, t.angle)).collect();
        assert_eq!(
            t,
            [
                ("A\"B\nC", Pt { x: MM, y: 2 * MM }, 180.0),
                // Locked: 180° shows upright; local (0, -1) turned 180° lands below the footprint.
                ("R1", Pt { x: 10 * MM, y: 21 * MM }, 0.0),
                ("R1=10k", Pt { x: 10 * MM, y: 20 * MM }, 180.0),
                ("R1", Pt { x: 10 * MM, y: 20 * MM }, 0.0),
                ("r10 1.3 x ${NOPE}", Pt::default(), 0.0),
            ]
        );
        let g = &b.texts[0];
        assert_eq!((g.size, g.thickness, g.layer, g.justify, g.mirror), ((1_500_000, 2 * MM), 300_000, 1, (1, -1), true));
        // No thickness in the file: KiCad draws width / 8.
        assert_eq!((b.texts[2].size, b.texts[2].thickness, b.texts[2].layer), ((2 * MM, MM), 250_000, 2));
    }
}
