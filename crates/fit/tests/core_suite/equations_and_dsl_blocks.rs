use curvebook_doc::{ConstantValue, HookSource, InfoSheet, SheetId};
use curvebook_fit::{DatasetSeries, FitStatus, HookContext, ParametricModel, SolverConfig, fit_nonlinear, get_builtin_model, list_builtin_equations};
use curvebook_math::dist::norm_cdf;
use curvebook_testkit::assert_approx_eq;

use crate::common::{
    assert_converged, assert_dsl_compile_err, assert_grad_matches_fd, assert_params_approx, cfg_rules, dataset_param_val, dsl_compile, find_derived,
    fit_default, fit_with, rule_col_title, rule_const, rule_hook, rule_local_bounds, rule_shared_bounds, series_xy,
};

fn swillens_specific_bound(hot_tot: f64, rt: f64, kd: f64, ns: f64) -> f64 {
    let a = 1.0 + ns;
    let b = kd * a + rt - hot_tot;
    let disc = (b * b + 4.0 * a * kd * hot_tot).sqrt();
    let l_free = if b >= 0.0 { (2.0 * kd * hot_tot) / (b + disc) } else { (-b + disc) / (2.0 * a) };
    rt * l_free / (kd + l_free)
}

#[test]
fn centered_polynomials_fit_and_grad_oracles() {
    let x_center = 2005.0_f64;
    let xs: Vec<f64> = (1995..=2015).map(|v| v as f64).collect();
    let true_coeffs = [12.5, -1.8, 0.25, -0.02, 0.0015, -0.00008, 0.000003];
    let catalog = list_builtin_equations();

    for deg in 1..=6 {
        let id = format!("centered_poly_{deg}");
        assert!(catalog.iter().any(|e| e.id == id && !e.name.contains("--")));
        let model = get_builtin_model(&id).unwrap();
        assert_eq!(model.param_count(), deg + 2);
        assert_eq!(model.param_names().last().map(String::as_str), Some("XCenter"));

        let mut test_params = true_coeffs[..=deg].to_vec();
        test_params.push(x_center);
        assert_grad_matches_fd(model.as_ref(), &[2009.0], &test_params, 0, 1e-6);

        let ys: Vec<f64> = xs.iter().map(|&x| (0..=deg).fold(0.0_f64, |acc, k| acc + true_coeffs[k] * (x - x_center).powi(k as i32))).collect();
        let res = fit_with(
            model.as_ref(),
            &[series_xy(0, &id, &xs, &ys)],
            &SolverConfig { parameter_rules: vec![rule_const("XCenter", x_center)], max_iters: 1000, ..Default::default() },
        );
        assert_converged(&res, &id);
        for (k, &expected) in true_coeffs.iter().enumerate().take(deg + 1) {
            assert_approx_eq(dataset_param_val(&res, 0, &format!("B{k}")), expected, 1e-5, 1e-5, &format!("{id} B{k}"));
        }

        let res_free = fit_default(model.as_ref(), &[series_xy(0, &format!("{id}_free"), &xs, &ys)]);
        assert!(matches!(res_free.status, FitStatus::Converged | FitStatus::Ambiguous));
        for (&y_hat, &y_obs) in res_free.datasets[0].y_hat.iter().zip(&ys) {
            assert_approx_eq(y_hat, y_obs, 1e-4, 1e-5, &format!("{id} pred"));
        }
    }
}

#[test]
fn semi_log_line_oracle() {
    let semi = get_builtin_model("semi_log_line").unwrap();
    assert_eq!((semi.eval(&[0.0], &[2.0, 1.0], 0), semi.eval(&[-1.0], &[2.0, 1.0], 0)), (None, None));
    assert_grad_matches_fd(semi.as_ref(), &[12.5], &[18.4, 7.25], 0, 1e-8);

    let xs: [f64; 8] = [0.5, 1.0, 2.5, 10.0, 25.0, 100.0, 250.0, 1000.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 24.5 * x.log10() - 11.2).collect();
    let res = fit_default(semi.as_ref(), &[series_xy(0, "SemiLog", &xs, &ys)]);
    assert_converged(&res, "semi_log_line");
    assert_params_approx(&res, 0, &[("Slope", 24.5, 1e-8, 1e-8), ("YIntercept", -11.2, 1e-8, 1e-8)], "semi_log");
}

