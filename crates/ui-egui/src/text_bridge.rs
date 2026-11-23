//! L6 Text Shaping and Measurement Bridge (`R10.9`, `#248` §2).
//!
//! Provides unified typography metrics and shaping behind [`TextMeasurer`]:
//! - Test/verification harnesses (`test-support` feature) use `DesktopTextMeasurer`, backed by
//!   [`crate::theme::FontBook`] (OS host fonts + `CURVEBOOK_FONTS_DIR` + `curvebook-text`
//!   clean-room glyph outlines).
//! - Native `egui` shell rendering consumes shaped metrics and fonts via `egui_shell::fonts`.

use curvebook_palette::Rgba8;
use curvebook_text::{RichText, SpanSlope, SpanWeight, TextDirection};
use serde::{Deserialize, Serialize};

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

    #[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
pub use desktop::DesktopTextMeasurer;

#[cfg(any(test, feature = "test-support"))]
mod desktop {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    use curvebook_text::{RichText, SpanScript, SpanWeight, TextDirection, contains_rtl_script, is_rtl_locale, measure_text_pt, reorder_visual_bidi};

    use super::{FontFamilyKind, MeasuredTextMetrics, ShapedGlyphRun, ShapedTextLine, TextMeasurer, UiTextStyle};
    use crate::theme::FontBook;

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
            let placeholder = CachedGlyphEntry {
                glyph_id: None,
                face_index: None,
                prop_em: 0.54,
                tabular_em: 0.60,
                used_vector_fallback: false,
                missing_glyph: false,
            };
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
        let tabular_digit =
            (style.tabular_nums || matches!(style.family, FontFamilyKind::GridTabular | FontFamilyKind::UiMono)) && ch.is_ascii_digit();
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

        while let Some(tail) = text.get(i..) {
            let Some(ch) = tail.chars().next() else {
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
                && let Some(end) =
                    match_bracketed_interval(text, i).or_else(|| match_formula_span(text, i)).or_else(|| match_signed_number_span(text, i))
                && let Some(span) = text.get(i..end)
            {
                out.push('\u{2066}');
                out.push_str(span);
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
        let slice = text.get(start..)?;
        let open = slice.chars().next()?;
        let close = match open {
            '[' => ']',
            '(' => ')',
            _ => return None,
        };
        let mut depth = 0usize;
        let mut end_idx = None;
        for (rel, ch) in slice.char_indices() {
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
        let inner = text.get(start + open.len_utf8()..end - close.len_utf8())?;
        let has_digit = inner.bytes().any(|b| b.is_ascii_digit());
        let has_sep = inner.contains(',') || inner.contains(';');
        if has_digit && (has_sep || has_sign_before_digit(inner)) { Some(end) } else { None }
    }

    fn match_formula_span(text: &str, start: usize) -> Option<usize> {
        let after_eq = text.get(start..)?.strip_prefix('=')?;
        let prev = text.get(..start).and_then(|s| s.chars().next_back());
        if let Some(p) = prev
            && !p.is_whitespace()
            && !matches!(p, ':' | '(' | '[' | '"' | '\'')
        {
            return None;
        }
        let first = after_eq.chars().next()?;
        if !(first.is_ascii_alphanumeric() || matches!(first, '_' | '(' | '-' | '+' | '\u{2212}' | '.' | '$')) {
            return None;
        }
        let mut depth = 0usize;
        let mut end = start + 1;
        let mut has_alnum = false;
        for (rel, ch) in after_eq.char_indices() {
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
            let last = text.get(start + 1..end)?.chars().next_back()?;
            if matches!(last, ',' | ';' | ':' | '.' | '!' | '?') {
                end -= last.len_utf8();
            } else {
                break;
            }
        }
        if has_alnum && end > start + 1 { Some(end) } else { None }
    }

    fn match_signed_number_span(text: &str, start: usize) -> Option<usize> {
        let sign = text.get(start..)?.chars().next()?;
        if !matches!(sign, '-' | '+' | '\u{2212}' | '\u{00B1}') {
            return None;
        }
        if let Some(prev) = text.get(..start).and_then(|s| s.chars().next_back())
            && prev.is_alphanumeric()
        {
            return None;
        }
        let mut p = start + sign.len_utf8();
        let mut saw_digit = false;
        if text.get(p..).is_some_and(|s| s.starts_with('.')) {
            p += 1;
        }
        while let Some(ch) = text.get(p..).and_then(|s| s.chars().next()) {
            if ch.is_ascii_digit() {
                saw_digit = true;
                p += 1;
            } else if matches!(ch, '.' | ',') && text.get(p + 1..).and_then(|s| s.chars().next()).is_some_and(|d| d.is_ascii_digit()) {
                p += 1;
            } else {
                break;
            }
        }
        if !saw_digit {
            return None;
        }
        if text.get(p..).is_some_and(|s| s.starts_with(['e', 'E'])) {
            let mut exp_p = p + 1;
            if let Some(es) = text.get(exp_p..).and_then(|s| s.chars().next())
                && matches!(es, '+' | '-' | '\u{2212}')
            {
                exp_p += es.len_utf8();
            }
            let mut exp_digits = false;
            while let Some(d) = text.get(exp_p..).and_then(|s| s.chars().next()) {
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
}

#[cfg(test)]
mod tests {
    use super::{DesktopTextMeasurer, TextMeasurer, UiTextStyle};

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
