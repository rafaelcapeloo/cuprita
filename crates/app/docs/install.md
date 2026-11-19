# Install Cuprita

The project's releases have an installer for each system: Windows (`…-windows-x64-setup.exe`),
macOS (`.dmg`: drag Cuprita to Applications; its first start is right-click › Open, as it isn't
notarized), Debian and Ubuntu (`.deb`: `sudo apt install ./cuprita_*.deb`), Fedora (`.rpm`) and
any Linux with Flatpak (`flatpak install --user cuprita.flatpak`).

## From source
Cuprita builds from source with Rust (1.95 or newer) and a C linker. Install Rust once with
`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`, then get the code:
`git clone <the repository> && cd cuprita` (later, `git pull` updates it).

## Linux
- For you alone: `./install.sh` builds Cuprita and puts it in the app grid, with `cuprita` in
  `~/.local/bin`. Run it again after `git pull` to update; `./install.sh remove` takes it away.
- For every user, as a package: `./package.sh` builds one for the system it runs on, in
  `target/package`:
  - Fedora, RHEL, openSUSE: `sudo dnf install target/package/cuprita-*.rpm`
  - Debian, Ubuntu: `sudo apt install ./target/package/cuprita_*.deb`

  Build each package on the system it installs on: the app links that system's C library.
- Or run it from the code: `cargo run --release -p cuprita -- [project or board]`.

The first build takes a few minutes; later ones a few seconds.

## KiCad, ngspice and Ollama (all optional)
Cuprita finds these with nothing to set up, and Home lists what it found:
- KiCad (10 best): its symbol and footprint libraries (Add part), 3D models and ngspice.
  Without it Cuprita opens and edits projects, but has no libraries to add parts from.
- ngspice, for simulation: KiCad brings it. Where a Flatpak KiCad's is built for a newer system
  than yours, Home says so: install your system's own (`libngspice`).
- Ollama, for Ask: install it from ollama.com, then `ollama pull gemma4`.

## In a browser
`./web.sh serve`, then open http://127.0.0.1:8080 and drop a `.kicad_pcb` (with its
`.kicad_pro`) on the page: the board, its checks and analysis; no schematic, libraries or files
written beside it (Save and the fab files download).

To show a project to others, Share (the QR button at the window's top right) puts its schematic,
board and 3D view on a web page, to look at: scan the code with a phone on your network, or open
the address on another computer. Nothing there edits the project, and the page follows the files
as you save them. Without a window, `cuprita --serve project.kicad_pro` does the same at
http://127.0.0.1:8080 (`--port` picks another port, `--bind 0.0.0.0` opens it to your network).

## macOS and Windows, from source
`cargo run --release -p cuprita` builds and runs it.

## Where Cuprita keeps its things
Settings (Ctrl+,) shows the folder: the recent projects and their pictures, the units, the
language and your Digi-Key key (a file only you can read). Your projects stay where they are; Cuprita writes beside
them only `<file>-bak` (the version before each save) and `_autosave-<file>` (until you save).
