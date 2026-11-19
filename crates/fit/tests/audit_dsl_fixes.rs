use std::time::Instant;

use curvebook_doc::{ParameterRule, ParameterScope};
use curvebook_fit::dsl::{DslCompileWarning, DslWarningKind};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, DslEquationKind, FitError, HookContext, ParametricModel, SolverConfig, fit_nonlinear, integrate_rk45,
};

fn make_dataset(ds_idx: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
    DatasetSeries {
        data_set_index: ds_idx,
        label: label.to_owned(),
        points: xs
            .iter()
            .zip(ys.iter())
            .enumerate()
            .map(|(row, (&x, &y))| DataPoint { row, data_set_index: ds_idx, ..DataPoint::xy(x, y) })
            .collect(),
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn find_ds_param(res: &curvebook_fit::NonlinearFitResult, ds_idx: usize, name: &str) -> f64 {
    res.parameters.iter().find(|p| p.name == name && (p.dataset_index.is_none() || p.dataset_index == Some(ds_idx))).map_or(f64::NAN, |p| p.estimate)
}

#[test]
fn audit_42_ode_preserves_dataset_prefixed_ic_and_t0_in_global_fit() {
    // 1) Separate Y0 initial conditions by dataset (`<A>`, `<B>`) + reverse order mixed with generic fallback.
    let src = "\
<A> Y0 = A0
<B> Y(4) = B0
dY/dX = -k * Y";
    let model = CompiledDslModel::compile("ode_ds_ic", src).expect("dataset-scoped ODE compiles");
    assert_eq!(model.kind(), DslEquationKind::OdeFirstOrder);
    assert_eq!(model.param_names(), &["k", "A0", "B0"]);

    let true_k = 0.4_f64;
    let true_a0 = 100.0_f64;
    let true_b0 = 40.0_f64;
    let p_vec = [true_k, true_a0, true_b0];

    let xs_a = [0.0, 1.0, 2.5, 5.0, 8.0];
    let xs_b = [4.0, 5.0, 6.5, 9.0, 12.0];
    for &x in &xs_a {
        let got = model.eval(&[x], &p_vec, 0).expect("ds A eval");
        let exp = true_a0 * (-true_k * (x - 0.0)).exp();
        assert!((got - exp).abs() < 1e-6, "ds A at {x}: got {got}, exp {exp}");
    }
    for &x in &xs_b {
        let got = model.eval(&[x], &p_vec, 1).expect("ds B eval");
        let exp = true_b0 * (-true_k * (x - 4.0)).exp();
        assert!((got - exp).abs() < 1e-6, "ds B at {x}: got {got}, exp {exp}");
    }
    // Outside explicitly filtered IC datasets (`ds_idx = 2`), `select_ode_init` has no IC and returns `None`.
    assert_eq!(model.eval(&[1.0], &p_vec, 2), None);

    let ys_a: Vec<f64> = xs_a.iter().map(|&x| true_a0 * (-true_k * x).exp()).collect();
    let ys_b: Vec<f64> = xs_b.iter().map(|&x| true_b0 * (-true_k * (x - 4.0)).exp()).collect();
    let cfg = SolverConfig {
        parameter_rules: vec![
            ParameterRule {
                name: "k".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(0.25),
                lower_bound: Some(1e-4),
                upper_bound: None,
            },
            ParameterRule {
                name: "A0".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(80.0),
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "B0".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(30.0),
                lower_bound: None,
                upper_bound: None,
            },
        ],
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(&model, &[make_dataset(0, "A", &xs_a, &ys_a), make_dataset(1, "B", &xs_b, &ys_b)], &HookContext::default(), &cfg)
        .expect("global scoped ODE IC fit converges");
    assert!((find_ds_param(&fit, 0, "k") - true_k).abs() < 1e-5);
    assert!((find_ds_param(&fit, 0, "A0") - true_a0).abs() < 1e-4);
    assert!((find_ds_param(&fit, 0, "B0") - true_b0).abs() < 1e-4);

    // 2) Specific `<A>` IC placed BEFORE generic `Y(2) = B0` must NOT be overwritten by generic fallback.
    let ordered_src = "\
<A> Y0 = A0
Y(2) = B0
dY/dX = -k * Y";
    let ordered_model = CompiledDslModel::compile("ode_spec_wins", ordered_src).expect("ordered ODE compiles");
    let ord_params = [0.5_f64, 20.0_f64, 50.0_f64];
    assert!((ordered_model.eval(&[0.0], &ord_params, 0).expect("ds0 at t0=0") - 20.0).abs() < 1e-12);
    assert!((ordered_model.eval(&[2.0], &ord_params, 1).expect("ds1 at t0=2") - 50.0).abs() < 1e-12);

    // 3) Completely disjoint IC vs ODE derivative scope fails cleanly at compile time.
    let disjoint = CompiledDslModel::compile("ode_disjoint", "<A> Y0 = A0\n<B> dY/dX = -k * Y");
    assert!(matches!(disjoint, Err(FitError::DslCompileError { .. })));
}

#[test]
fn audit_43_ode_initial_resolves_in_scope_locals_and_rejects_invalid_references() {
    // 1) In-scope local preceding Y0 is resolved cleanly rather than turned into a spurious parameter.
    let src = "\
Ytop = Bottom + Span
Y0 = Ytop
dY/dX = -k * (Y - Bottom)";
    let model = CompiledDslModel::compile("ode_local_ic", src).expect("local IC compiles");
    assert_eq!(model.param_names(), &["Bottom", "Span", "k"], "Ytop must not leak as fitted parameter");
    let (bottom, span, k) = (10.0_f64, 90.0_f64, 0.35_f64);
    for &x in &[0.0, 1.0, 3.0, 7.0] {
        let got = model.eval(&[x], &[bottom, span, k], 0).expect("ODE local IC eval");
        let exp = bottom + span * (-k * x).exp();
        assert!((got - exp).abs() < 1e-6, "at x={x}: got {got}, exp {exp}");
    }

    // 2) Preceding local referencing `Y` that is NOT used by `Y0` (e.g., `rate = k * Y`) must still compile.
    let src_rate = "\
Yinit = Bottom + Span
rate = k * (Y - Bottom)
Y0 = Yinit
dY/dX = -rate";
    let rate_model = CompiledDslModel::compile("ode_rate_local", src_rate).expect("rate local compiles");
    assert_eq!(rate_model.param_names(), &["Bottom", "Span", "k"]);
    let got_r = rate_model.eval(&[2.0], &[10.0, 90.0, 0.35], 0).expect("rate ODE eval");
    assert!((got_r - (10.0 + 90.0 * (-0.7_f64).exp())).abs() < 1e-6);

    // 3) Variable shadowed after `Y0` uses the pre-`Y0` value in `Y0` and post-`Y0` value in derivative.
    let shadow_src = "\
v = A0 * 2
Y0 = v
v = k * Y
dY/dX = -v";
    let shadow_m = CompiledDslModel::compile("ode_shadow_ic", shadow_src).expect("shadowing ODE compiles");
    assert_eq!(shadow_m.param_names(), &["A0", "k"]);
    let got_s = shadow_m.eval(&[2.0], &[15.0, 0.25], 0).expect("shadow ODE eval");
    assert!((got_s - 30.0 * (-0.5_f64).exp()).abs() < 1e-6);

    // 4) Reject later-assigned, out-of-scope, or Y-dependent locals in ODE initial condition.
    for bad in [
        "Y0 = Ytop\nYtop = Bottom + Span\ndY/dX = -k * Y",
        "<A> Ytop = Bottom + Span\n<B> Y0 = Ytop\n<B> dY/dX = -k * Y",
        "<A> Ytop = Bottom + Span\nY0 = Ytop\ndY/dX = -k * Y",
        "Y0 = Y + 1\ndY/dX = -k * Y",
        "tmp = 2 * Y\nY0 = tmp\ndY/dX = -k * Y",
    ] {
        let err = CompiledDslModel::compile("bad_ode_ic", bad);
        assert!(matches!(err, Err(FitError::DslCompileError { .. })), "expected compile error for:\n{bad}");
    }
}

#[test]
fn audit_44_transitive_y_through_locals_compiles_as_implicit_and_solves() {
    // 1) Single-hop `t = c * Y; Y = A * X + t`
    let m1 = CompiledDslModel::compile("implicit_local_1hop", "t = c * Y\nY = A * X + t").expect("1-hop implicit compiles");
    assert_eq!(m1.kind(), DslEquationKind::Implicit);
    assert_eq!(m1.param_names(), &["c", "A"]);
    for &x in &[0.0, 1.0, 4.0, 10.0] {
        let got = m1.eval(&[x], &[0.25, 3.0], 0).expect("1-hop implicit eval");
        let exp = (3.0 * x) / 0.75;
        assert!((got - exp).abs() < 1e-8, "x={x}: got {got}, exp {exp}");
    }

    assert!((m1.eval(&[2.0], &[0.5, 1.0], 0).expect("exact #44 reproducer") - 4.0).abs() < 1e-12);

    // Nonlinear identifiable transitive-Y local implicit fit (`Y = A * X + c * sin(Y)`).
    let m_kepler = CompiledDslModel::compile("implicit_local_kepler", "t = c * sin(Y)\nY = A * X + t").expect("kepler local compiles");
    assert_eq!(m_kepler.kind(), DslEquationKind::Implicit);
    let (true_c, true_a) = (0.45_f64, 1.2_f64);
    let ys: Vec<f64> = (1..=12).map(|i| 0.4 * (i as f64)).collect();
    let xs: Vec<f64> = ys.iter().map(|&y| (y - true_c * y.sin()) / true_a).collect();
    let cfg = SolverConfig {
        parameter_rules: vec![
            ParameterRule {
                name: "c".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(0.2),
                lower_bound: Some(-0.9),
                upper_bound: Some(0.9),
            },
            ParameterRule {
                name: "A".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(1.0),
                lower_bound: None,
                upper_bound: None,
            },
        ],
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(&m_kepler, &[make_dataset(0, "D0", &xs, &ys)], &HookContext::default(), &cfg).expect("implicit local fit converges");
    assert!((find_ds_param(&fit, 0, "c") - true_c).abs() < 1e-6);
    assert!((find_ds_param(&fit, 0, "A") - true_a).abs() < 1e-6);

    // 2) Multi-hop transitive Y through locals
    let m2 = CompiledDslModel::compile("implicit_local_2hop", "a = c * Y\nb = a + 2\nY = A * X + b").expect("2-hop compiles");
    assert_eq!(m2.kind(), DslEquationKind::Implicit);
    let got2 = m2.eval(&[4.0], &[0.2, 3.0], 0).expect("2-hop eval");
    assert!((got2 - (3.0 * 4.0 + 2.0) / 0.8).abs() < 1e-8);

    // 3) Reassigned local overwriting `Y` leaves equation Explicit
    let m3 = CompiledDslModel::compile("explicit_after_shadow", "t = c * Y\nt = A * X\nY = t").expect("shadow reset compiles");
    assert_eq!(m3.kind(), DslEquationKind::Explicit);
    assert_eq!(m3.eval(&[4.0], &[0.5, 3.0], 0), Some(12.0));

    // 4) Scoped `<A>` transitive `Y` promotes model cleanly across branches
    let m4 = CompiledDslModel::compile("scoped_implicit_local", "<A> t = c * Y\n<A> Y = A * X + t\n<B> Y = A * X").expect("scoped compiles");
    assert_eq!(m4.kind(), DslEquationKind::Implicit);
    assert!((m4.eval(&[4.0], &[0.25, 3.0], 0).expect("ds0") - 16.0).abs() < 1e-8);
    assert!((m4.eval(&[4.0], &[0.25, 3.0], 1).expect("ds1") - 12.0).abs() < 1e-8);
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
        let err = CompiledDslModel::compile("bad_local_order", bad);
        assert!(matches!(err, Err(FitError::DslCompileError { .. })), "expected compile error for:\n{bad}");
    }

    // Valid ODE IC after dY/dX must still compile cleanly.
    let valid_ode = CompiledDslModel::compile("ode_ic_after_deriv", "dY/dX = -k * Y\nY(0) = Y0");
    assert!(valid_ode.is_ok());
}

#[test]
fn audit_46_nan_propagation_and_inf_consistency() {
    let m_frac = CompiledDslModel::compile("frac_if", "Y = IF((X - 5)^0.5 > 1, 10, 20)").expect("compiles");
    assert_eq!(m_frac.eval(&[1.0], &[], 0), None);
    assert_eq!(m_frac.eval(&[10.0], &[], 0), Some(10.0));
    assert_eq!(m_frac.eval(&[5.5], &[], 0), Some(20.0));

    let m_neg_cbrt = CompiledDslModel::compile("neg_pow", "Y = IF((-8)^(1/3) < 0, 1, 2)").expect("compiles");
    assert_eq!(m_neg_cbrt.eval(&[0.0], &[], 0), None);

    let m_not_nan = CompiledDslModel::compile("not_dom", "Y = NOT((X - 5)^0.5)").expect("compiles");
    assert_eq!(m_not_nan.eval(&[1.0], &[], 0), None);
    assert_eq!(m_not_nan.eval(&[5.0], &[], 0), Some(1.0));
    assert_eq!(m_not_nan.eval(&[9.0], &[], 0), Some(0.0));

    let m_or_lhs = CompiledDslModel::compile("or_lhs", "Y = ((X - 5)^0.5) || 1").expect("compiles");
    assert_eq!(m_or_lhs.eval(&[1.0], &[], 0), None);

    let m_or_rhs = CompiledDslModel::compile("or_rhs", "Y = 0 || ((X - 5)^0.5)").expect("compiles");
    assert_eq!(m_or_rhs.eval(&[1.0], &[], 0), None);

    let m_and_rhs = CompiledDslModel::compile("and_rhs", "Y = 1 && ((X - 5)^0.5)").expect("compiles");
    assert_eq!(m_and_rhs.eval(&[1.0], &[], 0), None);

    let m_guarded = CompiledDslModel::compile("guarded_and", "Y = IF(X > 0 && sqrt(X) > 1, 1, 0)").expect("compiles");
    assert_eq!(m_guarded.eval(&[-1.0], &[], 0), Some(0.0));
    assert_eq!(m_guarded.eval(&[4.0], &[], 0), Some(1.0));

    for nan_src in ["Y = IF(NAN, 1, 2)", "Y = NOT(NAN)", "Y = 1 && NAN", "Y = NAN == NAN"] {
        let m = CompiledDslModel::compile("nan_prop", nan_src).expect("compiles");
        assert_eq!(m.eval(&[0.0], &[], 0), None, "{nan_src} must fail domain on NaN");
    }
    assert_eq!(CompiledDslModel::compile("nan_x", "Y = IF(X > 0, 1, 2)").expect("ok").eval(&[f64::NAN], &[], 0), None);

    // INF equality and truthiness consistency:
    assert_eq!(CompiledDslModel::compile("inf_eq", "Y = INF == INF").expect("ok").eval(&[0.0], &[], 0), Some(1.0));
    assert_eq!(CompiledDslModel::compile("ninf_eq", "Y = -INF == -INF").expect("ok").eval(&[0.0], &[], 0), Some(1.0));
    assert_eq!(CompiledDslModel::compile("inf_ne_ninf", "Y = INF == -INF").expect("ok").eval(&[0.0], &[], 0), Some(0.0));
    assert_eq!(CompiledDslModel::compile("if_inf", "Y = IF(INF, 1, 2)").expect("ok").eval(&[0.0], &[], 0), Some(1.0));
    assert_eq!(CompiledDslModel::compile("not_inf", "Y = NOT(INF)").expect("ok").eval(&[0.0], &[], 0), Some(0.0));
    assert_eq!(CompiledDslModel::compile("if_ninf", "Y = IF(-INF, 1, 2)").expect("ok").eval(&[0.0], &[], 0), Some(1.0));
    assert_eq!(CompiledDslModel::compile("not_ninf", "Y = NOT(-INF)").expect("ok").eval(&[0.0], &[], 0), Some(0.0));
    assert_eq!(CompiledDslModel::compile("inf_and", "Y = INF && 1").expect("ok").eval(&[0.0], &[], 0), Some(1.0));
}

#[test]
fn audit_47_exp_and_exp10_overflow_saturate_sigmoid_and_steep_4pl_fits() {
    let m_exp = CompiledDslModel::compile("sigmoid_exp", "Y = 1 / (1 + exp(X))").expect("compiles");
    assert_eq!(m_exp.eval(&[800.0], &[], 0), Some(0.0));
    assert_eq!(m_exp.eval(&[-800.0], &[], 0), Some(1.0));

    let m_exp10 = CompiledDslModel::compile("sigmoid_exp10", "Y = 1 / (1 + exp10(X))").expect("compiles");
    assert_eq!(m_exp10.eval(&[400.0], &[], 0), Some(0.0));
    assert_eq!(m_exp10.eval(&[-400.0], &[], 0), Some(1.0));

    // Top-level infinite result must still evaluate to `None`.
    assert_eq!(CompiledDslModel::compile("top_exp", "Y = exp(800)").expect("ok").eval(&[0.0], &[], 0), None);
    assert_eq!(CompiledDslModel::compile("top_exp10", "Y = exp10(400)").expect("ok").eval(&[0.0], &[], 0), None);

    // Steep 4PL curve with HillSlope = 200 over log concentrations -12..=-4.
    let fourpl_exp = CompiledDslModel::compile("fourpl_exp", "Y = Bottom + (Top - Bottom) / (1 + exp(HillSlope * (LogEC50 - X)))").expect("compiles");
    let fourpl_pow = CompiledDslModel::compile("fourpl_pow", "Y = Bottom + (Top - Bottom) / (1 + E^(HillSlope * (LogEC50 - X)))").expect("compiles");

    let (bottom, top, hill, log_ec50) = (5.0_f64, 95.0_f64, 200.0_f64, -6.0_f64);
    let xs: Vec<f64> = (-12..=-4).map(|i| i as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| bottom + (top - bottom) / (1.0 + (hill * (log_ec50 - x)).exp())).collect();

    for &x in &xs {
        let ve = fourpl_exp.eval(&[x], &[bottom, top, hill, log_ec50], 0).expect("exp eval");
        let vp = fourpl_pow.eval(&[x], &[bottom, top, hill, log_ec50], 0).expect("E^ eval");
        assert!((ve - vp).abs() < 1e-14, "at x={x}: exp={ve}, pow={vp}");
    }

    let cfg = SolverConfig {
        parameter_rules: vec![
            ParameterRule {
                name: "Bottom".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(2.0),
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "Top".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(100.0),
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "HillSlope".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(50.0),
                lower_bound: Some(0.1),
                upper_bound: None,
            },
            ParameterRule {
                name: "LogEC50".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(-6.05),
                lower_bound: None,
                upper_bound: None,
            },
        ],
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(&fourpl_exp, &[make_dataset(0, "Steep4PL", &xs, &ys)], &HookContext::default(), &cfg)
        .expect("steep 4PL with exp overflow converges");
    assert!((find_ds_param(&fit, 0, "Bottom") - bottom).abs() < 1e-5);
    assert!((find_ds_param(&fit, 0, "Top") - top).abs() < 1e-5);
}

#[test]
fn audit_48_reserved_identifier_warnings_and_assignment_errors() {
    fn find_warning<'a>(warnings: &'a [DslCompileWarning], kind: DslWarningKind, ident: &str) -> Option<&'a DslCompileWarning> {
        warnings.iter().find(|w| w.kind == kind && w.identifier == ident)
    }

    let m_e = CompiledDslModel::compile("res_E", "Y = A * E^(-k * X)").expect("E expression compiles");
    assert_eq!(m_e.param_names(), &["A", "k"]);
    assert!(find_warning(m_e.warnings(), DslWarningKind::ReservedIdentifierUsed, "E").is_some());

    let m_lc = CompiledDslModel::compile("res_lc", "Y = A * e^(-k * X) + Pi * B").expect("lowercase/mixed reserved compiles");
    assert_eq!(m_lc.param_names(), &["A", "k", "B"]);
    assert!(find_warning(m_lc.warnings(), DslWarningKind::NonCanonicalReservedCase, "e").is_some());
    assert!(find_warning(m_lc.warnings(), DslWarningKind::NonCanonicalReservedCase, "Pi").is_some());

    let m_alias_t = CompiledDslModel::compile("alias_t", "Y = A * exp(-t * X)").expect("alias t compiles");
    assert_eq!(m_alias_t.param_names(), &["A"]);
    assert!(find_warning(m_alias_t.warnings(), DslWarningKind::AliasTWithPredictorX, "t").is_some());

    // Canonical `dY/dt` kinetic form referencing `t` without `X` emits zero warnings.
    let m_ode_dt = CompiledDslModel::compile("ode_dt", "Y0 = A0\ndY/dt = -k * Y + c * t").expect("dY/dt compiles");
    assert!(m_ode_dt.warnings().is_empty(), "expected no warnings on dY/dt, got {:?}", m_ode_dt.warnings());

    // Explicit assignment / hint collision with reserved identifiers produces compile error.
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
        assert!(matches!(CompiledDslModel::compile("res_assign", bad), Err(FitError::DslCompileError { .. })), "must reject:\n{bad}");
    }

    // Valid ODE `# initial: Y0 = 10` remains accepted when Y0 is a fitted parameter.
    let m_hint_y0 = CompiledDslModel::compile("ode_y0_hint", "dY/dX = -k * Y # initial: Y0 = 10, k = 0.5").expect("Y0 hint compiles");
    assert_eq!(m_hint_y0.param_names(), &["k", "Y0"]);
}

#[test]
fn audit_49_scale_invariant_rk45_and_order_independent_trajectory_cache() {
    let model = CompiledDslModel::compile("ode_nano", "Y0 = A0\ndY/dX = -k * Y").expect("compiles");

    // 1) Nanomolar & picomolar scale-invariant accuracy
    for (a0, k, x) in [(1e-9_f64, 1.0_f64, 10.0_f64), (1e-6_f64, 2.0_f64, 5.0_f64), (1e-12_f64, 1.5_f64, 10.0_f64)] {
        let got = model.eval(&[x], &[k, a0], 0).expect("tiny scale ODE eval");
        let exact = a0 * (-k * x).exp();
        let rel_err = (got - exact).abs() / exact;
        assert!(rel_err < 1e-6, "a0={a0}, k={k}, x={x}: got={got}, exact={exact}, rel_err={rel_err}");
    }
    let rk_direct = integrate_rk45(0.0, 10.0, 1e-9, |_t, y| Some(-y)).expect("direct integrate_rk45 at 1e-9");
    let rk_exact = 1e-9 * (-10.0_f64).exp();
    assert!(((rk_direct - rk_exact).abs() / rk_exact) < 1e-6);

    // 2) Bit-for-bit order independence of cached trajectory evaluation
    // Use 4 distinct parameter tuples (one per LRU slot) with distinct k/A0 so cache entries start fresh.
    let pts: Vec<f64> = vec![1.0, 2.0, 3.0, 5.0, 7.0, 10.0];
    let params_fresh = [0.41_f64, 2.34e-7_f64];
    let mut cold_vals = Vec::new();
    for (idx, &x) in pts.iter().enumerate() {
        // Distinct minor shift per fresh trajectory would alter math; instead displace LRU cache between runs.
        for filler in 0..4_u32 {
            let _ = model.eval(&[1.0], &[0.11 + f64::from(filler) * 0.01, 5.0], 0);
        }
        let v = model.eval(&[x], &params_fresh, 0).expect("cold eval");
        cold_vals.push((idx, v));
    }

    for filler in 0..4_u32 {
        let _ = model.eval(&[1.0], &[0.11 + f64::from(filler) * 0.01, 5.0], 0);
    }
    let forward_vals: Vec<f64> = pts.iter().map(|&x| model.eval(&[x], &params_fresh, 0).expect("fwd eval")).collect();

    for filler in 0..4_u32 {
        let _ = model.eval(&[1.0], &[0.11 + f64::from(filler) * 0.01, 5.0], 0);
    }
    let mut reverse_vals: Vec<f64> = pts.iter().rev().map(|&x| model.eval(&[x], &params_fresh, 0).expect("rev eval")).collect();
    reverse_vals.reverse();

    for i in 0..pts.len() {
        assert_eq!(
            cold_vals[i].1.to_bits(),
            forward_vals[i].to_bits(),
            "cold vs forward mismatch at x={}: {} vs {}",
            pts[i],
            cold_vals[i].1,
            forward_vals[i]
        );
        assert_eq!(
            forward_vals[i].to_bits(),
            reverse_vals[i].to_bits(),
            "forward vs reverse mismatch at x={}: {} vs {}",
            pts[i],
            forward_vals[i],
            reverse_vals[i]
        );
    }

    // 3) Scale-invariant nanomolar nonlinear fit recovers identical rate constant `k` as molar fit.
    let xs_fit: Vec<f64> = (0..=10).map(|i| i as f64).collect();
    let ys_molar: Vec<f64> = xs_fit.iter().map(|&x| 1.0 * (-0.35 * x).exp()).collect();
    let ys_nano: Vec<f64> = ys_molar.iter().map(|&y| y * 1e-9).collect();
    let fit_cfg = |a0_guess: f64| SolverConfig {
        parameter_rules: vec![
            ParameterRule {
                name: "k".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(0.1),
                lower_bound: Some(1e-4),
                upper_bound: None,
            },
            ParameterRule {
                name: "A0".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(a0_guess),
                lower_bound: None,
                upper_bound: None,
            },
        ],
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        ..SolverConfig::default()
    };
    let r_molar = fit_nonlinear(&model, &[make_dataset(0, "M", &xs_fit, &ys_molar)], &HookContext::default(), &fit_cfg(0.8)).expect("molar fit");
    let r_nano = fit_nonlinear(&model, &[make_dataset(0, "nM", &xs_fit, &ys_nano)], &HookContext::default(), &fit_cfg(0.8e-9)).expect("nano fit");
    assert!((find_ds_param(&r_molar, 0, "k") - 0.35).abs() < 1e-6);
    assert!((find_ds_param(&r_nano, 0, "k") - 0.35).abs() < 1e-6);
}

#[test]
fn audit_50_vm_opcode_cap_and_ode_vm_op_budget_bounds_heavy_rhs() {
    // 1) Program exceeding `MAX_TOTAL_OPCODES <= MAX_VM_STEPS` fails cleanly at compile time rather than compiling and unconditionally failing VM eval.
    let mut big_src = String::from("a = X + 1\n");
    for _ in 0..15_000 {
        // ~7 opcodes per line * 15,000 = ~105,000 opcodes > MAX_TOTAL_OPCODES (100_000)
        big_src.push_str("a = a + p - q + 1\n");
    }
    big_src.push_str("Y = a");
    let compiled_big = CompiledDslModel::compile("oversized_opcodes", &big_src);
    assert!(matches!(compiled_big, Err(FitError::DslCompileError { .. })), "expected compile opcode cap error");

    // 2) Heavy per-RHS ODE evaluates within bounded VM-op budgets in well under 1s.
    let mut heavy_ode = String::from("Y0 = A\ns = Y\n");
    for _ in 0..500 {
        heavy_ode.push_str("s = s + 0.00001 * sin(s) * cos(X)\n");
    }
    heavy_ode.push_str("dY/dX = -K * s");
    let model = CompiledDslModel::compile("heavy_ode", &heavy_ode).expect("heavy ODE compiles within opcode cap");
    let xs: Vec<f64> = (1..=30).map(|i| i as f64).collect();
    let ys: Vec<f64> = vec![0.0; xs.len()];
    let cfg = SolverConfig {
        parameter_rules: vec![
            ParameterRule {
                name: "K".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(10_000.0),
                lower_bound: Some(5_000.0),
                upper_bound: None,
            },
            ParameterRule {
                name: "A".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: Some(1.0),
                lower_bound: None,
                upper_bound: None,
            },
        ],
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        ..SolverConfig::default()
    };

    let t0 = Instant::now();
    let res = fit_nonlinear(&model, &[make_dataset(0, "Heavy", &xs, &ys)], &HookContext::default(), &cfg);
    let elapsed = t0.elapsed().as_secs_f64();
    assert!(elapsed < 1.5, "heavy ODE VM budget should cap work quickly, took {elapsed:.3}s");
    if let Err(err) = res {
        assert!(matches!(err, FitError::OdeIntegrationFailed { .. }), "expected OdeIntegrationFailed on VM budget exhaustion, got {err:?}");
    }
}
