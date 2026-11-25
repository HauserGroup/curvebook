//! D1 literature oracles and regression reproducers for `R4.6` fixes in [`curvebook_fit::linear`].
//! References: Deming (1943); Linnet (1990, 1993); Fieller (1954); Draper & Smith (1998);
//! Seber & Lee (2003); NIST/SEMATECH e-Handbook §4.5.1.1.

use curvebook_doc::CiBound;
use curvebook_fit::linear::{InverseIntervalSet, interpolate_x_from_y};
use curvebook_fit::{DataPoint, DemingOptions, FitError, SimpleLinearOptions, deming_regression, simple_linear_regression};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::student_t_two_sided_isf;
use curvebook_testkit::assert_approx_eq as assert_close;

use crate::common::xy_pts;

fn cross_points(dx: f64, dy: f64) -> Vec<DataPoint> {
    [-3.0_f64, -1.0, 1.0, 3.0]
        .into_iter()
        .flat_map(|xk| {
            let yk = 1.0 + 2.0 * xk;
            [DataPoint::xy(xk + dx, yk), DataPoint::xy(xk - dx, yk), DataPoint::xy(xk, yk + dy), DataPoint::xy(xk, yk - dy)]
        })
        .collect()
}

#[test]
fn deming_lambda_sigma_x2_over_sigma_y2_reproducer() {
    for (dx, dy, lam, ols_cap) in [(1.0, 2.0, 0.25, 1.95), (2.0, 1.0, 4.0, 1.85)] {
        let pts = cross_points(dx, dy);
        let dem = deming_regression(&pts, &DemingOptions { variance_ratio_lambda: lam, ..Default::default() }).unwrap();
        assert_close(dem.slope, 2.0, 1e-12, 1e-12, &format!("lam={lam} slope"));
        assert_close(dem.intercept, 1.0, 1e-12, 1e-12, &format!("lam={lam} int"));
        assert_close(dem.lambda, lam, 1e-15, 1e-15, &format!("lam={lam}"));
        assert!(simple_linear_regression(&pts, &SimpleLinearOptions::default()).unwrap().slope < ols_cap);
    }
}

