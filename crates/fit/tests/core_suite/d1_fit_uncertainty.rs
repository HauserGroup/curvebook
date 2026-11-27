#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! D1 reference oracles for fitting uncertainty (`R4.1`):
//! 1. Poisson nonlinear regression (`Y = exp(Intercept + Slope * X)`) with zero counts and
//!    Venzon–Moolgavkar (1988) profile-deviance confidence intervals.
//! 2. Globally shared weighted Gaussian regression (`straight_line` with shared `Slope`,
//!    unequal dataset sizes, and `1/|X|` weights) verifying covariance & pointwise CI/PI bands.
//!
//! Reference values: `crates/fit/tests/reference/fit_uncertainty.py` (`mpmath 1.3.0`, 80/120 digits,
//! corroborated with R 4.5.2). References: Bates & Watts (1988); Green (1984); McCullagh & Nelder (1989);
//! Motulsky & Christopoulos (2004); Seber & Wild (1989); Venzon & Moolgavkar (1988).

use curvebook_doc::{AnalysisKind, AnalysisSpec, FitMethod, FittedCurveOverlay, NonlinearFitSpec, ResultStatus, WeightingScheme};
use curvebook_fit::{CompiledDslModel, DatasetSeries, FitStatus, HookContext, ParameterNote, SolverConfig, execute_analysis, get_builtin_model};
use curvebook_math::ConfidenceLevel;
use curvebook_testkit::assert_approx_eq;

use crate::common::{
    assert_ci_cell, assert_gof_num, assert_num_cell, find_param_exact_ds, find_tab, fit_with, rule_const, rule_shared, series_xy, tab_row,
    unwrap_finite_ci, xy_sheet_single, xy_sheet_staggered,
};

const POISSON_EST_ABS_TOL: f64 = 2e-6;
const POISSON_DEV_ABS_TOL: f64 = 1e-10;
const POISSON_IC_ABS_TOL: f64 = 1e-7;
const POISSON_COV_ABS_TOL: f64 = 5e-7;
const POISSON_PROFILE_ABS_TOL: f64 = 2e-7;
const POISSON_CUTOFF_ABS_TOL: f64 = 5e-6;
const GAUSS_EXACT_ABS_TOL: f64 = 1e-12;
const GAUSS_EXACT_REL_TOL: f64 = 1e-12;

const POISSON_DSL: &str = "Y = exp(Intercept + Slope * X)";
const POISSON_XS: [f64; 10] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
const POISSON_YS: [f64; 10] = [0.0, 1.0, 0.0, 2.0, 1.0, 4.0, 3.0, 7.0, 9.0, 14.0];

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

const GAUSS_XS_A: [f64; 8] = [1.0, 2.0, 4.0, 6.0, 8.0, 10.0, 13.0, 16.0];
const GAUSS_YS_A: [f64; 8] = [5.4, 7.1, 11.5, 14.2, 19.1, 21.8, 28.9, 34.2];
const GAUSS_XS_B: [f64; 6] = [-11.0, -8.0, -6.0, -4.0, -2.0, -1.0];
const GAUSS_YS_B: [f64; 6] = [-25.4, -18.1, -15.2, -11.0, -6.3, -4.9];

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
const OVERLAY_INDICES: [usize; 6] = [0, 51, 102, 153, 204, 255];

struct BandPointOracle {
    grid_index: usize,
    x: f64,
    y_hat: f64,
    ci: (f64, f64),
    pred: (f64, f64),
}

const fn band(grid_index: usize, x: f64, y_hat: f64, ci: (f64, f64), pred: (f64, f64)) -> BandPointOracle {
    BandPointOracle { grid_index, x, y_hat, ci, pred }
}

const DS0_BAND_ORACLES: [BandPointOracle; 6] = [
    band(0, 1.0, 5.253_483_374_898_641, (4.858_545_480_481_681, 5.648_421_269_315_602), (4.598_741_065_487_202, 5.908_225_684_310_08)),
    band(51, 4.0, 11.095_195_893_115_259, (10.747_401_359_699_051, 11.442_990_426_531_466), (9.994_374_725_703_453, 12.196_017_060_527_064)),
    band(102, 7.0, 16.936_908_411_331_878, (16.500_731_003_944_022, 17.373_085_818_719_73), (15.488_035_887_703_917, 18.385_780_934_959_836)),
    band(153, 10.0, 22.778_620_929_548_495, (22.175_425_256_345_612, 23.381_816_602_751_38), (21.020_508_153_575_374, 24.536_733_705_521_616)),
    band(204, 13.0, 28.620_333_447_765_113, (27.819_244_248_087_905, 29.421_422_647_442_32), (26.574_118_876_277_2, 30.666_548_019_253_028)),
    band(255, 16.0, 34.462_045_965_981_73, (33.450_143_111_097_944, 35.473_948_820_865_516), (32.140_983_233_457_526, 36.783_108_698_505_934)),
];

const DS1_BAND_ORACLES: [BandPointOracle; 6] = [
    band(0, -11.0, -24.336_976_315_251_86, (-25.055_469_710_414_037, -23.618_482_920_089_683), (-26.212_092_264_479_487, -22.461_860_366_024_233)),
    band(51, -9.0, -20.442_501_303_107_45, (-21.033_820_420_127_455, -19.851_182_186_087_442), (-22.117_034_748_499_22, -18.767_967_857_715_68)),
    band(102, -7.0, -16.548_026_290_963_037, (-17.027_026_745_798_064, -16.069_025_836_128_01), (-18.010_360_826_813_375, -15.085_691_755_112_698)),
    band(153, -5.0, -12.653_551_278_818_625, (-13.047_990_338_279_282, -12.259_112_219_357_966), (-13.886_085_224_826_937, -11.421_017_332_810_312)),
    band(204, -3.0, -8.759_076_266_674_212, (-9.116_959_635_722_637, -8.401_192_897_625_789), (-9.731_812_181_351_069, -7.786_340_351_997_357)),
    band(255, -1.0, -4.864_601_254_529_8, (-5.247_924_145_017_562, -4.481_278_364_042_039), (-5.512_403_685_567_857, -4.216_798_823_491_745)),
];

