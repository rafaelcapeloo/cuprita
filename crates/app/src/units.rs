//! The units lengths show in: millimetres or mils (Ctrl+U, KiCad's key, or a click on the status
//! bar's units), kept for the next run in Cuprita's data folder (the browser's local storage).
//! Files keep KiCad's millimetres.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether lengths show in mils.
static MILS: AtomicBool = AtomicBool::new(false);

pub(crate) fn mils() -> bool {
    MILS.load(Ordering::Relaxed)
}

/// The units' name, as lengths end.
pub(crate) fn name() -> &'static str {
    if mils() { "mil" } else { "mm" }
}

/// Shows lengths in the other units; keeps the choice for the next run.
pub(crate) fn toggle() {
    MILS.store(!mils(), Ordering::Relaxed);
    crate::recent::keep("units", name());
}

/// Takes up the units kept from the last run.
pub(crate) fn read() {
    MILS.store(crate::recent::kept("units").is_some_and(|u| u == "mil"), Ordering::Relaxed);
}

/// A length in nm as a number of the units, every place kept (for columns): mm to 3 decimals,
/// mils to 1 (a 0.1 mil step is 2.54 µm).
pub(crate) fn fixed(nm: impl Into<f64>) -> String {
    fixed_in(nm.into(), mils())
}

/// The same without trailing zeros: `0.15`, `100`.
pub(crate) fn num(nm: impl Into<f64>) -> String {
    trim(fixed(nm))
}

/// A length in nm with its units: `0.15 mm`, `5.9 mil`.
pub(crate) fn len(nm: impl Into<f64>) -> String {
    format!("{} {}", num(nm), name())
}

/// A length typed, in nm: a number in the units shown, or with its own (`0.2mm`, `8 mil`,
/// `0.01in`, `150um`), its decimal point a dot or a comma (`0,2`) as KiCad reads them; `None`
/// when it isn't one or doesn't fit a board.
pub(crate) fn parse(text: &str) -> Option<i32> {
    parse_in(text, mils())
}

fn parse_in(text: &str, mils: bool) -> Option<i32> {
    let t = text.trim().to_lowercase().replace(',', ".");
    let end = t.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))).unwrap_or(t.len());
    let nm_per = match t[end..].trim() {
        "" if mils => 25_400.0,
        "" | "mm" => 1e6,
        "mil" | "mils" | "th" => 25_400.0,
        "in" | "\"" => 25.4e6,
        "um" | "µm" => 1e3,
        _ => return None,
    };
    let nm = (t[..end].parse::<f64>().ok()? * nm_per).round();
    (nm.abs() <= i32::MAX as f64).then_some(nm as i32)
}

fn fixed_in(nm: f64, mils: bool) -> String {
    if mils { format!("{:.1}", nm / 25_400.0) } else { format!("{:.3}", nm / 1e6) }
}

fn trim(s: String) -> String {
    let t = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if t == "-0" { "0".into() } else { t.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_lengths_in_millimetres_or_mils() {
        let both = |nm: f64| (trim(fixed_in(nm, false)), trim(fixed_in(nm, true)));
        assert_eq!(both(150_000.0), ("0.15".into(), "5.9".into()));
        assert_eq!(both(2_540_000.0), ("2.54".into(), "100".into()));
        assert_eq!(both(-100.0), ("0".into(), "0".into()));
        assert_eq!(fixed_in(-1_270_000.0, false), "-1.270");
    }

    #[test]
    fn reads_lengths_in_the_units_shown_or_their_own() {
        let both = |t: &str| (parse_in(t, false), parse_in(t, true));
        assert_eq!(both("0.2"), (Some(200_000), Some(5_080)));
        assert_eq!(both(" 0.2 MM "), (Some(200_000), Some(200_000)));
        assert_eq!(both("0,2mm"), both("0.2mm"));
        assert_eq!(
            [both("8mil").0, both("0.01 in").0, both("150um").0, both("-1").0],
            [8 * 25_400, 254_000, 150_000, -1_000_000].map(Some)
        );
        // As shown (`len`), it reads back.
        assert_eq!(both("5.9 mil").1, Some(149_860));
        assert_eq!([both("").0, both("mm").0, both("1 ft").0, both("1.2.3").0, both("9999").0], [None; 5]);
    }
}
