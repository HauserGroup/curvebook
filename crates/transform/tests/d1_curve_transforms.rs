//! Independent curve-transform references. AUC uses exact piecewise-linear geometry and
//! Bailer (1988) trapezoidal SE weights. Smoothing values are from R 4.5.2 using local cubic
//! least squares (Savitzky & Golay 1964, Gorry 1990) and Cleveland (1979) LOWESS.
//! Reproduce the smoothing values with `reference/curve_smoothing.R`.

use curvebook_doc::{Cell, CellCoord, DataSheet, SubcolumnMode, TableArchetype, XyTable};
use curvebook_testkit::assert_approx_eq;
use curvebook_transform::{AucOptions, AucResult, SmoothMethod, TransformError, compute_auc_with_se, compute_sheet_auc, smooth_xy};

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
    let rows = [
        (0.0, 0.0, 0.25, 4.0, false),
        (2.0, 5.0, 0.75, 9.0, false),
        (3.0, 100.0, 0.01, 4.0, true),
        (5.0, 1.0, 2.0, 16.0, false),
        (9.0, 4.0, 1.875, 25.0, false),
    ];
    let xy = XyTable::new(SubcolumnMode::MeanSdN, 1, rows.len()).unwrap();
    let mut sheet = DataSheet::new("auc-oracle", "AUC oracle", TableArchetype::Xy(xy)).unwrap();
    for (row, (x, mean, sd, n, excluded)) in rows.into_iter().enumerate() {
        sheet.set_x_or_time_cell(row, 0, Cell::number(x)).unwrap();
        sheet.set_cell(CellCoord::new(row, 0, 0), Cell { excluded, ..Cell::number(mean) }).unwrap();
        sheet.set_cell(CellCoord::new(row, 0, 1), Cell::number(sd)).unwrap();
        sheet.set_cell(CellCoord::new(row, 0, 2), Cell::number(n)).unwrap();
    }

    // SD/sqrt(N) gives the four independent SEs used in the analytical reference.
    let result = compute_sheet_auc(&sheet, 0, &AucOptions { baseline: 2.0, ..AucOptions::default() }).unwrap();
    assert_irregular_auc(&result);
}

fn assert_smoothed<const N: usize>(points: &[(f64, f64); N], method: SmoothMethod, expected: [f64; N]) -> Result<(), TransformError> {
    let actual = smooth_xy(points, method)?;
    assert_eq!(actual.len(), N);
    for ((&(actual_x, actual_y), &(x, _)), expected_y) in actual.iter().zip(points).zip(expected) {
        assert_eq!(actual_x.to_bits(), x.to_bits(), "{method:?} output coordinate");
        assert_approx_eq(actual_y, expected_y, SMOOTH_ABS_TOL, 0.0, &format!("{method:?} at x={x}"));
    }
    Ok(())
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
        assert_smoothed(&points, method, expected).unwrap();
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
    // Eight neighbors under both CurveBook's rounding and the reference's truncation.
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
        assert_smoothed(&points, SmoothMethod::Lowess { span, robustness_iters }, expected).unwrap();
    }
}
