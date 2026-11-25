#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! D1 Edge Oracle Matrix for `crates/fit` (`Wave 2 G1`).
//! Clean-room black-box provenance (2026-11-13): weighted regression, F-distribution, and AICc
//! references generated independently via R 4.5.2 `stats` (`lm`, `optim`, `qf`, `pf`).

use curvebook_doc::{CiBound, ConfidenceLevel, FitMethod, ModelComparisonSpec, WeightingScheme};
use curvebook_fit::diagnostics::{compare_models_aicc, compare_models_f_test};
use curvebook_fit::error::{FitError, FitStatus, ParameterNote};
use curvebook_fit::library::get_builtin_model;
use curvebook_fit::linear::{DemingOptions, SimpleLinearOptions, deming_regression, simple_linear_regression};
use curvebook_fit::solver::{ModelComparisonOutcome, SolverConfig, compare_models};
use curvebook_fit::types::{DataPoint, HookContext};
use curvebook_testkit::metrics::{assert_approx_eq, assert_lre_at_least};

use crate::common::{
    cfg_rules, dsl_compile, find_param, fit_default, fit_with, param_val, rule_const, rule_local_bounds, rule_shared, rule_shared_bounds,
    series_from_pts, series_xy, xy_pts,
};

#[test]
fn edge_01a_flat_plateau_ambiguous_collinear_and_baseline_only() {
    let cfg = SolverConfig { confidence: ConfidenceLevel::DEFAULT_95, compute_profile_ci: true, ..SolverConfig::default() };
    let dsl_ambig = dsl_compile("Y = (A + B) * exp(-K * X) + Base\n# Inits: A=2.0, B=3.0, K=0.5, Base=1.0");
    let xs = [0.0_f64, 0.5, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 1.0 + 5.0 * (-0.5 * x).exp()).collect();
    let res = fit_with(&dsl_ambig, &[series_xy(0, "Ambiguous", &xs, &ys)], &cfg);
    assert_eq!(res.status, FitStatus::Ambiguous);
    assert!(res.covariance.ambiguous && res.covariance.numerical_rank < 4);

    let (pa, pb, pk) = (find_param(&res, "A"), find_param(&res, "B"), find_param(&res, "K"));
    assert!(pa.dependency > 0.9999 && pb.dependency > 0.9999);
    assert!(pa.notes.contains(&ParameterNote::Ambiguous) && pb.notes.contains(&ParameterNote::Ambiguous));
    assert_approx_eq(pa.estimate + pb.estimate, 5.0, 1e-6, 1e-6, "A + B");
    assert_approx_eq(pk.estimate, 0.5, 1e-6, 1e-6, "K");

    let model_4pl = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let res_flat = fit_with(
        model_4pl.as_ref(),
        &[series_xy(0, "BaselineOnly", &[-11.5, -11.0, -10.5, -10.0, -9.5, -9.0], &[10.05, 9.96, 10.02, 9.98, 10.04, 9.95])],
        &cfg,
    );
    let flagged = res_flat.status == FitStatus::Ambiguous
        || res_flat
            .parameters
            .iter()
            .any(|p| p.notes.contains(&ParameterNote::Ambiguous) || matches!(p.ci_lower, CiBound::Open) || matches!(p.ci_upper, CiBound::Open));
    assert!(flagged, "baseline-only 4PL must report ambiguity or open bounds");
}

#[test]
fn edge_01b_open_profile_likelihood_bounds_on_sub_saturating_mm() {
    let mm = get_builtin_model("michaelis_menten").unwrap();
    let xs = [1.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 15.0];
    let ys: Vec<f64> = xs.iter().zip([-0.8, 1.2, -1.0, 1.6, -1.2, 1.4, -1.0, 0.6]).map(|(&x, e)| 600.0 * x / (120.0 + x) + e).collect();
    let res = fit_with(
        mm.as_ref(),
        &[series_xy(0, "SubSaturatingMM", &xs, &ys)],
        &SolverConfig { confidence: ConfidenceLevel::DEFAULT_95, compute_profile_ci: true, ..Default::default() },
    );
    assert!(!res.covariance.ambiguous && res.status == FitStatus::Converged);
    let (vmax, km) = (find_param(&res, "Vmax"), find_param(&res, "Km"));
    assert!(matches!(vmax.ci_upper, CiBound::Open) && matches!(km.ci_upper, CiBound::Open));
    assert!(vmax.notes.contains(&ParameterNote::UnboundedCi) && km.notes.contains(&ParameterNote::UnboundedCi));
}

