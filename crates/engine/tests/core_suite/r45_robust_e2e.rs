#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CiBound, FitMethod, GraphSheet, ModelComparisonSpec, NonlinearFitSpec, ProjectDocument, RegressionOverlaySpec,
    ResultCell, SheetId, SourceCellId, TableArchetype, WeightingScheme,
};
use curvebook_engine::Session;
use curvebook_fit::{
    DataPoint, DatasetSeries, HookContext, InferenceAvailability, ModelComparisonOutcome, ModelComparisonResult, NonlinearFitResult, SolverConfig,
    compare_models_checked, execute_analysis, fit_nonlinear, get_builtin_model,
};
use serde_json::{Value, json};

use super::common::{ds, ok_cmd as exec_ok, rs, run_cmds, xy_sheet};

fn build_seeded_xy_session() -> Session {
    let mut session = Session::new();
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let mut x_cells: Vec<Value> = xs.iter().enumerate().map(|(r, &x)| json!({ "row": r, "value": x })).collect();
    let mut y_cells: Vec<Value> = xs
        .iter()
        .enumerate()
        .map(|(r, &x)| {
            let mut y = model.eval(&[x], &[10.0, 100.0, -6.5, 1.0], 0).unwrap() + if r % 2 == 0 { 0.25 } else { -0.25 };
            if r == 5 {
                y += 300.0;
            }
            json!({ "row": r, "col": 0, "subcol": 0, "value": y })
        })
        .collect();
    y_cells.push(json!({ "row": 12, "col": 0, "subcol": 0, "value": 55.0 }));
    x_cells.push(json!({ "row": 13, "value": -6.5 }));
    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "xy-dose", "title": "Dose Response", "archetype": "xy", "rows": 14, "cols": 1, "replicates": 1, "auto_graph": false }),
            ),
            ("table.setXCells", json!({ "sheet_id": "xy-dose", "edits": x_cells })),
            ("table.setCells", json!({ "sheet_id": "xy-dose", "edits": y_cells })),
        ],
    );
    session
}

#[test]
fn r45_versioned_serde_roundtrip_and_legacy_compatibility() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let alt_model = get_builtin_model("log_agonist_vs_response_3pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let ys: Vec<f64> = xs.iter().map(|&x| model.eval(&[x], &[10.0, 100.0, -6.5, 1.0], 0).unwrap()).collect();
    let data = vec![DatasetSeries {
        data_set_index: 0,
        label: "Dose".to_owned(),
        points: xs.iter().zip(ys.iter()).enumerate().map(|(row, (&x, &y))| DataPoint { row, ..DataPoint::xy(x, y) }).collect(),
        unknowns_y: vec![(11, 55.0).into()],
        unknowns_x: vec![(12, -6.5)],
        is_pre_summarized: false,
        column_title_value: None,
    }];

    // 1. RobustCauchy result serde round-trip
    let robust_cfg = SolverConfig {
        method: FitMethod::RobustCauchy,
        compute_bands: true,
        compute_prediction_bands: true,
        interpolate_unknowns: true,
        curve_overlay_segments: 16,
        ..Default::default()
    };
    let robust_fit = fit_nonlinear(model.as_ref(), &data, &HookContext::default(), &robust_cfg).unwrap();
    let rt: NonlinearFitResult = serde_json::from_str(&serde_json::to_string(&robust_fit).unwrap()).unwrap();
    assert_eq!(
        (rt.fit_method, rt.inference_availability, rt.rsdr),
        (FitMethod::RobustCauchy, InferenceAvailability::NotApplicableInRobustMode, robust_fit.rsdr)
    );
    assert!(rt.parameters.iter().all(|p| p.se_option().is_none() && p.ci_bounds_option().is_none() && !p.se.is_finite()));
    assert!(rt.interpolated_x[0].ci_bounds_option().is_none() && rt.interpolated_x[0].output_estimate.is_finite());

    // 2. ModelComparisonResult round-trip in NotApplicableInRobustMode
    let comp = compare_models_checked(
        &robust_fit,
        alt_model.as_ref(),
        &data,
        &HookContext::default(),
        &robust_cfg,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "log_agonist_vs_response_3pl".to_owned() },
    )
    .unwrap();
    let comp_rt: ModelComparisonResult = serde_json::from_str(&serde_json::to_string(&comp).unwrap()).unwrap();
    assert_eq!(comp_rt.inference_availability, InferenceAvailability::NotApplicableInRobustMode);
    assert!(!comp_rt.is_applicable() && matches!(comp_rt.outcome, ModelComparisonOutcome::NotApplicableInRobustMode { .. }));

    // 3. Legacy LeastSquares JSON missing new fields deserializes cleanly via #[serde(default)]
    let ls_fit = fit_nonlinear(model.as_ref(), &data, &HookContext::default(), &SolverConfig::default()).unwrap();
    let mut ls_val = serde_json::to_value(&ls_fit).unwrap();
    let obj = ls_val.as_object_mut().unwrap();
    for k in ["fit_method", "inference_availability", "rsdr"] {
        obj.remove(k);
    }
    for (arr_key, field) in [("parameters", "inference_availability"), ("interpolated_x", "inference_availability"), ("datasets", "rsdr")] {
        if let Some(items) = obj.get_mut(arr_key).and_then(Value::as_array_mut) {
            for item in items {
                item.as_object_mut().unwrap().remove(field);
            }
        }
    }
    let legacy: NonlinearFitResult = serde_json::from_value(ls_val).unwrap();
    assert_eq!((legacy.fit_method, legacy.inference_availability, legacy.rsdr), (FitMethod::LeastSquares, InferenceAvailability::Available, None));
    assert!(legacy.parameters.iter().all(|p| p.se_option().is_some()));
}

