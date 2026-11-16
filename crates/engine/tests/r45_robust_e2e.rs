#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, DataSheet, FitMethod, GraphSheet, ModelComparisonSpec, NonlinearFitSpec, ProjectDocument,
    RegressionOverlaySpec, ResultCell, SheetId, SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
};
use curvebook_engine::{CommandOutcome, Session};
use curvebook_fit::{
    DataPoint, DatasetSeries, HookContext, InferenceAvailability, ModelComparisonOutcome, ModelComparisonResult, NonlinearFitResult, SolverConfig,
    compare_models_checked, execute_analysis, fit_nonlinear, get_builtin_model,
};
use serde_json::{Value, json};

fn exec_ok(session: &mut Session, command_id: &str, params: Value) -> CommandOutcome {
    session.execute(command_id, params).unwrap_or_else(|err| panic!("command `{command_id}` failed: {err}"))
}

fn build_seeded_xy_session() -> Session {
    let mut session = Session::new();
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({
            "sheet_id": "xy-dose",
            "title": "Dose Response",
            "archetype": "xy",
            "rows": 14,
            "cols": 1,
            "replicates": 1,
            "auto_graph": false
        }),
    );

    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let true_p = [10.0, 100.0, -6.5, 1.0];
    let mut x_cells = Vec::new();
    let mut y_cells = Vec::new();
    for (r, &x) in xs.iter().enumerate() {
        let mut y = model.eval(&[x], &true_p, 0).unwrap() + if r % 2 == 0 { 0.25 } else { -0.25 };
        if r == 5 {
            y += 300.0; // Outlier at midpoint x=-6.5
        }
        x_cells.push(json!({ "row": r, "value": x }));
        y_cells.push(json!({ "row": r, "col": 0, "subcol": 0, "value": y }));
    }
    // Row 12: blank X, Y=55 -> unknown X interpolation
    y_cells.push(json!({ "row": 12, "col": 0, "subcol": 0, "value": 55.0 }));
    // Row 13: X=-6.5, blank Y -> unknown Y interpolation
    x_cells.push(json!({ "row": 13, "value": -6.5 }));

    exec_ok(&mut session, "table.setXCells", json!({ "sheet_id": "xy-dose", "cells": x_cells }));
    exec_ok(&mut session, "table.setCells", json!({ "sheet_id": "xy-dose", "cells": y_cells }));
    session
}