#[test]
fn edge_02a_active_box_constraints_and_slack_invariance() {
    let decay = get_builtin_model("one_phase_decay").unwrap();
    let xs = [0.0_f64, 0.5, 1.0, 2.0, 4.0, 6.0, 8.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 5.0 + 95.0 * (-0.8 * x).exp()).collect();
    let ds = [series_xy(0, "Decay", &xs, &ys)];

    let res_free = fit_default(decay.as_ref(), &ds);
    assert_approx_eq(param_val(&res_free, "Y0"), 100.0, 1e-8, 1e-8, "free Y0");
    assert_approx_eq(param_val(&res_free, "Plateau"), 5.0, 1e-8, 1e-8, "free Plateau");
    assert_approx_eq(param_val(&res_free, "K"), 0.8, 1e-8, 1e-8, "free K");

    let cfg_slack = cfg_rules(vec![rule_local_bounds("Plateau", None, Some(1.0), Some(10.0)), rule_local_bounds("K", None, Some(0.1), Some(2.0))]);
    let res_slack = fit_with(decay.as_ref(), &ds, &cfg_slack);
    for (pf, ps) in res_free.parameters.iter().zip(&res_slack.parameters) {
        assert_lre_at_least(ps.estimate, pf.estimate, 10.0, &format!("slack {}", pf.name));
        assert!(!ps.notes.contains(&ParameterNote::HitLowerBound) && !ps.notes.contains(&ParameterNote::HitUpperBound));
    }

    let cfg_active = cfg_rules(vec![rule_local_bounds("Plateau", None, Some(15.0), None), rule_local_bounds("K", None, None, Some(0.45))]);
    let res_act = fit_with(decay.as_ref(), &ds, &cfg_active);
    let (pp, pk) = (find_param(&res_act, "Plateau"), find_param(&res_act, "K"));
    assert_approx_eq(pp.estimate, 15.0, 1e-6, 1e-6, "clamped Plateau");
    assert_approx_eq(pk.estimate, 0.45, 1e-6, 1e-6, "clamped K");
    assert!(
        pp.notes.contains(&ParameterNote::HitLowerBound) && pk.notes.contains(&ParameterNote::HitUpperBound) && res_act.rss > res_free.rss + 100.0
    );
}

