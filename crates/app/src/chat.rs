//! Ask (`?`, or the button): a chat about the project with a local model in Ollama
//! (`crates/core/ollama.md`), which looks things up with the agents' commands that only look, on
//! a copy of the board and schematic as they are, edits not saved yet too, and in the project's
//! repo and its parts' datasheets (`files.rs`). Enter asks, Esc stops an answer. Desktop only.

pub(crate) mod files;

use crate::agent::{Session, functions};
use crate::dock::Tab;
use crate::tr::{t, tr, trf};
use crate::{App, focus, icon, theme};
use cuprita_core::json::json_str;
use cuprita_core::ollama::{self, Ollama};
use eframe::egui::{self, RichText};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// The agents' commands the model may call: those that only look.
const TOOLS: [&str; 7] = ["query", "find", "drc", "erc", "diff", "ir_drop", "simulate"];
/// How many times an answer may call tools before it must answer with what they gave.
const ROUNDS: usize = 8;
/// The most of a tool's result the model reads, in characters (~2,000 tokens): a big board's
/// DRC would fill the conversation.
const READ: usize = 8000;
pub(crate) const FIELD: &str = "chat question";
const CLOUD_ONLY: &str = t!(
    "Ollama's models here run in its cloud, which sends what you ask and the project's data to ollama.com: pick one above to use it, or pull one that runs on this computer (ollama pull gemma4:e4b)"
);
const NO_MODEL: &str =
    t!("Ollama has no model that can look things up: pull one, e.g. `ollama pull gemma4` (Ollama's app can too), then ask again");

/// The chat: the conversation, the question being typed and the answer coming.
#[derive(Default)]
pub(crate) struct Chat {
    /// The conversation as Ollama's messages, the model's instructions first.
    said: Vec<String>,
    /// What the panel shows of it.
    shown: Vec<Line>,
    question: String,
    /// The models Ollama has that can call tools, and the one that answers.
    models: Vec<String>,
    model: String,
    answer: Option<Answer>,
}

enum Line {
    You(String),
    Model(String),
    /// A tool the model called, with its arguments.
    Tool(String),
    Error(String),
}

/// An answer coming: what its thread hears, the flag that stops it, and whether the model is
/// thinking.
struct Answer {
    heard: Receiver<Heard>,
    stop: Arc<AtomicBool>,
    thinking: bool,
}

impl Drop for Answer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What an answer's thread hears, as it comes.
enum Heard {
    Models(Vec<String>, String),
    Thinking,
    Text(String),
    Tool(String),
    /// The conversation with the answer, or what went wrong.
    Done(Result<Vec<String>, String>),
}

impl Chat {
    /// Forgets the conversation, stopping an answer coming: New chat, or another project.
    pub(crate) fn forget(&mut self) {
        (self.said, self.shown, self.answer) = (vec![], vec![], None);
    }

    fn hear(&mut self, h: Heard) {
        let thinking = matches!(h, Heard::Thinking);
        self.answer.iter_mut().for_each(|a| a.thinking = thinking);
        match h {
            Heard::Models(models, model) => (self.models, self.model) = (models, model),
            Heard::Thinking => {}
            Heard::Text(t) => match self.shown.last_mut() {
                Some(Line::Model(s)) => s.push_str(&t),
                _ => self.shown.push(Line::Model(t)),
            },
            Heard::Tool(t) => self.shown.push(Line::Tool(t)),
            Heard::Done(done) => {
                self.answer = None;
                match done {
                    Ok(said) => self.said = said,
                    Err(e) => self.shown.push(Line::Error(e)),
                }
            }
        }
    }
}

impl App {
    /// `?`, or the button: the Ask tab, ready to type in, or back to Inspect.
    pub(crate) fn toggle_chat(&mut self, ctx: &egui::Context) {
        self.right.toggle(Tab::Ask, Tab::Inspect);
        if self.right.shows(Tab::Ask) {
            focus(ctx, FIELD, "?");
        }
    }

