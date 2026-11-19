//! New part… (the Parts tab's button, the palette): a window that makes a part of one's own: its
//! name and fields, its pins in a table (Fill makes N at once), its footprint picked from the
//! libraries or made from a package's numbers (`cuprita_core::newpart`), both drawn as they are
//! typed; Create puts it into the project's own library as an import does, and the Parts tab
//! shows it, ready to add.

use crate::tr::{t, tr, trf};
use crate::{App, theme, units};
use cuprita_core::board::{Board, Geom, Nm, Pad, PadShape, Pt, Shape, Text};
use cuprita_core::library::LibPart;
use cuprita_core::newpart::{self, Footprint, Kind, PRESETS, Package, Part, Pin, Side, TYPES};
use cuprita_core::schematic::{Ink, Project, Schematic};
use eframe::egui::{self, Color32, RichText, vec2};

/// The pins' electrical types as KiCad's symbol editor names them, in `TYPES`' order.
const KINDS: [&str; 12] = [
    t!("Input"),
    t!("Output"),
    t!("Bidirectional"),
    t!("Tri-state"),
    t!("Passive"),
    t!("Free"),
    t!("Unspecified"),
    t!("Power input"),
    t!("Power output"),
    t!("Open collector"),
    t!("Open emitter"),
    t!("Not connected"),
];

const SIDES: [(Side, &str); 4] =
    [(Side::Left, t!("Left")), (Side::Right, t!("Right")), (Side::Top, t!("Top")), (Side::Bottom, t!("Bottom"))];

/// The part being made.
pub(crate) struct Wizard {
    part: Part,
    /// Whether its footprint is made from `package` (and which preset it started from), else
    /// the libraries' `chosen` one, picked with the words `choosing` (while the list shows).
    make: bool,
    package: Package,
    preset: usize,
    chosen: String,
    choosing: Option<String>,
    /// How many pins Fill makes.
    count: usize,
    focus: bool,
    /// Create was clicked: the part is made once no window is over the Parts tab, whose field
    /// then takes the keys. Why the last one failed.
    create: bool,
    error: String,
    /// What the previews draw, while it stays the same: the symbol's library text and its
    /// sheet, the footprint's text (or why it can't be made) and its board.
    sheet: Option<(String, Option<Schematic>)>,
    board: Option<(String, Result<Board, String>)>,
}

impl Wizard {
    fn new() -> Self {
        let count = 8;
        let package = (PRESETS[0].1)(count);
        let part = Part {
            name: String::new(),
            reference: "U".into(),
            description: String::new(),
            datasheet: String::new(),
            pins: newpart::pins(count),
            footprint: Footprint::Made(package.clone()),
        };
        Wizard {
            part,
            make: true,
            package,
            preset: 0,
            chosen: String::new(),
            choosing: Some(String::new()),
            count,
            focus: true,
            create: false,
            error: String::new(),
            sheet: None,
            board: None,
        }
    }

    /// The part as chosen, its footprint made or the libraries'.
    fn part(&self) -> Part {
        let footprint = if self.make { Footprint::Made(self.package.clone()) } else { Footprint::Library(self.chosen.clone()) };
        Part { footprint, ..self.part.clone() }
    }

    /// Preset `k`'s numbers for the pins the part has (its exposed pad the last of them).
    fn preset(&mut self, k: usize) {
        let pins = self.part.pins.len();
        let exposed = (PRESETS[k].1)(pins).exposed.is_some() as usize;
        (self.preset, self.package) = (k, (PRESETS[k].1)(pins.saturating_sub(exposed)));
    }

