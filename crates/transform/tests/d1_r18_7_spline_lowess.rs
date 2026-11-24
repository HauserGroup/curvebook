//! D1 reference oracle tests for R18.7 `FitSplineLowess` (`crates/transform/src/spline.rs`).
//!
//! Reference oracles:
//! - R `4.5.2` (`crates/transform/tests/reference/r18_7_spline_lowess.R`):
//!   - `stats::splinefun(..., method = "natural")` (de Boor 1978)
//!   - `stats::smooth.spline(..., df = ..., all.knots = TRUE)` (Reinsch 1967)
//!   - `stats::lowess(..., f = 0.5, iter = 3, delta = 0)` (Cleveland 1979)
//! - Analytical Akima (1970) 5-point secant slope and step-overshoot suppression oracle.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, ColumnDesignHint, ColumnTable, DataSheet, LowessPreset, ResultCell, ResultStatus,
    ResultTabKind, SplineLowessMethod, SplineLowessSpec, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_transform::{
    TransformError, execute_spline_lowess, fit_akima_subspline, fit_lowess_curve, fit_natural_cubic_spline, fit_point_to_point, fit_smoothing_spline,
    sample_curve,
};

#[test]
fn d1_natural_cubic_spline_matches_r_4_5_2_splinefun() {
    let pts = [(0.0, 1.2), (1.0, 3.8), (2.5, 2.1), (4.0, 5.5), (6.0, 4.9), (9.0, 8.2)];
    let model = fit_natural_cubic_spline(&pts).expect("natural cubic spline fit");

    // R 4.5.2: stats::splinefun(x_nat, y_nat, method = "natural")(xq_nat)
    let expected = [
        (-0.5, -0.622_120_418_848_167),
        (0.0, 1.2),
        (0.5, 2.891_590_314_136_125),
        (1.75, 2.994_150_850_785_34),
        (3.25, 3.517_203_861_256_545),
        (5.0, 5.778_097_731_239_093),
        (7.5, 5.634_195_026_178_01),
        (9.0, 8.2),
        (10.0, 10.114_048_865_619_544),
    ];

    for (x, want) in expected {
        let got = model.eval(x);
        assert!((got - want).abs() < 1e-12, "natural cubic spline at x={x}: got={got:.15}, want={want:.15}");
    }
}

#[test]
fn d1_akima_subspline_analytical_slopes_and_step_overshoot_suppression() {
    // Flat-to-flat step function: Akima (1970) must produce zero overshoot on the flat plateaus
    // [0, 2] and [3, 5], while passing smoothly through every knot.
    let step_pts = [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 10.0), (4.0, 10.0), (5.0, 10.0)];
    let akima = fit_akima_subspline(&step_pts).expect("Akima fit");
    let nat = fit_natural_cubic_spline(&step_pts).expect("natural spline fit");

    for &(x, y) in &step_pts {
        assert!((akima.eval(x) - y).abs() < 1e-13, "Akima must interpolate knot ({x}, {y})");
    }
    // Flat plateaus must have zero oscillation in Akima (unlike natural cubic spline).
    assert!(akima.eval(0.5).abs() < 1e-13, "Akima left plateau overshoot: {}", akima.eval(0.5));
    assert!(akima.eval(1.5).abs() < 1e-13, "Akima pre-step plateau overshoot: {}", akima.eval(1.5));
    assert!((akima.eval(3.5) - 10.0).abs() < 1e-13, "Akima post-step plateau overshoot: {}", akima.eval(3.5));
    assert!((akima.eval(4.5) - 10.0).abs() < 1e-13, "Akima right plateau overshoot: {}", akima.eval(4.5));
    // Verify natural cubic spline genuinely overshoots on this step so the contrast is real.
    assert!(nat.eval(1.5) < -0.2, "natural spline should undershoot before step, got {}", nat.eval(1.5));

    // Analytical Akima 5-point slope check at interior knot x=2 on non-trivial secants:
    // x = [0, 1, 2, 3, 4], y = [0, 1, 3, 2, 6] => secants d = [1, 2, -1, 4]
    // At knot i=2 (x=2): d_{i-2}=1, d_{i-1}=2, d_i=-1, d_{i+1}=4
    // w1 = |4 - (-1)| = 5, w2 = |2 - 1| = 1 => m_2 = (5*2 + 1*(-1)) / 6 = 9/6 = 1.5
    let non_trivial = [(0.0, 0.0), (1.0, 1.0), (2.0, 3.0), (3.0, 2.0), (4.0, 6.0)];
    let m = fit_akima_subspline(&non_trivial).expect("Akima non-trivial");
    let curvebook_transform::FittedCurveModel::PiecewiseCubic { pieces, .. } = &m else {
        panic!("Akima must produce PiecewiseCubic");
    };
    let left_deriv = pieces[1].deriv(2.0);
    let right_deriv = pieces[2].deriv(2.0);
    assert!((left_deriv - 1.5).abs() < 1e-13, "Akima left derivative at x=2: got={left_deriv}, want=1.5");
    assert!((right_deriv - 1.5).abs() < 1e-13, "Akima right derivative at x=2: got={right_deriv}, want=1.5");
}

