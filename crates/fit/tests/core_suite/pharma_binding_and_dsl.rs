use std::time::{Duration, Instant};

use curvebook_fit::{CompiledDslModel, EquationCategory, FitStatus, ParametricModel, get_builtin_model, list_builtin_equations};
use curvebook_testkit::assert_approx_eq;

use crate::common::{
    assert_dsl_compile_err, assert_grad_matches_fd, cfg_rules, dataset_param_val, dsl_compile, fit_with, rule_const, rule_per_ds, rule_shared_bounds,
    series_eval,
};

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
            let err = CompiledDslModel::compile("flat_200k", &flat).unwrap_err();
            assert!(start.elapsed() < Duration::from_secs(2));
            assert!(err.to_string().contains("AST") || err.to_string().contains("nodes"), "{err}");

            assert_dsl_compile_err(&format!("Y = {}X{}", "(".repeat(50_000), ")".repeat(50_000)));
            assert_dsl_compile_err(&format!("Y = {}X", "-".repeat(50_000)));
            assert_dsl_compile_err(&format!("Y = {}X{}", "IF(1,".repeat(50_000), ",0)".repeat(50_000)));

            let mut scoped_sum = String::from("<A> total = 1");
            for _ in 1..2_000 {
                scoped_sum.push_str("+1");
            }
            scoped_sum.push_str("\n<A> Y = total\n<~A> Y = X");
            let scoped = dsl_compile(&scoped_sum);
            assert_eq!((scoped.eval(&[5.0], &[], 0), scoped.eval(&[5.0], &[], 1)), (Some(2000.0), Some(5.0)));
        })
        .unwrap();
    handle.join().unwrap();
}

#[test]
fn builtin_catalog_registers_new_receptor_binding_models() {
    let infos = list_builtin_equations();
    for &(id, params) in &[
        ("two_site_total_and_ns_global", &["BmaxHi", "KdHi", "BmaxLo", "KdLo", "NS", "Background"][..]),
        ("one_site_competition_ligand_depletion", &["Rt", "HotNM", "KdNM", "LogKi", "NS"][..]),
    ] {
        let info = infos.iter().find(|e| e.id == id).unwrap();
        assert_eq!(
            (info.category, info.param_names.as_slice()),
            (EquationCategory::ReceptorBinding, params.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>().as_slice())
        );
        assert!(!info.name.contains(" -- ") && get_builtin_model(id).is_some());
    }
}

#[test]
fn two_site_total_and_ns_global_grad_and_fit_recovery() {
    let model = get_builtin_model("two_site_total_and_ns_global").unwrap();
    let true_p = [120.0, 0.45, 240.0, 22.0, 0.75, 14.0];

    for &x in &[0.02, 0.25, 1.5, 12.0, 60.0] {
        assert_grad_matches_fd(model.as_ref(), &[x], &true_p, 0, 1e-6);
        assert_grad_matches_fd(model.as_ref(), &[x], &true_p, 1, 1e-6);
    }
    for (x, p) in [
        (-0.1, true_p),
        (1.0, [120.0, 0.0, 240.0, 22.0, 0.75, 14.0]),
        (1.0, [120.0, 0.45, 240.0, -1.0, 0.75, 14.0]),
        (1.0, [120.0, 0.45, 240.0, 22.0, f64::NAN, 14.0]),
    ] {
        assert!(model.eval(&[x], &p, 0).is_none());
    }

    let xs = [0.05, 0.15, 0.4, 1.0, 2.5, 6.0, 15.0, 35.0, 80.0, 160.0];
    let datasets = [series_eval(model.as_ref(), 0, "Total", &xs, &true_p), series_eval(model.as_ref(), 1, "Nonspecific", &xs, &true_p)];

    let cfg_def = cfg_rules(vec![
        rule_shared_bounds("BmaxHi", None, Some(0.0), None),
        rule_shared_bounds("KdHi", None, Some(1e-12), None),
        rule_shared_bounds("BmaxLo", None, Some(0.0), None),
        rule_shared_bounds("KdLo", None, Some(1e-12), None),
        rule_shared_bounds("NS", None, Some(0.0), None),
        rule_shared_bounds("Background", None, None, None),
    ]);
    let res_def = fit_with(model.as_ref(), &datasets, &cfg_def);
    assert_eq!(res_def.status, FitStatus::Converged);

    let (mut b_hi, mut k_hi, mut b_lo, mut k_lo) = (
        dataset_param_val(&res_def, 0, "BmaxHi"),
        dataset_param_val(&res_def, 0, "KdHi"),
        dataset_param_val(&res_def, 0, "BmaxLo"),
        dataset_param_val(&res_def, 0, "KdLo"),
    );
    if k_hi > k_lo {
        std::mem::swap(&mut b_hi, &mut b_lo);
        std::mem::swap(&mut k_hi, &mut k_lo);
    }
    assert_approx_eq(b_hi, true_p[0], 1e-4, 1e-4, "def BmaxHi");
    assert_approx_eq(k_hi, true_p[1], 1e-4, 1e-4, "def KdHi");
    assert_approx_eq(b_lo, true_p[2], 1e-4, 1e-4, "def BmaxLo");
    assert_approx_eq(k_lo, true_p[3], 1e-4, 1e-4, "def KdLo");
    for ds in [0, 1] {
        assert_approx_eq(dataset_param_val(&res_def, ds, "NS"), true_p[4], 1e-6, 1e-6, "def NS");
        assert_approx_eq(dataset_param_val(&res_def, ds, "Background"), true_p[5], 1e-5, 1e-5, "def Background");
    }

    let cfg_ovr = cfg_rules(vec![
        rule_shared_bounds("BmaxHi", Some(80.0), Some(0.0), None),
        rule_shared_bounds("KdHi", Some(0.2), Some(1e-12), None),
        rule_shared_bounds("BmaxLo", Some(180.0), Some(0.0), None),
        rule_shared_bounds("KdLo", Some(10.0), Some(1e-12), None),
        rule_shared_bounds("NS", Some(0.3), Some(0.0), None),
        rule_shared_bounds("Background", Some(5.0), None, None),
    ]);
    let res_ovr = fit_with(model.as_ref(), &datasets, &cfg_ovr);
    assert_eq!(res_ovr.status, FitStatus::Converged);
    for (name, exp) in [("BmaxHi", true_p[0]), ("KdHi", true_p[1]), ("BmaxLo", true_p[2]), ("KdLo", true_p[3])] {
        assert_approx_eq(dataset_param_val(&res_ovr, 0, name), exp, 1e-5, 1e-5, name);
    }
}

