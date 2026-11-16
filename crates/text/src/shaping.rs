//! Proportional UI, tabular-numeral grid, and multi-script text shaping (`R10.9`).
//!
//! Provides the `L2` [`TextMeasurer`] boundary shared by desktop, headless, and web text
//! pipelines alongside [`SystemFontSet`], a zero-bundled-font runtime measurer supporting
//! `LUMENCRAFT_FONTS_DIR` overrides, host SFNT/TTF/TTC discovery, and cleanroom Core-14 /
//! vector-outline fallbacks (wasm-safe and `--no-default-features` safe).

use std::borrow::Cow;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::BUNDLED_FONTS;
use crate::glyph_paths::has_cleanroom_vector_outline;
use crate::markup::{RichSpan, RichText, SpanSlope, SpanWeight, TextDirection, contains_rtl_script, parse_markup, reorder_visual_bidi};
use crate::metrics::{
    CachedSfntLayout, SfntFaceMeta, TrueTypeFont, core14_advance_em, effective_span_scale, inspect_sfnt_face_meta_at, is_combining_mark, read_u32_be,
    reorder_rich_text_bidi, rich_contains_rtl, script_baseline_offset_pt, split_rich_text_lines,
};

const BASE_UI_DPI: f64 = 96.0;
const MAX_LOGICAL_SIZE_PX: f64 = 10_000.0;
#[cfg(not(target_arch = "wasm32"))]
const MAX_HOST_FONT_BYTES: usize = 64 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const MAX_DIR_FONTS: usize = 64;

/// Semantic UI and scientific typography family categories (`R10.9`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FontFamilyKind {
    /// Mixed-case proportional sans-serif for menus, buttons, tabs, labels, and dialogs.
    #[default]
    UiSans,
    /// Fixed-width monospace for formulas, code, logs, and shortcut badges.
    UiMono,
    /// Proportional text with uniform tabular numeral/punctuation advances (`0`–`9`, `.`, `,`, `-`, `+`, `E`, `e`).
    GridTabular,
    /// Proportional serif for mathematical notation and publication captions.
    ScientificSerif,
}

impl FontFamilyKind {
    const ALL: [Self; 4] = [Self::UiSans, Self::UiMono, Self::GridTabular, Self::ScientificSerif];

    const fn index(self) -> usize {
        match self {
            Self::UiSans => 0,
            Self::UiMono => 1,
            Self::GridTabular => 2,
            Self::ScientificSerif => 3,
        }
    }
}

/// Style descriptor for proportional UI and grid text measurement and line shaping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiTextStyle {
    pub family: FontFamilyKind,
    pub size_px: f32,
    pub weight: SpanWeight,
    pub slope: SpanSlope,
    pub tabular_nums: bool,
    pub rich: bool,
}

impl UiTextStyle {
    /// Construct a regular upright style for `family` at logical `size_px`.
    #[must_use]
    pub const fn new(family: FontFamilyKind, size_px: f32) -> Self {
        Self {
            family,
            size_px,
            weight: SpanWeight::Regular,
            slope: SpanSlope::Upright,
            tabular_nums: matches!(family, FontFamilyKind::GridTabular),
            rich: false,
        }
    }

    /// Construct a proportional sans-serif UI style at logical `size_px`.
    #[must_use]
    pub const fn ui_sans(size_px: f32) -> Self {
        Self::new(FontFamilyKind::UiSans, size_px)
    }

    /// Construct a fixed-width monospace UI style at logical `size_px`.
    #[must_use]
    pub const fn ui_mono(size_px: f32) -> Self {
        Self::new(FontFamilyKind::UiMono, size_px)
    }

    /// Construct a tabular-numeral grid style at logical `size_px`.
    #[must_use]
    pub const fn grid_tabular(size_px: f32) -> Self {
        Self::new(FontFamilyKind::GridTabular, size_px)
    }

    /// Construct a serif style at logical `size_px`.
    #[must_use]
    pub const fn scientific_serif(size_px: f32) -> Self {
        Self::new(FontFamilyKind::ScientificSerif, size_px)
    }

    /// Return a copy with `weight` set.
    #[must_use]
    pub const fn with_weight(mut self, weight: SpanWeight) -> Self {
        self.weight = weight;
        self
    }

    /// Return a copy with `slope` set.
    #[must_use]
    pub const fn with_slope(mut self, slope: SpanSlope) -> Self {
        self.slope = slope;
        self
    }

    /// Return a copy with `tabular_nums` set.
    #[must_use]
    pub const fn with_tabular_nums(mut self, tabular_nums: bool) -> Self {
        self.tabular_nums = tabular_nums;
        self
    }

    /// Return a copy with `rich` markup parsing enabled or disabled.
    #[must_use]
    pub const fn with_rich(mut self, rich: bool) -> Self {
        self.rich = rich;
        self
    }
}

impl Default for UiTextStyle {
    fn default() -> Self {
        Self::ui_sans(13.0)
    }
}

/// Measured typographic bounds in physical pixels (`px`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct MeasuredTextMetrics {
    pub width_px: f64,
    pub ascent_px: f64,
    pub descent_px: f64,
    pub line_height_px: f64,
}

impl MeasuredTextMetrics {
    /// Total typographic box height (`ascent_px + descent_px`).
    #[must_use]
    pub const fn height_px(self) -> f64 {
        self.ascent_px + self.descent_px
    }
}

