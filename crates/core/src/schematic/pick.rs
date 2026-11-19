//! What a click picks on a sheet: a net label, a no-connect flag, a wire or a placed symbol.

use super::{CROSS, Schematic};
use crate::board::{Nm, Pt, crosses, nearest};
use crate::tr::tr;

/// An item of a sheet, by its place in the sheet's order: a placed symbol (`Schematic::symbols`),
/// a wire, a net label (local, global or hierarchical) or a no-connect flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    Symbol(usize),
    Wire(usize),
    Label(usize),
    NoConnect(usize),
}

impl Schematic {
    /// What a click at `p` picks, within `tol`: a label whose box holds it (the smallest), else a
    /// no-connect flag's cross, else the nearest wire, else the symbol `symbol_at` finds.
    pub fn item_at(&self, p: Pt, tol: Nm) -> Option<Item> {
        let w = &self.wiring;
        let near = |(lo, hi): (Pt, Pt)| (lo.x - tol..=hi.x + tol).contains(&p.x) && (lo.y - tol..=hi.y + tol).contains(&p.y);
        let area = |(lo, hi): (Pt, Pt)| (hi.x - lo.x) as i64 * (hi.y - lo.y) as i64;
        let label = (0..w.labels.len()).filter(|&i| near(w.labels[i].bounds)).min_by_key(|&i| area(w.labels[i].bounds));
        let flag = (0..w.no_connects.len()).find(|&i| near(self.extent(Item::NoConnect(i))));
        let d = |(a, b): (Pt, Pt)| {
            let q = nearest(a, b, p);
            (q.x as f64 - p.x as f64).hypot(q.y as f64 - p.y as f64)
        };
        let wire = (0..w.wires.len()).map(|i| (d(w.wires[i]), i)).filter(|&(d, _)| d <= tol as f64).min_by(|a, b| a.0.total_cmp(&b.0));
        let symbol = || self.symbol_at(p, tol).and_then(|s| self.symbols.iter().position(|o| std::ptr::eq(o, s)));
        let flag_or_wire = flag.map(Item::NoConnect).or(wire.map(|(_, i)| Item::Wire(i)));
        label.map(Item::Label).or(flag_or_wire).or_else(|| symbol().map(Item::Symbol))
    }

    /// The items a box from `a` to `b` holds whole, or (`touching`) also those it touches: what
    /// KiCad's selection box takes drawn left to right, or right to left.
    pub fn items_in(&self, a: Pt, b: Pt, touching: bool) -> Vec<Item> {
        let (lo, hi) = (Pt { x: a.x.min(b.x), y: a.y.min(b.y) }, Pt { x: a.x.max(b.x), y: a.y.max(b.y) });
        let holds = |p: Pt| (lo.x..=hi.x).contains(&p.x) && (lo.y..=hi.y).contains(&p.y);
        let meets = |(p, q): (Pt, Pt)| p.x.min(q.x) <= hi.x && p.x.max(q.x) >= lo.x && p.y.min(q.y) <= hi.y && p.y.max(q.y) >= lo.y;
        let takes = |item: Item| match (item, self.extent(item)) {
            (_, (p, q)) if !touching => holds(p) && holds(q),
            (Item::Wire(_), (p, q)) => crosses((lo, hi), p, q),
            (_, area) => meets(area),
        };
        let w = &self.wiring;
        let all = (0..self.symbols.len()).map(Item::Symbol).chain((0..w.wires.len()).map(Item::Wire));
        let all = all.chain((0..w.labels.len()).map(Item::Label)).chain((0..w.no_connects.len()).map(Item::NoConnect));
        all.filter(|&it| takes(it)).collect()
    }

    /// Where `item` is: a wire's two ends, else the corners of the box a symbol, label or
    /// no-connect flag's cross takes.
    pub fn extent(&self, item: Item) -> (Pt, Pt) {
        match item {
            Item::Symbol(i) => self.symbols[i].bounds,
            Item::Wire(i) => self.wiring.wires[i],
            Item::Label(i) => self.wiring.labels[i].bounds,
            Item::NoConnect(i) => {
                let p = self.wiring.no_connects[i];
                (Pt { x: p.x - CROSS, y: p.y - CROSS }, Pt { x: p.x + CROSS, y: p.y + CROSS })
            }
        }
    }

    /// What `item` is, in words (the language shown): `R5`, `Wire`, `Label SDA`, `Global label VCC`.
    pub fn describe(&self, item: Item) -> String {
        match item {
            Item::Symbol(i) => self.symbols[i].reference.clone(),
            Item::Wire(_) => tr("Wire").into(),
            Item::Label(i) => format!("{} {}", tr(self.wiring.labels[i].kind.name()), self.wiring.labels[i].name),
            Item::NoConnect(_) => tr("No-connect flag").into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_labels_then_wires_then_symbols_and_what_a_box_takes() {
        let src = r#"(kicad_sch (version 20250114) (uuid "r")
  (lib_symbols (symbol "Device:R" (property "Reference" "R" (at 2.032 0 90))
    (symbol "R_1_1" (rectangle (start -1.016 -2.54) (end 1.016 2.54)) (pin passive line (at 0 3.81 270) (length 1.27) (name "") (number "1"))
      (pin passive line (at 0 -3.81 90) (length 1.27) (name "") (number "2")))))
  (symbol (lib_id "Device:R") (at 10.16 10.16 0) (unit 1) (uuid "u1") (property "Reference" "R1" (at 12.192 10.16 90)))
  (wire (pts (xy 10.16 6.35) (xy 20.32 6.35)) (uuid "w"))
  (wire (pts (xy 30 30) (xy 40 40)) (uuid "s"))
  (label "SDA" (at 15.24 6.35 0) (effects (font (size 1.27 1.27)) (justify left bottom)) (uuid "l"))
)"#;
        let s = Schematic::parse(src, "").unwrap();
        let pt = |x: f64, y: f64| Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm };
        let at = |x, y| s.item_at(pt(x, y), 100_000);
        // The label's text above the wire, and where it joins it; the wire; the resistor; nothing.
        let picked = [at(16.5, 5.5), at(16.5, 6.35), at(12.0, 6.35), at(10.16, 10.16), at(30.0, 20.0)];
        assert_eq!(picked, [Some(Item::Label(0)), Some(Item::Label(0)), Some(Item::Wire(0)), Some(Item::Symbol(0)), None]);
        assert_eq!((s.describe(Item::Label(0)), s.describe(Item::Wire(1))), ("Label SDA".into(), "Wire".into()));
        // Drawn left to right, a box takes what it holds whole; right to left, what it touches.
        let (a, b) = (pt(9.0, 5.0), pt(21.0, 7.0));
        let taken = (s.items_in(a, b, false), s.items_in(b, a, true));
        assert_eq!(taken, (vec![Item::Wire(0)], vec![Item::Symbol(0), Item::Wire(0), Item::Label(0)]));
        // A slanting wire touches a box its line crosses, not one beside it.
        let (beside, across) = (s.items_in(pt(36.0, 30.0), pt(39.0, 32.0), true), s.items_in(pt(34.0, 33.0), pt(36.0, 36.0), true));
        assert_eq!((beside, across), (vec![], vec![Item::Wire(1)]));
    }
}
