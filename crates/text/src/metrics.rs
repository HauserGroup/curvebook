//! Deterministic PostScript-point text layout metrics (`1 pt = 1/72 inch`).
//!
//! Provides clean-room Adobe Core-14 Helvetica (Regular/Bold/Oblique/BoldOblique) and Symbol
//! advance-width tables when `BUNDLED_FONTS` is empty, plus a safe zero-panic SFNT/TrueType
//! (`head`/`hhea`/`maxp`/`cmap`/`hmtx`/`loca`/`glyf`) metric and glyph-slice reader when bundled
//! font faces are active.

use std::sync::OnceLock;

use read_fonts::tables::cmap::{Cmap, Cmap0, Cmap4, Cmap12, CmapIterLimits, CmapSubtable, PlatformId};
use read_fonts::tables::head::MacStyle;
use read_fonts::tables::name::Name;
use read_fonts::tables::os2::{Os2, SelectionFlags};
use read_fonts::tables::post::Post;
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{CollectionRef, FontData, FontRead, FontRef, ReadError, TTCHeader, TableProvider};
use serde::{Deserialize, Serialize};

use crate::BUNDLED_FONTS;
use crate::markup::{RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, TextDirection, contains_rtl_script, shape_bidi_runs};

/// Bounding box and vertical font metrics measured in PostScript points (`pt`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct TextMetricsPt {
    pub width_pt: f64,
    pub ascent_pt: f64,
    pub descent_pt: f64,
    pub line_height_pt: f64,
}

impl TextMetricsPt {
    /// Total typographic box height (`ascent_pt + descent_pt`).
    #[must_use]
    pub const fn height_pt(self) -> f64 {
        self.ascent_pt + self.descent_pt
    }
}

/// Effective scale factor for a [`RichSpan`], preventing double-scaling when `scale` is already
/// pre-set to a script factor (e.g. `0.7` from [`crate::parse_markup`]) while still applying `0.7`
/// when a caller constructs a `Superscript`/`Subscript` span with default `scale = 1.0`.
#[must_use]
pub fn effective_span_scale(span: &RichSpan) -> f64 {
    let raw = if span.scale.is_finite() && span.scale > 0.0 { span.scale } else { 1.0 };
    match span.script {
        SpanScript::Baseline => raw,
        SpanScript::Superscript | SpanScript::Subscript => {
            if (raw - 1.0).abs() <= 1e-9 {
                0.7
            } else {
                raw
            }
        }
    }
}

/// Baseline Y-offset in Y-down PostScript points (`negative` shifts upward, `positive` downward).
#[must_use]
pub const fn script_baseline_offset_pt(script: SpanScript, base_size_pt: f64) -> f64 {
    match script {
        SpanScript::Baseline => 0.0,
        SpanScript::Superscript => -0.35 * base_size_pt,
        SpanScript::Subscript => 0.20 * base_size_pt,
    }
}

/// Measure deterministic typographic bounds in PostScript points for `rich` at `font_size_pt`.
///
/// Accounts for per-codepoint Core-14 Helvetica/Symbol widths (or bundled TrueType `hmtx` widths
/// when `BUNDLED_FONTS` is populated), bold/italic style, superscript/subscript scales, and
/// baseline vertical offsets.
#[must_use]
pub fn measure_text_pt(rich: &RichText, font_size_pt: f64) -> TextMetricsPt {
    let base_size = if font_size_pt.is_finite() && font_size_pt > 0.0 { font_size_pt } else { 0.0 };
    if base_size == 0.0 || rich.is_empty() {
        return TextMetricsPt::default();
    }

    let mut current_line_width = 0.0_f64;
    let mut max_line_width = 0.0_f64;
    let mut line_count = 1usize;
    let mut max_ascent = 0.0_f64;
    let mut max_descent = 0.0_f64;
    let mut has_visible_chars = false;

    for span in &rich.spans {
        if span.text.is_empty() {
            continue;
        }
        let scale = effective_span_scale(span);
        let glyph_size = base_size * scale;
        let y_offset = script_baseline_offset_pt(span.script, base_size);
        let bundled = select_bundled_font(span.weight, span.slope);
        let (asc_em, desc_em) = font_vertical_em(bundled);
        max_ascent = max_ascent.max(-y_offset + asc_em * glyph_size);
        max_descent = max_descent.max(y_offset + desc_em * glyph_size);

        for ch in span.text.chars() {
            if ch == '\n' {
                max_line_width = max_line_width.max(current_line_width);
                current_line_width = 0.0;
                line_count += 1;
                continue;
            }
            if ch == '\r' {
                continue;
            }
            has_visible_chars = true;
            let adv_em = char_advance_em_with_font(ch, span.weight, span.slope, bundled);
            current_line_width += glyph_size * adv_em;
        }
    }

    max_line_width = max_line_width.max(current_line_width);
    if !has_visible_chars && line_count == 1 {
        return TextMetricsPt::default();
    }
    if max_ascent == 0.0 && max_descent == 0.0 {
        max_ascent = base_size * 0.80;
        max_descent = base_size * 0.20;
    }

    let single_line_height = (base_size * 1.20).max(max_ascent + max_descent);
    let extra_lines = line_count.saturating_sub(1) as f64;
    let total_descent = max_descent + extra_lines * single_line_height;

    TextMetricsPt { width_pt: max_line_width, ascent_pt: max_ascent, descent_pt: total_descent, line_height_pt: single_line_height }
}

/// Measure advance width in PostScript points for a single character under a span style.
#[must_use]
pub fn char_advance_pt(ch: char, font_size_pt: f64, weight: SpanWeight, slope: SpanSlope) -> f64 {
    if !font_size_pt.is_finite() || font_size_pt <= 0.0 {
        return 0.0;
    }
    let bundled = select_bundled_font(weight, slope);
    font_size_pt * char_advance_em_with_font(ch, weight, slope, bundled)
}

/// Returns `true` if `ch` is a Unicode combining diacritical mark with zero horizontal advance.
#[must_use]
pub const fn is_combining_mark(ch: char) -> bool {
    matches!(ch as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

/// Split `rich` into individual lines separated by `\n` (ignoring `\r`), preserving per-span styling.
#[must_use]
pub fn split_rich_text_lines(rich: &RichText) -> Vec<RichText> {
    if rich.spans.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<Vec<RichSpan>> = vec![Vec::new()];
    for span in &rich.spans {
        for (idx, part) in span.text.split('\n').enumerate() {
            if idx > 0 {
                lines.push(Vec::new());
            }
            let cleaned = if part.contains('\r') { part.replace('\r', "") } else { part.to_owned() };
            if !cleaned.is_empty()
                && let Some(cur) = lines.last_mut()
            {
                cur.push(RichSpan { text: cleaned, ..span.clone() });
            }
        }
    }
    lines.into_iter().map(RichText::new).collect()
}

/// Returns `true` if any span in `rich` contains an RTL script character.
#[must_use]
pub fn rich_contains_rtl(rich: &RichText) -> bool {
    rich.spans.iter().any(|span| contains_rtl_script(&span.text))
}

fn infer_base_direction(rich: &RichText) -> TextDirection {
    let mut buf = [0u8; 4];
    for span in &rich.spans {
        for ch in span.text.chars() {
            if contains_rtl_script(ch.encode_utf8(&mut buf)) {
                return TextDirection::Rtl;
            }
            if ch.is_alphabetic() {
                return TextDirection::Ltr;
            }
        }
    }
    TextDirection::Ltr
}

fn consume_chars_prefix(text: &str, cursor: &mut usize, char_count: usize) -> String {
    let start = *cursor;
    let mut end = start;
    for _ in 0..char_count {
        let Some(ch) = text.get(end..).and_then(|s| s.chars().next()) else {
            break;
        };
        end += ch.len_utf8();
    }
    *cursor = end;
    text.get(start..end).unwrap_or_default().to_owned()
}

/// Reorder `rich` into visual left-to-right span order while preserving span styles and character
/// count, inferring the paragraph base direction from the first strong character.
#[must_use]
pub fn reorder_rich_text_bidi(rich: &RichText) -> RichText {
    reorder_rich_text_bidi_with_direction(rich, infer_base_direction(rich))
}

/// Reorder `rich` into visual left-to-right span order under an explicit paragraph `base_dir`
/// while preserving span styles and character count across multi-span runs.
#[must_use]
pub fn reorder_rich_text_bidi_with_direction(rich: &RichText, base_dir: TextDirection) -> RichText {
    if rich.spans.len() == 1
        && let Some(span) = rich.spans.first()
    {
        let visual_spans = shape_bidi_runs(&span.text, base_dir).into_iter().map(|run| RichSpan { text: run.text, ..span.clone() }).collect();
        return RichText::new(visual_spans);
    }

    let mut plain = String::new();
    let mut span_ranges = Vec::with_capacity(rich.spans.len());
    for span in &rich.spans {
        if span.text.is_empty() {
            continue;
        }
        let start = plain.len();
        plain.push_str(&span.text);
        span_ranges.push((start, plain.len(), span));
    }
    if plain.is_empty() {
        return rich.clone();
    }

    let mut visual_spans = Vec::new();
    for run in shape_bidi_runs(&plain, base_dir) {
        if let Some(&(_, _, single_span)) = span_ranges.iter().find(|&&(s, e, _)| s <= run.start_byte && run.end_byte <= e) {
            visual_spans.push(RichSpan { text: run.text, ..single_span.clone() });
            continue;
        }
        let mut cursor = 0usize;
        let push_slice = |s_start: usize, s_end: usize, span: &RichSpan, cursor: &mut usize, dst: &mut Vec<RichSpan>| {
            let o_start = run.start_byte.max(s_start);
            let o_end = run.end_byte.min(s_end);
            if o_start < o_end
                && let Some(slice) = plain.get(o_start..o_end)
            {
                let piece = consume_chars_prefix(&run.text, cursor, slice.chars().count());
                if !piece.is_empty() {
                    dst.push(RichSpan { text: piece, ..span.clone() });
                }
            }
        };
        match run.direction {
            TextDirection::Ltr => {
                for &(s_start, s_end, span) in &span_ranges {
                    push_slice(s_start, s_end, span, &mut cursor, &mut visual_spans);
                }
            }
            TextDirection::Rtl => {
                for &(s_start, s_end, span) in span_ranges.iter().rev() {
                    push_slice(s_start, s_end, span, &mut cursor, &mut visual_spans);
                }
            }
        }
        if cursor < run.text.len()
            && let Some(tail) = run.text.get(cursor..)
            && let Some(last) = visual_spans.last_mut()
        {
            last.text.push_str(tail);
        }
    }
    RichText::new(visual_spans)
}

/// Clean-room Adobe Core-14 Helvetica (Regular/Bold/Oblique/BoldOblique) and Symbol advance
/// width for Unicode codepoint `ch`, normalized in `em` units (`width_1000 / 1000.0`).
#[must_use]
pub fn core14_advance_em(ch: char, weight: SpanWeight, _slope: SpanSlope) -> f64 {
    if is_combining_mark(ch) {
        return 0.0;
    }
    if let Some(w) = core14_ascii_width_1000(ch, weight) {
        return f64::from(w) / 1000.0;
    }
    if let Some(w) = core14_accented_latin_width_1000(ch, weight) {
        return f64::from(w) / 1000.0;
    }
    if let Some(w) = core14_cyrillic_width_1000(ch, weight) {
        return f64::from(w) / 1000.0;
    }
    if let Some(w) = core14_symbol_width_1000(ch) {
        return f64::from(w) / 1000.0;
    }
    if is_wide_cjk(ch) {
        return 1.0;
    }
    match ch {
        '\n' | '\r' => 0.0,
        '\t' => 4.0 * 0.278,
        '\u{00A0}' => 0.278,
        _ => 0.556,
    }
}

pub(crate) fn char_advance_em_with_font(ch: char, weight: SpanWeight, slope: SpanSlope, bundled: Option<&TrueTypeFont<'_>>) -> f64 {
    if matches!(ch, '\n' | '\r') {
        return 0.0;
    }
    if ch == '\t' {
        return 4.0 * char_advance_em_with_font(' ', weight, slope, bundled);
    }
    if let Some(ttf) = bundled
        && let Some(adv) = ttf.advance_em(ch)
    {
        return adv;
    }
    core14_advance_em(ch, weight, slope)
}

fn font_vertical_em(bundled: Option<&TrueTypeFont<'_>>) -> (f64, f64) {
    if let Some(ttf) = bundled {
        let upem = f64::from(ttf.units_per_em());
        if upem > 0.0 {
            let asc = (f64::from(ttf.ascent()) / upem).clamp(0.5, 1.2);
            let desc = (f64::from(-ttf.descent()) / upem).clamp(0.1, 0.6);
            return (asc, desc);
        }
    }
    (0.80, 0.20)
}

fn core14_ascii_width_1000(ch: char, weight: SpanWeight) -> Option<u16> {
    if !ch.is_ascii() || ch.is_ascii_control() {
        return None;
    }
    let idx = (ch as usize).checked_sub(0x20)?;
    let table = match weight {
        SpanWeight::Regular => &HELVETICA_REGULAR_ASCII_WIDTHS,
        SpanWeight::Bold => &HELVETICA_BOLD_ASCII_WIDTHS,
    };
    table.get(idx).copied()
}

/// Core-14 Helvetica Regular widths for ASCII `0x20..=0x7E` in 1/1000 em units.
/// (Helvetica Oblique shares identical advance widths.)
const HELVETICA_REGULAR_ASCII_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // 0x20..0x2F: ' '..'/'
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // 0x30..0x3F: '0'..'?'
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // 0x40..0x4F: '@'..'O'
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // 0x50..0x5F: 'P'..'_'
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // 0x60..0x6F: '`'..'o'
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, // 0x70..0x7E: 'p'..'~'
];

/// Core-14 Helvetica Bold widths for ASCII `0x20..=0x7E` in 1/1000 em units.
/// (Helvetica BoldOblique shares identical advance widths.)
const HELVETICA_BOLD_ASCII_WIDTHS: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, // 0x20..0x2F
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, // 0x30..0x3F
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, // 0x40..0x4F
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, // 0x50..0x5F
    333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, // 0x60..0x6F
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584, // 0x70..0x7E
];

