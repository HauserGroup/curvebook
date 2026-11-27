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
use curvebook_fit::result::{execute_analysis, execute_nonlinear_analysis};
use curvebook_fit::solver::{InferenceAvailability, ModelComparisonOutcome, SolverConfig, compare_models, compare_models_checked, fit_nonlinear};
use curvebook_fit::types::{
    DataPoint, HookContext, MultivarExtractionSpec, UnknownResponse, extract_datasets_from_sheet_with_options, parse_leading_number,
};
use curvebook_fit::{
    EquationCategory, ParametricModel, detect_log_x_scale_mismatch, detect_raw_non_log10_x_scale, is_log_x_equation_id,
    non_log_counterpart_equation_id, suggested_default_dose_response_equation,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::norm_ppf;
use curvebook_testkit::metrics::assert_approx_eq;

use crate::common::{
    cfg_fast, cfg_rules, dsl_compile, exec_nl, find_param, find_tab, fit_default, fit_with, nl_fast, param_val, rule_col_title, rule_const,
    rule_local_bounds, rule_local_init, rule_shared, rule_shared_bounds, series_replicates, series_xy, unit_cov_1d, xy_sheet, xy_sheet_replicates,
    xy_sheet_single,
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
    assert_eq!(gof.column_headers, vec!["CurveA", "CurveB"]);
    let runs_row_idx = gof.row_labels.iter().position(|l| l == "Runs test P value").expect("per-dataset Runs test P value row");
    assert_eq!(gof.rows[runs_row_idx].len(), 2);
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
        display_decimals: None,
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

    // AUD-03-25: PoissonDeviance respects max_iters and converges with finite profile CI on one_phase_decay.
    let xs_p: [f64; 9] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0];
    let ys_p: Vec<f64> = xs_p.iter().map(|&x| (200.0 * (-0.35 * x).exp() + 5.0).round()).collect();
    let fit_p = fit_with(
        get_builtin_model("one_phase_decay").unwrap().as_ref(),
        &[series_xy(0, "Counts", &xs_p, &ys_p)],
        &SolverConfig { method: FitMethod::PoissonDeviance, max_iters: 100, compute_profile_ci: true, ..cfg_fast() },
    );
    assert_eq!(fit_p.status, FitStatus::Converged);
    let pk = find_param(&fit_p, "K");
    assert_approx_eq(pk.estimate, 0.35, 0.03, 0.1, "Poisson decay K");
    assert!(matches!((pk.ci_lower, pk.ci_upper), (CiBound::Finite(_), CiBound::Finite(_))));
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
    let mut nl_11 = NonlinearFitSpec::new("log_agonist_vs_response_3pl");
    nl_11.identifiability_mode = Some(curvebook_doc::IdentifiabilityMode::IdentifyAmbiguousFits);
    let rs_11 = exec_nl(&sheet_11, nl_11);
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

#[test]
fn aud_03_10_11_15_36_37_solver_fd_inverse_interp_nonfirst_and_bounds() {
    // AUD-03-10: scale-aware FD on nanomolar box [1e-12, 1e-8] + BadGradLine NaN/Inf fallback to FD.
    let m_nm = dsl_compile("Y = Vmax * X / (Kd + X)\n# Inits: Vmax=90.0, Kd=2e-9");
    let xs_nm = [1e-10, 3e-10, 1e-9, 3e-9, 1e-8, 3e-8, 1e-7];
    let fit_nm = fit_with(
        &m_nm,
        &[series_xy(0, "nM", &xs_nm, &xs_nm.map(|x| 100.0 * x / (1e-9 + x)))],
        &SolverConfig { parameter_rules: vec![rule_local_bounds("Kd", Some(2e-9), Some(1e-12), Some(1e-8))], compute_profile_ci: true, ..cfg_fast() },
    );
    assert_eq!(fit_nm.status, FitStatus::Converged);
    assert_approx_eq(param_val(&fit_nm, "Vmax"), 100.0, 1e-5, 1e-5, "nM Vmax");
    assert_approx_eq(param_val(&fit_nm, "Kd"), 1e-9, 1e-14, 1e-5, "nM Kd");

    struct BadGradLine;
    impl ParametricModel for BadGradLine {
        fn id(&self) -> &'static str {
            "bad_grad_line"
        }
        fn name(&self) -> &'static str {
            "Bad Grad Line"
        }
        fn category(&self) -> EquationCategory {
            EquationCategory::StandardCurve
        }
        fn param_names(&self) -> &[String] {
            static P: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
            P.get_or_init(|| vec!["m".into(), "b".into()])
        }
        fn eval(&self, x: &[f64], p: &[f64], _: usize) -> Option<f64> {
            Some(p[0] * x[0] + p[1])
        }
        fn grad(&self, _: &[f64], _: &[f64], _: usize, out: &mut [f64]) -> bool {
            out[0] = f64::NAN;
            out[1] = f64::INFINITY;
            true
        }
        fn initial_guess(&self, _: &[DataPoint]) -> Vec<f64> {
            vec![0.1, 0.1]
        }
    }
    let fit_bg = fit_with(&BadGradLine, &[series_xy(0, "L", &[1.0, 2.0, 3.0, 4.0, 5.0], &[5.0, 8.0, 11.0, 14.0, 17.0])], &cfg_fast());
    assert_eq!(fit_bg.status, FitStatus::Converged);
    assert_approx_eq(param_val(&fit_bg, "m"), 3.0, 1e-8, 1e-8, "FD fallback m");
    assert_approx_eq(param_val(&fit_bg, "b"), 2.0, 1e-8, 1e-8, "FD fallback b");

    // AUD-03-11: inverse interpolation rejects poles, handles multi-decade & Gaussian flank CI coverage, and tiny-Y roots.
    let cfg_ix = SolverConfig { interpolate_unknowns: true, ..cfg_fast() };
    let xs_mm = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let mut ds_mm = series_xy(0, "MM", &xs_mm, &xs_mm.map(|x| 100.0 * x / (5.0 + x)));
    ds_mm.unknowns_y.push(UnknownResponse::Single((6, 120.0)));
    assert!(fit_with(get_builtin_model("michaelis_menten").unwrap().as_ref(), &[ds_mm], &cfg_ix).interpolated_x[0].output_estimate.is_nan());

    let x_dec = [1e-4, 1e-3, 1e-2, 0.1, 0.3, 0.5, 1.0, 3.0, 10.0, 100.0, 1000.0];
    let e_dec = [0.2, -0.1, 0.15, -0.2, 0.1, -0.15, 0.2, -0.1, 0.15, -0.2, 0.1];
    let mut ds_dec = series_xy(0, "MultiDec", &x_dec, &x_dec.iter().zip(e_dec).map(|(&x, e)| 100.0 * x / (0.5 + x) + e).collect::<Vec<_>>());
    ds_dec.unknowns_y.push(UnknownResponse::Single((11, 50.0)));
    let ix = &fit_with(get_builtin_model("agonist_vs_response_4pl").unwrap().as_ref(), &[ds_dec], &cfg_ix).interpolated_x[0];
    assert_approx_eq(ix.output_estimate, 0.5, 0.05, 0.1, "multi-decade EC50 root");
    let (CiBound::Finite(lo), CiBound::Finite(hi)) = (ix.ci_lower, ix.ci_upper) else { panic!("expected finite inverse CI") };
    assert!(lo <= ix.output_estimate && ix.output_estimate <= hi);

    let xg = [1.0, 2.0, 3.0, 4.0, 4.5, 5.0, 5.5, 6.0, 7.0, 8.0, 9.0];
    let eg = [0.1, -0.1, 0.2, -0.15, 0.1, -0.2, 0.15, -0.1, 0.2, -0.1, 0.1];
    let mut ds_g =
        series_xy(0, "Gauss", &xg, &xg.iter().zip(eg).map(|(&x, e)| 100.0 * (-0.5 * ((x - 5.0) / 1.5_f64).powi(2)).exp() + e).collect::<Vec<_>>());
    ds_g.unknowns_y.push(UnknownResponse::Single((11, 80.0)));
    let ig = &fit_with(get_builtin_model("gaussian").unwrap().as_ref(), &[ds_g], &cfg_ix).interpolated_x[0];
    assert!(ig.output_estimate.is_finite());
    assert!(matches!(ig.ci_lower, CiBound::Finite(v) if v <= ig.output_estimate) || !matches!(ig.ci_lower, CiBound::Finite(_)));
    assert!(matches!(ig.ci_upper, CiBound::Finite(v) if v >= ig.output_estimate) || !matches!(ig.ci_upper, CiBound::Finite(_)));

    let sl = get_builtin_model("straight_line").unwrap();
    let xs_t = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0];
    let mut ds_t = series_xy(0, "TinyY", &xs_t, &xs_t.map(|x| 1e-14 * x));
    ds_t.unknowns_y.push(UnknownResponse::Single((7, 5e-14)));
    assert_approx_eq(fit_with(sl.as_ref(), &[ds_t], &cfg_ix).interpolated_x[0].output_estimate, 5.0, 1e-6, 1e-6, "tiny-Y inverse root");

    // AUD-03-15: non-first sheet columns work for both custom DSL (<C>=2) and built-in paired global binding ([2,3]).
    let dsl_c = dsl_compile("<A> Y = Top * X / (Kd + X)\n<C> Y = Slope * X + Base");
    let fit_c = fit_with(
        &dsl_c,
        &[series_xy(2, "ColC", &[1.0, 2.0, 3.0, 4.0, 5.0], &[5.0, 8.0, 11.0, 14.0, 17.0])],
        &SolverConfig { parameter_rules: vec![rule_const("Top", 1.0), rule_const("Kd", 1.0)], ..cfg_fast() },
    );
    assert_eq!(fit_c.status, FitStatus::Converged);
    assert_approx_eq(param_val(&fit_c, "Slope"), 3.0, 1e-6, 1e-6, "<C> Slope");
    assert_approx_eq(param_val(&fit_c, "Base"), 2.0, 1e-6, 1e-6, "<C> Base");
    let xs_b = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0];
    let fit_b = fit_with(
        get_builtin_model("one_site_total_and_ns_global").unwrap().as_ref(),
        &[
            series_xy(2, "Total", &xs_b, &xs_b.map(|x| 100.0 * x / (2.5 + x) + 1.5 * x + 3.0)),
            series_xy(3, "NS", &xs_b, &xs_b.map(|x| 1.5 * x + 3.0)),
        ],
        &SolverConfig { parameter_rules: vec![rule_shared("Bmax"), rule_shared("Kd"), rule_shared("NS"), rule_shared("Background")], ..cfg_fast() },
    );
    assert_eq!(fit_b.status, FitStatus::Converged);
    for (p, exp) in [("Bmax", 100.0), ("Kd", 2.5), ("NS", 1.5), ("Background", 3.0)] {
        assert_approx_eq(param_val(&fit_b, p), exp, 1e-4, 1e-4, p);
    }

    // AUD-03-36 & AUD-03-37: Nelder-Mead escapes upper-bound start and GeneralPowerY(NaN) errors on all-1.0 Y.
    let fit_ub = fit_with(
        sl.as_ref(),
        &[series_xy(0, "UB", &[1.0, 2.0, 3.0, 4.0, 5.0], &[3.0, 5.0, 7.0, 9.0, 11.0])],
        &SolverConfig {
            use_simplex_warm_start: true,
            parameter_rules: vec![rule_local_bounds("Slope", Some(20.0), Some(0.05), Some(20.0))],
            ..cfg_fast()
        },
    );
    assert_eq!(fit_ub.status, FitStatus::Converged);
    assert_approx_eq(param_val(&fit_ub, "Slope"), 2.0, 1e-6, 1e-6, "escaped upper bound");
    let err_nan = fit_nonlinear(
        sl.as_ref(),
        &[series_xy(0, "Ones", &[1.0, 2.0, 3.0, 4.0], &[1.0; 4])],
        &HookContext::default(),
        &SolverConfig { weighting: WeightingScheme::GeneralPowerY(f64::NAN), ..cfg_fast() },
    )
    .unwrap_err();
    assert!(matches!(err_nan, FitError::ImpossibleWeights { .. } | FitError::InvalidParameter { .. }));
}

