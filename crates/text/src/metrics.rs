//! Deterministic PostScript-point text layout metrics (`1 pt = 1/72 inch`).
//!
//! Provides clean-room Adobe Core-14 Helvetica (Regular/Bold/Oblique/BoldOblique) and Symbol
//! advance-width tables when `BUNDLED_FONTS` is empty, plus a safe zero-panic SFNT/TrueType
//! (`head`/`hhea`/`maxp`/`cmap`/`hmtx`/`loca`/`glyf`) metric and glyph-slice reader when bundled
//! font faces are active.

use std::sync::OnceLock;

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

/// Reorder `rich` into visual left-to-right span order while preserving span styles and character count.
#[must_use]
pub fn reorder_rich_text_bidi(rich: &RichText) -> RichText {
    let base_dir = infer_base_direction(rich);
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

/// Safe, zero-allocation SFNT/TrueType font metric & glyph slice reader.
#[derive(Debug, Clone, Copy)]
pub struct TrueTypeFont<'a> {
    head: &'a [u8],
    hhea: &'a [u8],
    cmap: &'a [u8],
    hmtx: &'a [u8],
    loca: Option<&'a [u8]>,
    glyf: Option<&'a [u8]>,
    units_per_em: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    num_glyphs: u16,
    num_h_metrics: u16,
    index_to_loc_format: i16,
    cmap_subtable_offset: usize,
    cmap_format: u16,
}

impl<'a> TrueTypeFont<'a> {
    /// Parse an SFNT/TrueType font byte slice (`head`, `hhea`, `maxp`, `cmap`, `hmtx`, plus optional
    /// `loca`/`glyf`), returning `None` if malformed or missing required tables.
    #[must_use]
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        Self::parse_at(bytes, 0)
    }

    /// Parse an SFNT/TrueType font face starting at `sfnt_offset` inside `bytes` (used both for
    /// standalone SFNT files at `0` and individual faces inside a `.ttc` TrueType collection).
    #[must_use]
    pub fn parse_at(bytes: &'a [u8], sfnt_offset: usize) -> Option<Self> {
        let header = bytes.get(sfnt_offset..sfnt_offset.checked_add(12)?)?;
        let version = header.get(0..4)?;
        if version != [0x00, 0x01, 0x00, 0x00] && version != b"true" && version != b"OTTO" {
            return None;
        }
        let num_tables = read_u16_be(bytes, sfnt_offset.checked_add(4)?)?;
        if num_tables == 0 || num_tables > 128 {
            return None;
        }

        let head = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"head")?;
        let hhea = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"hhea")?;
        let maxp = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"maxp")?;
        let cmap = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"cmap")?;
        let hmtx = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"hmtx")?;
        let loca = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"loca");
        let glyf = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"glyf");

        if head.len() < 54 || hhea.len() < 36 || maxp.len() < 6 || cmap.len() < 4 {
            return None;
        }
        let units_per_em = read_u16_be(head, 18)?;
        if !(16..=16384).contains(&units_per_em) {
            return None;
        }
        let index_to_loc_format = read_i16_be(head, 50)?;
        let ascent = read_i16_be(hhea, 4)?;
        let descent = read_i16_be(hhea, 6)?;
        let line_gap = read_i16_be(hhea, 8)?;
        let num_h_metrics = read_u16_be(hhea, 34)?;
        let num_glyphs = read_u16_be(maxp, 4)?;
        if num_glyphs == 0 || num_h_metrics == 0 || num_h_metrics > num_glyphs {
            return None;
        }
        if hmtx.len() < usize::from(num_h_metrics) * 4 {
            return None;
        }
        let (cmap_subtable_offset, cmap_format) = select_cmap_subtable(cmap)?;

        Some(Self {
            head,
            hhea,
            cmap,
            hmtx,
            loca,
            glyf,
            units_per_em,
            ascent,
            descent,
            line_gap,
            num_glyphs,
            num_h_metrics,
            index_to_loc_format,
            cmap_subtable_offset,
            cmap_format,
        })
    }

    /// Parse a single SFNT or TrueType Collection (`.ttc`) byte slice into `(sfnt_offset, font)` pairs.
    #[must_use]
    pub fn parse_collection(bytes: &'a [u8]) -> Vec<(usize, Self)> {
        if bytes.get(0..4) == Some(b"ttcf") {
            let Some(num_fonts_raw) = read_u32_be(bytes, 8) else {
                return Vec::new();
            };
            let count = usize::try_from(num_fonts_raw).unwrap_or(0).min(64);
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let Some(off) = read_u32_be(bytes, 12 + i * 4).and_then(|v| usize::try_from(v).ok()) else {
                    break;
                };
                if let Some(font) = Self::parse_at(bytes, off) {
                    out.push((off, font));
                }
            }
            return out;
        }
        Self::parse_at(bytes, 0).map_or_else(Vec::new, |font| vec![(0, font)])
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
        self.loca.is_some() && self.glyf.is_some()
    }

    /// Map Unicode scalar value `ch` to a non-zero TrueType glyph index via `cmap`.
    #[must_use]
    pub fn glyph_id(&self, ch: char) -> Option<u16> {
        let sub = self.cmap.get(self.cmap_subtable_offset..)?;
        let gid = match self.cmap_format {
            12 => lookup_cmap_format12(sub, u32::from(ch))?,
            4 => {
                let code = u16::try_from(u32::from(ch)).ok()?;
                lookup_cmap_format4(sub, code)?
            }
            0 => {
                let code = u8::try_from(u32::from(ch)).ok()?;
                u16::from(*sub.get(6 + usize::from(code))?)
            }
            _ => return None,
        };
        (gid != 0 && gid < self.num_glyphs).then_some(gid)
    }

    /// Read raw advance width in font design units from `hmtx` for `glyph_id`.
    #[must_use]
    pub fn glyph_advance_units(&self, glyph_id: u16) -> Option<u16> {
        if glyph_id >= self.num_glyphs {
            return None;
        }
        let metric_idx = glyph_id.min(self.num_h_metrics.saturating_sub(1));
        read_u16_be(self.hmtx, usize::from(metric_idx) * 4)
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
        let _ = (self.head, self.hhea);
        let loca = self.loca?;
        let glyf = self.glyf?;
        let idx = usize::from(glyph_id);
        let (start, end) = match self.index_to_loc_format {
            0 => {
                let s = usize::from(read_u16_be(loca, idx.checked_mul(2)?)?) * 2;
                let e = usize::from(read_u16_be(loca, (idx + 1).checked_mul(2)?)?) * 2;
                (s, e)
            }
            1 => {
                let s = usize::try_from(read_u32_be(loca, idx.checked_mul(4)?)?).ok()?;
                let e = usize::try_from(read_u32_be(loca, (idx + 1).checked_mul(4)?)?).ok()?;
                (s, e)
            }
            _ => return None,
        };
        if end < start || end > glyf.len() {
            return None;
        }
        glyf.get(start..end)
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

pub(crate) fn inspect_sfnt_face_meta_at(bytes: &[u8], sfnt_offset: usize, fallback_name: &str) -> SfntFaceMeta {
    let Some(num_tables) = read_u16_be(bytes, sfnt_offset.saturating_add(4)).filter(|&n| n > 0 && n <= 128) else {
        return SfntFaceMeta::default();
    };
    let head = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"head");
    let os2 = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"OS/2").and_then(|t| parse_os2_table(t).ok());
    let post = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"post").and_then(|t| parse_post_table(t).ok());
    let parsed_names = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"name").and_then(|t| parse_name_table(t).ok());
    let is_variable = find_sfnt_table_at(bytes, sfnt_offset, num_tables, *b"fvar").is_some();

    let mac_style = head.and_then(|h| read_u16_be(h, 44)).unwrap_or(0);
    let family_name = parsed_names.map(|(fam, ps)| if !fam.is_empty() { fam } else { ps }).unwrap_or_default();
    let combined_label = format!("{} {}", family_name.to_ascii_lowercase(), fallback_name.to_ascii_lowercase());

    let is_bold =
        os2.is_some_and(|m| m.weight_class >= 600 || (m.fs_selection & 0x0020) != 0) || (mac_style & 0x0001) != 0 || combined_label.contains("bold");
    let is_italic = os2.is_some_and(|m| (m.fs_selection & 0x0001) != 0)
        || (mac_style & 0x0002) != 0
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
        && (os2.is_some_and(|m| {
            let class_id = (m.family_class >> 8) & 0xFF;
            (1..=7).contains(&class_id)
        }) || combined_label.contains("serif")
            || combined_label.contains("times")
            || combined_label.contains("new york")
            || combined_label.contains("newyork")
            || combined_label.contains("georgia")
            || combined_label.contains("cambria"));

    SfntFaceMeta { family_name, is_bold, is_italic, is_fixed_pitch, is_serif, is_variable }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CachedSfntLayout {
    head: (usize, usize),
    hhea: (usize, usize),
    cmap: (usize, usize),
    hmtx: (usize, usize),
    loca: Option<(usize, usize)>,
    glyf: Option<(usize, usize)>,
    units_per_em: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    num_glyphs: u16,
    num_h_metrics: u16,
    index_to_loc_format: i16,
    cmap_subtable_offset: usize,
    cmap_format: u16,
}

