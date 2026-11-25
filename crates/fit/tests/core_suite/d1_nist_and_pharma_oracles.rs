//! D1 public-oracle verification suite for `curvebook-fit`.
//!
//! Oracles and literature references: NIST StRD Nonlinear Least Squares (Lower/Average/Higher);
//! Motulsky & Christopoulos (2004); Arunlakshana & Schild (1959); Cheng & Prusoff (1973);
//! Cornish-Bowden (1974); Verhulst (1838); Gompertz (1825); Venzon & Moolgavkar (1988);
//! Deming (1943); Motulsky & Brown (2006).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CiBound, ConstantValue, DataSheet, FitMethod, HookSource, InfoSheet, LinearRegressionSpec,
    ModelComparisonSpec, NonlinearFitSpec, ParameterRule, SheetId, SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
};
use curvebook_fit::{
    DataPoint, DatasetSeries, DemingOptions, FitError, FitStatus, HookContext, ModelComparisonOutcome, NonlinearFitResult, ParameterNote,
    ParametricModel, SolverConfig, compare_models, compare_models_aicc, compare_models_f_test, deming_regression, execute_linear_analysis,
    execute_nonlinear_analysis, fit_nonlinear, get_builtin_model, rout_detect_outliers, simple_linear_regression,
};
use curvebook_testkit::nist_strd::{NistDifficulty, default_nist_strd_dir, load_corpus_nonlinear_all};
use curvebook_testkit::{assert_approx_eq, assert_lre_at_least, log_relative_error};

use crate::common::{
    assert_converged, assert_params_approx, cfg_rules, dataset_param_val, dsl_compile, find_derived, find_param_ds, fit_default, fit_with, nl_fast,
    param_ds_val, rule_const, rule_hook, rule_local, rule_local_bounds, rule_shared, rule_shared_bounds, series_from_pts, series_xy, xy_sheet,
};

fn assert_nist_converged(status: FitStatus, label: &str) {
    assert!(matches!(status, FitStatus::Converged | FitStatus::Ambiguous), "{label}: expected Converged or Ambiguous, got {status:?}");
}

fn eval_model_at(model: &dyn ParametricModel, res: &NonlinearFitResult, ds_idx: usize, x: f64) -> f64 {
    model.eval(&[x], &res.datasets[ds_idx].params, ds_idx).unwrap_or_else(|| panic!("model eval returned None at x={x}"))
}

fn assert_normal_equations_small(model: &dyn ParametricModel, res: &NonlinearFitResult, ds: &DatasetSeries, free_names: &[&str], tol: f64) {
    let p_names = model.param_names();
    let full = &res.datasets[0].params;
    for &fname in free_names {
        let p_idx = p_names.iter().position(|p| p.eq_ignore_ascii_case(fname)).unwrap();
        let h = 1e-7 * (1.0 + full[p_idx].abs());
        let (mut plus, mut minus) = (full.clone(), full.clone());
        plus[p_idx] += h;
        minus[p_idx] -= h;
        let grad: f64 = ds
            .points
            .iter()
            .map(|pt| {
                let x = [pt.x[0]];
                let r = pt.y - model.eval(&x, full, 0).unwrap();
                r * (model.eval(&x, &plus, 0).unwrap() - model.eval(&x, &minus, 0).unwrap()) / (2.0 * h)
            })
            .sum();
        assert!(grad.abs() <= tol, "normal equation |J^T r| for `{fname}` = {grad:.3e} > tol {tol:.3e}");
    }
}

// ============================================================================
// SECTION A: NIST StRD Nonlinear Least Squares Oracles
// ============================================================================

fn verify_nist_tier(difficulty: NistDifficulty, expected_count: usize) {
    let dir = default_nist_strd_dir();
    if !dir.exists() {
        assert_ne!(std::env::var("CURVEBOOK_REQUIRE_CORPUS").ok().as_deref(), Some("1"), "missing NIST StRD corpus at {}", dir.display());
        return;
    }

    let tier_ds: Vec<_> = load_corpus_nonlinear_all(&dir).unwrap().into_iter().filter(|d| d.difficulty == difficulty).collect();
    assert_eq!(tier_ds.len(), expected_count, "unexpected NIST dataset count for {difficulty:?}");

    for ds in &tier_ds {
        let model = get_builtin_model(&format!("nist_{}", ds.name.to_ascii_lowercase())).unwrap();
        let is_nelson = ds.name.eq_ignore_ascii_case("Nelson");
        let n_obs = ds.y.len();
        let points: Vec<DataPoint> = (0..n_obs)
            .map(|row| DataPoint {
                x: ds.predictors[row].clone(),
                y: if is_nelson { ds.y[row].ln() } else { ds.y[row] },
                row,
                ..DataPoint::xy(0.0, 0.0)
            })
            .collect();
        let input = series_from_pts(0, &ds.name, points);

        for (start_label, starts) in [("start1", &ds.start_1), ("start2", &ds.start_2)] {
            let rules: Vec<ParameterRule> =
                model.param_names().iter().zip(starts.iter()).map(|(n, &v)| rule_local_bounds(n, Some(v), None, None)).collect();
            let cfg = SolverConfig { parameter_rules: rules, compute_profile_ci: false, ..SolverConfig::default() };
            let res = fit_with(model.as_ref(), std::slice::from_ref(&input), &cfg);

            assert_nist_converged(res.status, &format!("{} ({start_label})", ds.name));
            let expected_df = n_obs - model.param_names().len();
            assert_eq!(res.df, expected_df, "{} df mismatch", ds.name);
            if ds.name != "Rat43" {
                assert_eq!(ds.certified_df, expected_df);
            }

            let beta_floor = match difficulty {
                NistDifficulty::Lower => 7.0,
                NistDifficulty::Average if matches!(ds.name.as_str(), "Lanczos1" | "ENSO" | "Hahn1") => 6.5,
                NistDifficulty::Average => 7.0,
                NistDifficulty::Higher => 6.5,
            };
            let se_floor = if ds.name == "Lanczos1" {
                2.5
            } else if difficulty == NistDifficulty::Lower {
                5.5
            } else {
                5.0
            };
            for (idx, ((&cb, &cse), p)) in ds.certified_betas.iter().zip(&ds.certified_se_betas).zip(&res.parameters).enumerate() {
                let tag = format!("{} ({start_label}) b{}", ds.name, idx + 1);
                assert_lre_at_least(p.estimate, cb, beta_floor, &format!("{tag} beta"));
                assert_lre_at_least(p.se, cse, se_floor, &format!("{tag} SE"));
            }

            if ds.name == "Lanczos1" {
                assert!(res.rss < 1e-22 && res.rsd < 1e-11, "Lanczos1 rss={}, rsd={}", res.rss, res.rsd);
            } else {
                let rss_floor = match difficulty {
                    NistDifficulty::Lower => 8.0,
                    NistDifficulty::Average => 7.5,
                    NistDifficulty::Higher => 7.0,
                };
                assert!(log_relative_error(res.rss, ds.certified_rss) >= rss_floor, "{} ({start_label}) RSS LRE low", ds.name);
                assert!(log_relative_error(res.rsd, ds.certified_residual_sd) >= 6.5, "{} ({start_label}) RSD LRE low", ds.name);
            }
        }
    }
}

#[test]
fn nist_strd_lower_difficulty_8_datasets() {
    verify_nist_tier(NistDifficulty::Lower, 8);
}

#[test]
fn nist_strd_average_difficulty_11_datasets() {
    verify_nist_tier(NistDifficulty::Average, 11);
}

#[test]
fn nist_strd_higher_difficulty_8_datasets() {
    verify_nist_tier(NistDifficulty::Higher, 8);
}

