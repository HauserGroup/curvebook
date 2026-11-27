//! IBM Code Page 437 filename decoding for ZIP archives.
//!
//! Sources:
//! - PKWARE `APPNOTE.TXT` Appendix D (Language Encoding / EFS): when general-purpose bit 11
//!   (`0x0800`) is unset, entry filenames default to IBM Code Page 437.
//! - Unicode Consortium `CP437.TXT` (`MAPPINGS/VENDORS/MICSFT/PC/CP437.TXT`, version 2.01):
//!   exact static mapping for bytes `0x80..=0xFF` (`0x00..=0x7F` map identically to ASCII).

use crate::error::FormatError;

const FLAG_UTF8: u16 = 0x0800;

/// Static CP437 high-byte mapping (`0x80..=0xFF`) from Unicode Consortium `CP437.TXT`.
#[rustfmt::skip]
const CP437_HIGH: [char; 128] = [
    // 0x80..=0x87
    '\u{00C7}', '\u{00FC}', '\u{00E9}', '\u{00E2}', '\u{00E4}', '\u{00E0}', '\u{00E5}', '\u{00E7}',
    // 0x88..=0x8F
    '\u{00EA}', '\u{00EB}', '\u{00E8}', '\u{00EF}', '\u{00EE}', '\u{00EC}', '\u{00C4}', '\u{00C5}',
    // 0x90..=0x97
    '\u{00C9}', '\u{00E6}', '\u{00C6}', '\u{00F4}', '\u{00F6}', '\u{00F2}', '\u{00FB}', '\u{00F9}',
    // 0x98..=0x9F
    '\u{00FF}', '\u{00D6}', '\u{00DC}', '\u{00A2}', '\u{00A3}', '\u{00A5}', '\u{20A7}', '\u{0192}',
    // 0xA0..=0xA7
    '\u{00E1}', '\u{00ED}', '\u{00F3}', '\u{00FA}', '\u{00F1}', '\u{00D1}', '\u{00AA}', '\u{00BA}',
    // 0xA8..=0xAF
    '\u{00BF}', '\u{2310}', '\u{00AC}', '\u{00BD}', '\u{00BC}', '\u{00A1}', '\u{00AB}', '\u{00BB}',
    // 0xB0..=0xB7
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    // 0xB8..=0xBF
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255D}', '\u{255C}', '\u{255B}', '\u{2510}',
    // 0xC0..=0xC7
    '\u{2514}', '\u{2534}', '\u{252C}', '\u{251C}', '\u{2500}', '\u{253C}', '\u{255E}', '\u{255F}',
    // 0xC8..=0xCF
    '\u{255A}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256C}', '\u{2567}',
    // 0xD0..=0xD7
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256B}',
    // 0xD8..=0xDF
    '\u{256A}', '\u{2518}', '\u{250C}', '\u{2588}', '\u{2584}', '\u{258C}', '\u{2590}', '\u{2580}',
    // 0xE0..=0xE7
    '\u{03B1}', '\u{00DF}', '\u{0393}', '\u{03C0}', '\u{03A3}', '\u{03C3}', '\u{00B5}', '\u{03C4}',
    // 0xE8..=0xEF
    '\u{03A6}', '\u{0398}', '\u{03A9}', '\u{03B4}', '\u{221E}', '\u{03C6}', '\u{03B5}', '\u{2229}',
    // 0xF0..=0xF7
    '\u{2261}', '\u{00B1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{00F7}', '\u{2248}',
    // 0xF8..=0xFF
    '\u{00B0}', '\u{2219}', '\u{00B7}', '\u{221A}', '\u{207F}', '\u{00B2}', '\u{25A0}', '\u{00A0}',
];

/// Decode a raw byte slice using IBM Code Page 437 (`CP437.TXT`).
#[must_use]
pub fn decode_cp437(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b < 0x80 {
            out.push(char::from(b));
        } else if let Some(&ch) = CP437_HIGH.get(usize::from(b - 0x80)) {
            out.push(ch);
        }
    }
    out
}

/// Decode a ZIP central-directory entry filename according to general-purpose bit 11 (UTF-8 flag)
/// and whether CP437 fallback is enabled.
///
/// - When `decode_cp437_fallback` is `false` (strict native mode): requires valid UTF-8.
/// - When `decode_cp437_fallback` is `true` and bit 11 is set: requires valid UTF-8.
/// - When `decode_cp437_fallback` is `true` and bit 11 is unset: accepts valid UTF-8 (for tools
///   that omit bit 11 on UTF-8 paths) and decodes non-UTF-8 bytes as CP437 per APPNOTE Appendix D.
pub(crate) fn decode_zip_entry_name(bytes: &[u8], flags: u16, decode_cp437_fallback: bool) -> Result<String, FormatError> {
    if let Ok(utf8) = std::str::from_utf8(bytes) {
        return Ok(utf8.to_owned());
    }
    if decode_cp437_fallback && (flags & FLAG_UTF8) == 0 {
        return Ok(decode_cp437(bytes));
    }
    Err(FormatError::InvalidZip { reason: String::from("non-UTF-8 ZIP entry filename") })
}
