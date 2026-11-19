# Cuprita's installers

Each is built on the system it is for (the app links that system's C library), by hand or by
`.github/workflows/installers.yml`: pushing a tag `v*` publishes them all as a GitHub release,
"Run workflow" builds them (all, or the one chosen in `only`) as the run's artifacts. Files are named for `Cargo.toml`'s version:
raise it before tagging. Each carries the page Share and `cuprita --serve` show (`../web.sh`
builds it into `target/web`, the same on every system: `package.sh` runs it, the workflow's
`web` job hands it to the other three), where `serve.rs` `web_dir` looks beside the app.

| Installer | Made by | Built on |
|---|---|---|
| RPM (Fedora) | `../package.sh` (rpmbuild) | a Fedora container |
| .deb (Debian, Ubuntu) | `../package.sh` (dpkg-deb) | Ubuntu 22.04: installs there and on newer ones |
| Flatpak bundle | `flatpak/io.github.rafaelcapeloo.Cuprita.yml` | the Freedesktop 25.08 runtime |
| Windows setup | `windows/cuprita.iss` (Inno Setup 6) | Windows |
| macOS disk image | the workflow's `macos` job, `macos/Info.plist` | macOS: one app for Apple silicon and Intel |

## KiCad's libraries
Every installer and `install.sh` carry KiCad 10's symbols, footprints and 3D models (3.6 GB installed, some
300 MB of an installer) in `kicad/` beside `web/`, where `library::kicad_dirs` looks after an installed KiCad's
own. `libraries.sh` fetches the three release archives (SHA-256 checked; a new KiCad: its version and sums
there) into `target/libraries`, symbols packed a file per library as KiCad installs them, with their licences.

## Flatpak
- Builds ngspice (shared, for Simulate) and git (Changes, the project's files) beside Cuprita,
  as the runtime has neither (git without its Rust part: `NO_RUST`). Cargo fetches the crates
  while it builds (`--share=network`), which Flathub wouldn't take as is. The page comes built
  (`./web.sh` first: the SDK's Rust has no wasm target) and goes to `/app/share/cuprita/web`.
- The sandbox sees the home folder (projects), the system's KiCad under `/run/host`
  (`host-os`, which `library::kicad_dirs` looks in) and the KiCad Flatpak's library runtime; its
  window takes the Flatpak's id (`FLATPAK_ID`) for its launcher. No pdftotext: Ask reads PDFs
  as it does on Windows and macOS.
- Here: `flatpak run org.flatpak.Builder --user --install --force-clean
  --state-dir=target/flatpak/state target/flatpak/build packaging/flatpak/<id>.yml` (Flathub's
  `org.flatpak.Builder`, `org.freedesktop.Sdk//25.08` and its `rust-stable` extension); its
  folders go under `target/`, as the Builder's sandbox has a `/tmp` of its own. A bundle:
  `flatpak build-bundle`; installed with `flatpak install --user cuprita.flatpak`.

## Windows
- A release build is a window app, without a console; `--mcp`, `--json` and `--bench` print to
  the console they were started from unless their output is a pipe already (`main.rs` `console`).
- The setup installs for the user without an administrator (or for everyone), in the app's
  languages, with a Start menu entry and, if chosen, one on the desktop; it claims no file types.
  The page goes in `web\` beside the `.exe`, an update's replacing the last one's.

## macOS
A universal app in a disk image beside a link to Applications, signed ad hoc (Apple silicon runs
only signed code) but not notarized: its first start is right-click › Open. The page is in the
app's `Contents/Resources/web`.

## Icons
`python3 packaging/icons.py` draws `windows/cuprita.ico` and `macos/cuprita.icns` from the mark
(`crates/app/assets/cuprita.svg`) with ImageMagick, PNGs inside; below 24 px without its vias.

## Checked
Here: the RPM built (with the libraries: KiCad hidden, Parts finds them); the Flatpak built and run (Home finds the KiCad Flatpak's libraries, Simulate
runs its ngspice, Changes its git; with the page, its Cuprita module alone: `--serve` there gave
the page, the project and the wasm); the .deb with a stand-in dpkg-deb only; Windows type-checked
(`cargo clippy --target x86_64-pc-windows-gnu`). The workflow hasn't run yet, so neither has any
of its jobs: Windows and macOS, the `web` job and the page in their installers are untried.
