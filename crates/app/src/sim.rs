//! Simulation in the schematic view (S, or the button): the project's netlist (`Project::spice`)
//! run in ngspice in the background (S again or Esc stops it), then each analysis below the
//! sheet: a plot of the signals ticked (the workbook's, else the nets labels name), or an
//! operating point's values (`crates/core/spice.md`, `ngspice.md`).

use crate::App;
use crate::tr::{n, tr, trf};
use cuprita_core::ngspice::{self, Plot, Sim, Vector};
use cuprita_core::schematic::Deck;
use eframe::egui;
use egui_plot::{HoverPosition, Legend, Line, Plot as Chart};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use web_time::{Duration, Instant};

/// A simulation of the schematic: going, or what it gave.
pub(crate) enum Simulation {
    Running {
        stop: Arc<AtomicBool>,
        thread: JoinHandle<(Deck, Result<Sim, String>)>,
        since: Instant,
    },
    /// Its analyses, the one shown, and which of each one's signals are drawn.
    Done {
        deck: Deck,
        sim: Sim,
        took: Duration,
        tab: usize,
        shown: Vec<Vec<bool>>,
    },
    Failed(String),
}

/// The most points a signal is drawn with: past it, each stretch's lowest and highest, so peaks show.
const DRAWN: usize = 4000;

impl App {
    /// S, or the button: simulates the project's schematic as it is, edits not saved yet too, in
    /// the background; or stops the run going.
    pub(crate) fn simulate(&mut self, ctx: &egui::Context) {
        let Some(sch) = &mut self.sch else { return };
        if let Some(Simulation::Running { stop, .. }) = &sch.sim {
            return stop.store(true, Ordering::SeqCst);
        }
        let dir = sch.sheets.root.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
        sch.sim = Some(match sch.sheets.project.spice(&dir) {
            Err(e) => Simulation::Failed(e),
            Ok(deck) => {
                let (stop, ctx) = (Arc::new(AtomicBool::new(false)), ctx.clone());
                let go = stop.clone();
                let thread = std::thread::spawn(move || {
                    let sim = ngspice::simulate(&deck, &go);
                    ctx.request_repaint();
                    (deck, sim)
                });
                Simulation::Running { stop, thread, since: Instant::now() }
            }
        });
    }

    /// The simulation below the sheet: how far it is while it runs (Esc stops it), then a tab
    /// for each analysis: its signals to tick beside their plot (AC's in dB over a log
    /// frequency), an operating point's values; hover shows a point's values.
    pub(crate) fn sim_panel(&mut self, ui: &mut egui::Ui) {
        let Some(sch) = &mut self.sch else { return };
        if let Some(Simulation::Running { thread, since, .. }) = &sch.sim
            && thread.is_finished()
        {
            let took = since.elapsed();
            let Some(Simulation::Running { thread, .. }) = sch.sim.take() else { return };
            sch.sim = Some(match thread.join() {
                Ok((deck, Ok(sim))) => {
                    let shown = sim.plots.iter().map(|p| drawn_first(p, &deck)).collect();
                    Simulation::Done { deck, sim, took, tab: 0, shown }
                }
                Ok((_, Err(e))) => Simulation::Failed(e),
                Err(_) => Simulation::Failed(tr("the simulation stopped with an error inside ngspice").into()),
            });
        }
        let Some(sim) = &mut sch.sim else { return };
        let mut close = false;
        egui::Panel::bottom("sim").resizable(true).default_size(280.0).show(ui, |ui| match sim {
            Simulation::Running { stop, since, .. } => {
                ui.horizontal(|ui| {
                    let (p, secs) = (ngspice::progress(), format!("{:.1}", since.elapsed().as_secs_f32()));
                    ui.spinner();
                    ui.label(if p > 0.0 {
                        trf("Simulating in ngspice: {0}%, {1} s", &[&format!("{p:.0}"), &secs])
                    } else {
                        trf("Simulating in ngspice: {0} s", &[&secs])
                    });
                    if ui.button(tr("Stop (Esc)")).clicked() || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                        stop.store(true, Ordering::SeqCst);
                    }
                });
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
            Simulation::Failed(e) => {
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, tr("Not simulated"));
                    close = ui.button(tr("Close")).clicked();
                });
                egui::ScrollArea::vertical().show(ui, |ui| ui.label(e.as_str()));
            }
            Simulation::Done { deck, sim, took, tab, shown } => {
                ui.horizontal(|ui| {
                    for (k, p) in sim.plots.iter().enumerate() {
                        if ui.selectable_label(*tab == k, analysis(&p.name)).clicked() {
                            *tab = k;
                        }
                    }
                    ui.label(trf("simulated in {0} s", &[&format!("{:.2}", took.as_secs_f32())]));
                    if !sim.log.is_empty() {
                        let log = sim.log[..sim.log.len().min(40)].join("\n");
                        ui.label(crate::count(sim.log.len(), n!("ngspice message"))).on_hover_text(log);
                    }
                    close = ui.button(tr("Close")).clicked();
                });
                if let (Some(p), Some(shown)) = (sim.plots.get(*tab), shown.get_mut(*tab)) {
                    plot(ui, p, deck, shown);
                }
            }
        });
        if close {
            sch.sim = None;
        }
    }
}

