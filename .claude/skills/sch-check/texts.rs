//! `src/main.rs` of a scratch cargo project (`cuprita-core = { path = "<repo>/crates/core" }`):
//! `texts <root.kicad_sch> <out dir>` reads the root's whole schematic and writes each page's
//! texts to `<out>/<root>[-<sheet>…].txt`, as KiCad names its SVG plots: per line the ink, the
//! pen, the box of its strokes (mm) and the text.

use cuprita_core::font;
use cuprita_core::schematic::{Ink, Project};
use std::io::Write;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, root, out] = &args[..] else { panic!("usage: texts <root.kicad_sch> <out dir>") };
    let (root, out) = (Path::new(root), Path::new(out));
    let dir = root.parent().unwrap_or(Path::new("."));
    let pro = std::fs::read_to_string(root.with_extension("kicad_pro")).unwrap_or_default();
    let name = root.file_name().unwrap().to_string_lossy();
    let p = Project::load(&name, &pro, |f| std::fs::read_to_string(dir.join(f)).map_err(|e| format!("{f}: {e}"))).unwrap();
    eprintln!("{} pages; errors {:?}", p.pages.len(), p.errors);
    std::fs::create_dir_all(out).unwrap();
    let stem = name.trim_end_matches(".kicad_sch");
    let mm = |v: i32| v as f64 / 1e6;
    for page in &p.pages {
        // KiCad names the plots by the sheet names trimmed (`I2C ` plots as `…-I2C.svg`).
        let file = std::iter::once(stem).chain(page.path.iter().map(|s| s.trim())).collect::<Vec<_>>().join("-");
        let mut f = std::fs::File::create(out.join(format!("{}.txt", file.replace('/', "_")))).unwrap();
        for t in &page.sheet.texts {
            let (mut lo, mut hi) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
            font::strokes(t, |pts| {
                for p in pts {
                    (lo, hi) = ((lo.0.min(p.x), lo.1.min(p.y)), (hi.0.max(p.x), hi.1.max(p.y)));
                }
            });
            if lo.0 <= hi.0 {
                let ink = format!("{:?}", Ink::ALL[t.layer as usize]);
                writeln!(f, "{ink} {:.4} {:.4} {:.4} {:.4} {:.4} {:?}", mm(t.thickness), mm(lo.0), mm(lo.1), mm(hi.0), mm(hi.1), t.text)
                    .unwrap();
            }
        }
    }
}
