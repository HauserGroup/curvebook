//! `.pzfx` rich-text, Symbol-font, strict ISO-8601 date, and colour-tag helpers.
//!
//! Sources:
//! - Unicode Consortium `SYMBOL.TXT` (`MAPPINGS/VENDORS/ADOBE/SYMBOL.TXT`, updated with standard
//!   Unicode 3.2+ Miscellaneous Technical bracket/extender scalar values `U+239B..=U+23AF`,
//!   `U+23D0`, and `U+203E` in place of legacy Private Use Area code points `U+F8E5..=U+F8FE`,
//!   plus Euro `U+20AC` at `0xA0`).
//! - CurveBook inline rich-text markup grammar (`crates/text/src/markup.rs`: `<b>`, `<i>`,
//!   `<sup>`, `<sub>`, and backslash escapes for literal markup triggers).
//! - W3C CSS Color Module Level 3 neutral named sRGB colours and `#RRGGBB` / `0xRRGGBB` hex forms.
//! - ISO-8601 extended calendar date format (`YYYY-MM-DD`) validated structurally with static
//!   per-month day bounds and zero Gregorian leap-year or calendar arithmetic (`AGENTS.md` §1).

use curvebook_doc::{ConstantValue, FloatingNote};
use curvebook_palette::Rgba8;

use crate::error::FormatError;
use crate::xml::{XmlElement, XmlNode, XmlWriter};

const MAX_RICHTEXT_DEPTH: usize = 128;

/// Neutral 1-based highlight-tag sRGB palette for `.pzfx` `Table@Highlighted` (`"1"`..`"7"`).
pub(crate) const HIGHLIGHT_PALETTE: [(&str, Rgba8); 7] = [
    ("1", Rgba8::rgb(0xFF, 0xE6, 0x80)), // Yellow
    ("2", Rgba8::rgb(0xFF, 0xA8, 0xA8)), // Red
    ("3", Rgba8::rgb(0xA8, 0xE6, 0xB8)), // Green
    ("4", Rgba8::rgb(0xA8, 0xD0, 0xFF)), // Blue
    ("5", Rgba8::rgb(0xD8, 0xB8, 0xFF)), // Purple
    ("6", Rgba8::rgb(0xFF, 0xC8, 0x90)), // Orange
    ("7", Rgba8::rgb(0xD0, 0xD4, 0xDA)), // Gray
];

/// Corpus-observed `FloatingNote@Color` named values (`Yellow`, `Blue`, `Gray`, `Red`).
const FLOATING_NOTE_COLORS: [(&str, Rgba8); 4] = [
    ("Yellow", Rgba8::rgb(0xFF, 0xFF, 0x00)),
    ("Blue", Rgba8::rgb(0x00, 0x00, 0xFF)),
    ("Gray", Rgba8::rgb(0x80, 0x80, 0x80)),
    ("Red", Rgba8::rgb(0xFF, 0x00, 0x00)),
];

