//! Independent D1 numerical oracles for custom implicit and ODE equations (`R4.3`).
//!
//! Reference constants are generated independently in `reference/custom_equations.py`
//! using `mpmath` 1.3.0 (identical IEEE-754 binary64 values at 80 and 120 decimal digits)
//! and corroborated independently with R 4.5.2 (`stats::uniroot` and `stats::nls`).
//!
//! # References
//! - Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*, Ch. 4. Prentice-Hall.
//! - Dormand, J. R. & Prince, P. J. (1980). A family of embedded Runge-Kutta formulae.
//!   *J. Comput. Appl. Math.* 6(1), 19-26.
//! - Hairer, E., Nørsett, S. P. & Wanner, G. (1993). *Solving Ordinary Differential Equations I:
//!   Nonstiff Problems*, 2nd ed., §II.4. Springer.
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.

use std::collections::HashMap;
use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, NonlinearFitSpec, ParameterRule, ParameterScope, ResultCell, ResultSheet, ResultStatus,
    SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, DslEquationKind, FitError, FitStatus, HookContext, ParameterNote, ParametricModel, SolverConfig,
    execute_analysis, fit_nonlinear,
};
use curvebook_testkit::assert_approx_eq;

// Safeguarded Brent root refinement uses rel_tol = 1e-13 and abs_tol ~ 1e-14 * bracket_scale,
// yielding <= 1e-12 relative root error and unscaled residual <= O(1e-11 * F'(y) * |y|).
const IMPLICIT_ROOT_REL_TOL: f64 = 1e-12;
const IMPLICIT_ROOT_ABS_TOL: f64 = 1e-15;
const IMPLICIT_RESIDUAL_REL_TOL: f64 = 1e-11;
const IMPLICIT_RESIDUAL_ABS_TOL: f64 = 1e-15;

// Dormand-Prince RK45 uses rtol = 1e-8, atol = 1e-10; across spans up to |t - t0| = 3.5 with
// rates up to 1.25 (backward integration amplification <= exp(1.25 * 2) ~ 12.2), global error
// stays well inside 5e-7 relative and 5e-7 absolute. At t == t0, the initial state is exact.
const ODE_T0_ABS_TOL: f64 = 1e-14;
const ODE_TRAJ_ABS_TOL: f64 = 5e-7;
const ODE_TRAJ_REL_TOL: f64 = 5e-7;

// Two-dataset ODE regression combines RK45 (rtol = 1e-8) with central finite-difference Jacobians
// (step h = 1e-6 * max(|theta|, 1)) and LM termination tolerances (ftol = 1e-15, xtol = 1e-14).
const ODE_FIT_PARAM_ABS_TOL: f64 = 1e-5;
const ODE_FIT_PRED_ABS_TOL: f64 = 1e-5;
const ODE_FIT_RSS_ABS_TOL: f64 = 1e-6;

