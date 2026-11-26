//! Regression tests for verified scientific-chart gap audit findings (`GAP-02-01`..`GAP-02-13` and `GAP-06-03`).
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::time::Instant;

use curvebook_chart::compile_graph;
use curvebook_chart::scene::{ChartScene, PathCmd, ScenePrimitive};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, BlandAltmanSpec, BracketLabelMode, BracketStyle, Cell, ClusterDistance, ClusterLinkage,
    ColorChannelMapping, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, DiagnosticPlotKind, GlmFamily, GlmSpec, GraphKind, GraphSheet,
    GraphSpec, GraphicalDataObject, GroupedArrangement, GroupedTable, HeatmapLabelSpec, MultipleRowTestsSpec, MultipleVariablesTable,
    MultiplicityMethod, MultiplicitySpec, MultivariateGraphEncoding, MultivariatePlotMode, MultivariateSpec, NonlinearFitSpec, PairwiseBracketSpec,
    PairwiseComparisonRecord, ParameterRule, ParameterScope, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTabKind, RocCiMethod,
    RocSpec, SheetId, SizeChannelMapping, SubcolumnMode, SurvivalAnalysisSpec, SurvivalTable, TableArchetype, VariableRole, WhiskerMethod, XyTable,
};
use curvebook_fit::execute_analysis as execute_fit_analysis;
use curvebook_math::ConfidenceLevel;
use curvebook_palette::{MarkerStyle, PaletteId, Rgba8};
use curvebook_stats::multivariate::run_multivariate_analysis;
use curvebook_stats::regression::run_glm_analysis;
use curvebook_stats::roc::run_roc_analysis;
use curvebook_stats::survival::run_survival_analysis;
use curvebook_stats::two_group::{run_bland_altman_analysis, run_multiple_row_t_tests_analysis};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn plain_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn censor_tick_x_coords(scene: &ChartScene) -> Vec<f64> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() == 2 => {
                match (path.commands.first(), path.commands.get(1)) {
                    (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b))) if (a.x - b.x).abs() < 1e-6 && ((b.y - a.y).abs() - 7.0).abs() < 0.5 => {
                        Some(a.x)
                    }
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn all_points_finite(scene: &ChartScene) -> bool {
    scene.primitives.iter().all(|p| match p {
        ScenePrimitive::Path { path, .. } => path.commands.iter().all(|c| match *c {
            PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => pt.x.is_finite() && pt.y.is_finite(),
            PathCmd::QuadTo { control, to } => control.x.is_finite() && control.y.is_finite() && to.x.is_finite() && to.y.is_finite(),
            PathCmd::CubicTo { c1, c2, to } => {
                c1.x.is_finite() && c1.y.is_finite() && c2.x.is_finite() && c2.y.is_finite() && to.x.is_finite() && to.y.is_finite()
            }
            PathCmd::Close => true,
        }),
        ScenePrimitive::Text(t) => t.anchor_pt.x.is_finite() && t.anchor_pt.y.is_finite(),
        ScenePrimitive::RasterImage(img) => img.bounds_pt.x.is_finite() && img.bounds_pt.y.is_finite(),
        ScenePrimitive::PushClip { rect } => rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite(),
        ScenePrimitive::PopClip => true,
    })
}

fn km_spec_default() -> SurvivalAnalysisSpec {
    SurvivalAnalysisSpec::KaplanMeier {
        asymmetrical_greenwood_ci: false,
        log_rank_mantel_cox: false,
        gehan_breslow_wilcoxon: false,
        trend_test: false,
        multiplicity: MultiplicitySpec::default(),
        mv_time_column: None,
        mv_status_column: None,
        mv_group_column: None,
    }
}

fn compile_result_km_for_obs(obs: &[(f64, u8)]) -> TestResult<(ChartScene, f64)> {
    let mut surv = SurvivalTable::new(1, obs.len())?;
    surv.cohorts[0].title = String::from("Arm A");
    for (r, &(t, st)) in obs.iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(f64::from(st));
    }
    let ds = DataSheet::new(SheetId::new("surv-ds"), "Survival", TableArchetype::Survival(surv))?;
    let km_spec = km_spec_default();
    let mut an_spec = AnalysisSpec::new(SheetId::new("km-res"), ds.id.clone(), AnalysisKind::Survival(km_spec.clone()))?;
    an_spec.selected_data_sets = vec![0];
    let res = run_survival_analysis(&ds, &an_spec, &km_spec)?;

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res.clone())?;

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-km"), &ds)?;
    gs.spec.kind = GraphKind::Survival;
    gs.spec.primary_source_sheet_id = res.id;
    gs.spec.x_axis.segments[0].min = Some(0.0);
    let max_x = 10.0;
    gs.spec.x_axis.segments[0].max = Some(max_x);
    let scene = compile_graph(&gs, &doc)?;
    Ok((scene, max_x))
}

#[test]
fn gap_02_01_result_sourced_km_censor_tick_placement() -> TestResult {
    // 1. Events at t=1, 1, 2 (0 censored) -> must draw 0 censor ticks.
    let (sc_events_only, _) = compile_result_km_for_obs(&[(1.0, 1), (1.0, 1), (2.0, 1)])?;
    assert_eq!(censor_tick_x_coords(&sc_events_only).len(), 0, "zero censored subjects must produce zero censor ticks");

    // 2. Censor at t=1 before 2 events at t=2 -> must draw 1 censor tick at t=1.0 (not t=2.0).
    let (sc_early_censor, max_x) = compile_result_km_for_obs(&[(1.0, 0), (2.0, 1), (2.0, 1)])?;
    let ticks_early = censor_tick_x_coords(&sc_early_censor);
    assert_eq!(ticks_early.len(), 1, "expected 1 censor tick at t=1, got {ticks_early:?}");
    let expected_x_t1 = sc_early_censor.plot_rect_pt.x + (1.0 / max_x) * sc_early_censor.plot_rect_pt.width;
    assert!((ticks_early[0] - expected_x_t1).abs() <= 0.5, "censor tick x={} must match t=1.0 (expected {expected_x_t1})", ticks_early[0]);

    // 3. End-of-study censoring at last row (event at t=1, 2 censors at t=5) -> 1 tick at t=5.0.
    let (sc_eos, max_x_eos) = compile_result_km_for_obs(&[(1.0, 1), (5.0, 0), (5.0, 0)])?;
    let ticks_eos = censor_tick_x_coords(&sc_eos);
    assert_eq!(ticks_eos.len(), 1, "end-of-study censoring at last row must emit a tick: {ticks_eos:?}");
    let expected_x_t5 = sc_eos.plot_rect_pt.x + (5.0 / max_x_eos) * sc_eos.plot_rect_pt.width;
    assert!((ticks_eos[0] - expected_x_t5).abs() <= 0.5, "end-of-study censor tick x={} must match t=5.0 (expected {expected_x_t5})", ticks_eos[0]);

    // 4. Simultaneous event + censor at t=2 before event at t=4 -> 1 tick at t=2.0.
    let (sc_tied, max_x_tied) = compile_result_km_for_obs(&[(2.0, 1), (2.0, 0), (4.0, 1)])?;
    let ticks_tied = censor_tick_x_coords(&sc_tied);
    assert_eq!(ticks_tied.len(), 1, "tied event+censor at t=2 must emit 1 tick: {ticks_tied:?}");
    let expected_x_t2 = sc_tied.plot_rect_pt.x + (2.0 / max_x_tied) * sc_tied.plot_rect_pt.width;
    assert!((ticks_tied[0] - expected_x_t2).abs() <= 0.5);
    Ok(())
}

fn build_cox_mv_sheet() -> TestResult<DataSheet> {
    let rows = [
        (2.0, 1.0, 1.0, 62.0),
        (3.0, 1.0, 1.0, 65.0),
        (4.0, 1.0, 1.0, 68.0),
        (5.0, 1.0, 0.0, 58.0),
        (6.0, 1.0, 1.0, 60.0),
        (7.0, 0.0, 1.0, 55.0),
        (8.0, 1.0, 0.0, 54.0),
        (10.0, 1.0, 0.0, 51.0),
        (12.0, 0.0, 0.0, 48.0),
        (14.0, 1.0, 0.0, 49.0),
        (16.0, 0.0, 0.0, 45.0),
        (18.0, 0.0, 0.0, 43.0),
    ];
    let mut mv = MultipleVariablesTable::new(4, rows.len())?;
    mv.variables[0].title = String::from("Time");
    mv.variables[1].title = String::from("Status");
    mv.variables[2].title = String::from("Dose");
    mv.variables[3].title = String::from("Age");
    for (r, (t, st, dose, age)) in rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(t);
        mv.variables[1].cells[r] = Cell::number(st);
        mv.variables[2].cells[r] = Cell::number(dose);
        mv.variables[3].cells[r] = Cell::number(age);
    }
    Ok(DataSheet::new(SheetId::new("cox-mv"), "CoxMV", TableArchetype::MultipleVariables(mv))?)
}

