//! Remediation regression and D1 oracle tests for AUD-03 DSL, Special Functions, and ODE findings.
//!
//! Covers:
//! - `AUD-03-01`: Branch-expansion OOM/allocation amplification & filter-class budget
//! - `AUD-03-02`: Special-function CPU DoS metering into VM steps & ODE budget + Debye overflow screen
//! - `AUD-03-17`: `hyp1f1` / `hyp2f1` premature `k = 1` termination & small-total cancellation
//! - `AUD-03-18`: `bessel_kn` exponential scaling avoiding underflow at `x >= 746` & subnormal loss on `(710, 746)`
//! - `AUD-03-19`: `confluent_u_laplace_pos_a` geometric left-tail expansion & `[0, delta]` analytic tail
//! - `AUD-03-30`: ODE `y_ref` fallback when `y0 == 0 && f0 == 0` and running scale updates
//! - `AUD-03-31`: Empirical slope anchors on replicate tables via collapsed `(x, mean_y)` knots
//! - `AUD-03-32` / `AUD-03-15`: `ParametricModel::initial_guess_with_column_titles` & `uses_sheet_dataset_index`
//! - `AUD-03-43`: `[InitialValues]` case-insensitive param slots, subset filter scoping, and rule precedence
//! - `AUD-03-44`: `is_defined(expr)` compound expression fault-catching with fatal step-budget exhaustion
//! - `AUD-03-45`: `[Transforms]` reserved-ident guard, reversed `<C:A>` error, `besseli0`/`i1` parity, `lngamma` reflection
//! - `AUD-03-47`: Variadic `min`/`max` (`argc >= 2`) and UTF-8 scalar lexer error reporting
//!
//! D1 Provenance:
//! - Independent 80-digit decimal reference values computed from NIST DLMF §§5.5.3, 10.25.2, 10.32.9,
//!   13.2.2, 13.2.42, 15.2.1 and Abramowitz & Stegun (1964) §§6.1.17, 9.8.5–9.8.8.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::excessive_precision)]

mod common;

use std::time::Instant;

use curvebook_fit::{CompiledDslModel, DataPoint, FitError, HookContext, ParametricModel, fit_nonlinear};
use curvebook_testkit::assert_approx_eq;

use common::{assert_dsl_compile_err, cfg_fast_rules, dsl_compile, rule_shared_init, series_xy};

fn eval_dsl(expr: &str, x: &[f64]) -> Option<f64> {
    dsl_compile(expr).eval(x, &[], 0)
}

