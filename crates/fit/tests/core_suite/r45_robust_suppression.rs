#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    Cell, ChecklistStatus, CiBound, DataSheet, FitMethod, ModelComparisonSpec, NonlinearFitSpec, ResultCell, ResultSheet, ResultStatus,
    ResultTabKind, SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
};
use curvebook_fit::{
    HookContext, InferenceAvailability, ModelComparisonOutcome, NonlinearFitResult, ParameterNote, SolverConfig, compare_models,
    compare_models_checked, compute_rsdr, get_builtin_model,
};

use crate::common::{dsl_compile, exec_nl, find_tab, fit_with, param_val, rule_shared_init, series_xy, tab_row};

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

fn assert_robust_fit_suppressed(fit: &NonlinearFitResult) {
    assert_eq!((fit.fit_method, fit.inference_availability), (FitMethod::RobustCauchy, InferenceAvailability::NotApplicableInRobustMode));
    assert!(fit.is_robust_cauchy());
    assert_eq!(fit.covariance_if_available().unwrap_err(), InferenceAvailability::NotApplicableInRobustMode);
    assert!(fit.covariance.covariance_matrix.is_empty() && fit.covariance.correlation_matrix.is_empty());
    assert_eq!((fit.rsd_option(), fit.aic_c_option(), fit.bic_option()), (None, None, None));
    assert!(fit.lack_of_fit.is_none());

    for p in &fit.parameters {
        assert_eq!(p.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
        assert_eq!(
            (p.se_option(), p.ci_bounds_option(), p.symmetric_ci_option(), p.dependency_option(), p.hougaard_g1_option()),
            (None, None, None, None, None)
        );
        assert!(!p.se.is_finite() && p.ci_lower == CiBound::Open && p.ci_upper == CiBound::Open);
        assert!(!p.symmetric_ci_lower.is_finite() && !p.symmetric_ci_upper.is_finite() && !p.dependency.is_finite() && !p.hougaard_g1.is_finite());
        assert!(!p.notes.contains(&ParameterNote::Ambiguous) && !p.notes.contains(&ParameterNote::UnboundedCi));
    }
    for ds in &fit.datasets {
        assert!(ds.studentized_residuals.is_empty() && !ds.sy_x.is_finite());
        if let Some(ov) = &ds.curve_overlay {
            assert!(!ov.points.is_empty());
            assert!(ov.points.iter().all(|pt| pt.y.is_finite()
                && pt.ci_lower.is_none()
                && pt.ci_upper.is_none()
                && pt.pred_lower.is_none()
                && pt.pred_upper.is_none()));
        }
    }
}

fn assert_rout_fit_full_inference(fit: &NonlinearFitResult, expect_overlay: bool) {
    assert_eq!((fit.inference_availability, fit.rsdr), (InferenceAvailability::Available, None));
    assert!(
        fit.rsd_option().is_some_and(|v| v.is_finite() && v > 0.0)
            && fit.aic_c_option().is_some_and(f64::is_finite)
            && fit.bic_option().is_some_and(f64::is_finite)
    );
    let cov = fit.covariance_if_available().unwrap();
    assert_eq!((cov.covariance_matrix.len(), cov.correlation_matrix.len()), (fit.k_free, fit.k_free));
    for p in &fit.parameters {
        assert_eq!(p.inference_availability, InferenceAvailability::Available);
        assert!(p.se_option().is_some_and(|se| se.is_finite() && se > 0.0));
        let (lo, hi) = p.ci_bounds_option().unwrap();
        assert!(matches!((lo, hi), (CiBound::Finite(a), CiBound::Finite(b)) if a.is_finite() && b.is_finite()));
        assert!(p.dependency_option().is_some_and(f64::is_finite) && p.hougaard_g1_option().is_some_and(f64::is_finite));
    }
    if expect_overlay {
        let ov = fit.datasets[0].curve_overlay.as_ref().unwrap();
        assert!(ov.points.iter().any(|pt| pt.ci_lower.is_some() && pt.ci_upper.is_some()));
        assert!(ov.points.iter().any(|pt| pt.pred_lower.is_some() && pt.pred_upper.is_some()));
    }
}

fn assert_robust_sheet_suppressed(res: &ResultSheet) {
    let na = ResultCell::Text("Not applicable".to_owned());
    assert_eq!(res.status, ResultStatus::UpToDate);
    let ptab = find_tab(res, "nonlinear_parameters");
    for row in &ptab.rows {
        assert!(matches!(row[0], ResultCell::Number { value, .. } if value.is_finite()) && row[1..=4].iter().all(|c| c == &na));
    }
    assert!(ptab.footnotes.iter().any(|f| f.contains("NotApplicableInRobustMode") && f.contains("RSDR =")));
    for lbl in ["Sy.x", "AICc", "BIC"] {
        assert_eq!(tab_row(res, "nonlinear_gof", lbl)[0], na);
    }
    assert!(matches!(tab_row(res, "nonlinear_gof", "RSDR (Robust SD of residuals)")[0], ResultCell::Number { value, .. } if value.is_finite()));
    assert!(
        res.scalars.iter().any(|s| s.key == "RSDR" && s.value.is_finite()) && res.scalars.iter().any(|s| s.key == "LogEC50" && s.value.is_finite())
    );
    assert!(res.tabs.iter().all(|t| t.id != "nonlinear_covariance"));

    let res_tab = find_tab(res, "nonlinear_residuals");
    let stud_col = res_tab.column_headers.iter().position(|h| h == "Studentized Residual").unwrap();
    assert!(res_tab.rows.iter().all(|r| r[stud_col] == na));

    let interp_tab = find_tab(res, "nonlinear_interpolated");
    assert_eq!(interp_tab.rows.len(), 2);
    assert!(interp_tab.rows.iter().all(|r| matches!(r[1], ResultCell::Number { value, .. } if value.is_finite()) && r[2] == na));
    assert!(interp_tab.footnotes.iter().any(|f| f.contains("NotApplicableInRobustMode")));

    let comp_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::ModelComparison).unwrap();
    let inf_row = comp_tab.row_labels.iter().position(|l| l == "Inference availability").unwrap();
    assert_eq!(comp_tab.rows[inf_row][0], ResultCell::Text("Not applicable (NotApplicableInRobustMode)".to_owned()));
    assert!(comp_tab.rows.iter().flatten().any(|c| c == &na) && comp_tab.footnotes.iter().any(|f| f.contains("NotApplicableInRobustMode")));
    assert!(res.checklist.iter().any(|c| c.status == ChecklistStatus::Info && c.diagnostic_metric.as_deref() == Some("NotApplicableInRobustMode")));
    assert!(!res.checklist.iter().any(|c| c.assumption.contains("Hougaard") || c.assumption.contains("lack of fit")));
    assert!(
        res.fitted_curves[0]
            .points
            .iter()
            .all(|pt| pt.ci_lower.is_none() && pt.ci_upper.is_none() && pt.pred_lower.is_none() && pt.pred_upper.is_none())
    );
}

