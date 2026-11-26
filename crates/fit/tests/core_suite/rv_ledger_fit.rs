#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Regression tests for the 2026-11-16 milestone review ledger (`RV-23`, `RV-24`,
//! `RV-36`..`RV-42`, `RV-68`, `RV-69`, `RV-93`, `RV-96`, `RV-99`, and Section 2 `S3` notes).
//!
//! References: Motulsky & Christopoulos (2004); Motulsky & Brown (2006); Bates & Watts (1988);
//! Venzon & Moolgavkar (1988); McCullagh & Nelder (1989); Dormand & Prince (1980);
//! Hairer, Nørsett & Wanner (1993); Brent (1973).

use curvebook_doc::{ConfidenceLevel, FitMethod, ModelComparisonSpec, NonlinearFitSpec, ResultCell, ResultStatus, WeightingScheme};
use curvebook_fit::diagnostics::compute_covariance_diagnostics;
use curvebook_fit::error::{FitError, FitStatus};
use curvebook_fit::library::{ParametricModel, get_builtin_model};
use curvebook_fit::solver::{SolverConfig, compare_models, fit_nonlinear};
use curvebook_fit::tex_format::format_model_equation_tex;
use curvebook_fit::types::{DataPoint, DatasetSeries, EquationCategory, HookContext};
use curvebook_math::dist::student_t_two_sided_isf;
use curvebook_math::linalg::DenseMatrix;

use crate::common::{
    cell_num, dsl_compile, exec_nl, find_param, find_tab, fit_default, fit_with, param_val, rule_local_bounds, rule_shared_bounds, series_xy,
    tab_row, unwrap_finite_ci, xy_sheet_single,
};

#[test]
fn rv23_and_rv24_tex_text_escapes_and_unary_sign_after_cdot() {
    let tex_id = format_model_equation_tex(r"param_a<b\c>d*e", None, &[], false, 4);
    for needle in [r"\textless{}", r"\textbackslash{}", r"\textgreater{}", r"\*"] {
        assert!(tex_id.contains(needle), "missing {needle} in {tex_id}");
    }
    assert!(!tex_id.contains(r"\<") && !tex_id.contains(r"\>") && !tex_id.contains(r"\\"), "{tex_id}");

    // AUD-03-34: valid `%` now renders through the AST as `\bmod` with parenthesized negative factor,
    // while unparseable fallback DSL still exercises `normalize_tex_signs` unary preservation after `\cdot`.
    let ast_rem = format_model_equation_tex("custom_dsl", Some("Y = a % b + c * -3"), &[], false, 4);
    assert_eq!(ast_rem, r"Y = a \bmod b + c \cdot (-3)");
    let fb = format_model_equation_tex("custom_dsl", Some("Y = a @ b + c * -3"), &[], false, 4);
    assert!(fb.contains(r"\cdot -3") && !fb.contains(r"\cdot - 3"), "{fb}");
}

#[test]
fn rv36_dsl_exponentiation_binds_tighter_than_prefix_unary() {
    for &(src, x, p, exp, tol) in &[
        ("Y = -2^2", 0.0, &[][..], -4.0, 0.0),
        ("Y = (-2)^2", 0.0, &[][..], 4.0, 0.0),
        ("Y = -X^2 + A", 3.0, &[10.0][..], 1.0, 0.0),
        ("Y = X^-2", 2.0, &[][..], 0.25, 0.0),
        ("Y = -X^-2", 2.0, &[][..], -0.25, 0.0),
        ("Y = 2^3^2", 0.0, &[][..], 512.0, 0.0),
        ("Y = 2^-3^2", 0.0, &[][..], 2.0_f64.powf(-9.0), 1e-15),
    ] {
        let got = dsl_compile(src).eval(&[x], p, 0).unwrap();
        assert!((got - exp).abs() <= tol, "{src}: got {got}, exp {exp}");
    }
    assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = -X^2 + A"), &[], false, 4), "Y = -X^{2} + A");

    let quad = dsl_compile("Y = -X^2 + A");
    let xs = [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0];
    let ys: Vec<f64> = xs.iter().map(|&x| -(x * x) + 12.5).collect();
    let fit = fit_default(&quad, &[series_xy(0, "Parabola", &xs, &ys)]);
    assert_eq!(fit.status, FitStatus::Converged);
    assert!((param_val(&fit, "A") - 12.5).abs() < 1e-10);
}

