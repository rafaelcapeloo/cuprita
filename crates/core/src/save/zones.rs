//! Saving zones: new ones, with their settings, outline and fills, edited ones' settings and
//! outline, and refilled ones' fills, as the file's KiCad version writes them (`save-zones.md`).

use super::{Edit, KICAD6, KICAD8, list, quote, uuid, uuid_text};
use crate::board::{Board, Connect, LayerId, NetId, Nm, Pt, Zone};
use crate::parse::THIN_FILLS;
use crate::polygon::grown;
use crate::sexpr::{Cursor, Result};
use crate::units::nm_to_mm;
use std::collections::HashMap;
use std::fmt::Write;

impl Board {
    /// The zones at the indices `zones`, as KiCad of file `version` writes them; `codes` numbers
    /// the nets in files before KiCad 10. `salt` keeps the ids of zones added in different
    /// sessions apart.
    pub(super) fn zones_text(
        &self,
        version: u32,
        codes: &HashMap<NetId, u32>,
        zones: impl Iterator<Item = usize>,
        salt: usize,
    ) -> std::result::Result<String, &'static str> {
        let layer = |l: u8| quote(&self.layers[l as usize], version);
        // A zone's settings: up to KiCad 7 on four or five lines, from KiCad 8 a line per list
        // (`list`); then its outline and fills.
        let list = |head: &str, items: &[String]| list(head, items, version);
        let mut new_zones = String::new();
        for (i, z) in zones.map(|i| (i, &self.zones[i])) {
            let mut head = vec![];
            head.extend(self.net_field(z.net, codes)?);
            if !codes.is_empty() {
                head.push(format!("(net_name {})", quote(&self.nets[z.net as usize], version)));
            }
            head.push(format!("(layer {})", layer(z.layers.trailing_zeros() as LayerId)));
            // Id 0: the board's zones have none (KiCad's always do), so it gets one made from it.
            let id = if z.id == 0 { uuid((salt, i, &z.outline)) } else { uuid_text(z.id) };
            match version {
                KICAD8.. => head.push(format!("(uuid \"{id}\")")),
                KICAD6.. => head.push(format!("(tstamp {id})")),
                _ if z.id != 0 => head.push(format!("(tstamp {:08X})", z.id)),
                _ => {}
            }
            if !z.name.is_empty() {
                head.push(format!("(name {})", quote(&z.name, version)));
            }
            head.push("(hatch edge 0.5)".into());
            let mut width = vec![format!("(min_thickness {})", nm_to_mm(z.min_width))];
            if (KICAD6..THIN_FILLS).contains(&version) {
                width.push("(filled_areas_thickness no)".into());
            }
            let fill =
                [format!("(thermal_gap {})", nm_to_mm(z.thermal_gap)), format!("(thermal_bridge_width {})", nm_to_mm(z.spoke_width))];
            let lines = [
                head,
                (z.priority > 0).then(|| format!("(priority {})", z.priority)).into_iter().collect(),
                vec![list(&format!("connect_pads{}", connect_kind(z.connect)), &[format!("(clearance {})", nm_to_mm(z.clearance))])],
                width,
                vec![list("fill yes", &fill)],
            ];
            new_zones += &match version {
                KICAD8.. => format!("\n\t(zone\n\t\t{}", lines.concat().join("\n\t\t")),
                _ => {
                    format!("\n  (zone {}", lines.iter().filter(|l| !l.is_empty()).map(|l| l.join(" ")).collect::<Vec<_>>().join("\n    "))
                }
            };
            z.outline.iter().for_each(|pts| polygon(&mut new_zones, "polygon", pts, None, version));
            new_zones += &self.fills_text(z, version);
            new_zones += if version >= KICAD8 { "\n\t)" } else { "\n  )" };
        }
        Ok(new_zones)
    }

    /// Writes zone `z`'s settings and outline (with its cutouts) over those of `read`, the file's
    /// zone (`c` just inside it), where they differ, as KiCad of file `version` writes them; the
    /// rest of its text stays. `net` is the board's net of `read`'s; `codes` numbers the nets in
    /// files before KiCad 10.
    pub(super) fn zone_edits(
        &self,
        c: &mut Cursor,
        (z, read, net): (&Zone, &Zone, NetId),
        version: u32,
        codes: &HashMap<NetId, u32>,
        edits: &mut Vec<Edit>,
    ) -> Result<()> {
        // The file's settings: where each value is, each list with the text before it and where
        // it closes; where the lists it lacks would go, after its ids (net, layer, uuid) and its
        // head (name, hatch); its outline and cutouts.
        let start = c.span().end;
        let (mut end, mut ids, mut head, mut kind, mut polygons) = (start, start, start, None, vec![]);
        let (mut at, mut lists, mut ends) = (HashMap::new(), HashMap::new(), HashMap::new());
        while let Some(key) = c.child()? {
            match key {
                "net" | "net_name" | "name" | "priority" | "min_thickness" => {
                    c.atom()?;
                    at.insert(key, c.span());
                }
                "connect_pads" | "fill" => {
                    let after = c.span().end;
                    if key == "connect_pads" {
                        kind = Some(after..c.opt_atom()?.map_or(after, |_| c.span().end));
                    }
                    while let Some(k) = c.child()? {
                        if matches!(k, "clearance" | "thermal_gap" | "thermal_bridge_width") {
                            c.atom()?;
                            at.insert(k, c.span());
                        }
                        c.close()?;
                    }
                }
                _ => {}
            }
            c.close()?;
            ends.insert(key, c.span().start);
            let list = std::mem::replace(&mut end, c.span().end)..end;
            match key {
                "net" | "net_name" | "layer" | "layers" | "uuid" | "tstamp" => (ids, head) = (end, end),
                "name" | "hatch" => head = end,
                "polygon" => polygons.push(list.clone()),
                _ => {}
            }
            lists.insert(key, list);
        }
        let sep = if version >= KICAD8 { "\n\t\t" } else { "\n    " };
        // A setting's value where the file has it, else its list put in at `to`; an optional one's
        // list taken out for none.
        let set = |key: &str, value: &str, to: usize, list: String| match at.get(key) {
            Some(r) => (r.clone(), value.to_string()),
            None => (to..to, list),
        };
        let opt = |key: &str, none: bool, value: &str, to: usize, list: String| {
            if none { lists.get(key).map(|l| (l.clone(), String::new())) } else { Some(set(key, value, to, list)) }
        };
        if net != z.net {
            let name = quote(&self.nets[z.net as usize], version);
            match codes.get(&z.net) {
                Some(code) => {
                    edits.push(set("net", &code.to_string(), start, format!("{sep}(net {code})")));
                    edits.push(set("net_name", &name, ids, format!(" (net_name {name})")));
                }
                None => edits.extend(opt("net", z.net == 0, &name, start, format!("{sep}(net {name})"))),
            }
        }
        if z.name != read.name {
            let name = quote(&z.name, version);
            let to = format!("{}(name {name})", if version >= KICAD8 { sep } else { " " });
            edits.extend(opt("name", z.name.is_empty(), &name, ids, to));
        }
        if z.priority != read.priority {
            let p = z.priority.to_string();
            edits.extend(opt("priority", z.priority == 0, &p, head, format!("{sep}(priority {p})")));
        }
        let mm = |v: Nm| nm_to_mm(v);
        let clearance = mm(z.clearance);
        match kind.zip(ends.get("connect_pads")) {
            Some((kind, &close)) => {
                if z.connect != read.connect {
                    edits.push((kind, connect_kind(z.connect).into()));
                }
                if z.clearance != read.clearance {
                    edits.push(set("clearance", &clearance, close, format!(" (clearance {clearance})")));
                }
            }
            None if z.connect != read.connect || z.clearance != read.clearance => {
                edits.push((head..head, format!("{sep}(connect_pads{} (clearance {clearance}))", connect_kind(z.connect))));
            }
            None => {}
        }
        if z.min_width != read.min_width {
            let w = mm(z.min_width);
            edits.push(set("min_thickness", &w, head, format!("{sep}(min_thickness {w})")));
        }
        let fill = [("thermal_gap", z.thermal_gap, read.thermal_gap), ("thermal_bridge_width", z.spoke_width, read.spoke_width)];
        let changed: Vec<_> = fill.iter().filter(|(_, v, was)| v != was).collect();
        match ends.get("fill") {
            Some(&close) => edits.extend(changed.iter().map(|&&(key, v, _)| set(key, &mm(v), close, format!(" ({key} {})", mm(v))))),
            None if !changed.is_empty() => {
                edits.push((
                    head..head,
                    format!("{sep}(fill (thermal_gap {}) (thermal_bridge_width {}))", mm(z.thermal_gap), mm(z.spoke_width)),
                ));
            }
            None => {}
        }
        if z.outline != read.outline {
            let mut text = String::new();
            z.outline.iter().for_each(|pts| polygon(&mut text, "polygon", pts, None, version));
            let mut old = polygons.into_iter();
            edits.push((old.next().unwrap_or(end..end), text));
            edits.extend(old.map(|span| (span, String::new())));
        }
        Ok(())
    }

    /// Writes the board's fills of zone `z` over the zone's filled areas in the file (`c` just
    /// inside the zone), as KiCad of file `version` writes them.
    // shortcut: no `(island)` marks (KiCad 6 on mark the islands a zone keeps) and KiCad 5's
    // `fill_segments` stay; write or drop them when a board needs it.
    pub(super) fn zone_fills(&self, c: &mut Cursor, z: &Zone, version: u32, edits: &mut Vec<Edit>) -> Result<()> {
        // The file's filled areas, each with the text before it; new ones go after the outline.
        let (mut end, mut at, mut old) = (c.span().end, None, vec![]);
        while let Some(head) = c.child()? {
            c.close()?;
            let start = std::mem::replace(&mut end, c.span().end);
            match head {
                "polygon" => at = Some(end),
                "filled_polygon" => old.push(start..end),
                _ => {}
            }
        }
        let mut old = old.into_iter();
        let at = at.unwrap_or(end);
        edits.push((old.next().unwrap_or(at..at), self.fills_text(z, version)));
        edits.extend(old.map(|span| (span, String::new())));
        Ok(())
    }

    /// Zone `z`'s fills as KiCad of file `version` writes them.
    fn fills_text(&self, z: &Zone, version: u32) -> String {
        // Fills KiCad draws with an outline as wide as the minimum width: saved shrunk by what they
        // were grown by (half of it less 1 nm, `fill.rs`), so channels that wide stay.
        let by = z.min_width / 2 - 1;
        let mut text = String::new();
        for f in &self.fills[z.fills.clone()] {
            let parts = if z.outlined && by > 0 { grown(&f.pts, -by) } else { vec![f.pts.clone()] };
            parts.iter().for_each(|pts| polygon(&mut text, "filled_polygon", pts, Some(&self.layers[f.layer as usize]), version));
        }
        text
    }
}

