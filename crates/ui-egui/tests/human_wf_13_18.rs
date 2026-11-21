#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-13`–`WF-18` + `R13.1`, `R15.4`/`R15.5`).

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ColumnDesignHint, DataSheet, DiagnosticPlotKind, GraphKind, LinearRegressionSpec, MultipleVariablesTable,
    MultivariateSpec, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod, PValueFormat, ParameterRule, ParameterScope, PassingBablokSpec,
    PowerAnalysisSpec, PowerEffectSpec, PowerSolveTarget, PowerTailMode, PowerTestFamily, ProjectDocument, ResultCell, ResultSheet, ResultTabKind,
    SheetId, SubcolumnAggregation, SubcolumnMode, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_testkit::wf_datasets;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::command_palette::filter_palette_commands;
use curvebook_ui_egui::egui_shell::top_chrome::effective_menu_item_enabled;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::power_wizard::PowerWizardStage;
use curvebook_ui_egui::{AssumptionDiagnosticBanner, inspect_ui_tree_for_metrics};
use serde_json::json;

use human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
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

fn widget_enabled(run: &WorkflowRun, wid: &str) -> Result<bool, String> {
    run.frame_meta().find_by_widget_id(wid).map(|n| n.enabled).ok_or_else(|| format!("missing widget `{wid}`"))
}

fn widget_value_bool(run: &WorkflowRun, wid: &str) -> Result<bool, String> {
    let node = run.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing widget `{wid}`"))?;
    node.value.as_ref().and_then(|v| v.as_bool()).ok_or_else(|| format!("widget `{wid}` has non-bool value: {:?}", node.value))
}

fn enter_xy_replicate_fixture(run: &mut WorkflowRun, fx: &wf_datasets::XyReplicateFixture) -> TestRes {
    for (c, title) in fx.series_titles.iter().enumerate() {
        run.enter_column_title(c, title)?;
    }
    for (r, row) in fx.rows.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{}", row.x))?;
        for (c, reps) in row.series_replicates.iter().enumerate() {
            for (s, &val) in reps.iter().enumerate() {
                run.enter_grid_cell(r, c, s, &format!("{val}"))?;
            }
        }
    }
    Ok(())
}

#[test]
fn wf13_human_rm_anova_sphericity_gg_and_hf() -> TestRes {
    let mut run = WorkflowRun::start("WF-13");
    let ds_id = run.create_table_via_welcome("welcome.arch.column", "RM Timecourse", &["welcome.design.paired"])?;
    let rows = [[10.0, 14.0, 22.0], [12.0, 19.0, 25.0], [14.0, 17.0, 34.0], [16.0, 24.0, 31.0]];
    for (r, vals) in rows.iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = run.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;

    // Explicitly switch to RM ANOVA + Geisser-Greenhouse correction via Cmd+T -> Repeated Measures tab,
    // verify Assume Sphericity toggle (`geisser_greenhouse: false`) and back to GG (`true`), and verify both
    // Greenhouse-Geisser and Huynh-Feldt epsilon scalars and adjusted P-values (`G-G P`, `H-F P`).
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.paired_or_matched")?;
    run.click("edit.tab.repeated_measures")?;
    ensure(widget_enabled(&run, "edit.sphericity.geisser_greenhouse")?, "WF-13 GG checkbox must be enabled")?;
    ensure(widget_enabled(&run, "edit.rm.assume_sphericity")?, "WF-13 Assume Sphericity radio must be enabled")?;
    run.click("edit.rm.assume_sphericity")?;
    ensure(!widget_value_bool(&run, "edit.sphericity.geisser_greenhouse")?, "WF-13 Assume Sphericity must uncheck GG")?;
    run.commit_modal_ok()?;
    ensure(
        matches!(
            run.document().result_sheet(&rs_id).map(|r| &r.analysis.kind),
            Some(AnalysisKind::OneWayAnova(a)) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })
        ),
        "WF-13 expected RepeatedMeasures { geisser_greenhouse: false }",
    )?;

    run.open_cmd_t()?;
    run.click("edit.tab.repeated_measures")?;
    run.click("edit.sphericity.geisser_greenhouse")?;
    ensure(widget_value_bool(&run, "edit.sphericity.geisser_greenhouse")?, "WF-13 GG checkbox must select GG=true")?;
    run.commit_modal_ok()?;
    ensure(
        matches!(
            run.document().result_sheet(&rs_id).map(|r| &r.analysis.kind),
            Some(AnalysisKind::OneWayAnova(a)) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: true })
        ),
        "WF-13 expected RepeatedMeasures { geisser_greenhouse: true }",
    )?;

    let rm_tab_idx = run
        .document()
        .result_sheet(&rs_id)
        .and_then(|r| r.tabs.iter().position(|t| t.id == "rm-one-way-anova"))
        .ok_or("WF-13 missing rm-one-way-anova tab")?;

    let mut ref_doc = ProjectDocument::new("WF-13 Oracle").map_err(|e| e.to_string())?;
    let mut col_tbl = curvebook_doc::ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, rows.len()).map_err(|e| e.to_string())?;
    for (r, vals) in rows.iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            col_tbl.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let ref_sheet = DataSheet::new("ref-rm", "RM Timecourse", TableArchetype::Column(col_tbl)).map_err(|e| e.to_string())?;
    ref_doc.add_data_sheet(ref_sheet).map_err(|e| e.to_string())?;

    let gg_kind = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
        ..OneWayAnovaSpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-rm", "kind": gg_kind })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_document(ref_doc)
        .scalar("F (treatment)", None, 1e-9, 1e-9)
        .scalar("Greenhouse-Geisser epsilon", None, 1e-9, 1e-9)
        .scalar("Huynh-Feldt epsilon", None, 1e-9, 1e-9)
        .scalar("P value", None, 1e-12, 1e-12)
        .tab_cell(rm_tab_idx, 0, 5, 1e-12, 1e-12)
        .tab_cell(rm_tab_idx, 0, 6, 1e-12, 1e-12),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(ds_id, 2, 2, 0, "68.0"),
        vec![
            LiveEditExpectation::scalar_changed(rs_id.clone(), "F (treatment)", None, 0.1),
            LiveEditExpectation::scalar_changed(rs_id.clone(), "Greenhouse-Geisser epsilon", None, 1e-3),
            LiveEditExpectation::tab_cell_changed(rs_id, rm_tab_idx, 0, 6),
        ],
    )?;
    run.finish()
}

