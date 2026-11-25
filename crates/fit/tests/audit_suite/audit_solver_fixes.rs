#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Regression suite for Wave 2 audit findings in `curvebook-fit`:
//! #51, #52, #53, #80, #87, #93, #94, #100, #114, #115, #116, and #121 (issue #11).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, DataSheet, FitMethod, HookSource, LevelOrderSpec, LinearRegressionSpec,
    ModelComparisonSpec, MultipleVariablesTable, NonlinearFitSpec, ResultCell, SheetId, TableArchetype, VariableColumn, VariableRole, VariableSource,
    WeightingScheme,
};
use curvebook_fit::diagnostics::{anderson_darling_test, compute_profile_likelihood_cis, rout_detect_outliers};
use curvebook_fit::error::{FitError, FitIdentifiability, FitStatus, ParameterNote};
use curvebook_fit::library::get_builtin_model;
use curvebook_fit::result::execute_analysis;
use curvebook_fit::solver::{InferenceAvailability, ModelComparisonOutcome, SolverConfig, compare_models, compare_models_checked, fit_nonlinear};
use curvebook_fit::types::{HookContext, MultivarExtractionSpec, extract_datasets_from_sheet_with_options, parse_leading_number};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::norm_ppf;
use curvebook_testkit::metrics::assert_approx_eq;

use crate::common::{
    cfg_rules, dsl_compile, exec_nl, find_param, find_tab, fit_default, fit_with, param_val, rule_col_title, rule_const, rule_local_init,
    rule_shared, rule_shared_bounds, series_replicates, series_xy, unit_cov_1d, xy_sheet, xy_sheet_replicates, xy_sheet_single,
};

#[test]
fn audit_51_runs_test_per_dataset_no_interleaving() {
    let xs: Vec<f64> = (0..=12).map(f64::from).collect();
    let ys_a: Vec<f64> = xs.iter().map(|&x| (x - 6.0).powi(2)).collect();
    let ys_b: Vec<f64> = xs.iter().map(|&x| 50.0 - (x - 6.0).powi(2)).collect();
    let model = get_builtin_model("straight_line").unwrap();
    let (ds_a, ds_b) = (series_xy(0, "CurveA", &xs, &ys_a), series_xy(1, "CurveB", &xs, &ys_b));

    for ds in [&ds_a, &ds_b] {
        let r = fit_default(model.as_ref(), std::slice::from_ref(ds)).runs_test.unwrap();
        assert!(r.runs == 3 && r.p_too_few_runs < 0.01 && r.deviates_significantly);
    }
    for rules in [vec![], vec![rule_shared("Slope")]] {
        let fit = fit_with(model.as_ref(), &[ds_a.clone(), ds_b.clone()], &cfg_rules(rules));
        assert_eq!(fit.datasets.len(), 2);
        for d in &fit.datasets {
            let r = d.runs_test.as_ref().unwrap();
            assert!(r.runs == 3 && r.p_too_few_runs < 0.01 && r.deviates_significantly);
        }
        let comb = fit.runs_test.as_ref().unwrap();
        assert!(comb.runs == 6 && comb.p_too_few_runs < 0.001 && comb.deviates_significantly);
    }

    let sheet = xy_sheet("xy-51", "XY Runs", &xs, &[("CurveA", &ys_a), ("CurveB", &ys_b)]);
    let rs = exec_nl(&sheet, NonlinearFitSpec::new("straight_line"));
    let gof = find_tab(&rs, "nonlinear_gof");
    assert!(gof.row_labels.contains(&"Runs test (CurveA) P value".to_owned()));
    assert!(gof.row_labels.contains(&"Runs test (CurveB) P value".to_owned()));
    let runs_checks: Vec<_> = rs.checklist.iter().filter(|c| c.assumption.starts_with("Runs test")).collect();
    assert_eq!(runs_checks.len(), 3);
    assert!(runs_checks.iter().all(|c| c.status == ChecklistStatus::Warn));
}