/// A single visually positioned glyph emitted by [`TextMeasurer::shape_line`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShapedGlyphRun {
    pub ch: char,
    pub x_px: f64,
    pub y_px: f64,
    pub advance_px: f64,
    pub glyph_id: Option<u16>,
    pub used_vector_fallback: bool,
    pub missing_glyph: bool,
}

/// Complete visually ordered line and glyph-coverage telemetry produced by [`TextMeasurer::shape_line`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ShapedTextLine {
    pub visual_text: String,
    pub direction: TextDirection,
    pub metrics: MeasuredTextMetrics,
    pub glyphs: Vec<ShapedGlyphRun>,
    pub missing_glyph_count: usize,
}

/// Host-abstracted proportional/tabular text measurement and shaping trait (`L2`).
pub trait TextMeasurer {
    /// Measure bounding metrics in physical pixels at `dpi` (`96` = `1.0x`, `144` = `1.5x`, `192` = `2.0x`, `288` = `3.0x`).
    fn measure(&self, style: &UiTextStyle, text: &str, dpi: u16) -> MeasuredTextMetrics;

    /// Shape `text` into visually ordered glyphs with coverage tracking (`glyph_id`, `used_vector_fallback`, `missing_glyph`).
    fn shape_line(&self, style: &UiTextStyle, text: &str, dpi: u16) -> ShapedTextLine;
}

impl<T: TextMeasurer + ?Sized> TextMeasurer for &T {
    fn measure(&self, style: &UiTextStyle, text: &str, dpi: u16) -> MeasuredTextMetrics {
        (*self).measure(style, text, dpi)
    }

    fn shape_line(&self, style: &UiTextStyle, text: &str, dpi: u16) -> ShapedTextLine {
        (*self).shape_line(style, text, dpi)
    }
}

impl<T: TextMeasurer + ?Sized> TextMeasurer for Box<T> {
    fn measure(&self, style: &UiTextStyle, text: &str, dpi: u16) -> MeasuredTextMetrics {
        (**self).measure(style, text, dpi)
    }

    fn shape_line(&self, style: &UiTextStyle, text: &str, dpi: u16) -> ShapedTextLine {
        (**self).shape_line(style, text, dpi)
    }
}

impl<T: TextMeasurer + ?Sized> TextMeasurer for Arc<T> {
    fn measure(&self, style: &UiTextStyle, text: &str, dpi: u16) -> MeasuredTextMetrics {
        (**self).measure(style, text, dpi)
    }

    fn shape_line(&self, style: &UiTextStyle, text: &str, dpi: u16) -> ShapedTextLine {
        (**self).shape_line(style, text, dpi)
    }
}

#[derive(Debug, Clone)]
enum FontBytes {
    Static(&'static [u8]),
    Shared(Arc<Vec<u8>>),
}

impl FontBytes {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Static(bytes) => bytes,
            Self::Shared(arc) => arc.as_slice(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FontSourceTier {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    HostScriptFallback,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    HostPrimary,
    Bundled,
    EnvOverride,
}

#[derive(Debug, Clone)]
struct LoadedFontFace {
    name: String,
    bytes: FontBytes,
    sfnt_offset: usize,
    layout: CachedSfntLayout,
    source_tier: FontSourceTier,
    family_kind: FontFamilyKind,
    meta: SfntFaceMeta,
    ascent_em: f64,
    descent_em: f64,
    line_gap_em: f64,
    tabular_digit_em: f64,
    mono_cell_em: f64,
    is_script_fallback: bool,
}

impl LoadedFontFace {
    fn parsed(&self) -> Option<TrueTypeFont<'_>> {
        self.layout.font(self.bytes.as_slice())
    }
}

/// System/environment SFNT font collection implementing [`TextMeasurer`] without bundled font binaries.
///
/// Discovers fonts in priority order:
/// 1. Runtime `LUMENCRAFT_FONTS_DIR` override (`fonts/manifest.txt`, `manifest.txt`, or font files)
/// 2. Optional compile-time `BUNDLED_FONTS` when the `bundled-fonts` feature is enabled
/// 3. Host OS SFNT/TTF/TTC fonts on native platforms (no-op on `wasm32`)
/// 4. Deterministic cleanroom Core-14 metrics and `glyph_paths` vector outlines
#[derive(Debug, Clone)]
pub struct SystemFontSet {
    faces: Vec<Arc<LoadedFontFace>>,
    orders: [Vec<usize>; 16],
}

impl Default for SystemFontSet {
    fn default() -> Self {
        Self::from_env_and_system()
    }
}

impl SystemFontSet {
    /// Construct a [`SystemFontSet`] using runtime `LUMENCRAFT_FONTS_DIR`, optional `BUNDLED_FONTS`,
    /// and host OS system fonts (or cleanroom fallback on `wasm32`).
    #[must_use]
    pub fn new() -> Self {
        Self::from_env_and_system()
    }

    /// Construct a [`SystemFontSet`] from `LUMENCRAFT_FONTS_DIR` (if set), optional `BUNDLED_FONTS`,
    /// and cached host OS system fonts.
    #[must_use]
    pub fn from_env_and_system() -> Self {
        let mut faces = Vec::new();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(env_dir) = std::env::var_os("LUMENCRAFT_FONTS_DIR").filter(|v| !v.is_empty()) {
            load_faces_from_dir(std::path::Path::new(&env_dir), FontSourceTier::EnvOverride, &mut faces);
        }
        append_bundled_faces(&mut faces);
        #[cfg(not(target_arch = "wasm32"))]
        faces.extend(cached_host_system_faces().iter().cloned());
        Self::from_loaded(faces)
    }