/// Static Symbol-font single-byte mapping table (`0x00..=0xFF`) from Unicode Consortium
/// `MAPPINGS/VENDORS/ADOBE/SYMBOL.TXT` (with standard Unicode 3.2+ code points for extensible
/// mathematical delimiters and Euro `U+20AC` at `0xA0`).
#[rustfmt::skip]
const SYMBOL_TABLE: [Option<char>; 256] = [
    // 0x00..=0x1F (control characters — unmapped)
    None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None,
    // 0x20..=0x27: space, !, ∀, #, ∃, %, &, ∋
    Some('\u{0020}'), Some('\u{0021}'), Some('\u{2200}'), Some('\u{0023}'),
    Some('\u{2203}'), Some('\u{0025}'), Some('\u{0026}'), Some('\u{220B}'),
    // 0x28..=0x2F: (, ), ∗, +, ,, −, ., /
    Some('\u{0028}'), Some('\u{0029}'), Some('\u{2217}'), Some('\u{002B}'),
    Some('\u{002C}'), Some('\u{2212}'), Some('\u{002E}'), Some('\u{002F}'),
    // 0x30..=0x37: 0..7
    Some('\u{0030}'), Some('\u{0031}'), Some('\u{0032}'), Some('\u{0033}'),
    Some('\u{0034}'), Some('\u{0035}'), Some('\u{0036}'), Some('\u{0037}'),
    // 0x38..=0x3F: 8, 9, :, ;, <, =, >, ?
    Some('\u{0038}'), Some('\u{0039}'), Some('\u{003A}'), Some('\u{003B}'),
    Some('\u{003C}'), Some('\u{003D}'), Some('\u{003E}'), Some('\u{003F}'),
    // 0x40..=0x47: ≅, Α, Β, Χ, Δ, Ε, Φ, Γ
    Some('\u{2245}'), Some('\u{0391}'), Some('\u{0392}'), Some('\u{03A7}'),
    Some('\u{0394}'), Some('\u{0395}'), Some('\u{03A6}'), Some('\u{0393}'),
    // 0x48..=0x4F: Η, Ι, ϑ, Κ, Λ, Μ, Ν, Ο
    Some('\u{0397}'), Some('\u{0399}'), Some('\u{03D1}'), Some('\u{039A}'),
    Some('\u{039B}'), Some('\u{039C}'), Some('\u{039D}'), Some('\u{039F}'),
    // 0x50..=0x57: Π, Θ, Ρ, Σ, Τ, Υ, ς, Ω
    Some('\u{03A0}'), Some('\u{0398}'), Some('\u{03A1}'), Some('\u{03A3}'),
    Some('\u{03A4}'), Some('\u{03A5}'), Some('\u{03C2}'), Some('\u{03A9}'),
    // 0x58..=0x5F: Ξ, Ψ, Ζ, [, ∴, ], ⊥, _
    Some('\u{039E}'), Some('\u{03A8}'), Some('\u{0396}'), Some('\u{005B}'),
    Some('\u{2234}'), Some('\u{005D}'), Some('\u{22A5}'), Some('\u{005F}'),
    // 0x60..=0x67: ‾ (radicalex), α, β, χ, δ, ε, φ, γ
    Some('\u{203E}'), Some('\u{03B1}'), Some('\u{03B2}'), Some('\u{03C7}'),
    Some('\u{03B4}'), Some('\u{03B5}'), Some('\u{03C6}'), Some('\u{03B3}'),
    // 0x68..=0x6F: η, ι, ϕ, κ, λ, μ, ν, ο
    Some('\u{03B7}'), Some('\u{03B9}'), Some('\u{03D5}'), Some('\u{03BA}'),
    Some('\u{03BB}'), Some('\u{03BC}'), Some('\u{03BD}'), Some('\u{03BF}'),
    // 0x70..=0x77: π, θ, ρ, σ, τ, υ, ϖ, ω
    Some('\u{03C0}'), Some('\u{03B8}'), Some('\u{03C1}'), Some('\u{03C3}'),
    Some('\u{03C4}'), Some('\u{03C5}'), Some('\u{03D6}'), Some('\u{03C9}'),
    // 0x78..=0x7F: ξ, ψ, ζ, {, |, }, ∼, (DEL unmapped)
    Some('\u{03BE}'), Some('\u{03C8}'), Some('\u{03B6}'), Some('\u{007B}'),
    Some('\u{007C}'), Some('\u{007D}'), Some('\u{223C}'), None,
    // 0x80..=0x9F (unused in Symbol encoding)
    None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None,
    // 0xA0..=0xA7: €, ϒ, ′, ≤, ⁄, ∞, ƒ, ♣
    Some('\u{20AC}'), Some('\u{03D2}'), Some('\u{2032}'), Some('\u{2264}'),
    Some('\u{2044}'), Some('\u{221E}'), Some('\u{0192}'), Some('\u{2663}'),
    // 0xA8..=0xAF: ♦, ♥, ♠, ↔, ←, ↑, →, ↓
    Some('\u{2666}'), Some('\u{2665}'), Some('\u{2660}'), Some('\u{2194}'),
    Some('\u{2190}'), Some('\u{2191}'), Some('\u{2192}'), Some('\u{2193}'),
    // 0xB0..=0xB7: °, ±, ″, ≥, ×, ∝, ∂, •
    Some('\u{00B0}'), Some('\u{00B1}'), Some('\u{2033}'), Some('\u{2265}'),
    Some('\u{00D7}'), Some('\u{221D}'), Some('\u{2202}'), Some('\u{2022}'),
    // 0xB8..=0xBF: ÷, ≠, ≡, ≈, …, ⏐, ⎯, ↵
    Some('\u{00F7}'), Some('\u{2260}'), Some('\u{2261}'), Some('\u{2248}'),
    Some('\u{2026}'), Some('\u{23D0}'), Some('\u{23AF}'), Some('\u{21B5}'),
    // 0xC0..=0xC7: ℵ, ℑ, ℜ, ℘, ⊗, ⊕, ∅, ∩
    Some('\u{2135}'), Some('\u{2111}'), Some('\u{211C}'), Some('\u{2118}'),
    Some('\u{2297}'), Some('\u{2295}'), Some('\u{2205}'), Some('\u{2229}'),
    // 0xC8..=0xCF: ∪, ⊃, ⊇, ⊄, ⊂, ⊆, ∈, ∉
    Some('\u{222A}'), Some('\u{2283}'), Some('\u{2287}'), Some('\u{2284}'),
    Some('\u{2282}'), Some('\u{2286}'), Some('\u{2208}'), Some('\u{2209}'),
    // 0xD0..=0xD7: ∠, ∇, ®, ©, ™, ∏, √, ⋅
    Some('\u{2220}'), Some('\u{2207}'), Some('\u{00AE}'), Some('\u{00A9}'),
    Some('\u{2122}'), Some('\u{220F}'), Some('\u{221A}'), Some('\u{22C5}'),
    // 0xD8..=0xDF: ¬, ∧, ∨, ⇔, ⇐, ⇑, ⇒, ⇓
    Some('\u{00AC}'), Some('\u{2227}'), Some('\u{2228}'), Some('\u{21D4}'),
    Some('\u{21D0}'), Some('\u{21D1}'), Some('\u{21D2}'), Some('\u{21D3}'),
    // 0xE0..=0xE7: ◊, 〈, ® (sans), © (sans), ™ (sans), ∑, ⎛, ⎜
    Some('\u{25CA}'), Some('\u{2329}'), Some('\u{00AE}'), Some('\u{00A9}'),
    Some('\u{2122}'), Some('\u{2211}'), Some('\u{239B}'), Some('\u{239C}'),
    // 0xE8..=0xEF: ⎝, ⎡, ⎢, ⎣, ⎧, ⎨, ⎩, ⎪
    Some('\u{239D}'), Some('\u{23A1}'), Some('\u{23A2}'), Some('\u{23A3}'),
    Some('\u{23A7}'), Some('\u{23A8}'), Some('\u{23A9}'), Some('\u{23AA}'),
    // 0xF0..=0xF7: (unmapped), 〉, ∫, ⌠, ⎮, ⌡, ⎞, ⎟
    None, Some('\u{232A}'), Some('\u{222B}'), Some('\u{2320}'),
    Some('\u{23AE}'), Some('\u{2321}'), Some('\u{239E}'), Some('\u{239F}'),
    // 0xF8..=0xFF: ⎠, ⎤, ⎥, ⎦, ⎫, ⎬, ⎭, (unmapped)
    Some('\u{23A0}'), Some('\u{23A4}'), Some('\u{23A5}'), Some('\u{23A6}'),
    Some('\u{23AB}'), Some('\u{23AC}'), Some('\u{23AD}'), None,
];

/// Map a single Adobe Symbol-font byte (`0x00..=0xFF`) to its Unicode scalar per `SYMBOL.TXT`.
#[must_use]
pub(crate) fn map_symbol_byte(byte: u8) -> Option<char> {
    SYMBOL_TABLE.get(usize::from(byte)).copied().flatten()
}

/// Map a character inside a `<Font Face="Symbol">` run to Unicode.
///
/// Maps both direct single-byte code points (`U+0020..=U+00FE`) and Windows Symbol Private Use
/// Area code points (`U+F020..=U+F0FE`) through [`map_symbol_byte`], leaving already-Unicode
/// characters (such as `μ` or `α`) and unmapped control characters unchanged.
#[must_use]
pub(crate) fn map_symbol_char(ch: char) -> char {
    let cp = ch as u32;
    if cp <= 0xFF {
        map_symbol_byte(cp as u8).unwrap_or(ch)
    } else if (0xF020..=0xF0FE).contains(&cp) {
        map_symbol_byte((cp - 0xF000) as u8).unwrap_or(ch)
    } else {
        ch
    }
}

/// Decode a string authored in the legacy `"Symbol"` font into standard Unicode text.
#[must_use]
pub(crate) fn decode_symbol_font_text(input: &str) -> String {
    input.chars().map(map_symbol_char).collect()
}

