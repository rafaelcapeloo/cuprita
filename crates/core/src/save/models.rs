//! Saving a footprint's 3D models, which Inspect sets (`Board::set_models`): its shown models in
//! the file give way to the new ones, written as KiCad of the file's version writes them; hidden
//! ones stay as they are.

use super::{Edit, KICAD6, KICAD8, apply};
use crate::board::Model;
use crate::parse::footprint::model;
use crate::sexpr::{Cursor, Result, escape};

/// The edits giving the footprint whose children cursor `c` is at (in `src`, of file `version`)
/// the shown 3D models `models`; none when it has them already.
pub(super) fn edits(mut c: Cursor, src: &str, models: &[Model], version: u32) -> Result<Vec<Edit>> {
    let (mut shown, mut spans, mut last, mut space) = (vec![], vec![], c.span().end, "\n  ");
    while let Some(head) = c.child()? {
        // The list with the space before it, which a model removed takes along.
        let open = src[..c.span().start].rfind('(').unwrap_or_default();
        let before = src[..open].trim_end().len();
        space = &src[before..open];
        let found = if head == "model" { model(&mut c)? } else { None };
        c.close()?;
        last = c.span().end;
        if let Some(m) = found {
            shown.push(m);
            spans.push(before..last);
        }
    }
    if shown == models {
        return Ok(vec![]);
    }
    let text: String = models.iter().map(|m| written(m, space, version)).collect();
    // The new ones go where the first shown one was, else after the footprint's last list.
    let mut out: Vec<Edit> = spans.iter().skip(1).map(|s| (s.clone(), String::new())).collect();
    out.push(spans.first().map_or((last..last, text.clone()), |first| (first.clone(), text)));
    Ok(out)
}

/// Library footprint `text` (one added, or one F8 swapped in) with the shown 3D models `models`,
/// written as its own KiCad version writes them; as it is when it doesn't read.
pub(crate) fn rewritten(text: &str, models: &[Model]) -> String {
    // KiCad 5's footprints name no version: theirs place a model `at` inches.
    let version = text.split("(version ").nth(1).and_then(|v| v.split(')').next()?.trim().parse().ok()).unwrap_or(0);
    let edits = || -> Result<Vec<Edit>> {
        let mut c = Cursor::new(text);
        c.child()?;
        edits(c, text, models, version)
    };
    edits().map_or_else(|_| text.to_string(), |e| apply(text, e))
}

/// Model `m` as KiCad of file `version` writes it, after `space` (the space before its
/// footprint's lists, whose indent its own lists take one step further).
fn written(m: &Model, space: &str, version: u32) -> String {
    let unit = if space.contains('\t') { "\t" } else { "  " };
    let (inner, deep) = (format!("{space}{unit}"), format!("{space}{unit}{unit}"));
    let xyz = |v: [f64; 3]| format!("(xyz {} {} {})", num(v[0]), num(v[1]), num(v[2]));
    // KiCad 5 placed a model `at` inches, KiCad 6 on `offset` millimetres.
    let (at, offset) = if version >= KICAD6 { ("offset", m.offset) } else { ("at", m.offset.map(|mm| mm / 25.4)) };
    let file = escape(&m.file);
    let (offset, scale, rotate) = (xyz(offset), xyz(m.scale), xyz(m.rotate));
    if version >= KICAD8 {
        // KiCad 8 on: each list on lines of its own.
        let list = |head: &str, v: &str| format!("{inner}({head}{deep}{v}{inner})");
        format!("{space}(model \"{file}\"{}{}{}{space})", list(at, &offset), list("scale", &scale), list("rotate", &rotate))
    } else {
        format!("{space}(model \"{file}\"{inner}({at} {offset}){inner}(scale {scale}){inner}(rotate {rotate}){space})")
    }
}

/// `v` with at most six decimals, no trailing zeros: `0`, `1.5`, `-90`.
fn num(v: f64) -> String {
    let s = format!("{:.6}", v + 0.0);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}
