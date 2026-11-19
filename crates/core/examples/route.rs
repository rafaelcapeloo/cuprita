//! Times routing: `cargo run --release -p cuprita-core --example route -- board.kicad_pcb
//! [board.kicad_pro]` searches paths between pads of one net on the top layer, 3 to 40 mm apart:
//! walking round other nets' copper, then as a shoving route step does (the search through what a
//! shove moves, the shove, and the walk round when it can't make room), and short steps beside them.

use cuprita_core::board::{Board, Nm, Pt};
use cuprita_core::route::Router;
use cuprita_core::shove::Shover;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: route board.kicad_pcb [board.kicad_pro]");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut b = Board::parse(&src).unwrap_or_else(|e| panic!("{path}: {e} at {:?}", e.line_col(&src)));
    if let Some(pro) = args.next() {
        b.apply_pro(&std::fs::read_to_string(&pro).unwrap_or_else(|e| panic!("{pro}: {e}")));
    }
    let (top, width) = (b.stackup[0].layer, b.rules.class_track_width);
    let (mut ms, mut shoving, mut near, mut built) = (vec![], vec![], vec![], vec![]);
    let (mut reached, mut shoved, mut moved, mut beside) = (0, 0, 0, 0);
    for p in b.pads.iter().step_by(7).filter(|p| p.net != 0 && p.layers & 1 << top != 0) {
        let far = |q: &&cuprita_core::board::Pad| (3e6..40e6).contains(&((q.at.x - p.at.x) as f64).hypot((q.at.y - p.at.y) as f64));
        let Some(q) = b.pads.iter().filter(|q| q.net == p.net && q.layers & 1 << top != 0).find(far) else { continue };
        let t = Instant::now();
        let (mut r, shover) = (Router::new(&b, p.net, top, width, p.at), Shover::new(&b, p.net));
        built.push(t.elapsed().as_secs_f64() * 1e3);
        let (to, t) = (r.snap(q.at, 0), Instant::now());
        let found = r.path(p.at, to);
        ms.push(t.elapsed().as_secs_f64() * 1e3);
        reached += usize::from(found.last() == Some(&to));
        // A fresh router, so the step pays for the cells it reads as the app's first step does.
        let mut r = Router::new(&b, p.net, top, width, p.at);
        let t = Instant::now();
        if let (_, Ok(s)) = shover.step(&b, &mut r, (p.at, to), width, top) {
            (shoved, moved) = (shoved + 1, moved + s.runs.len() + s.vias.len());
        }
        shoving.push(t.elapsed().as_secs_f64() * 1e3);
        // Short steps beside the pad, each way, where shoving makes way most.
        for (dx, dy) in [(1.0, 0.0), (0.7, 0.7), (0.0, 1.0), (-0.7, 0.7), (-1.0, 0.0), (-0.7, -0.7), (0.0, -1.0), (0.7, -0.7)] {
            let aim = Pt { x: p.at.x + (dx * 1.5e6) as Nm, y: p.at.y + (dy * 1.5e6) as Nm };
            let mut r = Router::new(&b, p.net, top, width, p.at);
            let (to, t) = (r.snap(aim, 0), Instant::now());
            if let (_, Ok(s)) = shover.step(&b, &mut r, (p.at, to), width, top) {
                beside += usize::from(s != Default::default());
            }
            near.push(t.elapsed().as_secs_f64() * 1e3);
        }
        if ms.len() == 25 {
            break;
        }
    }
    let stats = |v: &[f64]| (v.iter().sum::<f64>() / v.len().max(1) as f64, v.iter().copied().fold(0.0, f64::max));
    let ((mean, worst), (smean, sworst), (nmean, nworst), (bmean, _)) = (stats(&ms), stats(&shoving), stats(&near), stats(&built));
    println!("{path}: {} paths ({reached} reached), search mean {mean:.1} ms, worst {worst:.1} ms", ms.len());
    println!("  shoving them: step mean {smean:.1} ms, worst {sworst:.1} ms ({shoved} made way, moving {moved}; the rest walked round)");
    println!("  1.5 mm steps beside the pads: mean {nmean:.1} ms, worst {nworst:.1} ms ({} of {} moved others)", beside, near.len());
    println!("  router and shover built in {bmean:.1} ms");
}
