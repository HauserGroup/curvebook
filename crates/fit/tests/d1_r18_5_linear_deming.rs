//! D1 verified reference oracles for R18.5:
//! - Arbitrary fixed-point simple linear regression (`Y - Y0 = beta1 * (X - X0)`, `df = N - 1`)
//! - Always-on Fieller (1940) *X*-intercept confidence set & Standard Addition unknown concentration (`-X_int`, `[-U, -L]`)
//! - Plot range extension down to *X*-intercept (`extend_line_to_x_intercept`)
//! - Model II Deming regression with unequal errors (`DemingErrorSpec::{EqualError, StandardDeviations, VarianceRatio}`)
//! - Duplicate-pair repeatability SD (`duplicate_pair_repeatability_sd`, Dahlberg 1940; Bland & Altman 1996)
//!
//! D1 Provenance:
//! - Independent reference values verified with `R 4.5.2 (2025-10-31)` via
//!   `crates/fit/tests/reference/r18_5_linear_deming.R` (2026-11-23).
//! - Primary literature: Fieller (1940) *J. R. Stat. Soc. Suppl.* 7:1–64;
//!   Deming (1943) *Statistical Adjustment of Data*;
//!   Linnet (1990) *Stat. Med.* 9:1463–1473 & Linnet (1993) *Clin. Chem.* 39:424–432;
//!   Dahlberg (1940) & Bland & Altman (1996) *BMJ* 313:744.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use curvebook_doc::{AnalysisKind, AnalysisSpec, DemingErrorSpec, LinearRegressionSpec};
use curvebook_fit::{
    DemingOptions, FitError, InverseIntervalSet, SimpleLinearOptions, deming_regression, duplicate_pair_repeatability_sd, execute_linear_analysis,
    simple_linear_regression,
};
use curvebook_math::ConfidenceLevel;
use curvebook_testkit::assert_approx_eq;

use common::{assert_ci_cell, tab_row, xy_pts, xy_sheet_single};

fn assert_oracles(items: &[(f64, f64, f64, f64, &str)]) {
    for &(actual, expected, abs_tol, rel_tol, lbl) in items {
        assert_approx_eq(actual, expected, abs_tol, rel_tol, lbl);
    }
}

