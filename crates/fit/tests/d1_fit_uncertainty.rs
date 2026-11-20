#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! D1 reference oracles for fitting uncertainty (`R4.1`):
//! 1. Poisson nonlinear regression (`Y = exp(Intercept + Slope * X)`) with zero counts among
//!    observations and Venzon–Moolgavkar (1988) profile-deviance confidence intervals.
//! 2. Globally shared weighted Gaussian regression (`straight_line` with `Slope` shared globally,
//!    dataset-specific intercepts, unequal dataset sizes, nonzero residuals, and `1/|X|` weights)
//!    verifying all covariance entries and pointwise confidence & prediction limits across both
//!    dataset overlays.
//!
//! Reference values are generated independently at 80 and 120 decimal digits in
//! `crates/fit/tests/reference/fit_uncertainty.py` (`mpmath 1.3.0`) and corroborated with R 4.5.2.
//!
//! # References
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//! - Green, P. J. (1984). Iteratively reweighted least squares for maximum likelihood estimation,
//!   and some robust and resistant alternatives. *JRSS-B* 46(2), 149–192.
//! - McCullagh, P. & Nelder, J. A. (1989). *Generalized Linear Models*, 2nd ed. Chapman & Hall.
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and
//!   Nonlinear Regression*. Oxford University Press.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//! - Venzon, D. J. & Moolgavkar, S. H. (1988). A method for computing profile-likelihood-based
//!   confidence intervals. *JRSS-C (Applied Statistics)* 37(1), 87–94. doi:10.2307/2347496

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, DataSheet, FitMethod, FittedCurveOverlay, NonlinearFitSpec, ParameterRule, ParameterScope, ResultCell,
    ResultSheet, ResultStatus, SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, FitStatus, HookContext, NonlinearFitResult, ParameterNote, SolverConfig, execute_analysis,
    fit_nonlinear, get_builtin_model,
};
use curvebook_math::ConfidenceLevel;
use curvebook_testkit::assert_approx_eq;

// Poisson IRLS stops when relative deviance change < 1e-10 (at most 12 IRLS iterations),
// using central finite-difference Jacobians (h = 1e-6 * max(|theta|, 1)) for DSL models.
// Because deviance is quadratic near the MLE, rel_change < 1e-10 corresponds to ~1e-6
// parameter tolerance (observed max ~4.5e-7 on Intercept and ~9.8e-7 on unweighted RSS).
const POISSON_EST_ABS_TOL: f64 = 2e-6;
const POISSON_DEV_ABS_TOL: f64 = 1e-10;
const POISSON_IC_ABS_TOL: f64 = 1e-7;
const POISSON_COV_ABS_TOL: f64 = 5e-7;

// `compute_deviance_profile_likelihood_cis` refines bracketed crossings with Brent at
// rel_tol = 1e-7, abs_tol = 1e-7 * base_step, reoptimizing nuisance IRLS at each trial point.
// Locally steep deviance near the lower 99% Slope endpoint (~78) amplifies ~1.5e-8 root tol
// to ~1.2e-6 deviance-increment error.
const POISSON_PROFILE_ABS_TOL: f64 = 2e-7;
const POISSON_CUTOFF_ABS_TOL: f64 = 5e-6;

// Weighted straight-line regression uses analytical gradients and solves a convex quadratic
// objective in a single Gauss–Newton trust-region step, matching the exact linear algebra to
// IEEE-754 binary64 rounding accuracy.
const GAUSS_EXACT_ABS_TOL: f64 = 1e-12;
const GAUSS_EXACT_REL_TOL: f64 = 1e-12;

const POISSON_DSL: &str = "Y = exp(Intercept + Slope * X)";
const POISSON_XS: [f64; 10] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
const POISSON_YS: [f64; 10] = [0.0, 1.0, 0.0, 2.0, 1.0, 4.0, 3.0, 7.0, 9.0, 14.0];

// Independent 80/120-digit mpmath references for Fixture 1 (`fit_uncertainty.py`).
const POISSON_INTERCEPT_HAT: f64 = -1.091_463_174_178_690_2;
const POISSON_SLOPE_HAT: f64 = 0.415_847_109_407_824_4;
const POISSON_SE_INTERCEPT: f64 = 0.579_175_044_552_987_7;
const POISSON_SE_SLOPE: f64 = 0.077_251_991_444_878_46;
const POISSON_MU_HAT: [f64; 10] = [
    0.335_724_910_138_585_4,
    0.508_842_843_875_659_2,
    0.771_229_753_718_862_6,
    1.168_917_555_155_008_8,
    1.771_674_710_630_585,
    2.685_246_077_831_152_4,
    4.069_904_286_177_437,
    6.168_567_207_078_522,
    9.349_414_313_618_478,
    14.170_478_341_775_71,
];
const POISSON_DEV_MIN: f64 = 4.459_498_564_481_714;
const POISSON_RSS: f64 = 5.950_628_563_637_789;
const POISSON_AICC: f64 = 35.395_118_235_549_31;
const POISSON_BIC: f64 = 34.286_002_707_251_69;
const POISSON_COV: [[f64; 2]; 2] = [[0.335_443_732_232_955_37, -0.043_085_111_559_294_495], [-0.043_085_111_559_294_495, 0.005_967_870_182_199_575]];

