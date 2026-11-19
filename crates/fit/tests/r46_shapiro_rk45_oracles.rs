//! Regression reproducers and D1 primary-literature oracle tests for `R4.6` fixes in
//! [`curvebook_fit::diagnostics`] and [`curvebook_fit::dsl`]:
//! - Shapiro–Wilk `n = 4..5` Royston (1992/1995) AS R94 polynomial weights (and `n = 3..5000` oracles)
//! - Dormand–Prince RK45 rejected-step shrink exponent `-1/5` (Hairer, Nørsett & Wanner 1993, §II.4)
//! - ROUT Motulsky & Brown (2006) top-30% furthest-point FDR scope in `rout_detect_outliers`
//! - Replicates lack-of-fit same-`(data_set_index, x)` grouping across pre-summarized and mixed rows
//!
//! # References
//! - Shapiro, S. S. & Wilk, M. B. (1965). An analysis of variance test for normality (complete samples).
//!   *Biometrika* 52(3/4), 591–611.
//! - Royston, P. (1992). Approximating the Shapiro–Wilk W-test for non-normality.
//!   *Statistics and Computing* 2(3), 117–119.
//! - Royston, P. (1995). Remark AS R94: A remark on algorithm AS 181: The W-test for normality.
//!   *Applied Statistics* 44(4), 547–551.
//! - Dormand, J. R. & Prince, P. J. (1980). A family of embedded Runge–Kutta formulae.
//!   *J. Comput. Appl. Math.* 6(1), 19–26.
//! - Hairer, E., Nørsett, S. P. & Wanner, G. (1993). *Solving Ordinary Differential Equations I:
//!   Nonstiff Problems*, 2nd ed., §II.4 (eq. 4.13). Springer.
//! - Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear
//!   regression – a new method based on robust nonlinear regression and the false discovery rate.
//!   *BMC Bioinformatics* 7, 123.
//! - Benjamini, Y. & Hochberg, Y. (1995). Controlling the false discovery rate. *JRSS-B* 57(1), 289–300.
//! - Draper, N. R. & Smith, H. (1998). *Applied Regression Analysis*, 3rd ed., §1.5. Wiley.

use curvebook_fit::diagnostics::{replicates_lack_of_fit, rout_detect_outliers, shapiro_wilk_test};
use curvebook_fit::dsl::{CompiledDslModel, integrate_rk45};
use curvebook_fit::{
    DataPoint, DatasetSeries, FitError, HookContext, ParametricModel, SimpleLinearOptions, SolverConfig, fit_nonlinear, simple_linear_regression,
};
use curvebook_math::dist::student_t_two_sided_p;

fn assert_close(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, label: &str) {
    assert!(actual.is_finite(), "{label}: expected finite near {expected}, got {actual}");
    let diff = (actual - expected).abs();
    let bound = abs_tol.max(rel_tol * expected.abs());
    assert!(diff <= bound, "{label}: actual={actual:.16e}, expected={expected:.16e}, diff={diff:.3e} > bound={bound:.3e}");
}

