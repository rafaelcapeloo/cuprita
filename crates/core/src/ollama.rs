//! Chat with a local model in Ollama (ollama.com), found with nothing to set up: its server on
//! this computer, or where `OLLAMA_HOST` says as for Ollama's own command line, and the models
//! pulled into it (`crates/core/ollama.md`).

use crate::json::{Val, json_str, list};
use crate::tr::{tr, trf};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The context a chat gets, in tokens: Ollama's default (4096 with an 8 GB GPU) would drop the
/// start of the conversation once a few tools' results fill it.
const CONTEXT: u32 = 16384;

/// Ollama's server, by where it listens (`127.0.0.1:11434`).
pub struct Ollama {
    pub host: String,
}

/// What a model said: its text, and the tools it calls with their arguments.
#[derive(Debug, Default, PartialEq)]
pub struct Reply {
    pub text: String,
    pub calls: Vec<(String, Val)>,
}

impl Default for Ollama {
    /// The server `OLLAMA_HOST` names (`host`, `host:port`, `http://host:port`), else Ollama's.
    fn default() -> Self {
        let env = std::env::var("OLLAMA_HOST").unwrap_or_default();
        // An Ollama listening on every address (0.0.0.0) answers on this computer's.
        let at = env.trim().trim_start_matches("http://").trim_end_matches('/').replace("0.0.0.0", "127.0.0.1");
        let host = match at.as_str() {
            "" => "127.0.0.1:11434".into(),
            h if h.contains(':') => at,
            h => format!("{h}:11434"),
        };
        Ollama { host }
    }
}

impl Ollama {
    /// The models pulled into Ollama that can call tools, the last pulled first, and how many of
    /// them run on this computer: those in Ollama's cloud come after, as they send the
    /// conversation to ollama.com.
    pub fn models(&self) -> Result<(Vec<String>, usize), String> {
        let go = AtomicBool::new(false);
        let (mut r, mut body) = (self.send("/api/tags", "", &go)?, String::new());
        while let Some(l) = line(&mut r, &go)? {
            body += &l;
        }
        let tags = Val::parse(body.trim()).ok_or_else(|| tr("Ollama's list of models isn't JSON: update Ollama"))?;
        // An Ollama too old to list what its models can do has each one tried.
        let tools = |m: &&Val| m.arr("capabilities").is_none_or(|c| c.contains(&Val::Str("tools".into())));
        let models: Vec<&Val> = tags.arr("models").unwrap_or_default().iter().filter(tools).collect();
        let named = |cloud: bool| models.iter().filter(move |m| m.get("remote_host").is_some() == cloud).filter_map(|m| m.str("name"));
        Ok((named(false).chain(named(true)).map(String::from).collect(), named(false).count()))
    }

    /// `model`'s reply to the conversation `messages` (`message`, `tool_message`,
    /// `Reply::message`), which may call `tools` (a JSON array of Ollama's functions); its text
    /// and thinking go to `heard` as they come (`true`: thinking). Once `stop` is set, what it
    /// said by then.
    pub fn chat(
        &self,
        model: &str,
        messages: &[String],
        tools: &str,
        stop: &AtomicBool,
        mut heard: impl FnMut(&str, bool),
    ) -> Result<Reply, String> {
        let body = format!(
            r#"{{"model":{},"messages":{},"tools":{tools},"options":{{"num_ctx":{CONTEXT}}}}}"#,
            json_str(model),
            list(messages.iter().cloned())
        );
        let mut r = match self.send("/api/chat", &body, stop) {
            Err(_) if stop.load(Ordering::Relaxed) => return Ok(Reply::default()),
            r => r?,
        };
        let mut reply = Reply::default();
        while let Some(l) = line(&mut r, stop)? {
            let Some(v) = Val::parse(l.trim()) else { continue };
            if let Some(e) = v.str("error") {
                return Err(format!("Ollama: {e}"));
            }
            let Some(m) = v.get("message") else { continue };
            for (key, thinking) in [("thinking", true), ("content", false)] {
                if let Some(t) = m.str(key).filter(|t| !t.is_empty()) {
                    heard(t, thinking);
                }
            }
            reply.text += m.str("content").unwrap_or_default();
            for f in m.arr("tool_calls").unwrap_or_default().iter().filter_map(|c| c.get("function")) {
                if let Some(name) = f.str("name") {
                    reply.calls.push((name.into(), f.get("arguments").cloned().unwrap_or(Val::Obj(vec![]))));
                }
            }
        }
        Ok(reply)
    }