#[test]
fn wf14_human_friedman_diagnostics_banner_and_cmd_t() -> TestRes {
    let mut run = WorkflowRun::start("WF-14");
    let ds_id = run.create_table_via_welcome("welcome.arch.column", "Friedman RM", &["welcome.design.paired"])?;
    // Matched skewed observations so initial Ordinary One-Way ANOVA triggers AssumptionDiagnosticBanner.
    let rows = [[1.0, 2.0, 5.0], [1.5, 2.8, 7.0], [2.0, 3.5, 9.0], [1.8, 3.1, 14.0], [2.2, 4.0, 42.0], [2.5, 4.5, 110.0]];
    for (r, vals) in rows.iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = run.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    run.select_sheet(&rs_id)?;

    // Verify AssumptionDiagnosticBanner is emitted and QOL-7 auto-remediation is deprecated/disabled.
    {
        let rs_before = run.document().result_sheet(&rs_id).ok_or("missing WF-14 initial ResultSheet")?;
        ensure(AssumptionDiagnosticBanner::from_result(rs_before).is_some(), "WF-14 expected AssumptionDiagnosticBanner on skewed data")?;
    }
    ensure(run.frame_meta().find_by_widget_id("result.diagnostic_banner.inspect").is_some(), "WF-14 banner inspect button missing")?;
    ensure(run.frame_meta().find_by_widget_id("result.diagnostic_banner.remediate").is_none(), "WF-14 QOL-7 banner must not render auto-remediate")?;
    ensure(!effective_menu_item_enabled("ui.result.applyRemediation", true), "QOL-7 applyRemediation menu state must be disabled")?;
    ensure(
        filter_palette_commands(run.app(), "Remediation", 10).iter().all(|c| !c.enabled || c.command_id != "ui.result.applyRemediation"),
        "QOL-7 command palette must disable/omit ui.result.applyRemediation",
    )?;
    run.click("menu.analyze")?;
    ensure(run.frame_meta().find_by_widget_id("menu.ui.result.inspectDiagnostics").is_some(), "Analyze menu must expose inspectDiagnostics")?;
    ensure(
        run.frame_meta().find_by_widget_id("menu.ui.result.applyRemediation").is_none(),
        "QOL-7 Analyze menu must omit deprecated applyRemediation",
    )?;
    run.press_key("Escape", KeyModifiers::default())?;

    // Click banner inspectDiagnostics button and verify diagnostics tab opens cleanly.
    run.click("result.diagnostic_banner.inspect")?;
    {
        let rs_view = run.document().result_sheet(&rs_id).ok_or("missing WF-14 ResultSheet")?;
        let active_tab = run.app().canvas_dialogs.result_view.active_tab_index;
        let is_diag = rs_view.tabs.get(active_tab).is_some_and(|t| matches!(t.kind, ResultTabKind::Diagnostics | ResultTabKind::Residuals));
        ensure(is_diag, format!("inspectDiagnostics should focus Diagnostics/Residuals tab, active={active_tab}"))?;
    }

    // Switch via Cmd+T explicitly to paired + non-Gaussian -> Friedman nonparametric RM ANOVA.
    run.open_cmd_t()?;
    run.click("edit.paired_or_matched")?;
    run.click("edit.assume_gaussian")?;
    ensure(widget_value_bool(&run, "edit.anova.friedman")?, "WF-14 expected Friedman method radio selected")?;
    run.commit_modal_ok()?;
    {
        let rs_fr = run.document().result_sheet(&rs_id).ok_or("missing WF-14 Friedman ResultSheet")?;
        ensure(matches!(rs_fr.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::Friedman), "WF-14 expected Friedman")?;
    }

    let mut col_tbl = curvebook_doc::ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, rows.len()).map_err(|e| e.to_string())?;
    for (r, vals) in rows.iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            col_tbl.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let ref_sheet = DataSheet::new("ref-friedman", "Friedman RM", TableArchetype::Column(col_tbl)).map_err(|e| e.to_string())?;
    let friedman_kind =
        serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::Friedman, ..OneWayAnovaSpec::default() }))
            .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-friedman", "kind": friedman_kind })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .scalar("Friedman Q", None, 1e-9, 1e-9)
        .scalar("P value", None, 1e-12, 1e-12)
        .scalar("Kendall's W", None, 1e-9, 1e-9),
    )?;

    run.live_edit(LiveEditTarget::sheet_grid_cell(ds_id, 5, 0, 0, "200.0"), LiveEditExpectation::scalar_changed(rs_id, "Friedman Q", None, 0.5))?;
    run.finish()
}