#[test]
fn deming_50_digit_oracles_across_lambda_and_ols_limits() {
    let xs = [1.2, 2.1, 2.9, 4.3, 5.0, 6.2, 7.1, 8.4, 9.1, 10.5];
    let ys = [2.9, 5.4, 6.1, 9.8, 10.3, 13.7, 14.2, 18.1, 18.4, 22.3];
    let pts = xy_pts(&xs, &ys);

    struct DemingOracle {
        lam: f64,
        slope: f64,
        int: f64,
        sy_x: f64,
        se_s: f64,
        se_i: f64,
        ci_s: (f64, f64),
        ci_i: (f64, f64),
    }
    const fn d(lam: f64, slope: f64, int: f64, sy_x: f64, se: (f64, f64), ci_s: (f64, f64), ci_i: (f64, f64)) -> DemingOracle {
        DemingOracle { lam, slope, int, sy_x, se_s: se.0, se_i: se.1, ci_s, ci_i }
    }
    let oracles = [
        d(
            0.25,
            2.029_493_225_595_37,
            0.592_478_478_618_312,
            0.400_631_050_601_127,
            (0.060_192_779_385_176_3, 0.336_311_373_799_274),
            (1.890_688_427_423_72, 2.168_298_023_767_02),
            (-0.183_056_940_079_009, 1.368_013_897_315_63),
        ),
        d(
            0.5,
            2.031_934_115_932_34,
            0.578_614_221_504_309,
            0.326_286_233_543_534,
            (0.059_488_751_778_492_6, 0.334_136_812_412_184),
            (1.894_752_808_333, 2.169_115_423_531_68),
            (-0.191_906_649_642_126, 1.349_135_092_650_74),
        ),
        d(
            1.0,
            2.033_869_948_849_82,
            0.567_618_690_533_048,
            0.252_195_157_547_999,
            (0.058_907_192_619_154_7, 0.332_365_466_308_344),
            (1.898_029_719_076_78, 2.169_710_178_622_85),
            (-0.198_817_449_173_055, 1.334_054_830_239_15),
        ),
        d(
            2.0,
            2.035_151_481_751_01,
            0.560_339_583_654_243,
            0.187_692_882_900_988,
            (0.058_511_442_443_862_6, 0.331_173_641_649_463),
            (1.900_223_853_518_71, 2.170_079_109_983_32),
            (-0.203_348_203_460_041, 1.324_027_370_768_53),
        ),
        d(
            4.0,
            2.035_901_780_320_18,
            0.556_077_887_781_378,
            0.136_442_782_224_106,
            (0.058_275_914_097_137_1, 0.330_469_729_402_452),
            (1.901_517_281_429_38, 2.170_286_279_210_98),
            (-0.205_986_674_780_479, 1.318_142_450_343_23),
        ),
    ];

    let mut prev_slope = 0.0_f64;
    for o in &oracles {
        let res = deming_regression(&pts, &DemingOptions { variance_ratio_lambda: o.lam, ..Default::default() }).unwrap();
        for (tag, got, exp) in [
            ("slope", res.slope, o.slope),
            ("int", res.intercept, o.int),
            ("sy_x", res.sy_x, o.sy_x),
            ("se_s", res.se_slope, o.se_s),
            ("se_i", res.se_intercept, o.se_i),
            ("ci_s0", res.ci_slope.0, o.ci_s.0),
            ("ci_s1", res.ci_slope.1, o.ci_s.1),
            ("ci_i0", res.ci_intercept.0, o.ci_i.0),
            ("ci_i1", res.ci_intercept.1, o.ci_i.1),
        ] {
            assert_close(got, exp, 1e-11, 1e-11, &format!("lam={} {tag}", o.lam));
        }
        assert!(res.slope > prev_slope);
        prev_slope = res.slope;
    }

    let ols_yx = simple_linear_regression(&pts, &SimpleLinearOptions::default()).unwrap();
    let dem_low = deming_regression(&pts, &DemingOptions { variance_ratio_lambda: 1e-12, ..Default::default() }).unwrap();
    assert_close(dem_low.slope, ols_yx.slope, 1e-10, 1e-10, "low slope");
    assert_close(dem_low.intercept, ols_yx.intercept, 1e-10, 1e-10, "low int");
    assert_close(dem_low.sy_x, ols_yx.sy_x, 1e-10, 1e-10, "low sy_x");

    let ols_xy = simple_linear_regression(&xy_pts(&ys, &xs), &SimpleLinearOptions::default()).unwrap();
    let dem_high = deming_regression(&pts, &DemingOptions { variance_ratio_lambda: 1e12, ..Default::default() }).unwrap();
    assert_close(dem_high.slope, 1.0 / ols_xy.slope, 1e-10, 1e-10, "high slope");
    assert_close(dem_high.sy_x, 2.809_913_460_547_39e-7, 1e-12, 1e-10, "high sy_x");
}

