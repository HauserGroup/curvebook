#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Regression suite for Wave 2 audit findings in `curvebook-fit`:
//! #51, #52, #53, #80, #93, #94, #100, #114, #115, #116, and #121 (issue #11).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, DataSheet, FitMethod, HookSource, LinearRegressionSpec, ModelComparisonSpec,
    MultipleVariablesTable, NonlinearFitSpec, ParameterRule, ParameterScope, ResultCell, SheetId, SubcolumnMode, TableArchetype, VariableRole,
    WeightingScheme, XColumnMode, XyTable,
};
use curvebook_fit::diagnostics::{anderson_darling_test, compute_profile_likelihood_cis, rout_detect_outliers};
use curvebook_fit::dsl::CompiledDslModel;
use curvebook_fit::error::{FitError, FitIdentifiability, FitStatus, ParameterNote};
use curvebook_fit::library::get_builtin_model;
use curvebook_fit::result::execute_analysis;
use curvebook_fit::solver::{
    FittedParameterSummary, InferenceAvailability, ModelComparisonOutcome, NonlinearFitResult, SolverConfig, compare_models, compare_models_checked,
    fit_nonlinear,
};
use curvebook_fit::types::{
    DataPoint, DatasetSeries, HookContext, MultivarExtractionSpec, extract_datasets_from_sheet_with_options, parse_leading_number,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::norm_ppf;
use curvebook_testkit::metrics::assert_approx_eq;

fn series_from_xy(idx: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
    let points = xs
        .iter()
        .zip(ys.iter())
        .enumerate()
        .map(|(row, (&x, &y))| DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: idx, row })
        .collect();
    DatasetSeries {
        data_set_index: idx,
        label: label.to_owned(),
        points,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn find_param<'a>(res: &'a NonlinearFitResult, name: &str) -> &'a FittedParameterSummary {
    res.parameters.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("missing parameter `{name}` in {:?}", res.parameters))
}

fn rule_const(name: &str, val: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Constant(val), initial_override: None, lower_bound: None, upper_bound: None }
}

fn rule_shared(name: &str) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::SharedGlobal, initial_override: None, lower_bound: None, upper_bound: None }
}

fn rule_local_init(name: &str, init: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Local, initial_override: Some(init), lower_bound: None, upper_bound: None }
}

// ============================================================================
// #51: Runs test computed per dataset in own X order & combined without interleaving
// ============================================================================

#[test]
fn audit_51_runs_test_per_dataset_no_interleaving() {
    let xs: Vec<f64> = (0..=12).map(f64::from).collect();
    let ys_a: Vec<f64> = xs.iter().map(|&x| (x - 6.0).powi(2)).collect();
    let ys_b: Vec<f64> = xs.iter().map(|&x| 50.0 - (x - 6.0).powi(2)).collect();

    let model = get_builtin_model("straight_line").unwrap();
    let ds_a = series_from_xy(0, "CurveA", &xs, &ys_a);
    let ds_b = series_from_xy(1, "CurveB", &xs, &ys_b);

    let fit_a = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds_a), &HookContext::default(), &SolverConfig::default()).unwrap();
    let fit_b = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds_b), &HookContext::default(), &SolverConfig::default()).unwrap();
    let single_a_runs = fit_a.runs_test.as_ref().unwrap();
    let single_b_runs = fit_b.runs_test.as_ref().unwrap();
    assert_eq!(single_a_runs.runs, 3);
    assert_eq!(single_b_runs.runs, 3);
    assert!(single_a_runs.p_too_few_runs < 0.01 && single_a_runs.deviates_significantly);
    assert!(single_b_runs.p_too_few_runs < 0.01 && single_b_runs.deviates_significantly);

    // Independent curves and shared-slope multi-curve fit must both preserve per-curve runs.
    for rules in [vec![], vec![rule_shared("Slope")]] {
        let cfg = SolverConfig { parameter_rules: rules, ..SolverConfig::default() };
        let fit = fit_nonlinear(model.as_ref(), &[ds_a.clone(), ds_b.clone()], &HookContext::default(), &cfg).unwrap();
        assert_eq!(fit.datasets.len(), 2);
        let ra = fit.datasets[0].runs_test.as_ref().expect("per-dataset runs A");
        let rb = fit.datasets[1].runs_test.as_ref().expect("per-dataset runs B");
        assert_eq!(ra.runs, 3);
        assert_eq!(rb.runs, 3);
        assert!(ra.p_too_few_runs < 0.01 && ra.deviates_significantly);
        assert!(rb.p_too_few_runs < 0.01 && rb.deviates_significantly);

        let combined = fit.runs_test.as_ref().expect("combined runs test");
        assert_eq!(combined.runs, 6, "sum of within-curve runs must be 6, never interleaved 24");
        assert!(combined.p_too_few_runs < 0.001 && combined.deviates_significantly);
    }

    // Check ResultSheet formatting via execute_analysis on a 2-dataset XY sheet.
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 2, xs.len()).unwrap();
    xy.data_sets[0].title = String::from("CurveA");
    xy.data_sets[1].title = String::from("CurveB");
    for (r, ((&x, &ya), &yb)) in xs.iter().zip(ys_a.iter()).zip(ys_b.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(ya);
        xy.data_sets[1].cells[r][0] = Cell::number(yb);
    }
    let sheet = DataSheet::new(SheetId::new("xy-51"), "XY Runs", TableArchetype::Xy(xy)).unwrap();
    let spec = AnalysisSpec::new("res-51", SheetId::new("xy-51"), AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).unwrap();
    let rs = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();

    let gof = rs.tabs.iter().find(|t| t.id == "nonlinear_gof").unwrap();
    assert!(gof.row_labels.contains(&"Runs test (CurveA) P value".to_owned()));
    assert!(gof.row_labels.contains(&"Runs test (CurveB) P value".to_owned()));
    let runs_checks: Vec<_> = rs.checklist.iter().filter(|c| c.assumption.starts_with("Runs test")).collect();
    assert_eq!(runs_checks.len(), 3);
    assert!(runs_checks.iter().all(|c| c.status == ChecklistStatus::Warn));
}

