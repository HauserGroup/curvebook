//! L6 Text Shaping, Measurement, and Coverage-Mask Glyph Atlas Bridge (`R10.9`, `#248` §2).
//!
//! Decouples `LayoutTree` typography and software rasterization behind [`TextMeasurer`]:
//! - Desktop/CLI/tests use [`DesktopTextMeasurer`], backed by [`crate::theme::FontBook`] (OS host
//!   fonts + `CURVEBOOK_FONTS_DIR` + `curvebook-text` clean-room glyph outlines, zero bundled font
//!   requirements) and [`SoftwareTextPainter`], a cached 4x4 supersampled nonzero-winding glyph
//!   coverage atlas replacing `FONT_5X7`.
//! - Browser (`wasm32`) and future L2 shaper upgrades implement [`TextMeasurer`] directly.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use curvebook_palette::Rgba8;
use curvebook_render::RasterImage;
use curvebook_text::glyph_paths::outline_ttf_glyph;
use curvebook_text::{
    GlyphOutlineCmd, RichText, SpanScript, SpanSlope, SpanWeight, TextDirection, contains_rtl_script, is_rtl_locale, measure_text_pt,
    outline_rich_text, reorder_visual_bidi,
};
use serde::{Deserialize, Serialize};

use crate::theme::FontBook;

/// Logical font family classification shared across desktop and web text bridges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FontFamilyKind {
    #[default]
    UiSans,
    UiMono,
    GridTabular,
    ScientificSerif,
}

/// Complete style specification passed into [`TextMeasurer`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiTextStyle {
    pub family: FontFamilyKind,
    pub size_px: f64,
    pub weight: SpanWeight,
    pub slope: SpanSlope,
    pub tabular_nums: bool,
    pub rich: Option<RichText>,
}

impl Default for UiTextStyle {
    fn default() -> Self {
        Self::ui_sans(13.0)
    }
}

impl UiTextStyle {
    /// Standard proportional UI sans-serif style at `size_px`.
    #[must_use]
    pub const fn ui_sans(size_px: f64) -> Self {
        Self { family: FontFamilyKind::UiSans, size_px, weight: SpanWeight::Regular, slope: SpanSlope::Upright, tabular_nums: false, rich: None }
    }

    /// Semibold/bold UI header or title style.
    #[must_use]
    pub const fn ui_bold(size_px: f64) -> Self {
        Self { family: FontFamilyKind::UiSans, size_px, weight: SpanWeight::Bold, slope: SpanSlope::Upright, tabular_nums: false, rich: None }
    }

    /// Fixed-digit-width numeric grid/table style.
    #[must_use]
    pub const fn grid_tabular(size_px: f64) -> Self {
        Self { family: FontFamilyKind::GridTabular, size_px, weight: SpanWeight::Regular, slope: SpanSlope::Upright, tabular_nums: true, rich: None }
    }

    /// Monospace code / formula bar / equation token style.
    #[must_use]
    pub const fn ui_mono(size_px: f64) -> Self {
        Self { family: FontFamilyKind::UiMono, size_px, weight: SpanWeight::Regular, slope: SpanSlope::Upright, tabular_nums: true, rich: None }
    }

    /// Clone with bold weight enabled.
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.weight = SpanWeight::Bold;
        self
    }

    /// Clone with italic slope enabled (used e.g. for excluded data cells).
    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.slope = SpanSlope::Italic;
        self
    }

    /// Clone with rich-text span markup attached.
    #[must_use]
    pub fn with_rich(mut self, rich: RichText) -> Self {
        self.rich = Some(rich);
        self
    }

    fn clamped_size_px(&self) -> f64 {
        if self.size_px.is_finite() && self.size_px > 0.0 { self.size_px.clamp(6.0, 96.0) } else { 13.0 }
    }
}

/// Vertical & horizontal font metrics returned by [`TextMeasurer::measure`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeasuredTextMetrics {
    pub width_px: f64,
    pub ascent_px: f64,
    pub descent_px: f64,
    pub line_height_px: f64,
}

/// Positioned glyph run inside a shaped single line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapedGlyphRun {
    pub ch: char,
    pub x_px: f64,
    pub y_px: f64,
    pub advance_px: f64,
    pub glyph_id: Option<u16>,
    pub face_index: Option<usize>,
    pub font_scale: f64,
    pub weight: SpanWeight,
    pub slope: SpanSlope,
    pub color_override: Option<Rgba8>,
    pub used_vector_fallback: bool,
    pub missing_glyph: bool,
}

/// Visually ordered shaped single line ready for pixel painting and bounds verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapedTextLine {
    pub visual_text: String,
    pub direction: TextDirection,
    pub metrics: MeasuredTextMetrics,
    pub glyphs: Vec<ShapedGlyphRun>,
    pub missing_glyph_count: usize,
}

/// Cross-backend text measurement & shaping interface shared between desktop CPU rendering and web
/// `<canvas>` measurement.
pub trait TextMeasurer {
    fn measure(&self, text: &str, style: &UiTextStyle, locale: &str) -> MeasuredTextMetrics;
    fn shape_line(&self, text: &str, style: &UiTextStyle, locale: &str) -> ShapedTextLine;
}

#[derive(Debug, Clone, Copy)]
struct CachedGlyphEntry {
    glyph_id: Option<u16>,
    face_index: Option<usize>,
    prop_em: f64,
    tabular_em: f64,
    used_vector_fallback: bool,
    missing_glyph: bool,
}

const PRECOMPUTED_NON_ASCII: &[char] =
    &['…', '–', '—', '±', '×', '÷', '≤', '≥', '≠', '≈', '°', 'µ', 'μ', 'α', 'β', 'γ', 'σ', 'Δ', '∑', '√', '•', '→', '←', '↑', '↓'];

/// Desktop / headless CPU text measurer wrapping [`FontBook`] and `curvebook-text` clean-room
/// outlines without any external font bundle requirement.
#[derive(Debug, Clone)]
pub struct DesktopTextMeasurer {
    font_book: FontBook,
    book_fingerprint: u64,
    ascii_cache: [CachedGlyphEntry; 128],
    non_ascii_cache: BTreeMap<char, CachedGlyphEntry>,
}