#[test]
fn log_log_line_and_probit_line_oracles() {
    let loglog = get_builtin_model("log_log_line").unwrap();
    assert_eq!(loglog.eval(&[0.0], &[1.2, 0.5], 0), None);
    assert_grad_matches_fd(loglog.as_ref(), &[8.0], &[1.35, -0.4], 0, 1e-6);

    let xs_pos: [f64; 8] = [0.5, 1.0, 2.5, 10.0, 25.0, 100.0, 250.0, 1000.0];
    let ys_ll: Vec<f64> = xs_pos.iter().map(|&x| 10.0_f64.powf(1.42 * x.log10() + 0.75)).collect();
    let res_ll = fit_default(loglog.as_ref(), &[series_xy(0, "LogLog", &xs_pos, &ys_ll)]);
    assert_converged(&res_ll, "log_log_line");
    assert_params_approx(&res_ll, 0, &[("Slope", 1.42, 1e-6, 1e-6), ("LogYIntercept", 0.75, 1e-6, 1e-6)], "log_log");

    let probit = get_builtin_model("probit_line").unwrap();
    assert_grad_matches_fd(probit.as_ref(), &[-2.8], &[1.6, 4.8], 0, 1e-6);
    assert_approx_eq(probit.eval(&[-4.8 / 1.6], &[1.6, 4.8], 0).unwrap(), 50.0, 1e-10, 1e-10, "probit 50%");

    let xs_p: Vec<f64> = (-50..=-10).step_by(4).map(|k| k as f64 * 0.1).collect();
    let ys_p: Vec<f64> = xs_p.iter().map(|&x| norm_cdf(1.6 * x + 4.8) * 100.0).collect();
    let res_p = fit_default(probit.as_ref(), &[series_xy(0, "Probit", &xs_p, &ys_p)]);
    assert_converged(&res_p, "probit_line");
    assert_params_approx(&res_p, 0, &[("Slope", 1.6, 1e-6, 1e-6), ("Intercept", 4.8, 1e-6, 1e-6)], "probit");
}

#[test]
fn dose_response_3pl_4pl_analytical_grad_oracles() {
    for &(id, params) in &[
        ("log_agonist_vs_response_3pl", &[12.0, 185.0, -7.2][..]),
        ("log_agonist_vs_response_4pl", &[12.0, 185.0, -7.2, 1.45][..]),
        ("log_inhibitor_vs_response_3pl", &[12.0, 185.0, -6.8][..]),
        ("log_inhibitor_vs_response_4pl", &[12.0, 185.0, -6.8, 1.25][..]),
        ("log_agonist_vs_response_normalized", &[-7.0][..]),
        ("log_agonist_vs_response_normalized_var", &[-7.0, 1.6][..]),
        ("log_inhibitor_vs_response_normalized", &[-6.5][..]),
        ("log_inhibitor_vs_response_normalized_var", &[-6.5, 1.3][..]),
        ("one_site_fit_logic50", &[10.0, 500.0, -8.1][..]),
    ] {
        let m = get_builtin_model(id).unwrap();
        for &x in &[-10.0, -7.0, -4.0] {
            assert_grad_matches_fd(m.as_ref(), &[x], params, 0, 1e-5);
        }
        let mut extremum_grad = vec![0.0_f64; params.len()];
        assert!(m.grad(&[500.0], params, 0, &mut extremum_grad) && extremum_grad.iter().all(|v| v.is_finite()));
    }

    for &(id, params) in &[
        ("agonist_vs_response_3pl", &[10.0, 220.0, 2.5e-7][..]),
        ("agonist_vs_response_4pl", &[10.0, 220.0, 2.5e-7, 1.35][..]),
        ("inhibitor_vs_response_3pl", &[15.0, 195.0, 4.0e-8][..]),
        ("inhibitor_vs_response_4pl", &[15.0, 195.0, 4.0e-8, 1.15][..]),
    ] {
        let m = get_builtin_model(id).unwrap();
        for &c in &[0.0, 1e-9, params[2], 5.0 * params[2]] {
            assert_grad_matches_fd(m.as_ref(), &[c], params, 0, 1e-5);
        }
    }

    let m4 = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let log_x: Vec<f64> = (-11..=-4).map(|k| k as f64).collect();
    let batch: Vec<DatasetSeries> = (0..96)
        .map(|i| {
            let (bot, top, ec, hill) =
                (10.0 + (i % 5) as f64, 110.0 + (i % 7) as f64 * 5.0, -8.0 + ((i % 9) as f64) * 0.2, 0.9 + ((i % 4) as f64) * 0.2);
            let ys: Vec<f64> = log_x.iter().map(|&x| bot + (top - bot) / (1.0 + 10.0_f64.powf((ec - x) * hill))).collect();
            series_xy(i, &format!("Well_{i}"), &log_x, &ys)
        })
        .collect();
    let batch_res = fit_default(m4.as_ref(), &batch);
    assert_converged(&batch_res, "96-curve 4PL");
    assert_eq!(batch_res.datasets.len(), 96);
    assert_approx_eq(dataset_param_val(&batch_res, 0, "LogEC50"), -8.0, 1e-5, 1e-5, "well 0 LogEC50");
}

