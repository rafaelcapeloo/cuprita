//! Translations: the English text the app shows, in the user's language, from the catalogs in
//! `crates/app/i18n/` (gettext's `.po`, which translators' tools open), built into the app. The
//! language is the one Settings keep, else the system's (`LC_ALL`, `LC_MESSAGES`, `LANG`; the
//! browser's), else English. `tr("…")` translates a text, `trf("… {0} …", &[&x])` a template
//! with its values, `t!("…")` marks a text kept in a constant, translated where it shows, and
//! `n!("zone")` a counted thing (`crate::count`).

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A language Cuprita speaks: its tag (BCP 47), its own name, its catalog.
pub(crate) struct Language {
    pub(crate) tag: &'static str,
    pub(crate) name: &'static str,
    po: &'static str,
}

/// English first: the text in the code, with no catalog; then by their own names.
pub(crate) const LANGUAGES: [Language; 2] = [
    Language { tag: "en", name: "English", po: "" },
    Language { tag: "pt-BR", name: "Português (Brasil)", po: include_str!("../i18n/pt-BR.po") },
];

/// Marks a text for the catalogs where it is kept in a constant; `tr` translates it where it shows.
macro_rules! t {
    ($text:literal) => {
        $text
    };
}
pub(crate) use t;

/// Marks a counted thing for the catalogs (`crate::count`): its word for one, and for many after a
/// `|` where an s doesn't make it; a catalog gives both forms.
macro_rules! n {
    ($forms:literal) => {
        $forms
    };
}
pub(crate) use n;

/// The language shown: its place in `LANGUAGES`.
static SHOWN: AtomicUsize = AtomicUsize::new(0);

/// `english` in the language shown (itself where the catalog lacks it).
pub(crate) fn tr(english: &'static str) -> &'static str {
    match SHOWN.load(Ordering::Relaxed) {
        0 => english,
        k => catalog(k).get(english).copied().unwrap_or(english),
    }
}

/// `template` in the language shown, its `{0}`, `{1}`… given `args` (in one pass, so a value
/// holding `{1}` stays as it is: `cuprita_core::tr::fill`).
pub(crate) fn trf(template: &'static str, args: &[&dyn std::fmt::Display]) -> String {
    cuprita_core::tr::fill(tr(template), args)
}

/// The language shown.
pub(crate) fn shown() -> &'static Language {
    &LANGUAGES[SHOWN.load(Ordering::Relaxed)]
}

/// The language kept in Settings, if one is: else the system's shows.
pub(crate) fn chosen() -> Option<&'static Language> {
    let tag = crate::recent::kept("language")?;
    LANGUAGES.iter().find(|l| l.tag == tag)
}

/// Shows `language` (`None`: the system's) and keeps the choice for the next run.
pub(crate) fn choose(language: Option<&Language>) {
    crate::recent::keep("language", language.map_or("", |l| l.tag));
    read();
}

/// Takes up the language kept, else the system's, for core's texts too.
pub(crate) fn read() {
    cuprita_core::tr::set(tr);
    let tag = chosen().map_or_else(system, |l| l.tag.to_string());
    SHOWN.store(best(&tag), Ordering::Relaxed);
}

/// The language the system speaks, of those Cuprita does (else English).
pub(crate) fn system_language() -> &'static Language {
    &LANGUAGES[best(&system())]
}

/// The system's language, as a tag: `pt-BR` from `pt_BR.UTF-8`.
fn system() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    let tag = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| std::env::var(v).ok().filter(|v| !v.is_empty()));
    #[cfg(target_arch = "wasm32")]
    let tag = web_sys::window().and_then(|w| w.navigator().language());
    let tag = tag.unwrap_or_default();
    tag.split(['.', '@']).next().unwrap_or_default().replace('_', "-")
}

/// The place in `LANGUAGES` of the one that speaks `tag`: the same tag, else the same language
/// (`pt` takes `pt-BR`), else English.
fn best(tag: &str) -> usize {
    let lang = |t: &str| t.split('-').next().unwrap_or_default().to_lowercase();
    let same = LANGUAGES.iter().position(|l| l.tag.eq_ignore_ascii_case(tag));
    same.or_else(|| LANGUAGES.iter().position(|l| lang(l.tag) == lang(tag))).unwrap_or(0)
}

/// The catalog of `LANGUAGES[k]`, read once: English text → its translation.
fn catalog(k: usize) -> &'static HashMap<&'static str, &'static str> {
    static READ: OnceLock<Vec<HashMap<&'static str, &'static str>>> = OnceLock::new();
    &READ.get_or_init(|| LANGUAGES.iter().map(|l| read_po(l.po)).collect())[k]
}

/// The pairs of a `.po` file: each `msgid` with its `msgstr`, their quoted lines joined and
/// unescaped; the header (an empty `msgid`) and texts not translated yet left out. Read once and
/// kept for the run.
fn read_po(po: &str) -> HashMap<&'static str, &'static str> {
    let (mut map, mut id, mut text, mut in_id) = (HashMap::new(), String::new(), String::new(), true);
    let mut done = |id: &mut String, text: &mut String| {
        if !id.is_empty() && !text.is_empty() {
            let keep = |s: &mut String| &*Box::leak(std::mem::take(s).into_boxed_str());
            map.insert(keep(id), keep(text));
        }
        id.clear();
        text.clear();
    };
    for line in po.lines().map(str::trim) {
        if let Some(q) = line.strip_prefix("msgid ") {
            done(&mut id, &mut text);
            (in_id, id) = (true, unquote(q));
        } else if let Some(q) = line.strip_prefix("msgstr ") {
            (in_id, text) = (false, unquote(q));
        } else if line.starts_with('"') {
            if in_id { &mut id } else { &mut text }.push_str(&unquote(line));
        }
    }
    done(&mut id, &mut text);
    map
}