#[test]
fn wf15_human_full_four_stage_power_wizard_and_hover() -> TestRes {
    let mut run = WorkflowRun::start("WF-15");
    run.open_power_wizard()?;
    ensure(matches!(run.active_dialog(), ActiveDialog::PowerAndSampleSize), "WF-15 expected PowerAndSampleSize dialog")?;
    ensure(run.app().canvas_dialogs.power_wizard.stage == PowerWizardStage::ExperimentDesign, "WF-15 Stage 1")?;

    // Configure One-Way ANOVA, advance to Stage 3 via power_wizard_next verifying one-sided predicate disabled.
    run.click("power.family.one_way_anova")?;
    run.power_wizard_next()?;
    run.power_wizard_next()?;
    ensure(!widget_enabled(&run, "power.one_sided")?, "WF-15 Stage 3 must disable one-sided test for One-Way ANOVA")?;

    // Navigate back to Stage 1 via power_wizard_back, select Two-Sample t with non-default 90% target power.
    run.power_wizard_back()?;
    run.power_wizard_back()?;
    ensure(run.app().canvas_dialogs.power_wizard.stage == PowerWizardStage::ExperimentDesign, "WF-15 back to Stage 1")?;
    run.click("power.family.two_sample_t")?;
    run.power_wizard_next()?;
    run.click("power.effect_mode.direct")?;
    run.click("power.pilot.preset.medium")?;
    run.power_wizard_next()?;
    run.click("power.stage3.power.90")?;
    run.power_wizard_next()?;
    ensure(run.app().canvas_dialogs.power_wizard.stage == PowerWizardStage::Results, "WF-15 Stage 4")?;

    // Hover power curve and inspect sensitivity table on Stage 4 before creating ResultSheet.
    run.click("power.tab.curve")?;
    run.hover("power.curve_hover")?;
    let hover_node = run.frame_meta().find_by_widget_id("power.curve_hover").ok_or("missing power.curve_hover")?;
    let hover_n = hover_node.value.as_ref().and_then(|v| v.get("hover_n")).and_then(|v| v.as_f64());
    let sens_len = hover_node.value.as_ref().and_then(|v| v.get("sensitivity_len")).and_then(|v| v.as_u64()).unwrap_or(0);
    ensure(hover_n.is_some() && hover_node.label.contains("N = "), format!("WF-15 missing hover readout: {:?}", hover_node.label))?;
    ensure(sens_len > 0, "WF-15 sensitivity table must be populated")?;
    run.click("power.tab.sensitivity")?;
    ensure(run.frame_meta().find_by_widget_id("power.sensitivity.row.0").is_some(), "WF-15 missing sensitivity matrix row")?;

    let eff_val =
        run.app().canvas_dialogs.power_wizard.derived_standardized_effect().unwrap_or(run.app().canvas_dialogs.power_wizard.standardized_effect);
    let alpha_val = run.app().canvas_dialogs.power_wizard.alpha;
    let rs_id = run.power_wizard_create()?;
    let ref_spec = PowerAnalysisSpec {
        family: PowerTestFamily::TwoSampleUnpairedT,
        solve_target: PowerSolveTarget::SampleSize,
        tail_mode: PowerTailMode::TwoSided,
        alpha: alpha_val,
        target_power: Some(0.90),
        effect: PowerEffectSpec::Standardized(eff_val),
        ..PowerAnalysisSpec::default()
    };
    let ref_power_xy =
        DataSheet::new("ref-power-xy", "Ref Power XY", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).map_err(|e| e.to_string())?))
            .map_err(|e| e.to_string())?;
    let power_kind = serde_json::to_value(AnalysisKind::PowerAnalysis(ref_spec)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-power-xy", "kind": power_kind })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_power_xy)?
        .scalar("NPerGroup", None, 0.0, 0.0)
        .scalar("TotalN", None, 0.0, 0.0)
        .scalar("AchievedPower", None, 1e-9, 1e-9),
    )?;

    // Stage 5 LiveEdit via Cmd+T on the Power ResultSheet (GAP: WF-15 blocks edit/commit via Cmd+T).
    run.live_edit(
        LiveEditTarget::gui(|run| {
            run.open_cmd_t()?;
            run.power_wizard_stage(3)?;
            run.click("power.stage3.power.95")?;
            run.power_wizard_stage(4)?;
            run.commit_modal_ok()?;
            ensure(!matches!(run.active_dialog(), ActiveDialog::EditAnalysisParameters { .. }), "GAP: WF-15 Power Cmd+T modal stayed open")?;
            Ok(())
        }),
        LiveEditExpectation::scalar_changed(rs_id, "NPerGroup", None, 1.0),
    )?;
    run.finish()
}

