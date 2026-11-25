//! Independent curve-transform references. AUC uses exact piecewise-linear geometry and
//! Bailer (1988) trapezoidal SE weights. Smoothing values are from R 4.5.2 using local cubic
//! least squares (Savitzky & Golay 1964, Gorry 1990) and Cleveland (1979) LOWESS.
//! Reproduce the smoothing values with `reference/curve_smoothing.R`.

use curvebook_doc::SubcolumnMode;
use curvebook_testkit::assert_approx_eq;
use curvebook_transform::{
    AucOptions, AucResult, DerivativeOrder, SmoothMethod, TransformError, compute_auc, compute_auc_with_se, compute_sheet_auc, differentiate_xy,
    integrate_xy, smooth_xy,
};

use crate::common::{Xy3Col, xy_3sub, xy_3sub_clean, xy_single};

// Geometry is exact for piecewise-linear curves. The allowance covers binary64 rounding of
// crossing coordinates and sums over a few short segments.
const AUC_ABS_TOL: f64 = 1e-12;
// The reference and production smoothers use different least-squares routines and weighting
// arithmetic. Allow 2e-11 in y units, including for predictions near zero.
const SMOOTH_ABS_TOL: f64 = 2e-11;

fn assert_irregular_auc(result: &AucResult) {
    // Baseline-relative ordinates at x = [0, 2, 5, 9] are [-2, 3, -1, 2].
    // The three crossings are 4/5, 17/4, and 19/3. Summing triangle areas gives
    // positive 9/5 + 27/8 + 8/3 and negative 4/5 + 3/8 + 2/3.
    assert_approx_eq(result.total_area, 941.0 / 120.0 + 221.0 / 120.0, AUC_ABS_TOL, 0.0, "total unsigned area");
    assert_approx_eq(result.positive_area, 941.0 / 120.0, AUC_ABS_TOL, 0.0, "positive area");
    assert_approx_eq(result.negative_area, 221.0 / 120.0, AUC_ABS_TOL, 0.0, "negative area");
    assert_approx_eq(result.net_area, 6.0, AUC_ABS_TOL, 0.0, "signed net area");
    // Independent SEs [1/8, 1/4, 1/2, 3/8] with trapezoid coefficients [1, 5/2, 7/2, 2].
    assert_approx_eq(result.standard_error.unwrap_or(f64::NAN), (129.0_f64 / 32.0).sqrt(), AUC_ABS_TOL, 0.0, "net area SE");
}

#[test]
fn irregular_auc_matches_triangle_areas_crossings_and_independent_se() {
    let points = [(0.0, 0.0, 0.125), (2.0, 5.0, 0.25), (5.0, 1.0, 0.5), (9.0, 4.0, 0.375)];
    let result = compute_auc_with_se(&points, &AucOptions { baseline: 2.0, ..AucOptions::default() }).unwrap();
    assert_irregular_auc(&result);

    let [first, second] = result.peaks.as_slice() else {
        panic!("expected two positive excursions, got {}", result.peaks.len());
    };
    assert_approx_eq(first.start_x, 4.0 / 5.0, AUC_ABS_TOL, 0.0, "first peak starts at crossing");
    assert_approx_eq(first.end_x, 17.0 / 4.0, AUC_ABS_TOL, 0.0, "first peak ends at crossing");
    assert_approx_eq(first.area, 207.0 / 40.0, AUC_ABS_TOL, 0.0, "first peak area");
    assert_approx_eq(second.start_x, 19.0 / 3.0, AUC_ABS_TOL, 0.0, "second peak starts at crossing");
    assert_approx_eq(second.end_x, 9.0, AUC_ABS_TOL, 0.0, "second peak ends at the last observation");
    assert_approx_eq(second.area, 8.0 / 3.0, AUC_ABS_TOL, 0.0, "second peak area");
}

