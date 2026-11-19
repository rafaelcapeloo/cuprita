//! Agents' `library`, the parts the libraries have; `import`, a part's files from a vendor
//! (SnapMagic, Ultra Librarian, SamacSys…) into the project's own library, and a 3D model onto a
//! part on the board; and `new_part`, a part of one's own made into that library
//! (`cuprita_core::newpart`), as the New part window makes one.

use super::{Session, mm_pt, redraw};
use crate::edit::NO_LIBRARIES;
use crate::pick_name;
use crate::sheet_edit::NO_SYMBOLS;
use cuprita_core::board::{Model, Nm};
use cuprita_core::history::Pick;
use cuprita_core::json::{Val, json_str, list};
use cuprita_core::library;
use cuprita_core::newpart::{self, Footprint, PRESETS, Package, Part, Pin, Side, TYPES};

impl Session {
    /// A part's own text (`part_text`): said where and how it is, or moved, turned, sized, hidden
    /// or shown again, one undo step, as the window does with one picked.
    pub(super) fn part_text(&mut self, args: &Val) -> Result<String, String> {
        let b = &self.board;
        let q = args.str("reference").ok_or("give reference, the part whose text it is")?;
        let part = b.footprints.iter().position(|f| f.reference.eq_ignore_ascii_case(q)).ok_or_else(|| format!("not found: {q}"))?;
        let f = &b.footprints[part];
        let (name, words) = match args.str("text") {
            None | Some("reference") => (f.reference.clone(), f.reference.clone()),
            Some("value") => (f.reference.clone(), f.value.clone()),
            Some(words) => (f.reference.clone(), words.to_string()),
        };
        // Of those reading so, the one on the layer named, else on a silkscreen, else the first.
        let on = |l: u8| b.layers[l as usize].clone();
        let best = |l: u8| args.str("layer").map_or(!on(l).ends_with(".SilkS"), |name| on(l) != name);
        let shown = f.texts.clone().filter(|&i| b.texts[i].text == words).min_by_key(|&i| best(b.texts[i].layer));
        let hidden = (0..f.hidden.len()).filter(|&k| f.hidden[k].text == words).min_by_key(|&k| best(f.hidden[k].layer));
        let (at, angle, height) = (args.arr("at"), args.num("angle"), args.num("height"));
        let off = |layer: Option<String>| {
            let layer = layer.map_or(String::new(), |l| format!(r#""layer":{},"#, json_str(&l)));
            Ok(format!(r#"{{"text":{},"of":{},{layer}"shown":false}}"#, json_str(&words), json_str(&name)))
        };
        let (i, done) = match (shown, hidden, args.get("show").map(|v| matches!(v, Val::Bool(true)))) {
            (Some(i), _, Some(false)) => {
                let what = pick_name(b, Pick::Text(i));
                let undo = self.board.delete_items(&[Pick::Text(i)]);
                self.edits.push(&mut self.board, undo, format!("{what} hidden"));
                redraw(&mut self.scene, &self.board);
                return off(None);
            }
            (Some(i), ..) => (i, "set"),
            (None, Some(_), Some(false)) => return off(None),
            (None, Some(k), Some(true)) => {
                let (pick, undo) = self.board.show_text(part, k);
                let what = pick_name(&self.board, pick);
                self.edits.push(&mut self.board, undo, format!("{what} shown"));
                let Pick::Text(i) = pick else { unreachable!("show_text picks a text") };
                (i, "shown")
            }
            (None, Some(_), None) if at.is_some() || angle.is_some() || height.is_some() => {
                return Err(format!("{name}'s text {words} is hidden: show it first (show: true)"));
            }
            (None, Some(k), None) => return off(Some(on(f.hidden[k].layer))),
            (None, None, _) => {
                let has = f.texts.clone().map(|i| &b.texts[i]).chain(&f.hidden).map(|t| t.text.as_str()).collect::<Vec<_>>().join(", ");
                return Err(format!("{name} has no text {words}; it has: {has}"));
            }
        };
        let mut t = self.board.texts[i].clone();
        match at {
            Some([Val::Num(x), Val::Num(y)]) if x.is_finite() && y.is_finite() => t.at = mm_pt(*x, *y),
            Some(_) => return Err("at must be [x_mm, y_mm]".into()),
            None => {}
        }
        // A height sets the letters' width and pen with it, as KiCad's own do: 15% of it.
        match height {
            Some(h) if h > 0.0 && h.is_finite() => {
                let h = (h * 1e6).round() as Nm;
                (t.size, t.thickness) = ((h, h), h * 3 / 20);
            }
            Some(_) => return Err("height must be more than 0 mm".into()),
            None => {}
        }
        t.angle = angle.filter(|a| a.is_finite()).map_or(t.angle, |a| a.rem_euclid(360.0));
        if t != self.board.texts[i] {
            let (what, undo) = (pick_name(&self.board, Pick::Text(i)), self.board.set_text(i, t));
            self.edits.push(&mut self.board, undo, format!("{what} {done}"));
        }
        redraw(&mut self.scene, &self.board);
        let (b, mm) = (&self.board, |v: Nm| v as f64 / 1e6);
        let (t, (lo, hi)) = (&b.texts[i], b.extent(&[Pick::Text(i)]).unwrap_or_default());
        Ok(format!(
            r#"{{"text":{},"of":{},"layer":{},"shown":true,"at_mm":[{},{}],"angle":{},"height_mm":{},"box_mm":[{},{},{},{}]}}"#,
            json_str(&t.text),
            json_str(&b.footprints[part].reference),
            json_str(&b.layers[t.layer as usize]),
            mm(t.at.x),
            mm(t.at.y),
            t.angle,
            mm(t.size.1),
            mm(lo.x),
            mm(lo.y),
            mm(hi.x),
            mm(hi.y)
        ))
    }

    /// The symbols or footprints of KiCad's libraries and the project's own that words find, best
    /// first (`library::search`, as `add` and the Parts tab find them).
    pub(super) fn library(&mut self, args: &Val) -> Result<String, String> {
        let folder = self.folder();
        let (all, q, missing) = match (args.str("symbol"), args.str("footprint")) {
            (Some(q), None) => (self.symbols.get_or_insert_with(|| crate::parts::symbols(folder.as_deref())), q, NO_SYMBOLS),
            (None, Some(q)) => (self.parts.get_or_insert_with(|| crate::parts::footprints(folder.as_deref())), q, NO_LIBRARIES),
            _ => return Err("give symbol or footprint: words of what to find (regulator 3.3, usb c plug)".into()),
        };
        if all.is_empty() {
            return Err(missing.into());
        }
        let hits = library::search(all, q);
        let limit = args.num("limit").map_or(20, |n| n.max(1.0) as usize);
        let parts = hits.iter().take(limit).map(|p| format!(r#"{{"id":{},"about":{}}}"#, json_str(&p.id()), json_str(&p.about)));
        Ok(format!(r#"{{"matched":{},"parts":{}}}"#, hits.len(), list(parts)))
    }

    /// Puts the files at paths `files` into the open project's own libraries (`parts::import`);
    /// with `reference`, that board part shows the 3D model that came instead of its own, an edit.
    pub(super) fn import(&mut self, args: &Val) -> Result<String, String> {
        let paths: Vec<&str> = args
            .arr("files")
            .unwrap_or_default()
            .iter()
            .filter_map(|v| if let Val::Str(s) = v { Some(s.as_str()) } else { None })
            .collect();
        if paths.is_empty() {
            return Err("give files: the paths of a part's files (.kicad_sym, .kicad_mod, .step) or of the zip they came in".into());
        }
        let root = self.path.clone().or_else(|| self.sch.as_ref().map(|s| s.root.clone()));
        let (Some(dir), Some(name)) = (self.folder(), root.and_then(|f| Some(f.file_stem()?.to_string_lossy().into_owned()))) else {
            return Err("no project open: open its board (open) or start one (new) first; parts go into its own libraries".into());
        };
        let to = match args.str("reference") {
            Some(r) => Some(
                self.board
                    .footprints
                    .iter()
                    .position(|f| f.reference.eq_ignore_ascii_case(r))
                    .ok_or_else(|| format!("no part {r} on the board"))?,
            ),
            None => None,
        };
        let files = paths.iter().map(|p| std::fs::read(p).map(|b| (p.to_string(), b)).map_err(|e| format!("{p}: {e}")));
        let done = cuprita_core::parts::import(&dir, &name, files.collect::<Result<_, _>>()?)?;
        (self.parts, self.symbols) = (None, None);
        let mut given = "null".to_string();
        if let Some(i) = to {
            let reference = self.board.footprints[i].reference.clone();
            let Some(file) = done.models.first() else {
                return Err(format!(
                    "no 3D model for {reference} among the files (a .step, .stp or .wrl, alone or zipped); the others went in"
                ));
            };
            let undo = self.board.set_models(i, vec![Model { file: file.clone(), offset: [0.0; 3], scale: [1.0; 3], rotate: [0.0; 3] }]);
            self.edits.push(&mut self.board, undo, format!("{reference}'s 3D model"));
            given = json_str(&reference);
        }
        let names = |v: &[String]| list(v.iter().map(|s| json_str(s)));
        Ok(format!(
            r#"{{"symbols":{},"footprints":{},"models":{},"skipped":{},"model_of":{given}}}"#,
            names(&done.symbols),
            names(&done.footprints),
            names(&done.models),
            names(&done.skipped)
        ))
    }

    /// `new_part`: `name`, `reference` (else U), `description`, `datasheet`; `pins`, how many
    /// (`newpart::pins`) or each one's `number`, `name`, `type` and `side`; and its footprint, a
    /// library's (`footprint`: `lib:name`) or made from `package` (a preset by name, its numbers in
    /// mm to change). Returns them as `add` takes them.
    pub(super) fn new_part(&mut self, args: &Val) -> Result<String, String> {
        let file = self.path.clone().or_else(|| self.sch.as_ref().map(|s| s.root.clone()));
        let (Some(dir), Some(project)) = (self.folder(), file.and_then(|f| Some(f.file_stem()?.to_string_lossy().into_owned()))) else {
            return Err("no project open: open its board (or a sheet of it) first".into());
        };
        let pins = match args.get("pins") {
            Some(Val::Num(n)) if *n >= 1.0 => newpart::pins(*n as usize),
            Some(Val::Arr(pins)) => pins.iter().enumerate().map(|(i, p)| pin(i, p, pins.len())).collect::<Result<_, _>>()?,
            _ => return Err("missing pins: how many, or a list of {number, name, type, side}".into()),
        };
        let footprint = match (args.str("footprint"), args.get("package")) {
            (Some(id), _) => Footprint::Library(id.to_string()),
            (None, Some(p)) => Footprint::Made(package(p, pins.len())?),
            _ => {
                return Err(
                    "missing footprint (a library's, as Package_SO:SOIC-8_3.9x4.9mm_P1.27mm) or package ({\"preset\":\"SOIC\"})".into()
                );
            }
        };
        let text = |k: &str| args.str(k).unwrap_or_default().to_string();
        let reference = args.str("reference").unwrap_or("U").to_string();
        let part = Part { name: text("name"), reference, description: text("description"), datasheet: text("datasheet"), pins, footprint };
        let done = newpart::create(&dir, &project, &part)?;
        // `add` finds it in the project's library from now on.
        (self.parts, self.symbols) = (None, None);
        let ids = |v: &[String]| list(v.iter().map(|s| json_str(s)));
        Ok(format!(r#"{{"symbol":{},"footprint":{}}}"#, ids(&done.symbols), ids(&done.footprints)))
    }
}

/// Pin `i` of `n` as an agent gives it: its number (else `i + 1`), name, KiCad electrical type
/// (else passive) and side (else the first half's left, the rest's right).
fn pin(i: usize, p: &Val, n: usize) -> Result<Pin, String> {
    let number = match p.get("number") {
        Some(Val::Num(k)) => k.to_string(),
        Some(Val::Str(k)) => k.clone(),
        _ => (i + 1).to_string(),
    };
    let kind = p.str("type").unwrap_or("passive");
    let kind =
        TYPES.into_iter().find(|t| *t == kind).ok_or_else(|| format!("pin {number}: type {kind} isn't KiCad's: {}", TYPES.join(", ")))?;
    let side = match p.str("side") {
        None if i < n.div_ceil(2) => Side::Left,
        None => Side::Right,
        Some("left") => Side::Left,
        Some("right") => Side::Right,
        Some("top") => Side::Top,
        Some("bottom") => Side::Bottom,
        Some(s) => return Err(format!("pin {number}: side {s}: left, right, top or bottom")),
    };
    Ok(Pin { number, name: p.str("name").unwrap_or_default().to_string(), kind, side })
}

/// A package as an agent gives it: `preset`'s numbers for `pins` pins (its exposed pad the last of
/// them), each one given in mm (`pitch_mm`, `span_mm`, `drill_mm`, `pad_mm`, `body_mm` and
/// `exposed_mm` as [width, length]; `exposed_mm` null for none) in place of the preset's.
fn package(p: &Val, pins: usize) -> Result<Package, String> {
    let named = p.str("preset").unwrap_or_default();
    let names = || PRESETS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ");
    let (_, make) =
        PRESETS.iter().find(|(n, _)| n.eq_ignore_ascii_case(named)).ok_or_else(|| format!("preset {named}: one of {}", names()))?;
    let has = make(pins).exposed.is_some() as usize;
    let mut out = make(pins.saturating_sub(has));
    let nm = |mm: f64| (mm * 1e6).round() as Nm;
    let pair = |k: &str| match p.arr(k) {
        Some([Val::Num(a), Val::Num(b)]) => Some((nm(*a), nm(*b))),
        _ => None,
    };
    for (key, size) in [("pitch_mm", &mut out.pitch), ("span_mm", &mut out.span), ("drill_mm", &mut out.drill)] {
        if let Some(mm) = p.num(key) {
            *size = nm(mm);
        }
    }
    out.pad = pair("pad_mm").unwrap_or(out.pad);
    out.body = pair("body_mm").unwrap_or(out.body);
    match p.get("exposed_mm") {
        Some(Val::Null) => out.exposed = None,
        Some(_) => out.exposed = pair("exposed_mm"),
        None => {}
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_library_parts_words_find() {
        let mut s = Session::default();
        let part = |lib: &str, name: &str, about: &str| library::LibPart {
            lib: lib.into(),
            name: name.into(),
            path: Default::default(),
            about: about.into(),
        };
        s.symbols = Some(vec![
            part("Regulator_Linear", "AP2112K-3.3", "600mA low dropout linear regulator, 3.3V"),
            part("Device", "R", "Resistor"),
        ]);
        s.parts = Some(vec![]);
        let found = s.cli_line(r#"{"cmd":"library","symbol":"regulator 3.3"}"#);
        assert_eq!(
            found,
            r#"{"matched":1,"parts":[{"id":"Regulator_Linear:AP2112K-3.3","about":"600mA low dropout linear regulator, 3.3V"}]}"#
        );
        let mut error = |c: &str| Val::parse(&s.cli_line(c)).unwrap().str("error").map(String::from);
        assert_eq!(error(r#"{"cmd":"library","footprint":"usb"}"#).as_deref(), Some(NO_LIBRARIES));
        assert_eq!(
            error(r#"{"cmd":"library"}"#).as_deref(),
            Some("give symbol or footprint: words of what to find (regulator 3.3, usb c plug)")
        );
    }

    #[test]
    fn imports_a_vendors_part_and_gives_a_part_its_model() {
        let dir = std::env::temp_dir().join(format!("cuprita-import-{}", std::process::id()));
        let (got, mut s) = (dir.join("got"), Session::default());
        std::fs::create_dir_all(&got).unwrap();
        let pad = r#"(pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))"#;
        std::fs::write(got.join("SOT23.kicad_mod"), format!(r#"(footprint "SOT23" (layer "F.Cu") (property "Reference" "REF**") {pad})"#))
            .unwrap();
        std::fs::write(got.join("SOT23.step"), "ISO-10303-21;").unwrap();
        std::fs::write(got.join("other.wrl"), "#VRML V2.0 utf8").unwrap();
        let files = |names: &[&str]| list(names.iter().map(|n| json_str(&got.join(n).display().to_string())));
        let import =
            |s: &mut Session, names: &[&str], more: &str| s.cli_line(&format!(r#"{{"cmd":"import","files":{}{more}}}"#, files(names)));
        let error = |reply: String| Val::parse(&reply).unwrap().str("error").map(String::from);
        assert_eq!(
            error(import(&mut s, &["SOT23.step"], "")).as_deref(),
            Some("no project open: open its board (open) or start one (new) first; parts go into its own libraries")
        );
        let new = format!(r#"{{"cmd":"new","dir":{},"name":"cam"}}"#, json_str(&dir.join("cam").display().to_string()));
        assert_eq!(error(s.cli_line(&new)), None);
        let done = Val::parse(&import(&mut s, &["SOT23.kicad_mod", "SOT23.step"], "")).unwrap();
        assert_eq!(done.arr("footprints").unwrap()[0], Val::Str("cam:SOT23".into()));
        assert_eq!(done.arr("models").unwrap()[0], Val::Str("${KIPRJMOD}/libraries/cam.3dshapes/SOT23.step".into()));
        // The part added from it shows its model; another model, given it, shows instead, undone as one step.
        assert_eq!(error(s.cli_line(r#"{"cmd":"add","footprint":"cam:SOT23","at":[10,10]}"#)), None);
        let model =
            |s: &Session| s.board.footprints[0].models.iter().map(|m| m.file.rsplit('/').next().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(model(&s), ["SOT23.step"]);
        assert_eq!(Val::parse(&import(&mut s, &["other.wrl"], r#","reference":"ref1""#)).unwrap().str("model_of"), Some("REF1"));
        assert_eq!(model(&s), ["other.wrl"]);
        s.cli_line(r#"{"cmd":"undo"}"#);
        assert_eq!(model(&s), ["SOT23.step"]);
        let errors = [
            import(&mut s, &[], ""),
            import(&mut s, &["other.wrl"], r#","reference":"U9""#),
            import(&mut s, &["SOT23.kicad_mod"], r#","reference":"REF1""#),
        ];
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            errors.map(error),
            [
                Some("give files: the paths of a part's files (.kicad_sym, .kicad_mod, .step) or of the zip they came in".into()),
                Some("no part U9 on the board".into()),
                Some("no 3D model for REF1 among the files (a .step, .stp or .wrl, alone or zipped); the others went in".into()),
            ]
        );
    }

    #[test]
    fn makes_a_part_agents_then_add() {
        let dir = std::env::temp_dir().join(format!("cuprita-new-part-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Session::default();
        let made =
            s.cli_line(&format!(r#"{{"cmd":"new","dir":{},"name":"glow"}}"#, cuprita_core::json::json_str(&dir.display().to_string())));
        assert!(made.starts_with(r#"{"board":"glow.kicad_pcb""#), "{made}");
        let part = r#"{"cmd":"new_part","name":"TMP117","description":"Temperature sensor","pins":[{"name":"V+","type":"power_in","side":"top"},
            {"name":"SCL","type":"input"},{"name":"GND","type":"power_in","side":"bottom"},{"name":"ALERT","type":"output","side":"right"},
            {"name":"SDA","type":"bidirectional"},{"name":"ADD0","type":"input","side":"right"},{"name":"EP"}],"package":{"preset":"dfn","pitch_mm":0.65}}"#;
        let done = Val::parse(&s.cli_line(&part.replace('\n', " "))).unwrap();
        assert_eq!(done.arr("symbol"), Some(&[Val::Str("glow:TMP117".into())][..]), "{done:?}");
        assert_eq!(done.arr("footprint"), Some(&[Val::Str("glow:DFN-6-1EP_3x2.35mm_P0.65mm_EP1.55x1.75mm".into())][..]));
        // `add` finds it in the project's library, its pins on the sheet.
        let added = s.cli_line(r#"{"cmd":"add","symbol":"glow:TMP117","at":[50.8,50.8]}"#);
        assert!(added.starts_with(r#"{"reference":"U1","symbol":"glow:TMP117""#), "{added}");
        // Mistakes say what and how.
        let mut error = |args: &str| s.cli_line(&format!(r#"{{"cmd":"new_part","name":"X",{args}}}"#));
        assert_eq!(
            error(r#""pins":2,"package":{"preset":"BGA"}"#),
            r#"{"error":"preset BGA: one of SOIC, TSSOP, DFN, QFN, LQFP, Pin header, Pin header, 2 rows, DIP"}"#
        );
        assert!(
            error(r#""pins":[{"type":"pwr"}],"footprint":"a:b""#).starts_with(r#"{"error":"pin 1: type pwr isn't KiCad's: input, output"#)
        );
        assert_eq!(error(r#""footprint":"a:b""#), r#"{"error":"missing pins: how many, or a list of {number, name, type, side}"}"#);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