// ============================================================================
// #52: Normality & homoscedasticity use weighted residuals when weighting active
// ============================================================================

#[test]
fn audit_52_weighted_normality_and_homoscedasticity() {
    let n = 30_usize;
    // Fixed permutation of exact standard-normal Blom quantiles so 1/Y^2 residuals are Gaussian.
    let z_sorted: Vec<f64> = (0..n).map(|i| norm_ppf(((i as f64) + 0.375) / ((n as f64) + 0.25)).unwrap()).collect();
    let xs: Vec<f64> = (0..n).map(|i| 1.25_f64.powi(i as i32)).collect();
    let mut ys = Vec::with_capacity(n);
    for (i, &x) in xs.iter().enumerate() {
        let perm = (i * 7 + 3) % n;
        ys.push(2.0 * x * (1.0 + 0.05 * z_sorted[perm]));
    }

    let model = CompiledDslModel::compile("line_origin", "Y = Slope * X\n# Inits: Slope=2.0").unwrap();
    let ds = series_from_xy(0, "Heteroscedastic", &xs, &ys);
    let cfg = SolverConfig {
        weighting: WeightingScheme::RelativeSquare,
        compute_profile_ci: false,
        compute_normality: true,
        compute_homoscedasticity: true,
        ..SolverConfig::default()
    };

    let fit = fit_nonlinear(&model, &[ds], &HookContext::default(), &cfg).unwrap();
    let sw = fit.normality.as_ref().expect("Shapiro-Wilk");
    let ad = fit.normality_anderson_darling.as_ref().expect("Anderson-Darling");
    let ks = fit.normality_lilliefors_ks.as_ref().expect("Lilliefors");
    let homo = fit.homoscedasticity.as_ref().expect("Spearman homoscedasticity");

    assert!(sw.passed && sw.statistic > 0.96 && sw.p_value > 0.3, "unexpected weighted SW: {sw:?}");
    assert!(ad.passed && ad.p_value > 0.3, "unexpected weighted AD: {ad:?}");
    assert!(ks.passed && ks.p_value > 0.2, "unexpected weighted KS: {ks:?}");
    assert!(homo.homoscedastic && homo.spearman_rho.abs() < 0.35 && homo.p_value > 0.05, "unexpected weighted Spearman: {homo:?}");
}

// ============================================================================
// #53: Linear regression slope CI label matches requested confidence level
// ============================================================================

#[test]
fn audit_53_linear_slope_ci_label_uses_confidence_level() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let ys = [2.3, 4.1, 6.4, 7.7, 10.6, 11.8, 14.5, 15.6];
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, xs.len()).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new(SheetId::new("xy-53"), "Linear CI", TableArchetype::Xy(xy)).unwrap();

    let mut spec_99 = AnalysisSpec::new("res-99", SheetId::new("xy-53"), AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
    spec_99.confidence_level = ConfidenceLevel::new(99.0).unwrap();
    let rs_99 = execute_analysis(&spec_99, &sheet, &HookContext::default()).unwrap();
    let tab_99 = rs_99.tabs.iter().find(|t| t.id == "linear_summary").unwrap();
    assert!(tab_99.row_labels.contains(&"99% CI Slope".to_owned()), "got labels {:?}", tab_99.row_labels);
    assert!(!tab_99.row_labels.contains(&"95% CI Slope".to_owned()));
    let ci_idx = tab_99.row_labels.iter().position(|l| l == "99% CI Slope").unwrap();
    let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo99), upper: CiBound::Finite(hi99) } = tab_99.rows[ci_idx][0] else {
        panic!("expected finite 99% CI");
    };
    assert_approx_eq(lo99, 1.7272, 5e-4, 5e-4, "99% CI lower matches R confint");
    assert_approx_eq(hi99, 2.1823, 5e-4, 5e-4, "99% CI upper matches R confint");

    let mut spec_90 = AnalysisSpec::new("res-90", SheetId::new("xy-53"), AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
    spec_90.confidence_level = ConfidenceLevel::new(90.0).unwrap();
    let rs_90 = execute_analysis(&spec_90, &sheet, &HookContext::default()).unwrap();
    let tab_90 = rs_90.tabs.iter().find(|t| t.id == "linear_summary").unwrap();
    assert!(tab_90.row_labels.contains(&"90% CI Slope".to_owned()));

    let dem_lin = LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() };
    let mut spec_dem = AnalysisSpec::new("res-dem99", SheetId::new("xy-53"), AnalysisKind::LinearRegression(dem_lin)).unwrap();
    spec_dem.confidence_level = ConfidenceLevel::new(99.0).unwrap();
    let rs_dem = execute_analysis(&spec_dem, &sheet, &HookContext::default()).unwrap();
    let tab_dem = rs_dem.tabs.iter().find(|t| t.id == "deming_summary").unwrap();
    assert!(tab_dem.row_labels.contains(&"99% CI Slope".to_owned()));
    assert!(tab_dem.row_labels.contains(&"99% CI Intercept".to_owned()));
}

