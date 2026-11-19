//! Copy and paste on a sheet. `copy_items` gives what is picked as KiCad's own clipboard text:
//! the library symbols it uses, then each item as its sheet writes it, so KiCad pastes it too.
//! `paste` puts such a clip (ours, or what KiCad copied) on a page at a point of the grid: each
//! item moved there with new ids, parts numbered on from the project's highest (a part's units
//! together), the library symbols the sheet lacks added, and a junction wherever what lands
//! needs one. What it lands on it joins, as in KiCad: the user's own doing.

use super::edit::{Layout, Patch, items_end, junction_text, needs_junction, on_grid, reference, xy};
use super::net::on;
use super::placed::{entries, flag, wire};
use super::symbol::below_root;
use super::{Item, Project, Schematic};
use crate::board::Pt;
use crate::save::{Edit, read_at, uuid};
use crate::sexpr::{Cursor, Result, escape, unescape};
use crate::tr::{t, tr, trf};
use std::collections::HashMap;
use std::ops::Range;

/// What a clip may hold besides symbols: each a list of places, moved as one.
const PLAIN: [&str; 15] = [
    "wire",
    "bus",
    "bus_entry",
    "junction",
    "no_connect",
    "label",
    "global_label",
    "hierarchical_label",
    "text",
    "text_box",
    "polyline",
    "rectangle",
    "circle",
    "arc",
    "image",
];

impl Project {
    /// `items` of page `page` as a clip: the library symbols they use, each item's text, and the
    /// junctions the wires among them need.
    pub fn copy_items(&self, page: usize, items: &[Item]) -> std::result::Result<String, String> {
        let (file, src) = self.page_text(page)?;
        let (sheet, read) = (&self.pages[page].sheet, |e: crate::sexpr::Error| format!("{file}: {}", e.msg));
        let (w, entries, layout) = (&sheet.wiring, entries(src).map_err(read)?, Layout::read(src).map_err(read)?);
        let (mut libs, mut out): (Vec<&str>, Vec<&str>) = (vec![], vec![]);
        let (mut wires, mut ends): (Vec<(Pt, Pt)>, Vec<Pt>) = (vec![], vec![]);
        let lost = || trf("{0}: an item picked isn't in its text", &[&file]);
        for &it in items {
            match it {
                Item::Symbol(i) => {
                    let s = sheet.symbols.get(i).ok_or_else(lost)?;
                    if let Some((_, copy)) =
                        layout.copies.iter().find(|(name, _)| *name == s.lib_id).filter(|c| !libs.contains(&&src[c.1.clone()]))
                    {
                        libs.push(&src[copy.clone()]);
                    }
                    out.push(&src[s.span.clone()]);
                    ends.extend(w.pins.iter().filter(|q| q.symbol == i).map(|q| q.at));
                }
                Item::Wire(i) => {
                    let e = wire(&entries, i).ok_or_else(lost)?;
                    if !out.contains(&&src[e.span.clone()]) {
                        out.push(&src[e.span.clone()]);
                        wires.extend(e.points.windows(2).map(|p| (p[0].0, p[1].0)));
                    }
                }
                Item::Label(i) => out.push(&src[w.labels.get(i).ok_or_else(lost)?.span.clone()]),
                Item::NoConnect(i) => {
                    out.push(&src[flag(&entries, *w.no_connects.get(i).ok_or_else(lost)?).ok_or_else(lost)?.span.clone()])
                }
            }
        }
        if out.is_empty() {
            return Err(tr("nothing picked to copy: pick a symbol, wire, label or no-connect flag").into());
        }
        let joined =
            |e: &&super::placed::Entry| e.head == "junction" && e.points.first().is_some_and(|p| needs_junction(p.0, &wires, &ends));
        out.extend(entries.iter().filter(joined).map(|e| &src[e.span.clone()]));
        // A copy as its library writes it: a level less deep than a sheet keeps it.
        let libs: String = libs.iter().map(|l| format!("\n\t{}", l.replace("\n\t", "\n"))).collect();
        // As KiCad writes a selection: the list of library symbols only when it has any.
        let libs = if libs.is_empty() { libs } else { format!("(lib_symbols{libs}\n)\n") };
        Ok(format!("{libs}{}\n", out.join("\n")))
    }

