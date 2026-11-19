//! Fabrication outputs, named and written as KiCad's (JLCPCB, PCBWay and OSH Park read those as
//! they are): Gerber X2 layers, Excellon drill files and pick-and-place positions.

use crate::board::{Board, LayerId, Nm, Pt, place};
use crate::bom::ref_key;
use std::collections::BTreeMap;
use std::fmt::Write;

impl Board {
    /// The files a fab makes the board from, as (name, text): Gerber X2 copper, solder mask,
    /// paste, silkscreen and outline layers named `<stem>-<layer>.<Protel extension>`, and
    /// Excellon drill files (`drills`).
    pub fn fab_files(&self, stem: &str) -> Vec<(String, String)> {
        let n = self.stackup.len();
        let mut layers: Vec<(LayerId, String, String)> = (self.stackup.iter().enumerate())
            .map(|(i, c)| {
                let (ext, side) = match i {
                    0 => ("gtl".into(), "Top"),
                    _ if i + 1 == n => ("gbl".into(), "Bot"),
                    _ => (format!("g{i}"), "Inr"),
                };
                (c.layer, ext, format!("Copper,L{},{side}", i + 1))
            })
            .collect();
        let others = [
            ("F.Mask", "gts", "Soldermask,Top"),
            ("B.Mask", "gbs", "Soldermask,Bot"),
            ("F.Paste", "gtp", "Paste,Top"),
            ("B.Paste", "gbp", "Paste,Bot"),
            ("F.SilkS", "gto", "Legend,Top"),
            ("B.SilkS", "gbo", "Legend,Bot"),
            ("Edge.Cuts", "gm1", "Profile,NP"),
        ];
        for (name, ext, function) in others {
            if let Some(l) = self.layers.iter().position(|l| l == name) {
                layers.push((l as LayerId, ext.into(), function.into()));
            }
        }
        let mut files: Vec<(String, String)> = (layers.into_iter())
            .map(|(l, ext, function)| (format!("{stem}-{}.{ext}", self.layers[l as usize].replace('.', "_")), self.gerber(l, &function)))
            .collect();
        files.extend(self.drills(stem));
        files
    }

    /// Excellon drill files, as KiCad names them: the plated holes through the board (vias,
    /// plated pads) in `<stem>-PTH.drl`, each span of blind or buried vias in its own
    /// (`<stem>-front-in1.drl`), the unplated holes in `<stem>-NPTH.drl`.
    fn drills(&self, stem: &str) -> Vec<(String, String)> {
        let n = self.stackup.len();
        let through = (0, n.saturating_sub(1));
        // Holes (size, ends) by the first and last copper layer they span, in stack order.
        let (mut plated, mut unplated) = (BTreeMap::<(usize, usize), Vec<Hole>>::new(), vec![]);
        for v in &self.vias {
            let mut span = (self.stackup.iter().enumerate()).filter(|(_, c)| v.layers & 1 << c.layer != 0).map(|(i, _)| i);
            let span = span.next().map_or(through, |top| (top, span.next_back().unwrap_or(top)));
            plated.entry(span).or_default().push((v.drill, v.at, v.at));
        }
        for p in &self.pads {
            let Some((w, h)) = p.drill else { continue };
            // A slot runs between the centres of its round ends.
            let d = w.min(h);
            let end = Pt { x: (w - d) / 2, y: (h - d) / 2 };
            let hole = (d, place(Pt { x: -end.x, y: -end.y }, p.at, p.angle), place(end, p.at, p.angle));
            if p.layers & self.copper != 0 { plated.entry(through).or_default().push(hole) } else { unplated.push(hole) }
        }
        let side = |i: usize| match i {
            0 => "front".to_string(),
            _ if i + 1 == n => "back".into(),
            _ => format!("in{i}"),
        };
        let mut files =
            vec![(format!("{stem}-PTH.drl"), excellon(plated.remove(&through).unwrap_or_default(), &format!("Plated,1,{n},PTH")))];
        for ((a, b), holes) in plated {
            let kind = if a == 0 || b + 1 == n { "Blind" } else { "Buried" };
            files.push((format!("{stem}-{}-{}.drl", side(a), side(b)), excellon(holes, &format!("Plated,{},{},{kind}", a + 1, b + 1))));
        }
        files.push((format!("{stem}-NPTH.drl"), excellon(unplated, &format!("NonPlated,1,{n},NPTH"))));
        files
    }

    /// Where a pick-and-place machine puts each part it places (`Footprint::placed`), as CSV in
    /// JLCPCB's columns (PCBWay reads them too): reference, anchor in mm with Y up (the Gerbers'
    /// axes), side, and angle in degrees as KiCad writes it, in (-180, 180].
    pub fn positions_csv(&self) -> String {
        let mut parts: Vec<_> = self.footprints.iter().filter(|f| f.placed).collect();
        parts.sort_by_key(|f| ref_key(&f.reference));
        let mut out = String::from("Designator,Mid X,Mid Y,Layer,Rotation\n");
        for f in parts {
            let side = if f.back { "Bottom" } else { "Top" };
            let angle = if f.angle > 180.0 { f.angle - 360.0 } else { f.angle };
            let _ = writeln!(out, "{},{},{},{side},{angle}", csv(&f.reference), mm(f.at.x), mm(-f.at.y));
        }
        out
    }
}

/// A hole: its size, and the centres of its ends (the same for a round one).
type Hole = (Nm, Pt, Pt);

