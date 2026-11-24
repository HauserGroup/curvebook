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

use curvebook_doc::{CiBound, FitMethod, ParameterRule, ParameterScope, WeightingScheme};
use curvebook_fit::library::pharma::kb_ci_from_pkb_ci;
use curvebook_fit::{DataPoint, DatasetSeries, FitStatus, HookContext, NonlinearFitResult, SolverConfig, fit_nonlinear, get_builtin_model};
use curvebook_math::ConfidenceLevel;
use curvebook_testkit::assert_approx_eq;

fn linspace(start: f64, end: f64, n: usize) -> Vec<f64> {
    if n <= 1 {
        return vec![start];
    }
    let step = (end - start) / (n - 1) as f64;
    (0..n).map(|i| start + i as f64 * step).collect()
}

fn logspace(exp_start: f64, exp_end: f64, n: usize) -> Vec<f64> {
    linspace(exp_start, exp_end, n).into_iter().map(|e| 10.0_f64.powf(e)).collect()
}

fn eps_vec(n: usize, amp: f64) -> Vec<f64> {
    (1..=n).map(|i| amp * (2.3 * i as f64 + 0.7).sin()).collect()
}

fn make_series(ds_idx: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
    let points = xs
        .iter()
        .zip(ys.iter())
        .enumerate()
        .map(|(row, (&x, &y))| DataPoint {
            x: vec![x],
            y,
            sd: None,
            n: None,
            explicit_weight: None,
            source_subcolumn: 0,
            data_set_index: ds_idx,
            row,
        })
        .collect();
    DatasetSeries {
        data_set_index: ds_idx,
        label: label.to_owned(),
        points,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn rule_local(name: &str, init: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Local, initial_override: Some(init), lower_bound: None, upper_bound: None }
}

fn rule_shared(name: &str, init: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::SharedGlobal, initial_override: Some(init), lower_bound: None, upper_bound: None }
}

fn rule_const(name: &str, val: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Constant(val), initial_override: None, lower_bound: None, upper_bound: None }
}

fn rule_per_ds(name: &str, pairs: Vec<(usize, f64)>) -> ParameterRule {
    ParameterRule {
        name: name.to_owned(),
        scope: ParameterScope::PerDatasetConstant(pairs),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }
}

fn run_fit(model_id: &str, series: &[DatasetSeries], rules: Vec<ParameterRule>, profile_ci: bool) -> NonlinearFitResult {
    let model = get_builtin_model(model_id).unwrap_or_else(|| panic!("missing built-in model `{model_id}`"));
    let cfg = SolverConfig {
        method: FitMethod::LeastSquares,
        weighting: WeightingScheme::None,
        parameter_rules: rules,
        confidence: ConfidenceLevel::DEFAULT_95,
        max_iters: 4_000,
        compute_profile_ci: profile_ci,
        compute_hougaard: false,
        compute_bands: false,
        compute_prediction_bands: false,
        compute_runs_test: false,
        compute_lack_of_fit: false,
        compute_normality: false,
        interpolate_unknowns: false,
        use_simplex_warm_start: true,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(model.as_ref(), series, &HookContext::default(), &cfg).unwrap_or_else(|e| panic!("fit failed for `{model_id}`: {e:?}"));
    assert!(
        matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous),
        "`{model_id}` did not converge: status={:?}, rss={:.6e}",
        fit.status,
        fit.rss
    );
    fit
}

fn assert_param(fit: &NonlinearFitResult, name: &str, ds: Option<usize>, exp_est: f64, exp_se: f64, rel_est: f64, rel_se: f64, ctx: &str) {
    let p = fit
        .parameters
        .iter()
        .find(|p| p.name == name && (ds.is_none() || p.dataset_index == ds))
        .unwrap_or_else(|| panic!("{ctx}: missing param `{name}` (ds={ds:?})"));
    assert_approx_eq(p.estimate, exp_est, 1e-12, rel_est, &format!("{ctx} {name} estimate"));
    assert_approx_eq(p.se, exp_se, 1e-12, rel_se, &format!("{ctx} {name} SE"));
}

fn derived_val(fit: &NonlinearFitResult, ds: usize, name: &str) -> f64 {
    fit.derived_scalars[ds].iter().find(|d| d.name == name).unwrap_or_else(|| panic!("missing derived scalar `{name}` on dataset {ds}")).value
}

#[test]
fn d1_r18_3_group1_linear_normalized_dose_response() {
    let x_c = logspace(-3.0, 3.0, 15);
    let eps = eps_vec(x_c.len(), 0.6);

    // 1. agonist_vs_response_normalized
    let y: Vec<f64> = x_c.iter().zip(&eps).map(|(&x, &e)| 100.0 * x / (2.5 + x) + e).collect();
    let fit = run_fit("agonist_vs_response_normalized", &[make_series(0, "A", &x_c, &y)], vec![], false);
    assert_param(&fit, "EC50", None, 2.499_406_558_767_244, 2.664_777_923_259_584e-2, 1e-7, 1e-6, "ag_norm");
    assert_approx_eq(fit.rss, 2.687_733_043_062_753, 1e-12, 1e-8, "ag_norm RSS");
    assert_approx_eq(derived_val(&fit, 0, "LogEC50"), 2.499_406_558_767_244_f64.log10(), 1e-12, 1e-7, "ag_norm LogEC50");

    // 2. agonist_vs_response_normalized_var
    let y: Vec<f64> = x_c.iter().zip(&eps).map(|(&x, &e)| 100.0 / (1.0 + (2.5 / x).powf(1.6)) + e).collect();
    let fit = run_fit("agonist_vs_response_normalized_var", &[make_series(0, "A", &x_c, &y)], vec![], false);
    assert_param(&fit, "EC50", None, 2.496_299_008_767_880, 2.172_672_094_891_990e-2, 1e-7, 1e-6, "ag_norm_var");
    assert_param(&fit, "HillSlope", None, 1.593_175_580_361_390, 1.959_452_357_085_131e-2, 1e-7, 1e-6, "ag_norm_var");
    assert_approx_eq(fit.rss, 2.656_913_224_363_825, 1e-12, 1e-8, "ag_norm_var RSS");

    // 3. inhibitor_vs_response_normalized
    let y: Vec<f64> = x_c.iter().zip(&eps).map(|(&x, &e)| 100.0 / (1.0 + x / 3.2) + e).collect();
    let fit = run_fit("inhibitor_vs_response_normalized", &[make_series(0, "A", &x_c, &y)], vec![], false);
    assert_param(&fit, "IC50", None, 3.200_201_430_578_598, 3.412_008_372_703_434e-2, 1e-7, 1e-6, "inh_norm");
    assert_approx_eq(fit.rss, 2.687_821_480_217_989, 1e-12, 1e-8, "inh_norm RSS");
    assert_approx_eq(derived_val(&fit, 0, "LogIC50"), 3.200_201_430_578_598_f64.log10(), 1e-12, 1e-7, "inh_norm LogIC50");

    // 4. inhibitor_vs_response_normalized_var (negative HillSlope convention)
    let y: Vec<f64> = x_c.iter().zip(&eps).map(|(&x, &e)| 100.0 / (1.0 + (3.2 / x).powf(-1.4)) + e).collect();
    let fit = run_fit("inhibitor_vs_response_normalized_var", &[make_series(0, "A", &x_c, &y)], vec![], false);
    assert_param(&fit, "IC50", None, 3.200_990_891_751_175, 2.973_625_823_296_110e-2, 1e-7, 1e-6, "inh_norm_var");
    assert_param(&fit, "HillSlope", None, -1.405_506_013_960_904, 1.617_448_382_318_552e-2, 1e-7, 1e-6, "inh_norm_var");
    assert_approx_eq(fit.rss, 2.664_034_882_214_670, 1e-12, 1e-8, "inh_norm_var RSS");
}