#[test]
fn wf16_human_linear_deming_pb_ancova_and_r8_r9() -> TestRes {
    let mut run = WorkflowRun::start("WF-16");

    // 1. Multi-series XY table with 2 raw replicates (`Lot A`, `Lot B`) for Simple Linear + ANCOVA + R8.
    let xy_id = run.create_table_via_welcome(
        "welcome.arch.xy",
        "Standard Curve ANCOVA",
        &["welcome.subcol.raw", "welcome.stepper.replicates.dec", "welcome.stepper.cols.dec"],
    )?;
    run.enter_column_title(0, "Lot A")?;
    run.enter_column_title(1, "Lot B")?;
    for r in 0..6 {
        let x = (r + 1) as f64;
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        run.enter_grid_cell(r, 0, 0, &format!("{:.1}", 2.5 * x + 3.9))?;
        run.enter_grid_cell(r, 0, 1, &format!("{:.1}", 2.5 * x + 4.1))?;
        run.enter_grid_cell(r, 1, 0, &format!("{:.1}", 2.5 * x + 11.9))?;
        run.enter_grid_cell(r, 1, 1, &format!("{:.1}", 2.5 * x + 12.1))?;
    }
    let ols_id = run.run_analyze_entry("Linear", "analyze.entry.fit.linear")?;

    // Check R9 P-value precision gating (`APA` disables `edit.pvalue.digits.inc`; `Custom` enables) and toggle non-default confidence bands.
    run.select_sheet(&ols_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.options")?;
    run.click("edit.pval.apa")?;
    ensure(!widget_enabled(&run, "edit.pvalue.digits.inc")?, "R9: edit.pvalue.digits must disable under APA")?;
    run.click("edit.pval.custom")?;
    ensure(widget_enabled(&run, "edit.pvalue.digits.inc")?, "R9: edit.pvalue.digits must enable under Custom")?;
    run.click("edit.pvalue.digits.dec")?;
    run.click("edit.tab.diagnostics")?;
    run.click("edit.lin.conf_bands")?;
    run.commit_modal_ok()?;

    // Verify OLS + ANCOVA against headless oracle.
    let mut ref_xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 6).map_err(|e| e.to_string())?;
    ref_xy.data_sets[0].title = String::from("Lot A");
    ref_xy.data_sets[1].title = String::from("Lot B");
    for r in 0..6 {
        let x = (r + 1) as f64;
        ref_xy.x_cells[r][0] = Cell::number(x);
        ref_xy.data_sets[0].cells[r][0] = Cell::number(2.5 * x + 3.9);
        ref_xy.data_sets[0].cells[r][1] = Cell::number(2.5 * x + 4.1);
        ref_xy.data_sets[1].cells[r][0] = Cell::number(2.5 * x + 11.9);
        ref_xy.data_sets[1].cells[r][1] = Cell::number(2.5 * x + 12.1);
    }
    let ref_ols_sheet = DataSheet::new("ref-ols", "Standard Curve ANCOVA", TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    let ols_kind =
        serde_json::to_value(AnalysisKind::LinearRegression(LinearRegressionSpec { confidence_bands: false, ..LinearRegressionSpec::default() }))
            .map_err(|e| e.to_string())?;
    let mut ols_spec_ref = ReferenceSpec::for_result(ols_id.clone(), move |sess| {
        sess.execute(
            "analysis.run",
            json!({
                "source_sheet_id": "ref-ols",
                "kind": ols_kind,
                "p_value_format": PValueFormat::Custom { decimals: 3, scientific_below: Some(1e-4) }
            }),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .with_reference_sheet(ref_ols_sheet)?
    .scalar("Slope", Some(0), 1e-9, 1e-9)
    .scalar("YIntercept", Some(0), 1e-9, 1e-9)
    .scalar("Slope", Some(1), 1e-9, 1e-9)
    .scalar("YIntercept", Some(1), 1e-9, 1e-9);
    let ancova_tab_idx = run
        .document()
        .result_sheet(&ols_id)
        .and_then(|rs| rs.tabs.iter().position(|t| t.kind == ResultTabKind::ModelComparison))
        .ok_or("WF-16 missing ANCOVA ModelComparison tab")?;
    ols_spec_ref = ols_spec_ref.tab_cell(ancova_tab_idx, 0, 0, 1e-9, 1e-9).tab_cell(ancova_tab_idx, 1, 0, 1e-9, 1e-9);
    run.verify_reference(ols_spec_ref)?;

    // Verify R8 on Descriptive Statistics (`RawReplicates(2)` enables `edit.desc.per_subcol`).
    run.select_sheet(&xy_id)?;
    let desc_raw_id = run.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;
    run.select_sheet(&desc_raw_id)?;
    run.open_cmd_t()?;
    ensure(widget_enabled(&run, "edit.desc.per_subcol")?, "R8: RawReplicates(2) must enable edit.desc.per_subcol")?;
    run.click("edit.desc.per_subcol")?;
    run.commit_modal_ok()?;
    ensure(
        matches!(
            run.document().result_sheet(&desc_raw_id).map(|r| &r.analysis.kind),
            Some(AnalysisKind::DescriptiveStats(d)) if d.aggregation == SubcolumnAggregation::PerSubcolumn
        ),
        "R8 PerSubcolumn mode not committed",
    )?;

    // 2. Method Comparison XY table: Deming Model II + Passing-Bablok + R8 disabled on Single mode.
    let mc_id = run.create_table_via_welcome("welcome.arch.xy", "Method Comparison", &["welcome.subcol.single", "welcome.stepper.cols.dec"])?;
    let mc_rows = [(10.0, 11.2), (20.0, 21.0), (30.0, 31.5), (40.0, 40.8), (50.0, 52.1), (60.0, 61.4), (70.0, 72.0), (80.0, 81.6)];
    for (r, &(x, y)) in mc_rows.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        run.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }

    let dem_id = run.run_analyze_entry("Linear", "analyze.entry.fit.linear")?;
    run.select_sheet(&dem_id)?;
    run.open_cmd_t()?;
    run.click("edit.lin.deming")?;
    ensure(!widget_enabled(&run, "edit.lin.origin")?, "Deming Model II must disable Force through origin")?;
    run.commit_modal_ok()?;

    let mut ref_mc = XyTable::new(SubcolumnMode::Single, 1, mc_rows.len()).map_err(|e| e.to_string())?;
    for (r, &(x, y)) in mc_rows.iter().enumerate() {
        ref_mc.x_cells[r][0] = Cell::number(x);
        ref_mc.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ref_mc_sheet = DataSheet::new("ref-mc", "Method Comparison", TableArchetype::Xy(ref_mc)).map_err(|e| e.to_string())?;
    let dem_kind =
        serde_json::to_value(AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() }))
            .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(dem_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-mc", "kind": dem_kind })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_mc_sheet.clone())?
        .scalar("Slope", Some(0), 1e-9, 1e-9)
        .scalar("YIntercept", Some(0), 1e-9, 1e-9),
    )?;

    run.select_sheet(&mc_id)?;
    let pb_id = run.run_analyze_entry("Passing-Bablok", "analyze.entry.stats.passing_bablok")?;
    let pb_kind = serde_json::to_value(AnalysisKind::PassingBablok(PassingBablokSpec::default())).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(pb_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-mc", "kind": pb_kind })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_mc_sheet)?
        .scalar("Slope", None, 1e-9, 1e-9)
        .scalar("Intercept", None, 1e-9, 1e-9),
    )?;

    // R8 disabled on Single subcolumn DescriptiveStats.
    run.select_sheet(&mc_id)?;
    let desc_single_id = run.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;
    run.select_sheet(&desc_single_id)?;
    run.open_cmd_t()?;
    ensure(!widget_enabled(&run, "edit.desc.per_subcol")?, "R8: Single subcolumn table must disable edit.desc.per_subcol")?;
    run.click("modal.cancel")?;

    // R9 Random Sample Rows proportional allocation disabled without stratification on MV table.
    let mv_id = run.create_table_via_welcome("welcome.arch.multiple_variables", "MV R9 Sample", &["welcome.mv.role.1.Continuous"])?;
    for r in 0..4 {
        run.enter_grid_cell(r, 0, 0, &format!("{}", r + 1))?;
        run.enter_grid_cell(r, 1, 0, &format!("{}", (r + 1) * 2))?;
        run.enter_grid_cell(r, 2, 0, &format!("{}", (r + 1) * 3))?;
    }
    run.select_sheet(&mv_id)?;
    run.click("toolbar.sample_rows")?;
    ensure(!widget_enabled(&run, "sample.proportional_allocation")?, "R9: sample.proportional_allocation disabled without strata")?;
    run.click("modal.cancel")?;

    // Stage 5 LiveEdit on both ANCOVA XY and Method Comparison XY verifying immediate reactive updates.
    run.live_edit(LiveEditTarget::sheet_grid_cell(xy_id, 5, 0, 0, "40.0"), LiveEditExpectation::scalar_changed(ols_id, "Slope", Some(0), 0.1))?;
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mc_id, 7, 0, 0, "120.0"),
        vec![LiveEditExpectation::scalar_changed(dem_id, "Slope", Some(0), 0.05), LiveEditExpectation::scalar_changed(pb_id, "Slope", None, 0.01)],
    )?;
    run.finish()
}

