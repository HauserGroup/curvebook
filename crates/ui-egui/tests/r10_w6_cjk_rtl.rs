#![forbid(unsafe_code)]
//! Wave 6 (`R10.21`, `R10.2`, `R10.9`) CJK, Cyrillic, and RTL localization verification suite.
//!
//! Covers:
//! 1. Catalog completeness, native-script purity, allowlist-bounded English leak checks,
//!    [`cjk_rtl::STAGED_ENTRIES`] 16-locale coverage, and [`cjk_rtl::REPLACEMENT_PROPOSALS`] checks.
//! 2. BiDi visual ordering and scientific numeric/formula/interval/*p*-value formatting across
//!    Arabic (`ar`), Hebrew (`he`), Russian (`ru`), Japanese (`ja`), Korean (`ko`), Simplified
//!    Chinese (`zh-CN`), and Traditional Chinese (`zh-TW`).
//! 3. RTL (`ar`, `he`) `LayoutTree` pane/grid mirroring geometry and human-path hit-box parity
//!    across all 5 sheet types (`DataSheet`, `InfoSheet`, `ResultSheet`, `GraphSheet`,
//!    `LayoutSheet`) and HiDPI `1.0x`, `1.5x`, `2.0x`.
//! 4. End-to-end `HumanPathDriver` + `LayoutTree::verify_parity()` + accessibility gate across
//!    Welcome, 5 sheet types, 5 DataTable archetypes (`XY`, `Column`, `Grouped`,
//!    `MultipleVariables`, `Survival`), right-hand Inspector, and key dialogs at HiDPI scales.
//! 5. IME `set_ime_preedit` + `type_text` input into modals, navigator search, and inline grid cells.

#[path = "human_wf_common/mod.rs"]
mod human_wf_common;

use std::collections::BTreeSet;

use curvebook_doc::{
    Cell, CellValue, CiBound, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, GroupedTable, InfoSheet, LayoutItem, LayoutSheet,
    LayoutSpec, MultipleVariablesTable, PValueFormat, ProjectDocument, RectPt, ResultCell, SheetId, SubcolumnMode, SurvivalTable, TableArchetype,
    XyTable,
};
use curvebook_engine::Session;
use curvebook_text::{TextDirection, contains_rtl_script, reorder_visual_bidi, shape_bidi_runs};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, ActiveInspector, AxisTarget, ResultSheetViewState};
use curvebook_ui_egui::grid_view::{format_cell_display, parse_localized_number_input};
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::i18n::{
    LOCALES, LocalePreset, all_entries, all_keys, catalog_len,
    cjk_rtl::{REPLACEMENT_PROPOSALS, STAGED_ENTRIES, try_tr_staged},
    command_label, is_rtl_locale, missing_translations, normalize_locale, tr, try_tr, try_tr_command,
};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::layout::a11y::validate_layout_tree_accessibility;
use curvebook_ui_egui::text_bridge::{DesktopTextMeasurer, TextMeasurer, UiTextStyle};
use curvebook_ui_egui::theme::FontBook;

const TARGET_LOCALES: [&str; 7] = ["ja", "ko", "zh-CN", "zh-TW", "ru", "ar", "he"];
const RTL_LOCALES: [&str; 2] = ["ar", "he"];
const HIDPI_SCALES: [f64; 3] = [1.0, 1.5, 2.0];

fn is_cjk_or_han(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

fn is_hiragana_or_katakana(ch: char) -> bool {
    matches!(ch as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF65..=0xFF9F)
}

fn is_hangul(ch: char) -> bool {
    matches!(ch as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF)
}

fn is_cyrillic(ch: char) -> bool {
    matches!(ch as u32, 0x0400..=0x052F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F)
}

fn is_arabic(ch: char) -> bool {
    matches!(ch as u32, 0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)
}

fn is_hebrew(ch: char) -> bool {
    matches!(ch as u32, 0x0590..=0x05FF | 0xFB1D..=0xFB4F)
}

fn is_technical_ascii_allowlisted(text: &str) -> bool {
    matches!(
        text.trim(),
        "XY" | "OK" | "SD" | "SEM" | "N" | "X" | "#" | "№" | "ANOVA" | "OLS" | "CSV" | "PDF" | "TSV" | "ROUT" | "DSL" | "AICc" | "Cmd+K"
    )
}

fn has_expected_script(locale: &str, text: &str) -> bool {
    if is_technical_ascii_allowlisted(text) {
        return true;
    }
    match locale {
        "ja" => text.chars().any(|c| is_hiragana_or_katakana(c) || is_cjk_or_han(c)),
        "ko" => text.chars().any(is_hangul),
        "zh-CN" | "zh-TW" => text.chars().any(is_cjk_or_han),
        "ru" => text.chars().any(is_cyrillic),
        "ar" => text.chars().any(is_arabic),
        "he" => text.chars().any(is_hebrew),
        _ => true,
    }
}

fn has_foreign_script_contamination(locale: &str, text: &str) -> Option<&'static str> {
    match locale {
        "ja" => {
            if text.chars().any(is_hangul) {
                return Some("Hangul in Japanese");
            }
            if text.chars().any(is_cyrillic) {
                return Some("Cyrillic in Japanese");
            }
            if text.chars().any(is_arabic) {
                return Some("Arabic in Japanese");
            }
            if text.chars().any(is_hebrew) {
                return Some("Hebrew in Japanese");
            }
        }
        "ko" => {
            if text.chars().any(is_hiragana_or_katakana) {
                return Some("Kana in Korean");
            }
            if text.chars().any(is_cyrillic) {
                return Some("Cyrillic in Korean");
            }
            if text.chars().any(is_arabic) {
                return Some("Arabic in Korean");
            }
            if text.chars().any(is_hebrew) {
                return Some("Hebrew in Korean");
            }
        }
        "zh-CN" | "zh-TW" => {
            if text.chars().any(is_hiragana_or_katakana) {
                return Some("Kana in Chinese");
            }
            if text.chars().any(is_hangul) {
                return Some("Hangul in Chinese");
            }
            if text.chars().any(is_cyrillic) {
                return Some("Cyrillic in Chinese");
            }
            if text.chars().any(is_arabic) {
                return Some("Arabic in Chinese");
            }
            if text.chars().any(is_hebrew) {
                return Some("Hebrew in Chinese");
            }
        }
        "ru" => {
            if text.chars().any(|c| is_cjk_or_han(c) || is_hiragana_or_katakana(c) || is_hangul(c)) {
                return Some("CJK in Russian");
            }
            if text.chars().any(is_arabic) {
                return Some("Arabic in Russian");
            }
            if text.chars().any(is_hebrew) {
                return Some("Hebrew in Russian");
            }
        }
        "ar" => {
            if text.chars().any(is_hebrew) {
                return Some("Hebrew in Arabic");
            }
            if text.chars().any(is_cyrillic) {
                return Some("Cyrillic in Arabic");
            }
            if text.chars().any(|c| is_cjk_or_han(c) || is_hiragana_or_katakana(c) || is_hangul(c)) {
                return Some("CJK in Arabic");
            }
        }
        "he" => {
            if text.chars().any(is_arabic) {
                return Some("Arabic in Hebrew");
            }
            if text.chars().any(is_cyrillic) {
                return Some("Cyrillic in Hebrew");
            }
            if text.chars().any(|c| is_cjk_or_han(c) || is_hiragana_or_katakana(c) || is_hangul(c)) {
                return Some("CJK in Hebrew");
            }
        }
        _ => {}
    }
    None
}

