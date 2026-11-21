#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-32`–`WF-37`, `R10.28`).

mod human_wf_common;

use curvebook_chart::compile_graph;
use curvebook_doc::{
    AnalysisKind, ClusterDistance, ClusterLinkage, DiagnosticPlotKind, GraphKind, GraphSheet, GraphicalDataObject, MultivariatePlotMode,
    MultivariateSpec, ParameterRule, ParameterScope, ProjectDocument, ResultSheet, ResultTabKind, SheetId, SumOfSquaresType, SurvivalAnalysisSpec,
    TableArchetype, TwoGroupMethod, VariableRole,
};
use curvebook_testkit::wf_datasets;
use serde_json::json;

use human_wf_common::TestRes;
use human_wf_common::ensure;
use human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};

fn find_graph_for_source(doc: &ProjectDocument, source_id: &SheetId, predicate: impl Fn(&GraphSheet) -> bool) -> Result<SheetId, String> {
    doc.graph_sheets
        .iter()
        .find(|g| g.spec.primary_source_sheet_id == *source_id && predicate(g))
        .map(|g| g.id.clone())
        .ok_or_else(|| format!("no matching GraphSheet bound to `{source_id}`"))
}

fn find_col_in_tab(rs: &ResultSheet, tab_id: &str, col_name: &str) -> Result<(usize, usize), String> {
    let tab_idx = rs.tabs.iter().position(|t| t.id == tab_id).ok_or_else(|| format!("missing tab `{tab_id}` in `{}`", rs.id))?;
    let col_idx =
        rs.tabs[tab_idx].column_headers.iter().position(|h| h == col_name).ok_or_else(|| format!("missing col `{col_name}` in `{tab_id}`"))?;
    Ok((tab_idx, col_idx))
}

fn find_row_col_in_tab(rs: &ResultSheet, tab_id: &str, row_label: &str, col_name: &str) -> Result<(usize, usize, usize), String> {
    let (tab_idx, col_idx) = find_col_in_tab(rs, tab_id, col_name)?;
    let row_idx =
        rs.tabs[tab_idx].row_labels.iter().position(|r| r == row_label).ok_or_else(|| format!("missing row `{row_label}` in `{tab_id}`"))?;
    Ok((tab_idx, row_idx, col_idx))
}

fn create_mv_table_via_welcome(run: &mut WorkflowRun, title: &str, roles: &[&str], col_titles: &[&str], tsv: &str) -> Result<SheetId, String> {
    run.click("welcome.arch.multiple_variables")?;
    for _ in 3..roles.len() {
        run.click("welcome.stepper.cols.inc")?;
    }
    let ds_id = run.create_table_via_welcome("welcome.arch.multiple_variables", title, &[])?;
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard(tsv)?;
    run.click("mv_formula.role_badge.0")?;
    run.click("mv_formula.role_manage.0")?;
    for (col_idx, &title_text) in col_titles.iter().enumerate() {
        run.click(&format!("mv_formula.vars.col.{col_idx}"))?;
        run.replace_textbox("mv_formula.vars.title_input", title_text)?;
        run.click("mv_formula.vars.meta.apply")?;
        run.click(&format!("mv_formula.vars.role.{}", roles[col_idx]))?;
    }
    run.click("mv_formula.vars.close")?;
    Ok(ds_id)
}

fn apply_mv_formula(run: &mut WorkflowRun, col_idx: usize, formula: &str) -> TestRes {
    run.click(&format!("inspector.mv.col.{col_idx}"))?;
    run.replace_textbox("inspector.mv.formula_input", formula)?;
    let can_apply = run.frame_meta().find_by_widget_id("inspector.mv.apply_formula").ok_or("missing inspector.mv.apply_formula")?.enabled;
    ensure(can_apply, format!("expected valid MV formula `{formula}` to enable apply_formula"))?;
    run.click("inspector.mv.apply_formula")
}

fn click_to_select(run: &mut WorkflowRun, wid: &str, desired_selected: bool) -> TestRes {
    let entry = run.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing widget `{wid}`"))?;
    let cur = match entry.value.as_ref() {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    };
    if cur != desired_selected {
        run.click(wid)?;
    }
    Ok(())
}

// ============================================================================
// WF-32: Direct MV Welch's t-Test, Calculated Difference & CategoricalSummary
// ============================================================================

