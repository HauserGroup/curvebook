//! Targeted regression and domain-oracle (D1) tests for Wave 1 `curvebook-fit` solver,
//! diagnostics, linear/Deming, result-sheet, TeX formatting, and cross-crate (`AUD-13`, `EXT-01`)
//! remediations.
//!
//! References:
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//! - Motulsky, H. J. & Brown, R. E. (2006). *BMC Bioinformatics* 7, 123. `doi:10.1186/1471-2105-7-123`.
//! - Hurvich, C. M. & Tsai, C.-L. (1989). *Biometrika* 76(2), 297–307.
//! - Burnham, K. P. & Anderson, D. R. (2002). *Model Selection and Multimodel Inference*, 2nd ed. Springer.
//! - Golub, G. H. & Van Loan, C. F. (2013). *Matrix Computations*, 4th ed. Johns Hopkins University Press.
//! - Linnet, K. (1990). *Statistics in Medicine* 9(12), 1463–1473.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, DataSheet, DemingErrorSpec, FitFlagRules, FitMethod, IdentifiabilityMode,
    LinearRegressionSpec, ModelComparisonMethod, ModelComparisonQuestion, ModelComparisonSpec, OutlierHandling, ReplicateFitMode, ResultCell,
    ResultStatus, SubcolumnMode, TableArchetype, WeightingScheme, WeightingSource, XColumnMode, XyTable,
};
use curvebook_fit::diagnostics::compute_covariance_diagnostics;
use curvebook_fit::{
    DataPoint, DatasetSeries, DemingOptions, FitError, FitStatus, HookContext, ModelComparisonOutcome, ParameterNote, ParametricModel,
    SimpleLinearOptions, SolverConfig, assemble_objective_hessian, compare_linear_lines, compare_models, compare_models_question, deming_regression,
    detect_log_x_scale_mismatch, detect_raw_non_log10_x_scale, evaluate_fit_flags, execute_linear_analysis, execute_nonlinear_analysis,
    fit_nonlinear, format_linear_regression_equation_tex, format_model_equation_tex, format_model_equation_tex_for_dataset,
    format_polynomial_equation_tex, get_builtin_model, greedy_hessian_reduction, hessian_kappa_threshold, is_fit_well_defined, is_log_x_equation_id,
    non_log_counterpart_equation_id, simple_linear_regression, suggested_default_dose_response_equation,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::linalg::DenseMatrix;
use curvebook_testkit::assert_approx_eq;

use common::{
    cfg_fast, dsl_compile, exec_nl, find_param, fit_with, nl_fast, rule_const, rule_local_bounds, rule_shared, series_from_pts, series_replicates,
    series_xy, tab_row, xy_pts, xy_sheet, xy_sheet_single,
};

#[test]
fn aud_03_03_is_fit_well_defined_rejects_unconverged_statuses() {
    let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
    let ys = [1.0, 3.0, 5.0, 7.0, 9.0, 11.0];
    let ds = series_xy(0, "A", &xs, &ys);
    let model = get_builtin_model("straight_line").unwrap();
    let mut fit = fit_with(model.as_ref(), &[ds], &cfg_fast());
    assert_eq!(fit.status, FitStatus::Converged);
    assert!(is_fit_well_defined(&fit, None).0);

    for bad_status in [FitStatus::HitMaxIters, FitStatus::Diverged, FitStatus::BadInitialValues, FitStatus::ImpossibleWeights, FitStatus::Ambiguous] {
        fit.status = bad_status;
        let (ok, reasons) = is_fit_well_defined(&fit, None);
        assert!(!ok, "is_fit_well_defined must reject {bad_status:?}");
        assert!(!reasons.is_empty(), "expected rejection reason for {bad_status:?}");
    }
}

#[test]
fn aud_03_04_greedy_reduction_masks_cov_corr_and_fixes_unstable_in_profile_ci() {
    let xs: Vec<f64> = (0..12).map(f64::from).collect();
    let noise = [0.02, -0.015, 0.01, -0.025, 0.015, -0.01, 0.02, -0.02, 0.005, -0.015, 0.025, -0.01];
    let ys: Vec<f64> = xs.iter().zip(noise).map(|(&x, e)| 1.5 + 2.5 * x + e).collect();
    let ds = series_xy(0, "Col", &xs, &ys);
    let model = dsl_compile("Y = A*X + B + C*(X + 1 + 0.00001*X*X)\n# Inits: A=2.0, B=1.0, C=0.0");

    let cfg = SolverConfig {
        identifiability_mode: Some(IdentifiabilityMode::IdentifyUnstableParameters),
        compute_profile_ci: true,
        compute_bands: true,
        ..SolverConfig::default()
    };
    let fit = fit_with(&model, &[ds], &cfg);
    let p_a = find_param(&fit, "A");
    let p_b = find_param(&fit, "B");
    let p_c = find_param(&fit, "C");
    assert!(p_c.notes.contains(&ParameterNote::Unstable));
    assert!(p_c.se.is_nan());
    assert_eq!((p_c.ci_lower, p_c.ci_upper), (CiBound::Open, CiBound::Open));
    assert!(p_c.hougaard_g1.is_nan());

    // Unstable row/col in covariance and correlation matrices must be NaN (not stale full-SVD numbers).
    let idx_c = p_c.slot_index;
    for j in 0..3 {
        assert!(fit.covariance.covariance_matrix[idx_c][j].is_nan(), "cov[{idx_c}][{j}] must be NaN");
        assert!(fit.covariance.covariance_matrix[j][idx_c].is_nan(), "cov[{j}][{idx_c}] must be NaN");
        assert!(fit.covariance.correlation_matrix[idx_c][j].is_nan(), "corr[{idx_c}][{j}] must be NaN");
        assert!(fit.covariance.correlation_matrix[j][idx_c].is_nan(), "corr[{j}][{idx_c}] must be NaN");
    }

    // Stable parameters A and B retain finite reduced-subspace SEs and finite profile CIs.
    assert!(p_a.se.is_finite() && p_a.se > 0.0);
    assert!(p_b.se.is_finite() && p_b.se > 0.0);
    assert!(matches!(p_a.ci_lower, CiBound::Finite(_)) && matches!(p_a.ci_upper, CiBound::Finite(_)));
    assert!(matches!(p_b.ci_lower, CiBound::Finite(_)) && matches!(p_b.ci_upper, CiBound::Finite(_)));
}

#[test]
fn aud_03_05_equilibrated_greedy_hessian_is_scale_invariant_and_handles_large_k() {
    // 1) Scale invariance: multiplying parameter column by 1e-6 must not flip well-conditioned to unstable.
    let xs: Vec<f64> = (0..12).map(f64::from).collect();
    let j_unit: Vec<f64> = xs.iter().flat_map(|&x| [x, 1.0, x * x]).collect();
    let j_scaled: Vec<f64> = xs.iter().flat_map(|&x| [x, 1.0, 1e-6 * x * x]).collect();
    let h_unit = assemble_objective_hessian(&DenseMatrix::from_row_major(12, 3, j_unit).unwrap(), false).unwrap();
    let h_scaled = assemble_objective_hessian(&DenseMatrix::from_row_major(12, 3, j_scaled).unwrap(), false).unwrap();
    let r_unit = greedy_hessian_reduction(&h_unit, hessian_kappa_threshold()).unwrap();
    let r_scaled = greedy_hessian_reduction(&h_scaled, hessian_kappa_threshold()).unwrap();
    assert_eq!(r_unit.unstable_indices, r_scaled.unstable_indices);
    assert!(r_scaled.unstable_indices.is_empty());
    assert_approx_eq(r_unit.final_kappa, r_scaled.final_kappa, 1e-6, 1e-9, "scale-invariant kappa");

    // 2) Zero diagonal column dropped immediately and large K > 32 executes fast via single-SVD pivot.
    let k = 40;
    let mut h_big = DenseMatrix::zeros(k, k).unwrap();
    for i in 0..k {
        h_big.set(i, i, if i == 5 { 0.0 } else { 1.0 }).unwrap();
    }
    // Make columns 10 and 11 strictly collinear.
    h_big.set(10, 11, 1.0).unwrap();
    h_big.set(11, 10, 1.0).unwrap();
    let red_big = greedy_hessian_reduction(&h_big, hessian_kappa_threshold()).unwrap();
    assert!(red_big.unstable_indices.contains(&5), "zero diagonal must be dropped");
    assert!(red_big.unstable_indices.contains(&10) || red_big.unstable_indices.contains(&11), "collinear column must be dropped");
    assert_eq!(red_big.stable_indices.len(), k - 2);
}

#[test]
fn aud_03_06_rank_deficient_covariance_emits_nan_se_and_corr_never_zero() {
    // 2x2 rank-1 Jacobian: column 1 is identically 0 -> parameter 1 unidentifiable.
    let j = DenseMatrix::from_row_major(6, 2, vec![1.0, 0.0, 2.0, 0.0, 3.0, 0.0, 4.0, 0.0, 5.0, 0.0, 6.0, 0.0]).unwrap();
    let cov = compute_covariance_diagnostics(&j, 0.5, 4, &[2.0, 1.0], ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(cov.numerical_rank, 1);
    assert!(cov.standard_errors[0].is_finite() && cov.standard_errors[0] > 0.0);
    assert!(cov.standard_errors[1].is_nan(), "unidentifiable parameter SE must be NaN, not 0.0");
    assert!(cov.symmetric_ci_lower[1].is_nan());
    assert!(cov.symmetric_ci_upper[1].is_nan());
    assert!(cov.correlation_matrix[1][1].is_nan());
    assert!(cov.correlation_matrix[0][1].is_nan());
    assert!(cov.correlation_matrix[1][0].is_nan());

    // Under ReportAll, dead parameter must still have NaN SE and Open CI.
    let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
    let ys = [1.02, 2.98, 5.03, 6.97, 9.01, 10.99];
    let ds = series_xy(0, "Dead", &xs, &ys);
    let dead = dsl_compile("Y = A*X + B + C*0\n# Inits: A=2.0, B=1.0, C=1.0");
    let fit =
        fit_with(&dead, &[ds], &SolverConfig { identifiability_mode: Some(IdentifiabilityMode::ReportAll), compute_profile_ci: true, ..cfg_fast() });
    let pc = find_param(&fit, "C");
    assert!(pc.se.is_nan(), "ReportAll must not emit SE=0 for rank-deficient dead param");
    assert_eq!((pc.ci_lower, pc.ci_upper), (CiBound::Open, CiBound::Open));
}

#[test]
fn aud_03_07_independent_rout_predicted_y_impossible_weights_aborts_cleanly() {
    // Line crossing y=0 with PredictedY 1/Y^2 weighting triggers ImpossibleWeights on dataset 1.
    let ds0 = series_xy(0, "Pos", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[10.0, 12.0, 14.0, 16.0, 18.0, 20.0]);
    let ds1 = series_xy(1, "ZeroHit", &[-2.0, -1.0, 0.0, 1.0, 2.0, 3.0], &[-4.0, -2.0, 0.0, 2.0, 4.0, 6.0]);
    let model = get_builtin_model("straight_line").unwrap();
    let cfg = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        outlier_handling: OutlierHandling::Eliminate,
        weighting: WeightingScheme::RelativeSquare,
        weighting_source: WeightingSource::PredictedY,
        ..cfg_fast()
    };
    let fit = fit_with(model.as_ref(), &[ds0, ds1], &cfg);
    assert_eq!(fit.status, FitStatus::ImpossibleWeights);
    assert!(fit.rout.is_none(), "aborted ROUT must not fabricate outlier audit");
    assert_eq!(fit.n_outliers_excluded, 0);
}

#[test]
fn aud_03_08_single_global_vs_separate_preserves_model_default_constants() {
    // `log_agonist_vs_response_3pl` fixes HillSlope = 1.0 via default_bounds `(Some(1.0), Some(1.0))`.
    let x = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let y_a: Vec<f64> = x.iter().map(|&v| 10.0 + 90.0 / (1.0 + 10.0_f64.powf(-6.5 - v))).collect();
    let y_b: Vec<f64> = x.iter().map(|&v| 10.0 + 90.0 / (1.0 + 10.0_f64.powf(-5.5 - v))).collect();
    let ds = [series_xy(0, "A", &x, &y_a), series_xy(1, "B", &x, &y_b)];
    let model = get_builtin_model("log_agonist_vs_response_3pl").unwrap();
    let cfg = cfg_fast();
    let primary = fit_with(model.as_ref(), &ds, &cfg);
    let q = compare_models_question(
        model.as_ref(),
        &primary,
        &ds,
        &HookContext::default(),
        &cfg,
        ModelComparisonMethod::ExtraSumOfSquaresF,
        &ModelComparisonQuestion::SingleGlobalCurveVsSeparate,
        false,
        None,
    )
    .unwrap();
    // 3PL has 3 free params per dataset (Bottom, Top, LogEC50), NOT 4 (HillSlope must stay fixed!).
    assert_eq!(q.model_a_fit.k_free, 3, "global 3PL must keep HillSlope fixed (k_free=3)");
    assert_eq!(q.model_b_fit.k_free, 6, "separate 3PL must keep HillSlope fixed (k_free=6)");
}

#[test]
fn aud_03_09_predicted_y_ic_and_lrt_include_log_weight_jacobian() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let noise = [0.1, -0.2, 0.15, -0.1, 0.25, -0.3, 0.2, -0.4, 0.3, -0.2];
    let ys: Vec<f64> = xs.iter().zip(noise).map(|(&x, e)| 5.0 + 2.0 * x + 0.15 * x * x + e * (1.0 + 0.1 * x)).collect();
    let ds = [series_xy(0, "Curve", &xs, &ys)];

    let m_lin = get_builtin_model("straight_line").unwrap();
    let m_quad = get_builtin_model("quadratic").unwrap();
    let cfg = SolverConfig { weighting: WeightingScheme::RelativeSquare, weighting_source: WeightingSource::PredictedY, ..cfg_fast() };
    let fit_lin = fit_with(m_lin.as_ref(), &ds, &cfg);
    let fit_quad = fit_with(m_quad.as_ref(), &ds, &cfg);

    let nlw_lin = fit_lin.predicted_y_neg_log_weight_sum.expect("PredictedY log-weight sum recorded");
    let nlw_quad = fit_quad.predicted_y_neg_log_weight_sum.expect("PredictedY log-weight sum recorded");
    // Exact manual check: -sum ln w_i = sum 2 * ln(|y_hat_i|) for 1/Y^2.
    let expected_nlw_lin: f64 = fit_lin.datasets[0].y_hat.iter().map(|yh| 2.0 * yh.abs().ln()).sum();
    assert_approx_eq(nlw_lin, expected_nlw_lin, 1e-10, 1e-10, "neg_log_w_sum = 2 sum ln|y_hat|");

    // Verify AICc matches N*ln(SS_w/N) + neg_log_w + 2*K_ic + 2*K_ic*(K_ic+1)/(N - K_ic - 1), K_ic = k_free + 1.
    let n = fit_lin.n_obs as f64;
    let k_ic = (fit_lin.k_free + 1) as f64;
    let expected_aicc = n * (fit_lin.weighted_ss / n).ln() + nlw_lin + 2.0 * k_ic + (2.0 * k_ic * (k_ic + 1.0)) / (n - k_ic - 1.0);
    assert_approx_eq(fit_lin.aic_c.unwrap(), expected_aicc, 1e-9, 1e-9, "PredictedY AICc with Jacobian term");

    // Verify compare_models(Aicc) uses the Jacobian-corrected AICc values.
    let (_, aicc_out) = compare_models(
        &fit_lin,
        m_quad.as_ref(),
        &ds,
        &HookContext::default(),
        &cfg,
        &ModelComparisonSpec::Aicc { alternative_equation_id: "quadratic".into() },
    )
    .unwrap();
    let ModelComparisonOutcome::Aicc(aicc) = aicc_out else { panic!("expected Aicc") };
    assert_approx_eq(aicc.aicc_a, fit_lin.aic_c.unwrap(), 1e-9, 1e-9, "compare_models aicc_a");
    assert_approx_eq(aicc.aicc_b, fit_quad.aic_c.unwrap(), 1e-9, 1e-9, "compare_models aicc_b");

    // Verify LRT matches exact (-2 ln L_null) - (-2 ln L_alt).
    let (_, lrt_out) = compare_models(
        &fit_lin,
        m_quad.as_ref(),
        &ds,
        &HookContext::default(),
        &cfg,
        &ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: "quadratic".into() },
    )
    .unwrap();
    let ModelComparisonOutcome::LikelihoodRatioTest(lrt) = lrt_out else { panic!("expected LRT") };
    let expected_d = (n * (fit_lin.weighted_ss / n).ln() + nlw_lin) - (n * (fit_quad.weighted_ss / n).ln() + nlw_quad);
    assert_approx_eq(lrt.lr_statistic, expected_d.max(0.0), 1e-8, 1e-8, "PredictedY LRT statistic");
}

