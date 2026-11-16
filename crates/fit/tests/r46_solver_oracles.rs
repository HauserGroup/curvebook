#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Reproducer and D1 reference-oracle suite for `R4.6` solver & results fixes in
//! [`curvebook_fit::solver`] and [`curvebook_fit::result`].
//!
//! # References
//! - Akaike, H. (1974). A new look at the statistical model identification.
//!   *IEEE Transactions on Automatic Control* 19(6), 716–723.
//! - Hurvich, C. M. & Tsai, C.-L. (1989). Regression and time series model selection in small
//!   samples. *Biometrika* 76(2), 297–307.
//! - Schwarz, G. (1978). Estimating the dimension of a model. *Annals of Statistics* 6(2), 461–464.
//! - Burnham, K. P. & Anderson, D. R. (2002). *Model Selection and Multimodel Inference*, 2nd ed.
//! - McCullagh, P. & Nelder, J. A. (1989). *Generalized Linear Models*, 2nd ed. Chapman & Hall.
//! - Draper, N. R. & Smith, H. (1998). *Applied Regression Analysis*, 3rd ed. Wiley.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//! - Seber, G. A. F. & Lee, A. J. (2003). *Linear Regression Analysis*, 2nd ed., §6.1.3. Wiley.
//! - Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear
//!   regression – a new method based on robust nonlinear regression and the false discovery rate.
//!   *BMC Bioinformatics* 7, 123.
//! - Benjamini, Y. & Hochberg, Y. (1995). Controlling the false discovery rate. *JRSS-B* 57(1), 289–300.
//! - Moré, J. J. (1978). The Levenberg–Marquardt algorithm: implementation and theory.
//!   *Lecture Notes in Mathematics* 630, 105–116.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, DataSheet, FitMethod, LinearRegressionSpec, ModelComparisonSpec, NonlinearFitSpec, ParameterRule,
    ParameterScope, ResultCell, SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, FitError, FitStatus, HookContext, ModelComparisonOutcome, SolverConfig, compare_models,
    compare_models_checked, execute_analysis, fit_nonlinear, get_builtin_model,
};

fn assert_close(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, label: &str) {
    assert!(actual.is_finite(), "{label}: expected finite value near {expected}, got {actual}");
    let diff = (actual - expected).abs();
    let bound = abs_tol.max(rel_tol * expected.abs());
    assert!(diff <= bound, "{label}: actual={actual:.15e}, expected={expected:.15e}, diff={diff:.3e} > bound={bound:.3e}");
}

