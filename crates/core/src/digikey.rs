//! A part number looked up at Digi-Key with the user's own API key, and the KiCad symbol and
//! fields that place it; see `crates/core/digikey.md`.

use crate::json::{Val, json_str};
use crate::library::{self, LibPart};
use crate::tr::{tr, trf};
use std::io::Write;
use std::process::{Command, Stdio};

/// The parts KiCad draws with one symbol whatever their number: Digi-Key's category for them
/// (its older names too), the `Device` symbol, and the parameter that is their value (none: the
/// part number, as KiCad's diodes and LEDs show theirs). Their pins are KiCad's: an LED's and a
/// diode's 1 the cathode, as KiCad's footprints for them number it.
const GENERIC: [(&str, &str, &str); 10] = [
    ("Chip Resistor - Surface Mount", "R", "Resistance"),
    ("Ceramic Capacitors", "C", "Capacitance"),
    ("Fixed Inductors", "L", "Inductance"),
    ("LED Indication - Discrete", "LED", ""),
    ("LED White Lighting", "LED", ""),
    ("LED Color Lighting", "LED", ""),
    ("Single Diodes", "D", ""),
    ("Diodes - Rectifiers - Single", "D", ""),
    ("Single Zener Diodes", "D_Zener", ""),
    ("Diodes - Zener - Single", "D_Zener", ""),
];

/// The fields a part's symbol takes on a sheet: name, value.
pub type Fields = Vec<(&'static str, String)>;

/// A part as Digi-Key sells it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Part {
    /// Its maker's part number, as Digi-Key writes it.
    pub mpn: String,
    pub maker: String,
    /// What it is: "IC OPAMP GP 2 CIRCUIT 8SOIC".
    pub about: String,
    pub datasheet: String,
    /// Its narrowest category: "Chip Resistor - Surface Mount".
    pub category: String,
    /// Its parameters by Digi-Key's names: ("Package / Case", "0603 (1608 Metric)").
    pub params: Vec<(String, String)>,
}

/// The user's key: their Digi-Key app's Client ID and Secret.
pub type Key = (String, String);