#[test]
fn nist_strd_inline_certified_danwood_and_boxbod_oracles() {
    struct InlineNist<'a> {
        id: &'a str,
        label: &'a str,
        xs: &'a [f64],
        ys: &'a [f64],
        starts: Option<[f64; 2]>,
        betas: [f64; 2],
        ses: [f64; 2],
        rss: f64,
        beta_lre: f64,
        rss_lre: f64,
    }
    let cases = [
        InlineNist {
            id: "nist_danwood",
            label: "DanWood",
            xs: &[1.309, 1.471, 1.490, 1.565, 1.611, 1.680],
            ys: &[2.138, 3.421, 3.597, 4.340, 4.882, 5.660],
            starts: None,
            betas: [0.76886226176, 3.8604055871],
            ses: [0.018281973860, 0.051726610913],
            rss: 0.0043173084083,
            beta_lre: 9.0,
            rss_lre: 9.0,
        },
        InlineNist {
            id: "nist_boxbod",
            label: "BoxBOD",
            xs: &[1.0, 2.0, 3.0, 5.0, 7.0, 10.0],
            ys: &[109.0, 149.0, 149.0, 191.0, 213.0, 224.0],
            starts: Some([100.0, 0.75]),
            betas: [213.80940889, 0.54723748542],
            ses: [12.354515176, 0.10455993237],
            rss: 1168.0088766,
            beta_lre: 8.0,
            rss_lre: 8.0,
        },
    ];
    for c in cases {
        let model = get_builtin_model(c.id).unwrap();
        let cfg = c
            .starts
            .map(|[b1, b2]| cfg_rules(vec![rule_local_bounds("b1", Some(b1), None, None), rule_local_bounds("b2", Some(b2), None, None)]))
            .unwrap_or_default();
        let res = fit_with(model.as_ref(), &[series_xy(0, c.label, c.xs, c.ys)], &cfg);
        assert_converged(&res, c.label);
        for i in 0..2 {
            assert_lre_at_least(res.parameters[i].estimate, c.betas[i], c.beta_lre, &format!("{} b{}", c.label, i + 1));
            assert_lre_at_least(res.parameters[i].se, c.ses[i], 7.0, &format!("{} SE{}", c.label, i + 1));
        }
        assert_lre_at_least(res.rss, c.rss, c.rss_lre, &format!("{} RSS", c.label));
    }
}

// ============================================================================
// SECTION B: Dose-Response Pharmacology (4PL, 5PL, Gaddum-Schild, Cheng-Prusoff)
// ============================================================================

#[test]
fn pharma_4pl_agonist_and_inhibitor_oracles() {
    let log_c: Vec<f64> = (-20..=-7).map(|i| i as f64 * 0.5).collect();
    let (true_b, true_t, true_ec, true_h) = (12.5_f64, 98.0, -6.75, 1.35);
    let y_ag: Vec<f64> = log_c.iter().map(|&x| true_b + (true_t - true_b) / (1.0 + 10_f64.powf((true_ec - x) * true_h))).collect();
    let ag_model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let ag_res = fit_default(ag_model.as_ref(), &[series_xy(0, "Agonist", &log_c, &y_ag)]);
    assert_converged(&ag_res, "4PL agonist");
    assert_params_approx(
        &ag_res,
        0,
        &[("Bottom", true_b, 1e-6, 1e-6), ("Top", true_t, 1e-6, 1e-6), ("LogEC50", true_ec, 1e-6, 1e-6), ("HillSlope", true_h, 1e-6, 1e-6)],
        "4PL ag",
    );
    assert_approx_eq(find_derived(&ag_res, 0, "EC50"), 10_f64.powf(true_ec), 1e-12, 1e-6, "4PL ag derived EC50");
    assert_approx_eq(find_derived(&ag_res, 0, "Span"), true_t - true_b, 1e-6, 1e-6, "4PL ag derived Span");
    assert_approx_eq(eval_model_at(ag_model.as_ref(), &ag_res, 0, true_ec), 0.5 * (true_b + true_t), 1e-6, 1e-6, "4PL midpoint");

    let log_i: Vec<f64> = (-20..=-6).map(|i| i as f64 * 0.5).collect();
    let (inh_b, inh_t, inh_ic, inh_h) = (8.0_f64, 104.0, -7.2, 1.15);
    let noise = [0.4, -0.6, 0.5, -0.3, 0.8, -0.7, 0.2, -0.5, 0.6, -0.4, 0.3, -0.2, 0.5, -0.4, 0.2];
    let y_inh: Vec<f64> = log_i.iter().zip(noise).map(|(&x, e)| inh_b + (inh_t - inh_b) / (1.0 + 10_f64.powf((x - inh_ic) * inh_h)) + e).collect();
    let inh_ds = series_xy(0, "Inhibitor", &log_i, &y_inh);
    let inh_model = get_builtin_model("log_inhibitor_vs_response_4pl").unwrap();
    let inh_res = fit_default(inh_model.as_ref(), std::slice::from_ref(&inh_ds));
    assert_converged(&inh_res, "4PL inhibitor");
    assert_normal_equations_small(inh_model.as_ref(), &inh_res, &inh_ds, &["Bottom", "Top", "LogIC50", "HillSlope"], 1e-5);
    for (name, truth) in [("Bottom", inh_b), ("Top", inh_t), ("LogIC50", inh_ic), ("HillSlope", inh_h)] {
        let p = find_param_ds(&inh_res, 0, name);
        assert!((p.estimate - truth).abs() <= 2.5 * p.se, "noisy 4PL `{name}` est={} se={}", p.estimate, p.se);
    }
}

#[test]
fn pharma_5pl_asymmetric_richards_oracle() {
    let log_c: Vec<f64> = (-22..=-8).map(|i| i as f64 * 0.5).collect();
    let model = get_builtin_model("log_agonist_vs_response_5pl").unwrap();
    for (true_s, tag) in [(2.4_f64, "S=2.4"), (0.45_f64, "S=0.45")] {
        let (true_b, true_t, true_xb, true_h) = (5.0_f64, 105.0, -7.4, 1.25);
        let y: Vec<f64> = log_c.iter().map(|&x| true_b + (true_t - true_b) / (1.0 + 10_f64.powf((true_xb - x) * true_h)).powf(true_s)).collect();
        let cfg = SolverConfig {
            parameter_rules: vec![
                rule_local_bounds("Bottom", Some(5.0), None, None),
                rule_local_bounds("Top", Some(105.0), None, None),
                rule_local_bounds("LogXb", Some(-7.2), None, None),
                rule_local_bounds("HillSlope", Some(1.0), Some(0.05), Some(10.0)),
                rule_local_bounds("S", Some(1.0), Some(1e-3), Some(20.0)),
            ],
            max_iters: 2000,
            ..SolverConfig::default()
        };
        let res = fit_with(model.as_ref(), &[series_xy(0, tag, &log_c, &y)], &cfg);
        assert_converged(&res, &format!("5PL {tag}"));
        assert_params_approx(
            &res,
            0,
            &[
                ("Bottom", true_b, 1e-5, 1e-6),
                ("Top", true_t, 1e-5, 1e-6),
                ("LogXb", true_xb, 1e-5, 1e-6),
                ("HillSlope", true_h, 1e-5, 1e-6),
                ("S", true_s, 1e-5, 1e-6),
            ],
            tag,
        );

        let exp_ec = true_xb - (2_f64.powf(1.0 / true_s) - 1.0).log10() / true_h;
        assert_approx_eq(find_derived(&res, 0, "LogEC50"), exp_ec, 1e-6, 1e-6, &format!("{tag} LogEC50"));
        assert_approx_eq(find_derived(&res, 0, "EC50"), 10_f64.powf(exp_ec), 1e-11, 1e-5, &format!("{tag} EC50"));
        assert_approx_eq(eval_model_at(model.as_ref(), &res, 0, exp_ec), 0.5 * (true_b + true_t), 1e-4, 1e-5, &format!("{tag} Y(EC50)"));
        assert!((eval_model_at(model.as_ref(), &res, 0, true_xb) - 0.5 * (true_b + true_t)).abs() > 5.0);
    }
}