#[test]
fn gap_02_02_forest_plot_selects_ratio_columns_ahead_of_beta() -> TestResult {
    // 1. Cox PH (headers: ["beta", "SE", "Wald z", "P value", "Hazard Ratio", "95% CI (HR)", "Schoenfeld P"])
    let ds = build_cox_mv_sheet()?;
    let cox_spec = SurvivalAnalysisSpec::CoxProportionalHazards {
        time_column: 0,
        status_column: 1,
        predictor_columns: vec![2, 3],
        reference_levels: Vec::new(),
        tie_method: curvebook_doc::CoxTieMethod::Efron,
    };
    let an = AnalysisSpec::new(SheetId::new("res-cox"), ds.id.clone(), AnalysisKind::Survival(cox_spec.clone()))?;
    let res = run_survival_analysis(&ds, &an, &cox_spec)?;
    let cox_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals).expect("cox params tab");
    let hr_col = cox_tab.column_headers.iter().position(|h| h == "Hazard Ratio").expect("HR col");
    let expected_hr0 = match cox_tab.rows[0][hr_col] {
        ResultCell::Number { value, .. } => value,
        _ => panic!("expected numeric HR"),
    };

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res.clone())?;

    for src in [res.id.clone(), ds.id.clone()] {
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cox-forest"), &ds)?;
        gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
        gs.spec.primary_source_sheet_id = src;
        gs.spec.x_axis.segments[0].scale = AxisScale::Log10;
        let scene = compile_graph(&gs, &doc)?;
        assert!(all_points_finite(&scene));
        assert!(
            !scene.warnings.iter().any(|w| w.code == "invalid_forest_interval"),
            "Cox forest plot must pair Hazard Ratio with HR CI without invalid_forest_interval: {:?}",
            scene.warnings
        );
        let texts = plain_texts(&scene);
        assert!(texts.iter().any(|t| t == "Dose"), "expected covariate row label 'Dose': {texts:?}");
        assert!(texts.iter().any(|t| t == "Age"), "expected covariate row label 'Age': {texts:?}");
        let hr_str = format!("{expected_hr0:.2} [");
        assert!(
            texts.iter().any(|t| t.starts_with(&hr_str)),
            "expected CI label starting with Hazard Ratio '{hr_str}', not log-scale beta; got {texts:?}"
        );
        assert!(!texts.iter().any(|t| t.contains("Study 1")), "must not fall back to raw MV rows: {texts:?}");
    }

    // 2. Logistic GLM (headers: ["beta", "SE", "Wald z", "P value", "Odds Ratio", "95% CI (OR)"])
    let log_rows = [(0.0, 0.2), (0.0, 0.5), (0.0, 0.9), (0.0, 1.1), (0.0, 1.4), (1.0, 1.2), (1.0, 1.8), (1.0, 2.3), (1.0, 2.7), (1.0, 3.1)];
    let mut mv_log = MultipleVariablesTable::new(2, log_rows.len())?;
    mv_log.variables[0].title = String::from("Outcome");
    mv_log.variables[1].title = String::from("Biomarker");
    for (r, (y, x)) in log_rows.into_iter().enumerate() {
        mv_log.variables[0].cells[r] = Cell::number(y);
        mv_log.variables[1].cells[r] = Cell::number(x);
    }
    let ds_log = DataSheet::new(SheetId::new("log-mv"), "LogMV", TableArchetype::MultipleVariables(mv_log))?;
    let glm_spec =
        GlmSpec { family: GlmFamily::Logistic, response_column: 0, predictor_columns: vec![1], reference_levels: Vec::new(), compute_vif: false };
    let an_log = AnalysisSpec::new(SheetId::new("res-log"), ds_log.id.clone(), AnalysisKind::GeneralizedLinearModel(glm_spec.clone()))?;
    let res_log = run_glm_analysis(&ds_log, &an_log, &glm_spec)?;
    let log_tab = res_log.tabs.iter().find(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals).expect("logistic tab");
    let or_col = log_tab.column_headers.iter().position(|h| h == "Odds Ratio").expect("OR col");
    let expected_or1 = match log_tab.rows[1][or_col] {
        ResultCell::Number { value, .. } => value,
        _ => panic!("expected numeric OR"),
    };
    let mut doc_log = ProjectDocument::default();
    doc_log.add_data_sheet(ds_log.clone())?;
    doc_log.add_result_sheet(res_log.clone())?;
    let mut gs_log = GraphSheet::new_uninitialized_for_data(SheetId::new("g-log-forest"), &ds_log)?;
    gs_log.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
    gs_log.spec.primary_source_sheet_id = res_log.id;
    let sc_log = compile_graph(&gs_log, &doc_log)?;
    assert!(!sc_log.warnings.iter().any(|w| w.code == "invalid_forest_interval"), "warnings: {:?}", sc_log.warnings);
    let or_prefix = format!("{expected_or1:.2} [");
    assert!(
        plain_texts(&sc_log).iter().any(|t| t.starts_with(&or_prefix)),
        "expected Odds Ratio '{or_prefix}' in Logistic forest plot: {:?}",
        plain_texts(&sc_log)
    );

    // 3. Poisson GLM (headers: ["beta", "SE", "Wald z", "P value", "Rate Ratio", "95% CI (RR)"])
    let pois_rows = [(2.0, 0.0), (3.0, 0.5), (5.0, 1.0), (7.0, 1.5), (11.0, 2.0), (16.0, 2.5), (22.0, 3.0), (30.0, 3.5)];
    let mut mv_pois = MultipleVariablesTable::new(2, pois_rows.len())?;
    mv_pois.variables[0].title = String::from("Events");
    mv_pois.variables[1].title = String::from("Exposure");
    for (r, (y, x)) in pois_rows.into_iter().enumerate() {
        mv_pois.variables[0].cells[r] = Cell::number(y);
        mv_pois.variables[1].cells[r] = Cell::number(x);
    }
    let ds_pois = DataSheet::new(SheetId::new("pois-mv"), "PoisMV", TableArchetype::MultipleVariables(mv_pois))?;
    let pois_spec =
        GlmSpec { family: GlmFamily::Poisson, response_column: 0, predictor_columns: vec![1], reference_levels: Vec::new(), compute_vif: false };
    let an_pois = AnalysisSpec::new(SheetId::new("res-pois"), ds_pois.id.clone(), AnalysisKind::GeneralizedLinearModel(pois_spec.clone()))?;
    let res_pois = run_glm_analysis(&ds_pois, &an_pois, &pois_spec)?;
    let pois_tab = res_pois.tabs.iter().find(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals).expect("poisson tab");
    let rr_col = pois_tab.column_headers.iter().position(|h| h == "Rate Ratio").expect("RR col");
    let expected_rr1 = match pois_tab.rows[1][rr_col] {
        ResultCell::Number { value, .. } => value,
        _ => panic!("expected numeric RR"),
    };
    let mut doc_pois = ProjectDocument::default();
    doc_pois.add_data_sheet(ds_pois.clone())?;
    doc_pois.add_result_sheet(res_pois.clone())?;
    let mut gs_pois = GraphSheet::new_uninitialized_for_data(SheetId::new("g-pois-forest"), &ds_pois)?;
    gs_pois.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
    gs_pois.spec.primary_source_sheet_id = res_pois.id;
    let sc_pois = compile_graph(&gs_pois, &doc_pois)?;
    assert!(!sc_pois.warnings.iter().any(|w| w.code == "invalid_forest_interval"), "warnings: {:?}", sc_pois.warnings);
    let rr_prefix = format!("{expected_rr1:.2} [");
    assert!(
        plain_texts(&sc_pois).iter().any(|t| t.starts_with(&rr_prefix)),
        "expected Rate Ratio '{rr_prefix}' in Poisson forest plot: {:?}",
        plain_texts(&sc_pois)
    );
    Ok(())
}

