# crates/app: the schematic view (`sheet.rs`, `sheet_edit.rs`)

What it does, with its keys: `features.md`; the core side: `crates/core/schematic.md`; editing
it (`sheet_edit.rs`): `sheet-edit.md`.

## Entry points
- `sheet.rs` — schematic view (Ctrl+E): `App::{toggle_sch, open_sch, canvas_sch, label_field,
  sheet_path, erc_tab}`; `Sch` holds the `Sheets` read (`read_project`) from the root sheet beside the board
  (or `root_of` a `.kicad_sch`: the folder's `.kicad_pro`'s), its ERC violations (`D` lists them),
  the page shown, its scene, marks and status, the symbol being placed and the wire tool; each
  view keeps its zoom and marks. `Sch::picked` is what a click (`Schematic::item_at`; Shift adds or
  takes away), a box (Shift+drag on empty paper: `pick_box`, `items_in`) or `find_sch` (a part's
  symbols) picks, `Sch::part` its part when it is one. `Sch::lit` is the net lit, by its name, and
  where it is (`Project::net_items`): a click on a wire or label `light_net`s its net
  (`Project::net_of`), the status naming the sheets it reaches. `App::probe` (Ctrl+E) selects
  the picked part on the board (`find_target`), or the board's `App::selected` part here, and
  lights the net lit there here (`App::net` by name), showing it when no part is shown, unless
  the view shown already does. `App::update_board` (F8, the
  buttons): `Board::update` from the schematic, footprints by `footprint_text` from `App::parts`,
  the parts added in view; from the schematic, its board first (`open_board`, as Ctrl+E);
  `board_sch` reads the schematic beside the board without showing it.
- `sheet_edit.rs` — `Sheets`, the schematic as edited (undo, save, autosave), and the editing
  tools: `sheet-edit.md`.
- `file.rs`: `App::unsaved` (the board's edits or the schematic's) is what leaving and closing
  ask about; Save there saves both.
- `sim.rs` — simulation (S): `sim.md`.

## The page's colours
`page_scene` is `build_sch` as the window shows it: KiCad's inks on its light paper, or
(`theme::dark_sheet`, Settings' Sheets, kept as `sheet`; the default) the same inks made for a
dark page; the helpers draw in the board's colours there (`helpers::on_sheet`).

## Missing
Picking or moving buses, bus entries, junctions, sheets; the browser (no schematic view there).

## Gotchas
- The first A in the schematic lists KiCad's symbols on the UI thread: 46 ms, 82 ms from a cold
  disk cache (NVMe).
- A preview step (the pointer on another grid point, or R) reads the edited sheet's pages twice
  and draws the page again: at most 9.4 ms on the corpus projects (glasgow-revD1's level shifters,
  4 places), a label's 8.3 ms; a drag's step about 10 ms there (7 ms on hackrf-one's mcu sheet);
  with a net lit, it is found again (≤ 2 ms more).