    /// Pastes `clip` (what `copy_items` gives, or KiCad copied) on page `page`, its top left
    /// item on the grid point nearest `at`. Returns the patch that undoes it, and the symbols,
    /// wires, labels and flags pasted as the page has them.
    // shortcut: a sheet's box isn't pasted (its file would be shown twice: Sheet does that).
    pub fn paste(&mut self, page: usize, clip: &str, at: Pt) -> std::result::Result<(Patch, Vec<Item>), String> {
        let bad = |e: crate::sexpr::Error| trf("not a schematic clip: {0}", &[&e.msg]);
        let (file, src) = self.page_text(page)?;
        let Some(Ok(root)) = self.texts.get(&self.root) else { return Err(trf("{0} isn't read", &[&self.root])) };
        let (sheet, top) = (Layout::read(src).map_err(|e| format!("{file}: {}", e.msg))?, Layout::read(root).map_err(|e| e.msg)?);
        let wrapped = format!("(clip {clip})");
        let (libs, items) = lists(&wrapped).map_err(bad)?;
        let items: Vec<&(&str, Range<usize>)> = items.iter().filter(|(head, _)| *head == "symbol" || PLAIN.contains(head)).collect();
        // Where each item's places are; the clip's top left goes to `at`.
        let places: Vec<Vec<(Range<usize>, Pt)>> =
            items.iter().map(|(_, span)| places(&wrapped[span.clone()])).collect::<Result<_>>().map_err(bad)?;
        // A symbol's or a label's own place (its first; the rest are its fields'), every point of a line.
        let own = |k: usize| if matches!(items[k].0, "symbol" | "label" | "global_label" | "hierarchical_label") { 1 } else { usize::MAX };
        let first = |v: fn(&Pt) -> i32| places.iter().enumerate().flat_map(|(k, p)| p.iter().take(own(k))).map(|p| v(&p.1)).min();
        let (Some(x), Some(y)) = (first(|p| p.x), first(|p| p.y)) else {
            return Err(tr("the clip holds nothing to paste: copy symbols, wires or labels first").into());
        };
        let (to, seed) = (on_grid(at), (file, src.len(), at.x, at.y));
        let moved = |p: Pt| Pt { x: p.x - x + to.x, y: p.y - y + to.y };
        // Parts numbered on from the project's highest of their kind, in each place the sheet is
        // used; a part's units keep one number.
        let pages: Vec<usize> = (0..self.pages.len()).filter(|&i| self.pages[i].file == file).collect();
        let mut highest: HashMap<String, u32> = HashMap::new();
        let mut numbered: HashMap<String, Vec<String>> = HashMap::new();
        let (project, mut text, mut ids, mut rooted) = (self.root.trim_end_matches(".kicad_sch"), String::new(), vec![], String::new());
        let mut copies: Vec<Edit> = vec![];
        for (k, ((head, span), places)) in items.iter().zip(&places).enumerate() {
            let item = &wrapped[span.clone()];
            let mut edits: Vec<Edit> = places.iter().map(|(at, p)| (at.clone(), xy(moved(*p)))).collect();
            // Every id anew: the item's own, its pins'.
            let own = uuid((&seed, k));
            let fresh = fresh_ids(item, |n| if n == 0 { own.clone() } else { uuid((&seed, k, n)) }).map_err(bad)?;
            edits.extend(fresh);
            if *head == "symbol" {
                let s = Pasted::read(item).map_err(bad)?;
                let prefix = s.reference.trim_end_matches(|c: char| c.is_ascii_digit() || c == '?').to_string();
                let refs = numbered.entry(s.reference.clone()).or_insert_with(|| {
                    let n = highest.entry(prefix.clone()).or_insert_with(|| {
                        let used = self
                            .pages
                            .iter()
                            .flat_map(|p| &p.sheet.symbols)
                            .filter_map(|s| s.designator.strip_prefix(&prefix)?.parse().ok());
                        used.max().unwrap_or(0)
                    });
                    (pages.iter())
                        .map(|_| {
                            *n += 1;
                            reference(&prefix, *n)
                        })
                        .collect()
                });
                edits.push((s.value.clone(), format!("\"{}\"", escape(&refs[0]))));
                let paths = pages.iter().zip(refs.iter()).map(|(&i, r)| (self.pages[i].sheet.path.as_str(), r));
                match top.instances {
                    // A KiCad 6 root lists every part's places itself.
                    Some(_) => {
                        edits.extend(s.instances.clone().map(|list| (item[..list.start].trim_end().len()..list.end, String::new())));
                        for (path, r) in paths {
                            let entry = format!(
                                "(reference \"{r}\") (unit {}) (value {}) (footprint {})",
                                s.unit,
                                s.field("Value"),
                                s.field("Footprint")
                            );
                            rooted += &format!("\n    (path \"{}/{own}\"\n      {entry}\n    )", below_root(path));
                        }
                    }
                    None => {
                        let paths: String = paths
                            .map(|(path, r)| {
                                format!("\n\t\t\t\t(path \"{path}\"\n\t\t\t\t\t(reference \"{r}\")\n\t\t\t\t\t(unit {})\n\t\t\t\t)", s.unit)
                            })
                            .collect();
                        let list = format!("(instances\n\t\t\t(project \"{project}\"{paths}\n\t\t\t)\n\t\t)");
                        edits.push(match &s.instances {
                            Some(old) => (old.clone(), list),
                            None => (item.len() - 1..item.len() - 1, format!("\t{list}\n\t")),
                        });
                    }
                }
                // Its library symbol, where the sheet has none so named.
                let named = format!("\"{}\"", escape(&s.lib_id));
                if let Some(copy) = libs
                    .iter()
                    .find(|(name, _)| *name == s.lib_id)
                    .filter(|_| !copies.iter().any(|c| c.1.contains(&format!("(symbol {named}"))))
                {
                    copies.extend(sheet.copy(&named, &wrapped[copy.1.clone()]));
                }
                ids.push(own);
            }
            edits.sort_by_key(|e| e.0.start);
            text += &format!("\n\t{}", crate::save::apply(item, edits));
        }
        if text.is_empty() {
            return Err(tr("the clip holds nothing to paste: copy symbols, wires or labels first").into());
        }
        // What lands, read as a sheet of its own: its wires and its pins' ends, for the junctions
        // they need with what is there (a wire's end on a wire's middle, three ends).
        let lib: String = libs.iter().map(|(_, span)| &wrapped[span.clone()]).collect::<Vec<_>>().join(" ");
        let landed = Schematic::parse(&format!("(kicad_sch (lib_symbols {lib}) {text})"), "")
            .map_err(|e| trf("not a schematic clip: {0}", &[&e.msg]))?;
        let here = &self.pages[page].sheet;
        let wires: Vec<(Pt, Pt)> = here.wiring.wires.iter().chain(&landed.wiring.wires).copied().collect();
        let ends: Vec<Pt> = here.ends(|_, p| Some(p)).into_iter().chain(landed.wiring.pins.iter().map(|q| q.at)).collect();
        let inside = |p: Pt, &(a, b): &(Pt, Pt)| p != a && p != b && on(p, a, b);
        let old = here.wiring.wires.iter().flat_map(|&(a, b)| [a, b]).chain(here.ends(|_, p| Some(p)));
        let mut joins: Vec<Pt> =
            landed.wiring.wires.iter().flat_map(|&(a, b)| [a, b]).chain(landed.wiring.pins.iter().map(|q| q.at)).collect();
        joins.extend(old.filter(|&p| landed.wiring.wires.iter().any(|w| inside(p, w))));
        joins.sort_by_key(|p| (p.x, p.y));
        joins.dedup();
        joins.retain(|p| !here.wiring.junctions.contains(p) && !landed.wiring.junctions.contains(p) && needs_junction(*p, &wires, &ends));
        text.extend(joins.iter().enumerate().map(|(k, &p)| junction_text(p, &uuid((&seed, "junction", k)))));

        let end = items_end(src).map_err(|e| format!("{file}: {}", e.msg))?;
        // The library symbols go in before the items: what was there moves on by their length.
        let grown: usize = copies.iter().map(|e| e.1.len()).sum();
        copies.push((end..end, text));
        copies.sort_by_key(|e| e.0.start);
        let (file, mut patch) = (file.to_string(), Patch(vec![]));
        patch.0.push((file.clone(), copies));
        if let (Some(end), false) = (top.instances, rooted.is_empty()) {
            match patch.0.iter_mut().find(|(f, _)| *f == self.root) {
                Some((_, edits)) => edits.push((end..end, rooted)),
                None => patch.0.push((self.root.clone(), vec![(end..end, rooted)])),
            }
        }
        let undo = self.apply_readable(patch, t!("the clip wasn't pasted, as it would leave a sheet unreadable: {0}"))?;
        // What was pasted, as the page has it now: symbols by their ids, the rest by their places.
        let now = &self.pages[page].sheet;
        let mut pasted: Vec<Item> =
            (0..now.symbols.len()).filter(|&i| ids.iter().any(|id| now.symbols[i].link.ends_with(id.as_str()))).map(Item::Symbol).collect();
        let wire_at = |&(a, b): &(Pt, Pt)| now.wiring.wires.iter().rposition(|&w| w == (a, b));
        pasted.extend(landed.wiring.wires.iter().filter_map(wire_at).map(Item::Wire));
        let from = end + grown;
        pasted.extend((0..now.wiring.labels.len()).filter(|&i| now.wiring.labels[i].span.start >= from).map(Item::Label));
        let flag_at = |p: &Pt| now.wiring.no_connects.iter().position(|q| q == p);
        pasted.extend(landed.wiring.no_connects.iter().filter_map(flag_at).map(Item::NoConnect));
        Ok((undo, pasted))
    }
}

