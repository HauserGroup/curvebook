use curvebook_doc::{
    Cell, CellCoord, CiBound, DataSheet, FitMethod, LinearRegressionSpec, ModelComparisonSpec, NonlinearFitSpec, ResultCell, SubcolumnMode,
    TableArchetype, WeightingScheme, XyTable,
};
use curvebook_fit::{
    DataPoint, DslEquationKind, FitError, HookContext, ParameterNote, ParametricModel, SolverConfig, UnknownResponse, compare_models,
    extract_datasets_from_sheet, fit_nonlinear, get_builtin_model,
};

use crate::common::{
    assert_dsl_compile_err, dsl_compile, exec_lin, exec_nl, find_tab, fit_default, fit_with, series_eval, series_xy, xy_sheet_single,
};

#[test]
fn r1_s03_04_implicit_dsl_handles_tiny_and_large_scales_without_fixed_clamps() {
    let model = dsl_compile("Y = A*(X - Y)");
    assert_eq!(model.kind(), DslEquationKind::Implicit);
    for &(src, x, p, exp, tol) in &[
        ("Y = A*(X - Y)", 5e-14, &[1.0][..], 2.5e-14, 1e-16),
        ("Y = A*(X - Y)", 200_000.0, &[1.0][..], 100_000.0, 1.0),
        ("Y + 1e12 = X", 0.0, &[][..], -1e12, 1e2),
        ("Y + 1e-18 = X", 0.0, &[][..], -1e-18, 1e-24),
    ] {
        let got = dsl_compile(src).eval(&[x], p, 0).unwrap();
        assert!((got - exp).abs() < tol, "{src} at {x}: got {got}, exp {exp}");
    }
    assert!(dsl_compile("Y * Y + 1 = 0").eval(&[0.0], &[], 0).is_none());
}

#[test]
fn r1_s03_05_hypergeometric_functions_converge_or_return_none_strictly() {
    let m1 = dsl_compile("Y = hyp1f1(1, 1, X)");
    let exp300 = 300.0_f64.exp();
    assert!((m1.eval(&[300.0], &[], 0).unwrap() - exp300).abs() / exp300 < 1e-6);

    let m1_neg = dsl_compile("Y = hyp1f1(1, 2, X)");
    let exp_neg = (1.0 - (-50.0_f64).exp()) / 50.0;
    assert!((m1_neg.eval(&[-50.0], &[], 0).unwrap() - exp_neg).abs() / exp_neg < 1e-12);

    let m2 = dsl_compile("Y = hyp2f1(1, 1, 2, X)");
    let exp2 = -(1.0 - 0.999_f64).ln() / 0.999;
    assert!((m2.eval(&[0.999], &[], 0).unwrap() - exp2).abs() / exp2 < 1e-6);
    let exp2_p = 3.0_f64.ln() / 2.0;
    assert!((m2.eval(&[-2.0], &[], 0).unwrap() - exp2_p).abs() / exp2_p < 1e-12);

    assert!(m2.eval(&[0.999_999_9], &[], 0).is_none());
    assert!(m2.eval(&[1.0], &[], 0).is_none());
    assert!(m1.eval(&[1_000.0], &[], 0).is_none());
}

#[test]
fn r1_s03_07_batch_underdetermined_curves_rejected_and_zero_df_emits_nan_se() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs0: Vec<f64> = (0..12).map(|i| -9.0 + (i as f64) * 0.5).collect();
    let ds0 = series_eval(model.as_ref(), 0, "Good", &xs0, &[10.0, 100.0, -6.0, 1.0]);
    let ds1_short = series_xy(1, "Underdetermined", &[-7.0, -6.0, -5.0], &[20.0, 55.0, 90.0]);

    let hooks = HookContext::default();
    assert!(fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds1_short), &hooks, &SolverConfig::default()).is_err());
    let batch_err = fit_nonlinear(model.as_ref(), &[ds0.clone(), ds1_short.clone()], &hooks, &SolverConfig::default()).unwrap_err();
    assert!(matches!(batch_err, FitError::InsufficientData { required: 4, actual: 3 }), "err={batch_err:?}");

    let rout_cfg = SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, ..SolverConfig::default() };
    assert!(matches!(fit_nonlinear(model.as_ref(), &[ds0.clone(), ds1_short], &hooks, &rout_cfg), Err(FitError::InsufficientData { .. })));

    let ds1_exact = series_eval(model.as_ref(), 1, "Exact", &[-7.0, -6.5, -5.5, -5.0], &[10.0, 100.0, -6.0, 1.0]);
    let exact_fit = fit_default(model.as_ref(), &[ds0, ds1_exact]);
    for p in exact_fit.parameters.iter().filter(|p| p.dataset_index == Some(1)) {
        assert!(!p.se.is_finite(), "df=0 {} had finite SE {}", p.name, p.se);
        assert_eq!((p.ci_lower, p.ci_upper), (CiBound::Open, CiBound::Open));
    }
    assert!(!exact_fit.datasets[1].sy_x.is_finite());
}

