# crates/core: zones filled alone (`Board::{add_zone, set_zone, refill}`)

Part of `fill.md`. App `Z`, MCP `zone`; `Patch::added` undoes it.

KiCad 10's settings: clearance `Rules::zone_clearance` (the project's zone `min_clearance`, KiCad
5's `zone_clearance`, else 0.5 mm), minimum width 0.25, thermal gap and spokes 0.5 mm. Its id is
one below the board's least, so it ranks last. Filled alone, around the fills it keeps out of
(`fill.md`, step 2) as they are: with every fill current, as `fill_zones` fills it (so does KiCad
10). kria-k26 60 ms for a 20 mm square, 0.1 s for a whole inner layer (redrawn).

## Edited (`zone.rs`, app: the picked zone's corners)
- `Zone::{move_corner, add_corner, remove_corner, add_cutout}` edit a copy of the outline (ring 0,
  then the cutouts) and keep it only while it stays simple, as KiCad's editor wants: the edges
  changed cross, touch or fold back on no other; every cutout inside the outline, none inside
  another (a ring's first corner tells, once no edge crosses). Else nothing changes, and it says
  what to do. A corner dropped on one beside it goes; the outline keeps three, a cutout left
  with two goes. Only the changed edges are checked: O(their count × the zone's corners).
- `Board::set_zone(i, z)` puts the copy (its outline or settings edited: net, name, priority,
  clearance, minimum width, pad connection, thermal gap and spokes) in place, filled alone as a
  new zone; `Patch::zone` (its zone and fills) undoes it, moving the fills of the zones after it
  when their number changed. Lower zones it overlaps keep their fills until B: a refill of all.
  kria-k26's whole In3.Cu plane: refilled alone in 40 ms, 0.11 s redrawn and re-checked.

## After an edit (`fill/refill.rs`)
- `Board::refill(undo)`, given the patch undoing an edit, fills again the zones whose fill it can
  change: on the layers of the copper it put in, changed or took away (`Patch::changed`), their
  outline (grown by what they keep clear) meeting where it is or was; then those keeping out of
  fills that changed, near where they did. Not the zones the edit made (`Patch::zones_put`),
  which keep the fills it gave them; all, when the board's edge changed.
- Where that is under a quarter of a zone (`Ctx::window`: the box the changes reach, with the
  spokes there, worked out in one holding all that decides it), only its fill there is made
  again (`refill_layer`); else the zone fills whole. Its patch goes into the edit's
  (`Patch::then`): one undo step, undone to the very fills read (saved byte for byte).
- Used by every board edit, the app's (`App::edited`) and the agents' (`Edits::push`); B still
  fills all. Tests: after each refill, `fill_zones` changes nothing; the corpus' ~200 items
  moved, turned and deleted per board, refilled, re-check as full checks and undo as read.
  KiCad 10's DRC without a refill finds no new error after glasgow-revD1's shove.
- A part moved, with its refill (12 threads): hackrf-one mean 1.5 ms, worst 23; jetson-orin 1.7,
  14; kria-k26 14, 38; glasgow-revD1 20, 144 (its ground plane, whole).