#[test]
fn d1_smoothing_spline_matches_r_4_5_2_smooth_spline() {
    let pts = [
        (0.0, 0.4),
        (0.5, 1.9),
        (1.2, 3.1),
        (2.0, 3.8),
        (2.8, 4.9),
        (3.5, 4.2),
        (4.4, 3.6),
        (5.1, 4.1),
        (6.0, 5.3),
        (7.2, 6.8),
        (8.5, 7.4),
        (10.0, 8.9),
    ];

    // Exact Reinsch (1967) (I + lambda * Q R^{-1} Q^T)^{-1} y at df = 5.0 (verified in R 4.5.2):
    // lambda = 0.827261215104195, g[0] = 0.897067102810930.
    let ss5_exact = fit_smoothing_spline(&pts, None, Some(5.0)).expect("smoothing spline df=5 exact");
    assert!((ss5_exact.effective_df - 5.0).abs() < 1e-8);
    assert!((ss5_exact.lambda - 0.827_261_215_104_195).abs() < 1e-7, "Reinsch lambda mismatch: {}", ss5_exact.lambda);
    assert!((ss5_exact.model.eval(0.0) - 0.897_067_102_810_930).abs() < 1e-10, "Reinsch g(0) mismatch: {}", ss5_exact.model.eval(0.0));

    // R 4.5.2: stats::smooth.spline(x_sm, y_sm, df = 5.0, all.knots = TRUE)
    // Tolerance 5e-4 accounts for R's Fortran sbart B-spline spar convergence tolerance.
    let expected_ss5 = [
        (0.0, 0.897_080_246_470_879),
        (1.0, 2.556_762_325_821_962),
        (2.5, 4.060_910_756_339_11),
        (4.0, 4.210_570_819_677_765),
        (6.5, 5.696_083_033_859_579),
        (9.0, 7.992_089_680_585_596),
        (10.0, 8.881_332_774_035_632),
    ];
    for (x, want) in expected_ss5 {
        let got = ss5_exact.model.eval(x);
        assert!((got - want).abs() < 5e-4, "smoothing spline df=5 at x={x}: got={got:.12}, want={want:.12}");
    }

    // R 4.5.2: stats::smooth.spline(x_sm, y_sm, df = 8.0, all.knots = TRUE)
    let ss8 = fit_smoothing_spline(&pts, None, Some(8.0)).expect("smoothing spline df=8");
    assert!((ss8.effective_df - 8.0).abs() < 1e-8);
    let expected_ss8 = [
        (0.0, 0.541_976_113_958_901),
        (1.0, 2.728_480_768_169_336),
        (2.5, 4.450_979_271_818_463),
        (4.0, 3.913_845_224_040_784),
        (6.5, 5.929_655_252_913_065),
        (9.0, 7.879_374_868_175_166),
        (10.0, 8.877_349_426_025_985),
    ];
    for (x, want) in expected_ss8 {
        let got = ss8.model.eval(x);
        assert!((got - want).abs() < 5e-4, "smoothing spline df=8 at x={x}: got={got:.12}, want={want:.12}");
    }

    // Boundary limits: df = 2 -> OLS line, df = m -> exact natural cubic spline.
    let ss_lin = fit_smoothing_spline(&pts, None, Some(2.0)).expect("df=2 linear limit");
    assert!((ss_lin.effective_df - 2.0).abs() < 1e-9);
    let ss_interp = fit_smoothing_spline(&pts, None, Some(pts.len() as f64)).expect("df=m interpolant");
    assert!(ss_interp.weighted_rss < 1e-14);
}