    /// The previews brought up to the part as it is now (`footprints`: the libraries', for a
    /// picked one's file).
    fn draw_again(&mut self, footprints: &[LibPart]) {
        let mut part = self.part();
        if part.name.trim().is_empty() {
            part.name = "PART".into();
        }
        let library = part.library();
        if self.sheet.as_ref().is_none_or(|(was, _)| *was != library) {
            self.sheet = Some((library.clone(), sheet_of(part.name.trim(), &library)));
        }
        // The footprint drawn again once its numbers, or the one picked, change.
        let key = match &part.footprint {
            Footprint::Made(package) => format!("{package:?} {}", part.pads()),
            Footprint::Library(id) => id.clone(),
        };
        if self.board.as_ref().is_none_or(|(was, _)| *was != key) {
            let text = match &part.footprint {
                Footprint::Made(package) => package.footprint(part.pads()).map(|(_, text)| text),
                Footprint::Library(id) => match footprints.iter().find(|f| f.id() == *id) {
                    Some(f) => std::fs::read_to_string(&f.path).map_err(|e| format!("{}: {e}", f.path.display())),
                    None => Err(tr("Choose the part's footprint, or make one from a package").into()),
                },
            };
            let board = text.and_then(|text| {
                let mut board = Board::default();
                board.add_footprint(&text, "part", Pt::default(), 0.0).map(|_| board).map_err(|e| e.msg.to_string())
            });
            self.board = Some((key, board));
        }
    }
}

/// The symbol `name` of library text `library` as Cuprita draws it on a sheet: placed on one of
/// its own.
fn sheet_of(name: &str, library: &str) -> Option<Schematic> {
    let symbol = cuprita_core::library::symbol(library, name).ok()?;
    let [_, (root, sheet), _] = cuprita_core::new::files("part", 2, (10.0, 10.0), 0);
    let mut project = Project::load(&root, "", |_| Ok(sheet.clone())).ok()?;
    project.add_symbol(0, "part", &symbol, &[], Pt::default(), 0.0).ok()?;
    Some(project.pages.swap_remove(0).sheet)
}

impl App {
    /// New part…: the window, a part of 8 pins in a SOIC to start from.
    pub(crate) fn new_part(&mut self) {
        self.wizard = Some(Wizard::new());
    }

    /// The New part window, while it is open: Create puts the part into the project's library,
    /// Cancel or Esc closes it.
    pub(crate) fn wizard(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.wizard.take() else { return };
        // egui keeps a window shown last frame over the rest for this one too.
        if w.create && ctx.memory(|m| m.top_modal_layer()).is_some() {
            self.wizard = Some(w);
            return ctx.request_repaint();
        }
        if std::mem::take(&mut w.create) {
            let part = w.part();
            match self.put_parts(ctx, |dir, name| newpart::create(dir, name, &part)) {
                Ok(()) => return,
                Err(e) => w.error = e,
            }
        }
        let folder = self.folder();
        // KiCad's footprints are read once one of them is to be picked.
        let footprints = if w.make { &[][..] } else { self.parts.get_or_insert_with(|| crate::parts::footprints(folder.as_deref())) };
        w.draw_again(footprints);
        let (project, check) = (self.project(), w.part().check());
        // Esc leaves a field typed in first, and closes the window only then.
        let typing = ctx.memory(|m| m.focused()).is_some();
        let frame = egui::Frame::popup(&ctx.global_style()).fill(theme::PANEL).corner_radius(16).inner_margin(24);
        let modal = egui::Modal::new(egui::Id::unique("new part")).frame(frame).show(ctx, |ui| {
            ui.set_width(1080.0_f32.min(ctx.content_rect().width() - 80.0));
            ui.label(RichText::new(tr("New part")).family(theme::strong()).size(20.0).color(theme::TEXT));
            ui.add_space(12.0);
            // A window shorter than it (a laptop's) scrolls it, its buttons left in view.
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 220.0).show(ui, |ui| {
                fields(ui, &mut w);
                ui.add_space(12.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(470.0);
                        pins(ui, &mut w);
                    });
                    ui.add_space(20.0);
                    ui.vertical(|ui| {
                        previews(ui, &w, check.as_ref().err());
                        ui.add_space(10.0);
                        footprint(ui, &mut w, &mut self.parts, folder.as_deref());
                    });
                });
            });
            ui.add_space(14.0);
            let (note, color) = match (&check, w.error.is_empty()) {
                (Err(e), _) => (e.clone(), theme::WARN),
                (Ok(()), false) => (w.error.clone(), theme::FAIL),
                (Ok(()), true) => (trf("It goes into {0}'s own library (libraries/), which KiCad reads too.", &[&project]), theme::QUIET),
            };
            ui.add(egui::Label::new(RichText::new(note).color(color)).wrap());
            ui.add_space(8.0);
            let (mut create, mut cancel) = (false, false);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    create = ui.add_enabled(check.is_ok(), theme::primary(tr("Create part"))).clicked();
                    cancel = ui.button(tr("Cancel")).clicked();
                });
            });
            (create, cancel)
        });
        let (create, cancel) = modal.inner;
        let esc = !typing
            && modal.is_top_modal
            && !modal.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancel || esc {
            return;
        }
        if create {
            w.create = true;
            ctx.request_repaint();
        }
        self.wizard = Some(w);
    }
}