#[test]
fn r45_engine_session_exports_and_methods_report_for_robust_and_rout() {
    let mut session = build_seeded_xy_session();

    let robust_out = exec_ok(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "fit-robust", "source_sheet_id": "xy-dose", "kind": "4pl", "method": "robust", "confidence_bands": true, "prediction_bands": true, "emit_covariance_tab": true, "emit_residuals_tab": true, "interpolate_unknowns": true, "replicates_lack_of_fit_test": true, "comparison": { "method": "f_test", "alternative_equation_id": "log_agonist_vs_response_3pl" } }),
    );
    assert_eq!(robust_out.payload["inference_availability"], "NotApplicableInRobustMode");
    assert!(robust_out.payload["scalars"]["RSDR"]["value"].as_f64().is_some_and(f64::is_finite));
    assert!((robust_out.payload["scalars"]["LogEC50"]["value"].as_f64().unwrap() + 6.5).abs() < 0.1);

    let json_text = exec_ok(&mut session, "analysis.exportJson", json!({ "sheet_id": "fit-robust" })).payload["json"].as_str().unwrap().to_owned();
    assert!(json_text.contains("\"Not applicable\"") && json_text.contains("NotApplicableInRobustMode"));

    let csv_text = exec_ok(&mut session, "analysis.exportCsv", json!({ "sheet_id": "fit-robust" })).payload["csv"].as_str().unwrap().to_owned();
    assert!(csv_text.contains("Not applicable") && csv_text.contains("RSDR"));

    let report = exec_ok(&mut session, "result.generateReport", json!({ "result_sheet_id": "fit-robust", "format": "markdown" })).payload["report"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        report.contains("NotApplicableInRobustMode")
            && report.contains("not applicable")
            && !report.contains("profile-likelihood confidence intervals were computed")
    );

    // Contrast against ROUT Stage 2: full LS inference reported cleanly.
    let rout_out = exec_ok(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "fit-rout", "source_sheet_id": "xy-dose", "kind": "4pl", "method": "rout", "confidence_bands": true, "prediction_bands": true, "emit_covariance_tab": true, "interpolate_unknowns": true, "comparison": { "method": "f_test", "alternative_equation_id": "log_agonist_vs_response_3pl" } }),
    );
    assert!(rout_out.payload.get("inference_availability").is_none() && rout_out.payload["scalars"].get("RSDR").is_none());
    let res_rout = rs(&session, "fit-rout");
    assert!(res_rout.tabs.iter().any(|t| t.id == "nonlinear_covariance"));
    let interp = res_rout.tabs.iter().find(|t| t.id == "nonlinear_interpolated").unwrap();
    assert!(interp.rows.iter().all(|row| matches!(row[2], ResultCell::ConfidenceInterval { lower: CiBound::Finite(_), upper: CiBound::Finite(_) })));
}