#[test]
fn d1_lowess_matches_r_4_5_2_lowess_with_bisquare_outlier_resistance() {
    let pts: Vec<(f64, f64)> = [1.1, 2.0, 2.9, 4.2, 4.8, 6.1, 6.9, 8.2, 8.8, 25.0, 11.1, 11.9, 13.2, 13.8, 15.1, 16.0, 17.2, 17.9, 19.1, 20.0]
        .iter()
        .enumerate()
        .map(|(i, &y)| ((i + 1) as f64, y))
        .collect();

    let model = fit_lowess_curve(&pts, 0.5, 3).expect("LOWESS fit");

    // R 4.5.2: stats::lowess(x_lo, y_lo, f = 0.5, iter = 3, delta = 0.0)$y
    let expected_r = [
        1.038_491_654_397_311,
        2.028_148_775_694_473,
        3.018_787_202_719_621,
        4.009_988_011_118_697,
        5.004_135_504_690_518,
        6.001_425_390_096_808,
        6.996_199_789_123_163,
        7.996_827_305_036_917,
        9.003_595_740_685_071,
        10.006_727_837_963_755,
        11.001_498_617_041_259,
        11.999_647_700_796_126,
        13.011_570_568_205_142,
        14.023_198_764_926_352,
        15.026_153_107_699_66,
        16.030_550_257_736_692,
        17.032_719_717_245_815,
        18.034_994_284_832_23,
        19.035_601_103_813_41,
        20.034_867_907_222_15,
    ];

    for (i, &want) in expected_r.iter().enumerate() {
        let x = (i + 1) as f64;
        let got = model.eval(x);
        // Tolerance 1e-3 accounts for R's Fortran tie-breaking when symmetric left/right neighbors
        // are equidistant at interior integers on a uniform grid, while confirming exact bisquare
        // outlier rejection at x = 10 (y = 25 -> fitted ~= 10.007).
        assert!((got - want).abs() < 1e-3, "LOWESS at x={x}: got={got:.12}, want={want:.12}");
    }
}

#[test]
fn d1_dense_point_to_point_resamples_data_space_linearly_for_log_axes() {
    let pts = [(1.0, 1.0), (100.0, 100.0)];
    let model = fit_point_to_point(&pts).expect("point-to-point fit");
    // Data-space linear interpolation at x = 10 gives y = 10.0 (not log-axis chord midpoint 50.5).
    assert!((model.eval(10.0) - 10.0).abs() < 1e-12);
    let sampled = sample_curve(&model, 1.0, 100.0, 100).expect("sample 100 vertices");
    assert_eq!(sampled.len(), 100);
    for &(x, y) in &sampled {
        assert!((y - x).abs() < 1e-12);
    }
}

