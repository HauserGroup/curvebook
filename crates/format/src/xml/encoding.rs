//! XML character encoding detection and streaming decoding.
//!
//! Supports:
//! - UTF-8 (with or without BOM `EF BB BF`),
//! - UTF-16 LE and UTF-16 BE (with BOM `FF FE` / `FE FF`, or BOM-less autodetection per
//!   W3C XML 1.0 Fifth Edition Appendix F.1),
//! - Single-byte encodings declared via `<?xml ... encoding="..."?>`:
//!   - ISO-8859-1 (direct `0x00..=0xFF` to Unicode `U+0000..=U+00FF` mapping),
//!   - Windows-1252 via static lookup table from Unicode Consortium `CP1252.TXT`
//!     (`MAPPINGS/VENDORS/MICSFT/WINDOWS/CP1252.TXT`, version 2.01),
//!   - MacRoman via static lookup table from Unicode Consortium `ROMAN.TXT`
//!     (`MAPPINGS/VENDORS/APPLE/ROMAN.TXT`, version 2.1).
//!
//! Unknown or unsupported declared encodings (and UTF-32 streams) return a typed
//! [`FormatError::UnsupportedFormat`] rather than guessing.

use std::io::{self, BufRead, Read};

use crate::error::FormatError;

/// Detected or declared XML character encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum XmlEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Iso8859_1,
    Windows1252,
    MacRoman,
}

impl XmlEncoding {
    /// Canonical IANA-style label for diagnostics and reporting.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf16Be => "UTF-16BE",
            Self::Iso8859_1 => "ISO-8859-1",
            Self::Windows1252 => "Windows-1252",
            Self::MacRoman => "MacRoman",
        }
    }

    /// Whether this encoding is 16-bit UTF-16 (LE or BE).
    #[must_use]
    pub const fn is_utf16(self) -> bool {
        matches!(self, Self::Utf16Le | Self::Utf16Be)
    }
}