#[test]
fn r45_chart_xy_suppresses_robust_bands_even_on_tampered_overlay_and_keeps_rout_bands() {
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
    let mut ys: Vec<f64> =
        xs.iter().enumerate().map(|(i, &x)| model.eval(&[x], &[10.0, 100.0, -6.5, 1.0], 0).unwrap() + if i % 2 == 0 { 0.2 } else { -0.2 }).collect();
    ys[2] += 250.0;
    let sheet = xy_sheet("d-chart", "Assay", "X", &xs, &[("Y", &ys)]);

    let mk_fit = |id: &str, method: FitMethod| {
        let nl = NonlinearFitSpec {
            method,
            confidence_bands: true,
            prediction_bands: true,
            weighting: WeightingScheme::None,
            ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
        };
        execute_analysis(&AnalysisSpec::new(id, "d-chart", AnalysisKind::NonlinearRegression(nl)).unwrap(), &sheet, &HookContext::default()).unwrap()
    };
    let mut res_robust = mk_fit("res-robust", FitMethod::RobustCauchy);
    for pt in &mut res_robust.fitted_curves[0].points {
        (pt.ci_lower, pt.ci_upper, pt.pred_lower, pt.pred_upper) = (Some(-5_000.0), Some(5_000.0), Some(-9_000.0), Some(9_000.0));
    }

    let mut doc = ProjectDocument::new("Chart Band Suppression").unwrap();
    doc.add_data_sheet(sheet.clone()).unwrap();
    doc.add_result_sheet(res_robust).unwrap();
    doc.add_result_sheet(mk_fit("res-rout", FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 })).unwrap();

    let compile_ov = |gid: &str, rid: &str, bands: bool| {
        let mut g = GraphSheet::new_uninitialized_for_data(gid, &sheet).unwrap();
        g.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
            result_sheet_id: SheetId::new(rid),
            sample_segments: 64,
            show_confidence_band: bands,
            show_prediction_band: bands,
            shaded_ribbon: true,
        });
        compile_graph(&g, &doc).unwrap()
    };

    let (scene_off, scene_on) = (compile_ov("g-r-off", "res-robust", false), compile_ov("g-r-on", "res-robust", true));
    assert!(
        !scene_off.warnings.iter().any(|w| w.code == "robust_bands_suppressed")
            && scene_on.warnings.iter().any(|w| w.code == "robust_bands_suppressed")
    );
    assert_eq!(scene_on.primitives, scene_off.primitives);

    let (rout_off, rout_on) = (compile_ov("g-rout-off", "res-rout", false), compile_ov("g-rout-on", "res-rout", true));
    assert!(!rout_on.warnings.iter().any(|w| w.code == "robust_bands_suppressed"));
    let count_ribbons = |sc: &curvebook_chart::ChartScene| {
        sc.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), stroke: None, .. })).count()
    };
    assert!(count_ribbons(&rout_on) >= count_ribbons(&rout_off) + 2);
}

#[test]
fn rout_overlay_marks_the_exact_raw_replicate_and_follows_recalculation() {
    let mut session = Session::new();
    let sheet_id = SheetId::new("replicated");
    let x_cells: Vec<Value> = (0..10).map(|row| json!({"row": row, "value": f64::from(row)})).collect();
    let y_cells: Vec<Value> = (0..10)
        .flat_map(|row| {
            let base = 3.0 + 2.0 * f64::from(row);
            [
                json!({"row": row, "col": 0, "subcol": 0, "value": base - 0.2}),
                json!({"row": row, "col": 0, "subcol": 2, "value": base + if row == 5 { 50.0 } else { 0.2 }}),
            ]
        })
        .collect();
    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": &sheet_id, "title": "Replicated calibration", "archetype": "xy", "rows": 10, "cols": 1, "replicates": 3, "auto_graph": false }),
            ),
            ("table.setXCells", json!({"sheet_id": &sheet_id, "edits": x_cells})),
            ("table.setCells", json!({"sheet_id": &sheet_id, "edits": y_cells})),
            ("fit.run", json!({"sheet_id": "replicated-fit", "source_sheet_id": &sheet_id, "equation_id": "straight_line", "method": "rout"})),
        ],
    );

    let source = SourceCellId { sheet_id: sheet_id.clone(), row: 5, col: 0, subcol: 2 };
    assert_eq!(rs(&session, "replicated-fit").eliminated_outliers.as_slice(), std::slice::from_ref(&source));
    let sheet = ds(&session, "replicated");
    let TableArchetype::Xy(xy) = &sheet.archetype else { panic!("expected XY") };
    assert_eq!(xy.data_sets[0].cells[5][2].active_number(), Some(63.0));

    let mut graph = GraphSheet::new_uninitialized_for_data("rout-graph", sheet).unwrap();
    assert!(compile_graph(&graph, session.document()).unwrap().plotted_points.iter().all(|pt| !pt.value_summary.starts_with("ROUT outlier")));

    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("replicated-fit"),
        sample_segments: 64,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    let scene = compile_graph(&graph, session.document()).unwrap();
    let outliers: Vec<_> = scene.plotted_points.iter().filter(|pt| pt.value_summary.starts_with("ROUT outlier")).collect();
    assert_eq!(outliers.len(), 1);
    assert_eq!((&outliers[0].source_cell, scene.hit_test_point(outliers[0].center_pt, 0.0)), (&source, Some(outliers[0])));
    assert!(
        scene
            .primitives
            .iter()
            .any(|p| matches!(p, ScenePrimitive::Path { fill: None, stroke: Some(st), clip: Some(_), .. } if (st.width_pt - 1.5).abs() < 1e-12))
    );

    exec_ok(&mut session, "table.setCells", json!({"sheet_id": &sheet_id, "edits": [{"row": 5, "col": 0, "subcol": 2, "value": 13.2}]}));
    assert!(rs(&session, "replicated-fit").eliminated_outliers.is_empty());
    assert!(compile_graph(&graph, session.document()).unwrap().plotted_points.iter().all(|pt| !pt.value_summary.starts_with("ROUT outlier")));
}