    /// Construct a [`SystemFontSet`] loading SFNT/TTF/TTC faces solely from `dir` (without host OS fallback).
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn from_fonts_dir(dir: &std::path::Path) -> Self {
        let mut faces = Vec::new();
        load_faces_from_dir(dir, FontSourceTier::EnvOverride, &mut faces);
        Self::from_loaded(faces)
    }

    /// Construct a [`SystemFontSet`] from caller-supplied in-memory SFNT/TTF/TTC buffers (wasm-safe).
    #[must_use]
    pub fn from_sfnt_bytes(entries: &[(&str, &[u8])]) -> Self {
        let mut faces = Vec::new();
        for &(name, bytes) in entries {
            let shared = FontBytes::Shared(Arc::new(bytes.to_vec()));
            append_faces_from_bytes(name, &shared, FontSourceTier::EnvOverride, false, &mut faces);
        }
        Self::from_loaded(faces)
    }

    /// Construct a [`SystemFontSet`] with zero SFNT faces (deterministic Core-14 + vector outlines only).
    #[must_use]
    pub const fn cleanroom_only() -> Self {
        const EMPTY: [Vec<usize>; 16] = [const { Vec::new() }; 16];
        Self { faces: Vec::new(), orders: EMPTY }
    }

    fn from_loaded(faces: Vec<Arc<LoadedFontFace>>) -> Self {
        let orders = build_face_order_table(&faces);
        Self { faces, orders }
    }

    /// Total number of loaded SFNT font faces.
    #[must_use]
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Returns `true` when at least one valid SFNT font face is loaded.
    #[must_use]
    pub fn has_sfnt_faces(&self) -> bool {
        !self.faces.is_empty()
    }

    /// Names or paths of loaded SFNT font faces in registration order.
    #[must_use]
    pub fn face_names(&self) -> Vec<&str> {
        self.faces.iter().map(|f| f.name.as_str()).collect()
    }

    /// Return the raw SFNT buffer and sub-font byte offset for `face_idx`.
    #[must_use]
    pub fn face_bytes(&self, face_idx: usize) -> Option<(&[u8], usize)> {
        let face = self.faces.get(face_idx)?;
        Some((face.bytes.as_slice(), face.sfnt_offset))
    }

    /// Parse face `face_idx` as a zero-copy [`TrueTypeFont`] reader.
    #[must_use]
    pub fn parsed_face(&self, face_idx: usize) -> Option<TrueTypeFont<'_>> {
        self.faces.get(face_idx)?.parsed()
    }

    /// Resolve the preferred loaded face index and non-zero SFNT glyph ID for `ch` under `style`.
    #[must_use]
    pub fn resolve_face_for_char(&self, style: &UiTextStyle, ch: char) -> Option<(usize, u16)> {
        let order = self.face_order(style.family, style.weight, style.slope);
        for &idx in order {
            if let Some(ttf) = self.faces.get(idx).and_then(|f| f.parsed())
                && let Some(gid) = ttf.glyph_id(ch)
            {
                return Some((idx, gid));
            }
        }
        None
    }

    /// Shape `text` using an explicit [`TextDirection`] override (`None` infers direction from strong characters).
    #[must_use]
    pub fn shape_line_with_direction(&self, style: &UiTextStyle, text: &str, dpi: u16, direction: Option<TextDirection>) -> ShapedTextLine {
        self.shape_internal(style, text, dpi, direction, true)
    }

    fn face_order(&self, family: FontFamilyKind, weight: SpanWeight, slope: SpanSlope) -> &[usize] {
        let slot = style_slot_index(family, weight, slope);
        self.orders.get(slot).map_or(&[], Vec::as_slice)
    }

    fn primary_metrics_em(&self, family: FontFamilyKind, weight: SpanWeight, slope: SpanSlope) -> LineEmMetrics {
        if let Some(&first_idx) = self.face_order(family, weight, slope).first()
            && let Some(face) = self.faces.get(first_idx)
        {
            return LineEmMetrics {
                ascent_em: face.ascent_em,
                descent_em: face.descent_em,
                line_gap_em: face.line_gap_em,
                tabular_digit_em: face.tabular_digit_em,
                mono_cell_em: face.mono_cell_em,
            };
        }
        LineEmMetrics::cleanroom_default()
    }

    fn shape_internal(&self, style: &UiTextStyle, text: &str, dpi: u16, dir_override: Option<TextDirection>, collect_glyphs: bool) -> ShapedTextLine {
        let eff_px = effective_size_px(style.size_px, dpi);
        let fallback_dir = dir_override.unwrap_or_else(|| infer_text_direction(text));
        if eff_px <= 0.0 {
            return ShapedTextLine { direction: fallback_dir, ..Default::default() };
        }

        let em = self.primary_metrics_em(style.family, style.weight, style.slope);
        let base_ascent = em.ascent_em * eff_px;
        let base_descent = em.descent_em * eff_px;
        let base_line_height = ((em.ascent_em + em.descent_em + em.line_gap_em).max(1.20)) * eff_px;

        if text.is_empty() {
            return ShapedTextLine {
                visual_text: String::new(),
                direction: fallback_dir,
                metrics: MeasuredTextMetrics { width_px: 0.0, ascent_px: base_ascent, descent_px: base_descent, line_height_px: base_line_height },
                glyphs: Vec::new(),
                missing_glyph_count: 0,
            };
        }

        if style.rich {
            self.shape_rich_text(style, text, eff_px, em, dir_override, collect_glyphs)
        } else {
            self.shape_plain_text(style, text, eff_px, em, fallback_dir, collect_glyphs)
        }
    }