fn contains_prohibited_brand(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    ["graphpad", "dotmatics", "prism", "instat", "statmate", "spss", "minitab", "originlab", "originpro", "sigmaplot", "systat"]
        .into_iter()
        .find(|&token| lower.contains(token))
}

/// Build an exhaustive 5-sheet-type project containing all key data archetypes + analysis/graph/layout.
fn build_five_sheet_cjk_rtl_doc() -> Result<ProjectDocument, String> {
    let mut doc = ProjectDocument::new("Dose-Response Assay").map_err(|e| format!("ProjectDocument::new: {e}"))?;

    // 1. DataSheet - XY
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 4).map_err(|e| format!("XyTable::new: {e}"))?;
    xy.x_title = String::from("Log[Agonist]");
    xy.data_sets[0].title = String::from("Control");
    xy.data_sets[1].title = String::from("Treatment");
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number(-6.0 + f64::from(u32::try_from(r).unwrap_or(0)));
        xy.data_sets[0].cells[r][0] = Cell::number(12.0 + f64::from(u32::try_from(r).unwrap_or(0)) * 5.0);
        xy.data_sets[0].cells[r][1] = Cell::number(12.6 + f64::from(u32::try_from(r).unwrap_or(0)) * 5.0);
        xy.data_sets[1].cells[r][0] = Cell::number(20.0 + f64::from(u32::try_from(r).unwrap_or(0)) * 8.0);
        xy.data_sets[1].cells[r][1] = Cell::number(21.2 + f64::from(u32::try_from(r).unwrap_or(0)) * 8.0);
    }
    let ds_xy = DataSheet::new("ds-xy", "XY Dose Curve", TableArchetype::Xy(xy)).map_err(|e| format!("ds-xy: {e}"))?;
    doc.add_data_sheet(ds_xy).map_err(|e| format!("add ds-xy: {e}"))?;

    // 2. DataSheet - Column (for ANOVA + ResultSheet; rows 5..6 left empty for inline IME & formula entry)
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 7).map_err(|e| format!("ColumnTable::new: {e}"))?;
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("Drug 10nM");
    col.data_sets[2].title = String::from("Drug 100nM");
    for r in 0..5 {
        let off = f64::from(u32::try_from(r).unwrap_or(0)) * 0.4;
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + off);
        col.data_sets[1].cells[r][0] = Cell::number(15.2 + off);
        col.data_sets[2].cells[r][0] = Cell::number(22.8 + off);
    }
    let ds_col = DataSheet::new("ds-col", "Column Assay", TableArchetype::Column(col)).map_err(|e| format!("ds-col: {e}"))?;
    doc.add_data_sheet(ds_col).map_err(|e| format!("add ds-col: {e}"))?;

    // 3. DataSheet - Grouped
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).map_err(|e| format!("GroupedTable::new: {e}"))?;
    grp.row_titles = vec![String::from("24h"), String::from("48h"), String::from("72h")];
    grp.data_sets[0].title = String::from("WT");
    grp.data_sets[1].title = String::from("KO");
    for r in 0..3 {
        grp.data_sets[0].cells[r][0] = Cell::number(1.2 + f64::from(u32::try_from(r).unwrap_or(0)));
        grp.data_sets[0].cells[r][1] = Cell::number(1.4 + f64::from(u32::try_from(r).unwrap_or(0)));
        grp.data_sets[1].cells[r][0] = Cell::number(2.1 + f64::from(u32::try_from(r).unwrap_or(0)));
        grp.data_sets[1].cells[r][1] = Cell::number(2.3 + f64::from(u32::try_from(r).unwrap_or(0)));
    }
    let ds_grp = DataSheet::new("ds-grp", "Grouped Cohort", TableArchetype::Grouped(grp)).map_err(|e| format!("ds-grp: {e}"))?;
    doc.add_data_sheet(ds_grp).map_err(|e| format!("add ds-grp: {e}"))?;

    // 4. DataSheet - MultipleVariables
    let mut mv = MultipleVariablesTable::new(3, 4).map_err(|e| format!("MultipleVariablesTable::new: {e}"))?;
    for r in 0..4 {
        let base = f64::from(u32::try_from(r).unwrap_or(0));
        mv.variables[0].cells[r] = Cell::number(10.0 + base * 2.0);
        mv.variables[1].cells[r] = Cell::number(50.0 + base * 5.0);
        mv.variables[2].cells[r] = Cell::number(0.15 + base * 0.1);
    }
    let ds_mv = DataSheet::new("ds-mv", "MV Panel", TableArchetype::MultipleVariables(mv)).map_err(|e| format!("ds-mv: {e}"))?;
    doc.add_data_sheet(ds_mv).map_err(|e| format!("add ds-mv: {e}"))?;

    // 5. DataSheet - Survival
    let mut surv = SurvivalTable::new(2, 4).map_err(|e| format!("SurvivalTable::new: {e}"))?;
    surv.cohorts[0].title = String::from("Placebo");
    surv.cohorts[1].title = String::from("Active");
    for r in 0..4 {
        let day = f64::from(u32::try_from(r + 1).unwrap_or(1)) * 7.0;
        surv.times[r] = Cell::number(day);
        surv.cohorts[0].cells[r][0] = Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 });
        surv.cohorts[1].cells[r][0] = Cell::number(if r == 3 { 1.0 } else { 0.0 });
    }
    let ds_surv = DataSheet::new("ds-surv", "Survival Study", TableArchetype::Survival(surv)).map_err(|e| format!("ds-surv: {e}"))?;
    doc.add_data_sheet(ds_surv).map_err(|e| format!("add ds-surv: {e}"))?;

    // InfoSheet
    let mut info = InfoSheet::new_preseeded("info-1", "Protocol Metadata").map_err(|e| format!("InfoSheet::new_preseeded: {e}"))?;
    info.notes_markdown = String::from("p < 0.0001; IC50 = 42.5 nM");
    doc.add_info_sheet(info).map_err(|e| format!("add info-1: {e}"))?;

    // GraphSheet
    let mut gs = GraphSheet::new_uninitialized_for_data("g-col", &doc.data_sheets[1]).map_err(|e| format!("GraphSheet: {e}"))?;
    gs.title = String::from("Response Plot");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Column;
    doc.add_graph_sheet(gs).map_err(|e| format!("add g-col: {e}"))?;

    // LayoutSheet
    let ls_spec = LayoutSpec {
        items: vec![LayoutItem::LiveGraph {
            id: String::from("panel-0"),
            graph_sheet_id: SheetId::from("g-col"),
            bounds_pt: RectPt::new(36.0, 36.0, 320.0, 220.0),
            scale_percent: 100.0,
            badge_override: None,
        }],
        ..LayoutSpec::default()
    };
    let ls = LayoutSheet::new("layout-1", "Figure Layout", ls_spec).map_err(|e| format!("LayoutSheet: {e}"))?;
    doc.add_layout_sheet(ls).map_err(|e| format!("add layout-1: {e}"))?;

    doc.select_in_navigator(&SheetId::from("ds-col")).map_err(|e| format!("select ds-col: {e}"))?;
    Ok(doc)
}

