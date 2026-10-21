//! Hex formatting and parsing for logs, dry runs and the `raw` escape hatch.

use std::fmt::Write as _;

/// Space-separated upper-case hex, e.g. `07 82 01`.
pub fn to_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 3);
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{b:02X}");
    }
    s
}

/// Compact dump: 16 bytes per line with offsets, only up to the last non-zero byte, followed by
/// `(zeros to N)` when trailing zeros were cut.
///
/// ```text
/// 0000  07 82 01 00 00 00 00 00 53 5F 5C 59 62 B4 60 5D
/// 0010  5A BA
///       (zeros to 264)
/// ```
pub fn dump(data: &[u8]) -> String {
    let used = data.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let mut s = String::new();
    for (line, chunk) in data[..used].chunks(16).enumerate() {
        let _ = writeln!(s, "{:04X}  {}", line * 16, to_hex(chunk));
    }
    if used < data.len() {
        let _ = writeln!(s, "      (zeros to {})", data.len());
    }
    s
}

/// Parses hex bytes: `07 82 01`, `078201`, `0x07,0x82` and mixtures. Separators are spaces,
/// commas, colons and dashes.
pub fn parse(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for token in s.split(|c: char| c.is_whitespace() || matches!(c, ',' | ':' | '-')) {
        let t = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
            .unwrap_or(token);
        if t.is_empty() {
            continue;
        }
        if t.len() % 2 != 0 || !t.is_ascii() {
            return Err(format!("odd or invalid hex group {token:?}"));
        }
        for i in (0..t.len()).step_by(2) {
            out.push(
                u8::from_str_radix(&t[i..i + 2], 16)
                    .map_err(|_| format!("invalid hex {token:?}"))?,
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_trims_trailing_zeros() {
        let mut buf = vec![0u8; 264];
        buf[0] = 0x07;
        buf[1] = 0x82;
        buf[17] = 0xAB;
        assert_eq!(
            dump(&buf),
            "0000  07 82 00 00 00 00 00 00 00 00 00 00 00 00 00 00\n0010  00 AB\n      (zeros to 264)\n"
        );
        assert_eq!(dump(&[0, 0]), "      (zeros to 2)\n");
        assert_eq!(dump(&[1, 2]), "0000  01 02\n");
    }

    #[test]
    fn parse_variants() {
        assert_eq!(parse("07 82 01"), Ok(vec![7, 0x82, 1]));
        assert_eq!(parse("078201"), Ok(vec![7, 0x82, 1]));
        assert_eq!(parse("0x07,0x82, 0X01"), Ok(vec![7, 0x82, 1]));
        assert_eq!(parse(""), Ok(vec![]));
        assert!(parse("7").is_err());
        assert!(parse("zz").is_err());
        assert_eq!(to_hex(&[0xDE, 0xAD]), "DE AD");
    }
}
