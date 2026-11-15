use std::time::Instant;

use lumencraft_doc::{ParameterRule, ParameterScope};
use lumencraft_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, FitError, HookContext, ParametricModel, SolverConfig, fit_nonlinear, get_builtin_model,
};

fn make_dataset(ds_idx: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
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

#[test]
fn r1_s03_10_ode_fit_work_is_bounded_and_returns_cleanly() {
    let model = CompiledDslModel::compile("ode_stiff", "Y0 = A\ndY/dX = -K*(Y - sin(X))").expect("ODE compiles");
    let xs: Vec<f64> = (1..=50).map(|i| i as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| x.sin() + 0.01 * (-x).exp()).collect();

    let cfg = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: "K".to_owned(),
            scope: ParameterScope::SharedGlobal,
            initial_override: Some(2000.0),
            lower_bound: None,
            upper_bound: None,
        }],
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        ..SolverConfig::default()
    };

    let t0 = Instant::now();
    let res = fit_nonlinear(&model, &[make_dataset(0, "A", &xs, &ys)], &HookContext::default(), &cfg);
    let elapsed = t0.elapsed().as_secs_f64();

    assert!(elapsed < 1.0, "stiff ODE fit exceeded 1s budget (took {elapsed:.3}s)");
    if let Err(err) = res {
        assert!(
            matches!(err, FitError::OdeIntegrationFailed { .. }),
            "expected structured OdeIntegrationFailed on stiff budget exhaustion, got {err:?}"
        );
    }
}

#[test]
fn r1_s03_10_unsorted_and_two_sided_ode_sweeps_match_sorted_and_succeed() {
    let model = CompiledDslModel::compile("ode_decay", "Y(0) = A0\ndY/dX = -K * Y").expect("ODE compiles");
    let true_a0 = 12.0;
    let true_k = 0.35;

    let sorted_xs: Vec<f64> = (-3..=10).map(|i| i as f64).collect();
    let sorted_ys: Vec<f64> = sorted_xs.iter().map(|&x| true_a0 * (-true_k * x).exp()).collect();
    let reversed_xs: Vec<f64> = sorted_xs.iter().copied().rev().collect();
    let reversed_ys: Vec<f64> = sorted_ys.iter().copied().rev().collect();

    // Direct model.eval trajectory cache consistency across arbitrary point ordering.
    let eval_params: Vec<f64> = model.param_names().iter().map(|name| if name == "K" { true_k } else { true_a0 }).collect();
    for (&xs, &ys) in reversed_xs.iter().zip(reversed_ys.iter()) {
        let val = model.eval(&[xs], &eval_params, 0).expect("ODE eval should succeed");
        assert!((val - ys).abs() < 1e-6, "mismatch at X={xs}: got {val}, expected {ys}");
    }

    let cfg = SolverConfig {
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        ..SolverConfig::default()
    };

    let fit_sorted = fit_nonlinear(&model, &[make_dataset(0, "Sorted", &sorted_xs, &sorted_ys)], &HookContext::default(), &cfg)
        .expect("sorted ODE fit should succeed");
    let fit_rev = fit_nonlinear(&model, &[make_dataset(0, "Reversed", &reversed_xs, &reversed_ys)], &HookContext::default(), &cfg)
        .expect("unsorted ODE fit should succeed");

    for (p_s, p_r) in fit_sorted.parameters.iter().zip(fit_rev.parameters.iter()) {
        assert!(
            (p_s.estimate - p_r.estimate).abs() < 1e-6,
            "param {} differed between sorted and unsorted: {} vs {}",
            p_s.name,
            p_s.estimate,
            p_r.estimate
        );
        let expected = if p_r.name == "K" { true_k } else { true_a0 };
        assert!((p_r.estimate - expected).abs() < 1e-4, "param {} expected {expected}, got {}", p_r.name, p_r.estimate);
    }
}

#[test]
fn r1_s03_10_stiff_ode_fit_budget_fails_cleanly_with_structured_error() {
    // Extremely stiff linear ODE (K = 100_000) exhausts the deterministic per-fit step budget cleanly.
    let model = CompiledDslModel::compile("very_stiff", "Y0 = A\ndY/dX = -K * Y").expect("ODE compiles");
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
    let err = fit_nonlinear(&model, &[make_dataset(0, "VeryStiff", &xs, &ys)], &HookContext::default(), &cfg)
        .expect_err("stiff budget exhaustion must return clean error");
    assert!(t0.elapsed().as_secs_f64() < 1.0);
    assert!(matches!(err, FitError::OdeIntegrationFailed { .. }), "expected OdeIntegrationFailed, got {err:?}");
}

#[test]
fn r1_s03_11_missing_predictor_returns_none_not_zero() {
    let model = CompiledDslModel::compile("e2", "Y = A*X + B*X2").expect("compiles");
    assert!(model.eval(&[2.0], &[1.0, 100.0], 0).is_none(), "missing X2 must return None rather than defaulting to 0.0");

    let model3 = CompiledDslModel::compile("e3", "Y = A*X1 + B*X3").expect("compiles");
    assert!(model3.eval(&[2.0, 3.0], &[1.0, 5.0], 0).is_none(), "missing X3 must return None rather than defaulting to 0.0");

    // When all required predictors are supplied, evaluation must succeed normally.
    let present = model.eval(&[2.0, 4.0], &[3.0, 10.0], 0).expect("X2 supplied should succeed");
    assert!((present - 46.0).abs() < 1e-12);

    // ODE model referencing X2 must also return None when X2 is missing and succeed when present.
    let ode = CompiledDslModel::compile("ode_x2", "Y0 = 0\ndY/dX = A + B * X2").expect("ODE compiles");
    assert!(ode.eval(&[2.0], &[1.0, 5.0], 0).is_none());
    let ode_ok = ode.eval(&[2.0, 3.0], &[1.0, 5.0], 0).expect("ODE with X2 should succeed");
    assert!((ode_ok - 32.0).abs() < 1e-7);

    // Fitting a 2-predictor equation against 1-predictor data must fail cleanly rather than fitting X2=0.
    let ds_1d = make_dataset(0, "OnlyX1", &[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0]);
    let fit_res = fit_nonlinear(&model, &[ds_1d], &HookContext::default(), &SolverConfig::default());
    assert!(fit_res.is_err(), "fitting Y=A*X+B*X2 with 1D points must error cleanly");
}

