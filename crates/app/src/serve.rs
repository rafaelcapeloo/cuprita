//! `cuprita --serve <project>`, and Share in the window (`share.rs`): a web page that shows the
//! project's schematic, board and 3D view to whoever opens it, to look at: the browser build
//! (`web.rs`, built by `web.sh`) fetches the project's files from here and follows them as they
//! are saved. Nothing is written: every request reads. See `web.md`.

use cuprita_core::json::{json_str, list};
use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

/// A request's head is read this far; one that doesn't come in this long is dropped.
const HEAD: u64 = 16 * 1024;
const PATIENCE: Duration = Duration::from_secs(10);

/// The project shown: its folder, its board and root sheet there (those it has), the folder of
/// the page's files, and the key its address carries when other computers can reach it.
#[derive(Debug)]
struct Served {
    dir: PathBuf,
    name: String,
    board: Option<String>,
    root: Option<String>,
    web: PathBuf,
    key: Option<String>,
}

/// What a request gets: its status, the kind of its body, the body, whether it never changes
/// (a file `web.sh` names by its contents), and where it is instead (a redirect).
struct Reply {
    status: u16,
    kind: &'static str,
    body: Vec<u8>,
    forever: bool,
    moved: Option<String>,
}

/// A project being shown: its name, the page's address, and the server, which stops when this goes.
pub(crate) struct Server {
    pub(crate) name: String,
    pub(crate) url: String,
    /// Whether another device can open the address (else only this computer: it has no network).
    pub(crate) reached: bool,
    at: SocketAddr,
    stop: Arc<AtomicBool>,
    answering: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// Serves until the process ends (`cuprita --serve`).
    fn wait(mut self) {
        let _ = self.answering.take().map(std::thread::JoinHandle::join);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Told to stop, then woken from its wait for a request.
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&self.at, Duration::from_millis(200));
    }
}

