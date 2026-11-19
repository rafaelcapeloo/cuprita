//! The Inspect tab: what is picked, in numbers. On the board a part, track, via or zone and the net
//! lit, or how many of each are picked together; in the schematic the part selected; with
//! nothing picked, the board itself.

use crate::cmd::Cmd;
use crate::dock::heading;
use crate::tr::{n, t, tr, trf};
use crate::units::{len, num};
use crate::{App, count, icon, picks_name, theme, units};
use cuprita_core::board::{Board, Nm, Span, Via};
use cuprita_core::bom::part_number_column;
use cuprita_core::history::Pick;
use cuprita_core::schematic::Item;
use eframe::egui::{self, RichText};

/// A line of the tab: what, then its value in mono on the right.
pub(crate) fn row(ui: &mut egui::Ui, what: &str, value: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(what).color(theme::QUIET));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(value.into()).monospace().color(theme::TEXT)).truncate());
        });
    });
}

/// How far via `v` goes: through the board, or between which layers, blind or buried.
fn via_span(b: &Board, v: &Via) -> String {
    let mut on = b.stackup.iter().filter(|c| v.layers >> c.layer & 1 != 0).map(|c| b.layers[c.layer as usize].as_str());
    let top = on.next().unwrap_or_default();
    let bottom = on.next_back().unwrap_or(top);
    match b.span(v) {
        Span::Through => tr("through the board").into(),
        Span::Blind => trf("{0} to {1}, blind", &[&top, &bottom]),
        Span::Buried => trf("{0} to {1}, buried", &[&top, &bottom]),
    }
}

/// The tab's title: an icon, a name in mono and what it is.
fn title(ui: &mut egui::Ui, icon: &str, name: &str, what: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(18.0).color(theme::SIGNAL));
        ui.label(RichText::new(name).monospace().size(15.0).color(theme::TEXT));
        ui.add(egui::Label::new(RichText::new(what).color(theme::TEXT_2)).truncate());
    });
    ui.add_space(4.0);
}