#[test]
fn r45_robust_cauchy_typed_suppression_r1_s03_06_and_preserved_quantities() {
    let decay = dsl_compile("Y = A*exp(-K*X)");
    let decay_ds = series_xy(0, "Decay", &[0.0, 1.0, 2.0, 4.0, 8.0, 12.0], &[100.0, 74.1, 54.9, 30.1, 9.1, 2.7]);
    let cfg = robust_solver_cfg();

    let decay_fit = fit_with(&decay, &[decay_ds], &cfg);
    assert_robust_fit_suppressed(&decay_fit);
    assert!((param_val(&decay_fit, "A") - 100.0).abs() < 1.0 && (param_val(&decay_fit, "K") - 0.3).abs() < 0.01);
    let abs_res: Vec<f64> = decay_fit.datasets[0].residuals.iter().map(|r| r.abs()).collect();
    let rsdr = decay_fit.rsdr.unwrap();
    assert!(rsdr.is_finite() && rsdr >= 0.0 && (rsdr - compute_rsdr(&abs_res, decay_fit.k_free)).abs() < 1e-12);
    assert_eq!(decay_fit.datasets[0].rsdr, Some(rsdr));

    let hill = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let exact_ys: Vec<f64> = xs.iter().map(|&x| hill.eval(&[x], &[10.0, 100.0, -6.5, 1.0], 0).unwrap()).collect();
    let mut ys = exact_ys.clone();
    ys[5] += 400.0;

    let mut ds = series_xy(0, "DoseResponse", &xs, &ys);
    ds.unknowns_y.extend([(11, 55.0), (12, 120.0)].into_iter().map(curvebook_fit::UnknownResponse::from));
    ds.unknowns_x.extend([(13, -6.5), (14, -2.0)]);

    let fit = fit_with(hill.as_ref(), &[ds], &cfg);
    assert_robust_fit_suppressed(&fit);
    assert!((param_val(&fit, "LogEC50") - (-6.5)).abs() < 0.05);
    assert!(fit.rsdr.is_some_and(|v| v.is_finite() && (0.0..1e-9).contains(&v)));
    assert!((fit.datasets[0].residuals[5] - 400.0).abs() < 0.05);
    assert_eq!((fit.interpolated_x.len(), fit.interpolated_y.len()), (2, 2));

    let in_x = &fit.interpolated_x[0];
    assert_eq!(
        (in_x.inference_availability, in_x.ci_bounds_option(), in_x.ci_lower, in_x.ci_upper),
        (InferenceAvailability::NotApplicableInRobustMode, None, CiBound::Open, CiBound::Open)
    );
    assert!((in_x.output_estimate - (-6.5)).abs() < 0.05);
    let in_y = &fit.interpolated_y[0];
    assert_eq!((in_y.inference_availability, in_y.ci_bounds_option()), (InferenceAvailability::NotApplicableInRobustMode, None));
    assert!((in_y.output_estimate - 55.0).abs() < 1.0);
    let ex_y = &fit.interpolated_y[1];
    assert_eq!((ex_y.inference_availability, ex_y.ci_bounds_option()), (InferenceAvailability::NotApplicableInRobustMode, None));
    assert!(ex_y.note.as_deref().is_some_and(|n| n.contains("Extrapolated above measured standard X range")));

    let exact_fit = fit_with(hill.as_ref(), &[series_xy(0, "Exact", &xs, &exact_ys)], &cfg);
    assert_robust_fit_suppressed(&exact_fit);
    assert_eq!(exact_fit.rsdr, Some(0.0));

    let mut noisy_ys = exact_ys;
    for (idx, y) in noisy_ys.iter_mut().enumerate() {
        *y += if idx % 2 == 0 { 0.5 } else { -0.5 };
    }
    noisy_ys[5] += 400.0;
    let noisy_fit = fit_with(hill.as_ref(), &[series_xy(0, "NoisyWithOutlier", &xs, &noisy_ys)], &cfg);
    assert_robust_fit_suppressed(&noisy_fit);
    assert!((param_val(&noisy_fit, "LogEC50") - (-6.5)).abs() < 0.1);
    assert!(noisy_fit.rsdr.is_some_and(|v| v.is_finite() && v > 0.2 && v < 2.0));
}