#[test]
fn audit_52_weighted_normality_and_homoscedasticity() {
    let n = 30_usize;
    let z_sorted: Vec<f64> = (0..n).map(|i| norm_ppf(((i as f64) + 0.375) / ((n as f64) + 0.25)).unwrap()).collect();
    let xs: Vec<f64> = (0..n).map(|i| 1.25_f64.powi(i as i32)).collect();
    let ys: Vec<f64> = xs.iter().enumerate().map(|(i, &x)| 2.0 * x * (1.0 + 0.05 * z_sorted[(i * 7 + 3) % n])).collect();

    let model = dsl_compile("Y = Slope * X\n# Inits: Slope=2.0");
    let cfg = SolverConfig {
        weighting: WeightingScheme::RelativeSquare,
        compute_profile_ci: false,
        compute_normality: true,
        compute_homoscedasticity: true,
        ..SolverConfig::default()
    };
    let fit = fit_with(&model, &[series_xy(0, "Heteroscedastic", &xs, &ys)], &cfg);
    let (sw, ad, ks, homo) = (
        fit.normality.as_ref().unwrap(),
        fit.normality_anderson_darling.as_ref().unwrap(),
        fit.normality_lilliefors_ks.as_ref().unwrap(),
        fit.homoscedasticity.as_ref().unwrap(),
    );
    assert!(sw.passed && sw.statistic > 0.96 && sw.p_value > 0.3, "{sw:?}");
    assert!(ad.passed && ad.p_value > 0.3, "{ad:?}");
    assert!(ks.passed && ks.p_value > 0.2, "{ks:?}");
    assert!(homo.homoscedastic && homo.spearman_rho.abs() < 0.35 && homo.p_value > 0.05, "{homo:?}");
}

#[test]
fn audit_53_linear_slope_ci_label_uses_confidence_level() {
    let sheet = xy_sheet_single("xy-53", "Linear CI", "Y", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], &[2.3, 4.1, 6.4, 7.7, 10.6, 11.8, 14.5, 15.6]);
    let run = |deming: bool, pct: f64| {
        let mut spec = AnalysisSpec::new(
            "res",
            sheet.id.clone(),
            AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: deming, ..Default::default() }),
        )
        .unwrap();
        spec.confidence_level = ConfidenceLevel::new(pct).unwrap();
        execute_analysis(&spec, &sheet, &HookContext::default()).unwrap()
    };

    let rs_99 = run(false, 99.0);
    let tab_99 = find_tab(&rs_99, "linear_summary");
    assert!(tab_99.row_labels.contains(&"99% CI Slope".to_owned()) && !tab_99.row_labels.contains(&"95% CI Slope".to_owned()));
    let ci_idx = tab_99.row_labels.iter().position(|l| l == "99% CI Slope").unwrap();
    let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo99), upper: CiBound::Finite(hi99) } = tab_99.rows[ci_idx][0] else {
        panic!("expected finite 99% CI");
    };
    assert_approx_eq(lo99, 1.7272, 5e-4, 5e-4, "99% CI lower");
    assert_approx_eq(hi99, 2.1823, 5e-4, 5e-4, "99% CI upper");

    assert!(find_tab(&run(false, 90.0), "linear_summary").row_labels.contains(&"90% CI Slope".to_owned()));
    let rs_dem = run(true, 99.0);
    let tab_dem = find_tab(&rs_dem, "deming_summary");
    assert!(tab_dem.row_labels.contains(&"99% CI Slope".to_owned()) && tab_dem.row_labels.contains(&"99% CI Intercept".to_owned()));
}