#[test]
fn sheet_auc_excludes_summary_row_and_converts_sd_to_se() {
    let s = xy_3sub(
        "auc-oracle",
        SubcolumnMode::MeanSdN,
        &[0.0, 2.0, 3.0, 5.0, 9.0],
        &[Xy3Col {
            title: "A",
            triples: &[
                [Some(0.0), Some(0.25), Some(4.0)],
                [Some(5.0), Some(0.75), Some(9.0)],
                [Some(100.0), Some(0.01), Some(4.0)],
                [Some(1.0), Some(2.0), Some(16.0)],
                [Some(4.0), Some(1.875), Some(25.0)],
            ],
            excl_primary: &[false, false, true, false, false],
        }],
    );
    // SD/sqrt(N) gives the four independent SEs used in the analytical reference.
    let result = compute_sheet_auc(&s, 0, &AucOptions { baseline: 2.0, ..AucOptions::default() }).unwrap();
    assert_irregular_auc(&result);
}

fn assert_smoothed<const N: usize>(points: &[(f64, f64); N], method: SmoothMethod, expected: [f64; N]) {
    let actual = smooth_xy(points, method).unwrap();
    assert_eq!(actual.len(), N);
    for ((&(actual_x, actual_y), &(x, _)), expected_y) in actual.iter().zip(points).zip(expected) {
        assert_eq!(actual_x.to_bits(), x.to_bits(), "{method:?} output coordinate");
        assert_approx_eq(actual_y, expected_y, SMOOTH_ABS_TOL, 0.0, &format!("{method:?} at x={x}"));
    }
}

#[test]
fn irregular_savitzky_golay_matches_local_cubic_regression_at_every_point() {
    let points =
        [(0.0, 1.0), (1.0, 3.0), (3.0, 0.0), (4.0, 5.0), (7.0, 2.0), (9.0, 10.0), (10.0, 4.0), (14.0, 13.0), (17.0, 6.0), (18.0, 15.0), (23.0, 11.0)];
    for (method, expected) in [
        (
            SmoothMethod::SavitzkyGolay5,
            [
                1.717_241_379_310_348,
                1.326_436_781_609_197,
                2.510_344_827_586_206,
                2.123_420_796_890_186,
                5.262_846_748_522_055,
                5.638_526_521_464_284,
                7.844_393_592_677_346,
                9.375_611_832_304_745,
                11.289_328_674_637_183,
                10.834_653_668_723_215,
                11.189_873_337_038_245,
            ],
        ),
        (
            SmoothMethod::SavitzkyGolay7,
            [
                1.789_244_590_494_107,
                1.529_611_496_110_986,
                2.062_055_280_976_296,
                2.683_982_827_386_079,
                4.016_099_578_150_451,
                6.564_823_705_143_311,
                6.779_876_857_705_929,
                10.725_052_911_507_074,
                11.165_796_848_707_792,
                11.142_007_809_022_097,
                11.165_239_070_181_03,
            ],
        ),
        (
            SmoothMethod::SavitzkyGolay9,
            [
                2.410_676_882_519_137,
                1.554_319_506_773_515,
                1.317_412_270_203_779,
                1.786_594_951_863_473,
                4.643_083_451_523_416,
                6.065_846_202_698_316,
                7.170_871_859_635_028,
                9.965_221_106_049_745,
                11.343_047_809_563_542,
                11.610_429_818_523_151,
                11.042_860_747_568_37,
            ],
        ),
    ] {
        assert_smoothed(&points, method, expected);
    }
}

