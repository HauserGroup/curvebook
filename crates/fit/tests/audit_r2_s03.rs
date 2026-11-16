//! Permanent Round 2 (`R0.19`) audit regression suite for Slice `S03` (`crates/fit/*`).
//! Covers all 9 surviving findings `R2-S03-01` .. `R2-S03-09`.

use std::collections::HashMap;

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, MultipleVariablesTable, NonlinearFitSpec, ParameterRule, ParameterScope, ResultSheet,
    ResultStatus, SubcolumnMode, TableArchetype, XyTable,
};
use lumencraft_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, FitStatus, HookContext, SolverConfig, execute_analysis, fit_nonlinear, format_model_equation_tex,
    format_model_equation_tex_for_dataset, format_polynomial_equation_tex, get_builtin_model,
};

fn collect_ds_params(result: &ResultSheet, ds_index: usize) -> Vec<(String, f64)> {
    let mut by_key: HashMap<String, f64> = HashMap::new();
    for s in &result.scalars {
        if s.data_set_index.is_none() && s.value.is_finite() {
            by_key.insert(s.key.clone(), s.value);
        }
    }
    for s in &result.scalars {
        if s.data_set_index == Some(ds_index) && s.value.is_finite() {
            by_key.insert(s.key.clone(), s.value);
        }
    }
    by_key.into_iter().collect()
}

