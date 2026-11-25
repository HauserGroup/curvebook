//! Permanent Round 2 (`R0.19`) audit regression suite for Slice `S03` (`crates/fit/*`).
//! Covers all 9 surviving findings `R2-S03-01` .. `R2-S03-09`.

use std::collections::HashMap;

use curvebook_doc::{NonlinearFitSpec, ResultSheet, ResultStatus};
use curvebook_fit::{FitStatus, format_model_equation_tex, format_model_equation_tex_for_dataset, format_polynomial_equation_tex, get_builtin_model};

use crate::common::{
    assert_grad_matches_fd, cfg_rules, dsl_compile, exec_nl, fit_with, mv_sheet_num, nl_fast, rule_col_title, rule_const, rule_per_ds, series_eval,
    xy_sheet, xy_sheet_single,
};

fn collect_ds_params(result: &ResultSheet, ds_index: usize) -> Vec<(String, f64)> {
    let mut by_key: HashMap<String, f64> = HashMap::new();
    for want in [None, Some(ds_index)] {
        for s in &result.scalars {
            if s.data_set_index == want && s.value.is_finite() {
                by_key.insert(s.key.clone(), s.value);
            }
        }
    }
    by_key.into_iter().collect()
}

fn check_tex_text_escapes(tex: &str) -> Result<(), String> {
    let bytes = tex.as_bytes();
    let (mut i, mut depth) = (0_usize, 0_i32);
    while i < bytes.len() {
        if tex.get(i..).is_some_and(|s| s.starts_with("\\text{")) {
            depth += 1;
            i += "\\text{".len();
            let text_depth = depth;
            while i < bytes.len() && depth >= text_depth {
                match bytes[i] {
                    b'\\' => {
                        let tail = tex.get(i..).unwrap_or("");
                        if let Some(cmd) = ["\\textbackslash{}", "\\textless{}", "\\textgreater{}"].into_iter().find(|p| tail.starts_with(p)) {
                            i += cmd.len();
                            continue;
                        }
                        i += 1;
                        if i >= bytes.len() {
                            return Err(format!("dangling backslash inside \\text{{...}} in `{tex}`"));
                        }
                        if matches!(bytes[i], b'\\' | b'<' | b'>') {
                            return Err(format!("invalid TeX text escape `\\{}` inside \\text{{...}} in `{tex}`", bytes[i] as char));
                        }
                        i += 1;
                    }
                    b'{' => return Err(format!("unescaped `{{` inside \\text{{...}} in `{tex}`")),
                    b'}' if depth == text_depth => {
                        depth -= 1;
                        i += 1;
                        break;
                    }
                    b'}' => return Err(format!("unescaped `}}` inside \\text{{...}} in `{tex}`")),
                    b'_' | b'^' | b'*' | b'$' | b'&' | b'%' | b'#' | b'<' | b'>' => {
                        return Err(format!("unescaped metachar `{}` inside \\text{{...}} in `{tex}`", bytes[i] as char));
                    }
                    _ => i += 1,
                }
            }
            continue;
        }
        match bytes[i] {
            b'\\' => i += if i + 1 < bytes.len() { 2 } else { 1 },
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
    if depth != 0 { Err(format!("unbalanced braces in `{tex}`")) } else { Ok(()) }
}

fn assert_valid_tex_text_escapes(tex: &str) {
    assert_eq!(check_tex_text_escapes(tex), Ok(()));
}

#[test]
fn r2_s03_01_depletion_competition_includes_ns_free_hot_grad_and_joint_fit() {
    let model = get_builtin_model("one_site_competition_ligand_depletion").unwrap();
    assert_eq!(model.param_names(), &["Rt", "HotNM", "KdNM", "LogKi", "NS"]);

    let params = [5.0_f64, 2.0, 1.0, -8.0, 0.25];
    assert!((model.eval(&[-1.0], &params, 0).unwrap() - 0.4).abs() < 1e-5);
    let y_neg4 = model.eval(&[-4.0], &params, 0).unwrap();
    assert!(y_neg4 > 0.4 && y_neg4 < 0.401, "got {y_neg4}");

    for test_params in [[5.0_f64, 2.0, 1.0, -8.0, 0.25], [12.0, 0.5, 0.8, -7.2, 0.0], [0.0, 2.0, 1.0, -8.0, 0.15]] {
        for &x in &[-11.0, -9.0, -8.0, -7.0, -5.0] {
            assert_grad_matches_fd(model.as_ref(), &[x], &test_params, 0, 1e-5);
        }
    }

    let xs: Vec<f64> = (-24..=-8).map(|k| 0.5 * f64::from(k)).collect();
    let ds = [series_eval(model.as_ref(), 0, "Comp", &xs, &params)];
    let fit = fit_with(model.as_ref(), &ds, &cfg_rules(vec![rule_const("HotNM", 2.0)]));
    assert_eq!(fit.status, FitStatus::Converged);
    let got = &fit.datasets[0].params;
    for (idx, exp, tol) in [(0, 5.0, 1e-4), (2, 1.0, 1e-3), (3, -8.0, 1e-4), (4, 0.25, 1e-4)] {
        assert!((got[idx] - exp).abs() < tol, "param[{idx}]={}, exp={exp}", got[idx]);
    }

    let sym = format_model_equation_tex("one_site_competition_ligand_depletion", None, &[], false, 4);
    assert!(sym.contains("\\text{NS} \\cdot K_{d,\\text{nM}}") && sym.contains("1 + \\text{NS}"), "sym={sym}");
    let sub_params: Vec<(String, f64)> = model.param_names().iter().cloned().zip(params).collect();
    assert!(format_model_equation_tex("one_site_competition_ligand_depletion", None, &sub_params, true, 4).contains("0.25"));
}

#[test]
fn r2_s03_02_deeply_nested_and_long_dsl_equations_never_overflow_stack() {
    let handle = std::thread::Builder::new()
        .name("r2-s03-02-small-stack".to_owned())
        .stack_size(256 * 1024)
        .spawn(|| {
            for deep in [format!("Y = {}X{}", "(".repeat(20_000), ")".repeat(20_000)), format!("Y = {}X", "-".repeat(20_000))] {
                let r = format_model_equation_tex("custom_dsl", Some(&deep), &[], false, 4);
                assert_valid_tex_text_escapes(&r);
                assert!(r.starts_with("Y = "));
            }
            let r_sum = format_model_equation_tex("custom_dsl", Some(&format!("Y = {}", vec!["X"; 2_000].join(" + "))), &[], false, 4);
            assert!(!r_sum.contains("f_{") && r_sum.matches(" + X").count() == 1_999);

            let r_prod = format_model_equation_tex("custom_dsl", Some(&format!("Y = ({}) / 2", vec!["X"; 2_000].join(" * "))), &[], false, 4);
            assert!(!r_prod.contains("f_{") && r_prod.contains("\\frac{") && r_prod.ends_with("}{2}"));

            let nested_ok = format!("Y = {}X + A{}", "(".repeat(120), ")".repeat(120));
            assert_eq!(format_model_equation_tex("custom_dsl", Some(&nested_ok), &[("A".to_owned(), 7.0)], true, 4), "Y = X + 7");
        })
        .unwrap();
    handle.join().unwrap();
}

#[test]
fn r2_s03_03_narrowed_and_disjoint_plot_bounds_integrate_full_data_domain() {
    let xs = [0.0, 2.5, 5.0, 7.5, 10.0];
    let sheet = xy_sheet_single("ds-const", "Const", "Const", &xs, &[2.0; 5]);
    for (label, pmin, pmax) in [("full", None, None), ("narrowed", Some(5.0), Some(10.0)), ("disjoint", Some(20.0), Some(30.0))] {
        let res =
            exec_nl(&sheet, NonlinearFitSpec { custom_dsl: Some("Y = A".to_owned()), plot_x_min: pmin, plot_x_max: pmax, ..nl_fast("custom_dsl") });
        assert!((res.lookup_scalar("AUC (fitted curve)", Some(0)).unwrap() - 20.0).abs() < 1e-10, "{label} fit AUC");
        assert!((res.lookup_scalar("AUC (trapezoidal)", Some(0)).unwrap() - 20.0).abs() < 1e-12, "{label} emp AUC");
    }

    let ys2: Vec<f64> = xs.iter().map(|&x| 2.0 * x + 1.0).collect();
    let sheet2 = xy_sheet_single("ds-line", "Line", "Line", &xs, &ys2);
    let res_line = exec_nl(&sheet2, NonlinearFitSpec { plot_x_min: Some(5.0), plot_x_max: Some(8.0), ..nl_fast("straight_line") });
    assert!((res_line.lookup_scalar("AUC (fitted curve)", Some(0)).unwrap() - 110.0).abs() < 1e-10);
}

#[test]
fn r2_s03_04_multi_predictor_fits_omit_1d_auc_while_1d_mv_keeps_it() {
    let pairs = [(0.0, 0.0), (0.0, 1.0), (0.0, 2.0), (1.0, 0.0), (1.0, 1.0), (1.0, 2.0), (2.0, 0.0), (2.0, 1.0), (2.0, 2.0)];
    let x1s: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let x2s: Vec<f64> = pairs.iter().map(|p| p.1).collect();
    let ys: Vec<f64> = pairs.iter().map(|&(a, b)| 3.0 * a + 2.0 * b).collect();
    let sheet_mv = mv_sheet_num("mv-2d", "MV 2D", &[("X1", &x1s), ("X2", &x2s), ("Response", &ys)]);

    let res_2d = exec_nl(
        &sheet_mv,
        NonlinearFitSpec {
            custom_dsl: Some("Y = A * X1 + B * X2".to_owned()),
            mv_x_columns: vec![0, 1],
            mv_y_column: Some(2),
            ..nl_fast("custom_dsl")
        },
    );
    assert_eq!(res_2d.status, ResultStatus::UpToDate);
    assert!((res_2d.lookup_scalar("A", Some(0)).unwrap() - 3.0).abs() < 1e-6);
    assert!((res_2d.lookup_scalar("B", Some(0)).unwrap() - 2.0).abs() < 1e-6);
    assert!(res_2d.scalars.iter().all(|s| !s.key.contains("AUC")));
    assert!(res_2d.tabs.iter().all(|t| t.id != "fit_auc"));

    let sheet_1d = mv_sheet_num("mv-1d", "MV 1D", &[("X1", &[0.0, 1.0, 2.0, 3.0]), ("Response", &[1.0, 3.0, 5.0, 7.0])]);
    let res_1d = exec_nl(&sheet_1d, NonlinearFitSpec { mv_x_columns: vec![0], mv_y_column: Some(1), ..nl_fast("straight_line") });
    assert!((res_1d.lookup_scalar("AUC (fitted curve)", Some(0)).unwrap() - 12.0).abs() < 1e-10);
    assert!(res_1d.tabs.iter().any(|t| t.id == "fit_auc"));
}

#[test]
fn r2_s03_05_builtin_curve_tex_matches_evaluator_formulas() {
    let lor = get_builtin_model("lorentzian").unwrap();
    assert_eq!(lor.param_names(), &["Amplitude", "Center", "Width"]);
    assert!((lor.eval(&[7.0], &[10.0, 5.0, 4.0], 0).unwrap() - 5.0).abs() < 1e-12);
    let lor_sym = format_model_equation_tex("lorentzian", None, &[], false, 4);
    assert!(lor_sym.contains("0.5 \\cdot \\text{Width}") || lor_sym.contains("0.5 \\cdot Width"), "{lor_sym}");
    let lor_sub = format_model_equation_tex(
        "lorentzian",
        None,
        &[("Amplitude".to_owned(), 10.0), ("Center".to_owned(), 5.0), ("Width".to_owned(), 4.0)],
        true,
        4,
    );
    assert!(lor_sub.contains("0.5 \\cdot 4"), "{lor_sub}");

    let log_m = get_builtin_model("logarithmic_curve").unwrap();
    assert_eq!(log_m.param_names(), &["a", "b"]);
    assert!((log_m.eval(&[std::f64::consts::E], &[3.0, 10.0], 0).unwrap() - 13.0).abs() < 1e-12);
    assert_eq!(format_model_equation_tex("logarithmic_curve", None, &[], false, 4), "Y = a \\cdot \\ln(X) + b");
    assert_eq!(
        format_model_equation_tex("logarithmic_curve", None, &[("a".to_owned(), 3.0), ("b".to_owned(), 10.0)], true, 4),
        "Y = 3 \\cdot \\ln(X) + 10"
    );

    let probit = get_builtin_model("probit_line").unwrap();
    assert_eq!(probit.param_names(), &["Slope", "Intercept"]);
    assert!((probit.eval(&[0.0], &[2.0, 0.0], 0).unwrap() - 50.0).abs() < 1e-12);
    assert!(format_model_equation_tex("probit_line", None, &[], false, 4).starts_with("Y = 100 \\cdot \\Phi("));
    assert!(
        format_model_equation_tex("probit_line", None, &[("Slope".to_owned(), 2.0), ("Intercept".to_owned(), -1.0)], true, 4)
            .starts_with("Y = 100 \\cdot \\Phi(2 \\cdot X - 1)")
    );

    for id in ["power_series", "power_law"] {
        let pm = get_builtin_model(id).unwrap();
        assert_eq!(pm.param_names(), &["a", "b"]);
        assert!((pm.eval(&[3.0], &[5.0, 2.0], 0).unwrap() - 45.0).abs() < 1e-12);
        assert_eq!(format_model_equation_tex(id, None, &[], false, 4), "Y = a \\cdot X^{b}");
        assert_eq!(format_model_equation_tex(id, None, &[("a".to_owned(), 5.0), ("b".to_owned(), 2.0)], true, 4), "Y = 5 \\cdot X^{2}");
    }
}

#[test]
fn r2_s03_06_substituted_precedence_parenthesizes_sci_divisors_powers_and_negated_products() {
    let owned = |pairs: &[(&str, f64)]| -> Vec<(String, f64)> { pairs.iter().map(|&(k, v)| (k.to_owned(), v)).collect() };
    for &(eq, dsl, params, expected, exact) in &[
        ("sinc_wave", None, &[("Amplitude", 2.0), ("Center", 1.0), ("Width", -2.0)][..], "/ (-2)", false),
        ("sinc_wave", None, &[("Amplitude", 2.0), ("Center", 1.0), ("Width", 1e-5)][..], "(1 \\times 10^{-5})", false),
        ("custom_dsl", Some("Y = 10^(X / Kd)"), &[("Kd", 1e-5)][..], "Y = 10^{\\frac{X}{1 \\times 10^{-5}}}", true),
        ("custom_dsl", Some("Y = pow(A, 2)"), &[("A", 1e-5)][..], "Y = (1 \\times 10^{-5})^{2}", true),
        ("custom_dsl", Some("Y = A^2"), &[("A", 1e-5)][..], "(1 \\times 10^{-5})^{2}", false),
        ("custom_dsl", Some("Y = 2 * A * X"), &[("A", 1e-6)][..], "(1 \\times 10^{-6})", false),
        ("custom_dsl", Some("Y = A * -(B * X)"), &[("A", 3.0), ("B", 2.0)][..], "Y = 3 \\cdot (-2 \\cdot X)", true),
        ("custom_dsl", Some("Y = A * -(B * X)"), &[("A", 3.0), ("B", -2.0)][..], "Y = 3 \\cdot (2 \\cdot X)", true),
        ("custom_dsl", Some("Y = (-(B * X))^2"), &[("B", -2.0)][..], "Y = (2 \\cdot X)^{2}", true),
        ("custom_dsl", Some("Y = -(A + B) / C"), &[("A", -2.0), ("B", 3.0), ("C", 4.0)][..], "-\\frac{-2 + 3}{4}", false),
    ] {
        let tex = format_model_equation_tex(eq, dsl, &owned(params), true, 4);
        if exact {
            assert_eq!(tex, expected, "eq={eq}, dsl={dsl:?}");
        } else {
            assert!(tex.contains(expected), "eq={eq}, dsl={dsl:?}: missing `{expected}` in `{tex}`");
        }
    }
}

#[test]
fn r2_s03_07_custom_dsl_implicit_locals_and_dataset_prefixes() {
    let imp = "Y + a * sin(Y) = X";
    let _ = dsl_compile(imp);
    assert_eq!(format_model_equation_tex("custom_dsl", Some(imp), &[("a".to_owned(), 0.5)], true, 4), "Y + 0.5 \\cdot \\sin(Y) = X");

    let bare = "Y^2 + a * Y - X";
    let _ = dsl_compile(bare);
    assert_eq!(format_model_equation_tex("custom_dsl", Some(bare), &[("a".to_owned(), 2.0)], true, 4), "Y^{2} + 2 \\cdot Y - X = 0");

    let multi = "kobs = kon * X + koff\nY = Span * (1 - exp(-kobs * X))";
    let _ = dsl_compile(multi);
    let multi_sub = format_model_equation_tex(
        "custom_dsl",
        Some(multi),
        &[("kon".to_owned(), 2.0), ("koff".to_owned(), 0.5), ("Span".to_owned(), 100.0)],
        true,
        4,
    );
    assert!(!multi_sub.contains("kobs") && multi_sub.contains("100") && multi_sub.contains("2 \\cdot X + 0.5"), "{multi_sub}");

    let mut dag = String::from("L0 = X + A\n");
    for i in 1..=16 {
        dag.push_str(&format!("L{i} = L{p} + L{p}\n", p = i - 1));
    }
    dag.push_str("Y = L16");
    let dag_tex = format_model_equation_tex("custom_dsl", Some(&dag), &[("A".to_owned(), 3.0)], true, 4);
    assert!(dag_tex.contains("\\text{L16};") && dag_tex.contains("\\text{L0} = X + 3") && dag_tex.len() < 4096);
    assert_valid_tex_text_escapes(&dag_tex);

    let prefixed = "<A> I = 0\n<~A> I = 5\n<A> Y = Top * X + I\n<B:D> Y = Bottom + Top * X + I";
    let _ = dsl_compile(prefixed);
    let params = [("Top".to_owned(), 3.0), ("Bottom".to_owned(), 10.0)];
    let ds0 = format_model_equation_tex("custom_dsl", Some(prefixed), &params, true, 4);
    assert_eq!(ds0, format_model_equation_tex_for_dataset("custom_dsl", Some(prefixed), &params, Some(0), true, 4));
    assert!(ds0.contains("3 \\cdot X + 0") && !ds0.contains("10"), "ds0={ds0}");
    let ds1 = format_model_equation_tex_for_dataset("custom_dsl", Some(prefixed), &params, Some(1), true, 4);
    assert!(ds1.contains("10 + 3 \\cdot X + 5"), "ds1={ds1}");
}

#[test]
fn r2_s03_08_tex_ident_escapes_metacharacters_and_preserves_subscripts() {
    assert_eq!(
        format_model_equation_tex("custom_dsl", Some("Y = k_on * X + V_max_1"), &[], false, 4),
        "Y = k_{\\text{on}} \\cdot X + V_{\\text{max,1}}"
    );
    assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = tau_fast * alpha_1"), &[], false, 4), "Y = \\tau_{\\text{fast}} \\cdot \\alpha_{1}");
    for expr in [
        "Y = _lead * X",
        "Y = trail_ * X",
        "Y = a__b * X",
        "Y = * + _",
        "Y = my_custom_func(X)",
        "Y = A_1_2 * exp(-k_off_2 * X)",
        "foo_1_2 = A_1 * X + a*b",
        "not a valid {dsl} * ^ _ equation",
    ] {
        assert_valid_tex_text_escapes(&format_model_equation_tex("custom_dsl", Some(expr), &[], false, 4));
    }
    for eq in ["bad_eq*name^{1}$&%#<>", "a\\b_c{d}"] {
        assert_valid_tex_text_escapes(&format_model_equation_tex(eq, None, &[], false, 4));
    }
    assert_valid_tex_text_escapes(&format_model_equation_tex("custom_dsl", Some("Y_a\\}b + A = X"), &[("A".to_owned(), -2.0)], true, 4));
}

#[test]
fn r2_s03_09_per_dataset_constants_and_empty_linear_fallback() {
    assert_eq!(format_polynomial_equation_tex(1, false, &[], true, 4), "Y = \\text{Slope} \\cdot X + \\text{Intercept}");
    assert_eq!(format_polynomial_equation_tex(1, true, &[], true, 4), "Y = \\text{Slope} \\cdot X");
    assert_eq!(format_polynomial_equation_tex(2, false, &[], true, 4), "Y = b_0 + b_1 \\cdot X + b_2 \\cdot X^2");
    assert_eq!(format_polynomial_equation_tex(1, false, &[("YIntercept".to_owned(), 3.0)], true, 4), "Y = \\text{Slope} \\cdot X + 3");

    let gaddum = get_builtin_model("ec50_shift_gaddum_schild").unwrap();
    let xs_g = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0];
    let y_g0: Vec<f64> = xs_g.iter().map(|&x| gaddum.eval(&[x], &[0.0, 100.0, -7.0, 1.0, 8.0, 1.0, 0.0], 0).unwrap()).collect();
    let y_g1: Vec<f64> = xs_g.iter().map(|&x| gaddum.eval(&[x], &[0.0, 100.0, -7.0, 1.0, 8.0, 1.0, 1e-7], 1).unwrap()).collect();
    let sheet_g = xy_sheet("ds-gaddum", "Gaddum", &xs_g, &[("0", &y_g0), ("1e-7", &y_g1)]);
    let res_g = exec_nl(&sheet_g, NonlinearFitSpec { parameter_rules: vec![rule_col_title("AntagConc")], ..nl_fast("ec50_shift_gaddum_schild") });
    assert_eq!(res_g.lookup_scalar("AntagConc", Some(0)), Some(0.0));
    assert!((res_g.lookup_scalar("AntagConc", Some(1)).unwrap() - 1e-7).abs() < 1e-18);
    let eq_g0 = format_model_equation_tex_for_dataset("ec50_shift_gaddum_schild", None, &collect_ds_params(&res_g, 0), Some(0), true, 4);
    let eq_g1 = format_model_equation_tex_for_dataset("ec50_shift_gaddum_schild", None, &collect_ds_params(&res_g, 1), Some(1), true, 4);
    assert!(!eq_g0.contains("AntagConc") && !eq_g1.contains("AntagConc") && eq_g1.contains("10^{-7}"));

    let dep = get_builtin_model("one_site_competition_ligand_depletion").unwrap();
    let xs_d = [-11.0, -10.0, -9.0, -8.0, -7.0, -6.0, -5.0];
    let y_d0: Vec<f64> = xs_d.iter().map(|&x| dep.eval(&[x], &[5.0, 1.5, 1.0, -8.0, 0.25], 0).unwrap()).collect();
    let y_d1: Vec<f64> = xs_d.iter().map(|&x| dep.eval(&[x], &[5.0, 3.0, 1.0, -8.0, 0.25], 1).unwrap()).collect();
    let sheet_d = xy_sheet("ds-dep", "Depletion", &xs_d, &[("Hot1.5", &y_d0), ("Hot3.0", &y_d1)]);
    let res_d = exec_nl(
        &sheet_d,
        NonlinearFitSpec {
            parameter_rules: vec![rule_per_ds("HotNM", &[(0, 1.5), (1, 3.0)]), rule_const("KdNM", 1.0)],
            ..nl_fast("one_site_competition_ligand_depletion")
        },
    );
    assert_eq!(
        (res_d.lookup_scalar("HotNM", Some(0)), res_d.lookup_scalar("HotNM", Some(1)), res_d.lookup_scalar("KdNM", None)),
        (Some(1.5), Some(3.0), Some(1.0))
    );
    let eq_d0 = format_model_equation_tex_for_dataset("one_site_competition_ligand_depletion", None, &collect_ds_params(&res_d, 0), Some(0), true, 4);
    let eq_d1 = format_model_equation_tex_for_dataset("one_site_competition_ligand_depletion", None, &collect_ds_params(&res_d, 1), Some(1), true, 4);
    assert!(!eq_d0.contains("HotNM") && eq_d0.contains("1.5") && !eq_d1.contains("HotNM") && eq_d1.contains("3"));

    let suffixed = [("Rt", 5.0), ("HotNM [DS1]", 1.5), ("HotNM [DS2]", 3.0), ("KdNM", 1.0), ("LogKi", -8.0), ("NS", 0.25)]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect::<Vec<_>>();
    let eq_s0 = format_model_equation_tex_for_dataset("one_site_competition_ligand_depletion", None, &suffixed, Some(0), true, 4);
    let eq_s1 = format_model_equation_tex_for_dataset("one_site_competition_ligand_depletion", None, &suffixed, Some(1), true, 4);
    assert!(!eq_s0.contains("HotNM") && eq_s0.contains("1.5") && !eq_s1.contains("HotNM") && eq_s1.contains("3"));
}