#[test]
fn pharma_gaddum_schild_competitive_antagonism_pa2_builtin_and_global() {
    let (bottom, top, log_ec50, hill, pa2, schild_slope) = (10.0_f64, 100.0, -8.0, 1.1, 7.4, 1.0);
    let log_ag: Vec<f64> = (-20..=-6).map(|i| i as f64 * 0.5).collect();
    let gen_curve = |antag_m: f64| -> Vec<f64> {
        let log_dr = if antag_m > 0.0 { (1.0 + (antag_m * 10_f64.powf(pa2)).powf(schild_slope)).log10() } else { 0.0 };
        log_ag.iter().map(|&x| bottom + (top - bottom) / (1.0 + 10_f64.powf((log_ec50 + log_dr - x) * hill))).collect()
    };

    let shift_model = get_builtin_model("ec50_shift_gaddum_schild").unwrap();
    let cfg_s = cfg_rules(vec![
        rule_const("LogEC50", -8.0),
        rule_const("SchildSlope", 1.0),
        rule_const("AntagConc", 1e-6),
        rule_local_bounds("pA2", Some(7.0), Some(3.0), Some(12.0)),
    ]);
    let single_res = fit_with(shift_model.as_ref(), &[series_xy(0, "[B]=1uM", &log_ag, &gen_curve(1e-6))], &cfg_s);
    assert_converged(&single_res, "single Gaddum-Schild");
    let pa2_est = param_ds_val(&single_res, 0, "pA2");
    assert_approx_eq(pa2_est, pa2, 1e-6, 1e-6, "builtin pA2");
    assert_approx_eq(10_f64.powf(-pa2_est), 10_f64.powf(-pa2), 1e-13, 1e-6, "builtin Kb_M");

    let dsl = dsl_compile(
        "<A> Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))\n\
         <B> Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + log10(1 + (1e-7 * 10^pA2)^SchildSlope) - X) * HillSlope))\n\
         <C> Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + log10(1 + (1e-6 * 10^pA2)^SchildSlope) - X) * HillSlope))\n\
         <D> Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + log10(1 + (1e-5 * 10^pA2)^SchildSlope) - X) * HillSlope))",
    );
    let datasets: Vec<_> = [("Vehicle", 0.0), ("100 nM", 1e-7), ("1 uM", 1e-6), ("10 uM", 1e-5)]
        .into_iter()
        .enumerate()
        .map(|(i, (lbl, c))| series_xy(i, lbl, &log_ag, &gen_curve(c)))
        .collect();
    let global_rules: Vec<ParameterRule> = dsl
        .param_names()
        .iter()
        .map(|n| match n.as_str() {
            "Bottom" => rule_shared_bounds(n, Some(10.0), None, None),
            "Top" => rule_shared_bounds(n, Some(100.0), None, None),
            "LogEC50" => rule_shared_bounds(n, Some(-8.0), None, None),
            "HillSlope" | "SchildSlope" => rule_shared_bounds(n, Some(1.0), Some(0.1), Some(5.0)),
            "pA2" => rule_shared_bounds(n, Some(7.0), None, None),
            other => panic!("unexpected param {other}"),
        })
        .collect();
    let global_res = fit_with(&dsl, &datasets, &SolverConfig { parameter_rules: global_rules, max_iters: 2000, ..SolverConfig::default() });
    assert_converged(&global_res, "global Gaddum-Schild");
    assert_eq!(global_res.k_free, 6);
    assert_params_approx(
        &global_res,
        0,
        &[("pA2", pa2, 1e-6, 1e-6), ("SchildSlope", schild_slope, 1e-6, 1e-6), ("LogEC50", log_ec50, 1e-6, 1e-6), ("HillSlope", hill, 1e-6, 1e-6)],
        "global Schild",
    );
}

#[test]
fn pharma_cheng_prusoff_one_site_and_two_sites_ki_oracle() {
    let (hot_conc, hot_kd, true_ki) = (2.4_f64, 0.8_f64, 5e-9_f64);
    let (true_log_ki, expected_ic50) = (true_ki.log10(), true_ki * (1.0 + hot_conc / hot_kd));
    let shift = (1.0 + hot_conc / hot_kd).log10();
    let log_comp: Vec<f64> = (-22..=-8).map(|i| i as f64 * 0.5).collect();
    let (true_top, true_bot) = (1200.0_f64, 150.0);
    let y1: Vec<f64> = log_comp.iter().map(|&x| true_bot + (true_top - true_bot) / (1.0 + 10_f64.powf(x - (true_log_ki + shift)))).collect();

    let mut info = InfoSheet::new_preseeded("info-1", "Assay Conditions").unwrap();
    info.set_constant("HotConc_nM", ConstantValue::Number(hot_conc), None).unwrap();
    info.set_constant("HotKd_nM", ConstantValue::Number(hot_kd), None).unwrap();
    let infos = [info];
    let hooks = HookContext { info_sheets: &infos, ..HookContext::default() };
    let hook_rules = vec![
        rule_hook("HotConc_nM", HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("info-1")), constant_name: "HotConc_nM".to_owned() }),
        rule_hook("HotKd_nM", HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("info-1")), constant_name: "HotKd_nM".to_owned() }),
    ];

    let m1 = get_builtin_model("one_site_fit_ki").unwrap();
    let res1 = fit_nonlinear(m1.as_ref(), &[series_xy(0, "Competition", &log_comp, &y1)], &hooks, &cfg_rules(hook_rules.clone())).unwrap();
    assert_converged(&res1, "one_site_fit_ki");
    assert_params_approx(&res1, 0, &[("Top", true_top, 1e-5, 1e-6), ("Bottom", true_bot, 1e-5, 1e-6), ("LogKi", true_log_ki, 1e-6, 1e-6)], "Ki");
    let ki_est = 10_f64.powf(param_ds_val(&res1, 0, "LogKi"));
    assert_approx_eq(ki_est, true_ki, 1e-14, 1e-6, "Ki == 10^LogKi");
    assert_approx_eq(ki_est * (1.0 + hot_conc / hot_kd), expected_ic50, 1e-14, 1e-6, "Cheng-Prusoff IC50");

    let (log_ki1, log_ki2, frac_hi) = (5e-10_f64.log10(), 2e-7_f64.log10(), 0.40_f64);
    let log_wide: Vec<f64> = (-24..=-6).map(|i| i as f64 * 0.5).collect();
    let y2: Vec<f64> = log_wide
        .iter()
        .map(|&x| {
            true_bot
                + (true_top - true_bot)
                    * (frac_hi / (1.0 + 10_f64.powf(x - (log_ki1 + shift))) + (1.0 - frac_hi) / (1.0 + 10_f64.powf(x - (log_ki2 + shift))))
        })
        .collect();
    let mut rules2 = hook_rules;
    rules2.extend([
        rule_local_bounds("FractionHi", Some(0.5), Some(0.0), Some(1.0)),
        rule_local_bounds("LogKi_1", Some(-9.5), None, None),
        rule_local_bounds("LogKi_2", Some(-6.5), None, None),
    ]);
    let m2 = get_builtin_model("two_sites_fit_ki").unwrap();
    let res2 = fit_nonlinear(
        m2.as_ref(),
        &[series_xy(0, "TwoSites", &log_wide, &y2)],
        &hooks,
        &SolverConfig { parameter_rules: rules2, max_iters: 2000, ..SolverConfig::default() },
    )
    .unwrap();
    assert_converged(&res2, "two_sites_fit_ki");
    assert_params_approx(
        &res2,
        0,
        &[("FractionHi", frac_hi, 1e-5, 1e-5), ("LogKi_1", log_ki1, 1e-5, 1e-5), ("LogKi_2", log_ki2, 1e-5, 1e-5)],
        "TwoSites",
    );
}

// ============================================================================
// SECTION C: Enzyme Kinetics (Michaelis-Menten, Allosteric, Inhibition Modes)
// ============================================================================

#[test]
fn enzyme_michaelis_menten_and_allosteric_sigmoidal_oracles() {
    let s = [0.25, 0.5, 1.0, 2.25, 4.5, 9.0, 18.0, 36.0, 72.0];
    let (vmax, km) = (120.0_f64, 4.5);
    let v_mm: Vec<f64> = s.iter().map(|&x| vmax * x / (km + x)).collect();
    let mm = get_builtin_model("michaelis_menten").unwrap();
    let mm_res = fit_default(mm.as_ref(), &[series_xy(0, "MM", &s, &v_mm)]);
    assert_converged(&mm_res, "Michaelis-Menten");
    assert_params_approx(&mm_res, 0, &[("Vmax", vmax, 1e-7, 1e-7), ("Km", km, 1e-7, 1e-7)], "MM");
    assert_approx_eq(eval_model_at(mm.as_ref(), &mm_res, 0, km), 0.5 * vmax, 1e-7, 1e-7, "v(Km)");

    let (allo_vmax, khalf, h) = (85.0_f64, 12.0_f64, 2.4_f64);
    let v_allo: Vec<f64> = s.iter().map(|&x| allo_vmax * x.powf(h) / (khalf.powf(h) + x.powf(h))).collect();
    let allo = get_builtin_model("allosteric_sigmoidal_kinetics").unwrap();
    let allo_res = fit_default(allo.as_ref(), &[series_xy(0, "Allosteric", &s, &v_allo)]);
    assert_converged(&allo_res, "Allosteric sigmoidal");
    assert_params_approx(&allo_res, 0, &[("Vmax", allo_vmax, 1e-6, 1e-6), ("Khalf", khalf, 1e-6, 1e-6), ("h", h, 1e-6, 1e-6)], "Allo");
    let (khalf_est, h_est) = (param_ds_val(&allo_res, 0, "Khalf"), param_ds_val(&allo_res, 0, "h"));
    assert_approx_eq(khalf_est.powf(h_est), khalf.powf(h), 1e-4, 1e-6, "Allo Kprime");
    assert_approx_eq(eval_model_at(allo.as_ref(), &allo_res, 0, khalf), 0.5 * allo_vmax, 1e-6, 1e-6, "v(Khalf)");
}

