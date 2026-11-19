# crates/core: PDF plots (`pdf.rs`)

`Project::pdf(title)` — the schematic as a PDF 1.4 (the app's Ctrl+Shift+P and agents' `pdf`
write it as `<root>.pdf` beside the project), a page per place a sheet is used, in KiCad's
order: `Project::number(i)` is a page's number as KiCad keeps it (its sheet box's `instances`
from KiCad 7, the root's `sheet_instances` in KiCad 6, else its place in the project).

## What it draws
- Each sheet on its paper, its `shapes` and `texts` ink by ink, back to front, in KiCad's
  colours (`Ink::color`, the window's too): lines and fills as vectors, arcs and circles as
  Béziers (a quarter turn each), text as the strokes the window draws (not searchable).
- The drawing sheet the window draws too (`schematic/frame.rs`: `Project::frame(i)`, on
  `Ink::Frame`), Cuprita's own, not KiCad's: 10 mm margins, a 2 mm band of zones about 50 mm long,
  numbered across and lettered down; a title block 110 mm wide in the corner: the sheet's
  comments, company and title (its `title_block`), its path, revision, page (`3/11`), file and
  date. Text too wide for its cell is narrowed. `Project::free_spot(i, size)`: room on the page
  for a box (the app's New sheet), clear of what it draws by 2.54 mm.
- Text variables (`Project::var`): a title block's fields and a sheet's texts show what their
  `${NAME}`s stand for: the title block's `TITLE`, `ISSUE_DATE`, `REVISION`, `COMPANY`, the
  sheet's `#`, `##`, `SHEETNAME`, `SHEETPATH`, `FILENAME`, `PROJECTNAME`, else the project's
  (`.kicad_pro` `text_variables`, which board texts take too: `Board::apply_pro`); others stay as
  written, as in KiCad. Checked against `kicad-cli sch export svg` on hackrf-one's title blocks.
- Numbers to 1/100 pt (3.5 µm); streams deflated at zlib level 3: within 3% of the default's
  size in half its time.

## Checked
- `pdf.rs` test: each object where the cross-reference table says; A3 and A4 pages; colours;
  a zone tick and a title block cell where they belong (it caught two i32 overflows on A3);
  every point on its page; page numbers as KiCad 7 and KiCad 6 keep them.
- glasgow-revD1 against `kicad-cli sch export pdf`: poppler reads its 11 A4 pages, each like
  KiCad's page of the same number; drawn alike but for what `schematic.md` lists as missing.

## Speed
glasgow-revD1's 11 sheets in 0.11 s, 1.3 MB; kria-k26's 19 A3 sheets in 0.32 s at level 6.

## Missing
Searchable text, links between sheets, KiCad's own drawing sheet (`.kicad_wks`), black and
white, the board (its layers as chosen).
