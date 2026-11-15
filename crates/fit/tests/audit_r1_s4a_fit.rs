use std::time::Instant;

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ChecklistStatus, CiBound, DataSheet, FitMethod, LinearRegressionSpec, ModelComparisonSpec,
    NonlinearFitSpec, ResultCell, SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
};
use lumencraft_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, DslEquationKind, FitError, HookContext, ParameterNote, ParametricModel, SolverConfig, compare_models,
    execute_analysis, extract_datasets_from_sheet, fit_nonlinear, get_builtin_model,
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

#[test]
fn r1_s03_04_implicit_dsl_handles_tiny_and_large_scales_without_fixed_clamps() {
    let model = CompiledDslModel::compile("imp_scale", "Y = A*(X - Y)").unwrap();
    assert_eq!(model.kind(), DslEquationKind::Implicit);

    let small = model.eval(&[5e-14], &[1.0], 0).expect("tiny positive scale should solve");
    assert!((small - 2.5e-14).abs() < 1e-16, "expected ~2.5e-14, got {small}");

    let big = model.eval(&[200_000.0], &[1.0], 0).expect("large positive scale should solve");
    assert!((big - 100_000.0).abs() < 1.0, "expected ~100_000, got {big}");

    let neg_big = CompiledDslModel::compile("imp_neg", "Y + 1e12 = X").unwrap();
    let root_neg = neg_big.eval(&[0.0], &[], 0).expect("large negative root should solve");
    assert!((root_neg - (-1e12)).abs() < 1e2, "expected -1e12, got {root_neg}");

    let neg_tiny = CompiledDslModel::compile("imp_neg_tiny", "Y + 1e-18 = X").unwrap();
    let root_neg_tiny = neg_tiny.eval(&[0.0], &[], 0).expect("tiny negative root should solve");
    assert!((root_neg_tiny - (-1e-18)).abs() < 1e-24, "expected -1e-18, got {root_neg_tiny}");

    let unsolvable = CompiledDslModel::compile("imp_none", "Y * Y + 1 = 0").unwrap();
    assert!(unsolvable.eval(&[0.0], &[], 0).is_none());
}

#[test]
fn r1_s03_05_hypergeometric_functions_converge_or_return_none_strictly() {
    let m1 = CompiledDslModel::compile("h1", "Y = hyp1f1(1, 1, X)").unwrap();
    let val1 = m1.eval(&[300.0], &[], 0).expect("hyp1f1(1, 1, 300) should converge accurately");
    let exp300 = 300.0_f64.exp();
    assert!((val1 - exp300).abs() / exp300 < 1e-6, "val1={val1} exp300={exp300}");

    // Negative z via Kummer's transform avoids catastrophic alternating cancellation.
    let m1_neg = CompiledDslModel::compile("h1_neg", "Y = hyp1f1(1, 2, X)").unwrap();
    let val1_neg = m1_neg.eval(&[-50.0], &[], 0).expect("hyp1f1(1, 2, -50) should converge via Kummer");
    let expected_neg = (1.0 - (-50.0_f64).exp()) / 50.0;
    assert!((val1_neg - expected_neg).abs() / expected_neg < 1e-12, "val1_neg={val1_neg} expected={expected_neg}");

    let m2 = CompiledDslModel::compile("h2", "Y = hyp2f1(1, 1, 2, X)").unwrap();
    let val2 = m2.eval(&[0.999], &[], 0).expect("hyp2f1(1, 1, 2, 0.999) should converge");
    let expected2 = -(1.0 - 0.999_f64).ln() / 0.999;
    assert!((val2 - expected2).abs() / expected2 < 1e-6, "val2={val2} expected={expected2}");

    // Negative z <= -1 via Pfaff transformation.
    let val2_pfaff = m2.eval(&[-2.0], &[], 0).expect("hyp2f1(1, 1, 2, -2) should converge via Pfaff");
    let expected2_pfaff = 3.0_f64.ln() / 2.0;
    assert!((val2_pfaff - expected2_pfaff).abs() / expected2_pfaff < 1e-12, "val2_pfaff={val2_pfaff}");

    // Too close to branch cut or overflowing series must return None instead of a partial sum.
    assert!(m2.eval(&[0.999_999_9], &[], 0).is_none());
    assert!(m2.eval(&[1.0], &[], 0).is_none());
    assert!(m1.eval(&[1_000.0], &[], 0).is_none());
}