impl CachedSfntLayout {
    pub(crate) fn extract(bytes: &[u8], sfnt_offset: usize) -> Option<Self> {
        let header = bytes.get(sfnt_offset..sfnt_offset.checked_add(12)?)?;
        let version = header.get(0..4)?;
        if version != [0x00, 0x01, 0x00, 0x00] && version != b"true" && version != b"OTTO" {
            return None;
        }
        let num_tables = read_u16_be(bytes, sfnt_offset.checked_add(4)?)?;
        if num_tables == 0 || num_tables > 128 {
            return None;
        }
        let head_r = find_sfnt_table_range_at(bytes, sfnt_offset, num_tables, *b"head")?;
        let hhea_r = find_sfnt_table_range_at(bytes, sfnt_offset, num_tables, *b"hhea")?;
        let maxp_r = find_sfnt_table_range_at(bytes, sfnt_offset, num_tables, *b"maxp")?;
        let cmap_r = find_sfnt_table_range_at(bytes, sfnt_offset, num_tables, *b"cmap")?;
        let hmtx_r = find_sfnt_table_range_at(bytes, sfnt_offset, num_tables, *b"hmtx")?;
        let loca_r = find_sfnt_table_range_at(bytes, sfnt_offset, num_tables, *b"loca");
        let glyf_r = find_sfnt_table_range_at(bytes, sfnt_offset, num_tables, *b"glyf");

        let head = bytes.get(head_r.0..head_r.1)?;
        let hhea = bytes.get(hhea_r.0..hhea_r.1)?;
        let maxp = bytes.get(maxp_r.0..maxp_r.1)?;
        let cmap = bytes.get(cmap_r.0..cmap_r.1)?;
        let hmtx = bytes.get(hmtx_r.0..hmtx_r.1)?;
        if head.len() < 54 || hhea.len() < 36 || maxp.len() < 6 || cmap.len() < 4 {
            return None;
        }
        let units_per_em = read_u16_be(head, 18)?;
        if !(16..=16384).contains(&units_per_em) {
            return None;
        }
        let index_to_loc_format = read_i16_be(head, 50)?;
        let ascent = read_i16_be(hhea, 4)?;
        let descent = read_i16_be(hhea, 6)?;
        let line_gap = read_i16_be(hhea, 8)?;
        let num_h_metrics = read_u16_be(hhea, 34)?;
        let num_glyphs = read_u16_be(maxp, 4)?;
        if num_glyphs == 0 || num_h_metrics == 0 || num_h_metrics > num_glyphs || hmtx.len() < usize::from(num_h_metrics) * 4 {
            return None;
        }
        let (cmap_subtable_offset, cmap_format) = select_cmap_subtable(cmap)?;
        Some(Self {
            head: head_r,
            hhea: hhea_r,
            cmap: cmap_r,
            hmtx: hmtx_r,
            loca: loca_r,
            glyf: glyf_r,
            units_per_em,
            ascent,
            descent,
            line_gap,
            num_glyphs,
            num_h_metrics,
            index_to_loc_format,
            cmap_subtable_offset,
            cmap_format,
        })
    }

