//! Saving the texts of a part an edit placed, hid or showed again (`Board::tidy_texts`, a text
//! picked): one the board shows otherwise than the file and the part's placement put it gets its
//! place, angle, size and pen as KiCad writes them; one it hid, `(hide yes)` (KiCad 7 and
//! before: `hide`), which one it shows again loses (`tidy.md`).

use super::{At, Edit, KICAD8, angle_edit, quote, read_at, xy};
use crate::board::{Board, Footprint, MM, Nm, Pt, Text, place, rotate, upright};
use crate::sexpr::{Cursor, Result, escape};
use crate::units::nm_to_mm;
use std::ops::{Range, RangeInclusive};

/// A part's `fp_text` or `property` as the file has it: what saving compares and where it writes.
#[derive(Default)]
pub(super) struct Lists<'a> {
    pub(super) at: Option<At>,
    /// `(size H W)`: its width and height, and where both are written.
    size: Option<((Nm, Nm), Range<usize>)>,
    thickness: Option<(Nm, Range<usize>)>,
    /// Where `(font` and `(effects` end, after which a list they lack goes.
    font: Option<usize>,
    effects: Option<usize>,
    /// The end of its value and of its last list, after which an `(at …)` and an `(effects …)` it
    /// lacks go.
    value: usize,
    last: usize,
    /// The end of its `(layer …)`, with the space before that list; the value of a `(hide …)`,
    /// and what hides it in the file with the space before: a `(hide yes)`, or KiCad 7 and
    /// before's bare `hide`.
    layer: Option<(usize, &'a str)>,
    hide: Option<Range<usize>>,
    hides: Option<Range<usize>>,
    /// The layer's name; where `(justify …)` ends, and its `mirror`.
    on: Option<Range<usize>>,
    justify: Option<usize>,
    mirror: Option<Range<usize>>,
}

/// The lists of the text `c` is in, read past its value.
pub(super) fn read<'a>(c: &mut Cursor<'a>, src: &'a str) -> Result<Lists<'a>> {
    let mut l = Lists { value: c.span().end, last: c.span().end, ..Lists::default() };
    // With the space before it: what taking the flag or the list away takes.
    let spaced = |from: usize, to: usize| src[..from].trim_end().len()..to;
    loop {
        while let Some(flag) = c.opt_atom()? {
            if flag == "hide" {
                l.hides = Some(spaced(c.span().start, c.span().end));
            }
        }
        let Some(head) = c.child()? else { break };
        let open = src[..c.span().start].rfind('(').unwrap_or_default();
        match head {
            "at" => l.at = Some(read_at(c)?),
            "hide" => l.hide = Some(c.atom().map(|_| c.span())?),
            "layer" => l.on = Some(c.atom().map(|_| c.span())?),
            "effects" => {
                l.effects = Some(c.span().end);
                loop {
                    while let Some(flag) = c.opt_atom()? {
                        if flag == "hide" {
                            l.hides = Some(spaced(c.span().start, c.span().end));
                        }
                    }
                    let Some(head) = c.child()? else { break };
                    match head {
                        "font" => {
                            l.font = Some(c.span().end);
                            while let Some(head) = c.child()? {
                                match head {
                                    // Height first.
                                    "size" => {
                                        let (h, at) = (c.nm()?, c.span().start);
                                        l.size = Some(((c.nm()?, h), at..c.span().end));
                                    }
                                    "thickness" => l.thickness = Some((c.nm()?, c.span())),
                                    _ => {}
                                }
                                c.close()?;
                            }
                        }
                        "hide" => l.hide = Some(c.atom().map(|_| c.span())?),
                        "justify" => {
                            while let Some(word) = c.opt_atom()? {
                                if word == "mirror" {
                                    l.mirror = Some(c.span());
                                }
                            }
                        }
                        _ => {}
                    }
                    c.close()?;
                    if head == "justify" {
                        l.justify = Some(c.span().end);
                    }
                }
            }
            _ => {}
        }
        c.close()?;
        l.last = c.span().end;
        if head == "layer" {
            l.layer = Some((l.last, &src[src[..open].trim_end().len()..open]));
        }
    }
    if let Some(value) = l.hide.clone().filter(|v| &src[v.clone()] == "yes") {
        l.hides =
            Some(spaced(src[..value.start].rfind('(').unwrap_or_default(), value.end + src[value.end..].find(')').map_or(0, |i| i + 1)));
    }
    Ok(l)
}

