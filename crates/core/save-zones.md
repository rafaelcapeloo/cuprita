# crates/core: saving zones (`save/zones.rs`)

Part of `save.md`. `Board::write` pairs the board's zones with the file's by `Zone::src`, their
place there: the file's it lacks go with the whitespace before them (`Patch::delete`); one in a
place the text lacks fails (read from other text); the rest are new (`Board::add_zone`, `fill-new.md`).

## What changes
- A zone whose outline differs from the file's (moved or turned with what is picked): its outline
  and cutouts written as its KiCad version writes them in place of its first `polygon`, the
  others taken away; its fills, moved along, as a refill's.
- A zone whose fills differ from the file's (a refill): its `filled_polygon`s are replaced, the
  new ones after its outline, written as its KiCad version does (layer named from KiCad 6, tabs
  from 8). Fills KiCad draws with an outline as wide as the minimum width (`Zone::outlined`: a
  zone saying `(filled_areas_thickness yes)`, or nothing in files before version 20250210, as
  KiCad 5's) are read grown by half of it less 1 nm, as KiCad 10 loads them (within 0.02%; parsing
  hackrf-operacake takes 19 ms, 2 without), and saved shrunk back.
- An edited zone (`set_zone`, `fill-new.md`): its outline, then each cutout, a `polygon` each as
  KiCad 10 saves cutouts, in place of the file's (with its arcs, made corners). Its settings
  changed where the file has them: the net (by number and `net_name` to KiCad 9, by name in 10,
  where none is left out), `name` (after the id), `priority` (after `hatch`; 0 is left out), the
  pad connection (`connect_pads yes`, `no`, `thru_hole_only`, nothing for thermal reliefs), the
  clearance, `min_thickness`, `thermal_gap`, `thermal_bridge_width`; a list the file lacks goes
  where KiCad writes it. The rest stays as it was, unknown fields too.
- New zones go after the file's last zone (else with the new tracks): their settings, the
  outline, the fills and its id, as the file's KiCad version writes a zone. KiCad 4–5: bare names,
  an 8-digit `tstamp`, no `filled_areas_thickness` (KiCad 5 draws every fill with an outline:
  saved shrunk); 6–7: quoted names, `(tstamp UUID)`, settings on four lines; 6–9 say
  `(filled_areas_thickness no)`; 8 on: a line per list, tab indents, `(uuid "UUID")`; 10 names
  the net (no `net_name`).

## Checked
- Fills: `save/zones.rs` tests write refilled zones for KiCad 4–10 (exact text) and shrink outlined
  ones; `tests/corpus.rs` reads every refilled board's fills back (KiCad 6 on). The writer gives
  KiCad's own text for 552 of 553 corpus fills (one has an `(island)` mark). KiCad 10 loads each
  refilled corpus board with Cuprita's fill area per zone and layer: exact from KiCad 6, KiCad 4/5
  within 0.05% (thermal spokes come out round where they start on their pad; arcs).
- New zones: `save/zones.rs` tests write one for KiCad 5, 7, 8 and 10 (exact text) and read it back;
  `tests/corpus.rs` adds one to every corpus board and reads it back. KiCad 10 (skill
  `kicad-check`) loads one on each corpus board (KiCad 4–10; two on kria-k26) as its own new
  zone with the board's defaults: net, layer, outline, clearance, minimum width, thermal gap and
  spokes, pad connection, island removal, priority, fill mode, smoothing and hatch.
- Edited zones: `save/zones.rs` tests write a corner moved and a cutout for KiCad 5, 7 and 10, and
  every setting there and back in KiCad 7 and 10 text (KiCad 10's own for them), exact;
  `tests/corpus.rs` gives every corpus board's last zone a corner and new settings, read back. KiCad 10
  (skill `kicad-check`) loads a small KiCad 10 board and hackrf-one (KiCad 6) so edited, in the
  app and the core, with each corner, cutout (a hole) and setting as meant; its own refill of the
  edited zone is within 0.01–0.04% of Cuprita's on the small board, 0.03–0.2% on hackrf-one's GND
  plane (as unedited).
- Deleted zones: a `save/zones.rs` test deletes zones of each kind (text and fills gone, the rest
  byte for byte); `tests/corpus.rs` deletes every corpus board's first zone; KiCad 10 loads boards
  of KiCad 10, 7, 6 and 5 with one deleted (planes, a rule area) as its own delete leaves them.
