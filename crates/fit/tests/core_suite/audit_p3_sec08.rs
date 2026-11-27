//! Regression suite for SEC-08 findings (`FA-SEC08-01` .. `FA-SEC08-12`) in `crates/fit`.

use curvebook_doc::NonlinearFitSpec;
use curvebook_fit::dsl::DslWarningKind;
use curvebook_fit::{CompiledDslModel, DataPoint, DslEquationKind, ParametricModel, get_builtin_model};
use curvebook_testkit::assert_approx_eq;

use crate::common::{assert_dsl_compile_err, dsl_compile, exec_nl, rule_const, xy_sheet_single};

fn assert_rel_close(actual: f64, expected: f64, rel_tol: f64, abs_tol: f64, ctx: &str) {
    assert_approx_eq(actual, expected, abs_tol, rel_tol, ctx);
}

#[test]
fn fa_sec08_01_dsl_dataset_scoped_branch_precedence_over_trailing_fallback() {
    let m_a = dsl_compile("<A> Y = Baseline\nY = Bottom + (Top - Bottom) / (1 + 10^(LogEC50 - X))");
    assert_eq!(m_a.param_names(), &["Baseline", "Bottom", "Top", "LogEC50"]);
    assert_rel_close(m_a.eval(&[0.0], &[42.0, 10.0, 100.0, 0.0], 0).unwrap(), 42.0, 1e-12, 1e-12, "<A> Baseline");
    assert_rel_close(m_a.eval(&[0.0], &[42.0, 10.0, 100.0, 0.0], 1).unwrap(), 55.0, 1e-12, 1e-12, "ds1 fallback");

    let m_not_a = dsl_compile("<~A> Y = a * X\nY = b * X");
    assert_eq!(m_not_a.param_names(), &["b", "a"]);
    for (ds, exp) in [(0, 40.0), (1, 12.0), (2, 12.0)] {
        assert_rel_close(m_not_a.eval(&[4.0], &[10.0, 3.0], ds).unwrap(), exp, 1e-12, 1e-12, &format!("<~A> ds{ds}"));
    }

    let m_any = dsl_compile("<A> Y = Baseline\nm = 3\nY = m * X");
    assert_eq!(m_any.param_names(), &["Baseline"]);
    assert_eq!((m_any.eval(&[5.0], &[7.0], 0), m_any.eval(&[5.0], &[7.0], 1)), (Some(7.0), Some(15.0)));

    let m_not_a_loc = dsl_compile("<~A> Y = a * X\nm = 3\nY = m * X");
    assert_eq!(m_not_a_loc.param_names(), &["a"]);
    assert_eq!((m_not_a_loc.eval(&[5.0], &[4.0], 0), m_not_a_loc.eval(&[5.0], &[4.0], 1)), (Some(15.0), Some(20.0)));

    assert_dsl_compile_err("<A> Y = Baseline\nm = 3");

    let m_ode = dsl_compile("<A> Y0 = A0\n<A> dY/dX = -k1 * Y\nY0 = B0\ndY/dX = -k2 * Y");
    assert_eq!(m_ode.kind(), DslEquationKind::OdeFirstOrder);
    assert_rel_close(m_ode.eval(&[2.0], &[0.5, 1.5, 20.0, 50.0], 0).unwrap(), 20.0 * (-1.0_f64).exp(), 1e-6, 1e-9, "<A> ODE");
    assert_rel_close(m_ode.eval(&[2.0], &[0.5, 1.5, 20.0, 50.0], 1).unwrap(), 50.0 * (-3.0_f64).exp(), 1e-6, 1e-9, "ds1 ODE");
}

