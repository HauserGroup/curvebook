//! Browser `<canvas>` text measurement cache and Z-layered RGBA/text bridge (`R10.1`, `R10.7`).
//!
//! Provides a zero-bundled-font [`WebTextMeasurer`] implementation ([`BrowserCanvasTextMeasurer`])
//! backed by an offscreen `CanvasRenderingContext2d::measure_text` probe on `wasm32` and a
//! deterministic [`curvebook_text`] fallback in native tests, plus [`WebTextBridge`] which splits
//! non-text RGBA layers from Z-ordered [`WebTextDrawOp`] runs across [`ZLayer`] bands.
#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

use curvebook_text::{
    RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, TextDirection, contains_rtl_script, measure_text_pt, parse_markup, reorder_visual_bidi,
    shape_bidi_runs,
};
use curvebook_ui_egui::RasterImage;
use curvebook_ui_egui::layout::{ClipId, LayoutRectPx as TreeLayoutRectPx, LayoutTree, PaintOp, PaintOpKind, ZLayer as TreeZLayer};
use curvebook_ui_egui::serde_json::{Map, Value, json};
use curvebook_ui_egui::text_bridge::{FontFamilyKind as TreeFontFamilyKind, UiTextStyle as TreeUiTextStyle};

use crate::host_adapters::{WebAdapterError, decode_base64_str, encode_base64_slice};

/// Default maximum cached text measurements retained per [`BrowserCanvasTextMeasurer`].
pub const DEFAULT_TEXT_CACHE_CAPACITY: usize = 4096;

/// Baseline CSS pixel density (`1 CSS px = 1/96 in`).
pub const CSS_REFERENCE_DPI: u16 = 96;

/// Font family classification matching `curvebook-text` shaping categories (`#248` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum FontFamilyKind {
    #[default]
    UiSans,
    UiMono,
    GridTabular,
    ScientificSerif,
}

impl FontFamilyKind {
    /// CSS font-family stack emitted to the browser 2D canvas context without external network fonts.
    #[must_use]
    pub const fn css_family_stack(self) -> &'static str {
        match self {
            Self::UiSans => "system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, sans-serif",
            Self::UiMono | Self::GridTabular => "ui-monospace, \"SFMono-Regular\", Menlo, Monaco, Consolas, monospace",
            Self::ScientificSerif => "Georgia, \"Times New Roman\", Cambria, serif",
        }
    }

    /// Stable JSON token for this font family kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UiSans => "ui_sans",
            Self::UiMono => "ui_mono",
            Self::GridTabular => "grid_tabular",
            Self::ScientificSerif => "scientific_serif",
        }
    }

    /// Parse a JSON font family token.
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ui_sans" | "uisans" | "sans" => Some(Self::UiSans),
            "ui_mono" | "uimono" | "mono" => Some(Self::UiMono),
            "grid_tabular" | "gridtabular" | "tabular" => Some(Self::GridTabular),
            "scientific_serif" | "scientificserif" | "serif" => Some(Self::ScientificSerif),
            _ => None,
        }
    }
}

/// Text styling descriptor passed to [`WebTextMeasurer`] and [`WebTextDrawOp`] (`#248` §2).
#[derive(Debug, Clone, PartialEq)]
pub struct UiTextStyle {
    pub family: FontFamilyKind,
    pub size_px: f32,
    pub weight: SpanWeight,
    pub slope: SpanSlope,
    pub tabular_nums: bool,
    pub rich: bool,
}

impl Default for UiTextStyle {
    fn default() -> Self {
        Self {
            family: FontFamilyKind::UiSans,
            size_px: 13.0,
            weight: SpanWeight::Regular,
            slope: SpanSlope::Upright,
            tabular_nums: false,
            rich: false,
        }
    }
}

impl UiTextStyle {
    /// Construct a proportional UI label/menu style at `size_px`.
    #[must_use]
    pub const fn ui_sans(size_px: f32) -> Self {
        Self { family: FontFamilyKind::UiSans, size_px, weight: SpanWeight::Regular, slope: SpanSlope::Upright, tabular_nums: false, rich: false }
    }

    /// Construct a bold UI heading/badge style at `size_px`.
    #[must_use]
    pub const fn ui_sans_bold(size_px: f32) -> Self {
        Self { family: FontFamilyKind::UiSans, size_px, weight: SpanWeight::Bold, slope: SpanSlope::Upright, tabular_nums: false, rich: false }
    }

    /// Construct a tabular-numeral data-grid/result style at `size_px`.
    #[must_use]
    pub const fn grid_tabular(size_px: f32) -> Self {
        Self { family: FontFamilyKind::GridTabular, size_px, weight: SpanWeight::Regular, slope: SpanSlope::Upright, tabular_nums: true, rich: false }
    }

    /// Construct a monospace code/shortcut badge style at `size_px`.
    #[must_use]
    pub const fn ui_mono(size_px: f32) -> Self {
        Self { family: FontFamilyKind::UiMono, size_px, weight: SpanWeight::Regular, slope: SpanSlope::Upright, tabular_nums: true, rich: false }
    }

    /// Convert a shared [`curvebook_ui_egui::text_bridge::UiTextStyle`] into its browser-canvas equivalent.
    #[must_use]
    pub fn from_tree_style(style: &TreeUiTextStyle) -> Self {
        let family = match style.family {
            TreeFontFamilyKind::UiSans => FontFamilyKind::UiSans,
            TreeFontFamilyKind::UiMono => FontFamilyKind::UiMono,
            TreeFontFamilyKind::GridTabular => FontFamilyKind::GridTabular,
            TreeFontFamilyKind::ScientificSerif => FontFamilyKind::ScientificSerif,
        };
        #[allow(clippy::cast_possible_truncation)]
        let size_px = if style.size_px.is_finite() && style.size_px > 0.0 { style.size_px as f32 } else { 13.0 };
        Self { family, size_px, weight: style.weight, slope: style.slope, tabular_nums: style.tabular_nums, rich: style.rich.is_some() }
    }

    /// Encode this style descriptor into JSON.
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        json!({
            "family": self.family.as_str(),
            "size_px": self.size_px,
            "weight": match self.weight { SpanWeight::Regular => "regular", SpanWeight::Bold => "bold" },
            "slope": match self.slope { SpanSlope::Upright => "upright", SpanSlope::Italic => "italic" },
            "tabular_nums": self.tabular_nums,
            "rich": self.rich,
        })
    }

    /// Decode [`UiTextStyle`] from JSON.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when malformed.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let obj = value.as_object().ok_or_else(|| WebAdapterError::Serialization(String::from("style must be an object")))?;
        let family = obj.get("family").and_then(Value::as_str).and_then(FontFamilyKind::from_name).unwrap_or(FontFamilyKind::UiSans);
        #[allow(clippy::cast_possible_truncation)]
        let size_px = obj.get("size_px").and_then(Value::as_f64).unwrap_or(13.0) as f32;
        let weight = match obj.get("weight").and_then(Value::as_str) {
            Some("bold" | "Bold") => SpanWeight::Bold,
            _ => SpanWeight::Regular,
        };
        let slope = match obj.get("slope").and_then(Value::as_str) {
            Some("italic" | "Italic") => SpanSlope::Italic,
            _ => SpanSlope::Upright,
        };
        let tabular_nums = obj.get("tabular_nums").and_then(Value::as_bool).unwrap_or(false);
        let rich = obj.get("rich").and_then(Value::as_bool).unwrap_or(false);
        Ok(Self { family, size_px, weight, slope, tabular_nums, rich })
    }
}

/// Measured text metrics in pixels (`#248` §2).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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

/// Shaped per-glyph run returned by [`WebTextMeasurer::shape_line`] (`#248` §2).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedGlyphRun {
    pub ch: char,
    pub x_px: f64,
    pub y_px: f64,
    pub advance_px: f64,
    pub glyph_id: Option<u16>,
    pub used_vector_fallback: bool,
    pub missing_glyph: bool,
}

/// Shaped single visual line with BiDi metadata (`#248` §2).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedTextLine {
    pub visual_text: String,
    pub direction: TextDirection,
    pub metrics: MeasuredTextMetrics,
    pub glyphs: Vec<ShapedGlyphRun>,
    pub missing_glyph_count: usize,
}

/// Trait matching the `R10.1` `TextMeasurer` boundary (`#248` §2).
pub trait WebTextMeasurer {
    /// Measure `text` rendered with `style` at physical `dpi`.
    fn measure(&self, style: &UiTextStyle, text: &str, dpi: u16) -> MeasuredTextMetrics;

    /// Shape `text` into visual order and glyph positions at physical `dpi`.
    fn shape_line(&self, style: &UiTextStyle, text: &str, dpi: u16) -> ShapedTextLine;
}

/// Physical pixel size for `style.size_px` scaled by `dpi / 96.0`.
#[must_use]
pub fn effective_font_px(size_px: f32, dpi: u16) -> f64 {
    let base = if size_px.is_finite() && size_px > 0.0 { f64::from(size_px) } else { 13.0 };
    let safe_dpi = f64::from(dpi.max(72));
    base * (safe_dpi / f64::from(CSS_REFERENCE_DPI))
}

/// Build the deterministic CSS `CanvasRenderingContext2d.font` specification for `style` at `dpi`.
#[must_use]
pub fn canvas_font_css(style: &UiTextStyle, dpi: u16) -> String {
    let px = effective_font_px(style.size_px, dpi);
    let slope = match style.slope {
        SpanSlope::Upright => "",
        SpanSlope::Italic => "italic ",
    };
    let weight = match style.weight {
        SpanWeight::Regular => "400 ",
        SpanWeight::Bold => "700 ",
    };
    format!("{slope}{weight}{px:.2}px {}", style.family.css_family_stack())
}

