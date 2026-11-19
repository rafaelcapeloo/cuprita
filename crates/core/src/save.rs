//! Saving: a board back to the KiCad text it was read from. Unedited text stays byte for byte;
//! moved footprints get their new placement, footprints, tracks, vias, drawings and texts the
//! board no longer has are removed, the tracks and vias it gained are written after the file's
//! last track, its drawings and texts after the last of those and its zones after the last zone,
//! its own texts edited get their new place, size, layer and words where they are, and refilled
//! zones get their new fills, the way the file's KiCad version writes them.

use crate::board::{Board, Footprint, Geom, LayerId, LayerMask, NetId, Pt, Shape, Span, Src, Text, Track, Via, arc_center, place};
use crate::parse::Parser;
use crate::sexpr::{Cursor, Result, escape};
use crate::tr::{t, tr};
use crate::units::nm_to_mm;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

mod fields;
mod flip;
pub(crate) mod models;
mod swap;
mod texts;
mod zones;

/// File versions of KiCad 6.0, which quotes every string and gives tracks a `tstamp` id, and of
/// KiCad 8.0, which writes each child on its own line and calls the id `uuid`.
pub(crate) const KICAD6: u32 = 20211014;
pub(crate) const KICAD8: u32 = 20240108;

const MISMATCH: &str = t!("the board was not read from this text");

/// Text replacing a byte range of the source; empty ranges insert.
pub(crate) type Edit = (Range<usize>, String);

