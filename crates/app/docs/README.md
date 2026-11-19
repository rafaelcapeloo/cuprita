# Cuprita guide

Cuprita draws schematics and lays out circuit boards in KiCad's own files: open the project you
have, or start one, and take it to the fab in one window. In the app, F1 shows these pages and
every command with its keys; Ctrl+K finds any command, part or net by a few letters.

## Pages
- [Install](install.md): Linux packages, building from source, the browser version.
- [Your first board](first-board.md): from an empty project to the files a fab takes.
- [Schematic](schematic.md): symbols, wires, labels, sheets, buses, checks, simulation.
- [Parts](parts.md): finding a part, bringing one from the web, making your own.
- [Board](board.md): placing, routing, zones, the outline, layers, checks, 3D.
- [Make](make.md): what an order still needs, the fab files, ordering, assembling by hand.
- [Analysis](analysis.md): a net's length, current and impedance, IR drop, changes since a commit.
- [Ask and agents](ai.md): questions answered on this computer; Cuprita driven by AI agents.
- [Working with KiCad](kicad.md): what Cuprita keeps, writes and leaves to KiCad.

## Ten keys to start
| Key | Does |
|---|---|
| Ctrl+K | Find or run anything |
| F1 | Help: every command and key |
| Ctrl+E | Switch between schematic and board (the part picked stays picked) |
| A | Add a part (a symbol on the sheet, a footprint on the board) |
| W | Draw a wire |
| F8 | Bring the schematic's parts and nets to the board |
| X | Route a track |
| D | Checks: what is left to route, design rule problems |
| Ctrl+P | Write the fab files |
| Ctrl+Z | Undo anything |

Keys are KiCad's where KiCad has one, so they work the same in both.

Em português: [o guia em português](pt-BR/README.md).
