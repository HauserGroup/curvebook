//! Property-based and adversarial fuzz suite for the equation DSL parser/compiler,
//! stack VM evaluator, implicit root solver, RK45 ODE integrator, and nonlinear fitter.
#![forbid(unsafe_code)]

use curvebook_doc::{FitMethod, WeightingScheme};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, DslEquationKind, HookContext, ParametricModel, SolverConfig, fit_nonlinear, integrate_rk45,
    replicates_lack_of_fit,
};
use proptest::prelude::{BoxedStrategy, Just, ProptestConfig, Strategy, any, prop_assert, prop_assert_eq, prop_oneof, proptest};
use proptest::sample::select;
use std::f64::consts::{FRAC_PI_2, PI};
use std::time::{Duration, Instant};

use crate::common::{cfg_fast, dsl_compile, rule_const, rule_local_bounds, rule_shared, series_from_pts, series_xy};

const ADVERSARIAL_F64S: [f64; 18] = [
    0.0,
    -0.0,
    1.0,
    -1.0,
    0.5,
    FRAC_PI_2,
    PI,
    1e-12,
    -1e-12,
    1e-300,
    -1e-300,
    1e12,
    -1e12,
    1e300,
    -1e300,
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
];

fn assert_eval_bounded_and_clean(model: &CompiledDslModel, x: &[f64], params: &[f64], ds_idx: usize, budget: Duration, ctx: &str) {
    let t0 = Instant::now();
    let val = model.eval(x, params, ds_idx);
    let elapsed = t0.elapsed();
    assert!(elapsed <= budget, "eval exceeded time budget ({elapsed:?} > {budget:?}) for `{ctx}`");
    if let Some(v) = val {
        assert!(v.is_finite(), "eval returned non-finite Some({v}) for `{ctx}` with x={x:?}, params={params:?}");
    }
}

fn compile_and_exercise(source: &str, budget: Duration) -> Result<CompiledDslModel, String> {
    let t0 = Instant::now();
    let compiled = CompiledDslModel::compile("fuzz", source);
    let elapsed = t0.elapsed();
    assert!(elapsed <= budget, "compile exceeded budget ({elapsed:?} > {budget:?}) on len={}", source.len());
    let Ok(model) = compiled else {
        return Err("clean compile error".to_owned());
    };

    let p_count = model.param_count();
    let sample_pts = vec![DataPoint::xy(0.0, 1.0), DataPoint::xy(1.0, 2.0), DataPoint::xy(-1.0, -0.5)];
    let _guess = model.initial_guess(&sample_pts);
    let _bounds = model.default_bounds();

    for xv in [0.0, -0.0, 1.0, -1.0, 1e-10, 1e12, f64::NAN, f64::INFINITY] {
        let mut params = vec![1.0_f64; p_count];
        assert_eval_bounded_and_clean(&model, &[xv], &params, 0, budget, source);
        if p_count > 0 {
            params[0] = xv;
            assert_eval_bounded_and_clean(&model, &[xv, 0.5], &params, 1, budget, source);
        }
    }
    // Also probe mismatched predictor/parameter slice lengths cleanly returning None or finite.
    assert_eval_bounded_and_clean(&model, &[], &[], 0, budget, source);
    assert_eval_bounded_and_clean(&model, &[1.0, 2.0, 3.0], &[1.0; 16], 2, budget, source);
    Ok(model)
}

fn check_fit_clean(model: &dyn ParametricModel, datasets: &[DatasetSeries], cfg: &SolverConfig, budget: Duration, ctx: &str) {
    let t0 = Instant::now();
    let res = fit_nonlinear(model, datasets, &HookContext::default(), cfg);
    let elapsed = t0.elapsed();
    assert!(elapsed <= budget, "fit_nonlinear exceeded budget ({elapsed:?} > {budget:?}) for `{ctx}`");
    if let Ok(fit) = res {
        assert!(!fit.weighted_ss.is_nan(), "weighted_ss is NaN for `{ctx}`");
        assert!(!fit.rss.is_nan(), "rss is NaN for `{ctx}`");
        for p in &fit.parameters {
            assert!(p.estimate.is_finite(), "non-finite fitted parameter `{}` = {} for `{ctx}`", p.name, p.estimate);
        }
        for ds in &fit.datasets {
            assert!(ds.params.iter().all(|v| v.is_finite()), "non-finite dataset param for `{ctx}`");
            assert!(ds.y_hat.iter().all(|v| v.is_finite()), "non-finite y_hat for `{ctx}`");
        }
    }
}

