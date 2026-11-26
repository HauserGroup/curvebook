//! End-to-end engine, DAG reactivity, methods report, and chart overlay tests for R18.7 `FitSplineLowess`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use curvebook_doc::{Cell, DataSheet, ProjectDocument, ResultStatus, ResultTabKind, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

fn seed_xy_session() -> Session {
    let mut table = XyTable::new(SubcolumnMode::Single, 1, 8).expect("xy table");
    table.data_sets[0].title = String::from("Signal");
    let pts = [(0.0, 0.0), (1.0, 1.0), (2.0, 4.0), (3.0, 9.0), (4.0, 16.0), (5.0, 25.0)];
    for (r, &(x, y)) in pts.iter().enumerate() {
        table.x_cells[r][0] = Cell::number(x);
        table.data_sets[0].cells[r][0] = Cell::number(y);
    }
    // Row 6: forward unknown (X = 2.5, blank Y)
    table.x_cells[6][0] = Cell::number(2.5);
    // Row 7: inverse unknown (blank X, Y = 9.0)
    table.data_sets[0].cells[7][0] = Cell::number(9.0);
    let ds = DataSheet::new("xy-1", "Standard Curve", TableArchetype::Xy(table)).expect("sheet");
    let mut doc = ProjectDocument::new("R18.7 Spline/LOWESS").expect("doc");
    doc.data_sheets.push(ds);
    Session::from_document(doc)
}

#[test]
fn engine_runs_all_five_spline_lowess_methods_reactively_and_renders_chart_overlay() {
    let mut session = seed_xy_session();

    // 1. Run Natural Cubic Spline via `analysis.run` with coordinates tab + unknown interpolation.
    let out = session
        .execute(
            "analysis.run",
            json!({
                "source_sheet_id": "xy-1",
                "sheet_id": "res-spline",
                "kind": "fit_spline_lowess",
                "segment_count": 120,
                "emit_curve_coordinates_tab": true,
                "interpolate_unknowns": true
            }),
        )
        .expect("run fit_spline_lowess");
    assert_eq!(out.payload["fitted_curves_count"], 1);

    let res = session.document().result_sheet(&"res-spline".into()).expect("result sheet");
    assert_eq!(res.status, ResultStatus::UpToDate);
    assert_eq!(res.fitted_curves[0].points.len(), 120);
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::CurveCoordinates));
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::InterpolatedUnknowns));
    let y_mid_before = res.fitted_curves[0].points[60].y;

    // 2. Reactive DAG update: mutating an upstream cell recomputes the spline curve.
    session.execute("table.setCell", json!({ "sheet_id": "xy-1", "row": 3, "col": 0, "subcol": 0, "value": 15.0 })).expect("edit y");
    let res_after = session.document().result_sheet(&"res-spline".into()).expect("result sheet");
    assert_eq!(res_after.status, ResultStatus::UpToDate);
    let y_mid_after = res_after.fitted_curves[0].points[60].y;
    assert!((y_mid_after - y_mid_before).abs() > 1.0, "upstream edit must recompute spline overlay");

    // 3. In-place parameter switch via `analysis.updateParams` through all other methods.
    for (method, expected_eq) in
        [("AkimaSubspline", "akima_subspline"), ("SmoothingSpline", "smoothing_spline"), ("Lowess", "lowess"), ("PointToPoint", "point_to_point")]
    {
        session
            .execute("analysis.updateParams", json!({ "sheet_id": "res-spline", "method": method }))
            .unwrap_or_else(|e| panic!("updateParams to {method} failed: {e}"));
        let updated = session.document().result_sheet(&"res-spline".into()).expect("updated");
        assert_eq!(updated.status, ResultStatus::UpToDate);
        assert_eq!(updated.fitted_curves[0].equation_id, expected_eq);
    }

    // 4. Methods report succeeds across all formats.
    for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
        let text = generate_methods_report(session.document(), &"res-spline".into(), fmt).expect("methods report");
        assert!(!text.is_empty(), "methods report ({fmt:?}) must be non-empty");
    }

    // 5. Attach FitSplineLowess result sheet as a chart overlay and render via curvebook-chart.
    session
        .execute("graph.create", json!({ "sheet_id": "g-spline", "data_sheet_id": "xy-1", "kind": "xy", "title": "Spline Graph" }))
        .expect("create graph");
    session
        .execute(
            "graph.setRegressionOverlay",
            json!({
                "sheet_id": "g-spline",
                "series_index": 0,
                "result_sheet_id": "res-spline",
                "show_confidence_band": false,
                "show_prediction_band": false
            }),
        )
        .expect("attach overlay");
    let graph = session.document().graph_sheet(&"g-spline".into()).expect("graph");
    let scene = curvebook_chart::compile_graph(graph, session.document()).expect("build chart scene");
    assert!(!scene.primitives.is_empty(), "chart scene with FitSplineLowess overlay must render primitives");
    assert!(
        scene.warnings.iter().all(|w| w.code != "missing_fitted_curve" && w.code != "failed_regression_result"),
        "chart scene must attach FitSplineLowess overlay cleanly: {:?}",
        scene.warnings
    );
}
