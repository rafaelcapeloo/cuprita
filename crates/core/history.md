# crates/core: undo, redo and items picked together (`history.rs`, `picked.rs`)

Parent: `STATE.md`. ROADMAP decision 3: undo keeps what each edit replaced, not board snapshots.

## Patches (`history.rs`)
- `Patch`: what undoes an edit, list by list (nets, tracks, vias, parts, pads, graphics, texts,
  zones, fills): each list's splices in order and apart, each where the list has its items
  before any is made (made at once). In place, or alone, a splice changes its list where it is;
  else the list is built again once (deleting thousands of items moves the rest once).
- Made before an edit: `Patch::{footprint, item, added, fills, update}`; `Patch::delete(b, Pick)`
  takes a part (with its pads, graphics, texts), track, via, zone (with its fills) or one of the
  board's own texts or drawings (`drawing.md`) away; the patches of several picks, made from one
  board, `collect` into one: one undo step.
- `Board::apply` makes a patch and returns the one undoing it; the parts it didn't put in follow
  their items by where its splices lie (`span`: ranges moved; a part without items stays after
  those of the parts before it), pads are renumbered, zones' fills ranged again.
- What re-checks take from the patch undoing a change: `moved` (where an item before it is
  after it, or gone), `items` (tracks, vias, pads put in or changed), `nets` (whose ratsnest to
  join again: of those and of the zone fills), `edge` (a line on Edge.Cuts drawn, moved or
  gone, a part's too: re-check everything); a refill (`fill-new.md`) takes `changed` (what it
  put in, changed or took away, as is and was) and `zones_put`; `then` joins its patch to the
  edit's, one undo step. A board's fills are its zones', in their order: `apply` ranges them so.
- `History::{push, undo, redo, peek, mark_saved, unsaved, restored}` of what is `Edited`: a
  `Board`, or with `History<schematic::Patch>` a schematic `Project`; `undo`/`redo` give the
  patch the re-checks take.

## Items picked together (`picked.rs`)
- `Board::items_in(a, b, touching, layers, shown)`: KiCad's selection box. Left to right
  (`touching` false) it takes what it holds whole, right to left what it touches: parts by their
  box, tracks and zone outlines by their lines (a stretch crossing it), vias by their round; only
  the tracks, vias and zones on the copper `layers` shown, the board's own texts and drawings on
  the layers `shown`.
- `Board::extent(picks)` — their box; `Board::hits(pick, p, tol)` — a press on one (a part's
  box, a track or via, inside a zone's outline or on it).
- `Board::{move_items, turn_items, flip_items, delete_items}` — one edit each, returning the
  patch undoing all of it: parts with their pads, graphics and texts, tracks (arcs too), vias,
  zones with their fills; a turn about a point exact for right angles, parts' angles along;
  a flip puts the parts (only) on the other side as KiCad's Flip, mirrored top to bottom about
  the line through a point, so together they mirror. Picks given twice count once; a part's
  text alone moves and turns as the board's own, and Delete hides it (`drawing.md`).
- Saved: parts as moved ones, tracks and vias moved written anew (`save.md`), a zone's outline
  in place of its own (`save-zones.md`).

## Checked
- `history.rs`: items apart deleted and changed as one patch, undone and redone exactly;
  `picked.rs`: the box rule, four quarter turns back to the board read (re-checks as full
  ones each turn), a flip and back, a move and turn saved and read back, undone byte for byte.
- `tests/corpus.rs`: ~200 items spread over each corpus board moved, turned and deleted: the
  re-checks find what full checks find (it found parts drawing Edge.Cuts and the ratsnest's
  island order); saved, read back as the board has them; all undone, byte for byte.
- KiCad 10 (skill `kicad-check`): those edits of 7 corpus boards (KiCad 4–10) saved, every part
  (its pads), track, via and zone is where KiCad's own Move, Rotate and Remove put them, as many;
  copper the edit left on another net's copper may read with that net (KiCad mends shorts).

## Speed (release, kria-k26 without its project: 41.6k violations)
210 items round U36 moved, turned or deleted: the edit 0.03–1.6 ms, DRC re-check 3.5 ms (`moved`
by binary search: 15–17 ms walking), ratsnest 5 ms; in the window 11–16 ms with the redraw.