impl App {
    pub(crate) fn inspect_tab(&mut self, ui: &mut egui::Ui) {
        // A served view reads what is picked, as clear as ever: its fields and buttons set nothing.
        if self.viewer {
            ui.visuals_mut().disabled_alpha = 1.0;
            ui.disable();
        }
        if self.sch_view() {
            return self.inspect_symbol(ui);
        }
        let b = &self.board;
        // While routing, the track being routed takes the place of what is picked.
        let routing = self.route.as_ref().filter(|_| self.routing).map(|r| r.start);
        if let Some(s) = routing {
            let net = b.nets.get(s.net as usize).map_or("", String::as_str);
            title(ui, icon::ROUTE, if net.is_empty() { tr("no net") } else { net }, tr("routing"));
            let from = b.copper_at(s.at, 1, &[s.layer]).map_or(String::new(), |i| crate::helpers::copper_name(b, i));
            let from = from.split(" · ").next().unwrap_or_default();
            let left = self.ratsnest.iter().filter(|l| l.net == s.net).count();
            let left =
                if left == 0 { tr("every connection routed").to_string() } else { count(left, n!("connection left|connections left")) };
            let what = if from.is_empty() { left } else { trf("from {0} · {1}", &[&from, &left]) };
            ui.label(RichText::new(what).size(12.0).color(theme::TEXT_2));
            ui.add_space(4.0);
            row(ui, tr("Layer"), b.layers[s.layer as usize].as_str());
            row(ui, tr("Width"), len(s.width));
            let (keeps, via) = (b.class(s.net).map_or(b.rules.clearance, |c| c.clearance), b.via(s.at, s.net));
            row(ui, tr("Clearance"), len(keeps));
            row(ui, tr("Via"), format!("{} / {}", num(via.size), len(via.drill)));
            if b.stackup.len() > 2 {
                row(ui, tr("Vias go"), tr(if self.blind { t!("between the layers joined") } else { t!("through the board") }));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let mut choose_model = None;
        let mut show_text = None;
        match self.selected.filter(|_| routing.is_none()) {
            Some(Pick::Footprint(i)) => {
                let f = &b.footprints[i];
                let description = f.fields.iter().find(|(k, _)| k == "Description").map_or("", |(_, v)| v.as_str());
                title(ui, icon::PART, &f.reference, &f.value);
                if !description.is_empty() {
                    ui.add(egui::Label::new(RichText::new(description).color(theme::TEXT_2)).wrap());
                }
                row(ui, tr("Footprint"), f.lib_id.rsplit(':').next().unwrap_or_default());
                row(ui, tr("Side"), if f.back { tr("bottom") } else { tr("top") });
                row(ui, "X", len(f.at.x));
                row(ui, "Y", len(f.at.y));
                row(ui, tr("Turned"), format!("{}°", (f.angle * 10.0).round() / 10.0));
                row(ui, tr("Pads"), f.pads.len().to_string());
                // Its 3D model, and Choose… for one from a file (a vendor's STEP).
                #[cfg(not(target_arch = "wasm32"))]
                ui.horizontal(|ui| {
                    let model = f.models.first().map_or(tr("none"), |m| m.file.rsplit(['/', '\\']).next().unwrap_or(&m.file));
                    ui.label(RichText::new(tr("3D")).color(theme::QUIET));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.link(tr("Choose…")).on_hover_text(tr("A 3D model from a file: a vendor's STEP")).clicked() {
                            choose_model = Some(i);
                        }
                        ui.add(egui::Label::new(RichText::new(model).monospace().color(theme::TEXT)).truncate());
                    });
                });
                if !f.fitted {
                    row(ui, tr("Fitted"), tr("no (DNP)"));
                }
                let skip = ["Description", "Sheetfile", "Sheetname", "ki_fp_filters", "ki_description", "ki_keywords"];
                for (k, v) in f.fields.iter().filter(|(k, v)| !v.is_empty() && !skip.contains(&k.as_str())) {
                    row(ui, k, v.as_str());
                }
                // Its texts not shown (its reference hidden, a value, a field): a click shows one.
                let hidden: Vec<(usize, String)> = (f.hidden.iter().enumerate())
                    .filter(|(_, t)| !t.text.trim().is_empty())
                    .map(|(k, t)| (k, t.text.lines().next().unwrap_or_default().chars().take(24).collect()))
                    .collect();
                if !hidden.is_empty() {
                    ui.label(RichText::new(tr("Hidden texts")).small().color(theme::QUIET));
                    ui.horizontal_wrapped(|ui| {
                        for (k, words) in hidden {
                            if ui.link(words).on_hover_text(tr("Shows this text on the board again, picked to drag into place")).clicked() {
                                show_text = Some((i, k));
                            }
                        }
                    });
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for cmd in [Cmd::Turn, Cmd::TurnBack, Cmd::FlipPart, Cmd::TidyTexts, Cmd::Delete, Cmd::Schematic] {
                        self.icon_cmd(ui, cmd, 32.0, false);
                    }
                });
            }
            Some(Pick::Track(i)) => {
                let t = &b.tracks[i];
                title(ui, icon::ROUTE, tr("Track"), &b.layers[t.layer as usize]);
                row(ui, tr("Net"), b.nets[t.net as usize].as_str());
                row(ui, tr("Length"), len(t.len()));
                self.track_width(ui, &[i]);
                self.tune_field(ui, i);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for cmd in [Cmd::Run, Cmd::Delete] {
                        self.icon_cmd(ui, cmd, 32.0, false);
                    }
                });
            }
            Some(Pick::Via(i)) => {
                let v = &b.vias[i];
                title(ui, icon::VIA, tr("Via"), "");
                row(ui, tr("Net"), b.nets[v.net as usize].as_str());
                row(ui, tr("Size"), len(v.size));
                row(ui, tr("Drill"), len(v.drill));
                row(ui, tr("Goes"), via_span(b, v));
                ui.add_space(6.0);
                self.icon_cmd(ui, Cmd::Delete, 32.0, false);
            }
            Some(Pick::Zone(i)) => {
                let z = &b.zones[i];
                let on: Vec<&str> = (0..b.layers.len()).filter(|&l| z.layers >> l & 1 != 0).map(|l| b.layers[l].as_str()).collect();
                title(ui, icon::ZONE, if z.keepout { tr("Rule area") } else { tr("Zone") }, &on.join(", "));
                if z.keepout {
                    row(ui, tr("Keeps out"), tr("zone copper"));
                } else {
                    row(ui, tr("Filled area"), format!("{:.2} mm²", b.fill_area(i)));
                }
                self.zone_settings(ui, i);
                ui.add_space(6.0);
                let hint = tr("Drag a corner to move it, an edge's middle to add one; Delete with the pointer on a corner takes it away.");
                ui.label(RichText::new(hint).small().color(theme::QUIET));
                ui.horizontal(|ui| {
                    for cmd in [Cmd::Cutout, Cmd::Delete] {
                        self.icon_cmd(ui, cmd, 32.0, false);
                    }
                });
            }
            Some(Pick::Text(i)) => {
                title(ui, icon::TEXT, tr("Text"), &b.layers[b.texts[i].layer as usize]);
                self.text_settings(ui, i);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for cmd in [Cmd::Turn, Cmd::TurnBack, Cmd::Delete] {
                        self.icon_cmd(ui, cmd, 32.0, false);
                    }
                });
            }
            Some(Pick::Shape(i)) => {
                title(ui, icon::LINE, tr(crate::drawing::kind(&b.shapes[i])), &b.layers[b.shapes[i].layer as usize]);
                self.shape_settings(ui, i);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for cmd in [Cmd::Turn, Cmd::TurnBack, Cmd::Delete] {
                        self.icon_cmd(ui, cmd, 32.0, false);
                    }
                });
            }
            None if !self.picked.is_empty() => self.inspect_picks(ui),
            None => {}
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(i) = choose_model {
            self.ask_model(i);
        }
        if let Some((part, k)) = show_text {
            self.show_text(part, k);
        }
        // A change refused says why here, beside the item (the status bar shows what is picked),
        // while that stays picked.
        self.refused = self.refused.take().filter(|(p, _)| self.picks().contains(p));
        if let Some((_, why)) = &self.refused {
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(why).color(theme::FAIL)).wrap());
        }
        let (b, lit) = (&self.board, routing.map_or(self.net, |s| s.net));
        if lit != 0 {
            ui.add_space(10.0);
            heading(ui, if routing.is_some() { tr("The net so far") } else { tr("Net") });
            let n = b.inspect_net(lit);
            row(ui, tr("Name"), b.nets[lit as usize].as_str());
            #[cfg(not(target_arch = "wasm32"))]
            self.net_class(ui, lit);
            row(ui, tr("Length"), len(n.length_mm * 1e6));
            // One side of a differential pair: the other, and how much longer (+) or shorter this one is.
            if let Some(pair) = self.board.pair(lit) {
                let other = pair.nets[usize::from(pair.nets[0] == lit)];
                let skew = n.length_mm * 1e6 - self.board.net_length(other);
                row(ui, tr("Pair with"), self.board.nets[other as usize].as_str());
                row(ui, tr("Skew"), format!("{}{}", if skew < 0.0 { "−" } else { "+" }, len(skew.abs())));
            }
            match n.width {
                (0, 0) => {}
                (w0, w1) if w0 == w1 => row(ui, tr("Width"), len(w0)),
                (w0, w1) => row(ui, tr("Width"), format!("{}–{}", num(w0), len(w1))),
            }
            if n.width.1 > 0 {
                row(ui, tr("Current, 10 °C rise"), format!("{:.2} A", n.current_a));
            }
            match n.impedance_ohm {
                (_, z1) if z1 <= 0.0 => {}
                (z0, z1) if (z1 - z0).abs() < 0.05 => row(ui, tr("Impedance"), format!("{z0:.1} Ω")),
                (z0, z1) => row(ui, tr("Impedance"), format!("{z0:.1}–{z1:.1} Ω")),
            }
            row(ui, tr("Items"), format!("{} · {} · {}", count(n.pads, n!("pad")), count(n.tracks, n!("track")), count(n.vias, n!("via"))));
            let note = tr("Current by IPC-2152 at the narrowest track; impedance from the stackup.");
            ui.label(RichText::new(note).small().color(theme::QUIET));
        }
        if self.picks().is_empty() && lit == 0 && routing.is_none() {
            self.inspect_board(ui);
        }
    }

    /// Several items picked on the board: how many of each, and what they do together.
    fn inspect_picks(&mut self, ui: &mut egui::Ui) {
        let has = |kind: fn(&Pick) -> bool| self.picked.iter().any(kind);
        let icon = match () {
            _ if has(|p| matches!(p, Pick::Footprint(_))) => icon::PART,
            _ if has(|p| matches!(p, Pick::Track(_))) => icon::ROUTE,
            _ if has(|p| matches!(p, Pick::Via(_))) => icon::VIA,
            _ if has(|p| matches!(p, Pick::Zone(_))) => icon::ZONE,
            _ if has(|p| matches!(p, Pick::Text(_))) => icon::TEXT,
            _ => icon::LINE,
        };
        title(ui, icon, "", &picks_name(&self.board, &self.picked));
        // Tracks alone (a run) take a width together.
        let tracks: Vec<usize> = self.picked.iter().map_while(|&p| if let Pick::Track(i) = p { Some(i) } else { None }).collect();
        if tracks.len() == self.picked.len() {
            self.track_width(ui, &tracks);
        }
        ui.add_space(6.0);
        let hint = tr("Drag one to move them all, the arrows a grid step; tracks not picked stay where they are.");
        ui.label(RichText::new(hint).small().color(theme::QUIET));
        ui.horizontal(|ui| {
            for cmd in [Cmd::Turn, Cmd::TurnBack, Cmd::FlipPart, Cmd::Delete] {
                self.icon_cmd(ui, cmd, 32.0, false);
            }
        });
    }

    /// The Width field of tracks `ts`, their width when they share one: typed in the units shown
    /// or with its own (`0.3mm`, `12mil`), it sets them all, one undo step.
    fn track_width(&mut self, ui: &mut egui::Ui, ts: &[usize]) {
        let key = Pick::Track(ts[0]);
        let widths: std::collections::BTreeSet<Nm> = ts.iter().map(|&i| self.board.tracks[i].width).collect();
        let shown = if widths.len() == 1 { len(widths.into_iter().next().unwrap_or_default()) } else { String::new() };
        let mut typing = self.typing.take().filter(|(k, ..)| *k == key).map(|(_, name, text)| (name, text));
        let typed = field_row(ui, tr("Width"), &shown, &mut typing);
        self.typing = typing.map(|(name, text)| (key, name, text));
        let Some(text) = typed else { return };
        match units::parse(&text).filter(|&w| w > 0) {
            Some(w) => {
                let (t, undo) = (web_time::Instant::now(), self.board.set_widths(ts, w));
                let what = trf("{0} {1} wide", &[&count(ts.len(), n!("track")), &len(w)]);
                self.edited(undo, t, what);
            }
            None => {
                let why = trf("Width {0} isn't a length: type one in {1}, or as 0.3mm or 12mil", &[&format!("{text:?}"), &units::name()]);
                self.refused = Some((key, why));
            }
        }
    }

    /// The length of track `i`'s net, to tune: typed longer, the track takes a meander that adds
    /// the rest (`Board::tune`); the shorter side of a differential pair has a link that makes it
    /// as long as the other.
    fn tune_field(&mut self, ui: &mut egui::Ui, i: usize) {
        let (key, net) = (Pick::Track(i), self.board.tracks[i].net);
        if net == 0 {
            return;
        }
        let now = self.board.net_length(net);
        let mut typing = self.typing.take().filter(|(k, ..)| *k == key).map(|(_, name, text)| (name, text));
        let typed = field_row(ui, tr("Net length"), &len(now), &mut typing);
        self.typing = typing.map(|(name, text)| (key, name, text));
        let mut target = typed.map(|text| {
            let why =
                || trf("Length {0} isn't a length: type one in {1}, or as 45.2mm or 1780mil", &[&format!("{text:?}"), &units::name()]);
            units::parse(&text).map(f64::from).ok_or_else(why)
        });
        let other = self.board.pair(net).map(|p| p.nets[usize::from(p.nets[0] == net)]);
        if let Some((other, longer)) = other.map(|o| (o, self.board.net_length(o))).filter(|&(_, l)| l > now + 1000.0) {
            let link = ui.link(trf("Match {0}: {1}", &[&self.board.nets[other as usize], &len(longer)]));
            if link.on_hover_text(tr("A meander on this track makes its net as long as the pair's other side")).clicked() {
                target = Some(Ok(longer));
            }
        }
        match target {
            Some(Ok(to)) => self.tune(i, to),
            Some(Err(why)) => self.refused = Some((key, why)),
            None => {}
        }
    }

    /// With nothing picked: the board in numbers, and its project's net classes (`classes.rs`).
    fn inspect_board(&mut self, ui: &mut egui::Ui) {
        let Some(scene) = &self.scene else {
            ui.label(RichText::new(tr("Open a board to inspect it.")).color(theme::QUIET));
            return;
        };
        let b = &self.board;
        let [x0, y0, x1, y1] = scene.bounds;
        title(ui, icon::BOARD, &self.name, "");
        row(ui, tr("Size"), format!("{} × {}", num((x1 - x0) * 1000.0), len((y1 - y0) * 1000.0)));
        row(ui, tr("Copper layers"), b.copper.count_ones().to_string());
        row(ui, tr("Parts"), b.footprints.len().to_string());
        row(ui, tr("Nets"), b.nets.len().saturating_sub(1).to_string());
        row(ui, tr("Tracks · vias"), format!("{} · {}", b.tracks.len(), b.vias.len()));
        row(ui, tr("Zones"), b.zones.len().to_string());
        ui.add_space(8.0);
        let hint =
            tr("Click a part, track, via, zone, text or line to inspect it; its net lights up. Shift+click or Shift+drag picks several.");
        ui.label(RichText::new(hint).color(theme::QUIET));
        #[cfg(not(target_arch = "wasm32"))]
        self.class_list(ui);
    }

    /// The part selected in the schematic: its fields, typed in place (Enter or leaving a field
    /// sets it on every unit, an edit to undo; Esc keeps it as it was), its footprint chosen from
    /// those its symbol's filters fit, and where the board has it.
    fn inspect_symbol(&mut self, ui: &mut egui::Ui) {
        let folder = self.folder();
        let (Some(sch), parts) = (&mut self.sch, &mut self.parts) else { return };
        let Some(part) = sch.part().map(String::from) else {
            return self.inspect_picked(ui);
        };
        if sch.fields.as_ref().is_none_or(|(p, _)| *p != part) {
            if sch.fields.as_ref().is_some_and(|(p, _)| *p != part) {
                (sch.typing, sch.choosing) = (None, None);
            }
            sch.fields = sch.sheets.project.fields(&part).map(|f| (part.clone(), f));
        }
        let Some((_, f)) = sch.fields.clone() else { return };
        let get = |name: &str| f.fields.iter().find(|(k, _)| k == name).map_or("", |(_, v)| v.as_str());
        title(ui, icon::PART, &part, get("Value"));
        let mut set = None;
        // The reference names each place of the part: KiCad's annotation renumbers them.
        for (name, value) in f.fields.iter().filter(|(k, _)| k != "Reference" && !k.starts_with("ki_")) {
            if name == "Footprint" {
                set = set.or(choose_footprint(ui, value, &f.filters, &mut sch.choosing, parts, folder.as_deref()));
            } else if let Some(v) = field_row(ui, name, value, &mut sch.typing) {
                set = Some((name.clone(), v));
            }
        }
        // Assembly buys a part by its number: the fields it reads, for a part that has neither.
        for (column, name) in [(1, "MPN"), (2, "LCSC")] {
            if !f.fields.iter().any(|(k, _)| part_number_column(k) == Some(column))
                && let Some(v) = field_row(ui, name, "", &mut sch.typing)
            {
                set = Some((name.into(), v));
            }
        }
        if let Some(fp) = self.board.footprints.iter().find(|f| f.reference == part) {
            ui.add_space(8.0);
            heading(ui, tr("On the board"));
            row(ui, tr("Side"), if fp.back { tr("bottom") } else { tr("top") });
            row(ui, tr("At"), format!("{}, {}", num(fp.at.x), len(fp.at.y)));
        }
        ui.add_space(6.0);
        ui.label(RichText::new(tr("Drag it on the sheet to move it; its wires follow.")).small().color(theme::QUIET));
        ui.horizontal(|ui| {
            for cmd in [Cmd::Turn, Cmd::MirrorX, Cmd::MirrorY, Cmd::Duplicate, Cmd::Delete, Cmd::Board, Cmd::Update] {
                self.icon_cmd(ui, cmd, 32.0, false);
            }
        });
        if let Some((name, value)) = set {
            self.set_field(&part, &name, &value);
        }
    }
}