#[test]
fn rem_aud_03_01_dsl_branch_oom_and_filter_amplification_rejected() {
    // PoC 1: 24 unscoped 4,095-node locals + 4,096 singleton targets (~148 KB) would previously
    // allocate ~25.7 GB of Box<Expr> before MAX_TOTAL_OPCODES was checked.
    let big_rhs = std::iter::repeat_n("1", 2_048).collect::<Vec<_>>().join("+");
    let mut poc1 = String::with_capacity(160_000);
    for i in 0..24 {
        poc1.push_str(&format!("v{i}={big_rhs}\n"));
    }
    for d in 0..4_096 {
        poc1.push_str(&format!("<{d}>Y=v0\n"));
    }
    let t0 = Instant::now();
    assert_dsl_compile_err(&poc1);
    assert!(t0.elapsed().as_secs_f64() < 2.0, "PoC 1 should fail fast without OOM");

    // PoC 2: 4,096 singleton targets + many distinct Exclude filters would blow up matching_classes.
    let mut poc2 = String::with_capacity(120_000);
    for d in 0..4_096 {
        poc2.push_str(&format!("<{d}>Y=X\n"));
    }
    for i in 0..200 {
        let j = (i + 1) % 4_096;
        poc2.push_str(&format!("<~{i},{j}>Y=X\n"));
    }
    let t1 = Instant::now();
    assert_dsl_compile_err(&poc2);
    assert!(t1.elapsed().as_secs_f64() < 2.0, "PoC 2 should fail fast on filter-class budget");

    // PoC 3: Large unscoped targets across many classes must also charge AST nodes * matched classes.
    let mut poc3 = String::with_capacity(64_000);
    for d in 0..100 {
        poc3.push_str(&format!("<{d}>k=1\n"));
    }
    poc3.push_str(&format!("Y={big_rhs}\n"));
    assert_dsl_compile_err(&poc3);

    // PoC 4: Many `<~A>` 4,095-node locals + thousands of `Y = s` candidates where `s` is `<A>`-scoped
    // must evaluate `select_outside_branch` in linear O(total_nodes) time without re-walking locals.
    let mut poc4 = String::with_capacity(900_000);
    poc4.push_str("<A> s = 1\n<A> Y = s\n");
    for i in 0..200 {
        poc4.push_str(&format!("<~A> v{i} = {big_rhs}\n"));
    }
    for _ in 0..4_000 {
        poc4.push_str("Y = s\n");
    }
    let t2 = Instant::now();
    let compiled_poc4 = dsl_compile(&poc4);
    assert!(t2.elapsed().as_secs_f64() < 2.0, "PoC 4 select_outside_branch must stay linear");
    assert_approx_eq(compiled_poc4.eval(&[1.0], &[], 0).unwrap(), 1.0, 1e-14, 1e-14, "PoC 4 <A> branch");

    // Verify outside-branch local binding order is preserved: `<~A>` local binding `s` allows
    // `Y = s` placed after it while earlier `Y = s` is still skipped.
    let shadowed = dsl_compile("<A> s = 10\n<A> Y = s\nY = s + 100\n<~A> s = 20\nY = s + 5\n");
    assert_approx_eq(shadowed.eval(&[1.0], &[], 0).unwrap(), 10.0, 1e-14, 1e-14, "shadowed <A>");
    assert_approx_eq(shadowed.eval(&[1.0], &[], 1).unwrap(), 25.0, 1e-14, 1e-14, "shadowed <~A>");

    // Valid multi-dataset branch model with locals, targets, and implicit/ODE forms still works.
    let valid = dsl_compile("<A> m = 2\n<B> m = 5\nbase = m * X\nY = base + b");
    assert_approx_eq(valid.eval(&[3.0], &[1.0], 0).unwrap(), 7.0, 1e-14, 1e-14, "valid branch A");
    assert_approx_eq(valid.eval(&[3.0], &[1.0], 1).unwrap(), 16.0, 1e-14, 1e-14, "valid branch B");
}

#[test]
fn rem_aud_03_02_unmetered_special_fn_cpu_dos_bounded() {
    // Debye overflow bound rejects besseli(0, 500000) in O(1) instead of ~504k Miller iterations.
    let m_overflow = dsl_compile("Y = besseli(0, 500000)");
    let t0 = Instant::now();
    assert_eq!(m_overflow.eval(&[0.0], &[], 0), None);
    assert!(t0.elapsed().as_secs_f64() < 0.05, "besseli overflow should reject in O(1)");

    // Chained finite-returning besseli calls must trip MAX_VM_STEPS quickly.
    let mut chain_in = String::from("s = 0\n");
    for _ in 0..200 {
        chain_in.push_str("s = s + besseli(1000, 1000)\n");
    }
    chain_in.push_str("Y = s");
    let m_chain_in = dsl_compile(&chain_in);
    let t1 = Instant::now();
    assert_eq!(m_chain_in.eval(&[0.0], &[], 0), None);
    assert!(t1.elapsed().as_secs_f64() < 0.5, "chained besseli should hit MAX_VM_STEPS rapidly");

    // Chained besseljn calls must also be metered into VM steps.
    let mut chain_jn = String::from("s = 0\n");
    for _ in 0..40 {
        chain_jn.push_str("s = s + besseljn(5000, 5000)\n");
    }
    chain_jn.push_str("Y = s");
    let m_chain_jn = dsl_compile(&chain_jn);
    let t2 = Instant::now();
    assert_eq!(m_chain_jn.eval(&[0.0], &[], 0), None);
    assert!(t2.elapsed().as_secs_f64() < 0.5, "chained besseljn should hit MAX_VM_STEPS rapidly");

    // Debye underflow bound resolves besselk(2000, 2000) = 0 in O(1), while chained finite-returning
    // besselk(1500, 1000) (~4e-7, 1,499 recurrence iterations each) must trip MAX_VM_STEPS rapidly.
    assert_eq!(eval_dsl("Y = besselk(2000, 2000)", &[0.0]), Some(0.0));
    let mut chain_kn = String::from("s = 0\n");
    for _ in 0..80 {
        chain_kn.push_str("s = s + besselk(1500, 1000)\n");
    }
    chain_kn.push_str("Y = s");
    let m_chain_kn = dsl_compile(&chain_kn);
    let t3 = Instant::now();
    assert_eq!(m_chain_kn.eval(&[0.0], &[], 0), None);
    assert!(t3.elapsed().as_secs_f64() < 0.5, "chained besselk should hit MAX_VM_STEPS rapidly");

    // Heavy special function inside an ODE RHS must respect OdeEvalBudget.
    let ode_heavy = dsl_compile("Y0 = A\ndY/dt = -k * Y + 1e-6 * besseljn(5000, 5000 + t)");
    let xs: Vec<f64> = (1..=20).map(f64::from).collect();
    let ys = vec![0.5; xs.len()];
    let cfg = cfg_fast_rules(vec![rule_shared_init("k", 500.0), rule_shared_init("A", 1.0)]);
    let t4 = Instant::now();
    let _ = fit_nonlinear(&ode_heavy, &[series_xy(0, "OdeHeavy", &xs, &ys)], &HookContext::default(), &cfg);
    assert!(t4.elapsed().as_secs_f64() < 2.0, "ODE with heavy special fn must remain bounded");
}

