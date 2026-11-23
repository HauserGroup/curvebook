//! Integration tests for `curvebook-ui-egui` theming, WCAG 2.1 AA contrast, font feature
//! handling, and 16-locale scientific internationalization, RTL layout helpers, and number formatting.

#![allow(clippy::unwrap_used)]

use std::collections::HashSet;

use curvebook_doc::{CellCoord, SheetSection, SubcolumnMode, TableKind};
use curvebook_format::{export_datasheet_to_csv, import_csv_to_datasheet};
use curvebook_math::{AxisScale, PValueFormat};
use curvebook_palette::Rgba8;
use curvebook_ui_egui::i18n::{
    LOCALES, LocalePreset, all_keys, catalog_len, is_rtl_locale, locale_display_name, mirror_x_for_rtl, missing_translations, normalize_locale,
    sheet_section_label, table_kind_label, tr, tr_or, try_tr,
};
use curvebook_ui_egui::theme::{
    UiThemeMode, UiThemeSpec, WCAG_AA_TEXT_MIN_RATIO, WCAG_AA_UI_MIN_RATIO, WCAG_AAA_TEXT_MIN_RATIO, bundled_font_entries, install_fonts,
    wcag_contrast_ratio,
};
use curvebook_ui_egui::{UiApp, bundled_fonts_enabled};

#[test]
fn wcag_contrast_meets_aa_for_light_dark_and_high_contrast() {
    assert!((wcag_contrast_ratio(Rgba8::BLACK, Rgba8::WHITE) - 21.0).abs() < 1e-9);
    assert!((wcag_contrast_ratio(Rgba8::rgb(29, 78, 216), Rgba8::WHITE) - wcag_contrast_ratio(Rgba8::WHITE, Rgba8::rgb(29, 78, 216))).abs() < 1e-12);

    for mode in UiThemeMode::ALL {
        let spec = UiThemeSpec::for_mode(mode);
        assert_eq!(spec.mode, mode);
        assert_eq!(spec.bg, spec.panel_bg);
        assert_eq!(spec.surface, spec.surface_bg);
        // Publication canvas stays crisp white even in dark and high-contrast UI chrome.
        assert_eq!(spec.paper_white, Rgba8::WHITE, "{mode:?} must keep paper_white pure white");

        let primary_surface = wcag_contrast_ratio(spec.text_primary, spec.surface);
        let primary_bg = wcag_contrast_ratio(spec.text_primary, spec.bg);
        let muted_surface = wcag_contrast_ratio(spec.text_muted, spec.surface);
        let muted_bg = wcag_contrast_ratio(spec.text_muted, spec.bg);
        let accent_surface = wcag_contrast_ratio(spec.accent, spec.surface);
        let border_surface = wcag_contrast_ratio(spec.border, spec.surface);
        let border_bg = wcag_contrast_ratio(spec.border, spec.bg);

        assert!(primary_surface >= WCAG_AA_TEXT_MIN_RATIO, "{mode:?} primary/surface={primary_surface:.2}");
        assert!(primary_bg >= WCAG_AA_TEXT_MIN_RATIO, "{mode:?} primary/bg={primary_bg:.2}");
        assert!(muted_surface >= WCAG_AA_TEXT_MIN_RATIO, "{mode:?} muted/surface={muted_surface:.2}");
        assert!(muted_bg >= WCAG_AA_TEXT_MIN_RATIO, "{mode:?} muted/bg={muted_bg:.2}");
        assert!(accent_surface >= WCAG_AA_TEXT_MIN_RATIO, "{mode:?} accent/surface={accent_surface:.2}");
        assert!(border_surface >= WCAG_AA_UI_MIN_RATIO, "{mode:?} border/surface={border_surface:.2}");
        assert!(border_bg >= WCAG_AA_UI_MIN_RATIO, "{mode:?} border/bg={border_bg:.2}");
        assert!(spec.meets_wcag_aa());
        assert!(spec.meets_wcag_aa_borders());

        let json = serde_json::to_string(&spec).unwrap();
        let decoded: UiThemeSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, spec);
    }

    let hc = UiThemeSpec::high_contrast();
    assert!(hc.primary_text_contrast() >= WCAG_AAA_TEXT_MIN_RATIO);
    assert!(hc.primary_text_contrast() > UiThemeSpec::light().primary_text_contrast());
    assert!(hc.primary_text_contrast() > UiThemeSpec::dark().primary_text_contrast());
    assert!(hc.muted_text_contrast() >= WCAG_AAA_TEXT_MIN_RATIO);
}

