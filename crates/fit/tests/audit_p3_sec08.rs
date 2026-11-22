//! Regression suite for SEC-08 findings (`FA-SEC08-01` .. `FA-SEC08-12`) in `crates/fit`.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, NonlinearFitSpec, ParameterRule, ParameterScope, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_fit::dsl::DslWarningKind;
use curvebook_fit::{CompiledDslModel, DataPoint, DslEquationKind, FitError, HookContext, ParametricModel, execute_analysis, get_builtin_model};

fn assert_rel_close(actual: f64, expected: f64, rel_tol: f64, abs_tol: f64, ctx: &str) {
    assert!(actual.is_finite() && expected.is_finite(), "{ctx}: non-finite values actual={actual}, expected={expected}");
    let diff = (actual - expected).abs();
    let tol = abs_tol.max(rel_tol * expected.abs());
    assert!(diff <= tol, "{ctx}: actual={actual:.16e}, expected={expected:.16e}, diff={diff:.3e}, tol={tol:.3e}");
}

// ============================================================================
// FA-SEC08-01: DSL dataset-scoped branch precedence (<A>/<~A> vs trailing Any)
// ============================================================================

#[test]
fn fa_sec08_01_dsl_dataset_scoped_branch_precedence_over_trailing_fallback() {
    // 1) Specific `<A>` declared BEFORE unprefixed fallback must not be overwritten.
    let src_a_first = "\
<A> Y = Baseline
Y = Bottom + (Top - Bottom) / (1 + 10^(LogEC50 - X))";
    let m_a_first = CompiledDslModel::compile("sec08_01_a_first", src_a_first).expect("<A> before fallback compiles");
    assert_eq!(m_a_first.param_names(), &["Baseline", "Bottom", "Top", "LogEC50"]);
    // Params: [Baseline=42, Bottom=10, Top=100, LogEC50=0]
    let params_a = [42.0_f64, 10.0, 100.0, 0.0];
    assert_rel_close(m_a_first.eval(&[0.0], &params_a, 0).expect("ds0"), 42.0, 1e-12, 1e-12, "<A> evaluates Baseline");
    assert_rel_close(m_a_first.eval(&[0.0], &params_a, 1).expect("ds1"), 55.0, 1e-12, 1e-12, "ds1 falls back to 4PL");

    // 2) Negated `<~A>` declared BEFORE unprefixed fallback must win on `~A` (`ds >= 1`).
    let src_not_a_first = "\
<~A> Y = a * X
Y = b * X";
    let m_not_a = CompiledDslModel::compile("sec08_01_not_a_first", src_not_a_first).expect("<~A> before fallback compiles");
    assert_eq!(m_not_a.param_names(), &["b", "a"]);
    let p_ab = [10.0_f64, 3.0]; // b = 10, a = 3
    assert_rel_close(m_not_a.eval(&[4.0], &p_ab, 0).expect("ds0"), 40.0, 1e-12, 1e-12, "ds0 uses b*X");
    assert_rel_close(m_not_a.eval(&[4.0], &p_ab, 1).expect("ds1"), 12.0, 1e-12, 1e-12, "ds1 uses <~A> a*X");
    assert_rel_close(m_not_a.eval(&[4.0], &p_ab, 2).expect("ds2"), 12.0, 1e-12, 1e-12, "ds2 uses <~A> a*X");

    // 3) Specific branch preceding `Any` local + `Any` fallback still compiles and evaluates both cleanly.
    let src_with_any_local = "\
<A> Y = Baseline
m = 3
Y = m * X";
    let m_any_loc = CompiledDslModel::compile("sec08_01_any_loc", src_with_any_local).expect("specific before Any local+target compiles");
    assert_eq!(m_any_loc.param_names(), &["Baseline"]);
    assert_eq!(m_any_loc.eval(&[5.0], &[7.0], 0), Some(7.0));
    assert_eq!(m_any_loc.eval(&[5.0], &[7.0], 1), Some(15.0));

    let src_not_a_with_any_local = "\
<~A> Y = a * X
m = 3
Y = m * X";
    let m_not_a_loc = CompiledDslModel::compile("sec08_01_not_a_any_loc", src_not_a_with_any_local).expect("<~A> before Any local+target compiles");
    assert_eq!(m_not_a_loc.param_names(), &["a"]);
    assert_eq!(m_not_a_loc.eval(&[5.0], &[4.0], 0), Some(15.0));
    assert_eq!(m_not_a_loc.eval(&[5.0], &[4.0], 1), Some(20.0));

    // A dangling `Any` local with NO `Any` target afterward must still be rejected as trailing.
    let trailing = CompiledDslModel::compile("sec08_01_trailing", "<A> Y = Baseline\nm = 3");
    assert!(matches!(trailing, Err(FitError::DslCompileError { .. })));

    // 4) ODE derivative specificity `<A> dY/dX` before fallback `dY/dX` must not be overwritten.
    let src_ode = "\
<A> Y0 = A0
<A> dY/dX = -k1 * Y
Y0 = B0
dY/dX = -k2 * Y";
    let m_ode = CompiledDslModel::compile("sec08_01_ode_spec", src_ode).expect("ODE specificity compiles");
    assert_eq!(m_ode.kind(), DslEquationKind::OdeFirstOrder);
    // params: [k1, k2, A0, B0]
    let p_ode = [0.5_f64, 1.5, 20.0, 50.0];
    assert_rel_close(m_ode.eval(&[2.0], &p_ode, 0).expect("ds0 ode"), 20.0 * (-1.0_f64).exp(), 1e-6, 1e-9, "<A> ODE uses k1 & A0");
    assert_rel_close(m_ode.eval(&[2.0], &p_ode, 1).expect("ds1 ode"), 50.0 * (-3.0_f64).exp(), 1e-6, 1e-9, "ds1 ODE uses k2 & B0");
}

