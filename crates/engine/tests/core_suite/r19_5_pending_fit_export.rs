//! `R19.5`: a fit pending recalculation must never supply its cached curve to an exported figure,
//! and every figure export must state the warning instead of returning a figure that looks complete.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CurveSamplePoint, FittedCurveOverlay, GraphSheet, NonlinearFitSpec, ProjectDocument, RegressionOverlaySpec,
    ResultSheet, ResultStatus, SheetId,
};
use curvebook_engine::{CommandOutcome, Session};
use serde_json::{Value, json};

use super::common::{ok_cmd, xy_sheet};

#[derive(Clone, Copy)]
enum Overlay {
    Current,
    Pending,
    None,
}

fn doc(overlay: Overlay) -> ProjectDocument {
    let mut doc = ProjectDocument::new("Pending fit").unwrap();
    let ds = xy_sheet("d", "Dose response", "", &[1.0, 2.0, 3.0, 4.0], &[("", &[10.0, 21.0, 29.0, 41.0])]);
    doc.add_data_sheet(ds.clone()).unwrap();

    let mut res =
        ResultSheet::new(AnalysisSpec::new("fit", "d", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("linear"))).unwrap()).unwrap();
    res.status = if matches!(overlay, Overlay::Pending) { ResultStatus::Dirty } else { ResultStatus::UpToDate };
    let pt = |x, y| CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None };
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: true,
        ambiguous: false,
        points: vec![pt(1.0, 10.0), pt(4.0, 40.0)],
    });
    doc.add_result_sheet(res).unwrap();

    let mut graph = GraphSheet::new_uninitialized_for_data("g", &ds).unwrap();
    graph.spec.series[0].regression_overlay = (!matches!(overlay, Overlay::None)).then(|| RegressionOverlaySpec {
        result_sheet_id: SheetId::new("fit"),
        sample_segments: 50,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    doc.add_graph_sheet(graph).unwrap();
    doc
}

fn export(overlay: Overlay, command: &str, params: Value) -> CommandOutcome {
    ok_cmd(&mut Session::from_document(doc(overlay)), command, params)
}

fn warning_codes(outcome: &CommandOutcome) -> Vec<String> {
    outcome.payload["warnings"].as_array().unwrap().iter().map(|w| w["code"].as_str().unwrap().to_owned()).collect()
}

#[test]
fn pending_fit_exports_the_figure_without_its_old_curve() {
    let svg = |overlay| export(overlay, "graph.exportSvg", json!({ "sheet_id": "g" })).payload["svg"].as_str().unwrap().to_owned();
    let (current, pending, without) = (svg(Overlay::Current), svg(Overlay::Pending), svg(Overlay::None));
    assert_ne!(current, without, "fixture must draw a fitted curve when the fit is up to date");
    assert_eq!(pending, without, "a pending fit must not contribute its cached curve to the exported figure");
}

#[test]
fn every_figure_export_states_the_pending_fit_warning() {
    for (command, params) in [
        ("graph.exportSvg", json!({ "sheet_id": "g" })),
        ("export.svg", json!({ "sheet_id": "g" })),
        ("graph.exportPdf", json!({ "sheet_id": "g" })),
        ("graph.exportPng", json!({ "sheet_id": "g", "width_px": 200, "height_px": 150 })),
        ("export.tiff", json!({ "sheet_id": "g", "width_px": 200, "height_px": 150 })),
        ("graph.exportEmf", json!({ "sheet_id": "g" })),
    ] {
        let out = export(Overlay::Pending, command, params);
        let codes = warning_codes(&out);
        assert!(codes.iter().any(|c| c == "pending_regression_result"), "{command}: payload warnings {codes:?}");
        assert_eq!(out.payload["warnings_count"], json!(codes.len()), "{command}: warnings_count");
        assert!(out.warnings.iter().any(|w| w.starts_with("pending_regression_result: ") && w.contains("'fit'")), "{command}: {:?}", out.warnings);
        assert!(out.message.contains("warning(s)"), "{command}: message `{}`", out.message);
    }
}

#[test]
fn current_fit_exports_without_warnings() {
    let out = export(Overlay::Current, "graph.exportSvg", json!({ "sheet_id": "g" }));
    assert!(warning_codes(&out).is_empty() && out.warnings.is_empty());
    assert_eq!(out.message, "exported SVG figure");
}
