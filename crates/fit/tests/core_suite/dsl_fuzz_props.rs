//! Property-based and adversarial fuzz suite for the equation DSL parser/compiler,
//! stack VM evaluator, implicit root solver, RK45 ODE integrator, and nonlinear fitter.
#![forbid(unsafe_code)]

use curvebook_doc::{FitMethod, ParameterRule, ParameterScope, WeightingScheme};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, DslEquationKind, HookContext, ParametricModel, SolverConfig, fit_nonlinear, integrate_rk45,
    replicates_lack_of_fit,
};
use proptest::prelude::{BoxedStrategy, Just, ProptestConfig, Strategy, any, prop_assert, prop_assert_eq, prop_oneof, proptest};
use std::f64::consts::{FRAC_PI_2, PI};
use std::time::{Duration, Instant};

fn make_series(idx: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
    let points = xs
        .iter()
        .zip(ys.iter())
        .enumerate()
        .map(|(row, (&x, &y))| DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: idx, row })
        .collect();
    DatasetSeries {
        data_set_index: idx,
        label: label.to_owned(),
        points,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn adversarial_f64_values() -> Vec<f64> {
    vec![
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
    ]
}

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

    let hostiles = [0.0, -0.0, 1.0, -1.0, 1e-10, 1e12, f64::NAN, f64::INFINITY];
    for &xv in &hostiles {
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
            for &pv in &ds.params {
                assert!(pv.is_finite(), "non-finite dataset param {pv} for `{ctx}`");
            }
            for &yh in &ds.y_hat {
                assert!(yh.is_finite(), "non-finite y_hat {yh} for `{ctx}`");
            }
        }
    }
}

// ============================================================================
// Targeted Deterministic Adversarial Edge Cases
// ============================================================================

#[test]
fn empty_whitespace_comment_and_huge_equations_return_cleanly() {
    let empty_cases = [
        "",
        "   \n\t\r\n  ",
        "# pure comment\n// slash comment\n; semicolon comment",
        "# initial: a=1, b=NAN\n# t0=INF\n",
        "<A>   \n<B> # just comment",
    ];
    for case in empty_cases {
        let res = CompiledDslModel::compile("empty", case);
        assert!(res.is_err(), "expected empty equation Err for `{case:?}`");
    }

    // Huge expressions, long identifiers, long numeric literals, many locals.
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
        let parens = format!("Y = {}X{}", "(".repeat(depth), ")".repeat(depth));
        let unary_neg = format!("Y = {}X", "-".repeat(depth));
        let unary_not = format!("Y = {}X", "!".repeat(depth));
        let right_pow = format!("Y = {}X", "2 ^ ".repeat(depth));
        let nested_if = format!("Y = {}X{}", "IF(X > 0, ".repeat(depth.min(2_500)), ", 0)".repeat(depth.min(2_500)));

        for src in [&parens, &unary_neg, &unary_not, &right_pow, &nested_if] {
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
        let elapsed = t0.elapsed();
        assert!(elapsed <= budget, "prefix `{src}` took {elapsed:?} > {budget:?}");
        if let Ok(model) = res {
            assert_eval_bounded_and_clean(&model, &[1.0], &[1.0; 4], 0, budget, src);
        }
    }
}

#[test]
fn vm_domain_errors_and_overflow_semantics() {
    let budget = Duration::from_millis(250);

    // Specific expected semantic checks.
    let zero_pow_zero = CompiledDslModel::compile("z0", "Y = 0 ^ 0").expect("0^0 compiles");
    assert_eq!(zero_pow_zero.eval(&[0.0], &[], 0), Some(1.0));

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
        let m = CompiledDslModel::compile("dom", none_src).expect("domain expr compiles");
        assert_eq!(m.eval(&[1.0], &[], 0), None, "expected None for `{none_src}`");
    }

    let trig_sing = CompiledDslModel::compile("tan_pi_2", "Y = tan(PI / 2)").expect("tan compiles");
    assert_eval_bounded_and_clean(&trig_sing, &[0.0], &[], 0, budget, "tan(PI/2)");

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
    let vals = adversarial_f64_values();
    for src in domain_exprs {
        let m = CompiledDslModel::compile("dom_all", src).expect("compiles");
        for &xv in &vals {
            assert_eval_bounded_and_clean(&m, &[xv], &[], 0, budget, src);
        }
    }

    // Intermediate overflows and IEEE special arithmetic folded inside subexpressions.
    let intermediate_overflows = [
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
    ];
    for src in intermediate_overflows {
        let m = CompiledDslModel::compile("interm", src).expect("compiles");
        assert_eval_bounded_and_clean(&m, &[1.0], &[], 0, budget, src);
    }
}

