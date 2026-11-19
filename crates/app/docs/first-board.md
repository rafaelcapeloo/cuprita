# Your first board

From an empty project to the files a fab takes. For a guided start instead, Ctrl+K › First
board walks you through one that lights an LED from a coin cell, checking each step as you go.

## 1. Start a project
On Home, New project (Ctrl+N): give it a name and choose Blank (2 to 16 copper layers, and the
board's size) or one of KiCad's templates. Cuprita writes the KiCad files in their folder and
opens the schematic, with First steps at its top right: each step ticks off as you get there.

## 2. Draw the schematic
- A adds a part: type what it is or its number (`R`, `LED`, `LM358DR`, `regulator 3.3`), Enter,
  then click to put it down; R turns it. References number themselves (R1, R2…).
- W draws a wire: click a pin, click where it turns, end on a pin (or double click, or Enter).
- L names a net: type `SDA`, then click a pin or wire. Power symbols come from A: `GND`, `+3V3`.
- D checks the schematic (ERC): each problem in plain words, with its fix when you hover it. Q
  flags a pin you leave unconnected on purpose.

## 3. Choose footprints
Click a part, then in Inspect, Footprint › Choose…: the footprints that fit its symbol, narrowed
by what you type (`0603`). Parts found by number at Digi-Key come with theirs.

## 4. Bring it to the board
F8, from the schematic or the board, puts the parts below the board with their nets as thin
lines: the connections still to route. The board has its outline already (the size you gave, or
the template's); O draws another, a rectangle dragged or corners clicked.

## 5. Place the parts
Drag them inside the outline (a dashed guide shows when one lines up with another, and how far
apart they are); R turns, F puts one on the back, the arrow keys nudge. Ctrl+E jumps between a
part's footprint and its symbol.

## 6. Route
X, then click a pad and move: the track keeps its clearance, and other nets' tracks make way (P
walks round them instead). Each click fixes it up to the pointer; it ends on a pad of its net.
V drops a via to the other side, W makes it wider, / bends it the other way, Esc drops the part
not fixed. The status bar counts what is left: `3 to route`.

## 7. Pour copper
Z, click a pad of GND, then click the corners of the area: Enter fills it, keeping clear of the
other nets. It fills again by itself as you move things near it.

## 8. Check and make
D lists what is left to route and every design rule problem, with its fix. When the header says
Ready to make, Make (Ctrl+4) writes the fab files (Ctrl+P): Gerbers and drills in one zip, the BOM
and the parts' positions, which JLCPCB, PCBWay and OSH Park take as they are. Its order buttons
open their pages, and its numbers fill in their quote forms.