// ============================================================================
// Targeted Deterministic Adversarial Edge Cases
// ============================================================================

#[test]
fn empty_whitespace_comment_and_huge_equations_return_cleanly() {
    for case in [
        "",
        "   \n\t\r\n  ",
        "# pure comment\n// slash comment\n; semicolon comment",
        "# initial: a=1, b=NAN\n# t0=INF\n",
        "<A>   \n<B> # just comment",
    ] {
        assert!(CompiledDslModel::compile("empty", case).is_err(), "expected empty equation Err for `{case:?}`");
    }

    let huge_sum = format!("Y = {}", std::iter::repeat_n("1", 5_000).collect::<Vec<_>>().join(" + "));
    let huge_ident = format!("Y = {}", "a".repeat(200_000));
    let huge_number = format!("Y = {} + X", "9".repeat(100_000));
    let mut many_locals = String::from("v0 = X\n");
    for i in 1..2_000 {
        many_locals.push_str(&format!("v{i} = v{} + 1\n", i - 1));
    }
    many_locals.push_str("Y = v1999");

    for src in [&huge_sum, &huge_ident, &huge_number, &many_locals] {
        let _ = compile_and_exercise(src, Duration::from_secs(2));
    }
}

#[test]
fn deeply_nested_parens_and_long_chains_do_not_overflow_stack() {
    let budget = Duration::from_secs(2);
    for depth in [16, 64, 128, 512, 2_000, 10_000] {
        let d_if = depth.min(2_500);
        let cases = [
            format!("Y = {}X{}", "(".repeat(depth), ")".repeat(depth)),
            format!("Y = {}X", "-".repeat(depth)),
            format!("Y = {}X", "!".repeat(depth)),
            format!("Y = {}X", "2 ^ ".repeat(depth)),
            format!("Y = {}X{}", "IF(X > 0, ".repeat(d_if), ", 0)".repeat(d_if)),
        ];
        for src in &cases {
            let _ = compile_and_exercise(src, budget);
        }
    }
}

#[test]
fn huge_dataset_prefix_ranges_and_malformed_prefixes_do_not_oom_or_hang() {
    let budget = Duration::from_millis(500);
    let prefixes = [
        "<0:10000000000> Y = a * X",
        "<1:999999999999> Y = a * X",
        "<0:18446744073709551615> Y = a * X",
        "<~0:1000000000> Y = a * X",
        "<A:ZZZZZZ> Y = a * X",
        "<A:B,0:999999999> Y = a * X",
        "<> Y = X",
        "<-> Y = X",
        "<:> Y = X",
        "<A:B:C> Y = X",
        "<unclosed Y = X",
        "<A> ",
        "<999999999999999999999999999> Y = X",
    ];
    for src in prefixes {
        let t0 = Instant::now();
        let res = CompiledDslModel::compile("prefix", src);
        assert!(t0.elapsed() <= budget, "prefix `{src}` exceeded {budget:?}");
        if let Ok(model) = res {
            assert_eval_bounded_and_clean(&model, &[1.0], &[1.0; 4], 0, budget, src);
        }
    }
}

