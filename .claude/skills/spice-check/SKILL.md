---
name: spice-check
description: Check Cuprita's SPICE netlists against KiCad 10's - `kicad-cli sch export netlist --format spice` - on KiCad's simulation demos, or on a test sheet written to learn how KiCad writes a model. Use after any change to `crates/core/src/schematic/spice.rs` or to what a sheet records of parts' simulation models and directives.
---
# Check SPICE netlists against KiCad

KiCad 10 is the flatpak `org.kicad.kicad`. Work in the session scratch dir `$S` (the sandbox sees
only `$HOME` and what `--filesystem` adds) and delete what you made. Projects to check: KiCad's
18 simulation demos, 10 of them in the corpus (`corpus/boards/sim/`, skill `corpus`), all in the
flatpak's library (`/var/lib/flatpak/runtime/org.kicad.kicad.Library/x86_64/stable/active/files/
demos/simulation/`), copied into `$S/demos/`.

## Export both
- KiCad's: `flatpak run --filesystem=$S --command=kicad-cli org.kicad.kicad sch export netlist
  --format spice -o $S/k/<name>.cir $S/demos/<demo>/<root>.kicad_sch` (a model it can't find
  is on stderr).
- Cuprita's: `cargo run --release -p cuprita-core --example spice -- $S/demos/<demo>/<root>.kicad_sch
  > $S/e/<name>.cir` (the time and errors on stderr).

## Compare
`python3 -I .claude/skills/spice-check/spice.py diff $S/k/<name>.cir $S/e/<name>.cir` prints
the parts, models, includes and directives that differ, by meaning: numbers as ngspice reads
them, spacing and case ignored, KiCad's library model copies (`Q7.Q2SC2240`) as the model. The
workbook's analyses only Cuprita's has (KiCad's export leaves the workbook out), and the
differences `crates/core/spice.md` lists, are expected.

## Simulate
`--example spice -- --run <root>.kicad_sch…` runs each in ngspice (`crates/core/ngspice.md`) and
prints its plots, their sizes, ngspice's warnings and the time. KiCad's export runs the same
way once its analysis is in it: simulate both and compare the vectors to see a difference matter.

## Learn a KiCad rule
Write a spec (the format is in `spice.py`'s first lines): parts with the `Sim.*` fields to try,
each pin on its own label, notes, model library files. `spice.py sheet $S/x/<case> <spec.py>`
writes the project and prints KiCad's netlist; `--example spice` on `$S/x/<case>/t.kicad_sch` gives
Cuprita's. Put every case in one sheet (each part is apart) and read how KiCad writes it. The
rules in `crates/core/spice.md` were found so (values, nodes, pins by name then position, which
parameters go on the line, sources' parameter orders, directives): write what you find there, and
a test in `spice.rs` with KiCad's answer.

## Gotchas
- `kicad-cli`'s export doesn't read the workbook (`.wbk`): demos whose analysis is only there
  export none, and ngspice runs nothing.
- A library KiCad fails to read (q17's `all_devices.lib`) makes it wire the part in symbol order
  and name a model copy it never writes; compare against a small library holding the same model.
- A sheet's version doesn't change how KiCad 10 reads `Sim.Pins` (checked 20231120–20250114).