#[test]
fn rem_aud_03_17_hyp1f1_and_hyp2f1_series_growth_peak_and_cancellation() {
    // D1 80-digit oracles (NIST DLMF §13.2.2, §15.2.1):
    let cases = [
        ("Y = hyp1f1(1e-16, 600, 1000)", 1.042_822_244_121_725_4e24, 1e-11),
        ("Y = hyp1f1(1e-15, 200, 350)", 1.536_490_432_597_118_0e1, 1e-11),
        ("Y = hyp2f1(1e-16, 500, 180, 0.6)", 9.132_113_406_555_672_4e8, 1e-11),
        ("Y = hyp2f1(1e-15, 300, 120, 0.7)", 4.362_635_361_613_934_7e9, 1e-11),
        // Increasing |factor(k)| approaching future_bound from below must still converge:
        ("Y = hyp1f1(0.5, 1e8, 8.5e7)", 2.581_988_586_557_395_1, 1e-12),
        ("Y = hyp2f1(1, 1, 10, 0.5)", 1.055_220_678_650_444_1, 1e-12),
    ];
    for (expr, expected, rel_tol) in cases {
        let actual = eval_dsl(expr, &[0.0]).unwrap_or_else(|| panic!("expected Some for `{expr}`"));
        assert_approx_eq(actual, expected, 0.0, rel_tol, expr);
    }

    // Subnormal term underflow before growth peak (`a + k != 0`, `future_bound >= 1`) must return None
    // instead of falsely terminating at 1.0, while genuinely convergent underflow returns Some(1.0).
    assert_eq!(eval_dsl("Y = hyp1f1(1e-321, 1000, 3000)", &[0.0]), None);
    assert_eq!(eval_dsl("Y = hyp2f1(1e-170, 1e-170, 1e-300, 0.5)", &[0.0]), None);
    assert_approx_eq(eval_dsl("Y = hyp1f1(1e-322, 10, 1)", &[0.0]).unwrap(), 1.0, 1e-15, 1e-15, "convergent underflow 1F1");
    assert_approx_eq(eval_dsl("Y = hyp2f1(1e-322, 1, 10, 0.5)", &[0.0]).unwrap(), 1.0, 1e-15, 1e-15, "convergent underflow 2F1");

    // Exact low-degree terminating polynomial roots must still return Some(0.0).
    assert_eq!(eval_dsl("Y = hyp1f1(-1, 2, 2)", &[0.0]), Some(0.0));
    assert_eq!(eval_dsl("Y = hyp2f1(-1, 2, 1, 0.5)", &[0.0]), Some(0.0));

    // Small-total catastrophic cancellation in terminating polynomials (`term == 0.0`) must be rejected
    // rather than masked by `.max(1.0)`:
    // - hyp1f1(-2, 1, 2 - sqrt(2)) = L_2(2 - sqrt(2)) = 0 (exact root; max_abs ~ 1.17, sum ~ 1e-16).
    // - hyp2f1(-20, 1, 1, 0.99) = (1 - 0.99)^20 = 1e-40 (max_abs ~ 1.67e5, sum ~ 1e-11).
    assert_eq!(eval_dsl("Y = hyp1f1(-2, 1, 2 - sqrt(2))", &[0.0]), None);
    assert_eq!(eval_dsl("Y = hyp2f1(-20, 1, 1, 0.99)", &[0.0]), None);
}