#[test]
fn enzyme_competitive_noncompetitive_uncompetitive_inhibition_oracles() {
    let s = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let (vmax, km, ki) = (100.0_f64, 5.0, 3.0);
    let inh_concs = [0.0_f64, 4.0, 12.0];
    let comp_y = |i: f64| -> Vec<f64> { s.iter().map(|&x| vmax * x / (km * (1.0 + i / ki) + x)).collect() };
    let noncomp_y = |i: f64| -> Vec<f64> { s.iter().map(|&x| (vmax / (1.0 + i / ki)) * x / (km + x)).collect() };
    let uncomp_y = |i: f64| -> Vec<f64> { s.iter().map(|&x| (vmax / (1.0 + i / ki)) * x / (km / (1.0 + i / ki) + x)).collect() };

    let fixed_i = 6.0;
    for (eq_id, ki_name, y_vec) in [
        ("competitive_inhibition", "Ki", comp_y(fixed_i)),
        ("noncompetitive_inhibition", "Ki", noncomp_y(fixed_i)),
        ("uncompetitive_inhibition", "alphaKi", uncomp_y(fixed_i)),
    ] {
        let model = get_builtin_model(eq_id).unwrap();
        let cfg = cfg_rules(vec![
            rule_const("Vmax", vmax),
            rule_const("Km", km),
            rule_const("I", fixed_i),
            rule_local_bounds(ki_name, Some(5.0), Some(1e-6), None),
        ]);
        let res = fit_with(model.as_ref(), &[series_xy(0, eq_id, &s, &y_vec)], &cfg);
        assert_converged(&res, eq_id);
        assert_approx_eq(param_ds_val(&res, 0, ki_name), ki, 1e-6, 1e-6, &format!("{eq_id} {ki_name}"));
    }

    let make_global_cfg = |ki_name: &str| {
        cfg_rules(vec![
            rule_shared_bounds("Vmax", Some(80.0), Some(1e-6), None),
            rule_shared_bounds("Km", Some(4.0), Some(1e-6), None),
            rule_shared_bounds(ki_name, Some(4.0), Some(1e-6), None),
        ])
    };
    let comp_dsl = dsl_compile(
        "<A> Y = Vmax * X / (Km * (1 + 0 / Ki) + X)\n<B> Y = Vmax * X / (Km * (1 + 4 / Ki) + X)\n<C> Y = Vmax * X / (Km * (1 + 12 / Ki) + X)",
    );
    let noncomp_dsl = dsl_compile(
        "<A> Y = (Vmax / (1 + 0 / Ki)) * X / (Km + X)\n<B> Y = (Vmax / (1 + 4 / Ki)) * X / (Km + X)\n<C> Y = (Vmax / (1 + 12 / Ki)) * X / (Km + X)",
    );
    let uncomp_dsl = dsl_compile(
        "<A> Y = (Vmax / (1 + 0 / alphaKi)) * X / (Km / (1 + 0 / alphaKi) + X)\n<B> Y = (Vmax / (1 + 4 / alphaKi)) * X / (Km / (1 + 4 / alphaKi) + X)\n<C> Y = (Vmax / (1 + 12 / alphaKi)) * X / (Km / (1 + 12 / alphaKi) + X)",
    );

    let mk_ds = |f: &dyn Fn(f64) -> Vec<f64>| -> Vec<DatasetSeries> {
        inh_concs.iter().enumerate().map(|(k, &ci)| series_xy(k, &format!("I={ci}"), &s, &f(ci))).collect()
    };
    for (tag, dsl, ki_name, ds) in [
        ("global comp", &comp_dsl, "Ki", mk_ds(&comp_y)),
        ("global noncomp", &noncomp_dsl, "Ki", mk_ds(&noncomp_y)),
        ("global uncomp", &uncomp_dsl, "alphaKi", mk_ds(&uncomp_y)),
    ] {
        let res = fit_with(dsl, &ds, &make_global_cfg(ki_name));
        assert_params_approx(&res, 0, &[("Vmax", vmax, 1e-6, 1e-6), ("Km", km, 1e-6, 1e-6), (ki_name, ki, 1e-6, 1e-6)], tag);
    }

    let ds_noisy: Vec<_> = inh_concs
        .iter()
        .enumerate()
        .map(|(k, &ci)| {
            let y: Vec<f64> = comp_y(ci).into_iter().enumerate().map(|(j, yv)| yv + if (j + k) % 2 == 0 { 0.4 } else { -0.4 }).collect();
            series_xy(k, &format!("I={ci}"), &s, &y)
        })
        .collect();
    let cfg_c = make_global_cfg("Ki");
    let fit_c = fit_with(&comp_dsl, &ds_noisy, &cfg_c);
    let (fit_nc, cmp) = compare_models(
        &fit_c,
        &noncomp_dsl,
        &ds_noisy,
        &HookContext::default(),
        &cfg_c,
        &ModelComparisonSpec::Aicc { alternative_equation_id: "global_noncomp".to_owned() },
    )
    .unwrap();
    let ModelComparisonOutcome::Aicc(aicc) = cmp else { panic!("expected Aicc") };
    assert_eq!(aicc.preferred_index, 0);
    assert!(fit_c.rss < fit_nc.rss);
}

// ============================================================================
// SECTION D: PK / Exponential Decay, Binding Kinetics, Growth Models
// ============================================================================