impl Board {
    /// The board as KiCad text, given `src`, the text it was read from: unedited text stays byte
    /// for byte, moved footprints get their new placement (pads, texts and zones turned along),
    /// footprints, tracks and vias the board no longer has are removed, the tracks and vias it
    /// gained go after the file's last track and the zones after its last zone, and zones whose
    /// fills changed get the new ones.
    pub fn write(&self, src: &str) -> Result<String> {
        let c = &mut Cursor::new(src);
        c.open("kicad_pcb")?;
        // The file's tracks and vias, read as the parser reads them, with the text each one
        // takes from the end of the item before it, so removing it leaves no blank line.
        let (mut p, mut edits, mut track_spans, mut via_spans) = (Parser::new(), vec![], vec![], vec![]);
        // The footprints read and the board's found among them, in the same order, with their
        // pads' nets as the file has them; those F8 swapped, with their text.
        let (mut read, mut fps, mut pad_nets, mut swapped) = (0, 0, vec![], vec![]);
        // Where each zone the parser reads starts (it skips rule areas that allow zone copper),
        // and its text; the net table's entries (up to KiCad 9), with their nets.
        let (mut zones, mut table) = (vec![], vec![]);
        // New tracks go after the last one, or after the last item of a board without tracks;
        // new zones after the last zone, else with the new tracks; new parts after the last one.
        let (mut end, mut after_tracks, mut after_zones, mut after_parts) = (c.span().end, None, None, None);
        // The board's own drawings (`gr_line`…, not its parts'), as the parser reads them; new
        // ones go after the last, else after the parts. Its own texts shown, each where it starts.
        let (mut drawing_spans, mut after_drawings, mut own_texts) = (vec![], None, vec![]);
        while let Some(head) = c.child()? {
            let (mut gone, mut net, mut zone, mut text) = (false, 0, None, None);
            match head {
                "version" => p.b.version = c.atom()?.parse().unwrap_or(0),
                "layers" => p.layers(c)?,
                "net" => net = p.net(c)?,
                "footprint" | "module" => {
                    match self.footprints.get(fps).map(|f| (f, &f.src)) {
                        Some((f, Src::File(k))) if *k == read => {
                            pad_nets.push((f, footprint(self, p.b.version, c, src, f, &mut edits)?));
                            fps += 1;
                        }
                        Some((f, Src::Swapped(k, lib))) if *k == read => {
                            let (open, mut list) = (src[..c.span().start].rfind('(').unwrap_or_default(), c.clone());
                            list.close()?;
                            swapped.push((f, lib, open..list.span().end));
                            fps += 1;
                        }
                        _ => gone = true,
                    }
                    read += 1;
                }
                "segment" | "arc" => p.track(c)?,
                "via" => p.via(c)?,
                "gr_line" | "gr_arc" | "gr_circle" | "gr_rect" | "gr_poly" => p.shape(c, head, false)?,
                "gr_text" => {
                    let at = c.clone();
                    text = p.own_text(c)?.then_some(at);
                }
                "zone" => {
                    let (at, n) = (c.clone(), p.b.zones.len());
                    p.zone(c)?;
                    zone = (p.b.zones.len() > n).then_some(at);
                }
                _ => {}
            }
            c.close()?;
            let start = std::mem::replace(&mut end, c.span().end);
            if gone {
                edits.push((start..end, String::new()));
            }
            match head {
                "net" => {
                    table.push((net, start..end));
                    continue;
                }
                "segment" | "arc" => track_spans.push(start..end),
                "via" => via_spans.push(start..end),
                "zone" => {
                    zones.extend(zone.map(|at| (at, start..end)));
                    after_zones = Some(end);
                    continue;
                }
                "footprint" | "module" => {
                    after_parts = Some(end);
                    continue;
                }
                "gr_line" | "gr_arc" | "gr_circle" | "gr_rect" | "gr_poly" => {
                    drawing_spans.push(start..end);
                    after_drawings = Some(end);
                    continue;
                }
                "gr_text" => {
                    own_texts.extend(text.map(|at| (at, start..end)));
                    after_drawings = Some(end);
                    continue;
                }
                _ => continue,
            }
            after_tracks = Some(end);
        }
        let added: Vec<_> =
            self.footprints[fps..].iter().filter_map(|f| if let Src::Library(text) = &f.src { Some((f, text)) } else { None }).collect();
        if added.len() != self.footprints.len() - fps {
            return Err(c.err(tr(MISMATCH)));
        }
        let version = p.b.version;
        // The parser numbers layers and nets as it meets them, KiCad 10 nets where first used:
        // match them to the board's by name, nets by the names the board read them by.
        let ids = |names: &[String], ours: &[String]| {
            let ours: HashMap<&str, usize> = ours.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
            names.iter().map(|n| ours.get(n.as_str()).copied().unwrap_or(usize::MAX)).collect::<Vec<_>>()
        };
        let (layer, net) = (ids(&p.b.layers, &self.layers), ids(&p.b.nets, &self.file_nets));
        // Up to KiCad 9, files number nets (`(net 5 "GND")`, tracks `(net 5)`): nets new to the
        // file get the next numbers, declared after its last.
        let mut codes: HashMap<NetId, u32> = HashMap::new();
        for (&code, &id) in &p.net_codes {
            if let Some(&n) = net.get(id as usize).filter(|&&n| n != usize::MAX) {
                let c = codes.entry(n as NetId).or_insert(code);
                *c = code.min(*c);
            }
        }
        let mut declared = String::new();
        if let Some(&last) = codes.values().max() {
            let new = (1..self.nets.len() as NetId).filter(|id| !codes.contains_key(id)).collect::<Vec<_>>();
            for (code, id) in (last + 1..).zip(new) {
                codes.insert(id, code);
                declared +=
                    &format!("{}(net {code} {})", if version >= KICAD8 { "\n\t" } else { "\n  " }, quote(&self.nets[id as usize], version));
            }
        }
        // Parts added from a library, placed as the board has them, a level in.
        let mut parts = String::new();
        let indent = if version >= KICAD8 { "\n\t" } else { "\n  " };
        for (f, text) in added {
            let (fc, mut placed) = (&mut Cursor::new(text), vec![]);
            fc.child()?;
            let pads = footprint(self, version, fc, text, f, &mut placed)?;
            self.pad_nets(f, &pads, &codes, version, &mut placed).map_err(|msg| c.err(msg))?;
            parts += indent;
            parts += &apply(text, placed).replace('\n', indent);
        }
        for (f, pads) in pad_nets {
            self.pad_nets(f, &pads, &codes, version, &mut edits).map_err(|msg| c.err(msg))?;
        }
        // Parts F8 swapped: the file's text of each with the library's lists (`save/swap.rs`),
        // placed as the board has it.
        for (f, lib, span) in swapped {
            let text = swap::merged(self, version, &src[span.clone()], lib, f)?;
            let (fc, mut placed) = (&mut Cursor::new(&text), vec![]);
            fc.child()?;
            let pads = footprint(self, version, fc, &text, f, &mut placed)?;
            self.pad_nets(f, &pads, &codes, version, &mut placed).map_err(|msg| c.err(msg))?;
            edits.push((span, apply(&text, placed)));
        }
        // The file's items on a net the board renamed name it as the board does.
        let rename = |id: NetId, span: &Range<usize>, edits: &mut Vec<Edit>| -> Result<()> {
            let n = net.get(id as usize).copied().unwrap_or(usize::MAX);
            if let Some(name) = self.nets.get(n).filter(|name| **name != p.b.nets[id as usize]) {
                edits.extend(named(src, span.clone())?.into_iter().map(|at| (at, quote(name, version))));
            }
            Ok(())
        };
        for (id, span) in &table {
            rename(*id, span, &mut edits)?;
        }
        let on = |m: LayerMask| {
            (0..p.b.layers.len()).filter(|&i| m >> i & 1 != 0).fold(0, |o, i| o | 1u128.checked_shl(layer[i] as u32).unwrap_or(0))
        };
        let tracks =
            p.b.tracks.iter().map(|t| Track { layer: layer[t.layer as usize] as LayerId, net: net[t.net as usize] as NetId, ..*t });
        let vias: Vec<(Via, Range<usize>)> =
            p.b.vias
                .iter()
                .map(|v| Via { layers: on(v.layers), net: net[v.net as usize] as NetId, ..*v })
                .zip(via_spans.iter().cloned())
                .collect();
        // shortcut: tracks are matched by value, so a moved one is written anew (a new id, after the
        // last track: a group or tuning pattern naming its old id loses it); give them their place in
        // the file (a `src`, as parts have) when one is edited in place.
        let (new_tracks, gone_tracks) = matched(&self.tracks, tracks.zip(track_spans.iter().cloned()));
        let (mut new_vias, mut gone_vias) = matched(&self.vias, vias.iter().cloned());
        // A via the board moved (a shove) keeps its text, its id and all KiCad wrote of it, but its place.
        let was = |span: &Range<usize>| vias.iter().find(|f| f.1 == *span).map(|f| f.0);
        let mut moved = vec![];
        new_vias.retain(|&i| {
            let v = self.vias[i];
            let Some(k) = gone_vias.iter().position(|s| was(s).is_some_and(|f| f == Via { at: f.at, ..v })) else { return true };
            moved.push((gone_vias.remove(k), v.at));
            false
        });
        for (span, at) in moved {
            edits.push(via_at(src, span, at)?);
        }
        let gone: HashSet<usize> = gone_tracks.iter().chain(&gone_vias).map(|span| span.start).collect();
        let file_nets = p.b.tracks.iter().map(|t| t.net).zip(&track_spans).chain(p.b.vias.iter().map(|v| v.net).zip(&via_spans));
        for (id, span) in file_nets.filter(|(_, span)| !gone.contains(&span.start)) {
            rename(id, span, &mut edits)?;
        }
        edits.extend(gone_tracks.into_iter().chain(gone_vias).map(|span| (span, String::new())));
        // The board's zones are the file's it kept, by their place there, then the new ones.
        let mut kept = self.zones.iter().filter(|z| z.src.is_some()).peekable();
        for (k, (read, (at, span))) in p.b.zones.iter().zip(&mut zones).enumerate() {
            let Some(z) = kept.next_if(|z| z.src == Some(k)) else {
                edits.push((span.clone(), String::new()));
                continue;
            };
            // A zone on another net has it written anew; its net renamed, only its name.
            let file_net = net[read.net as usize] as NetId;
            if file_net == z.net {
                rename(read.net, span, &mut edits)?;
            }
            self.zone_edits(&mut at.clone(), (z, read, file_net), version, &codes, &mut edits)?;
            let file = p.b.fills[read.fills.clone()].iter().map(|f| (layer[f.layer as usize], &f.pts));
            if !self.fills[z.fills.clone()].iter().map(|f| (f.layer as usize, &f.pts)).eq(file) {
                self.zone_fills(at, z, version, &mut edits)?;
            }
        }
        if let (false, Some((_, span))) = (declared.is_empty(), table.last()) {
            edits.push((span.end..span.end, declared));
        }
        let at = after_tracks.unwrap_or(end);
        let copper = self.added(version, &codes, &new_tracks, &new_vias, src.len()).map_err(|msg| c.err(msg))?;
        if kept.next().is_some() {
            return Err(c.err(tr(MISMATCH)));
        }
        let new_zones = (0..self.zones.len()).filter(|&i| self.zones[i].src.is_none());
        let zone_text = self.zones_text(version, &codes, new_zones, src.len()).map_err(|msg| c.err(msg))?;
        edits.push((at..at, copper));
        let at_parts = after_parts.unwrap_or(at);
        edits.push((at_parts..at_parts, parts));
        // The board's own drawings, matched to the file's by value.
        // shortcut: as tracks, a drawing moved is written anew, with a new id; give them their
        // place in the file when groups of drawings are edited here.
        let (own_shapes, own) = self.own();
        let ours: Vec<&Shape> = own_shapes.into_iter().map(|i| &self.shapes[i]).collect();
        let theirs: Vec<Shape> = p.b.shapes.iter().map(|s| Shape { layer: layer[s.layer as usize] as LayerId, ..s.clone() }).collect();
        let (new_drawings, gone_drawings) = matched(&ours, theirs.iter().zip(drawing_spans));
        edits.extend(gone_drawings.into_iter().map(|span| (span, String::new())));
        let new_drawings: Vec<&Shape> = new_drawings.into_iter().map(|i| ours[i]).collect();
        let mut drawn = self.drawings_text(version, &new_drawings, src.len()).map_err(|msg| c.err(msg))?;
        // Its own texts are the file's it kept, each by its place among them, edited where it is;
        // then the new ones.
        let mut kept: HashMap<usize, &Text> = own.iter().filter_map(|&i| Some((self.texts[i].src?, &self.texts[i]))).collect();
        for (k, ((mut at, span), read)) in own_texts.into_iter().zip(&p.b.texts).enumerate() {
            match kept.remove(&k) {
                Some(t) => {
                    let read = Text { layer: layer[read.layer as usize] as LayerId, ..read.clone() };
                    texts::own_edits(self, t, &read, &mut at, src, version, &mut edits)?;
                }
                None => edits.push((span, String::new())),
            }
        }
        if !kept.is_empty() {
            return Err(c.err(tr(MISMATCH)));
        }
        let new_texts: Vec<&Text> = own.iter().map(|&i| &self.texts[i]).filter(|t| t.src.is_none()).collect();
        drawn += &self.texts_text(version, &new_texts, src.len());
        let at = after_drawings.unwrap_or(at_parts);
        edits.push((at..at, drawn));
        let at = after_zones.unwrap_or(at);
        edits.push((at..at, zone_text));
        Ok(apply(src, edits))
    }