/// A clip's lists: its library symbols (name, where), and the rest (head, where).
type Lists<'a> = (Vec<(String, Range<usize>)>, Vec<(&'a str, Range<usize>)>);

fn lists(wrapped: &str) -> Result<Lists<'_>> {
    let (c, mut libs, mut items) = (&mut Cursor::new(wrapped), vec![], vec![]);
    c.open("clip")?;
    while let Some(head) = c.child()? {
        let start = c.span().start - 1;
        if head == "lib_symbols" {
            while let Some(h) = c.child()? {
                let from = c.span().start - 1;
                let name = if h == "symbol" { unescape(c.atom()?) } else { String::new() };
                c.close()?;
                libs.push((name, from..c.span().end));
            }
        }
        c.close()?;
        if head != "lib_symbols" {
            items.push((head, start..c.span().end));
        }
    }
    Ok((libs, items))
}

/// Every place in an item's text: where its X Y are, and the point; its own first.
fn places(item: &str) -> Result<Vec<(Range<usize>, Pt)>> {
    fn walk(c: &mut Cursor, out: &mut Vec<(Range<usize>, Pt)>) -> Result<()> {
        while let Some(head) = c.child()? {
            if matches!(head, "at" | "xy" | "start" | "end" | "mid" | "center") {
                let a = read_at(c)?;
                out.push((a.xy, a.at));
            } else {
                walk(c, out)?;
            }
            c.close()?;
        }
        Ok(())
    }
    let (c, mut out) = (&mut Cursor::new(item), vec![]);
    c.tok()?;
    c.atom()?;
    while c.opt_atom()?.is_some() {}
    walk(c, &mut out)?;
    Ok(out)
}