/// Starts showing the project of `file` (its `.kicad_pro`, board or root sheet) on `bind`'s
/// `port`, or with `any` on a free one when that is taken; an error says what is missing and how
/// to get it. Bound to more than this computer, the page's address carries a key made now, and
/// nothing answers without it: who has the address (its QR code) sees the project, no one else.
pub(crate) fn start(file: &Path, bind: &str, port: u16, any: bool) -> Result<Server, String> {
    let ip: IpAddr = bind.parse().map_err(|_| format!("{bind} isn't an address to serve on: 127.0.0.1, or 0.0.0.0 for the network"))?;
    let key = (!ip.is_loopback()).then(|| {
        let random = || std::collections::hash_map::RandomState::new().build_hasher().finish();
        format!("{:016x}", random() ^ random().rotate_left(32))
    });
    let served = Arc::new(Served { key, ..Served::of(file, web_dir()?)? });
    let listener = TcpListener::bind((ip, port))
        .or_else(|e| if any { TcpListener::bind((ip, 0)) } else { Err(e) })
        .map_err(|e| format!("can't serve on {bind}:{port} ({e}): give another --port"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    // The address another device opens: this computer's on its network, when it has one.
    let lan = if ip.is_unspecified() { lan_ip() } else { Some(ip) };
    let host = lan.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let url = format!("http://{}/{}", SocketAddr::new(host, port), served.key.as_ref().map(|k| format!("{k}/")).unwrap_or_default());
    let at = SocketAddr::new(if ip.is_unspecified() { IpAddr::V4(Ipv4Addr::LOCALHOST) } else { ip }, port);
    let (stop, name) = (Arc::new(AtomicBool::new(false)), served.name.clone());
    let (stopped, live) = (stop.clone(), Arc::new(AtomicUsize::new(0)));
    let answering = std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if stopped.load(Ordering::Relaxed) {
                break;
            }
            // A page asks for a handful at once: past these, someone is holding them open.
            if live.fetch_add(1, Ordering::Relaxed) >= 64 {
                live.fetch_sub(1, Ordering::Relaxed);
                continue;
            }
            let (served, live) = (served.clone(), live.clone());
            std::thread::spawn(move || {
                _ = answer(stream, &served);
                live.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });
    Ok(Server { name, url, reached: lan.is_some_and(|ip| !ip.is_loopback()), at, stop, answering: Some(answering) })
}

/// `cuprita --serve`: shows the project of `file` until the process ends.
pub(crate) fn run(file: &Path, bind: &str, port: u16) -> Result<(), String> {
    let server = start(file, bind, port, false)?;
    println!("Cuprita shows {} at {}, to look at; it follows the files as they are saved. Ctrl+C stops it.", server.name, server.url);
    server.wait();
    Ok(())
}

/// This computer's address on its network: the one its way out goes from (asked of the system's
/// routes: nothing is sent), none without a network.
fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // An address kept for examples (RFC 5737): a UDP socket "connected" sends nothing.
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    socket.local_addr().ok().map(|a| a.ip()).filter(|ip| !ip.is_unspecified())
}

/// Reads a request's head and writes its reply.
fn answer(mut stream: TcpStream, served: &Served) -> std::io::Result<()> {
    stream.set_read_timeout(Some(PATIENCE))?;
    stream.set_write_timeout(Some(PATIENCE))?;
    let mut head = BufReader::new(stream.try_clone()?.take(HEAD));
    let (mut request, mut gzip, mut named) = (String::new(), false, true);
    head.read_line(&mut request)?;
    loop {
        let mut line = String::new();
        if head.read_line(&mut line)? <= 2 {
            break;
        }
        let line = line.to_ascii_lowercase();
        gzip |= line.starts_with("accept-encoding:") && line.contains("gzip");
        named &= line.strip_prefix("host:").is_none_or(by_address);
    }
    let mut words = request.split_whitespace();
    let reply = match (words.next(), words.next()) {
        // Asked for by a name that isn't this computer's: a web page elsewhere, its name
        // pointed here to read the project through the browser. The page's address is a number.
        _ if !named => Reply::status(400),
        (Some("GET"), Some(target)) => served.get(target),
        _ => Reply::status(405),
    };
    reply.write(&mut stream, gzip)
}

/// Whether a request's `Host` names this computer by its address or as `localhost`, with or
/// without a port.
fn by_address(host: &str) -> bool {
    let host = host.trim();
    let name = host.parse::<SocketAddr>().map(|a| a.ip().to_string()).unwrap_or_else(|_| host.trim_matches(['[', ']']).to_string());
    name.parse::<IpAddr>().is_ok() || host.rsplit_once(':').map_or(host, |(name, _)| name) == "localhost"
}

impl Served {
    fn of(file: &Path, web: PathBuf) -> Result<Served, String> {
        let file = std::path::absolute(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let (dir, name) = match (file.parent(), file.file_stem()) {
            (Some(dir), Some(stem)) if file.is_file() => (dir.to_path_buf(), stem.to_string_lossy().into_owned()),
            _ => return Err(format!("{} isn't there: give a project's .kicad_pro, .kicad_pcb or .kicad_sch", file.display())),
        };
        let has = |ext: &str| Some(format!("{name}.{ext}")).filter(|f| dir.join(f).is_file());
        let (board, root) = (has("kicad_pcb"), has("kicad_sch"));
        if board.is_none() && root.is_none() {
            return Err(format!("{} has no {name}.kicad_pcb or {name}.kicad_sch to show", dir.display()));
        }
        Ok(Served { dir, name, board, root, web, key: None })
    }

    /// What `target` (a request's path and query) gets: nothing without the address's key, when
    /// it has one.
    fn get(&self, target: &str) -> Reply {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let Some(path) = decode(path) else { return Reply::status(400) };
        let path = match &self.key {
            None => path.as_str(),
            // The page's own address ends in a slash: what it asks for is beside it.
            Some(key) if path.strip_prefix('/') == Some(key) => return Reply { moved: Some(format!("/{key}/")), ..Reply::status(308) },
            Some(key) => match path.strip_prefix('/').and_then(|p| p.strip_prefix(key.as_str())).filter(|p| p.starts_with('/')) {
                Some(path) => path,
                None => return Reply::status(404),
            },
        };
        match path {
            "/" | "/index.html" => Reply::file(&self.web.join("index.html"), "text/html; charset=utf-8", false),
            "/project.json" => Reply { kind: "application/json", body: self.listed().into_bytes(), ..Reply::status(200) },
            "/model" => match query.strip_prefix("f=").and_then(decode).and_then(|f| self.model(&f)) {
                Some(file) => Reply::file(&file, "application/octet-stream", false),
                None => Reply::status(404),
            },
            path => match path.strip_prefix("/project/") {
                // Only what the list names: the project's own files, none a link to another.
                Some(file) if self.files().iter().any(|f| f.0 == file) && !self.dir.join(file).is_symlink() => {
                    Reply::file(&self.dir.join(file), "text/plain; charset=utf-8", false)
                }
                Some(_) => Reply::status(404),
                None => self.page(path.strip_prefix('/').unwrap_or_default()),
            },
        }
    }

    /// One of the page's own files, which `web.sh` puts beside `index.html`.
    fn page(&self, name: &str) -> Reply {
        let kind = match name.rsplit_once('.').map(|n| n.1) {
            Some("js") => "text/javascript",
            Some("wasm") => "application/wasm",
            Some("svg") => "image/svg+xml",
            _ => return Reply::status(404),
        };
        if name.contains(['/', '\\']) || name.starts_with('.') {
            return Reply::status(404);
        }
        // The script and the program are named by their contents: a browser keeps them.
        Reply::file(&self.web.join(name), kind, kind != "image/svg+xml")
    }

    /// The project's files a page shows, each with when it was written (as nanoseconds, its
    /// version): the board, the project file and every sheet in the folder and its folders.
    fn files(&self) -> Vec<(String, u128)> {
        let mut found = vec![];
        let pro = format!("{}.kicad_pro", self.name);
        for name in self.board.iter().chain([&pro]) {
            found.extend(written(&self.dir.join(name)).map(|at| (name.clone(), at)));
        }
        sheets(&self.dir, "", 0, &mut found);
        found
    }

    /// `project.json`: the project's name, its board and root sheet, and its files with their versions.
    fn listed(&self) -> String {
        let name = |n: &Option<String>| n.as_deref().map_or("null".into(), json_str);
        let files = list(self.files().iter().map(|(file, at)| format!("[{},\"{at}\"]", json_str(file))));
        format!(r#"{{"name":{},"board":{},"root":{},"files":{files}}}"#, json_str(&self.name), name(&self.board), name(&self.root))
    }

    /// Where 3D model `file`, as a board writes it, is: in the project's folder or KiCad's
    /// libraries, as the app finds it. A name that could reach anything else finds nothing.
    fn model(&self, file: &str) -> Option<PathBuf> {
        let var = file.strip_prefix("${").or(file.strip_prefix("$(")).and_then(|v| v.split_once(['}', ')'])).map(|v| v.0);
        let known = |v: &str| v == "KIPRJMOD" || v == "KISYS3DMOD" || v.starts_with("KICAD") && v.ends_with("_3DMODEL_DIR");
        let model = [".step", ".stp", ".wrl", ".wrz"].iter().any(|ext| file.to_ascii_lowercase().ends_with(ext));
        let inside = !file.split(['/', '\\']).any(|part| part == "..") && (var.is_some() || !Path::new(file).has_root());
        (model && inside && var.is_none_or(known)).then(|| cuprita_core::models::path(file, Some(&self.dir)).ok()).flatten()
    }
}

/// When the file at `path` was last written, in nanoseconds.
fn written(path: &Path) -> Option<u128> {
    let at = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    Some(at.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos())
}

/// Adds the sheets of folder `dir` (`under` the project's: `power/`) to `found`, and of its
/// folders three deep; autosaves, backups and hidden folders aren't the project's.
fn sheets(dir: &Path, under: &str, depth: usize, found: &mut Vec<(String, u128)>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if name.starts_with(['.', '_']) {
            continue;
        }
        if path.is_dir() && depth < 3 {
            sheets(&path, &format!("{under}{name}/"), depth + 1, found);
        } else if name.ends_with(".kicad_sch") {
            found.extend(written(&path).map(|at| (format!("{under}{name}"), at)));
        }
    }
}

/// The folder of the page's files, which `web.sh` builds: where a package or `install.sh` puts
/// it beside the app (`<prefix>/share/cuprita/web`; a Mac's app, in its `Resources`; Windows'
/// setup, beside the `.exe`), in Cuprita's data folder, else where `web.sh` leaves it when Cuprita
/// runs from its source (`target/web`).
fn web_dir() -> Result<PathBuf, String> {
    let bin = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.to_path_buf()));
    let beside = bin.iter().flat_map(|bin| ["../share/cuprita/web", "../Resources/web", "../web", "web"].map(|web| bin.join(web)));
    let places: Vec<PathBuf> = beside.chain(crate::recent::dir().map(|data| data.join("web"))).collect();
    places.iter().find(|dir| dir.join("index.html").is_file()).cloned().ok_or_else(|| {
        let looked = places.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
        // What to do first; where it looked on a line of its own, which the window leaves out.
        format!("The page's files aren't installed: in Cuprita's source, ./web.sh builds them and ./install.sh puts them in place\n(looked in {looked})")
    })
}

