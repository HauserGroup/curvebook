//! Regression reproducers and D1 primary-literature oracle tests for R4.6 equation library fixes.

use curvebook_doc::{ParameterRule, ParameterScope};
use curvebook_fit::{
    DataPoint, DatasetSeries, EquationCategory, FitStatus, HookContext, NonlinearFitResult, ParametricModel, SolverConfig, fit_nonlinear,
    format_model_equation_tex, get_builtin_model, list_builtin_equations,
};

fn series(label: &str, xs: Vec<f64>, ys: Vec<f64>) -> DatasetSeries {
    let points = xs
        .into_iter()
        .zip(ys)
        .enumerate()
        .map(|(row, (x, y))| {
            let mut pt = DataPoint::xy(x, y);
            pt.row = row;
            pt
        })
        .collect();
    DatasetSeries {
        data_set_index: 0,
        label: label.to_owned(),
        points,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn rule_local(name: &str, initial: Option<f64>, lower: Option<f64>, upper: Option<f64>) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Local, initial_override: initial, lower_bound: lower, upper_bound: upper }
}

fn rule_const(name: &str, value: f64) -> ParameterRule {
    ParameterRule {
        name: name.to_owned(),
        scope: ParameterScope::Constant(value),
        initial_override: Some(value),
        lower_bound: None,
        upper_bound: None,
    }
}

fn assert_approx_eq(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, ctx: &str) {
    let diff = (actual - expected).abs();
    let tol = abs_tol.max(rel_tol * expected.abs());
    assert!(diff <= tol, "{ctx}: actual={actual:.12e}, expected={expected:.12e}, diff={diff:.3e}, tol={tol:.3e}");
}

fn log_relative_error(actual: f64, certified: f64) -> f64 {
    if actual == certified {
        return 16.0;
    }
    let rel = if certified == 0.0 { (actual - certified).abs() } else { (actual - certified).abs() / certified.abs() };
    if rel <= 1e-16 { 16.0 } else { (-rel.log10()).clamp(0.0, 16.0) }
}

fn find_ds_param(res: &NonlinearFitResult, ds: usize, name: &str) -> f64 {
    let dataset_opt = res.datasets.get(ds);
    assert!(dataset_opt.is_some(), "missing dataset {ds}");
    let Some(dataset) = dataset_opt else {
        return f64::NAN;
    };
    let idx_opt = dataset.param_names.iter().position(|p| p.eq_ignore_ascii_case(name));
    assert!(idx_opt.is_some(), "missing parameter `{name}` in {:?}", dataset.param_names);
    let Some(idx) = idx_opt else {
        return f64::NAN;
    };
    dataset.params[idx]
}

fn assert_grad_matches_fd(model: &dyn ParametricModel, x: &[f64], params: &[f64], ds_idx: usize, abs_tol: f64, rel_tol: f64, ctx: &str) {
    let p_count = model.param_count();
    let mut analytic = vec![0.0_f64; p_count];
    assert!(model.grad(x, params, ds_idx, &mut analytic), "{ctx}: analytical grad returned false");
    let mut p_plus = params.to_vec();
    let mut p_minus = params.to_vec();
    for idx in 0..p_count {
        let h = 1e-6 * params[idx].abs().max(1.0);
        p_plus[idx] = params[idx] + h;
        p_minus[idx] = params[idx] - h;
        let yp_opt = model.eval(x, &p_plus, ds_idx);
        let ym_opt = model.eval(x, &p_minus, ds_idx);
        p_plus[idx] = params[idx];
        p_minus[idx] = params[idx];
        assert!(yp_opt.is_some() && ym_opt.is_some(), "{ctx}: finite-difference eval None at param[{idx}]");
        if let (Some(yp), Some(ym)) = (yp_opt, ym_opt) {
            let fd = (yp - ym) / (2.0 * h);
            assert_approx_eq(analytic[idx], fd, abs_tol, rel_tol, &format!("{ctx} d/dp[{idx}] ({})", model.param_names()[idx]));
        }
    }
}