#[test]
fn implicit_monotone_cubic_matches_bracketed_oracle_across_predictors_and_scales() {
    let a = 2.5_f64;
    // Negative, near-zero negative, exact zero, near-zero positive, and positive predictors.
    let predictors: [f64; 7] = [-3.0, -0.4, -1e-9, 0.0, 1e-9, 0.4, 3.0];
    // Independent 80/120-digit mpmath bracketed roots for Y + Y^3 = 2.5 * X (Cardano-verified).
    let expected_roots: [f64; 7] =
        [-1.787_609_408_374_904, -0.682_327_803_828_019_4, -2.5e-9, 0.0, 2.5e-9, 0.682_327_803_828_019_4, 1.787_609_408_374_904];

    // Documented scale range: 400 orders of magnitude [1e-200, 1e200], keeping s * |F(y)| inside
    // IEEE-754 normal binary64 range at bracketing probes near the root for all predictors.
    let literal_scales: [&str; 15] =
        ["1e-200", "1e-100", "1e-50", "1e-24", "1e-12", "1e-6", "0.000125", "1", "7.5", "1e6", "1e12", "1e24", "1e50", "1e100", "1e200"];

    let canonical = CompiledDslModel::compile("implicit_cubic_canonical", "Y + Y^3 = A * X").expect("canonical implicit compiles");
    assert_eq!(canonical.kind(), DslEquationKind::Implicit);
    assert_eq!(canonical.param_names(), &["A"]);

    let mut unscaled_roots = [0.0_f64; 7];
    for (i, (&x, &expected)) in predictors.iter().zip(expected_roots.iter()).enumerate() {
        let root = canonical.eval(&[x], &[a], 0).unwrap_or_else(|| panic!("canonical implicit failed at X={x}"));
        assert_approx_eq(root, expected, IMPLICIT_ROOT_ABS_TOL, IMPLICIT_ROOT_REL_TOL, &format!("canonical root at X={x}"));
        let unscaled_res = root + root.powi(3) - a * x;
        let res_scale = (1.0 + 3.0 * expected * expected) * expected.abs();
        let res_bound = IMPLICIT_RESIDUAL_ABS_TOL.max(IMPLICIT_RESIDUAL_REL_TOL * res_scale);
        assert_approx_eq(unscaled_res, 0.0, res_bound, 0.0, &format!("canonical residual at X={x}"));
        unscaled_roots[i] = root;
    }

    let mut roots_by_scale: Vec<(&str, [f64; 7])> = Vec::with_capacity(literal_scales.len());
    for scale_str in literal_scales {
        let eq_src = format!("{scale_str} * (Y + Y^3 - A * X) = 0");
        let model = CompiledDslModel::compile(format!("implicit_cubic_scale_{scale_str}"), &eq_src)
            .unwrap_or_else(|e| panic!("failed compiling `{eq_src}`: {e:?}"));
        assert_eq!(model.kind(), DslEquationKind::Implicit);
        assert_eq!(model.param_names(), &["A"]);

        let mut scaled_roots = [0.0_f64; 7];
        for (i, (&x, &expected)) in predictors.iter().zip(expected_roots.iter()).enumerate() {
            let root = model.eval(&[x], &[a], 0).unwrap_or_else(|| panic!("scaled ({scale_str}) implicit failed at X={x}"));
            assert_approx_eq(root, expected, IMPLICIT_ROOT_ABS_TOL, IMPLICIT_ROOT_REL_TOL, &format!("scale={scale_str} root vs oracle at X={x}"));
            assert_approx_eq(
                root,
                unscaled_roots[i],
                IMPLICIT_ROOT_ABS_TOL,
                IMPLICIT_ROOT_REL_TOL,
                &format!("scale={scale_str} root vs unscaled at X={x}"),
            );
            let unscaled_res = root + root.powi(3) - a * x;
            let res_scale = (1.0 + 3.0 * expected * expected) * expected.abs();
            let res_bound = IMPLICIT_RESIDUAL_ABS_TOL.max(IMPLICIT_RESIDUAL_REL_TOL * res_scale);
            assert_approx_eq(unscaled_res, 0.0, res_bound, 0.0, &format!("scale={scale_str} unscaled residual at X={x}"));
            scaled_roots[i] = root;
        }
        roots_by_scale.push((scale_str, scaled_roots));
    }

    // Verify pairwise agreement across the entire 400-decade scale range.
    for i in 0..predictors.len() {
        let expected = expected_roots[i];
        let min_root = roots_by_scale.iter().map(|(_, r)| r[i]).fold(f64::INFINITY, f64::min);
        let max_root = roots_by_scale.iter().map(|(_, r)| r[i]).fold(f64::NEG_INFINITY, f64::max);
        assert_approx_eq(
            max_root - min_root,
            0.0,
            IMPLICIT_ROOT_ABS_TOL.max(IMPLICIT_ROOT_REL_TOL * expected.abs()),
            0.0,
            &format!("pairwise scale spread at X={}", predictors[i]),
        );
    }

    // Also verify parameterized residual scale `S * (Y + Y^3 - A * X) = 0` so probe-anchor scaling
    // with extreme parameter `S` remains invariant.
    let param_scaled = CompiledDslModel::compile("implicit_cubic_param_scale", "S * (Y + Y^3 - A * X) = 0").expect("param-scaled compiles");
    assert_eq!(param_scaled.param_names(), &["S", "A"]);
    for &s_val in &[1e-100_f64, 1.0, 1e100] {
        for (&x, &expected) in predictors.iter().zip(expected_roots.iter()) {
            let root = param_scaled.eval(&[x], &[s_val, a], 0).unwrap_or_else(|| panic!("param-scaled S={s_val:e} failed at X={x}"));
            assert_approx_eq(root, expected, IMPLICIT_ROOT_ABS_TOL, IMPLICIT_ROOT_REL_TOL, &format!("param S={s_val:e} at X={x}"));
        }
    }
}