/// A request path's text: its `%XX` bytes decoded; `None` when it isn't UTF-8.
fn decode(path: &str) -> Option<String> {
    let (bytes, mut out, mut i) = (path.as_bytes(), Vec::with_capacity(path.len()), 0);
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => (out.push(byte), i += 3).1,
            (byte, _) => (out.push(byte), i += 1).1,
        }
    }
    String::from_utf8(out).ok()
}

impl Reply {
    fn status(status: u16) -> Reply {
        let body = match status {
            200 | 308 => "",
            400 => "A request Cuprita can't read",
            405 => "Cuprita's page only shows: it takes no changes",
            _ => "Not part of the project shown",
        };
        Reply { status, kind: "text/plain; charset=utf-8", body: body.into(), forever: false, moved: None }
    }

    fn file(path: &Path, kind: &'static str, forever: bool) -> Reply {
        match std::fs::read(path) {
            Ok(body) => Reply { kind, body, forever, ..Reply::status(200) },
            Err(_) => Reply::status(404),
        }
    }

    /// Writes the reply, deflated when the browser takes that and it helps: boards, sheets, STEP
    /// models and the program are text-like, and long.
    fn write(self, stream: &mut TcpStream, gzip: bool) -> std::io::Result<()> {
        let said = match self.status {
            200 => "OK",
            308 => "Permanent Redirect",
            400 => "Bad Request",
            405 => "Method Not Allowed",
            _ => "Not Found",
        };
        let (body, packed) = if gzip && self.body.len() > 1024 {
            let mut packer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(3));
            packer.write_all(&self.body)?;
            (packer.finish()?, "Content-Encoding: gzip\r\n")
        } else {
            (self.body, "")
        };
        let keep = if self.forever { "public, max-age=31536000, immutable" } else { "no-cache" };
        let moved = self.moved.map(|to| format!("Location: {to}\r\n")).unwrap_or_default();
        // A page with a key in its address tells no other site where it came from.
        let head = format!(
            "HTTP/1.1 {} {said}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{packed}{moved}Cache-Control: {keep}\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
            self.status,
            self.kind,
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_the_projects_files_and_nothing_else() {
        let tmp = std::env::temp_dir().join(format!("cuprita-serve-{}", std::process::id()));
        let (dir, web) = (tmp.join("glow"), tmp.join("web"));
        let _ = std::fs::remove_dir_all(&tmp);
        for d in [dir.join("power"), dir.join(".git"), dir.join("3d"), web.clone()] {
            std::fs::create_dir_all(d).unwrap();
        }
        let files = ["glow.kicad_pcb", "glow.kicad_pro", "glow.kicad_sch", "power/5 V.kicad_sch", "_autosave-glow.kicad_sch", "notes.txt"];
        for f in files.iter().chain(&[".git/x.kicad_sch", "3d/usb.step", "secret.txt"]) {
            std::fs::write(dir.join(f), format!("({f})")).unwrap();
        }
        std::fs::write(tmp.join("outside.step"), "no").unwrap();
        for (f, text) in [("index.html", "<html>"), ("cuprita-1.js", "js"), ("cuprita-1_bg.wasm", "wasm")] {
            std::fs::write(web.join(f), text).unwrap();
        }
        // Any of the project's three files names it.
        let s = Served::of(&dir.join("glow.kicad_pro"), web.clone()).unwrap();
        assert_eq!((s.name.as_str(), s.board.as_deref(), s.root.as_deref()), ("glow", Some("glow.kicad_pcb"), Some("glow.kicad_sch")));
        assert!(Served::of(&dir.join("notes.txt"), web.clone()).unwrap_err().contains("no notes.kicad_pcb"));
        assert!(Served::of(&dir.join("none.kicad_pro"), web).unwrap_err().contains("isn't there"));
        // The list: the board, the project file, the sheets (a folder's too); no autosave, no hidden folder.
        let names: Vec<String> = s.files().into_iter().map(|f| f.0).collect();
        assert_eq!(names, ["glow.kicad_pcb", "glow.kicad_pro", "glow.kicad_sch", "power/5 V.kicad_sch"]);
        assert!(s.listed().starts_with(r#"{"name":"glow","board":"glow.kicad_pcb","root":"glow.kicad_sch","files":[["glow.kicad_pcb",""#));
        let got = |target: &str| {
            let r = s.get(target);
            (r.status, String::from_utf8(r.body).unwrap(), r.forever)
        };
        assert_eq!(got("/"), (200, "<html>".into(), false));
        assert_eq!(got("/cuprita-1_bg.wasm"), (200, "wasm".into(), true));
        assert_eq!(got("/project/glow.kicad_pcb"), (200, "(glow.kicad_pcb)".into(), false));
        assert_eq!(got("/project/power/5%20V.kicad_sch").1, "(power/5 V.kicad_sch)");
        // Nothing the list doesn't name, however it is asked for; no page file outside the page's folder.
        for target in ["/project/secret.txt", "/project/../outside.step", "/project/%2e%2e/outside.step", "/project/.git/x.kicad_sch"] {
            assert_eq!(got(target).0, 404, "{target}");
        }
        for target in ["/../glow/secret.txt", "/glow.kicad_pcb", "/.hidden.js", "/sub/cuprita-1.js", "/%ff"] {
            assert!(matches!(got(target).0, 400 | 404), "{target}");
        }
        // A model in the project's folder, as the board writes it; none from anywhere else.
        assert_eq!(got("/model?f=%24%7BKIPRJMOD%7D%2F3d%2Fusb.step"), (200, "(3d/usb.step)".into(), false));
        assert_eq!(got("/model?f=3d/usb.step").0, 200);
        for f in [
            "${KIPRJMOD}/../outside.step",
            "../outside.step",
            "${HOME}/outside.step",
            "/etc/passwd",
            "secret.txt",
            "${KIPRJMOD}/secret.txt",
        ] {
            let target = format!("/model?f={}", f.replace('$', "%24").replace('{', "%7B").replace('}', "%7D"));
            assert_eq!(got(&target).0, 404, "{f}");
        }
        // With a key in its address (shown past this computer), nothing answers without it.
        let keyed = Served { key: Some("k3y".into()), ..Served::of(&dir.join("glow.kicad_pcb"), tmp.join("web")).unwrap() };
        let got = |target: &str| {
            let r = keyed.get(target);
            (r.status, r.moved, String::from_utf8(r.body).unwrap())
        };
        assert_eq!(got("/k3y/"), (200, None, "<html>".into()));
        assert_eq!(got("/k3y/project/glow.kicad_pcb").2, "(glow.kicad_pcb)");
        assert_eq!(got("/k3y"), (308, Some("/k3y/".into()), String::new()));
        for target in ["/", "/project.json", "/project/glow.kicad_pcb", "/k3yx/", "/k3/y/", "/K3Y/", "/x/k3y/"] {
            assert_eq!(got(target).0, 404, "{target}");
        }
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn starts_on_a_free_port_and_stops_when_dropped() {
        let tmp = std::env::temp_dir().join(format!("cuprita-serve-start-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.kicad_sch"), "(kicad_sch)").unwrap();
        // The page's files where a source build leaves them, or the test has no page to serve.
        if web_dir().is_err() {
            return std::fs::remove_dir_all(&tmp).unwrap();
        }
        let ask = |url: &str, path: &str| {
            let (host, _) = url.strip_prefix("http://").unwrap().split_once('/').unwrap();
            let mut stream = TcpStream::connect(host.replace("0.0.0.0", "127.0.0.1")).ok()?;
            stream.write_all(format!("GET {path} HTTP/1.1\r\n\r\n").as_bytes()).ok()?;
            let mut reply = String::new();
            stream.read_to_string(&mut reply).ok()?;
            Some(reply)
        };
        // On this computer alone: a plain address. A taken port: another one, when any will do.
        let local = start(&tmp.join("a.kicad_sch"), "127.0.0.1", 0, false).unwrap();
        assert!(
            local.url.starts_with("http://127.0.0.1:") && local.url.ends_with('/') && local.url.matches('/').count() == 3,
            "{}",
            local.url
        );
        assert!(ask(&local.url, "/project.json").unwrap().contains(r#""root":"a.kicad_sch""#));
        let port: u16 = local.url.rsplit(':').next().unwrap().trim_end_matches('/').parse().unwrap();
        assert!(start(&tmp.join("a.kicad_sch"), "127.0.0.1", port, false).is_err_and(|e| e.contains("another --port")));
        let other = start(&tmp.join("a.kicad_sch"), "127.0.0.1", port, true).unwrap();
        assert_ne!(other.url, local.url);
        // For the network: a key of 16 hex digits in the address, and no answer without it.
        let shared = start(&tmp.join("a.kicad_sch"), "0.0.0.0", 0, false).unwrap();
        let key = shared.url.trim_end_matches('/').rsplit('/').next().unwrap().to_string();
        assert!(key.len() == 16 && key.chars().all(|c| c.is_ascii_hexdigit()), "{}", shared.url);
        let local_url = format!("http://127.0.0.1:{}/", shared.at.port());
        assert!(ask(&local_url, &format!("/{key}/project.json")).unwrap().starts_with("HTTP/1.1 200"));
        assert!(ask(&local_url, "/project.json").unwrap().starts_with("HTTP/1.1 404"));
        // Dropped, it answers no more.
        let at = local.at;
        drop(local);
        std::thread::sleep(Duration::from_millis(100));
        assert!(TcpStream::connect_timeout(&at, Duration::from_millis(300)).is_err());
        assert!(
            start(&tmp.join("none.kicad_pcb"), "127.0.0.1", 0, false).is_err()
                && start(&tmp.join("a.kicad_sch"), "localhost", 0, false).is_err()
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn answers_a_browser_over_the_wire() {
        let tmp = std::env::temp_dir().join(format!("cuprita-serve-wire-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.kicad_pcb"), "(kicad_pcb)".repeat(200)).unwrap();
        std::fs::write(tmp.join("index.html"), "<html>").unwrap();
        let served = Arc::new(Served::of(&tmp.join("a.kicad_pcb"), tmp.clone()).unwrap());
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || listener.incoming().flatten().for_each(|stream| _ = answer(stream, &served)));
        let ask = |request: &str| {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.write_all(request.as_bytes()).unwrap();
            let mut reply = vec![];
            stream.read_to_end(&mut reply).unwrap();
            String::from_utf8_lossy(&reply).into_owned()
        };
        // Asked for by a name of someone else's (pointed at this computer), nothing answers.
        for (host, status) in [("evil.example:8080", "400"), ("localhost:8080", "200"), ("[::1]:8080", "200"), ("192.168.1.7", "200")] {
            let reply = ask(&format!("GET /project/a.kicad_pcb HTTP/1.1\r\nHost: {host}\r\n\r\n"));
            assert!(reply.starts_with(&format!("HTTP/1.1 {status} ")), "{host}: {reply}");
        }
        assert!(ask("GET é HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404 "));
        let plain = ask("GET /project/a.kicad_pcb HTTP/1.1\r\nHost: 127.0.0.1:8080\r\n\r\n");
        assert!(
            plain.starts_with("HTTP/1.1 200 OK\r\n") && plain.contains("Content-Length: 2200\r\n") && plain.ends_with("(kicad_pcb)"),
            "{plain}"
        );
        let packed = ask("GET /project/a.kicad_pcb HTTP/1.1\r\nAccept-Encoding: gzip, br\r\n\r\n");
        assert!(packed.contains("Content-Encoding: gzip\r\n") && packed.len() < 400, "{packed}");
        // It only shows: nothing is taken.
        assert!(ask("POST /project/a.kicad_pcb HTTP/1.1\r\nContent-Length: 1\r\n\r\nx").starts_with("HTTP/1.1 405 "));
        assert!(ask("GET /project.json HTTP/1.1\r\n\r\n").contains(r#""root":null"#));
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
