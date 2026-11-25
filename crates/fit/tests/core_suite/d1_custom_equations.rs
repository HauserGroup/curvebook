//! Independent D1 numerical oracles for custom implicit and ODE equations (`R4.3`).
//!
//! Reference constants: `reference/custom_equations.py` (`mpmath` 1.3.0, 80/120 digits,
//! corroborated with R 4.5.2 `stats::uniroot` and `stats::nls`).
//! References: Brent (1973); Dormand & Prince (1980); Hairer, Nørsett & Wanner (1993); Bates & Watts (1988).

use std::collections::HashMap;
use std::time::Instant;

use curvebook_doc::{NonlinearFitSpec, ResultStatus};
use curvebook_fit::{DslEquationKind, FitError, FitStatus, HookContext, ParameterNote, ParametricModel, SolverConfig, fit_nonlinear};
use curvebook_testkit::assert_approx_eq;

use crate::common::{
    assert_gof_num, assert_num_cell, cfg_fast_rules, dsl_compile, exec_nl, find_param_exact_ds, find_tab, fit_with, nl_fast, rule_local_init,
    rule_shared_bounds, series_xy, tab_row, xy_sheet,
};

const IMPLICIT_ROOT_REL_TOL: f64 = 1e-12;
const IMPLICIT_ROOT_ABS_TOL: f64 = 1e-15;
const IMPLICIT_RESIDUAL_REL_TOL: f64 = 1e-11;
const IMPLICIT_RESIDUAL_ABS_TOL: f64 = 1e-15;
const ODE_T0_ABS_TOL: f64 = 1e-14;
const ODE_TRAJ_ABS_TOL: f64 = 5e-7;
const ODE_TRAJ_REL_TOL: f64 = 5e-7;
const ODE_FIT_PARAM_ABS_TOL: f64 = 1e-5;
const ODE_FIT_PRED_ABS_TOL: f64 = 1e-5;
const ODE_FIT_RSS_ABS_TOL: f64 = 1e-6;

#[test]
fn implicit_monotone_cubic_matches_bracketed_oracle_across_predictors_and_scales() {
    let a = 2.5_f64;
    let predictors: [f64; 7] = [-3.0, -0.4, -1e-9, 0.0, 1e-9, 0.4, 3.0];
    let expected_roots: [f64; 7] =
        [-1.787_609_408_374_904, -0.682_327_803_828_019_4, -2.5e-9, 0.0, 2.5e-9, 0.682_327_803_828_019_4, 1.787_609_408_374_904];
    let literal_scales: [&str; 15] =
        ["1e-200", "1e-100", "1e-50", "1e-24", "1e-12", "1e-6", "0.000125", "1", "7.5", "1e6", "1e12", "1e24", "1e50", "1e100", "1e200"];

    let check_residual = |root: f64, x: f64, expected: f64, ctx: &str| {
        let res_bound = IMPLICIT_RESIDUAL_ABS_TOL.max(IMPLICIT_RESIDUAL_REL_TOL * (1.0 + 3.0 * expected * expected) * expected.abs());
        assert_approx_eq(root + root.powi(3) - a * x, 0.0, res_bound, 0.0, ctx);
    };

    let canonical = dsl_compile("Y + Y^3 = A * X");
    assert_eq!((canonical.kind(), canonical.param_names()), (DslEquationKind::Implicit, &["A".to_owned()][..]));
    let mut unscaled = [0.0_f64; 7];
    for (i, (&x, &exp)) in predictors.iter().zip(&expected_roots).enumerate() {
        let root = canonical.eval(&[x], &[a], 0).unwrap();
        assert_approx_eq(root, exp, IMPLICIT_ROOT_ABS_TOL, IMPLICIT_ROOT_REL_TOL, &format!("canonical X={x}"));
        check_residual(root, x, exp, &format!("canonical res X={x}"));
        unscaled[i] = root;
    }

    let mut roots_by_scale = Vec::with_capacity(literal_scales.len());
    for scale_str in literal_scales {
        let model = dsl_compile(&format!("{scale_str} * (Y + Y^3 - A * X) = 0"));
        assert_eq!((model.kind(), model.param_names()), (DslEquationKind::Implicit, &["A".to_owned()][..]));
        let mut scaled = [0.0_f64; 7];
        for (i, (&x, &exp)) in predictors.iter().zip(&expected_roots).enumerate() {
            let root = model.eval(&[x], &[a], 0).unwrap();
            assert_approx_eq(root, exp, IMPLICIT_ROOT_ABS_TOL, IMPLICIT_ROOT_REL_TOL, &format!("s={scale_str} X={x}"));
            assert_approx_eq(root, unscaled[i], IMPLICIT_ROOT_ABS_TOL, IMPLICIT_ROOT_REL_TOL, &format!("s={scale_str} vs unscaled"));
            check_residual(root, x, exp, &format!("s={scale_str} res X={x}"));
            scaled[i] = root;
        }
        roots_by_scale.push(scaled);
    }

    for (i, &exp) in expected_roots.iter().enumerate() {
        let min_r = roots_by_scale.iter().map(|r| r[i]).fold(f64::INFINITY, f64::min);
        let max_r = roots_by_scale.iter().map(|r| r[i]).fold(f64::NEG_INFINITY, f64::max);
        assert_approx_eq(max_r - min_r, 0.0, IMPLICIT_ROOT_ABS_TOL.max(IMPLICIT_ROOT_REL_TOL * exp.abs()), 0.0, "scale spread");
    }

    let param_scaled = dsl_compile("S * (Y + Y^3 - A * X) = 0");
    assert_eq!(param_scaled.param_names(), &["S", "A"]);
    for s_val in [1e-100_f64, 1.0, 1e100] {
        for (&x, &exp) in predictors.iter().zip(&expected_roots) {
            assert_approx_eq(
                param_scaled.eval(&[x], &[s_val, a], 0).unwrap(),
                exp,
                IMPLICIT_ROOT_ABS_TOL,
                IMPLICIT_ROOT_REL_TOL,
                &format!("S={s_val:e}"),
            );
        }
    }
}

