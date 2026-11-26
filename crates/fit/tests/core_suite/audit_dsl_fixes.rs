use std::time::Instant;

use curvebook_fit::dsl::DslWarningKind;
use curvebook_fit::{CompiledDslModel, DslEquationKind, FitError, HookContext, ParametricModel, fit_nonlinear, integrate_rk45};

use crate::common::{assert_dsl_compile_err, cfg_fast_rules, dsl_compile, fit_with, param_ds_val, rule_shared_bounds, rule_shared_init, series_xy};

#[test]
fn audit_42_ode_preserves_dataset_prefixed_ic_and_t0_in_global_fit() {
    let model = dsl_compile("<A> Y0 = A0\n<B> Y(4) = B0\ndY/dX = -k * Y");
    assert_eq!(model.kind(), DslEquationKind::OdeFirstOrder);
    assert_eq!(model.param_names(), &["k", "A0", "B0"]);

    let (true_k, true_a0, true_b0) = (0.4_f64, 100.0_f64, 40.0_f64);
    let p_vec = [true_k, true_a0, true_b0];
    let xs_a = [0.0, 1.0, 2.5, 5.0, 8.0];
    let xs_b = [4.0, 5.0, 6.5, 9.0, 12.0];
    for (ds, xs, a0, t0) in [(0, &xs_a[..], true_a0, 0.0), (1, &xs_b[..], true_b0, 4.0)] {
        for &x in xs {
            let got = model.eval(&[x], &p_vec, ds).expect("scoped ODE eval");
            let exp = a0 * (-true_k * (x - t0)).exp();
            assert!((got - exp).abs() < 1e-6, "ds {ds} at {x}: got {got}, exp {exp}");
        }
    }
    assert_eq!(model.eval(&[1.0], &p_vec, 2), None);

    let ys_a: Vec<f64> = xs_a.iter().map(|&x| true_a0 * (-true_k * x).exp()).collect();
    let ys_b: Vec<f64> = xs_b.iter().map(|&x| true_b0 * (-true_k * (x - 4.0)).exp()).collect();
    let cfg = cfg_fast_rules(vec![rule_shared_bounds("k", Some(0.25), Some(1e-4), None), rule_shared_init("A0", 80.0), rule_shared_init("B0", 30.0)]);
    let fit = fit_with(&model, &[series_xy(0, "A", &xs_a, &ys_a), series_xy(1, "B", &xs_b, &ys_b)], &cfg);
    assert!((param_ds_val(&fit, 0, "k") - true_k).abs() < 1e-5);
    assert!((param_ds_val(&fit, 0, "A0") - true_a0).abs() < 1e-4);
    assert!((param_ds_val(&fit, 0, "B0") - true_b0).abs() < 1e-4);

    // Specific `<A>` IC before generic `Y(2) = B0` must not be overwritten.
    let ordered = dsl_compile("<A> Y0 = A0\nY(2) = B0\ndY/dX = -k * Y");
    let ord_params = [0.5_f64, 20.0, 50.0];
    assert!((ordered.eval(&[0.0], &ord_params, 0).unwrap() - 20.0).abs() < 1e-12);
    assert!((ordered.eval(&[2.0], &ord_params, 1).unwrap() - 50.0).abs() < 1e-12);

    assert_dsl_compile_err("<A> Y0 = A0\n<B> dY/dX = -k * Y");
}

