//! Net classes in the window and for agents: the project's classes (`.kicad_pro` `net_settings`,
//! where KiCad's Board Setup › Net Classes keeps them) listed in Inspect with nothing picked, a
//! class's values and nets typed in place, a net's class chosen beside the net. Each change is
//! written to the project file at once (kept as `-bak` first); the board's rules and checks
//! follow. Desktop only: a page has no project file to write.

use crate::inspect::field_row;
use crate::tr::{t, tr, trf};
use crate::units::{len, num};
use crate::{App, count, n, theme, units};
use cuprita_core::board::{NetId, Nm};
use cuprita_core::json::Val;
use eframe::egui::{self, RichText};

/// A class's values where a `.kicad_pro` keeps them, and what Inspect calls each.
pub(crate) const VALUES: [(&str, &str); 6] = [
    ("clearance", t!("Clearance")),
    ("track_width", t!("Track width")),
    ("via_diameter", t!("Via size")),
    ("via_drill", t!("Via drill")),
    ("diff_pair_width", t!("Pair width")),
    ("diff_pair_gap", t!("Pair gap")),
];

/// The list at `path` in `pro`, made empty where the project has none.
fn list<'a>(pro: &'a mut Val, path: &[&str]) -> &'a mut Vec<Val> {
    let at = pro.slot(path);
    if !matches!(at, Val::Arr(_)) {
        *at = Val::Arr(vec![]);
    }
    let Val::Arr(list) = at else { unreachable!("made a list above") };
    list
}

fn named(class: &Val, name: &str) -> bool {
    class.str("name") == Some(name)
}

/// Takes each (net, class) that `gone` says out of what the schematic's labels assign and of the
/// lists KiCad 6 keeps in its classes.
fn unassign(pro: &mut Val, gone: impl Fn(&str, &str) -> bool) {
    if let Val::Obj(nets) = pro.slot(&["net_settings", "netclass_assignments"]) {
        nets.retain_mut(|(net, class)| match class {
            Val::Str(class) => !gone(net, class),
            Val::Arr(of) => {
                of.retain(|c| !matches!(c, Val::Str(c) if gone(net, c)));
                !of.is_empty()
            }
            _ => true,
        });
    }
    for class in list(pro, &["net_settings", "classes"]) {
        let name = class.str("name").unwrap_or_default().to_string();
        if let Val::Arr(nets) = class.slot(&["nets"]) {
            nets.retain(|n| !matches!(n, Val::Str(n) if gone(n, &name)));
        }
    }
}

/// Sets `values` (mm, by their names in the file) of class `name`. A new class goes after the
/// others as a copy of Default's, as KiCad writes each class in full.
pub(crate) fn set(pro: &mut Val, name: &str, values: &[(&str, f64)]) {
    let classes = list(pro, &["net_settings", "classes"]);
    if !classes.iter().any(|c| named(c, name)) {
        let mut new = classes.iter().find(|c| named(c, "Default")).cloned().unwrap_or(Val::Obj(vec![]));
        *new.slot(&["name"]) = Val::Str(name.into());
        // KiCad 9 on orders classes by priority, Default's the last.
        if new.get("priority").is_some() {
            *new.slot(&["priority"]) = Val::Num(classes.iter().filter(|c| !named(c, "Default")).count() as f64);
        }
        classes.push(new);
    }
    for (key, mm) in values {
        *pro.slot(&["net_settings", "classes", name, key]) = Val::Num(*mm);
    }
}

/// Class `name`'s nets are `nets` (names, or patterns of them), in place of those it had by a
/// pattern, a label of the schematic or KiCad 6's list: patterns after the other classes'.
pub(crate) fn set_nets(pro: &mut Val, name: &str, nets: &[String]) {
    unassign(pro, |_, class| class == name);
    let patterns = list(pro, &["net_settings", "netclass_patterns"]);
    patterns.retain(|p| p.str("netclass") != Some(name));
    let entry = |net: &String| Val::Obj(vec![("netclass".into(), Val::Str(name.into())), ("pattern".into(), Val::Str(net.clone()))]);
    patterns.extend(nets.iter().map(entry));
}