    /// Sends `body` to `path` (a `GET` when empty): the reply to read on past its headers, or
    /// what went wrong with how to fix it. `stop` ends the wait.
    fn send(&self, path: &str, body: &str, stop: &AtomicBool) -> Result<BufReader<TcpStream>, String> {
        let absent =
            || trf("no Ollama at {0}: install it from ollama.com (it then runs by itself), or start it with `ollama serve`", &[&self.host]);
        let addrs = self.host.to_socket_addrs().map_err(|_| absent())?;
        let mut s = addrs.into_iter().find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).ok()).ok_or_else(absent)?;
        // Short waits, so a stop is seen while a model loads or thinks.
        s.set_read_timeout(Some(Duration::from_millis(100))).map_err(|e| e.to_string())?;
        let method = if body.is_empty() { "GET" } else { "POST" };
        // HTTP/1.0: the reply runs to the end of the connection, never in chunks.
        let head = format!(
            "{method} {path} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            self.host,
            body.len()
        );
        s.write_all((head + body).as_bytes()).map_err(|e| trf("Ollama at {0}: {1}", &[&self.host, &e]))?;
        let mut r = BufReader::new(s);
        let status = line(&mut r, stop)?.ok_or_else(|| trf("Ollama at {0} closed the connection", &[&self.host]))?;
        while line(&mut r, stop)?.is_some_and(|h| !h.trim().is_empty()) {}
        if status.split(' ').nth(1) == Some("200") {
            return Ok(r);
        }
        let mut rest = String::new();
        while let Some(l) = line(&mut r, stop)? {
            rest += &l;
        }
        let why = Val::parse(rest.trim()).and_then(|v| v.str("error").map(String::from));
        Err(format!("Ollama: {}", why.as_deref().unwrap_or(status.trim())))
    }
}

