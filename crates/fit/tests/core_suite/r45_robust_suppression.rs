#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, DataSheet, FitMethod, ModelComparisonSpec, NonlinearFitSpec, ParameterRule,
    ParameterScope, ResultCell, ResultSheet, ResultStatus, SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, HookContext, InferenceAvailability, ModelComparisonOutcome, NonlinearFitResult, ParameterNote,
    SolverConfig, compare_models, compare_models_checked, compute_rsdr, execute_analysis, fit_nonlinear, get_builtin_model,
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

fn robust_solver_cfg() -> SolverConfig {
    SolverConfig {
        method: FitMethod::RobustCauchy,
        compute_bands: true,
        compute_prediction_bands: true,
        compute_profile_ci: true,
        compute_hougaard: true,
        compute_lack_of_fit: true,
        compute_runs_test: true,
        compute_normality: true,
        compute_homoscedasticity: true,
        interpolate_unknowns: true,
        curve_overlay_segments: 48,
        ..SolverConfig::default()
    }
}

fn assert_robust_params_and_datasets_suppressed(fit: &NonlinearFitResult) {
    for p in &fit.parameters {
        assert_eq!(p.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
        assert_eq!(p.se_option(), None);
        assert_eq!(p.ci_bounds_option(), None);
        assert_eq!(p.symmetric_ci_option(), None);
        assert_eq!(p.dependency_option(), None);
        assert_eq!(p.hougaard_g1_option(), None);
        assert!(!p.se.is_finite());
        assert_eq!(p.ci_lower, CiBound::Open);
        assert_eq!(p.ci_upper, CiBound::Open);
        assert!(!p.symmetric_ci_lower.is_finite());
        assert!(!p.symmetric_ci_upper.is_finite());
        assert!(!p.dependency.is_finite());
        assert!(!p.hougaard_g1.is_finite());
        assert!(!p.notes.contains(&ParameterNote::Ambiguous));
        assert!(!p.notes.contains(&ParameterNote::UnboundedCi));
    }

    for ds in &fit.datasets {
        assert!(ds.studentized_residuals.is_empty());
        assert!(!ds.sy_x.is_finite());
        if let Some(overlay) = &ds.curve_overlay {
            assert!(!overlay.points.is_empty());
            assert!(overlay.points.iter().all(|pt| pt.y.is_finite()));
            assert!(overlay.points.iter().all(|pt| pt.ci_lower.is_none() && pt.ci_upper.is_none()));
            assert!(overlay.points.iter().all(|pt| pt.pred_lower.is_none() && pt.pred_upper.is_none()));
        }
    }
}

fn assert_robust_fit_suppressed(fit: &NonlinearFitResult) {
    assert_eq!(fit.fit_method, FitMethod::RobustCauchy);
    assert_eq!(fit.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
    assert!(fit.is_robust_cauchy());
    assert_eq!(fit.covariance_if_available().unwrap_err(), InferenceAvailability::NotApplicableInRobustMode);
    assert!(fit.covariance.covariance_matrix.is_empty());
    assert!(fit.covariance.correlation_matrix.is_empty());
    assert_eq!(fit.rsd_option(), None);
    assert_eq!(fit.aic_c_option(), None);
    assert_eq!(fit.bic_option(), None);
    assert!(fit.lack_of_fit.is_none());
    assert_robust_params_and_datasets_suppressed(fit);
}

fn assert_rout_fit_full_inference(fit: &NonlinearFitResult, expect_overlay: bool) {
    assert_eq!(fit.inference_availability, InferenceAvailability::Available);
    assert_eq!(fit.rsdr, None);
    assert!(fit.rsd_option().is_some_and(|v| v.is_finite() && v > 0.0));
    assert!(fit.aic_c_option().is_some_and(f64::is_finite));
    assert!(fit.bic_option().is_some_and(f64::is_finite));

    let cov = fit.covariance_if_available().expect("ROUT Stage 2 covariance must be available");
    assert_eq!(cov.covariance_matrix.len(), fit.k_free);
    assert_eq!(cov.correlation_matrix.len(), fit.k_free);
    for p in &fit.parameters {
        assert_eq!(p.inference_availability, InferenceAvailability::Available);
        assert!(p.se_option().is_some_and(|se| se.is_finite() && se > 0.0));
        let (lo, hi) = p.ci_bounds_option().expect("ROUT Stage 2 CI must be present");
        assert!(matches!(lo, CiBound::Finite(v) if v.is_finite()));
        assert!(matches!(hi, CiBound::Finite(v) if v.is_finite()));
        assert!(p.dependency_option().is_some_and(f64::is_finite));
        assert!(p.hougaard_g1_option().is_some_and(f64::is_finite));
    }

    if expect_overlay {
        let overlay = fit.datasets[0].curve_overlay.as_ref().unwrap();
        assert!(overlay.points.iter().any(|pt| pt.ci_lower.is_some() && pt.ci_upper.is_some()));
        assert!(overlay.points.iter().any(|pt| pt.pred_lower.is_some() && pt.pred_upper.is_some()));
    }
}

fn assert_robust_diagnostics_and_comparison_suppressed(res: &ResultSheet) {
    let res_tab = res.tabs.iter().find(|t| t.id == "nonlinear_residuals").unwrap();
    let stud_col = res_tab.column_headers.iter().position(|h| h == "Studentized Residual").unwrap();
    assert!(res_tab.rows.iter().all(|row| row[stud_col] == ResultCell::Text("Not applicable".to_owned())));

    let interp_tab = res.tabs.iter().find(|t| t.id == "nonlinear_interpolated").unwrap();
    assert_eq!(interp_tab.rows.len(), 2);
    for row in &interp_tab.rows {
        assert!(matches!(row[1], ResultCell::Number { value, .. } if value.is_finite()));
        assert_eq!(row[2], ResultCell::Text("Not applicable".to_owned()));
    }
    assert!(interp_tab.footnotes.iter().any(|f| f.contains("NotApplicableInRobustMode")));

    let comp_tab = res.tabs.iter().find(|t| t.kind == curvebook_doc::ResultTabKind::ModelComparison).unwrap();
    let inf_row = comp_tab.row_labels.iter().position(|l| l == "Inference availability").unwrap();
    assert_eq!(comp_tab.rows[inf_row][0], ResultCell::Text("Not applicable (NotApplicableInRobustMode)".to_owned()));
    assert!(comp_tab.rows.iter().flatten().any(|c| c == &ResultCell::Text("Not applicable".to_owned())));
    assert!(comp_tab.footnotes.iter().any(|f| f.contains("NotApplicableInRobustMode")));

    assert!(res.checklist.iter().any(|c| c.status == ChecklistStatus::Info && c.diagnostic_metric.as_deref() == Some("NotApplicableInRobustMode")));
    assert!(!res.checklist.iter().any(|c| c.assumption.contains("Hougaard")));
    assert!(!res.checklist.iter().any(|c| c.assumption.contains("lack of fit")));

    let overlay = &res.fitted_curves[0];
    assert!(overlay.points.iter().all(|pt| pt.ci_lower.is_none() && pt.ci_upper.is_none()));
    assert!(overlay.points.iter().all(|pt| pt.pred_lower.is_none() && pt.pred_upper.is_none()));
}

fn assert_robust_sheet_suppressed(res: &ResultSheet) {
    assert_eq!(res.status, ResultStatus::UpToDate);

    let param_tab = res.tabs.iter().find(|t| t.id == "nonlinear_parameters").unwrap();
    for row in &param_tab.rows {
        assert!(matches!(row[0], ResultCell::Number { value, .. } if value.is_finite()));
        assert!(row[1..=4].iter().all(|c| c == &ResultCell::Text("Not applicable".to_owned())));
    }
    assert!(param_tab.footnotes.iter().any(|f| f.contains("NotApplicableInRobustMode") && f.contains("RSDR =")));

    let gof_tab = res.tabs.iter().find(|t| t.id == "nonlinear_gof").unwrap();
    let find_gof = |label: &str| &gof_tab.rows[gof_tab.row_labels.iter().position(|l| l == label).unwrap()][0];
    assert_eq!(find_gof("Sy.x"), &ResultCell::Text("Not applicable".to_owned()));
    assert_eq!(find_gof("AICc"), &ResultCell::Text("Not applicable".to_owned()));
    assert_eq!(find_gof("BIC"), &ResultCell::Text("Not applicable".to_owned()));
    assert!(matches!(find_gof("RSDR (Robust SD of residuals)"), ResultCell::Number { value, .. } if value.is_finite()));

    assert!(res.scalars.iter().any(|s| s.key == "RSDR" && s.value.is_finite()));
    assert!(res.scalars.iter().any(|s| s.key == "LogEC50" && s.value.is_finite()));
    assert!(res.tabs.iter().all(|t| t.id != "nonlinear_covariance"));

    assert_robust_diagnostics_and_comparison_suppressed(res);
}

#[test]
fn r45_robust_cauchy_typed_suppression_r1_s03_06_and_preserved_quantities() {
    let decay = CompiledDslModel::compile("exp_decay", "Y = A*exp(-K*X)").unwrap();
    let decay_xs = [0.0, 1.0, 2.0, 4.0, 8.0, 12.0];
    let decay_ys = [100.0, 74.1, 54.9, 30.1, 9.1, 2.7];
    let decay_ds = make_dataset(0, "Decay", &decay_xs, &decay_ys);
    let cfg = robust_solver_cfg();

    let decay_fit = fit_nonlinear(&decay, std::slice::from_ref(&decay_ds), &HookContext::default(), &cfg).unwrap();
    assert_robust_fit_suppressed(&decay_fit);

    let a_est = decay_fit.parameters.iter().find(|p| p.name == "A").unwrap().estimate;
    let k_est = decay_fit.parameters.iter().find(|p| p.name == "K").unwrap().estimate;
    assert!((a_est - 100.0).abs() < 1.0, "A estimate={a_est}");
    assert!((k_est - 0.3).abs() < 0.01, "K estimate={k_est}");

    let abs_res: Vec<f64> = decay_fit.datasets[0].residuals.iter().map(|r| r.abs()).collect();
    let expected_rsdr = compute_rsdr(&abs_res, decay_fit.k_free);
    let rsdr = decay_fit.rsdr.expect("RSDR must be populated in RobustCauchy mode");
    assert!(rsdr.is_finite() && rsdr >= 0.0);
    assert!((rsdr - expected_rsdr).abs() < 1e-12);
    assert_eq!(decay_fit.datasets[0].rsdr, Some(rsdr));

    let hill = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let true_p = [10.0, 100.0, -6.5, 1.0];
    let mut ys: Vec<f64> = xs.iter().map(|&x| hill.eval(&[x], &true_p, 0).unwrap()).collect();
    ys[5] += 400.0;

    let mut ds = make_dataset(0, "DoseResponse", &xs, &ys);
    ds.unknowns_y.push((11, 55.0).into());
    ds.unknowns_y.push((12, 120.0).into());
    ds.unknowns_x.push((13, -6.5));
    ds.unknowns_x.push((14, -2.0));

    let fit = fit_nonlinear(hill.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg).unwrap();
    assert_robust_fit_suppressed(&fit);
    let log_ec50 = fit.parameters.iter().find(|p| p.name == "LogEC50").unwrap().estimate;
    assert!((log_ec50 - (-6.5)).abs() < 0.05, "Robust fit should resist outlier: LogEC50={log_ec50}");
    assert!(
        fit.rsdr.is_some_and(|v| v.is_finite() && (0.0..1e-9).contains(&v)),
        "exact 10/11 inliers + 1 outlier recovers true curve with RSDR near 0, got {:?}",
        fit.rsdr
    );
    assert!((fit.datasets[0].residuals[5] - 400.0).abs() < 0.05, "outlier residual at spike index 5 must remain ~400");

    assert_eq!(fit.interpolated_x.len(), 2);
    assert_eq!(fit.interpolated_y.len(), 2);
    let in_range_x = &fit.interpolated_x[0];
    assert_eq!(in_range_x.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
    assert_eq!(in_range_x.ci_bounds_option(), None);
    assert_eq!(in_range_x.ci_lower, CiBound::Open);
    assert_eq!(in_range_x.ci_upper, CiBound::Open);
    assert!((in_range_x.output_estimate - (-6.5)).abs() < 0.05);

    let in_range_y = &fit.interpolated_y[0];
    assert_eq!(in_range_y.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
    assert_eq!(in_range_y.ci_bounds_option(), None);
    assert!((in_range_y.output_estimate - 55.0).abs() < 1.0);

    let extrap_y = &fit.interpolated_y[1];
    assert_eq!(extrap_y.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
    assert_eq!(extrap_y.ci_bounds_option(), None);
    assert!(extrap_y.note.as_deref().is_some_and(|n| n.contains("Extrapolated above measured standard X range")));

    let exact_ys: Vec<f64> = xs.iter().map(|&x| hill.eval(&[x], &true_p, 0).unwrap()).collect();
    let exact_ds = make_dataset(0, "Exact", &xs, &exact_ys);
    let exact_fit = fit_nonlinear(hill.as_ref(), std::slice::from_ref(&exact_ds), &HookContext::default(), &cfg).unwrap();
    assert_robust_fit_suppressed(&exact_fit);
    assert_eq!(exact_fit.rsdr, Some(0.0));

    let mut noisy_ys = exact_ys;
    for (idx, y) in noisy_ys.iter_mut().enumerate() {
        *y += if idx % 2 == 0 { 0.5 } else { -0.5 };
    }
    noisy_ys[5] += 400.0;
    let noisy_ds = make_dataset(0, "NoisyWithOutlier", &xs, &noisy_ys);
    let noisy_fit = fit_nonlinear(hill.as_ref(), std::slice::from_ref(&noisy_ds), &HookContext::default(), &cfg).unwrap();
    assert_robust_fit_suppressed(&noisy_fit);
    let noisy_ec50 = noisy_fit.parameters.iter().find(|p| p.name == "LogEC50").unwrap().estimate;
    assert!((noisy_ec50 - (-6.5)).abs() < 0.1, "noisy robust fit should resist +400 outlier: LogEC50={noisy_ec50}");
    assert!(
        noisy_fit.rsdr.is_some_and(|v| v.is_finite() && v > 0.2 && v < 2.0),
        "noisy robust fit + outlier must yield positive finite RSDR, got {:?}",
        noisy_fit.rsdr
    );
}

#[test]
fn r45_robust_cauchy_shared_independent_and_plot_function_only_suppress_inference() {
    let model = CompiledDslModel::compile("decay", "Y = A * exp(-K * X)").unwrap();
    let xs = [0.0_f64, 1.0, 2.0, 3.0, 4.0, 5.0];
    let ys1: Vec<f64> = xs.iter().map(|&x| 80.0 * (-0.4 * x).exp()).collect();
    let ys2: Vec<f64> = xs.iter().map(|&x| 50.0 * (-0.4 * x).exp()).collect();
    let ds = vec![make_dataset(0, "Ctrl", &xs, &ys1), make_dataset(1, "Treated", &xs, &ys2)];

    let mut shared_cfg = robust_solver_cfg();
    shared_cfg.parameter_rules = vec![ParameterRule {
        name: "K".to_owned(),
        scope: ParameterScope::SharedGlobal,
        initial_override: Some(0.4),
        lower_bound: None,
        upper_bound: None,
    }];
    let shared_fit = fit_nonlinear(&model, &ds, &HookContext::default(), &shared_cfg).unwrap();
    assert_robust_fit_suppressed(&shared_fit);
    assert!(shared_fit.rsdr.is_some());
    assert!(shared_fit.datasets.iter().all(|d| d.rsdr.is_some()));

    let ind_fit = fit_nonlinear(&model, &ds, &HookContext::default(), &robust_solver_cfg()).unwrap();
    assert_robust_fit_suppressed(&ind_fit);
    assert!(ind_fit.rsdr.is_some());

    let mut plot_cfg = robust_solver_cfg();
    plot_cfg.plot_function_only = true;
    let plot_fit = fit_nonlinear(&model, &ds, &HookContext::default(), &plot_cfg).unwrap();
    assert_robust_fit_suppressed(&plot_fit);
}

#[test]
fn r45_compare_models_suppresses_whenever_either_model_uses_robust_cauchy() {
    let model = CompiledDslModel::compile("exp_decay", "Y = A*exp(-K*X)").unwrap();
    let alt_model = CompiledDslModel::compile("line", "Y = A + K*X").unwrap();
    let xs = [0.0, 1.0, 2.0, 4.0, 8.0, 12.0];
    let ys = [100.0, 74.1, 54.9, 30.1, 9.1, 2.7];
    let ds = vec![make_dataset(0, "Decay", &xs, &ys)];

    let robust_cfg = robust_solver_cfg();
    let ls_cfg = SolverConfig { method: FitMethod::LeastSquares, curve_overlay_segments: 16, ..SolverConfig::default() };

    let robust_primary = fit_nonlinear(&model, &ds, &HookContext::default(), &robust_cfg).unwrap();
    let ls_primary = fit_nonlinear(&model, &ds, &HookContext::default(), &ls_cfg).unwrap();

    let specs = [
        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "line".to_owned() },
        ModelComparisonSpec::Aicc { alternative_equation_id: "line".to_owned() },
        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: "line".to_owned() },
    ];

    for (primary, comp_cfg, case_name) in [
        (&robust_primary, &robust_cfg, "both robust"),
        (&robust_primary, &ls_cfg, "primary robust / alt LS"),
        (&ls_primary, &robust_cfg, "primary LS / alt robust"),
    ] {
        for spec in &specs {
            let Err(err) = compare_models(primary, &alt_model, &ds, &HookContext::default(), comp_cfg, spec) else {
                panic!("{case_name}: compare_models must reject");
            };
            assert!(err.to_string().contains("NotApplicableInRobustMode"), "{case_name}: {err}");

            let checked = compare_models_checked(primary, &alt_model, &ds, &HookContext::default(), comp_cfg, spec)
                .unwrap_or_else(|e| panic!("{case_name}: compare_models_checked must return typed status: {e}"));
            assert_eq!(checked.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
            assert!(!checked.is_applicable());
            assert_eq!(checked.outcome.inference_availability(), InferenceAvailability::NotApplicableInRobustMode);
            assert!(matches!(checked.outcome, ModelComparisonOutcome::NotApplicableInRobustMode { .. }));
        }
    }
}

#[test]
fn r45_rout_stage2_preserves_full_least_squares_inference() {
    let hill = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let true_p = [10.0, 100.0, -6.5, 1.0];
    let mut ys1: Vec<f64> = xs.iter().map(|&x| hill.eval(&[x], &true_p, 0).unwrap() + 0.05).collect();
    for (i, y) in ys1.iter_mut().enumerate() {
        *y += if i % 2 == 0 { 0.2 } else { -0.2 };
    }
    ys1[2] += 350.0;

    let mut ds1 = make_dataset(0, "Dose1", &xs, &ys1);
    ds1.unknowns_y.push((20, 55.0).into());

    let rout_cfg = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        compute_bands: true,
        compute_prediction_bands: true,
        compute_profile_ci: true,
        compute_hougaard: true,
        compute_lack_of_fit: true,
        interpolate_unknowns: true,
        curve_overlay_segments: 32,
        ..SolverConfig::default()
    };

    let fit = fit_nonlinear(hill.as_ref(), std::slice::from_ref(&ds1), &HookContext::default(), &rout_cfg).unwrap();
    assert_eq!(fit.fit_method, FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 });
    assert!(fit.n_outliers_excluded >= 1, "expected ROUT to exclude injected outlier");
    assert!(fit.rout.is_some());
    assert_rout_fit_full_inference(&fit, true);

    assert_eq!(fit.interpolated_x.len(), 1);
    let interp = &fit.interpolated_x[0];
    assert_eq!(interp.inference_availability, InferenceAvailability::Available);
    let (ilo, ihi) = interp.ci_bounds_option().expect("ROUT Stage 2 interpolated CI must be present");
    assert!(matches!(ilo, CiBound::Finite(v) if v.is_finite()));
    assert!(matches!(ihi, CiBound::Finite(v) if v.is_finite()));

    let alt_model = get_builtin_model("log_agonist_vs_response_3pl").unwrap();
    let comp = compare_models_checked(
        &fit,
        alt_model.as_ref(),
        std::slice::from_ref(&ds1),
        &HookContext::default(),
        &rout_cfg,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "log_agonist_vs_response_3pl".to_owned() },
    )
    .unwrap();
    assert_eq!(comp.inference_availability, InferenceAvailability::Available);
    assert!(comp.is_applicable());
    assert!(matches!(comp.outcome, ModelComparisonOutcome::ExtraSumOfSquaresF(_)));

    let mut ys2: Vec<f64> = xs.iter().map(|&x| hill.eval(&[x], &[5.0, 90.0, -6.0, 1.2], 0).unwrap() + 0.05).collect();
    for (i, y) in ys2.iter_mut().enumerate() {
        *y += if i % 2 == 0 { 0.15 } else { -0.15 };
    }
    let ds2 = make_dataset(1, "Dose2", &xs, &ys2);
    let multi_rout = fit_nonlinear(hill.as_ref(), &[ds1, ds2], &HookContext::default(), &rout_cfg).unwrap();
    assert_rout_fit_full_inference(&multi_rout, false);
}

#[test]
fn r45_result_sheet_rendering_robust_vs_rout_all_tabs() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let true_p = [10.0, 100.0, -6.5, 1.0];
    let mut ys: Vec<f64> = xs.iter().enumerate().map(|(i, &x)| model.eval(&[x], &true_p, 0).unwrap() + if i % 2 == 0 { 0.2 } else { -0.2 }).collect();
    ys[2] += 250.0;

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, xs.len() + 2).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    xy.data_sets[0].cells[xs.len()][0] = Cell::number(55.0);
    xy.set_x_cell(xs.len() + 1, Cell::number(-6.5)).unwrap();

    let sheet = DataSheet::new("sheet-r45", "Assay", TableArchetype::Xy(xy)).unwrap();

    for comp_spec in [
        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "log_agonist_vs_response_3pl".to_owned() },
        ModelComparisonSpec::Aicc { alternative_equation_id: "log_agonist_vs_response_3pl".to_owned() },
        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: "log_agonist_vs_response_3pl".to_owned() },
    ] {
        let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
        nl.method = FitMethod::RobustCauchy;
        nl.confidence_bands = true;
        nl.prediction_bands = true;
        nl.emit_covariance_tab = true;
        nl.emit_residuals_tab = true;
        nl.interpolate_unknowns = true;
        nl.replicates_lack_of_fit_test = true;
        nl.weighting = WeightingScheme::None;
        nl.comparison = Some(comp_spec);

        let spec = AnalysisSpec::new("fit-robust", "sheet-r45", AnalysisKind::NonlinearRegression(nl)).unwrap();
        let res = execute_analysis(&spec, &sheet, &HookContext::default()).expect("RobustCauchy ResultSheet with comparison must succeed");
        assert_robust_sheet_suppressed(&res);
    }

    let mut nl_rout = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_rout.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl_rout.show_standard_errors = Some(true);
    nl_rout.confidence_bands = true;
    nl_rout.prediction_bands = true;
    nl_rout.emit_covariance_tab = true;
    nl_rout.emit_residuals_tab = true;
    nl_rout.interpolate_unknowns = true;
    nl_rout.weighting = WeightingScheme::None;
    nl_rout.comparison = Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "log_agonist_vs_response_3pl".to_owned() });

    let rout_spec = AnalysisSpec::new("fit-rout", "sheet-r45", AnalysisKind::NonlinearRegression(nl_rout)).unwrap();
    let rout_res = execute_analysis(&rout_spec, &sheet, &HookContext::default()).unwrap();
    assert_eq!(rout_res.status, ResultStatus::UpToDate);
    assert!(rout_res.tabs.iter().any(|t| t.id == "nonlinear_covariance"));
    let rout_params = rout_res.tabs.iter().find(|t| t.id == "nonlinear_parameters").unwrap();
    for row in &rout_params.rows {
        assert!(matches!(row[1], ResultCell::Number { value, .. } if value.is_finite()));
        assert!(matches!(row[2], ResultCell::ConfidenceInterval { lower: CiBound::Finite(_), upper: CiBound::Finite(_) }));
    }
    let rout_interp = rout_res.tabs.iter().find(|t| t.id == "nonlinear_interpolated").unwrap();
    assert!(
        rout_interp.rows.iter().all(|row| matches!(row[2], ResultCell::ConfidenceInterval { lower: CiBound::Finite(_), upper: CiBound::Finite(_) }))
    );
    let rout_comp = rout_res.tabs.iter().find(|t| t.id == "model_comparison_f").unwrap();
    let f_row = rout_comp.row_labels.iter().position(|l| l == "F (DFn, DFd)").unwrap();
    assert!(matches!(&rout_comp.rows[f_row][0], ResultCell::Text(s) if !s.contains("Not applicable")));
    assert!(!rout_res.scalars.iter().any(|s| s.key == "RSDR"));
}