// ============================================================================
// FA-SEC08-02: MorrisonKi cancellation-free hypot + rationalized discriminant
// ============================================================================

#[test]
fn fa_sec08_02_morrison_ki_low_et_and_excess_inhibitor_no_cancellation() {
    let model = get_builtin_model("morrison_tight_binding_ki").expect("morrison_tight_binding_ki exists");

    // 1) Extreme low-Et / excess-I regime where naive sum^2 - 4*Et*I rounded disc == sum.
    let p_extreme = [100.0_f64, 1e-18, 1e-3, 0.0, 1e-3];
    let v_extreme = model.eval(&[0.1], &p_extreme, 0).expect("extreme low-Et eval");
    let expected_extreme = 100.0 * (1e-3 / (0.1 + 1e-3));
    assert_rel_close(v_extreme, expected_extreme, 1e-12, 1e-14, "Morrison low-Et I=0.1");

    // 2) Nanomolar/picomolar regime (Et=1e-12, Ki=1e-10, S=1e-3, Km=1e-3, I=1e-5).
    let p_nano = [250.0_f64, 1e-12, 1e-10, 1e-3, 1e-3];
    let v_nano = model.eval(&[1e-5], &p_nano, 0).expect("picomolar Et eval");
    let ki_app = 2e-10_f64;
    let expected_nano = 250.0 * (2.0 * ki_app) / ((1e-5 + ki_app - 1e-12) + (1e-12 - 1e-5 + ki_app).hypot(2.0 * (1e-5_f64).sqrt() * ki_app.sqrt()));
    assert_rel_close(v_nano, expected_nano, 1e-13, 1e-15, "Morrison nanomolar/picomolar accuracy");

    // 3) Huge and tiny symmetric scales avoid intermediate overflow/underflow.
    let expected_golden = 10.0 * (5.0_f64.sqrt() - 1.0) * 0.5;
    assert_rel_close(
        model.eval(&[1e200], &[10.0, 1e200, 1e200, 0.0, 1.0], 0).expect("huge scale"),
        expected_golden,
        1e-12,
        1e-14,
        "Morrison 1e200 scale",
    );
    assert_rel_close(
        model.eval(&[1e-200], &[10.0, 1e-200, 1e-200, 0.0, 1.0], 0).expect("tiny scale"),
        expected_golden,
        1e-12,
        1e-14,
        "Morrison 1e-200 scale",
    );

    // 4) Boundaries & non-finite domain guards.
    assert_eq!(model.eval(&[0.0], &p_extreme, 0), Some(100.0));
    assert_eq!(model.eval(&[-0.1], &p_extreme, 0), None);
    assert_eq!(model.eval(&[0.1], &[100.0, 1e-18, 1e-3, -1.0, 1e-3], 0), None);
    assert_eq!(model.eval(&[f64::NAN], &p_extreme, 0), None);
    assert_eq!(model.eval(&[0.1], &[f64::NAN, 1e-18, 1e-3, 0.0, 1e-3], 0), None);
}

