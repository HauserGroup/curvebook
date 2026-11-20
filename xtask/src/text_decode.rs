//! Shared UTF-16 endianness detection and lossy decoding helpers for xtask policy gates.

/// Detect BOM-less UTF-16 text by counting ASCII graphic/whitespace code units paired with NUL bytes.
///
/// Returns `Some(true)` for little-endian, `Some(false)` for big-endian, or `None` when the
/// byte slice does not clear the minimum length, NUL-pair ratio, and endianness-dominance thresholds.
pub(crate) fn detect_bomless_utf16(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 8 || !bytes.contains(&0) {
        return None;
    }
    let mut le_ascii = 0usize;
    let mut be_ascii = 0usize;
    let (pairs, _) = bytes.as_chunks::<2>();
    let total_pairs = pairs.len();
    for &[a, b] in pairs {
        if a != 0 && (a.is_ascii_graphic() || a.is_ascii_whitespace()) && b == 0 {
            le_ascii += 1;
        } else if a == 0 && b != 0 && (b.is_ascii_graphic() || b.is_ascii_whitespace()) {
            be_ascii += 1;
        }
    }
    if le_ascii >= 4 && le_ascii * 4 >= total_pairs && le_ascii > be_ascii.saturating_mul(2) {
        Some(true)
    } else if be_ascii >= 4 && be_ascii * 4 >= total_pairs && be_ascii > le_ascii.saturating_mul(2) {
        Some(false)
    } else {
        None
    }
}

/// Lossily decode a UTF-16 byte stream in the requested endianness, replacing unpaired surrogates
/// and any trailing odd byte with `U+FFFD`.
pub(crate) fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let (pairs, remainder) = bytes.as_chunks::<2>();
    let units = pairs.iter().map(|&arr| if little_endian { u16::from_le_bytes(arr) } else { u16::from_be_bytes(arr) });
    let mut out = String::with_capacity(pairs.len());
    for ch in char::decode_utf16(units) {
        out.push(ch.unwrap_or('\u{FFFD}'));
    }
    if !remainder.is_empty() {
        out.push('\u{FFFD}');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{decode_utf16, detect_bomless_utf16};

    #[test]
    fn detects_bomless_utf16_le_and_be_and_rejects_short_or_plain_utf8() {
        let le: Vec<u8> = "CurveBook".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = "CurveBook".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(detect_bomless_utf16(&le), Some(true));
        assert_eq!(detect_bomless_utf16(&be), Some(false));
        assert_eq!(decode_utf16(&le, true), "CurveBook");
        assert_eq!(decode_utf16(&be, false), "CurveBook");

        // Too short (< 8 bytes) or lacking NUL bytes.
        assert_eq!(detect_bomless_utf16(b"A\0B\0C\0"), None);
        assert_eq!(detect_bomless_utf16(b"CurveBook"), None);
    }

    #[test]
    fn decode_utf16_replaces_unpaired_surrogates_and_odd_tail_bytes() {
        // High surrogate 0xD800 without low surrogate, followed by 'A' and a trailing odd byte.
        let raw = [0x00, 0xD8, b'A', 0x00, 0x7F];
        assert_eq!(decode_utf16(&raw, true), "\u{FFFD}A\u{FFFD}");
    }
}