/// Pluggable text measurement backend (browser 2D canvas on `wasm32`, deterministic fallback on native).
pub trait CanvasMeasureBackend {
    /// Measure uncached `text` with resolved CSS font descriptor `font_css`.
    fn measure_uncached(&self, style: &UiTextStyle, font_css: &str, text: &str, dpi: u16) -> MeasuredTextMetrics;
}

/// Deterministic pure-Rust text metric fallback using `curvebook_text` Core-14 & BiDi rules.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicTextMeasureBackend;

impl CanvasMeasureBackend for DeterministicTextMeasureBackend {
    fn measure_uncached(&self, style: &UiTextStyle, _font_css: &str, text: &str, dpi: u16) -> MeasuredTextMetrics {
        measure_deterministic_px(style, text, dpi)
    }
}

/// Measure `text` deterministically in physical pixels without browser DOM access.
#[must_use]
pub fn measure_deterministic_px(style: &UiTextStyle, text: &str, dpi: u16) -> MeasuredTextMetrics {
    let eff_px = effective_font_px(style.size_px, dpi);
    if text.is_empty() {
        return MeasuredTextMetrics { width_px: 0.0, ascent_px: eff_px * 0.80, descent_px: eff_px * 0.20, line_height_px: (eff_px * 1.25).ceil() };
    }

    let rich = if style.rich {
        parse_markup(text)
    } else {
        RichText::new(vec![RichSpan {
            text: text.to_owned(),
            script: SpanScript::Baseline,
            weight: style.weight,
            slope: style.slope,
            color: None,
            font_family: None,
            scale: 1.0,
        }])
    };

    let size_pt = eff_px * (72.0 / f64::from(CSS_REFERENCE_DPI));
    let pt_metrics = measure_text_pt(&rich, size_pt);
    let pt_to_px = f64::from(CSS_REFERENCE_DPI) / 72.0;

    let width_px = if matches!(style.family, FontFamilyKind::UiMono | FontFamilyKind::GridTabular) || style.tabular_nums {
        measure_with_tabular_digits(&rich, style, eff_px, pt_metrics.width_pt * pt_to_px)
    } else {
        pt_metrics.width_pt * pt_to_px
    };

    let ascent_px = (pt_metrics.ascent_pt * pt_to_px).max(eff_px * 0.78);
    let descent_px = (pt_metrics.descent_pt * pt_to_px).max(eff_px * 0.22);
    let line_height_px = (pt_metrics.line_height_pt * pt_to_px).max(ascent_px + descent_px);

    MeasuredTextMetrics { width_px, ascent_px, descent_px, line_height_px }
}

fn measure_with_tabular_digits(rich: &RichText, style: &UiTextStyle, eff_px: f64, fallback_width_px: f64) -> f64 {
    if rich.spans.is_empty() {
        return fallback_width_px;
    }
    let mono = matches!(style.family, FontFamilyKind::UiMono);
    let size_pt = eff_px * (72.0 / f64::from(CSS_REFERENCE_DPI));
    let pt_to_px = f64::from(CSS_REFERENCE_DPI) / 72.0;
    let tabular_digit_w = eff_px * 0.56;
    let mono_char_w = eff_px * 0.60;

    let mut cur_line = 0.0_f64;
    let mut max_line = 0.0_f64;
    for span in &rich.spans {
        let span_scale = if span.scale.is_finite() && span.scale > 0.0 { span.scale } else { 1.0 };
        for ch in span.text.chars() {
            if ch == '\n' {
                max_line = max_line.max(cur_line);
                cur_line = 0.0;
                continue;
            }
            if ch == '\r' {
                continue;
            }
            let adv = if mono {
                mono_char_w * span_scale
            } else if ch.is_ascii_digit() {
                tabular_digit_w * span_scale
            } else {
                let mut buf = [0u8; 4];
                let ch_str = ch.encode_utf8(&mut buf);
                let single = RichText {
                    spans: vec![RichSpan {
                        text: ch_str.to_owned(),
                        script: span.script,
                        weight: span.weight,
                        slope: span.slope,
                        color: None,
                        font_family: None,
                        scale: span_scale,
                    }],
                };
                measure_text_pt(&single, size_pt).width_pt * pt_to_px
            };
            cur_line += adv;
        }
    }
    max_line.max(cur_line)
}

/// Browser offscreen `CanvasRenderingContext2d::measure_text` backend on `wasm32` (`#248` §2).
#[cfg(target_arch = "wasm32")]
#[derive(Debug, Default)]
pub struct WasmCanvasMeasureBackend {
    ctx2d: RefCell<Option<wasm_bindgen::JsValue>>,
}

#[cfg(target_arch = "wasm32")]
impl WasmCanvasMeasureBackend {
    fn ensure_ctx2d(&self) -> Option<wasm_bindgen::JsValue> {
        if let Some(existing) = self.ctx2d.borrow().as_ref() {
            return Some(existing.clone());
        }
        use wasm_bindgen::JsCast;

        let win = web_sys::window()?;
        let doc = js_sys::Reflect::get(&win, &wasm_bindgen::JsValue::from_str("document")).ok()?;
        let create_el = js_sys::Reflect::get(&doc, &wasm_bindgen::JsValue::from_str("createElement")).ok()?.dyn_into::<js_sys::Function>().ok()?;
        let canvas = create_el.call1(&doc, &wasm_bindgen::JsValue::from_str("canvas")).ok()?;
        let get_ctx = js_sys::Reflect::get(&canvas, &wasm_bindgen::JsValue::from_str("getContext")).ok()?.dyn_into::<js_sys::Function>().ok()?;
        let ctx = get_ctx.call1(&canvas, &wasm_bindgen::JsValue::from_str("2d")).ok()?;
        if ctx.is_null() || ctx.is_undefined() {
            return None;
        }
        if let Ok(mut guard) = self.ctx2d.try_borrow_mut() {
            *guard = Some(ctx.clone());
        }
        Some(ctx)
    }

    fn try_measure_dom(&self, style: &UiTextStyle, font_css: &str, text: &str, dpi: u16) -> Option<MeasuredTextMetrics> {
        use wasm_bindgen::JsCast;

        let ctx = self.ensure_ctx2d()?;
        let _ = js_sys::Reflect::set(&ctx, &wasm_bindgen::JsValue::from_str("font"), &wasm_bindgen::JsValue::from_str(font_css));
        let measure_fn = js_sys::Reflect::get(&ctx, &wasm_bindgen::JsValue::from_str("measureText")).ok()?.dyn_into::<js_sys::Function>().ok()?;
        let raw_metrics = measure_fn.call1(&ctx, &wasm_bindgen::JsValue::from_str(text)).ok()?;
        let width_px = js_sys::Reflect::get(&raw_metrics, &wasm_bindgen::JsValue::from_str("width"))
            .ok()
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite() && *v >= 0.0)?;

        let eff_px = effective_font_px(style.size_px, dpi);
        let ascent_px = read_metric_f64(&raw_metrics, "actualBoundingBoxAscent")
            .or_else(|| read_metric_f64(&raw_metrics, "fontBoundingBoxAscent"))
            .unwrap_or(eff_px * 0.80);
        let descent_px = read_metric_f64(&raw_metrics, "actualBoundingBoxDescent")
            .or_else(|| read_metric_f64(&raw_metrics, "fontBoundingBoxDescent"))
            .unwrap_or(eff_px * 0.20);
        let line_height_px = (ascent_px + descent_px).max(eff_px * 1.25);

        Some(MeasuredTextMetrics { width_px, ascent_px, descent_px, line_height_px })
    }
}

#[cfg(target_arch = "wasm32")]
impl Clone for WasmCanvasMeasureBackend {
    fn clone(&self) -> Self {
        Self::default()
    }
}

#[cfg(target_arch = "wasm32")]
fn read_metric_f64(metrics: &wasm_bindgen::JsValue, key: &str) -> Option<f64> {
    js_sys::Reflect::get(metrics, &wasm_bindgen::JsValue::from_str(key)).ok().and_then(|v| v.as_f64()).filter(|v| v.is_finite() && *v >= 0.0)
}

#[cfg(target_arch = "wasm32")]
impl CanvasMeasureBackend for WasmCanvasMeasureBackend {
    fn measure_uncached(&self, style: &UiTextStyle, font_css: &str, text: &str, dpi: u16) -> MeasuredTextMetrics {
        self.try_measure_dom(style, font_css, text, dpi).unwrap_or_else(|| measure_deterministic_px(style, text, dpi))
    }
}

/// Default platform measurement backend (`WasmCanvasMeasureBackend` on `wasm32`, deterministic fallback on native).
#[cfg(target_arch = "wasm32")]
pub type PlatformCanvasMeasureBackend = WasmCanvasMeasureBackend;
/// Default platform measurement backend (`WasmCanvasMeasureBackend` on `wasm32`, deterministic fallback on native).
#[cfg(not(target_arch = "wasm32"))]
pub type PlatformCanvasMeasureBackend = DeterministicTextMeasureBackend;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct TextCacheKey {
    family: FontFamilyKind,
    size_q4: u32,
    weight_bold: bool,
    slope_italic: bool,
    tabular_nums: bool,
    rich: bool,
    dpi: u16,
    text: String,
}

