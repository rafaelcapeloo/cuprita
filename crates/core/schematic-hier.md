# crates/core: a project's hierarchy edited (`schematic/hier.rs`)

Part of `schematic-edit.md`: sheets added, and the sheet pins hierarchical labels bring. What
they join: `nets.md`.

## Hierarchical labels (`Project::add_label` with `LabelKind::Hier`, `schematic-wiring.md`)
- Not on the root (nothing is above it). Each box showing its sheet (every place it is used)
  gets the pin of its name and shape, as KiCad 10 writes one, unless it has one (`sheet_pins`):
  after its last pin, on its left side for an input, else its right (the other side when
  full), at the first grid point down from the top 100 mil from the side's other pins; refused,
  saying so, when both sides are full. One patch, so one undo step for both files.

## Adding a sheet (`Project::add_sheet(page, name, file, a, b, existing)`)
- A box from corner to corner (on the grid; not flat) as KiCad 10 writes one: its flags,
  stroke, fill, id, Sheetname and Sheetfile fields where KiCad 10 places them (measured:
  0.0766 mm + half the text size above, + 0.4 of it below), and a page number after the
  project's highest for each place its page is used, in its `instances` (KiCad 7+) or the
  root's `sheet_instances` (KiCad 6, the place's sheet ids below the root).
- Its name is one line, unique on its page; its file ends in `.kicad_sch`, beside the root,
  not the page's own nor one above it (it would show itself).
- The file: the project's text of it, else `existing` (what the caller read: KiCad's "use an
  existing sheet"), else a new sheet, empty, as KiCad 10 writes one (`generator "cuprita"`), its
  paper and title block the root's. A sheet shown again: the box gets a pin for each of its
  hierarchical labels, as `sheet_pins` places them; in each new place, the pages under it
  (`number_places`) give each part a reference of its own, numbered on from the project's
  highest of its kind (a part's units alike; `R?` stays), and each box a page, as KiCad's
  annotation does: entries in the symbols' and boxes' `instances` for the project (`item_slots`
  finds them, or where the lists go), KiCad 6's in the root's lists.
- The new file goes into the project's texts (empty, then the patch's text): undone, its text
  is empty again, which saving and autosaving skip (`Sheets::save`).
- `Project::reread` reads the whole project again only when a page's boxes open other pages:
  a box's place or pins changing reads its page alone.

## Checked
- `hier.rs` tests: labels' pins on each box (sides, spacing, full, a sheet used twice, the
  root refused), sheets added (text, new file, nested, existing, KiCad 6 root, mistakes, undo),
  a sheet shown again numbered there (KiCad 7+ and 6).
- KiCad 10 (skill `erc-check`): a resistor and a hierarchical label on its pin on every sheet
  below the root, each box's new pin wired to a resistor: KiCad's netlists equal Cuprita's (388,
  856, 818 nets; glasgow-revD1's but 4 resistors that landed on unlabelled bus entries,
  `nets.md`), each label joining its boxes' resistors; its ERC adds what Cuprita's does (those 4
  pins aside); its plots draw the 20 labels' and pins' texts within 0.06 mm of Cuprita's. A new
  sheet on each corpus root, one in it, and the first shown again, each with a label wired to
  its box's pin: KiCad reads them (no annotation errors), its netlists equal Cuprita's, names
  too (609, 390, 815, 802 nets), its ERC adds what Cuprita's does, its plots draw the boxes,
  names and pins where Cuprita draws them (file names' glyph advances aside).

## Speed
A label with its 4 boxes' pins 4.9 ms (glasgow-revD1's level shifters; jetson-orin's 1.3 MB usb
sheet 7); a sheet added 10–12 ms on glasgow-revD1, the whole project read again.

## Missing (`// shortcut:` in the code)
A new sheet pin's place along the box (KiCad hands it to the pointer); its top and bottom edges;
a box growing to take more pins. A sheet file from outside the project, shown, keeps the
references it brings.
