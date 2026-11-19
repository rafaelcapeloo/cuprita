//! Renders a board or a schematic sheet to PNG without a window, or a board's 3D view with `--3d`:
//! `cargo run --release -p cuprita-render --example png -- board.kicad_pcb out.png [width] [--3d]`

use cuprita_core::board::Board;
use cuprita_core::models;
use cuprita_core::schematic::Project;
use cuprita_render::{Orbit, build, build_sch, render_rgba, render_rgba3d};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output, ..] = &args[..] else { panic!("usage: png <board.kicad_pcb | sheet.kicad_sch> <out.png> [width]") };
    let width: u32 = args.get(3).filter(|a| *a != "--3d").map_or(2048, |w| w.parse().expect("width"));
    let three = args.iter().any(|a| a == "--3d");
    let src = std::fs::read_to_string(input).expect("read board");
    let t = Instant::now();
    let (board, sheet) = if input.ends_with(".kicad_sch") {
        // The project's drawing settings, from any `.kicad_pro` beside the sheet.
        let dir = std::path::Path::new(input).parent().map_or(".".into(), |d| d.join("."));
        let pro = std::fs::read_dir(&dir).into_iter().flatten().flatten().find(|e| e.path().extension().is_some_and(|x| x == "kicad_pro"));
        let pro = pro.and_then(|e| std::fs::read_to_string(e.path()).ok()).unwrap_or_default();
        // Its root page, with the sheets under it for the page count its title block shows.
        let name = std::path::Path::new(input).file_name().and_then(|n| n.to_str()).expect("file name");
        let read =
            |f: &str| if f == name { Ok(src.clone()) } else { std::fs::read_to_string(dir.join(f)).map_err(|e| format!("{f}: {e}")) };
        (Board::default(), Some(Project::load(name, &pro, read).unwrap_or_else(|e| panic!("{e}"))))
    } else {
        (Board::parse(&src).unwrap_or_else(|e| panic!("{e} at {:?}", e.line_col(&src))), None)
    };
    let parse = t.elapsed();
    let t = Instant::now();
    let scene = sheet.as_ref().map_or_else(|| build(&board), |p| build_sch(p, 0));
    let scene_time = t.elapsed();
    let t = Instant::now();
    let visible: Vec<bool> = scene.layers.iter().map(|l| l.visible).collect();
    let (w, h, pixels) = if three {
        let t = Instant::now();
        let models = models::load(&models::files(&board), std::path::Path::new(input).parent());
        let read = models.values().filter(|m| m.is_ok()).count();
        println!("{read} of {} 3D models read in {:.1?}", models.len(), t.elapsed());
        for e in models.values().filter_map(|m| m.as_ref().err()).take(5) {
            println!("  {e}");
        }
        let size = [width, width * 3 / 4];
        (size[0], size[1], render_rgba3d(&scene, &board, &models, &Orbit::fit(scene.bounds, size.map(|s| s as f32)), size))
    } else {
        render_rgba(&scene, scene.bounds, width, &visible, 0, false)
    };
    let gpu = t.elapsed();
    let file = std::io::BufWriter::new(std::fs::File::create(output).expect("create png"));
    let mut png = png::Encoder::new(file, w, h);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header().and_then(|mut wr| wr.write_image_data(&pixels)).expect("write png");
    let tris = scene.indices.len() / 3;
    println!(
        "parse {parse:.1?} | scene {scene_time:.1?} ({} instances, {tris} triangles, {} fill failures) | gpu with device setup {gpu:.1?}",
        scene.insts.len(),
        scene.tess_failures
    );
}