fn independent_poisson_deviance_at_fixed_slope(xs: &[f64], ys: &[f64], slope: f64) -> f64 {
    let intercept = (ys.iter().sum::<f64>() / xs.iter().map(|&x| (slope * x).exp()).sum::<f64>()).ln();
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| {
            let mu = (intercept + slope * x).exp();
            if y == 0.0 { 2.0 * mu } else { 2.0 * (y * (y / mu).ln() - (y - mu)) }
        })
        .sum()
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
    let fit = fit_with(model, series, &cfg);
    assert_eq!(fit.status, FitStatus::Converged);

    let tag = format!("{}%", confidence.percent());
    for (pname, exp_ci) in [("Slope", expected_slope_ci), ("Intercept", expected_intercept_ci)] {
        let p = find_param_exact_ds(&fit, Some(0), pname);
        assert!(!p.notes.contains(&ParameterNote::UnboundedCi) && !p.notes.contains(&ParameterNote::Ambiguous));
        let (lo, hi) = unwrap_finite_ci((p.ci_lower, p.ci_upper), &format!("Poisson {pname} profile CI"));
        assert_approx_eq(lo, exp_ci.0, POISSON_PROFILE_ABS_TOL, 0.0, &format!("{tag} {pname} lo"));
        assert_approx_eq(hi, exp_ci.1, POISSON_PROFILE_ABS_TOL, 0.0, &format!("{tag} {pname} hi"));
        for (side, val) in [("lo", lo), ("hi", hi)] {
            if pname == "Slope" {
                let dev_indep = independent_poisson_deviance_at_fixed_slope(&POISSON_XS, &POISSON_YS, val);
                assert_approx_eq(dev_indep - min_deviance, expected_chi2_cut, POISSON_CUTOFF_ABS_TOL, 0.0, &format!("{tag} Slope {side} indep dev"));
            }
            let fixed_cfg = SolverConfig {
                method: FitMethod::PoissonDeviance,
                confidence,
                compute_profile_ci: false,
                parameter_rules: vec![rule_const(pname, val)],
                ..SolverConfig::default()
            };
            let refit = fit_with(model, series, &fixed_cfg);
            assert_approx_eq(
                refit.weighted_ss - fit.weighted_ss,
                expected_chi2_cut,
                POISSON_CUTOFF_ABS_TOL,
                0.0,
                &format!("{tag} {pname} {side} refit dev"),
            );
        }
    }
}

#[test]
fn poisson_log_linear_profile_deviance_intervals_match_independent_oracle() {
    assert_eq!(POISSON_YS.iter().filter(|&&y| y == 0.0).count(), 2);
    let model = CompiledDslModel::compile("poisson_log_linear", POISSON_DSL).unwrap();
    let series = [series_xy(0, "Counts", &POISSON_XS, &POISSON_YS)];
    let cfg95 =
        SolverConfig { method: FitMethod::PoissonDeviance, confidence: ConfidenceLevel::DEFAULT_95, compute_profile_ci: true, ..Default::default() };
    let fit = fit_with(&model, &series, &cfg95);
    assert_eq!((fit.status, fit.n_obs, fit.k_free, fit.df), (FitStatus::Converged, 10, 2, 8));

    let (p_int, p_slp) = (find_param_exact_ds(&fit, Some(0), "Intercept"), find_param_exact_ds(&fit, Some(0), "Slope"));
    assert_approx_eq(p_int.estimate, POISSON_INTERCEPT_HAT, POISSON_EST_ABS_TOL, 0.0, "Intercept MLE");
    assert_approx_eq(p_slp.estimate, POISSON_SLOPE_HAT, POISSON_EST_ABS_TOL, 0.0, "Slope MLE");
    assert_approx_eq(p_int.se, POISSON_SE_INTERCEPT, POISSON_COV_ABS_TOL, 0.0, "Intercept SE");
    assert_approx_eq(p_slp.se, POISSON_SE_SLOPE, POISSON_COV_ABS_TOL, 0.0, "Slope SE");

    for (i, (&got, &exp)) in fit.datasets[0].y_hat.iter().zip(&POISSON_MU_HAT).enumerate() {
        assert!(got > 0.0);
        assert_approx_eq(got, exp, POISSON_EST_ABS_TOL, 1e-8, &format!("mu[{i}]"));
    }
    assert_approx_eq(fit.weighted_ss, POISSON_DEV_MIN, POISSON_DEV_ABS_TOL, 0.0, "deviance");
    assert_approx_eq(fit.rss, POISSON_RSS, POISSON_EST_ABS_TOL, 0.0, "RSS");
    assert_approx_eq(fit.aic_c.unwrap(), POISSON_AICC, POISSON_IC_ABS_TOL, 0.0, "AICc");
    assert_approx_eq(fit.bic.unwrap(), POISSON_BIC, POISSON_IC_ABS_TOL, 0.0, "BIC");
    for (r, row) in POISSON_COV.iter().enumerate() {
        for (c, &exp) in row.iter().enumerate() {
            assert_approx_eq(fit.covariance.covariance_matrix[r][c], exp, POISSON_COV_ABS_TOL, 0.0, &format!("cov[{r}][{c}]"));
        }
    }

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

    let sheet = xy_sheet_single("pois-sheet", "Poisson Counts", "Counts", &POISSON_XS, &POISSON_YS);
    let nl = NonlinearFitSpec {
        custom_dsl: Some(POISSON_DSL.to_owned()),
        method: FitMethod::PoissonDeviance,
        profile_likelihood_ci: true,
        show_standard_errors: Some(true),
        emit_covariance_tab: true,
        emit_residuals_tab: true,
        ..NonlinearFitSpec::new("poisson_log_linear")
    };
    let mut spec = AnalysisSpec::new("pois-fit", "pois-sheet", AnalysisKind::NonlinearRegression(nl)).unwrap();
    spec.confidence_level = ConfidenceLevel::DEFAULT_95;
    let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    assert_eq!(res.status, ResultStatus::UpToDate);

    for (name, est, se, ci) in [
        ("Intercept", POISSON_INTERCEPT_HAT, POISSON_SE_INTERCEPT, POISSON_95_INTERCEPT_CI),
        ("Slope", POISSON_SLOPE_HAT, POISSON_SE_SLOPE, POISSON_95_SLOPE_CI),
    ] {
        let row = tab_row(&res, "nonlinear_parameters", name);
        assert_num_cell(&row[0], est, POISSON_EST_ABS_TOL, 0.0, &format!("RS {name}"));
        assert_num_cell(&row[1], se, POISSON_COV_ABS_TOL, 0.0, &format!("RS {name} SE"));
        assert_ci_cell(&row[2], ci, POISSON_PROFILE_ABS_TOL, 0.0, &format!("RS {name} CI"));
    }
    assert_gof_num(&res, "Degrees of Freedom", 8.0, 0.0, 0.0);
    assert_gof_num(&res, "Poisson Deviance", POISSON_DEV_MIN, POISSON_DEV_ABS_TOL, 0.0);
    assert_gof_num(&res, "Sum of Squares (RSS)", POISSON_RSS, POISSON_EST_ABS_TOL, 0.0);
    assert_gof_num(&res, "AICc", POISSON_AICC, POISSON_IC_ABS_TOL, 0.0);
    assert_gof_num(&res, "BIC", POISSON_BIC, POISSON_IC_ABS_TOL, 0.0);
    assert_approx_eq(res.lookup_scalar("Weighted SS", None).unwrap(), POISSON_DEV_MIN, POISSON_DEV_ABS_TOL, 0.0, "Weighted SS scalar");

    let cov_tab = find_tab(&res, "nonlinear_covariance");
    assert_eq!(cov_tab.row_labels, vec!["Intercept", "Slope"]);
    for (r, row) in POISSON_COV.iter().enumerate() {
        for (c, &exp) in row.iter().enumerate() {
            assert_num_cell(&cov_tab.rows[r][c], exp, POISSON_COV_ABS_TOL, 0.0, &format!("RS cov[{r}][{c}]"));
        }
    }
    for (i, (row, &exp_mu)) in find_tab(&res, "nonlinear_residuals").rows.iter().zip(&POISSON_MU_HAT).enumerate() {
        assert_num_cell(&row[0], exp_mu, POISSON_EST_ABS_TOL, 1e-8, &format!("RS pred row {}", i + 1));
    }
}