fn check_tex_text_escapes(tex: &str) -> Result<(), String> {
    let bytes = tex.as_bytes();
    let mut i = 0_usize;
    let mut depth = 0_i32;

    while i < bytes.len() {
        if tex[i..].starts_with("\\text{") {
            depth += 1;
            i += "\\text{".len();
            let text_depth = depth;
            while i < bytes.len() && depth >= text_depth {
                match bytes[i] {
                    b'\\' => {
                        i += 1;
                        if i >= bytes.len() {
                            return Err(format!("dangling backslash inside \\text{{...}} in `{tex}`"));
                        }
                        i += 1;
                    }
                    b'{' => return Err(format!("unescaped `{{` inside \\text{{...}} in `{tex}`")),
                    b'}' => {
                        if depth == text_depth {
                            depth -= 1;
                            i += 1;
                            break;
                        }
                        return Err(format!("unescaped `}}` inside \\text{{...}} in `{tex}`"));
                    }
                    b'_' | b'^' | b'*' | b'$' | b'&' | b'%' | b'#' | b'<' | b'>' => {
                        return Err(format!("unescaped metachar `{}` inside \\text{{...}} in `{tex}`", bytes[i] as char));
                    }
                    _ => i += 1,
                }
            }
            continue;
        }
        match bytes[i] {
            b'\\' => {
                i += 1;
                if i < bytes.len() {
                    i += 1;
                }
            }
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!("unmatched `}}` in `{tex}`"));
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    if depth != 0 {
        return Err(format!("unbalanced braces in `{tex}`"));
    }
    Ok(())
}

/// Assert balanced `{}` braces (respecting `\` escapes) and no unescaped TeX metacharacters
/// (`_`, `^`, `{`, `}`, `*`, `$`, `&`, `%`, `#`, `<`, `>`) inside `\text{...}` spans.
fn assert_valid_tex_text_escapes(tex: &str) {
    assert_eq!(check_tex_text_escapes(tex), Ok(()));
}

// ============================================================================
// R2-S03-01: OneSiteCompetitionLigandDepletion includes NS * FreeHot (Swillens 1995)
// ============================================================================

#[test]
fn r2_s03_01_depletion_competition_includes_ns_free_hot_grad_and_joint_fit() {
    let model = get_builtin_model("one_site_competition_ligand_depletion").expect("model");
    assert_eq!(model.param_names(), &["Rt", "HotNM", "KdNM", "LogKi", "NS"]);

    // Params: [Rt=5.0, HotNM=2.0, KdNM=1.0, LogKi=-8.0, NS=0.25]
    let params = [5.0_f64, 2.0, 1.0, -8.0, 0.25];

    // 1. At saturating competitor (X = -1 -> 0.1 M >> Ki = 10 nM), free receptor R -> 0,
    //    so total bound converges to nonspecific binding of total radioligand:
    //    HotNM * NS / (1 + NS) = 2.0 * 0.25 / 1.25 = 0.4 nM.
    let y_high_comp = model.eval(&[-1.0], &params, 0).expect("high competitor eval");
    assert!((y_high_comp - 0.4).abs() < 1e-5, "expected Swillens total-bound plateau 0.4 nM, got {y_high_comp}");
    // Even at X = -4 (100 uM, Ki = 10 nM -> C/Ki = 10^4), bound matches R_sp + NS*FreeHot accurately (~0.40064 nM).
    let y_x_neg4 = model.eval(&[-4.0], &params, 0).expect("X=-4 eval");
    assert!(y_x_neg4 > 0.4 && y_x_neg4 < 0.401, "expected Swillens total-bound ~0.40064 nM at X=-4, got {y_x_neg4}");

    // 2. Analytical gradient matches high-precision finite differences across regimes.
    for test_params in [[5.0_f64, 2.0, 1.0, -8.0, 0.25], [12.0, 0.5, 0.8, -7.2, 0.0], [0.0, 2.0, 1.0, -8.0, 0.15]] {
        for &x in &[-11.0, -9.0, -8.0, -7.0, -5.0] {
            let mut analytic = [0.0_f64; 5];
            assert!(model.grad(&[x], &test_params, 0, &mut analytic));
            let y0 = model.eval(&[x], &test_params, 0).unwrap();
            for (k, &ak) in analytic.iter().enumerate() {
                let h = 1e-6_f64 * test_params[k].abs().max(1.0);
                let mut p_plus = test_params;
                p_plus[k] += h;
                let fd = if test_params[k] == 0.0 {
                    // At physical boundary (Rt=0 or NS=0), p - h < 0 is outside domain; use one-sided forward FD.
                    (model.eval(&[x], &p_plus, 0).unwrap() - y0) / h
                } else {
                    let mut p_minus = test_params;
                    p_minus[k] -= h;
                    (model.eval(&[x], &p_plus, 0).unwrap() - model.eval(&[x], &p_minus, 0).unwrap()) / (2.0 * h)
                };
                assert!((ak - fd).abs() < 1e-5 * fd.abs().max(1.0), "grad[{k}] mismatch at x={x}, params={test_params:?}: analytic={ak}, fd={fd}");
            }
        }
    }

    // 3. Because total bound = HotNM * (R + NS * KdNM) / (KdNM * (1 + NS) + R),
    //    NS and KdNM are no longer algebraically collinear (unlike specific-only NS * R/KdNM),
    //    so fitting noiseless synthetic data with ONLY HotNM fixed recovers all 4 parameters
    //    (Rt, KdNM, LogKi, NS) simultaneously!
    let xs: Vec<f64> = (-24..=-8).map(|k| 0.5 * f64::from(k)).collect();
    let points: Vec<DataPoint> = xs.iter().map(|&x| DataPoint::xy(x, model.eval(&[x], &params, 0).unwrap())).collect();
    let ds = vec![DatasetSeries {
        data_set_index: 0,
        label: "Comp".to_owned(),
        points,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }];
    let cfg = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: "HotNM".to_owned(),
            scope: ParameterScope::Constant(2.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(model.as_ref(), &ds, &HookContext::default(), &cfg).expect("joint fit");
    assert_eq!(fit.status, FitStatus::Converged, "joint depletion fit should converge");
    let got = &fit.datasets[0].params;
    assert!((got[0] - 5.0).abs() < 1e-4, "Rt={}", got[0]);
    assert!((got[2] - 1.0).abs() < 1e-3, "KdNM={}", got[2]);
    assert!((got[3] - (-8.0)).abs() < 1e-4, "LogKi={}", got[3]);
    assert!((got[4] - 0.25).abs() < 1e-4, "NS={}", got[4]);

    // 4. Symbolic and substituted TeX equations match the total-bound Swillens formula.
    let sym = format_model_equation_tex("one_site_competition_ligand_depletion", None, &[], false, 4);
    assert!(sym.contains("\\text{NS} \\cdot K_{d,\\text{nM}}") && sym.contains("1 + \\text{NS}"), "unexpected symbolic TeX: {sym}");
    let sub_params: Vec<(String, f64)> = model.param_names().iter().cloned().zip(params).collect();
    let sub = format_model_equation_tex("one_site_competition_ligand_depletion", None, &sub_params, true, 4);
    assert!(sub.contains("0.25"), "substituted TeX missing NS=0.25: {sub}");
}

// ============================================================================
// R2-S03-02: Iterative DSL TeX parser, renderer, and arena Drop never overflow stack
// ============================================================================

#[test]
fn r2_s03_02_deeply_nested_and_long_dsl_equations_never_overflow_stack() {
    let handle = std::thread::Builder::new()
        .name("r2-s03-02-small-stack".to_owned())
        .stack_size(256 * 1024)
        .spawn(|| {
            // 20,000 nested parentheses exceeds depth cap cleanly -> safe fallback without stack overflow.
            let deep_parens = format!("Y = {}X{}", "(".repeat(20_000), ")".repeat(20_000));
            let r_parens = format_model_equation_tex("custom_dsl", Some(&deep_parens), &[], false, 4);
            assert_valid_tex_text_escapes(&r_parens);
            assert!(r_parens.starts_with("Y = "));

            // 20,000 chained unary negations exceeds depth cap -> safe fallback without stack overflow.
            let deep_unary = format!("Y = {}X", "-".repeat(20_000));
            let r_unary = format_model_equation_tex("custom_dsl", Some(&deep_unary), &[], false, 4);
            assert_valid_tex_text_escapes(&r_unary);
            assert!(r_unary.starts_with("Y = "));

            // 2,000-term flat sum forms a 2,000-deep left-skewed Add tree; iterative AST/renderer/Drop handles it directly.
            let long_sum = format!("Y = {}", vec!["X"; 2_000].join(" + "));
            let r_sum = format_model_equation_tex("custom_dsl", Some(&long_sum), &[], false, 4);
            assert!(!r_sum.contains("f_{"), "2000-term sum should render cleanly, not fallback");
            assert_eq!(r_sum.matches(" + X").count(), 1_999);

            // 2,000-factor product inside numerator should render cleanly without stack overflow.
            let long_prod = format!("Y = ({}) / 2", vec!["X"; 2_000].join(" * "));
            let r_prod = format_model_equation_tex("custom_dsl", Some(&long_prod), &[], false, 4);
            assert!(!r_prod.contains("f_{"), "2000-factor product should render cleanly, not fallback");
            assert!(r_prod.contains("\\frac{") && r_prod.ends_with("}{2}"));

            // Moderate deep nesting within depth cap (120 levels < MAX_DSL_TEX_DEPTH = 128) renders cleanly.
            let nested_ok = format!("Y = {}X + A{}", "(".repeat(120), ")".repeat(120));
            let r_ok = format_model_equation_tex("custom_dsl", Some(&nested_ok), &[("A".to_owned(), 7.0)], true, 4);
            assert_eq!(r_ok, "Y = X + 7");
        })
        .expect("spawn small stack thread");
    handle.join().expect("thread must not overflow stack or panic");
}

// ============================================================================
// R2-S03-03: AUC integrates full data domain [X min, X max] despite narrowed/disjoint plot bounds
// ============================================================================

#[test]
fn r2_s03_03_narrowed_and_disjoint_plot_bounds_integrate_full_data_domain() {
    // 1. Constant Y = 2.0 across X in [0, 10] -> exact AUC = 20.0 regardless of plot_x_min/max.
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    xy.data_sets[0].title = "Const".to_owned();
    let mut sheet = DataSheet::new("ds-const", "Const", TableArchetype::Xy(xy)).unwrap();
    for (r, &x) in [0.0, 2.5, 5.0, 7.5, 10.0].iter().enumerate() {
        sheet.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
        sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(2.0)).unwrap();
    }

    let run_const = |pmin: Option<f64>, pmax: Option<f64>| {
        let mut nl = NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("custom_dsl") };
        nl.custom_dsl = Some("Y = A".to_owned());
        nl.plot_x_min = pmin;
        nl.plot_x_max = pmax;
        let spec = AnalysisSpec::new("fit-c", "ds-const", AnalysisKind::NonlinearRegression(nl)).unwrap();
        execute_analysis(&spec, &sheet, &HookContext::default()).unwrap()
    };

    for (label, pmin, pmax) in [("full", None, None), ("narrowed [5, 10]", Some(5.0), Some(10.0)), ("disjoint [20, 30]", Some(20.0), Some(30.0))] {
        let res = run_const(pmin, pmax);
        let auc_fit = res.lookup_scalar("AUC (fitted curve)", Some(0)).unwrap_or_else(|| panic!("{label}: missing AUC (fitted curve)"));
        let auc_emp = res.lookup_scalar("AUC (trapezoidal)", Some(0)).unwrap_or_else(|| panic!("{label}: missing AUC (trapezoidal)"));
        assert!((auc_fit - 20.0).abs() < 1e-10, "{label}: expected fitted AUC 20.0, got {auc_fit}");
        assert!((auc_emp - 20.0).abs() < 1e-12, "{label}: expected emp AUC 20.0, got {auc_emp}");
    }

    // 2. Non-constant Y = 2*X + 1 across X in [0, 10] -> integral from 0 to 10 is [X^2 + X]_0^10 = 110.0.
    let mut xy2 = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    xy2.data_sets[0].title = "Line".to_owned();
    let mut sheet2 = DataSheet::new("ds-line", "Line", TableArchetype::Xy(xy2)).unwrap();
    for (r, &x) in [0.0, 2.5, 5.0, 7.5, 10.0].iter().enumerate() {
        sheet2.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
        sheet2.set_cell(CellCoord::new(r, 0, 0), Cell::number(2.0 * x + 1.0)).unwrap();
    }
    let mut nl_narrow = NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("straight_line") };
    nl_narrow.plot_x_min = Some(5.0);
    nl_narrow.plot_x_max = Some(8.0);
    let spec_line = AnalysisSpec::new("fit-l", "ds-line", AnalysisKind::NonlinearRegression(nl_narrow)).unwrap();
    let res_line = execute_analysis(&spec_line, &sheet2, &HookContext::default()).unwrap();
    let auc_line = res_line.lookup_scalar("AUC (fitted curve)", Some(0)).expect("line AUC");
    assert!((auc_line - 110.0).abs() < 1e-10, "expected full-domain AUC 110.0, got {auc_line}");
}