/// Windows-1252 high-byte mapping (`0x80..=0xFF`) from Unicode Consortium `CP1252.TXT`.
/// Bytes `0x81`, `0x8D`, `0x8F`, `0x90`, and `0x9D` are explicitly `#UNDEFINED` in `CP1252.TXT`.
#[rustfmt::skip]
const CP1252_HIGH: [Option<char>; 128] = [
    // 0x80..=0x87
    Some('\u{20AC}'), None, Some('\u{201A}'), Some('\u{0192}'), Some('\u{201E}'), Some('\u{2026}'), Some('\u{2020}'), Some('\u{2021}'),
    // 0x88..=0x8F
    Some('\u{02C6}'), Some('\u{2030}'), Some('\u{0160}'), Some('\u{2039}'), Some('\u{0152}'), None, Some('\u{017D}'), None,
    // 0x90..=0x97
    None, Some('\u{2018}'), Some('\u{2019}'), Some('\u{201C}'), Some('\u{201D}'), Some('\u{2022}'), Some('\u{2013}'), Some('\u{2014}'),
    // 0x98..=0x9F
    Some('\u{02DC}'), Some('\u{2122}'), Some('\u{0161}'), Some('\u{203A}'), Some('\u{0153}'), None, Some('\u{017E}'), Some('\u{0178}'),
    // 0xA0..=0xA7
    Some('\u{00A0}'), Some('\u{00A1}'), Some('\u{00A2}'), Some('\u{00A3}'), Some('\u{00A4}'), Some('\u{00A5}'), Some('\u{00A6}'), Some('\u{00A7}'),
    // 0xA8..=0xAF
    Some('\u{00A8}'), Some('\u{00A9}'), Some('\u{00AA}'), Some('\u{00AB}'), Some('\u{00AC}'), Some('\u{00AD}'), Some('\u{00AE}'), Some('\u{00AF}'),
    // 0xB0..=0xB7
    Some('\u{00B0}'), Some('\u{00B1}'), Some('\u{00B2}'), Some('\u{00B3}'), Some('\u{00B4}'), Some('\u{00B5}'), Some('\u{00B6}'), Some('\u{00B7}'),
    // 0xB8..=0xBF
    Some('\u{00B8}'), Some('\u{00B9}'), Some('\u{00BA}'), Some('\u{00BB}'), Some('\u{00BC}'), Some('\u{00BD}'), Some('\u{00BE}'), Some('\u{00BF}'),
    // 0xC0..=0xC7
    Some('\u{00C0}'), Some('\u{00C1}'), Some('\u{00C2}'), Some('\u{00C3}'), Some('\u{00C4}'), Some('\u{00C5}'), Some('\u{00C6}'), Some('\u{00C7}'),
    // 0xC8..=0xCF
    Some('\u{00C8}'), Some('\u{00C9}'), Some('\u{00CA}'), Some('\u{00CB}'), Some('\u{00CC}'), Some('\u{00CD}'), Some('\u{00CE}'), Some('\u{00CF}'),
    // 0xD0..=0xD7
    Some('\u{00D0}'), Some('\u{00D1}'), Some('\u{00D2}'), Some('\u{00D3}'), Some('\u{00D4}'), Some('\u{00D5}'), Some('\u{00D6}'), Some('\u{00D7}'),
    // 0xD8..=0xDF
    Some('\u{00D8}'), Some('\u{00D9}'), Some('\u{00DA}'), Some('\u{00DB}'), Some('\u{00DC}'), Some('\u{00DD}'), Some('\u{00DE}'), Some('\u{00DF}'),
    // 0xE0..=0xE7
    Some('\u{00E0}'), Some('\u{00E1}'), Some('\u{00E2}'), Some('\u{00E3}'), Some('\u{00E4}'), Some('\u{00E5}'), Some('\u{00E6}'), Some('\u{00E7}'),
    // 0xE8..=0xEF
    Some('\u{00E8}'), Some('\u{00E9}'), Some('\u{00EA}'), Some('\u{00EB}'), Some('\u{00EC}'), Some('\u{00ED}'), Some('\u{00EE}'), Some('\u{00EF}'),
    // 0xF0..=0xF7
    Some('\u{00F0}'), Some('\u{00F1}'), Some('\u{00F2}'), Some('\u{00F3}'), Some('\u{00F4}'), Some('\u{00F5}'), Some('\u{00F6}'), Some('\u{00F7}'),
    // 0xF8..=0xFF
    Some('\u{00F8}'), Some('\u{00F9}'), Some('\u{00FA}'), Some('\u{00FB}'), Some('\u{00FC}'), Some('\u{00FD}'), Some('\u{00FE}'), Some('\u{00FF}'),
];