    fn shape_plain_text(
        &self,
        style: &UiTextStyle,
        text: &str,
        eff_px: f64,
        em: LineEmMetrics,
        direction: TextDirection,
        collect_glyphs: bool,
    ) -> ShapedTextLine {
        let base_ascent = em.ascent_em * eff_px;
        let base_descent = em.descent_em * eff_px;
        let line_height = ((em.ascent_em + em.descent_em + em.line_gap_em).max(1.20)) * eff_px;

        let mut visual_text = if collect_glyphs { String::with_capacity(text.len()) } else { String::new() };
        let mut glyphs = if collect_glyphs { Vec::with_capacity(text.len()) } else { Vec::new() };
        let mut max_width = 0.0_f64;
        let mut prev_pen_x = 0.0_f64;
        let mut missing_count = 0usize;
        let mut line_count = 0usize;
        let order = self.face_order(style.family, style.weight, style.slope);

        for (line_idx, raw_line) in text.split('\n').enumerate() {
            line_count = line_idx + 1;
            let cleaned_owned;
            let clean_line = if raw_line.contains('\r') {
                cleaned_owned = raw_line.replace('\r', "");
                cleaned_owned.as_str()
            } else {
                raw_line
            };
            if line_idx > 0 && collect_glyphs {
                visual_text.push('\n');
                let prev_y = ((line_idx - 1) as f64) * line_height;
                glyphs.push(ShapedGlyphRun {
                    ch: '\n',
                    x_px: prev_pen_x,
                    y_px: prev_y,
                    advance_px: 0.0,
                    glyph_id: None,
                    used_vector_fallback: false,
                    missing_glyph: false,
                });
            }
            let reordered;
            let visual_line = if direction == TextDirection::Rtl || contains_rtl_script(clean_line) {
                reordered = reorder_visual_bidi(clean_line, direction);
                reordered.as_str()
            } else {
                clean_line
            };
            if collect_glyphs {
                visual_text.push_str(visual_line);
            }

            let y_line = (line_idx as f64) * line_height;
            let mut pen_x = 0.0_f64;
            for ch in visual_line.chars() {
                let run = self.shape_single_char(style, style.weight, style.slope, ch, pen_x, y_line, eff_px, em, order);
                if run.missing_glyph {
                    missing_count += 1;
                }
                pen_x += run.advance_px;
                if collect_glyphs {
                    glyphs.push(run);
                }
            }
            prev_pen_x = pen_x;
            max_width = max_width.max(pen_x);
        }

        let extra_lines = line_count.saturating_sub(1) as f64;
        ShapedTextLine {
            visual_text,
            direction,
            metrics: MeasuredTextMetrics {
                width_px: max_width,
                ascent_px: base_ascent,
                descent_px: base_descent + extra_lines * line_height,
                line_height_px: line_height,
            },
            glyphs,
            missing_glyph_count: missing_count,
        }
    }