// ============================================================================
// FA-SEC08-03: OneSiteBinding (with_hill) & AllostericSigmoidal normalization
// ============================================================================

#[test]
fn fa_sec08_03_one_site_hill_and_allosteric_sigmoidal_normalized_no_nan() {
    let hill = get_builtin_model("one_site_specific_hill").expect("one_site_specific_hill exists");
    let allo = get_builtin_model("allosteric_sigmoidal_kinetics").expect("allosteric_sigmoidal_kinetics exists");
    let sat = get_builtin_model("one_site_specific_binding").expect("one_site_specific_binding exists");

    for (model, label) in [(hill.as_ref(), "one_site_specific_hill"), (allo.as_ref(), "allosteric_sigmoidal")] {
        // Within OneSiteBinding::default_bounds: h in [0.05, 20.0], kd >= 1e-30.
        assert_rel_close(model.eval(&[1e20], &[100.0, 1e20, 20.0], 0).expect("large c=kd"), 50.0, 1e-12, 1e-12, &format!("{label} 1e20"));
        assert_rel_close(model.eval(&[1e-20], &[100.0, 1e-20, 20.0], 0).expect("tiny c=kd"), 50.0, 1e-12, 1e-12, &format!("{label} 1e-20"));
        assert_rel_close(model.eval(&[1e30], &[100.0, 1.0, 20.0], 0).expect("ratio overflow"), 100.0, 1e-12, 1e-12, &format!("{label} overflow"));
        assert_rel_close(model.eval(&[1e-30], &[100.0, 1.0, 20.0], 0).expect("ratio underflow"), 0.0, 1e-12, 1e-12, &format!("{label} underflow"));
        assert_rel_close(model.eval(&[1e200], &[100.0, 1e-200, 0.05], 0).expect("c/kd inf"), 100.0, 1e-12, 1e-12, &format!("{label} c/kd inf"));
        assert_rel_close(model.eval(&[1e-200], &[100.0, 1e200, 0.05], 0).expect("c/kd underflow"), 1e-18, 1e-6, 1e-24, &format!("{label} c/kd tiny"));

        // NaN inputs always yield None, never Some(NaN).
        assert_eq!(model.eval(&[f64::NAN], &[100.0, 1.0, 1.0], 0), None);
        assert_eq!(model.eval(&[1.0], &[100.0, 1.0, f64::NAN], 0), None);
        assert_eq!(model.eval(&[1.0], &[f64::NAN, 1.0, 1.0], 0), None);
        assert_eq!(model.eval(&[0.0], &[100.0, 1.0, -1.0], 0), None);
        assert_eq!(model.eval(&[1e-30], &[100.0, 1.0, -20.0], 0), None);
    }

    // Non-hill OneSiteBinding handles huge bmax & c without overflow to NaN.
    assert_rel_close(sat.eval(&[1e308], &[100.0, 1e308], 0).expect("sat 1e308"), 50.0, 1e-12, 1e-12, "sat 1e308");
    assert_eq!(sat.eval(&[1e308], &[1e308, 1.0], 0), Some(1e308));
}