#[test]
fn rv37_heteroscedastic_inverse_y2_rout_and_robust_cauchy_use_weighted_residuals() {
    let xs = [1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0, 35.0, 50.0, 80.0, 120.0, 180.0, 260.0, 380.0, 520.0, 700.0, 850.0, 1000.0];
    let jitter =
        [0.015, -0.018, 0.020, -0.012, 0.016, -0.022, 0.019, -0.014, 0.021, -0.017, 0.013, -0.020, 0.018, -0.016, 0.022, -0.019, 0.021, -0.023];
    let mut ys: Vec<f64> = xs.iter().zip(jitter).map(|(&x, e)| (2.0 * x + 5.0) * (1.0 + e)).collect();
    ys[1] = 25.2;

    let line = get_builtin_model("straight_line").unwrap();
    let ds = series_xy(0, "HetCalib", &xs, &ys);
    let cfg_rout = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        weighting: WeightingScheme::RelativeSquare,
        ..SolverConfig::default()
    };
    let fit_rout = fit_with(line.as_ref(), std::slice::from_ref(&ds), &cfg_rout);
    assert_eq!(fit_rout.status, FitStatus::Converged);
    assert_eq!(fit_rout.rout.as_ref().unwrap().outlier_indices, vec![1]);
    assert!((param_val(&fit_rout, "Slope") - 2.0).abs() < 0.03 && (param_val(&fit_rout, "YIntercept") - 5.0).abs() < 0.15);

    let ds2 = series_xy(1, "HetCalib2", &xs, &ys);
    let fit_multi = fit_with(line.as_ref(), &[ds.clone(), ds2], &cfg_rout);
    assert_eq!(fit_multi.rout.as_ref().unwrap().outlier_indices, vec![1, xs.len() + 1]);

    let cfg_cauchy = SolverConfig { method: FitMethod::RobustCauchy, weighting: WeightingScheme::RelativeSquare, ..SolverConfig::default() };
    let fit_c = fit_with(line.as_ref(), &[ds], &cfg_cauchy);
    assert_eq!(fit_c.status, FitStatus::Converged);
    assert!((param_val(&fit_c, "Slope") - 2.0).abs() < 0.04 && (param_val(&fit_c, "YIntercept") - 5.0).abs() < 0.20);
    let rsdr = fit_c.rsdr.unwrap();
    assert!(rsdr > 0.005 && rsdr < 0.08, "rsdr={rsdr}");
}

#[test]
fn rv38_poisson_deviance_rejects_non_unweighted_schemes_and_explicit_weights() {
    let model = get_builtin_model("exponential_growth").unwrap();
    let ds = series_xy(0, "Counts", &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], &[4.0, 7.0, 12.0, 20.0, 33.0, 55.0]);
    let ok_cfg = SolverConfig { method: FitMethod::PoissonDeviance, weighting: WeightingScheme::None, ..SolverConfig::default() };
    assert!(fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &ok_cfg).is_ok());

    for bad in [
        WeightingScheme::RelativeSquare,
        WeightingScheme::Relative,
        WeightingScheme::GeneralPowerY(2.0),
        WeightingScheme::InverseX,
        WeightingScheme::InverseX2,
        WeightingScheme::ObservedSd,
    ] {
        let cfg = SolverConfig { method: FitMethod::PoissonDeviance, weighting: bad, ..SolverConfig::default() };
        assert!(matches!(
            fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg),
            Err(FitError::InvalidParameter { name: "weighting", .. })
        ));
    }
    let mut w_ds = ds;
    w_ds.points[0].explicit_weight = Some(2.0);
    assert!(matches!(
        fit_nonlinear(model.as_ref(), &[w_ds], &HookContext::default(), &ok_cfg),
        Err(FitError::InvalidParameter { name: "weighting", .. })
    ));
}