#[test]
fn wf32_human_direct_mv_welch_t_and_categorical_summary() -> TestRes {
    let mut run = WorkflowRun::start("WF-32");
    let tsv = "\
V1\tVehicle\t10.0\t18.5
V2\tVehicle\t10.2\t20.2
V3\tVehicle\t9.9\t21.0
V4\tVehicle\t10.4\t22.4
V5\tVehicle\t9.8\t19.8
V6\tVehicle\t10.1\t23.1
V7\tVehicle\t10.0\t20.9
V8\tVehicle\t10.3\t21.5
T1\tTreated\t10.0\t31.0
T2\tTreated\t10.5\t34.5
T3\tTreated\t9.8\t42.0
T4\tTreated\t10.2\t38.2
T5\tTreated\t10.4\t46.8
T6\tTreated\t9.7\t35.4
T7\tTreated\t10.1\t40.1
T8\tTreated\t10.3\t52.0";
    let mv_ds_id = create_mv_table_via_welcome(
        &mut run,
        "Biomarker Cohort MV",
        &["label", "categorical", "continuous", "continuous", "continuous"],
        &["SampleID", "Treatment", "Baseline", "Biomarker", "Delta"],
        tsv,
    )?;

    apply_mv_formula(&mut run, 4, "= [Biomarker] - [Baseline]")?;

    // Direct MV Welch's t-test comparing Delta (col 4) across Treatment (col 1).
    let welch_id = run.run_analyze_entry("welch", "analyze.entry.stats.two_group")?;
    run.open_cmd_t()?;
    run.click("edit.tg.welch_t")?;
    click_to_select(&mut run, "edit.compute_effect_sizes", true)?;
    run.click("edit.two_group.group.1")?;
    run.click("edit.two_group.resp.4")?;
    run.commit_modal_ok()?;

    ensure(
        matches!(
            run.document().result_sheet(&welch_id).map(|r| &r.analysis.kind),
            Some(AnalysisKind::TwoGroupComparison(t)) if t.method == TwoGroupMethod::WelchT && t.compute_effect_sizes
        ),
        "expected non-default WelchT + effect sizes on ResultSheet",
    )?;

    // Switch linked MV graph to CategoricalSummary (X = Treatment [col 1], Y = Delta [col 4]) via Format Graph.
    let mv_graph_id = find_graph_for_source(run.document(), &mv_ds_id, |g| g.spec.kind == GraphKind::MultipleVariables)?;
    let (start_x, start_y, n_cols) = {
        let doc = run.document();
        let ds = doc.data_sheet(&mv_ds_id).ok_or("missing mv_ds_id for col count")?;
        let cols = match &ds.archetype {
            TableArchetype::MultipleVariables(mv) => mv.variables.len().max(1),
            _ => return Err(String::from("expected MultipleVariables archetype")),
        };
        let enc = doc.graph_sheet(&mv_graph_id).and_then(|g| g.spec.multivariate.as_ref()).ok_or("missing multivariate encoding on mv_graph_id")?;
        (enc.x_column, enc.y_column, cols)
    };
    run.select_sheet(&mv_graph_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.format.open")?;
    run.click("edit.tab.format_graph.axis_variables")?;
    for _ in 0..((1 + n_cols - (start_x % n_cols)) % n_cols) {
        run.click("format_graph.mv.x_next")?;
    }
    for _ in 0..((4 + n_cols - (start_y % n_cols)) % n_cols) {
        run.click("format_graph.mv.y_next")?;
    }
    run.click("format_graph.mv.plot_mode.categorical_summary")?;
    run.commit_modal_ok()?;
    {
        let doc = run.document();
        let gs = doc.graph_sheet(&mv_graph_id).ok_or("missing mv_graph_id")?;
        let scene = compile_graph(gs, doc).map_err(|e| e.to_string())?;
        ensure(!scene.primitives.is_empty(), "expected non-empty CategoricalSummary scene")?;
    }

    let ref_mv = run.document().data_sheet(&mv_ds_id).ok_or("missing mv_ds_id")?.clone();
    let ref_mv_id = ref_mv.id.clone();
    let (tg_tab_idx, df_row, tg_col) = {
        let rs = run.document().result_sheet(&welch_id).ok_or("missing welch_id")?;
        let col = rs.tabs.iter().find(|t| t.id == "two-group").and_then(|t| t.column_headers.first()).ok_or("missing two-group col")?.clone();
        find_row_col_in_tab(rs, "two-group", "df (Satterthwaite)", &col).or_else(|_| find_row_col_in_tab(rs, "two-group", "df", &col))?
    };

    run.verify_reference(
        ReferenceSpec::for_result(welch_id.clone(), move |sess| {
            let k = serde_json::to_value(AnalysisKind::TwoGroupComparison(curvebook_doc::TwoGroupSpec {
                method: TwoGroupMethod::WelchT,
                two_tailed: true,
                compute_effect_sizes: true,
                mv_group_column: Some(1),
                mv_response_column: Some(4),
            }))
            .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_mv_id.as_str(), "kind": k })).map_err(|e| e.to_string())?;
            let g_mv = serde_json::to_value(GraphKind::MultipleVariables).map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "data_sheet_id": ref_mv_id.as_str(), "kind": g_mv })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_mv)?
        .with_graph(mv_graph_id.clone())
        .scalar("PValue", None, 1e-9, 1e-6)
        .scalar("CohensD", None, 1e-6, 1e-6)
        .tab_cell(tg_tab_idx, df_row, tg_col, 1e-6, 1e-6)
        .check_graph(|gui_gs, _ref_gs| {
            let enc = gui_gs.spec.multivariate.as_ref().ok_or("missing MV encoding")?;
            ensure(
                enc.x_column == 1
                    && enc.y_column == 4
                    && matches!(enc.plot_mode, MultivariatePlotMode::CategoricalSummary { gdo: GraphicalDataObject::BoxAndWhisker { .. }, .. }),
                format!("expected CategoricalSummary BoxAndWhisker (x=1, y=4), got {enc:?}"),
            )?;
            Ok(())
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mv_ds_id, 15, 3, 0, "65.0"),
        vec![LiveEditExpectation::scalar_changed(welch_id, "PValue", None, 1e-9), LiveEditExpectation::graph_svg_changed(mv_graph_id)],
    )?;
    run.finish()
}

