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
    RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, TextDirection, contains_rtl_script, measure_text_pt, parse_markup, shape_bidi_runs,
};
use curvebook_ui_egui::RasterImage;
use curvebook_ui_egui::ZLayer as TreeZLayer;
use curvebook_ui_egui::text_bridge::{FontFamilyKind as TreeFontFamilyKind, UiTextStyle as TreeUiTextStyle};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};

use crate::chrome_bridge::{WebChromeLayout, WebChromeRole};
use crate::host_adapters::{WebAdapterError, decode_base64_str, encode_base64_slice};

/// Default maximum cached text measurements retained per [`BrowserCanvasTextMeasurer`].
pub const DEFAULT_TEXT_CACHE_CAPACITY: usize = 4096;

/// Baseline CSS pixel density (`1 CSS px = 1/96 in`).
pub const CSS_REFERENCE_DPI: u16 = 96;

/// Maximum combined RGBA byte budget across a layered frame and its overlay bands (`64 MiB`).
pub const MAX_FRAME_RGBA_BYTES: usize = 64 * 1024 * 1024;

/// Font family classification matching `curvebook-text` shaping categories (`#248` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FontFamilyKind {
    #[default]
    UiSans,
    UiMono,
    GridTabular,
    ScientificSerif,
}

impl<'de> Deserialize<'de> for FontFamilyKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from_name(&raw).unwrap_or(Self::UiSans))
    }
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

const fn default_style_size_px() -> f32 {
    13.0
}

mod span_weight_wire {
    use super::{Deserializer, Serialize, Serializer, SpanWeight};
    use serde::Deserialize;

    pub fn serialize<S: Serializer>(w: &SpanWeight, s: S) -> Result<S::Ok, S::Error> {
        match w {
            SpanWeight::Regular => "regular",
            SpanWeight::Bold => "bold",
        }
        .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SpanWeight, D::Error> {
        let raw = Option::<String>::deserialize(d)?;
        Ok(match raw.as_deref() {
            Some("bold" | "Bold") => SpanWeight::Bold,
            _ => SpanWeight::Regular,
        })
    }
}

mod span_slope_wire {
    use super::{Deserializer, Serialize, Serializer, SpanSlope};
    use serde::Deserialize;

    pub fn serialize<S: Serializer>(sl: &SpanSlope, s: S) -> Result<S::Ok, S::Error> {
        match sl {
            SpanSlope::Upright => "upright",
            SpanSlope::Italic => "italic",
        }
        .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SpanSlope, D::Error> {
        let raw = Option::<String>::deserialize(d)?;
        Ok(match raw.as_deref() {
            Some("italic" | "Italic") => SpanSlope::Italic,
            _ => SpanSlope::Upright,
        })
    }
}

/// Text styling descriptor passed to [`WebTextMeasurer`] and [`WebTextDrawOp`] (`#248` §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiTextStyle {
    pub family: FontFamilyKind,
    #[serde(default = "default_style_size_px")]
    pub size_px: f32,
    #[serde(with = "span_weight_wire")]
    pub weight: SpanWeight,
    #[serde(with = "span_slope_wire")]
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
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }

    /// Decode [`UiTextStyle`] from JSON.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when malformed.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        if !value.is_object() {
            return Err(WebAdapterError::Serialization(String::from("style must be an object")));
        }
        serde_json::from_value(value.clone()).map_err(|err| WebAdapterError::Serialization(err.to_string()))
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
        let runs = shape_bidi_runs(text, direction);
        let visual_text: String = runs.iter().map(|run| run.text.as_str()).collect();
        let metrics = self.measure(style, &visual_text, dpi);
        let eff_px = effective_font_px(style.size_px, dpi);
        let size_pt = eff_px * (72.0 / f64::from(CSS_REFERENCE_DPI));
        let pt_to_px = f64::from(CSS_REFERENCE_DPI) / 72.0;

        let mut glyphs = Vec::with_capacity(visual_text.chars().count());
        let mut pen_x = 0.0_f64;
        for run in &runs {
            let mut run_prev_w = 0.0_f64;
            let mut char_iter = run.text.char_indices().peekable();
            while let Some((idx, ch)) = char_iter.next() {
                let end = char_iter.peek().map_or(run.text.len(), |&(next_idx, _)| next_idx);
                let adv_px = if matches!(style.family, FontFamilyKind::UiMono) {
                    let adv = eff_px * 0.60;
                    run_prev_w += adv;
                    adv
                } else if style.tabular_nums && ch.is_ascii_digit() {
                    let adv = eff_px * 0.56;
                    run_prev_w += adv;
                    adv
                } else {
                    let prefix_w = run.text.get(..end).map_or(0.0, |prefix| self.measure(style, prefix, dpi).width_px);
                    let adv = if prefix_w > run_prev_w {
                        prefix_w - run_prev_w
                    } else if let Some(slice) = run.text.get(idx..end) {
                        let single_w = self.measure(style, slice, dpi).width_px;
                        if single_w > 0.0 { single_w } else { measure_text_pt(&RichText::plain(slice), size_pt).width_pt * pt_to_px }
                    } else {
                        eff_px * 0.5
                    };
                    run_prev_w = if prefix_w >= run_prev_w { prefix_w } else { run_prev_w + adv };
                    adv
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
        }

        ShapedTextLine { visual_text, direction, metrics, glyphs, missing_glyph_count: 0 }
    }
}

/// Z-order layers matching `crates/ui-egui/src/layout/mod.rs` (`#248` §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
#[serde(rename_all = "snake_case")]
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

impl<'de> Deserialize<'de> for ZLayer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from_name(&raw).unwrap_or(Self::Workspace))
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ZLayerBand {
    #[default]
    Workspace = 0,
    PopupDropdown = 1,
    ModalOverlay = 2,
    TopOverlay = 3,
}