    fn shape_rich_text(
        &self,
        style: &UiTextStyle,
        text: &str,
        eff_px: f64,
        em: LineEmMetrics,
        dir_override: Option<TextDirection>,
        collect_glyphs: bool,
    ) -> ShapedTextLine {
        let parsed = apply_style_to_rich(parse_markup(text), style);
        let direction = dir_override.unwrap_or_else(|| infer_rich_direction(&parsed));
        let lines = split_rich_text_lines(&parsed);
        let base_ascent = em.ascent_em * eff_px;
        let base_descent = em.descent_em * eff_px;
        let base_line_height = ((em.ascent_em + em.descent_em + em.line_gap_em).max(1.20)) * eff_px;

        if lines.is_empty() {
            return ShapedTextLine {
                visual_text: String::new(),
                direction,
                metrics: MeasuredTextMetrics { width_px: 0.0, ascent_px: base_ascent, descent_px: base_descent, line_height_px: base_line_height },
                glyphs: Vec::new(),
                missing_glyph_count: 0,
            };
        }

        let (max_ascent, max_descent) = compute_rich_vertical_bounds(&lines, eff_px, em, base_ascent, base_descent);
        let line_height = base_line_height.max(max_ascent + max_descent);

        let mut visual_text = if collect_glyphs { String::with_capacity(text.len()) } else { String::new() };
        let mut glyphs = if collect_glyphs { Vec::with_capacity(text.len()) } else { Vec::new() };
        let mut max_width = 0.0_f64;
        let mut prev_pen_x = 0.0_f64;
        let mut missing_count = 0usize;

        for (line_idx, raw_line) in lines.iter().enumerate() {
            if line_idx > 0 && collect_glyphs {
                visual_text.push('\n');
                let prev_y = ((line_idx - 1) as f64) * line_height;
                glyphs.push(ShapedGlyphRun {
                    ch: '\n',
                    x_px: prev_pen_x,
                    y_px: prev_y,
                    advance_px: 0.0,
                    glyph_id: None,
                    used_vector_fallback: false,
                    missing_glyph: false,
                });
            }
            let visual_line = reorder_rich_line(raw_line, direction);
            let mut pen_x = 0.0_f64;
            let y_line = (line_idx as f64) * line_height;
            for span in &visual_line.spans {
                let glyph_px = eff_px * effective_span_scale(span);
                let y_span = y_line + script_baseline_offset_pt(span.script, eff_px);
                let order = self.face_order(style.family, span.weight, span.slope);
                for ch in span.text.chars() {
                    if collect_glyphs {
                        visual_text.push(ch);
                    }
                    let run = self.shape_single_char(style, span.weight, span.slope, ch, pen_x, y_span, glyph_px, em, order);
                    if run.missing_glyph {
                        missing_count += 1;
                    }
                    pen_x += run.advance_px;
                    if collect_glyphs {
                        glyphs.push(run);
                    }
                }
            }
            prev_pen_x = pen_x;
            max_width = max_width.max(pen_x);
        }

        let extra_lines = lines.len().saturating_sub(1) as f64;
        ShapedTextLine {
            visual_text,
            direction,
            metrics: MeasuredTextMetrics {
                width_px: max_width,
                ascent_px: max_ascent,
                descent_px: max_descent + extra_lines * line_height,
                line_height_px: line_height,
            },
            glyphs,
            missing_glyph_count: missing_count,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_single_char(
        &self,
        style: &UiTextStyle,
        weight: SpanWeight,
        slope: SpanSlope,
        ch: char,
        x_px: f64,
        y_px: f64,
        glyph_px: f64,
        em: LineEmMetrics,
        order: &[usize],
    ) -> ShapedGlyphRun {
        if ch == '\t' {
            let space_run = self.shape_single_char(style, weight, slope, ' ', x_px, y_px, glyph_px, em, order);
            return ShapedGlyphRun {
                ch: '\t',
                x_px,
                y_px,
                advance_px: 4.0 * space_run.advance_px,
                glyph_id: None,
                used_vector_fallback: false,
                missing_glyph: false,
            };
        }
        if ch.is_control() || is_zero_width_format(ch) {
            return ShapedGlyphRun { ch, x_px, y_px, advance_px: 0.0, glyph_id: None, used_vector_fallback: false, missing_glyph: false };
        }

        let mut matched_gid = None;
        let mut sfnt_adv_em = None;
        for &idx in order {
            if let Some(ttf) = self.faces.get(idx).and_then(|f| f.parsed())
                && let Some(gid) = ttf.glyph_id(ch)
            {
                matched_gid = Some(gid);
                sfnt_adv_em = ttf.advance_em(ch);
                break;
            }
        }

        let adv_em = compute_styled_advance_em(style, weight, slope, ch, sfnt_adv_em, em);
        let (used_vector_fallback, missing_glyph) = if matched_gid.is_some() || ch.is_whitespace() {
            (false, false)
        } else if has_cleanroom_vector_outline(ch) {
            (true, false)
        } else {
            (false, true)
        };

        ShapedGlyphRun { ch, x_px, y_px, advance_px: adv_em * glyph_px, glyph_id: matched_gid, used_vector_fallback, missing_glyph }
    }
}

impl TextMeasurer for SystemFontSet {
    fn measure(&self, style: &UiTextStyle, text: &str, dpi: u16) -> MeasuredTextMetrics {
        self.shape_internal(style, text, dpi, None, false).metrics
    }

    fn shape_line(&self, style: &UiTextStyle, text: &str, dpi: u16) -> ShapedTextLine {
        self.shape_internal(style, text, dpi, None, true)
    }
}

#[derive(Debug, Clone, Copy)]
struct LineEmMetrics {
    ascent_em: f64,
    descent_em: f64,
    line_gap_em: f64,
    tabular_digit_em: f64,
    mono_cell_em: f64,
}

impl LineEmMetrics {
    const fn cleanroom_default() -> Self {
        Self { ascent_em: 0.80, descent_em: 0.20, line_gap_em: 0.20, tabular_digit_em: 0.556, mono_cell_em: 0.600 }
    }
}

const fn style_slot_index(family: FontFamilyKind, weight: SpanWeight, slope: SpanSlope) -> usize {
    let w = match weight {
        SpanWeight::Regular => 0,
        SpanWeight::Bold => 1,
    };
    let s = match slope {
        SpanSlope::Upright => 0,
        SpanSlope::Italic => 1,
    };
    family.index() * 4 + w * 2 + s
}

fn build_face_order_table(faces: &[Arc<LoadedFontFace>]) -> [Vec<usize>; 16] {
    let mut table = [const { Vec::new() }; 16];
    for family in FontFamilyKind::ALL {
        for weight in [SpanWeight::Regular, SpanWeight::Bold] {
            for slope in [SpanSlope::Upright, SpanSlope::Italic] {
                let mut scored: Vec<(i32, usize)> =
                    faces.iter().enumerate().map(|(idx, face)| (score_face(face, family, weight, slope), idx)).collect();
                scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
                let slot = style_slot_index(family, weight, slope);
                if let Some(target) = table.get_mut(slot) {
                    *target = scored.into_iter().map(|(_, idx)| idx).collect();
                }
            }
        }
    }
    table
}

fn effective_size_px(size_px: f32, dpi: u16) -> f64 {
    let raw = f64::from(size_px);
    if !raw.is_finite() || raw <= 0.0 {
        return 0.0;
    }
    let clamped_px = raw.min(MAX_LOGICAL_SIZE_PX);
    let eff_dpi = if dpi == 0 { BASE_UI_DPI } else { f64::from(dpi) };
    clamped_px * (eff_dpi / BASE_UI_DPI)
}

fn compute_styled_advance_em(
    style: &UiTextStyle,
    weight: SpanWeight,
    slope: SpanSlope,
    ch: char,
    sfnt_adv_em: Option<f64>,
    em: LineEmMetrics,
) -> f64 {
    if is_combining_mark(ch) || is_rtl_combining_mark(ch) {
        return 0.0;
    }
    let natural = sfnt_adv_em.unwrap_or_else(|| core14_advance_em(ch, weight, slope));
    if style.family == FontFamilyKind::UiMono {
        return if is_wide_cjk_char(ch) { natural.max(em.mono_cell_em) } else { em.mono_cell_em };
    }
    let use_tabular = style.tabular_nums || style.family == FontFamilyKind::GridTabular;
    if use_tabular && is_tabular_numeral_char(ch) {
        return em.tabular_digit_em;
    }
    natural
}

const fn is_tabular_numeral_char(ch: char) -> bool {
    matches!(ch, '0'..='9' | '.' | ',' | '-' | '+' | 'E' | 'e' | '\u{2212}')
}

const fn is_rtl_combining_mark(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0591..=0x05BD
            | 0x05BF
            | 0x05C1..=0x05C2
            | 0x05C4..=0x05C5
            | 0x05C7
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06ED
    )
}

const fn is_zero_width_format(ch: char) -> bool {
    matches!(ch as u32, 0x200B..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069 | 0xFEFF)
}

const fn is_wide_cjk_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF01..=0xFF60
    )
}