/// The edits giving each `(uuid …)` of an item's text the id `id(n)`, `n` counting from its own
/// (the first one level in).
fn fresh_ids(item: &str, id: impl Fn(usize) -> String) -> Result<Vec<Edit>> {
    fn walk(c: &mut Cursor, depth: usize, found: &mut Vec<(usize, Range<usize>)>) -> Result<()> {
        while let Some(head) = c.child()? {
            if head == "uuid" {
                c.atom()?;
                found.push((depth, c.span()));
            } else {
                walk(c, depth + 1, found)?;
            }
            c.close()?;
        }
        Ok(())
    }
    let (c, mut found) = (&mut Cursor::new(item), vec![]);
    c.tok()?;
    c.atom()?;
    while c.opt_atom()?.is_some() {}
    walk(c, 0, &mut found)?;
    // The item's own id is the one at its top; the others (pins') follow.
    let own = found.iter().position(|(depth, _)| *depth == 0);
    let mut n = 0;
    Ok((found.iter().enumerate())
        .map(|(k, (_, span))| {
            let which = if Some(k) == own {
                0
            } else {
                n += 1;
                n
            };
            (span.clone(), format!("\"{}\"", id(which)))
        })
        .collect())
}

/// What pasting a placed symbol takes from its text.
struct Pasted<'a> {
    lib_id: String,
    unit: u32,
    /// Its Reference field's value, and where it is (quoted).
    reference: String,
    value: Range<usize>,
    fields: Vec<(String, &'a str)>,
    instances: Option<Range<usize>>,
}

