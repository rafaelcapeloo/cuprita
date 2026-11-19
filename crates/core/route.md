# crates/core: router (`route.rs`)

Interactive routing for the app's route tool (`X`) and the agent `route` command; other nets'
tracks and vias making way for it: `shove.md`.

## Flow
1. `Board::route_start(p, tol, layers)` — the pad (centre), via (centre) or track (end, or
   nearest point) `Board::copper_at` picks under a click: net, layer (the track's, else the first
   of `layers` the pad or via reaches: the active one), width (its net's class's, or the track
   started on; never below the minimum). `Board::joins_layers` says a pad or via there joins two
   layers, so a route changes layer without a new via.
2. `Router::new(board, net, layer, width, start)` — obstacles for that net and layer;
   `Router::on_grid(…, (at, step))` (the app's) puts its cells on the user's grid too.
3. Per pointer move: `snap(p, tol)` (own pad/via centre or track end, else nearest grid point),
   then `path(from, to)` → corner points; `connects(p)` says the end is on the net's copper.
4. `Board::add_route(shoved, route::tracks(path, …), via)` — the tracks, a via (`Board::via`:
   its net's class's size, through all copper; `Board::via_between(at, net, a, b)`: only the layers from `a` to `b`, blind or buried, as `Board::span` tells them) and what a shove moved, one undo patch; a via that
   breaks a rule (`drc::check_items`) is refused with it, nothing changed (`add_via` alone).
5. `Board::step_width(width, wider)` — the next of the project's track widths (else 0.1–2 mm)
   and the net classes', none below the minimum (W, Shift+W): a new `Router` for it.

## How it searches
- Grid through the start, step `(width + clearance) / 8` clamped to 10–50 µm; `on_grid`: through
  the user's grid points, its step the coarsest whole fraction of theirs that fine (31.75 µm for
  mil grids; none from 10 µm: as `new`). It covers the board outline's box (cells outside are
  blocked) and the start, or the copper + 10 mm with no outline.
- A cell is blocked when a track centred there would come within clearance (+ a ~2 µm margin
  for diagonal steps) of another net's track, via or pad (in or near a custom pad's copper), within
  the hole clearance of a hole (another net's, or any where its pad has no copper on the layer), or
  near the board edge (edge clearance). Zone fills are no obstacle: KiCad refills around tracks.
- A cell has two bits: blocked by anything, and by what a shove can't move. `search(…, through,
  settle)` reads the second to cross other nets' unlocked straight tracks and vias (`shove.md`).
- Blocked bits are rasterised per 64×64 tile the first time a search touches it and kept for the
  router's life; a custom pad a row at a time (the stretches near its outline), not cell by cell.
  Search marks are per-tile arrays made per search.
- A* over (cell, heading): steps 100/141, turns 45° +300, 90° +1000, sharper never; a 45°
  bend the other way round than `Router::diagonal_first` (diagonal last by default; `/`) +1,
  so of two as short the wanted one wins. Open states sit in a ring of 2048 buckets by
  estimated total (no heap). Exact, so paths are optimal for those costs; LIFO buckets reach
  the goal fast in open space.
- Only the path's ends may sit closer than the clearance: the start, and a `to` that
  `connects`. A blocked `to` elsewhere becomes the nearest clear cell on the line back to the
  start; after 200 000 settled states the path stops at the closest cell reached.

## Speed (`examples/route.rs`, pads 3–40 mm apart on fully routed boards)
kria-k26 with its project rules: mean 5.5 ms, worst 15 ms per path; the worst cases are
unreachable goals that use the whole search budget. hackrf-one: mean 7 ms, worst 14 ms. On a
0.25 mm or 10 mil grid (`on_grid`, aims on it 0.3 mm off the pads) the same: kria-k26 mean
5.4 ms, worst 15.5; hackrf-one mean 7.6 ms, worst 12.9. glasgow-revD1's 6.4 mm custom pad cost
37 ms to rasterise cell by cell; row by row it is gone from the worst cases.

## Gotchas
- A `Router` caches what it learned: build a new one when other nets change (the app drops
  the active route when a part moves, and builds one after a fix that shoved).
- Paths end on grid points except at a `to` that connects (a short last leg to the centre); on
  the user's grid, one from a pad off it starts with a short leg onto it, inside the pad.
- `tests/corpus.rs` routes hackrf-one pads and requires DRC-clean middle segments.