#[test]
fn r46_5_shapiro_wilk_n3_n4_n5_royston_as_r94_reproducer_and_oracles() {
    // D1 Provenance: Shapiro & Wilk (1965) / Royston (1992, 1995 AS R94); independent black-box
    // cross-check via R 4.5.2 `stats::shapiro.test` on synthetic small samples (2026-11-16).
    // Before R4.6 fix: `n = 4..5` in `diagnostics.rs` used normalized Blom scores instead of
    // Royston AS R94 `a_n` polynomial correction (e.g. giving W = 0.6687 vs 0.6876 on [1,2,3,4,20]).
    let oracles: &[(&[f64], f64, f64, &str)] = &[
        // n = 3 (exact Shapiro-Wilk arcsine p-value)
        (&[1.0, 2.0, 3.0], 1.0, 1.0, "n=3 symmetric [1,2,3]"),
        (&[1.0, 1.0, 2.0], 0.75, 0.0, "n=3 extreme [1,1,2]"),
        (&[1.0, 1.5, 10.0], 0.791_530_944_625_407_1, 0.094_436_484_202_600_3, "n=3 skewed [1,1.5,10]"),
        // n = 4 (Royston 1992/1995 AS R94)
        (&[1.0, 2.0, 3.0, 4.0], 0.992_912_006_800_619_5, 0.971_877_057_620_898_6, "n=4 uniform [1,2,3,4]"),
        (&[1.0, 2.0, 3.0, 10.0], 0.806_885_664_567_700_2, 0.115_152_981_754_536_6, "n=4 skewed [1,2,3,10]"),
        (&[1.0, 1.0, 1.0, 10.0], 0.629_776_264_913_707_5, 0.001_240_725_931_977_3, "n=4 extreme outlier [1,1,1,10]"),
        (&[2.3, 2.9, 3.1, 3.8], 0.986_879_654_506_802_3, 0.940_991_197_620_188_6, "n=4 normal-like [2.3,2.9,3.1,3.8]"),
        // n = 5 (Royston 1992/1995 AS R94)
        (&[1.0, 2.0, 3.0, 4.0, 5.0], 0.986_762_155_447_719_4, 0.967_173_935_968_039_8, "n=5 uniform [1,2,3,4,5]"),
        (&[1.0, 2.0, 3.0, 4.0, 20.0], 0.687_579_225_298_494, 0.007_068_598_540_515_0, "n=5 skewed [1,2,3,4,20]"),
        (&[1.2, 2.5, 2.7, 3.1, 4.9], 0.951_279_243_274_782_6, 0.746_324_237_460_133_3, "n=5 sample [1.2,2.5,2.7,3.1,4.9]"),
        // n = 6 (both a_n and a_{n-1} polynomials)
        (&[1.0, 2.0, 3.0, 4.0, 5.0, 15.0], 0.758_802_835_602_816_8, 0.024_258_247_776_970_1, "n=6 skewed [1..5,15]"),
    ];

    for &(sample, exp_w, exp_p, label) in oracles {
        let res = shapiro_wilk_test(sample).unwrap_or_else(|| panic!("{label}: returned None"));
        assert_close(res.statistic, exp_w, 1e-11, 1e-11, &format!("{label} W"));
        assert_close(res.p_value, exp_p, 1e-9, 1e-9, &format!("{label} p"));
        assert_eq!(res.passed, res.p_value >= 0.05, "{label} passed flag");

        // Permutation & positive-affine invariance (shift by 1e6, scale by 1e-3).
        let mut permuted_affine: Vec<f64> = sample.iter().rev().map(|&v| 1.0e6 + 1.0e-3 * v).collect();
        let res_aff = shapiro_wilk_test(&permuted_affine).unwrap_or_else(|| panic!("{label} affine: returned None"));
        assert_close(res_aff.statistic, exp_w, 1e-6, 1e-6, &format!("{label} affine W"));
        if exp_p > 1e-6 {
            assert_close(res_aff.p_value, exp_p, 1e-4, 1e-4, &format!("{label} affine p"));
        }
        permuted_affine.clear();
    }

    // Domain & degenerate guards.
    assert!(shapiro_wilk_test(&[1.0, 2.0]).is_none(), "n < 3 must return None");
    assert!(shapiro_wilk_test(&[5.0; 4]).is_none(), "zero variance must return None");
    assert!(shapiro_wilk_test(&[1.0, 2.0, f64::NAN, 4.0]).is_none(), "NaN must return None");
    assert!(shapiro_wilk_test(&[1.0, 2.0, 3.0, f64::INFINITY]).is_none(), "Inf must return None");

    // End-to-end nonlinear fit with n = 5 residuals (`compute_normality = true`).
    let model = CompiledDslModel::compile("line", "Y = a + b * X").unwrap();
    let pts = vec![DataPoint::xy(0.0, 1.0), DataPoint::xy(1.0, 2.0), DataPoint::xy(2.0, 3.0), DataPoint::xy(3.0, 4.0), DataPoint::xy(4.0, 20.0)];
    let ds = DatasetSeries {
        data_set_index: 0,
        label: "N5".to_owned(),
        points: pts,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    };
    let cfg = SolverConfig { compute_normality: true, compute_profile_ci: false, ..SolverConfig::default() };
    let fit = fit_nonlinear(&model, &[ds], &HookContext::default(), &cfg).unwrap();
    let fit_sw = fit.normality.as_ref().expect("n=5 residuals must emit Shapiro-Wilk test");
    let direct_sw = shapiro_wilk_test(&fit.datasets[0].residuals).unwrap();
    assert_close(fit_sw.statistic, direct_sw.statistic, 1e-14, 1e-14, "solver n=5 SW W parity");
    assert_close(fit_sw.p_value, direct_sw.p_value, 1e-14, 1e-14, "solver n=5 SW p parity");
}

