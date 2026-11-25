//! Regression reproducers and D1 primary-literature oracle tests for R4.6 equation library fixes.

use curvebook_doc::{ParameterRule, ParameterScope};
use curvebook_fit::{
    DataPoint, DatasetSeries, EquationCategory, FitStatus, SolverConfig, format_model_equation_tex, get_builtin_model, list_builtin_equations,
};
use curvebook_testkit::{assert_approx_eq, log_relative_error};

use crate::common::{
    assert_grad_matches_fd, assert_params_approx, cfg_rules, dataset_param_val, fit_default, fit_with, rule_const, rule_local, rule_local_init,
    series_from_pts, series_xy, xy_pts,
};

#[test]
fn r46_1_beta_growth_holds_ym_after_te_and_rejects_te_le_tm() {
    let model = get_builtin_model("beta_growth_then_decay").unwrap();
    assert_eq!(model.param_names(), &["Ym", "te", "tm"]);
    let (ym, te, tm) = (85.0_f64, 14.0, 7.5);
    let params = [ym, te, tm];

    assert_approx_eq(model.eval(&[0.0], &params, 0).unwrap(), 0.0, 1e-12, 1e-12, "Y(0)");
    for t in [te, 14.5, 18.0, 24.0, 100.0] {
        assert_approx_eq(model.eval(&[t], &params, 0).unwrap(), ym, 1e-12, 1e-12, &format!("t={t}"));
    }
    for bad in [[ym, 8.0, 8.0], [ym, 6.0, 10.0], [ym, -2.0, -5.0]] {
        assert!(model.eval(&[5.0], &bad, 0).is_none());
    }

    let h = 1e-4_f64;
    let d2 = (model.eval(&[tm + h], &params, 0).unwrap() - 2.0 * model.eval(&[tm], &params, 0).unwrap() + model.eval(&[tm - h], &params, 0).unwrap())
        / (h * h);
    assert_approx_eq(d2, 0.0, 1e-6, 1e-6, "Y''(tm) == 0");

    let g = model.initial_guess(&[DataPoint::xy(0.1, 1.0), DataPoint::xy(0.4, 5.0)]);
    assert!(g[1] > 0.0 && g[2] >= 0.0 && g[2] < g[1]);

    let ts: Vec<f64> = (0..=24).map(|i| i as f64).collect();
    let ys: Vec<f64> = ts
        .iter()
        .map(|&t| {
            if t <= 0.0 {
                0.0
            } else if t >= te {
                ym
            } else {
                ym * (1.0 + (te - t) / (te - tm)) * (t / te).powf(te / (te - tm))
            }
        })
        .collect();
    let res = fit_default(model.as_ref(), &[series_xy(0, "Yin2003", &ts, &ys)]);
    assert_eq!(res.status, FitStatus::Converged);
    assert_params_approx(&res, 0, &[("Ym", ym, 1e-5, 1e-6), ("te", te, 1e-5, 1e-6), ("tm", tm, 1e-5, 1e-6)], "Yin2003");
}

#[test]
fn r46_2_biphasic_dose_response_sign_convention_and_direction_guess() {
    let model = get_builtin_model("biphasic_dose_response").unwrap();
    assert_eq!(
        (model.category(), model.param_names()),
        (EquationCategory::DoseResponseStimulation, &["Bottom", "Top", "Frac", "LogEC50_1", "LogEC50_2", "nH1", "nH2"].map(str::to_owned)[..])
    );

    let pos_p = [10.0, 110.0, 0.4, -7.0, -5.0, 1.2, 0.8];
    let vals: Vec<f64> = [-9.0, -7.0, -5.0, -3.0].into_iter().map(|x| model.eval(&[x], &pos_p, 0).unwrap()).collect();
    assert!(vals.windows(2).all(|w| w[0] < w[1]));
    assert_approx_eq(model.eval(&[-15.0], &pos_p, 0).unwrap(), 10.0, 1e-6, 1e-6, "low asymptote");
    assert_approx_eq(model.eval(&[3.0], &pos_p, 0).unwrap(), 110.0, 1e-6, 1e-6, "high asymptote");

    let mut xs: Vec<f64> = (-9..=-2).map(|k| k as f64).chain([-6.5, -5.5, -4.5, -3.5]).collect();
    xs.sort_by(f64::total_cmp);
    for (tag, frac, nh1, nh2, rising) in [("Rise", 0.4, 1.2, 0.8, true), ("Fall", 0.45, -1.3, -0.9, false)] {
        let ys: Vec<f64> = xs
            .iter()
            .map(|&x| 10.0 + 100.0 * (frac / (1.0 + 10.0_f64.powf((-7.0 - x) * nh1)) + (1.0 - frac) / (1.0 + 10.0_f64.powf((-5.0 - x) * nh2))))
            .collect();
        let g = model.initial_guess(&xy_pts(&xs, &ys));
        let (lo, hi) = (model.eval(&[xs[0]], &g, 0).unwrap(), model.eval(&[*xs.last().unwrap()], &g, 0).unwrap());
        if rising {
            assert!(g[5] > 0.0 && g[6] > 0.0 && lo < hi);
        } else {
            assert!(g[5] < 0.0 && g[6] < 0.0 && lo > hi);
        }
        let res = fit_default(model.as_ref(), &[series_xy(0, tag, &xs, &ys)]);
        assert_eq!(res.status, FitStatus::Converged);
        for (&yh, &yo) in res.datasets[0].y_hat.iter().zip(&ys) {
            assert_approx_eq(yh, yo, 1e-4, 1e-5, tag);
        }
    }
    assert!(format_model_equation_tex("biphasic_dose_response", None, &[], false, 4).contains("\\log\\text{EC}_{50,1} - X"));
}

