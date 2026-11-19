# crates/app: editing the schematic (`sheet_edit.rs`)

Part of `sheet.md`; the core side: `crates/core/schematic-{edit, wiring, move}.md`.

## Entry points
- `Sheets`: a project's schematic as edited (`read`, `push`, `undo`, `save`: each
  edited sheet through `save_file`, `<file>-bak` first), the app's and agents' (`agent.rs`);
  `autosave` writes the edited sheets to KiCad's `_autosave-<file>` (`App::autosave`, 5 s after
  `Sch::changed`, not while placing), `autosaved` finds newer ones: `load_sch` asks
  (`Ask::RestoreSheets`) and `read` takes them as unsaved edits.
- Symbols: `App::pick_symbol` (A, then a symbol of
  `App::symbols`, `library::symbols` read on first use) starts placing it (`place_symbol`, with the
  fields a Digi-Key part sets: `lookup.rs`, the add panel's row for a part number, `look_up` on a
  thread, `poll_lookup` each frame: `Part::kicad` then placing, else the row says what went wrong); `App::show_placing`
  shows it at the pointer's grid point (`on_grid`): the patch of the one shown undone,
  `Project::add_symbol` again; `turn_placing` (R), `put_down` (a click: its patch goes to the
  history and ERC runs again: `Sch::edited`).
- Mirror, copy, paste, duplicate (`mirror_picked`: X, Y; `copy_picked`: Ctrl+C, the clip kept in
  `App::clip` and put on the clipboard; `paste_clip`: Ctrl+V, the clipboard's text when it is a
  clip, else the last copied here; `duplicate_picked`: Ctrl+D, the clipboard left alone): a clip
  is placed as a symbol is (`Placeable::Clip`, `Project::paste` again at each grid point) and
  picked once put down. Copy, Cut and Paste reach the sheet as egui's clipboard events
  (`sheet.rs`), in the browser the page's own. `page_settings`: Inspect's Page with nothing
  picked (`Project::page_setup`, read again after an edit or on another page).
- Wiring: `wire_tool` (W or the button, on and off; B for buses: `Sch::bus`; a click starts one),
  `wire_click` (`Project::add_wire` or `add_bus` from where the wire goes on from, bent once by
  `wire_path`, the other way with `Sch::swap_bend` (`/`), straight with Shift; ends where `Schematic::joins`, `joins_bus`), `draw_wire` (what the next click
  adds, egui lines); `place_entry` (Z: bus entries placed as symbols are, one after another,
  `Project::add_bus_entry`; `Placing::entry`);
  `no_connect_tool` (Q or the button, on and off), `flag_pin` (a click flags or unflags:
  `Project::toggle_no_connect`; `Schematic::pin_at` names the pin ringed).
  `label_tool` (L, Ctrl+L global, H hierarchical on a sheet below the root, or the buttons:
  `Sch::naming`, a `Naming`: the name typed in the tool hint's `LABEL_FIELD`, its Global box, a
  global or hierarchical label's shape box, `SHAPES`), `pick_label` (Enter: the label is placed
  as a symbol is, `Project::add_label`; a hierarchical one's boxes get its pin too).
- Sheets: `sheet_tool` (Shift+S, or the button: `Sch::sheeting`, a `NewSheet`), `drag_sheet`
  (a drag on the canvas draws its box on the grid), `new_sheet` (the Project tab's New sheet: the
  box where `Project::free_spot` finds room), `sheet_field` (the tool hint's name and file
  fields, the file `file_for` the name until typed), `add_sheet` (Enter or Add:
  `Project::add_sheet`, the file read when it is there); a double click opens it. `Sheets::save`
  and `autosave` skip a made sheet undone (empty); `read_project` takes a sheet made since the
  last save from its autosave when restoring, and `Sheets::read` marks it edited.
- `tidy_fields` (T, Tidy texts, on a sheet): `Project::tidy_fields` on the page shown, one edit.
- Picks: a drag from a
  picked item (`Sch::drag`) shows what is picked moved (`drag_to`: at each grid step the move shown
  undone, `Project::move_items` again), `drop_drag` keeps it (an
  undo step); R `turn_picked`s it, Delete `delete_picked`s it (a symbol's part, every unit).
- `stop_tool` (Esc, Ctrl+Z, another page or tool, the board) ends them
  all; `undo_sch`, `save_sch` (Ctrl+S), `redraw_sch` (the page's scene again, its inks as they
  were, the net lit found again).
