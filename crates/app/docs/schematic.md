# Schematic

Ctrl+E shows the project's schematic (again, the board). Drag pans, the wheel zooms, Home fits
the sheet. Every sheet reads as KiCad draws it, on its paper with its frame and title block as
Ctrl+Shift+P prints it, and saves as KiCad writes it.

## Parts
A adds a symbol (R turns it before the click that puts it down); finding parts, bringing them
from the web and making your own: [Parts](parts.md).

## Wires and nets
- W picks the wire tool: a click starts a wire, each click after adds a stretch, bent once the
  longer way first (/ bends it the other way, Shift goes straight at any angle); it ends on a
  pin, wire or label, or with a double click or Enter. Junctions go where KiCad needs them.
- L names a net: type its name (`SDA`, `~{RESET}`), Enter, then click a pin or wire. Ctrl+L
  makes it global (joined on every sheet); H, on a sheet below the root, hierarchical (joined to
  the sheet above through a pin of its name on the sheet's box).
- B draws a bus, Z its 45° entries; a label names its members: `D[0..7]`, `{SDA SCL}`.
- Q, then a click, flags a pin left unconnected on purpose; a click on a flag takes it away.
  Click a wire or label to light its net on every sheet; Ctrl+E then lights it on the board.

## Sheets
New sheet, in the Project tab, adds a sheet below the one shown, its box where there is room:
type its name, Enter. Shift+S does it where you drag the box. The Project tab lists the sheets,
a click shows one; a double click on a box opens its sheet, Backspace (or the path at the top
left) goes back up.

## Edit
Click picks a symbol, wire, label or no-connect flag; Shift+click adds or takes away, Shift+drag
draws a box (left to right takes what it holds, right to left what it touches). Drag moves what
is picked, and wires stay joined to its pins; R turns it, Delete deletes it. Ctrl+Z undoes any
step. A symbol's reference and value go beside it as it goes in, on a side without pins; T (Tidy
texts) puts every symbol's on the sheet there again.

## Check
D lists every electrical rule problem, as KiCad checks them with the project's severities:
errors red, warnings amber. Hover one for its fix; click to see it on its sheet.

## Simulate
S runs the schematic in ngspice as KiCad's simulator does: each part's model (else its value),
the sheets' directives (`.tran 1u 10m`) and analyses. The plots show below the sheet: drag or
the wheel pans them, Ctrl+wheel zooms, hover reads a point. S again stops it.

## To the board
F8 brings the parts and nets to the board: new parts in rows below it, a sheet's parts together,
changed footprints swapped in place, values and fields carried. Save (Ctrl+S) writes the sheets
and the board.
