//! XML 1.0 predefined entities, W3C HTML 4.01 named character entities, scientific aliases, and
//! numeric character reference decoding.
//!
//! Sources:
//! - W3C Extensible Markup Language (XML) 1.0 (Fifth Edition), §2.2 (`Char`) and §4.6 (predefined
//!   entities: `amp`, `apos`, `gt`, `lt`, `quot`).
//! - W3C HTML 4.01 Specification, Chapter 24: Character entity references (`HTMLlat1.ent`,
//!   `HTMLsymbol.ent`, and `HTMLspecial.ent` — all 252 Latin-1, symbol, Greek, mathematical,
//!   currency, and punctuation named entities).
//! - CurveBook scientific aliases (`approx`, `bullet`, `cdot`, `div`, `geq`, `leq`, `mp`, `neq`,
//!   `ohm`, `pm`, `varepsilon`, `varkappa`, `varpi`, `varrho`, `varsigma`, `vartheta`, `varphi`).
//!
//! `<!DOCTYPE` and `<!ENTITY` declarations remain strictly forbidden at the parser level.

use crate::error::FormatError;

/// Sorted static lookup table of named XML/HTML 4 character entities (`&name;`).
/// Kept strictly sorted in Rust `&str` lexicographical order for `O(log n)` binary search.
const NAMED_XML_ENTITIES: &[(&str, char)] = &[
    ("AElig", '\u{00C6}'),
    ("Aacute", '\u{00C1}'),
    ("Acirc", '\u{00C2}'),
    ("Agrave", '\u{00C0}'),
    ("Alpha", '\u{0391}'),
    ("Aring", '\u{00C5}'),
    ("Atilde", '\u{00C3}'),
    ("Auml", '\u{00C4}'),
    ("Beta", '\u{0392}'),
    ("Ccedil", '\u{00C7}'),
    ("Chi", '\u{03A7}'),
    ("Dagger", '\u{2021}'),
    ("Delta", '\u{0394}'),
    ("ETH", '\u{00D0}'),
    ("Eacute", '\u{00C9}'),
    ("Ecirc", '\u{00CA}'),
    ("Egrave", '\u{00C8}'),
    ("Epsilon", '\u{0395}'),
    ("Eta", '\u{0397}'),
    ("Euml", '\u{00CB}'),
    ("Gamma", '\u{0393}'),
    ("Iacute", '\u{00CD}'),
    ("Icirc", '\u{00CE}'),
    ("Igrave", '\u{00CC}'),
    ("Iota", '\u{0399}'),
    ("Iuml", '\u{00CF}'),
    ("Kappa", '\u{039A}'),
    ("Lambda", '\u{039B}'),
    ("Mu", '\u{039C}'),
    ("Ntilde", '\u{00D1}'),
    ("Nu", '\u{039D}'),
    ("OElig", '\u{0152}'),
    ("Oacute", '\u{00D3}'),
    ("Ocirc", '\u{00D4}'),
    ("Ograve", '\u{00D2}'),
    ("Omega", '\u{03A9}'),
    ("Omicron", '\u{039F}'),
    ("Oslash", '\u{00D8}'),
    ("Otilde", '\u{00D5}'),
    ("Ouml", '\u{00D6}'),
    ("Phi", '\u{03A6}'),
    ("Pi", '\u{03A0}'),
    ("Prime", '\u{2033}'),
    ("Psi", '\u{03A8}'),
    ("Rho", '\u{03A1}'),
    ("Scaron", '\u{0160}'),
    ("Sigma", '\u{03A3}'),
    ("THORN", '\u{00DE}'),
    ("Tau", '\u{03A4}'),
    ("Theta", '\u{0398}'),
    ("Uacute", '\u{00DA}'),
    ("Ucirc", '\u{00DB}'),
    ("Ugrave", '\u{00D9}'),
    ("Upsilon", '\u{03A5}'),
    ("Uuml", '\u{00DC}'),
    ("Xi", '\u{039E}'),
    ("Yacute", '\u{00DD}'),
    ("Yuml", '\u{0178}'),
    ("Zeta", '\u{0396}'),
    ("aacute", '\u{00E1}'),
    ("acirc", '\u{00E2}'),
    ("acute", '\u{00B4}'),
    ("aelig", '\u{00E6}'),
    ("agrave", '\u{00E0}'),
    ("alefsym", '\u{2135}'),
    ("alpha", '\u{03B1}'),
    ("amp", '&'),
    ("and", '\u{2227}'),
    ("ang", '\u{2220}'),
    ("apos", '\''),
    ("approx", '\u{2248}'),
    ("aring", '\u{00E5}'),
    ("asymp", '\u{2248}'),
    ("atilde", '\u{00E3}'),
    ("auml", '\u{00E4}'),
    ("bdquo", '\u{201E}'),
    ("beta", '\u{03B2}'),
    ("brvbar", '\u{00A6}'),
    ("bull", '\u{2022}'),
    ("bullet", '\u{2022}'),
    ("cap", '\u{2229}'),
    ("ccedil", '\u{00E7}'),
    ("cdot", '\u{00B7}'),
    ("cedil", '\u{00B8}'),
    ("cent", '\u{00A2}'),
    ("chi", '\u{03C7}'),
    ("circ", '\u{02C6}'),
    ("clubs", '\u{2663}'),
    ("cong", '\u{2245}'),
    ("copy", '\u{00A9}'),
    ("crarr", '\u{21B5}'),
    ("cup", '\u{222A}'),
    ("curren", '\u{00A4}'),
    ("dArr", '\u{21D3}'),
    ("dagger", '\u{2020}'),
    ("darr", '\u{2193}'),
    ("deg", '\u{00B0}'),
    ("delta", '\u{03B4}'),
    ("diams", '\u{2666}'),
    ("div", '\u{00F7}'),
    ("divide", '\u{00F7}'),
    ("eacute", '\u{00E9}'),
    ("ecirc", '\u{00EA}'),
    ("egrave", '\u{00E8}'),
    ("empty", '\u{2205}'),
    ("emsp", '\u{2003}'),
    ("ensp", '\u{2002}'),
    ("epsilon", '\u{03B5}'),
    ("equiv", '\u{2261}'),
    ("eta", '\u{03B7}'),
    ("eth", '\u{00F0}'),
    ("euml", '\u{00EB}'),
    ("euro", '\u{20AC}'),
    ("exist", '\u{2203}'),
    ("fnof", '\u{0192}'),
    ("forall", '\u{2200}'),
    ("frac12", '\u{00BD}'),
    ("frac14", '\u{00BC}'),
    ("frac34", '\u{00BE}'),
    ("frasl", '\u{2044}'),
    ("gamma", '\u{03B3}'),
    ("ge", '\u{2265}'),
    ("geq", '\u{2265}'),
    ("gt", '>'),
    ("hArr", '\u{21D4}'),
    ("harr", '\u{2194}'),
    ("hearts", '\u{2665}'),
    ("hellip", '\u{2026}'),
    ("iacute", '\u{00ED}'),
    ("icirc", '\u{00EE}'),
    ("iexcl", '\u{00A1}'),
    ("igrave", '\u{00EC}'),
    ("image", '\u{2111}'),
    ("infin", '\u{221E}'),
    ("int", '\u{222B}'),
    ("iota", '\u{03B9}'),
    ("iquest", '\u{00BF}'),
    ("isin", '\u{2208}'),
    ("iuml", '\u{00EF}'),
    ("kappa", '\u{03BA}'),
    ("lArr", '\u{21D0}'),
    ("lambda", '\u{03BB}'),
    ("lang", '\u{2329}'),
    ("laquo", '\u{00AB}'),
    ("larr", '\u{2190}'),
    ("lceil", '\u{2308}'),
    ("ldquo", '\u{201C}'),
    ("le", '\u{2264}'),
    ("leq", '\u{2264}'),
    ("lfloor", '\u{230A}'),
    ("lowast", '\u{2217}'),
    ("loz", '\u{25CA}'),
    ("lrm", '\u{200E}'),
    ("lsaquo", '\u{2039}'),
    ("lsquo", '\u{2018}'),
    ("lt", '<'),
    ("macr", '\u{00AF}'),
    ("mdash", '\u{2014}'),
    ("micro", '\u{00B5}'),
    ("middot", '\u{00B7}'),
    ("minus", '\u{2212}'),
    ("mp", '\u{2213}'),
    ("mu", '\u{03BC}'),
    ("nabla", '\u{2207}'),
    ("nbsp", '\u{00A0}'),
    ("ndash", '\u{2013}'),
    ("ne", '\u{2260}'),
    ("neq", '\u{2260}'),
    ("ni", '\u{220B}'),
    ("not", '\u{00AC}'),
    ("notin", '\u{2209}'),
    ("nsub", '\u{2284}'),
    ("ntilde", '\u{00F1}'),
    ("nu", '\u{03BD}'),
    ("oacute", '\u{00F3}'),
    ("ocirc", '\u{00F4}'),
    ("oelig", '\u{0153}'),
    ("ograve", '\u{00F2}'),
    ("ohm", '\u{03A9}'),
    ("oline", '\u{203E}'),
    ("omega", '\u{03C9}'),
    ("omicron", '\u{03BF}'),
    ("oplus", '\u{2295}'),
    ("or", '\u{2228}'),
    ("ordf", '\u{00AA}'),
    ("ordm", '\u{00BA}'),
    ("oslash", '\u{00F8}'),
    ("otilde", '\u{00F5}'),
    ("otimes", '\u{2297}'),
    ("ouml", '\u{00F6}'),
    ("para", '\u{00B6}'),
    ("part", '\u{2202}'),
    ("permil", '\u{2030}'),
    ("perp", '\u{22A5}'),
    ("phi", '\u{03C6}'),
    ("pi", '\u{03C0}'),
    ("piv", '\u{03D6}'),
    ("plusmn", '\u{00B1}'),
    ("pm", '\u{00B1}'),
    ("pound", '\u{00A3}'),
    ("prime", '\u{2032}'),
    ("prod", '\u{220F}'),
    ("prop", '\u{221D}'),
    ("psi", '\u{03C8}'),
    ("quot", '"'),
    ("rArr", '\u{21D2}'),
    ("radic", '\u{221A}'),
    ("rang", '\u{232A}'),
    ("raquo", '\u{00BB}'),
    ("rarr", '\u{2192}'),
    ("rceil", '\u{2309}'),
    ("rdquo", '\u{201D}'),
    ("real", '\u{211C}'),
    ("reg", '\u{00AE}'),
    ("rfloor", '\u{230B}'),
    ("rho", '\u{03C1}'),
    ("rlm", '\u{200F}'),
    ("rsaquo", '\u{203A}'),
    ("rsquo", '\u{2019}'),
    ("sbquo", '\u{201A}'),
    ("scaron", '\u{0161}'),
    ("sdot", '\u{22C5}'),
    ("sect", '\u{00A7}'),
    ("shy", '\u{00AD}'),
    ("sigma", '\u{03C3}'),
    ("sigmaf", '\u{03C2}'),
    ("sim", '\u{223C}'),
    ("spades", '\u{2660}'),
    ("sub", '\u{2282}'),
    ("sube", '\u{2286}'),
    ("sum", '\u{2211}'),
    ("sup", '\u{2283}'),
    ("sup1", '\u{00B9}'),
    ("sup2", '\u{00B2}'),
    ("sup3", '\u{00B3}'),
    ("supe", '\u{2287}'),
    ("szlig", '\u{00DF}'),
    ("tau", '\u{03C4}'),
    ("there4", '\u{2234}'),
    ("theta", '\u{03B8}'),
    ("thetasym", '\u{03D1}'),
    ("thinsp", '\u{2009}'),
    ("thorn", '\u{00FE}'),
    ("tilde", '\u{02DC}'),
    ("times", '\u{00D7}'),
    ("trade", '\u{2122}'),
    ("uArr", '\u{21D1}'),
    ("uacute", '\u{00FA}'),
    ("uarr", '\u{2191}'),
    ("ucirc", '\u{00FB}'),
    ("ugrave", '\u{00F9}'),
    ("uml", '\u{00A8}'),
    ("upsih", '\u{03D2}'),
    ("upsilon", '\u{03C5}'),
    ("uuml", '\u{00FC}'),
    ("varepsilon", '\u{03F5}'),
    ("varkappa", '\u{03F0}'),
    ("varphi", '\u{03D5}'),
    ("varpi", '\u{03D6}'),
    ("varrho", '\u{03F1}'),
    ("varsigma", '\u{03C2}'),
    ("vartheta", '\u{03D1}'),
    ("weierp", '\u{2118}'),
    ("xi", '\u{03BE}'),
    ("yacute", '\u{00FD}'),
    ("yen", '\u{00A5}'),
    ("yuml", '\u{00FF}'),
    ("zeta", '\u{03B6}'),
    ("zwj", '\u{200D}'),
    ("zwnj", '\u{200C}'),
];

