//! `src/main.rs` of a scratch cargo project (`cuprita-core = { path = "<repo>/crates/core" }`):
//! `check <root.kicad_sch> <out dir>` reads the root's whole project and writes Cuprita's nets to
//! `<out>/nets.tsv` (`name<TAB>ref-pin ref-pin…`) and its ERC to `<out>/erc.tsv` (`kind<TAB>error
//! or warning<TAB>/sheet<TAB>x,y mm<TAB>what<TAB>fix`), for `kicad.py nets` and `kicad.py erc`.

use cuprita_core::schematic::Project;
use std::io::Write;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, root, out] = &args[..] else { panic!("usage: check <root.kicad_sch> <out dir>") };
    let (root, out) = (Path::new(root), Path::new(out));
    let dir = root.parent().unwrap_or(Path::new("."));
    let pro = std::fs::read_to_string(root.with_extension("kicad_pro")).unwrap_or_default();
    let name = root.file_name().unwrap().to_string_lossy();
    let p = Project::load(&name, &pro, |f| std::fs::read_to_string(dir.join(f)).map_err(|e| format!("{f}: {e}"))).unwrap();
    std::fs::create_dir_all(out).unwrap();
    let t = std::time::Instant::now();
    let mut nets = std::fs::File::create(out.join("nets.tsv")).unwrap();
    for (name, pins) in p.netlist() {
        writeln!(nets, "{name}\t{}", pins.iter().map(|(r, n)| format!("{r}-{n}")).collect::<Vec<_>>().join(" ")).unwrap();
    }
    let vs = p.erc();
    eprintln!("{} pages, nets and ERC in {:.1?}; errors {:?}", p.pages.len(), t.elapsed(), p.errors);
    let mut erc = std::fs::File::create(out.join("erc.tsv")).unwrap();
    for v in vs {
        let (kind, sheet) = (if v.error { "error" } else { "warning" }, p.pages[v.page].path.join("/"));
        let (x, y) = (v.at.x as f64 / 1e6, v.at.y as f64 / 1e6);
        writeln!(erc, "{}\t{kind}\t/{sheet}\t{x:.2},{y:.2}\t{}\t{}", v.kind, v.what, v.fix).unwrap();
    }
}