// ============================================================================
// #80: Reject ROUT Q <= 0 or Q > 50 and display exact Q used
// ============================================================================

#[test]
fn audit_80_rout_rejects_invalid_q_and_prints_exact_q() {
    let xs: Vec<f64> = (0..12).map(f64::from).collect();
    let mut ys: Vec<f64> = xs.iter().map(|&x| 1.0 + 2.0 * x).collect();
    ys[11] += 25.0;
    let ds = series_from_xy(0, "Outliers", &xs, &ys);
    let model = get_builtin_model("straight_line").unwrap();

    for bad_q in [0.0, -0.5, 50.001, 80.0, 100.0, f64::NAN, f64::INFINITY] {
        let cfg = SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: bad_q }, ..SolverConfig::default() };
        let err = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg).unwrap_err();
        assert!(matches!(err, FitError::InvalidParameter { .. }), "expected InvalidParameter for Q={bad_q}, got {err:?}");

        let res: Vec<f64> = ys.iter().zip(xs.iter()).map(|(&y, &x)| y - (1.0 + 2.0 * x)).collect();
        let err_diag = rout_detect_outliers(&ds.points, &res, 2, bad_q).unwrap_err();
        assert!(matches!(err_diag, FitError::InvalidParameter { .. }), "diag expected InvalidParameter for Q={bad_q}, got {err_diag:?}");
    }

    // Valid Q = 0.001% must execute without truncation and appear exact in checklist.
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, xs.len()).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new(SheetId::new("xy-80"), "ROUT Exact Q", TableArchetype::Xy(xy)).unwrap();
    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 0.001 };
    let spec = AnalysisSpec::new("res-80", SheetId::new("xy-80"), AnalysisKind::NonlinearRegression(nl)).unwrap();
    let rs = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    let rout_item = rs.checklist.iter().find(|c| c.assumption == "ROUT outlier detection").unwrap();
    let metric = rout_item.diagnostic_metric.as_deref().unwrap();
    assert!(metric.contains("Q = 0.001%"), "expected exact Q = 0.001% in `{metric}`");
}

// ============================================================================
// #93: MV X/Y persisted bindings never retarget when calculated variable is added
// ============================================================================

#[test]
fn audit_93_mv_explicit_bindings_resist_added_calculated_variable() {
    let mut mv = MultipleVariablesTable::new(2, 4).unwrap();
    mv.variables[0].title = String::from("Dose");
    mv.variables[0].lock_role(VariableRole::Continuous);
    mv.variables[1].title = String::from("Resp");
    mv.variables[1].lock_role(VariableRole::Continuous);
    for (r, x) in [1.0, 2.0, 3.0, 4.0].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(2.0 * x + 1.0);
    }
    let sheet_before = DataSheet::new(SheetId::new("mv-93"), "MV", TableArchetype::MultipleVariables(mv.clone())).unwrap();

    // Extended MV table after user adds calculated column `DoseSq = [Dose]^2` (col 2).
    let mut mv_after = mv;
    mv_after.variables.push(curvebook_doc::VariableColumn {
        title: String::from("DoseSq"),
        role: VariableRole::Continuous,
        role_inferred: false,
        source: curvebook_doc::VariableSource::Calculated { formula: String::from("[Dose]*[Dose]") },
        level_order: curvebook_doc::LevelOrderSpec::default(),
        cells: [1.0, 4.0, 9.0, 16.0].into_iter().map(Cell::number).collect(),
    });
    let sheet_after = DataSheet::new(SheetId::new("mv-93"), "MV", TableArchetype::MultipleVariables(mv_after)).unwrap();

    // 1. Nonlinear with persisted mv_x_columns = [0], mv_y_column = Some(1)
    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.mv_x_columns = vec![0];
    nl.mv_y_column = Some(1);
    let nl_spec = AnalysisSpec::new("nl-93", SheetId::new("mv-93"), AnalysisKind::NonlinearRegression(nl)).unwrap();
    let res_b = execute_analysis(&nl_spec, &sheet_before, &HookContext::default()).unwrap();
    let res_a = execute_analysis(&nl_spec, &sheet_after, &HookContext::default()).unwrap();
    assert_approx_eq(res_b.lookup_scalar("Slope", None).unwrap(), 2.0, 1e-10, 1e-10, "NL before slope");
    assert_approx_eq(res_a.lookup_scalar("Slope", None).unwrap(), 2.0, 1e-10, 1e-10, "NL after slope unchanged");
    assert_approx_eq(res_a.lookup_scalar("YIntercept", None).unwrap(), 1.0, 1e-10, 1e-10, "NL after intercept unchanged");

    // 2. Partial explicit X + selected_data_sets=[0,1] (no longer scans new calc col 2 for Y)
    let partial_x = extract_datasets_from_sheet_with_options(
        &sheet_after,
        &[0, 1],
        &MultivarExtractionSpec { group_columns: &[], x_columns: &[0], y_column: None },
    )
    .unwrap();
    assert_approx_eq(partial_x[0].points[2].y, 7.0, 1e-12, 1e-12, "partial explicit X keeps selected col 1 as Y");

    let partial_y = extract_datasets_from_sheet_with_options(
        &sheet_after,
        &[0],
        &MultivarExtractionSpec { group_columns: &[], x_columns: &[], y_column: Some(1) },
    )
    .unwrap();
    assert_approx_eq(partial_y[0].points[2].x[0], 3.0, 1e-12, 1e-12, "partial explicit Y keeps selected col 0 as X");

    // 3. LinearRegression with persisted selected_data_sets = [0, 1]
    let lin = LinearRegressionSpec::default();
    let mut lin_spec = AnalysisSpec::new("lin-93", SheetId::new("mv-93"), AnalysisKind::LinearRegression(lin)).unwrap();
    lin_spec.selected_data_sets = vec![0, 1];
    let lin_after = execute_analysis(&lin_spec, &sheet_after, &HookContext::default()).unwrap();
    assert_approx_eq(lin_after.lookup_scalar("Slope", None).unwrap(), 2.0, 1e-10, 1e-10, "Linear after calc col stays slope=2");
    assert_approx_eq(lin_after.lookup_scalar("YIntercept", None).unwrap(), 1.0, 1e-10, 1e-10, "Linear after calc col stays intercept=1");
}