/// MacRoman high-byte mapping (`0x80..=0xFF`) from Unicode Consortium `ROMAN.TXT` (v2.1).
#[rustfmt::skip]
const MAC_ROMAN_HIGH: [char; 128] = [
    // 0x80..=0x87
    '\u{00C4}', '\u{00C5}', '\u{00C7}', '\u{00C9}', '\u{00D1}', '\u{00D6}', '\u{00DC}', '\u{00E1}',
    // 0x88..=0x8F
    '\u{00E0}', '\u{00E2}', '\u{00E4}', '\u{00E3}', '\u{00E5}', '\u{00E7}', '\u{00E9}', '\u{00E8}',
    // 0x90..=0x97
    '\u{00EA}', '\u{00EB}', '\u{00ED}', '\u{00EC}', '\u{00EE}', '\u{00EF}', '\u{00F1}', '\u{00F3}',
    // 0x98..=0x9F
    '\u{00F2}', '\u{00F4}', '\u{00F6}', '\u{00F5}', '\u{00FA}', '\u{00F9}', '\u{00FB}', '\u{00FC}',
    // 0xA0..=0xA7
    '\u{2020}', '\u{00B0}', '\u{00A2}', '\u{00A3}', '\u{00A7}', '\u{2022}', '\u{00B6}', '\u{00DF}',
    // 0xA8..=0xAF
    '\u{00AE}', '\u{00A9}', '\u{2122}', '\u{00B4}', '\u{00A8}', '\u{2260}', '\u{00C6}', '\u{00D8}',
    // 0xB0..=0xB7
    '\u{221E}', '\u{00B1}', '\u{2264}', '\u{2265}', '\u{00A5}', '\u{00B5}', '\u{2202}', '\u{2211}',
    // 0xB8..=0xBF
    '\u{220F}', '\u{03C0}', '\u{222B}', '\u{00AA}', '\u{00BA}', '\u{03A9}', '\u{00E6}', '\u{00F8}',
    // 0xC0..=0xC7
    '\u{00BF}', '\u{00A1}', '\u{00AC}', '\u{221A}', '\u{0192}', '\u{2248}', '\u{2206}', '\u{00AB}',
    // 0xC8..=0xCF
    '\u{00BB}', '\u{2026}', '\u{00A0}', '\u{00C0}', '\u{00C3}', '\u{00D5}', '\u{0152}', '\u{0153}',
    // 0xD0..=0xD7
    '\u{2013}', '\u{2014}', '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '\u{00F7}', '\u{25CA}',
    // 0xD8..=0xDF
    '\u{00FF}', '\u{0178}', '\u{2044}', '\u{20AC}', '\u{2039}', '\u{203A}', '\u{FB01}', '\u{FB02}',
    // 0xE0..=0xE7
    '\u{2021}', '\u{00B7}', '\u{201A}', '\u{201E}', '\u{2030}', '\u{00C2}', '\u{00CA}', '\u{00C1}',
    // 0xE8..=0xEF
    '\u{00CB}', '\u{00C8}', '\u{00CD}', '\u{00CE}', '\u{00CF}', '\u{00CC}', '\u{00D3}', '\u{00D4}',
    // 0xF0..=0xF7
    '\u{F8FF}', '\u{00D2}', '\u{00DA}', '\u{00DB}', '\u{00D9}', '\u{0131}', '\u{02C6}', '\u{02DC}',
    // 0xF8..=0xFF
    '\u{00AF}', '\u{02D8}', '\u{02D9}', '\u{02DA}', '\u{00B8}', '\u{02DD}', '\u{02DB}', '\u{02C7}',
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclaredEncoding {
    Utf8,
    Utf16Any,
    Utf16Le,
    Utf16Be,
    Iso8859_1,
    Windows1252,
    MacRoman,
}

/// Detect the character encoding and BOM byte length of a raw XML byte buffer.
///
/// Detection order:
/// 1. Explicit BOM (`UTF-8`, `UTF-16LE`, `UTF-16BE`; `UTF-32` BOMs return `UnsupportedFormat`).
/// 2. BOM-less `UTF-16LE` / `UTF-16BE` leading XML syntax (`<` or leading XML whitespace + `<`).
/// 3. Optional `<?xml ... encoding="..."?>` declaration (`UTF-8`, `ISO-8859-1`, `Windows-1252`,
///    `MacRoman`, or matching `UTF-16*` for UTF-16 streams).
///
/// Unknown declared encodings always fail with [`FormatError::UnsupportedFormat`]; contradictory
/// BOM and declared encodings fail with [`FormatError::XmlParse`].
pub fn detect_xml_encoding(bytes: &[u8]) -> Result<(XmlEncoding, usize), FormatError> {
    if bytes.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) || bytes.starts_with(&[0x00, 0x00, 0x00, 0x3C]) {
        return Err(FormatError::UnsupportedFormat { reason: String::from("unsupported XML encoding `UTF-32BE`") });
    }
    if bytes.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) || bytes.starts_with(&[0x3C, 0x00, 0x00, 0x00]) {
        return Err(FormatError::UnsupportedFormat { reason: String::from("unsupported XML encoding `UTF-32LE`") });
    }

    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let after_bom = bytes.get(3..).unwrap_or_default();
        validate_utf8_bom_decl(after_bom)?;
        return Ok((XmlEncoding::Utf8, 3));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let after_bom = bytes.get(2..).unwrap_or_default();
        validate_utf16_decl(after_bom, XmlEncoding::Utf16Le)?;
        return Ok((XmlEncoding::Utf16Le, 2));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let after_bom = bytes.get(2..).unwrap_or_default();
        validate_utf16_decl(after_bom, XmlEncoding::Utf16Be)?;
        return Ok((XmlEncoding::Utf16Be, 2));
    }

    if let Some(utf16_enc) = detect_bomless_utf16(bytes) {
        validate_utf16_decl(bytes, utf16_enc)?;
        return Ok((utf16_enc, 0));
    }

    if let Some(declared_raw) = extract_xml_decl_encoding(bytes) {
        let declared = parse_declared_encoding_name(&declared_raw)?;
        let resolved = match declared {
            DeclaredEncoding::Utf8 => XmlEncoding::Utf8,
            DeclaredEncoding::Iso8859_1 => XmlEncoding::Iso8859_1,
            DeclaredEncoding::Windows1252 => XmlEncoding::Windows1252,
            DeclaredEncoding::MacRoman => XmlEncoding::MacRoman,
            DeclaredEncoding::Utf16Any | DeclaredEncoding::Utf16Le | DeclaredEncoding::Utf16Be => {
                return Err(FormatError::XmlParse {
                    offset: 0,
                    reason: format!("XML declaration specifies `{declared_raw}` but byte stream is not UTF-16"),
                });
            }
        };
        return Ok((resolved, 0));
    }

    Ok((XmlEncoding::Utf8, 0))
}