#[test]
fn d1_r18_3_group2_linear_special_dose_response() {
    // 5. agonist_vs_response_5pl
    let x5 = logspace(-3.0, 3.0, 19);
    let e5 = eps_vec(x5.len(), 0.25);
    let f5 = 2.0_f64.powf(1.0 / 1.8) - 1.0;
    let y5: Vec<f64> = x5.iter().zip(&e5).map(|(&x, &e)| 5.0 + 90.0 / (1.0 + f5 * (1.5 / x).powf(1.3)).powf(1.8) + e).collect();
    let fit = run_fit(
        "agonist_vs_response_5pl",
        &[make_series(0, "A", &x5, &y5)],
        vec![rule_local("Bottom", 4.0), rule_local("Top", 96.0), rule_local("EC50", 1.2), rule_local("HillSlope", 1.1), rule_local("S", 1.5)],
        false,
    );
    assert_param(&fit, "Bottom", None, 4.988_530_687_510_472, 8.050_035_619_633_227e-2, 1e-6, 1e-4, "5pl_conc");
    assert_param(&fit, "Top", None, 94.984_406_788_219_18, 9.408_706_958_077_390e-2, 1e-6, 1e-4, "5pl_conc");
    assert_param(&fit, "EC50", None, 1.501_069_798_007_869, 7.344_100_651_080_574e-3, 1e-6, 1e-4, "5pl_conc");
    assert_param(&fit, "HillSlope", None, 1.307_030_982_965_748, 1.841_954_018_801_089e-2, 1e-6, 1e-4, "5pl_conc");
    assert_param(&fit, "S", None, 1.760_033_200_788_555, 1.057_744_945_156_111e-1, 1e-5, 1e-4, "5pl_conc");
    assert_approx_eq(fit.rss, 5.760_455_706_317_876e-1, 1e-12, 1e-7, "5pl_conc RSS");

    // 6. biphasic_dose_response_conc
    let xb = logspace(-4.0, 4.0, 25);
    let eb = eps_vec(xb.len(), 0.3);
    let yb: Vec<f64> =
        xb.iter().zip(&eb).map(|(&x, &e)| 10.0 + 90.0 * (0.4 / (1.0 + (0.05 / x).powf(1.2)) + 0.6 / (1.0 + (50.0 / x).powf(1.1))) + e).collect();
    let fit = run_fit(
        "biphasic_dose_response_conc",
        &[make_series(0, "A", &xb, &yb)],
        vec![
            rule_local("Bottom", 8.0),
            rule_local("Top", 102.0),
            rule_local("Frac", 0.45),
            rule_local("EC50_1", 0.04),
            rule_local("EC50_2", 40.0),
            rule_local("nH1", 1.0),
            rule_local("nH2", 1.0),
        ],
        false,
    );
    assert_param(&fit, "Bottom", None, 9.987_709_379_427_113, 1.191_275_733_717_345e-1, 1e-6, 1e-5, "biphasic_conc");
    assert_param(&fit, "Top", None, 100.051_206_011_185_3, 1.471_734_501_536_028e-1, 1e-6, 1e-5, "biphasic_conc");
    assert_param(&fit, "Frac", None, 3.997_641_283_003_502e-1, 3.086_644_019_243_629e-3, 1e-6, 1e-5, "biphasic_conc");
    assert_param(&fit, "EC50_1", None, 4.993_229_168_927_931e-2, 1.025_315_155_300_658e-3, 1e-6, 1e-5, "biphasic_conc");
    assert_param(&fit, "EC50_2", None, 50.111_259_249_909_64, 7.418_082_744_639_449e-1, 1e-6, 1e-5, "biphasic_conc");
    assert_approx_eq(fit.rss, 1.119_341_645_690_582, 1e-12, 1e-7, "biphasic_conc RSS");

    // 7. bell_shaped_dose_response_conc
    let ybell: Vec<f64> = xb
        .iter()
        .zip(&eb)
        .map(|(&x, &e)| 90.0 + (10.0 - 90.0) / (1.0 + (x / 0.1).powf(1.3)) + (15.0 - 90.0) * (1.0 - 1.0 / (1.0 + (x / 20.0).powf(1.1))) + e)
        .collect();
    let fit = run_fit(
        "bell_shaped_dose_response_conc",
        &[make_series(0, "A", &xb, &ybell)],
        vec![
            rule_local("Plateau1", 12.0),
            rule_local("Plateau2", 18.0),
            rule_local("Dip", 88.0),
            rule_local("EC50_1", 0.08),
            rule_local("EC50_2", 18.0),
            rule_local("nH1", 1.1),
            rule_local("nH2", 1.0),
        ],
        false,
    );
    assert_param(&fit, "Plateau1", None, 9.994_866_331_562_198, 1.066_649_391_729_973e-1, 1e-6, 1e-5, "bell_conc");
    assert_param(&fit, "Plateau2", None, 15.038_656_829_999_82, 1.274_194_508_365_115e-1, 1e-6, 1e-5, "bell_conc");
    assert_param(&fit, "Dip", None, 89.973_740_709_371_68, 4.066_229_552_803_637e-1, 1e-6, 1e-5, "bell_conc");
    assert_param(&fit, "EC50_1", None, 9.995_311_337_529_553e-2, 1.124_332_210_329_994e-3, 1e-6, 1e-5, "bell_conc");
    assert_param(&fit, "EC50_2", None, 19.991_286_491_009_51, 2.822_559_739_650_533e-1, 1e-6, 1e-5, "bell_conc");
    assert_approx_eq(fit.rss, 1.121_930_505_265_480, 1e-12, 1e-7, "bell_conc RSS");

    // 8. operational_model_agonist_conc
    let xo = logspace(-3.0, 2.0, 16);
    let eo = eps_vec(xo.len(), 0.25);
    let yo: Vec<f64> = xo.iter().zip(&eo).map(|(&x, &e)| 5.0 + 95.0 / (1.0 + ((2.0 + x) / (1.5 * x)).powf(1.2)) + e).collect();
    let fit = run_fit(
        "operational_model_agonist_conc",
        &[make_series(0, "A", &xo, &yo)],
        vec![rule_local("Basal", 4.0), rule_const("Emax", 100.0), rule_local("KA", 1.8), rule_local("Tau", 1.4), rule_local("n", 1.1)],
        false,
    );
    assert_param(&fit, "Basal", None, 4.993_492_002_449_718, 9.233_084_449_236_710e-2, 1e-6, 1e-5, "op_ag_conc");
    assert_param(&fit, "KA", None, 1.995_570_691_391_321, 7.283_045_824_374_790e-2, 1e-6, 1e-5, "op_ag_conc");
    assert_param(&fit, "Tau", None, 1.498_694_304_294_045, 1.653_193_771_435_948e-2, 1e-6, 1e-5, "op_ag_conc");
    assert_param(&fit, "n", None, 1.200_297_985_438_651, 2.271_855_521_335_176e-2, 1e-6, 1e-5, "op_ag_conc");
    assert_approx_eq(fit.rss, 4.671_387_856_843_455e-1, 1e-12, 1e-7, "op_ag_conc RSS");

    // 9. ec50_shift_gaddum_schild_conc (3 datasets)
    let x_g = logspace(-9.0, -4.0, 12);
    let eg = eps_vec(x_g.len(), 0.35);
    let antags = [0.0, 1e-7, 1e-6];
    let ds_g: Vec<DatasetSeries> = antags
        .iter()
        .enumerate()
        .map(|(k, &b_conc)| {
            let dr = if b_conc > 0.0 { 1.0 + (b_conc * 10.0_f64.powf(7.5)).powf(1.15) } else { 1.0 };
            let ec_eff = 1e-7 * dr;
            let sign = if (k + 1) % 2 == 0 { -1.0 } else { 1.0 };
            let y: Vec<f64> = x_g.iter().zip(&eg).map(|(&x, &e)| 5.0 + 95.0 / (1.0 + (ec_eff / x).powf(1.2)) + sign * e).collect();
            make_series(k, &format!("B={b_conc}"), &x_g, &y)
        })
        .collect();
    let fit = run_fit(
        "ec50_shift_gaddum_schild_conc",
        &ds_g,
        vec![
            rule_shared("Bottom", 4.0),
            rule_shared("Top", 98.0),
            rule_shared("EC50", 1.2e-7),
            rule_shared("HillSlope", 1.1),
            rule_shared("pA2", 7.3),
            rule_shared("SchildSlope", 1.0),
            rule_per_ds("AntagConc", vec![(0, 0.0), (1, 1e-7), (2, 1e-6)]),
        ],
        false,
    );
    assert_param(&fit, "EC50", None, 9.995_193_242_356_752e-8, 6.968_528_657_848_965e-10, 1e-5, 1e-4, "gs_conc");
    assert_param(&fit, "pA2", None, 7.499_837_941_579_449, 5.721_919_206_944_623e-3, 1e-6, 1e-4, "gs_conc");
    assert_param(&fit, "SchildSlope", None, 1.150_167_152_156_678, 4.493_679_352_400_328e-3, 1e-6, 1e-4, "gs_conc");
    assert_approx_eq(fit.rss, 2.050_378_685_049_887, 1e-12, 1e-7, "gs_conc RSS");

    // 10. allosteric_ec50_shift_conc (3 datasets)
    let mods = [0.0, 1e-6, 1e-5];
    let ds_a: Vec<DatasetSeries> = mods
        .iter()
        .enumerate()
        .map(|(k, &m_conc)| {
            let ec_eff = 1e-7 * (1.0 + m_conc / 2e-6) / (1.0 + 0.1 * m_conc / 2e-6);
            let sign = if (k + 1) % 2 == 0 { -1.0 } else { 1.0 };
            let y: Vec<f64> = x_g.iter().zip(&eg).map(|(&x, &e)| 4.0 + 92.0 / (1.0 + (ec_eff / x).powf(1.1)) + sign * e).collect();
            make_series(k, &format!("M={m_conc}"), &x_g, &y)
        })
        .collect();
    let fit = run_fit(
        "allosteric_ec50_shift_conc",
        &ds_a,
        vec![
            rule_shared("Bottom", 3.0),
            rule_shared("Top", 95.0),
            rule_shared("EC50", 1.1e-7),
            rule_shared("HillSlope", 1.0),
            rule_shared("Kb", 1.8e-6),
            rule_shared("alpha", 0.12),
            rule_per_ds("ModConc", vec![(0, 0.0), (1, 1e-6), (2, 1e-5)]),
        ],
        false,
    );
    assert_param(&fit, "EC50", None, 9.998_569_874_422_053e-8, 7.626_438_361_113_712e-10, 1e-5, 1e-4, "allo_shift_conc");
    assert_param(&fit, "Kb", None, 1.994_097_903_837_269e-6, 8.354_668_897_651_919e-8, 1e-5, 1e-4, "allo_shift_conc");
    assert_param(&fit, "alpha", None, 1.003_079_874_636_961e-1, 5.814_833_128_540_418e-3, 1e-5, 1e-4, "allo_shift_conc");
    assert_approx_eq(fit.rss, 2.050_173_261_212_730, 1e-12, 1e-7, "allo_shift_conc RSS");

    // 11. agonist_vs_response_ecf (F = 80 fixed)
    let x_c = logspace(-3.0, 3.0, 15);
    let ec = eps_vec(x_c.len(), 0.35);
    let ec50_80 = 4.0 * (20.0_f64 / 80.0).powf(1.0 / 1.4);
    let y_ecf: Vec<f64> = x_c.iter().zip(&ec).map(|(&x, &e)| 5.0 + 95.0 / (1.0 + (ec50_80 / x).powf(1.4)) + e).collect();
    let fit = run_fit(
        "agonist_vs_response_ecf",
        &[make_series(0, "A", &x_c, &y_ecf)],
        vec![rule_local("Bottom", 4.0), rule_local("Top", 98.0), rule_local("ECF", 3.5), rule_local("HillSlope", 1.2), rule_const("F", 80.0)],
        false,
    );
    assert_param(&fit, "ECF", None, 3.988_876_092_453_611, 4.676_302_238_424_108e-2, 1e-6, 1e-5, "ecf_conc");
    assert_param(&fit, "HillSlope", None, 1.401_773_407_489_595, 1.264_599_667_081_122e-2, 1e-6, 1e-5, "ecf_conc");
    assert_approx_eq(fit.rss, 9.089_563_850_842_785e-1, 1e-12, 1e-7, "ecf_conc RSS");

    // 12. absolute_ic50_4pl_conc (Control0 = 0, Control100 = 100)
    let ic_rel = 5.0 * ((50.0_f64 - 15.0) / (95.0 - 50.0)).powf(1.0 / 1.3);
    let y_abs: Vec<f64> = x_c.iter().zip(&ec).map(|(&x, &e)| 15.0 + 80.0 / (1.0 + (x / ic_rel).powf(1.3)) + e).collect();
    let fit = run_fit(
        "absolute_ic50_4pl_conc",
        &[make_series(0, "A", &x_c, &y_abs)],
        vec![
            rule_local("Bottom", 14.0),
            rule_local("Top", 96.0),
            rule_local("IC50", 4.5),
            rule_local("HillSlope", 1.2),
            rule_const("Control0", 0.0),
            rule_const("Control100", 100.0),
        ],
        false,
    );
    assert_param(&fit, "IC50", None, 4.996_341_798_701_130, 4.049_792_661_980_433e-2, 1e-6, 1e-5, "abs_ic50_conc");
    assert_param(&fit, "HillSlope", None, 1.303_665_520_752_858, 1.391_285_778_600_612e-2, 1e-6, 1e-5, "abs_ic50_conc");
    assert_approx_eq(fit.rss, 9.082_744_230_454_814e-1, 1e-12, 1e-7, "abs_ic50_conc RSS");
    assert!(derived_val(&fit, 0, "RelativeIC50") > 0.0);
}

