#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-19`–`WF-24`, `R15.4`/`R15.5`).

mod human_wf_common;

use curvebook_chart::{PathCmd, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ClusterLinkage, ColumnDesignHint, ContingencyMethod, ContingencySpec, DataSheet, DiagnosticPlotKind, GlmFamily,
    GlmSpec, GraphAxisScale, GraphKind, GroupedArrangement, GroupedTable, MultipleRowTestsSpec, MultipleVariablesTable, MultivariateSpec,
    OneWayAnovaSpec, ProjectDocument, ResultCell, ResultSheet, ResultTabKind, RocCiMethod, RocSpec, SheetId, SubcolumnMode, SurvivalAnalysisSpec,
    TableArchetype, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, VariableRole, XyTable,
};
use curvebook_palette::Rgba8;
use curvebook_testkit::wf_datasets;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use serde_json::json;

use human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use human_wf_common::{TestRes, ensure};

fn widget_enabled(run: &WorkflowRun, wid: &str) -> Result<bool, String> {
    run.frame_meta().find_by_widget_id(wid).map(|n| n.enabled).ok_or_else(|| format!("missing widget `{wid}`"))
}

fn widget_value_bool(run: &WorkflowRun, wid: &str) -> Result<bool, String> {
    let node = run.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing widget `{wid}`"))?;
    node.value.as_ref().and_then(|v| v.as_bool()).ok_or_else(|| format!("widget `{wid}` has non-bool value: {:?}", node.value))
}

fn enter_contingency_fixture(run: &mut WorkflowRun, fx: &wf_datasets::ContingencyFixture) -> TestRes {
    for (c, title) in fx.column_titles.iter().enumerate() {
        run.enter_column_title(c, title)?;
    }
    for (r, row) in fx.counts.iter().enumerate() {
        if let Some(rt) = fx.row_titles.get(r) {
            run.enter_row_title(r, rt)?;
        }
        for (c, &count) in row.iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{count}"))?;
        }
    }
    Ok(())
}

fn enter_mv_fixture(run: &mut WorkflowRun, fx: &wf_datasets::MvFixture) -> TestRes {
    for (c, col) in fx.columns.iter().enumerate() {
        run.enter_column_title(c, col.title)?;
    }
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let txt = match cell {
                wf_datasets::MvCellValue::Number(v) => format!("{v}"),
                wf_datasets::MvCellValue::Text(s) => (*s).to_owned(),
            };
            run.enter_grid_cell(r, c, 0, &txt)?;
        }
    }
    Ok(())
}

fn scalar_val(rs: &ResultSheet, key: &str) -> Result<f64, String> {
    rs.scalars
        .iter()
        .find(|s| s.key.eq_ignore_ascii_case(key))
        .map(|s| s.value)
        .or_else(|| {
            let key_lower = key.to_ascii_lowercase();
            rs.tabs.iter().find_map(|tab| {
                let r_idx = tab
                    .row_labels
                    .iter()
                    .position(|l| l.eq_ignore_ascii_case(key))
                    .or_else(|| tab.row_labels.iter().position(|l| l.to_ascii_lowercase().contains(&key_lower)))?;
                tab.rows.get(r_idx)?.iter().find_map(|c| match c {
                    ResultCell::Number { value, .. } => Some(*value),
                    _ => None,
                })
            })
        })
        .ok_or_else(|| format!("missing scalar `{key}`"))
}

fn find_new_graph_for_source(run: &WorkflowRun, before: &[SheetId], source_id: &SheetId) -> Result<SheetId, String> {
    run.document()
        .graph_sheets
        .iter()
        .find(|g| !before.contains(&g.id) && g.spec.primary_source_sheet_id == *source_id)
        .map(|g| g.id.clone())
        .ok_or_else(|| format!("missing newly created GraphSheet for source `{}`", source_id.as_str()))
}