#[test]
fn d1_r18_5_fixed_point_linear_regression_matches_r_4_5_2() {
    let pts = xy_pts(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2.4, 4.9, 6.8, 9.3, 11.2, 13.7]);
    let opts = SimpleLinearOptions {
        fixed_point: Some((2.0, 5.0)),
        confidence_level: ConfidenceLevel::DEFAULT_95,
        compute_bands: true,
        band_grid_points: 11,
        ..SimpleLinearOptions::default()
    };
    let res = simple_linear_regression(&pts, &opts).unwrap();
    assert_eq!((res.n, res.df_residual, res.fixed_point, res.force_through_origin), (6, 5, Some((2.0, 5.0)), false));
    assert!(res.r.is_nan());

    let (x_lo, x_hi) = res.x_intercept_ci.finite_bounds().expect("bounded Fieller CI");
    let pt45 = &res.bands[7];
    // Reference: R 4.5.2 `lm(I(y - 5) ~ 0 + I(x - 2))`
    assert_oracles(&[
        (res.slope, 2.141_935_483_870_968, 1e-12, 1e-12, "fp slope"),
        (res.intercept, 0.716_129_032_258_064, 1e-12, 1e-12, "fp intercept"),
        (res.se_slope, 0.051_147_091_549_622, 1e-12, 1e-12, "fp se_slope"),
        (res.se_intercept, 0.102_294_183_099_244, 1e-12, 1e-12, "fp se_intercept"),
        (res.ci_slope.0, 2.010_457_699_387_882, 1e-10, 1e-10, "fp ci_slope.0"),
        (res.ci_slope.1, 2.273_413_268_354_054, 1e-10, 1e-10, "fp ci_slope.1"),
        (res.ci_intercept.0, 0.453_173_463_291_891, 1e-10, 1e-10, "fp ci_intercept.0"),
        (res.ci_intercept.1, 0.979_084_601_224_237, 1e-10, 1e-10, "fp ci_intercept.1"),
        (res.rss, 0.405_483_870_967_742, 1e-12, 1e-12, "fp rss"),
        (res.sy_x, 0.284_774_953_592_391, 1e-12, 1e-12, "fp sy_x"),
        (res.r_squared, 0.997_157_092_680_588, 1e-12, 1e-12, "fp r_squared"),
        (res.f_stat, 1_753.762_927_605_407_2, 1e-9, 1e-9, "fp f_stat"),
        (res.p_value, 1.464_627_090_721_154e-7, 1e-11, 1e-8, "fp p_value"),
        (res.intercept + res.slope * 2.0, 5.0, 1e-14, 1e-14, "passes through fixed point"),
        (res.x_intercept.expect("x_intercept"), -0.334_337_349_397_590, 1e-12, 1e-12, "fp x_intercept"),
        (x_lo, -0.486_995_872_393_801, 1e-10, 1e-10, "fp x_int_ci lo"),
        (x_hi, -0.199_336_156_606_488, 1e-10, 1e-10, "fp x_int_ci hi"),
        (pt45.x, 4.5, 1e-14, 1e-14, "band x=4.5"),
        (pt45.y, 10.354_838_709_677_42, 1e-12, 1e-12, "band y(4.5)"),
        (pt45.ci_lower.unwrap(), 10.026_144_248_469_704, 1e-10, 1e-10, "band ci_lo(4.5)"),
        (pt45.ci_upper.unwrap(), 10.683_533_170_885_136, 1e-10, 1e-10, "band ci_hi(4.5)"),
        (pt45.pred_lower.unwrap(), 9.552_393_265_414_299, 1e-10, 1e-10, "band pi_lo(4.5)"),
        (pt45.pred_upper.unwrap(), 11.157_284_153_940_541, 1e-10, 1e-10, "band pi_hi(4.5)"),
    ]);

    let bad = simple_linear_regression(&pts, &SimpleLinearOptions { fixed_point: Some((f64::NAN, 1.0)), ..SimpleLinearOptions::default() });
    assert!(matches!(bad, Err(FitError::InvalidParameter { name: "fixed_point", .. })));
}

