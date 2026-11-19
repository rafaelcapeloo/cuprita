---
name: erc-check
description: Check Cuprita's schematic nets and ERC against KiCad 10's - its netlist export and `kicad-cli sch erc` - on a project or on a test sheet written to learn a KiCad rule. Use after any change to `crates/core/src/schematic/{net, erc, edit}.rs` or to what a sheet records of its wiring, or to learn how KiCad connects or checks something.
---
# Check nets and ERC against KiCad

KiCad 10 is the flatpak `org.kicad.kicad`. Work in the session scratch dir `$S` (the sandbox
sees only `$HOME` and what `--filesystem` adds) and delete what you made. Projects to check: the
corpus boards' (`corpus/boards/<name>/`, skill `corpus`), copied into `$S/p/<name>/`.

## Export KiCad's
`flatpak run --filesystem=$S --command=kicad-cli org.kicad.kicad sch export netlist --format
kicadsexpr -o $S/k/<name>.net $S/p/<name>/<root>.kicad_sch`, and the same with `sch erc --format
json --severity-all -o $S/k/<name>-erc.json` (positions there are in units of 100 mm).

## Cuprita's
A scratch cargo project with `cuprita-core = { path = "<repo>/crates/core" }` and
`.claude/skills/erc-check/check.rs` as its `src/main.rs`: `check $S/p/<name>/<root>.kicad_sch
$S/e/<name>` writes `nets.tsv` and `erc.tsv`.

## Compare
`python3 -I .claude/skills/erc-check/kicad.py nets $S/k/<name>.net $S/e/<name>/nets.tsv` prints
how many nets are the same (as sets of pins), KiCad's biggest ones Cuprita splits or joins, and
names unlike KiCad's (pin-made `Net-(…)` ones too). `kicad.py erc <erc.json> <erc.tsv>` prints
counts by kind; the differences `crates/core/erc.md` lists are expected. `kicad.py digest
<kicad.net>…` prints the numbers `tests/corpus.rs` pins for a project's netlist.

## Check an edit
That KiCad reads what an edit writes (`schematic/edit.rs`) as Cuprita means it: a scratch program
loads a corpus project, makes the edits (`Project::add_symbol`, symbols from `library::symbol`),
copies the project's folder and writes each `Project::text(file)` over it. Export KiCad's
netlist and ERC of the copy and compare them as above; its texts with skill `sch-check`.

## Learn a KiCad rule
Write a spec (the format is in `kicad.py`'s first lines): parts, wires, labels, buses and sheets
at mm coordinates, each case apart. `kicad.py sheet $S/x/<case> <spec.py>` writes the project,
exports KiCad's netlist and ERC and prints both; run `check` on `$S/x/<case>/t.kicad_sch` to
compare. Put every case in one sheet, far apart, and read which pins KiCad joins. The rules in
`crates/core/{nets, erc}.md` were found so (a wire's T without a junction, labels along wires,
power symbols named by value, bus members by position or name, which names win, what ERC counts
once): write what you find there, and a test in `net.rs` or `erc.rs` with KiCad's answer.

## Gotchas
- A wire or bus is two points: a longer path is several (the spec splits it).
- A sheet's pins sit on its box's edge, or KiCad moves them there: size the box to hold them.
- KiCad checks some problems in each use of a sheet, others once (`erc.md`): use one child sheet
  twice to see which.
- A wire on nothing, KiCad reports at its start; a T's open end, at the wire's start too.