#[test]
fn vm_domain_errors_and_overflow_semantics() {
    let budget = Duration::from_millis(250);
    assert_eq!(dsl_compile("Y = 0 ^ 0").eval(&[0.0], &[], 0), Some(1.0));

    for none_src in [
        "Y = 1 / 0",
        "Y = 0 / 0",
        "Y = 1 % 0",
        "Y = ln(-1)",
        "Y = ln(0)",
        "Y = log10(-5)",
        "Y = log2(0)",
        "Y = sqrt(-5)",
        "Y = exp(1e9)",
        "Y = exp10(400)",
        "Y = (-1) ^ 0.5",
        "Y = probit(0)",
        "Y = probit(1)",
        "Y = y0(-1)",
        "Y = k0(0)",
    ] {
        assert_eq!(dsl_compile(none_src).eval(&[1.0], &[], 0), None, "expected None for `{none_src}`");
    }

    assert_eval_bounded_and_clean(&dsl_compile("Y = tan(PI / 2)"), &[0.0], &[], 0, budget, "tan(PI/2)");

    let domain_exprs = [
        "Y = X / (X - X)",
        "Y = (-2) ^ (-3)",
        "Y = pow(10, 500)",
        "Y = acosh(X)",
        "Y = atanh(X)",
        "Y = asin(X)",
        "Y = acos(X)",
        "Y = gamma(X)",
        "Y = lngamma(X)",
        "Y = beta(X, -2)",
        "Y = digamma(X)",
        "Y = probit(X)",
        "Y = besselj0(X)",
        "Y = besselj1(X)",
        "Y = bessely0(X)",
        "Y = bessely1(X)",
        "Y = besseli0(X)",
        "Y = besseli1(X)",
        "Y = besselk0(X)",
        "Y = besselk1(X)",
        "Y = hyp1f1(1, X, 2)",
        "Y = hyp2f1(1, 2, X, 1.5)",
        "Y = confluent_u(1, 2, X)",
        "Y = hypot(X, 1e300)",
        "Y = atan2(X, X)",
    ];
    for src in domain_exprs {
        let m = dsl_compile(src);
        for xv in ADVERSARIAL_F64S {
            assert_eval_bounded_and_clean(&m, &[xv], &[], 0, budget, src);
        }
    }

    for src in [
        "Y = exp(1000) / exp(1000)",
        "Y = (1e300 * 1e300) - (1e300 * 1e300)",
        "Y = 1 / exp(1e9)",
        "Y = atan(exp(1e9))",
        "Y = min(exp(1e9), 1)",
        "Y = max(NAN, 1)",
        "Y = min(INF, 1)",
        "Y = IF(NAN > 0, 1, 2)",
        "Y = (0.0 ^ -1.0)",
        "Y = (-0.0) ^ (-1.0)",
    ] {
        assert_eval_bounded_and_clean(&dsl_compile(src), &[1.0], &[], 0, budget, src);
    }
}

#[test]
fn malformed_syntax_and_token_edge_cases() {
    let budget = Duration::from_millis(250);
    for src in [
        "Y = (((X",
        "Y = X)))",
        "Y = IF((1, 2)",
        "Y = IF(1)",
        "Y = IF(1, 2)",
        "Y = IF(1, 2, 3, 4)",
        "Y = clamp()",
        "Y = sin()",
        "Y = sin(1, 2)",
        "Y = 1e",
        "Y = 1e+",
        "Y = 1e+999999",
        "Y = 1e-999999",
        "Y = 0.",
        "Y = .5",
        "Y = 1..2",
        "Y = X + * 1",
        "Y = X \0 + 1",
        "Y = 1\ndY/dX = Y",
        "dY/dX = 1\nY = X",
        "Y(abc) = 1\ndY/dX = Y",
        "Y0 = Y + 1\ndY/dX = -Y",
        "dY/dX = -Y",
        "X = Y",
        "X1 = 5\nY = X1 + X2",
    ] {
        let _ = compile_and_exercise(src, budget);
    }
}

#[test]
fn clamp_nan_bounds_and_bessel_order_extremes_do_not_panic_or_hang() {
    let budget = Duration::from_millis(300);

    for src in ["Y = clamp(X, NAN, NAN)", "Y = clamp(X, INF - INF, 1)", "Y = clamp(X, 1, NAN)", "Y = clamp(NAN, 0, 1)", "Y = clamp(X, a, b)"] {
        let m = dsl_compile(src);
        let n_p = m.param_count();
        for xv in [0.0, 1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eval_bounded_and_clean(&m, &[xv], &vec![f64::NAN; n_p], 0, budget, src);
            assert_eval_bounded_and_clean(&m, &[xv], &vec![f64::INFINITY; n_p], 0, budget, src);
        }
    }

    for src in [
        "Y = jn(-1e12, X)",
        "Y = jn(-2147483648, X)",
        "Y = yn(-1e12, X)",
        "Y = yn(-2147483648, X)",
        "Y = jn(1e12, X)",
        "Y = yn(1e12, X)",
        "Y = besseljn(1000000, X)",
        "Y = besselyn(1000000, X)",
        "Y = besseljn(INF, X)",
        "Y = besselyn(NAN, X)",
        "Y = jn(n, X)",
        "Y = yn(n, X)",
    ] {
        let m = dsl_compile(src);
        let n_p = m.param_count();
        for order in [-1e15, -2_147_483_648.0, 0.0, 100_001.0, 1e12, f64::NAN, f64::INFINITY] {
            let params = vec![order; n_p];
            for xv in [1.5, 0.0, -2.5] {
                assert_eval_bounded_and_clean(&m, &[xv], &params, 0, budget, src);
            }
        }
    }
}