#[test]
fn wf17_human_dose_response_4pl_5pl_interp_and_heatmap_r7() -> TestRes {
    let mut run = WorkflowRun::start("WF-17");

    // 1. Dose-response 4PL + unknown interpolation (`wf_datasets::dose_response_4pl` + 2 unknown Y-only rows).
    let fx4 = wf_datasets::dose_response_4pl();
    let dose4_id = run.create_table_via_welcome("welcome.arch.xy", fx4.title, &["welcome.subcol.raw", "welcome.stepper.cols.dec"])?;
    enter_xy_replicate_fixture(&mut run, &fx4)?;
    let unk_rows = [32.0, 78.0];
    for (u_idx, &uy) in unk_rows.iter().enumerate() {
        let r = fx4.rows.len() + u_idx;
        for s in 0..fx4.replicates {
            run.enter_grid_cell(r, 0, s, &format!("{uy}"))?;
        }
    }

    let r4pl_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    run.select_sheet(&r4pl_id)?;
    run.open_cmd_t()?;
    run.click("fit.tab.output")?;
    run.click("fit.out.interp")?;
    run.click("fit.out.conf_bands")?;
    run.commit_modal_ok()?;
    ensure(
        run.document().result_sheet(&r4pl_id).is_some_and(|r| r.tabs.iter().any(|t| t.kind == ResultTabKind::InterpolatedUnknowns)),
        "WF-17 4PL missing InterpolatedUnknowns tab",
    )?;

    let base_len = fx4.rows.len();
    let mut ref4_tbl =
        XyTable::new(SubcolumnMode::RawReplicates(fx4.replicates), fx4.series_titles.len(), base_len + unk_rows.len()).map_err(|e| e.to_string())?;
    for (c, title) in fx4.series_titles.iter().enumerate() {
        ref4_tbl.data_sets[c].title = (*title).to_owned();
    }
    for (r, row) in fx4.rows.iter().enumerate() {
        ref4_tbl.x_cells[r][0] = Cell::number(row.x);
        for (c, reps) in row.series_replicates.iter().enumerate() {
            for (s, &val) in reps.iter().enumerate() {
                ref4_tbl.data_sets[c].cells[r][s] = Cell::number(val);
            }
        }
    }
    for (u_idx, &uy) in unk_rows.iter().enumerate() {
        for s in 0..fx4.replicates {
            ref4_tbl.data_sets[0].cells[base_len + u_idx][s] = Cell::number(uy);
        }
    }
    let ref4 = DataSheet::new("ref-4pl", fx4.title, TableArchetype::Xy(ref4_tbl)).map_err(|e| e.to_string())?;
    let mut fit4 = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit4.interpolate_unknowns = true;
    fit4.confidence_bands = true;
    let fit4_val = serde_json::to_value(AnalysisKind::NonlinearRegression(fit4)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(r4pl_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-4pl", "kind": fit4_val })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref4)?
        .scalar("LogEC50", Some(0), 1e-6, 1e-6)
        .scalar("HillSlope", Some(0), 1e-6, 1e-6)
        .scalar("LogEC50", Some(1), 1e-6, 1e-6),
    )?;

    // 2. Asymmetric 5PL (`wf_datasets::dose_response_5pl`).
    let fx5 = wf_datasets::dose_response_5pl();
    let dose5_id = run.create_table_via_welcome("welcome.arch.xy", fx5.title, &["welcome.subcol.raw", "welcome.stepper.cols.dec"])?;
    enter_xy_replicate_fixture(&mut run, &fx5)?;
    let r5pl_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    run.select_sheet(&r5pl_id)?;
    run.open_cmd_t()?;
    run.click("fit.eq.search")?;
    run.type_text("5pl")?;
    run.click("fit.eq.item.log_agonist_vs_response_5pl")?;
    run.commit_modal_ok()?;
    let ref5 = fx5.to_data_sheet("ref-5pl").map_err(|e| e.to_string())?;
    let fit5_val =
        serde_json::to_value(AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_5pl"))).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(r5pl_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-5pl", "kind": fit5_val })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref5)?
        .scalar("LogEC50", Some(0), 1e-6, 1e-6)
        .scalar("HillSlope", Some(0), 1e-6, 1e-6)
        .scalar("S", Some(0), 1e-6, 1e-6),
    )?;

    // 3. Correlation Matrix + Heatmap + R7 predicate (`analyze.correlation.*` and `edit.corr.*`).
    let mv_id = run.create_table_via_welcome("welcome.arch.multiple_variables", "Biomarker Correlations", &["welcome.mv.role.1.Continuous"])?;
    run.enter_column_title(0, "VarA")?;
    run.enter_column_title(1, "VarB")?;
    run.enter_column_title(2, "VarC")?;
    let corr_rows = [[1.0, 2.2, 10.0], [2.0, 4.1, 8.2], [3.0, 6.3, 6.1], [4.0, 7.9, 4.3], [5.0, 10.2, 2.1], [6.0, 11.8, 0.4]];
    for (r, vals) in corr_rows.iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }

    let graphs_before: Vec<SheetId> = run.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
    run.select_sheet(&mv_id)?;
    run.click("toolbar.analyze")?;
    run.click("analyze.search")?;
    run.type_text("Correlation Matrix")?;
    run.click("analyze.entry.stats.corr_matrix")?;
    ensure(widget_enabled(&run, "analyze.correlation.heatmap")?, "R7: Analyze heatmap enabled when all_pairs=true")?;
    run.click("analyze.correlation_all_pairs")?;
    ensure(!widget_enabled(&run, "analyze.correlation.heatmap")?, "R7: Analyze heatmap disabled when all_pairs=false")?;
    run.click("analyze.correlation_all_pairs")?;
    ensure(widget_enabled(&run, "analyze.correlation.heatmap")?, "R7: Analyze heatmap re-enabled when all_pairs=true")?;
    run.click("analyze.correlation.heatmap")?;
    run.commit_modal_ok()?;
    let corr_rs_id = run
        .document()
        .result_sheets
        .iter()
        .rev()
        .find(|r| {
            r.analysis.source_sheet_id == mv_id && matches!(r.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
        })
        .map(|r| r.id.clone())
        .ok_or("WF-17 missing Correlation Matrix ResultSheet")?;

    // Switch to Spearman (non-default) and verify R7 in EditAnalysisParameters + spawn Heatmap GraphSheet.
    run.select_sheet(&corr_rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.corr.spearman")?;
    ensure(widget_enabled(&run, "edit.corr.heatmap")?, "R7: EditParams heatmap enabled when all_pairs=true")?;
    run.click("edit.corr.all_pairs")?;
    ensure(!widget_enabled(&run, "edit.corr.heatmap")?, "R7: EditParams heatmap disabled when all_pairs=false")?;
    run.click("edit.corr.all_pairs")?;
    ensure(widget_enabled(&run, "edit.corr.heatmap")?, "R7: EditParams heatmap re-enabled when all_pairs=true")?;
    if !widget_value_bool(&run, "edit.corr.heatmap")? {
        run.click("edit.corr.heatmap")?;
    }
    run.commit_modal_ok()?;

    let hm_gs_id = run
        .document()
        .graph_sheets
        .iter()
        .find(|g| !graphs_before.contains(&g.id) && g.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap))
        .map(|g| g.id.clone())
        .ok_or("WF-17 missing newly generated Correlation Heatmap GraphSheet")?;

    let mut ref_mv = MultipleVariablesTable::new(3, corr_rows.len()).map_err(|e| e.to_string())?;
    for (c, title) in ["VarA", "VarB", "VarC"].iter().enumerate() {
        ref_mv.variables[c].title = (*title).to_owned();
        ref_mv.variables[c].role = VariableRole::Continuous;
    }
    let mut ref_mv_sheet =
        DataSheet::new("ref-mv-corr", "Biomarker Correlations", TableArchetype::MultipleVariables(ref_mv)).map_err(|e| e.to_string())?;
    for (r, vals) in corr_rows.iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            ref_mv_sheet.set_cell(CellCoord::new(r, c, 0), Cell::number(v)).map_err(|e| e.to_string())?;
        }
    }
    let corr_val =
        serde_json::to_value(AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: true })).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(corr_rs_id.clone(), move |sess| {
            let out = sess.execute("analysis.run", json!({ "source_sheet_id": "ref-mv-corr", "kind": corr_val })).map_err(|e| e.to_string())?;
            let ref_corr_id = out.payload["sheet_id"].as_str().ok_or("missing ref corr sheet_id")?;
            sess.execute(
                "graph.create",
                json!({
                    "data_sheet_id": ref_corr_id,
                    "kind": GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap),
                    "initialized": true
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_mv_sheet)?
        .tab_cell(0, 0, 1, 1e-9, 1e-9)
        .tab_cell(0, 0, 2, 1e-9, 1e-9)
        .with_graph(hm_gs_id.clone())
        .check_graph(|gui_gs, _ref_gs| ensure(gui_gs.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap), "expected Heatmap GraphKind")),
    )?;

    // Stage 5 LiveEdit across both 4PL curve and Correlation Matrix + Heatmap SVG.
    run.live_edit(
        LiveEditTarget::sheet_grid_cell(dose4_id, 4, 0, 0, "88.0"),
        LiveEditExpectation::scalar_changed(r4pl_id, "LogEC50", Some(0), 1e-3),
    )?;
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mv_id, 2, 1, 0, "1.0"),
        vec![LiveEditExpectation::tab_cell_changed(corr_rs_id, 0, 0, 1), LiveEditExpectation::graph_svg_changed(hm_gs_id)],
    )?;
    let _ = dose5_id;
    let _ = r5pl_id;
    run.finish()
}