#[test]
fn gap_02_03_km_legend_rename_preserves_at_risk_and_censor_ticks() -> TestResult {
    let mut surv = SurvivalTable::new(2, 6)?;
    surv.cohorts[0].title = String::from("Placebo");
    surv.cohorts[1].title = String::from("Drug");
    for (r, (t, s0, s1)) in [(1.0, 1, 0), (2.0, 0, 1), (3.0, 1, 0), (4.0, 0, 1), (5.0, 1, 0), (6.0, 0, 0)].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(f64::from(s0));
        surv.cohorts[1].cells[r][0] = Cell::number(f64::from(s1));
    }
    let ds = DataSheet::new(SheetId::new("surv-ren"), "SurvRename", TableArchetype::Survival(surv))?;
    let km_spec = km_spec_default();
    let mut an = AnalysisSpec::new(SheetId::new("res-km-ren"), ds.id.clone(), AnalysisKind::Survival(km_spec.clone()))?;
    an.selected_data_sets = vec![0, 1];
    let res = run_survival_analysis(&ds, &an, &km_spec)?;

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res.clone())?;

    let mut gs_orig = GraphSheet::new_uninitialized_for_data(SheetId::new("g-km-orig"), &ds)?;
    gs_orig.spec.kind = GraphKind::Survival;
    gs_orig.spec.primary_source_sheet_id = res.id.clone();
    gs_orig.spec.survival_number_at_risk = true;
    let sc_orig = compile_graph(&gs_orig, &doc)?;

    let mut gs_ren = gs_orig.clone();
    gs_ren.spec.series[0].legend_label = Some(String::from("Vehicle (Renamed)"));
    gs_ren.spec.series[1].legend_label = Some(String::from("Active Arm (Renamed)"));
    let sc_ren = compile_graph(&gs_ren, &doc)?;

    let orig_ticks = censor_tick_x_coords(&sc_orig);
    let ren_ticks = censor_tick_x_coords(&sc_ren);
    assert!(!orig_ticks.is_empty(), "baseline result-sourced KM must have censor ticks");
    assert_eq!(ren_ticks.len(), orig_ticks.len(), "renaming legend must preserve all censor ticks");

    let ren_texts = plain_texts(&sc_ren);
    assert!(ren_texts.iter().any(|t| t == "Number at risk"), "NAR header must remain visible after renaming");
    assert!(ren_texts.iter().any(|t| t == "Vehicle (Renamed)"));
    assert!(ren_texts.iter().any(|t| t == "Active Arm (Renamed)"));
    let nar_counts_orig: Vec<String> = sc_orig
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t)
                if t.anchor_pt.y > sc_orig.plot_rect_pt.bottom() + 45.0 && t.rich.plain_text().chars().all(|c| c.is_ascii_digit()) =>
            {
                Some(t.rich.plain_text())
            }
            _ => None,
        })
        .collect();
    let nar_counts_ren: Vec<String> = sc_ren
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t)
                if t.anchor_pt.y > sc_ren.plot_rect_pt.bottom() + 45.0 && t.rich.plain_text().chars().all(|c| c.is_ascii_digit()) =>
            {
                Some(t.rich.plain_text())
            }
            _ => None,
        })
        .collect();
    assert!(!nar_counts_orig.is_empty());
    assert_eq!(nar_counts_ren, nar_counts_orig, "NAR counts must stay identical after legend rename");
    Ok(())
}

#[test]
fn gap_02_04_forest_pooled_diamond_off_for_covariate_tables() -> TestResult {
    // 1. General MultipleVariables table defaults forest_show_pooled_diamond = false.
    let mut mv = MultipleVariablesTable::new(3, 8)?;
    mv.variables[0].title = String::from("Response");
    mv.variables[1].title = String::from("Age");
    mv.variables[2].title = String::from("Dose");
    for r in 0..8 {
        let age = 20.0 + (r as f64) * 5.0;
        let dose = ((r % 3) as f64) * 10.0 + 1.0;
        let y = 5.0 + 0.4 * age + 1.8 * dose + if r % 2 == 0 { 0.5 } else { -0.5 };
        mv.variables[0].cells[r] = Cell::number(y);
        mv.variables[1].cells[r] = Cell::number(age);
        mv.variables[2].cells[r] = Cell::number(dose);
    }
    let ds = DataSheet::new(SheetId::new("mlr-mv"), "MLR", TableArchetype::MultipleVariables(mv))?;
    let default_spec = GraphSpec::default_for_data_sheet(&ds);
    assert!(!default_spec.forest_show_pooled_diamond, "non-meta-analysis MultipleVariables table must default forest_show_pooled_diamond = false");

    // 2. Multiple Linear Regression (GlmFamily::Gaussian) forest plot omits Pooled (FE) by default,
    //    and suppresses + warns even if a legacy spec has forest_show_pooled_diamond = true.
    let glm_spec =
        GlmSpec { family: GlmFamily::Linear, response_column: 0, predictor_columns: vec![1, 2], reference_levels: Vec::new(), compute_vif: false };
    let an = AnalysisSpec::new(SheetId::new("res-mlr"), ds.id.clone(), AnalysisKind::GeneralizedLinearModel(glm_spec.clone()))?;
    let res = run_glm_analysis(&ds, &an, &glm_spec)?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res.clone())?;

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-mlr-for"), &ds)?;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
    gs.spec.primary_source_sheet_id = res.id;
    let sc_default = compile_graph(&gs, &doc)?;
    assert!(!plain_texts(&sc_default).iter().any(|t| t.contains("Pooled")), "MLR forest plot must not render Pooled (FE) by default");

    gs.spec.forest_show_pooled_diamond = true;
    let sc_forced = compile_graph(&gs, &doc)?;
    assert!(
        !plain_texts(&sc_forced).iter().any(|t| t.contains("Pooled")),
        "heterogeneous regression covariates must never be pooled into a FE diamond"
    );
    assert!(
        sc_forced.warnings.iter().any(|w| w.code == "pooled_diamond_suppressed_for_covariates"),
        "expected pooled_diamond_suppressed_for_covariates warning, got {:?}",
        sc_forced.warnings
    );

    // 3. Degenerate zero-width CI on a study-level forest plot warns instead of 1e12 weight explosion.
    let mut grp = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 1, 2)?;
    grp.row_titles = vec![String::from("Study 1"), String::from("ZeroWidth")];
    grp.data_sets[0].cells[0] = vec![Cell::number(1.0), Cell::number(0.5), Cell::number(1.5)];
    grp.data_sets[0].cells[1] = vec![Cell::number(5.0), Cell::number(5.0), Cell::number(5.0)];
    let ds_g = DataSheet::new(SheetId::new("deg-for"), "DegFor", TableArchetype::Grouped(grp))?;
    let mut doc_g = ProjectDocument::default();
    doc_g.add_data_sheet(ds_g.clone())?;
    let mut gs_g = GraphSheet::new_uninitialized_for_data(SheetId::new("g-deg"), &ds_g)?;
    gs_g.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
    gs_g.spec.forest_show_pooled_diamond = true;
    let sc_deg = compile_graph(&gs_g, &doc_g)?;
    assert!(
        sc_deg.warnings.iter().any(|w| w.code == "pooled_diamond_degenerate_ci"),
        "degenerate zero-width CI must warn instead of collapsing pool onto 1e12 weight: {:?}",
        sc_deg.warnings
    );
    assert!(!plain_texts(&sc_deg).iter().any(|t| t.contains("Pooled")));
    Ok(())
}

#[test]
fn gap_02_05_multivariate_unassigned_categorical_color_isolated_from_group_zero() -> TestResult {
    let mut mv = MultipleVariablesTable::new(3, 8)?;
    mv.variables[0].title = String::from("X");
    mv.variables[1].title = String::from("Y");
    mv.variables[2].title = String::from("Group");
    mv.variables[2].lock_role(VariableRole::Categorical);
    // Group 0 ("Alpha") tightly clustered in [1..3, 1..3]
    for (r, (x, y)) in [(1.0, 1.0), (2.0, 2.5), (3.0, 1.5)].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(y);
        mv.variables[2].cells[r] = Cell::text("Alpha");
    }
    // Group 1 ("Beta") in [6..8, 6..8]
    for (i, (x, y)) in [(6.0, 6.0), (7.0, 7.5), (8.0, 6.5)].into_iter().enumerate() {
        let r = 3 + i;
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(y);
        mv.variables[2].cells[r] = Cell::text("Beta");
    }
    // Row 6: blank categorical color at far corner (80, 80)
    mv.variables[0].cells[6] = Cell::number(80.0);
    mv.variables[1].cells[6] = Cell::number(80.0);
    mv.variables[2].cells[6] = Cell::empty();
    // Row 7: excluded categorical color cell at far corner (90, 90)
    mv.variables[0].cells[7] = Cell::number(90.0);
    mv.variables[1].cells[7] = Cell::number(90.0);
    let mut excl = Cell::text("Beta");
    excl.excluded = true;
    mv.variables[2].cells[7] = excl;

    let ds = DataSheet::new(SheetId::new("mv-unassigned"), "MV", TableArchetype::MultipleVariables(mv))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-mv-u"), &ds)?;
    gs.spec.kind = GraphKind::MultipleVariables;
    gs.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Categorical { column: 2, palette: PaletteId::OkabeIto, level_order_override: None },
        size: SizeChannelMapping::Constant { radius_pt: 4.5 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: true,
        show_confidence_ellipses: true,
    });

    let scene = compile_graph(&gs, &doc)?;
    assert!(
        scene.warnings.iter().any(|w| w.code == "unassigned_categorical_color"),
        "blank/excluded categorical color cells must emit unassigned_categorical_color warning: {:?}",
        scene.warnings
    );

    // Unassigned rows (6 and 7) must NOT have Alpha's series_label.
    let unassigned_meta: Vec<_> = scene.plotted_points.iter().filter(|p| p.source_cell.row >= 6).collect();
    assert_eq!(unassigned_meta.len(), 2);
    assert!(unassigned_meta.iter().all(|p| p.series_label == "Unassigned"), "got {unassigned_meta:?}");
    assert!(plain_texts(&scene).iter().any(|t| t == "Unassigned"), "Unassigned legend entry expected");

    // Group 0 ("Alpha") hull & ellipse must stay inside Alpha's screen region, far left of (80, 80).
    let alpha_color = PaletteId::OkabeIto.color_at(0);
    let max_alpha_pt_x = scene.plotted_points.iter().filter(|p| p.series_label == "Alpha").map(|p| p.center_pt.x).fold(f64::NEG_INFINITY, f64::max);
    let min_unassigned_x = unassigned_meta.iter().map(|p| p.center_pt.x).fold(f64::INFINITY, f64::min);
    assert!(min_unassigned_x > max_alpha_pt_x + 50.0);

    let alpha_hull_max_x = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(f), stroke: Some(_), clip: Some(_) }
                if f.color.r == alpha_color.r && f.color.g == alpha_color.g && f.color.b == alpha_color.b && f.color.a == 34 =>
            {
                let mut mx = f64::NEG_INFINITY;
                for cmd in &path.commands {
                    if let PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) = *cmd {
                        mx = mx.max(pt.x);
                    }
                }
                Some(mx)
            }
            _ => None,
        })
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        alpha_hull_max_x.is_finite() && alpha_hull_max_x <= max_alpha_pt_x + 1.0,
        "Group 0 convex hull ({alpha_hull_max_x}) must not include unassigned points near {min_unassigned_x}"
    );
    Ok(())
}

