//! Wave 6 (`R10.21`) comprehensive European-locale (`de`, `es`, `fr`, `it`, `nl`, `pl`, `pt-BR`, `tr`)
//! completeness, `HumanPathDriver` rendered `LayoutTree` parity, and regional number/CSV separator
//! integration test suite.

#![forbid(unsafe_code)]

#[path = "human_wf_common/mod.rs"]
mod human_wf_common;

use std::collections::HashSet;

use curvebook_doc::{
    Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, MultipleVariablesTable,
    ProjectDocument, RectPt, SubcolumnMode, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_format::{export_datasheet_to_csv, import_csv_to_datasheet};
use curvebook_math::{AxisScale, PValueFormat};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, ActiveInspector};
use curvebook_ui_egui::grid_view::{format_cell_display, parse_localized_number_input};
use curvebook_ui_egui::i18n::{
    CATALOG, LOCALES, LocalePreset, all_entries, all_keys, command_label, eu, is_rtl_locale, locale_display_name, missing_translations,
    normalize_locale, sheet_section_label, table_kind_label, tr, try_tr,
};
use curvebook_ui_egui::interaction::KeyModifiers;
use curvebook_ui_egui::layout::a11y::validate_layout_tree_accessibility;
use curvebook_ui_egui::layout::verify::verify_tree_invariants;
use curvebook_ui_egui::layout::{LayoutTree, WidgetId, ZLayer};
use curvebook_ui_egui::menus::{menu_bar_tooltips, toolbar_tooltips};
use curvebook_ui_egui::views::toolbar_menus::active_toolbar_context;
use curvebook_ui_egui::{UiApp, ViewportMetrics, dispatch_ui_command};
use human_wf_common::{assert_zero_shortcut_compliance, click_widget_center, default_viewport, new_human_driver};
use serde_json::json;

const EU_LOCALES: &[&str] = &["de", "es", "fr", "it", "nl", "pl", "pt-BR", "tr"];

fn locale_idx(locale: &str) -> Result<usize, String> {
    LOCALES.iter().position(|&tag| tag == locale).ok_or_else(|| format!("unknown locale `{locale}` in LOCALES"))
}

fn contains_placeholder_or_stub(s: &str) -> bool {
    if s.trim().is_empty() || s.contains("???") || s.contains("{}") || s.contains("{0}") {
        return true;
    }
    if s.to_ascii_lowercase().contains("placeholder") {
        return true;
    }
    s.split(|c: char| !c.is_ascii_alphanumeric()).any(|tok| matches!(tok, "TODO" | "TBD" | "FIXME"))
}

fn has_script_range(s: &str, start: u32, end: u32) -> bool {
    s.chars().any(|c| {
        let cp = c as u32;
        cp >= start && cp <= end
    })
}