    pub(crate) fn font<'a>(&self, bytes: &'a [u8]) -> Option<TrueTypeFont<'a>> {
        Some(TrueTypeFont {
            head: bytes.get(self.head.0..self.head.1)?,
            hhea: bytes.get(self.hhea.0..self.hhea.1)?,
            cmap: bytes.get(self.cmap.0..self.cmap.1)?,
            hmtx: bytes.get(self.hmtx.0..self.hmtx.1)?,
            loca: self.loca.and_then(|(s, e)| bytes.get(s..e)),
            glyf: self.glyf.and_then(|(s, e)| bytes.get(s..e)),
            units_per_em: self.units_per_em,
            ascent: self.ascent,
            descent: self.descent,
            line_gap: self.line_gap,
            num_glyphs: self.num_glyphs,
            num_h_metrics: self.num_h_metrics,
            index_to_loc_format: self.index_to_loc_format,
            cmap_subtable_offset: self.cmap_subtable_offset,
            cmap_format: self.cmap_format,
        })
    }
}

fn find_sfnt_table(bytes: &[u8], num_tables: u16, target: [u8; 4]) -> Option<&[u8]> {
    find_sfnt_table_at(bytes, 0, num_tables, target)
}

fn find_sfnt_table_at(bytes: &[u8], sfnt_offset: usize, num_tables: u16, target: [u8; 4]) -> Option<&[u8]> {
    let (start, end) = find_sfnt_table_range_at(bytes, sfnt_offset, num_tables, target)?;
    bytes.get(start..end)
}

fn find_sfnt_table_range_at(bytes: &[u8], sfnt_offset: usize, num_tables: u16, target: [u8; 4]) -> Option<(usize, usize)> {
    let dir_base = sfnt_offset.checked_add(12)?;
    for i in 0..usize::from(num_tables) {
        let rec_off = dir_base.checked_add(i.checked_mul(16)?)?;
        let tag = bytes.get(rec_off..rec_off + 4)?;
        if tag == target {
            let offset = usize::try_from(read_u32_be(bytes, rec_off + 8)?).ok()?;
            let length = usize::try_from(read_u32_be(bytes, rec_off + 12)?).ok()?;
            let end = offset.checked_add(length)?;
            if end <= bytes.len() {
                return Some((offset, end));
            }
            return None;
        }
    }
    None
}

fn select_cmap_subtable(cmap: &[u8]) -> Option<(usize, u16)> {
    let num_subtables = read_u16_be(cmap, 2)?.min(64);
    let mut best: Option<(i32, usize, u16)> = None;
    for i in 0..usize::from(num_subtables) {
        let rec_off = 4usize.checked_add(i.checked_mul(8)?)?;
        let platform = read_u16_be(cmap, rec_off)?;
        let encoding = read_u16_be(cmap, rec_off + 2)?;
        let sub_off = usize::try_from(read_u32_be(cmap, rec_off + 4)?).ok()?;
        let fmt = read_u16_be(cmap, sub_off)?;
        if !matches!(fmt, 0 | 4 | 12) {
            continue;
        }
        let rank = match (platform, encoding, fmt) {
            (3, 10, 12) => 100,
            (0, 4, 12) => 95,
            (3, 1, 4) => 90,
            (0, 3, 4) => 85,
            (0, _, 4) => 80,
            (0, _, 12) => 75,
            (1, 0, 0) => 20,
            _ => 10,
        };
        if best.is_none_or(|(b_rank, _, _)| rank > b_rank) {
            best = Some((rank, sub_off, fmt));
        }
    }
    best.map(|(_, off, fmt)| (off, fmt))
}