/// Whether a `<Font Face="...">` font-family name identifies the legacy Adobe/OS `"Symbol"` font.
#[must_use]
pub(crate) fn is_symbol_font_face(face: &str) -> bool {
    let trimmed = face.trim();
    trimmed.eq_ignore_ascii_case("symbol")
        || trimmed.eq_ignore_ascii_case("symbolmt")
        || trimmed.eq_ignore_ascii_case("symbolps")
        || trimmed.eq_ignore_ascii_case("symbol mt")
}

/// Result of converting a `.pzfx` XML rich-text subtree into `curvebook-text` inline markup
/// alongside its unstyled Unicode plain-text fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConvertedRichText {
    /// Inline markup compatible with `curvebook_text::markup::parse_markup` (`<b>`, `<i>`,
    /// `<sup>`, `<sub>`, Unicode text decoded from `<Font Face="Symbol">`, `\n` for `<br/>`,
    /// and safe escaping of literal markup syntax triggers).
    pub(crate) markup: String,
    /// Unstyled Unicode plain-text fallback (Symbol font decoded to Unicode, `<br/>` converted
    /// to `\n`, formatting tags stripped, zero markup escapes).
    pub(crate) plain: String,
    /// Whether any active inline formatting tag (`<b>`, `<i>`, `<sup>`, `<sub>`) wrapped
    /// non-empty content.
    pub(crate) has_formatting: bool,
}

/// Convert a `.pzfx` XML rich-text element (such as `<Title>`, `<d>`, or `<Note>`) into both
/// `curvebook-text` inline markup and an unstyled Unicode plain-text fallback.
#[must_use]
pub(crate) fn convert_pzfx_rich_text(el: &XmlElement) -> ConvertedRichText {
    let mut star_count = 0_usize;
    let mut dollar_count = 0_usize;
    count_trigger_chars(el, false, 0, &mut star_count, &mut dollar_count);

    let mut conv = RichTextConverter { tokens: Vec::new(), plain: String::new(), has_formatting: false };
    let initial = FontContext::default();
    conv.visit_children(&el.children, initial, 0);

    let escape_stars = star_count >= 2;
    let escape_dollars = dollar_count >= 2;
    let mut markup = String::new();
    for (idx, token) in conv.tokens.iter().enumerate() {
        match token {
            RtToken::Open(tag) | RtToken::Close(tag) => markup.push_str(tag),
            RtToken::Text(text) => {
                let has_following =
                    conv.tokens.get(idx + 1..).is_some_and(|rest| rest.iter().any(|next| matches!(next, RtToken::Open(_) | RtToken::Text(_))));
                push_escaped_markup_text(&mut markup, text, escape_stars, escape_dollars, has_following);
            }
        }
    }

    ConvertedRichText { markup, plain: conv.plain, has_formatting: conv.has_formatting }
}

/// Convert a `.pzfx` XML rich-text element into `curvebook-text` inline markup when formatting
/// is present, or clean Unicode plain text otherwise.
#[must_use]
pub(crate) fn pzfx_element_to_markup(el: &XmlElement) -> String {
    let converted = convert_pzfx_rich_text(el);
    if converted.has_formatting { converted.markup } else { converted.plain }
}

/// Convert a `.pzfx` XML rich-text element into unstyled Unicode plain text (decoding
/// `<Font Face="Symbol">` runs and converting `<br/>` to `\n`).
#[must_use]
pub(crate) fn pzfx_element_to_plain_text(el: &XmlElement) -> String {
    convert_pzfx_rich_text(el).plain
}

