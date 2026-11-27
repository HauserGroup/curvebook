//! Chart rendering tests for schema v9 imported curve overlay labels, imported-hook isolation,
//! and 2-subcolumn `SubcolumnMode::MeanSd` / `MeanSem` error bars (schema v9 §3.4, §3.5).

use curvebook_chart::{ScenePrimitive, compile_graph, extract_fit_summary_entries};
use curvebook_doc::{
    AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, CurveSamplePoint, DataSheet, ErrorBarMetric, ErrorBarSpec, FittedCurveOverlay, GraphSheet,
    GraphSpec, ProjectDocument, RegressionOverlaySpec, ResultScalarEntry, ResultSheet, ResultStatus, SheetId, SubcolumnMode, TableArchetype,
    XColumnMode, XyTable,
};

fn collect_scene_text(scene: &curvebook_chart::ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

#[test]
fn v9_imported_overlay_renders_with_visible_imported_and_stale_labels_and_blocks_hooks() {
    let mut doc = ProjectDocument::new("Imported Overlay Chart").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3).unwrap();
    xy.x_cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)], vec![Cell::number(3.0)]];
    xy.data_sets[0].title = String::from("Compound A");
    xy.data_sets[0].cells = vec![vec![Cell::number(10.0)], vec![Cell::number(25.0)], vec![Cell::number(40.0)]];
    doc.add_data_sheet(DataSheet::new("xy-1", "Dose", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let mut imp = ResultSheet::new_imported(
        AnalysisSpec::new_imported_snapshot("imp-fit", Some(SheetId::new("xy-1")), "Imported 4PL").unwrap(),
        "external_archive",
        "4PL Fit",
        "Imported from external file",
    )
    .unwrap();
    imp.scalars.push(ResultScalarEntry { key: String::from("IC50"), data_set_index: Some(0), value: 2.0, units: Some(String::from("uM")) });
    imp.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("imported_4pl"),
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 1.0, y: 10.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 2.0, y: 25.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 3.0, y: 40.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    doc.add_result_sheet(imp).unwrap();

    let mut gspec = GraphSpec::default_for_data_sheet(&doc.data_sheets[0]);
    gspec.title = String::from("Dose IC50={{result:imp-fit:0:IC50}}");
    gspec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("imp-fit"),
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    let gs = GraphSheet {
        id: SheetId::new("g-1"),
        title: String::from("Dose Plot"),
        initialized: true,
        spec: gspec,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    };
    doc.add_graph_sheet(gs).unwrap();

    // 1. Fresh imported overlay: visible "Imported" label on scene & fit-summary entries; inline hook NOT expanded to "2".
    let scene = compile_graph(&doc.graph_sheets[0], &doc).unwrap();
    let texts = collect_scene_text(&scene);
    assert!(texts.iter().any(|t| t.contains("Imported")), "expected visible Imported overlay label in scene texts: {texts:?}");
    assert!(!texts.iter().any(|t| t.contains("IC50=2")), "inline {{result:...}} hook must not resolve scalars from imported results: {texts:?}");
    let summary_entries = extract_fit_summary_entries(&doc.graph_sheets[0], &doc, 6);
    assert_eq!(summary_entries.len(), 1);
    assert_eq!(summary_entries[0].imported_label.as_deref(), Some("Imported (not computed by CurveBook)"));

    // 2. Stale imported overlay: visible "Imported (stale)" label on scene & fit-summary entries.
    doc.result_sheet_mut(&SheetId::new("imp-fit")).unwrap().status = ResultStatus::ImportedStale;
    let stale_scene = compile_graph(&doc.graph_sheets[0], &doc).unwrap();
    let stale_texts = collect_scene_text(&stale_scene);
    assert!(stale_texts.iter().any(|t| t.contains("Imported (stale)")), "expected stale label in scene texts: {stale_texts:?}");
    let stale_entries = extract_fit_summary_entries(&doc.graph_sheets[0], &doc, 6);
    assert_eq!(stale_entries[0].imported_label.as_deref(), Some("Imported (stale)"));
}

#[test]
fn v9_mean_sd_and_mean_sem_plot_error_bars_and_warn_when_ci95_requested() {
    let mut doc = ProjectDocument::new("No-N Summary Graphs").unwrap();

    // XY with MeanSd Y and MeanSem X.
    let mut xy = XyTable::new(SubcolumnMode::MeanSd, 1, 3).unwrap();
    xy.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSem };
    xy.x_cells =
        vec![vec![Cell::number(1.0), Cell::number(0.1)], vec![Cell::number(2.0), Cell::number(0.15)], vec![Cell::number(3.0), Cell::number(0.2)]];
    xy.data_sets[0].cells =
        vec![vec![Cell::number(10.0), Cell::number(1.2)], vec![Cell::number(20.0), Cell::number(1.8)], vec![Cell::number(30.0), Cell::number(2.4)]];
    doc.add_data_sheet(DataSheet::new("xy-sd", "XY MeanSd", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let mut gspec = GraphSpec::default_for_data_sheet(&doc.data_sheets[0]);
    gspec.series[0].y_error_bars = Some(ErrorBarSpec { metric: ErrorBarMetric::Sd, ..ErrorBarSpec::default() });
    let gs_sd = GraphSheet {
        id: SheetId::new("g-xy-sd"),
        title: String::from("XY SD"),
        initialized: true,
        spec: gspec.clone(),
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    };
    let scene_sd = compile_graph(&gs_sd, &doc).unwrap();
    assert!(!scene_sd.primitives.is_empty());
    assert!(!scene_sd.warnings.iter().any(|w| w.code == "missing_sample_size_for_ci"));

    // Requesting 95% CI on MeanSd (no N) emits `missing_sample_size_for_ci` warning without panicking.
    gspec.series[0].y_error_bars = Some(ErrorBarSpec { metric: ErrorBarMetric::ConfidenceInterval95, ..ErrorBarSpec::default() });
    let gs_ci = GraphSheet {
        id: SheetId::new("g-xy-ci"),
        title: String::from("XY CI"),
        initialized: true,
        spec: gspec,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    };
    let scene_ci = compile_graph(&gs_ci, &doc).unwrap();
    assert!(
        scene_ci.warnings.iter().any(|w| w.code == "missing_sample_size_for_ci"),
        "expected missing_sample_size_for_ci warning, got {:?}",
        scene_ci.warnings
    );

    // Column table with MeanSem compiles cleanly.
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSem, 2, 1).unwrap();
    col.data_sets[0].cells = vec![vec![Cell::number(15.0), Cell::number(1.5)]];
    col.data_sets[1].cells = vec![vec![Cell::number(22.0), Cell::number(2.1)]];
    doc.add_data_sheet(DataSheet::new("col-sem", "Col MeanSem", TableArchetype::Column(col)).unwrap()).unwrap();
    let gspec_col = GraphSpec::default_for_data_sheet(&doc.data_sheets[1]);
    let gs_col = GraphSheet {
        id: SheetId::new("g-col-sem"),
        title: String::from("Col SEM"),
        initialized: true,
        spec: gspec_col,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    };
    let scene_col = compile_graph(&gs_col, &doc).unwrap();
    assert!(!scene_col.primitives.is_empty());
}
