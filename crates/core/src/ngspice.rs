//! Simulation in ngspice: its shared library, KiCad's or the system's, found with nothing to set
//! up, runs a project's SPICE netlist (`schematic::Deck`) in this process, or, where KiCad's
//! Flatpak's loads only in its sandbox, in Cuprita run there (`crates/core/ngspice.md`).

use crate::schematic::Deck;
use crate::tr::{tr, trf};
use libloading::Library;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// A simulation's results: each analysis', in the order ngspice ran them.
#[derive(Debug, Default)]
pub struct Sim {
    pub plots: Vec<Plot>,
    /// What ngspice said of the netlist and the run: warnings, and why a run stopped short.
    pub log: Vec<String>,
}

/// One analysis' results.
#[derive(Debug)]
pub struct Plot {
    /// ngspice's name for it: `tran1`, `ac1`, `dc1`, `op1`.
    pub name: String,
    /// Its vectors as ngspice made them: the scale (time, frequency, sweep) first, but in an
    /// operating point, which has none.
    pub vectors: Vec<Vector>,
}

#[derive(Debug)]
pub struct Vector {
    /// ngspice's name, in lower case: a node's (`/out`), a source's current (`v1#branch`).
    pub name: String,
    /// `s`, `Hz`, `V`, `A`, or none.
    pub unit: &'static str,
    /// Its values; an AC analysis' are complex, `im` holding their imaginary parts (else empty).
    pub re: Vec<f64>,
    pub im: Vec<f64>,
}

/// ngspice's `vector_info`.
#[repr(C)]
struct VectorInfo {
    name: *const c_char,
    kind: c_int,
    flags: i16,
    real: *const f64,
    complex: *const [f64; 2],
    len: c_int,
}

type Print = extern "C" fn(*const c_char, c_int, *mut c_void) -> c_int;
type Exit = extern "C" fn(c_int, bool, bool, c_int, *mut c_void) -> c_int;
type Running = extern "C" fn(bool, c_int, *mut c_void) -> c_int;
type Init =
    unsafe extern "C" fn(Option<Print>, Option<Print>, Option<Exit>, *const c_void, *const c_void, Option<Running>, *mut c_void) -> c_int;

/// The functions of libngspice this calls (ngspice's manual, "ngspice as shared library").
struct Ngspice {
    init: Init,
    command: unsafe extern "C" fn(*const c_char) -> c_int,
    circ: unsafe extern "C" fn(*const *const c_char) -> c_int,
    all_plots: unsafe extern "C" fn() -> *const *const c_char,
    all_vecs: unsafe extern "C" fn(*const c_char) -> *const *const c_char,
    vec_info: unsafe extern "C" fn(*const c_char) -> *const VectorInfo,
    reset: Option<unsafe extern "C" fn() -> c_int>,
    input_path: Option<unsafe extern "C" fn(*const c_char) -> *const c_char>,
    /// Where it is (its code models are near), if it was found by path.
    path: Option<PathBuf>,
    _lib: Library,
}

static NGSPICE: Mutex<Option<Ngspice>> = Mutex::new(None);
/// What ngspice prints on stderr (`stderr …` lines), errors and warnings, during a run.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Whether ngspice asked to quit after a fatal error: it starts again before the next run.
static QUIT: AtomicBool = AtomicBool::new(false);
/// How far the run going is: the percentage ngspice tells (`tran: 45.3%`), as `f32` bits.
static PROGRESS: AtomicU32 = AtomicU32::new(0);
/// How many background runs started, and how many ended.
static STARTED: AtomicUsize = AtomicUsize::new(0);
static ENDED: AtomicUsize = AtomicUsize::new(0);

extern "C" fn print(text: *const c_char, _: c_int, _: *mut c_void) -> c_int {
    // SAFETY: ngspice passes a C string it owns for the call.
    let line = if text.is_null() { "".into() } else { unsafe { CStr::from_ptr(text) }.to_string_lossy() };
    if let (Some(e), Ok(mut log)) = (line.strip_prefix("stderr "), LOG.lock()) {
        log.push(e.to_string());
    }
    0
}

