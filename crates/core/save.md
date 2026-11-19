# crates/core: saving (`save.rs`)

`Board::write(src)` gives the board back as KiCad text. `src` must be the text the board was
read from: the app keeps it for the session, so every save writes all edits into it.

## What changes
- Nothing else: an unedited board writes `src` byte for byte.
- A moved or turned footprint: its `(at X Y A)`; turned, every pad, `fp_text` and `property` angle
  turns along (absolute in the file); a footprint `zone`'s corners move and turn about the anchor
  (board coordinates). Positions inside a footprint are local and stay; tidied texts: `tidy.md`.
- A footprint on the other side: its text as KiCad's Flip writes it (`save-flip.md`). F8's: a value or field rewritten
  where the file has it, else added; a part swapped (`Src::Swapped`): its text with the library's lists (`update-parts.md`).
- Angles as KiCad writes them: footprints in (-180°, 180°] (KiCad 4–5: [0°, 360°)), pads and texts
  in [0°, 360°), at most 6 decimals; a missing angle is added only when it isn't 0; a footprint without `(at)` gets one.
- A deleted footprint: its text goes, with the whitespace before it. The board's footprints are
  found among the file's by `Footprint::src` (`Src::File`), their place there.
- A part added from a library (`Src::Library`, `library.md`): its text, placed as a moved one,
  after the file's last footprint (else with the new tracks), a level in (a tab from KiCad 8).
- Tracks and vias: the file's are read as the parser reads them (layers and nets matched to the
  board's by name), matched to the board's by value, as many of each as the board has; a via it
  moved (a shove) keeps its text but `(at)`. The rest of the file's go, with the whitespace before
  them; the board's unmatched ones (a moved track: a new id) are new, after the file's last track
  or via (else last item), as its KiCad version does:
  KiCad 4–5 (`version` < 20211014) one line, bare layer names, no id; 6–7 one line, quoted names,
  `(tstamp UUID)`; 8 on (≥ 20240108) a line per child, tab indents, `(uuid "UUID")`.
- Nets by number from the file's `(net N "name")` lines; KiCad 10 files have none and name the
  net (`(net "GND")`, left out for no net). Vias name the outermost copper layers they span; a new one that stops inside the board is `blind`, from KiCad 10 (20250926) `buried` between inner layers.
- A pad on another net: its `(net …)` rewritten, put after its `(layers …)`, or taken out for none.
  A net the board renamed (found by its name in `Board::file_nets`) is renamed where the file names
  it: its declaration, its pads, KiCad 10's tracks, vias and zones, earlier zones' `net_name`. Nets
  new to a file that numbers them get the next numbers, declared after its last.
- New ids are version 4 UUIDs hashed from the item, its index and the text length: writing twice
  gives the same text, and items added in other sessions get other ids.
- Zones refilled (new fills), edited or moved (outline, settings), deleted or added: `save-zones.md`.
- The board's own drawings (`gr_line`, `_arc`, `_circle`, `_rect`, `_poly`; not its parts'): matched
  by value as tracks are, the file's it lost go, its new ones follow the file's last drawing (else its
  parts): KiCad 5 bare, 6–7 `width` and `tstamp`, 8 on `stroke`. Its own texts: edited where they are (`drawing.md`).

## Checked
- A new outline (`outline.rs`) on corpus boards of KiCad 4, 6, 7, 8 and 10, a new project and a KiCad 5
  board: KiCad 10 loads it closed, of the area drawn, cutouts kept, everything else as it was.
- `tests/corpus.rs`: every corpus board writes byte for byte unedited; after its first track and
  last via go, its biggest part moves and turns, a track and via are added and its middle part
  is deleted, the text reads back to the same board (nets compared by name).
- `save.rs` tests: a net renamed, one added, a pad given one and one none, in KiCad 7 and 10 text.
- KiCad 10 (skill `kicad-check`) loads every edited corpus board as its own move/turn would have
  them: placement, pads, texts, the keepout zone of jetson-orin's BT1; new tracks and vias
  with their nets, layers and sizes; with a track and via removed (and the track put back after
  that save), exactly those differ from the original; with the first and middle parts deleted,
  every footprint (placement, pads, nets), track, via, zone and drawing is as KiCad's delete.

## Speed (release)
kria-k26 (54 MB) 72 ms (100 with every zone refilled), jetson-orin 50, glasgow-revD1 15, hackrf-one 9.

## Gotchas
- Anything put back after a save (an undo) is still in `src`: its text comes back as it was.
- KiCad 10 files number nets where first used: deleting a part can renumber nets on reading back.
- A footprint's children may come before its `(at)`: they are collected first, edited after.
- KiCad gives a track with no net, or one overlapping another net's pad, the net of the pad it
  touches when it loads the board: start test tracks on a pad of their own net.