// ============================================================================
// #94: Strict column-title numeric hook parsing
// ============================================================================

#[test]
fn audit_94_strict_column_title_numeric_hook() {
    for (title, expected) in
        [("10", 10.0), ("-2.5", -2.5), ("1e-7", 1e-7), ("10 nM", 10.0), ("[I] = 10 nM", 10.0), ("Conc = 2.5 uM", 2.5), ("5%", 5.0)]
    {
        assert_approx_eq(parse_leading_number(title).unwrap(), expected, 1e-12, 1e-12, title);
    }
    let bad_titles = ["Inhibitor-10 nM", "IL-6 10 ng/mL", "CO2 5%", "Hot1.5", "10-20 nM", "10 20", "", "NaN", "inf"];
    for bad in bad_titles {
        assert_eq!(parse_leading_number(bad), None, "ambiguous title `{bad}` must be rejected");
    }

    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 4).unwrap();
    xy.data_sets[0].title = String::from("Inhibitor-10 nM");
    for r in 0..4 {
        xy.set_x_cell(r, Cell::number(r as f64)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 + 2.0 * (r as f64));
    }
    let ds_id = SheetId::new("xy-94");
    let sheet = DataSheet::new(ds_id.clone(), "AmbiguousTitle", TableArchetype::Xy(xy)).unwrap();
    let ctx = HookContext { data_sheets: &[sheet], ..HookContext::default() };
    let err = ctx.resolve(&HookSource::ColumnTitleValue { data_sheet_id: ds_id.clone(), data_set_index: 0 }).unwrap_err();
    assert!(matches!(err, FitError::HookResolutionError { .. }), "expected HookResolutionError, got {err:?}");

    let model = CompiledDslModel::compile("m", "Y = A + B*X").unwrap();
    let ds = series_from_xy(0, "IL-6 10 ng/mL", &[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]);
    let cfg = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: String::from("A"),
            scope: ParameterScope::ColumnTitleConstant,
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let err_ct = fit_nonlinear(&model, &[ds], &HookContext::default(), &cfg).unwrap_err();
    assert!(matches!(err_ct, FitError::HookResolutionError { .. }), "expected HookResolutionError, got {err_ct:?}");
}

// ============================================================================
// #100: Anderson-Darling P value stays monotone non-increasing for large A*
// ============================================================================

#[test]
fn audit_100_anderson_darling_monotone_for_large_a2_star() {
    // 1. 999 zeros + single 1
    let mut sparse = vec![0.0_f64; 1000];
    sparse[999] = 1.0;
    let ad_sparse = anderson_darling_test(&sparse).unwrap();
    assert!(ad_sparse.statistic > 350.0, "expected A* ~ 385.8, got {}", ad_sparse.statistic);
    assert!(ad_sparse.p_value < 0.05 && !ad_sparse.passed, "sparse spike must fail normality cleanly, got {ad_sparse:?}");

    // 2. Deterministic seeded lognormal sample (sigma = 2.0, n = 3000)
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next_u01 = || -> f64 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (((state >> 11) as f64) + 0.5) / ((1_u64 << 53) as f64)
    };
    let mut lognorm = Vec::with_capacity(3000);
    for _ in 0..1500 {
        let u1 = next_u01().clamp(1e-12, 1.0 - 1e-12);
        let u2 = next_u01();
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = core::f64::consts::TAU * u2;
        lognorm.push((2.0 * r * theta.cos()).exp());
        lognorm.push((2.0 * r * theta.sin()).exp());
    }
    let ad_ln = anderson_darling_test(&lognorm).unwrap();
    assert!(ad_ln.statistic > 500.0, "expected large A*, got {}", ad_ln.statistic);
    assert!(ad_ln.p_value < 0.05 && !ad_ln.passed, "lognormal must fail AD normality, got {ad_ln:?}");

    // 3. Verify P is non-increasing as sample non-normality A* crosses 100 -> 230 -> 385+
    let exp_sample = |n: usize| -> Vec<f64> { (0..n).map(|i| -(((i as f64) + 0.5) / (n as f64)).ln()).collect() };
    let r_small = anderson_darling_test(&exp_sample(1500)).unwrap();
    let r_mid = anderson_darling_test(&exp_sample(3600)).unwrap();
    assert!(r_small.statistic < r_mid.statistic && r_mid.statistic < ad_sparse.statistic);
    assert!(
        r_small.p_value >= r_mid.p_value && r_mid.p_value >= ad_sparse.p_value,
        "P must stay non-increasing: {} -> {} -> {}",
        r_small.p_value,
        r_mid.p_value,
        ad_sparse.p_value
    );
}

