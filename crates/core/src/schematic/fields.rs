//! A placed part's fields (Value, Footprint, a part number…): read from its first unit, and set
//! on every unit of it on every sheet, as patches that undo themselves.

use super::edit::{Patch, field_id};
use super::{Project, Symbol};
use crate::save::Edit;
use crate::sexpr::{Cursor, escape, unescape};
use crate::tr::{tr, trf};
use std::collections::HashSet;
use std::ops::Range;

/// A part's fields as its first unit has them, and its library symbol's footprint filters
/// (`ki_fp_filters`: globs, space separated; empty when it has none).
#[derive(Clone, Debug, PartialEq)]
pub struct PartFields {
    pub fields: Vec<(String, String)>,
    pub filters: String,
}

/// A `(property …)` of a symbol's text: its name and value, where its value (quotes included)
/// and the whole list are.
struct Property {
    name: String,
    value: String,
    at: Range<usize>,
    span: Range<usize>,
}

/// The properties of a symbol's text, `(symbol …)` as a sheet has it.
fn properties(text: &str) -> Vec<Property> {
    let c = &mut Cursor::new(text);
    let mut out = vec![];
    if c.open("symbol").is_err() {
        return out;
    }
    while let Ok(Some(head)) = c.child() {
        if head == "property" {
            let start = text[..c.span().start].rfind('(').unwrap_or(0);
            let Ok(name) = c.atom() else { break };
            // KiCad 9 on keeps a library's note on its symbols (`(property private "KLC_S4.1" "…")`):
            // no field of the part's, and no board gets it.
            let private = name == "private" && c.span().len() == name.len();
            let Ok(value) = c.atom() else { break };
            let at = c.span();
            if c.close().is_err() {
                break;
            }
            if private {
                continue;
            }
            out.push(Property { name: unescape(name), value: unescape(value), at, span: start..c.span().end });
        } else if c.close().is_err() {
            break;
        }
    }
    out
}

/// The library copy of symbol `lib_id` in a sheet's `lib_symbols`.
fn lib_copy<'a>(sheet: &'a str, lib_id: &str) -> Option<&'a str> {
    let c = &mut Cursor::new(sheet);
    c.open("kicad_sch").ok()?;
    while let Some(head) = c.child().ok()? {
        if head == "lib_symbols" {
            while let Some(head) = c.child().ok()? {
                let start = sheet[..c.span().start].rfind('(')?;
                let name = if head == "symbol" { c.atom().ok() } else { None };
                c.close().ok()?;
                if name.map(unescape).as_deref() == Some(lib_id) {
                    return Some(&sheet[start..c.span().end]);
                }
            }
            return None;
        }
        c.close().ok()?;
    }
    None
}

impl Project {
    /// The fields of part `designator` (`R5`), and its footprint filters.
    pub fn fields(&self, designator: &str) -> Option<PartFields> {
        let (page, s) = self.pages.iter().find_map(|p| Some((p, p.sheet.symbols.iter().find(|s| s.designator == designator)?)))?;
        let text = self.text(&page.file)?;
        let filters = lib_copy(text, &s.lib_id).and_then(|copy| crate::library::field(copy, "ki_fp_filters")).unwrap_or_default();
        Some(PartFields { fields: self.symbol_fields(&page.file, s), filters })
    }

    /// The fields of symbol `s` of sheet `file`: name and value, as its own text has them.
    pub(crate) fn symbol_fields(&self, file: &str, s: &Symbol) -> Vec<(String, String)> {
        let text = self.text(file).map_or("", |t| &t[s.span.clone()]);
        properties(text).into_iter().map(|p| (p.name, p.value)).collect()
    }