fn core14_symbol_width_1000(ch: char) -> Option<u16> {
    match ch {
        // Greek Uppercase (Adobe Symbol)
        '\u{0391}' => Some(722), // Alpha
        '\u{0392}' => Some(667), // Beta
        '\u{0393}' => Some(603), // Gamma
        '\u{0394}' => Some(612), // Delta
        '\u{0395}' => Some(611), // Epsilon
        '\u{0396}' => Some(611), // Zeta
        '\u{0397}' => Some(722), // Eta
        '\u{0398}' => Some(741), // Theta
        '\u{0399}' => Some(333), // Iota
        '\u{039A}' => Some(722), // Kappa
        '\u{039B}' => Some(686), // Lambda
        '\u{039C}' => Some(889), // Mu
        '\u{039D}' => Some(722), // Nu
        '\u{039E}' => Some(645), // Xi
        '\u{039F}' => Some(722), // Omicron
        '\u{03A0}' => Some(768), // Pi
        '\u{03A1}' => Some(556), // Rho
        '\u{03A3}' => Some(592), // Sigma
        '\u{03A4}' => Some(611), // Tau
        '\u{03A5}' => Some(690), // Upsilon
        '\u{03A6}' => Some(763), // Phi
        '\u{03A7}' => Some(722), // Chi
        '\u{03A8}' => Some(795), // Psi
        '\u{03A9}' => Some(768), // Omega

        // Greek Lowercase + variants (Adobe Symbol)
        '\u{03B1}' => Some(631),              // alpha
        '\u{03B2}' => Some(549),              // beta
        '\u{03B3}' => Some(411),              // gamma
        '\u{03B4}' => Some(494),              // delta
        '\u{03B5}' | '\u{03F5}' => Some(439), // epsilon / varepsilon
        '\u{03B6}' => Some(494),              // zeta
        '\u{03B7}' => Some(603),              // eta
        '\u{03B8}' => Some(521),              // theta
        '\u{03D1}' => Some(603),              // vartheta
        '\u{03B9}' => Some(329),              // iota
        '\u{03BA}' | '\u{03F0}' => Some(549), // kappa / varkappa
        '\u{03BB}' => Some(549),              // lambda
        '\u{03BC}' | '\u{00B5}' => Some(576), // mu / micro
        '\u{03BD}' => Some(521),              // nu
        '\u{03BE}' => Some(493),              // xi
        '\u{03BF}' => Some(549),              // omicron
        '\u{03C0}' => Some(549),              // pi
        '\u{03D6}' => Some(713),              // varpi
        '\u{03C1}' | '\u{03F1}' => Some(549), // rho / varrho
        '\u{03C2}' => Some(439),              // final sigma
        '\u{03C3}' => Some(603),              // sigma
        '\u{03C4}' => Some(439),              // tau
        '\u{03C5}' => Some(576),              // upsilon
        '\u{03C6}' => Some(521),              // phi
        '\u{03D5}' => Some(603),              // varphi
        '\u{03C7}' => Some(549),              // chi
        '\u{03C8}' => Some(686),              // psi
        '\u{03C9}' => Some(686),              // omega

        // Scientific, typographic & mathematical operators
        '\u{00B0}' | '\u{2218}' => Some(400), // degree / ring
        '\u{00B1}' | '\u{2213}' => Some(549), // plus-minus / minus-plus
        '\u{00B2}'
        | '\u{00B3}'
        | '\u{00B9}'
        | '\u{2070}'
        | '\u{2074}'..='\u{2079}'
        | '\u{207F}'
        | '\u{2080}'..='\u{2089}'
        | '\u{1D63}'..='\u{1D65}'
        | '\u{2090}'..='\u{209C}' => Some(333),
        '\u{207A}' | '\u{207B}' | '\u{207C}' | '\u{208A}' | '\u{208B}' | '\u{208C}' => Some(350),
        '\u{1D62}' | '\u{2071}' | '\u{207D}' | '\u{207E}' | '\u{208D}' | '\u{208E}' => Some(240),
        '\u{00B7}' => Some(250),                                         // middle dot
        '\u{00C5}' => Some(667),                                         // Angstrom A-ring
        '\u{00D7}' | '\u{00F7}' => Some(549),                            // times / divide
        '\u{2013}' | '\u{2500}' => Some(556),                            // en-dash / box horizontal
        '\u{2014}' | '\u{2026}' | '\u{2030}' | '\u{2122}' => Some(1000), // em-dash / ellipsis / permil / TM
        '\u{2018}' | '\u{2019}' => Some(222),                            // single quotes
        '\u{201C}' | '\u{201D}' | '\u{2039}' | '\u{203A}' => Some(333),  // double quotes / single guillemets
        '\u{00AB}' | '\u{00BB}' => Some(556),                            // double guillemets
        '\u{2020}' | '\u{2021}' | '\u{00A7}' => Some(556),               // dagger / double dagger / section
        '\u{2022}' => Some(350),                                         // bullet
        '\u{2032}' => Some(247),                                         // prime
        '\u{2033}' => Some(411),                                         // double prime
        '\u{00A9}' | '\u{00AE}' => Some(737),                            // copyright / registered
        '\u{2190}' | '\u{2192}' => Some(987),                            // left/right arrow
        '\u{2191}' | '\u{2193}' => Some(603),                            // up/down arrow
        '\u{2194}' => Some(1042),                                        // left-right arrow
        '\u{2200}' | '\u{2207}' | '\u{2208}' | '\u{2209}' | '\u{2211}' | '\u{221D}' | '\u{221E}' | '\u{226A}' | '\u{226B}' | '\u{2282}'
        | '\u{2286}' => Some(713),
        '\u{2202}' => Some(494), // partial
        '\u{2203}' | '\u{2212}' | '\u{221A}' | '\u{223C}' | '\u{2248}' | '\u{2260}' | '\u{2261}' | '\u{2264}' | '\u{2265}' => Some(549),
        '\u{2205}' | '\u{220F}' => Some(823), // empty set / product
        '\u{2229}' | '\u{222A}' => Some(768), // intersection / union
        '\u{222B}' => Some(274),              // integral
        '\u{2234}' => Some(863),              // therefore
        '\u{25B6}' | '\u{25B8}' | '\u{25BC}' | '\u{25BE}' => Some(556),
        _ => None,
    }
}

fn core14_accented_latin_width_1000(ch: char, weight: SpanWeight) -> Option<u16> {
    let base = match ch {
        'À'..='Å' | 'Ā' | 'Ă' | 'Ą' => 'A',
        'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => 'C',
        'Ď' | 'Đ' | 'Ð' => 'D',
        'È'..='Ë' | 'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => 'E',
        'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => 'G',
        'Ĥ' | 'Ħ' => 'H',
        'Ì'..='Ï' | 'Ĩ' | 'Ī' | 'Ĭ' | 'Į' | 'İ' => 'I',
        'Ĵ' => 'J',
        'Ķ' => 'K',
        'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => 'L',
        'Ñ' | 'Ń' | 'Ņ' | 'Ň' | 'Ŋ' => 'N',
        'Ò'..='Ö' | 'Ø' | 'Ō' | 'Ŏ' | 'Ő' => 'O',
        'Ŕ' | 'Ŗ' | 'Ř' => 'R',
        'Ś' | 'Ŝ' | 'Ş' | 'Š' => 'S',
        'Ţ' | 'Ť' | 'Ŧ' => 'T',
        'Ù'..='Ü' | 'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => 'U',
        'Ŵ' => 'W',
        'Ý' | 'Ŷ' | 'Ÿ' => 'Y',
        'Ź' | 'Ż' | 'Ž' => 'Z',
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' | 'ð' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' | '\u{0237}' => 'j',
        'ķ' | 'ĸ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' | 'ſ' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŋ' => 'n',
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        'Æ' | 'Œ' | 'Ĳ' => return Some(if ch == 'Ĳ' { 722 } else { 1000 }),
        'æ' | 'œ' | 'ß' => return Some(if ch == 'ß' { 611 } else { 889 }),
        _ => return None,
    };
    core14_ascii_width_1000(base, weight)
}

