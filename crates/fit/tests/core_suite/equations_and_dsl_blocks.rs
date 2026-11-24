use curvebook_doc::{ConstantValue, HookSource, InfoSheet, ParameterRule, ParameterScope, SheetId};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, FitStatus, HookContext, NonlinearFitResult, ParametricModel, SolverConfig, fit_nonlinear,
    get_builtin_model, list_builtin_equations,
};
use curvebook_math::dist::norm_cdf;
use curvebook_testkit::assert_approx_eq;

fn series(label: &str, xs: Vec<f64>, ys: Vec<f64>) -> DatasetSeries {
    series_idx(0, label, xs, ys)
}

fn series_idx(data_set_index: usize, label: &str, xs: Vec<f64>, ys: Vec<f64>) -> DatasetSeries {
    let points = xs
        .into_iter()
        .zip(ys)
        .enumerate()
        .map(|(row, (x, y))| {
            let mut pt = DataPoint::xy(x, y);
            pt.data_set_index = data_set_index;
            pt.row = row;
            pt
        })
        .collect();
    DatasetSeries {
        data_set_index,
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

fn rule_shared(name: &str, initial: Option<f64>, lower: Option<f64>, upper: Option<f64>) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::SharedGlobal, initial_override: initial, lower_bound: lower, upper_bound: upper }
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

fn assert_converged(status: FitStatus, ctx: &str) {
    assert!(matches!(status, FitStatus::Converged), "{ctx}: expected Converged, got {status:?}");
}

fn find_ds_param(res: &NonlinearFitResult, ds: usize, name: &str) -> f64 {
    let dataset = res.datasets.get(ds);
    assert!(dataset.is_some(), "missing dataset {ds}");
    let Some(dataset) = dataset else {
        return f64::NAN;
    };
    let idx = dataset.param_names.iter().position(|n| n == name);
    assert!(idx.is_some(), "missing param `{name}` in dataset {ds}");
    let Some(idx) = idx else {
        return f64::NAN;
    };
    dataset.params[idx]
}

fn find_derived(res: &NonlinearFitResult, ds: usize, name: &str) -> f64 {
    let scalars = res.derived_scalars.get(ds);
    assert!(scalars.is_some(), "missing derived scalars for dataset {ds}");
    let Some(scalars) = scalars else {
        return f64::NAN;
    };
    let found = scalars.iter().find(|s| s.name == name);
    assert!(found.is_some(), "missing derived scalar `{name}` in dataset {ds}");
    let Some(found) = found else {
        return f64::NAN;
    };
    found.value
}

fn assert_grad_matches_fd(model: &dyn ParametricModel, x: &[f64], params: &[f64], ds: usize, abs_tol: f64, rel_tol: f64, ctx: &str) {
    let p_len = model.param_count();
    let mut analytic = vec![0.0_f64; p_len];
    assert!(model.grad(x, params, ds, &mut analytic), "{ctx}: model.grad returned false");
    for idx in 0..p_len {
        let h = 1e-6 * params[idx].abs().max(1.0);
        let mut p_plus = params.to_vec();
        let mut p_minus = params.to_vec();
        p_plus[idx] += h;
        p_minus[idx] -= h;
        if let (Some(yp), Some(ym)) = (model.eval(x, &p_plus, ds), model.eval(x, &p_minus, ds)) {
            let fd = (yp - ym) / (2.0 * h);
            assert_approx_eq(analytic[idx], fd, abs_tol, rel_tol, &format!("{ctx} d/dp[{idx}] ({})", model.param_names()[idx]));
        }
    }
}

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
        assert!(catalog.iter().any(|e| e.id == id && !e.name.contains("--")), "catalog check for {id}");
        let model_opt = get_builtin_model(&id);
        assert!(model_opt.is_some(), "missing builtin model {id}");
        let Some(model) = model_opt else {
            return;
        };
        assert_eq!(model.param_count(), deg + 2);
        assert_eq!(model.param_names().last().map(String::as_str), Some("XCenter"));

        let mut test_params = true_coeffs[..=deg].to_vec();
        test_params.push(x_center);
        assert_grad_matches_fd(model.as_ref(), &[2009.0], &test_params, 0, 1e-6, 1e-6, &id);

        let ys: Vec<f64> = xs
            .iter()
            .map(|&x| {
                let dx = x - x_center;
                (0..=deg).fold(0.0_f64, |acc, k| acc + true_coeffs[k] * dx.powi(k as i32))
            })
            .collect();

        let cfg = SolverConfig { parameter_rules: vec![rule_const("XCenter", x_center)], max_iters: 1000, ..SolverConfig::default() };
        let res = fit_nonlinear(model.as_ref(), &[series(&id, xs.clone(), ys.clone())], &HookContext::default(), &cfg);
        assert!(res.is_ok(), "fit failed for {id}");
        let Ok(res) = res else {
            return;
        };
        assert_converged(res.status, &id);
        for (k, &expected) in true_coeffs.iter().enumerate().take(deg + 1) {
            let pname = format!("B{k}");
            assert_approx_eq(find_ds_param(&res, 0, &pname), expected, 1e-5, 1e-5, &format!("{id} {pname}"));
        }

        let res_free = fit_nonlinear(
            model.as_ref(),
            &[series(&format!("{id}_free"), xs.clone(), ys.clone())],
            &HookContext::default(),
            &SolverConfig::default(),
        );
        assert!(res_free.is_ok(), "unconstrained fit failed for {id}");
        let Ok(res_free) = res_free else {
            return;
        };
        assert!(matches!(res_free.status, FitStatus::Converged | FitStatus::Ambiguous), "{id} unconstrained stalled: {:?}", res_free.status);
        for (y_hat, y_obs) in res_free.datasets[0].y_hat.iter().zip(ys.iter()) {
            assert_approx_eq(*y_hat, *y_obs, 1e-4, 1e-5, &format!("{id} predicted curve"));
        }
    }
}