#[test]
fn r46_3_centered_poly_free_xcenter_is_rank_deficient_and_default_rules_fix_sample_mean() {
    let xs: Vec<f64> = (1995..=2015).map(|v| v as f64).collect();
    let (true_center, true_coeffs) = (2005.0_f64, [12.5_f64, -1.8, 0.25, -0.02, 0.0015, -0.00008, 0.000003]);
    let ys_cubic: Vec<f64> = xs
        .iter()
        .map(|&x| {
            let dx = x - true_center;
            12.5 - 1.8 * dx + 0.25 * dx * dx - 0.02 * dx.powi(3)
        })
        .collect();
    let cubic = get_builtin_model("centered_poly_3").unwrap();
    let res_free = fit_with(cubic.as_ref(), &[series_xy(0, "CenteredCubic", &xs, &ys_cubic)], &cfg_rules(vec![rule_local("XCenter")]));
    assert_eq!(res_free.status, FitStatus::Ambiguous);
    assert!(res_free.covariance.numerical_rank < res_free.k_free);

    for deg in 1..=6 {
        let id = format!("centered_poly_{deg}");
        let model = get_builtin_model(&id).unwrap();
        let ys: Vec<f64> = xs.iter().map(|&x| (0..=deg).fold(0.0_f64, |acc, k| acc + true_coeffs[k] * (x - true_center).powi(k as i32))).collect();
        let ds = series_xy(0, &id, &xs, &ys);
        let rules = model.default_parameter_rules(&ds.points);
        assert_eq!((rules.len(), rules[0].name.as_str(), &rules[0].scope), (1, "XCenter", &ParameterScope::Constant(true_center)));

        let res = fit_with(model.as_ref(), &[ds], &cfg_rules(rules));
        assert_eq!(
            (res.status, res.k_free, res.covariance.numerical_rank, res.covariance.ambiguous),
            (FitStatus::Converged, deg + 1, deg + 1, false)
        );
        assert!(res.parameters.iter().all(|p| p.se.is_finite()));
        assert!(res.fixed_constants.iter().any(|(n, v, _)| n == "XCenter" && (*v - true_center).abs() < 1e-12));
        for (k, &expected) in true_coeffs.iter().enumerate().take(deg + 1) {
            assert_approx_eq(dataset_param_val(&res, 0, &format!("B{k}")), expected, 1e-5, 1e-5, &format!("{id} B{k}"));
        }
    }

    let skew_xs = [100.0, 101.0, 103.0, 108.0, 120.0, 140.0];
    let mean_x = skew_xs.iter().sum::<f64>() / skew_xs.len() as f64;
    let ys_skew: Vec<f64> = skew_xs.iter().map(|&x| 4.0 + 1.5 * (x - mean_x) - 0.05 * (x - mean_x).powi(2)).collect();
    let quad = get_builtin_model("centered_poly_2").unwrap();
    let ds_skew = series_xy(0, "SkewQuad", &skew_xs, &ys_skew);
    let skew_rules = quad.default_parameter_rules(&ds_skew.points);
    let Some(ParameterScope::Constant(xc)) = skew_rules.first().map(|r| &r.scope) else { panic!("expected Constant") };
    assert_approx_eq(*xc, mean_x, 1e-12, 1e-12, "skewed mean XCenter");
    let res_skew = fit_with(quad.as_ref(), &[ds_skew], &cfg_rules(skew_rules));
    assert_eq!(res_skew.status, FitStatus::Converged);
    assert_params_approx(&res_skew, 0, &[("B0", 4.0, 1e-6, 1e-6), ("B1", 1.5, 1e-6, 1e-6), ("B2", -0.05, 1e-6, 1e-6)], "skew");
}