// ============================================================================
// #114: Parameter rules reject unknown and duplicate names
// ============================================================================

#[test]
fn audit_114_parameter_rules_reject_unknown_and_duplicate() {
    let model = get_builtin_model("straight_line").unwrap();
    let ds = series_from_xy(0, "Line", &[1.0, 2.0, 3.0, 4.0], &[3.0, 5.0, 7.0, 9.0]);

    let cfg_unknown = SolverConfig { parameter_rules: vec![rule_const("NotAParam", 1.0)], ..SolverConfig::default() };
    let err_u = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg_unknown).unwrap_err();
    assert!(matches!(err_u, FitError::InvalidParameter { name: "parameter_rules", .. }), "got {err_u:?}");

    let cfg_dup = SolverConfig { parameter_rules: vec![rule_const("Slope", 2.0), rule_const("slope", 3.0)], ..SolverConfig::default() };
    let err_d = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg_dup).unwrap_err();
    assert!(matches!(err_d, FitError::InvalidParameter { name: "parameter_rules", .. }), "got {err_d:?}");
}

// ============================================================================
// #87 (follow-up to #114): compare_models / compare_models_checked filter primary-only parameter_rules
// ============================================================================

#[test]
fn audit_87_compare_models_filters_primary_only_parameter_rules_for_alt_model() {
    let model_4pl = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let model_3pl = get_builtin_model("log_agonist_vs_response_3pl").unwrap();
    let xs = [-9.5_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let ys_a: Vec<f64> =
        xs.iter().enumerate().map(|(i, &x)| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.3)) + if i % 2 == 0 { 0.4 } else { -0.4 }).collect();
    let ys_b: Vec<f64> =
        xs.iter().enumerate().map(|(i, &x)| 10.0 + 85.0 / (1.0 + 10.0_f64.powf((-5.8 - x) * 1.3)) + if i % 2 == 0 { -0.4 } else { 0.4 }).collect();
    let datasets = vec![series_from_xy(0, "WT", &xs, &ys_a), series_from_xy(1, "Mutant", &xs, &ys_b)];

    let cfg = SolverConfig {
        parameter_rules: vec![
            ParameterRule {
                name: String::from("HillSlope"),
                scope: ParameterScope::SharedGlobal,
                initial_override: None,
                lower_bound: Some(0.1),
                upper_bound: Some(10.0),
            },
            rule_const("Bottom", 10.0),
        ],
        ..SolverConfig::default()
    };

    let primary = fit_nonlinear(model_4pl.as_ref(), &datasets, &HookContext::default(), &cfg).unwrap();
    assert_eq!(primary.k_free, 5);

    let comp_spec = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: String::from("log_agonist_vs_response_3pl") };

    let (alt_direct, outcome_direct) = compare_models(&primary, model_3pl.as_ref(), &datasets, &HookContext::default(), &cfg, &comp_spec).unwrap();
    assert_eq!(alt_direct.equation_id, "log_agonist_vs_response_3pl");
    assert_eq!(alt_direct.k_free, 4);
    assert!(
        alt_direct.fixed_constants.iter().any(|(name, val, _)| name == "Bottom" && (*val - 10.0).abs() < 1e-12),
        "shared parameter rule `Bottom` must still apply to alt_model: {:?}",
        alt_direct.fixed_constants
    );
    assert!(matches!(outcome_direct, ModelComparisonOutcome::ExtraSumOfSquaresF(_)));

    let checked = compare_models_checked(&primary, model_3pl.as_ref(), &datasets, &HookContext::default(), &cfg, &comp_spec).unwrap();
    assert_eq!(checked.inference_availability, InferenceAvailability::Available);
    assert_eq!(checked.alternative_fit.equation_id, "log_agonist_vs_response_3pl");
    assert_eq!(checked.alternative_fit.k_free, 4);
    assert!(matches!(checked.outcome, ModelComparisonOutcome::ExtraSumOfSquaresF(_)));

    // RobustCauchy early-return branch in compare_models_checked must also filter alt_model rules.
    let robust_cfg = SolverConfig { method: FitMethod::RobustCauchy, ..cfg.clone() };
    let primary_robust = fit_nonlinear(model_4pl.as_ref(), &datasets, &HookContext::default(), &robust_cfg).unwrap();
    let checked_robust =
        compare_models_checked(&primary_robust, model_3pl.as_ref(), &datasets, &HookContext::default(), &robust_cfg, &comp_spec).unwrap();
    assert_eq!(checked_robust.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
    assert_eq!(checked_robust.alternative_fit.equation_id, "log_agonist_vs_response_3pl");
    assert_eq!(checked_robust.alternative_fit.k_free, 4);

    // Primary / standalone fits still strictly reject unknown parameter names (#114).
    let err_standalone_3pl = fit_nonlinear(model_3pl.as_ref(), &datasets, &HookContext::default(), &cfg).unwrap_err();
    assert!(matches!(err_standalone_3pl, FitError::InvalidParameter { name: "parameter_rules", .. }), "got {err_standalone_3pl:?}");

    let cfg_unknown = SolverConfig { parameter_rules: vec![rule_const("NotAParam", 1.0)], ..SolverConfig::default() };
    let err_primary_4pl = fit_nonlinear(model_4pl.as_ref(), &datasets, &HookContext::default(), &cfg_unknown).unwrap_err();
    assert!(matches!(err_primary_4pl, FitError::InvalidParameter { name: "parameter_rules", .. }), "got {err_primary_4pl:?}");
}