// ============================================================================
// R2-S03-04: Multi-predictor MultipleVariables fits skip 1D trapezoidal/curve AUC
// ============================================================================

#[test]
fn r2_s03_04_multi_predictor_fits_omit_1d_auc_while_1d_mv_keeps_it() {
    // 1. 2D factorial grid: Y = 3.0 * X1 + 2.0 * X2 (two predictors in mv_x_columns).
    let pairs = [(0.0, 0.0), (0.0, 1.0), (0.0, 2.0), (1.0, 0.0), (1.0, 1.0), (1.0, 2.0), (2.0, 0.0), (2.0, 1.0), (2.0, 2.0)];
    let mut mv = MultipleVariablesTable::new(3, pairs.len()).unwrap();
    mv.variables[0].title = "X1".to_owned();
    mv.variables[1].title = "X2".to_owned();
    mv.variables[2].title = "Response".to_owned();
    for (r, &(x1, x2)) in pairs.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(x1);
        mv.variables[1].cells[r] = Cell::number(x2);
        mv.variables[2].cells[r] = Cell::number(3.0 * x1 + 2.0 * x2);
    }
    let sheet_mv = DataSheet::new("mv-2d", "MV 2D", TableArchetype::MultipleVariables(mv)).unwrap();

    let mut nl_2d = NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("custom_dsl") };
    nl_2d.custom_dsl = Some("Y = A * X1 + B * X2".to_owned());
    nl_2d.mv_x_columns = vec![0, 1];
    nl_2d.mv_y_column = Some(2);
    let spec_2d = AnalysisSpec::new("fit-mv-2d", "mv-2d", AnalysisKind::NonlinearRegression(nl_2d)).unwrap();
    let res_2d = execute_analysis(&spec_2d, &sheet_mv, &HookContext::default()).unwrap();
    assert_eq!(res_2d.status, ResultStatus::UpToDate);
    assert!((res_2d.lookup_scalar("A", Some(0)).expect("A") - 3.0).abs() < 1e-6);
    assert!((res_2d.lookup_scalar("B", Some(0)).expect("B") - 2.0).abs() < 1e-6);
    assert!(res_2d.scalars.iter().all(|s| !s.key.contains("AUC")), "2D predictor fit must emit zero AUC scalars: {:?}", res_2d.scalars);
    assert!(res_2d.tabs.iter().all(|t| t.id != "fit_auc"), "2D predictor fit must emit zero fit_auc tab");

    // 2. Same table analyzed with a SINGLE predictor (`mv_x_columns = [0]` for X1 -> Response, rows with X2 = 0
    //    or dedicated 1D MV table) still emits AUC normally.
    let mut mv_1d = MultipleVariablesTable::new(2, 4).unwrap();
    mv_1d.variables[0].title = "X1".to_owned();
    mv_1d.variables[1].title = "Response".to_owned();
    for (r, &x) in [0.0, 1.0, 2.0, 3.0].iter().enumerate() {
        mv_1d.variables[0].cells[r] = Cell::number(x);
        mv_1d.variables[1].cells[r] = Cell::number(2.0 * x + 1.0);
    }
    let sheet_1d = DataSheet::new("mv-1d", "MV 1D", TableArchetype::MultipleVariables(mv_1d)).unwrap();
    let mut nl_1d = NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("straight_line") };
    nl_1d.mv_x_columns = vec![0];
    nl_1d.mv_y_column = Some(1);
    let spec_1d = AnalysisSpec::new("fit-mv-1d", "mv-1d", AnalysisKind::NonlinearRegression(nl_1d)).unwrap();
    let res_1d = execute_analysis(&spec_1d, &sheet_1d, &HookContext::default()).unwrap();
    let auc_1d = res_1d.lookup_scalar("AUC (fitted curve)", Some(0)).expect("1D MV AUC");
    assert!((auc_1d - 12.0).abs() < 1e-10, "expected AUC 12.0 on [0, 3], got {auc_1d}");
    assert!(res_1d.tabs.iter().any(|t| t.id == "fit_auc"));
}