fn lookup_cmap_format4(sub: &[u8], code: u16) -> Option<u16> {
    if sub.len() < 16 {
        return None;
    }
    let seg_count_x2 = read_u16_be(sub, 6)?;
    if seg_count_x2 < 2 || !seg_count_x2.is_multiple_of(2) {
        return None;
    }
    let seg_count = usize::from(seg_count_x2 / 2);
    let end_off = 14usize;
    let start_off = end_off.checked_add(seg_count.checked_mul(2)?)?.checked_add(2)?;
    let delta_off = start_off.checked_add(seg_count.checked_mul(2)?)?;
    let range_off = delta_off.checked_add(seg_count.checked_mul(2)?)?;
    if range_off.checked_add(seg_count.checked_mul(2)?)? > sub.len() {
        return None;
    }

    let mut lo = 0usize;
    let mut hi = seg_count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let end_code = read_u16_be(sub, end_off + mid * 2)?;
        if end_code < code {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo >= seg_count {
        return None;
    }

    let start_code = read_u16_be(sub, start_off + lo * 2)?;
    if code < start_code {
        return None;
    }
    let id_delta = read_i16_be(sub, delta_off + lo * 2)?;
    let id_range_offset = read_u16_be(sub, range_off + lo * 2)?;
    if id_range_offset == 0 {
        let gid = ((i32::from(code) + i32::from(id_delta)) & 0xFFFF) as u16;
        return Some(gid);
    }

    let slot_off = range_off + lo * 2;
    let glyph_addr = slot_off.checked_add(usize::from(id_range_offset))?.checked_add(usize::from(code - start_code) * 2)?;
    let raw_gid = read_u16_be(sub, glyph_addr)?;
    if raw_gid == 0 {
        return Some(0);
    }
    let gid = ((i32::from(raw_gid) + i32::from(id_delta)) & 0xFFFF) as u16;
    Some(gid)
}

fn lookup_cmap_format12(sub: &[u8], code: u32) -> Option<u16> {
    if sub.len() < 16 {
        return None;
    }
    let num_groups = usize::try_from(read_u32_be(sub, 12)?).ok()?;
    let table_end = 16usize.checked_add(num_groups.checked_mul(12)?)?;
    if table_end > sub.len() {
        return None;
    }

    let mut lo = 0usize;
    let mut hi = num_groups;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let g_off = 16 + mid * 12;
        let end_code = read_u32_be(sub, g_off + 4)?;
        if end_code < code {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo >= num_groups {
        return None;
    }

    let g_off = 16 + lo * 12;
    let start_code = read_u32_be(sub, g_off)?;
    let end_code = read_u32_be(sub, g_off + 4)?;
    let start_gid = read_u32_be(sub, g_off + 8)?;
    if code < start_code || code > end_code {
        return None;
    }
    let gid = start_gid.checked_add(code - start_code)?;
    u16::try_from(gid).ok()
}

pub(crate) fn read_u16_be(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let arr: [u8; 2] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u16::from_be_bytes(arr))
}

pub(crate) fn read_i16_be(bytes: &[u8], offset: usize) -> Option<i16> {
    let end = offset.checked_add(2)?;
    let arr: [u8; 2] = bytes.get(offset..end)?.try_into().ok()?;
    Some(i16::from_be_bytes(arr))
}

pub(crate) fn read_u32_be(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let arr: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u32::from_be_bytes(arr))
}

pub(crate) fn read_i32_be(bytes: &[u8], offset: usize) -> Option<i32> {
    let end = offset.checked_add(4)?;
    let arr: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(i32::from_be_bytes(arr))
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
        if let Some(code) = char_to_win1252_byte(ch)
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
    let num_tables = validate_sfnt_directory(sfnt_bytes)?;

    let head = require_sfnt_table(sfnt_bytes, num_tables, *b"head")?;
    let hhea = require_sfnt_table(sfnt_bytes, num_tables, *b"hhea")?;
    let maxp = require_sfnt_table(sfnt_bytes, num_tables, *b"maxp")?;
    let hmtx = require_sfnt_table(sfnt_bytes, num_tables, *b"hmtx")?;
    let cmap = require_sfnt_table(sfnt_bytes, num_tables, *b"cmap")?;

    let os2 = find_sfnt_table(sfnt_bytes, num_tables, *b"OS/2");
    let post = find_sfnt_table(sfnt_bytes, num_tables, *b"post");
    let name = find_sfnt_table(sfnt_bytes, num_tables, *b"name");

    let head_metrics = parse_head_table(head)?;
    let hhea_metrics = parse_hhea_table(hhea)?;
    let num_glyphs = parse_maxp_num_glyphs(maxp)?;

    if hhea_metrics.num_h_metrics == 0 || hhea_metrics.num_h_metrics > num_glyphs {
        return Err(invalid_sfnt("hhea numberOfHMetrics out of range for maxp numGlyphs"));
    }
    let min_hmtx_bytes = usize::from(hhea_metrics.num_h_metrics) * 4;
    if hmtx.len() < min_hmtx_bytes {
        return Err(invalid_sfnt("hmtx table shorter than numberOfHMetrics * 4"));
    }

    let os2_metrics = os2.map(parse_os2_table).transpose()?;
    let post_metrics = post.map(parse_post_table).transpose()?;
    let (parsed_family, parsed_ps) = name.map(parse_name_table).transpose()?.unwrap_or_default();
    let cmap_basic = parse_cmap_mappings(cmap, num_glyphs)?;

    let units_per_em = head_metrics.units_per_em;
    let raw_asc = if hhea_metrics.ascender != 0 { hhea_metrics.ascender } else { os2_metrics.map_or(800, |m| m.typo_ascender) };
    let raw_desc = if hhea_metrics.descender != 0 { hhea_metrics.descender } else { os2_metrics.map_or(-200, |m| m.typo_descender) };

    let ascender_1000 = scale_font_units_1000(i32::from(raw_asc), units_per_em);
    let descender_1000 = scale_font_units_1000(i32::from(raw_desc), units_per_em);
    let cap_height_1000 = os2_metrics
        .and_then(|m| m.cap_height)
        .filter(|&c| c > 0)
        .map_or_else(|| (ascender_1000 * 7 / 8).max(1), |c| scale_font_units_1000(i32::from(c), units_per_em));

    let is_italic_flag = os2_metrics.is_some_and(|m| (m.fs_selection & 0x0001) != 0) || (head_metrics.mac_style & 0x0002) != 0;
    let italic_angle = post_metrics.map_or(if is_italic_flag { -12.0 } else { 0.0 }, |p| p.italic_angle);

    let is_fixed_pitch = post_metrics.is_some_and(|p| p.is_fixed_pitch);
    let is_serif = os2_metrics.is_some_and(|m| {
        let class_id = (m.family_class >> 8) & 0xFF;
        (1..=7).contains(&class_id)
    });
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

    let weight_class = os2_metrics.map_or(if (head_metrics.mac_style & 0x0001) != 0 { 700 } else { 400 }, |m| m.weight_class);
    let stem_v = (10 + (i32::from(weight_class) / 100) * 18).clamp(30, 300);

    let bbox_1000 = [
        scale_font_units_1000(i32::from(head_metrics.x_min), units_per_em),
        scale_font_units_1000(i32::from(head_metrics.y_min), units_per_em),
        scale_font_units_1000(i32::from(head_metrics.x_max), units_per_em),
        scale_font_units_1000(i32::from(head_metrics.y_max), units_per_em),
    ];

    let widths_1000 = build_win1252_widths(hmtx, hhea_metrics.num_h_metrics, units_per_em, &cmap_basic)?;

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

fn validate_sfnt_directory(bytes: &[u8]) -> Result<u16, crate::TextError> {
    let version = bytes.get(0..4).ok_or_else(|| invalid_sfnt("SFNT header shorter than 12 bytes"))?;
    if version != [0x00, 0x01, 0x00, 0x00] && version != b"true" && version != b"OTTO" {
        return Err(invalid_sfnt("unrecognized SFNT version tag"));
    }
    if bytes.len() < 12 {
        return Err(invalid_sfnt("SFNT header truncated"));
    }
    let num_tables = read_u16_be(bytes, 4).ok_or_else(|| invalid_sfnt("missing numTables"))?;
    if num_tables == 0 || num_tables > 128 {
        return Err(invalid_sfnt("invalid SFNT numTables"));
    }
    let dir_end =
        usize::from(num_tables).checked_mul(16).and_then(|n| 12usize.checked_add(n)).ok_or_else(|| invalid_sfnt("SFNT directory size overflow"))?;
    if dir_end > bytes.len() {
        return Err(invalid_sfnt("SFNT table directory truncated"));
    }
    for i in 0..usize::from(num_tables) {
        let rec_off = 12 + i * 16;
        let offset = read_u32_be(bytes, rec_off + 8).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| invalid_sfnt("invalid table offset"))?;
        let length = read_u32_be(bytes, rec_off + 12).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| invalid_sfnt("invalid table length"))?;
        let end = offset.checked_add(length).ok_or_else(|| invalid_sfnt("table range overflow"))?;
        if end > bytes.len() {
            return Err(invalid_sfnt("SFNT table extends past EOF"));
        }
    }
    Ok(num_tables)
}

