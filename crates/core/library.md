# crates/core: KiCad libraries (`library.rs`)

`library::footprints()` lists the footprints of the libraries KiCad installs (`LibPart`:
library, name, file) with nothing to set up: the folders `KICAD*_FOOTPRINT_DIR` name, then
`/usr/share`, `/usr/local/share` (also under `/run/host`, where a Flatpak sandbox sees them),
Flatpak (system, user), Snap, the macOS app and Windows'
`Program Files\KiCad\<version>` (newest first), then the copy of KiCad 10's symbols,
footprints and 3D models Cuprita comes with, beside the app (`own_dirs`; `packaging/README.md`),
so Parts and the 3D view have them where no KiCad is installed; a library found again counts once. The same
search (`kicad_dirs`) finds KiCad's 3D models (`models.md`). KiCad 10's: 15,450 footprints in
155 libraries, listed in 8.5 ms. `library::symbols()` lists the symbols of its `.kicad_sym`
libraries so, each with what it is (`about`: description, keywords), the files read in
parallel: KiCad 10's 22,860 in 223 libraries in 40 ms. `library::search(all, query)`: those
whose `lib:name` or `about` holds every word (any case), the one named first, then the one KiCad
names for a part number (`library::named`: LM358DR finds `LM358`, STM32F103C8T6 `STM32F103C8Tx`,
MCP2515-I/SO `MCP2515-xSO`; 30 of 35 common ones), then those whose name holds every word, then
their `lib:name`, then shorter names (footprints 1 ms, symbols 3–5).
`library::symbol(text, name)`: a symbol as a sheet keeps its copy; one that `extends` another
(12,318 of KiCad 10's) gets that one's units (named after it), pins and flags, its own fields
and flags first. Every KiCad 10 symbol flattens (2 ms each: its library is read whole).
`library::field(text, name)`: a symbol's field (`Footprint`, `ki_fp_filters`); `library::fits(filter,
fp)`: a footprint against a symbol's filter as KiCad matches them (`*`, `?`, any case, `lib:name` when
the filter has a `:`).

## Adding one (`Board::add_footprint(text, lib, at, angle)`)
- The board parser reads the `.kicad_mod` text into a board of this one's layers: `*.Cu` is its
  copper, and a layer it lacks comes in plain (once a layer table is read, only it makes copper;
  the renderer hides plain layers named `*.Cu`, as KiCad hides layers a board lacks, e.g. a layer
  marker's inner ones). No nets; an error changes nothing.
- Reference: the next free number of `H` (MountingHole), `TP` (TestPoint), `FID` (Fiducial),
  `LOGO` (Symbol), else `REF`; a schematic's part (`update.md`) its own, its value and the
  `(path …)` that links it to its symbol.
- Kept as text at the origin (`Src::Library`): linked `lib:name`, the reference, an id by the
  board's version (`uuid` from KiCad 8, `tstamp` 6–7, none before) and `(at 0 0)` after its
  layer; the library file's `version` and `generator` go; the ids of its pads, texts and graphics become its own (the library's would be those of every part made from it: KiCad's Repair Board calls that duplicate ids). Saving writes it placed (`save.md`).
- `Patch::added`, made before, undoes it.

## Checked
- `library.rs` and `save.rs` tests: reference, turned pads and texts, layers, errors, undo, the
  text for KiCad 5–10, the saved text and its read-back; search.
- Every KiCad 10 footprint (15,450) adds to glasgow-revD1, framework-mcu, kria-k26 and
  hackrf-operacake (KiCad 4) without an error, then saves and reads back the same (1.9 s).
- KiCad 10 (skill `kicad-check`) loads five parts added to each corpus board (KiCad 4–10),
  turned 0–270°, as its own placement of the library's: link, pads, texts, graphics, reference;
  a test point (turned 30°) and an SOIC-8 added to glasgow-revD1 and flipped, as its own Flip of them.

## Missing
Libraries only the user's global `fp-lib-table` or `sym-lib-table` lists (a project's own:
`parts.md`; `// shortcut:`); a part goes in as
its library's KiCad version wrote it, also into older boards (KiCad of that version or later
reads them). A part goes in on the front: F flips it.
