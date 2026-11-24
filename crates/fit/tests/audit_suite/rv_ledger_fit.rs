#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Regression tests for the 2026-11-16 milestone review ledger (`RV-23`, `RV-24`,
//! `RV-36`..`RV-42`, `RV-68`, `RV-69`, and Section 2 `S3` notes) in `curvebook-fit`.
//!
//! Literature references:
//! - Motulsky, H. J. & Christopoulos, A. (2004). *Fitting Models to Biological Data using
//!   Linear and Nonlinear Regression*. Oxford University Press.
//! - Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear
//!   regression – a new method based on robust nonlinear regression and the false discovery rate.
//!   *BMC Bioinformatics* 7:123.
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//! - Venzon, D. J. & Moolgavkar, S. H. (1988). A method for computing profile-likelihood-based
//!   confidence intervals. *Applied Statistics* 37(1), 87–94.
//! - McCullagh, P. & Nelder, J. A. (1989). *Generalized Linear Models*, 2nd ed. Chapman & Hall.
//! - Dormand, J. R. & Prince, P. J. (1980). A family of embedded Runge–Kutta formulae.
//!   *J. Comput. Appl. Math.* 6(1), 19–26.
//! - Hairer, E., Nørsett, S. P. & Wanner, G. (1993). *Solving Ordinary Differential Equations I*,
//!   2nd rev. ed. Springer.
//! - Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*. Prentice-Hall.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, ConfidenceLevel, DataSheet, FitMethod, ModelComparisonSpec, NonlinearFitSpec, ParameterRule,
    ParameterScope, ResultCell, ResultStatus, SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
};
use curvebook_fit::diagnostics::compute_covariance_diagnostics;
use curvebook_fit::dsl::CompiledDslModel;
use curvebook_fit::error::{FitError, FitStatus};
use curvebook_fit::library::{ParametricModel, get_builtin_model};
use curvebook_fit::result::execute_analysis;
use curvebook_fit::solver::{FittedParameterSummary, NonlinearFitResult, SolverConfig, compare_models, fit_nonlinear};
use curvebook_fit::tex_format::format_model_equation_tex;
use curvebook_fit::types::{DataPoint, DatasetSeries, EquationCategory, HookContext};
use curvebook_math::dist::student_t_two_sided_isf;
use curvebook_math::linalg::DenseMatrix;