// ============================================================================
// R2-S03-05: Built-in curve TeX formulas match `library/curves.rs`
// ============================================================================

#[test]
fn r2_s03_05_builtin_curve_tex_matches_evaluator_formulas() {
    // 1. lorentzian: Amplitude / (1 + ((X - Center) / (0.5 * Width))^2)
    let lor = get_builtin_model("lorentzian").expect("lorentzian");
    assert_eq!(lor.param_names(), &["Amplitude", "Center", "Width"]);
    // At X = Center + 0.5 * Width = 7, y must be Amplitude / 2 = 5.0 (FWHM definition).
    assert!((lor.eval(&[7.0], &[10.0, 5.0, 4.0], 0).unwrap() - 5.0).abs() < 1e-12);
    let lor_sym = format_model_equation_tex("lorentzian", None, &[], false, 4);
    assert!(lor_sym.contains("0.5 \\cdot \\text{Width}") || lor_sym.contains("0.5 \\cdot Width"), "lorentzian symbolic TeX: {lor_sym}");
    let lor_sub = format_model_equation_tex(
        "lorentzian",
        None,
        &[("Amplitude".to_owned(), 10.0), ("Center".to_owned(), 5.0), ("Width".to_owned(), 4.0)],
        true,
        4,
    );
    assert!(lor_sub.contains("0.5 \\cdot 4"), "lorentzian substituted TeX must include 0.5 * 4 FWHM half-width: {lor_sub}");

    // 2. logarithmic_curve: a * ln(X) + b (param order & positions must NOT be swapped!)
    let log_m = get_builtin_model("logarithmic_curve").expect("logarithmic_curve");
    assert_eq!(log_m.param_names(), &["a", "b"]);
    assert!((log_m.eval(&[std::f64::consts::E], &[3.0, 10.0], 0).unwrap() - 13.0).abs() < 1e-12);
    let log_sym = format_model_equation_tex("logarithmic_curve", None, &[], false, 4);
    assert_eq!(log_sym, "Y = a \\cdot \\ln(X) + b");
    let log_sub = format_model_equation_tex("logarithmic_curve", None, &[("a".to_owned(), 3.0), ("b".to_owned(), 10.0)], true, 4);
    assert_eq!(log_sub, "Y = 3 \\cdot \\ln(X) + 10");

    // 3. probit_line: 100 * Phi(Slope * X + Intercept)
    let probit = get_builtin_model("probit_line").expect("probit_line");
    assert_eq!(probit.param_names(), &["Slope", "Intercept"]);
    assert!((probit.eval(&[0.0], &[2.0, 0.0], 0).unwrap() - 50.0).abs() < 1e-12);
    let probit_sym = format_model_equation_tex("probit_line", None, &[], false, 4);
    assert!(probit_sym.starts_with("Y = 100 \\cdot \\Phi("), "probit_line symbolic TeX: {probit_sym}");
    let probit_sub = format_model_equation_tex("probit_line", None, &[("Slope".to_owned(), 2.0), ("Intercept".to_owned(), -1.0)], true, 4);
    assert!(probit_sub.starts_with("Y = 100 \\cdot \\Phi(2 \\cdot X - 1)"), "probit_line substituted TeX: {probit_sub}");

    // 4. power_series and power_law: a * X^b (NO + c)
    for id in ["power_series", "power_law"] {
        let pm = get_builtin_model(id).expect(id);
        assert_eq!(pm.param_names(), &["a", "b"]);
        assert!((pm.eval(&[3.0], &[5.0, 2.0], 0).unwrap() - 45.0).abs() < 1e-12);
        assert_eq!(format_model_equation_tex(id, None, &[], false, 4), "Y = a \\cdot X^{b}");
        assert_eq!(format_model_equation_tex(id, None, &[("a".to_owned(), 5.0), ("b".to_owned(), 2.0)], true, 4), "Y = 5 \\cdot X^{2}");
    }
}