#[test]
fn execute_spline_lowess_coords_tab_unknown_interpolation_and_lowess_warnings() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8).expect("xy table");
    xy.x_title = String::from("Conc");
    xy.data_sets[0].title = String::from("Signal");

    // Monotonic standard curve Y = X^2 over X in [0, 5] (6 points) + 2 unknown rows:
    // Row 6: (X = 2.5, blank Y) -> forward Y_hat ~= 6.25
    // Row 7: (blank X, Y = 9.0) -> inverse X_hat == 3.0
    let known = [(0.0, 0.0), (1.0, 1.0), (2.0, 4.0), (3.0, 9.0), (4.0, 16.0), (5.0, 25.0)];
    for (r, &(x, y)) in known.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).expect("set x");
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    xy.set_x_cell(6, Cell::number(2.5)).expect("set unknown x");
    xy.data_sets[0].cells[7][0] = Cell::number(9.0);

    let sheet = DataSheet::new("std-curve", "Standard Curve", TableArchetype::Xy(xy)).expect("sheet");

    // 1. Natural Cubic Spline with curve coordinates tab + unknown interpolation.
    let spec = SplineLowessSpec {
        method: SplineLowessMethod::NaturalCubicSpline,
        segment_count: 101,
        emit_curve_coordinates_tab: true,
        interpolate_unknowns: true,
        ..SplineLowessSpec::default()
    };
    let analysis = AnalysisSpec::new("res-spline", "std-curve", AnalysisKind::FitSplineLowess(spec.clone())).expect("analysis");
    let res = execute_spline_lowess(&sheet, &analysis, &spec).expect("execute spline");
    assert_eq!(res.status, ResultStatus::UpToDate);
    assert_eq!(res.fitted_curves.len(), 1);
    assert_eq!(res.fitted_curves[0].points.len(), 101);
    assert_eq!(res.fitted_curves[0].equation_id, "natural_cubic_spline");

    let coords_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::CurveCoordinates).expect("CurveCoordinates tab");
    assert_eq!(coords_tab.rows.len(), 101);

    let unk_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::InterpolatedUnknowns).expect("InterpolatedUnknowns tab");
    assert_eq!(unk_tab.rows.len(), 2);
    // Forward Y from X = 2.5
    let ResultCell::Number { value: fwd_y, .. } = unk_tab.rows[0][3] else {
        panic!("expected numeric forward interpolation");
    };
    assert!((fwd_y - 6.25).abs() < 0.08, "forward spline Y(2.5) = {fwd_y}");
    // Inverse X from Y = 9.0 (exact knot at x=3.0)
    let ResultCell::Number { value: inv_x, .. } = unk_tab.rows[1][3] else {
        panic!("expected numeric inverse interpolation");
    };
    assert!((inv_x - 3.0).abs() < 1e-10, "inverse spline X(9.0) = {inv_x}");

    // 2. LOWESS on N = 6 (< 20) enforces segment_count >= 4 * N and emits assumption warning.
    let lowess_spec = SplineLowessSpec {
        method: SplineLowessMethod::Lowess,
        segment_count: 10, // smaller than 4 * 6 = 24 -> must bump to 24
        lowess_preset: LowessPreset::Medium,
        ..SplineLowessSpec::default()
    };
    let lowess_analysis = AnalysisSpec::new("res-lowess", "std-curve", AnalysisKind::FitSplineLowess(lowess_spec.clone())).expect("analysis");
    let lowess_res = execute_spline_lowess(&sheet, &lowess_analysis, &lowess_spec).expect("execute lowess");
    assert_eq!(lowess_res.fitted_curves[0].points.len(), 24, "LOWESS must enforce segment_count >= 4 * N");
    assert!(
        lowess_res.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.assumption.contains("LOWESS")),
        "LOWESS must emit assumption warning when N < 20"
    );

    // 3. Reject non-XY table cleanly.
    let col_sheet = DataSheet::new("col-1", "Columns", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::default(), 2, 5).expect("col")))
        .expect("col sheet");
    let err = execute_spline_lowess(&col_sheet, &analysis, &spec).expect_err("must reject Column table");
    assert!(matches!(err, TransformError::IncompatibleTable { .. }));
}