#[test]
fn aud_03_10_fd_scale_aware_nanomolar_fit_and_huge_jacobian_never_false_converges() {
    // 1) Custom DSL Michaelis-Menten at nanomolar Kd = 1e-9 M on a narrow box [1e-12, 1e-8].
    let model = dsl_compile("Y = Vmax * X / (Kd + X)\n# Inits: Vmax=90.0, Kd=2e-9");
    let xs = [1e-10, 3e-10, 1e-9, 3e-9, 1e-8, 3e-8, 1e-7];
    let ys: Vec<f64> = xs.iter().map(|&x| 100.0 * x / (1e-9 + x)).collect();
    let ds = [series_xy(0, "nM", &xs, &ys)];
    let cfg =
        SolverConfig { parameter_rules: vec![rule_local_bounds("Kd", Some(2e-9), Some(1e-12), Some(1e-8))], compute_profile_ci: true, ..cfg_fast() };
    let fit = fit_with(&model, &ds, &cfg);
    assert_eq!(fit.status, FitStatus::Converged);
    assert_approx_eq(find_param(&fit, "Vmax").estimate, 100.0, 1e-5, 1e-5, "nM Vmax");
    assert_approx_eq(find_param(&fit, "Kd").estimate, 1e-9, 1e-14, 1e-5, "nM Kd");

    // 2) Model with NaN analytical gradient falls back to FD cleanly instead of false iter-0 convergence.
    struct BadGradLine;
    impl ParametricModel for BadGradLine {
        fn id(&self) -> &'static str {
            "bad_grad_line"
        }
        fn name(&self) -> &'static str {
            "Bad Grad Line"
        }
        fn category(&self) -> curvebook_fit::EquationCategory {
            curvebook_fit::EquationCategory::StandardCurve
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
    let line_ds = [series_xy(0, "L", &[1.0, 2.0, 3.0, 4.0, 5.0], &[5.0, 8.0, 11.0, 14.0, 17.0])];
    let fit_bg = fit_with(&BadGradLine, &line_ds, &cfg_fast());
    assert_eq!(fit_bg.status, FitStatus::Converged);
    assert_approx_eq(find_param(&fit_bg, "m").estimate, 3.0, 1e-8, 1e-8, "FD fallback m");
    assert_approx_eq(find_param(&fit_bg, "b").estimate, 2.0, 1e-8, 1e-8, "FD fallback b");
}