impl<'de> Deserialize<'de> for ZLayerBand {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_name(&raw).ok_or_else(|| serde::de::Error::custom(format!("unknown z_band `{raw}`")))
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
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
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }

    /// Decode a [`LayoutRectPx`] from JSON with checked `i32`/`u32` bounds.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when integer fields are missing, invalid, or out of range.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        serde_json::from_value(value.clone()).map_err(|err| WebAdapterError::Serialization(err.to_string()))
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

/// Horizontal text alignment inside [`WebTextDrawOp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebTextAlign {
    #[default]
    Start,
    Center,
    End,
    Left,
    Right,
}

impl<'de> Deserialize<'de> for WebTextAlign {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from_name(&raw).unwrap_or(Self::Start))
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebTextDirection {
    #[default]
    Ltr,
    Rtl,
}

impl<'de> Deserialize<'de> for WebTextDirection {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from_name(&raw).unwrap_or(Self::Ltr))
    }
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

const fn default_opaque_black_rgba() -> [u8; 4] {
    [0, 0, 0, 255]
}

/// Single Z-ordered browser `ctx.fillText(...)` draw instruction (`#248` §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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
    #[serde(default = "default_opaque_black_rgba")]
    pub color_rgba: [u8; 4],
    pub align: WebTextAlign,
    pub direction: WebTextDirection,
    pub ellipsized: bool,
    pub node_id: Option<String>,
}

impl Default for WebTextDrawOp {
    fn default() -> Self {
        Self {
            z_layer: ZLayer::Workspace,
            z_band: ZLayerBand::Workspace,
            paint_order: 0,
            clip_px: None,
            x_px: 0.0,
            y_px: 0.0,
            baseline_y_px: 0.0,
            max_width_px: None,
            text: String::new(),
            style: UiTextStyle::default(),
            font_css: String::new(),
            color_rgba: default_opaque_black_rgba(),
            align: WebTextAlign::Start,
            direction: WebTextDirection::Ltr,
            ellipsized: false,
            node_id: None,
        }
    }
}

impl WebTextDrawOp {
    /// Encode this draw instruction as JSON for browser `ctx.fillText(...)` execution.
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }

    /// Decode a [`WebTextDrawOp`] from JSON.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when fields are invalid.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let obj = value.as_object().ok_or_else(|| WebAdapterError::Serialization(String::from("text_op must be an object")))?;
        let has_explicit_band = obj.get("z_band").is_some_and(|v| !v.is_null());
        let mut op: Self = serde_json::from_value(value.clone()).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        if !has_explicit_band {
            op.z_band = op.z_layer.band();
        }
        Ok(op)
    }
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

#[derive(Serialize, Deserialize)]
struct WebRgbaLayerWire {
    #[serde(default)]
    z_band: ZLayerBand,
    rect_px: LayoutRectPx,
    #[serde(default)]
    uses_base_frame: bool,
    #[serde(default)]
    opaque: bool,
    #[serde(default)]
    rgba_base64: String,
}

