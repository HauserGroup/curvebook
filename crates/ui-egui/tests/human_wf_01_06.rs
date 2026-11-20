#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-01`–`WF-06`, `R15.4`/`R15.5`).

mod human_wf_common;

use curvebook_doc::{AnalysisKind, Cell, CellCoord, MultivariateSpec, OneWayAnovaSpec, OneWayMethod, TableKind, TransformAnalysisSpec};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::import_wizard::ImportWizardStep;
use curvebook_ui_egui::egui_shell::stat_anova_modal::resolve_one_way_anova_banner;
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use serde_json::Value;

use human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, ensure};

#[test]
fn r15_4_phase_a_wf01_24_egui_harness_and_14_locale_gate() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    h.assert_parity_and_layout("Phase A WF-01..24 initial Welcome")?;

    let ds_id = h.create_table_via_welcome("welcome.arch.column", "Phase A Drug Cohort", &[])?;
    for (r, vals) in [[10.0, 14.0, 22.0], [11.0, 15.0, 24.0], [9.5, 13.5, 20.5], [10.5, 14.5, 23.0]].iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let val = h.app().document().data_sheet(&ds_id).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    ensure(val == Some(10.0), "Phase A column grid edit failed")?;

    let mut app = h.app().clone();
    app.set_gui_only(false);
    app.set_human_path_only(false);
    let kind_val = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    let anova_res =
        app.execute_command("analysis.run", serde_json::json!({ "source_sheet_id": ds_id.as_str(), "kind": kind_val })).map_err(|e| e.to_string())?;
    let rs_id_str = anova_res.payload.get("sheet_id").and_then(Value::as_str).ok_or("missing anova sheet_id")?.to_owned();
    let rs_id = curvebook_doc::SheetId::from(rs_id_str.as_str());
    let mut h2 = EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    h2.select_sheet(&rs_id)?;
    h2.open_cmd_t()?;
    h2.click("edit.tab.multiple_comparisons")?;
    ensure(!h2.driver.frame_meta().find_by_widget_id("edit.control_column").unwrap().enabled, "R3 control_column starts disabled")?;
    h2.click("edit.compare_against_control")?;
    ensure(h2.driver.frame_meta().find_by_widget_id("edit.control_column").unwrap().enabled, "R3 unlocks control_column")?;
    h2.click("edit.tab.experimental_design")?;
    h2.click("edit.assume_gaussian")?;
    ensure(resolve_one_way_anova_banner(h2.app()).contains("Kruskal-Wallis"), "R2 banner")?;
    h2.click("modal.ok")?;

    let rs = h2.app().document().result_sheet(&rs_id).ok_or("missing updated ResultSheet")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::KruskalWallis), "expected Kruskal-Wallis")?;
    let inspect_snap = h2.app_mut().inspect_egui_shell_snapshot(ViewportMetrics::new(1280.0, 800.0, 1.0), None, true);
    ensure(inspect_snap.active_sheet_id.as_deref() == Some(rs_id.as_str()), "ui.inspect mismatch")?;
    let shot = h2.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    ensure(shot.starts_with(b"\x89PNG\r\n\x1a\n") && shot.len() > 256, "invalid egui PNG screenshot")?;
    h2.driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
    h2.assert_parity_and_layout("Phase A WF-01..24 ResultSheet 1440x600")?;
    h2.select_sheet(&ds_id)?;
    h2.verify_14_locales_section_9_4("Phase A WF-01..24")?;
    Ok(())
}