#[test]
fn aud_03_11_inverse_interpolation_rejects_poles_handles_multidecade_and_covers_x_hat() {
    // 1) Pole rejection: Michaelis-Menten Vmax=100, Km=5, y_target=120 (> Vmax) must NOT return pole x=-5.
    let mm = get_builtin_model("michaelis_menten").unwrap();
    let xs = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 100.0 * x / (5.0 + x)).collect();
    let mut ds = series_xy(0, "MM", &xs, &ys);
    ds.unknowns_y.push(curvebook_fit::UnknownResponse::Single((6, 120.0)));
    let fit = fit_with(mm.as_ref(), &[ds], &SolverConfig { interpolate_unknowns: true, ..cfg_fast() });
    assert_eq!(fit.interpolated_x.len(), 1);
    assert!(fit.interpolated_x[0].output_estimate.is_nan(), "pole x=-Km must not be returned as root");

    // 2) Multi-decade positive-X agonist_vs_response_4pl on [1e-4, 1000] with EC50 = 0.5 finds root & finite CI.
    let ag = get_builtin_model("agonist_vs_response_4pl").unwrap();
    let x_dec = [1e-4, 1e-3, 1e-2, 0.1, 0.3, 0.5, 1.0, 3.0, 10.0, 100.0, 1000.0];
    let noise = [0.2, -0.1, 0.15, -0.2, 0.1, -0.15, 0.2, -0.1, 0.15, -0.2, 0.1];
    let y_dec: Vec<f64> = x_dec.iter().zip(noise).map(|(&x, e)| 100.0 * x / (0.5 + x) + e).collect();
    let mut ds_dec = series_xy(0, "MultiDec", &x_dec, &y_dec);
    ds_dec.unknowns_y.push(curvebook_fit::UnknownResponse::Single((11, 50.0)));
    let fit_dec = fit_with(ag.as_ref(), &[ds_dec], &SolverConfig { interpolate_unknowns: true, ..cfg_fast() });
    let ix = &fit_dec.interpolated_x[0];
    assert_approx_eq(ix.output_estimate, 0.5, 0.05, 0.1, "multi-decade EC50 root");
    let (CiBound::Finite(lo), CiBound::Finite(hi)) = (ix.ci_lower, ix.ci_upper) else {
        panic!("expected finite inverse CI on multi-decade curve, got ({:?}, {:?})", ix.ci_lower, ix.ci_upper);
    };
    assert!(lo <= ix.output_estimate && ix.output_estimate <= hi, "CI [{lo}, {hi}] must contain x_hat {}", ix.output_estimate);

    // 3) Non-monotone Gaussian flank: inverse CI must cover x_hat.
    let gauss = get_builtin_model("gaussian").unwrap();
    let xg = [1.0, 2.0, 3.0, 4.0, 4.5, 5.0, 5.5, 6.0, 7.0, 8.0, 9.0];
    let eg = [0.1, -0.1, 0.2, -0.15, 0.1, -0.2, 0.15, -0.1, 0.2, -0.1, 0.1];
    let yg: Vec<f64> = xg.iter().zip(eg).map(|(&x, e)| 100.0 * (-0.5 * ((x - 5.0) / 1.5_f64).powi(2)).exp() + e).collect();
    let mut ds_g = series_xy(0, "Gauss", &xg, &yg);
    ds_g.unknowns_y.push(curvebook_fit::UnknownResponse::Single((11, 80.0)));
    let fit_g = fit_with(gauss.as_ref(), &[ds_g], &SolverConfig { interpolate_unknowns: true, ..cfg_fast() });
    let ig = &fit_g.interpolated_x[0];
    assert!(ig.output_estimate.is_finite());
    if let CiBound::Finite(lo) = ig.ci_lower {
        assert!(lo <= ig.output_estimate, "Gaussian ci_lower {lo} > x_hat {}", ig.output_estimate);
    }
    if let CiBound::Finite(hi) = ig.ci_upper {
        assert!(hi >= ig.output_estimate, "Gaussian ci_upper {hi} < x_hat {}", ig.output_estimate);
    }

    // 4) Tiny-Y (1e-14 scale) inverse interpolation must find the true root x = 5, not first_x.
    let sl = get_builtin_model("straight_line").unwrap();
    let xs_tiny = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0];
    let ys_tiny: Vec<f64> = xs_tiny.iter().map(|&x| 1e-14 * x).collect();
    let mut ds_tiny = series_xy(0, "TinyY", &xs_tiny, &ys_tiny);
    ds_tiny.unknowns_y.push(curvebook_fit::UnknownResponse::Single((7, 5e-14)));
    let fit_tiny = fit_with(sl.as_ref(), &[ds_tiny], &SolverConfig { interpolate_unknowns: true, ..cfg_fast() });
    assert_approx_eq(fit_tiny.interpolated_x[0].output_estimate, 5.0, 1e-6, 1e-6, "tiny-Y inverse root");
}

#[test]
fn aud_03_12_observed_sd_weighting_omits_dimensionally_invalid_prediction_bands() {
    let pts: Vec<DataPoint> = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
        .iter()
        .enumerate()
        .map(|(row, &x)| DataPoint {
            row,
            sd: Some(50.0),
            n: Some(3.0),
            ..DataPoint::xy(x, 1000.0 + 200.0 * x + if row % 2 == 0 { 15.0 } else { -15.0 })
        })
        .collect();
    let mut ds = series_from_pts(0, "ObsSd", pts);
    ds.is_pre_summarized = true;
    ds.unknowns_y.push(curvebook_fit::UnknownResponse::Single((6, 1700.0)));
    let model = get_builtin_model("straight_line").unwrap();
    let cfg = SolverConfig {
        weighting: WeightingScheme::ObservedSd,
        compute_bands: true,
        compute_prediction_bands: true,
        interpolate_unknowns: true,
        ..cfg_fast()
    };
    let fit = fit_with(model.as_ref(), &[ds], &cfg);
    let overlay = fit.datasets[0].curve_overlay.as_ref().unwrap();
    assert!(overlay.points.iter().any(|p| p.ci_lower.is_some() && p.ci_upper.is_some()), "confidence bands stay valid");
    assert!(overlay.points.iter().all(|p| p.pred_lower.is_none() && p.pred_upper.is_none()), "ObservedSd prediction bands must be omitted");
    assert_eq!((fit.interpolated_x[0].pred_lower, fit.interpolated_x[0].pred_upper), (CiBound::Open, CiBound::Open));
}