#[test]
fn lowess_matches_local_robust_regression_with_an_outlier() {
    let points = [
        (0.0, 2.0),
        (1.0, 4.0),
        (2.0, 3.0),
        (4.0, 6.0),
        (5.0, 5.0),
        (7.0, 8.0),
        (8.0, 42.0),
        (11.0, 10.0),
        (12.0, 9.0),
        (14.0, 13.0),
        (15.0, 12.0),
        (17.0, 13.0),
        (20.0, 17.0),
    ];
    let span = 0.65;
    for (robustness_iters, expected) in [
        (
            0,
            [
                -0.010_657_546_968_800_89,
                1.900_199_851_178_507,
                4.004_298_528_188_856,
                8.861_584_086_683_003,
                11.510_579_444_550_84,
                16.442_904_402_907_8,
                16.426_698_166_310_715,
                16.189_042_056_973_115,
                14.767_015_647_626_14,
                11.867_050_334_377_934,
                12.513_955_443_977_258,
                13.965_267_872_714_259,
                16.235_818_818_780_25,
            ],
        ),
        (
            2,
            [
                2.311_236_130_882_126,
                3.043_408_182_436_375,
                3.782_375_318_253_711,
                5.274_069_307_223_749,
                6.034_231_508_307_229,
                7.370_504_814_905_786,
                7.887_420_820_359_51,
                9.779_894_211_791_717,
                10.396_974_588_354_592,
                11.574_016_484_355_504,
                12.377_599_003_399_693,
                14.030_029_227_958_69,
                16.475_073_797_306_766,
            ],
        ),
    ] {
        assert_smoothed(&points, SmoothMethod::Lowess { span, robustness_iters }, expected);
    }
}

#[test]
fn auc_central_only_meanlowhigh_order_independent_ties_and_total_area() {
    // 1. MeanLowHigh uses central subcolumn 0 only and reports total_area = positive_area + negative_area.
    let sheet_lh = xy_3sub_clean(
        "xy-lh-auc",
        SubcolumnMode::MeanLowHigh,
        &[0.0, 2.0, 4.0],
        &[("A", &[[Some(0.0), Some(99.0), Some(200.0)], [Some(4.0), Some(100.0), Some(500.0)], [Some(0.0), Some(50.0), Some(75.0)]])],
    );
    let auc_lh = compute_sheet_auc(&sheet_lh, 0, &AucOptions { baseline: 2.0, ..AucOptions::default() }).unwrap();
    assert_approx_eq(auc_lh.positive_area, 2.0, AUC_ABS_TOL, 0.0, "lh pos area");
    assert_approx_eq(auc_lh.negative_area, 2.0, AUC_ABS_TOL, 0.0, "lh neg area");
    assert_approx_eq(auc_lh.net_area, 0.0, AUC_ABS_TOL, 0.0, "lh net area");
    assert_approx_eq(auc_lh.total_area, 4.0, AUC_ABS_TOL, 0.0, "lh total unsigned area");
    assert_eq!(auc_lh.standard_error, None);

    // 2. Tied X values collapse order-independently across raw points, SE points, and XY sheets.
    let res_a = compute_auc(&[(0.0, 0.0), (1.0, 2.0), (1.0, 10.0), (3.0, 0.0)], &AucOptions::default()).unwrap();
    let res_b = compute_auc(&[(3.0, 0.0), (1.0, 10.0), (1.0, 2.0), (0.0, 0.0)], &AucOptions::default()).unwrap();
    assert_approx_eq(res_a.total_area, 9.0, AUC_ABS_TOL, 0.0, "tied auc order A");
    assert_approx_eq(res_b.total_area, 9.0, AUC_ABS_TOL, 0.0, "tied auc order B");
    assert_approx_eq(res_a.positive_area, res_b.positive_area, AUC_ABS_TOL, 0.0, "tied auc pos symmetry");

    let se_a = compute_auc_with_se(&[(0.0, 0.0, 0.2), (1.0, 2.0, 0.3), (1.0, 10.0, 0.4), (3.0, 0.0, 0.1)], &AucOptions::default()).unwrap();
    let se_b = compute_auc_with_se(&[(3.0, 0.0, 0.1), (1.0, 10.0, 0.4), (1.0, 2.0, 0.3), (0.0, 0.0, 0.2)], &AucOptions::default()).unwrap();
    assert_approx_eq(se_a.total_area, 9.0, AUC_ABS_TOL, 0.0, "tied se auc");
    assert_approx_eq(se_a.standard_error.unwrap(), se_b.standard_error.unwrap(), AUC_ABS_TOL, 0.0, "tied se symmetry");

    let tied_sheet = xy_single("xy-tied", &[3.0, 1.0, 1.0, 0.0], &[&[0.0, 10.0, 2.0, 0.0]]);
    assert_approx_eq(compute_sheet_auc(&tied_sheet, 0, &AucOptions::default()).unwrap().total_area, 9.0, AUC_ABS_TOL, 0.0, "sheet tied auc");

    // 3. Only 1 distinct finite X rejects cleanly rather than emitting 0.
    assert!(matches!(compute_auc(&[(1.0, 2.0), (1.0, 10.0)], &AucOptions::default()), Err(TransformError::InvalidInput { .. })));
}

