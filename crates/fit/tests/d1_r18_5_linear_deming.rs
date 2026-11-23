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

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, DataSheet, DemingErrorSpec, LinearRegressionSpec, ResultCell, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_fit::{
    DataPoint, DemingOptions, FitError, InverseIntervalSet, SimpleLinearOptions, deming_regression, duplicate_pair_repeatability_sd,
    execute_linear_analysis, simple_linear_regression,
};
use curvebook_math::ConfidenceLevel;
use curvebook_testkit::assert_approx_eq;

fn xy_points(xs: &[f64], ys: &[f64]) -> Vec<DataPoint> {
    xs.iter()
        .zip(ys.iter())
        .enumerate()
        .map(|(row, (&x, &y))| DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row })
        .collect()
}

fn xy_sheet(id: &str, xs: &[f64], ys: &[f64]) -> DataSheet {
    let mut table = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
    table.data_sets[0].title = String::from("Sample");
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        table.x_cells[r][0] = Cell::number(x);
        table.data_sets[0].cells[r][0] = Cell::number(y);
    }
    DataSheet::new(id, "Data", TableArchetype::Xy(table)).unwrap()
}

#[test]
fn d1_r18_5_fixed_point_linear_regression_matches_r_4_5_2() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let ys = [2.4, 4.9, 6.8, 9.3, 11.2, 13.7];
    let pts = xy_points(&xs, &ys);

    let opts = SimpleLinearOptions {
        fixed_point: Some((2.0, 5.0)),
        confidence_level: ConfidenceLevel::DEFAULT_95,
        compute_bands: true,
        band_grid_points: 11, // grid step 0.5: 1.0, 1.5, ..., 4.5 (idx 7), ..., 6.0
        ..SimpleLinearOptions::default()
    };
    let res = simple_linear_regression(&pts, &opts).unwrap();

    assert_eq!(res.n, 6);
    assert_eq!(res.df_residual, 5);
    assert_eq!(res.fixed_point, Some((2.0, 5.0)));
    assert!(!res.force_through_origin);
    assert!(res.r.is_nan());

    // Reference: R 4.5.2 `lm(I(y - 5) ~ 0 + I(x - 2))`
    assert_approx_eq(res.slope, 2.141_935_483_870_968, 1e-12, 1e-12, "fp slope");
    assert_approx_eq(res.intercept, 0.716_129_032_258_064, 1e-12, 1e-12, "fp intercept");
    assert_approx_eq(res.se_slope, 0.051_147_091_549_622, 1e-12, 1e-12, "fp se_slope");
    assert_approx_eq(res.se_intercept, 0.102_294_183_099_244, 1e-12, 1e-12, "fp se_intercept");
    assert_approx_eq(res.ci_slope.0, 2.010_457_699_387_882, 1e-10, 1e-10, "fp ci_slope.0");
    assert_approx_eq(res.ci_slope.1, 2.273_413_268_354_054, 1e-10, 1e-10, "fp ci_slope.1");
    assert_approx_eq(res.ci_intercept.0, 0.453_173_463_291_891, 1e-10, 1e-10, "fp ci_intercept.0");
    assert_approx_eq(res.ci_intercept.1, 0.979_084_601_224_237, 1e-10, 1e-10, "fp ci_intercept.1");
    assert_approx_eq(res.rss, 0.405_483_870_967_742, 1e-12, 1e-12, "fp rss");
    assert_approx_eq(res.sy_x, 0.284_774_953_592_391, 1e-12, 1e-12, "fp sy_x");
    assert_approx_eq(res.r_squared, 0.997_157_092_680_588, 1e-12, 1e-12, "fp r_squared");
    assert_approx_eq(res.f_stat, 1_753.762_927_605_407_2, 1e-9, 1e-9, "fp f_stat");
    assert_approx_eq(res.p_value, 1.464_627_090_721_154e-7, 1e-11, 1e-8, "fp p_value");

    // Line passes exactly through (x0, y0) = (2.0, 5.0)
    assert_approx_eq(res.intercept + res.slope * 2.0, 5.0, 1e-14, 1e-14, "passes through fixed point");

    // X-intercept & exact Fieller (1940) CI
    let x_int = res.x_intercept.expect("x_intercept present when y0 != 0");
    assert_approx_eq(x_int, -0.334_337_349_397_590, 1e-12, 1e-12, "fp x_intercept");
    let (x_lo, x_hi) = res.x_intercept_ci.finite_bounds().expect("bounded Fieller CI");
    assert_approx_eq(x_lo, -0.486_995_872_393_801, 1e-10, 1e-10, "fp x_int_ci lo");
    assert_approx_eq(x_hi, -0.199_336_156_606_488, 1e-10, 1e-10, "fp x_int_ci hi");

    // Pointwise CI & PI band at x = 4.5 (index 7 in 1.0..=6.0 step 0.5)
    let pt45 = &res.bands[7];
    assert_approx_eq(pt45.x, 4.5, 1e-14, 1e-14, "band x=4.5");
    assert_approx_eq(pt45.y, 10.354_838_709_677_42, 1e-12, 1e-12, "band y(4.5)");
    assert_approx_eq(pt45.ci_lower.unwrap(), 10.026_144_248_469_704, 1e-10, 1e-10, "band ci_lo(4.5)");
    assert_approx_eq(pt45.ci_upper.unwrap(), 10.683_533_170_885_136, 1e-10, 1e-10, "band ci_hi(4.5)");
    assert_approx_eq(pt45.pred_lower.unwrap(), 9.552_393_265_414_299, 1e-10, 1e-10, "band pi_lo(4.5)");
    assert_approx_eq(pt45.pred_upper.unwrap(), 11.157_284_153_940_541, 1e-10, 1e-10, "band pi_hi(4.5)");

    // Invalid non-finite fixed point rejected cleanly
    let bad = simple_linear_regression(&pts, &SimpleLinearOptions { fixed_point: Some((f64::NAN, 1.0)), ..SimpleLinearOptions::default() });
    assert!(matches!(bad, Err(FitError::InvalidParameter { name: "fixed_point", .. })));
}