#[test]
fn receptor_binding_global_ns_and_ligand_depletion_oracles() {
    let m_glob = get_builtin_model("one_site_total_and_ns_global").unwrap();
    let (bmax, kd, ns, bg) = (1800.0, 3.5, 42.0, 85.0);
    assert_grad_matches_fd(m_glob.as_ref(), &[2.5], &[bmax, kd, ns, bg], 0, 1e-6);
    assert_grad_matches_fd(m_glob.as_ref(), &[2.5], &[bmax, kd, ns, bg], 1, 1e-6);

    let hot_x = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let y_tot: Vec<f64> = hot_x.iter().map(|&x| bmax * x / (kd + x) + ns * x + bg).collect();
    let y_ns: Vec<f64> = hot_x.iter().map(|&x| ns * x + bg).collect();
    let cfg_glob = cfg_rules(vec![
        rule_shared_bounds("Bmax", Some(1500.0), Some(0.0), None),
        rule_shared_bounds("Kd", Some(2.0), Some(1e-6), None),
        rule_shared_bounds("NS", Some(20.0), Some(0.0), None),
        rule_shared_bounds("Background", Some(50.0), None, None),
    ]);
    let res_glob = fit_with(m_glob.as_ref(), &[series_xy(0, "Hot", &hot_x, &y_tot), series_xy(1, "[NS]", &hot_x, &y_ns)], &cfg_glob);
    assert_converged(&res_glob, "one_site_total_and_ns_global");
    assert_approx_eq(dataset_param_val(&res_glob, 0, "Bmax"), bmax, 1e-5, 1e-6, "global Bmax");
    assert_approx_eq(dataset_param_val(&res_glob, 0, "Kd"), kd, 1e-6, 1e-6, "global Kd");
    assert_approx_eq(dataset_param_val(&res_glob, 1, "NS"), ns, 1e-6, 1e-6, "global NS");
    assert_approx_eq(dataset_param_val(&res_glob, 1, "Background"), bg, 1e-6, 1e-6, "global Background");

    let m_dep = get_builtin_model("one_site_specific_ligand_depletion").unwrap();
    let (rt_t, kd_t, ns_t) = (25.0, 3.0, 0.08);
    assert_grad_matches_fd(m_dep.as_ref(), &[12.0], &[rt_t, kd_t, ns_t], 0, 1e-6);
    for &lt in &[0.0, 2.0, 15.0, 80.0] {
        let morrison = 0.5 * ((rt_t + lt + kd_t) - ((rt_t + lt + kd_t).powi(2) - 4.0 * rt_t * lt).sqrt());
        assert_approx_eq(m_dep.eval(&[lt], &[rt_t, kd_t, 0.0], 0).unwrap(), morrison, 1e-12, 1e-12, "Morrison NS=0");
    }

    let x_tot = [1.0, 3.0, 7.5, 15.0, 25.0, 40.0, 70.0, 120.0];
    let y_dep: Vec<f64> = x_tot.iter().map(|&lt| swillens_specific_bound(lt, rt_t, kd_t, ns_t)).collect();
    let cfg_dep = cfg_rules(vec![
        rule_local_bounds("Rt", Some(20.0), Some(0.0), None),
        rule_local_bounds("Kd", Some(5.0), Some(1e-6), None),
        rule_const("NS", ns_t),
    ]);
    let res_dep = fit_with(m_dep.as_ref(), &[series_xy(0, "Depleted", &x_tot, &y_dep)], &cfg_dep);
    assert_converged(&res_dep, "one_site_specific_ligand_depletion");
    assert_approx_eq(dataset_param_val(&res_dep, 0, "Rt"), rt_t, 1e-6, 1e-6, "depletion Rt");
    assert_approx_eq(dataset_param_val(&res_dep, 0, "Kd"), kd_t, 1e-6, 1e-6, "depletion Kd");
}