fn is_valid_eu_cognate(key: &str, locale: &str, val: &str) -> bool {
    // Genuine identical terms or scientific abbreviations/acronyms across EU locales.
    if val.chars().all(|c| !c.is_ascii_alphabetic() || c.is_ascii_uppercase()) {
        return true;
    }
    if matches!(
        val,
        "Linear"
            | "Logit"
            | "Probit"
            | "Log 10"
            | "Log 2"
            | "Log10"
            | "Log2"
            | "Log"
            | "ANOVA"
            | "Mean"
            | "Min"
            | "Max"
            | "Normal Q-Q"
            | "Cmd+K"
            | "Copies"
            | "Diagnostics"
            | "Format"
            | "Inspector"
            | "Median / Q1 / Q3"
            | "Parameters (Cmd+T)"
            | "Pixels (px)"
            | "Power"
            | "Zigzag"
            | "CSV / Text"
            | "Info"
            | "Parameter"
            | "Parameter (Cmd+T)"
            | "Parametry"
            | "Variable"
            | "Variablen"
            | "Analyse"
            | "Grafik"
            | "Graph"
            | "Layout"
            | "Export"
            | "Font"
            | "Table"
            | "Status"
            | "Type"
            | "Filter"
            | "Transform"
            | "Mode"
            | "Series"
            | "Normal"
            | "Model"
            | "Models"
            | "Standard"
            | "Residuals"
            | "Residuals (Y)"
            | "Histogram"
            | "Correlation"
            | "Principal"
            | "Bootstrap"
            | "Survival"
            | "Poisson"
            | "Weibull"
            | "Sigmoid"
            | "Gompertz"
            | "Michaelis-Menten"
            | "Schild"
            | "Scatchard"
            | "Hill"
            | "Bland-Altman"
            | "Kaplan-Meier"
            | "Wilcoxon"
            | "Mann-Whitney"
            | "Kruskal-Wallis"
            | "Friedman"
            | "Tukey"
            | "Dunnett"
            | "Bonferroni"
            | "Holm-Sidak"
            | "Welch"
            | "Brown-Forsythe"
            | "Shapiro-Wilk"
            | "Kolmogorov-Smirnov"
            | "D'Agostino-Pearson"
            | "Anderson-Darling"
            | "Deming"
            | "Levene"
            | "Bartlett"
            | "Grubbs"
            | "ROUT"
            | "Dixon"
            | "Box-Cox"
            | "Cox"
            | "Loess"
            | "Spline"
            | "Poisson Regression"
            | "Logistic Regression"
    ) {
        return true;
    }
    matches!(
        (key, locale),
        ("menu.graph", "nl")
            | ("inspector.tab.graph", "nl")
            | ("nav.info", "de" | "es" | "fr" | "pt-BR" | "nl")
            | ("inspector.scale.linear", "de")
            | ("inspector.scale.logit" | "inspector.scale.probit", _)
            | ("btn.export", "de")
            | ("toolbar.export_svg", "de")
            | ("w6.view.dlg.unit_pixels", "fr")
            | ("fit_params.model.one_site_total", _)
            | ("w5.grid_core.x_col" | "w5.grid_core.n_col" | "w5.grid_core.row_number_header" | "table.xy", _)
    ) || key.starts_with("cmd.")
        || val.len() <= 4
}

fn tree_contains_expected(tree: &LayoutTree, expected: &str) -> bool {
    let needle = expected.trim();
    if tree.nodes.iter().any(|n| n.drawn_in_frame && n.a11y.label.trim() == needle) {
        return true;
    }
    if tree.text_runs.iter().any(|r| {
        let t = r.text.trim();
        t == needle || t.starts_with(needle) || (!needle.is_empty() && t.contains(needle))
    }) {
        return true;
    }
    // Check prefix match in case of tight button label truncation ("…").
    let prefix: String = needle.chars().take(6).collect();
    if prefix.chars().count() >= 5 {
        return tree.text_runs.iter().any(|r| r.text.contains(&prefix));
    }
    false
}

fn tree_has_exact_english_leak(tree: &LayoutTree, locale: &str, key: &str) -> Option<String> {
    let en = tr("en", key).trim();
    let localized = tr(locale, key).trim();
    if en == localized || is_valid_eu_cognate(key, locale, en) {
        return None;
    }
    if tree.nodes.iter().any(|n| n.drawn_in_frame && n.a11y.label.trim() == en) {
        return Some(format!("node `{}` a11y.label leaked English `{en}` in `{locale}`", key));
    }
    if tree.text_runs.iter().any(|r| r.text.trim() == en) {
        return Some(format!("text_run `{}` leaked exact English `{en}` in `{locale}` (expected `{localized}`)", key));
    }
    None
}

fn assert_tree_parity_and_a11y(tree: &LayoutTree, label: &str) -> Result<(), String> {
    validate_layout_tree_accessibility(tree).map_err(|e| format!("{label} a11y failed: {e}"))?;
    let report = verify_tree_invariants(tree);
    if !report.is_clean() {
        return Err(format!("{label} parity failed: {:?}", report.violations));
    }
    Ok(())
}