#[test]
fn deming_xy_swap_reciprocity_and_edge_cases() {
    let xs = [1.2, 2.1, 2.9, 4.3, 5.0, 6.2, 7.1, 8.4, 9.1, 10.5];
    let ys = [2.9, 5.4, 6.1, 9.8, 10.3, 13.7, 14.2, 18.1, 18.4, 22.3];
    let (fwd, rev) = (xy_pts(&xs, &ys), xy_pts(&ys, &xs));

    for lam in [0.25_f64, 0.5, 4.0] {
        let f = deming_regression(&fwd, &DemingOptions { variance_ratio_lambda: lam, ..Default::default() }).unwrap();
        let r = deming_regression(&rev, &DemingOptions { variance_ratio_lambda: 1.0 / lam, ..Default::default() }).unwrap();
        assert_close(f.slope * r.slope, 1.0, 1e-12, 1e-12, "swap slope");
        assert_close(r.intercept, -f.intercept / f.slope, 1e-11, 1e-11, "swap int");
        assert_close(r.sy_x, lam.sqrt() * f.sy_x, 1e-12, 1e-12, "swap sy_x");
    }

    let rect = [DataPoint::xy(-2.0, 1.0), DataPoint::xy(-2.0, 3.0), DataPoint::xy(2.0, 1.0), DataPoint::xy(2.0, 3.0)];
    let h_dem = deming_regression(&rect, &DemingOptions { variance_ratio_lambda: 1.0, ..Default::default() }).unwrap();
    assert_close(h_dem.slope, 0.0, 1e-14, 1e-14, "rect slope");
    assert_close(h_dem.intercept, 2.0, 1e-14, 1e-14, "rect int");
    for bad in [4.0, 8.0] {
        assert!(matches!(
            deming_regression(&rect, &DemingOptions { variance_ratio_lambda: bad, ..Default::default() }),
            Err(FitError::InvalidParameter { .. })
        ));
    }

    let tri = xy_pts(&[1.0, 2.0, 4.0], &[2.2, 3.8, 8.1]);
    let dem3 = deming_regression(&tri, &DemingOptions { variance_ratio_lambda: 0.25, ..Default::default() }).unwrap();
    assert!(dem3.sy_x.is_finite() && dem3.se_slope.is_finite() && dem3.se_intercept.is_finite());

    assert!(matches!(deming_regression(&[], &Default::default()), Err(FitError::EmptyInput)));
    assert!(matches!(deming_regression(&tri[..2], &Default::default()), Err(FitError::InsufficientData { required: 3, actual: 2 })));
    for bad_lam in [0.0, -0.5, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            deming_regression(&tri, &DemingOptions { variance_ratio_lambda: bad_lam, ..Default::default() }),
            Err(FitError::InvalidParameter { name: "variance_ratio_lambda", .. })
        ));
    }
    assert!(matches!(
        deming_regression(&xy_pts(&[1.0, f64::NAN, 3.0], &[2.0, 4.0, 6.0]), &Default::default()),
        Err(FitError::InvalidParameter { .. })
    ));
}

