//! Shared UTF-16 endianness detection and lossy decoding helpers for xtask policy gates.

/// Detect BOM-less UTF-16 text, handling both ASCII-rich and non-ASCII-heavy UTF-16 files.
///
/// Returns `Some(true)` for little-endian, `Some(false)` for big-endian, or `None` when the
/// byte slice does not clear minimum length, UTF-16 structural cleanliness, and dominance rules.
pub(crate) fn detect_bomless_utf16(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 8 || !bytes.contains(&0) {
        return None;
    }
    let mut le_ascii = 0usize;
    let mut be_ascii = 0usize;
    let mut le_nul_hi = 0usize;
    let mut be_nul_hi = 0usize;
    let (pairs, remainder) = bytes.as_chunks::<2>();
    let total_pairs = pairs.len();
    for &[a, b] in pairs {
        if a != 0 && (a.is_ascii_graphic() || a.is_ascii_whitespace()) && b == 0 {
            le_ascii += 1;
        } else if a == 0 && b != 0 && (b.is_ascii_graphic() || b.is_ascii_whitespace()) {
            be_ascii += 1;
        } else if a >= 0x80 && b == 0 {
            le_nul_hi += 1;
        } else if a == 0 && b >= 0x80 {
            be_nul_hi += 1;
        }
    }
    let le_strong = le_ascii >= 4 && le_ascii * 4 >= total_pairs && le_ascii > be_ascii.saturating_mul(2);
    let be_strong = be_ascii >= 4 && be_ascii * 4 >= total_pairs && be_ascii > le_ascii.saturating_mul(2);
    if le_strong {
        return Some(true);
    }
    if be_strong {
        return Some(false);
    }
    if !remainder.is_empty() {
        return None;
    }
    let le_sig = le_ascii + le_nul_hi;
    let be_sig = be_ascii + be_nul_hi;
    let le_non_ascii = is_clean_utf16(pairs, true) && le_sig >= 3 && le_ascii >= 2 && le_sig > be_sig.saturating_mul(2);
    let be_non_ascii = is_clean_utf16(pairs, false) && be_sig >= 3 && be_ascii >= 2 && be_sig > le_sig.saturating_mul(2);
    if le_non_ascii && !be_non_ascii {
        Some(true)
    } else if be_non_ascii && !le_non_ascii {
        Some(false)
    } else {
        None
    }
}

fn is_clean_utf16(pairs: &[[u8; 2]], little_endian: bool) -> bool {
    let units = pairs.iter().map(|&arr| if little_endian { u16::from_le_bytes(arr) } else { u16::from_be_bytes(arr) });
    for decoded in char::decode_utf16(units) {
        let Ok(ch) = decoded else {
            return false;
        };
        let code = ch as u32;
        if ch == '\0'
            || (code < 0x20 && !matches!(ch, '\n' | '\r' | '\t'))
            || (0x007F..=0x009F).contains(&code)
            || (0xFDD0..=0xFDEF).contains(&code)
            || (code & 0xFFFE) == 0xFFFE
        {
            return false;
        }
    }
    true
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

/// Decode text bytes preferring UTF BOMs and BOM-less UTF-16 over UTF-8, returning `None` if the
/// payload is binary/corrupt (invalid UTF-8 or unexpected NUL bytes outside recognized UTF-16).
pub(crate) fn decode_utf8_or_utf16(bytes: &[u8]) -> Result<Option<String>, String> {
    if let Some(bomless) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let s = std::str::from_utf8(bomless).map_err(|e| format!("invalid UTF-8 after BOM: {e}"))?;
        if s.as_bytes().contains(&0) {
            return Ok(None);
        }
        return Ok(Some(s.to_owned()));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Ok(Some(decode_utf16(rest, true)));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Ok(Some(decode_utf16(rest, false)));
    }
    if let Some(little_endian) = detect_bomless_utf16(bytes) {
        return Ok(Some(decode_utf16(bytes, little_endian)));
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        if s.as_bytes().contains(&0) {
            return Ok(None);
        }
        return Ok(Some(s.to_owned()));
    }
    Ok(None)
}

/// Decode common XML/HTML named and numeric character entities (`&#58;`, `&#x2f;`, `&colon;`, etc.)
/// so obfuscated URLs, attributes, and brand strings in SVG/XML/HTML are checked canonically.
pub(crate) fn decode_xml_html_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_owned();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some((before, after_amp)) = rest.split_once('&') {
        out.push_str(before);
        if let Some((body, tail)) = after_amp.split_once(';')
            && !body.is_empty()
            && body.len() <= 12
            && let Some(decoded) = decode_entity_body(body)
        {
            out.push(decoded);
            rest = tail;
        } else {
            out.push('&');
            rest = after_amp;
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity_body(body: &str) -> Option<char> {
    if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
        let code = u32::from_str_radix(hex, 16).ok()?;
        return char::from_u32(code).filter(|&c| c != '\0');
    }
    if let Some(dec) = body.strip_prefix('#') {
        let code = dec.parse::<u32>().ok()?;
        return char::from_u32(code).filter(|&c| c != '\0');
    }
    match body {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "colon" => Some(':'),
        "sol" => Some('/'),
        "period" => Some('.'),
        "quest" => Some('?'),
        "equals" => Some('='),
        "nbsp" => Some(' '),
        "hellip" => Some('…'),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_utf8_or_utf16, decode_utf16, detect_bomless_utf16};

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
    fn detects_non_ascii_heavy_bomless_utf16_le_and_be() {
        let mut cjk_text = "\u{4E2D}\u{6587}\u{6D4B}\u{8BD5}".repeat(40);
        cjk_text.push_str("Alpha");
        let le: Vec<u8> = cjk_text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = cjk_text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(detect_bomless_utf16(&le), Some(true));
        assert_eq!(detect_bomless_utf16(&be), Some(false));
        assert_eq!(decode_utf8_or_utf16(&le).unwrap().as_deref(), Some(cjk_text.as_str()));
        assert_eq!(decode_utf8_or_utf16(&be).unwrap().as_deref(), Some(cjk_text.as_str()));

        // ASCII UTF-16LE (which is also valid UTF-8 with NULs) must still decode as UTF-16 before UTF-8.
        let ascii_le: Vec<u8> = "CurveBook".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode_utf8_or_utf16(&ascii_le).unwrap().as_deref(), Some("CurveBook"));
        // Stray NUL in UTF-8 must not silently pass as UTF-8 text.
        assert_eq!(decode_utf8_or_utf16(b"Curve\0Book").unwrap(), None);
        assert_eq!(super::decode_xml_html_entities("https&#58;&#x2f;&#x2f;evil.example&sol;t"), "https://evil.example/t");
    }

    #[test]
    fn decode_utf16_replaces_unpaired_surrogates_and_odd_tail_bytes() {
        // High surrogate 0xD800 without low surrogate, followed by 'A' and a trailing odd byte.
        let raw = [0x00, 0xD8, b'A', 0x00, 0x7F];
        assert_eq!(decode_utf16(&raw, true), "\u{FFFD}A\u{FFFD}");
    }
}
