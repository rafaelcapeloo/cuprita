//! Browser entry point: runs the app on the page's `<canvas id="cuprita">`, opens the boards
//! dropped on it, saves them as downloads and has the browser ask before unsaved edits are lost
//! with the page; on a page `cuprita --serve` shows a project on (`serve.rs`), it fetches that
//! project and follows it, to look at. Build with `./web.sh`.

use crate::tr::{tr, trf};
use crate::{App, Files, dropped_board};
use cuprita_core::json::Val;
use cuprita_core::step::Mesh;
use eframe::egui;
use eframe::wasm_bindgen::{JsCast, JsValue, closure::Closure};
use eframe::web_sys::js_sys::futures::{JsFuture, spawn_local};
use eframe::web_sys::{self, js_sys};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Where the app reads a served project's files: no folder of the visitor's computer.
const DIR: &str = "/project";

/// How often the page asks the server whether the project's files changed, in milliseconds.
const EVERY: i32 = 2000;

thread_local! {
    /// Dropped files the browser has finished reading, for the next frame.
    static READ: RefCell<Option<Files>> = const { RefCell::new(None) };
    /// The last downloads' object URLs, freed at the next ones: freeing them at once can cancel them.
    static URLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Whether the board has unsaved edits, which leaving the page would lose.
    pub static UNSAVED: Cell<bool> = const { Cell::new(false) };
    /// A served project's files by the path the app reads each at: its text, and its version on
    /// the server as a time (`file::written`).
    static FILES: RefCell<HashMap<PathBuf, (String, SystemTime)>> = RefCell::new(HashMap::new());
    /// The served project's file to open once its files are here: its board, else its root sheet.
    static OPEN: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// The served project's 3D models by their file as the board writes it: `None` while one is
    /// fetched; and those read since the app last took them (`poll`).
    static MESHES: RefCell<HashMap<String, Option<Result<Mesh, String>>>> = RefCell::new(HashMap::new());
    static READY: RefCell<Vec<(String, Result<Mesh, String>)>> = const { RefCell::new(Vec::new()) };
    /// The app's window, to draw again when something fetched is here.
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    /// Whether the served project opened on a narrow page, its docks folded, and no hand has
    /// touched the page since.
    static FOLDED: Cell<bool> = const { Cell::new(false) };
}

/// Has the app draw again: what the page fetched is here.
fn redraw() {
    CTX.with_borrow(|ctx| ctx.iter().for_each(egui::Context::request_repaint));
}

/// A served project's file as `file::read_text` reads it.
pub(crate) fn read_text(path: &Path) -> std::io::Result<String> {
    let text = FILES.with_borrow(|files| files.get(path).map(|f| f.0.clone()));
    text.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "the server doesn't show it"))
}

/// A served project's file's version, as `file::written` gives a file's time.
pub(crate) fn written(path: &Path) -> Option<SystemTime> {
    FILES.with_borrow(|files| files.get(path).map(|f| f.1))
}

/// Starts the app; if the browser can't run it, the page's note says why.
pub fn start() {
    // Closing, reloading or leaving the page with unsaved edits: the browser asks first.
    let ask = Closure::<dyn Fn(web_sys::Event)>::new(|e: web_sys::Event| {
        if UNSAVED.get() {
            e.prevent_default();
        }
    });
    if let Some(w) = web_sys::window() {
        let _ = w.add_event_listener_with_callback("beforeunload", ask.as_ref().unchecked_ref());
    }
    ask.forget(); // the page keeps it for good
    spawn_local(async {
        let doc = web_sys::window().and_then(|w| w.document()).expect("runs in a web page");
        let canvas = doc.get_element_by_id("cuprita").and_then(|e| e.dyn_into().ok()).expect("page has <canvas id=\"cuprita\">");
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| {
                    CTX.set(Some(cc.egui_ctx.clone()));
                    spawn_local(follow());
                    let mut app = App::new(cc, None, false, None, None, None);
                    app.status = tr("Drop a .kicad_pcb file here, with its .kicad_pro for the design rules").into();
                    Ok(Box::new(app))
                }),
            )
            .await;
        // The page's "Loading" note goes away, or says why the app can't run.
        if let Some(note) = doc.get_element_by_id("note") {
            match started {
                Ok(()) => note.remove(),
                Err(e) => {
                    crate::tr::read(); // the app that reads the user's language never started
                    note.set_text_content(Some(&trf(
                        "Cuprita can't run in this browser: {0}. It needs WebGPU or WebGL 2: try a current Chrome, Edge, Firefox or Safari.",
                        &[&e.as_string().unwrap_or_else(|| format!("{e:?}"))],
                    )))
                }
            }
        }
    });
}