#[test]
fn d1_r18_3_group3_mechanistic_pharmacology_and_reversed_kb_ci() {
    // 13 & 14. operational_model_depletion_log & _conc (verify default_parameter_rules auto-shares!)
    let log_x = linspace(-8.0, -3.0, 14);
    let conc_x: Vec<f64> = log_x.iter().map(|&lx| 10.0_f64.powf(lx)).collect();
    let ed = eps_vec(log_x.len(), 0.3);
    let taus = [5.0, 0.8];
    let ds_dep_log: Vec<DatasetSeries> = taus
        .iter()
        .enumerate()
        .map(|(k, &tau)| {
            let sign = if k == 1 { -1.0 } else { 1.0 };
            let y: Vec<f64> = conc_x.iter().zip(&ed).map(|(&a, &e)| 3.0 + 102.0 / (1.0 + ((1e-5 + a) / (tau * a)).powf(1.15)) + sign * e).collect();
            make_series(k, &format!("tau={tau}"), &log_x, &y)
        })
        .collect();
    let ds_dep_conc: Vec<DatasetSeries> = taus
        .iter()
        .enumerate()
        .map(|(k, &tau)| {
            let sign = if k == 1 { -1.0 } else { 1.0 };
            let y: Vec<f64> = conc_x.iter().zip(&ed).map(|(&a, &e)| 3.0 + 102.0 / (1.0 + ((1e-5 + a) / (tau * a)).powf(1.15)) + sign * e).collect();
            make_series(k, &format!("tau={tau}"), &conc_x, &y)
        })
        .collect();

    // Pass NO parameter rules to verify default_parameter_rules shares Basal/EffectMax/LogKA/n automatically!
    let fit_log = run_fit("operational_model_depletion_log", &ds_dep_log, vec![], false);
    assert_param(&fit_log, "Basal", None, 2.992_150_091_571_427, 9.326_783_428_658_374e-2, 1e-5, 1e-4, "op_dep_log");
    assert_param(&fit_log, "EffectMax", None, 105.162_672_303_416_7, 4.694_773_917_306_487e-1, 1e-5, 1e-4, "op_dep_log");
    assert_param(&fit_log, "LogKA", None, -5.000_635_301_018_728, 1.055_785_069_101_162e-2, 1e-5, 1e-4, "op_dep_log");
    assert_param(&fit_log, "LogTau", Some(0), 6.970_397_349_817_230e-1, 1.025_101_645_174_665e-2, 1e-5, 1e-4, "op_dep_log");
    assert_param(&fit_log, "LogTau", Some(1), -9.865_462_419_477_283e-2, 4.436_835_396_729_064e-3, 1e-5, 1e-4, "op_dep_log");
    assert_param(&fit_log, "n", None, 1.147_947_381_588_692, 1.213_801_703_548_078e-2, 1e-5, 1e-4, "op_dep_log");
    assert_approx_eq(fit_log.rss, 1.267_339_719_477_657, 1e-12, 1e-7, "op_dep_log RSS");

    let fit_conc = run_fit("operational_model_depletion_conc", &ds_dep_conc, vec![rule_shared("KA", 9e-6), rule_local("Tau", 2.0)], false);
    assert_param(&fit_conc, "KA", None, 9.985_382_380_859_280e-6, 2.427_481_574_406_685e-7, 1e-5, 1e-4, "op_dep_conc");
    assert_param(&fit_conc, "Tau", Some(0), 4.977_826_294_098_003, 1.174_958_051_352_043e-1, 1e-5, 1e-4, "op_dep_conc");
    assert_param(&fit_conc, "Tau", Some(1), 7.967_927_562_386_462e-1, 8.140_187_085_372_374e-3, 1e-5, 1e-4, "op_dep_conc");
    assert_approx_eq(fit_conc.rss, 1.267_339_719_477_645, 1e-12, 1e-7, "op_dep_conc RSS");

    // 15 & 16. ec50_shift_gaddum_schild_unity_log & _conc (with profile CI + reversed KB CI check)
    let x_g = logspace(-9.0, -4.0, 12);
    let lx_g: Vec<f64> = x_g.iter().map(|&x| x.log10()).collect();
    let eg = eps_vec(x_g.len(), 0.35);
    let antags = [0.0, 1e-7, 1e-6];
    let ds_su_log: Vec<DatasetSeries> = antags
        .iter()
        .enumerate()
        .map(|(k, &b_conc)| {
            let dr = 1.0 + b_conc * 10.0_f64.powf(7.8);
            let ec_eff = 1e-7 * dr;
            let sign = if (k + 1) % 2 == 0 { -1.0 } else { 1.0 };
            let y: Vec<f64> = x_g.iter().zip(&eg).map(|(&x, &e)| 5.0 + 95.0 / (1.0 + (ec_eff / x).powf(1.2)) + sign * e).collect();
            make_series(k, &format!("B={b_conc}"), &lx_g, &y)
        })
        .collect();
    let ds_su_conc: Vec<DatasetSeries> = antags
        .iter()
        .enumerate()
        .map(|(k, &b_conc)| {
            let dr = 1.0 + b_conc * 10.0_f64.powf(7.8);
            let ec_eff = 1e-7 * dr;
            let sign = if (k + 1) % 2 == 0 { -1.0 } else { 1.0 };
            let y: Vec<f64> = x_g.iter().zip(&eg).map(|(&x, &e)| 5.0 + 95.0 / (1.0 + (ec_eff / x).powf(1.2)) + sign * e).collect();
            make_series(k, &format!("B={b_conc}"), &x_g, &y)
        })
        .collect();

    let fit_su_log =
        run_fit("ec50_shift_gaddum_schild_unity_log", &ds_su_log, vec![rule_per_ds("AntagConc", vec![(0, 0.0), (1, 1e-7), (2, 1e-6)])], true);
    assert_param(&fit_su_log, "LogEC50", None, -7.000_188_832_014_302, 2.977_259_252_250_148e-3, 1e-6, 1e-4, "gs_unity_log");
    assert_param(&fit_su_log, "pKB", None, 7.800_042_360_182_306, 3.544_037_920_283_844e-3, 1e-6, 1e-4, "gs_unity_log");
    assert_approx_eq(fit_su_log.rss, 2.050_616_178_006_570, 1e-12, 1e-7, "gs_unity_log RSS");
    let kb_est = derived_val(&fit_su_log, 0, "KB");
    assert_approx_eq(kb_est, 10.0_f64.powf(-7.800_042_360_182_306), 1e-15, 1e-6, "gs_unity_log derived KB");

    // Verify reversed CI bounds: pKB lower -> KB upper, pKB upper -> KB lower.
    let pkb_param = fit_su_log.parameters.iter().find(|p| p.name == "pKB").unwrap();
    let (kb_lo, kb_hi) = kb_ci_from_pkb_ci(pkb_param.ci_lower, pkb_param.ci_upper);
    match (pkb_param.ci_lower, pkb_param.ci_upper, kb_lo, kb_hi) {
        (CiBound::Finite(pl), CiBound::Finite(pu), CiBound::Finite(kl), CiBound::Finite(kh)) => {
            assert!(pl < pkb_param.estimate && pkb_param.estimate < pu);
            assert!(kl < kb_est && kb_est < kh);
            assert_approx_eq(kl, 10.0_f64.powf(-pu), 1e-20, 1e-12, "KB lower from -pKB upper");
            assert_approx_eq(kh, 10.0_f64.powf(-pl), 1e-20, 1e-12, "KB upper from -pKB lower");
        }
        other => panic!("expected finite profile CIs for pKB and KB, got {other:?}"),
    }

    let fit_su_conc = run_fit(
        "ec50_shift_gaddum_schild_unity_conc",
        &ds_su_conc,
        vec![rule_shared("EC50", 1.1e-7), rule_per_ds("AntagConc", vec![(0, 0.0), (1, 1e-7), (2, 1e-6)])],
        false,
    );
    assert_param(&fit_su_conc, "EC50", None, 9.995_652_922_435_616e-8, 6.852_412_641_606_030e-10, 1e-5, 1e-4, "gs_unity_conc");
    assert_param(&fit_su_conc, "pKB", None, 7.800_042_476_587_072, 3.544_037_846_160_965e-3, 1e-6, 1e-4, "gs_unity_conc");
    assert_approx_eq(fit_su_conc.rss, 2.050_616_178_218_110, 1e-12, 1e-7, "gs_unity_conc RSS");

    // 17 & 18. ec50_shift_ratio_log & _conc (default_parameter_rules auto-shares!)
    let ds_sr_log: Vec<DatasetSeries> = [1e-7, 6.5e-7]
        .iter()
        .enumerate()
        .map(|(k, &ec_eff)| {
            let sign = if k == 1 { -1.0 } else { 1.0 };
            let y: Vec<f64> = x_g.iter().zip(&eg).map(|(&x, &e)| 4.0 + 94.0 / (1.0 + (ec_eff / x).powf(1.25)) + sign * e).collect();
            make_series(k, if k == 0 { "Control" } else { "Shifted" }, &lx_g, &y)
        })
        .collect();
    let ds_sr_conc: Vec<DatasetSeries> = [1e-7, 6.5e-7]
        .iter()
        .enumerate()
        .map(|(k, &ec_eff)| {
            let sign = if k == 1 { -1.0 } else { 1.0 };
            let y: Vec<f64> = x_g.iter().zip(&eg).map(|(&x, &e)| 4.0 + 94.0 / (1.0 + (ec_eff / x).powf(1.25)) + sign * e).collect();
            make_series(k, if k == 0 { "Control" } else { "Shifted" }, &x_g, &y)
        })
        .collect();

    let fit_sr_log = run_fit("ec50_shift_ratio_log", &ds_sr_log, vec![], false);
    assert_param(&fit_sr_log, "LogEC50Control", None, -7.000_314_708_472_887, 3.171_331_364_884_057e-3, 1e-6, 1e-4, "shift_ratio_log");
    assert_param(&fit_sr_log, "EC50Ratio", None, 6.497_343_711_255_527, 5.865_264_676_029_960e-2, 1e-6, 1e-4, "shift_ratio_log");
    assert_approx_eq(fit_sr_log.rss, 1.362_751_006_946_411, 1e-12, 1e-7, "shift_ratio_log RSS");

    let fit_sr_conc = run_fit("ec50_shift_ratio_conc", &ds_sr_conc, vec![], false);
    assert_param(&fit_sr_conc, "EC50Control", None, 9.992_756_200_052_560e-8, 7.296_970_732_635_028e-10, 1e-5, 1e-4, "shift_ratio_conc");
    assert_param(&fit_sr_conc, "EC50Ratio", None, 6.497_343_810_119_635, 5.865_265_318_704_808e-2, 1e-5, 1e-4, "shift_ratio_conc");
    assert_approx_eq(fit_sr_conc.rss, 1.362_751_006_946_840, 1e-12, 1e-7, "shift_ratio_conc RSS");
}

