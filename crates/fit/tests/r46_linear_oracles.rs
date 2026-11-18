//! D1 literature oracles and regression reproducers for `R4.6` fixes in [`curvebook_fit::linear`]:
//! - Deming Model II regression `variance_ratio_lambda = sigma_x^2 / sigma_y^2`
//! - Linear inverse calibration (`interpolate_x_from_y`) prediction vs mean-response intervals
//!
//! # References
//! - Deming, W. E. (1943). *Statistical Adjustment of Data*, Ch. 9. Wiley.
//! - Linnet, K. (1990). Estimation of the linear relationship between the measurements of two
//!   methods with proportional errors. *Statistics in Medicine* 9(12), 1463–1473.
//! - Linnet, K. (1993). Evaluation of regression procedures for methods comparison studies.
//!   *Clinical Chemistry* 39(3), 424–432.
//! - Fieller, E. C. (1954). Some problems in interval estimation. *JRSS-B* 16(2), 175–185.
//! - Draper, N. R. & Smith, H. (1998). *Applied Regression Analysis*, 3rd ed., Ch. 3. Wiley.
//! - Seber, G. A. F. & Lee, A. J. (2003). *Linear Regression Analysis*, 2nd ed., §6.1.3. Wiley.
//! - NIST/SEMATECH e-Handbook of Statistical Methods, §4.5.1.1 (Linear Calibration).

use curvebook_doc::CiBound;
use curvebook_fit::linear::{InverseIntervalSet, interpolate_x_from_y};
use curvebook_fit::{DataPoint, DemingOptions, FitError, SimpleLinearOptions, deming_regression, simple_linear_regression};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::student_t_two_sided_isf;

fn assert_close(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, label: &str) {
    assert!(actual.is_finite(), "{label}: actual is non-finite ({actual})");
    let diff = (actual - expected).abs();
    let bound = abs_tol.max(rel_tol * expected.abs());
    assert!(diff <= bound, "{label}: actual={actual:.15e}, expected={expected:.15e}, diff={diff:.3e} > bound={bound:.3e}");
}

fn xy_points(xs: &[f64], ys: &[f64]) -> Vec<DataPoint> {
    xs.iter().zip(ys.iter()).map(|(&x, &y)| DataPoint::xy(x, y)).collect()
}

fn cross_points(delta_x: f64, delta_y: f64) -> Vec<DataPoint> {
    // Symmetric cross perturbations of half-widths (delta_x, delta_y) around y = 1 + 2x.
    // With S_xx = 4 S_0 + 2 K delta_x^2, S_yy = 4 b^2 S_0 + 2 K delta_y^2, S_xy = 4 b S_0,
    // the Deming normal equation lambda S_xy b^2 + (S_xx - lambda S_yy) b - S_xy = 0 holds
    // identically at b = 2 iff delta_x^2 / delta_y^2 = sigma_x^2 / sigma_y^2 = lambda.
    let mut pts = Vec::with_capacity(16);
    for xk in [-3.0_f64, -1.0, 1.0, 3.0] {
        let yk = 1.0 + 2.0 * xk;
        pts.push(DataPoint::xy(xk + delta_x, yk));
        pts.push(DataPoint::xy(xk - delta_x, yk));
        pts.push(DataPoint::xy(xk, yk + delta_y));
        pts.push(DataPoint::xy(xk, yk - delta_y));
    }
    pts
}

#[test]
fn deming_lambda_sigma_x2_over_sigma_y2_reproducer() {
    // Reproducer for R4.6 checkbox 1: variance_ratio_lambda = sigma_x^2 / sigma_y^2.
    // At lambda = 0.25 (sigma_x = 1, sigma_y = 2), exact cross perturbations recover slope = 2.0, intercept = 1.0
    // while OLS attenuates below 1.95 and inverted lambda wrongly overshoots above 2.17.
    let pts_025 = cross_points(1.0, 2.0);
    let dem_025 =
        deming_regression(&pts_025, &DemingOptions { variance_ratio_lambda: 0.25, ..DemingOptions::default() }).expect("Deming lambda=0.25");
    assert_close(dem_025.slope, 2.0, 1e-12, 1e-12, "cross lambda=0.25 slope");
    assert_close(dem_025.intercept, 1.0, 1e-12, 1e-12, "cross lambda=0.25 intercept");
    assert_close(dem_025.lambda, 0.25, 1e-15, 1e-15, "reported lambda=0.25");
    let ols_025 = simple_linear_regression(&pts_025, &SimpleLinearOptions::default()).expect("OLS");
    assert!(ols_025.slope < 1.95, "OLS must attenuate on X noise: got {}", ols_025.slope);

    // At lambda = 4.0 (sigma_x = 2, sigma_y = 1), exact cross perturbations recover slope = 2.0, intercept = 1.0.
    let pts_400 = cross_points(2.0, 1.0);
    let dem_400 = deming_regression(&pts_400, &DemingOptions { variance_ratio_lambda: 4.0, ..DemingOptions::default() }).expect("Deming lambda=4.0");
    assert_close(dem_400.slope, 2.0, 1e-12, 1e-12, "cross lambda=4.0 slope");
    assert_close(dem_400.intercept, 1.0, 1e-12, 1e-12, "cross lambda=4.0 intercept");
    let ols_400 = simple_linear_regression(&pts_400, &SimpleLinearOptions::default()).expect("OLS");
    assert!(ols_400.slope < 1.85, "OLS must attenuate strongly when sigma_x > sigma_y: got {}", ols_400.slope);
}

