use std::time::{Duration, Instant};

use curvebook_doc::{ParameterRule, ParameterScope};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, EquationCategory, FitStatus, HookContext, NonlinearFitResult, ParametricModel, SolverConfig,
    fit_nonlinear, get_builtin_model, list_builtin_equations,
};

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

fn assert_approx_eq(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, ctx: &str) {
    assert!(actual.is_finite() && expected.is_finite(), "{ctx}: non-finite values actual={actual}, expected={expected}");
    let diff = (actual - expected).abs();
    let tol = abs_tol.max(rel_tol * expected.abs());
    assert!(diff <= tol, "{ctx}: actual={actual:.12e}, expected={expected:.12e}, diff={diff:.3e}, tol={tol:.3e}");
}

fn find_ds_param(res: &NonlinearFitResult, ds: usize, name: &str) -> f64 {
    let dataset = &res.datasets[ds];
    let idx = dataset.param_names.iter().position(|n| n == name);
    assert!(idx.is_some(), "missing param `{name}` in dataset {ds}");
    let Some(idx) = idx else {
        return f64::NAN;
    };
    dataset.params[idx]
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
        let yp = model.eval(x, &p_plus, ds);
        let ym = model.eval(x, &p_minus, ds);
        assert!(yp.is_some(), "{ctx}: eval failed for +{idx}");
        assert!(ym.is_some(), "{ctx}: eval failed for -{idx}");
        if let (Some(yp), Some(ym)) = (yp, ym) {
            let fd = (yp - ym) / (2.0 * h);
            assert_approx_eq(analytic[idx], fd, abs_tol, rel_tol, &format!("{ctx} d/dp[{idx}] ({})", model.param_names()[idx]));
        }
    }
}

#[test]
fn dsl_200k_flat_sum_and_50k_nested_fail_fast_without_stack_overflow() {
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut flat = String::with_capacity(800_004);
            flat.push_str("Y = 1");
            for _ in 1..200_000 {
                flat.push_str("+1");
            }
            let start = Instant::now();
            let err = CompiledDslModel::compile("flat_200k", &flat).expect_err("200k flat sum must hit MAX_AST_NODES");
            assert!(start.elapsed() < Duration::from_secs(2), "200k flat sum took too long: {:?}", start.elapsed());
            assert!(err.to_string().contains("AST") || err.to_string().contains("nodes"), "unexpected error message: {err}");

            let nested_parens = format!("Y = {}X{}", "(".repeat(50_000), ")".repeat(50_000));
            assert!(CompiledDslModel::compile("nested_parens_50k", &nested_parens).is_err());

            let nested_neg = format!("Y = {}X", "-".repeat(50_000));
            assert!(CompiledDslModel::compile("nested_neg_50k", &nested_neg).is_err());

            let mut nested_if = String::with_capacity(350_000);
            for _ in 0..50_000 {
                nested_if.push_str("IF(1,");
            }
            nested_if.push('X');
            for _ in 0..50_000 {
                nested_if.push_str(",0)");
            }
            let nested_if_src = format!("Y = {nested_if}");
            assert!(CompiledDslModel::compile("nested_if_50k", &nested_if_src).is_err());

            let mut scoped_sum = String::with_capacity(8_200);
            scoped_sum.push_str("<A> total = 1");
            for _ in 1..2_000 {
                scoped_sum.push_str("+1");
            }
            scoped_sum.push_str("\n<A> Y = total\n<~A> Y = X");
            let scoped = CompiledDslModel::compile("scoped_2k_sum", &scoped_sum).expect("2000-term scoped sum stays within MAX_AST_NODES");
            assert_eq!(scoped.eval(&[5.0], &[], 0), Some(2000.0));
            assert_eq!(scoped.eval(&[5.0], &[], 1), Some(5.0));
        })
        .unwrap();
    handle.join().unwrap();
}

#[test]
fn builtin_catalog_registers_new_receptor_binding_models() {
    let infos = list_builtin_equations();
    let two_site = infos.iter().find(|info| info.id == "two_site_total_and_ns_global").expect("missing two_site_total_and_ns_global");
    assert_eq!(two_site.category, EquationCategory::ReceptorBinding);
    assert_eq!(two_site.param_names, vec!["BmaxHi", "KdHi", "BmaxLo", "KdLo", "NS", "Background"]);
    assert!(!two_site.name.contains(" -- "));

    let comp_dep =
        infos.iter().find(|info| info.id == "one_site_competition_ligand_depletion").expect("missing one_site_competition_ligand_depletion");
    assert_eq!(comp_dep.category, EquationCategory::ReceptorBinding);
    assert_eq!(comp_dep.param_names, vec!["Rt", "HotNM", "KdNM", "LogKi", "NS"]);
    assert!(!comp_dep.name.contains(" -- "));

    assert!(get_builtin_model("two_site_total_and_ns_global").is_some());
    assert!(get_builtin_model("one_site_competition_ligand_depletion").is_some());
}

