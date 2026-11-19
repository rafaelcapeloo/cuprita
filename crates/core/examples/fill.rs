//! Times zone filling and compares it with the fill in the file (save the board from KiCad
//! after its refill, skill `kicad-check`):
//! `cargo run --release -p cuprita-core --example fill -- board.kicad_pcb...`

use cuprita_core::board::{Board, ZoneFill};
use i_overlay::core::{fill_rule::FillRule, overlay::Overlay, overlay::ShapeType, overlay_rule::OverlayRule};
use i_overlay::i_float::int::point::IntPoint;
use i_overlay::i_shape::int::area::Area;
use std::time::Instant;

fn main() {
    for path in std::env::args().skip(1) {
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let file = Board::parse(&src).unwrap_or_else(|e| panic!("{path}: {e} at {:?}", e.line_col(&src)));
        let mut best = f64::MAX;
        let mut b = Board::parse(&src).unwrap();
        for _ in 0..3 {
            b = Board::parse(&src).unwrap();
            let t = Instant::now();
            b.fill_zones();
            best = best.min(t.elapsed().as_secs_f64());
        }
        // Per zone and layer: the file's area, ours, and the area only one of them covers.
        let (mut theirs, mut ours, mut off) = (0.0, 0.0, 0.0);
        let mut worst = vec![];
        for (i, (zf, zb)) in file.zones.iter().zip(&b.zones).enumerate() {
            for l in (0..128u8).filter(|&l| zb.layers & b.copper & 1 << l != 0) {
                let pick = |fills: &[ZoneFill]| -> Vec<Vec<IntPoint>> {
                    fills.iter().filter(|f| f.layer == l).map(|f| f.pts.iter().map(|p| IntPoint::new(p.x, p.y)).collect()).collect()
                };
                let (a, c) = (pick(&file.fills[zf.fills.clone()]), pick(&b.fills[zb.fills.clone()]));
                let area = |rule, a: &[Vec<IntPoint>], c: &[Vec<IntPoint>]| {
                    let mut o = Overlay::new(a.len() + c.len());
                    a.iter().for_each(|x| o.add_contour(x, ShapeType::Subject));
                    c.iter().for_each(|x| o.add_contour(x, ShapeType::Clip));
                    o.overlay(rule, FillRule::NonZero).iter().map(|s| s.area_two().unsigned_abs() as f64 / 2e12).sum::<f64>()
                };
                let (t, u, x) = (area(OverlayRule::Subject, &a, &[]), area(OverlayRule::Subject, &c, &[]), area(OverlayRule::Xor, &a, &c));
                (theirs, ours, off) = (theirs + t, ours + u, off + x);
                worst.push((x, i, b.layers[l as usize].clone(), b.nets[zb.net as usize].clone(), t, u));
            }
        }
        worst.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!(
            "{path}: {} zones filled in {:.1} ms | {} fills, {ours:.1} mm² (file {theirs:.1}), differ on {off:.2} mm² ({:.2}%)",
            b.zones.len(),
            best * 1e3,
            b.fills.len(),
            100.0 * off / theirs.max(1e-9)
        );
        for (x, i, layer, net, t, u) in worst.iter().take(8).filter(|w| w.0 > 0.01) {
            println!("  zone {i} {net} on {layer}: differ {x:.3} mm² (file {t:.3}, ours {u:.3})");
        }
    }
}