#[test]
fn r46_8_rk45_rejected_step_shrink_exponent_and_ode_accuracy_oracles() {
    // Dormand & Prince (1980) 5(4) pair with Hairer, Nørsett & Wanner (1993) §II.4 eq. (4.13):
    // error-per-step controller scales rejected steps with exponent -1/(q+1) = -1/5 = -0.20
    // (same exponent as accepted steps), NOT -1/4 = -0.25.
    // Dormand-Prince error weights `E = b - b*`:
    const E1: f64 = 71.0 / 57600.0;
    const E3: f64 = -71.0 / 16695.0;
    const E4: f64 = 71.0 / 1920.0;
    const E5: f64 = -17253.0 / 339200.0;
    const E6: f64 = 22.0 / 525.0;
    const E7: f64 = -1.0 / 40.0;
    const B1: f64 = 35.0 / 384.0;
    const B3: f64 = 500.0 / 1113.0;
    const B4: f64 = 125.0 / 192.0;
    const B5: f64 = -2187.0 / 6784.0;
    const B6: f64 = 11.0 / 84.0;

    for &lam in &[5.0_f64, 6.0_f64] {
        let mut stage_ts = Vec::new();
        let mut stage_ks = Vec::new();
        let y_end = integrate_rk45(0.0, 1.0, 1.0, |t, y| {
            let k = -lam * y;
            stage_ts.push(t);
            stage_ks.push(k);
            Some(k)
        })
        .expect("RK45 decay converged");
        assert_close(y_end, (-lam).exp(), 1e-8, 1e-7, &format!("RK45 exp(-{lam})"));

        // Step 0 tries t=0, h0 = 0.05 (7 stages: k1..k6 + k7 at t=h0).
        // Because ratio > 1, step 0 is rejected and step 1 retries at t=0 with k2 at t = 0.2 * h1.
        assert!(stage_ts.len() >= 14, "expected at least one rejected step + retry");
        assert_close(stage_ts[0], 0.0, 1e-15, 0.0, "step 0 k1 t");
        assert_close(stage_ts[7], 0.0, 1e-15, 0.0, "rejected retry k1 t == 0");

        let h0 = 0.05_f64;
        assert_close(stage_ts[1], 0.2 * h0, 1e-15, 1e-15, "step 0 k2 t == 0.2 * h0");
        let h1_observed = 5.0 * stage_ts[8];

        let (k1, k3, k4, k5, k6, k7) = (stage_ks[0], stage_ks[2], stage_ks[3], stage_ks[4], stage_ks[5], stage_ks[6]);
        let y_next = 1.0 + h0 * (B1 * k1 + B3 * k3 + B4 * k4 + B5 * k5 + B6 * k6);
        let err = (h0 * (E1 * k1 + E3 * k3 + E4 * k4 + E5 * k5 + E6 * k6 + E7 * k7)).abs();
        let tol = 1e-10 + 1e-8 * 1.0_f64.max(y_next.abs());
        let ratio = err / tol;
        assert!(ratio > 10.0, "test fixture requires rejection with unclamped shrink factor, ratio={ratio}");

        let expected_factor = (0.9 * ratio.powf(-0.2)).clamp(0.1, 0.5);
        assert!(expected_factor > 0.105 && expected_factor < 0.495, "factor={expected_factor} must lie strictly inside clamp bounds");
        let observed_factor = h1_observed / h0;
        assert_close(
            observed_factor,
            expected_factor,
            1e-12,
            1e-12,
            &format!("RK45 rejected-step shrink factor for lam={lam} must use -1/5 (-0.20) exponent"),
        );
    }

    // Clamp lower bound check on sharp initial slope (dy/dt = -20*y => factor clamped to 0.1).
    let mut ts_clamped = Vec::new();
    let y_stiff = integrate_rk45(0.0, 1.0, 1.0, |t, y| {
        ts_clamped.push(t);
        Some(-20.0 * y)
    })
    .expect("RK45 exp(-20) converged");
    assert_close(y_stiff, (-20.0_f64).exp(), 1e-9, 1e-6, "RK45 exp(-20) accuracy");
    assert_close(ts_clamped[7], 0.0, 1e-15, 0.0, "stiff retry at t=0");
    assert_close((5.0 * ts_clamped[8]) / 0.05, 0.1, 1e-12, 1e-12, "clamped rejected-step shrink factor == 0.1");

    // Hairer, Nørsett & Wanner (1993) §II.4 accuracy oracles via direct integrate_rk45 + DSL ODE:
    // 1) Nonlinear logistic ODE: dy/dt = 2*y*(1 - y/10), y(0) = 0.5 => y(t) = 10 / (1 + 19*exp(-2*t)).
    let logistic_exact = |t: f64| 10.0 / (1.0 + 19.0 * (-2.0 * t).exp());
    for &t_end in &[0.5_f64, 2.0, 5.0] {
        let y_num = integrate_rk45(0.0, t_end, 0.5, |_t, y| Some(2.0 * y * (1.0 - y / 10.0))).expect("logistic forward");
        assert_close(y_num, logistic_exact(t_end), 1e-7, 1e-7, &format!("logistic forward t={t_end}"));
    }
    let y_back_mod = integrate_rk45(2.0, 0.5, logistic_exact(2.0), |_t, y| Some(2.0 * y * (1.0 - y / 10.0))).expect("logistic backward 2 -> 0.5");
    assert_close(y_back_mod, logistic_exact(0.5), 1e-7, 1e-7, "logistic backward 2 -> 0.5");
    // Backward from t=5->0.5 amplifies local rtol=1e-8 by exp(2*4.5) ~ 8103 near y=10 equilibrium.
    let y_back_long = integrate_rk45(5.0, 0.5, logistic_exact(5.0), |_t, y| Some(2.0 * y * (1.0 - y / 10.0))).expect("logistic backward 5 -> 0.5");
    assert_close(y_back_long, logistic_exact(0.5), 1e-5, 1e-5, "logistic backward 5 -> 0.5");

    // 2) Non-autonomous oscillatory ODE via DSL: dy/dX = B*Y*cos(X), Y(0) = A => Y(X) = A*exp(B*sin(X)).
    let dsl_osc = CompiledDslModel::compile("osc_ode", "Y0 = A\ndY/dX = B * Y * cos(X)").unwrap();
    let params_osc: Vec<f64> = dsl_osc.param_names().iter().map(|n| if n == "B" { 6.0 } else { 1.0 }).collect();
    for &x in &[0.5_f64, core::f64::consts::FRAC_PI_2, core::f64::consts::PI, 2.0 * core::f64::consts::PI] {
        let y_dsl = dsl_osc.eval(&[x], &params_osc, 0).expect("DSL oscillatory ODE");
        let y_exact = (6.0 * x.sin()).exp();
        assert_close(y_dsl, y_exact, 1e-6, 1e-6, &format!("DSL oscillatory ODE x={x}"));
    }
    let y_osc_back = integrate_rk45(2.0 * core::f64::consts::PI, 0.0, 1.0, |t, y| Some(6.0 * y * t.cos())).expect("oscillatory backward");
    assert_close(y_osc_back, 1.0, 1e-6, 1e-6, "oscillatory backward 2*pi -> 0");
}