#[test]
fn wf19_human_contingency_2x2_fisher_r5_r6_and_rxc_chi_square() -> TestRes {
    let mut run = WorkflowRun::start("WF-19");

    // 1. Strict 2x2 table -> Fisher's Exact + R5 effect sizes + R6 CI method selectors.
    let fx2 = wf_datasets::contingency_2x2();
    let mut extra2 = vec!["welcome.stepper.cols.dec"];
    extra2.extend(std::iter::repeat_n("welcome.stepper.rows.dec", 10));
    let ds2_id = run.create_table_via_welcome("welcome.arch.contingency", fx2.title, &extra2)?;
    enter_contingency_fixture(&mut run, &fx2)?;

    let rs2_id = run.run_analyze_entry("Contingency", "analyze.entry.stats.contingency")?;
    run.select_sheet(&rs2_id)?;
    run.open_cmd_t()?;
    run.click("edit.cont.fisher")?;
    ensure(widget_enabled(&run, "edit.cont.or_rr")?, "R5: edit.cont.or_rr must be enabled for 2x2")?;
    ensure(widget_enabled(&run, "edit.cont.sens_spec")?, "R5: edit.cont.sens_spec must be enabled for 2x2")?;
    if !widget_value_bool(&run, "edit.cont.or_rr")? {
        run.click("edit.cont.or_rr")?;
    }
    if !widget_value_bool(&run, "edit.cont.sens_spec")? {
        run.click("edit.cont.sens_spec")?;
    }
    // Verify R6 gates `newcombe_wilson` on attributable risk, then enable attributable risk and verify all R6 CI selectors.
    if widget_value_bool(&run, "edit.contingency.attributable_risk")? {
        run.click("edit.contingency.attributable_risk")?;
    }
    ensure(!widget_enabled(&run, "edit.contingency.ci.newcombe_wilson")?, "R6: newcombe_wilson must disable when attributable risk is off")?;
    run.click("edit.contingency.attributable_risk")?;
    ensure(widget_value_bool(&run, "edit.contingency.attributable_risk")?, "edit.contingency.attributable_risk must check on")?;
    ensure(widget_enabled(&run, "edit.contingency.ci.baptista_pike")?, "R6: edit.contingency.ci.baptista_pike must be enabled")?;
    ensure(widget_enabled(&run, "edit.contingency.ci.koopman")?, "R6: edit.contingency.ci.koopman must be enabled")?;
    ensure(widget_enabled(&run, "edit.contingency.ci.newcombe_wilson")?, "R6: edit.contingency.ci.newcombe_wilson must be enabled")?;
    ensure(widget_enabled(&run, "edit.contingency.ci.wilson_brown")?, "R6: edit.contingency.ci.wilson_brown must be enabled")?;
    let baptista = &run.frame_meta().find_by_widget_id("edit.contingency.ci.baptista_pike").ok_or("missing R6 Baptista-Pike")?.label;
    ensure(baptista.contains("Baptista-Pike"), format!("expected Baptista-Pike CI label, got `{baptista}`"))?;
    let koopman = &run.frame_meta().find_by_widget_id("edit.contingency.ci.koopman").ok_or("missing R6 Koopman")?.label;
    ensure(koopman.contains("Koopman"), format!("expected Koopman CI label, got `{koopman}`"))?;
    let newcombe = &run.frame_meta().find_by_widget_id("edit.contingency.ci.newcombe_wilson").ok_or("missing R6 Newcombe-Wilson")?.label;
    ensure(newcombe.contains("Newcombe-Wilson"), format!("expected Newcombe-Wilson CI label, got `{newcombe}`"))?;
    run.commit_modal_ok()?;
    {
        let rs2 = run.document().result_sheet(&rs2_id).ok_or("missing WF-19 2x2 ResultSheet")?;
        ensure(
            matches!(rs2.analysis.kind, AnalysisKind::Contingency(ref c) if c.method == ContingencyMethod::FisherExact),
            "WF-19 2x2 must strictly use ContingencyMethod::FisherExact",
        )?;
    }

    let ref2 = fx2.to_data_sheet("ref-cont-2x2").map_err(|e| e.to_string())?;
    let kind2 = serde_json::to_value(AnalysisKind::Contingency(ContingencySpec {
        method: ContingencyMethod::FisherExact,
        compute_odds_and_risk_ratios: true,
        compute_sensitivity_specificity: true,
        ..ContingencySpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(rs2_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-cont-2x2", "kind": kind2 })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref2)?
        .scalar("Fisher's exact P", None, 1e-12, 1e-12)
        .scalar("Odds ratio", None, 1e-9, 1e-9)
        .scalar("Relative risk", None, 1e-9, 1e-9)
        .scalar("Risk difference", None, 1e-9, 1e-9),
    )?;

    // 2. Separate RxC (3x3) table -> strict Pearson Chi-square + R5/R6 disabled on > 2x2.
    let fx3 = wf_datasets::contingency_3x3();
    let ds3_id = run.create_table_via_welcome("welcome.arch.contingency", fx3.title, &["welcome.stepper.cols.inc", "welcome.stepper.rows.inc"])?;
    enter_contingency_fixture(&mut run, &fx3)?;
    let rs3_id = run.run_analyze_entry("Contingency", "analyze.entry.stats.contingency")?;
    run.select_sheet(&rs3_id)?;
    run.open_cmd_t()?;
    ensure(!widget_enabled(&run, "edit.cont.or_rr")?, "R5: edit.cont.or_rr must be disabled on 3x3")?;
    ensure(!widget_enabled(&run, "edit.cont.sens_spec")?, "R5: edit.cont.sens_spec must be disabled on 3x3")?;
    ensure(!widget_enabled(&run, "edit.contingency.ci.baptista_pike")?, "R6: OR CI must be disabled on 3x3")?;
    ensure(!widget_enabled(&run, "edit.contingency.ci.koopman")?, "R6: RR CI must be disabled on 3x3")?;
    ensure(!widget_enabled(&run, "edit.contingency.ci.newcombe_wilson")?, "R6: RD CI must be disabled on 3x3")?;
    ensure(!widget_enabled(&run, "edit.contingency.ci.wilson_brown")?, "R6: Diag CI must be disabled on 3x3")?;
    run.click("edit.cont.chisq")?;
    run.commit_modal_ok()?;
    {
        let rs3 = run.document().result_sheet(&rs3_id).ok_or("missing WF-19 3x3 ResultSheet")?;
        ensure(
            matches!(rs3.analysis.kind, AnalysisKind::Contingency(ref c) if matches!(c.method, ContingencyMethod::PearsonChiSquare { yates_continuity: false })),
            "WF-19 3x3 must strictly use PearsonChiSquare",
        )?;
    }

    let ref3 = fx3.to_data_sheet("ref-cont-3x3").map_err(|e| e.to_string())?;
    let kind3 = serde_json::to_value(AnalysisKind::Contingency(ContingencySpec {
        method: ContingencyMethod::PearsonChiSquare { yates_continuity: false },
        ..ContingencySpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(rs3_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-cont-3x3", "kind": kind3 })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref3)?
        .scalar("Chi-square", None, 1e-9, 1e-9)
        .scalar("Chi-square P value", None, 1e-12, 1e-12)
        .scalar("Cramer's V", None, 1e-9, 1e-9),
    )?;

    run.live_edit(
        LiveEditTarget::sheet_grid_cell(ds2_id, 0, 0, 0, "8"),
        LiveEditExpectation::scalar_changed(rs2_id, "Fisher's exact P", None, 1e-4),
    )?;
    run.live_edit(LiveEditTarget::sheet_grid_cell(ds3_id, 0, 0, 0, "5"), LiveEditExpectation::scalar_changed(rs3_id, "Chi-square", None, 0.5))?;
    run.finish()
}

#[test]
fn wf20_human_roc_curve_and_cochran_armitage_trend() -> TestRes {
    let mut run = WorkflowRun::start("WF-20");

    // 1. Column table for ROC Curve (`AUC`, Youden's J, Hanley-McNeil non-default CI, ROC GraphSheet).
    let graphs_before: Vec<SheetId> = run.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
    let roc_col_id = run.create_table_via_welcome("welcome.arch.column", "ROC Controls Cases", &["welcome.stepper.cols.dec"])?;
    let roc_gs_id = find_new_graph_for_source(&run, &graphs_before, &roc_col_id)?;
    let roc_rows = [[10.0, 22.0], [12.0, 25.0], [14.0, 29.0], [15.5, 31.0], [17.0, 34.0], [18.5, 16.0], [20.0, 38.0], [23.0, 42.0]];
    run.enter_column_title(0, "Control")?;
    run.enter_column_title(1, "Diseased")?;
    for (r, vals) in roc_rows.iter().enumerate() {
        run.enter_grid_cell(r, 0, 0, &format!("{}", vals[0]))?;
        run.enter_grid_cell(r, 1, 0, &format!("{}", vals[1]))?;
    }

    let roc_rs_id = run.run_analyze_entry("ROC", "analyze.entry.stats.roc")?;
    run.select_sheet(&roc_rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.roc.hanley")?;
    run.commit_modal_ok()?;
    {
        let rs = run.document().result_sheet(&roc_rs_id).ok_or("WF-20 missing ROC ResultSheet")?;
        ensure(matches!(rs.analysis.kind, AnalysisKind::RocCurve(ref r) if r.ci_method == RocCiMethod::HanleyMcNeil), "expected HanleyMcNeil ROC")?;
        ensure(rs.tabs.iter().any(|t| t.kind == ResultTabKind::CurveCoordinates), "WF-20 missing ROC CurveCoordinates tab")?;
    }

    // Switch linked GraphSheet to Diagnostic ROC Curve via Change Graph Type modal.
    run.select_sheet(&roc_gs_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.show_all")?;
    run.click("graph_type.card.diag.roc")?;
    run.commit_modal_ok()?;

    let mut ref_roc_tbl = curvebook_doc::ColumnTable::new(ColumnDesignHint::default(), 2, roc_rows.len()).map_err(|e| e.to_string())?;
    ref_roc_tbl.data_sets[0].title = String::from("Control");
    ref_roc_tbl.data_sets[1].title = String::from("Diseased");
    for (r, vals) in roc_rows.iter().enumerate() {
        ref_roc_tbl.data_sets[0].cells[r][0] = Cell::number(vals[0]);
        ref_roc_tbl.data_sets[1].cells[r][0] = Cell::number(vals[1]);
    }
    let mut ref_roc_doc = ProjectDocument::new("WF-20 ROC Oracle").map_err(|e| e.to_string())?;
    let ref_roc_ds = DataSheet::new("ref-roc-col", "ROC Controls Cases", TableArchetype::Column(ref_roc_tbl)).map_err(|e| e.to_string())?;
    ref_roc_doc.add_data_sheet(ref_roc_ds).map_err(|e| e.to_string())?;
    let roc_kind = serde_json::to_value(AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::HanleyMcNeil })).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(roc_rs_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-roc-col", "kind": roc_kind })).map_err(|e| e.to_string())?;
            sess.execute(
                "graph.create",
                json!({
                    "data_sheet_id": "ref-roc-col",
                    "kind": GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve),
                    "initialized": true
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_document(ref_roc_doc)
        .scalar("Area under ROC curve (AUC)", None, 1e-9, 1e-9)
        .scalar("Max Youden J", None, 1e-9, 1e-9)
        .scalar("Optimal cutoff (Youden J)", None, 1e-9, 1e-9)
        .with_graph(roc_gs_id.clone())
        .check_graph(|gui_gs, _ref_gs| {
            ensure(gui_gs.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve), "WF-20 expected Diagnostic RocCurve GraphKind")
        }),
    )?;

    // 2. 2x3 ordered Contingency table (`wf_datasets::contingency_2x3_trend`) -> R5 enables Cochran-Armitage trend.
    let fx_trend = wf_datasets::contingency_2x3_trend();
    let mut extra_trend = vec!["welcome.stepper.cols.inc"];
    extra_trend.extend(std::iter::repeat_n("welcome.stepper.rows.dec", 10));
    let trend_ds_id = run.create_table_via_welcome("welcome.arch.contingency", fx_trend.title, &extra_trend)?;
    enter_contingency_fixture(&mut run, &fx_trend)?;
    let trend_rs_id = run.run_analyze_entry("Contingency", "analyze.entry.stats.contingency")?;
    run.select_sheet(&trend_rs_id)?;
    run.open_cmd_t()?;
    ensure(widget_enabled(&run, "edit.cont.trend")?, "R5: edit.cont.trend must be enabled for 2x3 ordered table")?;
    ensure(!widget_enabled(&run, "edit.cont.or_rr")?, "R5: edit.cont.or_rr must be disabled for 2x3 table")?;
    run.click("edit.cont.trend")?;
    run.commit_modal_ok()?;
    {
        let rs_t = run.document().result_sheet(&trend_rs_id).ok_or("WF-20 missing Trend ResultSheet")?;
        ensure(
            matches!(rs_t.analysis.kind, AnalysisKind::Contingency(ref c) if c.method == ContingencyMethod::CochranArmitageTrend),
            "WF-20 expected CochranArmitageTrend",
        )?;
    }

    let ref_trend = fx_trend.to_data_sheet("ref-cont-trend").map_err(|e| e.to_string())?;
    let trend_kind = serde_json::to_value(AnalysisKind::Contingency(ContingencySpec {
        method: ContingencyMethod::CochranArmitageTrend,
        ..ContingencySpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(trend_rs_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-cont-trend", "kind": trend_kind })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_trend)?
        .tab_cell(0, 0, 0, 1e-9, 1e-9)
        .tab_cell(0, 0, 2, 1e-12, 1e-12)
        .scalar("Chi-square", None, 1e-9, 1e-9),
    )?;

    // Stage 5 LiveEdit on both ROC source & 2x3 trend table.
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(roc_col_id, 5, 1, 0, "36.0"),
        vec![
            LiveEditExpectation::scalar_changed(roc_rs_id, "Area under ROC curve (AUC)", None, 0.01),
            LiveEditExpectation::graph_svg_changed(roc_gs_id),
        ],
    )?;
    run.live_edit(LiveEditTarget::sheet_grid_cell(trend_ds_id, 0, 2, 0, "2"), LiveEditExpectation::tab_cell_changed(trend_rs_id, 0, 0, 0))?;
    run.finish()
}

#[test]
fn wf21_human_kaplan_meier_survival_logistic_and_r4() -> TestRes {
    let mut run = WorkflowRun::start("WF-21");

    // 1. Kaplan-Meier Survival (`wf_datasets::survival_two_group` = 16 rows x 2 cohorts).
    let surv_fx = wf_datasets::survival_two_group();
    let surv_extra =
        ["welcome.stepper.cols.dec", "welcome.stepper.rows.inc", "welcome.stepper.rows.inc", "welcome.stepper.rows.inc", "welcome.stepper.rows.inc"];
    let graphs_before: Vec<SheetId> = run.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
    let surv_id = run.create_table_via_welcome("welcome.arch.survival", surv_fx.title, &surv_extra)?;
    let km_gs_id = find_new_graph_for_source(&run, &graphs_before, &surv_id)?;
    for (c, title) in surv_fx.cohort_titles.iter().enumerate() {
        run.enter_column_title(c, title)?;
    }
    for (r, (time, cells)) in surv_fx.rows.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{time}"))?;
        for (c, st_opt) in cells.iter().enumerate() {
            if let Some(st) = st_opt {
                run.enter_grid_cell(r, c, 0, &format!("{st}"))?;
            }
        }
    }

    let km_rs_id = run.run_analyze_entry("Kaplan", "analyze.entry.stats.survival_km")?;
    run.select_sheet(&km_rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.km.gehan")?;
    run.commit_modal_ok()?;
    {
        let rs = run.document().result_sheet(&km_rs_id).ok_or("WF-21 missing KM ResultSheet")?;
        ensure(
            matches!(
                rs.analysis.kind,
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { log_rank_mantel_cox: true, gehan_breslow_wilcoxon: true, .. })
            ),
            "WF-21 expected KM with Mantel-Cox + Gehan-Breslow-Wilcoxon",
        )?;
        ensure(rs.tabs.iter().any(|t| t.kind == ResultTabKind::NumberAtRisk), "WF-21 missing NumberAtRisk tab")?;
    }

    // Verify Number-at-Risk track renders when viewing KM GraphSheet.
    run.select_sheet(&km_gs_id)?;
    ensure(run.frame_meta().find_by_widget_id("canvas.km_number_at_risk").is_some(), "WF-21 KM canvas missing Number-at-Risk track")?;

    let ref_surv = surv_fx.to_data_sheet("ref-surv-2").map_err(|e| e.to_string())?;
    let km_kind = serde_json::to_value(AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
        log_rank_mantel_cox: true,
        gehan_breslow_wilcoxon: true,
        trend_test: false,
        asymmetrical_greenwood_ci: false,
        multiplicity: Default::default(),
        mv_time_column: None,
        mv_status_column: None,
        mv_group_column: None,
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(km_rs_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-surv-2", "kind": km_kind })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_surv)?
        .scalar("Log-rank P value", None, 1e-12, 1e-12)
        .scalar("Hazard ratio (log-rank)", None, 1e-9, 1e-9)
        .scalar("Log-rank chi-square", None, 1e-9, 1e-9),
    )?;

    // 2. Multiple Logistic Regression (`wf_datasets::glm_logistic`, 16 rows x 4 cols).
    let log_fx = wf_datasets::glm_logistic();
    let mv_extra = ["welcome.stepper.cols.inc", "welcome.stepper.cols.inc", "welcome.mv.role.1.Continuous", "welcome.mv.role.3.Categorical"];
    let log_mv_id = run.create_table_via_welcome("welcome.arch.multiple_variables", log_fx.title, &mv_extra)?;
    enter_mv_fixture(&mut run, &log_fx)?;

    let glm_rs_id = run.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    run.select_sheet(&glm_rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.glm.logistic")?;
    run.click("edit.tab.data")?;
    run.click("edit.glm.resp.0")?;
    if !widget_value_bool(&run, "edit.glm.pred.1")? {
        run.click("edit.glm.pred.1")?;
    }
    if !widget_value_bool(&run, "edit.glm.pred.2")? {
        run.click("edit.glm.pred.2")?;
    }
    if !widget_value_bool(&run, "edit.glm.pred.3")? {
        run.click("edit.glm.pred.3")?;
    }
    run.commit_modal_ok()?;
    {
        let rs_glm = run.document().result_sheet(&glm_rs_id).ok_or("WF-21 missing Logistic ResultSheet")?;
        ensure(
            matches!(rs_glm.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Logistic),
            "WF-21 expected GlmFamily::Logistic",
        )?;
    }

    let ref_log = log_fx.to_data_sheet("ref-glm-log").map_err(|e| e.to_string())?;
    let glm_kind = serde_json::to_value(AnalysisKind::GeneralizedLinearModel(GlmSpec {
        family: GlmFamily::Logistic,
        response_column: 0,
        predictor_columns: vec![1, 2, 3],
        reference_levels: Vec::new(),
        compute_vif: true,
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(glm_rs_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-glm-log", "kind": glm_kind })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_log)?
        .scalar("McFadden R squared", None, 1e-6, 1e-6)
        .scalar("Nagelkerke R squared", None, 1e-6, 1e-6),
    )?;

    // 3. R4 predicate checks on Two-Group Comparison (`paired` disables Welch; `Welch` disables Diagnostics tab).
    let r4_col_id = run.create_table_via_welcome("welcome.arch.column", "R4 Two-Group", &["welcome.stepper.cols.dec", "welcome.stepper.cols.dec"])?;
    for (r, (a, b)) in [(10.0, 14.1), (11.0, 15.6), (12.0, 15.7), (13.0, 17.8), (14.0, 19.3)].iter().enumerate() {
        run.enter_grid_cell(r, 0, 0, &format!("{a}"))?;
        run.enter_grid_cell(r, 1, 0, &format!("{b}"))?;
    }
    let r4_rs_id = run.run_analyze_entry("t tests", "analyze.entry.stats.two_group")?;
    run.select_sheet(&r4_rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.paired_or_matched")?;
    ensure(!widget_enabled(&run, "edit.two_group.welch")?, "R4: paired mode must disable Welch correction")?;
    run.click("edit.paired_or_matched")?;
    run.click("edit.two_group.welch")?;
    ensure(!widget_enabled(&run, "edit.tab.diagnostics")?, "R4: Welch t-test must disable Two-Group Diagnostics tab")?;
    run.commit_modal_ok()?;
    ensure(
        matches!(
            run.document().result_sheet(&r4_rs_id).map(|r| &r.analysis.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, .. }))
        ),
        "R4 expected WelchT committed",
    )?;
    let _ = r4_col_id;

    // Stage 5 LiveEdit on Survival & Logistic MV inputs.
    run.live_edit(
        LiveEditTarget::sheet_grid_cell(surv_id, 1, 1, 0, "1"),
        LiveEditExpectation::scalar_changed(km_rs_id, "Log-rank P value", None, 1e-4),
    )?;
    run.live_edit(
        LiveEditTarget::sheet_grid_cell(log_mv_id, 2, 1, 0, "8.2"),
        LiveEditExpectation::scalar_changed(glm_rs_id, "McFadden R squared", None, 1e-3),
    )?;
    run.finish()
}

#[test]
fn wf22_human_pca_clustering_glm_volcano_format_points() -> TestRes {
    let mut run = WorkflowRun::start("WF-22");

    // 1. Multi-Omic 4-col MV table -> PCA + Hierarchical Clustering + Multiple Linear GLM.
    let mv_id = run.create_table_via_welcome(
        "welcome.arch.multiple_variables",
        "Multi-Omic Cohort",
        &["welcome.stepper.cols.inc", "welcome.mv.role.1.Continuous"],
    )?;
    run.enter_column_title(0, "Feature1")?;
    run.enter_column_title(1, "Feature2")?;
    run.enter_column_title(2, "Feature3")?;
    run.enter_column_title(3, "Outcome")?;

    let mut mv_rows = Vec::new();
    for r in 0..10 {
        let f1 = (r as f64) + 1.0;
        let f2 = 2.0 * f1 + if r % 2 == 0 { 0.3 } else { -0.3 };
        let f3 = (12.0 - f1) + ((r % 3) as f64) * 0.4;
        let y = 3.0 * f1 - 0.5 * f2 + 0.8 * f3 + 1.0;
        mv_rows.push([f1, f2, f3, y]);
        for (c, &v) in mv_rows[r].iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{v:.2}"))?;
        }
    }

    // 1a. PCA (switch non-default to Covariance matrix).
    let pca_id = run.run_analyze_entry("Principal", "analyze.entry.stats.pca")?;
    run.select_sheet(&pca_id)?;
    run.open_cmd_t()?;
    run.click("edit.pca.cov")?;
    run.commit_modal_ok()?;

    // 1b. Hierarchical Clustering (switch non-default linkage to Complete).
    run.select_sheet(&mv_id)?;
    let clust_id = run.run_analyze_entry("Hierarchical Clustering", "analyze.entry.stats.clustering")?;
    run.select_sheet(&clust_id)?;
    run.open_cmd_t()?;
    run.click("edit.cluster.link.complete")?;
    run.commit_modal_ok()?;
    ensure(
        run.document().result_sheet(&clust_id).is_some_and(|r| r.tabs.iter().any(|t| t.id == "hierarchical-dendrogram")),
        "WF-22 missing dendrogram tab",
    )?;

    // 1c. Multiple Linear GLM (`Outcome` col 3 predicted by cols 0, 1, 2).
    run.select_sheet(&mv_id)?;
    let glm_id = run.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    run.select_sheet(&glm_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.data")?;
    run.click("edit.glm.resp.3")?;
    for p_wid in ["edit.glm.pred.0", "edit.glm.pred.1", "edit.glm.pred.2"] {
        if !widget_value_bool(&run, p_wid)? {
            run.click(p_wid)?;
        }
    }
    run.commit_modal_ok()?;

    let mut ref_mv = MultipleVariablesTable::new(4, mv_rows.len()).map_err(|e| e.to_string())?;
    for (c, title) in ["Feature1", "Feature2", "Feature3", "Outcome"].iter().enumerate() {
        ref_mv.variables[c].title = (*title).to_owned();
        ref_mv.variables[c].role = VariableRole::Continuous;
    }
    let mut ref_mv_ds = DataSheet::new("ref-wf22-mv", "Multi-Omic Cohort", TableArchetype::MultipleVariables(ref_mv)).map_err(|e| e.to_string())?;
    for (r, row) in mv_rows.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            let rounded = (v * 100.0).round() / 100.0;
            ref_mv_ds.set_cell(CellCoord::new(r, c, 0), Cell::number(rounded)).map_err(|e| e.to_string())?;
        }
    }
    let pca_val = serde_json::to_value(AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: false, parallel_analysis_seed: Some(7) }))
        .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(pca_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf22-mv", "kind": pca_val })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_mv_ds.clone())?
        .scalar("PC1 eigenvalue", None, 1e-6, 1e-6)
        .scalar("PC1 variance proportion", None, 1e-6, 1e-6),
    )?;
    let clust_val = serde_json::to_value(AnalysisKind::Multivariate(MultivariateSpec::Clustering {
        k_means_k: Some(3),
        distance: Default::default(),
        linkage: ClusterLinkage::Complete,
        k_sweep_max: Some(6),
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(clust_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf22-mv", "kind": clust_val })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_mv_ds.clone())?
        .scalar("Clusters (k)", None, 0.0, 0.0)
        .scalar("Total WCSS", None, 1e-6, 1e-6),
    )?;
    let glm_val = serde_json::to_value(AnalysisKind::GeneralizedLinearModel(GlmSpec {
        family: GlmFamily::Linear,
        response_column: 3,
        predictor_columns: vec![0, 1, 2],
        reference_levels: Vec::new(),
        compute_vif: true,
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(glm_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf22-mv", "kind": glm_val })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_mv_ds)?
        .scalar("R squared", None, 1e-6, 1e-6),
    )?;

    // 2. Grouped table (`5 rows x 2 groups x 3 replicates`) -> Multiple row t-Tests + Volcano Plot + Format Selected Points.
    let graphs_before: Vec<SheetId> = run.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
    let mut grp_extra = vec!["welcome.stepper.cols.dec", "welcome.stepper.cols.dec"];
    grp_extra.extend(std::iter::repeat_n("welcome.stepper.rows.dec", 7));
    let grp_id = run.create_table_via_welcome("welcome.arch.grouped", "Gene Screen", &grp_extra)?;
    let volc_gs_id = find_new_graph_for_source(&run, &graphs_before, &grp_id)?;
    run.enter_column_title(0, "Ctrl")?;
    run.enter_column_title(1, "Treated")?;
    for r in 0..5 {
        run.enter_row_title(r, &format!("Gene{}", r + 1))?;
        let delta = match r {
            0 => 14.0,
            1 => 6.0,
            _ => 0.2,
        };
        for rep in 0..3 {
            let jit = ((rep as f64) - 1.0) * 0.25;
            run.enter_grid_cell(r, 0, rep, &format!("{:.2}", 10.0 + jit))?;
            run.enter_grid_cell(r, 1, rep, &format!("{:.2}", 10.0 + delta + jit))?;
        }
    }

    let mrt_id = run.run_analyze_entry("Multiple row", "analyze.entry.stats.multiple_row_t_tests")?;
    run.select_sheet(&mrt_id)?;
    run.open_cmd_t()?;
    run.click("edit.row_t.equal_sd")?;
    run.commit_modal_ok()?;

    run.select_sheet(&volc_gs_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.show_all")?;
    run.click("graph_type.card.diag.volcano")?;
    run.commit_modal_ok()?;

    // Select a point on the data grid and customize via Format Selected Points modal.
    run.select_sheet(&grp_id)?;
    run.right_click("grid.cell.0.0.0")?;
    run.click("grid.ctx.style_points")?;
    ensure(matches!(run.active_dialog(), ActiveDialog::FormatSelectedPoints { .. }), "WF-22 expected FormatSelectedPoints modal")?;
    run.click("fmt_pts.col.Orange")?;
    run.click("fmt_pts.sym.Diamond")?;
    run.commit_modal_ok()?;
    {
        let vg = run.document().graph_sheet(&volc_gs_id).ok_or("WF-22 missing Volcano graph")?;
        ensure(vg.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Volcano), "WF-22 expected Volcano GraphKind")?;
        ensure(!vg.spec.point_overrides.is_empty(), "WF-22 Format Selected Points must persist point_overrides")?;
    }

    let mut ref_grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 5).map_err(|e| e.to_string())?;
    ref_grp.data_sets[0].title = String::from("Ctrl");
    ref_grp.data_sets[1].title = String::from("Treated");
    for r in 0..5 {
        ref_grp.row_titles[r] = format!("Gene{}", r + 1);
        let delta = match r {
            0 => 14.0,
            1 => 6.0,
            _ => 0.2,
        };
        for rep in 0..3 {
            let jit = ((rep as f64) - 1.0) * 0.25;
            ref_grp.data_sets[0].cells[r][rep] = Cell::number(10.0 + jit);
            ref_grp.data_sets[1].cells[r][rep] = Cell::number(10.0 + delta + jit);
        }
    }
    let mut ref_grp_doc = ProjectDocument::new("WF-22 Volcano Oracle").map_err(|e| e.to_string())?;
    let ref_grp_ds = DataSheet::new("ref-wf22-grp", "Gene Screen", TableArchetype::Grouped(ref_grp)).map_err(|e| e.to_string())?;
    ref_grp_doc.add_data_sheet(ref_grp_ds).map_err(|e| e.to_string())?;
    let mrt_val =
        serde_json::to_value(AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec { assume_equal_sd: true, ..MultipleRowTestsSpec::default() }))
            .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(mrt_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf22-grp", "kind": mrt_val })).map_err(|e| e.to_string())?;
            sess.execute(
                "graph.create",
                json!({
                    "data_sheet_id": "ref-wf22-grp",
                    "kind": GraphKind::Diagnostic(DiagnosticPlotKind::Volcano),
                    "initialized": true
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_document(ref_grp_doc)
        .scalar("MinRawP", None, 1e-12, 1e-12)
        .scalar("Discoveries", None, 0.0, 0.0)
        .with_graph(volc_gs_id.clone())
        .check_graph(|gui_gs, _ref_gs| ensure(gui_gs.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Volcano), "Volcano GraphKind mismatch")),
    )?;

    // Stage 5 LiveEdit across both MV features and Volcano source sheet.
    run.live_edit(LiveEditTarget::sheet_grid_cell(mv_id, 0, 0, 0, "9.5"), LiveEditExpectation::scalar_changed(pca_id, "PC1 eigenvalue", None, 0.05))?;
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(grp_id, 0, 1, 0, "8.0"),
        vec![LiveEditExpectation::scalar_changed(mrt_id, "MinRawP", None, 1e-6), LiveEditExpectation::graph_svg_changed(volc_gs_id)],
    )?;
    run.finish()
}

#[test]
fn wf23_human_stacked_superimposed_pcr_r10_and_chaining() -> TestRes {
    let mut run = WorkflowRun::start("WF-23");

    // 1. Multi-dataset Grouped Stacked vs Superimposed arrangement & ChartScene z-order check.
    let graphs_before: Vec<SheetId> = run.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
    let mut grp_extra = vec!["welcome.stepper.cols.dec", "welcome.stepper.replicates.dec"];
    grp_extra.extend(std::iter::repeat_n("welcome.stepper.rows.dec", 10));
    let grp_id = run.create_table_via_welcome("welcome.arch.grouped", "Arrangement Check", &grp_extra)?;
    let grp_gs_id = find_new_graph_for_source(&run, &graphs_before, &grp_id)?;
    run.enter_column_title(0, "Short")?;
    run.enter_column_title(1, "Tall")?;
    run.enter_row_title(0, "Baseline")?;
    run.enter_row_title(1, "Followup")?;
    for (r, (s0, s1)) in [(10.0, 30.0), (15.0, 25.0)].iter().enumerate() {
        for rep in 0..2 {
            run.enter_grid_cell(r, 0, rep, &format!("{s0}"))?;
            run.enter_grid_cell(r, 1, rep, &format!("{s1}"))?;
        }
    }

    run.select_sheet(&grp_gs_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.card.grouped.stacked")?;
    run.commit_modal_ok()?;
    {
        let g_st = run.document().graph_sheet(&grp_gs_id).ok_or("WF-23 missing stacked graph")?;
        ensure(g_st.spec.grouped_arrangement == GroupedArrangement::Stacked, "WF-23 expected Stacked arrangement")?;
    }

    run.click("toolbar.graph_type")?;
    run.click("graph_type.card.grouped.superimposed")?;
    run.commit_modal_ok()?;
    {
        let g_sup = run.document().graph_sheet(&grp_gs_id).ok_or("WF-23 missing superimposed graph")?;
        ensure(g_sup.spec.grouped_arrangement == GroupedArrangement::Superimposed, "WF-23 expected Superimposed arrangement")?;
    }

    let mut ref_grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).map_err(|e| e.to_string())?;
    ref_grp.data_sets[0].title = String::from("Short");
    ref_grp.data_sets[1].title = String::from("Tall");
    ref_grp.row_titles = vec![String::from("Baseline"), String::from("Followup")];
    for (r, (s0, s1)) in [(10.0, 30.0), (15.0, 25.0)].iter().enumerate() {
        ref_grp.data_sets[0].cells[r] = vec![Cell::number(*s0), Cell::number(*s0)];
        ref_grp.data_sets[1].cells[r] = vec![Cell::number(*s1), Cell::number(*s1)];
    }
    let mut ref_grp_doc = ProjectDocument::new("WF-23 Sup Oracle").map_err(|e| e.to_string())?;
    let ref_grp_ds = DataSheet::new("ref-wf23-grp", "Arrangement Check", TableArchetype::Grouped(ref_grp)).map_err(|e| e.to_string())?;
    ref_grp_doc.add_data_sheet(ref_grp_ds).map_err(|e| e.to_string())?;
    let doc_snap = run.document().clone();
    run.verify_reference(
        ReferenceSpec::for_graph(grp_gs_id.clone(), move |sess| {
            let out = sess
                .execute("graph.create", json!({ "data_sheet_id": "ref-wf23-grp", "kind": GraphKind::Grouped, "initialized": true }))
                .map_err(|e| e.to_string())?;
            let gid = out.payload["sheet_id"].as_str().ok_or("missing ref gid")?;
            sess.execute("graph.setSpec", json!({ "sheet_id": gid, "grouped_arrangement": "Superimposed" })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_document(ref_grp_doc)
        .check_graph(move |gui_gs, _ref_gs| {
            let scene = compile_graph(gui_gs, &doc_snap).map_err(|e| e.to_string())?;
            let c0 = gui_gs.spec.palette.color_at(0);
            let c1 = gui_gs.spec.palette.color_at(1);
            let rgb = |c: Rgba8| (c.r, c.g, c.b);
            let bar_rgbs: Vec<(u8, u8, u8)> = scene
                .primitives
                .iter()
                .filter_map(|p| match p {
                    ScenePrimitive::Path { path, fill: Some(f), clip: Some(_), .. }
                        if matches!(path.commands.first(), Some(PathCmd::MoveTo(_))) && (rgb(f.color) == rgb(c0) || rgb(f.color) == rgb(c1)) =>
                    {
                        Some(rgb(f.color))
                    }
                    _ => None,
                })
                .collect();
            let strictly_tall_first = !bar_rgbs.windows(2).any(|w| w[0] == rgb(c0) && w[1] == rgb(c1));
            ensure(bar_rgbs.len() >= 4 && strictly_tall_first, format!("expected tall bars strictly before short in z-order, got {bar_rgbs:?}"))
        }),
    )?;

    // 2. 2-hop ResultSheet `Analyze...` chaining (XY -> Normalize Transform -> Linear Regression).
    let xy_id = run.create_table_via_welcome(
        "welcome.arch.xy",
        "Raw Dose Chain",
        &["welcome.stepper.cols.dec", "welcome.subcol.single", "welcome.stepper.rows.inc", "welcome.stepper.rows.inc", "welcome.stepper.rows.inc"],
    )?;
    for r in 0..5 {
        let x = (r + 1) as f64;
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        run.enter_grid_cell(r, 0, 0, &format!("{:.1}", x * 10.0))?;
    }
    let norm_id = run.run_analyze_entry("Normalize", "analyze.entry.transform.normalize")?;
    run.select_sheet(&norm_id)?;
    let chained_id = run.run_analyze_entry("Linear", "analyze.entry.fit.linear")?;
    {
        let chained = run.document().result_sheet(&chained_id).ok_or("WF-23 missing chained ResultSheet")?;
        ensure(chained.analysis.source_sheet_id == norm_id, "WF-23 chained source must point to Normalize ResultSheet")?;
        let slope = scalar_val(chained, "Slope")?;
        ensure((slope - 25.0).abs() < 0.05, format!("expected normalized linear slope ~25.0, got {slope}"))?;
    }
    let mut ref_xy = XyTable::new(SubcolumnMode::Single, 1, 5).map_err(|e| e.to_string())?;
    for r in 0..5 {
        let x = (r + 1) as f64;
        ref_xy.x_cells[r][0] = Cell::number(x);
        ref_xy.data_sets[0].cells[r][0] = Cell::number(x * 10.0);
    }
    let ref_xy_sheet = DataSheet::new("ref-wf23-xy", "Raw Dose Chain", TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    let norm_kind = serde_json::to_value(AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
        zero_anchor: curvebook_doc::NormalizeAnchor::ColumnMin,
        hundred_anchor: curvebook_doc::NormalizeAnchor::ColumnMax,
        output_percent: true,
        per_data_set: true,
    }))
    .map_err(|e| e.to_string())?;
    let lin_kind = serde_json::to_value(AnalysisKind::LinearRegression(Default::default())).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(chained_id.clone(), move |sess| {
            let n_out = sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf23-xy", "kind": norm_kind })).map_err(|e| e.to_string())?;
            let n_id = n_out.payload["sheet_id"].as_str().ok_or("missing ref norm id")?;
            sess.execute("analysis.run", json!({ "source_sheet_id": n_id, "kind": lin_kind })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_xy_sheet)?
        .scalar("Slope", Some(0), 1e-6, 1e-6),
    )?;

    // 3. Leakage-safe PCR + R10 guard in AnalyzeSheet modal.
    let pcr_mv_id = run.create_table_via_welcome(
        "welcome.arch.multiple_variables",
        "PCR Cohort",
        &["welcome.stepper.rows.inc", "welcome.stepper.rows.inc", "welcome.stepper.rows.inc", "welcome.mv.role.1.Continuous"],
    )?;
    run.enter_column_title(0, "X1")?;
    run.enter_column_title(1, "X2")?;
    run.enter_column_title(2, "Target")?;
    for r in 0..8 {
        let x1 = (r + 1) as f64;
        let x2 = x1 + 1.0 + if r > 4 { 1.0 } else { 0.0 };
        let y = 3.0 * x1 + 2.0 * x2;
        run.enter_grid_cell(r, 0, 0, &format!("{x1}"))?;
        run.enter_grid_cell(r, 1, 0, &format!("{x2}"))?;
        run.enter_grid_cell(r, 2, 0, &format!("{y}"))?;
    }
    run.select_sheet(&pcr_mv_id)?;
    run.click("toolbar.analyze")?;
    run.click("analyze.search")?;
    run.type_text("Principal Component Regression")?;
    run.click("analyze.entry.stats.pcr")?;
    run.click("analyze.pca.pred.0")?;
    run.click("analyze.pca.pred.1")?;
    ensure(!widget_enabled(&run, "analyze.pcr.outcome.0")?, "R10: predictor col 0 cannot be chosen as PCR outcome")?;
    ensure(!widget_enabled(&run, "analyze.pcr.outcome.1")?, "R10: predictor col 1 cannot be chosen as PCR outcome")?;
    ensure(widget_enabled(&run, "analyze.pcr.outcome.2")?, "R10: non-predictor col 2 must be enabled as PCR outcome")?;
    run.click("analyze.pcr.outcome.2")?;
    run.commit_modal_ok()?;

    // Stage 5 LiveEdit across Grouped bars and 2-hop chained XY -> Normalize -> LinearRegression.
    run.live_edit(LiveEditTarget::sheet_grid_cell(grp_id, 0, 0, 0, "22.0"), LiveEditExpectation::graph_svg_changed(grp_gs_id))?;
    run.live_edit(LiveEditTarget::sheet_grid_cell(xy_id, 4, 0, 0, "90.0"), LiveEditExpectation::scalar_changed(chained_id, "Slope", Some(0), 1.0))?;
    run.finish()
}

#[test]
fn wf24_human_significance_brackets_stars_and_axes() -> TestRes {
    let mut run = WorkflowRun::start("WF-24");
    let fx = wf_datasets::column_three_group();
    let graphs_before: Vec<SheetId> = run.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
    let col_id = run.create_table_via_welcome("welcome.arch.column", fx.title, &[])?;
    let col_gs_id = find_new_graph_for_source(&run, &graphs_before, &col_id)?;
    for (c, title) in fx.group_titles.iter().enumerate() {
        run.enter_column_title(c, title)?;
        for (r, &v) in fx.columns[c].iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }

    let anova_id = run.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    run.select_sheet(&anova_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.multiple_comparisons")?;
    run.click("edit.post_hoc.tukey")?;
    run.commit_modal_ok()?;

    // Add significance brackets & stars from pairwise comparisons via Pairwise Comparisons modal.
    run.select_sheet(&col_gs_id)?;
    run.click("inspector.graph.tab.legends")?;
    run.click("inspector.graph.more_pairwise")?;
    run.click("pairwise.auto_add")?;
    run.commit_modal_ok()?;
    {
        let gs = run.document().graph_sheet(&col_gs_id).ok_or("WF-24 missing GraphSheet")?;
        ensure(!gs.spec.comparisons.is_empty(), "WF-24 expected auto-added significance brackets")?;
    }

    // Configure Y-axis Log10 scale + custom major/minor ticks (`GWF-18`) + additional reference tick.
    run.click("toolbar.axes")?;
    run.click("edit.tab.axes.y")?;
    run.click("axes.scale.log10")?;
    run.replace_textbox("axes.seg.0.major_tick_interval", "1")?;
    run.replace_textbox("axes.seg.0.minor_ticks_per_interval", "8")?;
    run.click("axes.seg.0.minor_ticks.plus")?;
    run.click("axes.additional_ticks.open")?;
    run.click("axes.add_tick.value")?;
    run.type_text("20.0")?;
    run.click("axes.add_tick.label")?;
    run.type_text("Threshold")?;
    run.click("axes.add_tick.save")?;
    run.click("axes.add_tick.close")?;
    run.commit_modal_ok()?;
    {
        let gs = run.document().graph_sheet(&col_gs_id).ok_or("WF-24 missing GraphSheet after FormatAxes")?;
        ensure(gs.spec.y_axis.segments[0].scale == GraphAxisScale::Log10, "WF-24 expected Log10 Y scale")?;
        ensure(gs.spec.y_axis.segments[0].major_tick_interval == Some(1.0), "WF-24 expected custom major_tick_interval=1")?;
        ensure(gs.spec.y_axis.segments[0].minor_ticks_per_interval == 9, "WF-24 expected minor_ticks_per_interval=9")?;
    }

    // Export check via ExportFigure SVG dialog.
    run.click("toolbar.export_svg")?;
    ensure(matches!(run.active_dialog(), ActiveDialog::ExportFigure { .. }), "WF-24 expected ExportFigure modal")?;
    run.click("export.fmt.svg")?;
    run.commit_modal_ok()?;
    ensure(!matches!(run.active_dialog(), ActiveDialog::ExportFigure { .. }), "WF-24 ExportFigure must close after ok")?;

    let ref_col = fx.to_data_sheet("ref-wf24-col").map_err(|e| e.to_string())?;
    let anova_kind = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(anova_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf24-col", "kind": anova_kind })).map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "data_sheet_id": "ref-wf24-col", "kind": GraphKind::Column, "initialized": true }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_col)?
        .scalar("F", None, 1e-9, 1e-9)
        .scalar("P value", None, 1e-12, 1e-12)
        .scalar("R squared", None, 1e-9, 1e-9)
        .with_graph(col_gs_id.clone())
        .check_graph(|gui_gs, _ref_gs| {
            ensure(!gui_gs.spec.comparisons.is_empty(), "WF-24 expected brackets preserved")?;
            ensure(gui_gs.spec.y_axis.segments[0].scale == GraphAxisScale::Log10, "WF-24 expected Log10 scale")
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(col_id, 0, 2, 0, "42.0"),
        vec![LiveEditExpectation::scalar_changed(anova_id, "F", None, 0.2), LiveEditExpectation::graph_svg_changed(col_gs_id)],
    )?;
    run.finish()
}