/// A small heading over a part of the window.
fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).family(theme::strong()).color(theme::QUIET));
    ui.add_space(4.0);
}

/// The part's name, reference prefix, description and datasheet.
fn fields(ui: &mut egui::Ui, w: &mut Wizard) {
    let p = &mut w.part;
    ui.horizontal(|ui| {
        let quiet = |ui: &mut egui::Ui, text| ui.label(RichText::new(text).color(theme::QUIET));
        quiet(ui, tr("Name"));
        let name = ui.add(egui::TextEdit::singleline(&mut p.name).hint_text("LM358").desired_width(150.0));
        if std::mem::take(&mut w.focus) {
            name.request_focus();
        }
        ui.add_space(8.0);
        quiet(ui, tr("Reference"));
        ui.add(egui::TextEdit::singleline(&mut p.reference).desired_width(36.0))
            .on_hover_text(tr("Its letters: U, J, D, Q…; KiCad numbers the parts after them"));
        ui.add_space(8.0);
        quiet(ui, tr("Description"));
        ui.add(egui::TextEdit::singleline(&mut p.description).hint_text(tr("Dual op-amp")).desired_width(300.0));
        ui.add_space(8.0);
        quiet(ui, tr("Datasheet"));
        ui.add(egui::TextEdit::singleline(&mut p.datasheet).hint_text("https://…").desired_width(f32::INFINITY));
    });
}

