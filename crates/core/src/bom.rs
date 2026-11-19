//! Bill of materials: fitted parts grouped into lines, for the bench checklist, agents and fabs.

use crate::board::{Board, Footprint};
use crate::fab::csv;
use std::collections::HashMap;
use std::fmt::Write;

impl Board {
    /// Fitted footprints grouped by value, library link and part numbers, as indices into
    /// `footprints`; the lines and the parts in each are in reference order (`C2` before `C10`).
    pub fn bom(&self) -> Vec<Vec<usize>> {
        let fp = &self.footprints;
        let mut order: Vec<usize> = (0..fp.len()).filter(|&i| fp[i].fitted).collect();
        order.sort_by_key(|&i| ref_key(&fp[i].reference));
        let (mut lines, mut line_of) = (Vec::<Vec<usize>>::new(), HashMap::new());
        for i in order {
            let k = *line_of.entry((&fp[i].value, &fp[i].lib_id, fp[i].part_numbers())).or_insert_with(|| {
                lines.push(Vec::new());
                lines.len() - 1
            });
            lines[k].push(i);
        }
        lines
    }

    /// The BOM as CSV in JLCPCB's columns, which PCBWay reads too: value, references, footprint
    /// name and count, then the manufacturer, part number and LCSC number the parts' fields give.
    pub fn bom_csv(&self) -> String {
        let mut out = String::from("Comment,Designator,Footprint,Quantity,Manufacturer,MPN,LCSC Part #\n");
        for line in self.bom() {
            let f = &self.footprints[line[0]];
            let refs: Vec<&str> = line.iter().map(|&i| self.footprints[i].reference.as_str()).collect();
            let name = f.lib_id.rsplit(':').next().unwrap_or_default();
            let [maker, mpn, lcsc] = f.part_numbers().map(csv);
            let _ = writeln!(out, "{},{},{},{},{maker},{mpn},{lcsc}", csv(&f.value), csv(&refs.join(",")), csv(name), line.len());
        }
        out
    }
}

impl Footprint {
    /// Its manufacturer, part number and LCSC (JLCPCB's parts) number, from its fields by the
    /// names KiCad users give them (any case, spaces and signs aside); empty when it has none.
    pub fn part_numbers(&self) -> [&str; 3] {
        let mut found = [""; 3];
        for (name, value) in &self.fields {
            if let Some(column) = part_number_column(name)
                && found[column].is_empty()
            {
                found[column] = value;
            }
        }
        found
    }
}

/// Which of `part_numbers` a field named `name` gives (0 the manufacturer, 1 its part number,
/// 2 the LCSC number), by the names KiCad users give them: any case, spaces and signs aside.
pub fn part_number_column(name: &str) -> Option<usize> {
    let key: String = name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match key.as_str() {
        "manufacturer" | "mfr" | "mfg" => Some(0),
        "mpn" | "partnumber" | "manufacturerpartnumber" | "mfrpartnumber" | "mfrpn" => Some(1),
        _ if key.starts_with("lcsc") || key.starts_with("jlc") => Some(2),
        _ => None,
    }
}

/// Natural sort key of a reference: prefix, number, then any suffix.
pub(crate) fn ref_key(r: &str) -> (&str, u64, &str) {
    let num = r.find(|c: char| c.is_ascii_digit()).unwrap_or(r.len());
    let end = r[num..].find(|c: char| !c.is_ascii_digit()).map_or(r.len(), |n| num + n);
    (&r[..num], r[num..end].parse().unwrap_or(0), &r[end..])
}

#[cfg(test)]
mod tests {
    use super::part_number_column;
    use crate::board::Board;

    #[test]
    fn groups_fitted_parts_by_value_and_footprint_in_reference_order() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal))
              (footprint "C:C_0402" (property "Reference" "C10") (property "Value" "100n"))
              (footprint "R:R_0603" (property "Reference" "R1") (property "Value" "10k"))
              (footprint "C:C_0402" (property "Reference" "C2") (property "Value" "100n"))
              (footprint "C:C_0603" (property "Reference" "C3") (property "Value" "100n"))
              (footprint "R:R_0603" (property "Reference" "R2") (property "Value" "10k") (attr smd dnp))
              (footprint "H:M3" (property "Reference" "H1") (property "Value" "M3") (attr exclude_from_bom))
              (module TP (fp_text reference TP1) (fp_text value TP) (attr virtual)))"#,
        )
        .unwrap();
        let refs: Vec<Vec<&str>> = b.bom().iter().map(|l| l.iter().map(|&i| b.footprints[i].reference.as_str()).collect()).collect();
        assert_eq!(refs, [vec!["C2", "C10"], vec!["C3"], vec!["R1"]]);
    }

    #[test]
    fn knows_part_number_fields_by_the_names_users_give_them() {
        let names = ["MPN", "Manufacturer Part Number", "Mfr.", "LCSC Part #", "JLCPCB", "Value"];
        assert_eq!(names.map(part_number_column), [Some(1), Some(1), Some(0), Some(2), Some(2), None]);
    }

    #[test]
    fn writes_jlcpcb_columns_one_part_number_a_line() {
        // Part numbers come from the fields KiCad users name so; a different one makes its own line.
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal))
              (footprint "R:R_0603" (property "Reference" "R1") (property "Value" "10k") (property "MPN" "RC0603-10K") (property "LCSC Part #" "C25804"))
              (footprint "R:R_0603" (property "Reference" "R2") (property "Value" "10k") (property "MPN" "RC0603-10K") (property "LCSC Part #" "C25804"))
              (footprint "R:R_0603" (property "Reference" "R3") (property "Value" "10k") (property "Part Number" "ERJ-3EKF1002V")
                (property "Manufacturer" "Panasonic") (property "Alt_MPN1" "x"))
              (footprint "C_0402" (property "Reference" "C1") (property "Value" "1u, 10V") (property "lcsc" "C52923")))"#,
        )
        .unwrap();
        let rows = "Comment,Designator,Footprint,Quantity,Manufacturer,MPN,LCSC Part #\n\
                    \"1u, 10V\",C1,C_0402,1,,,C52923\n\
                    10k,\"R1,R2\",R_0603,2,,RC0603-10K,C25804\n\
                    10k,R3,R_0603,1,Panasonic,ERJ-3EKF1002V,\n";
        assert_eq!(b.bom_csv(), rows);
    }
}