#[test]
fn wf18_human_enzyme_kinetics_global_vmax_diagnostics() -> TestRes {
    let mut run = WorkflowRun::start("WF-18");
    let fx = wf_datasets::michaelis_menten();
    let enz_id = run.create_table_via_welcome(
        "welcome.arch.xy",
        fx.title,
        &["welcome.subcol.raw", "welcome.stepper.replicates.dec", "welcome.stepper.cols.dec"],
    )?;
    enter_xy_replicate_fixture(&mut run, &fx)?;

    let rs_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("fit.eq.search")?;
    run.type_text("michaelis")?;
    run.click("fit.eq.item.michaelis_menten")?;
    run.click("fit.tab.constraints")?;
    run.click("fit.con.rule.Vmax.shared")?;
    run.click("fit.tab.output")?;
    run.click("fit.out.normality")?;
    run.click("fit.out.cov_tab")?;
    if !widget_value_bool(&run, "fit.out.residuals_tab")? {
        run.click("fit.out.residuals_tab")?;
    }
    if !widget_value_bool(&run, "fit.out.conf_bands")? {
        run.click("fit.out.conf_bands")?;
    }
    run.commit_modal_ok()?;

    {
        let rs = run.document().result_sheet(&rs_id).ok_or("WF-18 missing ResultSheet")?;
        ensure(
            matches!(
                &rs.analysis.kind,
                AnalysisKind::NonlinearRegression(f)
                    if f.equation_id == "michaelis_menten"
                        && f.parameter_rules.iter().any(|r| r.name == "Vmax" && r.scope == ParameterScope::SharedGlobal)
                        && f.normality_of_residuals
                        && f.emit_residuals_tab
                        && f.emit_covariance_tab
                        && f.confidence_bands
            ),
            "WF-18 global Vmax / diagnostics spec mismatch",
        )?;
        ensure(rs.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals), "WF-18 missing Residuals tab")?;
        ensure(rs.tabs.iter().any(|t| t.kind == ResultTabKind::CovarianceMatrix), "WF-18 missing CovarianceMatrix tab")?;
    }

    let ref_enz = fx.to_data_sheet("ref-mm").map_err(|e| e.to_string())?;
    let mut fit_mm = NonlinearFitSpec::new("michaelis_menten");
    fit_mm.parameter_rules = vec![ParameterRule {
        name: String::from("Vmax"),
        scope: ParameterScope::SharedGlobal,
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }];
    fit_mm.normality_of_residuals = true;
    fit_mm.emit_residuals_tab = true;
    fit_mm.emit_covariance_tab = true;
    fit_mm.confidence_bands = true;
    let mm_val = serde_json::to_value(AnalysisKind::NonlinearRegression(fit_mm)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-mm", "kind": mm_val })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_enz)?
        .scalar("Vmax", None, 1e-6, 1e-6)
        .scalar("Km", Some(0), 1e-6, 1e-6)
        .scalar("Km", Some(1), 1e-6, 1e-6),
    )?;

    run.live_edit(LiveEditTarget::sheet_grid_cell(enz_id, 7, 0, 0, "160.0"), LiveEditExpectation::scalar_changed(rs_id, "Km", Some(0), 0.1))?;
    run.finish()
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
