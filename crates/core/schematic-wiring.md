# crates/core: wiring schematics (`schematic/{edit, bus}.rs`)

Wires, buses and labels added to a project's sheets as patches (`schematic-edit.md`); what
they join: `nets.md`.

## Adding a wire (`Project::add_wire(page, pts)`) or a bus (`add_bus`)
- A segment between each two points (on the grid), as KiCad 10 writes them (`add_line`); and a
  junction where a wire's end, a pin or a bus entry's end lies on a wire's middle (a T joins only at one), or
  three ends meet, as KiCad's editor adds them; none where wires cross. A bus's: where a bus's
  end lies on a bus's middle, or three bus ends meet. `Schematic::joins(p)` (`joins_bus`):
  whether a wire (bus) ending at `p` joins something there (the app's wire ends there).
- A bus entry (`add_bus_entry(page, at, angle)`, KiCad's wire to bus entry): from `at`, 100 mil
  right and down, turned 0–270° counterclockwise, as KiCad 10 writes one, with a junction where
  an end lies on a wire's middle. Labels name a bus as a wire, `D[0..7]`, `{SDA SCL}`
  (`nets.md`): a wire takes a member by a label of its name; the entry is drawn only.

## Adding a label (`Project::add_label(page, name, kind, shape, at, angle)`)
- A label of `LabelKind` (local, global, hierarchical) as KiCad 10 writes one: on the grid,
  turned 0–270°, its text upright from its point (`justify left`, at 180° and 270° `right`), in
  the project's `default_text_size`; the name trimmed, one line, its quotes and backslashes
  escaped. It joins a wire or bus anywhere along it and a pin at its point (`nets.md`), so it
  needs no junction. A global or hierarchical one has a shape of `SHAPES` (KiCad's five), its
  box centred on its point.
- Global: joins the labels of its name on every sheet. Without the inter-sheet references
  field KiCad writes in one (its form differs by version): KiCad adds it, and its plots place
  the references as for its own labels.
- Hierarchical: the boxes showing its sheet get its pin too (`schematic-hier.md`).

## Adding a no-connect flag (`Project::add_no_connect(page, at)`)
- On the pin ending at the grid point nearest `at` (`Schematic::pin_at`: its reference and
  number; not a hidden pin), as KiCad 10 writes one; none off a pin's end or where one is.

## Checked
- `edit.rs` tests: wires' junctions (T, crossing, pin or end on a middle) and nets; labels'
  text, nets and errors; global labels joining across sheets. `tests/corpus.rs`: `schematic-edit.md`.
- KiCad 10 (skill `erc-check`): three resistors and five wires (ends, a T, a crossing, a pin on
  a middle) on every page: KiCad's netlists equal Cuprita's, joined as meant. Two resistors and
  four labels (on pins, a wire's middle and end, 0–270°) on every sheet: KiCad's netlists equal
  Cuprita's (386, 620, 811, 842 nets), its ERC adds what Cuprita's does, its plots draw the labels
  where Cuprita does but for glyph advances (≤ 0.11 mm along them). Two resistors and four global
  labels (0–270°) on every sheet: KiCad's netlists equal Cuprita's (383, 602, 795, 808 nets), each
  name one net across all sheets; neither ERC changes; its plots draw the 176 labels' texts
  within 0.073 mm of Cuprita's (skill `sch-check`). A flag on glasgow-revD1's R22 (its wire
  deleted): KiCad's ERC accepts it (no `no_connect_*`), the pin no longer unconnected.
  Hierarchical labels: `schematic-hier.md`. `bus.rs` tests: a T, entries both ways, a vector and
  its other name joined member by member, undo. On every sheet a bus named `EBUS[0..3]` and
  `EOUT[0..3]` and one named `G{CLK DAT}` and `H{CLK DAT}`, entries to wired resistors labelled
  `EBUS0`… and `EOUT0`…: KiCad's netlists equal Cuprita's, names too (796, 451, 1146, 971 nets),
  so its ERC (`net_not_bus_member`, `multiple_net_names`); its plots draw them where Cuprita does.

## Speed
A wire, bus or bus entry: 4.4 ms at worst (glasgow-revD1's level shifters, used in 4 places); a
label, local or global, 4.1.

## Missing (`// shortcut:` in the code)
KiCad's unfold from bus (C); moving buses and bus entries (`schematic-move.md`).
