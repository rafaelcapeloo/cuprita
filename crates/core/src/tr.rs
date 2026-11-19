//! Texts core gives people to read (checks' messages, why a route or an edit stops), in their
//! language: the app hands `set` its catalog's lookup (`crates/app/src/tr.rs`); left unset, as
//! for agents and tests, they stay English. `tr("…")`, `trf("… {0} …", &[&x])` and `count` work
//! as the app's, whose catalog test reads core's marks too.

use std::fmt::Display;
use std::sync::OnceLock;

/// Marks a text for the catalogs where it is kept in a constant; `tr` translates it where it shows.
macro_rules! t {
    ($text:literal) => {
        $text
    };
}
pub(crate) use t;

/// The lookup `set` was given.
static TRANSLATE: OnceLock<fn(&'static str) -> &'static str> = OnceLock::new();

/// Has core's texts translated by `translate` (the app's catalog lookup), once.
pub fn set(translate: fn(&'static str) -> &'static str) {
    let _ = TRANSLATE.set(translate);
}

/// `english` in the language shown (itself with no lookup set, or none for it).
pub fn tr(english: &'static str) -> &'static str {
    TRANSLATE.get().map_or(english, |translate| translate(english))
}

/// `template` in the language shown, its `{0}`, `{1}`… given `args`.
pub fn trf(template: &'static str, args: &[&dyn Display]) -> String {
    fill(tr(template), args)
}

/// `text`'s `{0}`, `{1}`… given `args`, in one pass, so a value holding `{1}` stays as it is.
pub fn fill(text: &str, args: &[&dyn Display]) -> String {
    let (mut out, mut rest) = (String::new(), text);
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let hole = rest.find('}').and_then(|end| Some((args.get(rest[1..end].parse::<usize>().ok()?)?, end + 1)));
        let (text, taken) = hole.map_or(("{".to_string(), 1), |(arg, len)| (arg.to_string(), len));
        out += &text;
        rest = &rest[taken..];
    }
    out + rest
}

/// `n` of a thing, in the language shown: "1 zone", "3 zones". `forms` is the thing (the app
/// marks it `n!`): its word for one, and for many after a `|` where an s doesn't make it ("part to
/// add|parts to add"); a catalog gives both ("zona|zonas").
// shortcut: one form for 1, another for the rest, as English and Portuguese count; languages
// with more forms (Polish, Russian) want gettext's plural rules.
pub fn count(n: usize, forms: &'static str) -> String {
    let forms = tr(forms);
    let (one, many) = forms.split_once('|').map_or((forms.to_string(), format!("{forms}s")), |(o, m)| (o.into(), m.into()));
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_templates_in_one_pass_and_counts() {
        assert_eq!(trf("{0} of {1}", &[&2, &"3"]), "2 of 3");
        assert_eq!(fill("{1}, {0} {2} {x}", &[&"{1}", &"b"]), "b, {1} {2} {x}");
        assert_eq!((count(1, "zone"), count(2, "zone")), ("1 zone".to_string(), "2 zones".to_string()));
        assert_eq!(count(2, "part to add|parts to add"), "2 parts to add");
    }
}
