//! Buses added to a project's sheets: the bus itself, as a wire is (`add_line`), and its entries.
//! Labels name a bus as they name a wire (`add_label`), its members by KiCad's syntax (`nets.md`).

use super::edit::{GRID, Patch, items_end, junction_text, needs_junction, on_grid, quarter, xy};
use super::net::on;
use super::{Project, Schematic};
use crate::board::Pt;
use crate::save::uuid;
use crate::tr::t;
use crate::units::nm_to_mm;

impl Project {
    /// Adds a bus along `pts` (each taken to its nearest grid point) to page `page`, as KiCad 10
    /// writes one: a segment from each point to the next, a junction where a bus's end lies on a
    /// bus's middle or three bus ends meet. Returns the patch that undoes it.
    pub fn add_bus(&mut self, page: usize, pts: &[Pt]) -> Result<Patch, String> {
        self.add_line(page, pts, true)
    }

    /// Adds a bus entry (KiCad's wire to bus entry) to page `page`: from the grid point nearest
    /// `at`, 100 mil right and 100 mil down, turned `angle` degrees (0, 90, 180 or 270)
    /// counterclockwise, as KiCad 10 writes one; with a junction where an end lies on a wire's
    /// middle. Returns the patch that undoes it.
    pub fn add_bus_entry(&mut self, page: usize, at: Pt, angle: f64) -> Result<Patch, String> {
        quarter(angle, t!("angle {0}: a bus entry turns by 0, 90, 180 or 270 degrees"))?;
        let (x, y) = match angle.rem_euclid(360.0) as i32 {
            0 => (1, 1),
            90 => (1, -1),
            180 => (-1, -1),
            _ => (-1, 1),
        };
        let (file, src) = self.page_text(page)?;
        let end = items_end(src).map_err(|e| format!("{file}: {}", e.msg))?;
        let (at, s, size) = (on_grid(at), &self.pages[page].sheet, Pt { x: x * 2 * GRID, y: y * 2 * GRID });
        let other = Pt { x: at.x + size.x, y: at.y + size.y };
        let id = uuid((file, at, other, src.len(), "bus_entry"));
        let mut text = format!(
            "\n\t(bus_entry\n\t\t(at {})\n\t\t(size {} {})\n\t\t(stroke\n\t\t\t(width 0)\n\t\t\t(type default)\n\t\t)\n\t\t(uuid \"{id}\")\n\t)",
            xy(at),
            nm_to_mm(size.x),
            nm_to_mm(size.y)
        );
        let ends: Vec<Pt> = s.ends(|_, p| Some(p)).into_iter().chain([at, other]).collect();
        for p in [at, other].into_iter().filter(|p| !s.wiring.junctions.contains(p) && needs_junction(*p, &s.wiring.wires, &ends)) {
            text += &junction_text(p, &uuid((&id, p)));
        }
        let patch = Patch(vec![(file.to_string(), vec![(end..end, text)])]);
        self.apply_readable(patch, t!("the bus entry wasn't added, as it would leave a sheet unreadable: {0}"))
    }
}

impl Schematic {
    /// Whether a bus ending at `p` joins something there: a bus, a bus entry's end, a label or a
    /// sheet pin.
    pub fn joins_bus(&self, p: Pt) -> bool {
        let w = &self.wiring;
        w.buses.iter().any(|&(a, b)| on(p, a, b))
            || w.entries.iter().any(|&(a, b)| a == p || b == p)
            || w.labels.iter().any(|l| l.at == p)
            || self.sheets.iter().any(|s| s.pins.iter().any(|q| q.1 == p))
    }
}

#[cfg(test)]
mod tests {
    use super::super::LabelKind;
    use super::super::net::tests::{part, project};
    use crate::board::{Nm, Pt};