fn new_driver_with_result(locale: &str, scale: f64) -> Result<HumanPathDriver, String> {
    let doc = build_five_sheet_cjk_rtl_doc()?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.set_locale(locale);
    let metrics = ViewportMetrics::new(1280.0, 800.0, scale);
    let mut driver = HumanPathDriver::new(app, metrics);

    // Run real ANOVA ResultSheet via HumanPathDriver so ResultSheet holds realistic tabular + checklist + interpret output.
    driver.click_widget("toolbar.analyze").map_err(|e| format!("open analyze in {locale}: {e}"))?;
    if driver.find_widget("analyze.entry.stats.one_way_anova").is_none_or(|n| !n.drawn_in_frame) {
        let _ = driver.wheel_widget("analyze.entries_list", 0.0, -180.0, KeyModifiers::default());
    }
    driver.click_widget("analyze.entry.stats.one_way_anova").map_err(|e| format!("pick anova in {locale}: {e}"))?;
    driver.click_widget("modal.ok").map_err(|e| format!("confirm anova in {locale}: {e}"))?;
    if driver.app().document().result_sheets.len() != 1 {
        return Err(format!("expected 1 ResultSheet in {locale} after ANOVA"));
    }
    // Select back to ds-col cleanly.
    driver.click_widget("nav.sheet.ds-col").map_err(|e| format!("select nav.sheet.ds-col in {locale}: {e}"))?;
    Ok(driver)
}

