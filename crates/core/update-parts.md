# crates/core: F8 on the parts already on the board (`update.rs`, `save/fields.rs`, `save/swap.rs`)

Part of `update.md`. Each part the schematic has, matched by reference, takes what its symbol
(its first unit) says, as KiCad's Update PCB from Schematic does by default.

## Footprints
- A symbol whose Footprint field names another `Library:Name` (not empty) swaps its part for that
  footprint from KiCad's libraries, in its place: same anchor, angle and side (flipped onto the
  back with `flip_footprint`), reference, value, fields (KiCad 8's Footprint field names the new
  one), link to the symbol and attributes (DNP, BOM and position-file flags; SMD or through-hole
  too: `// shortcut:` none of the corpus swaps changed it); the library's pads, graphics, zones,
  description and 3D models. Its pads take the nets of their pins.
- Decision: the part keeps every text as placed (reference, value, fields and its own texts: place,
  angle, layer, size, shown or not), as KiCad's Change Footprint keeps them unless told to reset:
  users place references by hand; the library's texts don't come in. A part added from a library
  this session (never placed by hand) becomes the new one whole.
- One it can't swap stays, told in `Updated::missing` (not in KiCad's libraries, not readable).
- The undo keeps every part, pad, text and net, and the graphics from the first part swapped on
  (`Patch::update`); `Patch::part` puts one in place of another, the parts after it moving along.
- Saving (`Src::Swapped`): the file's text of the part keeps its own lists (`layer`, `at`, ids,
  `path`, sheet, fields, texts, `attr`) and takes the library's others, placed into the file's frame
  (as an added part is placed), after its own lists, spaced as they are; then it is placed as the
  board has it, its pads' nets and fields written.

## Values and fields
- The value; and fields as each KiCad version keeps them on its boards (seen on the corpus boards
  it updated): KiCad 5 none; from 6 a symbol's own fields (MPN, LCSC, Sim.*…) but Reference, Value
  and Footprint; from 8 its Datasheet too. A field the symbol lacks stays (KiCad's "Remove extra
  fields" is off). `~` (KiCad 6–7's empty field) is empty; KiCad 9's `(property private …)` is the
  library's note on the symbol, which no board gets. Parts added by the same update take them too.
- Saving rewrites a value or field where the file has another, and adds a new field after the
  part's last field (else its `(at …)`): KiCad 6–7 `(property "MPN" "…")`; from KiCad 8 hidden on
  the part's side's Fab layer at its anchor, turned with it, mirrored on the back, unlocked, a line
  per list, as KiCad 10 adds one.
- The part's texts that showed a changed value show the new one (a text composing it with other
  words, `${REFERENCE}: ${VALUE}`, keeps the old until the board is read again: `// shortcut:`).
- `Updated::{footprints, values, fields}` count them; the undo step and the agents' `update` say them.

## Checked
- `update.rs` tests: a turned part on the back swapped for a wider one, its texts, fields and
  attributes kept, its pads on its nets, the saved text read back, undone; one KiCad lacks, told; a
  value and fields changed and added on KiCad 7 and 10 boards, the saved text exact, read back.
- `tests/corpus.rs`: a resistor on each side of each corpus project swapped (texts, fields, nets of
  its pads as before; saved, read back; undone, as read); glasgow-revD1's U9 takes the part number
  its symbol has since (W25Q32JVZPIM), the other boards none; saved, every value and field reads back.
- KiCad 10 (skill `kicad-check`, oracle `pcbnew.FootprintLoad` placed, flipped for the back): resistors
  swapped for `Resistor_SMD:R_0805_2012Metric` on hackrf-one, kria-k26, jetson-orin, glasgow-revD1
  (KiCad 6, 7, 8, 10; front and back, turned 45°–180°): FPID, place, side, pads (place, angle,
  layers, size, nets as before), graphics, 3D model as KiCad's own, the reference where it was,
  fields and attributes kept; a value, a part number with quotes and an LCSC number set, updated
  and saved on the same boards: as set, the new fields hidden on F.Fab.