fn series_xy(idx: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
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

fn unwrap_finite_ci(ci: (CiBound, CiBound), ctx: &str) -> (f64, f64) {
    match ci {
        (CiBound::Finite(lo), CiBound::Finite(hi)) => (lo, hi),
        other => panic!("{ctx}: expected finite CI bounds, got {other:?}"),
    }
}

#[test]
fn rv23_and_rv24_tex_text_escapes_and_unary_sign_after_cdot() {
    // RV-23: `\`, `<`, `>` inside `\text{...}` use valid LaTeX text escapes rather than `\\`, `\<`, `\>`.
    let tex_id = format_model_equation_tex(r"param_a<b\c>d*e", None, &[], false, 4);
    assert!(tex_id.contains(r"\textless{}"), "expected \\textless{{}}, got: {tex_id}");
    assert!(tex_id.contains(r"\textbackslash{}"), "expected \\textbackslash{{}}, got: {tex_id}");
    assert!(tex_id.contains(r"\textgreater{}"), "expected \\textgreater{{}}, got: {tex_id}");
    assert!(tex_id.contains(r"\*"), "expected \\*, got: {tex_id}");
    assert!(!tex_id.contains(r"\<") && !tex_id.contains(r"\>") && !tex_id.contains(r"\\"), "invalid raw escapes in: {tex_id}");

    // RV-24: `\cdot` before a negative literal in fallback custom DSL formats `-3` as unary, not binary `- 3`.
    let fallback_mul_neg = format_model_equation_tex("custom_dsl", Some("Y = a % b + c * -3"), &[], false, 4);
    assert!(fallback_mul_neg.contains(r"\cdot -3"), "expected `\\cdot -3` without binary spacing, got: {fallback_mul_neg}");
    assert!(!fallback_mul_neg.contains(r"\cdot - 3"), "unexpected binary `- 3` after \\cdot: {fallback_mul_neg}");
}

#[test]
fn rv36_dsl_exponentiation_binds_tighter_than_prefix_unary() {
    // `-2^2 == -4` while `(-2)^2 == 4`.
    let neg_lit = CompiledDslModel::compile("neg_lit", "Y = -2^2").unwrap();
    assert_eq!(neg_lit.eval(&[0.0], &[], 0), Some(-4.0));

    let paren_lit = CompiledDslModel::compile("paren_lit", "Y = (-2)^2").unwrap();
    assert_eq!(paren_lit.eval(&[0.0], &[], 0), Some(4.0));

    // `-X^2 + A` evaluates `-(X^2) + A` and fits upward-offset downward parabola cleanly.
    let quad = CompiledDslModel::compile("neg_quad", "Y = -X^2 + A").unwrap();
    assert_eq!(quad.eval(&[3.0], &[10.0], 0), Some(1.0));
    assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = -X^2 + A"), &[], false, 4), "Y = -X^{2} + A");

    let xs = [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0];
    let ys: Vec<f64> = xs.iter().map(|&x| -(x * x) + 12.5).collect();
    let fit = fit_nonlinear(&quad, &[series_xy(0, "Parabola", &xs, &ys)], &HookContext::default(), &SolverConfig::default()).unwrap();
    assert_eq!(fit.status, FitStatus::Converged);
    assert!((find_param(&fit, "A").estimate - 12.5).abs() < 1e-10);

    // Unary minus in the right-hand exponent remains permitted (`X^-2 == X^(-2)` and `-X^-2 == -(X^(-2))`).
    let inv_sq = CompiledDslModel::compile("inv_sq", "Y = X^-2").unwrap();
    assert_eq!(inv_sq.eval(&[2.0], &[], 0), Some(0.25));

    let neg_inv_sq = CompiledDslModel::compile("neg_inv_sq", "Y = -X^-2").unwrap();
    assert_eq!(neg_inv_sq.eval(&[2.0], &[], 0), Some(-0.25));

    // Right-associativity with unary exponent (`2^3^2 == 512`, `2^-3^2 == 2^(-9)`).
    let tower = CompiledDslModel::compile("tower", "Y = 2^3^2").unwrap();
    assert_eq!(tower.eval(&[0.0], &[], 0), Some(512.0));

    let neg_exp_tower = CompiledDslModel::compile("neg_exp_tower", "Y = 2^-3^2").unwrap();
    assert!((neg_exp_tower.eval(&[0.0], &[], 0).unwrap() - 2.0_f64.powf(-9.0)).abs() < 1e-15);
}

#[test]
fn rv37_heteroscedastic_inverse_y2_rout_and_robust_cauchy_use_weighted_residuals() {
    // Heteroscedastic calibration spanning ~3 decades (Y ~ 7 .. 2005) with ~2% proportional scatter
    // and ONE planted +180% relative outlier at low Y (index 1: Y_true = 9.0 -> Y_obs = 25.2, raw
    // residual = +16.2). High-Y points have normal 2% proportional jitter (e.g. raw residual ~ +45
    // at X = 1000), which exceeds +16.2 in raw units but is tiny (0.022) in 1/Y-weighted units.
    let xs = [1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0, 35.0, 50.0, 80.0, 120.0, 180.0, 260.0, 380.0, 520.0, 700.0, 850.0, 1000.0];
    let rel_jitter =
        [0.015, -0.018, 0.020, -0.012, 0.016, -0.022, 0.019, -0.014, 0.021, -0.017, 0.013, -0.020, 0.018, -0.016, 0.022, -0.019, 0.021, -0.023];
    let mut ys: Vec<f64> = xs.iter().zip(rel_jitter.iter()).map(|(&x, &e)| (2.0 * x + 5.0) * (1.0 + e)).collect();
    // Plant a single severe relative outlier at low Y (index 1, X = 2, Y_true = 9 -> Y_obs = 25.2).
    ys[1] = 25.2;

    let line = get_builtin_model("straight_line").unwrap();
    let ds = series_xy(0, "HetCalib", &xs, &ys);

    // 1. Weighted ROUT (1/Y^2, Q = 1%) per Motulsky & Brown (2006) flags ONLY index 1 and zero high-Y points.
    let cfg_rout = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        weighting: WeightingScheme::RelativeSquare,
        ..SolverConfig::default()
    };
    let fit_rout = fit_nonlinear(line.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg_rout).unwrap();
    assert_eq!(fit_rout.status, FitStatus::Converged);
    let rout = fit_rout.rout.as_ref().expect("ROUT audit present");
    assert_eq!(rout.outlier_indices, vec![1], "expected only low-Y relative outlier index 1, got {:?}", rout.outlier_indices);
    assert!((find_param(&fit_rout, "Slope").estimate - 2.0).abs() < 0.03, "Slope={}", find_param(&fit_rout, "Slope").estimate);
    assert!((find_param(&fit_rout, "YIntercept").estimate - 5.0).abs() < 0.15, "YIntercept={}", find_param(&fit_rout, "YIntercept").estimate);

    // 2. Multi-dataset independent ROUT with 1/Y^2 weighting also standardizes per-dataset residuals.
    let mut ds2 = ds.clone();
    ds2.data_set_index = 1;
    ds2.label = "HetCalib2".to_owned();
    for pt in &mut ds2.points {
        pt.data_set_index = 1;
    }
    let fit_rout_multi = fit_nonlinear(line.as_ref(), &[ds.clone(), ds2], &HookContext::default(), &cfg_rout).unwrap();
    let rout_multi = fit_rout_multi.rout.as_ref().unwrap();
    assert_eq!(rout_multi.outlier_indices, vec![1, xs.len() + 1]);

    // 3. Robust Cauchy IRLS with 1/Y^2 weighting recovers parameters accurately and reports weighted RSDR.
    let cfg_cauchy = SolverConfig { method: FitMethod::RobustCauchy, weighting: WeightingScheme::RelativeSquare, ..SolverConfig::default() };
    let fit_cauchy = fit_nonlinear(line.as_ref(), &[ds], &HookContext::default(), &cfg_cauchy).unwrap();
    assert_eq!(fit_cauchy.status, FitStatus::Converged);
    assert!((find_param(&fit_cauchy, "Slope").estimate - 2.0).abs() < 0.04, "Cauchy Slope={}", find_param(&fit_cauchy, "Slope").estimate);
    assert!(
        (find_param(&fit_cauchy, "YIntercept").estimate - 5.0).abs() < 0.20,
        "Cauchy YIntercept={}",
        find_param(&fit_cauchy, "YIntercept").estimate
    );
    let rsdr = fit_cauchy.rsdr.expect("RSDR reported in RobustCauchy");
    assert!(rsdr > 0.005 && rsdr < 0.08, "expected dimensionless relative RSDR ~ 0.02, got {rsdr}");
}