    /// The tracks and vias at the indices `tracks` and `vias`, as KiCad of file `version` writes
    /// them; `codes` numbers the nets in files before KiCad 10, which names them. `salt` keeps
    /// the ids of items added in different sessions apart.
    fn added(
        &self,
        version: u32,
        codes: &HashMap<NetId, u32>,
        tracks: &[usize],
        vias: &[usize],
        salt: usize,
    ) -> std::result::Result<String, &'static str> {
        let layer = |l: u8| quote(&self.layers[l as usize], version);
        let item = |head: &str, fields: Vec<String>, id: String| item(head, fields, id, version);
        let mut out = String::new();
        for (i, t) in tracks.iter().map(|&i| (i, &self.tracks[i])) {
            let mut fields = vec![format!("(start {})", xy(t.a))];
            fields.extend(t.mid.map(|m| format!("(mid {})", xy(m))));
            fields.extend([format!("(end {})", xy(t.b)), format!("(width {})", nm_to_mm(t.width)), format!("(layer {})", layer(t.layer))]);
            fields.extend(self.net_field(t.net, codes)?);
            out += &item(if t.mid.is_some() { "arc" } else { "segment" }, fields, uuid((salt, i, t)));
        }
        for (i, v) in vias.iter().map(|&i| (i, &self.vias[i])) {
            // A via names the outermost copper layers it spans.
            let mut spanned = self.stackup.iter().filter(|cu| v.layers & 1 << cu.layer != 0).map(|cu| layer(cu.layer));
            let top = spanned.next().ok_or_else(|| tr("a new via spans no copper layer"))?;
            let bottom = spanned.next_back().unwrap_or_else(|| top.clone());
            let (size, drill) = (nm_to_mm(v.size), nm_to_mm(v.drill));
            let mut fields = vec![
                format!("(at {})", xy(v.at)),
                format!("(size {size})"),
                format!("(drill {drill})"),
                format!("(layers {top} {bottom})"),
            ];
            fields.extend(self.net_field(v.net, codes)?);
            // One that stops inside the board says so: KiCad 10 tells buried from blind, files
            // before it call both blind.
            let head = match (self.span(v), version) {
                (Span::Through, _) => "via",
                (Span::Buried, 20250926..) => "via buried",
                _ => "via blind",
            };
            out += &item(head, fields, uuid((salt, i, v)));
        }
        Ok(out)
    }

    /// The board's own drawings `shapes` (what it gains), as KiCad of file `version` writes them:
    /// from KiCad 8 with a `stroke`, before it a `width`; from KiCad 6 arcs by three points,
    /// before it by centre and angle, and closed ones with their `fill`. `salt` keeps the ids of
    /// drawings added in different sessions apart.
    fn drawings_text(&self, version: u32, shapes: &[&Shape], salt: usize) -> std::result::Result<String, &'static str> {
        let mut out = String::new();
        for (i, s) in shapes.iter().enumerate() {
            let (width, layer) =
                (format!("(width {})", nm_to_mm(s.width)), format!("(layer {})", quote(&self.layers[s.layer as usize], version)));
            let (head, mut fields, closed) = match &s.geom {
                Geom::Segment { a, b } => ("gr_line", vec![format!("(start {})", xy(*a)), format!("(end {})", xy(*b))], false),
                Geom::Arc { start, mid, end } if version >= KICAD6 => {
                    ("gr_arc", vec![format!("(start {})", xy(*start)), format!("(mid {})", xy(*mid)), format!("(end {})", xy(*end))], false)
                }
                // KiCad 5: its `start` the centre, its `end` where the arc starts, turned `angle`,
                // which it keeps in tenths of a degree: the centre is where that angle puts it.
                Geom::Arc { start, mid, end } => {
                    let sweep = arc_center(*start, *mid, *end).ok_or_else(|| tr("an arc through three points in a line"))?.4;
                    let angle = (sweep.to_degrees() * 10.0).round() / 10.0;
                    let (dx, dy, k) = ((end.x - start.x) as f64, (end.y - start.y) as f64, 0.5 / (angle.to_radians() / 2.0).tan());
                    let centre = Pt {
                        x: ((start.x + end.x) as f64 / 2.0 - dy * k).round() as i32,
                        y: ((start.y + end.y) as f64 / 2.0 + dx * k).round() as i32,
                    };
                    ("gr_arc", vec![format!("(start {})", xy(centre)), format!("(end {})", xy(*start)), format!("(angle {angle})")], false)
                }
                Geom::Circle { center, radius } => {
                    let end = Pt { x: center.x + radius, y: center.y };
                    ("gr_circle", vec![format!("(center {})", xy(*center)), format!("(end {})", xy(end))], true)
                }
                Geom::Poly(pts) => {
                    let pts: Vec<String> = pts.iter().map(|&p| format!("(xy {})", xy(p))).collect();
                    ("gr_poly", vec![list("pts", &[pts.join(" ")], version)], true)
                }
            };
            // KiCad 9 on asks `(fill yes)`; 6 to 8 `(fill solid)`; 5 fills every polygon.
            let fill = match (closed, version) {
                (true, 20241229..) => Some(format!("(fill {})", if s.filled { "yes" } else { "no" })),
                (true, KICAD6..) => Some(format!("(fill {})", if s.filled { "solid" } else { "none" })),
                _ => None,
            };
            match version {
                KICAD8.. => {
                    fields.push(list("stroke", &[width, "(type default)".into()], version));
                    fields.extend(fill);
                    fields.push(layer);
                }
                _ => {
                    fields.extend([layer, width]);
                    fields.extend(fill);
                }
            }
            out += &item(head, fields, uuid((salt, i, s)), version);
        }
        Ok(out)
    }

    /// The board's own texts `texts` (what it gains), as KiCad of file `version` writes them;
    /// `salt` keeps the ids of texts added in different sessions apart.
    fn texts_text(&self, version: u32, texts: &[&Text], salt: usize) -> String {
        let mut out = String::new();
        for (i, t) in texts.iter().enumerate() {
            let deg = ((t.angle * 1e6).round() / 1e6).rem_euclid(360.0) + 0.0;
            // KiCad leaves a 0° out, but from KiCad 8 on.
            let at = if deg == 0.0 && version < KICAD8 { format!("(at {})", xy(t.at)) } else { format!("(at {} {deg})", xy(t.at)) };
            let font = [format!("(size {} {})", nm_to_mm(t.size.1), nm_to_mm(t.size.0)), format!("(thickness {})", nm_to_mm(t.thickness))];
            let font = match version {
                KICAD8.. => format!("(font\n\t\t\t\t{}\n\t\t\t)", font.join("\n\t\t\t\t")),
                _ => format!("(font {})", font.join(" ")),
            };
            let justify = [
                (t.justify.0 < 0).then_some("left"),
                (t.justify.0 > 0).then_some("right"),
                (t.justify.1 < 0).then_some("top"),
                (t.justify.1 > 0).then_some("bottom"),
                t.mirror.then_some("mirror"),
            ];
            let justify: Vec<&str> = justify.into_iter().flatten().collect();
            let mut effects = vec![font];
            if !justify.is_empty() {
                effects.push(format!("(justify {})", justify.join(" ")));
            }
            let fields =
                vec![at, format!("(layer {})", quote(&self.layers[t.layer as usize], version)), list("effects", &effects, version)];
            out += &item(&format!("gr_text {}", quote(&escape(&t.text), version)), fields, uuid((salt, i, &t.text, t.at)), version);
        }
        out
    }

    /// An item's `(net …)` as KiCad of the file writes it: by number (`codes`) up to KiCad 9, by
    /// name from KiCad 10, which leaves "no net" out.
    fn net_field(&self, n: NetId, codes: &HashMap<NetId, u32>) -> std::result::Result<Option<String>, &'static str> {
        let name = self.nets.get(n as usize).ok_or_else(|| tr("a new item's net is not on the board"))?;
        if !codes.is_empty() {
            return Ok(Some(format!("(net {})", codes.get(&n).ok_or_else(|| tr("a new item's net is not declared in the file"))?)));
        }
        Ok((n != 0).then(|| format!("(net \"{name}\")")))
    }

    /// Gives footprint `f`'s pads the board's nets over `pads`, the file's (`footprint`), as
    /// KiCad of file `version` writes them: by number (`codes`) up to KiCad 9, by name after.
    fn pad_nets(
        &self,
        f: &Footprint,
        pads: &[PadNet],
        codes: &HashMap<NetId, u32>,
        version: u32,
        edits: &mut Vec<Edit>,
    ) -> std::result::Result<(), &'static str> {
        if pads.len() != f.pads.len() {
            return Err(tr(MISMATCH));
        }
        for (pad, file) in self.pads[f.pads.clone()].iter().zip(pads) {
            let (code, name) = (codes.get(&pad.net).copied(), self.nets[pad.net as usize].as_str());
            match file.net {
                // Its net as the file has it, by number and name, or by number alone.
                Some((c, n)) if c == code && (n == name || n.is_empty() && c.is_some()) => {}
                None if pad.net == 0 => {}
                Some(_) if pad.net == 0 => edits.push((file.spaced.clone(), String::new())),
                _ => {
                    let net = match code {
                        Some(code) => format!("(net {code} {})", quote(name, version)),
                        None if codes.is_empty() => format!("(net {})", quote(name, version)),
                        None => return Err(tr("a pad's net is not declared in the file")),
                    };
                    let space = if file.net.is_none() { file.space } else { "" };
                    edits.push((file.list.clone(), format!("{space}{net}")));
                }
            }
        }
        Ok(())
    }
}