#[test]
fn r45_robust_cauchy_shared_independent_and_plot_function_only_suppress_inference() {
    let model = dsl_compile("Y = A * exp(-K * X)");
    let xs = [0.0_f64, 1.0, 2.0, 3.0, 4.0, 5.0];
    let ds = [
        series_xy(0, "Ctrl", &xs, &xs.iter().map(|&x| 80.0 * (-0.4 * x).exp()).collect::<Vec<_>>()),
        series_xy(1, "Treated", &xs, &xs.iter().map(|&x| 50.0 * (-0.4 * x).exp()).collect::<Vec<_>>()),
    ];
    for (cfg, check_rsdr) in [
        (SolverConfig { parameter_rules: vec![rule_shared_init("K", 0.4)], ..robust_solver_cfg() }, true),
        (robust_solver_cfg(), true),
        (SolverConfig { plot_function_only: true, ..robust_solver_cfg() }, false),
    ] {
        let fit = fit_with(&model, &ds, &cfg);
        assert_robust_fit_suppressed(&fit);
        if check_rsdr {
            assert!(fit.rsdr.is_some() && fit.datasets.iter().all(|d| d.rsdr.is_some()));
        }
    }
}

#[test]
fn r45_compare_models_suppresses_whenever_either_model_uses_robust_cauchy() {
    let (model, alt) = (dsl_compile("Y = A*exp(-K*X)"), dsl_compile("Y = A + K*X"));
    let ds = [series_xy(0, "Decay", &[0.0, 1.0, 2.0, 4.0, 8.0, 12.0], &[100.0, 74.1, 54.9, 30.1, 9.1, 2.7])];
    let robust_cfg = robust_solver_cfg();
    let ls_cfg = SolverConfig { method: FitMethod::LeastSquares, curve_overlay_segments: 16, ..Default::default() };
    let (rob_p, ls_p) = (fit_with(&model, &ds, &robust_cfg), fit_with(&model, &ds, &ls_cfg));

    let specs = [
        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "line".to_owned() },
        ModelComparisonSpec::Aicc { alternative_equation_id: "line".to_owned() },
        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: "line".to_owned() },
    ];
    for (primary, comp_cfg, tag) in [(&rob_p, &robust_cfg, "both"), (&rob_p, &ls_cfg, "prim_rob"), (&ls_p, &robust_cfg, "alt_rob")] {
        for spec in &specs {
            let err = compare_models(primary, &alt, &ds, &HookContext::default(), comp_cfg, spec).unwrap_err();
            assert!(err.to_string().contains("NotApplicableInRobustMode"), "{tag}: {err}");
            let checked = compare_models_checked(primary, &alt, &ds, &HookContext::default(), comp_cfg, spec).unwrap();
            assert_eq!(
                (checked.inference_availability, checked.is_applicable(), checked.outcome.inference_availability()),
                (InferenceAvailability::NotApplicableInRobustMode, false, InferenceAvailability::NotApplicableInRobustMode)
            );
            assert!(matches!(checked.outcome, ModelComparisonOutcome::NotApplicableInRobustMode { .. }));
        }
    }
}

