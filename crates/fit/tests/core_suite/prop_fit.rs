//! Property-based tests (`proptest!`) for nonlinear parameter recovery, weighting scaling
//! invariance, AICc ranking, profile-likelihood CI bracketing, and ODE vs closed-form decay.

use curvebook_doc::CiBound;
use curvebook_fit::{DataPoint, FitStatus, ParametricModel, SolverConfig, compare_models_aicc, get_builtin_model};
use proptest::prelude::*;

use crate::common::{cfg_fast, cfg_fast_rules, dsl_compile, fit_with, param_val, rule_local_bounds, series_eval, series_from_pts, xy_pts};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn recovers_4pl_parameters_and_profile_ci_brackets_mle(
        bottom in -10.0_f64..15.0,
        span in 25.0_f64..120.0,
        log_ec50 in -8.0_f64..-4.0,
        hill in 0.6_f64..2.2,
    ) {
        let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
        let true_p = [bottom, bottom + span, log_ec50, hill];
        let xs: Vec<f64> = (-11..=-1).map(|v| v as f64).collect();
        let cfg = SolverConfig { compute_profile_ci: true, ..cfg_fast() };
        let fit = fit_with(model.as_ref(), &[series_eval(model.as_ref(), 0, "A", &xs, &true_p)], &cfg);
        prop_assert_eq!(fit.status, FitStatus::Converged);
        for (i, &truth) in true_p.iter().enumerate() {
            let est = fit.parameters[i].estimate;
            prop_assert!((est - truth).abs() < 1e-4 * truth.abs().max(1.0));
            if let (CiBound::Finite(lo), CiBound::Finite(hi)) = (fit.parameters[i].ci_lower, fit.parameters[i].ci_upper) {
                prop_assert!(lo <= est + 1e-9 && est - 1e-9 <= hi);
            }
        }
    }

    #[test]
    fn weight_scaling_invariance_preserves_parameter_estimates(
        vmax in 20.0_f64..200.0,
        km in 1.0_f64..25.0,
        scale_c in 0.1_f64..50.0,
    ) {
        let model = get_builtin_model("michaelis_menten").unwrap();
        let xs = [0.5, 1.5, 4.0, 10.0, 25.0, 60.0, 120.0];
        let make_weighted = |w: f64| {
            let pts = xs.iter().enumerate().map(|(row, &x)| {
                let y = (model.eval(&[x], &[vmax, km], 0).unwrap() + if row % 2 == 0 { 0.3 } else { -0.3 }).max(0.5);
                DataPoint { row, explicit_weight: Some(w), ..DataPoint::xy(x, y) }
            }).collect();
            series_from_pts(0, "A", pts)
        };
        let cfg = cfg_fast();
        let fit1 = fit_with(model.as_ref(), &[make_weighted(1.0)], &cfg);
        let fit2 = fit_with(model.as_ref(), &[make_weighted(scale_c)], &cfg);
        for i in 0..2 {
            let (e1, e2) = (fit1.parameters[i].estimate, fit2.parameters[i].estimate);
            prop_assert!((e1 - e2).abs() <= 1e-5 * e1.abs().max(1.0));
        }
    }

    #[test]
    fn aicc_prefers_parsimonious_model_on_equal_rss(
        rss in 1.0_f64..100.0,
        n_obs in 12_usize..50,
    ) {
        let cmp = compare_models_aicc(rss, 2, rss, 4, n_obs).unwrap();
        prop_assert_eq!(cmp.preferred_index, 0);
        prop_assert!(cmp.weight_a > cmp.weight_b);
        prop_assert!((cmp.weight_a + cmp.weight_b - 1.0).abs() < 1e-12);
    }

    #[test]
    fn ode_linear_decay_matches_closed_form(
        y0 in 10.0_f64..250.0,
        k in 0.05_f64..1.5,
    ) {
        let ode = dsl_compile("Y0 = A0\ndY/dt = -k * Y");
        let closed = get_builtin_model("exponential_growth").unwrap();
        let ts = [0.0, 0.5, 1.0, 2.0, 3.0, 5.0];
        let ys: Vec<f64> = ts.iter().map(|&t| y0 * (-k * t).exp()).collect();
        let cfg = cfg_fast_rules(vec![
            rule_local_bounds("A0", Some(y0 * 0.9), Some(0.0), None),
            rule_local_bounds("k", Some(k * 1.1), Some(0.0), None),
        ]);
        let fit = fit_with(&ode, &[series_from_pts(0, "A", xy_pts(&ts, &ys))], &cfg);
        let theta: Vec<f64> = fit.parameters.iter().map(|p| p.estimate).collect();
        prop_assert!((param_val(&fit, "A0") - y0).abs() <= 1e-4 * y0);
        prop_assert!((param_val(&fit, "k") - k).abs() <= 1e-4 * k);
        for &t in &ts {
            prop_assert!((ode.eval(&[t], &theta, 0).unwrap() - closed.eval(&[t], &[y0, -k], 0).unwrap()).abs() <= 1e-4 * y0);
        }
    }
}
