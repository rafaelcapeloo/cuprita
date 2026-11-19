# crates/app: Home, the docks, the drawing helpers

What the window shows besides the shell (`ui.md`, its parent).

## Home (`home.rs`, `recent.rs`)
- `Bench::Home` (where the app starts; the mark goes there): New project (Ctrl+N: `start_new`, a
  `Modal` naming it, its folder, Blank with copper layers and size or a KiCad template; writes
  with `cuprita_core::new`, then shows its schematic), Open… (Ctrl+O: `rfd` on a thread), Recent
  (`recent.rs`: a list and a picture per board in the user's data folder, kept by `App::load`
  unless `--screenshot`/`--bench` runs; `standing`: where each stood, kept beside its picture),
  Examples (KiCad's demos), the drop zone, `found_list` (`look_around` on a thread, again each
  time Home shows: KiCad's libraries, ngspice, Ollama's models), today's `tip`.
- Pictures (`picture`): a recent board's PNG, or a template drawn now with `Renderer::shot` on
  the canvas's renderer, one a frame, only while Home shows (leaving it, `go` uploads the scene).
- `empty_hint` — what to do first on an empty sheet or board.

## First steps (`guide.rs`)
- `steps`: nine, each checked from the project's `Progress` (its parts, ERC's unconnected pins,
  footprints, the board's parts, `outline_box`, the ratsnest, DRC, the gerbers zip);
  `App::guide` floats them at the canvas's top right while `App::guide` is set (open or folded):
  New project sets it, `Cmd::Guide` (Ctrl+K › First steps) toggles it; the empty hint gives way.
  `tour` (`App::tour`, `start_tour`: Ctrl+K › First board, or its offer) names an LED's
  parts (`CELLS`, `RESISTORS`, `LEDS`); `lit` follows their loop through the netlist.

## Drawing helpers (`helpers.rs`)
- `board_helpers`, `sheet_helpers` (end of each canvas): while a tool draws, the system pointer
  hides and a `crosshair` marks the point a click takes; a `ring` (with `copper_name`) on the
  copper or pin it joins, else a `diamond` on the grid; a `readout` (route: length, angle, layer,
  width; zone: edge, angle; wire: length); a dashed `guide` on past the track or zone edge drawn
  (the readout steps left of one running down-right). No tool: what a click picks glows
  (`hover_glow`; a wire too). Both return what the point snaps to, named: `App::snap_to` keeps it
  for the status bar, drawn before the canvas (a change asks for the next frame).
- `lock45` keeps zone corners at multiples of 45° (`App::zone_aim`; Shift frees them); `span`
  is a segment's length and angle as KiCad turns. `Ink`: dark on the sheet's paper, light on boards.

## Docks (`dock.rs`, `inspect.rs`)
- `App::docks` before the canvas: `App::left` (Project, Layers, Parts) and `App::right` (Inspect,
  Checks, Ask, and IR drop, Changes or BOM while on), each a `Dock { tab, open }`; folded, a rail
  of icons; open, as wide as its tabs' names need (a language's longer ones), the fold button after.
  `Dock::toggle(tab, back)` is how a command shows its tab (A, D, ?), and again goes back.
- A tab's state is the dock's: the Parts tab out is `App::adding()`, the Checks tab out marks
  the violations on the canvas (its count: `checks_count`), the Ask tab out is the chat (`chat_poll` runs whether or not).
- `project_tab` — the schematic's pages by hierarchy (a click shows one; its parts counted; New sheet; the board's root sheet before it is read), the board, the fab
  files written, `files_list`: the folder's `documents` (README first; git's list, read again at
  most every 10 s) and the parts' Datasheet links, opened by `open_url`; `layers_tab` — sets (All,
  Copper, Front, Back), a row per layer with its eye (a copper layer's row makes it the active one,
  marked in copper and ACTIVE, how PgUp/PgDn step at the foot; an inner one shows its `plane`, the net of its biggest zone), a line after the
  copper, hidden layers seldom shown folded under "N more, hidden";
  `inspect_tab` — the part, track, via, zone (its settings set in place: `zone_settings.rs`, typed
  as a part's fields, `field_row`, in `App::typing`; a change refused says why below, from
  `App::refused`) and net picked in numbers (while routing, the route's), or how many of each
  are picked together (turn, flip, delete), else the board's; in the
  schematic the part picked and where the board has it, else what is picked (a wire's length, counts).