#[test]
fn r45_rout_stage2_preserves_full_least_squares_inference() {
    let hill = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let mut ys1: Vec<f64> = xs
        .iter()
        .enumerate()
        .map(|(i, &x)| hill.eval(&[x], &[10.0, 100.0, -6.5, 1.0], 0).unwrap() + 0.05 + if i % 2 == 0 { 0.2 } else { -0.2 })
        .collect();
    ys1[2] += 350.0;
    let mut ds1 = series_xy(0, "Dose1", &xs, &ys1);
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
    let fit = fit_with(hill.as_ref(), std::slice::from_ref(&ds1), &rout_cfg);
    assert_eq!(fit.fit_method, FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 });
    assert!(fit.n_outliers_excluded >= 1 && fit.rout.is_some());
    assert_rout_fit_full_inference(&fit, true);

    assert_eq!(fit.interpolated_x.len(), 1);
    assert_eq!(fit.interpolated_x[0].inference_availability, InferenceAvailability::Available);
    assert!(matches!(fit.interpolated_x[0].ci_bounds_option(), Some((CiBound::Finite(a), CiBound::Finite(b))) if a.is_finite() && b.is_finite()));

    let alt = get_builtin_model("log_agonist_vs_response_3pl").unwrap();
    let comp = compare_models_checked(
        &fit,
        alt.as_ref(),
        std::slice::from_ref(&ds1),
        &HookContext::default(),
        &rout_cfg,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "log_agonist_vs_response_3pl".to_owned() },
    )
    .unwrap();
    assert_eq!((comp.inference_availability, comp.is_applicable()), (InferenceAvailability::Available, true));
    assert!(matches!(comp.outcome, ModelComparisonOutcome::ExtraSumOfSquaresF(_)));

    let ys2: Vec<f64> = xs
        .iter()
        .enumerate()
        .map(|(i, &x)| hill.eval(&[x], &[5.0, 90.0, -6.0, 1.2], 0).unwrap() + 0.05 + if i % 2 == 0 { 0.15 } else { -0.15 })
        .collect();
    assert_rout_fit_full_inference(&fit_with(hill.as_ref(), &[ds1, series_xy(1, "Dose2", &xs, &ys2)], &rout_cfg), false);
}