/// Each frame: a served project opens once its files are here, to look at (`App::viewer`), and
/// takes its 3D models as they are read; any other page opens the boards dropped on it.
pub fn poll(app: &mut App, ctx: &egui::Context) {
    if let Some(path) = OPEN.take() {
        app.viewer = true;
        // On a phone's page the docks start folded: the canvas first.
        if crate::shell::narrow(ctx) {
            (app.left.open, app.right.open) = (false, false);
            FOLDED.set(true);
        }
        app.open(&path);
    }
    if !app.viewer {
        return open_dropped(app, ctx);
    }
    // A page may open before its window has its size (a tab not yet shown): wide before any hand
    // touched it, it was no phone's, and gets its docks.
    if FOLDED.get() {
        let touched =
            ctx.input(|i| i.pointer.any_down() || i.any_touches() || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. })));
        if !touched && !crate::shell::narrow(ctx) {
            (app.left.open, app.right.open) = (true, true);
        }
        FOLDED.set(!touched && crate::shell::narrow(ctx));
    }
    let read = READY.take();
    if !read.is_empty() {
        app.models.extend(read);
        app.sync_gpu();
    }
}

/// Follows the project a server shows on this page: fetches the files `project.json` lists,
/// each again when its version there changes, so the app shows the project as it is now
/// (`watch.rs` reads again what changed). A page no server shows a project on has no list: nothing.
async fn follow() {
    let mut shown = false;
    loop {
        if let Some(list) = fetch_text("project.json").await.ok().and_then(|text| Val::parse(&text)) {
            let listed: Vec<(PathBuf, String, SystemTime)> = (list.arr("files").unwrap_or_default().iter())
                .filter_map(|f| match f {
                    Val::Arr(f) => match &f[..] {
                        [Val::Str(name), Val::Str(version)] => {
                            let at = SystemTime::UNIX_EPOCH + Duration::from_nanos(version.parse().ok()?);
                            Some((Path::new(DIR).join(name), name.clone(), at))
                        }
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            let mut changed = false;
            for (path, name, at) in &listed {
                if written(path) != Some(*at)
                    && let Ok(text) = fetch_text(&format!("project/{}", String::from(js_sys::encode_uri(name)))).await
                {
                    FILES.with_borrow_mut(|files| files.insert(path.clone(), (text, *at)));
                    changed = true;
                }
            }
            FILES.with_borrow_mut(|files| files.retain(|path, _| listed.iter().any(|l| l.0 == *path)));
            if !shown && let Some(first) = list.str("board").or(list.str("root")) {
                OPEN.set(Some(Path::new(DIR).join(first)));
                shown = true;
            }
            if changed {
                redraw();
            }
        } else if !shown {
            return;
        }
        sleep(EVERY).await;
    }
}

/// Asks the server for the 3D models `files` (as the board writes them) it hasn't been asked for,
/// and reads them one at a time, the page drawing between them; `poll` hands them to the app, at
/// once those read before (a board shown again asks again).
pub(crate) fn fetch_models(files: Vec<String>) {
    let mut new = vec![];
    MESHES.with_borrow_mut(|meshes| {
        for file in files {
            match meshes.get(&file) {
                Some(Some(read)) => READY.with_borrow_mut(|r| r.push((file, read.clone()))),
                Some(None) => {}
                None => {
                    meshes.insert(file.clone(), None);
                    new.push(file);
                }
            }
        }
    });
    redraw();
    spawn_local(async move {
        for file in new {
            let url = format!("model?f={}", String::from(js_sys::encode_uri_component(&file)));
            let mesh = match fetch(&url).await {
                Ok(reply) => bytes(reply).await.and_then(|data| cuprita_core::step::read(&data)).map_err(|e| format!("{file}: {e}")),
                Err(_) => Err(trf("{0}: the computer showing this project has no such 3D model", &[&file])),
            };
            MESHES.with_borrow_mut(|meshes| meshes.insert(file.clone(), Some(mesh.clone())));
            READY.with_borrow_mut(|r| r.push((file, mesh)));
            redraw();
        }
    });
}

/// The server's reply to a request for `url` (beside the page), an error when it has none.
async fn fetch(url: &str) -> Result<web_sys::Response, String> {
    let window = web_sys::window().ok_or("not in a web page")?;
    let reply: web_sys::Response = JsFuture::from(window.fetch_with_str(url)).await.map_err(js)?.dyn_into().map_err(js)?;
    if reply.ok() { Ok(reply) } else { Err(format!("{url}: {}", reply.status_text())) }
}

async fn fetch_text(url: &str) -> Result<String, String> {
    let text = JsFuture::from(fetch(url).await?.text().map_err(js)?).await.map_err(js)?;
    text.as_string().ok_or_else(|| format!("{url}: not text"))
}

async fn bytes(reply: web_sys::Response) -> Result<Vec<u8>, String> {
    let data = JsFuture::from(reply.array_buffer().map_err(js)?).await.map_err(js)?;
    Ok(js_sys::Uint8Array::new(&data).to_vec())
}

/// Waits `ms` milliseconds, the page going on.
async fn sleep(ms: i32) {
    let wait = js_sys::Promise::new(&mut |done, _| {
        let _ = web_sys::window().map(|w| w.set_timeout_with_callback_and_timeout_and_arguments_0(&done, ms));
    });
    let _ = JsFuture::from(wait).await;
}

/// What the browser said went wrong, in words.
fn js(e: JsValue) -> String {
    format!("{e:?}")
}

/// Opens the board dropped on the page once the browser has read it, with the design rules of
/// a `.kicad_pro` dropped along with it.
fn open_dropped(app: &mut App, ctx: &egui::Context) {
    let dropped = ctx.input(|i| i.raw.dropped_files.clone());
    if !dropped.is_empty() {
        let ctx = ctx.clone();
        spawn_local(async move {
            let mut files = Vec::new();
            for f in &dropped {
                files.push((f.path().display().to_string(), f.bytes_async().await));
            }
            READ.set(Some(files));
            ctx.request_repaint();
        });
    }
    let Some(files) = READ.take() else { return };
    match dropped_board(files) {
        Ok((name, src, pro)) => app.leave(move |app| _ = app.load(None, &name, src, pro.as_deref())),
        Err(e) => app.status = e,
    }
}

/// Offers `files` (name, contents) to the browser as downloads.
pub fn download(files: &[(&str, &[u8])]) -> Result<(), String> {
    for old in URLS.take() {
        let _ = web_sys::Url::revoke_object_url(&old);
    }
    let doc = web_sys::window().and_then(|w| w.document()).ok_or("not in a web page")?;
    for &(name, data) in files {
        let bytes = web_sys::js_sys::Uint8Array::from(data);
        let blob = web_sys::Blob::new_with_u8_array_sequence(&web_sys::js_sys::Array::of1(&bytes)).map_err(js)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
        let link: web_sys::HtmlAnchorElement = doc.create_element("a").map_err(js)?.dyn_into().map_err(|_| "no link element")?;
        link.set_href(&url);
        link.set_download(name);
        link.click();
        URLS.with_borrow_mut(|u| u.push(url));
    }
    Ok(())
}