#[test]
fn ext_01_raw_x_scale_detection_and_nonlinear_checklist_warning() {
    for (id, counterpart) in [
        ("log_agonist_vs_response_4pl", Some("agonist_vs_response_4pl")),
        ("log_inhibitor_vs_response_4pl", Some("inhibitor_vs_response_4pl")),
        ("4pl", Some("agonist_vs_response_4pl")),
        ("absolute_ic50_4pl", Some("absolute_ic50_4pl_conc")),
        ("operational_model_depletion_log", Some("operational_model_depletion_conc")),
        ("one_site_fit_ki", None),
        ("one_site_fit_logic50", None),
        ("two_sites_fit_logic50", None),
        ("two_sites_fit_ki", None),
        ("homologous_one_site_competition", None),
        ("one_site_competition_ligand_depletion", None),
        ("allosteric_modulator_competition", None),
    ] {
        assert!(is_log_x_equation_id(id), "{id} must be recognized as log10(X)");
        assert_eq!(non_log_counterpart_equation_id(id), counterpart, "{id}");
    }
    assert!(!is_log_x_equation_id("agonist_vs_response_4pl") && !is_log_x_equation_id("log_gaussian"));

    let raw_molar = [1e-9, 1e-8, 1e-7, 1e-6];
    for xs in [&raw_molar[..], &[0.01, 0.1, 1.0, 10.0][..], &[0.0, 10.0, 30.0, 100.0, 300.0][..]] {
        assert!(detect_raw_non_log10_x_scale(xs).is_some());
        assert_eq!(suggested_default_dose_response_equation(xs), "agonist_vs_response_4pl");
        assert_eq!(
            detect_log_x_scale_mismatch("log_agonist_vs_response_4pl", xs).unwrap().suggested_equation_id.as_deref(),
            Some("agonist_vs_response_4pl")
        );
        assert_eq!(detect_log_x_scale_mismatch("one_site_fit_ki", xs).unwrap().suggested_equation_id, None);
    }
    for log_xs in [&[-9.0, -8.0, -7.0, -6.0, -5.0][..], &[-3.0, -2.0, -1.0, 0.0, 1.0, 2.0][..], &[0.0, 1.0, 2.0, 3.0, 4.0][..]] {
        assert!(detect_raw_non_log10_x_scale(log_xs).is_none());
        assert_eq!(suggested_default_dose_response_equation(log_xs), "log_agonist_vs_response_4pl");
        assert!(detect_log_x_scale_mismatch("log_agonist_vs_response_4pl", log_xs).is_none());
    }

    let sheet_molar = xy_sheet_single("xy-molar", "Molar", "Drug", &raw_molar, &raw_molar.map(|c| 10.0 + 90.0 * c / (3e-8 + c)));
    let nl = nl_fast("log_agonist_vs_response_4pl");
    let spec = AnalysisSpec::new("res-molar", "xy-molar", AnalysisKind::NonlinearRegression(nl.clone())).unwrap();
    let res = execute_nonlinear_analysis(&spec, &nl, &sheet_molar, &HookContext::default()).unwrap();
    assert!(res.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.explanation.contains("agonist_vs_response_4pl")));
    let res_ki = exec_nl(&sheet_molar, nl_fast("one_site_fit_logic50"));
    assert!(res_ki.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.explanation.contains("Transform X to log10(X)")));

    let log_out = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, 100.0];
    let mut nl_filt = nl_fast("log_agonist_vs_response_4pl");
    nl_filt.x_range_filter = Some((Some(-10.0), Some(-3.0)));
    let res_filt = exec_nl(
        &xy_sheet_single("xy-log-filt", "LogFilt", "Drug", &log_out, &log_out.map(|x| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.2)))),
        nl_filt,
    );
    assert!(res_filt.checklist.iter().all(|c| !c.assumption.contains("X-axis scale")));

    let log_x = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let res_log = exec_nl(
        &xy_sheet_single("xy-log", "Log", "Drug", &log_x, &log_x.map(|x| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.2)))),
        nl_fast("log_agonist_vs_response_4pl"),
    );
    assert!(res_log.checklist.iter().all(|c| !c.assumption.contains("X-axis scale")));
}