#[test]
fn audit_43_ode_initial_resolves_in_scope_locals_and_rejects_invalid_references() {
    let model = dsl_compile("Ytop = Bottom + Span\nY0 = Ytop\ndY/dX = -k * (Y - Bottom)");
    assert_eq!(model.param_names(), &["Bottom", "Span", "k"], "Ytop must not leak as fitted parameter");
    for &x in &[0.0, 1.0, 3.0, 7.0] {
        let got = model.eval(&[x], &[10.0, 90.0, 0.35], 0).unwrap();
        assert!((got - (10.0 + 90.0 * (-0.35 * x).exp())).abs() < 1e-6);
    }

    let rate = dsl_compile("Yinit = Bottom + Span\nrate = k * (Y - Bottom)\nY0 = Yinit\ndY/dX = -rate");
    assert_eq!(rate.param_names(), &["Bottom", "Span", "k"]);
    assert!((rate.eval(&[2.0], &[10.0, 90.0, 0.35], 0).unwrap() - (10.0 + 90.0 * (-0.7_f64).exp())).abs() < 1e-6);

    let shadow = dsl_compile("v = A0 * 2\nY0 = v\nv = k * Y\ndY/dX = -v");
    assert_eq!(shadow.param_names(), &["A0", "k"]);
    assert!((shadow.eval(&[2.0], &[15.0, 0.25], 0).unwrap() - 30.0 * (-0.5_f64).exp()).abs() < 1e-6);

    for bad in [
        "Y0 = Ytop\nYtop = Bottom + Span\ndY/dX = -k * Y",
        "<A> Ytop = Bottom + Span\n<B> Y0 = Ytop\n<B> dY/dX = -k * Y",
        "<A> Ytop = Bottom + Span\nY0 = Ytop\ndY/dX = -k * Y",
        "Y0 = Y + 1\ndY/dX = -k * Y",
        "tmp = 2 * Y\nY0 = tmp\ndY/dX = -k * Y",
    ] {
        assert_dsl_compile_err(bad);
    }
}

#[test]
fn audit_44_transitive_y_through_locals_compiles_as_implicit_and_solves() {
    let m1 = dsl_compile("t = c * Y\nY = A * X + t");
    assert_eq!(m1.kind(), DslEquationKind::Implicit);
    assert_eq!(m1.param_names(), &["c", "A"]);
    for &x in &[0.0, 1.0, 4.0, 10.0] {
        assert!((m1.eval(&[x], &[0.25, 3.0], 0).unwrap() - (3.0 * x) / 0.75).abs() < 1e-8);
    }
    assert!((m1.eval(&[2.0], &[0.5, 1.0], 0).unwrap() - 4.0).abs() < 1e-12);

    // Nonlinear identifiable transitive-Y local implicit fit (`Y = A * X + c * sin(Y)`).
    let m_kepler = dsl_compile("t = c * sin(Y)\nY = A * X + t");
    assert_eq!(m_kepler.kind(), DslEquationKind::Implicit);
    let (true_c, true_a) = (0.45_f64, 1.2_f64);
    let ys: Vec<f64> = (1..=12).map(|i| 0.4 * (i as f64)).collect();
    let xs: Vec<f64> = ys.iter().map(|&y| (y - true_c * y.sin()) / true_a).collect();
    let cfg = cfg_fast_rules(vec![rule_shared_bounds("c", Some(0.2), Some(-0.9), Some(0.9)), rule_shared_init("A", 1.0)]);
    let fit = fit_with(&m_kepler, &[series_xy(0, "D0", &xs, &ys)], &cfg);
    assert!((param_ds_val(&fit, 0, "c") - true_c).abs() < 1e-6);
    assert!((param_ds_val(&fit, 0, "A") - true_a).abs() < 1e-6);

    let m2 = dsl_compile("a = c * Y\nb = a + 2\nY = A * X + b");
    assert_eq!(m2.kind(), DslEquationKind::Implicit);
    assert!((m2.eval(&[4.0], &[0.2, 3.0], 0).unwrap() - (3.0 * 4.0 + 2.0) / 0.8).abs() < 1e-8);

    let m3 = dsl_compile("t = c * Y\nt = A * X\nY = t");
    assert_eq!(m3.kind(), DslEquationKind::Explicit);
    assert_eq!(m3.eval(&[4.0], &[0.5, 3.0], 0), Some(12.0));

    let m4 = dsl_compile("<A> t = c * Y\n<A> Y = A * X + t\n<B> Y = A * X");
    assert_eq!(m4.kind(), DslEquationKind::Implicit);
    assert!((m4.eval(&[4.0], &[0.25, 3.0], 0).unwrap() - 16.0).abs() < 1e-8);
    assert!((m4.eval(&[4.0], &[0.25, 3.0], 1).unwrap() - 12.0).abs() < 1e-8);
}