/// The pins' table: each one's number, name, electrical type and side, and Fill for N at once.
fn pins(ui: &mut egui::Ui, w: &mut Wizard) {
    heading(ui, tr("PINS"));
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut w.count).range(1..=500).speed(0.2));
        ui.label(RichText::new(tr("pins")).color(theme::QUIET));
        let fill =
            ui.button(tr("Fill")).on_hover_text(tr("A table of that many pins, numbered from 1, half on the left and half on the right"));
        if fill.clicked() {
            w.part.pins = newpart::pins(w.count);
            w.preset(w.preset);
        }
        if ui.button(tr("Add pin")).clicked() {
            let next = w.part.pins.iter().filter_map(|p| p.number.trim().parse::<usize>().ok()).max().unwrap_or(0) + 1;
            let side = w.part.pins.last().map_or(Side::Left, |p| p.side);
            w.part.pins.push(Pin { number: next.to_string(), name: String::new(), kind: "passive", side });
        }
    });
    ui.add_space(6.0);
    let mut drop = None;
    egui::ScrollArea::vertical().id_salt("pins").max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("pin table").num_columns(5).spacing(vec2(8.0, 4.0)).striped(true).show(ui, |ui| {
            for head in [tr("Number"), tr("Name"), tr("Type"), tr("Side"), ""] {
                ui.label(RichText::new(head).small().color(theme::QUIET));
            }
            ui.end_row();
            for (i, pin) in w.part.pins.iter_mut().enumerate() {
                // Sized, as a grid's column is as wide as its first frame's cells.
                let tall = ui.spacing().interact_size.y;
                ui.add_sized(vec2(44.0, tall), egui::TextEdit::singleline(&mut pin.number));
                ui.add_sized(vec2(130.0, tall), egui::TextEdit::singleline(&mut pin.name).hint_text(tr("name")))
                    .on_hover_text(tr("~{RESET} draws a bar over RESET"));
                let kind = TYPES.iter().position(|&k| k == pin.kind).unwrap_or(4);
                egui::ComboBox::from_id_salt(("pin type", i)).selected_text(tr(KINDS[kind])).width(130.0).height(400.0).show_ui(ui, |ui| {
                    for (k, name) in KINDS.iter().enumerate() {
                        ui.selectable_value(&mut pin.kind, TYPES[k], tr(name));
                    }
                });
                let side = SIDES.iter().find(|s| s.0 == pin.side).map_or("", |s| s.1);
                egui::ComboBox::from_id_salt(("pin side", i)).selected_text(tr(side)).width(100.0).show_ui(ui, |ui| {
                    for (s, name) in SIDES {
                        ui.selectable_value(&mut pin.side, s, tr(name));
                    }
                });
                if ui.small_button(crate::icon::CLOSE).on_hover_text(tr("Takes this pin away")).clicked() {
                    drop = Some(i);
                }
                ui.end_row();
            }
        });
    });
    if let Some(i) = drop {
        w.part.pins.remove(i);
    }
}

/// The footprint: made from a package (a preset's numbers, each one set), or one of the
/// libraries' picked as Inspect's chooser picks one.
fn footprint(ui: &mut egui::Ui, w: &mut Wizard, parts: &mut Option<Vec<LibPart>>, folder: Option<&std::path::Path>) {
    heading(ui, tr("FOOTPRINT"));
    ui.horizontal(|ui| {
        ui.radio_value(&mut w.make, true, tr("Make one from a package"));
        ui.radio_value(&mut w.make, false, tr("Pick one from the libraries"));
    });
    ui.add_space(6.0);
    if !w.make {
        let picked = crate::inspect::choose_footprint(ui, &w.chosen, "", &mut w.choosing, parts, folder);
        if let Some((_, id)) = picked {
            w.chosen = id;
        }
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (k, (name, _)) in PRESETS.iter().enumerate() {
            if ui.add(egui::Button::selectable(w.preset == k, tr(name))).clicked() {
                w.preset(k);
            }
        }
    });
    ui.add_space(6.0);
    let p = &mut w.package;
    let label = |ui: &mut egui::Ui, text: &str, hint: &str| {
        ui.label(RichText::new(text).color(theme::QUIET)).on_hover_text(hint);
    };
    let size = |ui: &mut egui::Ui, a: &mut Nm, b: &mut Nm| {
        length(ui, a);
        ui.label("×");
        length(ui, b);
    };
    ui.horizontal_wrapped(|ui| {
        label(ui, tr("Pitch"), tr("Pad centre to centre along a row"));
        length(ui, &mut p.pitch);
        if p.kind != Kind::Header || p.rows == 2 {
            ui.add_space(6.0);
            label(ui, tr("Span"), tr("Pad centre to centre across the package: from row to row, or side to side"));
            length(ui, &mut p.span);
        }
        ui.add_space(6.0);
        label(ui, tr("Pad"), tr("Its length out from the body, and its width along the row"));
        size(ui, &mut p.pad.0, &mut p.pad.1);
        if p.through() {
            ui.add_space(6.0);
            label(ui, tr("Drill"), tr("The hole through the pad"));
            length(ui, &mut p.drill);
        }
        ui.add_space(6.0);
        label(ui, tr("Body"), tr("Its width across the rows, and its length along them"));
        size(ui, &mut p.body.0, &mut p.body.1);
        ui.label(RichText::new(units::name()).color(theme::QUIET));
    });
    if !p.through() {
        ui.horizontal(|ui| {
            let mut on = p.exposed.is_some();
            let hint = tr("A pad under the body, as QFNs and DFNs have: the table's last pin");
            if ui.checkbox(&mut on, tr("Exposed pad")).on_hover_text(hint).changed() {
                p.exposed = on.then_some((p.body.0 / 2, p.body.1 / 2));
            }
            if let Some((a, b)) = &mut p.exposed {
                size(ui, a, b);
                ui.label(RichText::new(units::name()).color(theme::QUIET));
            }
        });
    }
    let pads = w.part().pads();
    ui.label(RichText::new(w.package.name(pads)).monospace().color(theme::TEXT_2));
}