// ============================================================================
// WF-33: Direct MV Multifactor (3-Way) ANOVA, Type III SS, and Tukey HSD
// ============================================================================

#[test]
fn wf33_human_direct_mv_multifactor_anova() -> TestRes {
    let mut run = WorkflowRun::start("WF-33");
    // Use numeric timepoint codes ("0" / "24") and explicitly cast all 3 factors to Categorical via `set_mv_role`.
    let tsv = "\
S01\tWT\tVehicle\t0\t10.0
S02\tWT\tVehicle\t0\t11.0
S03\tWT\tVehicle\t24\t12.0
S04\tWT\tVehicle\t24\t13.0
S05\tWT\tDrug\t0\t15.0
S06\tWT\tDrug\t0\t16.0
S07\tWT\tDrug\t24\t28.0
S08\tWT\tDrug\t24\t30.0
S09\tKO\tVehicle\t0\t10.5
S10\tKO\tVehicle\t0\t11.5
S11\tKO\tVehicle\t24\t12.5
S12\tKO\tVehicle\t24\t13.5
S13\tKO\tDrug\t0\t14.0
S14\tKO\tDrug\t0\t15.0
S15\tKO\tDrug\t24\t16.0
S16\tKO\tDrug\t24\t17.0";
    let mv_ds_id = create_mv_table_via_welcome(
        &mut run,
        "3-Way Genetic Pharmacology MV",
        &["label", "categorical", "categorical", "categorical", "continuous"],
        &["Sample", "Genotype", "Drug", "Timepoint", "Signal"],
        tsv,
    )?;

    if let TableArchetype::MultipleVariables(mv) = &run.document().data_sheet(&mv_ds_id).ok_or("missing mv_ds")?.archetype {
        for fc in [1usize, 2, 3] {
            ensure(mv.variables[fc].role == VariableRole::Categorical, format!("factor col {fc} must have Categorical role"))?;
        }
    }

    let anova_id = run.run_analyze_entry("factorial", "analyze.entry.stats.factorial_anova")?;
    run.open_cmd_t()?;
    run.click("edit.fac.ss_type_iii")?;
    run.click("edit.fac.order.3")?;
    click_to_select(&mut run, "edit.compute_effect_sizes", true)?;
    run.click("edit.fac.resp.4")?;
    click_to_select(&mut run, "edit.fac.col.1", true)?;
    click_to_select(&mut run, "edit.fac.col.2", true)?;
    click_to_select(&mut run, "edit.fac.col.3", true)?;
    run.click("edit.tab.multiple_comparisons")?;
    run.click("edit.post_hoc.tukey")?;
    run.commit_modal_ok()?;

    ensure(
        matches!(
            run.document().result_sheet(&anova_id).map(|r| &r.analysis.kind),
            Some(AnalysisKind::FactorialAnova(fa))
                if fa.ss_type == SumOfSquaresType::TypeIII
                    && fa.max_interaction_order == 3
                    && fa.compute_effect_sizes
                    && fa.multiplicity.method == curvebook_doc::MultiplicityMethod::TukeyHsd
        ),
        "expected TypeIII 3-way FactorialAnova with Tukey HSD",
    )?;
    ensure(
        run.document().result_sheet(&anova_id).is_some_and(|r| r.tabs.iter().any(|t| t.kind == ResultTabKind::MultipleComparisons)),
        "expected Tukey MultipleComparisons tab",
    )?;

    let ref_mv = run.document().data_sheet(&mv_ds_id).ok_or("missing mv_ds_id")?.clone();
    let ref_mv_id = ref_mv.id.clone();
    let (tab_idx, eta_col) =
        find_col_in_tab(run.document().result_sheet(&anova_id).ok_or("missing anova")?, "factorial-anova", "Partial eta squared")?;
    let (_, row_3w, _) = find_row_col_in_tab(
        run.document().result_sheet(&anova_id).ok_or("missing anova")?,
        "factorial-anova",
        "Genotype × Drug × Timepoint",
        "Partial eta squared",
    )?;

    run.verify_reference(
        ReferenceSpec::for_result(anova_id.clone(), move |sess| {
            let k = serde_json::to_value(AnalysisKind::FactorialAnova(curvebook_doc::FactorialAnovaSpec {
                ss_type: SumOfSquaresType::TypeIII,
                max_interaction_order: 3,
                compute_effect_sizes: true,
                three_way: true,
                response_column: Some(4),
                multifactor_columns: vec![1, 2, 3],
                multiplicity: curvebook_doc::MultiplicitySpec { method: curvebook_doc::MultiplicityMethod::TukeyHsd, ..Default::default() },
                ..Default::default()
            }))
            .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_mv_id.as_str(), "kind": k })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_mv)?
        .scalar("MS residual", None, 1e-6, 1e-6)
        .tab_cell(tab_idx, row_3w, eta_col, 1e-6, 1e-6),
    )?;

    run.live_edit(
        LiveEditTarget::sheet_grid_cell(mv_ds_id, 7, 4, 0, "40.0"),
        LiveEditExpectation::scalar_changed(anova_id, "MS residual", None, 0.1),
    )?;
    run.finish()
}

