# crates/core: a part put on the other side (`board.rs`, `save/flip.rs`)

Part of `save.md`. `Board::flip_footprint(i)` does what KiCad's Flip (F) does, top to bottom
about the part's anchor (KiCad's default; its "Flip board items L/R" setting is off); `Patch::footprint`,
made before, undoes it.

## The board
- Anchor kept; the part's angle and its pads' negated; every item mirrored top to bottom, on
  the other side's layer: `F.*` ↔ `B.*`, copper in reverse stack order (In1 ↔ In(n-2) of n),
  others (`Dwgs.User`, `Edge.Cuts`) kept; a counterpart the board lacks (`B.Fab` of a KiCad 4
  board) is added, as reading it would. A custom pad's copper is mirrored in its own frame.
- Texts: at 180° − their angle, as KiCad (a mirrored text reads backwards along its angle);
  mirrored (toggled) only when their layer changes side; justification kept; locked ones upright.
- 3D models stay as written: KiCad shows a part's models on its side.

## Saving it (`Footprint::back` other than its `(layer …)`)
The file's text as KiCad 10 writes the flip, then a move and turn as for any part: its layer and
every item's (`layer`, `layers`, a zone's), local Y negated (`at`, `start`, `mid`, `end`,
`center`, `xy`, a hole's `offset`, a trapezoid's `rect_delta`), pad angles negated, text angles
180° − angle, `(justify … mirror)` toggled on sided layers (`(justify mirror)` added after the font,
spaced as it, and taken out with the space before it), chamfered corners top ↔ bottom, KiCad 5
arcs' `angle` negated, a footprint zone's corners (board coordinates) mirrored about the anchor.
KiCad leaves a 0° angle out, but for texts from KiCad 8 on: so does the flip. Unchanged: 3D
models, text boxes, dimensions and barcodes (`// shortcut:`). KiCad also swaps an arc's start and
end and sorts the items; the text written is the same shape.

## Checked
- `save/flip.rs` tests: glasgow-revD1's D80 written exactly as KiCad 10 flips it; every kind of
  item above, flipped and turned; flipped twice, the text as read (also saved, read and flipped back).
- `tests/corpus.rs`: every part of every corpus board flipped reads back as flipped (custom pads'
  copper within 0.01% of the area: its strokes mirror within nm); flipped back, byte for byte.
- KiCad 10 (skill `kicad-check`, oracle pcbnew's `FOOTPRINT.Flip(pos, FLIP_DIRECTION_TOP_BOTTOM)`)
  reads every part of every corpus board (KiCad 4–10) flipped, and flipped, turned 90° or −33.3°
  and moved, as its own: placement, side, pads (place, angle, layers, hole offset, chamfers,
  copper), texts (place, angle, layer, mirror, justification), graphics, zones, 3D models:
  ~443k items, 0 differ (2 nm, 0.001°; `*.Cu` compared on the board's copper).

## Speed (release)
Flipping every part of kria-k26 (660) 1 ms; saving them all 193 ms (a part: as a move).