#[test]
fn malformed_syntax_and_token_edge_cases() {
    let budget = Duration::from_millis(250);
    let malformed = [
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
    ];
    for src in malformed {
        let _ = compile_and_exercise(src, budget);
    }
}

#[test]
fn clamp_nan_bounds_and_bessel_order_extremes_do_not_panic_or_hang() {
    let budget = Duration::from_millis(300);

    for src in ["Y = clamp(X, NAN, NAN)", "Y = clamp(X, INF - INF, 1)", "Y = clamp(X, 1, NAN)", "Y = clamp(NAN, 0, 1)", "Y = clamp(X, a, b)"] {
        let m = CompiledDslModel::compile("clamp_nan", src).expect("clamp compiles");
        let n_p = m.param_count();
        for &xv in &[0.0, 1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let p_nan = vec![f64::NAN; n_p];
            let p_inf = vec![f64::INFINITY; n_p];
            assert_eval_bounded_and_clean(&m, &[xv], &p_nan, 0, budget, src);
            assert_eval_bounded_and_clean(&m, &[xv], &p_inf, 0, budget, src);
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
        let m = CompiledDslModel::compile("bessel_ext", src).expect("bessel compiles");
        let n_p = m.param_count();
        for &order in &[-1e15, -2_147_483_648.0, 0.0, 100_001.0, 1e12, f64::NAN, f64::INFINITY] {
            let params = vec![order; n_p];
            assert_eval_bounded_and_clean(&m, &[1.5], &params, 0, budget, src);
            assert_eval_bounded_and_clean(&m, &[0.0], &params, 0, budget, src);
            assert_eval_bounded_and_clean(&m, &[-2.5], &params, 0, budget, src);
        }
    }
}

#[test]
fn unbound_shadowed_cyclic_and_multi_predictor_identifiers() {
    let budget = Duration::from_millis(250);
    let cases = [
        // Self-referential / mutually cyclic locals & mixed case builtins
        "a = a + 1\nY = a * X",
        "a = b + 1\nb = a + 1\nY = a + b + X",
        "Y = x + X1 + x2 + X99999999999999999999 + pI + e + InF + NaN + dS + DaTaSeT + unbound_p",
        "<A> k = 1\n<B> Y = k * X",
        "k = X * p\nk = k + 1\nY = k",
        "# initial: p=NAN, q=INF\nY = p * X + q",
    ];
    for src in cases {
        let _ = compile_and_exercise(src, budget);
    }

    // Dataset selector scoping verification.
    let m = CompiledDslModel::compile("ds", "<A> Y = a * X\n<B> Y = b * X1 + c * X2").expect("multi-dataset compiles");
    assert!(m.param_count() >= 3);
    assert_eval_bounded_and_clean(&m, &[2.0, 3.0], &[1.0; 3], 0, budget, "ds0");
    assert_eval_bounded_and_clean(&m, &[2.0, 3.0], &[1.0; 3], 1, budget, "ds1");
    assert_eq!(m.eval(&[2.0, 3.0], &[1.0; 3], 2), None);
}

#[test]
fn rk45_small_span_non_finite_and_stiff_ode_edge_cases() {
    let budget = Duration::from_millis(500);

    // Direct integrate_rk45 edge cases (tiny span < 1e-8, NaN/Inf endpoints & y0, blowup, stiff).
    let direct_cases: [(f64, f64, f64); 11] = [
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
    ];
    for (t0, t_end, y0) in direct_cases {
        let start = Instant::now();
        let out = integrate_rk45(t0, t_end, y0, |_t, y| Some(-y));
        assert!(start.elapsed() <= budget, "rk45 hung on ({t0}, {t_end}, {y0})");
        if let Some(v) = out {
            assert!(v.is_finite(), "rk45 returned non-finite Some({v}) on ({t0}, {t_end}, {y0})");
        }
    }

    // Stiff, finite-time blowup, and domain-failing RHS in direct integrate_rk45.
    let stiff = integrate_rk45(0.0, 5.0, 1.0, |_t, y| Some(-1e5 * y));
    if let Some(v) = stiff {
        assert!(v.is_finite());
    }
    let blowup = integrate_rk45(0.0, 5.0, 2.0, |_t, y| Some(y * y));
    assert!(blowup.is_none() || blowup.is_some_and(f64::is_finite));
    let domain_none = integrate_rk45(0.0, 1.0, 1.0, |t, y| if t > 0.3 { None } else { Some(y) });
    assert_eq!(domain_none, None);

    // Via DSL ODE parser + evaluator.
    let ode_sources = [
        "Y0 = y0\ndY/dX = -k * Y",
        "Y(nan) = 1\ndY/dX = -Y",
        "Y(inf) = 1\ndY/dX = -Y",
        "Y(-1e-9) = 1\ndY/dX = -Y",
        "Y0 = 2\ndY/dX = Y^2",
        "Y0 = 0\ndY/dX = -1e5 * (Y - 1)",
        "Y0 = 1\nD(Y, X) = IF(X < 0.5, -100 * Y, Y / X)",
        "Y(1) = a\ndY/dT = 50 * cos(50 * T) - Y",
    ];
    let probe_xs = [0.0, 1e-10, -1e-10, 0.5, 2.0, f64::NAN, f64::INFINITY];
    for src in ode_sources {
        let t0 = Instant::now();
        if let Ok(model) = CompiledDslModel::compile("ode_edge", src) {
            assert_eq!(model.kind(), DslEquationKind::OdeFirstOrder);
            let params = vec![1.0; model.param_count()];
            for &xv in &probe_xs {
                assert_eval_bounded_and_clean(&model, &[xv], &params, 0, budget, src);
            }
        }
        assert!(t0.elapsed() <= Duration::from_secs(2));
    }
}

#[test]
fn replicates_lack_of_fit_extreme_n_does_not_overflow() {
    let hostile_ns = [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 1e30, -5.0, 0.0, 1.0, 1.5, 2.0];
    let mut pts = Vec::new();
    for (i, &n_val) in hostile_ns.iter().enumerate() {
        pts.push(DataPoint {
            x: vec![1.0 + (i % 2) as f64],
            y: 2.0 + i as f64,
            sd: Some(0.5),
            n: Some(n_val),
            explicit_weight: None,
            source_subcolumn: 0,
            data_set_index: 0,
            row: i,
        });
    }
    let _ = replicates_lack_of_fit(&pts, 4.0, 4);

    // Also probe via fit_nonlinear with compute_lack_of_fit enabled.
    let model = CompiledDslModel::compile("lin", "Y = a * X + b").expect("linear compiles");
    let ds = DatasetSeries {
        data_set_index: 0,
        label: "A".to_owned(),
        points: pts,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: true,
        column_title_value: None,
    };
    let cfg = SolverConfig {
        compute_lack_of_fit: true,
        max_iters: 40,
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        ..SolverConfig::default()
    };
    check_fit_clean(&model, &[ds], &cfg, Duration::from_secs(2), "lack_of_fit_extreme_n");
}

#[test]
fn hostile_fit_inputs_initial_guesses_and_weights() {
    let lin = CompiledDslModel::compile("lin", "Y = a * X + b").expect("lin compiles");
    let exp_m = CompiledDslModel::compile("exp", "Y = A * exp(-k * X)").expect("exp compiles");
    let imp_m = CompiledDslModel::compile("imp", "Y + 0.1 * Y^3 = a * X + b").expect("imp compiles");
    assert_eq!(imp_m.kind(), DslEquationKind::Implicit);
    let unsolv_imp = CompiledDslModel::compile("imp_unsolv", "Y^2 + a^2 + 1 = X * 0").expect("compiles");
    let stiff_ode = CompiledDslModel::compile("ode_stiff", "Y0 = A0\ndY/dX = -k * (Y - C)").expect("ode compiles");

    let degenerate_datasets = [
        ("empty", make_series(0, "empty", &[], &[])),
        ("single", make_series(0, "single", &[1.0], &[2.0])),
        ("dup_x", make_series(0, "dup_x", &[2.0; 6], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])),
        ("const_y", make_series(0, "const_y", &[0.0, 1.0, 2.0, 3.0, 4.0], &[5.0; 5])),
        ("zero_y", make_series(0, "zero_y", &[0.0, 1.0, 2.0, 3.0, 4.0], &[0.0; 5])),
        ("huge_y", make_series(0, "huge_y", &[0.0, 1.0, 2.0, 3.0], &[1e150, 2e150, 3e150, 4e150])),
        ("nan_inf_y", make_series(0, "nan_y", &[0.0, 1.0, f64::NAN, 3.0], &[1.0, f64::INFINITY, 2.0, 3.0])),
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
    let hostile_overrides = [None, Some(0.0), Some(-1e12), Some(1e300), Some(f64::NAN), Some(f64::INFINITY)];

    for (ds_name, ds) in &degenerate_datasets {
        for &method in &methods {
            for &weighting in &weightings {
                let cfg = SolverConfig {
                    method,
                    weighting,
                    max_iters: 30,
                    compute_profile_ci: false,
                    compute_hougaard: false,
                    compute_bands: false,
                    compute_runs_test: false,
                    use_simplex_warm_start: true,
                    ..SolverConfig::default()
                };
                check_fit_clean(&lin, std::slice::from_ref(ds), &cfg, Duration::from_secs(2), &format!("lin/{ds_name}/{method:?}/{weighting:?}"));
            }
        }
    }

    for &init_ovr in &hostile_overrides {
        let rules = vec![
            ParameterRule {
                name: "a".to_owned(),
                scope: ParameterScope::Local,
                initial_override: init_ovr,
                lower_bound: Some(10.0),
                upper_bound: Some(5.0), // deliberately inverted bounds
            },
            ParameterRule {
                name: "A".to_owned(),
                scope: ParameterScope::Local,
                initial_override: init_ovr,
                lower_bound: Some(f64::NAN),
                upper_bound: Some(1e6),
            },
            ParameterRule {
                name: "k".to_owned(),
                scope: ParameterScope::Constant(init_ovr.unwrap_or(1.0)),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ];
        let cfg = SolverConfig {
            parameter_rules: rules,
            max_iters: 30,
            compute_profile_ci: true,
            compute_hougaard: false,
            compute_bands: false,
            ..SolverConfig::default()
        };
        let ds = make_series(0, "normal", &[0.0, 1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0, 5.0]);
        for m in [&lin as &dyn ParametricModel, &exp_m, &imp_m, &unsolv_imp] {
            check_fit_clean(m, std::slice::from_ref(&ds), &cfg, Duration::from_secs(2), &format!("override/{}/init={init_ovr:?}", m.id()));
        }
    }

    // Hostile stiff ODE fit must stay bounded in time and never panic.
    let ode_ds = make_series(0, "stiff", &[0.0, 0.2, 0.5, 1.0, 2.0], &[10.0, 1.0, 1.0, 1.0, 1.0]);
    let ode_cfg = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: "k".to_owned(),
            scope: ParameterScope::Local,
            initial_override: Some(1e5),
            lower_bound: Some(0.0),
            upper_bound: Some(1e7),
        }],
        max_iters: 25,
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        use_simplex_warm_start: false,
        ..SolverConfig::default()
    };
    check_fit_clean(&stiff_ode, &[ode_ds], &ode_cfg, Duration::from_secs(4), "stiff_ode_fit");

    // Exercise degenerate datasets directly on implicit and ODE models too.
    let degen_cfg = SolverConfig {
        max_iters: 25,
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_runs_test: false,
        use_simplex_warm_start: false,
        ..SolverConfig::default()
    };
    for (ds_name, ds) in &degenerate_datasets {
        for m in [&imp_m as &dyn ParametricModel, &unsolv_imp, &stiff_ode] {
            check_fit_clean(m, std::slice::from_ref(ds), &degen_cfg, Duration::from_secs(3), &format!("degen/{}/{ds_name}", m.id()));
        }
    }

    // Multi-dataset global-vs-local sharing with one degenerate dataset and custom DSL prefixes.
    let multi_ds_model = CompiledDslModel::compile("multi", "<A> Y = a * X + b\n<B> Y = a * X + c").expect("multi compiles");
    let ds_a = make_series(0, "A", &[0.0, 1.0, 2.0, 3.0], &[1.0, 3.0, 5.0, 7.0]);
    let ds_b = make_series(1, "B", &[2.0; 4], &[4.0; 4]);
    let multi_cfg = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: "a".to_owned(),
            scope: ParameterScope::SharedGlobal,
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        max_iters: 30,
        compute_profile_ci: true,
        compute_hougaard: false,
        compute_bands: false,
        ..SolverConfig::default()
    };
    check_fit_clean(&multi_ds_model, &[ds_a, ds_b], &multi_cfg, Duration::from_secs(2), "multi_ds_shared_degen");
}