// ============================================================================
// R2-S03-06: Operator precedence / parenthesization for scientific notation & negated products
// ============================================================================

#[test]
fn r2_s03_06_substituted_precedence_parenthesizes_sci_divisors_powers_and_negated_products() {
    // sinc_wave with negative or scientific-notation Width
    let sinc_neg = format_model_equation_tex(
        "sinc_wave",
        None,
        &[("Amplitude".to_owned(), 2.0), ("Center".to_owned(), 1.0), ("Width".to_owned(), -2.0)],
        true,
        4,
    );
    assert!(sinc_neg.contains("/ (-2)"), "negative sinc_wave Width must be parenthesized: {sinc_neg}");

    let sinc_sci = format_model_equation_tex(
        "sinc_wave",
        None,
        &[("Amplitude".to_owned(), 2.0), ("Center".to_owned(), 1.0), ("Width".to_owned(), 1e-5)],
        true,
        4,
    );
    assert!(sinc_sci.contains("(1 \\times 10^{-5})"), "scientific sinc_wave Width must be parenthesized: {sinc_sci}");

    // Scientific-notation value in fraction divisor (`\frac`), power base (`^`, `pow`), and product
    let frac_div_sci = format_model_equation_tex("custom_dsl", Some("Y = 10^(X / Kd)"), &[("Kd".to_owned(), 1e-5)], true, 4);
    assert_eq!(frac_div_sci, "Y = 10^{\\frac{X}{1 \\times 10^{-5}}}");

    let pow_fn_sci = format_model_equation_tex("custom_dsl", Some("Y = pow(A, 2)"), &[("A".to_owned(), 1e-5)], true, 4);
    assert_eq!(pow_fn_sci, "Y = (1 \\times 10^{-5})^{2}");

    let pow_sci = format_model_equation_tex("custom_dsl", Some("Y = A^2"), &[("A".to_owned(), 1e-5)], true, 4);
    assert!(pow_sci.contains("(1 \\times 10^{-5})^{2}"), "sci power base must be parenthesized: {pow_sci}");

    let mul_sci = format_model_equation_tex("custom_dsl", Some("Y = 2 * A * X"), &[("A".to_owned(), 1e-6)], true, 4);
    assert!(mul_sci.contains("(1 \\times 10^{-6})"), "sci factor in product must be parenthesized: {mul_sci}");

    // Negated product under outer multiplication/power preserves parentheses for both positive & negative inner scalar
    let neg_prod = format_model_equation_tex("custom_dsl", Some("Y = A * -(B * X)"), &[("A".to_owned(), 3.0), ("B".to_owned(), 2.0)], true, 4);
    assert_eq!(neg_prod, "Y = 3 \\cdot (-2 \\cdot X)");

    let double_neg_prod =
        format_model_equation_tex("custom_dsl", Some("Y = A * -(B * X)"), &[("A".to_owned(), 3.0), ("B".to_owned(), -2.0)], true, 4);
    assert_eq!(double_neg_prod, "Y = 3 \\cdot (2 \\cdot X)");

    let neg_prod_pow = format_model_equation_tex("custom_dsl", Some("Y = (-(B * X))^2"), &[("B".to_owned(), -2.0)], true, 4);
    assert_eq!(neg_prod_pow, "Y = (2 \\cdot X)^{2}");

    // Negated Add/Sub numerator inside Div must preserve sign as `-(a + b) / c`, NOT flip inner negative literal!
    let neg_sum_div = format_model_equation_tex(
        "custom_dsl",
        Some("Y = -(A + B) / C"),
        &[("A".to_owned(), -2.0), ("B".to_owned(), 3.0), ("C".to_owned(), 4.0)],
        true,
        4,
    );
    assert!(neg_sum_div.contains("-\\frac{-2 + 3}{4}"), "negated sum in Div numerator must remain `-\\frac{{-2 + 3}}{{4}}`, got: {neg_sum_div}");
}