#[test]
fn audit_80_rout_rejects_invalid_q_and_prints_exact_q() {
    let xs: Vec<f64> = (0..12).map(f64::from).collect();
    let mut ys: Vec<f64> = xs.iter().map(|&x| 1.0 + 2.0 * x).collect();
    ys[11] += 25.0;
    let ds = series_xy(0, "Outliers", &xs, &ys);
    let model = get_builtin_model("straight_line").unwrap();
    let res_vec: Vec<f64> = ys.iter().zip(xs.iter()).map(|(&y, &x)| y - (1.0 + 2.0 * x)).collect();

    for bad_q in [0.0, -0.5, 50.001, 80.0, 100.0, f64::NAN, f64::INFINITY] {
        let cfg = SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: bad_q }, ..SolverConfig::default() };
        assert!(matches!(
            fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg),
            Err(FitError::InvalidParameter { .. })
        ));
        assert!(matches!(rout_detect_outliers(&ds.points, &res_vec, 2, bad_q), Err(FitError::InvalidParameter { .. })));
    }

    let sheet = xy_sheet_single("xy-80", "ROUT Exact Q", "Y", &xs, &ys);
    let rs = exec_nl(
        &sheet,
        NonlinearFitSpec { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 0.001 }, ..NonlinearFitSpec::new("straight_line") },
    );
    let metric = rs.checklist.iter().find(|c| c.assumption == "ROUT outlier detection").unwrap().diagnostic_metric.as_deref().unwrap();
    assert!(metric.contains("Q = 0.001%"), "metric=`{metric}`");
}

#[test]
fn audit_93_mv_explicit_bindings_resist_added_calculated_variable() {
    let mut mv = MultipleVariablesTable::new(2, 4).unwrap();
    for (c, title) in [(0, "Dose"), (1, "Resp")] {
        mv.variables[c].title = title.to_owned();
        mv.variables[c].lock_role(VariableRole::Continuous);
    }
    for (r, x) in [1.0, 2.0, 3.0, 4.0].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(2.0 * x + 1.0);
    }
    let sheet_before = DataSheet::new(SheetId::new("mv-93"), "MV", TableArchetype::MultipleVariables(mv.clone())).unwrap();
    mv.variables.push(VariableColumn {
        title: "DoseSq".to_owned(),
        role: VariableRole::Continuous,
        role_inferred: false,
        source: VariableSource::Calculated { formula: "[Dose]*[Dose]".to_owned() },
        level_order: LevelOrderSpec::default(),
        cells: [1.0, 4.0, 9.0, 16.0].into_iter().map(Cell::number).collect(),
    });
    let sheet_after = DataSheet::new(SheetId::new("mv-93"), "MV", TableArchetype::MultipleVariables(mv)).unwrap();

    let nl = NonlinearFitSpec { mv_x_columns: vec![0], mv_y_column: Some(1), ..NonlinearFitSpec::new("straight_line") };
    assert_approx_eq(exec_nl(&sheet_before, nl.clone()).lookup_scalar("Slope", None).unwrap(), 2.0, 1e-10, 1e-10, "NL before slope");
    let res_a = exec_nl(&sheet_after, nl);
    assert_approx_eq(res_a.lookup_scalar("Slope", None).unwrap(), 2.0, 1e-10, 1e-10, "NL after slope");
    assert_approx_eq(res_a.lookup_scalar("YIntercept", None).unwrap(), 1.0, 1e-10, 1e-10, "NL after intercept");

    let px = extract_datasets_from_sheet_with_options(
        &sheet_after,
        &[0, 1],
        &MultivarExtractionSpec { group_columns: &[], x_columns: &[0], y_column: None },
    )
    .unwrap();
    assert_approx_eq(px[0].points[2].y, 7.0, 1e-12, 1e-12, "partial X keeps col 1 Y");
    let py = extract_datasets_from_sheet_with_options(
        &sheet_after,
        &[0],
        &MultivarExtractionSpec { group_columns: &[], x_columns: &[], y_column: Some(1) },
    )
    .unwrap();
    assert_approx_eq(py[0].points[2].x[0], 3.0, 1e-12, 1e-12, "partial Y keeps col 0 X");

    let mut lin_spec = AnalysisSpec::new("lin-93", SheetId::new("mv-93"), AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
    lin_spec.selected_data_sets = vec![0, 1];
    let lin_after = execute_analysis(&lin_spec, &sheet_after, &HookContext::default()).unwrap();
    assert_approx_eq(lin_after.lookup_scalar("Slope", None).unwrap(), 2.0, 1e-10, 1e-10, "Linear slope");
    assert_approx_eq(lin_after.lookup_scalar("YIntercept", None).unwrap(), 1.0, 1e-10, 1e-10, "Linear intercept");
}

