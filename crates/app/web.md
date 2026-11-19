# crates/app: browser build

## Entry points
- `web.rs` (wasm32) — `start` runs the app on `index.html`'s `<canvas id="cuprita">`;
  `open_dropped` reads dropped files (async) and opens what `dropped_board` (`main.rs`, tested
  natively) picks: the `.kicad_pcb`, with the rules of a `.kicad_pro` dropped along with it;
  `download` offers files as browser downloads (a saved board, fab files); a `beforeunload` listener has the browser
  ask before leaving the page while `UNSAVED` (set each frame) holds. `index.html` cancels the
  browser's own action for Cuprita's Ctrl keys (Ctrl+F finds in Cuprita, not the page; a test in
  `cmd.rs` keeps its list whole).
- `../../web.sh [serve]` — wasm build into `target/web`, file names new per build (installs the
  matching `wasm-bindgen` CLI). Cargo's `web` profile optimises it whole (fat LTO, one unit) and
  the functions' names are left out: 3.5 MB gzipped, not 4.1, as fast to start. `check.sh` lints
  the wasm32 build.

## A project served to look at (`cuprita --serve`, Share; `serve.rs`, `share.rs`)
- `serve::start(file, bind, port, any)` gives a `Server` (its `url`; dropped, it stops), `run` waits
  on one. Bound past this computer, the address carries a key (16 hex digits, made at start)
  and nothing answers without it (`/<key>` redirects to `/<key>/`); `lan_ip` is this computer's
  address there (asked of the routes: nothing is sent). `share.rs`: `App::share` starts one on
  `0.0.0.0:8080` (or a free port), `share_panel` shows its address as a QR code (`qrcodegen`).
- The standard library's sockets, a thread a request, `GET` only
  (`Served::get`; anything else 405): the page's files from `web_dir` (`../share/cuprita/web`
  beside the app, the data folder's `web`, else `target/web`); `project.json` (`listed`: name,
  board, root sheet, each file with when it was written); `project/<file>`, only files the list
  names (board, `.kicad_pro`, every `.kicad_sch` of the folder, three folders deep; no autosave
  or hidden folder); `model?f=<file as the board writes it>` (`models::path`: the project's
  folder or KiCad's libraries, no other variable, no `..`). Replies gzipped (level 3).
- `web.rs` `follow`: the page fetches the list every 2 s and each file whose version changed
  into `FILES`, which `file::{read_text, written}` read in a browser; the first time `poll`
  sets `App::viewer` and opens `/project/<board>`. `watch.rs` then shows again what changed,
  the view kept. `fetch_models` (from `App::load_models`): STEP files fetched and read in the
  page one at a time, kept for a board shown again.
- `App::viewer`: only commands that look run (`Cmd::looks`, in `can`); no Make tab, Parts tab,
  Save, undo or readiness pill (a View only pill); nothing is grabbed (`canvas.rs`, `sheet.rs`),
  Inspect is disabled, a drop opens nothing. A phone's page (`shell::narrow`, under 700 points):
  docks start folded (the right one away), tabs and pill are icons, no hints; two fingers pinch
  and slide (`canvas::zoom_at`, `three.rs`). Checked with `ff.py`'s `size` and `pinch` steps. A page
  may open before its window has its size (a tab not yet shown): wide before any hand touched it,
  it gets its docks back (`FOLDED`), and the view, still as fitted, fits the canvas again.
- On a real GPU (Brave, Chromium 156 with WebGPU, Linux): the board, the sheets and the 3D view
  with its models draw as on the desktop, the console quiet; opened at 640 points it is the
  phone's page, widened it is the desktop's.
- `install.sh`, `package.sh` and every installer put the page's files in place
  (`packaging/README.md`; without them `--serve` and Share say where they looked).

## Status
`./web.sh serve`, WebGPU or WebGL 2: the same viewer and analysis, no Diff (needs git); the 3D
view shows the board without parts' models (no files to read them from).
Save downloads the board; Fab files (Ctrl+P) download its three files (the browser may ask once
to allow several). Opening a dropped board with unsaved edits asks first.

## Missing
Open by URL, Diff, autosave (no file to write next to; boards outgrow localStorage), the schematic
view of a dropped project (a served one has it: its sheets are fetched), recent boards (they are
dropped files; the units are kept, in local storage).

## Gotchas
- `std::time::Instant` panics in the browser: use `web_time::Instant`. A browser drop opens a frame
  or more later, once the file is read.