    /// Takes in what the model has said since the last frame, shown or not.
    pub(crate) fn chat_poll(&mut self) {
        let chat = &mut self.chat;
        while let Some(a) = &chat.answer {
            match a.heard.try_recv() {
                Ok(h) => chat.hear(h),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => chat.hear(Heard::Done(Err(tr("the answer stopped short: ask again").into()))),
            }
        }
    }

    /// The Ask tab: the conversation, the model that answers (to pick another), the question to
    /// type (Enter asks it, Shift+Enter starts a line); Stop (Esc) while it answers.
    pub(crate) fn ask_tab(&mut self, ui: &mut egui::Ui) {
        // The board's parts and nets an answer names are links that show them.
        let names: HashSet<&str> =
            self.board.footprints.iter().map(|f| f.reference.as_str()).chain(self.board.nets.iter().skip(1).map(String::as_str)).collect();
        let chat = &mut self.chat;
        let (mut ask, mut show) = (false, None);
        {
            ui.horizontal(|ui| {
                ui.strong(tr("Ask"));
                if chat.models.len() > 1 {
                    let pick = egui::ComboBox::from_id_salt("model").selected_text(chat.model.as_str()).show_ui(ui, |ui| {
                        for m in &chat.models {
                            ui.selectable_value(&mut chat.model, m.clone(), m);
                        }
                    });
                    pick.response.on_hover_text(tr("The model in Ollama that answers"));
                } else {
                    ui.label(&chat.model);
                }
                if ui.add_enabled(!chat.shown.is_empty(), egui::Button::new(tr("New chat"))).clicked() {
                    chat.forget();
                }
            });
            egui::Panel::bottom("question").show(ui, |ui| {
                if let Some(a) = &chat.answer {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(if a.thinking { tr("Thinking…") } else { "" });
                        if ui.button(tr("Stop (Esc)")).clicked()
                            || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                        {
                            a.stop.store(true, Ordering::Relaxed);
                        }
                    });
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let field = egui::TextEdit::multiline(&mut chat.question)
                        .id(egui::Id::unique(FIELD))
                        .hint_text(tr("What is U3? Any ERC errors? (Enter)"))
                        .desired_rows(2)
                        .desired_width(ui.available_width() - 40.0)
                        .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter));
                    ask = ui.add(field).has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                    let send = egui::Button::new(RichText::new(icon::SEND).color(theme::ON_COPPER)).fill(theme::COPPER).corner_radius(8);
                    ask |= ui
                        .add_enabled(chat.answer.is_none() && !chat.question.trim().is_empty(), send)
                        .on_hover_text(tr("Ask (Enter)"))
                        .clicked();
                });
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    let on = !chat.models.is_empty();
                    ui.painter().circle_filled(dot.center(), 3.0, if on { theme::PASS } else { theme::QUIET });
                    let model = if on { trf("{0} · on this computer", &[&chat.model]) } else { tr("Ollama, on this computer").into() };
                    ui.label(RichText::new(model).small().color(theme::QUIET));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(tr("It only looks")).small().color(theme::QUIET));
                    });
                });
            });
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
                if chat.shown.is_empty() {
                    ui.weak(tr("Ask about this project in plain words. A model on this computer (Ollama) answers, looking up the parts, nets, DRC, ERC and simulation of the board and schematic as they are now (edits not saved yet too), the project's files and its parts' datasheets."));
                }
                let mut k = 0;
                while let Some(line) = chat.shown.get(k) {
                    k += 1;
                    match line {
                        Line::You(t) => bubble(ui, t),
                        Line::Model(t) => {
                            if let Some(name) = linked(ui, t.trim(), &names) {
                                show = Some(name);
                            }
                        }
                        Line::Tool(t) => {
                            // A run of lookups shows as chips.
                            let mut run = vec![t.as_str()];
                            while let Some(Line::Tool(t)) = chat.shown.get(k) {
                                (run, k) = ([run, vec![t.as_str()]].concat(), k + 1);
                            }
                            chips(ui, &run);
                        }
                        Line::Error(t) => {
                            ui.colored_label(ui.visuals().error_fg_color, t);
                        }
                    }
                }
            });
        }
        if ask {
            self.ask(ui.ctx());
        }
        match show {
            Some(name) if self.sch_view() => _ = self.find_sch(&name),
            Some(name) => self.find(&name),
            None => {}
        }
    }

    /// Asks the question typed: the model answers in the background, looking things up on a copy
    /// of the board and schematic as they are now.
    fn ask(&mut self, ctx: &egui::Context) {
        let q = self.chat.question.trim().to_string();
        if q.is_empty() || self.chat.answer.is_some() {
            return;
        }
        // The open board's project, else the schematic's.
        let sheets = self.sch.as_ref().map(|s| &s.sheets);
        let path = self.path.clone().or_else(|| sheets.map(|s| s.root.with_extension("kicad_pcb")));
        let sch = sheets.filter(|s| Some(s.root.with_extension("kicad_pcb")) == path).cloned();
        let board = self.scene.is_some().then(|| self.board.clone());
        let chat = &mut self.chat;
        if chat.said.is_empty() {
            chat.said.push(ollama::message("system", &instructions(path.as_deref())));
        }
        let mut said = chat.said.clone();
        said.push(ollama::message("user", &q));
        chat.question.clear();
        chat.shown.push(Line::You(q));
        let (tell, heard) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (go, model, ctx, name) = (stop.clone(), chat.model.clone(), ctx.clone(), self.name.clone());
        std::thread::spawn(move || {
            let tell = |h| {
                let _ = tell.send(h);
                ctx.request_repaint();
            };
            let session = Session::of(path, name, board, sch);
            tell(Heard::Done(answer(&Ollama::default(), session, said, model, &go, &tell)));
        });
        chat.answer = Some(Answer { heard, stop, thinking: false });
    }
}