#[test]
fn audit_45_rejects_trailing_and_out_of_order_locals() {
    for bad in [
        "Y = A * X * k\nk = 2",
        "dY/dX = -k * Y\nk = 2\nY0 = A0",
        "a = b + 1\nb = 2\nY = a * X",
        "<A> Y = Top * X\n<A> Top = 5",
        "<A> x_loc = 1\n<B> Y = x_loc * X",
    ] {
        assert_dsl_compile_err(bad);
    }
    assert!(CompiledDslModel::compile("ode_ic_after_deriv", "dY/dX = -k * Y\nY(0) = Y0").is_ok());
}

#[test]
fn audit_46_nan_propagation_and_inf_consistency() {
    for &(src, x, exp) in &[
        ("Y = IF((X - 5)^0.5 > 1, 10, 20)", 1.0, None),
        ("Y = IF((X - 5)^0.5 > 1, 10, 20)", 10.0, Some(10.0)),
        ("Y = IF((X - 5)^0.5 > 1, 10, 20)", 5.5, Some(20.0)),
        ("Y = IF((-8)^(1/3) < 0, 1, 2)", 0.0, None),
        ("Y = NOT((X - 5)^0.5)", 1.0, None),
        ("Y = NOT((X - 5)^0.5)", 5.0, Some(1.0)),
        ("Y = NOT((X - 5)^0.5)", 9.0, Some(0.0)),
        ("Y = ((X - 5)^0.5) || 1", 1.0, None),
        ("Y = 0 || ((X - 5)^0.5)", 1.0, None),
        ("Y = 1 && ((X - 5)^0.5)", 1.0, None),
        ("Y = IF(X > 0 && sqrt(X) > 1, 1, 0)", -1.0, Some(0.0)),
        ("Y = IF(X > 0 && sqrt(X) > 1, 1, 0)", 4.0, Some(1.0)),
        ("Y = IF(NAN, 1, 2)", 0.0, None),
        ("Y = NOT(NAN)", 0.0, None),
        ("Y = 1 && NAN", 0.0, None),
        ("Y = NAN == NAN", 0.0, None),
        ("Y = IF(X > 0, 1, 2)", f64::NAN, None),
        ("Y = INF == INF", 0.0, Some(1.0)),
        ("Y = -INF == -INF", 0.0, Some(1.0)),
        ("Y = INF == -INF", 0.0, Some(0.0)),
        ("Y = IF(INF, 1, 2)", 0.0, Some(1.0)),
        ("Y = NOT(INF)", 0.0, Some(0.0)),
        ("Y = IF(-INF, 1, 2)", 0.0, Some(1.0)),
        ("Y = NOT(-INF)", 0.0, Some(0.0)),
        ("Y = INF && 1", 0.0, Some(1.0)),
    ] {
        assert_eq!(dsl_compile(src).eval(&[x], &[], 0), exp, "src={src}, x={x}");
    }
}