impl TextCacheKey {
    fn new(style: &UiTextStyle, text: &str, dpi: u16) -> Self {
        let clamped_px = if style.size_px.is_finite() && style.size_px > 0.0 { style.size_px.clamp(1.0, 1024.0) } else { 13.0 };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let size_q4 = (clamped_px * 16.0).round() as u32;
        Self {
            family: style.family,
            size_q4,
            weight_bold: matches!(style.weight, SpanWeight::Bold),
            slope_italic: matches!(style.slope, SpanSlope::Italic),
            tabular_nums: style.tabular_nums,
            rich: style.rich,
            dpi: dpi.max(72),
            text: text.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct BrowserTextCache {
    capacity: usize,
    entries: BTreeMap<TextCacheKey, MeasuredTextMetrics>,
    order: VecDeque<TextCacheKey>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl BrowserTextCache {
    fn with_capacity(capacity: usize) -> Self {
        Self { capacity: capacity.max(1), ..Self::default() }
    }

    fn get(&mut self, key: &TextCacheKey) -> Option<MeasuredTextMetrics> {
        if let Some(&metrics) = self.entries.get(key) {
            self.hits = self.hits.saturating_add(1);
            Some(metrics)
        } else {
            self.misses = self.misses.saturating_add(1);
            None
        }
    }

    fn insert(&mut self, key: TextCacheKey, value: MeasuredTextMetrics) {
        if let Some(slot) = self.entries.get_mut(&key) {
            *slot = value;
            return;
        }
        while self.entries.len() >= self.capacity {
            if let Some(evicted_key) = self.order.pop_front() {
                if self.entries.remove(&evicted_key).is_some() {
                    self.evictions = self.evictions.saturating_add(1);
                }
            } else {
                break;
            }
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, value);
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

/// Cache telemetry counters exposed for diagnostics and native tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
    pub capacity: usize,
}

/// Cached browser canvas `TextMeasurer` bridge (`R10.1` web binding rule 3).
///
/// Uses single-owner [`RefCell`] caching (zero `std::sync::Mutex`, WASM-safe) with bounded eviction.
#[derive(Debug, Clone)]
pub struct BrowserCanvasTextMeasurer<B: CanvasMeasureBackend = PlatformCanvasMeasureBackend> {
    backend: B,
    cache: RefCell<BrowserTextCache>,
}

impl Default for BrowserCanvasTextMeasurer<PlatformCanvasMeasureBackend> {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserCanvasTextMeasurer<PlatformCanvasMeasureBackend> {
    /// Construct a measurer backed by the active platform backend with default `4096`-entry cache.
    #[must_use]
    pub fn new() -> Self {
        Self::with_backend(PlatformCanvasMeasureBackend::default(), DEFAULT_TEXT_CACHE_CAPACITY)
    }

    /// Construct a measurer backed by the active platform backend with explicit cache capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_backend(PlatformCanvasMeasureBackend::default(), capacity)
    }
}

impl<B: CanvasMeasureBackend> BrowserCanvasTextMeasurer<B> {
    /// Construct a measurer wrapping an explicit [`CanvasMeasureBackend`] and bounded cache size.
    #[must_use]
    pub fn with_backend(backend: B, capacity: usize) -> Self {
        Self { backend, cache: RefCell::new(BrowserTextCache::with_capacity(capacity)) }
    }

    /// Borrow the underlying backend (for mock inspection in tests).
    #[must_use]
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// Snapshot current cache hit/miss/eviction statistics.
    #[must_use]
    pub fn cache_stats(&self) -> TextCacheStats {
        let Ok(guard) = self.cache.try_borrow() else {
            return TextCacheStats::default();
        };
        TextCacheStats { hits: guard.hits, misses: guard.misses, evictions: guard.evictions, entries: guard.entries.len(), capacity: guard.capacity }
    }

    /// Clear all cached text measurements while keeping cumulative hit/miss counters.
    pub fn clear_cache(&self) {
        if let Ok(mut guard) = self.cache.try_borrow_mut() {
            guard.clear();
        }
    }
}

impl<B: CanvasMeasureBackend> WebTextMeasurer for BrowserCanvasTextMeasurer<B> {
    fn measure(&self, style: &UiTextStyle, text: &str, dpi: u16) -> MeasuredTextMetrics {
        let key = TextCacheKey::new(style, text, dpi);
        if let Ok(mut guard) = self.cache.try_borrow_mut()
            && let Some(hit) = guard.get(&key)
        {
            return hit;
        }
        let font_css = canvas_font_css(style, dpi);
        let measured = self.backend.measure_uncached(style, &font_css, text, dpi);
        if let Ok(mut guard) = self.cache.try_borrow_mut() {
            guard.insert(key, measured);
        }
        measured
    }

    fn shape_line(&self, style: &UiTextStyle, text: &str, dpi: u16) -> ShapedTextLine {
        let direction = if contains_rtl_script(text) { TextDirection::Rtl } else { TextDirection::Ltr };
        let visual_text = if direction == TextDirection::Rtl { reorder_visual_bidi(text, direction) } else { text.to_owned() };
        let metrics = self.measure(style, &visual_text, dpi);
        let eff_px = effective_font_px(style.size_px, dpi);
        let size_pt = eff_px * (72.0 / f64::from(CSS_REFERENCE_DPI));
        let pt_to_px = f64::from(CSS_REFERENCE_DPI) / 72.0;

        let mut glyphs = Vec::with_capacity(visual_text.chars().count());
        let mut pen_x = 0.0_f64;
        for _run in shape_bidi_runs(&visual_text, direction) {
            // `visual_text` is already visually ordered; iterate its chars with measured advances.
        }
        for ch in visual_text.chars() {
            let adv_px = if matches!(style.family, FontFamilyKind::UiMono) {
                eff_px * 0.60
            } else if style.tabular_nums && ch.is_ascii_digit() {
                eff_px * 0.56
            } else {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                let measured_ch = self.measure(style, s, dpi).width_px;
                if measured_ch > 0.0 { measured_ch } else { measure_text_pt(&RichText::plain(s), size_pt).width_pt * pt_to_px }
            };
            glyphs.push(ShapedGlyphRun {
                ch,
                x_px: pen_x,
                y_px: metrics.ascent_px,
                advance_px: adv_px,
                glyph_id: None,
                used_vector_fallback: false,
                missing_glyph: false,
            });
            pen_x += adv_px;
        }
        if pen_x > 0.0 && metrics.width_px > 0.0 {
            let scale = metrics.width_px / pen_x;
            if (scale - 1.0).abs() > 1e-4 {
                for g in &mut glyphs {
                    g.x_px *= scale;
                    g.advance_px *= scale;
                }
            }
        }

        ShapedTextLine { visual_text, direction, metrics, glyphs, missing_glyph_count: 0 }
    }
}

/// Z-order layers matching `crates/ui-egui/src/layout/mod.rs` (`#248` §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum ZLayer {
    #[default]
    Workspace = 0,
    FloatingOverlay = 10,
    SplitHandle = 20,
    DropdownMenu = 30,
    ModalBackdrop = 40,
    ModalDialog = 50,
    ModalDropdown = 60,
    ContextMenu = 70,
    Tooltip = 80,
}

impl ZLayer {
    /// Numeric layer rank in paint order.
    #[must_use]
    pub const fn rank(self) -> u8 {
        self as u8
    }

    /// Coarse compositing band grouping RGBA surfaces and text runs (`#248` §2).
    #[must_use]
    pub const fn band(self) -> ZLayerBand {
        match self {
            Self::Workspace | Self::FloatingOverlay | Self::SplitHandle => ZLayerBand::Workspace,
            Self::DropdownMenu => ZLayerBand::PopupDropdown,
            Self::ModalBackdrop | Self::ModalDialog | Self::ModalDropdown => ZLayerBand::ModalOverlay,
            Self::ContextMenu | Self::Tooltip => ZLayerBand::TopOverlay,
        }
    }

    /// Wire token for JSON transport.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::FloatingOverlay => "floating_overlay",
            Self::SplitHandle => "split_handle",
            Self::DropdownMenu => "dropdown_menu",
            Self::ModalBackdrop => "modal_backdrop",
            Self::ModalDialog => "modal_dialog",
            Self::ModalDropdown => "modal_dropdown",
            Self::ContextMenu => "context_menu",
            Self::Tooltip => "tooltip",
        }
    }

    /// Parse a [`ZLayer`] from wire token or enum name.
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "workspace" => Some(Self::Workspace),
            "floating_overlay" | "floatingoverlay" => Some(Self::FloatingOverlay),
            "split_handle" | "splithandle" => Some(Self::SplitHandle),
            "dropdown_menu" | "dropdownmenu" => Some(Self::DropdownMenu),
            "modal_backdrop" | "modalbackdrop" => Some(Self::ModalBackdrop),
            "modal_dialog" | "modaldialog" => Some(Self::ModalDialog),
            "modal_dropdown" | "modaldropdown" => Some(Self::ModalDropdown),
            "context_menu" | "contextmenu" => Some(Self::ContextMenu),
            "tooltip" => Some(Self::Tooltip),
            _ => None,
        }
    }
}

impl From<TreeZLayer> for ZLayer {
    fn from(value: TreeZLayer) -> Self {
        match value {
            TreeZLayer::Workspace => Self::Workspace,
            TreeZLayer::FloatingOverlay => Self::FloatingOverlay,
            TreeZLayer::SplitHandle => Self::SplitHandle,
            TreeZLayer::DropdownMenu => Self::DropdownMenu,
            TreeZLayer::ModalBackdrop => Self::ModalBackdrop,
            TreeZLayer::ModalDialog => Self::ModalDialog,
            TreeZLayer::ModalDropdown => Self::ModalDropdown,
            TreeZLayer::ContextMenu => Self::ContextMenu,
            TreeZLayer::Tooltip => Self::Tooltip,
        }
    }
}