/// Task 1 (`beta_growth_then_decay`) — Yin, Goudriaan, Lantinga, Vos & Spiertz (2003),
/// *Annals of Botany* 91(3):361–371, Eq. 5–6 (`tb = 0`, `wb = 0`):
/// holds `Ym` for `t > te` and rejects invalid domain `te <= tm`.
#[test]
fn r46_1_beta_growth_holds_ym_after_te_and_rejects_te_le_tm() {
    let model = get_builtin_model("beta_growth_then_decay").expect("missing beta_growth_then_decay");
    assert_eq!(model.param_names(), &["Ym", "te", "tm"]);

    let (ym, te, tm) = (85.0_f64, 14.0_f64, 7.5_f64);
    let params = [ym, te, tm];

    // Before emergence `t <= 0` => 0; at end of growth `t == te` => Ym; beyond `t > te` => holds Ym.
    assert_approx_eq(model.eval(&[0.0], &params, 0).expect("t=0"), 0.0, 1e-12, 1e-12, "Yin (2003) Y(0)");
    assert_approx_eq(model.eval(&[te], &params, 0).expect("t=te"), ym, 1e-12, 1e-12, "Yin (2003) Y(te) == Ym");
    for &t_post in &[14.5, 18.0, 24.0, 100.0] {
        let val = model.eval(&[t_post], &params, 0).expect("t > te must evaluate cleanly");
        assert_approx_eq(val, ym, 1e-12, 1e-12, &format!("Yin (2003) holds Ym at t={t_post} > te"));
    }

    // Domain constraint in Yin et al. (2003): `0 <= tm < te` (`te <= tm` or `te <= 0` must reject).
    assert!(model.eval(&[5.0], &[ym, 8.0, 8.0], 0).is_none(), "te == tm must return None");
    assert!(model.eval(&[5.0], &[ym, 6.0, 10.0], 0).is_none(), "te < tm must return None");
    assert!(model.eval(&[5.0], &[ym, -2.0, -5.0], 0).is_none(), "te <= 0 must return None");

    // Inflection point at t == tm: second derivative equals zero (Yin et al. 2003 Eq. 4-5).
    let h = 1e-4_f64;
    let y_m = model.eval(&[tm - h], &params, 0).expect("tm-h");
    let y_0 = model.eval(&[tm], &params, 0).expect("tm");
    let y_p = model.eval(&[tm + h], &params, 0).expect("tm+h");
    let d2 = (y_p - 2.0 * y_0 + y_m) / (h * h);
    assert_approx_eq(d2, 0.0, 1e-6, 1e-6, "Yin (2003) inflection Y''(tm) == 0");

    // Initial guess on small sub-unit ranges must satisfy `te > 0` and `0 <= tm < te`.
    let small_pts = vec![DataPoint::xy(0.1, 1.0), DataPoint::xy(0.4, 5.0)];
    let g_small = model.initial_guess(&small_pts);
    assert!(g_small[1] > 0.0 && g_small[2] >= 0.0 && g_small[2] < g_small[1], "invalid initial_guess: {g_small:?}");

    // Nonlinear fit across both the growth window [0, te] and the plateau (te, 24].
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
    let res = fit_nonlinear(model.as_ref(), &[series("Yin2003", ts, ys)], &HookContext::default(), &SolverConfig::default())
        .expect("beta_growth_then_decay fit");
    assert_eq!(res.status, FitStatus::Converged, "expected Converged, got {:?}", res.status);
    assert_approx_eq(find_ds_param(&res, 0, "Ym"), ym, 1e-5, 1e-6, "fitted Ym");
    assert_approx_eq(find_ds_param(&res, 0, "te"), te, 1e-5, 1e-6, "fitted te");
    assert_approx_eq(find_ds_param(&res, 0, "tm"), tm, 1e-5, 1e-6, "fitted tm");
}