fn require_sfnt_table(bytes: &[u8], num_tables: u16, tag: [u8; 4]) -> Result<&[u8], crate::TextError> {
    find_sfnt_table(bytes, num_tables, tag).ok_or(crate::TextError::MissingSfntTable { tag })
}

struct HeadMetrics {
    units_per_em: u16,
    x_min: i16,
    y_min: i16,
    x_max: i16,
    y_max: i16,
    mac_style: u16,
}

fn parse_head_table(head: &[u8]) -> Result<HeadMetrics, crate::TextError> {
    if head.len() < 54 {
        return Err(invalid_sfnt("head table shorter than 54 bytes"));
    }
    let units_per_em = read_u16_be(head, 18).ok_or_else(|| invalid_sfnt("missing unitsPerEm"))?;
    if !(16..=16384).contains(&units_per_em) {
        return Err(invalid_sfnt("head unitsPerEm outside 16..=16384"));
    }
    let x_min = read_i16_be(head, 36).ok_or_else(|| invalid_sfnt("missing head xMin"))?;
    let y_min = read_i16_be(head, 38).ok_or_else(|| invalid_sfnt("missing head yMin"))?;
    let x_max = read_i16_be(head, 40).ok_or_else(|| invalid_sfnt("missing head xMax"))?;
    let y_max = read_i16_be(head, 42).ok_or_else(|| invalid_sfnt("missing head yMax"))?;
    let mac_style = read_u16_be(head, 44).ok_or_else(|| invalid_sfnt("missing head macStyle"))?;
    Ok(HeadMetrics { units_per_em, x_min, y_min, x_max, y_max, mac_style })
}

struct HheaMetrics {
    ascender: i16,
    descender: i16,
    num_h_metrics: u16,
}

fn parse_hhea_table(hhea: &[u8]) -> Result<HheaMetrics, crate::TextError> {
    if hhea.len() < 36 {
        return Err(invalid_sfnt("hhea table shorter than 36 bytes"));
    }
    let ascender = read_i16_be(hhea, 4).ok_or_else(|| invalid_sfnt("missing hhea ascender"))?;
    let descender = read_i16_be(hhea, 6).ok_or_else(|| invalid_sfnt("missing hhea descender"))?;
    let num_h_metrics = read_u16_be(hhea, 34).ok_or_else(|| invalid_sfnt("missing hhea numberOfHMetrics"))?;
    Ok(HheaMetrics { ascender, descender, num_h_metrics })
}

fn parse_maxp_num_glyphs(maxp: &[u8]) -> Result<u16, crate::TextError> {
    if maxp.len() < 6 {
        return Err(invalid_sfnt("maxp table shorter than 6 bytes"));
    }
    let num_glyphs = read_u16_be(maxp, 4).ok_or_else(|| invalid_sfnt("missing maxp numGlyphs"))?;
    if num_glyphs == 0 {
        return Err(invalid_sfnt("maxp numGlyphs is zero"));
    }
    Ok(num_glyphs)
}

#[derive(Clone, Copy)]
struct Os2Metrics {
    weight_class: u16,
    family_class: i16,
    fs_selection: u16,
    typo_ascender: i16,
    typo_descender: i16,
    cap_height: Option<i16>,
}