// ============================================================================
// R2-S03-07: Custom DSL implicit equations, multi-line locals, and dataset prefixes
// ============================================================================

#[test]
fn r2_s03_07_custom_dsl_implicit_locals_and_dataset_prefixes() {
    // 1. Implicit `LHS = RHS` preserves both sides and substitutes parameters cleanly.
    let imp = "Y + a * sin(Y) = X";
    CompiledDslModel::compile("imp-check", imp).expect("DSL compiles");
    let imp_sub = format_model_equation_tex("custom_dsl", Some(imp), &[("a".to_owned(), 0.5)], true, 4);
    assert_eq!(imp_sub, "Y + 0.5 \\cdot \\sin(Y) = X");

    // 2. Bare implicit expression (no `=`) formats as `expr = 0` and substitutes cleanly.
    let bare = "Y^2 + a * Y - X";
    CompiledDslModel::compile("bare-check", bare).expect("DSL compiles");
    let bare_sub = format_model_equation_tex("custom_dsl", Some(bare), &[("a".to_owned(), 2.0)], true, 4);
    assert_eq!(bare_sub, "Y^{2} + 2 \\cdot Y - X = 0");

    // 3. Multi-statement model with local intermediate assignment inlines locals cleanly.
    let multi = "kobs = kon * X + koff\nY = Span * (1 - exp(-kobs * X))";
    CompiledDslModel::compile("multi-check", multi).expect("DSL compiles");
    let multi_sub = format_model_equation_tex(
        "custom_dsl",
        Some(multi),
        &[("kon".to_owned(), 2.0), ("koff".to_owned(), 0.5), ("Span".to_owned(), 100.0)],
        true,
        4,
    );
    assert!(!multi_sub.contains("kobs"), "local intermediate kobs should be inlined: {multi_sub}");
    assert!(multi_sub.contains("100") && multi_sub.contains("2 \\cdot X + 0.5"), "substituted locals missing: {multi_sub}");

    // 4. Exponential-growth local DAG exceeds inlining cap cleanly and falls back to helper defs.
    let mut dag = String::from("L0 = X + A\n");
    for i in 1..=16 {
        dag.push_str(&format!("L{i} = L{prev} + L{prev}\n", prev = i - 1));
    }
    dag.push_str("Y = L16");
    let dag_tex = format_model_equation_tex("custom_dsl", Some(&dag), &[("A".to_owned(), 3.0)], true, 4);
    assert!(
        dag_tex.contains("\\text{L16};") && dag_tex.contains("\\text{L0} = X + 3") && dag_tex.len() < 4096,
        "oversized DAG should fall back to bounded helper defs: {dag_tex}"
    );
    assert_valid_tex_text_escapes(&dag_tex);

    // 5. Dataset-prefixed rules filter appropriately for DS0 (`format_model_equation_tex` & `Some(0)`) vs DS1 (`Some(1)`).
    let prefixed = "<A> I = 0\n<~A> I = 5\n<A> Y = Top * X + I\n<B:D> Y = Bottom + Top * X + I";
    CompiledDslModel::compile("prefix-check", prefixed).expect("DSL compiles");
    let params = [("Top".to_owned(), 3.0), ("Bottom".to_owned(), 10.0)];
    let ds0 = format_model_equation_tex("custom_dsl", Some(prefixed), &params, true, 4);
    let ds0_explicit = format_model_equation_tex_for_dataset("custom_dsl", Some(prefixed), &params, Some(0), true, 4);
    let ds1 = format_model_equation_tex_for_dataset("custom_dsl", Some(prefixed), &params, Some(1), true, 4);
    assert_eq!(ds0, ds0_explicit);
    assert!(ds0.contains("3 \\cdot X + 0") && !ds0.contains("10"), "DS0 wrong: {ds0}");
    assert!(ds1.contains("10 + 3 \\cdot X + 5"), "DS1 wrong: {ds1}");
}