// ============================================================================
// FA-SEC08-04: RichardsFivePl derived_transforms small/large S finiteness
// ============================================================================

#[test]
fn fa_sec08_04_richards_5pl_derived_transforms_small_and_large_s_finite() {
    let model = get_builtin_model("log_agonist_vs_response_5pl").expect("5pl exists");

    // 1) Small S inside default_bounds ([1e-4, 100.0]) with positive & negative HillSlope.
    for hill in [1.0_f64, -1.0_f64, 2.5_f64] {
        let params = [0.0, 100.0, -6.0, hill, 1e-4];
        let scalars = model.derived_transforms(&params);
        let log_ec50 = scalars.iter().find(|s| s.name == "LogEC50").expect("LogEC50 emitted for S=1e-4");
        assert!(log_ec50.value.is_finite(), "LogEC50 must be finite for hill={hill}, got {}", log_ec50.value);
        let expected_log_ec50 = -6.0 - (10_000.0 * core::f64::consts::LOG10_2) / hill;
        assert_rel_close(log_ec50.value, expected_log_ec50, 1e-12, 1e-12, &format!("5PL S=1e-4 hill={hill}"));
        assert!(scalars.iter().all(|s| s.value.is_finite()), "all derived scalars must be finite: {scalars:?}");
    }

    // 2) Exact S=1.0 and large S=1e20 avoid cancellation/underflow to -Inf.
    let s1 = model.derived_transforms(&[0.0, 100.0, -7.5, 1.5, 1.0]);
    let log_ec50_s1 = s1.iter().find(|s| s.name == "LogEC50").unwrap().value;
    assert_eq!(log_ec50_s1, -7.5);

    let s_huge = model.derived_transforms(&[0.0, 100.0, -6.0, 1.0, 1e20]);
    let log_ec50_huge = s_huge.iter().find(|s| s.name == "LogEC50").expect("LogEC50 emitted for S=1e20");
    assert!(log_ec50_huge.value.is_finite());
    let expected_huge = -6.0 - (core::f64::consts::LN_2 / 1e20).log10();
    assert_rel_close(log_ec50_huge.value, expected_huge, 1e-12, 1e-12, "5PL S=1e20");

    // 3) Extreme sub-normal S=1e-310 filters non-finite LogEC50 instead of emitting Inf.
    let s_under = model.derived_transforms(&[0.0, 100.0, -6.0, 1.0, 1e-310]);
    assert!(s_under.iter().all(|s| s.value.is_finite()), "must never emit non-finite scalars: {s_under:?}");

    // 4) End-to-end execute_analysis on 5PL with S fixed at 1e-4 emits finite scalars only.
    let xs = [-12.0, -10.0, -8.0, -6.0, -4.0, -2.0];
    let p_true = [5.0_f64, 95.0, -6.0, 1.2, 1e-4];
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
    let mut sheet = DataSheet::new("ds-5pl", "5PL small S", TableArchetype::Xy(xy.clone())).unwrap();
    for (row, &x) in xs.iter().enumerate() {
        let y = model.eval(&[x], &p_true, 0).unwrap();
        xy.set_x_cell(row, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[row][0] = Cell::number(y);
        sheet.set_x_or_time_cell(row, 0, Cell::number(x)).unwrap();
        sheet.set_cell(CellCoord::new(row, 0, 0), Cell::number(y)).unwrap();
    }
    let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_5pl");
    nl.parameter_rules = vec![ParameterRule {
        name: "S".to_owned(),
        scope: ParameterScope::Constant(1e-4),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }];
    let spec = AnalysisSpec::new("ana-5pl", sheet.id.clone(), AnalysisKind::NonlinearRegression(nl)).unwrap();
    let res = execute_analysis(&spec, &sheet, &HookContext::default()).expect("execute_analysis 5pl succeeds");
    assert!(res.scalars.iter().all(|s| s.value.is_finite()), "ResultSheet scalars contain non-finite value: {:?}", res.scalars);
}

// ============================================================================
// FA-SEC08-05: Unbound/out-of-scope lowercase local `t` raises compile errors
// ============================================================================

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
        let err = CompiledDslModel::compile("bad_local_t", bad);
        assert!(matches!(err, Err(FitError::DslCompileError { .. })), "expected DslCompileError for:\n{bad}");
    }

    // Valid local `t` defined before use still compiles and computes the local expression.
    let valid_local = CompiledDslModel::compile("valid_local_t", "t = 2 * X\nY = a * t").expect("valid local t compiles");
    assert_eq!(valid_local.param_names(), &["a"]);
    assert_eq!(valid_local.eval(&[3.0], &[5.0], 0), Some(30.0));

    // Unbound non-local `t` still resolves to X with appropriate warning behavior.
    let alias_t = CompiledDslModel::compile("alias_t", "Y = A * exp(-t * X)").expect("alias t compiles");
    assert_eq!(alias_t.param_names(), &["A"]);
    assert!(alias_t.warnings().iter().any(|w| w.kind == DslWarningKind::AliasTWithPredictorX && w.identifier == "t"));

    let ode_dt = CompiledDslModel::compile("ode_dt", "Y0 = A0\ndY/dt = -k * Y + c * t").expect("dY/dt compiles");
    assert!(ode_dt.warnings().is_empty());
}