/// A board item `(head fields…)` with its id, as KiCad of file `version` writes it: up to KiCad 7
/// on one line, from KiCad 8 a line per field.
fn item(head: &str, mut fields: Vec<String>, id: String, version: u32) -> String {
    if version >= KICAD8 {
        fields.push(format!("(uuid \"{id}\")"));
        return format!("\n\t({head}\n\t\t{}\n\t)", fields.join("\n\t\t"));
    }
    if version >= KICAD6 {
        fields.push(format!("(tstamp {id})"));
    }
    format!("\n  ({head} {})", fields.join(" "))
}

/// A list inside an item, `(head items…)`, as KiCad of file `version` writes it: from KiCad 8 a
/// line per item.
fn list(head: &str, items: &[String], version: u32) -> String {
    match version {
        KICAD8.. => format!("({head}\n\t\t\t{}\n\t\t)", items.join("\n\t\t\t")),
        _ => format!("({head} {})", items.join(" ")),
    }
}

/// A point as KiCad writes one, in mm.
fn xy(p: Pt) -> String {
    format!("{} {}", nm_to_mm(p.x), nm_to_mm(p.y))
}

/// The edit that writes `at` over the place of the via whose text is `span` of `src`.
fn via_at(src: &str, span: Range<usize>, at: Pt) -> Result<Edit> {
    let c = &mut Cursor::new(&src[span.clone()]);
    c.child()?;
    while let Some(head) = c.child()? {
        if head == "at" {
            let xy_span = read_at(c)?.xy;
            return Ok((span.start + xy_span.start..span.start + xy_span.end, xy(at)));
        }
        c.close()?;
    }
    Err(c.err(tr("a via without (at)")))
}

/// Matches the file's items (with their text) to the board's by value, as many of each as the
/// board has: returns the indices of the board's new ones and the text of the file's it lacks.
fn matched<T: Copy + Eq + Hash>(board: &[T], file: impl Iterator<Item = (T, Range<usize>)>) -> (Vec<usize>, Vec<Range<usize>>) {
    let mut left: HashMap<T, usize> = HashMap::new();
    for &item in board {
        *left.entry(item).or_default() += 1;
    }
    let mut gone = vec![];
    for (item, span) in file {
        match left.get_mut(&item) {
            Some(n) if *n > 0 => *n -= 1,
            _ => gone.push(span),
        }
    }
    let mut new = vec![];
    for (i, item) in board.iter().enumerate() {
        if let Some(n) = left.get_mut(item).filter(|n| **n > 0) {
            *n -= 1;
            new.push(i);
        }
    }
    (new, gone)
}

/// A pad's net as the file has it, and where saving writes the board's.
struct PadNet<'a> {
    /// The number and name its `(net …)` list writes, `None` when it has none.
    net: Option<(Option<u32>, &'a str)>,
    /// That list, without and with the space before it; when it has none, where one goes: right
    /// after `(layers …)`, with the space before that in `space`.
    list: Range<usize>,
    spaced: Range<usize>,
    space: &'a str,
}