#[test]
fn r46_4_absolute_ic50_sebaugh_2011_controls_midpoint() {
    let model = get_builtin_model("absolute_ic50_4pl").unwrap();
    assert_eq!(model.param_names(), &["Bottom", "Top", "LogIC50", "HillSlope", "Control0", "Control100"]);
    let (c0, c100, bot, top, hill, rel_ic50) = (20.0_f64, 100.0, 35.0, 95.0, 1.2, -6.0);
    let y50 = 0.5 * (c0 + c100);
    let true_abs = rel_ic50 + ((top - y50) / (y50 - bot)).log10() / hill;
    assert_approx_eq(model.eval(&[true_abs], &[bot, top, true_abs, hill, c0, c100], 0).unwrap(), y50, 1e-12, 1e-12, "Y(AbsIC50)");

    for bad in [[70.0, 95.0, -6.0, 1.2, c0, c100], [35.0, 55.0, -6.0, 1.2, c0, c100], [35.0, 95.0, -6.0, 0.0, c0, c100]] {
        assert!(model.eval(&[-6.0], &bad, 0).is_none());
    }

    let log_x = [-9.0, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0, -3.0];
    let ys: Vec<f64> = log_x.iter().map(|&x| bot + (top - bot) / (1.0 + 10.0_f64.powf((x - rel_ic50) * hill))).collect();
    let ds = series_xy(0, "Sebaugh2011", &log_x, &ys);
    let def_rules = model.default_parameter_rules(&ds.points);
    assert_eq!(def_rules.len(), 2);
    assert!(matches!(def_rules[0].scope, ParameterScope::Constant(_)) && def_rules[0].name == "Control0");
    assert!(matches!(def_rules[1].scope, ParameterScope::Constant(_)) && def_rules[1].name == "Control100");

    let res = fit_with(model.as_ref(), &[ds], &cfg_rules(vec![rule_const("Control0", c0), rule_const("Control100", c100)]));
    assert_eq!(res.status, FitStatus::Converged);
    assert_params_approx(
        &res,
        0,
        &[("Bottom", bot, 1e-6, 1e-6), ("Top", top, 1e-6, 1e-6), ("LogIC50", true_abs, 1e-6, 1e-6), ("HillSlope", hill, 1e-6, 1e-6)],
        "Sebaugh",
    );
    let tex = format_model_equation_tex("absolute_ic50_4pl", None, &[], false, 4);
    assert!(tex.contains("Control}_0") && tex.contains("Control}_{100}"));
}

#[test]
fn r46_5_power_law_rename_and_legacy_alias() {
    let catalog = list_builtin_equations();
    assert!(catalog.len() >= 117 && catalog.iter().any(|e| e.id == "power_law") && !catalog.iter().any(|e| e.id == "power_series"));
    for id in ["power_law", "power_series"] {
        let m = get_builtin_model(id).unwrap();
        assert_eq!((m.id(), m.param_names()), ("power_law", &["a".to_owned(), "b".to_owned()][..]));
    }
    assert_eq!(format_model_equation_tex("power_law", None, &[], false, 4), "Y = a \\cdot X^{b}");
    assert_eq!(format_model_equation_tex("power_series", None, &[("a".into(), 2.75), ("b".into(), 0.68)], true, 4), "Y = 2.75 \\cdot X^{0.68}");

    let xs: Vec<f64> = (1..=12).map(|i| i as f64 * 1.5).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| 2.75 * x.powf(0.68)).collect();
    let res = fit_default(get_builtin_model("power_law").unwrap().as_ref(), &[series_xy(0, "Allometry", &xs, &ys)]);
    assert_eq!(res.status, FitStatus::Converged);
    assert_params_approx(&res, 0, &[("a", 2.75, 1e-6, 1e-6), ("b", 0.68, 1e-6, 1e-6)], "power_law");
}