#[test]
fn deming_50_digit_oracles_across_lambda_and_ols_limits() {
    // Asymmetric 10-point method-comparison series verified against 60-digit Decimal arithmetic
    // using Deming (1943) / Linnet (1990, 1993) closed-form and Leave-One-Out jackknife SEs/CIs.
    let xs = [1.2, 2.1, 2.9, 4.3, 5.0, 6.2, 7.1, 8.4, 9.1, 10.5];
    let ys = [2.9, 5.4, 6.1, 9.8, 10.3, 13.7, 14.2, 18.1, 18.4, 22.3];
    let pts = xy_points(&xs, &ys);

    struct DemingOracle {
        lambda: f64,
        slope: f64,
        intercept: f64,
        sy_x: f64,
        se_slope: f64,
        se_intercept: f64,
        ci_slope: (f64, f64),
        ci_intercept: (f64, f64),
    }

    let oracles = [
        DemingOracle {
            lambda: 0.25,
            slope: 2.029_493_225_595_37,
            intercept: 0.592_478_478_618_312,
            sy_x: 0.400_631_050_601_127,
            se_slope: 0.060_192_779_385_176_3,
            se_intercept: 0.336_311_373_799_274,
            ci_slope: (1.890_688_427_423_72, 2.168_298_023_767_02),
            ci_intercept: (-0.183_056_940_079_009, 1.368_013_897_315_63),
        },
        DemingOracle {
            lambda: 0.5,
            slope: 2.031_934_115_932_34,
            intercept: 0.578_614_221_504_309,
            sy_x: 0.326_286_233_543_534,
            se_slope: 0.059_488_751_778_492_6,
            se_intercept: 0.334_136_812_412_184,
            ci_slope: (1.894_752_808_333, 2.169_115_423_531_68),
            ci_intercept: (-0.191_906_649_642_126, 1.349_135_092_650_74),
        },
        DemingOracle {
            lambda: 1.0,
            slope: 2.033_869_948_849_82,
            intercept: 0.567_618_690_533_048,
            sy_x: 0.252_195_157_547_999,
            se_slope: 0.058_907_192_619_154_7,
            se_intercept: 0.332_365_466_308_344,
            ci_slope: (1.898_029_719_076_78, 2.169_710_178_622_85),
            ci_intercept: (-0.198_817_449_173_055, 1.334_054_830_239_15),
        },
        DemingOracle {
            lambda: 2.0,
            slope: 2.035_151_481_751_01,
            intercept: 0.560_339_583_654_243,
            sy_x: 0.187_692_882_900_988,
            se_slope: 0.058_511_442_443_862_6,
            se_intercept: 0.331_173_641_649_463,
            ci_slope: (1.900_223_853_518_71, 2.170_079_109_983_32),
            ci_intercept: (-0.203_348_203_460_041, 1.324_027_370_768_53),
        },
        DemingOracle {
            lambda: 4.0,
            slope: 2.035_901_780_320_18,
            intercept: 0.556_077_887_781_378,
            sy_x: 0.136_442_782_224_106,
            se_slope: 0.058_275_914_097_137_1,
            se_intercept: 0.330_469_729_402_452,
            ci_slope: (1.901_517_281_429_38, 2.170_286_279_210_98),
            ci_intercept: (-0.205_986_674_780_479, 1.318_142_450_343_23),
        },
    ];

    let mut prev_slope = 0.0_f64;
    for o in &oracles {
        let lam = o.lambda;
        let res = deming_regression(&pts, &DemingOptions { variance_ratio_lambda: lam, ..DemingOptions::default() })
            .unwrap_or_else(|e| panic!("deming lambda={lam} failed: {e}"));
        assert_close(res.slope, o.slope, 1e-11, 1e-11, &format!("lam={lam} slope"));
        assert_close(res.intercept, o.intercept, 1e-11, 1e-11, &format!("lam={lam} intercept"));
        assert_close(res.sy_x, o.sy_x, 1e-11, 1e-11, &format!("lam={lam} sy_x"));
        assert_close(res.se_slope, o.se_slope, 1e-11, 1e-11, &format!("lam={lam} se_slope"));
        assert_close(res.se_intercept, o.se_intercept, 1e-11, 1e-11, &format!("lam={lam} se_intercept"));
        assert_close(res.ci_slope.0, o.ci_slope.0, 1e-11, 1e-11, &format!("lam={lam} ci_slope.0"));
        assert_close(res.ci_slope.1, o.ci_slope.1, 1e-11, 1e-11, &format!("lam={lam} ci_slope.1"));
        assert_close(res.ci_intercept.0, o.ci_intercept.0, 1e-11, 1e-11, &format!("lam={lam} ci_int.0"));
        assert_close(res.ci_intercept.1, o.ci_intercept.1, 1e-11, 1e-11, &format!("lam={lam} ci_int.1"));
        assert!(res.slope > prev_slope, "Deming slope must increase monotonically with lambda");
        prev_slope = res.slope;
    }

    // Limiting behavior as lambda -> 0+ (sigma_x << sigma_y): must converge to OLS of Y on X.
    let ols_yx = simple_linear_regression(&pts, &SimpleLinearOptions::default()).expect("OLS Y~X");
    let dem_low = deming_regression(&pts, &DemingOptions { variance_ratio_lambda: 1e-12, ..DemingOptions::default() }).expect("Deming lambda=1e-12");
    assert_close(dem_low.slope, ols_yx.slope, 1e-10, 1e-10, "lambda=1e-12 -> OLS(Y~X) slope");
    assert_close(dem_low.intercept, ols_yx.intercept, 1e-10, 1e-10, "lambda=1e-12 -> OLS(Y~X) intercept");
    assert_close(dem_low.sy_x, ols_yx.sy_x, 1e-10, 1e-10, "lambda=1e-12 -> OLS(Y~X) sy_x");

    // Limiting behavior as lambda -> +inf (sigma_y << sigma_x): must converge to 1 / OLS(X on Y) = S_yy / S_xy.
    let swapped = xy_points(&ys, &xs);
    let ols_xy = simple_linear_regression(&swapped, &SimpleLinearOptions::default()).expect("OLS X~Y");
    let dem_high = deming_regression(&pts, &DemingOptions { variance_ratio_lambda: 1e12, ..DemingOptions::default() }).expect("Deming lambda=1e12");
    assert_close(dem_high.slope, 1.0 / ols_xy.slope, 1e-10, 1e-10, "lambda=1e12 -> 1/OLS(X~Y) slope");
    assert_close(dem_high.sy_x, 2.809_913_460_547_39e-7, 1e-12, 1e-10, "lambda=1e12 sy_x -> sigma_y oracle");
}