#[derive(Debug, Clone, Copy, Default)]
struct FontContext {
    bold: bool,
    italic: bool,
    superscript: bool,
    subscript: bool,
    symbol_font: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RtToken {
    Open(&'static str),
    Close(&'static str),
    Text(String),
}

struct RichTextConverter {
    tokens: Vec<RtToken>,
    plain: String,
    has_formatting: bool,
}

impl RichTextConverter {
    fn visit_children(&mut self, children: &[XmlNode], ctx: FontContext, depth: usize) {
        if depth >= MAX_RICHTEXT_DEPTH {
            return;
        }
        for node in children {
            match node {
                XmlNode::Text(raw) => self.push_text(raw, ctx),
                XmlNode::Element(child) => self.visit_element(child, ctx, depth + 1),
            }
        }
    }

    fn visit_element(&mut self, el: &XmlElement, ctx: FontContext, depth: usize) {
        let local = el.local_name();
        if local.eq_ignore_ascii_case("br") {
            self.push_decoded_text("\n");
            return;
        }

        let mut next = ctx;
        let mut open_tags: Vec<&'static str> = Vec::new();
        let mut close_tags: Vec<&'static str> = Vec::new();

        if local.eq_ignore_ascii_case("b") || local.eq_ignore_ascii_case("strong") {
            if !next.bold {
                next.bold = true;
                open_tags.push("<b>");
                close_tags.push("</b>");
            }
        } else if local.eq_ignore_ascii_case("i") || local.eq_ignore_ascii_case("em") {
            if !next.italic {
                next.italic = true;
                open_tags.push("<i>");
                close_tags.push("</i>");
            }
        } else if local.eq_ignore_ascii_case("sup") {
            if !next.superscript {
                next.superscript = true;
                next.subscript = false;
                open_tags.push("<sup>");
                close_tags.push("</sup>");
            }
        } else if local.eq_ignore_ascii_case("sub") {
            if !next.subscript {
                next.subscript = true;
                next.superscript = false;
                open_tags.push("<sub>");
                close_tags.push("</sub>");
            }
        } else if local.eq_ignore_ascii_case("font") {
            apply_font_element_attrs(el, &mut next, &mut open_tags, &mut close_tags);
        }

        let open_pos = self.tokens.len();
        for tag in &open_tags {
            self.tokens.push(RtToken::Open(tag));
        }
        let content_start = self.tokens.len();

        self.visit_children(&el.children, next, depth);

        if !open_tags.is_empty() {
            if self.tokens.len() == content_start {
                self.tokens.truncate(open_pos);
            } else {
                self.has_formatting = true;
                for tag in close_tags.iter().rev() {
                    self.tokens.push(RtToken::Close(tag));
                }
            }
        }
    }

    fn push_text(&mut self, raw: &str, ctx: FontContext) {
        let normalized = normalize_line_endings(raw);
        let decoded = if ctx.symbol_font { decode_symbol_font_text(&normalized) } else { normalized };
        self.push_decoded_text(&decoded);
    }

    fn push_decoded_text(&mut self, decoded: &str) {
        if decoded.is_empty() {
            return;
        }
        self.plain.push_str(decoded);
        if let Some(RtToken::Text(prev)) = self.tokens.last_mut() {
            prev.push_str(decoded);
        } else {
            self.tokens.push(RtToken::Text(decoded.to_owned()));
        }
    }
}

fn apply_font_element_attrs(el: &XmlElement, ctx: &mut FontContext, open_tags: &mut Vec<&'static str>, close_tags: &mut Vec<&'static str>) {
    if let Some(face) = attr_ci(el, "face").or_else(|| attr_ci(el, "name")).or_else(|| attr_ci(el, "family")) {
        ctx.symbol_font = is_symbol_font_face(face);
    }
    if !ctx.bold && (attr_ci_is_truthy(el, "bold") || attr_ci(el, "weight").is_some_and(|w| w.trim().eq_ignore_ascii_case("bold"))) {
        ctx.bold = true;
        open_tags.push("<b>");
        close_tags.push("</b>");
    }
    if !ctx.italic && (attr_ci_is_truthy(el, "italic") || attr_ci(el, "style").is_some_and(|s| s.trim().eq_ignore_ascii_case("italic"))) {
        ctx.italic = true;
        open_tags.push("<i>");
        close_tags.push("</i>");
    }
    if (attr_ci_is_truthy(el, "super") || attr_ci_is_truthy(el, "superscript")) && !ctx.superscript {
        ctx.superscript = true;
        ctx.subscript = false;
        open_tags.push("<sup>");
        close_tags.push("</sup>");
    }
    if (attr_ci_is_truthy(el, "sub") || attr_ci_is_truthy(el, "subscript")) && !ctx.subscript {
        ctx.subscript = true;
        ctx.superscript = false;
        open_tags.push("<sub>");
        close_tags.push("</sub>");
    }
}

pub(crate) fn attr_ci<'a>(el: &'a XmlElement, key: &str) -> Option<&'a str> {
    el.attrs
        .iter()
        .find(|(k, _)| {
            let local = k.rsplit_once(':').map_or(k.as_str(), |(_, l)| l);
            local.eq_ignore_ascii_case(key)
        })
        .map(|(_, v)| v.as_str())
}

fn attr_ci_is_truthy(el: &XmlElement, key: &str) -> bool {
    attr_ci(el, key).is_some_and(|raw| {
        let t = raw.trim();
        t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
    })
}

fn count_trigger_chars(el: &XmlElement, symbol_font: bool, depth: usize, stars: &mut usize, dollars: &mut usize) {
    if depth >= MAX_RICHTEXT_DEPTH {
        return;
    }
    for node in &el.children {
        match node {
            XmlNode::Text(raw) => {
                for ch in raw.chars() {
                    let mapped = if symbol_font { map_symbol_char(ch) } else { ch };
                    if mapped == '*' {
                        *stars = stars.saturating_add(1);
                    } else if mapped == '$' {
                        *dollars = dollars.saturating_add(1);
                    }
                }
            }
            XmlNode::Element(child) => {
                let next_symbol = if child.local_name().eq_ignore_ascii_case("font") {
                    if let Some(face) = attr_ci(child, "face").or_else(|| attr_ci(child, "name")).or_else(|| attr_ci(child, "family")) {
                        is_symbol_font_face(face)
                    } else {
                        symbol_font
                    }
                } else {
                    symbol_font
                };
                count_trigger_chars(child, next_symbol, depth + 1, stars, dollars);
            }
        }
    }
}

fn normalize_line_endings(raw: &str) -> String {
    if !raw.contains('\r') {
        return raw.to_owned();
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out
}

fn push_escaped_markup_text(out: &mut String, text: &str, escape_stars: bool, escape_dollars: bool, has_following: bool) {
    for (idx, ch) in text.char_indices() {
        let rest = text.get(idx..).unwrap_or_default();
        let after = text.get(idx + ch.len_utf8()..).unwrap_or_default();
        let needs_escape = match ch {
            '\\' => true,
            '<' => starts_with_curvebook_markup_tag(rest),
            '&' => starts_with_curvebook_markup_entity(rest),
            '^' | '_' => is_script_trigger_after(after, has_following),
            '*' => escape_stars && is_star_emphasis_trigger_after(after, has_following),
            '$' => escape_dollars,
            _ => false,
        };
        if needs_escape {
            out.push('\\');
        }
        out.push(ch);
    }
}

fn starts_with_curvebook_markup_tag(rest: &str) -> bool {
    let Some(after) = rest.strip_prefix('<') else {
        return false;
    };
    let Some(close_idx) = after.as_bytes().iter().take(8).position(|&b| b == b'>') else {
        return false;
    };
    let Some(inner) = after.get(..close_idx) else {
        return false;
    };
    inner.eq_ignore_ascii_case("b")
        || inner.eq_ignore_ascii_case("/b")
        || inner.eq_ignore_ascii_case("i")
        || inner.eq_ignore_ascii_case("/i")
        || inner.eq_ignore_ascii_case("sup")
        || inner.eq_ignore_ascii_case("/sup")
        || inner.eq_ignore_ascii_case("sub")
        || inner.eq_ignore_ascii_case("/sub")
}

fn starts_with_curvebook_markup_entity(rest: &str) -> bool {
    let Some(after) = rest.strip_prefix('&') else {
        return false;
    };
    let Some(semi_idx) = after.as_bytes().iter().take(17).position(|&b| b == b';') else {
        return false;
    };
    if semi_idx == 0 || semi_idx > 16 {
        return false;
    }
    let Some(name) = after.get(..semi_idx) else {
        return false;
    };
    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())
    } else if let Some(dec) = name.strip_prefix('#') {
        !dec.is_empty() && dec.bytes().all(|b| b.is_ascii_digit())
    } else {
        name.bytes().all(|b| b.is_ascii_alphabetic())
    }
}

fn is_script_trigger_after(after: &str, has_following: bool) -> bool {
    let Some(first) = after.chars().next() else {
        return has_following;
    };
    if first == '{' || first == '\\' || first.is_alphanumeric() {
        return true;
    }
    if first == '-' || first == '\u{2212}' {
        return after.get(first.len_utf8()..).and_then(|s| s.chars().next()).is_some_and(char::is_alphanumeric);
    }
    false
}

fn is_star_emphasis_trigger_after(after: &str, has_following: bool) -> bool {
    let after_run = after.trim_start_matches('*');
    match after_run.chars().next() {
        Some(next) => !next.is_whitespace(),
        None => has_following,
    }
}

// ---------------------------------------------------------------------------
// CurveBook inline markup -> `.pzfx` XML rich-text writer
// ---------------------------------------------------------------------------

