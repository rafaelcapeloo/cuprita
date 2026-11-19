//! Exact conversion between KiCad's decimal millimetres and integer nanometres.

/// Parses `"-12.345678"` (mm) into nanometres without going through floats.
/// Extra decimals are rounded; exponents fall back to `f64`. `None` if invalid or out of `i32`.
pub fn mm_to_nm(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    let (neg, digits) = match b.first()? {
        b'-' => (true, &b[1..]),
        b'+' => (false, &b[1..]),
        _ => (false, b),
    };
    let mut int: i64 = 0;
    let mut frac: i64 = 0;
    let mut frac_len = 0;
    let mut round_up = false;
    let mut seen_dot = false;
    let mut any = false;
    for &c in digits {
        match c {
            b'0'..=b'9' if !seen_dot => {
                int = int.checked_mul(10)?.checked_add((c - b'0') as i64)?;
                any = true;
            }
            b'0'..=b'9' => {
                if frac_len < 6 {
                    frac = frac * 10 + (c - b'0') as i64;
                } else if frac_len == 6 {
                    round_up = c >= b'5';
                }
                frac_len += 1;
                any = true;
            }
            b'.' if !seen_dot => seen_dot = true,
            b'e' | b'E' => {
                let v = s.parse::<f64>().ok()? * 1e6;
                return (v.abs() <= i32::MAX as f64).then(|| v.round() as i32);
            }
            _ => return None,
        }
    }
    if !any {
        return None;
    }
    for _ in frac_len..6 {
        frac *= 10;
    }
    let mag = int.checked_mul(1_000_000)?.checked_add(frac + round_up as i64)?;
    i32::try_from(if neg { -mag } else { mag }).ok()
}

/// Writes nanometres as KiCad writes millimetres: exact, without trailing zeros (`-1.5`, `85`).
pub fn nm_to_mm(nm: i32) -> String {
    let (int, frac) = (nm / 1_000_000, (nm % 1_000_000).unsigned_abs());
    // The integer part of -0.5 is 0, which has no sign of its own.
    let sign = if nm < 0 && int == 0 { "-" } else { "" };
    format!("{sign}{int}.{frac:06}").trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::{mm_to_nm, nm_to_mm};

    #[test]
    fn parses_exactly() {
        assert_eq!(mm_to_nm("0"), Some(0));
        assert_eq!(mm_to_nm("1.5"), Some(1_500_000));
        assert_eq!(mm_to_nm("-0.000001"), Some(-1));
        assert_eq!(mm_to_nm("204.800001"), Some(204_800_001));
        assert_eq!(mm_to_nm("-.5"), Some(-500_000));
        assert_eq!(mm_to_nm("5."), Some(5_000_000));
        assert_eq!(mm_to_nm("1e-3"), Some(1_000));
    }

    #[test]
    fn rounds_extra_decimals() {
        assert_eq!(mm_to_nm("0.0000014"), Some(1));
        assert_eq!(mm_to_nm("0.0000015"), Some(2));
    }

    #[test]
    fn writes_exactly_and_reads_back() {
        let nms = [0, 1, -1, 85_000_000, -1_500_000, -500_000, 204_800_001, i32::MIN, i32::MAX];
        let written = nms.map(nm_to_mm);
        assert_eq!(written[..7], ["0", "0.000001", "-0.000001", "85", "-1.5", "-0.5", "204.800001"]);
        assert_eq!(written.map(|s| mm_to_nm(&s)), nms.map(Some));
    }

    #[test]
    fn rejects_garbage_and_overflow() {
        assert_eq!(mm_to_nm("2147.483647"), Some(i32::MAX));
        assert_eq!(mm_to_nm("2147.483648"), None);
        for bad in ["", "-", ".", "abc", "1.2.3", "1x"] {
            assert_eq!(mm_to_nm(bad), None, "{bad}");
        }
    }
}