mod fit_report_clarity_and_null_diag {
    use curvebook_doc::{AnalysisKind, AnalysisSpec, ChecklistStatus, CiBound, IdentifiabilityMode, NonlinearFitSpec, ResultCell, ResultStatus};
    use curvebook_fit::{
        FitStatus, HookContext, HorizontalLineClassification, compare_nonlinear_analysis_vs_horizontal_line, execute_analysis, get_builtin_model,
    };

    use crate::common::{cell_num, exec_nl, find_tab, rule_shared, tab_row, xy_sheet, xy_sheet_single};

    #[test]
    fn r19_7_independent_vs_shared_gof_columns_and_clean_footnote() {
        let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
        let xs = [-9.0_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
        let noise_a = [0.4, -0.3, 0.25, -0.2, 0.35, -0.3, 0.2, -0.25, 0.3, -0.2];
        let noise_b = [-0.3, 0.2, -0.25, 0.3, -0.2, 0.25, -0.3, 0.2, -0.2, 0.3];
        let ys_a: Vec<f64> = xs.iter().zip(noise_a).map(|(&x, e)| model.eval(&[x], &[5.0, 95.0, -6.5, 1.0], 0).unwrap() + e).collect();
        let ys_b: Vec<f64> = xs.iter().zip(noise_b).map(|(&x, e)| model.eval(&[x], &[8.0, 82.0, -5.8, 1.1], 1).unwrap() + e).collect();
        let ys_flat = [10.05, 9.96, 10.02, 9.98, 10.04, 9.95, 10.01, 9.97, 10.03, 9.99];

        let sheet = xy_sheet("xy-gof", "Gof Matrix", &xs, &[("CleanA", &ys_a), ("CleanB", &ys_b), ("FlatVehicle", &ys_flat)]);

        let rs_indep = exec_nl(&sheet, NonlinearFitSpec::new("log_agonist_vs_response_4pl"));
        let gof = find_tab(&rs_indep, "nonlinear_gof");
        assert_eq!(gof.column_headers, vec!["CleanA", "CleanB", "FlatVehicle"]);
        let r2_row = tab_row(&rs_indep, "nonlinear_gof", "R squared");
        assert_eq!(r2_row.len(), 3);
        assert!(cell_num(&r2_row[0]) > 0.99, "CleanA R² must stay high, got {:?}", r2_row[0]);
        assert!(cell_num(&r2_row[1]) > 0.99, "CleanB R² must stay high, got {:?}", r2_row[1]);
        for lbl in ["Degrees of Freedom", "Sum of Squares (RSS)", "Sy.x", "AICc", "BIC", "Number of points analyzed", "Runs test P value"] {
            assert_eq!(tab_row(&rs_indep, "nonlinear_gof", lbl).len(), 3, "expected 3 columns for {lbl}");
        }

        let ptab = find_tab(&rs_indep, "nonlinear_parameters");
        assert_eq!(ptab.footnotes.len(), 1);
        let footnote = &ptab.footnotes[0];
        assert!(footnote.contains("R² = "), "footnote must use Unicode R²: {footnote}");
        assert!(!footnote.contains("R2 ="), "footnote must not use ASCII R2: {footnote}");
        assert!(footnote.contains("2 of 3 converged; 1 ambiguous"), "expected convergence summary in footnote: {footnote}");

        let mut nl_shared = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
        nl_shared.parameter_rules = vec![rule_shared("Top"), rule_shared("HillSlope")];
        let rs_shared = exec_nl(&sheet, nl_shared);
        let gof_shared = find_tab(&rs_shared, "nonlinear_gof");
        assert_eq!(gof_shared.column_headers.len(), 1);
    }

    #[test]
    fn r19_7_safe_default_unstable_display_preserves_raw_iterate_without_scalar_leak() {
        let xs = [0.0_f64, 0.5, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0];
        let ys: Vec<f64> = xs.iter().map(|&x| 1.0 + 5.0 * (-0.5 * x).exp()).collect();
        let sheet = xy_sheet_single("xy-col", "Collinear", "Curve", &xs, &ys);

        let dsl = "Y = (A + B) * exp(-K * X) + Base\n# Inits: A=2.0, B=3.0, K=0.5, Base=1.0";
        let mut nl_default = NonlinearFitSpec::new("custom_collinear");
        nl_default.custom_dsl = Some(dsl.to_owned());
        let rs_default = exec_nl(&sheet, nl_default.clone());

        for pname in ["A", "B"] {
            let row = tab_row(&rs_default, "nonlinear_parameters", &format!("{pname} (Unstable)"));
            assert_eq!(row[0], ResultCell::Text("Unstable".to_owned()), "{pname} Best-fit must default to Unstable");
            assert!(rs_default.lookup_scalar(pname, Some(0)).is_none(), "{pname} must never leak into result.scalars (AUD-13-05)");
        }
        let row_k = tab_row(&rs_default, "nonlinear_parameters", "K");
        assert!(matches!(row_k[0], ResultCell::Number { value, prefix: None, .. } if (value - 0.5).abs() < 1e-6));
        assert!((rs_default.lookup_scalar("K", Some(0)).unwrap() - 0.5).abs() < 1e-6);

        let ptab = find_tab(&rs_default, "nonlinear_parameters");
        let a_idx = ptab.row_labels.iter().position(|l| l == "A (Unstable)").unwrap();
        let a_detail = rs_default.cell_detail("nonlinear_parameters", a_idx, 0).expect("raw iterate metadata for unstable A");
        assert!(a_detail.raw_value.is_some_and(f64::is_finite), "raw iterate must be stored on ResultCellDetail: {a_detail:?}");
        assert!(a_detail.detail.contains("raw solver iterate ="), "unexpected detail: {}", a_detail.detail);

        let mut nl_amb = nl_default.clone();
        nl_amb.identifiability_mode = Some(IdentifiabilityMode::IdentifyAmbiguousFits);
        let rs_amb = exec_nl(&sheet, nl_amb);
        let a_amb_row = tab_row(&rs_amb, "nonlinear_parameters", "A");
        assert!(matches!(a_amb_row[0], ResultCell::Number { prefix: Some(ref p), .. } if p == "~"));

        let mut nl_raw = nl_default;
        nl_raw.identifiability_mode = Some(IdentifiabilityMode::ReportAll);
        let rs_raw = exec_nl(&sheet, nl_raw);
        let a_raw_row = tab_row(&rs_raw, "nonlinear_parameters", "A");
        assert!(matches!(a_raw_row[0], ResultCell::Number { prefix: None, .. }));
    }

    #[test]
    fn r19_7_failing_dataset_checklist_and_ext01_preserved() {
        let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
        let xs = [0.0_f64, 3.0, 6.0, 9.0, 12.0, 15.0, 18.0, 21.0];
        let clean_ys: Vec<f64> = xs
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                let e = if i % 2 == 0 { 0.3 } else { -0.3 };
                model.eval(&[x], &[5.0, 95.0, 10.5, 0.5], 0).unwrap() + e
            })
            .collect();
        let flat_a = [10.02, 9.98, 10.04, 9.97, 10.01, 9.99, 10.03, 9.96];
        let flat_b = [20.01, 19.99, 20.03, 19.96, 20.02, 19.98, 20.00, 19.97];