impl<'a> Pasted<'a> {
    fn read(item: &'a str) -> Result<Self> {
        let c = &mut Cursor::new(item);
        c.open("symbol")?;
        let mut s = Pasted { lib_id: String::new(), unit: 1, reference: String::new(), value: 0..0, fields: vec![], instances: None };
        while let Some(head) = c.child()? {
            let start = c.span().start - 1;
            match head {
                "lib_id" => s.lib_id = unescape(c.atom()?),
                "unit" => s.unit = c.atom()?.parse().unwrap_or(1),
                "property" => {
                    let name = unescape(c.atom()?);
                    let text = c.atom()?;
                    if name == "Reference" {
                        (s.reference, s.value) = (unescape(text), c.span());
                    }
                    s.fields.push((name, &item[c.span()]));
                }
                _ => {}
            }
            c.close()?;
            if head == "instances" {
                s.instances = Some(start..c.span().end);
            }
        }
        Ok(s)
    }

    /// A field's value as written (quoted), `""` without it.
    fn field(&self, name: &str) -> &str {
        self.fields.iter().find(|f| f.0 == name).map_or("\"\"", |f| f.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Nm;

    const LIB: &str = r#"(lib_symbols (symbol "Device:R" (property "Reference" "R" (at 2.032 0 90)) (property "Value" "R" (at 0 0 90))
        (symbol "R_1_1" (pin passive line (at 0 3.81 270) (length 1.27) (name "") (number "1"))
          (pin passive line (at 0 -3.81 90) (length 1.27) (name "") (number "2")))))"#;

    fn project(sheet: &str) -> Project {
        let sheet = sheet.to_string();
        Project::load("a.kicad_sch", "", |f| if f == "a.kicad_sch" { Ok(sheet.clone()) } else { Err(f.into()) }).unwrap()
    }

