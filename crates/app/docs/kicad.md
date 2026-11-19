# Working with KiCad

Cuprita's files are KiCad's: there is nothing to import or export, and a project goes back and
forth between the two as you like.

## What it opens
Boards from KiCad 4 to 10, schematics from KiCad 6 to 10, with their `.kicad_pro` (design rules,
net classes, track widths). A `.kicad_pro`, `.kicad_pcb` or `.kicad_sch` opens with Open (Ctrl+O)
or a drop on the window (Windows, macOS, X11: Wayland doesn't pass drops to Cuprita yet).

## What it saves
- Everything you didn't change stays byte for byte as it was, unknown fields included; what you
  changed is written as KiCad writes it, and KiCad reads it back the same.
- Before each save, the file's previous version is kept as `<file>-bak`.
- Edits are autosaved 5 s after the last one to `_autosave-<file>` (KiCad's name), until you
  save; opening a file with a newer autosave asks to restore it.

## The same keys
Cuprita keeps KiCad's keys where KiCad has one: X route, V via, W and Shift+W widths, / bend, B
fill, F flip, R turn, U run, N grid, F8 update, Ctrl+E switch, PgUp and PgDn layers, Ctrl+,
settings. F1 lists them all.

## Where it differs
- Shove is on while routing; P walks round instead (KiCad has both as router modes).
- Design rule checks and the ratsnest run again as you edit: a big board's full check takes
  milliseconds. Zones refill near each edit, in its undo step (KiCad waits for B).
- Help, checks and errors say what to do next, in plain words.

## Still KiCad's
Drawing a symbol or footprint by hand (New part… makes one from its pins and package), library
management, microvias, differential pairs across layers, and plotting to
anything but the fab files. Use KiCad for them: it opens what Cuprita saves, and Cuprita what it
saves.
