//! Shared UTF-16/UTF-32 decoding and XML/HTML entity helpers for `xtask` policy gates.

/// Detect BOM-less UTF-16 text (`Some(true)` = LE, `Some(false)` = BE, `None` = not UTF-16).
pub(crate) fn detect_bomless_utf16(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 8 || !bytes.contains(&0) {
        return None;
    }
    let (pairs, rem) = bytes.as_chunks::<2>();
    if pairs.contains(&[0, 0]) {
        return None;
    }
    let (mut le_a, mut be_a, mut le_h, mut be_h) = (0usize, 0usize, 0usize, 0usize);
    for &[a, b] in pairs {
        if b == 0 && (a.is_ascii_graphic() || a.is_ascii_whitespace()) {
            le_a += 1;
        } else if a == 0 && (b.is_ascii_graphic() || b.is_ascii_whitespace()) {
            be_a += 1;
        } else if b == 0 && a >= 0x80 {
            le_h += 1;
        } else if a == 0 && b >= 0x80 {
            be_h += 1;
        }
    }
    let n = pairs.len();
    if le_a >= 4 && le_a * 4 >= n && le_a > be_a.saturating_mul(2) {
        return Some(true);
    }
    if be_a >= 4 && be_a * 4 >= n && be_a > le_a.saturating_mul(2) {
        return Some(false);
    }
    if !rem.is_empty() {
        return None;
    }
    let (le_sig, be_sig) = (le_a + le_h, be_a + be_h);
    let le_ok = is_clean_utf16(pairs, true) && le_sig >= 3 && le_a >= 2 && le_sig > be_sig.saturating_mul(2);
    let be_ok = is_clean_utf16(pairs, false) && be_sig >= 3 && be_a >= 2 && be_sig > le_sig.saturating_mul(2);
    match (le_ok, be_ok) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

fn is_clean_utf16(pairs: &[[u8; 2]], le: bool) -> bool {
    let units = pairs.iter().map(|&a| if le { u16::from_le_bytes(a) } else { u16::from_be_bytes(a) });
    char::decode_utf16(units).all(|r| {
        r.is_ok_and(|ch| {
            let c = ch as u32;
            ch != '\0'
                && (c >= 0x20 || matches!(ch, '\n' | '\r' | '\t'))
                && !(0x7F..=0x9F).contains(&c)
                && !(0xFDD0..=0xFDEF).contains(&c)
                && (c & 0xFFFE) != 0xFFFE
        })
    })
}

/// Lossily decode a UTF-16 byte stream, replacing unpaired surrogates or an odd tail byte with `U+FFFD`.
pub(crate) fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let (pairs, rem) = bytes.as_chunks::<2>();
    let units = pairs.iter().map(|&a| if little_endian { u16::from_le_bytes(a) } else { u16::from_be_bytes(a) });
    let mut out: String = char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect();
    if !rem.is_empty() {
        out.push('\u{FFFD}');
    }
    out
}

fn decode_utf32_strict(bytes: &[u8], le: bool) -> Option<String> {
    (!bytes.is_empty() && bytes.len().is_multiple_of(4))
        .then(|| {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&a| char::from_u32(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) }).filter(|&c| c != '\0'))
                .collect()
        })
        .flatten()
}

/// Decode text bytes preferring UTF BOMs and BOM-less UTF-16 over UTF-8, returning `Ok(None)` for
/// binary/corrupt inputs. Any `Ok(Some(text))` is guaranteed to contain zero `'\0'` characters.
pub(crate) fn decode_utf8_or_utf16(bytes: &[u8]) -> Result<Option<String>, String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let s = std::str::from_utf8(rest).map_err(|e| format!("invalid UTF-8 after BOM: {e}"))?;
        return Ok((!s.as_bytes().contains(&0)).then(|| s.to_owned()));
    }
    if let Some((rest, le)) =
        bytes.strip_prefix(&[0xFF, 0xFE, 0x00, 0x00]).map(|r| (r, true)).or_else(|| bytes.strip_prefix(&[0x00, 0x00, 0xFE, 0xFF]).map(|r| (r, false)))
    {
        return Ok(decode_utf32_strict(rest, le));
    }
    if let Some((rest, le)) = bytes
        .strip_prefix(&[0xFF, 0xFE])
        .map(|r| (r, true))
        .or_else(|| bytes.strip_prefix(&[0xFE, 0xFF]).map(|r| (r, false)))
        .or_else(|| detect_bomless_utf16(bytes).map(|le| (bytes, le)))
    {
        let decoded = decode_utf16(rest, le);
        return Ok((!decoded.contains('\0')).then_some(decoded));
    }
    Ok(std::str::from_utf8(bytes).ok().filter(|s| !s.as_bytes().contains(&0)).map(str::to_owned))
}

/// Decode common XML/HTML named and numeric character entities (`&#58;`, `&#x2f;`, `&colon;`, etc.).
pub(crate) fn decode_xml_html_entities(input: &str) -> String {
    decode_entities(input, false)
}