#[test]
fn rv38_poisson_deviance_rejects_non_unweighted_schemes_and_explicit_weights() {
    let exp_model = get_builtin_model("exponential_growth").unwrap();
    let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
    let ys = [4.0, 7.0, 12.0, 20.0, 33.0, 55.0];
    let ds = series_xy(0, "Counts", &xs, &ys);

    let ok_cfg = SolverConfig { method: FitMethod::PoissonDeviance, weighting: WeightingScheme::None, ..SolverConfig::default() };
    assert!(fit_nonlinear(exp_model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &ok_cfg).is_ok());

    for bad_scheme in [
        WeightingScheme::RelativeSquare,
        WeightingScheme::Relative,
        WeightingScheme::GeneralPowerY(2.0),
        WeightingScheme::InverseX,
        WeightingScheme::InverseX2,
        WeightingScheme::ObservedSd,
    ] {
        let bad_cfg = SolverConfig { method: FitMethod::PoissonDeviance, weighting: bad_scheme, ..SolverConfig::default() };
        let err = fit_nonlinear(exp_model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &bad_cfg)
            .expect_err("non-None WeightingScheme must be rejected for PoissonDeviance");
        assert!(matches!(err, FitError::InvalidParameter { name: "weighting", .. }), "unexpected error for {bad_scheme:?}: {err:?}");
    }

    let mut weighted_ds = ds;
    weighted_ds.points[0].explicit_weight = Some(2.0);
    let err_exp = fit_nonlinear(exp_model.as_ref(), &[weighted_ds], &HookContext::default(), &ok_cfg)
        .expect_err("explicit_weight must be rejected for PoissonDeviance");
    assert!(matches!(err_exp, FitError::InvalidParameter { name: "weighting", .. }));
}

