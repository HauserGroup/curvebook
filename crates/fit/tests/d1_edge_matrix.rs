#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! D1 Edge Oracle Matrix for `crates/fit` (`Wave 2 G1`).
//!
//! Covers flat-plateau non-identifiability (`FitStatus::Ambiguous` and `CiBound::Open`
//! profile-likelihood bounds), active box constraints (`HitLowerBound` / `HitUpperBound`),
//! `SharedGlobal` dataset/permutation invariance, 10-decade (`1e-12..1e-2`) weighted `1/Y^2` &
//! `1/SD^2` fits, `20%` outlier ROUT elimination, and Extra-SS F / AICc nested discrimination edge cases.
//!
//! Clean-room black-box provenance (2026-11-13):
//! - Weighted regression, F-distribution, and AICc references generated independently via
//!   R 4.5.2 (2025-10-31) `stats` (`lm`, `optim`, `qf`, `pf`) on purely synthetic fixtures.
//! - Zero proprietary software references and zero vendor-derived fixtures.

use lumencraft_doc::{CiBound, ConfidenceLevel, FitMethod, ModelComparisonSpec, ParameterRule, ParameterScope, WeightingScheme};
use lumencraft_fit::diagnostics::{compare_models_aicc, compare_models_f_test};
use lumencraft_fit::dsl::CompiledDslModel;
use lumencraft_fit::error::{FitError, FitStatus, ParameterNote};
use lumencraft_fit::library::get_builtin_model;
use lumencraft_fit::solver::{FittedParameterSummary, ModelComparisonOutcome, NonlinearFitResult, SolverConfig, compare_models, fit_nonlinear};
use lumencraft_fit::types::{DataPoint, DatasetSeries, HookContext};
use lumencraft_testkit::metrics::{assert_approx_eq, assert_lre_at_least};

fn series_from_xy(idx: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
    let points = xs
        .iter()
        .zip(ys)
        .enumerate()
        .map(|(row, (&x, &y))| DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, data_set_index: idx, row })
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

#[test]
fn edge_01a_flat_plateau_ambiguous_collinear_and_baseline_only() {
    let hooks = HookContext::default();
    let cfg = SolverConfig { confidence: ConfidenceLevel::DEFAULT_95, compute_profile_ci: true, ..SolverConfig::default() };

    let dsl_ambig =
        CompiledDslModel::compile("redundant_amp_decay", "Y = (A + B) * exp(-K * X) + Base\n# Inits: A=2.0, B=3.0, K=0.5, Base=1.0").unwrap();
    let xs = [0.0_f64, 0.5, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 1.0 + 5.0 * (-0.5 * x).exp()).collect();
    let res_ambig = fit_nonlinear(&dsl_ambig, &[series_from_xy(0, "Ambiguous", &xs, &ys)], &hooks, &cfg).unwrap();
    assert_eq!(res_ambig.status, FitStatus::Ambiguous);
    assert!(res_ambig.covariance.ambiguous && res_ambig.covariance.numerical_rank < 4);

    let p_a = find_param(&res_ambig, "A");
    let p_b = find_param(&res_ambig, "B");
    let p_k = find_param(&res_ambig, "K");
    assert!(p_a.dependency > 0.9999 && p_b.dependency > 0.9999);
    assert!(p_a.notes.contains(&ParameterNote::Ambiguous) && p_b.notes.contains(&ParameterNote::Ambiguous));
    assert_approx_eq(p_a.estimate + p_b.estimate, 5.0, 1e-6, 1e-6, "A + B identifiable sum");
    assert_approx_eq(p_k.estimate, 0.5, 1e-6, 1e-6, "identifiable K");

    let model_4pl = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let flat_x = [-11.5, -11.0, -10.5, -10.0, -9.5, -9.0];
    let flat_y = [10.05, 9.96, 10.02, 9.98, 10.04, 9.95];
    let res_flat = fit_nonlinear(model_4pl.as_ref(), &[series_from_xy(0, "BaselineOnly", &flat_x, &flat_y)], &hooks, &cfg).unwrap();
    let flagged = res_flat.status == FitStatus::Ambiguous
        || res_flat
            .parameters
            .iter()
            .any(|p| p.notes.contains(&ParameterNote::Ambiguous) || matches!(p.ci_lower, CiBound::Open) || matches!(p.ci_upper, CiBound::Open));
    assert!(flagged, "baseline-only 4PL must report ambiguity or open bounds");
}

