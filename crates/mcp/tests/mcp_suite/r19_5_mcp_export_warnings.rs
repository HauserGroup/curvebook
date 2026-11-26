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

#[test]
fn gap_06_01_live_attach_export_with_path_forwards_figure_warnings() {
    let out_dir = std::env::temp_dir().join(format!("cb-gap-06-01-live-export-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&out_dir);

    let mut st = McpServerState::with_control_port(49861).unwrap().with_control_handler(|method, params| {
        assert_eq!(method, "doc.export");
        let path = params.get("path").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
        let format = params.get("format").and_then(serde_json::Value::as_str).unwrap_or("svg").to_owned();
        Ok(json!({
            "exported": true,
            "format": format,
            "sheet_id": "g",
            "path": path,
            "bytes_written": 256,
            "warnings": [
                {
                    "code": "pending_regression_result",
                    "message": "Series 'Y' regression overlay from 'fit' was omitted because the analysis is pending recalculation."
                }
            ]
        }))
    });

    for (format, path_key) in [("svg", "path"), ("pdf", "output_path"), ("png", "path")] {
        let dest = out_dir.join(format!("figure.{format}"));
        let result = call_tool(&mut st, "export", json!({ "format": format, "sheet_id": "g", path_key: dest.display().to_string() }))
            .unwrap_or_else(|e| panic!("live export {format}: {e:?}"));
        assert!(!result.is_error, "{format}: {result:?}");

        let sc = result.structured_content.as_ref().expect("live file export structured_content");
        assert_eq!(sc["exported"], json!(true));
        assert_eq!(sc["mode"], json!("live_attach"));
        assert_eq!(sc["bytes_written"], json!(256));
        let warnings = sc.get("warnings").and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
        assert!(
            warnings.iter().any(|w| w["code"] == json!("pending_regression_result")),
            "{format}: missing pending_regression_result in structured warnings: {warnings:?}"
        );
        assert!(
            result.content.iter().any(
                |b| matches!(b, ContentBlock::Text { text } if text.contains("Figure warnings (1):") && text.contains("pending_regression_result"))
            ),
            "{format}: missing Figure warnings text block in {:?}",
            result.content
        );
    }

    // Empty warnings array from desktop on a current fit must not inject a warnings block.
    let mut clean = McpServerState::with_control_port(49862).unwrap().with_control_handler(|method, params| {
        assert_eq!(method, "doc.export");
        Ok(json!({
            "exported": true,
            "format": "svg",
            "sheet_id": "g",
            "path": params["path"],
            "bytes_written": 128,
            "warnings": []
        }))
    });
    let clean_dest = out_dir.join("clean.svg");
    let clean_res = call_tool(&mut clean, "export", json!({ "format": "svg", "sheet_id": "g", "path": clean_dest.display().to_string() })).unwrap();
    assert!(!clean_res.is_error);
    let clean_sc = clean_res.structured_content.as_ref().expect("clean structured_content");
    assert!(clean_sc.get("warnings").is_none(), "empty warnings should not populate structured_content.warnings: {clean_sc:?}");
    assert!(!clean_res.content.iter().any(|b| matches!(b, ContentBlock::Text { text } if text.contains("Figure warnings"))));

    let _ = std::fs::remove_dir_all(&out_dir);
}
