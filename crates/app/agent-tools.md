# crates/app: agents' commands (`agent.rs`, `agent/`)

`Session` runs them over MCP stdio (`--mcp`, JSON-RPC 2.0, a message a line: `tools/list` sends
`TOOLS`, a tool a line in `agent/mcp.rs`, joined) or the JSON CLI (`--json`, or inline JSON
args). `cuprita --help` prints `agent::usage`: the modes and every tool (`mcp::names`).

## Board
- `open`, `new` (a project, blank or from a KiCad template: `new.rs`), `query`
  (summary/layers/nets/footprints/bom or `at` net pick), `find` (a part's value, footprint, fields
  and pads, each with its net, `at_mm` and copper layers; or a net), `place` (`side` front or back
  flips it), `route` (`shove`: other nets make way; `via`, `via_to` a layer: blind or buried, `layer`, `width_mm`), `delete`, `zone`
  (`pads`: thermal, solid, thru_hole_thermal or none: `Board::set_zone`), `fill`, `tidy` (Tidy
  texts, of the `references` given, else all), `fab`, `drc` (with the connections still to route),
  `diff`, `ir_drop`, `render` (headless PNG, optional zoom/net/diff/ir/flip/layers, `3d` with
  `models_missing`: `agent/render.rs`).
- `outline`: the board's outline through corners (`Board::set_outline`), cutouts kept.
- `pair`: a differential pair routed from its pads to its pads or a point, `through` corners; `tune`: a
  net made as long as asked, or as its pair's other side (`match`), by a meander (`crates/core/pair.md`).
- `text`, `line`: a text, or lines through points, of the board's own on a layer (`drawing.md`);
  `delete` at a point takes one away, after copper and before a zone, as a click picks it.
  `part_text` (`agent/parts.rs`): a part's own text said, moved, turned, sized, hidden or shown.
- `rules` (`agent/rules.rs`): the design rules in mm, those given first set where the
  `.kicad_pro` keeps them (`RULES`; `Val::pretty` writes it as KiCad does, `-bak` first), then read
  back as `Board::apply_pro` reads them, with the project's net classes. `netclass`: a class's
  values and nets set, made or deleted (`classes.rs`, which Inspect's list uses too).

## Schematic (`agent/sheet.rs`)
- `add` with `symbol` (a part number finds KiCad's for it) or `part` (looked up at Digi-Key:
  `Part::kicad`'s symbol with its fields), `fields` set as it goes in; the reply lists its pins
  (`pins_json`: `Schematic::pin_ends`, each pin's number, name, type, point and facing), as
  `field` does. `label` (`global`), `hier_label` (its boxes get its pin; `shape` for both),
  `no_connect`, `bus_entry`, `wire`, `bus`, `sheet` (a new sheet, or a file shown again).
- `connect`: pins named `REF:PIN` (a number, else every pin of that name) join a net: a label on
  each pin's end reading away from its symbol, one `Project::add_labels` edit a sheet.
- They edit the project's schematic (`Session::sch`, a `Sheets` as the app's, read again while it
  has no unsaved edits); `erc`, `simulate`, `pdf` and `update` (the board from it) read it.

## Parts (`agent/parts.rs`)
- `library`: the symbols or footprints of KiCad's libraries and the project's own that words
  find (`library::search`, as `add` and the Parts tab find them), best first.
- `import`: a vendor's part files into the project's own library (`parts::import`);
  `reference` gives that part the model that came, an edit.
- `new_part`: a part of one's own into that library, as New part… makes it: name, pins (how
  many, or each one's number, name, type, side), a library's footprint or a package preset
  (`crates/core/newpart.md`); returns its symbol and footprint as `add` takes them.

## Edits
`Edits` orders the session's edits, board's and schematic's: `undo` takes the last, `save`
writes them (`<file>-bak` first); an Cuprita window with the project open shows what is saved.

## Checked
A USB-C thermal camera dongle (ESP32-S3 module, MLX90640, 20 × 32 mm, two layers) was built with
these commands alone, a script of ~150: KiCad's ERC, DRC and schematic parity pass on it, but for
silkscreen warnings.

## Missing
Moving or hiding a part's texts on the board; deleting schematic items; a symbol's BOM and DNP
flags; a part's courtyard box in replies (`bounds_mm` is its pads').