#[test]
fn audit_94_strict_column_title_numeric_hook() {
    for (title, expected) in
        [("10", 10.0), ("-2.5", -2.5), ("1e-7", 1e-7), ("10 nM", 10.0), ("[I] = 10 nM", 10.0), ("Conc = 2.5 uM", 2.5), ("5%", 5.0)]
    {
        assert_approx_eq(parse_leading_number(title).unwrap(), expected, 1e-12, 1e-12, title);
    }
    for bad in ["Inhibitor-10 nM", "IL-6 10 ng/mL", "CO2 5%", "Hot1.5", "10-20 nM", "10 20", "", "NaN", "inf"] {
        assert_eq!(parse_leading_number(bad), None, "bad=`{bad}`");
    }

    let sheet = xy_sheet_single("xy-94", "AmbiguousTitle", "Inhibitor-10 nM", &[0.0, 1.0, 2.0, 3.0], &[10.0, 12.0, 14.0, 16.0]);
    let ctx = HookContext { data_sheets: &[sheet], ..HookContext::default() };
    assert!(matches!(
        ctx.resolve(&HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("xy-94"), data_set_index: 0 }),
        Err(FitError::HookResolutionError { .. })
    ));

    let model = dsl_compile("Y = A + B*X");
    let ds = series_xy(0, "IL-6 10 ng/mL", &[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]);
    assert!(matches!(
        fit_nonlinear(&model, &[ds], &HookContext::default(), &cfg_rules(vec![rule_col_title("A")])),
        Err(FitError::HookResolutionError { .. })
    ));
}

#[test]
fn audit_100_anderson_darling_monotone_for_large_a2_star() {
    let mut sparse = vec![0.0_f64; 1000];
    sparse[999] = 1.0;
    let ad_sparse = anderson_darling_test(&sparse).unwrap();
    assert!(ad_sparse.statistic > 350.0 && ad_sparse.p_value < 0.05 && !ad_sparse.passed);

    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next_u01 = || -> f64 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (((state >> 11) as f64) + 0.5) / ((1_u64 << 53) as f64)
    };
    let mut lognorm = Vec::with_capacity(3000);
    for _ in 0..1500 {
        let r = (-2.0 * next_u01().clamp(1e-12, 1.0 - 1e-12).ln()).sqrt();
        let theta = core::f64::consts::TAU * next_u01();
        lognorm.push((2.0 * r * theta.cos()).exp());
        lognorm.push((2.0 * r * theta.sin()).exp());
    }
    let ad_ln = anderson_darling_test(&lognorm).unwrap();
    assert!(ad_ln.statistic > 500.0 && ad_ln.p_value < 0.05 && !ad_ln.passed);

    let exp_sample = |n: usize| -> Vec<f64> { (0..n).map(|i| -(((i as f64) + 0.5) / (n as f64)).ln()).collect() };
    let (r_small, r_mid) = (anderson_darling_test(&exp_sample(1500)).unwrap(), anderson_darling_test(&exp_sample(3600)).unwrap());
    assert!(r_small.statistic < r_mid.statistic && r_mid.statistic < ad_sparse.statistic);
    assert!(r_small.p_value >= r_mid.p_value && r_mid.p_value >= ad_sparse.p_value);
}

