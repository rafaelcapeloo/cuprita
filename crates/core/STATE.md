# crates/core: state

KiCad file parsing and the board model (what a `Board` holds and lacks: `model.md`). No GUI/GPU
dependencies (tests, CLI, WASM use it). `// shortcut:` comments mark accepted limits.

## Entry points
- `parse.rs`: `Board::parse(&str)` — `.kicad_pcb` (KiCad 4–10) → `Board`, all in board coordinates;
  `Board::apply_pro(&str)` applies design rules, the net classes with their nets and track widths from a `.kicad_pro`;
  footprints with their pads and 3D models are read in `parse/footprint.rs`, zones in `parse/zone.rs`.
- `drc::{check, check_items, recheck, RTree}` — design rule checks; see `drc.md`.
- `outline.rs`: `Board::{set_outline, outline_box, outline_size}` — a new outline on Edge.Cuts in place of the outer loops,
  cutouts kept (refused when a part draws part of it: an edge connector's tongue); saved as lines. `drawing.rs`: `Board::{own, drawing_at, drawing_layers, ink, new_text, add_text, set_text, add_lines, set_shape}` — the board's own texts and drawings: `drawing.md`.
- `ratsnest.rs`: `Board::ratsnest`, `ratsnest::recheck` — what is left to route: each net's copper in islands
  (touching on a layer they share, fills too), joined by the shortest lines; an edit joins its nets again.
- `route.rs`: `Board::{route_start, joins_layers, via, add_route}`, `Router::{new, on_grid, snap, path, connects}`: routing
  round the clearances (`route.md`); `shove.rs`: `Shover::{new, step, shove}`: other nets make way (`shove.md`). `pair.rs`, `tune.rs`: `Board::{pair, pair_ends, add_pair, net_length, tune}`, `PairRouter` — differential pairs side by side and nets tuned to a length: `pair.md`.
- `save.rs`: `Board::write(src)` — the board back to the KiCad text it was read from; see `save.md` (zones: `save/zones.rs`, a flipped part: `save/flip.rs`).
- `fill.rs`: `Board::fill_zones` — refills every copper zone, ranked by priority then id, in
  parallel, as KiCad 10 does, and says how many changed; `Board::{add_zone, set_zone}` — a zone
  added or edited (`zone.rs`), filled alone (`fill-new.md`); see `fill.md`; `examples/fill.rs` times it.
- `library.rs`: `library::{footprints, symbols, found, search, named, symbol, field, fits}`, `Board::add_footprint`
  — KiCad's libraries found with nothing to set up, searched (by part number too), a symbol as sheets copy
  it, its fields, footprint filters, a part added: `library.md`; `digikey.rs`: `digikey::lookup(mpn, key)`,
  `Part::kicad` — a part number at Digi-Key (the user's key), KiCad's symbol and fields for it: `digikey.md`.
- `parts.rs`: `parts::{table, symbols, footprints, import}` — a project's own libraries, and a part's KiCad files (or a zip: `zip.rs`) put into
  its one, where Cuprita and KiCad find them: `parts.md`; `newpart.rs`: `newpart::{Part, Package, PRESETS, create}` — a part made from its pins and a package: `newpart.md`.
- `models.rs`: `models::{files, load}` — footprints' 3D models found as KiCad finds them, read
  (`step::read`, STEP → triangles) in parallel; `Board::body` (`body.rs`); see `models.md`.
- `history.rs`: `Patch` (what undoes an edit: the items it replaced), `Board::apply`, `History::{push, undo,
  redo}` of a board or a schematic; `picked.rs`: `Board::{items_in, move_items, turn_items, flip_items,
  delete_items}` — what a box takes, edited together as one undo step: `history.md`; `tracks.rs`:
  `Board::{run, set_widths, dragged, drag_segment}` — a track's run (end to end, up to pads, vias,
  branches), widths set, a segment dragged with its neighbours following at 45°.
- `tidy.rs`: `Board::tidy_texts(parts)` — Tidy texts: references placed where they read (or hidden), values in their bodies: `tidy.md`.
- Shared tools (polygon booleans, threads, texts in the language shown, JSON, tokens, units,
  lookups by name and place, the font): `tools.md`.
- `analysis.rs`: `Board::{inspect_net, layer_impedance, default_ir_net, ir_drop}`,
  `ipc2152_current` — track length, IPC-2152 current, microstrip/stripline impedance and a
  nodal DC IR-drop solver (tracks, vias, pads and zones).
- `fab.rs`, `gerber.rs`: `Board::{fab_files, positions_csv}`, and `bom.rs`: `Board::{bom, bom_csv}` —
  Gerber X2, Excellon, positions and BOM as KiCad writes them, which fabs take; see `fab.md`.
- `diff::Diff::between(old, new)` — visual and structural diff, matched by layer/net/ref names.
- `schematic.rs`, `schematic/`: `Schematic::{parse, item_at, items_in}`, `Project::{load, netlist, net_of, net_items, erc, spice,
  add_symbol, add_wire, add_bus, add_bus_entry, add_label, add_no_connect, add_sheet, move_items, turn_items, delete_items, tidy_fields, apply, text}` — a project's sheets (KiCad 6–10) read and drawn as KiCad
  draws them, joined into nets (cross-probed by name), checked, written for ngspice, edited: `schematic.md` and the files it links;
  `Project::{pdf, number, frame, free_spot}` — every sheet plotted on its paper with a frame and title block (the window's too), as KiCad numbers them: `pdf.md`.
- `new.rs`: `new::{files, write, templates, copy_template, examples}` — a new project as KiCad 10 writes it
  (KiCad reads it: 0 DRC, 0 ERC), or a copy of a KiCad template under its new name (KiCad's checks match
  the template's); KiCad's templates and demos found as its libraries are; never into a folder holding files.
- `update.rs`: `Board::update(&Project, footprint)` — KiCad's Update PCB from Schematic (F8): the parts the
  board lacks added, footprints swapped, values and fields carried, the parts' pads on the schematic's nets, a renamed net keeping its copper; `update.md`.
- `ngspice.rs`: `ngspice::{simulate(&Deck, stop), found}` — the netlist run in ngspice's shared library, KiCad's
  or the system's, found with nothing to set up; each analysis' plot of vectors; see `ngspice.md`.
- `ollama.rs`: `Ollama::{models, chat}` — a conversation with a local model in Ollama, found with
  nothing to set up: its reply streamed, the tools it calls; see `ollama.md`.
- `examples/{load, route, spice}.rs` — parse, path search and shove timing, a project's netlist and its
  simulation (`--run`); `tests/corpus.rs` — the corpus against KiCad 10's results: `tests.md`.

## Gotchas
The board model's and its parsers' (angles, Y down, nets, escapes, `.kicad_pro` keys, lists, text): `model.md`.
