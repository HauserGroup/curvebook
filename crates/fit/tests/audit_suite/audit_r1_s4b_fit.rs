use std::time::Instant;

use curvebook_fit::{FitError, HookContext, ParametricModel, SolverConfig, fit_nonlinear, get_builtin_model};

use crate::common::{cfg_fast, cfg_fast_rules, dsl_compile, fit_with, rule_shared_bounds, rule_shared_init, series_eval, series_xy};

#[test]
fn r1_s03_10_ode_fit_work_is_bounded_and_returns_cleanly() {
    let model = dsl_compile("Y0 = A\ndY/dX = -K*(Y - sin(X))");
    let xs: Vec<f64> = (1..=50).map(|i| i as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| x.sin() + 0.01 * (-x).exp()).collect();
    let cfg = cfg_fast_rules(vec![rule_shared_init("K", 2000.0)]);

    let t0 = Instant::now();
    let res = fit_nonlinear(&model, &[series_xy(0, "A", &xs, &ys)], &HookContext::default(), &cfg);
    assert!(t0.elapsed().as_secs_f64() < 1.0, "stiff ODE fit exceeded 1s budget");
    if let Err(err) = res {
        assert!(matches!(err, FitError::OdeIntegrationFailed { .. }), "got {err:?}");
    }
}

#[test]
fn r1_s03_10_unsorted_and_two_sided_ode_sweeps_match_sorted_and_succeed() {
    let model = dsl_compile("Y(0) = A0\ndY/dX = -K * Y");
    let (true_a0, true_k) = (12.0, 0.35);
    let sorted_xs: Vec<f64> = (-3..=10).map(|i| i as f64).collect();
    let sorted_ys: Vec<f64> = sorted_xs.iter().map(|&x| true_a0 * (-true_k * x).exp()).collect();
    let rev_xs: Vec<f64> = sorted_xs.iter().copied().rev().collect();
    let rev_ys: Vec<f64> = sorted_ys.iter().copied().rev().collect();

    let eval_params: Vec<f64> = model.param_names().iter().map(|n| if n == "K" { true_k } else { true_a0 }).collect();
    for (&x, &y) in rev_xs.iter().zip(rev_ys.iter()) {
        assert!((model.eval(&[x], &eval_params, 0).unwrap() - y).abs() < 1e-6);
    }

    let cfg = cfg_fast();
    let fit_s = fit_with(&model, &[series_xy(0, "Sorted", &sorted_xs, &sorted_ys)], &cfg);
    let fit_r = fit_with(&model, &[series_xy(0, "Reversed", &rev_xs, &rev_ys)], &cfg);
    for (ps, pr) in fit_s.parameters.iter().zip(fit_r.parameters.iter()) {
        assert!((ps.estimate - pr.estimate).abs() < 1e-6);
        let exp = if pr.name == "K" { true_k } else { true_a0 };
        assert!((pr.estimate - exp).abs() < 1e-4);
    }
}

#[test]
fn r1_s03_10_stiff_ode_fit_budget_fails_cleanly_with_structured_error() {
    let model = dsl_compile("Y0 = A\ndY/dX = -K * Y");
    let xs: Vec<f64> = (1..=30).map(|i| i as f64).collect();
    let cfg = cfg_fast_rules(vec![rule_shared_bounds("K", Some(100_000.0), Some(50_000.0), None)]);
    let t0 = Instant::now();
    let err = fit_nonlinear(&model, &[series_xy(0, "VeryStiff", &xs, &vec![0.0; xs.len()])], &HookContext::default(), &cfg).unwrap_err();
    assert!(t0.elapsed().as_secs_f64() < 1.0);
    assert!(matches!(err, FitError::OdeIntegrationFailed { .. }), "got {err:?}");
}

#[test]
fn r1_s03_11_missing_predictor_returns_none_not_zero() {
    let m2 = dsl_compile("Y = A*X + B*X2");
    assert!(m2.eval(&[2.0], &[1.0, 100.0], 0).is_none());
    assert!(dsl_compile("Y = A*X1 + B*X3").eval(&[2.0, 3.0], &[1.0, 5.0], 0).is_none());
    assert!((m2.eval(&[2.0, 4.0], &[3.0, 10.0], 0).unwrap() - 46.0).abs() < 1e-12);

    let ode = dsl_compile("Y0 = 0\ndY/dX = A + B * X2");
    assert!(ode.eval(&[2.0], &[1.0, 5.0], 0).is_none());
    assert!((ode.eval(&[2.0, 3.0], &[1.0, 5.0], 0).unwrap() - 32.0).abs() < 1e-7);

    let ds_1d = series_xy(0, "OnlyX1", &[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0]);
    assert!(fit_nonlinear(&m2, &[ds_1d], &HookContext::default(), &SolverConfig::default()).is_err());
}

#[test]
fn r1_s03_12_extrapolated_and_plateau_unknowns_are_flagged_cleanly() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs: Vec<f64> = (0..9).map(|i| -9.0 + 0.5 * (i as f64)).collect();
    let mut ds = series_eval(model.as_ref(), 0, "4PL", &xs, &[0.0, 100.0, -7.0, 1.0]);
    ds.unknowns_y.extend([(99, 50.0), (100, 99.5), (101, 0.5), (102, 150.0), (103, -25.0)].into_iter().map(curvebook_fit::UnknownResponse::from));
    ds.unknowns_x.extend([(200, -7.0), (201, -4.0), (202, -10.5)]);

    let fit = fit_with(model.as_ref(), &[ds], &SolverConfig { interpolate_unknowns: true, ..cfg_fast() });
    assert_eq!((fit.interpolated_x.len(), fit.interpolated_y.len()), (5, 3));
    assert!(fit.interpolated_x[0].note.is_none());
    assert!(fit.interpolated_x[1].output_estimate > -5.0 && fit.interpolated_x[1].note.as_deref().unwrap_or("").contains("AboveTopPlateau"));
    assert!(fit.interpolated_x[2].output_estimate < -9.0 && fit.interpolated_x[2].note.as_deref().unwrap_or("").contains("BelowBottomPlateau"));
    assert!(
        fit.interpolated_x[3].output_estimate.is_nan()
            && fit.interpolated_x[3].note.as_deref().unwrap_or("").contains("AboveTopPlateau")
            && fit.interpolated_x[3].note.as_deref().unwrap_or("").contains("outside fitted curve range")
    );
    assert!(
        fit.interpolated_x[4].output_estimate.is_nan()
            && fit.interpolated_x[4].note.as_deref().unwrap_or("").contains("BelowBottomPlateau")
            && fit.interpolated_x[4].note.as_deref().unwrap_or("").contains("outside fitted curve range")
    );
    assert!(fit.interpolated_y[0].note.is_none() && fit.interpolated_y[1].note.is_some() && fit.interpolated_y[2].note.is_some());
}

#[test]
fn r1_s03_12_non_monotone_curve_warns_and_prefers_in_range_root() {
    let quad = dsl_compile("Y = a * X^2 + c");
    let xs: Vec<f64> = (-5..=5).map(|i| i as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| x * x + 1.0).collect();
    let mut ds = series_xy(0, "Quad", &xs, &ys);
    ds.unknowns_y.push((10, 5.0).into());
    let fit = fit_with(&quad, &[ds], &SolverConfig { interpolate_unknowns: true, ..cfg_fast() });
    assert_eq!(fit.interpolated_x.len(), 1);
    let u = &fit.interpolated_x[0];
    assert!((-5.0..=5.0).contains(&u.output_estimate) && u.note.as_deref().unwrap_or("").contains("Non-monotone"));
}
