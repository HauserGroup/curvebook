//! Regression reproducers and D1 primary-literature oracle tests for `R4.6` fixes in
//! [`curvebook_fit::diagnostics`] and [`curvebook_fit::dsl`].
//! References: Shapiro & Wilk (1965); Royston (1992, 1995 AS R94); Dormand & Prince (1980);
//! Hairer, Nørsett & Wanner (1993, §II.4); Motulsky & Brown (2006); Benjamini & Hochberg (1995);
//! Draper & Smith (1998, §1.5).

use curvebook_fit::diagnostics::{replicates_lack_of_fit, rout_detect_outliers, shapiro_wilk_test};
use curvebook_fit::dsl::integrate_rk45;
use curvebook_fit::{DataPoint, FitError, ParametricModel, SimpleLinearOptions, SolverConfig, simple_linear_regression};
use curvebook_math::dist::student_t_two_sided_p;
use curvebook_testkit::assert_approx_eq as assert_close;

use crate::common::{dsl_compile, fit_with, series_xy};

#[test]
fn r46_5_shapiro_wilk_n3_n4_n5_royston_as_r94_reproducer_and_oracles() {
    let oracles: &[(&[f64], f64, f64, &str)] = &[
        (&[1.0, 2.0, 3.0], 1.0, 1.0, "n=3 [1,2,3]"),
        (&[1.0, 1.0, 2.0], 0.75, 0.0, "n=3 [1,1,2]"),
        (&[1.0, 1.5, 10.0], 0.791_530_944_625_407_1, 0.094_436_484_202_600_3, "n=3 [1,1.5,10]"),
        (&[1.0, 2.0, 3.0, 4.0], 0.992_912_006_800_619_5, 0.971_877_057_620_898_6, "n=4 [1..4]"),
        (&[1.0, 2.0, 3.0, 10.0], 0.806_885_664_567_700_2, 0.115_152_981_754_536_6, "n=4 [1,2,3,10]"),
        (&[1.0, 1.0, 1.0, 10.0], 0.629_776_264_913_707_5, 0.001_240_725_931_977_3, "n=4 [1,1,1,10]"),
        (&[2.3, 2.9, 3.1, 3.8], 0.986_879_654_506_802_3, 0.940_991_197_620_188_6, "n=4 [2.3..3.8]"),
        (&[1.0, 2.0, 3.0, 4.0, 5.0], 0.986_762_155_447_719_4, 0.967_173_935_968_039_8, "n=5 [1..5]"),
        (&[1.0, 2.0, 3.0, 4.0, 20.0], 0.687_579_225_298_494, 0.007_068_598_540_515_0, "n=5 [1..4,20]"),
        (&[1.2, 2.5, 2.7, 3.1, 4.9], 0.951_279_243_274_782_6, 0.746_324_237_460_133_3, "n=5 [1.2..4.9]"),
        (&[1.0, 2.0, 3.0, 4.0, 5.0, 15.0], 0.758_802_835_602_816_8, 0.024_258_247_776_970_1, "n=6 [1..5,15]"),
    ];
    for &(sample, exp_w, exp_p, label) in oracles {
        let res = shapiro_wilk_test(sample).unwrap();
        assert_close(res.statistic, exp_w, 1e-11, 1e-11, &format!("{label} W"));
        assert_close(res.p_value, exp_p, 1e-9, 1e-9, &format!("{label} p"));
        assert_eq!(res.passed, res.p_value >= 0.05);

        let affine: Vec<f64> = sample.iter().rev().map(|&v| 1.0e6 + 1.0e-3 * v).collect();
        let res_aff = shapiro_wilk_test(&affine).unwrap();
        assert_close(res_aff.statistic, exp_w, 1e-6, 1e-6, &format!("{label} affine W"));
        if exp_p > 1e-6 {
            assert_close(res_aff.p_value, exp_p, 1e-4, 1e-4, &format!("{label} affine p"));
        }
    }
    for bad in [&[1.0, 2.0][..], &[5.0; 4], &[1.0, 2.0, f64::NAN, 4.0], &[1.0, 2.0, 3.0, f64::INFINITY]] {
        assert!(shapiro_wilk_test(bad).is_none());
    }

    let fit = fit_with(
        &dsl_compile("Y = a + b * X"),
        &[series_xy(0, "N5", &[0.0, 1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0, 20.0])],
        &SolverConfig { compute_normality: true, compute_profile_ci: false, ..Default::default() },
    );
    let (fit_sw, dir_sw) = (fit.normality.as_ref().unwrap(), shapiro_wilk_test(&fit.datasets[0].residuals).unwrap());
    assert_close(fit_sw.statistic, dir_sw.statistic, 1e-14, 1e-14, "n=5 SW W");
    assert_close(fit_sw.p_value, dir_sw.p_value, 1e-14, 1e-14, "n=5 SW p");
}