fn check_mm_profile_scale_invariance(
    xs: &[f64],
    ys_unit: &[f64],
    scale: f64,
    rel_tol: f64,
) -> (curvebook_fit::NonlinearFitResult, curvebook_fit::NonlinearFitResult) {
    let mm = get_builtin_model("michaelis_menten").unwrap();
    let ys_scaled: Vec<f64> = ys_unit.iter().map(|&y| y * scale).collect();
    let cfg = SolverConfig { confidence: ConfidenceLevel::DEFAULT_95, compute_profile_ci: true, ..SolverConfig::default() };
    let fit_u = fit_with(mm.as_ref(), &[series_xy(0, "Unit", xs, ys_unit)], &cfg);
    let fit_s = fit_with(mm.as_ref(), &[series_xy(0, "Scaled", xs, &ys_scaled)], &cfg);
    assert_eq!((fit_u.status, fit_s.status), (FitStatus::Converged, FitStatus::Converged));

    for (name, factor) in [("Vmax", scale), ("Km", 1.0)] {
        let (pu, ps) = (find_param(&fit_u, name), find_param(&fit_s, name));
        let (u_lo, u_hi) = unwrap_finite_ci((pu.ci_lower, pu.ci_upper), name);
        let (s_lo, s_hi) = unwrap_finite_ci((ps.ci_lower, ps.ci_upper), name);
        assert!(s_lo < ps.estimate && ps.estimate < s_hi, "{name} CI collapsed: [{s_lo}, {s_hi}]");
        assert!(((ps.estimate / factor) - pu.estimate).abs() / pu.estimate < rel_tol, "{name} est");
        assert!(((s_lo / factor) - u_lo).abs() / u_lo < rel_tol, "{name} lo");
        assert!(((s_hi / factor) - u_hi).abs() / u_hi < rel_tol, "{name} hi");
    }
    (fit_u, fit_s)
}

#[test]
fn rv39_profile_likelihood_cis_are_scale_invariant_at_1e_minus_6() {
    let xs = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let noise = [0.4, -0.5, 0.7, -0.8, 1.1, -0.9, 0.6, -0.5];
    let ys_unit: Vec<f64> = xs.iter().zip(noise).map(|(&x, e)| 100.0 * x / (6.0 + x) + e).collect();
    let scale = 1e-6_f64;
    let (fit_u, fit_m) = check_mm_profile_scale_invariance(&xs, &ys_unit, scale, 1e-5);
    assert!((fit_m.weighted_ss / (scale * scale * fit_u.weighted_ss) - 1.0).abs() < 1e-8);
}

#[test]
fn rv40_rejected_inner_trust_region_steps_check_optimality_before_converged() {
    struct DomainWallStallModel(Vec<String>);
    impl ParametricModel for DomainWallStallModel {
        fn id(&self) -> &str {
            "domain_wall_stall"
        }
        fn name(&self) -> &str {
            "Domain wall stall test model"
        }
        fn category(&self) -> EquationCategory {
            EquationCategory::UserDefined
        }
        fn param_names(&self) -> &[String] {
            &self.0
        }
        fn eval(&self, x: &[f64], params: &[f64], _: usize) -> Option<f64> {
            (params[0] <= 1.0).then_some(params[0] * x[0])
        }
        fn grad(&self, x: &[f64], _: &[f64], _: usize, out: &mut [f64]) -> bool {
            out[0] = x[0];
            true
        }
        fn initial_guess(&self, _: &[DataPoint]) -> Vec<f64> {
            vec![1.0]
        }
    }
    let cfg = SolverConfig { use_simplex_warm_start: false, compute_profile_ci: false, ..SolverConfig::default() };
    let fit = fit_with(&DomainWallStallModel(vec!["a".to_owned()]), &[series_xy(0, "Stalled", &[1.0, 2.0, 3.0, 4.0], &[3.0, 6.0, 9.0, 12.0])], &cfg);
    assert_eq!(fit.status, FitStatus::Diverged);
    assert!(!fit.datasets[0].curve_overlay.as_ref().unwrap().converged);
}