// ============================================================================
// Proptest Generators
// ============================================================================

fn arb_expr_strategy() -> BoxedStrategy<String> {
    let leaf = prop_oneof![
        Just("X".to_owned()),
        Just("X1".to_owned()),
        Just("X2".to_owned()),
        Just("T".to_owned()),
        Just("DS".to_owned()),
        Just("PI".to_owned()),
        Just("E".to_owned()),
        Just("INF".to_owned()),
        Just("NAN".to_owned()),
        Just("0".to_owned()),
        Just("-0".to_owned()),
        Just("1e308".to_owned()),
        Just("1e-308".to_owned()),
        Just("a".to_owned()),
        Just("b".to_owned()),
        Just("n".to_owned()),
        Just("unbound_var".to_owned()),
        (-1e6_f64..1e6_f64).prop_map(|v| format!("{v:.4}")),
    ];

    let unary_fns = prop_oneof![
        Just("abs"),
        Just("sqrt"),
        Just("cbrt"),
        Just("exp"),
        Just("exp10"),
        Just("ln"),
        Just("log10"),
        Just("log2"),
        Just("sin"),
        Just("cos"),
        Just("tan"),
        Just("asin"),
        Just("acos"),
        Just("atan"),
        Just("sinh"),
        Just("cosh"),
        Just("tanh"),
        Just("asinh"),
        Just("acosh"),
        Just("atanh"),
        Just("floor"),
        Just("ceil"),
        Just("round"),
        Just("trunc"),
        Just("sign"),
        Just("erf"),
        Just("erfc"),
        Just("normcdf"),
        Just("probit"),
        Just("gamma"),
        Just("lngamma"),
        Just("digamma"),
        Just("j0"),
        Just("j1"),
        Just("y0"),
        Just("y1"),
        Just("i0"),
        Just("i1"),
        Just("k0"),
        Just("k1"),
    ];

    let bin_ops = prop_oneof![
        Just("+"),
        Just("-"),
        Just("*"),
        Just("/"),
        Just("%"),
        Just("^"),
        Just("**"),
        Just("<"),
        Just("<="),
        Just(">"),
        Just(">="),
        Just("=="),
        Just("!="),
        Just("AND"),
        Just("OR"),
    ];

    let bin_fns = prop_oneof![Just("min"), Just("max"), Just("atan2"), Just("hypot"), Just("pow"), Just("beta"), Just("jn"), Just("yn"),];

    leaf.prop_recursive(4, 32, 4, move |inner| {
        prop_oneof![
            3 => (inner.clone(), bin_ops.clone(), inner.clone())
                .prop_map(|(l, op, r)| format!("({l} {op} {r})")),
            2 => (unary_fns.clone(), inner.clone()).prop_map(|(f, a)| format!("{f}({a})")),
            1 => (inner.clone(),).prop_map(|(a,)| format!("-({a})")),
            1 => (inner.clone(),).prop_map(|(a,)| format!("NOT ({a})")),
            1 => (bin_fns.clone(), inner.clone(), inner.clone())
                .prop_map(|(f, a, b)| format!("{f}({a}, {b})")),
            1 => (inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(a, b, c)| format!("clamp({a}, {b}, {c})")),
            1 => (inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(a, b, c)| format!("IF({a} > 0, {b}, {c})")),
            1 => (inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(a, b, c)| format!("hyp1f1({a}, {b}, {c})")),
            1 => (inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(a, b, c)| format!("confluent_u({a}, {b}, {c})")),
            1 => (inner.clone(), inner.clone(), inner.clone(), inner)
                .prop_map(|(a, b, c, d)| format!("hyp2f1({a}, {b}, {c}, {d})")),
        ]
    })
    .boxed()
}