impl WebRgbaLayer {
    /// Encode this layer to JSON for `RenderRgbaFrame`.
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        let wire = WebRgbaLayerWire {
            z_band: self.z_band,
            rect_px: self.rect_px,
            uses_base_frame: self.uses_base_frame,
            opaque: self.opaque,
            rgba_base64: if self.uses_base_frame { String::new() } else { encode_base64_slice(&self.rgba_bytes) },
        };
        serde_json::to_value(wire).unwrap_or_else(|_| json!({}))
    }

    /// Decode a [`WebRgbaLayer`] from JSON.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when fields are invalid or [`WebAdapterError::Validation`] on oversized buffers.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let wire: WebRgbaLayerWire = serde_json::from_value(value.clone()).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        let rgba_bytes = if wire.uses_base_frame {
            Vec::new()
        } else {
            if wire.rect_px.width > 0 && wire.rect_px.height > 0 {
                let expected = checked_rgba_byte_len(wire.rect_px.width, wire.rect_px.height)?;
                if expected > MAX_FRAME_RGBA_BYTES {
                    return Err(WebAdapterError::Validation(format!(
                        "rgba_layer buffer size {expected} exceeds maximum {MAX_FRAME_RGBA_BYTES} bytes"
                    )));
                }
            }
            let max_b64_bytes = (MAX_FRAME_RGBA_BYTES / 3).saturating_add(2).saturating_mul(4);
            if wire.rgba_base64.len() > max_b64_bytes {
                return Err(WebAdapterError::Validation(format!("rgba_layer base64 length {} exceeds bounded frame budget", wire.rgba_base64.len())));
            }
            decode_base64_str(&wire.rgba_base64)?
        };
        Ok(Self { z_band: wire.z_band, rect_px: wire.rect_px, uses_base_frame: wire.uses_base_frame, opaque: wire.opaque, rgba_bytes })
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
    /// Split non-text RGBA background/overlay primitives and `text_ops` into sorted [`ZLayerBand`] layers,
    /// returning an error if combined RGBA buffers would exceed [`MAX_FRAME_RGBA_BYTES`] or fail reservation.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if frame dimensions or combined buffer bytes exceed the cap.
    pub fn try_compose_layered_frame(
        base_frame: RasterImage,
        rect_primitives: &[WebRectPrimitive],
        mut text_ops: Vec<WebTextDrawOp>,
    ) -> Result<WebLayeredFrame, WebAdapterError> {
        let width_px = base_frame.width_px;
        let height_px = base_frame.height_px;
        let base_expected = checked_rgba_byte_len(width_px, height_px)?;
        if base_frame.rgba.len() != base_expected {
            return Err(WebAdapterError::Validation(format!("base_frame rgba len {} != expected {base_expected}", base_frame.rgba.len())));
        }
        if base_expected > MAX_FRAME_RGBA_BYTES {
            return Err(WebAdapterError::Validation(format!("frame base buffer {base_expected} bytes exceeds {MAX_FRAME_RGBA_BYTES}-byte cap")));
        }

        let viewport_rect = LayoutRectPx::new(0, 0, width_px, height_px);
        let mut band_rects: [Vec<WebRectPrimitive>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for prim in rect_primitives {
            let idx = usize::from(prim.z_layer.band() as u8);
            if let Some(bucket) = band_rects.get_mut(idx) {
                bucket.push(*prim);
            }
        }

        // Preflight total byte budget across all non-workspace overlay bands BEFORE allocating any buffer.
        let mut projected_total = base_expected;
        let mut planned_bands: Vec<(ZLayerBand, LayoutRectPx, usize)> = Vec::with_capacity(3);
        for band in [ZLayerBand::PopupDropdown, ZLayerBand::ModalOverlay, ZLayerBand::TopOverlay] {
            let prims = band_rects.get(band as usize).map_or(&[][..], Vec::as_slice);
            if let Some(bounds) = band_bounds_for_primitives(viewport_rect, prims) {
                let band_bytes = checked_rgba_byte_len(bounds.width, bounds.height)?;
                projected_total = projected_total
                    .checked_add(band_bytes)
                    .filter(|&sum| sum <= MAX_FRAME_RGBA_BYTES)
                    .ok_or_else(|| WebAdapterError::Validation(format!("layered frame RGBA allocation exceeds {MAX_FRAME_RGBA_BYTES}-byte cap")))?;
                planned_bands.push((band, bounds, band_bytes));
            }
        }

        let mut base_img = base_frame;
        if let Some(ws_prims) = band_rects.get(ZLayerBand::Workspace as usize) {
            for prim in ws_prims {
                paint_rect_primitive(&mut base_img, viewport_rect, prim);
            }
        }

        let mut rgba_layers = Vec::with_capacity(1 + planned_bands.len());
        rgba_layers.push(WebRgbaLayer {
            z_band: ZLayerBand::Workspace,
            rect_px: viewport_rect,
            uses_base_frame: true,
            opaque: true,
            rgba_bytes: Vec::new(),
        });

        for (band, bounds, band_bytes) in planned_bands {
            let prims = band_rects.get(band as usize).map_or(&[][..], Vec::as_slice);
            rgba_layers.push(try_rasterize_band_layer(band, bounds, band_bytes, prims)?);
        }

        normalize_text_ops(&mut text_ops);
        Ok(WebLayeredFrame { width_px, height_px, base_rgba_bytes: base_img.rgba, rgba_layers, text_ops })
    }

    /// Split non-text RGBA background/overlay primitives and `text_ops` into sorted [`ZLayerBand`] layers
    /// while bounding allocations within [`MAX_FRAME_RGBA_BYTES`].
    #[must_use]
    pub fn compose_layered_frame(base_frame: RasterImage, rect_primitives: &[WebRectPrimitive], text_ops: Vec<WebTextDrawOp>) -> WebLayeredFrame {
        if let Ok(frame) = Self::try_compose_layered_frame(base_frame.clone(), rect_primitives, text_ops.clone()) {
            return frame;
        }
        // Bounded fallback if caller passed an oversized frame or overlay primitives: never allocate > cap.
        let mut safe_text = text_ops;
        normalize_text_ops(&mut safe_text);
        let safe_base = try_solid_raster_image(1, 1, base_frame.dpi.max(72), [24, 28, 36, 255]).unwrap_or(RasterImage {
            width_px: 1,
            height_px: 1,
            dpi: CSS_REFERENCE_DPI,
            rgba: vec![24, 28, 36, 255],
        });
        WebLayeredFrame {
            width_px: 1,
            height_px: 1,
            base_rgba_bytes: safe_base.rgba,
            rgba_layers: vec![WebRgbaLayer {
                z_band: ZLayerBand::Workspace,
                rect_px: LayoutRectPx::new(0, 0, 1, 1),
                uses_base_frame: true,
                opaque: true,
                rgba_bytes: Vec::new(),
            }],
            text_ops: safe_text,
        }
    }

    /// Render the minimal strict-CSP placeholder frame until full browser canvas rollout (`R11.11`),
    /// returning an error if dimensions exceed [`MAX_FRAME_RGBA_BYTES`].
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] when frame buffers exceed [`MAX_FRAME_RGBA_BYTES`].
    pub fn try_compose_placeholder_frame(
        width_px: u32,
        height_px: u32,
        dpi: u16,
        surface_bg: [u8; 4],
        status_label: &str,
        modal_active: bool,
    ) -> Result<WebLayeredFrame, WebAdapterError> {
        Self::try_compose_placeholder_frame_with_popup(width_px, height_px, dpi, surface_bg, status_label, false, modal_active)
    }

    /// Render a [`WebChromeLayout`] (menu bar/sections, dropdown items & shortcuts, toolbar buttons,
    /// context menu items, modal inputs/items/buttons, and status bar) into layered RGBA + [`WebTextDrawOp`]s,
    /// enforcing [`MAX_FRAME_RGBA_BYTES`] before any pixel allocation.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] when projected RGBA buffers exceed [`MAX_FRAME_RGBA_BYTES`]
    /// or allocation reservation fails.
    pub fn try_compose_chrome_frame(
        width_px: u32,
        height_px: u32,
        dpi: u16,
        surface_bg: [u8; 4],
        layout: &WebChromeLayout,
    ) -> Result<WebLayeredFrame, WebAdapterError> {
        let w = width_px.max(1);
        let h = height_px.max(1);
        let eff_dpi = dpi.max(72);
        let (rect_primitives, text_ops) = build_chrome_frame_primitives(w, h, eff_dpi, surface_bg, layout);
        preflight_layered_rgba_budget(w, h, &rect_primitives)?;
        let base_img = try_solid_raster_image(w, h, eff_dpi, [surface_bg[0], surface_bg[1], surface_bg[2], 255])?;
        Self::try_compose_layered_frame(base_img, &rect_primitives, text_ops)
    }

    /// Render the minimal strict-CSP placeholder frame with optional popup-dropdown and modal layers,
    /// enforcing [`MAX_FRAME_RGBA_BYTES`] before any pixel allocation.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] when projected RGBA buffers exceed [`MAX_FRAME_RGBA_BYTES`]
    /// or allocation reservation fails.
    pub fn try_compose_placeholder_frame_with_popup(
        width_px: u32,
        height_px: u32,
        dpi: u16,
        surface_bg: [u8; 4],
        status_label: &str,
        popup_open: bool,
        modal_active: bool,
    ) -> Result<WebLayeredFrame, WebAdapterError> {
        let w = width_px.max(1);
        let h = height_px.max(1);
        let _total = project_placeholder_frame_bytes(w, h, popup_open, modal_active)?;
        let base_img = try_solid_raster_image(w, h, dpi.max(72), [surface_bg[0], surface_bg[1], surface_bg[2], 255])?;
        let (rect_primitives, text_ops) = build_placeholder_primitives(w, h, dpi, surface_bg, status_label, popup_open, modal_active);
        Self::try_compose_layered_frame(base_img, &rect_primitives, text_ops)
    }

    /// Render the minimal strict-CSP placeholder frame until full browser canvas rollout (`R11.11`).
    #[must_use]
    pub fn compose_placeholder_frame(
        width_px: u32,
        height_px: u32,
        dpi: u16,
        surface_bg: [u8; 4],
        status_label: &str,
        modal_active: bool,
    ) -> WebLayeredFrame {
        Self::compose_placeholder_frame_with_popup(width_px, height_px, dpi, surface_bg, status_label, false, modal_active)
    }

    /// Render the minimal strict-CSP placeholder frame with optional popup-dropdown and modal layers,
    /// clamping oversized dimensions into [`MAX_FRAME_RGBA_BYTES`] so direct callers never abort on OOM.
    #[must_use]
    pub fn compose_placeholder_frame_with_popup(
        width_px: u32,
        height_px: u32,
        dpi: u16,
        surface_bg: [u8; 4],
        status_label: &str,
        popup_open: bool,
        modal_active: bool,
    ) -> WebLayeredFrame {
        let (bw, bh) = bound_placeholder_dimensions(width_px.max(1), height_px.max(1), popup_open, modal_active);
        if let Ok(frame) = Self::try_compose_placeholder_frame_with_popup(bw, bh, dpi, surface_bg, status_label, popup_open, modal_active) {
            return frame;
        }
        Self::try_compose_placeholder_frame_with_popup(1, 1, dpi, surface_bg, status_label, false, false).unwrap_or(WebLayeredFrame {
            width_px: 1,
            height_px: 1,
            base_rgba_bytes: vec![surface_bg[0], surface_bg[1], surface_bg[2], 255],
            rgba_layers: vec![WebRgbaLayer {
                z_band: ZLayerBand::Workspace,
                rect_px: LayoutRectPx::new(0, 0, 1, 1),
                uses_base_frame: true,
                opaque: true,
                rgba_bytes: Vec::new(),
            }],
            text_ops: Vec::new(),
        })
    }

    /// Validate Z-order invariants, RGBA layer byte lengths, total frame byte budget, and text clip bounds on `frame`.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if any band, buffer size, total budget, or text draw op is malformed.
    pub fn validate_layered_frame(frame: &WebLayeredFrame) -> Result<(), WebAdapterError> {
        let expected_base = checked_rgba_byte_len(frame.width_px, frame.height_px)?;
        if frame.base_rgba_bytes.len() != expected_base {
            return Err(WebAdapterError::Validation(format!("base_rgba_bytes len {} != expected {expected_base}", frame.base_rgba_bytes.len())));
        }
        let mut total_rgba_bytes = frame.base_rgba_bytes.len();
        if total_rgba_bytes > MAX_FRAME_RGBA_BYTES {
            return Err(WebAdapterError::Validation(format!("base_rgba_bytes len {total_rgba_bytes} exceeds {MAX_FRAME_RGBA_BYTES}-byte cap")));
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
                let expected = checked_rgba_byte_len(layer.rect_px.width, layer.rect_px.height)?;
                if layer.rgba_bytes.len() != expected {
                    return Err(WebAdapterError::Validation(format!(
                        "band {:?} rgba_bytes len {} != expected {expected}",
                        layer.z_band,
                        layer.rgba_bytes.len()
                    )));
                }
                total_rgba_bytes = total_rgba_bytes
                    .checked_add(layer.rgba_bytes.len())
                    .filter(|&sum| sum <= MAX_FRAME_RGBA_BYTES)
                    .ok_or_else(|| WebAdapterError::Validation(format!("total layered RGBA bytes exceed {MAX_FRAME_RGBA_BYTES}-byte cap")))?;
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

fn checked_rgba_byte_len(width_px: u32, height_px: u32) -> Result<usize, WebAdapterError> {
    if width_px == 0 || height_px == 0 {
        return Err(WebAdapterError::Validation(String::from("RGBA dimensions must be positive")));
    }
    let w = usize::try_from(width_px).map_err(|_| WebAdapterError::Validation(String::from("frame width exceeds addressable range")))?;
    let h = usize::try_from(height_px).map_err(|_| WebAdapterError::Validation(String::from("frame height exceeds addressable range")))?;
    let pixels =
        w.checked_mul(h).ok_or_else(|| WebAdapterError::Validation(format!("frame dimensions {width_px}x{height_px} overflow pixel count")))?;
    pixels.checked_mul(4).ok_or_else(|| WebAdapterError::Validation(format!("frame dimensions {width_px}x{height_px} overflow RGBA byte count")))
}

fn band_bounds_for_primitives(viewport_rect: LayoutRectPx, prims: &[WebRectPrimitive]) -> Option<LayoutRectPx> {
    if prims.is_empty() || viewport_rect.width == 0 || viewport_rect.height == 0 {
        return None;
    }
    let mut bounds = LayoutRectPx::default();
    for prim in prims {
        if let Some(clipped) = prim.rect_px.intersect(viewport_rect) {
            bounds = bounds.union(clipped);
        }
    }
    (bounds.width > 0 && bounds.height > 0).then_some(bounds)
}

fn project_placeholder_frame_bytes(w: u32, h: u32, popup_open: bool, modal_active: bool) -> Result<usize, WebAdapterError> {
    let base_bytes = checked_rgba_byte_len(w, h)?;
    let popup_bytes = if popup_open {
        let pw = w.saturating_sub(16).min(220);
        let ph = h.saturating_sub(32).min(240);
        if pw > 0 && ph > 0 { checked_rgba_byte_len(pw, ph)? } else { 0 }
    } else {
        0
    };
    let modal_bytes = if modal_active { base_bytes } else { 0 };
    let total = base_bytes
        .checked_add(popup_bytes)
        .and_then(|s| s.checked_add(modal_bytes))
        .ok_or_else(|| WebAdapterError::Validation(format!("frame {w}x{h} overflows RGBA budget")))?;
    if total > MAX_FRAME_RGBA_BYTES {
        return Err(WebAdapterError::Validation(format!("frame {w}x{h} requires {total} RGBA bytes, exceeding {MAX_FRAME_RGBA_BYTES}-byte cap")));
    }
    Ok(total)
}

fn bound_placeholder_dimensions(w: u32, h: u32, popup_open: bool, modal_active: bool) -> (u32, u32) {
    if project_placeholder_frame_bytes(w, h, popup_open, modal_active).is_ok() {
        return (w, h);
    }
    let layer_multiplier: f64 = if modal_active { 2.0 } else { 1.0 };
    let popup_reserve: f64 = if popup_open { 220.0 * 240.0 * 4.0 } else { 0.0 };
    let avail_pixels = ((MAX_FRAME_RGBA_BYTES as f64 - popup_reserve).max(4096.0)) / (4.0 * layer_multiplier);
    let cur_pixels = f64::from(w) * f64::from(h);
    let scale = (avail_pixels / cur_pixels).sqrt().clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut bw = ((f64::from(w) * scale).floor() as u32).max(1);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut bh = ((f64::from(h) * scale).floor() as u32).max(1);
    while project_placeholder_frame_bytes(bw, bh, popup_open, modal_active).is_err() && (bw > 1 || bh > 1) {
        bw = (bw / 2).max(1);
        bh = (bh / 2).max(1);
    }
    (bw, bh)
}

fn normalize_text_ops(text_ops: &mut [WebTextDrawOp]) {
    text_ops.sort_by(|a, b| a.z_layer.cmp(&b.z_layer).then_with(|| a.paint_order.cmp(&b.paint_order)));
    for op in text_ops {
        op.z_band = op.z_layer.band();
        if op.font_css.is_empty() {
            op.font_css = canvas_font_css(&op.style, CSS_REFERENCE_DPI);
        }
    }
}

fn build_placeholder_primitives(
    w: u32,
    h: u32,
    dpi: u16,
    surface_bg: [u8; 4],
    status_label: &str,
    popup_open: bool,
    modal_active: bool,
) -> (Vec<WebRectPrimitive>, Vec<WebTextDrawOp>) {
    let viewport_rect = LayoutRectPx::new(0, 0, w, h);
    let header_h = h.min(36);
    let mut rect_primitives = vec![WebRectPrimitive {
        z_layer: ZLayer::Workspace,
        clip_px: Some(viewport_rect),
        rect_px: LayoutRectPx::new(0, 0, w, header_h),
        fill: Some([surface_bg[0].saturating_sub(10), surface_bg[1].saturating_sub(10), surface_bg[2].saturating_sub(8), 255]),
        stroke: None,
        stroke_width_px: 0,
    }];
    if popup_open {
        rect_primitives.push(WebRectPrimitive {
            z_layer: ZLayer::DropdownMenu,
            clip_px: Some(viewport_rect),
            rect_px: LayoutRectPx::new(8, 28, w.saturating_sub(16).min(220), h.saturating_sub(32).min(240)),
            fill: Some([30, 41, 59, 240]),
            stroke: Some([99, 102, 241, 255]),
            stroke_width_px: 1,
        });
    }
    if modal_active {
        rect_primitives.push(WebRectPrimitive {
            z_layer: ZLayer::ModalBackdrop,
            clip_px: Some(viewport_rect),
            rect_px: viewport_rect,
            fill: Some([15, 23, 42, 110]),
            stroke: None,
            stroke_width_px: 0,
        });
    }
    let style = UiTextStyle::ui_sans(13.0);
    let font_css = canvas_font_css(&style, dpi.max(72));
    let direction = if curvebook_text::contains_rtl_script(status_label) { WebTextDirection::Rtl } else { WebTextDirection::Ltr };
    let mut text_ops = Vec::new();
    if !status_label.is_empty() {
        text_ops.push(WebTextDrawOp {
            z_layer: if modal_active { ZLayer::ModalDialog } else { ZLayer::Workspace },
            z_band: if modal_active { ZLayerBand::ModalOverlay } else { ZLayerBand::Workspace },
            paint_order: 1,
            clip_px: Some(viewport_rect),
            x_px: 16.0,
            y_px: 10.0,
            baseline_y_px: 23.0,
            max_width_px: Some(f64::from(w.saturating_sub(32))),
            text: status_label.to_owned(),
            style: style.clone(),
            font_css: font_css.clone(),
            color_rgba: [226, 232, 240, 255],
            align: WebTextAlign::Left,
            direction,
            ellipsized: false,
            node_id: Some(String::from("status.bar")),
        });
    }
    if popup_open {
        let pop_clip = LayoutRectPx::new(8, 28, w.saturating_sub(16).min(220), h.saturating_sub(32).min(240));
        text_ops.push(WebTextDrawOp {
            z_layer: ZLayer::DropdownMenu,
            z_band: ZLayerBand::PopupDropdown,
            paint_order: 2,
            clip_px: Some(pop_clip),
            x_px: 16.0,
            y_px: 34.0,
            baseline_y_px: 47.0,
            max_width_px: Some(200.0),
            text: String::from("Menu"),
            style,
            font_css,
            color_rgba: [226, 232, 240, 255],
            align: WebTextAlign::Left,
            direction: WebTextDirection::Ltr,
            ellipsized: false,
            node_id: Some(String::from("popup.menu")),
        });
    }
    (rect_primitives, text_ops)
}

fn preflight_layered_rgba_budget(w: u32, h: u32, rect_primitives: &[WebRectPrimitive]) -> Result<(), WebAdapterError> {
    let mut total = checked_rgba_byte_len(w, h)?;
    if total > MAX_FRAME_RGBA_BYTES {
        return Err(WebAdapterError::Validation(format!("frame {w}x{h} base buffer {total} exceeds {MAX_FRAME_RGBA_BYTES}-byte cap")));
    }
    let viewport_rect = LayoutRectPx::new(0, 0, w, h);
    let mut band_rects: [Vec<WebRectPrimitive>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for prim in rect_primitives {
        let idx = usize::from(prim.z_layer.band() as u8);
        if let Some(bucket) = band_rects.get_mut(idx) {
            bucket.push(*prim);
        }
    }
    for band in [ZLayerBand::PopupDropdown, ZLayerBand::ModalOverlay, ZLayerBand::TopOverlay] {
        let prims = band_rects.get(band as usize).map_or(&[][..], Vec::as_slice);
        if let Some(bounds) = band_bounds_for_primitives(viewport_rect, prims) {
            let band_bytes = checked_rgba_byte_len(bounds.width, bounds.height)?;
            total = total
                .checked_add(band_bytes)
                .filter(|&sum| sum <= MAX_FRAME_RGBA_BYTES)
                .ok_or_else(|| WebAdapterError::Validation(format!("frame {w}x{h} overlay RGBA buffers exceed {MAX_FRAME_RGBA_BYTES}-byte cap")))?;
        }
    }
    Ok(())
}

fn build_chrome_frame_primitives(
    w: u32,
    h: u32,
    dpi: u16,
    surface_bg: [u8; 4],
    layout: &WebChromeLayout,
) -> (Vec<WebRectPrimitive>, Vec<WebTextDrawOp>) {
    let viewport_rect = LayoutRectPx::new(0, 0, w, h);
    let style = UiTextStyle::ui_sans(12.0);
    let font_css = canvas_font_css(&style, dpi.max(72));
    let mut rect_primitives = Vec::with_capacity(layout.nodes.len());
    let mut text_ops = Vec::with_capacity(layout.nodes.len());
    let mut order = 0u32;
    let mut next_order = || {
        order = order.saturating_add(1);
        order
    };

    let text_dir = |s: &str| {
        if curvebook_text::contains_rtl_script(s) { WebTextDirection::Rtl } else { WebTextDirection::Ltr }
    };

    for node in &layout.nodes {
        match node.role {
            WebChromeRole::MenuBar | WebChromeRole::StatusBar => {
                rect_primitives.push(WebRectPrimitive {
                    z_layer: node.z_layer,
                    clip_px: Some(viewport_rect),
                    rect_px: node.rect_px,
                    fill: Some([surface_bg[0].saturating_sub(10), surface_bg[1].saturating_sub(10), surface_bg[2].saturating_sub(8), 255]),
                    stroke: None,
                    stroke_width_px: 0,
                });
                if node.role == WebChromeRole::StatusBar && !node.label.is_empty() {
                    text_ops.push(WebTextDrawOp {
                        z_layer: node.z_layer,
                        z_band: node.z_layer.band(),
                        paint_order: next_order(),
                        clip_px: Some(node.clip_px),
                        x_px: f64::from(node.rect_px.x) + 8.0,
                        y_px: f64::from(node.rect_px.y) + 4.0,
                        baseline_y_px: f64::from(node.rect_px.y) + 16.0,
                        max_width_px: Some(f64::from(node.rect_px.width.saturating_sub(16))),
                        text: node.label.clone(),
                        style: style.clone(),
                        font_css: font_css.clone(),
                        color_rgba: [226, 232, 240, 255],
                        align: WebTextAlign::Left,
                        direction: text_dir(&node.label),
                        ellipsized: false,
                        node_id: Some(node.id.clone()),
                    });
                }
            }
            WebChromeRole::MenuSection | WebChromeRole::ToolbarButton | WebChromeRole::ModalButton | WebChromeRole::ModalCloseButton => {
                let (fill, stroke, text_col) = if node.role == WebChromeRole::ModalButton && node.id == "modal.ok" {
                    ([37, 99, 235, 255], [96, 165, 250, 255], [255, 255, 255, 255])
                } else if node.role == WebChromeRole::MenuSection {
                    ([30, 41, 59, 180], [51, 65, 85, 255], [226, 232, 240, 255])
                } else if node.enabled {
                    ([30, 41, 59, 230], [71, 85, 105, 255], [241, 245, 249, 255])
                } else {
                    ([24, 30, 44, 210], [51, 65, 85, 180], [148, 163, 184, 255])
                };
                rect_primitives.push(WebRectPrimitive {
                    z_layer: node.z_layer,
                    clip_px: Some(node.clip_px),
                    rect_px: node.rect_px,
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width_px: 1,
                });
                if !node.label.is_empty() {
                    text_ops.push(WebTextDrawOp {
                        z_layer: node.z_layer,
                        z_band: node.z_layer.band(),
                        paint_order: next_order(),
                        clip_px: Some(node.clip_px),
                        x_px: f64::from(node.rect_px.x) + 6.0,
                        y_px: f64::from(node.rect_px.y) + 6.0,
                        baseline_y_px: f64::from(node.rect_px.y) + 18.0,
                        max_width_px: Some(f64::from(node.rect_px.width.saturating_sub(12).max(1))),
                        text: node.label.clone(),
                        style: style.clone(),
                        font_css: font_css.clone(),
                        color_rgba: text_col,
                        align: WebTextAlign::Left,
                        direction: text_dir(&node.label),
                        ellipsized: false,
                        node_id: Some(node.id.clone()),
                    });
                }
            }
            WebChromeRole::DropdownMenu | WebChromeRole::ContextMenu => {
                rect_primitives.push(WebRectPrimitive {
                    z_layer: node.z_layer,
                    clip_px: Some(viewport_rect),
                    rect_px: node.rect_px,
                    fill: Some([30, 41, 59, 244]),
                    stroke: Some([99, 102, 241, 255]),
                    stroke_width_px: 1,
                });
            }
            WebChromeRole::MenuItem | WebChromeRole::ModalItem => {
                let text_col = if node.enabled { [241, 245, 249, 255] } else { [148, 163, 184, 255] };
                if !node.label.is_empty() {
                    text_ops.push(WebTextDrawOp {
                        z_layer: node.z_layer,
                        z_band: node.z_layer.band(),
                        paint_order: next_order(),
                        clip_px: Some(node.clip_px),
                        x_px: f64::from(node.rect_px.x) + 4.0,
                        y_px: f64::from(node.rect_px.y) + 1.0,
                        baseline_y_px: f64::from(node.rect_px.y) + 11.0,
                        max_width_px: Some(f64::from(node.rect_px.width.saturating_sub(8).max(1))),
                        text: node.label.clone(),
                        style: style.clone(),
                        font_css: font_css.clone(),
                        color_rgba: text_col,
                        align: WebTextAlign::Left,
                        direction: text_dir(&node.label),
                        ellipsized: false,
                        node_id: Some(node.id.clone()),
                    });
                }
                if let Some(ref sc) = node.shortcut
                    && !sc.is_empty()
                {
                    text_ops.push(WebTextDrawOp {
                        z_layer: node.z_layer,
                        z_band: node.z_layer.band(),
                        paint_order: next_order(),
                        clip_px: Some(node.clip_px),
                        x_px: f64::from(node.rect_px.right()) - 4.0,
                        y_px: f64::from(node.rect_px.y) + 1.0,
                        baseline_y_px: f64::from(node.rect_px.y) + 11.0,
                        max_width_px: Some(f64::from(node.rect_px.width / 2)),
                        text: sc.clone(),
                        style: style.clone(),
                        font_css: font_css.clone(),
                        color_rgba: [148, 163, 184, 255],
                        align: WebTextAlign::Right,
                        direction: WebTextDirection::Ltr,
                        ellipsized: false,
                        node_id: Some(format!("{}.shortcut", node.id)),
                    });
                }
            }
            WebChromeRole::ModalBackdrop => {
                rect_primitives.push(WebRectPrimitive {
                    z_layer: ZLayer::ModalBackdrop,
                    clip_px: Some(viewport_rect),
                    rect_px: node.rect_px,
                    fill: Some([15, 23, 42, 110]),
                    stroke: None,
                    stroke_width_px: 0,
                });
            }
            WebChromeRole::ModalDialog => {
                rect_primitives.push(WebRectPrimitive {
                    z_layer: ZLayer::ModalDialog,
                    clip_px: Some(viewport_rect),
                    rect_px: node.rect_px,
                    fill: Some([24, 30, 44, 250]),
                    stroke: Some([99, 102, 241, 255]),
                    stroke_width_px: 1,
                });
                if !node.label.is_empty() {
                    text_ops.push(WebTextDrawOp {
                        z_layer: node.z_layer,
                        z_band: node.z_layer.band(),
                        paint_order: next_order(),
                        clip_px: Some(node.clip_px),
                        x_px: f64::from(node.rect_px.x) + 16.0,
                        y_px: f64::from(node.rect_px.y) + 12.0,
                        baseline_y_px: f64::from(node.rect_px.y) + 24.0,
                        max_width_px: Some(f64::from(node.rect_px.width.saturating_sub(32).max(1))),
                        text: node.label.clone(),
                        style: style.clone(),
                        font_css: font_css.clone(),
                        color_rgba: [248, 250, 252, 255],
                        align: WebTextAlign::Left,
                        direction: text_dir(&node.label),
                        ellipsized: false,
                        node_id: Some(node.id.clone()),
                    });
                }
            }
            WebChromeRole::ModalInput => {
                rect_primitives.push(WebRectPrimitive {
                    z_layer: node.z_layer,
                    clip_px: Some(node.clip_px),
                    rect_px: node.rect_px,
                    fill: Some([15, 23, 42, 255]),
                    stroke: Some([100, 116, 139, 255]),
                    stroke_width_px: 1,
                });
                if !node.label.is_empty() {
                    text_ops.push(WebTextDrawOp {
                        z_layer: node.z_layer,
                        z_band: node.z_layer.band(),
                        paint_order: next_order(),
                        clip_px: Some(node.clip_px),
                        x_px: f64::from(node.rect_px.x) + 8.0,
                        y_px: f64::from(node.rect_px.y) + 6.0,
                        baseline_y_px: f64::from(node.rect_px.y) + 18.0,
                        max_width_px: Some(f64::from(node.rect_px.width.saturating_sub(16).max(1))),
                        text: node.label.clone(),
                        style: style.clone(),
                        font_css: font_css.clone(),
                        color_rgba: [241, 245, 249, 255],
                        align: WebTextAlign::Left,
                        direction: text_dir(&node.label),
                        ellipsized: false,
                        node_id: Some(node.id.clone()),
                    });
                }
            }
            WebChromeRole::Tooltip => {
                rect_primitives.push(WebRectPrimitive {
                    z_layer: ZLayer::Tooltip,
                    clip_px: Some(viewport_rect),
                    rect_px: node.rect_px,
                    fill: Some([15, 23, 42, 245]),
                    stroke: Some([96, 165, 250, 255]),
                    stroke_width_px: 1,
                });
                if !node.label.is_empty() {
                    text_ops.push(WebTextDrawOp {
                        z_layer: node.z_layer,
                        z_band: node.z_layer.band(),
                        paint_order: next_order(),
                        clip_px: Some(node.clip_px),
                        x_px: f64::from(node.rect_px.x) + 6.0,
                        y_px: f64::from(node.rect_px.y) + 4.0,
                        baseline_y_px: f64::from(node.rect_px.y) + 16.0,
                        max_width_px: Some(f64::from(node.rect_px.width.saturating_sub(12).max(1))),
                        text: node.label.clone(),
                        style: style.clone(),
                        font_css: font_css.clone(),
                        color_rgba: [241, 245, 249, 255],
                        align: WebTextAlign::Left,
                        direction: text_dir(&node.label),
                        ellipsized: false,
                        node_id: Some(node.id.clone()),
                    });
                }
            }
        }
    }
    (rect_primitives, text_ops)
}

fn try_solid_raster_image(width_px: u32, height_px: u32, dpi: u16, rgba: [u8; 4]) -> Result<RasterImage, WebAdapterError> {
    let byte_len = checked_rgba_byte_len(width_px, height_px)?;
    if byte_len > MAX_FRAME_RGBA_BYTES {
        return Err(WebAdapterError::Validation(format!(
            "solid raster {width_px}x{height_px} ({byte_len} bytes) exceeds {MAX_FRAME_RGBA_BYTES}-byte cap"
        )));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(byte_len)
        .map_err(|_| WebAdapterError::Validation(format!("failed reserving {byte_len} bytes for {width_px}x{height_px} RGBA buffer")))?;
    for _ in 0..(byte_len / 4) {
        bytes.extend_from_slice(&rgba);
    }
    Ok(RasterImage { width_px, height_px, dpi, rgba: bytes })
}

fn try_rasterize_band_layer(
    band: ZLayerBand,
    bounds: LayoutRectPx,
    byte_len: usize,
    prims: &[WebRectPrimitive],
) -> Result<WebRgbaLayer, WebAdapterError> {
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(byte_len).map_err(|_| WebAdapterError::Validation(format!("failed reserving {byte_len} bytes for band {band:?}")))?;
    rgba.resize(byte_len, 0);
    let mut img = RasterImage { width_px: bounds.width, height_px: bounds.height, dpi: CSS_REFERENCE_DPI, rgba };
    for prim in prims {
        paint_rect_primitive(&mut img, bounds, prim);
    }
    Ok(WebRgbaLayer { z_band: band, rect_px: bounds, uses_base_frame: false, opaque: false, rgba_bytes: img.rgba })
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
    let Some(dst_slice) = img.rgba.get_mut(idx..idx + 4) else {
        return;
    };
    let [dr, dg, db, da] = dst_slice else {
        return;
    };
    if src[3] == 255 || *da == 0 {
        *dr = src[0];
        *dg = src[1];
        *db = src[2];
        *da = src[3];
        return;
    }
    let src_a = u32::from(src[3]);
    let dst_a = u32::from(*da);
    let inv = 255 - src_a;
    #[allow(clippy::cast_possible_truncation)]
    if dst_a == 255 {
        *dr = ((u32::from(src[0]) * src_a + u32::from(*dr) * inv + 127) / 255) as u8;
        *dg = ((u32::from(src[1]) * src_a + u32::from(*dg) * inv + 127) / 255) as u8;
        *db = ((u32::from(src[2]) * src_a + u32::from(*db) * inv + 127) / 255) as u8;
        *da = 255;
    } else {
        let out_a_num = src_a * 255 + dst_a * inv;
        if out_a_num == 0 {
            return;
        }
        *dr = ((u32::from(src[0]) * src_a * 255 + u32::from(*dr) * dst_a * inv + out_a_num / 2) / out_a_num).min(255) as u8;
        *dg = ((u32::from(src[1]) * src_a * 255 + u32::from(*dg) * dst_a * inv + out_a_num / 2) / out_a_num).min(255) as u8;
        *db = ((u32::from(src[2]) * src_a * 255 + u32::from(*db) * dst_a * inv + out_a_num / 2) / out_a_num).min(255) as u8;
        *da = ((out_a_num + 127) / 255).min(255) as u8;
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
    fn compose_placeholder_frame_separates_bands_and_validates() {
        let frame = WebTextBridge::compose_placeholder_frame(960, 600, CSS_REFERENCE_DPI, [24, 28, 36, 255], "CurveBook Web Placeholder", true);
        WebTextBridge::validate_layered_frame(&frame).expect("composed placeholder frame validates cleanly");
        assert_eq!(frame.width_px, 960);
        assert_eq!(frame.height_px, 600);
        assert!(frame.rgba_layers.iter().any(|l| l.z_band == ZLayerBand::ModalOverlay && !l.uses_base_frame));
        assert!(frame.text_ops.iter().any(|op| op.node_id.as_deref() == Some("status.bar")));
    }
}