#[test]
fn aud_03_15_non_first_sheet_columns_fit_custom_dsl_and_builtin_paired_models() {
    // 1) Custom DSL uses sheet data_set_index (<C> = index 2).
    let dsl = dsl_compile("<A> Y = Top * X / (Kd + X)\n<C> Y = Slope * X + Base");
    let ds_c = series_xy(2, "ColC", &[1.0, 2.0, 3.0, 4.0, 5.0], &[5.0, 8.0, 11.0, 14.0, 17.0]);
    let fit_dsl = fit_with(&dsl, &[ds_c], &SolverConfig { parameter_rules: vec![rule_const("Top", 1.0), rule_const("Kd", 1.0)], ..cfg_fast() });
    assert_eq!(fit_dsl.status, FitStatus::Converged);
    assert_approx_eq(find_param(&fit_dsl, "Slope").estimate, 3.0, 1e-6, 1e-6, "<C> Slope");
    assert_approx_eq(find_param(&fit_dsl, "Base").estimate, 2.0, 1e-6, 1e-6, "<C> Base");

    // 2) Built-in `one_site_total_and_ns_global` on selected sheet columns [2, 3] (Total at local 0, NS at local 1).
    let model = get_builtin_model("one_site_total_and_ns_global").unwrap();
    let xs = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0];
    let y_tot: Vec<f64> = xs.iter().map(|&x| 100.0 * x / (2.5 + x) + 1.5 * x + 3.0).collect();
    let y_ns: Vec<f64> = xs.iter().map(|&x| 1.5 * x + 3.0).collect();
    let ds_pair = [series_xy(2, "Total", &xs, &y_tot), series_xy(3, "NS", &xs, &y_ns)];
    let fit_bind = fit_with(
        model.as_ref(),
        &ds_pair,
        &SolverConfig { parameter_rules: vec![rule_shared("Bmax"), rule_shared("Kd"), rule_shared("NS"), rule_shared("Background")], ..cfg_fast() },
    );
    assert_eq!(fit_bind.status, FitStatus::Converged);
    assert_approx_eq(find_param(&fit_bind, "Bmax").estimate, 100.0, 1e-4, 1e-4, "col [2,3] Bmax");
    assert_approx_eq(find_param(&fit_bind, "Kd").estimate, 2.5, 1e-4, 1e-4, "col [2,3] Kd");
    assert_approx_eq(find_param(&fit_bind, "NS").estimate, 1.5, 1e-4, 1e-4, "col [2,3] NS");
    assert_approx_eq(find_param(&fit_bind, "Background").estimate, 3.0, 1e-4, 1e-4, "col [2,3] Background");
}

#[test]
fn aud_03_20_and_aud_13_17_deming_rejects_unsupported_options_and_emits_default_overlay() {
    let sheet = xy_sheet_single("xy-dem", "Dem", "A", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2.1, 3.9, 6.2, 8.1, 9.8, 12.1]);

    // AUD-13-17: default Deming (no plot bounds / range filter) MUST emit FittedCurveOverlay.
    for err_spec in
        [DemingErrorSpec::EqualError, DemingErrorSpec::StandardDeviations { sd_x: 1.0, sd_y: 2.0 }, DemingErrorSpec::VarianceRatio { lambda: 0.5 }]
    {
        let lin_spec = LinearRegressionSpec { deming_model_ii: true, deming_error: err_spec, ..LinearRegressionSpec::default() };
        let spec = AnalysisSpec::new("res-dem-def", "xy-dem", AnalysisKind::LinearRegression(lin_spec)).unwrap();
        let res = execute_linear_analysis(&spec, &lin_spec, &sheet).unwrap();
        assert_eq!(res.fitted_curves.len(), 1, "default Deming must emit FittedCurveOverlay");
        assert!(res.fitted_curves[0].points.len() >= 2);
    }

    // AUD-03-20: unsupported Deming combinations must fail cleanly with InvalidParameter.
    let bad_specs = [
        (LinearRegressionSpec { deming_model_ii: true, force_through_origin: true, ..LinearRegressionSpec::default() }, "fixed_point"),
        (LinearRegressionSpec { deming_model_ii: true, fixed_point: Some((1.0, 2.0)), ..LinearRegressionSpec::default() }, "fixed_point"),
        (LinearRegressionSpec { deming_model_ii: true, standard_addition: true, ..LinearRegressionSpec::default() }, "standard_addition"),
        (LinearRegressionSpec { deming_model_ii: true, interpolate_unknowns: true, ..LinearRegressionSpec::default() }, "interpolate_unknowns"),
    ];
    for (lin_spec, expected_name) in bad_specs {
        let spec = AnalysisSpec::new("res-dem-bad", "xy-dem", AnalysisKind::LinearRegression(lin_spec)).unwrap();
        let err = execute_linear_analysis(&spec, &lin_spec, &sheet).unwrap_err();
        assert!(
            matches!(err, FitError::InvalidParameter { name, .. } if name == expected_name),
            "expected InvalidParameter({expected_name}), got {err:?}"
        );
    }
}

#[test]
fn aud_03_21_rout_row_means_only_rejects_presummarized_and_excludes_all_subcols() {
    // 1) Pre-summarized dataset must still be rejected by ROUT even under RowMeansOnly.
    let mut pre = series_xy(0, "Pre", &[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 4.0, 6.0, 8.0, 10.0]);
    pre.is_pre_summarized = true;
    let model = get_builtin_model("straight_line").unwrap();
    let err = fit_nonlinear(
        model.as_ref(),
        &[pre],
        &HookContext::default(),
        &SolverConfig {
            method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
            outlier_handling: OutlierHandling::Eliminate,
            replicate_fit_mode: ReplicateFitMode::RowMeansOnly,
            ..cfg_fast()
        },
    )
    .unwrap_err();
    assert!(matches!(err, FitError::RequiresRawReplicates));

    // 2) Raw 3-replicate XY sheet with gross outlier on row 4 under RowMeansOnly must mark all 3 subcols.
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), 1, xs.len()).unwrap();
    xy.data_sets[0].title = "Rep".to_owned();
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        let y_base = if r == 4 { 500.0 } else { 2.0 * x + 1.0 };
        for s in 0..3 {
            xy.data_sets[0].cells[r][s] = Cell::number(y_base + (s as f64 - 1.0) * 0.05);
        }
    }
    let sheet = DataSheet::new("xy-rep-rout", "RepRout", TableArchetype::Xy(xy)).unwrap();
    let mut nl = nl_fast("straight_line");
    nl.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl.outlier_handling = OutlierHandling::Eliminate;
    nl.replicate_fit_mode = ReplicateFitMode::RowMeansOnly;
    nl.emit_cleaned_data_tab = true;
    let res = exec_nl(&sheet, nl);
    assert_eq!(res.status, ResultStatus::UpToDate);
    let row4_subcols: Vec<usize> = res.eliminated_outliers.iter().filter(|c| c.row == 4 && c.col == 0).map(|c| c.subcol).collect();
    assert_eq!(row4_subcols, vec![0, 1, 2], "all 3 replicate subcolumns of outlier row 4 must be eliminated");
    let Some(DataSheet { archetype: TableArchetype::Xy(clean_xy), .. }) = &res.analyzable_data else {
        panic!("expected cleaned XY analyzable_data");
    };
    for s in 0..3 {
        assert!(clean_xy.data_sets[0].cells[4][s].excluded, "cleaned sheet row 4 subcol {s} must be excluded");
    }
}