// ============================================================================
// WF-34: qPCR ddCt Clean-Room Pipeline (Calculated Variables + Welch t-Test)
// ============================================================================

#[test]
fn wf34_human_qpcr_ddct_calculated_variables_and_welch() -> TestRes {
    let mut run = WorkflowRun::start("WF-34");
    let qpcr = wf_datasets::qpcr_ddct();
    let body_tsv = qpcr.to_tsv().split_once('\n').ok_or("missing qpcr rows")?.1.to_owned();
    let mv_ds_id = create_mv_table_via_welcome(
        &mut run,
        qpcr.title,
        &["categorical", "continuous", "continuous", "label", "continuous", "continuous", "continuous", "continuous", "continuous"],
        &["Group", "TargetCt", "RefCt", "Well", "DeltaCt", "CalMean", "DeltaDeltaCt", "FoldChange", "Log2FC"],
        &body_tsv,
    )?;

    apply_mv_formula(&mut run, 4, "= [TargetCt] - [RefCt]")?;
    apply_mv_formula(&mut run, 5, "= MEANIF([Group] == \"Calibrator\", [DeltaCt])")?;
    apply_mv_formula(&mut run, 6, "= [DeltaCt] - [CalMean]")?;
    apply_mv_formula(&mut run, 7, "= 2 ^ (-[DeltaDeltaCt])")?;
    apply_mv_formula(&mut run, 8, "= -[DeltaDeltaCt]")?;

    let welch_id = run.run_analyze_entry("welch", "analyze.entry.stats.two_group")?;
    run.open_cmd_t()?;
    run.click("edit.tg.welch_t")?;
    click_to_select(&mut run, "edit.compute_effect_sizes", true)?;
    run.click("edit.two_group.group.0")?;
    run.click("edit.two_group.resp.8")?;
    run.commit_modal_ok()?;

    // Switch linked MV graph to CategoricalSummary (X = Group [col 0], Y = Log2FC [col 8]) via Bubble Plot modal.
    let mv_graph_id = find_graph_for_source(run.document(), &mv_ds_id, |g| g.spec.kind == GraphKind::MultipleVariables)?;
    let (start_x, start_y, n_cols) = {
        let doc = run.document();
        let ds = doc.data_sheet(&mv_ds_id).ok_or("missing qpcr mv for cols")?;
        let cols = match &ds.archetype {
            TableArchetype::MultipleVariables(mv) => mv.variables.len().max(1),
            _ => return Err(String::from("expected MultipleVariables archetype")),
        };
        let enc = doc.graph_sheet(&mv_graph_id).and_then(|g| g.spec.multivariate.as_ref()).ok_or("missing qpcr MV encoding")?;
        (enc.x_column, enc.y_column, cols)
    };
    run.select_sheet(&mv_graph_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.bubblePlot.open")?;
    for _ in 0..((n_cols - (start_x % n_cols)) % n_cols) {
        run.click("bubble.x_next")?;
    }
    for _ in 0..((8 + n_cols - (start_y % n_cols)) % n_cols) {
        run.click("bubble.y_next")?;
    }
    run.click("bubble.plot_mode.categorical_summary")?;
    run.commit_modal_ok()?;
    {
        let doc = run.document();
        let gs = doc.graph_sheet(&mv_graph_id).ok_or("missing qpcr mv_graph_id")?;
        let scene = compile_graph(gs, doc).map_err(|e| e.to_string())?;
        ensure(!scene.primitives.is_empty(), "expected non-empty qpcr CategoricalSummary scene")?;
    }

    let ref_mv = run.document().data_sheet(&mv_ds_id).ok_or("missing qpcr mv")?.clone();
    let ref_mv_id = ref_mv.id.clone();
    run.verify_reference(
        ReferenceSpec::for_result(welch_id.clone(), move |sess| {
            let k = serde_json::to_value(AnalysisKind::TwoGroupComparison(curvebook_doc::TwoGroupSpec {
                method: TwoGroupMethod::WelchT,
                two_tailed: true,
                compute_effect_sizes: true,
                mv_group_column: Some(0),
                mv_response_column: Some(8),
            }))
            .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_mv_id.as_str(), "kind": k })).map_err(|e| e.to_string())?;
            let g_mv = serde_json::to_value(GraphKind::MultipleVariables).map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "data_sheet_id": ref_mv_id.as_str(), "kind": g_mv })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_mv)?
        .with_graph(mv_graph_id.clone())
        .scalar("PValue", None, 1e-9, 1e-6)
        .scalar("CohensD", None, 1e-6, 1e-6)
        .check_graph(|gui_gs, _ref_gs| {
            let enc = gui_gs.spec.multivariate.as_ref().ok_or("missing qpcr MV encoding")?;
            ensure(
                enc.x_column == 0 && enc.y_column == 8 && matches!(enc.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }),
                format!("expected qpcr CategoricalSummary (x=0, y=8), got {enc:?}"),
            )?;
            Ok(())
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mv_ds_id, 6, 1, 0, "20.1"),
        vec![LiveEditExpectation::scalar_changed(welch_id, "PValue", None, 1e-9), LiveEditExpectation::graph_svg_changed(mv_graph_id)],
    )?;
    run.finish()
}