#[test]
fn pk_one_two_three_phase_exponential_decay_oracles() {
    let t1: Vec<f64> = (0..=12).map(|i| i as f64 * 1.5).collect();
    let (y0_1, plat_1, k1) = (100.0_f64, 10.0, 0.25);
    let y1: Vec<f64> = t1.iter().map(|&t| plat_1 + (y0_1 - plat_1) * (-k1 * t).exp()).collect();
    let res1 = fit_default(get_builtin_model("one_phase_decay").unwrap().as_ref(), &[series_xy(0, "1Phase", &t1, &y1)]);
    assert_converged(&res1, "one_phase_decay");
    assert_params_approx(&res1, 0, &[("Y0", y0_1, 1e-6, 1e-6), ("Plateau", plat_1, 1e-6, 1e-6), ("K", k1, 1e-7, 1e-7)], "1P");
    assert_approx_eq(find_derived(&res1, 0, "HalfLife"), 2_f64.ln() / k1, 1e-6, 1e-6, "1P HalfLife");
    assert_approx_eq(find_derived(&res1, 0, "Tau"), 1.0 / k1, 1e-6, 1e-6, "1P Tau");
    assert_approx_eq(find_derived(&res1, 0, "Span"), y0_1 - plat_1, 1e-6, 1e-6, "1P Span");

    let t2: [f64; 14] = [0.0, 0.2, 0.4, 0.8, 1.2, 1.8, 2.5, 4.0, 6.0, 9.0, 14.0, 20.0, 28.0, 40.0];
    let (y0_2, plat_2, pf2, kf2, ks2) = (100.0_f64, 5.0_f64, 65.0_f64, 1.4_f64, 0.12_f64);
    let y2: Vec<f64> = t2.iter().map(|&t| plat_2 + (y0_2 - plat_2) * 0.01 * (pf2 * (-kf2 * t).exp() + (100.0 - pf2) * (-ks2 * t).exp())).collect();
    let cfg2 = SolverConfig {
        parameter_rules: vec![
            rule_local_bounds("Y0", Some(100.0), None, None),
            rule_local_bounds("Plateau", Some(5.0), None, None),
            rule_local_bounds("PercentFast", Some(50.0), Some(0.0), Some(100.0)),
            rule_local_bounds("KFast", Some(1.0), Some(0.2), None),
            rule_local_bounds("KSlow", Some(0.1), Some(1e-4), None),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let res2 = fit_with(get_builtin_model("two_phase_decay").unwrap().as_ref(), &[series_xy(0, "2Phase", &t2, &y2)], &cfg2);
    assert_converged(&res2, "two_phase_decay");
    assert_params_approx(
        &res2,
        0,
        &[
            ("Y0", y0_2, 1e-5, 1e-6),
            ("Plateau", plat_2, 1e-5, 1e-6),
            ("PercentFast", pf2, 1e-4, 1e-6),
            ("KFast", kf2, 1e-5, 1e-6),
            ("KSlow", ks2, 1e-5, 1e-6),
        ],
        "2P",
    );
    assert_approx_eq(2_f64.ln() / param_ds_val(&res2, 0, "KFast"), 2_f64.ln() / kf2, 1e-5, 1e-6, "2P HalfLifeFast");
    assert_approx_eq(2_f64.ln() / param_ds_val(&res2, 0, "KSlow"), 2_f64.ln() / ks2, 1e-5, 1e-6, "2P HalfLifeSlow");

    let t3: [f64; 17] = [0.0, 0.1, 0.2, 0.4, 0.7, 1.0, 1.5, 2.5, 4.0, 6.0, 9.0, 14.0, 22.0, 35.0, 55.0, 80.0, 120.0];
    let (y0_3, plat_3, pf3, ps3, kf3, ks3, kvs3) = (120.0_f64, 4.0_f64, 45.0_f64, 35.0_f64, 2.5_f64, 0.35_f64, 0.03_f64);
    let y3: Vec<f64> = t3
        .iter()
        .map(|&t| plat_3 + (y0_3 - plat_3) * 0.01 * (pf3 * (-kf3 * t).exp() + ps3 * (-ks3 * t).exp() + (100.0 - pf3 - ps3) * (-kvs3 * t).exp()))
        .collect();
    let cfg3 = SolverConfig {
        parameter_rules: vec![
            rule_local_bounds("Y0", Some(120.0), None, None),
            rule_local_bounds("Plateau", Some(4.0), Some(0.0), None),
            rule_local_bounds("PercentFast", Some(40.0), Some(0.0), Some(100.0)),
            rule_local_bounds("PercentSlow", Some(35.0), Some(0.0), Some(100.0)),
            rule_local_bounds("KFast", Some(2.0), Some(1.0), None),
            rule_local_bounds("KSlow", Some(0.3), Some(0.1), Some(1.0)),
            rule_local_bounds("KVerySlow", Some(0.04), Some(1e-4), Some(0.1)),
        ],
        max_iters: 3000,
        ..SolverConfig::default()
    };
    let res3 = fit_with(get_builtin_model("three_phase_decay").unwrap().as_ref(), &[series_xy(0, "3Phase", &t3, &y3)], &cfg3);
    assert_converged(&res3, "three_phase_decay");
    assert_params_approx(
        &res3,
        0,
        &[
            ("Y0", y0_3, 1e-4, 1e-5),
            ("Plateau", plat_3, 1e-4, 1e-5),
            ("PercentFast", pf3, 1e-3, 1e-4),
            ("PercentSlow", ps3, 1e-3, 1e-4),
            ("KFast", kf3, 1e-4, 1e-5),
            ("KSlow", ks3, 1e-4, 1e-5),
            ("KVerySlow", kvs3, 1e-5, 1e-5),
        ],
        "3P",
    );
}

#[test]
fn binding_assoc_then_dissoc_and_growth_gompertz_logistic_oracles() {
    let (bmax, kon, koff, lig, time0) = (250.0_f64, 4.0e6, 0.08, 2.0e-8, 30.0);
    let (kobs, kd) = (kon * lig + koff, koff / kon);
    let eq = bmax * lig / (lig + kd);
    let y_wash = eq * (1.0 - (-kobs * time0).exp());
    let t_ad: Vec<f64> = (0..=24).map(|i| i as f64 * 2.5).collect();
    let y_ad: Vec<f64> =
        t_ad.iter().map(|&t| if t < time0 { eq * (1.0 - (-kobs * t).exp()) } else { y_wash * (-koff * (t - time0)).exp() }).collect();
    let ad_cfg = SolverConfig {
        parameter_rules: vec![
            rule_local_bounds("Bmax", Some(200.0), Some(1.0), None),
            rule_local_bounds("Kon", Some(2.0e6), Some(1.0), None),
            rule_local_bounds("Koff", Some(0.05), Some(1e-6), None),
            rule_const("LigandConc", lig),
            rule_const("Time0", time0),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let ad_res = fit_with(get_builtin_model("assoc_then_dissoc").unwrap().as_ref(), &[series_xy(0, "AssocDissoc", &t_ad, &y_ad)], &ad_cfg);
    assert_converged(&ad_res, "assoc_then_dissoc");
    assert_approx_eq(param_ds_val(&ad_res, 0, "Bmax"), bmax, 1e-4, 1e-6, "AD Bmax");
    let (kon_est, koff_est) = (param_ds_val(&ad_res, 0, "Kon"), param_ds_val(&ad_res, 0, "Koff"));
    assert_lre_at_least(kon_est, kon, 7.0, "AD Kon");
    assert_approx_eq(koff_est, koff, 1e-6, 1e-6, "AD Koff");
    assert_approx_eq(koff_est / kon_est, kd, 1e-13, 1e-6, "AD Kd_M");

    let t_g: Vec<f64> = (0..=20).map(|i| i as f64).collect();
    let (log_ym, log_y0, log_k) = (500.0_f64, 12.0, 0.42);
    let y_log: Vec<f64> = t_g.iter().map(|&t| log_ym * log_y0 / (log_y0 + (log_ym - log_y0) * (-log_k * t).exp())).collect();
    let m_log = get_builtin_model("logistic_growth").unwrap();
    let res_log = fit_default(m_log.as_ref(), &[series_xy(0, "Logistic", &t_g, &y_log)]);
    assert_converged(&res_log, "logistic_growth");
    assert_params_approx(&res_log, 0, &[("YM", log_ym, 1e-5, 1e-6), ("Y0", log_y0, 1e-5, 1e-6), ("k", log_k, 1e-6, 1e-6)], "Logistic");
    assert_approx_eq(
        eval_model_at(m_log.as_ref(), &res_log, 0, ((log_ym - log_y0) / log_y0).ln() / log_k),
        0.5 * log_ym,
        1e-5,
        1e-6,
        "Logistic inflection",
    );

    let (gom_ym, gom_y0, gom_k) = (400.0_f64, 8.0, 0.28);
    let y_gom: Vec<f64> = t_g.iter().map(|&t| gom_ym * (-(gom_ym / gom_y0).ln() * (-gom_k * t).exp()).exp()).collect();
    let m_gom = get_builtin_model("gompertz_growth").unwrap();
    let res_gom = fit_default(m_gom.as_ref(), &[series_xy(0, "Gompertz", &t_g, &y_gom)]);
    assert_converged(&res_gom, "gompertz_growth");
    assert_params_approx(&res_gom, 0, &[("YM", gom_ym, 1e-5, 1e-6), ("Y0", gom_y0, 1e-5, 1e-6), ("k", gom_k, 1e-6, 1e-6)], "Gompertz");
    assert_approx_eq(
        eval_model_at(m_gom.as_ref(), &res_gom, 0, ((gom_ym / gom_y0).ln()).ln() / gom_k),
        gom_ym / std::f64::consts::E,
        1e-5,
        1e-6,
        "Gompertz inflection",
    );
}

// ============================================================================
// SECTION E: Shared Global, Constraints & Hooks, Profile CI, F/AICc, Deming/WLS/ROUT
// ============================================================================

#[test]
fn global_shared_parameters_multi_dataset_oracle() {
    let t: Vec<f64> = (0..=10).map(|i| i as f64 * 1.5).collect();
    let (k_shared, plat_shared, y0s) = (0.32_f64, 7.5, [130.0_f64, 85.0, 48.0]);
    let noise = [0.4, -0.5, 0.3, -0.2, 0.4, -0.3, 0.2, -0.4, 0.3, -0.2, 0.0];
    let curves: Vec<DatasetSeries> = y0s
        .iter()
        .enumerate()
        .map(|(ds_i, &y0)| {
            let y: Vec<f64> = t
                .iter()
                .zip(noise)
                .map(|(&ti, e)| plat_shared + (y0 - plat_shared) * (-k_shared * ti).exp() + if ds_i % 2 == 0 { e } else { -e })
                .collect();
            series_xy(ds_i, &format!("Curve{ds_i}"), &t, &y)
        })
        .collect();
    let cfg = cfg_rules(vec![rule_local("Y0"), rule_shared("Plateau"), rule_shared_bounds("K", None, Some(1e-5), None)]);
    let res = fit_with(get_builtin_model("one_phase_decay").unwrap().as_ref(), &curves, &cfg);
    assert_converged(&res, "global shared one_phase_decay");
    assert_eq!((res.k_free, res.n_obs, res.df), (5, 33, 28));

    let (k0, p0) = (find_param_ds(&res, 0, "K"), find_param_ds(&res, 0, "Plateau"));
    assert!(k0.notes.contains(&ParameterNote::SharedGlobal) && p0.notes.contains(&ParameterNote::SharedGlobal));
    for ds_i in 0..3 {
        assert_approx_eq(dataset_param_val(&res, ds_i, "K"), k0.estimate, 1e-12, 1e-12, "shared K");
        assert_approx_eq(dataset_param_val(&res, ds_i, "Plateau"), p0.estimate, 1e-12, 1e-12, "shared Plateau");
    }
    assert!((k0.estimate - k_shared).abs() <= 2.5 * k0.se && (p0.estimate - plat_shared).abs() <= 2.5 * p0.se);
    for (ds_i, &y0_true) in y0s.iter().enumerate() {
        let p = find_param_ds(&res, ds_i, "Y0");
        assert!(!p.notes.contains(&ParameterNote::SharedGlobal) && (p.estimate - y0_true).abs() <= 2.5 * p.se);
    }
}

#[test]
fn box_constraints_and_hooked_constants_end_to_end_oracle() {
    let t: Vec<f64> = (0..=8).map(|i| i as f64 * 1.5).collect();
    let m1 = get_builtin_model("one_phase_decay").unwrap();

    let y_neg: Vec<f64> = t.iter().map(|&ti| -12.0 + 112.0 * (-0.2 * ti).exp()).collect();
    let res_lb =
        fit_with(m1.as_ref(), &[series_xy(0, "LowerBound", &t, &y_neg)], &cfg_rules(vec![rule_local_bounds("Plateau", Some(5.0), Some(0.0), None)]));
    let plat_lb = find_param_ds(&res_lb, 0, "Plateau");
    assert_approx_eq(plat_lb.estimate, 0.0, 1e-10, 1e-10, "HitLowerBound Plateau");
    assert!(plat_lb.notes.contains(&ParameterNote::HitLowerBound));

    let y_fast: Vec<f64> = t.iter().map(|&ti| 10.0 + 90.0 * (-0.35 * ti).exp()).collect();
    let res_ub = fit_with(
        m1.as_ref(),
        &[series_xy(0, "UpperBound", &t, &y_fast)],
        &cfg_rules(vec![rule_local_bounds("K", Some(0.1), Some(1e-4), Some(0.15))]),
    );
    let k_ub = find_param_ds(&res_ub, 0, "K");
    assert_approx_eq(k_ub.estimate, 0.15, 1e-10, 1e-10, "HitUpperBound K");
    assert!(k_ub.notes.contains(&ParameterNote::HitUpperBound));

    let log_x: Vec<f64> = (-20..=-8).map(|i| i as f64 * 0.5).collect();
    let y_base: Vec<f64> = log_x.iter().map(|&x| 42.0 + (220.0 - 42.0) / (1.0 + 10_f64.powf(-7.0 - x))).collect();
    let base_sheet = xy_sheet("sheet-base", "Baseline 4PL", &log_x, &[("Agonist", &y_base)]);
    let base_nl = nl_fast("log_agonist_vs_response_4pl");
    let base_spec = AnalysisSpec::new(SheetId::new("ana-base"), base_sheet.id.clone(), AnalysisKind::NonlinearRegression(base_nl.clone())).unwrap();
    let base_res = execute_nonlinear_analysis(&base_spec, &base_nl, &base_sheet, &HookContext::default()).unwrap();

    let true_ki = 1e-8_f64;
    let true_log_ic50 = true_ki.log10() + (1.0 + 3.0 / 1.5_f64).log10();
    let y_comp: Vec<f64> = log_x.iter().map(|&x| 42.0 + (500.0 - 42.0) / (1.0 + 10_f64.powf(x - true_log_ic50))).collect();
    let comp_sheet = xy_sheet("sheet-comp", "Competition", &log_x, &[("Radioligand", &y_comp)]);

    let mut info = InfoSheet::new_preseeded("info-assay", "Assay Info").unwrap();
    info.set_constant("HotConc_nM", ConstantValue::Number(3.0), None).unwrap();
    info.set_constant("HotKd_nM", ConstantValue::Number(1.5), None).unwrap();

    let s_vals = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let v_enz: Vec<f64> = s_vals.iter().map(|&sv| 100.0 * sv / (4.0 * (1.0 + 25.0 / 5.0) + sv)).collect();
    let enz_sheet = xy_sheet("sheet-enz", "Enzyme", &s_vals, &[("25.0 uM", &v_enz)]);

    let (infos, results, sheets) = ([info], [base_res], [enz_sheet.clone()]);
    let hooks = HookContext { info_sheets: &infos, result_sheets: &results, data_sheets: &sheets, ..HookContext::default() };

    let ki_nl = NonlinearFitSpec {
        parameter_rules: vec![
            rule_hook(
                "HotConc_nM",
                HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("info-assay")), constant_name: "HotConc_nM".to_owned() },
            ),
            rule_hook("HotKd_nM", HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("info-assay")), constant_name: "HotKd_nM".to_owned() }),
            rule_hook(
                "Bottom",
                HookSource::ResultScalar { result_sheet_id: SheetId::new("ana-base"), parameter_key: "Bottom".to_owned(), data_set_index: Some(0) },
            ),
        ],
        ..nl_fast("one_site_fit_ki")
    };
    let ki_spec = AnalysisSpec::new(SheetId::new("ana-ki"), comp_sheet.id.clone(), AnalysisKind::NonlinearRegression(ki_nl.clone())).unwrap();
    let ki_res = execute_nonlinear_analysis(&ki_spec, &ki_nl, &comp_sheet, &hooks).unwrap();
    assert_approx_eq(ki_res.lookup_scalar("LogKi", Some(0)).unwrap(), true_ki.log10(), 1e-5, 1e-5, "ResultSheet hooked LogKi");
    assert_approx_eq(ki_res.lookup_scalar("Bottom", None).unwrap(), 42.0, 1e-6, 1e-6, "ResultScalar Bottom");

    let enz_nl = NonlinearFitSpec {
        parameter_rules: vec![
            rule_const("Vmax", 100.0),
            rule_const("Km", 4.0),
            rule_hook("I", HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("sheet-enz"), data_set_index: 0 }),
        ],
        ..nl_fast("competitive_inhibition")
    };
    let enz_spec = AnalysisSpec::new(SheetId::new("ana-enz"), enz_sheet.id.clone(), AnalysisKind::NonlinearRegression(enz_nl.clone())).unwrap();
    let enz_res = execute_nonlinear_analysis(&enz_spec, &enz_nl, &enz_sheet, &hooks).unwrap();
    assert_approx_eq(enz_res.lookup_scalar("Ki", Some(0)).unwrap(), 5.0, 1e-6, 1e-6, "ColumnTitleValue Ki");
}

