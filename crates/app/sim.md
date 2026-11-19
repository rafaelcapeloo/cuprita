# crates/app: simulation (`sim.rs`)

The schematic run in ngspice (S); the core side: `crates/core/spice.md`, `crates/core/ngspice.md`.
Parent: `sheet.md`.

## Entry points
- `sim.rs` — simulation (S, the button; desktop only): `App::simulate` writes the netlist of the
  sheets as edited (`Project::spice`) and runs it on a thread (`ngspice::simulate`; S again or
  Esc stops it); `Sch::sim` is a `Simulation`: running, failed (the parts and fixes), done.
  `App::sim_panel` shows it below the sheet: progress, then a tab per analysis, the signals
  (KiCad's names, `Deck::signals`) to tick beside an `egui_plot` chart (the workbook's traces
  ticked, else labelled nets that vary; AC in dB over log frequency; at most 4,000 points a
  line, peaks kept), an operating point's values in SI units; hover reads a point.

## Missing
Probing a net on the sheet to plot it, an operating point's values on the sheet, two y axes
(volts and amps share one), phase in AC, editing a part's model or the analysis in Cuprita
(KiCad's Simulation Model and Simulator set them).