/// Validate an `encoding=` attribute value when parsing an already-decoded UTF-8 `&str`.
pub(crate) fn validate_str_declared_encoding(src: &str) -> Result<(), FormatError> {
    if let Some(declared_raw) = extract_xml_decl_encoding(src.as_bytes()) {
        let _ = parse_declared_encoding_name(&declared_raw)?;
    }
    Ok(())
}

fn validate_utf8_bom_decl(after_bom: &[u8]) -> Result<(), FormatError> {
    let Some(declared_raw) = extract_xml_decl_encoding(after_bom) else {
        return Ok(());
    };
    let declared = parse_declared_encoding_name(&declared_raw)?;
    if declared != DeclaredEncoding::Utf8 {
        return Err(FormatError::XmlParse { offset: 3, reason: format!("UTF-8 BOM conflicts with declared XML encoding `{declared_raw}`") });
    }
    Ok(())
}

fn validate_utf16_decl(after_bom: &[u8], wire: XmlEncoding) -> Result<(), FormatError> {
    let ascii_prefix = decode_utf16_ascii_decl_prefix(after_bom, wire);
    let Some(declared_raw) = extract_xml_decl_encoding(&ascii_prefix) else {
        return Ok(());
    };
    let declared = parse_declared_encoding_name(&declared_raw)?;
    let compatible = matches!(
        (wire, declared),
        (XmlEncoding::Utf16Le, DeclaredEncoding::Utf16Any | DeclaredEncoding::Utf16Le)
            | (XmlEncoding::Utf16Be, DeclaredEncoding::Utf16Any | DeclaredEncoding::Utf16Be)
    );
    if !compatible {
        return Err(FormatError::XmlParse {
            offset: 0,
            reason: format!("{} stream conflicts with declared XML encoding `{declared_raw}`", wire.label()),
        });
    }
    Ok(())
}

fn decode_utf16_ascii_decl_prefix(bytes: &[u8], wire: XmlEncoding) -> Vec<u8> {
    let mut out = Vec::new();
    let mut idx = 0_usize;
    let limit = bytes.len().min(512);
    while idx + 1 < limit {
        let (Some(&b0), Some(&b1)) = (bytes.get(idx), bytes.get(idx + 1)) else {
            break;
        };
        let unit = match wire {
            XmlEncoding::Utf16Le => u16::from_le_bytes([b0, b1]),
            XmlEncoding::Utf16Be => u16::from_be_bytes([b0, b1]),
            _ => break,
        };
        if unit > 0x7F {
            break;
        }
        out.push(unit as u8);
        idx += 2;
        if out.ends_with(b"?>") {
            break;
        }
    }
    out
}