/// A length dragged or typed in the units shown (Ctrl+U), or with its own (`0.65mm`, `25mil`).
fn length(ui: &mut egui::Ui, nm: &mut Nm) {
    let field = egui::DragValue::new(nm).speed(10_000.0).range(0..=100_000_000);
    ui.add(field.custom_formatter(|v, _| units::num(v)).custom_parser(|s| units::parse(s).map(f64::from)));
}

/// The symbol as it shows on a sheet, and the footprint as it shows on a board (else why not:
/// `wrong`, what is wrong with the part).
fn previews(ui: &mut egui::Ui, w: &Wizard, wrong: Option<&String>) {
    heading(ui, tr("PREVIEW"));
    let size = vec2(((ui.available_width() - 12.0) / 2.0).max(160.0), 240.0);
    ui.horizontal(|ui| {
        let ink = |layer: u8| Some(rgb(Ink::ALL.get(layer as usize)?.color()));
        match w.sheet.as_ref().and_then(|s| s.1.as_ref()) {
            Some(sheet) => picture(ui, size, rgb(0xF5F4EF), &sheet.shapes, &sheet.texts, &[], ink),
            None => note(ui, size, wrong.map_or("", String::as_str)),
        }
        ui.add_space(12.0);
        match w.board.as_ref().map(|b| &b.1) {
            Some(Ok(b)) => {
                let layer = |l: u8| match b.layers.get(l as usize).map(String::as_str) {
                    Some("F.SilkS") => Some(rgb(0xEEEEEE)),
                    Some("F.Fab") => Some(rgb(0x9AA0A6)),
                    Some("F.CrtYd") => Some(rgb(0xD04FD0)),
                    _ => None,
                };
                let copper =
                    |p: &Pad| if p.layers & b.copper != 0 { rgb(0xE05A47) } else { theme::tint(rgb(0xE05A47), rgb(0xBFA58A), 0.8) };
                // Its name, the value, shows below its numbers: the pads get the room.
                let value = b.footprints.first().map_or("", |f| f.value.as_str());
                let texts: Vec<Text> = b.texts.iter().filter(|t| t.text != value).cloned().collect();
                picture(ui, size, theme::CANVAS, &b.shapes, &texts, &b.pads.iter().map(|p| (p, copper(p))).collect::<Vec<_>>(), layer);
            }
            Some(Err(e)) => note(ui, size, e),
            None => {}
        }
    });
}

/// A box `size` big saying `text` where a picture would be.
fn note(ui: &mut egui::Ui, size: egui::Vec2, text: &str) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, 8.0, theme::CANVAS);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(16.0)).layout(egui::Layout::top_down(egui::Align::Center)));
    inner.add_space(size.y / 2.0 - 30.0);
    inner.add(egui::Label::new(RichText::new(text).color(theme::QUIET)).wrap());
}