const CHI2_1_95: f64 = 3.841_458_820_694_126;
const POISSON_95_SLOPE_CI: (f64, f64) = (0.273_909_200_668_559_06, 0.578_637_798_303_727_1);
const POISSON_95_INTERCEPT_CI: (f64, f64) = (-2.348_530_252_640_104, -0.063_216_622_655_341_16);

const CHI2_1_99: f64 = 6.634_896_601_021_215;
const POISSON_99_SLOPE_CI: (f64, f64) = (0.232_700_000_407_190_74, 0.634_982_880_941_070_1);
const POISSON_99_INTERCEPT_CI: (f64, f64) = (-2.797_796_879_709_108, 0.219_759_554_947_764_1);

// Fixture 2 inputs (`1/|X|` weighted shared slope + local intercepts across unequal sizes).
const GAUSS_XS_A: [f64; 8] = [1.0, 2.0, 4.0, 6.0, 8.0, 10.0, 13.0, 16.0];
const GAUSS_YS_A: [f64; 8] = [5.4, 7.1, 11.5, 14.2, 19.1, 21.8, 28.9, 34.2];
const GAUSS_XS_B: [f64; 6] = [-11.0, -8.0, -6.0, -4.0, -2.0, -1.0];
const GAUSS_YS_B: [f64; 6] = [-25.4, -18.1, -15.2, -11.0, -6.3, -4.9];

// Independent 80/120-digit mpmath references for Fixture 2 (`fit_uncertainty.py`).
const GAUSS_THETA_HAT: [f64; 3] = [1.947_237_506_072_206, 3.306_245_868_826_435, -2.917_363_748_457_594_5];
const GAUSS_SE: [f64; 3] = [0.034_585_842_240_002_676, 0.198_473_297_192_530_87, 0.189_384_059_962_053_76];
const GAUSS_CI_LO: [f64; 3] = [1.871_114_580_552_689_4, 2.869_409_087_031_217, -3.334_195_253_991_98];
const GAUSS_CI_HI: [f64; 3] = [2.023_360_431_591_722_6, 3.743_082_650_621_653_2, -2.500_532_242_923_209];
const GAUSS_Y_HAT_DS0: [f64; 8] = [
    5.253_483_374_898_641,
    7.200_720_880_970_847,
    11.095_195_893_115_259,
    14.989_670_905_259_672,
    18.884_145_917_404_084,
    22.778_620_929_548_495,
    28.620_333_447_765_113,
    34.462_045_965_981_73,
];
const GAUSS_Y_HAT_DS1: [f64; 6] =
    [-24.336_976_315_251_86, -18.495_263_797_035_243, -14.600_788_784_890_831, -10.706_313_772_746_418, -6.811_838_760_602_006, -4.864_601_254_529_8];
const GAUSS_DF: usize = 11;
const GAUSS_WEIGHTED_SS: f64 = 0.619_244_260_812_100_9;
const GAUSS_RSS: f64 = 3.965_022_190_317_690_8;
const GAUSS_SIGMA2: f64 = 0.056_294_932_801_100_09;
const GAUSS_T_CRIT_95: f64 = 2.200_985_160_091_639_7;
const GAUSS_COV: [[f64; 3]; 3] = [
    [0.001_196_180_483_450_353_6, -0.004_195_119_413_646_315, 0.003_365_452_727_860_319_4],
    [-0.004_195_119_413_646_315, 0.039_391_649_698_474_675, -0.011_802_964_744_610_591],
    [0.003_365_452_727_860_319_4, -0.011_802_964_744_610_591, 0.035_866_322_167_710_78],
];

// Exact 256-point linear grid indices `0, 51, 102, 153, 204, 255` correspond to fractions 0..=5 / 5.
const OVERLAY_INDICES: [usize; 6] = [0, 51, 102, 153, 204, 255];

struct BandPointOracle {
    grid_index: usize,
    x: f64,
    y_hat: f64,
    ci_lower: f64,
    ci_upper: f64,
    pred_lower: f64,
    pred_upper: f64,
}

const DS0_BAND_ORACLES: [BandPointOracle; 6] = [
    BandPointOracle {
        grid_index: 0,
        x: 1.0,
        y_hat: 5.253_483_374_898_641,
        ci_lower: 4.858_545_480_481_681,
        ci_upper: 5.648_421_269_315_602,
        pred_lower: 4.598_741_065_487_202,
        pred_upper: 5.908_225_684_310_08,
    },
    BandPointOracle {
        grid_index: 51,
        x: 4.0,
        y_hat: 11.095_195_893_115_259,
        ci_lower: 10.747_401_359_699_051,
        ci_upper: 11.442_990_426_531_466,
        pred_lower: 9.994_374_725_703_453,
        pred_upper: 12.196_017_060_527_064,
    },
    BandPointOracle {
        grid_index: 102,
        x: 7.0,
        y_hat: 16.936_908_411_331_878,
        ci_lower: 16.500_731_003_944_022,
        ci_upper: 17.373_085_818_719_73,
        pred_lower: 15.488_035_887_703_917,
        pred_upper: 18.385_780_934_959_836,
    },
    BandPointOracle {
        grid_index: 153,
        x: 10.0,
        y_hat: 22.778_620_929_548_495,
        ci_lower: 22.175_425_256_345_612,
        ci_upper: 23.381_816_602_751_38,
        pred_lower: 21.020_508_153_575_374,
        pred_upper: 24.536_733_705_521_616,
    },
    BandPointOracle {
        grid_index: 204,
        x: 13.0,
        y_hat: 28.620_333_447_765_113,
        ci_lower: 27.819_244_248_087_905,
        ci_upper: 29.421_422_647_442_32,
        pred_lower: 26.574_118_876_277_2,
        pred_upper: 30.666_548_019_253_028,
    },
    BandPointOracle {
        grid_index: 255,
        x: 16.0,
        y_hat: 34.462_045_965_981_73,
        ci_lower: 33.450_143_111_097_944,
        ci_upper: 35.473_948_820_865_516,
        pred_lower: 32.140_983_233_457_526,
        pred_upper: 36.783_108_698_505_934,
    },
];