fn make_series(ds_idx: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
    DatasetSeries {
        data_set_index: ds_idx,
        label: label.to_owned(),
        points: xs
            .iter()
            .zip(ys.iter())
            .enumerate()
            .map(|(row, (&x, &y))| DataPoint { row, data_set_index: ds_idx, ..DataPoint::xy(x, y) })
            .collect(),
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn find_param(res: &curvebook_fit::NonlinearFitResult, name: &str) -> f64 {
    res.parameters.iter().find(|p| p.name.eq_ignore_ascii_case(name)).unwrap_or_else(|| panic!("missing parameter `{name}`")).estimate
}

#[test]
fn r46_2_poisson_aic_aicc_bic_and_model_comparison_reproducer_and_oracle() {
    // Counts generated around mu(x) = 100 * exp(-0.3 * x) over N = 10 time points.
    let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0];
    let ys = [100.0, 74.0, 55.0, 41.0, 30.0, 22.0, 16.0, 9.0, 5.0, 3.0];
    let ds = vec![make_series(0, "Counts", &xs, &ys)];

    let exp2 = CompiledDslModel::compile("exp2", "Y = A * exp(-K * X)").unwrap();
    let decay3 = get_builtin_model("one_phase_decay").unwrap();
    let poisson_cfg = SolverConfig { method: FitMethod::PoissonDeviance, compute_profile_ci: false, ..SolverConfig::default() };

    let fit2 = fit_nonlinear(&exp2, &ds, &HookContext::default(), &poisson_cfg).unwrap();
    assert_eq!(fit2.status, FitStatus::Converged);

    // Exact Poisson log-likelihood, AIC, AICc (K = k_free = 2, fixed dispersion phi = 1), and BIC:
    // Independent Python/R oracle at the MLE mu_hat (within < 0.05 deviance of A=100, K=0.3 where
    // D = 0.05134817, ll = -24.68603945, AICc = 55.08636460, BIC = 53.97724908).
    // Before R4.6 fix: solver fed Poisson deviance (~0.05) into Gaussian `N * ln(SS / N) + 2*(k+1)`,
    // yielding a nonsensical negative AICc around -42.7!
    let mut ll_expected = 0.0_f64;
    for (&y, &mu) in ys.iter().zip(fit2.datasets[0].y_hat.iter()) {
        let lg = curvebook_math::special::ln_gamma(y + 1.0).unwrap();
        ll_expected += y * mu.ln() - mu - lg;
    }
    let neg2_ll = -2.0 * ll_expected;
    let k = 2.0_f64;
    let n = 10.0_f64;
    let expected_aicc = neg2_ll + 2.0 * k + (2.0 * k * (k + 1.0)) / (n - k - 1.0);
    let expected_bic = neg2_ll + k * n.ln();

    let got_aicc = fit2.aic_c.expect("Poisson fit must emit likelihood-based AICc");
    let got_bic = fit2.bic.expect("Poisson fit must emit likelihood-based BIC");
    assert!(got_aicc > 50.0, "Poisson AICc must be likelihood-based (>50), not Gaussian N*ln(D/N) ({got_aicc})");
    assert_close(got_aicc, expected_aicc, 1e-10, 1e-10, "Poisson AICc exact oracle");
    assert_close(got_bic, expected_bic, 1e-10, 1e-10, "Poisson BIC exact oracle");

    // 1) Extra-sum-of-squares F test MUST be rejected for PoissonDeviance fits.
    let f_spec = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "one_phase_decay".to_owned() };
    let f_err = compare_models(&fit2, decay3.as_ref(), &ds, &HookContext::default(), &poisson_cfg, &f_spec)
        .expect_err("Extra-SS F test must be rejected for PoissonDeviance");
    assert!(matches!(f_err, FitError::InvalidParameter { name: "comparison", .. }), "expected InvalidParameter, got {f_err:?}");

    let f_checked_err = compare_models_checked(&fit2, decay3.as_ref(), &ds, &HookContext::default(), &poisson_cfg, &f_spec)
        .expect_err("compare_models_checked must reject Extra-SS F under PoissonDeviance");
    assert!(matches!(f_checked_err, FitError::InvalidParameter { name: "comparison", .. }));

    // 2) Mixed error models (Poisson primary vs Gaussian config) must be rejected.
    let ls_cfg = SolverConfig { method: FitMethod::LeastSquares, compute_profile_ci: false, ..SolverConfig::default() };
    let mixed_err = compare_models(
        &fit2,
        decay3.as_ref(),
        &ds,
        &HookContext::default(),
        &ls_cfg,
        &ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: "one_phase_decay".to_owned() },
    )
    .expect_err("Mixed Poisson vs LeastSquares comparison must fail");
    assert!(matches!(mixed_err, FitError::InvalidParameter { name: "comparison", .. }));

    // 3) Poisson AICc comparison must use Poisson likelihood AICc (K = k_free), not Gaussian N*ln(SS/N).
    let (fit3, aicc_cmp) = compare_models(
        &fit2,
        decay3.as_ref(),
        &ds,
        &HookContext::default(),
        &poisson_cfg,
        &ModelComparisonSpec::Aicc { alternative_equation_id: "one_phase_decay".to_owned() },
    )
    .expect("Poisson AICc comparison must succeed");
    let ModelComparisonOutcome::Aicc(a) = aicc_cmp else {
        panic!("expected Aicc outcome, got {aicc_cmp:?}");
    };
    let expected_aicc_3 = fit3.aic_c.expect("3-param Poisson AICc");
    assert_close(a.aicc_a, expected_aicc, 1e-10, 1e-10, "Poisson compare AICc A");
    assert_close(a.aicc_b, expected_aicc_3, 1e-10, 1e-10, "Poisson compare AICc B");
    assert_close(a.delta_aicc, expected_aicc_3 - expected_aicc, 1e-10, 1e-10, "Poisson delta AICc");
    assert_eq!(a.preferred_index, 0, "parsimonious 2-param Poisson model must be preferred");

    // 4) Poisson Deviance LRT: D_null - D_alt ~ chi^2_1 on nested exponential with/without nonzero baseline.
    let ys_plateau = [120.0, 94.0, 75.0, 61.0, 50.0, 42.0, 36.0, 29.0, 25.0, 23.0];
    let ds_plat = vec![make_series(0, "CountsPlateau", &xs, &ys_plateau)];
    let fit2_p = fit_nonlinear(&exp2, &ds_plat, &HookContext::default(), &poisson_cfg).unwrap();
    let (fit3_p, lrt_cmp) = compare_models(
        &fit2_p,
        decay3.as_ref(),
        &ds_plat,
        &HookContext::default(),
        &poisson_cfg,
        &ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: "one_phase_decay".to_owned() },
    )
    .expect("Poisson deviance LRT must succeed");
    let ModelComparisonOutcome::LikelihoodRatioTest(lrt) = lrt_cmp else {
        panic!("expected LikelihoodRatioTest outcome");
    };
    assert_eq!(lrt.df_diff, 1);
    assert_close(lrt.lr_statistic, (fit2_p.weighted_ss - fit3_p.weighted_ss).max(0.0), 1e-10, 1e-10, "Deviance LRT D_null - D_alt");
    assert!(lrt.preferred_complex_model && lrt.p_value < 0.01);
}