#[test]
fn inverse_x_from_y_calibration_vs_mean_response_d1_oracles() {
    let cal_x = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let cal_y = [3.1, 4.8, 7.3, 8.7, 11.4, 12.6, 15.2, 16.9];
    let pts = xy_pts(&cal_x, &cal_y);

    let res = simple_linear_regression(
        &pts,
        &SimpleLinearOptions { unknown_y_values: vec![10.0, 15.0], confidence_level: ConfidenceLevel::DEFAULT_95, ..Default::default() },
    )
    .unwrap();
    assert_eq!(res.interpolated_x_from_y.len(), 2);

    let mut pi_pairs = Vec::new();
    for (idx, (exp_est, exp_ci, exp_pi)) in [
        (4.5, (4.362_750_432_716_67, 4.637_249_567_283_33), (4.088_251_298_150_02, 4.911_748_701_849_98)),
        (7.011_961_722_488_04, (6.817_113_821_777_65, 7.224_835_885_130_93), (6.582_501_513_338_9, 7.459_448_193_569_68)),
    ]
    .into_iter()
    .enumerate()
    {
        let it = &res.interpolated_x_from_y[idx];
        assert_close(it.estimate, exp_est, 1e-12, 1e-12, &format!("it[{idx}] est"));
        let (CiBound::Finite(clo), CiBound::Finite(chi)) = (it.ci_lower, it.ci_upper) else { panic!("finite CI") };
        let (plo, phi) = (it.pred_lower.unwrap(), it.pred_upper.unwrap());
        assert_close(clo, exp_ci.0, 1e-11, 1e-11, "CI lo");
        assert_close(chi, exp_ci.1, 1e-11, 1e-11, "CI hi");
        assert_close(plo, exp_pi.0, 1e-11, 1e-11, "PI lo");
        assert_close(phi, exp_pi.1, 1e-11, 1e-11, "PI hi");
        pi_pairs.push((if idx == 0 { 10.0 } else { 15.0 }, plo, phi));
    }

    let s_xx: f64 = cal_x.iter().map(|&x| (x - 4.5).powi(2)).sum();
    let t95 = student_t_two_sided_isf(0.05, 6.0).unwrap();
    for (y0, plo, phi) in pi_pairs {
        for ep in [plo, phi] {
            let rhs = t95 * res.sy_x * (1.0 + 1.0 / 8.0 + (ep - 4.5).powi(2) / s_xx).sqrt();
            assert_close((y0 - (res.intercept + res.slope * ep)).abs(), rhs, 1e-11, 1e-11, "PI duality");
        }
    }

    let cal_m1 = interpolate_x_from_y(&pts, false, ConfidenceLevel::DEFAULT_95, &[10.0, 15.0], 1).unwrap();
    let cal_m4 = interpolate_x_from_y(&pts, false, ConfidenceLevel::DEFAULT_95, &[10.0, 15.0], 4).unwrap();
    let it0 = &res.interpolated_x_from_y[0];
    assert_eq!(cal_m1[0].mean_response_ci, (it0.ci_lower, it0.ci_upper));
    assert_eq!(cal_m1[0].calibration_pi, (CiBound::Finite(it0.pred_lower.unwrap()), CiBound::Finite(it0.pred_upper.unwrap())));
    let (m4_0_lo, m4_0_hi) = cal_m4[0].calibration_set.finite_bounds().unwrap();
    assert_close(m4_0_lo, 4.262_276_776_148_44, 1e-11, 1e-11, "m4 y0=10 lo");
    assert_close(m4_0_hi, 4.737_723_223_851_56, 1e-11, 1e-11, "m4 y0=10 hi");
    assert!(it0.pred_lower.unwrap() < m4_0_lo && m4_0_lo < 4.362_750_432_716_674);
    let (m4_1_lo, m4_1_hi) = cal_m4[1].calibration_set.finite_bounds().unwrap();
    assert_close(m4_1_lo, 6.739_489_138_585_51, 1e-11, 1e-11, "m4 y0=15 lo");
    assert_close(m4_1_hi, 7.302_460_568_323_06, 1e-11, 1e-11, "m4 y0=15 hi");

    let cal_99 = interpolate_x_from_y(&pts, false, ConfidenceLevel::PERCENT_99, &[10.0], 4).unwrap();
    assert_eq!(cal_99[0].unknown_replicates, 4);
    let (mc_lo, mc_hi) = cal_99[0].mean_response_set.finite_bounds().unwrap();
    let (pi_lo, pi_hi) = cal_99[0].calibration_set.finite_bounds().unwrap();
    assert_close(mc_lo, 4.291_561_830_496_8, 1e-11, 1e-11, "99% mean lo");
    assert_close(mc_hi, 4.708_438_169_503_2, 1e-11, 1e-11, "99% mean hi");
    assert_close(pi_lo, 4.138_974_500_183_8, 1e-11, 1e-11, "99% pi lo");
    assert_close(pi_hi, 4.861_025_499_816_2, 1e-11, 1e-11, "99% pi hi");

    let neg = interpolate_x_from_y(
        &xy_pts(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[17.8, 15.1, 12.4, 8.9, 6.2, 3.0]),
        false,
        ConfidenceLevel::DEFAULT_95,
        &[10.0],
        3,
    )
    .unwrap();
    assert_close(neg[0].estimate, 3.690_339_091_490_72, 1e-11, 1e-11, "neg x_hat");
    let ((ncl, nch), (npl, nph)) = (neg[0].mean_response_set.finite_bounds().unwrap(), neg[0].calibration_set.finite_bounds().unwrap());
    assert_close(ncl, 3.600_049_779_072_15, 1e-11, 1e-11, "neg CI lo");
    assert_close(nch, 3.781_691_732_131_96, 1e-11, 1e-11, "neg CI hi");
    assert_close(npl, 3.534_210_642_388_06, 1e-11, 1e-11, "neg PI lo");
    assert_close(nph, 3.847_530_868_816_05, 1e-11, 1e-11, "neg PI hi");

    let orig = interpolate_x_from_y(&pts, true, ConfidenceLevel::DEFAULT_95, &[10.0, 0.0], 4).unwrap();
    let ((ocl, och), (opl, oph)) = (orig[0].mean_response_set.finite_bounds().unwrap(), orig[0].calibration_set.finite_bounds().unwrap());
    assert_close(ocl, 4.402_966_200_398_16, 1e-11, 1e-11, "orig CI lo");
    assert_close(och, 4.812_728_478_073_7, 1e-11, 1e-11, "orig CI hi");
    assert_close(opl, 4.229_689_603_392_96, 1e-11, 1e-11, "orig PI lo");
    assert_close(oph, 4.986_005_075_078_89, 1e-11, 1e-11, "orig PI hi");
    assert_eq!(orig[1].mean_response_set.finite_bounds(), Some((0.0, 0.0)));
    let (zlo, zhi) = orig[1].calibration_set.finite_bounds().unwrap();
    assert!(zlo < 0.0 && zhi > 0.0 && (zlo + zhi).abs() < 1e-14);

    let raw_rep = xy_pts(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0], &[2.9, 3.3, 4.8, 5.2, 6.7, 7.5]);
    let sum_pts = vec![
        DataPoint { sd: Some(0.4_f64.hypot(0.0) / 2.0_f64.sqrt()), n: Some(2.0), ..DataPoint::xy(1.0, 3.1) },
        DataPoint { sd: Some(0.4_f64.hypot(0.0) / 2.0_f64.sqrt()), n: Some(2.0), ..DataPoint::xy(2.0, 5.0) },
        DataPoint { sd: Some(0.8_f64.hypot(0.0) / 2.0_f64.sqrt()), n: Some(2.0), ..DataPoint::xy(3.0, 7.1) },
    ];
    let (cal_raw, cal_sum) = (
        interpolate_x_from_y(&raw_rep, false, ConfidenceLevel::DEFAULT_95, &[5.5], 2).unwrap(),
        interpolate_x_from_y(&sum_pts, false, ConfidenceLevel::DEFAULT_95, &[5.5], 2).unwrap(),
    );
    assert_close(cal_raw[0].estimate, cal_sum[0].estimate, 1e-13, 1e-13, "raw vs sum est");
    assert_close(cal_raw[0].fieller_g, cal_sum[0].fieller_g, 1e-13, 1e-13, "raw vs sum g");
    let ((rl, ru), (sl, su)) = (cal_raw[0].calibration_set.finite_bounds().unwrap(), cal_sum[0].calibration_set.finite_bounds().unwrap());
    assert_close(rl, sl, 1e-12, 1e-12, "raw vs sum lo");
    assert_close(ru, su, 1e-12, 1e-12, "raw vs sum hi");
}