#[test]
fn d1_r18_5_standard_addition_fieller_ci_and_line_extension_matches_r_4_5_2() {
    let xs = [0.0, 5.0, 10.0, 15.0, 20.0, 25.0];
    let ys = [0.182, 0.348, 0.505, 0.678, 0.835, 1.008];
    let pts = xy_points(&xs, &ys);

    let opts = SimpleLinearOptions {
        standard_addition: true,
        extend_line_to_x_intercept: true,
        confidence_level: ConfidenceLevel::DEFAULT_95,
        compute_bands: true,
        band_grid_points: 101,
        ..SimpleLinearOptions::default()
    };
    let res = simple_linear_regression(&pts, &opts).unwrap();

    // Reference: R 4.5.2 `lm(y_sa ~ x_sa)` + Fieller (1940)
    assert_approx_eq(res.slope, 0.032_937_142_857_143, 1e-12, 1e-12, "sa slope");
    assert_approx_eq(res.intercept, 0.180_952_380_952_381, 1e-12, 1e-12, "sa intercept");
    assert_approx_eq(res.sy_x, 0.004_447_042_891_202, 1e-12, 1e-12, "sa sy_x");

    let x_int = res.x_intercept.expect("x_intercept");
    assert_approx_eq(x_int, -5.493_869_997_686_791, 1e-11, 1e-11, "sa x_intercept");
    let (xi_lo, xi_hi) = res.x_intercept_ci.finite_bounds().expect("bounded x_int CI");
    assert_approx_eq(xi_lo, -5.856_711_322_459_036, 1e-10, 1e-10, "sa x_int_ci lo");
    assert_approx_eq(xi_hi, -5.142_591_525_927_385, 1e-10, 1e-10, "sa x_int_ci hi");

    // Standard addition unknown concentration = -x_int with negated, order-swapped limits [-U, -L]
    let conc = res.standard_addition_concentration.expect("std add concentration");
    assert_approx_eq(conc, 5.493_869_997_686_791, 1e-11, 1e-11, "sa unknown_conc");
    let (c_lo, c_hi) = res.standard_addition_ci.expect("std add ci").finite_bounds().expect("bounded conc CI");
    assert_approx_eq(c_lo, 5.142_591_525_927_385, 1e-10, 1e-10, "sa conc_ci lo");
    assert_approx_eq(c_hi, 5.856_711_322_459_036, 1e-10, 1e-10, "sa conc_ci hi");
    assert_approx_eq(c_lo, -xi_hi, 1e-14, 1e-14, "negated order-swapped lower");
    assert_approx_eq(c_hi, -xi_lo, 1e-14, 1e-14, "negated order-swapped upper");

    // Fitted line extended down to X_int (~ -5.4939) where Y = 0
    let first = res.bands.first().expect("bands non-empty");
    let last = res.bands.last().expect("bands non-empty");
    assert_approx_eq(first.x, x_int, 1e-12, 1e-12, "extended line starts at x_intercept");
    assert_approx_eq(first.y, 0.0, 1e-12, 1e-12, "extended line y=0 at x_intercept");
    assert_approx_eq(last.x, 25.0, 1e-12, 1e-12, "extended line ends at data max");

    // End-to-end `execute_linear_analysis` emits summary rows, scalars, and extended overlay
    let sheet = xy_sheet("xy-sa", &xs, &ys);
    let lin_spec = LinearRegressionSpec { standard_addition: true, extend_line_to_x_intercept: true, ..LinearRegressionSpec::default() };
    let spec = AnalysisSpec::new("res-sa", "xy-sa", AnalysisKind::LinearRegression(lin_spec)).unwrap();
    let result = execute_linear_analysis(&spec, &lin_spec, &sheet).unwrap();
    assert_approx_eq(result.fitted_curves[0].points[0].x, x_int, 1e-12, 1e-12, "overlay extended to x_int");
    let conc_scalar = result.scalars.iter().find(|s| s.key == "StandardAdditionConcentration").expect("conc scalar");
    assert_approx_eq(conc_scalar.value, conc, 1e-11, 1e-11, "conc scalar value");
    let tab = result.tabs.iter().find(|t| t.id == "linear_summary").expect("summary tab");
    let ci_idx = tab.row_labels.iter().position(|lbl| lbl == "95% CI Unknown concentration").expect("conc CI row");
    let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } = tab.rows[ci_idx][0] else {
        panic!("expected finite CI cell, got {:?}", tab.rows[ci_idx][0]);
    };
    assert_approx_eq(lo, c_lo, 1e-10, 1e-10, "tab conc CI lo");
    assert_approx_eq(hi, c_hi, 1e-10, 1e-10, "tab conc CI hi");

    // Unbounded Fieller regime (g > 1, non-zero but non-significant slope)
    let noisy_pts = xy_points(&[0.0, 1.0, 2.0, 3.0, 4.0], &[10.0, 12.0, 8.0, 13.0, 10.5]);
    let noisy_res = simple_linear_regression(&noisy_pts, &SimpleLinearOptions { standard_addition: true, ..SimpleLinearOptions::default() }).unwrap();
    assert!(
        matches!(noisy_res.x_intercept_ci, InverseIntervalSet::TwoRays { .. } | InverseIntervalSet::AllReals),
        "expected unbounded Fieller set for non-significant slope, got {:?}",
        noisy_res.x_intercept_ci
    );
    assert_eq!(noisy_res.standard_addition_ci, Some(noisy_res.x_intercept_ci.negate()));
}

