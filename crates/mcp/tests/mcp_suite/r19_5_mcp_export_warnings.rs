//! `R19.5`: the MCP `export` tool reports figure warnings, so an agent exporting a graph whose fit is
//! pending recalculation learns that the fitted curve was omitted.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CurveSamplePoint, DataSheet, FittedCurveOverlay, GraphSheet, NonlinearFitSpec, ProjectDocument,
    RegressionOverlaySpec, ResultSheet, ResultStatus, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_mcp::{ContentBlock, McpServerState, ToolCallResult, call_tool};
use serde_json::json;

fn state(status: ResultStatus) -> McpServerState {
    let mut doc = ProjectDocument::new("Pending fit").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3).unwrap();
    for (r, (x, y)) in [(1.0, 10.0), (2.0, 21.0), (3.0, 29.0)].into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new("d", "Dose response", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();
    let mut res =
        ResultSheet::new(AnalysisSpec::new("fit", "d", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("linear"))).unwrap()).unwrap();
    res.status = status;
    let pt = |x: f64, y: f64| CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None };
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: true,
        ambiguous: false,
        points: vec![pt(1.0, 10.0), pt(3.0, 30.0)],
    });
    doc.add_result_sheet(res).unwrap();
    let mut graph = GraphSheet::new_uninitialized_for_data("g", &ds).unwrap();
    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("fit"),
        sample_segments: 20,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    doc.add_graph_sheet(graph).unwrap();
    McpServerState::from_session(Session::from_document(doc))
}

fn export(state: &mut McpServerState, format: &str) -> ToolCallResult {
    call_tool(state, "export", json!({ "format": format, "sheet_id": "g" })).unwrap_or_else(|e| panic!("export {format}: {e:?}"))
}

#[test]
fn export_tool_reports_pending_fit_warning() {
    let mut st = state(ResultStatus::Dirty);
    for format in ["svg", "pdf", "png"] {
        let result = export(&mut st, format);
        assert!(!result.is_error, "{format}: {result:?}");
        let warnings = result.structured_content.as_ref().and_then(|v| v.get("warnings")).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        assert!(warnings.iter().any(|w| w["code"] == json!("pending_regression_result")), "{format}: structured warnings {warnings:?}");
        assert!(
            result.content.iter().any(
                |b| matches!(b, ContentBlock::Text { text } if text.contains("Figure warnings (") && text.contains("pending_regression_result"))
            ),
            "{format}: no warning text block in {:?}",
            result.content
        );
    }
}

#[test]
fn export_tool_adds_nothing_for_current_fit() {
    let mut st = state(ResultStatus::UpToDate);
    let result = export(&mut st, "svg");
    assert!(result.structured_content.is_none(), "inline SVG without warnings keeps its plain text result: {result:?}");
    assert!(!result.content.iter().any(|b| matches!(b, ContentBlock::Text { text } if text.contains("Figure warnings"))));
}