fn detect_bomless_utf16(bytes: &[u8]) -> Option<XmlEncoding> {
    if looks_like_bomless_utf16_stream(bytes, true) {
        Some(XmlEncoding::Utf16Be)
    } else if looks_like_bomless_utf16_stream(bytes, false) {
        Some(XmlEncoding::Utf16Le)
    } else {
        None
    }
}

fn looks_like_bomless_utf16_stream(bytes: &[u8], big_endian: bool) -> bool {
    let mut idx = 0_usize;
    let scan_limit = bytes.len().min(4096);
    while idx + 1 < scan_limit {
        let (Some(&b0), Some(&b1)) = (bytes.get(idx), bytes.get(idx + 1)) else {
            return false;
        };
        let unit = if big_endian { u16::from_be_bytes([b0, b1]) } else { u16::from_le_bytes([b0, b1]) };
        if unit == u16::from(b'<') {
            return true;
        }
        if matches!(unit, 0x0020 | 0x0009 | 0x000A | 0x000D) {
            idx += 2;
            continue;
        }
        return false;
    }
    false
}

pub(crate) fn parse_declared_encoding_name(raw: &str) -> Result<DeclaredEncoding, FormatError> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "utf-8" | "utf8" | "us-ascii" | "ascii" => Ok(DeclaredEncoding::Utf8),
        "utf-16" | "utf16" => Ok(DeclaredEncoding::Utf16Any),
        "utf-16le" | "utf16le" | "utf-16-le" => Ok(DeclaredEncoding::Utf16Le),
        "utf-16be" | "utf16be" | "utf-16-be" => Ok(DeclaredEncoding::Utf16Be),
        "iso-8859-1" | "iso_8859-1" | "iso-8859_1" | "iso8859-1" | "iso88591" | "latin1" | "latin-1" | "l1" | "iso-ir-100" | "csisolatin1"
        | "ibm819" | "cp819" => Ok(DeclaredEncoding::Iso8859_1),
        "windows-1252" | "windows1252" | "cp1252" | "cp-1252" | "win-1252" | "win1252" | "x-cp1252" | "ms-ansi" => Ok(DeclaredEncoding::Windows1252),
        "macintosh" | "macroman" | "mac-roman" | "mac_roman" | "x-mac-roman" | "mac" | "csmacintosh" => Ok(DeclaredEncoding::MacRoman),
        _ => Err(FormatError::UnsupportedFormat { reason: format!("unsupported XML encoding `{}`", raw.trim()) }),
    }
}

/// Extract the `encoding="..."` / `encoding='...'` value from a leading ASCII `<?xml ... ?>`
/// declaration, if present.
pub(crate) fn extract_xml_decl_encoding(bytes: &[u8]) -> Option<String> {
    let after_prefix = bytes.strip_prefix(b"<?xml")?;
    let first = *after_prefix.first()?;
    if !is_ascii_xml_ws(first) {
        return None;
    }
    let search_len = after_prefix.len().min(4096);
    let window = after_prefix.get(..search_len)?;
    let close_pos = window.windows(2).position(|pair| pair == b"?>")?;
    let decl_body = window.get(..close_pos)?;
    parse_decl_encoding_pseudo_attr(decl_body)
}