// ============================================================================
// WF-35: Direct MV Kaplan-Meier Survival (Time/Status/Cohort + Trend + Graph)
// ============================================================================

#[test]
fn wf35_human_direct_mv_kaplan_meier_survival() -> TestRes {
    let mut run = WorkflowRun::start("WF-35");
    let tsv = "\
P01\tPlacebo\t10\t1
P02\tPlacebo\t14\t1
P03\tPlacebo\t18\t1
P04\tPlacebo\t22\t1
P05\tPlacebo\t26\t0
P06\tPlacebo\t30\t1
L01\tLowDose\t20\t1
L02\tLowDose\t28\t1
L03\tLowDose\t35\t0
L04\tLowDose\t42\t1
L05\tLowDose\t48\t0
L06\tLowDose\t55\t1
H01\tHighDose\t40\t1
H02\tHighDose\t56\t0
H03\tHighDose\t64\t0
H04\tHighDose\t72\t1
H05\tHighDose\t80\t0
H06\tHighDose\t90\t0";
    let mv_ds_id = create_mv_table_via_welcome(
        &mut run,
        "Survival MV Trial",
        &["label", "categorical", "continuous", "continuous"],
        &["PatientID", "Cohort", "Days", "Event"],
        tsv,
    )?;

    let km_id = run.run_analyze_entry("kaplan", "analyze.entry.stats.survival_km")?;
    run.open_cmd_t()?;
    run.click("edit.tab.data")?;
    run.click("edit.km.time.2")?;
    run.click("edit.km.status.3")?;
    click_to_select(&mut run, "edit.km.group.1", true)?;
    run.click("edit.tab.experimental_design")?;
    click_to_select(&mut run, "edit.km.mantel_cox", true)?;
    click_to_select(&mut run, "edit.km.gehan", true)?;
    click_to_select(&mut run, "edit.km.trend", true)?;
    click_to_select(&mut run, "edit.km.asym_ci", true)?;
    run.click("edit.tab.multiple_comparisons")?;
    run.click("edit.post_hoc.holm_sidak")?;
    run.commit_modal_ok()?;

    ensure(
        matches!(
            run.document().result_sheet(&km_id).map(|r| &r.analysis.kind),
            Some(AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: true,
                trend_test: true,
                mv_time_column: Some(2),
                mv_status_column: Some(3),
                mv_group_column: Some(1),
                ..
            }))
        ),
        "expected explicit MV KM columns and non-default survival tests",
    )?;

    // Create dedicated Survival graph and add Median Y threshold via Format Axes.
    run.open_menu_and_click("menu.insert", &[], "menu.ui.graph.create.open")?;
    run.click("create_graph.source.0")?;
    run.click("create_graph.kind.survival")?;
    run.commit_modal_ok()?;
    let km_graph_id = find_graph_for_source(run.document(), &mv_ds_id, |g| g.spec.kind == GraphKind::Survival)?;
    run.select_sheet(&km_graph_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.axes.open")?;
    run.click("edit.tab.axes.y")?;
    run.click("axes.additional_ticks.open")?;
    run.replace_textbox("axes.add_tick.value", "50")?;
    run.replace_textbox("axes.add_tick.label", "Median survival")?;
    click_to_select(&mut run, "axes.add_tick.show_line", true)?;
    run.click("axes.add_tick.save")?;
    run.click("axes.add_tick.close")?;
    run.commit_modal_ok()?;

    let ref_mv = run.document().data_sheet(&mv_ds_id).ok_or("missing surv mv")?.clone();
    let ref_mv_id = ref_mv.id.clone();
    let (km_tab_idx, placebo_row, events_col) = {
        let rs = run.document().result_sheet(&km_id).ok_or("missing km_id")?;
        find_row_col_in_tab(rs, "km-summary", "Placebo", "Events")?
    };

    run.verify_reference(
        ReferenceSpec::for_result(km_id.clone(), move |sess| {
            let k = serde_json::to_value(AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: true,
                trend_test: true,
                asymmetrical_greenwood_ci: true,
                multiplicity: curvebook_doc::MultiplicitySpec { method: curvebook_doc::MultiplicityMethod::HolmSidak, ..Default::default() },
                mv_time_column: Some(2),
                mv_status_column: Some(3),
                mv_group_column: Some(1),
            }))
            .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_mv_id.as_str(), "kind": k })).map_err(|e| e.to_string())?;
            let g_val = serde_json::to_value(GraphKind::Survival).map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "data_sheet_id": ref_mv_id.as_str(), "kind": g_val })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_mv)?
        .with_graph(km_graph_id.clone())
        .scalar("Log-rank chi-square", None, 1e-6, 1e-6)
        .scalar("Log-rank P value", None, 1e-6, 1e-6)
        .tab_cell(km_tab_idx, placebo_row, events_col, 1e-6, 1e-6)
        .check_graph(|gs, _ref| ensure(!gs.spec.y_axis.thresholds.is_empty(), "expected Median threshold on Survival Y axis")),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mv_ds_id, 1, 3, 0, "0"),
        vec![
            LiveEditExpectation::tab_cell_changed(km_id.clone(), km_tab_idx, placebo_row, events_col),
            LiveEditExpectation::scalar_changed(km_id, "Log-rank chi-square", None, 1e-4),
            LiveEditExpectation::graph_svg_changed(km_graph_id),
        ],
    )?;
    run.finish()
}