#[test]
fn profile_likelihood_vs_wald_asymmetric_ci_and_open_bound_oracle() {
    let s = [1.0, 2.0, 4.0, 6.0, 10.0, 15.0, 25.0, 40.0, 70.0, 100.0];
    let noise = [3.2, -2.8, 4.1, -3.5, 2.7, -4.4, 3.8, -3.0, 2.5, -2.6];
    let y: Vec<f64> = s.iter().zip(noise).map(|(&sv, e)| 120.0 * sv / (12.0 + sv) + e).collect();
    let model = get_builtin_model("michaelis_menten").unwrap();
    let res = fit_with(model.as_ref(), &[series_xy(0, "MM CI", &s, &y)], &SolverConfig { compute_profile_ci: true, ..SolverConfig::default() });
    assert_converged(&res, "MM profile CI");

    let km = find_param_ds(&res, 0, "Km");
    let (half_sym_lo, half_sym_hi) = (km.estimate - km.symmetric_ci_lower, km.symmetric_ci_upper - km.estimate);
    assert_approx_eq(half_sym_lo, half_sym_hi, 1e-10, 1e-10, "Wald symmetric");
    let (CiBound::Finite(p_lo), CiBound::Finite(p_hi)) = (km.ci_lower, km.ci_upper) else { panic!("expected finite Km profile CI") };
    assert!((p_hi - km.estimate) > (km.estimate - p_lo) + 0.05 && ((p_hi - km.estimate) - half_sym_hi).abs() > 0.05);

    assert_eq!(res.df, 8);
    let expected_ss_crit = res.rss * (1.0 + (2.3060041350333704_f64).powi(2) / 8.0);
    let profile_rss = |km_test: f64| -> f64 {
        let f: Vec<f64> = s.iter().map(|&sv| sv / (km_test + sv)).collect();
        let vmax_star = y.iter().zip(&f).map(|(&yi, &fi)| yi * fi).sum::<f64>() / f.iter().map(|&fi| fi * fi).sum::<f64>();
        y.iter().zip(&f).map(|(&yi, &fi)| (yi - vmax_star * fi).powi(2)).sum()
    };
    assert_lre_at_least(profile_rss(p_lo), expected_ss_crit, 4.0, "profile RSS lo");
    assert_lre_at_least(profile_rss(p_hi), expected_ss_crit, 4.0, "profile RSS hi");

    let s_part = [0.3, 0.7, 1.2, 1.9, 2.8, 3.8];
    let v_part: Vec<f64> = s_part.iter().zip([-1.8, 2.2, -2.4, 2.6, -2.0, -0.4]).map(|(&x, e)| 120.0 * x / (12.0 + x) + e).collect();
    let cfg_open = SolverConfig {
        compute_profile_ci: true,
        identify_unstable_parameters: true,
        parameter_rules: vec![rule_local_bounds("Vmax", Some(120.0), Some(0.0), None), rule_local_bounds("Km", Some(12.0), Some(0.0), None)],
        ..SolverConfig::default()
    };
    let res_open = fit_with(model.as_ref(), &[series_xy(0, "IncompleteMM", &s_part, &v_part)], &cfg_open);
    let vmax_p = find_param_ds(&res_open, 0, "Vmax");
    assert!(res_open.identifiability.is_ambiguous() && vmax_p.notes.contains(&ParameterNote::Unstable));
    assert!(
        matches!(vmax_p.ci_upper, CiBound::Open) || matches!(vmax_p.ci_lower, CiBound::Open) || vmax_p.notes.contains(&ParameterNote::UnboundedCi)
    );
}