/// A message of a conversation: `role` (`system`, `user`) says `text`.
pub fn message(role: &str, text: &str) -> String {
    format!(r#"{{"role":{},"content":{}}}"#, json_str(role), json_str(text))
}

/// What `tool` gave, as the message that answers the model's call to it.
pub fn tool_message(tool: &str, text: &str) -> String {
    format!(r#"{{"role":"tool","tool_name":{},"content":{}}}"#, json_str(tool), json_str(text))
}

impl Reply {
    /// The reply as the model's message, to send back with the rest of the conversation.
    pub fn message(&self) -> String {
        let call = |(name, args): &(String, Val)| format!(r#"{{"function":{{"name":{},"arguments":{}}}}}"#, json_str(name), args.to_json());
        format!(r#"{{"role":"assistant","content":{},"tool_calls":{}}}"#, json_str(&self.text), list(self.calls.iter().map(call)))
    }
}

/// The next line `r` reads, waiting for it; none at the end, or once `stop` is set.
fn line(r: &mut impl BufRead, stop: &AtomicBool) -> Result<Option<String>, String> {
    let mut buf = vec![];
    loop {
        match r.read_until(b'\n', &mut buf) {
            Ok(_) if buf.is_empty() => return Ok(None),
            Ok(_) => return Ok(Some(String::from_utf8_lossy(&buf).into_owned())),
            // What came before the wait ran out stays in `buf`.
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {
                if stop.load(Ordering::Relaxed) {
                    return Ok(None);
                }
            }
            Err(e) => return Err(format!("Ollama: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    /// An Ollama that answers one request with `reply` (status line, headers and body); gives
    /// its host and the request's body.
    fn ollama(reply: &'static str) -> (Ollama, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = l.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let (mut h, mut len) = (String::new(), 0);
            while r.read_line(&mut h).unwrap() > 2 {
                if let Some(n) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = n.trim().parse().unwrap();
                }
                h.clear();
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            s.write_all(reply.as_bytes()).unwrap();
            String::from_utf8(body).unwrap()
        });
        (Ollama { host }, server)
    }

    #[test]
    fn streams_a_reply_with_its_tool_calls() {
        let (o, server) = ollama(concat!(
            "HTTP/1.0 200 OK\r\nContent-Type: application/x-ndjson\r\n\r\n",
            r#"{"message":{"role":"assistant","content":"","thinking":"R1?"},"done":false}"#,
            "\n",
            r#"{"message":{"role":"assistant","content":"R1 is "},"done":false}"#,
            "\n",
            r#"{"message":{"role":"assistant","content":"","tool_calls":[{"id":"c","function":{"index":0,"name":"find","arguments":{"query":"R1"}}}]},"done":false}"#,
            "\n",
            r#"{"message":{"role":"assistant","content":""},"done":true}"#,
        ));
        let mut heard = vec![];
        let said = [message("user", "What is R1?")];
        let reply = o.chat("gemma4", &said, "[]", &AtomicBool::new(false), |t, thinking| heard.push((t.to_string(), thinking))).unwrap();
        assert_eq!(heard, [("R1?".to_string(), true), ("R1 is ".to_string(), false)]);
        assert_eq!(reply, Reply { text: "R1 is ".into(), calls: vec![("find".into(), Val::parse(r#"{"query":"R1"}"#).unwrap())] });
        let sent = Val::parse(&server.join().unwrap()).unwrap();
        assert_eq!((sent.str("model"), sent.at(&["options", "num_ctx"])), (Some("gemma4"), Some(&Val::Num(16384.0))));
        assert_eq!(sent.arr("messages").unwrap()[0].str("content"), Some("What is R1?"));
        // The reply goes back as the model's message.
        let back = Val::parse(&reply.message()).unwrap();
        assert_eq!(back.arr("tool_calls").unwrap()[0].at(&["function", "arguments"]).unwrap().str("query"), Some("R1"));
    }

    #[test]
    fn lists_the_models_that_call_tools() {
        let (o, _) = ollama(concat!(
            "HTTP/1.0 200 OK\r\n\r\n",
            r#"{"models":[{"name":"gemma4:cloud","remote_host":"https://ollama.com:443","capabilities":["tools"]},"#,
            r#"{"name":"gemma4:e2b","capabilities":["completion","tools"]},{"name":"all-minilm","capabilities":["embedding"]},{"name":"old"}]}"#,
        ));
        assert_eq!(o.models().unwrap(), (["gemma4:e2b", "old", "gemma4:cloud"].map(String::from).to_vec(), 2));
    }

    #[test]
    fn says_what_went_wrong() {
        let (o, _) = ollama("HTTP/1.0 404 Not Found\r\n\r\n{\"error\":\"model 'gemma9' not found\"}");
        assert_eq!(o.chat("gemma9", &[], "[]", &AtomicBool::new(false), |_, _| {}), Err("Ollama: model 'gemma9' not found".into()));
        let (o, _) = ollama("HTTP/1.0 200 OK\r\n\r\n{\"error\":\"model runner has unexpectedly stopped\"}\n");
        assert_eq!(o.chat("m", &[], "[]", &AtomicBool::new(false), |_, _| {}), Err("Ollama: model runner has unexpectedly stopped".into()));
        // Nothing listens where the server was.
        let host = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().to_string();
        let e = Ollama { host: host.clone() }.models().unwrap_err();
        assert!(e.starts_with(&format!("no Ollama at {host}: install it")), "{e}");
    }

    #[test]
    fn stops_waiting_when_asked() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let o = Ollama { host: l.local_addr().unwrap().to_string() };
        // A model loading for ever: the request is taken, never answered.
        let _server = std::thread::spawn(move || {
            let c = l.accept();
            std::thread::sleep(Duration::from_secs(5));
            drop(c);
        });
        let stop = AtomicBool::new(true);
        let t = std::time::Instant::now();
        assert_eq!(o.chat("m", &[], "[]", &stop, |_, _| {}), Ok(Reply::default()));
        assert!(t.elapsed() < Duration::from_secs(1));
    }
}
