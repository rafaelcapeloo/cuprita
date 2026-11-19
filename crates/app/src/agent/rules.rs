//! Agents' `rules`: the board's design rules, as its project's `.kicad_pro` keeps them for KiCad.

use super::Session;
use cuprita_core::board::Nm;
use cuprita_core::json::{Val, json_str};

/// Each rule by the name agents give it, and where a `.kicad_pro` keeps it: the Default net
/// class's (what new tracks and vias get), else the board's limits (what a fab can make).
const RULES: [(&str, &[&str]); 13] = [
    ("clearance", &["net_settings", "classes", "Default", "clearance"]),
    ("track_width", &["net_settings", "classes", "Default", "track_width"]),
    ("via_diameter", &["net_settings", "classes", "Default", "via_diameter"]),
    ("via_drill", &["net_settings", "classes", "Default", "via_drill"]),
    ("min_clearance", &["board", "design_settings", "rules", "min_clearance"]),
    ("min_track_width", &["board", "design_settings", "rules", "min_track_width"]),
    ("min_via_diameter", &["board", "design_settings", "rules", "min_via_diameter"]),
    ("min_hole", &["board", "design_settings", "rules", "min_through_hole_diameter"]),
    ("min_annular_ring", &["board", "design_settings", "rules", "min_via_annular_width"]),
    ("min_hole_to_hole", &["board", "design_settings", "rules", "min_hole_to_hole"]),
    ("min_hole_clearance", &["board", "design_settings", "rules", "min_hole_clearance"]),
    ("min_edge_clearance", &["board", "design_settings", "rules", "min_copper_edge_clearance"]),
    ("zone_clearance", &["board", "design_settings", "defaults", "zones", "min_clearance"]),
];

/// A net class's values by the names agents give them, and where a `.kicad_pro` keeps each.
const CLASS: [(&str, &str); 6] = [
    ("clearance", "clearance"),
    ("track_width", "track_width"),
    ("via_diameter", "via_diameter"),
    ("via_drill", "via_drill"),
    ("pair_width", "diff_pair_width"),
    ("pair_gap", "diff_pair_gap"),
];

impl Session {
    /// The open board's design rules in mm, those `args` gives set first in its `.kicad_pro`
    /// (kept as `-bak` first).
    pub(super) fn rules(&mut self, args: &Val) -> Result<String, String> {
        let board = self.path.clone().ok_or("no board open: open its board (open) or start a project (new)")?;
        let Val::Obj(given) = args else { return Err("give the rules as an object of mm values".into()) };
        let mut set = vec![];
        for (name, v) in given.iter().filter(|(k, _)| k != "cmd") {
            let names = || RULES.map(|r| r.0).join(", ");
            let path = RULES.iter().find(|r| r.0 == name).ok_or_else(|| format!("no rule {name}: the rules are {}", names()))?.1;
            match v {
                Val::Num(mm) if mm.is_finite() && *mm >= 0.0 => set.push((path, *mm)),
                _ => return Err(format!("{name}: give it in mm, a number 0 or more")),
            }
        }
        if !set.is_empty() {
            let text = crate::classes::edit_file(&board, |pro| set.iter().for_each(|(path, mm)| *pro.slot(path) = Val::Num(*mm)))?;
            self.board.apply_pro(&text);
        }
        Ok(self.rules_json())
    }

    /// A net class of the open board's project, set as `args` gives it and made when new: its
    /// values in mm, its `nets` (names, or patterns of them) in place of those it had; with
    /// `delete`, taken away. Written to the `.kicad_pro` (kept as `-bak` first); gives the rules.
    pub(super) fn netclass(&mut self, args: &Val) -> Result<String, String> {
        let board = self.path.clone().ok_or("no board open: open its board (open) or start a project (new)")?;
        let name = args.str("name").map(str::trim).filter(|n| !n.is_empty()).ok_or("give the class's name")?.to_string();
        let Val::Obj(given) = args else { return Err("give the class as an object".into()) };
        let (mut values, mut nets) = (vec![], None);
        for (key, v) in given.iter().filter(|(k, _)| !["cmd", "name", "delete"].contains(&k.as_str())) {
            let names = || CLASS.map(|c| c.0).join(", ");
            match (CLASS.iter().find(|c| c.0 == key), v) {
                (Some(c), Val::Num(mm)) if mm.is_finite() && *mm > 0.0 => values.push((c.1, *mm)),
                (Some(_), _) => return Err(format!("{key}: give it in mm, a number above 0")),
                (None, Val::Arr(of)) if key == "nets" => {
                    nets = Some(of.iter().filter_map(|n| if let Val::Str(n) = n { Some(n.clone()) } else { None }).collect::<Vec<_>>());
                }
                _ => return Err(format!("no {key} in a class: it has {}, and nets (a list of names or patterns)", names())),
            }
        }
        let delete = matches!(args.get("delete"), Some(Val::Bool(true)));
        if name == "Default" && (delete || nets.is_some()) {
            return Err("Default is the class of every net no other takes: it has no nets of its own, and stays".into());
        }
        let text = crate::classes::edit_file(&board, |pro| match delete {
            true => crate::classes::remove(pro, &name),
            false => {
                crate::classes::set(pro, &name, &values);
                nets.iter().for_each(|nets| crate::classes::set_nets(pro, &name, nets));
            }
        })?;
        self.board.apply_pro(&text);
        Ok(self.rules_json())
    }