#[test]
fn inverse_x_from_y_fieller_regimes_g_eq_1_and_gt_1_and_edges() {
    let weak_pts = xy_pts(&[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 0.0, 4.0, 1.0, 3.5]);
    let weak_cal = interpolate_x_from_y(&weak_pts, false, ConfidenceLevel::DEFAULT_95, &[2.1, 8.0, 15.0, -10.0], 1).unwrap();
    assert!(weak_cal[0].fieller_g > 20.0);
    assert_eq!(
        (&weak_cal[0].mean_response_set, &weak_cal[0].calibration_set, weak_cal[0].calibration_pi),
        (&InverseIntervalSet::AllReals, &InverseIntervalSet::AllReals, (CiBound::Open, CiBound::Open))
    );

    let InverseIntervalSet::TwoRays { left_upper: l8, right_lower: r8 } = weak_cal[1].mean_response_set else { panic!("expected TwoRays") };
    assert_close(l8, -0.893_513_901_870_457, 1e-11, 1e-11, "y0=8 left");
    assert_close(r8, 5.361_518_221_846_23, 1e-11, 1e-11, "y0=8 right");
    assert!(
        weak_cal[1].mean_response_set.contains(l8) && weak_cal[1].mean_response_set.contains(-10.0) && !weak_cal[1].mean_response_set.contains(3.0)
    );
    assert_eq!(weak_cal[1].calibration_set, InverseIntervalSet::AllReals);

    for (idx, m_lo, m_hi, c_lo, c_hi) in [
        (2, -6.071_680_168_957_39, 8.722_062_495_684_08, -5.322_694_586_061_7, 7.973_076_912_788_39),
        (3, -2.348_959_978_253_02, 11.490_849_423_726_4, -1.541_809_198_474_25, 10.683_698_643_947_7),
    ] {
        let InverseIntervalSet::TwoRays { left_upper: ml, right_lower: mr } = weak_cal[idx].mean_response_set else { panic!("mean TwoRays") };
        let InverseIntervalSet::TwoRays { left_upper: cl, right_lower: cr } = weak_cal[idx].calibration_set else { panic!("cal TwoRays") };
        assert_close(ml, m_lo, 1e-11, 1e-11, "ml");
        assert_close(mr, m_hi, 1e-11, 1e-11, "mr");
        assert_close(cl, c_lo, 1e-11, 1e-11, "cl");
        assert_close(cr, c_hi, 1e-11, 1e-11, "cr");
        assert!(ml < cl && cr < mr && weak_cal[idx].calibration_set.contains(ml) && weak_cal[idx].calibration_set.contains(mr));
    }

    let weak_ols = simple_linear_regression(&weak_pts, &SimpleLinearOptions { unknown_y_values: vec![2.1, 15.0], ..Default::default() }).unwrap();
    for item in &weak_ols.interpolated_x_from_y {
        assert_eq!((item.ci_lower, item.ci_upper, item.pred_lower, item.pred_upper), (CiBound::Open, CiBound::Open, None, None));
    }

    let conf80 = ConfidenceLevel::new(80.0).unwrap();
    let half_pts = xy_pts(&[-2.0, -2.0, 2.0, 2.0], &[-3.5, -0.5, 3.5, 0.5]);
    let half_m1 = interpolate_x_from_y(&half_pts, false, conf80, &[5.0, -5.0, 0.0, 1.0], 1).unwrap();
    assert_close(half_m1[0].fieller_g, 1.0, 1e-13, 1e-13, "exact g == 1");
    let InverseIntervalSet::HalfLine { lower: Some(h0_ml), upper: None } = half_m1[0].mean_response_set else { panic!("HalfLine") };
    let InverseIntervalSet::HalfLine { lower: Some(h0_cl), upper: None } = half_m1[0].calibration_set else { panic!("HalfLine") };
    let (CiBound::Finite(ci_h0), CiBound::Open) = half_m1[0].mean_response_ci else { panic!("Finite/Open") };
    let (CiBound::Finite(pi_h0), CiBound::Open) = half_m1[0].calibration_pi else { panic!("Finite/Open") };
    assert_close(h0_ml, 2.1, 1e-12, 1e-12, "g=1 y0=5 mean lo");
    assert_close(h0_cl, 0.5, 1e-12, 1e-12, "g=1 y0=5 cal lo");
    assert_close(ci_h0, 2.1, 1e-12, 1e-12, "g=1 mean ci");
    assert_close(pi_h0, 0.5, 1e-12, 1e-12, "g=1 cal pi");

    let InverseIntervalSet::HalfLine { lower: None, upper: Some(h1_mh) } = half_m1[1].mean_response_set else { panic!("HalfLine") };
    let InverseIntervalSet::HalfLine { lower: None, upper: Some(h1_ch) } = half_m1[1].calibration_set else { panic!("HalfLine") };
    assert_close(h1_mh, -2.1, 1e-12, 1e-12, "g=1 y0=-5 mean hi");
    assert_close(h1_ch, -0.5, 1e-12, 1e-12, "g=1 y0=-5 cal hi");
    assert_eq!((&half_m1[2].mean_response_set, &half_m1[2].calibration_set), (&InverseIntervalSet::AllReals, &InverseIntervalSet::AllReals));

    let InverseIntervalSet::HalfLine { lower: Some(h3_ml), upper: None } = half_m1[3].mean_response_set else { panic!("HalfLine") };
    let InverseIntervalSet::HalfLine { lower: Some(h3_cl), upper: None } = half_m1[3].calibration_set else { panic!("HalfLine") };
    assert_close(h3_ml, -1.5, 1e-12, 1e-12, "g=1 y0=1 mean lo");
    assert_close(h3_cl, -9.5, 1e-12, 1e-12, "g=1 y0=1 cal lo");

    let half_m4 = interpolate_x_from_y(&half_pts, false, conf80, &[5.0], 4).unwrap();
    let InverseIntervalSet::HalfLine { lower: Some(hm4_lo), upper: None } = half_m4[0].calibration_set else { panic!("HalfLine") };
    assert_close(hm4_lo, 1.7, 1e-12, 1e-12, "g=1 m=4 lo");

    let exact_pts = xy_pts(&[1.0, 2.0, 3.0, 4.0], &[3.0, 5.0, 7.0, 9.0]);
    let exact_cal = interpolate_x_from_y(&exact_pts, false, ConfidenceLevel::DEFAULT_95, &[6.0], 1).unwrap();
    assert_eq!(
        (exact_cal[0].fieller_g, &exact_cal[0].mean_response_set, &exact_cal[0].calibration_set),
        (0.0, &InverseIntervalSet::Bounded { lower: 2.5, upper: 2.5 }, &InverseIntervalSet::Bounded { lower: 2.5, upper: 2.5 })
    );

    let flat_cal = interpolate_x_from_y(&xy_pts(&[1.0, 2.0, 3.0, 4.0], &[5.0; 4]), false, ConfidenceLevel::DEFAULT_95, &[5.0, 6.0], 1).unwrap();
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
            ..Default::default()
        },
    )
    .unwrap();
    for item in edge_ys.interpolated_x_from_y.iter().chain(&edge_ys.interpolated_y_from_x) {
        assert!(
            item.estimate.is_nan()
                && (item.ci_lower, item.ci_upper) == (CiBound::Open, CiBound::Open)
                && item.pred_lower.is_none()
                && item.pred_upper.is_none()
        );
    }
    assert!(matches!(
        interpolate_x_from_y(&exact_pts, false, ConfidenceLevel::DEFAULT_95, &[5.0], 0),
        Err(FitError::InvalidParameter { name: "unknown_replicates", .. })
    ));
}