#[test]
fn forced_ode_nonzero_t0_matches_integrating_factor_under_interleaved_queries() {
    let model = dsl_compile("Y(1.5) = Y0\n<A> dY/dX = -k * Y + q * cos(w * X)\n<B> dY/dX = -k * Y + q * exp(-r * (X - 1.5))");
    assert_eq!((model.kind(), model.param_names()), (DslEquationKind::OdeFirstOrder, &["k", "q", "w", "r", "Y0"].map(str::to_owned)[..]));

    let times: [f64; 7] = [-0.5, 0.25, 1.0, 1.5, 2.25, 3.5, 5.0];
    let param_sets: [[f64; 5]; 3] = [[0.6, 2.5, 1.8, 1.1, 3.0], [1.25, -1.5, 2.5, 0.4, -2.0], [0.35, 1.2, 0.9, 0.85, 4.5]];
    let expected: [[[f64; 7]; 2]; 3] = [
        [
            [
                8.717_184_492_821_877,
                6.936_406_218_042_491,
                4.959_577_695_638_048,
                3.0,
                0.570_532_529_790_912,
                1.293_761_217_203_519_3,
                0.483_589_450_722_761_6,
            ],
            [
                -18.564_132_115_278_227,
                -2.839_383_481_701_488_7,
                2.132_605_371_271_048_4,
                3.0,
                2.909_850_250_649_440_4,
                1.855_537_903_485_947_4,
                0.873_252_743_831_969_4,
            ],
        ],
        [
            [
                -29.726_563_241_336_383,
                -12.265_312_887_997_57,
                -4.711_957_903_946_162,
                -2.0,
                -0.864_025_965_445_447_2,
                -0.315_319_851_106_731_7,
                -0.238_745_946_743_892_02,
            ],
            [
                -6.793_894_335_152_23,
                -4.032_033_579_345_544,
                -2.595_003_916_149_058,
                -2.0,
                -1.399_468_772_185_807_7,
                -0.812_247_583_412_484_8,
                -0.438_133_028_659_878_24,
            ],
        ],
        [
            [
                6.489_382_710_137_542,
                5.764_132_422_704_625,
                5.084_127_735_837_796_5,
                4.5,
                3.359_179_679_641_423,
                1.164_355_513_040_907_3,
                -0.266_126_563_968_880_6,
            ],
            [
                0.757_419_941_401_008_9,
                3.742_298_794_563_292,
                4.548_581_887_433_162,
                4.5,
                4.038_302_383_904_743,
                2.987_998_138_434_162_5,
                1.904_414_290_622_406,
            ],
        ],
    ];

    let eval_schedule: &[(usize, usize, usize, bool)] = &[
        (0, 0, 5, false),
        (0, 0, 1, false),
        (0, 0, 3, false),
        (0, 0, 6, false),
        (0, 0, 0, false),
        (0, 1, 6, false),
        (0, 1, 0, false),
        (0, 1, 3, false),
        (0, 1, 4, false),
        (1, 0, 4, false),
        (1, 0, 2, false),
        (1, 0, 5, false),
        (1, 1, 1, false),
        (1, 1, 5, false),
        (1, 1, 2, false),
        (1, 1, 3, false),
        (0, 0, 4, false),
        (0, 0, 2, false),
        (0, 0, 5, true),
        (0, 0, 1, true),
        (0, 0, 3, true),
        (2, 0, 6, false),
        (2, 0, 0, false),
        (2, 0, 4, false),
        (2, 0, 2, false),
        (2, 0, 3, false),
        (2, 1, 0, false),
        (2, 1, 6, false),
        (2, 1, 1, false),
        (2, 1, 5, false),
        (0, 1, 1, false),
        (0, 1, 2, false),
        (0, 1, 5, false),
        (0, 1, 6, false),
        (0, 1, 0, false),
        (1, 0, 0, false),
        (1, 0, 1, false),
        (1, 0, 3, false),
        (1, 0, 6, false),
        (1, 0, 4, false),
        (1, 1, 0, false),
        (1, 1, 4, false),
        (1, 1, 6, false),
        (1, 1, 1, false),
        (2, 0, 1, false),
        (2, 0, 5, false),
        (2, 1, 2, false),
        (2, 1, 3, false),
        (2, 1, 4, false),
        (0, 0, 6, false),
        (0, 0, 0, false),
    ];

    let mut first_observed: HashMap<(usize, usize, usize), f64> = HashMap::new();
    let mut revisits = 0_usize;
    for (step, &(p_i, ds_i, t_i, warm_hit)) in eval_schedule.iter().enumerate() {
        let got = model.eval(&[times[t_i]], &param_sets[p_i], ds_i).unwrap();
        let (atol, rtol) = if t_i == 3 { (ODE_T0_ABS_TOL, 0.0) } else { (ODE_TRAJ_ABS_TOL, ODE_TRAJ_REL_TOL) };
        assert_approx_eq(got, expected[p_i][ds_i][t_i], atol, rtol, &format!("step {step} P{p_i} DS{ds_i}"));
        if let Some(&prev) = first_observed.get(&(p_i, ds_i, t_i)) {
            revisits += 1;
            if warm_hit {
                assert_eq!(got.to_bits(), prev.to_bits(), "step {step} warm-cache revisit");
            }
        } else {
            first_observed.insert((p_i, ds_i, t_i), got);
        }
    }
    assert_eq!(first_observed.len(), 3 * 2 * 7);
    assert!(revisits >= 9);
}