#[test]
fn r46_6_exponential_plateau_vs_one_phase_association_resolution() {
    let (assoc, plateau) = (get_builtin_model("one_phase_association").unwrap(), get_builtin_model("exponential_plateau").unwrap());
    assert_eq!((assoc.category(), assoc.param_names()), (EquationCategory::Exponential, &["Y0", "Plateau", "K"].map(str::to_owned)[..]));
    assert_eq!((plateau.category(), plateau.param_names()), (EquationCategory::Growth, &["YM", "Y0", "k"].map(str::to_owned)[..]));
    assert_grad_matches_fd(assoc.as_ref(), &[2.5], &[15.0, 85.0, 0.35], 0, 1e-6);
    assert_grad_matches_fd(plateau.as_ref(), &[2.5], &[85.0, 15.0, 0.35], 0, 1e-6);

    let ts: Vec<f64> = (0..=15).map(|i| i as f64 * 0.8).collect();
    let ys: Vec<f64> = ts.iter().map(|&t| 85.0 - 70.0 * (-0.35 * t).exp()).collect();
    let ds = [series_xy(0, "Monomolecular", &ts, &ys)];

    let (r_a, r_p) = (fit_default(assoc.as_ref(), &ds), fit_default(plateau.as_ref(), &ds));
    assert_eq!((r_a.status, r_p.status), (FitStatus::Converged, FitStatus::Converged));
    assert_params_approx(&r_a, 0, &[("Plateau", 85.0, 1e-6, 1e-6), ("Y0", 15.0, 1e-6, 1e-6), ("K", 0.35, 1e-6, 1e-6)], "assoc");
    assert_params_approx(&r_p, 0, &[("YM", 85.0, 1e-6, 1e-6), ("Y0", 15.0, 1e-6, 1e-6), ("k", 0.35, 1e-6, 1e-6)], "exp_plateau");
    assert!(!r_p.derived_scalars.is_empty() && !r_p.derived_scalars[0].is_empty());
}

/// Exact 128-row NIST StRD `Nelson` dataset (`x1`, `[[y; 4]; 4]` for `x2 in [180, 225, 250, 275]`).
const NELSON_TEMPS: [f64; 4] = [180.0, 225.0, 250.0, 275.0];
const NELSON_BLOCKS: &[(f64, [[f64; 4]; 4])] = &[
    (1.0, [[15.0, 17.0, 15.5, 16.5], [15.5, 15.0, 16.0, 14.5], [15.0, 14.5, 12.5, 11.0], [14.0, 13.0, 14.0, 11.5]]),
    (2.0, [[14.0, 16.0, 13.0, 13.5], [13.0, 13.5, 12.5, 12.5], [12.5, 12.0, 11.5, 12.0], [13.0, 11.5, 13.0, 12.5]]),
    (4.0, [[13.5, 17.5, 17.5, 13.5], [12.5, 12.5, 15.0, 13.0], [12.0, 13.0, 12.0, 13.5], [10.0, 11.5, 11.0, 9.5]]),
    (8.0, [[15.0, 15.0, 15.5, 16.0], [13.0, 10.5, 13.5, 14.0], [12.5, 12.0, 11.5, 11.5], [6.5, 5.5, 6.0, 6.0]]),
    (16.0, [[18.5, 17.0, 15.3, 16.0], [13.0, 14.0, 12.5, 11.0], [12.0, 12.0, 11.5, 12.0], [6.0, 6.0, 5.0, 5.5]]),
    (32.0, [[12.5, 13.0, 16.0, 12.0], [11.0, 9.5, 11.0, 11.0], [11.0, 10.0, 10.5, 10.5], [2.7, 2.7, 2.5, 2.4]]),
    (48.0, [[13.0, 13.5, 16.5, 13.6], [11.5, 10.5, 13.5, 12.0], [7.0, 6.9, 8.8, 7.9], [1.2, 1.5, 1.0, 1.5]]),
    (64.0, [[13.0, 12.5, 16.5, 16.0], [11.0, 11.5, 10.5, 10.0], [7.27, 7.5, 6.7, 7.6], [1.5, 1.0, 1.2, 1.2]]),
];