#[test]
fn one_site_competition_ligand_depletion_mass_balance_grad_and_fit() {
    let model = get_builtin_model("one_site_competition_ligand_depletion").unwrap();
    let sat_depl = get_builtin_model("one_site_specific_ligand_depletion").unwrap();
    let cheng_p = get_builtin_model("one_site_fit_ki").unwrap();

    for params in [[15.0, 2.0, 1.5, -8.5, 0.15], [0.5, 1.0, 2.0, -9.0, 0.05], [200.0, 50.0, 5.0, -7.2, 0.25]] {
        let [rt, hot_nm, kd_nm, log_ki, ns] = params;
        let (kd, ki) = (kd_nm * (1.0 + ns), 10.0_f64.powf(log_ki + 9.0));
        for &x in &[-12.0, -10.5, -9.0, -8.0, -6.5, -4.0] {
            let y_tot = model.eval(&[x], &params, 0).unwrap();
            let r = kd_nm * (y_tot * (1.0 + ns) - ns * hot_nm) / (hot_nm - y_tot);
            let res = r + r * hot_nm / (kd + r) + r * 10.0_f64.powf(x + 9.0) / (ki + r) - rt;
            assert!(res.abs() <= 1e-10 * rt.max(1.0), "mass-balance residual={res:.3e}");
            assert_grad_matches_fd(model.as_ref(), &[x], &params, 0, 1e-6);
        }
        let y_sat_spec = sat_depl.eval(&[hot_nm], &[rt, kd_nm, ns], 0).unwrap();
        let y_sat_tot = y_sat_spec + ns * (hot_nm - y_sat_spec) / (1.0 + ns);
        assert_approx_eq(model.eval(&[-18.0], &params, 0).unwrap(), y_sat_tot, 1e-10, 1e-6, "zero-competitor limit");
    }

    assert_grad_matches_fd(model.as_ref(), &[-9.0], &[0.5, 1.0, 2.0, -9.0, 0.0], 0, 1e-5);

    let tiny_rt = [1e-4, 2.0, 2.0, -8.0, 0.0];
    let top_no_depl = tiny_rt[0] * tiny_rt[1] / (tiny_rt[2] + tiny_rt[1]);
    for &x in &[-11.0, -9.0, -8.0, -7.5, -6.0, -4.0] {
        let y_cp = cheng_p.eval(&[x], &[top_no_depl, 0.0, tiny_rt[3], tiny_rt[1], tiny_rt[2]], 0).unwrap();
        assert_approx_eq(model.eval(&[x], &tiny_rt, 0).unwrap(), y_cp, 1e-12, 1e-4, "Cheng-Prusoff limit");
    }

    assert_approx_eq(model.eval(&[-8.0], &[0.0, 2.0, 1.0, -8.0, 0.1], 0).unwrap(), 2.0 * 0.1 / 1.1, 1e-14, 1e-14, "Rt=0 plateau");
    assert_eq!(model.eval(&[-8.0], &[10.0, 0.0, 1.0, -8.0, 0.1], 0), Some(0.0));
    for bad in [[-0.1, 2.0, 1.0, -8.0, 0.0], [10.0, -0.1, 1.0, -8.0, 0.0], [10.0, 2.0, 0.0, -8.0, 0.0], [10.0, 2.0, 1.0, -8.0, -0.1]] {
        assert!(model.eval(&[-8.0], &bad, 0).is_none());
    }

    let true_fit = [12.0, 2.5, 1.2, -8.25, 0.1];
    let xs: Vec<f64> = (-22..=-8).map(|k| k as f64 * 0.5).collect();
    let ds0 = series_eval(model.as_ref(), 0, "CompetitionDepleted", &xs, &true_fit);
    let res_def = fit_with(
        model.as_ref(),
        std::slice::from_ref(&ds0),
        &cfg_rules(vec![rule_const("HotNM", true_fit[1]), rule_const("KdNM", true_fit[2]), rule_const("NS", true_fit[4])]),
    );
    assert_eq!(res_def.status, FitStatus::Converged);
    assert_approx_eq(dataset_param_val(&res_def, 0, "Rt"), true_fit[0], 1e-5, 1e-5, "def Rt");
    assert_approx_eq(dataset_param_val(&res_def, 0, "LogKi"), true_fit[3], 1e-5, 1e-5, "def LogKi");

    let ds1 = series_eval(model.as_ref(), 1, "HighHot", &xs, &[12.0, 8.0, 1.2, -8.25, 0.1]);
    let cfg_g = cfg_rules(vec![
        rule_shared_bounds("Rt", Some(8.0), Some(0.0), None),
        rule_per_ds("HotNM", &[(0, 2.5), (1, 8.0)]),
        rule_shared_bounds("KdNM", Some(2.0), Some(1e-9), None),
        rule_shared_bounds("LogKi", Some(-7.5), None, None),
        rule_const("NS", 0.1),
    ]);
    let res_g = fit_with(model.as_ref(), &[ds0, ds1], &cfg_g);
    assert_eq!(res_g.status, FitStatus::Converged);
    assert_approx_eq(dataset_param_val(&res_g, 0, "Rt"), true_fit[0], 1e-5, 1e-5, "global Rt");
    assert_approx_eq(dataset_param_val(&res_g, 0, "KdNM"), true_fit[2], 1e-5, 1e-5, "global KdNM");
    assert_approx_eq(dataset_param_val(&res_g, 0, "LogKi"), true_fit[3], 1e-5, 1e-5, "global LogKi");
}