#[test]
fn semi_log_line_oracle() {
    let model_opt = get_builtin_model("semi_log_line");
    assert!(model_opt.is_some(), "missing semi_log_line");
    let Some(semi) = model_opt else {
        return;
    };
    assert_eq!(semi.eval(&[0.0], &[2.0, 1.0], 0), None);
    assert_eq!(semi.eval(&[-1.0], &[2.0, 1.0], 0), None);
    assert_grad_matches_fd(semi.as_ref(), &[12.5], &[18.4, 7.25], 0, 1e-8, 1e-8, "semi_log_line");

    let xs: Vec<f64> = vec![0.5, 1.0, 2.5, 10.0, 25.0, 100.0, 250.0, 1000.0];
    let (true_slope, true_int) = (24.5, -11.2);
    let ys: Vec<f64> = xs.iter().map(|&x| true_slope * x.log10() + true_int).collect();
    let res = fit_nonlinear(semi.as_ref(), &[series("SemiLog", xs, ys)], &HookContext::default(), &SolverConfig::default());
    assert!(res.is_ok(), "semi_log_line fit failed");
    let Ok(res) = res else {
        return;
    };
    assert_converged(res.status, "semi_log_line");
    assert_approx_eq(find_ds_param(&res, 0, "Slope"), true_slope, 1e-8, 1e-8, "semi_log Slope");
    assert_approx_eq(find_ds_param(&res, 0, "YIntercept"), true_int, 1e-8, 1e-8, "semi_log YIntercept");
}

#[test]
fn log_log_line_and_probit_line_oracles() {
    let loglog_opt = get_builtin_model("log_log_line");
    assert!(loglog_opt.is_some(), "missing log_log_line");
    let Some(loglog) = loglog_opt else {
        return;
    };
    assert_eq!(loglog.eval(&[0.0], &[1.2, 0.5], 0), None);
    assert_grad_matches_fd(loglog.as_ref(), &[8.0], &[1.35, -0.4], 0, 1e-6, 1e-6, "log_log_line");

    let xs_pos: Vec<f64> = vec![0.5, 1.0, 2.5, 10.0, 25.0, 100.0, 250.0, 1000.0];
    let (true_slope_ll, true_int_ll) = (1.42, 0.75);
    let ys_ll: Vec<f64> = xs_pos.iter().map(|&x| 10.0_f64.powf(true_slope_ll * x.log10() + true_int_ll)).collect();
    let res_ll = fit_nonlinear(loglog.as_ref(), &[series("LogLog", xs_pos, ys_ll)], &HookContext::default(), &SolverConfig::default());
    assert!(res_ll.is_ok(), "log_log_line fit failed");
    let Ok(res_ll) = res_ll else {
        return;
    };
    assert_converged(res_ll.status, "log_log_line");
    assert_approx_eq(find_ds_param(&res_ll, 0, "Slope"), true_slope_ll, 1e-6, 1e-6, "log_log Slope");
    assert_approx_eq(find_ds_param(&res_ll, 0, "LogYIntercept"), true_int_ll, 1e-6, 1e-6, "log_log LogYIntercept");

    let probit_opt = get_builtin_model("probit_line");
    assert!(probit_opt.is_some(), "missing probit_line");
    let Some(probit) = probit_opt else {
        return;
    };
    let (true_slope_p, true_int_p) = (1.6, 4.8);
    assert_grad_matches_fd(probit.as_ref(), &[-2.8], &[true_slope_p, true_int_p], 0, 1e-6, 1e-6, "probit_line");
    assert_approx_eq(
        probit.eval(&[-true_int_p / true_slope_p], &[true_slope_p, true_int_p], 0).unwrap_or(f64::NAN),
        50.0,
        1e-10,
        1e-10,
        "probit 50%",
    );

    let xs_p: Vec<f64> = (-50..=-10).step_by(4).map(|k| k as f64 * 0.1).collect();
    let ys_p: Vec<f64> = xs_p.iter().map(|&x| norm_cdf(true_slope_p * x + true_int_p) * 100.0).collect();
    let res_p = fit_nonlinear(probit.as_ref(), &[series("Probit", xs_p, ys_p)], &HookContext::default(), &SolverConfig::default());
    assert!(res_p.is_ok(), "probit_line fit failed");
    let Ok(res_p) = res_p else {
        return;
    };
    assert_converged(res_p.status, "probit_line");
    assert_approx_eq(find_ds_param(&res_p, 0, "Slope"), true_slope_p, 1e-6, 1e-6, "probit Slope");
    assert_approx_eq(find_ds_param(&res_p, 0, "Intercept"), true_int_p, 1e-6, 1e-6, "probit Intercept");
}