/// Coarse Z-ordered compositing band interleaved between RGBA layers and `ctx.fillText` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum ZLayerBand {
    #[default]
    Workspace = 0,
    PopupDropdown = 1,
    ModalOverlay = 2,
    TopOverlay = 3,
}

impl ZLayerBand {
    /// All compositing bands in strict paint order (`Workspace -> PopupDropdown -> ModalOverlay -> TopOverlay`).
    pub const ALL: [Self; 4] = [Self::Workspace, Self::PopupDropdown, Self::ModalOverlay, Self::TopOverlay];

    /// Stable wire identifier used by `index.html`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::PopupDropdown => "popup_dropdown",
            Self::ModalOverlay => "modal_overlay",
            Self::TopOverlay => "top_overlay",
        }
    }

    /// Parse a band string or integer rank from JSON.
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "workspace" => Some(Self::Workspace),
            "popup_dropdown" | "dropdown" => Some(Self::PopupDropdown),
            "modal_overlay" | "modal" => Some(Self::ModalOverlay),
            "top_overlay" | "context_tooltip" | "top" => Some(Self::TopOverlay),
            _ => None,
        }
    }
}

/// Physical pixel rectangle matching `crates/ui-egui/src/layout/mod.rs::LayoutRectPx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LayoutRectPx {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl LayoutRectPx {
    /// Construct a rectangle at `(x, y)` with `(width, height)` in physical pixels.
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    /// Encode this rectangle to JSON.
    #[must_use]
    pub fn to_json_value(self) -> Value {
        json!({ "x": self.x, "y": self.y, "width": self.width, "height": self.height })
    }

    /// Decode a [`LayoutRectPx`] from JSON.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when missing integer fields.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let obj = value.as_object().ok_or_else(|| WebAdapterError::Serialization(String::from("rect_px must be an object")))?;
        #[allow(clippy::cast_possible_truncation)]
        let x = obj.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
        #[allow(clippy::cast_possible_truncation)]
        let y = obj.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let width = obj.get("width").and_then(Value::as_u64).unwrap_or(0) as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let height = obj.get("height").and_then(Value::as_u64).unwrap_or(0) as u32;
        Ok(Self { x, y, width, height })
    }

    /// Right edge (`x + width`) saturating cleanly.
    #[must_use]
    pub const fn right(self) -> i32 {
        #[allow(clippy::cast_possible_wrap)]
        let w = if self.width > i32::MAX as u32 { i32::MAX } else { self.width as i32 };
        self.x.saturating_add(w)
    }

    /// Bottom edge (`y + height`) saturating cleanly.
    #[must_use]
    pub const fn bottom(self) -> i32 {
        #[allow(clippy::cast_possible_wrap)]
        let h = if self.height > i32::MAX as u32 { i32::MAX } else { self.height as i32 };
        self.y.saturating_add(h)
    }

    /// Whether integer pixel coordinate `(px, py)` falls inside this rectangle.
    #[must_use]
    pub const fn contains(self, px: i32, py: i32) -> bool {
        self.width > 0 && self.height > 0 && px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// Intersect two rectangles, returning `None` when the intersection is empty.
    #[must_use]
    pub const fn intersect(self, other: Self) -> Option<Self> {
        let x0 = if self.x > other.x { self.x } else { other.x };
        let y0 = if self.y > other.y { self.y } else { other.y };
        let r = if self.right() < other.right() { self.right() } else { other.right() };
        let b = if self.bottom() < other.bottom() { self.bottom() } else { other.bottom() };
        if r <= x0 || b <= y0 {
            return None;
        }
        #[allow(clippy::cast_sign_loss)]
        Some(Self { x: x0, y: y0, width: (r - x0) as u32, height: (b - y0) as u32 })
    }

    /// Union bounding rectangle covering both non-empty rectangles.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        if self.width == 0 || self.height == 0 {
            return other;
        }
        if other.width == 0 || other.height == 0 {
            return self;
        }
        let x0 = if self.x < other.x { self.x } else { other.x };
        let y0 = if self.y < other.y { self.y } else { other.y };
        let r = if self.right() > other.right() { self.right() } else { other.right() };
        let b = if self.bottom() > other.bottom() { self.bottom() } else { other.bottom() };
        let w = if r > x0 { r - x0 } else { 0 };
        let h = if b > y0 { b - y0 } else { 0 };
        #[allow(clippy::cast_sign_loss)]
        Self { x: x0, y: y0, width: w as u32, height: h as u32 }
    }
}

impl From<TreeLayoutRectPx> for LayoutRectPx {
    fn from(value: TreeLayoutRectPx) -> Self {
        Self { x: value.x, y: value.y, width: value.width, height: value.height }
    }
}

/// Horizontal text alignment inside [`WebTextDrawOp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebTextAlign {
    #[default]
    Start,
    Center,
    End,
    Left,
    Right,
}

impl WebTextAlign {
    /// CSS `CanvasRenderingContext2d.textAlign` token.
    #[must_use]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Center => "center",
            Self::End => "end",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// Parse from CSS / JSON token.
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "start" => Some(Self::Start),
            "center" => Some(Self::Center),
            "end" => Some(Self::End),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// Serialized BiDi direction token for [`WebTextDrawOp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebTextDirection {
    #[default]
    Ltr,
    Rtl,
}

impl From<TextDirection> for WebTextDirection {
    fn from(value: TextDirection) -> Self {
        match value {
            TextDirection::Ltr => Self::Ltr,
            TextDirection::Rtl => Self::Rtl,
        }
    }
}

impl WebTextDirection {
    /// CSS `CanvasRenderingContext2d.direction` token.
    #[must_use]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::Ltr => "ltr",
            Self::Rtl => "rtl",
        }
    }

    /// Parse from CSS / JSON token.
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ltr" => Some(Self::Ltr),
            "rtl" => Some(Self::Rtl),
            _ => None,
        }
    }
}

/// Single Z-ordered browser `ctx.fillText(...)` draw instruction (`#248` §2).
#[derive(Debug, Clone, PartialEq)]
pub struct WebTextDrawOp {
    pub z_layer: ZLayer,
    pub z_band: ZLayerBand,
    pub paint_order: u32,
    pub clip_px: Option<LayoutRectPx>,
    pub x_px: f64,
    pub y_px: f64,
    pub baseline_y_px: f64,
    pub max_width_px: Option<f64>,
    pub text: String,
    pub style: UiTextStyle,
    pub font_css: String,
    pub color_rgba: [u8; 4],
    pub align: WebTextAlign,
    pub direction: WebTextDirection,
    pub ellipsized: bool,
    pub node_id: Option<String>,
}

impl WebTextDrawOp {
    /// Encode this draw instruction as JSON for browser `ctx.fillText(...)` execution.
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        json!({
            "z_layer": self.z_layer.as_str(),
            "z_band": self.z_band.as_str(),
            "paint_order": self.paint_order,
            "clip_px": self.clip_px.map(LayoutRectPx::to_json_value),
            "x_px": self.x_px,
            "y_px": self.y_px,
            "baseline_y_px": self.baseline_y_px,
            "max_width_px": self.max_width_px,
            "text": self.text,
            "style": self.style.to_json_value(),
            "font_css": self.font_css,
            "color_rgba": self.color_rgba,
            "align": self.align.as_css(),
            "direction": self.direction.as_css(),
            "ellipsized": self.ellipsized,
            "node_id": self.node_id,
        })
    }

    /// Decode a [`WebTextDrawOp`] from JSON.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when fields are invalid.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let obj = value.as_object().ok_or_else(|| WebAdapterError::Serialization(String::from("text_op must be an object")))?;
        let z_layer = obj.get("z_layer").and_then(Value::as_str).and_then(ZLayer::from_name).unwrap_or(ZLayer::Workspace);
        let z_band = obj.get("z_band").and_then(Value::as_str).and_then(ZLayerBand::from_name).unwrap_or_else(|| z_layer.band());
        #[allow(clippy::cast_possible_truncation)]
        let paint_order = obj.get("paint_order").and_then(Value::as_u64).unwrap_or(0) as u32;
        let clip_px = obj.get("clip_px").filter(|v| !v.is_null()).map(LayoutRectPx::from_json_value).transpose()?;
        let x_px = obj.get("x_px").and_then(Value::as_f64).unwrap_or(0.0);
        let y_px = obj.get("y_px").and_then(Value::as_f64).unwrap_or(0.0);
        let baseline_y_px = obj.get("baseline_y_px").and_then(Value::as_f64).unwrap_or(0.0);
        let max_width_px = obj.get("max_width_px").and_then(Value::as_f64);
        let text = obj.get("text").and_then(Value::as_str).unwrap_or_default().to_owned();
        let style = obj.get("style").map(UiTextStyle::from_json_value).transpose()?.unwrap_or_default();
        let font_css = obj.get("font_css").and_then(Value::as_str).unwrap_or_default().to_owned();
        let color_rgba = parse_rgba_array(obj.get("color_rgba"));
        let align = obj.get("align").and_then(Value::as_str).and_then(WebTextAlign::from_name).unwrap_or(WebTextAlign::Start);
        let direction = obj.get("direction").and_then(Value::as_str).and_then(WebTextDirection::from_name).unwrap_or(WebTextDirection::Ltr);
        let ellipsized = obj.get("ellipsized").and_then(Value::as_bool).unwrap_or(false);
        let node_id = obj.get("node_id").and_then(Value::as_str).map(str::to_owned);
        Ok(Self {
            z_layer,
            z_band,
            paint_order,
            clip_px,
            x_px,
            y_px,
            baseline_y_px,
            max_width_px,
            text,
            style,
            font_css,
            color_rgba,
            align,
            direction,
            ellipsized,
            node_id,
        })
    }
}