/// A `.po` quoted string's text: `"say \"hi\"\n"` → `say "hi"` and a new line.
fn unquote(q: &str) -> String {
    let q = q.trim();
    let (mut out, mut chars) = (String::new(), q.strip_prefix('"').and_then(|q| q.strip_suffix('"')).unwrap_or(q).chars());
    while let Some(c) = chars.next() {
        out.push(match (c, chars.clone().next()) {
            ('\\', Some(e)) => {
                chars.next();
                match e {
                    'n' => '\n',
                    't' => '\t',
                    e => e,
                }
            }
            (c, _) => c,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The texts `tr("…")`, `trf("…"`, `t!("…")` and `n!("…")` mark in `src`, as the strings
    /// they are; whether each is a counted thing's.
    fn marked(src: &str) -> Vec<(String, bool)> {
        let mut out = vec![];
        for marker in ["tr(", "trf(", "t!(", "n!("] {
            for (at, _) in src.match_indices(marker) {
                let rest = src[at + marker.len()..].trim_start();
                if src[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') || !rest.starts_with('"') {
                    continue;
                }
                let (mut text, mut chars) = (String::new(), rest[1..].chars().peekable());
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => text.push('\n'),
                            // A line ending escaped: the next one's indent goes too.
                            Some('\n') => while chars.next_if(|c| c.is_whitespace()).is_some() {},
                            Some(c) => text.push(c),
                            None => break,
                        },
                        c => text.push(c),
                    }
                }
                out.push((text, marker == "n!("));
            }
        }
        out
    }

    /// Every text the app, core and the renderer mark for the catalogs, and every command's name
    /// and hint; whether each is a counted thing's.
    fn english() -> BTreeSet<(String, bool)> {
        let mut all = BTreeSet::new();
        let src = |crate_dir: &str| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(crate_dir).join("src");
        let mut dirs = vec![src("."), src("../core"), src("../render")];
        while let Some(dir) = dirs.pop() {
            for f in std::fs::read_dir(dir).unwrap().map(|f| f.unwrap().path()) {
                match f.extension() {
                    None => dirs.push(f),
                    Some(e) if e == "rs" && !f.ends_with("tr.rs") => all.extend(marked(&std::fs::read_to_string(&f).unwrap())),
                    _ => {}
                }
            }
        }
        for c in crate::cmd::Cmd::ALL {
            for web in [false, true] {
                let about = c.about_in(web);
                all.extend([(about.name.to_string(), false), (about.hint.to_string(), false)]);
            }
        }
        all
    }

    #[test]
    fn every_catalog_translates_all_the_app_says_and_nothing_else() {
        let marked = english();
        let english: BTreeSet<&str> = marked.iter().map(|(e, _)| e.as_str()).collect();
        for (k, lang) in LANGUAGES.iter().enumerate().skip(1) {
            let catalog = catalog(k);
            let missing: Vec<_> = english.iter().filter(|e| !catalog.contains_key(**e)).collect();
            let unused: Vec<_> = catalog.keys().filter(|t| !english.contains(**t)).collect();
            assert!(missing.is_empty() && unused.is_empty(), "i18n/{}.po: missing {missing:#?}; not used {unused:#?}", lang.tag);
            for (thing, _) in marked.iter().filter(|(_, counted)| *counted) {
                let forms = catalog[thing.as_str()];
                assert_eq!(forms.matches('|').count(), 1, "{}: {thing:?} counts as {forms:?}, not one|many", lang.tag);
            }
            for (en, text) in catalog {
                let holes = |s: &str| (0..4).filter(|k| s.contains(&format!("{{{k}}}"))).collect::<Vec<_>>();
                assert_eq!(holes(en), holes(text), "{}: {en:?} and {text:?} hold different values", lang.tag);
            }
        }
    }

    #[test]
    fn reads_po_files_and_picks_the_language_spoken() {
        let po = "msgid \"\"\nmsgstr \"Language: pt\\n\"\n\n# a comment\nmsgid \"Say \\\"hi\\\"\"\nmsgstr \"Diga \"\n\"\\\"oi\\\"\"\n\nmsgid \"Later\"\nmsgstr \"\"\n";
        let map = read_po(po);
        assert_eq!((map.len(), map.get("Say \"hi\"").copied()), (1, Some("Diga \"oi\"")));
        let tags = ["pt-BR", "pt", "pt-PT", "de-DE", ""];
        assert_eq!(tags.map(best), [1, 1, 1, 0, 0]);
        let texts = marked("x(tr(\"a\\\"b\"), t!(\"c \\\n    d\"), str(\"no\"), tr(no), trf(\n \"e\"), n!(\"zone\"))");
        let texts: Vec<_> = texts.iter().map(|(t, counted)| (t.as_str(), *counted)).collect();
        assert_eq!(texts, [("a\"b", false), ("e", false), ("c d", false), ("zone", true)]);
        assert_eq!(trf("{0} of {1}", &[&2, &"3"]), "2 of 3");
        assert_eq!(trf("{1}, {0} {2} {x}", &[&"{1}", &"b"]), "b, {1} {2} {x}");
    }
}