/// Tokenized segment for balanced CurveBook inline markup (`<b>`, `<i>`, `<sup>`, `<sub>`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkupToken {
    Text(String),
    Open(&'static str),
    Close(&'static str),
}

/// Write a `.pzfx` element (`<Title>` or `<d>`) carrying either plain text or balanced CurveBook
/// inline formatting tags (`<b>`, `<i>`, `<sup>`, `<sub>` -> `<B>`, `<I>`, `<SUP>`, `<SUB>`).
///
/// Plain strings without balanced inline tags are written verbatim without stripping backslashes.
pub(crate) fn write_pzfx_markup_element(writer: &mut XmlWriter, tag: &str, attrs: &[(&str, &str)], content: &str) -> Result<(), FormatError> {
    if let Some(tokens) = tokenize_balanced_inline_markup(content) {
        writer.start_element(tag, attrs)?;
        // Mark the container as mixed-content so pretty-printing never injects trailing
        // indentation before `</Title>` or `</d>` when the element contains only child tags
        // (e.g. `<Title><B>Control</B></Title>`).
        writer.text("")?;
        for token in &tokens {
            match token {
                MarkupToken::Text(t) => writer.text(t)?,
                MarkupToken::Open(name) => {
                    writer.start_inline_element(name, &[])?;
                    writer.text("")?;
                }
                MarkupToken::Close(_) => writer.end_element()?,
            }
        }
        writer.end_element()?;
        return Ok(());
    }
    writer.text_element(tag, attrs, content)
}

/// Write a `.pzfx` `<FloatingNote>` body using corpus-conformant `<Font>` + `<BR/>` line breaks
/// and optional `<B>` / `<I>` inline runs.
pub(crate) fn write_pzfx_floating_note_body(writer: &mut XmlWriter, body: &str) -> Result<(), FormatError> {
    if !body.contains('\n') && tokenize_balanced_inline_markup(body).is_none() {
        writer.start_inline_element("Font", &[])?;
        writer.text(body)?;
        writer.end_element()?;
        return Ok(());
    }
    writer.start_inline_element("Font", &[])?;
    writer.text("")?;
    let lines: Vec<&str> = body.split('\n').collect();
    for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
            writer.empty_inline_element("BR", &[])?;
        }
        if !line.is_empty() {
            writer.text(line)?;
        }
    }
    writer.end_element()
}

fn tokenize_balanced_inline_markup(input: &str) -> Option<Vec<MarkupToken>> {
    if !input.contains('<') {
        return None;
    }
    let mut tokens = Vec::new();
    let mut stack: Vec<&'static str> = Vec::new();
    let mut text_buf = String::new();
    let mut saw_tag = false;
    let bytes = input.as_bytes();
    let mut pos = 0_usize;

    while pos < bytes.len() {
        let rest = input.get(pos..)?;
        if let Some(after_slash) = rest.strip_prefix('\\')
            && let Some(next_ch) = after_slash.chars().next()
            && matches!(next_ch, '\\' | '<' | '&' | '^' | '_' | '*' | '$')
        {
            text_buf.push(next_ch);
            pos += 1 + next_ch.len_utf8();
            continue;
        }
        if let Some((tag_name, is_close, tag_len)) = parse_inline_tag_prefix(rest) {
            if !text_buf.is_empty() {
                tokens.push(MarkupToken::Text(std::mem::take(&mut text_buf)));
            }
            if is_close {
                let open = stack.pop()?;
                if open != tag_name {
                    return None;
                }
                tokens.push(MarkupToken::Close(tag_name));
            } else {
                stack.push(tag_name);
                tokens.push(MarkupToken::Open(tag_name));
                saw_tag = true;
            }
            pos += tag_len;
            continue;
        }
        let ch = rest.chars().next()?;
        text_buf.push(ch);
        pos += ch.len_utf8();
    }

    if !saw_tag || !stack.is_empty() {
        return None;
    }
    if !text_buf.is_empty() {
        tokens.push(MarkupToken::Text(text_buf));
    }
    Some(tokens)
}

fn parse_inline_tag_prefix(rest: &str) -> Option<(&'static str, bool, usize)> {
    let after = rest.strip_prefix('<')?;
    let close_idx = after.as_bytes().iter().take(8).position(|&b| b == b'>')?;
    let inner = after.get(..close_idx)?;
    let (is_close, raw_name) = match inner.strip_prefix('/') {
        Some(name) => (true, name),
        None => (false, inner),
    };
    let pzfx_tag = if raw_name.eq_ignore_ascii_case("b") {
        "B"
    } else if raw_name.eq_ignore_ascii_case("i") {
        "I"
    } else if raw_name.eq_ignore_ascii_case("sup") {
        "SUP"
    } else if raw_name.eq_ignore_ascii_case("sub") {
        "SUB"
    } else {
        return None;
    };
    Some((pzfx_tag, is_close, close_idx + 2))
}

// ---------------------------------------------------------------------------
// Strict ISO-8601 date parsing (no `chrono`, no calendar/leap-year math)
// ---------------------------------------------------------------------------

/// Validate a strict ISO-8601 extended calendar date (`YYYY-MM-DD`) using structural parsing and
/// static per-month day upper bounds (`Feb <= 29`, zero Gregorian leap-year arithmetic).
///
/// Rejects slash dates, non-padded dates (`2026-1-5`), timestamps (`2026-01-15T12:00:00Z`),
/// year `0000`, and out-of-range months or days.
#[must_use]
pub(crate) fn parse_strict_iso8601_date(raw: &str) -> Option<String> {
    let s = raw.trim();
    let bytes = s.as_bytes();
    if bytes.len() != 10 {
        return None;
    }
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let year = parse_ascii_digits_u16(bytes.get(0..4)?)?;
    let month = parse_ascii_digits_u8(bytes.get(5..7)?)?;
    let day = parse_ascii_digits_u8(bytes.get(8..10)?)?;

    if year == 0 || !(1..=12).contains(&month) {
        return None;
    }
    let max_day = static_max_day_for_month(month);
    if day == 0 || day > max_day {
        return None;
    }
    Some(s.to_owned())
}

/// Convert a strict ISO-8601 date string (`YYYY-MM-DD`) into [`ConstantValue::Date`].
#[must_use]
pub(crate) fn parse_pzfx_constant_date(raw: &str) -> Option<ConstantValue> {
    parse_strict_iso8601_date(raw).map(ConstantValue::Date)
}

const fn static_max_day_for_month(month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 29,
        _ => 0,
    }
}

fn parse_ascii_digits_u16(bytes: &[u8]) -> Option<u16> {
    let mut acc = 0_u16;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        acc = acc.checked_mul(10)?.checked_add(u16::from(b - b'0'))?;
    }
    Some(acc)
}