fn parse_rgba_array(value: Option<&Value>) -> [u8; 4] {
    let Some(arr) = value.and_then(Value::as_array) else {
        return [0, 0, 0, 255];
    };
    let ch = |idx: usize, fallback: u8| -> u8 { arr.get(idx).and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok()).unwrap_or(fallback) };
    [ch(0, 0), ch(1, 0), ch(2, 0), ch(3, 255)]
}

/// Non-text RGBA surface slice belonging to a single [`ZLayerBand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebRgbaLayer {
    pub z_band: ZLayerBand,
    pub rect_px: LayoutRectPx,
    pub uses_base_frame: bool,
    pub opaque: bool,
    pub rgba_bytes: Vec<u8>,
}

impl WebRgbaLayer {
    /// Encode this layer to JSON for `RenderRgbaFrame`.
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        json!({
            "z_band": self.z_band.as_str(),
            "rect_px": self.rect_px.to_json_value(),
            "uses_base_frame": self.uses_base_frame,
            "opaque": self.opaque,
            "rgba_base64": if self.uses_base_frame { String::new() } else { encode_base64_slice(&self.rgba_bytes) },
        })
    }

    /// Decode a [`WebRgbaLayer`] from JSON.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when fields are invalid.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let obj = value.as_object().ok_or_else(|| WebAdapterError::Serialization(String::from("rgba_layer must be an object")))?;
        let band_str = obj.get("z_band").and_then(Value::as_str).unwrap_or("workspace");
        let z_band = ZLayerBand::from_name(band_str).ok_or_else(|| WebAdapterError::Serialization(format!("unknown z_band `{band_str}`")))?;
        let rect_val = obj.get("rect_px").ok_or_else(|| WebAdapterError::Serialization(String::from("missing `rect_px`")))?;
        let rect_px = LayoutRectPx::from_json_value(rect_val)?;
        let uses_base_frame = obj.get("uses_base_frame").and_then(Value::as_bool).unwrap_or(false);
        let opaque = obj.get("opaque").and_then(Value::as_bool).unwrap_or(false);
        let rgba_bytes = if uses_base_frame {
            Vec::new()
        } else if let Some(b64) = obj.get("rgba_base64").and_then(Value::as_str) {
            decode_base64_str(b64)?
        } else {
            Vec::new()
        };
        Ok(Self { z_band, rect_px, uses_base_frame, opaque, rgba_bytes })
    }
}

/// Software non-text rectangle primitive painted into its [`ZLayerBand`] RGBA buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebRectPrimitive {
    pub z_layer: ZLayer,
    pub rect_px: LayoutRectPx,
    pub clip_px: Option<LayoutRectPx>,
    pub fill: Option<[u8; 4]>,
    pub stroke: Option<[u8; 4]>,
    pub stroke_width_px: u32,
}

/// Band-split RGBA layers and Z-ordered text operations emitted by [`WebTextBridge`].
#[derive(Debug, Clone, PartialEq)]
pub struct WebLayeredFrame {
    pub width_px: u32,
    pub height_px: u32,
    pub base_rgba_bytes: Vec<u8>,
    pub rgba_layers: Vec<WebRgbaLayer>,
    pub text_ops: Vec<WebTextDrawOp>,
}

/// Bridges software RGBA frame/overlay rectangles and shaped UI text into Z-band layers (`#248` §2).
#[derive(Debug, Clone, Copy, Default)]
pub struct WebTextBridge;