#[test]
fn r46_7_nist_nelson_fits_log_y_oracle() {
    let model = get_builtin_model("nist_nelson").unwrap();
    let cert_betas = [2.5906836021_f64, 5.6177717026e-9, -5.7701013174e-2];
    let cert_ses = [1.9149996413e-2_f64, 6.1124096540e-9, 3.9572366543e-3];
    let (cert_rss, cert_rsd) = (3.7976833176_f64, 1.7430280130e-1_f64);
    assert_grad_matches_fd(model.as_ref(), &[16.0, 250.0], &cert_betas, 0, 1e-6);

    let build_series = |label: &str, use_log: bool| -> DatasetSeries {
        let mut pts = Vec::with_capacity(128);
        for &(x1, ref temp_rows) in NELSON_BLOCKS {
            for (&x2, ys) in NELSON_TEMPS.iter().zip(temp_rows) {
                for &y in ys {
                    pts.push(DataPoint { x: vec![x1, x2], y: if use_log { y.ln() } else { y }, row: pts.len(), ..DataPoint::xy(0.0, 0.0) });
                }
            }
        }
        assert_eq!(pts.len(), 128);
        series_from_pts(0, label, pts)
    };

    let ds_log = build_series("Nelson_log_y", true);
    for (tag, starts) in [("start1", Some([2.0, 1e-4, -0.01])), ("start2", Some([2.5, 5e-9, -0.05])), ("default_init", None)] {
        let rules: Vec<ParameterRule> =
            starts.map(|vals| model.param_names().iter().zip(vals).map(|(n, v)| rule_local_init(n, v)).collect()).unwrap_or_default();
        let res = fit_with(
            model.as_ref(),
            std::slice::from_ref(&ds_log),
            &SolverConfig { parameter_rules: rules, compute_profile_ci: false, ..Default::default() },
        );
        assert_eq!((res.status, res.df), (FitStatus::Converged, 125), "Nelson {tag}");
        for i in 0..3 {
            assert!(log_relative_error(res.parameters[i].estimate, cert_betas[i]) >= 7.0, "{tag} b{}", i + 1);
            assert!(log_relative_error(res.parameters[i].se, cert_ses[i]) >= 5.0, "{tag} SE{}", i + 1);
        }
        assert!(log_relative_error(res.rss, cert_rss) >= 7.5 && log_relative_error(res.rsd, cert_rsd) >= 7.5);
    }

    let rules_raw: Vec<ParameterRule> = model.param_names().iter().zip([15.0, 1e-4, -0.01]).map(|(n, v)| rule_local_init(n, v)).collect();
    let res_raw = fit_with(
        model.as_ref(),
        &[build_series("Nelson_raw_y", false)],
        &SolverConfig { parameter_rules: rules_raw, compute_profile_ci: false, ..Default::default() },
    );
    assert!((res_raw.rss - cert_rss).abs() > 100.0);
}

#[test]
fn r46_8_logistic_growth_and_operational_model_oracles() {
    let logistic = get_builtin_model("logistic_growth").unwrap();
    let (ym, y0, k) = (250.0, 10.0, 0.35);
    let ts: Vec<f64> = (0..=20).map(|i| i as f64).collect();
    let ys_log: Vec<f64> = ts.iter().map(|&t| ym * y0 / (y0 + (ym - y0) * (-k * t).exp())).collect();
    let res_log = fit_default(logistic.as_ref(), &[series_xy(0, "Logistic", &ts, &ys_log)]);
    assert_eq!(res_log.status, FitStatus::Converged);
    assert_params_approx(&res_log, 0, &[("YM", ym, 1e-5, 1e-6), ("Y0", y0, 1e-5, 1e-6), ("k", k, 1e-5, 1e-6)], "logistic");

    let op = get_builtin_model("operational_model_agonist").unwrap();
    let (basal, emax, log_ka, log_tau, n) = (5.0_f64, 95.0, -6.0, 0.8, 1.3);
    let (ka, tau) = (10.0_f64.powf(log_ka), 10.0_f64.powf(log_tau));
    let log_xs: Vec<f64> = (-10..=-3).map(|i| i as f64).collect();
    let ys_op: Vec<f64> = log_xs
        .iter()
        .map(|&lx| {
            let a = 10.0_f64.powf(lx);
            let num = (tau * a).powf(n);
            let exp = basal + (emax - basal) * num / (num + (a + ka).powf(n));
            assert_approx_eq(op.eval(&[lx], &[basal, emax, log_ka, log_tau, n], 0).unwrap(), exp, 1e-12, 1e-12, &format!("op logA={lx}"));
            exp
        })
        .collect();
    let res_op = fit_with(
        op.as_ref(),
        &[series_xy(0, "Operational", &log_xs, &ys_op)],
        &cfg_rules(vec![rule_const("Basal", basal), rule_const("Emax", emax)]),
    );
    assert_eq!(res_op.status, FitStatus::Converged);
    assert_params_approx(&res_op, 0, &[("LogKA", log_ka, 1e-5, 1e-5), ("LogTau", log_tau, 1e-5, 1e-5), ("n", n, 1e-5, 1e-5)], "op");
}