#[test]
fn rem_aud_03_18_bessel_kn_exponential_scaling_and_large_x() {
    // D1 80-digit oracles via K_n(x) = int_0^inf exp(-x cosh t) cosh(n t) dt (DLMF §10.32.9).
    // A&S 9.8.6/9.8.8 base seeds have ~1e-7 relative error (and ~2e-9 for x >= 700).
    let cases = [
        ("Y = besselk(1000, 750)", 6.349_801_573_783_100e-68, 1e-6),
        ("Y = besselk(1150, 750)", 1.375_317_858_038_887e8, 1e-6),
        ("Y = besselk(1500, 800)", 1.871_598_682_931_527e163, 1e-6),
        ("Y = besselk(100, 730)", 3.974_942_982_630_591e-316, 1e-5),
    ];
    for (expr, expected, rel_tol) in cases {
        let actual = eval_dsl(expr, &[0.0]).unwrap_or_else(|| panic!("expected Some for `{expr}`"));
        assert_approx_eq(actual, expected, 0.0, rel_tol, expr);
    }
    // Overflowing K_2000(800) must return None, never Some(0.0).
    assert_eq!(eval_dsl("Y = besselk(2000, 800)", &[0.0]), None);
}

#[test]
fn rem_aud_03_19_confluent_u_laplace_left_tail_oracles() {
    // D1 80-digit oracles via DLMF §13.2.42 connection formula; all hit confluent_u_laplace_pos_a.
    let cases = [
        ("Y = confluent_u(50, 1.02, 0.01)", 8.686_752_514_972_594e-64, 1e-10),
        ("Y = confluent_u(50, 1.02, 0.001)", 3.748_827_276_028_872e-63, 1e-10),
        ("Y = confluent_u(50, 1.02, 0.0001)", 7.847_919_236_215_557e-63, 1e-10),
        ("Y = confluent_u(100, 1.02, 0.0001)", 4.320_475_514_115_335e-156, 1e-10),
        ("Y = confluent_u(50, 0.98, 0.0001)", 6.037_002_932_619_734e-63, 1e-10),
        // Near-integer b < 1 with tiny z requires geometric right-tail s_max expansion:
        ("Y = confluent_u(0.5, 0.99, 1e-30)", 2.875_476_324_695_537_6e1, 1e-10),
        ("Y = confluent_u(2.0, 0.99, 1e-40)", 5.896_541_621_458_294_1e1, 1e-10),
        ("Y = confluent_u(1.0, 0.99, 1e-60)", 7.473_363_822_496_281_4e1, 1e-10),
    ];
    for (expr, expected, rel_tol) in cases {
        let actual = eval_dsl(expr, &[0.0]).unwrap_or_else(|| panic!("expected Some for `{expr}`"));
        assert_approx_eq(actual, expected, 0.0, rel_tol, expr);
    }
}

#[test]
fn rem_aud_03_30_ode_zero_initial_value_and_zero_initial_slope() {
    // Smooth analytic ODE Y(0) = 0, dY/dt = t^5 => exact Y(t) = t^6 / 6.
    let poly_ode = dsl_compile("Y(0) = 0\ndY/dt = t^5");
    for &t in &[0.5_f64, 1.0, 2.0, 3.0] {
        let got = poly_ode.eval(&[t], &[], 0).unwrap_or_else(|| panic!("expected Some at t={t}"));
        let exact = t.powi(6) / 6.0;
        assert_approx_eq(got, exact, 1e-9, 1e-6, &format!("t^5 ODE at t={t}"));
    }

    // Delayed-start pharmacokinetic ODE Y(0) = 0, dY/dt = IF(t < 1, 0, sin(t - 1)).
    // Exact solution: 0 for t <= 1, and 1 - cos(t - 1) for t > 1.
    let delayed_ode = dsl_compile("Y(0) = 0\ndY/dt = IF(t < 1, 0, sin(t - 1))");
    for &t in &[0.5_f64, 1.0, 1.5, 2.0, 3.5] {
        let got = delayed_ode.eval(&[t], &[], 0).unwrap_or_else(|| panic!("expected Some at t={t}"));
        let exact = if t <= 1.0 { 0.0 } else { 1.0 - (t - 1.0).cos() };
        assert_approx_eq(got, exact, 1e-6, 1e-5, &format!("delayed ODE at t={t}"));
    }
}