fn core14_cyrillic_width_1000(ch: char, weight: SpanWeight) -> Option<u16> {
    let base = match ch {
        'А' => 'A',
        'Б' | 'В' | 'Г' | 'Е' | 'Ё' | 'З' | 'Р' | 'С' | 'Т' | 'У' | 'Ь' | 'Э' => 'E',
        'Д' | 'Ж' | 'М' | 'Ф' | 'Ы' | 'Ю' => return Some(if matches!(ch, 'Ж' | 'М' | 'Ю') { 889 } else { 778 }),
        'Ш' | 'Щ' => return Some(944),
        'И' | 'Й' | 'К' | 'Л' | 'Н' | 'О' | 'П' | 'Х' | 'Ц' | 'Ч' | 'Ъ' | 'Я' => 'H',
        'а' | 'б' | 'в' | 'г' | 'д' | 'е' | 'ё' | 'з' | 'и' | 'й' | 'к' | 'л' | 'н' | 'о' | 'п' | 'р' | 'с' | 'т' | 'у' | 'х' | 'ц' | 'ч' | 'ъ'
        | 'ь' | 'э' | 'я' => 'a',
        'ж' | 'м' | 'ф' | 'ш' | 'щ' | 'ы' | 'ю' => return Some(if matches!(ch, 'ш' | 'щ') { 833 } else { 778 }),
        _ => return None,
    };
    core14_ascii_width_1000(base, weight)
}

const fn is_wide_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF01..=0xFF60
    )
}

/// Safe, zero-allocation SFNT/TrueType font metric & glyph slice reader backed by `read-fonts`.
#[derive(Clone)]
pub struct TrueTypeFont<'a> {
    font: FontRef<'a>,
    units_per_em: u16,
    x_min: i16,
    y_min: i16,
    x_max: i16,
    y_max: i16,
    mac_style: MacStyle,
    index_to_loc_format: i16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    num_glyphs: u16,
    cmap_subtables: CmapSubtables,
    has_glyf_outlines: bool,
}

impl std::fmt::Debug for TrueTypeFont<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrueTypeFont")
            .field("units_per_em", &self.units_per_em)
            .field("num_glyphs", &self.num_glyphs)
            .field("index_to_loc_format", &self.index_to_loc_format)
            .finish_non_exhaustive()
    }
}

impl<'a> TrueTypeFont<'a> {
    /// Parse an SFNT/TrueType font byte slice (`head`, `hhea`, `maxp`, `cmap`, `hmtx`, plus optional
    /// `loca`/`glyf`), returning `None` if malformed or missing required tables.
    #[must_use]
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        Self::parse_checked_at(bytes, 0).ok()
    }

    /// Parse an SFNT/TrueType font face starting at `sfnt_offset` inside `bytes` (used both for
    /// standalone SFNT files at `0` and individual faces inside a `.ttc` TrueType collection).
    #[must_use]
    pub fn parse_at(bytes: &'a [u8], sfnt_offset: usize) -> Option<Self> {
        Self::parse_checked_at(bytes, sfnt_offset).ok()
    }

    /// Parse and validate an SFNT/TrueType font byte slice, returning a structured [`crate::TextError`]
    /// on malformed headers, missing required tables, or unsupported `cmap` subtables.
    ///
    /// # Errors
    /// Returns [`crate::TextError`] when the SFNT directory or required tables are invalid.
    pub fn parse_checked(bytes: &'a [u8]) -> Result<Self, crate::TextError> {
        Self::parse_checked_at(bytes, 0)
    }

    /// Parse and validate an SFNT/TrueType font face starting at `sfnt_offset` inside `bytes`.
    ///
    /// # Errors
    /// Returns [`crate::TextError`] when the SFNT directory or required tables are invalid.
    pub fn parse_checked_at(bytes: &'a [u8], sfnt_offset: usize) -> Result<Self, crate::TextError> {
        if let Ok(collection) = CollectionRef::new(bytes) {
            let ttc = TTCHeader::read(FontData::new(bytes)).map_err(map_read_error)?;
            let face_index = ttc
                .table_directory_offsets()
                .iter()
                .position(|off| usize::try_from(off.get()).ok() == Some(sfnt_offset))
                .and_then(|idx| u32::try_from(idx).ok())
                .ok_or_else(|| invalid_sfnt("SFNT offset not found in TTC header"))?;
            let font = collection.get(face_index).map_err(map_read_error)?;
            return Self::from_font_ref(font);
        }
        if sfnt_offset != 0 {
            return Err(invalid_sfnt("non-zero SFNT offset on non-TTC font"));
        }
        let font = FontRef::new(bytes).map_err(map_read_error)?;
        Self::from_font_ref(font)
    }

    /// Parse a single SFNT or TrueType Collection (`.ttc`) byte slice into `(sfnt_offset, font)` pairs.
    #[must_use]
    pub fn parse_collection(bytes: &'a [u8]) -> Vec<(usize, Self)> {
        if let (Ok(collection), Ok(ttc)) = (CollectionRef::new(bytes), TTCHeader::read(FontData::new(bytes))) {
            return ttc
                .table_directory_offsets()
                .iter()
                .enumerate()
                .take(64)
                .filter_map(|(idx, off_be)| {
                    let off = usize::try_from(off_be.get()).ok()?;
                    let idx_u32 = u32::try_from(idx).ok()?;
                    let font = collection.get(idx_u32).ok()?;
                    let parsed = Self::from_font_ref(font).ok()?;
                    Some((off, parsed))
                })
                .collect();
        }
        Self::parse_at(bytes, 0).map_or_else(Vec::new, |font| vec![(0, font)])
    }

    pub(crate) fn parse_ttc_index(bytes: &'a [u8], face_index: u32) -> Option<Self> {
        let collection = CollectionRef::new(bytes).ok()?;
        let font = collection.get(face_index).ok()?;
        Self::from_font_ref(font).ok()
    }

    pub(crate) fn from_font_ref(font: FontRef<'a>) -> Result<Self, crate::TextError> {
        validate_sfnt_directory(&font)?;
        let head = font.head().map_err(map_read_error)?;
        let units_per_em = head.units_per_em();
        if !(16..=16384).contains(&units_per_em) {
            return Err(invalid_sfnt("head unitsPerEm outside 16..=16384"));
        }
        let hhea = font.hhea().map_err(map_read_error)?;
        let maxp = font.maxp().map_err(map_read_error)?;
        let num_glyphs = maxp.num_glyphs();
        if num_glyphs == 0 {
            return Err(invalid_sfnt("maxp numGlyphs is zero"));
        }
        let num_h_metrics = hhea.number_of_h_metrics();
        if num_h_metrics == 0 || num_h_metrics > num_glyphs {
            return Err(invalid_sfnt("hhea numberOfHMetrics out of range for maxp numGlyphs"));
        }
        let hmtx_data = font.table_data(Tag::new(b"hmtx")).ok_or(crate::TextError::MissingSfntTable { tag: *b"hmtx" })?;
        if hmtx_data.len() < usize::from(num_h_metrics) * 4 {
            return Err(invalid_sfnt("hmtx table shorter than numberOfHMetrics * 4"));
        }
        let _ = font.hmtx().map_err(map_read_error)?;
        let cmap = font.cmap().map_err(map_read_error)?;
        let cmap_subtables = CmapSubtables::parse(&cmap)?;
        let has_glyf_outlines = font.table_data(Tag::new(b"loca")).is_some() && font.table_data(Tag::new(b"glyf")).is_some();

        Ok(Self {
            font,
            units_per_em,
            x_min: head.x_min(),
            y_min: head.y_min(),
            x_max: head.x_max(),
            y_max: head.y_max(),
            mac_style: head.mac_style(),
            index_to_loc_format: head.index_to_loc_format(),
            ascent: hhea.ascender().to_i16(),
            descent: hhea.descender().to_i16(),
            line_gap: hhea.line_gap().to_i16(),
            num_glyphs,
            cmap_subtables,
            has_glyf_outlines,
        })
    }

    pub(crate) const fn font_ref(&self) -> &FontRef<'a> {
        &self.font
    }

    #[must_use]
    pub const fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    #[must_use]
    pub const fn ascent(&self) -> i16 {
        self.ascent
    }

    #[must_use]
    pub const fn descent(&self) -> i16 {
        self.descent
    }

    #[must_use]
    pub const fn line_gap(&self) -> i16 {
        self.line_gap
    }

    #[must_use]
    pub const fn num_glyphs(&self) -> u16 {
        self.num_glyphs
    }

    #[must_use]
    pub const fn index_to_loc_format(&self) -> i16 {
        self.index_to_loc_format
    }

    #[must_use]
    pub const fn has_glyf_outlines(&self) -> bool {
        self.has_glyf_outlines
    }

    /// Map Unicode scalar value `ch` to a non-zero TrueType glyph index via `cmap`.
    #[must_use]
    pub fn glyph_id(&self, ch: char) -> Option<u16> {
        let cmap = self.font.cmap().ok()?;
        let rec = cmap.encoding_records().get(self.cmap_subtables.primary_idx)?;
        let sub = rec.subtable(cmap.offset_data()).ok()?;
        let gid = u16::try_from(sub.map_codepoint(u32::from(ch))?.to_u32()).ok()?;
        (gid != 0 && gid < self.num_glyphs).then_some(gid)
    }

    /// Read raw advance width in font design units from `hmtx` for `glyph_id`.
    #[must_use]
    pub fn glyph_advance_units(&self, glyph_id: u16) -> Option<u16> {
        if glyph_id >= self.num_glyphs {
            return None;
        }
        let hmtx = self.font.hmtx().ok()?;
        hmtx.advance(GlyphId::from(glyph_id))
    }

    /// Compute normalized `em` advance width for Unicode `ch` when mapped in `cmap`.
    #[must_use]
    pub fn advance_em(&self, ch: char) -> Option<f64> {
        let gid = self.glyph_id(ch)?;
        let adv = self.glyph_advance_units(gid)?;
        Some(f64::from(adv) / f64::from(self.units_per_em))
    }

    /// Return the raw `glyf` table slice for `glyph_id` (empty slice for empty glyphs like space).
    #[must_use]
    pub fn glyf_slice(&self, glyph_id: u16) -> Option<&'a [u8]> {
        if glyph_id >= self.num_glyphs {
            return None;
        }
        let loca = self.font.loca(Some(self.index_to_loc_format == 1)).ok()?;
        let glyf = self.font.table_data(Tag::new(b"glyf"))?;
        let idx = usize::from(glyph_id);
        let start = usize::try_from(loca.get_raw(idx)?).ok()?;
        let end = usize::try_from(loca.get_raw(idx + 1)?).ok()?;
        if end < start || end > glyf.len() {
            return None;
        }
        glyf.as_bytes().get(start..end)
    }

    /// Extract PDF-descriptor metadata, Windows-1252 widths, and Unicode `cmap` mappings from this
    /// already-parsed SFNT face without re-scanning the SFNT directory.
    ///
    /// # Errors
    /// Returns [`crate::TextError`] if optional `OS/2`, `post`, `name`, or `cmap` subtable payloads
    /// are malformed.
    pub fn inspect_info(&self) -> Result<TrueTypeFontInfo, crate::TextError> {
        let os2_metrics = self.font.table_data(Tag::new(b"OS/2")).map(parse_os2_table).transpose()?;
        let post_metrics = self.font.table_data(Tag::new(b"post")).map(parse_post_table).transpose()?;
        let (parsed_family, parsed_ps) = self.font.table_data(Tag::new(b"name")).map(parse_name_table).transpose()?.unwrap_or_default();
        let cmap = self.font.cmap().map_err(map_read_error)?;
        let cmap_basic = collect_cmap_mappings(&cmap, self.cmap_subtables, self.num_glyphs)?;

        let units_per_em = self.units_per_em;
        let raw_asc = if self.ascent != 0 { self.ascent } else { os2_metrics.map_or(800, |m| m.typo_ascender) };
        let raw_desc = if self.descent != 0 { self.descent } else { os2_metrics.map_or(-200, |m| m.typo_descender) };

        let ascender_1000 = scale_font_units_1000(i32::from(raw_asc), units_per_em);
        let descender_1000 = scale_font_units_1000(i32::from(raw_desc), units_per_em);
        let cap_height_1000 = os2_metrics
            .and_then(|m| m.cap_height)
            .filter(|&c| c > 0)
            .map_or_else(|| (ascender_1000 * 7 / 8).max(1), |c| scale_font_units_1000(i32::from(c), units_per_em));

        let is_italic_flag = os2_metrics.is_some_and(|m| m.is_italic) || self.mac_style.contains(MacStyle::ITALIC);
        let italic_angle = post_metrics.map_or(if is_italic_flag { -12.0 } else { 0.0 }, |p| p.italic_angle);

        let is_fixed_pitch = post_metrics.is_some_and(|p| p.is_fixed_pitch);
        let is_serif = os2_metrics.is_some_and(|m| (1..=7).contains(&((m.family_class >> 8) & 0xFF)));
        let mut flags = 32u32;
        if is_fixed_pitch {
            flags |= 1;
        }
        if is_serif {
            flags |= 2;
        }
        if italic_angle.abs() > 1e-6 || is_italic_flag {
            flags |= 64;
        }

        let weight_class = os2_metrics.map_or(if self.mac_style.contains(MacStyle::BOLD) { 700 } else { 400 }, |m| m.weight_class);
        let stem_v = (10 + (i32::from(weight_class) / 100) * 18).clamp(30, 300);

        let bbox_1000 = [
            scale_font_units_1000(i32::from(self.x_min), units_per_em),
            scale_font_units_1000(i32::from(self.y_min), units_per_em),
            scale_font_units_1000(i32::from(self.x_max), units_per_em),
            scale_font_units_1000(i32::from(self.y_max), units_per_em),
        ];

        let widths_1000 = build_win1252_widths(self, &cmap_basic)?;

        let family_name = if !parsed_family.is_empty() {
            parsed_family
        } else if !parsed_ps.is_empty() {
            parsed_ps.clone()
        } else {
            String::from("CurveBook Sans")
        };
        let postscript_name = if !parsed_ps.is_empty() { sanitize_postscript_name(&parsed_ps) } else { sanitize_postscript_name(&family_name) };

        Ok(TrueTypeFontInfo {
            postscript_name,
            family_name,
            units_per_em,
            ascender_1000,
            descender_1000,
            cap_height_1000,
            italic_angle,
            flags,
            stem_v,
            bbox_1000,
            widths_1000,
            cmap_basic,
        })
    }
}

