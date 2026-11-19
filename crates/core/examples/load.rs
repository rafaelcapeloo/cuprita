//! Times board parsing: `cargo run --release -p cuprita-core --example load -- corpus/boards/*.kicad_pcb`;
//! a root `.kicad_sch` times reading its project's whole schematic (`corpus/boards/*/<name>.kicad_sch`)
//! and its ERC.

use cuprita_core::board::Board;
use cuprita_core::schematic::Project;
use std::path::Path;
use std::time::Instant;

fn main() {
    for path in std::env::args().skip(1) {
        if path.ends_with(".kicad_sch") {
            sheets(Path::new(&path));
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut best = f64::MAX;
        let mut board = None;
        for _ in 0..3 {
            let t = Instant::now();
            let parsed = Board::parse(&src);
            best = best.min(t.elapsed().as_secs_f64());
            board = Some(parsed.unwrap_or_else(|e| panic!("{path}: {e} at {:?}", e.line_col(&src))));
        }
        let b = board.unwrap();
        let mb = src.len() as f64 / 1e6;
        println!(
            "{path}: {mb:.1} MB in {:.1} ms ({:.0} MB/s) | v{} {} layers {} nets {} tracks {} vias {} pads {} shapes {} fills {} texts",
            best * 1e3,
            mb / best,
            b.version,
            b.layers.len(),
            b.nets.len(),
            b.tracks.len(),
            b.vias.len(),
            b.pads.len(),
            b.shapes.len(),
            b.fills.len(),
            b.texts.len()
        );
    }
}

/// Reads the schematic of root sheet `root` and checks it three times each (files read once),
/// best times.
fn sheets(root: &Path) {
    let dir = root.parent().unwrap_or(Path::new("."));
    let pro = std::fs::read_to_string(root.with_extension("kicad_pro")).unwrap_or_default();
    let files: std::collections::HashMap<String, String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .filter_map(|e| Some((e.file_name().into_string().ok()?, std::fs::read_to_string(e.path()).ok()?)))
        .collect();
    let name = root.file_name().unwrap().to_string_lossy();
    let mut best = f64::MAX;
    let mut project = None;
    for _ in 0..3 {
        let t = Instant::now();
        let p = Project::load(&name, &pro, |f| files.get(f).cloned().ok_or(format!("{f}: not in {}", dir.display())));
        best = best.min(t.elapsed().as_secs_f64());
        project = Some(p.unwrap_or_else(|e| panic!("{e}")));
    }
    let p = project.unwrap();
    let (mut erc, mut found) = (f64::MAX, 0);
    for _ in 0..3 {
        let t = Instant::now();
        found = p.erc().len();
        erc = erc.min(t.elapsed().as_secs_f64());
    }
    let count = |f: fn(&cuprita_core::schematic::Schematic) -> usize| p.pages.iter().map(|g| f(&g.sheet)).sum::<usize>();
    println!(
        "{}: {} pages in {:.1} ms, ERC {:.1} ms ({found} found) | {} symbols {} shapes {} texts | {} errors {:?}",
        root.display(),
        p.pages.len(),
        best * 1e3,
        erc * 1e3,
        count(|s| s.symbols.len()),
        count(|s| s.shapes.len()),
        count(|s| s.texts.len()),
        p.errors.len(),
        p.errors
    );
}