#[test]
fn allosteric_saturation_and_homologous_competition_oracles() {
    let m_allo = get_builtin_model("allosteric_modulator_saturation").unwrap();
    let (bmax, kd, alpha, kb) = (250.0, 4.0, 5.0, 12.0);
    assert_grad_matches_fd(m_allo.as_ref(), &[3.0], &[bmax, kd, alpha, kb, 15.0], 0, 1e-6);

    let x_rad = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let datasets: Vec<DatasetSeries> = [0.0, 4.0, 16.0, 64.0]
        .into_iter()
        .enumerate()
        .map(|(i, mc)| {
            let kd_app = kd * (1.0 + mc / kb) / (1.0 + alpha * mc / kb);
            let ys: Vec<f64> = x_rad.iter().map(|&x| bmax * x / (kd_app + x)).collect();
            series_xy(i, &format!("{mc} nM"), &x_rad, &ys)
        })
        .collect();
    let cfg_allo = SolverConfig {
        parameter_rules: vec![
            rule_shared_bounds("Bmax", Some(220.0), Some(1.0), None),
            rule_shared_bounds("Kd", Some(3.0), Some(1e-6), None),
            rule_shared_bounds("alpha", Some(2.0), Some(0.0), None),
            rule_shared_bounds("Kb", Some(10.0), Some(1e-6), None),
            rule_col_title("ModConc"),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let res_allo = fit_with(m_allo.as_ref(), &datasets, &cfg_allo);
    assert_converged(&res_allo, "allosteric_modulator_saturation");
    assert_params_approx(
        &res_allo,
        0,
        &[("Bmax", bmax, 1e-5, 1e-6), ("Kd", kd, 1e-6, 1e-6), ("alpha", alpha, 1e-5, 1e-5), ("Kb", kb, 1e-5, 1e-5)],
        "Allo",
    );

    let m_hom = get_builtin_model("homologous_one_site_competition").unwrap();
    let (bmax_h, kd_h, ns_h, hot_h) = (2400.0, 4.5e-9, 120.0, 2.0e-9);
    assert_grad_matches_fd(m_hom.as_ref(), &[-8.5], &[bmax_h, kd_h, ns_h, hot_h], 0, 1e-5);
    let log_cold: Vec<f64> = (-24..=-10).map(|i| i as f64 * 0.5).collect();
    let y_hom: Vec<f64> = log_cold.iter().map(|&lx| bmax_h * hot_h / (hot_h + 10.0_f64.powf(lx) + kd_h) + ns_h).collect();
    let cfg_hom = SolverConfig {
        parameter_rules: vec![
            rule_local_bounds("Bmax", Some(2000.0), Some(0.0), None),
            rule_local_bounds("Kd", Some(5.0e-9), Some(1e-18), None),
            rule_local_bounds("NS", Some(100.0), None, None),
            rule_const("HotConc", hot_h),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let res_hom = fit_with(m_hom.as_ref(), &[series_xy(0, "Homologous", &log_cold, &y_hom)], &cfg_hom);
    assert_converged(&res_hom, "homologous_one_site_competition");
    assert_params_approx(&res_hom, 0, &[("Bmax", bmax_h, 1e-3, 1e-5), ("Kd", kd_h, 1e-15, 1e-5), ("NS", ns_h, 1e-4, 1e-5)], "Hom");
}

#[test]
fn motulsky_mahan_competitive_kinetics_oracle() {
    let m_mm = get_builtin_model("motulsky_mahan_kinetics").unwrap();
    let (bmax, kon1, koff1, kon2, koff2, l, i_conc) = (100.0, 5.0e6, 0.15, 2.5e6, 0.04, 1.0e-8, 4.0e-8);
    let params = [bmax, kon1, koff1, kon2, koff2, l, i_conc];

    let (dt, mut rl, mut ri) = (0.002_f64, 0.0_f64, 0.0_f64);
    let deriv = |y1: f64, y2: f64| {
        let free_r = bmax - y1 - y2;
        (kon1 * l * free_r - koff1 * y1, kon2 * i_conc * free_r - koff2 * y2)
    };
    for step in 1..=25_000 {
        let (k1a, k1b) = deriv(rl, ri);
        let (k2a, k2b) = deriv(rl + 0.5 * dt * k1a, ri + 0.5 * dt * k1b);
        let (k3a, k3b) = deriv(rl + 0.5 * dt * k2a, ri + 0.5 * dt * k2b);
        let (k4a, k4b) = deriv(rl + dt * k3a, ri + dt * k3b);
        rl += (dt / 6.0) * (k1a + 2.0 * k2a + 2.0 * k3a + k4a);
        ri += (dt / 6.0) * (k1b + 2.0 * k2b + 2.0 * k3b + k4b);
        if step % 2500 == 0 {
            let t = step as f64 * dt;
            assert_approx_eq(m_mm.eval(&[t], &params, 0).unwrap(), rl, 1e-8, 1e-8, &format!("MM RK4 t={t}"));
        }
    }

    let eq_expected = bmax * l / (l + (koff1 / kon1) * (1.0 + i_conc / (koff2 / kon2)));
    assert_approx_eq(m_mm.eval(&[1e5], &params, 0).unwrap(), eq_expected, 1e-8, 1e-8, "MM equilibrium");

    let k_eq = kon1 * l + koff1;
    assert_approx_eq(
        m_mm.eval(&[6.0], &[bmax, kon1, koff1, 0.0, k_eq, l, 0.0], 0).unwrap(),
        (bmax * kon1 * l / k_eq) * (1.0 - (-k_eq * 6.0).exp()),
        1e-9,
        1e-9,
        "degenerate Ka==Kb",
    );

    let ts = [0.0, 0.5, 1.0, 2.0, 4.0, 7.0, 12.0, 20.0, 35.0, 60.0, 100.0, 160.0];
    let ys: Vec<f64> = ts.iter().map(|&t| m_mm.eval(&[t], &params, 0).unwrap()).collect();
    let cfg = SolverConfig {
        parameter_rules: vec![
            rule_const("Bmax", bmax),
            rule_const("kon1", kon1),
            rule_const("koff1", koff1),
            rule_local_bounds("kon2", Some(1.0e6), Some(1.0), None),
            rule_local_bounds("koff2", Some(0.08), Some(1e-6), None),
            rule_const("L", l),
            rule_const("I", i_conc),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let res = fit_with(m_mm.as_ref(), &[series_xy(0, "CompKinetics", &ts, &ys)], &cfg);
    assert_converged(&res, "motulsky_mahan_kinetics");
    assert_params_approx(&res, 0, &[("kon2", kon2, 1.0, 1e-5), ("koff2", koff2, 1e-7, 1e-6)], "MM");
}

#[test]
fn dsl_initial_values_and_transforms_end_to_end() {
    let model = dsl_compile(
        "[Equation]\nY = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))\n\
         [InitialValues]\nBottom = ymin\nTop = YMAX\nSpanInit = SPAN\nHalfCheck = YMID\n\
         XRangeCheck = XMAX - XMIN\nXMidCheck = XMID\nLoX = XATYMIN\nHiX = XATYMAX\n\
         LogEC50 = XATY50\nHillSlope = SIGN + 0 * (SpanInit + HalfCheck + XRangeCheck + XMidCheck + LoX + HiX)\n\
         [Transforms]\nEC50 = 10^LogEC50\nSpan = Top - Bottom\nRatio = EC50 / Span",
    );
    assert_eq!(model.param_names(), &["Bottom", "Top", "LogEC50", "HillSlope"]);

    let (bot_t, top_t, log_ec50_t, hill_t) = (320.0, 4520.0, -8.25, -1.35);
    let log_x: Vec<f64> = (-24..=-10).map(|i| i as f64 * 0.5).collect();
    let ys: Vec<f64> = log_x.iter().map(|&x| bot_t + (top_t - bot_t) / (1.0 + 10.0_f64.powf((log_ec50_t - x) * hill_t))).collect();
    let s = series_xy(0, "Inhibitor", &log_x, &ys);

    let init = model.initial_guess(&s.points);
    assert!(init[0] > 300.0 && init[0] < 350.0 && init[1] > 4500.0 && (init[2] - log_ec50_t).abs() < 0.6);
    assert_approx_eq(init[3], -1.0, 1e-12, 1e-12, "SIGN == -1");

    let res = fit_default(&model, &[s]);
    assert_converged(&res, "DSL [InitialValues]+[Transforms]");
    assert_params_approx(
        &res,
        0,
        &[("Bottom", bot_t, 1e-4, 1e-5), ("Top", top_t, 1e-4, 1e-5), ("LogEC50", log_ec50_t, 1e-5, 1e-5), ("HillSlope", hill_t, 1e-5, 1e-5)],
        "DSL",
    );
    let (exp_ec50, exp_span) = (10.0_f64.powf(log_ec50_t), top_t - bot_t);
    assert_approx_eq(find_derived(&res, 0, "EC50"), exp_ec50, 1e-13, 1e-5, "EC50");
    assert_approx_eq(find_derived(&res, 0, "Span"), exp_span, 1e-4, 1e-5, "Span");
    assert_approx_eq(find_derived(&res, 0, "Ratio"), exp_ec50 / exp_span, 1e-16, 1e-5, "Ratio");

    let m_scoped = dsl_compile("[Equation]\nY = A * X + B\n[InitialValues]\n<A> A = 10\n<B> A = 20\nB = YMIN");
    let ds_a = series_xy(0, "A", &[0.0, 1.0], &[5.0, 15.0]);
    let ds_b = series_xy(1, "B", &[0.0, 1.0], &[7.0, 27.0]);
    assert_approx_eq(m_scoped.initial_guess(&ds_a.points)[0], 10.0, 1e-12, 1e-12, "<A> A");
    assert_approx_eq(m_scoped.initial_guess(&ds_b.points)[0], 20.0, 1e-12, 1e-12, "<B> A");
    assert_approx_eq(m_scoped.initial_guess(&ds_b.points)[1], 7.0, 1e-12, 1e-12, "<B> B");

    for bad in [
        "Y = A * X\n[Bogus]\nk = 1",
        "Y = A * X\n[InitialValues]\nA = X + 1",
        "Y = A * X\n[InitialValues]\nA = UNKNOWN_STAT",
        "Y = A * X\n[Transforms]\nT = A * X",
        "Y = A * X\n[Transforms]\nT = NO_SUCH_PARAM * 2",
        "Y = A * X\n[Transforms]\n<A> T = A * 2",
    ] {
        assert_dsl_compile_err(bad);
    }
}

#[test]
fn dsl_generalized_prefixes_and_scoped_intermediates() {
    let m_uc = dsl_compile(
        "<A> I = 0\n<B> I = 5\n<C> I = 15\n<D> I = 40\nVapp = Vmax / (1 + I / alphaKi)\nKapp = Km / (1 + I / alphaKi)\nY = Vapp * X / (Kapp + X)",
    );
    assert_eq!(m_uc.param_names(), &["Vmax", "alphaKi", "Km"]);
    assert_eq!(m_uc.eval(&[10.0], &[100.0, 10.0, 4.0], 4), None);

    let (vmax_t, km_t, aki_t) = (140.0, 4.5, 12.0);
    let s_vals = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let ds_series: Vec<DatasetSeries> = [0.0, 5.0, 15.0, 40.0]
        .into_iter()
        .enumerate()
        .map(|(k, ci)| {
            let ys: Vec<f64> = s_vals.iter().map(|&s| (vmax_t / (1.0 + ci / aki_t)) * s / (km_t / (1.0 + ci / aki_t) + s)).collect();
            series_xy(k, &format!("I={ci}"), &s_vals, &ys)
        })
        .collect();
    let cfg = cfg_rules(vec![
        rule_shared_bounds("Vmax", Some(100.0), Some(1.0), None),
        rule_shared_bounds("alphaKi", Some(8.0), Some(1e-3), None),
        rule_shared_bounds("Km", Some(3.0), Some(1e-3), None),
    ]);
    let res_uc = fit_with(&m_uc, &ds_series, &cfg);
    assert_converged(&res_uc, "scoped DSL global uncomp");
    assert_params_approx(&res_uc, 0, &[("Vmax", vmax_t, 1e-5, 1e-6), ("Km", km_t, 1e-5, 1e-6), ("alphaKi", aki_t, 1e-5, 1e-6)], "DSL");

    let m_wide =
        dsl_compile("span = Top - Bottom\n<A> scale = 1\n<C> scale = 2\n<Z> scale = 3\n<30> scale = 4\n<A,C,Z,30> Y = Bottom + scale * span * X");
    assert_eq!(m_wide.param_names(), &["Top", "Bottom"]);
    for (ds, exp) in [(0, Some(25.0)), (2, Some(45.0)), (25, Some(65.0)), (30, Some(85.0)), (1, None)] {
        assert_eq!(m_wide.eval(&[2.0], &[15.0, 5.0], ds), exp, "wide ds={ds}");
    }

    let m_rng = dsl_compile("shift = Offset * 2\n<1:3> gain = 10\n<4:5> gain = 20\n<1:5> Y = shift + gain * X");
    for (ds, exp) in [(2, Some(33.0)), (5, Some(63.0)), (0, None), (6, None)] {
        assert_eq!(m_rng.eval(&[3.0], &[1.5], ds), exp, "range ds={ds}");
    }

    let _ = dsl_compile(&format!("Y = {}X{}", "(".repeat(100), ")".repeat(100)));
    assert_dsl_compile_err(&format!("Y = {}X{}", "(".repeat(200), ")".repeat(200)));
    let _ = dsl_compile("<0:9999> Y = X");
    assert_dsl_compile_err("<0:10000> Y = X");

    let mut info = InfoSheet::new_preseeded("assay", "Radioligand Info").unwrap();
    info.set_constant("HotConc", ConstantValue::Number(2.0), None).unwrap();
    let infos = [info];
    let hooks = HookContext { info_sheets: &infos, ..HookContext::default() };
    let m_hom = get_builtin_model("homologous_one_site_competition").unwrap();
    let lx: Vec<f64> = (-3..=4).map(|k| k as f64).collect();
    let yh: Vec<f64> = lx.iter().map(|&x| 500.0 * 2.0 / (2.0 + 10.0_f64.powf(x) + 3.0) + 25.0).collect();
    let cfg_hook = cfg_rules(vec![
        rule_local_bounds("Bmax", Some(450.0), Some(0.0), None),
        rule_local_bounds("Kd", Some(4.0), Some(1e-6), None),
        rule_local_bounds("NS", Some(20.0), None, None),
        rule_hook("HotConc", HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("assay")), constant_name: "HotConc".to_owned() }),
    ]);
    let res_hook = fit_nonlinear(m_hom.as_ref(), &[series_xy(0, "HomHook", &lx, &yh)], &hooks, &cfg_hook).unwrap();
    assert_converged(&res_hook, "homologous InfoConstant hook");
    assert_approx_eq(dataset_param_val(&res_hook, 0, "Kd"), 3.0, 1e-5, 1e-5, "HomHook Kd");
}
