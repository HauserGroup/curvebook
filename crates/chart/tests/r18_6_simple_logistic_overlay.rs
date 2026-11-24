#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Chart overlay & equation box tests for R18.6 Standalone XY Simple Logistic Regression.

use curvebook_chart::compile_graph;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, EquationDisplayMode, GraphAnnotation, GraphSheet, ProjectDocument, RectPt, RegressionOverlaySpec,
    SheetId, SimpleLogisticSpec, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_stats::run_analysis;

#[test]
fn sigmoid_curve_overlay_and_equation_box_compile_cleanly() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 12).expect("xy");
    let pts = [
        (0.5, 0.0),
        (1.0, 0.0),
        (1.5, 0.0),
        (2.0, 1.0),
        (2.5, 0.0),
        (3.0, 1.0),
        (3.5, 0.0),
        (4.0, 1.0),
        (4.5, 1.0),
        (5.0, 1.0),
        (5.5, 1.0),
        (6.0, 1.0),
    ];
    for (r, (x, y)) in pts.into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new("xy-1", "Binary Dose", TableArchetype::Xy(xy)).expect("sheet");
    let sl_spec = SimpleLogisticSpec {
        emit_row_classification: true,
        confidence_bands: true,
        classification_cutoff: 0.5,
        plot_x_min: Some(0.0),
        plot_x_max: Some(7.0),
        plot_segments: Some(80),
    };
    let spec = AnalysisSpec::new("res-sl", "xy-1", AnalysisKind::SimpleLogisticRegression(sl_spec)).expect("spec");
    let result = run_analysis(&sheet, &spec).expect("run_analysis");

    let mut graph = GraphSheet::new_uninitialized_for_data("graph-1", &sheet).expect("graph");
    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::try_new("res-sl").expect("sheet id"),
        sample_segments: 256,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::try_new("res-sl").expect("sheet id"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: RectPt::new(80.0, 40.0, 220.0, 40.0),
        font_size_pt: 10.0,
        bordered: true,
    });

    let mut doc = ProjectDocument::new("Logistic Chart").expect("doc");
    doc.data_sheets.push(sheet);
    doc.result_sheets.push(result);
    doc.graph_sheets.push(graph.clone());

    let scene = compile_graph(&graph, &doc).expect("compile_graph");
    assert!(scene.warnings.is_empty(), "expected zero chart warnings, got {:?}", scene.warnings);
    assert!(!scene.primitives.is_empty());
}