#[test]
fn fa_sec08_02_morrison_ki_low_et_and_excess_inhibitor_no_cancellation() {
    let model = get_builtin_model("morrison_tight_binding_ki").unwrap();
    let p_ext = [100.0_f64, 1e-18, 1e-3, 0.0, 1e-3];
    let ki_app = 2e-10_f64;
    let exp_nano = 250.0 * (2.0 * ki_app) / ((1e-5 + ki_app - 1e-12) + (1e-12 - 1e-5 + ki_app).hypot(2.0 * (1e-5_f64).sqrt() * ki_app.sqrt()));
    let exp_gold = 10.0 * (5.0_f64.sqrt() - 1.0) * 0.5;

    for (tag, x, p, exp, rtol, atol) in [
        ("low-Et", 0.1, p_ext, 100.0 * (1e-3 / (0.1 + 1e-3)), 1e-12, 1e-14),
        ("picomolar", 1e-5, [250.0, 1e-12, 1e-10, 1e-3, 1e-3], exp_nano, 1e-13, 1e-15),
        ("1e200", 1e200, [10.0, 1e200, 1e200, 0.0, 1.0], exp_gold, 1e-12, 1e-14),
        ("1e-200", 1e-200, [10.0, 1e-200, 1e-200, 0.0, 1.0], exp_gold, 1e-12, 1e-14),
    ] {
        assert_rel_close(model.eval(&[x], &p, 0).unwrap(), exp, rtol, atol, tag);
    }

    assert_eq!(model.eval(&[0.0], &p_ext, 0), Some(100.0));
    for (x, p) in [(-0.1, p_ext), (0.1, [100.0, 1e-18, 1e-3, -1.0, 1e-3]), (f64::NAN, p_ext), (0.1, [f64::NAN, 1e-18, 1e-3, 0.0, 1e-3])] {
        assert_eq!(model.eval(&[x], &p, 0), None);
    }
}

#[test]
fn fa_sec08_03_one_site_hill_and_allosteric_sigmoidal_normalized_no_nan() {
    let hill = get_builtin_model("one_site_specific_hill").unwrap();
    let allo = get_builtin_model("allosteric_sigmoidal_kinetics").unwrap();
    let sat = get_builtin_model("one_site_specific_binding").unwrap();

    for (model, label) in [(hill.as_ref(), "one_site_specific_hill"), (allo.as_ref(), "allosteric_sigmoidal")] {
        for (tag, x, p, exp, rtol, atol) in [
            ("1e20", 1e20, [100.0, 1e20, 20.0], 50.0, 1e-12, 1e-12),
            ("1e-20", 1e-20, [100.0, 1e-20, 20.0], 50.0, 1e-12, 1e-12),
            ("overflow", 1e30, [100.0, 1.0, 20.0], 100.0, 1e-12, 1e-12),
            ("underflow", 1e-30, [100.0, 1.0, 20.0], 0.0, 1e-12, 1e-12),
            ("c/kd inf", 1e200, [100.0, 1e-200, 0.05], 100.0, 1e-12, 1e-12),
            ("c/kd tiny", 1e-200, [100.0, 1e200, 0.05], 1e-18, 1e-6, 1e-24),
        ] {
            assert_rel_close(model.eval(&[x], &p, 0).unwrap(), exp, rtol, atol, &format!("{label} {tag}"));
        }
        for (x, p) in [
            (f64::NAN, [100.0, 1.0, 1.0]),
            (1.0, [100.0, 1.0, f64::NAN]),
            (1.0, [f64::NAN, 1.0, 1.0]),
            (0.0, [100.0, 1.0, -1.0]),
            (1e-30, [100.0, 1.0, -20.0]),
        ] {
            assert_eq!(model.eval(&[x], &p, 0), None);
        }
    }

    assert_rel_close(sat.eval(&[1e308], &[100.0, 1e308], 0).unwrap(), 50.0, 1e-12, 1e-12, "sat 1e308");
    assert_eq!(sat.eval(&[1e308], &[1e308, 1.0], 0), Some(1e308));
}

