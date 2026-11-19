# crates/core: Tidy texts (`tidy.rs`, `save/texts.rs`)

Parent: `STATE.md`. KiCad lacks it: on a dense board the references pile on pads and on each
other (`unlocked` ones turn with their parts) and the Fab values cover the parts.

## The board (`Board::tidy_texts(parts)`: all when none is given; one undo step)
- A part's silkscreen reference (its first text on a `*.SilkS` layer showing its reference) goes
  upright (0° or 90°) at KiCad's 1 mm, 0.15 mm pen (JLCPCB's smallest; a bigger one keeps its
  own) to the first free spot round its courtyard's box (else `Footprint::bounds`): its box
  touching a side from outside, 0.1 mm off, sliding along it by 0.25 mm, up to 0.5 mm farther
  out; the nearest the part first, a level one before an upright one 1 mm nearer; off the
  courtyards of the parts on its side where a spot is, else over them. Free: 0.15 mm from solder
  mask openings (pads' with their margins, untented vias', drawn ones), 0.1 mm from the layer's
  graphics, texts and the references placed, 0.25 mm from the edge, inside the outline when it
  closes (but for a part wholly off it, as F8 adds them) (`drc::RTree`). The smallest parts go
  first: they have the fewest spots. None: hidden.
- Its value on its Fab layer: upright along its body's long side (its graphics there, else its
  courtyard, else its box), in the middle, below the part's reference on that layer when that
  lies in the body (it goes above): KiCad's line pitch (1.61 heights), shrunk to fit with a tenth
  of the short side clear, to 10 µm, never grown; under 0.1 mm the value is hidden.
- `Text::src` (its place among its part's `fp_text`s and `property`s in the file, hidden ones
  counted) and `Footprint::hidden` (its texts not shown, the file's too) are how saving finds them; `Patch::texts`
  puts a part's texts in place of its own (empty splices of its pads and graphics, as `apply`
  pairs a part put in with them). `Tidied` says what it did; a tidy board changes nothing.

## Saving (`save/texts.rs`)
- A text the board has otherwise than the file and its part's placement put it (over 10 nm or
  1e-4° off, another size or pen): its `(at X Y A)` rewritten (local, its angle in [0°, 360°),
  0 left out before KiCad 8), its `(size H W)` and `(thickness T)` (a list it lacks added); the
  part's turn and flip leave that place, a flip still turning its layer and mirror.
- A text an edit hid: `(hide yes)` on a line after its `(layer …)` from KiCad 8, ` hide` before;
  a `(hide no)` becomes `yes`; one the file hides, shown again, loses that list or word (in
  its `effects` too). The rest of the file stays byte for byte.

## Checked
- `tidy.rs` tests: a reference level above its courtyard, a gap off, clear of mask openings; a
  neighbour kept off; only the parts picked; a part off the board; no room: hidden, saved so;
  value and reference stacked in the body, a body too small; KiCad 10 and 6 text exactly as
  KiCad writes it; tidied, turned, flipped and moved, read back; undone, byte for byte; again,
  nothing moves.
- `tests/corpus.rs`: every corpus board tidied, saved and read back as tidied, also with every
  part flipped; undone, byte for byte.
- KiCad 10 (skill `kicad-check`, `GetFields()` and `PCB_TEXT`s: `GetTextPos`, `GetDrawRotation`,
  size, pen, `IsVisible`): the 7 corpus boards (KiCad 4–10) and thermal-dongle tidied, and then
  flipped, turned 33.3° and moved: every shown text (3,378) as Cuprita has it (within 1 nm), the
  hidden ones hidden.
- `kicad-cli pcb drc` on thermal-dongle: silk_over_copper 16 → 0, silk_overlap 20 → 0,
  silk_edge_clearance 4 → 0 (17 references placed, 4 hidden); on corpus boards none left names a
  reference (hackrf-operacake 199 + 199 → 183 + 77: its parts' own graphics).

## Speed (release)
kria-k26 (653 parts) 40 ms, jetson-orin 47, hackrf-one 27, thermal-dongle 0.5; saved as an edit.

## Missing
The schematic's texts (`Cmd::TidyTexts` is the board's for now); texts other than the reference
and the value (user texts, the Fab reference without a value).