fn infer_text_direction(text: &str) -> TextDirection {
    let mut buf = [0u8; 4];
    for ch in text.chars() {
        if contains_rtl_script(ch.encode_utf8(&mut buf)) {
            return TextDirection::Rtl;
        }
        if ch.is_alphabetic() {
            return TextDirection::Ltr;
        }
    }
    TextDirection::Ltr
}

fn infer_rich_direction(rich: &RichText) -> TextDirection {
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

fn apply_style_to_rich(mut rich: RichText, style: &UiTextStyle) -> RichText {
    if style.weight == SpanWeight::Bold || style.slope == SpanSlope::Italic {
        for span in &mut rich.spans {
            if style.weight == SpanWeight::Bold {
                span.weight = SpanWeight::Bold;
            }
            if style.slope == SpanSlope::Italic {
                span.slope = SpanSlope::Italic;
            }
        }
    }
    rich
}

fn reorder_rich_line<'a>(line: &'a RichText, direction: TextDirection) -> Cow<'a, RichText> {
    if direction == TextDirection::Rtl || rich_contains_rtl(line) {
        if direction == TextDirection::Rtl && !rich_contains_rtl(line) {
            let reordered_spans =
                line.spans.iter().map(|span| RichSpan { text: reorder_visual_bidi(&span.text, TextDirection::Rtl), ..span.clone() }).collect();
            Cow::Owned(RichText::new(reordered_spans))
        } else {
            Cow::Owned(reorder_rich_text_bidi(line))
        }
    } else {
        Cow::Borrowed(line)
    }
}

fn compute_rich_vertical_bounds(lines: &[RichText], eff_px: f64, em: LineEmMetrics, base_ascent: f64, base_descent: f64) -> (f64, f64) {
    let mut max_ascent = base_ascent;
    let mut max_descent = base_descent;
    for line in lines {
        for span in &line.spans {
            if span.text.is_empty() {
                continue;
            }
            let glyph_px = eff_px * effective_span_scale(span);
            let y_offset = script_baseline_offset_pt(span.script, eff_px);
            max_ascent = max_ascent.max(-y_offset + em.ascent_em * glyph_px);
            max_descent = max_descent.max(y_offset + em.descent_em * glyph_px);
        }
    }
    (max_ascent, max_descent)
}

fn score_face(face: &LoadedFontFace, family: FontFamilyKind, weight: SpanWeight, slope: SpanSlope) -> i32 {
    let mut score = match face.source_tier {
        FontSourceTier::EnvOverride => 1000,
        FontSourceTier::Bundled => 500,
        FontSourceTier::HostPrimary => 100,
        FontSourceTier::HostScriptFallback => 0,
    };

    score += match (family, face.family_kind) {
        (FontFamilyKind::UiMono, FontFamilyKind::UiMono) => 60,
        (FontFamilyKind::ScientificSerif, FontFamilyKind::ScientificSerif) => 60,
        (FontFamilyKind::GridTabular, FontFamilyKind::GridTabular) => 65,
        (FontFamilyKind::GridTabular, FontFamilyKind::UiSans) => 60,
        (FontFamilyKind::UiSans, FontFamilyKind::UiSans | FontFamilyKind::GridTabular) => 60,
        (FontFamilyKind::UiMono, _) if face.meta.is_fixed_pitch => 50,
        (FontFamilyKind::ScientificSerif, _) if face.meta.is_serif => 50,
        (FontFamilyKind::UiSans | FontFamilyKind::GridTabular, _) if !face.meta.is_fixed_pitch && !face.meta.is_serif => 40,
        _ => 0,
    };

    if face.meta.is_bold == (weight == SpanWeight::Bold) {
        score += 20;
    }
    if face.meta.is_italic == (slope == SpanSlope::Italic) {
        score += 20;
    }
    if !face.meta.is_variable {
        score += 5;
    }
    if !face.is_script_fallback {
        score += 10;
    }
    score
}