// ============================================================================
// FA-SEC08-06: Kinetic binding evals guard exp overflow & NaN inputs
// ============================================================================

#[test]
fn fa_sec08_06_kinetic_binding_evals_guard_exp_overflow_and_nan() {
    let dissoc = get_builtin_model("dissociation_one_phase").expect("dissociation_one_phase");
    let assoc1 = get_builtin_model("association_kinetics_one_conc").expect("association_kinetics_one_conc");
    let assoc2 = get_builtin_model("association_kinetics_two_conc").expect("association_kinetics_two_conc");
    let ad = get_builtin_model("assoc_then_dissoc").expect("assoc_then_dissoc");

    // 1) AssocThenDissoc negative Time0 exp overflow at t >= Time0 must return None, never -Inf/NaN.
    let p_neg_t0 = [100.0_f64, 1e6, 0.1, 1.0, -1e-3];
    assert_eq!(ad.eval(&[0.0], &p_neg_t0, 0), None);
    assert_eq!(ad.eval(&[1e4], &p_neg_t0, 0), None);
    assert_eq!(ad.eval(&[-1.0], &p_neg_t0, 0), None);

    // 2) NaN inputs across all four kinetic models return None (never Some(NaN)).
    assert_eq!(dissoc.eval(&[f64::NAN], &[100.0, 5.0, 0.1], 0), None);
    assert_eq!(dissoc.eval(&[1.0], &[f64::NAN, 5.0, 0.1], 0), None);
    assert_eq!(dissoc.eval(&[-1e5], &[100.0, 5.0, 0.1], 0), None);

    assert_eq!(assoc1.eval(&[f64::NAN], &[100.0, 0.1], 0), None);
    assert_eq!(assoc1.eval(&[1.0], &[f64::NAN, 0.1], 0), None);
    assert_eq!(assoc1.eval(&[-1e5], &[100.0, 0.1], 0), None);

    assert_eq!(assoc2.eval(&[f64::NAN], &[100.0, 1e6, 0.1, 1e-8], 0), None);
    assert_eq!(assoc2.eval(&[1.0], &[f64::NAN, 1e6, 0.1, 1e-8], 0), None);
    assert_eq!(assoc2.eval(&[-1e5], &[100.0, 1e6, 0.1, 1e-8], 0), None);

    assert_eq!(ad.eval(&[f64::NAN], &[100.0, 1e6, 0.1, 1e-8, 10.0], 0), None);
    assert_eq!(ad.eval(&[15.0], &[f64::NAN, 1e6, 0.1, 1e-8, 10.0], 0), None);

    // 3) Valid values still evaluate accurately.
    let v_ad = ad.eval(&[20.0], &[100.0, 1e6, 0.1, 1e-7, 10.0], 0).expect("valid assoc_then_dissoc");
    let kobs = 0.2_f64;
    let eq = 50.0_f64;
    let expected_ad = eq * (1.0 - (-kobs * 10.0).exp()) * (-0.1_f64 * 10.0).exp();
    assert_rel_close(v_ad, expected_ad, 1e-12, 1e-12, "valid assoc_then_dissoc");
}