#[test]
fn model_comparison_extra_f_test_and_aicc_oracles() {
    let t = [0.0, 0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 5.0, 8.0, 12.0, 18.0, 26.0, 36.0];
    let noise = [0.5, -0.4, 0.6, -0.5, 0.3, -0.6, 0.4, -0.3, 0.5, -0.4, 0.2, -0.3, 0.2];
    let (m1, m2) = (get_builtin_model("one_phase_decay").unwrap(), get_builtin_model("two_phase_decay").unwrap());
    let f_spec = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "two_phase_decay".to_owned() };

    let y_mono: Vec<f64> = t.iter().zip(noise).map(|(&ti, e)| 8.0 + 92.0 * (-0.25_f64 * ti).exp() + e).collect();
    let ds_mono = [series_xy(0, "Mono", &t, &y_mono)];
    let (_, cmp_mono) =
        compare_models(&fit_default(m1.as_ref(), &ds_mono), m2.as_ref(), &ds_mono, &HookContext::default(), &SolverConfig::default(), &f_spec)
            .unwrap();
    assert!(matches!(cmp_mono, ModelComparisonOutcome::ExtraSumOfSquaresF(f) if !f.preferred_complex_model && f.p_value > 0.05));

    let y_bi: Vec<f64> = t.iter().zip(noise).map(|(&ti, e)| 5.0 + 55.0 * (-2.0_f64 * ti).exp() + 45.0 * (-0.1_f64 * ti).exp() + e).collect();
    let ds_bi = [series_xy(0, "BiExp", &t, &y_bi)];
    let cfg_2p = SolverConfig {
        parameter_rules: vec![
            rule_local_bounds("PercentFast", Some(55.0), Some(0.0), Some(100.0)),
            rule_local_bounds("KFast", Some(1.5), Some(0.2), None),
            rule_local_bounds("KSlow", Some(0.1), Some(1e-4), None),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let (_, cmp_bi) = compare_models(&fit_default(m1.as_ref(), &ds_bi), m2.as_ref(), &ds_bi, &HookContext::default(), &cfg_2p, &f_spec).unwrap();
    assert!(matches!(cmp_bi, ModelComparisonOutcome::ExtraSumOfSquaresF(f) if f.preferred_complex_model && f.p_value < 1e-4));

    let f_dir = compare_models_f_test(120.0, 20, 40.0, 18, 0.05).unwrap();
    assert_approx_eq(f_dir.f_stat, 18.0, 1e-12, 1e-12, "exact F-stat");
    assert_eq!((f_dir.df_num, f_dir.df_den), (2, 18));
    assert!(f_dir.p_value < 1e-4 && f_dir.preferred_complex_model);

    let aicc_dir = compare_models_aicc(30.0, 3, 26.0, 5, 15).unwrap();
    assert_approx_eq(aicc_dir.aicc_a, 15.0 * (30.0 / 15.0_f64).ln() + 8.0 + 40.0 / 10.0, 1e-12, 1e-12, "exact AICc_A");
    assert_approx_eq(aicc_dir.aicc_b, 15.0 * (26.0 / 15.0_f64).ln() + 12.0 + 84.0 / 8.0, 1e-12, 1e-12, "exact AICc_B");
    assert_approx_eq(aicc_dir.weight_a + aicc_dir.weight_b, 1.0, 1e-12, 1e-12, "weights sum");
    assert_approx_eq(aicc_dir.evidence_ratio, (0.5 * (aicc_dir.aicc_a - aicc_dir.aicc_b).abs()).exp(), 1e-12, 1e-12, "ER");
    assert_eq!(aicc_dir.preferred_index, 0);

    let s_vals = [0.5_f64, 1.0, 2.0, 4.0, 6.0, 8.0, 10.0, 14.0, 20.0, 30.0, 50.0, 80.0];
    let y_hill: Vec<f64> = s_vals
        .iter()
        .enumerate()
        .map(|(i, &sv)| 100.0 * sv.powf(2.5) / (8.0_f64.powf(2.5) + sv.powf(2.5)) + if i % 2 == 0 { 0.3 } else { -0.3 })
        .collect();
    let ds_hill = [series_xy(0, "Hill", &s_vals, &y_hill)];
    let (mm, allo) = (get_builtin_model("michaelis_menten").unwrap(), get_builtin_model("allosteric_sigmoidal_kinetics").unwrap());
    let (_, cmp_aicc) = compare_models(
        &fit_default(mm.as_ref(), &ds_hill),
        allo.as_ref(),
        &ds_hill,
        &HookContext::default(),
        &SolverConfig::default(),
        &ModelComparisonSpec::Aicc { alternative_equation_id: "allosteric_sigmoidal_kinetics".to_owned() },
    )
    .unwrap();
    assert!(matches!(cmp_aicc, ModelComparisonOutcome::Aicc(h) if h.preferred_index == 1));
}

#[test]
fn deming_model_ii_and_weighting_and_poisson_and_robust_oracles() {
    let mut dem_pts = Vec::with_capacity(16);
    for xk in [-3.0_f64, -1.0, 1.0, 3.0] {
        let yk = 1.0 + 2.0 * xk;
        dem_pts.extend([DataPoint::xy(xk + 1.5, yk), DataPoint::xy(xk - 1.5, yk), DataPoint::xy(xk, yk + 1.5), DataPoint::xy(xk, yk - 1.5)]);
    }
    let dem = deming_regression(&dem_pts, &DemingOptions { variance_ratio_lambda: 1.0, ..Default::default() }).unwrap();
    let n = dem_pts.len() as f64;
    let (xb, yb) = (dem_pts.iter().map(|p| p.x[0]).sum::<f64>() / n, dem_pts.iter().map(|p| p.y).sum::<f64>() / n);
    let s_xx: f64 = dem_pts.iter().map(|p| (p.x[0] - xb).powi(2)).sum();
    let s_yy: f64 = dem_pts.iter().map(|p| (p.y - yb).powi(2)).sum();
    let s_xy: f64 = dem_pts.iter().map(|p| (p.x[0] - xb) * (p.y - yb)).sum();
    let exp_slope = ((s_yy - s_xx) + ((s_yy - s_xx).powi(2) + 4.0 * s_xy * s_xy).sqrt()) / (2.0 * s_xy);
    assert_approx_eq(dem.slope, exp_slope, 1e-12, 1e-12, "Deming slope");
    assert_approx_eq(dem.intercept, yb - exp_slope * xb, 1e-12, 1e-12, "Deming intercept");
    let ols = simple_linear_regression(&dem_pts, &Default::default()).unwrap();
    assert!(ols.slope < 1.95 && (dem.slope - 2.0).abs() < (ols.slope - 2.0).abs());
    assert!(dem.se_slope > 0.0 && dem.ci_slope.0 < dem.slope && dem.slope < dem.ci_slope.1);

    let (dem_x, dem_y): (Vec<f64>, Vec<f64>) = dem_pts.iter().map(|p| (p.x[0], p.y)).unzip();
    let dem_sheet = xy_sheet("sheet-dem", "Deming", &dem_x, &[("Y", &dem_y)]);
    let lin_spec = LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() };
    let dem_ana = AnalysisSpec::new(SheetId::new("ana-dem"), dem_sheet.id.clone(), AnalysisKind::LinearRegression(lin_spec)).unwrap();
    let dem_rs = execute_linear_analysis(&dem_ana, &lin_spec, &dem_sheet).unwrap();
    assert_approx_eq(dem_rs.lookup_scalar("Slope", Some(0)).unwrap(), dem.slope, 1e-10, 1e-10, "Deming ResultSheet slope");

    let c_vals = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0];
    let rel_err = [0.05, -0.05, 0.05, -0.05, 0.05, -0.05, 0.05, -0.05, 0.05, -0.05];
    let (true_bmax, true_kd) = (220.0_f64, 6.0);
    let y_cv: Vec<f64> = c_vals.iter().zip(rel_err).map(|(&c, r)| (true_bmax * c / (true_kd + c)) * (1.0 + r)).collect();
    let bind_model = get_builtin_model("one_site_specific_binding").unwrap();
    let ds_cv = [series_xy(0, "ConstCV", &c_vals, &y_cv)];

    let kd_unw = param_ds_val(&fit_default(bind_model.as_ref(), &ds_cv), 0, "Kd");
    let kd_w = param_ds_val(
        &fit_with(bind_model.as_ref(), &ds_cv, &SolverConfig { weighting: WeightingScheme::RelativeSquare, ..Default::default() }),
        0,
        "Kd",
    );
    assert!((kd_w - true_kd).abs() < (kd_unw - true_kd).abs());
    for scheme in [WeightingScheme::Relative, WeightingScheme::InverseX, WeightingScheme::InverseX2] {
        assert_converged(&fit_with(bind_model.as_ref(), &ds_cv, &SolverConfig { weighting: scheme, ..Default::default() }), &format!("{scheme:?}"));
    }

    let mut xy_msd = XyTable::new(SubcolumnMode::MeanSdN, 1, c_vals.len()).unwrap();
    xy_msd.data_sets[0].title = "PreSummarized".to_owned();
    let mut sheet_msd = DataSheet::new("sheet-msd", "MeanSdN", TableArchetype::Xy(xy_msd)).unwrap();
    for (r, &c) in c_vals.iter().enumerate() {
        sheet_msd.set_x_or_time_cell(r, 0, Cell::number(c)).unwrap();
        let clean = true_bmax * c / (true_kd + c);
        sheet_msd.set_cell(CellCoord::new(r, 0, 0), Cell::number(clean * (1.0 + rel_err[r]))).unwrap();
        sheet_msd.set_cell(CellCoord::new(r, 0, 1), Cell::number(0.05 * clean)).unwrap();
        sheet_msd.set_cell(CellCoord::new(r, 0, 2), Cell::number(3.0)).unwrap();
    }
    let msd_nl = NonlinearFitSpec { weighting: WeightingScheme::ObservedSd, ..nl_fast("one_site_specific_binding") };
    let msd_spec = AnalysisSpec::new(SheetId::new("ana-msd"), sheet_msd.id.clone(), AnalysisKind::NonlinearRegression(msd_nl.clone())).unwrap();
    let kd_msd = execute_nonlinear_analysis(&msd_spec, &msd_nl, &sheet_msd, &HookContext::default()).unwrap().lookup_scalar("Kd", Some(0)).unwrap();
    assert!((kd_msd - true_kd).abs() < 0.2, "kd_msd={kd_msd}");

    let t_p: Vec<f64> = (0..=10).map(|i| i as f64).collect();
    let y_counts: Vec<f64> = t_p.iter().map(|&ti| (20.0 + 500.0 * (-0.3_f64 * ti).exp()).round()).collect();
    let dec_m = get_builtin_model("one_phase_decay").unwrap();
    let res_pois = fit_with(
        dec_m.as_ref(),
        &[series_xy(0, "Counts", &t_p, &y_counts)],
        &SolverConfig { method: FitMethod::PoissonDeviance, ..Default::default() },
    );
    assert_converged(&res_pois, "PoissonDeviance");
    assert!((param_ds_val(&res_pois, 0, "K") - 0.3).abs() < 0.02);
    let exp_dev: f64 = t_p
        .iter()
        .zip(&y_counts)
        .map(|(&ti, &yi)| {
            let mu = eval_model_at(dec_m.as_ref(), &res_pois, 0, ti);
            2.0 * (yi * (yi / mu).ln() - (yi - mu))
        })
        .sum();
    assert_approx_eq(res_pois.weighted_ss, exp_dev, 1e-5, 1e-6, "PoissonDeviance objective");

    let log_d: Vec<f64> = (-20..=-8).map(|i| i as f64 * 0.5).collect();
    let mut y_rob: Vec<f64> = log_d.iter().map(|&x| 10.0 + 90.0 / (1.0 + 10_f64.powf((-7.0 - x) * 1.2))).collect();
    y_rob[3] += 80.0;
    let ag_m = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let ds_rob = [series_xy(0, "Outlier", &log_d, &y_rob)];
    let ec_ols = param_ds_val(&fit_default(ag_m.as_ref(), &ds_rob), 0, "LogEC50");
    let ec_rob =
        param_ds_val(&fit_with(ag_m.as_ref(), &ds_rob, &SolverConfig { method: FitMethod::RobustCauchy, ..Default::default() }), 0, "LogEC50");
    assert!((ec_rob - (-7.0)).abs() < 0.05 && (ec_rob - (-7.0)).abs() < (ec_ols - (-7.0)).abs());
}