#[test]
fn unbound_shadowed_cyclic_and_multi_predictor_identifiers() {
    let budget = Duration::from_millis(250);
    for src in [
        "a = a + 1\nY = a * X",
        "a = b + 1\nb = a + 1\nY = a + b + X",
        "Y = x + X1 + x2 + X99999999999999999999 + pI + e + InF + NaN + dS + DaTaSeT + unbound_p",
        "<A> k = 1\n<B> Y = k * X",
        "k = X * p\nk = k + 1\nY = k",
        "# initial: p=NAN, q=INF\nY = p * X + q",
    ] {
        let _ = compile_and_exercise(src, budget);
    }

    let m = dsl_compile("<A> Y = a * X\n<B> Y = b * X1 + c * X2");
    assert!(m.param_count() >= 3);
    assert_eval_bounded_and_clean(&m, &[2.0, 3.0], &[1.0; 3], 0, budget, "ds0");
    assert_eval_bounded_and_clean(&m, &[2.0, 3.0], &[1.0; 3], 1, budget, "ds1");
    assert_eq!(m.eval(&[2.0, 3.0], &[1.0; 3], 2), None);
}

#[test]
fn rk45_small_span_non_finite_and_stiff_ode_edge_cases() {
    let budget = Duration::from_millis(500);
    for (t0, t_end, y0) in [
        (0.0, 0.0, 1.0),
        (0.0, 1e-10, 1.0),
        (1.0, 1.0 + 1e-12, 2.0),
        (1.0, 1.0 - 1e-10, 2.0),
        (0.0, 1e-8 - 1e-12, 1.0),
        (f64::NAN, 1.0, 1.0),
        (0.0, f64::NAN, 1.0),
        (0.0, 1.0, f64::NAN),
        (0.0, f64::INFINITY, 1.0),
        (f64::NEG_INFINITY, 0.0, 1.0),
        (0.0, 1.0, f64::INFINITY),
    ] {
        let start = Instant::now();
        let out = integrate_rk45(t0, t_end, y0, |_t, y| Some(-y));
        assert!(start.elapsed() <= budget, "rk45 hung on ({t0}, {t_end}, {y0})");
        assert!(out.is_none_or(f64::is_finite), "rk45 non-finite {out:?} on ({t0}, {t_end}, {y0})");
    }

    assert!(integrate_rk45(0.0, 5.0, 1.0, |_t, y| Some(-1e5 * y)).is_none_or(f64::is_finite));
    assert!(integrate_rk45(0.0, 5.0, 2.0, |_t, y| Some(y * y)).is_none_or(f64::is_finite));
    assert_eq!(integrate_rk45(0.0, 1.0, 1.0, |t, y| if t > 0.3 { None } else { Some(y) }), None);

    let probe_xs = [0.0, 1e-10, -1e-10, 0.5, 2.0, f64::NAN, f64::INFINITY];
    for src in [
        "Y0 = y0\ndY/dX = -k * Y",
        "Y(nan) = 1\ndY/dX = -Y",
        "Y(inf) = 1\ndY/dX = -Y",
        "Y(-1e-9) = 1\ndY/dX = -Y",
        "Y0 = 2\ndY/dX = Y^2",
        "Y0 = 0\ndY/dX = -1e5 * (Y - 1)",
        "Y0 = 1\nD(Y, X) = IF(X < 0.5, -100 * Y, Y / X)",
        "Y(1) = a\ndY/dT = 50 * cos(50 * T) - Y",
    ] {
        let t0 = Instant::now();
        if let Ok(model) = CompiledDslModel::compile("ode_edge", src) {
            assert_eq!(model.kind(), DslEquationKind::OdeFirstOrder);
            let params = vec![1.0; model.param_count()];
            for xv in probe_xs {
                assert_eval_bounded_and_clean(&model, &[xv], &params, 0, budget, src);
            }
        }
        assert!(t0.elapsed() <= Duration::from_secs(2));
    }
}

#[test]
fn replicates_lack_of_fit_extreme_n_does_not_overflow() {
    let pts: Vec<DataPoint> = [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 1e30, -5.0, 0.0, 1.0, 1.5, 2.0]
        .iter()
        .enumerate()
        .map(|(row, &n_val)| DataPoint { sd: Some(0.5), n: Some(n_val), row, ..DataPoint::xy(1.0 + (row % 2) as f64, 2.0 + row as f64) })
        .collect();
    let _ = replicates_lack_of_fit(&pts, 4.0, 4);

    let model = dsl_compile("Y = a * X + b");
    let mut ds = series_from_pts(0, "A", pts);
    ds.is_pre_summarized = true;
    let cfg = SolverConfig { compute_lack_of_fit: true, max_iters: 40, ..cfg_fast() };
    check_fit_clean(&model, &[ds], &cfg, Duration::from_secs(2), "lack_of_fit_extreme_n");
}