fn classify_face_family(name: &str, meta: &SfntFaceMeta) -> FontFamilyKind {
    let lower = format!("{name} {}", meta.family_name).to_ascii_lowercase();
    if meta.is_fixed_pitch
        || lower.contains("mono")
        || lower.contains("courier")
        || lower.contains("consola")
        || lower.contains("menlo")
        || lower.contains("monaco")
    {
        FontFamilyKind::UiMono
    } else if meta.is_serif
        || lower.contains("serif")
        || lower.contains("times")
        || lower.contains("new york")
        || lower.contains("newyork")
        || lower.contains("georgia")
        || lower.contains("cambria")
    {
        FontFamilyKind::ScientificSerif
    } else if lower.contains("tabular") || lower.contains("grid") {
        FontFamilyKind::GridTabular
    } else {
        FontFamilyKind::UiSans
    }
}

fn compute_tabular_digit_em(ttf: &TrueTypeFont<'_>) -> f64 {
    let mut max_w = 0.0_f64;
    let mut any = false;
    for d in '0'..='9' {
        if let Some(adv) = ttf.advance_em(d)
            && adv.is_finite()
            && adv > 0.0
        {
            max_w = max_w.max(adv);
            any = true;
        }
    }
    if any { max_w.clamp(0.42, 0.80) } else { 0.556 }
}

fn compute_mono_cell_em(ttf: &TrueTypeFont<'_>) -> f64 {
    ttf.advance_em('0').or_else(|| ttf.advance_em('M')).or_else(|| ttf.advance_em(' ')).filter(|w| w.is_finite() && *w > 0.0).unwrap_or(0.600)
}

fn collect_sfnt_offsets(slice: &[u8]) -> Vec<usize> {
    if slice.get(0..4) == Some(b"ttcf") {
        let Some(num_raw) = read_u32_be(slice, 8) else {
            return Vec::new();
        };
        let count = usize::try_from(num_raw).unwrap_or(0).min(64);
        let mut offsets = Vec::with_capacity(count);
        for i in 0..count {
            let Some(off) = read_u32_be(slice, 12 + i * 4).and_then(|v| usize::try_from(v).ok()) else {
                break;
            };
            offsets.push(off);
        }
        return offsets;
    }
    vec![0]
}

fn append_faces_from_bytes(name: &str, bytes: &FontBytes, source_tier: FontSourceTier, is_script_fallback: bool, out: &mut Vec<Arc<LoadedFontFace>>) {
    let slice = bytes.as_slice();
    for sfnt_offset in collect_sfnt_offsets(slice) {
        let Some(layout) = CachedSfntLayout::extract(slice, sfnt_offset) else {
            continue;
        };
        let Some(ttf) = layout.font(slice) else {
            continue;
        };
        let upem = f64::from(ttf.units_per_em().max(1));
        let ascent_em = (f64::from(ttf.ascent()) / upem).clamp(0.5, 1.25);
        let descent_em = (f64::from(-ttf.descent()) / upem).clamp(0.1, 0.65);
        let line_gap_em = (f64::from(ttf.line_gap()) / upem).clamp(0.0, 0.50);
        let meta = inspect_sfnt_face_meta_at(slice, sfnt_offset, name);
        let family_kind = classify_face_family(name, &meta);
        let tabular_digit_em = compute_tabular_digit_em(&ttf);
        let mono_cell_em = compute_mono_cell_em(&ttf);
        let face_name = if sfnt_offset == 0 { name.to_owned() } else { format!("{name}@{sfnt_offset}") };
        out.push(Arc::new(LoadedFontFace {
            name: face_name,
            bytes: bytes.clone(),
            sfnt_offset,
            layout,
            source_tier,
            family_kind,
            meta,
            ascent_em,
            descent_em,
            line_gap_em,
            tabular_digit_em,
            mono_cell_em,
            is_script_fallback,
        }));
    }
}