// ============================================================================
// #115: Poisson fits converge from infeasible starts; ImpossibleWeights only when truly impossible
// ============================================================================

#[test]
fn audit_115_poisson_infeasible_start_recovers_feasible_optimum() {
    let model = CompiledDslModel::compile("line_pois", "Y = A + B*X").unwrap();
    let xs: Vec<f64> = (-10..=-1).map(f64::from).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| 50.0 + 3.0 * x).collect();
    let ds = series_from_xy(0, "Poisson", &xs, &ys);

    // Default start (A=1, B=1 -> mu <= 0 at all negative X), explicit (1,1), and (0,1):
    for rules in [vec![], vec![rule_local_init("A", 1.0), rule_local_init("B", 1.0)], vec![rule_local_init("A", 0.0), rule_local_init("B", 1.0)]] {
        let cfg = SolverConfig { method: FitMethod::PoissonDeviance, parameter_rules: rules, ..SolverConfig::default() };
        let res = fit_nonlinear(&model, std::slice::from_ref(&ds), &HookContext::default(), &cfg).unwrap();
        assert_eq!(res.status, FitStatus::Converged);
        assert_approx_eq(find_param(&res, "A").estimate, 50.0, 1e-8, 1e-8, "A recovered");
        assert_approx_eq(find_param(&res, "B").estimate, 3.0, 1e-8, 1e-8, "B recovered");
        assert!(res.weighted_ss < 1e-10);
    }

    // Disabling rescue with an infeasible starting guess must report BadInitialValues, not ImpossibleWeights.
    let cfg_no_rescue = SolverConfig {
        method: FitMethod::PoissonDeviance,
        use_simplex_warm_start: false,
        parameter_rules: vec![rule_local_init("A", 0.0), rule_local_init("B", 1.0)],
        ..SolverConfig::default()
    };
    let err_start = fit_nonlinear(&model, std::slice::from_ref(&ds), &HookContext::default(), &cfg_no_rescue).unwrap_err();
    assert!(matches!(err_start, FitError::BadInitialValues { .. }), "expected BadInitialValues, got {err_start:?}");

    // Truly impossible model where no parameter value yields positive means still returns ImpossibleWeights.
    let cfg_impossible = SolverConfig {
        method: FitMethod::PoissonDeviance,
        parameter_rules: vec![rule_const("A", -5.0), rule_const("B", 0.0)],
        ..SolverConfig::default()
    };
    let err_imp = fit_nonlinear(&model, &[ds], &HookContext::default(), &cfg_impossible).unwrap_err();
    assert!(matches!(err_imp, FitError::ImpossibleWeights { .. }), "expected ImpossibleWeights, got {err_imp:?}");
}

// ============================================================================
// #116: Rank-deficient Jacobian flags only unidentifiable/weak slots, not A and B in A*X + B + C*0
// ============================================================================