#[test]
fn r1_s03_06_robust_cauchy_suppresses_parametric_inference_and_comparisons() {
    let model = CompiledDslModel::compile("exp_decay", "Y = A*exp(-K*X)").unwrap();
    let xs = [0.0, 1.0, 2.0, 4.0, 8.0, 12.0];
    let ys = [100.0, 74.1, 54.9, 30.1, 9.1, 2.7];
    let ds = make_dataset(0, "Decay", &xs, &ys);

    let cfg = SolverConfig {
        method: FitMethod::RobustCauchy,
        compute_bands: true,
        compute_prediction_bands: true,
        compute_profile_ci: true,
        compute_hougaard: true,
        curve_overlay_segments: 32,
        ..SolverConfig::default()
    };

    let fit = fit_nonlinear(&model, std::slice::from_ref(&ds), &HookContext::default(), &cfg).unwrap();
    assert!(fit.parameters.iter().all(|p| !p.se.is_finite()));
    assert!(fit.parameters.iter().all(|p| p.ci_lower == CiBound::Open && p.ci_upper == CiBound::Open));
    assert!(fit.parameters.iter().all(|p| !p.symmetric_ci_lower.is_finite() && !p.symmetric_ci_upper.is_finite()));
    assert!(fit.parameters.iter().all(|p| !p.dependency.is_finite() && !p.hougaard_g1.is_finite()));
    assert!(fit.parameters.iter().all(|p| !p.notes.contains(&ParameterNote::UnboundedCi)));
    assert!(fit.covariance.covariance_matrix.is_empty());
    assert!(fit.covariance.correlation_matrix.is_empty());
    assert!(!fit.rsd.is_finite());
    assert!(fit.aic_c.is_none());
    assert!(fit.bic.is_none());
    assert!(!fit.datasets[0].sy_x.is_finite());

    let overlay = fit.datasets[0].curve_overlay.as_ref().expect("curve overlay still rendered in robust mode");
    assert!(!overlay.points.is_empty());
    assert!(overlay.points.iter().all(|pt| pt.ci_lower.is_none() && pt.ci_upper.is_none()));
    assert!(overlay.points.iter().all(|pt| pt.pred_lower.is_none() && pt.pred_upper.is_none()));

    let alt_model = CompiledDslModel::compile("line", "Y = A + K*X").unwrap();
    for spec in [
        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "line".to_owned() },
        ModelComparisonSpec::Aicc { alternative_equation_id: "line".to_owned() },
        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: "line".to_owned() },
    ] {
        let err = compare_models(&fit, &alt_model, std::slice::from_ref(&ds), &HookContext::default(), &cfg, &spec)
            .expect_err("model comparison must be rejected under RobustCauchy");
        assert!(err.to_string().contains("NotApplicableInRobustMode"), "unexpected error: {err}");
    }

    // End-to-end ResultSheet rendering for RobustCauchy.
    let xy = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
    let mut sheet = DataSheet::new("sheet-robust", "Robust", TableArchetype::Xy(xy)).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        sheet.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
        sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(y)).unwrap();
    }
    let mut nl = NonlinearFitSpec::new("one_phase_decay");
    nl.method = FitMethod::RobustCauchy;
    nl.confidence_bands = true;
    nl.prediction_bands = true;
    nl.emit_covariance_tab = true;
    nl.weighting = WeightingScheme::None;
    let spec = AnalysisSpec::new("fit-robust", "sheet-robust", AnalysisKind::NonlinearRegression(nl)).unwrap();
    let result = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();

    let param_tab = result.tabs.iter().find(|t| t.id == "nonlinear_parameters").unwrap();
    for row in &param_tab.rows {
        assert_eq!(row[1], ResultCell::Text("Not applicable".to_owned()));
        assert_eq!(row[2], ResultCell::Text("Not applicable".to_owned()));
        assert_eq!(row[3], ResultCell::Text("Not applicable".to_owned()));
        assert_eq!(row[4], ResultCell::Text("Not applicable".to_owned()));
    }
    let gof_tab = result.tabs.iter().find(|t| t.id == "nonlinear_gof").unwrap();
    let find_gof = |label: &str| {
        let idx = gof_tab.row_labels.iter().position(|l| l == label).unwrap();
        &gof_tab.rows[idx][0]
    };
    assert_eq!(find_gof("Sy.x"), &ResultCell::Text("Not applicable".to_owned()));
    assert_eq!(find_gof("AICc"), &ResultCell::Text("Not applicable".to_owned()));
    assert_eq!(find_gof("BIC"), &ResultCell::Text("Not applicable".to_owned()));
    assert!(result.tabs.iter().all(|t| t.id != "nonlinear_covariance"));
    assert!(
        result.checklist.iter().any(|c| { c.status == ChecklistStatus::Info && c.diagnostic_metric.as_deref() == Some("NotApplicableInRobustMode") })
    );
}

