# crates/core: moving, turning, deleting what is on a sheet (`schematic/{pick, placed}.rs`)

Part of `schematic-edit.md`. Each is an edit of a sheet's text that undoes itself (`Patch`).

## Picking (`pick.rs`)
`Item`: a symbol (one unit), wire, net label (local, global, hierarchical) or no-connect flag, by
its place in the sheet's order. `Schematic::item_at(p, tol)`: the label whose box holds `p`, else a
flag's cross, else the nearest wire, else `symbol_at`'s symbol; `items_in(a, b, touching)`: what a
box holds whole, or touches too (left to right or right to left, as KiCad's; a slanting wire
touches a box its line crosses); `extent` (a label's: its text's, `font::bounds`), `describe`
(`R5`, `Global label VCC`), `names(i)` (label `i` is on a pin, wire, bus, bus entry or sheet pin).

## `Project::move_items(page, items, by)`, `Project::turn_items(page, items)`
- Any mix at once; `by` on the grid; a quarter turn counterclockwise about a lone symbol's origin,
  a lone label's point, else the grid point nearest their middle. Returns the undo patch and the
  items as the page has them now (wires found again by their ends).
- Symbols: `(at)`, fields (angle +90°, a mirrored one's −90°, in its frame). Labels: point, angle,
  `justify` (left at 0°/90°, right at 180°/270°), fields. Wires: ends. Flags: their point alone.
- Connections hold, none is made by the way. Points that move: pins, wires' ends, junctions and
  labels along moved wires (a label moved itself drags nothing):
  - a wire with an end there keeps its part up to the first junction or label on it, and a new
    stretch reaches the new place; with none between, the wire is the stretch's first part (level
    where it can; gone when the point lands on its far end);
  - what stays on an old place (another symbol's or a sheet's pin, a bus entry, a wire passing
    with a junction, two wires' ends) gets a new wire to the new place, unless one runs there;
  - wires with both ends moving go whole; no-connect flags go along, labels and junctions too
    where nothing stays;
  - a stretch is straight, else bent once either way, else slanting: the first whose corners and
    lines meet no point where something joins nor lie along a wire or bus, bent on none of them.
- Junctions as KiCad's editor keeps them: added where a moved point (or an old one a wire now
  runs through) needs one (an end, pin or bus entry's end on a wire's middle; three ends); gone
  from old places where no longer needed.
- Refused, saying where: no clear stretch; a pin, end, junction or label landing on a bus (only
  bus entries and bus labels join one); a wire going whole lying along one that stays.
- A pin or end landing where something joins joins it, as in KiCad: the user's own doing; a label
  moved itself names what it lands on.

## `Project::delete_items(page, items)`
Symbols' parts, every unit on every sheet (a sheet used twice once, a KiCad 6 root's
`symbol_instances` entries), wires, labels, no-connect flags, each with the space before it. What
was wired stays; junctions there no longer joining three or more go, as KiCad cleans them up.

## Checked
- `placed.rs` tests: symbols moved, turned (mirrored too); a wire dragged across and along, a
  T sliding, turned, landing; labels moved, turned, deleted; deletes and their junctions; a part
  picked twice; undo byte for byte. `pick.rs`: what a click and a box take.
- The four corpus projects, one edit at a time (29,000): every wire and label moved 2.54 right and
  5.08 down, and turned; every symbol with the wires on its pins too: no net split; joins only
  where a pin, end, junction or label lands on something; undo byte for byte. Of these blind
  moves 19% are refused (mostly a wire landing along another, two rows down).
- KiCad 10 (skill `erc-check`): every 7th wire, label or symbol with its wires moved, the next
  turned, the next deleted (6,000 edits, those joining nets taken back): netlists equal Cuprita's
  but two of kria's (a hierarchical bus label moved off its bus) and a few names (`nets.md`).

## Speed
An edit 2–6 ms (glasgow-revD1's level shifters, used in 4 places: ≤ 9.4 ms, as placing a symbol).

## Missing
Stretches routed round what is in the way (refused instead); KiCad 7's orthogonal drag (a level
wire moving along with a pin); moving buses, bus entries, junctions, sheets. Mirror: `schematic-clip.md`.