#[test]
fn deming_xy_swap_reciprocity_and_edge_cases() {
    let xs = [1.2, 2.1, 2.9, 4.3, 5.0, 6.2, 7.1, 8.4, 9.1, 10.5];
    let ys = [2.9, 5.4, 6.1, 9.8, 10.3, 13.7, 14.2, 18.1, 18.4, 22.3];
    let fwd_pts = xy_points(&xs, &ys);
    let rev_pts = xy_points(&ys, &xs);

    // Swapping (X, Y) maps sigma_x^2 / sigma_y^2 -> sigma_y^2 / sigma_x^2 = 1 / lambda:
    // b' = 1 / b, a' = -a / b, sy_x' = sqrt(lambda) * sy_x.
    for &lam in &[0.25, 0.5, 4.0] {
        let f = deming_regression(&fwd_pts, &DemingOptions { variance_ratio_lambda: lam, ..DemingOptions::default() }).unwrap();
        let r = deming_regression(&rev_pts, &DemingOptions { variance_ratio_lambda: 1.0 / lam, ..DemingOptions::default() }).unwrap();
        assert_close(f.slope * r.slope, 1.0, 1e-12, 1e-12, &format!("swap slope product lam={lam}"));
        assert_close(r.intercept, -f.intercept / f.slope, 1e-11, 1e-11, &format!("swap intercept lam={lam}"));
        assert_close(r.sy_x, lam.sqrt() * f.sy_x, 1e-12, 1e-12, &format!("swap sy_x scaling lam={lam}"));
    }

    // Zero covariance S_xy == 0 with lambda S_yy < S_xx gives exact horizontal slope 0.0:
    // Symmetric rectangle x in {-2, 2}, y in {1, 3} => S_xx = 16, S_yy = 4, S_xy = 0.
    // At lambda = 1.0: 1*4 < 16 -> horizontal slope 0.0, intercept = 2.0.
    // At lambda = 4.0: 4*4 == 16 -> indeterminate axis -> Err(InvalidParameter).
    // At lambda = 8.0: 8*4 > 16 -> vertical axis -> Err(InvalidParameter).
    let rect = [DataPoint::xy(-2.0, 1.0), DataPoint::xy(-2.0, 3.0), DataPoint::xy(2.0, 1.0), DataPoint::xy(2.0, 3.0)];
    let h_dem = deming_regression(&rect, &DemingOptions { variance_ratio_lambda: 1.0, ..DemingOptions::default() }).expect("horizontal zero-cov");
    assert_close(h_dem.slope, 0.0, 1e-14, 1e-14, "rect slope == 0");
    assert_close(h_dem.intercept, 2.0, 1e-14, 1e-14, "rect intercept");
    assert!(matches!(
        deming_regression(&rect, &DemingOptions { variance_ratio_lambda: 4.0, ..DemingOptions::default() }),
        Err(FitError::InvalidParameter { .. })
    ));
    assert!(matches!(
        deming_regression(&rect, &DemingOptions { variance_ratio_lambda: 8.0, ..DemingOptions::default() }),
        Err(FitError::InvalidParameter { .. })
    ));

    // Exact n = 3 minimum: jackknife subsamples have size 2; sy_x and se_slope must stay finite (no 0/0 NaN).
    let tri = xy_points(&[1.0, 2.0, 4.0], &[2.2, 3.8, 8.1]);
    let dem_3 = deming_regression(&tri, &DemingOptions { variance_ratio_lambda: 0.25, ..DemingOptions::default() }).expect("n=3 deming");
    assert!(dem_3.sy_x.is_finite() && dem_3.se_slope.is_finite() && dem_3.se_intercept.is_finite());

    // Error handling: empty, n < 3, invalid lambda, non-finite inputs.
    assert!(matches!(deming_regression(&[], &DemingOptions::default()), Err(FitError::EmptyInput)));
    assert!(matches!(deming_regression(&tri[..2], &DemingOptions::default()), Err(FitError::InsufficientData { required: 3, actual: 2 })));
    for &bad_lam in &[0.0, -0.5, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            deming_regression(&tri, &DemingOptions { variance_ratio_lambda: bad_lam, ..DemingOptions::default() }),
            Err(FitError::InvalidParameter { name: "variance_ratio_lambda", .. })
        ));
    }
    let bad_pts = xy_points(&[1.0, f64::NAN, 3.0], &[2.0, 4.0, 6.0]);
    assert!(matches!(deming_regression(&bad_pts, &DemingOptions::default()), Err(FitError::InvalidParameter { .. })));
}