/// What the model is told first: what it answers about, and how.
fn instructions(board: Option<&Path>) -> String {
    let project = match board.and_then(|b| b.file_stem()) {
        Some(name) => format!("The project open is {}.", name.display()),
        None => "No project is open: the user opens one by dropping its .kicad_pcb or .kicad_pro on Cuprita's window.".into(),
    };
    format!(
        "You are the assistant in Cuprita, a PCB and schematic editor for KiCad projects. {project} Answer the user's questions about its board and schematic. Look things up with the tools, which see them as they are now (edits not saved yet too), rather than guessing, and say so when they can't tell: find tells what a part is (U17) and what a pin or net connects (U17 pin 15, GND), query bom lists the parts by value, erc and drc the design's problems, files and read what the project's documents and the parts' datasheets say. Be brief, in plain text without Markdown. Lengths are in mm."
    )
}

/// The thread of an answer to the conversation `said`: the model to ask (`model` if Ollama has
/// it, else its first), then its replies, running the tools it calls on `session`, until it
/// answers without calling one; the conversation with them, or what went wrong.
fn answer(
    ollama: &Ollama,
    mut session: Session,
    mut said: Vec<String>,
    model: String,
    stop: &AtomicBool,
    tell: &impl Fn(Heard),
) -> Result<Vec<String>, String> {
    // One in Ollama's cloud sends the project to ollama.com: only when it was picked.
    let (models, local) = ollama.models()?;
    let chosen = if models.contains(&model) { Some(model) } else { models[..local].first().cloned() };
    let none = if models.is_empty() { NO_MODEL } else { CLOUD_ONLY };
    tell(Heard::Models(models, chosen.clone().unwrap_or_default()));
    let model = chosen.ok_or(tr(none))?;
    // The agents' commands, then the repo's tools.
    let tools = format!("{},{}]", functions(&TOOLS).trim_end_matches(']'), files::TOOLS);
    for round in 0..=ROUNDS {
        // The last round offers none: the model answers with what the tools gave.
        let offered = if round < ROUNDS { tools.as_str() } else { "[]" };
        let reply = ollama
            .chat(&model, &said, offered, stop, |t, thinking| tell(if thinking { Heard::Thinking } else { Heard::Text(t.into()) }))?;
        said.push(reply.message());
        if reply.calls.is_empty() || stop.load(Ordering::Relaxed) {
            break;
        }
        for (name, args) in &reply.calls {
            let shown = args.to_json();
            tell(Heard::Tool(if shown == "{}" { name.clone() } else { format!("{name} {shown}") }));
            let out = match name.as_str() {
                "files" | "read" => files::run(name, args, session.path(), session.board()),
                n if TOOLS.contains(&n) => session.exec(name, args),
                _ => Err(format!("no tool {name}: call one of {}, files, read", TOOLS.join(", "))),
            };
            let out = out.unwrap_or_else(|e| format!(r#"{{"error":{}}}"#, json_str(&e)));
            said.push(ollama::tool_message(name, &cut(&out)));
        }
    }
    Ok(said)
}

/// The user's question, as a bubble on the right.
fn bubble(ui: &mut egui::Ui, text: &str) {
    ui.add_space(10.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        let corners = egui::CornerRadius { nw: 12, ne: 12, sw: 12, se: 4 };
        egui::Frame::new().fill(theme::RAISED).corner_radius(corners).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_max_width(ui.available_width() * 0.86);
            ui.add(egui::Label::new(RichText::new(text).color(theme::TEXT)).wrap());
        });
    });
    ui.add_space(6.0);
}