#[test]
fn rv39_profile_likelihood_cis_are_scale_invariant_at_1e_minus_6() {
    let mm = get_builtin_model("michaelis_menten").unwrap();
    let xs = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let noise = [0.4, -0.5, 0.7, -0.8, 1.1, -0.9, 0.6, -0.5];
    let ys_unit: Vec<f64> = xs.iter().zip(noise.iter()).map(|(&x, &e)| 100.0 * x / (6.0 + x) + e).collect();
    let scale = 1e-6_f64;
    let ys_micro: Vec<f64> = ys_unit.iter().map(|&y| y * scale).collect();

    let cfg = SolverConfig { confidence: ConfidenceLevel::DEFAULT_95, compute_profile_ci: true, ..SolverConfig::default() };
    let fit_unit = fit_nonlinear(mm.as_ref(), &[series_xy(0, "Unit", &xs, &ys_unit)], &HookContext::default(), &cfg).unwrap();
    let fit_micro = fit_nonlinear(mm.as_ref(), &[series_xy(0, "Micro", &xs, &ys_micro)], &HookContext::default(), &cfg).unwrap();

    assert_eq!(fit_unit.status, FitStatus::Converged);
    assert_eq!(fit_micro.status, FitStatus::Converged);
    assert!((fit_micro.weighted_ss / (scale * scale * fit_unit.weighted_ss) - 1.0).abs() < 1e-8);

    // Amplitude parameter `Vmax` scales by `1e-6`.
    let vmax_u = find_param(&fit_unit, "Vmax");
    let vmax_m = find_param(&fit_micro, "Vmax");
    let (vmax_u_lo, vmax_u_hi) = unwrap_finite_ci((vmax_u.ci_lower, vmax_u.ci_upper), "unit Vmax");
    let (vmax_m_lo, vmax_m_hi) = unwrap_finite_ci((vmax_m.ci_lower, vmax_m.ci_upper), "micro Vmax");
    assert!(((vmax_m.estimate / scale) - vmax_u.estimate).abs() / vmax_u.estimate < 1e-6);
    assert!(((vmax_m_lo / scale) - vmax_u_lo).abs() / vmax_u_lo < 1e-5, "Vmax lo: unit={vmax_u_lo}, micro/s={}", vmax_m_lo / scale);
    assert!(((vmax_m_hi / scale) - vmax_u_hi).abs() / vmax_u_hi < 1e-5, "Vmax hi: unit={vmax_u_hi}, micro/s={}", vmax_m_hi / scale);

    // Affinity parameter `Km` is invariant (`1.0x`) under Y rescaling!
    // Previously (`RV-39`), `run_reduced_lm` stopped after 1 nuisance step when SS ~ 1e-11, making Km's profile CI too narrow.
    let km_u = find_param(&fit_unit, "Km");
    let km_m = find_param(&fit_micro, "Km");
    let (km_u_lo, km_u_hi) = unwrap_finite_ci((km_u.ci_lower, km_u.ci_upper), "unit Km");
    let (km_m_lo, km_m_hi) = unwrap_finite_ci((km_m.ci_lower, km_m.ci_upper), "micro Km");
    assert!((km_m.estimate - km_u.estimate).abs() / km_u.estimate < 1e-6);
    assert!((km_m_lo - km_u_lo).abs() / km_u_lo < 1e-5, "Km lo: unit={km_u_lo}, micro={km_m_lo}");
    assert!((km_m_hi - km_u_hi).abs() / km_u_hi < 1e-5, "Km hi: unit={km_u_hi}, micro={km_m_hi}");
}

#[test]
fn rv40_rejected_inner_trust_region_steps_check_optimality_before_converged() {
    struct DomainWallStallModel {
        params: Vec<String>,
    }
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
            &self.params
        }
        fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
            // Evaluates smoothly up to `a = 1.0`, while every positive descent step toward optimal `a = 3.0` fails domain check.
            if params[0] <= 1.0 { Some(params[0] * x[0]) } else { None }
        }
        fn grad(&self, x: &[f64], _params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
            out[0] = x[0];
            true
        }
        fn initial_guess(&self, _points: &[DataPoint]) -> Vec<f64> {
            vec![1.0]
        }
    }

    let model = DomainWallStallModel { params: vec!["a".to_owned()] };
    let xs = [1.0, 2.0, 3.0, 4.0];
    let ys = [3.0, 6.0, 9.0, 12.0];
    let cfg = SolverConfig { use_simplex_warm_start: false, compute_profile_ci: false, ..SolverConfig::default() };
    let fit = fit_nonlinear(&model, &[series_xy(0, "Stalled", &xs, &ys)], &HookContext::default(), &cfg).unwrap();
    assert_eq!(fit.status, FitStatus::Diverged, "stalled fit with large gradient must not report Converged");
    assert!(!fit.datasets[0].curve_overlay.as_ref().unwrap().converged);
}