#[test]
fn gap_02_06_contingency_manual_y_max_100_or_1_preserves_raw_counts() -> TestResult {
    let mut cont = ContingencyTable::new(2, 2)?;
    cont.row_titles = vec![String::from("Group A"), String::from("Group B")];
    cont.data_sets[0].title = String::from("Pos");
    cont.data_sets[1].title = String::from("Neg");
    cont.data_sets[0].cells[0][0] = Cell::number(15.0);
    cont.data_sets[1].cells[0][0] = Cell::number(25.0); // row sum = 40
    cont.data_sets[0].cells[1][0] = Cell::number(10.0);
    cont.data_sets[1].cells[1][0] = Cell::number(30.0); // row sum = 40
    let ds = DataSheet::new(SheetId::new("cont-100"), "Cont", TableArchetype::Contingency(cont))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cont-count"), &ds)?;
    gs.spec.kind = GraphKind::Contingency;
    gs.spec.grouped_arrangement = GroupedArrangement::Stacked;
    gs.spec.y_axis.title = String::from("Count");
    gs.spec.y_axis.segments[0].min = Some(0.0);
    gs.spec.y_axis.segments[0].max = Some(100.0);
    let scene_count = compile_graph(&gs, &doc)?;

    // Topmost bar edge for raw counts (sum=40 on [0, 100]) must sit at 40% of plot height (NOT 100%).
    let min_bar_y = scene_count
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } => path
                .commands
                .iter()
                .filter_map(|c| match *c {
                    PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => Some(pt.y),
                    _ => None,
                })
                .reduce(f64::min),
            _ => None,
        })
        .reduce(f64::min)
        .expect("stacked bar rects");
    let expected_y_40 = scene_count.plot_rect_pt.bottom() - 0.40 * scene_count.plot_rect_pt.height;
    assert!(
        (min_bar_y - expected_y_40).abs() <= 1.0,
        "Count chart with manual y_max=100 must keep raw count 40 (y={expected_y_40}), not normalize to 100% (got y={min_bar_y})"
    );

    // Also verify manual y_max = 1.0 on a Count chart preserves raw counts (count=1 stays at 1.0, not normalized to 0.5).
    let mut gs_one = gs.clone();
    gs_one.spec.grouped_arrangement = GroupedArrangement::Interleaved;
    gs_one.spec.y_axis.segments[0].max = Some(1.0);
    let mut cont1 = ContingencyTable::new(2, 1)?;
    cont1.data_sets[0].cells[0][0] = Cell::number(1.0);
    cont1.data_sets[1].cells[0][0] = Cell::number(1.0); // row sum = 2 -> proportion would be 0.5
    let ds1 = DataSheet::new(SheetId::new("cont-1"), "Cont1", TableArchetype::Contingency(cont1))?;
    doc.add_data_sheet(ds1.clone())?;
    gs_one.spec.primary_source_sheet_id = ds1.id;
    let scene_one = compile_graph(&gs_one, &doc)?;
    let min_one_y = scene_one
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } => path
                .commands
                .iter()
                .filter_map(|c| match *c {
                    PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => Some(pt.y),
                    _ => None,
                })
                .reduce(f64::min),
            _ => None,
        })
        .reduce(f64::min)
        .expect("interleaved count=1 bars");
    assert!(
        (min_one_y - scene_one.plot_rect_pt.y).abs() <= 1.0,
        "Count chart with manual y_max=1.0 must keep raw count 1.0 at plot top y={}, not normalize to 0.5 (got {min_one_y})",
        scene_one.plot_rect_pt.y
    );

    // Explicit "Percent (%)" title still normalizes stacked rows to 100.
    let mut gs_pct = gs;
    gs_pct.spec.y_axis.title = String::from("Percent (%)");
    let scene_pct = compile_graph(&gs_pct, &doc)?;
    let min_pct_y = scene_pct
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } => path
                .commands
                .iter()
                .filter_map(|c| match *c {
                    PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => Some(pt.y),
                    _ => None,
                })
                .reduce(f64::min),
            _ => None,
        })
        .reduce(f64::min)
        .expect("pct stacked bars");
    assert!(
        (min_pct_y - scene_pct.plot_rect_pt.y).abs() <= 1.0,
        "Percent (%) stacked contingency chart must normalize to 100% (y={}, got {min_pct_y})",
        scene_pct.plot_rect_pt.y
    );
    Ok(())
}

#[test]
fn gap_02_07_superimposed_layout_scales_subquadratically() -> TestResult {
    let nrows = 2_500;
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, nrows)?;
    for r in 0..nrows {
        grp.data_sets[0].cells[r][0] = Cell::number(((r % 50) + 10) as f64);
        grp.data_sets[1].cells[r][0] = Cell::number(((r % 30) + 5) as f64);
    }
    let ds = DataSheet::new(SheetId::new("sup-big"), "SupBig", TableArchetype::Grouped(grp))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;

    for orient in [curvebook_doc::CartesianOrientation::Vertical, curvebook_doc::CartesianOrientation::Horizontal] {
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-sup"), &ds)?;
        gs.spec.kind = GraphKind::Grouped;
        gs.spec.grouped_arrangement = GroupedArrangement::Superimposed;
        gs.spec.orientation = orient;
        for s in &mut gs.spec.series {
            s.y_error_bars = None;
            if let GraphicalDataObject::BarWithReplicates { replicate_overlay, .. } = &mut s.gdo {
                *replicate_overlay = None;
            }
        }
        let t0 = Instant::now();
        let scene = compile_graph(&gs, &doc)?;
        let elapsed = t0.elapsed();
        assert!(elapsed.as_secs_f64() < 5.0, "5000-slot {orient:?} superimposed compile took {elapsed:?}");
        assert!(all_points_finite(&scene));
    }
    Ok(())
}