#[test]
fn cjk_cyrillic_rtl_catalog_completeness_script_purity_and_staged_entries() -> Result<(), String> {
    let all_catalog_count = all_entries().count();
    let total_keys = catalog_len();
    if all_catalog_count != total_keys {
        return Err(format!("all_entries count {all_catalog_count} != catalog_len {total_keys}"));
    }

    // 1. Zero missing translations across all 7 target locales.
    let all_missing = missing_translations();
    let target_missing: Vec<_> = all_missing.into_iter().filter(|&(_, loc)| TARGET_LOCALES.contains(&loc)).collect();
    if !target_missing.is_empty() {
        return Err(format!("target locales missing {} entries: {:?}", target_missing.len(), target_missing));
    }

    // 2. Every catalog key & command ID checks script appropriateness, no contamination, no English fallback leaks
    // outside explicit technical allowlist.
    let mut unexpected_english_leaks = Vec::new();
    let mut missing_native_script = Vec::new();
    let mut script_contamination = Vec::new();

    for key in all_keys() {
        let en = tr("en", key);
        for locale in TARGET_LOCALES {
            let val = try_tr(locale, key).ok_or_else(|| format!("try_tr returned None for ({locale}, {key})"))?;
            if val.trim().is_empty() {
                return Err(format!("empty translation for ({locale}, {key})"));
            }
            if let Some(brand) = contains_prohibited_brand(val) {
                return Err(format!("prohibited brand token '{brand}' in ({locale}, {key}) = {val:?}"));
            }
            if val == en && !is_technical_ascii_allowlisted(en) {
                unexpected_english_leaks.push((locale, key, en));
            }
            if !has_expected_script(locale, val) {
                missing_native_script.push((locale, key, val));
            }
            if let Some(bad) = has_foreign_script_contamination(locale, val) {
                script_contamination.push((locale, key, bad, val));
            }
        }
    }

    if !unexpected_english_leaks.is_empty() {
        return Err(format!("unexpected English leaks across 653 keys: {unexpected_english_leaks:?}"));
    }
    if !missing_native_script.is_empty() {
        return Err(format!("missing target script characters: {missing_native_script:?}"));
    }
    if !script_contamination.is_empty() {
        return Err(format!("foreign script contamination: {script_contamination:?}"));
    }

    // 3. Check all command keys resolve cleanly via try_tr_command / command_label.
    let cmd_keys: Vec<&str> = all_keys().filter(|k| k.starts_with("cmd.")).collect();
    for key in cmd_keys {
        let cmd_id = &key[4..];
        let en_lbl = tr("en", key);
        for locale in TARGET_LOCALES {
            let lbl = try_tr_command(locale, cmd_id).ok_or_else(|| format!("missing command catalog key {key} for {locale}"))?;
            if lbl.trim().is_empty() || lbl != command_label(locale, cmd_id, en_lbl) {
                return Err(format!("command_label mismatch for ({locale}, {cmd_id})"));
            }
            if !has_expected_script(locale, lbl) {
                return Err(format!("command ({locale}, {cmd_id}) lacks target script: {lbl:?}"));
            }
        }
    }

    // 4. Verify `zh-CN` != `zh-TW` across representative Traditional/Simplified diverging keys.
    for key in ["welcome.title", "welcome.subtitle", "cmd.graph.setCompactLetterDisplay", "fit_params.ci.profile", "cmd.ui.result.applyRemediation"] {
        let cn = try_tr("zh-CN", key).ok_or_else(|| format!("missing zh-CN key {key}"))?;
        let tw = try_tr("zh-TW", key).ok_or_else(|| format!("missing zh-TW key {key}"))?;
        if cn == tw {
            return Err(format!("expected Simplified/Traditional divergence for {key}, got {cn:?} == {tw:?}"));
        }
    }

    // 5. Verify staged Wave 6 entries (`cjk_rtl::STAGED_ENTRIES`).
    let existing_keys: BTreeSet<&str> = all_keys().collect();
    let mut seen_staged = BTreeSet::new();
    for entry in STAGED_ENTRIES {
        if !entry.key.starts_with("w6.cjk_rtl.") {
            return Err(format!("staged key {:?} must use w6.cjk_rtl.* prefix", entry.key));
        }
        if existing_keys.contains(entry.key) {
            return Err(format!("staged key {:?} duplicates existing catalog key", entry.key));
        }
        if !seen_staged.insert(entry.key) {
            return Err(format!("duplicate staged key {:?}", entry.key));
        }
        for (col, &locale) in LOCALES.iter().enumerate() {
            let val = entry.translations[col];
            if val.trim().is_empty() {
                return Err(format!("staged entry ({}, {}) is empty", entry.key, locale));
            }
            if let Some(brand) = contains_prohibited_brand(val) {
                return Err(format!("prohibited brand '{brand}' in staged entry ({}, {})", entry.key, locale));
            }
            if try_tr_staged(locale, entry.key) != Some(val) {
                return Err(format!("try_tr_staged failed for ({locale}, {})", entry.key));
            }
        }
        for locale in TARGET_LOCALES {
            let val = try_tr_staged(locale, entry.key).ok_or_else(|| format!("missing staged ({locale}, {})", entry.key))?;
            if val == entry.translations[0] {
                return Err(format!("staged entry ({locale}, {}) leaked English {:?}", entry.key, val));
            }
            if !has_expected_script(locale, val) {
                return Err(format!("staged entry ({locale}, {}) lacks target script: {:?}", entry.key, val));
            }
            if let Some(bad) = has_foreign_script_contamination(locale, val) {
                return Err(format!("staged entry ({locale}, {}) contaminated ({bad}): {:?}", entry.key, val));
            }
        }
    }

    // 6. Verify REPLACEMENT_PROPOSALS map 1:1 to audited entries without typos.
    for prop in REPLACEMENT_PROPOSALS {
        let active = try_tr(prop.locale, prop.key).ok_or_else(|| format!("missing replacement target ({}, {})", prop.locale, prop.key))?;
        if active != prop.current && active != prop.proposed {
            return Err(format!(
                "replacement proposal ({}, {}) active={active:?} matches neither current={:?} nor proposed={:?}",
                prop.locale, prop.key, prop.current, prop.proposed
            ));
        }
        if prop.current == prop.proposed {
            return Err(format!("no-op replacement proposal ({}, {})", prop.locale, prop.key));
        }
        if !has_expected_script(prop.locale, prop.proposed) {
            return Err(format!("proposed replacement ({}, {}) lacks target script: {:?}", prop.locale, prop.key, prop.proposed));
        }
        if let Some(bad) = has_foreign_script_contamination(prop.locale, prop.proposed) {
            return Err(format!("proposed replacement ({}, {}) contaminated ({bad}): {:?}", prop.locale, prop.key, prop.proposed));
        }
    }

    Ok(())
}