impl WebTextBridge {
    /// Split non-text RGBA background/overlay primitives and `text_ops` into sorted [`ZLayerBand`] layers.
    #[must_use]
    pub fn compose_layered_frame(base_frame: RasterImage, rect_primitives: &[WebRectPrimitive], mut text_ops: Vec<WebTextDrawOp>) -> WebLayeredFrame {
        let width_px = base_frame.width_px;
        let height_px = base_frame.height_px;
        let mut base_img = base_frame;
        let viewport_rect = LayoutRectPx::new(0, 0, width_px, height_px);

        let mut band_rects: [Vec<WebRectPrimitive>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for prim in rect_primitives {
            let idx = usize::from(prim.z_layer.band() as u8);
            if let Some(bucket) = band_rects.get_mut(idx) {
                bucket.push(*prim);
            }
        }

        // Paint Workspace-band chrome backgrounds directly onto the base RGBA buffer so no
        // redundant full-viewport buffer is duplicated over the bridge.
        for prim in &band_rects[ZLayerBand::Workspace as usize] {
            paint_rect_primitive(&mut base_img, viewport_rect, prim);
        }

        let mut rgba_layers = Vec::with_capacity(4);
        rgba_layers.push(WebRgbaLayer {
            z_band: ZLayerBand::Workspace,
            rect_px: viewport_rect,
            uses_base_frame: true,
            opaque: true,
            rgba_bytes: Vec::new(),
        });

        for band in [ZLayerBand::PopupDropdown, ZLayerBand::ModalOverlay, ZLayerBand::TopOverlay] {
            let prims = &band_rects[band as usize];
            if let Some(layer) = rasterize_band_layer(band, viewport_rect, prims) {
                rgba_layers.push(layer);
            }
        }

        text_ops.sort_by(|a, b| a.z_layer.cmp(&b.z_layer).then_with(|| a.paint_order.cmp(&b.paint_order)));
        for op in &mut text_ops {
            op.z_band = op.z_layer.band();
            if op.font_css.is_empty() {
                op.font_css = canvas_font_css(&op.style, CSS_REFERENCE_DPI);
            }
        }

        WebLayeredFrame { width_px, height_px, base_rgba_bytes: base_img.rgba, rgba_layers, text_ops }
    }

    /// Render a shared [`LayoutTree`] into Z-banded RGBA layers and browser-measured [`WebTextDrawOp`] runs.
    #[must_use]
    pub fn compose_layout_tree_frame(tree: &LayoutTree, measurer: &dyn WebTextMeasurer, dpi: u16, surface_bg: [u8; 4]) -> WebLayeredFrame {
        let width_px = tree.metrics.physical_width_px().max(1);
        let height_px = tree.metrics.physical_height_px().max(1);
        let viewport_rect = LayoutRectPx::new(0, 0, width_px, height_px);
        let mut base_img = solid_raster_image(width_px, height_px, dpi.max(72), [surface_bg[0], surface_bg[1], surface_bg[2], 255]);

        let mut sorted_ops: Vec<&PaintOp> = tree.paint_ops.iter().collect();
        sorted_ops.sort_by_key(|op| (op.z_layer, op.paint_order));

        let mut band_ops: [Vec<(&PaintOp, LayoutRectPx)>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        let mut text_ops = Vec::new();
        for op in sorted_ops {
            let Some(clip_rect) = resolve_tree_clip(op.clip, &tree.clips, viewport_rect) else {
                continue;
            };
            if matches!(op.kind, PaintOpKind::TextRun { .. }) {
                if let Some(text_op) = tree_op_to_text_draw_op(op, clip_rect, measurer) {
                    text_ops.push(text_op);
                }
                continue;
            }
            let z_band = ZLayer::from(op.z_layer).band();
            if let Some(bucket) = band_ops.get_mut(usize::from(z_band as u8)) {
                bucket.push((op, clip_rect));
            }
        }

        for (op, clip_rect) in &band_ops[ZLayerBand::Workspace as usize] {
            paint_tree_op_into_image(&mut base_img, viewport_rect, op, *clip_rect);
        }

        let mut rgba_layers = Vec::with_capacity(4);
        rgba_layers.push(WebRgbaLayer {
            z_band: ZLayerBand::Workspace,
            rect_px: viewport_rect,
            uses_base_frame: true,
            opaque: true,
            rgba_bytes: Vec::new(),
        });
        for band in [ZLayerBand::PopupDropdown, ZLayerBand::ModalOverlay, ZLayerBand::TopOverlay] {
            if let Some(layer) = rasterize_tree_band_layer(band, &band_ops[band as usize], dpi.max(72)) {
                rgba_layers.push(layer);
            }
        }

        WebLayeredFrame { width_px, height_px, base_rgba_bytes: base_img.rgba, rgba_layers, text_ops }
    }

    /// Validate Z-order invariants, RGBA layer byte lengths, and text clip bounds on `frame`.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if any band, buffer size, or text draw op is malformed.
    pub fn validate_layered_frame(frame: &WebLayeredFrame) -> Result<(), WebAdapterError> {
        let expected_base = (frame.width_px as usize).saturating_mul(frame.height_px as usize).saturating_mul(4);
        if frame.base_rgba_bytes.len() != expected_base {
            return Err(WebAdapterError::Validation(format!("base_rgba_bytes len {} != expected {expected_base}", frame.base_rgba_bytes.len())));
        }

        let mut last_band: Option<ZLayerBand> = None;
        for layer in &frame.rgba_layers {
            if let Some(prev) = last_band
                && layer.z_band < prev
            {
                return Err(WebAdapterError::Validation(String::from("rgba_layers are not sorted by ZLayerBand")));
            }
            last_band = Some(layer.z_band);
            if !layer.uses_base_frame {
                let expected = (layer.rect_px.width as usize).saturating_mul(layer.rect_px.height as usize).saturating_mul(4);
                if layer.rgba_bytes.len() != expected {
                    return Err(WebAdapterError::Validation(format!(
                        "band {:?} rgba_bytes len {} != expected {expected}",
                        layer.z_band,
                        layer.rgba_bytes.len()
                    )));
                }
            }
        }

        let mut last_key: Option<(ZLayer, u32)> = None;
        for op in &frame.text_ops {
            if op.z_band != op.z_layer.band() {
                return Err(WebAdapterError::Validation(String::from("text_op z_band mismatches z_layer")));
            }
            let key = (op.z_layer, op.paint_order);
            if let Some(prev) = last_key
                && key < prev
            {
                return Err(WebAdapterError::Validation(String::from("text_ops are not sorted by (z_layer, paint_order)")));
            }
            last_key = Some(key);
            if op.font_css.trim().is_empty() {
                return Err(WebAdapterError::Validation(String::from("text_op font_css must not be empty")));
            }
        }
        Ok(())
    }
}

fn solid_raster_image(width_px: u32, height_px: u32, dpi: u16, rgba: [u8; 4]) -> RasterImage {
    let pixel_count = (width_px as usize).saturating_mul(height_px as usize);
    let mut bytes = Vec::with_capacity(pixel_count.saturating_mul(4));
    for _ in 0..pixel_count {
        bytes.extend_from_slice(&rgba);
    }
    RasterImage { width_px, height_px, dpi, rgba: bytes }
}

fn resolve_tree_clip(clip: Option<ClipId>, clips: &[TreeLayoutRectPx], viewport: LayoutRectPx) -> Option<LayoutRectPx> {
    let raw = clip.and_then(|id| clips.get(id.0).copied()).map(LayoutRectPx::from).unwrap_or(viewport);
    raw.intersect(viewport)
}

fn tree_op_to_text_draw_op(op: &PaintOp, clip_rect: LayoutRectPx, measurer: &dyn WebTextMeasurer) -> Option<WebTextDrawOp> {
    let PaintOpKind::TextRun { text, style, origin_px, color, shaped, .. } = &op.kind else {
        return None;
    };
    if color.a == 0 || text.is_empty() {
        return None;
    }
    let web_style = UiTextStyle::from_tree_style(style);
    let metrics = measurer.measure(&web_style, text, CSS_REFERENCE_DPI);
    let font_css = canvas_font_css(&web_style, CSS_REFERENCE_DPI);
    let z_layer = ZLayer::from(op.z_layer);
    let avail = f64::from(clip_rect.right()) - origin_px[0];
    let direction = if shaped.direction == TextDirection::Rtl || contains_rtl_script(text) { WebTextDirection::Rtl } else { WebTextDirection::Ltr };
    Some(WebTextDrawOp {
        z_layer,
        z_band: z_layer.band(),
        paint_order: op.paint_order,
        clip_px: Some(clip_rect),
        x_px: origin_px[0],
        y_px: origin_px[1] - metrics.ascent_px,
        baseline_y_px: origin_px[1],
        max_width_px: (avail > 1.0).then_some(avail),
        text: text.clone(),
        style: web_style,
        font_css,
        color_rgba: [color.r, color.g, color.b, color.a],
        align: WebTextAlign::Left,
        direction,
        ellipsized: text.ends_with('\u{2026}'),
        node_id: op.owner_widget_id.as_ref().map(|id| id.as_str().to_owned()),
    })
}

fn paint_op_bounds_px(op: &PaintOp, clip: LayoutRectPx) -> Option<LayoutRectPx> {
    let target = match &op.kind {
        PaintOpKind::FillRect { rect, color } | PaintOpKind::StrokeRect { rect, color, .. } | PaintOpKind::DimRect { rect, color } => {
            if color.a == 0 {
                return None;
            }
            LayoutRectPx::from(*rect)
        }
        PaintOpKind::LineSegment { p0, p1, color, thickness } => {
            if color.a == 0 {
                return None;
            }
            let r = i32::try_from(*thickness / 2).unwrap_or(0);
            let stamp = (*thickness).max(1);
            let min_x = p0[0].min(p1[0]) - r;
            let min_y = p0[1].min(p1[1]) - r;
            let w = u32::try_from(p0[0].max(p1[0]).saturating_sub(p0[0].min(p1[0]))).unwrap_or(0).saturating_add(stamp);
            let h = u32::try_from(p0[1].max(p1[1]).saturating_sub(p0[1].min(p1[1]))).unwrap_or(0).saturating_add(stamp);
            LayoutRectPx::new(min_x, min_y, w, h)
        }
        PaintOpKind::CircleIndicator { center, radius, fill, stroke } => {
            if fill.a == 0 && stroke.is_none_or(|c| c.a == 0) {
                return None;
            }
            let r = i32::try_from(*radius).unwrap_or(0);
            let d = radius.saturating_mul(2).saturating_add(1);
            LayoutRectPx::new(center[0] - r, center[1] - r, d, d)
        }
        PaintOpKind::BlitImage { dst, .. } => LayoutRectPx::from(*dst),
        PaintOpKind::TextRun { .. } => return None,
    };
    target.intersect(clip)
}

fn rasterize_tree_band_layer(band: ZLayerBand, ops: &[(&PaintOp, LayoutRectPx)], dpi: u16) -> Option<WebRgbaLayer> {
    if ops.is_empty() {
        return None;
    }
    let mut bounds = LayoutRectPx::default();
    for (op, clip) in ops {
        if let Some(vis) = paint_op_bounds_px(op, *clip) {
            bounds = bounds.union(vis);
        }
    }
    if bounds.width == 0 || bounds.height == 0 {
        return None;
    }
    let byte_len = (bounds.width as usize).saturating_mul(bounds.height as usize).saturating_mul(4);
    let mut img = RasterImage { width_px: bounds.width, height_px: bounds.height, dpi, rgba: vec![0; byte_len] };
    for (op, clip) in ops {
        paint_tree_op_into_image(&mut img, bounds, op, *clip);
    }
    Some(WebRgbaLayer { z_band: band, rect_px: bounds, uses_base_frame: false, opaque: false, rgba_bytes: img.rgba })
}

fn paint_tree_op_into_image(img: &mut RasterImage, origin: LayoutRectPx, op: &PaintOp, clip: LayoutRectPx) {
    match &op.kind {
        PaintOpKind::FillRect { rect, color } | PaintOpKind::DimRect { rect, color } => {
            fill_clipped_rect_px(img, origin, LayoutRectPx::from(*rect), clip, [color.r, color.g, color.b, color.a]);
        }
        PaintOpKind::StrokeRect { rect, color, thickness } => {
            stroke_clipped_rect_px(img, origin, LayoutRectPx::from(*rect), clip, [color.r, color.g, color.b, color.a], *thickness);
        }
        PaintOpKind::LineSegment { p0, p1, color, thickness } => {
            draw_clipped_line_px(img, origin, *p0, *p1, clip, [color.r, color.g, color.b, color.a], *thickness);
        }
        PaintOpKind::CircleIndicator { center, radius, fill, stroke } => {
            let stroke_rgba = stroke.map(|c| [c.r, c.g, c.b, c.a]);
            draw_clipped_circle_px(img, origin, *center, *radius, clip, [fill.r, fill.g, fill.b, fill.a], stroke_rgba);
        }
        PaintOpKind::BlitImage { dst, image } => {
            blit_clipped_image_px(img, origin, LayoutRectPx::from(*dst), image, clip);
        }
        PaintOpKind::TextRun { .. } => {}
    }
}

fn fill_clipped_rect_px(img: &mut RasterImage, origin: LayoutRectPx, rect: LayoutRectPx, clip: LayoutRectPx, color: [u8; 4]) {
    if color[3] == 0 {
        return;
    }
    let Some(vis) = rect.intersect(origin).and_then(|r| r.intersect(clip)) else {
        return;
    };
    #[allow(clippy::cast_sign_loss)]
    let local_x = (vis.x - origin.x).max(0) as u32;
    #[allow(clippy::cast_sign_loss)]
    let local_y = (vis.y - origin.y).max(0) as u32;
    let x_end = local_x.saturating_add(vis.width).min(img.width_px);
    let y_end = local_y.saturating_add(vis.height).min(img.height_px);
    for py in local_y..y_end {
        for px in local_x..x_end {
            blend_pixel_rgba(img, px, py, color);
        }
    }
}

fn stroke_clipped_rect_px(img: &mut RasterImage, origin: LayoutRectPx, rect: LayoutRectPx, clip: LayoutRectPx, color: [u8; 4], thickness: u32) {
    if color[3] == 0 || rect.width == 0 || rect.height == 0 {
        return;
    }
    let Some(vis) = rect.intersect(origin).and_then(|r| r.intersect(clip)) else {
        return;
    };
    let sw = i32::try_from(thickness.max(1)).unwrap_or(1);
    let rect_r = rect.right();
    let rect_b = rect.bottom();
    for sy in vis.y..vis.bottom() {
        #[allow(clippy::cast_sign_loss)]
        let py = (sy - origin.y) as u32;
        if py >= img.height_px {
            continue;
        }
        for sx in vis.x..vis.right() {
            let on_edge = sx < rect.x.saturating_add(sw)
                || sx >= rect_r.saturating_sub(sw)
                || sy < rect.y.saturating_add(sw)
                || sy >= rect_b.saturating_sub(sw);
            if on_edge {
                #[allow(clippy::cast_sign_loss)]
                let px = (sx - origin.x) as u32;
                blend_pixel_rgba(img, px, py, color);
            }
        }
    }
}

fn draw_clipped_line_px(img: &mut RasterImage, origin: LayoutRectPx, p0: [i32; 2], p1: [i32; 2], clip: LayoutRectPx, color: [u8; 4], thickness: u32) {
    if color[3] == 0 {
        return;
    }
    let r = i32::try_from(thickness / 2).unwrap_or(0);
    let stamp = thickness.max(1);
    if p0[0] == p1[0] || p0[1] == p1[1] {
        let min_x = p0[0].min(p1[0]);
        let min_y = p0[1].min(p1[1]);
        let w = u32::try_from(p0[0].max(p1[0]).saturating_sub(min_x)).unwrap_or(0).saturating_add(stamp);
        let h = u32::try_from(p0[1].max(p1[1]).saturating_sub(min_y)).unwrap_or(0).saturating_add(stamp);
        fill_clipped_rect_px(img, origin, LayoutRectPx::new(min_x - r, min_y - r, w, h), clip, color);
        return;
    }
    let dx = (p1[0] - p0[0]).abs();
    let dy = (p1[1] - p0[1]).abs();
    let steps = dx.max(dy).max(1);
    for step in 0..=steps {
        let fx = p0[0] + (p1[0] - p0[0]) * step / steps;
        let fy = p0[1] + (p1[1] - p0[1]) * step / steps;
        fill_clipped_rect_px(img, origin, LayoutRectPx::new(fx - r, fy - r, stamp, stamp), clip, color);
    }
}

fn draw_clipped_circle_px(
    img: &mut RasterImage,
    origin: LayoutRectPx,
    center: [i32; 2],
    radius: u32,
    clip: LayoutRectPx,
    fill: [u8; 4],
    stroke: Option<[u8; 4]>,
) {
    let r = i32::try_from(radius).unwrap_or(0);
    if r <= 0 {
        return;
    }
    let Some(bounds) = clip.intersect(origin) else {
        return;
    };
    let r_sq = r.saturating_mul(r);
    let inner_sq = (r - 1).max(0).saturating_mul((r - 1).max(0));
    for dy in -r..=r {
        let sy = center[1].saturating_add(dy);
        if sy < bounds.y || sy >= bounds.bottom() {
            continue;
        }
        #[allow(clippy::cast_sign_loss)]
        let py = (sy - origin.y) as u32;
        for dx in -r..=r {
            let sx = center[0].saturating_add(dx);
            if sx < bounds.x || sx >= bounds.right() {
                continue;
            }
            let d2 = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
            if d2 <= r_sq {
                let c = if let Some(sc) = stroke
                    && d2 >= inner_sq
                {
                    sc
                } else {
                    fill
                };
                #[allow(clippy::cast_sign_loss)]
                let px = (sx - origin.x) as u32;
                blend_pixel_rgba(img, px, py, c);
            }
        }
    }
}

fn blit_clipped_image_px(dst_img: &mut RasterImage, origin: LayoutRectPx, dst_rect: LayoutRectPx, src_img: &RasterImage, clip: LayoutRectPx) {
    let Some(vis) = dst_rect.intersect(origin).and_then(|r| r.intersect(clip)) else {
        return;
    };
    if src_img.width_px == 0 || src_img.height_px == 0 {
        return;
    }
    let dw = dst_rect.width.max(1) as usize;
    let dh = dst_rect.height.max(1) as usize;
    let sw = src_img.width_px as usize;
    let sh = src_img.height_px as usize;
    for sy_screen in vis.y..vis.bottom() {
        #[allow(clippy::cast_sign_loss)]
        let local_y = (sy_screen - dst_rect.y).max(0) as usize;
        let src_y = (local_y.saturating_mul(sh) / dh).min(sh - 1);
        let src_row = src_y.saturating_mul(sw);
        #[allow(clippy::cast_sign_loss)]
        let dst_y = (sy_screen - origin.y) as u32;
        for sx_screen in vis.x..vis.right() {
            #[allow(clippy::cast_sign_loss)]
            let local_x = (sx_screen - dst_rect.x).max(0) as usize;
            let src_x = (local_x.saturating_mul(sw) / dw).min(sw - 1);
            let sidx = src_row.saturating_add(src_x).saturating_mul(4);
            let Some(src) = src_img.rgba.get(sidx..sidx + 4) else {
                continue;
            };
            #[allow(clippy::cast_sign_loss)]
            let dst_x = (sx_screen - origin.x) as u32;
            blend_pixel_rgba(dst_img, dst_x, dst_y, [src[0], src[1], src[2], src[3]]);
        }
    }
}

fn rasterize_band_layer(band: ZLayerBand, viewport_rect: LayoutRectPx, prims: &[WebRectPrimitive]) -> Option<WebRgbaLayer> {
    if prims.is_empty() || viewport_rect.width == 0 || viewport_rect.height == 0 {
        return None;
    }
    let mut bounds = LayoutRectPx::default();
    for prim in prims {
        if let Some(clipped) = prim.rect_px.intersect(viewport_rect) {
            bounds = bounds.union(clipped);
        }
    }
    if bounds.width == 0 || bounds.height == 0 {
        return None;
    }

    let byte_len = (bounds.width as usize).saturating_mul(bounds.height as usize).saturating_mul(4);
    let mut img = RasterImage { width_px: bounds.width, height_px: bounds.height, dpi: CSS_REFERENCE_DPI, rgba: vec![0; byte_len] };
    for prim in prims {
        paint_rect_primitive(&mut img, bounds, prim);
    }

    Some(WebRgbaLayer { z_band: band, rect_px: bounds, uses_base_frame: false, opaque: false, rgba_bytes: img.rgba })
}

fn paint_rect_primitive(img: &mut RasterImage, origin: LayoutRectPx, prim: &WebRectPrimitive) {
    let clip = prim.clip_px.unwrap_or(origin);
    if let Some(fill) = prim.fill {
        fill_clipped_rect_px(img, origin, prim.rect_px, clip, fill);
    }
    if let Some(stroke) = prim.stroke {
        stroke_clipped_rect_px(img, origin, prim.rect_px, clip, stroke, prim.stroke_width_px.max(1));
    }
}

fn blend_pixel_rgba(img: &mut RasterImage, x: u32, y: u32, src: [u8; 4]) {
    if x >= img.width_px || y >= img.height_px || src[3] == 0 {
        return;
    }
    let idx = (y as usize).saturating_mul(img.width_px as usize).saturating_add(x as usize).saturating_mul(4);
    let Some(dst) = img.rgba.get_mut(idx..idx + 4) else {
        return;
    };
    if src[3] == 255 || dst[3] == 0 {
        dst.copy_from_slice(&src);
        return;
    }
    let src_a = u32::from(src[3]);
    let dst_a = u32::from(dst[3]);
    let inv = 255 - src_a;
    #[allow(clippy::cast_possible_truncation)]
    if dst_a == 255 {
        dst[0] = ((u32::from(src[0]) * src_a + u32::from(dst[0]) * inv + 127) / 255) as u8;
        dst[1] = ((u32::from(src[1]) * src_a + u32::from(dst[1]) * inv + 127) / 255) as u8;
        dst[2] = ((u32::from(src[2]) * src_a + u32::from(dst[2]) * inv + 127) / 255) as u8;
        dst[3] = 255;
    } else {
        let out_a_num = src_a * 255 + dst_a * inv;
        if out_a_num == 0 {
            return;
        }
        dst[0] = ((u32::from(src[0]) * src_a * 255 + u32::from(dst[0]) * dst_a * inv + out_a_num / 2) / out_a_num).min(255) as u8;
        dst[1] = ((u32::from(src[1]) * src_a * 255 + u32::from(dst[1]) * dst_a * inv + out_a_num / 2) / out_a_num).min(255) as u8;
        dst[2] = ((u32::from(src[2]) * src_a * 255 + u32::from(dst[2]) * dst_a * inv + out_a_num / 2) / out_a_num).min(255) as u8;
        dst[3] = ((out_a_num + 127) / 255).min(255) as u8;
    }
}

/// Serialize `text_ops` cleanly for JSON transport.
#[must_use]
pub fn encode_text_ops_json(text_ops: &[WebTextDrawOp]) -> Value {
    Value::Array(text_ops.iter().map(WebTextDrawOp::to_json_value).collect())
}

/// Decode `text_ops` from a JSON array field if present.
///
/// # Errors
/// Returns [`WebAdapterError::Serialization`] when the JSON field is present but malformed.
pub fn decode_text_ops_json(obj: &Map<String, Value>) -> Result<Vec<WebTextDrawOp>, WebAdapterError> {
    let Some(raw) = obj.get("text_ops") else {
        return Ok(Vec::new());
    };
    if raw.is_null() {
        return Ok(Vec::new());
    }
    let arr = raw.as_array().ok_or_else(|| WebAdapterError::Serialization(String::from("`text_ops` must be an array")))?;
    arr.iter().map(WebTextDrawOp::from_json_value).collect()
}

/// Decode `rgba_layers` from a JSON array field if present.
///
/// # Errors
/// Returns [`WebAdapterError::Serialization`] when the JSON field is present but malformed.
pub fn decode_rgba_layers_json(obj: &Map<String, Value>) -> Result<Vec<WebRgbaLayer>, WebAdapterError> {
    let Some(raw) = obj.get("rgba_layers") else {
        return Ok(Vec::new());
    };
    if raw.is_null() {
        return Ok(Vec::new());
    }
    let arr = raw.as_array().ok_or_else(|| WebAdapterError::Serialization(String::from("`rgba_layers` must be an array")))?;
    arr.iter().map(WebRgbaLayer::from_json_value).collect()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[derive(Debug, Default)]
    struct CountingMeasureBackend {
        calls: Cell<usize>,
    }

    impl CanvasMeasureBackend for CountingMeasureBackend {
        fn measure_uncached(&self, style: &UiTextStyle, font_css: &str, text: &str, dpi: u16) -> MeasuredTextMetrics {
            self.calls.set(self.calls.get() + 1);
            assert!(!font_css.is_empty());
            measure_deterministic_px(style, text, dpi)
        }
    }

    #[test]
    fn browser_canvas_text_measurer_caches_and_evicts_deterministically() {
        let measurer = BrowserCanvasTextMeasurer::with_backend(CountingMeasureBackend::default(), 2);
        let style = UiTextStyle::ui_sans(13.0);

        let m1 = measurer.measure(&style, "One-way ANOVA", 96);
        let m2 = measurer.measure(&style, "One-way ANOVA", 96);
        assert_eq!(m1, m2);
        assert_eq!(measurer.backend().calls.get(), 1);
        assert_eq!(measurer.cache_stats().hits, 1);
        assert_eq!(measurer.cache_stats().misses, 1);

        // Different DPI misses cache and scales metrics proportionally.
        let m_hidpi = measurer.measure(&style, "One-way ANOVA", 192);
        assert!((m_hidpi.width_px - m1.width_px * 2.0).abs() < 1e-6);
        assert_eq!(measurer.backend().calls.get(), 2);

        // Third distinct entry evicts the first entry because capacity = 2.
        let _ = measurer.measure(&style, "Tukey post-hoc", 96);
        let stats = measurer.cache_stats();
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.evictions, 1);

        // Evicted key misses and invokes backend again.
        let _ = measurer.measure(&style, "One-way ANOVA", 96);
        assert_eq!(measurer.backend().calls.get(), 4);
    }

    #[test]
    fn tabular_numerals_align_and_bidi_shapes_rtl_and_unicode() {
        let measurer = BrowserCanvasTextMeasurer::new();
        let tab = UiTextStyle::grid_tabular(12.0);
        let w_ones = measurer.measure(&tab, "111.11", 96).width_px;
        let w_eights = measurer.measure(&tab, "888.88", 96).width_px;
        assert!((w_ones - w_eights).abs() < 1e-9, "tabular digits must share uniform width");

        let shaped_rtl = measurer.shape_line(&UiTextStyle::ui_sans(13.0), "تحليل 123", 144);
        assert_eq!(shaped_rtl.direction, TextDirection::Rtl);
        assert_eq!(shaped_rtl.missing_glyph_count, 0);
        assert!(!shaped_rtl.glyphs.is_empty());

        let shaped_math = measurer.shape_line(&UiTextStyle::ui_sans(13.0), "α ≤ 0.05 (μ ± SD)", 96);
        assert_eq!(shaped_math.direction, TextDirection::Ltr);
        assert!(shaped_math.metrics.width_px > 0.0);
    }

    #[test]
    fn z_band_splitting_preserves_band_order_clips_and_alpha_overlays() {
        let mut base = RasterImage { width_px: 320, height_px: 200, dpi: CSS_REFERENCE_DPI, rgba: vec![0; 320 * 200 * 4] };
        for px in base.rgba.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&[245, 246, 248, 255]);
        }
        let prims = vec![
            WebRectPrimitive {
                z_layer: ZLayer::Tooltip,
                rect_px: LayoutRectPx::new(20, 140, 120, 28),
                clip_px: None,
                fill: Some([20, 24, 34, 245]),
                stroke: Some([96, 165, 250, 255]),
                stroke_width_px: 1,
            },
            WebRectPrimitive {
                z_layer: ZLayer::ModalBackdrop,
                rect_px: LayoutRectPx::new(0, 0, 320, 200),
                clip_px: None,
                fill: Some([16, 20, 28, 120]),
                stroke: None,
                stroke_width_px: 0,
            },
            WebRectPrimitive {
                z_layer: ZLayer::ModalDialog,
                rect_px: LayoutRectPx::new(60, 40, 200, 120),
                clip_px: None,
                fill: Some([255, 255, 255, 255]),
                stroke: Some([29, 78, 216, 255]),
                stroke_width_px: 2,
            },
        ];