#[test]
fn rem_aud_03_31_empirical_slopes_on_replicate_tables() {
    let model = dsl_compile(
        "[Equation]\nY = SMin * X + SMid + SMax\n[InitialValues]\n\
         SMin = SlopeAtXmin\nSMid = SlopeAtXmid\nSMax = SlopeAtXmax\n",
    );

    // 8 concentrations x 3 replicates (N = 24 < 30): y = x^2 with symmetric replicate offsets (-1, 0, +1).
    let xs8 = [0.0, 1.0, 2.0, 3.0, 5.0, 7.0, 9.0, 10.0];
    let mut pts24 = Vec::new();
    for &x in &xs8 {
        for &delta in &[-1.0, 0.0, 1.0] {
            pts24.push(DataPoint::xy(x, x * x + delta));
        }
    }
    let init24 = model.initial_guess(&pts24);
    // Collapsed knots (n = 8 even): s_min = (1-0)/(1-0) = 1.0, s_mid = (25-9)/(5-3) = 8.0, s_max = (100-81)/(10-9) = 19.0.
    assert_approx_eq(init24[0], 1.0, 1e-12, 1e-12, "rep24 SlopeAtXmin");
    assert_approx_eq(init24[1], 8.0, 1e-12, 1e-12, "rep24 SlopeAtXmid");
    assert_approx_eq(init24[2], 19.0, 1e-12, 1e-12, "rep24 SlopeAtXmax");

    // 10 concentrations x 4 replicates (N = 40 >= 30): linear y = 3*x + 7 => all slopes exact 3.0 (not fallback 1.0).
    let mut pts40 = Vec::new();
    for i in 0..10 {
        let x = f64::from(i);
        for &delta in &[-0.6, -0.2, 0.2, 0.6] {
            pts40.push(DataPoint::xy(x, 3.0 * x + 7.0 + delta));
        }
    }
    let init40 = model.initial_guess(&pts40);
    for (idx, lbl) in ["SlopeAtXmin", "SlopeAtXmid", "SlopeAtXmax"].into_iter().enumerate() {
        assert_approx_eq(init40[idx], 3.0, 1e-12, 1e-12, &format!("rep40 {lbl}"));
    }
}

#[test]
fn rem_aud_03_32_and_15_dsl_trait_column_titles_and_sheet_index() {
    let model = dsl_compile(
        "[Equation]\nY = Kd * X + LogKd\n[InitialValues]\n\
         Kd = MeanColumnTitle\nLogKd = LogMeanColumnTitle\n",
    );
    let dyn_model: &dyn ParametricModel = &model;
    assert!(dyn_model.uses_sheet_dataset_index(), "CompiledDslModel must use sheet dataset index");

    let pts = vec![DataPoint::xy(1.0, 2.0), DataPoint::xy(2.0, 4.0)];
    let via_trait = dyn_model.initial_guess_with_column_titles(&pts, &[10.0, 100.0, 190.0]);
    assert_approx_eq(via_trait[0], 100.0, 1e-13, 1e-13, "trait MeanColumnTitle");
    assert_approx_eq(via_trait[1], 2.0, 1e-13, 1e-13, "trait LogMeanColumnTitle");

    // When caller passes empty column_titles slice, falls back to attached column_title_values.
    let attached = model.with_column_title_values(vec![50.0, 150.0]);
    let dyn_attached: &dyn ParametricModel = &attached;
    let fallback = dyn_attached.initial_guess_with_column_titles(&pts, &[]);
    assert_approx_eq(fallback[0], 100.0, 1e-13, 1e-13, "attached fallback MeanColumnTitle");
    assert_approx_eq(fallback[1], 2.0, 1e-13, 1e-13, "attached fallback LogMeanColumnTitle");
}

