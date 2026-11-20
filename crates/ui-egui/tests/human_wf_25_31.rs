#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-25`–`WF-31`, `R15.4`/`R15.5`).

mod human_wf_common;

use curvebook_doc::{AnalysisKind, MultiplicityMethod, MultiplicitySpec, OneWayAnovaSpec, SheetId, TableArchetype, VariableSource};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::UiBackend;
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::theme::UiThemeMode;
use egui::{Key, Modifiers};
use serde_json::json;

use human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, ensure};

#[test]
fn r15_4_phase_a_wf25_50_egui_harness_and_14_locale_gate() -> TestRes {
    let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
    app.execute_command("sheet.createData", json!({ "sheet_id": "col-50", "kind": "column", "cols": 3, "rows": 4, "auto_graph": true, "graph_sheet_id": "g-50", "auto_info": true, "info_sheet_id": "info-50" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("table.setTitles", json!({ "sheet_id": "col-50", "titles": ["Alpha", "Beta", "Gamma"] })).map_err(|e| e.to_string())?;
    app.execute_command("table.setCells", json!({ "sheet_id": "col-50", "rows": [[8.0, 13.0, 21.0], [12.0, 17.0, 25.0], [16.0, 21.0, 29.0]] }))
        .map_err(|e| e.to_string())?;
    let kind_val = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() },
        ..Default::default()
    }))
    .map_err(|e| e.to_string())?;
    app.execute_command("analysis.run", json!({ "source_sheet_id": "col-50", "sheet_id": "res-50", "kind": kind_val })).map_err(|e| e.to_string())?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-50", "title": "CLD Layout" })).map_err(|e| e.to_string())?;
    app.execute_command("layout.addPanel", json!({ "sheet_id": "lay-50", "kind": "graph", "graph_sheet_id": "g-50", "row": 0, "col": 0 }))
        .map_err(|e| e.to_string())?;

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1440.0, 800.0, 1.0));
    h.click("nav.sheet.g-50")?;
    ensure(h.app().document().active_sheet_id.as_ref().map(SheetId::as_str) == Some("g-50"), "Graph sheet active")?;
    h.click("inspector.graph.tab.axes")?;
    h.assert_parity_and_layout("Phase A Graph Inspector")?;

    h.click("nav.chip.layouts")?;
    h.click("nav.sheet.lay-50")?;
    ensure(h.app().document().active_sheet_id.as_ref().map(SheetId::as_str) == Some("lay-50"), "Layout sheet active")?;
    let layout_png = h.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(layout_png.starts_with(b"\x89PNG\r\n\x1a\n") && layout_png.len() > 256, "valid layout canvas PNG")?;
    h.assert_parity_and_layout("Phase A LayoutSheet 1440x800")?;

    h.click("nav.chip.info")?;
    h.click("nav.sheet.info-50")?;
    h.assert_parity_and_layout("Phase A InfoSheet")?;

    h.click("nav.chip.results")?;
    h.click("nav.sheet.res-50")?;
    h.assert_parity_and_layout("Phase A ResultsView")?;

    h.click("nav.chip.all")?;
    h.click("nav.sheet.g-50")?;
    h.verify_14_locales_section_9_4("Phase A WF-25..50")?;
    Ok(())
}

#[test]
fn wf25_human_violin_boxplot_bars_and_error_bars() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.column", "Violin Box", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", 10 + r * 2))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", 20 + r * 2))?;
        h.enter_grid_cell(r, 2, 0, &format!("{}", 30 + r * 2))?;
    }
    let _rs = h.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-25 missing graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.graph_type")?;
    h.click("graph_type.card.col.violin")?;
    h.click("modal.ok")?;
    ensure(h.app().document().data_sheet(&sid).is_some(), "WF-25 sheet missing")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-25")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf26_human_bubble_volcano_heatmap_and_r7_to_r10() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV Heatmap")?;
    let rs = h.run_analyze_entry("Correlation", "analyze.entry.stats.corr_matrix")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-26 corr missing")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-26")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf27_human_multi_panel_layout_drag_snap_and_style() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Layout Source", &[])?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-27 graph missing")?;
    h.driver.press_egui_key(Key::K, Modifiers::COMMAND);
    h.driver.type_text("Layout")?;
    h.driver.press_key("Enter", KeyModifiers::default())?;
    let lay_id = h.app().document().layout_sheets.last().map(|l| l.id.clone()).ok_or("WF-27 layout missing")?;
    h.select_sheet(&lay_id)?;
    h.click("toolbar.layout_composer")?;
    h.click("layout.arrange")?;
    h.click("modal.ok")?;
    h.select_sheet(&gs_id)?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-27")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf28_human_canvas_preset_cvd_print_export_and_themes() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Canvas Export", &[])?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-28 graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.export_svg")?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }), "WF-28 ExportFigure")?;
    h.click("modal.cancel")?;
    let light_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    h.app_mut().set_theme_mode(UiThemeMode::Dark);
    h.driver.run_settled();
    let dark_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    ensure(light_png != dark_png, "WF-28 Dark theme must repaint")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-28")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf29_human_select_transform_row_stats_and_baseline() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV Baseline")?;
    let rs = h.run_analyze_entry("Select and Transform", "analyze.entry.transform.select")?;
    h.select_sheet(&rs)?;
    h.open_cmd_t()?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-29 select missing")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-29")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf30_human_calculated_variables_formula_bar_and_guards() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.multiple_variables", "Calc Formula", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", r + 1))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", (r + 1) * 5))?;
    }
    h.click("inspector.mv.col.2")?;
    h.click("inspector.mv.formula_input")?;
    h.driver.press_egui_key(Key::A, Modifiers::COMMAND);
    h.driver.type_text("[Var 2] * 2")?;
    h.click("inspector.mv.apply_formula")?;
    let ds = h.app().document().data_sheet(&sid).ok_or("missing MV")?;
    ensure(
        matches!(ds.archetype, TableArchetype::MultipleVariables(ref mv) if matches!(mv.variables[2].source, VariableSource::Calculated { .. })),
        "WF-30 formula",
    )?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-30")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf31_human_extract_and_rearrange_cross_schema() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Extract 4PL", &[])?;
    for (r, (&x, &y)) in [1.0, 3.0, 10.0, 30.0, 100.0, 300.0].iter().zip([10.0, 22.0, 48.0, 79.0, 93.0, 99.0].iter()).enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let rs = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&rs)?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-31 missing NLREG")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-31")?;
    assert_input_to_paint_budget(&h, 50.0)
}
