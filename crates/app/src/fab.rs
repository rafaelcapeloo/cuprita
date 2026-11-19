//! Fab files (Ctrl+P): with the zones filled first, `<board>-gerbers.zip` (Gerber X2 layers and
//! Excellon drills, named as KiCad names them), `<board>-bom.csv` and `<board>-positions.csv`
//! (JLCPCB's columns), which JLCPCB, PCBWay and OSH Park take as they are. Written next to the
//! board; in the browser, downloaded. And the schematic's PDF (Ctrl+Shift+P), beside the project.

use crate::App;
use crate::tr::{t, trf};
use cuprita_core::board::Board;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
use web_time::Instant; // std's panics in the browser

impl App {
    /// Fills the zones (an undo step when a fill changed), then writes the fab files.
    pub(crate) fn fab(&mut self) {
        self.fill();
        let t = Instant::now();
        let stem = self.name.strip_suffix(".kicad_pcb").unwrap_or(&self.name).to_string();
        let files = fab_files(&self.board, &stem);
        // How many files were there before, kept as backups.
        #[cfg(target_arch = "wasm32")]
        let kept = crate::web::download(&files.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect::<Vec<_>>()).map(|()| 0);
        #[cfg(not(target_arch = "wasm32"))]
        let kept = self
            .path
            .as_deref()
            .ok_or_else(|| crate::tr::tr("no board file").to_string())
            .and_then(|p| write_files(p, &files))
            .map(|(_, kept)| kept.len());
        self.status = match kept {
            Ok(kept) => {
                let made = if kept == 0 {
                    t!("Made {0}-gerbers.zip, -bom.csv and -positions.csv in {1}, zones filled first")
                } else {
                    t!(
                        "Made {0}-gerbers.zip, -bom.csv and -positions.csv in {1}, zones filled first; the previous ones are kept as <file>-bak"
                    )
                };
                trf(made, &[&stem, &format!("{:.0?}", t.elapsed())])
            }
            Err(e) => trf("Fab files not made: {0}", &[&e]),
        };
    }

    /// Writes the project's schematic as `<root>.pdf` beside it (the one there kept as `-bak`),
    /// and opens it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn pdf(&mut self, ctx: &eframe::egui::Context) {
        if self.sch.is_none() {
            self.board_sch();
        }
        let Some(sch) = &self.sch else {
            return self.status = crate::tr::tr("This project has no schematic to write: Ctrl+E shows why").into();
        };
        let t = Instant::now();
        let (root, project) = (sch.sheets.root.clone(), &sch.sheets.project);
        let title = root.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let (pdf, path) = (project.pdf(&title), root.with_extension("pdf"));
        let sheets = crate::count(project.pages.len(), crate::tr::n!("sheet"));
        let said = match crate::file::save_file(&path, &pdf) {
            Ok(bak) => {
                self.open_url(ctx, &path.display().to_string());
                let wrote = if bak.is_some() {
                    t!("Wrote {0}, {1}, in {2}; the previous one is kept as {0}-bak")
                } else {
                    t!("Wrote {0}, {1}, in {2}")
                };
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                trf(wrote, &[&name, &sheets, &format!("{:.0?}", t.elapsed())])
            }
            Err(e) => trf("The schematic's PDF wasn't written: {0}", &[&e]),
        };
        match (self.sch_view(), &mut self.sch) {
            (true, Some(sch)) => sch.status = said,
            _ => self.status = said,
        }
    }
}

/// `board`'s fab files named after `stem`, as (name, contents): its Gerbers and drills zipped,
/// its BOM and its pick-and-place positions.
pub(crate) fn fab_files(board: &Board, stem: &str) -> [(String, Vec<u8>); 3] {
    [
        (format!("{stem}-gerbers.zip"), zip(&board.fab_files(stem))),
        (format!("{stem}-bom.csv"), board.bom_csv().into_bytes()),
        (format!("{stem}-positions.csv"), board.positions_csv().into_bytes()),
    ]
}

/// Writes `files` next to the board at `board`, each one there before kept as `<file>-bak`;
/// returns the files written and the copies kept.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_files(board: &Path, files: &[(String, Vec<u8>)]) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let (mut written, mut kept) = (vec![], vec![]);
    for (name, data) in files {
        let path = board.with_file_name(name);
        kept.extend(crate::file::save_file(&path, data)?);
        written.push(path);
    }
    Ok((written, kept))
}

/// A zip archive of `files` (name, text), deflated, as fabs take Gerbers.
fn zip(files: &[(String, String)]) -> Vec<u8> {
    use flate2::{Compression, write::DeflateEncoder};
    use std::io::Write;
    let (mut out, mut dir) = (vec![], vec![]);
    for (name, text) in files {
        let mut z = DeflateEncoder::new(vec![], Compression::fast());
        let data = z.write_all(text.as_bytes()).and_then(|()| z.finish()).expect("deflating into memory");
        // Version 2.0, UTF-8 names, deflated, 1980-01-01 00:00, then CRC-32 and sizes.
        let mut head = [&20u16.to_le_bytes()[..], &0x800u16.to_le_bytes(), &8u16.to_le_bytes(), &[0, 0, 0x21, 0]].concat();
        for v in [crc32fast::hash(text.as_bytes()), data.len() as u32, text.len() as u32] {
            head.extend(v.to_le_bytes());
        }
        head.extend((name.len() as u16).to_le_bytes());
        head.extend([0, 0]);
        dir.extend(
            [&0x02014b50u32.to_le_bytes()[..], &20u16.to_le_bytes(), &head, &[0; 10], &(out.len() as u32).to_le_bytes(), name.as_bytes()]
                .concat(),
        );
        out.extend([&0x04034b50u32.to_le_bytes()[..], &head, name.as_bytes(), &data].concat());
    }
    let (n, at) = ((files.len() as u16).to_le_bytes(), (out.len() as u32).to_le_bytes());
    let end = [&0x06054b50u32.to_le_bytes()[..], &[0; 4], &n, &n, &(dir.len() as u32).to_le_bytes(), &at, &[0, 0]].concat();
    [out, dir, end].concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn zips_files_any_unzip_reads() {
        let files = [("a-F_Cu.gtl".to_string(), "G04 x*\n".repeat(50)), ("ü.drl".to_string(), String::new())];
        let z = zip(&files);
        let u32_at = |i: usize| u32::from_le_bytes(z[i..i + 4].try_into().unwrap());
        let u16_at = |i: usize| u16::from_le_bytes(z[i..i + 2].try_into().unwrap()) as usize;
        // The end record finds the directory, whose entries find each file's header and data.
        let end = z.len() - 22;
        assert_eq!((u32_at(end), u16_at(end + 10)), (0x06054b50, 2));
        let mut entry = u32_at(end + 16) as usize;
        for (name, text) in &files {
            let (head, len, n) = (u32_at(entry + 42) as usize, u32_at(entry + 20) as usize, u16_at(entry + 28));
            assert_eq!(
                (u32_at(entry), &z[entry + 46..entry + 46 + n], u32_at(entry + 16)),
                (0x02014b50, name.as_bytes(), crc32fast::hash(text.as_bytes()))
            );
            assert_eq!((u32_at(head), u16_at(head + 26)), (0x04034b50, n));
            let data = &z[head + 30 + n..head + 30 + n + len];
            let mut back = String::new();
            flate2::read::DeflateDecoder::new(data).read_to_string(&mut back).unwrap();
            assert_eq!(&back, text);
            entry += 46 + n;
        }
    }
}