#[test]
fn r46_8_rk45_rejected_step_shrink_exponent_and_ode_accuracy_oracles() {
    const E: [f64; 6] = [71.0 / 57600.0, -71.0 / 16695.0, 71.0 / 1920.0, -17253.0 / 339200.0, 22.0 / 525.0, -1.0 / 40.0];
    const B: [f64; 5] = [35.0 / 384.0, 500.0 / 1113.0, 125.0 / 192.0, -2187.0 / 6784.0, 11.0 / 84.0];

    for lam in [5.0_f64, 6.0] {
        let (mut stage_ts, mut stage_ks) = (Vec::new(), Vec::new());
        let y_end = integrate_rk45(0.0, 1.0, 1.0, |t, y| {
            let k = -lam * y;
            stage_ts.push(t);
            stage_ks.push(k);
            Some(k)
        })
        .unwrap();
        assert_close(y_end, (-lam).exp(), 1e-8, 1e-7, &format!("RK45 exp(-{lam})"));
        assert!(stage_ts.len() >= 14);
        assert_close(stage_ts[0], 0.0, 1e-15, 0.0, "k1 t");
        assert_close(stage_ts[7], 0.0, 1e-15, 0.0, "retry k1 t");
        let h0 = 0.05_f64;
        assert_close(stage_ts[1], 0.2 * h0, 1e-15, 1e-15, "k2 t");

        let (k1, k3, k4, k5, k6, k7) = (stage_ks[0], stage_ks[2], stage_ks[3], stage_ks[4], stage_ks[5], stage_ks[6]);
        let y_next = 1.0 + h0 * (B[0] * k1 + B[1] * k3 + B[2] * k4 + B[3] * k5 + B[4] * k6);
        let err = (h0 * (E[0] * k1 + E[1] * k3 + E[2] * k4 + E[3] * k5 + E[4] * k6 + E[5] * k7)).abs();
        let ratio = err / (1e-10 + 1e-8 * 1.0_f64.max(y_next.abs()));
        let exp_factor = (0.9 * ratio.powf(-0.2)).clamp(0.1, 0.5);
        assert!(ratio > 10.0 && exp_factor > 0.105 && exp_factor < 0.495);
        assert_close((5.0 * stage_ts[8]) / h0, exp_factor, 1e-12, 1e-12, &format!("shrink factor lam={lam}"));
    }

    let mut ts_clamped = Vec::new();
    let y_stiff = integrate_rk45(0.0, 1.0, 1.0, |t, y| {
        ts_clamped.push(t);
        Some(-20.0 * y)
    })
    .unwrap();
    assert_close(y_stiff, (-20.0_f64).exp(), 1e-9, 1e-6, "exp(-20)");
    assert_close(ts_clamped[7], 0.0, 1e-15, 0.0, "stiff retry t");
    assert_close((5.0 * ts_clamped[8]) / 0.05, 0.1, 1e-12, 1e-12, "clamped shrink == 0.1");

    let logistic_exact = |t: f64| 10.0 / (1.0 + 19.0 * (-2.0 * t).exp());
    let log_rhs = |_t: f64, y: f64| Some(2.0 * y * (1.0 - y / 10.0));
    for t_end in [0.5_f64, 2.0, 5.0] {
        assert_close(integrate_rk45(0.0, t_end, 0.5, log_rhs).unwrap(), logistic_exact(t_end), 1e-7, 1e-7, "logistic fwd");
    }
    assert_close(integrate_rk45(2.0, 0.5, logistic_exact(2.0), log_rhs).unwrap(), logistic_exact(0.5), 1e-7, 1e-7, "logistic back 2->0.5");
    assert_close(integrate_rk45(5.0, 0.5, logistic_exact(5.0), log_rhs).unwrap(), logistic_exact(0.5), 1e-5, 1e-5, "logistic back 5->0.5");

    let dsl_osc = dsl_compile("Y0 = A\ndY/dX = B * Y * cos(X)");
    let p_osc: Vec<f64> = dsl_osc.param_names().iter().map(|n| if n == "B" { 6.0 } else { 1.0 }).collect();
    for x in [0.5_f64, core::f64::consts::FRAC_PI_2, core::f64::consts::PI, 2.0 * core::f64::consts::PI] {
        assert_close(dsl_osc.eval(&[x], &p_osc, 0).unwrap(), (6.0 * x.sin()).exp(), 1e-6, 1e-6, "DSL osc ODE");
    }
    assert_close(integrate_rk45(2.0 * core::f64::consts::PI, 0.0, 1.0, |t, y| Some(6.0 * y * t.cos())).unwrap(), 1.0, 1e-6, 1e-6, "osc back");
}