#[test]
fn r1_s03_07_batch_underdetermined_curves_rejected_and_zero_df_emits_nan_se() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();

    let xs0: Vec<f64> = (0..12).map(|i| -9.0 + (i as f64) * 0.5).collect();
    let ys0: Vec<f64> = xs0.iter().map(|&x| model.eval(&[x], &[10.0, 100.0, -6.0, 1.0], 0).unwrap()).collect();
    let ds0 = make_dataset(0, "Good", &xs0, &ys0);

    // Only 3 points for 4 free parameters -> underdetermined (p = 4).
    let ds1_short = make_dataset(1, "Underdetermined", &[-7.0, -6.0, -5.0], &[20.0, 55.0, 90.0]);

    assert!(fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds1_short), &HookContext::default(), &SolverConfig::default()).is_err());

    let batch_err = fit_nonlinear(model.as_ref(), &[ds0.clone(), ds1_short.clone()], &HookContext::default(), &SolverConfig::default())
        .expect_err("batch fit with underdetermined curve must return Err");
    assert!(matches!(batch_err, FitError::InsufficientData { required: 4, actual: 3 }), "unexpected err: {batch_err:?}");

    let rout_cfg = SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, ..SolverConfig::default() };
    let batch_rout_err = fit_nonlinear(model.as_ref(), &[ds0.clone(), ds1_short], &HookContext::default(), &rout_cfg)
        .expect_err("batch ROUT with underdetermined curve must return Err");
    assert!(matches!(batch_rout_err, FitError::InsufficientData { .. }), "unexpected err: {batch_rout_err:?}");

    // Exact determination (n_d == p_d == 4 => df == 0) is allowed but must emit NaN SE/Sy.x, not 0.0.
    let xs1_exact = [-7.0, -6.5, -5.5, -5.0];
    let ys1_exact: Vec<f64> = xs1_exact.iter().map(|&x| model.eval(&[x], &[10.0, 100.0, -6.0, 1.0], 1).unwrap()).collect();
    let ds1_exact = make_dataset(1, "Exact", &xs1_exact, &ys1_exact);
    let exact_fit = fit_nonlinear(model.as_ref(), &[ds0, ds1_exact], &HookContext::default(), &SolverConfig::default()).unwrap();
    for p in exact_fit.parameters.iter().filter(|p| p.dataset_index == Some(1)) {
        assert!(!p.se.is_finite(), "df=0 parameter {} should not fabricate finite SE {}", p.name, p.se);
        assert_eq!((p.ci_lower, p.ci_upper), (CiBound::Open, CiBound::Open));
    }
    assert!(!exact_fit.datasets[1].sy_x.is_finite(), "df=0 Sy.x must be NaN");
}

#[test]
fn r1_s03_08_y0_assignment_in_non_ode_dsl_is_local_variable() {
    let model = CompiledDslModel::compile("e", "Y0 = 5\nY = Y0*exp(-K*X)").unwrap();
    assert_eq!(model.kind(), DslEquationKind::Explicit);
    assert_eq!(model.param_names(), &["K"]);
    assert!((model.eval(&[0.0], &[0.5], 0).unwrap() - 5.0).abs() < 1e-12);
    assert!((model.eval(&[2.0], &[0.5], 0).unwrap() - 5.0 * (-1.0_f64).exp()).abs() < 1e-12);

    // Without `Y0 = ...` in a non-ODE equation, Y0 remains a normal free parameter.
    let free_y0 = CompiledDslModel::compile("e_free", "Y = Y0*exp(-K*X)").unwrap();
    assert_eq!(free_y0.param_names(), &["Y0", "K"]);

    // True ODE initial condition `Y(0) = ...` without an ODE derivative must be rejected cleanly.
    let err = CompiledDslModel::compile("e_bad_ic", "Y(0) = 5\nY = A*exp(-K*X)").unwrap_err();
    assert!(matches!(err, FitError::DslCompileError { .. }), "unexpected error: {err:?}");

    // Real ODE `Y0 = ...` still acts as the ODE initial condition.
    let ode = CompiledDslModel::compile("ode_ic", "Y0 = 5\ndY/dX = -K*Y").unwrap();
    assert_eq!(ode.kind(), DslEquationKind::OdeFirstOrder);
    assert_eq!(ode.param_names(), &["K"]);
    assert!((ode.eval(&[1.0], &[0.5], 0).unwrap() - 5.0 * (-0.5_f64).exp()).abs() < 1e-7);
}