#[test]
fn edge_02b_shared_global_parameter_and_permutation_invariance() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let log_x = [-10.0, -9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0, -3.0];
    let mk = |top: f64, ec: f64, ns: f64| -> Vec<f64> {
        log_x
            .iter()
            .enumerate()
            .map(|(i, &x)| 12.0 + (top - 12.0) / (1.0 + 10.0_f64.powf((ec - x) * 1.35)) + (if i % 2 == 0 { 0.15 } else { -0.15 }) * ns)
            .collect()
    };
    let (c0, c1, c2) = (
        series_xy(0, "Potent", &log_x, &mk(95.0, -7.5, 1.0)),
        series_xy(1, "Mid", &log_x, &mk(110.0, -6.0, 1.2)),
        series_xy(2, "Weak", &log_x, &mk(82.0, -4.5, 0.8)),
    );

    let cfg_global = cfg_rules(vec![rule_shared("Bottom"), rule_shared_bounds("HillSlope", None, Some(0.1), Some(5.0))]);
    let res_g = fit_with(model.as_ref(), &[c0.clone(), c1.clone(), c2.clone()], &cfg_global);
    assert_eq!((res_g.k_free, res_g.df, res_g.status), (8, 22, FitStatus::Converged));

    let (gb, gh) = (find_param(&res_g, "Bottom"), find_param(&res_g, "HillSlope"));
    assert!(gb.dataset_index.is_none() && gh.dataset_index.is_none());
    assert!(gb.notes.contains(&ParameterNote::SharedGlobal) && gh.notes.contains(&ParameterNote::SharedGlobal));
    assert_approx_eq(gb.estimate, 12.0, 0.2, 0.02, "shared Bottom");
    assert_approx_eq(gh.estimate, 1.35, 0.05, 0.02, "shared HillSlope");
    for d in 0..3 {
        assert_approx_eq(res_g.datasets[d].params[0], gb.estimate, 1e-13, 1e-13, "ds Bottom");
        assert_approx_eq(res_g.datasets[d].params[3], gh.estimate, 1e-13, 1e-13, "ds Hill");
    }

    let (mut c2p, mut c0p, mut c1p) = (c2.clone(), c0.clone(), c1.clone());
    (c2p.data_set_index, c0p.data_set_index, c1p.data_set_index) = (0, 1, 2);
    let res_perm = fit_with(model.as_ref(), &[c2p, c0p, c1p], &cfg_global);
    assert_approx_eq(param_val(&res_perm, "Bottom"), gb.estimate, 1e-9, 1e-9, "permuted Bottom");
    assert_approx_eq(param_val(&res_perm, "HillSlope"), gh.estimate, 1e-9, 1e-9, "permuted Hill");
    assert_approx_eq(res_perm.rss, res_g.rss, 1e-8, 1e-8, "permuted RSS");

    let res_local = fit_default(model.as_ref(), &[c0, c1, c2]);
    assert_eq!(res_local.k_free, 12);
    assert!(res_g.rss + 1e-10 >= res_local.rss);
}

#[test]
fn edge_03a_multi_decade_1e_minus_12_to_1e_minus_2_weighting_1y2_and_1sd2() {
    let x_dec: Vec<f64> = (-12..=-2).map(|p| 10.0_f64.powi(p)).collect();
    let rel_err = [-0.02, 0.03, -0.015, 0.025, -0.01, 0.02, -0.025, 0.015, -0.02, 0.01, -0.015];
    let sd_mult = [0.01, 0.01, 0.02, 0.02, 0.03, 0.05, 0.02, 0.25, 0.02, 0.40, 0.01];
    let y_true: Vec<f64> = x_dec.iter().map(|&x| 5e-13 + 0.8 * x).collect();
    let pts_sd: Vec<DataPoint> = (0..11)
        .map(|i| DataPoint {
            sd: Some(y_true[i].abs() * sd_mult[i]),
            n: Some(3.0),
            row: i,
            ..DataPoint::xy(x_dec[i], y_true[i] * (1.0 + rel_err[i]))
        })
        .collect();
    let ds_dec = [series_from_pts(0, "10DecadeCalib", pts_sd)];
    let line = get_builtin_model("straight_line").unwrap();

    assert!(param_val(&fit_default(line.as_ref(), &ds_dec), "YIntercept").abs() > 1e-7);

    let res_y2 = fit_with(line.as_ref(), &ds_dec, &SolverConfig { weighting: WeightingScheme::RelativeSquare, ..Default::default() });
    assert_lre_at_least(param_val(&res_y2, "YIntercept"), 4.790_189_325_950_841e-13, 8.0, "1/Y^2 b0");
    assert_lre_at_least(param_val(&res_y2, "Slope"), 8.004_557_696_147_121e-1, 8.0, "1/Y^2 slope");
    assert_lre_at_least(res_y2.weighted_ss, 3.934_660_923_134_964e-3, 8.0, "1/Y^2 wSS");

    let res_sd2 = fit_with(line.as_ref(), &ds_dec, &SolverConfig { weighting: WeightingScheme::ObservedSd, ..Default::default() });
    assert_lre_at_least(param_val(&res_sd2, "YIntercept"), 4.791_812_178_521_521e-13, 8.0, "1/SD^2 b0");
    assert_lre_at_least(param_val(&res_sd2, "Slope"), 8.009_717_579_232_87e-1, 8.0, "1/SD^2 slope");
    assert_lre_at_least(res_sd2.weighted_ss, 1.783_481_962_963_079e1, 8.0, "1/SD^2 wSS");

    let t_dec: Vec<f64> = (0..=10).map(|i| i as f64).collect();
    let y_exp: Vec<f64> = t_dec.iter().zip(rel_err).map(|(&t, e)| 1e-2 * (-std::f64::consts::LN_10 * t).exp() * (1.0 + e)).collect();
    let cfg_exp =
        SolverConfig { weighting: WeightingScheme::RelativeSquare, parameter_rules: vec![rule_const("Plateau", 0.0)], ..Default::default() };
    let res_exp = fit_with(get_builtin_model("one_phase_decay").unwrap().as_ref(), &[series_xy(0, "Exp10Decades", &t_dec, &y_exp)], &cfg_exp);
    assert_lre_at_least(param_val(&res_exp, "Y0"), 1.003_182_871_256_514e-2, 8.0, "10-dec exp Y0");
    assert_lre_at_least(param_val(&res_exp, "K"), 2.303_461_763_630_347, 8.0, "10-dec exp K");
    assert_lre_at_least(res_exp.weighted_ss, 4.098_716_924_374_951e-3, 8.0, "10-dec exp wSS");
}

