//! Property-based tests (`proptest!`) for nonlinear parameter recovery, weighting scaling
//! invariance, AICc ranking, profile-likelihood CI bracketing, and ODE vs closed-form decay.

use curvebook_doc::{CiBound, FitMethod, ParameterRule, ParameterScope, WeightingScheme};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, FitStatus, HookContext, ParametricModel, SolverConfig, compare_models_aicc, fit_nonlinear,
    get_builtin_model,
};
use curvebook_math::ConfidenceLevel;
use proptest::prelude::*;

fn make_series(pts: Vec<DataPoint>) -> DatasetSeries {
    DatasetSeries {
        data_set_index: 0,
        label: "A".to_owned(),
        points: pts,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn recovers_4pl_parameters_and_profile_ci_brackets_mle(
        bottom in -10.0_f64..15.0,
        span in 25.0_f64..120.0,
        log_ec50 in -8.0_f64..-4.0,
        hill in 0.6_f64..2.2,
    ) {
        let top = bottom + span;
        let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
        let true_p = [bottom, top, log_ec50, hill];
        let xs: Vec<f64> = (-11..=-1).map(|v| v as f64).collect();
        let pts: Vec<DataPoint> = xs
            .iter()
            .enumerate()
            .map(|(row, &x)| DataPoint {
                row,
                ..DataPoint::xy(x, model.eval(&[x], &true_p, 0).unwrap())
            })
            .collect();

        let cfg = SolverConfig {
            compute_profile_ci: true,
            compute_hougaard: false,
            compute_bands: false,
            ..SolverConfig::default()
        };
        let fit = fit_nonlinear(model.as_ref(), &[make_series(pts)], &HookContext::default(), &cfg).unwrap();
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
        let mut pts1 = Vec::new();
        let mut pts2 = Vec::new();
        for (r, &x) in xs.iter().enumerate() {
            // Add slight deterministic perturbation so optimal weights still matter non-trivially.
            let y_clean = model.eval(&[x], &[vmax, km], 0).unwrap();
            let noise = if r % 2 == 0 { 0.3 } else { -0.3 };
            let y = (y_clean + noise).max(0.5);
            pts1.push(DataPoint { row: r, explicit_weight: Some(1.0), ..DataPoint::xy(x, y) });
            pts2.push(DataPoint { row: r, explicit_weight: Some(scale_c), ..DataPoint::xy(x, y) });
        }

        let cfg = SolverConfig {
            compute_profile_ci: false,
            compute_hougaard: false,
            compute_bands: false,
            ..SolverConfig::default()
        };
        let fit1 = fit_nonlinear(model.as_ref(), &[make_series(pts1)], &HookContext::default(), &cfg).unwrap();
        let fit2 = fit_nonlinear(model.as_ref(), &[make_series(pts2)], &HookContext::default(), &cfg).unwrap();

        for i in 0..2 {
            let e1 = fit1.parameters[i].estimate;
            let e2 = fit2.parameters[i].estimate;
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
        let ode = CompiledDslModel::compile("decay_ode", "Y0 = A0\ndY/dt = -k * Y").unwrap();
        let closed = get_builtin_model("exponential_growth").unwrap();
        let ts = [0.0, 0.5, 1.0, 2.0, 3.0, 5.0];
        let pts: Vec<DataPoint> = ts
            .iter()
            .enumerate()
            .map(|(row, &t)| DataPoint {
                row,
                ..DataPoint::xy(t, y0 * (-k * t).exp())
            })
            .collect();
        let rules = vec![
            ParameterRule {
                name: "A0".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(y0 * 0.9),
                lower_bound: Some(0.0),
                upper_bound: None,
            },
            ParameterRule {
                name: "k".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(k * 1.1),
                lower_bound: Some(0.0),
                upper_bound: None,
            },
        ];
        let cfg = SolverConfig {
            method: FitMethod::LeastSquares,
            weighting: WeightingScheme::None,
            parameter_rules: rules,
            confidence: ConfidenceLevel::DEFAULT_95,
            compute_profile_ci: false,
            compute_hougaard: false,
            compute_bands: false,
            ..SolverConfig::default()
        };
        let fit = fit_nonlinear(&ode, &[make_series(pts)], &HookContext::default(), &cfg).unwrap();
        let theta: Vec<f64> = fit.parameters.iter().map(|p| p.estimate).collect();
        let a0_hat = fit.parameters.iter().find(|p| p.name == "A0").unwrap().estimate;
        let k_hat = fit.parameters.iter().find(|p| p.name == "k").unwrap().estimate;
        prop_assert!((a0_hat - y0).abs() <= 1e-4 * y0);
        prop_assert!((k_hat - k).abs() <= 1e-4 * k);
        for &t in &ts {
            let y_ode = ode.eval(&[t], &theta, 0).unwrap();
            let y_closed = closed.eval(&[t], &[y0, -k], 0).unwrap();
            prop_assert!((y_ode - y_closed).abs() <= 1e-4 * y0);
        }
    }
}