#[test]
fn inverse_x_from_y_calibration_vs_mean_response_d1_oracles() {
    // 8-point calibration standard curve verified against 60-digit Seber & Lee (2003) §6.1.3 & NIST §4.5.1.1.
    let cal_x = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let cal_y = [3.1, 4.8, 7.3, 8.7, 11.4, 12.6, 15.2, 16.9];
    let pts = xy_points(&cal_x, &cal_y);

    let opts =
        SimpleLinearOptions { unknown_y_values: vec![10.0, 15.0], confidence_level: ConfidenceLevel::DEFAULT_95, ..SimpleLinearOptions::default() };
    let res = simple_linear_regression(&pts, &opts).expect("OLS calibration fit");
    assert_eq!(res.interpolated_x_from_y.len(), 2);

    // 1) simple_linear_regression matches mean-response (1/m=0) in ci_* and calibration (m=1) in pred_*.
    let it0 = &res.interpolated_x_from_y[0];
    assert_close(it0.estimate, 4.5, 1e-12, 1e-12, "y0=10 estimate");
    let (CiBound::Finite(ci0_lo), CiBound::Finite(ci0_hi)) = (it0.ci_lower, it0.ci_upper) else {
        panic!("expected finite CI at y0=10");
    };
    assert_close(ci0_lo, 4.362_750_432_716_67, 1e-11, 1e-11, "y0=10 mean CI lower");
    assert_close(ci0_hi, 4.637_249_567_283_33, 1e-11, 1e-11, "y0=10 mean CI upper");
    let (pi0_lo, pi0_hi) = (it0.pred_lower.expect("y0=10 pred_lower"), it0.pred_upper.expect("y0=10 pred_upper"));
    assert_close(pi0_lo, 4.088_251_298_150_02, 1e-11, 1e-11, "y0=10 calibration PI lower");
    assert_close(pi0_hi, 4.911_748_701_849_98, 1e-11, 1e-11, "y0=10 calibration PI upper");

    let it1 = &res.interpolated_x_from_y[1];
    assert_close(it1.estimate, 7.011_961_722_488_04, 1e-12, 1e-12, "y0=15 estimate");
    let (CiBound::Finite(ci1_lo), CiBound::Finite(ci1_hi)) = (it1.ci_lower, it1.ci_upper) else {
        panic!("expected finite CI at y0=15");
    };
    assert_close(ci1_lo, 6.817_113_821_777_65, 1e-11, 1e-11, "y0=15 mean CI lower");
    assert_close(ci1_hi, 7.224_835_885_130_93, 1e-11, 1e-11, "y0=15 mean CI upper");
    let (pi1_lo, pi1_hi) = (it1.pred_lower.expect("y0=15 pred_lower"), it1.pred_upper.expect("y0=15 pred_upper"));
    assert_close(pi1_lo, 6.582_501_513_338_9, 1e-11, 1e-11, "y0=15 calibration PI lower");
    assert_close(pi1_hi, 7.459_448_193_569_68, 1e-11, 1e-11, "y0=15 calibration PI upper");

    // 2) Exact forward/inverse duality: forward prediction band yhat(x) +- t*s*sqrt(1 + h(x)) equals y0 at both PI endpoints.
    let s_xx: f64 = cal_x.iter().map(|&x| (x - 4.5).powi(2)).sum();
    let t95 = student_t_two_sided_isf(0.05, 6.0).unwrap();
    for &(y0, ep_lo, ep_hi) in &[(10.0, pi0_lo, pi0_hi), (15.0, pi1_lo, pi1_hi)] {
        for &ep in &[ep_lo, ep_hi] {
            let y_hat = res.intercept + res.slope * ep;
            let h = 1.0 / 8.0 + (ep - 4.5).powi(2) / s_xx;
            let rhs = t95 * res.sy_x * (1.0 + h).sqrt();
            assert_close((y0 - y_hat).abs(), rhs, 1e-11, 1e-11, &format!("forward/inverse PI duality at y0={y0}, x={ep}"));
        }
    }

    // 3) Direct interpolate_x_from_y API for m = 1, m = 4, and 99% confidence.
    let cal_m1 = interpolate_x_from_y(&pts, false, ConfidenceLevel::DEFAULT_95, &[10.0, 15.0], 1).expect("cal m=1");
    let cal_m4 = interpolate_x_from_y(&pts, false, ConfidenceLevel::DEFAULT_95, &[10.0, 15.0], 4).expect("cal m=4");
    assert_eq!(cal_m1[0].mean_response_ci, (it0.ci_lower, it0.ci_upper));
    assert_eq!(cal_m1[0].calibration_pi, (CiBound::Finite(pi0_lo), CiBound::Finite(pi0_hi)));
    let (m4_0_lo, m4_0_hi) = cal_m4[0].calibration_set.finite_bounds().expect("m=4 bounded");
    assert_close(m4_0_lo, 4.262_276_776_148_44, 1e-11, 1e-11, "y0=10 m=4 lower");
    assert_close(m4_0_hi, 4.737_723_223_851_56, 1e-11, 1e-11, "y0=10 m=4 upper");
    assert!(pi0_lo < m4_0_lo && m4_0_lo < ci1_lo.min(4.362_750_432_716_674));
    let (m4_1_lo, m4_1_hi) = cal_m4[1].calibration_set.finite_bounds().expect("m=4 bounded y0=15");
    assert_close(m4_1_lo, 6.739_489_138_585_51, 1e-11, 1e-11, "y0=15 m=4 lower");
    assert_close(m4_1_hi, 7.302_460_568_323_06, 1e-11, 1e-11, "y0=15 m=4 upper");

    let cal_99_m4 = interpolate_x_from_y(&pts, false, ConfidenceLevel::PERCENT_99, &[10.0], 4).expect("cal 99% m=4");
    assert_eq!(cal_99_m4[0].unknown_replicates, 4);
    let (mc99_lo, mc99_hi) = cal_99_m4[0].mean_response_set.finite_bounds().unwrap();
    assert_close(mc99_lo, 4.291_561_830_496_8, 1e-11, 1e-11, "99% mean lower");
    assert_close(mc99_hi, 4.708_438_169_503_2, 1e-11, 1e-11, "99% mean upper");
    let (pi99_lo, pi99_hi) = cal_99_m4[0].calibration_set.finite_bounds().unwrap();
    assert_close(pi99_lo, 4.138_974_500_183_8, 1e-11, 1e-11, "99% m=4 lower");
    assert_close(pi99_hi, 4.861_025_499_816_2, 1e-11, 1e-11, "99% m=4 upper");

    // 4) Negative-slope curve (n = 6, df = 4).
    let neg_pts = xy_points(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[17.8, 15.1, 12.4, 8.9, 6.2, 3.0]);
    let neg_cal = interpolate_x_from_y(&neg_pts, false, ConfidenceLevel::DEFAULT_95, &[10.0], 3).expect("neg slope m=3");
    assert_close(neg_cal[0].estimate, 3.690_339_091_490_72, 1e-11, 1e-11, "neg slope x_hat");
    let (n_ci_lo, n_ci_hi) = neg_cal[0].mean_response_set.finite_bounds().unwrap();
    assert_close(n_ci_lo, 3.600_049_779_072_15, 1e-11, 1e-11, "neg slope mean CI lo");
    assert_close(n_ci_hi, 3.781_691_732_131_96, 1e-11, 1e-11, "neg slope mean CI hi");
    let (n_pi_lo, n_pi_hi) = neg_cal[0].calibration_set.finite_bounds().unwrap();
    assert_close(n_pi_lo, 3.534_210_642_388_06, 1e-11, 1e-11, "neg slope m=3 PI lo");
    assert_close(n_pi_hi, 3.847_530_868_816_05, 1e-11, 1e-11, "neg slope m=3 PI hi");

    // 5) Origin-forced calibration (df = n - 1 = 7): including y0 = 0 where mean CI is [0, 0] while m=1 PI is positive width.
    let orig_m4 = interpolate_x_from_y(&pts, true, ConfidenceLevel::DEFAULT_95, &[10.0, 0.0], 4).expect("origin m=4");
    let (o_ci_lo, o_ci_hi) = orig_m4[0].mean_response_set.finite_bounds().unwrap();
    assert_close(o_ci_lo, 4.402_966_200_398_16, 1e-11, 1e-11, "origin mean CI lo");
    assert_close(o_ci_hi, 4.812_728_478_073_7, 1e-11, 1e-11, "origin mean CI hi");
    let (o_pi_lo, o_pi_hi) = orig_m4[0].calibration_set.finite_bounds().unwrap();
    assert_close(o_pi_lo, 4.229_689_603_392_96, 1e-11, 1e-11, "origin m=4 PI lo");
    assert_close(o_pi_hi, 4.986_005_075_078_89, 1e-11, 1e-11, "origin m=4 PI hi");
    assert_eq!(orig_m4[1].mean_response_set.finite_bounds(), Some((0.0, 0.0)));
    let (z_lo, z_hi) = orig_m4[1].calibration_set.finite_bounds().unwrap();
    assert!(z_lo < 0.0 && z_hi > 0.0 && (z_lo + z_hi).abs() < 1e-14);

    // 6) RawReplicates vs MeanSdN pre-summarized equivalence on calibration & inverse CI.
    let raw_rep_pts = vec![
        DataPoint::xy(1.0, 2.9),
        DataPoint::xy(1.0, 3.3),
        DataPoint::xy(2.0, 4.8),
        DataPoint::xy(2.0, 5.2),
        DataPoint::xy(3.0, 6.7),
        DataPoint::xy(3.0, 7.5),
    ];
    let sd0 = ((2.9_f64 - 3.1).powi(2) + (3.3_f64 - 3.1).powi(2)).sqrt(); // Bessel df=1 SD
    let sd1 = ((4.8_f64 - 5.0).powi(2) + (5.2_f64 - 5.0).powi(2)).sqrt();
    let sd2 = ((6.7_f64 - 7.1).powi(2) + (7.5_f64 - 7.1).powi(2)).sqrt();
    let sum_pts = vec![
        DataPoint { sd: Some(sd0), n: Some(2.0), ..DataPoint::xy(1.0, 3.1) },
        DataPoint { sd: Some(sd1), n: Some(2.0), ..DataPoint::xy(2.0, 5.0) },
        DataPoint { sd: Some(sd2), n: Some(2.0), ..DataPoint::xy(3.0, 7.1) },
    ];
    let cal_raw = interpolate_x_from_y(&raw_rep_pts, false, ConfidenceLevel::DEFAULT_95, &[5.5], 2).unwrap();
    let cal_sum = interpolate_x_from_y(&sum_pts, false, ConfidenceLevel::DEFAULT_95, &[5.5], 2).unwrap();
    assert_close(cal_raw[0].estimate, cal_sum[0].estimate, 1e-13, 1e-13, "raw vs MeanSdN estimate");
    assert_close(cal_raw[0].fieller_g, cal_sum[0].fieller_g, 1e-13, 1e-13, "raw vs MeanSdN fieller_g");
    let (rl, ru) = cal_raw[0].calibration_set.finite_bounds().unwrap();
    let (sl, su) = cal_sum[0].calibration_set.finite_bounds().unwrap();
    assert_close(rl, sl, 1e-12, 1e-12, "raw vs MeanSdN cal lo");
    assert_close(ru, su, 1e-12, 1e-12, "raw vs MeanSdN cal hi");
}