fn check_overlay_bands(overlay: &FittedCurveOverlay, oracles: &[BandPointOracle; 6], ds_label: &str) {
    assert_eq!(overlay.points.len(), 256);
    assert!(overlay.converged && !overlay.ambiguous);
    for o in oracles {
        let pt = overlay.points[o.grid_index];
        assert_approx_eq(pt.x, o.x, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} x[{}]", o.grid_index));
        assert_approx_eq(pt.y, o.y_hat, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} y_hat"));
        let (ci_lo, ci_hi, pi_lo, pi_hi) = (pt.ci_lower.unwrap(), pt.ci_upper.unwrap(), pt.pred_lower.unwrap(), pt.pred_upper.unwrap());
        assert_approx_eq(ci_lo, o.ci.0, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} ci_lo"));
        assert_approx_eq(ci_hi, o.ci.1, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} ci_hi"));
        assert_approx_eq(pi_lo, o.pred.0, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} pi_lo"));
        assert_approx_eq(pi_hi, o.pred.1, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{ds_label} pi_hi"));
        assert!(pi_lo < ci_lo && ci_lo < pt.y && pt.y < ci_hi && ci_hi < pi_hi);
    }
}

fn verify_band_variance_identity(cov: &[[f64; 3]; 3], sigma2: f64, t_crit: f64, ds_idx: usize, overlay: &FittedCurveOverlay) {
    for &idx in &OVERLAY_INDICES {
        let pt = overlay.points[idx];
        let g = if ds_idx == 0 { [pt.x, 1.0, 0.0] } else { [pt.x, 0.0, 1.0] };
        let mut mean_var = 0.0;
        for a in 0..3 {
            for b in 0..3 {
                mean_var += g[a] * cov[a][b] * g[b];
            }
        }
        let ci_half = t_crit * mean_var.sqrt();
        let pi_half = t_crit * (sigma2 * pt.x.abs() + mean_var).sqrt();
        assert_approx_eq(pt.y - pt.ci_lower.unwrap(), ci_half, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "CI lo half");
        assert_approx_eq(pt.ci_upper.unwrap() - pt.y, ci_half, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "CI hi half");
        assert_approx_eq(pt.y - pt.pred_lower.unwrap(), pi_half, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "PI lo half");
        assert_approx_eq(pt.pred_upper.unwrap() - pt.y, pi_half, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "PI hi half");
    }
}

#[test]
fn shared_weighted_gaussian_covariance_and_pointwise_bands_match_independent_oracle() {
    let model = get_builtin_model("straight_line").unwrap();
    let (ds0, ds1) = (series_xy(0, "Series A", &GAUSS_XS_A, &GAUSS_YS_A), series_xy(1, "Series B", &GAUSS_XS_B, &GAUSS_YS_B));
    let shared_rule = rule_shared("Slope");
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

    let fit = fit_with(model.as_ref(), &[ds0.clone(), ds1.clone()], &cfg);
    assert_eq!((fit.status, fit.n_obs, fit.k_free, fit.df), (FitStatus::Converged, 14, 3, GAUSS_DF));
    assert_eq!((fit.datasets[0].y_hat.len(), fit.datasets[1].y_hat.len()), (8, 6));
    assert!(fit.datasets.iter().all(|ds| ds.residuals.iter().all(|&r| r.abs() > 1e-2)) && fit.weighted_ss > 0.0 && fit.rss > 0.0);
    assert_approx_eq(fit.weighted_ss, GAUSS_WEIGHTED_SS, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "weighted SS");
    assert_approx_eq(fit.rss, GAUSS_RSS, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "RSS");

    let (p_slope, p_a, p_b) = (
        find_param_exact_ds(&fit, None, "Slope"),
        find_param_exact_ds(&fit, Some(0), "YIntercept"),
        find_param_exact_ds(&fit, Some(1), "YIntercept"),
    );
    assert!(
        p_slope.notes.contains(&ParameterNote::SharedGlobal)
            && !p_a.notes.contains(&ParameterNote::SharedGlobal)
            && !p_b.notes.contains(&ParameterNote::SharedGlobal)
    );
    for (i, (p, lbl)) in [p_slope, p_a, p_b].into_iter().zip(["Slope", "YIntercept_A", "YIntercept_B"]).enumerate() {
        assert_approx_eq(p.estimate, GAUSS_THETA_HAT[i], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{lbl} est"));
        assert_approx_eq(p.se, GAUSS_SE[i], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{lbl} SE"));
        assert_approx_eq(p.symmetric_ci_lower, GAUSS_CI_LO[i], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{lbl} CI lo"));
        assert_approx_eq(p.symmetric_ci_upper, GAUSS_CI_HI[i], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("{lbl} CI hi"));
    }
    assert_approx_eq(fit.datasets[0].params[0], GAUSS_THETA_HAT[0], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "DS0 Slope");
    assert_approx_eq(fit.datasets[0].params[1], GAUSS_THETA_HAT[1], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "DS0 Int");
    assert_approx_eq(fit.datasets[1].params[0], GAUSS_THETA_HAT[0], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "DS1 Slope");
    assert_approx_eq(fit.datasets[1].params[1], GAUSS_THETA_HAT[2], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "DS1 Int");
    for (ds_i, exp_yhat) in [GAUSS_Y_HAT_DS0.as_slice(), GAUSS_Y_HAT_DS1.as_slice()].into_iter().enumerate() {
        for (i, (&got, &exp)) in fit.datasets[ds_i].y_hat.iter().zip(exp_yhat).enumerate() {
            assert_approx_eq(got, exp, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("DS{ds_i} y_hat[{i}]"));
        }
    }

    let fit_prof = fit_with(model.as_ref(), &[ds0, ds1], &SolverConfig { compute_profile_ci: true, ..cfg });
    for (i, p) in fit_prof.parameters.iter().enumerate() {
        let (p_lo, p_hi) = unwrap_finite_ci((p.ci_lower, p.ci_upper), &p.name);
        assert_approx_eq(p_lo, GAUSS_CI_LO[i], 1e-6, 1e-6, &format!("prof lo {}", p.name));
        assert_approx_eq(p_hi, GAUSS_CI_HI[i], 1e-6, 1e-6, &format!("prof hi {}", p.name));
    }

    assert_eq!((fit.covariance.numerical_rank, fit.covariance.ambiguous), (3, false));
    for (r, row) in GAUSS_COV.iter().enumerate() {
        for (c, &exp) in row.iter().enumerate() {
            assert_approx_eq(fit.covariance.covariance_matrix[r][c], exp, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("cov[{r}][{c}]"));
        }
    }
    let (ov0, ov1) = (fit.datasets[0].curve_overlay.as_ref().unwrap(), fit.datasets[1].curve_overlay.as_ref().unwrap());
    check_overlay_bands(ov0, &DS0_BAND_ORACLES, "fit DS0");
    check_overlay_bands(ov1, &DS1_BAND_ORACLES, "fit DS1");
    verify_band_variance_identity(&GAUSS_COV, GAUSS_SIGMA2, GAUSS_T_CRIT_95, 0, ov0);
    verify_band_variance_identity(&GAUSS_COV, GAUSS_SIGMA2, GAUSS_T_CRIT_95, 1, ov1);

    let sheet =
        xy_sheet_staggered("shared-wls-sheet", "Shared WLS", &[("Series A", &GAUSS_XS_A, &GAUSS_YS_A), ("Series B", &GAUSS_XS_B, &GAUSS_YS_B)]);
    let nl = NonlinearFitSpec {
        method: FitMethod::LeastSquares,
        weighting: WeightingScheme::InverseX,
        parameter_rules: vec![shared_rule],
        profile_likelihood_ci: false,
        confidence_bands: true,
        prediction_bands: true,
        emit_covariance_tab: true,
        emit_residuals_tab: true,
        ..NonlinearFitSpec::new("straight_line")
    };
    let mut spec = AnalysisSpec::new("shared-wls-fit", "shared-wls-sheet", AnalysisKind::NonlinearRegression(nl)).unwrap();
    spec.confidence_level = ConfidenceLevel::DEFAULT_95;
    let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    assert_eq!(res.status, ResultStatus::UpToDate);
    assert_eq!(res.fitted_curves.len(), 2);

    for (i, label) in ["Slope", "YIntercept [Series A]", "YIntercept [Series B]"].into_iter().enumerate() {
        let row = tab_row(&res, "nonlinear_parameters", label);
        assert_num_cell(&row[0], GAUSS_THETA_HAT[i], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("RS {label} est"));
        assert_num_cell(&row[1], GAUSS_SE[i], GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("RS {label} SE"));
        assert_ci_cell(&row[2], (GAUSS_CI_LO[i], GAUSS_CI_HI[i]), GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("RS {label} CI"));
    }
    assert_gof_num(&res, "Degrees of Freedom", GAUSS_DF as f64, 0.0, 0.0);
    assert_gof_num(&res, "Weighted Sum of Squares", GAUSS_WEIGHTED_SS, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL);
    assert_gof_num(&res, "Sum of Squares (RSS)", GAUSS_RSS, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL);
    assert_approx_eq(res.lookup_scalar("Weighted SS", None).unwrap(), GAUSS_WEIGHTED_SS, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, "Weighted SS");

    let cov_tab = find_tab(&res, "nonlinear_covariance");
    assert_eq!(cov_tab.row_labels, vec!["Slope", "YIntercept [Series A]", "YIntercept [Series B]"]);
    for (r, row) in GAUSS_COV.iter().enumerate() {
        for (c, &exp) in row.iter().enumerate() {
            assert_num_cell(&cov_tab.rows[r][c], exp, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("RS cov[{r}][{c}]"));
        }
    }
    let res_tab = find_tab(&res, "nonlinear_residuals");
    assert_eq!(res_tab.rows.len(), GAUSS_XS_A.len() + GAUSS_XS_B.len());
    for (i, &exp_y) in GAUSS_Y_HAT_DS0.iter().chain(&GAUSS_Y_HAT_DS1).enumerate() {
        assert_num_cell(&res_tab.rows[i][0], exp_y, GAUSS_EXACT_ABS_TOL, GAUSS_EXACT_REL_TOL, &format!("RS pred[{i}]"));
    }
    check_overlay_bands(&res.fitted_curves[0], &DS0_BAND_ORACLES, "RS DS0");
    check_overlay_bands(&res.fitted_curves[1], &DS1_BAND_ORACLES, "RS DS1");
    verify_band_variance_identity(&GAUSS_COV, GAUSS_SIGMA2, GAUSS_T_CRIT_95, 0, &res.fitted_curves[0]);
    verify_band_variance_identity(&GAUSS_COV, GAUSS_SIGMA2, GAUSS_T_CRIT_95, 1, &res.fitted_curves[1]);
}

mod hougaard_skewness_uncertainty {
    use curvebook_doc::{ChecklistStatus, FitMethod, IdentifiabilityMode, NonlinearFitSpec, ResultCell, ResultStatus};
    use curvebook_fit::diagnostics::compute_hougaard_skewness;
    use curvebook_fit::{
        DataPoint, EquationCategory, FitStatus, HookContext, ParameterNote, ParametricModel, SolverConfig, fit_nonlinear, get_builtin_model,
    };
    use curvebook_math::linalg::DenseMatrix;
    use curvebook_testkit::assert_approx_eq;

    use crate::common::{
        cell_num, cfg_fast, dsl_compile, exec_nl, find_param, find_tab, fit_with, nl_fast, rule_local_bounds, rule_shared, series_xy, tab_row,
        xy_sheet, xy_sheet_single,
    };

    fn hougaard_col_index(tab: &curvebook_doc::ResultTab) -> usize {
        tab.column_headers.iter().position(|h| h == "Hougaard |g1|").expect("nonlinear_parameters tab must contain `Hougaard |g1|` header")
    }

    struct PerturbFailModel {
        params: [String; 1],
    }

    impl PerturbFailModel {
        fn new() -> Self {
            Self { params: ["A".to_owned()] }
        }
    }

    impl ParametricModel for PerturbFailModel {
        fn id(&self) -> &str {
            "perturb_fail_test"
        }

        fn name(&self) -> &str {
            "Perturb Fail Test"
        }

        fn category(&self) -> EquationCategory {
            EquationCategory::UserDefined
        }

        fn param_names(&self) -> &[String] {
            &self.params
        }

        fn eval(&self, x: &[f64], params: &[f64], _data_set_index: usize) -> Option<f64> {
            let a = params[0];
            if (a - 2.0).abs() >= 1e-4 {
                return None;
            }
            Some(a * a * x[0])
        }

        fn grad(&self, x: &[f64], params: &[f64], _data_set_index: usize, out: &mut [f64]) -> bool {
            out[0] = 2.0 * params[0] * x[0];
            true
        }

        fn initial_guess(&self, _points: &[DataPoint]) -> Vec<f64> {
            vec![2.0]
        }
    }

    #[test]
    fn r19_6_compute_hougaard_skewness_returns_nan_on_uncomputed_and_exact_signed_g1() {
        // Analytical oracle for f(x; theta) = theta^2 * x with unit weights:
        //   f' = 2*theta*x, f'' = 2*x, J^T J = 4*theta^2*S2, L = 1/(4*theta^2*S2),
        //   W_000 = 4*theta*S2  =>  g1 = -sigma * L^{3/2} * (3 * W_000) = -1.5 * sigma * sgn(theta) / (theta^2 * sqrt(S2)).
        let xs = [1.0_f64, 2.0, 3.0];
        let s2: f64 = xs.iter().map(|x| x * x).sum();
        let sqrt_w = [1.0_f64; 3];

        for (theta_val, sigma, expected_sign) in [(1.0_f64, 4.0_f64, -1.0_f64), (-1.0_f64, 4.0_f64, 1.0_f64)] {
            let j_mat = DenseMatrix::from_row_major(3, 1, xs.iter().map(|&x| 2.0 * theta_val * x).collect()).unwrap();
            let cov_u = DenseMatrix::from_row_major(1, 1, vec![1.0 / (4.0 * theta_val * theta_val * s2)]).unwrap();
            let g1 =
                compute_hougaard_skewness(&[theta_val], &sqrt_w, &j_mat, &cov_u, sigma, |th| Some(xs.iter().map(|&x| th[0] * th[0] * x).collect()));
            let expected = -1.5 * sigma * theta_val.signum() / (theta_val * theta_val * s2.sqrt());
            assert_eq!(g1.len(), 1);
            assert!(g1[0] * expected_sign > 0.0, "expected sign {expected_sign}, got {}", g1[0]);
            assert_approx_eq(g1[0], expected, 1e-8, 1e-8, "closed-form signed Hougaard g1");
        }

        let j_ok = DenseMatrix::from_row_major(3, 1, vec![2.0, 4.0, 6.0]).unwrap();
        let cov_ok = DenseMatrix::from_row_major(1, 1, vec![1.0 / (4.0 * s2)]).unwrap();
        let eval_ok = |th: &[f64]| Some(xs.iter().map(|&x| th[0] * th[0] * x).collect());

        for bad_sigma in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
            let out = compute_hougaard_skewness(&[1.0], &sqrt_w, &j_ok, &cov_ok, bad_sigma, eval_ok);
            assert_eq!(out.len(), 1);
            assert!(out[0].is_nan(), "bad sigma {bad_sigma} must yield NaN, got {}", out[0]);
        }

        let fail_init = compute_hougaard_skewness(&[1.0], &sqrt_w, &j_ok, &cov_ok, 1.0, |_| None);
        assert!(fail_init[0].is_nan(), "initial eval failure must yield NaN, got {}", fail_init[0]);

        let fail_perturb = compute_hougaard_skewness(&[1.0], &sqrt_w, &j_ok, &cov_ok, 1.0, |th| {
            if (th[0] - 1.0).abs() < 1e-12 { Some(xs.iter().map(|&x| th[0] * th[0] * x).collect()) } else { None }
        });
        assert!(fail_perturb[0].is_nan(), "perturbation eval failure must yield NaN, got {}", fail_perturb[0]);

        let non_finite_eval = compute_hougaard_skewness(&[1.0], &sqrt_w, &j_ok, &cov_ok, 1.0, |_| Some(vec![1.0, f64::NAN, 3.0]));
        assert!(non_finite_eval[0].is_nan(), "non-finite model eval must yield NaN, got {}", non_finite_eval[0]);

        let j_two = DenseMatrix::from_row_major(3, 2, vec![2.0, 1.0, 4.0, 1.0, 6.0, 1.0]).unwrap();
        for bad_diag in [0.0, -0.5, f64::NAN] {
            let cov_mixed = DenseMatrix::from_row_major(2, 2, vec![0.25, 0.0, 0.0, bad_diag]).unwrap();
            let out = compute_hougaard_skewness(&[1.0, 2.0], &sqrt_w, &j_two, &cov_mixed, 1.0, |th| {
                Some(xs.iter().map(|&x| th[0] * th[0] * x + th[1]).collect())
            });
            assert_eq!(out.len(), 2);
            assert!(out[0].is_finite(), "valid slot 0 should remain finite for bad slot-1 diag {bad_diag}, got {}", out[0]);
            assert!(out[1].is_nan(), "non-positive/NaN covariance diagonal {bad_diag} must yield NaN, got {}", out[1]);
        }
    }

    fn assert_all_hougaard_nan(fit: &curvebook_fit::NonlinearFitResult, ctx: &str) {
        assert!(!fit.parameters.is_empty(), "{ctx}: expected at least one free parameter");
        for p in &fit.parameters {
            assert!(p.hougaard_g1.is_nan(), "{ctx}: expected NaN hougaard_g1 for {}, got {}", p.name, p.hougaard_g1);
            assert!(p.hougaard_g1_option().is_none(), "{ctx}: expected None hougaard_g1_option for {}", p.name);
        }
    }

    fn assert_all_hougaard_empty(res: &curvebook_doc::ResultSheet, ctx: &str) {
        let ptab = find_tab(res, "nonlinear_parameters");
        assert!(!ptab.rows.is_empty(), "{ctx}: expected parameter rows");
        let hc = hougaard_col_index(ptab);
        for (lbl, row) in ptab.row_labels.iter().zip(&ptab.rows) {
            assert_eq!(row[hc], ResultCell::Empty, "{ctx}: row `{lbl}` must emit Empty Hougaard cell, got {:?}", row[hc]);
        }
    }

    #[test]
    fn r19_6_disabled_plot_only_df_zero_and_ambiguous_emit_nan_and_empty_never_zero() {
        let line = get_builtin_model("straight_line").unwrap();

        let ds_line = series_xy(0, "Line", &[0.0, 1.0, 2.0, 3.0, 4.0], &[1.1, 2.9, 5.2, 6.8, 9.1]);
        let fit_disabled = fit_with(line.as_ref(), std::slice::from_ref(&ds_line), &cfg_fast());
        assert_eq!(fit_disabled.status, FitStatus::Converged);
        assert_all_hougaard_nan(&fit_disabled, "compute_hougaard=false");

        let fit_plot_only = fit_with(
            line.as_ref(),
            std::slice::from_ref(&ds_line),
            &SolverConfig { plot_function_only: true, compute_hougaard: true, ..SolverConfig::default() },
        );
        assert_all_hougaard_nan(&fit_plot_only, "plot_function_only");
        let sheet_line = xy_sheet_single("xy-plot-only", "Plot Only", "A", &[0.0, 1.0, 2.0, 3.0, 4.0], &[1.1, 2.9, 5.2, 6.8, 9.1]);
        let mut nl_plot_only = nl_fast("straight_line");
        nl_plot_only.plot_function_only = true;
        assert_all_hougaard_empty(&exec_nl(&sheet_line, nl_plot_only), "plot_function_only");

        let ds_df0 = series_xy(0, "TwoPts", &[0.0, 1.0], &[1.0, 3.0]);
        let fit_df0 = fit_with(
            line.as_ref(),
            std::slice::from_ref(&ds_df0),
            &SolverConfig { compute_hougaard: true, compute_profile_ci: false, ..SolverConfig::default() },
        );
        assert_eq!(fit_df0.df, 0);
        assert_all_hougaard_nan(&fit_df0, "df == 0");
        let sheet_df0 = xy_sheet_single("xy-df0", "DF Zero", "A", &[0.0, 1.0], &[1.0, 3.0]);
        assert_all_hougaard_empty(&exec_nl(&sheet_df0, nl_fast("straight_line")), "df == 0");

        let xs_amb: Vec<f64> = (0..12).map(f64::from).collect();
        let noise = [0.02, -0.015, 0.01, -0.025, 0.015, -0.01, 0.02, -0.02, 0.005, -0.015, 0.025, -0.01];
        let ys_amb: Vec<f64> = xs_amb.iter().zip(noise).map(|(&x, e)| 1.5 + 2.5 * x + e).collect();
        let ds_amb = series_xy(0, "Amb", &xs_amb, &ys_amb);
        let dead_dsl = "Y = A*X + B + C*0\n# Inits: A=2.0, B=1.0, C=1.0";
        let dead_model = dsl_compile(dead_dsl);
        let fit_amb = fit_with(
            &dead_model,
            std::slice::from_ref(&ds_amb),
            &SolverConfig {
                compute_hougaard: true,
                compute_profile_ci: false,
                identifiability_mode: Some(IdentifiabilityMode::IdentifyAmbiguousFits),
                ..SolverConfig::default()
            },
        );
        assert!(fit_amb.covariance.ambiguous, "dead-parameter fit must be ambiguous");
        assert_all_hougaard_nan(&fit_amb, "cov.ambiguous");
        let sheet_amb = xy_sheet_single("xy-amb", "Ambiguous", "A", &xs_amb, &ys_amb);
        for (mode, profile_ci) in [
            (Some(IdentifiabilityMode::IdentifyAmbiguousFits), false),
            (Some(IdentifiabilityMode::IdentifyUnstableParameters), false),
            (Some(IdentifiabilityMode::IdentifyUnstableParameters), true),
        ] {
            let mut nl_amb = NonlinearFitSpec::new("dead_param");
            nl_amb.custom_dsl = Some(dead_dsl.to_owned());
            nl_amb.identifiability_mode = mode;
            nl_amb.profile_likelihood_ci = profile_ci;
            assert_all_hougaard_empty(&exec_nl(&sheet_amb, nl_amb), &format!("ambiguous ({mode:?}, profile={profile_ci})"));
        }
    }

    #[test]
    fn r19_6_global_k_gt_10_and_eval_failure_emit_nan_and_empty_never_zero() {
        let line = get_builtin_model("straight_line").unwrap();

        let xs_g = [0.0_f64, 1.0, 2.0, 3.0];
        let n_ds = 10_usize;
        let mut series_g = Vec::with_capacity(n_ds);
        let mut col_names = Vec::with_capacity(n_ds);
        let mut col_ys = Vec::with_capacity(n_ds);
        for d in 0..n_ds {
            let intercept = 1.0 + (d as f64) * 2.0;
            let ys: Vec<f64> =
                xs_g.iter().enumerate().map(|(i, &x)| intercept + 3.0 * x + if (i + d).is_multiple_of(2) { 0.15 } else { -0.15 }).collect();
            let name = format!("DS{}", d + 1);
            series_g.push(series_xy(d, &name, &xs_g, &ys));
            col_names.push(name);
            col_ys.push(ys);
        }
        let fit_global_11 = fit_with(
            line.as_ref(),
            &series_g,
            &SolverConfig {
                parameter_rules: vec![rule_shared("Slope")],
                compute_hougaard: true,
                compute_profile_ci: false,
                compute_bands: false,
                ..SolverConfig::default()
            },
        );
        assert_eq!(fit_global_11.status, FitStatus::Converged);
        assert_eq!(fit_global_11.k_free, 11);
        assert!(fit_global_11.df > 0 && !fit_global_11.covariance.ambiguous);
        assert_all_hougaard_nan(&fit_global_11, "k_free > 10 global fit");

        let cols_ref: Vec<(&str, &[f64])> = col_names.iter().zip(&col_ys).map(|(n, y)| (n.as_str(), y.as_slice())).collect();
        let sheet_g11 = xy_sheet("xy-g11", "Global 11 Params", &xs_g, &cols_ref);
        let mut nl_g11 = nl_fast("straight_line");
        nl_g11.parameter_rules = vec![rule_shared("Slope")];
        let res_g11 = exec_nl(&sheet_g11, nl_g11);
        assert_eq!(find_tab(&res_g11, "nonlinear_parameters").rows.len(), 11);
        assert_all_hougaard_empty(&res_g11, "k_free > 10 global fit");

        let ds_pf = series_xy(0, "PF", &[1.0, 2.0, 3.0, 4.0], &[5.0, 7.0, 11.0, 17.0]);
        let pf_model = PerturbFailModel::new();
        let fit_pf = fit_nonlinear(
            &pf_model,
            &[ds_pf],
            &HookContext::default(),
            &SolverConfig {
                compute_hougaard: true,
                compute_profile_ci: false,
                compute_bands: false,
                use_simplex_warm_start: false,
                ..SolverConfig::default()
            },
        )
        .unwrap();
        assert_eq!(fit_pf.status, FitStatus::Converged);
        assert!(fit_pf.df > 0 && !fit_pf.covariance.ambiguous);
        assert_all_hougaard_nan(&fit_pf, "perturbation eval failure");
    }

    #[test]
    fn r19_6_negative_signed_g1_preserved_in_summary_and_rendered_as_abs_in_table_and_checklist() {
        let xs = [1.0_f64, 2.0, 3.0, 4.0];
        let errs = [4.0_f64, -5.0, -2.0, 3.0];
        let ys: Vec<f64> = xs.iter().zip(errs).map(|(&x, e)| x + e).collect();
        let dsl_pos = "Y = Theta * Theta * X\n# Inits: Theta=1.0";
        let model_pos = dsl_compile(dsl_pos);

        let ds = series_xy(0, "QuadParam", &xs, &ys);
        let fit = fit_with(
            &model_pos,
            std::slice::from_ref(&ds),
            &SolverConfig {
                method: FitMethod::LeastSquares,
                compute_hougaard: true,
                compute_profile_ci: false,
                compute_bands: false,
                ..SolverConfig::default()
            },
        );
        assert_eq!(fit.status, FitStatus::Converged);
        assert_eq!(fit.df, 3);
        let p_theta = find_param(&fit, "Theta");
        let expected_g1 = -1.5 * (18.0_f64 / 30.0).sqrt();
        assert!(p_theta.estimate > 0.0, "expected positive Theta optimum, got {}", p_theta.estimate);
        assert!(p_theta.hougaard_g1.is_finite() && p_theta.hougaard_g1 < -1.0, "expected signed g1 < -1.0, got {}", p_theta.hougaard_g1);
        assert_approx_eq(p_theta.hougaard_g1, expected_g1, 1e-6, 1e-6, "signed g1 on FittedParameterSummary");
        assert_eq!(p_theta.hougaard_g1_option(), Some(p_theta.hougaard_g1));

        let sheet = xy_sheet_single("xy-neg-g1", "Negative g1", "QuadParam", &xs, &ys);
        for (profile_ci, expected_cols) in [(false, 5_usize), (true, 4_usize)] {
            let mut nl = NonlinearFitSpec::new("quad_param_neg_g1");
            nl.custom_dsl = Some(dsl_pos.to_owned());
            nl.profile_likelihood_ci = profile_ci;
            let res = exec_nl(&sheet, nl);
            assert_eq!(res.status, ResultStatus::UpToDate);
            let ptab = find_tab(&res, "nonlinear_parameters");
            assert_eq!(ptab.column_headers.len(), expected_cols);
            let hc = hougaard_col_index(ptab);
            let row = tab_row(&res, "nonlinear_parameters", "Theta");
            let cell_val = cell_num(&row[hc]);
            assert!(cell_val > 1.0, "table cell under `Hougaard |g1|` must be positive |g1|, got {cell_val}");
            assert_approx_eq(cell_val, p_theta.hougaard_g1.abs(), 1e-6, 1e-6, "table cell matches |g1|");

            let chk = res
                .checklist
                .iter()
                .find(|c| c.assumption == "Hougaard's skewness (nonlinearity)")
                .expect("expected Hougaard checklist warning when |g1| > 1");
            assert_eq!(chk.status, ChecklistStatus::Warn);
            assert_eq!(chk.diagnostic_metric.as_deref(), Some(format!("max |g1| = {:.3}", p_theta.hougaard_g1.abs()).as_str()));
        }

        let neg_rule = rule_local_bounds("Theta", Some(-1.0), None, Some(-0.1));
        let fit_pos_g1 = fit_with(
            &model_pos,
            std::slice::from_ref(&ds),
            &SolverConfig {
                parameter_rules: vec![neg_rule.clone()],
                compute_hougaard: true,
                compute_profile_ci: false,
                compute_bands: false,
                ..SolverConfig::default()
            },
        );
        let p_neg = find_param(&fit_pos_g1, "Theta");
        assert!(p_neg.estimate < 0.0 && p_neg.hougaard_g1 > 1.0, "expected positive g1 > 1, got {}", p_neg.hougaard_g1);
        assert_approx_eq(p_neg.hougaard_g1, -expected_g1, 1e-6, 1e-6, "positive signed g1");
        let mut nl_pos_g1 = nl_fast("quad_param_pos_g1");
        nl_pos_g1.custom_dsl = Some(dsl_pos.to_owned());
        nl_pos_g1.parameter_rules = vec![neg_rule];
        let res_pos_g1 = exec_nl(&sheet, nl_pos_g1);
        let ptab_pos = find_tab(&res_pos_g1, "nonlinear_parameters");
        let cell_pos = cell_num(&tab_row(&res_pos_g1, "nonlinear_parameters", "Theta")[hougaard_col_index(ptab_pos)]);
        assert_approx_eq(cell_pos, p_neg.hougaard_g1.abs(), 1e-6, 1e-6, "positive g1 table cell matches |g1|");

        let xs_u: Vec<f64> = (0..12).map(f64::from).collect();
        let noise_u = [0.02, -0.015, 0.01, -0.025, 0.015, -0.01, 0.02, -0.02, 0.005, -0.015, 0.025, -0.01];
        let ys_u: Vec<f64> = xs_u.iter().zip(noise_u).map(|(&x, e)| 1.5 + 2.5 * x + e).collect();
        let dsl_u = "Y = A*X + B + C*(X + 1 + 0.00001*X*X)\n# Inits: A=2.0, B=1.0, C=0.0";
        let fit_u = fit_with(
            &dsl_compile(dsl_u),
            &[series_xy(0, "Col", &xs_u, &ys_u)],
            &SolverConfig {
                identifiability_mode: Some(IdentifiabilityMode::IdentifyUnstableParameters),
                compute_hougaard: true,
                compute_profile_ci: false,
                ..SolverConfig::default()
            },
        );
        let p_c = find_param(&fit_u, "C");
        assert!(p_c.notes.contains(&ParameterNote::Unstable));
        assert!(p_c.hougaard_g1.is_nan());
        assert!(p_c.hougaard_g1_option().is_none());
    }
}