/// Task 2 (`biphasic_dose_response`) — Two-site Hill (1910) composite following the library's
/// agonist Hill-slope convention (Motulsky & Christopoulos 2004): positive `nH` rises with `X`,
/// and `initial_guess` aligns with observed curve direction (rising vs falling).
#[test]
fn r46_2_biphasic_dose_response_sign_convention_and_direction_guess() {
    let model = get_builtin_model("biphasic_dose_response").expect("missing biphasic_dose_response");
    assert_eq!(model.category(), EquationCategory::DoseResponseStimulation);
    assert_eq!(model.param_names(), &["Bottom", "Top", "Frac", "LogEC50_1", "LogEC50_2", "nH1", "nH2"]);

    // Positive nH1, nH2 with Top > Bottom must produce a monotonically rising curve with X.
    let pos_params = [10.0, 110.0, 0.4, -7.0, -5.0, 1.2, 0.8];
    let y_neg9 = model.eval(&[-9.0], &pos_params, 0).expect("eval -9");
    let y_neg7 = model.eval(&[-7.0], &pos_params, 0).expect("eval -7");
    let y_neg5 = model.eval(&[-5.0], &pos_params, 0).expect("eval -5");
    let y_neg3 = model.eval(&[-3.0], &pos_params, 0).expect("eval -3");
    assert!(y_neg9 < y_neg7 && y_neg7 < y_neg5 && y_neg5 < y_neg3, "positive nH must rise: {y_neg9}, {y_neg7}, {y_neg5}, {y_neg3}");
    assert_approx_eq(model.eval(&[-15.0], &pos_params, 0).expect("-15"), 10.0, 1e-6, 1e-6, "low asymptote == Bottom");
    assert_approx_eq(model.eval(&[3.0], &pos_params, 0).expect("+3"), 110.0, 1e-6, 1e-6, "high asymptote == Top");

    let xs: Vec<f64> = (-9..=-2).map(|k| k as f64).chain([-6.5, -5.5, -4.5, -3.5]).collect();
    let mut xs_sorted = xs;
    xs_sorted.sort_by(f64::total_cmp);

    // Rising curve oracle & automatic initial_guess alignment.
    let ys_rise: Vec<f64> = xs_sorted
        .iter()
        .map(|&x| {
            let s1 = 1.0 / (1.0 + 10.0_f64.powf((-7.0 - x) * 1.2));
            let s2 = 1.0 / (1.0 + 10.0_f64.powf((-5.0 - x) * 0.8));
            10.0 + 100.0 * (0.4 * s1 + 0.6 * s2)
        })
        .collect();
    let pts_rise: Vec<DataPoint> = xs_sorted.iter().zip(&ys_rise).map(|(&x, &y)| DataPoint::xy(x, y)).collect();
    let g_rise = model.initial_guess(&pts_rise);
    assert!(g_rise[5] > 0.0 && g_rise[6] > 0.0, "rising initial_guess must have positive slopes: {g_rise:?}");
    let g_rise_lo = model.eval(&[xs_sorted[0]], &g_rise, 0).expect("g_rise lo");
    let g_rise_hi = model.eval(&[xs_sorted[xs_sorted.len() - 1]], &g_rise, 0).expect("g_rise hi");
    assert!(g_rise_lo < g_rise_hi, "rising initial_guess curve must rise: {g_rise_lo} < {g_rise_hi}");

    let res_rise = fit_nonlinear(
        model.as_ref(),
        &[series("BiphasicRise", xs_sorted.clone(), ys_rise.clone())],
        &HookContext::default(),
        &SolverConfig::default(),
    )
    .expect("rising biphasic fit");
    assert_eq!(res_rise.status, FitStatus::Converged);
    for (yh, yo) in res_rise.datasets[0].y_hat.iter().zip(&ys_rise) {
        assert_approx_eq(*yh, *yo, 1e-4, 1e-5, "rising biphasic y_hat");
    }

    // Falling curve oracle & automatic initial_guess alignment (`nH1, nH2 < 0`).
    let ys_fall: Vec<f64> = xs_sorted
        .iter()
        .map(|&x| {
            let s1 = 1.0 / (1.0 + 10.0_f64.powf((-7.0 - x) * (-1.3)));
            let s2 = 1.0 / (1.0 + 10.0_f64.powf((-5.0 - x) * (-0.9)));
            10.0 + 100.0 * (0.45 * s1 + 0.55 * s2)
        })
        .collect();
    let pts_fall: Vec<DataPoint> = xs_sorted.iter().zip(&ys_fall).map(|(&x, &y)| DataPoint::xy(x, y)).collect();
    let g_fall = model.initial_guess(&pts_fall);
    assert!(g_fall[5] < 0.0 && g_fall[6] < 0.0, "falling initial_guess must have negative slopes: {g_fall:?}");
    let g_fall_lo = model.eval(&[xs_sorted[0]], &g_fall, 0).expect("g_fall lo");
    let g_fall_hi = model.eval(&[xs_sorted[xs_sorted.len() - 1]], &g_fall, 0).expect("g_fall hi");
    assert!(g_fall_lo > g_fall_hi, "falling initial_guess curve must fall: {g_fall_lo} > {g_fall_hi}");

    let res_fall =
        fit_nonlinear(model.as_ref(), &[series("BiphasicFall", xs_sorted, ys_fall.clone())], &HookContext::default(), &SolverConfig::default())
            .expect("falling biphasic fit");
    assert_eq!(res_fall.status, FitStatus::Converged);
    for (yh, yo) in res_fall.datasets[0].y_hat.iter().zip(&ys_fall) {
        assert_approx_eq(*yh, *yo, 1e-4, 1e-5, "falling biphasic y_hat");
    }

    let tex = format_model_equation_tex("biphasic_dose_response", None, &[], false, 4);
    assert!(tex.contains("\\log\\text{EC}_{50,1} - X"), "expected agonist sign in TeX, got: {tex}");
}