#[test]
fn forced_ode_nonzero_t0_matches_integrating_factor_under_interleaved_queries() {
    let source = "\
Y(1.5) = Y0
<A> dY/dX = -k * Y + q * cos(w * X)
<B> dY/dX = -k * Y + q * exp(-r * (X - 1.5))
";
    let model = CompiledDslModel::compile("forced_ode_t0_1p5", source).expect("forced ODE compiles");
    assert_eq!(model.kind(), DslEquationKind::OdeFirstOrder);
    assert_eq!(model.param_names(), &["k", "q", "w", "r", "Y0"]);

    let times: [f64; 7] = [-0.5, 0.25, 1.0, 1.5, 2.25, 3.5, 5.0];
    let param_sets: [[f64; 5]; 3] = [
        [0.6, 2.5, 1.8, 1.1, 3.0],    // P0
        [1.25, -1.5, 2.5, 0.4, -2.0], // P1
        [0.35, 1.2, 0.9, 0.85, 4.5],  // P2
    ];

    // Independent 80/120-digit mpmath closed-form integrating factor values indexed as
    // expected[param_idx][ds_idx][time_idx].
    let expected: [[[f64; 7]; 2]; 3] = [
        // P0
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
        // P1
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
        // P2
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

    // Explicitly interleaved evaluation sequence (param_idx, ds_idx, time_idx, expect_warm_cache_hit)
    // covering predictions before (t < 1.5), at (t == 1.5), and after (t > 1.5), alternating
    // datasets (0 vs 1) and parameter sets (0..=2 -> 6 trajectories > 4-slot cache), and revisiting
    // times both while cached (Steps 18..20) and after cache eviction/re-meshing (Steps 30..50).
    let eval_schedule: &[(usize, usize, usize, bool)] = &[
        // Phase 1: P0 on DS0 non-monotone (after, before, t0, far-after, far-before)
        (0, 0, 5, false),
        (0, 0, 1, false),
        (0, 0, 3, false),
        (0, 0, 6, false),
        (0, 0, 0, false),
        // Phase 2: alternate to P0 on DS1 (2 trajectories cached)
        (0, 1, 6, false),
        (0, 1, 0, false),
        (0, 1, 3, false),
        (0, 1, 4, false),
        // Phase 3: alternate parameters P1 across DS0 and DS1 (4 trajectories cached)
        (1, 0, 4, false),
        (1, 0, 2, false),
        (1, 0, 5, false),
        (1, 1, 1, false),
        (1, 1, 5, false),
        (1, 1, 2, false),
        (1, 1, 3, false),
        // Phase 4: revisit P0 on DS0 at unvisited interior times AND warm-cache exact hits
        (0, 0, 4, false),
        (0, 0, 2, false),
        (0, 0, 5, true),
        (0, 0, 1, true),
        (0, 0, 3, true),
        // Phase 5: introduce P2 across DS0 and DS1 (evicts older cache slots)
        (2, 0, 6, false),
        (2, 0, 0, false),
        (2, 0, 4, false),
        (2, 0, 2, false),
        (2, 0, 3, false),
        (2, 1, 0, false),
        (2, 1, 6, false),
        (2, 1, 1, false),
        (2, 1, 5, false),
        // Phase 6: complete remaining grid points and revisit earlier combinations post-eviction
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
    let mut revisit_count = 0_usize;
    for (step, &(p_idx, ds_idx, t_idx, expect_warm_hit)) in eval_schedule.iter().enumerate() {
        let t = times[t_idx];
        let exp_val = expected[p_idx][ds_idx][t_idx];
        let got = model
            .eval(&[t], &param_sets[p_idx], ds_idx)
            .unwrap_or_else(|| panic!("step {step}: ODE eval returned None for P{p_idx} DS{ds_idx} t={t}"));
        if t_idx == 3 {
            assert_approx_eq(got, exp_val, ODE_T0_ABS_TOL, 0.0, &format!("step {step} t==t0 P{p_idx} DS{ds_idx}"));
        } else {
            assert_approx_eq(got, exp_val, ODE_TRAJ_ABS_TOL, ODE_TRAJ_REL_TOL, &format!("step {step} P{p_idx} DS{ds_idx} t={t}"));
        }
        if let Some(&prev_val) = first_observed.get(&(p_idx, ds_idx, t_idx)) {
            revisit_count += 1;
            if expect_warm_hit {
                assert_eq!(got.to_bits(), prev_val.to_bits(), "step {step} warm-cache revisit should reproduce identical value");
            }
        } else {
            first_observed.insert((p_idx, ds_idx, t_idx), got);
        }
    }
    // Confirm every (parameter vector, dataset, time) tuple in the 3 x 2 x 7 = 42 matrix was checked.
    assert_eq!(first_observed.len(), 3 * 2 * 7);
    assert!(revisit_count >= 9);
}

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

fn result_row<'a>(result: &'a ResultSheet, tab_id: &str, label: &str) -> Option<&'a [ResultCell]> {
    let tab = result.tabs.iter().find(|tab| tab.id == tab_id)?;
    let row = tab.row_labels.iter().position(|name| name == label)?;
    tab.rows.get(row).map(Vec::as_slice)
}

