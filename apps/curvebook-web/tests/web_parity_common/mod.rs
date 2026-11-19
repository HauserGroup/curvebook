#![forbid(unsafe_code)]
//! Shared test-only parity filtering for `apps/curvebook-web` integration tests (`R14.1`).
//!
//! Filters out `text_glyphs` missing-glyph violations strictly when every missing glyph in the
//! offending [`LayoutTextRun`] is a CJK/RTL character unbacked by the host OS font stack, while
//! preserving all ASCII/Latin/symbol `text_glyphs` checks and all non-`text_glyphs` invariants.

use std::sync::OnceLock;

use curvebook_ui_egui::layout::LayoutTree;
use curvebook_ui_egui::layout::verify::ParityViolation;
use curvebook_ui_egui::text_bridge::{DesktopTextMeasurer, TextMeasurer, UiTextStyle};
use curvebook_ui_egui::theme::FontBook;

/// Host font coverage across the six CJK and RTL script samples probed against
/// [`FontBook::with_system_and_fallback`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostScriptCoverage {
    pub ja: bool,
    pub zh_cn: bool,
    pub zh_tw: bool,
    pub ko: bool,
    pub ar: bool,
    pub he: bool,
}

impl HostScriptCoverage {
    /// Coverage flags representing a headless fontless runner lacking all CJK and RTL system fonts.
    #[must_use]
    pub const fn none() -> Self {
        Self { ja: false, zh_cn: false, zh_tw: false, ko: false, ar: false, he: false }
    }

    /// Coverage flags representing a host with all six CJK and RTL script families backed by fonts.
    #[must_use]
    pub const fn all() -> Self {
        Self { ja: true, zh_cn: true, zh_tw: true, ko: true, ar: true, he: true }
    }
}

/// Probe and cache host system font coverage for CJK/RTL sample strings using
/// `DesktopTextMeasurer::with_font_book(FontBook::with_system_and_fallback())`.
#[must_use]
pub fn host_script_coverage() -> HostScriptCoverage {
    static COVERAGE: OnceLock<HostScriptCoverage> = OnceLock::new();
    *COVERAGE.get_or_init(|| {
        let measurer = DesktopTextMeasurer::with_font_book(FontBook::with_system_and_fallback());
        let style = UiTextStyle::ui_sans(13.0);
        let covers = |locale: &str, sample: &str| measurer.shape_line(sample, &style, locale).missing_glyph_count == 0;
        HostScriptCoverage {
            ja: covers("ja", "日本語データ"),
            zh_cn: covers("zh-CN", "简体中文数据"),
            zh_tw: covers("zh-TW", "繁體中文數據"),
            ko: covers("ko", "한국어데이터"),
            ar: covers("ar", "البيانات"),
            he: covers("he", "נתונים"),
        }
    })
}

const fn is_hebrew_char(ch: char) -> bool {
    matches!(ch as u32, 0x0590..=0x05FF | 0xFB1D..=0xFB4F)
}

const fn is_arabic_char(ch: char) -> bool {
    matches!(ch as u32, 0x0600..=0x06FF | 0x0750..=0x077F | 0x0870..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)
}

const fn is_kana_char(ch: char) -> bool {
    matches!(ch as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF65..=0xFF9F)
}

const fn is_hangul_char(ch: char) -> bool {
    matches!(ch as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF)
}

const fn is_cjk_han_or_punct_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3000..=0x303F
            | 0x3100..=0x312F
            | 0x31A0..=0x31BF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFF64
            | 0xFFA0..=0xFFEF
            | 0x20000..=0x2CEAF
            | 0x2F800..=0x2FA1F
    )
}

const fn is_unbacked_cjk_rtl_char(ch: char, coverage: HostScriptCoverage) -> bool {
    if is_hebrew_char(ch) {
        return !coverage.he;
    }
    if is_arabic_char(ch) {
        return !coverage.ar;
    }
    if is_kana_char(ch) {
        return !coverage.ja;
    }
    if is_hangul_char(ch) {
        return !coverage.ko;
    }
    if is_cjk_han_or_punct_char(ch) {
        return !(coverage.ja && coverage.zh_cn && coverage.zh_tw);
    }
    false
}

fn is_unbacked_cjk_rtl_text_glyph_violation(tree: &LayoutTree, violation: &ParityViolation, coverage: HostScriptCoverage) -> bool {
    if violation.check != "text_glyphs" {
        return false;
    }
    let mut matched = false;
    for run in &tree.text_runs {
        if run.owner_widget_id.0 != violation.widget_id || run.shaped.missing_glyph_count == 0 {
            continue;
        }
        let expected_detail = format!("text run `{}` has {} missing glyph(s)", run.text, run.shaped.missing_glyph_count);
        if expected_detail != violation.detail {
            continue;
        }
        matched = true;
        let has_missing_glyph = run.shaped.glyphs.iter().any(|g| g.missing_glyph);
        let all_missing_are_unbacked_cjk_rtl =
            has_missing_glyph && run.shaped.glyphs.iter().filter(|g| g.missing_glyph).all(|g| is_unbacked_cjk_rtl_char(g.ch, coverage));
        if !all_missing_are_unbacked_cjk_rtl {
            return false;
        }
    }
    matched
}

/// Filter out `text_glyphs` parity violations caused solely by CJK/RTL characters absent from the
/// host font stack, while keeping ASCII/Latin `text_glyphs` failures and all non-`text_glyphs`
/// checks intact.
#[must_use]
pub fn filter_unbacked_cjk_rtl_text_glyphs(tree: &LayoutTree, violations: Vec<ParityViolation>) -> Vec<ParityViolation> {
    filter_unbacked_cjk_rtl_text_glyphs_with_coverage(tree, violations, host_script_coverage())
}

/// Filter `violations` against `tree` using explicit [`HostScriptCoverage`] flags.
#[must_use]
pub fn filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
    tree: &LayoutTree,
    violations: Vec<ParityViolation>,
    coverage: HostScriptCoverage,
) -> Vec<ParityViolation> {
    violations.into_iter().filter(|v| !is_unbacked_cjk_rtl_text_glyph_violation(tree, v, coverage)).collect()
}