fn verify_required_catalog_coverage() {
    let required_keys = [
        "menu.file",
        "menu.edit",
        "menu.insert",
        "menu.analyze",
        "menu.graph",
        "menu.view",
        "menu.help",
        "nav.data_tables",
        "nav.info",
        "nav.results",
        "nav.graphs",
        "nav.layouts",
        "nav.family",
        "table.xy",
        "table.column",
        "table.grouped",
        "table.contingency",
        "table.survival",
        "table.parts_of_whole",
        "table.multiple_variables",
        "table.nested",
        "analysis.one_way_anova",
        "analysis.nonlinear_regression",
        "analysis.kaplan_meier",
        "analysis.dose_response",
        "btn.ok",
        "btn.cancel",
        "btn.analyze",
        "btn.clone_pipeline",
        "btn.freeze",
        "btn.import",
        "btn.export",
        "btn.change_data_analyzed",
        "btn.apply_style_from",
        "inspector.graph_type",
        "inspector.axis_scale",
        "inspector.bubble_plot",
        "inspector.decimal_separator",
        "inspector.csv_delimiter",
        "theme.light",
        "theme.dark",
        "theme.high_contrast",
    ];

    for &locale in LOCALES {
        assert!(!locale_display_name(locale).is_empty());
        for &key in &required_keys {
            assert!(try_tr(locale, key).is_some(), "missing ({locale}, {key})");
        }
        for kind in TableKind::ALL {
            assert!(!table_kind_label(locale, kind).trim().is_empty());
        }
        for section in [SheetSection::Data, SheetSection::Info, SheetSection::Analyses, SheetSection::Graphs, SheetSection::Layouts] {
            assert!(!sheet_section_label(locale, section).trim().is_empty());
        }
    }
}

fn verify_non_english_translations() {
    let sample_translated_keys = [
        "menu.file",
        "menu.edit",
        "menu.analyze",
        "nav.data_tables",
        "nav.results",
        "table.grouped",
        "table.survival",
        "table.multiple_variables",
        "btn.cancel",
        "btn.clone_pipeline",
        "btn.change_data_analyzed",
        "inspector.decimal_separator",
        "theme.high_contrast",
    ];
    for &locale in &LOCALES[1..] {
        for &key in &sample_translated_keys {
            let en_val = tr("en", key);
            let loc_val = tr(locale, key);
            assert_ne!(loc_val, en_val, "expected {locale} translation for `{key}` to differ from English `{en_val}`");
        }
    }
}