#[test]
fn rv41_noisy_1e_minus_16_data_gets_proper_profile_cis_and_exact_fit_stays_zero_width() {
    let mm = get_builtin_model("michaelis_menten").unwrap();
    let xs = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let noise = [0.5, -0.6, 0.8, -0.9, 1.0, -0.7, 0.4];
    let ys_unit: Vec<f64> = xs.iter().zip(noise.iter()).map(|(&x, &e)| 50.0 * x / (5.0 + x) + e).collect();
    let scale = 1e-16_f64;
    let ys_tiny: Vec<f64> = ys_unit.iter().map(|&y| y * scale).collect();

    let cfg = SolverConfig { confidence: ConfidenceLevel::DEFAULT_95, compute_profile_ci: true, ..SolverConfig::default() };
    let fit_unit = fit_nonlinear(mm.as_ref(), &[series_xy(0, "Unit", &xs, &ys_unit)], &HookContext::default(), &cfg).unwrap();
    let fit_tiny = fit_nonlinear(mm.as_ref(), &[series_xy(0, "Tiny1e-16", &xs, &ys_tiny)], &HookContext::default(), &cfg).unwrap();

    assert_eq!(fit_tiny.status, FitStatus::Converged);
    assert!(fit_tiny.weighted_ss > 0.0 && fit_tiny.weighted_ss < 1e-30, "expected 0 < SS < 1e-30, got {}", fit_tiny.weighted_ss);

    let vmax_u = find_param(&fit_unit, "Vmax");
    let vmax_t = find_param(&fit_tiny, "Vmax");
    let (vmax_u_lo, vmax_u_hi) = unwrap_finite_ci((vmax_u.ci_lower, vmax_u.ci_upper), "unit Vmax");
    let (vmax_t_lo, vmax_t_hi) = unwrap_finite_ci((vmax_t.ci_lower, vmax_t.ci_upper), "tiny Vmax");
    assert!(vmax_t_lo < vmax_t.estimate && vmax_t.estimate < vmax_t_hi, "tiny Vmax profile CI collapsed: [{vmax_t_lo}, {vmax_t_hi}]");
    assert!(((vmax_t_lo / scale) - vmax_u_lo).abs() / vmax_u_lo < 1e-4);
    assert!(((vmax_t_hi / scale) - vmax_u_hi).abs() / vmax_u_hi < 1e-4);

    let km_u = find_param(&fit_unit, "Km");
    let km_t = find_param(&fit_tiny, "Km");
    let (km_u_lo, km_u_hi) = unwrap_finite_ci((km_u.ci_lower, km_u.ci_upper), "unit Km");
    let (km_t_lo, km_t_hi) = unwrap_finite_ci((km_t.ci_lower, km_t.ci_upper), "tiny Km");
    assert!(km_t_lo < km_t.estimate && km_t.estimate < km_t_hi, "tiny Km profile CI collapsed: [{km_t_lo}, {km_t_hi}]");
    assert!((km_t_lo - km_u_lo).abs() / km_u_lo < 1e-4);
    assert!((km_t_hi - km_u_hi).abs() / km_u_hi < 1e-4);

    // Exact noise-free data still cleanly produces zero-width finite CIs `[theta, theta]`.
    let ys_exact_tiny: Vec<f64> = xs.iter().map(|&x| scale * 50.0 * x / (5.0 + x)).collect();
    let fit_exact = fit_nonlinear(mm.as_ref(), &[series_xy(0, "ExactTiny", &xs, &ys_exact_tiny)], &HookContext::default(), &cfg).unwrap();
    let km_ex = find_param(&fit_exact, "Km");
    let (km_ex_lo, km_ex_hi) = unwrap_finite_ci((km_ex.ci_lower, km_ex.ci_upper), "exact Km");
    assert!((km_ex_hi - km_ex_lo).abs() <= 1e-12);
}

#[test]
fn rv42_dsl_min_max_propagate_nan_and_modulo_zero_is_checked() {
    for bad_expr in [
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
        let m = CompiledDslModel::compile("bad", bad_expr).unwrap();
        assert_eq!(m.eval(&[3.0], &[], 0), None, "`{bad_expr}` must propagate domain error as None");
    }

    let ok_min_inf = CompiledDslModel::compile("min_inf", "Y = min(X, INF)").unwrap();
    assert_eq!(ok_min_inf.eval(&[4.5], &[], 0), Some(4.5));

    let ok_rem = CompiledDslModel::compile("ok_rem", "Y = X % 2").unwrap();
    assert_eq!(ok_rem.eval(&[7.5], &[], 0), Some(1.5));
}