/// Reads a footprint's children (`src` is its text, of file `version`) and, when board `b` moved,
/// turned or flipped it, edits its placement, its pad and text angles (absolute in the file) and
/// its zones' corners (in board coordinates in the file), a flipped one's sides too (`flip.rs`);
/// its texts placed or hidden otherwise (`texts.rs`); returns its pads' nets as the file has them.
// shortcut: text boxes, dimensions and barcodes in a footprint keep their angle; turn them too
// when boards have them.
fn footprint<'a>(
    b: &Board,
    version: u32,
    c: &mut Cursor<'a>,
    src: &'a str,
    f: &Footprint,
    edits: &mut Vec<Edit>,
) -> Result<Vec<PadNet<'a>>> {
    let (first, mut back) = (c.clone(), false);
    let (mut at, mut angles, mut corners, mut end, mut pads) = (None, vec![], vec![], None, vec![]);
    // Its value and fields, and where a new field goes: after the last, else after `(at …)`;
    // its texts' lists.
    let (mut fields, mut after_field, mut after_at, mut texts) = (vec![], None, None, vec![]);
    while let Some(head) = c.child()? {
        let open = src[..c.span().start].rfind('(').unwrap_or_default();
        match head {
            "at" => at = Some(read_at(c)?),
            "layer" => back = b.on_back(c.atom()?),
            "fp_text" | "property" => {
                let (name, raw) = (c.atom()?, c.atom()?);
                fields.push(fields::Field { property: head == "property", name, raw, value: c.span() });
                texts.push(texts::read(c, src)?);
            }
            "pad" => {
                let (mut pad, mut layers) = (PadNet { net: None, list: 0..0, spaced: 0..0, space: "" }, false);
                while let Some(child) = c.child()? {
                    // Where the list starts, and the space before it.
                    let open = src[..c.span().start].rfind('(').unwrap_or_default();
                    let space = src[..open].trim_end().len();
                    let net = match child {
                        "at" => {
                            angles.push(read_at(c)?);
                            None
                        }
                        "net" => Some(net_list(c, child)?),
                        _ => None,
                    };
                    c.close()?;
                    let end = c.span().end;
                    if let Some((code, name)) = net {
                        let net = Some((code, name.map_or("", |(_, name)| name)));
                        pad = PadNet { net, list: open..end, spaced: space..end, space: &src[space..open] };
                    } else if pad.net.is_none() && !layers {
                        (pad.list, pad.spaced, pad.space, layers) = (end..end, end..end, &src[space..open], child == "layers");
                    }
                }
                pads.push(pad);
            }
            "zone" => zone_corners(c, &mut corners)?,
            _ => {}
        }
        c.close()?;
        end = Some(c.span().end);
        let after = Some((c.span().end, &src[src[..open].trim_end().len()..open]));
        match head {
            "property" => after_field = after,
            "at" => after_at = after,
            _ => {}
        }
    }
    let last = after_field.or(after_at).unwrap_or((end.unwrap_or(c.span().end), " "));
    fields::edits(f, &fields, last, version, edits);
    edits.extend(models::edits(first.clone(), src, &f.models, version)?);
    let (from, deg) = at.as_ref().map_or((Pt::default(), 0.0), |a| (a.at, a.deg));
    // Flipped, its angle is the file's negated, then turned.
    let flipped = back != f.back;
    let turn = (f.angle - if flipped { -deg } else { deg }).rem_euclid(360.0);
    let (moved, turned) = (f.at != from, turn.min(360.0 - turn) > 1e-6);
    let turn = if turned { turn } else { 0.0 };
    // Its texts placed or hidden otherwise: the places written are no longer the part's to turn.
    let own = texts::edits(b, f, &texts, version, (flipped, turn), edits);
    angles.extend(texts.into_iter().filter_map(|t| t.at).filter(|a| !own.iter().any(|r| r.contains(&a.xy.start))));
    if !moved && !turned && !flipped {
        return Ok(pads);
    }
    let xy = |p: Pt| format!("{} {}", nm_to_mm(p.x), nm_to_mm(p.y));
    // KiCad keeps footprint angles in (-180°, 180°] (KiCad 4 and 5 in [0°, 360°)), pad and text
    // angles in [0°, 360°).
    let angle = if version < KICAD6 { f.angle } else { 180.0 - (180.0 - f.angle).rem_euclid(360.0) };
    match at {
        Some(a) => {
            if moved {
                edits.push((a.xy, xy(f.at)));
            }
            if turned || flipped {
                edits.extend(angle_edit(a.angle, angle));
            }
        }
        None => {
            let end = end.ok_or_else(|| c.err(tr("footprint without items")))?;
            let a = angle_edit(end..end, angle).map_or(String::new(), |(_, a)| a);
            edits.push((end..end, format!(" (at {}{a})", xy(f.at))));
        }
    }
    if flipped {
        let n = edits.len();
        flip::Flip { b, version, src, from, to: f.at, turn }.part(&mut first.clone(), edits)?;
        let theirs: Vec<Edit> = edits.drain(n..).filter(|(span, _)| !own.iter().any(|r| r.contains(&span.start))).collect();
        edits.extend(theirs);
        return Ok(pads);
    }
    if turned {
        edits.extend(angles.into_iter().filter_map(|a| angle_edit(a.angle, (a.deg + turn).rem_euclid(360.0))));
    }
    let local = |p: Pt| Pt { x: p.x.saturating_sub(from.x), y: p.y.saturating_sub(from.y) };
    edits.extend(corners.into_iter().map(|(span, p)| (span, xy(place(local(p), f.at, turn)))));
    Ok(pads)
}

/// A value as written in the file, with where it is.
type Written<'a> = (Range<usize>, &'a str);

/// The number and name, with where the name is, of a `(net …)` or `(net_name …)` list just
/// entered: `(net 5 "GND")`, KiCad 10's `(net "GND")`, KiCad 5's bare `(net 5 GND)`; `(net 5)`
/// names none.
fn net_list<'a>(c: &mut Cursor<'a>, head: &str) -> Result<(Option<u32>, Option<Written<'a>>)> {
    let mut values = vec![];
    while let Some(v) = c.opt_atom()? {
        values.push((c.span(), v));
    }
    let quoted = |(span, v): &(Range<usize>, &str)| span.len() > v.len();
    Ok(match &values[..] {
        [name] if head == "net_name" || quoted(name) => (None, Some(name.clone())),
        [code, rest @ ..] => (code.1.parse().ok(), rest.first().cloned()),
        [] => (None, None),
    })
}

/// Where the item at `span` names nets: the names of its `(net …)` and `(net_name …)` lists, or
/// its own when it is a `(net 5 "GND")` declaration.
fn named(src: &str, span: Range<usize>) -> Result<Vec<Range<usize>>> {
    let c = &mut Cursor::new(&src[span.clone()]);
    let mut out = vec![];
    match c.child()? {
        Some("net") => out.extend(net_list(c, "net")?.1),
        _ => {
            while let Some(head) = c.child()? {
                if head == "net" || head == "net_name" {
                    out.extend(net_list(c, head)?.1);
                }
                c.close()?;
            }
        }
    }
    Ok(out.into_iter().map(|(at, _)| at.start + span.start..at.end + span.start).collect())
}

/// A string as KiCad of file `version` writes it: up to KiCad 5 bare when it can be.
fn quote(s: &str, version: u32) -> String {
    let bare = version < KICAD6 && !s.is_empty() && !s.contains([' ', '(', ')', '"']);
    if bare { s.to_string() } else { format!("\"{s}\"") }
}

/// An `(at X Y [A] …)` or `(xy X Y)` list: its point and angle, where X Y sit, where Y sits and
/// where the angle sits (an empty range right after Y when it has none).
pub(crate) struct At {
    pub(crate) at: Pt,
    pub(crate) deg: f64,
    pub(crate) xy: Range<usize>,
    pub(crate) y: Range<usize>,
    pub(crate) angle: Range<usize>,
}

