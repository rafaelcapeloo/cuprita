# crates/core: design rule checks (`drc.rs`)

## Entry points
- `drc::check(board)` — every violation of the board's rules: `clearance` (two nets keep the
  larger of their classes', `Board::clearances`), `copper_edge_clearance` (from Edge.Cuts and
  from unplated slots, milled as it is), `hole_clearance` (copper to a hole of another net or
  an unplated one; one said of a pair, its copper's clearance first), `solder_mask_bridge` (two
  openings of nets apart with no mask left between them, or copper of another net bare in one,
  front and back), `track_width`, `via_diameter`, `drill_out_of_range`, `annular_width`,
  `hole_to_hole` (not two blind or buried vias that share no layer, as KiCad), `courtyards_overlap` (parts' F.CrtYd or B.CrtYd outlines; names no item).
- Warnings, as KiCad starts them (`Violation::warning`): `track_dangling` and `via_dangling`
  (`dangling`: an end on no copper of its net, a via its net meets on one layer alone, as
  KiCad's connectivity counts them), and the silkscreen's (`drc/silk.rs`): `silk_over_copper`
  (ink over a pad's or untented via's mask opening), `silk_overlap` (ink over other ink, but a
  part's own graphics), `silk_edge_clearance` (ink across the edge). Make counts none of them.
- The project's `rule_severities` say otherwise (`judged`): a kind an error, a warning, or not
  told at all (kria-k26 ignores silkscreen over pads; hackrf-one keeps 0 mm from the edge).
- `drc::check_items(board, items)` checks only those; `drc::recheck(board, old, undo)` updates
  the list after a change: the items it changed, the loose ends near it, the ink where it was.

## Speed (kria-k26, release)
Whole check 44 ms on its cores (the pairs in runs of items, loose ends and ink beside them:
137 ms on one; 59 ms before the last six checks); `kicad-cli` 5.25 s. Re-check after moving its
242-pad part 24 ms, after deleting a part 8 ms. A fill is asked whether an end is in it by its
edges in rows (`Rows`), not its tens of thousands of points.

## Checked
`tests/corpus.rs`: counts per kind against `kicad-cli` on four boards; a re-check after a move
and a delete equals a whole check. By hand with their `.kicad_pro` (skill `corpus`), KiCad
10.0.8 kind for kind on hackrf-one, glasgow-revD1, kria-k26 and jetson-orin: the same edge
crossings of ink (13, 77, 10, 3), ink over pads (5, 11), hole clearances (1, 3), courtyards
(17, 27), no loose ends; and on boards made for it, a class's clearance, six mask bridges.

## Missing
Texts in a TrueType face or knocked out aren't checked as ink (drawn here in the stroke font),
and the stroke font is near KiCad's, not it: a text must lie a twentieth of its height deep to
be told. An unplated hole in a pad's opening is no mask bridge here (hole_clearance says it);
openings drawn on the mask layers, net ties and zone copper under an opening aren't looked at.
Zone fills aren't measured to other copper (KiCad does: a file's stale fills), nor same-net
zones that overlap (`zones_intersect`), thermal reliefs short of spokes (`starved_thermal`),
fill islands (`isolated_copper`). A drilled custom pad's ring is measured on its anchor.

## Gotchas
- Courtyards may overlap by 10 µm (`TOUCHING`): parts placed touching overlap that much once
  rounded, and KiCad passes 7.5 µm (glasgow-revD1) but reports 20 µm (kria-k26).
- Clearances (copper, edge, hole to hole) may fall 0.5 µm short, as in KiCad (`SLACK`): copper
  routed at the limit measures a little under it. Sizes (width, drill, ring) are exact. Ink
  that only touches isn't over (`RBox::collides`), and a curve's must lie 3 µm deep (`SKIN`).
- Arc tracks measure exactly to arc tracks (`arcs_apart`), to the rest by 32 chords.
- Custom pads measure to all their copper (`Pad::copper`, `RBox::dist_poly`); their R-tree box
  is `RBox::pad`, the box around it. Checked against `kicad-cli` on a probe board.
- Violations name items by index: `drc::recheck` moves them with the patch (`Patch::moved`), as
  a delete or its undo moves the items after it.
- A change to Edge.Cuts (a new outline, a part drawing a notch of it moved) re-checks every item:
  the edge each keeps clear of moved (`Patch::edge`).