#[test]
fn rv68_rk45_ode_shrinks_step_when_trial_stages_probe_outside_domain() {
    // dY/dX = -sqrt(Y), Y(0) = 1 has exact solution Y(X) = (1 - X/2)^2 on [0, 2].
    // Near X -> 2 (Y -> 0+), Dormand–Prince stages probe Y < 0; rejecting the trial step and
    // shrinking `h` allows integration right up to the boundary without IntrinsicFailed.
    let ode = CompiledDslModel::compile("sqrt_decay_ode", "Y0 = 1\ndY/dX = -sqrt(Y)").unwrap();
    for x in [0.25_f64, 0.75, 1.25, 1.75, 1.95, 1.99, 1.999, 1.9999] {
        let expected = (1.0 - 0.5 * x).powi(2);
        let got = ode.eval(&[x], &[], 0).unwrap_or_else(|| panic!("RK45 failed near Y=0 at X={x}"));
        assert!((got - expected).abs() < 5e-7, "X={x}: got {got}, expected {expected}");
    }

    // Fitting parameterized `Y0 = A, dY/dX = -k * sqrt(Y)` (`Y(X) = (sqrt(A) - k*X/2)^2`) near Y = 0 converges.
    let ode_fit_model = CompiledDslModel::compile("param_sqrt_ode", "Y0 = A\ndY/dX = -k * sqrt(Y)").unwrap();
    let xs = [0.0, 0.4, 0.8, 1.2, 1.6, 1.9, 1.98];
    let ys: Vec<f64> = xs.iter().map(|&x| (2.0_f64 - 1.0 * x).powi(2)).collect(); // A = 4.0, k = 2.0 -> zero at X = 2.0
    let cfg = SolverConfig {
        parameter_rules: vec![
            ParameterRule {
                name: "A".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(3.5),
                lower_bound: Some(0.1),
                upper_bound: None,
            },
            ParameterRule {
                name: "k".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(1.7),
                lower_bound: Some(0.1),
                upper_bound: None,
            },
        ],
        compute_profile_ci: false,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(&ode_fit_model, &[series_xy(0, "SqrtOde", &xs, &ys)], &HookContext::default(), &cfg).unwrap();
    assert_eq!(fit.status, FitStatus::Converged);
    assert!((find_param(&fit, "A").estimate - 4.0).abs() < 1e-4);
    assert!((find_param(&fit, "k").estimate - 2.0).abs() < 1e-4);

    // Invalid initial state (`k1` fails at t0) still fails immediately.
    let bad_init = CompiledDslModel::compile("bad_init_ode", "Y0 = -1\ndY/dX = -sqrt(Y)").unwrap();
    assert_eq!(bad_init.eval(&[0.5], &[], 0), None);
}

#[test]
fn rv69_implicit_root_finder_rejects_poles_and_finds_true_roots() {
    // Pure pole `1 / (Y - A) = 0` has sign change across Y = A, but |F| -> inf; must return None.
    let pure_pole = CompiledDslModel::compile("pure_pole", "1 / (Y - A) = 0").unwrap();
    assert_eq!(pure_pole.eval(&[1.0], &[2.0], 0), None);

    // Scaled tiny pole `1e-20 / (Y - A) = 0` must also be rejected.
    let tiny_pole = CompiledDslModel::compile("tiny_pole", "1e-20 / (Y - A) = 0").unwrap();
    assert_eq!(tiny_pole.eval(&[1.0], &[2.0], 0), None);

    // Jump discontinuity `IF(Y > A, 1, -1) = 0` has no root and must return None.
    let step_jump = CompiledDslModel::compile("step_jump", "IF(Y > A, 1, -1) = 0").unwrap();
    assert_eq!(step_jump.eval(&[1.0], &[2.0], 0), None);

    // `1 / (Y - 0.5) + 1 = 0` has a pole at Y = +0.5 (probed first on positive half-line) and
    // a genuine root at Y = -0.5; rejecting the pole allows the solver to return Y = -0.5!
    let pole_plus_root = CompiledDslModel::compile("pole_plus_root", "1 / (Y - 0.5) + 1 = 0").unwrap();
    let root = pole_plus_root.eval(&[1.0], &[], 0).expect("must skip pole at +0.5 and find root at -0.5");
    assert!((root - (-0.5)).abs() < 1e-11, "expected root -0.5, got {root}");
}

#[test]
fn rv_section2_s3_notes_three_phase_bounds_and_covariance_t_critical() {
    // 1. ThreePhaseDecay bounds PercentFast and PercentSlow to [0, 100] and rate constants to >= 0.
    let m3 = get_builtin_model("three_phase_decay").unwrap();
    assert_eq!(
        m3.default_bounds(),
        vec![(None, None), (None, None), (Some(0.0), Some(100.0)), (Some(0.0), Some(100.0)), (Some(0.0), None), (Some(0.0), None), (Some(0.0), None),]
    );

    // 2. `compute_covariance_diagnostics` handles `df == 0` with NaN CIs and uses exact Student-t for df >= 1.
    let j = DenseMatrix::from_row_major(2, 1, vec![1.0, 1.0]).unwrap();
    let cov_df0 = compute_covariance_diagnostics(&j, 0.0, 0, &[5.0], ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(cov_df0.standard_errors[0].is_nan());
    assert!(cov_df0.symmetric_ci_lower[0].is_nan() && cov_df0.symmetric_ci_upper[0].is_nan());

    let conf99 = ConfidenceLevel::new(99.0).unwrap();
    let cov_df1 = compute_covariance_diagnostics(&j, 2.0, 1, &[5.0], conf99).unwrap();
    let expected_t = student_t_two_sided_isf(conf99.alpha(), 1.0).unwrap();
    let expected_half = expected_t * cov_df1.standard_errors[0];
    assert!((cov_df1.symmetric_ci_upper[0] - (5.0 + expected_half)).abs() < 1e-10);
}

#[test]
fn rv93_global_fit_per_dataset_sy_x_uses_effective_dataset_df_not_total_k_free() {
    // Three datasets with 3 local parameters (Top, Bottom, LogEC50) + 1 SharedGlobal (HillSlope) -> k_free = 10,
    // effective parameters per dataset k_eff_d = 3 + 1/3 = 10/3.
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs_10: Vec<f64> = (-9..=0).map(|i| -9.0 + (i as f64) * 0.55).collect();
    let scales = [0.2_f64, 1.0, 4.0];
    let ds_10: Vec<DatasetSeries> = scales
        .iter()
        .enumerate()
        .map(|(d, &amp)| {
            let ys: Vec<f64> = xs_10
                .iter()
                .enumerate()
                .map(|(r, &x)| {
                    let clean = model.eval(&[x], &[10.0 + (d as f64) * 2.0, 100.0, -6.5 + (d as f64) * 0.2, 1.2], d).unwrap();
                    let jitter = if r % 2 == 0 { amp } else { -amp };
                    clean + jitter
                })
                .collect();
            series_xy(d, &format!("DS{d}"), &xs_10, &ys)
        })
        .collect();

    let cfg = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: String::from("HillSlope"),
            scope: ParameterScope::SharedGlobal,
            initial_override: Some(1.2),
            lower_bound: Some(0.2),
            upper_bound: Some(5.0),
        }],
        compute_profile_ci: false,
        compute_hougaard: false,
        ..SolverConfig::default()
    };
    let fit_10 = fit_nonlinear(model.as_ref(), &ds_10, &HookContext::default(), &cfg).unwrap();
    assert_eq!(fit_10.k_free, 10);

    // Previously n_d = 10 <= k_free = 10 fell back to global `fit_10.rsd` for all 3 datasets!
    // Now each dataset uses df_d = 10 - (3 + 1/3) = 20/3, reflecting its own residual scale.
    let expected_df_d = 10.0 - (3.0 + 1.0 / 3.0);
    for (d, ds_res) in fit_10.datasets.iter().enumerate() {
        let expected_sy_x = (ds_res.ss_residual / expected_df_d).sqrt();
        assert!((ds_res.sy_x - expected_sy_x).abs() < 1e-12, "ds[{d}] sy_x={} vs expected={expected_sy_x}", ds_res.sy_x);
    }
    assert!(fit_10.datasets[0].sy_x < fit_10.datasets[1].sy_x && fit_10.datasets[1].sy_x < fit_10.datasets[2].sy_x);
    assert!((fit_10.datasets[0].sy_x - fit_10.rsd).abs() > 0.1);
}