// ============================================================================
// FA-SEC08-07: strip_comment selects earliest `#` or `;` delimiter
// ============================================================================

#[test]
fn fa_sec08_07_strip_comment_earliest_delimiter() {
    let m1 = CompiledDslModel::compile("semi_then_hash", "; note #1\nY = A * X ; sample #2").expect("semicolon comment containing # compiles");
    assert_eq!(m1.param_names(), &["A"]);
    assert_eq!(m1.eval(&[3.0], &[4.0], 0), Some(12.0));

    // `;` before `# initial:` means `# initial:` is part of the semicolon comment and must not set hint.
    let m_disabled_hint = CompiledDslModel::compile("disabled_hint", "Y = A * X ; disabled # initial: A = 99").expect("disabled hint compiles");
    assert_eq!(m_disabled_hint.initial_guess(&[DataPoint::xy(1.0, 2.0)]), vec![1.0]);

    // `; initial:` still works when `;` is the earliest comment marker.
    let m_semi_hint = CompiledDslModel::compile("semi_hint", "Y = A * X ; initial: A = 7").expect("semicolon initial hint compiles");
    assert_eq!(m_semi_hint.initial_guess(&[DataPoint::xy(1.0, 2.0)]), vec![7.0]);

    // `#` before `;` continues to work.
    let m_hash_first = CompiledDslModel::compile("hash_then_semi", "Y = A * X # sample ;1").expect("hash comment containing ; compiles");
    assert_eq!(m_hash_first.eval(&[3.0], &[4.0], 0), Some(12.0));
}

// ============================================================================
// FA-SEC08-08: Multi-branch ODE validates missing per-branch ICs at compile time
// ============================================================================

#[test]
fn fa_sec08_08_multi_branch_ode_missing_ic_compile_error() {
    for bad in [
        "<A> Y0 = yA\n<A> dY/dX = -k1 * Y\n<B> dY/dX = -k2 * Y",
        "<A> Y0 = yA\n<A> dY/dX = -k1 * Y\ndY/dX = -k2 * Y",
        "<A> Y0 = yA\n<A> dY/dX = -k1 * Y\n<~A> dY/dX = -k2 * Y",
        "<A> Y0 = yA\n<A> k_eff = k1\n<B> k_eff = k2\ndY/dX = -k_eff * Y",
    ] {
        let err = CompiledDslModel::compile("ode_missing_ic", bad);
        assert!(matches!(err, Err(FitError::DslCompileError { .. })), "expected compile error for missing ODE IC:\n{bad}");
    }

    // Providing a fallback `Y0 = yDef` satisfies every branch cleanly.
    let ok = CompiledDslModel::compile("ode_with_default_ic", "<A> Y0 = yA\nY0 = yDef\n<A> dY/dX = -k1 * Y\ndY/dX = -k2 * Y")
        .expect("ODE with fallback IC compiles");
    // params: [k1, k2, yA, yDef]
    let p = [0.5_f64, 1.0, 10.0, 30.0];
    assert_rel_close(ok.eval(&[2.0], &p, 0).expect("ds0"), 10.0 * (-1.0_f64).exp(), 1e-6, 1e-9, "ds0 with <A> IC");
    assert_rel_close(ok.eval(&[2.0], &p, 1).expect("ds1"), 30.0 * (-2.0_f64).exp(), 1e-6, 1e-9, "ds1 with fallback IC");
}