#[test]
fn rv41_noisy_1e_minus_16_data_gets_proper_profile_cis_and_exact_fit_stays_zero_width() {
    let xs = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let noise = [0.5, -0.6, 0.8, -0.9, 1.0, -0.7, 0.4];
    let ys_unit: Vec<f64> = xs.iter().zip(noise).map(|(&x, e)| 50.0 * x / (5.0 + x) + e).collect();
    let scale = 1e-16_f64;
    let (_, fit_tiny) = check_mm_profile_scale_invariance(&xs, &ys_unit, scale, 1e-4);
    assert!(fit_tiny.weighted_ss > 0.0 && fit_tiny.weighted_ss < 1e-30);

    let mm = get_builtin_model("michaelis_menten").unwrap();
    let ys_exact: Vec<f64> = xs.iter().map(|&x| scale * 50.0 * x / (5.0 + x)).collect();
    let cfg = SolverConfig { confidence: ConfidenceLevel::DEFAULT_95, compute_profile_ci: true, ..SolverConfig::default() };
    let fit_exact = fit_with(mm.as_ref(), &[series_xy(0, "ExactTiny", &xs, &ys_exact)], &cfg);
    let km = find_param(&fit_exact, "Km");
    let (lo, hi) = unwrap_finite_ci((km.ci_lower, km.ci_upper), "exact Km");
    assert!((hi - lo).abs() <= 1e-12);
}

#[test]
fn rv42_dsl_min_max_propagate_nan_and_modulo_zero_is_checked() {
    for bad in [
        "Y = min(asin(2), 1)",
        "Y = max(acos(2), 1)",
        "Y = min((-1)^0.5, 1)",
        "Y = max(1, (-1)^0.5)",
        "Y = min(NAN, 1)",
        "Y = max(1, NAN)",
        "Y = sign((-1)^0.5)",
        "Y = X % 0",
        "Y = 5 % 0",
    ] {
        assert_eq!(dsl_compile(bad).eval(&[3.0], &[], 0), None, "bad=`{bad}`");
    }
    assert_eq!(dsl_compile("Y = min(X, INF)").eval(&[4.5], &[], 0), Some(4.5));
    assert_eq!(dsl_compile("Y = X % 2").eval(&[7.5], &[], 0), Some(1.5));
}

#[test]
fn rv68_rk45_ode_shrinks_step_when_trial_stages_probe_outside_domain() {
    let ode = dsl_compile("Y0 = 1\ndY/dX = -sqrt(Y)");
    for x in [0.25_f64, 0.75, 1.25, 1.75, 1.95, 1.99, 1.999, 1.9999] {
        assert!((ode.eval(&[x], &[], 0).unwrap() - (1.0 - 0.5 * x).powi(2)).abs() < 5e-7, "X={x}");
    }

    let ode_fit = dsl_compile("Y0 = A\ndY/dX = -k * sqrt(Y)");
    let xs = [0.0, 0.4, 0.8, 1.2, 1.6, 1.9, 1.98];
    let ys: Vec<f64> = xs.iter().map(|&x| (2.0_f64 - x).powi(2)).collect();
    let cfg = SolverConfig {
        parameter_rules: vec![rule_local_bounds("A", Some(3.5), Some(0.1), None), rule_local_bounds("k", Some(1.7), Some(0.1), None)],
        compute_profile_ci: false,
        ..SolverConfig::default()
    };
    let fit = fit_with(&ode_fit, &[series_xy(0, "SqrtOde", &xs, &ys)], &cfg);
    assert_eq!(fit.status, FitStatus::Converged);
    assert!((param_val(&fit, "A") - 4.0).abs() < 1e-4 && (param_val(&fit, "k") - 2.0).abs() < 1e-4);
    assert_eq!(dsl_compile("Y0 = -1\ndY/dX = -sqrt(Y)").eval(&[0.5], &[], 0), None);
}