#[test]
fn edge_01b_open_profile_likelihood_bounds_on_sub_saturating_mm() {
    let hooks = HookContext::default();
    let cfg = SolverConfig { confidence: ConfidenceLevel::DEFAULT_95, compute_profile_ci: true, ..SolverConfig::default() };
    let mm = get_builtin_model("michaelis_menten").unwrap();
    let mm_x = [1.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 15.0];
    let mm_noise = [-0.8, 1.2, -1.0, 1.6, -1.2, 1.4, -1.0, 0.6];
    let mm_y: Vec<f64> = mm_x.iter().zip(mm_noise).map(|(&x, e)| 600.0 * x / (120.0 + x) + e).collect();
    let res_mm = fit_nonlinear(mm.as_ref(), &[series_from_xy(0, "SubSaturatingMM", &mm_x, &mm_y)], &hooks, &cfg).unwrap();
    assert!(!res_mm.covariance.ambiguous);
    assert_eq!(res_mm.status, FitStatus::Converged);

    let vmax = find_param(&res_mm, "Vmax");
    let km = find_param(&res_mm, "Km");
    assert!(matches!(vmax.ci_upper, CiBound::Open) && matches!(km.ci_upper, CiBound::Open));
    assert!(vmax.notes.contains(&ParameterNote::UnboundedCi) && km.notes.contains(&ParameterNote::UnboundedCi));
}

#[test]
fn edge_02a_active_box_constraints_and_slack_invariance() {
    let hooks = HookContext::default();
    let decay = get_builtin_model("one_phase_decay").unwrap();
    let xs = [0.0_f64, 0.5, 1.0, 2.0, 4.0, 6.0, 8.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 5.0 + 95.0 * (-0.8 * x).exp()).collect();
    let ds = vec![series_from_xy(0, "Decay", &xs, &ys)];

    let res_free = fit_nonlinear(decay.as_ref(), &ds, &hooks, &SolverConfig::default()).unwrap();
    assert_approx_eq(find_param(&res_free, "Y0").estimate, 100.0, 1e-8, 1e-8, "free Y0");
    assert_approx_eq(find_param(&res_free, "Plateau").estimate, 5.0, 1e-8, 1e-8, "free Plateau");
    assert_approx_eq(find_param(&res_free, "K").estimate, 0.8, 1e-8, 1e-8, "free K");

    let cfg_slack = SolverConfig {
        parameter_rules: vec![
            ParameterRule {
                name: String::from("Plateau"),
                scope: ParameterScope::Local,
                initial_override: None,
                lower_bound: Some(1.0),
                upper_bound: Some(10.0),
            },
            ParameterRule {
                name: String::from("K"),
                scope: ParameterScope::Local,
                initial_override: None,
                lower_bound: Some(0.1),
                upper_bound: Some(2.0),
            },
        ],
        ..SolverConfig::default()
    };
    let res_slack = fit_nonlinear(decay.as_ref(), &ds, &hooks, &cfg_slack).unwrap();
    for (p_f, p_s) in res_free.parameters.iter().zip(&res_slack.parameters) {
        assert_lre_at_least(p_s.estimate, p_f.estimate, 10.0, &format!("slack invariance {}", p_f.name));
        assert!(!p_s.notes.contains(&ParameterNote::HitLowerBound));
        assert!(!p_s.notes.contains(&ParameterNote::HitUpperBound));
    }

    let cfg_active = SolverConfig {
        parameter_rules: vec![
            ParameterRule {
                name: String::from("Plateau"),
                scope: ParameterScope::Local,
                initial_override: None,
                lower_bound: Some(15.0),
                upper_bound: None,
            },
            ParameterRule {
                name: String::from("K"),
                scope: ParameterScope::Local,
                initial_override: None,
                lower_bound: None,
                upper_bound: Some(0.45),
            },
        ],
        ..SolverConfig::default()
    };
    let res_active = fit_nonlinear(decay.as_ref(), &ds, &hooks, &cfg_active).unwrap();
    let p_plat = find_param(&res_active, "Plateau");
    let p_k = find_param(&res_active, "K");
    assert_approx_eq(p_plat.estimate, 15.0, 1e-6, 1e-6, "clamped Plateau");
    assert_approx_eq(p_k.estimate, 0.45, 1e-6, 1e-6, "clamped K");
    assert!(p_plat.notes.contains(&ParameterNote::HitLowerBound));
    assert!(p_k.notes.contains(&ParameterNote::HitUpperBound));
    assert!(res_active.rss > res_free.rss + 100.0);
}

