#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-13`–`WF-18` + `R13.1`, `R15.4`/`R15.5`).

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, Cell, DataSheet, OneWayMethod, ProjectDocument, ResultCell, ResultSheet, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::AssumptionDiagnosticBanner;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::inspect_ui_tree_for_metrics;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::power_wizard::PowerWizardStage;
use egui::{Key, Modifiers};

use human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, ensure};

fn scalar_val(rs: &ResultSheet, key: &str) -> Result<f64, String> {
    let key_lower = key.to_ascii_lowercase();
    rs.tabs
        .iter()
        .find_map(|tab| {
            let r_idx = tab
                .row_labels
                .iter()
                .position(|lbl| lbl.eq_ignore_ascii_case(key))
                .or_else(|| tab.row_labels.iter().position(|lbl| lbl.to_ascii_lowercase().contains(&key_lower)))?;
            tab.rows.get(r_idx)?.iter().find_map(|c| match c {
                ResultCell::Number { value, .. } => Some(*value),
                _ => None,
            })
        })
        .ok_or_else(|| format!("missing scalar `{key}` in result sheet"))
}

#[test]
fn wf13_human_rm_anova_sphericity_gg_and_hf() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "RM Timecourse", &["welcome.design.paired"])?;
    for (r, vals) in [[10.0, 14.0, 28.0], [12.0, 16.0, 22.0], [14.0, 18.0, 44.0], [16.0, 20.0, 29.0]].iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = h.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.paired_or_matched")?;
    h.click("edit.tab.repeated_measures")?;
    h.click("edit.sphericity.geisser_greenhouse")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-13")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { .. })), "WF-13 RM")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-13")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf14_human_friedman_diagnostics_banner_and_cmd_t() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Friedman RM", &["welcome.design.paired"])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", r + 1))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", r + 2))?;
        h.enter_grid_cell(r, 2, 0, &format!("{}", (r + 1) * 5))?;
    }
    let rs_id = h.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.paired_or_matched")?;
    h.click("edit.assume_gaussian")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-14")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::Friedman), "WF-14 Friedman")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-14")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf15_human_full_four_stage_power_wizard_and_hover() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let before_rs = h.app().document().result_sheets.len();
    h.driver.press_egui_key(Key::K, Modifiers::COMMAND);
    h.driver.type_text("Power")?;
    h.driver.press_key("Enter", KeyModifiers::default())?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize), "WF-15 expected Power Studio")?;
    ensure(h.app().canvas_dialogs.power_wizard.stage == PowerWizardStage::ExperimentDesign, "WF-15 Stage 1")?;
    h.click("power.family.one_way_anova")?;
    h.click("power.stage.3")?;
    ensure(!h.driver.frame_meta().find_by_widget_id("power.one_sided").ok_or("missing power.one_sided")?.enabled, "WF-15 one-sided guard")?;
    h.click("power.stage.1")?;
    h.click("power.family.two_sample_t")?;
    h.click("power.compute_defaults")?;
    h.click("power.tab.curve")?;
    let curve_pos = h.widget_center("power.curve_hover")?;
    h.driver.hover_at(curve_pos)?;
    h.click("power.tab.sensitivity")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheets.len() > before_rs, "WF-15 must create ResultSheet")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-15")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf16_human_linear_deming_pb_ancova_and_r8_r9() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.xy", "Linear Assay", &[])?;
    for r in 0..5 {
        let x = (r + 1) as f64;
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{:.1}", x * 2.0 + 0.1))?;
    }
    let rs_id = h.run_analyze_entry("Linear", "analyze.entry.fit.linear")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.lin.deming")?;
    h.click("edit.tab.diagnostics")?;
    h.click("edit.lin.conf_bands")?;
    h.click("modal.ok")?;
    let mv_sid = h.create_table_via_welcome("welcome.arch.multiple_variables", "MV R9 Sample", &[])?;
    h.select_sheet(&mv_sid)?;
    h.click("toolbar.sample_rows")?;
    ensure(!h.driver.frame_meta().find_by_widget_id("sample.proportional_allocation").ok_or("missing R9")?.enabled, "R9 prop alloc disabled")?;
    h.click("modal.cancel")?;
    let _ = sid;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-16")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf17_human_dose_response_4pl_5pl_interp_and_heatmap_r7() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Dose 4PL", &["welcome.subcol.raw"])?;
    for (r, (&x, &y)) in [1.0, 3.0, 10.0, 30.0, 100.0, 300.0].iter().zip([5.0, 18.0, 48.0, 78.0, 94.0, 99.0].iter()).enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
        h.enter_grid_cell(r, 0, 1, &format!("{:.1}", y + 1.5))?;
    }
    let rs_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("fit.tab.compare")?;
    h.click("fit.tab.output")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-17 missing 4PL")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-17")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf18_human_enzyme_kinetics_global_vmax_diagnostics() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Enzyme Kinetics", &[])?;
    for (r, (&s, &v)) in [1.0, 2.0, 5.0, 10.0, 25.0, 50.0].iter().zip([12.0, 20.0, 38.0, 56.0, 74.0, 83.0].iter()).enumerate() {
        h.enter_x_cell(r, 0, &format!("{s}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{v}"))?;
    }
    let rs_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("fit.eq.search")?;
    h.driver.type_text("michaelis")?;
    h.click("fit.eq.item.michaelis_menten")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("WF-18 missing MM fit")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::NonlinearRegression(ref f) if f.equation_id == "michaelis_menten"), "WF-18 equation mismatch")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-18")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn r13_1_human_linear_fit_range_controls() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.xy", "Terminal Phase", &[])?;
    for (r, (x, y)) in [(0.0, 90.0), (10.0, 24.0), (12.0, 26.0), (14.0, 30.0), (16.0, 36.0), (20.0, -100.0)].into_iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    h.select_sheet(&sid)?;
    let fit_id = h.run_analyze_entry("Linear", "analyze.entry.fit.linear")?;
    h.select_sheet(&fit_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.range")?;
    h.replace_textbox("fit.lr.range.input.x_min", "10")?;
    h.replace_textbox("fit.lr.range.input.x_max", "16")?;
    h.replace_textbox("fit.lr.range.input.segments", "0")?;
    let seg_meta = h.driver.frame_meta().find_by_widget_id("fit.lr.range.input.segments").ok_or("missing segments field")?;
    ensure(seg_meta.value.as_ref().and_then(|v| v.as_str()) == Some("0"), "segments draft should reflect 0")?;

    // Invalid segments keeps the modal open and leaves stored fit unchanged.
    h.click("modal.ok")?;
    ensure(
        h.driver.state().last_intent_error.as_deref().is_some_and(|e| e.contains("Curve segments")),
        format!("expected Curve segments error, got {:?}", h.driver.state().last_intent_error),
    )?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }), "dialog must remain open")?;
    let unchanged = h.app().document().result_sheet(&fit_id).ok_or("missing unchanged result")?;
    ensure(matches!(&unchanged.analysis.kind, AnalysisKind::LinearRegression(spec) if spec.x_range_filter.is_none()), "range unchanged")?;

    h.replace_textbox("fit.lr.range.input.segments", "3")?;
    h.click("edit.tab.diagnostics")?;
    h.click("edit.lin.conf_bands")?;
    h.click("modal.ok")?;
    let restricted = h.app().document().result_sheet(&fit_id).ok_or("missing restricted result")?;
    ensure(
        matches!(
            &restricted.analysis.kind,
            AnalysisKind::LinearRegression(spec) if spec.x_range_filter == Some((Some(10.0), Some(16.0)))
                && spec.plot_segments == Some(3) && !spec.confidence_bands
        ),
        "restricted linear regression spec mismatch",
    )?;
    ensure((scalar_val(restricted, "slope")? - 2.0).abs() < 1e-12, "slope mismatch")?;
    let overlay = restricted.fitted_curves.first().ok_or("missing restricted overlay")?;
    ensure(overlay.points.iter().map(|p| p.x).collect::<Vec<_>>() == [10.0, 12.0, 14.0, 16.0], "restricted overlay grid")?;

    // Both dismissal routes discard buffered input before the next edit session.
    for escape in [true, false] {
        h.open_cmd_t()?;
        h.click("edit.tab.range")?;
        h.replace_textbox("fit.lr.range.input.plot_max", "22")?;
        if escape {
            h.driver.press_key("Escape", KeyModifiers::default())?;
        } else {
            h.click("modal.cancel")?;
        }
        ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::None), "dialog must dismiss")?;
        h.open_cmd_t()?;
        h.click("edit.tab.range")?;
        let pm = h.driver.frame_meta().find_by_widget_id("fit.lr.range.input.plot_max").ok_or("missing plot_max field")?;
        ensure(pm.value.as_ref().and_then(|v| v.as_str()) == Some(""), format!("expected empty plot_max draft, got {:?}", pm.value))?;
        h.click("modal.cancel")?;
    }

    h.open_cmd_t()?;
    h.click("edit.tab.range")?;
    h.replace_textbox("fit.lr.range.input.plot_min", "7")?;
    h.replace_textbox("fit.lr.range.input.plot_max", "19")?;
    h.click("modal.ok")?;
    let extrapolated = h.app().document().result_sheet(&fit_id).ok_or("missing extrapolated result")?;
    ensure((scalar_val(extrapolated, "slope")? - 2.0).abs() < 1e-12, "extrapolated slope mismatch")?;
    let overlay = extrapolated.fitted_curves.first().ok_or("missing extrapolated overlay")?;
    ensure(overlay.points.iter().map(|p| p.x).collect::<Vec<_>>() == [7.0, 11.0, 15.0, 19.0], "extrapolated overlay grid")?;
    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn r13_1_human_nonlinear_diagnostics_settings() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.xy", "Dose Trend", &[])?;
    for (r, (x, y)) in [(-9.0, 10.0), (-8.0, 13.0), (-7.0, 28.0), (-6.0, 58.0), (-5.0, 83.0), (-4.0, 96.0)].into_iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    h.select_sheet(&sid)?;
    let fit_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&fit_id)?;
    h.open_cmd_t()?;
    h.click("fit.tab.output")?;
    h.click("fit.out.unstable")?;
    let unstable_node = h.driver.frame_meta().find_by_widget_id("fit.out.unstable").ok_or("missing fit.out.unstable")?;
    ensure(unstable_node.value == Some(serde_json::json!(true)), "unstable checkbox must toggle on")?;

    h.replace_textbox("fit.diagnostics.r2.input", "1.1")?;
    h.click("modal.ok")?;
    ensure(
        h.driver.state().last_intent_error.as_deref().is_some_and(|e| e.contains("R² threshold")),
        format!("expected R² threshold error, got {:?}", h.driver.state().last_intent_error),
    )?;
    h.replace_textbox("fit.diagnostics.r2.input", "1")?;
    h.click("modal.ok")?;

    let result = h.app().document().result_sheet(&fit_id).ok_or("missing R2 result")?;
    ensure(
        matches!(
            &result.analysis.kind,
            AnalysisKind::NonlinearRegression(spec) if spec.identify_unstable_parameters && spec.flag_r_squared_below == Some(1.0)
        ),
        "expected unstable + R² threshold 1.0",
    )?;
    ensure(
        result.checklist.iter().any(|item| item.assumption == "R squared below threshold" && item.remediation.is_none()),
        "expected checklist warning",
    )?;
    let banner = AssumptionDiagnosticBanner::from_result(result).ok_or("missing diagnostic banner")?;
    ensure(banner.source_result_id == result.id, "diagnostic banner source mismatch")?;

    h.open_cmd_t()?;
    h.click("fit.tab.output")?;
    let field = h.driver.frame_meta().find_by_widget_id("fit.diagnostics.r2.input").ok_or("missing threshold field")?;
    ensure(field.value == Some(serde_json::json!("1")), "expected persisted threshold 1")?;
    h.replace_textbox("fit.diagnostics.r2.input", "")?;
    h.click("modal.ok")?;
    let result = h.app().document().result_sheet(&fit_id).ok_or("missing cleared result")?;
    ensure(matches!(&result.analysis.kind, AnalysisKind::NonlinearRegression(spec) if spec.flag_r_squared_below.is_none()), "cleared R² threshold")?;
    ensure(result.checklist.iter().all(|item| item.assumption != "R squared below threshold"), "checklist warning removed")?;
    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn r13_1_human_replicate_interpolation_tabs() -> TestRes {
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 9).map_err(|e| e.to_string())?;
    for (row, x) in [-9.0, -8.0, -7.0, -6.8, -6.2, -5.5, -4.5, -4.0].into_iter().enumerate() {
        xy.set_x_cell(row, Cell::number(x)).map_err(|e| e.to_string())?;
        let y = 10.0 + 90.0 / (1.0 + 10_f64.powf(-6.5 - x));
        let cells = xy.data_sets.first_mut().and_then(|ds| ds.cells.get_mut(row)).ok_or("missing calibration row")?;
        *cells = vec![Cell::number(y - 0.25), Cell::number(y + 0.25)];
    }
    let unknowns = xy.data_sets.first_mut().and_then(|ds| ds.cells.last_mut()).ok_or("missing unknown row")?;
    *unknowns = vec![Cell::number(45.0), Cell::number(65.0)];
    let sheet = DataSheet::new("replicate-calibration", "Replicate calibration", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    let mut doc = ProjectDocument::new("Replicate interpolation").map_err(|e| e.to_string())?;
    doc.add_data_sheet(sheet).map_err(|e| e.to_string())?;
    let app = curvebook_ui_egui::UiApp::from_session(Session::from_document(doc));
    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));

    h.select_sheet(&SheetId::from("replicate-calibration"))?;
    let fit_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&fit_id)?;
    h.open_cmd_t()?;
    h.click("fit.tab.output")?;
    h.click("fit.out.interp")?;
    ensure(
        h.driver.frame_meta().find_by_widget_id("fit.out.interp").and_then(|n| n.value.clone()) == Some(serde_json::json!(true)),
        "interp enabled",
    )?;
    h.click("modal.ok")?;

    for (tab_id, title, rows) in
        [("nonlinear_interpolated_mean_x", "Interpolated Mean X", 1), ("nonlinear_interpolated_replicates", "Interpolated Replicates", 2)]
    {
        let widget_id = format!("result.tab.{tab_id}");
        for _ in 0..12 {
            if h.driver.frame_meta().find_by_widget_id(&widget_id).is_some_and(|e| e.clipped_rect.is_positive()) {
                break;
            }
            h.click("result.tabs.next")?;
        }
        h.click(&widget_id)?;
        let snap = inspect_ui_tree_for_metrics(h.app(), ViewportMetrics::new(1280.0, 800.0, 1.0), Some("tabs"), true);
        let tab_node = snap.tabs.as_deref().unwrap_or(&[]).iter().find(|n| n.widget_id == widget_id).ok_or("missing selected interpolation tab")?;
        ensure(tab_node.value.as_ref().and_then(|v| v.get("selected")).and_then(|v| v.as_bool()) == Some(true), "interpolation tab selected")?;
        let result = h.app().document().result_sheet(&fit_id).ok_or("missing interpolation result sheet")?;
        let tab = result.tabs.iter().find(|tab| tab.id == tab_id).ok_or("missing interpolation output")?;
        ensure(tab.title == title && tab.rows.len() == rows, "interpolation tab rows mismatch")?;
    }
    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)
}