#[test]
fn r1_s03_08_y0_assignment_in_non_ode_dsl_is_local_variable() {
    let model = dsl_compile("Y0 = 5\nY = Y0*exp(-K*X)");
    assert_eq!(model.kind(), DslEquationKind::Explicit);
    assert_eq!(model.param_names(), &["K"]);
    assert!((model.eval(&[0.0], &[0.5], 0).unwrap() - 5.0).abs() < 1e-12);
    assert!((model.eval(&[2.0], &[0.5], 0).unwrap() - 5.0 * (-1.0_f64).exp()).abs() < 1e-12);

    assert_eq!(dsl_compile("Y = Y0*exp(-K*X)").param_names(), &["Y0", "K"]);
    assert_dsl_compile_err("Y(0) = 5\nY = A*exp(-K*X)");

    let ode = dsl_compile("Y0 = 5\ndY/dX = -K*Y");
    assert_eq!(ode.kind(), DslEquationKind::OdeFirstOrder);
    assert_eq!(ode.param_names(), &["K"]);
    assert!((ode.eval(&[1.0], &[0.5], 0).unwrap() - 5.0 * (-0.5_f64).exp()).abs() < 1e-7);
}

#[test]
fn r1_s03_09_dsl_compilation_scales_linearly_across_transforms_initials_and_prefixes() {
    let build_transforms = |n: usize| {
        let mut s = String::from("[Equation]\nY = A*X + B\n[Transforms]\nT0 = A + B\n");
        for i in 1..n {
            s.push_str(&format!("T{i} = T{} + 1\n", i - 1));
        }
        s
    };
    let m1000 = dsl_compile(&build_transforms(1_000));
    let m4000 = dsl_compile(&build_transforms(4_000));
    let derived = m4000.derived_transforms(&[2.0, 3.0]);
    assert_eq!((m1000.derived_transforms(&[2.0, 3.0]).len(), derived.len()), (1_000, 4_000));
    assert!((derived[3_999].value - 4_004.0).abs() < 1e-12);

    let build_inits = |n: usize| {
        let mut s = String::from("[Equation]\nY = A*X + B\n[InitialValues]\nL0 = YMIN\n");
        for i in 1..n {
            s.push_str(&format!("L{i} = L{} + 0.1\n", i - 1));
        }
        s.push_str(&format!("A = L{}\nB = YMAX\n", n - 1));
        s
    };
    let _ = dsl_compile(&build_inits(1_000));
    let g = dsl_compile(&build_inits(4_000)).initial_guess(&[DataPoint::xy(0.0, 10.0), DataPoint::xy(1.0, 20.0)]);
    assert!((g[0] - (10.0 + 3_999.0 * 0.1)).abs() < 1e-6 && (g[1] - 20.0).abs() < 1e-12);

    let mut prefix_src = String::from("[Equation]\nY = A*X\n");
    for i in 1..=2_000 {
        prefix_src.push_str(&format!("<{i}> Y = A*X + {i}\n"));
    }
    let m_pref = dsl_compile(&prefix_src);
    assert!((m_pref.eval(&[2.0], &[3.0], 0).unwrap() - 6.0).abs() < 1e-12);
    assert!((m_pref.eval(&[2.0], &[3.0], 1_500).unwrap() - 1_506.0).abs() < 1e-12);

    assert_dsl_compile_err(&format!("Y = A*X\n[Transforms]\n{}", "T = A\n".repeat(200_000)));
}

fn excluded_cell(v: f64) -> Cell {
    let mut c = Cell::number(v);
    c.excluded = true;
    c
}