#[test]
fn r46_3_replicates_lack_of_fit_weighted_and_presummarized_oracles() {
    let line = get_builtin_model("straight_line").unwrap();
    let hooks = HookContext::default();

    // 1) Reproducer: 1/Y^2 weighted regression on curved data (y = 100 + 2x + 25x^2).
    // Before R4.6 fix: weighted_ss (~0.61) minus unweighted pure error (82.5) clamped to 0.0 => F = 0.0, p = 1.0!
    // Exact 60-digit Python oracle for 1/Y^2 weighted lack-of-fit:
    // ss_res_w = 0.610666268049, ss_pe_w = 0.000134510441, ss_lof_w = 0.610531757608, F = 18155.668852.
    let rep_xs = [1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 4.0, 4.0, 4.0, 8.0, 8.0, 8.0];
    let rep_ys_curv = [126.5, 127.0, 127.5, 203.0, 204.0, 205.0, 506.0, 508.0, 510.0, 1710.0, 1716.0, 1722.0];
    let ds_curv = vec![make_series(0, "WlsCurved", &rep_xs, &rep_ys_curv)];
    let cfg_wls_lof =
        SolverConfig { weighting: WeightingScheme::RelativeSquare, compute_lack_of_fit: true, compute_profile_ci: false, ..SolverConfig::default() };
    let fit_curv = fit_nonlinear(line.as_ref(), &ds_curv, &hooks, &cfg_wls_lof).unwrap();
    let lof_curv = fit_curv.lack_of_fit.expect("WLS curved data must emit lack_of_fit");
    assert_eq!(lof_curv.df_pure_error, 8);
    assert_eq!(lof_curv.df_lack_of_fit, 2);
    assert_close(lof_curv.ss_pure_error, 0.000_134_510_441, 1e-8, 1e-6, "WLS curved ss_pe_w");
    assert_close(lof_curv.ss_lack_of_fit, 0.610_531_757_608, 1e-8, 1e-6, "WLS curved ss_lof_w");
    assert_close(lof_curv.ss_pure_error + lof_curv.ss_lack_of_fit, fit_curv.weighted_ss, 1e-10, 1e-10, "WLS partition sum == weighted_ss");
    assert_close(lof_curv.f_stat, 18_155.668_852, 1e-2, 1e-5, "WLS curved F stat");
    assert!(lof_curv.significant_lack_of_fit && lof_curv.p_value < 1e-12);

    // 2) True straight line under 1/Y^2 weighting: exact orthogonal partition with F ~ 0, non-significant.
    let rep_ys_lin = [14.5, 15.0, 15.5, 24.0, 25.0, 26.0, 43.0, 45.0, 47.0, 81.0, 85.0, 89.0];
    let ds_lin = vec![make_series(0, "WlsLinear", &rep_xs, &rep_ys_lin)];
    let fit_lin = fit_nonlinear(line.as_ref(), &ds_lin, &hooks, &cfg_wls_lof).unwrap();
    let lof_lin = fit_lin.lack_of_fit.expect("WLS linear data must emit lack_of_fit");
    assert_close(lof_lin.ss_pure_error, 0.013_810_289_595, 1e-9, 1e-6, "WLS linear ss_pe_w");
    assert_close(lof_lin.ss_lack_of_fit, 8.549_263_765_673e-9, 1e-11, 1e-4, "WLS linear ss_lof_w");
    assert_close(lof_lin.ss_pure_error + lof_lin.ss_lack_of_fit, fit_lin.weighted_ss, 1e-12, 1e-12, "WLS linear partition");
    assert!(!lof_lin.significant_lack_of_fit);

    // 3) Pre-summarized Mean/SD/N (unweighted) with multiple rows at identical X must match RawReplicates identically!
    let raw_groups = [(1.0, vec![2.0, 4.0, 6.0]), (1.0, vec![3.0, 5.0]), (2.0, vec![8.0, 9.0, 10.0]), (4.0, vec![16.0, 18.0, 20.0])];
    let mut raw_pts = Vec::new();
    let mut sum_pts = Vec::new();
    for (row_idx, (x, ys)) in raw_groups.iter().enumerate() {
        for &y in ys {
            raw_pts.push(DataPoint { row: raw_pts.len(), ..DataPoint::xy(*x, y) });
        }
        let n = ys.len() as f64;
        let mean = ys.iter().sum::<f64>() / n;
        let sd = (ys.iter().map(|y| (y - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
        sum_pts.push(DataPoint { x: vec![*x], y: mean, sd: Some(sd), n: Some(n), explicit_weight: None, data_set_index: 0, row: row_idx });
    }
    let ds_raw = vec![DatasetSeries {
        data_set_index: 0,
        label: "Raw".to_owned(),
        points: raw_pts,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }];
    let ds_sum = vec![DatasetSeries {
        data_set_index: 0,
        label: "Sum".to_owned(),
        points: sum_pts,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: true,
        column_title_value: None,
    }];
    let cfg_unw_lof = SolverConfig { compute_lack_of_fit: true, compute_profile_ci: false, ..SolverConfig::default() };
    let fit_raw = fit_nonlinear(line.as_ref(), &ds_raw, &hooks, &cfg_unw_lof).unwrap();
    let fit_sum = fit_nonlinear(line.as_ref(), &ds_sum, &hooks, &cfg_unw_lof).unwrap();
    let lof_raw = fit_raw.lack_of_fit.expect("raw LOF");
    let lof_sum = fit_sum.lack_of_fit.expect("summarized multi-row LOF");
    assert_eq!(lof_sum.df_pure_error, lof_raw.df_pure_error);
    assert_eq!(lof_sum.df_lack_of_fit, lof_raw.df_lack_of_fit);
    assert_close(lof_sum.ss_pure_error, lof_raw.ss_pure_error, 1e-10, 1e-10, "summarized vs raw ss_pe");
    assert_close(lof_sum.ss_lack_of_fit, lof_raw.ss_lack_of_fit, 1e-10, 1e-10, "summarized vs raw ss_lof");
    assert_close(lof_sum.f_stat, lof_raw.f_stat, 1e-10, 1e-10, "summarized vs raw F");
    assert_close(lof_sum.p_value, lof_raw.p_value, 1e-10, 1e-10, "summarized vs raw p");

    // 4) Pre-summarized Mean/SD/N with ObservedSd (1/SD^2) weighting (1 row per distinct X):
    // Solver fits 4 row means (df = 2), so lack-of-fit must NOT subtract unweighted (n-1)*sd^2 or sum(n-1) df_pe!
    let sum_unique_pts: Vec<DataPoint> = [(1.0, 4.0, 1.5, 3.0), (2.0, 9.0, 1.0, 3.0), (3.0, 14.0, 1.2, 3.0), (4.0, 19.0, 1.8, 3.0)]
        .into_iter()
        .enumerate()
        .map(|(r, (x, y, sd, n))| DataPoint { x: vec![x], y, sd: Some(sd), n: Some(n), explicit_weight: None, data_set_index: 0, row: r })
        .collect();
    let ds_obs_sd = vec![DatasetSeries {
        data_set_index: 0,
        label: "ObsSd".to_owned(),
        points: sum_unique_pts,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: true,
        column_title_value: None,
    }];
    let cfg_obs_sd =
        SolverConfig { weighting: WeightingScheme::ObservedSd, compute_lack_of_fit: true, compute_profile_ci: false, ..SolverConfig::default() };
    let fit_obs_sd = fit_nonlinear(line.as_ref(), &ds_obs_sd, &hooks, &cfg_obs_sd).unwrap();
    assert!(fit_obs_sd.lack_of_fit.is_none(), "single summarized row per X under 1/SD^2 has no row-level replicates (df_pe=0)");
}

#[test]
fn r46_4_rout_motulsky_brown_2006_top_30_percent_bh_reproducer_and_oracle() {
    // Motulsky & Brown (2006, BMC Bioinformatics 7:123): only the top 30% of residuals furthest
    // from the fitted curve (pos >= floor(0.70 * N), i.e., ceil(0.30 * N) points) are tested in the BH FDR step.
    // Reproducer: N = 100 points along Y = 10 + 2 * X (k_free = 2, df = 98), Q = 3.6%:
    // - 69 bulk points: symmetric +-0.50 around the line (so P68.27 = 0.50, RSDR = 0.50 * 100 / 98 = 0.510204).
    // - 31 points (31% > 30%, symmetrically mirrored so Cauchy fit keeps slope=2, intercept=10): +-1.320
    //   (|r| / RSDR = 2.5872, two-sided p(df=98) = 0.011144).
    // For the top 30% (j = 1..=30): max BH cutoff is 0.036 * 30 / 100 = 0.010800 < 0.011144 -> 0 rejections!
    // Before R4.6 fix: testing all N points checks j = 31 (31%!) with cutoff 0.036 * 31 / 100 = 0.011160 > 0.011144
    // and excludes 31 points (31% > 30%)!
    let mut xs = Vec::with_capacity(100);
    let mut ys = Vec::with_capacity(100);
    let mut is_elevated = [false; 100];
    for k in 0..15 {
        is_elevated[k] = true;
        is_elevated[99 - k] = true;
    }
    is_elevated[49] = true; // 31st elevated point (middle of X range)
    for i in 0..100 {
        let x = (i as f64) - 49.5;
        let eps = if is_elevated[i] {
            if i == 49 {
                1.320
            } else if i < 50 {
                if i % 2 == 0 { 1.320 } else { -1.320 }
            } else if (99 - i) % 2 == 0 {
                -1.320
            } else {
                1.320
            }
        } else if i % 2 == 0 {
            0.50
        } else {
            -0.50
        };
        xs.push(x);
        ys.push(10.0 + 2.0 * x + eps);
    }
    let ds = vec![make_series(0, "Rout31pct", &xs, &ys)];
    let line = get_builtin_model("straight_line").unwrap();
    let cfg = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 3.6 },
        compute_profile_ci: false,
        compute_bands: false,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(line.as_ref(), &ds, &HookContext::default(), &cfg).unwrap();
    let rout = fit.rout.as_ref().expect("ROUT metadata");
    assert_eq!(
        fit.n_outliers_excluded, 0,
        "Motulsky & Brown (2006) top-30% BH scope must exclude 0 points here (got {}, outliers={:?})",
        fit.n_outliers_excluded, rout.outlier_indices
    );
    assert!(rout.audit.iter().all(|a| !a.is_outlier));

    // Also verify Motulsky & Brown (2006) detection when <= 30% ARE true extreme outliers:
    // Plant 3 huge spikes (+50, -50, +60) inside top 30% alongside 28 moderate (+-1.320) shifts.
    // Only the 3 genuine extreme spikes may be excluded (not the 31% cascade!).
    let mut ys_mixed = ys;
    ys_mixed[0] += 50.0;
    ys_mixed[1] -= 50.0;
    ys_mixed[99] += 60.0;
    let ds_mixed = vec![make_series(0, "RoutMixed", &xs, &ys_mixed)];
    let fit_mixed = fit_nonlinear(line.as_ref(), &ds_mixed, &HookContext::default(), &cfg).unwrap();
    let rout_mixed = fit_mixed.rout.as_ref().expect("ROUT mixed metadata");
    assert_eq!(fit_mixed.n_outliers_excluded, 3);
    assert_eq!(rout_mixed.outlier_indices, vec![0, 1, 99]);
    // Untested bottom 70% points (70 points) must carry q_value == 1.0 and is_outlier == false.
    let n_untested = rout_mixed.audit.iter().filter(|a| (a.q_value - 1.0).abs() < 1e-12 && !a.is_outlier).count();
    assert!(n_untested >= 70, "at least bottom 70% (70/100) must remain untested with q_value=1.0, got {n_untested}");
}

#[test]
fn r46_6_inverse_interpolation_calibration_vs_mean_response_and_deming_note() {
    // 1) Linear ResultSheet (`linear_interpolated`) must expose BOTH "Mean-Response CI" and
    //    "Calibration / Prediction Interval" matching Seber & Lee (2003) / NIST §4.5.1.1!
    let cal_x = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let cal_y = [3.1, 4.8, 7.3, 8.7, 11.4, 12.6, 15.2, 16.9];
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, cal_x.len() + 2).unwrap();
    for (r, (&x, &y)) in cal_x.iter().zip(cal_y.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    // Row 8: blank X, Y = 10.0 (X from Y). Row 9: X = 4.5, blank Y (Y from X).
    xy.data_sets[0].cells[8][0] = Cell::number(10.0);
    xy.set_x_cell(9, Cell::number(4.5)).unwrap();
    let sheet = DataSheet::new("cal-sheet", "Calibration", TableArchetype::Xy(xy)).unwrap();

    let lin_spec = AnalysisSpec::new(
        "lin-cal",
        "cal-sheet",
        AnalysisKind::LinearRegression(LinearRegressionSpec { interpolate_unknowns: true, ..LinearRegressionSpec::default() }),
    )
    .unwrap();
    let lin_res = execute_analysis(&lin_spec, &sheet, &HookContext::default()).unwrap();
    let lin_tab = lin_res.tabs.iter().find(|t| t.id == "linear_interpolated").expect("linear_interpolated tab");
    assert_eq!(lin_tab.column_headers, vec!["Input", "Interpolated", "Mean-Response CI", "Calibration / Prediction Interval"]);

    let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lin_ci_lo), upper: CiBound::Finite(lin_ci_hi) } = lin_tab.rows[0][2] else {
        panic!("expected finite Mean-Response CI in linear_interpolated row 0");
    };
    let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lin_pi_lo), upper: CiBound::Finite(lin_pi_hi) } = lin_tab.rows[0][3] else {
        panic!("expected finite Calibration / Prediction Interval in linear_interpolated row 0");
    };
    assert_close(lin_ci_lo, 4.362_750_432_716_67, 1e-10, 1e-10, "linear mean CI lower");
    assert_close(lin_ci_hi, 4.637_249_567_283_33, 1e-10, 1e-10, "linear mean CI upper");
    assert_close(lin_pi_lo, 4.088_251_298_150_02, 1e-10, 1e-10, "linear calibration PI lower");
    assert_close(lin_pi_hi, 4.911_748_701_849_98, 1e-10, 1e-10, "linear calibration PI upper");
    assert!(lin_pi_lo < lin_ci_lo && lin_ci_hi < lin_pi_hi, "calibration interval must be strictly wider than mean-response CI");

    // 2) Nonlinear `straight_line` ResultSheet (`nonlinear_interpolated`) on the exact same sheet must
    //    produce the exact same calibration interval by forward prediction-band duality!
    let mut nl_lin = NonlinearFitSpec::new("straight_line");
    nl_lin.interpolate_unknowns = true;
    nl_lin.profile_likelihood_ci = false;
    let nl_spec = AnalysisSpec::new("nl-cal", "cal-sheet", AnalysisKind::NonlinearRegression(nl_lin)).unwrap();
    let nl_res = execute_analysis(&nl_spec, &sheet, &HookContext::default()).unwrap();
    let nl_tab = nl_res.tabs.iter().find(|t| t.id == "nonlinear_interpolated").expect("nonlinear_interpolated tab");
    assert_eq!(nl_tab.column_headers, vec!["Input", "Interpolated", "Mean-Response CI", "Calibration / Prediction Interval", "Direction", "Dataset"]);
    let ResultCell::ConfidenceInterval { lower: CiBound::Finite(nl_ci_lo), upper: CiBound::Finite(nl_ci_hi) } = nl_tab.rows[0][2] else {
        panic!("expected finite nonlinear Mean-Response CI");
    };
    let ResultCell::ConfidenceInterval { lower: CiBound::Finite(nl_pi_lo), upper: CiBound::Finite(nl_pi_hi) } = nl_tab.rows[0][3] else {
        panic!("expected finite nonlinear Calibration / Prediction Interval");
    };
    assert_close(nl_ci_lo, lin_ci_lo, 1e-8, 1e-8, "nonlinear straight_line matches linear mean CI lower");
    assert_close(nl_ci_hi, lin_ci_hi, 1e-8, 1e-8, "nonlinear straight_line matches linear mean CI upper");
    assert_close(nl_pi_lo, lin_pi_lo, 1e-8, 1e-8, "nonlinear straight_line matches linear calibration PI lower");
    assert_close(nl_pi_hi, lin_pi_hi, 1e-8, 1e-8, "nonlinear straight_line matches linear calibration PI upper");

    // 3) Deming summary sheet cleanly documents hardcoded orthogonal lambda = 1.0 (LinearRegressionSpec lacks lambda).
    let dem_spec = AnalysisSpec::new(
        "dem-cal",
        "cal-sheet",
        AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() }),
    )
    .unwrap();
    let dem_res = execute_analysis(&dem_spec, &sheet, &HookContext::default()).unwrap();
    let dem_tab = dem_res.tabs.iter().find(|t| t.id == "deming_summary").expect("deming_summary tab");
    assert!(dem_tab.footnotes.iter().any(|f| f.contains("1.0") && f.contains("lambda")));
    // 4) Weighted (1/Y^2) forward-inverse calibration duality check via `fit_nonlinear` + `pred_bounds_option`.
    let line_model = get_builtin_model("straight_line").unwrap();
    let w_xs = [1.0, 2.0, 4.0, 8.0, 16.0];
    let w_ys = [5.1, 9.8, 20.5, 39.2, 81.4];
    let ds_w = vec![DatasetSeries {
        data_set_index: 0,
        label: "WlsCal".to_owned(),
        points: w_xs.iter().zip(w_ys.iter()).enumerate().map(|(r, (&x, &y))| DataPoint { row: r, ..DataPoint::xy(x, y) }).collect(),
        unknowns_y: vec![(5, 30.0)],
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }];
    let cfg_w = SolverConfig {
        weighting: WeightingScheme::RelativeSquare,
        interpolate_unknowns: true,
        compute_profile_ci: false,
        compute_bands: true,
        compute_prediction_bands: true,
        ..SolverConfig::default()
    };
    let fit_w = fit_nonlinear(line_model.as_ref(), &ds_w, &HookContext::default(), &cfg_w).unwrap();
    let unk_w = &fit_w.interpolated_x[0];
    let (CiBound::Finite(ci_lo_w), CiBound::Finite(ci_hi_w)) = unk_w.ci_bounds_option().expect("mean-response CI bounds") else {
        panic!("expected finite WLS mean CI bounds");
    };
    let (CiBound::Finite(pi_lo_w), CiBound::Finite(pi_hi_w)) = unk_w.pred_bounds_option().expect("calibration PI bounds") else {
        panic!("expected finite WLS calibration PI bounds");
    };
    assert!(pi_lo_w < ci_lo_w && ci_lo_w < unk_w.output_estimate && unk_w.output_estimate < ci_hi_w && ci_hi_w < pi_hi_w);
    // Re-evaluate forward prediction band at the inverse calibration endpoints `pi_lo_w` and `pi_hi_w`:
    let ds_fwd_check = vec![DatasetSeries { unknowns_y: Vec::new(), unknowns_x: vec![(0, pi_lo_w), (1, pi_hi_w)], ..ds_w[0].clone() }];
    let fit_fwd = fit_nonlinear(line_model.as_ref(), &ds_fwd_check, &HookContext::default(), &cfg_w).unwrap();
    let (_, CiBound::Finite(fwd_hi_at_lo)) = fit_fwd.interpolated_y[0].pred_bounds_option().unwrap() else {
        panic!("expected finite forward upper prediction bound");
    };
    let (CiBound::Finite(fwd_lo_at_hi), _) = fit_fwd.interpolated_y[1].pred_bounds_option().unwrap() else {
        panic!("expected finite forward lower prediction bound");
    };
    assert_close(fwd_hi_at_lo, 30.0, 1e-6, 1e-6, "WLS calibration lower duality: Y_pred_hi(X_pi_lo) == Y_target");
    assert_close(fwd_lo_at_hi, 30.0, 1e-6, 1e-6, "WLS calibration upper duality: Y_pred_lo(X_pi_hi) == Y_target");
}