#[test]
fn dose_response_3pl_4pl_analytical_grad_oracles() {
    let log_cases = [
        ("log_agonist_vs_response_3pl", vec![12.0, 185.0, -7.2]),
        ("log_agonist_vs_response_4pl", vec![12.0, 185.0, -7.2, 1.45]),
        ("log_inhibitor_vs_response_3pl", vec![12.0, 185.0, -6.8]),
        ("log_inhibitor_vs_response_4pl", vec![12.0, 185.0, -6.8, 1.25]),
        ("log_agonist_vs_response_normalized", vec![-7.0]),
        ("log_agonist_vs_response_normalized_var", vec![-7.0, 1.6]),
        ("log_inhibitor_vs_response_normalized", vec![-6.5]),
        ("log_inhibitor_vs_response_normalized_var", vec![-6.5, 1.3]),
        ("one_site_fit_logic50", vec![10.0, 500.0, -8.1]),
    ];
    for (id, params) in log_cases {
        let model_opt = get_builtin_model(id);
        assert!(model_opt.is_some(), "missing {id}");
        let Some(m) = model_opt else {
            return;
        };
        for &x in &[-10.0, -7.0, -4.0] {
            assert_grad_matches_fd(m.as_ref(), &[x], &params, 0, 1e-6, 1e-5, &format!("{id} x={x}"));
        }
        let mut extremum_grad = vec![0.0_f64; params.len()];
        assert!(m.grad(&[500.0], &params, 0, &mut extremum_grad));
        assert!(extremum_grad.iter().all(|v| v.is_finite()), "{id} non-finite tail grad at x=500");
    }

    let linear_cases = [
        ("agonist_vs_response_3pl", vec![10.0, 220.0, 2.5e-7]),
        ("agonist_vs_response_4pl", vec![10.0, 220.0, 2.5e-7, 1.35]),
        ("inhibitor_vs_response_3pl", vec![15.0, 195.0, 4.0e-8]),
        ("inhibitor_vs_response_4pl", vec![15.0, 195.0, 4.0e-8, 1.15]),
    ];
    for (id, params) in linear_cases {
        let model_opt = get_builtin_model(id);
        assert!(model_opt.is_some(), "missing {id}");
        let Some(m) = model_opt else {
            return;
        };
        assert_grad_matches_fd(m.as_ref(), &[0.0], &params, 0, 1e-6, 1e-6, &format!("{id} c=0"));
        for &c in &[1e-9, params[2], 5.0 * params[2]] {
            assert_grad_matches_fd(m.as_ref(), &[c], &params, 0, 1e-5, 1e-5, &format!("{id} c={c:e}"));
        }
    }

    let m4_opt = get_builtin_model("log_agonist_vs_response_4pl");
    assert!(m4_opt.is_some(), "missing log_agonist_vs_response_4pl");
    let Some(m4) = m4_opt else {
        return;
    };
    let log_x: Vec<f64> = (-11..=-4).map(|k| k as f64).collect();
    let batch: Vec<DatasetSeries> = (0..96)
        .map(|i| {
            let bot = 10.0 + (i % 5) as f64;
            let top = 110.0 + (i % 7) as f64 * 5.0;
            let ec = -8.0 + ((i % 9) as f64) * 0.2;
            let hill = 0.9 + ((i % 4) as f64) * 0.2;
            let ys = log_x.iter().map(|&x| bot + (top - bot) / (1.0 + 10.0_f64.powf((ec - x) * hill))).collect();
            series_idx(i, &format!("Well_{i}"), log_x.clone(), ys)
        })
        .collect();
    let batch_res = fit_nonlinear(m4.as_ref(), &batch, &HookContext::default(), &SolverConfig::default());
    assert!(batch_res.is_ok(), "96-curve batch fit failed");
    let Ok(batch_res) = batch_res else {
        return;
    };
    assert_converged(batch_res.status, "96-curve 4PL");
    assert_eq!(batch_res.datasets.len(), 96);
    assert_approx_eq(find_ds_param(&batch_res, 0, "LogEC50"), -8.0, 1e-5, 1e-5, "well 0 LogEC50");
}