#[test]
fn fa_sec08_04_richards_5pl_derived_transforms_small_and_large_s_finite() {
    let model = get_builtin_model("log_agonist_vs_response_5pl").unwrap();
    for hill in [1.0_f64, -1.0, 2.5] {
        let scalars = model.derived_transforms(&[0.0, 100.0, -6.0, hill, 1e-4]);
        let log_xb = scalars.iter().find(|s| s.name == "LogXb").unwrap();
        assert_rel_close(log_xb.value, -6.0 + (10_000.0 * core::f64::consts::LOG10_2) / hill, 1e-12, 1e-12, &format!("5PL hill={hill}"));
        assert!(scalars.iter().all(|s| s.value.is_finite()));
    }

    assert_eq!(model.derived_transforms(&[0.0, 100.0, -7.5, 1.5, 1.0]).iter().find(|s| s.name == "LogXb").unwrap().value, -7.5);
    let log_xb_huge = model.derived_transforms(&[0.0, 100.0, -6.0, 1.0, 1e20]).iter().find(|s| s.name == "LogXb").unwrap().value;
    assert_rel_close(log_xb_huge, -6.0 + (core::f64::consts::LN_2 / 1e20).log10(), 1e-12, 1e-12, "5PL S=1e20");
    assert!(model.derived_transforms(&[0.0, 100.0, -6.0, 1.0, 1e-310]).iter().all(|s| s.value.is_finite()));

    let xs = [-12.0, -10.0, -8.0, -6.0, -4.0, -2.0];
    let ys: Vec<f64> = xs.iter().map(|&x| model.eval(&[x], &[5.0, 95.0, -6.0, 1.2, 1e-4], 0).unwrap()).collect();
    let sheet = xy_sheet_single("ds-5pl", "5PL small S", "Y", &xs, &ys);
    let res =
        exec_nl(&sheet, NonlinearFitSpec { parameter_rules: vec![rule_const("S", 1e-4)], ..NonlinearFitSpec::new("log_agonist_vs_response_5pl") });
    assert!(res.scalars.iter().all(|s| s.value.is_finite()), "non-finite scalars: {:?}", res.scalars);

    // AUD-03-14: log-space eval continuity across u = 700, exact eval(LogEC50) == 50, extreme tails & NaN/S<=0 guards.
    for s in [1e-4_f64, 1e-3] {
        let params = [0.0, 100.0, -6.0, 1.0, s];
        let log_xb = model.derived_transforms(&params).into_iter().find(|d| d.name == "LogXb").unwrap().value;
        let (x_lo, x_hi) = (log_xb - 700.0 / core::f64::consts::LN_10, log_xb - 700.001 / core::f64::consts::LN_10);
        let (y_lo, y_hi) = (model.eval(&[x_lo], &params, 0).unwrap(), model.eval(&[x_hi], &params, 0).unwrap());
        assert_approx_eq(y_lo, 100.0 * (-s * 700.0).exp(), 1e-10, 1e-12, &format!("5PL u=700 S={s}"));
        assert_approx_eq(y_hi, 100.0 * (-s * 700.001).exp(), 1e-10, 1e-12, &format!("5PL u=700.001 S={s}"));
        assert!((y_lo - y_hi).abs() < 1e-3, "5PL discontinuity at u=700 for S={s}: {y_lo} -> {y_hi}");
        assert_approx_eq(model.eval(&[-6.0], &params, 0).unwrap(), 50.0, 1e-10, 1e-11, &format!("5PL eval(LogEC50) S={s}"));
    }
    assert_approx_eq(model.eval(&[10_000.0], &[5.0, 95.0, -6.0, 1.0, 1e-4], 0).unwrap(), 95.0, 1e-12, 1e-12, "5PL upper tail");
    assert_approx_eq(model.eval(&[-1e9], &[5.0, 95.0, -6.0, 1.0, 1e-4], 0).unwrap(), 5.0, 1e-12, 1e-12, "5PL lower tail");
    for (x, p) in [(f64::NAN, [5.0, 95.0, -6.0, 1.0, 1.0]), (-6.0, [f64::NAN, 95.0, -6.0, 1.0, 1.0]), (-6.0, [5.0, 95.0, -6.0, 1.0, 0.0])] {
        assert!(model.eval(&[x], &p, 0).is_none());
    }
}

#[test]
fn fa_sec08_05_lowercase_local_t_forward_and_out_of_scope_rejected() {
    for bad in [
        "u = t * k\nt = X^2\nY = u",
        "t = t + 1\nY = t",
        "<A> t = 2 * X\n<A> Y = a * t\n<B> Y = b * t",
        "<A> t = 2 * X\n<A> Y = a * t\n<~A> Y = b * t",
        "Y0 = t + 1\nt = X^2\ndY/dX = -k * Y",
        "<A> t = 2\n<B> Y0 = t\n<B> dY/dX = -k * Y",
    ] {
        assert_dsl_compile_err(bad);
    }

    let valid = dsl_compile("t = 2 * X\nY = a * t");
    assert_eq!(valid.param_names(), &["a"]);
    assert_eq!(valid.eval(&[3.0], &[5.0], 0), Some(30.0));

    let alias_t = dsl_compile("Y = A * exp(-t * X)");
    assert_eq!(alias_t.param_names(), &["A"]);
    assert!(alias_t.warnings().iter().any(|w| w.kind == DslWarningKind::AliasTWithPredictorX && w.identifier == "t"));
    assert!(dsl_compile("Y0 = A0\ndY/dt = -k * Y + c * t").warnings().is_empty());
}

