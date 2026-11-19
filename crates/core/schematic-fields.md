# crates/core: a part's fields, their values and places (`schematic/{fields, autoplace}.rs`)

Part of `schematic-edit.md`: each change is a patch that undoes itself.

## Values (`fields.rs`)
`Project::fields(part)` — its first unit's fields and its library copy's `ki_fp_filters`;
`Project::set_field(part, name, value)` — set on every unit, every sheet (a sheet used twice is
edited once); a field the symbol lacks goes in after its others as its Footprint field sits
(hidden in KiCad's libraries; `(id n)` numbered on in KiCad 6–7); KiCad 6 roots' `symbol_instances`
value and footprint follow. Checked with KiCad 10: its BOM reads the new Value and MPN, the
sheet differs only there. Symbols keep their text (`Symbol::span`) and library (`lib_id`).

## Places (`autoplace.rs`)
- `beside`: a symbol's visible fields stacked, reading across the sheet, 1.27 mm off and a line
  apart, on the grid, where they meet no item (nor, when added, a text or another new unit): on
  a side without pins (right, top, bottom, left), else past the pins on the top or bottom, else
  right or left, nearest first, up to 8 steps of 1.27 mm out; else the first of those sides.
- `add_symbol` places a new symbol's so (a power symbol's stay where its library puts them);
  `Project::tidy_fields(page)` (the app's T on a sheet) every symbol's but power symbols' and
  fields marked Do not autoplace, one undo, leaving out fields already in place, with how many
  symbols' moved: 8 ms on the largest corpus sheet (jetson-orin `usb.kicad_sch`).
- Written as KiCad keeps them: the place on the sheet, the angle and `justify` in the symbol's
  frame (`symbol::shown` run backwards: the one of 0°/90° and 9 justifications showing it so).

## Checked
`autoplace.rs` test: a standing and a lying resistor, a side a label takes, a hidden field left,
undo byte for byte. thermal-dongle tidied and plotted by `kicad-cli`: every field where Cuprita
draws it, none over another item.

## Missing
`fields_autoplaced` (KiCad places such fields again as their symbol moves) isn't written;
`tidy_fields` avoids the stacks it has placed, not fields it leaves (power symbols', fixed ones).