#[test]
fn gap_02_08_before_after_missing_cells_keep_slot_extents_aligned() -> TestResult {
    // 3-column table: Before (col 0), Mid (col 1), After (col 2).
    // Row 0 is missing in Before, peaks at 100.0 in Mid, and is 20.0 in After.
    // A bracket between Mid and After must sit strictly above Mid's 100.0 point (smaller y_pt).
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 2)?;
    col.data_sets[0].title = String::from("Before");
    col.data_sets[1].title = String::from("Mid");
    col.data_sets[2].title = String::from("After");
    col.data_sets[0].cells[0][0] = Cell::empty();
    col.data_sets[1].cells[0][0] = Cell::number(100.0);
    col.data_sets[2].cells[0][0] = Cell::number(20.0);
    col.data_sets[0].cells[1][0] = Cell::number(10.0);
    col.data_sets[1].cells[1][0] = Cell::number(15.0);
    col.data_sets[2].cells[1][0] = Cell::number(25.0);

    let ds = DataSheet::new(SheetId::new("ba-miss"), "BAMiss", TableArchetype::Column(col))?;
    let mut res = ResultSheet::new(AnalysisSpec::new(SheetId::new("res-ba-p"), ds.id.clone(), AnalysisKind::OneWayAnova(Default::default()))?)?;
    res.status = ResultStatus::UpToDate;
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: 1,
        group_b: 2,
        label_a: String::from("Mid"),
        label_b: String::from("After"),
        estimate_diff: Some(40.0),
        raw_p_or_q: 0.001,
        is_fdr_q: false,
        significant: true,
    });
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res.clone())?;

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-ba-miss"), &ds)?;
    for s in &mut gs.spec.series {
        s.gdo =
            GraphicalDataObject::BeforeAfterLines { marker: Some(MarkerStyle::default()), line_color: Rgba8::rgb(40, 80, 160), stroke_width_pt: 1.2 };
    }
    gs.spec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: res.id,
        group_a: 1,
        group_b: 2,
        style: BracketStyle::default(),
        label_mode: BracketLabelMode::Custom(String::from("Mid-After-Bracket")),
        hide_if_non_significant: false,
        stroke_width_pt: 1.0,
        font_size_pt: 8.5,
        bracket_step_pt: 14.0,
    });

    let scene = compile_graph(&gs, &doc)?;
    let mid_peak_y = scene.plotted_points.iter().find(|p| p.source_cell.col == 1 && p.source_cell.row == 0).expect("Mid row 0 point").center_pt.y;
    let bracket_label_y = scene
        .primitives
        .iter()
        .find_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text() == "Mid-After-Bracket" => Some(t.anchor_pt.y),
            _ => None,
        })
        .expect("bracket text");
    assert!(
        bracket_label_y < mid_peak_y,
        "bracket label y={bracket_label_y} must sit strictly above Mid peak y={mid_peak_y} (slot extent must not shift onto Before)"
    );

    // Also verify pre-summarized multi-column Grouped Before/After with missing col 0 keeps col 1 extent aligned.
    let mut grp = GroupedTable::new(SubcolumnMode::MeanSdN, 3, 1)?;
    grp.row_titles = vec![String::from("Week 1")];
    grp.data_sets[0].title = String::from("Ctrl");
    grp.data_sets[1].title = String::from("Low");
    grp.data_sets[2].title = String::from("High");
    // Ctrl missing, Low = 90.0 (peak), High = 20.0
    grp.data_sets[0].cells[0] = vec![Cell::empty(), Cell::empty(), Cell::empty()];
    grp.data_sets[1].cells[0] = vec![Cell::number(90.0), Cell::number(2.0), Cell::number(4.0)];
    grp.data_sets[2].cells[0] = vec![Cell::number(20.0), Cell::number(2.0), Cell::number(4.0)];
    let ds_g = DataSheet::new(SheetId::new("g-ba-pre"), "GBAPre", TableArchetype::Grouped(grp))?;
    let mut res_g = ResultSheet::new(AnalysisSpec::new(SheetId::new("res-gba"), ds_g.id.clone(), AnalysisKind::OneWayAnova(Default::default()))?)?;
    res_g.status = ResultStatus::UpToDate;
    res_g.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: 1,
        group_b: 2,
        label_a: String::from("Low:Week 1"),
        label_b: String::from("High:Week 1"),
        estimate_diff: Some(70.0),
        raw_p_or_q: 0.001,
        is_fdr_q: false,
        significant: true,
    });
    let mut doc_g = ProjectDocument::default();
    doc_g.add_data_sheet(ds_g.clone())?;
    doc_g.add_result_sheet(res_g.clone())?;
    let mut gs_g = GraphSheet::new_uninitialized_for_data(SheetId::new("g-gba"), &ds_g)?;
    gs_g.spec.kind = GraphKind::Grouped;
    gs_g.spec.grouped_arrangement = GroupedArrangement::Interleaved;
    for s in &mut gs_g.spec.series {
        s.gdo =
            GraphicalDataObject::BeforeAfterLines { marker: Some(MarkerStyle::default()), line_color: Rgba8::rgb(40, 80, 160), stroke_width_pt: 1.2 };
    }
    gs_g.spec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: res_g.id,
        group_a: 1,
        group_b: 2,
        style: BracketStyle::default(),
        label_mode: BracketLabelMode::Custom(String::from("Low-High-Bracket")),
        hide_if_non_significant: false,
        stroke_width_pt: 1.0,
        font_size_pt: 8.5,
        bracket_step_pt: 14.0,
    });
    let sc_g = compile_graph(&gs_g, &doc_g)?;
    let low_peak_y = sc_g.plotted_points.iter().find(|p| p.source_cell.col == 1).expect("Low point").center_pt.y;
    let grp_bracket_y = sc_g
        .primitives
        .iter()
        .find_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text() == "Low-High-Bracket" => Some(t.anchor_pt.y),
            _ => None,
        })
        .expect("grouped bracket text");
    assert!(grp_bracket_y < low_peak_y, "pre-summarized grouped bracket y={grp_bracket_y} must sit strictly above Low peak y={low_peak_y}");
    Ok(())
}

#[test]
fn gap_02_09_presummarized_box_and_violin_avoid_fabricated_kde_and_iqr() -> TestResult {
    // ColumnTable pre-summarized parametric modes (MeanSdN, MeanSemN, MeanCvN)
    // and GroupedTable parametric/limit modes (MeanSdN, MeanSemN, MeanCvN, MeanLowHigh).
    let check_non_iqr_sheet = |label: &str, ds: &DataSheet| -> TestResult {
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone())?;

        // 1. Violin over PreSummarized must warn and emit NO multi-vertex KDE contour.
        let mut gs_v = GraphSheet::new_uninitialized_for_data(SheetId::new("g-v"), ds)?;
        gs_v.spec.series[0].gdo = GraphicalDataObject::Violin {
            fill_color: Rgba8::rgb(100, 150, 200),
            stroke_color: Rgba8::BLACK,
            clip_to_observed_range: false,
            show_median_and_quartiles: true,
            half_violin_with_points: false,
        };
        let sc_v = compile_graph(&gs_v, &doc)?;
        assert!(
            sc_v.warnings.iter().any(|w| w.code == "violin_unsupported_pre_summarized"),
            "{label} Violin must emit violin_unsupported_pre_summarized: {:?}",
            sc_v.warnings
        );
        let has_kde = sc_v.primitives.iter().any(|p| {
            matches!(p, ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } if path.commands.len() > 10 && matches!(path.commands.last(), Some(PathCmd::Close)))
        });
        assert!(!has_kde, "{label} Violin must never fabricate a KDE polygon from 3 summary values");

        // 2. BoxAndWhisker over parametric/limit PreSummarized must warn and emit NO IQR box rect.
        let mut gs_b = GraphSheet::new_uninitialized_for_data(SheetId::new("g-b"), ds)?;
        gs_b.spec.series[0].gdo = GraphicalDataObject::BoxAndWhisker {
            whiskers: WhiskerMethod::Tukey1_5Iqr,
            fill_color: Rgba8::rgb(120, 160, 210),
            stroke_color: Rgba8::BLACK,
            show_outliers: true,
            show_all_points: false,
        };
        let sc_b = compile_graph(&gs_b, &doc)?;
        assert!(
            sc_b.warnings.iter().any(|w| w.code == "boxplot_unsupported_pre_summarized"),
            "{label} BoxAndWhisker must emit boxplot_unsupported_pre_summarized: {:?}",
            sc_b.warnings
        );
        let has_box_rect = sc_b.primitives.iter().any(|p| {
            matches!(p, ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } if path.commands.len() == 5 && matches!(path.commands.last(), Some(PathCmd::Close)))
        });
        assert!(!has_box_rect, "{label} BoxAndWhisker must not draw an IQR-looking box rectangle");
        Ok(())
    };

    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 1, 1)?;
        col.data_sets[0].cells[0] = vec![Cell::number(20.0), Cell::number(3.0), Cell::number(5.0)];
        let ds = DataSheet::new(SheetId::new("pre-col"), "Pre", TableArchetype::Column(col))?;
        check_non_iqr_sheet(&format!("Column/{mode:?}"), &ds)?;
    }

    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN, SubcolumnMode::MeanLowHigh] {
        let mut grp = GroupedTable::new(mode, 1, 1)?;
        grp.data_sets[0].cells[0] = match mode {
            SubcolumnMode::MeanLowHigh => vec![Cell::number(20.0), Cell::number(25.0), Cell::number(15.0)],
            _ => vec![Cell::number(20.0), Cell::number(3.0), Cell::number(5.0)],
        };
        let ds = DataSheet::new(SheetId::new("pre-grp"), "PreGrp", TableArchetype::Grouped(grp))?;
        check_non_iqr_sheet(&format!("Grouped/{mode:?}"), &ds)?;
    }

    // MedianIqr (Grouped): BoxAndWhisker draws exact Q1..Q3 box cleanly; Violin still warns and emits no KDE.
    let mut grp_iqr = GroupedTable::new(SubcolumnMode::MedianIqr, 1, 1)?;
    grp_iqr.data_sets[0].cells[0] = vec![Cell::number(20.0), Cell::number(15.0), Cell::number(26.0)];
    let ds_iqr = DataSheet::new(SheetId::new("iqr-grp"), "IQR", TableArchetype::Grouped(grp_iqr))?;
    let mut doc_iqr = ProjectDocument::default();
    doc_iqr.add_data_sheet(ds_iqr.clone())?;
    let mut gs_iqr_box = GraphSheet::new_uninitialized_for_data(SheetId::new("g-iqr-b"), &ds_iqr)?;
    gs_iqr_box.spec.series[0].gdo = GraphicalDataObject::BoxAndWhisker {
        whiskers: WhiskerMethod::MinToMax,
        fill_color: Rgba8::rgb(120, 160, 210),
        stroke_color: Rgba8::BLACK,
        show_outliers: false,
        show_all_points: false,
    };
    let sc_iqr_box = compile_graph(&gs_iqr_box, &doc_iqr)?;
    assert!(!sc_iqr_box.warnings.iter().any(|w| w.code == "boxplot_unsupported_pre_summarized"));

    let mut gs_iqr_vio = GraphSheet::new_uninitialized_for_data(SheetId::new("g-iqr-v"), &ds_iqr)?;
    gs_iqr_vio.spec.series[0].gdo = GraphicalDataObject::Violin {
        fill_color: Rgba8::rgb(100, 150, 200),
        stroke_color: Rgba8::BLACK,
        clip_to_observed_range: false,
        show_median_and_quartiles: true,
        half_violin_with_points: false,
    };
    let sc_iqr_vio = compile_graph(&gs_iqr_vio, &doc_iqr)?;
    assert!(sc_iqr_vio.warnings.iter().any(|w| w.code == "violin_unsupported_pre_summarized"));
    let iqr_has_kde = sc_iqr_vio.primitives.iter().any(|p| {
        matches!(p, ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } if path.commands.len() > 10 && matches!(path.commands.last(), Some(PathCmd::Close)))
    });
    assert!(!iqr_has_kde, "MedianIqr Violin must never fabricate a KDE polygon");
    Ok(())
}