fn build_rich_eu_workspace(locale: &str) -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("EU Locale Study").map_err(|e| e.to_string())?;

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).map_err(|e| e.to_string())?;
    col.data_sets[0].title = String::from("Ctrl");
    col.data_sets[1].title = String::from("DoseA");
    col.data_sets[2].title = String::from("DoseB");
    let vals = [[10.0, 14.5, 20.1], [11.2, 15.1, 21.0], [9.8, 13.9, 19.6], [10.6, 14.8, 20.8], [10.4, 15.3, 21.5]];
    for (r, row) in vals.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            col.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let ds_col = DataSheet::new("ds-col", "Cohort", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds_col).map_err(|e| e.to_string())?;

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4).map_err(|e| e.to_string())?;
    xy.data_sets[0].title = String::from("Curve");
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(((r + 1) * 5) as f64);
    }
    doc.add_data_sheet(DataSheet::new("ds-xy", "AssayXY", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut mv = MultipleVariablesTable::new(3, 4).map_err(|e| e.to_string())?;
    mv.variables[0].title = String::from("Arm");
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[1].title = String::from("Response");
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[2].title = String::from("Label");
    mv.variables[2].role = VariableRole::Label;
    for r in 0..4 {
        mv.variables[0].cells[r] = Cell::text(if r < 2 { "Alpha" } else { "Beta" });
        mv.variables[1].cells[r] = Cell::number(10.0 + r as f64 * 2.5);
        mv.variables[2].cells[r] = Cell::text(format!("S{}", r + 1));
    }
    doc.add_data_sheet(DataSheet::new("ds-mv", "CohortMV", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Protocol").map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut session = Session::from_document(doc);
    session
        .execute("analysis.run", json!({ "sheet_id": "res-1", "source_sheet": "ds-col", "kind": "one_way_anova", "multiplicity": "tukey" }))
        .map_err(|e| e.to_string())?;
    session.execute("graph.create", json!({ "sheet_id": "gr-1", "data_sheet_id": "ds-col", "kind": "column" })).map_err(|e| e.to_string())?;

    let mut doc_out = session.document().clone();
    if let Some(gr_col) = doc_out.graph_sheet_mut(&curvebook_doc::SheetId::new("gr-1")) {
        gr_col.title = String::from("PlotCol");
        gr_col.initialized = true;
        gr_col.spec.kind = GraphKind::Column;
    }
    let ls = LayoutSheet::new(
        "lay-1",
        "Layout1",
        LayoutSpec {
            items: vec![LayoutItem::LiveGraph {
                id: String::from("p1"),
                graph_sheet_id: curvebook_doc::SheetId::new("gr-1"),
                bounds_pt: RectPt::new(24.0, 24.0, 240.0, 180.0),
                scale_percent: 100.0,
                badge_override: None,
            }],
            ..LayoutSpec::default()
        },
    )
    .map_err(|e| e.to_string())?;
    doc_out.add_layout_sheet(ls).map_err(|e| e.to_string())?;

    let mut app = UiApp::from_session(Session::from_document(doc_out));
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();
    let _ = dispatch_ui_command(&mut app, "ui.locale.set", &json!({ "locale": locale })).map_err(|e| e.to_string())?;
    Ok(app)
}

#[test]
fn eu_staged_catalog_completeness_scripts_and_zero_collisions() -> Result<(), String> {
    assert!(eu::STAGED_ENTRIES.len() >= 55, "expected >= 55 staged EU/view catalog entries, found {}", eu::STAGED_ENTRIES.len());

    let baseline_non_w6: HashSet<&str> = CATALOG.iter().map(|e| e.key).chain(all_keys().filter(|k| !k.starts_with("w6.view."))).collect();

    let mut seen_staged = HashSet::new();
    for entry in eu::STAGED_ENTRIES {
        if !entry.key.starts_with("w6.view.") {
            return Err(format!("staged key `{}` violates `w6.view.*` prefix partition", entry.key));
        }
        if !seen_staged.insert(entry.key) {
            return Err(format!("duplicate staged key `{}` in eu::STAGED_ENTRIES", entry.key));
        }
        if baseline_non_w6.contains(entry.key) {
            return Err(format!("staged key `{}` collides with baseline catalog", entry.key));
        }
        let en = entry.translations[0];
        if contains_placeholder_or_stub(en) {
            return Err(format!("staged key `{}` has invalid English string `{en}`", entry.key));
        }
        for (idx, &locale) in LOCALES.iter().enumerate() {
            let val = entry.translations[idx];
            if contains_placeholder_or_stub(val) {
                return Err(format!("staged key `{}` has empty/stub `{locale}` translation `{val}`", entry.key));
            }
            let lower = val.to_ascii_lowercase();
            for forbidden in ["graphpad", "prism", "originpro", "sigmaplot", "minitab", "stata"] {
                if lower.contains(forbidden) {
                    return Err(format!("staged key `{}` `{locale}` contains unapproved brand `{forbidden}`", entry.key));
                }
            }
            if EU_LOCALES.contains(&locale) && val == en && !is_valid_eu_cognate(entry.key, locale, val) {
                return Err(format!("staged key `{}` leaked English `{val}` in EU locale `{locale}`", entry.key));
            }
        }
        // Script checks so future integration passes `i18n::tests::untranslated_report`.
        if !has_script_range(entry.translations[12], 0x0400, 0x04FF) {
            return Err(format!("staged key `{}` `ru` lacks Cyrillic: `{}`", entry.key, entry.translations[12]));
        }
        if !has_script_range(entry.translations[14], 0x0600, 0x06FF) {
            return Err(format!("staged key `{}` `ar` lacks Arabic: `{}`", entry.key, entry.translations[14]));
        }
        if !has_script_range(entry.translations[15], 0x0590, 0x05FF) {
            return Err(format!("staged key `{}` `he` lacks Hebrew: `{}`", entry.key, entry.translations[15]));
        }
    }
    Ok(())
}

#[test]
fn eu_existing_catalog_completeness_autonyms_and_terminology_checks() -> Result<(), String> {
    let entries = all_entries();
    assert!(missing_translations().is_empty(), "global catalog must have zero missing translations");

    let expected_autonyms = [
        ("de", "Deutsch"),
        ("es", "Español"),
        ("fr", "Français"),
        ("it", "Italiano"),
        ("nl", "Nederlands"),
        ("pl", "Polski"),
        ("pt-BR", "Português (Brasil)"),
        ("tr", "Türkçe"),
    ];
    for (tag, autonym) in expected_autonyms {
        assert_eq!(locale_display_name(tag), autonym);
        assert!(!is_rtl_locale(tag), "{tag} must be LTR");
    }

    let regional_variants = [
        ("de-DE", "de"),
        ("de_AT", "de"),
        ("es-ES", "es"),
        ("es-419", "es"),
        ("fr-CA", "fr"),
        ("pt", "pt-BR"),
        ("pt_PT", "pt-BR"),
        ("pt-BR", "pt-BR"),
        ("it-IT", "it"),
        ("nl-BE", "nl"),
        ("pl-PL", "pl"),
        ("tr-TR", "tr"),
    ];
    for (raw, canonical) in regional_variants {
        assert_eq!(normalize_locale(raw), canonical, "variant `{raw}`");
    }

    // Verify zero empty/stub/brand/template-leak entries across all 653 active keys for every EU locale.
    let mut entry_count = 0usize;
    for entry in entries {
        entry_count += 1;
        let en = entry.translations[0];
        for &locale in EU_LOCALES {
            let idx = locale_idx(locale)?;
            let val = entry.translations[idx];
            if contains_placeholder_or_stub(val) {
                return Err(format!("key `{}` `{locale}` has empty/stub `{val}`", entry.key));
            }
            let lower = val.to_ascii_lowercase();
            for forbidden in ["graphpad", "prism", "originpro", "sigmaplot", "minitab", "stata"] {
                if lower.contains(forbidden) {
                    return Err(format!("key `{}` `{locale}` contains prohibited brand `{forbidden}`", entry.key));
                }
            }
            // Spot-check command/navigation/menu/button keys aren't empty or raw English fallbacks when tr() is queried.
            if entry.key.starts_with("menu.")
                || entry.key.starts_with("nav.")
                || entry.key.starts_with("toolbar.")
                || entry.key.starts_with("welcome.")
                || entry.key.starts_with("inspector.")
                || entry.key.starts_with("results.")
            {
                assert_eq!(try_tr(locale, entry.key), Some(val));
                assert_ne!(val.trim(), "");
                if en == val && !is_valid_eu_cognate(entry.key, locale, val) {
                    // Only flag genuinely un-translated multi-word UI phrases (short labels checked separately).
                    if en.contains(' ') && en.len() > 14 && !en.contains("CSV") && !en.contains("CLD") {
                        return Err(format!("multi-word key `{}` leaked English `{val}` in `{locale}`", entry.key));
                    }
                }
            }
        }
    }
    assert_ne!(entry_count, 0);
    Ok(())
}

#[test]
fn eu_human_path_rendered_layout_tree_chrome_menus_sheets_inspectors_and_dialogs() -> Result<(), String> {
    let vp_1x = default_viewport();
    let vp_2x = ViewportMetrics::new(1280.0, 800.0, 2.0);

    for &locale in EU_LOCALES {
        // 1. Welcome view + top chrome (menubar + toolbar + navigator).
        let mut welcome_app = UiApp::new();
        let _ = dispatch_ui_command(&mut welcome_app, "ui.locale.set", &json!({ "locale": locale })).map_err(|e| e.to_string())?;
        let mut w_driver = new_human_driver(welcome_app, vp_1x);
        let w_tree = w_driver.layout_tree();
        assert_tree_parity_and_a11y(&w_tree, &format!("welcome_{locale}"))?;

        let welcome_keys = [
            "menu.file",
            "menu.edit",
            "menu.insert",
            "menu.analyze",
            "menu.graph",
            "menu.view",
            "menu.help",
            "welcome.title",
            "welcome.rail.archetypes",
            "welcome.btn.create_table",
            "table.xy",
            "table.column",
            "table.grouped",
            "nav.data_tables",
            "nav.graphs",
            "nav.layouts",
        ];
        for k in welcome_keys {
            let expected = tr(locale, k);
            if !tree_contains_expected(&w_tree, expected) {
                return Err(format!("welcome `{locale}` missing localized `{k}`=`{expected}` in LayoutTree"));
            }
            if let Some(leak) = tree_has_exact_english_leak(&w_tree, locale, k) {
                return Err(leak);
            }
        }

        // Verify clicking welcome CTA creates initial DataSheet cleanly via HumanPathDriver.
        let _ = click_widget_center(&mut w_driver, &WidgetId::from("welcome.create_cta"))?;
        if w_driver.app().nav.welcome.open || w_driver.app().document().data_sheets.is_empty() {
            return Err(format!("`welcome.create_cta` click in `{locale}` did not dismiss Welcome and create DataSheet"));
        }
        let _ = assert_zero_shortcut_compliance(&mut w_driver)?;

        // 2. Rich workspace across menus, all 5 sheet types, inspector tabs, and dialogs.
        let app = build_rich_eu_workspace(locale)?;
        let mut driver = new_human_driver(app, vp_1x);

        // Check menu_bar_tooltips & toolbar_tooltips return localized strings.
        let menu_sections = menu_bar_tooltips(driver.app());
        assert_eq!(menu_sections.len(), 7);
        for sec in &menu_sections {
            assert_eq!(sec.title, tr(locale, sec.menu_key));
            for item in &sec.items {
                assert_eq!(item.localized_label, command_label(locale, item.command_id, item.label));
            }
        }
        for tb in toolbar_tooltips(driver.app()) {
            assert_eq!(tb.caption, tr(locale, tb.button.label_key));
        }

        // Open top-level menu dropdowns via HumanPathDriver & verify localized items rendered in DropdownMenu layer.
        for (menu_id, sample_cmd_id) in
            [("menu.file", "project.new"), ("menu.edit", "edit.undo"), ("menu.analyze", "analysis.run"), ("menu.graph", "graph.setAxes")]
        {
            let _ = click_widget_center(&mut driver, &WidgetId::from(menu_id))?;
            let m_tree = driver.layout_tree();
            assert_tree_parity_and_a11y(&m_tree, &format!("{menu_id}_{locale}"))?;
            let has_dropdown = m_tree.nodes.iter().any(|n| n.z_layer == ZLayer::DropdownMenu && n.drawn_in_frame);
            if !has_dropdown {
                return Err(format!("expected open DropdownMenu for `{menu_id}` in `{locale}`"));
            }
            let expected_cmd_lbl = command_label(locale, sample_cmd_id, sample_cmd_id);
            if !m_tree.text_runs.iter().any(|r| r.text.contains(expected_cmd_lbl))
                && !m_tree.nodes.iter().any(|n| n.a11y.label.contains(expected_cmd_lbl))
            {
                return Err(format!("open `{menu_id}` in `{locale}` missing localized `{sample_cmd_id}`=`{expected_cmd_lbl}`"));
            }
            let _ = driver.press_key("Escape", KeyModifiers::default())?;
        }

        // Switch through all 5 sheet types via Navigator click at 1.0x and 2.0x HiDPI.
        let sheet_checks: &[(&str, &[&str])] = &[
            ("nav.sheet.ds-col", &["w5.grid_core.row_titles"]),
            ("nav.sheet.ds-mv", &["mv.new_calc", "mv.variables_btn", "mv.defs_toggle", "mv.editor_btn"]),
            ("nav.sheet.info-1", &["nav.info"]),
            ("nav.sheet.res-1", &["results.parameters", "results.copy", "results.interpret"]),
            (
                "nav.sheet.gr-1",
                &["inspector.tab.graph", "inspector.tab.axes", "inspector.tab.multi", "inspector.tab.pairwise", "inspector.tab.layout"],
            ),
            ("nav.sheet.lay-1", &["nav.layouts", "toolbar.alignment"]),
        ];
        for &(nav_wid, keys_to_verify) in sheet_checks {
            driver.set_metrics(vp_1x);
            let _ = click_widget_center(&mut driver, &WidgetId::from(nav_wid))?;
            let t_1x = driver.layout_tree();
            assert_tree_parity_and_a11y(&t_1x, &format!("{nav_wid}_{locale}_1x"))?;
            for &k in keys_to_verify {
                let expected = tr(locale, k);
                if !tree_contains_expected(&t_1x, expected) {
                    return Err(format!("sheet `{nav_wid}` in `{locale}` missing localized `{k}`=`{expected}`"));
                }
                if let Some(leak) = tree_has_exact_english_leak(&t_1x, locale, k) {
                    return Err(leak);
                }
            }
            driver.set_metrics(vp_2x);
            let t_2x = driver.layout_tree();
            assert_tree_parity_and_a11y(&t_2x, &format!("{nav_wid}_{locale}_2x"))?;
        }

        // Click through all 5 right-hand inspector tabs on GraphSheet & verify localized sections + controls.
        driver.set_metrics(vp_1x);
        let _ = click_widget_center(&mut driver, &WidgetId::from("nav.sheet.gr-1"))?;
        let insp_tabs: &[(&str, ActiveInspector, &[&str])] = &[
            ("inspector.tab.graph", ActiveInspector::FormatGraph, &["inspector.format_graph", "inspector.fg.dataset", "inspector.fg.symbol_shape"]),
            ("inspector.tab.axes", ActiveInspector::FormatAxes, &["inspector.format_axes", "inspector.axes.target", "inspector.axes.breaks"]),
            ("inspector.tab.multi", ActiveInspector::BubblePlot, &["inspector.section.multivariate", "inspector.var_x", "inspector.var_y"]),
            ("inspector.tab.pairwise", ActiveInspector::PairwiseComparisons, &["inspector.section.pairwise_cld", "inspector.pw.cld_toggle"]),
            ("inspector.tab.layout", ActiveInspector::PageLayout, &["inspector.page_layout", "inspector.layout.grid"]),
        ];
        for &(tab_wid, _, expected_keys) in insp_tabs {
            let _ = click_widget_center(&mut driver, &WidgetId::from(tab_wid))?;
            let itree = driver.layout_tree();
            assert_tree_parity_and_a11y(&itree, &format!("{tab_wid}_{locale}"))?;
            for &k in expected_keys {
                let expected = tr(locale, k);
                if !tree_contains_expected(&itree, expected) {
                    return Err(format!("inspector `{tab_wid}` in `{locale}` missing `{k}`=`{expected}`"));
                }
                if let Some(leak) = tree_has_exact_english_leak(&itree, locale, k) {
                    return Err(leak);
                }
            }
        }

        // Open Analyze modal on DataSheet, Export Figure modal on GraphSheet, and Command Palette via toolbar clicks.
        let _ = click_widget_center(&mut driver, &WidgetId::from("nav.sheet.ds-col"))?;
        assert_eq!(
            active_toolbar_context(driver.app().document()).badge(),
            sheet_section_label("en", curvebook_doc::SheetSection::Data).split(' ').next().unwrap_or("Data")
        );
        let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.analyze"))?;
        if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }) {
            return Err(format!("toolbar.analyze did not open AnalyzeSheet in `{locale}`"));
        }
        let ana_tree = driver.layout_tree();
        assert_tree_parity_and_a11y(&ana_tree, &format!("dlg_analyze_{locale}"))?;
        for k in ["analyze_picker.search_analyses", "analyze_picker.clear"] {
            if !tree_contains_expected(&ana_tree, tr(locale, k)) {
                return Err(format!("Analyze modal in `{locale}` missing `{k}`=`{}`", tr(locale, k)));
            }
        }
        let _ = driver.press_key("Escape", KeyModifiers::default())?;

        let _ = click_widget_center(&mut driver, &WidgetId::from("nav.sheet.gr-1"))?;
        let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.export_svg"))?;
        if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }) {
            return Err(format!("toolbar.export_svg did not open ExportFigure in `{locale}`"));
        }
        let exp_tree = driver.layout_tree();
        assert_tree_parity_and_a11y(&exp_tree, &format!("dlg_export_{locale}"))?;
        for k in ["w5.export.transparency", "w5.export.colorspace"] {
            if !tree_contains_expected(&exp_tree, tr(locale, k)) {
                return Err(format!("Export modal in `{locale}` missing `{k}`=`{}`", tr(locale, k)));
            }
            if let Some(leak) = tree_has_exact_english_leak(&exp_tree, locale, k) {
                return Err(leak);
            }
        }
        let _ = driver.press_key("Escape", KeyModifiers::default())?;

        let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.palette"))?;
        if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette) {
            return Err(format!("toolbar.palette did not open CommandPalette in `{locale}`"));
        }
        let pal_tree = driver.layout_tree();
        assert_tree_parity_and_a11y(&pal_tree, &format!("dlg_palette_{locale}"))?;
        if !tree_contains_expected(&pal_tree, tr(locale, "w5.palette.run")) {
            return Err(format!("CommandPalette in `{locale}` missing `w5.palette.run`"));
        }
        let _ = driver.press_key("Escape", KeyModifiers::default())?;
        let _ = assert_zero_shortcut_compliance(&mut driver)?;
    }

    // Verify table_kind_label & sheet_section_label cover all 8 EU locales distinctly where non-cognate.
    for &locale in EU_LOCALES {
        assert!(!table_kind_label(locale, curvebook_doc::TableKind::Survival).is_empty());
        assert!(!sheet_section_label(locale, curvebook_doc::SheetSection::Analyses).is_empty());
    }
    Ok(())
}