/// The part numbered `mpn` (any case) at Digi-Key, asked with the user's `key`.
// shortcut: a token for each lookup (two requests); keep it its 10 minutes when lookups come in
// bulk (a BOM's).
pub fn lookup(mpn: &str, (id, secret): &Key) -> Result<Part, String> {
    let form = format!("client_id={}&client_secret={}&grant_type=client_credentials", encode(id), encode(secret));
    let reply = curl("https://api.digikey.com/v1/oauth2/token", &[], &form)?;
    let token = reply.str("access_token").ok_or_else(|| error(&reply))?;
    let headers = [format!("X-DIGIKEY-Client-Id: {id}"), format!("Authorization: Bearer {token}"), "Content-Type: application/json".into()];
    let body = format!(r#"{{"Keywords":{},"Limit":10}}"#, json_str(mpn.trim()));
    found(&curl("https://api.digikey.com/products/v4/search/keyword", &headers, &body)?, mpn)
}

/// The user's key in the environment: `DIGIKEY_CLIENT_ID` and `DIGIKEY_CLIENT_SECRET`, as KiCost
/// and Digi-Key's Python client name it, so their users have it set.
pub fn env_key() -> Option<Key> {
    let var = |name| std::env::var(name).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    Some((var("DIGIKEY_CLIENT_ID")?, var("DIGIKEY_CLIENT_SECRET")?))
}

/// Digi-Key's JSON reply to `data` posted to `url` with `headers`, by curl, which reads them from
/// its input: the key stays out of the computer's list of processes.
fn curl(url: &str, headers: &[String], data: &str) -> Result<Val, String> {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut config = format!("url = {}\ndata = {}\n", q(url), q(data));
    headers.iter().for_each(|h| config += &format!("header = {}\n", q(h)));
    let mut curl = Command::new("curl");
    curl.args(["-sS", "--max-time", "30", "-K", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = curl.spawn().map_err(|_| tr("looking a part up at Digi-Key takes curl: install it"))?;
    child.stdin.take().expect("piped").write_all(config.as_bytes()).map_err(|e| format!("curl: {e}"))?;
    let done = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !done.status.success() {
        return Err(format!("Digi-Key: {}", String::from_utf8_lossy(&done.stderr).trim()));
    }
    let text = String::from_utf8_lossy(&done.stdout);
    Val::parse(&text).ok_or_else(|| trf("Digi-Key's reply isn't JSON: {0}", &[&text.chars().take(200).collect::<String>()]))
}

/// `s` as a form's value.
fn encode(s: &str) -> String {
    let plain = |b: u8| b.is_ascii_alphanumeric() || b"-._~".contains(&b);
    s.bytes().map(|b| if plain(b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Digi-Key's own words for a reply that isn't the one asked for, and how to fix a key it refuses.
fn error(reply: &Val) -> String {
    let what = reply.str("ErrorMessage").or(reply.str("title")).unwrap_or_else(|| tr("an unexpected reply"));
    let why = reply.str("ErrorDetails").or(reply.str("detail")).map_or(String::new(), |d| format!(": {d}"));
    let said = format!("Digi-Key: {what}{why}");
    match reply.num("StatusCode").or(reply.num("status")) {
        Some(401.0 | 403.0) => trf("{0}; check the key's Client ID and Secret against your app at developer.digikey.com", &[&said]),
        _ => said,
    }
}

/// The part numbered `mpn` (any case) in Digi-Key's reply to a keyword search.
fn found(reply: &Val, mpn: &str) -> Result<Part, String> {
    let list = |k| reply.arr(k).unwrap_or_default().iter();
    let number = |p: &Val| p.str("ManufacturerProductNumber").unwrap_or_default().to_string();
    let Some(p) = list("ExactMatches").chain(list("Products")).find(|p| number(p).eq_ignore_ascii_case(mpn.trim())) else {
        if reply.get("Products").is_none() {
            return Err(error(reply));
        }
        let near: Vec<String> = list("Products").map(number).take(5).collect();
        return Err(match near.is_empty() {
            true => trf("Digi-Key has no part numbered {0}", &[&mpn]),
            false => trf("Digi-Key has no part numbered {0}; it has {1}", &[&mpn, &near.join(", ")]),
        });
    };
    let text = |path: &[&str]| p.at(path).and_then(|v| if let Val::Str(s) = v { Some(s.trim().to_string()) } else { None });
    let mut category = p.get("Category");
    while let Some(child) = category.and_then(|c| c.arr("ChildCategories")).and_then(<[Val]>::first) {
        category = Some(child);
    }
    let datasheet = text(&["DatasheetUrl"]).unwrap_or_default();
    Ok(Part {
        mpn: number(p),
        maker: text(&["Manufacturer", "Name"]).unwrap_or_default(),
        about: text(&["Description", "ProductDescription"]).unwrap_or_default(),
        // Some links leave their scheme out.
        datasheet: if datasheet.starts_with("//") { format!("https:{datasheet}") } else { datasheet },
        category: category.and_then(|c| c.str("Name")).unwrap_or_default().to_string(),
        params: (p.arr("Parameters").unwrap_or_default().iter())
            .filter_map(|q| Some((q.str("ParameterText")?.to_string(), q.str("ValueText")?.to_string())))
            .collect(),
    })
}

impl Part {
    /// Its parameter Digi-Key names `name`.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|p| p.0 == name).map(|p| p.1.as_str())
    }

    /// KiCad's symbol for it (its library's text, as `library::symbol` gives it), and the fields
    /// it takes there: the symbol KiCad names for its number (`library::named`), else the one
    /// KiCad draws every chip resistor, ceramic capacitor or fixed inductor with, valued as
    /// Digi-Key's; the symbol's own footprint, else its package's (`footprint`); its datasheet,
    /// maker and number.
    pub fn kicad(&self, symbols: &[LibPart], footprints: &[LibPart]) -> Result<(LibPart, String, Fields), String> {
        let generic = GENERIC.iter().find(|g| g.0 == self.category);
        // A Schottky diode is in the plain diodes' category, its technology saying so.
        let schottky = self.param("Technology").is_some_and(|t| t.contains("Schottky"));
        let device = generic.map(|g| if g.1 == "D" && schottky { "D_Schottky" } else { g.1 });
        let symbol = match (library::named(symbols, &self.mpn).first(), device) {
            (Some(&s), _) => s,
            (None, Some(name)) => symbols
                .iter()
                .find(|s| s.lib == "Device" && s.name == name)
                .ok_or_else(|| trf("KiCad's libraries have no Device:{0}", &[&name]))?,
            (None, None) => {
                let what = [&self.maker, &self.about, &self.category];
                let what: Vec<&str> = what.into_iter().map(String::as_str).filter(|s| !s.is_empty()).collect();
                return Err(trf("KiCad's libraries have no symbol for {0} ({1}): add one by what it is", &[&self.mpn, &what.join(", ")]));
            }
        };
        let read = std::fs::read_to_string(&symbol.path).map_err(|e| e.to_string());
        let text = read.and_then(|t| library::symbol(&t, &symbol.name)).map_err(|e| format!("{}: {e}", symbol.path.display()))?;
        let mut fields = vec![];
        match generic {
            Some(g) if g.2.is_empty() => fields.push(("Value", self.mpn.clone())),
            Some(g) => fields.extend(self.param(g.2).and_then(value).map(|v| ("Value", v))),
            None => {}
        }
        if library::field(&text, "Footprint").is_none_or(|f| f.is_empty())
            && let Some(fp) = self.footprint(&library::field(&text, "ki_fp_filters").unwrap_or_default(), footprints)
        {
            fields.push(("Footprint", fp));
        }
        if !self.datasheet.is_empty() {
            fields.push(("Datasheet", self.datasheet.clone()));
        }
        fields.extend([("Manufacturer", self.maker.clone()), ("MPN", self.mpn.clone())]);
        Ok((symbol.clone(), text, fields))
    }

    /// The footprint of its package that a symbol's `filters` allow (any without them), the
    /// plainest (its name shortest), of the body's width when Digi-Key gives one: for "8-SOIC
    /// (0.154\", 3.90mm Width)" `SOIC-8_3.9x4.9mm_P1.27mm`, for "0603 (1608 Metric)" a symbol's
    /// `R_0603_1608Metric`.
    fn footprint(&self, filters: &str, footprints: &[LibPart]) -> Option<String> {
        let (forms, width) = self.package();
        let fits = |f: &LibPart| filters.split_whitespace().next().is_none() || filters.split_whitespace().any(|g| library::fits(g, f));
        (footprints.iter())
            .filter(|f| forms.iter().any(|form| whole(&f.name, form)) && fits(f))
            .min_by_key(|f| (width.as_ref().is_none_or(|w| !f.name.contains(w.as_str())), f.name.len()))
            .map(LibPart::id)
    }

    /// Its package as KiCad's footprints name it, from Digi-Key's "Package / Case" and "Supplier
    /// Device Package": `SOIC-8` for "8-SOIC", `0603_1608Metric` for "0603 (1608 Metric)", what
    /// holds a digit of the rest ("SC-74A, SOT-753"); and its body's width (`_3.9x` for "(0.154\",
    /// 3.90mm Width)").
    fn package(&self) -> (Vec<String>, Option<String>) {
        let (mut forms, mut width) = (vec![], None);
        for v in ["Package / Case", "Supplier Device Package"].into_iter().filter_map(|k| self.param(k)) {
            // Its kinds, split at the commas outside parentheses: what each is called, and says in them.
            let mut depth = 0;
            for kind in v.split(|c| {
                depth += (c == '(') as i32 - (c == ')') as i32;
                c == ',' && depth == 0
            }) {
                let (name, said) = kind.split_once('(').map_or((kind, ""), |(n, s)| (n, s.trim_end_matches(')')));
                let name = name.split_whitespace().next().unwrap_or_default();
                let wide = said.split(", ").find_map(|s| s.strip_suffix("mm Width")?.parse::<f64>().ok());
                width = width.or(wide.map(|w| format!("_{w}x")));
                let form = match (said.strip_suffix(" Metric"), name.split_once('-')) {
                    (Some(metric), _) => format!("{name}_{metric}Metric"),
                    (_, Some((pins, family))) if pins.parse::<u32>().is_ok() && family.starts_with(char::is_alphabetic) => {
                        format!("{family}-{pins}")
                    }
                    _ => name.to_string(),
                };
                if form.contains(|c: char| c.is_ascii_digit()) {
                    forms.push(form);
                }
            }
        }
        (forms, width)
    }
}

/// Whether footprint name `name` holds `form` whole: from its start or a `_` to its end or a `_`
/// (`SOT-23` in `SOT-23_Handsoldering`, not in `SOT-23-5`), any case.
fn whole(name: &str, form: &str) -> bool {
    let (name, form) = (name.to_ascii_lowercase(), form.to_ascii_lowercase());
    let edge = |b: Option<&u8>| matches!(b, None | Some(b'_'));
    name.match_indices(&form).any(|(i, _)| (i == 0 || edge(name.as_bytes().get(i - 1))) && edge(name.as_bytes().get(i + form.len())))
}

/// A value as Digi-Key writes it ("0.1 µF", "4.7 kOhms", "1000 pF") as KiCad's values hold it:
/// `100n`, `4.7k`, `1n`.
fn value(v: &str) -> Option<String> {
    let (number, unit) = v.trim().split_once(' ')?;
    let mut x: f64 = number.parse().ok()?;
    let mut e = match unit.chars().next()? {
        'p' => -4,
        'n' => -3,
        'µ' | 'μ' | 'u' => -2,
        'm' => -1,
        'k' => 1,
        'M' => 2,
        'G' => 3,
        _ => 0,
    };
    while x >= 1000.0 && e < 3 {
        (x, e) = (x / 1000.0, e + 1);
    }
    while x > 0.0 && x < 1.0 && e > -4 {
        (x, e) = (x * 1000.0, e - 1);
    }
    Some(format!("{}{}", (x * 1000.0).round() / 1000.0, ["p", "n", "u", "m", "", "k", "M", "G"][(e + 4) as usize]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A keyword search's reply, as Digi-Key's Product Information V4 writes it (cut short).
    const REPLY: &str = r#"{"Products":[
        {"ManufacturerProductNumber":"LM358DRG4","Manufacturer":{"Id":296,"Name":"Texas Instruments"}},
        {"ManufacturerProductNumber":"LM358DR","Manufacturer":{"Id":296,"Name":"Texas Instruments"},
         "Description":{"ProductDescription":"IC OPAMP GP 2 CIRCUIT 8SOIC","DetailedDescription":"General Purpose Amplifier 2 Circuit 8-SOIC"},
         "DatasheetUrl":"//www.ti.com/lit/ds/symlink/lm358.pdf",
         "Parameters":[{"ParameterId":16,"ParameterText":"Package / Case","ParameterType":"String","ValueId":"1","ValueText":"8-SOIC (0.154\", 3.90mm Width)"},
                       {"ParameterId":1291,"ParameterText":"Supplier Device Package","ParameterType":"String","ValueId":"2","ValueText":"8-SOIC"}],
         "Category":{"CategoryId":32,"Name":"Integrated Circuits (ICs)","ChildCategories":[{"CategoryId":2042,"Name":"Linear",
            "ChildCategories":[{"CategoryId":687,"Name":"Instrumentation, Op Amps, Buffer Amps","ChildCategories":[]}]}]}}],
        "ProductsCount":2,"ExactMatches":[]}"#;

    #[test]
    fn reads_the_part_numbered_from_a_keyword_search() {
        let part = found(&Val::parse(REPLY).unwrap(), " lm358dr").unwrap();
        assert_eq!(
            (part.mpn.as_str(), part.maker.as_str(), part.about.as_str()),
            ("LM358DR", "Texas Instruments", "IC OPAMP GP 2 CIRCUIT 8SOIC")
        );
        assert_eq!(
            (part.datasheet.as_str(), part.category.as_str()),
            ("https://www.ti.com/lit/ds/symlink/lm358.pdf", "Instrumentation, Op Amps, Buffer Amps")
        );
        assert_eq!(part.param("Supplier Device Package"), Some("8-SOIC"));
        assert_eq!(
            found(&Val::parse(REPLY).unwrap(), "LM358D").unwrap_err(),
            "Digi-Key has no part numbered LM358D; it has LM358DRG4, LM358DR"
        );
        assert_eq!(found(&Val::parse(r#"{"Products":[]}"#).unwrap(), "X1").unwrap_err(), "Digi-Key has no part numbered X1");
        // Its errors in its words, as it answers them (a token's, then a search's).
        let token = r#"{"ErrorResponseVersion":"3.0.0.0","StatusCode": 401,"ErrorMessage":"Invalid clientId","ErrorDetails":"Send valid client_id"}"#;
        let (fix, search) = ("; check the key's Client ID", r#"{"title":"Too Many Requests","status":429,"detail":"Try again later"}"#);
        assert!(error(&Val::parse(token).unwrap()).starts_with(&format!("Digi-Key: Invalid clientId: Send valid client_id{fix}")));
        assert_eq!(found(&Val::parse(search).unwrap(), "X1").unwrap_err(), "Digi-Key: Too Many Requests: Try again later");
        assert_eq!(encode("a b&c=é"), "a%20b%26c%3D%C3%A9");
    }

    #[test]
    fn writes_values_as_kicad_does() {
        let cases = [("10 kOhms", "10k"), ("0.1 µF", "100n"), ("1000 pF", "1n"), ("4.7 µH", "4.7u"), ("0 Ohms", "0"), ("1 MOhms", "1M")];
        for (digikey, kicad) in cases {
            assert_eq!(value(digikey).as_deref(), Some(kicad), "{digikey}");
        }
        assert_eq!(value("-"), None);
    }

    #[test]
    fn places_a_part_with_kicads_symbol_and_its_packages_footprint() {
        let dir = std::env::temp_dir().join(format!("cuprita-digikey-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sym = |name: &str, footprint: &str, filters: &str| {
            format!(
                "(symbol \"{name}\" (property \"Reference\" \"U\") (property \"Value\" \"{name}\") (property \"Footprint\" \"{footprint}\") \
                 (property \"ki_fp_filters\" \"{filters}\"))"
            )
        };
        let lib = |name: &str, symbols: &[String]| {
            let path = dir.join(format!("{name}.kicad_sym"));
            std::fs::write(&path, format!("(kicad_symbol_lib {})", symbols.join(" "))).unwrap();
            path
        };
        let (device, opamps) = (
            lib("Device", &[sym("R", "", "R_*"), sym("LED", "", "LED* LED_SMD:* LED_THT:*"), sym("D_Schottky", "", "D_*")]),
            lib("Amp", &[sym("LM358", "", "SOIC*3.9x4.9mm*P1.27mm* DIP*W7.62mm*"), sym("NE555D", "SO:SOIC-8", "")]),
        );
        let symbols: Vec<LibPart> = [
            ("Device", "R", &device),
            ("Device", "LED", &device),
            ("Device", "D_Schottky", &device),
            ("Amp", "LM358", &opamps),
            ("Amp", "NE555D", &opamps),
        ]
        .map(|(lib, name, path)| LibPart { lib: lib.into(), name: name.into(), path: path.clone(), about: String::new() })
        .into();
        let names = [
            "R_0603_1608Metric_Pad0.98x0.95mm_HandSolder",
            "R_0603_1608Metric",
            "R_0201_0603Metric",
            "C_0603_1608Metric",
            "SOIC-8-1EP_3.9x4.9mm_P1.27mm_EP2.29x3mm",
            "SOIC-8_5.3x5.3mm_P1.27mm",
            "SOIC-8_3.9x4.9mm_P1.27mm",
            "DIP-8_W7.62mm",
            "LED_PLCC_2835_Handsoldering",
            "LED_PLCC_2835",
        ];
        let footprints: Vec<LibPart> =
            names.map(|n| LibPart { lib: "L".into(), name: n.into(), path: PathBuf::new(), about: String::new() }).into();
        let part = |mpn: &str, category: &str, params: &[(&str, &str)]| Part {
            mpn: mpn.into(),
            maker: "M".into(),
            category: category.into(),
            params: params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            ..Part::default()
        };
        let placed = |p: Part| p.kicad(&symbols, &footprints).map(|(s, _, fields)| (s.id(), fields));

        // A chip resistor: KiCad's resistor, valued, on its size's plainest footprint.
        let r = part(
            "RC0603FR-0710KL",
            "Chip Resistor - Surface Mount",
            &[("Resistance", "10 kOhms"), ("Package / Case", "0603 (1608 Metric)")],
        );
        let fields = |v: &[(&'static str, &str)]| v.iter().map(|(k, v)| (*k, v.to_string())).collect::<Vec<_>>();
        let made = fields(&[("Value", "10k"), ("Footprint", "L:R_0603_1608Metric"), ("Manufacturer", "M"), ("MPN", "RC0603FR-0710KL")]);
        assert_eq!(placed(r), Ok(("Device:R".into(), made)));
        // Its own symbol, on the footprint of its package and width the symbol allows.
        let lm358 = part("LM358DR", "Instrumentation, Op Amps, Buffer Amps", &[("Package / Case", "8-SOIC (0.154\", 3.90mm Width)")]);
        let fp = placed(lm358.clone()).unwrap().1;
        assert_eq!((fp[0].0, fp[0].1.as_str()), ("Footprint", "L:SOIC-8_3.9x4.9mm_P1.27mm"));
        assert_eq!(
            placed(Part { params: vec![("Package / Case".into(), "8-DIP (0.300\", 7.62mm)".into())], ..lm358 }).unwrap().1[0].1,
            "L:DIP-8_W7.62mm"
        );
        // An LED KiCad has no symbol of its own for (the owner's, as Digi-Key describes it): KiCad's
        // LED. Its "1411 (3528 Metric)" is the trade's 2835 and also its 3528, pads apart: none
        // is guessed, Inspect's Choose… lists the LED footprints.
        let led = part(
            "JB2835BWT-G-U40GA0000-N0000001",
            "LED White Lighting",
            &[("Package / Case", "1411 (3528 Metric)"), ("Supplier Device Package", "1411")],
        );
        let made = fields(&[("Value", "JB2835BWT-G-U40GA0000-N0000001"), ("Manufacturer", "M"), ("MPN", "JB2835BWT-G-U40GA0000-N0000001")]);
        assert_eq!(placed(led), Ok(("Device:LED".into(), made)));
        // One whose package names it as KiCad's footprints do lands on it.
        let indicator = part("LTST-C190KRKT", "LED Indication - Discrete", &[("Package / Case", "2835")]);
        assert_eq!(placed(indicator).unwrap().1[1], ("Footprint", "L:LED_PLCC_2835".to_string()));
        // A Schottky diode, from the plain diodes' category.
        assert_eq!(placed(part("BAT54", "Single Diodes", &[("Technology", "Schottky")])).unwrap().0, "Device:D_Schottky");
        // A symbol's own footprint stays; a part KiCad has no symbol for says so.
        assert_eq!(placed(part("NE555DR", "Timers", &[("Package / Case", "8-SOIC")])).unwrap().1[0].0, "Manufacturer");
        let none = placed(Part { about: "IC BUCK".into(), ..part("TPS9999", "DC DC", &[]) });
        assert_eq!(none, Err("KiCad's libraries have no symbol for TPS9999 (M, IC BUCK, DC DC): add one by what it is".into()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