impl Default for DesktopTextMeasurer {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopTextMeasurer {
    /// Construct a measurer using host OS fonts (`CURVEBOOK_FONTS_DIR`, OS system faces) and the
    /// built-in minimal SFNT + clean-room vector fallback.
    #[must_use]
    pub fn new() -> Self {
        Self::with_font_book(FontBook::with_system_and_fallback())
    }

    /// Construct a measurer backed by a caller-supplied [`FontBook`].
    #[must_use]
    pub fn with_font_book(font_book: FontBook) -> Self {
        let book_fingerprint = font_book.fingerprint();
        let parsed = font_book.parsed_faces();
        let face_tabular_em: Vec<f64> = parsed.iter().map(|(_, ttf)| tabular_digit_advance_em(ttf).max(0.55)).collect();
        let placeholder =
            CachedGlyphEntry { glyph_id: None, face_index: None, prop_em: 0.54, tabular_em: 0.60, used_vector_fallback: false, missing_glyph: false };
        let mut ascii_cache = [placeholder; 128];
        for (byte, slot) in ascii_cache.iter_mut().enumerate() {
            let ch = char::from(byte as u8);
            *slot = compute_cached_glyph_entry(&parsed, &face_tabular_em, ch);
        }
        let mut non_ascii_cache = BTreeMap::new();
        for &ch in PRECOMPUTED_NON_ASCII {
            non_ascii_cache.insert(ch, compute_cached_glyph_entry(&parsed, &face_tabular_em, ch));
        }
        Self { font_book, book_fingerprint, ascii_cache, non_ascii_cache }
    }

    /// Return the process-wide cached default [`DesktopTextMeasurer`].
    #[must_use]
    pub fn global() -> &'static Self {
        static GLOBAL: OnceLock<DesktopTextMeasurer> = OnceLock::new();
        GLOBAL.get_or_init(Self::new)
    }

    /// Underlying [`FontBook`].
    #[must_use]
    pub const fn font_book(&self) -> &FontBook {
        &self.font_book
    }

    /// Stable hash identifying the loaded font face set.
    #[must_use]
    pub const fn book_fingerprint(&self) -> u64 {
        self.book_fingerprint
    }

    fn resolve_glyph_fast<'p>(
        &'p self,
        parsed_cache: &mut Option<Vec<(&'p str, curvebook_text::TrueTypeFont<'p>)>>,
        ch: char,
        style: &UiTextStyle,
        eff_size: f64,
    ) -> GlyphResolution {
        if ch.is_control() || is_bidi_format_char(ch) {
            return GlyphResolution { advance_px: 0.0, glyph_id: None, face_index: None, used_vector_fallback: false, missing_glyph: false };
        }
        if ch == ' ' {
            let space_ratio = if matches!(style.family, FontFamilyKind::UiMono | FontFamilyKind::GridTabular) { 0.56 } else { 0.28 };
            return GlyphResolution {
                advance_px: eff_size * space_ratio,
                glyph_id: None,
                face_index: None,
                used_vector_fallback: false,
                missing_glyph: false,
            };
        }
        if ch.is_ascii() {
            return apply_cached_glyph(self.ascii_cache[ch as usize], ch, style, eff_size);
        }
        if let Some(&entry) = self.non_ascii_cache.get(&ch) {
            return apply_cached_glyph(entry, ch, style, eff_size);
        }
        let parsed = parsed_cache.get_or_insert_with(|| self.font_book.parsed_faces());
        let tabular_digit =
            (style.tabular_nums || matches!(style.family, FontFamilyKind::GridTabular | FontFamilyKind::UiMono)) && ch.is_ascii_digit();
        for (face_idx, (_, ttf)) in parsed.iter().enumerate() {
            if let Some(gid) = ttf.glyph_id(ch) {
                let raw_adv = if tabular_digit {
                    tabular_digit_advance_em(ttf).max(0.55) * eff_size
                } else if style.family == FontFamilyKind::UiMono {
                    0.60 * eff_size
                } else {
                    ttf.advance_em(ch).unwrap_or_else(|| fallback_char_advance_em(ch)) * eff_size
                };
                let bold_pad = if style.weight == SpanWeight::Bold { 1.04 } else { 1.0 };
                return GlyphResolution {
                    advance_px: (raw_adv * bold_pad).max(eff_size * 0.22),
                    glyph_id: Some(gid),
                    face_index: Some(face_idx),
                    used_vector_fallback: false,
                    missing_glyph: false,
                };
            }
        }
        let fallback_em = if tabular_digit || style.family == FontFamilyKind::UiMono { 0.60 } else { fallback_char_advance_em(ch) };
        let bold_pad = if style.weight == SpanWeight::Bold { 1.04 } else { 1.0 };
        let has_vector = has_cleanroom_outline(ch);
        GlyphResolution {
            advance_px: (fallback_em * eff_size * bold_pad).max(eff_size * 0.22),
            glyph_id: None,
            face_index: None,
            used_vector_fallback: has_vector,
            missing_glyph: !has_vector && !ch.is_whitespace(),
        }
    }
}