#[test]
fn d1_r18_3_group4_receptor_binding() {
    // 19. one_site_total_ligand_depletion (Swillens 1995)
    let x_cpm = linspace(500.0, 15000.0, 16);
    let e_cpm = eps_vec(x_cpm.len(), 8.0);
    let kd_cpm = 0.0025 * 0.5 * 1000.0 * 2000.0;
    let y_cpm: Vec<f64> = x_cpm
        .iter()
        .zip(&e_cpm)
        .map(|(&x, &e)| {
            let a = -(1.0 + 0.04);
            let b = kd_cpm * (1.0 + 0.04) + x * (1.0 + 2.0 * 0.04) + 3200.0;
            let c = -x * (0.04 * kd_cpm + 0.04 * x + 3200.0);
            (-b + (b * b - 4.0 * a * c).sqrt()) / (2.0 * a) + e
        })
        .collect();
    let fit = run_fit(
        "one_site_total_ligand_depletion",
        &[make_series(0, "Total", &x_cpm, &y_cpm)],
        vec![rule_const("SpecAct", 2000.0), rule_const("Vol", 0.5)],
        false,
    );
    assert_param(&fit, "Bmax", None, 3.211_291_175_153_305e3, 4.493_607_500_533_236e1, 1e-5, 1e-4, "swillens_total");
    assert_param(&fit, "Kd", None, 2.512_683_670_459_944e-3, 5.657_682_415_236_535e-5, 1e-5, 1e-4, "swillens_total");
    assert_param(&fit, "NS", None, 3.929_236_029_160_844e-2, 2.528_227_310_564_613e-3, 1e-5, 1e-4, "swillens_total");
    assert_approx_eq(fit.rss, 4.763_056_576_451_885e2, 1e-10, 1e-7, "swillens_total RSS");
    assert_approx_eq(derived_val(&fit, 0, "Kd_cpm"), 2.512_683_670_459_944e-3 * 1e6, 1e-6, 1e-5, "swillens_total Kd_cpm");

    // 20. two_sites_fit_logic50
    let x_comp = linspace(-11.0, -3.0, 21);
    let e_comp = eps_vec(x_comp.len(), 0.3);
    let y_comp: Vec<f64> = x_comp
        .iter()
        .zip(&e_comp)
        .map(|(&x, &e)| 5.0 + 95.0 * (0.35 / (1.0 + 10.0_f64.powf(x + 8.5)) + 0.65 / (1.0 + 10.0_f64.powf(x + 5.5))) + e)
        .collect();
    let fit = run_fit(
        "two_sites_fit_logic50",
        &[make_series(0, "Comp", &x_comp, &y_comp)],
        vec![
            rule_local("Bottom", 4.0),
            rule_local("Top", 98.0),
            rule_local("FractionHi", 0.4),
            rule_local("LogIC50_Hi", -8.3),
            rule_local("LogIC50_Lo", -5.7),
        ],
        false,
    );
    assert_param(&fit, "FractionHi", None, 3.498_610_233_057_352e-1, 1.783_599_518_584_730e-3, 1e-6, 1e-5, "two_sites_logic50");
    assert_param(&fit, "LogIC50_Hi", None, -8.499_476_029_523_899, 1.119_829_399_682_676e-2, 1e-6, 1e-5, "two_sites_logic50");
    assert_param(&fit, "LogIC50_Lo", None, -5.498_993_823_825_103, 6.026_882_910_725_775e-3, 1e-6, 1e-5, "two_sites_logic50");
    assert_approx_eq(fit.rss, 9.288_730_790_959_201e-1, 1e-12, 1e-7, "two_sites_logic50 RSS");

    // 21. allosteric_modulator_competition
    let x_mod = linspace(-9.0, -3.0, 16);
    let e_mod = eps_vec(x_mod.len(), 0.35);
    let y_mod: Vec<f64> = x_mod
        .iter()
        .zip(&e_mod)
        .map(|(&x, &e)| {
            let u = 10.0_f64.powf(x + 6.2);
            let k_app = 2.0 * (1.0 + u) / (1.0 + 10.0_f64.powf(-0.8) * u);
            100.0 * (1.0 + 2.0) / (1.0 + k_app) + e
        })
        .collect();
    let fit = run_fit(
        "allosteric_modulator_competition",
        &[make_series(0, "Allo", &x_mod, &y_mod)],
        vec![rule_const("RadioligandNM", 1.0), rule_const("HotKdNM", 2.0)],
        false,
    );
    assert_param(&fit, "Y0", None, 99.986_135_727_199_50, 1.175_687_451_441_320e-1, 1e-6, 1e-5, "allo_mod_comp");
    assert_param(&fit, "LogKB", None, -6.199_358_924_456_318, 4.448_824_299_209_523e-3, 1e-6, 1e-5, "allo_mod_comp");
    assert_param(&fit, "LogAlpha", None, -8.004_216_653_193_957e-1, 2.444_538_825_229_352e-3, 1e-6, 1e-5, "allo_mod_comp");
    assert_approx_eq(fit.rss, 9.158_952_160_303_985e-1, 1e-12, 1e-7, "allo_mod_comp RSS");
    assert_approx_eq(derived_val(&fit, 0, "Alpha"), 10.0_f64.powf(-8.004_216_653_193_957e-1), 1e-12, 1e-6, "allo_mod_comp Alpha");
}