fn parse_decl_encoding_pseudo_attr(mut rest: &[u8]) -> Option<String> {
    loop {
        rest = skip_ascii_xml_ws(rest);
        if rest.is_empty() {
            return None;
        }
        let name_len = rest.iter().position(|b| !b.is_ascii_alphanumeric() && *b != b'_' && *b != b'-' && *b != b'.')?;
        if name_len == 0 {
            return None;
        }
        let name = rest.get(..name_len)?;
        rest = skip_ascii_xml_ws(rest.get(name_len..)?);
        let after_eq = rest.strip_prefix(b"=")?;
        rest = skip_ascii_xml_ws(after_eq);
        let quote = *rest.first()?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        let after_open = rest.get(1..)?;
        let val_len = after_open.iter().position(|&b| b == quote)?;
        let val_bytes = after_open.get(..val_len)?;
        rest = after_open.get(val_len + 1..)?;
        if name == b"encoding" {
            let val_str = std::str::from_utf8(val_bytes).ok()?;
            return Some(val_str.to_owned());
        }
    }
}

const fn is_ascii_xml_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn skip_ascii_xml_ws(mut bytes: &[u8]) -> &[u8] {
    while let Some((&first, tail)) = bytes.split_first() {
        if is_ascii_xml_ws(first) {
            bytes = tail;
        } else {
            break;
        }
    }
    bytes
}

/// Incremental single-byte (`ISO-8859-1`, `Windows-1252`, `MacRoman`) to UTF-8 [`BufRead`] adapter.
///
/// Uses the same `<` / `>` chunk-boundary rule as [`Utf16ToUtf8Reader`] so trailing bytes past
/// `</Root>` are never decoded and raw byte offsets at tag boundaries stay exact.
pub(crate) struct SingleByteToUtf8Reader<'a> {
    input: &'a [u8],
    input_pos: usize,
    chunk_start_input_pos: usize,
    encoding: XmlEncoding,
    buf: Vec<u8>,
    buf_pos: usize,
    decode_error: Option<FormatError>,
}

impl<'a> SingleByteToUtf8Reader<'a> {
    pub(crate) fn new(input: &'a [u8], encoding: XmlEncoding) -> Self {
        Self {
            input,
            input_pos: 0,
            chunk_start_input_pos: 0,
            encoding,
            buf: Vec::with_capacity(STREAM_CHUNK_CAPACITY),
            buf_pos: 0,
            decode_error: None,
        }
    }

    pub(crate) const fn raw_input_pos(&self) -> usize {
        self.input_pos
    }

    pub(crate) const fn last_chunk_start_input_pos(&self) -> usize {
        self.chunk_start_input_pos
    }

    pub(crate) fn take_error(&mut self) -> Option<FormatError> {
        self.decode_error.take()
    }

    fn decode_byte(&self, byte: u8, offset: usize) -> Result<char, FormatError> {
        if byte < 0x80 {
            return Ok(char::from(byte));
        }
        match self.encoding {
            XmlEncoding::Iso8859_1 => Ok(char::from(byte)),
            XmlEncoding::Windows1252 => CP1252_HIGH
                .get(usize::from(byte - 0x80))
                .copied()
                .flatten()
                .ok_or_else(|| FormatError::XmlParse { offset, reason: format!("undefined Windows-1252 byte 0x{byte:02X}") }),
            XmlEncoding::MacRoman => MAC_ROMAN_HIGH
                .get(usize::from(byte - 0x80))
                .copied()
                .ok_or_else(|| FormatError::XmlParse { offset, reason: format!("invalid MacRoman byte 0x{byte:02X}") }),
            XmlEncoding::Utf8 | XmlEncoding::Utf16Le | XmlEncoding::Utf16Be => Ok(char::from(byte)),
        }
    }

    fn refill(&mut self) -> io::Result<()> {
        self.buf.clear();
        self.buf_pos = 0;
        self.chunk_start_input_pos = self.input_pos;

        while self.buf.len() + 4 <= STREAM_CHUNK_CAPACITY {
            let Some(&byte) = self.input.get(self.input_pos) else {
                break;
            };
            if byte == b'<' && !self.buf.is_empty() {
                break;
            }
            let ch = match self.decode_byte(byte, self.input_pos) {
                Ok(ch) => ch,
                Err(err) => {
                    self.decode_error = Some(err);
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid single-byte XML character"));
                }
            };
            self.input_pos += 1;
            let mut enc = [0_u8; 4];
            let encoded = ch.encode_utf8(&mut enc);
            self.buf.extend_from_slice(encoded.as_bytes());
            if byte == b'>' {
                break;
            }
        }
        Ok(())
    }
}

