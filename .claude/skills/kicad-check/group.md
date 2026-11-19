# kicad-check: items edited together

Part of `SKILL.md`: oracles for `Board::{move_items, turn_items, flip_items, delete_items}`
(`crates/core/history.md`).

- Pick in Cuprita, write a manifest of the picks as found in the original (a part by reference and
  anchor, a track by its ends and width, a via by its place, a zone by its first corner and a
  layer), save the edited board; in KiCad find them in the original the same way, taking each
  match out of the candidates (two zones can share a first corner).
- KiCad's own edit: `item.Move(pcbnew.VECTOR2I(dx, dy))`, then `item.Rotate(centre,
  pcbnew.EDA_ANGLE(90, pcbnew.DEGREES_T))` for every item; `board.Remove(item)` to delete.
- Compare every part (its pads), track, via and zone as sorted tuples, both boards through a
  `pcbnew.SaveBoard` and `LoadBoard`: KiCad mends copper left on another net's copper (a short)
  as it saves and reads, picking the net by the items' order, so a few such items may differ by
  net alone (Cuprita writes moved tracks after the file's last); geometry and counts must not.