impl App {
    /// What is picked in the schematic when it isn't one part: a wire (its length), a label, or
    /// how many of each; else how to pick.
    fn inspect_picked(&mut self, ui: &mut egui::Ui) {
        let Some(sch) = &self.sch else { return };
        let sheet = &sch.sheets.project.pages[sch.page].sheet;
        let has = |kind: fn(&Item) -> bool| sch.picked.iter().any(kind);
        let (icon, length) = match sch.picked[..] {
            [] => {
                let hint = tr("Click a symbol, wire or label to inspect it; Shift+click or Shift+drag picks several.");
                ui.label(RichText::new(hint).color(theme::QUIET));
                return self.page_settings(ui);
            }
            [Item::Wire(i)] => {
                let (a, b) = sheet.extent(Item::Wire(i));
                (icon::WIRE, len((b.x as f64 - a.x as f64).hypot(b.y as f64 - a.y as f64)))
            }
            _ if has(|it| matches!(it, Item::Symbol(_))) => (icon::PART, String::new()),
            _ if has(|it| matches!(it, Item::Wire(_))) => (icon::WIRE, String::new()),
            _ if has(|it| matches!(it, Item::Label(_))) => (icon::LABEL, String::new()),
            _ => (icon::NO_CONNECT, String::new()),
        };
        title(ui, icon, &length, &sch.what());
        ui.add_space(6.0);
        let hint = match sch.picked.len() {
            1 => tr("Drag it on the sheet to move it; what joins it follows."),
            _ => tr("Drag one on the sheet to move them all; what joins them follows."),
        };
        ui.label(RichText::new(hint).small().color(theme::QUIET));
        ui.horizontal(|ui| {
            for cmd in [Cmd::Turn, Cmd::MirrorX, Cmd::MirrorY, Cmd::Duplicate, Cmd::Delete] {
                self.icon_cmd(ui, cmd, 32.0, false);
            }
        });
    }
}