#[test]
fn r45_versioned_serde_roundtrip_and_legacy_compatibility() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let alt_model = get_builtin_model("log_agonist_vs_response_3pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let ys: Vec<f64> = xs.iter().map(|&x| model.eval(&[x], &[10.0, 100.0, -6.5, 1.0], 0).unwrap()).collect();
    let ds = vec![DatasetSeries {
        data_set_index: 0,
        label: "Dose".to_owned(),
        points: xs.iter().zip(ys.iter()).enumerate().map(|(row, (&x, &y))| DataPoint { row, ..DataPoint::xy(x, y) }).collect(),
        unknowns_y: vec![(11, 55.0)],
        unknowns_x: vec![(12, -6.5)],
        is_pre_summarized: false,
        column_title_value: None,
    }];

    // 1. RobustCauchy result contains NaNs for suppressed LS stats and serializes/deserializes cleanly.
    let robust_cfg = SolverConfig {
        method: FitMethod::RobustCauchy,
        compute_bands: true,
        compute_prediction_bands: true,
        interpolate_unknowns: true,
        curve_overlay_segments: 16,
        ..SolverConfig::default()
    };
    let robust_fit = fit_nonlinear(model.as_ref(), &ds, &HookContext::default(), &robust_cfg).unwrap();
    let json_text = serde_json::to_string(&robust_fit).unwrap();
    let roundtrip: NonlinearFitResult = serde_json::from_str(&json_text).unwrap();
    assert_eq!(roundtrip.fit_method, FitMethod::RobustCauchy);
    assert_eq!(roundtrip.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
    assert_eq!(roundtrip.rsdr, robust_fit.rsdr);
    assert!(roundtrip.parameters.iter().all(|p| p.se_option().is_none() && p.ci_bounds_option().is_none() && !p.se.is_finite()));
    assert!(roundtrip.interpolated_x[0].ci_bounds_option().is_none());
    assert!(roundtrip.interpolated_x[0].output_estimate.is_finite());

    // 2. ModelComparisonResult round-trip in NotApplicableInRobustMode.
    let comp = compare_models_checked(
        &robust_fit,
        alt_model.as_ref(),
        &ds,
        &HookContext::default(),
        &robust_cfg,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "log_agonist_vs_response_3pl".to_owned() },
    )
    .unwrap();
    let comp_json = serde_json::to_string(&comp).unwrap();
    let comp_rt: ModelComparisonResult = serde_json::from_str(&comp_json).unwrap();
    assert_eq!(comp_rt.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
    assert!(!comp_rt.is_applicable());
    assert!(matches!(comp_rt.outcome, ModelComparisonOutcome::NotApplicableInRobustMode { .. }));

    // 3. Legacy LeastSquares JSON missing new fields deserializes cleanly via #[serde(default)].
    let ls_fit = fit_nonlinear(model.as_ref(), &ds, &HookContext::default(), &SolverConfig::default()).unwrap();
    let mut ls_val = serde_json::to_value(&ls_fit).unwrap();
    let obj = ls_val.as_object_mut().unwrap();
    obj.remove("fit_method");
    obj.remove("inference_availability");
    obj.remove("rsdr");
    if let Some(params) = obj.get_mut("parameters").and_then(Value::as_array_mut) {
        for p in params {
            p.as_object_mut().unwrap().remove("inference_availability");
        }
    }
    if let Some(interps) = obj.get_mut("interpolated_x").and_then(Value::as_array_mut) {
        for item in interps {
            item.as_object_mut().unwrap().remove("inference_availability");
        }
    }
    if let Some(dsets) = obj.get_mut("datasets").and_then(Value::as_array_mut) {
        for d in dsets {
            d.as_object_mut().unwrap().remove("rsdr");
        }
    }
    let legacy_decoded: NonlinearFitResult = serde_json::from_value(ls_val).unwrap();
    assert_eq!(legacy_decoded.fit_method, FitMethod::LeastSquares);
    assert_eq!(legacy_decoded.inference_availability, InferenceAvailability::Available);
    assert_eq!(legacy_decoded.rsdr, None);
    assert!(legacy_decoded.parameters.iter().all(|p| p.se_option().is_some()));
}

#[test]
fn r45_engine_session_exports_and_methods_report_for_robust_and_rout() {
    let mut session = build_seeded_xy_session();

    // Run RobustCauchy with bands, covariance, comparison, and unknown interpolation enabled.
    let robust_out = exec_ok(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "fit-robust",
            "source_sheet_id": "xy-dose",
            "kind": "4pl",
            "method": "robust",
            "confidence_bands": true,
            "prediction_bands": true,
            "emit_covariance_tab": true,
            "emit_residuals_tab": true,
            "interpolate_unknowns": true,
            "replicates_lack_of_fit_test": true,
            "comparison": { "method": "f_test", "alternative_equation_id": "log_agonist_vs_response_3pl" }
        }),
    );
    assert_eq!(robust_out.payload["inference_availability"], json!("NotApplicableInRobustMode"));
    assert!(robust_out.payload["scalars"]["RSDR"]["value"].as_f64().is_some_and(f64::is_finite));
    let log_ec50 = robust_out.payload["scalars"]["LogEC50"]["value"].as_f64().unwrap_or_else(|| {
        panic!("missing LogEC50 in scalars: {}", robust_out.payload["scalars"]);
    });
    assert!((log_ec50 - (-6.5)).abs() < 0.1, "unexpected LogEC50={log_ec50}, scalars={}", robust_out.payload["scalars"]);

    // Export JSON and CSV cleanly (no non-finite leaks, renders "Not applicable").
    let json_exp = exec_ok(&mut session, "analysis.exportJson", json!({ "sheet_id": "fit-robust" }));
    let json_text = json_exp.payload["json"].as_str().unwrap();
    assert!(json_text.contains("\"Not applicable\""));
    assert!(json_text.contains("NotApplicableInRobustMode"));

    let csv_exp = exec_ok(&mut session, "analysis.exportCsv", json!({ "sheet_id": "fit-robust" }));
    let csv_text = csv_exp.payload["csv"].as_str().unwrap();
    assert!(csv_text.contains("Not applicable"));
    assert!(csv_text.contains("RSDR"));

    // Methods report renders accurate RobustCauchy explanation instead of LS CIs / F-test claims.
    let report_out = exec_ok(&mut session, "result.generateReport", json!({ "result_sheet_id": "fit-robust", "format": "markdown" }));
    let report = report_out.payload["report"].as_str().unwrap();
    assert!(report.contains("NotApplicableInRobustMode"));
    assert!(report.contains("not applicable"));
    assert!(!report.contains("profile-likelihood confidence intervals were computed"));

    // Contrast against ROUT Stage 2 on same data: full LS inference reported cleanly.
    let rout_out = exec_ok(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "fit-rout",
            "source_sheet_id": "xy-dose",
            "kind": "4pl",
            "method": "rout",
            "confidence_bands": true,
            "prediction_bands": true,
            "emit_covariance_tab": true,
            "interpolate_unknowns": true,
            "comparison": { "method": "f_test", "alternative_equation_id": "log_agonist_vs_response_3pl" }
        }),
    );
    assert!(rout_out.payload.get("inference_availability").is_none());
    assert!(rout_out.payload["scalars"].get("RSDR").is_none());
    let res_rout = session.document().result_sheet(&SheetId::new("fit-rout")).unwrap();
    assert!(res_rout.tabs.iter().any(|t| t.id == "nonlinear_covariance"));
    let interp_tab = res_rout.tabs.iter().find(|t| t.id == "nonlinear_interpolated").unwrap();
    assert!(
        interp_tab.rows.iter().all(|row| matches!(row[2], ResultCell::ConfidenceInterval { lower: CiBound::Finite(_), upper: CiBound::Finite(_) }))
    );
}