        let sheet = xy_sheet("xy-fail-list", "Panel", &xs, &[("ACTIVE", &clean_ys), ("VEHICLE", &flat_a), ("CLOZAPINE", &flat_b)]);
        let rs = exec_nl(&sheet, NonlinearFitSpec::new("log_agonist_vs_response_4pl"));

        let amb_item = rs
            .checklist
            .iter()
            .find(|c| c.assumption == "Parameter identifiability (Ambiguous)" && c.status == ChecklistStatus::Violated)
            .expect("expected consolidated failing-dataset checklist item");
        assert!(
            amb_item.explanation.contains("Ambiguous / unbounded fits in 2 of 3 datasets (VEHICLE, CLOZAPINE)"),
            "unexpected explanation: {}",
            amb_item.explanation
        );
        assert_eq!(amb_item.diagnostic_metric.as_deref(), Some("2 of 3 datasets (VEHICLE, CLOZAPINE)"));

        assert!(
            rs.checklist.iter().any(|c| c.assumption == "X-axis scale matches log10(X) equation" && c.status == ChecklistStatus::Warn),
            "EXT-01 RawXScaleWarning must remain intact: {:?}",
            rs.checklist
        );
    }

    #[test]
    fn r19_7_horizontal_line_null_model_distinguishes_flat_vs_partial_curve() {
        let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
        let xs = [-9.0_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5];
        let clean_ys: Vec<f64> = xs
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                let noise = if i % 2 == 0 { 0.25 } else { -0.25 };
                model.eval(&[x], &[5.0, 95.0, -7.2, 1.1], 0).unwrap() + noise
            })
            .collect();
        let flat_ys = [10.05, 9.94, 10.08, 9.92, 10.04, 9.95, 10.02, 9.97];
        let partial_ys: Vec<f64> = xs
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                let noise = if i % 2 == 0 { 0.05 } else { -0.05 };
                model.eval(&[x], &[5.0, 200.0, -4.0, 1.0], 2).unwrap() + noise
            })
            .collect();

        let sheet =
            xy_sheet("xy-horiz", "Flat vs Partial", &xs, &[("CleanCurve", &clean_ys), ("FlatVehicle", &flat_ys), ("PartialCurve", &partial_ys)]);
        let spec =
            AnalysisSpec::new("fit-horiz", "xy-horiz", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl")))
                .unwrap();
        let report = compare_nonlinear_analysis_vs_horizontal_line(&spec, &sheet, &HookContext::default()).unwrap();

        assert_eq!(report.per_dataset.len(), 3);
        let flat_diag = report.per_dataset.iter().find(|d| d.label == "FlatVehicle").unwrap();
        assert_eq!(flat_diag.classification, HorizontalLineClassification::NotDistinguishableFromHorizontalLine, "{flat_diag:?}");
        assert!(flat_diag.summary.starts_with("Not distinguishable from horizontal line (P = "), "{}", flat_diag.summary);
        assert!(flat_diag.p_value.is_some_and(|p| p >= 0.05));

        let partial_diag = report.per_dataset.iter().find(|d| d.label == "PartialCurve").unwrap();
        assert_eq!(partial_diag.classification, HorizontalLineClassification::IncompleteOrUnboundedCurve, "{partial_diag:?}");
        assert!(partial_diag.summary.starts_with("Incomplete / unbounded curve (plateau or EC50 outside tested range"), "{}", partial_diag.summary);
        assert!(partial_diag.p_value.is_some_and(|p| p < 0.05));
    }

    #[test]
    fn r20_10_ci_bound_unavailable_for_df0_and_singular_vs_open_profile() {
        let sheet_df0 = xy_sheet_single("xy-df0-ci", "DF0", "Obs", &[1.0, 2.0], &[3.0, 5.0]);
        let mut nl_df0 = NonlinearFitSpec::new("straight_line");
        nl_df0.profile_likelihood_ci = false;
        let rs_df0 = exec_nl(&sheet_df0, nl_df0);
        let ptab_df0 = find_tab(&rs_df0, "nonlinear_parameters");
        let ci_col = ptab_df0.column_headers.iter().position(|h| h == "Confidence Interval").unwrap();
        for row in &ptab_df0.rows {
            assert_eq!(
                row[ci_col],
                ResultCell::ConfidenceInterval { lower: CiBound::Unavailable, upper: CiBound::Unavailable },
                "DF=0 fit must emit CiBound::Unavailable: {row:?}"
            );
        }

        let xs_amb: Vec<f64> = (0..8).map(f64::from).collect();
        let ys_amb: Vec<f64> = xs_amb.iter().map(|&x| 1.0 + 2.0 * x).collect();
        let sheet_sing = xy_sheet_single("xy-sing-ci", "Sing", "Obs", &xs_amb, &ys_amb);
        for (profile_ci, mode) in [
            (false, IdentifiabilityMode::IdentifyAmbiguousFits),
            (true, IdentifiabilityMode::IdentifyAmbiguousFits),
            (true, IdentifiabilityMode::ReportAll),
        ] {
            let mut nl_sing = NonlinearFitSpec::new("dead_sing");
            nl_sing.custom_dsl = Some("Y = A*X + B + C*0\n# Inits: A=2.0, B=1.0, C=1.0".to_owned());
            nl_sing.profile_likelihood_ci = profile_ci;
            nl_sing.identifiability_mode = Some(mode);
            let rs_sing = exec_nl(&sheet_sing, nl_sing);
            let ptab_sing = find_tab(&rs_sing, "nonlinear_parameters");
            let ci_col_s = ptab_sing.column_headers.iter().position(|h| h == "Confidence Interval").unwrap();
            let c_row = tab_row(&rs_sing, "nonlinear_parameters", "C");
            assert_eq!(
                c_row[ci_col_s],
                ResultCell::ConfidenceInterval { lower: CiBound::Unavailable, upper: CiBound::Unavailable },
                "singular dead C with profile_ci={profile_ci}, mode={mode:?} must emit Unavailable CI"
            );
        }

        let sheet_mixed = xy_sheet(
            "xy-mixed-df0",
            "MixedDF0",
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            &[("10", &[3.0, 5.0, f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN]), ("20", &[2.1, 4.0, 6.1, 8.0, 9.9, 12.1, 13.9, 16.0])],
        );
        let mut nl_mixed = NonlinearFitSpec::new("straight_line");
        nl_mixed.summary_parameter = Some("Slope".to_owned());
        let rs_mixed = exec_nl(&sheet_mixed, nl_mixed);
        let ptab_mixed = find_tab(&rs_mixed, "nonlinear_parameters");
        let ci_col_m = ptab_mixed.column_headers.iter().position(|h| h == "Confidence Interval").unwrap();
        let slope_a = tab_row(&rs_mixed, "nonlinear_parameters", "Slope [10]");
        assert!(matches!(slope_a[0], ResultCell::Number { value, .. } if (value - 2.0).abs() < 1e-6));
        assert_eq!(slope_a[ci_col_m], ResultCell::ConfidenceInterval { lower: CiBound::Unavailable, upper: CiBound::Unavailable });
        let slope_b = tab_row(&rs_mixed, "nonlinear_parameters", "Slope [20]");
        assert!(matches!(slope_b[ci_col_m], ResultCell::ConfidenceInterval { lower: CiBound::Finite(_), upper: CiBound::Finite(_) }));

        let sum_mixed = find_tab(&rs_mixed, "nonlinear_parameter_summary");
        let sum_ci_col = sum_mixed.column_headers.iter().position(|h| h == "Confidence Interval").unwrap();
        assert_eq!(sum_mixed.rows[0][sum_ci_col], ResultCell::ConfidenceInterval { lower: CiBound::Unavailable, upper: CiBound::Unavailable });
        assert!(matches!(sum_mixed.rows[1][sum_ci_col], ResultCell::ConfidenceInterval { lower: CiBound::Finite(_), upper: CiBound::Finite(_) }));

        let xs_mm = [1.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 15.0];
        let ys_mm: Vec<f64> = xs_mm.iter().zip([-0.8, 1.2, -1.0, 1.6, -1.2, 1.4, -1.0, 0.6]).map(|(&x, e)| 600.0 * x / (120.0 + x) + e).collect();
        let sheet_mm = xy_sheet_single("xy-mm-open", "MM", "SubSat", &xs_mm, &ys_mm);
        let mut nl_mm = NonlinearFitSpec::new("michaelis_menten");
        nl_mm.profile_likelihood_ci = true;
        nl_mm.identifiability_mode = Some(IdentifiabilityMode::IdentifyAmbiguousFits);
        let rs_mm = exec_nl(&sheet_mm, nl_mm);
        let vmax_row = tab_row(&rs_mm, "nonlinear_parameters", "Vmax");
        let ci_col_mm = find_tab(&rs_mm, "nonlinear_parameters").column_headers.iter().position(|h| h == "Confidence Interval").unwrap();
        assert!(
            matches!(vmax_row[ci_col_mm], ResultCell::ConfidenceInterval { lower: CiBound::Finite(_), upper: CiBound::Open }),
            "expected finite lower and Open upper profile CI for Vmax, got {:?}",
            vmax_row[ci_col_mm]
        );
    }

    #[test]
    fn r19_7_summary_param_tab_also_masks_unstable_and_retains_raw_detail() {
        let xs = [0.0_f64, 0.5, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0];
        let ys_a: Vec<f64> = xs.iter().map(|&x| 1.0 + 5.0 * (-0.5 * x).exp()).collect();
        let ys_b: Vec<f64> = xs.iter().map(|&x| 2.0 + 6.0 * (-0.4 * x).exp()).collect();
        let sheet = xy_sheet("xy-sum", "Summary", &xs, &[("Curve1", &ys_a), ("Curve2", &ys_b)]);

        let mut nl = NonlinearFitSpec::new("custom_sum");
        nl.custom_dsl = Some("Y = (A + B) * exp(-K * X) + Base\n# Inits: A=2.0, B=3.0, K=0.5, Base=1.0".to_owned());
        nl.summary_parameter = Some("A".to_owned());
        let spec = AnalysisSpec::new("fit-sum", "xy-sum", AnalysisKind::NonlinearRegression(nl)).unwrap();
        let rs = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();

        assert_eq!(rs.status.clone(), ResultStatus::UpToDate);
        let sum_tab = find_tab(&rs, "nonlinear_parameter_summary");
        assert_eq!(sum_tab.rows[0][0], ResultCell::Text("Unstable".to_owned()));
        assert!(rs.cell_detail("nonlinear_parameter_summary", 0, 0).is_some_and(|d| d.raw_value.is_some()));
        assert!(rs.lookup_scalar("A", Some(0)).is_none());
        let _ = FitStatus::Converged;
    }
}