#[test]
fn hostile_fit_inputs_initial_guesses_and_weights() {
    let lin = dsl_compile("Y = a * X + b");
    let exp_m = dsl_compile("Y = A * exp(-k * X)");
    let imp_m = dsl_compile("Y + 0.1 * Y^3 = a * X + b");
    assert_eq!(imp_m.kind(), DslEquationKind::Implicit);
    let unsolv_imp = dsl_compile("Y^2 + a^2 + 1 = X * 0");
    let stiff_ode = dsl_compile("Y0 = A0\ndY/dX = -k * (Y - C)");

    let degenerate_datasets = [
        ("empty", series_xy(0, "empty", &[], &[])),
        ("single", series_xy(0, "single", &[1.0], &[2.0])),
        ("dup_x", series_xy(0, "dup_x", &[2.0; 6], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])),
        ("const_y", series_xy(0, "const_y", &[0.0, 1.0, 2.0, 3.0, 4.0], &[5.0; 5])),
        ("zero_y", series_xy(0, "zero_y", &[0.0, 1.0, 2.0, 3.0, 4.0], &[0.0; 5])),
        ("huge_y", series_xy(0, "huge_y", &[0.0, 1.0, 2.0, 3.0], &[1e150, 2e150, 3e150, 4e150])),
        ("nan_inf_y", series_xy(0, "nan_y", &[0.0, 1.0, f64::NAN, 3.0], &[1.0, f64::INFINITY, 2.0, 3.0])),
    ];
    let methods =
        [FitMethod::LeastSquares, FitMethod::PoissonDeviance, FitMethod::RobustCauchy, FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }];
    let weightings = [
        WeightingScheme::None,
        WeightingScheme::RelativeSquare,
        WeightingScheme::Relative,
        WeightingScheme::GeneralPowerY(2.0),
        WeightingScheme::InverseX,
        WeightingScheme::InverseX2,
        WeightingScheme::ObservedSd,
    ];

    for (ds_name, ds) in &degenerate_datasets {
        for &method in &methods {
            for &weighting in &weightings {
                let cfg = SolverConfig { method, weighting, max_iters: 30, use_simplex_warm_start: true, ..cfg_fast() };
                check_fit_clean(&lin, std::slice::from_ref(ds), &cfg, Duration::from_secs(2), &format!("lin/{ds_name}/{method:?}/{weighting:?}"));
            }
        }
    }

    let normal_ds = series_xy(0, "normal", &[0.0, 1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0, 5.0]);
    for init_ovr in [None, Some(0.0), Some(-1e12), Some(1e300), Some(f64::NAN), Some(f64::INFINITY)] {
        let cfg = SolverConfig {
            parameter_rules: vec![
                rule_local_bounds("a", init_ovr, Some(10.0), Some(5.0)), // deliberately inverted bounds
                rule_local_bounds("A", init_ovr, Some(f64::NAN), Some(1e6)),
                rule_const("k", init_ovr.unwrap_or(1.0)),
            ],
            max_iters: 30,
            compute_profile_ci: true,
            ..cfg_fast()
        };
        for m in [&lin as &dyn ParametricModel, &exp_m, &imp_m, &unsolv_imp] {
            check_fit_clean(m, std::slice::from_ref(&normal_ds), &cfg, Duration::from_secs(2), &format!("override/{}/init={init_ovr:?}", m.id()));
        }
    }

    let ode_ds = series_xy(0, "stiff", &[0.0, 0.2, 0.5, 1.0, 2.0], &[10.0, 1.0, 1.0, 1.0, 1.0]);
    let ode_cfg = SolverConfig {
        parameter_rules: vec![rule_local_bounds("k", Some(1e5), Some(0.0), Some(1e7))],
        max_iters: 25,
        use_simplex_warm_start: false,
        ..cfg_fast()
    };
    check_fit_clean(&stiff_ode, &[ode_ds], &ode_cfg, Duration::from_secs(4), "stiff_ode_fit");

    let degen_cfg = SolverConfig { max_iters: 25, use_simplex_warm_start: false, ..cfg_fast() };
    for (ds_name, ds) in &degenerate_datasets {
        for m in [&imp_m as &dyn ParametricModel, &unsolv_imp, &stiff_ode] {
            check_fit_clean(m, std::slice::from_ref(ds), &degen_cfg, Duration::from_secs(3), &format!("degen/{}/{ds_name}", m.id()));
        }
    }

    let multi_ds_model = dsl_compile("<A> Y = a * X + b\n<B> Y = a * X + c");
    let ds_a = series_xy(0, "A", &[0.0, 1.0, 2.0, 3.0], &[1.0, 3.0, 5.0, 7.0]);
    let ds_b = series_xy(1, "B", &[2.0; 4], &[4.0; 4]);
    let multi_cfg = SolverConfig { parameter_rules: vec![rule_shared("a")], max_iters: 30, compute_profile_ci: true, ..cfg_fast() };
    check_fit_clean(&multi_ds_model, &[ds_a, ds_b], &multi_cfg, Duration::from_secs(2), "multi_ds_shared_degen");
}