#[test]
fn gap_02_10_survival_invalid_cohort_emits_warning() -> TestResult {
    let mut surv = SurvivalTable::new(2, 3)?;
    surv.cohorts[0].title = String::from("ValidArm");
    surv.cohorts[1].title = String::from("BrokenArm");
    for (r, t) in [1.0, 2.0, 3.0].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(1.0);
        surv.cohorts[1].cells[r][0] = Cell::number(1.0);
    }
    // Invalid status code `2.0` on BrokenArm triggers cohort_observations Err.
    surv.cohorts[1].cells[1][0] = Cell::number(2.0);

    let ds = DataSheet::new(SheetId::new("surv-bad"), "SurvBad", TableArchetype::Survival(surv))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    let gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-surv-bad"), &ds)?;
    let scene = compile_graph(&gs, &doc)?;
    assert!(
        scene.warnings.iter().any(|w| w.code == "invalid_survival_cohort" && w.message.contains("BrokenArm")),
        "invalid survival cohort must emit invalid_survival_cohort warning: {:?}",
        scene.warnings
    );
    // Valid cohort still rendered!
    assert!(plain_texts(&scene).iter().any(|t| t == "ValidArm"));
    Ok(())
}

#[test]
fn gap_02_11_volcano_respects_significant_flag_and_difference_units() -> TestResult {
    // MultipleRowTTests with small significant differences (|diff| = 0.35 < 1.0) and a large
    // non-significant row.
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, 3)?;
    grp.row_titles = vec![String::from("SmallUp"), String::from("SmallDown"), String::from("NoisyNS")];
    grp.data_sets[0].title = String::from("Treated");
    grp.data_sets[1].title = String::from("Control");
    // Row 0: Treated ~ 10.35, Control ~ 10.00 -> diff = +0.35, tiny SD -> highly significant
    for (s, (a, b)) in [(10.34, 9.99), (10.36, 10.01), (10.35, 10.00), (10.35, 10.00)].into_iter().enumerate() {
        grp.data_sets[0].cells[0][s] = Cell::number(a);
        grp.data_sets[1].cells[0][s] = Cell::number(b);
    }
    // Row 1: Treated ~ 9.70, Control ~ 10.00 -> diff = -0.30, tiny SD -> highly significant
    for (s, (a, b)) in [(9.69, 10.01), (9.71, 9.99), (9.70, 10.00), (9.70, 10.00)].into_iter().enumerate() {
        grp.data_sets[0].cells[1][s] = Cell::number(a);
        grp.data_sets[1].cells[1][s] = Cell::number(b);
    }
    // Row 2: large mean diff ~ +2.0 but huge variance -> NOT significant
    for (s, (a, b)) in [(2.0, 0.0), (22.0, 20.0), (12.0, 10.0), (12.0, 10.0)].into_iter().enumerate() {
        grp.data_sets[0].cells[2][s] = Cell::number(a);
        grp.data_sets[1].cells[2][s] = Cell::number(b);
    }

    let ds = DataSheet::new(SheetId::new("mr-vol"), "MRVol", TableArchetype::Grouped(grp))?;
    let mr_spec = MultipleRowTestsSpec {
        paired: false,
        nonparametric: false,
        assume_equal_sd: false,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::BenjaminiHochbergFdr, threshold: 0.05, report_confidence_intervals: false },
    };
    let an = AnalysisSpec::new(SheetId::new("res-mr-vol"), ds.id.clone(), AnalysisKind::MultipleRowTTests(mr_spec))?;
    let res = run_multiple_row_t_tests_analysis(&ds, &an, &mr_spec)?;
    assert!(res.pairwise_comparisons[0].significant, "SmallUp must be significant");
    assert!(res.pairwise_comparisons[1].significant, "SmallDown must be significant");
    assert!(!res.pairwise_comparisons[2].significant, "NoisyNS must not be significant");

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res.clone())?;
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-vol-diff"), &ds)?;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Volcano);
    gs.spec.primary_source_sheet_id = res.id;
    let scene = compile_graph(&gs, &doc)?;

    let texts = plain_texts(&scene);
    assert!(texts.contains(&String::from("SmallUp")), "significant |diff|=0.35 discovery label must be shown: {texts:?}");
    assert!(texts.contains(&String::from("SmallDown")), "significant |diff|=0.30 discovery label must be shown: {texts:?}");
    assert!(!texts.contains(&String::from("NoisyNS")), "non-significant row label must stay hidden: {texts:?}");

    // Check value_summary uses "Difference=" rather than hardcoded "log2FC=".
    assert_eq!(scene.plotted_points.len(), 3);
    assert!(
        scene.plotted_points.iter().all(|p| p.value_summary.starts_with("Difference=") && !p.value_summary.contains("log2FC")),
        "MultipleRowTTests volcano value_summary must report Difference=, got {:?}",
        scene.plotted_points.iter().map(|p| &p.value_summary).collect::<Vec<_>>()
    );

    // Verify colored Up (orange) and Down (blue) markers exist for SmallUp / SmallDown.
    let has_up =
        scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { fill: Some(f), clip: Some(_), .. } if f.color == Rgba8::rgb(213, 94, 0)));
    let has_down =
        scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { fill: Some(f), clip: Some(_), .. } if f.color == Rgba8::rgb(0, 114, 178)));
    assert!(has_up && has_down, "both Up and Down significant small-difference points must be colored");
    Ok(())
}

#[test]
fn gap_02_12_nonlinear_residuals_dataset_column_and_filtering() -> TestResult {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 6)?;
    xy.data_sets[0].title = String::from("Drug A");
    xy.data_sets[1].title = String::from("Drug B");
    for r in 0..6 {
        let x = (r + 1) as f64;
        xy.x_cells[r][0] = Cell::number(x);
        if r < 5 {
            xy.data_sets[0].cells[r][0] = Cell::number(2.0 * x + 1.0 + if r % 2 == 0 { 0.1 } else { -0.1 });
        }
        xy.data_sets[1].cells[r][0] = Cell::number(2.0 * x + 5.0 + if r % 2 == 0 { -0.2 } else { 0.2 });
    }
    let ds = DataSheet::new(SheetId::new("nl-multi"), "MultiNL", TableArchetype::Xy(xy))?;
    let mut nl_spec = NonlinearFitSpec::new("straight_line");
    nl_spec.emit_residuals_tab = true;
    nl_spec.parameter_rules.push(ParameterRule {
        name: String::from("Slope"),
        scope: ParameterScope::SharedGlobal,
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    let an = AnalysisSpec::new(SheetId::new("res-nl-multi"), ds.id.clone(), AnalysisKind::NonlinearRegression(nl_spec))?;
    let fit_res = execute_fit_analysis(&an, &ds, &Default::default())?;

    let resid_tab = fit_res.tabs.iter().find(|t| t.id == "nonlinear_residuals").expect("nonlinear_residuals tab");
    assert_eq!(resid_tab.column_headers.first().map(String::as_str), Some("Predicted Y"));
    let ds_col = resid_tab.column_headers.iter().position(|h| h == "Data Set").expect("Data Set column");
    assert_eq!(resid_tab.rows.len(), 11);
    assert_eq!(resid_tab.rows.iter().filter(|r| matches!(r.get(ds_col), Some(ResultCell::Text(s)) if s == "Drug A")).count(), 5);
    assert_eq!(resid_tab.rows.iter().filter(|r| matches!(r.get(ds_col), Some(ResultCell::Text(s)) if s == "Drug B")).count(), 6);

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(fit_res.clone())?;

    for (ds_idx, expected_n) in [(0_usize, 5_usize), (1_usize, 6_usize)] {
        for kind in [DiagnosticPlotKind::ResidualPlot, DiagnosticPlotKind::NormalQqPlot] {
            let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new(format!("g-nl-{ds_idx}")), &ds)?;
            gs.spec.kind = GraphKind::Diagnostic(kind);
            gs.spec.primary_source_sheet_id = fit_res.id.clone();
            gs.spec.series.truncate(1);
            gs.spec.series[0].data_set_index = ds_idx;
            let scene = compile_graph(&gs, &doc)?;
            assert_eq!(scene.plotted_points.len(), expected_n, "{kind:?} ds={ds_idx} must filter to {expected_n} points");
            assert!(
                scene.plotted_points.iter().all(|p| p.source_cell.sheet_id == ds.id && p.source_cell.col == ds_idx),
                "drill-back SourceCellId col must match selected data set {ds_idx}: {:?}",
                scene.plotted_points
            );
            let mut rows: Vec<usize> = scene.plotted_points.iter().map(|p| p.source_cell.row).collect();
            rows.sort_unstable();
            assert_eq!(rows, (0..expected_n).collect::<Vec<_>>(), "drill-back rows must match 0..{expected_n}");
        }
    }

    // Legacy fallback: strip the "Data Set" column to simulate a saved pre-fix table.
    let mut legacy_res = fit_res;
    if let Some(tab) = legacy_res.tabs.iter_mut().find(|t| t.id == "nonlinear_residuals") {
        let _ = tab.column_headers.pop();
        for row in &mut tab.rows {
            let _ = row.pop();
        }
    }
    let mut doc_legacy = ProjectDocument::default();
    doc_legacy.add_data_sheet(ds.clone())?;
    doc_legacy.add_result_sheet(legacy_res.clone())?;
    let mut gs_leg = GraphSheet::new_uninitialized_for_data(SheetId::new("g-leg-1"), &ds)?;
    gs_leg.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot);
    gs_leg.spec.primary_source_sheet_id = legacy_res.id;
    gs_leg.spec.series.truncate(1);
    gs_leg.spec.series[0].data_set_index = 1;
    let sc_leg = compile_graph(&gs_leg, &doc_legacy)?;
    assert_eq!(sc_leg.plotted_points.len(), 6, "legacy residual table without Data Set col must still filter by row label prefix");
    assert!(sc_leg.plotted_points.iter().all(|p| p.source_cell.col == 1));
    Ok(())
}

