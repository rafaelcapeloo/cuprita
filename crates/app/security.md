# Cuprita: a project from someone else

Parent: `STATE.md`. A board, schematic, part zip, STEP model or whole folder (its `.git` too)
may come from anyone: opening it must run no program and write no file but the project's own.
Audited 2026-11-18, before the first public announcement; keep each rule when touching its code.

## Rules, and where each is kept
- SPICE (`crates/core/src/schematic/spice.rs` `harmless`): a `.control` block of a sheet's text,
  or of a file the deck includes (to 8 deep), runs only `COMMANDS` (set, run, measure, show:
  KiCad's demos pass), no `>` and no backquote; else the simulation is refused, with the line.
  ngspice's `shell`, `source`, `write` reach programs and files, and the chat can simulate.
- git (`file.rs` `git`): every call sets `core.fsmonitor=false` and `log.showSignature=false`
  (a project's `.git/config` names programs there); a revision follows `--end-of-options`.
- Opening a link or file (`file.rs` `shell_open`): never through a command line. Windows:
  `ShellExecuteW` (`cmd /C start` ran what followed an `&` in a Datasheet field).
- Documents listed in Project (`dock.rs` `documents`): by extension (md, txt, pdf, rst, adoc),
  a README too, or one without extension: the system opens them, and would run `README.bat`.
- Files of Cuprita's own beside the user's (`file.rs` `own_file`): `-bak`, `_autosave-*`, their
  `.part` are never written through a link someone put there in their name.
- ngspice (`crates/core/src/ngspice.rs` `libraries`): by bare name only on Linux; Windows and
  macOS also search the folder Cuprita was started in, so there by path alone.
- The served page (`serve.rs`): GET only; a key in the address when bound beyond this computer;
  answers a `Host` that is an address or `localhost` only (a page elsewhere may point its name
  here); only the files the project lists.
- Readers (`crates/core`): `step.rs` (nesting ≤ 64, items ≤ 32 deep, a text cut short, a spline
  of too few points, knot counts ≤ 1024), `json.rs` (nesting ≤ 128), `zip.rs` (1 GB unpacked, a
  stored entry counted as long as it is), `schematic/project.rs` (≤ 4096 sheets).
- The chat (`chat.rs`): tools that only look; a model in Ollama's cloud only when picked;
  datasheets by `curl --globoff --proto =http,https --max-filesize 100M`.
- Digi-Key's key: in the data folder, mode 0600, to curl on stdin; never in a file of a project.
- Part zips (`crates/core/src/parts.rs`): written under `libraries/` by base name only.

- ngspice won't start in a folder (but the home) holding a `.spiceinit`, which it would run
  (`ngspice.rs` `start`). The served page takes 64 connections at once and serves no link.
- Releases: actions pinned to commits, `SHA256SUMS.txt` beside the installers; `cargo audit`
  clean on 2026-11-18 (run it before a release); licences: `packaging/NOTICE.txt` and
  `CRATES.txt` (`packaging/notices.py`, checked by `check.sh`) go with every installer.

## Left (known)
- Installers: the Windows setup is unsigned (SmartScreen warns), the macOS app signed ad hoc, not
  notarized (first start: right-click › Open): both want paid certificates.
- A model named by a Windows drive path (`${KIPRJMOD}C:/…`) is served from there (models only).