#[test]
fn r46_7_centered_poly_default_parameter_rules_wiring_oracle() {
    // Draper & Smith (1998): skewed predictors (mean != median) across centered_poly_1..=6.
    // With SolverConfig::default() (no explicit rules), ParameterLayout::build must apply
    // ParametricModel::default_parameter_rules -> XCenter = Constant(mean_x), giving non-singular
    // full-rank Jacobian, Converged status, and finite SEs.
    let skew_xs = [100.0, 100.5, 101.0, 101.5, 102.0, 102.6, 103.3, 104.1, 105.0, 106.0, 107.2, 108.6];
    let mean_x = skew_xs.iter().sum::<f64>() / (skew_xs.len() as f64);
    let coeffs = [12.5, -1.8, 0.25, -0.02, 0.005, -0.0005, 0.0001];

    for deg in 1..=6 {
        let id = format!("centered_poly_{deg}");
        let model = get_builtin_model(&id).unwrap();
        let ys: Vec<f64> = skew_xs.iter().map(|&x| (0..=deg).fold(0.0, |acc, k| acc + coeffs[k] * (x - mean_x).powi(k as i32))).collect();
        let ds = vec![make_series(0, &id, &skew_xs, &ys)];

        let fit = fit_nonlinear(model.as_ref(), &ds, &HookContext::default(), &SolverConfig::default()).unwrap();
        assert_eq!(fit.status, FitStatus::Converged, "{id} must converge with default_parameter_rules");
        assert_eq!(fit.k_free, deg + 1, "{id} k_free");
        assert_eq!(fit.covariance.numerical_rank, deg + 1, "{id} Jacobian must be full rank");
        assert!(!fit.covariance.ambiguous, "{id} must not be ambiguous");
        assert!(fit.parameters.iter().all(|p| p.se.is_finite()), "{id} must yield finite SEs");
        assert!(
            fit.fixed_constants.iter().any(|(name, val, _)| name == "XCenter" && (*val - mean_x).abs() < 1e-10),
            "{id} must record fixed XCenter == sample mean {mean_x}"
        );
        for (k, &expected) in coeffs.iter().enumerate().take(deg + 1) {
            assert_close(find_param(&fit, &format!("B{k}")), expected, 1e-5, 1e-5, &format!("{id} B{k}"));
        }
    }

    // Explicit user override (e.g. Constant(120.0) or Local free) MUST take precedence over default_parameter_rules.
    let poly2 = get_builtin_model("centered_poly_2").unwrap();
    let ys_custom: Vec<f64> = skew_xs.iter().map(|&x| 4.0 + 1.5 * (x - 120.0) - 0.05 * (x - 120.0).powi(2)).collect();
    let ds_custom = vec![make_series(0, "CustomXCenter", &skew_xs, &ys_custom)];

    let cfg_const = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: "XCenter".to_owned(),
            scope: ParameterScope::Constant(120.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let fit_const = fit_nonlinear(poly2.as_ref(), &ds_custom, &HookContext::default(), &cfg_const).unwrap();
    assert_eq!(fit_const.status, FitStatus::Converged);
    assert!(fit_const.fixed_constants.iter().any(|(n, v, _)| n == "XCenter" && (*v - 120.0).abs() < 1e-12));
    assert_close(find_param(&fit_const, "B0"), 4.0, 1e-6, 1e-6, "override B0");

    let cfg_local = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: "XCenter".to_owned(),
            scope: ParameterScope::Local,
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let fit_local = fit_nonlinear(poly2.as_ref(), &ds_custom, &HookContext::default(), &cfg_local).unwrap();
    assert_eq!(fit_local.status, FitStatus::Ambiguous, "explicit Local override on XCenter must expose collinearity");
    assert_eq!(fit_local.k_free, 4);
    assert!(fit_local.covariance.numerical_rank < 4);
}

#[test]
fn r46_8_more_newton_trust_region_lm_convergence_oracles() {
    // Multi-scale badly conditioned saturation/binding problem spanning 8 orders of magnitude
    // (Bmax ~ 2.5e5, Kd ~ 4.0e-3) + box constraints, verifying Moré (1978) / MINPACK-1 Newton
    // trust-region solver converges cleanly from far initial estimates without simplex rescue.
    let model = get_builtin_model("one_site_specific_binding").unwrap();
    let xs = [1e-4, 5e-4, 1e-3, 2e-3, 4e-3, 8e-3, 2e-2, 5e-2, 0.1, 0.5];
    let (true_bmax, true_kd) = (250_000.0_f64, 0.004_f64);
    let ys: Vec<f64> = xs.iter().map(|&x| true_bmax * x / (true_kd + x)).collect();
    let ds = vec![make_series(0, "MultiScale", &xs, &ys)];

    let cfg = SolverConfig {
        use_simplex_warm_start: false,
        parameter_rules: vec![
            ParameterRule {
                name: "Bmax".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(10_000.0),
                lower_bound: Some(0.0),
                upper_bound: None,
            },
            ParameterRule {
                name: "Kd".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(0.2),
                lower_bound: Some(1e-9),
                upper_bound: None,
            },
        ],
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(model.as_ref(), &ds, &HookContext::default(), &cfg).unwrap();
    assert_eq!(fit.status, FitStatus::Converged);
    assert_close(find_param(&fit, "Bmax"), true_bmax, 1e-4, 1e-8, "Moré LM multi-scale Bmax");
    assert_close(find_param(&fit, "Kd"), true_kd, 1e-10, 1e-8, "Moré LM multi-scale Kd");
    assert!(fit.rss < 1e-8, "expected near-zero RSS, got {}", fit.rss);

    // Non-linearly constrained minimum hitting lower bound cleanly under Moré trust-region QR.
    let line = get_builtin_model("straight_line").unwrap();
    let l_xs = [0.0, 1.0, 2.0, 3.0, 4.0];
    let l_ys = [1.0, 3.0, 5.0, 7.0, 9.0];
    let l_ds = vec![make_series(0, "Constrained", &l_xs, &l_ys)];
    let cfg_bound = SolverConfig {
        use_simplex_warm_start: false,
        parameter_rules: vec![ParameterRule {
            name: "YIntercept".to_owned(),
            scope: ParameterScope::Local,
            initial_override: Some(5.0),
            lower_bound: Some(2.0),
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let fit_b = fit_nonlinear(line.as_ref(), &l_ds, &HookContext::default(), &cfg_bound).unwrap();
    assert_eq!(fit_b.status, FitStatus::Converged);
    // Min sum_{x=0..4} (2 + b*x - (1 + 2*x))^2 = sum (1 + (b-2)*x)^2 => b = 2 - sum(x)/sum(x^2) = 2 - 10/30 = 5/3.
    assert_close(find_param(&fit_b, "YIntercept"), 2.0, 1e-10, 1e-10, "bounded intercept");
    assert_close(find_param(&fit_b, "Slope"), 5.0 / 3.0, 1e-10, 1e-10, "constrained slope = 5/3");
}