#[test]
fn edge_03b_rout_20_percent_outlier_contamination_and_clean_guard() {
    let decay = get_builtin_model("one_phase_decay").unwrap();
    let r_x: Vec<f64> = (0..20).map(|i| 0.5 * (i as f64)).collect();
    let jitter = [0.3, -0.25, 0.2, -0.35, 0.15, -0.2, 0.3, -0.15, 0.25, -0.3, 0.2, -0.15, 0.3, -0.2, 0.25, -0.25, 0.15, -0.3, 0.2, -0.1];
    let r_y_clean: Vec<f64> = r_x.iter().zip(jitter).map(|(&x, e)| 10.0 + 90.0 * (-0.4 * x).exp() + e).collect();
    let mut r_y_contam = r_y_clean.clone();
    for (idx, spike) in [(3, 48.0), (7, -38.0), (12, 55.0), (16, -32.0)] {
        r_y_contam[idx] += spike;
    }

    let ds_contam = [series_xy(0, "ROUT20pct", &r_x, &r_y_contam)];
    assert!(fit_default(decay.as_ref(), &ds_contam).rsd > 14.0);

    let cfg_rout = SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, ..Default::default() };
    let res = fit_with(decay.as_ref(), &ds_contam, &cfg_rout);
    assert_eq!((res.n_outliers_excluded, res.df), (4, 13));
    assert_eq!(res.rout.as_ref().unwrap().outlier_indices, vec![3, 7, 12, 16]);
    assert_approx_eq(param_val(&res, "Y0"), 100.0, 1.5, 0.02, "ROUT Y0");
    assert_approx_eq(param_val(&res, "Plateau"), 10.0, 1.0, 0.1, "ROUT Plateau");
    assert_approx_eq(param_val(&res, "K"), 0.4, 0.03, 0.08, "ROUT K");
    assert!(res.rsd < 1.0);
    assert_eq!(fit_with(decay.as_ref(), &[series_xy(0, "ROUTClean", &r_x, &r_y_clean)], &cfg_rout).n_outliers_excluded, 0);
}

