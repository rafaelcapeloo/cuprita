---
name: kicad-check
description: Check that KiCad reads a board Cuprita saved the way Cuprita meant it - load it with KiCad 10's pcbnew Python and compare with KiCad's own edit. Use after any change to saving (`crates/core/src/save.rs`) or to what an edit writes.
---
# Check a saved board in KiCad

KiCad 10 is the flatpak `org.kicad.kicad`; its Python has `pcbnew`. Work outside the repo
(the session scratch dir) and delete what you made.

## Make edited boards
Copy a corpus board out, then edit and save it with the agent CLI:
`target/release/cuprita --json /s/b.kicad_pcb '{"cmd":"place","reference":"U1","at":[100,80],"angle":90}'
'{"cmd":"route","from":[x,y],"to":[x2,y2],"via":true}' '{"cmd":"save"}'`, or for many boards a
scratch cargo project with `cuprita-core = { path = ".../crates/core" }` that calls
`place_footprint`, `add_tracks`, pushes a `Via` and writes `Board::write(&src)`, printing what
it placed (reference, anchor nm, angle; track ends, width, layer and net names).

## Load them in KiCad
`flatpak run --filesystem=/s --command=python3 org.kicad.kicad /s/check.py` (the sandbox sees
only `$HOME` and what `--filesystem` adds; drop the noise: `grep -vE 'property.h|Debug: Adding'`).
The oracle is KiCad's own edit of the original board:
```python
import pcbnew
orig, mine = pcbnew.LoadBoard(original), pcbnew.LoadBoard(saved)  # raises if KiCad can't read it
fo, fm = orig.FindFootprintByReference(ref), mine.FindFootprintByReference(ref)
fo.SetOrientationDegrees(angle); fo.SetPosition(pcbnew.VECTOR2I(x_nm, y_nm))
for a, b in zip(fo.Pads(), fm.Pads()):  # GetPosition() .x .y in nm, GetOrientationDegrees()
    ...
texts = lambda f: list(f.GetFields()) + [g for g in f.GraphicalItems() if g.GetClass() == 'PCB_TEXT']
# texts: GetTextPos(), GetDrawRotation().AsDegrees(); zones: f.Zones(), z.Outline().COutline(0)
# .CPoint(i); tracks: mine.GetTracks(), GetClass() 'PCB_TRACK'/'PCB_VIA'/'PCB_ARC', GetStart(),
# GetEnd(), GetWidth(), GetLayer() == mine.GetLayerID(name), GetNetname(); vias: GetPosition(),
# GetWidth(pcbnew.F_Cu), GetDrillValue(), mine.GetLayerName(v.TopLayer()), BottomLayer().
```
Compare positions within 2 nm and angles modulo 360°; also count tracks and vias. A part added from a
library: the oracle is `fp = pcbnew.FootprintLoad('/app/share/kicad/footprints/<Lib>.pretty', name)`,
`fp.SetPosition(...)`, `fp.SetOrientationDegrees(a)`; compare `GetFPID()` too. Flipped and F8-swapped parts: `flip.md`; items edited together: `group.md`.

## Zone fills
KiCad's own refill is the oracle (fills saved in corpus files are stale, up to 13% off):
`b = pcbnew.LoadBoard(src); pcbnew.ZONE_FILLER(b).Fill(b.Zones()); pcbnew.SaveBoard(out, b)`,
then `cargo run --release -p cuprita-core --example fill -- out.kicad_pcb` refills it and prints,
per zone and layer, the area only one of them covers. To see where, draw both fills' polygons
(e.g. PIL) over the window of the biggest difference. To learn a rule, build a small board and
probe KiCad's fill: `z.GetFilledPolysList(layer).Contains(pcbnew.VECTOR2I(x, y))`. A pad's
outline: `pad.TransformShapeToPolygon(poly, layer, 0, 5000, pcbnew.ERROR_INSIDE)` into a
`pcbnew.SHAPE_POLY_SET()` (`GetPrimitives(layer)` can't be iterated from Python).
Saved fills: load a board Cuprita refilled and saved, `pcbnew.SaveBoard` KiCad's view of it and
run `examples/fill` on that (exact from KiCad 6). KiCad prints `Legacy zone fill strategy is not
supported anymore` when it inflates fills drawn with an outline (`crates/core/save-zones.md`).

## Gotchas
- `GetLayerName` gives the user's name (`C1F` for `F.Cu` on hackrf boards): compare layer ids.
- KiCad 6/7 footprint properties without `(at)` load at arbitrary places: skip hidden fields
  when `mine.GetFileFormatVersionAtLoad() < 20240108`.
- On load, KiCad gives a track with no net, or one lying on another net's pad, the net of the
  pad it touches: start test tracks on a pad of their own net.
- `FOOTPRINT.GetFields()`, not `Fields()`; a via's width takes a layer.
- Deleting in KiCad (`board.Remove(f)`) hands `f` to Python: keep it referenced to the end, or
  later footprints come back as bare `SwigPyObject`s; drop the `memory leak` lines it prints.