#[test]
fn audit_114_parameter_rules_reject_unknown_and_duplicate() {
    let model = get_builtin_model("straight_line").unwrap();
    let ds = [series_xy(0, "Line", &[1.0, 2.0, 3.0, 4.0], &[3.0, 5.0, 7.0, 9.0])];
    for rules in [vec![rule_const("NotAParam", 1.0)], vec![rule_const("Slope", 2.0), rule_const("slope", 3.0)]] {
        let err = fit_nonlinear(model.as_ref(), &ds, &HookContext::default(), &cfg_rules(rules)).unwrap_err();
        assert!(matches!(err, FitError::InvalidParameter { name: "parameter_rules", .. }), "err={err:?}");
    }
}

#[test]
fn audit_87_compare_models_filters_primary_only_parameter_rules_for_alt_model() {
    let (m4, m3) = (get_builtin_model("log_agonist_vs_response_4pl").unwrap(), get_builtin_model("log_agonist_vs_response_3pl").unwrap());
    let xs = [-9.5_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let mk = |top: f64, ec: f64, sgn: f64| -> Vec<f64> {
        xs.iter().enumerate().map(|(i, &x)| 10.0 + top / (1.0 + 10.0_f64.powf((ec - x) * 1.3)) + if i % 2 == 0 { sgn } else { -sgn }).collect()
    };
    let datasets = [series_xy(0, "WT", &xs, &mk(90.0, -6.5, 0.4)), series_xy(1, "Mutant", &xs, &mk(85.0, -5.8, -0.4))];
    let cfg = cfg_rules(vec![rule_shared_bounds("HillSlope", None, Some(0.1), Some(10.0)), rule_const("Bottom", 10.0)]);

    let primary = fit_with(m4.as_ref(), &datasets, &cfg);
    assert_eq!(primary.k_free, 5);
    let comp_spec = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "log_agonist_vs_response_3pl".to_owned() };
    let hooks = HookContext::default();

    let (alt, out) = compare_models(&primary, m3.as_ref(), &datasets, &hooks, &cfg, &comp_spec).unwrap();
    assert_eq!((alt.equation_id.as_str(), alt.k_free), ("log_agonist_vs_response_3pl", 4));
    assert!(alt.fixed_constants.iter().any(|(n, v, _)| n == "Bottom" && (*v - 10.0).abs() < 1e-12));
    assert!(matches!(out, ModelComparisonOutcome::ExtraSumOfSquaresF(_)));

    let checked = compare_models_checked(&primary, m3.as_ref(), &datasets, &hooks, &cfg, &comp_spec).unwrap();
    assert_eq!(
        (checked.inference_availability, checked.alternative_fit.equation_id.as_str(), checked.alternative_fit.k_free),
        (InferenceAvailability::Available, "log_agonist_vs_response_3pl", 4)
    );
    assert!(matches!(checked.outcome, ModelComparisonOutcome::ExtraSumOfSquaresF(_)));

    let rob_cfg = SolverConfig { method: FitMethod::RobustCauchy, ..cfg.clone() };
    let primary_rob = fit_with(m4.as_ref(), &datasets, &rob_cfg);
    let checked_rob = compare_models_checked(&primary_rob, m3.as_ref(), &datasets, &hooks, &rob_cfg, &comp_spec).unwrap();
    assert_eq!(
        (checked_rob.inference_availability, checked_rob.alternative_fit.equation_id.as_str(), checked_rob.alternative_fit.k_free),
        (InferenceAvailability::NotApplicableInRobustMode, "log_agonist_vs_response_3pl", 4)
    );

    assert!(matches!(fit_nonlinear(m3.as_ref(), &datasets, &hooks, &cfg), Err(FitError::InvalidParameter { name: "parameter_rules", .. })));
    assert!(matches!(
        fit_nonlinear(m4.as_ref(), &datasets, &hooks, &cfg_rules(vec![rule_const("NotAParam", 1.0)])),
        Err(FitError::InvalidParameter { name: "parameter_rules", .. })
    ));
}