/// Decode HTML5 attribute entities (also accepting semicolon-less numeric refs, mapping invalid codepoints to U+FFFD).
pub(crate) fn unescape_html_attr(input: &str) -> String {
    decode_entities(input, true)
}

fn decode_entities(input: &str, html5_attr: bool) -> String {
    if !input.contains('&') {
        return input.to_owned();
    }
    let (bytes, mut out, mut i) = (input.as_bytes(), String::with_capacity(input.len()), 0usize);
    while i < bytes.len() {
        if bytes[i] == b'&'
            && let Some((ch, next_i)) = try_decode_entity_at(input, bytes, i, html5_attr)
        {
            out.push(ch);
            i = next_i;
            continue;
        }
        let ch = input.get(i..).and_then(|s| s.chars().next()).unwrap_or('\u{FFFD}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn try_decode_entity_at(input: &str, bytes: &[u8], amp: usize, html5_attr: bool) -> Option<(char, usize)> {
    let start = amp + 1;
    if *bytes.get(start)? == b'#' {
        let (hex, mut j) = if matches!(bytes.get(start + 1), Some(b'x' | b'X')) { (true, start + 2) } else { (false, start + 1) };
        let digits_start = j;
        while bytes.get(j) == Some(&b'0') {
            j += 1;
        }
        let sig_start = j;
        while bytes.get(j).is_some_and(|&b| if hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() }) {
            j += 1;
        }
        let has_semi = bytes.get(j) == Some(&b';');
        if j == digits_start || (!html5_attr && !has_semi) {
            return None;
        }
        let sig = input.get(sig_start..j)?;
        let next_i = j + usize::from(has_semi);
        let ch = if sig.is_empty() || sig.len() > if hex { 6 } else { 7 } {
            if html5_attr { '\u{FFFD}' } else { return None }
        } else {
            let code = u32::from_str_radix(sig, if hex { 16 } else { 10 }).ok()?;
            match char::from_u32(code).filter(|&c| c != '\0') {
                Some(c) => c,
                None if html5_attr => '\u{FFFD}',
                None => return None,
            }
        };
        return Some((ch, next_i));
    }
    let mut j = start;
    while j < bytes.len() && j - start <= 16 && bytes[j].is_ascii_alphanumeric() {
        j += 1;
    }
    if j == start || bytes.get(j) != Some(&b';') {
        return None;
    }
    let ch = match input.get(start..j)?.to_ascii_lowercase().as_str() {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "colon" => ':',
        "sol" => '/',
        "semi" => ';',
        "period" => '.',
        "quest" => '?',
        "equals" => '=',
        "nbsp" => ' ',
        "tab" => '\t',
        "newline" => '\n',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        _ => return None,
    };
    Some((ch, j + 1))
}

#[cfg(test)]
mod tests {
    use super::{decode_utf8_or_utf16, decode_utf16, decode_xml_html_entities, detect_bomless_utf16, unescape_html_attr};

    #[test]
    fn decodes_utf16_utf32_and_entities_and_rejects_embedded_nuls() {
        let le: Vec<u8> = "CurveBook".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = "CurveBook".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!((detect_bomless_utf16(&le), detect_bomless_utf16(&be)), (Some(true), Some(false)));
        assert_eq!((decode_utf16(&le, true).as_str(), decode_utf16(&be, false).as_str()), ("CurveBook", "CurveBook"));
        assert_eq!((detect_bomless_utf16(b"A\0B\0C\0"), detect_bomless_utf16(b"CurveBook")), (None, None));
        assert_eq!(decode_utf8_or_utf16(&le).unwrap().as_deref(), Some("CurveBook"));
        assert_eq!(decode_utf8_or_utf16(b"Curve\0Book").unwrap(), None);
        assert_eq!(decode_utf16(&[0x00, 0xD8, b'A', 0x00, 0x7F], true), "\u{FFFD}A\u{FFFD}");

        let cjk = format!("{}Alpha", "\u{4E2D}\u{6587}\u{6D4B}\u{8BD5}".repeat(20));
        let cjk_le: Vec<u8> = cjk.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode_utf8_or_utf16(&cjk_le).unwrap().as_deref(), Some(cjk.as_str()));

        let mut u32le = vec![0xFF, 0xFE, 0x00, 0x00];
        u32le.extend("CurveBook".chars().flat_map(|c| (c as u32).to_le_bytes()));
        assert_eq!(decode_utf8_or_utf16(&u32le).unwrap().as_deref(), Some("CurveBook"));
        assert_eq!(decode_utf8_or_utf16(&[0xFF, 0xFE, b'A', 0x00, 0x00, 0x00]).unwrap(), None);

        assert_eq!(decode_xml_html_entities("https&#000058;&#x2f;&sol;e.test &copy;&reg;&trade;"), "https://e.test ©®™");
        assert_eq!(decode_xml_html_entities("&#0000;&#x110000;"), "&#0000;&#x110000;");
        assert_eq!(unescape_html_attr("&#x6a;ava&#115cript&#0;"), "javascript\u{FFFD}");
    }
}