#[test]
fn d1_r18_5_standard_addition_fieller_ci_and_line_extension_matches_r_4_5_2() {
    let (xs, ys) = ([0.0, 5.0, 10.0, 15.0, 20.0, 25.0], [0.182, 0.348, 0.505, 0.678, 0.835, 1.008]);
    let res = simple_linear_regression(
        &xy_pts(&xs, &ys),
        &SimpleLinearOptions {
            standard_addition: true,
            extend_line_to_x_intercept: true,
            confidence_level: ConfidenceLevel::DEFAULT_95,
            compute_bands: true,
            band_grid_points: 101,
            ..SimpleLinearOptions::default()
        },
    )
    .unwrap();

    let x_int = res.x_intercept.expect("x_intercept");
    let (xi_lo, xi_hi) = res.x_intercept_ci.finite_bounds().expect("bounded x_int CI");
    let conc = res.standard_addition_concentration.expect("std add concentration");
    let (c_lo, c_hi) = res.standard_addition_ci.expect("std add ci").finite_bounds().expect("bounded conc CI");
    let (first, last) = (res.bands.first().unwrap(), res.bands.last().unwrap());

    assert_oracles(&[
        (res.slope, 0.032_937_142_857_143, 1e-12, 1e-12, "sa slope"),
        (res.intercept, 0.180_952_380_952_381, 1e-12, 1e-12, "sa intercept"),
        (res.sy_x, 0.004_447_042_891_202, 1e-12, 1e-12, "sa sy_x"),
        (x_int, -5.493_869_997_686_791, 1e-11, 1e-11, "sa x_intercept"),
        (xi_lo, -5.856_711_322_459_036, 1e-10, 1e-10, "sa x_int_ci lo"),
        (xi_hi, -5.142_591_525_927_385, 1e-10, 1e-10, "sa x_int_ci hi"),
        (conc, 5.493_869_997_686_791, 1e-11, 1e-11, "sa unknown_conc"),
        (c_lo, 5.142_591_525_927_385, 1e-10, 1e-10, "sa conc_ci lo"),
        (c_hi, 5.856_711_322_459_036, 1e-10, 1e-10, "sa conc_ci hi"),
        (c_lo, -xi_hi, 1e-14, 1e-14, "negated order-swapped lower"),
        (c_hi, -xi_lo, 1e-14, 1e-14, "negated order-swapped upper"),
        (first.x, x_int, 1e-12, 1e-12, "extended line starts at x_intercept"),
        (first.y, 0.0, 1e-12, 1e-12, "extended line y=0 at x_intercept"),
        (last.x, 25.0, 1e-12, 1e-12, "extended line ends at data max"),
    ]);

    // End-to-end `execute_linear_analysis` emits summary rows, scalars, and extended overlay
    let sheet = xy_sheet_single("xy-sa", "Data", "Sample", &xs, &ys);
    let lin_spec = LinearRegressionSpec { standard_addition: true, extend_line_to_x_intercept: true, ..LinearRegressionSpec::default() };
    let spec = AnalysisSpec::new("res-sa", "xy-sa", AnalysisKind::LinearRegression(lin_spec)).unwrap();
    let result = execute_linear_analysis(&spec, &lin_spec, &sheet).unwrap();
    assert_approx_eq(result.fitted_curves[0].points[0].x, x_int, 1e-12, 1e-12, "overlay extended to x_int");
    assert_approx_eq(result.scalars.iter().find(|s| s.key == "StandardAdditionConcentration").unwrap().value, conc, 1e-11, 1e-11, "conc scalar");
    assert_ci_cell(&tab_row(&result, "linear_summary", "95% CI Unknown concentration")[0], (c_lo, c_hi), 1e-10, 1e-10, "tab conc CI");

    // Unbounded Fieller regime (g > 1, non-zero but non-significant slope)
    let noisy_res = simple_linear_regression(
        &xy_pts(&[0.0, 1.0, 2.0, 3.0, 4.0], &[10.0, 12.0, 8.0, 13.0, 10.5]),
        &SimpleLinearOptions { standard_addition: true, ..SimpleLinearOptions::default() },
    )
    .unwrap();
    assert!(matches!(noisy_res.x_intercept_ci, InverseIntervalSet::TwoRays { .. } | InverseIntervalSet::AllReals));
    assert_eq!(noisy_res.standard_addition_ci, Some(noisy_res.x_intercept_ci.negate()));
}