    /// Sets field `name` of part `designator` to `value` on every unit of it, on every sheet: the
    /// field's value where its symbol has the field, else a new field after its others, placed
    /// and hidden as its Footprint field (as KiCad's symbols hide theirs). KiCad 6 keeps each
    /// placement's Value and Footprint in the root sheet too: those follow. Returns the patch that
    /// undoes it.
    // shortcut: a new field copies the Footprint field's look, hidden in KiCad's libraries; hide it
    // itself when a library shows its Footprint field.
    pub fn set_field(&mut self, designator: &str, name: &str, value: &str) -> Result<Patch, String> {
        let quoted = format!("\"{}\"", escape(value));
        let mut files: Vec<(String, Vec<Edit>)> = vec![];
        let mut push = |file: &str, edit: Edit| match files.iter_mut().find(|(f, _)| f == file) {
            Some((_, edits)) => edits.push(edit),
            None => files.push((file.to_string(), vec![edit])),
        };
        // A sheet used twice holds one text for both places: each symbol is edited once.
        let mut done = HashSet::new();
        let mut links = vec![];
        for page in &self.pages {
            let Some(text) = self.text(&page.file) else { continue };
            for s in page.sheet.symbols.iter().filter(|s| s.designator == designator) {
                links.push(s.link.clone());
                if done.insert((page.file.clone(), s.span.start)) {
                    push(&page.file, property_edit(text, s.span.clone(), name, &quoted)?);
                }
            }
        }
        if links.is_empty() {
            return Err(trf("no part {0} on the sheets", &[&designator]));
        }
        let kind = match name {
            "Value" => Some("value"),
            "Footprint" => Some("footprint"),
            _ => None,
        };
        if let (Some(kind), Some(root)) = (kind, self.text(&self.root)) {
            for edit in instances(root, &links, kind, &quoted) {
                push(&self.root.clone(), edit);
            }
        }
        Ok(self.apply(Patch(files)))
    }
}

/// The edit setting field `name` of the symbol at `span` of `text` to `quoted`.
fn property_edit(text: &str, span: Range<usize>, name: &str, quoted: &str) -> Result<Edit, String> {
    let sym = &text[span.clone()];
    let props = properties(sym);
    let shift = |r: Range<usize>| span.start + r.start..span.start + r.end;
    if let Some(p) = props.iter().find(|p| p.name == name) {
        return Ok((shift(p.at.clone()), quoted.to_string()));
    }
    let model = props.iter().find(|p| p.name == "Footprint").or(props.last()).ok_or(tr("the symbol has no fields to place one by"))?;
    let last = props.last().expect("a model field is one of them");
    let mut rest = sym[model.at.end..model.span.end].to_string();
    // KiCad 6 and 7 number fields (`(id 2)`): a new one takes the next number.
    if let (Some(own), Some(top)) = (field_id(&rest), props.iter().filter_map(|p| field_id(&sym[p.span.clone()])).max()) {
        rest = rest.replacen(&format!("(id {own})"), &format!("(id {})", top + 1), 1);
    }
    let indent = &sym[sym[..last.span.start].rfind('\n').map_or(last.span.start, |n| n + 1)..last.span.start];
    let field = format!("\n{indent}(property \"{}\" {quoted}{rest}", escape(name));
    Ok((shift(last.span.end..last.span.end), field))
}