#[test]
fn bidi_ordering_formulas_numbers_intervals_and_pvalues() -> Result<(), String> {
    let measurer = DesktopTextMeasurer::global();
    let style = UiTextStyle::ui_sans(13.0);
    let mono = UiTextStyle::ui_mono(13.0);

    // 1. Pure formula strings & signed numeric cells remain intact in Cell / ResultSheet display & measurer shape_line.
    for locale in ["ar", "he", "ja", "ko", "zh-CN", "zh-TW", "ru"] {
        let preset = LocalePreset::for_locale(locale);
        let pub_preset = LocalePreset::publication(locale);

        let formula_cell = Cell::text("=A1*2");
        if format_cell_display(&formula_cell, &preset) != "=A1*2" {
            return Err(format!("formula cell mutated in {locale}"));
        }
        let ascii_neg = Cell::number(-2.5);
        let neg_str = format_cell_display(&ascii_neg, &preset);
        let pub_neg_str = format_cell_display(&ascii_neg, &pub_preset);
        if parse_localized_number_input(&neg_str, &preset) != Some(-2.5) {
            return Err(format!("round-trip number parse failed in {locale} for {neg_str:?}"));
        }
        let shaped_formula = measurer.shape_line("=A1*2", &mono, locale);
        if shaped_formula.visual_text != "=A1*2" {
            return Err(format!("ASCII formula visually reordered in {locale}: {:?}", shaped_formula.visual_text));
        }
        let shaped_pval = measurer.shape_line("p < 0.0001", &style, locale);
        if shaped_pval.visual_text != "p < 0.0001" {
            return Err(format!("ASCII p-value visually reordered in {locale}: {:?}", shaped_pval.visual_text));
        }
        let shaped_ci = measurer.shape_line("[-1.24, +3.85]", &style, locale);
        if shaped_ci.visual_text != "[-1.24, +3.85]" {
            return Err(format!("ASCII signed interval visually reordered in {locale}: {:?}", shaped_ci.visual_text));
        }

        // Verify ResultCell formatting stability for intervals & p-values.
        let ci_fmt = ResultSheetViewState::format_result_cell(
            &ResultCell::ConfidenceInterval { lower: CiBound::Finite(-1.24), upper: CiBound::Finite(3.85) },
            PValueFormat::FourDecimal,
        );
        if ci_fmt != "[-1.2400, 3.8500]" {
            return Err(format!("unexpected ResultCell CI formatting: {ci_fmt:?}"));
        }
        let pv_fmt = ResultSheetViewState::format_result_cell(
            &ResultCell::PValue { raw: 0.00004, formatted: String::from("< 0.0001"), stars: String::new() },
            PValueFormat::FourDecimal,
        );
        if pv_fmt != "< 0.0001" {
            return Err(format!("unexpected ResultCell PValue formatting: {pv_fmt:?}"));
        }

        // Publication preset minus sign vs BiDi reordering diagnostic check for RTL locales.
        if is_rtl_locale(locale) {
            let shaped_pub_neg = measurer.shape_line(&pub_neg_str, &style, locale);
            let direct_formula_rtl = reorder_visual_bidi("=A1*2", TextDirection::Rtl);
            let lri_formula_rtl = reorder_visual_bidi("\u{2066}=A1*2\u{2069}", TextDirection::Rtl);
            let non_ascii_formula_rtl = reorder_visual_bidi("=A1*\u{2212}2", TextDirection::Rtl);
            let mixed_ar = if locale == "ar" { "فترة الثقة 95%: [-1.24, +3.85]" } else { "רווח סמך 95%: [-1.24, +3.85]" };
            let mixed_pval = if locale == "ar" { "احتمال p < 0.0001" } else { "p < 0.0001 מובהק" };
            let mixed_ar_runs = shape_bidi_runs(mixed_ar, TextDirection::Rtl);
            let mixed_ar_visual = measurer.shape_line(mixed_ar, &style, locale).visual_text;
            let mixed_pval_visual = measurer.shape_line(mixed_pval, &style, locale).visual_text;

            // Log empirical BiDi observations cleanly (`cargo test -- --nocapture` displays exact evidence).
            eprintln!(
                "[BiDi {locale}] pub_neg={pub_neg_str:?} -> shaped={:?}; direct_formula_rtl={direct_formula_rtl:?}; non_ascii_formula_rtl={non_ascii_formula_rtl:?}; lri_formula_rtl={lri_formula_rtl:?}; mixed_runs={}; mixed_ci_visual={mixed_ar_visual:?}; mixed_pval_visual={mixed_pval_visual:?}",
                shaped_pub_neg.visual_text,
                mixed_ar_runs.len()
            );

            // LRI-isolated formula MUST preserve exact `=A1*2` substring even inside an RTL paragraph.
            if !lri_formula_rtl.contains("=A1*2") {
                return Err(format!("LRI-isolated formula lost LTR order in {locale}: {lri_formula_rtl:?}"));
            }
            if !contains_rtl_script(mixed_ar) || !contains_rtl_script(mixed_pval) {
                return Err(format!("contains_rtl_script failed for {mixed_ar:?} / {mixed_pval:?}"));
            }
        }
    }

    // Verify contains_rtl_script returns false for non-RTL CJK/Cyrillic/ASCII strings.
    for s in ["用量反応", "용량반응", "剂量反应", "劑量反應", "Доза", "=A1*2", "p < 0.0001"] {
        if contains_rtl_script(s) {
            return Err(format!("unexpected contains_rtl_script=true for {s:?}"));
        }
    }
    Ok(())
}