/// A drawing fit into a box `size` big on `paper`: its pads (each in its colour), shapes and
/// texts, each in the colour `ink` gives its layer (none: not drawn), its lines as wide as theirs.
fn picture(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    paper: Color32,
    shapes: &[Shape],
    texts: &[Text],
    pads: &[(&Pad, Color32)],
    ink: impl Fn(u8) -> Option<Color32>,
) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, paper);
    // What is drawn's box: shapes' points, texts' boxes, pads' corners.
    let mut pts: Vec<Pt> = vec![];
    for s in shapes.iter().filter(|s| ink(s.layer).is_some()) {
        match &s.geom {
            Geom::Segment { a, b } => pts.extend([*a, *b]),
            Geom::Arc { start, mid, end } => pts.extend([*start, *mid, *end]),
            Geom::Circle { center: c, radius: r } => pts.extend([Pt { x: c.x - r, y: c.y - r }, Pt { x: c.x + r, y: c.y + r }]),
            Geom::Poly(p) => pts.extend(p),
        }
    }
    for t in texts.iter().filter(|t| ink(t.layer).is_some()) {
        let (lo, hi) = cuprita_core::font::bounds(t);
        pts.extend([lo, hi]);
    }
    for (p, _) in pads {
        let r = p.size.0.max(p.size.1) / 2;
        pts.extend([Pt { x: p.at.x - r, y: p.at.y - r }, Pt { x: p.at.x + r, y: p.at.y + r }]);
    }
    let Some(lo) = pts.iter().copied().reduce(|a, b| Pt { x: a.x.min(b.x), y: a.y.min(b.y) }) else { return };
    let hi = pts.iter().copied().fold(lo, |a, b| Pt { x: a.x.max(b.x), y: a.y.max(b.y) });
    let span = vec2((hi.x - lo.x).max(1) as f32, (hi.y - lo.y).max(1) as f32);
    let scale = ((rect.size() - vec2(28.0, 28.0)) / span).min_elem();
    let mid = (lo.x as f32 + span.x / 2.0, lo.y as f32 + span.y / 2.0);
    let at = |p: Pt| rect.center() + vec2(p.x as f32 - mid.0, p.y as f32 - mid.1) * scale;
    let pen = |w: Nm, c: Color32| egui::Stroke::new((w as f32 * scale).max(1.0), c);
    for (p, color) in pads {
        let (w, h) = if (p.angle / 90.0).round() as i64 % 2 == 0 { p.size } else { (p.size.1, p.size.0) };
        let r = egui::Rect::from_center_size(at(p.at), vec2(w as f32, h as f32) * scale);
        let round = match p.shape {
            PadShape::Circle | PadShape::Oval => r.width().min(r.height()) / 2.0,
            PadShape::RoundRect(k) => k * r.width().min(r.height()),
            _ => 0.0,
        };
        painter.rect_filled(r, round, *color);
        if let Some((d, _)) = p.drill {
            painter.circle_filled(at(p.at), d as f32 / 2.0 * scale, paper);
        }
    }
    for s in shapes {
        let Some(color) = ink(s.layer) else { continue };
        let stroke = pen(s.width, color);
        match &s.geom {
            Geom::Segment { a, b } => _ = painter.line_segment([at(*a), at(*b)], stroke),
            Geom::Arc { start, mid, end } => {
                _ = painter.line(cuprita_core::board::arc_points(*start, *mid, *end).into_iter().map(at).collect(), stroke)
            }
            Geom::Circle { center, radius } if s.filled => _ = painter.circle_filled(at(*center), *radius as f32 * scale, color),
            Geom::Circle { center, radius } => _ = painter.circle_stroke(at(*center), *radius as f32 * scale, stroke),
            Geom::Poly(p) if s.filled => _ = painter.add(egui::Shape::convex_polygon(p.iter().copied().map(at).collect(), color, stroke)),
            Geom::Poly(p) => _ = painter.add(egui::Shape::closed_line(p.iter().copied().map(at).collect(), stroke)),
        }
    }
    for t in texts {
        let Some(color) = ink(t.layer) else { continue };
        let stroke = pen(t.thickness, color);
        cuprita_core::font::strokes(t, |line| _ = painter.line(line.iter().copied().map(at).collect(), stroke));
    }
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
