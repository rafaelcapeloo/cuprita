# crates/app: AI — agents' commands and the chat (`agent.rs`, `agent/`, `chat.rs`)

Decision 8 (`ROADMAP.md`): AI goes through the same commands as the UI, MCP for outside agents,
a local model in the app. Desktop only.

## Agents (`agent.rs`, `agent/`)
Their commands, over MCP and the JSON CLI: `agent-tools.md`.

## The chat (`chat.rs`, `?` or the Ask button, in both views)
- `App::ask` copies the board (5 ms for kria-k26) and the schematic as edited into
  `Session::of` (a board the app hasn't read, from the schematic view, read from its file there),
  and a thread (`answer`) runs the conversation in Ollama (`cuprita_core::ollama`,
  `crates/core/ollama.md`): the model, the picked one if Ollama still has it, else its first;
  each reply's tool calls run on the session, among `TOOLS`, the commands that only look
  (`functions` gives them to Ollama with MCP's descriptions), or are the repo's; the result goes
  back cut to `READ` characters; up to `ROUNDS` rounds, the last one without tools. `ask_tab`
  shows a question as a `bubble`, a run of lookups as `chips`, an answer `linked`: its words that
  are the board's parts or nets are links that `find` them; the model and "It only looks" below.
- `chat/files.rs`: `files` lists the repo (git's files, else the folder's), `read` a page of a
  file in it, or of a datasheet a part's field links to (`curl` downloads it once to the
  data folder's `datasheets/`; poppler's `pdftotext -layout` reads PDFs, a page a form
  feed; text files 100 lines a page); `find` words pick the page. Paths outside the repo and links
  no part holds are refused: a file can't steer the model into sending anything elsewhere.
- `Heard` carries the thread's news (models, thinking, text, tool calls, the conversation or an
  error) to `App::chat_poll`, which drains it each frame, shown or not; the Ask tab (`ask_tab`)
  shows it. Dropping the `Answer` (New chat, another board) stops it.
- The instructions name the project and which tool tells what. Gemma 4 E4B (`gemma4`, 6.6 GB)
  picks the tools and answers right (what U17 is, what its pin 15 joins, ERC); E2B often answers
  from a cut BOM instead of calling `find`.

## Speed (Gemma 4 E4B, RTX 5060)
A question 4–8 s warm (two rounds, thinking most of it; ~1,000 tokens of instructions and tools);
the first after Ollama loads the model ~17 s.

## Gotchas
- A conversation past 16,384 tokens (`ollama.rs`) loses its start, instructions too: New chat.
- Stop doesn't end a `simulate` running: the answer stops once it's done.

## Missing
Chat: editing through it (the agents' edits, with the app's undo), a model downloaded from the
app (`ollama pull` does it), the browser; PDFs read without poppler (Windows and macOS lack it);
datasheet links only the schematic holds (KiCad 6–7 boards' parts have none; symbols' aren't read);
sites that refuse a download (Nexperia's, 403): the user opens the link.