// ============================================================================
// FA-SEC08-09: WeibullPeak continuity at X=0 (Shape <= 1) and finite check
// ============================================================================

#[test]
fn fa_sec08_09_weibull_peak_x_zero_continuity_and_finite_guard() {
    let model = get_builtin_model("weibull_peak").expect("weibull_peak exists");

    // Shape == 1.0: Exponential limit Amplitude / Scale at X = 0.
    let y0_exp = model.eval(&[0.0], &[100.0, 2.0, 1.0], 0).expect("Shape=1 at X=0");
    let y_near0 = model.eval(&[1e-12], &[100.0, 2.0, 1.0], 0).expect("Shape=1 at X=1e-12");
    assert_rel_close(y0_exp, 50.0, 1e-12, 1e-12, "Weibull Shape=1 X=0");
    assert_rel_close(y0_exp, y_near0, 1e-10, 1e-10, "Weibull Shape=1 continuity at 0");

    // Shape < 1.0: Pole at X = 0 returns None.
    assert_eq!(model.eval(&[0.0], &[100.0, 2.0, 0.5], 0), None);

    // Shape > 1.0: Smooth zero at X = 0.
    assert_eq!(model.eval(&[0.0], &[100.0, 2.0, 2.0], 0), Some(0.0));

    // Overflowing tiny positive X with Shape < 1.0 returns None instead of Some(Inf).
    assert_eq!(model.eval(&[1e-300], &[1e100, 1.0, 0.1], 0), None);
    assert_eq!(model.eval(&[f64::NAN], &[100.0, 2.0, 1.0], 0), None);
    assert_eq!(model.eval(&[1.0], &[f64::NAN, 2.0, 1.0], 0), None);
}

// ============================================================================
// FA-SEC08-10: initial_guess helpers filter non-finite points
// ============================================================================

#[test]
fn fa_sec08_10_initial_guess_filters_non_finite_points() {
    let four_pl = get_builtin_model("log_agonist_vs_response_4pl").expect("4PL");
    let g_4pl_span = four_pl.initial_guess(&[DataPoint::xy(1.0, 10.0), DataPoint::xy(f64::NAN, 30.0), DataPoint::xy(3.0, 20.0)]);
    assert!(g_4pl_span.iter().all(|v| v.is_finite()), "4PL initial_guess non-finite: {g_4pl_span:?}");
    assert_rel_close(g_4pl_span[2], 2.0, 1e-12, 1e-12, "4PL LogEC50 midpoint ignoring NaN x");

    let g_4pl_only_one_finite = four_pl.initial_guess(&[DataPoint::xy(1.0, 10.0), DataPoint::xy(f64::NAN, 30.0)]);
    assert!(g_4pl_only_one_finite.iter().all(|v| v.is_finite()), "4PL single finite point: {g_4pl_only_one_finite:?}");

    let gauss = get_builtin_model("gaussian").expect("gaussian");
    let g_gauss =
        gauss.initial_guess(&[DataPoint::xy(1.0, 5.0), DataPoint::xy(2.0, 10.0), DataPoint::xy(999.0, f64::NAN), DataPoint::xy(f64::NAN, 1000.0)]);
    assert!(g_gauss.iter().all(|v| v.is_finite()), "gaussian initial_guess non-finite: {g_gauss:?}");
    assert_rel_close(g_gauss[1], 2.0, 1e-12, 1e-12, "gaussian Mean ignores NaN y and NaN x");

    let global_ns = get_builtin_model("one_site_total_and_ns_global").expect("one_site_total_and_ns_global");
    let pts_global = vec![
        DataPoint { data_set_index: 0, row: 0, ..DataPoint::xy(1.0, 20.0) },
        DataPoint { data_set_index: 0, row: 1, ..DataPoint::xy(10.0, 80.0) },
        DataPoint { data_set_index: 1, row: 0, ..DataPoint::xy(1.0, 2.0) },
        DataPoint { data_set_index: 1, row: 1, ..DataPoint::xy(10.0, f64::NAN) },
    ];
    let g_global = global_ns.initial_guess(&pts_global);
    assert!(g_global.iter().all(|v| v.is_finite()), "global NS initial_guess non-finite: {g_global:?}");

    let dsl_m = CompiledDslModel::compile("dsl_init", "Y = Y0 + A * X").expect("compiles");
    let g_dsl = dsl_m.initial_guess(&[DataPoint::xy(1.0, 20.0), DataPoint::xy(2.0, f64::NAN), DataPoint::xy(3.0, f64::INFINITY)]);
    assert!(g_dsl.iter().all(|v| v.is_finite()), "DSL initial_guess non-finite: {g_dsl:?}");
    assert_rel_close(g_dsl[0], 20.0, 1e-12, 1e-12, "DSL Y0 guess ignores NaN/Inf points");
}

