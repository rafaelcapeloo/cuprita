//! Prints a project's SPICE netlist, as Cuprita simulates it, and how long writing it took; with
//! `--run`, simulates it in ngspice and prints its plots instead: `cargo run --release -p
//! cuprita-core --example spice -- [--run] <root>.kicad_sch…` (skill `spice-check`).

use cuprita_core::ngspice;
use cuprita_core::schematic::Project;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn main() {
    let run = std::env::args().any(|a| a == "--run");
    for path in std::env::args().skip(1).filter(|a| a != "--run") {
        let root = Path::new(&path);
        let dir = root.parent().unwrap_or(Path::new("."));
        let pro = std::fs::read_to_string(root.with_extension("kicad_pro")).unwrap_or_default();
        let name = root.file_name().unwrap().to_string_lossy();
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).map_err(|e| format!("{f}: {e}"));
        let project = Project::load(&name, &pro, read).unwrap_or_else(|e| panic!("{e}"));
        let t = Instant::now();
        let deck = project.spice(dir);
        eprintln!("{path}: written in {:.2} ms", t.elapsed().as_secs_f64() * 1e3);
        let deck = match deck {
            Ok(deck) => deck,
            Err(e) => {
                eprintln!("{e}");
                continue;
            }
        };
        if !run {
            print!("{}", deck.text);
            continue;
        }
        let t = Instant::now();
        match ngspice::simulate(&deck, &AtomicBool::new(false)) {
            Ok(sim) => {
                println!("{path}: simulated in {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
                for p in &sim.plots {
                    let points = p.vectors.first().map_or(0, |v| v.re.len());
                    println!("  {}: {} vectors of {points} points", p.name, p.vectors.len());
                }
                sim.log.iter().for_each(|l| println!("  ngspice: {l}"));
            }
            Err(e) => println!("{path}: {e}"),
        }
    }
}