#[test]
fn edge_04a_extra_ss_f_boundary_aicc_small_sample_flip_and_guardrails() {
    let f_sig = compare_models_f_test(12.60, 8, 7.0, 7, 0.05).unwrap();
    assert_approx_eq(f_sig.f_stat, 5.60, 1e-12, 1e-12, "F just sig");
    assert_approx_eq(f_sig.p_value, 0.049_867_230_568_885, 1e-10, 1e-10, "p just below 0.05");
    assert!(f_sig.preferred_complex_model);

    let f_ns = compare_models_f_test(12.58, 8, 7.0, 7, 0.05).unwrap();
    assert_approx_eq(f_ns.f_stat, 5.58, 1e-12, 1e-12, "F just ns");
    assert_approx_eq(f_ns.p_value, 0.050_178_449_434_496, 1e-10, 1e-10, "p just above 0.05");
    assert!(!f_ns.preferred_complex_model);

    let aicc = compare_models_aicc(10.0, 2, 4.8, 4, 8).unwrap();
    assert_approx_eq(aicc.aicc_a, 13.785_148_410_513_678, 1e-11, 1e-11, "small-n AICc A");
    assert_approx_eq(aicc.aicc_b, 35.913_395_009_872_076, 1e-11, 1e-11, "small-n AICc B");
    assert_approx_eq(aicc.weight_a + aicc.weight_b, 1.0, 1e-12, 1e-12, "weights sum");
    assert!(aicc.preferred_index == 0 && aicc.evidence_ratio > 63_000.0);

    assert!(matches!(compare_models_f_test(10.0, 7, 5.0, 7, 0.05), Err(FitError::InvalidParameter { .. })));
    let clamped = compare_models_f_test(4.0, 8, 5.0, 7, 0.05).unwrap();
    assert_approx_eq(clamped.f_stat, 0.0, 1e-12, 1e-12, "clamped F");
    assert_approx_eq(clamped.p_value, 1.0, 1e-12, 1e-12, "clamped p");
    assert!(!clamped.preferred_complex_model);
    assert!(matches!(compare_models_f_test(-1.0, 8, 5.0, 7, 0.05), Err(FitError::InvalidParameter { .. })));
    assert!(matches!(compare_models_aicc(10.0, 4, 5.0, 2, 6), Err(FitError::InsufficientData { required: 7, actual: 6 })));
}

#[test]
fn edge_04b_end_to_end_compare_models_f_and_aicc_nested_discrimination() {
    let (hooks, cfg) = (HookContext::default(), SolverConfig::default());

    let x_q = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
    let y_q: Vec<f64> =
        x_q.iter().zip([0.04, -0.03, 0.05, -0.04, 0.02, -0.05, 0.03, -0.02, 0.04, -0.04]).map(|(&x, e)| 2.0 + 1.5 * x + 0.8 * x * x + e).collect();
    let ds_q = [series_xy(0, "Quad", &x_q, &y_q)];
    let fit_line = fit_default(get_builtin_model("straight_line").unwrap().as_ref(), &ds_q);
    let poly2 = get_builtin_model("polynomial_2").unwrap();
    let (_, cmp_fq) = compare_models(
        &fit_line,
        poly2.as_ref(),
        &ds_q,
        &hooks,
        &cfg,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "polynomial_2".to_owned() },
    )
    .unwrap();
    assert!(
        matches!(cmp_fq, ModelComparisonOutcome::ExtraSumOfSquaresF(f) if (f.df_num, f.df_den) == (1, 7) && f.preferred_complex_model && f.p_value < 1e-8)
    );
    let (_, cmp_aq) = compare_models(
        &fit_line,
        poly2.as_ref(),
        &ds_q,
        &hooks,
        &cfg,
        &ModelComparisonSpec::Aicc { alternative_equation_id: "polynomial_2".to_owned() },
    )
    .unwrap();
    assert!(matches!(cmp_aq, ModelComparisonOutcome::Aicc(a) if a.preferred_index == 1));

    let x_m = [0.0_f64, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0];
    let y_m: Vec<f64> = x_m
        .iter()
        .zip([0.5, -0.4, 0.6, -0.5, 0.3, -0.4, 0.5, -0.3, 0.4, -0.5, 0.3, -0.4])
        .map(|(&x, e)| 10.0 + 90.0 * (-0.6 * x).exp() + e)
        .collect();
    let ds_m = [series_xy(0, "MonoExp", &x_m, &y_m)];
    let fit_d1 = fit_default(get_builtin_model("one_phase_decay").unwrap().as_ref(), &ds_m);
    let decay2 = get_builtin_model("two_phase_decay").unwrap();
    let (_, cmp_fm) = compare_models(
        &fit_d1,
        decay2.as_ref(),
        &ds_m,
        &hooks,
        &cfg,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "two_phase_decay".to_owned() },
    )
    .unwrap();
    assert!(matches!(cmp_fm, ModelComparisonOutcome::ExtraSumOfSquaresF(f) if (f.df_num, f.df_den) == (2, 7) && !f.preferred_complex_model));
    let (_, cmp_am) = compare_models(
        &fit_d1,
        decay2.as_ref(),
        &ds_m,
        &hooks,
        &cfg,
        &ModelComparisonSpec::Aicc { alternative_equation_id: "two_phase_decay".to_owned() },
    )
    .unwrap();
    assert!(matches!(cmp_am, ModelComparisonOutcome::Aicc(a) if a.preferred_index == 0));
}

