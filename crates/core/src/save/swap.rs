//! Saving a part F8 swapped for the footprint its symbol names (`Src::Swapped`): the file's text
//! of it keeps its own lists (where and on which side it is, its link, fields, texts as placed,
//! attributes) and takes the library's pads, graphics, zones and 3D models in place of the rest
//! (`crates/core/update-parts.md`).

use super::{apply, footprint, read_at};
use crate::board::{Board, Footprint, Pt};
use crate::sexpr::{Cursor, Result};

/// The lists that stay the part's own; a library's `version` and `generator` go too.
// shortcut: the part keeps its `attr`, its SMD or through-hole type too; take the type from the
// library when a swap changes it (no corpus part's would).
const OWN: [&str; 13] =
    ["layer", "uuid", "tstamp", "tedit", "at", "path", "sheetname", "sheetfile", "property", "fp_text", "attr", "locked", "placed"];

/// The file's footprint text `old` of part `f` with the lists of library footprint `lib` (its
/// text at the origin, as `Src::Swapped` keeps it) in place of its own but `OWN`: the library's
/// linked as it, placed where and on the side the file has the part, after the part's own lists,
/// spaced as they are; saving then places the whole as the board has the part.
pub(super) fn merged(b: &Board, version: u32, old: &str, lib: &str, f: &Footprint) -> Result<String> {
    let (c, mut edits) = (&mut Cursor::new(old), vec![]);
    c.child()?;
    c.atom()?;
    let (name, mut end, mut space) = (c.span(), c.span().end, None);
    let (mut at, mut deg, mut back) = (Pt::default(), 0.0, false);
    while let Some(head) = c.child()? {
        let open = old[..c.span().start].rfind('(').unwrap_or_default();
        let gap = old[..open].trim_end().len();
        match head {
            "at" => (at, deg) = read_at(c).map(|a| (a.at, a.deg))?,
            "layer" => back = b.on_back(c.atom()?),
            _ => {}
        }
        c.close()?;
        if !OWN.contains(&head) {
            edits.push((gap..c.span().end, String::new()));
        }
        // The space before its last list: a line of its own (KiCad 6 starts on the head's line).
        (end, space) = (c.span().end, Some(&old[gap..open]));
    }
    // The library's lists, placed where the file has the part, a level in as the file's are.
    // Its texts are the file's: the library's go.
    let frame = Footprint { at, angle: deg.rem_euclid(360.0), back, texts: 0..0, hidden: vec![], ..f.clone() };
    let mut placing = vec![];
    let lc = &mut Cursor::new(lib);
    lc.child()?;
    footprint(b, version, &mut lc.clone(), lib, &frame, &mut placing)?;
    let placed = apply(lib, placing);
    let (lc, space) = (&mut Cursor::new(&placed), space.unwrap_or(" "));
    lc.child()?;
    lc.atom()?;
    edits.push((name, placed[lc.span()].to_string()));
    let mut theirs = String::new();
    while let Some(head) = lc.child()? {
        let open = placed[..lc.span().start].rfind('(').unwrap_or_default();
        lc.close()?;
        if !OWN.contains(&head) && !matches!(head, "version" | "generator" | "generator_version") {
            theirs += space;
            theirs += &placed[open..lc.span().end].replace("\n\t", space);
        }
    }
    edits.push((end..end, theirs));
    Ok(apply(old, edits))
}
