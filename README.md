# Cuprita

A fast PCB and schematic editor for KiCad projects: open the project you have (nothing to
convert) or start one, and take it from the schematic to the fab's files in one window. Files
stay KiCad's, written as KiCad writes them, the previous version kept as `<file>-bak`.

## Speed
The same jobs on the same files, a 54 MB board with its 19 sheets (Antmicro's Kria K26 devboard),
median of 5 runs on a Ryzen 5 7600X; `corpus/bench.sh` runs it, `corpus/bench.md` says how:

| | KiCad 10.0.8 | Cuprita 0.0.1 | |
|---|--:|--:|--:|
| Open the board | 1.06 s | 0.09 s | 12× |
| Refill its 298 zones | 2.15 s | 0.28 s | 7.7× |
| Design rule check | 5.68 s | 0.22 s | 26× |
| Electrical rule check | 2.12 s | 0.02 s | 118× |
| Schematic as a PDF | 1.14 s | 0.16 s | 7.3× |
| Fab files (Gerbers, drills) | 3.44 s | 0.54 s | 6.4× |

KiCad does far more than Cuprita, and checks more kinds of rules: these are the jobs both do.

## Install
The releases have installers for Windows, macOS, Debian and Ubuntu, Fedora and Flatpak, each with
KiCad 10's symbols, footprints and 3D models inside; with KiCad installed (10 best), Cuprita uses
its own, and its ngspice to simulate. With Rust: `cargo install cuprita`. From the source, on Linux:
- `./install.sh` builds it and installs `~/.local/bin/cuprita` with its launcher and icon
  (`./install.sh remove` takes them away); `./package.sh` makes an RPM or a .deb.
- Or run it from here: `cargo run --release -p cuprita -- [project or board]`; in a browser (the
  board, its checks and analysis; no schematic): `./web.sh serve`.
- A project on a web page for others to look at: Share in the window, or `cuprita --serve
  project.kicad_pro` (`--port 8080`, `--bind 0.0.0.0` for the network).

## Start
- Home: New project (blank, or one of KiCad's templates), Open (`.kicad_pro`, `.kicad_pcb`,
  `.kicad_sch`), recent projects, KiCad's examples.
- New to boards? Ctrl+K › First board walks you through one, an LED lit by a coin cell, from
  the schematic to the fab's files.
- Ctrl+K finds any command (with its key), part or net; F1 shows the guide (`docs/`) and every
  command with its keys; Ctrl+, opens Settings (units, your Digi-Key key).

## Workbenches
- Schematic (Ctrl+E): symbols (A), wires (W), labels (L), no-connect flags (Q), ERC (D),
  simulation in ngspice (S); F8 brings the parts and nets to the board.
- Board: parts (A), routing (X), zones (Z), outline (O), DRC (D), IR drop (I), changes since
  a git commit (G), the BOM checklist (Shift+B).
- 3D (Alt+3); Make (Ctrl+4): what an order still needs, the fab files (Ctrl+P), order pages.
- Ask (?): questions about the project, answered on this computer by a model in Ollama.

## Agents
`cuprita --mcp` serves Cuprita's commands to AI agents over MCP (stdio); `cuprita --json <board>
'{"cmd":"drc"}'` runs them from scripts (`crates/app/agent.md`).

## More
- The guide: `docs/README.md`; every command and key: `crates/app/features.md`; the plan:
  `ROADMAP.md`; the look: `BRAND.md`; working on Cuprita: `AGENTS.md`. License: GPL-3.0-or-later.

## Names
KiCad® is a registered trademark of LF Projects, LLC; Gerber Format® of Ucamco. Cuprita opens and
writes their files and is not affiliated with, or endorsed by, either. KiCad's libraries, which
the installers bring, are theirs, under CC-BY-SA 4.0 (`packaging/NOTICE.txt`).