/// Net `net` is of class `class` by its name (`None`: of none, so Default's), before every
/// pattern, which the first that fits a net wins.
pub(crate) fn assign(pro: &mut Val, net: &str, class: Option<&str>) {
    unassign(pro, |n, _| n == net);
    let patterns = list(pro, &["net_settings", "netclass_patterns"]);
    patterns.retain(|p| p.str("pattern") != Some(net));
    if let Some(class) = class {
        patterns.insert(0, Val::Obj(vec![("netclass".into(), Val::Str(class.into())), ("pattern".into(), Val::Str(net.into()))]));
    }
}

/// Class `from` is named `to`, and so wherever nets are assigned to it.
pub(crate) fn rename(pro: &mut Val, from: &str, to: &str) {
    let renamed = |v: &mut Val| {
        if matches!(v, Val::Str(s) if s == from) {
            *v = Val::Str(to.into());
        }
    };
    list(pro, &["net_settings", "classes"]).iter_mut().for_each(|c| renamed(c.slot(&["name"])));
    list(pro, &["net_settings", "netclass_patterns"]).iter_mut().for_each(|p| renamed(p.slot(&["netclass"])));
    if let Val::Obj(nets) = pro.slot(&["net_settings", "netclass_assignments"]) {
        for (_, class) in nets {
            match class {
                Val::Arr(of) => of.iter_mut().for_each(renamed),
                one => renamed(one),
            }
        }
    }
}

/// Class `name` goes, its nets Default's again.
pub(crate) fn remove(pro: &mut Val, name: &str) {
    unassign(pro, |_, class| class == name);
    list(pro, &["net_settings", "netclass_patterns"]).retain(|p| p.str("netclass") != Some(name));
    list(pro, &["net_settings", "classes"]).retain(|c| !named(c, name));
}

/// The project file's text after `edit`, written beside `board` (the one there kept as `-bak`).
pub(crate) fn edit_file(board: &std::path::Path, edit: impl FnOnce(&mut Val)) -> Result<String, String> {
    let pro = board.with_extension("kicad_pro");
    let text = std::fs::read_to_string(&pro).unwrap_or_else(|_| "{}".into());
    let mut v = Val::parse(&text).ok_or_else(|| trf("{0}: not JSON; fix it in KiCad or a text editor first", &[&pro.display()]))?;
    edit(&mut v);
    let text = v.pretty();
    crate::file::save_file(&pro, text.as_bytes())?;
    Ok(text)
}

impl App {
    /// Changes the project file by `edit` and takes its rules again; the checks follow, and the
    /// status says `what` was done (zones keep their fills until they are filled again).
    fn edit_project(&mut self, what: String, edit: impl FnOnce(&mut Val)) {
        let Some(board) = self.path.clone() else { return };
        match edit_file(&board, edit) {
            Ok(text) => {
                self.board.apply_pro(&text);
                self.violations = cuprita_core::drc::check(&self.board);
                self.status = what;
            }
            Err(e) => self.status = e,
        }
    }

    /// The values of class `class` (`None`: Default's), in `VALUES`' order.
    fn class_values(&self, class: Option<usize>) -> [Nm; 6] {
        let r = &self.board.rules;
        match class.and_then(|c| self.board.classes.get(c)) {
            Some(c) => [c.clearance, c.track_width, c.via_diameter, c.via_drill, c.pair_width, c.pair_gap],
            None => [r.clearance, r.class_track_width, r.class_via_diameter, r.class_via_drill, r.class_pair_width, r.class_pair_gap],
        }
    }