fn compute_cached_glyph_entry(parsed: &[(&str, curvebook_text::TrueTypeFont<'_>)], face_tabular_em: &[f64], ch: char) -> CachedGlyphEntry {
    if ch.is_control() || ch == ' ' || is_bidi_format_char(ch) {
        return CachedGlyphEntry {
            glyph_id: None,
            face_index: None,
            prop_em: 0.28,
            tabular_em: 0.56,
            used_vector_fallback: false,
            missing_glyph: false,
        };
    }
    for (face_idx, (_, ttf)) in parsed.iter().enumerate() {
        if let Some(gid) = ttf.glyph_id(ch) {
            return CachedGlyphEntry {
                glyph_id: Some(gid),
                face_index: Some(face_idx),
                prop_em: ttf.advance_em(ch).unwrap_or_else(|| fallback_char_advance_em(ch)),
                tabular_em: face_tabular_em.get(face_idx).copied().unwrap_or(0.56),
                used_vector_fallback: false,
                missing_glyph: false,
            };
        }
    }
    let has_vector = has_cleanroom_outline(ch);
    CachedGlyphEntry {
        glyph_id: None,
        face_index: None,
        prop_em: fallback_char_advance_em(ch),
        tabular_em: 0.60,
        used_vector_fallback: has_vector,
        missing_glyph: !has_vector && !ch.is_whitespace(),
    }
}

fn apply_cached_glyph(entry: CachedGlyphEntry, ch: char, style: &UiTextStyle, eff_size: f64) -> GlyphResolution {
    let tabular_digit = (style.tabular_nums || matches!(style.family, FontFamilyKind::GridTabular | FontFamilyKind::UiMono)) && ch.is_ascii_digit();
    let base_em = if entry.glyph_id.is_some() {
        if tabular_digit {
            entry.tabular_em
        } else if style.family == FontFamilyKind::UiMono {
            0.60
        } else {
            entry.prop_em
        }
    } else if tabular_digit || style.family == FontFamilyKind::UiMono {
        0.60
    } else {
        entry.prop_em
    };
    let bold_pad = if style.weight == SpanWeight::Bold { 1.04 } else { 1.0 };
    GlyphResolution {
        advance_px: (base_em * eff_size * bold_pad).max(eff_size * 0.22),
        glyph_id: entry.glyph_id,
        face_index: entry.face_index,
        used_vector_fallback: entry.used_vector_fallback,
        missing_glyph: entry.missing_glyph,
    }
}

#[derive(Debug, Clone, Copy)]
struct GlyphResolution {
    advance_px: f64,
    glyph_id: Option<u16>,
    face_index: Option<usize>,
    used_vector_fallback: bool,
    missing_glyph: bool,
}

fn tabular_digit_advance_em(ttf: &curvebook_text::TrueTypeFont<'_>) -> f64 {
    let mut max_adv = 0.56_f64;
    for d in '0'..='9' {
        if let Some(a) = ttf.advance_em(d) {
            max_adv = max_adv.max(a);
        }
    }
    max_adv
}

fn fallback_char_advance_em(ch: char) -> f64 {
    if ch.is_ascii() {
        return match ch {
            'i' | 'l' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' => 0.28,
            'f' | 'j' | 'r' | 't' | '(' | ')' | '[' | ']' | '{' | '}' | '/' | '\\' | '-' => 0.36,
            'm' | 'w' | 'M' | 'W' | '@' | '%' | '&' => 0.80,
            'A'..='Z' => 0.66,
            '0'..='9' => 0.58,
            _ => 0.54,
        };
    }
    let rich = RichText::plain(ch.to_string());
    let m = measure_text_pt(&rich, 10.0);
    (m.width_pt / 10.0).clamp(0.45, 1.05)
}

fn has_cleanroom_outline(ch: char) -> bool {
    ch.is_whitespace() || curvebook_text::has_cleanroom_vector_outline(ch)
}

impl TextMeasurer for DesktopTextMeasurer {
    fn measure(&self, text: &str, style: &UiTextStyle, locale: &str) -> MeasuredTextMetrics {
        let size = style.clamped_size_px();
        let ascent_px = size * 0.80;
        let descent_px = size * 0.24;
        let line_height_px = (size * 1.32).ceil();
        if text.is_empty() && style.rich.is_none() {
            return MeasuredTextMetrics { width_px: 0.0, ascent_px, descent_px, line_height_px };
        }
        let mut parsed = None;
        let mut width_px = 0.0;
        if let Some(rich) = &style.rich {
            for span in &rich.spans {
                let scale = match span.script {
                    SpanScript::Baseline => span.scale.max(0.5),
                    SpanScript::Superscript | SpanScript::Subscript => span.scale.max(0.5) * 0.72,
                };
                let eff = size * scale;
                let span_style = UiTextStyle { weight: span.weight, slope: span.slope, ..style.clone() };
                for ch in span.text.chars() {
                    width_px += self.resolve_glyph_fast(&mut parsed, ch, &span_style, eff).advance_px;
                }
            }
        } else if text.is_ascii() {
            for b in text.bytes() {
                width_px += self.resolve_glyph_fast(&mut parsed, char::from(b), style, size).advance_px;
            }
        } else {
            let dir = if is_rtl_locale(locale) { TextDirection::Rtl } else { TextDirection::Ltr };
            let visual = shape_visual_text(text, dir);
            for ch in visual.chars() {
                width_px += self.resolve_glyph_fast(&mut parsed, ch, style, size).advance_px;
            }
        }
        MeasuredTextMetrics { width_px, ascent_px, descent_px, line_height_px }
    }

    fn shape_line(&self, text: &str, style: &UiTextStyle, locale: &str) -> ShapedTextLine {
        let size = style.clamped_size_px();
        let ascent_px = size * 0.80;
        let descent_px = size * 0.24;
        let line_height_px = (size * 1.32).ceil();
        let direction = if is_rtl_locale(locale) { TextDirection::Rtl } else { TextDirection::Ltr };
        let mut parsed = None;
        let mut glyphs = Vec::new();
        let mut visual_text = String::new();
        let mut pen_x = 0.0_f64;
        let mut missing_glyph_count = 0usize;

        if let Some(rich) = &style.rich {
            for span in &rich.spans {
                let (scale, y_off) = match span.script {
                    SpanScript::Baseline => (span.scale.max(0.5), 0.0),
                    SpanScript::Superscript => (span.scale.max(0.5) * 0.72, -size * 0.34),
                    SpanScript::Subscript => (span.scale.max(0.5) * 0.72, size * 0.18),
                };
                let eff = size * scale;
                let span_style = UiTextStyle { weight: span.weight, slope: span.slope, ..style.clone() };
                let reordered = shape_visual_text(&span.text, direction);
                for ch in reordered.chars() {
                    visual_text.push(ch);
                    let res = self.resolve_glyph_fast(&mut parsed, ch, &span_style, eff);
                    if res.missing_glyph {
                        missing_glyph_count += 1;
                    }
                    glyphs.push(ShapedGlyphRun {
                        ch,
                        x_px: pen_x,
                        y_px: y_off,
                        advance_px: res.advance_px,
                        glyph_id: res.glyph_id,
                        face_index: res.face_index,
                        font_scale: scale,
                        weight: span.weight,
                        slope: span.slope,
                        color_override: span.color,
                        used_vector_fallback: res.used_vector_fallback,
                        missing_glyph: res.missing_glyph,
                    });
                    pen_x += res.advance_px;
                }
            }
        } else {
            visual_text = shape_visual_text(text, direction);
            glyphs.reserve(visual_text.len());
            for ch in visual_text.chars() {
                let res = self.resolve_glyph_fast(&mut parsed, ch, style, size);
                if res.missing_glyph {
                    missing_glyph_count += 1;
                }
                glyphs.push(ShapedGlyphRun {
                    ch,
                    x_px: pen_x,
                    y_px: 0.0,
                    advance_px: res.advance_px,
                    glyph_id: res.glyph_id,
                    face_index: res.face_index,
                    font_scale: 1.0,
                    weight: style.weight,
                    slope: style.slope,
                    color_override: None,
                    used_vector_fallback: res.used_vector_fallback,
                    missing_glyph: res.missing_glyph,
                });
                pen_x += res.advance_px;
            }
        }

        ShapedTextLine {
            visual_text,
            direction,
            metrics: MeasuredTextMetrics { width_px: pen_x, ascent_px, descent_px, line_height_px },
            glyphs,
            missing_glyph_count,
        }
    }
}

fn is_bidi_format_char(ch: char) -> bool {
    matches!(ch as u32, 0x200E..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069)
}

fn is_rtl_codepoint(ch: char) -> bool {
    let mut buf = [0u8; 4];
    contains_rtl_script(ch.encode_utf8(&mut buf))
}

fn shape_visual_text(text: &str, direction: TextDirection) -> String {
    if text.is_empty() || text.is_ascii() {
        return text.to_owned();
    }
    if direction == TextDirection::Ltr {
        return reorder_visual_bidi(text, TextDirection::Ltr);
    }
    if !contains_rtl_script(text) && !text.chars().any(is_bidi_format_char) {
        return text.to_owned();
    }
    let (prepared, added_isolates) = isolate_rtl_scientific_spans(text);
    let reordered = reorder_visual_bidi(&prepared, TextDirection::Rtl);
    if added_isolates { reordered.chars().filter(|&ch| !matches!(ch, '\u{2066}' | '\u{2069}')).collect() } else { reordered }
}

fn isolate_rtl_scientific_spans(text: &str) -> (String, bool) {
    let mut out = String::with_capacity(text.len() + 16);
    let mut added = false;
    let mut i = 0usize;
    let mut lri_depth = 0usize;

    while i < text.len() {
        let Some(ch) = text[i..].chars().next() else {
            break;
        };
        if ch == '\u{2066}' {
            lri_depth += 1;
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if ch == '\u{2069}' {
            lri_depth = lri_depth.saturating_sub(1);
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if lri_depth == 0
            && let Some(end) = match_bracketed_interval(text, i).or_else(|| match_formula_span(text, i)).or_else(|| match_signed_number_span(text, i))
        {
            out.push('\u{2066}');
            out.push_str(&text[i..end]);
            out.push('\u{2069}');
            added = true;
            i = end;
            continue;
        }
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, added)
}

fn has_sign_before_digit(s: &str) -> bool {
    let mut chars = s.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if matches!(ch, '+' | '-' | '\u{2212}' | '\u{00B1}') {
            let Some(&(_, next)) = chars.peek() else {
                continue;
            };
            if next.is_ascii_digit() {
                return true;
            }
            if next == '.' {
                let mut look = chars.clone();
                let _ = look.next();
                if look.next().is_some_and(|(_, d)| d.is_ascii_digit()) {
                    return true;
                }
            }
        }
    }
    false
}

fn match_bracketed_interval(text: &str, start: usize) -> Option<usize> {
    let open = text[start..].chars().next()?;
    let close = match open {
        '[' => ']',
        '(' => ')',
        _ => return None,
    };
    let mut depth = 0usize;
    let mut end_idx = None;
    for (rel, ch) in text[start..].char_indices() {
        if matches!(ch, '\n' | '\r') || is_rtl_codepoint(ch) || is_bidi_format_char(ch) {
            return None;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                end_idx = Some(start + rel + ch.len_utf8());
                break;
            }
        }
    }
    let end = end_idx?;
    let inner = &text[start + open.len_utf8()..end - close.len_utf8()];
    let has_digit = inner.bytes().any(|b| b.is_ascii_digit());
    let has_sep = inner.contains(',') || inner.contains(';');
    if has_digit && (has_sep || has_sign_before_digit(inner)) { Some(end) } else { None }
}

fn match_formula_span(text: &str, start: usize) -> Option<usize> {
    if !text[start..].starts_with('=') {
        return None;
    }
    let prev = text[..start].chars().next_back();
    if let Some(p) = prev
        && !p.is_whitespace()
        && !matches!(p, ':' | '(' | '[' | '"' | '\'')
    {
        return None;
    }
    let first = text[start + 1..].chars().next()?;
    if !(first.is_ascii_alphanumeric() || matches!(first, '_' | '(' | '-' | '+' | '\u{2212}' | '.' | '$')) {
        return None;
    }
    let mut depth = 0usize;
    let mut end = start + 1;
    let mut has_alnum = false;
    for (rel, ch) in text[start + 1..].char_indices() {
        if ch.is_control() || is_rtl_codepoint(ch) || is_bidi_format_char(ch) {
            break;
        }
        if ch.is_whitespace() && depth == 0 {
            break;
        }
        if matches!(ch, '(' | '[') {
            depth += 1;
        } else if matches!(ch, ')' | ']') {
            if depth == 0 {
                break;
            }
            depth -= 1;
        }
        if ch.is_ascii_alphanumeric() {
            has_alnum = true;
        }
        end = start + 1 + rel + ch.len_utf8();
    }
    while end > start + 1 {
        let last = text[start + 1..end].chars().next_back()?;
        if matches!(last, ',' | ';' | ':' | '.' | '!' | '?') {
            end -= last.len_utf8();
        } else {
            break;
        }
    }
    if has_alnum && end > start + 1 { Some(end) } else { None }
}

fn match_signed_number_span(text: &str, start: usize) -> Option<usize> {
    let sign = text[start..].chars().next()?;
    if !matches!(sign, '-' | '+' | '\u{2212}' | '\u{00B1}') {
        return None;
    }
    if let Some(prev) = text[..start].chars().next_back()
        && prev.is_alphanumeric()
    {
        return None;
    }
    let mut p = start + sign.len_utf8();
    let mut saw_digit = false;
    if text[p..].starts_with('.') {
        p += 1;
    }
    while let Some(ch) = text[p..].chars().next() {
        if ch.is_ascii_digit() {
            saw_digit = true;
            p += 1;
        } else if matches!(ch, '.' | ',') && text[p + 1..].chars().next().is_some_and(|d| d.is_ascii_digit()) {
            p += 1;
        } else {
            break;
        }
    }
    if !saw_digit {
        return None;
    }
    if text[p..].starts_with(['e', 'E']) {
        let mut exp_p = p + 1;
        if let Some(es) = text[exp_p..].chars().next()
            && matches!(es, '+' | '-' | '\u{2212}')
        {
            exp_p += es.len_utf8();
        }
        let mut exp_digits = false;
        while let Some(d) = text[exp_p..].chars().next() {
            if d.is_ascii_digit() {
                exp_digits = true;
                exp_p += 1;
            } else {
                break;
            }
        }
        if exp_digits {
            p = exp_p;
        }
    }
    Some(p)
}

/// Measure `text` and append `'…'` if it exceeds `max_width_px`, returning `(display_text, ellipsized)`.
#[must_use]
pub fn ellipsize_text(measurer: &dyn TextMeasurer, text: &str, style: &UiTextStyle, locale: &str, max_width_px: f64) -> (String, bool) {
    if !max_width_px.is_finite() || max_width_px <= 0.0 || text.is_empty() {
        return (String::new(), !text.is_empty());
    }
    if measurer.measure(text, style, locale).width_px <= max_width_px {
        return (text.to_owned(), false);
    }
    let ellipsis = "…";
    let ell_w = measurer.measure(ellipsis, style, locale).width_px;
    if ell_w >= max_width_px {
        return (String::from(ellipsis), true);
    }
    let mut candidate = String::new();
    for ch in text.chars() {
        candidate.push(ch);
        let probe = format!("{candidate}{ellipsis}");
        if measurer.measure(&probe, style, locale).width_px > max_width_px {
            candidate.pop();
            break;
        }
    }
    candidate.push('…');
    (candidate, true)
}

/// Axis-aligned clipping rectangle in physical frame pixels (`x, y, width, height`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaintClipPx {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PaintClipPx {
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (i64::from(self.x) + i64::from(self.width)).min(i64::from(other.x) + i64::from(other.width));
        let y1 = (i64::from(self.y) + i64::from(self.height)).min(i64::from(other.y) + i64::from(other.height));
        let w = u32::try_from((x1 - i64::from(x0)).max(0)).unwrap_or(0);
        let h = u32::try_from((y1 - i64::from(y0)).max(0)).unwrap_or(0);
        Self { x: x0, y: y0, width: w, height: h }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct GlyphCacheKey {
    book_fingerprint: u64,
    face_index: Option<usize>,
    codepoint: u32,
    size_q4: u16,
    bold: bool,
    italic: bool,
}

#[derive(Debug, Clone)]
struct GlyphCoverageMask {
    offset_x: i32,
    offset_y: i32,
    width: u32,
    height: u32,
    alpha: Vec<u8>,
}

#[derive(Debug, Default)]
struct SoftwareGlyphAtlas {
    entries: BTreeMap<GlyphCacheKey, GlyphCoverageMask>,
}

impl SoftwareGlyphAtlas {
    const MAX_CACHED_GLYPHS: usize = 8_192;

    fn get_or_rasterize(&mut self, measurer: &DesktopTextMeasurer, key: GlyphCacheKey, ch: char, eff_size_px: f64) -> &GlyphCoverageMask {
        if self.entries.len() >= Self::MAX_CACHED_GLYPHS {
            self.entries.clear();
        }
        self.entries.entry(key).or_insert_with(|| rasterize_glyph_mask(measurer, key, ch, eff_size_px))
    }
}

thread_local! {
    static THREAD_GLYPH_ATLAS: RefCell<SoftwareGlyphAtlas> = RefCell::new(SoftwareGlyphAtlas::default());
}

/// Styling and clipping bundle passed to [`SoftwareTextPainter`].
#[derive(Debug, Clone, Copy)]
pub struct TextPaintStyle<'a> {
    pub style: &'a UiTextStyle,
    pub locale: &'a str,
    pub color: Rgba8,
    pub clip: PaintClipPx,
}

/// Coverage-mask glyph painter rasterizing [`ShapedTextLine`] onto RGBA buffers using the
/// thread-local coverage atlas (zero `std::sync::Mutex`, deterministic, `<= 16.67 ms` grid budgets).
#[derive(Debug, Default, Clone, Copy)]
pub struct SoftwareTextPainter;

impl SoftwareTextPainter {
    /// Paint a shaped line at baseline `origin_px = [x, baseline_y]`.
    pub fn paint_shaped_line(
        measurer: &DesktopTextMeasurer,
        img: &mut RasterImage,
        shaped: &ShapedTextLine,
        opts: &TextPaintStyle<'_>,
        origin_px: [f64; 2],
    ) {
        if opts.color.a == 0 || opts.clip.width == 0 || opts.clip.height == 0 {
            return;
        }
        let img_w = i32::try_from(img.width_px).unwrap_or(i32::MAX) as u32;
        let frame_clip = opts.clip.intersect(PaintClipPx::new(0, 0, img_w, img.height_px));
        if frame_clip.width == 0 || frame_clip.height == 0 {
            return;
        }
        let base_size = opts.style.clamped_size_px();
        THREAD_GLYPH_ATLAS.with(|cell| {
            let mut atlas = cell.borrow_mut();
            for glyph in &shaped.glyphs {
                if glyph.ch.is_whitespace() || glyph.ch.is_control() || is_bidi_format_char(glyph.ch) {
                    continue;
                }
                let eff_size = (base_size * glyph.font_scale).clamp(5.0, 96.0);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let size_q4 = (eff_size * 4.0).round().clamp(1.0, f64::from(u16::MAX)) as u16;
                let key = GlyphCacheKey {
                    book_fingerprint: measurer.book_fingerprint(),
                    face_index: glyph.face_index,
                    codepoint: u32::from(glyph.ch),
                    size_q4,
                    bold: glyph.weight == SpanWeight::Bold,
                    italic: glyph.slope == SpanSlope::Italic,
                };
                let mask = atlas.get_or_rasterize(measurer, key, glyph.ch, eff_size);
                if mask.width == 0 || mask.height == 0 {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation)]
                let gx = (origin_px[0] + glyph.x_px).round() as i32 + mask.offset_x;
                #[allow(clippy::cast_possible_truncation)]
                let gy = (origin_px[1] + glyph.y_px).round() as i32 + mask.offset_y;
                let ink = glyph.color_override.unwrap_or(opts.color);
                blit_alpha_mask(img, mask, gx, gy, ink, frame_clip);
            }
        });
    }

    /// Shape and paint `text` from top-left box `top_left_px = [x, top_y]`.
    pub fn paint_text_top_left(
        measurer: &DesktopTextMeasurer,
        img: &mut RasterImage,
        text: &str,
        opts: &TextPaintStyle<'_>,
        top_left_px: [f64; 2],
    ) -> ShapedTextLine {
        let shaped = measurer.shape_line(text, opts.style, opts.locale);
        let baseline_y = top_left_px[1] + shaped.metrics.ascent_px;
        Self::paint_shaped_line(measurer, img, &shaped, opts, [top_left_px[0], baseline_y]);
        shaped
    }
}