#[test]
fn audit_116_rank_deficient_jacobian_preserves_identifiable_subspace_vif() {
    let model = CompiledDslModel::compile("dead_param", "Y = A*X + B + C*0\n# Inits: A=1.0, B=0.5, C=1.0").unwrap();
    let xs: Vec<f64> = (0..12).map(|i| 0.1 + 0.5 * (i as f64)).collect();
    let noise = [0.02, -0.015, 0.01, -0.025, 0.015, -0.01, 0.02, -0.02, 0.005, -0.015, 0.025, -0.01];
    let ys: Vec<f64> = xs.iter().zip(noise).map(|(&x, e)| 1.0 + 2.0 * x + e).collect();
    let ds = series_from_xy(0, "DeadC", &xs, &ys);

    let cfg = SolverConfig { identify_unstable_parameters: true, ..SolverConfig::default() };
    let res = fit_nonlinear(&model, std::slice::from_ref(&ds), &HookContext::default(), &cfg).unwrap();

    let sum_x: f64 = xs.iter().sum();
    let sum_x2: f64 = xs.iter().map(|x| x * x).sum();
    let expected_dep = (sum_x * sum_x) / ((xs.len() as f64) * sum_x2);

    let pa = find_param(&res, "A");
    let pb = find_param(&res, "B");
    let pc = find_param(&res, "C");
    assert_approx_eq(pa.dependency, expected_dep, 1e-8, 1e-8, "A uncentred dependency matches identifiable subspace");
    assert_approx_eq(pb.dependency, expected_dep, 1e-8, 1e-8, "B uncentred dependency matches identifiable subspace");
    assert!(pc.dependency > 0.9999, "zero-sensitivity slot C must stay flagged, got {}", pc.dependency);
    assert!(!pa.notes.contains(&ParameterNote::Ambiguous) && !pa.notes.contains(&ParameterNote::Unstable));
    assert!(!pb.notes.contains(&ParameterNote::Ambiguous) && !pb.notes.contains(&ParameterNote::Unstable));
    assert!(pc.notes.contains(&ParameterNote::Unstable));
    let FitIdentifiability::Ambiguous { reasons } = &res.identifiability else {
        panic!("expected Ambiguous identifiability, got {:?}", res.identifiability);
    };
    assert!(reasons.iter().any(|r| r.contains('C')) && !reasons.iter().any(|r| r.contains("A ") || r.contains("B ")), "reasons={reasons:?}");

    let cfg_no_id = SolverConfig { identify_unstable_parameters: false, ..SolverConfig::default() };
    let res_no_id = fit_nonlinear(&model, &[ds], &HookContext::default(), &cfg_no_id).unwrap();
    assert!(!find_param(&res_no_id, "A").notes.contains(&ParameterNote::Ambiguous));
    assert!(!find_param(&res_no_id, "B").notes.contains(&ParameterNote::Ambiguous));
    assert!(find_param(&res_no_id, "C").notes.contains(&ParameterNote::Ambiguous));
}

// ============================================================================
// #121 (Issue #11): Nearly flat 3PL data sets A & B must report Open upper CIs for LogEC50 and Top
// ============================================================================