#[test]
fn r45_result_sheet_rendering_robust_vs_rout_all_tabs() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let mut ys: Vec<f64> =
        xs.iter().enumerate().map(|(i, &x)| model.eval(&[x], &[10.0, 100.0, -6.5, 1.0], 0).unwrap() + if i % 2 == 0 { 0.2 } else { -0.2 }).collect();
    ys[2] += 250.0;

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, xs.len() + 2).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(&ys).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    xy.data_sets[0].cells[xs.len()][0] = Cell::number(55.0);
    xy.set_x_cell(xs.len() + 1, Cell::number(-6.5)).unwrap();
    let sheet = DataSheet::new("sheet-r45", "Assay", TableArchetype::Xy(xy)).unwrap();

    let alt_id = "log_agonist_vs_response_3pl".to_owned();
    for comp in [
        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: alt_id.clone() },
        ModelComparisonSpec::Aicc { alternative_equation_id: alt_id.clone() },
        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: alt_id.clone() },
    ] {
        let nl = NonlinearFitSpec {
            method: FitMethod::RobustCauchy,
            confidence_bands: true,
            prediction_bands: true,
            emit_covariance_tab: true,
            emit_residuals_tab: true,
            interpolate_unknowns: true,
            replicates_lack_of_fit_test: true,
            weighting: WeightingScheme::None,
            comparison: Some(comp),
            ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
        };
        assert_robust_sheet_suppressed(&exec_nl(&sheet, nl));
    }

    let rout_res = exec_nl(
        &sheet,
        NonlinearFitSpec {
            method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
            show_standard_errors: Some(true),
            confidence_bands: true,
            prediction_bands: true,
            emit_covariance_tab: true,
            emit_residuals_tab: true,
            interpolate_unknowns: true,
            weighting: WeightingScheme::None,
            comparison: Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: alt_id }),
            ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
        },
    );
    assert_eq!(rout_res.status, ResultStatus::UpToDate);
    assert!(rout_res.tabs.iter().any(|t| t.id == "nonlinear_covariance"));
    for row in &find_tab(&rout_res, "nonlinear_parameters").rows {
        assert!(matches!(row[1], ResultCell::Number { value, .. } if value.is_finite()));
        assert!(matches!(row[2], ResultCell::ConfidenceInterval { lower: CiBound::Finite(_), upper: CiBound::Finite(_) }));
    }
    assert!(
        find_tab(&rout_res, "nonlinear_interpolated")
            .rows
            .iter()
            .all(|r| matches!(r[2], ResultCell::ConfidenceInterval { lower: CiBound::Finite(_), upper: CiBound::Finite(_) }))
    );
    assert!(matches!(&tab_row(&rout_res, "model_comparison_f", "F (DFn, DFd)")[0], ResultCell::Text(s) if !s.contains("Not applicable")));
    assert!(!rout_res.scalars.iter().any(|s| s.key == "RSDR"));
}