impl Read for SingleByteToUtf8Reader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(out.len());
        if let (Some(dst), Some(src)) = (out.get_mut(..count), available.get(..count)) {
            dst.copy_from_slice(src);
        }
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for SingleByteToUtf8Reader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.buf_pos >= self.buf.len() {
            self.refill()?;
        }
        Ok(self.buf.get(self.buf_pos..).unwrap_or_default())
    }

    fn consume(&mut self, amt: usize) {
        self.buf_pos = self.buf_pos.saturating_add(amt).min(self.buf.len());
    }
}

/// Check whether a raw trailing slice after the root element consists solely of XML 1.0
/// whitespace (`#x20 | #x9 | #xD | #xA`) in `encoding`, stopping immediately at the first
/// non-whitespace byte without decoding the tail.
pub(crate) fn tail_is_only_xml_whitespace(tail: &[u8], encoding: XmlEncoding) -> bool {
    match encoding {
        XmlEncoding::Utf8 | XmlEncoding::Iso8859_1 | XmlEncoding::Windows1252 | XmlEncoding::MacRoman => tail.iter().all(|&b| is_ascii_xml_ws(b)),
        XmlEncoding::Utf16Le | XmlEncoding::Utf16Be => {
            if !tail.len().is_multiple_of(2) {
                return false;
            }
            let mut idx = 0_usize;
            while idx + 1 < tail.len() {
                let (Some(&b0), Some(&b1)) = (tail.get(idx), tail.get(idx + 1)) else {
                    return false;
                };
                let unit = if encoding == XmlEncoding::Utf16Be { u16::from_be_bytes([b0, b1]) } else { u16::from_le_bytes([b0, b1]) };
                if !matches!(unit, 0x0020 | 0x0009 | 0x000A | 0x000D) {
                    return false;
                }
                idx += 2;
            }
            true
        }
    }
}

/// Incremental UTF-16LE / UTF-16BE to UTF-8 [`BufRead`] adapter.
///
/// Stops refilling immediately after emitting `>` (`U+003E`) and immediately before emitting `<`
/// (`U+003C`) when the internal buffer is non-empty, guaranteeing:
/// 1. Exact raw UTF-16 byte positions at every element tag boundary (`raw_input_pos()` and
///    `last_chunk_start_input_pos()`), and
/// 2. Zero bytes of trailing data past `</Root>` are ever read or decoded.
pub(crate) struct Utf16ToUtf8Reader<'a> {
    input: &'a [u8],
    input_pos: usize,
    chunk_start_input_pos: usize,
    big_endian: bool,
    buf: Vec<u8>,
    buf_pos: usize,
    decode_error: Option<FormatError>,
}

const STREAM_CHUNK_CAPACITY: usize = 4096;

impl<'a> Utf16ToUtf8Reader<'a> {
    pub(crate) fn new(input: &'a [u8], big_endian: bool) -> Self {
        Self {
            input,
            input_pos: 0,
            chunk_start_input_pos: 0,
            big_endian,
            buf: Vec::with_capacity(STREAM_CHUNK_CAPACITY),
            buf_pos: 0,
            decode_error: None,
        }
    }

    /// Raw byte offset consumed so far from `input`.
    pub(crate) const fn raw_input_pos(&self) -> usize {
        self.input_pos
    }

    /// Raw byte offset where the most recently filled chunk started.
    /// Because every `<...>` tag starts a fresh chunk, this gives the exact `<` byte offset of
    /// the current tag.
    pub(crate) const fn last_chunk_start_input_pos(&self) -> usize {
        self.chunk_start_input_pos
    }

    /// Take any stored structured UTF-16 decoding error.
    pub(crate) fn take_error(&mut self) -> Option<FormatError> {
        self.decode_error.take()
    }