/// Reads X Y and an optional angle from the list just entered.
pub(crate) fn read_at(c: &mut Cursor) -> Result<At> {
    let x = c.nm()?;
    let start = c.span().start;
    let at = Pt { x, y: c.nm()? };
    let y = c.span();
    let (deg, angle) = match c.opt_atom()?.map(str::parse::<f64>) {
        Some(Ok(deg)) => (deg, c.span()),
        _ => (0.0, y.end..y.end),
    };
    Ok(At { at, deg, xy: start..y.end, y, angle })
}

/// Writes `deg` over an angle, or after Y when there is none (`span` empty) and it isn't 0,
/// as KiCad leaves 0 out.
pub(crate) fn angle_edit(span: Range<usize>, deg: f64) -> Option<Edit> {
    // Micro-degrees: finer than any placement, and no float noise like 90.00000000000001.
    let text = format!("{}", (deg * 1e6).round() / 1e6 + 0.0);
    if !span.is_empty() { Some((span, text)) } else { (text != "0").then(|| (span, format!(" {text}"))) }
}

/// Every corner of a zone's outlines and fills (`xy`, and the points of arcs), with its place.
fn zone_corners(c: &mut Cursor, out: &mut Vec<(Range<usize>, Pt)>) -> Result<()> {
    while let Some(head) = c.child()? {
        if matches!(head, "xy" | "start" | "mid" | "end") {
            let a = read_at(c)?;
            out.push((a.xy, a.at));
        } else {
            zone_corners(c, out)?;
        }
        c.close()?;
    }
    Ok(())
}

/// A version 4 UUID made from `seed`, so writing the same board twice gives the same text.
pub(crate) fn uuid(seed: impl Hash) -> String {
    uuid_text(new_id(seed))
}

/// A version 4 UUID made from `seed`, as a number.
pub(crate) fn new_id(seed: impl Hash) -> u128 {
    let half = |n: u8| {
        let mut h = DefaultHasher::new();
        (n, &seed).hash(&mut h);
        h.finish() as u128
    };
    (half(0) << 64 | half(1)) & !(0xf << 76 | 0xc << 60) | (4 << 76 | 8 << 60)
}

/// An id as KiCad writes a UUID.
fn uuid_text(id: u128) -> String {
    let field = |shift: u32, digits: usize| format!("{:0digits$x}", (id >> shift) & ((1 << (4 * digits)) - 1));
    [field(96, 8), field(80, 4), field(64, 4), field(48, 4), field(0, 12)].join("-")
}