#[test]
fn rem_aud_03_43_initial_values_case_scoping_and_precedence() {
    let pts_a = vec![DataPoint { data_set_index: 0, ..DataPoint::xy(0.0, 10.0) }, DataPoint { data_set_index: 0, ..DataPoint::xy(5.0, 50.0) }];
    let pts_b = vec![DataPoint { data_set_index: 1, ..DataPoint::xy(0.0, 10.0) }, DataPoint { data_set_index: 1, ..DataPoint::xy(5.0, 50.0) }];

    // (a) Case-insensitive parameter slot registration: raw `bottom` registers canonical `Bottom`.
    let m_case = dsl_compile("[Equation]\nY = Bottom + (Top - Bottom) * X\n[InitialValues]\nbottom = YMIN\nTop = Bottom + SPAN");
    let g_case = m_case.initial_guess(&pts_a);
    assert_approx_eq(g_case[0], 10.0, 1e-13, 1e-13, "case Bottom");
    assert_approx_eq(g_case[1], 50.0, 1e-13, 1e-13, "case Top = Bottom + SPAN");

    // (b) Subset scoping: `<A:C>` local is readable by narrower `<A>`, while disjoint `<B>` is rejected.
    let m_subset = dsl_compile("[Equation]\nY = K * X\n[InitialValues]\n<A:C> base = YMAX\n<A> K = base * 2");
    assert_approx_eq(m_subset.initial_guess(&pts_a)[0], 100.0, 1e-13, 1e-13, "subset <A> reads <A:C> base");
    assert_dsl_compile_err("[Equation]\nY = K * X\n[InitialValues]\n<A> base = YMAX\n<B> K = base * 2");

    // Scoped shadowing of an unscoped local is visible to later unscoped rules on the shadowed dataset.
    let m_shadow = dsl_compile("[Equation]\nY = K * X\n[InitialValues]\nbase = 10\n<A> base = 50\nK = base * 2");
    assert_approx_eq(m_shadow.initial_guess(&pts_a)[0], 100.0, 1e-13, 1e-13, "shadowed base on <A>");
    assert_approx_eq(m_shadow.initial_guess(&pts_b)[0], 20.0, 1e-13, 1e-13, "unshadowed base on <B>");

    // (c) Specific dataset rule overrides later unscoped `Any` rule on that dataset.
    let m_prec = dsl_compile("[Equation]\nY = A * X\n[InitialValues]\n<A> A = 10\nA = 1");
    assert_approx_eq(m_prec.initial_guess(&pts_a)[0], 10.0, 1e-13, 1e-13, "<A> A = 10 beats later A = 1");
    assert_approx_eq(m_prec.initial_guess(&pts_b)[0], 1.0, 1e-13, 1e-13, "ds B falls back to A = 1");
}

#[test]
fn rem_aud_03_44_is_defined_compound_expressions_and_budget_fatal() {
    let m_add = dsl_compile("Y = IF(is_defined(X2 + 1), 2 * X1 + X2, 2 * X1)");
    assert_approx_eq(m_add.eval(&[4.0, f64::NAN], &[], 0).unwrap(), 8.0, 1e-14, 1e-14, "is_defined(X2 + 1) NaN");
    assert_approx_eq(m_add.eval(&[4.0, 3.0], &[], 0).unwrap(), 11.0, 1e-14, 1e-14, "is_defined(X2 + 1) finite");
    assert_approx_eq(m_add.eval(&[4.0], &[], 0).unwrap(), 8.0, 1e-14, 1e-14, "is_defined(X2 + 1) missing X2");

    let m_div = dsl_compile("Y = IF(is_defined(1 / X), 1 / X, 0)");
    assert_approx_eq(m_div.eval(&[0.0], &[], 0).unwrap(), 0.0, 1e-14, 1e-14, "is_defined(1 / 0)");
    assert_approx_eq(m_div.eval(&[2.0], &[], 0).unwrap(), 0.5, 1e-14, 1e-14, "is_defined(1 / 2)");

    let m_sqrt = dsl_compile("Y = IF(is_defined(sqrt(X)), sqrt(X), 0)");
    assert_approx_eq(m_sqrt.eval(&[-1.0], &[], 0).unwrap(), 0.0, 1e-14, 1e-14, "is_defined(sqrt(-1))");
    assert_approx_eq(m_sqrt.eval(&[9.0], &[], 0).unwrap(), 3.0, 1e-14, 1e-14, "is_defined(sqrt(9))");

    // Nested is_defined works cleanly.
    let m_nested = dsl_compile("Y = is_defined(is_defined(1 / X) + 1)");
    assert_approx_eq(m_nested.eval(&[0.0], &[], 0).unwrap(), 1.0, 1e-14, 1e-14, "nested is_defined");

    // Normal expressions outside is_defined remain strict.
    assert_eq!(eval_dsl("Y = 1 / X", &[0.0]), None);
    assert_eq!(eval_dsl("Y = sqrt(X)", &[-1.0]), None);
    assert_eq!(eval_dsl("Y = X2 + 1", &[4.0, f64::NAN]), None);

    // VM step-budget exhaustion inside is_defined must stay fatal (returns None, not 0.0).
    let m_budget = dsl_compile("Y = is_defined(besseli(100000, 66000))");
    assert_eq!(m_budget.eval(&[0.0], &[], 0), None);
}