fn verify_non_latin_scripts(keys: &[&'static str]) {
    let has_cyrillic = |s: &str| s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
    let has_arabic = |s: &str| s.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
    let has_hebrew = |s: &str| s.chars().any(|c| ('\u{0590}'..='\u{05FF}').contains(&c));
    assert!(has_cyrillic(locale_display_name("ru")));
    assert!(has_arabic(locale_display_name("ar")));
    assert!(has_hebrew(locale_display_name("he")));
    for &key in keys {
        if matches!(key, "table.xy" | "w5.grid_core.n_col" | "w5.grid_core.x_col" | "w5.grid_core.row_number_header") {
            continue;
        }
        assert!(has_cyrillic(tr("ru", key)), "expected Cyrillic in ru translation for `{key}`");
        assert!(has_arabic(tr("ar", key)), "expected Arabic script in ar translation for `{key}`");
        assert!(has_hebrew(tr("he", key)), "expected Hebrew script in he translation for `{key}`");
    }
}

fn verify_locale_tag_normalization() {
    assert_eq!(normalize_locale("EN_us"), "en");
    assert_eq!(normalize_locale("de-DE"), "de");
    assert_eq!(normalize_locale("pt_br"), "pt-BR");
    assert_eq!(normalize_locale("zh-Hans"), "zh-CN");
    assert_eq!(normalize_locale("zh-Hant-HK"), "zh-TW");
    assert_eq!(normalize_locale("it-IT"), "it");
    assert_eq!(normalize_locale("nl_NL"), "nl");
    assert_eq!(normalize_locale("pl-PL"), "pl");
    assert_eq!(normalize_locale("ru_RU"), "ru");
    assert_eq!(normalize_locale("tr-TR"), "tr");
    assert_eq!(normalize_locale("ar-EG"), "ar");
    assert_eq!(normalize_locale("he_IL"), "he");
    assert_eq!(normalize_locale("zz-Unknown"), "en");
    assert_eq!(tr("zz-Unknown", "menu.file"), "File");
    assert!(try_tr("en", "no.such.key").is_none());
    assert_eq!(tr("de", "no.such.key"), "");
    assert_eq!(tr_or("de", "custom.literal").as_ref(), "custom.literal");
}

#[test]
fn all_sixteen_locales_are_complete_and_genuinely_translated() {
    let expected_locales = ["en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW", "it", "nl", "pl", "ru", "tr", "ar", "he"];
    assert_eq!(LOCALES, &expected_locales);
    assert_eq!(LOCALES.len(), 16);
    assert_eq!(catalog_len(), 704);
    assert!(missing_translations().is_empty());

    let keys: Vec<&'static str> = all_keys().collect();
    let unique: HashSet<&'static str> = keys.iter().copied().collect();
    assert_eq!(keys.len(), unique.len());

    verify_required_catalog_coverage();
    verify_non_english_translations();
    verify_non_latin_scripts(&keys);
    verify_locale_tag_normalization();

    // Command catalog localization parity across all 16 locales.
    assert_eq!(curvebook_ui_egui::i18n::command_label("de", "project.save", ""), "Projekt speichern");
    assert_eq!(curvebook_ui_egui::i18n::command_label("ja", "ui.palette.open", ""), "コマンドパレットを開く");
    assert_eq!(curvebook_ui_egui::i18n::translate_command_label("fr", "About CurveBook"), "À propos de CurveBook");
}

#[test]
fn locale_presets_integrate_number_policy_and_csv_options() {
    for &locale in LOCALES {
        let preset = LocalePreset::for_locale(locale);
        let expect_comma = matches!(locale, "de" | "es" | "fr" | "pt-BR" | "it" | "nl" | "pl" | "ru" | "tr");
        assert_eq!(preset.decimal_separator, if expect_comma { ',' } else { '.' }, "{locale}");
        assert_eq!(preset.csv_delimiter, if expect_comma { ';' } else { ',' }, "{locale}");
        assert_eq!(preset.is_rtl(), matches!(locale, "ar" | "he"), "{locale}");

        let csv = preset.csv_options();
        assert_eq!(csv.delimiter, preset.csv_delimiter);
        assert_eq!(csv.decimal_separator, preset.decimal_separator);

        let tick = preset.format_axis_tick(12.34, AxisScale::Linear);
        assert_eq!(tick, if expect_comma { "12,34" } else { "12.34" });

        let pval = preset.format_pvalue(0.0023, PValueFormat::FourDecimal, true, true);
        assert_eq!(pval, if expect_comma { "P = 0,0023 **" } else { "P = 0.0023 **" });

        let apa = preset.format_pvalue(0.0004, PValueFormat::Apa, false, false);
        assert_eq!(apa, if expect_comma { "< ,001" } else { "< .001" });

        let json = serde_json::to_string(&preset).unwrap();
        let decoded: LocalePreset = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, preset);
    }

    let pub_fr = LocalePreset::publication("fr");
    assert_eq!(pub_fr.format_axis_tick(-2.5, AxisScale::Linear), "\u{2212}2,5");

    let explicit_eu = LocalePreset::comma_semicolon("en");
    assert_eq!(explicit_eu.decimal_separator, ',');
    assert_eq!(explicit_eu.csv_delimiter, ';');

    let explicit_us = LocalePreset::period_comma("de");
    assert_eq!(explicit_us.decimal_separator, '.');
    assert_eq!(explicit_us.csv_delimiter, ',');

    // End-to-end semicolon + comma-decimal CSV import/export using LocalePreset::for_locale("de").
    let de_csv_opts = LocalePreset::for_locale("de").csv_options();
    let de_input = "X;Y\n1,5;2,75\n3,0;4,25\n";
    let sheet = import_csv_to_datasheet("csv-de", "Dose", de_input, TableKind::Xy, SubcolumnMode::Single, &de_csv_opts).unwrap();
    assert_eq!(sheet.x_or_time_cell(0, 0).unwrap().active_number(), Some(1.5));
    assert_eq!(sheet.cell(CellCoord::new(0, 0, 0)).unwrap().active_number(), Some(2.75));
    assert_eq!(sheet.x_or_time_cell(1, 0).unwrap().active_number(), Some(3.0));
    assert_eq!(sheet.cell(CellCoord::new(1, 0, 0)).unwrap().active_number(), Some(4.25));
    let exported_de = export_datasheet_to_csv(&sheet, &de_csv_opts).unwrap();
    assert!(exported_de.contains("1,5;2,75"), "{exported_de}");

    let en_csv_opts = LocalePreset::for_locale("en").csv_options();
    let exported_en = export_datasheet_to_csv(&sheet, &en_csv_opts).unwrap();
    assert!(exported_en.contains("1.5,2.75"), "{exported_en}");
}

