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
    if pairs.contains(&[0, 0]) {
        return None;
    }
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

/// Strictly decode a UTF-32 byte stream, returning `None` if empty, misaligned, containing any
/// embedded `U+0000` NUL code unit, or containing invalid Unicode scalar values.
fn decode_utf32_strict(bytes: &[u8], little_endian: bool) -> Option<String> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let (quads, _) = bytes.as_chunks::<4>();
    let mut out = String::with_capacity(quads.len());
    for &arr in quads {
        let code = if little_endian { u32::from_le_bytes(arr) } else { u32::from_be_bytes(arr) };
        let ch = char::from_u32(code).filter(|&c| c != '\0')?;
        out.push(ch);
    }
    Some(out)
}

/// Decode text bytes preferring UTF BOMs and BOM-less UTF-16 over UTF-8, returning `None` if the
/// payload is binary/corrupt (invalid UTF-8 or unexpected NUL bytes outside recognized UTF-16/UTF-32).
///
/// Postcondition: any returned `Ok(Some(text))` is guaranteed to contain zero `'\0'` characters.
pub(crate) fn decode_utf8_or_utf16(bytes: &[u8]) -> Result<Option<String>, String> {
    if let Some(bomless) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let s = std::str::from_utf8(bomless).map_err(|e| format!("invalid UTF-8 after BOM: {e}"))?;
        if s.as_bytes().contains(&0) {
            return Ok(None);
        }
        return Ok(Some(s.to_owned()));
    }
    // Check 4-byte UTF-32 BOMs BEFORE 2-byte UTF-16 BOMs (UTF-32LE starts with 0xFF 0xFE 0x00 0x00).
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE, 0x00, 0x00]) {
        return Ok(decode_utf32_strict(rest, true));
    }
    if let Some(rest) = bytes.strip_prefix(&[0x00, 0x00, 0xFE, 0xFF]) {
        return Ok(decode_utf32_strict(rest, false));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let decoded = decode_utf16(rest, true);
        return Ok((!decoded.contains('\0')).then_some(decoded));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let decoded = decode_utf16(rest, false);
        return Ok((!decoded.contains('\0')).then_some(decoded));
    }
    if let Some(little_endian) = detect_bomless_utf16(bytes) {
        let decoded = decode_utf16(bytes, little_endian);
        return Ok((!decoded.contains('\0')).then_some(decoded));
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
///
/// Handles spec-valid numeric character references with arbitrary leading zeros (`&#000000000080;`)
/// in $O(N)$ time without arbitrary total-length caps.
pub(crate) fn decode_xml_html_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_owned();
    }
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'&'
            && let Some((decoded, next_i)) = try_decode_entity_at(input, bytes, i)
        {
            out.push(decoded);
            i = next_i;
            continue;
        }
        let ch = input.get(i..).and_then(|s| s.chars().next()).unwrap_or('\u{FFFD}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn try_decode_entity_at(input: &str, bytes: &[u8], amp: usize) -> Option<(char, usize)> {
    let start = amp + 1;
    let first = *bytes.get(start)?;
    if first == b'#' {
        let (is_hex, mut j) = if matches!(bytes.get(start + 1), Some(b'x' | b'X')) { (true, start + 2) } else { (false, start + 1) };
        let digits_start = j;
        while bytes.get(j) == Some(&b'0') {
            j += 1;
        }
        let sig_start = j;
        while let Some(&b) = bytes.get(j) {
            let ok = if is_hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() };
            if ok {
                j += 1;
            } else {
                break;
            }
        }
        if j == digits_start || bytes.get(j) != Some(&b';') {
            return None;
        }
        let sig = input.get(sig_start..j)?;
        // Reject overflow before parsing; max Unicode scalar is 0x10FFFF (6 hex / 7 dec digits).
        if sig.is_empty() || sig.len() > if is_hex { 6 } else { 7 } {
            return None;
        }
        let radix = if is_hex { 16 } else { 10 };
        let code = u32::from_str_radix(sig, radix).ok()?;
        let ch = char::from_u32(code).filter(|&c| c != '\0')?;
        return Some((ch, j + 1));
    }
    // Named entity: ASCII alphanumeric name up to 16 chars followed by `;`.
    let mut j = start;
    while j < bytes.len() && j - start <= 16 && bytes[j].is_ascii_alphanumeric() {
        j += 1;
    }
    if j == start || bytes.get(j) != Some(&b';') {
        return None;
    }
    let name = input.get(start..j)?;
    let ch = decode_named_entity(name)?;
    Some((ch, j + 1))
}