#[test]
fn aud_03_22_compare_linear_lines_exact_parallel_no_cancellation_false_positive() {
    let l1 = xy_pts(&[1.0, 2.0, 3.0, 4.0, 5.0], &[1_000_003.0, 2_000_003.0, 3_000_003.0, 4_000_003.0, 5_000_003.0]);
    let l2 = xy_pts(&[1.0, 2.0, 3.0, 4.0, 5.0], &[1_000_007.0, 2_000_007.0, 3_000_007.0, 4_000_007.0, 5_000_007.0]);
    let cmp = compare_linear_lines(&[&l1, &l2], false).unwrap();
    assert!(!cmp.slopes_significantly_different, "exact parallel lines must NOT be flagged as different slopes");
    assert_approx_eq(cmp.f_slopes, 0.0, 1e-12, 1e-12, "f_slopes == 0");
    assert_approx_eq(cmp.p_slopes, 1.0, 1e-12, 1e-12, "p_slopes == 1");
    assert_eq!(cmp.intercepts_significantly_different, Some(true), "parallel elevation offset must be detected");

    // Non-dyadic exact parallel lines (0.3 * x) must also remain parallel with zero slope difference.
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let nd1 = xy_pts(&xs, &xs.map(|x| 0.3 * x + 1.1));
    let nd2 = xy_pts(&xs, &xs.map(|x| 0.3 * x + 2.7));
    let cmp_nd = compare_linear_lines(&[&nd1, &nd2], false).unwrap();
    assert!(!cmp_nd.slopes_significantly_different);
    assert_approx_eq(cmp_nd.f_slopes, 0.0, 1e-12, 1e-12, "non-dyadic f_slopes == 0");
    assert_eq!(cmp_nd.intercepts_significantly_different, Some(true));

    // Tiny-Y (1e-14 scale) strongly distinct slopes and intercepts must NOT be masked by any 1.0 floor.
    let noise = [1e-17, -1e-17, 1e-17, -1e-17, 1e-17];
    let small_s1 = xy_pts(&xs, &[1e-14 + noise[0], 2e-14 + noise[1], 3e-14 + noise[2], 4e-14 + noise[3], 5e-14 + noise[4]]);
    let small_s2 = xy_pts(&xs, &[5e-14 + noise[0], 10e-14 + noise[1], 15e-14 + noise[2], 20e-14 + noise[3], 25e-14 + noise[4]]);
    let cmp_small_s = compare_linear_lines(&[&small_s1, &small_s2], false).unwrap();
    assert!(cmp_small_s.slopes_significantly_different, "small-Y distinct slopes must be detected (F={})", cmp_small_s.f_slopes);
    assert!(cmp_small_s.f_slopes > 1e5, "expected huge F on small-Y distinct slopes, got {}", cmp_small_s.f_slopes);

    let small_i1 = xy_pts(&xs, &[1e-14 + noise[0], 2e-14 + noise[1], 3e-14 + noise[2], 4e-14 + noise[3], 5e-14 + noise[4]]);
    let small_i2 = xy_pts(&xs, &[6e-14 + noise[0], 7e-14 + noise[1], 8e-14 + noise[2], 9e-14 + noise[3], 10e-14 + noise[4]]);
    let cmp_small_i = compare_linear_lines(&[&small_i1, &small_i2], false).unwrap();
    assert!(!cmp_small_i.slopes_significantly_different);
    assert_eq!(cmp_small_i.intercepts_significantly_different, Some(true), "small-Y distinct intercepts must be detected");
}

#[test]
fn aud_03_23_irls_converges_when_optimal_parameter_is_zero() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 2.0 * x).collect();
    let ds = [series_xy(0, "ZeroInt", &xs, &ys)];
    let model = get_builtin_model("straight_line").unwrap();

    for (method, weighting, source) in [
        (FitMethod::LeastSquares, WeightingScheme::RelativeSquare, WeightingSource::PredictedY),
        (FitMethod::RobustCauchy, WeightingScheme::None, WeightingSource::ObservedY),
    ] {
        let cfg = SolverConfig { method, weighting, weighting_source: source, ..cfg_fast() };
        let fit = fit_with(model.as_ref(), &ds, &cfg);
        assert_eq!(fit.status, FitStatus::Converged, "zero-optimum param must converge under {method:?}/{source:?}");
        assert_approx_eq(find_param(&fit, "YIntercept").estimate, 0.0, 1e-6, 1e-6, "YIntercept ~ 0");
        assert_approx_eq(find_param(&fit, "Slope").estimate, 2.0, 1e-6, 1e-6, "Slope ~ 2");
    }
}

#[test]
fn aud_03_24_rout_identify_only_and_question_union_synchronization() {
    let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    // Quadratic curve with outlier at index 5: straight_line vs quadratic may flag different points.
    let mut ys: Vec<f64> = xs.iter().map(|&x| 1.0 + 0.5 * x + 0.4 * x * x).collect();
    ys[5] += 80.0;
    let ds = [series_xy(0, "Q", &xs, &ys)];
    let m_lin = get_builtin_model("straight_line").unwrap();
    let m_quad = get_builtin_model("quadratic").unwrap();

    // 1) IdentifyAndCountOnly retains all points (n_outliers_excluded == 0): comparison must succeed!
    let cfg_id = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        outlier_handling: OutlierHandling::IdentifyAndCountOnly,
        ..cfg_fast()
    };
    let primary_id = fit_with(m_lin.as_ref(), &ds, &cfg_id);
    assert_eq!(primary_id.n_outliers_excluded, 0);
    let (_, outcome_id) = compare_models(
        &primary_id,
        m_quad.as_ref(),
        &ds,
        &HookContext::default(),
        &cfg_id,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "quadratic".into() },
    )
    .unwrap();
    assert!(matches!(outcome_id, ModelComparisonOutcome::ExtraSumOfSquaresF(_)));

    // 2) Eliminate via compare_models_question must synchronize O_1 U O_2 when models flag different outliers.
    let cfg_el =
        SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, outlier_handling: OutlierHandling::Eliminate, ..cfg_fast() };
    let primary_el = fit_with(m_lin.as_ref(), &ds, &cfg_el);
    let q_res = compare_models_question(
        m_lin.as_ref(),
        &primary_el,
        &ds,
        &HookContext::default(),
        &cfg_el,
        ModelComparisonMethod::ExtraSumOfSquaresF,
        &ModelComparisonQuestion::AlternativeEquation { alternative_equation_id: "quadratic".into() },
        false,
        None,
    )
    .unwrap();
    assert_eq!(q_res.model_a_fit.n_obs, q_res.model_b_fit.n_obs);
    assert_eq!(q_res.model_a_fit.n_outliers_excluded, q_res.model_b_fit.n_outliers_excluded);
}

#[test]
fn aud_03_25_poisson_respects_max_iters_and_profile_ci_converges() {
    let xs: [f64; 9] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0];
    let ys: Vec<f64> = xs.iter().map(|&x| (200.0 * (-0.35 * x).exp() + 5.0).round()).collect();
    let ds = [series_xy(0, "Counts", &xs, &ys)];
    let model = get_builtin_model("one_phase_decay").unwrap();
    let cfg = SolverConfig { method: FitMethod::PoissonDeviance, max_iters: 100, compute_profile_ci: true, ..cfg_fast() };
    let fit = fit_with(model.as_ref(), &ds, &cfg);
    assert_eq!(fit.status, FitStatus::Converged);
    let pk = find_param(&fit, "K");
    assert_approx_eq(pk.estimate, 0.35, 0.03, 0.1, "Poisson decay K");
    assert!(matches!(pk.ci_lower, CiBound::Finite(_)) && matches!(pk.ci_upper, CiBound::Finite(_)));
}

#[test]
fn aud_03_26_zero_tss_r_squared_is_nan_and_flags_min_r_squared() {
    let sheet = xy_sheet_single("xy-const", "Const", "A", &[1.0, 2.0, 3.0, 4.0, 5.0], &[5.0, 5.0, 5.0, 5.0, 5.0]);
    let ds = [series_xy(0, "A", &[1.0, 2.0, 3.0, 4.0, 5.0], &[5.0, 5.0, 5.0, 5.0, 5.0])];
    let model = get_builtin_model("straight_line").unwrap();
    let cfg = SolverConfig { parameter_rules: vec![rule_const("YIntercept", 0.0), rule_const("Slope", 0.0)], ..cfg_fast() };
    let fit = fit_with(model.as_ref(), &ds, &cfg);
    assert!(fit.r_squared.is_nan(), "R^2 must be NaN when TSS == 0");
    assert!(fit.datasets[0].r_squared.is_nan(), "dataset R^2 must be NaN when TSS == 0");
    let flags = evaluate_fit_flags(&fit, &FitFlagRules { min_r_squared: Some(0.95), ..FitFlagRules::default() });
    assert!(flags.flagged, "NaN R^2 must trigger min_r_squared flag");

    // End-to-end ResultSheet must omit non-finite R2 from result.scalars.
    let mut nl = nl_fast("straight_line");
    nl.parameter_rules = vec![rule_const("YIntercept", 0.0), rule_const("Slope", 0.0)];
    let res = exec_nl(&sheet, nl);
    assert!(res.scalars.iter().all(|s| s.value.is_finite()), "result.scalars must never contain non-finite R2");
}