#[test]
fn rv96_weighted_and_poisson_result_sheets_label_unweighted_vs_weighted_objective() {
    let xs = [0.5_f64, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let ys: Vec<f64> = xs.iter().enumerate().map(|(i, &x)| 200.0 * x / (5.0 + x) * (1.0 + if i % 2 == 0 { 0.04 } else { -0.04 })).collect();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new("xy-w", "Weighted Binding", TableArchetype::Xy(xy)).unwrap();
    let mut nl = NonlinearFitSpec::new("one_site_specific_binding");
    nl.weighting = WeightingScheme::RelativeSquare;
    let spec = AnalysisSpec::new("fit-w", "xy-w", AnalysisKind::NonlinearRegression(nl)).unwrap();
    let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    assert_eq!(res.status, ResultStatus::UpToDate);

    let gof = res.tabs.iter().find(|t| t.id == "nonlinear_gof").expect("nonlinear_gof tab");
    let wss_row = gof.row_labels.iter().position(|l| l == "Weighted Sum of Squares").expect("Weighted Sum of Squares row");
    let wss_val = match gof.rows[wss_row][0] {
        ResultCell::Number { value, .. } => value,
        ref other => panic!("expected numeric Weighted SS, got {other:?}"),
    };
    let rss_val = res.lookup_scalar("RSS", None).unwrap();
    let wss_scalar = res.lookup_scalar("Weighted SS", None).unwrap();
    assert!((wss_val - wss_scalar).abs() < 1e-12);
    assert!(rss_val > wss_val * 100.0, "unweighted RSS ({rss_val}) vs 1/Y^2 Weighted SS ({wss_val}) must differ clearly");
    assert!(
        gof.footnotes.iter().any(|f| f.contains("unweighted residuals in original Y units") && f.contains("weighted / deviance objective")),
        "missing unweighted-vs-weighted footnote: {:?}",
        gof.footnotes
    );
}

#[test]
fn rv99_compare_models_guards_mismatched_rout_subsets_and_surfaces_null_alt_roles() {
    let model_4pl = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let model_3pl = get_builtin_model("log_agonist_vs_response_3pl").unwrap();

    // 1. Construct a steep Hill=2.5 curve with one spike outlier near the inflection where 3PL
    // (fixed HillSlope=1.0) vs 4PL (variable HillSlope) exclude different observation sets under ROUT.
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let mut ys: Vec<f64> = xs.iter().map(|&x| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 2.5))).collect();
    // Plant a spike at index 3 (X=-7.5) and verify 4PL vs 3PL differ in excluded points (or test explicit mismatch).
    ys[3] += 45.0;
    let ds = vec![series_xy(0, "SteepRout", &xs, &ys)];
    let rout_cfg = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        compute_profile_ci: false,
        compute_hougaard: false,
        ..SolverConfig::default()
    };
    let fit_4pl = fit_nonlinear(model_4pl.as_ref(), &ds, &HookContext::default(), &rout_cfg).unwrap();
    let fit_3pl = fit_nonlinear(model_3pl.as_ref(), &ds, &HookContext::default(), &rout_cfg).unwrap();

    // If these two fits excluded different subsets, compare_models must reject directly;
    // otherwise construct a primary fit from a subset-altered ROUT result to verify the guard unconditionally.
    let mismatched_primary =
        if fit_4pl.n_obs != fit_3pl.n_obs || fit_4pl.rout.as_ref().map(|r| &r.outlier_indices) != fit_3pl.rout.as_ref().map(|r| &r.outlier_indices) {
            fit_4pl.clone()
        } else {
            let mut tampered = fit_4pl.clone();
            tampered.n_obs = fit_3pl.n_obs.saturating_sub(1);
            tampered.n_outliers_excluded = fit_3pl.n_outliers_excluded + 1;
            tampered
        };
    for spec in [
        ModelComparisonSpec::Aicc { alternative_equation_id: String::from("log_agonist_vs_response_3pl") },
        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: String::from("log_agonist_vs_response_3pl") },
        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: String::from("log_agonist_vs_response_3pl") },
    ] {
        let err = compare_models(&mismatched_primary, model_3pl.as_ref(), &ds, &HookContext::default(), &rout_cfg, &spec)
            .expect_err("compare_models must reject mismatched observation/ROUT outlier subsets");
        assert!(matches!(err, FitError::InvalidParameter { name: "comparison", .. }), "unexpected err: {err:?}");
    }

    // Also verify identical n_obs + different excluded outlier index set is rejected!
    if let Some(ref r3) = fit_3pl.rout {
        let mut diff_idx_primary = fit_3pl.clone();
        if let Some(ref mut r_mod) = diff_idx_primary.rout {
            r_mod.outlier_indices = if r3.outlier_indices == vec![3] { vec![4] } else { vec![3] };
        }
        let err = compare_models(
            &diff_idx_primary,
            model_3pl.as_ref(),
            &ds,
            &HookContext::default(),
            &rout_cfg,
            &ModelComparisonSpec::Aicc { alternative_equation_id: String::from("log_agonist_vs_response_3pl") },
        )
        .expect_err("different ROUT outlier_indices with same count must be rejected");
        assert!(matches!(err, FitError::InvalidParameter { name: "comparison", .. }));
    }

    // 2. When primary is 4PL (complex, fewer df) and alt is 3PL (simpler, more df) on identical points,
    // `execute_analysis` must label Null=3PL, Alt=4PL, and emit an explicit role-assignment footnote.
    let ys_clean: Vec<f64> =
        xs.iter().enumerate().map(|(i, &x)| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 2.5)) + if i % 2 == 0 { 0.2 } else { -0.2 }).collect();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(ys_clean.iter()).enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new("xy-cmp", "4PL vs 3PL", TableArchetype::Xy(xy)).unwrap();
    let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl.comparison = Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: String::from("log_agonist_vs_response_3pl") });
    let res =
        execute_analysis(&AnalysisSpec::new("fit-cmp", "xy-cmp", AnalysisKind::NonlinearRegression(nl)).unwrap(), &sheet, &HookContext::default())
            .unwrap();
    let cmp_tab = res.tabs.iter().find(|t| t.id == "model_comparison_f").expect("model_comparison_f tab");
    let null_row = cmp_tab.row_labels.iter().position(|l| l == "Null hypothesis model").unwrap();
    let alt_row = cmp_tab.row_labels.iter().position(|l| l == "Alternative model").unwrap();
    assert_eq!(cmp_tab.rows[null_row][0], ResultCell::Text(model_3pl.name().to_owned()));
    assert_eq!(cmp_tab.rows[alt_row][0], ResultCell::Text(model_4pl.name().to_owned()));
    assert!(cmp_tab.footnotes.iter().any(|f| f.contains("fewer residual degrees of freedom")));
}