#[test]
fn savitzky_golay_nanoscale_non_uniform_uses_relative_grid_tolerance() {
    let xs_nm = [0.0_f64, 1.0, 2.0, 10.0, 50.0, 60.0, 70.0, 80.0, 90.0];
    let pts: Vec<(f64, f64)> = xs_nm.iter().map(|&nm| (nm * 1e-9, 2.5e9 * (nm * 1e-9) + 1.5)).collect();
    for method in [SmoothMethod::SavitzkyGolay5, SmoothMethod::SavitzkyGolay7, SmoothMethod::SavitzkyGolay9] {
        for ((x, y_exp), (sx, sy)) in pts.iter().zip(smooth_xy(&pts, method).unwrap().iter()) {
            assert_approx_eq(*sx, *x, 1e-24, 0.0, "nanoscale x preserved");
            assert_approx_eq(*sy, *y_exp, 1e-9, 1e-11, &format!("{method:?} nanoscale linear exactness"));
        }
    }
}

#[test]
fn curve_transforms_reject_non_finite_inf_and_nan_inputs() {
    for bad in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        for pts in [[(0.0, 1.0), (bad, 2.0), (3.0, 4.0)], [(0.0, 1.0), (1.0, bad), (3.0, 4.0)]] {
            assert!(matches!(compute_auc(&pts, &AucOptions::default()), Err(TransformError::InvalidInput { .. })), "AUC bad={bad:?}");
            for method in
                [SmoothMethod::SavitzkyGolay5, SmoothMethod::MovingAverage { window: 3 }, SmoothMethod::Lowess { span: 0.7, robustness_iters: 1 }]
            {
                assert!(matches!(smooth_xy(&pts, method), Err(TransformError::InvalidInput { .. })), "{method:?} bad={bad:?}");
            }
            assert!(matches!(differentiate_xy(&pts, DerivativeOrder::First), Err(TransformError::InvalidInput { .. })), "diff bad={bad:?}");
            assert!(matches!(integrate_xy(&pts, 0.0), Err(TransformError::InvalidInput { .. })), "integrate bad={bad:?}");
        }
        assert!(
            matches!(
                compute_auc_with_se(&[(0.0, 1.0, 0.1), (1.0, 2.0, bad), (2.0, 3.0, 0.2)], &AucOptions::default()),
                Err(TransformError::InvalidInput { .. })
            ),
            "AUC SE bad={bad:?}"
        );
        for bad_opt in [
            AucOptions { baseline: bad, ..AucOptions::default() },
            AucOptions { min_peak_height: bad, ..AucOptions::default() },
            AucOptions { min_area_fraction: bad, ..AucOptions::default() },
        ] {
            assert!(matches!(compute_auc(&[(0.0, 1.0), (2.0, 3.0)], &bad_opt), Err(TransformError::InvalidSpec { .. })), "AUC opt bad={bad:?}");
        }
        assert!(matches!(integrate_xy(&[(0.0, 1.0), (2.0, 3.0)], bad), Err(TransformError::InvalidSpec { .. })), "integrate baseline bad={bad:?}");
    }
}
