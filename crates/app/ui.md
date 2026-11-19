# crates/app: the window

How the app looks and is driven; the look itself is `../../BRAND.md`. Parent: `STATE.md`.

## Look
- `theme.rs` — colour tokens (`CANVAS` … `FAIL`), `apply` (fonts subset in `assets/`: Inter
  Medium and SemiBold, JetBrains Mono, Phosphor icons; the egui style), `strong()` (SemiBold),
  `tint`, brand widgets: `mark` (the logo), `tab` (a workbench tab), `tool` (an icon button with
  its key in a corner); `window_icon` (`assets/cuprita.png`, drawn from `cuprita.svg`).
- `icon.rs` — the icons, named for what they do; adding one: skill `icons`.
- `tr.rs` — translations: text shown goes through `tr("…")`, a template through `trf("… {0}",
  &[&x])`, a constant's text is marked `t!("…")` (and `tr`'d where it shows), a counted thing
  `n!("zone")` for `count` (its catalog entry `zona|zonas`: one|many); `i18n/<tag>.po`
  (gettext's format) holds each language's, built in; `LANGUAGES`, `read` (Settings' choice, else
  `LANG`, the browser's; it hands core `tr` too: `cuprita_core::tr::set`), `choose`. A test lists
  what each catalog lacks or no longer uses, of the app's marks and core's.
- `motion.rs` — moves (`BRAND.md`): `eased(ctx, start, secs)` is how far one is, asking for frames
  only while it runs; `reduced` (Settings; `--screenshot`, `--bench`). `App::{settle_view, arrive,
  fade_in}` glide a canvas to `App::fit` (a view still as fitted, `App::fitted`, follows its canvas's size) and veil one just shown; `helpers::ping`; 3D: `three.rs`.
- `units.rs` — mm or mils (`Cmd::Units`, Ctrl+U), kept in the data folder as `units`: every length
  shown goes through `len` (`0.15 mm`, `5.9 mil`), `num` (no units) or `fixed` (columns), every
  length typed through `parse` (in them, or with its own: `0.2mm`, `8mil`; `0,2` too).

## Commands
- `cmd.rs` — every command once (`Cmd`: name, icon, keys, hint): `App::{can, on, run}`; `keys`
  runs the keys pressed (more modifiers first: Shift+B before B; while a field is typed in only
  Ctrl and F-keys, never the text's own Ctrl+Z/Y/A/C/V/X; F flips the parts picked, else the
  view); `icon_cmd` is a command's button, `cmd_tip` its tooltip, `key_text` a key as its keycap
  reads (`/`; egui says `Slash` off a Mac). A new command goes there, never a key check in a panel.
- A canvas tool given in 3D or Make shows the board first (`run`).
- `palette.rs`, `help.rs`, `settings.rs` — the panels over the window (Ctrl+K, F1, Ctrl+,):
  `over.md`.

## Shell (`shell.rs`)
- `Bench` — Schematic, Board, ThreeD, Make; `App::bench`, `App::go(to)`: the schematic and the
  board keep their own place and zoom (`toggle_sch`), 3D and Make sit on the board's side.
  Keys: Ctrl+E (KiCad's) switches schematic and board, Ctrl+1–4 each workbench, Alt+3 3D.
- `header` — the mark (Home), the project (`project_menu`: its folder, Open, Home, the recent ones; unsaved dot), Save/Undo/Redo, the workbench tabs centred
  between the left and right groups, the palette's button, the readiness pill (`checks_pill`: Make's checks left, "2 to fix", or Ready to make; a click shows Make), Help and Settings on Home, Ask.
- `chrome.rs` — on Wayland (`own`: GNOME draws no title bar) the header is the window's: a drag
  moves it, a double click maximizes it, the window's buttons end it; its edges resize it.
- `tools` — the workbench's tools floating at the canvas's bottom (an `Area`, no fade-in), Select
  first (`tool_on`, `select_tool`, in `cmd.rs`);
  `tool_hint` — the tool on and what to do, at its top (the label tool's name field there);
  `sheet.rs` `sheet_path` — the schematic page's path at its top left (Backspace goes up).
- `status_bar` — the pointer's place (`App::pointer`, set by the canvases; blank off them), its
  units (a click switches them), the grid, what it snaps to (`App::snapped`), the active copper
  layer (board), then `news`: the ruler, the selection, the route or zone, the net, else
  `App::status`; what is left to route, `rules_count` (DRC or ERC; a click lists them),
  `saved_text` (saved, autosaved and when, or not); fps.
- `make.rs` `make_page` — the Make workbench: `make.md`.

## Home, docks, drawing helpers
What the window shows around and on the canvas: `views.md`.

## Gotchas
- `App::sch_view()` borrows all of `App`: take it into a local before borrowing a field mutably.
- Floating `Area`s sit above the canvas: its `hover_pos` is `None` under them, as wanted.
- A new `Area`'s first frame only sizes it (invisible, so a field in it drops the focus asked
  for then): ask again once it shows, as `label_field` does.