#[test]
fn restricted_linear_fit_matches_terminal_phase_ols_oracle() {
    let points = xy_pts(&[0.0, 10.0, 12.0, 14.0, 16.0, 20.0], &[90.0, 24.0, 26.0, 30.0, 36.0, -100.0]);
    let mut opts = SimpleLinearOptions { x_range_filter: Some((Some(10.0), Some(16.0))), band_grid_points: 4, ..Default::default() };
    let fit = simple_linear_regression(&points, &opts).unwrap();
    assert_eq!((fit.n, fit.n_excluded_by_range, fit.df_residual), (4, 2, 2));
    assert_close(fit.slope, 2.0, 1e-12, 0.0, "slope");
    assert_close(fit.intercept, 3.0, 1e-12, 0.0, "int");
    assert_close(fit.rss, 4.0, 1e-12, 0.0, "RSS");
    assert_close(fit.r_squared, 20.0 / 21.0, 1e-12, 0.0, "R2");
    assert_eq!(fit.residuals.iter().map(|p| (p.x, p.residual)).collect::<Vec<_>>(), [(10.0, 1.0), (12.0, -1.0), (14.0, -1.0), (16.0, 1.0)]);
    assert_eq!(fit.bands.iter().map(|p| p.x).collect::<Vec<_>>(), [10.0, 12.0, 14.0, 16.0]);

    opts.plot_x_range = Some((None, Some(19.0)));
    let ext = simple_linear_regression(&points, &opts).unwrap();
    assert_eq!(ext.bands.iter().map(|p| p.x).collect::<Vec<_>>(), [10.0, 13.0, 16.0, 19.0]);
    assert_eq!((ext.residuals, ext.r_squared), (fit.residuals, fit.r_squared));
}

#[test]
fn restricted_linear_fit_counts_replicates_and_validates_bounds() {
    let mut points = xy_pts(&[0.0, 11.0, 13.0, 15.0], &[9.0, 25.0, 29.0, 33.0]);
    (points[0].n, points[1].n) = (Some(7.0), Some(3.0));
    let mut opts = SimpleLinearOptions { x_range_filter: Some((Some(10.0), None)), ..Default::default() };

    let fit = simple_linear_regression(&points, &opts).unwrap();
    assert_eq!((fit.n, fit.n_excluded_by_range, fit.df_residual), (5, 7, 3));
    assert_eq!((fit.bands.first().map(|p| p.x), fit.bands.last().map(|p| p.x)), (Some(10.0), Some(15.0)));

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