fn parse_os2_table(os2: &[u8]) -> Result<Os2Metrics, crate::TextError> {
    if os2.len() < 78 {
        return Err(invalid_sfnt("OS/2 table shorter than 78 bytes"));
    }
    let version = read_u16_be(os2, 0).ok_or_else(|| invalid_sfnt("missing OS/2 version"))?;
    if version >= 2 && os2.len() < 96 {
        return Err(invalid_sfnt("OS/2 v2+ table shorter than 96 bytes"));
    }
    let weight_class = read_u16_be(os2, 4).ok_or_else(|| invalid_sfnt("missing OS/2 usWeightClass"))?;
    let family_class = read_i16_be(os2, 30).ok_or_else(|| invalid_sfnt("missing OS/2 sFamilyClass"))?;
    let fs_selection = read_u16_be(os2, 62).ok_or_else(|| invalid_sfnt("missing OS/2 fsSelection"))?;
    let typo_ascender = read_i16_be(os2, 68).ok_or_else(|| invalid_sfnt("missing OS/2 sTypoAscender"))?;
    let typo_descender = read_i16_be(os2, 70).ok_or_else(|| invalid_sfnt("missing OS/2 sTypoDescender"))?;
    let cap_height = if version >= 2 { read_i16_be(os2, 88) } else { None };
    Ok(Os2Metrics { weight_class, family_class, fs_selection, typo_ascender, typo_descender, cap_height })
}

#[derive(Clone, Copy)]
struct PostMetrics {
    italic_angle: f64,
    is_fixed_pitch: bool,
}

fn parse_post_table(post: &[u8]) -> Result<PostMetrics, crate::TextError> {
    if post.len() < 32 {
        return Err(invalid_sfnt("post table shorter than 32 bytes"));
    }
    let italic_fixed = read_i32_be(post, 4).ok_or_else(|| invalid_sfnt("missing post italicAngle"))?;
    let is_fixed_pitch = read_u32_be(post, 12).ok_or_else(|| invalid_sfnt("missing post isFixedPitch"))? != 0;
    Ok(PostMetrics { italic_angle: f64::from(italic_fixed) / 65536.0, is_fixed_pitch })
}