    fn read_unit(&self, pos: usize) -> Option<u16> {
        let (&b0, &b1) = (self.input.get(pos)?, self.input.get(pos + 1)?);
        Some(if self.big_endian { u16::from_be_bytes([b0, b1]) } else { u16::from_le_bytes([b0, b1]) })
    }

    fn refill(&mut self) -> io::Result<()> {
        self.buf.clear();
        self.buf_pos = 0;
        self.chunk_start_input_pos = self.input_pos;

        while self.buf.len() + 4 <= STREAM_CHUNK_CAPACITY {
            if self.input_pos >= self.input.len() {
                break;
            }
            if self.input_pos + 1 >= self.input.len() {
                self.decode_error =
                    Some(FormatError::XmlParse { offset: self.input_pos, reason: String::from("truncated trailing byte in UTF-16 XML stream") });
                return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated UTF-16 stream"));
            }
            let Some(unit) = self.read_unit(self.input_pos) else {
                break;
            };
            if unit == u16::from(b'<') && !self.buf.is_empty() {
                break;
            }
            let ch = self.decode_next_char(unit)?;
            let mut enc = [0_u8; 4];
            let encoded = ch.encode_utf8(&mut enc);
            self.buf.extend_from_slice(encoded.as_bytes());
            if unit == u16::from(b'>') {
                break;
            }
        }
        Ok(())
    }

    fn decode_next_char(&mut self, unit: u16) -> io::Result<char> {
        if (0xD800..=0xDBFF).contains(&unit) {
            let Some(low) = self.read_unit(self.input_pos + 2) else {
                self.decode_error =
                    Some(FormatError::XmlParse { offset: self.input_pos, reason: format!("unpaired UTF-16 high surrogate 0x{unit:04X}") });
                return Err(io::Error::new(io::ErrorKind::InvalidData, "unpaired UTF-16 high surrogate"));
            };
            if !(0xDC00..=0xDFFF).contains(&low) {
                self.decode_error =
                    Some(FormatError::XmlParse { offset: self.input_pos, reason: format!("invalid UTF-16 surrogate pair 0x{unit:04X} 0x{low:04X}") });
                return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-16 surrogate pair"));
            }
            let scalar = 0x1_0000_u32 + ((u32::from(unit - 0xD800)) << 10) + u32::from(low - 0xDC00);
            let Some(ch) = char::from_u32(scalar) else {
                self.decode_error = Some(FormatError::XmlParse { offset: self.input_pos, reason: format!("invalid UTF-16 scalar 0x{scalar:06X}") });
                return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-16 scalar"));
            };
            self.input_pos += 4;
            Ok(ch)
        } else if (0xDC00..=0xDFFF).contains(&unit) {
            self.decode_error = Some(FormatError::XmlParse { offset: self.input_pos, reason: format!("unpaired UTF-16 low surrogate 0x{unit:04X}") });
            Err(io::Error::new(io::ErrorKind::InvalidData, "unpaired UTF-16 low surrogate"))
        } else {
            let Some(ch) = char::from_u32(u32::from(unit)) else {
                self.decode_error = Some(FormatError::XmlParse { offset: self.input_pos, reason: format!("invalid UTF-16 code unit 0x{unit:04X}") });
                return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-16 code unit"));
            };
            self.input_pos += 2;
            Ok(ch)
        }
    }
}

impl Read for Utf16ToUtf8Reader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(out.len());
        if let (Some(dst), Some(src)) = (out.get_mut(..count), available.get(..count)) {
            dst.copy_from_slice(src);
        }
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for Utf16ToUtf8Reader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.buf_pos >= self.buf.len() {
            self.refill()?;
        }
        Ok(self.buf.get(self.buf_pos..).unwrap_or_default())
    }

    fn consume(&mut self, amt: usize) {
        self.buf_pos = self.buf_pos.saturating_add(amt).min(self.buf.len());
    }
}