#[test]
fn r46_4_rout_detect_outliers_top_30_percent_motulsky_brown_2006() {
    let pts: Vec<DataPoint> = (0..100).map(|i| DataPoint { row: i, ..DataPoint::xy(i as f64, 100.0) }).collect();
    let res: Vec<f64> = (0..100)
        .map(|i| {
            if i < 69 {
                if i % 2 == 0 { 0.01 } else { -0.01 }
            } else if i == 69 {
                1.0
            } else {
                10.0
            }
        })
        .collect();
    let out = rout_detect_outliers(&pts, &res, 2, 5.0).unwrap();
    assert_eq!(out.outlier_indices.len(), 30);
    assert!(!out.audit[69].is_outlier);
    assert_close(out.audit[69].q_value, 1.0, 1e-15, 0.0, "31st point untested");
    assert!((70..100).all(|i| out.audit[i].is_outlier));

    let pts20: Vec<DataPoint> = (0..20).map(|i| DataPoint { row: i, ..DataPoint::xy(i as f64, 50.0) }).collect();
    let mut res20: Vec<f64> = (0..20).map(|i| if i % 2 == 0 { 0.1 } else { -0.1 }).collect();
    for (idx, val) in [(2, 0.14), (5, -0.17), (9, 0.20), (11, -0.25), (14, 4.0), (18, -5.5)] {
        res20[idx] = val;
    }
    let out20 = rout_detect_outliers(&pts20, &res20, 2, 1.0).unwrap();
    assert_eq!(out20.outlier_indices, vec![14, 18]);

    let (p14, p18, p11) = (
        student_t_two_sided_p(4.0 / out20.rsdr, 18.0).unwrap(),
        student_t_two_sided_p(5.5 / out20.rsdr, 18.0).unwrap(),
        student_t_two_sided_p(0.25 / out20.rsdr, 18.0).unwrap(),
    );
    assert_close(out20.audit[18].p_value, p18, 1e-15, 1e-12, "p rank 1");
    assert_close(out20.audit[14].p_value, p14, 1e-15, 1e-12, "p rank 2");
    assert_close(out20.audit[11].p_value, p11, 1e-15, 1e-12, "p rank 3");
    assert_close(out20.audit[14].q_value, (p14 * 20.0 / 2.0).min(1.0), 1e-15, 1e-12, "q rank 2");
    assert_close(out20.audit[11].q_value, (p11 * 20.0 / 3.0).min(1.0), 1e-15, 1e-12, "q rank 3");
    for i in [0, 1, 3, 4, 6, 7, 8, 10, 12, 13, 15, 16, 17, 19] {
        assert_close(out20.audit[i].q_value, 1.0, 1e-15, 0.0, "untested q");
        assert!(!out20.audit[i].is_outlier);
    }

    assert!(matches!(rout_detect_outliers(&pts20, &res20, 2, f64::NAN), Err(FitError::InvalidParameter { .. })));
    assert!(matches!(rout_detect_outliers(&pts20, &res20, 2, 120.0), Err(FitError::InvalidParameter { .. })));
    let mut bad = res20;
    bad[0] = f64::NAN;
    assert!(matches!(rout_detect_outliers(&pts20, &bad, 2, 1.0), Err(FitError::InvalidParameter { .. })));
}