#[test]
fn inverse_x_from_y_fieller_regimes_g_eq_1_and_gt_1_and_edges() {
    // 1) Weak-slope g > 1 regime (n = 5, df = 3, g = 20.25592897...): covers AllReals and TwoRays.
    let weak_pts = xy_points(&[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 0.0, 4.0, 1.0, 3.5]);
    let weak_cal = interpolate_x_from_y(&weak_pts, false, ConfidenceLevel::DEFAULT_95, &[2.1, 8.0, 15.0, -10.0], 1).expect("weak calibration");
    assert!(weak_cal[0].fieller_g > 20.0);

    // y0 = 2.1 (d = 0): both mean-response and calibration m=1 cover (-inf, +inf).
    assert_eq!(weak_cal[0].mean_response_set, InverseIntervalSet::AllReals);
    assert_eq!(weak_cal[0].calibration_set, InverseIntervalSet::AllReals);
    assert_eq!(weak_cal[0].calibration_pi, (CiBound::Open, CiBound::Open));

    // y0 = 8.0: intermediate d where mean-response is TwoRays while wider m=1 calibration is AllReals!
    let InverseIntervalSet::TwoRays { left_upper: l8, right_lower: r8 } = weak_cal[1].mean_response_set else {
        panic!("expected TwoRays for y0=8.0 mean_response_set, got {:?}", weak_cal[1].mean_response_set);
    };
    assert_close(l8, -0.893_513_901_870_457, 1e-11, 1e-11, "y0=8.0 mean TwoRays left_upper");
    assert_close(r8, 5.361_518_221_846_23, 1e-11, 1e-11, "y0=8.0 mean TwoRays right_lower");
    assert!(
        weak_cal[1].mean_response_set.contains(l8) && weak_cal[1].mean_response_set.contains(-10.0) && !weak_cal[1].mean_response_set.contains(3.0)
    );
    assert_eq!(weak_cal[1].calibration_set, InverseIntervalSet::AllReals);

    // y0 = 15.0 & y0 = -10.0: both mean-response and m=1 calibration are TwoRays, with cal superset of mean.
    let expected_rays = [
        (2_usize, -6.071_680_168_957_39, 8.722_062_495_684_08, -5.322_694_586_061_7, 7.973_076_912_788_39),
        (3_usize, -2.348_959_978_253_02, 11.490_849_423_726_4, -1.541_809_198_474_25, 10.683_698_643_947_7),
    ];
    for &(idx, m_lo, m_hi, c_lo, c_hi) in &expected_rays {
        let InverseIntervalSet::TwoRays { left_upper: ml, right_lower: mr } = weak_cal[idx].mean_response_set else {
            panic!("expected TwoRays mean_response at idx={idx}");
        };
        let InverseIntervalSet::TwoRays { left_upper: cl, right_lower: cr } = weak_cal[idx].calibration_set else {
            panic!("expected TwoRays calibration at idx={idx}");
        };
        assert_close(ml, m_lo, 1e-11, 1e-11, &format!("idx={idx} mean TwoRays left"));
        assert_close(mr, m_hi, 1e-11, 1e-11, &format!("idx={idx} mean TwoRays right"));
        assert_close(cl, c_lo, 1e-11, 1e-11, &format!("idx={idx} cal TwoRays left"));
        assert_close(cr, c_hi, 1e-11, 1e-11, &format!("idx={idx} cal TwoRays right"));
        assert!(ml < cl && cr < mr, "calibration TwoRays must be strictly wider than mean-response TwoRays");
        assert!(weak_cal[idx].calibration_set.contains(ml) && weak_cal[idx].calibration_set.contains(mr));
    }

    // SimpleLinearResult maps g > 1 to Open CI and None finite pred bounds (no false finite bounds!).
    let weak_ols =
        simple_linear_regression(&weak_pts, &SimpleLinearOptions { unknown_y_values: vec![2.1, 15.0], ..SimpleLinearOptions::default() }).unwrap();
    for item in &weak_ols.interpolated_x_from_y {
        assert_eq!((item.ci_lower, item.ci_upper), (CiBound::Open, CiBound::Open));
        assert!(item.pred_lower.is_none() && item.pred_upper.is_none());
    }

    // 2) Exact rational g == 1.0 HalfLine dataset: x=[-2,-2,2,2], y=[-3.5,-0.5,3.5,0.5], 80% CI (t^2 = 32/9 => g = 1.0).
    let conf80 = ConfidenceLevel::new(80.0).unwrap();
    let half_pts = xy_points(&[-2.0, -2.0, 2.0, 2.0], &[-3.5, -0.5, 3.5, 0.5]);
    let half_m1 = interpolate_x_from_y(&half_pts, false, conf80, &[5.0, -5.0, 0.0, 1.0], 1).expect("g=1 m=1");
    assert_close(half_m1[0].fieller_g, 1.0, 1e-13, 1e-13, "exact g == 1");
    let InverseIntervalSet::HalfLine { lower: Some(h0_m_lo), upper: None } = half_m1[0].mean_response_set else {
        panic!("expected HalfLine [2.1, +inf) at y0=5 mean");
    };
    assert_close(h0_m_lo, 2.1, 1e-12, 1e-12, "g=1 y0=5 mean lower");
    let InverseIntervalSet::HalfLine { lower: Some(h0_c_lo), upper: None } = half_m1[0].calibration_set else {
        panic!("expected HalfLine [0.5, +inf) at y0=5 cal m=1");
    };
    assert_close(h0_c_lo, 0.5, 1e-12, 1e-12, "g=1 y0=5 cal lower");
    let (CiBound::Finite(ci_h0), CiBound::Open) = half_m1[0].mean_response_ci else {
        panic!("expected Finite/Open mean_response_ci at g=1");
    };
    assert_close(ci_h0, 2.1, 1e-12, 1e-12, "g=1 mean_response_ci finite bound");
    let (CiBound::Finite(pi_h0), CiBound::Open) = half_m1[0].calibration_pi else {
        panic!("expected Finite/Open calibration_pi at g=1");
    };
    assert_close(pi_h0, 0.5, 1e-12, 1e-12, "g=1 calibration_pi finite bound");
    let InverseIntervalSet::HalfLine { lower: None, upper: Some(h1_m_hi) } = half_m1[1].mean_response_set else {
        panic!("expected HalfLine (-inf, -2.1] at y0=-5 mean");
    };
    assert_close(h1_m_hi, -2.1, 1e-12, 1e-12, "g=1 y0=-5 mean upper");
    let InverseIntervalSet::HalfLine { lower: None, upper: Some(h1_c_hi) } = half_m1[1].calibration_set else {
        panic!("expected HalfLine (-inf, -0.5] at y0=-5 cal m=1");
    };
    assert_close(h1_c_hi, -0.5, 1e-12, 1e-12, "g=1 y0=-5 cal upper");
    assert_eq!(half_m1[2].mean_response_set, InverseIntervalSet::AllReals);
    assert_eq!(half_m1[2].calibration_set, InverseIntervalSet::AllReals);
    let InverseIntervalSet::HalfLine { lower: Some(h3_m_lo), upper: None } = half_m1[3].mean_response_set else {
        panic!("expected HalfLine [-1.5, +inf)");
    };
    assert_close(h3_m_lo, -1.5, 1e-12, 1e-12, "g=1 y0=1 mean lower");
    let InverseIntervalSet::HalfLine { lower: Some(h3_c_lo), upper: None } = half_m1[3].calibration_set else {
        panic!("expected HalfLine [-9.5, +inf)");
    };
    assert_close(h3_c_lo, -9.5, 1e-12, 1e-12, "g=1 y0=1 cal lower");

    let half_m4 = interpolate_x_from_y(&half_pts, false, conf80, &[5.0], 4).expect("g=1 m=4");
    let InverseIntervalSet::HalfLine { lower: Some(hm4_lo), upper: None } = half_m4[0].calibration_set else {
        panic!("expected HalfLine [1.7, +inf) at y0=5 cal m=4");
    };
    assert_close(hm4_lo, 1.7, 1e-12, 1e-12, "g=1 y0=5 cal m=4 lower");

    // 3) Noiseless fit (sy_x == 0), flat slope (slope == 0), invalid inputs, and unknown_replicates == 0.
    let exact_pts = xy_points(&[1.0, 2.0, 3.0, 4.0], &[3.0, 5.0, 7.0, 9.0]);
    let exact_cal = interpolate_x_from_y(&exact_pts, false, ConfidenceLevel::DEFAULT_95, &[6.0], 1).unwrap();
    assert_eq!(exact_cal[0].fieller_g, 0.0);
    assert_eq!(exact_cal[0].mean_response_set, InverseIntervalSet::Bounded { lower: 2.5, upper: 2.5 });
    assert_eq!(exact_cal[0].calibration_set, InverseIntervalSet::Bounded { lower: 2.5, upper: 2.5 });

    let flat_pts = xy_points(&[1.0, 2.0, 3.0, 4.0], &[5.0, 5.0, 5.0, 5.0]);
    let flat_cal = interpolate_x_from_y(&flat_pts, false, ConfidenceLevel::DEFAULT_95, &[5.0, 6.0], 1).unwrap();
    assert!(
        flat_cal.iter().all(|c| c.estimate.is_nan()
            && c.mean_response_set == InverseIntervalSet::Undefined
            && c.calibration_set == InverseIntervalSet::Undefined)
    );

    let edge_ys = simple_linear_regression(
        &exact_pts,
        &SimpleLinearOptions {
            unknown_y_values: vec![f64::NAN, f64::INFINITY],
            unknown_x_values: vec![f64::NAN, f64::NEG_INFINITY],
            ..SimpleLinearOptions::default()
        },
    )
    .unwrap();
    for item in edge_ys.interpolated_x_from_y.iter().chain(edge_ys.interpolated_y_from_x.iter()) {
        assert!(item.estimate.is_nan());
        assert_eq!((item.ci_lower, item.ci_upper), (CiBound::Open, CiBound::Open));
        assert!(item.pred_lower.is_none() && item.pred_upper.is_none());
    }

    assert!(matches!(
        interpolate_x_from_y(&exact_pts, false, ConfidenceLevel::DEFAULT_95, &[5.0], 0),
        Err(FitError::InvalidParameter { name: "unknown_replicates", .. })
    ));
}

