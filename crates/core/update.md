# crates/core: updating the board from its schematic (`update.rs`)

KiCad's Update PCB from Schematic (F8): the board takes what its project's schematic says.

`Board::update(&Project, footprint)` — first the parts the board lacks (by reference; parts kept
off the board, power symbols and unnumbered `R?` left out) come in from their Footprint field
(`footprint` gives a `Library:Footprint`'s `.kicad_mod` text), named, valued and linked to their
symbol (`(path …)`, KiCad's link) as the schematic has them, in rows below the board's outline
(else its parts), 5 mm down, a row as wide as the board: a sheet's parts together (sheets by
file, a sheet's next 3 mm down), each 1 mm clear of the next's courtyard (else its pads), its
anchor on a 0.1 mm grid, so none start out overlapping. Those it can't add are told
with what to do (`Updated::missing`: no footprint chosen, one not in KiCad's libraries).
Then every part, matched by reference (KiCad's "Re-link footprints to schematic symbols based
on their reference designators"), takes its symbol's footprint, value and fields, and its pads the
nets `Project::netlist` puts their pins on, by KiCad's names (`nets.md`); a pad without a pin gets
none, as on every corpus board KiCad updated. One undo step (`Patch::update`); it says how many
parts it added, footprints, values and fields it changed, nets it renamed and added, pads it moved.
- Footprints, values and fields of the parts on the board: `update-parts.md`.
- A pad goes on the net of its pin's name. A net goes whole to the schematic's net most of its
  pads go to, when that net isn't on the board: renamed, it keeps its id, so its tracks, vias
  and zones go along. Each name goes to the net with most pads for it (then the lower id), and
  not while a net keeping its name has it (then its pads move there).
- An `unconnected-(…)` net with several pads: KiCad 8 gives each pad a net of its own (`…_1`,
  `…_2`), KiCad 7 each pin; a pad on one of them keeps it, the others take free ones in turn.
- Saving finds a renamed net by its name in `Board::file_nets` and renames it there (`save.md`).

## Checked
- `update.rs` tests: a part added below the board, its pads on their nets, saved with its link;
  one without a footprint told; a label renaming a net with a track on it, two nets joined, a pad
  without a pin, nothing left to do after, undo; a no-connect net's pads apart.
- `tests/corpus.rs`: every corpus board updated from its project: no pad changes the pads it
  shares a net with; hackrf-one (KiCad 6) gets 272 nets renamed as KiCad 7 on names them,
  kria-k26 (KiCad 7) 35 (sheet names trimmed), glasgow-revD1 and jetson-orin none; glasgow-revD1's
  U9 takes the part number its symbol has since; then a resistor added to the schematic comes in
  below the board on its net; saved, it reads back the same parts, values, fields and nets;
  undone, it is as read.
- KiCad 10 (skill `kicad-check`) reads the saved boards' nets as Cuprita means them, every pad,
  track, via and zone, also with GND renamed (glasgow-revD1, jetson-orin: 1,835 and 2,005
  items), a pad put on a new net and one on none; KiCad's AMS1117-3.3 and a resistor given its
  0603 footprint, added to every corpus project: their reference, value, footprint, link to
  the symbol, place and nets.

## Speed
1.9–5.2 ms a corpus board with nothing to add (release), its netlist (0.6–1.7) and fields included;
in the app, kria-k26 (35 nets renamed) 52 ms, its redraw and whole re-check included (48 before fields).

## Missing
Matching parts by KiCad's link to their symbol (`path`) first, and renaming re-annotated ones;
`sheetname` and `sheetfile`; the DNP and BOM flags of parts on the board (new ones take their
symbol's: `flagged`); pads' pin names and types (`pinfunction`,
`pintype`); deleting parts the schematic lost (KiCad's default keeps them too).
