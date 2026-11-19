# crates/core: editing schematics (`schematic/edit.rs`)

A project's sheets are edited as their text, unedited text byte for byte: `Project` keeps each
sheet file's (`Project::text(file)`, what saving writes). A `Patch` replaces byte ranges in sheet
files (`Patch::files`); `Project::apply(patch)` makes it, reads the pages of those files again
(the whole project when their sheet boxes open other pages) and returns the patch that undoes
it (each edit keeps what it replaced, ROADMAP decision 3); `history::History` keeps them
(`Edited`). Adding wires and labels: `schematic-wiring.md`; sheets, and hierarchical labels'
pins: `schematic-hier.md`; picking, moving, turning, deleting symbols, wires and labels:
`schematic-move.md`; mirroring, copying, pasting, the page's paper and title block:
`schematic-clip.md`; a part's fields, their values and places: `schematic-fields.md`.

## Adding a library symbol (`Project::add_symbol(page, lib, text, at, angle)`)
- `text` is the symbol as `library::symbol` gives it (`library.md`). Its copy goes into the
  sheet's `lib_symbols` as `Lib:Name`, in name order; a copy so named already there is used.
- Its origin on the nearest point of KiCad's 50 mil connection grid (`on_grid`), turned 0–270°;
  every unit (an op amp's power unit too), in a row a grid step apart, as far as bodies and pins
  reach, plus 15.24 mm (`ROOM`) a side whose pins face the gap, for their nets' names.
- Reference: the next free number of its prefix in the project (`R5`; power symbols and flags
  `#PWR012`, as KiCad numbers them), one for each place its sheet is used.
- Its fields are the library's but keywords and footprint filters, their angles in its frame (as
  KiCad keeps them); the visible ones beside it (`schematic-fields.md`), a power symbol's where
  the library puts them; its flags the library's.
- KiCad 7+ sheets get its references in the symbol (`instances`: project, path, reference,
  unit); KiCad 6 projects a `path` per place and unit in the root sheet's `symbol_instances`.
- A patch that would leave a sheet unreadable is undone, and the error says where (a bug).

## Checked
- `edit.rs` tests: numbering, grid, turned pins and fields, the copy kept or put in order,
  units in a row, KiCad 6's root list, a sheet used twice, undo byte for byte, errors.
- `tests/corpus.rs`: a resistor added to every page of the four corpus projects (KiCad 6, 7, 8,
  10) shows there with a reference not used before, and a wire round it joins its pins in a net
  a label on it names (global on every other page); all undone, every file is as read.
- KiCad 10 (skill `erc-check`): `Device:R`, `power:GND` and the derived, three-unit `LM358`
  added to every page of the corpus projects, turned 0–270°: KiCad's netlists equal Cuprita's
  (812, 419, 891, 994 nets), its ERC too but for `erc.md`'s differences; its plots draw every
  added text where Cuprita does (skill `sch-check`).

## Speed
An add takes 1–3 ms on the corpus projects (release), most of it reading the edited pages again;
taking one away and adding it a grid step on, as placing it does, 9.4 ms at worst with the page
drawn (glasgow-revD1's level shifters, used in 4 places).

## Missing (`// shortcut:` in the code)
Body styles but the first; a project's own connection grid; pins' `uuid` lists (KiCad writes them when it saves); a symbol
from KiCad 10's libraries goes in as KiCad 10 writes it, also into older sheets (only KiCad 10
reads those then); renaming a reference.