#[test]
fn restricted_linear_fit_matches_terminal_phase_ols_oracle() {
    // Draper & Smith (1998), Ch. 3. For the four included observations,
    // the residuals (1, -1, -1, 1) are orthogonal to both the intercept
    // and X columns, so the exact fit is Y = 3 + 2X, with RSS = 4, TSS = 84.
    let points = xy_points(&[0.0, 10.0, 12.0, 14.0, 16.0, 20.0], &[90.0, 24.0, 26.0, 30.0, 36.0, -100.0]);
    let mut opts = SimpleLinearOptions { x_range_filter: Some((Some(10.0), Some(16.0))), band_grid_points: 4, ..SimpleLinearOptions::default() };
    let fit = simple_linear_regression(&points, &opts).expect("restricted OLS");
    assert_eq!((fit.n, fit.n_excluded_by_range, fit.df_residual), (4, 2, 2));
    assert_close(fit.slope, 2.0, 1e-12, 0.0, "terminal slope");
    assert_close(fit.intercept, 3.0, 1e-12, 0.0, "terminal intercept");
    assert_close(fit.rss, 4.0, 1e-12, 0.0, "terminal RSS");
    assert_close(fit.r_squared, 20.0 / 21.0, 1e-12, 0.0, "terminal R squared");
    assert_eq!(fit.residuals.iter().map(|p| (p.x, p.residual)).collect::<Vec<_>>(), [(10.0, 1.0), (12.0, -1.0), (14.0, -1.0), (16.0, 1.0)]);
    assert_eq!(fit.bands.iter().map(|p| p.x).collect::<Vec<_>>(), [10.0, 12.0, 14.0, 16.0]);

    opts.plot_x_range = Some((None, Some(19.0)));
    let extended = simple_linear_regression(&points, &opts).expect("explicit plot range");
    assert_eq!(extended.bands.iter().map(|p| p.x).collect::<Vec<_>>(), [10.0, 13.0, 16.0, 19.0]);
    assert_eq!(extended.residuals, fit.residuals);
    assert_eq!(extended.r_squared, fit.r_squared);
}

