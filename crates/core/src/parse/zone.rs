//! Copper zones with their fills, and the rule areas that keep zone copper out.

use super::{Parser, THIN_FILLS, pts_into};
use crate::board::{Connect, Smoothing, Zone, ZoneFill};
use crate::polygon::grown;
use crate::sexpr::{Cursor, Result};
use crate::tr::tr;

impl<'a> Parser<'a> {
    /// A copper zone, or a rule area when it keeps zone copper out (other rule areas are skipped).
    pub(crate) fn zone(&mut self, c: &mut Cursor<'a>) -> Result<()> {
        let fills0 = self.b.fills.len();
        let mut z = Zone {
            outline: vec![],
            layers: 0,
            net: 0,
            priority: 0,
            id: 0,
            keepout: false,
            name: String::new(),
            clearance: 0,
            min_width: 0,
            connect: Connect::Thermal,
            thermal_gap: 0,
            spoke_width: 0,
            island_min: i64::MAX,
            smoothing: Smoothing::None,
            hatched: false,
            outlined: false,
            fills: fills0..fills0,
            src: None,
        };
        let (mut zone_layer, mut rule_area, mut smoothing, mut radius, mut islands, mut island_mm2) = (None, false, "", 0, "0", 0.0);
        let mut outlined = None;
        while let Some(head) = c.child()? {
            match head {
                "net" => z.net = self.net(c)?,
                "layer" => {
                    let l = self.layer_at(c)?;
                    (zone_layer, z.layers) = (Some(l), 1 << l);
                }
                "layers" => z.layers = self.layer_list(c)?,
                "priority" => z.priority = c.atom()?.parse().map_err(|_| c.err(tr("bad priority")))?,
                "name" => z.name = c.atom()?.into(),
                "uuid" | "tstamp" => z.id = u128::from_str_radix(&c.atom()?.replace('-', ""), 16).unwrap_or(0),
                "connect_pads" => {
                    z.connect = match c.opt_atom()? {
                        Some("yes" | "full") => Connect::Solid,
                        Some("no") => Connect::None,
                        Some("thru_hole_only") => Connect::ThruHoleThermal,
                        _ => Connect::Thermal,
                    };
                    while let Some(k) = c.child()? {
                        if k == "clearance" {
                            z.clearance = c.nm()?;
                        }
                        c.close()?;
                    }
                }
                "min_thickness" => z.min_width = c.nm()?,
                "filled_areas_thickness" => outlined = Some(c.atom()? == "yes"),
                "keepout" => {
                    rule_area = true;
                    while let Some(k) = c.child()? {
                        z.keepout |= k == "copperpour" && c.atom()? == "not_allowed";
                        c.close()?;
                    }
                }
                "fill" => {
                    while let Some(k) = c.child()? {
                        match k {
                            "mode" => z.hatched = c.atom()? == "hatch",
                            "thermal_gap" => z.thermal_gap = c.nm()?,
                            "thermal_bridge_width" => z.spoke_width = c.nm()?,
                            "smoothing" => smoothing = c.atom()?,
                            "radius" => radius = c.nm()?,
                            "island_removal_mode" => islands = c.atom()?,
                            "island_area_min" => island_mm2 = c.f64()?,
                            _ => {}
                        }
                        c.close()?;
                    }
                }
                "polygon" => {
                    let mut pts = vec![];
                    while let Some(k) = c.child()? {
                        if k == "pts" {
                            pts_into(c, &mut pts)?;
                        }
                        c.close()?;
                    }
                    z.outline.push(pts);
                }
                "filled_polygon" => {
                    let (mut layer, mut pts) = (zone_layer, vec![]);
                    while let Some(head) = c.child()? {
                        match head {
                            "layer" => layer = Some(self.layer_at(c)?),
                            "pts" => pts_into(c, &mut pts)?,
                            _ => {}
                        }
                        c.close()?;
                    }
                    if let (Some(layer), true) = (layer, pts.len() >= 3) {
                        self.b.fills.push(ZoneFill { pts, layer, net: 0 });
                    }
                }
                _ => {}
            }
            c.close()?;
        }
        // Fills KiCad draws with an outline: as drawn, grown by half its width less 1 nm, as
        // `fill_zones` grows them (saving shrinks them back).
        z.outlined = outlined.unwrap_or(self.b.version < THIN_FILLS);
        let by = z.min_width / 2 - 1;
        if z.outlined && by > 0 {
            let thin = self.b.fills.split_off(fills0);
            self.b.fills.extend(thin.iter().flat_map(|f| grown(&f.pts, by).into_iter().map(|pts| ZoneFill { pts, ..*f })));
        }
        for f in &mut self.b.fills[fills0..] {
            f.net = z.net;
        }
        z.fills.end = self.b.fills.len();
        z.smoothing = match smoothing {
            "chamfer" => Smoothing::Chamfer(radius),
            "fillet" => Smoothing::Fillet(radius),
            _ => Smoothing::None,
        };
        z.island_min = match islands {
            "1" => 0,
            "2" => (island_mm2 * 1e12) as i64,
            _ => i64::MAX,
        };
        if !rule_area || z.keepout {
            self.b.zones.push(Zone { src: Some(self.b.zones.len()), ..z });
        } else {
            // A board's fills are its zones', in their order.
            self.b.fills.truncate(fills0);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Board, MM, Pt};

    #[test]
    fn parses_zone_settings_and_rule_areas() {
        let b = Board::parse(
            r#"(kicad_pcb (version 20260206) (layers (0 "F.Cu" signal) (2 "In1.Cu" signal) (31 "B.Cu" signal))
              (zone (net "GND") (layers "F.Cu" "B.Cu") (name "Plane \"A\"") (priority 3) (connect_pads yes (clearance 0.2)) (min_thickness 0.25)
                (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4) (smoothing fillet) (radius 1)
                  (island_removal_mode 2) (island_area_min 1.5))
                (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10))) (polygon (pts (xy 1 1) (xy 2 1) (xy 2 2)))
                (filled_polygon (layer "B.Cu") (pts (xy 0 0) (xy 1 0) (xy 1 1))))
              (zone (net 0) (layers "*.Cu") (keepout (tracks allowed) (copperpour not_allowed)) (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))
              (zone (net 0) (layer "F.Cu") (keepout (tracks not_allowed) (copperpour allowed)) (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))
              (zone (net 1) (net_name GND) (layer B.Cu) (connect_pads (clearance 0.1524)) (min_thickness 0.1524)
                (fill yes (arc_segments 32) (thermal_gap 0.508) (thermal_bridge_width 0.508) (smoothing chamfer) (radius 0.3))
                (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1)))))"#,
        )
        .unwrap();
        // The rule area that lets zone copper in plays no part in filling.
        assert_eq!(b.zones.len(), 3);
        let (z, keepout, legacy) = (&b.zones[0], &b.zones[1], &b.zones[2]);
        assert_eq!((z.layers, z.net, z.priority, z.connect, z.clearance, z.min_width), (0b101, 1, 3, Connect::Solid, 200_000, 250_000));
        assert_eq!((z.thermal_gap, z.spoke_width, z.smoothing, z.island_min), (500_000, 400_000, Smoothing::Fillet(MM), 1_500_000_000_000));
        assert_eq!((z.outline.len(), z.outline[1][0], z.fills.clone(), z.keepout), (2, Pt { x: MM, y: MM }, 0..1, false));
        assert_eq!((z.name.as_str(), legacy.name.as_str()), (r#"Plane \"A\""#, ""));
        assert_eq!((keepout.keepout, keepout.layers, keepout.fills.clone()), (true, 0b111, 1..1));
        assert_eq!((legacy.connect, legacy.smoothing, legacy.island_min), (Connect::Thermal, Smoothing::Chamfer(300_000), i64::MAX));
    }
}