    fn at(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    #[test]
    fn copies_and_pastes_parts_with_their_wiring() {
        // R1 standing at (10.16, 10.16), its pin 1 wired right to a label A, a tee down from the
        // wire's middle with its junction, and a flag on pin 2.
        let src = format!(
            "(kicad_sch (version 20250114) (uuid \"r\") {LIB}
  (symbol (lib_id \"Device:R\") (at 10.16 10.16 0) (unit 1) (uuid \"u1\")
    (property \"Reference\" \"R1\" (at 12.192 10.16 90)) (property \"Value\" \"10k\" (at 10.16 10.16 90))
    (pin \"1\" (uuid \"p1\")) (pin \"2\" (uuid \"p2\"))
    (instances (project \"a\" (path \"/r\" (reference \"R1\") (unit 1)))))
  (wire (pts (xy 10.16 6.35) (xy 20.32 6.35)) (uuid \"w1\"))
  (wire (pts (xy 15.24 6.35) (xy 15.24 2.54)) (uuid \"w2\"))
  (junction (at 15.24 6.35) (diameter 0) (uuid \"j\"))
  (label \"A\" (at 20.32 6.35 0) (uuid \"l\"))
  (no_connect (at 10.16 13.97) (uuid \"n\"))
)"
        );
        let mut p = project(&src);
        let all = [Item::Symbol(0), Item::Wire(0), Item::Wire(1), Item::Label(0), Item::NoConnect(0)];
        let clip = p.copy_items(0, &all).unwrap();
        // KiCad's clipboard text: the library symbol, then the items, the wires' junction among them.
        assert!(clip.starts_with("(lib_symbols\n\t(symbol \"Device:R\"") && clip.contains("\n(junction (at 15.24 6.35)"), "{clip}");
        assert_eq!((clip.matches("(symbol (lib_id").count(), clip.matches("(wire").count(), clip.matches("(label").count()), (1, 2, 1));
        // Pasted 25.4 mm right (the corner of its places, left of the part and level with the
        // tee's end, on the point given): a second part numbered on, with ids of its own, the
        // same wiring beside the first.
        let (undo, pasted) = p.paste(0, &clip, at(35.56, 2.54)).unwrap();
        let t = p.text("a.kicad_sch").unwrap().to_string();
        let sheet = &p.pages[0].sheet;
        assert_eq!(sheet.symbols.iter().map(|s| s.reference.as_str()).collect::<Vec<_>>(), ["R1", "R2"]);
        assert!(
            t.contains("(at 35.56 10.16 0)")
                && t.contains("\"R2\" (at 37.592 10.16 90)")
                && t.contains("(path \"/r\"\n\t\t\t\t\t(reference \"R2\")"),
            "{t}"
        );
        assert_eq!((t.matches("(uuid \"u1\")").count(), t.matches("(uuid \"p1\")").count(), t.matches("(junction").count()), (1, 1, 2));
        assert_eq!(pasted, [Item::Symbol(1), Item::Wire(2), Item::Wire(3), Item::Label(1), Item::NoConnect(1)]);
        let nets = p.netlist();
        assert_eq!(nets.iter().find(|n| n.0 == "/A").map(|n| n.1.len()), Some(2), "{nets:?}");
        // Onto a wire's middle, what lands is joined there by a junction, as KiCad adds one.
        let tee = "(lib_symbols)\n(wire (pts (xy 0 0) (xy 0 5.08)) (uuid \"x\"))\n";
        p.paste(0, tee, at(12.7, 6.35)).unwrap();
        assert!(p.text("a.kicad_sch").unwrap().contains("(junction\n\t\t(at 12.7 6.35)"), "{}", p.text("a.kicad_sch").unwrap());
        // Undone, the sheet is as it was; a clip of nothing, or of no schematic, says so.
        let mut q = project(&src);
        let (undo2, _) = q.paste(0, &clip, at(50.8, 50.8)).unwrap();
        q.apply(undo2);
        assert_eq!(q.text("a.kicad_sch").unwrap(), src);
        assert!(q.paste(0, "(lib_symbols)", at(0.0, 0.0)).unwrap_err().starts_with("the clip holds nothing"));
        assert!(q.paste(0, "hello (", at(0.0, 0.0)).unwrap_err().starts_with("not a schematic clip"));
        assert!(q.copy_items(0, &[]).unwrap_err().starts_with("nothing picked"));
        drop(undo);
    }

    #[test]
    fn pastes_a_parts_units_as_one_part_and_into_a_sheet_without_its_symbol() {
        // Two units of U1 copied from one project into another's empty sheet (a KiCad 6 root,
        // which lists parts' places itself): one new part, U1 there, both units; its library
        // symbol goes in.
        let unit = |u: u32, x: f64| {
            format!(
                "(symbol (lib_id \"Amp:OPA\") (at {x} 10.16 0) (unit {u}) (uuid \"u{u}\") (property \"Reference\" \"U7\" (at {x} 7.62 0))
    (property \"Value\" \"OPA\" (at {x} 12.7 0)) (property \"Footprint\" \"SOIC\" (at {x} 15.24 0)))"
            )
        };
        let clip = format!(
            "(lib_symbols (symbol \"Amp:OPA\" (property \"Reference\" \"U\" (at 0 0 0))
  (symbol \"OPA_1_1\" (pin input line (at -5.08 0 0) (length 2.54) (name \"+\") (number \"3\")))
  (symbol \"OPA_2_1\" (pin input line (at -5.08 0 0) (length 2.54) (name \"+\") (number \"5\")))))\n{}\n{}\n",
            unit(1, 10.16),
            unit(2, 25.4)
        );
        let mut p = project("(kicad_sch (version 20211123) (uuid \"r\") (lib_symbols)\n  (symbol_instances)\n)");
        let (undo, pasted) = p.paste(0, &clip, at(50.8, 50.8)).unwrap();
        let t = p.text("a.kicad_sch").unwrap().to_string();
        assert_eq!(p.pages[0].sheet.symbols.iter().map(|s| s.reference.as_str()).collect::<Vec<_>>(), ["U1A", "U1B"], "{t}");
        assert!(
            t.contains("(lib_symbols\n\t\t(symbol \"Amp:OPA\"") && t.contains("(at 50.8 50.8 0)") && t.contains("(at 66.04 50.8 0)"),
            "{t}"
        );
        assert_eq!(
            (pasted.len(), t.matches("(reference \"U1\") (unit").count(), t.matches("(value \"OPA\") (footprint \"SOIC\")").count()),
            (2, 2, 2),
            "{t}"
        );
        // Pasted again: U2, both units.
        p.paste(0, &clip, at(50.8, 76.2)).unwrap();
        assert_eq!(p.pages[0].sheet.symbols.iter().map(|s| s.designator.as_str()).collect::<Vec<_>>(), ["U1", "U1", "U2", "U2"]);
        drop(undo);
    }
}