/// The lookups the model made in a row, as chips: each tool once, with how many times, and its
/// calls on hover.
fn chips(ui: &mut egui::Ui, run: &[&str]) {
    let mut tools: Vec<(&str, Vec<&str>)> = vec![];
    for call in run {
        let name = call.split(' ').next().unwrap_or(call);
        match tools.iter_mut().find(|(n, _)| *n == name) {
            Some((_, calls)) => calls.push(call),
            None => tools.push((name, vec![call])),
        }
    }
    ui.horizontal_wrapped(|ui| {
        for (name, calls) in tools {
            let text = if calls.len() > 1 { format!("{name} ×{}", calls.len()) } else { name.to_string() };
            let chip = egui::Frame::new()
                .fill(theme::RAISED)
                .stroke((1.0, theme::LINE))
                .corner_radius(255)
                .inner_margin(egui::Margin::symmetric(8, 2));
            chip.show(ui, |ui| ui.label(RichText::new(text).monospace().size(11.0).color(theme::TEXT_2)))
                .response
                .on_hover_text(calls.join("\n"));
        }
    });
}

/// `text` as the model wrote it, the words naming a part or net of the board in `names` as
/// links; the name clicked.
fn linked(ui: &mut egui::Ui, text: &str, names: &HashSet<&str>) -> Option<String> {
    let mut clicked = None;
    for line in text.lines() {
        ui.horizontal_wrapped(|ui| {
            // Each word whole: the row breaks between words.
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            for word in line.split_inclusive(' ') {
                let name = word.trim_end().trim_matches(|c: char| ",.:;()`'\"*".contains(c));
                match word.find(name).filter(|_| !name.is_empty() && names.contains(name)) {
                    Some(at) => {
                        ui.label(&word[..at]);
                        if ui.link(name).on_hover_text(tr("Show it")).clicked() {
                            clicked = Some(name.to_string());
                        }
                        ui.label(&word[at + name.len()..]);
                    }
                    None => _ = ui.label(word),
                }
            }
        });
    }
    clicked
}