/// What an analysis is, by ngspice's name for its plot (`tran1`).
fn analysis(plot: &str) -> String {
    let name = plot.trim_end_matches(|c: char| c.is_ascii_digit());
    let what = match name {
        "tran" => tr("Transient"),
        "ac" => tr("AC"),
        "dc" => tr("DC sweep"),
        "op" => tr("Operating point"),
        "noise" => tr("Noise"),
        "tf" => tr("Transfer function"),
        _ => name,
    };
    format!("{what} ({plot})")
}

/// Which of a plot's signals (its vectors but the scale) are drawn at first: those its workbook
/// plots, else the nets labels name that vary (not a supply's flat line), at most 8.
fn drawn_first(p: &Plot, deck: &Deck) -> Vec<bool> {
    let signals = &p.vectors[if p.name.starts_with("op") { 0 } else { 1 }.min(p.vectors.len())..];
    let name = |v: &Vector| name(v, deck);
    let traced: Vec<bool> = signals.iter().map(|v| deck.traces.iter().any(|t| t.eq_ignore_ascii_case(&name(v)))).collect();
    if traced.contains(&true) {
        return traced;
    }
    let mut labelled = 0;
    signals
        .iter()
        .map(|v| {
            let n = name(v);
            let named = n.starts_with("V(") && !n.starts_with("V(Net-(") && !n.starts_with("V(unconnected-(");
            let (lo, hi) = size(v).fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
            let named = named && hi - lo > 1e-9 * hi.abs().max(lo.abs());
            labelled += named as usize;
            named && labelled <= 8
        })
        .collect()
}

/// A vector's values, an AC one's magnitudes.
fn size(v: &Vector) -> impl Iterator<Item = f64> + '_ {
    v.re.iter().enumerate().map(|(i, re)| v.im.get(i).map_or(*re, |im| re.hypot(*im)))
}

/// A vector's name as KiCad's simulator shows it: `V(/out)`, `I(V1)`.
fn name(v: &Vector, deck: &Deck) -> String {
    deck.signals.get(&v.name).cloned().unwrap_or_else(|| v.name.clone())
}