#[test]
fn rv69_implicit_root_finder_rejects_poles_and_finds_true_roots() {
    for pole in ["1 / (Y - A) = 0", "1e-20 / (Y - A) = 0", "IF(Y > A, 1, -1) = 0"] {
        assert_eq!(dsl_compile(pole).eval(&[1.0], &[2.0], 0), None, "{pole}");
    }
    assert!((dsl_compile("1 / (Y - 0.5) + 1 = 0").eval(&[1.0], &[], 0).unwrap() - (-0.5)).abs() < 1e-11);
}

#[test]
fn rv_section2_s3_notes_three_phase_bounds_and_covariance_t_critical() {
    let m3 = get_builtin_model("three_phase_decay").unwrap();
    assert_eq!(
        m3.default_bounds(),
        vec![(None, None), (None, None), (Some(0.0), Some(100.0)), (Some(0.0), Some(100.0)), (Some(0.0), None), (Some(0.0), None), (Some(0.0), None)]
    );

    let j = DenseMatrix::from_row_major(2, 1, vec![1.0, 1.0]).unwrap();
    let cov0 = compute_covariance_diagnostics(&j, 0.0, 0, &[5.0], ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(cov0.standard_errors[0].is_nan() && cov0.symmetric_ci_lower[0].is_nan() && cov0.symmetric_ci_upper[0].is_nan());

    let conf99 = ConfidenceLevel::new(99.0).unwrap();
    let cov1 = compute_covariance_diagnostics(&j, 2.0, 1, &[5.0], conf99).unwrap();
    let exp_half = student_t_two_sided_isf(conf99.alpha(), 1.0).unwrap() * cov1.standard_errors[0];
    assert!((cov1.symmetric_ci_upper[0] - (5.0 + exp_half)).abs() < 1e-10);
}

#[test]
fn rv93_global_fit_per_dataset_sy_x_uses_effective_dataset_df_not_total_k_free() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs_10: Vec<f64> = (-9..=0).map(|i| -9.0 + (i as f64) * 0.55).collect();
    let ds_10: Vec<DatasetSeries> = [0.2_f64, 1.0, 4.0]
        .into_iter()
        .enumerate()
        .map(|(d, amp)| {
            let ys: Vec<f64> = xs_10
                .iter()
                .enumerate()
                .map(|(r, &x)| {
                    model.eval(&[x], &[10.0 + (d as f64) * 2.0, 100.0, -6.5 + (d as f64) * 0.2, 1.2], d).unwrap()
                        + if r % 2 == 0 { amp } else { -amp }
                })
                .collect();
            series_xy(d, &format!("DS{d}"), &xs_10, &ys)
        })
        .collect();
    let cfg = SolverConfig {
        parameter_rules: vec![rule_shared_bounds("HillSlope", Some(1.2), Some(0.2), Some(5.0))],
        compute_profile_ci: false,
        compute_hougaard: false,
        ..SolverConfig::default()
    };
    let fit = fit_with(model.as_ref(), &ds_10, &cfg);
    assert_eq!(fit.k_free, 10);
    let exp_df_d = 10.0 - (3.0 + 1.0 / 3.0);
    for (d, ds_res) in fit.datasets.iter().enumerate() {
        assert!((ds_res.sy_x - (ds_res.ss_residual / exp_df_d).sqrt()).abs() < 1e-12, "ds[{d}]");
    }
    assert!(fit.datasets[0].sy_x < fit.datasets[1].sy_x && fit.datasets[1].sy_x < fit.datasets[2].sy_x);
    assert!((fit.datasets[0].sy_x - fit.rsd).abs() > 0.1);
}