fn blit_alpha_mask(img: &mut RasterImage, mask: &GlyphCoverageMask, gx: i32, gy: i32, ink: Rgba8, clip: PaintClipPx) {
    let x0 = gx.max(clip.x);
    let y0 = gy.max(clip.y);
    let x1 = (gx + i32::try_from(mask.width).unwrap_or(0)).min(clip.x + i32::try_from(clip.width).unwrap_or(0));
    let y1 = (gy + i32::try_from(mask.height).unwrap_or(0)).min(clip.y + i32::try_from(clip.height).unwrap_or(0));
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let img_w = img.width_px as usize;
    let mw = mask.width as usize;
    let ink_a = u32::from(ink.a);
    let span = (x1 - x0) as usize;
    let src_x0 = (x0 - gx) as usize;
    let dst_x0 = x0 as usize;

    for py in y0..y1 {
        let my = (py - gy) as usize;
        let iy = py as usize;
        let src_start = my * mw + src_x0;
        let dst_start = (iy * img_w + dst_x0) * 4;
        let Some(src_row) = mask.alpha.get(src_start..src_start + span) else {
            continue;
        };
        let Some(dst_row) = img.rgba.get_mut(dst_start..dst_start + span * 4) else {
            continue;
        };
        for (&raw_cov, dst) in src_row.iter().zip(dst_row.as_chunks_mut::<4>().0) {
            if raw_cov == 0 {
                continue;
            }
            let alpha = (u32::from(raw_cov) * ink_a + 127) / 255;
            if alpha == 0 {
                continue;
            }
            if alpha >= 255 {
                dst[0] = ink.r;
                dst[1] = ink.g;
                dst[2] = ink.b;
                dst[3] = 255;
            } else {
                let inv = 255 - alpha;
                dst[0] = ((u32::from(ink.r) * alpha + u32::from(dst[0]) * inv + 127) / 255) as u8;
                dst[1] = ((u32::from(ink.g) * alpha + u32::from(dst[1]) * inv + 127) / 255) as u8;
                dst[2] = ((u32::from(ink.b) * alpha + u32::from(dst[2]) * inv + 127) / 255) as u8;
                dst[3] = (alpha + (u32::from(dst[3]) * inv + 127) / 255).min(255) as u8;
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PolyEdge {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

fn rasterize_glyph_mask(measurer: &DesktopTextMeasurer, key: GlyphCacheKey, ch: char, size_px: f64) -> GlyphCoverageMask {
    let mut cmds: Option<Vec<GlyphOutlineCmd>> = None;
    if let Some(face_idx) = key.face_index
        && let Some(ttf) = measurer.font_book().parsed_face(face_idx)
    {
        cmds = outline_ttf_glyph(&ttf, ch, 0.0, 0.0, size_px, false);
    }
    let commands = match cmds {
        Some(c) if !c.is_empty() => c,
        _ => outline_rich_text(&RichText::plain(ch.to_string()), 0.0, 0.0, size_px).into_iter().flat_map(|g| g.commands).collect(),
    };

    let mut edges = Vec::new();
    flatten_glyph_edges(&commands, key.italic, &mut edges);
    if edges.is_empty() {
        return rasterize_fallback_glyph_box(ch, size_px);
    }

    let (min_x, min_y, max_x, max_y) = edge_bounds(&edges);
    let pad = if key.bold { 2.0 } else { 1.0 };
    #[allow(clippy::cast_possible_truncation)]
    let x0 = (min_x - pad).floor() as i32;
    #[allow(clippy::cast_possible_truncation)]
    let y0 = (min_y - pad).floor() as i32;
    #[allow(clippy::cast_possible_truncation)]
    let x1 = (max_x + pad).ceil() as i32;
    #[allow(clippy::cast_possible_truncation)]
    let y1 = (max_y + pad).ceil() as i32;

    let w = u32::try_from((x1 - x0).clamp(0, 192)).unwrap_or(0);
    let h = u32::try_from((y1 - y0).clamp(0, 192)).unwrap_or(0);
    if w == 0 || h == 0 {
        return GlyphCoverageMask { offset_x: 0, offset_y: 0, width: 0, height: 0, alpha: Vec::new() };
    }

    let mut alpha = scanline_fill_4x4(&edges, x0, y0, w, h);
    if key.bold {
        apply_synthetic_bold_dilation(&mut alpha, w, h);
    }
    GlyphCoverageMask { offset_x: x0, offset_y: y0, width: w, height: h, alpha }
}

fn rasterize_fallback_glyph_box(ch: char, size_px: f64) -> GlyphCoverageMask {
    if ch.is_whitespace() || ch.is_control() {
        return GlyphCoverageMask { offset_x: 0, offset_y: 0, width: 0, height: 0, alpha: Vec::new() };
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let w = (size_px * 0.56).round().clamp(4.0, 48.0) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let h = (size_px * 0.72).round().clamp(5.0, 64.0) as u32;
    #[allow(clippy::cast_possible_truncation)]
    let offset_y = -(h as i32);
    let mut alpha = vec![0u8; (w as usize) * (h as usize)];
    for y in 0..h {
        for x in 0..w {
            let border = x == 0 || y == 0 || x + 1 == w || y + 1 == h;
            let diag = (x * h).abs_diff(y * w) <= w.max(h) / 2;
            if border || diag {
                alpha[(y as usize) * (w as usize) + (x as usize)] = 210;
            }
        }
    }
    GlyphCoverageMask { offset_x: 1, offset_y, width: w, height: h, alpha }
}

fn flatten_glyph_edges(commands: &[GlyphOutlineCmd], italic: bool, out: &mut Vec<PolyEdge>) {
    let shear = if italic { -0.20 } else { 0.0 };
    let xf = |x: f64, y: f64| -> (f64, f64) { (x + shear * y, y) };
    let mut start: (f64, f64) = (0.0, 0.0);
    let mut cur: (f64, f64) = (0.0, 0.0);
    let mut has_contour = false;

    for cmd in commands {
        match *cmd {
            GlyphOutlineCmd::MoveTo { x_pt, y_pt } => {
                if has_contour && (cur.0 - start.0).abs() + (cur.1 - start.1).abs() > 1e-4 {
                    out.push(PolyEdge { x0: cur.0, y0: cur.1, x1: start.0, y1: start.1 });
                }
                start = xf(x_pt, y_pt);
                cur = start;
                has_contour = true;
            }
            GlyphOutlineCmd::LineTo { x_pt, y_pt } => {
                let next = xf(x_pt, y_pt);
                out.push(PolyEdge { x0: cur.0, y0: cur.1, x1: next.0, y1: next.1 });
                cur = next;
            }
            GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                let c = xf(cx_pt, cy_pt);
                let p = xf(x_pt, y_pt);
                const STEPS: usize = 8;
                for s in 1..=STEPS {
                    let t = (s as f64) / (STEPS as f64);
                    let mt = 1.0 - t;
                    let nx = mt * mt * cur.0 + 2.0 * mt * t * c.0 + t * t * p.0;
                    let ny = mt * mt * cur.1 + 2.0 * mt * t * c.1 + t * t * p.1;
                    let prev = if s == 1 { cur } else { eval_quad(cur, c, p, ((s - 1) as f64) / (STEPS as f64)) };
                    out.push(PolyEdge { x0: prev.0, y0: prev.1, x1: nx, y1: ny });
                }
                cur = p;
            }
            GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                let c1 = xf(c1x_pt, c1y_pt);
                let c2 = xf(c2x_pt, c2y_pt);
                let p = xf(x_pt, y_pt);
                const STEPS: usize = 10;
                for s in 1..=STEPS {
                    let prev = eval_cubic(cur, c1, c2, p, ((s - 1) as f64) / (STEPS as f64));
                    let next = eval_cubic(cur, c1, c2, p, (s as f64) / (STEPS as f64));
                    out.push(PolyEdge { x0: prev.0, y0: prev.1, x1: next.0, y1: next.1 });
                }
                cur = p;
            }
            GlyphOutlineCmd::Close => {
                if has_contour && (cur.0 - start.0).abs() + (cur.1 - start.1).abs() > 1e-4 {
                    out.push(PolyEdge { x0: cur.0, y0: cur.1, x1: start.0, y1: start.1 });
                }
                cur = start;
            }
        }
    }
    if has_contour && (cur.0 - start.0).abs() + (cur.1 - start.1).abs() > 1e-4 {
        out.push(PolyEdge { x0: cur.0, y0: cur.1, x1: start.0, y1: start.1 });
    }
}

fn eval_quad(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), t: f64) -> (f64, f64) {
    let mt = 1.0 - t;
    (mt * mt * p0.0 + 2.0 * mt * t * p1.0 + t * t * p2.0, mt * mt * p0.1 + 2.0 * mt * t * p1.1 + t * t * p2.1)
}

fn eval_cubic(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let t2 = t * t;
    (
        mt2 * mt * p0.0 + 3.0 * mt2 * t * p1.0 + 3.0 * mt * t2 * p2.0 + t2 * t * p3.0,
        mt2 * mt * p0.1 + 3.0 * mt2 * t * p1.1 + 3.0 * mt * t2 * p2.1 + t2 * t * p3.1,
    )
}

fn edge_bounds(edges: &[PolyEdge]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for e in edges {
        min_x = min_x.min(e.x0).min(e.x1);
        min_y = min_y.min(e.y0).min(e.y1);
        max_x = max_x.max(e.x0).max(e.x1);
        max_y = max_y.max(e.y0).max(e.y1);
    }
    (min_x, min_y, max_x, max_y)
}

fn scanline_fill_4x4(edges: &[PolyEdge], origin_x: i32, origin_y: i32, width: u32, height: u32) -> Vec<u8> {
    const SUB: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let w = width as usize;
    let h = height as usize;
    let mut hits = vec![0u8; w * h];
    let mut crossings: Vec<(f64, i32)> = Vec::with_capacity(16);

    for py in 0..h {
        let base_y = f64::from(origin_y + (py as i32));
        let Some(row_hits) = hits.get_mut(py * w..(py + 1) * w) else {
            continue;
        };
        for &sy in &SUB {
            accumulate_subscanline(edges, base_y + sy, origin_x, row_hits, &mut crossings);
        }
    }

    hits.into_iter().map(|count| ((u16::from(count) * 255 + 8) / 16).min(255) as u8).collect()
}

fn accumulate_subscanline(edges: &[PolyEdge], sample_y: f64, origin_x: i32, row_hits: &mut [u8], crossings: &mut Vec<(f64, i32)>) {
    crossings.clear();
    for e in edges {
        let (ya, yb, dir) = if e.y0 <= e.y1 { (e.y0, e.y1, 1) } else { (e.y1, e.y0, -1) };
        if sample_y >= ya && sample_y < yb && (yb - ya) > 1e-9 {
            let t = (sample_y - e.y0) / (e.y1 - e.y0);
            crossings.push((e.x0 + t * (e.x1 - e.x0), dir));
        }
    }
    if crossings.is_empty() {
        return;
    }
    crossings.sort_by(|a, b| a.0.total_cmp(&b.0));

    const SUB_X: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    for (px, cell) in row_hits.iter_mut().enumerate() {
        let base_x = f64::from(origin_x + (px as i32));
        for &sx in &SUB_X {
            if winding_at_x(crossings, base_x + sx) != 0 {
                *cell = cell.saturating_add(1);
            }
        }
    }
}

fn winding_at_x(crossings: &[(f64, i32)], sample_x: f64) -> i32 {
    let mut winding = 0i32;
    for &(cx, dir) in crossings {
        if cx <= sample_x {
            winding += dir;
        } else {
            break;
        }
    }
    winding
}

fn apply_synthetic_bold_dilation(alpha: &mut [u8], width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;
    for y in 0..h {
        let Some(row) = alpha.get_mut(y * w..(y + 1) * w) else {
            continue;
        };
        for x in (1..w).rev() {
            if let Some(&prev) = row.get(x - 1)
                && let Some(cur) = row.get_mut(x)
            {
                let shifted = (u16::from(prev) * 3 / 4).min(255) as u8;
                *cur = (*cur).max(shifted);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use curvebook_palette::Rgba8;
    use curvebook_render::RasterImage;

    use super::{DesktopTextMeasurer, PaintClipPx, SoftwareTextPainter, TextMeasurer, TextPaintStyle, UiTextStyle};

    #[test]
    fn proportional_mixed_case_tabular_and_unicode_shape_cleanly() {
        let measurer = DesktopTextMeasurer::new();
        let ui = UiTextStyle::ui_sans(13.0);

        let narrow = measurer.measure("iiii", &ui, "en");
        let wide = measurer.measure("MMMM", &ui, "en");
        assert!(wide.width_px > narrow.width_px, "proportional UI sans must distinguish 'i' and 'M'");

        let mixed = measurer.shape_line("Analyze (One-Way ANOVA)", &ui, "en");
        assert_eq!(mixed.visual_text, "Analyze (One-Way ANOVA)");
        assert_eq!(mixed.missing_glyph_count, 0);

        let tabular = UiTextStyle::grid_tabular(12.0);
        let w1 = measurer.measure("1111.1", &tabular, "en").width_px;
        let w8 = measurer.measure("8888.8", &tabular, "en").width_px;
        assert!((w1 - w8).abs() < 0.05, "tabular numerals must align column widths: {w1} vs {w8}");

        let uni = measurer.shape_line("α = 0.05, μ ± σ, χ² ≥ 1.96", &ui, "en");
        assert_eq!(uni.missing_glyph_count, 0, "scientific Greek/math symbols should resolve cleanly");
    }

    #[test]
    fn coverage_atlas_paints_distinct_mixed_case_and_italic_pixels() {
        let measurer = DesktopTextMeasurer::new();
        let mut img_a = RasterImage::new(160, 40, 96, Rgba8::WHITE).expect("img");
        let mut img_b = img_a.clone();
        let clip = PaintClipPx::new(0, 0, 160, 40);
        let style_a = UiTextStyle::ui_sans(13.0);
        let style_b = UiTextStyle::ui_sans(13.0).italic();

        SoftwareTextPainter::paint_text_top_left(
            &measurer,
            &mut img_a,
            "One-Way ANOVA",
            &TextPaintStyle { style: &style_a, locale: "en", color: Rgba8::rgb(17, 24, 39), clip },
            [6.0, 10.0],
        );
        SoftwareTextPainter::paint_text_top_left(
            &measurer,
            &mut img_b,
            "ONE-WAY ANOVA",
            &TextPaintStyle { style: &style_b, locale: "en", color: Rgba8::rgb(17, 24, 39), clip },
            [6.0, 10.0],
        );
        assert_ne!(img_a.rgba, img_b.rgba, "mixed-case upright must differ visually from uppercase/italic");
    }

    #[test]
    fn rtl_bidi_preserves_unicode_minus_formulas_and_signed_intervals() {
        let measurer = DesktopTextMeasurer::new();
        let style = UiTextStyle::ui_sans(13.0);
        let mono = UiTextStyle::ui_mono(13.0);

        for locale in ["ar", "he"] {
            let pub_neg = measurer.shape_line("\u{2212}2.5", &style, locale);
            assert_eq!(pub_neg.visual_text, "\u{2212}2.5");
            assert_eq!(pub_neg.missing_glyph_count, 0);

            let formula = measurer.shape_line("=A1*2", &mono, locale);
            assert_eq!(formula.visual_text, "=A1*2");
            assert_eq!(formula.missing_glyph_count, 0);

            let uni_formula = measurer.shape_line("=A1*\u{2212}2", &mono, locale);
            assert_eq!(uni_formula.visual_text, "=A1*\u{2212}2");
            assert_eq!(uni_formula.missing_glyph_count, 0);

            let embedded_formula = measurer.shape_line("الصيغة: =A1*\u{2212}2", &style, locale);
            assert!(embedded_formula.visual_text.contains("=A1*\u{2212}2"), "got {:?}", embedded_formula.visual_text);
            assert_eq!(embedded_formula.missing_glyph_count, 0);

            let embedded_neg = measurer.shape_line("النتيجة: \u{2212}2.5", &style, locale);
            assert!(embedded_neg.visual_text.contains("\u{2212}2.5"), "got {:?}", embedded_neg.visual_text);
            assert_eq!(embedded_neg.missing_glyph_count, 0);
        }

        let ar_ci = measurer.shape_line("فترة الثقة 95%: [-1.24, +3.85]", &style, "ar");
        assert!(ar_ci.visual_text.contains("[-1.24, +3.85]"), "ar CI visual: {:?}", ar_ci.visual_text);
        assert_eq!(ar_ci.missing_glyph_count, 0);
        assert!((measurer.measure("فترة الثقة 95%: [-1.24, +3.85]", &style, "ar").width_px - ar_ci.metrics.width_px).abs() < 1e-6);

        let he_ci = measurer.shape_line("רווח סמך 95%: [\u{2212}1.24, +3.85]", &style, "he");
        assert!(he_ci.visual_text.contains("[\u{2212}1.24, +3.85]"), "he CI visual: {:?}", he_ci.visual_text);
        assert_eq!(he_ci.missing_glyph_count, 0);

        // Existing mixed BiDi prose ordering remains intact.
        assert_eq!(measurer.shape_line("التركيز [IC50] = 5.2", &style, "ar").visual_text, "5.2 = [IC50] زيكرتلا");
        assert_eq!(measurer.shape_line("שלום (123)", &style, "he").visual_text, "(123) םולש");
        assert!(measurer.shape_line("احتمال p < 0.0001", &style, "ar").visual_text.contains("p < 0.0001"));
    }
}