#[test]
fn audit_115_poisson_infeasible_start_recovers_feasible_optimum() {
    let model = dsl_compile("Y = A + B*X");
    let xs: Vec<f64> = (-10..=-1).map(f64::from).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| 50.0 + 3.0 * x).collect();
    let ds = [series_xy(0, "Poisson", &xs, &ys)];

    for rules in [vec![], vec![rule_local_init("A", 1.0), rule_local_init("B", 1.0)], vec![rule_local_init("A", 0.0), rule_local_init("B", 1.0)]] {
        let cfg = SolverConfig { method: FitMethod::PoissonDeviance, parameter_rules: rules, ..SolverConfig::default() };
        let res = fit_with(&model, &ds, &cfg);
        assert_eq!(res.status, FitStatus::Converged);
        assert_approx_eq(param_val(&res, "A"), 50.0, 1e-8, 1e-8, "A");
        assert_approx_eq(param_val(&res, "B"), 3.0, 1e-8, 1e-8, "B");
        assert!(res.weighted_ss < 1e-10);
    }

    let cfg_no_rescue = SolverConfig {
        method: FitMethod::PoissonDeviance,
        use_simplex_warm_start: false,
        parameter_rules: vec![rule_local_init("A", 0.0), rule_local_init("B", 1.0)],
        ..SolverConfig::default()
    };
    assert!(matches!(fit_nonlinear(&model, &ds, &HookContext::default(), &cfg_no_rescue), Err(FitError::BadInitialValues { .. })));

    let cfg_imp = SolverConfig {
        method: FitMethod::PoissonDeviance,
        parameter_rules: vec![rule_const("A", -5.0), rule_const("B", 0.0)],
        ..SolverConfig::default()
    };
    assert!(matches!(fit_nonlinear(&model, &ds, &HookContext::default(), &cfg_imp), Err(FitError::ImpossibleWeights { .. })));
}

#[test]
fn audit_116_rank_deficient_jacobian_preserves_identifiable_subspace_vif() {
    let model = dsl_compile("Y = A*X + B + C*0\n# Inits: A=1.0, B=0.5, C=1.0");
    let xs: Vec<f64> = (0..12).map(|i| 0.1 + 0.5 * (i as f64)).collect();
    let noise = [0.02, -0.015, 0.01, -0.025, 0.015, -0.01, 0.02, -0.02, 0.005, -0.015, 0.025, -0.01];
    let ys: Vec<f64> = xs.iter().zip(noise).map(|(&x, e)| 1.0 + 2.0 * x + e).collect();
    let ds = [series_xy(0, "DeadC", &xs, &ys)];

    let res = fit_with(&model, &ds, &SolverConfig { identify_unstable_parameters: true, ..SolverConfig::default() });
    let sum_x: f64 = xs.iter().sum();
    let expected_dep = (sum_x * sum_x) / ((xs.len() as f64) * xs.iter().map(|x| x * x).sum::<f64>());

    let (pa, pb, pc) = (find_param(&res, "A"), find_param(&res, "B"), find_param(&res, "C"));
    assert_approx_eq(pa.dependency, expected_dep, 1e-8, 1e-8, "A dep");
    assert_approx_eq(pb.dependency, expected_dep, 1e-8, 1e-8, "B dep");
    assert!(pc.dependency > 0.9999 && pc.notes.contains(&ParameterNote::Unstable));
    assert!(!pa.notes.contains(&ParameterNote::Ambiguous) && !pa.notes.contains(&ParameterNote::Unstable));
    assert!(!pb.notes.contains(&ParameterNote::Ambiguous) && !pb.notes.contains(&ParameterNote::Unstable));
    let FitIdentifiability::Ambiguous { reasons } = &res.identifiability else { panic!("expected Ambiguous") };
    assert!(reasons.iter().any(|r| r.contains('C')) && !reasons.iter().any(|r| r.contains("A ") || r.contains("B ")));

    let res_no_id = fit_with(&model, &ds, &SolverConfig { identify_unstable_parameters: false, ..SolverConfig::default() });
    assert!(!find_param(&res_no_id, "A").notes.contains(&ParameterNote::Ambiguous));
    assert!(!find_param(&res_no_id, "B").notes.contains(&ParameterNote::Ambiguous));
    assert!(find_param(&res_no_id, "C").notes.contains(&ParameterNote::Ambiguous));
}