#[test]
fn r1_s03_12_extrapolated_and_plateau_unknowns_are_flagged_cleanly() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").expect("4PL model");
    let xs: Vec<f64> = (0..9).map(|i| -9.0 + 0.5 * (i as f64)).collect(); // [-9.0, -5.0]
    let p_true = [0.0_f64, 100.0, -7.0, 1.0];
    let ys: Vec<f64> = xs.iter().map(|&x| model.eval(&[x], &p_true, 0).unwrap()).collect();

    let mut ds = make_dataset(0, "4PL", &xs, &ys);
    // In-range unknown Y (within [y(-9), y(-5)]): no warning expected.
    ds.unknowns_y.push((99, 50.0));
    // Above highest standard's Y (y(-5)=99.0099, so y=99.5 -> x_hat ~ -4.70 > -5.0 = x_max): must flag extrapolation.
    ds.unknowns_y.push((100, 99.5));
    // Below lowest standard's Y (y(-9)=0.9901, so y=0.5 -> x_hat ~ -9.30 < -9.0 = x_min): must flag extrapolation.
    ds.unknowns_y.push((101, 0.5));
    // Above fitted Top plateau (~100): must flag with AboveTopPlateau.
    ds.unknowns_y.push((102, 150.0));
    // Below fitted Bottom plateau (~0): must flag with BelowBottomPlateau.
    ds.unknowns_y.push((103, -25.0));

    // Forward unknown X: one in-range, two extrapolated outside [-9, -5].
    ds.unknowns_x.push((200, -7.0));
    ds.unknowns_x.push((201, -4.0));
    ds.unknowns_x.push((202, -10.5));

    let cfg = SolverConfig {
        interpolate_unknowns: true,
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(model.as_ref(), &[ds], &HookContext::default(), &cfg).expect("4PL fit should succeed");

    assert_eq!(fit.interpolated_x.len(), 5);
    assert!(fit.interpolated_x[0].note.is_none(), "in-range unknown X should have note None, got {:?}", fit.interpolated_x[0].note);

    let above_std = &fit.interpolated_x[1];
    assert!(above_std.output_estimate > -5.0, "expected extrapolated X > -5, got {}", above_std.output_estimate);
    assert!(above_std.note.is_some(), "extrapolated X above Xmax must carry a note");
    assert!(above_std.note.as_deref().unwrap_or("").contains("AboveTopPlateau"));

    let below_std = &fit.interpolated_x[2];
    assert!(below_std.output_estimate < -9.0, "expected extrapolated X < -9, got {}", below_std.output_estimate);
    assert!(below_std.note.is_some(), "extrapolated X below Xmin must carry a note");
    assert!(below_std.note.as_deref().unwrap_or("").contains("BelowBottomPlateau"));

    let above_top = &fit.interpolated_x[3];
    assert!(above_top.output_estimate.is_nan());
    assert!(above_top.note.as_deref().unwrap_or("").contains("AboveTopPlateau"));
    assert!(above_top.note.as_deref().unwrap_or("").contains("outside fitted curve range"));

    let below_bot = &fit.interpolated_x[4];
    assert!(below_bot.output_estimate.is_nan());
    assert!(below_bot.note.as_deref().unwrap_or("").contains("BelowBottomPlateau"));
    assert!(below_bot.note.as_deref().unwrap_or("").contains("outside fitted curve range"));

    assert_eq!(fit.interpolated_y.len(), 3);
    assert!(fit.interpolated_y[0].note.is_none(), "in-range unknown Y should have note None, got {:?}", fit.interpolated_y[0].note);
    assert!(fit.interpolated_y[1].note.is_some(), "forward extrapolation above Xmax must carry a note");
    assert!(fit.interpolated_y[2].note.is_some(), "forward extrapolation below Xmin must carry a note");
}

#[test]
fn r1_s03_12_non_monotone_curve_warns_and_prefers_in_range_root() {
    let quad = CompiledDslModel::compile("quad", "Y = a * X^2 + c").expect("compiles");
    let xs: Vec<f64> = (-5..=5).map(|i| i as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| x * x + 1.0).collect();

    let mut ds = make_dataset(0, "Quad", &xs, &ys);
    ds.unknowns_y.push((10, 5.0)); // Roots inside [-5, 5] at X=-2 and X=+2.

    let cfg = SolverConfig {
        interpolate_unknowns: true,
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(&quad, &[ds], &HookContext::default(), &cfg).expect("quadratic fit should succeed");
    assert_eq!(fit.interpolated_x.len(), 1);
    let u = &fit.interpolated_x[0];
    assert!(u.output_estimate >= -5.0 && u.output_estimate <= 5.0);
    assert!(u.note.as_deref().unwrap_or("").contains("Non-monotone"), "expected non-monotone warning, got {:?}", u.note);
}