/// Task 3 (`centered_poly_*`) — Centred polynomial regression, Draper & Smith (1998):
/// leaving `XCenter` free produces an over-parameterized rank-deficient Jacobian (`Ambiguous`),
/// while `default_parameter_rules` constrains `XCenter` to the sample mean so orders `1..=6`
/// have full rank (`Converged`) and finite standard errors.
#[test]
fn r46_3_centered_poly_free_xcenter_is_rank_deficient_and_default_rules_fix_sample_mean() {
    let xs: Vec<f64> = (1995..=2015).map(|v| v as f64).collect();
    let true_center = 2005.0_f64;
    let true_coeffs = [12.5_f64, -1.8, 0.25, -0.02, 0.0015, -0.00008, 0.000003];
    let ys_cubic: Vec<f64> = xs
        .iter()
        .map(|&x| {
            let dx = x - true_center;
            12.5 - 1.8 * dx + 0.25 * dx * dx - 0.02 * dx.powi(3)
        })
        .collect();
    let ds_cubic = series("CenteredCubic", xs.clone(), ys_cubic);
    let cubic = get_builtin_model("centered_poly_3").expect("centered_poly_3");

    // Explicit Local rule overrides `CenteredPolynomial::default_parameter_rules` so leaving `XCenter` free creates an exact linear dependency in J -> Ambiguous.
    let free_xcenter_cfg = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: "XCenter".to_owned(),
            scope: ParameterScope::Local,
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let res_free =
        fit_nonlinear(cubic.as_ref(), std::slice::from_ref(&ds_cubic), &HookContext::default(), &free_xcenter_cfg).expect("free XCenter run");
    assert_eq!(res_free.status, FitStatus::Ambiguous);
    assert!(res_free.covariance.numerical_rank < res_free.k_free, "free XCenter Jacobian must be rank-deficient");

    // Oracle across orders 1..=6: `default_parameter_rules` fixes `XCenter` as Constant(mean_x).
    for deg in 1..=6 {
        let id = format!("centered_poly_{deg}");
        let model = get_builtin_model(&id).expect("centered_poly");
        let ys: Vec<f64> = xs.iter().map(|&x| (0..=deg).fold(0.0_f64, |acc, k| acc + true_coeffs[k] * (x - true_center).powi(k as i32))).collect();
        let ds = series(&id, xs.clone(), ys);
        let default_rules = model.default_parameter_rules(&ds.points);
        assert_eq!(default_rules.len(), 1, "{id} expected 1 default parameter rule");
        assert_eq!(default_rules[0].name, "XCenter");
        assert_eq!(default_rules[0].scope, ParameterScope::Constant(true_center));

        let cfg = SolverConfig { parameter_rules: default_rules, ..SolverConfig::default() };
        let res = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg).expect("centered fit");
        assert_eq!(res.status, FitStatus::Converged, "{id} must converge cleanly with default XCenter rule");
        assert_eq!(res.k_free, deg + 1);
        assert_eq!(res.covariance.numerical_rank, deg + 1);
        assert!(!res.covariance.ambiguous, "{id} must not be ambiguous");
        assert!(res.parameters.iter().all(|p| p.se.is_finite()), "{id} must produce finite SEs");
        assert!(
            res.fixed_constants.iter().any(|(n, v, _)| n == "XCenter" && (*v - true_center).abs() < 1e-12),
            "{id} expected fixed constant XCenter == {true_center}"
        );
        for (k, &expected) in true_coeffs.iter().enumerate().take(deg + 1) {
            assert_approx_eq(find_ds_param(&res, 0, &format!("B{k}")), expected, 1e-5, 1e-5, &format!("{id} B{k}"));
        }
    }

    // Skewed (non-symmetric) predictors: default XCenter must equal the arithmetic sample mean, not median.
    let skew_xs = vec![100.0, 101.0, 103.0, 108.0, 120.0, 140.0];
    let mean_x = skew_xs.iter().sum::<f64>() / skew_xs.len() as f64;
    let ys_skew: Vec<f64> = skew_xs.iter().map(|&x| 4.0 + 1.5 * (x - mean_x) - 0.05 * (x - mean_x).powi(2)).collect();
    let quad = get_builtin_model("centered_poly_2").expect("centered_poly_2");
    let ds_skew = series("SkewQuad", skew_xs, ys_skew);
    let skew_rules = quad.default_parameter_rules(&ds_skew.points);
    match skew_rules.first().map(|r| &r.scope) {
        Some(ParameterScope::Constant(xc)) => assert_approx_eq(*xc, mean_x, 1e-12, 1e-12, "skewed sample-mean XCenter"),
        other => panic!("expected ParameterScope::Constant(mean_x), got {other:?}"),
    }
    let res_skew =
        fit_nonlinear(quad.as_ref(), &[ds_skew], &HookContext::default(), &SolverConfig { parameter_rules: skew_rules, ..SolverConfig::default() })
            .expect("skew quad fit");
    assert_eq!(res_skew.status, FitStatus::Converged);
    assert_approx_eq(find_ds_param(&res_skew, 0, "B0"), 4.0, 1e-6, 1e-6, "skew B0");
    assert_approx_eq(find_ds_param(&res_skew, 0, "B1"), 1.5, 1e-6, 1e-6, "skew B1");
    assert_approx_eq(find_ds_param(&res_skew, 0, "B2"), -0.05, 1e-6, 1e-6, "skew B2");
}