// ============================================================================
// FA-SEC08-11: Predictor index X00 / overflowing X<digits> handled consistently
// ============================================================================

#[test]
fn fa_sec08_11_predictor_index_x00_and_overflow_consistent() {
    let m_x00 = CompiledDslModel::compile("x00_param", "Y = X00 * a").expect("X00 compiles consistently as parameter");
    assert_eq!(m_x00.param_names(), &["X00", "a"]);
    assert!(m_x00.warnings().is_empty(), "X00 is not a reserved predictor: {:?}", m_x00.warnings());
    assert_eq!(m_x00.eval(&[10.0], &[2.0, 5.0], 0), Some(10.0));

    let m_overflow = CompiledDslModel::compile("x_overflow_param", "Y = X999999999999999999999999999 * a")
        .expect("overflowing X<digits> compiles consistently as parameter");
    assert_eq!(m_overflow.param_names(), &["X999999999999999999999999999", "a"]);
    assert!(m_overflow.warnings().is_empty());
    assert_eq!(m_overflow.eval(&[10.0], &[3.0, 4.0], 0), Some(12.0));

    let m_x01 = CompiledDslModel::compile("x01_pred", "Y = X01 * a + X2 * b").expect("X01/X2 compile as predictors");
    assert_eq!(m_x01.param_names(), &["a", "b"]);
    assert_eq!(m_x01.eval(&[3.0, 7.0], &[2.0, 5.0], 0), Some(41.0));
}

// ============================================================================
// FA-SEC08-12: Remaining opcode-budget diagnostic on multi-branch DSL
// ============================================================================

#[test]
fn fa_sec08_12_opcode_budget_error_reports_remaining_limit() {
    // Each `a = a + p - q + 1` compiles to 7 opcodes (LoadLocal, LoadParam, Add, LoadParam, Sub, PushConst, Add, StoreLocal = 8 opcodes).
    // Branch <A> uses ~60_000 opcodes (< 100_000), leaving ~40_000 remaining; branch <B> uses ~60_000 opcodes and trips remaining budget.
    let mut src = String::from("<A> a = X + 1\n");
    for _ in 0..7_500 {
        src.push_str("<A> a = a + p - q + 1\n");
    }
    src.push_str("<A> Y = a\n<B> b = X + 1\n");
    for _ in 0..7_500 {
        src.push_str("<B> b = b + p - q + 1\n");
    }
    src.push_str("<B> Y = b");

    let err = CompiledDslModel::compile("multi_branch_budget", &src).expect_err("cumulative opcode budget must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("remaining bytecode budget") && msg.contains("of 100000 total opcodes") && !msg.contains("exceeds total bytecode budget"),
        "expected remaining cumulative budget message, got: {msg}"
    );
}
