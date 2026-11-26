#![allow(clippy::expect_used, clippy::unwrap_used)]

//! End-to-end engine integration tests for R18.6 Standalone XY Simple Logistic Regression.
//!
//! Verifies `stats.run` (`simple_logistic` shorthand), `analysis.update`, DAG reactive recompute
//! on upstream data edits, methods report generation, and invalid cutoff rejection.

use curvebook_doc::{
    AnalysisKind, Cell, DataSheet, ProjectDocument, ResultStatus, SheetId, SimpleLogisticSpec, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

fn build_binary_xy_session() -> Session {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 14).expect("xy");
    let data = [
        (0.5, Some(0.0)),
        (1.0, Some(0.0)),
        (1.5, Some(0.0)),
        (2.0, Some(1.0)),
        (2.5, Some(0.0)),
        (3.0, Some(1.0)),
        (3.5, Some(0.0)),
        (4.0, Some(1.0)),
        (4.5, Some(1.0)),
        (5.0, Some(1.0)),
        (5.5, Some(1.0)),
        (6.0, Some(1.0)),
        (2.2, None), // appended (X_new, blank Y)
        (4.8, None), // appended (X_new, blank Y)
    ];
    for (r, (x, y)) in data.into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = match y {
            Some(v) => Cell::number(v),
            None => Cell::empty(),
        };
    }
    let sheet = DataSheet::new("xy-1", "Dose Response", TableArchetype::Xy(xy)).expect("sheet");
    let mut doc = ProjectDocument::new("R18.6 Engine Test").expect("doc");
    doc.data_sheets.push(sheet);
    Session::from_document(doc)
}

#[test]
fn engine_runs_simple_logistic_updates_params_recomputes_and_emits_methods_report() {
    let mut session = build_binary_xy_session();

    let _out = session
        .execute(
            "stats.run",
            json!({
                "sheet_id": "res-sl",
                "source_sheet_id": "xy-1",
                "kind": "simple_logistic",
                "emit_row_classification": true,
                "confidence_bands": true,
                "classification_cutoff": 0.5,
                "plot_x_min": 0.0,
                "plot_x_max": 7.0,
                "plot_segments": 70
            }),
        )
        .expect("stats.run simple_logistic should succeed");

    let res_id_str = String::from("res-sl");
    let res_id = SheetId::try_new(&res_id_str).expect("valid sheet id");
    let res = session.document().result_sheet(&res_id).expect("result sheet present");
    assert_eq!(res.status, ResultStatus::UpToDate);
    assert!(matches!(res.analysis.kind, AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec { emit_row_classification: true, .. })));
    assert_eq!(res.tabs.len(), 4);
    assert_eq!(res.fitted_curves.len(), 2);
    let logistic_curve = res.fitted_curves.iter().find(|c| c.equation_id == "simple_logistic").expect("simple_logistic overlay");
    assert_eq!(logistic_curve.points.len(), 71);
    let roc_curve = res.fitted_curves.iter().find(|c| c.equation_id == "ROC").expect("ROC overlay");
    assert!(!roc_curve.points.is_empty());

    let beta1_before = res.scalars.iter().find(|s| s.key == "Beta1").expect("Beta1 scalar").value;
    assert!(beta1_before > 0.0);

    // Update parameters to disable Row Classification tab and change cutoff
    session
        .execute(
            "analysis.updateParams",
            json!({
                "sheet_id": res_id_str,
                "emit_row_classification": false,
                "classification_cutoff": 0.6
            }),
        )
        .expect("analysis.updateParams should succeed");
    let res_updated = session.document().result_sheet(&res_id).expect("result sheet present");
    assert_eq!(res_updated.status, ResultStatus::UpToDate);
    assert_eq!(res_updated.tabs.len(), 3);
    assert!(res_updated.tabs.iter().all(|t| t.title != "Row Classification"));

    // Reactive DAG recompute when an upstream Y cell changes
    session
        .execute(
            "table.setCell",
            json!({
                "sheet_id": "xy-1",
                "row": 3,
                "col": 0,
                "subcol": 0,
                "value": 0.0
            }),
        )
        .expect("table.setCell should succeed");
    let res_after = session.document().result_sheet(&res_id).expect("result sheet present");
    assert_eq!(res_after.status, ResultStatus::UpToDate);
    let beta1_after = res_after.scalars.iter().find(|s| s.key == "Beta1").expect("Beta1 scalar").value;
    assert!((beta1_after - beta1_before).abs() > 1e-4, "Beta1 must update reactively on upstream edit");

    // Methods report renders cleanly
    let report = generate_methods_report(session.document(), &res_id, ReportFormat::Markdown).expect("methods report");
    assert!(report.contains("simple binary logistic regression"));
    assert!(report.contains("Tjur's R^2"));
}

#[test]
fn engine_rejects_invalid_simple_logistic_cutoff() {
    let mut session = build_binary_xy_session();
    let err = session
        .execute(
            "stats.run",
            json!({
                "source_sheet_id": "xy-1",
                "kind": "simple_logistic",
                "classification_cutoff": 1.5
            }),
        )
        .expect_err("cutoff >= 1 must fail");
    assert!(err.message().contains("classification_cutoff"));
}