#[test]
fn edge_05_tied_predictors_replicates_and_n3_deming_duplicate_points() {
    let dem3 = deming_regression(&xy_pts(&[1.0, 1.0, 3.0], &[2.0, 2.0, 6.0]), &DemingOptions::default()).unwrap();
    assert_approx_eq(dem3.slope, 2.0, 1e-12, 1e-12, "n=3 dup slope");
    assert_approx_eq(dem3.intercept, 0.0, 1e-12, 1e-12, "n=3 dup int");
    assert_approx_eq(dem3.sy_x, 0.0, 1e-12, 1e-12, "n=3 dup sy_x");
    assert!(
        dem3.se_slope.is_nan()
            && dem3.se_intercept.is_nan()
            && dem3.ci_slope.0.is_nan()
            && dem3.ci_slope.1.is_nan()
            && dem3.ci_intercept.0.is_nan()
            && dem3.ci_intercept.1.is_nan()
    );
    assert!(matches!(deming_regression(&xy_pts(&[2.0; 3], &[4.0; 3]), &DemingOptions::default()), Err(FitError::InvalidParameter { .. })));

    let dem4 = deming_regression(&xy_pts(&[1.0, 1.0, 3.0, 3.0], &[2.0, 2.0, 6.0, 6.0]), &DemingOptions::default()).unwrap();
    assert_approx_eq(dem4.slope, 2.0, 1e-12, 1e-12, "n=4 dup slope");
    assert_approx_eq(dem4.se_slope, 0.0, 1e-12, 1e-12, "n=4 dup SE");

    let lin_rep =
        simple_linear_regression(&xy_pts(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0], &[2.0, 4.0, 4.0, 6.0, 6.0, 8.0]), &SimpleLinearOptions::default()).unwrap();
    assert_approx_eq(lin_rep.slope, 2.0, 1e-12, 1e-12, "tied OLS slope");
    assert_approx_eq(lin_rep.intercept, 1.0, 1e-12, 1e-12, "tied OLS int");
    assert_approx_eq(lin_rep.rss, 6.0, 1e-12, 1e-12, "tied OLS RSS");
    assert_approx_eq(lin_rep.sy_x, 1.5_f64.sqrt(), 1e-12, 1e-12, "tied OLS sy_x");
    assert!(lin_rep.lack_of_fit.is_some());
    assert!(matches!(
        simple_linear_regression(&xy_pts(&[2.0; 3], &[1.0, 3.0, 5.0]), &SimpleLinearOptions::default()),
        Err(FitError::InvalidParameter { .. })
    ));

    let decay = get_builtin_model("one_phase_decay").unwrap();
    let rep_xs = [0.0_f64, 0.0, 1.0, 1.0, 2.0, 2.0, 4.0, 4.0];
    let rep_ys: Vec<f64> = rep_xs.iter().enumerate().map(|(i, &x)| 5.0 + 95.0 * (-0.5 * x).exp() + if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
    let nl_rep =
        fit_with(decay.as_ref(), &[series_xy(0, "NlTies", &rep_xs, &rep_ys)], &SolverConfig { compute_lack_of_fit: true, ..Default::default() });
    assert_eq!(nl_rep.status, FitStatus::Converged);
    assert_approx_eq(param_val(&nl_rep, "Y0"), 100.0, 1e-6, 1e-6, "nl Y0");
    assert_approx_eq(param_val(&nl_rep, "Plateau"), 5.0, 1e-6, 1e-6, "nl Plateau");
    assert_approx_eq(param_val(&nl_rep, "K"), 0.5, 1e-6, 1e-6, "nl K");
    assert!(nl_rep.lack_of_fit.is_some());
}