#[test]
fn fa_sec08_06_kinetic_binding_evals_guard_exp_overflow_and_nan() {
    let p_neg_t0 = [100.0_f64, 1e6, 0.1, 1.0, -1e-3];
    for &(id, x, p) in &[
        ("assoc_then_dissoc", 0.0, &p_neg_t0[..]),
        ("assoc_then_dissoc", 1e4, &p_neg_t0[..]),
        ("assoc_then_dissoc", -1.0, &p_neg_t0[..]),
        ("dissociation_one_phase", f64::NAN, &[100.0, 5.0, 0.1][..]),
        ("dissociation_one_phase", 1.0, &[f64::NAN, 5.0, 0.1][..]),
        ("dissociation_one_phase", -1e5, &[100.0, 5.0, 0.1][..]),
        ("association_kinetics_one_conc", f64::NAN, &[1e6, 100.0, 0.1, 10.0][..]),
        ("association_kinetics_one_conc", 1.0, &[f64::NAN, 100.0, 0.1, 10.0][..]),
        ("association_kinetics_one_conc", -1e5, &[1e6, 100.0, 0.1, 10.0][..]),
        ("association_kinetics_two_conc", f64::NAN, &[100.0, 1e6, 0.1, 1e-8][..]),
        ("association_kinetics_two_conc", 1.0, &[f64::NAN, 1e6, 0.1, 1e-8][..]),
        ("association_kinetics_two_conc", -1e5, &[100.0, 1e6, 0.1, 1e-8][..]),
        ("assoc_then_dissoc", f64::NAN, &[100.0, 1e6, 0.1, 1e-8, 10.0][..]),
        ("assoc_then_dissoc", 15.0, &[f64::NAN, 1e6, 0.1, 1e-8, 10.0][..]),
    ] {
        assert_eq!(get_builtin_model(id).unwrap().eval(&[x], p, 0), None, "{id} x={x}");
    }

    let ad = get_builtin_model("assoc_then_dissoc").unwrap();
    let v_ad = ad.eval(&[20.0], &[100.0, 1e6, 0.1, 1e-7, 10.0], 0).unwrap();
    assert_rel_close(v_ad, 50.0 * (1.0 - (-0.2_f64 * 10.0).exp()) * (-0.1_f64 * 10.0).exp(), 1e-12, 1e-12, "valid assoc_then_dissoc");
}

#[test]
fn fa_sec08_07_strip_comment_earliest_delimiter() {
    let m1 = dsl_compile("; note #1\nY = A * X ; sample #2");
    assert_eq!(m1.param_names(), &["A"]);
    assert_eq!(m1.eval(&[3.0], &[4.0], 0), Some(12.0));
    assert_eq!(dsl_compile("Y = A * X ; disabled # initial: A = 99").initial_guess(&[DataPoint::xy(1.0, 2.0)]), vec![1.0]);
    assert_eq!(dsl_compile("Y = A * X ; initial: A = 7").initial_guess(&[DataPoint::xy(1.0, 2.0)]), vec![7.0]);
    assert_eq!(dsl_compile("Y = A * X # sample ;1").eval(&[3.0], &[4.0], 0), Some(12.0));
}

#[test]
fn fa_sec08_08_multi_branch_ode_missing_ic_compile_error() {
    for bad in [
        "<A> Y0 = yA\n<A> dY/dX = -k1 * Y\n<B> dY/dX = -k2 * Y",
        "<A> Y0 = yA\n<A> dY/dX = -k1 * Y\ndY/dX = -k2 * Y",
        "<A> Y0 = yA\n<A> dY/dX = -k1 * Y\n<~A> dY/dX = -k2 * Y",
        "<A> Y0 = yA\n<A> k_eff = k1\n<B> k_eff = k2\ndY/dX = -k_eff * Y",
    ] {
        assert_dsl_compile_err(bad);
    }

    let ok = dsl_compile("<A> Y0 = yA\nY0 = yDef\n<A> dY/dX = -k1 * Y\ndY/dX = -k2 * Y");
    assert_rel_close(ok.eval(&[2.0], &[0.5, 1.0, 10.0, 30.0], 0).unwrap(), 10.0 * (-1.0_f64).exp(), 1e-6, 1e-9, "ds0 <A> IC");
    assert_rel_close(ok.eval(&[2.0], &[0.5, 1.0, 10.0, 30.0], 1).unwrap(), 30.0 * (-2.0_f64).exp(), 1e-6, 1e-9, "ds1 fallback IC");
}

