//! A zone's settings in Inspect, set in place as a part's fields are: its net (a choice of the
//! board's), name, priority, clearance, minimum width, pad connection, thermal gap and spoke
//! width. Each change is an undo step: the zone refills alone, is re-checked, and saves as KiCad
//! writes it (`Board::set_zone`). Lengths show and are typed in the units shown (Ctrl+U), or
//! with their own (`0.2mm`, `8mil`).

use crate::inspect::field_row;
use crate::tr::{t, tr, trf};
use crate::{App, theme, units};
use cuprita_core::board::{Connect, Nm, Zone};
use cuprita_core::history::Pick;
use cuprita_core::sexpr::{escape, unescape};
use eframe::egui::{self, RichText};
use web_time::Instant; // std's panics in the browser

/// The lengths Inspect sets, in `lengths`' order: each one's name, whether it is a width (more
/// than 0), and its edit as the undo step names it (its own text, as some languages capitalise
/// the name there and some don't).
const LENGTHS: [(&str, bool, &str); 4] = [
    (t!("Clearance"), false, t!("clearance {0}")),
    (t!("Minimum width"), true, t!("minimum width {0}")),
    (t!("Thermal gap"), false, t!("thermal gap {0}")),
    (t!("Spoke width"), true, t!("spoke width {0}")),
];

/// Zone `z`'s lengths, in `LENGTHS`' order.
fn lengths(z: &mut Zone) -> [&mut Nm; 4] {
    [&mut z.clearance, &mut z.min_width, &mut z.thermal_gap, &mut z.spoke_width]
}

/// The pad connections, as KiCad's zone dialog names them, and as the undo step names a change
/// to each.
const CONNECTS: [(Connect, &str, &str); 4] = [
    (Connect::Solid, t!("Solid"), t!("pads connected solid")),
    (Connect::Thermal, t!("Thermal reliefs"), t!("pads with thermal reliefs")),
    (Connect::ThruHoleThermal, t!("Thermal reliefs on through-hole pads"), t!("thermal reliefs on through-hole pads")),
    (Connect::None, t!("None"), t!("pads not connected")),
];

/// A setting changed: zone `z` as set and what was done, or why it can't be.
fn changed(z: &Zone, set: impl FnOnce(&mut Zone) -> Result<String, String>) -> Result<(Zone, String), String> {
    let mut z = z.clone();
    set(&mut z).map(|what| (z, what))
}

impl App {
    /// Zone `i`'s settings, each changed in place (a rule area has only its name).
    pub(crate) fn zone_settings(&mut self, ui: &mut egui::Ui, i: usize) {
        let (b, z) = (&self.board, &self.board.zones[i]);
        let mut typing = self.typing.take().filter(|(k, ..)| *k == Pick::Zone(i)).map(|(_, name, text)| (name, text));
        let mut change = None;
        let net = |n: u32| match b.nets[n as usize].as_str() {
            "" => tr("no net").to_string(),
            name => name.to_string(),
        };
        if !z.keepout {
            ui.label(RichText::new(tr("Net")).small().color(theme::QUIET));
            egui::ComboBox::from_id_salt("zone net").selected_text(net(z.net)).width(ui.available_width()).height(320.0).show_ui(
                ui,
                |ui| {
                    let mut nets: Vec<u32> = (0..b.nets.len() as u32).collect();
                    nets.sort_by_cached_key(|&n| b.nets[n as usize].to_lowercase());
                    for n in nets {
                        if ui.selectable_label(n == z.net, net(n)).clicked() && n != z.net {
                            change = Some(changed(z, |z| {
                                z.net = n;
                                Ok(trf("net {0}", &[&net(n)]))
                            }));
                        }
                    }
                },
            );
        }
        // KiCad 5's files (before KiCad 6's version) name no zones.
        if b.version >= 20211014
            && let Some(text) = field_row(ui, tr("Name"), &unescape(&z.name), &mut typing)
        {
            change = Some(changed(z, |z| {
                z.name = escape(text.trim());
                Ok(if z.name.is_empty() { tr("name taken away").into() } else { trf("named {0}", &[&text.trim()]) })
            }));
        }
        if !z.keepout {
            if let Some(text) = field_row(ui, tr("Priority"), &z.priority.to_string(), &mut typing) {
                change = Some(changed(z, |z| {
                    let why =
                        trf("Priority {0} isn't a whole number: type 0 or more (the higher fills first)", &[&format!("{:?}", text.trim())]);
                    z.priority = text.trim().parse().map_err(|_| why)?;
                    Ok(trf("priority {0}", &[&z.priority]))
                }));
            }
            let shown = [z.clearance, z.min_width, z.thermal_gap, z.spoke_width];
            for (k, ((what, width, step), nm)) in LENGTHS.into_iter().zip(shown).enumerate() {
                let what = tr(what);
                let Some(text) = field_row(ui, what, &units::len(nm), &mut typing) else { continue };
                change = Some(changed(z, |z| {
                    let v = units::parse(&text).ok_or_else(|| {
                        trf("{0} {1} isn't a length: type one in {2}, or as 0.2mm or 8mil", &[&what, &format!("{text:?}"), &units::name()])
                    })?;
                    if v < 0 || v == 0 && width {
                        return Err(if width {
                            trf("{0} must be more than 0: type a length", &[&what])
                        } else {
                            trf("{0} must be at least 0: type a length", &[&what])
                        });
                    }
                    *lengths(z).into_iter().nth(k).unwrap() = v;
                    Ok(trf(step, &[&units::len(v)]))
                }));
            }
            ui.label(RichText::new(tr("Pad connection")).small().color(theme::QUIET));
            let now = CONNECTS.iter().find(|c| c.0 == z.connect).map_or("", |c| tr(c.1));
            egui::ComboBox::from_id_salt("zone pads").selected_text(now).width(ui.available_width()).show_ui(ui, |ui| {
                for (c, name, step) in CONNECTS.map(|(c, name, step)| (c, tr(name), step)) {
                    if ui.selectable_label(c == z.connect, name).clicked() && c != z.connect {
                        change = Some(changed(z, |z| {
                            z.connect = c;
                            Ok(tr(step).to_string())
                        }));
                    }
                }
            });
        }
        self.typing = typing.map(|(name, text)| (Pick::Zone(i), name, text));
        match change {
            Some(Ok((z, what))) => self.zone_edited(i, z, Ok(()), Instant::now(), Some(what)),
            Some(Err(why)) => self.refused = Some((Pick::Zone(i), why)),
            None => {}
        }
    }
}