const DS1_BAND_ORACLES: [BandPointOracle; 6] = [
    BandPointOracle {
        grid_index: 0,
        x: -11.0,
        y_hat: -24.336_976_315_251_86,
        ci_lower: -25.055_469_710_414_037,
        ci_upper: -23.618_482_920_089_683,
        pred_lower: -26.212_092_264_479_487,
        pred_upper: -22.461_860_366_024_233,
    },
    BandPointOracle {
        grid_index: 51,
        x: -9.0,
        y_hat: -20.442_501_303_107_45,
        ci_lower: -21.033_820_420_127_455,
        ci_upper: -19.851_182_186_087_442,
        pred_lower: -22.117_034_748_499_22,
        pred_upper: -18.767_967_857_715_68,
    },
    BandPointOracle {
        grid_index: 102,
        x: -7.0,
        y_hat: -16.548_026_290_963_037,
        ci_lower: -17.027_026_745_798_064,
        ci_upper: -16.069_025_836_128_01,
        pred_lower: -18.010_360_826_813_375,
        pred_upper: -15.085_691_755_112_698,
    },
    BandPointOracle {
        grid_index: 153,
        x: -5.0,
        y_hat: -12.653_551_278_818_625,
        ci_lower: -13.047_990_338_279_282,
        ci_upper: -12.259_112_219_357_966,
        pred_lower: -13.886_085_224_826_937,
        pred_upper: -11.421_017_332_810_312,
    },
    BandPointOracle {
        grid_index: 204,
        x: -3.0,
        y_hat: -8.759_076_266_674_212,
        ci_lower: -9.116_959_635_722_637,
        ci_upper: -8.401_192_897_625_789,
        pred_lower: -9.731_812_181_351_069,
        pred_upper: -7.786_340_351_997_357,
    },
    BandPointOracle {
        grid_index: 255,
        x: -1.0,
        y_hat: -4.864_601_254_529_8,
        ci_lower: -5.247_924_145_017_562,
        ci_upper: -4.481_278_364_042_039,
        pred_lower: -5.512_403_685_567_857,
        pred_upper: -4.216_798_823_491_745,
    },
];

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

fn find_param<'a>(fit: &'a NonlinearFitResult, name: &str, ds_idx: Option<usize>) -> &'a curvebook_fit::FittedParameterSummary {
    fit.parameters.iter().find(|p| p.name == name && p.dataset_index == ds_idx).unwrap_or_else(|| panic!("missing parameter {name} ({ds_idx:?})"))
}

fn unwrap_finite_ci(bounds: (CiBound, CiBound), label: &str) -> (f64, f64) {
    match bounds {
        (CiBound::Finite(lo), CiBound::Finite(hi)) => (lo, hi),
        other => panic!("{label}: expected finite profile CI bounds, got {other:?}"),
    }
}

fn poisson_deviance_from_params(xs: &[f64], ys: &[f64], intercept: f64, slope: f64) -> f64 {
    let mut dev = 0.0_f64;
    for (&x, &y) in xs.iter().zip(ys.iter()) {
        let mu = (intercept + slope * x).exp();
        assert!(mu > 0.0 && mu.is_finite(), "Poisson mean must be strictly positive");
        dev += if y == 0.0 { 2.0 * mu } else { 2.0 * (y * (y / mu).ln() - (y - mu)) };
    }
    dev
}

fn independent_poisson_deviance_at_fixed_slope(xs: &[f64], ys: &[f64], slope: f64) -> (f64, f64) {
    // Closed-form nuisance MLE for Poisson log-linear mean:
    // d(ell)/d(Intercept) = sum(y_i) - exp(Intercept) * sum(exp(slope * x_i)) = 0.
    let sum_y: f64 = ys.iter().sum();
    let sum_exp: f64 = xs.iter().map(|&x| (slope * x).exp()).sum();
    let intercept_opt = (sum_y / sum_exp).ln();
    let dev = poisson_deviance_from_params(xs, ys, intercept_opt, slope);
    (intercept_opt, dev)
}