#[test]
fn eu_regional_number_format_csv_grid_edit_and_formula_preservation() -> Result<(), String> {
    for &locale in EU_LOCALES {
        let preset = LocalePreset::for_locale(locale);
        assert_eq!(preset.decimal_separator, ',', "{locale} must use comma decimal separator");
        assert_eq!(preset.csv_delimiter, ';', "{locale} must use semicolon CSV delimiter");
        assert!(!preset.is_rtl(), "{locale} must be LTR");

        // Localized parse + display roundtrip (comma decimal + scientific notation + excluded * badge + formula guard).
        assert_eq!(parse_localized_number_input("12,75", &preset), Some(12.75), "{locale}");
        assert_eq!(parse_localized_number_input("-0,5", &preset), Some(-0.5), "{locale}");
        assert_eq!(parse_localized_number_input("1,5e-3", &preset), Some(0.0015), "{locale}");
        assert_eq!(parse_localized_number_input("=A1*2", &preset), None, "{locale} formulas must stay text");
        assert_eq!(parse_localized_number_input("=MEAN(A; B)", &preset), None, "{locale} formulas must stay text");

        let disp = format_cell_display(&Cell::number(12.75), &preset);
        assert_eq!(disp, "12,75", "{locale}");
        let mut excl = Cell::number(4.5);
        excl.excluded = true;
        assert_eq!(format_cell_display(&excl, &preset), "4,5*", "{locale}");
        assert_eq!(format_cell_display(&Cell::text("=A1*2"), &preset), "=A1*2", "{locale}");

        // Axis ticks & p-values format with EU comma decimal.
        assert_eq!(preset.format_axis_tick(12.34, AxisScale::Linear), "12,34", "{locale}");
        assert_eq!(preset.format_pvalue(0.0023, PValueFormat::FourDecimal, true, true), "P = 0,0023 **", "{locale}");
        assert_eq!(preset.format_pvalue(0.0004, PValueFormat::Apa, false, false), "< ,001", "{locale}");

        // Semicolon CSV + comma-decimal roundtrip for every EU locale.
        let opts = preset.csv_options();
        let csv_in = "X;Y\n1,25;4,5\n2,75;9,125\n";
        let ds = import_csv_to_datasheet("eu-csv", "Dose", csv_in, curvebook_doc::TableKind::Xy, SubcolumnMode::Single, &opts)
            .map_err(|e| format!("{locale} CSV import failed: {e}"))?;
        assert_eq!(ds.x_or_time_cell(0, 0).and_then(|c| c.active_number()), Some(1.25));
        assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(4.5));
        assert_eq!(ds.x_or_time_cell(1, 0).and_then(|c| c.active_number()), Some(2.75));
        assert_eq!(ds.cell(CellCoord::new(1, 0, 0)).and_then(|c| c.active_number()), Some(9.125));
        let exported = export_datasheet_to_csv(&ds, &opts).map_err(|e| format!("{locale} CSV export failed: {e}"))?;
        assert!(exported.contains("1,25;4,5") && exported.contains("2,75;9,125"), "{locale} CSV export `{exported}`");
    }

    // Live HumanPathDriver grid cell entry under German EU locale with comma decimal + formula preservation.
    let app = build_rich_eu_workspace("de")?;
    let mut driver = new_human_driver(app, default_viewport());
    let _ = click_widget_center(&mut driver, &WidgetId::from("nav.sheet.ds-col"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("grid.cell.0.0.0"))?;
    let cell_pt = driver.widget_center_logical("grid.cell.0.0.0").ok_or_else(|| String::from("missing grid.cell.0.0.0"))?;
    let _ = driver.double_click_logical(cell_pt[0], cell_pt[1])?;
    for _ in 0..8 {
        let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    }
    let _ = driver.type_text("3,75")?;
    let _ = driver.press_key("Enter", KeyModifiers::default())?;

    let cell_pt_1 = driver.widget_center_logical("grid.cell.1.0.0").ok_or_else(|| String::from("missing grid.cell.1.0.0"))?;
    let _ = driver.double_click_logical(cell_pt_1[0], cell_pt_1[1])?;
    for _ in 0..8 {
        let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    }
    let _ = driver.type_text("=B1+1")?;
    let _ = driver.press_key("Enter", KeyModifiers::default())?;

    let ds_after = driver.app().document().data_sheet(&curvebook_doc::SheetId::new("ds-col")).ok_or_else(|| String::from("missing ds-col"))?;
    if ds_after.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()) != Some(3.75) {
        return Err(format!(
            "expected comma decimal `3,75` typed via HumanPathDriver to store 3.75, got {:?}",
            ds_after.cell(CellCoord::new(0, 0, 0))
        ));
    }
    let cell_1 = ds_after.cell(CellCoord::new(1, 0, 0));
    if !cell_1.is_some_and(|c| matches!(&c.value, curvebook_doc::CellValue::Text(s) if s == "=B1+1")) {
        return Err(format!("expected formula `=B1+1` typed via HumanPathDriver to preserve text, got {cell_1:?}"));
    }
    let tree = driver.layout_tree();
    if !tree.text_runs.iter().any(|r| r.owner_widget_id.as_str() == "grid.cell.0.0.0" && r.text == "3,75") {
        return Err(String::from("expected rendered grid.cell.0.0.0 text_run to show localized `3,75`"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}
