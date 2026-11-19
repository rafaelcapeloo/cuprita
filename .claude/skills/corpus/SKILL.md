---
name: corpus
description: Add a real KiCad board to Cuprita's test corpus, fetch the corpus, or time parsing on it. Use when adding test boards, reproducing a parse failure on a real file, or measuring parser speed.
---
# Corpus

Boards live in `corpus/boards/` (git-ignored) and are listed in `corpus/manifest.tsv`:
`name<TAB>sha256<TAB>license<TAB>url`, with the URL pinned to a commit, never a branch. A
board's project files (every `.kicad_sch` sheet, the `.kicad_pro`) are lines named
`<name>/<file>`, fetched into `corpus/boards/<name>/` (the board stays `boards/<name>.kicad_pcb`).
KiCad's simulation demos are projects without a board, `sim/<demo>/<file>` (sheets, `.kicad_pro`,
workbook, model libraries), from KiCad 10.0.6's commit of its GitHub mirror (GitLab's raw files
answer curl with a browser check).

## Fetch and verify
`corpus/fetch.sh` downloads missing boards and checks every sha256.

## Add a board
1. Pick an openly licensed `.kicad_pcb`; prefer KiCad versions or sizes the corpus lacks.
2. Pin the URL: `https://raw.githubusercontent.com/<org>/<repo>/<commit>/<path>`
   (commit: `gh api repos/<org>/<repo>/commits/HEAD --jq .sha`).
3. Download to `corpus/boards/<name>.kicad_pcb`, append its manifest line (`sha256sum`,
   the repo's SPDX license), the same for its schematic's sheets and `.kicad_pro` (KiCad 6+),
   run `corpus/fetch.sh` to verify.
4. `cargo test -p cuprita-core --release --test corpus` must pass. A failure is a parser bug
   or a raw-text count false positive: fix the cause, don't drop the board.

The corpus tests read the boards the manifest lists, not other files there (the app's
`_autosave-` of a board edited in it). They pass at once, testing nothing, when they find none
(a fresh clone, a worktree: copy `corpus/boards` into it): a run under a second didn't check
the corpus. A worktree sharing the target dir also leaves binaries cargo reuses for the main
tree: `touch` its sources after.

## Real project rules
Corpus boards have no `.kicad_pro`, so DRC and routing use KiCad's default rules. For the
project's, download the manifest URL with `.kicad_pro` for `.kicad_pcb` into a temp dir and
symlink the board there under the same stem: the app and `cuprita --json` read the sibling.
Compare DRC with KiCad's there (flatpak `kicad-cli`, skill `look`): `pcb drc --refill-zones
--all-track-errors --format json --severity-all -o <dir>/out.json <board>`. Without the refill,
stale corpus fills flood `clearance`; a report keeps at most 199 of a kind (`clearance` 499).
Leave a `.kicad_dru` out: Cuprita doesn't read custom rules.

## Time it
`cargo run --release -p cuprita-core --example load -- corpus/boards/*.kicad_pcb`; a schematic
from its root: `corpus/boards/<name>/<root>.kicad_sch`; routing: `--example route -- <board> [<board>.kicad_pro]`.
Update the ROADMAP budget table when numbers change notably.

KiCad's load time for comparison (flatpak `org.kicad.kicad`):
```
flatpak run --command=python3 org.kicad.kicad -c 'import sys,timeit,pcbnew
for p in sys.argv[1:]: print(p, min(timeit.repeat(lambda: pcbnew.LoadBoard(p), number=1, repeat=3)))' \
  "$PWD"/corpus/boards/*.kicad_pcb 2>/dev/null
```