// ============================================================================
// R2-S03-08: Escape metacharacters inside `tex_ident` / `\text{...}`
// ============================================================================

#[test]
fn r2_s03_08_tex_ident_escapes_metacharacters_and_preserves_subscripts() {
    // Valid multi-segment / Greek identifiers still produce clean subscripts.
    assert_eq!(
        format_model_equation_tex("custom_dsl", Some("Y = k_on * X + V_max_1"), &[], false, 4),
        "Y = k_{\\text{on}} \\cdot X + V_{\\text{max,1}}"
    );
    assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = tau_fast * alpha_1"), &[], false, 4), "Y = \\tau_{\\text{fast}} \\cdot \\alpha_{1}");

    // Hostile variable names & function names inside expressions never produce unbalanced braces or unescaped metacharacters.
    for expr in [
        "Y = _lead * X",
        "Y = trail_ * X",
        "Y = a__b * X",
        "Y = * + _",
        "Y = my_custom_func(X)",
        "Y = A_1_2 * exp(-k_off_2 * X)",
        "foo_1_2 = A_1 * X + a*b",
    ] {
        let tex = format_model_equation_tex("custom_dsl", Some(expr), &[], false, 4);
        assert_valid_tex_text_escapes(&tex);
    }

    // Hostile identifiers in unknown equation fallback and unparsed DSL fallback are escaped cleanly too.
    for eq in ["bad_eq*name^{1}$&%#<>", "a\\b_c{d}"] {
        let tex = format_model_equation_tex(eq, None, &[], false, 4);
        assert_valid_tex_text_escapes(&tex);
    }
    let bad_dsl = "not a valid {dsl} * ^ _ equation";
    let bad_dsl_tex = format_model_equation_tex("custom_dsl", Some(bad_dsl), &[], false, 4);
    assert_valid_tex_text_escapes(&bad_dsl_tex);

    // Subscript containing `}` must be escaped as `\}` and normalize_tex_signs must keep braces balanced.
    let braced_lhs = "Y_a\\}b + A = X";
    let tex_braced = format_model_equation_tex("custom_dsl", Some(braced_lhs), &[("A".to_owned(), -2.0)], true, 4);
    assert_valid_tex_text_escapes(&tex_braced);
}

// ============================================================================
// R2-S03-09: Per-dataset fixed constants in TeX & empty linear parameter guard
// ============================================================================