        let text_ops = vec![
            WebTextDrawOp {
                z_layer: ZLayer::Tooltip,
                z_band: ZLayerBand::TopOverlay,
                paint_order: 2,
                clip_px: Some(LayoutRectPx::new(22, 142, 116, 24)),
                x_px: 28.0,
                y_px: 146.0,
                baseline_y_px: 158.0,
                max_width_px: Some(112.0),
                text: String::from("Tooltip"),
                style: UiTextStyle::ui_sans(11.0),
                font_css: canvas_font_css(&UiTextStyle::ui_sans(11.0), 96),
                color_rgba: [255, 255, 255, 255],
                align: WebTextAlign::Start,
                direction: WebTextDirection::Ltr,
                ellipsized: false,
                node_id: Some(String::from("tooltip")),
            },
            WebTextDrawOp {
                z_layer: ZLayer::ModalDialog,
                z_band: ZLayerBand::ModalOverlay,
                paint_order: 1,
                clip_px: Some(LayoutRectPx::new(64, 44, 192, 24)),
                x_px: 72.0,
                y_px: 48.0,
                baseline_y_px: 62.0,
                max_width_px: Some(180.0),
                text: String::from("Edit Parameters"),
                style: UiTextStyle::ui_sans_bold(13.0),
                font_css: canvas_font_css(&UiTextStyle::ui_sans_bold(13.0), 96),
                color_rgba: [24, 28, 36, 255],
                align: WebTextAlign::Start,
                direction: WebTextDirection::Ltr,
                ellipsized: false,
                node_id: Some(String::from("modal.title")),
            },
            WebTextDrawOp {
                z_layer: ZLayer::Workspace,
                z_band: ZLayerBand::Workspace,
                paint_order: 0,
                clip_px: Some(LayoutRectPx::new(0, 0, 320, 28)),
                x_px: 12.0,
                y_px: 6.0,
                baseline_y_px: 19.0,
                max_width_px: Some(64.0),
                text: String::from("File"),
                style: UiTextStyle::ui_sans(13.0),
                font_css: canvas_font_css(&UiTextStyle::ui_sans(13.0), 96),
                color_rgba: [24, 28, 36, 255],
                align: WebTextAlign::Start,
                direction: WebTextDirection::Ltr,
                ellipsized: false,
                node_id: Some(String::from("menu.file")),
            },
        ];

