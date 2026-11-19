# crates/core: fab outputs (`fab.rs`, `gerber.rs`, `bom.rs`)

The files a fab makes and assembles a board from, as KiCad 10 writes them. JLCPCB, PCBWay and
OSH Park take KiCad's outputs as they are (OSH Park also takes the `.kicad_pcb`), so there are
no per-fab presets. The app zips the Gerbers and drills (`crates/app/src/fab.rs`).

## Entry points
- `Board::fab_files(stem)` — (name, text) of each Gerber and drill file, named as
  `kicad-cli pcb export gerbers` (Protel extensions) and `export drill --excellon-separate-th`
  name them: `<stem>-F_Cu.gtl`, `-In1_Cu.g1`…, `-B_Cu.gbl`, `-F_Mask.gts`, `-B_Mask.gbs`,
  `-F_Paste.gtp`, `-B_Paste.gbp`, `-F_SilkS.gto`, `-B_SilkS.gbo`, `-Edge_Cuts.gm1`,
  `-PTH.drl`, `-NPTH.drl` (the middle part is the board's name for the layer).
- `Board::gerber(layer, function)` — one layer; its X2 file function decides what shows.
- `Board::positions_csv()` — placed parts (`Footprint::placed`) in JLCPCB's CPL columns.
- `Board::bom_csv()` (`bom.rs`) — `Board::bom` lines in JLCPCB's BOM columns, with the part
  numbers `Footprint::part_numbers` finds in its fields (`MPN`, `LCSC`, `Manufacturer`…).

## Gerber X2
- mm in 4.6 format, which is the board's nm; Y up. Pads flash C/R/O apertures, else an outline
  macro (curves within 5 µm); fills are regions; tracks, graphics and text draw with round
  apertures, arcs as arcs (a growing angle on the Y-down board is G02).
- Mask (polarity Negative): pads on the layer, grown by their margin (the pad's, else its
  footprint's, else `pad_to_mask_clearance`); vias, grown by the board's, when not tented
  (`tenting`; KiCad <= 8: `viasonmask` in the plot settings). Paste: margin plus ratio × side.
- KiCad's rules, found by probing it: an even margin grows corners by it (a rectangle's round);
  margins that differ (paste ratios) keep a round rectangle's corner ratio and round a rectangle
  by the x one; a pad without copper or hole (an opening itself) takes no margin.
- Plated holes' pads are on every copper layer (`parse.rs`), as KiCad has them.

## Drills and positions
Excellon in mm, 3 decimals, a tool per size from the smallest, oval holes as G85 slots
(KiCad's alternate mode, which JLCPCB asks for); plated (vias, plated pads) and unplated files,
and one per span of blind or buried vias (`<stem>-front-in1.drl`, `-in2-back.drl`), as KiCad.
Positions: anchor in mm with Y up, angle in (-180, 180] as KiCad's `.pos`, side Top/Bottom.
Placed = not DNP, not excluded from position files; in the BOM = fitted (not excluded from it).

## Checked
- Unit tests in `gerber.rs`, `fab.rs`, `bom.rs`; the app's `fab` command end to end.
- Against `kicad-cli` (skill `fab-check`): on every corpus board, each copper, mask, paste,
  outline and drill file covers the same area as KiCad's to the pixel (10–20 µm), except text,
  whose Hershey glyphs differ a little (`font.rs`), and the shortcuts below. Positions equal
  KiCad's `.pos` on every corpus board, but for the DNP parts left out on purpose.

## Speed
All 17 files of kria-k26 (8.6 MB) in 24 ms; `cuprita --json kria-k26.kicad_pcb '{"cmd":"fab"}'`
takes 0.55 s in all (open 0.15 s, refill 0.27 s, then a 2.2 MB zip and the CSVs).

## Missing (`// shortcut:` in the code)
Chamfered corners and trapezoids (plot as their rectangle; one corpus pad), knockout text
(plain strokes), a custom pad's per-axis paste growth, a via's own tenting (KiCad 9+),
`remove_unused_layers` (rings stay on every layer, which the fill keeps clear of), X2 aperture
and net attributes, the Gerber job file, drill maps and IPC-D-356 netlists (no fab needs
them), silkscreen clipped by the mask (fabs do it).