extern "C" fn status(text: *const c_char, _: c_int, _: *mut c_void) -> c_int {
    // SAFETY: ngspice passes a C string it owns for the call.
    let line = if text.is_null() { "".into() } else { unsafe { CStr::from_ptr(text) }.to_string_lossy() };
    if let Some(p) = line.rsplit(':').next().and_then(|p| p.trim().strip_suffix('%')?.parse::<f32>().ok()) {
        PROGRESS.store(p.to_bits(), Ordering::Relaxed);
    }
    0
}

/// How far the simulation going is, in percent of its analysis: 0 until ngspice tells.
pub fn progress() -> f32 {
    f32::from_bits(PROGRESS.load(Ordering::Relaxed))
}

extern "C" fn quit(_: c_int, _: bool, _: bool, _: c_int, _: *mut c_void) -> c_int {
    QUIT.store(true, Ordering::SeqCst);
    0
}

extern "C" fn running(no_run: bool, _: c_int, _: *mut c_void) -> c_int {
    (if no_run { &ENDED } else { &STARTED }).fetch_add(1, Ordering::SeqCst);
    0
}

/// Runs `deck` in ngspice, its analyses in the background so that `stop` set halts it (what
/// it simulated by then comes back); one run at a time in a process, others wait.
pub fn simulate(deck: &Deck, stop: &AtomicBool) -> Result<Sim, String> {
    let mut held = NGSPICE.lock().unwrap_or_else(PoisonError::into_inner);
    if QUIT.swap(false, Ordering::SeqCst) {
        if let Some(reset) = held.as_ref().and_then(|ng| ng.reset) {
            // SAFETY: ngspice's own reset, before it starts again.
            unsafe { reset() };
        }
        *held = None;
    }
    if held.is_none() {
        match start() {
            Ok(ng) => *held = Some(ng),
            // KiCad's Flatpak's is built for its runtime's C library, which may be newer than
            // this system's: it loads only in its sandbox, so the run goes there.
            Err(_) if sandboxed() => return inside(deck, stop),
            Err(e) => return Err(e),
        }
    }
    let ng = held.as_ref().expect("started");
    let say = |c: &str| {
        let c = CString::new(c).unwrap_or_default();
        // SAFETY: a C string, as `ngSpice_Command` takes it.
        unsafe { (ng.command)(c.as_ptr()) }
    };
    // The last run's circuit goes.
    say("remcirc");
    LOG.lock().unwrap_or_else(PoisonError::into_inner).clear();
    PROGRESS.store(0, Ordering::Relaxed);
    if !deck.behavior.is_empty() {
        say(&format!("set ngbehavior={}", deck.behavior));
    }
    if let Some(input) = ng.input_path {
        let dir = CString::new(format!("{}{}", deck.dir.display(), std::path::MAIN_SEPARATOR)).unwrap_or_default();
        // SAFETY: a C string; ngspice copies it.
        unsafe { input(dir.as_ptr()) };
    }
    let lines: Vec<CString> = deck.text.lines().map(|l| CString::new(l).unwrap_or_default()).collect();
    let mut ptrs: Vec<*const c_char> = lines.iter().map(|l| l.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    // SAFETY: a null-terminated array of C strings, which ngspice copies.
    if unsafe { (ng.circ)(ptrs.as_ptr()) } != 0 || QUIT.load(Ordering::SeqCst) {
        return Err(errors(tr("ngspice can't read the netlist")));
    }
    let (started, ended, at) = (STARTED.load(Ordering::SeqCst), ENDED.load(Ordering::SeqCst), Instant::now());
    say("bg_run");
    let (start, end) = (|| STARTED.load(Ordering::SeqCst) > started, || ENDED.load(Ordering::SeqCst) > ended);
    // ngspice's thread starts a moment after: halted before, it would run on unseen.
    while !start() && !end() && !QUIT.load(Ordering::SeqCst) {
        if at.elapsed() > Duration::from_secs(10) {
            return Err(tr("ngspice didn't start the simulation").into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut halted = false;
    while !end() && !QUIT.load(Ordering::SeqCst) {
        if stop.load(Ordering::SeqCst) && !halted {
            say("bg_halt");
            halted = true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // SAFETY: `ngSpice_AllPlots` takes nothing.
    let plots: Vec<Plot> = names(unsafe { (ng.all_plots)() }).into_iter().rev().filter(|p| p != "const").map(|p| plot(ng, p)).collect();
    // ngspice's copy goes.
    say("destroy all");
    if plots.is_empty() {
        return Err(errors(tr("ngspice ran no analysis")));
    }
    Ok(Sim { plots, log: std::mem::take(&mut LOG.lock().unwrap_or_else(PoisonError::into_inner)) })
}

/// The line before a run's results in what `helper` prints, and after the netlist it reads.
const MARK: &str = "cuprita ngspice 1";
/// The units a vector has (`vector`), as `read` gives them back.
const UNITS: [&str; 4] = ["s", "Hz", "V", "A"];

/// Whether KiCad's Flatpak is here with its ngspice: a run can go inside it.
fn sandboxed() -> bool {
    libraries().iter().any(|(p, _)| p.components().any(|c| c.as_os_str() == "org.kicad.kicad"))
}

/// Runs `deck` in Cuprita itself (`helper`) started inside KiCad's Flatpak, where its ngspice
/// loads: the same run, its netlist, progress and results over pipes. `stop` closes the pipe
/// in, which halts the run there; what it simulated by then comes back.
fn inside(deck: &Deck, stop: &AtomicBool) -> Result<Sim, String> {
    let failed = |why: String| trf("ngspice didn't run in KiCad's Flatpak: {0}", &[&why]);
    let exe = std::env::current_exe().map_err(|e| failed(e.to_string()))?;
    // The sandbox has its own /usr, the system's at /run/host/usr; other folders it is shown.
    let (mut run, system) = (Command::new("flatpak"), exe.starts_with("/usr"));
    run.args(["run", "--filesystem=host-os:ro"]);
    let seen = if system { Path::new("/run/host").join(exe.strip_prefix("/").unwrap_or(&exe)) } else { exe.clone() };
    // Whole paths: Flatpak takes none with `..`, and the run there may start in another folder.
    let dir = std::fs::canonicalize(&deck.dir).unwrap_or(deck.dir.clone());
    for shown in [exe.parent().filter(|_| !system), Some(dir.as_path())].into_iter().flatten() {
        run.arg(format!("--filesystem={}", shown.display()));
    }
    run.arg(format!("--command={}", seen.display())).args(["org.kicad.kicad", "--ngspice"]);
    let mut child = run.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| failed(e.to_string()))?;
    let (mut to, from, said) =
        (child.stdin.take().expect("piped"), child.stdout.take().expect("piped"), child.stderr.take().expect("piped"));
    PROGRESS.store(0, Ordering::Relaxed);
    let sent = write!(to, "{}\n{}\n{}\n{MARK}\n", deck.behavior, dir.display(), deck.text.trim_end()).and_then(|_| to.flush());
    let reading = std::thread::spawn(move || read(&mut std::io::BufReader::new(from)));
    // What Flatpak or ngspice say meanwhile is read, lest they wait on a full pipe: why, when no results come.
    let saying = std::thread::spawn(move || std::io::read_to_string(said).unwrap_or_default());
    let mut to = Some(to);
    while !reading.is_finished() {
        if stop.load(Ordering::SeqCst) {
            to.take();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let (sim, said, _) = (reading.join().ok().flatten(), saying.join().unwrap_or_default(), child.wait());
    sent.map_err(|e| failed(e.to_string()))?;
    sim.unwrap_or_else(|| Err(failed(said.lines().rfind(|l| !l.trim().is_empty()).unwrap_or(tr("it gave no results")).into())))
}

/// `cuprita --ngspice`, which `inside` starts in KiCad's Flatpak: runs the netlist stdin gives
/// (its ngbehavior and its folder, a line each, then its lines up to `MARK`) and prints its
/// results (`write`), how far it is meanwhile (`progress 45.3`); stdin closing halts the run.
pub fn helper() {
    static STOP: AtomicBool = AtomicBool::new(false);
    static DONE: AtomicBool = AtomicBool::new(false);
    let deck = {
        let mut lines = std::io::stdin().lines().map_while(Result::ok);
        let (behavior, dir) = (lines.next().unwrap_or_default(), lines.next().unwrap_or_default());
        let text = lines.take_while(|l| l != MARK).map(|l| l + "\n").collect();
        Deck { text, behavior: behavior.leak(), dir: dir.into(), signals: Default::default(), traces: vec![] }
    };
    std::thread::spawn(|| {
        let _ = std::io::stdin().read(&mut [0]);
        STOP.store(true, Ordering::SeqCst);
    });
    std::thread::spawn(|| {
        let mut last = 0.0;
        while !DONE.load(Ordering::SeqCst) {
            if progress() != last {
                last = progress();
                println!("progress {last}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let sim = simulate(&deck, &STOP);
    DONE.store(true, Ordering::SeqCst);
    // Held to the end: no line of progress falls among the results.
    let _ = write(&sim, &mut std::io::BufWriter::new(std::io::stdout().lock()));
}

/// A run's results, or why there are none, down a pipe after `MARK`: a line each, a vector's
/// values after its line as memory holds them (8 bytes each), so `read` gives them back to the
/// bit, and a long run's millions go as fast as they are copied.
fn write(sim: &Result<Sim, String>, out: &mut impl Write) -> std::io::Result<()> {
    writeln!(out, "{MARK}")?;
    match sim {
        Err(e) => e.split('\n').try_for_each(|l| writeln!(out, "error {l}"))?,
        Ok(sim) => {
            for p in &sim.plots {
                writeln!(out, "plot {}", p.name)?;
                for v in &p.vectors {
                    writeln!(out, "vector {} {} {}:{}", v.re.len(), v.im.len(), v.unit, v.name)?;
                    v.re.iter().chain(&v.im).try_for_each(|x| out.write_all(&x.to_ne_bytes()))?;
                }
            }
            sim.log.iter().flat_map(|l| l.lines()).try_for_each(|l| writeln!(out, "log {l}"))?;
        }
    }
    out.flush()
}

/// What `write` wrote, after whatever ngspice itself printed before it, `helper`'s lines of
/// progress going to `PROGRESS`; `None` without `MARK`, or cut short.
fn read(from: &mut impl BufRead) -> Option<Result<Sim, String>> {
    /// A vector's `n` values.
    fn floats(from: &mut impl BufRead, n: usize) -> Option<Vec<f64>> {
        let mut bytes = vec![0; n.checked_mul(8)?];
        from.read_exact(&mut bytes).ok()?;
        Some(bytes.as_chunks::<8>().0.iter().map(|b| f64::from_ne_bytes(*b)).collect())
    }
    let (mut sim, mut error, mut marked, mut line) = (Sim::default(), None::<String>, false, vec![]);
    while from.read_until(b'\n', &mut line).ok()? > 0 {
        let text = String::from_utf8_lossy(&line).trim_end_matches('\n').to_string();
        line.clear();
        let (kind, rest) = text.split_once(' ').unwrap_or((&text, ""));
        match (marked, kind) {
            (false, "progress") => PROGRESS.store(rest.parse::<f32>().unwrap_or(0.0).to_bits(), Ordering::Relaxed),
            (false, _) => marked = text == MARK,
            (_, "error") => error = Some(error.map_or(rest.into(), |e| format!("{e}\n{rest}"))),
            (_, "plot") => sim.plots.push(Plot { name: rest.into(), vectors: vec![] }),
            (_, "vector") => {
                let mut parts = rest.splitn(3, ' ');
                let (re, im) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
                let (unit, name) = parts.next()?.split_once(':')?;
                let unit = UNITS.iter().find(|u| **u == unit).copied().unwrap_or("");
                let (re, im) = (floats(from, re)?, floats(from, im)?);
                sim.plots.last_mut()?.vectors.push(Vector { name: name.into(), unit, re, im });
            }
            (_, "log") => sim.log.push(rest.into()),
            _ => {}
        }
    }
    marked.then(|| error.map_or(Ok(sim), Err))
}

/// The run's error, ngspice's words after `what`; with none, how to give the netlist an analysis.
fn errors(what: &str) -> String {
    let log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    let said: Vec<&str> = log.iter().map(String::as_str).filter(|l| !l.contains("initialization file spinit")).collect();
    match said.is_empty() {
        true => trf("{0}: give the schematic an analysis (KiCad's Simulator, or a text such as .tran 1u 1m)", &[&what]),
        false => format!("{what}: {}", said.join("\n")),
    }
}

/// An analysis' plot, its vectors copied (the scale, made first, is listed last) but those inside
/// subcircuits (`xu1.5`), which aren't the schematic's nets.
fn plot(ng: &Ngspice, name: String) -> Plot {
    let c = CString::new(name.as_str()).unwrap_or_default();
    // SAFETY: a plot's name, as `ngSpice_AllVecs` takes it.
    let all = names(unsafe { (ng.all_vecs)(c.as_ptr()) });
    let vectors = all.into_iter().rev().filter(|v| !v.contains('.')).filter_map(|v| vector(ng, &name, v)).collect();
    Plot { name, vectors }
}

fn vector(ng: &Ngspice, plot: &str, name: String) -> Option<Vector> {
    let full = CString::new(format!("{plot}.{name}")).ok()?;
    // SAFETY: ngspice's vector, valid until the next command; its data are copied at once.
    let info = unsafe { (ng.vec_info)(full.as_ptr()).as_ref()? };
    let len = info.len.max(0) as usize;
    let unit = UNITS.get((info.kind as usize).wrapping_sub(1)).copied().unwrap_or("");
    let (re, im) = if !info.real.is_null() {
        // SAFETY: `len` values at `real`.
        (unsafe { std::slice::from_raw_parts(info.real, len) }.to_vec(), vec![])
    } else if !info.complex.is_null() {
        // SAFETY: `len` complex values at `complex`.
        unsafe { std::slice::from_raw_parts(info.complex, len) }.iter().map(|c| (c[0], c[1])).unzip()
    } else {
        (vec![], vec![])
    };
    Some(Vector { name, unit, re, im })
}

/// The names in a null-terminated list ngspice gives (plots, or a plot's vectors).
fn names(mut p: *const *const c_char) -> Vec<String> {
    let mut out = vec![];
    // SAFETY: ngspice's null-terminated list of C strings.
    unsafe {
        while !p.is_null() && !(*p).is_null() {
            out.push(CStr::from_ptr(*p).to_string_lossy().into_owned());
            p = p.add(1);
        }
    }
    out
}

/// Loads and starts libngspice, its code models (XSPICE's, POLY sources') from beside it when
/// its own init file isn't where it was built to be (KiCad's flatpak, KiCad on Windows).
fn start() -> Result<Ngspice, String> {
    let (lib, path) = load()?;
    let ng = bind(lib, path).map_err(|e| trf("libngspice lacks {0}: install a newer ngspice", &[&e]))?;
    LOG.lock().unwrap_or_else(PoisonError::into_inner).clear();
    // As it starts, ngspice runs a `.spiceinit` in the folder Cuprita was started in, before the
    // user's own in their home: a project's folder may hold one from someone else.
    // shortcut: refused, as KiCad's ngspice crashes on `ngSpice_nospiceinit` before it starts;
    // call that instead once it holds.
    let home = ["HOME", "USERPROFILE"].iter().find_map(std::env::var_os).map(PathBuf::from);
    if let Some(here) = std::env::current_dir().ok().filter(|d| d.join(".spiceinit").exists() && Some(d) != home.as_ref()) {
        return Err(trf(
            "{0} holds a .spiceinit, which ngspice would run as it starts: remove it, or start Cuprita from another folder",
            &[&here.display()],
        ));
    }
    // SAFETY: the callbacks live as long as the process; the data pointers may be null.
    if unsafe { (ng.init)(Some(print), Some(status), Some(quit), std::ptr::null(), std::ptr::null(), Some(running), std::ptr::null_mut()) }
        != 0
    {
        return Err(tr("libngspice didn't start").into());
    }
    let spinit = LOG.lock().unwrap_or_else(PoisonError::into_inner).iter().any(|l| l.contains("initialization file spinit"));
    if spinit && let Some(dir) = ng.path.as_ref().and_then(|p| p.parent()) {
        // shortcut: code models are looked for beside the library and in ../lib (KiCad for
        // Windows); add a folder when an install keeps them elsewhere.
        for cm in
            [dir.join("ngspice"), dir.join("../lib/ngspice")].iter().flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        {
            if cm.path().extension().is_some_and(|e| e == "cm") {
                let c = CString::new(format!("codemodel {}", cm.path().display())).unwrap_or_default();
                // SAFETY: a C string, as `ngSpice_Command` takes it.
                unsafe { (ng.command)(c.as_ptr()) };
            }
        }
    }
    Ok(ng)
}

/// The first libngspice that loads, with its path when it is KiCad's own; else what to do and,
/// on a line of its own, the loader's words (Home shows the first line alone).
fn load() -> Result<(Library, Option<PathBuf>), String> {
    let mut failed = vec![];
    for (path, known) in libraries() {
        // SAFETY: loading ngspice runs its initializers, which have no conditions.
        match unsafe { Library::new(&path) } {
            Ok(lib) => return Ok((lib, known.then_some(path))),
            Err(e) if known => failed.push(format!("{}: {e}", path.display())),
            Err(_) => {}
        }
    }
    Err(match failed.is_empty() {
        true => tr("ngspice isn't installed: install KiCad, which brings it, or ngspice's shared library (libngspice)").into(),
        // A KiCad's own that doesn't load and isn't a Flatpak's to run in (`inside`).
        false => format!(
            "{}\n{}",
            tr("KiCad's ngspice doesn't load on this system: install the system's own (libngspice), which Cuprita tries first"),
            failed.join("; ")
        ),
    })
}

fn bind(lib: Library, path: Option<PathBuf>) -> Result<Ngspice, String> {
    /// A function of `lib`, which outlives its pointer (`Ngspice` keeps both).
    fn get<T: Copy>(lib: &Library, name: &str) -> Result<T, String> {
        // SAFETY: each is called with the type ngspice's `sharedspice.h` declares for it.
        unsafe { lib.get::<T>(name.as_bytes()) }.map(|f| *f).map_err(|_| name.to_string())
    }
    Ok(Ngspice {
        init: get(&lib, "ngSpice_Init")?,
        command: get(&lib, "ngSpice_Command")?,
        circ: get(&lib, "ngSpice_Circ")?,
        all_plots: get(&lib, "ngSpice_AllPlots")?,
        all_vecs: get(&lib, "ngSpice_AllVecs")?,
        vec_info: get(&lib, "ngGet_Vec_Info")?,
        reset: get(&lib, "ngSpice_Reset").ok(),
        input_path: get(&lib, "ngCM_Input_Path").ok(),
        path,
        _lib: lib,
    })
}

/// Whether libngspice is there to simulate with (it loads, though it isn't started, or KiCad's
/// Flatpak has it to run in), else what to do about it.
pub fn found() -> Result<(), String> {
    load().map(drop).or_else(|e| if sandboxed() { Ok(()) } else { Err(e) })
}

/// Whether libngspice loads in this process: else a run needs Cuprita's own program (`inside`),
/// which a test's isn't.
pub fn loads() -> bool {
    load().is_ok()
}

/// Where libngspice may be, with whether it's a path (else a name the system's loader finds):
/// the system's first, then KiCad's own, as its installs bring it.
fn libraries() -> Vec<(PathBuf, bool)> {
    // By name only where the loader looks in the system's folders alone: Windows' and macOS'
    // also look where Cuprita was started, a project's folder, for a library someone put there.
    let names: &[&str] = if cfg!(any(windows, target_os = "macos")) { &[] } else { &["libngspice.so.0", "libngspice.so"] };
    let mut out: Vec<(PathBuf, bool)> = names.iter().map(|n| (PathBuf::from(n), false)).collect();
    // Homebrew's, on Apple silicon and Intel.
    out.extend(["/opt/homebrew/lib", "/usr/local/lib"].map(|l| (Path::new(l).join("libngspice.0.dylib"), true)));
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let flatpak = Path::new("flatpak/app/org.kicad.kicad/current/active/files");
    for root in [Path::new("/var/lib").join(flatpak), home.join(".local/share").join(flatpak)] {
        out.extend(["lib64", "lib"].map(|l| (root.join(l).join("libngspice.so.0"), true)));
    }
    let arch = format!("/snap/kicad/current/usr/lib/{}-linux-gnu/libngspice.so.0", std::env::consts::ARCH);
    out.push((arch.into(), true));
    out.push(("/Applications/KiCad/KiCad.app/Contents/Frameworks/libngspice.0.dylib".into(), true));
    if let Some(programs) = std::env::var_os("ProgramFiles") {
        let mut versions: Vec<PathBuf> =
            std::fs::read_dir(Path::new(&programs).join("KiCad")).into_iter().flatten().flatten().map(|e| e.path()).collect();
        let number = |p: &PathBuf| p.file_name().and_then(|n| n.to_str()?.parse::<f64>().ok()).unwrap_or(0.0);
        versions.sort_by(|a, b| number(b).total_cmp(&number(a)));
        out.extend(versions.into_iter().map(|v| (v.join("bin/ngspice.dll"), true)));
    }
    out.retain(|(p, path)| !path || p.is_file());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carries_a_run_over_a_pipe_as_it_was() {
        let v = |name: &str, unit, re: &[f64], im: &[f64]| Vector { name: name.into(), unit, re: re.to_vec(), im: im.to_vec() };
        let odd = v("net-(d1-a) x", "", &[0.1 + 0.2, f64::MIN_POSITIVE], &[-1e-300, f64::NAN]);
        let sim = Sim {
            plots: vec![
                Plot { name: "op1".into(), vectors: vec![v("/out", "V", &[1.5], &[]), v("v1#branch", "A", &[-5e-4], &[])] },
                Plot { name: "ac1".into(), vectors: vec![v("frequency", "Hz", &[1.0, 1e6], &[0.0, 0.0]), odd] },
            ],
            log: vec!["Warning: singular matrix".into()],
        };
        let sent = |sim: &Result<Sim, String>| {
            let mut out = b"Note: no spinit\nprogress 45.5\n".to_vec();
            write(sim, &mut out).unwrap();
            out
        };
        // After what ngspice printed itself and the lines of progress, every value comes back to
        // the bit (a NaN too, a byte of 10 among them), names with their spaces, units, the log.
        let bytes = sent(&Ok(sim));
        let back = read(&mut &bytes[..]).unwrap().unwrap();
        assert_eq!(sent(&Ok(back)), bytes);
        let back = read(&mut &bytes[..]).unwrap().unwrap();
        let last = &back.plots[1].vectors[1];
        assert_eq!(
            (back.plots[1].name.as_str(), last.name.as_str(), last.unit, back.plots[0].vectors[1].unit),
            ("ac1", "net-(d1-a) x", "", "A")
        );
        assert_eq!((last.re.as_slice(), last.im[0], back.log.len()), (&[0.1 + 0.2, f64::MIN_POSITIVE][..], -1e-300, 1));
        assert!(last.im[1].is_nan());
        // An error keeps its lines; nothing comes of no mark, or of results cut short.
        let e = read(&mut &sent(&Err("ngspice can't read the netlist: x\nline 2".into()))[..]).unwrap().unwrap_err();
        assert_eq!(e, "ngspice can't read the netlist: x\nline 2");
        assert!(read(&mut &b"plot op1\n"[..]).is_none() && read(&mut &bytes[..bytes.len() - 40]).is_none());
    }

    fn deck(text: &str) -> Deck {
        Deck { text: text.into(), behavior: "ltpsa", dir: std::env::temp_dir(), signals: Default::default(), traces: vec![] }
    }

    #[test]
    fn simulates_and_says_what_went_wrong() {
        let go = AtomicBool::new(false);
        let rc = deck(".title t\nV1 a 0 DC 2\nR1 a b 1k\nR2 b 0 3k\nC1 b 0 1u\n.op\n.tran 10u 5m uic\n.end\n");
        let sim = match simulate(&rc, &go) {
            Err(e) if !loads() => return eprintln!("{e}"),
            sim => sim.unwrap(),
        };
        let at = |plot: &str, v: &str| -> Vec<f64> {
            let p = sim.plots.iter().find(|p| p.name.starts_with(plot)).unwrap();
            p.vectors.iter().find(|x| x.name == v).unwrap().re.clone()
        };
        // The divider's 1.5 V, which the capacitor reaches in its 0.75 ms time constant.
        assert!((at("op", "b")[0] - 1.5).abs() < 1e-9);
        let (t, b) = (at("tran", "time"), at("tran", "b"));
        let tau = b.iter().zip(&t).find(|(v, _)| **v > 1.5 * (1.0 - (-1f64).exp())).unwrap().1;
        assert!((tau - 0.75e-3).abs() < 0.02e-3, "{tau}");
        // ngspice runs the operating point first; a transient's scale, time, comes first.
        let names: Vec<(&str, &str)> = sim.plots.iter().map(|p| (p.name.as_str(), p.vectors[0].unit)).collect();
        assert_eq!(names, [("op1", "V"), ("tran1", "s")]);
        // A netlist ngspice can't read says why, and the next run works again.
        let e = simulate(&deck(".title t\n.include \"/none.lib\"\n.op\n.end\n"), &go).unwrap_err();
        assert!(e.contains("ngspice can't read the netlist") && e.contains("/none.lib"), "{e}");
        let e = simulate(&deck(".title t\nR1 a 0 1k\n.end\n"), &go).unwrap_err();
        assert!(e.contains("give the schematic an analysis") || e.contains("No job"), "{e}");
        assert!(simulate(&rc, &go).is_ok());
        // Stopped, a run gives what it simulated by then.
        let long = deck(".title t\nV1 a 0 SIN(0 1 1k)\nR1 a b 1k\nC1 b 0 1u\n.tran 1n 1\n.end\n");
        let stop = AtomicBool::new(true);
        let sim = simulate(&long, &stop).unwrap();
        let t = &sim.plots[0].vectors[0].re;
        assert!(t.last().is_some_and(|&end| end < 1.0), "ran to {:?}", t.last());
        assert!(simulate(&rc, &go).is_ok());
    }
}