#[test]
fn receptor_binding_global_ns_and_ligand_depletion_oracles() {
    let m_glob_opt = get_builtin_model("one_site_total_and_ns_global");
    assert!(m_glob_opt.is_some(), "missing one_site_total_and_ns_global");
    let Some(m_glob) = m_glob_opt else {
        return;
    };
    let (bmax, kd, ns, bg) = (1800.0, 3.5, 42.0, 85.0);
    assert_grad_matches_fd(m_glob.as_ref(), &[2.5], &[bmax, kd, ns, bg], 0, 1e-6, 1e-6, "global total ds0");
    assert_grad_matches_fd(m_glob.as_ref(), &[2.5], &[bmax, kd, ns, bg], 1, 1e-6, 1e-6, "global ns ds1");

    let hot_x = vec![0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let y_total: Vec<f64> = hot_x.iter().map(|&x| bmax * x / (kd + x) + ns * x + bg).collect();
    let y_ns: Vec<f64> = hot_x.iter().map(|&x| ns * x + bg).collect();
    let cfg_glob = SolverConfig {
        parameter_rules: vec![
            rule_shared("Bmax", Some(1500.0), Some(0.0), None),
            rule_shared("Kd", Some(2.0), Some(1e-6), None),
            rule_shared("NS", Some(20.0), Some(0.0), None),
            rule_shared("Background", Some(50.0), None, None),
        ],
        ..SolverConfig::default()
    };
    let res_glob = fit_nonlinear(
        m_glob.as_ref(),
        &[series_idx(0, "Hot", hot_x.clone(), y_total), series_idx(1, "[NS]", hot_x, y_ns)],
        &HookContext::default(),
        &cfg_glob,
    );
    assert!(res_glob.is_ok(), "global total+ns fit failed");
    let Ok(res_glob) = res_glob else {
        return;
    };
    assert_converged(res_glob.status, "one_site_total_and_ns_global");
    assert_approx_eq(find_ds_param(&res_glob, 0, "Bmax"), bmax, 1e-5, 1e-6, "global Bmax");
    assert_approx_eq(find_ds_param(&res_glob, 0, "Kd"), kd, 1e-6, 1e-6, "global Kd");
    assert_approx_eq(find_ds_param(&res_glob, 1, "NS"), ns, 1e-6, 1e-6, "global NS");
    assert_approx_eq(find_ds_param(&res_glob, 1, "Background"), bg, 1e-6, 1e-6, "global Background");

    let m_dep_opt = get_builtin_model("one_site_specific_ligand_depletion");
    assert!(m_dep_opt.is_some(), "missing one_site_specific_ligand_depletion");
    let Some(m_dep) = m_dep_opt else {
        return;
    };
    let (rt_true, kd_true, ns_true) = (25.0, 3.0, 0.08);
    assert_grad_matches_fd(m_dep.as_ref(), &[12.0], &[rt_true, kd_true, ns_true], 0, 1e-6, 1e-6, "ligand depletion");

    for &lt in &[0.0, 2.0, 15.0, 80.0] {
        let morrison = 0.5 * ((rt_true + lt + kd_true) - ((rt_true + lt + kd_true).powi(2) - 4.0 * rt_true * lt).sqrt());
        assert_approx_eq(m_dep.eval(&[lt], &[rt_true, kd_true, 0.0], 0).unwrap_or(f64::NAN), morrison, 1e-12, 1e-12, "Morrison NS=0 limit");
    }

    let x_tot = vec![1.0, 3.0, 7.5, 15.0, 25.0, 40.0, 70.0, 120.0];
    let y_dep: Vec<f64> = x_tot.iter().map(|&lt| swillens_specific_bound(lt, rt_true, kd_true, ns_true)).collect();
    let cfg_dep = SolverConfig {
        parameter_rules: vec![
            rule_local("Rt", Some(20.0), Some(0.0), None),
            rule_local("Kd", Some(5.0), Some(1e-6), None),
            rule_const("NS", ns_true),
        ],
        ..SolverConfig::default()
    };
    let res_dep = fit_nonlinear(m_dep.as_ref(), &[series("Depleted", x_tot, y_dep)], &HookContext::default(), &cfg_dep);
    assert!(res_dep.is_ok(), "depletion fit failed");
    let Ok(res_dep) = res_dep else {
        return;
    };
    assert_converged(res_dep.status, "one_site_specific_ligand_depletion");
    assert_approx_eq(find_ds_param(&res_dep, 0, "Rt"), rt_true, 1e-6, 1e-6, "depletion Rt");
    assert_approx_eq(find_ds_param(&res_dep, 0, "Kd"), kd_true, 1e-6, 1e-6, "depletion Kd");
}

#[test]
fn allosteric_saturation_and_homologous_competition_oracles() {
    let m_allo_opt = get_builtin_model("allosteric_modulator_saturation");
    assert!(m_allo_opt.is_some(), "missing allosteric_modulator_saturation");
    let Some(m_allo) = m_allo_opt else {
        return;
    };
    let (bmax, kd, alpha, kb) = (250.0, 4.0, 5.0, 12.0);
    assert_grad_matches_fd(m_allo.as_ref(), &[3.0], &[bmax, kd, alpha, kb, 15.0], 0, 1e-6, 1e-6, "allosteric saturation");

    let mod_concs = [0.0, 4.0, 16.0, 64.0];
    let x_rad = vec![0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let datasets: Vec<DatasetSeries> = mod_concs
        .iter()
        .enumerate()
        .map(|(ds_i, &mc)| {
            let ys = x_rad
                .iter()
                .map(|&x| {
                    let kd_app = kd * (1.0 + mc / kb) / (1.0 + alpha * mc / kb);
                    bmax * x / (kd_app + x)
                })
                .collect();
            series_idx(ds_i, &format!("{mc} nM"), x_rad.clone(), ys)
        })
        .collect();

    let cfg_allo = SolverConfig {
        parameter_rules: vec![
            rule_shared("Bmax", Some(220.0), Some(1.0), None),
            rule_shared("Kd", Some(3.0), Some(1e-6), None),
            rule_shared("alpha", Some(2.0), Some(0.0), None),
            rule_shared("Kb", Some(10.0), Some(1e-6), None),
            ParameterRule {
                name: "ModConc".to_owned(),
                scope: ParameterScope::ColumnTitleConstant,
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let res_allo = fit_nonlinear(m_allo.as_ref(), &datasets, &HookContext::default(), &cfg_allo);
    assert!(res_allo.is_ok(), "allosteric saturation fit failed");
    let Ok(res_allo) = res_allo else {
        return;
    };
    assert_converged(res_allo.status, "allosteric_modulator_saturation");
    assert_approx_eq(find_ds_param(&res_allo, 0, "Bmax"), bmax, 1e-5, 1e-6, "Allo Bmax");
    assert_approx_eq(find_ds_param(&res_allo, 0, "Kd"), kd, 1e-6, 1e-6, "Allo Kd");
    assert_approx_eq(find_ds_param(&res_allo, 0, "alpha"), alpha, 1e-5, 1e-5, "Allo alpha");
    assert_approx_eq(find_ds_param(&res_allo, 0, "Kb"), kb, 1e-5, 1e-5, "Allo Kb");

    let m_hom_opt = get_builtin_model("homologous_one_site_competition");
    assert!(m_hom_opt.is_some(), "missing homologous_one_site_competition");
    let Some(m_hom) = m_hom_opt else {
        return;
    };
    let (bmax_h, kd_h, ns_h, hot_h) = (2400.0, 4.5e-9, 120.0, 2.0e-9);
    assert_grad_matches_fd(m_hom.as_ref(), &[-8.5], &[bmax_h, kd_h, ns_h, hot_h], 0, 1e-5, 1e-5, "homologous competition");

    let log_cold: Vec<f64> = (-24..=-10).map(|i| i as f64 * 0.5).collect();
    let y_hom: Vec<f64> = log_cold.iter().map(|&lx| bmax_h * hot_h / (hot_h + 10.0_f64.powf(lx) + kd_h) + ns_h).collect();
    let cfg_hom = SolverConfig {
        parameter_rules: vec![
            rule_local("Bmax", Some(2000.0), Some(0.0), None),
            rule_local("Kd", Some(5.0e-9), Some(1e-18), None),
            rule_local("NS", Some(100.0), None, None),
            rule_const("HotConc", hot_h),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let res_hom = fit_nonlinear(m_hom.as_ref(), &[series("Homologous", log_cold, y_hom)], &HookContext::default(), &cfg_hom);
    assert!(res_hom.is_ok(), "homologous fit failed");
    let Ok(res_hom) = res_hom else {
        return;
    };
    assert_converged(res_hom.status, "homologous_one_site_competition");
    assert_approx_eq(find_ds_param(&res_hom, 0, "Bmax"), bmax_h, 1e-3, 1e-5, "Hom Bmax");
    assert_approx_eq(find_ds_param(&res_hom, 0, "Kd"), kd_h, 1e-15, 1e-5, "Hom Kd");
    assert_approx_eq(find_ds_param(&res_hom, 0, "NS"), ns_h, 1e-4, 1e-5, "Hom NS");
}

#[test]
fn motulsky_mahan_competitive_kinetics_oracle() {
    let m_mm_opt = get_builtin_model("motulsky_mahan_kinetics");
    assert!(m_mm_opt.is_some(), "missing motulsky_mahan_kinetics");
    let Some(m_mm) = m_mm_opt else {
        return;
    };
    let (bmax, kon1, koff1, kon2, koff2, l, i_conc) = (100.0, 5.0e6, 0.15, 2.5e6, 0.04, 1.0e-8, 4.0e-8);
    let params = [bmax, kon1, koff1, kon2, koff2, l, i_conc];

    let dt = 0.002_f64;
    let mut rl = 0.0_f64;
    let mut ri = 0.0_f64;
    let deriv = |y1: f64, y2: f64| -> (f64, f64) {
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
            let exact = m_mm.eval(&[t], &params, 0).unwrap_or(f64::NAN);
            assert_approx_eq(exact, rl, 1e-8, 1e-8, &format!("Motulsky-Mahan vs RK4 t={t}"));
        }
    }

    let kd = koff1 / kon1;
    let ki = koff2 / kon2;
    let eq_expected = bmax * l / (l + kd * (1.0 + i_conc / ki));
    assert_approx_eq(m_mm.eval(&[1e5], &params, 0).unwrap_or(f64::NAN), eq_expected, 1e-8, 1e-8, "Motulsky-Mahan equilibrium limit");

    let k_eq = kon1 * l + koff1;
    let deg_params = [bmax, kon1, koff1, 0.0, k_eq, l, 0.0];
    let t_deg = 6.0;
    let expected_deg = (bmax * kon1 * l / k_eq) * (1.0 - (-k_eq * t_deg).exp());
    assert_approx_eq(m_mm.eval(&[t_deg], &deg_params, 0).unwrap_or(f64::NAN), expected_deg, 1e-9, 1e-9, "degenerate Ka==Kb limit");

    let ts = vec![0.0, 0.5, 1.0, 2.0, 4.0, 7.0, 12.0, 20.0, 35.0, 60.0, 100.0, 160.0];
    let ys: Vec<f64> = ts.iter().map(|&t| m_mm.eval(&[t], &params, 0).unwrap_or(f64::NAN)).collect();
    let cfg_mm = SolverConfig {
        parameter_rules: vec![
            rule_const("Bmax", bmax),
            rule_const("kon1", kon1),
            rule_const("koff1", koff1),
            rule_local("kon2", Some(1.0e6), Some(1.0), None),
            rule_local("koff2", Some(0.08), Some(1e-6), None),
            rule_const("L", l),
            rule_const("I", i_conc),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let res_mm = fit_nonlinear(m_mm.as_ref(), &[series("CompKinetics", ts, ys)], &HookContext::default(), &cfg_mm);
    assert!(res_mm.is_ok(), "Motulsky-Mahan fit failed");
    let Ok(res_mm) = res_mm else {
        return;
    };
    assert_converged(res_mm.status, "motulsky_mahan_kinetics");
    assert_approx_eq(find_ds_param(&res_mm, 0, "kon2"), kon2, 1.0, 1e-5, "MM kon2");
    assert_approx_eq(find_ds_param(&res_mm, 0, "koff2"), koff2, 1e-7, 1e-6, "MM koff2");
}

#[test]
fn dsl_initial_values_and_transforms_end_to_end() {
    let source = r#"
[Equation]
Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))

[InitialValues]
Bottom = ymin
Top = YMAX
SpanInit = SPAN
HalfCheck = YMID
XRangeCheck = XMAX - XMIN
XMidCheck = XMID
LoX = XATYMIN
HiX = XATYMAX
LogEC50 = XATY50
HillSlope = SIGN + 0 * (SpanInit + HalfCheck + XRangeCheck + XMidCheck + LoX + HiX)

[Transforms]
EC50 = 10^LogEC50
Span = Top - Bottom
Ratio = EC50 / Span
"#;

    let compiled = CompiledDslModel::compile("custom_4pl", source);
    assert!(compiled.is_ok(), "failed compiling [InitialValues]/[Transforms] DSL");
    let Ok(model) = compiled else {
        return;
    };
    assert_eq!(model.param_names(), &["Bottom", "Top", "LogEC50", "HillSlope"]);

    let (bot_t, top_t, log_ec50_t, hill_t) = (320.0, 4520.0, -8.25, -1.35);
    let log_x: Vec<f64> = (-24..=-10).map(|i| i as f64 * 0.5).collect();
    let ys: Vec<f64> = log_x.iter().map(|&x| bot_t + (top_t - bot_t) / (1.0 + 10.0_f64.powf((log_ec50_t - x) * hill_t))).collect();

    let s = series("Inhibitor", log_x, ys);
    let init = model.initial_guess(&s.points);
    assert!(init[0] > 300.0 && init[0] < 350.0, "Bottom seed should reflect YMIN: {}", init[0]);
    assert!(init[1] > 4500.0, "Top seed should reflect YMAX: {}", init[1]);
    assert!((init[2] - log_ec50_t).abs() < 0.6, "LogEC50 seed should reflect XATY50: {}", init[2]);
    assert_approx_eq(init[3], -1.0, 1e-12, 1e-12, "HillSlope seed SIGN == -1");

    let res = fit_nonlinear(&model, &[s], &HookContext::default(), &SolverConfig::default());
    assert!(res.is_ok(), "DSL fit with [InitialValues]/[Transforms] failed");
    let Ok(res) = res else {
        return;
    };
    assert_converged(res.status, "DSL [InitialValues]+[Transforms]");
    assert_approx_eq(find_ds_param(&res, 0, "Bottom"), bot_t, 1e-4, 1e-5, "DSL Bottom");
    assert_approx_eq(find_ds_param(&res, 0, "Top"), top_t, 1e-4, 1e-5, "DSL Top");
    assert_approx_eq(find_ds_param(&res, 0, "LogEC50"), log_ec50_t, 1e-5, 1e-5, "DSL LogEC50");
    assert_approx_eq(find_ds_param(&res, 0, "HillSlope"), hill_t, 1e-5, 1e-5, "DSL HillSlope");

    let expected_ec50 = 10.0_f64.powf(log_ec50_t);
    let expected_span = top_t - bot_t;
    assert_approx_eq(find_derived(&res, 0, "EC50"), expected_ec50, 1e-13, 1e-5, "Transform EC50");
    assert_approx_eq(find_derived(&res, 0, "Span"), expected_span, 1e-4, 1e-5, "Transform Span");
    assert_approx_eq(find_derived(&res, 0, "Ratio"), expected_ec50 / expected_span, 1e-16, 1e-5, "Chained Transform Ratio");

    let scoped_init_dsl = "[Equation]\nY = A * X + B\n[InitialValues]\n<A> A = 10\n<B> A = 20\nB = YMIN";
    let m_scoped = CompiledDslModel::compile("scoped_init", scoped_init_dsl);
    assert!(m_scoped.is_ok(), "scoped initial values compile failed");
    let Ok(m_scoped) = m_scoped else {
        return;
    };
    let ds_a = series_idx(0, "A", vec![0.0, 1.0], vec![5.0, 15.0]);
    let ds_b = series_idx(1, "B", vec![0.0, 1.0], vec![7.0, 27.0]);
    assert_approx_eq(m_scoped.initial_guess(&ds_a.points)[0], 10.0, 1e-12, 1e-12, "<A> init A");
    assert_approx_eq(m_scoped.initial_guess(&ds_b.points)[0], 20.0, 1e-12, 1e-12, "<B> init A");
    assert_approx_eq(m_scoped.initial_guess(&ds_b.points)[1], 7.0, 1e-12, 1e-12, "<B> init B");

    assert!(CompiledDslModel::compile("bad_sec", "Y = A * X\n[Bogus]\nk = 1").is_err());
    assert!(CompiledDslModel::compile("bad_init_x", "Y = A * X\n[InitialValues]\nA = X + 1").is_err());
    assert!(CompiledDslModel::compile("bad_init_sym", "Y = A * X\n[InitialValues]\nA = UNKNOWN_STAT").is_err());
    assert!(CompiledDslModel::compile("bad_tr_x", "Y = A * X\n[Transforms]\nT = A * X").is_err());
    assert!(CompiledDslModel::compile("bad_tr_sym", "Y = A * X\n[Transforms]\nT = NO_SUCH_PARAM * 2").is_err());
    assert!(CompiledDslModel::compile("bad_tr_ds", "Y = A * X\n[Transforms]\n<A> T = A * 2").is_err());
}

#[test]
fn dsl_generalized_prefixes_and_scoped_intermediates() {
    let mm_inh_dsl = r#"
<A> I = 0
<B> I = 5
<C> I = 15
<D> I = 40
Vapp = Vmax / (1 + I / alphaKi)
Kapp = Km / (1 + I / alphaKi)
Y = Vapp * X / (Kapp + X)
"#;
    let compiled = CompiledDslModel::compile("global_uncomp_scoped", mm_inh_dsl);
    assert!(compiled.is_ok(), "scoped DSL compile failed");
    let Ok(m_uc) = compiled else {
        return;
    };
    assert_eq!(m_uc.param_names(), &["Vmax", "alphaKi", "Km"], "dataset-local I/Vapp/Kapp must not leak");
    assert_eq!(m_uc.eval(&[10.0], &[100.0, 10.0, 4.0], 4), None);

    let (vmax_t, km_t, aki_t) = (140.0, 4.5, 12.0);
    let concs = [0.0, 5.0, 15.0, 40.0];
    let s_vals = vec![0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let ds_series: Vec<DatasetSeries> = concs
        .iter()
        .enumerate()
        .map(|(k, &ci)| {
            let ys = s_vals.iter().map(|&s| (vmax_t / (1.0 + ci / aki_t)) * s / (km_t / (1.0 + ci / aki_t) + s)).collect();
            series_idx(k, &format!("I={ci}"), s_vals.clone(), ys)
        })
        .collect();
    let cfg = SolverConfig {
        parameter_rules: vec![
            rule_shared("Vmax", Some(100.0), Some(1.0), None),
            rule_shared("alphaKi", Some(8.0), Some(1e-3), None),
            rule_shared("Km", Some(3.0), Some(1e-3), None),
        ],
        ..SolverConfig::default()
    };
    let res_uc = fit_nonlinear(&m_uc, &ds_series, &HookContext::default(), &cfg);
    assert!(res_uc.is_ok(), "scoped global uncomp fit failed");
    let Ok(res_uc) = res_uc else {
        return;
    };
    assert_converged(res_uc.status, "scoped DSL global uncomp");
    assert_approx_eq(find_ds_param(&res_uc, 0, "Vmax"), vmax_t, 1e-5, 1e-6, "DSL Vmax");
    assert_approx_eq(find_ds_param(&res_uc, 0, "Km"), km_t, 1e-5, 1e-6, "DSL Km");
    assert_approx_eq(find_ds_param(&res_uc, 0, "alphaKi"), aki_t, 1e-5, 1e-6, "DSL alphaKi");

    let multi_prefix_dsl =
        "span = Top - Bottom\n<A> scale = 1\n<C> scale = 2\n<Z> scale = 3\n<30> scale = 4\n<A,C,Z,30> Y = Bottom + scale * span * X";
    let m_wide = CompiledDslModel::compile("wide_prefixes", multi_prefix_dsl);
    assert!(m_wide.is_ok(), "wide prefixes compile failed");
    let Ok(m_wide) = m_wide else {
        return;
    };
    assert_eq!(m_wide.param_names(), &["Top", "Bottom"]);
    assert_approx_eq(m_wide.eval(&[2.0], &[15.0, 5.0], 0).unwrap_or(f64::NAN), 25.0, 1e-12, 1e-12, "<A>");
    assert_approx_eq(m_wide.eval(&[2.0], &[15.0, 5.0], 2).unwrap_or(f64::NAN), 45.0, 1e-12, 1e-12, "<C>");
    assert_approx_eq(m_wide.eval(&[2.0], &[15.0, 5.0], 25).unwrap_or(f64::NAN), 65.0, 1e-12, 1e-12, "<Z>");
    assert_approx_eq(m_wide.eval(&[2.0], &[15.0, 5.0], 30).unwrap_or(f64::NAN), 85.0, 1e-12, 1e-12, "<30>");
    assert_eq!(m_wide.eval(&[2.0], &[15.0, 5.0], 1), None);

    let range_dsl = "shift = Offset * 2\n<1:3> gain = 10\n<4:5> gain = 20\n<1:5> Y = shift + gain * X";
    let m_rng = CompiledDslModel::compile("range_prefixes", range_dsl);
    assert!(m_rng.is_ok(), "range_prefixes compile failed");
    let Ok(m_rng) = m_rng else {
        return;
    };
    assert_approx_eq(m_rng.eval(&[3.0], &[1.5], 2).unwrap_or(f64::NAN), 33.0, 1e-12, 1e-12, "<2 in 1:3>");
    assert_approx_eq(m_rng.eval(&[3.0], &[1.5], 5).unwrap_or(f64::NAN), 63.0, 1e-12, 1e-12, "<5 in 4:5>");
    assert_eq!(m_rng.eval(&[3.0], &[1.5], 0), None);
    assert_eq!(m_rng.eval(&[3.0], &[1.5], 6), None);

    let deep_ok = format!("Y = {}X{}", "(".repeat(100), ")".repeat(100));
    assert!(CompiledDslModel::compile("deep_ok", &deep_ok).is_ok());
    let deep_bad = format!("Y = {}X{}", "(".repeat(200), ")".repeat(200));
    assert!(CompiledDslModel::compile("deep_bad", &deep_bad).is_err());
    assert!(CompiledDslModel::compile("exp_ok", "<0:9999> Y = X").is_ok());
    assert!(CompiledDslModel::compile("exp_bad", "<0:10000> Y = X").is_err());

    let info_res = InfoSheet::new_preseeded("assay", "Radioligand Info");
    assert!(info_res.is_ok(), "InfoSheet::new_preseeded failed");
    let Ok(mut info) = info_res else {
        return;
    };
    assert!(info.set_constant("HotConc", ConstantValue::Number(2.0), None).is_ok());
    let infos = [info];
    let hooks = HookContext { info_sheets: &infos, ..HookContext::default() };
    let m_hom_opt = get_builtin_model("homologous_one_site_competition");
    assert!(m_hom_opt.is_some(), "missing homologous_one_site_competition");
    let Some(m_hom) = m_hom_opt else {
        return;
    };
    let lx: Vec<f64> = (-3..=4).map(|k| k as f64).collect();
    let yh: Vec<f64> = lx.iter().map(|&x| 500.0 * 2.0 / (2.0 + 10.0_f64.powf(x) + 3.0) + 25.0).collect();
    let cfg_hook = SolverConfig {
        parameter_rules: vec![
            rule_local("Bmax", Some(450.0), Some(0.0), None),
            rule_local("Kd", Some(4.0), Some(1e-6), None),
            rule_local("NS", Some(20.0), None, None),
            ParameterRule {
                name: "HotConc".to_owned(),
                scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                    info_sheet_id: Some(SheetId::new("assay")),
                    constant_name: "HotConc".to_owned(),
                }),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ],
        ..SolverConfig::default()
    };
    let res_hook = fit_nonlinear(m_hom.as_ref(), &[series("HomHook", lx, yh)], &hooks, &cfg_hook);
    assert!(res_hook.is_ok(), "homologous hook fit failed");
    let Ok(res_hook) = res_hook else {
        return;
    };
    assert_converged(res_hook.status, "homologous InfoConstant hook");
    assert_approx_eq(find_ds_param(&res_hook, 0, "Kd"), 3.0, 1e-5, 1e-5, "HomHook Kd");
}
