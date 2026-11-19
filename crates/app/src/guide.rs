//! First steps: the way from an empty schematic to the fab's files, each step ticked off from
//! the project as it stands, the next one saying what to do, with its key. A project made with
//! New project opens with it; Ctrl+K › First steps shows or hides it. It floats at the top right
//! of the canvas, folded to its next step or open on all of them. On an empty project it offers
//! the walk-through: the same way with a first board's parts named, an LED lit by a coin cell.

use crate::cmd::Cmd;
use crate::shell::Bench;
use crate::tr::{t, tr};
use crate::{App, icon, theme};
use cuprita_core::board::{Board, Pt};
use cuprita_core::schematic::Symbol;
use eframe::egui::{self, Align, Layout, RichText, vec2};

/// A step: what it is, how to do it, the command that starts it (in the workbench it needs), and
/// whether the project has it done.
struct Step {
    title: &'static str,
    how: &'static str,
    cmd: Option<Cmd>,
    done: bool,
}

/// What the steps are checked by: the board, the schematic's parts (on the board's side),
/// whether its pins are all wired (ERC), what is left to route, the board's rule problems, and
/// whether the fab files were written; in the walk-through, whether its loop lights the LED.
struct Progress<'a> {
    board: &'a Board,
    parts: Vec<&'a Symbol>,
    wired: bool,
    unrouted: usize,
    problems: usize,
    exported: bool,
    lit: Option<bool>,
}

/// The walk-through's parts as KiCad's Device library names them: a coin cell (its + is pin 1),
/// a resistor and an LED (its anode is pin 2).
/// The panel's width, and its margin: inside it and from the canvas's edges.
const WIDTH: f32 = 300.0;
const MARGIN: f32 = 12.0;

/// How much of the canvas's top First steps takes, from its right edge, a margin left of it
/// too: what floats at the top keeps clear of it.
pub(crate) const TAKES: f32 = WIDTH + 4.0 * MARGIN;

const CELLS: [&str; 2] = ["Device:Battery_Cell", "Device:Battery"];
const RESISTORS: [&str; 4] = ["Device:R", "Device:R_Small", "Device:R_US", "Device:R_Small_US"];
const LEDS: [&str; 4] = ["Device:LED", "Device:LED_Small", "Device:LED_Filled", "Device:LED_Small_Filled"];

/// The walk-through's cell, resistor and LED among `parts`, the first of each.
fn tour_parts<'a>(parts: &[&'a Symbol]) -> [Option<&'a Symbol>; 3] {
    [&CELLS[..], &RESISTORS, &LEDS].map(|ids| parts.iter().find(|s| ids.contains(&s.lib_id.as_str())).copied())
}

/// Whether `nets` (a netlist: names and their parts' pins) wire `cell`, the resistor `r` and
/// `led` in a loop from the cell's + back to its −: the LED's anode toward the + (`Some(true)`,
/// it lights) or its cathode (`Some(false)`); `None` while the loop isn't closed.
fn lit(nets: &[(String, Vec<(String, String)>)], cell: &str, r: &str, led: &str) -> Option<bool> {
    let net = |part: &str, pin: &str| nets.iter().position(|(_, on)| on.iter().any(|(p, n)| p == part && n == pin));
    let (mut at, mut passed, mut forward) = (net(cell, "1")?, vec![], None);
    for _ in 0..2 {
        let (part, pin) = nets[at].1.iter().find(|(p, _)| (p == r || p == led) && !passed.contains(p))?;
        if part == led {
            forward = Some(pin == "2");
        }
        passed.push(part.clone());
        at = net(part, if pin == "1" { "2" } else { "1" })?;
    }
    nets[at].1.iter().any(|(p, n)| p == cell && n == "2").then_some(forward?)
}

