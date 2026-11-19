---
name: fab-check
description: Check Cuprita's fab files (Gerber X2, Excellon) against KiCad's own - export both, rasterize them and diff each layer. Use after any change to `gerber.rs`, `fab.rs`, or to what a pad, via, zone, graphic or text plots as.
---
# Check fab files against KiCad

KiCad 10 is the flatpak `org.kicad.kicad`. Work in the session scratch dir `$S` (the sandbox
sees only `$HOME` and what `--filesystem` adds) and delete what you made.

## Export both
- KiCad's: `K="flatpak run --filesystem=$S --command=kicad-cli org.kicad.kicad"`, then
  `$K pcb export gerbers -l F.Cu,B.Cu,In1.Cu,…,F.Mask,B.Mask,F.Paste,B.Paste,F.SilkS,B.SilkS,Edge.Cuts
  -o $S/b/k/ <board>` (Protel names by default) and `$K pcb export drill --excellon-separate-th
  -o $S/b/k/ <board>`; positions: `pcb export pos --format csv --units mm -o $S/b/k/pos.csv`.
- Cuprita's, with the file's own fills as KiCad plots them (`cuprita --json … fab` refills
  first): a scratch cargo project with `cuprita-core = { path = ".../crates/core" }` that writes
  each `(name, text)` of `Board::parse(&src)?.fab_files(stem)` into `$S/b/e/`.

## Compare
`python3 .claude/skills/fab-check/gerbcmp.py $S/b/k $S/b/e --um 10` pairs the files by kind and
prints, for each, the area only one covers (beyond 1 px) and a few places (board mm, Y down);
20 µm for boards over ~80 mm. Then look at a place: `gerbcmp.py <kicad file> <cuprita file>
--um 5 --box x0,y0,x1,y1 --png $S/d.png` (box in Gerber mm: Y negated) and read the PNG
(KiCad only red, Cuprita only green). One file alone: `gerbcmp.py <file> --png $S/f.png`.

## Expected differences
- Text (silkscreen, and copper or mask text): the Hershey glyphs differ a little (`font.rs`).
- Chamfered and trapezoid pads plot as their rectangle; round corners of outline macros may
  stand out ≤ 5 µm (a few pixels at 5 µm). See `crates/core/fab.md` for the rest.
Anything else is a bug: find the item in the board file near the reported place.

## Learn a KiCad rule
Write a small board by hand (KiCad 8 format: `(kicad_pcb (version 20240108) (layers …)
(setup …) (footprint …))`) with the case in question, export it, and read KiCad's apertures:
`grep -E '^%ADD|^D[0-9]+\*|D03' $S/p/k/*.gts`. Its macros: `RoundRect` (radius, then the four
corners of the core), `RotRect`, `OutlineNP`, `FreePolyN`, `HorizOval`; `C`, `R`, `O` as usual.
This is how the mask and paste margin rules in `fab.md` were found.