// ============================================================================
// WF-36: Direct MV 4PL Dose-Response with Calculated LogConc & Constraints
// ============================================================================

#[test]
fn wf36_human_direct_mv_4pl_dose_response() -> TestRes {
    let mut run = WorkflowRun::start("WF-36");
    let tsv = "\
A1\tInhibitorA\t1e-10\t99.0
A2\tInhibitorA\t1e-9\t98.5
A3\tInhibitorA\t1e-8\t91.0
A4\tInhibitorA\t1e-7\t50.0
A5\tInhibitorA\t1e-6\t9.5
A6\tInhibitorA\t1e-5\t1.5
A7\tInhibitorA\t1e-4\t0.5
A8\tInhibitorA\t1e-3\t0.2
B1\tInhibitorB\t1e-10\t99.5
B2\tInhibitorB\t1e-9\t99.0
B3\tInhibitorB\t1e-8\t98.0
B4\tInhibitorB\t1e-7\t96.5
B5\tInhibitorB\t1e-6\t76.0
B6\tInhibitorB\t1e-5\t24.0
B7\tInhibitorB\t1e-4\t3.0
B8\tInhibitorB\t1e-3\t0.8";
    let mv_ds_id = create_mv_table_via_welcome(
        &mut run,
        "Dose-Response 4PL MV",
        &["label", "categorical", "continuous", "continuous", "continuous"],
        &["Well", "Compound", "ConcM", "Response", "LogConcM"],
        tsv,
    )?;

    apply_mv_formula(&mut run, 4, "= LOG10([ConcM])")?;

    let fit_id = run.run_analyze_entry("nonlinear", "analyze.entry.fit.nonlinear")?;
    run.open_cmd_t()?;
    run.click("fit.tab.model")?;
    run.replace_textbox("fit.eq.search", "log_inhibitor_vs_response_4pl")?;
    run.click("fit.eq.item.log_inhibitor_vs_response_4pl")?;
    run.click("fit.tab.data")?;
    run.click("fit.mv.y.3")?;
    click_to_select(&mut run, "fit.mv.x.2", false)?;
    click_to_select(&mut run, "fit.mv.x.4", true)?;
    click_to_select(&mut run, "fit.mv.group.1", true)?;
    run.click("fit.tab.constraints")?;
    run.click("fit.con.rule.Bottom.fixed")?;
    run.replace_textbox("fit.con.const.Bottom", "0")?;
    run.click("fit.con.rule.Top.fixed")?;
    run.replace_textbox("fit.con.const.Top", "100")?;
    run.click("fit.tab.output")?;
    run.click("fit.ci.profile")?;
    run.commit_modal_ok()?;

    ensure(
        matches!(
            run.document().result_sheet(&fit_id).map(|r| &r.analysis.kind),
            Some(AnalysisKind::NonlinearRegression(nl))
                if nl.profile_likelihood_ci
                    && nl.parameter_rules.iter().any(|r| r.name == "Bottom" && r.scope == ParameterScope::Constant(0.0))
                    && nl.parameter_rules.iter().any(|r| r.name == "Top" && r.scope == ParameterScope::Constant(100.0))
        ),
        "expected constrained Bottom=0, Top=100, and profile-likelihood CI on 4PL ResultSheet",
    )?;

    let ref_mv = run.document().data_sheet(&mv_ds_id).ok_or("missing fit mv")?.clone();
    let ref_mv_id = ref_mv.id.clone();
    let (param_tab_idx, log_ic50_row, best_col) = {
        let rs = run.document().result_sheet(&fit_id).ok_or("missing fit_id")?;
        let (ti, r, c) = find_row_col_in_tab(rs, "nonlinear_parameters", "LogIC50 [InhibitorA]", "Best-fit value")?;
        let (_, ci_col) = find_col_in_tab(rs, "nonlinear_parameters", "Confidence Interval")?;
        let ci_cell = &rs.tabs[ti].rows[r][ci_col];
        ensure(!matches!(ci_cell, curvebook_doc::ResultCell::Empty), format!("expected populated IC50 CI cell, got {ci_cell:?}"))?;
        (ti, r, c)
    };

    run.verify_reference(
        ReferenceSpec::for_result(fit_id.clone(), move |sess| {
            let mut fit = curvebook_doc::NonlinearFitSpec::new("log_inhibitor_vs_response_4pl");
            fit.mv_x_columns = vec![4];
            fit.mv_y_column = Some(3);
            fit.mv_group_columns = vec![1];
            fit.profile_likelihood_ci = true;
            fit.parameter_rules = vec![
                ParameterRule {
                    name: String::from("Bottom"),
                    scope: ParameterScope::Constant(0.0),
                    initial_override: None,
                    lower_bound: None,
                    upper_bound: None,
                },
                ParameterRule {
                    name: String::from("Top"),
                    scope: ParameterScope::Constant(100.0),
                    initial_override: None,
                    lower_bound: None,
                    upper_bound: None,
                },
            ];
            let k = serde_json::to_value(AnalysisKind::NonlinearRegression(fit)).map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_mv_id.as_str(), "kind": k })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_mv)?
        .tab_cell(param_tab_idx, log_ic50_row, best_col, 1e-4, 1e-4),
    )?;

    run.live_edit(
        LiveEditTarget::sheet_grid_cell(mv_ds_id, 3, 3, 0, "82.0"),
        LiveEditExpectation::tab_cell_changed(fit_id, param_tab_idx, log_ic50_row, best_col),
    )?;
    run.finish()
}