#[test]
fn rem_aud_03_45_transforms_reserved_reversed_range_bessel_parity_and_lngamma() {
    // (a) [Transforms] rejects reserved built-in constants/variables on LHS.
    assert_dsl_compile_err("[Equation]\nY = k * X\n[Transforms]\nPI = 2 * k");
    assert_dsl_compile_err("[Equation]\nY = k * X\n[Transforms]\nE = k + 1");

    // (b) Reversed dataset range `<C:A>` returns DslCompileError instead of silent empty Include([]).
    assert_dsl_compile_err("<C:A> Y = a * X");
    assert_dsl_compile_err("<5:2> Y = a * X");

    // (c) besseli0 / besseli1 domain & accuracy match besseli(0, x) / besseli(1, x) up to x = 705.
    let i0_705 = eval_dsl("Y = besseli0(705)", &[0.0]).expect("besseli0(705) should succeed");
    let in0_705 = eval_dsl("Y = besseli(0, 705)", &[0.0]).expect("besseli(0, 705) should succeed");
    assert_approx_eq(i0_705, 2.262_050_552_655_472_5e304, 0.0, 1e-11, "besseli0(705)");
    assert_eq!(i0_705.to_bits(), in0_705.to_bits(), "besseli0 and besseli(0, x) must match");

    let i1_705 = eval_dsl("Y = besseli1(705)", &[0.0]).expect("besseli1(705) should succeed");
    assert_approx_eq(i1_705, 2.260_445_691_777_057_0e304, 0.0, 1e-11, "besseli1(705)");

    // (d) lngamma supports negative non-integers via Euler reflection and rejects non-positive integers.
    let lg_cases = [
        ("Y = lngamma(-0.5)", 1.265_512_123_484_645_4, 1e-12),
        ("Y = lngamma(-3.5)", -1.309_006_684_993_042_0, 1e-12),
        ("Y = lngamma(-100.25)", -3.634_009_232_278_215_4e2, 1e-11),
    ];
    for (expr, expected, rel_tol) in lg_cases {
        let actual = eval_dsl(expr, &[0.0]).unwrap_or_else(|| panic!("expected Some for `{expr}`"));
        assert_approx_eq(actual, expected, 1e-12, rel_tol, expr);
    }
    for pole in ["Y = lngamma(0)", "Y = lngamma(-1)", "Y = lngamma(-5)"] {
        assert_eq!(eval_dsl(pole, &[0.0]), None, "pole `{pole}` must return None");
    }
}

#[test]
fn rem_aud_03_47_variadic_min_max_and_utf8_lexer_error() {
    assert_approx_eq(eval_dsl("Y = min(3, 1, 2)", &[0.0]).unwrap(), 1.0, 1e-15, 1e-15, "min 3 args");
    assert_approx_eq(eval_dsl("Y = max(3, 1, 5, 2)", &[0.0]).unwrap(), 5.0, 1e-15, 1e-15, "max 4 args");
    assert_approx_eq(eval_dsl("Y = min(-2, -10, 4, -5)", &[0.0]).unwrap(), -10.0, 1e-15, 1e-15, "min 4 args");
    assert_dsl_compile_err("Y = min(1)");
    assert_dsl_compile_err("Y = max(1)");

    let err_minus = CompiledDslModel::compile("utf8_minus", "Y = a * X \u{2212} b").unwrap_err();
    match err_minus {
        FitError::DslCompileError { message, .. } => {
            assert!(message.contains('\u{2212}'), "expected Unicode minus in diagnostic, got: {message}");
            assert!(!message.contains('\u{00E2}'), "must not garble UTF-8 lead byte as `â`: {message}");
        }
        other => panic!("expected DslCompileError, got {other:?}"),
    }

    let err_alpha = CompiledDslModel::compile("utf8_alpha", "Y = \u{03B1} * X").unwrap_err();
    match err_alpha {
        FitError::DslCompileError { message, .. } => {
            assert!(message.contains('\u{03B1}'), "expected Greek alpha in diagnostic, got: {message}");
        }
        other => panic!("expected DslCompileError, got {other:?}"),
    }
}
