---
name: sch-check
description: Check what Cuprita draws of a KiCad schematic against KiCad's own plots - text by text and as overlaid images. Use after any change to `crates/core/src/schematic*`, `font.rs`, or `build_sch` in `crates/render`, or to learn how KiCad places something on a sheet.
---
# Check schematic drawing against KiCad

KiCad 10 is the flatpak `org.kicad.kicad`. Work in the session scratch dir `$S` (the sandbox
sees only `$HOME` and what `--filesystem` adds) and delete what you made. Projects to check: the
corpus boards' (`corpus/boards/<name>/`, skill `corpus`) and KiCad's 115 demo sheets in
`/var/lib/flatpak/runtime/org.kicad.kicad.Library/<arch>/stable/active/files/demos/`.

## Plot both
- KiCad's: copy the project into `$S/p/`, then `flatpak run --filesystem=$S --command=kicad-cli
  org.kicad.kicad sch export svg -o $S/k/ $S/p/<root>.kicad_sch`: every page, as
  `<root>[-<sheet>…].svg`. Each text is a `<g class="stroked-text"><desc>TEXT</desc>` of strokes.
- Cuprita's texts: a scratch cargo project with `cuprita-core = { path = "<repo>/crates/core" }` and
  `.claude/skills/sch-check/texts.rs` as its `src/main.rs`; `texts $S/p/<root>.kicad_sch $S/e/`
  writes each page's texts under the same names.
- Cuprita's picture of a sheet: `cargo run --release -p cuprita-render --example png --
  <sheet>.kicad_sch $S/e.png 8192` (the page and 2.5% of its width round it).

## Compare
`python3 .claude/skills/sch-check/schcmp.py $S/k $S/e` pairs each text with KiCad's nearest of
the same string within 1 mm and prints, per ink, how many centres sit within 0.05 mm (centres
compare: Hershey glyphs reach a stroke unit further each side than KiCad's), the worst, and the
texts only one side draws (KiCad's frame and title block aside; multi-line texts aren't
compared, KiCad plots each line apart).

Overlay a region: `schcmp.py overlay $S/k/<page>.svg $S/e.png x0,y0,x1,y1 $S/o.png` (sheet mm)
and read the PNG: red KiCad only, green Cuprita only, yellow both.

## Expected differences
- Glyph shapes (`1`, `R`, `µ`…) and advances (a long line runs up to ~2% short): `font.rs`.
- Subscripts and superscripts at full size (`V_{CC}`), so what follows them on the line moves.
- Not drawn yet: the frame and title block, inter-sheet references (`Pg.2`), tables, images,
  DNP crosses, net class flags (`crates/core/schematic.md`).
Anything else is a bug: find the item in the sheet file near the place reported.

## Learn a KiCad rule
Write a sheet with the case in question (a python script printing `(kicad_sch (version
20250114) (generator "eeschema") (uuid "…") (paper "A3") (lib_symbols …) … (sheet_instances
(path "/" (page "1"))))`, library symbols copied in from `/var/lib/flatpak/runtime/
org.kicad.kicad.Library/<arch>/stable/active/files/symbols/*.kicad_sym` renamed `Lib:Name`),
plot it, and read KiCad's strokes, `<path>`s and `<circle>`s. Use strings both fonts draw alike
(`HEH`), every angle and mirror, two text sizes, and a `.kicad_pro` beside it to change
`text_offset_ratio` and `label_size_ratio`. The label shapes, pin text gaps, text pens and field
placement in `schematic.md` were found so; write what you find there, and a test with KiCad's
numbers.