#[test]
fn aud_03_16_ligand_depletion_overflow_huge_ns_and_tight_binding() {
    let total_dep = get_builtin_model("one_site_total_ligand_depletion").unwrap();
    let spec_dep = get_builtin_model("one_site_specific_ligand_depletion").unwrap();

    let expected_ovf = 8e153 * 0.5 * (3.0 - 5.0_f64.sqrt());
    let y_ovf = total_dep.eval(&[8e153], &[8e153, 8e147, 0.0, 2000.0, 0.5], 0).unwrap();
    assert!(y_ovf <= 8e153);
    assert_approx_eq(y_ovf, expected_ovf, 1e140, 1e-13, "Swillens total 8e153 overflow");

    for huge_ns in [1e150_f64, 1e300] {
        let y_ns = total_dep.eval(&[1e5], &[3200.0, 2.5e-3, huge_ns, 2000.0, 0.5], 0).unwrap();
        assert!(y_ns.is_finite() && y_ns > 0.0 && y_ns <= 1e5, "NS={huge_ns}: Y={y_ns}");
        assert_approx_eq(y_ns, 1e5, 1e-6, 1e-12, &format!("Swillens total NS={huge_ns}"));
    }

    let y_tb_exact = 1e5 - 2e-6 / (1e-11 + 1e-11_f64.hypot(2e-3));
    let y_tb = total_dep.eval(&[1e5], &[1e5, 1e-17, 0.0, 2000.0, 0.5], 0).unwrap();
    assert!(y_tb < 1e5, "tight-binding must not collapse free ligand to 0");
    assert_approx_eq(y_tb, y_tb_exact, 1e-10, 1e-12, "Swillens tight-binding Kd_cpm=1e-11");

    for &(bmax, kd, ns, x) in &[(3200.0, 0.0025, 0.04, 5000.0), (500.0, 0.001, 0.0, 200.0), (1e5, 1e-17, 1.0, 1e5), (1e5, 0.01, 50.0, 2e4)] {
        let y = total_dep.eval(&[x], &[bmax, kd, ns, 2000.0, 0.5], 0).unwrap();
        let (kd_cpm, l_free) = (kd * 0.5 * 1000.0 * 2000.0, x - y);
        assert!((0.0..=x).contains(&y));
        assert_approx_eq(y, bmax * l_free / (kd_cpm + l_free) + ns * l_free, 1e-9, 1e-11, &format!("mass balance {bmax}/{kd}/{ns}/{x}"));
    }

    assert_approx_eq(spec_dep.eval(&[8e153], &[8e153, 8e153, 0.0], 0).unwrap(), expected_ovf, 1e140, 1e-13, "Swillens specific overflow");
    assert_approx_eq(spec_dep.eval(&[1e5], &[1e5, 1e-11, 0.0], 0).unwrap(), y_tb_exact, 1e-10, 1e-12, "Swillens specific tight-binding");
    assert_approx_eq(spec_dep.eval(&[1e5], &[3200.0, 2500.0, 1e150], 0).unwrap(), 1.28e-145, 1e-156, 1e-12, "Swillens specific huge NS");
}
