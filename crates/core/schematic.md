# crates/core: schematics (`schematic.rs`, `schematic/{symbol, label, project}.rs`)

What a schematic connects, its nets: `nets.md`; its electrical rules: `erc.md`; its SPICE netlist:
`spice.md`; editing it: `schematic-edit.md`, `-wiring.md`, `-hier.md`, `-move.md`, `-fields.md`.

`Schematic::parse(src, pro)` reads one `.kicad_sch` sheet (KiCad 6–10) as KiCad draws it, in
sheet coordinates (nm, Y down): `shapes` and `texts` (the board's `Shape` and `Text`, `layer` an
`Ink`: what KiCad's schematic colours tell apart), `symbols` (reference as shown and its part's,
value, footprint, `link` (a footprint's `path` to it), box, `on_board`: whether a footprint stands
for it; `symbol_at` picks one), `sheets` (name, file, box) and the `page`. `pro` is the project's
`.kicad_pro` text (`""` without one): its `text_offset_ratio` and `label_size_ratio` place labels.
KiCad 5's `.sch` errors: "open it in KiCad 6 or later, which converts it" (no importer, ROADMAP).

`Project::load(root, pro, read)` reads the root sheet and every sheet under it (`read`: a file's
text by the name sheets give it): a `Page` per place a sheet is used (sheet names from the root,
file, parent, the page each sheet box opens), its symbols showing that place's references and
units (KiCad 7+: the symbol's own `instances`; KiCad 6: the root's `symbol_instances`). A sheet
that can't be read, or holds itself, is an error (`Project::errors`) and opens nothing. `pro`
also gives the bus aliases and ERC settings.

## What it draws (KiCad 10's look, measured from its SVG plots, skill `look`)
- Wires and bus entries (6 mil when 0), buses (12 mil), junctions (36 mil), no-connects (48 mil).
- Symbols from the sheet's own library copies (`lib_symbols`; library Y up, text angles in
  tenths): the placed unit and body style, turned, then mirrored (`x` flips Y); outline and
  background fills; pins with bubbles and clock marks (25 mil); names inside the body (`pin_names
  offset`) or above the pin, numbers above (below when names are), 8 mil and half the pens off
  the line; a pin's alternate function, when the placed symbol picks one, by its name and look;
  fields upright, their justification turning with them (KiCad 5's `~` blank). References get
  the unit's letter (A–Z, AA…) when the part has several units.
- Local labels above the wire by (text offset ratio + 1/8) × height. Global labels: a box of the
  text's advance and 0.175 heights, a margin of label size ratio × height each side and an
  eighth-height pen; arrows half the box high; text 0.75 heights past an arrow, h/14 below the
  middle. Hierarchical labels: a flag one height long, text the text offset after it. Sheet pins
  are hierarchical labels seen from inside (input and output swap). Sheets: box, name, "File: …".
- Notes (0.25 mm up the sheet, whichever way they read), text boxes, lines, rectangles, circles,
  arcs, Béziers, rule areas.
- Text pens: their own; bold a fifth of the height; else 6 mil, at most a quarter of the height
  (pins: 6 mil). `font.rs` draws `~{…}` overbars and drops a text's final line break, as KiCad.
  The frame and title block: `Project::frame(i)` (`pdf.md`), not in the sheet's lists.

## Checked
- `schematic.rs` tests: labels, notes, pins, marks and fields (turned, mirrored) where KiCad 10
  plots them (text centres within 1 µm, fields 20 µm); wiring, sheets, page; the KiCad 5 error.
- `tests/corpus.rs`: the 41 sheets of the corpus boards' projects (KiCad 6, 7, 8 and 10) read,
  symbols, sheets, wires, buses, junctions and no-connects counted against the raw text.
- `project.rs` tests: a sheet used twice, each place its references; KiCad 6's; missing, looping.
  The corpus projects load whole (4–19 pages, 4–16 ms); every reference shown on every page is
  one KiCad's plot of it shows there, the four places of glasgow-revD1's IO bank sheet too.
- KiCad 10's 115 demo sheets read; the corpus projects' texts against KiCad's plots (skill
  `sch-check`): every one drawn where KiCad draws it but for glyph shapes and subscripts; none
  missing but tables and inter-sheet references.

## Speed
Each corpus sheet in at most 2.2 ms (jetson-orin `usb.kicad_sch`, 1.3 MB); whole projects in 3–13
ms once read (`examples/load.rs` takes a root sheet), 4–17 ms with the files; ERC: `erc.md`.

## Missing (`// shortcut:` in the code)
Sheet files outside the root's folder; images, tables, net class flags, DNP crosses, inter-sheet
references; dashed lines, own colours; IEEE pin looks; raised sub- and superscripts; a project's
own pin mark size and line widths. Hershey glyphs differ from KiCad's: advances ~1.5%, `1`, `µ`.
