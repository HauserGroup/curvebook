//! Regression tests for Round 2 (`R0.19`) (`R2-S11-16`): hit-testing auto-sized `ModelEquation` boxes.

use curvebook_chart::{PointPt, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, EquationDisplayMode, GraphAnnotation, GraphSheet, LinearRegressionSpec, ProjectDocument,
    RectPt as DocRectPt, ResultScalarEntry, ResultSheet, ResultStatus, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_tools::canvas::{CanvasHitTarget, hit_test_graph};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn build_doc_with_equation(ann: GraphAnnotation) -> Result<(ProjectDocument, GraphSheet), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("AutoSizeEq")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    for r in 0..4 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64))?;
        *xy.data_sets.first_mut().and_then(|ds| ds.cells.get_mut(r)).and_then(|row| row.first_mut()).ok_or("missing xy cell")? =
            Cell::number(2.0 * ((r + 1) as f64) + 1.5);
    }
    let ds = DataSheet::new(SheetId::new("xy-1"), "XY", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(ds)?;

    let lin_spec = AnalysisSpec::new(SheetId::new("res-lin"), SheetId::new("xy-1"), AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?;
    let mut res = ResultSheet::new(lin_spec)?;
    res.status = ResultStatus::UpToDate;
    res.scalars = vec![
        ResultScalarEntry { key: String::from("Slope"), data_set_index: Some(0), value: 2.0, units: None },
        ResultScalarEntry { key: String::from("YIntercept"), data_set_index: Some(0), value: 1.5, units: None },
    ];
    doc.add_result_sheet(res)?;

    let ds_ref = doc.data_sheet(&SheetId::new("xy-1")).ok_or("missing xy-1")?;
    let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new("g-1"), ds_ref)?;
    graph.spec.annotations.push(ann);
    Ok((doc, graph))
}

#[test]
fn r2_s11_16_auto_sized_model_equation_boxes_hit_test_across_bordered_unbordered_and_single_axis() -> TestResult {
    for (bordered, bounds) in [
        (true, DocRectPt::new(80.0, 50.0, 0.0, 0.0)),
        (false, DocRectPt::new(80.0, 50.0, 0.0, 0.0)),
        (true, DocRectPt::new(80.0, 50.0, 120.0, 0.0)),
        (false, DocRectPt::new(80.0, 50.0, 0.0, 26.0)),
    ] {
        let ann = GraphAnnotation::ModelEquation {
            result_sheet_id: SheetId::new("res-lin"),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: bounds,
            font_size_pt: 10.0,
            bordered,
        };
        let (doc, graph) = build_doc_with_equation(ann)?;
        let scene = compile_graph(&graph, &doc)?;

        // Interior point right over the compiled auto-sized equation (`x=105`, `y=60`).
        assert_eq!(
            hit_test_graph(&scene, &graph.spec, PointPt::new(105.0, 60.0), 2.0),
            Some(CanvasHitTarget::Annotation { index: 0 }),
            "expected hit on auto-sized ModelEquation (bordered={bordered}, bounds={bounds:?})"
        );
        // Point well outside the equation box must not falsely hit annotation 0.
        assert_ne!(
            hit_test_graph(&scene, &graph.spec, PointPt::new(260.0, 180.0), 2.0),
            Some(CanvasHitTarget::Annotation { index: 0 }),
            "unexpected hit far outside ModelEquation (bordered={bordered}, bounds={bounds:?})"
        );
    }
    Ok(())
}