/// Task 4 (`absolute_ic50_4pl`) — Sebaugh (2011), "Guidelines for accurate EC50/IC50 estimation",
/// *Pharmaceutical Statistics* 10(2):128–134; Hill (1910):
/// 50% response target is the mean `(Control0 + Control100) / 2` of the 0% and 100% controls,
/// NOT `Baseline / 2`.
#[test]
fn r46_4_absolute_ic50_sebaugh_2011_controls_midpoint() {
    let model = get_builtin_model("absolute_ic50_4pl").expect("missing absolute_ic50_4pl");
    assert_eq!(model.param_names(), &["Bottom", "Top", "LogIC50", "HillSlope", "Control0", "Control100"]);

    // Non-zero blank/0% control = 20.0, 100% vehicle control = 100.0 => target Y50 = (20 + 100)/2 = 60.0
    // (NOT 100/2 = 50.0!). Curve exhibits partial inhibition: Bottom = 35.0, Top = 95.0, LogRelIC50 = -6.0, Hill = 1.2.
    let control0 = 20.0_f64;
    let control100 = 100.0_f64;
    let y50_true = 0.5 * (control0 + control100);
    let bottom = 35.0_f64;
    let top = 95.0_f64;
    let hill = 1.2_f64;
    let log_rel_ic50 = -6.0_f64;
    let true_log_abs_ic50 = log_rel_ic50 + ((top - y50_true) / (y50_true - bottom)).log10() / hill;

    // Evaluating at X = LogAbsIC50 must return exact 0.5 * (Control0 + Control100) == 60.0.
    let y_at_abs = model.eval(&[true_log_abs_ic50], &[bottom, top, true_log_abs_ic50, hill, control0, control100], 0).expect("eval at LogAbsIC50");
    assert_approx_eq(y_at_abs, y50_true, 1e-12, 1e-12, "Sebaugh (2011) Y(AbsIC50) == mean(Control0, Control100)");

    // Domain guards: Top must exceed midpoint, Bottom must be below midpoint, HillSlope != 0.
    assert!(model.eval(&[-6.0], &[70.0, 95.0, -6.0, 1.2, control0, control100], 0).is_none());
    assert!(model.eval(&[-6.0], &[35.0, 55.0, -6.0, 1.2, control0, control100], 0).is_none());
    assert!(model.eval(&[-6.0], &[35.0, 95.0, -6.0, 0.0, control0, control100], 0).is_none());

    let log_x: Vec<f64> = vec![-9.0, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0, -3.0];
    let ys: Vec<f64> = log_x.iter().map(|&x| bottom + (top - bottom) / (1.0 + 10.0_f64.powf((x - log_rel_ic50) * hill))).collect();
    let ds = series("Sebaugh2011", log_x, ys);
    let default_rules = model.default_parameter_rules(&ds.points);
    assert_eq!(default_rules.len(), 2);
    assert!(matches!(default_rules[0].scope, ParameterScope::Constant(_)) && default_rules[0].name == "Control0");
    assert!(matches!(default_rules[1].scope, ParameterScope::Constant(_)) && default_rules[1].name == "Control100");

    let cfg =
        SolverConfig { parameter_rules: vec![rule_const("Control0", control0), rule_const("Control100", control100)], ..SolverConfig::default() };
    let res = fit_nonlinear(model.as_ref(), &[ds], &HookContext::default(), &cfg).expect("absolute_ic50_4pl fit");
    assert_eq!(res.status, FitStatus::Converged);
    assert_approx_eq(find_ds_param(&res, 0, "Bottom"), bottom, 1e-6, 1e-6, "Sebaugh Bottom");
    assert_approx_eq(find_ds_param(&res, 0, "Top"), top, 1e-6, 1e-6, "Sebaugh Top");
    assert_approx_eq(find_ds_param(&res, 0, "LogIC50"), true_log_abs_ic50, 1e-6, 1e-6, "Sebaugh LogAbsIC50");
    assert_approx_eq(find_ds_param(&res, 0, "HillSlope"), hill, 1e-6, 1e-6, "Sebaugh HillSlope");

    let tex = format_model_equation_tex("absolute_ic50_4pl", None, &[], false, 4);
    assert!(tex.contains("Control}_0") && tex.contains("Control}_{100}"), "TeX must show both controls: {tex}");
}

/// Task 5 (Rename `power_series` -> `power_law`, Huxley 1932 allometric power law `Y = a * X^b`),
/// keeping `"power_series"` backwards-compatible ID alias and preserving >= 117 catalog count.
#[test]
fn r46_5_power_law_rename_and_legacy_alias() {
    let catalog = list_builtin_equations();
    assert!(catalog.len() >= 117, "expected >= 117 built-ins, got {}", catalog.len());
    assert!(catalog.iter().any(|e| e.id == "power_law"), "catalog must expose canonical `power_law`");
    assert!(!catalog.iter().any(|e| e.id == "power_series"), "catalog must not expose legacy `power_series` primary ID");

    let canon = get_builtin_model("power_law").expect("power_law model");
    assert_eq!(canon.id(), "power_law");
    assert_eq!(canon.param_names(), &["a", "b"]);

    // Legacy alias must still resolve cleanly for persisted project files.
    let legacy = get_builtin_model("power_series").expect("legacy power_series alias must still resolve");
    assert_eq!(legacy.id(), "power_law");
    assert_eq!(legacy.param_names(), &["a", "b"]);

    // Substituted and symbolic TeX must emit `Y = a \cdot X^{b}` with no spurious `+ c`.
    let sym = format_model_equation_tex("power_law", None, &[], false, 4);
    assert_eq!(sym, "Y = a \\cdot X^{b}");
    let sub_legacy = format_model_equation_tex("power_series", None, &[("a".into(), 2.75), ("b".into(), 0.68)], true, 4);
    assert_eq!(sub_legacy, "Y = 2.75 \\cdot X^{0.68}");

    // Huxley (1932) allometric fit oracle `Y = 2.75 * X^0.68`.
    let xs: Vec<f64> = (1..=12).map(|i| i as f64 * 1.5).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| 2.75 * x.powf(0.68)).collect();
    let res =
        fit_nonlinear(canon.as_ref(), &[series("Allometry", xs, ys)], &HookContext::default(), &SolverConfig::default()).expect("power_law fit");
    assert_eq!(res.status, FitStatus::Converged);
    assert_approx_eq(find_ds_param(&res, 0, "a"), 2.75, 1e-6, 1e-6, "power_law a");
    assert_approx_eq(find_ds_param(&res, 0, "b"), 0.68, 1e-6, 1e-6, "power_law b");
}