#[test]
fn two_dataset_noisy_shared_rate_ode_fit_matches_closed_form_oracle() {
    let ode_src = "Y0 = A\ndY/dX = -k * Y";
    let model = dsl_compile(ode_src);
    assert_eq!((model.kind(), model.param_names()), (DslEquationKind::OdeFirstOrder, &["k".to_owned(), "A".to_owned()][..]));

    let xs = [0.0_f64, 0.5, 1.0, 2.0, 3.5, 5.0, 7.5, 10.0];
    let ys_ds1 = [79.1_f64, 68.4, 55.6, 40.7, 22.9, 14.6, 5.4, 2.8];
    let ys_ds2 = [35.8_f64, 28.2, 25.3, 16.9, 10.8, 5.7, 2.8, 0.9];

    let (exp_k, exp_a1, exp_a2) = (0.347_168_143_071_492_74_f64, 79.841_039_078_081_56_f64, 34.974_218_331_249_666_f64);
    let (exp_se_k, exp_se_a1, exp_se_a2) = (0.005_623_589_038_429_765_5_f64, 0.588_160_712_637_133_2_f64, 0.510_697_111_133_333_6_f64);
    let exp_fitted_ds1 = [
        79.841_039_078_081_56,
        67.118_087_961_595_12,
        56.422_583_969_816_03,
        39.873_078_038_445_73,
        23.687_539_388_334_49,
        14.072_139_646_025_926,
        5.907_818_087_907_918,
        2.480_242_197_544_467_5,
    ];
    let exp_fitted_ds2 = [
        34.974_218_331_249_666,
        29.400_953_312_358_265,
        24.715_807_726_948_1,
        17.466_327_504_727_98,
        10.376_282_471_568_166,
        6.164_274_539_145_576,
        2.587_908_700_257_89,
        1.086_465_471_052_621_8,
    ];
    let (exp_rss_total, exp_rss_ds1, exp_rss_ds2) = (8.072_911_971_797_648_f64, 4.811_860_811_463_919_f64, 3.261_051_160_333_729_5_f64);

    let rules = vec![rule_shared_bounds("k", Some(0.3), Some(0.0), None), rule_local_init("A", 50.0)];
    let fit = fit_with(
        &model,
        &[series_xy(0, "Group A", &xs, &ys_ds1), series_xy(1, "Group B", &xs, &ys_ds2)],
        &SolverConfig { parameter_rules: rules.clone(), compute_profile_ci: false, ..SolverConfig::default() },
    );
    assert_eq!((fit.status, fit.n_obs, fit.k_free, fit.df), (FitStatus::Converged, 16, 3, 13));

    let (p_k, p_a1, p_a2) = (find_param_exact_ds(&fit, None, "k"), find_param_exact_ds(&fit, Some(0), "A"), find_param_exact_ds(&fit, Some(1), "A"));
    assert!(
        p_k.notes.contains(&ParameterNote::SharedGlobal)
            && !p_a1.notes.contains(&ParameterNote::SharedGlobal)
            && !p_a2.notes.contains(&ParameterNote::SharedGlobal)
    );
    for (p, exp, se, lbl) in [(p_k, exp_k, exp_se_k, "k"), (p_a1, exp_a1, exp_se_a1, "A1"), (p_a2, exp_a2, exp_se_a2, "A2")] {
        assert_approx_eq(p.estimate, exp, ODE_FIT_PARAM_ABS_TOL, 0.0, lbl);
        assert_approx_eq(p.se, se, ODE_FIT_PARAM_ABS_TOL, 0.0, &format!("SE({lbl})"));
    }
    for (ds_i, exp_y) in [&exp_fitted_ds1, &exp_fitted_ds2].into_iter().enumerate() {
        assert_eq!(fit.datasets[ds_i].y_hat.len(), 8);
        for (i, (&got, &exp)) in fit.datasets[ds_i].y_hat.iter().zip(exp_y).enumerate() {
            assert_approx_eq(got, exp, ODE_FIT_PRED_ABS_TOL, 0.0, &format!("DS{ds_i} y_hat[{i}]"));
        }
    }
    assert_approx_eq(fit.rss, exp_rss_total, ODE_FIT_RSS_ABS_TOL, 0.0, "total RSS");
    assert_approx_eq(fit.datasets[0].ss_residual, exp_rss_ds1, ODE_FIT_RSS_ABS_TOL, 0.0, "DS1 RSS");
    assert_approx_eq(fit.datasets[1].ss_residual, exp_rss_ds2, ODE_FIT_RSS_ABS_TOL, 0.0, "DS2 RSS");

    let sheet = xy_sheet("ode-fit-data", "Shared-Rate Decay", &xs, &[("Group A", &ys_ds1), ("Group B", &ys_ds2)]);
    let result = exec_nl(&sheet, NonlinearFitSpec { custom_dsl: Some(ode_src.to_owned()), parameter_rules: rules, ..nl_fast("custom_dsl") });
    assert_eq!(result.status, ResultStatus::UpToDate);
    for (lbl, est, se) in [("k", exp_k, exp_se_k), ("A [Group A]", exp_a1, exp_se_a1), ("A [Group B]", exp_a2, exp_se_a2)] {
        let row = tab_row(&result, "nonlinear_parameters", lbl);
        assert_num_cell(&row[0], est, ODE_FIT_PARAM_ABS_TOL, 0.0, lbl);
        assert_num_cell(&row[1], se, ODE_FIT_PARAM_ABS_TOL, 0.0, &format!("SE {lbl}"));
    }
    assert_gof_num(&result, "Degrees of Freedom", 13.0, 0.0, 0.0);
    assert_gof_num(&result, "Sum of Squares (RSS)", exp_rss_total, ODE_FIT_RSS_ABS_TOL, 0.0);
    assert_gof_num(&result, "Number of points analyzed", 16.0, 0.0, 0.0);
    let res_tab = find_tab(&result, "nonlinear_residuals");
    assert_eq!(res_tab.rows.len(), 16);
    for (i, &exp) in exp_fitted_ds1.iter().chain(&exp_fitted_ds2).enumerate() {
        assert_num_cell(&res_tab.rows[i][0], exp, ODE_FIT_PRED_ABS_TOL, 0.0, &format!("sheet y_hat[{i}]"));
    }
}

#[test]
fn retained_root_domain_and_ode_budget_guards() {
    assert_eq!(dsl_compile("1 / (Y - A) = 0").eval(&[1.0], &[2.0], 0), None);
    assert_eq!(dsl_compile("Y^2 + 1 = 0").eval(&[0.0], &[], 0), None);
    assert_eq!(dsl_compile("Y + Y^3 = A * X").eval(&[f64::NAN], &[2.5], 0), None);

    let stiff = dsl_compile("Y0 = A\ndY/dX = -K * Y");
    let xs: Vec<f64> = (1..=30).map(|i| i as f64).collect();
    let cfg = cfg_fast_rules(vec![rule_shared_bounds("K", Some(100_000.0), Some(50_000.0), None)]);
    let t0 = Instant::now();
    let err = fit_nonlinear(&stiff, &[series_xy(0, "Stiff", &xs, &vec![0.0; xs.len()])], &HookContext::default(), &cfg).unwrap_err();
    assert!(t0.elapsed().as_secs_f64() < 1.0);
    assert!(matches!(err, FitError::OdeIntegrationFailed { .. }), "got {err:?}");
}