fn cell_number(cell: Option<&ResultCell>) -> f64 {
    match cell {
        Some(ResultCell::Number { value, .. }) => *value,
        _ => f64::NAN,
    }
}

#[test]
fn two_dataset_noisy_shared_rate_ode_fit_matches_closed_form_oracle() {
    let ode_src = "Y0 = A\ndY/dX = -k * Y";
    let model = CompiledDslModel::compile("shared_rate_decay_ode", ode_src).expect("ODE compiles");
    assert_eq!(model.kind(), DslEquationKind::OdeFirstOrder);
    assert_eq!(model.param_names(), &["k", "A"]);

    let xs = [0.0_f64, 0.5, 1.0, 2.0, 3.5, 5.0, 7.5, 10.0];
    let ys_ds1 = [79.1_f64, 68.4, 55.6, 40.7, 22.9, 14.6, 5.4, 2.8];
    let ys_ds2 = [35.8_f64, 28.2, 25.3, 16.9, 10.8, 5.7, 2.8, 0.9];

    // Independent 80/120-digit mpmath (corroborated with R nls) closed-form least-squares values.
    let exp_k = 0.347_168_143_071_492_74_f64;
    let exp_a1 = 79.841_039_078_081_56_f64;
    let exp_a2 = 34.974_218_331_249_666_f64;
    let exp_se_k = 0.005_623_589_038_429_765_5_f64;
    let exp_se_a1 = 0.588_160_712_637_133_2_f64;
    let exp_se_a2 = 0.510_697_111_133_333_6_f64;
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
    let exp_rss_total = 8.072_911_971_797_648_f64;
    let exp_rss_ds1 = 4.811_860_811_463_919_f64;
    let exp_rss_ds2 = 3.261_051_160_333_729_5_f64;

    let rules = vec![
        ParameterRule {
            name: "k".to_owned(),
            scope: ParameterScope::SharedGlobal,
            initial_override: Some(0.3),
            lower_bound: Some(0.0),
            upper_bound: None,
        },
        ParameterRule { name: "A".to_owned(), scope: ParameterScope::Local, initial_override: Some(50.0), lower_bound: None, upper_bound: None },
    ];

    // 1) Public `fit_nonlinear` entry point.
    let datasets = vec![make_series(0, "Group A", &xs, &ys_ds1), make_series(1, "Group B", &xs, &ys_ds2)];
    let cfg = SolverConfig { parameter_rules: rules.clone(), compute_profile_ci: false, ..SolverConfig::default() };
    let fit = fit_nonlinear(&model, &datasets, &HookContext::default(), &cfg).expect("ODE shared-rate fit succeeds");
    assert_eq!(fit.status, FitStatus::Converged);
    assert_eq!(fit.n_obs, 16);
    assert_eq!(fit.k_free, 3);
    assert_eq!(fit.df, 13);

    let p_k = fit.parameters.iter().find(|p| p.name == "k").expect("shared k");
    let p_a1 = fit.parameters.iter().find(|p| p.name == "A" && p.dataset_index == Some(0)).expect("local A DS1");
    let p_a2 = fit.parameters.iter().find(|p| p.name == "A" && p.dataset_index == Some(1)).expect("local A DS2");
    assert!(p_k.notes.contains(&ParameterNote::SharedGlobal));
    assert!(!p_a1.notes.contains(&ParameterNote::SharedGlobal));
    assert!(!p_a2.notes.contains(&ParameterNote::SharedGlobal));
    assert_approx_eq(p_k.estimate, exp_k, ODE_FIT_PARAM_ABS_TOL, 0.0, "fit_nonlinear shared k");
    assert_approx_eq(p_a1.estimate, exp_a1, ODE_FIT_PARAM_ABS_TOL, 0.0, "fit_nonlinear local A DS1");
    assert_approx_eq(p_a2.estimate, exp_a2, ODE_FIT_PARAM_ABS_TOL, 0.0, "fit_nonlinear local A DS2");
    assert_approx_eq(p_k.se, exp_se_k, ODE_FIT_PARAM_ABS_TOL, 0.0, "fit_nonlinear SE(k)");
    assert_approx_eq(p_a1.se, exp_se_a1, ODE_FIT_PARAM_ABS_TOL, 0.0, "fit_nonlinear SE(A DS1)");
    assert_approx_eq(p_a2.se, exp_se_a2, ODE_FIT_PARAM_ABS_TOL, 0.0, "fit_nonlinear SE(A DS2)");

    assert_eq!(fit.datasets[0].y_hat.len(), 8);
    assert_eq!(fit.datasets[1].y_hat.len(), 8);
    for (i, (&got, &exp)) in fit.datasets[0].y_hat.iter().zip(exp_fitted_ds1.iter()).enumerate() {
        assert_approx_eq(got, exp, ODE_FIT_PRED_ABS_TOL, 0.0, &format!("fit_nonlinear DS1 y_hat[{i}]"));
    }
    for (i, (&got, &exp)) in fit.datasets[1].y_hat.iter().zip(exp_fitted_ds2.iter()).enumerate() {
        assert_approx_eq(got, exp, ODE_FIT_PRED_ABS_TOL, 0.0, &format!("fit_nonlinear DS2 y_hat[{i}]"));
    }
    assert_approx_eq(fit.rss, exp_rss_total, ODE_FIT_RSS_ABS_TOL, 0.0, "fit_nonlinear total RSS");
    assert_approx_eq(fit.datasets[0].ss_residual, exp_rss_ds1, ODE_FIT_RSS_ABS_TOL, 0.0, "fit_nonlinear DS1 RSS");
    assert_approx_eq(fit.datasets[1].ss_residual, exp_rss_ds2, ODE_FIT_RSS_ABS_TOL, 0.0, "fit_nonlinear DS2 RSS");

    // 2) Public typed `execute_analysis` path via `DataSheet` + `NonlinearFitSpec { custom_dsl }`.
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, xs.len()).expect("XyTable");
    xy.data_sets[0].title = "Group A".to_owned();
    xy.data_sets[1].title = "Group B".to_owned();
    let mut sheet = DataSheet::new("ode-fit-data", "Shared-Rate Decay", TableArchetype::Xy(xy)).expect("DataSheet");
    for (row, &x) in xs.iter().enumerate() {
        sheet.set_x_or_time_cell(row, 0, Cell::number(x)).expect("set X");
        sheet.set_cell(CellCoord::new(row, 0, 0), Cell::number(ys_ds1[row])).expect("set Y1");
        sheet.set_cell(CellCoord::new(row, 1, 0), Cell::number(ys_ds2[row])).expect("set Y2");
    }
    let mut nl_spec = NonlinearFitSpec::new("custom_dsl");
    nl_spec.custom_dsl = Some(ode_src.to_owned());
    nl_spec.parameter_rules = rules;
    nl_spec.profile_likelihood_ci = false;
    let spec = AnalysisSpec::new("ode-fit-analysis", "ode-fit-data", AnalysisKind::NonlinearRegression(nl_spec)).expect("AnalysisSpec");
    let result = execute_analysis(&spec, &sheet, &HookContext::default()).expect("execute_analysis succeeds");
    assert_eq!(result.status, ResultStatus::UpToDate);

    let k_row = result_row(&result, "nonlinear_parameters", "k").unwrap_or(&[]);
    assert_approx_eq(cell_number(k_row.first()), exp_k, ODE_FIT_PARAM_ABS_TOL, 0.0, "sheet shared k");
    assert_approx_eq(cell_number(k_row.get(1)), exp_se_k, ODE_FIT_PARAM_ABS_TOL, 0.0, "sheet SE(k)");
    let a1_row = result_row(&result, "nonlinear_parameters", "A [Group A]").unwrap_or(&[]);
    assert_approx_eq(cell_number(a1_row.first()), exp_a1, ODE_FIT_PARAM_ABS_TOL, 0.0, "sheet local A [Group A]");
    assert_approx_eq(cell_number(a1_row.get(1)), exp_se_a1, ODE_FIT_PARAM_ABS_TOL, 0.0, "sheet SE(A [Group A])");
    let a2_row = result_row(&result, "nonlinear_parameters", "A [Group B]").unwrap_or(&[]);
    assert_approx_eq(cell_number(a2_row.first()), exp_a2, ODE_FIT_PARAM_ABS_TOL, 0.0, "sheet local A [Group B]");
    assert_approx_eq(cell_number(a2_row.get(1)), exp_se_a2, ODE_FIT_PARAM_ABS_TOL, 0.0, "sheet SE(A [Group B])");

    assert_approx_eq(
        cell_number(result_row(&result, "nonlinear_gof", "Degrees of Freedom").and_then(<[ResultCell]>::first)),
        13.0,
        0.0,
        0.0,
        "sheet DF",
    );
    assert_approx_eq(
        cell_number(result_row(&result, "nonlinear_gof", "Sum of Squares (RSS)").and_then(<[ResultCell]>::first)),
        exp_rss_total,
        ODE_FIT_RSS_ABS_TOL,
        0.0,
        "sheet RSS",
    );
    assert_approx_eq(
        cell_number(result_row(&result, "nonlinear_gof", "Number of points analyzed").and_then(<[ResultCell]>::first)),
        16.0,
        0.0,
        0.0,
        "sheet n_obs",
    );
    let residuals_tab = result.tabs.iter().find(|t| t.id == "nonlinear_residuals").expect("residuals tab");
    assert_eq!(residuals_tab.rows.len(), 16);
    for (i, &exp) in exp_fitted_ds1.iter().chain(exp_fitted_ds2.iter()).enumerate() {
        let got = cell_number(residuals_tab.rows[i].first());
        assert_approx_eq(got, exp, ODE_FIT_PRED_ABS_TOL, 0.0, &format!("sheet residual tab y_hat[{i}]"));
    }
}