#[test]
fn r1_x2_03_excluded_x_rows_never_queue_or_emit_interpolated_unknowns() {
    let mut sheet = DataSheet::new("sheet-x2", "Standard", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 5).unwrap())).unwrap();
    let (xs, ys) = ([1.0, 2.0, 3.0, 4.0, 5.0], [2.0, 4.0, 6.0, 8.0, 10.0]);
    for r in 0..3 {
        sheet.set_x_or_time_cell(r, 0, Cell::number(xs[r])).unwrap();
        sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(ys[r])).unwrap();
    }
    sheet.set_x_or_time_cell(3, 0, excluded_cell(4.0)).unwrap();
    sheet.set_cell(CellCoord::new(3, 0, 0), Cell::number(99.0)).unwrap();
    sheet.set_x_or_time_cell(4, 0, excluded_cell(5.0)).unwrap();

    let extracted = extract_datasets_from_sheet(&sheet, &[0]).unwrap();
    assert_eq!((extracted.len(), extracted[0].points.len()), (1, 3));
    assert!(extracted[0].unknowns_y.is_empty() && extracted[0].unknowns_x.is_empty());

    let lin_res = exec_lin(&sheet, LinearRegressionSpec { interpolate_unknowns: true, ..Default::default() });
    assert!(lin_res.tabs.iter().all(|t| !t.title.to_ascii_lowercase().contains("interpolat")));
    let nl_res =
        exec_nl(&sheet, NonlinearFitSpec { interpolate_unknowns: true, weighting: WeightingScheme::None, ..NonlinearFitSpec::new("straight_line") });
    assert!(nl_res.tabs.iter().all(|t| !t.title.to_ascii_lowercase().contains("interpolat")));

    let mut rep_sheet =
        DataSheet::new("sheet-rep", "Replicates", TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(2), 1, 6).unwrap())).unwrap();
    for r in 0..3 {
        rep_sheet.set_x_or_time_cell(r, 0, Cell::number(xs[r])).unwrap();
        rep_sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(ys[r])).unwrap();
        rep_sheet.set_cell(CellCoord::new(r, 0, 1), Cell::number(ys[r])).unwrap();
    }
    rep_sheet.set_cell(CellCoord::new(3, 0, 0), Cell::number(5.0)).unwrap();
    rep_sheet.set_x_or_time_cell(4, 0, excluded_cell(9.0)).unwrap();
    rep_sheet.set_cell(CellCoord::new(4, 0, 0), excluded_cell(18.0)).unwrap();
    rep_sheet.set_x_or_time_cell(5, 0, Cell::number(6.0)).unwrap();
    rep_sheet.set_cell(CellCoord::new(5, 0, 0), excluded_cell(12.0)).unwrap();
    rep_sheet.set_cell(CellCoord::new(5, 0, 1), excluded_cell(12.0)).unwrap();

    let rep_ext = extract_datasets_from_sheet(&rep_sheet, &[0]).unwrap();
    assert_eq!(rep_ext[0].unknowns_y, vec![UnknownResponse::Replicate { row: 3, subcol: 0, value: 5.0 }]);
    assert!(rep_ext[0].unknowns_x.is_empty());
}

#[test]
fn r1_s03_06_robust_cauchy_suppresses_parametric_inference_and_comparisons() {
    let model = dsl_compile("Y = A*exp(-K*X)");
    let (xs, ys) = ([0.0, 1.0, 2.0, 4.0, 8.0, 12.0], [100.0, 74.1, 54.9, 30.1, 9.1, 2.7]);
    let ds = series_xy(0, "Decay", &xs, &ys);
    let cfg = SolverConfig {
        method: FitMethod::RobustCauchy,
        compute_bands: true,
        compute_prediction_bands: true,
        compute_profile_ci: true,
        compute_hougaard: true,
        curve_overlay_segments: 32,
        ..SolverConfig::default()
    };
    let fit = fit_with(&model, std::slice::from_ref(&ds), &cfg);
    assert!(fit.parameters.iter().all(|p| !p.se.is_finite()
        && p.ci_lower == CiBound::Open
        && p.ci_upper == CiBound::Open
        && !p.symmetric_ci_lower.is_finite()
        && !p.symmetric_ci_upper.is_finite()
        && !p.dependency.is_finite()
        && !p.hougaard_g1.is_finite()
        && !p.notes.contains(&ParameterNote::UnboundedCi)));
    assert!(fit.covariance.covariance_matrix.is_empty() && fit.covariance.correlation_matrix.is_empty());
    assert!(!fit.rsd.is_finite() && fit.aic_c.is_none() && fit.bic.is_none() && !fit.datasets[0].sy_x.is_finite());
    let overlay = fit.datasets[0].curve_overlay.as_ref().expect("curve overlay rendered");
    assert!(
        !overlay.points.is_empty()
            && overlay.points.iter().all(|pt| pt.ci_lower.is_none() && pt.ci_upper.is_none() && pt.pred_lower.is_none() && pt.pred_upper.is_none())
    );

    let alt = dsl_compile("Y = A + K*X");
    for spec in [
        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "line".to_owned() },
        ModelComparisonSpec::Aicc { alternative_equation_id: "line".to_owned() },
        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: "line".to_owned() },
    ] {
        let err = compare_models(&fit, &alt, std::slice::from_ref(&ds), &HookContext::default(), &cfg, &spec).unwrap_err();
        assert!(err.to_string().contains("NotApplicableInRobustMode"), "unexpected error: {err}");
    }

    let sheet = xy_sheet_single("sheet-robust", "Robust", "A", &xs, &ys);
    let mut nl = NonlinearFitSpec::new("one_phase_decay");
    nl.method = FitMethod::RobustCauchy;
    nl.confidence_bands = true;
    nl.prediction_bands = true;
    nl.emit_covariance_tab = true;
    let result = exec_nl(&sheet, nl);
    let na = ResultCell::Text("Not applicable".to_owned());
    for row in &find_tab(&result, "nonlinear_parameters").rows {
        assert!(row[1..=4].iter().all(|c| *c == na), "expected Not applicable cells, got {row:?}");
    }
    assert!(result.tabs.iter().all(|t| t.id != "nonlinear_covariance"));
}