fn arb_fuzzy_token_string() -> BoxedStrategy<String> {
    let tokens = prop_oneof![
        Just("Y".to_owned()),
        Just("X".to_owned()),
        Just("dY/dX".to_owned()),
        Just("D(Y, X)".to_owned()),
        Just("Y0".to_owned()),
        Just("Y(0)".to_owned()),
        Just("=".to_owned()),
        Just("==".to_owned()),
        Just("!=".to_owned()),
        Just("<>".to_owned()),
        Just("+".to_owned()),
        Just("-".to_owned()),
        Just("*".to_owned()),
        Just("/".to_owned()),
        Just("%".to_owned()),
        Just("^".to_owned()),
        Just("**".to_owned()),
        Just("(".to_owned()),
        Just(")".to_owned()),
        Just(",".to_owned()),
        Just("\n".to_owned()),
        Just("<A>".to_owned()),
        Just("<~B>".to_owned()),
        Just("<0:2>".to_owned()),
        Just("<0:1000000000>".to_owned()),
        Just("IF".to_owned()),
        Just("AND".to_owned()),
        Just("OR".to_owned()),
        Just("NOT".to_owned()),
        Just("clamp".to_owned()),
        Just("jn".to_owned()),
        Just("exp".to_owned()),
        Just("ln".to_owned()),
        Just("NAN".to_owned()),
        Just("INF".to_owned()),
        Just("0".to_owned()),
        Just("1e9".to_owned()),
        Just("# initial: a=1\n".to_owned()),
        Just("; comment\n".to_owned()),
        Just("$$".to_owned()),
        Just("λ".to_owned()),
        Just("\0".to_owned()),
        proptest::collection::vec(any::<char>(), 1..=12).prop_map(|chars| chars.into_iter().collect::<String>()),
        proptest::collection::vec(
            proptest::sample::select(vec![
                'A', 'Z', 'a', 'b', 'x', 'y', '0', '9', '_', '<', '>', '(', ')', '+', '-', '*', '/', '^', '!', '=', '.', ';', '#', ':', ',', ' ',
                '\n', '\t',
            ]),
            1..=12,
        )
        .prop_map(|chars| chars.into_iter().collect::<String>()),
    ];
    proptest::collection::vec(tokens, 0..32).prop_map(|parts| parts.join(" ")).boxed()
}