/// `text`, cut to what the model reads of a tool's result, saying how much is left out.
fn cut(text: &str) -> String {
    match text.char_indices().nth(READ) {
        Some((i, _)) => format!("{}… (cut, {} more characters: ask for less, e.g. with filter)", &text[..i], text[i..].chars().count()),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tests::test_session;
    use cuprita_core::json::Val;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// An Ollama that answers each request with the next of `replies` (NDJSON bodies); gives
    /// its host and the bodies of the requests.
    fn ollama(replies: &'static [&'static str]) -> (Ollama, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = l.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let mut bodies = vec![];
            for reply in replies {
                let (mut s, _) = l.accept().unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let (mut h, mut len) = (String::new(), 0);
                while r.read_line(&mut h).unwrap() > 2 {
                    len = h.to_ascii_lowercase().strip_prefix("content-length:").map_or(len, |n| n.trim().parse().unwrap());
                    h.clear();
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                bodies.push(String::from_utf8(body).unwrap());
                s.write_all(format!("HTTP/1.0 200 OK\r\n\r\n{reply}").as_bytes()).unwrap();
            }
            bodies
        });
        (Ollama { host }, server)
    }

    #[test]
    fn answers_with_the_tools_it_calls() {
        let (o, server) = ollama(&[
            r#"{"models":[{"name":"gemma4:e2b","capabilities":["tools"]}]}"#,
            concat!(
                r#"{"message":{"content":"","tool_calls":[{"function":{"name":"find","arguments":{"query":"R1"}}}]}}"#,
                "\n",
                r#"{"message":{"content":"","tool_calls":[{"function":{"name":"route","arguments":{}}}]}}"#,
            ),
            r#"{"message":{"content":"R1 is a 10k resistor."}}"#,
        ]);
        let heard = std::sync::Mutex::new(vec![]);
        let said = vec![ollama::message("system", &instructions(Some(Path::new("/p/t.kicad_pcb")))), ollama::message("user", "R1?")];
        let tell = |h| heard.lock().unwrap().push(h);
        let said = answer(&o, test_session(), said, String::new(), &AtomicBool::new(false), &tell).unwrap();
        let said: Vec<Val> = said.iter().map(|m| Val::parse(m).unwrap()).collect();
        // The model's call, what find gave, the call it may not make, and the answer.
        assert_eq!(said.len(), 6);
        assert_eq!((said[3].str("tool_name"), said[4].str("tool_name")), (Some("find"), Some("route")));
        assert_eq!(Val::parse(said[3].str("content").unwrap()).unwrap().str("reference"), Some("R1"));
        assert!(said[4].str("content").unwrap().contains("no tool route: call one of query, find"));
        assert_eq!(said[5].str("content"), Some("R1 is a 10k resistor."));
        let heard = heard.into_inner().unwrap();
        assert!(matches!(&heard[0], Heard::Models(m, chosen) if m == &["gemma4:e2b"] && chosen == "gemma4:e2b"));
        assert!(matches!(&heard[1], Heard::Tool(t) if t == r#"find {"query":"R1"}"#));
        // Each request offers the tools that only look, and sends the conversation so far.
        let bodies = server.join().unwrap();
        let chat = Val::parse(&bodies[1]).unwrap();
        let tools: Vec<_> = chat.arr("tools").unwrap().iter().filter_map(|t| t.at(&["function", "name"])).collect();
        assert_eq!(tools.len(), TOOLS.len() + 2);
        assert_eq!(tools[TOOLS.len()..], [&Val::Str("files".into()), &Val::Str("read".into())]);
        assert!(chat.arr("messages").unwrap()[0].str("content").unwrap().contains("The project open is t."));
        assert_eq!(Val::parse(&bodies[2]).unwrap().arr("messages").unwrap().len(), 5);
    }

    #[test]
    fn says_how_much_of_a_result_is_cut() {
        assert_eq!(cut("{}"), "{}");
        let long = "é".repeat(READ + 5);
        assert!(cut(&long).ends_with("… (cut, 5 more characters: ask for less, e.g. with filter)"));
    }
}