#[test]
fn audit_47_exp_and_exp10_overflow_saturate_sigmoid_and_steep_4pl_fits() {
    for &(src, x, exp) in &[
        ("Y = 1 / (1 + exp(X))", 800.0, Some(0.0)),
        ("Y = 1 / (1 + exp(X))", -800.0, Some(1.0)),
        ("Y = 1 / (1 + exp10(X))", 400.0, Some(0.0)),
        ("Y = 1 / (1 + exp10(X))", -400.0, Some(1.0)),
        ("Y = exp(800)", 0.0, None),
        ("Y = exp10(400)", 0.0, None),
    ] {
        assert_eq!(dsl_compile(src).eval(&[x], &[], 0), exp, "src={src}, x={x}");
    }

    let fourpl_exp = dsl_compile("Y = Bottom + (Top - Bottom) / (1 + exp(HillSlope * (LogEC50 - X)))");
    let fourpl_pow = dsl_compile("Y = Bottom + (Top - Bottom) / (1 + E^(HillSlope * (LogEC50 - X)))");
    let (bottom, top, hill, log_ec50) = (5.0_f64, 95.0_f64, 200.0_f64, -6.0_f64);
    let xs: Vec<f64> = (-12..=-4).map(|i| i as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| bottom + (top - bottom) / (1.0 + (hill * (log_ec50 - x)).exp())).collect();
    for &x in &xs {
        let ve = fourpl_exp.eval(&[x], &[bottom, top, hill, log_ec50], 0).unwrap();
        let vp = fourpl_pow.eval(&[x], &[bottom, top, hill, log_ec50], 0).unwrap();
        assert!((ve - vp).abs() < 1e-14, "at x={x}: exp={ve}, pow={vp}");
    }

    let cfg = cfg_fast_rules(vec![
        rule_shared_init("Bottom", 2.0),
        rule_shared_init("Top", 100.0),
        rule_shared_bounds("HillSlope", Some(50.0), Some(0.1), None),
        rule_shared_init("LogEC50", -6.05),
    ]);
    let fit = fit_with(&fourpl_exp, &[series_xy(0, "Steep4PL", &xs, &ys)], &cfg);
    assert!((param_ds_val(&fit, 0, "Bottom") - bottom).abs() < 1e-5);
    assert!((param_ds_val(&fit, 0, "Top") - top).abs() < 1e-5);
}

#[test]
fn audit_48_reserved_identifier_warnings_and_assignment_errors() {
    for &(src, params, expected_warns) in &[
        ("Y = A * E^(-k * X)", &["A", "k"][..], &[(DslWarningKind::ReservedIdentifierUsed, "E")][..]),
        (
            "Y = A * e^(-k * X) + Pi * B",
            &["A", "k", "B"][..],
            &[(DslWarningKind::NonCanonicalReservedCase, "e"), (DslWarningKind::NonCanonicalReservedCase, "Pi")][..],
        ),
        ("Y = A * exp(-t * X)", &["A"][..], &[(DslWarningKind::AliasTWithPredictorX, "t")][..]),
        ("Y0 = A0\ndY/dt = -k * Y + c * t", &["k", "c", "A0"][..], &[][..]),
    ] {
        let m = dsl_compile(src);
        assert_eq!(m.param_names(), params, "src={src}");
        if expected_warns.is_empty() {
            assert!(m.warnings().is_empty(), "expected no warnings on `{src}`, got {:?}", m.warnings());
        } else {
            for &(kind, ident) in expected_warns {
                assert!(m.warnings().iter().any(|w| w.kind == kind && w.identifier == ident), "missing {kind:?}/{ident} on `{src}`");
            }
        }
    }

    for bad in [
        "E = 2\nY = E * X",
        "PI = 3.14\nY = PI * X",
        "T = 5\nY = T * X",
        "DS = 1\nY = DS * X",
        "Y = A * X\n[InitialValues]\nE = 1",
        "Y = A * X\n[InitialValues]\nt = 1",
        "Y = A * X # initial: PI = 3",
        "Y = A * X # initial: t = 2",
    ] {
        assert_dsl_compile_err(bad);
    }

    assert_eq!(dsl_compile("dY/dX = -k * Y # initial: Y0 = 10, k = 0.5").param_names(), &["k", "Y0"]);
}

