# Cuprita: rules for agents

Cuprita is a fast, easy PCB/schematic CAD in Rust that opens and saves KiCad files.
Free and open source (GPL-3.0-or-later), public on GitHub: never push, tag, release or post without the owner's OK.

## Find information (read only what the task needs, then stop)
1. This file, then `ROADMAP.md` for goals, decisions and speed budgets.
2. The `STATE.md` of each area you touch. Don't read whole crates to orient.
3. Code: start at the entry points STATE.md names; read the functions you need, not whole files.
4. `.claude/skills/` for recurring procedures (loaded on demand).

| Area | What | State |
|---|---|---|
| `crates/core` | KiCad parsing, board model, DRC, analysis; no GUI deps | `crates/core/STATE.md` |
| `crates/render` | wgpu board renderer | `crates/render/STATE.md` |
| `crates/app` | egui app (`cuprita`) for desktop and browser, MCP server and JSON CLI | `crates/app/STATE.md` |
| `corpus/` | real KiCad boards for tests and timing | skill `corpus` |
| `docs/` (in `crates/app`, which builds it in) | the user guide (`pt-BR/`: translated), which Help shows: change it with what users do | `crates/app/over.md` |

## Quality bar (every change)
- Complete, robust, reliable; then the least code that does it, easy to read.
- Fast and lean in time and memory; never regress the ROADMAP budgets (measure, don't guess).
- Interfaces (UI, CLI, MCP, Rust APIs) are modern, elegant and obvious: good defaults, few
  steps, shortcuts, clear names; every error says what, where, and how to fix.
- Lower the user's effort: nothing to set up or learn first; the common task is one action.
- Keep the repo coherent: one way to do each thing, docs match code, no dead code.
- Leave it better: fix bugs, debt and unclear code you meet, in their own commit. Before a
  roadmap item, pay the `## Debt` entries of the STATE.md files you'll touch.

## Work rules
- Small steps: one task = one focused commit (aim < 400 changed lines); split bigger work.
  Once a remote exists: one task = one PR.
- No options, abstractions or "for later" code that nothing uses yet.
- New logic gets a test; a bug fix starts with a failing test.
- Run `./check.sh` before every commit; it must pass. Check a split's earlier commits alone in a
  `git worktree` with its own `CARGO_TARGET_DIR` (a shared one leaves stale builds behind).
- Own implementation: KiCad's file format (https://dev-docs.kicad.org/en/file-formats/) and public
  docs, never its source. KiCad is a reference, not a ceiling: do better where standards (IPC, IEC,
  Gerber) allow; note where and why we differ.
- Never lose user data: KiCad files must round-trip; never overwrite a user file without a backup.
- Time parsing with the `load` example, rendering with `png`; put new numbers in ROADMAP budgets.
- New dependency only if it removes real code; say why in the commit message.
- One topic per source file: extend the module that owns it; split a file that mixes topics
  or passes ~1000 lines. A known limit gets `// shortcut: <limit>, <when to upgrade>`.
- Temp files go outside the repo; delete anything you made that isn't part of the change.
- You can't see windows: check render/UI changes with skill `look`.

## Docs rules (keep every agent's context small)
- Every `.md` ≤ 60 lines (check.sh enforces). Terse facts, no prose, no duplicates.
- Near the limit, split by topic into a sibling file (e.g. `crates/core/drc.md`) linked from
  its parent; never drop facts to fit.
- Update a STATE.md when your change makes it wrong or adds an entry point, gotcha or debt.
- New area → add its STATE.md and a row in the table above.
- A procedure you'll repeat → `.claude/skills/<name>/SKILL.md` (≤ 60 lines). Not for one-offs.
- Tick ROADMAP.md items when done; update it when a decision or budget changes.

## Commits
`area: imperative summary` (≤ 72 chars); the body says why. Example: `core: parse zone fills`.
Commit only when the owner says so. Until then, report what changed, how you checked it,
and what's next; pick up work at the first unticked ROADMAP item.