/// The steps, in the order of the work, each done or not as the project stands.
fn steps(p: &Progress) -> Vec<Step> {
    let (b, parts) = (p.board, &p.parts);
    let on_board = |d: &str| b.footprints.iter().any(|f| f.reference == d);
    let inside = |p: Pt, (a, z): (Pt, Pt)| (a.x..=z.x).contains(&p.x) && (a.y..=z.y).contains(&p.y);
    let outline = b.outline_box();
    let step = |title, how, cmd, done| Step { title, how, cmd, done };
    vec![
        step(t!("Add the parts"), t!("A finds a part by name or part number; a click puts it down"), Some(Cmd::Add), !parts.is_empty()),
        step(
            t!("Wire their pins"),
            t!("W, then click from pin to pin; L names a net instead"),
            Some(Cmd::Wire),
            !parts.is_empty() && p.wired,
        ),
        step(
            t!("Give each part a footprint"),
            t!("Click a part, then Footprint › Choose… in Inspect"),
            None,
            !parts.is_empty() && parts.iter().all(|s| !s.footprint.is_empty()),
        ),
        step(
            t!("Bring them onto the board"),
            t!("F8 puts the schematic's parts and nets on the board"),
            Some(Cmd::Update),
            !parts.is_empty() && parts.iter().all(|s| on_board(&s.designator)),
        ),
        step(t!("Draw the board's outline"), t!("O, then drag a rectangle where the board goes"), Some(Cmd::Outline), outline.is_some()),
        step(
            t!("Place the parts inside it"),
            t!("Click a part, then drag it into place; R turns it"),
            None,
            !b.footprints.is_empty() && outline.is_some_and(|o| b.footprints.iter().all(|f| inside(f.at, o))),
        ),
        step(
            t!("Route every connection"),
            t!("X, then click a pad and follow its thin line to the next"),
            Some(Cmd::Route),
            !b.footprints.is_empty() && p.unrouted == 0,
        ),
        step(
            t!("Pass the design rules"),
            t!("D lists what is wrong; hover one for its fix"),
            Some(Cmd::Checks),
            !b.footprints.is_empty() && p.problems == 0,
        ),
        step(t!("Export the fab files"), t!("Make has them and the fabs to order from"), Some(Cmd::Make), p.exported),
    ]
}

/// The walk-through's steps: its three parts added, wired, valued and given footprints, then
/// the way on from F8 as `steps` has it.
fn tour(p: &Progress) -> Vec<Step> {
    let [cell, r, led] = tour_parts(&p.parts);
    let all = [cell, r, led].iter().all(Option::is_some);
    let step = |title, how, cmd, done| Step { title, how, cmd, done };
    let wire = if p.lit == Some(false) {
        t!("The LED is the wrong way round: its A (anode) faces the cell's −. Delete its wires and wire it the other way")
    } else {
        t!("Zoom in on them (the wheel), then W from the cell's + to the resistor, the resistor to the LED's A, its K to the cell's −")
    };
    let mut steps = vec![
        step(t!("Add a coin cell"), t!("A, type battery cell, Enter, then a click puts it on the sheet"), Some(Cmd::Add), cell.is_some()),
        step(t!("Add a resistor"), t!("A, type R, Enter, and put it beside the cell"), Some(Cmd::Add), r.is_some()),
        step(t!("Add an LED"), t!("A, type LED, Enter, and put it beside the resistor"), Some(Cmd::Add), led.is_some()),
        step(t!("Wire them in a loop"), wire, Some(Cmd::Wire), all && p.lit == Some(true)),
        step(
            t!("Make the resistor 120 Ω"),
            t!("Click it and type 120 in Value: from a 3 V cell a red LED then draws about 10 mA"),
            None,
            r.is_some_and(|r| !r.value.is_empty() && r.value != "R"),
        ),
        step(
            t!("Give each part a footprint"),
            t!("Click each, then Footprint › Choose…: 3034 finds the cell's holder, 0805 HandSolder the others"),
            None,
            all && [cell, r, led].iter().flatten().all(|s| !s.footprint.is_empty()),
        ),
    ];
    // From "Bring them onto the board" on, as for any project.
    steps.extend(self::steps(p).into_iter().skip(3));
    steps
}

impl App {
    /// First steps as the walk-through (Ctrl+K › First board, or the guide's offer), in the
    /// schematic where it starts; from Home, in a new project.
    pub(crate) fn start_tour(&mut self, ctx: &egui::Context) {
        if self.bench == Bench::Home {
            return self.start_new(ctx, true);
        }
        (self.guide, self.tour) = (Some(true), true);
        if !matches!(self.bench, Bench::Schematic | Bench::Board) {
            self.go(Bench::Schematic);
        }
    }