fn parse_name_table(name: &[u8]) -> Result<(String, String), crate::TextError> {
    if name.len() < 6 {
        return Err(invalid_sfnt("name table shorter than 6 bytes"));
    }
    let format = read_u16_be(name, 0).ok_or_else(|| invalid_sfnt("missing name format"))?;
    if format > 1 {
        return Err(invalid_sfnt("unsupported name table format"));
    }
    let count = read_u16_be(name, 2).ok_or_else(|| invalid_sfnt("missing name count"))?;
    let str_base = usize::from(read_u16_be(name, 4).ok_or_else(|| invalid_sfnt("missing name stringOffset"))?);
    if str_base > name.len() {
        return Err(invalid_sfnt("name stringOffset past end of table"));
    }
    let recs_end = usize::from(count).checked_mul(12).and_then(|n| 6usize.checked_add(n)).ok_or_else(|| invalid_sfnt("name count overflow"))?;
    if recs_end > name.len() {
        return Err(invalid_sfnt("name records truncated"));
    }

    let mut best_family: Option<(i32, String)> = None;
    let mut best_ps: Option<(i32, String)> = None;

    for i in 0..usize::from(count) {
        let off = 6 + i * 12;
        let platform = read_u16_be(name, off).ok_or_else(|| invalid_sfnt("name record platformID"))?;
        let encoding = read_u16_be(name, off + 2).ok_or_else(|| invalid_sfnt("name record encodingID"))?;
        let lang = read_u16_be(name, off + 4).ok_or_else(|| invalid_sfnt("name record languageID"))?;
        let name_id = read_u16_be(name, off + 6).ok_or_else(|| invalid_sfnt("name record nameID"))?;
        let len = usize::from(read_u16_be(name, off + 8).ok_or_else(|| invalid_sfnt("name record length"))?);
        let str_off = usize::from(read_u16_be(name, off + 10).ok_or_else(|| invalid_sfnt("name record offset"))?);

        let start = str_base.checked_add(str_off).ok_or_else(|| invalid_sfnt("name string start overflow"))?;
        let end = start.checked_add(len).ok_or_else(|| invalid_sfnt("name string end overflow"))?;
        let slice = name.get(start..end).ok_or_else(|| invalid_sfnt("name string slice out of bounds"))?;

        if !matches!(name_id, 1 | 6 | 16) {
            continue;
        }
        let decoded = decode_name_string(slice, platform, encoding);
        let trimmed = decoded.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_owned();
        if trimmed.is_empty() {
            continue;
        }

        let base_rank = match (platform, lang) {
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

fn decode_name_string(bytes: &[u8], platform: u16, encoding: u16) -> String {
    if platform == 0 || (platform == 3 && matches!(encoding, 0 | 1 | 10)) {
        let (pairs, _) = bytes.as_chunks::<2>();
        let iter = pairs.iter().map(|&pair| u16::from_be_bytes(pair));
        return char::decode_utf16(iter).filter_map(Result::ok).collect();
    }
    bytes.iter().filter_map(|&b| (0x20..=0x7E).contains(&b).then_some(char::from(b))).collect()
}

fn parse_cmap_mappings(cmap: &[u8], num_glyphs: u16) -> Result<Vec<(u32, u16)>, crate::TextError> {
    if cmap.len() < 4 {
        return Err(invalid_sfnt("cmap table shorter than 4 bytes"));
    }
    let num_subtables = read_u16_be(cmap, 2).ok_or_else(|| invalid_sfnt("missing cmap numTables"))?;
    if num_subtables == 0 {
        return Err(crate::TextError::UnsupportedCmap { message: String::from("cmap has zero encoding records") });
    }
    let records_end =
        usize::from(num_subtables).checked_mul(8).and_then(|n| 4usize.checked_add(n)).ok_or_else(|| invalid_sfnt("cmap directory overflow"))?;
    if records_end > cmap.len() {
        return Err(invalid_sfnt("cmap encoding records truncated"));
    }

    let mut best_fmt4: Option<(i32, usize)> = None;
    let mut best_fmt12: Option<(i32, usize)> = None;
    let mut best_fmt0: Option<(i32, usize)> = None;

    for i in 0..usize::from(num_subtables) {
        let rec_off = 4 + i * 8;
        let platform = read_u16_be(cmap, rec_off).ok_or_else(|| invalid_sfnt("cmap platformID"))?;
        let encoding = read_u16_be(cmap, rec_off + 2).ok_or_else(|| invalid_sfnt("cmap encodingID"))?;
        let sub_off =
            read_u32_be(cmap, rec_off + 4).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| invalid_sfnt("cmap subtable offset overflow"))?;
        let fmt = read_u16_be(cmap, sub_off).ok_or_else(|| invalid_sfnt("cmap subtable offset out of bounds"))?;
        let rank = match (platform, encoding, fmt) {
            (3, 10, 12) => 100,
            (0, 4, 12) => 95,
            (3, 1, 4) => 90,
            (0, 3, 4) => 85,
            (0, _, 4) => 80,
            (0, _, 12) => 75,
            (1, 0, 0) => 20,
            _ => 10,
        };
        match fmt {
            4 if best_fmt4.is_none_or(|(r, _)| rank > r) => best_fmt4 = Some((rank, sub_off)),
            12 if best_fmt12.is_none_or(|(r, _)| rank > r) => best_fmt12 = Some((rank, sub_off)),
            0 if best_fmt0.is_none_or(|(r, _)| rank > r) => best_fmt0 = Some((rank, sub_off)),
            _ => {}
        }
    }

    let mut merged = Vec::new();
    if let Some((_, off)) = best_fmt4 {
        let sub = cmap.get(off..).ok_or_else(|| invalid_sfnt("cmap format 4 out of bounds"))?;
        collect_cmap_format4(sub, num_glyphs, &mut merged)?;
    }
    if let Some((_, off)) = best_fmt12 {
        let sub = cmap.get(off..).ok_or_else(|| invalid_sfnt("cmap format 12 out of bounds"))?;
        collect_cmap_format12(sub, num_glyphs, &mut merged)?;
    }
    if merged.is_empty()
        && let Some((_, off)) = best_fmt0
    {
        let sub = cmap.get(off..).ok_or_else(|| invalid_sfnt("cmap format 0 out of bounds"))?;
        collect_cmap_format0(sub, num_glyphs, &mut merged)?;
    }
    if best_fmt4.is_none() && best_fmt12.is_none() && best_fmt0.is_none() {
        return Err(crate::TextError::UnsupportedCmap { message: String::from("no format 0, 4, or 12 cmap subtable found") });
    }

    merged.sort_unstable_by_key(|&(cp, _)| cp);
    merged.dedup_by_key(|pair| pair.0);
    Ok(merged)
}

const MAX_CMAP4_INDIRECT_STEPS: usize = MAX_CMAP_COLLECT * 4;

fn collect_cmap_format4(sub: &[u8], num_glyphs: u16, out: &mut Vec<(u32, u16)>) -> Result<(), crate::TextError> {
    if sub.len() < 16 {
        return Err(invalid_sfnt("cmap format 4 subtable shorter than 16 bytes"));
    }
    let seg_count_x2 = read_u16_be(sub, 6).ok_or_else(|| invalid_sfnt("missing cmap4 segCountX2"))?;
    if seg_count_x2 < 2 || !seg_count_x2.is_multiple_of(2) {
        return Err(invalid_sfnt("invalid cmap4 segCountX2"));
    }
    let seg_count = usize::from(seg_count_x2 / 2);
    let end_off = 14usize;
    let start_off = end_off.checked_add(seg_count * 2).and_then(|n| n.checked_add(2)).ok_or_else(|| invalid_sfnt("cmap4 offset overflow"))?;
    let delta_off = start_off.checked_add(seg_count * 2).ok_or_else(|| invalid_sfnt("cmap4 delta overflow"))?;
    let range_off = delta_off.checked_add(seg_count * 2).ok_or_else(|| invalid_sfnt("cmap4 range overflow"))?;
    let min_len = range_off.checked_add(seg_count * 2).ok_or_else(|| invalid_sfnt("cmap4 size overflow"))?;
    if min_len > sub.len() {
        return Err(invalid_sfnt("cmap format 4 segments truncated"));
    }

    let mut indirect_steps = 0usize;
    for i in 0..seg_count {
        let end_code = read_u16_be(sub, end_off + i * 2).ok_or_else(|| invalid_sfnt("cmap4 endCode"))?;
        let start_code = read_u16_be(sub, start_off + i * 2).ok_or_else(|| invalid_sfnt("cmap4 startCode"))?;
        let id_delta = read_i16_be(sub, delta_off + i * 2).ok_or_else(|| invalid_sfnt("cmap4 idDelta"))?;
        let id_range_offset = read_u16_be(sub, range_off + i * 2).ok_or_else(|| invalid_sfnt("cmap4 idRangeOffset"))?;

        if start_code > end_code {
            return Err(invalid_sfnt("cmap4 segment has startCode > endCode"));
        }
        if start_code == 0xFFFF && end_code == 0xFFFF {
            continue;
        }
        if out.len() >= MAX_CMAP_COLLECT {
            return Ok(());
        }
        let seg_lo = start_code;
        let seg_hi = end_code.min(0xFFFE);
        if seg_lo > seg_hi {
            continue;
        }

        if id_range_offset == 0 {
            if num_glyphs <= 1 {
                continue;
            }
            let max_gid = num_glyphs - 1;
            let delta_u = u16::from_be_bytes(id_delta.to_be_bytes());
            let c_lo = 1u16.wrapping_sub(delta_u);
            let c_hi = max_gid.wrapping_sub(delta_u);
            let ranges = if c_lo <= c_hi { [(c_lo, c_hi), (1, 0)] } else { [(0, c_hi), (c_lo, 0xFFFE)] };
            for (r_lo, r_hi) in ranges {
                if r_lo > r_hi {
                    continue;
                }
                let lo = seg_lo.max(r_lo);
                let hi = seg_hi.min(r_hi);
                if lo > hi {
                    continue;
                }
                for code in lo..=hi {
                    if out.len() >= MAX_CMAP_COLLECT {
                        return Ok(());
                    }
                    let gid = code.wrapping_add_signed(id_delta);
                    out.push((u32::from(code), gid));
                }
            }
        } else {
            let slot_off = range_off + i * 2;
            let first_addr = slot_off.checked_add(usize::from(id_range_offset)).ok_or_else(|| invalid_sfnt("cmap4 glyphIdArray offset overflow"))?;
            let last_addr =
                first_addr.checked_add(usize::from(seg_hi - seg_lo) * 2).ok_or_else(|| invalid_sfnt("cmap4 glyphIdArray offset overflow"))?;
            if read_u16_be(sub, last_addr).is_none() {
                return Err(invalid_sfnt("cmap4 glyphIdArray out of bounds"));
            }
            if num_glyphs <= 1 {
                continue;
            }
            for code in seg_lo..=seg_hi {
                if out.len() >= MAX_CMAP_COLLECT || indirect_steps >= MAX_CMAP4_INDIRECT_STEPS {
                    return Ok(());
                }
                indirect_steps += 1;
                let glyph_addr = first_addr + usize::from(code - seg_lo) * 2;
                let raw = read_u16_be(sub, glyph_addr).ok_or_else(|| invalid_sfnt("cmap4 glyphIdArray out of bounds"))?;
                let gid = if raw == 0 { 0 } else { raw.wrapping_add_signed(id_delta) };
                if gid != 0 && gid < num_glyphs {
                    out.push((u32::from(code), gid));
                }
            }
        }
    }
    Ok(())
}

fn collect_cmap_format12(sub: &[u8], num_glyphs: u16, out: &mut Vec<(u32, u16)>) -> Result<(), crate::TextError> {
    if sub.len() < 16 {
        return Err(invalid_sfnt("cmap format 12 subtable shorter than 16 bytes"));
    }
    let num_groups = read_u32_be(sub, 12).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| invalid_sfnt("cmap12 numGroups overflow"))?;
    let table_end = num_groups.checked_mul(12).and_then(|n| 16usize.checked_add(n)).ok_or_else(|| invalid_sfnt("cmap12 size overflow"))?;
    if table_end > sub.len() {
        return Err(invalid_sfnt("cmap format 12 groups truncated"));
    }

    for i in 0..num_groups {
        let g_off = 16 + i * 12;
        let start_code = read_u32_be(sub, g_off).ok_or_else(|| invalid_sfnt("cmap12 startCharCode"))?;
        let end_code = read_u32_be(sub, g_off + 4).ok_or_else(|| invalid_sfnt("cmap12 endCharCode"))?;
        let start_gid = read_u32_be(sub, g_off + 8).ok_or_else(|| invalid_sfnt("cmap12 startGlyphID"))?;
        if start_code > end_code {
            return Err(invalid_sfnt("cmap12 group has startCharCode > endCharCode"));
        }
        let span = end_code - start_code;
        for delta in 0..=span {
            if out.len() >= MAX_CMAP_COLLECT {
                return Ok(());
            }
            let Some(gid_u32) = start_gid.checked_add(delta) else {
                break;
            };
            if gid_u32 >= u32::from(num_glyphs) {
                break;
            }
            if gid_u32 != 0
                && let Ok(gid) = u16::try_from(gid_u32)
            {
                out.push((start_code + delta, gid));
            }
        }
    }
    Ok(())
}

fn collect_cmap_format0(sub: &[u8], num_glyphs: u16, out: &mut Vec<(u32, u16)>) -> Result<(), crate::TextError> {
    if sub.len() < 262 {
        return Err(invalid_sfnt("cmap format 0 subtable shorter than 262 bytes"));
    }
    for code in 0usize..256 {
        let Some(&gid_byte) = sub.get(6 + code) else {
            return Err(invalid_sfnt("cmap format 0 truncated"));
        };
        let gid = u16::from(gid_byte);
        if gid != 0 && gid < num_glyphs {
            out.push((code as u32, gid));
        }
    }
    Ok(())
}

fn build_win1252_widths(hmtx: &[u8], num_h_metrics: u16, units_per_em: u16, cmap: &[(u32, u16)]) -> Result<Vec<u16>, crate::TextError> {
    let notdef_adv = hmtx_advance_for_gid(hmtx, num_h_metrics, 0)?;
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
        let adv = hmtx_advance_for_gid(hmtx, num_h_metrics, gid)?;
        let scaled = scale_font_units_1000(i32::from(adv), units_per_em).clamp(0, i32::from(u16::MAX));
        widths.push(u16::try_from(scaled).unwrap_or(notdef_1000));
    }
    Ok(widths)
}

fn hmtx_advance_for_gid(hmtx: &[u8], num_h_metrics: u16, gid: u16) -> Result<u16, crate::TextError> {
    let idx = gid.min(num_h_metrics.saturating_sub(1));
    read_u16_be(hmtx, usize::from(idx) * 4).ok_or_else(|| invalid_sfnt("hmtx advance offset out of bounds"))
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
