# crates/core: nets (`schematic/net.rs`)

What a schematic connects, as KiCad 10 connects it; its electrical rules are in `erc.md`.

`Project::netlist()` — each net's name as KiCad's netlist names it (`/Sheet/LABEL`, `GND`,
`Net-(R1-Pad2)`) and the parts' pins on it (power symbols, unnumbered `R?` left out).
Sheets keep what connects (`Wiring`: wires, buses, bus entries, junctions, no-connect flags,
labels, pins with their type, hidden ones too; `Sheet` pins) as they are read; `Project::nets`
joins them. KiCad 10's rules, found by exporting test sheets' netlists (skill `erc-check`):
- Ends meet the ends and pins at their point; a wire ending on another's middle, or a pin on a
  wire's middle, joins it only at a junction; labels join a wire or bus anywhere along it.
- Wires and buses are apart: bus entries are drawn only. A no-connect pin joins nothing.
- Names join on their sheet, whatever their kind; global labels, power symbols (by value; a
  flag's output names nothing) and parts' hidden power inputs (by pin name) on every sheet;
  KiCad 10's local power symbols on theirs. A sheet pin joins the name it has inside its sheet
  when a hierarchical label there has it (even on nothing); without one, nothing there.
- Bus members join the same members of the other names on their bus: vectors by position
  (`D[3..2]` counts up, as `D[2..3]`), groups by member (`P{X Y}`: `P.X`, `P.Y`), the project's
  aliases (`.kicad_pro` `bus_aliases`) as groups.
- A net's name: global label, power symbol, hidden power pin, local label, hierarchical label,
  sheet pin; then the sheet highest up; then the alphabet. Paths write sheet names trimmed, `/`
  as `{slash}`.
- Else a pin names it: one whose library name (not an alternate's) no other pin of its unit
  has, then by reference (the unit's: `U4A` before `U4B`), name and number, as text (`R10`
  before `R9`, `Y` before `Z`, `C` before `b`). `Net-(U1A-SDA)`; a name two pins have
  `Net-(U1A-SH-Pad1)`; no name (empty, `~` or the number) `Net-(U1-Pad5)`, the part's
  reference. `unconnected-(U1A-SDA-Pad5)`, `unconnected-(U1-Pad5)` when the pin is alone (a
  power flag's counts) or a no-connect flag is on the net.

Cross-probing (the board's nets have these names, F8): `Project::net_of(page, item)` — a wire's
or label's net's name (none for a bus, or a net with no name and no part's pin);
`Project::net_items(name)` — each page it reaches: its wires, labels, power symbols, and the
points of its pins and sheet pins (hidden pins left out).

## Checked
- `net.rs` tests: each rule above as KiCad 10 joins it, bus names as it reads them; a net found
  on each sheet it reaches.
- `tests/corpus.rs`: the four corpus projects' netlists equal KiCad 10's export (nets, pins and
  a digest of names, those made from pins too, and pins).

## Speed
0.5–1.6 ms a corpus project, once its sheets are read; a wire's net named and found on every
sheet ≤ 4 ms (glasgow-revD1, 1.5 ms hackrf-one).

## Missing (`// shortcut:` in the code)
KiCad 6–8 bus aliases, kept in the sheets (KiCad 9+ keeps them in the `.kicad_pro`). A pin named
`~` names nothing; KiCad 10 takes it as the name in its own sheets (which write none as `""`).
Seen after edits (`schematic-move.md`), unlike KiCad: which of a net's several names (left by an
edit across sheets) names it; a hierarchical bus label moved off its bus still joins its members;
pins on the wire ends of a bus's entries, with no label there, KiCad joins in a net named after
the bus (`/ADC/{ADC}`), Cuprita leaves apart.
