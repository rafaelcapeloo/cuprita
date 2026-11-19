# crates/core: a project's own parts (`parts.rs`, `zip.rs`)

`parts::table(dir, file)` — the libraries a project's `sym-lib-table` or `fp-lib-table` lists
(name, path; `${KIPRJMOD}` its folder; KiCad's own, named by other variables, left to `library`).
`parts::{symbols, footprints}(dir)` list their parts as `library` lists KiCad's; the app shows
them first in Parts, and reads a footprint's `lib:name` there too (F8, Inspect's Choose…).
A part made from a few numbers goes in the same way: `newpart.md`.

## Import (`parts::import(dir, project, files)`)
- Takes a part's KiCad files as a vendor or a site gives them, by name: `.kicad_sym`,
  `.kicad_mod`, `.step`/`.stp`/`.wrl`, and zips of them (`zip::read`: stored or deflated, at most
  1 GB unpacked; no Zip64 or encryption). Anything else is left out and named.
- Into the project's one library, under `libraries/`: symbols into `<project>.kicad_sym` (made
  as KiCad 10 writes one), footprints into `<project>.pretty`, models into
  `<project>.3dshapes`; both tables list them under the project's name as KiCad 10 writes a
  table (`version 7`), made when missing; each file kept as `<file>-bak` before it changes. A
  table lists a library once something goes in; libraries listed before stay as they are.
- A footprint's `(model …)` naming a model that came along points at it (`${KIPRJMOD}/…`); a
  lone footprint without one gets the lone model. A symbol's `Footprint` naming a footprint that
  came along (with any library) points at it as listed; an empty one at the lone footprint.
- A footprint given twice (KiCad 5's `module` and KiCad 6's `footprint`, as some vendors' zips
  hold it) goes in once, the newer.
- A file there already with the same bytes stays as it is; another of its name goes in as
  `_2` (a footprint renamed inside too, as KiCad names footprints after their files).
- Symbols (`merge`): one the library holds already (the same text but for spaces) stays;
  another of its name goes in as `<name>_2` (3…), its units (`<name>_2_1_1`) and the file's
  symbols extending it renamed with it; each goes in after the one it extends. A symbol from an
  older file reads as KiCad read it there (`upgraded`): before 20251024 a field, pin name or
  number `~` was empty (KiCad 10 shows a `~`); before 20210606 `~` toggled an overbar (`~~` a
  tilde), now `~{…}`. A library KiCad couldn't read stops the import before anything goes in.

## Checked
`parts.rs` and `zip.rs` tests; KiCad 10 (`kicad-cli sym export svg`, `sym upgrade`): 7
libraries of versions 20201005–20251024 (KiCad's demos, 786 of its op-amps) merged into one,
its 1137 units plot as from their own files and its 579 symbols upgrade to the same text; a
changed copy of Sensor_Temperature went in as 103 `_2`s (44 extending), each plotting as the
copy's own. In the window (skill `look`, `drop`): a vendor-style zip of KiCad 10's LM358, SOIC-8
and its STEP dropped on glasgow-revD1's schematic, Enter, a click, F8, Ctrl+S: KiCad 10's ERC
finds the symbol through the project's table, and its STEP export the model through `${KIPRJMOD}`;
since one library: DS1621 (a KiCad 9 file) and DIP-8 dropped on a new project, the same steps:
KiCad's ERC only its pins' own warnings, its DRC 0 (library checks too).

## Missing
KiCad 5's `.lib` symbols (converter coming); parts by LCSC number (EasyEDA, coming); a user's
global tables (`~/.config/kicad/*/…-lib-table`) beyond KiCad's own libraries.