/// An Excellon drill file of `holes` with X2 file function `function`: in mm, a tool per size
/// from the smallest, oval holes as G85 slots (KiCad's alternate drill mode, which JLCPCB asks for).
fn excellon(mut holes: Vec<Hole>, function: &str) -> String {
    holes.sort_by_key(|h| h.0);
    let mut out = format!(
        "M48\n; DRILL file Cuprita {v}\n; FORMAT={{-:-/ absolute / metric / decimal}}\n; #@! TF.GenerationSoftware,Cuprita,Cuprita,{v}\n\
         ; #@! TF.FileFunction,{function}\nFMAT,2\nMETRIC\n",
        v = env!("CARGO_PKG_VERSION")
    );
    let mut sizes: Vec<Nm> = holes.iter().map(|h| h.0).collect();
    sizes.dedup();
    for (t, d) in sizes.iter().enumerate() {
        let _ = writeln!(out, "T{}C{:.3}", t + 1, *d as f64 / 1e6);
    }
    out += "%\nG90\nG05\n";
    let xy = |p: Pt| format!("X{:.3}Y{:.3}", p.x as f64 / 1e6, -(p.y as f64) / 1e6);
    for (i, (d, a, b)) in holes.iter().enumerate() {
        if i == 0 || holes[i - 1].0 != *d {
            let _ = writeln!(out, "T{}", sizes.iter().position(|s| s == d).unwrap_or(0) + 1);
        }
        let _ = if a == b { writeln!(out, "{}", xy(*a)) } else { writeln!(out, "{}G85{}", xy(*a), xy(*b)) };
    }
    out + "M30\n"
}

/// `nm` in mm, as few digits as say it exactly.
fn mm(nm: Nm) -> String {
    let s = format!("{:.6}", nm as f64 / 1e6);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `s` as a CSV field: quoted when it holds a comma, quote or line break.
pub(crate) fn csv(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use crate::board::Board;

    #[test]
    fn names_files_as_kicad_and_drills_holes_and_slots() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (31 "B.Cu" signal)
                (38 "B.Mask" user) (39 "F.Mask" user) (37 "F.SilkS" user) (44 "Edge.Cuts" user))
              (footprint "J" (layer "F.Cu") (at 40 10)
                (pad "1" thru_hole oval (at 4 0 90) (size 2 1.4) (drill oval 1.2 0.6) (layers "*.Cu" "*.Mask"))
                (pad "" np_thru_hole circle (at 10 0) (size 3 3) (drill 3) (layers "*.Cu" "*.Mask")))
              (via (at 10 15) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu"))
              (via blind (at 12 15) (size 0.5) (drill 0.2) (layers "F.Cu" "In1.Cu")))"#,
        )
        .unwrap();
        let files = b.fab_files("b");
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "b-F_Cu.gtl",
                "b-In1_Cu.g1",
                "b-In2_Cu.g2",
                "b-B_Cu.gbl",
                "b-F_Mask.gts",
                "b-B_Mask.gbs",
                "b-F_SilkS.gto",
                "b-Edge_Cuts.gm1",
                "b-PTH.drl",
                "b-front-in1.drl",
                "b-NPTH.drl"
            ]
        );
        assert!(files[1].1.contains("%TF.FileFunction,Copper,L2,Inr*%") && files[3].1.contains("%TF.FileFunction,Copper,L4,Bot*%"));
        // Tools from the smallest; the slot runs between its round ends (turned with its pad).
        let body = |t: &str| t.lines().filter(|l| !l.starts_with(';')).collect::<Vec<_>>().join("\n");
        assert_eq!(
            body(&files[8].1),
            "M48\nFMAT,2\nMETRIC\nT1C0.300\nT2C0.600\n%\nG90\nG05\nT1\nX10.000Y-15.000\nT2\nX44.000Y-10.300G85X44.000Y-9.700\nM30"
        );
        assert!(files[8].1.contains("; #@! TF.FileFunction,Plated,1,4,PTH\n"));
        // A blind via's span has its own file.
        assert!(files[9].1.contains("; #@! TF.FileFunction,Plated,1,2,Blind\n"));
        assert!(body(&files[9].1).contains("T1C0.200\n%\nG90\nG05\nT1\nX12.000Y-15.000\n"));
        assert_eq!(body(&files[10].1), "M48\nFMAT,2\nMETRIC\nT1C3.000\n%\nG90\nG05\nT1\nX50.000Y-10.000\nM30");
    }

    #[test]
    fn lists_placed_parts_in_jlcpcb_columns() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
              (footprint "R" (layer "F.Cu") (at 10 20 90) (property "Reference" "R10"))
              (footprint "C" (layer "B.Cu") (at 5.5 7.25 270) (property "Reference" "C1"))
              (footprint "R" (layer "F.Cu") (at 1 2) (property "Reference" "R2"))
              (footprint "H" (layer "F.Cu") (at 0 0) (property "Reference" "H1") (attr exclude_from_pos_files))
              (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "R3") (attr smd dnp)))"#,
        )
        .unwrap();
        let rows = "Designator,Mid X,Mid Y,Layer,Rotation\nC1,5.5,-7.25,Bottom,-90\nR2,1,-2,Top,0\nR10,10,-20,Top,90\n";
        assert_eq!(b.positions_csv(), rows);
        assert_eq!([super::csv("R1"), super::csv("1u, 10V"), super::csv("a\"b")], ["R1", "\"1u, 10V\"", "\"a\"\"b\""]);
    }
}