/// A field typed in place: what it says, and the value to set once typing ends (Enter or
/// leaving it; Esc drops what was typed).
pub(crate) fn field_row(ui: &mut egui::Ui, name: &str, value: &str, typing: &mut Option<(String, String)>) -> Option<String> {
    ui.label(RichText::new(name).small().color(theme::QUIET));
    let mut text = match typing {
        Some((n, t)) if n == name => t.clone(),
        _ => value.to_string(),
    };
    let field = egui::TextEdit::singleline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY);
    let field = ui.add(field);
    if field.has_focus() {
        *typing = Some((name.to_string(), text.clone()));
    }
    if field.lost_focus() {
        let typed = typing.take_if(|(n, _)| n == name).map(|(_, t)| t);
        let kept = ui.input(|i| i.key_pressed(egui::Key::Escape));
        return typed.filter(|t| t != value && !kept);
    }
    None
}

/// The Footprint field: the footprint chosen, and a chooser of KiCad's footprints that fit the
/// symbol's `filters`, narrowed by the words typed; a click sets it. The pads of the one under
/// the pointer show beside the list.
pub(crate) fn choose_footprint(
    ui: &mut egui::Ui,
    value: &str,
    filters: &str,
    choosing: &mut Option<String>,
    parts: &mut Option<Vec<cuprita_core::library::LibPart>>,
    folder: Option<&std::path::Path>,
) -> Option<(String, String)> {
    ui.label(RichText::new(tr("Footprint")).small().color(theme::QUIET));
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = if choosing.is_some() { tr("Close") } else { tr("Choose…") };
            if ui.button(label).clicked() {
                *choosing = if choosing.is_some() { None } else { Some(String::new()) };
            }
            // The footprint in the room left, shortened: a long name would widen the panel.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let name = value.split_once(':').map_or(value, |(_, n)| n);
                let text = if value.is_empty() {
                    RichText::new(tr("none chosen: F8 can't add it to the board")).color(theme::WARN)
                } else {
                    RichText::new(name).monospace().color(theme::TEXT)
                };
                ui.add(egui::Label::new(text).truncate()).on_hover_text(value);
            });
        });
    });
    let words = choosing.as_mut()?;
    let all = parts.get_or_insert_with(|| crate::parts::footprints(folder));
    let typed = ui.add(egui::TextEdit::singleline(words).hint_text(tr("Words: 0603, SOT-23, 3.5mm…")).desired_width(f32::INFINITY));
    if typed.gained_focus() || words.is_empty() && !typed.has_focus() {
        typed.request_focus();
    }
    let lower: Vec<String> = words.split_whitespace().map(str::to_lowercase).collect();
    let fits = |p: &&cuprita_core::library::LibPart| {
        let id = p.id().to_lowercase();
        (filters.is_empty() || filters.split_whitespace().any(|f| cuprita_core::library::fits(f, p)))
            && lower.iter().all(|w| id.contains(w))
    };
    let mut hits: Vec<&cuprita_core::library::LibPart> = all.iter().filter(fits).collect();
    // The chosen footprint's library first: its neighbours are the likeliest other choices.
    let lib = value.split_once(':').map_or("", |(l, _)| l);
    hits.sort_by_key(|p| p.lib != lib);
    let note = match (hits.len(), filters.is_empty()) {
        (0, _) if all.is_empty() => tr("KiCad's footprint libraries aren't installed here").to_string(),
        (n, false) => format!("{} ({filters})", count(n, n!("fits its filters|fit its filters"))),
        (n, true) => count(n, n!("footprint")),
    };
    ui.label(RichText::new(note).small().color(theme::QUIET));
    let mut picked = None;
    let mut hovered = None;
    egui::ScrollArea::vertical().max_height(220.0).id_salt("footprints").show(ui, |ui| {
        for p in hits.iter().take(80) {
            let mut text = egui::text::LayoutJob::default();
            text.append(&p.name, 0.0, egui::TextFormat::simple(egui::FontId::monospace(12.0), theme::TEXT));
            text.append(&p.lib, 8.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), theme::QUIET));
            let row = ui.add(egui::Button::selectable(p.id() == value, text).truncate());
            if row.hovered() {
                hovered = Some(*p);
            }
            if row.clicked() {
                picked = Some(p.id());
            }
        }
    });
    if let Some(p) = hovered.or_else(|| hits.iter().copied().find(|p| p.id() == value)) {
        pads_preview(ui, p);
    }
    if picked.is_some() {
        *choosing = None;
    }
    picked.map(|id| ("Footprint".to_string(), id))
}

