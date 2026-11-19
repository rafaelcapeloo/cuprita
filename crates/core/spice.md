# crates/core: SPICE netlists (`schematic/spice.rs`)

`Project::spice(dir)` — the project's netlist for ngspice as KiCad 10's simulator writes it
(`Deck`: `text`; `behavior`, ngspice's `ngbehavior` for the `.kicad_pro`'s compatibility mode,
`ltpsa` without one; `dir`, the project's folder; `signals`, KiCad's names for ngspice's vectors,
`V(/out)`, `I(V1)`; `traces`, what the workbook plots). KiCad's rules, learned from `kicad-cli sch
export netlist --format spice` of test sheets (skill `spice-check`):
- A part's model is on its first unit's fields: `Sim.Device`, `.Type`, `.Params`, `.Pins`,
  `.Library`, `.Name`. Without them R, C, L, V and I of two pins take their value (`DC 5` for
  V, I). Exclude from simulation (KiCad 7: `Sim.Enable` 0) leaves a part out, DNP doesn't.
  Power symbols aren't parts; `GND` stays `GND`, which ngspice takes for 0.
- Values and parameters in KiCad's notation as SPICE reads them: `1M` `1Meg`, `4k7` `4.7k`,
  `4R7` `4.7`, `µ` `u`, `,` `.`, peta and exa `e15` `e18`, a unit `F` `H` `Ω` dropped; others as
  written (`100uF;16V`, `1meg`, `SINE(0 1 1k)`); raw SPICE (`Sim.Device` SPICE) as written.
- Nodes: net names with `(`, `)` and spaces `_`, `~{X}` `_X`, `_{X}` and `^{X}` `X`, `{slash}`
  `/`. `Sim.Pins` (`sym=model`) by the model pin's name, else its position; the part's other
  pins fill the model's other pins in order; a model pin it needs and lacks is `NC-<ref>-<n>`.
- The element's letter goes before a reference that lacks it (`MQ2`, `XU1`, `BV1`).
- Sources: `Sim.Type` DC, SIN, PULSE, EXP, AM, SFFM, PWL, white, pink and burst noise
  (`TRNOISE`), random uniform, normal, exponential and Poisson (`TRRANDOM`): KiCad's parameters
  in ngspice's order up to the last one given, those before it 0; DC from `dc` or the value;
  `AC ac ph`. Type `=`: a B source.
- Built-in models (`Sim.Device` D, NPN, PNP, NJFET, PJFET, NMES, PMES, NMOS, PMOS; `Sim.Type`
  as ngspice's levels: VBIC 4, BSIM3 8, BSIM4 14…; SW of type V): `.model __<ref> …` with the
  model's parameters; the line takes the device's own (area, m, w, l, temp…).
- Libraries: relative to the project, `${KIPRJMOD}`, `${KICAD*_SYMBOL_DIR}` (KiCad's symbols,
  which ship `Simulation_SPICE.sp`), other `${VAR}`s from the environment; `.include`d. A
  `.subckt` gives the ports (`Sim.Params` go on its line), a `.model` the device (model
  parameters set make a copy, `<ref>.<model>`). `Sim.Device` SPICE: its `type`, `model`, `lib`.
- Directives: a text or text box with a line starting with `.` or coupling inductors (`K1 L1
  L2 0.98`), whole, its relative `.include`/`.lib` made absolute. The workbook's (`.kicad_pro`
  `workbook_filename`) tabs' analyses replace the sheets': one run makes every tab's plot.
- An error names the parts and what to do: no model, a library or model not found, a model not
  written yet, KiCad 6's `Spice_` fields, a part not annotated; parts alike in one line.

## Where we differ from KiCad, on purpose
- Numbers keep their digits (KiCad rounds to 6, `1e3` to `1k`); a source's missing parameters
  stay ngspice's defaults (KiCad writes `PULSE( 0 5 0 1 )` for y1 and y2 alone).
- A source's value that is a text variable (`${SIM.PARAMS}`) is DC 0 (KiCad writes what it
  stands for: `DC ac=1`).
- q17's `all_devices.lib`: KiCad 10 doesn't read its ports and models (its export wires Q6 and
  Q8 in symbol order, writes `Q1 … Q1.IRF610h`, a model it never defines); Cuprita does.

## Checked
- `spice.rs` tests: values, nodes, directives and parameters as KiCad writes them; a project of
  each kind of part, its library, workbook and errors.
- `tests/corpus.rs`: KiCad 10's simulation demos (`boards/sim`, 10 projects) write KiCad's parts;
  all 18 of KiCad's demos but IBIS's equal its export line by line (skill `spice-check`), but
  for the differences above.

## Speed
0.06–0.35 ms a demo project (q17: 80 parts).

## Missing (`// shortcut:` in the code)
IBIS, XSPICE code-model devices, transmission lines, potentiometers, current-controlled
switches; KiCad 6's `Spice_` fields (KiCad 7+ converts them); `.include`s inside a library
aren't followed to find a model.
