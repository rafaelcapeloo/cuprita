# crates/core: board model (`board.rs`)

## What a `Board` holds
Tracks (arcs have `mid`; `locked`), vias (every copper layer spanned; `locked`), pads (number, footprint id,
flattened coordinates, own clearance and zone connection, thermal spoke angle, own mask and
paste margins; a custom pad's `shape` is its anchor, `custom` its whole shape in its own frame;
plated holes' pads on every copper layer, as in KiCad, unplated ones on none), footprints
(reference, value, other fields, library link, side, anchor and angle, box around pads (a part
without: its graphics), index ranges of its pads/shapes/texts, `hidden`: its texts not shown, the file's and an edit's (`drawing.md`), `src`: its place among the
footprints of the text read, an added part's library text, or both for one F8 swapped; `fitted` is false for DNP,
BOM-excluded and KiCad 5 `virtual` parts, `placed` for DNP, position-file-excluded and
`virtual` ones; its shown 3D `models`: file as written, offset, scale, turns, `models.md`), shapes (`gr_*`/`fp_*`), zones (outline with cutouts, layers, net, priority,
id, name, clearance, minimum width, pad connection, thermal gap and spokes, island removal,
smoothing, `src` its place in the file; rule areas only when they keep zone copper out) and their
fills (`fills`, as KiCad draws them: `outlined` ones grown, `save-zones.md`), shown texts (hidden
dropped, `${VAR}`s filled in, locked footprint text `upright`, a part's `src`: its place among the part's texts in the file), `stackup: Vec<CuLayer>`, board
`thickness` and the stackup's solder mask and silkscreen colours (names as written),
`rules: Rules` (minimums, the Default net class for new tracks and vias, new zones' clearance,
mask and paste margins, via tenting; the mask's narrowest web, its gap to bare copper, whether a
part's pads may share an opening, the silkscreen's clearance), `classes` and `class_nets` (the project's other net classes
and the names or patterns of each one's nets, the first that fits a net winning: `class`,
`net_classes`, `clearances`, `track_width`), `severities` (the checks the project makes an
error, a warning or leaves out), the project's `track_widths` (W steps a route through them),
`grid_origin` (KiCad's Place → Grid Origin) and
`copper: LayerMask` (the layer table's copper layers). Layer and net ids index
`Board::layers` / `Board::nets` (net 0 = none); `file_nets` holds each net's name in the text read
(an edit renaming a net keeps its id: saving finds it by that name).

## Missing
Hatch settings, footprint zones, pads' own thermal gap and spoke width, Bézier pad primitives,
trapezoid delta, chamfered corners, a via's own tenting, drill offset, text boxes,
dimensions, bold/italic/TrueType/knockout text, non-ASCII glyphs, custom `.kicad_dru`
rules, net class patterns as regular expressions, KiCad 9's classes merged by priority.

## Geometry and lookups (`board.rs`)
- `board::{rotate, arc_center, arc_points}` — KiCad rotation convention; circle through 3 points;
  points along an arc within 5 µm (`MAX_ERROR`, KiCad's), as `(arc)` inside polygon points becomes.
- `Board::{find_net, net_bounds}` — net by name (any case; `scl` finds `/Sheet/SCL`), its box.
- `Board::{footprint_at, place_footprint, flip_footprint}`, `Footprint::holds`, `Pad::holds` — the part
  at a point (smallest box, side in view first); move/turn a part with its pads, graphics and texts,
  or put it on the other side as KiCad's Flip (`save-flip.md`);
  `Pad::copper` — a custom pad's whole copper (`Pad::custom` placed), which clicks and `reach` take in.
- `Board::copper_at(p, tol, layers)` — the pad, via or track that shows on top at a point: on the
  first of the copper `layers` listed as they show (the app's: the active one, then from the side
  in view), then pad before via before track; `copper_shown_at`: none that a zone's fill on a
  layer above hides, as a pointer picks (kria-k26: 0.44 ms, 0.01 of it the fills).
  `Board::zone_at` — the zone a click picks, on the same `layers`: by its outline (within reach),
  else its fill (highest priority) (kria-k26 ≤ 0.6 ms: hover runs it); `Board::fill_area` — its mm².

## Gotchas
- Pad and text angles in the file are absolute, positions footprint-local. Footprint angles are in [0°, 360°);
  `place_footprint` keeps pad and text angles there too (locked text in (-90°, 90°]); `diff` compares modulo 360°.
- Y points down; positive angles turn counter-clockwise on screen (`rotate`).
- Nets: `(net 5)` codes in KiCad ≤ 9, `(net "GND")` names in 10; both map to one NetId.
- Strings keep their escapes (`\"`); unescape only for display.
- `.kicad_pro` reuses rule keys elsewhere (`"clearance": "error"` severities, zone `min_clearance`): read
  each by its path (`json::Val::at`), as `apply_pro` does.
- Parsers end each list they enter with `c.close()`; KiCad ≤ 5 arcs (centre, start, angle) become start/mid/end.
- Text `(size H W)` lists height first. Locked footprint text shows upright (angle kept in
  (-90°, 90°]); `font.rs` constants were measured from KiCad 10 plots (skill `look`).