#[test]
fn d1_r18_5_deming_unequal_error_specs_match_r_4_5_2() {
    let (xs, ys) = ([10.2, 19.8, 30.5, 39.7, 51.0, 59.4, 70.6, 80.1], [11.5, 22.1, 31.8, 43.2, 54.0, 62.7, 74.9, 85.2]);
    let (pts, sheet) = (xy_pts(&xs, &ys), xy_sheet_single("xy-dem", "Data", "Sample", &xs, &ys));

    let cases = [
        (
            DemingErrorSpec::EqualError,
            1.0,
            1.050_110_069_271_148,
            0.749_403_996_541_766,
            0.007_654_453_376_893,
            0.473_828_542_388_169,
            (1.031_380_296_589_193, 1.068_839_841_953_104),
            (-0.410_012_679_238_591, 1.908_820_672_322_122),
            0.421_073_013_842_648,
            -0.713_643_282_234_124,
        ),
        (
            DemingErrorSpec::StandardDeviations { sd_x: 1.5, sd_y: 0.75 },
            4.0,
            1.050_257_056_719_994,
            0.742_765_675_883_248,
            0.007_656_657_129_841,
            0.475_357_109_059_475,
            (1.031_521_891_648_832, 1.068_992_221_791_157),
            (-0.420_391_267_800_392, 1.905_922_619_566_889),
            0.262_485_129_636_526,
            -0.707_222_742_404_553,
        ),
        (
            DemingErrorSpec::VarianceRatio { lambda: 0.25 },
            0.25,
            1.049_954_226_275_462,
            0.756_442_255_834_422,
            0.007_652_798_671_149,
            0.472_225_566_933_626,
            (1.031_228_502_512_601, 1.068_679_950_038_324),
            (-0.399_052_080_309_118, 1.911_936_591_977_962),
            0.540_589_989_228_589,
            -0.720_452_603_460_414,
        ),
    ];

    for (err_spec, lambda, slope, intercept, se_slope, se_int, ci_slope, ci_int, sy_x, x_int) in cases {
        let lam = err_spec.resolved_lambda().expect("valid lambda");
        let dem = deming_regression(&pts, &DemingOptions { variance_ratio_lambda: lam, confidence_level: ConfidenceLevel::DEFAULT_95 }).unwrap();
        let lin_spec = LinearRegressionSpec { deming_model_ii: true, deming_error: err_spec, ..LinearRegressionSpec::default() };
        let spec = AnalysisSpec::new("res-dem", "xy-dem", AnalysisKind::LinearRegression(lin_spec)).unwrap();
        let rs = execute_linear_analysis(&spec, &lin_spec, &sheet).unwrap();

        assert_oracles(&[
            (lam, lambda, 1e-14, 1e-14, "resolved lambda"),
            (dem.slope, slope, 1e-12, 1e-12, "deming slope"),
            (dem.intercept, intercept, 1e-11, 1e-11, "deming intercept"),
            (dem.se_slope, se_slope, 1e-11, 1e-11, "deming se_slope"),
            (dem.se_intercept, se_int, 1e-10, 1e-10, "deming se_intercept"),
            (dem.ci_slope.0, ci_slope.0, 1e-10, 1e-10, "deming ci_slope.0"),
            (dem.ci_slope.1, ci_slope.1, 1e-10, 1e-10, "deming ci_slope.1"),
            (dem.ci_intercept.0, ci_int.0, 1e-10, 1e-10, "deming ci_intercept.0"),
            (dem.ci_intercept.1, ci_int.1, 1e-10, 1e-10, "deming ci_intercept.1"),
            (dem.sy_x, sy_x, 1e-12, 1e-12, "deming sy_x"),
            (dem.x_intercept.unwrap(), x_int, 1e-11, 1e-11, "deming x_intercept"),
            (rs.scalars.iter().find(|s| s.key == "Slope").unwrap().value, slope, 1e-12, 1e-12, "result sheet deming slope"),
        ]);
    }

    let bad_spec = LinearRegressionSpec {
        deming_model_ii: true,
        deming_error: DemingErrorSpec::StandardDeviations { sd_x: 0.0, sd_y: 1.0 },
        ..LinearRegressionSpec::default()
    };
    let spec = AnalysisSpec::new("res-bad-dem", "xy-dem", AnalysisKind::LinearRegression(bad_spec)).unwrap();
    assert!(matches!(execute_linear_analysis(&spec, &bad_spec, &sheet), Err(FitError::InvalidParameter { name: "deming_error", .. })));
}

#[test]
fn d1_r18_5_duplicate_pair_repeatability_sd_matches_r_4_5_2() {
    let pairs = [(10.1, 10.4), (20.3, 19.9), (30.0, 30.5), (40.2, 40.2), (50.8, 50.1), (60.4, 60.9)];
    assert_approx_eq(duplicate_pair_repeatability_sd(&pairs).unwrap(), 0.321_455_025_366_431, 1e-14, 1e-14, "duplicate pair repeatability sd");
    assert_eq!(duplicate_pair_repeatability_sd(&[(5.0, 5.0), (10.0, 10.0)]).unwrap(), 0.0);
    assert!(matches!(duplicate_pair_repeatability_sd(&[]), Err(FitError::EmptyInput)));
    assert!(matches!(duplicate_pair_repeatability_sd(&[(1.0, f64::NAN)]), Err(FitError::InvalidParameter { name: "pairs", .. })));
    assert!(matches!(duplicate_pair_repeatability_sd(&[(f64::INFINITY, 1.0)]), Err(FitError::InvalidParameter { name: "pairs", .. })));
    let big = duplicate_pair_repeatability_sd(&[(1e200, 1e200 + 2e190), (0.0, 2e190)]).unwrap();
    assert!(big.is_finite() && big > 0.0);
}