fn append_bundled_faces(out: &mut Vec<Arc<LoadedFontFace>>) {
    if !cfg!(feature = "bundled-fonts") {
        return;
    }
    for &(name, bytes) in BUNDLED_FONTS {
        if !name.is_empty() && !bytes.is_empty() {
            append_faces_from_bytes(name, &FontBytes::Static(bytes), FontSourceTier::Bundled, false, out);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_faces_from_dir(dir: &std::path::Path, tier: FontSourceTier, out: &mut Vec<Arc<LoadedFontFace>>) {
    let mut loaded_paths = Vec::new();
    let manifest_specs = [(dir.join("fonts").join("manifest.txt"), dir.join("fonts")), (dir.join("manifest.txt"), dir.to_path_buf())];
    for (manifest_path, base_dir) in manifest_specs {
        if let Ok(text) = read_regular_text_bounded(&manifest_path, 256 * 1024) {
            for line in text.lines() {
                let rel = line.split('#').next().unwrap_or_default().trim();
                if rel.is_empty() || rel.contains("..") {
                    continue;
                }
                let candidate = base_dir.join(rel);
                if load_font_file(&candidate, tier, false, out) {
                    loaded_paths.push(candidate);
                }
            }
        }
    }

    let mut discovered = Vec::new();
    collect_font_files_bounded(dir, 0, &mut discovered);
    discovered.sort();
    for path in discovered {
        if !loaded_paths.contains(&path) {
            let _ = load_font_file(&path, tier, false, out);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_font_files_bounded(dir: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
    if depth > 3 || out.len() >= MAX_DIR_FONTS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if out.len() >= MAX_DIR_FONTS {
            break;
        }
        if path.is_dir() {
            collect_font_files_bounded(&path, depth + 1, out);
        } else if is_sfnt_extension(&path) {
            out.push(path);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn is_sfnt_extension(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf") || ext.eq_ignore_ascii_case("ttc"))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_font_file(path: &std::path::Path, tier: FontSourceTier, is_script_fallback: bool, out: &mut Vec<Arc<LoadedFontFace>>) -> bool {
    let Some(bytes) = read_regular_bytes_bounded(path, MAX_HOST_FONT_BYTES) else {
        return false;
    };
    let before = out.len();
    let label = path.to_string_lossy();
    let shared = FontBytes::Shared(Arc::new(bytes));
    append_faces_from_bytes(&label, &shared, tier, is_script_fallback, out);
    out.len() > before
}

#[cfg(not(target_arch = "wasm32"))]
fn read_regular_text_bounded(path: &std::path::Path, max_bytes: usize) -> Result<String, ()> {
    let bytes = read_regular_bytes_bounded(path, max_bytes).ok_or(())?;
    String::from_utf8(bytes).map_err(|_| ())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_regular_bytes_bounded(path: &std::path::Path, max_bytes: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let pre_meta = std::fs::metadata(path).ok()?;
    if !pre_meta.file_type().is_file() || pre_meta.len() > u64::try_from(max_bytes).unwrap_or(u64::MAX) {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let fd_meta = file.metadata().ok()?;
    if !fd_meta.file_type().is_file() || fd_meta.len() > u64::try_from(max_bytes).unwrap_or(u64::MAX) {
        return None;
    }
    let cap = usize::try_from(fd_meta.len()).unwrap_or(0).min(max_bytes);
    let mut bytes = Vec::with_capacity(cap);
    let take_cap = u64::try_from(max_bytes).unwrap_or(u64::MAX).saturating_add(1);
    file.take(take_cap).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= max_bytes).then_some(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn cached_host_system_faces() -> &'static Vec<Arc<LoadedFontFace>> {
    static HOST_FACES: OnceLock<Vec<Arc<LoadedFontFace>>> = OnceLock::new();
    HOST_FACES.get_or_init(discover_host_system_faces)
}

#[cfg(not(target_arch = "wasm32"))]
fn discover_host_system_faces() -> Vec<Arc<LoadedFontFace>> {
    const PRIMARY_CANDIDATES: &[&str] = &[
        // macOS sans / mono / serif
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/System/Library/Fonts/Supplemental/Arial Italic.ttf",
        "/System/Library/Fonts/Supplemental/Arial Bold Italic.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "/System/Library/Fonts/Geneva.ttf",
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/SFNSItalic.ttf",
        "/System/Library/Fonts/Supplemental/Courier New.ttf",
        "/System/Library/Fonts/Supplemental/Courier New Bold.ttf",
        "/System/Library/Fonts/Supplemental/Courier New Italic.ttf",
        "/System/Library/Fonts/Supplemental/Courier New Bold Italic.ttf",
        "/System/Library/Fonts/SFNSMono.ttf",
        "/System/Library/Fonts/Monaco.ttf",
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
        "/System/Library/Fonts/Supplemental/Times New Roman Bold.ttf",
        "/System/Library/Fonts/Supplemental/Times New Roman Italic.ttf",
        "/System/Library/Fonts/Supplemental/Times New Roman Bold Italic.ttf",
        "/System/Library/Fonts/NewYork.ttf",
        "/System/Library/Fonts/NewYorkItalic.ttf",
        // Linux sans / mono / serif
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-BoldOblique.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Italic.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Italic.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSerif-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSerif.ttf",
        // Windows sans / mono / serif
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "C:\\Windows\\Fonts\\segoeuib.ttf",
        "C:\\Windows\\Fonts\\segoeuii.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\arialbd.ttf",
        "C:\\Windows\\Fonts\\ariali.ttf",
        "C:\\Windows\\Fonts\\consola.ttf",
        "C:\\Windows\\Fonts\\consolab.ttf",
        "C:\\Windows\\Fonts\\cour.ttf",
        "C:\\Windows\\Fonts\\times.ttf",
        "C:\\Windows\\Fonts\\timesbd.ttf",
        "C:\\Windows\\Fonts\\timesi.ttf",
    ];
    const SCRIPT_FALLBACKS: &[&str] = &[
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/System/Library/Fonts/SFArabic.ttf",
        "/System/Library/Fonts/SFHebrew.ttf",
        "/System/Library/Fonts/GeezaPro.ttc",
        "/System/Library/Fonts/ArialHB.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "C:\\Windows\\Fonts\\seguisym.ttf",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msgothic.ttc",
        "C:\\Windows\\Fonts\\malgun.ttf",
    ];
    const LARGE_CJK_FALLBACKS: &[&str] = &[
        "/System/Library/Fonts/Supplemental/AppleGothic.ttf",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    ];

    let mut out = Vec::new();
    for &path in PRIMARY_CANDIDATES {
        let _ = load_font_file(std::path::Path::new(path), FontSourceTier::HostPrimary, false, &mut out);
    }
    let mut loaded_cjk = false;
    for &path in SCRIPT_FALLBACKS {
        if load_font_file(std::path::Path::new(path), FontSourceTier::HostScriptFallback, true, &mut out)
            && (path.contains("Arial Unicode") || path.contains("CJK") || path.contains("msyh") || path.contains("msgothic"))
        {
            loaded_cjk = true;
        }
    }
    if !loaded_cjk {
        for &path in LARGE_CJK_FALLBACKS {
            let _ = load_font_file(std::path::Path::new(path), FontSourceTier::HostScriptFallback, true, &mut out);
        }
    }
    out
}