/// An analysis: an operating point's values; else its signals, ticked to draw, beside their plot.
fn plot(ui: &mut egui::Ui, p: &Plot, deck: &Deck, shown: &mut [bool]) {
    if p.name.starts_with("op") {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("op").striped(true).show(ui, |ui| {
                for v in &p.vectors {
                    ui.label(name(v, deck));
                    ui.label(si(v.re.first().copied().unwrap_or(f64::NAN), v.unit));
                    ui.end_row();
                }
            });
        });
        return;
    }
    let Some((x, signals)) = p.vectors.split_first() else { return };
    let ac = !x.im.is_empty() || x.unit == "Hz";
    // The panel keeps the size of what it holds: the plot takes what is left, a height at least.
    let height = ui.available_height().max(200.0);
    ui.horizontal_top(|ui| {
        egui::ScrollArea::vertical().id_salt("signals").max_width(220.0).max_height(height).show(ui, |ui| {
            ui.vertical(|ui| {
                for (v, on) in signals.iter().zip(shown.iter_mut()) {
                    ui.checkbox(on, name(v, deck));
                }
            });
        });
        let (xu, yu) = (x.unit, if ac { "dB" } else { signals.iter().zip(&*shown).find(|(_, on)| **on).map_or("", |(v, _)| v.unit) });
        let along = move |v: f64| if ac { si(10f64.powf(v), xu) } else { si(v, xu) };
        let chart = Chart::new("sim plot")
            .height(height)
            .legend(Legend::default())
            .x_axis_formatter(move |m, _| along(m.value))
            .y_axis_formatter(move |m, _| if ac { format!("{:.0} dB", m.value) } else { si(m.value, yu) })
            .label_formatter(move |pos| {
                let (name, at) = match pos {
                    HoverPosition::NearDataPoint { plot_name, position, .. } => (format!("{plot_name}\n"), position),
                    HoverPosition::Elsewhere { position } => (String::new(), position),
                };
                let y = if ac { format!("{:.2} dB", at.y) } else { si(at.y, yu) };
                Some(format!("{name}{}\n{y}", along(at.x)))
            });
        chart.show(ui, |chart| {
            for (v, _) in signals.iter().zip(&*shown).filter(|(_, on)| **on) {
                let ys: Vec<f64> = if ac { size(v).map(|m| 20.0 * m.log10()).collect() } else { v.re.clone() };
                let xs: Vec<f64> = if ac { x.re.iter().map(|f| f.log10()).collect() } else { x.re.clone() };
                chart.line(Line::new(name(v, deck), drawn(&xs, &ys)));
            }
        });
    });
}

/// A signal's points as drawn: at most `DRAWN`, past it each stretch's lowest and highest; none
/// that isn't a number (a zero's −∞ dB).
fn drawn(x: &[f64], y: &[f64]) -> Vec<[f64; 2]> {
    let (x, y): (Vec<f64>, Vec<f64>) = x.iter().zip(y).filter(|(x, y)| x.is_finite() && y.is_finite()).unzip();
    let n = x.len();
    if n <= DRAWN {
        return (0..n).map(|i| [x[i], y[i]]).collect();
    }
    let stretch = n.div_ceil(DRAWN / 2);
    let ends = |from: usize| {
        let to = (from + stretch).min(n);
        (from..to).fold((from, from), |(lo, hi), i| (if y[i] < y[lo] { i } else { lo }, if y[i] > y[hi] { i } else { hi }))
    };
    (0..n).step_by(stretch).map(ends).flat_map(|(lo, hi)| [lo.min(hi), lo.max(hi)]).map(|i| [x[i], y[i]]).collect()
}

/// A value with its SI prefix and unit, to 4 digits: `1.5 mA`, `20 µs`, `3.3 V`.
fn si(v: f64, unit: &str) -> String {
    if v == 0.0 || !v.is_finite() {
        return format!("{v} {unit}");
    }
    let k = ((v.abs().log10() / 3.0).floor() as i32).clamp(-5, 4);
    let prefix = ["f", "p", "n", "µ", "m", "", "k", "M", "G", "T"][(k + 5) as usize];
    let n = format!("{:.3}", v / 10f64.powi(3 * k));
    format!("{} {prefix}{unit}", n.trim_end_matches('0').trim_end_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_long_signals_with_their_peaks() {
        let x: Vec<f64> = (0..100_000).map(|i| i as f64).collect();
        let y: Vec<f64> = x.iter().map(|t| (t / 1000.0).sin()).collect();
        let d = drawn(&x, &y);
        assert!(d.len() <= DRAWN && d.windows(2).all(|w| w[0][0] <= w[1][0]));
        let (lo, hi) = d.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p[1]), hi.max(p[1])));
        assert!(lo < -0.9999 && hi > 0.9999, "{lo} {hi}");
        assert_eq!(drawn(&x[..3], &y[..3]).len(), 3);
        assert_eq!(drawn(&[1.0, 2.0], &[f64::NEG_INFINITY, 0.0]), [[2.0, 0.0]]);
        assert_eq!([si(1.5e-3, "A"), si(2e-5, "s"), si(3.3, "V"), si(-47000.0, "Ω")], ["1.5 mA", "20 µs", "3.3 V", "-47 kΩ"]);
        assert_eq!([analysis("tran1"), analysis("ac2")], ["Transient (tran1)", "AC (ac2)"]);
    }
}
