# crates/core: shared tools

What the other modules share. Parent: `STATE.md`.

- `polygon.rs`: i_overlay booleans and offsets that fill, body, gerber, parse, save share; `threads::par_map`.
- `tr.rs`: `tr::{tr, trf, count}` — the texts core gives people to read, in the language the app
  shows once it hands `tr::set` its catalog's lookup (else English: agents, tests); `fill`. A text
  kept in a constant is marked `t!("…")` for the catalog test, and `tr`'d where it is returned.
- `json::Val` — JSON read (`.kicad_pro`, agents' commands) and written (`json_str`, `list`).
- `zip::read` — a zip archive's files (a part's vendor download: `parts.md`), stored or deflated.
- `sexpr::{Lexer, Cursor}` — zero-copy tokens; `Cursor` walks lists: `child()` … `close()`;
  `span()` is where the last token sits. `units::{mm_to_nm, nm_to_mm}` — exact mm ↔ i32 nm.
- `board.rs`: geometry helpers and lookups by name and place (nets, parts, copper and zones at a point): `model.md`.
- `font::{strokes, width}` — text → stroke polylines laid out like KiCad (Hershey glyphs, `~{}` overbars); its advance.