// ============================================================================
// WF-37: Hierarchical Clustering (Non-Default Distance/Linkage) + Dendrogram
// ============================================================================

#[test]
fn wf37_human_hierarchical_clustering_and_dendrogram() -> TestRes {
    let mut run = WorkflowRun::start("WF-37");
    let clust_fx = wf_datasets::clustering_three_clusters();
    let body_tsv = clust_fx.to_tsv().split_once('\n').ok_or("missing cluster rows")?.1.to_owned();
    let roles: Vec<&str> = clust_fx.columns.iter().map(|c| c.role_slug).collect();
    let titles: Vec<&str> = clust_fx.columns.iter().map(|c| c.title).collect();
    let mv_ds_id = create_mv_table_via_welcome(&mut run, clust_fx.title, &roles, &titles, &body_tsv)?;

    let clust_id = run.run_analyze_entry("cluster", "analyze.entry.stats.clustering")?;
    run.open_cmd_t()?;
    // Explicitly configure NON-DEFAULT Manhattan distance + Complete linkage (not Euclidean + Ward).
    run.click("edit.cluster.dist.manhattan")?;
    run.click("edit.cluster.link.complete")?;
    run.commit_modal_ok()?;

    ensure(
        matches!(
            run.document().result_sheet(&clust_id).map(|r| &r.analysis.kind),
            Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                distance: ClusterDistance::Manhattan,
                linkage: ClusterLinkage::Complete,
                ..
            }))
        ),
        "expected non-default Manhattan + Complete clustering configuration",
    )?;

    let hm_graph_id = find_graph_for_source(run.document(), &mv_ds_id, |g| g.spec.kind == GraphKind::MultipleVariables)?;
    run.select_sheet(&hm_graph_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.family.mv_diag")?;
    run.click("graph_type.card.diag.heatmap")?;
    run.commit_modal_ok()?;

    let ref_mv = run.document().data_sheet(&mv_ds_id).ok_or("missing cluster mv")?.clone();
    let ref_mv_id = ref_mv.id.clone();
    let (dendro_tab_idx, dist_col) = {
        let rs = run.document().result_sheet(&clust_id).ok_or("missing clust_id")?;
        find_col_in_tab(rs, "hierarchical-dendrogram", "Complete distance")?
    };

    let doc_clone = run.document().clone();
    run.verify_reference(
        ReferenceSpec::for_result(clust_id.clone(), move |sess| {
            let k = serde_json::to_value(AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                k_means_k: None,
                distance: ClusterDistance::Manhattan,
                linkage: ClusterLinkage::Complete,
                k_sweep_max: Some(6),
            }))
            .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_mv_id.as_str(), "kind": k })).map_err(|e| e.to_string())?;
            let hm_val = serde_json::to_value(GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)).map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "data_sheet_id": ref_mv_id.as_str(), "kind": hm_val })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_mv)?
        .with_graph(hm_graph_id.clone())
        .scalar("cophenetic_r", None, 1e-6, 1e-6)
        .tab_cell(dendro_tab_idx, 0, dist_col, 1e-6, 1e-6)
        .check_graph(move |gs, _ref| {
            ensure(gs.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap), "expected Heatmap graph kind")?;
            let scene = compile_graph(gs, &doc_clone).map_err(|e| e.to_string())?;
            let texts: Vec<String> = scene
                .primitives
                .iter()
                .filter_map(|p| match p {
                    curvebook_chart::ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
                    _ => None,
                })
                .collect();
            ensure(
                texts.iter().any(|t| t == "Clustered: Manhattan / Complete"),
                format!("expected non-default dendrogram cue `Clustered: Manhattan / Complete`, got {texts:?}"),
            )
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mv_ds_id, 0, 1, 0, "18.5"),
        vec![LiveEditExpectation::scalar_changed(clust_id, "cophenetic_r", None, 1e-4), LiveEditExpectation::graph_svg_changed(hm_graph_id)],
    )?;
    run.finish()
}