#[must_use]
pub(crate) fn select_bundled_font(weight: SpanWeight, slope: SpanSlope) -> Option<&'static TrueTypeFont<'static>> {
    static PARSED: OnceLock<Vec<(&'static str, TrueTypeFont<'static>)>> = OnceLock::new();
    let fonts =
        PARSED.get_or_init(|| BUNDLED_FONTS.iter().filter_map(|&(name, bytes)| TrueTypeFont::parse(bytes).map(|font| (name, font))).collect());
    if fonts.is_empty() {
        return None;
    }

    let want_bold = weight == SpanWeight::Bold;
    let want_italic = slope == SpanSlope::Italic;
    let mut best: Option<(i32, &'static TrueTypeFont<'static>)> = None;

    for (name, font) in fonts {
        let lower = name.to_ascii_lowercase();
        let is_bold = lower.contains("bold");
        let is_italic = lower.contains("italic") || lower.contains("oblique");
        let is_sans = !lower.contains("mono") && !lower.contains("serif");
        let mut score = 0i32;
        if is_bold == want_bold {
            score += 4;
        }
        if is_italic == want_italic {
            score += 4;
        }
        if is_sans {
            score += 2;
        }
        if lower.contains("regular") && !want_bold && !want_italic {
            score += 1;
        }
        if best.is_none_or(|(b_score, _)| score > b_score) {
            best = Some((score, font));
        }
    }

    best.map(|(_, font)| font)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SfntFaceMeta {
    pub(crate) family_name: String,
    pub(crate) is_bold: bool,
    pub(crate) is_italic: bool,
    pub(crate) is_fixed_pitch: bool,
    pub(crate) is_serif: bool,
    pub(crate) is_variable: bool,
}

fn validate_sfnt_directory(font: &FontRef<'_>) -> Result<(), crate::TextError> {
    let bytes_len = font.data().len();
    let dir = font.table_directory();
    let num_tables = dir.num_tables();
    if num_tables == 0 || num_tables > 128 {
        return Err(invalid_sfnt("invalid SFNT numTables"));
    }
    let records = dir.table_records();
    if records.len() != usize::from(num_tables) {
        return Err(invalid_sfnt("SFNT table directory truncated"));
    }
    for rec in records {
        let offset = usize::try_from(rec.offset()).map_err(|_| invalid_sfnt("invalid table offset"))?;
        if offset == 0 {
            return Err(invalid_sfnt("null SFNT table offset"));
        }
        let length = usize::try_from(rec.length()).map_err(|_| invalid_sfnt("invalid table length"))?;
        let end = offset.checked_add(length).ok_or_else(|| invalid_sfnt("table range overflow"))?;
        if end > bytes_len {
            return Err(invalid_sfnt("SFNT table extends past EOF"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct CmapSubtables {
    primary_idx: usize,
    best_fmt4: Option<usize>,
    best_fmt12: Option<usize>,
    best_fmt0: Option<usize>,
}

impl CmapSubtables {
    fn parse(cmap: &Cmap<'_>) -> Result<Self, crate::TextError> {
        let num_tables = cmap.num_tables();
        if num_tables == 0 {
            return Err(crate::TextError::UnsupportedCmap { message: String::from("cmap has zero encoding records") });
        }
        let records = cmap.encoding_records();
        if records.len() != usize::from(num_tables) {
            return Err(invalid_sfnt("cmap encoding records truncated"));
        }

        let mut primary: Option<(i32, usize)> = None;
        let mut best_fmt4: Option<(i32, usize)> = None;
        let mut best_fmt12: Option<(i32, usize)> = None;
        let mut best_fmt0: Option<(i32, usize)> = None;
        let data = cmap.offset_data();

        for (idx, rec) in records.iter().enumerate() {
            let sub = match rec.subtable(data) {
                Ok(sub) => sub,
                Err(ReadError::InvalidFormat(_)) => continue,
                Err(err) => return Err(map_read_error(err)),
            };
            let fmt = match &sub {
                CmapSubtable::Format0(_) => 0u16,
                CmapSubtable::Format4(sub4) => {
                    validate_cmap4(sub4)?;
                    4u16
                }
                CmapSubtable::Format12(sub12) => {
                    validate_cmap12(sub12)?;
                    12u16
                }
                _ => continue,
            };
            let rank = cmap_subtable_rank(rec.platform_id(), rec.encoding_id(), fmt);
            if primary.is_none_or(|(b_rank, _)| rank > b_rank) {
                primary = Some((rank, idx));
            }
            match fmt {
                4 if best_fmt4.is_none_or(|(r, _)| rank > r) => best_fmt4 = Some((rank, idx)),
                12 if best_fmt12.is_none_or(|(r, _)| rank > r) => best_fmt12 = Some((rank, idx)),
                0 if best_fmt0.is_none_or(|(r, _)| rank > r) => best_fmt0 = Some((rank, idx)),
                _ => {}
            }
        }

        let Some((_, primary_idx)) = primary else {
            return Err(crate::TextError::UnsupportedCmap { message: String::from("no format 0, 4, or 12 cmap subtable found") });
        };
        Ok(Self {
            primary_idx,
            best_fmt4: best_fmt4.map(|(_, idx)| idx),
            best_fmt12: best_fmt12.map(|(_, idx)| idx),
            best_fmt0: best_fmt0.map(|(_, idx)| idx),
        })
    }
}

const fn cmap_subtable_rank(platform: PlatformId, encoding: u16, fmt: u16) -> i32 {
    match (platform, encoding, fmt) {
        (PlatformId::Windows, 10, 12) => 100,
        (PlatformId::Unicode, 4, 12) => 95,
        (PlatformId::Windows, 1, 4) => 90,
        (PlatformId::Unicode, 3, 4) => 85,
        (PlatformId::Unicode, _, 4) => 80,
        (PlatformId::Unicode, _, 12) => 75,
        (PlatformId::Macintosh, 0, 0) => 20,
        _ => 10,
    }
}

fn validate_cmap4(sub4: &Cmap4<'_>) -> Result<(), crate::TextError> {
    let seg_count_x2 = sub4.seg_count_x2();
    if seg_count_x2 < 2 || !seg_count_x2.is_multiple_of(2) {
        return Err(invalid_sfnt("invalid cmap4 segCountX2"));
    }
    let seg_count = usize::from(seg_count_x2 / 2);
    if sub4.end_code().len() != seg_count
        || sub4.start_code().len() != seg_count
        || sub4.id_delta().len() != seg_count
        || sub4.id_range_offsets().len() != seg_count
    {
        return Err(invalid_sfnt("cmap format 4 segments truncated"));
    }
    Ok(())
}

fn validate_cmap12(sub12: &Cmap12<'_>) -> Result<(), crate::TextError> {
    if usize::try_from(sub12.num_groups()).ok() != Some(sub12.groups().len()) {
        return Err(invalid_sfnt("cmap format 12 groups truncated"));
    }
    Ok(())
}

pub(crate) fn inspect_sfnt_face_meta(font: &FontRef<'_>, fallback_name: &str) -> SfntFaceMeta {
    let os2 = font.table_data(Tag::new(b"OS/2")).and_then(|d| parse_os2_table(d).ok());
    let post = font.table_data(Tag::new(b"post")).and_then(|d| parse_post_table(d).ok());
    let parsed_names = font.table_data(Tag::new(b"name")).and_then(|d| parse_name_table(d).ok());
    let is_variable = font.table_data(Tag::new(b"fvar")).is_some();

    let mac_style = font.head().map_or_else(|_| MacStyle::empty(), |h| h.mac_style());
    let family_name = parsed_names.map(|(fam, ps)| if !fam.is_empty() { fam } else { ps }).unwrap_or_default();
    let combined_label = format!("{} {}", family_name.to_ascii_lowercase(), fallback_name.to_ascii_lowercase());

    let is_bold = os2.is_some_and(|m| m.weight_class >= 600 || m.is_bold) || mac_style.contains(MacStyle::BOLD) || combined_label.contains("bold");
    let is_italic = os2.is_some_and(|m| m.is_italic)
        || mac_style.contains(MacStyle::ITALIC)
        || post.is_some_and(|p| p.italic_angle.abs() > 1e-6)
        || combined_label.contains("italic")
        || combined_label.contains("oblique");
    let is_fixed_pitch = post.is_some_and(|p| p.is_fixed_pitch)
        || combined_label.contains("mono")
        || combined_label.contains("courier")
        || combined_label.contains("consola")
        || combined_label.contains("menlo")
        || combined_label.contains("monaco");
    let is_serif = !is_fixed_pitch
        && (os2.is_some_and(|m| (1..=7).contains(&((m.family_class >> 8) & 0xFF)))
            || combined_label.contains("serif")
            || combined_label.contains("times")
            || combined_label.contains("new york")
            || combined_label.contains("newyork")
            || combined_label.contains("georgia")
            || combined_label.contains("cambria"));

    SfntFaceMeta { family_name, is_bold, is_italic, is_fixed_pitch, is_serif, is_variable }
}

// ---------------------------------------------------------------------------
// SFNT Inspection, Pure-Rust Fallback TTF Builder & CJK Font Candidates
// ---------------------------------------------------------------------------

const MAX_CMAP_COLLECT: usize = 65_536;

/// Parsed metadata, descriptor metrics, Windows-1252 advance widths, and Unicode `cmap`
/// mappings extracted from a TrueType/OpenType SFNT binary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrueTypeFontInfo {
    pub postscript_name: String,
    pub family_name: String,
    pub units_per_em: u16,
    pub ascender_1000: i32,
    pub descender_1000: i32,
    pub cap_height_1000: i32,
    pub italic_angle: f64,
    pub flags: u32,
    pub stem_v: i32,
    pub bbox_1000: [i32; 4],
    pub widths_1000: Vec<u16>,
    pub cmap_basic: Vec<(u32, u16)>,
}

impl TrueTypeFontInfo {
    /// Look up the TrueType glyph ID for Unicode scalar `ch` (`0` for `.notdef` when unmapped).
    #[must_use]
    pub fn glyph_id(&self, ch: char) -> u16 {
        let target = u32::from(ch);
        if let Ok(idx) = self.cmap_basic.binary_search_by_key(&target, |&(cp, _)| cp) {
            return self.cmap_basic.get(idx).map_or(0, |&(_, gid)| gid);
        }
        if self.cmap_basic.len() <= 64 {
            return self.cmap_basic.iter().find(|&&(cp, _)| cp == target).map_or(0, |&(_, gid)| gid);
        }
        0
    }

    /// Compute the advance width in PostScript points (`pt`) for `ch` at `font_size_pt`.
    #[must_use]
    pub fn width_pt(&self, ch: char, font_size_pt: f64) -> f64 {
        if !font_size_pt.is_finite() || font_size_pt <= 0.0 || matches!(ch, '\n' | '\r') {
            return 0.0;
        }
        if ch == '\t' {
            return 4.0 * self.width_pt(' ', font_size_pt);
        }
        if self.glyph_id(ch) != 0
            && let Some(code) = char_to_win1252_byte(ch)
            && code >= 32
            && let Some(&w1000) = self.widths_1000.get(usize::from(code - 32))
        {
            return f64::from(w1000) * font_size_pt / 1000.0;
        }
        core14_advance_em(ch, SpanWeight::Regular, SpanSlope::Upright) * font_size_pt
    }
}

/// Inspect an SFNT/TrueType byte slice (`head`, `hhea`, `maxp`, `hmtx`, `OS/2`, `post`, `name`,
/// and `cmap` format 4 / 12) with strict bounds checking and zero panics.
///
/// # Errors
/// Returns [`crate::TextError`] if the SFNT header or any table is malformed, truncated, or lacks
/// required tables (`head`, `hhea`, `maxp`, `hmtx`, `cmap`).
pub fn parse_truetype_font_info(sfnt_bytes: &[u8]) -> Result<TrueTypeFontInfo, crate::TextError> {
    TrueTypeFont::parse_checked(sfnt_bytes)?.inspect_info()
}

/// Return `true` when at least one valid bundled TrueType/OpenType font is compiled into
/// [`BUNDLED_FONTS`].
#[must_use]
pub fn has_bundled_or_ttf_font() -> bool {
    BUNDLED_FONTS.iter().any(|&(_, bytes)| TrueTypeFont::parse(bytes).is_some())
}

/// Standard macOS, Windows, and Linux CJK fallback font family names ordered by `locale`.
#[must_use]
pub fn system_cjk_font_candidates(locale: &str) -> &'static [&'static str] {
    const JA_CANDIDATES: &[&str] =
        &["Hiragino Sans", "Hiragino Kaku Gothic ProN", "Hirakaku", "Yu Gothic", "Meiryo", "Noto Sans CJK JP", "Noto Sans JP", "BIZ UDPGothic"];
    const KO_CANDIDATES: &[&str] = &["Apple SD Gothic Neo", "Malgun Gothic", "Noto Sans CJK KR", "Noto Sans KR", "NanumGothic"];
    const ZH_HANT_CANDIDATES: &[&str] = &["PingFang TC", "Microsoft JhengHei", "Noto Sans CJK TC", "Noto Sans TC", "Heiti TC"];
    const ZH_HANS_CANDIDATES: &[&str] = &["PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", "Noto Sans SC", "Hiragino Sans GB", "Heiti SC"];
    const GENERIC_CJK_CANDIDATES: &[&str] = &[
        "Noto Sans CJK",
        "PingFang SC",
        "PingFang TC",
        "Hiragino Sans",
        "Hiragino Kaku Gothic ProN",
        "Hirakaku",
        "Yu Gothic",
        "Malgun Gothic",
        "Microsoft YaHei",
        "Microsoft JhengHei",
    ];

    let normalized = locale.trim().to_ascii_lowercase().replace('_', "-");
    if normalized == "ja" || normalized.starts_with("ja-") {
        JA_CANDIDATES
    } else if normalized == "ko" || normalized.starts_with("ko-") {
        KO_CANDIDATES
    } else if normalized.starts_with("zh-tw")
        || normalized.starts_with("zh-hk")
        || normalized.starts_with("zh-mo")
        || normalized.starts_with("zh-hant")
    {
        ZH_HANT_CANDIDATES
    } else if normalized == "zh" || normalized.starts_with("zh-") {
        ZH_HANS_CANDIDATES
    } else {
        GENERIC_CJK_CANDIDATES
    }
}

/// Pure-Rust in-memory SFNT (`0x00010000`) TrueType font builder covering `.notdef`,
/// printable ASCII (`32..=126`), Latin-1 (`160..=255`), and scientific Greek
/// (`α`, `β`, `γ`, `Δ`, `μ`, `σ`).
///
/// Produces a valid binary font with `OS/2`, `cmap` (format 4), `glyf`, `head`, `hhea`,
/// `hmtx`, `loca`, `maxp`, `name`, and `post` tables without bundling external `.ttf` files.
#[must_use]
pub fn build_minimal_truetype_font() -> Vec<u8> {
    let glyphs = build_minimal_glyph_specs();
    let cmap = build_minimal_cmap_table();
    let name = build_sfnt_name_table("CurveBook Sans", "Regular", "CurveBook-Sans-Regular", "CurveBook Sans Regular", "CurveBookSans-Regular");
    build_sfnt_from_specs(&glyphs, cmap, name, 0x0000_0083, 32, 0x03C3)
}

/// Build an in-memory cleanroom TrueType fallback font containing CurveBook's mandatory UI and
/// scientific symbols (`→ ▾ ● ✓ ± ≤ ≥ μ χ²` plus directional/math/statistical glyphs) (`R15.1`).
///
/// Used as a deterministic fallback SFNT layer after `CURVEBOOK_FONTS_DIR` and host OS system
/// fonts so headless environments and primary faces lacking specific symbol glyphs never render
/// missing-glyph placeholders.
#[must_use]
pub fn build_symbol_fallback_truetype_font() -> Vec<u8> {
    let (glyphs, cmap) = build_symbol_fallback_glyphs_and_cmap();
    let name =
        build_sfnt_name_table("CurveBook Symbols", "Regular", "CurveBook-Symbols-Regular", "CurveBook Symbols Regular", "CurveBookSymbols-Regular");
    // Unicode ranges: Latin-1 (bit 1), Greek (bit 7), General Punctuation (bit 31/upper),
    // Arrows + Math Operators + Geometric Shapes + Dingbats.
    build_sfnt_from_specs(&glyphs, cmap, name, 0x0000_0082, 0x00B0, 0x2717)
}

fn build_sfnt_from_specs(
    glyphs: &[MinimalGlyphSpec],
    cmap: Vec<u8>,
    name: Vec<u8>,
    unicode_range_1: u32,
    first_char: u16,
    last_char: u16,
) -> Vec<u8> {
    let num_glyphs = glyphs.len() as u16;
    let (glyf, loca, font_bbox, h_extents) = build_glyf_and_loca(glyphs);
    let hmtx = build_hmtx_table(glyphs);
    let head = build_head_table(font_bbox);
    let hhea = build_hhea_table(num_glyphs, h_extents);
    let maxp = build_maxp_table(num_glyphs);
    let os2 = build_os2_table(unicode_range_1, first_char, last_char);
    let post = build_minimal_post_table();

    assemble_sfnt_tables(&[
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ])
}

// --- SFNT Parsing Helpers ---

fn invalid_sfnt(message: impl Into<String>) -> crate::TextError {
    crate::TextError::InvalidSfnt { message: message.into() }
}

fn map_read_error(err: ReadError) -> crate::TextError {
    match err {
        ReadError::TableIsMissing(tag) => crate::TextError::MissingSfntTable { tag: tag.into_bytes() },
        other => invalid_sfnt(other.to_string()),
    }
}

#[derive(Clone, Copy)]
struct Os2Metrics {
    weight_class: u16,
    family_class: i16,
    is_bold: bool,
    is_italic: bool,
    typo_ascender: i16,
    typo_descender: i16,
    cap_height: Option<i16>,
}

fn parse_os2_table(data: FontData<'_>) -> Result<Os2Metrics, crate::TextError> {
    let os2 = Os2::read(data).map_err(map_read_error)?;
    if os2.version() >= 2 && data.len() < 96 {
        return Err(invalid_sfnt("OS/2 v2+ table shorter than 96 bytes"));
    }
    let fs = os2.fs_selection();
    Ok(Os2Metrics {
        weight_class: os2.us_weight_class(),
        family_class: os2.s_family_class(),
        is_bold: fs.contains(SelectionFlags::BOLD),
        is_italic: fs.contains(SelectionFlags::ITALIC),
        typo_ascender: os2.s_typo_ascender(),
        typo_descender: os2.s_typo_descender(),
        cap_height: if os2.version() >= 2 { os2.s_cap_height() } else { None },
    })
}

#[derive(Clone, Copy)]
struct PostMetrics {
    italic_angle: f64,
    is_fixed_pitch: bool,
}

fn parse_post_table(data: FontData<'_>) -> Result<PostMetrics, crate::TextError> {
    let post = Post::read(data).map_err(map_read_error)?;
    Ok(PostMetrics { italic_angle: f64::from(post.italic_angle().to_bits()) / 65536.0, is_fixed_pitch: post.is_fixed_pitch() != 0 })
}

fn parse_name_table(data: FontData<'_>) -> Result<(String, String), crate::TextError> {
    let name = Name::read(data).map_err(map_read_error)?;
    if name.version() > 1 {
        return Err(invalid_sfnt("unsupported name table format"));
    }
    if usize::from(name.storage_offset()) > data.len() {
        return Err(invalid_sfnt("name stringOffset past end of table"));
    }
    let records = name.name_record();
    if records.len() != usize::from(name.count()) {
        return Err(invalid_sfnt("name records truncated"));
    }
    let string_data = name.string_data();
    let mut best_family: Option<(i32, String)> = None;
    let mut best_ps: Option<(i32, String)> = None;

    for rec in records {
        let decoded = rec.string(string_data).map_err(map_read_error)?;
        let name_id = rec.name_id().to_u16();
        if !matches!(name_id, 1 | 6 | 16) {
            continue;
        }
        let cleaned: String = decoded.chars().filter(|&c| c != '\0' && (c >= ' ' || c.is_whitespace())).collect();
        let trimmed = cleaned.trim().to_owned();
        if trimmed.is_empty() {
            continue;
        }

        let base_rank = match (rec.platform_id(), rec.language_id()) {
            (3, 0x0409) => 100,
            (3, _) => 80,
            (0, _) => 70,
            (1, 0) => 50,
            (1, _) => 30,
            _ => 10,
        };
        if name_id == 6 {
            if best_ps.as_ref().is_none_or(|(r, _)| base_rank > *r) {
                best_ps = Some((base_rank, trimmed));
            }
        } else {
            let fam_rank = base_rank + if name_id == 16 { 10 } else { 0 };
            if best_family.as_ref().is_none_or(|(r, _)| fam_rank > *r) {
                best_family = Some((fam_rank, trimmed));
            }
        }
    }

    Ok((best_family.map_or_else(String::new, |(_, s)| s), best_ps.map_or_else(String::new, |(_, s)| s)))
}

fn collect_cmap_mappings(cmap: &Cmap<'_>, subtables: CmapSubtables, num_glyphs: u16) -> Result<Vec<(u32, u16)>, crate::TextError> {
    let mut merged = Vec::new();
    let records = cmap.encoding_records();
    let data = cmap.offset_data();

    if let Some(idx) = subtables.best_fmt4
        && let Some(rec) = records.get(idx)
        && let CmapSubtable::Format4(sub4) = rec.subtable(data).map_err(map_read_error)?
    {
        collect_cmap_format4(&sub4, num_glyphs, &mut merged)?;
    }
    if let Some(idx) = subtables.best_fmt12
        && let Some(rec) = records.get(idx)
        && let CmapSubtable::Format12(sub12) = rec.subtable(data).map_err(map_read_error)?
    {
        collect_cmap_format12(&sub12, num_glyphs, &mut merged)?;
    }
    if merged.is_empty()
        && let Some(idx) = subtables.best_fmt0
        && let Some(rec) = records.get(idx)
        && let CmapSubtable::Format0(sub0) = rec.subtable(data).map_err(map_read_error)?
    {
        collect_cmap_format0(&sub0, num_glyphs, &mut merged);
    }

    merged.sort_unstable_by_key(|&(cp, _)| cp);
    merged.dedup_by_key(|pair| pair.0);
    Ok(merged)
}

fn collect_cmap_format4(sub4: &Cmap4<'_>, num_glyphs: u16, out: &mut Vec<(u32, u16)>) -> Result<(), crate::TextError> {
    for (start_be, end_be) in sub4.start_code().iter().zip(sub4.end_code().iter()) {
        if start_be.get() > end_be.get() {
            return Err(invalid_sfnt("cmap4 segment has startCode > endCode"));
        }
    }
    for (cp, gid) in sub4.iter() {
        if out.len() >= MAX_CMAP_COLLECT {
            return Ok(());
        }
        if cp == 0xFFFF {
            continue;
        }
        if let Ok(gid_u16) = u16::try_from(gid.to_u32())
            && gid_u16 != 0
            && gid_u16 < num_glyphs
        {
            out.push((cp, gid_u16));
        }
    }
    Ok(())
}

fn collect_cmap_format12(sub12: &Cmap12<'_>, num_glyphs: u16, out: &mut Vec<(u32, u16)>) -> Result<(), crate::TextError> {
    for group in sub12.groups() {
        if group.start_char_code() > group.end_char_code() {
            return Err(invalid_sfnt("cmap12 group has startCharCode > endCharCode"));
        }
    }
    let limits = CmapIterLimits { max_char: u32::from(char::MAX), glyph_count: u32::from(num_glyphs) };
    for (cp, gid) in sub12.iter_with_limits(limits) {
        if out.len() >= MAX_CMAP_COLLECT {
            return Ok(());
        }
        if let Ok(gid_u16) = u16::try_from(gid.to_u32())
            && gid_u16 != 0
            && gid_u16 < num_glyphs
        {
            out.push((cp, gid_u16));
        }
    }
    Ok(())
}

fn collect_cmap_format0(sub0: &Cmap0<'_>, num_glyphs: u16, out: &mut Vec<(u32, u16)>) {
    for (code, &gid_byte) in sub0.glyph_id_array().iter().enumerate() {
        let gid = u16::from(gid_byte);
        if gid != 0 && gid < num_glyphs {
            out.push((code as u32, gid));
        }
    }
}

fn build_win1252_widths(font: &TrueTypeFont<'_>, cmap: &[(u32, u16)]) -> Result<Vec<u16>, crate::TextError> {
    let units_per_em = font.units_per_em();
    let notdef_adv = font.glyph_advance_units(0).ok_or_else(|| invalid_sfnt("hmtx advance offset out of bounds"))?;
    let notdef_1000 = u16::try_from(scale_font_units_1000(i32::from(notdef_adv), units_per_em).clamp(0, i32::from(u16::MAX))).unwrap_or(500);
    let mut widths = Vec::with_capacity(224);

    for code in 32u8..=255u8 {
        let gid = win1252_byte_to_char(code)
            .and_then(|ch| cmap.binary_search_by_key(&u32::from(ch), |&(cp, _)| cp).ok().and_then(|idx| cmap.get(idx).map(|&(_, g)| g)))
            .unwrap_or(0);
        if gid == 0 {
            widths.push(notdef_1000);
            continue;
        }
        let adv = font.glyph_advance_units(gid).ok_or_else(|| invalid_sfnt("hmtx advance offset out of bounds"))?;
        let scaled = scale_font_units_1000(i32::from(adv), units_per_em).clamp(0, i32::from(u16::MAX));
        widths.push(u16::try_from(scaled).unwrap_or(notdef_1000));
    }
    Ok(widths)
}

fn scale_font_units_1000(value: i32, units_per_em: u16) -> i32 {
    let upem = i64::from(units_per_em.max(1));
    let num = i64::from(value) * 1000;
    let rounded = if num >= 0 { (num + upem / 2) / upem } else { (num - upem / 2) / upem };
    i32::try_from(rounded).unwrap_or(if rounded >= 0 { i32::MAX } else { i32::MIN })
}

fn sanitize_postscript_name(raw: &str) -> String {
    let cleaned: String =
        raw.chars().filter(|&c| c.is_ascii_graphic() && !matches!(c, '(' | ')' | '<' | '>' | '[' | ']' | '{' | '}' | '/' | '%')).collect();
    if cleaned.is_empty() { String::from("CurveBookSans-Regular") } else { cleaned }
}

const WIN1252_80_9F: [Option<char>; 32] = [
    Some('\u{20AC}'),
    None,
    Some('\u{201A}'),
    Some('\u{0192}'),
    Some('\u{201E}'),
    Some('\u{2026}'),
    Some('\u{2020}'),
    Some('\u{2021}'),
    Some('\u{02C6}'),
    Some('\u{2030}'),
    Some('\u{0160}'),
    Some('\u{2039}'),
    Some('\u{0152}'),
    None,
    Some('\u{017D}'),
    None,
    None,
    Some('\u{2018}'),
    Some('\u{2019}'),
    Some('\u{201C}'),
    Some('\u{201D}'),
    Some('\u{2022}'),
    Some('\u{2013}'),
    Some('\u{2014}'),
    Some('\u{02DC}'),
    Some('\u{2122}'),
    Some('\u{0161}'),
    Some('\u{203A}'),
    Some('\u{0153}'),
    None,
    Some('\u{017E}'),
    Some('\u{0178}'),
];

const fn win1252_byte_to_char(code: u8) -> Option<char> {
    match code {
        0x20..=0x7E | 0xA0..=0xFF => Some(code as char),
        0x80..=0x9F => WIN1252_80_9F[(code - 0x80) as usize],
        _ => None,
    }
}

fn char_to_win1252_byte(ch: char) -> Option<u8> {
    let cp = ch as u32;
    if (0x20..=0x7E).contains(&cp) || (0xA0..=0xFF).contains(&cp) {
        return u8::try_from(cp).ok();
    }
    for (idx, entry) in WIN1252_80_9F.iter().enumerate() {
        if *entry == Some(ch) {
            return u8::try_from(0x80 + idx).ok();
        }
    }
    None
}

// --- Pure-Rust Minimal SFNT / TrueType Builder Helpers ---

struct MinimalGlyphSpec {
    advance: u16,
    lsb: i16,
    bbox: Option<[i16; 4]>,
}

struct HheaExtents {
    advance_max: u16,
    min_lsb: i16,
    min_rsb: i16,
    x_max_extent: i16,
}

fn build_minimal_glyph_specs() -> Vec<MinimalGlyphSpec> {
    let mut glyphs = Vec::with_capacity(198);
    // GID 0: .notdef
    glyphs.push(MinimalGlyphSpec { advance: 556, lsb: 60, bbox: Some([60, 0, 496, 720]) });
    // GIDs 1..=95: ASCII 32..=126
    for code in 32u8..=126u8 {
        glyphs.push(glyph_spec_for_char(char::from(code)));
    }
    // GIDs 96..=191: Latin-1 160..=255
    for code in 160u8..=255u8 {
        glyphs.push(glyph_spec_for_char(char::from(code)));
    }
    // GIDs 192..=197: Greek Δ, α, β, γ, μ, σ
    for ch in ['\u{0394}', '\u{03B1}', '\u{03B2}', '\u{03B3}', '\u{03BC}', '\u{03C3}'] {
        glyphs.push(glyph_spec_for_char(ch));
    }
    glyphs
}

fn glyph_spec_for_char(ch: char) -> MinimalGlyphSpec {
    let advance = minimal_char_width_1000(ch);
    if matches!(ch, ' ' | '\u{00A0}' | '\u{00AD}') {
        return MinimalGlyphSpec { advance, lsb: 0, bbox: None };
    }
    let adv_i16 = i16::try_from(advance).unwrap_or(556);
    let pad = (adv_i16 / 10).max(20);
    let x_min = pad;
    let x_max = (adv_i16 - pad).max(x_min + 40);
    let (y_min, y_max) = if ch.is_ascii_uppercase() || ch.is_ascii_digit() || matches!(ch, 'Δ' | 'b' | 'd' | 'f' | 'h' | 'k' | 'l' | 't') {
        (0, 720)
    } else if matches!(ch, 'g' | 'j' | 'p' | 'q' | 'y' | 'β' | 'γ' | 'μ') {
        (-200, 520)
    } else {
        (0, 520)
    };
    MinimalGlyphSpec { advance, lsb: x_min, bbox: Some([x_min, y_min, x_max, y_max]) }
}

fn minimal_char_width_1000(ch: char) -> u16 {
    if let Some(w) = core14_ascii_width_1000(ch, SpanWeight::Regular) {
        return w;
    }
    if let Some(w) = core14_symbol_width_1000(ch) {
        return w;
    }
    match ch {
        '\u{00A0}' => 278,
        '\u{00AD}' => 333,
        '\u{00C6}' | '\u{00D0}'..='\u{00D6}' | '\u{00D8}'..='\u{00DE}' => 722,
        '\u{00C0}'..='\u{00C5}' | '\u{00C7}'..='\u{00CF}' => 667,
        '\u{00EC}'..='\u{00EF}' => 278,
        '\u{00E6}' | '\u{00FD}'..='\u{00FF}' => 611,
        '\u{03C7}' => 540,
        '\u{2013}' => 556,
        '\u{2014}' => 1000,
        '\u{2026}' => 800,
        '\u{2190}'..='\u{2193}' => 720,
        '\u{2212}' | '\u{00D7}' | '\u{00F7}' => 580,
        '\u{221E}' | '\u{2248}' | '\u{2260}' => 600,
        '\u{25B2}' | '\u{25B6}' | '\u{25B8}' | '\u{25BC}' | '\u{25BE}' | '\u{25C0}' => 560,
        '\u{25CF}' => 600,
        '\u{2713}' | '\u{2717}' => 700,
        _ => 556,
    }
}

/// Strictly sorted, non-overlapping BMP codepoint ranges covered by
/// [`build_symbol_fallback_truetype_font`].
const SYMBOL_FALLBACK_RANGES: &[(u16, u16)] = &[
    (0x00B0, 0x00B3), // ° ± ² ³
    (0x00B7, 0x00B7), // ·
    (0x00D7, 0x00D7), // ×
    (0x00F7, 0x00F7), // ÷
    (0x0394, 0x0394), // Δ
    (0x03B1, 0x03B3), // α β γ
    (0x03BC, 0x03BC), // μ
    (0x03C3, 0x03C3), // σ
    (0x03C7, 0x03C7), // χ
    (0x2013, 0x2014), // – —
    (0x2026, 0x2026), // …
    (0x2190, 0x2193), // ← ↑ → ↓
    (0x2212, 0x2212), // −
    (0x2219, 0x2219), // ∙
    (0x221E, 0x221E), // ∞
    (0x2248, 0x2248), // ≈
    (0x2260, 0x2260), // ≠
    (0x2264, 0x2265), // ≤ ≥
    (0x25B2, 0x25B2), // ▲
    (0x25B6, 0x25B6), // ▶
    (0x25B8, 0x25B8), // ▸
    (0x25BC, 0x25BC), // ▼
    (0x25BE, 0x25BE), // ▾
    (0x25C0, 0x25C0), // ◀
    (0x25CF, 0x25CF), // ●
    (0x2713, 0x2713), // ✓
    (0x2717, 0x2717), // ✗
];

fn build_symbol_fallback_glyphs_and_cmap() -> (Vec<MinimalGlyphSpec>, Vec<u8>) {
    let total_chars: usize = SYMBOL_FALLBACK_RANGES.iter().map(|&(s, e)| usize::from(e - s) + 1).sum();
    let mut glyphs = Vec::with_capacity(total_chars + 1);
    glyphs.push(MinimalGlyphSpec { advance: 556, lsb: 60, bbox: Some([60, 0, 496, 720]) });

    let mut segments = Vec::with_capacity(SYMBOL_FALLBACK_RANGES.len() + 1);
    let mut next_gid: i32 = 1;
    for &(start, end) in SYMBOL_FALLBACK_RANGES {
        let delta = i16::try_from(next_gid - i32::from(start)).unwrap_or(0);
        segments.push((start, end, delta));
        for code in start..=end {
            let ch = char::from_u32(u32::from(code)).unwrap_or('?');
            glyphs.push(glyph_spec_for_char(ch));
            next_gid += 1;
        }
    }
    segments.push((0xFFFF, 0xFFFF, 1));
    (glyphs, build_bmp_cmap_table(&segments))
}

fn build_glyf_and_loca(glyphs: &[MinimalGlyphSpec]) -> (Vec<u8>, Vec<u8>, [i16; 4], HheaExtents) {
    let mut glyf = Vec::new();
    let mut loca = Vec::with_capacity((glyphs.len() + 1) * 2);
    let mut min_x = i16::MAX;
    let mut min_y = i16::MAX;
    let mut max_x = i16::MIN;
    let mut max_y = i16::MIN;
    let mut adv_max = 0u16;
    let mut min_lsb = i16::MAX;
    let mut min_rsb = i16::MAX;
    let mut x_max_extent = i16::MIN;

    for g in glyphs {
        push_u16_be(&mut loca, u16::try_from(glyf.len() / 2).unwrap_or(u16::MAX));
        adv_max = adv_max.max(g.advance);
        min_lsb = min_lsb.min(g.lsb);

        if let Some([x0, y0, x1, y1]) = g.bbox {
            min_x = min_x.min(x0);
            min_y = min_y.min(y0);
            max_x = max_x.max(x1);
            max_y = max_y.max(y1);
            let adv_i16 = i16::try_from(g.advance).unwrap_or(556);
            min_rsb = min_rsb.min(adv_i16.saturating_sub(x1));
            x_max_extent = x_max_extent.max(x1);

            push_i16_be(&mut glyf, 1); // numberOfContours
            push_i16_be(&mut glyf, x0);
            push_i16_be(&mut glyf, y0);
            push_i16_be(&mut glyf, x1);
            push_i16_be(&mut glyf, y1);
            push_u16_be(&mut glyf, 3); // endPtsOfContours[0]
            push_u16_be(&mut glyf, 0); // instructionLength
            glyf.extend_from_slice(&[0x01, 0x01, 0x01, 0x01]);
            // Clockwise Y-up rectangle: (x0,y0) -> (x0,y1) -> (x1,y1) -> (x1,y0)
            for dx in [x0, 0, x1 - x0, 0] {
                push_i16_be(&mut glyf, dx);
            }
            for dy in [y0, y1 - y0, 0, y0 - y1] {
                push_i16_be(&mut glyf, dy);
            }
            if !glyf.len().is_multiple_of(2) {
                glyf.push(0);
            }
        }
    }
    push_u16_be(&mut loca, u16::try_from(glyf.len() / 2).unwrap_or(u16::MAX));

    let bbox = if min_x <= max_x { [min_x, min_y, max_x, max_y] } else { [0, -200, 955, 720] };
    let extents = HheaExtents {
        advance_max: adv_max,
        min_lsb: if min_lsb == i16::MAX { 0 } else { min_lsb },
        min_rsb: if min_rsb == i16::MAX { 0 } else { min_rsb },
        x_max_extent: if x_max_extent == i16::MIN { bbox[2] } else { x_max_extent },
    };
    (glyf, loca, bbox, extents)
}

fn build_hmtx_table(glyphs: &[MinimalGlyphSpec]) -> Vec<u8> {
    let mut hmtx = Vec::with_capacity(glyphs.len() * 4);
    for g in glyphs {
        push_u16_be(&mut hmtx, g.advance);
        push_i16_be(&mut hmtx, g.lsb);
    }
    hmtx
}

fn build_head_table(bbox: [i16; 4]) -> Vec<u8> {
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes()); // major=1, minor=0
    head[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes()); // fontRevision 1.0
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes()); // magicNumber
    head[16..18].copy_from_slice(&0x000Bu16.to_be_bytes()); // flags
    head[18..20].copy_from_slice(&1000u16.to_be_bytes()); // unitsPerEm
    head[36..38].copy_from_slice(&bbox[0].to_be_bytes());
    head[38..40].copy_from_slice(&bbox[1].to_be_bytes());
    head[40..42].copy_from_slice(&bbox[2].to_be_bytes());
    head[42..44].copy_from_slice(&bbox[3].to_be_bytes());
    head[46..48].copy_from_slice(&8u16.to_be_bytes()); // lowestRecPPEM
    head[48..50].copy_from_slice(&2i16.to_be_bytes()); // fontDirectionHint
    head
}

fn build_hhea_table(num_glyphs: u16, ext: HheaExtents) -> Vec<u8> {
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[8..10].copy_from_slice(&100i16.to_be_bytes());
    hhea[10..12].copy_from_slice(&ext.advance_max.to_be_bytes());
    hhea[12..14].copy_from_slice(&ext.min_lsb.to_be_bytes());
    hhea[14..16].copy_from_slice(&ext.min_rsb.to_be_bytes());
    hhea[16..18].copy_from_slice(&ext.x_max_extent.to_be_bytes());
    hhea[18..20].copy_from_slice(&1i16.to_be_bytes()); // caretSlopeRise
    hhea[34..36].copy_from_slice(&num_glyphs.to_be_bytes());
    hhea
}

fn build_maxp_table(num_glyphs: u16) -> Vec<u8> {
    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes()); // v1.0
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
    maxp[6..8].copy_from_slice(&4u16.to_be_bytes()); // maxPoints
    maxp[8..10].copy_from_slice(&1u16.to_be_bytes()); // maxContours
    maxp[28..30].copy_from_slice(&2u16.to_be_bytes()); // maxSizeOfInstructions
    maxp
}

fn build_os2_table(unicode_range_1: u32, first_char: u16, last_char: u16) -> Vec<u8> {
    let mut os2 = vec![0u8; 96];
    os2[0..2].copy_from_slice(&4u16.to_be_bytes()); // version 4
    os2[2..4].copy_from_slice(&556i16.to_be_bytes()); // xAvgCharWidth
    os2[4..6].copy_from_slice(&400u16.to_be_bytes()); // usWeightClass
    os2[6..8].copy_from_slice(&5u16.to_be_bytes()); // usWidthClass
    os2[10..12].copy_from_slice(&650i16.to_be_bytes());
    os2[12..14].copy_from_slice(&600i16.to_be_bytes());
    os2[16..18].copy_from_slice(&140i16.to_be_bytes());
    os2[18..20].copy_from_slice(&650i16.to_be_bytes());
    os2[20..22].copy_from_slice(&600i16.to_be_bytes());
    os2[24..26].copy_from_slice(&480i16.to_be_bytes());
    os2[26..28].copy_from_slice(&50i16.to_be_bytes());
    os2[28..30].copy_from_slice(&260i16.to_be_bytes());
    os2[30..32].copy_from_slice(&0x0800i16.to_be_bytes()); // Sans-serif
    os2[32..42].copy_from_slice(&[2, 11, 5, 2, 2, 2, 2, 2, 2, 4]); // PANOSE
    os2[42..46].copy_from_slice(&unicode_range_1.to_be_bytes());
    os2[58..62].copy_from_slice(b"LCFT");
    os2[62..64].copy_from_slice(&0x0040u16.to_be_bytes()); // REGULAR
    os2[64..66].copy_from_slice(&first_char.to_be_bytes()); // usFirstCharIndex
    os2[66..68].copy_from_slice(&last_char.to_be_bytes()); // usLastCharIndex
    os2[68..70].copy_from_slice(&800i16.to_be_bytes());
    os2[70..72].copy_from_slice(&(-200i16).to_be_bytes());
    os2[72..74].copy_from_slice(&100i16.to_be_bytes());
    os2[74..76].copy_from_slice(&850u16.to_be_bytes());
    os2[76..78].copy_from_slice(&220u16.to_be_bytes());
    os2[78..82].copy_from_slice(&1u32.to_be_bytes()); // Latin-1 codepage
    os2[86..88].copy_from_slice(&520i16.to_be_bytes()); // sxHeight
    os2[88..90].copy_from_slice(&720i16.to_be_bytes()); // sCapHeight
    os2[92..94].copy_from_slice(&32u16.to_be_bytes()); // usBreakChar
    os2
}

fn build_minimal_cmap_table() -> Vec<u8> {
    // 7 sorted non-overlapping segments:
    // (32..=126)->gid 1..=95, (160..=255)->gid 96..=191, Δ->192, α..=γ->193..=195, μ->196, σ->197, 0xFFFF->0
    const SEGMENTS: [(u16, u16, i16); 7] = [
        (32, 126, -31),
        (160, 255, -64),
        (0x0394, 0x0394, -724),
        (0x03B1, 0x03B3, -752),
        (0x03BC, 0x03BC, -760),
        (0x03C3, 0x03C3, -766),
        (0xFFFF, 0xFFFF, 1),
    ];
    build_bmp_cmap_table(&SEGMENTS)
}

fn build_bmp_cmap_table(segments: &[(u16, u16, i16)]) -> Vec<u8> {
    let seg_count = u16::try_from(segments.len()).unwrap_or(1);
    let max_pow2 = 1u16 << (15u32.saturating_sub(seg_count.leading_zeros()));
    let search_range = max_pow2 * 2;
    let entry_selector = u16::try_from(max_pow2.trailing_zeros()).unwrap_or(0);
    let range_shift = seg_count * 2 - search_range;

    let mut sub = Vec::new();
    let sub_len = 16u16 + seg_count * 8;
    push_u16_be(&mut sub, 4); // format
    push_u16_be(&mut sub, sub_len);
    push_u16_be(&mut sub, 0); // language
    push_u16_be(&mut sub, seg_count * 2);
    push_u16_be(&mut sub, search_range);
    push_u16_be(&mut sub, entry_selector);
    push_u16_be(&mut sub, range_shift);
    for &(_, end, _) in segments {
        push_u16_be(&mut sub, end);
    }
    push_u16_be(&mut sub, 0); // reservedPad
    for &(start, _, _) in segments {
        push_u16_be(&mut sub, start);
    }
    for &(_, _, delta) in segments {
        push_i16_be(&mut sub, delta);
    }
    for _ in 0..segments.len() {
        push_u16_be(&mut sub, 0);
    }

    let header_len = 4u32 + 2 * 8;
    let mut cmap = Vec::with_capacity(header_len as usize + sub.len());
    push_u16_be(&mut cmap, 0); // version
    push_u16_be(&mut cmap, 2); // numTables
    push_u16_be(&mut cmap, 0);
    push_u16_be(&mut cmap, 3);
    push_u32_be(&mut cmap, header_len);
    push_u16_be(&mut cmap, 3);
    push_u16_be(&mut cmap, 1);
    push_u32_be(&mut cmap, header_len);
    cmap.extend_from_slice(&sub);
    cmap
}

fn build_sfnt_name_table(family: &str, style: &str, unique_id: &str, full_name: &str, ps_name: &str) -> Vec<u8> {
    let entries: [(u16, &str); 5] = [(1, family), (2, style), (3, unique_id), (4, full_name), (6, ps_name)];
    let mut str_buf = Vec::new();
    let mut records = Vec::new();
    for (name_id, text) in entries {
        let start = str_buf.len() as u16;
        for unit in text.encode_utf16() {
            push_u16_be(&mut str_buf, unit);
        }
        let len = (str_buf.len() as u16) - start;
        records.push((name_id, len, start));
    }

    let string_offset = 6u16 + (records.len() as u16) * 12;
    let mut name = Vec::with_capacity(usize::from(string_offset) + str_buf.len());
    push_u16_be(&mut name, 0); // format
    push_u16_be(&mut name, records.len() as u16);
    push_u16_be(&mut name, string_offset);
    for (name_id, len, offset) in records {
        push_u16_be(&mut name, 3); // Windows
        push_u16_be(&mut name, 1); // Unicode BMP
        push_u16_be(&mut name, 0x0409); // en-US
        push_u16_be(&mut name, name_id);
        push_u16_be(&mut name, len);
        push_u16_be(&mut name, offset);
    }
    name.extend_from_slice(&str_buf);
    name
}

fn build_minimal_post_table() -> Vec<u8> {
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes()); // format 3.0
    post[8..10].copy_from_slice(&(-100i16).to_be_bytes()); // underlinePosition
    post[10..12].copy_from_slice(&50i16.to_be_bytes()); // underlineThickness
    post
}