/// The edits setting `kind` (`value`, `footprint`) of the placements `links` in a KiCad 6 root's
/// `symbol_instances` to `quoted`.
fn instances(root: &str, links: &[String], kind: &str, quoted: &str) -> Vec<Edit> {
    let mut out = vec![];
    let c = &mut Cursor::new(root);
    if c.open("kicad_sch").is_err() {
        return out;
    }
    while let Ok(Some(head)) = c.child() {
        if head == "symbol_instances" {
            while let Ok(Some(_)) = c.child() {
                let mine = c.atom().is_ok_and(|path| links.iter().any(|l| *l == unescape(path)));
                while let Ok(Some(field)) = c.child() {
                    if mine && field == kind && c.atom().is_ok() {
                        out.push((c.span(), quoted.to_string()));
                    }
                    if c.close().is_err() {
                        return out;
                    }
                }
                if c.close().is_err() {
                    return out;
                }
            }
        }
        if c.close().is_err() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIB: &str = r#"(lib_symbols (symbol "Device:R" (property "Reference" "R" (at 0 0 0)) (property "ki_fp_filters" "R_* Resistor_*" (at 0 0 0))
        (symbol "R_1_1" (pin passive line (at 0 3.81 270) (length 1.27) (name "~" (effects (font (size 1.27 1.27)))) (number "1" (effects (font (size 1.27 1.27))))))))"#;

    fn project(sheet: &str) -> Project {
        let sheet = sheet.to_string();
        Project::load("a.kicad_sch", "", |f| if f == "a.kicad_sch" { Ok(sheet.clone()) } else { Err(f.into()) }).unwrap()
    }

    #[test]
    fn sets_a_field_and_adds_one_hidden_undoably() {
        let sheet = format!(
            "(kicad_sch (version 20260306) (uuid \"r\") {LIB}\n\t(symbol (lib_id \"Device:R\") (at 10 10 0) (unit 1) (uuid \"s1\")\n\t\t(property \"Reference\" \"R1\" (at 12 10 0))\n\t\t(property \"Value\" \"R\" (at 14 10 0))\n\t\t(property private \"KLC_S4.1\" \"a library's note\" (at 10 10 0))\n\t\t(property \"Footprint\" \"\" (at 10 10 0) (effects (font (size 1.27 1.27)) (hide yes)))\n\t)\n)"
        );
        let mut p = project(&sheet);
        let fields = p.fields("R1").unwrap();
        assert_eq!(fields.filters, "R_* Resistor_*");
        // KiCad 9's private fields are the library's notes, not the part's.
        assert_eq!((fields.fields[1].clone(), fields.fields.len()), (("Value".into(), "R".into()), 3));
        let undo_value = p.set_field("R1", "Value", "10k \"5%\"").unwrap();
        let undo_mpn = p.set_field("R1", "MPN", "RC0603FR-0710KL").unwrap();
        let text = p.text("a.kicad_sch").unwrap();
        assert!(text.contains("(property \"Value\" \"10k \\\"5%\\\"\" (at 14 10 0))"), "{text}");
        assert!(
            text.contains("\n\t\t(property \"MPN\" \"RC0603FR-0710KL\" (at 10 10 0) (effects (font (size 1.27 1.27)) (hide yes)))\n\t)"),
            "{text}"
        );
        assert_eq!(p.pages[0].sheet.symbols[0].value, "10k \"5%\"");
        assert_eq!(p.fields("R1").unwrap().fields.last(), Some(&("MPN".into(), "RC0603FR-0710KL".into())));
        p.apply(undo_mpn);
        p.apply(undo_value);
        assert_eq!(p.text("a.kicad_sch").unwrap(), sheet);
        assert!(p.set_field("R9", "Value", "1k").unwrap_err().contains("no part R9"));
    }

    #[test]
    fn sets_every_unit_and_a_kicad6_roots_placements() {
        // KiCad 6: references and each placement's value in the root's symbol_instances.
        let sheet = format!(
            r#"(kicad_sch (version 20211123) (uuid "r") {LIB}
  (symbol (lib_id "Device:R") (at 10 10 0) (unit 1) (uuid "u1") (property "Reference" "R1" (id 0) (at 0 0 0)) (property "Value" "R" (id 1) (at 0 0 0)) (property "Footprint" "" (id 2) (at 0 0 0)))
  (symbol (lib_id "Device:R") (at 20 10 0) (unit 2) (uuid "u2") (property "Reference" "R1" (id 0) (at 0 0 0)) (property "Value" "R" (id 1) (at 0 0 0)) (property "Footprint" "" (id 2) (at 0 0 0)))
  (symbol_instances (path "/u1" (reference "R1") (unit 1) (value "R") (footprint "")) (path "/u2" (reference "R1") (unit 2) (value "R") (footprint ""))))"#
        );
        let mut p = project(&sheet);
        p.set_field("R1", "Footprint", "Resistor_SMD:R_0603").unwrap();
        p.set_field("R1", "LCSC", "C23").unwrap();
        let text = p.text("a.kicad_sch").unwrap();
        assert_eq!(text.matches("(property \"Footprint\" \"Resistor_SMD:R_0603\" (id 2)").count(), 2);
        assert_eq!(text.matches("(footprint \"Resistor_SMD:R_0603\")").count(), 2);
        assert_eq!(text.matches("(property \"LCSC\" \"C23\" (id 3)").count(), 2);
    }
}