// ============================================================================
// Proptest Generators
// ============================================================================

fn arb_expr_strategy() -> BoxedStrategy<String> {
    let static_leaves = select(vec!["X", "X1", "X2", "T", "DS", "PI", "E", "INF", "NAN", "0", "-0", "1e308", "1e-308", "a", "b", "n", "unbound_var"])
        .prop_map(str::to_owned);
    let leaf = prop_oneof![17 => static_leaves, 1 => (-1e6_f64..1e6_f64).prop_map(|v| format!("{v:.4}"))];

    let unary_fns = select(vec![
        "abs", "sqrt", "cbrt", "exp", "exp10", "ln", "log10", "log2", "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "asinh",
        "acosh", "atanh", "floor", "ceil", "round", "trunc", "sign", "erf", "erfc", "normcdf", "probit", "gamma", "lngamma", "digamma", "j0", "j1",
        "y0", "y1", "i0", "i1", "k0", "k1",
    ]);
    let bin_ops = select(vec!["+", "-", "*", "/", "%", "^", "**", "<", "<=", ">", ">=", "==", "!=", "AND", "OR"]);
    let bin_fns = select(vec!["min", "max", "atan2", "hypot", "pow", "beta", "jn", "yn"]);

    leaf.prop_recursive(4, 32, 4, move |inner| {
        prop_oneof![
            3 => (inner.clone(), bin_ops.clone(), inner.clone()).prop_map(|(l, op, r)| format!("({l} {op} {r})")),
            2 => (unary_fns.clone(), inner.clone()).prop_map(|(f, a)| format!("{f}({a})")),
            1 => (inner.clone(),).prop_map(|(a,)| format!("-({a})")),
            1 => (inner.clone(),).prop_map(|(a,)| format!("NOT ({a})")),
            1 => (bin_fns.clone(), inner.clone(), inner.clone()).prop_map(|(f, a, b)| format!("{f}({a}, {b})")),
            1 => (inner.clone(), inner.clone(), inner.clone()).prop_map(|(a, b, c)| format!("clamp({a}, {b}, {c})")),
            1 => (inner.clone(), inner.clone(), inner.clone()).prop_map(|(a, b, c)| format!("IF({a} > 0, {b}, {c})")),
            1 => (inner.clone(), inner.clone(), inner.clone()).prop_map(|(a, b, c)| format!("hyp1f1({a}, {b}, {c})")),
            1 => (inner.clone(), inner.clone(), inner.clone()).prop_map(|(a, b, c)| format!("confluent_u({a}, {b}, {c})")),
            1 => (inner.clone(), inner.clone(), inner.clone(), inner).prop_map(|(a, b, c, d)| format!("hyp2f1({a}, {b}, {c}, {d})")),
        ]
    })
    .boxed()
}

fn arb_fuzzy_token_string() -> BoxedStrategy<String> {
    let static_tokens = select(vec![
        "Y",
        "X",
        "dY/dX",
        "D(Y, X)",
        "Y0",
        "Y(0)",
        "=",
        "==",
        "!=",
        "<>",
        "+",
        "-",
        "*",
        "/",
        "%",
        "^",
        "**",
        "(",
        ")",
        ",",
        "\n",
        "<A>",
        "<~B>",
        "<0:2>",
        "<0:1000000000>",
        "IF",
        "AND",
        "OR",
        "NOT",
        "clamp",
        "jn",
        "exp",
        "ln",
        "NAN",
        "INF",
        "0",
        "1e9",
        "# initial: a=1\n",
        "; comment\n",
        "$$",
        "λ",
        "\0",
    ])
    .prop_map(str::to_owned);
    let ascii_chars = select(vec![
        'A', 'Z', 'a', 'b', 'x', 'y', '0', '9', '_', '<', '>', '(', ')', '+', '-', '*', '/', '^', '!', '=', '.', ';', '#', ':', ',', ' ', '\n', '\t',
    ]);
    let tokens = prop_oneof![
        42 => static_tokens,
        1 => proptest::collection::vec(any::<char>(), 1..=12).prop_map(|cs| cs.into_iter().collect::<String>()),
        1 => proptest::collection::vec(ascii_chars, 1..=12).prop_map(|cs| cs.into_iter().collect::<String>()),
    ];
    proptest::collection::vec(tokens, 0..32).prop_map(|parts| parts.join(" ")).boxed()
}

