//! End-to-end engine, DAG reactivity, methods report, and chart overlay tests for R18.7 `FitSplineLowess`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use curvebook_doc::{Cell, DataSheet, ResultStatus, ResultTabKind, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

use super::common::{gs, ok_cmd, rs, session_with};

fn seed_xy_session() -> Session {
    let mut table = XyTable::new(SubcolumnMode::Single, 1, 8).expect("xy table");
    table.data_sets[0].title = String::from("Signal");
    for (r, &(x, y)) in [(0.0, 0.0), (1.0, 1.0), (2.0, 4.0), (3.0, 9.0), (4.0, 16.0), (5.0, 25.0)].iter().enumerate() {
        table.x_cells[r][0] = Cell::number(x);
        table.data_sets[0].cells[r][0] = Cell::number(y);
    }
    table.x_cells[6][0] = Cell::number(2.5); // forward unknown (X = 2.5, blank Y)
    table.data_sets[0].cells[7][0] = Cell::number(9.0); // inverse unknown (blank X, Y = 9.0)
    session_with("R18.7 Spline/LOWESS", vec![DataSheet::new("xy-1", "Standard Curve", TableArchetype::Xy(table)).expect("sheet")])
}

#[test]
fn engine_runs_all_five_spline_lowess_methods_reactively_and_renders_chart_overlay() {
    let mut session = seed_xy_session();

    // 1. Run Natural Cubic Spline via `analysis.run` with coordinates tab + unknown interpolation.
    let out = ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "source_sheet_id": "xy-1", "sheet_id": "res-spline", "kind": "fit_spline_lowess", "segment_count": 120, "emit_curve_coordinates_tab": true, "interpolate_unknowns": true }),
    );
    assert_eq!(out.payload["fitted_curves_count"], 1);
    let res = rs(&session, "res-spline");
    assert_eq!((&res.status, res.fitted_curves[0].points.len()), (&ResultStatus::UpToDate, 120));
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::CurveCoordinates));
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::InterpolatedUnknowns));
    let y_mid_before = res.fitted_curves[0].points[60].y;

    // 2. Reactive DAG update: mutating an upstream cell recomputes the spline curve.
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "xy-1", "row": 3, "col": 0, "subcol": 0, "value": 15.0 }));
    let res_after = rs(&session, "res-spline");
    assert_eq!(res_after.status, ResultStatus::UpToDate);
    assert!((res_after.fitted_curves[0].points[60].y - y_mid_before).abs() > 1.0, "upstream edit must recompute spline overlay");

    // 3. In-place parameter switch via `analysis.updateParams` through all other methods.
    for (method, expected_eq) in
        [("AkimaSubspline", "akima_subspline"), ("SmoothingSpline", "smoothing_spline"), ("Lowess", "lowess"), ("PointToPoint", "point_to_point")]
    {
        ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-spline", "method": method }));
        let updated = rs(&session, "res-spline");
        assert_eq!((&updated.status, updated.fitted_curves[0].equation_id.as_str()), (&ResultStatus::UpToDate, expected_eq));
    }

    // 4. Methods report succeeds across all formats.
    for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
        assert!(!generate_methods_report(session.document(), &"res-spline".into(), fmt).expect("methods report").is_empty());
    }

    // 5. Attach FitSplineLowess result sheet as a chart overlay and render via curvebook-chart.
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-spline", "data_sheet_id": "xy-1", "kind": "xy", "title": "Spline Graph" }));
    ok_cmd(
        &mut session,
        "graph.setRegressionOverlay",
        json!({ "sheet_id": "g-spline", "series_index": 0, "result_sheet_id": "res-spline", "show_confidence_band": false, "show_prediction_band": false }),
    );
    let scene = curvebook_chart::compile_graph(gs(&session, "g-spline"), session.document()).expect("build chart scene");
    assert!(!scene.primitives.is_empty());
    assert!(scene.warnings.iter().all(|w| w.code != "missing_fitted_curve" && w.code != "failed_regression_result"), "{:?}", scene.warnings);
}
