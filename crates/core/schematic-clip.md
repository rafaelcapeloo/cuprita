# crates/core: mirroring, copying and pasting on a sheet, and its page

Part of `schematic-edit.md`. Each is an edit of a sheet's text that undoes itself (`Patch`).

## `Project::mirror_items(page, items, up_down)` (`schematic/placed.rs`)
- Left to right (KiCad's Mirror Horizontally) or top to bottom, about what `turn_items` turns
  about: a lone symbol's origin, a lone label's point, else the grid point nearest their middle.
- A symbol takes the angle and `(mirror x|y)` that make its frame the mirror of what it was
  (`Placed::mirrored`, by `symbol::turn`): a mirror given, taken or changed where that does it,
  else half a turn more (mirrored both ways is a half turn, as KiCad writes it). Its fields go
  to its other side. A label reads away from its point still: half a turn along its line.
- Made in place, as KiCad's: a pin that takes the place of another pin of what is mirrored
  takes what is wired there (a diode turned round, an amplifier's inputs changed over) and no
  wire moves. A pin going elsewhere draws its wires after it as a move does (`reshape`), and a
  label on it goes along, turned to read away from the symbol's other side.

## `Project::copy_items(page, items)`, `Project::paste(page, clip, at)` (`schematic/clip.rs`)
- A clip is KiCad's own clipboard text, which KiCad pastes too: `(lib_symbols …)` with the
  library symbols used (when any), then each item as its sheet writes it, and the junctions the
  wires among them need.
- `paste` takes ours or what KiCad copied: symbols, wires, buses, entries, junctions, labels,
  flags, texts and graphics (a sheet's box is left out). The corner of their places goes to the
  grid point nearest `at`; every `(uuid)` is made anew; a symbol is numbered on from the
  project's highest of its kind, in each place its sheet is used (`instances`, or a KiCad 6
  root's `symbol_instances`), a part's units keeping one number; the library symbols the sheet
  lacks go into its `lib_symbols`.
- What lands joins what is there, the user's own doing as in KiCad, and gets the junctions it
  then needs: a wire's end or a pin on a wire's middle, three ends (`needs_junction`, on the
  clip read as a sheet of its own).
- Returns the undo patch and the items pasted, as the page has them: the app picks them.

## `Project::page_setup`, `set_paper`, `set_title_line` (`schematic/page.rs`)
KiCad's Page Settings: the paper (`PAPERS`, standing with `portrait`), written after the
sheet's head when it has none, and the title block's lines (`LINES`: title, date, rev, company,
comments by number) in KiCad's order; a line emptied goes, and the block with its last.

## Checked
- `placed.rs`: a symbol mirrored each way and back byte for byte, both ways (a half turn), its
  pins changing places in place; a label alone, a label on a pin that changes sides; a group
  with its wires, nets kept; undone byte for byte.
- `clip.rs`: a part with its wiring copied and pasted beside it (numbered on, ids anew, the
  same nets; a junction where a pasted wire lands on a wire's middle); a part's two units into
  another project's empty KiCad 6 sheet, as one part with its library symbol; pasted twice.
- `page.rs`: paper and lines set, ordered, emptied, undone; a sheet without a paper.
- In the window (skill `look`): X, Ctrl+D, Ctrl+C and Ctrl+V on a sheet, a title typed.

## Missing
Mirror and turn of what is being pasted; a clip's sheets (boxes); KiCad 9's variants and
groups in a clip are pasted as written. Pasting isn't tried in KiCad itself here (no window to
drive): the text is laid out as `SCH_IO_KICAD_SEXPR::Format` writes a selection.