#[test]
fn two_site_total_and_ns_global_grad_and_fit_recovery() {
    let model = get_builtin_model("two_site_total_and_ns_global").expect("two_site_total_and_ns_global exists");
    let true_params = [120.0, 0.45, 240.0, 22.0, 0.75, 14.0];

    for &x in &[0.02, 0.25, 1.5, 12.0, 60.0] {
        assert_grad_matches_fd(model.as_ref(), &[x], &true_params, 0, 1e-8, 1e-6, &format!("two_site ds0 x={x}"));
        assert_grad_matches_fd(model.as_ref(), &[x], &true_params, 1, 1e-8, 1e-6, &format!("two_site ds1 x={x}"));
    }

    assert!(model.eval(&[-0.1], &true_params, 0).is_none());
    assert!(model.eval(&[1.0], &[120.0, 0.0, 240.0, 22.0, 0.75, 14.0], 0).is_none());
    assert!(model.eval(&[1.0], &[120.0, 0.45, 240.0, -1.0, 0.75, 14.0], 0).is_none());
    assert!(model.eval(&[1.0], &[120.0, 0.45, 240.0, 22.0, f64::NAN, 14.0], 0).is_none());

    let xs: Vec<f64> = vec![0.05, 0.15, 0.4, 1.0, 2.5, 6.0, 15.0, 35.0, 80.0, 160.0];
    let total_y: Vec<f64> = xs.iter().map(|&x| model.eval(&[x], &true_params, 0).unwrap()).collect();
    let ns_y: Vec<f64> = xs.iter().map(|&x| model.eval(&[x], &true_params, 1).unwrap()).collect();
    let datasets = vec![series_idx(0, "Total", xs.clone(), total_y), series_idx(1, "Nonspecific", xs, ns_y)];

    // Default initial_guess convergence (all parameters SharedGlobal across Total ds0 and NS ds1, no initial overrides).
    let cfg_default = SolverConfig {
        parameter_rules: vec![
            rule_shared("BmaxHi", None, Some(0.0), None),
            rule_shared("KdHi", None, Some(1e-12), None),
            rule_shared("BmaxLo", None, Some(0.0), None),
            rule_shared("KdLo", None, Some(1e-12), None),
            rule_shared("NS", None, Some(0.0), None),
            rule_shared("Background", None, None, None),
        ],
        ..SolverConfig::default()
    };
    let res_default =
        fit_nonlinear(model.as_ref(), &datasets, &HookContext::default(), &cfg_default).expect("global fit from default guess succeeds");
    assert_eq!(res_default.status, FitStatus::Converged);

    let mut bmax_hi = find_ds_param(&res_default, 0, "BmaxHi");
    let mut kd_hi = find_ds_param(&res_default, 0, "KdHi");
    let mut bmax_lo = find_ds_param(&res_default, 0, "BmaxLo");
    let mut kd_lo = find_ds_param(&res_default, 0, "KdLo");
    if kd_hi > kd_lo {
        std::mem::swap(&mut bmax_hi, &mut bmax_lo);
        std::mem::swap(&mut kd_hi, &mut kd_lo);
    }
    assert_approx_eq(bmax_hi, true_params[0], 1e-4, 1e-4, "default recovered BmaxHi");
    assert_approx_eq(kd_hi, true_params[1], 1e-4, 1e-4, "default recovered KdHi");
    assert_approx_eq(bmax_lo, true_params[2], 1e-4, 1e-4, "default recovered BmaxLo");
    assert_approx_eq(kd_lo, true_params[3], 1e-4, 1e-4, "default recovered KdLo");
    assert_approx_eq(find_ds_param(&res_default, 0, "NS"), true_params[4], 1e-6, 1e-6, "default recovered NS");
    assert_approx_eq(find_ds_param(&res_default, 0, "Background"), true_params[5], 1e-5, 1e-5, "default recovered Background");
    assert_approx_eq(find_ds_param(&res_default, 1, "NS"), true_params[4], 1e-6, 1e-6, "shared NS in ds1");
    assert_approx_eq(find_ds_param(&res_default, 1, "Background"), true_params[5], 1e-5, 1e-5, "shared Background in ds1");

    // Perturbed explicit overrides also recover exact parameters.
    let cfg_override = SolverConfig {
        parameter_rules: vec![
            rule_shared("BmaxHi", Some(80.0), Some(0.0), None),
            rule_shared("KdHi", Some(0.2), Some(1e-12), None),
            rule_shared("BmaxLo", Some(180.0), Some(0.0), None),
            rule_shared("KdLo", Some(10.0), Some(1e-12), None),
            rule_shared("NS", Some(0.3), Some(0.0), None),
            rule_shared("Background", Some(5.0), None, None),
        ],
        ..SolverConfig::default()
    };
    let res_override = fit_nonlinear(model.as_ref(), &datasets, &HookContext::default(), &cfg_override).expect("global fit with overrides succeeds");
    assert_eq!(res_override.status, FitStatus::Converged);
    assert_approx_eq(find_ds_param(&res_override, 0, "BmaxHi"), true_params[0], 1e-5, 1e-5, "override BmaxHi");
    assert_approx_eq(find_ds_param(&res_override, 0, "KdHi"), true_params[1], 1e-5, 1e-5, "override KdHi");
    assert_approx_eq(find_ds_param(&res_override, 0, "BmaxLo"), true_params[2], 1e-5, 1e-5, "override BmaxLo");
    assert_approx_eq(find_ds_param(&res_override, 0, "KdLo"), true_params[3], 1e-5, 1e-5, "override KdLo");
}