/// What `(connect_pads …)` says before its clearance for pads joined `c`: nothing for thermal
/// reliefs, KiCad's default.
fn connect_kind(c: Connect) -> &'static str {
    match c {
        Connect::Thermal => "",
        Connect::Solid => " yes",
        Connect::None => " no",
        Connect::ThruHoleThermal => " thru_hole_only",
    }
}

/// A zone's outline (`polygon`) or filled area (`filled_polygon`, on `layer`) as KiCad of file
/// `version` writes it: up to KiCad 5 five points a line; KiCad 6 and 7 name a fill's layer and
/// put one; from KiCad 8, tab indents and a new line once one is 99 characters long.
fn polygon(out: &mut String, head: &str, pts: &[Pt], layer: Option<&str>, version: u32) {
    let tab = if version >= KICAD8 { "\t" } else { "  " };
    let (i2, i3, i4) = (tab.repeat(2), tab.repeat(3), tab.repeat(4));
    *out += &format!("\n{i2}({head}");
    if let Some(layer) = layer.filter(|_| version >= KICAD6) {
        *out += &format!("\n{i3}(layer \"{layer}\")");
    }
    *out += &format!("\n{i3}(pts");
    let (per_line, width) = match version {
        KICAD8.. => (usize::MAX, 99),
        KICAD6..KICAD8 => (1, usize::MAX),
        _ => (5, usize::MAX),
    };
    let (mut line, mut n) = (0, 0);
    for (k, p) in pts.iter().enumerate() {
        if k == 0 || n == per_line || out.len() - line >= width {
            *out += "\n";
            (line, n) = (out.len(), 0);
            *out += &i4;
        } else {
            out.push(' ');
        }
        _ = write!(out, "(xy {} {})", nm_to_mm(p.x), nm_to_mm(p.y));
        n += 1;
    }
    *out += &format!("\n{i3})\n{i2})");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{MM, ZoneFill};
    use crate::history::{Patch, Pick};
    use crate::save::MISMATCH;
    use std::f64::consts::PI;

    #[test]
    fn writes_added_zones_as_the_files_kicad_version_does() {
        for (version, l, nets, net, id, new_id) in [
            ("20171130", "F.Cu", r#"(net 0 "") (net 1 GND)"#, "(net 1) (net_name GND)", "(tstamp 5FDEC726)", "5FDEC725"),
            (
                "20221018",
                r#""F.Cu""#,
                r#"(net 0 "") (net 1 "GND")"#,
                r#"(net 1) (net_name "GND")"#,
                "(tstamp 00000000-0000-0000-0000-00005fdec726)",
                "00000000-0000-0000-0000-00005fdec725",
            ),
            ("20240108", r#""F.Cu""#, r#"(net 0 "") (net 1 "GND")"#, r#"(net 1) (net_name "GND")"#, "", ""),
            (
                "20260206",
                r#""F.Cu""#,
                "",
                r#"(net "GND")"#,
                r#"(uuid "29ccb441-0479-4f07-8c20-b90754f4e2a4")"#,
                "29ccb441-0479-4f07-8c20-b90754f4e2a3",
            ),
        ] {
            let src = format!(
                "(kicad_pcb (version {version}) (layers (0 {l} signal)) {nets}\n  (zone {net} (layer {l}) {id} (polygon (pts (xy 20 0) (xy 30 0) (xy 30 10)))))"
            );
            let mut b = Board::parse(&src).unwrap();
            let pt = |x, y| Pt { x: x * MM, y: y * MM };
            b.add_zone(vec![pt(0, 20), pt(10, 20), pt(10, 30), pt(0, 30)], 0, 1);
            // A new zone's id is one below the least, so it ranks last; one made up when the zones have none.
            let id = if id.is_empty() { uuid((src.len(), 1usize, &b.zones[1].outline)) } else { new_id.into() };
            let fills = b.fills_text(&b.zones[1], version.parse().unwrap());
            let corners = ["(xy 0 20)", "(xy 10 20)", "(xy 10 30)", "(xy 0 30)"];
            let (connect, fill) = ("(connect_pads (clearance 0.5))", "(fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5))");
            let zone = match version {
                // KiCad 5: bare names, an 8-digit `tstamp`, five points a line; KiCad 6 and 7: quoted
                // names, a UUID `tstamp`, the fills' outline left out and a point a line.
                "20171130" => format!(
                    "\n  (zone {net} (layer {l}) (tstamp {id}) (hatch edge 0.5)\n    {connect}\n    (min_thickness 0.25)\n    {fill}\n    (polygon\n      (pts\n        {}\n      )\n    ){fills}\n  )",
                    corners.join(" ")
                ),
                "20221018" => format!(
                    "\n  (zone {net} (layer {l}) (tstamp {id}) (hatch edge 0.5)\n    {connect}\n    (min_thickness 0.25) (filled_areas_thickness no)\n    {fill}\n    (polygon\n      (pts\n        {}\n      )\n    ){fills}\n  )",
                    corners.join("\n        ")
                ),
                // KiCad 8 on: a line per list; KiCad 10 names the net, and its fills have no outline.
                _ => format!(
                    "\n\t(zone\n\t\t{}\n\t\t(layer {l})\n\t\t(uuid \"{id}\")\n\t\t(hatch edge 0.5)\n\t\t(connect_pads\n\t\t\t(clearance 0.5)\n\t\t)\n\t\t(min_thickness 0.25){}\n\t\t(fill yes\n\t\t\t(thermal_gap 0.5)\n\t\t\t(thermal_bridge_width 0.5)\n\t\t)\n\t\t(polygon\n\t\t\t(pts\n\t\t\t\t{}\n\t\t\t)\n\t\t){fills}\n\t)",
                    net.replace(") (", ")\n\t\t("),
                    if version == "20240108" { "\n\t\t(filled_areas_thickness no)" } else { "" },
                    corners.join(" ")
                ),
            };
            let out = b.write(&src).unwrap();
            assert_eq!(out, format!("{}{zone})", &src[..src.len() - 1]), "{version}");
            // Read back, it is the same zone with the id written; KiCad 5's fill was saved shrunk,
            // to be drawn grown.
            let back = Board::parse(&out).unwrap();
            let mut zones = b.zones.clone();
            (zones[1].id, zones[1].src) = (u128::from_str_radix(&id.replace('-', ""), 16).unwrap(), Some(1));
            assert_eq!(back.zones, zones, "{version}");
            assert!(version == "20171130" || back.fills == b.fills, "{version}");
            // A board with a zone in a place the text lacks wasn't read from it.
            b.zones[0].src = Some(1);
            assert_eq!(b.write(&src).map_err(|e| e.msg), Err(MISMATCH));
        }
    }

    #[test]
    fn writes_edited_outlines_with_their_cutouts_as_the_files_kicad_version_does() {
        let pt = |x, y| Pt { x: x * MM, y: y * MM };
        for (version, net) in
            [(20171130, "(net 1) (net_name GND)"), (20221018, r#"(net 1) (net_name "GND")"#), (20260206, r#"(net "GND")"#)]
        {
            // A zone with two stale fills, and one after it whose fills move along with them.
            let src = format!(
                r#"(kicad_pcb (version {version}) (layers (0 "F.Cu" signal)) (net 0 "") (net 1 "GND")
  (zone {net} (layer "F.Cu") (min_thickness 0)
    (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))
    (filled_polygon (layer "F.Cu") (pts (xy 0 0) (xy 1 0) (xy 1 1))) (filled_polygon (layer "F.Cu") (pts (xy 2 0) (xy 3 0) (xy 3 1))))
  (zone {net} (layer "F.Cu") (min_thickness 0) (polygon (pts (xy 20 0) (xy 30 0) (xy 30 10)))
    (filled_polygon (layer "F.Cu") (pts (xy 20 0) (xy 30 0) (xy 30 10)))))"#
            );
            let (orig, mut b) = (Board::parse(&src).unwrap(), Board::parse(&src).unwrap());
            let mut z = b.zones[0].clone();
            z.move_corner(0, 2, pt(12, 12)).unwrap();
            z.add_cutout(vec![pt(2, 2), pt(4, 2), pt(4, 4)]).unwrap();
            let undo = b.set_zone(0, z);
            // Refilled alone: one fill, holed by the cutout; the next zone's fill moves up.
            assert_eq!((b.zones[0].fills.clone(), b.zones[1].fills.clone(), &b.fills[1]), (0..1, 1..2, &orig.fills[2]));
            let out = b.write(&src).unwrap();
            // The outline, then the cutout, where the file's outline was; the next zone as it was.
            let t = if version >= KICAD8 { "\t" } else { "  " };
            let sep = if (KICAD6..KICAD8).contains(&version) { format!("\n{}", t.repeat(4)) } else { " ".into() };
            let polygon = |p: &[&str]| {
                let pts = p.iter().map(|xy| format!("(xy {xy})")).collect::<Vec<_>>().join(&sep);
                format!("\n{t}{t}(polygon\n{t}{t}{t}(pts\n{t}{t}{t}{t}{pts}\n{t}{t}{t})\n{t}{t})")
            };
            let outline = polygon(&["0 0", "10 0", "12 12", "0 10"]) + &polygon(&["2 2", "4 2", "4 4"]);
            let head = &src[..src.find("\n    (polygon").unwrap()];
            assert!(out.starts_with(&format!("{head}{outline}\n")), "{version}:\n{out}");
            assert!(out.ends_with(&src[src.rfind("\n  (zone").unwrap()..]), "{version}");
            let back = Board::parse(&out).unwrap();
            assert_eq!((&back.zones[0].outline, &back.fills), (&b.zones[0].outline, &b.fills), "{version}");
            // Undone, the board and its file are as they were.
            b.apply(undo);
            assert_eq!((&b.zones, &b.fills, b.write(&src).unwrap()), (&orig.zones, &orig.fills, src.clone()));
        }
    }

    #[test]
    fn writes_edited_settings_in_place_as_the_files_kicad_version_does() {
        // A zone as KiCad 7 and 10 write one with their defaults, then on net B, named, ranked 2,
        // its through-hole pads relieved, with other lengths: KiCad 10's own text for those.
        let id = "29ccb441-0479-4f07-8c20-b90754f4e2a4";
        let kicad7 = |net: &str, name: &str, priority: &str, kind: &str, [c, w, g, s]: [&str; 4]| {
            format!(
                "(kicad_pcb (version 20221018) (layers (0 \"F.Cu\" signal)) (net 0 \"\") (net 1 \"A\") (net 2 \"B\")\n  (zone {net} (layer \"F.Cu\") (tstamp {id}){name} (hatch edge 0.5){priority}\n    (connect_pads{kind} (clearance {c}))\n    (min_thickness {w}) (filled_areas_thickness no)\n    (fill yes (thermal_gap {g}) (thermal_bridge_width {s}))\n    (polygon\n      (pts\n        (xy 0 0)\n        (xy 10 0)\n        (xy 10 10)\n      )\n    )\n  )\n)"
            )
        };
        // KiCad 10 numbers nets where first used: tracks of A and B come first.
        let kicad10 = |net: &str, name: &str, priority: &str, kind: &str, [c, w, g, s]: [&str; 4]| {
            format!(
                "(kicad_pcb\n\t(version 20260206)\n\t(layers\n\t\t(0 \"F.Cu\" signal)\n\t)\n\t(segment (start 0 0) (end 1 0) (width 0.2) (layer \"F.Cu\") (net \"A\"))\n\t(segment (start 0 1) (end 1 1) (width 0.2) (layer \"F.Cu\") (net \"B\"))\n\t(zone{net}\n\t\t(layer \"F.Cu\")\n\t\t(uuid \"{id}\"){name}\n\t\t(hatch edge 0.5){priority}\n\t\t(connect_pads{kind}\n\t\t\t(clearance {c})\n\t\t)\n\t\t(min_thickness {w})\n\t\t(fill yes\n\t\t\t(thermal_gap {g})\n\t\t\t(thermal_bridge_width {s})\n\t\t)\n\t\t(polygon\n\t\t\t(pts\n\t\t\t\t(xy 0 0) (xy 10 0) (xy 10 10)\n\t\t\t)\n\t\t)\n\t)\n)"
            )
        };
        let (defaults, edited) = (["0.5", "0.25", "0.5", "0.5"], ["0.3", "0.2", "0.4", "0.6"]);
        let pairs = [
            (
                kicad7("(net 1) (net_name \"A\")", "", "", "", defaults),
                kicad7("(net 2) (net_name \"B\")", " (name \"Plane A\")", "\n    (priority 2)", " thru_hole_only", edited),
            ),
            (
                kicad10("\n\t\t(net \"A\")", "", "", "", defaults),
                kicad10("\n\t\t(net \"B\")", "\n\t\t(name \"Plane A\")", "\n\t\t(priority 2)", " thru_hole_only", edited),
            ),
            // KiCad 10 leaves out the net of a zone on none, and a solid connection says yes.
            (kicad10("", "", "", "", defaults), kicad10("\n\t\t(net \"B\")", "", "", " yes", defaults)),
        ];
        // Each way, one's settings written over the other's text give the other's.
        for (plain, set) in pairs {
            for (from, to) in [(&plain, &set), (&set, &plain)] {
                let (mut b, goal) = (Board::parse(from).unwrap(), Board::parse(to).unwrap());
                b.zones[0] = goal.zones[0].clone();
                assert_eq!(b.write(from).unwrap(), *to);
            }
            // A zone added, then given them, reads back with them.
            let (mut b, s) = (Board::parse(&plain).unwrap(), Board::parse(&set).unwrap().zones.remove(0));
            let i = b.add_zone(s.outline[0].clone(), 0, s.net);
            let z = &mut b.zones[i];
            (z.name, z.priority, z.connect, z.clearance, z.min_width) = (s.name, s.priority, s.connect, s.clearance, s.min_width);
            (z.thermal_gap, z.spoke_width) = (s.thermal_gap, s.spoke_width);
            let back = Board::parse(&b.write(&plain).unwrap()).unwrap();
            assert_eq!(back.zones[1], Zone { src: Some(1), ..b.zones[i].clone() });
        }
    }

    #[test]
    fn deletes_zones_with_their_fills_and_nothing_else() {
        // A zone of each net with a fill, a rule area letting copper in (not read) and a keepout.
        let (a, allowed, b_zone, keepout) = (
            "\n  (zone (net \"A\") (layer \"F.Cu\") (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1)))\n    (filled_polygon (layer \"F.Cu\") (pts (xy 0 0) (xy 1 0) (xy 1 1))))",
            "\n  (zone (layer \"F.Cu\") (keepout (copperpour allowed)) (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))",
            "\n  (zone (net \"B\") (layer \"B.Cu\") (polygon (pts (xy 2 0) (xy 3 0) (xy 3 1)))\n    (filled_polygon (layer \"B.Cu\") (pts (xy 2 0) (xy 3 0) (xy 3 1))))",
            "\n  (zone (layer \"F.Cu\") (keepout (copperpour not_allowed)) (polygon (pts (xy 5 0) (xy 6 0) (xy 6 1))))",
        );
        let head = r#"(kicad_pcb (version 20260206) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
  (segment (start 0 0) (end 1 0) (width 0.25) (layer "F.Cu") (net "A"))"#;
        let src = format!("{head}{a}{allowed}{b_zone}{keepout})");
        let (orig, mut b) = (Board::parse(&src).unwrap(), Board::parse(&src).unwrap());
        // B goes with its text and fill; the keepout after it keeps its place in the file.
        let undo = b.apply(Patch::delete(&b, Pick::Zone(1)));
        assert_eq!((b.zones.len(), b.fills.len(), b.zones[1].fills.clone(), b.zones[1].src), (2, 1, 1..1, Some(2)));
        let out = b.write(&src).unwrap();
        assert_eq!(out, format!("{head}{a}{allowed}{keepout})"));
        let back = Board::parse(&out).unwrap();
        let shapes = |zones: &[Zone]| zones.iter().map(|z| (z.outline.clone(), z.net == 0, z.fills.clone())).collect::<Vec<_>>();
        assert_eq!((shapes(&back.zones), &back.fills), (shapes(&b.zones), &b.fills));
        // Undone, it is as it was; then every zone goes, the fills after each moving up.
        b.apply(undo);
        assert_eq!((&b.zones, &b.fills, b.write(&src).unwrap()), (&orig.zones, &orig.fills, src.clone()));
        b.apply(Patch::delete(&b, Pick::Zone(0)));
        assert_eq!((b.zones[0].fills.clone(), &b.fills[..]), (0..1, &orig.fills[1..]));
        b.apply(Patch::delete(&b, Pick::Zone(0)));
        b.apply(Patch::delete(&b, Pick::Zone(0)));
        assert_eq!((b.zones.len(), b.fills.len(), b.write(&src).unwrap()), (0, 0, format!("{head}{allowed})")));
        // A track added goes after the last, where the zone after it went.
        b.add_tracks(&[Pt { x: MM, y: 0 }, Pt { x: 2 * MM, y: 0 }], 250_000, 0, b.find_net("A").unwrap());
        let back = Board::parse(&b.write(&src).unwrap()).unwrap();
        assert_eq!((back.tracks, back.zones.len()), (b.tracks, 0));
    }

    #[test]
    fn writes_refilled_zones_as_the_files_kicad_version_does() {
        let pts = "(xy 100.5 120) (xy 95.123456 110.654321) (xy 100 100) (xy 110.000001 100) (xy 120 105.5) (xy 120 110.25) (xy 110 120)";
        let xy: Vec<&str> = pts.split_inclusive(") ").map(str::trim).collect();
        let stale = "\n    (filled_polygon (layer \"F.Cu\") (pts (xy 1 1) (xy 2 1) (xy 2 2)))\n    (filled_polygon (pts (xy 3 3) (xy 4 3) (xy 4 4)))";
        for version in [4, 20171130, 20221018, 20240108, 20260206] {
            // A rule area the parser skips, a zone with stale fills and one never filled; with no
            // minimum width and nothing around, a fill is the outline.
            let src = format!(
                r#"(kicad_pcb (version {version}) (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") (net 1 "GND")
  (zone (net 0) (layer "F.Cu") (keepout (copperpour allowed)) (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))
  (zone (net 1) (layer "F.Cu") (min_thickness 0) (polygon (pts {pts})){stale})
  (zone (net 1) (layer "B.Cu") (min_thickness 0) (polygon (pts {pts}))))"#
            );
            let mut b = Board::parse(&src).unwrap();
            assert_eq!(b.write(&src).unwrap(), src);
            b.fill_zones();
            // KiCad 5 puts five points a line; KiCad 6 and 7 name the layer and put one; KiCad 8
            // indents with tabs and ends a line once past column 98, here after six points.
            let per_line = match version {
                ..KICAD6 => 5,
                KICAD6..KICAD8 => 1,
                _ => 6,
            };
            let t = if version >= KICAD8 { "\t" } else { "  " };
            let fill = |l: &str| {
                let layer = if version >= KICAD6 { format!("\n{t}{t}{t}(layer \"{l}\")") } else { String::new() };
                let pts: String = xy.chunks(per_line).map(|line| format!("\n{t}{t}{t}{t}{}", line.join(" "))).collect();
                format!("\n{t}{t}(filled_polygon{layer}\n{t}{t}{t}(pts{pts}\n{t}{t}{t})\n{t}{t})")
            };
            let want = src.replace(stale, &fill("F.Cu")).replace(&format!("{pts}))))"), &format!("{pts})){}))", fill("B.Cu")));
            let out = b.write(&src).unwrap();
            assert_eq!(out, want, "{version}");
            assert_eq!(Board::parse(&out).unwrap().fills, b.fills, "{version}");
        }
    }

    #[test]
    fn reads_and_saves_the_fills_kicad_draws_with_an_outline() {
        // Bounds and area (mm, mm²) of a zone's one fill.
        fn drawn(fills: &[ZoneFill]) -> ([f64; 4], f64) {
            let [f] = fills else { panic!("{fills:?}") };
            let (x, y) = (f.pts.iter().map(|p| p.x), f.pts.iter().map(|p| p.y));
            let bounds = [x.clone().min(), y.clone().min(), x.max(), y.max()].map(|v| v.unwrap() as f64 / 1e6);
            let area = f.pts.iter().zip(f.pts.iter().cycle().skip(1)).map(|(a, c)| a.x as f64 * c.y as f64 - c.x as f64 * a.y as f64);
            (bounds, area.sum::<f64>().abs() / 2e12)
        }
        let near = |(b, a): ([f64; 4], f64), (want_b, want_a): ([f64; 4], f64)| {
            b.iter().zip(want_b).all(|(v, w)| (v - w).abs() < 1e-5) && (a - want_a).abs() < 0.02
        };
        // A 9.5 mm square drawn with a 0.5 mm wide outline: a 10 mm square, corners rounded 0.25 mm
        // (arcs drawn within 5 µm, twice when saved and read back: 0.012 mm² short).
        let (square, rounded) = (([0.25, 0.25, 9.75, 9.75], 90.25), ([0.0, 0.0, 10.0, 10.0], 100.0 - (4.0 - PI) / 16.0));
        // KiCad 5 draws every fill with an outline as wide as the minimum width; later versions
        // the fills of zones that say so, or (in files before KiCad 10's) say nothing.
        for (version, says, outlined) in [
            (20171130, "", true),
            (20221018, "(filled_areas_thickness yes)", true),
            (20240108, "", true),
            (20240108, "(filled_areas_thickness no)", false),
            (20260206, "", false),
        ] {
            let src = format!(
                r#"(kicad_pcb (version {version}) (layers (0 "F.Cu" signal)) (net 0 "") (net 1 "GND")
  (zone (net 1) (layer "F.Cu") (min_thickness 0.5) {says} (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))
    (filled_polygon (pts (xy 0.25 0.25) (xy 9.75 0.25) (xy 9.75 9.75) (xy 0.25 9.75)))))"#
            );
            let mut b = Board::parse(&src).unwrap();
            let read = drawn(&b.fills);
            assert!(near(read, if outlined { rounded } else { square }), "{version} {says}: read {read:?}");
            // Refilled, it is the rounded square; saved, it reads back as that.
            b.fill_zones();
            let back = Board::parse(&b.write(&src).unwrap()).unwrap();
            assert!(near(drawn(&back.fills), rounded) && (outlined || back.fills == b.fills), "{version} {says}: {:?}", drawn(&back.fills));
        }
    }
}