#[test]
fn r46_4_rout_detect_outliers_top_30_percent_motulsky_brown_2006() {
    // Motulsky & Brown (2006), BMC Bioinformatics 7:123 ("Step 3: Identify outliers using FDR"):
    // Only the top 30% of points furthest from the curve (ranks 1..=m_test, m_test = N - floor(0.70*N))
    // are tested by Benjamini–Hochberg step-up; the remaining ~70% are never tested (q_value = 1.0).
    //
    // Construct N = 100 points (k_free = 2, df = 98):
    // - 69 core points (indices 0..69): |r| = 0.01  (sets P68.27 = 0.01 => RSDR = 0.01 * 100/98)
    // - 1 borderline-inner point (index 69, 31st largest => outside top 30%): |r| = 1.0 (t ~ 98, p ~ 0)
    // - 30 extreme outliers (indices 70..100, exact top 30%): |r| = 10.0
    // Before R4.6 fix: applying BH across all N points flagged 31 points (including index 69).
    let mut pts = Vec::with_capacity(100);
    let mut res = Vec::with_capacity(100);
    for i in 0..100_usize {
        pts.push(DataPoint { row: i, ..DataPoint::xy(i as f64, 100.0) });
        let r = if i < 69 {
            if i % 2 == 0 { 0.01 } else { -0.01 }
        } else if i == 69 {
            1.0
        } else {
            10.0
        };
        res.push(r);
    }
    let out = rout_detect_outliers(&pts, &res, 2, 5.0).expect("ROUT Q=5% on N=100");
    assert_eq!(out.outlier_indices.len(), 30, "Motulsky & Brown (2006) can flag at most top 30% (30/100)");
    assert!(!out.audit[69].is_outlier, "31st furthest point (index 69) lies outside top 30% and must not be flagged");
    assert_close(out.audit[69].q_value, 1.0, 1e-15, 0.0, "untested 31st point has q_value = 1.0");
    for i in 70..100 {
        assert!(out.audit[i].is_outlier, "top-30% extreme outlier {i} must be flagged");
    }

    // Exact Benjamini–Hochberg q-value check on N = 20, k_free = 2 (m_test = 20 - 14 = 6 tested points):
    // Verify top-6 p-values & BH step-up q_j = min_{k >= j} min(1, N * p_(k) / k) while bottom 14 stay 1.0.
    let mut pts20 = Vec::with_capacity(20);
    let mut res20 = vec![0.1_f64; 20];
    for i in 0..20_usize {
        pts20.push(DataPoint { row: i, ..DataPoint::xy(i as f64, 50.0) });
        if i % 2 == 1 {
            res20[i] = -0.1;
        }
    }
    res20[2] = 0.14;
    res20[5] = -0.17;
    res20[9] = 0.20;
    res20[11] = -0.25;
    res20[14] = 4.0;
    res20[18] = -5.5;
    let out20 = rout_detect_outliers(&pts20, &res20, 2, 1.0).expect("ROUT Q=1% on N=20");
    assert_eq!(out20.outlier_indices, vec![14, 18]);

    let expected_p14 = student_t_two_sided_p(4.0 / out20.rsdr, 18.0).unwrap();
    let expected_p18 = student_t_two_sided_p(5.5 / out20.rsdr, 18.0).unwrap();
    let expected_p11 = student_t_two_sided_p(0.25 / out20.rsdr, 18.0).unwrap();
    assert_close(out20.audit[18].p_value, expected_p18, 1e-15, 1e-12, "p_value rank 1");
    assert_close(out20.audit[14].p_value, expected_p14, 1e-15, 1e-12, "p_value rank 2");
    assert_close(out20.audit[11].p_value, expected_p11, 1e-15, 1e-12, "p_value rank 3");
    assert_close(out20.audit[14].q_value, (expected_p14 * 20.0 / 2.0).min(1.0), 1e-15, 1e-12, "BH q_value rank 2");
    assert_close(out20.audit[11].q_value, (expected_p11 * 20.0 / 3.0).min(1.0), 1e-15, 1e-12, "BH q_value rank 3");
    for i in [0_usize, 1, 3, 4, 6, 7, 8, 10, 12, 13, 15, 16, 17, 19] {
        assert_close(out20.audit[i].q_value, 1.0, 1e-15, 0.0, &format!("untested point {i} q_value == 1.0"));
        assert!(!out20.audit[i].is_outlier);
    }

    // Invalid Q% and non-finite residuals guards.
    assert!(matches!(rout_detect_outliers(&pts20, &res20, 2, f64::NAN), Err(FitError::InvalidParameter { .. })));
    assert!(matches!(rout_detect_outliers(&pts20, &res20, 2, 120.0), Err(FitError::InvalidParameter { .. })));
    let mut bad_res = res20;
    bad_res[0] = f64::NAN;
    assert!(matches!(rout_detect_outliers(&pts20, &bad_res, 2, 1.0), Err(FitError::InvalidParameter { .. })));
}

