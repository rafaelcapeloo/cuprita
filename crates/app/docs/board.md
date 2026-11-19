# Board

Drag pans, the wheel zooms, Home fits the board, F flips the view to the back. The tools wait at
the canvas's bottom, the one on says what to do at its top, and Esc stops it.

## Pick, move, turn
- Click a part, track, via or zone; Shift+click adds or takes away; Shift+drag draws a box (left
  to right takes what it holds, right to left what it touches).
- Drag moves what is picked, onto the grid (N steps it; Ctrl places off it); R turns it 90°, F
  puts parts on the back, the arrows nudge, Delete deletes. Each is one step for Ctrl+Z.
- A track picked alone drags with its neighbours following at 45°; U picks its whole run; Inspect
  sets the width of the tracks picked (`0.3mm`, `12mil`).
- A adds a footprint from KiCad's libraries: mounting holes, test points, fiducials, logos.

## Route
X, then click a pad, via or track: the track follows the pointer, keeping its clearance (the
soft band around it). Other nets' tracks and vias make way for it, and what they push moves too;
P walks round them instead, for the next tracks as well.
- A click fixes it up to the pointer; it ends on a pad, via or track of its net.
- V drops a via and goes on on the other side; PgUp, PgDn, + and − go on on another layer; Shift+V keeps vias to the layers they join (blind or buried: ask your fab what they cost).
- W and Shift+W step its width through the project's widths; / bends it the other way.
- A red ring shows where it can't reach, with the room it needs. Esc drops the part not fixed.
- 6 routes a differential pair (nets named as D+ and D-, or CLK_P and CLK_N): click one of its pads, lead its two tracks by clicks, and click the pair's other pads. They run side by side, as wide and as far apart as their net class says.
- A net too short: pick one of its tracks and type the length the net should have in Inspect (Net length), or click Match there for the shorter side of a pair: the track takes a meander. Inspect shows a pair's skew.

## Zones
Z, click a pad of the zone's net, then its corners (45° unless Shift is held); Enter closes and
fills it. Pick a zone to drag its corners, add corners at the middle of its edges, set its
priority, clearance, thermal reliefs and more in Inspect, or cut a hole (Shift+C). Zones fill
again by themselves near what you edit; B fills them all again.

## Outline
O, then drag a rectangle or click its corners: the board's new outline, holes in it kept.

## Texts and lines
Shift+T, type the words, Enter: they land in the middle of the view, picked, to drag where
they go. L draws lines: click their corners or drag a rectangle. Both go on the silkscreen of
the side shown, or the layer chosen beside the tool's name. Pick one to change its words,
layer, size or width in Inspect; R turns it, Delete removes it.

A click on a part's reference or value picks that text alone: drag moves it, R turns it, Inspect sets its size and pen, Delete hides it; pick the part and click it under Hidden texts in Inspect to show it again. T tidies the texts of the parts picked, else of all: each reference reads upright, 1 mm high (the smallest fabs print), just outside its part, off pads, other silkscreen and the edge (hidden where none fits); each value goes inside its part on the Fab layer, shown from "N more, hidden" in Layers. One step for Ctrl+Z.

## Layers
The active layer (PgUp, PgDn, + and −, or a click in Layers) is where tracks go, drawn brightest;
the eye beside each layer shows or hides it. Inner layers name their plane: `In1.Cu · GND`.

## Checks
D lists the connections still to route (thin lines on the board, counted in the status bar)
and every design rule problem, from the project's rules: hover one for its fix, click to see it.
Edits re-check as you make them.

## Measure and look
M measures between two points (on copper or the grid). Alt+3 shows the board in 3D with its
parts' models: drag turns it, the wheel zooms, F turns it over, Esc comes back. The view glides
to what you find; Settings (Ctrl+,) has Reduced motion.

## The schematic beside it
Ctrl+E jumps between a part's footprint and its symbol, and between a net on the board and on
the sheets. F8 brings the schematic's changes to the board.
