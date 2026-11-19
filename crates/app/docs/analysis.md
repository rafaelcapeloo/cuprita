# Analysis

Numbers about the board as it stands, edits not saved yet included.

## A net
Click a track, via or pad to light its net: Inspect gives its length, its narrowest width, the
current it carries for a 10 °C rise (IPC-2152), and its impedance as a microstrip or stripline
over the stackup. Find (Ctrl+F) lights a net by name.

## IR drop
I shows a heatmap of the DC voltage drop along the net picked (else the main power net) at a
test current: resistance, power lost and current density, through tracks, vias, pads and zones.
A click on a pad zooms to it; a right-click makes it the source.

## Changes since a commit
G compares the board with its last commit in git: added copper green, removed red; a click on a
change zooms to it. From the command line, `cuprita --diff [commit or file] <board>` does the
same.

## The ratsnest
The thin lines join the copper of each net still apart, by the shortest way. D lists them as
KiCad's DRC lists unconnected items (`GND: C1 pad 2 – U1 pad 2`); a click zooms to one.

## Simulation
In the schematic, S runs ngspice: see [Schematic](schematic.md).