#[test]
fn restricted_linear_fit_counts_replicates_and_validates_bounds() {
    let mut points = xy_points(&[0.0, 11.0, 13.0, 15.0], &[9.0, 25.0, 29.0, 33.0]);
    points[0].n = Some(7.0);
    points[1].n = Some(3.0);
    let mut opts = SimpleLinearOptions { x_range_filter: Some((Some(10.0), None)), ..SimpleLinearOptions::default() };

    let fit = simple_linear_regression(&points, &opts).expect("one-sided filter");
    assert_eq!((fit.n, fit.n_excluded_by_range, fit.df_residual), (5, 7, 3));
    assert_eq!(fit.bands.first().map(|p| p.x), Some(10.0));
    assert_eq!(fit.bands.last().map(|p| p.x), Some(15.0));

    opts.x_range_filter = Some((Some(20.0), None));
    assert!(matches!(simple_linear_regression(&points, &opts), Err(FitError::EmptyInput)));
    opts.x_range_filter = Some((Some(12.0), None));
    assert!(matches!(simple_linear_regression(&points, &opts), Err(FitError::InsufficientData { required: 3, actual: 2 })));

    for range in [(Some(f64::NAN), None), (None, Some(f64::INFINITY)), (Some(15.0), Some(11.0))] {
        opts.x_range_filter = Some(range);
        assert!(matches!(simple_linear_regression(&points, &opts), Err(FitError::InvalidParameter { name: "x_range_filter", .. })));
    }
    opts.x_range_filter = None;
    for range in [(Some(99.0), None), (None, Some(f64::MAX))] {
        opts.plot_x_range = Some(range);
        assert!(matches!(simple_linear_regression(&points, &opts), Err(FitError::InvalidParameter { name: "plot_x_range", .. })));
    }
}
