//! The board's own texts and drawings in the window (`cuprita_core::drawing`). The Text tool
//! (Shift+T): type the words, Enter puts them in the middle of the view, picked so a drag moves
//! them. The Line tool (`L`, the zone tool `lining`): click corners or drag a rectangle. Both
//! draw on the silkscreen of the side shown, or the layer chosen in their hint. Picked, a text
//! or drawing shows its settings in Inspect, each change an undo step.

use crate::inspect::field_row;
use crate::tr::{t, tr, trf};
use crate::{App, focus, pick_name, theme, units};
use cuprita_core::board::{Geom, LayerId, Nm, Shape, Text};
use cuprita_core::history::Pick;
use eframe::egui::{self, RichText};
use web_time::Instant; // std's panics in the browser

/// The Text tool's field.
pub(crate) const TEXT_FIELD: &str = "board text";

/// The Line tool's keys and clicks.
pub(crate) const LINE_HINT: &str = t!("Click its corners or drag a rectangle · Enter ends it · Backspace undoes a corner · Esc drops it");
/// Every key of it, in full: the hint's hover.
pub(crate) const LINE_KEYS: &str = t!(
    "Click the corners of a line, on the grid (Ctrl frees them, Shift their angle), or drag a rectangle · Enter or double click ends it · Backspace drops the last corner · Esc drops it, again leaves the tool"
);

/// What a drawing is, as Inspect and the status bar name it.
pub(crate) fn kind(s: &Shape) -> &'static str {
    match s.geom {
        Geom::Segment { .. } => t!("Line"),
        Geom::Arc { .. } => t!("Arc"),
        Geom::Circle { .. } => t!("Circle"),
        Geom::Poly(_) => t!("Polygon"),
    }
}

/// A length of a text.
type Length = fn(&mut Text) -> &mut Nm;

/// The lengths of a text Inspect sets: each one's name, where the text keeps it, and its edit as
/// the undo step names it.
const LENGTHS: [(&str, Length, &str); 3] = [
    (t!("Height"), |t| &mut t.size.1, t!("{0} high")),
    (t!("Width"), |t| &mut t.size.0, t!("letters {0} wide")),
    (t!("Pen"), |t| &mut t.thickness, t!("pen {0}")),
];

/// A text's words in a field of one line: `\n` where a line breaks.
fn one_line(words: &str) -> String {
    words.replace('\n', "\\n")
}

/// A length typed in the units shown or with its own, more than 0; else why not.
fn length(what: &str, text: &str) -> Result<Nm, String> {
    let v = units::parse(text).ok_or_else(|| {
        trf("{0} {1} isn't a length: type one in {2}, or as 0.2mm or 8mil", &[&what, &format!("{text:?}"), &units::name()])
    })?;
    if v > 0 { Ok(v) } else { Err(trf("{0} must be more than 0: type a length", &[&what])) }
}

impl App {
    /// The layer the Line and Text tools draw on: the one chosen, else the silkscreen of the side
    /// shown, else the first a drawing may go on; `None` on a board without one.
    pub(crate) fn ink_layer(&self) -> Option<LayerId> {
        let (b, layers) = (&self.board, self.board.drawing_layers());
        let silk = if self.flipped { "B.SilkS" } else { "F.SilkS" };
        let chosen = self.ink.filter(|l| layers.contains(l));
        chosen.or_else(|| layers.iter().copied().find(|&l| b.layers[l as usize] == silk)).or(layers.first().copied())
    }

    /// A chooser of the layers a text or drawing may go on, `now` chosen; returns another clicked.
    pub(crate) fn ink_choice(&self, ui: &mut egui::Ui, id: &str, now: LayerId) -> Option<LayerId> {
        let (b, mut chosen) = (&self.board, None);
        let combo = egui::ComboBox::from_id_salt(id).selected_text(b.layers[now as usize].as_str()).show_ui(ui, |ui| {
            for l in b.drawing_layers() {
                if ui.selectable_label(l == now, b.layers[l as usize].as_str()).clicked() && l != now {
                    chosen = Some(l);
                }
            }
        });
        combo.response.on_hover_text(tr("The layer it is on: a back layer mirrors a text, to read from that side"));
        chosen
    }

    /// Turns the Text tool on (Shift+T or its button: a field to type the words in), or off.
    pub(crate) fn text_tool(&mut self, ctx: &egui::Context) {
        let on = self.writing.is_some();
        self.select_tool();
        if !on {
            self.writing = Some(String::new());
            focus(ctx, TEXT_FIELD, "t");
        }
    }