#[test]
fn rout_outlier_elimination_q1_percent_and_guards_oracle() {
    let log_c: Vec<f64> = (-22..=-8).map(|i| i as f64 * 0.5).collect();
    let jitter = [0.15, -0.18, 0.12, -0.14, 0.20, 0.0, -0.16, 0.14, -0.12, 0.18, -0.15, 0.0, 0.12, -0.14, 0.16];
    let mut y: Vec<f64> = log_c.iter().zip(jitter).map(|(&x, e)| 10.0 + 90.0 / (1.0 + 10_f64.powf((-7.5 - x) * 1.1)) + e).collect();
    y[5] += 85.0;
    y[11] -= 55.0;

    let ag_m = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let rout_cfg = SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, ..SolverConfig::default() };
    let res = fit_with(ag_m.as_ref(), &[series_xy(0, "ROUT", &log_c, &y)], &rout_cfg);
    let rout = res.rout.as_ref().unwrap();
    assert_approx_eq(rout.q_threshold_percent, 1.0, 1e-12, 1e-12, "ROUT Q=1%");
    assert_eq!(rout.outlier_indices, vec![5, 11]);
    assert_eq!((res.n_outliers_excluded, res.df), (2, 9));
    assert_params_approx(
        &res,
        0,
        &[("Bottom", 10.0, 0.3, 1e-2), ("Top", 100.0, 0.3, 1e-2), ("LogEC50", -7.5, 0.02, 1e-2), ("HillSlope", 1.1, 0.03, 1e-2)],
        "ROUT clean",
    );

    let clean_pts: Vec<DataPoint> = (0..20).map(|i| DataPoint::xy(i as f64, 2.0 * i as f64 + 5.0)).collect();
    let mut dirty_res: Vec<f64> = (0..20).map(|i| if i % 2 == 0 { 0.08 } else { -0.08 }).collect();
    assert!(rout_detect_outliers(&clean_pts, &dirty_res, 2, 1.0).unwrap().outlier_indices.is_empty());
    dirty_res[4] = 15.0;
    dirty_res[14] = -18.0;
    assert_eq!(rout_detect_outliers(&clean_pts, &dirty_res, 2, 1.0).unwrap().outlier_indices, vec![4, 14]);

    let mut pre_ds = series_xy(0, "MeanSdN", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[10.0; 6]);
    pre_ds.is_pre_summarized = true;
    for pt in &mut pre_ds.points {
        pt.sd = Some(0.5);
        pt.n = Some(3.0);
    }
    assert!(matches!(fit_nonlinear(ag_m.as_ref(), &[pre_ds], &HookContext::default(), &rout_cfg), Err(FitError::RequiresRawReplicates)));
}