#[test]
fn one_site_competition_ligand_depletion_mass_balance_grad_and_fit() {
    let model = get_builtin_model("one_site_competition_ligand_depletion").expect("one_site_competition_ligand_depletion exists");
    let sat_depletion = get_builtin_model("one_site_specific_ligand_depletion").expect("one_site_specific_ligand_depletion exists");
    let cheng_prusoff = get_builtin_model("one_site_fit_ki").expect("one_site_fit_ki exists");

    // Check mass-balance identity and limiting equivalences across regimes.
    let regimes = [
        [15.0, 2.0, 1.5, -8.5, 0.15],   // strong receptor & ligand depletion + NS
        [0.5, 1.0, 2.0, -9.0, 0.05],    // moderate depletion with small NS
        [200.0, 50.0, 5.0, -7.2, 0.25], // high receptor density
    ];
    for params in &regimes {
        let [rt, hot_nm, kd_nm, log_ki, ns] = *params;
        let kd = kd_nm * (1.0 + ns);
        let ki = 10.0_f64.powf(log_ki + 9.0);
        for &x in &[-12.0, -10.5, -9.0, -8.0, -6.5, -4.0] {
            let y_tot = model.eval(&[x], params, 0).expect("eval succeeds");
            // Invert Swillens total bound Y = HotNM * (R + NS * KdNM) / (KdNM * (1 + NS) + R).
            let r = kd_nm * (y_tot * (1.0 + ns) - ns * hot_nm) / (hot_nm - y_tot);
            let i_tot = 10.0_f64.powf(x + 9.0);
            let residual = r + r * hot_nm / (kd + r) + r * i_tot / (ki + r) - rt;
            assert!(residual.abs() <= 1e-10 * rt.max(1.0), "mass-balance residual={residual:.3e} for x={x}, params={params:?}");
            assert_grad_matches_fd(model.as_ref(), &[x], params, 0, 1e-9, 1e-6, &format!("comp_depl x={x} rt={rt}"));
        }

        // Zero competitor limit matches Swillens saturation specific bound + NS * FreeHot.
        let y_no_cold = model.eval(&[-18.0], params, 0).unwrap();
        let y_sat_spec = sat_depletion.eval(&[hot_nm], &[rt, kd_nm, ns], 0).unwrap();
        let y_sat_tot = y_sat_spec + ns * (hot_nm - y_sat_spec) / (1.0 + ns);
        assert_approx_eq(y_no_cold, y_sat_tot, 1e-10, 1e-6, "zero-competitor limit matches Swillens total bound");
    }

    // NS == 0.0 boundary gradient (forward FD on NS) and zero-competitor limit.
    let zero_ns = [0.5, 1.0, 2.0, -9.0, 0.0];
    let mut grad_zero_ns = vec![0.0_f64; 5];
    assert!(model.grad(&[-9.0], &zero_ns, 0, &mut grad_zero_ns));
    let h_ns = 1e-7;
    let y0 = model.eval(&[-9.0], &zero_ns, 0).unwrap();
    let y1 = model.eval(&[-9.0], &[0.5, 1.0, 2.0, -9.0, h_ns], 0).unwrap();
    assert_approx_eq(grad_zero_ns[4], (y1 - y0) / h_ns, 1e-7, 1e-5, "NS=0 boundary forward FD");

    // Non-depletion limit (Rt = 1e-4 << KdNM = 2.0, NS = 0) matches Cheng-Prusoff.
    let tiny_rt = [1e-4, 2.0, 2.0, -8.0, 0.0];
    let top_no_depl = tiny_rt[0] * tiny_rt[1] / (tiny_rt[2] + tiny_rt[1]);
    for &x in &[-11.0, -9.0, -8.0, -7.5, -6.0, -4.0] {
        let y_comp = model.eval(&[x], &tiny_rt, 0).unwrap();
        let y_cp = cheng_prusoff.eval(&[x], &[top_no_depl, 0.0, tiny_rt[3], tiny_rt[1], tiny_rt[2]], 0).unwrap();
        assert_approx_eq(y_comp, y_cp, 1e-12, 1e-4, &format!("non-depletion Cheng-Prusoff equivalence x={x}"));
    }

    // Domain and boundary checks (Rt = 0 leaves nonspecific plateau HotNM * NS / (1 + NS)).
    let y_rt0 = model.eval(&[-8.0], &[0.0, 2.0, 1.0, -8.0, 0.1], 0).expect("Rt=0 succeeds");
    assert_approx_eq(y_rt0, 2.0 * 0.1 / 1.1, 1e-14, 1e-14, "Rt=0 nonspecific bound plateau");
    assert_eq!(model.eval(&[-8.0], &[10.0, 0.0, 1.0, -8.0, 0.1], 0), Some(0.0));
    assert!(model.eval(&[-8.0], &[-0.1, 2.0, 1.0, -8.0, 0.0], 0).is_none());
    assert!(model.eval(&[-8.0], &[10.0, -0.1, 1.0, -8.0, 0.0], 0).is_none());
    assert!(model.eval(&[-8.0], &[10.0, 2.0, 0.0, -8.0, 0.0], 0).is_none());
    assert!(model.eval(&[-8.0], &[10.0, 2.0, 1.0, -8.0, -0.1], 0).is_none());

    // Curve fit parameter recovery with strongly depleted competition curve.
    let true_fit = [12.0, 2.5, 1.2, -8.25, 0.1];
    let xs: Vec<f64> = (-22..=-8).map(|k| k as f64 * 0.5).collect(); // -11.0 .. -4.0
    let ys: Vec<f64> = xs.iter().map(|&x| model.eval(&[x], &true_fit, 0).unwrap()).collect();
    let datasets = vec![series_idx(0, "CompetitionDepleted", xs, ys)];

    let cfg_default = SolverConfig {
        parameter_rules: vec![rule_const("HotNM", true_fit[1]), rule_const("KdNM", true_fit[2]), rule_const("NS", true_fit[4])],
        ..SolverConfig::default()
    };
    let res_default =
        fit_nonlinear(model.as_ref(), &datasets, &HookContext::default(), &cfg_default).expect("depletion fit from default guess succeeds");
    assert_eq!(res_default.status, FitStatus::Converged);
    assert_approx_eq(find_ds_param(&res_default, 0, "Rt"), true_fit[0], 1e-5, 1e-5, "recovered Rt (default init)");
    assert_approx_eq(find_ds_param(&res_default, 0, "LogKi"), true_fit[3], 1e-5, 1e-5, "recovered LogKi (default init)");

    // Fit simultaneously across two radioligand concentrations (HotNM known per curve, Rt/KdNM/LogKi/NS shared).
    let params_hot2 = [12.0, 8.0, 1.2, -8.25, 0.1];
    let xs2: Vec<f64> = (-22..=-8).map(|k| k as f64 * 0.5).collect();
    let ys2: Vec<f64> = xs2.iter().map(|&x| model.eval(&[x], &params_hot2, 0).unwrap()).collect();
    let ds_global = vec![datasets[0].clone(), series_idx(1, "HighHot", xs2, ys2)];
    let cfg_global = SolverConfig {
        parameter_rules: vec![
            rule_shared("Rt", Some(8.0), Some(0.0), None),
            ParameterRule {
                name: "HotNM".to_owned(),
                scope: ParameterScope::PerDatasetConstant(vec![(0, 2.5), (1, 8.0)]),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
            rule_shared("KdNM", Some(2.0), Some(1e-9), None),
            rule_shared("LogKi", Some(-7.5), None, None),
            rule_const("NS", 0.1),
        ],
        ..SolverConfig::default()
    };
    let res_global = fit_nonlinear(model.as_ref(), &ds_global, &HookContext::default(), &cfg_global).expect("multi-hot depletion fit succeeds");
    assert_eq!(res_global.status, FitStatus::Converged);
    assert_approx_eq(find_ds_param(&res_global, 0, "Rt"), true_fit[0], 1e-5, 1e-5, "global Rt");
    assert_approx_eq(find_ds_param(&res_global, 0, "KdNM"), true_fit[2], 1e-5, 1e-5, "global KdNM");
    assert_approx_eq(find_ds_param(&res_global, 0, "LogKi"), true_fit[3], 1e-5, 1e-5, "global LogKi");
}
