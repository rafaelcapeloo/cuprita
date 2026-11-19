//! Zip archives read, as parts' vendors pack their files: each file's name and bytes, stored or
//! deflated (what every zip tool writes), its central directory trusted for where each is.

use crate::tr::{tr, trf};
use flate2::read::DeflateDecoder;
use std::io::Read;

/// The most an archive's files may hold together, unpacked: a part's files are a few megabytes,
/// and a bigger claim is a broken or hostile archive.
const MOST: u64 = 1 << 30;

/// The files of zip archive `data`, by their names in it (folders as `a/b.txt`), folders left out.
// shortcut: no Zip64 (archives over 4 GB or 65,535 files) or encryption; parts' archives use neither.
pub fn read(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let bad = |what: &str| trf("not a zip archive Cuprita can read: {0}", &[&what]);
    let u16_at = |at: usize| data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |at: usize| data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    // The end of the central directory: last in the file, before a comment of at most 64 KiB.
    let end = (0..data.len().saturating_sub(21)).rev().take(65_536 + 22).find(|&at| u32_at(at) == Some(0x0605_4b50));
    let end = end.ok_or_else(|| bad(tr("its end record is missing")))?;
    let (count, mut at) = (u16_at(end + 10).unwrap_or(0), u32_at(end + 16).unwrap_or(usize::MAX));
    let (mut files, mut total) = (vec![], 0u64);
    for _ in 0..count {
        if u32_at(at) != Some(0x0201_4b50) {
            return Err(bad(tr("its directory is broken")));
        }
        let field = |off: usize| u16_at(at + off).ok_or_else(|| bad(tr("its directory is cut short")));
        let (flags, method, size) = (field(8)?, field(10)?, u32_at(at + 24).unwrap_or(0) as u64);
        let (packed, name_len) = (u32_at(at + 20).unwrap_or(0), field(28)?);
        let local = u32_at(at + 42).ok_or_else(|| bad(tr("its directory is cut short")))?;
        let name = data.get(at + 46..at + 46 + name_len).ok_or_else(|| bad(tr("a name is cut short")))?;
        let name = String::from_utf8_lossy(name).replace('\\', "/");
        at += 46 + name_len + field(30)? + field(32)?;
        if name.ends_with('/') {
            continue;
        }
        if flags & 1 != 0 {
            return Err(trf("{0} in the zip is encrypted: unpack it with its password, then add its files", &[&name]));
        }
        // A stored file is as long as it says: else thousands could each name the whole archive.
        total += if method == 0 { packed.max(size as usize) as u64 } else { size };
        if total > MOST {
            return Err(bad(tr("it unpacks to more than a gigabyte")));
        }
        let start = local + 30 + u16_at(local + 26).unwrap_or(0) + u16_at(local + 28).unwrap_or(0);
        let raw = data.get(start..start + packed).filter(|_| u32_at(local) == Some(0x0403_4b50));
        let raw = raw.ok_or_else(|| trf("{0} in the zip is cut short: download it again", &[&name]))?;
        let bytes = match method {
            0 => raw.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(size as usize);
                DeflateDecoder::new(raw).take(size).read_to_end(&mut out).map_err(|e| trf("{0} in the zip: {1}", &[&name, &e]))?;
                out
            }
            m => {
                return Err(trf(
                    "{0} in the zip is packed in a way Cuprita doesn't read (method {1}): unpack it, then add its files",
                    &[&name, &m],
                ));
            }
        };
        files.push((name, bytes));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::DeflateEncoder};
    use std::io::Write;

    /// A zip of `files`, each deflated when `deflate`, as zip tools write one.
    fn zip(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
        let (mut out, mut dir) = (vec![], vec![]);
        for (name, data) in files {
            let packed = if deflate {
                let mut z = DeflateEncoder::new(vec![], Compression::default());
                z.write_all(data).unwrap();
                z.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if deflate { 8 } else { 0 };
            let head = |sig: u32| {
                let mut h = sig.to_le_bytes().to_vec();
                h.extend([20, 0, 0, 0]);
                h.extend(method.to_le_bytes());
                h.extend([0; 8]); // time, date, CRC (not checked)
                h.extend((packed.len() as u32).to_le_bytes());
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes());
                h
            };
            let local = out.len() as u32;
            out.extend(head(0x0403_4b50));
            out.extend([0, 0]);
            out.extend(name.as_bytes());
            out.extend(&packed);
            let mut d = head(0x0201_4b50);
            d.splice(4..4, [20, 0]); // made by
            d.extend([0; 12]);
            d.extend(local.to_le_bytes());
            d.extend(name.as_bytes());
            dir.push(d);
        }
        let (start, size) = (out.len() as u32, dir.iter().map(Vec::len).sum::<usize>() as u32);
        let n = dir.len() as u16;
        out.extend(dir.concat());
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0; 4]);
        out.extend(n.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend(size.to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend([0, 0]);
        out
    }

    #[test]
    fn reads_stored_and_deflated_files_and_says_why_not() {
        let files: [(&str, &[u8]); 3] = [("LM358/", b""), ("LM358/LM358.kicad_sym", b"(kicad_symbol_lib)"), ("SOIC-8.step", &[7; 300])];
        for deflate in [false, true] {
            let read = read(&zip(&files, deflate)).unwrap();
            assert_eq!(read, [("LM358/LM358.kicad_sym".to_string(), b"(kicad_symbol_lib)".to_vec()), ("SOIC-8.step".into(), vec![7; 300])]);
        }
        assert_eq!(read(b"PK no zip").unwrap_err(), "not a zip archive Cuprita can read: its end record is missing");
        let mut cut = zip(&files, true);
        cut.drain(40..60);
        assert!(read(&cut).is_err());
    }
}