/// The edits writing the texts of part `f` (its `lists` in the file, in order) that board `b`
/// shows otherwise than the file puts them with the part placed as the board has it (`flipped`
/// to the other side, turned `turn` degrees), and hiding those it hid; returns where it wrote
/// their places, which the part's turn and flip then leave.
pub(super) fn edits(
    b: &Board,
    f: &Footprint,
    lists: &[Lists],
    version: u32,
    (flipped, turn): (bool, f64),
    edits: &mut Vec<Edit>,
) -> Vec<RangeInclusive<usize>> {
    let mut own = vec![];
    for (k, l) in lists.iter().enumerate() {
        if f.hidden.iter().any(|t| t.src == Some(k)) {
            // A text shown has a layer: KiCad 8 on writes `(hide yes)` on a line after it.
            edits.extend(match (&l.hides, &l.hide, l.layer) {
                (Some(_), ..) => None,
                (None, Some(value), _) => Some((value.clone(), "yes".into())),
                (None, None, Some((end, space))) if version >= KICAD8 => Some((end..end, format!("{space}(hide yes)"))),
                (None, None, Some((end, _))) => Some((end..end, " hide".into())),
                (None, None, None) => None,
            });
            continue;
        }
        let Some(t) = b.texts[f.texts.clone()].iter().find(|t| t.src == Some(k)) else { continue };
        // One the file hides, shown again: its `(hide yes)` or `hide` goes.
        edits.extend(l.hides.clone().map(|hides| (hides, String::new())));
        // Where the file puts it, as the parser reads it: then flipped and turned with its part.
        let (local, deg) = l.at.as_ref().map_or((Pt::default(), 0.0), |a| (a.at, a.deg));
        let (local, deg) = if flipped { (Pt { x: local.x, y: -local.y }, 180.0 - deg) } else { (local, deg) };
        let (p, deg) = (place(local, f.at, f.angle), if t.upright { upright(deg + turn) } else { deg + turn });
        let off = (deg - t.angle).rem_euclid(360.0);
        if (p.x - t.at.x).abs() > 10 || (p.y - t.at.y).abs() > 10 || off.min(360.0 - off) > 1e-4 {
            let at = rotate(Pt { x: t.at.x - f.at.x, y: t.at.y - f.at.y }, -f.angle);
            let deg = ((t.angle * 1e6).round() / 1e6).rem_euclid(360.0);
            match &l.at {
                Some(a) => {
                    edits.push((a.xy.clone(), xy(at)));
                    // KiCad leaves a 0° out, but for texts from KiCad 8 on.
                    let zero = (deg * 1e6).round() == 0.0 && version < KICAD8;
                    edits.extend(match (zero, a.angle.is_empty()) {
                        (true, false) => Some((a.y.end..a.angle.end, String::new())),
                        (false, true) => Some((a.angle.clone(), format!(" {}", deg + 0.0))),
                        _ => angle_edit(a.angle.clone(), deg),
                    });
                    own.push(a.xy.start..=a.angle.end);
                }
                None => edits.push((l.value..l.value, format!(" (at {} {})", xy(at), deg + 0.0))),
            }
        }
        font(l, t, edits);
    }
    own
}

/// The edits giving the text whose lists are `l` the size and pen of `t`, where they differ.
fn font(l: &Lists, t: &Text, edits: &mut Vec<Edit>) {
    // As the parser reads them: 1 mm without a size, a pen of an eighth of its width without one.
    let size = l.size.as_ref().map_or((MM, MM), |s| s.0);
    let pen = l.thickness.as_ref().map(|t| t.0).filter(|&t| t > 0).unwrap_or(size.0 / 8);
    let (h_w, thick) = (format!("{} {}", nm_to_mm(t.size.1), nm_to_mm(t.size.0)), nm_to_mm(t.thickness));
    match (l.font, l.effects) {
        _ if t.size == size && t.thickness == pen => {}
        (Some(font), _) => {
            if t.size != size {
                edits.push(l.size.as_ref().map_or((font..font, format!(" (size {h_w})")), |s| (s.1.clone(), h_w)));
            }
            if t.thickness != pen {
                edits.push(l.thickness.as_ref().map_or((font..font, format!(" (thickness {thick})")), |p| (p.1.clone(), thick)));
            }
        }
        (None, Some(effects)) => edits.push((effects..effects, format!(" (font (size {h_w}) (thickness {thick}))"))),
        (None, None) => edits.push((l.last..l.last, format!(" (effects (font (size {h_w}) (thickness {thick})))"))),
    }
}

/// The edits writing the board's own text `t` over the file's, read as `file` (on the board's
/// layers) from `c`, at its `gr_text`: its place, angle, size and pen where they differ, its
/// layer and whether it is mirrored, and its words when they were typed. All else it has (its
/// id, its face) stays.
pub(super) fn own_edits<'a>(
    b: &Board,
    t: &Text,
    file: &Text,
    c: &mut Cursor<'a>,
    src: &'a str,
    version: u32,
    edits: &mut Vec<Edit>,
) -> Result<()> {
    let mut words = c.atom().map(|_| c.span())?;
    // KiCad 6 and 7's `locked` comes before them.
    if &src[words.clone()] == "locked" && c.opt_atom()?.is_some() {
        words = c.span();
    }
    let l = read(c, src)?;
    if t.typed {
        edits.push((words, quote(&escape(&t.text), version)));
    }
    if let (true, Some(on)) = (t.layer != file.layer, l.on.clone()) {
        edits.push((on, quote(&b.layers[t.layer as usize], version)));
    }
    match (t.mirror == file.mirror, &l.mirror, l.justify, l.effects) {
        (true, ..) => {}
        // With the space before it.
        (_, Some(word), ..) => edits.push((src[..word.start].trim_end().len()..word.end, String::new())),
        (_, None, Some(end), _) => edits.push((end - 1..end - 1, " mirror".into())),
        (_, None, None, Some(effects)) => edits.push((effects..effects, " (justify mirror)".into())),
        (_, None, None, None) => edits.push((l.last..l.last, " (effects (justify mirror))".into())),
    }
    let (deg, off) = (((t.angle * 1e6).round() / 1e6).rem_euclid(360.0), (t.angle - file.angle).rem_euclid(360.0));
    let turned = off.min(360.0 - off) > 1e-4;
    match &l.at {
        Some(a) => {
            if t.at != file.at {
                edits.push((a.xy.clone(), xy(t.at)));
            }
            if turned {
                edits.extend(angle_edit(a.angle.clone(), deg));
            }
        }
        None if t.at != file.at || turned => edits.push((l.value..l.value, format!(" (at {} {})", xy(t.at), deg + 0.0))),
        None => {}
    }
    font(&l, t, edits);
    Ok(())
}