#[test]
fn retained_root_domain_and_ode_budget_guards() {
    // Root-domain guards: pole, unsolvable equation, and NaN predictor cleanly return None.
    let pure_pole = CompiledDslModel::compile("pole", "1 / (Y - A) = 0").expect("pole compiles");
    assert_eq!(pure_pole.eval(&[1.0], &[2.0], 0), None);
    let unsolvable = CompiledDslModel::compile("no_root", "Y^2 + 1 = 0").expect("unsolvable compiles");
    assert_eq!(unsolvable.eval(&[0.0], &[], 0), None);
    let canonical = CompiledDslModel::compile("cubic_nan", "Y + Y^3 = A * X").expect("cubic compiles");
    assert_eq!(canonical.eval(&[f64::NAN], &[2.5], 0), None);

    // ODE step-budget guard: extremely stiff ODE fails cleanly with structured OdeIntegrationFailed.
    let stiff = CompiledDslModel::compile("stiff_budget", "Y0 = A\ndY/dX = -K * Y").expect("stiff compiles");
    let xs: Vec<f64> = (1..=30).map(|i| i as f64).collect();
    let ys: Vec<f64> = vec![0.0; xs.len()];
    let cfg = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: "K".to_owned(),
            scope: ParameterScope::SharedGlobal,
            initial_override: Some(100_000.0),
            lower_bound: Some(50_000.0),
            upper_bound: None,
        }],
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        ..SolverConfig::default()
    };
    let t0 = Instant::now();
    let err = fit_nonlinear(&stiff, &[make_series(0, "Stiff", &xs, &ys)], &HookContext::default(), &cfg)
        .expect_err("stiff ODE must fail cleanly on budget exhaustion");
    assert!(t0.elapsed().as_secs_f64() < 1.0);
    assert!(matches!(err, FitError::OdeIntegrationFailed { .. }), "expected OdeIntegrationFailed, got {err:?}");
}