#[test]
fn r46_3_replicates_lack_of_fit_presummarized_and_multidataset_oracles() {
    // Draper & Smith (1998) §1.5 pure-error / lack-of-fit partition across multiple
    // pre-summarized rows at identical X, mixed raw+summarized rows, and multi-dataset curves.
    //
    // Helper: construct n = 3 raw replicates whose exact mean is `y_bar` and exact sample SD is `sd`:
    // { y_bar - sd, y_bar, y_bar + sd } => css = 2 * sd^2 = (3 - 1) * sd^2!
    let expand_n3 = |x: f64, y_bar: f64, sd: f64, ds: usize| -> Vec<DataPoint> {
        [-1.0_f64, 0.0, 1.0]
            .into_iter()
            .map(|offset| DataPoint {
                x: vec![x],
                y: y_bar + offset * sd,
                sd: None,
                n: None,
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: ds,
                row: 0,
            })
            .collect()
    };

    // 1) Two pre-summarized rows at x=1.0 + one at x=2.0 => only 2 distinct X levels => 0 lack-of-fit df => None!
    // Before R4.6 fix: `diagnostics::replicates_lack_of_fit` ignored between-row pure error when
    // `has_summarized` was true, falsely emitting df_pe=6, df_lof=1 instead of None!
    let two_x_summarized = vec![
        DataPoint { x: vec![1.0], y: 10.0, sd: Some(1.0), n: Some(3.0), explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 0 },
        DataPoint { x: vec![1.0], y: 12.0, sd: Some(1.0), n: Some(3.0), explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 1 },
        DataPoint { x: vec![2.0], y: 15.0, sd: Some(1.0), n: Some(3.0), explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 2 },
    ];
    let fit_2x = simple_linear_regression(&two_x_summarized, &SimpleLinearOptions::default()).unwrap();
    assert!(fit_2x.lack_of_fit.is_none(), "2 distinct X values in 2-parameter linear regression have df_lof = 0, got {:?}", fit_2x.lack_of_fit);

    // 2) Add a 3rd X level with quadratic curvature (x=3.0): compare pre-summarized vs exact expanded raw replicates.
    let mut three_x_summarized = two_x_summarized;
    three_x_summarized.push(DataPoint {
        x: vec![3.0],
        y: 25.0,
        sd: Some(1.5),
        n: Some(3.0),
        explicit_weight: None,
        source_subcolumn: 0,
        data_set_index: 0,
        row: 3,
    });
    let mut three_x_raw = Vec::new();
    three_x_raw.extend(expand_n3(1.0, 10.0, 1.0, 0));
    three_x_raw.extend(expand_n3(1.0, 12.0, 1.0, 0));
    three_x_raw.extend(expand_n3(2.0, 15.0, 1.0, 0));
    three_x_raw.extend(expand_n3(3.0, 25.0, 1.5, 0));

    let fit_sum = simple_linear_regression(&three_x_summarized, &SimpleLinearOptions::default()).unwrap();
    let fit_raw = simple_linear_regression(&three_x_raw, &SimpleLinearOptions::default()).unwrap();
    let lof_sum = fit_sum.lack_of_fit.as_ref().expect("pre-summarized 3-X must produce LOF");
    let lof_raw = fit_raw.lack_of_fit.as_ref().expect("raw 3-X must produce LOF");

    assert_eq!(lof_sum.df_pure_error, 9);
    assert_eq!(lof_sum.df_lack_of_fit, 1);
    assert_eq!(lof_sum.df_pure_error, lof_raw.df_pure_error);
    assert_eq!(lof_sum.df_lack_of_fit, lof_raw.df_lack_of_fit);
    assert_close(lof_sum.ss_pure_error, lof_raw.ss_pure_error, 1e-12, 1e-12, "summarized vs raw SS_PE");
    assert_close(lof_sum.ss_lack_of_fit, lof_raw.ss_lack_of_fit, 1e-12, 1e-12, "summarized vs raw SS_LOF");
    assert_close(lof_sum.f_stat, lof_raw.f_stat, 1e-12, 1e-12, "summarized vs raw F");
    assert_close(lof_sum.p_value, lof_raw.p_value, 1e-12, 1e-12, "summarized vs raw p");

    // 3) Mixed raw + pre-summarized points at same X (`n=1` raw + `n=3` summarized at x=1) and
    // multi-dataset separation (`ds=0` vs `ds=1` at identical X=1.0 must NOT be pooled as replicates!).
    let mut mixed_pts = expand_n3(1.0, 10.0, 1.0, 0);
    mixed_pts.push(DataPoint {
        x: vec![1.0],
        y: 12.0,
        sd: Some(1.0),
        n: Some(3.0),
        explicit_weight: None,
        source_subcolumn: 0,
        data_set_index: 0,
        row: 3,
    });
    mixed_pts.extend(expand_n3(2.0, 15.0, 1.0, 0));
    mixed_pts.extend(expand_n3(3.0, 25.0, 1.5, 0));
    let lof_mixed = replicates_lack_of_fit(&mixed_pts, fit_raw.rss, fit_raw.df_residual).expect("mixed raw+summarized LOF");
    assert_close(lof_mixed.ss_pure_error, lof_raw.ss_pure_error, 1e-12, 1e-12, "mixed raw+summarized SS_PE");
    assert_eq!(lof_mixed.df_pure_error, lof_raw.df_pure_error);

    let two_curves_no_reps = vec![
        DataPoint { x: vec![1.0], y: 2.0, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 0 },
        DataPoint { x: vec![2.0], y: 4.0, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 1 },
        DataPoint { x: vec![1.0], y: 20.0, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 1, row: 0 },
        DataPoint { x: vec![2.0], y: 40.0, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 1, row: 1 },
    ];
    assert!(replicates_lack_of_fit(&two_curves_no_reps, 1.0, 2).is_none(), "distinct datasets at the same X are not replicates");
}
