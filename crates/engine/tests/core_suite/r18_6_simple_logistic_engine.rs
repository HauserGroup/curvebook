#![allow(clippy::expect_used, clippy::unwrap_used)]

//! End-to-end engine integration tests for R18.6 Standalone XY Simple Logistic Regression.

use curvebook_doc::{AnalysisKind, Cell, DataSheet, ResultStatus, SheetId, SimpleLogisticSpec, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

use super::common::{err_cmd, ok_cmd, rs, scalar, session_with};

fn build_binary_xy_session() -> Session {
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
        (2.2, None),
        (4.8, None),
    ];
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, data.len()).expect("xy");
    for (r, (x, y)) in data.into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = y.map_or_else(Cell::empty, Cell::number);
    }
    session_with("R18.6 Engine Test", vec![DataSheet::new("xy-1", "Dose Response", TableArchetype::Xy(xy)).expect("sheet")])
}

#[test]
fn engine_runs_simple_logistic_updates_params_recomputes_and_emits_methods_report() {
    let mut session = build_binary_xy_session();

    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "res-sl", "source_sheet_id": "xy-1", "kind": "simple_logistic", "emit_row_classification": true,
            "confidence_bands": true, "classification_cutoff": 0.5, "plot_x_min": 0.0, "plot_x_max": 7.0, "plot_segments": 70
        }),
    );

    let res = rs(&session, "res-sl");
    assert_eq!((&res.status, res.tabs.len(), res.fitted_curves.len()), (&ResultStatus::UpToDate, 4, 2));
    assert!(matches!(res.analysis.kind, AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec { emit_row_classification: true, .. })));
    assert_eq!(res.fitted_curves.iter().find(|c| c.equation_id == "simple_logistic").expect("simple_logistic overlay").points.len(), 71);
    assert!(!res.fitted_curves.iter().find(|c| c.equation_id == "ROC").expect("ROC overlay").points.is_empty());

    let beta1_before = scalar(&session, "res-sl", "Beta1");
    assert!(beta1_before > 0.0);

    // Update parameters to disable Row Classification tab and change cutoff
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-sl", "emit_row_classification": false, "classification_cutoff": 0.6 }));
    let res_updated = rs(&session, "res-sl");
    assert_eq!((&res_updated.status, res_updated.tabs.len()), (&ResultStatus::UpToDate, 3));
    assert!(res_updated.tabs.iter().all(|t| t.title != "Row Classification"));

    // Reactive DAG recompute when an upstream Y cell changes
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "xy-1", "row": 3, "col": 0, "subcol": 0, "value": 0.0 }));
    assert_eq!(rs(&session, "res-sl").status, ResultStatus::UpToDate);
    assert!((scalar(&session, "res-sl", "Beta1") - beta1_before).abs() > 1e-4, "Beta1 must update reactively on upstream edit");

    // Methods report renders cleanly
    let report = generate_methods_report(session.document(), &SheetId::new("res-sl"), ReportFormat::Markdown).expect("methods report");
    assert!(report.contains("simple binary logistic regression") && report.contains("Tjur's R^2"));
}

#[test]
fn engine_rejects_invalid_simple_logistic_cutoff() {
    let mut session = build_binary_xy_session();
    let err = err_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "xy-1", "kind": "simple_logistic", "classification_cutoff": 1.5 }));
    assert!(err.message().contains("classification_cutoff"));
}
