//! Saving a part's value and fields, which F8 carries from the schematic (`update.rs`): those
//! the file has another value of are rewritten, the ones it lacks added as its KiCad version adds
//! them (`crates/core/update.md`).

use super::{Edit, KICAD8, quote, uuid};
use crate::board::Footprint;
use crate::sexpr::{escape, unescape};
use std::ops::Range;

/// A footprint's `(fp_text KIND "…")` or `(property "NAME" "…")`: its kind or name as written,
/// its value as written and where that is.
pub(super) struct Field<'a> {
    pub(super) property: bool,
    pub(super) name: &'a str,
    pub(super) raw: &'a str,
    pub(super) value: Range<usize>,
}

/// The edits giving the file's footprint the value and fields of `f`: those of `fields` written
/// otherwise rewritten; those it lacks added after `last` (the end of its last field or `(at …)`,
/// with the space before that list), as KiCad of file `version` adds a field: up to KiCad 7
/// `(property "MPN" "…")`, from KiCad 8 hidden on the part's side's Fab layer at its anchor.
pub(super) fn edits(f: &Footprint, fields: &[Field], last: (usize, &str), version: u32, edits: &mut Vec<Edit>) {
    let text = |s: &str| quote(&escape(s), version);
    for field in fields {
        let want = match (field.property, field.name) {
            (false, "value") | (true, "Value") => Some(&f.value),
            (true, name) if name != "Reference" => f.fields.iter().find(|(k, _)| *k == unescape(name)).map(|(_, v)| v),
            _ => None,
        };
        if let Some(want) = want.filter(|want| **want != unescape(field.raw)) {
            edits.push((field.value.clone(), text(want)));
        }
    }
    let (at, space) = last;
    for (name, value) in f.fields.iter().filter(|(name, _)| !fields.iter().any(|g| g.property && unescape(g.name) == *name)) {
        let head = format!("(property {} {}", text(name), text(value));
        if version < KICAD8 {
            edits.push((at..at, format!("{space}{head})")));
            continue;
        }
        let i = format!("{space}\t");
        let (side, mirror) = if f.back { ("B", format!("{i}\t(justify mirror)")) } else { ("F", String::new()) };
        let (angle, id) = ((f.angle * 1e6).round() / 1e6 + 0.0, uuid((&f.reference, name, value)));
        edits.push((
            at..at,
            format!(
                "{space}{head}{i}(at 0 0 {angle}){i}(unlocked yes){i}(layer \"{side}.Fab\"){i}(hide yes){i}(uuid \"{id}\"){i}(effects{i}\t(font{i}\t\t(size 1 1){i}\t\t(thickness 0.15){i}\t){mirror}{i}){space})"
            ),
        ));
    }
}