    /// Inspect with nothing picked: the project's net classes, Default first, each with how
    /// many nets it has and what they keep. A click opens one: its values typed in place, and
    /// (but Default's) its name, its nets by their names or patterns, and Delete. New class adds one.
    pub(crate) fn class_list(&mut self, ui: &mut egui::Ui) {
        if self.path.is_none() || self.viewer {
            return;
        }
        ui.add_space(12.0);
        crate::dock::heading(ui, tr("Net classes"));
        let of = self.board.net_classes();
        let names: Vec<String> = self.board.classes.iter().map(|c| c.name.clone()).collect();
        let rows = [(None, "Default".to_string())].into_iter().chain(names.iter().cloned().enumerate().map(|(i, n)| (Some(i), n)));
        let (mut typing, mut edit) = (self.class_typing.take(), None::<(String, Box<dyn FnOnce(&mut Val)>)>);
        for (class, name) in rows {
            let (open, values) = (self.class_open.as_deref() == Some(name.as_str()), self.class_values(class));
            let nets = of.iter().skip(1).filter(|&&c| c == class).count();
            ui.horizontal(|ui| {
                if ui.selectable_label(open, RichText::new(&name).color(theme::TEXT)).clicked() {
                    self.class_open = (!open).then(|| name.clone());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let says = format!("{} · {} / {}", count(nets, n!("net")), num(values[0]), len(values[1]));
                    ui.label(RichText::new(says).small().color(theme::QUIET)).on_hover_text(tr("Its nets · clearance / track width"));
                });
            });
            if !open {
                continue;
            }
            ui.indent(("class", &name), |ui| {
                if class.is_some()
                    && let Some(to) = field_row(ui, tr("Name"), &name, &mut typing).map(|t| t.trim().to_string())
                {
                    if to.is_empty() || to == "Default" || names.contains(&to) {
                        self.status = trf("A class needs a name of its own: {0} is taken", &[&format!("{to:?}")]);
                    } else {
                        let (from, new) = (name.clone(), to.clone());
                        edit = Some((trf("Class {0} is now {1}", &[&name, &to]), Box::new(move |pro| rename(pro, &from, &new))));
                        self.class_open = Some(to);
                    }
                }
                for ((key, label), value) in VALUES.iter().zip(values) {
                    let Some(text) = field_row(ui, tr(label), &len(value), &mut typing) else { continue };
                    match units::parse(&text).filter(|&v| v > 0) {
                        Some(v) => {
                            let (name, what) = (name.clone(), trf("{0}: {1} {2}", &[&name, &tr(label).to_lowercase(), &len(v)]));
                            edit = Some((what, Box::new(move |pro| set(pro, &name, &[(key, v as f64 / 1e6)]))));
                        }
                        None => self.status = trf("{0} isn't a length: type one in {1}, or as 0.3mm or 12mil", &[&format!("{text:?}"), &units::name()]),
                    }
                }
                let Some(i) = class else { return };
                let has: Vec<&str> = self.board.class_nets.iter().filter(|(_, c)| *c == i).map(|(n, _)| n.as_str()).collect();
                if let Some(text) = field_row(ui, tr("Nets"), &has.join(", "), &mut typing) {
                    let nets: Vec<String> = text.split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
                    let (name, what) = (name.clone(), trf("{0} has {1}", &[&name, &count(nets.len(), n!("name|names"))]));
                    edit = Some((what, Box::new(move |pro| set_nets(pro, &name, &nets))));
                }
                let hint = tr("Their names with commas between, or patterns: VBUS, /USB/D?, /DDR/*. A net's own Class, with the net lit, puts one in too.");
                ui.label(RichText::new(hint).small().color(theme::QUIET));
                if ui.button(tr("Delete class")).on_hover_text(tr("Its nets are Default's again")).clicked() {
                    let gone = name.clone();
                    edit = Some((trf("Class {0} deleted", &[&name]), Box::new(move |pro| remove(pro, &gone))));
                    self.class_open = None;
                }
            });
        }
        self.class_typing = typing;
        if ui.button(tr("New class")).on_hover_text(tr("A class as Default, to name and set")).clicked() {
            let name = (1..).map(|k| format!("Class {k}")).find(|n| !names.contains(n)).expect("a free name");
            self.class_open = Some(name.clone());
            edit =
                Some((trf("Class {0} added: set what its nets keep, and its nets", &[&name]), Box::new(move |pro| set(pro, &name, &[]))));
        }
        if let Some((what, edit)) = edit {
            self.edit_project(what, edit);
        }
    }

    /// The class of net `net`, lit, in Inspect: chosen from the project's, which puts the net
    /// in it by its name.
    pub(crate) fn net_class(&mut self, ui: &mut egui::Ui, net: NetId) {
        let is = self.board.class(net).map_or("Default", |c| c.name.as_str()).to_string();
        ui.label(RichText::new(tr("Class")).small().color(theme::QUIET));
        if self.path.is_none() || self.viewer {
            return drop(ui.label(RichText::new(&is).monospace()));
        }
        let mut chosen = None;
        egui::ComboBox::from_id_salt("net class").selected_text(&is).width(ui.available_width()).show_ui(ui, |ui| {
            for name in ["Default"].into_iter().chain(self.board.classes.iter().map(|c| c.name.as_str())) {
                if ui.selectable_label(name == is, name).clicked() && name != is {
                    chosen = Some(name.to_string());
                }
            }
        });
        let Some(class) = chosen else { return };
        let (name, to) = (self.board.nets[net as usize].clone(), class.clone());
        let what = trf("{0} is of class {1}", &[&name, &class]);
        self.edit_project(what, move |pro| assign(pro, &name, (to != "Default").then_some(to.as_str())));
        // A pattern of another class may still fit it.
        let now = self.board.class(net).map_or("Default", |c| c.name.as_str());
        if now != class {
            let name = &self.board.nets[net as usize];
            self.status = trf(
                "{0} stays of class {1} by a pattern of its nets: change that in the class (Inspect, with nothing picked)",
                &[name, &now],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cuprita_core::board::Board;

    #[test]
    fn edits_the_projects_classes_as_kicad_keeps_them() {
        let mut pro = Val::parse(
            r#"{"net_settings": {"classes": [{"name": "Default", "clearance": 0.2, "track_width": 0.25, "priority": 2147483647},
                {"name": "Old", "clearance": 0.3, "nets": ["A", "B"], "priority": 0}],
              "netclass_assignments": {"C": "Old", "D": ["Old", "Other"], "E": "Other"},
              "netclass_patterns": [{"netclass": "Old", "pattern": "/bus/*"}, {"netclass": "Other", "pattern": "X"}]}}"#,
        )
        .unwrap();
        // A new class is Default's copy, next in priority; its values set, the rest kept.
        set(&mut pro, "Power", &[("clearance", 0.4), ("track_width", 0.5)]);
        let class = |pro: &Val, name: &str| {
            pro.at(&["net_settings"]).and_then(|n| n.arr("classes")).unwrap().iter().find(|c| named(c, name)).cloned()
        };
        let power = class(&pro, "Power").unwrap();
        assert_eq!((power.num("clearance"), power.num("track_width"), power.num("priority")), (Some(0.4), Some(0.5), Some(1.0)));
        set(&mut pro, "Default", &[("via_drill", 0.3)]);
        assert_eq!(
            (class(&pro, "Default").unwrap().num("via_drill"), class(&pro, "Default").unwrap().num("clearance")),
            (Some(0.3), Some(0.2))
        );
        // Its nets by patterns, after the others'; a net by its name goes first, out of where it was.
        set_nets(&mut pro, "Power", &["+5V".into(), "VBUS*".into()]);
        assign(&mut pro, "A", Some("Power"));
        assign(&mut pro, "C", None);
        let patterns = |pro: &Val| -> Vec<(String, String)> {
            let all = pro.at(&["net_settings"]).and_then(|n| n.arr("netclass_patterns")).unwrap();
            all.iter().map(|p| (p.str("netclass").unwrap().to_string(), p.str("pattern").unwrap().to_string())).collect()
        };
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            patterns(&pro),
            [pair("Power", "A"), pair("Old", "/bus/*"), pair("Other", "X"), pair("Power", "+5V"), pair("Power", "VBUS*")]
        );
        assert_eq!(pro.at(&["net_settings", "netclass_assignments"]).unwrap().to_json(), r#"{"D":["Old","Other"],"E":"Other"}"#);
        assert_eq!(class(&pro, "Old").unwrap().get("nets").unwrap().to_json(), r#"["B"]"#);
        // The board reads them so.
        let mut b =
            Board::parse(r#"(kicad_pcb (net 0 "") (net 1 "A") (net 2 "B") (net 3 "C") (net 4 "D") (net 5 "VBUS_IN") (net 6 "/bus/1"))"#)
                .unwrap();
        b.apply_pro(&pro.pretty());
        let of = |b: &Board| (1..7).map(|n| b.class(n).map_or("Default", |c| c.name.as_str()).to_string()).collect::<Vec<_>>();
        assert_eq!(of(&b), ["Power", "Old", "Default", "Old", "Power", "Old"]);
        assert_eq!(b.classes[1].clearance, 400_000);
        // Renamed, a class keeps its nets; deleted, they are Default's again.
        rename(&mut pro, "Old", "Bus");
        b.apply_pro(&pro.pretty());
        assert_eq!(of(&b), ["Power", "Bus", "Default", "Bus", "Power", "Bus"]);
        remove(&mut pro, "Bus");
        set_nets(&mut pro, "Power", &[]);
        b.apply_pro(&pro.pretty());
        assert_eq!((of(&b), b.classes.len()), (vec!["Default".to_string(); 6], 1));
        assert_eq!(pro.at(&["net_settings", "netclass_assignments"]).unwrap().to_json(), r#"{"D":["Other"],"E":"Other"}"#);
    }
}