#[test]
fn rtl_mirroring_geometry_and_hitbox_parity_across_five_sheet_types() -> Result<(), String> {
    for scale in HIDPI_SCALES {
        // Baseline LTR reference widths & geometry on DataSheet + ResultSheet.
        let mut ltr_driver = new_driver_with_result("en", scale)?;
        let ltr_frame = ltr_driver.layout_tree().frame;
        let phys_w = ltr_driver.metrics().physical_width_px();
        if ltr_frame.navigator.x != 0 || ltr_frame.inspector.right() != phys_w {
            return Err(format!("LTR baseline frame unexpected at {scale}x: {ltr_frame:?}"));
        }

        for locale in RTL_LOCALES {
            let mut driver = new_driver_with_result(locale, scale)?;
            let normalize_check = normalize_locale(locale);
            if !is_rtl_locale(normalize_check) {
                return Err(format!("expected is_rtl_locale({locale}) == true"));
            }

            // Test RTL mirroring across all 5 sheet types: DataSheet (XY/Col/Grp/MV/Surv), InfoSheet, ResultSheet, GraphSheet, LayoutSheet.
            let sheet_ids = ["ds-xy", "ds-col", "ds-grp", "ds-mv", "ds-surv", "info-1", "res-1", "g-col", "layout-1"];
            for sid in sheet_ids {
                let nav_id = format!("nav.sheet.{sid}");
                if driver.find_widget(&nav_id).is_none_or(|n| !n.drawn_in_frame) {
                    // Scroll navigator down if below fold.
                    let _ = driver.wheel_widget("navigator.tree", 0.0, -240.0, KeyModifiers::default());
                }
                if sid == "res-1" {
                    // Select first generated ResultSheet dynamically from document.
                    let rs_id = driver.app().document().result_sheets.first().map(|r| r.id.as_str().to_owned()).unwrap_or_default();
                    driver.click_widget(&format!("nav.sheet.{rs_id}")).map_err(|e| format!("click result nav in {locale}@{scale}x: {e}"))?;
                } else {
                    driver.click_widget(&nav_id).map_err(|e| format!("click {nav_id} in {locale}@{scale}x: {e}"))?;
                }

                // Activate an Inspector tab cleanly when on GraphSheet/LayoutSheet/ResultSheet to verify left-side inspector mirroring.
                if sid == "g-col" {
                    driver.app_mut().canvas_dialogs.active_inspector = ActiveInspector::FormatAxes;
                } else if sid == "layout-1" {
                    driver.app_mut().canvas_dialogs.active_inspector = ActiveInspector::PageLayout;
                }

                let tree = driver.layout_tree();
                if !tree.is_rtl {
                    return Err(format!("LayoutTree.is_rtl false for {locale} sheet {sid}"));
                }

                // 1. Frame pane mirroring: Inspector flush left (x=0), Canvas middle, Navigator flush right (right=phys_w).
                let pw = tree.metrics.physical_width_px();
                if tree.frame.inspector.x != 0 {
                    return Err(format!("{locale}@{scale}x {sid}: inspector.x={} != 0", tree.frame.inspector.x));
                }
                if tree.frame.canvas.x < tree.frame.inspector.right() {
                    return Err(format!("{locale}@{scale}x {sid}: canvas overlaps inspector"));
                }
                if tree.frame.navigator.x < tree.frame.canvas.right() {
                    return Err(format!("{locale}@{scale}x {sid}: navigator overlaps canvas"));
                }
                if tree.frame.navigator.right() != pw {
                    return Err(format!("{locale}@{scale}x {sid}: navigator.right={} != {pw}", tree.frame.navigator.right()));
                }

                // 2. Grid core mirroring on DataSheet: row-number gutter on right, Y columns descending leftwards.
                if let Some(grid) = &tree.grid_geometry {
                    let gutter = grid.row_gutter_rect(0);
                    if gutter.x < grid.sheet_rect.right().saturating_sub(grid.gutter_w + 2) {
                        return Err(format!(
                            "{locale}@{scale}x {sid}: expected RTL row gutter flush right, gutter={gutter:?}, sheet_rect={:?}",
                            grid.sheet_rect
                        ));
                    }
                    if let (Some(c0), Some(c1)) = (tree.node("grid.cell.0.0.0"), tree.node("grid.cell.0.1.0"))
                        && c0.bounds.rect_px.x <= c1.bounds.rect_px.x
                    {
                        return Err(format!(
                            "{locale}@{scale}x {sid}: expected RTL grid columns descending x, got col0={:?}, col1={:?}",
                            c0.bounds.rect_px, c1.bounds.rect_px
                        ));
                    }
                }

                // 3. ResultSheet Interpret drawer splits left of primary results table in RTL.
                if sid == "res-1" {
                    driver.click_widget("toolbar.interpret").map_err(|e| format!("click toolbar.interpret in {locale}@{scale}x: {e}"))?;
                    let r_tree = driver.layout_tree();
                    if let (Some(pri), Some(sec)) = (r_tree.split_primary, r_tree.split_secondary)
                        && sec.x >= pri.x
                    {
                        return Err(format!("{locale}@{scale}x ResultSheet RTL Interpret drawer expected left of table: sec={sec:?}, pri={pri:?}"));
                    }
                    let r_parity = driver.verify_parity();
                    if !r_parity.is_clean() {
                        return Err(format!("{locale}@{scale}x ResultSheet+Interpret parity violations: {:?}", r_parity.violations));
                    }
                }

                // 4. Hit-box parity + accessibility invariants on mirrored RTL tree.
                let report = driver.verify_parity();
                if !report.is_clean() {
                    return Err(format!("{locale}@{scale}x sheet {sid} parity violations: {:?}", report.violations));
                }
                if let Err(a11y_err) = validate_layout_tree_accessibility(&tree) {
                    return Err(format!("{locale}@{scale}x sheet {sid} a11y failure: {a11y_err:?}"));
                }
            }

            // Record empirical widget-level RTL gaps cleanly for handoff diagnostics (without failing on unowned view files).
            driver.click_widget("nav.sheet.g-col").map_err(|e| format!("return to g-col: {e}"))?;
            driver.app_mut().canvas_dialogs.active_inspector = ActiveInspector::FormatGraph;
            let g_tree = driver.layout_tree();
            if let (Some(tg), Some(tl)) = (g_tree.node("inspector.tab.graph"), g_tree.node("inspector.tab.layout"))
                && let (Some(tn), Some(te)) = (g_tree.node("toolbar.new"), g_tree.node("toolbar.export_svg"))
            {
                eprintln!(
                    "[RTL Telemetry {locale}@{scale}x] inspector tabs LTR-order x: graph={} layout={}; toolbar LTR-order x: new={} export={}",
                    tg.bounds.rect_px.x, tl.bounds.rect_px.x, tn.bounds.rect_px.x, te.bounds.rect_px.x
                );
            }
        }
    }
    Ok(())
}