#[test]
fn rtl_locale_predicates_and_mirroring_work_cleanly() {
    for rtl_code in ["ar", "he", "fa", "ur", "ar-EG", "ar_SA", "he-IL", "he_IL", "fa-IR", "ur-PK", " ar-EG "] {
        assert!(is_rtl_locale(rtl_code), "expected RTL predicate true for `{rtl_code}`");
    }
    for ltr_code in ["en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW", "it", "nl", "pl", "ru", "tr", "", "unknown"] {
        assert!(!is_rtl_locale(ltr_code), "expected RTL predicate false for `{ltr_code}`");
    }

    assert!(LocalePreset::for_locale("ar").is_rtl());
    assert!(LocalePreset::for_locale("ar-EG").is_rtl());
    assert!(LocalePreset::for_locale("he").is_rtl());
    assert!(LocalePreset::for_locale("he-IL").is_rtl());
    assert!(LocalePreset::publication("ar").is_rtl());
    assert!(!LocalePreset::for_locale("it").is_rtl());
    assert!(!LocalePreset::for_locale("ru").is_rtl());

    assert_eq!(mirror_x_for_rtl(24, 120, 800, false), 24);
    assert_eq!(mirror_x_for_rtl(24, 120, 800, true), 656);
    assert_eq!(mirror_x_for_rtl(656, 120, 800, true), 24);
    assert_eq!(mirror_x_for_rtl(0, 800, 800, true), 0);
    assert_eq!(mirror_x_for_rtl(750, 100, 800, true), 0);
    assert_eq!(mirror_x_for_rtl(u32::MAX, 50, 800, true), 0);
}

#[test]
fn bundled_fonts_and_ui_app_defaults_handle_feature_modes_cleanly() {
    let installed = install_fonts();
    let entries = bundled_font_entries();
    assert_eq!(installed, entries.len());

    if bundled_fonts_enabled() {
        let expected = curvebook_text::BUNDLED_FONTS.iter().filter(|(name, bytes)| !name.is_empty() && !bytes.is_empty()).count();
        assert_eq!(installed, expected);
    } else {
        assert_eq!(installed, 0);
        assert!(entries.is_empty());
    }

    let app = UiApp::new();
    assert_eq!(app.theme.mode, UiThemeMode::Light);
    assert_eq!(app.theme.paper_white, Rgba8::WHITE);
    assert!(app.theme.meets_wcag_aa());
    assert!(app.theme.scaled_pt(12.0) > 0.0);
}
