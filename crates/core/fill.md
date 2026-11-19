# crates/core: zone filling (`fill.rs`)

`Board::fill_zones()` fills every copper zone again, like KiCad's "Fill All Zones" (app `B`, MCP
`fill`; `Patch::fills`, made before, undoes it): it replaces `Board::fills` and each zone's
`fills` range with fills as KiCad has them, which `Board::write` saves (`save-zones.md`), and returns
how many zones' fills changed. `cuprita_render::rebuild` draws changed fills again.

## How (each zone on each of its copper layers)
1. Area: the outline (cutouts are polygons inside it, even-odd), convex corners chamfered or
   filleted, kept inside the board (Edge.Cuts loops, when they all close).
2. Knockouts, each grown by `r` (half the minimum width, less 1 nm) while the area shrinks by `r`:
   other nets' pads, tracks, vias, copper graphics and text (the largest of the zone's, a pad's
   own and the clearances of the zone's net's class and the other's); holes without copper (`hole_clearance`, or the pad's own);
   the edge (`edge_clearance`); keepouts (`copperpour not_allowed`) as drawn; fills of other
   nets' zones ranked above (the larger zone clearance): higher priority, then greater id (UUID),
   and of its net's zones of higher priority (no clearance), as KiCad 10. A zone without a net
   keeps clear of every net. Its net's pads: solid joins, none keeps clearance, thermal cuts a
   gap. Custom pads count with all their copper (`Pad::custom`).
3. Growing what is left by `r` drops copper narrower than the minimum width.
4. Thermal spokes, `thermal_bridge_width` wide at the pad's `thermal_bridge_angle` (45° on round
   pads and, from KiCad 7, round-anchored custom ones; else 90°), run from the pad centre (or its
   hole's edge) to the gap's edge. One stays where the fill without gaps would reach its end, it
   doesn't end on another pad of the net, and its part off the pad keeps clear of other nets.
   A custom pad's run as on its box (in its own frame), from the box's centre, also where they
   miss its copper. KiCad 10's rules, found by experiment.
5. Islands go (`island_removal_mode`, `island_area_min`) unless they touch copper of the net, or
   none does: a pad's or via's centre, a track's or spoke's end, a pad, via or track reaching in,
   a fill of the net filled before (higher priority). KiCad: joined to a pad through any copper.
6. Holes join the outline by zero-width cuts (`fracture`), as KiCad saves fills.

Waves, a thread per core (the browser uses one): a zone fills after the fills near it that it
keeps out of, as late as the zones waiting for it allow (the big planes last).
Curves become polygons within 5 µm (KiCad's default): outside the knockouts, inside the fills.
Polygon booleans and offsets: `i_overlay` (integer engine on our nm, within ±1.07 m).

## Checked
- `fill.rs` tests: five boards with KiCad 10's fill area (within 0.02–0.2 mm²), clearances,
  spokes, necks, islands and what keeps them, edge, holes, priorities, ids, custom pads; `polygon.rs`: fracture.
- `tests/corpus.rs`: every corpus board refilled; no pad, via, track or custom pad corner of
  another net in a fill.
- Against KiCad 10's own refill (skill `kicad-check`), area only one of them covers: kria-k26
  0.05%, hackrf-operacake 0.16%, jetson-orin 0.18%, hackrf-one 0.22%, glasgow-revD1 0.55%,
  framework-mcu 1.5%; new 16 mm zones over another net's plane within 0.4 mm². Left, as KiCad
  does it: spokes kept in narrow channels (most of glasgow-revD1's 48 mm² on B.Cu); necks within
  the 5 µm drawing error; islands kept that the minimum width cut off (its DRC flags them) or
  that only meet a lower zone of the net (jetson-orin 4 mm²), dropped that only copper no pad
  reaches touches (a lone via); of same-net zones of one priority overlapping, one left empty.

## Speed (release, 12 threads)
kria-k26 0.27 s (KiCad 10: 2.2 s; one core 0.76 s), jetson-orin 75 ms (5.8 s), glasgow-revD1
0.13 s (4.1 s), hackrf-one 30 ms (1.0 s). Slowest job: a kria plane whose 683 track knockouts
take 0.12 s to merge (the fragment solver halves it). `cargo run --release -p cuprita-core --example fill`.

## New and edited zones, and after an edit
`Board::{add_zone, set_zone, refill}`: `fill-new.md`.

## Missing (`// shortcut:` in the code)
Hatched fills and zones on no copper layer (they keep their fill), custom pads' convex-hull
clearance and Bézier primitives, footprint zones, pads' own thermal gap and spoke width,
`remove_unused_layers` (rings count everywhere).