#[test]
fn aud_03_32_independent_datasets_receive_all_column_titles_for_init_rules() {
    let src = "\
        [Equation]\n\
        Y = A + B * X\n\
        [InitialValues]\n\
        A = MeanColumnTitle\n\
        B = 1.0\n\
    ";
    let model = dsl_compile(src);
    let mut d0 = series_xy(0, "10", &[1.0, 2.0, 3.0, 4.0], &[21.0, 22.0, 23.0, 24.0]);
    d0.column_title_value = Some(10.0);
    let mut d1 = series_xy(1, "30", &[1.0, 2.0, 3.0, 4.0], &[21.0, 22.0, 23.0, 24.0]);
    d1.column_title_value = Some(30.0);
    // Plot-function-only shows the exact initial guesses built from all column titles: MeanColumnTitle = 20.0.
    let cfg = SolverConfig { plot_function_only: true, ..cfg_fast() };
    let fit = fit_with(&model, &[d0.clone(), d1.clone()], &cfg);
    assert_approx_eq(fit.datasets[0].params[0], 20.0, 1e-12, 1e-12, "MeanColumnTitle = 20");
    // Also verify independent multi-dataset fit (no shared params) succeeds with MeanColumnTitle.
    let fit_ind = fit_with(&model, &[d0, d1], &cfg_fast());
    assert_eq!(fit_ind.status, FitStatus::Converged);
}

#[test]
fn aud_03_33_and_34_and_13_18_tex_sections_piecewise_and_fixed_point_linear() {
    // AUD-03-33: pre-[Equation] sections and spaced headers.
    let pre_eq = "[InitialValues]\nA = YMAX\n[Equation]\nY = A * exp(-k * X)\n[Derived Scalars]\nHalfLife = ln(2) / k\n";
    assert_eq!(format_model_equation_tex("custom_dsl", Some(pre_eq), &[], false, 4), "Y = A \\cdot \\exp(-k \\cdot X)");

    let spaced = "[Equation]\nY = A * X\n[Initial Value]\nA = 2\n[Derived Scalars]\nDouble = 2 * A\n2 * A\n";
    assert_eq!(format_model_equation_tex("custom_dsl", Some(spaced), &[], false, 4), "Y = A \\cdot X");

    // AUD-03-34: IF(...), function sharing parameter name, negative/multi-digit exponent, ** operator.
    let if_gamma = format_model_equation_tex("custom_dsl", Some("Y = IF(X > 0, gamma * X + gamma(X), 0)"), &[("gamma".into(), 2.5)], true, 4);
    assert_eq!(if_gamma, "Y = \\text{IF}(X > 0, 2.5 \\cdot X + \\text{gamma}(X), 0)");

    let if_neg_pow = format_model_equation_tex("custom_dsl", Some("Y = IF(X > 0, X^b, 0)"), &[("b".into(), -2.5)], true, 4);
    assert_eq!(if_neg_pow, "Y = \\text{IF}(X > 0, X^{-2.5}, 0)");

    let if_big_pow = format_model_equation_tex("custom_dsl", Some("Y = IF(X > 0, X ** b, 0)"), &[("b".into(), 12.0)], true, 4);
    assert_eq!(if_big_pow, "Y = \\text{IF}(X > 0, X^{12}, 0)");

    let mod_and = format_model_equation_tex("custom_dsl", Some("Y = IF(X > 0 && !(X < 1), X % 2, 0)"), &[], false, 4);
    assert!(mod_and.contains("\\land") && mod_and.contains("\\neg") && mod_and.contains("\\bmod"), "mod_and={mod_and}");

    // Unary minus after relational, logical, and modulo operators must stay unary (no `- 2` binary spacing).
    assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = IF(X <= -2, 1, 0)"), &[], false, 4), "Y = \\text{IF}(X \\leq -2, 1, 0)");
    assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = IF(X > -2, 1, 0)"), &[], false, 4), "Y = \\text{IF}(X > -2, 1, 0)");
    assert_eq!(
        format_model_equation_tex("custom_dsl", Some("Y = IF(X >= -2 && X != -1, X % -3, 0)"), &[], false, 4),
        "Y = \\text{IF}(X \\geq -2 \\land X \\neq -1, X \\bmod (-3), 0)"
    );
    assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = X && -2 || -3"), &[], false, 4), "Y = X \\land -2 \\lor -3");
    assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = !(-X)"), &[], false, 4), "Y = \\neg -X");
    assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = !a"), &[("a".into(), -2.0)], true, 4), "Y = \\neg -2");
    // Unparseable fallback path with spaced `a ** X` and negative substituted `a` must wrap `(-2)`.
    let fallback_pow = format_model_equation_tex("custom_dsl", Some("Y = a ** X @ 1"), &[("a".into(), -2.0)], true, 4);
    assert!(fallback_pow.contains("(-2)"), "fallback spaced power base must wrap negative value: {fallback_pow}");

    // AUD-13-18 (fit side): fixed_point linear preserves Y-intercept even if force_through_origin is also set.
    let fp_spec = LinearRegressionSpec { force_through_origin: true, fixed_point: Some((10.0, 25.0)), ..LinearRegressionSpec::default() };
    let fp_tex = format_linear_regression_equation_tex(&fp_spec, &[("Slope".into(), 2.0), ("YIntercept".into(), 5.0)], true, 4);
    assert_eq!(fp_tex, "Y = 2 \\cdot X + 5");
    assert_eq!(format_linear_regression_equation_tex(&fp_spec, &[], false, 4), "Y = \\text{Slope} \\cdot X + \\text{Intercept}");
    // Backwards-compatible legacy caller with non-zero explicit YIntercept also preserves it.
    assert_eq!(format_polynomial_equation_tex(1, true, &[("Slope".into(), 2.0), ("YIntercept".into(), 5.0)], true, 4), "Y = 2 \\cdot X + 5");
    assert_eq!(format_polynomial_equation_tex(1, true, &[("Slope".into(), 2.0), ("YIntercept".into(), 0.0)], true, 4), "Y = 2 \\cdot X");

    // AUD-13-18 (fit side): dataset branch <B> + per-dataset [DS2] parameter substitution.
    let two_branch = "<A> Y = Bmax * X / (Kd + X) + NS * X\n<B> Y = NS * X\n";
    let ds_b_tex = format_model_equation_tex_for_dataset(
        "custom_dsl",
        Some(two_branch),
        &[("NS [DS1]".into(), 1.0), ("NS [DS2]".into(), 3.5)],
        Some(1),
        true,
        4,
    );
    assert_eq!(ds_b_tex, "Y = 3.5 \\cdot X");
}

#[test]
fn aud_03_35_shared_global_robust_apportions_ds_rsdr_and_matches_predicted_y_residuals() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let ds: Vec<DatasetSeries> = (0..4)
        .map(|d| {
            let ys: Vec<f64> = xs.iter().enumerate().map(|(i, &x)| 2.0 * x + 5.0 + if i == 2 { 0.5 } else { 0.1 * ((i + d) as f64 - 2.0) }).collect();
            series_xy(d, &format!("D{d}"), &xs, &ys)
        })
        .collect();
    let model = get_builtin_model("straight_line").unwrap();
    let cfg = SolverConfig { method: FitMethod::RobustCauchy, parameter_rules: vec![rule_shared("Slope"), rule_shared("YIntercept")], ..cfg_fast() };
    let fit = fit_with(model.as_ref(), &ds, &cfg);
    // 2 params shared across 4 datasets => k_eff_d = 0.5 per dataset; N_d = 5 => scale = 5/4.5, NOT 5/3!
    for d in &fit.datasets {
        let p68 = curvebook_fit::compute_rsdr(&d.residuals.iter().map(|r| r.abs()).collect::<Vec<_>>(), 0);
        let expected = p68 * (5.0 / 4.5);
        assert_approx_eq(d.rsdr.unwrap(), expected, 1e-12, 1e-12, "apportioned ds_rsdr");
    }

    // RobustCauchy + PredictedY must retain Cauchy weighting (so outlier weighted residual is downweighted).
    let mut ys_out = [3.0, 5.0, 7.0, 9.0, 11.0, 13.0, 15.0, 17.0];
    ys_out[7] = 200.0;
    let ds_out = [series_xy(0, "Out", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], &ys_out)];
    let fit_py = fit_with(
        model.as_ref(),
        &ds_out,
        &SolverConfig {
            method: FitMethod::RobustCauchy,
            weighting: WeightingScheme::Relative,
            weighting_source: WeightingSource::PredictedY,
            ..cfg_fast()
        },
    );
    let base_w_out = (fit_py.datasets[0].residuals[7]).abs() / fit_py.datasets[0].y_hat[7].abs().sqrt();
    let stored_w_out = fit_py.datasets[0].weighted_residuals[7].abs();
    assert!(stored_w_out < base_w_out * 0.5, "Cauchy weight must downweight outlier in PredictedY weighted_residuals");
}

