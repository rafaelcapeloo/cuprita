# crates/core: a part made from a few numbers (`newpart.rs`, `newpart/footprint.rs`)

`newpart::create(dir, project, &Part)` — a part of one's own into the project's library as an
import puts a vendor's (`parts.md`): its symbol, linked to its footprint, and a footprint made
for it. `Part`: name (and value), reference prefix, description, datasheet, `Pin`s (number,
name, one of KiCad's `TYPES`, `Side`), and a `Footprint`: a library's `lib:name`, or `Made`
from a `Package`. `pins(n)` numbers n from 1, half left, half right. `Part::check` says what to
fix: a name (no spaces, `: / \ "`), a prefix of letters, pins of their own numbers, a made
footprint's pads 1 to n those numbers (an exposed pad the last pin), the package possible.

## Symbol (`Part::symbol`, `Part::library`), as KiCad 10 writes one
- Pins 2.54 mm long, 2.54 mm apart, from the top down each side and centred across the top and
  bottom, their ends on the 50 mil grid; the box sized to the names inside it (KiCad's font,
  `font::width`, 0.508 mm in), top and bottom names in their own bands, edges on the grid.
- Reference above, Value below (past top and bottom pins), Footprint, Datasheet and
  Description hidden; units `<name>_0_1` (the box, background fill) and `<name>_1_1` (pins).

## Footprint (`Package::footprint(n)`, `PRESETS`), as KiCad 10 writes a `.kicad_mod`
- `Kind`: `Dual` (SMD rows, 1 top left, counter-clockwise), `Quad` (four sides), `Header`
  (through-hole, 1 or 2 rows numbered across), `Dip`. Pitch, span, pad, body, drill and an SMD
  exposed pad (heatsink, joined solid, its paste in squares 80% of their cells).
- Presets, KiCad's library numbers for each pad count: SOIC 1.27, TSSOP 0.65, DFN (exposed pad),
  QFN 0.5 (exposed pad, body n/4·0.5+1 mm), LQFP 0.5 (KiCad's body sizes), pin headers 2.54 in
  1 or 2 rows, DIP 7.62. Names as KiCad's: `QFN-16-1EP_3x3mm_P0.5mm_EP1.7x1.7mm`.
- SMD centred on the body, through-hole from pin 1 (KiCad's libraries). Roundrect pads (a
  quarter, at most 0.25 mm); THT pad 1 square. Courtyard: a box 0.25 mm round pads and body,
  out to a 0.01 mm grid. Fab: the body, pin 1's corner cut (a quarter, at most 1 mm), and
  `${REFERENCE}`. Silkscreen: the body 0.11 mm out, kept 0.2 mm (and half its line) off the pads:
  SMD lines cut where pads and their leads come, only the corners' pieces kept; THT edges moved in
  past the pads. Pin 1: a triangle pointing at it from outside. Reference on silk above, Value
  on fab below.
- Refused, saying how to fix it: sizes of 0, pads closer than 0.1 mm (or overlapping), a ring
  under 0.1 mm, a pad count the family can't lay out.

## Checked
`newpart.rs` tests: the SOIC-8 and QFN-16 presets give KiCad 10's library footprints' pads,
courtyard, silkscreen and fab outline to the micron; every preset at 4–48 pads reads back in
Cuprita's board parser; a part placed on a sheet has its pins where the layout says, each its own
net; the reasons it refuses. KiCad 10: `sym`/`fp export svg` plot all 8 presets' parts, and
`pcb drc` of a board holding the 8 footprints finds 0 violations (all severities, library checks
too). In the window (skill `look`): made, placed (Enter, a click) and brought to the board (F8).

## Missing
One unit, one body style; quads square, as many pads each side; no BGA, SOT or chip (R/C)
families (pick KiCad's footprint for them); a courtyard box, not KiCad's stepped outline; no 3D
model (a vendor's STEP: Inspect's 3D › Choose…).