    /// The project as the steps check it.
    fn progress(&self) -> Progress<'_> {
        let sch = self.sch.as_ref();
        // A symbol in hand isn't a part yet: counted, the steps done would flicker as it comes and goes.
        let held = sch.and_then(|s| s.in_hand());
        let parts: Vec<&Symbol> = (sch.iter().flat_map(|s| s.sheets.project.pages.iter().flat_map(|p| &p.sheet.symbols)))
            .filter(|s| s.on_board() && Some(s.designator.as_str()) != held)
            .collect();
        // The loop's netlist only in the walk-through, once its parts are there (a small project).
        let lit = match (self.tour, sch, tour_parts(&parts)) {
            (true, Some(s), [Some(c), Some(r), Some(l)]) => lit(&s.sheets.project.netlist(), &c.designator, &r.designator, &l.designator),
            _ => None,
        };
        Progress {
            board: &self.board,
            parts,
            wired: sch.is_some_and(|s| !s.violations.iter().any(|v| v.kind == "pin_not_connected")),
            unrouted: self.ratsnest.len(),
            problems: self.broken(),
            exported: self
                .path
                .as_ref()
                .is_some_and(|p| std::path::Path::new(&format!("{}-gerbers.zip", p.with_extension("").display())).exists()),
            lit,
        }
    }

    /// The guide, while it is on, at the top right of the canvas `rect`: how many steps are done
    /// and the next one, or (open) every step.
    pub(crate) fn guide(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let (Some(open), Bench::Schematic | Bench::Board) = (self.guide, self.bench) else { return };
        if self.sch.is_none() && self.path.is_some() {
            self.board_sch(); // the schematic's steps count too
        }
        let progress = self.progress();
        let steps = if self.tour { tour(&progress) } else { steps(&progress) };
        let done = steps.iter().filter(|s| s.done).count();
        let next = steps.iter().position(|s| !s.done);
        // Offered on an empty project, and on one with only its parts (it was left, or the app closed).
        let ours = |s: &&Symbol| [&CELLS[..], &RESISTORS, &LEDS].iter().any(|ids| ids.contains(&s.lib_id.as_str()));
        let offer = !self.tour && progress.parts.iter().all(ours);
        let (mut run, mut close, mut fold, mut tour) = (None, false, false, self.tour);
        let area = egui::Area::new(egui::Id::unique("guide")).order(egui::Order::Foreground).fade_in(false);
        area.fixed_pos(rect.right_top() + vec2(-MARGIN, MARGIN)).pivot(egui::Align2::RIGHT_TOP).show(ctx, |ui| {
            crate::shell::floating().corner_radius(12).inner_margin(MARGIN).show(ui, |ui| {
                ui.set_width(WIDTH);
                ui.horizontal(|ui| {
                    crate::make::ring(ui, done, steps.len(), 36.0);
                    ui.vertical(|ui| {
                        let title = if self.tour { tr("Your first board") } else { tr("First steps") };
                        ui.label(RichText::new(title).family(theme::strong()).color(theme::TEXT));
                        let what = next.map_or(tr("Done: the board is ready to order"), |k| tr(steps[k].title));
                        ui.label(RichText::new(what).size(12.0).color(theme::TEXT_2));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        let glyph = |ui: &mut egui::Ui, icon: &str, tip: &str| {
                            ui.add(egui::Button::new(RichText::new(icon).color(theme::QUIET)).frame(false)).on_hover_text(tip).clicked()
                        };
                        close = glyph(ui, icon::CLOSE, tr("Close: Ctrl+K › First steps shows it again"));
                        fold = glyph(
                            ui,
                            if open { icon::REFOLD } else { icon::UNFOLD },
                            if open { tr("Only the next step") } else { tr("Every step") },
                        );
                    });
                });
                for (k, s) in steps.iter().enumerate().filter(|&(k, _)| open || Some(k) == next) {
                    ui.add_space(6.0);
                    let color = if s.done {
                        theme::PASS
                    } else if Some(k) == next {
                        theme::COPPER
                    } else {
                        theme::QUIET
                    };
                    ui.horizontal(|ui| {
                        let mark = if s.done { icon::PASSES } else { "○" };
                        ui.label(RichText::new(mark).color(color));
                        let title =
                            RichText::new(format!("{}. {}", k + 1, tr(s.title))).color(if s.done { theme::QUIET } else { theme::TEXT });
                        ui.label(if Some(k) == next { title.family(theme::strong()) } else { title });
                    });
                    if Some(k) == next {
                        ui.add(egui::Label::new(RichText::new(tr(s.how)).size(12.0).color(theme::TEXT_2)).wrap());
                        if let Some(cmd) = s.cmd {
                            let about = cmd.about();
                            let key = about.keys().next().map(|k| format!("   {}", crate::cmd::key_text(ui.ctx(), &k))).unwrap_or_default();
                            let button =
                                egui::Button::new(RichText::new(format!("{}  {}{key}", about.icon, about.name)).color(theme::ON_COPPER));
                            if ui.add(button.fill(theme::COPPER).corner_radius(8)).clicked() {
                                run = Some(cmd);
                            }
                        }
                    }
                }
                let link = |ui: &mut egui::Ui, text: &str| {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::SIGNAL)).sense(egui::Sense::click())).clicked()
                };
                if offer && link(ui, tr("New to boards? Light an LED from a coin cell, step by step ›")) {
                    tour = true;
                } else if self.tour && open && link(ui, tr("Leave the walk-through")) {
                    tour = false;
                }
            });
        });
        // Closing hides it, the walk-through kept for when it shows again; leaving ends it.
        if close {
            self.guide = None;
        } else if fold {
            self.guide = Some(!open);
        }
        if tour && !self.tour {
            self.start_tour(ctx);
        }
        self.tour = tour;
        if let Some(cmd) = run {
            // The schematic's steps run there; F8 opens the board itself.
            if matches!(cmd, Cmd::Add | Cmd::Wire) && !self.sch_view() {
                self.run(ctx, Cmd::Schematic);
            }
            if !self.on(cmd) {
                self.run(ctx, cmd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_off_the_steps_the_project_has_done() {
        let board = |at: &str| {
            Board::parse(&format!(
                r#"(kicad_pcb (layers (0 "F.Cu" signal) (44 "Edge.Cuts" user))
                  (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts") (width 0.05))
                  (footprint "R" (layer "F.Cu") (at {at}) (property "Reference" "R1")))"#
            ))
            .unwrap()
        };
        let done = |p: &Progress| steps(p).iter().map(|s| s.done).collect::<Vec<_>>();
        // A board with its outline and a part inside it, routed and checked; no schematic.
        let (inside, outside) = (board("5 5"), board("30 5"));
        let mut p = Progress { board: &inside, parts: vec![], wired: false, unrouted: 0, problems: 0, exported: false, lit: None };
        assert_eq!(done(&p), [false, false, false, false, true, true, true, true, false]);
        (p.unrouted, p.problems, p.exported) = (1, 2, true);
        assert_eq!(done(&p)[5..], [true, false, false, true]);
        // A part off the board isn't placed.
        p.board = &outside;
        assert!(!done(&p)[5]);
    }

    #[test]
    fn the_walk_through_lights_the_led_only_the_right_way_round() {
        let net = |pins: &[(&str, &str)]| (String::new(), pins.iter().map(|&(p, n)| (p.to_string(), n.to_string())).collect::<Vec<_>>());
        // The cell's + to R1, R1 to the LED's anode (pin 2), its cathode back to the cell's −.
        let forward = [net(&[("BT1", "1"), ("R1", "1")]), net(&[("R1", "2"), ("D1", "2")]), net(&[("D1", "1"), ("BT1", "2")])];
        assert_eq!(lit(&forward, "BT1", "R1", "D1"), Some(true));
        // The resistor on the − side lights it too; the LED turned round doesn't.
        let after = [net(&[("BT1", "1"), ("D1", "2")]), net(&[("D1", "1"), ("R1", "2")]), net(&[("R1", "1"), ("BT1", "2")])];
        assert_eq!(lit(&after, "BT1", "R1", "D1"), Some(true));
        let turned = [net(&[("BT1", "1"), ("R1", "1")]), net(&[("R1", "2"), ("D1", "1")]), net(&[("D1", "2"), ("BT1", "2")])];
        assert_eq!(lit(&turned, "BT1", "R1", "D1"), Some(false));
        // Not closed yet: neither.
        assert_eq!(lit(&forward[..2], "BT1", "R1", "D1"), None);
    }
}