#[test]
fn r1_s03_09_dsl_compilation_scales_linearly_across_transforms_initials_and_prefixes() {
    let compile_ns = |src: &str| {
        let t0 = Instant::now();
        let m = CompiledDslModel::compile("scale", src).unwrap();
        (m, t0.elapsed().as_nanos() as f64)
    };

    // Chained transforms: each line references the prior local transform.
    let build_transforms = |n: usize| {
        let mut s = String::from("[Equation]\nY = A*X + B\n[Transforms]\nT0 = A + B\n");
        for i in 1..n {
            s.push_str(&format!("T{i} = T{} + 1\n", i - 1));
        }
        s
    };
    let (m1000, t1000) = compile_ns(&build_transforms(1_000));
    let (m4000, t4000) = compile_ns(&build_transforms(4_000));
    assert!(t4000 / t1000.max(1_000_000.0) < 8.0, "transforms 4x scaling too slow: t1000={t1000}ns, t4000={t4000}ns");

    let derived = m4000.derived_transforms(&[2.0, 3.0]);
    assert_eq!(derived.len(), 4_000);
    assert!((derived[3_999].value - 4_004.0).abs() < 1e-12);
    assert_eq!(m1000.derived_transforms(&[2.0, 3.0]).len(), 1_000);

    // Chained [InitialValues] rules.
    let build_inits = |n: usize| {
        let mut s = String::from("[Equation]\nY = A*X + B\n[InitialValues]\nL0 = YMIN\n");
        for i in 1..n {
            s.push_str(&format!("L{i} = L{} + 0.1\n", i - 1));
        }
        s.push_str(&format!("A = L{}\nB = YMAX\n", n - 1));
        s
    };
    let (_, init_t1000) = compile_ns(&build_inits(1_000));
    let (m_init4000, init_t4000) = compile_ns(&build_inits(4_000));
    assert!(init_t4000 / init_t1000.max(1_000_000.0) < 8.0, "init rules 4x scaling too slow: t1000={init_t1000}ns, t4000={init_t4000}ns");

    let pts = vec![DataPoint::xy(0.0, 10.0), DataPoint::xy(1.0, 20.0)];
    let g = m_init4000.initial_guess(&pts);
    assert!((g[0] - (10.0 + 3_999.0 * 0.1)).abs() < 1e-6);
    assert!((g[1] - 20.0).abs() < 1e-12);

    // Dataset selector prefix scaling across 2,000 branches.
    let mut prefix_src = String::from("[Equation]\nY = A*X\n");
    for i in 1..=2_000 {
        prefix_src.push_str(&format!("<{i}> Y = A*X + {i}\n"));
    }
    let (m_pref, _) = compile_ns(&prefix_src);
    assert!((m_pref.eval(&[2.0], &[3.0], 0).unwrap() - 6.0).abs() < 1e-12);
    assert!((m_pref.eval(&[2.0], &[3.0], 1_500).unwrap() - 1_506.0).abs() < 1e-12);

    // Source byte cap rejects oversized inputs cleanly before allocating massive ASTs/programs.
    let huge = format!("Y = A*X\n[Transforms]\n{}", "T = A\n".repeat(200_000));
    let err = CompiledDslModel::compile("huge_cap", &huge).expect_err("oversized DSL source must fail cleanly");
    assert!(matches!(err, FitError::DslCompileError { .. }), "unexpected err: {err:?}");
}

fn excluded_number_cell(v: f64) -> Cell {
    let mut c = Cell::number(v);
    c.excluded = true;
    c
}