#[test]
fn gap_02_13_heatmap_enforces_rendering_budget_and_warns() -> TestResult {
    let nrows = 4_000;
    let ncols = 10;
    let mut mv = MultipleVariablesTable::new(ncols, nrows)?;
    for c in 0..ncols {
        mv.variables[c].title = format!("V{c}");
        for r in 0..nrows {
            mv.variables[c].cells[r] = Cell::number(((r + c) % 100) as f64);
        }
    }
    let ds = DataSheet::new(SheetId::new("hm-huge"), "HugeHM", TableArchetype::MultipleVariables(mv))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-hm-huge"), &ds)?;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    gs.spec.heatmap_labels = HeatmapLabelSpec { annotate_cells: true, ..HeatmapLabelSpec::default() };

    let t0 = Instant::now();
    let scene = compile_graph(&gs, &doc)?;
    let elapsed = t0.elapsed();
    assert!(elapsed.as_secs_f64() < 5.0, "capped heatmap took too long: {elapsed:?}");
    assert!(all_points_finite(&scene));
    assert!(
        scene.warnings.iter().any(|w| w.code == "heatmap_cell_budget_exceeded"),
        "expected heatmap_cell_budget_exceeded warning: {:?}",
        scene.warnings
    );
    assert!(
        scene.warnings.iter().any(|w| w.code == "heatmap_cell_annotations_capped"),
        "expected heatmap_cell_annotations_capped warning: {:?}",
        scene.warnings
    );
    assert!(scene.primitives.len() <= 35_000, "heatmap primitives must stay strictly bounded, got {}", scene.primitives.len());
    Ok(())
}

