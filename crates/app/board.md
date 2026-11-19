# crates/app: the board view and its editing

The board canvas, what a click or a box picks, and the tools that edit it; the core side:
`crates/core/STATE.md` (`history.md`, `route.md`, `fill-new.md`). Parent: `STATE.md`.

## Entry points
- `canvas.rs` — `App::canvas` pans (drag), zooms at the cursor (wheel), picks (several: Shift+click, a Shift+drag box
  `Board::items_in` takes, in `App::picked`; `App::picks`), drags what is picked and zone handles, draws tool marks; `Paint`, the wgpu
  callback, carries the view to the renderer.
- `edit.rs` — `App::add_part` adds a library part in the middle of the view (`add_from_library`; `footprint_text`
  reads one by `Library:Name`; shared with `agent.rs`); `App::{move_picks, nudge, turn_picks, flip_picks, delete}` edit
  what is picked as one step (`crates/core/history.md`); `App::tidy_texts` (T: `Board::tidy_texts` on the parts picked, else
  all, `crates/core/tidy.md`; on a sheet `App::tidy_fields`, `sheet-edit.md`); `App::fill` refills the zones (`Patch::fills` undoes it); `App::edited`
  keeps its undo `Patch` in `App::history`, re-checks and redraws, as `App::undo` (Ctrl+Z; Ctrl+Y redoes) does for a step,
  dropping the picks; `App::retick` keeps BOM ticks by line, `keep_ticks` for the next opening;
  `App::pick_runs` (U: `Board::run`); Inspect's `track_width` (`Board::set_widths`); `App::drag_track` (a track
  picked alone, dragged: `Board::drag_segment`, shown as it goes by `Board::dragged`).
- `route.rs` — route tool: `App::{route_click, route_to, route_fix, route_via, switch_layer, toggle_shove,
  draw_route}` around `cuprita_core::{route::Router, shove::Shover}` (`crates/core/{route,shove}.md`).
- `zone.rs` — zone tool (outline, cutout): `App::{zone_click, zone_close, zone_back, draw_zone}`; a click takes net and layer as
  the route tool does (`Board::route_start`), `Board::add_zone` adds the zone; `corners.rs` — the picked zone's
  handles (`handle_at`, grabbed in `App::grab`): `App::{drop_handle, delete_corner, draw_handles, zone_edited}`;
  `zone_settings.rs` — its settings in Inspect (`App::zone_settings`, typed in `App::typing`).
- `pair.rs` — the Pair tool (`App::{pair_click, pair_to, pair_finish, pair_back, draw_pair}` around `cuprita_core::pair::PairRouter`,
  `crates/core/pair.md`); Inspect's `tune_field` and `App::tune` (`edit.rs`) lengthen a track's net by a meander (`Board::tune`).
- `drawing.rs` — the board's own texts and drawings (`crates/core/drawing.md`): the Text tool (`App::{text_tool, text_field}`,
  the words in `App::writing`), the Line tool (the zone tool `lining`: `zone_close` adds the lines), the layer both draw on
  (`App::{ink_layer, ink_choice}`), and Inspect's `App::{text_settings, shape_settings}`; `App::pick_at` picks them after copper, before parts,
  then a part's own text (its reference, its value): dragged, turned, sized in Inspect and hidden by Delete, alone;
  Inspect's Hidden texts of a part picked shows one again (`App::show_text`, `edit.rs`).

## Grid (`grid.rs`)
- `STEPS`: 1, 0.5, 0.25 (to start), 0.1, 0.05 mm, then 100, 50, 25, 10, 5 mil (KiCad's, fewer);
  N and Shift+N (KiCad's keys, `App::step_grid`) go round them. `App::board_grid`: through KiCad's
  grid origin (`Board::grid_origin`). The status bar shows it (`grid_label`; a click steps it).
- `dots`: one mesh of faint square dots over the board (white at 14%: seen on copper too), on every
  grid point or every 2, 4, 10, 20…, never closer than 10 points (KiCad's least spacing); 13 µs
  a frame on kria-k26. The schematic's 50 mil grid gets none: over the sheet they'd dot every wire.
- Snapping (Ctrl frees it, KiCad's key): `drag_by` puts a dragged part's anchor, or the pad grabbed,
  on the grid; `App::aim` a measure, zone or outline point on the pad or via centre or track end
  under the pointer (`copper_point`), else the grid, a corner whole steps along its 45° line
  (`lock45`); a route aims at the grid off its net's copper, its router's cells on the grid
  (`Router::on_grid`, rebuilt when N steps it).

## Gotchas
- An edit re-checks what it changed (DRC, ratsnest: a thread each) as the UI thread rebuilds the scene but
  its zone fills (`edit.rs` `rechecked`; kria-k26 ~12 ms); drag and route previews are egui shapes.
- A path search runs on the UI thread each time the pointer's grid point changes (≤ 15 ms).
