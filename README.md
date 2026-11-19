# Cuprita

A fast PCB and schematic editor for KiCad projects: open the project you have (nothing to
convert) or start one, and take it from the schematic to the fab's files in one window. Files
stay KiCad's, written as KiCad writes them, the previous version kept as `<file>-bak`.

## Install
The releases have installers for Windows, macOS, Debian and Ubuntu, Fedora and Flatpak
(`packaging/`, built by `.github/workflows/installers.yml`). From source, on Linux:
- `./install.sh` builds it (Rust's `cargo`) and installs `~/.local/bin/cuprita` with its launcher
  and icon; `./install.sh remove` takes them away. It claims no file types.
- A package, built on the system it installs on: `./package.sh` makes an RPM on Fedora, RHEL or
  openSUSE (`sudo dnf install target/package/cuprita-*.rpm`) and a .deb on Debian or Ubuntu
  (`sudo apt install ./target/package/cuprita_*.deb`), in `target/package`.
- Or run it from here: `cargo run --release -p cuprita -- [project or board]`.
- In a browser (the board, its checks and analysis; no schematic): `./web.sh serve`, open
  http://127.0.0.1:8080 and drop a `.kicad_pcb` (with its `.kicad_pro`) on the page.
- A project on a web page for others to look at, schematic, board and 3D, nothing there to
  edit: Share in the window (a QR code for a phone on your network), or without a window
  `cuprita --serve project.kicad_pro` (`--port 8080`, `--bind 0.0.0.0` for the network).
- The installers bring KiCad 10's symbols, footprints and 3D models. With KiCad installed (10
  best), Cuprita uses its own, and its ngspice to simulate: nothing to set up.

## Start
- Home: New project (blank, or one of KiCad's templates), Open (`.kicad_pro`, `.kicad_pcb`,
  `.kicad_sch`), recent projects, KiCad's examples.
- New to boards? Ctrl+K › First board walks you through one, an LED lit by a coin cell, from
  the schematic to the fab's files.
- First steps (on each new project) ticks off the way: parts, wires, footprints, F8 to the
  board, outline, placing, routing, design rules, fab files.
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
  `ROADMAP.md`; the look: `BRAND.md`.
- Working on Cuprita: `AGENTS.md`.
- License: GPL-3.0-or-later.

## Names
KiCad® is a registered trademark of LF Projects, LLC; Gerber Format® of Ucamco. Cuprita opens and
writes their files and is not affiliated with, or endorsed by, either. KiCad's libraries, which
the installers bring, are theirs, under CC-BY-SA 4.0 (`packaging/NOTICE.txt`).
