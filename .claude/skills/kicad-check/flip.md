# kicad-check: flipped and swapped parts

Part of `SKILL.md`: oracles for `Board::flip_footprint` (`crates/core/save-flip.md`) and F8's
footprint swap (`crates/core/update-parts.md`).

- Flip: `fo.Flip(fo.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)` is KiCad's F (its "Flip
  board items L/R" preference is off by default); a turn and a move after it as for a moved part.
- Swap: `fp = pcbnew.FootprintLoad(dir, name)`, `board.Add(fp)` (a footprint on no board crashes
  `Flip`), `fp.SetPosition(old.GetPosition())`, `fp.Flip(...)` for a back part, then
  `fp.SetOrientationDegrees(old.GetOrientationDegrees())`. Its FPID names no library: compare
  `Library:Name` by hand. The old part's reference text, fields and attributes are the oracle for
  what stays; the old pads' nets, by number, for the new pads'.
- KiCad's flip in memory keeps a `*.Cu` pad on the board's copper only, while a file's `*.Cu` reads
  as all 32 layers: compare layer sets on the layers `board.IsLayerEnabled`.
- A rect turned off a right angle is a polygon in KiCad: compare its corners as a set. KiCad swaps
  a flipped arc's start and end: compare them unordered, and its mid.
- Boards before KiCad 8: KiCad adds empty Footprint, Datasheet and Description fields wherever on
  reading, and KiCad 6/7 fields without `(at)` land anywhere: skip hidden fields there.
- Every part of every corpus board flipped (and flipped, turned, moved), saved and compared item
  by item: a few minutes in all, kria-k26's 262k items the longest.
