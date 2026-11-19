# crates/core: chat with a local model (`ollama.rs`)

`Ollama::chat(model, messages, tools, stop, heard)` sends a conversation (`message`,
`tool_message`, `Reply::message`) to a model in Ollama and gives its `Reply`: text, and the tools
it calls with their arguments (`Val`); `heard` gets its text and thinking as they come.
`Ollama::models()`: the models pulled into it that can call tools, the last pulled first, those
in Ollama's cloud (`remote_host`, they send the conversation to ollama.com) last.
Desktop only. Ollama runs the model (llama.cpp's engine) on the GPU or CPU; Cuprita runs none.

## Finding it, with nothing to set up
- Ollama's server on this computer, `127.0.0.1:11434`; `OLLAMA_HOST` (`host`, `host:port`,
  `http://host:port`; `0.0.0.0` means this computer) as Ollama's own command line takes it.
- Not there: "install it from ollama.com (it then runs by itself), or start it with `ollama serve`".
- A model that can call tools is one whose `capabilities` (in `/api/tags`, Ollama 0.40) hold
  `tools`; an Ollama that doesn't list them has each of its models tried.

## The protocol (Ollama's REST API, `/api/tags`, `/api/chat`)
- HTTP/1.0 over a `TcpStream`, no TLS: the reply's body runs to the end of the connection,
  never chunked; a chat streams one JSON object a line (NDJSON), tool calls whole, with
  `arguments` an object.
- `num_ctx` 16384: Ollama's default context is 4096 tokens with an 8 GB GPU (its log: "vram-based
  default context"), and a longer conversation loses its start silently.
- Thinking models (Gemma 4) think first: `heard(.., true)`; it isn't sent back.
- Reads wait 100 ms at a time, so `stop` is seen while a model loads; stopped, a chat gives what
  the model said by then. Errors: Ollama's own words (`{"error": …}`, also mid-stream), after
  `Ollama: `.

## Checked
`ollama.rs` tests, on a fake server: a streamed reply with thinking and a tool call, the request
sent, models filtered by capabilities, errors (HTTP and mid-stream, no server), a stop while a
model loads. By hand on Ollama 0.40.7 with `gemma4:e2b` (RTX 5060): a tool call, then an answer
from its result.

## Speed
Gemma 4 E2B on an RTX 5060: a reply 0.3–0.4 s warm (~330 tokens/s); the first after a load 3–4
s (56 s the very first after Ollama started); Ollama unloads a model 5 minutes after its last use.
