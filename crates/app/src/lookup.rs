//! A part number looked up at Digi-Key from the schematic's add panel (A): its row there, the
//! lookup on a thread (`cuprita_core::digikey`), then the part placed as a symbol is, with its
//! value, footprint, datasheet, maker and number. Desktop only.

use crate::App;
use crate::tr::{tr, trf};
use cuprita_core::digikey::{self, Part};
use cuprita_core::library;
use eframe::egui;
use std::thread::JoinHandle;
use std::time::Duration;

/// A part number being looked up (the thread asking Digi-Key), or what went wrong.
pub(crate) enum Lookup {
    Asking(String, JoinHandle<Result<Part, String>>),
    Failed(String, String),
}

/// The add panel's row for `query` when it is a part number (one word holding a digit): whether
/// it comes first (`keyed`: the user's key is set; else KiCad's matches do), and what it says:
/// what a lookup gives, the lookup going on, or what went wrong.
pub(crate) fn row(query: &str, lookup: &Option<Lookup>, keyed: bool) -> Option<(bool, String)> {
    let mpn = part_number(query)?;
    let says = match lookup {
        Some(Lookup::Asking(m, _)) if m == mpn => trf("Asking Digi-Key for {0}…", &[&mpn]),
        Some(Lookup::Failed(m, e)) if m == mpn => e.clone(),
        _ => trf("{0} at Digi-Key: its symbol, footprint, datasheet and maker", &[&mpn]),
    };
    Some((keyed, says))
}

/// `query` when it is a part number: one word holding a digit (`LM358DR`, `C2040`), not a
/// library's part (`glow:TMP117`).
pub(crate) fn part_number(query: &str) -> Option<&str> {
    let mpn = query.trim();
    (!mpn.contains(|c: char| c.is_whitespace() || c == ':') && mpn.contains(|c: char| c.is_ascii_digit())).then_some(mpn)
}

impl App {
    /// Asks Digi-Key, on a thread, for the part numbered as the add panel's query; without the
    /// user's key, says how to get one.
    pub(crate) fn look_up(&mut self) {
        let mpn = self.part_query.trim().to_string();
        if !matches!(&self.lookup, Some(Lookup::Asking(m, _)) if *m == mpn) {
            let ask = mpn.clone();
            self.lookup = Some(match crate::settings::digikey_key() {
                Ok(key) => Lookup::Asking(mpn, std::thread::spawn(move || digikey::lookup(&ask, &key))),
                Err(no) => Lookup::Failed(mpn, tr(no).into()),
            });
        }
    }

    /// Once Digi-Key has answered the add panel's query, places its part (`Part::kicad`), or
    /// keeps what went wrong; a lookup the panel no longer shows is dropped.
    pub(crate) fn poll_lookup(&mut self, ctx: &egui::Context) {
        let Some(Lookup::Asking(mpn, thread)) = &self.lookup else { return };
        if !self.adding() || *mpn != self.part_query.trim() {
            return self.lookup = None;
        }
        if !thread.is_finished() {
            return ctx.request_repaint_after(Duration::from_millis(100));
        }
        let Some(Lookup::Asking(mpn, thread)) = self.lookup.take() else { return };
        let folder = self.folder();
        let symbols = self.symbols.get_or_insert_with(|| crate::parts::symbols(folder.as_deref()));
        let footprints = self.parts.get_or_insert_with(|| crate::parts::footprints(folder.as_deref()));
        let found = thread.join().unwrap_or_else(|_| Err(tr("the lookup stopped").into()));
        match found.and_then(|part| Ok((part.kicad(symbols, footprints)?, part))) {
            Ok(((symbol, text, fields), part)) => {
                let set = |name| fields.iter().find(|f| f.0 == name).map(|f| f.1.clone());
                let footprint = set("Footprint").or_else(|| library::field(&text, "Footprint")).filter(|f| !f.is_empty());
                let package = part.param("Package / Case").unwrap_or(tr("none given"));
                let symbol_value = format!("{}{}", symbol.id(), set("Value").map_or(String::new(), |v| format!(" {v}")));
                let what = match footprint {
                    Some(f) => trf("{0} ({1}) as {2} on {3}", &[&part.mpn, &part.maker, &symbol_value, &f]),
                    None => trf("{0} ({1}) as {2}, no footprint for its package ({3})", &[&part.mpn, &part.maker, &symbol_value, &package]),
                };
                self.place_symbol(&symbol, text, fields, what);
            }
            Err(e) => self.lookup = Some(Lookup::Failed(mpn, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_a_lookup_for_a_part_number_and_says_how_it_goes() {
        assert_eq!(row("resistor", &None, true), None);
        assert_eq!(row("LM358 DR", &None, true), None);
        assert_eq!(row("glow:TMP117", &None, true), None);
        assert_eq!(row("LM358DR", &None, true).map(|r| r.0), Some(true));
        let says = |lookup| row(" LM358DR ", &lookup, false).map(|r| r.1);
        assert_eq!(says(None).as_deref(), Some("LM358DR at Digi-Key: its symbol, footprint, datasheet and maker"));
        assert_eq!(says(Some(Lookup::Failed("LM358DR".into(), "no key".into()))).as_deref(), Some("no key"));
        // Another part number's lookup says nothing of this one.
        assert_eq!(says(Some(Lookup::Failed("X1".into(), "no key".into()))).unwrap(), says(None).unwrap());
    }
}