#[test]
fn wf01_human_xy_dose_response_format_exclude_with_reason() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.xy", "Dose Response", &["welcome.subcol.raw"])?;
    for (r, &x) in [1.0, 3.0, 10.0, 30.0, 100.0, 300.0].iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{}", 100.0 - (r as f64) * 18.0))?;
        h.enter_grid_cell(r, 0, 1, &format!("{}", 102.0 - (r as f64) * 18.0))?;
    }
    h.click("grid.cell.2.0.1")?;
    h.driver.press_key("E", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }), "WF-01 ExcludeWithReason modal")?;
    h.click("exclude.preset.0")?;
    h.click("modal.ok")?;
    let ds = h.app().document().data_sheet(&sid).ok_or("missing xy sheet")?;
    ensure(ds.kind() == TableKind::Xy && ds.cell(CellCoord::new(2, 0, 1)).is_some_and(|c| c.excluded), "WF-01 exclusion failed")?;
    let rs_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    ensure(h.driver.frame_meta().find_by_widget_id("fit.eq.tex_preview").is_some(), "WF-01 FitModal TeX preview")?;
    h.click("modal.ok")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-01")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf02_human_normalize_to_result_sheet_chaining_and_log_x() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Norm Source", &["welcome.stepper.cols.dec", "welcome.stepper.cols.dec"])?;
    for (r, (&x, &y)) in [1.0, 10.0, 100.0, 1000.0].iter().zip([10.0, 40.0, 70.0, 100.0].iter()).enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let norm_rs = h.run_analyze_entry("Normalize", "analyze.entry.transform.normalize")?;
    h.select_sheet(&norm_rs)?;
    let log_rs = h.run_analyze_entry("Transform", "analyze.entry.transform.math")?;
    h.select_sheet(&log_rs)?;
    h.open_cmd_t()?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&log_rs).ok_or("missing log result")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { .. })), "WF-02 expected Transform::Math")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-02")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf03_human_column_brushing_format_and_undo_redo() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.column", "Brush Column", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", (r + 1) * 10))?;
    }
    let png0 = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    let c0 = h.widget_center("grid.cell.0.0.0")?;
    let c2 = h.widget_center("grid.cell.2.0.0")?;
    h.driver.drag(c0, c2)?;
    h.enter_grid_cell(0, 0, 0, "88.0")?;
    h.driver.press_key("Z", KeyModifiers { meta: true, ..Default::default() })?;
    let v_undo = h.app().document().data_sheet(&sid).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    ensure(v_undo == Some(10.0), "WF-03 Cmd+Z must undo")?;
    h.driver.press_key("Z", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    let v_redo = h.app().document().data_sheet(&sid).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    ensure(v_redo == Some(88.0), "WF-03 Cmd+Shift+Z must redo")?;
    let png1 = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    ensure(png0 != png1, "WF-03 screenshot must reflect edit")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-03")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf04_human_import_reshape_and_convert_archetype() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.column", "Convertible Column", &[])?;
    h.driver.state_mut().intents.push_open_import_wizard(ImportOriginKind::Clipboard, false, None);
    {
        let st = h.driver.state_mut();
        st.app.egui_views = st.views.clone();
        let _ = st.intents.commit_if_settled(&mut st.app, false);
        st.views = st.app.egui_views.clone();
    }
    h.driver.run_settled();
    h.click("import_paste.input.csv")?;
    h.driver.state_mut().views.import_wizard.raw_source_draft = String::from("X,A,B\n1,10,12\n2,20,24");
    h.driver.run_settled();
    h.click("modal.next")?;
    ensure(h.driver.state().views.import_wizard.active_step == ImportWizardStep::Preview, "WF-04 stage 2 Preview")?;
    h.click("modal.next")?;
    h.click("modal.next")?;
    h.click("modal.next")?;
    h.click("modal.ok")?;
    ensure(!h.app().grid.import_wizard.open, "WF-04 import wizard must close on OK")?;

    h.select_sheet(&sid)?;
    h.click("toolbar.convert_archetype")?;
    h.click("convert.target.grouped")?;
    h.click("convert.allow_lossy")?;
    h.click("modal.ok")?;
    ensure(h.app().document().data_sheet(&sid).is_some_and(|s| s.kind() == TableKind::Grouped), "WF-04 converted to Grouped")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-04")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf05_human_transform_pca_and_clustering_pipeline() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.multiple_variables", "Omics Matrix", &["welcome.stepper.cols.inc"])?;
    h.seed_mv_4col_5row(&sid)?;
    let pca_rs = h.run_analyze_entry("Principal", "analyze.entry.stats.pca")?;
    h.select_sheet(&pca_rs)?;
    h.open_cmd_t()?;
    h.click("edit.pca.corr")?;
    h.click("modal.ok")?;
    h.select_sheet(&sid)?;
    let clust_rs = h.run_analyze_entry("Cluster", "analyze.entry.stats.clustering")?;
    let rs = h.app().document().result_sheet(&clust_rs).ok_or("missing WF-05 clustering")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })), "WF-05 clustering")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-05")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf06_human_prune_descriptive_survival_imputation_reactive() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.column", "Cohort 6", &[])?;
    for (r, v) in [10.0, 12.0, 14.0, 16.0, 18.0, 20.0].iter().enumerate() {
        h.enter_grid_cell(r, 0, 0, &format!("{v}"))?;
    }
    let prune_rs = h.run_analyze_entry("Prune", "analyze.entry.transform.prune")?;
    ensure(h.app().document().result_sheet(&prune_rs).is_some(), "WF-06 prune missing")?;
    h.select_sheet(&sid)?;
    let desc_rs = h.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;
    h.select_sheet(&desc_rs)?;
    h.open_cmd_t()?;
    h.click("edit.desc.normality")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&desc_rs).is_some(), "WF-06 descriptive missing")?;
    h.select_sheet(&sid)?;
    h.enter_grid_cell(0, 0, 0, "15.0")?;
    ensure(h.app().document().result_sheet(&desc_rs).is_some(), "WF-06 reactive descriptive")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-06")?;
    assert_input_to_paint_budget(&h, 50.0)
}