#[test]
fn d1_r18_5_deming_unequal_error_specs_match_r_4_5_2() {
    let xs = [10.2, 19.8, 30.5, 39.7, 51.0, 59.4, 70.6, 80.1];
    let ys = [11.5, 22.1, 31.8, 43.2, 54.0, 62.7, 74.9, 85.2];
    let pts = xy_points(&xs, &ys);
    let sheet = xy_sheet("xy-dem", &xs, &ys);

    struct DemingCase {
        err_spec: DemingErrorSpec,
        lambda: f64,
        slope: f64,
        intercept: f64,
        se_slope: f64,
        se_intercept: f64,
        ci_slope: (f64, f64),
        ci_intercept: (f64, f64),
        sy_x: f64,
        x_intercept: f64,
    }
    let cases = [
        DemingCase {
            err_spec: DemingErrorSpec::EqualError,
            lambda: 1.0,
            slope: 1.050_110_069_271_148,
            intercept: 0.749_403_996_541_766,
            se_slope: 0.007_654_453_376_893,
            se_intercept: 0.473_828_542_388_169,
            ci_slope: (1.031_380_296_589_193, 1.068_839_841_953_104),
            ci_intercept: (-0.410_012_679_238_591, 1.908_820_672_322_122),
            sy_x: 0.421_073_013_842_648,
            x_intercept: -0.713_643_282_234_124,
        },
        DemingCase {
            err_spec: DemingErrorSpec::StandardDeviations { sd_x: 1.5, sd_y: 0.75 },
            lambda: 4.0,
            slope: 1.050_257_056_719_994,
            intercept: 0.742_765_675_883_248,
            se_slope: 0.007_656_657_129_841,
            se_intercept: 0.475_357_109_059_475,
            ci_slope: (1.031_521_891_648_832, 1.068_992_221_791_157),
            ci_intercept: (-0.420_391_267_800_392, 1.905_922_619_566_889),
            sy_x: 0.262_485_129_636_526,
            x_intercept: -0.707_222_742_404_553,
        },
        DemingCase {
            err_spec: DemingErrorSpec::VarianceRatio { lambda: 0.25 },
            lambda: 0.25,
            slope: 1.049_954_226_275_462,
            intercept: 0.756_442_255_834_422,
            se_slope: 0.007_652_798_671_149,
            se_intercept: 0.472_225_566_933_626,
            ci_slope: (1.031_228_502_512_601, 1.068_679_950_038_324),
            ci_intercept: (-0.399_052_080_309_118, 1.911_936_591_977_962),
            sy_x: 0.540_589_989_228_589,
            x_intercept: -0.720_452_603_460_414,
        },
    ];

    for c in cases {
        let lam = c.err_spec.resolved_lambda().expect("valid lambda");
        assert_approx_eq(lam, c.lambda, 1e-14, 1e-14, "resolved lambda");
        let dem = deming_regression(&pts, &DemingOptions { variance_ratio_lambda: lam, confidence_level: ConfidenceLevel::DEFAULT_95 }).unwrap();
        assert_approx_eq(dem.slope, c.slope, 1e-12, 1e-12, "deming slope");
        assert_approx_eq(dem.intercept, c.intercept, 1e-11, 1e-11, "deming intercept");
        assert_approx_eq(dem.se_slope, c.se_slope, 1e-11, 1e-11, "deming se_slope");
        assert_approx_eq(dem.se_intercept, c.se_intercept, 1e-10, 1e-10, "deming se_intercept");
        assert_approx_eq(dem.ci_slope.0, c.ci_slope.0, 1e-10, 1e-10, "deming ci_slope.0");
        assert_approx_eq(dem.ci_slope.1, c.ci_slope.1, 1e-10, 1e-10, "deming ci_slope.1");
        assert_approx_eq(dem.ci_intercept.0, c.ci_intercept.0, 1e-10, 1e-10, "deming ci_intercept.0");
        assert_approx_eq(dem.ci_intercept.1, c.ci_intercept.1, 1e-10, 1e-10, "deming ci_intercept.1");
        assert_approx_eq(dem.sy_x, c.sy_x, 1e-12, 1e-12, "deming sy_x");
        assert_approx_eq(dem.x_intercept.unwrap(), c.x_intercept, 1e-11, 1e-11, "deming x_intercept");

        // End-to-end `execute_linear_analysis` wires `DemingErrorSpec`
        let lin_spec = LinearRegressionSpec { deming_model_ii: true, deming_error: c.err_spec, ..LinearRegressionSpec::default() };
        let spec = AnalysisSpec::new("res-dem", "xy-dem", AnalysisKind::LinearRegression(lin_spec)).unwrap();
        let rs = execute_linear_analysis(&spec, &lin_spec, &sheet).unwrap();
        let slope_s = rs.scalars.iter().find(|s| s.key == "Slope").unwrap().value;
        assert_approx_eq(slope_s, c.slope, 1e-12, 1e-12, "result sheet deming slope");
    }

    // Invalid DemingErrorSpec rejected cleanly
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
    let sd = duplicate_pair_repeatability_sd(&pairs).unwrap();
    // Reference: R 4.5.2 `sqrt(sum((a - b)^2) / (2 * 6))`
    assert_approx_eq(sd, 0.321_455_025_366_431, 1e-14, 1e-14, "duplicate pair repeatability sd");

    // Edge cases: identical pairs -> 0, empty -> EmptyInput, non-finite -> InvalidParameter, extreme magnitude overflow-safe
    assert_eq!(duplicate_pair_repeatability_sd(&[(5.0, 5.0), (10.0, 10.0)]).unwrap(), 0.0);
    assert!(matches!(duplicate_pair_repeatability_sd(&[]), Err(FitError::EmptyInput)));
    assert!(matches!(duplicate_pair_repeatability_sd(&[(1.0, f64::NAN)]), Err(FitError::InvalidParameter { name: "pairs", .. })));
    assert!(matches!(duplicate_pair_repeatability_sd(&[(f64::INFINITY, 1.0)]), Err(FitError::InvalidParameter { name: "pairs", .. })));

    let big = duplicate_pair_repeatability_sd(&[(1e200, 1e200 + 2e190), (0.0, 2e190)]).unwrap();
    assert!(big.is_finite() && big > 0.0);
}
