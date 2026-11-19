# crates/core: differential pairs and length tuning (`pair.rs`, `tune.rs`)

Two nets that carry one signal: found by name, routed side by side, made as long as each other.
Parent: `STATE.md`; the single-track router it builds on: `route.md`.

## Pairs (`pair.rs`)
- `other_side(name)`: the name that pairs with a net's, by KiCad's rule (`DRC_ENGINE::MatchDpSuffix`):
  the last character before any digits and underscores swapped, `+`/`-` or `P`/`N`. `Board::pair(net)`
  is a `Pair` when the board has that net: positive then negative, with the class's
  `diff_pair_width` and `diff_pair_gap` (the positive net's class; `Rules::class_pair_*` for
  Default), the gap never under the clearance the two keep from each other, which DRC holds
  them to as any two nets.
- `Board::pair_ends(at, tol, layers)`: where a pair's route starts or ends: the pad, via or track
  end there (as `route_start` finds one) and the nearest of the other net's on that layer within
  5 mm; `Ends` holds both points (positive first) and how far their copper reaches.

## Routing a pair (`PairRouter`)
- The middle line is searched as one track `2·width + gap` across by a `Router` whose own copper
  is both nets' (`Router::pair`), on a grid no finer than 0.3·(width + gap), anchored where the
  line leaves the starting ends: square out of the line through them, past their copper, on the
  side the pointer is (until a click fixes a corner, it may change sides).
- `to(board, p, tol)` gives what would be laid (`Laid`): the middle line to the grid point at
  `p`, or, with the pair's other ends under it, to the grid point a step short of them, sideways
  at 45° onto the line square into them, and in between them (pads 0.5 mm apart leave a 0.2/0.25
  pair 25 µm of room: it must come in centred). Points that bend the line by nothing go; a stretch
  too short for the corners at its ends (two bends the same way) gives way to one corner where
  its neighbours meet (`relax`). The two tracks are the lines `(width + gap)/2` to each side
  (`beside`: corners where the sides of two stretches meet), each joined to its own end by a
  stub; the side that starts nearer the positive end is the positive net's.
- Refused (`Laid::refused`), with why: a turn back sharper than a right angle; arriving crossed
  (the positive track would end on the negative pad). `fix` keeps the line as a click's corner,
  `back` takes one back.
- `Board::add_pair(from, laid)` adds the tracks, one undo patch; checked as any tracks
  (`drc::check_items`), a rule broken refuses them, nothing changed.
- shortcut: one layer, no via pair; other nets don't make way (no shove); a pair read from a
  file is not followed as a pair when one of its tracks is dragged.

## Tuning (`tune.rs`)
- `Board::net_length(net)`: its tracks' lengths together (vias' heights not counted).
- `Board::tune(i, target)`: straight track `i` gets a meander so its net is `target` long: turns
  to one side, legs `max(3·width, width + clearance)` apart, as tall as it takes up to 1 mm
  (four widths of a wide track), corners cut at 45° by a quarter of the spacing; starts one
  spacing in from each end. The side where the new tracks keep the rules, else the other.
  `Tuned`: the undo patch, the tracks in the track's place, the turns, the length reached
  (within a few nm: points are whole nm).
- Says why not: long enough already; less to add than the smallest turn; more turns than the
  track has room for; an arc; no room on either side (with the rule it would break).

## Checked
- `pair.rs` tests: names as KiCad pairs them; ends found from either pad; a pair routed past a
  pad in its way into pads 0.5 mm apart: both nets joined, no rule broken, the gap the pair's
  wherever the two run side by side, undone exactly; crossed pads and nothing to lay refused.
- `tune.rs` tests: a net brought to 23.3 and 20.5 mm within 20 nm, ends kept, the other side
  taken when the first is shut, each refusal's words.
- KiCad 10.0.8 (`kicad-cli pcb drc`): a board with a USB pair routed between two pin headers
  (46.232 mm each side) and both sides then tuned to 50 mm: 0 violations, 0 unconnected; the
  lengths summed from the saved segments are 50.0 mm each.
- In the window (replay): 6, a pad, a corner, the pair's other pads; Inspect's Pair with, Skew.
