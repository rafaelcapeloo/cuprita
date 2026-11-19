# crates/core: simulation in ngspice (`ngspice.rs`)

`ngspice::simulate(&Deck, stop)` runs a project's netlist (`Project::spice`, `spice.md`) in
ngspice's shared library, in this process (else inside KiCad's Flatpak), and gives `Sim`: each
analysis' `Plot` (`tran1`, `ac1`, `dc1`, `op1`, in the order ngspice ran them) of `Vector`s
(lower-case name, unit s, Hz, V or A, values; AC's complex), and `log`, what ngspice said on
stderr (warnings, why a run stopped short). Desktop only (no ngspice in a browser).

## Finding it, with nothing to set up
- The system's `libngspice.so.0` (`.0.dylib`, `ngspice.dll`), then KiCad's, as its installs bring
  it: Flatpak (system, user: `lib64/`, `lib/`), Snap, the macOS app, `Program Files\KiCad\<ver>\
  bin` (newest first). A KiCad's own that doesn't load: `found` says to install the system's,
  the loader's words on a second line.
- KiCad's Flatpak's loads only on a host with a C library as new as its runtime's (10.0.8's is
  Fedora 45's: no on Fedora 44). The run then goes inside it (`inside`): `flatpak run
  --command=<this program> org.kicad.kicad --ngspice`, shown the program's folder (the system's
  `/usr` as `/run/host/usr`) and the project's; there `helper` reads the netlist from stdin, runs
  it in process and writes `progress` lines, then the results after a mark: a line a plot and a
  vector, its values as memory holds them (to the bit; class-d's 13.5 M values add no time one
  can measure). `stop` closes its stdin, which halts it; what it simulated comes back. Only the
  program does this, not a test's: `loads` says whether a run stays in process.
- An init file (`spinit`) not where it was built to be (KiCad's flatpak, KiCad on Windows) leaves
  the code models out: they're loaded (`codemodel`) from `ngspice/` beside the library or
  `../lib/ngspice` (XSPICE devices, POLY sources in vendor models, `d_state`).
- Not found: "install KiCad, which brings it, or ngspice's shared library".

## A run
- One ngspice per process: runs take turns (a lock). The last circuit and results go first
  (`remcirc`, `destroy all` once copied); `set ngbehavior=<Deck::behavior>` before the circuit;
  `ngCM_Input_Path` is the project's folder, where code models open their files.
- `bg_run`, then waiting on its thread's start and end callbacks: halted (`stop`) before its
  thread starts, a run would go on unseen, then crash ngspice when its results go. `stop` sends
  `bg_halt`; what it simulated by then comes back. Every analysis of the deck runs in one go.
- Vectors inside subcircuits (`xu1.5`, ngspice's names with a `.`) aren't kept: they aren't the
  schematic's nets (q17: 59 of 277).
- A netlist ngspice can't read, or a run without an analysis, is an error with ngspice's words;
  after a fatal one ngspice asks to quit and is reset and started again before the next run.
- `ngspice::progress()`: how far the run going is, in percent, as ngspice tells (`tran: 45.3%`,
  every ~150 ms of a long run).
- A BSIM3 model whose parameters ngspice warns about makes it write `b3v32check.log` in the
  working folder (q17), as under KiCad.

## Checked
- `ngspice.rs` tests: a divider's operating point and an RC's time constant, units, errors and the
  run after them, a stopped run (then another); results down a pipe and back, bit for bit.
- `crates/app/tests/simulate.rs`: the program simulates the rectifier (here, through the Flatpak;
  by hand there: all eight demos as below, class-d in 12 s, a long run stopped by its stdin).
- `tests/corpus.rs`: the corpus demos that run in under a second, as their circuits work (a
  rectifier's peak, a Sallen-Key filter's gain, sources' levels, an XSPICE counter counting).

## Speed (release)
Demos 1–170 ms; class-d 13 s (465k points), q17 25 s (197k points, 533 MB peak, mostly ngspice's
own copy of its 277 vectors while it runs), as fast as KiCad's (the same ngspice).

## Missing
OSDI (Verilog-A) models aren't loaded; with an older library lacking `ngCM_Input_Path`, code
models open their files from Cuprita's working folder.
