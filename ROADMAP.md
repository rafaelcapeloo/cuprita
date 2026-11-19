# Cuprita roadmap

**Position:** a KiCad-native, Rust-fast, analysis-first PCB studio. Open the KiCad project you already have
(no migration, no lock-in), or start one, and take it to the fab in one window, with the speed and analysis KiCad lacks.

## Decisions (expensive to change)
1. KiCad files are the native format; saving round-trips losslessly (unknown fields kept).
2. Integer nanometres (i32) with i64 math; f32 only on the GPU (µm from a scene origin).
3. Undo keeps what each edit replaced, not board snapshots: a step costs what the edit touched.
4. One spatial index (R-tree) for picking, DRC, routing and zone fill.
5. wgpu canvas, retained buffers: tracks/pads/vias as instanced SDF shapes, polygons tessellated once.
6. egui for panels/menus; the canvas is a paint callback, so the toolkit can be swapped.
7. GPL-3.0-or-later, own implementation (no KiCad source).
8. AI goes through the same commands as the UI: MCP for outside agents, local models in the app.
9. One window: workbenches in the order of the work (Home, Schematic, Board, 3D, Make), each
   command listed once (`crates/app/src/cmd.rs`) for keys, buttons and the palette.

## Budgets
| | Target | Now (kria-k26, 54 MB, RTX 5060) |
|---|---|---|
| Open largest corpus board | < 1 s | parse 80 ms + scene 48 ms + upload ~11 ms + ratsnest 18 ms; browser 0.28 s from drop; 3D view +7 ms, glasgow-revD1's 41 models 65 ms; schematic: a project's 10–19 sheets 11–17 ms, a sheet's scene ≤ 4.4 ms, its ERC ≤ 2.0 ms (`kicad-cli` 1.3–2.5 s) |
| Pan/zoom | ≥ 120 fps | 180 fps at full view (monitor cap; 600–1100 uncapped through XWayland), browser too (WebGL 2); 3D 0.8 ms a 1080p frame |
| Route step; DRC re-check after an edit | < 16 ms | path search mean 5.5, worst 15 ms; shoving (both searches, the shove) mean 4.8, worst 13.1 (glasgow-revD1 5.6, 12.2); re-check 3 ms, 1.6 after a delete (beside the redraw); an edit refills the zones it changes, a part moved mean 1.5–20 ms (worst 144: glasgow-revD1's plane, whole); 249 items picked (14 parts, 195 tracks, 40 vias) moved, turned or deleted 11–16 ms in the window (DRC 3.5, ratsnest 5 ms), ~37 ms when a filled zone moves (its fills drawn again: 33); ratsnest 0.2 ms (a signal's), 3.8 (GND's); redraw 10 ms (scene 6, upload 3); a zone corner's drag step ≤ 0.6 ms; schematic: a symbol added 1–3 ms, its placing step ≤ 9.4 ms with the page drawn, a drag step ≈ 10 ms, a wire ≤ 4.4 ms, a label ≤ 4.1 ms (hierarchical, its boxes' pins too, ≤ 4.9), a bus or bus entry ≤ 4.4 ms, a sheet added 10–12 ms (glasgow-revD1, the whole project read again), a net lit on every sheet ≤ 4 ms |
| Full DRC / zone refill, big board | < 2 s on 8 cores | DRC 54 ms with courtyards (1 thread; `kicad-cli` 5.25 s); refill 0.27 s (KiCad 10: 2.2 s); new zone ≤ 0.1 s, a zone edited (corner, cutout, setting) 0.11 s with its refill (40 ms for a whole plane) and redraw; update from schematic 52 ms, redraw and whole re-check included (core 5 ms; values, fields and footprints carried); SPICE netlist ≤ 0.35 ms (ngspice's run as KiCad's); tidy texts 40 ms (653 parts' references and values) |
| Cold start to first frame | < 500 ms | 0.26 s small board, 0.45 s kria-k26; browser 0.33 s (Firefox) + 3.5 MB gzipped download |

## Phases (each ships something usable)
**0–4** ✓ Prove it, viewer + analysis, PCB editing, schematic + simulation, the product (v0.1–0.9):
what each shipped and its "next": `ROADMAP-shipped.md`; every command and key: `crates/app/features.md`.

**1.0: a first board without KiCad**: someone new to PCB design draws, lays out and orders a
small board in Cuprita alone, at KiCad's pace or better.
- [ ] installers: RPM, .deb, Flatpak, Windows setup and macOS disk image, each with KiCad's symbols, footprints and 3D models inside, built by the Installers workflow (`packaging/`); its Windows and macOS jobs haven't run yet (this user: `install.sh`)
- [x] the rest (picking several, track edits, zones, hierarchy and buses, shove, the walk-through, the guide, translations): `ROADMAP-shipped.md`

**1.1: parts from anywhere, documents**: the part a design needs, whoever made its library, and
the papers that go with a board.
- [ ] parts from the web: a part's symbol, footprint and 3D model by its LCSC number (EasyEDA's library, no account), or a vendor's KiCad zip (SnapMagic, Ultra Librarian, SamacSys, which Parts links to) dropped on the window, into the project's own library, which KiCad reads too
- [x] a part of one's own: New part… draws its symbol from its pins and makes its footprint from a package's numbers (or picks one of the libraries'), into that library too (agents: `new_part`)
- [x] the schematic as a PDF: every sheet on its paper, framed, as KiCad numbers them (Ctrl+Shift+P, Make's files, agents' `pdf`)
- [ ] the board's layers as a PDF (assembly drawings, a layer to review)

**1.2: a complex board, seen and shared**: what a 6-layer FPGA, DDR3 and GbE design made with the agents' commands ran into (182 parts; KiCad's ERC, parity and DRC counts matched), and a look of 2026.
- [x] a project on a web page to look at, schematic, board and 3D, following its files (`cuprita --serve`, `crates/app/web.md`)
- [x] motion that says what changed (`BRAND.md`); the schematic on a dark page; Checks by kind; boards of 2 to 32 layers; F8's parts by sheet; text variables
- [x] net classes with their own rules: DRC, the router and shove, zone fills; set in Inspect or by agents' `netclass`
- [x] blind and buried vias: routed (Shift+V, agents' `via_to`), checked, saved as their KiCad calls them, drilled per span
- [x] differential pairs routed side by side on a layer (6, agents' `pair`), their skew in Inspect; a net tuned to a length by a meander (Inspect's Net length, agents' `tune`)
- [ ] a pair through vias, other nets making way for it, tuning both sides of a pair together; microvias
- [x] in the schematic: mirror (X, Y), copy, paste and duplicate, as KiCad's clipboard reads and writes; a sheet's page settings
- [x] the board's own texts and lines (silkscreen and other layers): written, drawn, picked, edited in Inspect, by agents' `text` and `line`
- [ ] dimensions, arcs and circles drawn (those a file has move and save), texts on copper
- [x] DRC's silkscreen, mask-bridge, hole-clearance and loose-end checks, as KiCad lists them, by the project's severities; unplated slots as edges

## Not building (yet)
Own file/library format, SPICE engine or LLM runtime (use ngspice, llama.cpp/Ollama), autorouter, importers
of other tools' designs (use KiCad's), ready-made application circuits (no source gives them as circuits: drawn from datasheet figures they'd be guesses), full-wave EM/SI/thermal solvers (export to openEMS), AI routing, cloud accounts, plugin API (after 1.0), mobile.

## Easy to use, from day one
Ctrl+K palette with shortcuts · select, then act · Esc cancels · undo everything · unit math in fields (`10mil + 0.1mm`) · every error says what, where, and how to fix.