#[test]
fn audit_121_issue_11_nearly_flat_3pl_open_upper_logec50_and_top() {
    let xs = [-13.0, -12.0, -11.0, -10.0, -9.5, -9.0, -8.5, -8.0, -7.5, -7.0, -6.0, -5.0];
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

    let (ds_a, ds_b) = (series_replicates(0, "DataSetA", &xs, &reps_a), series_replicates(1, "DataSetB", &xs, &reps_b));
    let model = get_builtin_model("log_agonist_vs_response_3pl").unwrap();
    let cfg = SolverConfig {
        confidence: ConfidenceLevel::DEFAULT_95,
        compute_profile_ci: true,
        identify_unstable_parameters: true,
        ..SolverConfig::default()
    };

    let fit_a = fit_with(model.as_ref(), std::slice::from_ref(&ds_a), &cfg);
    assert_eq!(fit_a.df, 33);
    assert_approx_eq(fit_a.rss, 0.004_784_543, 1e-6, 1e-6, "Data set A RSS");
    assert_eq!((find_param(&fit_a, "LogEC50").ci_upper, find_param(&fit_a, "Top").ci_upper), (CiBound::Open, CiBound::Open));
    assert!(matches!(fit_a.identifiability, FitIdentifiability::Ambiguous { .. }));

    let fit_b = fit_with(model.as_ref(), std::slice::from_ref(&ds_b), &cfg);
    assert_eq!(fit_b.df, 33);
    assert_eq!((find_param(&fit_b, "LogEC50").ci_upper, find_param(&fit_b, "Top").ci_upper), (CiBound::Open, CiBound::Open));

    let fit_joint = fit_with(model.as_ref(), &[ds_a, ds_b], &cfg);
    for p in fit_joint.parameters.iter().filter(|p| p.name == "LogEC50" || p.name == "Top") {
        assert_eq!(p.ci_upper, CiBound::Open, "joint {}", p.name);
    }

    let sheet_11 = xy_sheet_replicates("xy-121", "Issue 11", &xs, &[("DataSetA", &reps_a), ("DataSetB", &reps_b)]);
    let rs_11 = exec_nl(&sheet_11, NonlinearFitSpec::new("log_agonist_vs_response_3pl"));
    let ptab = find_tab(&rs_11, "nonlinear_parameters");
    let ci_col = ptab.column_headers.iter().position(|h| h == "Confidence Interval").unwrap();
    for needle in ["LogEC50", "Top"] {
        assert!(
            ptab.row_labels
                .iter()
                .zip(&ptab.rows)
                .any(|(l, r)| l.contains(needle) && matches!(r.get(ci_col), Some(ResultCell::ConfidenceInterval { upper: CiBound::Open, .. }))),
            "missing Open upper CI for {needle}"
        );
    }

    let fake_cov = unit_cov_1d();
    let non_mono = compute_profile_likelihood_cis(&[0.0], 10.0, 20, &fake_cov, &[(None, None)], ConfidenceLevel::DEFAULT_95, |_k, val, _w| {
        let obj = if val.abs() < 1.0 {
            11.0
        } else if val.abs() < 2.5 {
            10.5
        } else {
            15.0
        };
        Some((obj, vec![val]))
    });
    assert_eq!(non_mono, vec![(CiBound::Open, CiBound::Open)]);
    let disc = compute_profile_likelihood_cis(&[0.0], 10.0, 20, &fake_cov, &[(None, None)], ConfidenceLevel::DEFAULT_95, |_k, val, _w| {
        Some((if val.abs() < 1.5 { 11.0 } else { 15.0 }, vec![val]))
    });
    assert_eq!(disc, vec![(CiBound::Open, CiBound::Open)]);
}