#[test]
fn edge_02b_shared_global_parameter_and_permutation_invariance() {
    let hooks = HookContext::default();
    let model_4pl = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let log_x = [-10.0, -9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0, -3.0];
    let make_curve = |top: f64, log_ec50: f64, noise_scale: f64| -> Vec<f64> {
        log_x
            .iter()
            .enumerate()
            .map(|(i, &x)| 12.0 + (top - 12.0) / (1.0 + 10.0_f64.powf((log_ec50 - x) * 1.35)) + (if i % 2 == 0 { 0.15 } else { -0.15 }) * noise_scale)
            .collect()
    };

    let c0 = series_from_xy(0, "Potent", &log_x, &make_curve(95.0, -7.5, 1.0));
    let c1 = series_from_xy(1, "Mid", &log_x, &make_curve(110.0, -6.0, 1.2));
    let c2 = series_from_xy(2, "Weak", &log_x, &make_curve(82.0, -4.5, 0.8));

    let shared_rules = vec![
        ParameterRule {
            name: String::from("Bottom"),
            scope: ParameterScope::SharedGlobal,
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("HillSlope"),
            scope: ParameterScope::SharedGlobal,
            initial_override: None,
            lower_bound: Some(0.1),
            upper_bound: Some(5.0),
        },
    ];
    let cfg_global = SolverConfig { parameter_rules: shared_rules, ..SolverConfig::default() };

    let res_g = fit_nonlinear(model_4pl.as_ref(), &[c0.clone(), c1.clone(), c2.clone()], &hooks, &cfg_global).unwrap();
    assert_eq!((res_g.k_free, res_g.df), (8, 22));
    assert_eq!(res_g.status, FitStatus::Converged);

    let g_bottom = find_param(&res_g, "Bottom");
    let g_hill = find_param(&res_g, "HillSlope");
    assert!(g_bottom.dataset_index.is_none() && g_hill.dataset_index.is_none());
    assert!(g_bottom.notes.contains(&ParameterNote::SharedGlobal) && g_hill.notes.contains(&ParameterNote::SharedGlobal));
    assert_approx_eq(g_bottom.estimate, 12.0, 0.2, 0.02, "shared Bottom");
    assert_approx_eq(g_hill.estimate, 1.35, 0.05, 0.02, "shared HillSlope");

    for d in 0..3 {
        assert_approx_eq(res_g.datasets[d].params[0], g_bottom.estimate, 1e-13, 1e-13, "dataset Bottom == global");
        assert_approx_eq(res_g.datasets[d].params[3], g_hill.estimate, 1e-13, 1e-13, "dataset Hill == global");
    }

    let mut c2p = c2.clone();
    c2p.data_set_index = 0;
    let mut c0p = c0.clone();
    c0p.data_set_index = 1;
    let mut c1p = c1.clone();
    c1p.data_set_index = 2;
    let res_perm = fit_nonlinear(model_4pl.as_ref(), &[c2p, c0p, c1p], &hooks, &cfg_global).unwrap();
    assert_approx_eq(find_param(&res_perm, "Bottom").estimate, g_bottom.estimate, 1e-9, 1e-9, "permuted shared Bottom");
    assert_approx_eq(find_param(&res_perm, "HillSlope").estimate, g_hill.estimate, 1e-9, 1e-9, "permuted shared HillSlope");
    assert_approx_eq(res_perm.rss, res_g.rss, 1e-8, 1e-8, "permuted global RSS");

    let res_local = fit_nonlinear(model_4pl.as_ref(), &[c0, c1, c2], &hooks, &SolverConfig::default()).unwrap();
    assert_eq!(res_local.k_free, 12);
    assert!(res_g.rss + 1e-10 >= res_local.rss);
}