#[test]
fn aud_03_36_and_37_nelder_mead_upper_bound_and_general_power_nan_guard() {
    // AUD-03-36: model seeded right at upper bound still optimizes inward via Nelder-Mead + LM.
    let model = get_builtin_model("straight_line").unwrap();
    let ds = [series_xy(0, "UB", &[1.0, 2.0, 3.0, 4.0, 5.0], &[3.0, 5.0, 7.0, 9.0, 11.0])];
    let cfg = SolverConfig {
        use_simplex_warm_start: true,
        parameter_rules: vec![rule_local_bounds("Slope", Some(20.0), Some(0.05), Some(20.0))],
        ..cfg_fast()
    };
    let fit = fit_with(model.as_ref(), &ds, &cfg);
    assert_eq!(fit.status, FitStatus::Converged);
    assert_approx_eq(find_param(&fit, "Slope").estimate, 2.0, 1e-6, 1e-6, "escaped upper bound");

    // AUD-03-37: GeneralPowerY(NaN) must error even when every y_i == 1.0.
    let ds_ones = [series_xy(0, "Ones", &[1.0, 2.0, 3.0, 4.0], &[1.0, 1.0, 1.0, 1.0])];
    let err = fit_nonlinear(
        model.as_ref(),
        &ds_ones,
        &HookContext::default(),
        &SolverConfig { weighting: WeightingScheme::GeneralPowerY(f64::NAN), ..cfg_fast() },
    )
    .unwrap_err();
    assert!(matches!(err, FitError::ImpossibleWeights { .. } | FitError::InvalidParameter { .. }));
}

#[test]
fn aud_03_38_linear_fixed_point_flag_overflow_and_large_n_deming_jackknife() {
    let pts = xy_pts(&[1.0, 2.0, 3.0, 4.0, 5.0], &[3.0, 5.0, 7.0, 9.0, 11.0]);
    let res = simple_linear_regression(
        &pts,
        &SimpleLinearOptions { force_through_origin: true, fixed_point: Some((5.0, 10.0)), ..SimpleLinearOptions::default() },
    )
    .unwrap();
    assert!(!res.force_through_origin, "non-origin fixed_point must set force_through_origin = false");

    // Overflowing Y ~ 1e200 must error cleanly instead of returning bogus R2/p_value.
    let huge = xy_pts(&[1.0, 2.0, 3.0], &[1e200, 2.0001e200, 3e200]);
    let err = simple_linear_regression(&huge, &SimpleLinearOptions { compute_bands: false, ..SimpleLinearOptions::default() }).unwrap_err();
    assert!(matches!(err, FitError::InvalidParameter { .. }));

    // Large finite moments (s_xx = s_yy = s_xy = 2e200) must compute Pearson r = 1.0 without (s_xx*s_yy) overflow.
    let big_xy = xy_pts(&[1e100, 2e100, 3e100], &[1e100, 2e100, 3e100]);
    let res_big = simple_linear_regression(&big_xy, &SimpleLinearOptions { compute_bands: false, ..SimpleLinearOptions::default() }).unwrap();
    assert_approx_eq(res_big.r, 1.0, 1e-12, 1e-12, "large-scale Pearson r == 1");
    assert_approx_eq(res_big.r_squared, 1.0, 1e-12, 1e-12, "large-scale R^2 == 1");

    // Large-n Deming O(n) moment-downdated jackknife matches exact values cleanly.
    let n = 120;
    let dpts: Vec<DataPoint> = (0..n)
        .map(|i| {
            let x = (i as f64) * 0.5 + 1.0;
            let y = 1.2 * x + 3.5 + if i % 2 == 0 { 0.3 } else { -0.3 };
            DataPoint::xy(x, y)
        })
        .collect();
    let dem = deming_regression(&dpts, &DemingOptions::default()).unwrap();
    assert!(dem.slope.is_finite() && dem.se_slope.is_finite() && dem.se_slope > 0.0);
    assert!(dem.se_intercept.is_finite() && dem.se_intercept > 0.0);
    assert_approx_eq(dem.slope, 1.2, 1e-2, 1e-2, "large-n Deming slope");
}

#[test]
fn aud_03_39_aud_13_05_and_aud_13_15_unstable_summary_masking_and_cleaned_data_precedence() {
    let xs: Vec<f64> = (0..12).map(f64::from).collect();
    let noise = [0.02, -0.015, 0.01, -0.025, 0.015, -0.01, 0.02, -0.02, 0.005, -0.015, 0.025, -0.01];
    let y0: Vec<f64> = xs.iter().zip(noise).map(|(&x, e)| 1.5 + 2.5 * x + e).collect();
    let mut y1: Vec<f64> = xs.iter().zip(noise).map(|(&x, e)| 2.0 + 3.0 * x - e).collect();
    y1[6] = 400.0; // planted ROUT outlier
    let sheet = xy_sheet("xy-sum-rout", "SumRout", &xs, &[("10", &y0), ("20", &y1)]);

    // 1) AUD-13-05 + AUD-03-39: IdentifyUnstableParameters masks C in Parameter Summary tab,
    //    leaves C cell empty in analyzable_data, and omits C from result.scalars.
    let mut nl_u = nl_fast("ill_cond");
    nl_u.custom_dsl = Some("Y = A*X + B + C*(X + 1 + 0.00001*X*X)\n# Inits: A=2.0, B=1.0, C=0.0".to_owned());
    nl_u.identifiability_mode = Some(IdentifiabilityMode::IdentifyUnstableParameters);
    nl_u.summary_parameter = Some("C".to_owned());
    let res_u = exec_nl(&sheet, nl_u);
    assert!(res_u.scalars.iter().all(|s| !s.key.eq_ignore_ascii_case("C")), "unstable C must not leak into result.scalars");
    let row0 = tab_row(&res_u, "nonlinear_parameter_summary", "10");
    assert_eq!(row0[1], ResultCell::Text("Unstable".to_owned()));
    let Some(DataSheet { archetype: TableArchetype::Xy(sum_xy), .. }) = &res_u.analyzable_data else {
        panic!("expected Parameter Summary XY analyzable_data");
    };
    assert!(sum_xy.data_sets[0].cells[0][0].active_number().is_none(), "unstable C must remain empty in analyzable_data");

    // 2) AUD-13-15 + AUD-03-39: emit_cleaned_data_tab must NOT overwrite Parameter Summary in analyzable_data.
    let mut nl_both = nl_fast("straight_line");
    nl_both.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl_both.outlier_handling = OutlierHandling::Eliminate;
    nl_both.summary_parameter = Some("Slope".to_owned());
    nl_both.emit_cleaned_data_tab = true;
    let res_both = exec_nl(&sheet, nl_both);
    assert!(res_both.tabs.iter().any(|t| t.id == "cleaned_data"), "Cleaned data tab still emitted");
    assert!(res_both.tabs.iter().any(|t| t.id == "nonlinear_parameter_summary"), "Parameter Summary tab emitted");
    let ad = res_both.analyzable_data.as_ref().expect("analyzable_data present");
    assert!(ad.id.as_str().ends_with("_param_summary"), "Cleaned data must NOT clobber Parameter Summary analyzable_data (got {})", ad.id.as_str());
}