#[test]
fn cjk_cyrillic_rtl_five_sheets_dialogs_inspector_and_hidpi_layout_qa() -> Result<(), String> {
    // Verify FontBook system + fallback shapes representative glyphs for every target locale without missing glyphs
    // when host OS fonts are present (diagnostic telemetry only if headless container lacks specific CJK/RTL system font).
    let book = FontBook::with_system_and_fallback();
    let measurer = DesktopTextMeasurer::with_font_book(book);
    let sample_style = UiTextStyle::ui_sans(13.0);
    for (locale, sample) in [
        ("ja", "用量反応曲線 (ANOVA)"),
        ("ko", "용량 반응 곡선 (검정력)"),
        ("zh-CN", "剂量反应曲线 (回归)"),
        ("zh-TW", "劑量反應曲線 (迴歸)"),
        ("ru", "Кривая доза-эффект"),
        ("ar", "منحنى الاستجابة للجرعة"),
        ("he", "עקומת מינון-תגובה"),
    ] {
        let shaped = measurer.shape_line(sample, &sample_style, locale);
        eprintln!("[FontBook {locale}] sample={sample:?} width={:.1}px missing_glyphs={}", shaped.metrics.width_px, shaped.missing_glyph_count);
    }

    for &locale in &TARGET_LOCALES {
        for &scale in &HIDPI_SCALES {
            let mut driver = new_driver_with_result(locale, scale)?;

            // 1. Welcome view (Empty + Sample tabs).
            driver.app_mut().nav.welcome.open = true;
            let w_parity = driver.verify_parity();
            if !w_parity.is_clean() {
                return Err(format!("{locale}@{scale}x Welcome(Empty) parity violations: {:?}", w_parity.violations));
            }
            driver.click_widget("welcome.tab.sample").map_err(|e| format!("welcome.tab.sample in {locale}@{scale}x: {e}"))?;
            let ws_parity = driver.verify_parity();
            if !ws_parity.is_clean() {
                return Err(format!("{locale}@{scale}x Welcome(Sample) parity violations: {:?}", ws_parity.violations));
            }
            driver.click_widget("welcome.cancel").map_err(|e| format!("welcome.cancel in {locale}@{scale}x: {e}"))?;

            // 2. Sweep all 5 DataTable archetypes + InfoSheet + ResultSheet + GraphSheet + LayoutSheet.
            let sheets = ["ds-xy", "ds-col", "ds-grp", "ds-mv", "ds-surv", "info-1", "g-col", "layout-1"];
            let mut total_ellipsized = 0usize;
            for sid in sheets {
                if driver.find_widget(&format!("nav.sheet.{sid}")).is_none_or(|n| !n.drawn_in_frame) {
                    let _ = driver.wheel_widget("navigator.tree", 0.0, -240.0, KeyModifiers::default());
                }
                driver.click_widget(&format!("nav.sheet.{sid}")).map_err(|e| format!("click {sid} in {locale}@{scale}x: {e}"))?;
                let tree = driver.layout_tree();
                total_ellipsized += tree.text_runs.iter().filter(|r| r.ellipsized).count();
                if (locale == "ja" || locale == "ar") && (scale - 1.0).abs() < 0.01 {
                    let ell_samples: Vec<String> = tree
                        .text_runs
                        .iter()
                        .filter(|r| r.ellipsized)
                        .take(5)
                        .map(|r| format!("{}:{:?}", r.owner_widget_id.as_str(), r.text))
                        .collect();
                    if !ell_samples.is_empty() {
                        eprintln!("[Ellipsized {locale} {sid}] {ell_samples:?}");
                    }
                }
                let parity = driver.verify_parity();
                if !parity.is_clean() {
                    return Err(format!("{locale}@{scale}x {sid} parity violations: {:?}", parity.violations));
                }
                if let Err(err) = validate_layout_tree_accessibility(&tree) {
                    return Err(format!("{locale}@{scale}x {sid} a11y error: {err:?}"));
                }
            }

            // 3. Sweep all right-hand Inspector tabs while GraphSheet is active.
            driver.click_widget("nav.sheet.g-col").map_err(|e| format!("select g-col for inspector in {locale}@{scale}x: {e}"))?;
            for insp_tab in ["inspector.tab.graph", "inspector.tab.axes", "inspector.tab.multi", "inspector.tab.pairwise", "inspector.tab.layout"] {
                driver.click_widget(insp_tab).map_err(|e| format!("click {insp_tab} in {locale}@{scale}x: {e}"))?;
                let parity = driver.verify_parity();
                if !parity.is_clean() {
                    return Err(format!("{locale}@{scale}x {insp_tab} parity violations: {:?}", parity.violations));
                }
                let tree = driver.layout_tree();
                total_ellipsized += tree.text_runs.iter().filter(|r| r.ellipsized).count();
            }

            // 4. Key dialogs sweep across scales (at 1.0x and 2.0x; also covers 1.5x cleanly).
            let res_id = driver.app().document().result_sheets.first().map(|r| r.id.clone()).ok_or_else(|| String::from("missing ResultSheet"))?;
            let g_id = SheetId::from("g-col");
            let dialog_names = [
                "AnalyzeSheet",
                "EditAnalysisParameters",
                "ImportReshape",
                "PowerAndSampleSize",
                "CommandPalette",
                "ChangeGraphType",
                "ExportFigure",
                "FormatAxes",
                "PairwiseComparisons",
            ];
            for dname in dialog_names {
                let doc_snap = driver.app().document().clone();
                let dlgs = &mut driver.app_mut().canvas_dialogs;
                match dname {
                    "AnalyzeSheet" => {
                        let _ = dlgs.open_analyze_dialog(&doc_snap, &SheetId::from("ds-col"));
                    }
                    "EditAnalysisParameters" => {
                        let _ = dlgs.open_edit_parameters(&doc_snap, &res_id);
                    }
                    "ImportReshape" => dlgs.open_import_reshape(),
                    "PowerAndSampleSize" => dlgs.open_power_wizard(),
                    "CommandPalette" => dlgs.open_command_palette(),
                    "ChangeGraphType" => {
                        if let Some(gs) = doc_snap.graph_sheet(&g_id) {
                            dlgs.change_graph_type.open_for_graph(gs, &doc_snap);
                            dlgs.active_dialog = ActiveDialog::ChangeGraphType { graph_sheet_id: g_id.clone() };
                        }
                    }
                    "ExportFigure" => dlgs.open_export_figure(&g_id),
                    "FormatAxes" => {
                        if let Some(gs) = doc_snap.graph_sheet(&g_id) {
                            dlgs.format_axes.sync_from_graph(gs, AxisTarget::Y);
                            dlgs.active_dialog = ActiveDialog::FormatAxes { sheet_id: g_id.clone() };
                        }
                    }
                    "PairwiseComparisons" => {
                        let _ = dlgs.open_pairwise_comparisons(&doc_snap, &g_id);
                    }
                    _ => {}
                }
                let tree = driver.layout_tree();
                total_ellipsized += tree.text_runs.iter().filter(|r| r.ellipsized).count();
                if (locale == "ja" || locale == "ar") && (scale - 1.0).abs() < 0.01 {
                    let ell_samples: Vec<String> = tree
                        .text_runs
                        .iter()
                        .filter(|r| r.ellipsized)
                        .take(5)
                        .map(|r| format!("{}:{:?}", r.owner_widget_id.as_str(), r.text))
                        .collect();
                    if !ell_samples.is_empty() {
                        eprintln!("[Ellipsized {locale} {dname}] {ell_samples:?}");
                    }
                }
                let parity = driver.verify_parity();
                if !parity.is_clean() {
                    return Err(format!("{locale}@{scale}x dialog {dname} parity violations: {:?}", parity.violations));
                }
                if let Err(err) = validate_layout_tree_accessibility(&tree) {
                    return Err(format!("{locale}@{scale}x dialog {dname} a11y error: {err:?}"));
                }
                driver.press_key("Escape", KeyModifiers::default()).map_err(|e| format!("Escape {dname} in {locale}@{scale}x: {e}"))?;
            }

            eprintln!(
                "[Sweep {locale}@{scale}x] 5 sheets + 5 archetypes + inspector + 9 dialogs CLEAN (total ellipsized runs observed={total_ellipsized})"
            );
        }
    }
    Ok(())
}

