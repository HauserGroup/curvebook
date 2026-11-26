//! D1 R 4.5.2 `stats::nls` / `stats::lm` verification oracles for all 34 R18.3
//! built-in equation library additions.
//!
//! D1 Provenance:
//! - Independent reference parameter estimates, asymptotic standard errors, and
//!   residual sums of squares computed by `R 4.5.2 (2025-10-31)` `stats::nls` /
//!   `stats::lm` via `crates/fit/tests/reference/r18_3_extended_equations.R`
//!   (2026-11-23).
//! - Literature sources: Hill (1910), Gaddum (1937), Hodgkin & Huxley (1952),
//!   Arunlakshana & Schild (1959), Chadwick & Leenhouts (1973), Black & Leff (1983),
//!   Ehlert (1988), Swillens (1995), Bloomfield (2000), Limpert et al. (2001),
//!   Christopoulos & Kenakin (2002), Bevington & Robinson (2003), Motulsky &
//!   Christopoulos (2004), Gottschalk & Dunn (2005), Colquhoun (2007), Brenner (2008),
//!   Liao & Liu (2009), Sebaugh (2011).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::excessive_precision, clippy::too_many_arguments)]

use super::common;

use curvebook_doc::{CiBound, ParameterRule, ParameterScope};
use curvebook_fit::library::pharma::kb_ci_from_pkb_ci;
use curvebook_fit::{DatasetSeries, FitStatus, NonlinearFitResult, ParametricModel, SolverConfig, get_builtin_model};
use curvebook_testkit::assert_approx_eq;

use common::{
    assert_converged, cfg_fast, cfg_fast_rules, dataset_param_val, find_derived, find_param, fit_with, linspace, param_ds_val, rule_const,
    rule_local_init, rule_per_ds, rule_shared_init, series_eval, series_xy,
};