#[test]
fn edge_03a_multi_decade_1e_minus_12_to_1e_minus_2_weighting_1y2_and_1sd2() {
    // Provenance: R 4.5.2 (2025-10-31) `lm(..., weights = 1/y^2 / 1/sd^2)` and exact 1D variable projection
    // (`optimize(..., tol = 1e-15)`) across 10-decade signal dynamic range [1e-12 .. 1e-2].
    let hooks = HookContext::default();
    let x_dec: Vec<f64> = (-12..=-2).map(|p| 10.0_f64.powi(p)).collect();
    let rel_err = [-0.02, 0.03, -0.015, 0.025, -0.01, 0.02, -0.025, 0.015, -0.02, 0.01, -0.015];
    let sd_mult = [0.01, 0.01, 0.02, 0.02, 0.03, 0.05, 0.02, 0.25, 0.02, 0.40, 0.01];
    let y_true: Vec<f64> = x_dec.iter().map(|&x| 5e-13 + 0.8 * x).collect();
    let y_obs: Vec<f64> = y_true.iter().zip(rel_err).map(|(&y, e)| y * (1.0 + e)).collect();

    let pts_sd: Vec<DataPoint> = (0..11)
        .map(|i| DataPoint {
            x: vec![x_dec[i]],
            y: y_obs[i],
            sd: Some(y_true[i].abs() * sd_mult[i]),
            n: Some(3.0),
            explicit_weight: None,
            data_set_index: 0,
            row: i,
        })
        .collect();
    let ds_dec = vec![DatasetSeries {
        data_set_index: 0,
        label: String::from("10DecadeCalib"),
        points: pts_sd,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }];
    let line = get_builtin_model("straight_line").unwrap();

    let res_ols = fit_nonlinear(line.as_ref(), &ds_dec, &hooks, &SolverConfig::default()).unwrap();
    assert!(find_param(&res_ols, "YIntercept").estimate.abs() > 1e-7);

    let cfg_inv_y2 = SolverConfig { weighting: WeightingScheme::RelativeSquare, ..SolverConfig::default() };
    let res_y2 = fit_nonlinear(line.as_ref(), &ds_dec, &hooks, &cfg_inv_y2).unwrap();
    assert_lre_at_least(find_param(&res_y2, "YIntercept").estimate, 4.790_189_325_950_841e-13, 8.0, "1/Y^2 b0");
    assert_lre_at_least(find_param(&res_y2, "Slope").estimate, 8.004_557_696_147_121e-1, 8.0, "1/Y^2 slope");
    assert_lre_at_least(res_y2.weighted_ss, 3.934_660_923_134_964e-3, 8.0, "1/Y^2 wSS");

    let cfg_inv_sd2 = SolverConfig { weighting: WeightingScheme::ObservedSd, ..SolverConfig::default() };
    let res_sd2 = fit_nonlinear(line.as_ref(), &ds_dec, &hooks, &cfg_inv_sd2).unwrap();
    assert_lre_at_least(find_param(&res_sd2, "YIntercept").estimate, 4.791_812_178_521_521e-13, 8.0, "1/SD^2 b0");
    assert_lre_at_least(find_param(&res_sd2, "Slope").estimate, 8.009_717_579_232_87e-1, 8.0, "1/SD^2 slope");
    assert_lre_at_least(res_sd2.weighted_ss, 1.783_481_962_963_079e1, 8.0, "1/SD^2 wSS");

    let t_dec: Vec<f64> = (0..=10).map(|i| i as f64).collect();
    let y_exp_obs: Vec<f64> = t_dec.iter().zip(rel_err).map(|(&t, e)| 1e-2 * (-std::f64::consts::LN_10 * t).exp() * (1.0 + e)).collect();
    let decay = get_builtin_model("one_phase_decay").unwrap();
    let cfg_exp_y2 = SolverConfig {
        weighting: WeightingScheme::RelativeSquare,
        parameter_rules: vec![ParameterRule {
            name: String::from("Plateau"),
            scope: ParameterScope::Constant(0.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let res_exp_y2 = fit_nonlinear(decay.as_ref(), &[series_from_xy(0, "Exp10Decades", &t_dec, &y_exp_obs)], &hooks, &cfg_exp_y2).unwrap();
    assert_lre_at_least(find_param(&res_exp_y2, "Y0").estimate, 1.003_182_871_256_514e-2, 8.0, "10-dec exp Y0");
    assert_lre_at_least(find_param(&res_exp_y2, "K").estimate, 2.303_461_763_630_347, 8.0, "10-dec exp K");
    assert_lre_at_least(res_exp_y2.weighted_ss, 4.098_716_924_374_951e-3, 8.0, "10-dec exp wSS");
}

#[test]
fn edge_03b_rout_20_percent_outlier_contamination_and_clean_guard() {
    let hooks = HookContext::default();
    let decay = get_builtin_model("one_phase_decay").unwrap();
    let r_x: Vec<f64> = (0..20).map(|i| 0.5 * (i as f64)).collect();
    let clean_jitter = [0.3, -0.25, 0.2, -0.35, 0.15, -0.2, 0.3, -0.15, 0.25, -0.3, 0.2, -0.15, 0.3, -0.2, 0.25, -0.25, 0.15, -0.3, 0.2, -0.1];
    let r_y_clean: Vec<f64> = r_x.iter().zip(clean_jitter).map(|(&x, e)| 10.0 + 90.0 * (-0.4 * x).exp() + e).collect();
    let mut r_y_contam = r_y_clean.clone();
    r_y_contam[3] += 48.0;
    r_y_contam[7] -= 38.0;
    r_y_contam[12] += 55.0;
    r_y_contam[16] -= 32.0;

    let ds_contam = vec![series_from_xy(0, "ROUT20pct", &r_x, &r_y_contam)];
    let res_bad_ols = fit_nonlinear(decay.as_ref(), &ds_contam, &hooks, &SolverConfig::default()).unwrap();
    assert!(res_bad_ols.rsd > 14.0);

    let cfg_rout = SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, ..SolverConfig::default() };
    let res_rout = fit_nonlinear(decay.as_ref(), &ds_contam, &hooks, &cfg_rout).unwrap();
    assert_eq!(res_rout.n_outliers_excluded, 4);
    assert_eq!(res_rout.df, 13);
    assert_eq!(res_rout.rout.as_ref().unwrap().outlier_indices, vec![3, 7, 12, 16]);
    assert_approx_eq(find_param(&res_rout, "Y0").estimate, 100.0, 1.5, 0.02, "ROUT Y0");
    assert_approx_eq(find_param(&res_rout, "Plateau").estimate, 10.0, 1.0, 0.1, "ROUT Plateau");
    assert_approx_eq(find_param(&res_rout, "K").estimate, 0.4, 0.03, 0.08, "ROUT K");
    assert!(res_rout.rsd < 1.0);

    let res_rout_clean = fit_nonlinear(decay.as_ref(), &[series_from_xy(0, "ROUTClean", &r_x, &r_y_clean)], &hooks, &cfg_rout).unwrap();
    assert_eq!(res_rout_clean.n_outliers_excluded, 0);
}

#[test]
fn edge_04a_extra_ss_f_boundary_aicc_small_sample_flip_and_guardrails() {
    // Provenance: R 4.5.2 (2025-10-31) `qf(0.95, 1, 7)` / `pf(..., lower.tail=FALSE)` and finite-sample AICc.
    let f_just_sig = compare_models_f_test(12.60, 8, 7.0, 7, 0.05).unwrap();
    assert_approx_eq(f_just_sig.f_stat, 5.60, 1e-12, 1e-12, "F just sig");
    assert_approx_eq(f_just_sig.p_value, 0.049_867_230_568_885, 1e-10, 1e-10, "p just below 0.05");
    assert!(f_just_sig.preferred_complex_model);

    let f_just_ns = compare_models_f_test(12.58, 8, 7.0, 7, 0.05).unwrap();
    assert_approx_eq(f_just_ns.f_stat, 5.58, 1e-12, 1e-12, "F just ns");
    assert_approx_eq(f_just_ns.p_value, 0.050_178_449_434_496, 1e-10, 1e-10, "p just above 0.05");
    assert!(!f_just_ns.preferred_complex_model);

    let aicc_edge = compare_models_aicc(10.0, 2, 4.8, 4, 8).unwrap();
    assert_approx_eq(aicc_edge.aicc_a, 13.785_148_410_513_678, 1e-11, 1e-11, "small-n AICc A");
    assert_approx_eq(aicc_edge.aicc_b, 35.913_395_009_872_076, 1e-11, 1e-11, "small-n AICc B");
    assert_approx_eq(aicc_edge.weight_a + aicc_edge.weight_b, 1.0, 1e-12, 1e-12, "Akaike weight sum == 1");
    assert_eq!(aicc_edge.preferred_index, 0);
    assert!(aicc_edge.evidence_ratio > 63_000.0);

    assert!(matches!(compare_models_f_test(10.0, 7, 5.0, 7, 0.05), Err(FitError::InvalidParameter { .. })));
    let clamped = compare_models_f_test(4.0, 8, 5.0, 7, 0.05).unwrap();
    assert_approx_eq(clamped.f_stat, 0.0, 1e-12, 1e-12, "clamped F == 0 when ss_simple < ss_complex");
    assert_approx_eq(clamped.p_value, 1.0, 1e-12, 1e-12, "clamped p == 1");
    assert!(!clamped.preferred_complex_model);
    assert!(matches!(compare_models_f_test(-1.0, 8, 5.0, 7, 0.05), Err(FitError::InvalidParameter { .. })));
    assert!(matches!(compare_models_aicc(10.0, 4, 5.0, 2, 6), Err(FitError::InsufficientData { required: 7, actual: 6 })));
}

#[test]
fn edge_04b_end_to_end_compare_models_f_and_aicc_nested_discrimination() {
    let hooks = HookContext::default();
    let cfg = SolverConfig::default();

    let x_q = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
    let q_noise = [0.04, -0.03, 0.05, -0.04, 0.02, -0.05, 0.03, -0.02, 0.04, -0.04];
    let y_q: Vec<f64> = x_q.iter().zip(q_noise).map(|(&x, e)| 2.0 + 1.5 * x + 0.8 * x * x + e).collect();
    let ds_q = vec![series_from_xy(0, "Quad", &x_q, &y_q)];
    let fit_line = fit_nonlinear(get_builtin_model("straight_line").unwrap().as_ref(), &ds_q, &hooks, &cfg).unwrap();
    let poly2 = get_builtin_model("polynomial_2").unwrap();

    let (_, cmp_f_quad) = compare_models(
        &fit_line,
        poly2.as_ref(),
        &ds_q,
        &hooks,
        &cfg,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: String::from("polynomial_2") },
    )
    .unwrap();
    assert!(
        matches!(cmp_f_quad, ModelComparisonOutcome::ExtraSumOfSquaresF(f) if (f.df_num, f.df_den) == (1, 7) && f.preferred_complex_model && f.p_value < 1e-8)
    );

    let (_, cmp_aicc_quad) = compare_models(
        &fit_line,
        poly2.as_ref(),
        &ds_q,
        &hooks,
        &cfg,
        &ModelComparisonSpec::Aicc { alternative_equation_id: String::from("polynomial_2") },
    )
    .unwrap();
    assert!(matches!(cmp_aicc_quad, ModelComparisonOutcome::Aicc(a) if a.preferred_index == 1));

    let x_m = [0.0_f64, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0];
    let m_noise = [0.5, -0.4, 0.6, -0.5, 0.3, -0.4, 0.5, -0.3, 0.4, -0.5, 0.3, -0.4];
    let y_m: Vec<f64> = x_m.iter().zip(m_noise).map(|(&x, e)| 10.0 + 90.0 * (-0.6 * x).exp() + e).collect();
    let ds_m = vec![series_from_xy(0, "MonoExp", &x_m, &y_m)];
    let fit_d1 = fit_nonlinear(get_builtin_model("one_phase_decay").unwrap().as_ref(), &ds_m, &hooks, &cfg).unwrap();
    let decay2 = get_builtin_model("two_phase_decay").unwrap();

    let (_, cmp_f_mono) = compare_models(
        &fit_d1,
        decay2.as_ref(),
        &ds_m,
        &hooks,
        &cfg,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: String::from("two_phase_decay") },
    )
    .unwrap();
    assert!(matches!(cmp_f_mono, ModelComparisonOutcome::ExtraSumOfSquaresF(f) if (f.df_num, f.df_den) == (2, 7) && !f.preferred_complex_model));

    let (_, cmp_aicc_mono) = compare_models(
        &fit_d1,
        decay2.as_ref(),
        &ds_m,
        &hooks,
        &cfg,
        &ModelComparisonSpec::Aicc { alternative_equation_id: String::from("two_phase_decay") },
    )
    .unwrap();
    assert!(matches!(cmp_aicc_mono, ModelComparisonOutcome::Aicc(a) if a.preferred_index == 0));
}