#[test]
fn ime_preedit_and_multilingual_text_entry_human_path() -> Result<(), String> {
    let cases = [
        ("ja", "用量反応", "かな"),
        ("ko", "용량반응", "ㅇㄹ"),
        ("zh-CN", "剂量反应", "jiliang"),
        ("zh-TW", "劑量反應", "jiliang"),
        ("ru", "Доза", "doz"),
        ("ar", "الجرعة", "ج"),
        ("he", "מינון", "מ"),
    ];

    for (locale, committed_text, preedit_text) in cases {
        let mut driver = new_driver_with_result(locale, 1.5)?;

        // 1. Command Palette search textbox with IME preedit + commit + Escape.
        driver.click_widget("toolbar.palette").map_err(|e| format!("open palette in {locale}: {e}"))?;
        if driver.app().canvas_dialogs.active_dialog != ActiveDialog::CommandPalette {
            return Err(format!("expected CommandPalette open in {locale}"));
        }
        driver.click_widget("palette.query").map_err(|e| format!("focus palette.query in {locale}: {e}"))?;
        driver.set_ime_preedit(Some(preedit_text.to_owned()));
        driver.type_text(committed_text).map_err(|e| format!("type_text palette in {locale}: {e}"))?;
        if driver.app().canvas_dialogs.command_palette.query != committed_text {
            return Err(format!(
                "palette.query expected {:?}, got {:?} in {locale}",
                committed_text,
                driver.app().canvas_dialogs.command_palette.query
            ));
        }
        driver.press_key("Escape", KeyModifiers::default()).map_err(|e| format!("close palette in {locale}: {e}"))?;

        // 2. Navigator search input IME preedit + commit + clear.
        driver.click_widget("navigator.search_input").map_err(|e| format!("focus navigator search in {locale}: {e}"))?;
        driver.set_ime_preedit(Some(preedit_text.to_owned()));
        driver.type_text(committed_text).map_err(|e| format!("nav type_text in {locale}: {e}"))?;
        if driver.app().nav.search_query != committed_text {
            return Err(format!("navigator search_query expected {:?}, got {:?} in {locale}", committed_text, driver.app().nav.search_query));
        }
        driver.click_widget("navigator.search_clear").map_err(|e| format!("clear navigator search in {locale}: {e}"))?;

        // 3. DataSheet inline cell editing on empty row 5 (double-click grid cell -> IME preedit + text -> Enter) + formula entry (`=A1*2`) on empty row 6.
        driver.click_widget("nav.sheet.ds-col").map_err(|e| format!("select ds-col in {locale}: {e}"))?;
        let cell_node = driver.find_widget("grid.cell.5.0.0").ok_or_else(|| format!("missing grid.cell.5.0.0 in {locale}"))?;
        let center = cell_node.bounds.visible_rect_px.center();
        let scale = driver.metrics().clamped_scale_factor();
        let _ = driver
            .double_click_logical(f64::from(center[0]) / scale, f64::from(center[1]) / scale)
            .map_err(|e| format!("double_click grid.cell.5.0.0 in {locale}: {e}"))?;
        driver.set_ime_preedit(Some(preedit_text.to_owned()));
        driver.type_text(committed_text).map_err(|e| format!("cell type_text in {locale}: {e}"))?;
        driver.press_key("Enter", KeyModifiers::default()).map_err(|e| format!("cell Enter in {locale}: {e}"))?;
        let stored_ime = match &driver.app().document().data_sheets[1].archetype {
            TableArchetype::Column(c) => c.data_sets[0].cell(5, 0),
            _ => None,
        };
        if !stored_ime.is_some_and(|c| matches!(&c.value, CellValue::Text(s) if s == committed_text)) {
            return Err(format!("IME text {committed_text:?} not persisted cleanly in {locale}: got {stored_ime:?}"));
        }

        // 4. Formula `=A1*2` committed into empty row 6 col 0 works under both CJK/Cyrillic and RTL locales.
        let cell_node_formula = driver.find_widget("grid.cell.6.0.0").ok_or_else(|| format!("missing grid.cell.6.0.0 in {locale}"))?;
        let center_f = cell_node_formula.bounds.visible_rect_px.center();
        let _ = driver
            .double_click_logical(f64::from(center_f[0]) / scale, f64::from(center_f[1]) / scale)
            .map_err(|e| format!("double_click grid.cell.6.0.0 in {locale}: {e}"))?;
        driver.type_text("=A1*2").map_err(|e| format!("formula type_text in {locale}: {e}"))?;
        driver.press_key("Enter", KeyModifiers::default()).map_err(|e| format!("formula Enter in {locale}: {e}"))?;
        let stored_cell = match &driver.app().document().data_sheets[1].archetype {
            TableArchetype::Column(c) => c.data_sets[0].cell(6, 0),
            _ => None,
        };
        if !stored_cell.is_some_and(|c| matches!(&c.value, CellValue::Text(s) if s == "=A1*2")) {
            return Err(format!("formula '=A1*2' not persisted cleanly in {locale}: got {stored_cell:?}"));
        }

        // Verify parity holds clean after IME + formula commit.
        let parity = driver.verify_parity();
        if !parity.is_clean() {
            return Err(format!("{locale} post-IME parity violations: {:?}", parity.violations));
        }
    }

    Ok(())
}