fn arb_hostile_f64() -> BoxedStrategy<f64> {
    let specials = select(vec![0.0, -0.0, 1.0, -1.0, 1e-12, -1e-12, 1e12, -1e12, 1e150, -1e150, f64::NAN, f64::INFINITY, f64::NEG_INFINITY]);
    prop_oneof![13 => specials, 1 => -1e4_f64..1e4_f64].boxed()
}

fn arb_hostile_dataset() -> BoxedStrategy<DatasetSeries> {
    (0..5_usize, proptest::collection::vec(arb_hostile_f64(), 0..10))
        .prop_map(|(kind, ys)| {
            let points: Vec<DataPoint> = match kind {
                0 => ys.iter().enumerate().map(|(i, &y)| DataPoint::xy(i as f64 * 0.5, y)).collect(),
                1 => ys.iter().enumerate().map(|(row, &y)| DataPoint { sd: Some(0.1), n: Some(2.0), row, ..DataPoint::xy(2.5, y) }).collect(),
                2 => ys.iter().enumerate().map(|(i, _)| DataPoint::xy(i as f64 + 1.0, 3.25)).collect(),
                3 => ys.iter().enumerate().map(|(i, _)| DataPoint::xy(i as f64, 0.0)).collect(),
                _ => ys
                    .iter()
                    .enumerate()
                    .map(|(row, &y)| DataPoint {
                        sd: Some(if row % 2 == 0 { 0.5 } else { f64::NAN }),
                        n: Some(if row % 3 == 0 { f64::INFINITY } else { 3.0 }),
                        explicit_weight: if row % 4 == 0 { Some(-1.0) } else { None },
                        row,
                        ..DataPoint::xy(y * 0.1, y)
                    })
                    .collect(),
            };
            let mut ds = series_from_pts(0, "fuzz_ds", points);
            ds.is_pre_summarized = kind == 1 || kind == 4;
            ds
        })
        .boxed()
}