fn assemble_sfnt_tables(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let num_tables = tables.len() as u16;
    let max_pow2 = 8u16; // largest power of 2 <= 10
    let search_range = max_pow2 * 16;
    let entry_selector = 3u16;
    let range_shift = num_tables * 16 - search_range;

    let dir_size = 12 + tables.len() * 16;
    let mut font = Vec::with_capacity(dir_size + tables.iter().map(|(_, d)| d.len() + 3).sum::<usize>());
    push_u32_be(&mut font, 0x0001_0000);
    push_u16_be(&mut font, num_tables);
    push_u16_be(&mut font, search_range);
    push_u16_be(&mut font, entry_selector);
    push_u16_be(&mut font, range_shift);

    let mut cur_offset = dir_size;
    let mut head_offset = 0usize;
    for (tag, data) in tables {
        if tag == b"head" {
            head_offset = cur_offset;
        }
        font.extend_from_slice(tag);
        push_u32_be(&mut font, sfnt_checksum(data));
        push_u32_be(&mut font, cur_offset as u32);
        push_u32_be(&mut font, data.len() as u32);
        cur_offset += (data.len() + 3) & !3;
    }
    for (_, data) in tables {
        font.extend_from_slice(data);
        while !font.len().is_multiple_of(4) {
            font.push(0);
        }
    }

    if head_offset > 0 {
        let total = sfnt_checksum(&font);
        let adj = 0xB1B0_AFBAu32.wrapping_sub(total);
        if let Some(slot) = font.get_mut(head_offset + 8..head_offset + 12) {
            slot.copy_from_slice(&adj.to_be_bytes());
        }
    }
    font
}