#[test]
fn d1_r18_3_group5_curves_and_radiobiology() {
    // 22. two_phase_association
    let t_a = linspace(0.0, 12.0, 25);
    let e_a = eps_vec(t_a.len(), 0.25);
    let y_a: Vec<f64> =
        t_a.iter().zip(&e_a).map(|(&t, &e)| 10.0 + 90.0 * (0.4 * (1.0 - (-2.5 * t).exp()) + 0.6 * (1.0 - (-0.35 * t).exp())) + e).collect();
    let fit = run_fit(
        "two_phase_association",
        &[make_series(0, "A", &t_a, &y_a)],
        vec![rule_local("Y0", 9.0), rule_local("Plateau", 98.0), rule_local("PercentFast", 45.0), rule_local("KFast", 2.2), rule_local("KSlow", 0.3)],
        false,
    );
    assert_param(&fit, "Y0", None, 10.022_878_777_846_21, 1.948_263_571_215_341e-1, 1e-6, 1e-4, "two_phase_assoc");
    assert_param(&fit, "Plateau", None, 100.057_245_671_083_9, 1.174_163_742_216_940e-1, 1e-6, 1e-4, "two_phase_assoc");
    assert_param(&fit, "PercentFast", None, 40.373_309_194_771_47, 6.932_055_804_437_267e-1, 1e-5, 1e-4, "two_phase_assoc");
    assert_param(&fit, "KFast", None, 2.449_437_155_942_720, 7.057_712_934_262_081e-2, 1e-5, 1e-4, "two_phase_assoc");
    assert_param(&fit, "KSlow", None, 3.472_303_854_807_484e-1, 5.131_917_619_622_419e-3, 1e-5, 1e-4, "two_phase_assoc");
    assert_approx_eq(fit.rss, 7.614_920_004_915_199e-1, 1e-12, 1e-7, "two_phase_assoc RSS");
    assert_approx_eq(derived_val(&fit, 0, "TauFast"), 1.0 / 2.449_437_155_942_720, 1e-12, 1e-5, "two_phase_assoc TauFast");
    assert_approx_eq(
        derived_val(&fit, 0, "HalfTimeSlow"),
        std::f64::consts::LN_2 / 3.472_303_854_807_484e-1,
        1e-12,
        1e-5,
        "two_phase_assoc HalfTimeSlow",
    );

    // 23. two_lines_intersection (no rules needed — default_parameter_rules auto-shares XCross/YCross!)
    let x_l = linspace(0.0, 10.0, 11);
    let e_l = eps_vec(x_l.len(), 0.2);
    let y_l1: Vec<f64> = x_l.iter().zip(&e_l).map(|(&x, &e)| 11.5 + 1.8 * (x - 4.2) + e).collect();
    let y_l2: Vec<f64> = x_l.iter().zip(&e_l).map(|(&x, &e)| 11.5 - 0.9 * (x - 4.2) - e).collect();
    let fit = run_fit("two_lines_intersection", &[make_series(0, "L1", &x_l, &y_l1), make_series(1, "L2", &x_l, &y_l2)], vec![], false);
    assert_param(&fit, "XCross", None, 4.201_132_859_030_777, 2.549_080_138_026_538e-2, 1e-7, 1e-6, "two_lines");
    assert_param(&fit, "YCross", None, 11.500_509_786_528_40, 3.636_744_459_505_849e-2, 1e-7, 1e-6, "two_lines");
    assert_param(&fit, "Slope", Some(0), 1.803_861_651_773_806, 1.496_369_785_440_708e-2, 1e-7, 1e-6, "two_lines");
    assert_param(&fit, "Slope", Some(1), -9.038_616_520_078_280e-1, 1.496_369_767_179_871e-2, 1e-7, 1e-6, "two_lines");
    assert_approx_eq(fit.rss, 4.433_462_517_654_105e-1, 1e-12, 1e-8, "two_lines RSS");

    // 24. gaussian_area
    let x_g2 = linspace(-4.0, 10.0, 29);
    let e_g2 = eps_vec(x_g2.len(), 0.08);
    let sqrt_2pi = (2.0 * std::f64::consts::PI).sqrt();
    let y_g2: Vec<f64> = x_g2.iter().zip(&e_g2).map(|(&x, &e)| (25.0 / (1.8 * sqrt_2pi)) * (-0.5 * ((x - 3.2) / 1.8).powi(2)).exp() + e).collect();
    let fit = run_fit("gaussian_area", &[make_series(0, "G", &x_g2, &y_g2)], vec![], false);
    assert_param(&fit, "Area", None, 24.999_811_128_812_58, 1.312_583_196_158_744e-1, 1e-7, 1e-6, "gaussian_area");
    assert_param(&fit, "Mean", None, 3.199_996_408_056_260, 1.091_252_576_248_866e-2, 1e-7, 1e-6, "gaussian_area");
    assert_param(&fit, "SD", None, 1.799_974_400_344_441, 1.091_256_588_710_575e-2, 1e-7, 1e-6, "gaussian_area");
    assert_approx_eq(fit.rss, 9.360_374_786_982_736e-2, 1e-12, 1e-8, "gaussian_area RSS");

    // 25. lognormal_area
    let x_ln = linspace(0.5, 16.0, 24);
    let e_ln = eps_vec(x_ln.len(), 0.1);
    let y_ln: Vec<f64> = x_ln.iter().zip(&e_ln).map(|(&x, &e)| (40.0 / x) * (-0.5 * ((x / 4.5).ln() / 1.85_f64.ln()).powi(2)).exp() + e).collect();
    let fit = run_fit("lognormal_area", &[make_series(0, "LN", &x_ln, &y_ln)], vec![], false);
    assert_param(&fit, "A", None, 40.013_379_257_991_65, 1.451_978_561_659_727e-1, 1e-6, 1e-5, "lognormal_area");
    assert_param(&fit, "GeoMean", None, 4.498_089_687_421_308, 1.365_921_724_955_707e-2, 1e-6, 1e-5, "lognormal_area");
    assert_param(&fit, "GeoSD", None, 1.849_050_901_275_846, 4.788_872_904_460_787e-3, 1e-6, 1e-5, "lognormal_area");
    assert_approx_eq(fit.rss, 1.151_714_046_806_732e-1, 1e-12, 1e-7, "lognormal_area RSS");
    let expected_area = 40.013_379_257_991_65 * sqrt_2pi * 1.849_050_901_275_846_f64.ln();
    assert_approx_eq(derived_val(&fit, 0, "Area"), expected_area, 1e-12, 1e-6, "lognormal_area derived Area");

    // 26. standard_sine_wave
    let x_sw = linspace(0.0, 12.0, 25);
    let e_sw = eps_vec(x_sw.len(), 0.12);
    let y_sw: Vec<f64> = x_sw.iter().zip(&e_sw).map(|(&x, &e)| 4.5 * (2.0 * std::f64::consts::PI * x / 5.2 + 0.6).sin() + e).collect();
    let fit = run_fit(
        "standard_sine_wave",
        &[make_series(0, "SW", &x_sw, &y_sw)],
        vec![rule_local("Amplitude", 4.2), rule_local("Wavelength", 5.0), rule_local("PhaseShift", 0.5)],
        false,
    );
    assert_param(&fit, "Amplitude", None, 4.499_399_406_868_956, 2.517_865_157_990_953e-2, 1e-7, 1e-6, "std_sine");
    assert_param(&fit, "Wavelength", None, 5.202_063_112_354_752, 7.187_584_208_528_123e-3, 1e-7, 1e-6, "std_sine");
    assert_param(&fit, "PhaseShift", None, 6.013_964_098_791_527e-1, 1.156_175_063_488_139e-2, 1e-7, 1e-6, "std_sine");
    assert_approx_eq(fit.rss, 1.792_758_602_761_775e-1, 1e-12, 1e-8, "std_sine RSS");
    assert_approx_eq(derived_val(&fit, 0, "Frequency"), 1.0 / 5.202_063_112_354_752, 1e-12, 1e-7, "std_sine Frequency");

    // 27. log_exponential_growth
    let x_eg = linspace(0.0, 10.0, 15);
    let e_eg = eps_vec(x_eg.len(), 0.08);
    let y_eg: Vec<f64> = x_eg.iter().zip(&e_eg).map(|(&x, &e)| 1.4 + 0.32 * x + e).collect();
    let fit = run_fit("log_exponential_growth", &[make_series(0, "EG", &x_eg, &y_eg)], vec![], false);
    assert_param(&fit, "LnY0", None, 1.398_569_480_328_489, 2.979_430_821_250_200e-2, 1e-7, 1e-6, "log_exp_growth");
    assert_param(&fit, "k", None, 3.199_966_017_334_808e-1, 5.070_770_442_596_213e-3, 1e-7, 1e-6, "log_exp_growth");
    assert_approx_eq(fit.rss, 4.775_218_106_565_660e-2, 1e-12, 1e-8, "log_exp_growth RSS");
    assert_approx_eq(derived_val(&fit, 0, "Y0"), 1.398_569_480_328_489_f64.exp(), 1e-12, 1e-7, "log_exp_growth Y0");
    assert_approx_eq(derived_val(&fit, 0, "DoublingTime"), std::f64::consts::LN_2 / 3.199_966_017_334_808e-1, 1e-12, 1e-7, "log_exp_growth DT");

    // 28–32. 5 Linear-Quadratic variants
    let d_rad = linspace(0.0, 8.0, 17);
    let s_true: Vec<f64> = d_rad.iter().map(|&d| (-(0.25 * d + 0.04 * d * d)).exp()).collect();

    let e04 = eps_vec(d_rad.len(), 0.4);
    let y_ps: Vec<f64> = s_true.iter().zip(&e04).map(|(&s, &e)| 100.0 * s + e).collect();
    let fit = run_fit("linear_quadratic_percent_surviving", &[make_series(0, "PS", &d_rad, &y_ps)], vec![], false);
    assert_param(&fit, "Alpha", None, 2.502_737_974_801_110e-1, 2.573_528_469_035_298e-3, 1e-7, 1e-6, "lq_pct_surv");
    assert_param(&fit, "Beta", None, 3.990_196_152_215_829e-2, 8.204_432_584_106_746e-4, 1e-7, 1e-6, "lq_pct_surv");
    assert_approx_eq(fit.rss, 1.318_694_239_299_135, 1e-12, 1e-8, "lq_pct_surv RSS");

    let e12 = eps_vec(d_rad.len(), 1.2);
    let y_ns: Vec<f64> = s_true.iter().zip(&e12).map(|(&s, &e)| 250.0 * s + e).collect();
    let fit = run_fit("linear_quadratic_number_surviving", &[make_series(0, "NS", &d_rad, &y_ns)], vec![], false);
    assert_param(&fit, "Y0", None, 249.923_939_656_282_4, 7.940_674_310_019_946e-1, 1e-7, 1e-6, "lq_num_surv");
    assert_param(&fit, "Alpha", None, 2.500_235_703_254_511e-1, 4.510_060_700_761_018e-3, 1e-7, 1e-6, "lq_num_surv");
    assert_param(&fit, "Beta", None, 3.993_933_352_590_014e-2, 1.179_420_777_133_129e-3, 1e-7, 1e-6, "lq_num_surv");
    assert_approx_eq(fit.rss, 11.860_475_205_690_29, 1e-12, 1e-8, "lq_num_surv RSS");

    let e0004 = eps_vec(d_rad.len(), 0.004);
    let y_fd: Vec<f64> = s_true.iter().zip(&e0004).map(|(&s, &e)| (1.0 - s) + e).collect();
    let fit = run_fit("linear_quadratic_fraction_dead", &[make_series(0, "FD", &d_rad, &y_fd)], vec![], false);
    assert_param(&fit, "Alpha", None, 2.497_271_295_931_443e-1, 2.575_208_078_293_857e-3, 1e-7, 1e-6, "lq_frac_dead");
    assert_param(&fit, "Beta", None, 4.009_771_866_598_412e-2, 8.217_898_708_169_711e-4, 1e-7, 1e-6, "lq_frac_dead");
    assert_approx_eq(fit.rss, 1.318_699_372_390_538e-4, 1e-12, 1e-8, "lq_frac_dead RSS");

    let y_pd: Vec<f64> = s_true.iter().zip(&e04).map(|(&s, &e)| 100.0 * (1.0 - s) + e).collect();
    let fit = run_fit("linear_quadratic_percent_dead", &[make_series(0, "PD", &d_rad, &y_pd)], vec![], false);
    assert_param(&fit, "Alpha", None, 2.497_271_296_116_387e-1, 2.575_208_001_757_539e-3, 1e-7, 1e-6, "lq_pct_dead");
    assert_param(&fit, "Beta", None, 4.009_771_866_567_887e-2, 8.217_898_550_183_944e-4, 1e-7, 1e-6, "lq_pct_dead");
    assert_approx_eq(fit.rss, 1.318_699_372_390_552, 1e-12, 1e-8, "lq_pct_dead RSS");

    let y_nd: Vec<f64> = s_true.iter().zip(&e12).map(|(&s, &e)| 250.0 * (1.0 - s) + e).collect();
    let fit = run_fit("linear_quadratic_number_dead", &[make_series(0, "ND", &d_rad, &y_nd)], vec![], false);
    assert_param(&fit, "Y0", None, 250.261_988_014_177_2, 6.989_268_480_314_191e-1, 1e-7, 1e-6, "lq_num_dead");
    assert_param(&fit, "Alpha", None, 2.502_105_307_724_532e-1, 3.474_698_195_216_891e-3, 1e-7, 1e-6, "lq_num_dead");
    assert_param(&fit, "Beta", None, 3.962_427_541_535_845e-2, 1.652_151_768_431_412e-3, 1e-7, 1e-6, "lq_num_dead");
    assert_approx_eq(fit.rss, 11.749_779_654_311_22, 1e-12, 1e-8, "lq_num_dead RSS");

    // 33. boltzmann_sigmoid
    let x_v = linspace(-80.0, 40.0, 25);
    let e_v = eps_vec(x_v.len(), 0.25);
    let y_v: Vec<f64> = x_v.iter().zip(&e_v).map(|(&x, &e)| 2.0 + 46.0 / (1.0 + ((-22.5 - x) / 8.4).exp()) + e).collect();
    let fit = run_fit("boltzmann_sigmoid", &[make_series(0, "V", &x_v, &y_v)], vec![], false);
    assert_param(&fit, "Bottom", None, 1.990_204_764_220_386, 8.054_063_323_569_023e-2, 1e-7, 1e-6, "boltzmann");
    assert_param(&fit, "Top", None, 48.020_109_938_475_06, 7.462_177_578_520_970e-2, 1e-7, 1e-6, "boltzmann");
    assert_param(&fit, "V50", None, -22.494_320_668_609_93, 8.619_693_768_392_962e-2, 1e-7, 1e-6, "boltzmann");
    assert_param(&fit, "Slope", None, 8.412_280_357_822_368, 7.698_389_384_453_332e-2, 1e-7, 1e-6, "boltzmann");
    assert_approx_eq(fit.rss, 7.804_109_135_751_978e-1, 1e-12, 1e-8, "boltzmann RSS");

    // 34. power_series_two_term
    let x_p = linspace(0.5, 6.0, 20);
    let e_p = eps_vec(x_p.len(), 0.08);
    let y_p: Vec<f64> = x_p.iter().zip(&e_p).map(|(&x, &e)| 2.4 * x.powf(0.7) + 0.35 * x.powf(2.2) + e).collect();
    let fit = run_fit(
        "power_series_two_term",
        &[make_series(0, "P", &x_p, &y_p)],
        vec![rule_local("A", 2.2), rule_local("B", 0.65), rule_local("C", 0.3), rule_local("D", 2.1)],
        false,
    );
    assert_param(&fit, "A", None, 2.417_389_906_999_267, 6.060_556_214_041_116e-2, 1e-5, 1e-4, "power_two_term");
    assert_param(&fit, "B", None, 7.146_916_314_630_117e-1, 4.994_507_426_237_418e-2, 1e-5, 1e-4, "power_two_term");
    assert_param(&fit, "C", None, 3.300_571_758_925_960e-1, 6.093_004_348_269_092e-2, 1e-5, 1e-4, "power_two_term");
    assert_param(&fit, "D", None, 2.224_440_087_679_484, 7.414_762_724_647_737e-2, 1e-5, 1e-4, "power_two_term");
    assert_approx_eq(fit.rss, 5.987_795_085_953_559e-2, 1e-12, 1e-7, "power_two_term RSS");
}