#[test]
fn r1_x2_03_excluded_x_rows_never_queue_or_emit_interpolated_unknowns() {
    // 1. Excluded X with non-empty numeric Y (exact audit reproducer pattern) + excluded X with empty Y.
    let xy = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    let mut sheet = DataSheet::new("sheet-x2", "Standard", TableArchetype::Xy(xy)).unwrap();
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let ys = [2.0, 4.0, 6.0, 8.0, 10.0];
    for r in 0..3 {
        sheet.set_x_or_time_cell(r, 0, Cell::number(xs[r])).unwrap();
        sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(ys[r])).unwrap();
    }
    // Row 3: X excluded, Y numeric.
    sheet.set_x_or_time_cell(3, 0, excluded_number_cell(4.0)).unwrap();
    sheet.set_cell(CellCoord::new(3, 0, 0), Cell::number(99.0)).unwrap();
    // Row 4: X excluded, Y empty.
    sheet.set_x_or_time_cell(4, 0, excluded_number_cell(5.0)).unwrap();

    let extracted = extract_datasets_from_sheet(&sheet, &[0]).unwrap();
    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].points.len(), 3);
    assert!(extracted[0].unknowns_y.is_empty(), "excluded X must not produce Y unknowns: {:?}", extracted[0].unknowns_y);
    assert!(extracted[0].unknowns_x.is_empty(), "excluded X must not produce X unknowns: {:?}", extracted[0].unknowns_x);

    let lin = LinearRegressionSpec { interpolate_unknowns: true, ..LinearRegressionSpec::default() };
    let lin_spec = AnalysisSpec::new("lin-x2", "sheet-x2", AnalysisKind::LinearRegression(lin)).unwrap();
    let lin_res = execute_analysis(&lin_spec, &sheet, &HookContext::default()).unwrap();
    assert!(
        lin_res.tabs.iter().all(|t| !t.title.to_ascii_lowercase().contains("interpolat")),
        "linear fit unexpectedly emitted interpolation tab: {:?}",
        lin_res.tabs.iter().map(|t| &t.title).collect::<Vec<_>>()
    );

    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.interpolate_unknowns = true;
    nl.weighting = WeightingScheme::None;
    let nl_spec = AnalysisSpec::new("nl-x2", "sheet-x2", AnalysisKind::NonlinearRegression(nl)).unwrap();
    let nl_res = execute_analysis(&nl_spec, &sheet, &HookContext::default()).unwrap();
    assert!(nl_res.tabs.iter().all(|t| !t.title.to_ascii_lowercase().contains("interpolat")), "nonlinear fit unexpectedly emitted interpolation tab");

    // 2. Real unknown rows (empty X with valid Y, and valid X with empty Y) still interpolate normally,
    // while valid X with all-excluded Y replicates must NOT queue into unknowns_x.
    let xy_rep = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 6).unwrap();
    let mut rep_sheet = DataSheet::new("sheet-rep", "Replicates", TableArchetype::Xy(xy_rep)).unwrap();
    for r in 0..3 {
        rep_sheet.set_x_or_time_cell(r, 0, Cell::number(xs[r])).unwrap();
        rep_sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(ys[r])).unwrap();
        rep_sheet.set_cell(CellCoord::new(r, 0, 1), Cell::number(ys[r])).unwrap();
    }
    // Row 3: blank X, valid Y -> legitimate Y unknown.
    rep_sheet.set_cell(CellCoord::new(3, 0, 0), Cell::number(5.0)).unwrap();
    // Row 4: excluded X, excluded Y -> must NOT interpolate.
    rep_sheet.set_x_or_time_cell(4, 0, excluded_number_cell(9.0)).unwrap();
    rep_sheet.set_cell(CellCoord::new(4, 0, 0), excluded_number_cell(18.0)).unwrap();
    // Row 5: valid X, ALL excluded Y replicates -> must NOT interpolate as missing Y.
    rep_sheet.set_x_or_time_cell(5, 0, Cell::number(6.0)).unwrap();
    rep_sheet.set_cell(CellCoord::new(5, 0, 0), excluded_number_cell(12.0)).unwrap();
    rep_sheet.set_cell(CellCoord::new(5, 0, 1), excluded_number_cell(12.0)).unwrap();

    let rep_extracted = extract_datasets_from_sheet(&rep_sheet, &[0]).unwrap();
    assert_eq!(rep_extracted[0].unknowns_y, vec![(3, 5.0)]);
    assert!(rep_extracted[0].unknowns_x.is_empty(), "all-excluded Y must not become X unknown: {:?}", rep_extracted[0].unknowns_x);
}
