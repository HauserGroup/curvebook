//! Integration tests for `curvebook-ui-egui` theming, WCAG 2.1 AA contrast, font feature
//! handling, and 16-locale scientific internationalization, RTL layout helpers, and number formatting.
#![allow(clippy::unwrap_used)]

use std::collections::HashSet;

use curvebook_doc::{CellCoord, SheetSection, SubcolumnMode, TableKind};
use curvebook_format::{export_datasheet_to_csv, import_csv_to_datasheet};
use curvebook_math::{AxisScale, PValueFormat};
use curvebook_palette::Rgba8;
use curvebook_ui_egui::i18n::{
    LOCALES, LocalePreset, all_keys, catalog_len, command_label, is_rtl_locale, locale_display_name, mirror_x_for_rtl, missing_translations,
    normalize_locale, sheet_section_label, table_kind_label, tr, tr_or, translate_command_label, try_tr,
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
        assert_eq!((spec.mode, spec.bg, spec.surface, spec.paper_white), (mode, spec.panel_bg, spec.surface_bg, Rgba8::WHITE));
        for (lbl, fg, bg, min) in [
            ("primary/surface", spec.text_primary, spec.surface, WCAG_AA_TEXT_MIN_RATIO),
            ("primary/bg", spec.text_primary, spec.bg, WCAG_AA_TEXT_MIN_RATIO),
            ("muted/surface", spec.text_muted, spec.surface, WCAG_AA_TEXT_MIN_RATIO),
            ("muted/bg", spec.text_muted, spec.bg, WCAG_AA_TEXT_MIN_RATIO),
            ("accent/surface", spec.accent, spec.surface, WCAG_AA_TEXT_MIN_RATIO),
            ("border/surface", spec.border, spec.surface, WCAG_AA_UI_MIN_RATIO),
            ("border/bg", spec.border, spec.bg, WCAG_AA_UI_MIN_RATIO),
        ] {
            let ratio = wcag_contrast_ratio(fg, bg);
            assert!(ratio >= min, "{mode:?} {lbl}={ratio:.2}");
        }
        assert!(spec.meets_wcag_aa() && spec.meets_wcag_aa_borders());
        let decoded: UiThemeSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(decoded, spec);
    }

    let hc = UiThemeSpec::high_contrast();
    assert!(hc.primary_text_contrast() >= WCAG_AAA_TEXT_MIN_RATIO && hc.muted_text_contrast() >= WCAG_AAA_TEXT_MIN_RATIO);
    assert!(
        hc.primary_text_contrast() > UiThemeSpec::light().primary_text_contrast()
            && hc.primary_text_contrast() > UiThemeSpec::dark().primary_text_contrast()
    );
}

#[test]
fn all_sixteen_locales_are_complete_and_genuinely_translated() {
    let expected = ["en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW", "it", "nl", "pl", "ru", "tr", "ar", "he"];
    assert_eq!((LOCALES, LOCALES.len(), catalog_len(), missing_translations().is_empty()), (&expected[..], 16, 963, true));

    let keys: Vec<&'static str> = all_keys().collect();
    assert_eq!(keys.len(), keys.iter().copied().collect::<HashSet<_>>().len());

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
        for sec in [SheetSection::Data, SheetSection::Info, SheetSection::Analyses, SheetSection::Graphs, SheetSection::Layouts] {
            assert!(!sheet_section_label(locale, sec).trim().is_empty());
        }
    }

    let translated_sample = [
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
        for &key in &translated_sample {
            assert_ne!(tr(locale, key), tr("en", key), "{locale}:{key} stayed English");
        }
    }

    let has_cyr = |s: &str| s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
    let has_ar = |s: &str| s.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
    let has_he = |s: &str| s.chars().any(|c| ('\u{0590}'..='\u{05FF}').contains(&c));
    assert!(has_cyr(locale_display_name("ru")) && has_ar(locale_display_name("ar")) && has_he(locale_display_name("he")));
    for &key in &keys {
        if matches!(key, "table.xy" | "w5.grid_core.n_col" | "w5.grid_core.x_col" | "w5.grid_core.row_number_header") {
            continue;
        }
        assert!(has_cyr(tr("ru", key)), "ru:{key}");
        assert!(has_ar(tr("ar", key)), "ar:{key}");
        assert!(has_he(tr("he", key)), "he:{key}");
    }

    for (raw, norm) in [
        ("EN_us", "en"),
        ("de-DE", "de"),
        ("pt_br", "pt-BR"),
        ("zh-Hans", "zh-CN"),
        ("zh-Hant-HK", "zh-TW"),
        ("it-IT", "it"),
        ("nl_NL", "nl"),
        ("pl-PL", "pl"),
        ("ru_RU", "ru"),
        ("tr-TR", "tr"),
        ("ar-EG", "ar"),
        ("he_IL", "he"),
        ("zz-Unknown", "en"),
    ] {
        assert_eq!(normalize_locale(raw), norm);
    }
    assert_eq!(
        (tr("zz-Unknown", "menu.file"), try_tr("en", "no.such.key"), tr("de", "no.such.key"), tr_or("de", "custom.literal").as_ref()),
        ("File", None, "", "custom.literal")
    );
    assert_eq!(command_label("de", "project.save", ""), "Projekt speichern");
    assert_eq!(command_label("ja", "ui.palette.open", ""), "コマンドパレットを開く");
    assert_eq!(translate_command_label("fr", "About CurveBook"), "À propos de CurveBook");
}