#[test]
fn d1_r18_3_analytical_gradients_and_domain_guards() {
    // Check analytical grad() against central finite differences on all new models implementing grad().
    let cases: &[(&str, f64, &[f64])] = &[
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
    for &(id, x, params) in cases {
        let m = get_builtin_model(id).unwrap();
        let mut g = vec![0.0; params.len()];
        assert!(m.grad(&[x], params, 0, &mut g), "{id} grad() returned false");
        for k in 0..params.len() {
            let h = 1e-6 * params[k].abs().max(1.0);
            let mut p_hi = params.to_vec();
            let mut p_lo = params.to_vec();
            p_hi[k] += h;
            p_lo[k] -= h;
            let fd = (m.eval(&[x], &p_hi, 0).unwrap() - m.eval(&[x], &p_lo, 0).unwrap()) / (2.0 * h);
            assert_approx_eq(g[k], fd, 1e-7, 1e-6, &format!("{id} grad[{k}] vs FD"));
        }
    }

    // Domain & edge-case guards
    let pst = get_builtin_model("power_series_two_term").unwrap();
    assert!(pst.eval(&[0.0], &[1.0, -0.5, 1.0, 1.0], 0).is_none()); // X=0 with negative exponent
    assert!(pst.eval(&[-2.0], &[1.0, 0.5, 1.0, 2.0], 0).is_none()); // X<0 with fractional exponent
    assert_approx_eq(pst.eval(&[-2.0], &[1.0, 3.0, 2.0, 2.0], 0).unwrap(), -8.0 + 8.0, 1e-12, 1e-12, "X<0 integer exponents");
    assert!(pst.eval(&[f64::NAN], &[1.0, 1.0, 1.0, 2.0], 0).is_none());

    let ga = get_builtin_model("gaussian_area").unwrap();
    assert!(ga.eval(&[1.0], &[10.0, 0.0, 0.0], 0).is_none());
    assert!(ga.eval(&[1.0], &[10.0, 0.0, -1.0], 0).is_none());

    let lna = get_builtin_model("lognormal_area").unwrap();
    assert!(lna.eval(&[0.0], &[10.0, 2.0, 1.5], 0).is_none());
    assert!(lna.eval(&[1.0], &[10.0, 2.0, 1.0], 0).is_none());

    let ssw = get_builtin_model("standard_sine_wave").unwrap();
    assert!(ssw.eval(&[1.0], &[1.0, 0.0, 0.0], 0).is_none());

    let boltz = get_builtin_model("boltzmann_sigmoid").unwrap();
    assert!(boltz.eval(&[0.0], &[0.0, 1.0, 0.0, 0.0], 0).is_none());

    let (open_lo, open_hi) = kb_ci_from_pkb_ci(CiBound::Open, CiBound::Finite(8.0));
    assert_eq!(open_hi, CiBound::Open);
    assert!(matches!(open_lo, CiBound::Finite(v) if (v - 1e-8).abs() < 1e-18));
}
