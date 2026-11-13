//! Integration tests for `lumencraft-ui-egui` theming, WCAG 2.1 AA contrast, font feature
//! handling, and 9-locale scientific internationalization & number formatting.

#![allow(clippy::unwrap_used)]

use std::collections::HashSet;

use lumencraft_doc::{CellCoord, SheetSection, SubcolumnMode, TableKind};
use lumencraft_format::{export_datasheet_to_csv, import_csv_to_datasheet};
use lumencraft_math::{AxisScale, PValueFormat};
use lumencraft_palette::Rgba8;
use lumencraft_ui_egui::i18n::{
    LOCALES, LocalePreset, all_keys, catalog_len, locale_display_name, missing_translations, normalize_locale, sheet_section_label, table_kind_label,
    tr, tr_or, try_tr,
};
use lumencraft_ui_egui::theme::{
    UiThemeMode, UiThemeSpec, WCAG_AA_TEXT_MIN_RATIO, WCAG_AA_UI_MIN_RATIO, WCAG_AAA_TEXT_MIN_RATIO, bundled_font_entries, install_fonts,
    wcag_contrast_ratio,
};
use lumencraft_ui_egui::{UiApp, bundled_fonts_enabled};

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

#[test]
fn all_nine_locales_are_complete_and_genuinely_translated() {
    let expected_locales = ["en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW"];
    assert_eq!(LOCALES, &expected_locales);
    assert!(catalog_len() >= 100);
    assert!(missing_translations().is_empty());

    let keys: Vec<&'static str> = all_keys().collect();
    let unique: HashSet<&'static str> = keys.iter().copied().collect();
    assert_eq!(keys.len(), unique.len());

    // Verify required domain coverage keys exist across all 9 locales.
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

    // Confirm non-English locales provide real translated strings rather than English placeholders.
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

    // Tag normalization and unknown-input resilience.
    assert_eq!(normalize_locale("EN_us"), "en");
    assert_eq!(normalize_locale("de-DE"), "de");
    assert_eq!(normalize_locale("pt_br"), "pt-BR");
    assert_eq!(normalize_locale("zh-Hans"), "zh-CN");
    assert_eq!(normalize_locale("zh-Hant-HK"), "zh-TW");
    assert_eq!(normalize_locale("zz-Unknown"), "en");
    assert_eq!(tr("zz-Unknown", "menu.file"), "File");
    assert!(try_tr("en", "no.such.key").is_none());
    assert_eq!(tr("de", "no.such.key"), "");
    assert_eq!(tr_or("de", "custom.literal").as_ref(), "custom.literal");
}

#[test]
fn locale_presets_integrate_number_policy_and_csv_options() {
    for &locale in LOCALES {
        let preset = LocalePreset::for_locale(locale);
        let expect_comma = matches!(locale, "de" | "es" | "fr" | "pt-BR");
        assert_eq!(preset.decimal_separator, if expect_comma { ',' } else { '.' }, "{locale}");
        assert_eq!(preset.csv_delimiter, if expect_comma { ';' } else { ',' }, "{locale}");

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
fn bundled_fonts_and_ui_app_defaults_handle_feature_modes_cleanly() {
    let installed = install_fonts();
    let entries = bundled_font_entries();
    assert_eq!(installed, entries.len());

    if bundled_fonts_enabled() {
        let expected = lumencraft_text::BUNDLED_FONTS.iter().filter(|(name, bytes)| !name.is_empty() && !bytes.is_empty()).count();
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