    /// The board's rules in mm as agents read them, with its project's net classes.
    fn rules_json(&self) -> String {
        let (b, r, mm) = (&self.board, &self.board.rules, |v: Nm| v as f64 / 1e6);
        let values = [
            r.clearance,
            r.class_track_width,
            r.class_via_diameter,
            r.class_via_drill,
            r.track_width,
            r.via_diameter,
            r.hole_diameter,
            r.annular_width,
            r.hole_to_hole,
            r.hole_clearance,
            r.edge_clearance,
            r.zone_clearance,
        ];
        let names = RULES.iter().map(|r| r.0).filter(|&n| n != "min_clearance");
        let rules = names.zip(values).map(|(n, v)| format!(r#""{n}":{}"#, mm(v))).collect::<Vec<_>>().join(",");
        let of = b.net_classes();
        let classes = b.classes.iter().enumerate().map(|(i, c)| {
            let values = [c.clearance, c.track_width, c.via_diameter, c.via_drill, c.pair_width, c.pair_gap];
            let values = CLASS.iter().zip(values).map(|(k, v)| format!(r#""{}":{}"#, k.0, mm(v))).collect::<Vec<_>>().join(",");
            let nets = b.class_nets.iter().filter(|(_, class)| *class == i).map(|(n, _)| json_str(n)).collect::<Vec<_>>().join(",");
            let has = of.iter().filter(|&&class| class == Some(i)).count();
            format!(r#"{{"name":{},{values},"nets":[{nets}],"nets_on_board":{has}}}"#, json_str(&c.name))
        });
        format!(r#"{{{rules},"classes":[{}]}}"#, classes.collect::<Vec<_>>().join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_the_rules_in_the_project_and_reads_them_back() {
        let dir = std::env::temp_dir().join(format!("cuprita-rules-{}", std::process::id()));
        let mut s = Session::default();
        let new = format!(r#"{{"cmd":"new","dir":{},"name":"r"}}"#, cuprita_core::json::json_str(&dir.display().to_string()));
        assert!(!s.cli_line(&new).contains("error"));
        let set = s.cli_line(r#"{"cmd":"rules","clearance":0.1,"track_width":0.15,"min_clearance":0.1,"min_edge_clearance":0.3}"#);
        let got = Val::parse(&set).unwrap();
        assert_eq!([got.num("clearance"), got.num("track_width"), got.num("min_edge_clearance")], [Some(0.1), Some(0.15), Some(0.3)]);
        let zones = Val::parse(&s.cli_line(r#"{"cmd":"rules","zone_clearance":0.2}"#)).unwrap();
        assert_eq!(zones.num("zone_clearance"), Some(0.2));
        // KiCad reads them where it keeps them; the project as it was is kept.
        let pro = Val::parse(&std::fs::read_to_string(dir.join("r.kicad_pro")).unwrap()).unwrap();
        let class =
            pro.at(&["net_settings"]).and_then(|n| n.arr("classes")).unwrap().iter().find(|c| c.str("name") == Some("Default")).unwrap();
        assert_eq!((class.num("clearance"), class.num("via_drill")), (Some(0.1), Some(0.3)));
        assert_eq!(pro.at(&["board", "design_settings", "rules"]).and_then(|r| r.num("min_copper_edge_clearance")), Some(0.3));
        assert!(dir.join("r.kicad_pro-bak").exists());
        let error = |s: &mut Session, c: &str| Val::parse(&s.cli_line(c)).unwrap().str("error").map(String::from);
        assert!(
            error(&mut s, r#"{"cmd":"rules","clearence":0.1}"#)
                .unwrap()
                .starts_with("no rule clearence: the rules are clearance, track_width")
        );
        assert_eq!(error(&mut s, r#"{"cmd":"rules","clearance":-1}"#).as_deref(), Some("clearance: give it in mm, a number 0 or more"));
        // A net class: made with its values and nets, read back with the rules, then deleted.
        let set =
            s.cli_line(r#"{"cmd":"netclass","name":"Power","clearance":0.3,"track_width":0.5,"pair_gap":0.15,"nets":["+5V","VBUS*"]}"#);
        let classes = Val::parse(&set).unwrap();
        let power = &classes.arr("classes").unwrap_or_else(|| panic!("{set}"))[0];
        assert_eq!(
            (power.str("name"), power.num("clearance"), power.num("track_width"), power.num("via_drill")),
            (Some("Power"), Some(0.3), Some(0.5), Some(0.3))
        );
        assert_eq!((power.num("pair_gap"), power.get("nets").map(Val::to_json)), (Some(0.15), Some(r#"["+5V","VBUS*"]"#.to_string())));
        let pro = Val::parse(&std::fs::read_to_string(dir.join("r.kicad_pro")).unwrap()).unwrap();
        assert_eq!(pro.at(&["net_settings"]).and_then(|n| n.arr("netclass_patterns")).map(<[Val]>::len), Some(2));
        assert_eq!(error(&mut s, r#"{"cmd":"netclass","name":"Power","width":1}"#).unwrap().split(':').next(), Some("no width in a class"));
        assert!(
            error(&mut s, r#"{"cmd":"netclass","name":"Default","delete":true}"#).unwrap().starts_with("Default is the class of every net")
        );
        let gone = Val::parse(&s.cli_line(r#"{"cmd":"netclass","name":"Power","delete":true}"#)).unwrap();
        assert_eq!(gone.arr("classes").map(<[Val]>::len), Some(0));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