#[test]
fn r46_3_replicates_lack_of_fit_presummarized_and_multidataset_oracles() {
    let expand_n3 = |x: f64, y_bar: f64, sd: f64, ds: usize| -> Vec<DataPoint> {
        [-1.0_f64, 0.0, 1.0].into_iter().map(|o| DataPoint { data_set_index: ds, ..DataPoint::xy(x, y_bar + o * sd) }).collect()
    };
    let sum_pt = |row: usize, x: f64, y: f64, sd: f64| DataPoint { sd: Some(sd), n: Some(3.0), row, ..DataPoint::xy(x, y) };

    let mut sum_pts = vec![sum_pt(0, 1.0, 10.0, 1.0), sum_pt(1, 1.0, 12.0, 1.0), sum_pt(2, 2.0, 15.0, 1.0)];
    assert!(simple_linear_regression(&sum_pts, &SimpleLinearOptions::default()).unwrap().lack_of_fit.is_none());

    sum_pts.push(sum_pt(3, 3.0, 25.0, 1.5));
    let mut raw_pts = Vec::new();
    for (x, yb, sd) in [(1.0, 10.0, 1.0), (1.0, 12.0, 1.0), (2.0, 15.0, 1.0), (3.0, 25.0, 1.5)] {
        raw_pts.extend(expand_n3(x, yb, sd, 0));
    }
    let (fit_sum, fit_raw) =
        (simple_linear_regression(&sum_pts, &Default::default()).unwrap(), simple_linear_regression(&raw_pts, &Default::default()).unwrap());
    let (lof_s, lof_r) = (fit_sum.lack_of_fit.as_ref().unwrap(), fit_raw.lack_of_fit.as_ref().unwrap());
    assert_eq!((lof_s.df_pure_error, lof_s.df_lack_of_fit), (9, 1));
    assert_eq!((lof_s.df_pure_error, lof_s.df_lack_of_fit), (lof_r.df_pure_error, lof_r.df_lack_of_fit));
    assert_close(lof_s.ss_pure_error, lof_r.ss_pure_error, 1e-12, 1e-12, "SS_PE");
    assert_close(lof_s.ss_lack_of_fit, lof_r.ss_lack_of_fit, 1e-12, 1e-12, "SS_LOF");
    assert_close(lof_s.f_stat, lof_r.f_stat, 1e-12, 1e-12, "F");
    assert_close(lof_s.p_value, lof_r.p_value, 1e-12, 1e-12, "p");

    let mut mixed = expand_n3(1.0, 10.0, 1.0, 0);
    mixed.push(sum_pt(3, 1.0, 12.0, 1.0));
    mixed.extend(expand_n3(2.0, 15.0, 1.0, 0));
    mixed.extend(expand_n3(3.0, 25.0, 1.5, 0));
    let lof_m = replicates_lack_of_fit(&mixed, fit_raw.rss, fit_raw.df_residual).unwrap();
    assert_eq!(lof_m.df_pure_error, lof_r.df_pure_error);
    assert_close(lof_m.ss_pure_error, lof_r.ss_pure_error, 1e-12, 1e-12, "mixed SS_PE");

    let two_curves = [
        DataPoint { data_set_index: 0, row: 0, ..DataPoint::xy(1.0, 2.0) },
        DataPoint { data_set_index: 0, row: 1, ..DataPoint::xy(2.0, 4.0) },
        DataPoint { data_set_index: 1, row: 0, ..DataPoint::xy(1.0, 20.0) },
        DataPoint { data_set_index: 1, row: 1, ..DataPoint::xy(2.0, 40.0) },
    ];
    assert!(replicates_lack_of_fit(&two_curves, 1.0, 2).is_none());
}