fn arb_hostile_f64() -> BoxedStrategy<f64> {
    prop_oneof![
        Just(0.0),
        Just(-0.0),
        Just(1.0),
        Just(-1.0),
        Just(1e-12),
        Just(-1e-12),
        Just(1e12),
        Just(-1e12),
        Just(1e150),
        Just(-1e150),
        Just(f64::NAN),
        Just(f64::INFINITY),
        Just(f64::NEG_INFINITY),
        -1e4_f64..1e4_f64,
    ]
    .boxed()
}

fn arb_hostile_dataset() -> BoxedStrategy<DatasetSeries> {
    let mode = 0..5_usize;
    let vals = proptest::collection::vec(arb_hostile_f64(), 0..10);
    (mode, vals)
        .prop_map(|(kind, ys)| {
            let points: Vec<DataPoint> = match kind {
                // Normal/arbitrary spacing with potential NaN/Inf.
                0 => ys.iter().enumerate().map(|(i, &y)| DataPoint::xy(i as f64 * 0.5, y)).collect(),
                // All duplicate X values.
                1 => ys
                    .iter()
                    .enumerate()
                    .map(|(row, &y)| DataPoint {
                        x: vec![2.5],
                        y,
                        sd: Some(0.1),
                        n: Some(2.0),
                        explicit_weight: None,
                        source_subcolumn: 0,
                        data_set_index: 0,
                        row,
                    })
                    .collect(),
                // Constant Y vector.
                2 => ys.iter().enumerate().map(|(i, _)| DataPoint::xy(i as f64 + 1.0, 3.25)).collect(),
                // All-zero Y vector and X=0 point.
                3 => ys.iter().enumerate().map(|(i, _)| DataPoint::xy(i as f64, 0.0)).collect(),
                // Non-finite sd/n/explicit_weight mixtures.
                _ => ys
                    .iter()
                    .enumerate()
                    .map(|(row, &y)| DataPoint {
                        x: vec![y * 0.1],
                        y,
                        sd: Some(if row % 2 == 0 { 0.5 } else { f64::NAN }),
                        n: Some(if row % 3 == 0 { f64::INFINITY } else { 3.0 }),
                        explicit_weight: if row % 4 == 0 { Some(-1.0) } else { None },
                        source_subcolumn: 0,
                        data_set_index: 0,
                        row,
                    })
                    .collect(),
            };
            DatasetSeries {
                data_set_index: 0,
                label: "fuzz_ds".to_owned(),
                points,
                unknowns_y: Vec::new(),
                unknowns_x: Vec::new(),
                is_pre_summarized: kind == 1 || kind == 4,
                column_title_value: None,
            }
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
        with_prefix in prop_oneof![Just(""), Just("<A> "), Just("<~B> "), Just("<0:3> ")],
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
    fn fuzz_deep_nesting_and_long_chains(
        depth in 1_usize..4_096,
        variant in 0..4_usize,
    ) {
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
        let lhs = match lhs_kind {
            0 => "Y + a * Y^3",
            1 => "sin(Y) + b * Y",
            2 => "exp(Y) - Y^2",
            _ => "IF(Y > a, Y^2, Y - 1)",
        };
        let src = format!("{lhs} = {rhs_expr}");
        let t0 = Instant::now();
        if let Ok(model) = CompiledDslModel::compile("imp_fuzz", &src) {
            prop_assert!(t0.elapsed() <= Duration::from_secs(1));
            let params: Vec<f64> = (0..model.param_count())
                .map(|i| if i % 2 == 0 { p0 } else { p1 })
                .collect();
            assert_eval_bounded_and_clean(
                &model,
                &[xv],
                &params,
                0,
                Duration::from_millis(500),
                &src,
            );
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
            if v.is_finite() { Some(v) } else { None }
        });
        prop_assert!(start.elapsed() <= Duration::from_millis(750));
        if let Some(v) = out {
            prop_assert!(v.is_finite());
        }

        // Also compile and evaluate a DSL ODE equation using Y(t0) = y0.
        let ode_rhs = match rhs_kind {
            0 => "-k * Y",
            1 => "Y^2 + k",
            2 => "k * cos(20 * X) - Y",
            3 => "IF(Y > 0, -1e4 * Y, 1e4 * Y)",
            _ => "Y / X",
        };
        let src = format!("Y({t0}) = a\ndY/dX = {ode_rhs}");
        if let Ok(model) = CompiledDslModel::compile("ode_fuzz", &src) {
            prop_assert_eq!(model.kind(), DslEquationKind::OdeFirstOrder);
            let params: Vec<f64> = (0..model.param_count())
                .map(|i| if i == 0 { y0 } else { k })
                .collect();
            assert_eval_bounded_and_clean(
                &model,
                &[t_end],
                &params,
                0,
                Duration::from_millis(750),
                &src,
            );
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
        init_ovr in prop_oneof![Just(None), Just(Some(0.0)), Just(Some(-1e10)), Just(Some(f64::NAN)), Just(Some(f64::INFINITY))],
        lb in prop_oneof![Just(None), Just(Some(-10.0)), Just(Some(100.0)), Just(Some(f64::NAN))],
        ub in prop_oneof![Just(None), Just(Some(5.0)), Just(Some(f64::INFINITY))],
        ci in any::<bool>(),
    ) {
        let src = match eq_kind {
            0 => "Y = a * X + b",
            1 => "Y = a * exp(-b * X)",
            2 => "Y + 0.05 * Y^3 = a * X + b",
            _ => "Y0 = a\ndY/dX = -b * Y",
        };
        let model = CompiledDslModel::compile("fit_fuzz", src).expect("fixed model compiles");
        let method = match method_kind {
            0 => FitMethod::LeastSquares,
            1 => FitMethod::PoissonDeviance,
            2 => FitMethod::RobustCauchy,
            _ => FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        };
        let weighting = match weight_kind {
            0 => WeightingScheme::None,
            1 => WeightingScheme::RelativeSquare,
            2 => WeightingScheme::Relative,
            3 => WeightingScheme::GeneralPowerY(2.0),
            4 => WeightingScheme::InverseX,
            5 => WeightingScheme::InverseX2,
            _ => WeightingScheme::ObservedSd,
        };
        let rules = vec![ParameterRule {
            name: "a".to_owned(),
            scope: ParameterScope::Local,
            initial_override: init_ovr,
            lower_bound: lb,
            upper_bound: ub,
        }];
        let cfg = SolverConfig {
            method,
            weighting,
            parameter_rules: rules,
            max_iters: 35,
            compute_profile_ci: ci && eq_kind != 3,
            compute_hougaard: false,
            compute_bands: false,
            compute_runs_test: false,
            compute_lack_of_fit: true,
            use_simplex_warm_start: eq_kind != 3,
            ..SolverConfig::default()
        };
        check_fit_clean(&model, &[ds], &cfg, Duration::from_secs(4), src);
    }
}