fn parse_ascii_digits_u8(bytes: &[u8]) -> Option<u8> {
    let mut acc = 0_u8;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        acc = acc.checked_mul(10)?.checked_add(b - b'0')?;
    }
    Some(acc)
}

// ---------------------------------------------------------------------------
// `.pzfx` Color / ColorTag / Highlighted -> Rgba8 parsing & mapping
// ---------------------------------------------------------------------------

/// Parse a `.pzfx` `Color` or `ColorTag` value into [`Rgba8`].
///
/// Supports `#RRGGBB`, `#RRGGBBAA`, `#RGB`, `0xRRGGBB`, `0xRRGGBBAA`, `rgb(r,g,b)`,
/// `rgba(r,g,b,a)`, and neutral W3C CSS / X11 colour names.
#[must_use]
pub(crate) fn parse_pzfx_color(raw: &str) -> Option<Rgba8> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(hex) = trimmed.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        return parse_hex_color(hex);
    }
    if let Some(rgba) = parse_rgb_functional(trimmed) {
        return Some(rgba);
    }

    let key: String = trimmed.chars().filter(|ch| ch.is_ascii_alphanumeric()).map(|ch| ch.to_ascii_lowercase()).collect();
    lookup_named_color(&key)
}

/// Extract an optional colour (`ColorTag` or `Color` attribute) from a `.pzfx` XML element.
#[must_use]
pub(crate) fn extract_pzfx_color_tag(el: &XmlElement) -> Option<Rgba8> {
    let raw = attr_ci(el, "colortag").or_else(|| attr_ci(el, "color"))?;
    parse_pzfx_color(raw)
}

/// Extract an optional navigator colour tag from a `.pzfx` table element, checking
/// `Highlighted` (`"1"`..`"7"` corpus palette index or direct colour string) before falling
/// back to `ColorTag` / `Color`.
#[must_use]
pub(crate) fn extract_pzfx_table_color_tag(el: &XmlElement) -> Option<Rgba8> {
    if let Some(raw_hl) = attr_ci(el, "highlighted").map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((_, rgba)) = HIGHLIGHT_PALETTE.iter().find(|(idx, _)| *idx == raw_hl) {
            return Some(*rgba);
        }
        if let Some(rgba) = parse_pzfx_color(raw_hl) {
            return Some(rgba);
        }
    }
    extract_pzfx_color_tag(el)
}

/// Map a CurveBook `color_tag` to a `.pzfx` `Table@Highlighted` index (`"1"`..`"7"`), returning
/// `(index_str, is_exact_match)`.
#[must_use]
pub(crate) fn color_to_pzfx_highlighted(color: Rgba8) -> (&'static str, bool) {
    if let Some((idx, _)) = HIGHLIGHT_PALETTE.iter().find(|(_, swatch)| *swatch == color) {
        return (*idx, true);
    }
    let mut best_idx = "1";
    let mut best_dist = u32::MAX;
    for &(idx, swatch) in &HIGHLIGHT_PALETTE {
        let dist = rgb_distance_sq(color, swatch);
        if dist < best_dist {
            best_dist = dist;
            best_idx = idx;
        }
    }
    (best_idx, false)
}

/// Map a [`FloatingNote::fill_color`] to one of the four corpus-observed `FloatingNote@Color`
/// names (`"Yellow"`, `"Blue"`, `"Gray"`, `"Red"`).
#[must_use]
pub(crate) fn color_to_pzfx_floating_note_name(color: Rgba8) -> &'static str {
    if color == FloatingNote::DEFAULT_FILL {
        return "Yellow";
    }
    let mut best_name = "Yellow";
    let mut best_dist = u32::MAX;
    for &(name, swatch) in &FLOATING_NOTE_COLORS {
        let dist = rgb_distance_sq(color, swatch);
        if dist < best_dist {
            best_dist = dist;
            best_name = name;
        }
    }
    best_name
}

fn rgb_distance_sq(a: Rgba8, b: Rgba8) -> u32 {
    let dr = i32::from(a.r) - i32::from(b.r);
    let dg = i32::from(a.g) - i32::from(b.g);
    let db = i32::from(a.b) - i32::from(b.b);
    (dr * dr + dg * dg + db * db) as u32
}

fn parse_hex_color(hex: &str) -> Option<Rgba8> {
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(hex.get(0..1)?, 16).ok()? * 17;
            let g = u8::from_str_radix(hex.get(1..2)?, 16).ok()? * 17;
            let b = u8::from_str_radix(hex.get(2..3)?, 16).ok()? * 17;
            Some(Rgba8::rgb(r, g, b))
        }
        6 | 8 => Rgba8::from_hex(hex),
        _ => None,
    }
}

fn parse_rgb_functional(trimmed: &str) -> Option<Rgba8> {
    let lower = trimmed.to_ascii_lowercase();
    if let Some(inner) = lower.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let mut parts = inner.split(',').map(|p| p.trim().parse::<u8>().ok());
        let (r, g, b) = (parts.next()??, parts.next()??, parts.next()??);
        if parts.next().is_some() {
            return None;
        }
        return Some(Rgba8::rgb(r, g, b));
    }
    if let Some(inner) = lower.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
        let mut parts = inner.split(',').map(|p| p.trim().parse::<u8>().ok());
        let (r, g, b, a) = (parts.next()??, parts.next()??, parts.next()??, parts.next()??);
        if parts.next().is_some() {
            return None;
        }
        return Some(Rgba8::rgba(r, g, b, a));
    }
    None
}