/// Enforce XML 1.0 §2.2 `Char` validity (`#x9 | #xA | #xD | [#x20-#xD7FF] | [#xE000-#xFFFD] | [#x10000-#x10FFFF]`).
#[must_use]
pub const fn is_valid_xml10_char(ch: char) -> bool {
    matches!(ch as u32, 0x09 | 0x0A | 0x0D | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

/// Look up a named XML or W3C HTML 4 character entity (without leading `&` or trailing `;`).
#[must_use]
pub fn lookup_named_xml_entity(entity: &str) -> Option<char> {
    let idx = NAMED_XML_ENTITIES.binary_search_by_key(&entity, |(name, _)| *name).ok()?;
    NAMED_XML_ENTITIES.get(idx).map(|(_, ch)| *ch)
}

/// Decode standard XML character entities (`&lt;`, `&gt;`, `&amp;`, `&apos;`, `&quot;`), W3C HTML 4
/// named entities, scientific aliases, and numeric character references (`&#123;`, `&#x1F;`).
pub fn decode_xml_entities(raw: &str, base_offset: usize) -> Result<String, FormatError> {
    if !raw.contains('&') {
        return Ok(raw.to_owned());
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some((before, after_amp)) = rest.split_once('&') {
        out.push_str(before);
        let Some((entity, after_semi)) = after_amp.split_once(';') else {
            return Err(FormatError::XmlParse { offset: base_offset, reason: String::from("unterminated XML entity reference") });
        };
        out.push(decode_xml_entity_name(entity, base_offset)?);
        rest = after_semi;
    }
    out.push_str(rest);
    Ok(out)
}

pub(crate) fn decode_xml_entity_name(entity: &str, base_offset: usize) -> Result<char, FormatError> {
    if let Some(ch) = lookup_named_xml_entity(entity) {
        Ok(ch)
    } else if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
        let code = u32::from_str_radix(hex, 16)
            .map_err(|_| FormatError::XmlParse { offset: base_offset, reason: format!("invalid hex XML entity `&{entity};`") })?;
        decode_numeric_xml_char(code, entity, base_offset)
    } else if let Some(digits) = entity.strip_prefix('#') {
        let code = digits
            .parse::<u32>()
            .map_err(|_| FormatError::XmlParse { offset: base_offset, reason: format!("invalid decimal XML entity `&{entity};`") })?;
        decode_numeric_xml_char(code, entity, base_offset)
    } else {
        Err(FormatError::XmlParse { offset: base_offset, reason: format!("unknown XML entity `&{entity};`") })
    }
}

fn decode_numeric_xml_char(code: u32, entity: &str, base_offset: usize) -> Result<char, FormatError> {
    let ch =
        char::from_u32(code).ok_or_else(|| FormatError::XmlParse { offset: base_offset, reason: format!("invalid Unicode scalar `&{entity};`") })?;
    if !is_valid_xml10_char(ch) {
        return Err(FormatError::XmlParse { offset: base_offset, reason: format!("forbidden XML 1.0 character reference `&{entity};`") });
    }
    Ok(ch)
}

#[cfg(test)]
mod tests {
    use super::{NAMED_XML_ENTITIES, lookup_named_xml_entity};

    #[test]
    fn named_entity_table_is_strictly_sorted_and_unique() {
        for pair in NAMED_XML_ENTITIES.windows(2) {
            assert!(pair[0].0 < pair[1].0, "entity table out of order or duplicate: `{}` vs `{}`", pair[0].0, pair[1].0);
        }
        for &(name, ch) in NAMED_XML_ENTITIES {
            assert_eq!(lookup_named_xml_entity(name), Some(ch));
        }
    }
}