#[test]
fn rv96_weighted_and_poisson_result_sheets_label_unweighted_vs_weighted_objective() {
    let xs = [0.5_f64, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let ys: Vec<f64> = xs.iter().enumerate().map(|(i, &x)| 200.0 * x / (5.0 + x) * (1.0 + if i % 2 == 0 { 0.04 } else { -0.04 })).collect();
    let sheet = xy_sheet_single("xy-w", "Weighted Binding", "Y", &xs, &ys);
    let res = exec_nl(&sheet, NonlinearFitSpec { weighting: WeightingScheme::RelativeSquare, ..NonlinearFitSpec::new("one_site_specific_binding") });
    assert_eq!(res.status, ResultStatus::UpToDate);

    let wss_val = cell_num(&tab_row(&res, "nonlinear_gof", "Weighted Sum of Squares")[0]);
    let rss_val = res.lookup_scalar("RSS", None).unwrap();
    let wss_scalar = res.lookup_scalar("Weighted SS", None).unwrap();
    assert!((wss_val - wss_scalar).abs() < 1e-12 && rss_val > wss_val * 100.0);
    assert!(
        find_tab(&res, "nonlinear_gof")
            .footnotes
            .iter()
            .any(|f| f.contains("unweighted residuals in original Y units") && f.contains("weighted / deviance objective"))
    );
}

#[test]
fn rv99_compare_models_guards_mismatched_rout_subsets_and_surfaces_null_alt_roles() {
    let (m4, m3) = (get_builtin_model("log_agonist_vs_response_4pl").unwrap(), get_builtin_model("log_agonist_vs_response_3pl").unwrap());
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let mut ys: Vec<f64> = xs.iter().map(|&x| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 2.5))).collect();
    ys[3] += 45.0;
    let ds = [series_xy(0, "SteepRout", &xs, &ys)];
    let rout_cfg = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        compute_profile_ci: false,
        compute_hougaard: false,
        ..SolverConfig::default()
    };
    let (fit_4, fit_3) = (fit_with(m4.as_ref(), &ds, &rout_cfg), fit_with(m3.as_ref(), &ds, &rout_cfg));

    let mismatched =
        if fit_4.n_obs != fit_3.n_obs || fit_4.rout.as_ref().map(|r| &r.outlier_indices) != fit_3.rout.as_ref().map(|r| &r.outlier_indices) {
            fit_4.clone()
        } else {
            let mut t = fit_4.clone();
            t.n_obs = fit_3.n_obs.saturating_sub(1);
            t.n_outliers_excluded = fit_3.n_outliers_excluded + 1;
            t
        };
    let alt_id = "log_agonist_vs_response_3pl".to_owned();
    for spec in [
        ModelComparisonSpec::Aicc { alternative_equation_id: alt_id.clone() },
        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: alt_id.clone() },
        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: alt_id.clone() },
    ] {
        assert!(matches!(
            compare_models(&mismatched, m3.as_ref(), &ds, &HookContext::default(), &rout_cfg, &spec),
            Err(FitError::InvalidParameter { name: "comparison", .. })
        ));
    }
    if let Some(ref r3) = fit_3.rout {
        let mut diff_idx = fit_3.clone();
        if let Some(ref mut r_mod) = diff_idx.rout {
            r_mod.outlier_indices = if r3.outlier_indices == vec![3] { vec![4] } else { vec![3] };
        }
        assert!(matches!(
            compare_models(
                &diff_idx,
                m3.as_ref(),
                &ds,
                &HookContext::default(),
                &rout_cfg,
                &ModelComparisonSpec::Aicc { alternative_equation_id: alt_id.clone() }
            ),
            Err(FitError::InvalidParameter { name: "comparison", .. })
        ));
    }

    let ys_clean: Vec<f64> =
        xs.iter().enumerate().map(|(i, &x)| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 2.5)) + if i % 2 == 0 { 0.2 } else { -0.2 }).collect();
    let sheet = xy_sheet_single("xy-cmp", "4PL vs 3PL", "Y", &xs, &ys_clean);
    let res = exec_nl(
        &sheet,
        NonlinearFitSpec {
            comparison: Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: alt_id }),
            ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
        },
    );
    assert_eq!(tab_row(&res, "model_comparison_f", "Null hypothesis model")[0], ResultCell::Text(m3.name().to_owned()));
    assert_eq!(tab_row(&res, "model_comparison_f", "Alternative model")[0], ResultCell::Text(m4.name().to_owned()));
    assert!(find_tab(&res, "model_comparison_f").footnotes.iter().any(|f| f.contains("fewer residual degrees of freedom")));
}