#[test]
fn r2_s03_09_per_dataset_constants_and_empty_linear_fallback() {
    // 1. Empty or missing-Slope linear/polynomial parameters fall back to symbolic, never `Y = 1*X + 0`.
    assert_eq!(format_polynomial_equation_tex(1, false, &[], true, 4), "Y = \\text{Slope} \\cdot X + \\text{Intercept}");
    assert_eq!(format_polynomial_equation_tex(1, true, &[], true, 4), "Y = \\text{Slope} \\cdot X");
    assert_eq!(format_polynomial_equation_tex(2, false, &[], true, 4), "Y = b_0 + b_1 \\cdot X + b_2 \\cdot X^2");
    assert_eq!(format_polynomial_equation_tex(1, false, &[("YIntercept".to_owned(), 3.0)], true, 4), "Y = \\text{Slope} \\cdot X + 3");

    // 2. End-to-end multi-dataset `ec50_shift_gaddum_schild` with ColumnTitleConstant `AntagConc`
    //    emits per-dataset scalars so each dataset's equation substitutes its AntagConc!
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 7).unwrap();
    xy.data_sets[0].title = "0".to_owned();
    xy.data_sets[1].title = "1e-7".to_owned();
    let gaddum = get_builtin_model("ec50_shift_gaddum_schild").expect("gaddum_schild");
    let gaddum_true = [0.0_f64, 100.0, -7.0, 1.0, 8.0, 1.0];
    let mut sheet = DataSheet::new("ds-gaddum", "Gaddum", TableArchetype::Xy(xy)).unwrap();
    for (r, &x) in [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0].iter().enumerate() {
        sheet.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
        let y0 =
            gaddum.eval(&[x], &[gaddum_true[0], gaddum_true[1], gaddum_true[2], gaddum_true[3], gaddum_true[4], gaddum_true[5], 0.0], 0).unwrap();
        let y1 =
            gaddum.eval(&[x], &[gaddum_true[0], gaddum_true[1], gaddum_true[2], gaddum_true[3], gaddum_true[4], gaddum_true[5], 1e-7], 1).unwrap();
        sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(y0)).unwrap();
        sheet.set_cell(CellCoord::new(r, 1, 0), Cell::number(y1)).unwrap();
    }
    let mut nl_g = NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("ec50_shift_gaddum_schild") };
    nl_g.parameter_rules = vec![ParameterRule {
        name: "AntagConc".to_owned(),
        scope: ParameterScope::ColumnTitleConstant,
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }];
    let spec_g = AnalysisSpec::new("fit-g", "ds-gaddum", AnalysisKind::NonlinearRegression(nl_g)).unwrap();
    let res_g = execute_analysis(&spec_g, &sheet, &HookContext::default()).unwrap();
    assert_eq!(res_g.lookup_scalar("AntagConc", Some(0)), Some(0.0));
    assert!((res_g.lookup_scalar("AntagConc", Some(1)).expect("DS1 AntagConc") - 1e-7).abs() < 1e-18);

    let eq_g0 = format_model_equation_tex_for_dataset("ec50_shift_gaddum_schild", None, &collect_ds_params(&res_g, 0), Some(0), true, 4);
    let eq_g1 = format_model_equation_tex_for_dataset("ec50_shift_gaddum_schild", None, &collect_ds_params(&res_g, 1), Some(1), true, 4);
    assert!(!eq_g0.contains("AntagConc") && !eq_g1.contains("AntagConc"), "AntagConc remained unsubstituted: ds0={eq_g0}, ds1={eq_g1}");
    assert!(eq_g1.contains("10^{-7}"), "DS1 AntagConc=1e-7 should appear in equation: {eq_g1}");

    // 3. End-to-end multi-dataset `one_site_competition_ligand_depletion` with PerDatasetConstant `HotNM`
    //    AND `[DSk]`-suffixed parameter fallback in `format_model_equation_tex_for_dataset`.
    let dep = get_builtin_model("one_site_competition_ligand_depletion").expect("depletion");
    let mut xy_d = XyTable::new(SubcolumnMode::Single, 2, 7).unwrap();
    xy_d.data_sets[0].title = "Hot1.5".to_owned();
    xy_d.data_sets[1].title = "Hot3.0".to_owned();
    let mut sheet_d = DataSheet::new("ds-dep", "Depletion", TableArchetype::Xy(xy_d)).unwrap();
    for (r, &x) in [-11.0, -10.0, -9.0, -8.0, -7.0, -6.0, -5.0].iter().enumerate() {
        sheet_d.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
        let y0 = dep.eval(&[x], &[5.0, 1.5, 1.0, -8.0, 0.25], 0).unwrap();
        let y1 = dep.eval(&[x], &[5.0, 3.0, 1.0, -8.0, 0.25], 1).unwrap();
        sheet_d.set_cell(CellCoord::new(r, 0, 0), Cell::number(y0)).unwrap();
        sheet_d.set_cell(CellCoord::new(r, 1, 0), Cell::number(y1)).unwrap();
    }
    let mut nl_d = NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("one_site_competition_ligand_depletion") };
    nl_d.parameter_rules = vec![
        ParameterRule {
            name: "HotNM".to_owned(),
            scope: ParameterScope::PerDatasetConstant(vec![(0, 1.5), (1, 3.0)]),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule { name: "KdNM".to_owned(), scope: ParameterScope::Constant(1.0), initial_override: None, lower_bound: None, upper_bound: None },
    ];
    let spec_d = AnalysisSpec::new("fit-d", "ds-dep", AnalysisKind::NonlinearRegression(nl_d)).unwrap();
    let res_d = execute_analysis(&spec_d, &sheet_d, &HookContext::default()).unwrap();
    assert_eq!(res_d.lookup_scalar("HotNM", Some(0)), Some(1.5));
    assert_eq!(res_d.lookup_scalar("HotNM", Some(1)), Some(3.0));
    assert_eq!(res_d.lookup_scalar("KdNM", None), Some(1.0));

    let eq_d0 = format_model_equation_tex_for_dataset("one_site_competition_ligand_depletion", None, &collect_ds_params(&res_d, 0), Some(0), true, 4);
    let eq_d1 = format_model_equation_tex_for_dataset("one_site_competition_ligand_depletion", None, &collect_ds_params(&res_d, 1), Some(1), true, 4);
    assert!(!eq_d0.contains("HotNM") && eq_d0.contains("1.5"), "DS0 HotNM=1.5 missing: {eq_d0}");
    assert!(!eq_d1.contains("HotNM") && eq_d1.contains("3"), "DS1 HotNM=3 missing: {eq_d1}");

    // Also verify `[DSk]`-suffixed keys (from NonlinearFitResult::fixed_constants) resolve per dataset directly.
    let suffixed = vec![
        ("Rt".to_owned(), 5.0),
        ("HotNM [DS1]".to_owned(), 1.5),
        ("HotNM [DS2]".to_owned(), 3.0),
        ("KdNM".to_owned(), 1.0),
        ("LogKi".to_owned(), -8.0),
        ("NS".to_owned(), 0.25),
    ];
    let eq_s0 = format_model_equation_tex_for_dataset("one_site_competition_ligand_depletion", None, &suffixed, Some(0), true, 4);
    let eq_s1 = format_model_equation_tex_for_dataset("one_site_competition_ligand_depletion", None, &suffixed, Some(1), true, 4);
    assert!(!eq_s0.contains("HotNM") && eq_s0.contains("1.5"), "DS0 [DS1] lookup failed: {eq_s0}");
    assert!(!eq_s1.contains("HotNM") && eq_s1.contains("3"), "DS1 [DS2] lookup failed: {eq_s1}");
}