fn decode_named_entity(body: &str) -> Option<char> {
    match body.to_ascii_lowercase().as_str() {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "colon" => Some(':'),
        "sol" => Some('/'),
        "semi" => Some(';'),
        "period" => Some('.'),
        "quest" => Some('?'),
        "equals" => Some('='),
        "nbsp" => Some(' '),
        "tab" => Some('\t'),
        "newline" => Some('\n'),
        "copy" => Some('©'),
        "reg" => Some('®'),
        "trade" => Some('™'),
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

    #[test]
    fn aud_12_16_utf32_and_nul_utf16_never_emit_embedded_nuls() {
        // UTF-32LE with BOM decodes cleanly without embedded NULs.
        let mut utf32le_bom = vec![0xFF, 0xFE, 0x00, 0x00];
        for ch in "CurveBook".chars() {
            utf32le_bom.extend_from_slice(&(ch as u32).to_le_bytes());
        }
        assert_eq!(decode_utf8_or_utf16(&utf32le_bom).unwrap().as_deref(), Some("CurveBook"));

        // UTF-32BE with BOM decodes cleanly without embedded NULs.
        let mut utf32be_bom = vec![0x00, 0x00, 0xFE, 0xFF];
        for ch in "CurveBook".chars() {
            utf32be_bom.extend_from_slice(&(ch as u32).to_be_bytes());
        }
        assert_eq!(decode_utf8_or_utf16(&utf32be_bom).unwrap().as_deref(), Some("CurveBook"));

        // BOM-less UTF-32LE and UTF-32BE must NOT be misclassified as UTF-16 with embedded NULs.
        let utf32le_bomless: Vec<u8> = "CurveBook".chars().flat_map(|c| (c as u32).to_le_bytes()).collect();
        let utf32be_bomless: Vec<u8> = "CurveBook".chars().flat_map(|c| (c as u32).to_be_bytes()).collect();
        assert_eq!(detect_bomless_utf16(&utf32le_bomless), None);
        assert_eq!(detect_bomless_utf16(&utf32be_bomless), None);
        assert_eq!(decode_utf8_or_utf16(&utf32le_bomless).unwrap(), None);
        assert_eq!(decode_utf8_or_utf16(&utf32be_bomless).unwrap(), None);

        // UTF-16 BOM with embedded NUL code unit must return None.
        let utf16_with_nul = [0xFF, 0xFE, b'A', 0x00, 0x00, 0x00, b'B', 0x00];
        assert_eq!(decode_utf8_or_utf16(&utf16_with_nul).unwrap(), None);
        // UTF-16LE BOM + single NUL (also empty UTF-32LE BOM) must return None.
        assert_eq!(decode_utf8_or_utf16(&[0xFF, 0xFE, 0x00, 0x00]).unwrap(), None);
    }

    #[test]
    fn aud_12_28_decodes_leading_zero_numeric_entities_without_length_cap() {
        assert_eq!(super::decode_xml_html_entities("https&#000000000058;&#000000000047;&#x0000000000002f;evil.example"), "https://evil.example");
        assert_eq!(super::decode_xml_html_entities("Curve&#000000000080;ad &copy;"), "CurvePad ©");
        assert_eq!(super::decode_xml_html_entities("&#x000000000041;&reg;&trade;"), "A®™");
        // NUL entity and out-of-range entity stay literal.
        assert_eq!(super::decode_xml_html_entities("&#000000000000;"), "&#000000000000;");
        assert_eq!(super::decode_xml_html_entities("&#x000000110000;"), "&#x000000110000;");
    }
}