fn verify_poisson_profile_level(
    model: &CompiledDslModel,
    series: &[DatasetSeries],
    confidence: ConfidenceLevel,
    expected_chi2_cut: f64,
    expected_slope_ci: (f64, f64),
    expected_intercept_ci: (f64, f64),
    min_deviance: f64,
) {
    let cfg = SolverConfig { method: FitMethod::PoissonDeviance, confidence, compute_profile_ci: true, ..SolverConfig::default() };
    let fit = fit_nonlinear(model, series, &HookContext::default(), &cfg).unwrap();
    assert_eq!(fit.status, FitStatus::Converged);

    let p_intercept = find_param(&fit, "Intercept", Some(0));
    let p_slope = find_param(&fit, "Slope", Some(0));
    assert!(!p_intercept.notes.contains(&ParameterNote::UnboundedCi) && !p_intercept.notes.contains(&ParameterNote::Ambiguous));
    assert!(!p_slope.notes.contains(&ParameterNote::UnboundedCi) && !p_slope.notes.contains(&ParameterNote::Ambiguous));

    let (s_lo, s_hi) = unwrap_finite_ci((p_slope.ci_lower, p_slope.ci_upper), "Poisson Slope profile CI");
    let (b0_lo, b0_hi) = unwrap_finite_ci((p_intercept.ci_lower, p_intercept.ci_upper), "Poisson Intercept profile CI");

    let tag = format!("{}%", confidence.percent());
    assert_approx_eq(s_lo, expected_slope_ci.0, POISSON_PROFILE_ABS_TOL, 0.0, &format!("{tag} Slope profile lo"));
    assert_approx_eq(s_hi, expected_slope_ci.1, POISSON_PROFILE_ABS_TOL, 0.0, &format!("{tag} Slope profile hi"));
    assert_approx_eq(b0_lo, expected_intercept_ci.0, POISSON_PROFILE_ABS_TOL, 0.0, &format!("{tag} Intercept profile lo"));
    assert_approx_eq(b0_hi, expected_intercept_ci.1, POISSON_PROFILE_ABS_TOL, 0.0, &format!("{tag} Intercept profile hi"));

    // Verify both independent closed-form nuisance deviance increment AND CurveBook constrained refit
    // deviance increment at each reported finite endpoint equal chi2_{1, 1-alpha}.
    for (side, slope_val) in [("lo", s_lo), ("hi", s_hi)] {
        let (_, dev_indep) = independent_poisson_deviance_at_fixed_slope(&POISSON_XS, &POISSON_YS, slope_val);
        assert_approx_eq(
            dev_indep - min_deviance,
            expected_chi2_cut,
            POISSON_CUTOFF_ABS_TOL,
            0.0,
            &format!("{tag} Slope {side} independent nuisance deviance increment"),
        );

        let fixed_cfg = SolverConfig {
            method: FitMethod::PoissonDeviance,
            confidence,
            compute_profile_ci: false,
            parameter_rules: vec![ParameterRule {
                name: "Slope".to_owned(),
                scope: ParameterScope::Constant(slope_val),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            }],
            ..SolverConfig::default()
        };
        let refit = fit_nonlinear(model, series, &HookContext::default(), &fixed_cfg).unwrap();
        assert_approx_eq(
            refit.weighted_ss - fit.weighted_ss,
            expected_chi2_cut,
            POISSON_CUTOFF_ABS_TOL,
            0.0,
            &format!("{tag} Slope {side} CurveBook reoptimized deviance increment"),
        );
    }

    for (side, intercept_val) in [("lo", b0_lo), ("hi", b0_hi)] {
        let fixed_cfg = SolverConfig {
            method: FitMethod::PoissonDeviance,
            confidence,
            compute_profile_ci: false,
            parameter_rules: vec![ParameterRule {
                name: "Intercept".to_owned(),
                scope: ParameterScope::Constant(intercept_val),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            }],
            ..SolverConfig::default()
        };
        let refit = fit_nonlinear(model, series, &HookContext::default(), &fixed_cfg).unwrap();
        assert_approx_eq(
            refit.weighted_ss - fit.weighted_ss,
            expected_chi2_cut,
            POISSON_CUTOFF_ABS_TOL,
            0.0,
            &format!("{tag} Intercept {side} CurveBook reoptimized deviance increment"),
        );
    }
}

fn result_row<'a>(result: &'a ResultSheet, tab_id: &str, label: &str) -> &'a [ResultCell] {
    let tab = result.tabs.iter().find(|t| t.id == tab_id).unwrap_or_else(|| panic!("missing tab {tab_id}"));
    let idx = tab.row_labels.iter().position(|r| r == label).unwrap_or_else(|| panic!("missing row {label} in {tab_id}"));
    &tab.rows[idx]
}

fn assert_num_cell(cell: &ResultCell, expected: f64, abs_tol: f64, rel_tol: f64, label: &str) {
    let ResultCell::Number { value, .. } = *cell else {
        panic!("{label}: expected numeric cell, got {cell:?}");
    };
    assert_approx_eq(value, expected, abs_tol, rel_tol, label);
}

fn assert_ci_cell(cell: &ResultCell, expected: (f64, f64), abs_tol: f64, rel_tol: f64, label: &str) {
    let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } = *cell else {
        panic!("{label}: expected finite ConfidenceInterval cell, got {cell:?}");
    };
    assert_approx_eq(lo, expected.0, abs_tol, rel_tol, &format!("{label} lower"));
    assert_approx_eq(hi, expected.1, abs_tol, rel_tol, &format!("{label} upper"));
}