#[test]
fn audit_49_scale_invariant_rk45_and_order_independent_trajectory_cache() {
    let model = dsl_compile("Y0 = A0\ndY/dX = -k * Y");

    for (a0, k, x) in [(1e-9_f64, 1.0, 10.0), (1e-6, 2.0, 5.0), (1e-12, 1.5, 10.0)] {
        let got = model.eval(&[x], &[k, a0], 0).unwrap();
        let exact = a0 * (-k * x).exp();
        assert!((got - exact).abs() / exact < 1e-6, "a0={a0}, k={k}, x={x}: got={got}, exact={exact}");
    }
    let rk_direct = integrate_rk45(0.0, 10.0, 1e-9, |_t, y| Some(-y)).unwrap();
    let rk_exact = 1e-9 * (-10.0_f64).exp();
    assert!(((rk_direct - rk_exact).abs() / rk_exact) < 1e-6);

    let flush_cache = || {
        for filler in 0..4_u32 {
            let _ = model.eval(&[1.0], &[0.11 + f64::from(filler) * 0.01, 5.0], 0);
        }
    };
    let pts = [1.0, 2.0, 3.0, 5.0, 7.0, 10.0];
    let params_fresh = [0.41_f64, 2.34e-7];
    let cold_vals: Vec<f64> = pts
        .iter()
        .map(|&x| {
            flush_cache();
            model.eval(&[x], &params_fresh, 0).unwrap()
        })
        .collect();
    flush_cache();
    let forward_vals: Vec<f64> = pts.iter().map(|&x| model.eval(&[x], &params_fresh, 0).unwrap()).collect();
    flush_cache();
    let mut reverse_vals: Vec<f64> = pts.iter().rev().map(|&x| model.eval(&[x], &params_fresh, 0).unwrap()).collect();
    reverse_vals.reverse();
    for i in 0..pts.len() {
        assert_eq!(cold_vals[i].to_bits(), forward_vals[i].to_bits(), "cold vs forward at x={}", pts[i]);
        assert_eq!(forward_vals[i].to_bits(), reverse_vals[i].to_bits(), "forward vs reverse at x={}", pts[i]);
    }

    let xs_fit: Vec<f64> = (0..=10).map(|i| i as f64).collect();
    let ys_molar: Vec<f64> = xs_fit.iter().map(|&x| (-0.35 * x).exp()).collect();
    let ys_nano: Vec<f64> = ys_molar.iter().map(|&y| y * 1e-9).collect();
    let fit_cfg = |a0_guess: f64| cfg_fast_rules(vec![rule_shared_bounds("k", Some(0.1), Some(1e-4), None), rule_shared_init("A0", a0_guess)]);
    let r_molar = fit_with(&model, &[series_xy(0, "M", &xs_fit, &ys_molar)], &fit_cfg(0.8));
    let r_nano = fit_with(&model, &[series_xy(0, "nM", &xs_fit, &ys_nano)], &fit_cfg(0.8e-9));
    assert!((param_ds_val(&r_molar, 0, "k") - 0.35).abs() < 1e-6);
    assert!((param_ds_val(&r_nano, 0, "k") - 0.35).abs() < 1e-6);
}

#[test]
fn audit_50_vm_opcode_cap_and_ode_vm_op_budget_bounds_heavy_rhs() {
    let mut big_src = String::from("a = X + 1\n");
    for _ in 0..15_000 {
        big_src.push_str("a = a + p - q + 1\n");
    }
    big_src.push_str("Y = a");
    assert_dsl_compile_err(&big_src);

    let mut heavy_ode = String::from("Y0 = A\ns = Y\n");
    for _ in 0..500 {
        heavy_ode.push_str("s = s + 0.00001 * sin(s) * cos(X)\n");
    }
    heavy_ode.push_str("dY/dX = -K * s");
    let model = dsl_compile(&heavy_ode);
    let xs: Vec<f64> = (1..=30).map(|i| i as f64).collect();
    let ys = vec![0.0; xs.len()];
    let cfg = cfg_fast_rules(vec![rule_shared_bounds("K", Some(10_000.0), Some(5_000.0), None), rule_shared_init("A", 1.0)]);

    let t0 = Instant::now();
    let res = fit_nonlinear(&model, &[series_xy(0, "Heavy", &xs, &ys)], &HookContext::default(), &cfg);
    let elapsed = t0.elapsed().as_secs_f64();
    assert!(elapsed < 1.5, "heavy ODE VM budget should cap work quickly, took {elapsed:.3}s");
    if let Err(err) = res {
        assert!(matches!(err, FitError::OdeIntegrationFailed { .. }), "expected OdeIntegrationFailed, got {err:?}");
    }
}