#[test]
fn audit_121_issue_11_nearly_flat_3pl_open_upper_logec50_and_top() {
    let xs_grid = [-13.0, -12.0, -11.0, -10.0, -9.5, -9.0, -8.5, -8.0, -7.5, -7.0, -6.0, -5.0];
    let reps_a = [
        [0.158073, 0.142182, 0.125239],
        [0.142630, 0.128691, 0.127492],
        [0.148439, 0.143951, 0.124994],
        [0.153396, 0.130963, 0.131028],
        [0.166767, 0.123700, 0.147693],
        [0.162657, 0.130172, 0.137912],
        [0.159241, 0.140491, 0.132920],
        [0.155937, 0.138415, 0.148216],
        [0.168419, 0.148875, 0.143729],
        [0.154269, 0.149198, 0.138910],
        [0.163195, 0.149607, 0.134643],
        [0.162081, 0.162229, 0.153057],
    ];
    let reps_b = [
        [0.153568, 0.158465, 0.131933],
        [0.130976, 0.132838, 0.137386],
        [0.156624, 0.136648, 0.136798],
        [0.145428, 0.137140, 0.145892],
        [0.152816, 0.132685, 0.132433],
        [0.166189, 0.146827, 0.144328],
        [0.157080, 0.141104, 0.139714],
        [0.153376, 0.149128, 0.141243],
        [0.152285, 0.136487, 0.140378],
        [0.161958, 0.150063, 0.140534],
        [0.167567, 0.142290, 0.146858],
        [0.158073, 0.141105, 0.152835],
    ];

    let to_series = |idx: usize, label: &str, reps: &[[f64; 3]; 12]| -> DatasetSeries {
        let mut pts = Vec::with_capacity(36);
        for (row, (&x, trip)) in xs_grid.iter().zip(reps.iter()).enumerate() {
            for (sub, &y) in trip.iter().enumerate() {
                pts.push(DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, source_subcolumn: sub, data_set_index: idx, row });
            }
        }
        DatasetSeries {
            data_set_index: idx,
            label: label.to_owned(),
            points: pts,
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: false,
            column_title_value: None,
        }
    };

    let ds_a = to_series(0, "DataSetA", &reps_a);
    let ds_b = to_series(1, "DataSetB", &reps_b);
    let model = get_builtin_model("log_agonist_vs_response_3pl").unwrap();
    let cfg = SolverConfig {
        confidence: ConfidenceLevel::DEFAULT_95,
        compute_profile_ci: true,
        identify_unstable_parameters: true,
        ..SolverConfig::default()
    };

    let fit_a = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds_a), &HookContext::default(), &cfg).unwrap();
    assert_eq!(fit_a.df, 33);
    assert_approx_eq(fit_a.rss, 0.004_784_543, 1e-6, 1e-6, "Data set A RSS");
    let a_log = find_param(&fit_a, "LogEC50");
    let a_top = find_param(&fit_a, "Top");
    assert_eq!(a_log.ci_upper, CiBound::Open, "Data set A LogEC50 upper CI must be Open, got {:?}", a_log.ci_upper);
    assert_eq!(a_top.ci_upper, CiBound::Open, "Data set A Top upper CI must be Open, got {:?}", a_top.ci_upper);
    assert!(matches!(fit_a.identifiability, FitIdentifiability::Ambiguous { .. }));

    let fit_b = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds_b), &HookContext::default(), &cfg).unwrap();
    assert_eq!(fit_b.df, 33);
    let b_log = find_param(&fit_b, "LogEC50");
    let b_top = find_param(&fit_b, "Top");
    assert_eq!(b_log.ci_upper, CiBound::Open, "Data set B LogEC50 upper CI must be Open, got {:?}", b_log.ci_upper);
    assert_eq!(b_top.ci_upper, CiBound::Open, "Data set B Top upper CI must be Open, got {:?}", b_top.ci_upper);

    // Joint independent fit across A and B preserves Open upper CIs on both curves.
    let fit_joint = fit_nonlinear(model.as_ref(), &[ds_a.clone(), ds_b.clone()], &HookContext::default(), &cfg).unwrap();
    for p in fit_joint.parameters.iter().filter(|p| p.name == "LogEC50" || p.name == "Top") {
        assert_eq!(p.ci_upper, CiBound::Open, "Joint independent {} upper CI must be Open, got {:?}", p.name, p.ci_upper);
    }

    // End-to-end execute_analysis across A & B reports Open upper bounds in params tab.
    let mut xy_rep =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(3), 2, xs_grid.len())
            .unwrap();
    xy_rep.data_sets[0].title = String::from("DataSetA");
    xy_rep.data_sets[1].title = String::from("DataSetB");
    for (r, (&x, (ta, tb))) in xs_grid.iter().zip(reps_a.iter().zip(reps_b.iter())).enumerate() {
        xy_rep.set_x_cell(r, Cell::number(x)).unwrap();
        for s in 0..3 {
            xy_rep.data_sets[0].cells[r][s] = Cell::number(ta[s]);
            xy_rep.data_sets[1].cells[r][s] = Cell::number(tb[s]);
        }
    }
    let sheet_11 = DataSheet::new(SheetId::new("xy-121"), "Issue 11", TableArchetype::Xy(xy_rep)).unwrap();
    let spec_11 =
        AnalysisSpec::new("res-121", SheetId::new("xy-121"), AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_3pl")))
            .unwrap();
    let rs_11 = execute_analysis(&spec_11, &sheet_11, &HookContext::default()).unwrap();
    let params_tab = rs_11.tabs.iter().find(|t| t.id == "nonlinear_parameters").unwrap();
    let ci_col = params_tab.column_headers.iter().position(|h| h == "Confidence Interval").unwrap();
    assert!(
        params_tab.row_labels.iter().zip(params_tab.rows.iter()).any(|(lbl, row)| {
            lbl.contains("LogEC50") && matches!(row.get(ci_col), Some(ResultCell::ConfidenceInterval { upper: CiBound::Open, .. }))
        }),
        "ResultSheet params tab must report Open upper CI for LogEC50: {:?}",
        params_tab.rows
    );
    assert!(
        params_tab.row_labels.iter().zip(params_tab.rows.iter()).any(|(lbl, row)| {
            lbl.contains("Top") && matches!(row.get(ci_col), Some(ResultCell::ConfidenceInterval { upper: CiBound::Open, .. }))
        }),
        "ResultSheet params tab must report Open upper CI for Top: {:?}",
        params_tab.rows
    );

    let non_mono = compute_profile_likelihood_cis(
        &[0.0],
        10.0,
        20,
        &curvebook_fit::diagnostics::CovarianceDiagnostics {
            standard_errors: vec![1.0],
            symmetric_ci_lower: vec![-1.96],
            symmetric_ci_upper: vec![1.96],
            covariance_matrix: vec![vec![1.0]],
            correlation_matrix: vec![vec![1.0]],
            dependencies: vec![0.0],
            numerical_rank: 1,
            ambiguous: false,
        },
        &[(None, None)],
        ConfidenceLevel::DEFAULT_95,
        |_k, val, _w| {
            // Drops after rising (non-monotone profile)
            let obj = if val.abs() < 1.0 {
                11.0
            } else if val.abs() < 2.5 {
                10.5
            } else {
                15.0
            };
            Some((obj, vec![val]))
        },
    );
    assert_eq!(non_mono, vec![(CiBound::Open, CiBound::Open)]);
    let fake_cov = curvebook_fit::diagnostics::CovarianceDiagnostics {
        standard_errors: vec![1.0],
        symmetric_ci_lower: vec![-1.96],
        symmetric_ci_upper: vec![1.96],
        covariance_matrix: vec![vec![1.0]],
        correlation_matrix: vec![vec![1.0]],
        dependencies: vec![0.0],
        numerical_rank: 1,
        ambiguous: false,
    };
    let disc = compute_profile_likelihood_cis(&[0.0], 10.0, 20, &fake_cov, &[(None, None)], ConfidenceLevel::DEFAULT_95, |_k, val, _w| {
        // Step discontinuity from SS = 11.0 (below SS_crit ~ 12.19) to SS = 15.0 at |val| = 1.5
        let obj = if val.abs() < 1.5 { 11.0 } else { 15.0 };
        Some((obj, vec![val]))
    });
    assert_eq!(disc, vec![(CiBound::Open, CiBound::Open)]);
}