/// Task 6 (Resolve `exponential_plateau` vs `one_phase_association`):
/// `one_phase_association` (`Exponential`, Motulsky & Christopoulos 2004, `(Y0, Plateau, K)`)
/// vs `exponential_plateau` (`Growth`, Seber & Wild 1989 ch. 7 monomolecular growth, `(YM, Y0, k)`),
/// both with analytical gradients and matching fitted predictions.
#[test]
fn r46_6_exponential_plateau_vs_one_phase_association_resolution() {
    let assoc = get_builtin_model("one_phase_association").expect("one_phase_association");
    let plateau = get_builtin_model("exponential_plateau").expect("exponential_plateau");

    assert_eq!(assoc.category(), EquationCategory::Exponential);
    assert_eq!(assoc.param_names(), &["Y0", "Plateau", "K"]);
    assert_eq!(plateau.category(), EquationCategory::Growth);
    assert_eq!(plateau.param_names(), &["YM", "Y0", "k"]);

    assert_grad_matches_fd(assoc.as_ref(), &[2.5], &[15.0, 85.0, 0.35], 0, 1e-6, 1e-6, "one_phase_association");
    assert_grad_matches_fd(plateau.as_ref(), &[2.5], &[85.0, 15.0, 0.35], 0, 1e-6, 1e-6, "exponential_plateau");

    let ts: Vec<f64> = (0..=15).map(|i| i as f64 * 0.8).collect();
    let ys: Vec<f64> = ts.iter().map(|&t| 85.0 - 70.0 * (-0.35 * t).exp()).collect();
    let ds = series("Monomolecular", ts, ys);

    let r_assoc = fit_nonlinear(assoc.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &SolverConfig::default()).expect("assoc fit");
    let r_plat =
        fit_nonlinear(plateau.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &SolverConfig::default()).expect("exp_plateau fit");

    assert_eq!(r_assoc.status, FitStatus::Converged);
    assert_eq!(r_plat.status, FitStatus::Converged);
    assert_approx_eq(find_ds_param(&r_assoc, 0, "Plateau"), 85.0, 1e-6, 1e-6, "assoc Plateau");
    assert_approx_eq(find_ds_param(&r_plat, 0, "YM"), 85.0, 1e-6, 1e-6, "exp_plateau YM");
    assert_approx_eq(find_ds_param(&r_assoc, 0, "Y0"), 15.0, 1e-6, 1e-6, "assoc Y0");
    assert_approx_eq(find_ds_param(&r_plat, 0, "Y0"), 15.0, 1e-6, 1e-6, "exp_plateau Y0");
    assert_approx_eq(find_ds_param(&r_assoc, 0, "K"), 0.35, 1e-6, 1e-6, "assoc K");
    assert_approx_eq(find_ds_param(&r_plat, 0, "k"), 0.35, 1e-6, 1e-6, "exp_plateau k");
    assert!(!r_plat.derived_scalars.is_empty() && !r_plat.derived_scalars[0].is_empty(), "expected derived scalars on exponential_plateau");
}