#[test]
fn aud_03_42_result_omits_non_finite_derived_scalars() {
    let sheet = xy_sheet_single("xy-der", "Der", "A", &[1.0, 2.0, 3.0, 4.0], &[3.0, 5.0, 7.0, 9.0]);
    let mut nl = nl_fast("custom_nan_derived");
    nl.custom_dsl = Some("[Equation]\nY = m * X + b\n[Transforms]\nBad = 1 / (m - 2)\nGood = m * 10\n".to_owned());
    let res = exec_nl(&sheet, nl);
    assert!(res.scalars.iter().all(|s| s.value.is_finite()), "non-finite derived scalars must be omitted from result.scalars");

    // NistModel::eval and NistModel::grad both reject non-finite inputs cleanly (e.g. 1.0.powf(NaN)).
    let danwood = get_builtin_model("nist_danwood").unwrap();
    assert!(danwood.eval(&[1.0], &[1.0, f64::NAN], 0).is_none());
    let mut g = [0.0; 2];
    assert!(!danwood.grad(&[1.0], &[1.0, f64::NAN], 0, &mut g));
    assert!(!danwood.grad(&[f64::NAN], &[1.0, 2.0], 0, &mut g));
}

#[test]
fn aud_13_08_dual_interpolated_unknowns_emits_valid_upper_lower_limits_table() {
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 7).unwrap();
    xy.data_sets[0].title = "Std".to_owned();
    for (r, &x) in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0].iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(2.0 * x + 1.0 + 0.05);
        xy.data_sets[0].cells[r][1] = Cell::number(2.0 * x + 1.0 - 0.05);
    }
    // Row 6: unknown Y replicates (X blank).
    xy.data_sets[0].cells[6][0] = Cell::number(7.9);
    xy.data_sets[0].cells[6][1] = Cell::number(8.1);
    let sheet = DataSheet::new("xy-interp", "Interp", TableArchetype::Xy(xy)).unwrap();
    let mut nl = nl_fast("straight_line");
    nl.interpolate_unknowns = true;
    let res = exec_nl(&sheet, nl);
    let Some(DataSheet { archetype: TableArchetype::Xy(ix_xy), .. }) = &res.analyzable_data else {
        panic!("expected dual Interpolated Unknowns XY analyzable_data");
    };
    assert_eq!(ix_xy.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::UpperLowerLimits });
    assert_eq!(ix_xy.x_title, "Interpolated X");
    assert!(ix_xy.x_cells.len() >= 3, "expected mean + 2 replicate rows");
}

#[test]
fn ext_01_raw_x_scale_detection_and_nonlinear_checklist_warning() {
    assert!(is_log_x_equation_id("log_agonist_vs_response_4pl"));
    assert!(is_log_x_equation_id("4pl"));
    assert!(is_log_x_equation_id("absolute_ic50_4pl"));
    assert!(is_log_x_equation_id("operational_model_depletion_log"));
    for unpaired in [
        "one_site_fit_ki",
        "one_site_fit_logic50",
        "two_sites_fit_logic50",
        "two_sites_fit_ki",
        "homologous_one_site_competition",
        "one_site_competition_ligand_depletion",
        "allosteric_modulator_competition",
    ] {
        assert!(is_log_x_equation_id(unpaired), "{unpaired} must be recognized as a log10(X) equation");
        assert_eq!(non_log_counterpart_equation_id(unpaired), None);
    }
    assert!(!is_log_x_equation_id("agonist_vs_response_4pl"));
    assert!(!is_log_x_equation_id("log_gaussian"));
    assert_eq!(non_log_counterpart_equation_id("log_agonist_vs_response_4pl"), Some("agonist_vs_response_4pl"));
    assert_eq!(non_log_counterpart_equation_id("log_inhibitor_vs_response_4pl"), Some("inhibitor_vs_response_4pl"));
    assert_eq!(non_log_counterpart_equation_id("4pl"), Some("agonist_vs_response_4pl"));
    assert_eq!(non_log_counterpart_equation_id("absolute_ic50_4pl"), Some("absolute_ic50_4pl_conc"));
    assert_eq!(non_log_counterpart_equation_id("operational_model_depletion_log"), Some("operational_model_depletion_conc"));

    // Raw molar concentrations (1e-9..1e-6), multi-decade positive (0.01..10), and > 20 all trigger.
    let raw_molar = [1e-9, 1e-8, 1e-7, 1e-6];
    let raw_decades = [0.01, 0.1, 1.0, 10.0];
    let raw_large = [0.0, 10.0, 30.0, 100.0, 300.0];
    for xs in [&raw_molar[..], &raw_decades[..], &raw_large[..]] {
        assert!(detect_raw_non_log10_x_scale(xs).is_some(), "must flag raw X {xs:?}");
        assert_eq!(suggested_default_dose_response_equation(xs), "agonist_vs_response_4pl");
        let w = detect_log_x_scale_mismatch("log_agonist_vs_response_4pl", xs).unwrap();
        assert_eq!(w.suggested_equation_id.as_deref(), Some("agonist_vs_response_4pl"));
        let w_unpaired = detect_log_x_scale_mismatch("one_site_fit_ki", xs).unwrap();
        assert_eq!(w_unpaired.suggested_equation_id, None);
    }

    // Legitimate log10(X) scales must NOT false-positive.
    for log_xs in [&[-9.0, -8.0, -7.0, -6.0, -5.0][..], &[-3.0, -2.0, -1.0, 0.0, 1.0, 2.0][..], &[0.0, 1.0, 2.0, 3.0, 4.0][..]] {
        assert!(detect_raw_non_log10_x_scale(log_xs).is_none(), "must NOT flag log10 X {log_xs:?}");
        assert_eq!(suggested_default_dose_response_equation(log_xs), "log_agonist_vs_response_4pl");
        assert!(detect_log_x_scale_mismatch("log_agonist_vs_response_4pl", log_xs).is_none());
    }

    // End-to-end execute_nonlinear_analysis emits ChecklistStatus::Warn suggesting agonist_vs_response_4pl.
    let ys_molar: Vec<f64> = raw_molar.iter().map(|&c| 10.0 + 90.0 * c / (3e-8 + c)).collect();
    let sheet_molar = xy_sheet_single("xy-molar", "Molar", "Drug", &raw_molar, &ys_molar);
    let nl = nl_fast("log_agonist_vs_response_4pl");
    let spec = AnalysisSpec::new("res-molar", "xy-molar", AnalysisKind::NonlinearRegression(nl.clone())).unwrap();
    let res = execute_nonlinear_analysis(&spec, &nl, &sheet_molar, &HookContext::default()).unwrap();
    let warn_item = res.checklist.iter().find(|c| c.status == ChecklistStatus::Warn && c.explanation.contains("agonist_vs_response_4pl"));
    assert!(warn_item.is_some(), "expected EXT-01 X-scale warning checklist item in execute_nonlinear_analysis");

    // Unpaired log10(X) binding model also emits ChecklistStatus::Warn recommending log10(X) transform.
    let res_ki = exec_nl(&sheet_molar, nl_fast("one_site_fit_logic50"));
    assert!(
        res_ki.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.explanation.contains("Transform X to log10(X)")),
        "expected EXT-01 log10(X) warning on unpaired competition model"
    );

    // Respect nl_spec.x_range_filter: an out-of-range raw X point excluded by x_range_filter must not trigger warning.
    let log_x_with_outlier = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, 100.0];
    let log_y_with_outlier: Vec<f64> = log_x_with_outlier.iter().map(|&x| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.2))).collect();
    let sheet_filtered = xy_sheet_single("xy-log-filt", "LogFilt", "Drug", &log_x_with_outlier, &log_y_with_outlier);
    let mut nl_filt = nl_fast("log_agonist_vs_response_4pl");
    nl_filt.x_range_filter = Some((Some(-10.0), Some(-3.0)));
    let res_filt = exec_nl(&sheet_filtered, nl_filt);
    assert!(res_filt.checklist.iter().all(|c| !c.assumption.contains("X-axis scale")), "excluded out-of-range X=100 must not trigger EXT-01 warning");

    // Clean log10(X) sheet emits NO X-scale warning.
    let log_x = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let log_y: Vec<f64> = log_x.iter().map(|&x| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.2))).collect();
    let sheet_log = xy_sheet_single("xy-log", "Log", "Drug", &log_x, &log_y);
    let res_log = exec_nl(&sheet_log, nl_fast("log_agonist_vs_response_4pl"));
    assert!(res_log.checklist.iter().all(|c| !c.assumption.contains("X-axis scale")));
    let _ = series_replicates::<2>(0, "R", &[1.0], &[[1.0, 2.0]]);
}
