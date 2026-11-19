# Ask and agents

## Ask
? opens Ask beside the board or the sheet: ask about the project in plain words ("What is U3?",
"Any ERC errors?", "How wide is the 5 V track?"). A model on this computer answers, in Ollama:
install it from ollama.com, then `ollama pull gemma4`. It looks things up as it answers (the
parts, nets, checks, IR drop, changes, simulation, the project's documents and its parts'
datasheets), each lookup shown as a chip, and the parts and nets it names are links to them.

It only looks: nothing changes. Enter asks, Shift+Enter starts a new line, Esc stops an answer,
New chat starts over, and the list beside Ask picks another model (Ollama's cloud models send the
conversation to ollama.com; the others stay on this computer).

## Agents
AI agents drive Cuprita through the same commands the window uses:
- `cuprita --mcp` serves them over MCP (stdio), for Claude Code, Claude Desktop and other MCP
  clients;
- `cuprita --json <board> '{"cmd":"drc"}' '{"cmd":"route",…}'` runs them from a script, one JSON
  answer per command; `cuprita --help` lists them.

They open or start projects, look things up (parts, nets, the BOM, checks, IR drop, changes), edit
(place, route with shoving, zones, fill, tidy texts, delete, undo and redo), shape the outline,
set the design rules, search KiCad's libraries, bring in a part's files downloaded from SnapMagic,
Ultra Librarian or SamacSys (its 3D model too), save, write the fab files and render pictures; on
the schematic they add symbols (by part number too), wires, buses, labels and sheets (each part's
pins listed, a net joined by naming its pins), check, simulate and update the board. Each command
and its arguments: `crates/app/agent-tools.md`. Keep the project open in Cuprita to watch: what an
agent saves shows there within a second, the view kept (with edits of yours not saved, the status
says so instead).