#[test]
fn gap_06_03_dirty_results_omitted_with_pending_warning() -> TestResult {
    // 1. Dirty KM ResultSheet (direct) & Dirty Cox PH (linked via MV sheet)
    let mut surv = SurvivalTable::new(1, 3)?;
    surv.cohorts[0].title = String::from("Arm A");
    for (r, t) in [1.0, 2.0, 3.0].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(1.0);
    }
    let ds_surv = DataSheet::new(SheetId::new("s-dirty"), "Surv", TableArchetype::Survival(surv))?;
    let km_spec = km_spec_default();
    let mut an_km = AnalysisSpec::new(SheetId::new("res-km-dirty"), ds_surv.id.clone(), AnalysisKind::Survival(km_spec.clone()))?;
    an_km.selected_data_sets = vec![0];
    let mut res_km = run_survival_analysis(&ds_surv, &an_km, &km_spec)?;
    res_km.status = ResultStatus::Dirty;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds_surv.clone())?;
    doc.add_result_sheet(res_km.clone())?;

    let mut g_km = GraphSheet::new_uninitialized_for_data(SheetId::new("g-km-dirty"), &ds_surv)?;
    g_km.spec.kind = GraphKind::Survival;
    g_km.spec.primary_source_sheet_id = res_km.id;
    let sc_km = compile_graph(&g_km, &doc)?;
    assert!(sc_km.warnings.iter().any(|w| w.code == "pending_analysis_result"), "dirty KM: {:?}", sc_km.warnings);
    assert!(sc_km.warnings.iter().any(|w| w.code == "empty_survival_data"), "dirty KM must omit stale curves");

    // 2. Dirty Cox PH -> ForestPlot must emit pending_analysis_result and NOT fall back to raw MV columns!
    let ds_cox = build_cox_mv_sheet()?;
    let cox_spec = SurvivalAnalysisSpec::CoxProportionalHazards {
        time_column: 0,
        status_column: 1,
        predictor_columns: vec![2, 3],
        reference_levels: Vec::new(),
        tie_method: curvebook_doc::CoxTieMethod::Efron,
    };
    let an_cox = AnalysisSpec::new(SheetId::new("res-cox-dirty"), ds_cox.id.clone(), AnalysisKind::Survival(cox_spec.clone()))?;
    let mut res_cox = run_survival_analysis(&ds_cox, &an_cox, &cox_spec)?;
    res_cox.status = ResultStatus::Dirty;
    doc.add_data_sheet(ds_cox.clone())?;
    doc.add_result_sheet(res_cox.clone())?;
    for src in [res_cox.id.clone(), ds_cox.id.clone()] {
        let mut g_for = GraphSheet::new_uninitialized_for_data(SheetId::new("g-for-dirty"), &ds_cox)?;
        g_for.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
        g_for.spec.primary_source_sheet_id = src;
        let sc_for = compile_graph(&g_for, &doc)?;
        assert!(sc_for.warnings.iter().any(|w| w.code == "pending_analysis_result"), "dirty Forest: {:?}", sc_for.warnings);
        assert!(sc_for.warnings.iter().any(|w| w.code == "empty_forest"), "dirty Forest must omit stale/raw fallback entries");
    }

    // 3. Dirty ROC ResultSheet
    let mut roc_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    for (r, (a, b)) in [(1.0, 4.0), (2.0, 5.0), (2.5, 6.0), (3.0, 7.0)].into_iter().enumerate() {
        roc_col.data_sets[0].cells[r][0] = Cell::number(a);
        roc_col.data_sets[1].cells[r][0] = Cell::number(b);
    }
    let ds_roc = DataSheet::new(SheetId::new("roc-d"), "ROC", TableArchetype::Column(roc_col))?;
    let roc_spec = RocSpec { ci_method: RocCiMethod::DeLong };
    let an_roc = AnalysisSpec::new(SheetId::new("res-roc-dirty"), ds_roc.id.clone(), AnalysisKind::RocCurve(roc_spec))?;
    let mut res_roc = run_roc_analysis(&ds_roc, &an_roc, &roc_spec)?;
    res_roc.status = ResultStatus::Dirty;
    doc.add_data_sheet(ds_roc.clone())?;
    doc.add_result_sheet(res_roc.clone())?;
    let mut g_roc = GraphSheet::new_uninitialized_for_data(SheetId::new("g-roc-dirty"), &ds_roc)?;
    g_roc.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve);
    g_roc.spec.primary_source_sheet_id = res_roc.id;
    let sc_roc = compile_graph(&g_roc, &doc)?;
    assert!(sc_roc.warnings.iter().any(|w| w.code == "pending_analysis_result"), "dirty ROC: {:?}", sc_roc.warnings);
    assert!(sc_roc.warnings.iter().any(|w| w.code == "empty_roc"));

    // 4. Dirty Bland-Altman ResultSheet
    let ba_spec = BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, ..BlandAltmanSpec::default() };
    let an_ba = AnalysisSpec::new(SheetId::new("res-ba-dirty"), ds_roc.id.clone(), AnalysisKind::BlandAltman(ba_spec))?;
    let mut res_ba = run_bland_altman_analysis(&ds_roc, &an_ba, &ba_spec)?;
    res_ba.status = ResultStatus::Dirty;
    doc.add_result_sheet(res_ba.clone())?;
    let mut g_ba = GraphSheet::new_uninitialized_for_data(SheetId::new("g-ba-dirty"), &ds_roc)?;
    g_ba.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman);
    g_ba.spec.primary_source_sheet_id = res_ba.id;
    let sc_ba = compile_graph(&g_ba, &doc)?;
    assert!(sc_ba.warnings.iter().any(|w| w.code == "pending_analysis_result"), "dirty BA: {:?}", sc_ba.warnings);
    assert!(sc_ba.warnings.iter().any(|w| w.code == "empty_bland_altman"));

    // 5. Dirty NonlinearRegression -> ResidualPlot & NormalQqPlot
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 5)?;
    for r in 0..5 {
        let x = (r + 1) as f64;
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(2.0 * x + 1.0);
    }
    let ds_xy = DataSheet::new(SheetId::new("xy-dirty"), "XY", TableArchetype::Xy(xy))?;
    let mut nl_spec = NonlinearFitSpec::new("straight_line");
    nl_spec.emit_residuals_tab = true;
    let an_nl = AnalysisSpec::new(SheetId::new("res-nl-dirty"), ds_xy.id.clone(), AnalysisKind::NonlinearRegression(nl_spec))?;
    let mut res_nl = execute_fit_analysis(&an_nl, &ds_xy, &Default::default())?;
    res_nl.status = ResultStatus::Dirty;
    doc.add_data_sheet(ds_xy.clone())?;
    doc.add_result_sheet(res_nl.clone())?;
    for kind in [DiagnosticPlotKind::ResidualPlot, DiagnosticPlotKind::NormalQqPlot] {
        let mut g_diag = GraphSheet::new_uninitialized_for_data(SheetId::new("g-nl-dirty"), &ds_xy)?;
        g_diag.spec.kind = GraphKind::Diagnostic(kind);
        g_diag.spec.primary_source_sheet_id = res_nl.id.clone();
        let sc_d = compile_graph(&g_diag, &doc)?;
        assert!(sc_d.warnings.iter().any(|w| w.code == "pending_analysis_result"), "dirty {kind:?}: {:?}", sc_d.warnings);
        assert!(sc_d.plotted_points.is_empty(), "dirty {kind:?} must omit stale points");
    }

    // 6. Dirty Heatmap CorrelationMatrix & Clustering
    let mut hm_mv = MultipleVariablesTable::new(3, 5)?;
    for c in 0..3 {
        hm_mv.variables[c].title = format!("M{c}");
        for r in 0..5 {
            hm_mv.variables[c].cells[r] = Cell::number((r + 1) as f64 * (c + 1) as f64 + (r % 2) as f64);
        }
    }
    let ds_hm = DataSheet::new(SheetId::new("hm-dirty"), "HM", TableArchetype::MultipleVariables(hm_mv))?;
    let cl_spec = MultivariateSpec::Clustering {
        distance: ClusterDistance::Manhattan,
        linkage: ClusterLinkage::Complete,
        k_means_k: Some(2),
        k_sweep_max: None,
    };
    let an_cl = AnalysisSpec::new(SheetId::new("res-cl-dirty"), ds_hm.id.clone(), AnalysisKind::Multivariate(cl_spec.clone()))?;
    let mut res_cl = run_multivariate_analysis(&ds_hm, &an_cl, &cl_spec)?;
    res_cl.status = ResultStatus::Dirty;
    let corr_spec = MultivariateSpec::CorrelationMatrix { spearman_rank: false };
    let an_corr = AnalysisSpec::new(SheetId::new("res-corr-dirty"), ds_hm.id.clone(), AnalysisKind::Multivariate(corr_spec.clone()))?;
    let mut res_corr = run_multivariate_analysis(&ds_hm, &an_corr, &corr_spec)?;
    res_corr.status = ResultStatus::Dirty;
    doc.add_data_sheet(ds_hm.clone())?;
    doc.add_result_sheet(res_cl)?;
    doc.add_result_sheet(res_corr.clone())?;

    let mut g_hm_corr = GraphSheet::new_uninitialized_for_data(SheetId::new("g-hm-corr"), &ds_hm)?;
    g_hm_corr.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    g_hm_corr.spec.primary_source_sheet_id = res_corr.id;
    let sc_hm_corr = compile_graph(&g_hm_corr, &doc)?;
    assert!(sc_hm_corr.warnings.iter().any(|w| w.code == "pending_analysis_result"), "dirty corr heatmap: {:?}", sc_hm_corr.warnings);
    assert!(sc_hm_corr.warnings.iter().any(|w| w.code == "empty_heatmap"));

    let mut g_hm_cl = GraphSheet::new_uninitialized_for_data(SheetId::new("g-hm-cl"), &ds_hm)?;
    g_hm_cl.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    g_hm_cl.spec.primary_source_sheet_id = ds_hm.id;
    let sc_hm_cl = compile_graph(&g_hm_cl, &doc)?;
    assert!(sc_hm_cl.warnings.iter().any(|w| w.code == "pending_analysis_result"), "dirty clustering heatmap: {:?}", sc_hm_cl.warnings);
    assert!(!plain_texts(&sc_hm_cl).iter().any(|t| t.contains("Manhattan")), "dirty clustering cue must not leak into heatmap");

    // 7. Dirty Volcano MultipleRowTTests
    let mut grp_v = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2)?;
    for r in 0..2 {
        for s in 0..3 {
            grp_v.data_sets[0].cells[r][s] = Cell::number(10.0 + (s as f64));
            grp_v.data_sets[1].cells[r][s] = Cell::number(5.0 + (s as f64));
        }
    }
    let ds_v = DataSheet::new(SheetId::new("v-dirty"), "VDirty", TableArchetype::Grouped(grp_v))?;
    let mr_spec = MultipleRowTestsSpec {
        paired: false,
        nonparametric: false,
        assume_equal_sd: false,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::BenjaminiHochbergFdr, threshold: 0.05, report_confidence_intervals: false },
    };
    let an_v = AnalysisSpec::new(SheetId::new("res-v-dirty"), ds_v.id.clone(), AnalysisKind::MultipleRowTTests(mr_spec))?;
    let mut res_v = run_multiple_row_t_tests_analysis(&ds_v, &an_v, &mr_spec)?;
    res_v.status = ResultStatus::Dirty;
    doc.add_data_sheet(ds_v.clone())?;
    doc.add_result_sheet(res_v.clone())?;
    let mut g_v = GraphSheet::new_uninitialized_for_data(SheetId::new("g-v-dirty"), &ds_v)?;
    g_v.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Volcano);
    g_v.spec.primary_source_sheet_id = res_v.id;
    let sc_v = compile_graph(&g_v, &doc)?;
    assert!(sc_v.warnings.iter().any(|w| w.code == "pending_analysis_result"), "dirty volcano: {:?}", sc_v.warnings);
    assert!(sc_v.warnings.iter().any(|w| w.code == "empty_volcano"));

    // 8. Dirty Contingency upstream transformed ResultSheet + Dirty scalar-only extract_sheet_grid
    let mut cont = ContingencyTable::new(2, 2)?;
    cont.data_sets[0].cells[0][0] = Cell::number(10.0);
    cont.data_sets[1].cells[0][0] = Cell::number(20.0);
    cont.data_sets[0].cells[1][0] = Cell::number(15.0);
    cont.data_sets[1].cells[1][0] = Cell::number(25.0);
    let ds_cont = DataSheet::new(SheetId::new("cont-src"), "ContSrc", TableArchetype::Contingency(cont.clone()))?;
    let mut res_cont_tx =
        ResultSheet::new(AnalysisSpec::new(SheetId::new("res-cont-dirty"), ds_cont.id.clone(), AnalysisKind::DescriptiveStats(Default::default()))?)?;
    res_cont_tx.status = ResultStatus::Dirty;
    res_cont_tx.analyzable_data = Some(DataSheet::new(SheetId::new("cont-tx-stale"), "Stale", TableArchetype::Contingency(cont))?);
    res_cont_tx.scalars.push(curvebook_doc::ResultScalarEntry { key: String::from("STALE_SCALAR"), data_set_index: None, value: 999.0, units: None });
    doc.add_data_sheet(ds_cont.clone())?;
    doc.add_result_sheet(res_cont_tx.clone())?;
    let mut g_cont_d = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cont-dirty"), &ds_cont)?;
    g_cont_d.spec.kind = GraphKind::Contingency;
    g_cont_d.spec.primary_source_sheet_id = res_cont_tx.id.clone();
    let sc_cont_d = compile_graph(&g_cont_d, &doc)?;
    assert!(sc_cont_d.warnings.iter().any(|w| w.code == "pending_analysis_result"), "dirty contingency: {:?}", sc_cont_d.warnings);
    assert!(sc_cont_d.warnings.iter().any(|w| w.code == "empty_contingency_data"));

    // Embedded scalar-only Dirty ResultSheet must not leak via extract_sheet_grid (`GAP-01-03` shared.rs).
    let mut g_embed = GraphSheet::new_uninitialized_for_data(SheetId::new("g-embed-dirty"), &ds_xy)?;
    g_embed.spec.annotations.push(curvebook_doc::GraphAnnotation::EmbeddedResultTable {
        result_sheet_id: res_cont_tx.id,
        tab_id: None,
        bounds_pt: curvebook_doc::RectPt::new(150.0, 20.0, 120.0, 60.0),
        selected_rows: None,
        selected_cols: None,
        style: Default::default(),
    });
    let sc_embed = compile_graph(&g_embed, &doc)?;
    assert!(!plain_texts(&sc_embed).iter().any(|t| t.contains("STALE_SCALAR")), "Dirty scalar grid must not leak via extract_sheet_grid");

    // 9. Survival Log10 X axis with minor_ticks_per_interval = 0 compiles cleanly (GAP-01-07 check).
    let mut g_surv_log = GraphSheet::new_uninitialized_for_data(SheetId::new("g-surv-log"), &ds_surv)?;
    g_surv_log.spec.kind = GraphKind::Survival;
    g_surv_log.spec.survival_number_at_risk = true;
    g_surv_log.spec.x_axis.segments[0].scale = AxisScale::Log10;
    g_surv_log.spec.x_axis.segments[0].min = Some(0.5);
    g_surv_log.spec.x_axis.segments[0].max = Some(10.0);
    g_surv_log.spec.x_axis.segments[0].minor_ticks_per_interval = 0;
    let sc_surv_log = compile_graph(&g_surv_log, &doc)?;
    assert!(all_points_finite(&sc_surv_log));
    let _ = ConfidenceLevel::DEFAULT_95;
    Ok(())
}