#[test]
fn poisson_log_linear_profile_deviance_intervals_match_independent_oracle() {
    assert_eq!(POISSON_YS.iter().filter(|&&y| y == 0.0).count(), 2, "fixture must contain zero counts");
    let model = CompiledDslModel::compile("poisson_log_linear", POISSON_DSL).unwrap();
    let series = vec![make_series(0, "Counts", &POISSON_XS, &POISSON_YS)];
    let cfg95 = SolverConfig {
        method: FitMethod::PoissonDeviance,
        confidence: ConfidenceLevel::DEFAULT_95,
        compute_profile_ci: true,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(&model, &series, &HookContext::default(), &cfg95).unwrap();
    assert_eq!(fit.status, FitStatus::Converged);
    assert_eq!((fit.n_obs, fit.k_free, fit.df), (10, 2, 8));

    let p_intercept = find_param(&fit, "Intercept", Some(0));
    let p_slope = find_param(&fit, "Slope", Some(0));
    assert_approx_eq(p_intercept.estimate, POISSON_INTERCEPT_HAT, POISSON_EST_ABS_TOL, 0.0, "Poisson Intercept MLE");
    assert_approx_eq(p_slope.estimate, POISSON_SLOPE_HAT, POISSON_EST_ABS_TOL, 0.0, "Poisson Slope MLE");
    assert_approx_eq(p_intercept.se, POISSON_SE_INTERCEPT, POISSON_COV_ABS_TOL, 0.0, "Poisson Intercept SE");
    assert_approx_eq(p_slope.se, POISSON_SE_SLOPE, POISSON_COV_ABS_TOL, 0.0, "Poisson Slope SE");

    for (i, (&got_mu, &exp_mu)) in fit.datasets[0].y_hat.iter().zip(POISSON_MU_HAT.iter()).enumerate() {
        assert!(got_mu > 0.0, "fitted Poisson mean must be strictly positive at row {i}");
        assert_approx_eq(got_mu, exp_mu, POISSON_EST_ABS_TOL, 1e-8, &format!("Poisson fitted mean mu[{i}]"));
    }
    assert_approx_eq(fit.weighted_ss, POISSON_DEV_MIN, POISSON_DEV_ABS_TOL, 0.0, "Poisson minimized deviance");
    assert_approx_eq(fit.rss, POISSON_RSS, POISSON_EST_ABS_TOL, 0.0, "Poisson unweighted RSS");
    assert_approx_eq(fit.aic_c.unwrap(), POISSON_AICC, POISSON_IC_ABS_TOL, 0.0, "Poisson AICc");
    assert_approx_eq(fit.bic.unwrap(), POISSON_BIC, POISSON_IC_ABS_TOL, 0.0, "Poisson BIC");
    for (r, row) in POISSON_COV.iter().enumerate() {
        for (c, &expected) in row.iter().enumerate() {
            assert_approx_eq(fit.covariance.covariance_matrix[r][c], expected, POISSON_COV_ABS_TOL, 0.0, &format!("Poisson Fisher cov[{r}][{c}]"));
        }
    }

    // Check 95% and 99% finite profile endpoints and chi2_{1, 1-alpha} deviance increments.
    verify_poisson_profile_level(
        &model,
        &series,
        ConfidenceLevel::DEFAULT_95,
        CHI2_1_95,
        POISSON_95_SLOPE_CI,
        POISSON_95_INTERCEPT_CI,
        POISSON_DEV_MIN,
    );
    verify_poisson_profile_level(
        &model,
        &series,
        ConfidenceLevel::PERCENT_99,
        CHI2_1_99,
        POISSON_99_SLOPE_CI,
        POISSON_99_INTERCEPT_CI,
        POISSON_DEV_MIN,
    );

    // Typed ResultSheet path (`execute_analysis`).
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, POISSON_XS.len()).unwrap();
    for (r, (&x, &y)) in POISSON_XS.iter().zip(POISSON_YS.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new("pois-sheet", "Poisson Counts", TableArchetype::Xy(xy)).unwrap();
    let mut nl = NonlinearFitSpec::new("poisson_log_linear");
    nl.custom_dsl = Some(POISSON_DSL.to_owned());
    nl.method = FitMethod::PoissonDeviance;
    nl.profile_likelihood_ci = true;
    nl.emit_covariance_tab = true;
    nl.emit_residuals_tab = true;
    let mut spec = AnalysisSpec::new("pois-fit", "pois-sheet", AnalysisKind::NonlinearRegression(nl)).unwrap();
    spec.confidence_level = ConfidenceLevel::DEFAULT_95;

    let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    assert_eq!(res.status, ResultStatus::UpToDate);
    let intercept_row = result_row(&res, "nonlinear_parameters", "Intercept");
    assert_num_cell(&intercept_row[0], POISSON_INTERCEPT_HAT, POISSON_EST_ABS_TOL, 0.0, "ResultSheet Intercept");
    assert_num_cell(&intercept_row[1], POISSON_SE_INTERCEPT, POISSON_COV_ABS_TOL, 0.0, "ResultSheet Intercept SE");
    assert_ci_cell(&intercept_row[2], POISSON_95_INTERCEPT_CI, POISSON_PROFILE_ABS_TOL, 0.0, "ResultSheet Intercept 95% profile CI");

    let slope_row = result_row(&res, "nonlinear_parameters", "Slope");
    assert_num_cell(&slope_row[0], POISSON_SLOPE_HAT, POISSON_EST_ABS_TOL, 0.0, "ResultSheet Slope");
    assert_num_cell(&slope_row[1], POISSON_SE_SLOPE, POISSON_COV_ABS_TOL, 0.0, "ResultSheet Slope SE");
    assert_ci_cell(&slope_row[2], POISSON_95_SLOPE_CI, POISSON_PROFILE_ABS_TOL, 0.0, "ResultSheet Slope 95% profile CI");

    assert_num_cell(&result_row(&res, "nonlinear_gof", "Degrees of Freedom")[0], 8.0, 0.0, 0.0, "ResultSheet Poisson DF");
    assert_num_cell(
        &result_row(&res, "nonlinear_gof", "Poisson Deviance")[0],
        POISSON_DEV_MIN,
        POISSON_DEV_ABS_TOL,
        0.0,
        "ResultSheet Poisson Deviance",
    );
    assert_num_cell(&result_row(&res, "nonlinear_gof", "Sum of Squares (RSS)")[0], POISSON_RSS, POISSON_EST_ABS_TOL, 0.0, "ResultSheet Poisson RSS");
    assert_num_cell(&result_row(&res, "nonlinear_gof", "AICc")[0], POISSON_AICC, POISSON_IC_ABS_TOL, 0.0, "ResultSheet Poisson AICc");
    assert_num_cell(&result_row(&res, "nonlinear_gof", "BIC")[0], POISSON_BIC, POISSON_IC_ABS_TOL, 0.0, "ResultSheet Poisson BIC");
    assert_approx_eq(res.lookup_scalar("Weighted SS", None).unwrap(), POISSON_DEV_MIN, POISSON_DEV_ABS_TOL, 0.0, "ResultSheet Weighted SS scalar");

    let cov_tab = res.tabs.iter().find(|t| t.id == "nonlinear_covariance").expect("Poisson nonlinear_covariance tab");
    assert_eq!(cov_tab.row_labels, vec!["Intercept", "Slope"]);
    for (r, row) in POISSON_COV.iter().enumerate() {
        for (c, &expected) in row.iter().enumerate() {
            assert_num_cell(&cov_tab.rows[r][c], expected, POISSON_COV_ABS_TOL, 0.0, &format!("ResultSheet Poisson cov[{r}][{c}]"));
        }
    }

    let res_tab = res.tabs.iter().find(|t| t.id == "nonlinear_residuals").unwrap();
    for (i, (row, &exp_mu)) in res_tab.rows.iter().zip(POISSON_MU_HAT.iter()).enumerate() {
        assert_num_cell(&row[0], exp_mu, POISSON_EST_ABS_TOL, 1e-8, &format!("ResultSheet Predicted Y row {}", i + 1));
    }
}

fn check_overlay_bands(overlay: &FittedCurveOverlay, oracles: &[BandPointOracle; 6], ds_label: &str) {
    assert_eq!(overlay.points.len(), 256);
    assert!(overlay.converged && !overlay.ambiguous);
    for oracle in oracles {
        let pt = overlay.points[oracle.grid_index];
        assert_approx_eq(pt.x, oracle.x, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} overlay grid x[{}]", oracle.grid_index));
        assert_approx_eq(pt.y, oracle.y_hat, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} y_hat at x={}", oracle.x));
        let ci_lo = pt.ci_lower.expect("confidence band lower");
        let ci_hi = pt.ci_upper.expect("confidence band upper");
        let pi_lo = pt.pred_lower.expect("prediction band lower");
        let pi_hi = pt.pred_upper.expect("prediction band upper");
        assert_approx_eq(ci_lo, oracle.ci_lower, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} ci_lower at x={}", oracle.x));
        assert_approx_eq(ci_hi, oracle.ci_upper, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} ci_upper at x={}", oracle.x));
        assert_approx_eq(pi_lo, oracle.pred_lower, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} pred_lower at x={}", oracle.x));
        assert_approx_eq(pi_hi, oracle.pred_upper, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} pred_upper at x={}", oracle.x));
        assert!(pi_lo < ci_lo && ci_lo < pt.y && pt.y < ci_hi && ci_hi < pi_hi);
    }
}

fn verify_band_variance_identity(cov: &[[f64; 3]; 3], sigma2: f64, t_crit: f64, ds_idx: usize, overlay: &FittedCurveOverlay) {
    for &idx in &OVERLAY_INDICES {
        let pt = overlay.points[idx];
        let g = if ds_idx == 0 { [pt.x, 1.0, 0.0] } else { [pt.x, 0.0, 1.0] };
        let mut mean_var = 0.0_f64;
        for a in 0..3 {
            for b in 0..3 {
                mean_var += g[a] * cov[a][b] * g[b];
            }
        }
        // Future-observation variance under WeightingScheme::InverseX (w(x) = 1/|X|) is sigma^2 * |x|.
        let pred_var = sigma2 * pt.x.abs() + mean_var;
        let expected_ci_half = t_crit * mean_var.sqrt();
        let expected_pi_half = t_crit * pred_var.sqrt();
        assert_approx_eq(pt.y - pt.ci_lower.unwrap(), expected_ci_half, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "mean-response CI half-width");
        assert_approx_eq(
            pt.ci_upper.unwrap() - pt.y,
            expected_ci_half,
            GAUSS_EXACT_ABS_TOL,
            GAUSS_EXACT_REL_TOL,
            "mean-response CI upper half-width",
        );
        assert_approx_eq(pt.y - pt.pred_lower.unwrap(), expected_pi_half, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "prediction PI half-width");
        assert_approx_eq(pt.pred_upper.unwrap() - pt.y, expected_pi_half, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "prediction PI upper half-width");
    }
}

#[test]
fn shared_weighted_gaussian_covariance_and_pointwise_bands_match_independent_oracle() {
    let model = get_builtin_model("straight_line").unwrap();
    let ds0 = make_series(0, "Series A", &GAUSS_XS_A, &GAUSS_YS_A);
    let ds1 = make_series(1, "Series B", &GAUSS_XS_B, &GAUSS_YS_B);
    let shared_rule =
        ParameterRule { name: "Slope".to_owned(), scope: ParameterScope::SharedGlobal, initial_override: None, lower_bound: None, upper_bound: None };
    let cfg = SolverConfig {
        method: FitMethod::LeastSquares,
        weighting: WeightingScheme::InverseX,
        parameter_rules: vec![shared_rule.clone()],
        confidence: ConfidenceLevel::DEFAULT_95,
        compute_profile_ci: false,
        compute_bands: true,
        compute_prediction_bands: true,
        curve_overlay_segments: 256,
        ..SolverConfig::default()
    };

    let fit = fit_nonlinear(model.as_ref(), &[ds0.clone(), ds1.clone()], &HookContext::default(), &cfg).unwrap();
    assert_eq!(fit.status, FitStatus::Converged);
    assert_eq!((fit.n_obs, fit.k_free, fit.df), (14, 3, GAUSS_DF));
    assert_eq!(fit.datasets[0].y_hat.len(), 8);
    assert_eq!(fit.datasets[1].y_hat.len(), 6);
    assert!(fit.datasets.iter().all(|ds| ds.residuals.iter().all(|&r| r.abs() > 1e-2)));
    assert!(fit.weighted_ss > 0.0 && fit.rss > 0.0);

    assert_approx_eq(fit.weighted_ss, GAUSS_WEIGHTED_SS, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "shared weighted SS");
    assert_approx_eq(fit.rss, GAUSS_RSS, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "shared unweighted RSS");

    let p_slope = find_param(&fit, "Slope", None);
    let p_int_a = find_param(&fit, "YIntercept", Some(0));
    let p_int_b = find_param(&fit, "YIntercept", Some(1));
    assert!(p_slope.notes.contains(&ParameterNote::SharedGlobal));
    assert!(!p_int_a.notes.contains(&ParameterNote::SharedGlobal));
    assert!(!p_int_b.notes.contains(&ParameterNote::SharedGlobal));

    for (((param, (&exp_est, &exp_se)), (&exp_lo, &exp_hi)), label) in [p_slope, p_int_a, p_int_b]
        .into_iter()
        .zip(GAUSS_THETA_HAT.iter().zip(GAUSS_SE.iter()))
        .zip(GAUSS_CI_LO.iter().zip(GAUSS_CI_HI.iter()))
        .zip(["Slope", "YIntercept_A", "YIntercept_B"])
    {
        assert_approx_eq(param.estimate, exp_est, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{label} estimate"));
        assert_approx_eq(param.se, exp_se, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{label} SE"));
        assert_approx_eq(param.symmetric_ci_lower, exp_lo, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{label} CI lower"));
        assert_approx_eq(param.symmetric_ci_upper, exp_hi, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{label} CI upper"));
    }

    assert_approx_eq(fit.datasets[0].params[0], GAUSS_THETA_HAT[0], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "DS0 expanded Slope");
    assert_approx_eq(fit.datasets[0].params[1], GAUSS_THETA_HAT[1], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "DS0 expanded Intercept");
    assert_approx_eq(fit.datasets[1].params[0], GAUSS_THETA_HAT[0], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "DS1 expanded Slope");
    assert_approx_eq(fit.datasets[1].params[1], GAUSS_THETA_HAT[2], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "DS1 expanded Intercept");
    for (i, (&got, &exp)) in fit.datasets[0].y_hat.iter().zip(GAUSS_Y_HAT_DS0.iter()).enumerate() {
        assert_approx_eq(got, exp, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("DS0 y_hat[{i}]"));
    }
    for (i, (&got, &exp)) in fit.datasets[1].y_hat.iter().zip(GAUSS_Y_HAT_DS1.iter()).enumerate() {
        assert_approx_eq(got, exp, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("DS1 y_hat[{i}]"));
    }

    // For a linear Gaussian model, profile-t endpoints match symmetric t-CIs up to Brent root tolerance.
    let cfg_prof = SolverConfig { compute_profile_ci: true, ..cfg.clone() };
    let fit_prof = fit_nonlinear(model.as_ref(), &[ds0, ds1], &HookContext::default(), &cfg_prof).unwrap();
    for (param, (&exp_lo, &exp_hi)) in fit_prof.parameters.iter().zip(GAUSS_CI_LO.iter().zip(GAUSS_CI_HI.iter())) {
        let (p_lo, p_hi) = unwrap_finite_ci((param.ci_lower, param.ci_upper), &param.name);
        assert_approx_eq(p_lo, exp_lo, 1e-6, 1e-6, &format!("linear profile CI lo {}", param.name));
        assert_approx_eq(p_hi, exp_hi, 1e-6, 1e-6, &format!("linear profile CI hi {}", param.name));
    }

    assert_eq!(fit.covariance.numerical_rank, 3);
    assert!(!fit.covariance.ambiguous);
    for (r, row) in GAUSS_COV.iter().enumerate() {
        for (c, &expected) in row.iter().enumerate() {
            assert_approx_eq(
                fit.covariance.covariance_matrix[r][c],
                expected,
                GAUSS_EXACT_ABS_TOL,
                GAUSS_EXACT_REL_TOL,
                &format!("Gaussian cov[{r}][{c}]"),
            );
        }
    }

    let ov0 = fit.datasets[0].curve_overlay.as_ref().expect("DS0 overlay");
    let ov1 = fit.datasets[1].curve_overlay.as_ref().expect("DS1 overlay");
    check_overlay_bands(ov0, &DS0_BAND_ORACLES, "fit_nonlinear DS0");
    check_overlay_bands(ov1, &DS1_BAND_ORACLES, "fit_nonlinear DS1");
    verify_band_variance_identity(&GAUSS_COV, GAUSS_SIGMA2, GAUSS_T_CRIT_95, 0, ov0);
    verify_band_variance_identity(&GAUSS_COV, GAUSS_SIGMA2, GAUSS_T_CRIT_95, 1, ov1);

    // Typed ResultSheet path (`execute_analysis`) on an XY table with unequal dataset sizes.
    let total_rows = GAUSS_XS_A.len() + GAUSS_XS_B.len();
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, total_rows).unwrap();
    xy.data_sets[0].title = "Series A".to_owned();
    xy.data_sets[1].title = "Series B".to_owned();
    for (r, (&x, &y)) in GAUSS_XS_A.iter().zip(GAUSS_YS_A.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    for (idx, (&x, &y)) in GAUSS_XS_B.iter().zip(GAUSS_YS_B.iter()).enumerate() {
        let r = GAUSS_XS_A.len() + idx;
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[1].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new("shared-wls-sheet", "Shared WLS", TableArchetype::Xy(xy)).unwrap();

    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.method = FitMethod::LeastSquares;
    nl.weighting = WeightingScheme::InverseX;
    nl.parameter_rules = vec![shared_rule];
    nl.profile_likelihood_ci = false;
    nl.confidence_bands = true;
    nl.prediction_bands = true;
    nl.emit_covariance_tab = true;
    nl.emit_residuals_tab = true;
    let mut spec = AnalysisSpec::new("shared-wls-fit", "shared-wls-sheet", AnalysisKind::NonlinearRegression(nl)).unwrap();
    spec.confidence_level = ConfidenceLevel::DEFAULT_95;

    let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    assert_eq!(res.status, ResultStatus::UpToDate);
    assert_eq!(res.fitted_curves.len(), 2);

    for ((label, (&exp_est, &exp_se)), (&exp_lo, &exp_hi)) in ["Slope", "YIntercept [Series A]", "YIntercept [Series B]"]
        .into_iter()
        .zip(GAUSS_THETA_HAT.iter().zip(GAUSS_SE.iter()))
        .zip(GAUSS_CI_LO.iter().zip(GAUSS_CI_HI.iter()))
    {
        let row = result_row(&res, "nonlinear_parameters", label);
        assert_num_cell(&row[0], exp_est, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("ResultSheet {label} est"));
        assert_num_cell(&row[1], exp_se, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("ResultSheet {label} SE"));
        assert_ci_cell(&row[2], (exp_lo, exp_hi), GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("ResultSheet {label} CI"));
    }

    assert_num_cell(&result_row(&res, "nonlinear_gof", "Degrees of Freedom")[0], GAUSS_DF as f64, 0.0, 0.0, "ResultSheet DF");
    assert_num_cell(
        &result_row(&res, "nonlinear_gof", "Weighted Sum of Squares")[0],
        GAUSS_WEIGHTED_SS,
        GAUSS_EXACT_ABS_TOL,
        GAUSS_EXACT_REL_TOL,
        "ResultSheet Weighted SS",
    );
    assert_num_cell(
        &result_row(&res, "nonlinear_gof", "Sum of Squares (RSS)")[0],
        GAUSS_RSS,
        GAUSS_EXACT_ABS_TOL,
        GAUSS_EXACT_REL_TOL,
        "ResultSheet RSS",
    );
    assert_approx_eq(
        res.lookup_scalar("Weighted SS", None).unwrap(),
        GAUSS_WEIGHTED_SS,
        GAUSS_EXACT_ABS_TOL,
        GAUSS_EXACT_REL_TOL,
        "scalar Weighted SS",
    );

    let cov_tab = res.tabs.iter().find(|t| t.id == "nonlinear_covariance").expect("nonlinear_covariance tab");
    assert_eq!(cov_tab.row_labels, vec!["Slope", "YIntercept [Series A]", "YIntercept [Series B]"]);
    for (r, row) in GAUSS_COV.iter().enumerate() {
        for (c, &expected) in row.iter().enumerate() {
            assert_num_cell(&cov_tab.rows[r][c], expected, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("ResultSheet cov[{r}][{c}]"));
        }
    }

    let res_tab = res.tabs.iter().find(|t| t.id == "nonlinear_residuals").expect("nonlinear_residuals tab");
    assert_eq!(res_tab.rows.len(), total_rows);
    for (i, &exp_y) in GAUSS_Y_HAT_DS0.iter().chain(GAUSS_Y_HAT_DS1.iter()).enumerate() {
        assert_num_cell(&res_tab.rows[i][0], exp_y, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("ResultSheet predicted Y row {}", i + 1));
    }

    check_overlay_bands(&res.fitted_curves[0], &DS0_BAND_ORACLES, "ResultSheet DS0");
    check_overlay_bands(&res.fitted_curves[1], &DS1_BAND_ORACLES, "ResultSheet DS1");
    verify_band_variance_identity(&GAUSS_COV, GAUSS_SIGMA2, GAUSS_T_CRIT_95, 0, &res.fitted_curves[0]);
    verify_band_variance_identity(&GAUSS_COV, GAUSS_SIGMA2, GAUSS_T_CRIT_95, 1, &res.fitted_curves[1]);
}