fn lookup_named_color(key: &str) -> Option<Rgba8> {
    match key {
        "" | "none" | "transparent" | "auto" | "clear" => None,
        "black" => Some(Rgba8::rgb(0x00, 0x00, 0x00)),
        "white" => Some(Rgba8::rgb(0xFF, 0xFF, 0xFF)),
        "red" => Some(Rgba8::rgb(0xFF, 0x00, 0x00)),
        "darkred" => Some(Rgba8::rgb(0x8B, 0x00, 0x00)),
        "crimson" => Some(Rgba8::rgb(0xDC, 0x14, 0x3C)),
        "coral" => Some(Rgba8::rgb(0xFF, 0x7F, 0x50)),
        "salmon" => Some(Rgba8::rgb(0xFA, 0x80, 0x72)),
        "lightred" => Some(Rgba8::rgb(0xFF, 0x66, 0x66)),
        "green" => Some(Rgba8::rgb(0x00, 0x80, 0x00)),
        "lime" => Some(Rgba8::rgb(0x00, 0xFF, 0x00)),
        "darkgreen" => Some(Rgba8::rgb(0x00, 0x64, 0x00)),
        "lightgreen" => Some(Rgba8::rgb(0x90, 0xEE, 0x90)),
        "blue" => Some(Rgba8::rgb(0x00, 0x00, 0xFF)),
        "darkblue" => Some(Rgba8::rgb(0x00, 0x00, 0x8B)),
        "navy" => Some(Rgba8::rgb(0x00, 0x00, 0x80)),
        "lightblue" => Some(Rgba8::rgb(0xAD, 0xD8, 0xE6)),
        "skyblue" => Some(Rgba8::rgb(0x87, 0xCE, 0xEB)),
        "yellow" => Some(Rgba8::rgb(0xFF, 0xFF, 0x00)),
        "gold" => Some(Rgba8::rgb(0xFF, 0xD7, 0x00)),
        "darkyellow" | "olive" => Some(Rgba8::rgb(0x80, 0x80, 0x00)),
        "orange" => Some(Rgba8::rgb(0xFF, 0xA5, 0x00)),
        "darkorange" => Some(Rgba8::rgb(0xFF, 0x8C, 0x00)),
        "purple" => Some(Rgba8::rgb(0x80, 0x00, 0x80)),
        "violet" => Some(Rgba8::rgb(0xEE, 0x82, 0xEE)),
        "indigo" => Some(Rgba8::rgb(0x4B, 0x00, 0x82)),
        "lavender" => Some(Rgba8::rgb(0xE6, 0xE6, 0xFA)),
        "magenta" | "fuchsia" => Some(Rgba8::rgb(0xFF, 0x00, 0xFF)),
        "pink" => Some(Rgba8::rgb(0xFF, 0xC0, 0xCB)),
        "cyan" | "aqua" => Some(Rgba8::rgb(0x00, 0xFF, 0xFF)),
        "teal" => Some(Rgba8::rgb(0x00, 0x80, 0x80)),
        "turquoise" => Some(Rgba8::rgb(0x40, 0xE0, 0xD0)),
        "brown" => Some(Rgba8::rgb(0xA5, 0x2A, 0x2A)),
        "maroon" => Some(Rgba8::rgb(0x80, 0x00, 0x00)),
        "tan" => Some(Rgba8::rgb(0xD2, 0xB4, 0x8C)),
        "gray" | "grey" => Some(Rgba8::rgb(0x80, 0x80, 0x80)),
        "darkgray" | "darkgrey" => Some(Rgba8::rgb(0xA9, 0xA9, 0xA9)),
        "dimgray" | "dimgrey" => Some(Rgba8::rgb(0x69, 0x69, 0x69)),
        "lightgray" | "lightgrey" => Some(Rgba8::rgb(0xD3, 0xD3, 0xD3)),
        "silver" => Some(Rgba8::rgb(0xC0, 0xC0, 0xC0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::ConstantValue;
    use curvebook_palette::Rgba8;

    use super::{
        ConvertedRichText, HIGHLIGHT_PALETTE, color_to_pzfx_highlighted, convert_pzfx_rich_text, decode_symbol_font_text, extract_pzfx_color_tag,
        extract_pzfx_table_color_tag, is_symbol_font_face, map_symbol_byte, map_symbol_char, parse_pzfx_color, parse_pzfx_constant_date,
        parse_strict_iso8601_date, pzfx_element_to_markup, pzfx_element_to_plain_text, write_pzfx_markup_element,
    };
    use crate::xml::{XmlWriter, parse_xml};

    #[test]
    fn symbol_table_maps_greek_math_arrows_and_delimiters_per_symbol_txt() {
        assert_eq!(decode_symbol_font_text("ABGDEZHQIKLMNXOPRSTUFJYW"), "ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦϑΨΩ");
        assert_eq!(decode_symbol_font_text("abgdezhqiklmnxoprstufcyw"), "αβγδεζηθικλμνξοπρστυφχψω");
        assert_eq!(decode_symbol_font_text("CVjv"), "Χςϕϖ");

        let checks = [
            (0x22_u8, '\u{2200}'),
            (0x24, '\u{2203}'),
            (0x2A, '\u{2217}'),
            (0x2D, '\u{2212}'),
            (0x40, '\u{2245}'),
            (0x5C, '\u{2234}'),
            (0x5E, '\u{22A5}'),
            (0x60, '\u{203E}'),
            (0x7E, '\u{223C}'),
            (0xA0, '\u{20AC}'),
            (0xA3, '\u{2264}'),
            (0xA5, '\u{221E}'),
            (0xAB, '\u{2194}'),
            (0xAE, '\u{2192}'),
            (0xB0, '\u{00B0}'),
            (0xB1, '\u{00B1}'),
            (0xB3, '\u{2265}'),
            (0xB4, '\u{00D7}'),
            (0xB6, '\u{2202}'),
            (0xB8, '\u{00F7}'),
            (0xB9, '\u{2260}'),
            (0xBA, '\u{2261}'),
            (0xBB, '\u{2248}'),
            (0xBC, '\u{2026}'),
            (0xC6, '\u{2205}'),
            (0xCE, '\u{2208}'),
            (0xD1, '\u{2207}'),
            (0xD5, '\u{220F}'),
            (0xD6, '\u{221A}'),
            (0xD7, '\u{22C5}'),
            (0xE5, '\u{2211}'),
            (0xF2, '\u{222B}'),
            (0xFE, '\u{23AD}'),
        ];
        for (byte, expected) in checks {
            assert_eq!(map_symbol_byte(byte), Some(expected), "byte 0x{byte:02X}");
            assert_eq!(map_symbol_char(char::from(byte)), expected);
            let pua = char::from_u32(0xF000 + u32::from(byte)).unwrap();
            assert_eq!(map_symbol_char(pua), expected, "Windows Symbol PUA U+{:04X}", pua as u32);
        }

        for unmapped in [0x00_u8, 0x0A, 0x1F, 0x7F, 0x80, 0x9F, 0xF0, 0xFF] {
            assert_eq!(map_symbol_byte(unmapped), None, "byte 0x{unmapped:02X} should be unmapped");
        }
        assert_eq!(map_symbol_char('μ'), 'μ');
        assert_eq!(map_symbol_char('Δ'), 'Δ');
        assert!(is_symbol_font_face("Symbol") && is_symbol_font_face("  symbolmt ") && !is_symbol_font_face("Arial"));
    }

    #[test]
    fn richtext_converter_handles_tags_nested_fonts_symbol_and_line_breaks() {
        let el = parse_xml(
            r##"<Title><b><i>K</i><sub>d</sub></b> = 10<sup>-6</sup> <Font Face="Symbol">m</Font>M <Font Face="Symbol"><b>a</b> + <Font Face="Arial">x</Font> + <Font Color="#FF0000">b</Font></Font><br/>Line 2</Title>"##,
        )
        .unwrap();
        let res = convert_pzfx_rich_text(&el);
        assert_eq!(
            res,
            ConvertedRichText {
                markup: String::from("<b><i>K</i><sub>d</sub></b> = 10<sup>-6</sup> μM <b>α</b> + x + β\nLine 2"),
                plain: String::from("Kd = 10-6 μM α + x + β\nLine 2"),
                has_formatting: true,
            }
        );
        assert_eq!(pzfx_element_to_markup(&el), res.markup);
        assert_eq!(pzfx_element_to_plain_text(&el), res.plain);

        let font_styled =
            parse_xml(r#"<Title><Font Face="Symbol" Bold="1">D</Font><Font Sub="1">max</Font><b></b><i><i>it</i></i></Title>"#).unwrap();
        let styled_res = convert_pzfx_rich_text(&font_styled);
        assert_eq!(styled_res.markup, "<b>Δ</b><sub>max</sub><i>it</i>");
        assert_eq!(styled_res.plain, "Δmaxit");
        assert!(styled_res.has_formatting);

        let sym_only = parse_xml(r#"<Title><Font Face="Symbol">m</Font>M</Title>"#).unwrap();
        let sym_res = convert_pzfx_rich_text(&sym_only);
        assert_eq!(sym_res.plain, "μM");
        assert_eq!(sym_res.markup, "μM");
        assert!(!sym_res.has_formatting);
        assert_eq!(pzfx_element_to_markup(&sym_only), "μM");
    }

    #[test]
    fn richtext_converter_escapes_triggers_and_roundtrips_through_xml_writer() {
        let samples = [
            "<b><i>K</i><sub>d</sub></b> = 10<sup>-6</sup> μM",
            "Dose Response (10<sup>-6</sup> M)",
            "Plain_title *no* tags",
            r"<b>Dose\_1</b> \*star* \$x\$",
        ];
        for sample in samples {
            let mut w = XmlWriter::new();
            write_pzfx_markup_element(&mut w, "Title", &[], sample).unwrap();
            let xml = w.finish().unwrap();
            let el = parse_xml(&xml).unwrap();
            assert_eq!(pzfx_element_to_markup(&el), sample, "round-trip failed for `{sample}` via `{xml}`");
        }

        let el = parse_xml(r#"<Title><b>Dose_1</b> &lt;b&gt;lit&lt;/b&gt; &amp;alpha; \frac{1}{2} *star* $x$ 10^6</Title>"#).unwrap();
        let res = convert_pzfx_rich_text(&el);
        assert_eq!(res.plain, r"Dose_1 <b>lit</b> &alpha; \frac{1}{2} *star* $x$ 10^6");
        assert_eq!(res.markup, r"<b>Dose\_1</b> \<b>lit\</b> \&alpha; \\frac{1}{2} \*star* \$x\$ 10\^6");

        let multi_star = parse_xml(r#"<Title><b>***a***</b> **bold** *other* **bold**a *other* a**<i>b</i>**c</Title>"#).unwrap();
        let ms = convert_pzfx_rich_text(&multi_star);
        assert_eq!(ms.markup, r"<b>\*\*\*a\*\*\*</b> \*\*bold** \*other* \*\*bold\*\*a \*other* a\*\*<i>b</i>\*\*c");

        let end_star = parse_xml(r#"<Title>***a***</Title>"#).unwrap();
        assert_eq!(convert_pzfx_rich_text(&end_star).markup, r"\*\*\*a***");
    }

    #[test]
    fn strict_iso8601_date_parser_validates_structure_without_calendar_arithmetic() {
        for valid in ["2026-01-15", "2024-02-29", "2025-02-29", "1999-12-31", "0001-04-30"] {
            assert_eq!(parse_strict_iso8601_date(valid), Some(String::from(valid)), "{valid}");
            assert_eq!(parse_pzfx_constant_date(valid), Some(ConstantValue::Date(String::from(valid))), "{valid}");
        }
        assert_eq!(parse_strict_iso8601_date("  2026-05-09 \n"), Some(String::from("2026-05-09")));

        for invalid in [
            "",
            "2026-1-5",
            "2026/01/15",
            "01-15-2026",
            "2026-01-15T12:00:00Z",
            "0000-01-01",
            "2026-00-10",
            "2026-13-01",
            "2026-01-00",
            "2026-02-30",
            "2026-04-31",
            "2026-06-31",
            "2026-09-31",
            "2026-11-31",
            "2026-12-32",
            "abcd-01-01",
        ] {
            assert_eq!(parse_strict_iso8601_date(invalid), None, "expected None for `{invalid}`");
        }
    }

    #[test]
    fn pzfx_color_and_highlight_palette_roundtrip() {
        assert_eq!(parse_pzfx_color("#1A2B3C"), Some(Rgba8::rgb(0x1A, 0x2B, 0x3C)));
        assert_eq!(parse_pzfx_color("#1A2B3C80"), Some(Rgba8::rgba(0x1A, 0x2B, 0x3C, 0x80)));
        assert_eq!(parse_pzfx_color("#F0A"), Some(Rgba8::rgb(0xFF, 0x00, 0xAA)));
        assert_eq!(parse_pzfx_color("0x336699"), Some(Rgba8::rgb(0x33, 0x66, 0x99)));
        assert_eq!(parse_pzfx_color("rgb(12, 34, 56)"), Some(Rgba8::rgb(12, 34, 56)));
        assert_eq!(parse_pzfx_color("Yellow"), Some(Rgba8::rgb(0xFF, 0xFF, 0x00)));
        assert_eq!(parse_pzfx_color("none"), None);

        let note_el = parse_xml(r#"<FloatingNote Color="Blue">Note</FloatingNote>"#).unwrap();
        assert_eq!(extract_pzfx_color_tag(&note_el), Some(Rgba8::rgb(0x00, 0x00, 0xFF)));

        for &(idx, swatch) in &HIGHLIGHT_PALETTE {
            let tbl = parse_xml(&format!(r#"<Table Highlighted="{idx}"/>"#)).unwrap();
            assert_eq!(extract_pzfx_table_color_tag(&tbl), Some(swatch));
            assert_eq!(color_to_pzfx_highlighted(swatch), (idx, true));
        }
    }
}
