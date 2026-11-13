//! Deterministic PostScript-point text layout metrics (`1 pt = 1/72 inch`).
//!
//! Provides clean-room Adobe Core-14 Helvetica (Regular/Bold/Oblique/BoldOblique) and Symbol
//! advance-width tables when `BUNDLED_FONTS` is empty, plus a safe zero-panic SFNT/TrueType
//! (`head`/`hhea`/`maxp`/`cmap`/`hmtx`/`loca`/`glyf`) metric and glyph-slice reader when bundled
//! font faces are active.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::BUNDLED_FONTS;
use crate::markup::{RichSpan, RichText, SpanScript, SpanSlope, SpanWeight};

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
pub(crate) fn effective_span_scale(span: &RichSpan) -> f64 {
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
pub(crate) fn script_baseline_offset_pt(script: SpanScript, base_size_pt: f64) -> f64 {
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

/// Clean-room Adobe Core-14 Helvetica (Regular/Bold/Oblique/BoldOblique) and Symbol advance
/// width for Unicode codepoint `ch`, normalized in `em` units (`width_1000 / 1000.0`).
#[must_use]
pub fn core14_advance_em(ch: char, weight: SpanWeight, _slope: SpanSlope) -> f64 {
    if let Some(w) = core14_ascii_width_1000(ch, weight) {
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
        '\u{00B0}' | '\u{2218}' => Some(400),                            // degree / ring
        '\u{00B1}' | '\u{2213}' => Some(549),                            // plus-minus / minus-plus
        '\u{00B2}' | '\u{00B3}' | '\u{00B9}' => Some(333),               // superscript digits
        '\u{00B7}' => Some(250),                                         // middle dot
        '\u{00C5}' => Some(667),                                         // Angstrom A-ring
        '\u{00D7}' | '\u{00F7}' => Some(549),                            // times / divide
        '\u{2013}' => Some(556),                                         // en-dash
        '\u{2014}' | '\u{2026}' | '\u{2030}' | '\u{2122}' => Some(1000), // em-dash / ellipsis / permil / TM
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
        _ => None,
    }
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
        if bytes.len() < 12 {
            return None;
        }
        let version = bytes.get(0..4)?;
        if version != [0x00, 0x01, 0x00, 0x00] && version != b"true" && version != b"OTTO" {
            return None;
        }
        let num_tables = read_u16_be(bytes, 4)?;
        if num_tables == 0 || num_tables > 128 {
            return None;
        }

        let head = find_sfnt_table(bytes, num_tables, *b"head")?;
        let hhea = find_sfnt_table(bytes, num_tables, *b"hhea")?;
        let maxp = find_sfnt_table(bytes, num_tables, *b"maxp")?;
        let cmap = find_sfnt_table(bytes, num_tables, *b"cmap")?;
        let hmtx = find_sfnt_table(bytes, num_tables, *b"hmtx")?;
        let loca = find_sfnt_table(bytes, num_tables, *b"loca");
        let glyf = find_sfnt_table(bytes, num_tables, *b"glyf");

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

fn find_sfnt_table(bytes: &[u8], num_tables: u16, target: [u8; 4]) -> Option<&[u8]> {
    for i in 0..usize::from(num_tables) {
        let rec_off = 12usize.checked_add(i.checked_mul(16)?)?;
        let tag = bytes.get(rec_off..rec_off + 4)?;
        if tag == target {
            let offset = usize::try_from(read_u32_be(bytes, rec_off + 8)?).ok()?;
            let length = usize::try_from(read_u32_be(bytes, rec_off + 12)?).ok()?;
            let end = offset.checked_add(length)?;
            return bytes.get(offset..end);
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