/// A footprint's pads, drawn to fit a small box: its land pattern at a glance.
fn pads_preview(ui: &mut egui::Ui, part: &cuprita_core::library::LibPart) {
    let mut board = cuprita_core::board::Board::default();
    let Ok(text) = std::fs::read_to_string(&part.path) else { return };
    if board.add_footprint(&text, &part.lib, cuprita_core::board::Pt::default(), 0.0).is_err() || board.pads.is_empty() {
        return;
    }
    let half = |p: &cuprita_core::board::Pad| {
        let (w, h) = (p.size.0 as f32 / 2.0, p.size.1 as f32 / 2.0);
        if (p.angle / 90.0).round() as i64 % 2 == 0 { (w, h) } else { (h, w) }
    };
    let (mut lo, mut hi) = (egui::pos2(f32::MAX, f32::MAX), egui::pos2(f32::MIN, f32::MIN));
    for p in &board.pads {
        let (w, h) = half(p);
        lo = lo.min(egui::pos2(p.at.x as f32 - w, p.at.y as f32 - h));
        hi = hi.max(egui::pos2(p.at.x as f32 + w, p.at.y as f32 + h));
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 110.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 8.0, theme::CANVAS);
    let span = (hi - lo).max(egui::vec2(1.0, 1.0));
    let scale = ((rect.size() - egui::vec2(24.0, 24.0)) / span).min_elem();
    let at = |x: f32, y: f32| rect.center() + (egui::vec2(x, y) - (lo.to_vec2() + span / 2.0)) * scale;
    let copper = egui::Color32::from_rgb(0xE5, 0x60, 0x4E);
    for pad in &board.pads {
        let (w, h) = half(pad);
        let c = at(pad.at.x as f32, pad.at.y as f32);
        let r = egui::Rect::from_center_size(c, egui::vec2(2.0 * w, 2.0 * h) * scale);
        let round = if matches!(pad.shape, cuprita_core::board::PadShape::Circle | cuprita_core::board::PadShape::Oval) {
            r.width().min(r.height()) / 2.0
        } else {
            2.0
        };
        p.rect_filled(r, round, copper);
        if let Some((d, _)) = pad.drill {
            p.circle_filled(c, d as f32 / 2.0 * scale, theme::CANVAS);
        }
    }
    p.text(
        rect.left_bottom() + egui::vec2(8.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        count(board.pads.len(), n!("pad")),
        egui::FontId::proportional(11.0),
        theme::QUIET,
    );
}
