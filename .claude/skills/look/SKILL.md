---
name: look
description: See what Cuprita draws - headless PNG of a board, or a screenshot of the real app window or the browser build. Use after any change to rendering, the scene builder or the app UI, because agents cannot see windows.
---
# Look at the output

Write images outside the repo (system temp dir), view them, then delete them.

- Fastest, no window, any feature: the agent CLI, e.g. `target/release/cuprita --json <board>
  '{"cmd":"render","out":"/tmp/x.png","find":"U7"}'` (also `net`, `flip`, `diff`, `ir`, `layers`, `3d`).
- Renderer only (prints parse/scene/GPU timings; GPU includes device setup):
  `cargo run --release -p cuprita-render --example png -- corpus/boards/<board>.kicad_pcb /tmp/x.png [width] [--3d]`;
  a `.kicad_sch` sheet too (`corpus/boards/<project>/`, its `.kicad_pro` beside it read).
- 3D against KiCad: `flatpak run --filesystem=<dir> --command=kicad-cli org.kicad.kicad pcb render
  --side top|bottom --background opaque -o <dir>/k.png <board>` (orthographic; `--rotate`, `--perspective`).
- Whole app (status bar, layout, the real paint path):
  `cargo run --release -p cuprita -- --screenshot /tmp/x.png corpus/boards/<board>.kicad_pcb`;
  no board: Home (its recents and New's folder: give `XDG_DATA_HOME`, `HOME` a temp folder).
- Frame rate: `cargo run --release -p cuprita -- --bench <board>` prints fps every second.

Check: outline visible, layer order right, pad rotations right, zones present, text placed
like KiCad's, nothing clipped.
Good boards: `hackrf-one` (classic), `glasgow-revD1` (KiCad 10), `kria-k26` (largest).

## Script input: keys, clicks, drags, text (no input tool is installed)
Open a copy outside the repo, never `corpus/boards/` itself (Ctrl+S writes over it), and give
`XDG_DATA_HOME` a temp folder (Home's recent projects). `.claude/skills/look/replay.sh on`, build
release, write steps `frame step args` (logical points, the window 1440×900), run
`CUPRITA_REPLAY=steps timeout 60 target/release/cuprita <board>`, then `replay.sh off` (main.rs
back as it was at `on`: edits made to it meanwhile are lost; never commit it on). Steps: `key ctrl+shift K` or `key Escape` (a letter gets
its text too), `text …`, `copy`, `paste …` (as Ctrl+C and Ctrl+V reach egui), `drop <file>`, `move x y`, `click x y [shift]` (`rclick`: the right button), `press`/`release x y [mods]`, `drag
x0 y0 x1 y1 [mods]` (11 frames), `scroll lines`, `find U7` (zooms there, selects it), `say`
(prints status, workbench, selection and picks, net lit, the pointer in nm; in the schematic its status, picks, net lit), `shot out.png` (saves the frame, then closes; unsaved
edits make closing ask, hence `timeout`), `snap out.png` (saves the frame and goes on). Clicks less than 0.3 s apart count as a double click (it ends a wire or a route): leave ~80 frames
between them. Leave ~5 frames after opening and ~30 after a step
that loads (libraries, a schematic); an edit autosaves 5 s on (`_autosave-<file>`). To fake a
system tool (`xdg-open` for a link), put a logging script first in `PATH`, never empty it.
Ctrl+E needs the board beside its sheets, which the corpus keeps apart: copy
`corpus/boards/<p>/*` and `<p>.kicad_pcb`, named as the root sheet, into a temp folder.

## Browser build (Chrome via the claude-in-chrome tools)
`./web.sh`, serve the repo root so `corpus/boards/` is same-origin (`python3 -m http.server -b
127.0.0.1 8000`), open http://127.0.0.1:8000/target/web/ and reload it after every build.
Drop a board from JS: `const dt = new DataTransfer(); dt.items.add(new File([await (await
fetch('/corpus/boards/hackrf-one.kicad_pcb')).blob()], 'hackrf-one.kicad_pcb'))`, then dispatch
`new DragEvent(t, {dataTransfer: dt, bubbles: true})` for `dragover` and `drop` on `#cuprita`.
Leave a frame between a focusing click (or Ctrl+F) and keys or typing, or they are lost.
Panics show in the console. Without Chrome, `.claude/skills/look/ff.py <url> wait:7 shot:x.png
click:615,23 'sh:<command>'` drives a headless Firefox (no dependencies; console errors at its
end; `size:500x844` and `pinch:250,400,80,260` for a phone's page). A served project needs no drop: `cuprita --serve <project> --port 8174` (`crates/app/web.md`).

## Compare with KiCad (flatpak `org.kicad.kicad`)
Always pass `--command=kicad-cli`: without it the KiCad window opens. The sandbox sees only
`$HOME`, so expose an output dir (not `/tmp` itself): `mkdir -p /tmp/k`, then
`flatpak run --filesystem=/tmp/k --command=kicad-cli org.kicad.kicad pcb export svg --mode-single
--page-size-mode 2 --exclude-drawing-sheet --layers F.SilkS,Edge.Cuts -o /tmp/k/k.svg <board>`
and `magick -density 600 -background white /tmp/k/k.svg -flatten /tmp/k/k.png`.
Exact numbers: `pcb export gerbers --layers F.SilkS -o /tmp/k/` and read the `X…Y…D01*` strokes.
Schematics: skill `sch-check`.