#[test]
fn locale_presets_integrate_number_policy_and_csv_options() {
    for &locale in LOCALES {
        let preset = LocalePreset::for_locale(locale);
        let comma = matches!(locale, "de" | "es" | "fr" | "pt-BR" | "it" | "nl" | "pl" | "ru" | "tr");
        assert_eq!(
            (preset.decimal_separator, preset.csv_delimiter, preset.is_rtl()),
            (if comma { ',' } else { '.' }, if comma { ';' } else { ',' }, matches!(locale, "ar" | "he")),
            "{locale}"
        );
        let csv = preset.csv_options();
        assert_eq!((csv.delimiter, csv.decimal_separator), (preset.csv_delimiter, preset.decimal_separator));
        assert_eq!(preset.format_axis_tick(12.34, AxisScale::Linear), if comma { "12,34" } else { "12.34" });
        assert_eq!(preset.format_pvalue(0.0023, PValueFormat::FourDecimal, true, true), if comma { "P = 0,0023 **" } else { "P = 0.0023 **" });
        assert_eq!(preset.format_pvalue(0.0004, PValueFormat::Apa, false, false), if comma { "< ,001" } else { "< .001" });
        assert_eq!(serde_json::from_str::<LocalePreset>(&serde_json::to_string(&preset).unwrap()).unwrap(), preset);
    }

    assert_eq!(LocalePreset::publication("fr").format_axis_tick(-2.5, AxisScale::Linear), "\u{2212}2,5");
    assert_eq!((LocalePreset::comma_semicolon("en").decimal_separator, LocalePreset::comma_semicolon("en").csv_delimiter), (',', ';'));
    assert_eq!((LocalePreset::period_comma("de").decimal_separator, LocalePreset::period_comma("de").csv_delimiter), ('.', ','));

    let de_opts = LocalePreset::for_locale("de").csv_options();
    let sheet = import_csv_to_datasheet("csv-de", "Dose", "X;Y\n1,5;2,75\n3,0;4,25\n", TableKind::Xy, SubcolumnMode::Single, &de_opts).unwrap();
    assert_eq!(
        (sheet.x_or_time_cell(0, 0).unwrap().active_number(), sheet.cell(CellCoord::new(0, 0, 0)).unwrap().active_number()),
        (Some(1.5), Some(2.75))
    );
    assert_eq!(
        (sheet.x_or_time_cell(1, 0).unwrap().active_number(), sheet.cell(CellCoord::new(1, 0, 0)).unwrap().active_number()),
        (Some(3.0), Some(4.25))
    );
    assert!(export_datasheet_to_csv(&sheet, &de_opts).unwrap().contains("1,5;2,75"));
    assert!(export_datasheet_to_csv(&sheet, &LocalePreset::for_locale("en").csv_options()).unwrap().contains("1.5,2.75"));
}

#[test]
fn rtl_locale_predicates_and_mirroring_work_cleanly() {
    for code in ["ar", "he", "fa", "ur", "ar-EG", "ar_SA", "he-IL", "he_IL", "fa-IR", "ur-PK", " ar-EG "] {
        assert!(is_rtl_locale(code), "{code}");
    }
    for code in ["en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW", "it", "nl", "pl", "ru", "tr", "", "unknown"] {
        assert!(!is_rtl_locale(code), "{code}");
    }
    for (loc, rtl) in [("ar", true), ("ar-EG", true), ("he", true), ("he-IL", true), ("it", false), ("ru", false)] {
        assert_eq!(LocalePreset::for_locale(loc).is_rtl(), rtl);
    }
    assert!(LocalePreset::publication("ar").is_rtl());
    for (x, w, total, rtl, expected) in [
        (24, 120, 800, false, 24),
        (24, 120, 800, true, 656),
        (656, 120, 800, true, 24),
        (0, 800, 800, true, 0),
        (750, 100, 800, true, 0),
        (u32::MAX, 50, 800, true, 0),
    ] {
        assert_eq!(mirror_x_for_rtl(x, w, total, rtl), expected);
    }
}

#[test]
fn bundled_fonts_and_ui_app_defaults_handle_feature_modes_cleanly() {
    let (installed, entries) = (install_fonts(), bundled_font_entries());
    assert_eq!(installed, entries.len());
    if bundled_fonts_enabled() {
        assert_eq!(installed, curvebook_text::BUNDLED_FONTS.iter().filter(|(n, b)| !n.is_empty() && !b.is_empty()).count());
    } else {
        assert_eq!(installed, 0);
    }
    let app = UiApp::new();
    assert_eq!((app.theme.mode, app.theme.paper_white), (UiThemeMode::Light, Rgba8::WHITE));
    assert!(app.theme.meets_wcag_aa() && app.theme.scaled_pt(12.0) > 0.0);
}
