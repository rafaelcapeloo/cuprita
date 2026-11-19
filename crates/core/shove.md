# crates/core: shove (`shove.rs`)

Other nets' tracks and vias make way for a track being routed, as KiCad's Shove mode does: the
app's route tool (its default; P walks round instead) and the agent `route` command (`shove`).

## Flow
1. `Shover::new(board, net)` — an R-tree of every track, via, pad and edge piece; per route, again
   after a fix (others moved, or the route's own new copper, which others keep clear of).
2. Per route step, `Shover::step(board, router, (from, to), width, layer)`: the router's path
   across what a shove moves (`Router::search(…, through)`), and `shove`; where that can't make
   room, the path round everything with the `Stuck` why (what can't move, where). Both searches
   settle at most 3/4 of a walk round's 200 000 states (the first at most half).
3. `Shover::shove(board, path, width, layer, via)` → `Shoved` (each run's tracks and its corners
   now, each via and where it goes) or `Stuck`; `via` is the route's new one (V, a layer switch).
4. `Board::add_route(&shoved, tracks, via)` makes it all as one undo patch (`route.md`).

## How it moves things
- Never moved: pads, holes, the board edge, arcs, locked tracks and vias (`Track::locked`), the
  routed net's copper, and copper touching its net other than where tracks end on it (a track
  across its net's via, a via on a pad): moved, it would leave it.
- A track in the way moves as its run: the tracks joined end to end through joints of just two
  tracks of the net, as wide, with no via or pad there, 32 at most each way; a run's ends stay.
- The run goes round the room the copper pushing it keeps (an octagon about each segment: the
  clearances and half the widths, + 5 nm), along its sides, leaving and joining its course at 45°
  where it turned sharper; of the two ways round, the shorter clear of what can't move. A run
  the route crosses goes round the route's end.
- A via in the way moves to the nearest side of that room; the tracks ending at its centre come
  along (a 45° jog back onto their line where it fits), then go round as runs do.
- The route pushes first, then each item moved, with what of it moved. Each gives way to the
  route and the items moved before it and pushes those after (one in an earlier one's way goes
  round it itself): nothing pushes back, nothing swings to and fro. A way round that would run
  into the route or into an item moved before it is no way: stuck, and the step walks round.
- Kept as DRC measures (0.5 µm short allowed): the clearance between copper of nets apart on a
  layer, the edge clearance, the hole clearance (other nets' copper from any hole, every layer),
  the hole spacing. More than 64 pushes: stuck ("more than 64 tracks and vias would move").

## Checked
- Unit tests on small boards, DRC of each result: a track pushed aside, one crossed, a chain, 20
  nested, a via with its track, the route's via, the limit, what can't move, no way round, undo.
- KiCad 10 (skill `kicad-check`): 24 shoves on copies of kria-k26 (KiCad 7), glasgow-revD1 (10) and
  hackrf-one (6), runs of up to 52 tracks and 10 vias moved, half the routes ending on a via:
  pcbnew reads Cuprita's track and via counts; `kicad-cli pcb drc --refill-zones` finds no new
  clearance, hole clearance, edge or unconnected item (only the test routes' own open ends).

## Speed (`examples/route.rs`, release, one thread)
A shoving step (both searches, the shove): pads 3–40 mm apart, kria-k26 mean 4.8 ms, worst 13.1;
glasgow-revD1 mean 5.6, worst 12.2; hackrf-one mean 6.6, worst 12.3. Steps 1.5 mm beside the
pads, where it moves others most: mean 0.2–0.4 ms, worst 8.6. `Shover::new` with the router: 1.8 ms
on kria-k26.

## Gotchas
- A shove starts from the board as it is: each step's preview is computed afresh, none piles up.
- Zone fills follow: the step that makes the route refills the zones it changes (`Board::refill`).
- Shoved tracks are saved as new items (new ids); a moved via keeps its text but `(at)` (`save.md`).
- The router's crossing plane takes every unlocked straight track and via of another net as
  movable; where the shove then finds one can't move (touching its net), the step walks round.
