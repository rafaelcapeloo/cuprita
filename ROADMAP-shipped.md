# Cuprita roadmap: phases 0–4, shipped

What each phase shipped, with its keys and numbers, and what came next. Parent: `ROADMAP.md`.

**0 Prove it** ✓ corpus (7 boards, KiCad 4–10, 26 KB–54 MB), parser + lossless lexing test, GPU viewer, cold start; KiCad 10 loads boards 11–100× slower than we parse them (kria-k26 887/80 ms).

**1 Viewer + analysis (v0.1–0.2)** ✓ layers panel, net highlight, find by reference/net, flip,
measure; MCP server + JSON CLI for AI agents; DRC from the project's rules (checked against
`kicad-cli`); git visual diff; net inspector (length, IPC-2152 current, impedance); DC IR-drop
heatmap; bench BOM checklist (`Shift+B`); browser (WASM) build (`./web.sh`, drop boards on the page).

**2 PCB editing (v0.3–0.5)**
- [x] place parts: click or Find selects, drag moves, R turns; DRC re-runs; MCP `place`
- [x] route tracks (`X`) around other nets with live clearance; V adds a via; MCP `route`
- [x] save to KiCad (Ctrl+S, MCP `save`): unedited text byte for byte, edits written, `-bak` first
- [x] undo/redo (Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z, MCP `undo`/`redo`); saving removes lost tracks/vias
- [x] delete the selected part, track or via (Delete or Backspace, MCP `delete`); undo puts it back
- [x] autosave to `_autosave-<file>`, restored on opening; ask before unsaved edits are lost
- [x] zones: draw (`Z`, MCP `zone`; KiCad's settings, filled alone), refill in parallel (`B`, MCP `fill`), save, undo
- [x] add parts from KiCad's footprint libraries (`A`, MCP `add`; found with nothing to set up): mounting holes, logos, test points, fiducials
- [x] fab files (`Ctrl+P`, MCP `fab`): Gerber X2, Excellon, BOM and positions as KiCad writes them, zones filled first; JLCPCB, PCBWay and OSH Park take them as they are, so no presets
- [x] 3D view (`Alt+3`, MCP `render` 3d): body, copper, mask, silk in stackup colours; parts' STEP models (KiCad 10 ships no WRL)

**3 Schematic + simulation (v0.6–0.8)**
- [x] view the schematic (`Ctrl+E`): every sheet as KiCad 10 draws it (KiCad 6–10), sheets opened, parts found; plain-language ERC (`D` there, MCP `erc`): KiCad 10's checks and severities, on its netlist; cross-probe: the part selected (a click or Find) is selected in the other view on `Ctrl+E` (next: nets)
- [x] add symbols from KiCad's symbol libraries (`A` in the schematic, MCP `add`; found with nothing to set up, by name or what they are): every unit, the next free reference, undo, save; wire them (`W`, MCP `wire`), junctions where KiCad needs them to join; name nets with labels (`L`, MCP `add` label), across sheets with global ones (`Ctrl+L`, `global`); buses later (rarer in new designs)
- [x] update PCB from schematic (`F8`, MCP `update`): the parts it lacks added below it from their Footprint fields, every pad on its pin's net by KiCad's names, a renamed net keeping its copper, one undo step; next: choosing a footprint in Cuprita
- [x] simulate the schematic (`S`, MCP `simulate`) in ngspice's shared library, KiCad's own found with nothing to set up: each part's Simulation Model (else its value), the sheets' directives and the workbook's analyses as KiCad's simulator writes them; plots (AC in dB), operating points, ngspice's warnings; next: a net probed on the sheet, op values on it, IBIS and XSPICE parts
- [x] in-app chat on small local models about the project repo and its datasheets: Ask (`?`), in Ollama (Gemma 4 E4B answers right), through the agents' commands that only look, on the board and schematic as they are, the repo's files and the datasheets its parts link to; next: editing through it, PDFs without poppler
- [x] part-number search (`A` in the schematic, MCP `add`): KiCad's own symbol for it with nothing to set up (LM358DR finds `LM358`; 79% of KiCad's name their footprint, so their 3D model, and datasheet), and with the user's own Digi-Key key the exact part: its symbol, else a chip resistor's, capacitor's or inductor's valued, its package's footprint, datasheet, maker and number for the BOM; next: other distributors, LEDs and diodes

**4 Cuprita, the product (v0.9)** ✓ one window from a blank project to the fab's files, a look of
its own (`BRAND.md`: graphite and copper, Inter, JetBrains Mono, Phosphor icons; the window's own
title bar on Wayland); every command once (`cmd.rs`) for keys, buttons, hints and Ctrl+K (commands,
parts and nets by name; Find too); workbenches Schematic, Board, 3D, Make with a tool dock, tool
hints and a status bar (lengths in mm or mils, Ctrl+U); docks Project, Layers, Parts, Inspect,
Checks, Ask; new projects (blank or a KiCad template, MCP `new`), Home's recents and examples;
First steps guides a new project; helpers under the hand (crosshair, named snap rings, readouts,
hover glow, 45° corners, alignment guides) on a grid (N); ratsnest and the unrouted count; the
board outline tool; a part's fields typed in Inspect (MCP `field`), its footprint chosen by its
filters; schematic: several items picked (Shift+click, a box), moved with their wires following,
turned, deleted; no-connect flags (Q); F8 carries values, fields and footprints to placed parts;
routing: layer switch, width steps, `/` swaps the bend; zones selected and deleted; a part flipped
to the bottom (F); Make: what an order still needs, the board in numbers, order pages (JLCPCB,
PCBWay, OSH Park).

**1.0 A first board without KiCad** (all but the installers, `ROADMAP.md`)
- [x] board: several items picked at once (Shift+click, a box), moved, turned and deleted together
- [x] tracks: a segment dragged (its neighbours following at 45°), a laid track's width changed, a run picked (U)
- [x] zones: a corner moved, a cutout added, a zone's settings changed
- [x] schematic: hierarchical sheets and labels added, buses; nets cross-probed between the views
- [x] push-and-shove router: other nets' tracks make way
- [x] a 10-minute first-board walk-through on First steps
- [x] docs: the user guide (`docs/`), which Help (F1) shows
- [x] translations (`crates/app/i18n/`): the window, the guide and all core says, in English and Português (Brasil); German, Spanish and French were written and are set aside for now (the owner's call)