/// Exact 128-row NIST StRD `Nelson` dataset (`y`, `x1`, `x2`) from Nelson, W. (1981),
/// *IEEE Transactions on Reliability* R-30(2):149–155. Model: `log[y] = b1 - b2 * x1 * exp(-b3 * x2)`.
const NELSON_RAW_DATA: &[(f64, f64, f64)] = &[
    // x1 = 1
    (15.0, 1.0, 180.0),
    (17.0, 1.0, 180.0),
    (15.5, 1.0, 180.0),
    (16.5, 1.0, 180.0),
    (15.5, 1.0, 225.0),
    (15.0, 1.0, 225.0),
    (16.0, 1.0, 225.0),
    (14.5, 1.0, 225.0),
    (15.0, 1.0, 250.0),
    (14.5, 1.0, 250.0),
    (12.5, 1.0, 250.0),
    (11.0, 1.0, 250.0),
    (14.0, 1.0, 275.0),
    (13.0, 1.0, 275.0),
    (14.0, 1.0, 275.0),
    (11.5, 1.0, 275.0),
    // x1 = 2
    (14.0, 2.0, 180.0),
    (16.0, 2.0, 180.0),
    (13.0, 2.0, 180.0),
    (13.5, 2.0, 180.0),
    (13.0, 2.0, 225.0),
    (13.5, 2.0, 225.0),
    (12.5, 2.0, 225.0),
    (12.5, 2.0, 225.0),
    (12.5, 2.0, 250.0),
    (12.0, 2.0, 250.0),
    (11.5, 2.0, 250.0),
    (12.0, 2.0, 250.0),
    (13.0, 2.0, 275.0),
    (11.5, 2.0, 275.0),
    (13.0, 2.0, 275.0),
    (12.5, 2.0, 275.0),
    // x1 = 4
    (13.5, 4.0, 180.0),
    (17.5, 4.0, 180.0),
    (17.5, 4.0, 180.0),
    (13.5, 4.0, 180.0),
    (12.5, 4.0, 225.0),
    (12.5, 4.0, 225.0),
    (15.0, 4.0, 225.0),
    (13.0, 4.0, 225.0),
    (12.0, 4.0, 250.0),
    (13.0, 4.0, 250.0),
    (12.0, 4.0, 250.0),
    (13.5, 4.0, 250.0),
    (10.0, 4.0, 275.0),
    (11.5, 4.0, 275.0),
    (11.0, 4.0, 275.0),
    (9.5, 4.0, 275.0),
    // x1 = 8
    (15.0, 8.0, 180.0),
    (15.0, 8.0, 180.0),
    (15.5, 8.0, 180.0),
    (16.0, 8.0, 180.0),
    (13.0, 8.0, 225.0),
    (10.5, 8.0, 225.0),
    (13.5, 8.0, 225.0),
    (14.0, 8.0, 225.0),
    (12.5, 8.0, 250.0),
    (12.0, 8.0, 250.0),
    (11.5, 8.0, 250.0),
    (11.5, 8.0, 250.0),
    (6.5, 8.0, 275.0),
    (5.5, 8.0, 275.0),
    (6.0, 8.0, 275.0),
    (6.0, 8.0, 275.0),
    // x1 = 16
    (18.5, 16.0, 180.0),
    (17.0, 16.0, 180.0),
    (15.3, 16.0, 180.0),
    (16.0, 16.0, 180.0),
    (13.0, 16.0, 225.0),
    (14.0, 16.0, 225.0),
    (12.5, 16.0, 225.0),
    (11.0, 16.0, 225.0),
    (12.0, 16.0, 250.0),
    (12.0, 16.0, 250.0),
    (11.5, 16.0, 250.0),
    (12.0, 16.0, 250.0),
    (6.0, 16.0, 275.0),
    (6.0, 16.0, 275.0),
    (5.0, 16.0, 275.0),
    (5.5, 16.0, 275.0),
    // x1 = 32
    (12.5, 32.0, 180.0),
    (13.0, 32.0, 180.0),
    (16.0, 32.0, 180.0),
    (12.0, 32.0, 180.0),
    (11.0, 32.0, 225.0),
    (9.5, 32.0, 225.0),
    (11.0, 32.0, 225.0),
    (11.0, 32.0, 225.0),
    (11.0, 32.0, 250.0),
    (10.0, 32.0, 250.0),
    (10.5, 32.0, 250.0),
    (10.5, 32.0, 250.0),
    (2.7, 32.0, 275.0),
    (2.7, 32.0, 275.0),
    (2.5, 32.0, 275.0),
    (2.4, 32.0, 275.0),
    // x1 = 48
    (13.0, 48.0, 180.0),
    (13.5, 48.0, 180.0),
    (16.5, 48.0, 180.0),
    (13.6, 48.0, 180.0),
    (11.5, 48.0, 225.0),
    (10.5, 48.0, 225.0),
    (13.5, 48.0, 225.0),
    (12.0, 48.0, 225.0),
    (7.0, 48.0, 250.0),
    (6.9, 48.0, 250.0),
    (8.8, 48.0, 250.0),
    (7.9, 48.0, 250.0),
    (1.2, 48.0, 275.0),
    (1.5, 48.0, 275.0),
    (1.0, 48.0, 275.0),
    (1.5, 48.0, 275.0),
    // x1 = 64
    (13.0, 64.0, 180.0),
    (12.5, 64.0, 180.0),
    (16.5, 64.0, 180.0),
    (16.0, 64.0, 180.0),
    (11.0, 64.0, 225.0),
    (11.5, 64.0, 225.0),
    (10.5, 64.0, 225.0),
    (10.0, 64.0, 225.0),
    (7.27, 64.0, 250.0),
    (7.5, 64.0, 250.0),
    (6.7, 64.0, 250.0),
    (7.6, 64.0, 250.0),
    (1.5, 64.0, 275.0),
    (1.0, 64.0, 275.0),
    (1.2, 64.0, 275.0),
    (1.2, 64.0, 275.0),
];

/// Task 7 (Confirm NIST `Nelson` in `nist.rs` fits `log(y)`) accurately against certified NIST StRD
/// estimates, standard errors, RSS, and RSD (vs raw `y` which is orders of magnitude off).
#[test]
fn r46_7_nist_nelson_fits_log_y_oracle() {
    assert_eq!(NELSON_RAW_DATA.len(), 128);
    let model = get_builtin_model("nist_nelson").expect("nist_nelson");

    let cert_betas = [2.5906836021_f64, 5.6177717026e-9_f64, -5.7701013174e-2_f64];
    let cert_ses = [1.9149996413e-2_f64, 6.1124096540e-9_f64, 3.9572366543e-3_f64];
    let cert_rss = 3.7976833176_f64;
    let cert_rsd = 1.7430280130e-1_f64;

    assert_grad_matches_fd(model.as_ref(), &[16.0, 250.0], &cert_betas, 0, 1e-8, 1e-6, "nist_nelson");

    let build_series = |label: &str, use_log: bool| -> DatasetSeries {
        let points = NELSON_RAW_DATA
            .iter()
            .enumerate()
            .map(|(row, &(y, x1, x2))| DataPoint {
                x: vec![x1, x2],
                y: if use_log { y.ln() } else { y },
                sd: None,
                n: None,
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: 0,
                row,
            })
            .collect();
        DatasetSeries {
            data_set_index: 0,
            label: label.to_owned(),
            points,
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: false,
            column_title_value: None,
        }
    };

    let ds_log = build_series("Nelson_log_y", true);
    let start1 = [2.0, 1e-4, -0.01];
    let start2 = [2.5, 5e-9, -0.05];
    for (tag, starts) in [("start1", Some(start1)), ("start2", Some(start2)), ("default_init", None)] {
        let rules = starts
            .map(|vals| model.param_names().iter().zip(vals.iter()).map(|(n, &v)| rule_local(n, Some(v), None, None)).collect())
            .unwrap_or_default();
        let cfg = SolverConfig { parameter_rules: rules, compute_profile_ci: false, ..SolverConfig::default() };
        let res = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds_log), &HookContext::default(), &cfg)
            .unwrap_or_else(|e| panic!("Nelson log(y) {tag} failed: {e:?}"));
        assert_eq!(res.status, FitStatus::Converged, "Nelson log(y) {tag} did not converge: {:?}", res.status);
        assert_eq!(res.df, 125);
        for i in 0..3 {
            assert!(log_relative_error(res.parameters[i].estimate, cert_betas[i]) >= 7.0, "Nelson {tag} b{} LRE low", i + 1);
            assert!(log_relative_error(res.parameters[i].se, cert_ses[i]) >= 5.0, "Nelson {tag} SE{} LRE low", i + 1);
        }
        assert!(log_relative_error(res.rss, cert_rss) >= 7.5, "Nelson {tag} RSS={} vs {cert_rss}", res.rss);
        assert!(log_relative_error(res.rsd, cert_rsd) >= 7.5, "Nelson {tag} RSD={} vs {cert_rsd}", res.rsd);
    }

    // Raw untransformed `y` does NOT match the Nelson `log[y]` specification.
    let ds_raw = build_series("Nelson_raw_y", false);
    let rules_raw: Vec<ParameterRule> =
        model.param_names().iter().zip([15.0, 1e-4, -0.01]).map(|(n, v)| rule_local(n, Some(v), None, None)).collect();
    let res_raw = fit_nonlinear(
        model.as_ref(),
        std::slice::from_ref(&ds_raw),
        &HookContext::default(),
        &SolverConfig { parameter_rules: rules_raw, compute_profile_ci: false, ..SolverConfig::default() },
    )
    .expect("raw y fit");
    assert!((res_raw.rss - cert_rss).abs() > 100.0, "raw y unexpectedly matched log(y) RSS: {}", res_raw.rss);
}

