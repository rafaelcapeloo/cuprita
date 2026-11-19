# crates/core: the board's own texts and drawings (`drawing.rs`)

What a board draws that is no part's: the words and lines of its silkscreen, notes on its user
layers, its edge. Parent: `STATE.md`; picked and edited together with the rest: `history.md`.

## What it is
- `Board::own()`: the places in `shapes` and `texts` outside every part's range. The file mixes
  them with the parts' (a `gr_text` between two footprints), so nothing assumes an order.
- `Pick::Text(i)`, `Pick::Shape(i)`: one of them picked. `Board::drawing_at(p, tol, shown)` finds
  it (a text by its box, a drawing by its line, a filled one inside too) on the layers shown;
  `items_in` takes them in a box; `mark` is the line a pick is shown by; they move, turn and go
  with the other picks (`picked.rs`). A text turned keeps its angle in [0°, 360°).
- A part's text (`Board::part_text_at` finds it, `text_part(i)` names its part) is picked
  alone: it moves, turns (a locked one stays upright) and is sized by `set_text` as the board's
  own; `delete_items` hides it (`Board::set_texts`, as `tidy.md` hides one); picked with its
  part it goes as the part does. A box takes the part, not its texts. Saved: `tidy.md`.
- `Board::drawing_layers()`: where a new one may go: any layer but copper (DRC and zone fills
  don't take texts or lines there yet), Edge.Cuts (the Outline tool's), Margin and the
  courtyards. Those a file has on copper or the edge are picked, moved and deleted all the same.
- `Board::ink(layer)`: what a new line or text gets, from the project's `defaults` (`silk_*` on
  a silkscreen layer, `other_*` elsewhere: line width, text size and pen); without a project
  0.15 mm lines and 1 mm letters with a 0.15 mm pen on silkscreen, what a fab prints.
- `Board::{new_text, add_text, set_text, add_lines, set_shape}`: a text at a point (mirrored on
  a back layer, to read from that side), one put in place of another (its words `typed` when
  they changed), lines from corner to corner (closed back to the first or not; a corner given
  twice draws none). New ones go after the board's last, so parts' ranges stay.

## Saved (`save.rs`, `save/texts.rs`)
- A text the file has (`Text::src`: its place among the `gr_text`s shown) is edited where it
  is: its `(at)`, angle, `(size)`, `(thickness)`, layer name, `mirror` in `(justify)` (added in
  `(effects)` when it has none), and its words when typed. Its id, face, `locked` and
  `render_cache` stay: KiCad 10 draws a moved text of a TrueType face where it is now (checked:
  its cache holds the old place and is not used).
- A text read with `${VARIABLES}` shows them filled in; typed over, the words typed are saved.
- New texts follow the file's last drawing or text, as its KiCad writes them: 8 on a line per
  list with `(uuid)`, 6–7 on one line with `(tstamp)`, 5 bare.
- Drawings are matched by value as tracks are: one moved is written anew (a new id). Lines,
  circles, polygons (a `gr_rect` read comes back a `gr_poly`), arcs by three points, in KiCad 5
  by centre and an angle in tenths of a degree (its centre worked out from the chord, so it
  reads back within 2 nm); `fill` as `yes`/`no` from KiCad 9, `solid`/`none` before, none in 5.
- KiCad 6 and 7's `(gr_text locked "T" …)` is read as the text `T` (it read as "locked").

## Checked
- `drawing.rs` tests: own items among parts' found, picked by point and box; texts and lines
  added, a text, a part and a line deleted at once and all undone exactly; saved in KiCad 8, 6
  and 5 text and read back the same; an unedited board byte for byte.
- KiCad 10.0.8 (`kicad-cli pcb drc`, `export svg`): a board saved with a new two-line text and a
  rectangle, then with the text retyped, put on B.SilkS and made 2 mm high in place, loads with
  0 violations and plots them where Cuprita draws them, the back one mirrored.
- In the window (replay): Shift+T, the words, Enter; drag, R; L with clicks and a dragged
  rectangle; Inspect's fields; Ctrl+S.

## Gotchas
- `Board::apply` moves the ranges of the parts a patch didn't put in by where its splices lie
  (`span`), not by pairing splices with parts: a patch may take a part and a text of the board's
  at once. A part without items keeps its place after the items of the parts before it.
- A text typed here shows its `${VARIABLES}` as typed until the board is read again.
- Dimensions, text boxes and images are not read: they stay in the file as they are.
- A part's text keeps its words and layer (saving writes neither). Its hidden ones, the file's
  and an edit's, wait in `Footprint::hidden` where they would be: `Board::show_text` shows one.
