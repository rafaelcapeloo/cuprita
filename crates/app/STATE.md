# crates/app: state

`cuprita` binary (eframe + wgpu) for the desktop and the browser. UI and input only; logic lives in core/render.
What each command does, with its keys: `features.md`.

## Entry points
- `ui.md` — the window: `theme.rs` (the look), `icon.rs`, `cmd.rs` (every command, its keys and
  button), `shell.rs` (the workbenches, header, tool dock, status bar), `make.rs` (the Make page), `units.rs`; `views.md`: Home,
  the docks, the drawing helpers; the board's canvas and grid: `board.md`.
- `main.rs` — args `[--bench] [--screenshot out.png] [--diff [base]] [--ir [net]] [--mcp] [--json] [--serve] [file] ['{"cmd":...}' ...]`.
  `App::load` shows board text. The layers panel toggles `App::scene.layers[i].visible` and
  sets `App::active` (`shown_copper`: the copper in view, active first); a click sets `App::selected`
  to what `App::pick_at` picks: the pad's part, via or track `Board::copper_shown_at` finds (none a plane above hides), else the part
  `Board::footprint_at` finds, else the zone `Board::zone_at` finds, and `App::net` to its net (else `Scene::net_at`'s). `App::find` (Ctrl+F, Enter) shares `find_target` with `agent.rs`, `pick_name` too.
- `canvas.rs`, `edit.rs`, `route.rs`, `zone.rs`, `corners.rs`, `zone_settings.rs`, `grid.rs` — the board view: picking,
  dragging, the tools that edit it, the grid; see `board.md`.
- `file.rs` — `App::open` (asks to restore a newer `_autosave-<file>`), `App::save` (Ctrl+S:
  `board.write(&src)` via `save_file`, shared with `agent.rs`, `<file>-bak` first; `App::src` is the
  text read, which every save writes all edits into), `App::autosave` (5 s after `App::changed`),
  `App::leave(then)` asks (`Ask`, `App::ask_dialog`) before unsaved edits go; `App::open_url`.
  `watch.rs`: `App::follow_files` reads again, once a second, what another program saved.
- `fab.rs` — `App::fab` (Ctrl+P) fills the zones, then writes `fab_files` (the Gerbers and drills
  zipped by `zip`, the BOM and positions CSVs) next to the board with `write_files` (`save_file`,
  so `<file>-bak` first), shared with `agent.rs`; in the browser, `web::download`s them.
- `sheet.rs`, `sheet_edit.rs`, `sim.rs`, `lookup.rs` — the schematic view (Ctrl+E): ERC (`D`), adding symbols
  (`A`; a part number's at Digi-Key too), wires (`W`), buses (`B`, entries `Z`), labels (`L`; `H` hierarchical), sheets (Shift+S), no-connect flags (`Q`), nets cross-probed, undo, save, the board updated from it
  (F8), simulation (S); see `sheet.md`. `save_button`, `undo_buttons`, `focus` are both views'.
- `parts.rs` — a part's files dropped go into the project's own library (`App::import_parts`, `App::put_parts`), and `parts::{symbols, footprints}`
  fill the Parts tab's lists, the project's first; `newpart.rs` — New part… (`Wizard`): a part made from its pins and a package, drawn as typed
  (`picture`: shapes, texts and pads with egui's painter), put in as an import is.
- `three.rs` — 3D view (Alt+3): `App::toggle_3d`, `App::canvas3d` (drag turns, right or middle drag
  slides, wheel zooms, F turns it over; `Paint3d` renders before egui's pass and blits),
  `App::load_models` (a thread reads the models not read yet into `App::models`; the browser has none).
- `panels.rs` — the docks' lists: `parts_tab` (`library::search` over `App::parts`, in the schematic
  `App::symbols`, read on first use), `heat_tab` (IR), `changes_tab` (diff), `drc_tab` (`check_rows`: by kind once long, ERC's too), `bom_tab`; the
  docks themselves and the Project, Layers and Inspect tabs: `views.md`.
- `agent.rs`, `agent/`, `chat.rs` — AI: agents' commands (`Session`) over MCP (`--mcp`) and the JSON
  CLI (`--json`), and the chat (`?`) with a local model in Ollama on them; see `agent.md`, `agent-tools.md`.
- `tr.rs`, `i18n/` — translations (`ui.md`); `docs.rs` — the guide Help shows (`over.md`); what a project from someone else may do: `security.md`.
- `web.rs`, `serve.rs`, `share.rs`, `../../web.sh` — the browser (WASM) build, and a project served on a page to look at (`--serve`, Share's QR code, `App::viewer`): `web.md`; `../../install.sh [remove]` — Linux, this user; `../../package.sh` — an RPM or a .deb, each built on the system it installs on (launcher: `assets/cuprita.desktop`); the Flatpak, Windows and macOS installers and the workflow building them all: `../../packaging/README.md`.

## Missing
Background loading; routing: microvias; add part: from libraries only an `fp-lib-table` lists, and in the browser (it has no
libraries); a part's text: its words and layer set.

## Debt
- Make's look line names the board file's mask and silkscreen colours in English ("black"):
  translating them wants each language's agreement with "mask" and "silkscreen" (`make.rs`).

## Gotchas
- Parsing runs on the UI thread (fine below ~150 ms; move to a worker when it isn't).
- So does a refill (B): kria-k26 0.33 s on 12 threads, 0.8 s on one core (the browser has one).
- The app keeps each board's text (`App::src`, 54 MB for kria-k26) to save it byte for byte.
- Autosave writes on the UI thread once the board rests, pointer up (kria-k26 0.1 s: 84 ms text).
- On Wayland, fps tops out at the monitor refresh even with `--bench`.
- winit 0.30 passes no file drops on Wayland (X11, Windows, macOS do): Open… and Add part files…
  are the ways there; system dialogs answer through `App::home_answers`, read every frame.
- eframe is built without its default features: `clipboard` must stay listed (`Cargo.toml`), or
  Ctrl+C and Ctrl+V reach no system clipboard (look's replay `copy` checks it with `wl-paste`).