/// Task 8 (Dead-code cleanup parity: `logistic_growth`, `operational_model_agonist`).
#[test]
fn r46_8_logistic_growth_and_operational_model_oracles() {
    // Verhulst (1838) / Seber & Wild (1989) ch. 7 logistic growth: `Y = YM * Y0 / (Y0 + (YM - Y0) * exp(-k * t))`.
    let logistic = get_builtin_model("logistic_growth").expect("logistic_growth");
    let (ym, y0, k) = (250.0, 10.0, 0.35);
    let ts: Vec<f64> = (0..=20).map(|i| i as f64).collect();
    let ys_log: Vec<f64> = ts.iter().map(|&t| ym * y0 / (y0 + (ym - y0) * (-k * t).exp())).collect();
    let res_log =
        fit_nonlinear(logistic.as_ref(), &[series("Logistic", ts, ys_log)], &HookContext::default(), &SolverConfig::default()).expect("logistic fit");
    assert_eq!(res_log.status, FitStatus::Converged);
    assert_approx_eq(find_ds_param(&res_log, 0, "YM"), ym, 1e-5, 1e-6, "logistic YM");
    assert_approx_eq(find_ds_param(&res_log, 0, "Y0"), y0, 1e-5, 1e-6, "logistic Y0");
    assert_approx_eq(find_ds_param(&res_log, 0, "k"), k, 1e-5, 1e-6, "logistic k");

    // Black & Leff (1983) / Black, Leff, Shankley & Wood (1985) operational model of agonism:
    // `Y = Basal + (Emax - Basal) * (tau * [A])^n / ((tau * [A])^n + ([A] + KA)^n)`.
    let op = get_builtin_model("operational_model_agonist").expect("operational_model_agonist");
    let (basal, emax, log_ka, log_tau, n) = (5.0_f64, 95.0_f64, -6.0_f64, 0.8_f64, 1.3_f64);
    let ka = 10.0_f64.powf(log_ka);
    let tau = 10.0_f64.powf(log_tau);
    let log_xs: Vec<f64> = (-10..=-3).map(|i| i as f64).collect();
    let ys_op: Vec<f64> = log_xs
        .iter()
        .map(|&lx| {
            let a = 10.0_f64.powf(lx);
            let op_num = (tau * a).powf(n);
            let expected = basal + (emax - basal) * op_num / (op_num + (a + ka).powf(n));
            let actual = op.eval(&[lx], &[basal, emax, log_ka, log_tau, n], 0).expect("op eval");
            assert_approx_eq(actual, expected, 1e-12, 1e-12, &format!("operational model at logA={lx}"));
            expected
        })
        .collect();

    // Anchor system maximum Emax & Basal (standard Motulsky & Christopoulos 2004 single-partial-agonist setup)
    // and verify nonlinear fit recovers LogKA, LogTau, and transducer slope n accurately.
    let cfg_op = SolverConfig { parameter_rules: vec![rule_const("Basal", basal), rule_const("Emax", emax)], ..SolverConfig::default() };
    let res_op =
        fit_nonlinear(op.as_ref(), &[series("Operational", log_xs, ys_op)], &HookContext::default(), &cfg_op).expect("operational_model_agonist fit");
    assert_eq!(res_op.status, FitStatus::Converged);
    assert_approx_eq(find_ds_param(&res_op, 0, "LogKA"), log_ka, 1e-5, 1e-5, "operational LogKA");
    assert_approx_eq(find_ds_param(&res_op, 0, "LogTau"), log_tau, 1e-5, 1e-5, "operational LogTau");
    assert_approx_eq(find_ds_param(&res_op, 0, "n"), n, 1e-5, 1e-5, "operational n");
}
