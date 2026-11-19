# Make

Make (Ctrl+4, or the pill in the header: `2 to fix`, `Ready to make`) gathers what an order
needs on one page.

## Before you order
A ring counts the checks that pass, and one sentence says what is left ("Route the last 3
connections and fix 2 design rule problems"). Each check says what it is about, and a failing one
has its fix a click away:
- the schematic's electrical rules;
- the board matching the schematic (else Update brings the changes, as F8);
- parts on the board, a closed outline (with its size);
- every connection routed, no design rule problem;
- the zones filled;
- part numbers (MPN or LCSC) for assembly: Fill in shows the first part lacking one.

## The fab files
Export (Ctrl+P) fills the zones, then writes next to the board:
- `<board>-gerbers.zip`: Gerber X2 copper, mask, paste, silkscreen and outline, and Excellon
  drills, named as KiCad names them;
- `<board>-bom.csv` and `<board>-positions.csv`: the parts list and where each part sits, in
  JLCPCB's columns, with the MPN and LCSC fields.

A file already there is kept as `<file>-bak`. JLCPCB, PCBWay and OSH Park take them as they are;
Show opens their folder.

Ctrl+Shift+P (Schematic PDF) writes every sheet into `<project>.pdf` beside it, as KiCad numbers
them, each with a frame and title block, and opens it: for whoever assembles or reviews the
board.

## Order
Make lists the numbers a fab's quote form asks for (size, copper layers, thickness, smallest
track and gap, smallest hole, holes, parts on each side) and opens JLCPCB's, PCBWay's and OSH
Park's order pages, where you upload the zip. It also shows the board in 3D as the fab makes it.

## Assemble by hand
Shift+B opens the BOM checklist: one line per value and footprint. A line marks its parts on the
board; Space ticks it and moves to the next. The ticks stay for the next time the board opens.