type ParamOracle<'a> = (&'a str, Option<usize>, f64, f64, f64, f64);
type LqCase<'a> = (&'a str, &'a str, Vec<f64>, f64, &'a [ParamOracle<'a>]);

fn logspace(exp_start: f64, exp_end: f64, n: usize) -> Vec<f64> {
    linspace(exp_start, exp_end, n).into_iter().map(|e| 10.0_f64.powf(e)).collect()
}

fn eps_vec(n: usize, amp: f64) -> Vec<f64> {
    (1..=n).map(|i| amp * (2.3 * i as f64 + 0.7).sin()).collect()
}

fn local_inits(pairs: &[(&str, f64)]) -> Vec<ParameterRule> {
    pairs.iter().map(|&(n, v)| rule_local_init(n, v)).collect()
}

fn shared_inits(pairs: &[(&str, f64)]) -> Vec<ParameterRule> {
    pairs.iter().map(|&(n, v)| rule_shared_init(n, v)).collect()
}

fn verify_fit(
    model_id: &str,
    series: &[DatasetSeries],
    rules: Vec<ParameterRule>,
    profile_ci: bool,
    rss: (f64, f64),
    params: &[ParamOracle<'_>],
) -> NonlinearFitResult {
    let model = get_builtin_model(model_id).unwrap_or_else(|| panic!("missing built-in model `{model_id}`"));
    let cfg = SolverConfig {
        parameter_rules: rules,
        max_iters: 4_000,
        compute_profile_ci: profile_ci,
        compute_prediction_bands: false,
        compute_lack_of_fit: false,
        compute_normality: false,
        interpolate_unknowns: false,
        use_simplex_warm_start: true,
        ..cfg_fast()
    };
    let fit = fit_with(model.as_ref(), series, &cfg);
    assert!(matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous), "`{model_id}`: {:?}, rss={:.6e}", fit.status, fit.rss);
    for &(name, ds, exp_est, exp_se, rel_est, rel_se) in params {
        let p = fit
            .parameters
            .iter()
            .find(|p| p.name == name && (ds.is_none() || p.dataset_index == ds))
            .unwrap_or_else(|| panic!("{model_id}: missing param `{name}` (ds={ds:?})"));
        assert_approx_eq(p.estimate, exp_est, 1e-12, rel_est, &format!("{model_id} {name} estimate"));
        assert_approx_eq(p.se, exp_se, 1e-12, rel_se, &format!("{model_id} {name} SE"));
    }
    assert_approx_eq(fit.rss, rss.0, 1e-12, rss.1, &format!("{model_id} RSS"));
    fit
}

fn check(id: &str, xs: &[f64], ys: &[f64], rules: Vec<ParameterRule>, rss: (f64, f64), params: &[ParamOracle<'_>]) -> NonlinearFitResult {
    verify_fit(id, &[series_xy(0, "A", xs, ys)], rules, false, rss, params)
}

#[test]
fn d1_r18_3_group1_linear_normalized_dose_response() {
    let x_c = logspace(-3.0, 3.0, 15);
    let eps = eps_vec(x_c.len(), 0.6);

    let y1: Vec<f64> = x_c.iter().zip(&eps).map(|(&x, &e)| 100.0 * x / (2.5 + x) + e).collect();
    let f1 = check(
        "agonist_vs_response_normalized",
        &x_c,
        &y1,
        vec![],
        (2.687_733_043_062_753, 1e-8),
        &[("EC50", None, 2.499_406_558_767_244, 2.664_777_923_259_584e-2, 1e-7, 1e-6)],
    );
    assert_approx_eq(find_derived(&f1, 0, "LogEC50"), 2.499_406_558_767_244_f64.log10(), 1e-12, 1e-7, "ag_norm LogEC50");

    let y2: Vec<f64> = x_c.iter().zip(&eps).map(|(&x, &e)| 100.0 / (1.0 + (2.5 / x).powf(1.6)) + e).collect();
    check(
        "agonist_vs_response_normalized_var",
        &x_c,
        &y2,
        vec![],
        (2.656_913_224_363_825, 1e-8),
        &[
            ("EC50", None, 2.496_299_008_767_880, 2.172_672_094_891_990e-2, 1e-7, 1e-6),
            ("HillSlope", None, 1.593_175_580_361_390, 1.959_452_357_085_131e-2, 1e-7, 1e-6),
        ],
    );

    let y3: Vec<f64> = x_c.iter().zip(&eps).map(|(&x, &e)| 100.0 / (1.0 + x / 3.2) + e).collect();
    let f3 = check(
        "inhibitor_vs_response_normalized",
        &x_c,
        &y3,
        vec![],
        (2.687_821_480_217_989, 1e-8),
        &[("IC50", None, 3.200_201_430_578_598, 3.412_008_372_703_434e-2, 1e-7, 1e-6)],
    );
    assert_approx_eq(find_derived(&f3, 0, "LogIC50"), 3.200_201_430_578_598_f64.log10(), 1e-12, 1e-7, "inh_norm LogIC50");

    let y4: Vec<f64> = x_c.iter().zip(&eps).map(|(&x, &e)| 100.0 / (1.0 + (3.2 / x).powf(-1.4)) + e).collect();
    check(
        "inhibitor_vs_response_normalized_var",
        &x_c,
        &y4,
        vec![],
        (2.664_034_882_214_670, 1e-8),
        &[
            ("IC50", None, 3.200_990_891_751_175, 2.973_625_823_296_110e-2, 1e-7, 1e-6),
            ("HillSlope", None, -1.405_506_013_960_904, 1.617_448_382_318_552e-2, 1e-7, 1e-6),
        ],
    );
}

#[test]
fn d1_r18_3_group2_linear_special_dose_response() {
    // 5. agonist_vs_response_5pl
    let (x5, e5, f5) = (logspace(-3.0, 3.0, 19), eps_vec(19, 0.25), 2.0_f64.powf(1.0 / 1.8) - 1.0);
    let y5: Vec<f64> = x5.iter().zip(&e5).map(|(&x, &e)| 5.0 + 90.0 / (1.0 + f5 * (1.5 / x).powf(1.3)).powf(1.8) + e).collect();
    check(
        "agonist_vs_response_5pl",
        &x5,
        &y5,
        local_inits(&[("Bottom", 4.0), ("Top", 96.0), ("EC50", 1.2), ("HillSlope", 1.1), ("S", 1.5)]),
        (5.760_455_706_317_876e-1, 1e-7),
        &[
            ("Bottom", None, 4.988_530_687_510_472, 8.050_035_619_633_227e-2, 1e-6, 1e-4),
            ("Top", None, 94.984_406_788_219_18, 9.408_706_958_077_390e-2, 1e-6, 1e-4),
            ("EC50", None, 1.501_069_798_007_869, 7.344_100_651_080_574e-3, 1e-6, 1e-4),
            ("HillSlope", None, 1.307_030_982_965_748, 1.841_954_018_801_089e-2, 1e-6, 1e-4),
            ("S", None, 1.760_033_200_788_555, 1.057_744_945_156_111e-1, 1e-5, 1e-4),
        ],
    );

    // 6. biphasic_dose_response_conc & 7. bell_shaped_dose_response_conc
    let (xb, eb) = (logspace(-4.0, 4.0, 25), eps_vec(25, 0.3));
    let yb: Vec<f64> =
        xb.iter().zip(&eb).map(|(&x, &e)| 10.0 + 90.0 * (0.4 / (1.0 + (0.05 / x).powf(1.2)) + 0.6 / (1.0 + (50.0 / x).powf(1.1))) + e).collect();
    check(
        "biphasic_dose_response_conc",
        &xb,
        &yb,
        local_inits(&[("Bottom", 8.0), ("Top", 102.0), ("Frac", 0.45), ("EC50_1", 0.04), ("EC50_2", 40.0), ("nH1", 1.0), ("nH2", 1.0)]),
        (1.119_341_645_690_582, 1e-7),
        &[
            ("Bottom", None, 9.987_709_379_427_113, 1.191_275_733_717_345e-1, 1e-6, 1e-5),
            ("Top", None, 100.051_206_011_185_3, 1.471_734_501_536_028e-1, 1e-6, 1e-5),
            ("Frac", None, 3.997_641_283_003_502e-1, 3.086_644_019_243_629e-3, 1e-6, 1e-5),
            ("EC50_1", None, 4.993_229_168_927_931e-2, 1.025_315_155_300_658e-3, 1e-6, 1e-5),
            ("EC50_2", None, 50.111_259_249_909_64, 7.418_082_744_639_449e-1, 1e-6, 1e-5),
        ],
    );

    let ybell: Vec<f64> = xb
        .iter()
        .zip(&eb)
        .map(|(&x, &e)| 90.0 + (10.0 - 90.0) / (1.0 + (x / 0.1).powf(1.3)) + (15.0 - 90.0) * (1.0 - 1.0 / (1.0 + (x / 20.0).powf(1.1))) + e)
        .collect();
    check(
        "bell_shaped_dose_response_conc",
        &xb,
        &ybell,
        local_inits(&[("Plateau1", 12.0), ("Plateau2", 18.0), ("Dip", 88.0), ("EC50_1", 0.08), ("EC50_2", 18.0), ("nH1", 1.1), ("nH2", 1.0)]),
        (1.121_930_505_265_480, 1e-7),
        &[
            ("Plateau1", None, 9.994_866_331_562_198, 1.066_649_391_729_973e-1, 1e-6, 1e-5),
            ("Plateau2", None, 15.038_656_829_999_82, 1.274_194_508_365_115e-1, 1e-6, 1e-5),
            ("Dip", None, 89.973_740_709_371_68, 4.066_229_552_803_637e-1, 1e-6, 1e-5),
            ("EC50_1", None, 9.995_311_337_529_553e-2, 1.124_332_210_329_994e-3, 1e-6, 1e-5),
            ("EC50_2", None, 19.991_286_491_009_51, 2.822_559_739_650_533e-1, 1e-6, 1e-5),
        ],
    );

    // 8. operational_model_agonist_conc
    let (xo, eo) = (logspace(-3.0, 2.0, 16), eps_vec(16, 0.25));
    let yo: Vec<f64> = xo.iter().zip(&eo).map(|(&x, &e)| 5.0 + 95.0 / (1.0 + ((2.0 + x) / (1.5 * x)).powf(1.2)) + e).collect();
    let mut r8 = local_inits(&[("Basal", 4.0), ("KA", 1.8), ("Tau", 1.4), ("n", 1.1)]);
    r8.push(rule_const("Emax", 100.0));
    check(
        "operational_model_agonist_conc",
        &xo,
        &yo,
        r8,
        (4.671_387_856_843_455e-1, 1e-7),
        &[
            ("Basal", None, 4.993_492_002_449_718, 9.233_084_449_236_710e-2, 1e-6, 1e-5),
            ("KA", None, 1.995_570_691_391_321, 7.283_045_824_374_790e-2, 1e-6, 1e-5),
            ("Tau", None, 1.498_694_304_294_045, 1.653_193_771_435_948e-2, 1e-6, 1e-5),
            ("n", None, 1.200_297_985_438_651, 2.271_855_521_335_176e-2, 1e-6, 1e-5),
        ],
    );

    // 9. ec50_shift_gaddum_schild_conc & 10. allosteric_ec50_shift_conc (3 datasets each)
    let (x_g, eg) = (logspace(-9.0, -4.0, 12), eps_vec(12, 0.35));
    let ds_g: Vec<DatasetSeries> = [0.0, 1e-7, 1e-6]
        .iter()
        .enumerate()
        .map(|(k, &b_conc)| {
            let ec_eff = 1e-7 * if b_conc > 0.0 { 1.0 + (b_conc * 10.0_f64.powf(7.5)).powf(1.15) } else { 1.0 };
            let sign = if (k + 1) % 2 == 0 { -1.0 } else { 1.0 };
            let y: Vec<f64> = x_g.iter().zip(&eg).map(|(&x, &e)| 5.0 + 95.0 / (1.0 + (ec_eff / x).powf(1.2)) + sign * e).collect();
            series_xy(k, &format!("B={b_conc}"), &x_g, &y)
        })
        .collect();
    let mut r9 = shared_inits(&[("Bottom", 4.0), ("Top", 98.0), ("EC50", 1.2e-7), ("HillSlope", 1.1), ("pA2", 7.3), ("SchildSlope", 1.0)]);
    r9.push(rule_per_ds("AntagConc", &[(0, 0.0), (1, 1e-7), (2, 1e-6)]));
    verify_fit(
        "ec50_shift_gaddum_schild_conc",
        &ds_g,
        r9,
        false,
        (2.050_378_685_049_887, 1e-7),
        &[
            ("EC50", None, 9.995_193_242_356_752e-8, 6.968_528_657_848_965e-10, 1e-5, 1e-4),
            ("pA2", None, 7.499_837_941_579_449, 5.721_919_206_944_623e-3, 1e-6, 1e-4),
            ("SchildSlope", None, 1.150_167_152_156_678, 4.493_679_352_400_328e-3, 1e-6, 1e-4),
        ],
    );

    let ds_a: Vec<DatasetSeries> = [0.0, 1e-6, 1e-5]
        .iter()
        .enumerate()
        .map(|(k, &m_conc)| {
            let ec_eff = 1e-7 * (1.0 + m_conc / 2e-6) / (1.0 + 0.1 * m_conc / 2e-6);
            let sign = if (k + 1) % 2 == 0 { -1.0 } else { 1.0 };
            let y: Vec<f64> = x_g.iter().zip(&eg).map(|(&x, &e)| 4.0 + 92.0 / (1.0 + (ec_eff / x).powf(1.1)) + sign * e).collect();
            series_xy(k, &format!("M={m_conc}"), &x_g, &y)
        })
        .collect();
    let mut r10 = shared_inits(&[("Bottom", 3.0), ("Top", 95.0), ("EC50", 1.1e-7), ("HillSlope", 1.0), ("Kb", 1.8e-6), ("alpha", 0.12)]);
    r10.push(rule_per_ds("ModConc", &[(0, 0.0), (1, 1e-6), (2, 1e-5)]));
    verify_fit(
        "allosteric_ec50_shift_conc",
        &ds_a,
        r10,
        false,
        (2.050_173_261_212_730, 1e-7),
        &[
            ("EC50", None, 9.998_569_874_422_053e-8, 7.626_438_361_113_712e-10, 1e-5, 1e-4),
            ("Kb", None, 1.994_097_903_837_269e-6, 8.354_668_897_651_919e-8, 1e-5, 1e-4),
            ("alpha", None, 1.003_079_874_636_961e-1, 5.814_833_128_540_418e-3, 1e-5, 1e-4),
        ],
    );

    // 11. agonist_vs_response_ecf & 12. absolute_ic50_4pl_conc
    let (x_c, ec) = (logspace(-3.0, 3.0, 15), eps_vec(15, 0.35));
    let ec50_80 = 4.0 * (20.0_f64 / 80.0).powf(1.0 / 1.4);
    let y_ecf: Vec<f64> = x_c.iter().zip(&ec).map(|(&x, &e)| 5.0 + 95.0 / (1.0 + (ec50_80 / x).powf(1.4)) + e).collect();
    let mut r11 = local_inits(&[("Bottom", 4.0), ("Top", 98.0), ("ECF", 3.5), ("HillSlope", 1.2)]);
    r11.push(rule_const("F", 80.0));
    check(
        "agonist_vs_response_ecf",
        &x_c,
        &y_ecf,
        r11,
        (9.089_563_850_842_785e-1, 1e-7),
        &[
            ("ECF", None, 3.988_876_092_453_611, 4.676_302_238_424_108e-2, 1e-6, 1e-5),
            ("HillSlope", None, 1.401_773_407_489_595, 1.264_599_667_081_122e-2, 1e-6, 1e-5),
        ],
    );

    let ic_rel = 5.0 * ((50.0_f64 - 15.0) / (95.0 - 50.0)).powf(1.0 / 1.3);
    let y_abs: Vec<f64> = x_c.iter().zip(&ec).map(|(&x, &e)| 15.0 + 80.0 / (1.0 + (x / ic_rel).powf(1.3)) + e).collect();
    let mut r12 = local_inits(&[("Bottom", 14.0), ("Top", 96.0), ("IC50", 4.5), ("HillSlope", 1.2)]);
    r12.extend([rule_const("Control0", 0.0), rule_const("Control100", 100.0)]);
    let f12 = check(
        "absolute_ic50_4pl_conc",
        &x_c,
        &y_abs,
        r12,
        (9.082_744_230_454_814e-1, 1e-7),
        &[
            ("IC50", None, 4.996_341_798_701_130, 4.049_792_661_980_433e-2, 1e-6, 1e-5),
            ("HillSlope", None, 1.303_665_520_752_858, 1.391_285_778_600_612e-2, 1e-6, 1e-5),
        ],
    );
    assert!(find_derived(&f12, 0, "RelativeIC50") > 0.0);
}

#[test]
fn d1_r18_3_group3_mechanistic_pharmacology_and_reversed_kb_ci() {
    // 13 & 14. operational_model_depletion_log & _conc (default_parameter_rules auto-shares!)
    let log_x = linspace(-8.0, -3.0, 14);
    let conc_x: Vec<f64> = log_x.iter().map(|&lx| 10.0_f64.powf(lx)).collect();
    let ed = eps_vec(log_x.len(), 0.3);
    let dep_ys: Vec<Vec<f64>> = [5.0, 0.8]
        .iter()
        .enumerate()
        .map(|(k, &tau)| {
            let sign = if k == 1 { -1.0 } else { 1.0 };
            conc_x.iter().zip(&ed).map(|(&a, &e)| 3.0 + 102.0 / (1.0 + ((1e-5 + a) / (tau * a)).powf(1.15)) + sign * e).collect()
        })
        .collect();

    verify_fit(
        "operational_model_depletion_log",
        &[series_xy(0, "tau=5", &log_x, &dep_ys[0]), series_xy(1, "tau=0.8", &log_x, &dep_ys[1])],
        vec![],
        false,
        (1.267_339_719_477_657, 1e-7),
        &[
            ("Basal", None, 2.992_150_091_571_427, 9.326_783_428_658_374e-2, 1e-5, 1e-4),
            ("EffectMax", None, 105.162_672_303_416_7, 4.694_773_917_306_487e-1, 1e-5, 1e-4),
            ("LogKA", None, -5.000_635_301_018_728, 1.055_785_069_101_162e-2, 1e-5, 1e-4),
            ("LogTau", Some(0), 6.970_397_349_817_230e-1, 1.025_101_645_174_665e-2, 1e-5, 1e-4),
            ("LogTau", Some(1), -9.865_462_419_477_283e-2, 4.436_835_396_729_064e-3, 1e-5, 1e-4),
            ("n", None, 1.147_947_381_588_692, 1.213_801_703_548_078e-2, 1e-5, 1e-4),
        ],
    );
    verify_fit(
        "operational_model_depletion_conc",
        &[series_xy(0, "tau=5", &conc_x, &dep_ys[0]), series_xy(1, "tau=0.8", &conc_x, &dep_ys[1])],
        vec![rule_shared_init("KA", 9e-6), rule_local_init("Tau", 2.0)],
        false,
        (1.267_339_719_477_645, 1e-7),
        &[
            ("KA", None, 9.985_382_380_859_280e-6, 2.427_481_574_406_685e-7, 1e-5, 1e-4),
            ("Tau", Some(0), 4.977_826_294_098_003, 1.174_958_051_352_043e-1, 1e-5, 1e-4),
            ("Tau", Some(1), 7.967_927_562_386_462e-1, 8.140_187_085_372_374e-3, 1e-5, 1e-4),
        ],
    );

    // 15 & 16. ec50_shift_gaddum_schild_unity_log & _conc (with profile CI + reversed KB CI check)
    let (x_g, eg) = (logspace(-9.0, -4.0, 12), eps_vec(12, 0.35));
    let lx_g: Vec<f64> = x_g.iter().map(|&x| x.log10()).collect();
    let antags = [0.0, 1e-7, 1e-6];
    let su_ys: Vec<Vec<f64>> = antags
        .iter()
        .enumerate()
        .map(|(k, &b_conc)| {
            let ec_eff = 1e-7 * (1.0 + b_conc * 10.0_f64.powf(7.8));
            let sign = if (k + 1) % 2 == 0 { -1.0 } else { 1.0 };
            x_g.iter().zip(&eg).map(|(&x, &e)| 5.0 + 95.0 / (1.0 + (ec_eff / x).powf(1.2)) + sign * e).collect()
        })
        .collect();
    let ds_su_log: Vec<DatasetSeries> = (0..3).map(|k| series_xy(k, &format!("B={}", antags[k]), &lx_g, &su_ys[k])).collect();
    let ds_su_conc: Vec<DatasetSeries> = (0..3).map(|k| series_xy(k, &format!("B={}", antags[k]), &x_g, &su_ys[k])).collect();

    let fit_su_log = verify_fit(
        "ec50_shift_gaddum_schild_unity_log",
        &ds_su_log,
        vec![rule_per_ds("AntagConc", &[(0, 0.0), (1, 1e-7), (2, 1e-6)])],
        true,
        (2.050_616_178_006_570, 1e-7),
        &[
            ("LogEC50", None, -7.000_188_832_014_302, 2.977_259_252_250_148e-3, 1e-6, 1e-4),
            ("pKB", None, 7.800_042_360_182_306, 3.544_037_920_283_844e-3, 1e-6, 1e-4),
        ],
    );
    let kb_est = find_derived(&fit_su_log, 0, "KB");
    assert_approx_eq(kb_est, 10.0_f64.powf(-7.800_042_360_182_306), 1e-15, 1e-6, "gs_unity_log derived KB");

    let pkb_param = find_param(&fit_su_log, "pKB");
    let (kb_lo, kb_hi) = kb_ci_from_pkb_ci(pkb_param.ci_lower, pkb_param.ci_upper);
    match (pkb_param.ci_lower, pkb_param.ci_upper, kb_lo, kb_hi) {
        (CiBound::Finite(pl), CiBound::Finite(pu), CiBound::Finite(kl), CiBound::Finite(kh)) => {
            assert!(pl < pkb_param.estimate && pkb_param.estimate < pu && kl < kb_est && kb_est < kh);
            assert_approx_eq(kl, 10.0_f64.powf(-pu), 1e-20, 1e-12, "KB lower from -pKB upper");
            assert_approx_eq(kh, 10.0_f64.powf(-pl), 1e-20, 1e-12, "KB upper from -pKB lower");
        }
        other => panic!("expected finite profile CIs for pKB and KB, got {other:?}"),
    }

    verify_fit(
        "ec50_shift_gaddum_schild_unity_conc",
        &ds_su_conc,
        vec![rule_shared_init("EC50", 1.1e-7), rule_per_ds("AntagConc", &[(0, 0.0), (1, 1e-7), (2, 1e-6)])],
        false,
        (2.050_616_178_218_110, 1e-7),
        &[
            ("EC50", None, 9.995_652_922_435_616e-8, 6.852_412_641_606_030e-10, 1e-5, 1e-4),
            ("pKB", None, 7.800_042_476_587_072, 3.544_037_846_160_965e-3, 1e-6, 1e-4),
        ],
    );

    // 17 & 18. ec50_shift_ratio_log & _conc (default_parameter_rules auto-shares!)
    let sr_ys: Vec<Vec<f64>> = [1e-7, 6.5e-7]
        .iter()
        .enumerate()
        .map(|(k, &ec_eff)| {
            let sign = if k == 1 { -1.0 } else { 1.0 };
            x_g.iter().zip(&eg).map(|(&x, &e)| 4.0 + 94.0 / (1.0 + (ec_eff / x).powf(1.25)) + sign * e).collect()
        })
        .collect();
    verify_fit(
        "ec50_shift_ratio_log",
        &[series_xy(0, "Control", &lx_g, &sr_ys[0]), series_xy(1, "Shifted", &lx_g, &sr_ys[1])],
        vec![],
        false,
        (1.362_751_006_946_411, 1e-7),
        &[
            ("LogEC50Control", None, -7.000_314_708_472_887, 3.171_331_364_884_057e-3, 1e-6, 1e-4),
            ("EC50Ratio", None, 6.497_343_711_255_527, 5.865_264_676_029_960e-2, 1e-6, 1e-4),
        ],
    );
    verify_fit(
        "ec50_shift_ratio_conc",
        &[series_xy(0, "Control", &x_g, &sr_ys[0]), series_xy(1, "Shifted", &x_g, &sr_ys[1])],
        vec![],
        false,
        (1.362_751_006_946_840, 1e-7),
        &[
            ("EC50Control", None, 9.992_756_200_052_560e-8, 7.296_970_732_635_028e-10, 1e-5, 1e-4),
            ("EC50Ratio", None, 6.497_343_810_119_635, 5.865_265_318_704_808e-2, 1e-5, 1e-4),
        ],
    );
}

#[test]
fn d1_r18_3_group4_receptor_binding() {
    // 19. one_site_total_ligand_depletion (Swillens 1995)
    let (x_cpm, e_cpm, kd_cpm) = (linspace(500.0, 15000.0, 16), eps_vec(16, 8.0), 0.0025 * 0.5 * 1000.0 * 2000.0);
    let y_cpm: Vec<f64> = x_cpm
        .iter()
        .zip(&e_cpm)
        .map(|(&x, &e)| {
            let (a, b, c) = (-1.04, kd_cpm * 1.04 + x * 1.08 + 3200.0, -x * (0.04 * kd_cpm + 0.04 * x + 3200.0));
            (-b + (b * b - 4.0 * a * c).sqrt()) / (2.0 * a) + e
        })
        .collect();
    let f19 = check(
        "one_site_total_ligand_depletion",
        &x_cpm,
        &y_cpm,
        vec![rule_const("SpecAct", 2000.0), rule_const("Vol", 0.5)],
        (4.763_056_576_451_885e2, 1e-7),
        &[
            ("Bmax", None, 3.211_291_175_153_305e3, 4.493_607_500_533_236e1, 1e-5, 1e-4),
            ("Kd", None, 2.512_683_670_459_944e-3, 5.657_682_415_236_535e-5, 1e-5, 1e-4),
            ("NS", None, 3.929_236_029_160_844e-2, 2.528_227_310_564_613e-3, 1e-5, 1e-4),
        ],
    );
    assert_approx_eq(find_derived(&f19, 0, "Kd_cpm"), 2.512_683_670_459_944e-3 * 1e6, 1e-6, 1e-5, "swillens_total Kd_cpm");

    // 20. two_sites_fit_logic50
    let (x_comp, e_comp) = (linspace(-11.0, -3.0, 21), eps_vec(21, 0.3));
    let y_comp: Vec<f64> = x_comp
        .iter()
        .zip(&e_comp)
        .map(|(&x, &e)| 5.0 + 95.0 * (0.35 / (1.0 + 10.0_f64.powf(x + 8.5)) + 0.65 / (1.0 + 10.0_f64.powf(x + 5.5))) + e)
        .collect();
    check(
        "two_sites_fit_logic50",
        &x_comp,
        &y_comp,
        local_inits(&[("Bottom", 4.0), ("Top", 98.0), ("FractionHi", 0.4), ("LogIC50_Hi", -8.3), ("LogIC50_Lo", -5.7)]),
        (9.288_730_790_959_201e-1, 1e-7),
        &[
            ("FractionHi", None, 3.498_610_233_057_352e-1, 1.783_599_518_584_730e-3, 1e-6, 1e-5),
            ("LogIC50_Hi", None, -8.499_476_029_523_899, 1.119_829_399_682_676e-2, 1e-6, 1e-5),
            ("LogIC50_Lo", None, -5.498_993_823_825_103, 6.026_882_910_725_775e-3, 1e-6, 1e-5),
        ],
    );

    // 21. allosteric_modulator_competition
    let (x_mod, e_mod) = (linspace(-9.0, -3.0, 16), eps_vec(16, 0.35));
    let y_mod: Vec<f64> = x_mod
        .iter()
        .zip(&e_mod)
        .map(|(&x, &e)| {
            let u = 10.0_f64.powf(x + 6.2);
            100.0 * 3.0 / (1.0 + 2.0 * (1.0 + u) / (1.0 + 10.0_f64.powf(-0.8) * u)) + e
        })
        .collect();
    let f21 = check(
        "allosteric_modulator_competition",
        &x_mod,
        &y_mod,
        vec![rule_const("RadioligandNM", 1.0), rule_const("HotKdNM", 2.0)],
        (9.158_952_160_303_985e-1, 1e-7),
        &[
            ("Y0", None, 99.986_135_727_199_50, 1.175_687_451_441_320e-1, 1e-6, 1e-5),
            ("LogKB", None, -6.199_358_924_456_318, 4.448_824_299_209_523e-3, 1e-6, 1e-5),
            ("LogAlpha", None, -8.004_216_653_193_957e-1, 2.444_538_825_229_352e-3, 1e-6, 1e-5),
        ],
    );
    assert_approx_eq(find_derived(&f21, 0, "Alpha"), 10.0_f64.powf(-8.004_216_653_193_957e-1), 1e-12, 1e-6, "allo_mod_comp Alpha");
}

#[test]
fn d1_r18_3_group5_curves_and_radiobiology() {
    // 22. two_phase_association
    let (t_a, e_a) = (linspace(0.0, 12.0, 25), eps_vec(25, 0.25));
    let y_a: Vec<f64> =
        t_a.iter().zip(&e_a).map(|(&t, &e)| 10.0 + 90.0 * (0.4 * (1.0 - (-2.5 * t).exp()) + 0.6 * (1.0 - (-0.35 * t).exp())) + e).collect();
    let f22 = check(
        "two_phase_association",
        &t_a,
        &y_a,
        local_inits(&[("Y0", 9.0), ("Plateau", 98.0), ("PercentFast", 45.0), ("KFast", 2.2), ("KSlow", 0.3)]),
        (7.614_920_004_915_199e-1, 1e-7),
        &[
            ("Y0", None, 10.022_878_777_846_21, 1.948_263_571_215_341e-1, 1e-6, 1e-4),
            ("Plateau", None, 100.057_245_671_083_9, 1.174_163_742_216_940e-1, 1e-6, 1e-4),
            ("PercentFast", None, 40.373_309_194_771_47, 6.932_055_804_437_267e-1, 1e-5, 1e-4),
            ("KFast", None, 2.449_437_155_942_720, 7.057_712_934_262_081e-2, 1e-5, 1e-4),
            ("KSlow", None, 3.472_303_854_807_484e-1, 5.131_917_619_622_419e-3, 1e-5, 1e-4),
        ],
    );
    assert_approx_eq(find_derived(&f22, 0, "TauFast"), 1.0 / 2.449_437_155_942_720, 1e-12, 1e-5, "two_phase_assoc TauFast");
    assert_approx_eq(
        find_derived(&f22, 0, "HalfTimeSlow"),
        std::f64::consts::LN_2 / 3.472_303_854_807_484e-1,
        1e-12,
        1e-5,
        "two_phase_assoc HalfTimeSlow",
    );

    // 23. two_lines_intersection
    let (x_l, e_l) = (linspace(0.0, 10.0, 11), eps_vec(11, 0.2));
    let y_l1: Vec<f64> = x_l.iter().zip(&e_l).map(|(&x, &e)| 11.5 + 1.8 * (x - 4.2) + e).collect();
    let y_l2: Vec<f64> = x_l.iter().zip(&e_l).map(|(&x, &e)| 11.5 - 0.9 * (x - 4.2) - e).collect();
    verify_fit(
        "two_lines_intersection",
        &[series_xy(0, "L1", &x_l, &y_l1), series_xy(1, "L2", &x_l, &y_l2)],
        vec![],
        false,
        (4.433_462_517_654_105e-1, 1e-8),
        &[
            ("XCross", None, 4.201_132_859_030_777, 2.549_080_138_026_538e-2, 1e-7, 1e-6),
            ("YCross", None, 11.500_509_786_528_40, 3.636_744_459_505_849e-2, 1e-7, 1e-6),
            ("Slope", Some(0), 1.803_861_651_773_806, 1.496_369_785_440_708e-2, 1e-7, 1e-6),
            ("Slope", Some(1), -9.038_616_520_078_280e-1, 1.496_369_767_179_871e-2, 1e-7, 1e-6),
        ],
    );

    // 24. gaussian_area & 25. lognormal_area
    let sqrt_2pi = (2.0 * std::f64::consts::PI).sqrt();
    let (x_g2, e_g2) = (linspace(-4.0, 10.0, 29), eps_vec(29, 0.08));
    let y_g2: Vec<f64> = x_g2.iter().zip(&e_g2).map(|(&x, &e)| (25.0 / (1.8 * sqrt_2pi)) * (-0.5 * ((x - 3.2) / 1.8).powi(2)).exp() + e).collect();
    check(
        "gaussian_area",
        &x_g2,
        &y_g2,
        vec![],
        (9.360_374_786_982_736e-2, 1e-8),
        &[
            ("Area", None, 24.999_811_128_812_58, 1.312_583_196_158_744e-1, 1e-7, 1e-6),
            ("Mean", None, 3.199_996_408_056_260, 1.091_252_576_248_866e-2, 1e-7, 1e-6),
            ("SD", None, 1.799_974_400_344_441, 1.091_256_588_710_575e-2, 1e-7, 1e-6),
        ],
    );

    let (x_ln, e_ln) = (linspace(0.5, 16.0, 24), eps_vec(24, 0.1));
    let y_ln: Vec<f64> = x_ln.iter().zip(&e_ln).map(|(&x, &e)| (40.0 / x) * (-0.5 * ((x / 4.5).ln() / 1.85_f64.ln()).powi(2)).exp() + e).collect();
    let f25 = check(
        "lognormal_area",
        &x_ln,
        &y_ln,
        vec![],
        (1.151_714_046_806_732e-1, 1e-7),
        &[
            ("A", None, 40.013_379_257_991_65, 1.451_978_561_659_727e-1, 1e-6, 1e-5),
            ("GeoMean", None, 4.498_089_687_421_308, 1.365_921_724_955_707e-2, 1e-6, 1e-5),
            ("GeoSD", None, 1.849_050_901_275_846, 4.788_872_904_460_787e-3, 1e-6, 1e-5),
        ],
    );
    assert_approx_eq(
        find_derived(&f25, 0, "Area"),
        40.013_379_257_991_65 * sqrt_2pi * 1.849_050_901_275_846_f64.ln(),
        1e-12,
        1e-6,
        "lognormal_area derived Area",
    );

    // 26. standard_sine_wave & 27. log_exponential_growth
    let (x_sw, e_sw) = (linspace(0.0, 12.0, 25), eps_vec(25, 0.12));
    let y_sw: Vec<f64> = x_sw.iter().zip(&e_sw).map(|(&x, &e)| 4.5 * (2.0 * std::f64::consts::PI * x / 5.2 + 0.6).sin() + e).collect();
    let f26 = check(
        "standard_sine_wave",
        &x_sw,
        &y_sw,
        local_inits(&[("Amplitude", 4.2), ("Wavelength", 5.0), ("PhaseShift", 0.5)]),
        (1.792_758_602_761_775e-1, 1e-8),
        &[
            ("Amplitude", None, 4.499_399_406_868_956, 2.517_865_157_990_953e-2, 1e-7, 1e-6),
            ("Wavelength", None, 5.202_063_112_354_752, 7.187_584_208_528_123e-3, 1e-7, 1e-6),
            ("PhaseShift", None, 6.013_964_098_791_527e-1, 1.156_175_063_488_139e-2, 1e-7, 1e-6),
        ],
    );
    assert_approx_eq(find_derived(&f26, 0, "Frequency"), 1.0 / 5.202_063_112_354_752, 1e-12, 1e-7, "std_sine Frequency");

    let (x_eg, e_eg) = (linspace(0.0, 10.0, 15), eps_vec(15, 0.08));
    let y_eg: Vec<f64> = x_eg.iter().zip(&e_eg).map(|(&x, &e)| 1.4 + 0.32 * x + e).collect();
    let f27 = check(
        "log_exponential_growth",
        &x_eg,
        &y_eg,
        vec![],
        (4.775_218_106_565_660e-2, 1e-8),
        &[
            ("LnY0", None, 1.398_569_480_328_489, 2.979_430_821_250_200e-2, 1e-7, 1e-6),
            ("k", None, 3.199_966_017_334_808e-1, 5.070_770_442_596_213e-3, 1e-7, 1e-6),
        ],
    );
    assert_approx_eq(find_derived(&f27, 0, "Y0"), 1.398_569_480_328_489_f64.exp(), 1e-12, 1e-7, "log_exp_growth Y0");
    assert_approx_eq(find_derived(&f27, 0, "DoublingTime"), std::f64::consts::LN_2 / 3.199_966_017_334_808e-1, 1e-12, 1e-7, "log_exp_growth DT");

    // 28–32. 5 Linear-Quadratic variants
    let d_rad = linspace(0.0, 8.0, 17);
    let s_true: Vec<f64> = d_rad.iter().map(|&d| (-(0.25 * d + 0.04 * d * d)).exp()).collect();
    let (e04, e12, e0004) = (eps_vec(17, 0.4), eps_vec(17, 1.2), eps_vec(17, 0.004));
    let lq_cases: [LqCase<'_>; 5] = [
        (
            "linear_quadratic_percent_surviving",
            "lq_pct_surv",
            s_true.iter().zip(&e04).map(|(&s, &e)| 100.0 * s + e).collect(),
            1.318_694_239_299_135,
            &[
                ("Alpha", None, 2.502_737_974_801_110e-1, 2.573_528_469_035_298e-3, 1e-7, 1e-6),
                ("Beta", None, 3.990_196_152_215_829e-2, 8.204_432_584_106_746e-4, 1e-7, 1e-6),
            ],
        ),
        (
            "linear_quadratic_number_surviving",
            "lq_num_surv",
            s_true.iter().zip(&e12).map(|(&s, &e)| 250.0 * s + e).collect(),
            11.860_475_205_690_29,
            &[
                ("Y0", None, 249.923_939_656_282_4, 7.940_674_310_019_946e-1, 1e-7, 1e-6),
                ("Alpha", None, 2.500_235_703_254_511e-1, 4.510_060_700_761_018e-3, 1e-7, 1e-6),
                ("Beta", None, 3.993_933_352_590_014e-2, 1.179_420_777_133_129e-3, 1e-7, 1e-6),
            ],
        ),
        (
            "linear_quadratic_fraction_dead",
            "lq_frac_dead",
            s_true.iter().zip(&e0004).map(|(&s, &e)| (1.0 - s) + e).collect(),
            1.318_699_372_390_538e-4,
            &[
                ("Alpha", None, 2.497_271_295_931_443e-1, 2.575_208_078_293_857e-3, 1e-7, 1e-6),
                ("Beta", None, 4.009_771_866_598_412e-2, 8.217_898_708_169_711e-4, 1e-7, 1e-6),
            ],
        ),
        (
            "linear_quadratic_percent_dead",
            "lq_pct_dead",
            s_true.iter().zip(&e04).map(|(&s, &e)| 100.0 * (1.0 - s) + e).collect(),
            1.318_699_372_390_552,
            &[
                ("Alpha", None, 2.497_271_296_116_387e-1, 2.575_208_001_757_539e-3, 1e-7, 1e-6),
                ("Beta", None, 4.009_771_866_567_887e-2, 8.217_898_550_183_944e-4, 1e-7, 1e-6),
            ],
        ),
        (
            "linear_quadratic_number_dead",
            "lq_num_dead",
            s_true.iter().zip(&e12).map(|(&s, &e)| 250.0 * (1.0 - s) + e).collect(),
            11.749_779_654_311_22,
            &[
                ("Y0", None, 250.261_988_014_177_2, 6.989_268_480_314_191e-1, 1e-7, 1e-6),
                ("Alpha", None, 2.502_105_307_724_532e-1, 3.474_698_195_216_891e-3, 1e-7, 1e-6),
                ("Beta", None, 3.962_427_541_535_845e-2, 1.652_151_768_431_412e-3, 1e-7, 1e-6),
            ],
        ),
    ];
    for (id, _, ys, rss, params) in lq_cases {
        check(id, &d_rad, &ys, vec![], (rss, 1e-8), params);
    }

    // 33. boltzmann_sigmoid & 34. power_series_two_term
    let (x_v, e_v) = (linspace(-80.0, 40.0, 25), eps_vec(25, 0.25));
    let y_v: Vec<f64> = x_v.iter().zip(&e_v).map(|(&x, &e)| 2.0 + 46.0 / (1.0 + ((-22.5 - x) / 8.4).exp()) + e).collect();
    check(
        "boltzmann_sigmoid",
        &x_v,
        &y_v,
        vec![],
        (7.804_109_135_751_978e-1, 1e-8),
        &[
            ("Bottom", None, 1.990_204_764_220_386, 8.054_063_323_569_023e-2, 1e-7, 1e-6),
            ("Top", None, 48.020_109_938_475_06, 7.462_177_578_520_970e-2, 1e-7, 1e-6),
            ("V50", None, -22.494_320_668_609_93, 8.619_693_768_392_962e-2, 1e-7, 1e-6),
            ("Slope", None, 8.412_280_357_822_368, 7.698_389_384_453_332e-2, 1e-7, 1e-6),
        ],
    );

    let (x_p, e_p) = (linspace(0.5, 6.0, 20), eps_vec(20, 0.08));
    let y_p: Vec<f64> = x_p.iter().zip(&e_p).map(|(&x, &e)| 2.4 * x.powf(0.7) + 0.35 * x.powf(2.2) + e).collect();
    check(
        "power_series_two_term",
        &x_p,
        &y_p,
        local_inits(&[("A", 2.2), ("B", 0.65), ("C", 0.3), ("D", 2.1)]),
        (5.987_795_085_953_559e-2, 1e-7),
        &[
            ("A", None, 2.417_389_906_999_267, 6.060_556_214_041_116e-2, 1e-5, 1e-4),
            ("B", None, 7.146_916_314_630_117e-1, 4.994_507_426_237_418e-2, 1e-5, 1e-4),
            ("C", None, 3.300_571_758_925_960e-1, 6.093_004_348_269_092e-2, 1e-5, 1e-4),
            ("D", None, 2.224_440_087_679_484, 7.414_762_724_647_737e-2, 1e-5, 1e-4),
        ],
    );
}

#[test]
fn d1_r18_3_analytical_gradients_and_domain_guards() {
    let grad_cases: &[(&str, f64, &[f64])] = &[
        ("agonist_vs_response_normalized", 3.5, &[2.5]),
        ("agonist_vs_response_normalized_var", 3.5, &[2.5, 1.6]),
        ("inhibitor_vs_response_normalized", 3.5, &[3.2]),
        ("inhibitor_vs_response_normalized_var", 3.5, &[3.2, -1.4]),
        ("two_phase_association", 2.5, &[10.0, 100.0, 40.0, 2.5, 0.35]),
        ("log_exponential_growth", 4.0, &[1.4, 0.32]),
        ("boltzmann_sigmoid", -15.0, &[2.0, 48.0, -22.5, 8.4]),
        ("linear_quadratic_percent_surviving", 3.0, &[0.25, 0.04]),
        ("linear_quadratic_number_surviving", 3.0, &[250.0, 0.25, 0.04]),
        ("linear_quadratic_fraction_dead", 3.0, &[0.25, 0.04]),
        ("linear_quadratic_percent_dead", 3.0, &[0.25, 0.04]),
        ("linear_quadratic_number_dead", 3.0, &[250.0, 0.25, 0.04]),
        ("gaussian_area", 4.5, &[25.0, 3.2, 1.8]),
        ("standard_sine_wave", 2.3, &[4.5, 5.2, 0.6]),
        ("two_lines_intersection", 7.0, &[4.2, 11.5, 1.8]),
    ];
    for &(id, x, params) in grad_cases {
        let m = get_builtin_model(id).unwrap();
        let mut g = vec![0.0; params.len()];
        assert!(m.grad(&[x], params, 0, &mut g), "{id} grad() returned false");
        for k in 0..params.len() {
            let h = 1e-6 * params[k].abs().max(1.0);
            let (mut p_hi, mut p_lo) = (params.to_vec(), params.to_vec());
            p_hi[k] += h;
            p_lo[k] -= h;
            let fd = (m.eval(&[x], &p_hi, 0).unwrap() - m.eval(&[x], &p_lo, 0).unwrap()) / (2.0 * h);
            assert_approx_eq(g[k], fd, 1e-7, 1e-6, &format!("{id} grad[{k}] vs FD"));
        }
    }

    let none_cases: &[(&str, f64, &[f64])] = &[
        ("power_series_two_term", 0.0, &[1.0, -0.5, 1.0, 1.0]),
        ("power_series_two_term", -2.0, &[1.0, 0.5, 1.0, 2.0]),
        ("power_series_two_term", f64::NAN, &[1.0, 1.0, 1.0, 2.0]),
        ("gaussian_area", 1.0, &[10.0, 0.0, 0.0]),
        ("gaussian_area", 1.0, &[10.0, 0.0, -1.0]),
        ("lognormal_area", 0.0, &[10.0, 2.0, 1.5]),
        ("lognormal_area", 1.0, &[10.0, 2.0, 1.0]),
        ("standard_sine_wave", 1.0, &[1.0, 0.0, 0.0]),
        ("boltzmann_sigmoid", 0.0, &[0.0, 1.0, 0.0, 0.0]),
    ];
    for &(id, x, params) in none_cases {
        assert!(get_builtin_model(id).unwrap().eval(&[x], params, 0).is_none(), "{id}({x}, {params:?}) expected None");
    }
    assert_approx_eq(
        get_builtin_model("power_series_two_term").unwrap().eval(&[-2.0], &[1.0, 3.0, 2.0, 2.0], 0).unwrap(),
        0.0,
        1e-12,
        1e-12,
        "X<0 integer exponents",
    );

    let (open_lo, open_hi) = kb_ci_from_pkb_ci(CiBound::Open, CiBound::Finite(8.0));
    assert_eq!(open_hi, CiBound::Open);
    assert!(matches!(open_lo, CiBound::Finite(v) if (v - 1e-8).abs() < 1e-18));
}

fn check_grad_vs_fd_general(model: &dyn ParametricModel, x: f64, params: &[f64], ds: usize, one_sided_pos: &[usize], rel_tol: f64) {
    let mut grad = vec![0.0; params.len()];
    assert!(model.grad(&[x], params, ds, &mut grad), "{} grad() returned false", model.id());
    for k in 0..params.len() {
        let fd = if one_sided_pos.contains(&k) {
            let h = 1e-12;
            let mut p_hi = params.to_vec();
            p_hi[k] += h;
            (model.eval(&[x], &p_hi, ds).unwrap() - model.eval(&[x], params, ds).unwrap()) / h
        } else {
            let h = 1e-6 * params[k].abs().max(1e-8);
            let (mut p_hi, mut p_lo) = (params.to_vec(), params.to_vec());
            p_hi[k] += h;
            p_lo[k] -= h;
            (model.eval(&[x], &p_hi, ds).unwrap() - model.eval(&[x], &p_lo, ds).unwrap()) / (2.0 * h)
        };
        assert_approx_eq(grad[k], fd, 1e-6, rel_tol, &format!("{} grad[{k}]", model.id()));
    }
}

#[test]
fn aud_03_13_40_41_gaddum_schild_grad_domain_guards_and_allosteric_limits() {
    // AUD-03-13: exact analytical gradient at AntagConc == 0.0 and simplex-disabled escape.
    let (u_log, u_conc, s_conc) = (
        get_builtin_model("ec50_shift_gaddum_schild_unity_log").unwrap(),
        get_builtin_model("ec50_shift_gaddum_schild_unity_conc").unwrap(),
        get_builtin_model("ec50_shift_gaddum_schild_conc").unwrap(),
    );
    let (exp_ul, exp_sc) = (-90.0 * 0.25 * 1.2 * 10.0_f64.powf(7.8), -90.0 * 0.25 * 1.2 * 10.0_f64.powf(7.5));
    for (m, x0, p0, x1, p1, a_idx, exp_da) in [
        (u_log.as_ref(), -7.0, &[5.0, 95.0, -7.0, 1.2, 7.8, 0.0][..], -6.5, &[5.0, 95.0, -7.0, 1.2, 7.8, 1e-7][..], 5, exp_ul),
        (u_conc.as_ref(), 1e-7, &[5.0, 95.0, 1e-7, 1.2, 7.8, 0.0], 3e-7, &[5.0, 95.0, 1e-7, 1.2, 7.8, 1e-7], 5, exp_ul),
        (s_conc.as_ref(), 1e-7, &[5.0, 95.0, 1e-7, 1.2, 7.5, 1.0, 0.0], 3e-7, &[5.0, 95.0, 1e-7, 1.2, 7.5, 1.15, 1e-7], 6, exp_sc),
    ] {
        let mut g = vec![0.0; p0.len()];
        assert!(m.grad(&[x0], p0, 0, &mut g));
        assert_approx_eq(g[a_idx], exp_da, 1e-3, 1e-12, m.id());
        check_grad_vs_fd_general(m, x0, p0, 0, &[a_idx], 1e-4);
        check_grad_vs_fd_general(m, x1, p1, 0, &[], 1e-6);
    }
    let mut g_sc = vec![0.0; 7];
    assert!(!s_conc.grad(&[1e-7], &[5.0, 95.0, 1e-7, 1.2, 7.5, 0.5, 0.0], 0, &mut g_sc));
    assert!(s_conc.grad(&[1e-7], &[5.0, 95.0, 1e-7, 1.2, 7.5, 1.5, 0.0], 0, &mut g_sc) && g_sc[6] == 0.0);

    let log_xs = linspace(-9.0, -3.0, 15);
    let cfg_esc = SolverConfig {
        use_simplex_warm_start: false,
        parameter_rules: vec![
            rule_const("Bottom", 5.0),
            rule_const("Top", 95.0),
            rule_const("LogEC50", -7.0),
            rule_const("HillSlope", 1.2),
            rule_const("pKB", 7.8),
        ],
        ..cfg_fast()
    };
    let fit_esc = fit_with(u_log.as_ref(), &[series_eval(u_log.as_ref(), 0, "UnknownB", &log_xs, &[5.0, 95.0, -7.0, 1.2, 7.8, 5e-7])], &cfg_esc);
    assert_converged(&fit_esc, "GaddumSchildUnity escape from AntagConc=0");
    assert_approx_eq(param_ds_val(&fit_esc, 0, "AntagConc"), 5e-7, 1e-13, 1e-6, "recovered AntagConc");

    // AUD-03-40: LinearDoseResponse, GaddumSchild, and EnzymeInhibition domain guards & default_bounds.
    for (id, sat) in [("agonist_vs_response_4pl", 100.0), ("inhibitor_vs_response_4pl", 0.0)] {
        let m = get_builtin_model(id).unwrap();
        for (x, p) in [
            (1.0, [0.0, 100.0, f64::NAN, 1.0]),
            (1.0, [0.0, 100.0, 1.0, f64::NAN]),
            (1.0, [f64::NAN, 100.0, 1.0, 1.0]),
            (f64::NAN, [0.0, 100.0, 1.0, 1.0]),
        ] {
            assert!(m.eval(&[x], &p, 0).is_none());
        }
        assert_eq!(m.eval(&[1e200], &[0.0, 100.0, 1e-200, 10.0], 0), Some(sat));
    }
    let gs = get_builtin_model("ec50_shift_gaddum_schild").unwrap();
    for p in [[0.0, 100.0, -7.0, 1.0, 7.0, 1.0, -5.0], [0.0, 100.0, -7.0, 1.0, f64::NAN, 1.0, 0.0], [0.0, 100.0, -7.0, 1.0, 7.0, 1.0, f64::NAN]] {
        assert!(gs.eval(&[-7.0], &p, 0).is_none());
    }
    assert_eq!((gs.default_bounds()[5].0, gs.default_bounds()[6].0), (Some(1e-4), Some(0.0)));
    for (id, valid, neg_i) in [
        ("competitive_inhibition", &[100.0, 5.0, 1.0, 2.0][..], &[100.0, 5.0, 1.0, -2.0][..]),
        ("noncompetitive_inhibition", &[100.0, 5.0, 1.0, 2.0], &[100.0, 5.0, 1.0, -2.0]),
        ("uncompetitive_inhibition", &[100.0, 5.0, 1.0, 2.0], &[100.0, 5.0, 1.0, -2.0]),
        ("mixed_model_inhibition", &[100.0, 5.0, 1.0, 2.0, 2.0], &[100.0, 5.0, 1.0, 2.0, -2.0]),
    ] {
        let m = get_builtin_model(id).unwrap();
        assert!(m.eval(&[2.5], valid, 0).is_some() && m.eval(&[2.5], neg_i, 0).is_none() && m.eval(&[5.0], neg_i, 0).is_none());
        let mut with_nan = valid.to_vec();
        with_nan[0] = f64::NAN;
        assert!(m.eval(&[2.5], &with_nan, 0).is_none());
        let b = m.default_bounds();
        assert!(b[0].0 == Some(0.0) && b[1].0.is_some_and(|v| v > 0.0) && b[2].0.is_some_and(|v| v > 0.0) && b.last().unwrap().0 == Some(0.0));
    }

    // AUD-03-41: AllostericModulatorCompetition extreme LogAlpha regimes.
    let allo = get_builtin_model("allosteric_modulator_competition").unwrap();
    let (y0, hot, kd) = (1000.0_f64, 2.0, 1.0);
    for (lbl, x, log_kb, log_a, expected, atol) in [
        ("NAM log_u=10", 4.0, -6.0, -320.0, y0 * (hot + kd) / (hot + kd * (1.0 + 1e10)), 1e-18),
        ("NAM log_u=-2", -8.0, -6.0, -320.0, y0 * (hot + kd) / (hot + kd * 1.01), 1e-12),
        ("PAM log_u=10", 4.0, -6.0, 320.0, y0 * (hot + kd) / hot, 1e-10),
        ("PAM both>100", 120.0, 0.0, 350.0, y0 * (hot + kd) / hot, 1e-10),
    ] {
        assert_approx_eq(allo.eval(&[x], &[y0, log_kb, log_a, hot, kd], 0).unwrap(), expected, atol, 1e-12, lbl);
    }
}

#[test]
fn aud_03_27_default_parameter_rules_global_and_ecf() {
    let m1 = get_builtin_model("one_site_total_and_ns_global").unwrap();
    let (xs1, p1) = ([0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0], [1800.0, 3.5, 42.0, 85.0]);
    let fit1 = fit_with(m1.as_ref(), &[series_eval(m1.as_ref(), 0, "Total", &xs1, &p1), series_eval(m1.as_ref(), 1, "NS", &xs1, &p1)], &cfg_fast());
    assert_converged(&fit1, "one_site_total_and_ns_global default rules");
    assert_eq!(fit1.k_free, 4);
    for (ds, name, exp, atol) in [(0, "Bmax", p1[0], 1e-4), (0, "Kd", p1[1], 1e-5), (1, "NS", p1[2], 1e-5), (1, "Background", p1[3], 1e-4)] {
        assert_approx_eq(dataset_param_val(&fit1, ds, name), exp, atol, 1e-5, name);
    }

    let m2 = get_builtin_model("two_site_total_and_ns_global").unwrap();
    let (xs2, p2) = ([0.05, 0.15, 0.4, 1.0, 2.5, 6.0, 15.0, 35.0, 80.0, 160.0], [120.0, 0.45, 240.0, 22.0, 0.75, 14.0]);
    let fit2 = fit_with(m2.as_ref(), &[series_eval(m2.as_ref(), 0, "Total", &xs2, &p2), series_eval(m2.as_ref(), 1, "NS", &xs2, &p2)], &cfg_fast());
    assert_converged(&fit2, "two_site_total_and_ns_global default rules");
    assert_eq!(fit2.k_free, 6);
    assert_approx_eq(dataset_param_val(&fit2, 1, "NS"), p2[4], 1e-5, 1e-5, "global2 NS");
    assert_approx_eq(dataset_param_val(&fit2, 1, "Background"), p2[5], 1e-4, 1e-5, "global2 Background");

    let lx = linspace(-9.0, -3.5, 12);
    let cx: Vec<f64> = lx.iter().map(|&v| 10.0_f64.powf(v)).collect();
    for (id, xs, p_base, rule_name, concs, checks) in [
        (
            "ec50_shift_gaddum_schild",
            lx.as_slice(),
            [5.0, 95.0, -7.0, 1.1, 7.5, 1.15],
            "AntagConc",
            &[0.0, 1e-7, 1e-6][..],
            &[("pA2", 7.5, 1e-4)][..],
        ),
        ("ec50_shift_gaddum_schild_conc", cx.as_slice(), [5.0, 95.0, 1e-7, 1.1, 7.5, 1.15], "AntagConc", &[0.0, 1e-7, 1e-6], &[("pA2", 7.5, 1e-4)]),
        (
            "allosteric_ec50_shift",
            lx.as_slice(),
            [4.0, 96.0, -7.0, 1.1, 2e-6, 0.1],
            "ModConc",
            &[0.0, 1e-6, 1e-5, 1e-4],
            &[("Kb", 2e-6, 1e-10), ("alpha", 0.1, 1e-4)],
        ),
        (
            "allosteric_ec50_shift_conc",
            cx.as_slice(),
            [4.0, 96.0, 1e-7, 1.1, 2e-6, 0.1],
            "ModConc",
            &[0.0, 1e-6, 1e-5, 1e-4],
            &[("Kb", 2e-6, 1e-10), ("alpha", 0.1, 1e-4)],
        ),
    ] {
        let m = get_builtin_model(id).unwrap();
        let ds: Vec<_> = concs
            .iter()
            .enumerate()
            .map(|(k, &c)| series_eval(m.as_ref(), k, &format!("C={c}"), xs, &[p_base[0], p_base[1], p_base[2], p_base[3], p_base[4], p_base[5], c]))
            .collect();
        let per_ds: Vec<(usize, f64)> = concs.iter().copied().enumerate().collect();
        let fit = fit_with(m.as_ref(), &ds, &cfg_fast_rules(vec![rule_per_ds(rule_name, &per_ds)]));
        assert_converged(&fit, id);
        assert_eq!(fit.k_free, 6, "{id} k_free");
        for &(pname, exp, atol) in checks {
            assert_approx_eq(param_ds_val(&fit, 0, pname), exp, atol, 1e-4, &format!("{id} {pname}"));
        }
    }

    for (id, xs, true_mid, pname) in
        [("log_agonist_vs_response_ecf", lx.as_slice(), -7.0, "LogECF"), ("agonist_vs_response_ecf", cx.as_slice(), 1e-7, "ECF")]
    {
        let m = get_builtin_model(id).unwrap();
        assert!(
            m.default_parameter_rules(&[])
                .iter()
                .any(|r| r.name == "F" && matches!(r.scope, ParameterScope::Constant(v) if (v - 50.0).abs() < 1e-12))
        );
        let fit = fit_with(m.as_ref(), &[series_eval(m.as_ref(), 0, id, xs, &[5.0, 95.0, true_mid, 1.3, 50.0])], &cfg_fast());
        assert_converged(&fit, id);
        assert_eq!(fit.k_free, 4);
        assert_approx_eq(param_ds_val(&fit, 0, pname), true_mid, 1e-12, 1e-5, &format!("{id} {pname}"));
    }
}
