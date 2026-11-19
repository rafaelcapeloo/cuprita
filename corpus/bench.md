# Cuprita beside KiCad: what was timed, and how

`corpus/bench.sh` runs it (after `corpus/fetch.sh`; it needs KiCad's Flatpak, `org.kicad.kicad`)
and prints this table. Parent: `README.md` (its Speed table is the kria-k26 half of this one).

Measured 2026-11-18: AMD Ryzen 5 7600X (6 cores, 12 threads), 30 GB, Fedora 44; KiCad 10.0.8;
Cuprita 0.0.1, release build. Milliseconds, median of 5 runs.

| Job | Project | KiCad 10.0.8 | Cuprita 0.0.1 | |
|---|---|--:|--:|--:|
| Open the board | glasgow-revD1 | 774.0 | 19.2 | 40× |
| Refill every zone | glasgow-revD1 | 3428.9 | 135.0 | 25× |
| DRC | glasgow-revD1 | 5479 | 213 | 26× |
| ERC | glasgow-revD1 | 1615 | 15 | 108× |
| Schematic as a PDF | glasgow-revD1 | 663 | 124 | 5.3× |
| Fab files | glasgow-revD1 | 2140 | 266 | 8.0× |
| Open the board | kria-k26 | 1055.4 | 85.7 | 12× |
| Refill every zone | kria-k26 | 2149.1 | 278.1 | 7.7× |
| DRC | kria-k26 | 5677 | 217 | 26× |
| ERC | kria-k26 | 2117 | 18 | 118× |
| Schematic as a PDF | kria-k26 | 1136 | 155 | 7.3× |
| Fab files | kria-k26 | 3436 | 538 | 6.4× |

## The projects (others' open hardware, fetched, not copied here)
- glasgow-revD1: Glasgow Interface Explorer rev D1 (GlasgowEmbedded, 0BSD): a 10.6 MB board of
  KiCad 10, 8 zones, 11 sheets.
- kria-k26: Antmicro's Kria K26 devboard (Apache-2.0): a 54 MB board of KiCad 7, 298 zones, 19 sheets.

## How each is timed
- DRC, ERC, PDF, fab files: the command from its start to its exit (`kicad-cli …`,
  `cuprita --json …`), reading the files and writing the result included. KiCad's runs in its
  Flatpak and is timed inside the sandbox, whose own start (0.2 s) isn't counted.
- Open and refill: inside each program, without its start: KiCad's Python (`pcbnew.LoadBoard`,
  `ZONE_FILLER.Fill`), Cuprita's examples (`load`: the file read into its board; `fill`).
- Fab files: KiCad writes Gerbers, then drill files (two commands, added); Cuprita's one command
  also refills the zones and writes the BOM and the positions.

## What the numbers don't say
- KiCad is the larger program by far, and the reference: Cuprita reads and writes its files.
- KiCad checks kinds of rules Cuprita doesn't have yet: footprints and symbols against their
  libraries, text sizes, starved thermals, ends off the grid and more. For DRC and ERC the
  script sets those to ignore in a copy of each project, so both check the same kinds; as it
  ships, KiCad's DRC took 5.7 s on glasgow-revD1 (5.5 here) and its ERC 2.7 s (1.6 here).
- The kinds both have don't agree to the last finding yet. DRC on glasgow-revD1: 77 edge
  crossings of silkscreen, 5 over pads and 1 hole clearance in both; silkscreen overlaps 2
  (KiCad) and 3. ERC: the same counts on kria-k26 (4, 185, 24); on glasgow-revD1 Cuprita finds
  20 of KiCad's 23 undriven power pins and 1 of its 4 dangling wires (to look into: `erc.md`).
- KiCad's loading also prepares what its editor needs (connectivity, its view); Cuprita builds
  its scene and connections after, in another 0.07 s on kria-k26 (`ROADMAP.md` budgets).
- One computer, one KiCad build (the Flatpak's): other machines give other ratios.