#[test]
fn fa_sec08_09_weibull_peak_x_zero_continuity_and_finite_guard() {
    let model = get_builtin_model("weibull_peak").unwrap();
    let y0_exp = model.eval(&[0.0], &[100.0, 2.0, 1.0], 0).unwrap();
    assert_rel_close(y0_exp, 50.0, 1e-12, 1e-12, "Weibull Shape=1 X=0");
    assert_rel_close(y0_exp, model.eval(&[1e-12], &[100.0, 2.0, 1.0], 0).unwrap(), 1e-10, 1e-10, "Weibull continuity");
    assert_eq!(model.eval(&[0.0], &[100.0, 2.0, 2.0], 0), Some(0.0));
    for (x, p) in [(0.0, [100.0, 2.0, 0.5]), (1e-300, [1e100, 1.0, 0.1]), (f64::NAN, [100.0, 2.0, 1.0]), (1.0, [f64::NAN, 2.0, 1.0])] {
        assert_eq!(model.eval(&[x], &p, 0), None);
    }
}

#[test]
fn fa_sec08_10_initial_guess_filters_non_finite_points() {
    let four_pl = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let g_span = four_pl.initial_guess(&[DataPoint::xy(1.0, 10.0), DataPoint::xy(f64::NAN, 30.0), DataPoint::xy(3.0, 20.0)]);
    assert!(g_span.iter().all(|v| v.is_finite()));
    assert_rel_close(g_span[2], 2.0, 1e-12, 1e-12, "4PL LogEC50 midpoint");
    assert!(four_pl.initial_guess(&[DataPoint::xy(1.0, 10.0), DataPoint::xy(f64::NAN, 30.0)]).iter().all(|v| v.is_finite()));

    let g_gauss = get_builtin_model("gaussian").unwrap().initial_guess(&[
        DataPoint::xy(1.0, 5.0),
        DataPoint::xy(2.0, 10.0),
        DataPoint::xy(999.0, f64::NAN),
        DataPoint::xy(f64::NAN, 1000.0),
    ]);
    assert!(g_gauss.iter().all(|v| v.is_finite()));
    assert_rel_close(g_gauss[1], 2.0, 1e-12, 1e-12, "gaussian Mean");

    let pts_global = vec![
        DataPoint { data_set_index: 0, row: 0, ..DataPoint::xy(1.0, 20.0) },
        DataPoint { data_set_index: 0, row: 1, ..DataPoint::xy(10.0, 80.0) },
        DataPoint { data_set_index: 1, row: 0, ..DataPoint::xy(1.0, 2.0) },
        DataPoint { data_set_index: 1, row: 1, ..DataPoint::xy(10.0, f64::NAN) },
    ];
    assert!(get_builtin_model("one_site_total_and_ns_global").unwrap().initial_guess(&pts_global).iter().all(|v| v.is_finite()));

    let g_dsl =
        dsl_compile("Y = Y0 + A * X").initial_guess(&[DataPoint::xy(1.0, 20.0), DataPoint::xy(2.0, f64::NAN), DataPoint::xy(3.0, f64::INFINITY)]);
    assert!(g_dsl.iter().all(|v| v.is_finite()));
    assert_rel_close(g_dsl[0], 20.0, 1e-12, 1e-12, "DSL Y0 guess");
}

#[test]
fn fa_sec08_11_predictor_index_x00_and_overflow_consistent() {
    for &(src, params, check_no_warn, x, p, exp) in &[
        ("Y = X00 * a", &["X00", "a"][..], true, &[10.0][..], &[2.0, 5.0][..], 10.0),
        ("Y = X999999999999999999999999999 * a", &["X999999999999999999999999999", "a"][..], true, &[10.0][..], &[3.0, 4.0][..], 12.0),
        ("Y = X01 * a + X2 * b", &["a", "b"][..], false, &[3.0, 7.0][..], &[2.0, 5.0][..], 41.0),
    ] {
        let m = dsl_compile(src);
        assert_eq!(m.param_names(), params, "src={src}");
        if check_no_warn {
            assert!(m.warnings().is_empty(), "src={src}: {:?}", m.warnings());
        }
        assert_eq!(m.eval(x, p, 0), Some(exp));
    }
}

#[test]
fn fa_sec08_12_opcode_budget_error_reports_remaining_limit() {
    let mut src = String::from("<A> a = X + 1\n");
    for _ in 0..7_500 {
        src.push_str("<A> a = a + p - q + 1\n");
    }
    src.push_str("<A> Y = a\n<B> b = X + 1\n");
    for _ in 0..7_500 {
        src.push_str("<B> b = b + p - q + 1\n");
    }
    src.push_str("<B> Y = b");

    let msg = CompiledDslModel::compile("multi_branch_budget", &src).unwrap_err().to_string();
    assert!(
        msg.contains("remaining bytecode budget") && msg.contains("of 100000 total opcodes") && !msg.contains("exceeds total bytecode budget"),
        "got: {msg}"
    );
}