/// `src` with the edits made; they never overlap.
pub(crate) fn apply(src: &str, mut edits: Vec<Edit>) -> String {
    // Text put in where an item starts goes before that item's own edit (its removal).
    edits.sort_by_key(|(span, _)| (span.start, span.end));
    let mut out = String::with_capacity(src.len() + edits.iter().map(|(_, s)| s.len()).sum::<usize>());
    let mut done = 0;
    for (span, text) in &edits {
        out += &src[done..span.start];
        out += text;
        done = span.end;
    }
    out + &src[done..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{MM, Via};
    use crate::history::{Patch, Pick};

    #[test]
    fn rewrites_only_the_placement_of_moved_footprints() {
        let src = r#"(kicad_pcb (version 20221018)
  (footprint "R" (layer "F.Cu") (at 10 20 90)
    (fp_text reference "R1" (at 0 -1 90) (layer "F.SilkS"))
    (fp_text user "U" (at 0 1 unlocked) (layer "F.SilkS"))
    (pad "1" smd rect (at -1 0 90) (size 1 1) (layers "F.Cu"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu"))
    (zone (polygon (pts (xy 9 19) (xy 11 19) (xy 11 21)))))
  (footprint "C" (layer "F.Cu") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))))"#;
        let mut b = Board::parse(src).unwrap();
        assert_eq!(b.write(src).unwrap(), src);
        let pt = |x, y| Pt { x: x * MM, y: y * MM };
        // A quarter turn more: each angle turns along, the zone's corners (board coordinates)
        // move and turn about the anchor, and C, which has no (at), gets one.
        b.place_footprint(0, pt(12, 20), 180.0);
        b.place_footprint(1, pt(1, 2), 0.0);
        let out = b.write(src).unwrap();
        let want = r#"(kicad_pcb (version 20221018)
  (footprint "R" (layer "F.Cu") (at 12 20 180)
    (fp_text reference "R1" (at 0 -1 180) (layer "F.SilkS"))
    (fp_text user "U" (at 0 1 90 unlocked) (layer "F.SilkS"))
    (pad "1" smd rect (at -1 0 180) (size 1 1) (layers "F.Cu"))
    (pad "2" smd rect (at 1 0 90) (size 1 1) (layers "F.Cu"))
    (zone (polygon (pts (xy 11 21) (xy 11 19) (xy 13 19)))))
  (footprint "C" (layer "F.Cu") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")) (at 1 2)))"#;
        assert_eq!(out, want);
        let back = Board::parse(&out).unwrap();
        assert_eq!((back.footprints, back.pads, back.texts), (b.footprints.clone(), b.pads.clone(), b.texts.clone()));
        // KiCad's footprint angles stop at 180°; back where it was, the text is as it was.
        b.place_footprint(0, pt(12, 20), 270.0);
        assert!(b.write(src).unwrap().contains(r#"(at 12 20 -90)"#));
        b.place_footprint(0, pt(10, 20), 90.0);
        b.place_footprint(1, pt(0, 0), 0.0);
        assert_eq!(b.write(src).unwrap(), src);
    }

    #[test]
    fn writes_added_tracks_and_vias_as_the_files_kicad_version_does() {
        for (version, (f, b), nets, net) in [
            ("20171130", ("F.Cu", "B.Cu"), r#"(net 0 "") (net 1 GND)"#, "(net 1)"),
            ("20221018", (r#""F.Cu""#, r#""B.Cu""#), r#"(net 0 "") (net 1 "GND")"#, "(net 1)"),
            ("20240108", (r#""F.Cu""#, r#""B.Cu""#), r#"(net 0 "") (net 1 "GND")"#, "(net 1)"),
            ("20260206", (r#""F.Cu""#, r#""B.Cu""#), "", r#"(net "GND")"#),
        ] {
            let src = format!(
                "(kicad_pcb (version {version}) (layers (0 {f} signal) (31 {b} signal)) {nets}
  (segment (start 0 0) (end 1 0) (width 0.25) (layer {f}) {net})
  (zone {net}))"
            );
            let mut board = Board::parse(&src).unwrap();
            board.add_tracks(&[Pt { x: MM, y: 0 }, Pt { x: 2 * MM, y: 0 }], 250_000, 0, 1);
            board.vias.push(Via { at: Pt { x: 2 * MM, y: 0 }, size: 800_000, drill: 400_000, layers: board.copper, net: 1, locked: false });
            let out = board.write(&src).unwrap();
            let (t, v) = (uuid((src.len(), 1usize, &board.tracks[1])), uuid((src.len(), 0usize, &board.vias[0])));
            let (seg, via) = ("(start 1 0) (end 2 0) (width 0.25)", "(at 2 0) (size 0.8) (drill 0.4)");
            let added = match version {
                // KiCad 5: bare names and no ids; KiCad 6 and 7: quoted names and a `tstamp`.
                "20171130" => format!("\n  (segment {seg} (layer {f}) {net})\n  (via {via} (layers {f} {b}) {net})"),
                "20221018" => {
                    format!("\n  (segment {seg} (layer {f}) {net} (tstamp {t}))\n  (via {via} (layers {f} {b}) {net} (tstamp {v}))")
                }
                // KiCad 8 on: a line per child; KiCad 10 names the net.
                _ => format!(
                    "\n\t(segment\n\t\t{}\n\t\t(layer {f})\n\t\t{net}\n\t\t(uuid \"{t}\")\n\t)\n\t(via\n\t\t{}\n\t\t(layers {f} {b})\n\t\t{net}\n\t\t(uuid \"{v}\")\n\t)",
                    seg.replace(") (", ")\n\t\t("),
                    via.replace(") (", ")\n\t\t(")
                ),
            };
            let (before, after) = src.split_at(src.find("\n  (zone").unwrap());
            assert_eq!(out, format!("{before}{added}{after}"), "{version}");
            let back = Board::parse(&out).unwrap();
            assert_eq!((back.tracks, back.vias), (board.tracks, board.vias), "{version}");
        }
        let id = uuid(1);
        assert!(id.len() == 36 && id.as_bytes()[14] == b'4' && id != uuid(2), "{id}");
    }

    #[test]
    fn writes_the_drawings_the_board_gained_and_removes_those_it_lost() {
        for (version, (s, e)) in [
            ("20171130", ("F.SilkS", "Edge.Cuts")),
            ("20221018", (r#""F.SilkS""#, r#""Edge.Cuts""#)),
            ("20260206", (r#""F.SilkS""#, r#""Edge.Cuts""#)),
        ] {
            let head = format!("(kicad_pcb (version {version}) (layers (0 F.Cu signal) (37 {s} user) (44 {e} user))");
            let rect = format!("\n  (gr_rect (start 0 0) (end 20 10) (layer {e}) (width 0.05))");
            let (silk, track) = (
                format!("\n  (gr_line (start 1 1) (end 2 2) (layer {s}) (width 0.1))"),
                "\n  (segment (start 0 0) (end 1 0) (width 0.25) (layer F.Cu)))",
            );
            let src = format!("{head}{rect}{silk}{track}");
            let mut board = Board::parse(&src).unwrap();
            board.set_outline(&[(0, 0), (30, 0), (0, 15)].map(|(x, y)| Pt { x: x * MM, y: y * MM })).unwrap();
            let out = board.write(&src).unwrap();
            let id = |i: usize| uuid((src.len(), i, &board.shapes[i]));
            let line = |i: usize, (a, b): (&str, &str)| match version {
                // KiCad 5: bare names and no id; KiCad 6 and 7: quoted names, a `width` and a `tstamp`.
                "20171130" => format!("\n  (gr_line (start {a}) (end {b}) (layer {e}) (width 0.05))"),
                "20221018" => format!("\n  (gr_line (start {a}) (end {b}) (layer {e}) (width 0.05) (tstamp {}))", id(i)),
                // KiCad 8 on: a line per child, and a `stroke`.
                _ => format!(
                    "\n\t(gr_line\n\t\t(start {a})\n\t\t(end {b})\n\t\t(stroke\n\t\t\t(width 0.05)\n\t\t\t(type default)\n\t\t)\n\t\t(layer {e})\n\t\t(uuid \"{}\")\n\t)",
                    id(i)
                ),
            };
            let added: String =
                [("0 0", "30 0"), ("30 0", "0 15"), ("0 15", "0 0")].into_iter().enumerate().map(|(i, ab)| line(i, ab)).collect();
            // The rectangle goes with the space before it; the new lines follow the last drawing.
            assert_eq!(out, format!("{head}{silk}{added}{track}"), "{version}");
            assert_eq!(Board::parse(&out).unwrap().shapes[1..], board.shapes[..3], "{version}");
        }
    }

    #[test]
    fn removes_the_tracks_and_vias_the_board_lost() {
        // KiCad 10 names nets where first used: the pad's B comes before the tracks' A.
        let src = r#"(kicad_pcb (version 20260206) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
	(footprint "R" (layer "F.Cu") (pad "1" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "B")))
	(segment (start 0 0) (end 1 0) (width 0.25) (layer "F.Cu") (net "A") (uuid "a"))
	(segment (start 0 0) (end 1 0) (width 0.25) (layer "F.Cu") (net "A") (uuid "b"))
	(segment (start 1 0) (end 2 0) (width 0.25) (layer "F.Cu") (net "B") (uuid "c"))
	(via (at 2 0) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net "B") (uuid "d"))
	(zone (net "B")))"#;
        let mut b = Board::parse(src).unwrap();
        // Of two equal tracks the board kept one: the file keeps its first.
        let (twin, via) = (b.tracks.remove(1), b.vias.remove(0));
        let out = b.write(src).unwrap();
        let line = |id: &str| src.lines().find(|l| l.contains(id)).unwrap().to_string();
        assert_eq!(out, src.replace(&format!("\n{}", line("\"b\"")), "").replace(&format!("\n{}", line("\"d\"")), ""));
        let back = Board::parse(&out).unwrap();
        assert_eq!((&back.tracks, &back.vias), (&b.tracks, &b.vias));
        // Put back (an undo after that save), they are the text's again: it comes back as read.
        (b.tracks, b.vias) = (vec![twin, b.tracks[0], b.tracks[1]], vec![via]);
        assert_eq!(b.write(src).unwrap(), src);
    }

    #[test]
    fn a_moved_via_keeps_its_text_but_its_place() {
        let src = r#"(kicad_pcb (version 20240108) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
	(net 0 "") (net 1 "A")
	(via (at 2 0) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (free yes) (net 1) (uuid "d"))
	(via (at 5 0) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1) (uuid "e")))"#;
        let mut b = Board::parse(src).unwrap();
        b.vias[0].at = Pt { x: 2_500_000, y: -250_000 };
        let out = b.write(src).unwrap();
        assert_eq!(out, src.replace("(at 2 0)", "(at 2.5 -0.25)"));
        assert_eq!(Board::parse(&out).unwrap().vias, b.vias);
    }

    #[test]
    fn removes_the_footprints_the_board_lost() {
        let src = r#"(kicad_pcb (version 20240108)
	(footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "R1") (pad "1" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))
	(footprint "C" (layer "F.Cu") (at 10 0) (property "Reference" "C1") (pad "1" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))
	(footprint "D" (layer "F.Cu") (at 20 0) (property "Reference" "D1") (pad "1" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))
)"#;
        let mut b = Board::parse(src).unwrap();
        // C1 goes with the line it was on; D1, moved into its place in the list, is found.
        let undo = b.apply(Patch::delete(&b, Pick::Footprint(1)));
        b.place_footprint(1, Pt { x: 21 * MM, y: 0 }, 0.0);
        let out = b.write(src).unwrap();
        let c1 = src.lines().find(|l| l.contains("C1")).unwrap();
        assert_eq!(out, src.replace(&format!("\n{c1}"), "").replace("(at 20 0)", "(at 21 0)"));
        // Read back, the footprints are numbered by the new text.
        let back = Board::parse(&out).unwrap();
        let mut want = b.footprints.clone();
        want.iter_mut().enumerate().for_each(|(k, f)| f.src = Src::File(k));
        assert_eq!((back.footprints, back.pads), (want, b.pads.clone()));
        // Put back, with D1 where it was: the text as read.
        b.place_footprint(1, Pt { x: 20 * MM, y: 0 }, 0.0);
        b.apply(undo);
        assert_eq!(b.write(src).unwrap(), src);
        // A footprint the text lacks: the board was read from other text.
        b.footprints[2].src = Src::File(3);
        assert_eq!(b.write(src).map_err(|e| e.msg), Err(MISMATCH));
    }

    #[test]
    fn writes_parts_added_from_a_library_after_the_files_placed() {
        let src = "(kicad_pcb (version 20260206) (layers (0 \"F.Cu\" signal) (31 \"B.Cu\" signal) (37 \"F.SilkS\" user))
	(footprint \"R\" (layer \"F.Cu\") (at 1 1) (property \"Reference\" \"R1\"))
	(footprint \"R\" (layer \"F.Cu\") (at 2 2) (property \"Reference\" \"R2\"))
	(segment (start 0 0) (end 1 0) (width 0.25) (layer \"F.Cu\") (uuid \"a\"))
)";
        let lib = "(footprint \"TestPoint_Pad_D1.0mm\"
	(version 20260206)
	(layer \"F.Cu\")
	(property \"Reference\" \"REF**\" (at 0 -1.5 0) (layer \"F.SilkS\"))
	(pad \"1\" smd circle (at 0 0) (size 1 1) (layers \"F.Cu\"))
)";
        let mut b = Board::parse(src).unwrap();
        // R2 goes and the new part takes its place in the list: saving tells them apart.
        b.apply(Patch::delete(&b, Pick::Footprint(1)));
        let i = b.add_footprint(lib, "TestPoint", Pt { x: 5 * MM, y: 6 * MM }, 90.0).unwrap();
        let Src::Library(text) = &b.footprints[i].src else { panic!("an added part keeps its text") };
        let id = text.lines().find(|l| l.contains("(uuid")).unwrap().trim();
        let r2 = src.lines().find(|l| l.contains("R2")).unwrap();
        let added = format!(
            "\n\t(footprint \"TestPoint:TestPoint_Pad_D1.0mm\"\n\t\t(layer \"F.Cu\")\n\t\t{id}\n\t\t(at 5 6 90)
\t\t(property \"Reference\" \"TP1\" (at 0 -1.5 90) (layer \"F.SilkS\"))\n\t\t(pad \"1\" smd circle (at 0 0 90) (size 1 1) (layers \"F.Cu\"))\n\t)"
        );
        let out = b.write(src).unwrap();
        assert_eq!(out, src.replace(&format!("\n{r2}"), &added));
        // Read back, it is the part added.
        let back = Board::parse(&out).unwrap();
        let (f, g) = (&back.footprints[1], &b.footprints[1]);
        assert_eq!((&f.reference, &f.lib_id, f.at, f.angle, f.pads.clone()), (&g.reference, &g.lib_id, g.at, g.angle, g.pads.clone()));
        assert_eq!((back.pads, back.texts), (b.pads, b.texts));
    }

    #[test]
    fn writes_the_nets_the_board_renamed_added_and_gave_pads() {
        // Up to KiCad 9 nets are declared and numbered (a pad's by number alone too), from KiCad
        // 10 named where used.
        let kicad7 = r#"(kicad_pcb (version 20221018) (layers (0 "F.Cu" signal))
  (net 0 "")
  (net 1 "A")
  (net 2 "B")
  (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "R1" (at 0 0 0) (layer "F.Cu"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 2 "B")))
  (footprint "R" (layer "F.Cu") (at 5 0) (property "Reference" "R2" (at 0 0 0) (layer "F.Cu"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 2)))
  (segment (start 0 0) (end 5 0) (width 0.25) (layer "F.Cu") (net 1) (tstamp 1))
  (zone (net 1) (net_name "A") (layer "F.Cu") (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))
)"#;
        let kicad7_saved = r#"(kicad_pcb (version 20221018) (layers (0 "F.Cu" signal))
  (net 0 "")
  (net 1 "SDA")
  (net 2 "B")
  (net 3 "N")
  (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "R1" (at 0 0 0) (layer "F.Cu"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "SDA"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 3 "N")))
  (footprint "R" (layer "F.Cu") (at 5 0) (property "Reference" "R2" (at 0 0 0) (layer "F.Cu"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "SDA"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))
  (segment (start 0 0) (end 5 0) (width 0.25) (layer "F.Cu") (net 1) (tstamp 1))
  (zone (net 1) (net_name "SDA") (layer "F.Cu") (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))
)"#;
        let kicad10 = r#"(kicad_pcb (version 20260206) (layers (0 "F.Cu" signal))
  (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "R1" (at 0 0 0) (layer "F.Cu"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "A"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "B")))
  (footprint "R" (layer "F.Cu") (at 5 0) (property "Reference" "R2" (at 0 0 0) (layer "F.Cu"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "B")))
  (segment (start 0 0) (end 5 0) (width 0.25) (layer "F.Cu") (net "A") (uuid "1"))
  (zone (net "A") (layer "F.Cu") (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))
)"#;
        let kicad10_saved = kicad10.replace(r#"(net "A")"#, r#"(net "SDA")"#).replacen(r#"(net "B")"#, r#"(net "N")"#, 1).replacen(
            r#"(layers "F.Cu"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net "B")))"#,
            r#"(layers "F.Cu") (net "SDA"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))"#,
            1,
        );
        for (src, saved) in [(kicad7, kicad7_saved), (kicad10, kicad10_saved.as_str())] {
            let mut b = Board::parse(src).unwrap();
            // A renamed SDA, with its track and zone; R1.2 on a new net, R2.1 on SDA, R2.2 on none.
            let id = |b: &Board, name: &str| b.nets.iter().position(|n| n == name).unwrap() as NetId;
            let (a, n) = (id(&b, "A"), b.nets.len() as NetId);
            b.nets[a as usize] = "SDA".into();
            b.nets.push("N".into());
            (b.pads[1].net, b.pads[2].net, b.pads[3].net) = (n, a, 0);
            let out = b.write(src).unwrap();
            assert_eq!(out, saved);
            let back = Board::parse(&out).unwrap();
            let names = |b: &Board| {
                let name = |id: NetId| b.nets[id as usize].clone();
                (b.pads.iter().map(|p| name(p.net)).collect::<Vec<_>>(), name(b.tracks[0].net), name(b.zones[0].net))
            };
            assert_eq!(names(&back), names(&b));
        }
    }
}
