# crates/core: what `tests/corpus.rs` checks

On the corpus (`corpus/manifest.tsv`, skill `corpus`; skipped when it isn't fetched), against
KiCad 10's own results where there are any:
- Every corpus board: lossless lexing and saving (edits read back; every part flipped and back;
  ~200 items moved, turned and deleted together, re-checked as fully checked),
  item counts checked against raw text, DRC against `kicad-cli`, re-check equal to a full check,
  routes keeping clearances, parts, values, fields, footprints and nets from its schematic.
- Every corpus sheet read, its items counted against raw text; the projects' netlists and ERC
  against KiCad 10's (`kicad-cli sch export netlist`, `sch erc`); symbols, wires, labels added.
- KiCad's simulation demos (`boards/sim`): their SPICE netlists with KiCad's parts, and those
  that run in under a second simulated in ngspice, their circuits working (`ngspice.md`).