    fn at(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    #[test]
    fn adds_buses_and_entries_whose_labels_join_their_members() {
        // Outputs U1 and U2 left of where the buses go, inputs U3 and U4 below them.
        let parts = [("OUT", "U1", 0.0, 10.16), ("OUT", "U2", 0.0, 20.32), ("IN", "U3", 27.94, 25.4), ("IN", "U4", 35.56, 25.4)];
        let parts: String = parts.iter().map(|&(lib, r, x, y)| part(lib, r, "", x, y)).collect();
        let mut p = project(&[("root.kicad_sch", parts)], "");
        let before = p.text("root.kicad_sch").unwrap().to_string();
        let label = |p: &mut super::super::Project, name: &str, x, y| p.add_label(0, name, LabelKind::Local, "", at(x, y), 0.0).unwrap();
        // A bus down, and one off it in a T: a junction at the T.
        let mut undo = vec![p.add_bus(0, &[at(20.32, 0.0), at(20.32, 30.48)]).unwrap()];
        undo.push(p.add_bus(0, &[at(20.32, 15.24), at(40.64, 15.24)]).unwrap());
        let t = p.text("root.kicad_sch").unwrap();
        assert!(t.contains("\n\t(bus\n\t\t(pts\n\t\t\t(xy 20.32 0) (xy 20.32 30.48)\n\t\t)\n\t\t(stroke\n\t\t\t(width 0)\n\t\t\t(type default)\n\t\t)\n\t\t(uuid \""), "{t}");
        assert_eq!(p.pages[0].sheet.wiring.junctions, [at(20.32, 15.24)]);
        // Entries to wires: D0 and D1 from the outputs to the first bus, E0 and E1 from the second
        // to the inputs; the buses named D[0..1] and E[0..1] join them member by member.
        for (k, y) in [10.16, 20.32].into_iter().enumerate() {
            undo.push(p.add_wire(0, &[at(0.0, y), at(17.78, y)]).unwrap());
            undo.push(p.add_bus_entry(0, at(17.78, y), 0.0).unwrap());
            undo.push(label(&mut p, &format!("D{k}"), 10.16, y));
            let x = 25.4 + 7.62 * k as f64;
            undo.push(p.add_bus_entry(0, at(x, 15.24), 0.0).unwrap());
            undo.push(p.add_wire(0, &[at(x + 2.54, 17.78), at(x + 2.54, 25.4)]).unwrap());
            undo.push(label(&mut p, &format!("E{k}"), x + 2.54, 20.32));
        }
        undo.push(label(&mut p, "D[0..1]", 20.32, 5.08));
        undo.push(label(&mut p, "E[0..1]", 38.1, 15.24));
        assert!(p.text("root.kicad_sch").unwrap().contains("\n\t(bus_entry\n\t\t(at 17.78 10.16)\n\t\t(size 2.54 2.54)\n\t\t(stroke"));
        let nets: Vec<_> = p.netlist().into_iter().filter(|(n, _)| n.starts_with("/D")).collect();
        let pins = |a: &str, b: &str| vec![(a.to_string(), "1".to_string()), (b.to_string(), "1".to_string())];
        assert_eq!(nets, [("/D0".to_string(), pins("U1", "U3")), ("/D1".to_string(), pins("U2", "U4"))]);
        // Turned, an entry goes the other ways; a bus ends where it joins a bus, an entry, a label.
        undo.push(p.add_bus_entry(0, at(50.8, 50.8), 90.0).unwrap());
        assert!(p.text("root.kicad_sch").unwrap().contains("(at 50.8 50.8)\n\t\t(size 2.54 -2.54)"));
        let s = &p.pages[0].sheet;
        assert!(
            s.joins_bus(at(20.32, 25.4)) && s.joins_bus(at(20.32, 12.7)) && s.joins_bus(at(38.1, 15.24)) && !s.joins_bus(at(25.4, 0.0))
        );
        // Mistakes say what is wrong; undone, the sheet is as it was.
        assert!(p.add_bus(0, &[at(1.0, 1.0)]).unwrap_err().starts_with("a bus needs two points"));
        assert!(p.add_bus_entry(0, at(0.0, 0.0), 45.0).unwrap_err().contains("a bus entry turns by 0, 90, 180 or 270"));
        for u in undo.into_iter().rev() {
            p.apply(u);
        }
        assert_eq!(p.text("root.kicad_sch").unwrap(), before);
    }
}