#[test]
fn r45_chart_xy_suppresses_robust_bands_even_on_tampered_overlay_and_keeps_rout_bands() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let true_p = [10.0, 100.0, -6.5, 1.0];
    let mut ys: Vec<f64> = xs.iter().enumerate().map(|(i, &x)| model.eval(&[x], &true_p, 0).unwrap() + if i % 2 == 0 { 0.2 } else { -0.2 }).collect();
    ys[2] += 250.0;

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new("d-chart", "Assay", TableArchetype::Xy(xy)).unwrap();

    let mut nl_robust = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_robust.method = FitMethod::RobustCauchy;
    nl_robust.confidence_bands = true;
    nl_robust.prediction_bands = true;
    nl_robust.weighting = WeightingScheme::None;
    let spec_robust = AnalysisSpec::new("res-robust", "d-chart", AnalysisKind::NonlinearRegression(nl_robust)).unwrap();
    let mut res_robust = execute_analysis(&spec_robust, &sheet, &HookContext::default()).unwrap();

    // Also stuff huge bogus band numbers into the serialized curve to prove chart xy.rs strips them defense-in-depth.
    for pt in &mut res_robust.fitted_curves[0].points {
        pt.ci_lower = Some(-5_000.0);
        pt.ci_upper = Some(5_000.0);
        pt.pred_lower = Some(-9_000.0);
        pt.pred_upper = Some(9_000.0);
    }

    let mut nl_rout = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_rout.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl_rout.confidence_bands = true;
    nl_rout.prediction_bands = true;
    nl_rout.weighting = WeightingScheme::None;
    let spec_rout = AnalysisSpec::new("res-rout", "d-chart", AnalysisKind::NonlinearRegression(nl_rout)).unwrap();
    let res_rout = execute_analysis(&spec_rout, &sheet, &HookContext::default()).unwrap();

    let mut doc = ProjectDocument::new("Chart Band Suppression").unwrap();
    doc.add_data_sheet(sheet.clone()).unwrap();
    doc.add_result_sheet(res_robust).unwrap();
    doc.add_result_sheet(res_rout).unwrap();

    // 1. Robust overlay with band flags OFF vs ON -> IDENTICAL primitives, no bands drawn, warning emitted.
    let mut g_robust_off = GraphSheet::new_uninitialized_for_data("g-robust-off", &sheet).unwrap();
    g_robust_off.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-robust"),
        sample_segments: 64,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    let scene_off = compile_graph(&g_robust_off, &doc).unwrap();
    assert!(!scene_off.warnings.iter().any(|w| w.code == "robust_bands_suppressed"));

    let mut g_robust_on = g_robust_off.clone();
    g_robust_on.id = SheetId::new("g-robust-on");
    let ov_on = g_robust_on.spec.series[0].regression_overlay.as_mut().unwrap();
    ov_on.show_confidence_band = true;
    ov_on.show_prediction_band = true;
    let scene_on = compile_graph(&g_robust_on, &doc).unwrap();
    assert!(scene_on.warnings.iter().any(|w| w.code == "robust_bands_suppressed"));
    assert_eq!(scene_on.primitives, scene_off.primitives, "RobustCauchy must never emit CI or prediction band primitives");

    // 2. Contrast with ROUT Stage 2: emits filled ribbon polygons when bands + shaded_ribbon are enabled.
    let mut g_rout_off = g_robust_off.clone();
    g_rout_off.id = SheetId::new("g-rout-off");
    g_rout_off.spec.series[0].regression_overlay.as_mut().unwrap().result_sheet_id = SheetId::new("res-rout");
    let scene_rout_off = compile_graph(&g_rout_off, &doc).unwrap();

    let mut g_rout_on = g_rout_off.clone();
    g_rout_on.id = SheetId::new("g-rout-on");
    let rout_ov = g_rout_on.spec.series[0].regression_overlay.as_mut().unwrap();
    rout_ov.show_confidence_band = true;
    rout_ov.show_prediction_band = true;
    let scene_rout_on = compile_graph(&g_rout_on, &doc).unwrap();
    assert!(!scene_rout_on.warnings.iter().any(|w| w.code == "robust_bands_suppressed"));
    let filled_ribbons = scene_rout_on.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), stroke: None, .. })).count();
    let baseline_ribbons = scene_rout_off.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), stroke: None, .. })).count();
    assert!(filled_ribbons >= baseline_ribbons + 2, "ROUT Stage 2 must draw CI and prediction ribbons");
}