        let frame = WebTextBridge::compose_layered_frame(base, &prims, text_ops);
        WebTextBridge::validate_layered_frame(&frame).expect("layered frame must validate cleanly");
        assert_eq!(
            frame.rgba_layers.iter().map(|l| l.z_band).collect::<Vec<_>>(),
            vec![ZLayerBand::Workspace, ZLayerBand::ModalOverlay, ZLayerBand::TopOverlay]
        );
        assert!(frame.rgba_layers[0].uses_base_frame);
        assert!(!frame.rgba_layers[1].uses_base_frame);
        assert_eq!(frame.text_ops.iter().map(|op| op.z_layer).collect::<Vec<_>>(), vec![ZLayer::Workspace, ZLayer::ModalDialog, ZLayer::Tooltip]);
    }

    #[test]
    fn compose_layout_tree_frame_separates_bands_and_defers_text_without_double_baking() {
        let mut app = curvebook_ui_egui::UiApp::new();
        app.canvas_dialogs.open_command_palette();
        app.canvas_dialogs.command_palette.query = String::from("undo");
        app.interaction.hovered_widget_id = Some(String::from("toolbar.export_svg"));
        let viewport = curvebook_ui_egui::ViewportMetrics::new(960.0, 600.0, 1.0);
        let palette = curvebook_ui_egui::compositor::FramePalette::from_theme(&app.theme);
        let tree = curvebook_ui_egui::views::build_workspace_layout_tree(
            &app,
            viewport,
            curvebook_ui_egui::text_bridge::DesktopTextMeasurer::global(),
            &palette,
        );
        let measurer = BrowserCanvasTextMeasurer::new();
        let bg = app.theme.surface_bg;
        let frame = WebTextBridge::compose_layout_tree_frame(&tree, &measurer, CSS_REFERENCE_DPI, [bg.r, bg.g, bg.b, bg.a]);
        WebTextBridge::validate_layered_frame(&frame).expect("composed LayoutTree frame validates cleanly");
        assert_eq!(frame.width_px, 960);
        assert_eq!(frame.height_px, 600);
        assert!(frame.rgba_layers.iter().any(|l| l.z_band == ZLayerBand::ModalOverlay && !l.uses_base_frame));
        assert!(frame.text_ops.iter().any(|op| op.z_layer == ZLayer::Workspace && op.node_id.as_deref() == Some("menu.file")));
        assert!(frame.text_ops.iter().any(|op| op.z_layer == ZLayer::ModalDialog && op.node_id.as_deref() == Some("modal.dialog")));
        assert!(frame.text_ops.iter().any(|op| op.z_layer == ZLayer::ModalDialog && op.node_id.as_deref() == Some("palette.query")));
        assert!(frame.text_ops.iter().any(|op| op.z_layer == ZLayer::Tooltip && op.node_id.as_deref() == Some("overlay.tooltip")));
    }
}