// ============================================================================
// Proptest Suites
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn fuzz_arbitrary_unicode_tokens_never_panic(src in arb_fuzzy_token_string()) {
        let _ = compile_and_exercise(&src, Duration::from_millis(500));
    }

    #[test]
    fn fuzz_structured_expressions_and_all_builtins_never_panic(
        e1 in arb_expr_strategy(),
        e2 in arb_expr_strategy(),
        with_local in any::<bool>(),
        cyclic_local in any::<bool>(),
        with_prefix in select(vec!["", "<A> ", "<~B> ", "<0:3> "]),
    ) {
        let src = if with_local {
            let rhs_local = if cyclic_local { format!("{e1} + loc") } else { e1 };
            format!("{with_prefix}loc = {rhs_local}\n{with_prefix}Y = {e2} + loc")
        } else {
            format!("{with_prefix}Y = {e1} + {e2}")
        };
        let _ = compile_and_exercise(&src, Duration::from_millis(500));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn fuzz_deep_nesting_and_long_chains(depth in 1_usize..4_096, variant in 0..4_usize) {
        let src = match variant {
            0 => format!("Y = {}X{}", "(".repeat(depth), ")".repeat(depth)),
            1 => format!("Y = {}X", "-".repeat(depth)),
            2 => format!("Y = {}X", "!".repeat(depth)),
            _ => format!("Y = X{}", " + 1".repeat(depth)),
        };
        let _ = compile_and_exercise(&src, Duration::from_secs(2));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn fuzz_implicit_equations_bounded_and_finite_or_none(
        lhs_kind in 0..4_usize,
        rhs_expr in arb_expr_strategy(),
        xv in arb_hostile_f64(),
        p0 in arb_hostile_f64(),
        p1 in arb_hostile_f64(),
    ) {
        let lhs = ["Y + a * Y^3", "sin(Y) + b * Y", "exp(Y) - Y^2", "IF(Y > a, Y^2, Y - 1)"][lhs_kind];
        let src = format!("{lhs} = {rhs_expr}");
        let t0 = Instant::now();
        if let Ok(model) = CompiledDslModel::compile("imp_fuzz", &src) {
            prop_assert!(t0.elapsed() <= Duration::from_secs(1));
            let params: Vec<f64> = (0..model.param_count()).map(|i| if i % 2 == 0 { p0 } else { p1 }).collect();
            assert_eval_bounded_and_clean(&model, &[xv], &params, 0, Duration::from_millis(500), &src);
        }
    }

    #[test]
    fn fuzz_rk45_and_ode_dsl_never_panic_or_hang(
        span_kind in 0..5_usize,
        t0_base in -10.0_f64..10.0_f64,
        y0 in arb_hostile_f64(),
        rhs_kind in 0..5_usize,
        k in prop_oneof![Just(-1.0), Just(1.0), Just(1e3), Just(1e6), Just(f64::NAN), -50.0_f64..50.0_f64],
    ) {
        let (t0, t_end) = match span_kind {
            0 => (t0_base, t0_base + 1e-10),
            1 => (t0_base, t0_base - 1e-11),
            2 => (t0_base, t0_base + 5.0),
            3 => (f64::NAN, t0_base + 1.0),
            _ => (0.0, f64::INFINITY),
        };
        let start = Instant::now();
        let out = integrate_rk45(t0, t_end, y0, |t, y| {
            let v = match rhs_kind {
                0 => -k * y,
                1 => y * y + k,
                2 => k * (t * 20.0).cos() - y,
                3 => if y > 0.0 { -1e4 * y } else { 1e4 * y },
                _ => y / t,
            };
            v.is_finite().then_some(v)
        });
        prop_assert!(start.elapsed() <= Duration::from_millis(750));
        prop_assert!(out.is_none_or(f64::is_finite));

        let ode_rhs = ["-k * Y", "Y^2 + k", "k * cos(20 * X) - Y", "IF(Y > 0, -1e4 * Y, 1e4 * Y)", "Y / X"][rhs_kind];
        let src = format!("Y({t0}) = a\ndY/dX = {ode_rhs}");
        if let Ok(model) = CompiledDslModel::compile("ode_fuzz", &src) {
            prop_assert_eq!(model.kind(), DslEquationKind::OdeFirstOrder);
            let params: Vec<f64> = (0..model.param_count()).map(|i| if i == 0 { y0 } else { k }).collect();
            assert_eval_bounded_and_clean(&model, &[t_end], &params, 0, Duration::from_millis(750), &src);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn fuzz_fit_nonlinear_hostile_data_rules_methods_and_weights(
        eq_kind in 0..4_usize,
        ds in arb_hostile_dataset(),
        method_kind in 0..4_usize,
        weight_kind in 0..7_usize,
        init_ovr in select(vec![None, Some(0.0), Some(-1e10), Some(f64::NAN), Some(f64::INFINITY)]),
        lb in select(vec![None, Some(-10.0), Some(100.0), Some(f64::NAN)]),
        ub in select(vec![None, Some(5.0), Some(f64::INFINITY)]),
        ci in any::<bool>(),
    ) {
        let src = ["Y = a * X + b", "Y = a * exp(-b * X)", "Y + 0.05 * Y^3 = a * X + b", "Y0 = a\ndY/dX = -b * Y"][eq_kind];
        let model = dsl_compile(src);
        let method = [
            FitMethod::LeastSquares,
            FitMethod::PoissonDeviance,
            FitMethod::RobustCauchy,
            FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        ][method_kind];
        let weighting = [
            WeightingScheme::None,
            WeightingScheme::RelativeSquare,
            WeightingScheme::Relative,
            WeightingScheme::GeneralPowerY(2.0),
            WeightingScheme::InverseX,
            WeightingScheme::InverseX2,
            WeightingScheme::ObservedSd,
        ][weight_kind];
        let cfg = SolverConfig {
            method,
            weighting,
            parameter_rules: vec![rule_local_bounds("a", init_ovr, lb, ub)],
            max_iters: 35,
            compute_profile_ci: ci && eq_kind != 3,
            compute_lack_of_fit: true,
            use_simplex_warm_start: eq_kind != 3,
            ..cfg_fast()
        };
        check_fit_clean(&model, &[ds], &cfg, Duration::from_secs(4), src);
    }
}