fn sfnt_checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    let (chunks, rem) = data.as_chunks::<4>();
    for &c in chunks {
        sum = sum.wrapping_add(u32::from_be_bytes(c));
    }
    if !rem.is_empty() {
        let mut tail = [0u8; 4];
        for (dst, &src) in tail.iter_mut().zip(rem.iter()) {
            *dst = src;
        }
        sum = sum.wrapping_add(u32::from_be_bytes(tail));
    }
    sum
}

fn push_u16_be(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_i16_be(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_u32_be(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

#[cfg(test)]
mod symbol_fallback_tests {
    use super::{TrueTypeFont, build_minimal_truetype_font, build_symbol_fallback_truetype_font};

    #[test]
    fn symbol_fallback_sfnt_maps_all_mandatory_r15_1_symbols() {
        let bytes = build_symbol_fallback_truetype_font();
        let parsed = TrueTypeFont::parse(&bytes).expect("parse symbol fallback SFNT");
        for ch in ['→', '▾', '●', '✓', '±', '≤', '≥', 'μ', 'χ', '²'] {
            let gid = parsed.glyph_id(ch);
            assert!(gid.is_some(), "missing glyph for mandatory symbol {ch:?} (U+{:04X})", u32::from(ch));
            assert!(parsed.advance_em(ch).is_some(), "missing advance width for {ch:?}");
        }

        // Preserve build_minimal_truetype_font contract: lacks '▾' (U+25BE) and '≤' (U+2264).
        let minimal = build_minimal_truetype_font();
        let min_parsed = TrueTypeFont::parse(&minimal).expect("parse minimal SFNT");
        assert!(min_parsed.glyph_id('A').is_some());
        assert!(min_parsed.glyph_id('▾').is_none());
        assert!(min_parsed.glyph_id('≤').is_none());
    }
}