    /// The Text tool's field: the words (`\n` breaks a line) and the layer they go on; Enter
    /// adds them.
    pub(crate) fn text_field(&mut self, ui: &mut egui::Ui) {
        let (Some(mut words), Some(layer)) = (self.writing.take(), self.ink_layer()) else {
            return self.status = tr("The board has no layer to write on: add a silkscreen layer in KiCad").into();
        };
        let field = egui::TextEdit::singleline(&mut words).id(egui::Id::unique(TEXT_FIELD)).hint_text(tr("Words, then Enter"));
        let field = ui.add(field.desired_width(180.0)).on_hover_text(tr("What the text says; \\n breaks a line"));
        // The hint's first showing only sizes it, which drops the focus the tool asked for: ask
        // again once it shows.
        let refocus = egui::Id::unique((TEXT_FIELD, "again"));
        if ui.is_sizing_pass() {
            ui.data_mut(|d| d.insert_temp(refocus, true));
        } else if ui.data_mut(|d| d.remove_temp::<bool>(refocus)).is_some() {
            field.request_focus();
        }
        if let Some(l) = self.ink_choice(ui, "text layer", layer) {
            self.ink = Some(l);
            field.request_focus();
        }
        match (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)), words.trim()) {
            (true, "") => field.request_focus(),
            (true, typed) => return self.add_text(&typed.replace("\\n", "\n"), self.ink_layer().unwrap_or(layer)),
            _ => {}
        }
        self.writing = Some(words);
    }

    /// Adds `words` on `layer` at the grid point in the middle of the view, picked so a drag
    /// moves them and R turns them; the tool stops.
    fn add_text(&mut self, words: &str, layer: LayerId) {
        let Some(scene) = &self.scene else { return };
        let (t, at) = (Instant::now(), self.on_grid(scene.nm(self.center)));
        let (pick, undo) = self.board.add_text(self.board.new_text(words, at, layer));
        self.select_tool();
        (self.selected, self.picked) = (Some(pick), Vec::new());
        self.edited(undo, t, trf("{0} added", &[&pick_name(&self.board, pick)]));
    }

    /// Text `i` picked: the board's own with its words, layer, size and pen, each set in place; a
    /// part's (its reference, its value) with its size and pen.
    pub(crate) fn text_settings(&mut self, ui: &mut egui::Ui, i: usize) {
        let (key, was) = (Pick::Text(i), self.board.texts[i].clone());
        let mut typing = self.typing.take().filter(|(k, ..)| *k == key).map(|(_, name, text)| (name, text));
        let mut change: Option<Result<(Text, String), String>> = None;
        if let Some(f) = self.board.text_part(i) {
            // shortcut: a part's text keeps its words (the schematic's: F8 writes them) and its
            // layer, neither of which saving writes for it; set them here once it does.
            crate::inspect::row(ui, tr("Text"), one_line(&was.text));
            crate::inspect::row(ui, tr("Part"), self.board.footprints[f].reference.clone());
        } else {
            if let Some(text) = field_row(ui, tr("Text"), &one_line(&was.text), &mut typing) {
                change = Some(match text.trim() {
                    "" => Err(tr("A text needs words: Delete takes it away").into()),
                    words => Ok((Text { text: words.replace("\\n", "\n"), ..was.clone() }, trf("reads {0}", &[&words]))),
                });
            }
            ui.label(RichText::new(tr("Layer")).small().color(theme::QUIET));
            if let Some(l) = self.ink_choice(ui, "text on", was.layer) {
                // On the other side it reads from there, as KiCad flips it.
                let (name, back) = (&self.board.layers[l as usize], |l: LayerId| self.board.layers[l as usize].starts_with("B."));
                let mirror = if back(l) == back(was.layer) { was.mirror } else { !was.mirror };
                change = Some(Ok((Text { layer: l, mirror, ..was.clone() }, trf("put on {0}", &[name]))));
            }
        }
        for (what, of, step) in LENGTHS {
            let (what, mut to) = (tr(what), was.clone());
            let Some(text) = field_row(ui, what, &units::len(*of(&mut to)), &mut typing) else { continue };
            change = Some(length(what, &text).map(|v| {
                *of(&mut to) = v;
                (to, trf(step, &[&units::len(v)]))
            }));
        }
        crate::inspect::row(ui, tr("Turned"), format!("{}°", (was.angle * 10.0).round() / 10.0));
        if was.rough {
            let note = tr("In a face of its own or knocked out in KiCad: drawn here in plain strokes, it keeps its face in the file.");
            ui.label(RichText::new(note).small().color(theme::QUIET));
        }
        self.typing = typing.map(|(name, text)| (key, name, text));
        match change {
            Some(Ok((to, what))) => {
                let (t, undo) = (Instant::now(), self.board.set_text(i, to));
                self.edited(undo, t, format!("{}: {what}", pick_name(&self.board, key)));
            }
            Some(Err(why)) => self.refused = Some((key, why)),
            None => {}
        }
    }

    /// The board's own drawing `i` picked: its layer and its line's width, each set in place.
    pub(crate) fn shape_settings(&mut self, ui: &mut egui::Ui, i: usize) {
        let (key, was) = (Pick::Shape(i), self.board.shapes[i].clone());
        let mut typing = self.typing.take().filter(|(k, ..)| *k == key).map(|(_, name, text)| (name, text));
        let mut change = None;
        ui.label(RichText::new(tr("Layer")).small().color(theme::QUIET));
        if let Some(l) = self.ink_choice(ui, "drawing on", was.layer) {
            change = Some(Ok((Shape { layer: l, ..was.clone() }, trf("put on {0}", &[&self.board.layers[l as usize]]))));
        }
        if let Some(text) = field_row(ui, tr("Width"), &units::len(was.width), &mut typing) {
            change = Some(length(tr("Width"), &text).map(|width| (Shape { width, ..was.clone() }, trf("{0} wide", &[&units::len(width)]))));
        }
        self.typing = typing.map(|(name, text)| (key, name, text));
        match change {
            Some(Ok((to, what))) => {
                let (t, undo) = (Instant::now(), self.board.set_shape(i, to));
                self.edited(undo, t, format!("{}: {what}", pick_name(&self.board, key)));
            }
            Some(Err(why)) => self.refused = Some((key, why)),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_shows_in_one_line_and_a_length_must_be_one() {
        assert_eq!((one_line("J1\n3V3"), one_line("J1\n3V3").replace("\\n", "\n")), ("J1\\n3V3".to_string(), "J1\n3V3".to_string()));
        assert_eq!((length("Height", "1.5mm"), length("Height", "40mil")), (Ok(1_500_000), Ok(1_016_000)));
        assert!(length("Height", "0").is_err() && length("Pen", "thin").unwrap_err().contains("\"thin\""));
    }
}
