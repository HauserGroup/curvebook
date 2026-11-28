//! Histogram scene compilation tests (`GraphKind::Histogram`): contiguous bars, XY frequency
//! polygon curve, stepped outline, bin-center vs. bin-edge X ticks, cumulative mode, Gaussian
//! density/CDF overlay, multi-series overlay & side-by-side layout, raw DataSheet binding, and
//! stale/failed ResultSheet omission.

use curvebook_chart::{PathCmd, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, FrequencyBinningRule, FrequencyDistributionSpec,
    FrequencyOutputMode, GraphKind, GraphSheet, GraphicalDataObject, HistogramBinAlignment, HistogramDisplayStyle, HistogramMultiSeriesMode,
    HistogramSpec, ProjectDocument, ResultStatus, SheetId, TableArchetype,
};
use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_stats::run_analysis;

fn make_two_group_column_sheet() -> DataSheet {
    let a = [1.2, 2.5, 2.8, 3.1, 3.9, 4.0, 4.4, 5.2, 6.1, 7.8, 8.3, 9.5];
    let b = [4.0, 5.1, 5.8, 6.2, 7.0, 7.5, 8.1, 8.9, 9.4, 10.2, 11.0, 12.1];
    let mut col = ColumnTable::new(ColumnDesignHint::default(), 2, a.len()).unwrap();
    col.data_sets[0].title = String::from("Control");
    col.data_sets[1].title = String::from("Treated");
    let mut ds = DataSheet::new("col-1", "Assay Values", TableArchetype::Column(col)).unwrap();
    for r in 0..a.len() {
        ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(a[r])).unwrap();
        ds.set_cell(CellCoord::new(r, 1, 0), Cell::number(b[r])).unwrap();
    }
    ds
}

#[test]
fn histogram_from_frequency_result_renders_bars_gaussian_curve_and_edge_ticks() {
    let ds = make_two_group_column_sheet();
    let fd = FrequencyDistributionSpec {
        binning_rule: FrequencyBinningRule::ExplicitWidth { width: 2.0 },
        first_bin_center: Some(2.0),
        last_bin_center: Some(12.0),
        output_mode: FrequencyOutputMode::Counts,
        overlay_gaussian: true,
        ..FrequencyDistributionSpec::default()
    };
    let spec = AnalysisSpec::new("res-fd", "col-1", AnalysisKind::FrequencyDistribution(fd)).unwrap();
    let res = run_analysis(&ds, &spec).expect("frequency distribution");

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone()).unwrap();
    doc.add_result_sheet(res).unwrap();

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-hist"), &ds).unwrap();
    gs.initialized = true;
    gs.spec.kind = GraphKind::Histogram;
    gs.spec.primary_source_sheet_id = SheetId::new("res-fd");
    gs.spec.x_axis.title = String::from("Value");
    gs.spec.y_axis.title = String::from("Number of values");
    gs.spec.histogram = HistogramSpec {
        display_style: HistogramDisplayStyle::Bars,
        bin_alignment: HistogramBinAlignment::BinCenters,
        cumulative: false,
        show_gaussian_overlay: true,
        multi_series_mode: HistogramMultiSeriesMode::Overlay,
        ..HistogramSpec::default()
    };
    doc.add_graph_sheet(gs.clone()).unwrap();

    // 1. Bars + overlay + Gaussian overlay.
    let scene = compile_graph(&gs, &doc).expect("compile histogram bars");
    assert!(scene.warnings.is_empty(), "unexpected warnings: {:?}", scene.warnings);
    assert_eq!(scene.plotted_points.len(), 12, "2 series x 6 bins plotted points");
    let filled_rects = scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: Some(_), .. })).count();
    assert!(filled_rects >= 6, "expected histogram bar rectangles, got {filled_rects}");
    let gaussian_paths = scene
        .primitives
        .iter()
        .filter(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() > 50))
        .count();
    assert_eq!(gaussian_paths, 2, "expected 2 Gaussian overlay curves");

    // 2. Side-by-side multi-series + BinEdges ticks + cumulative curve.
    let mut gs_sbs = gs.clone();
    gs_sbs.spec.histogram.multi_series_mode = HistogramMultiSeriesMode::SideBySide;
    gs_sbs.spec.histogram.bin_alignment = HistogramBinAlignment::BinEdges;
    gs_sbs.spec.histogram.cumulative = true;
    let scene_sbs = compile_graph(&gs_sbs, &doc).expect("compile side-by-side cumulative histogram");
    assert!(scene_sbs.warnings.is_empty());
    // Edge tick labels should include "1" and "13" (edges [1, 3, 5, 7, 9, 11, 13]).
    let tick_texts: Vec<String> = scene_sbs
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect();
    assert!(tick_texts.iter().any(|t| t == "1"), "expected left edge tick '1' in {tick_texts:?}");
    assert!(tick_texts.iter().any(|t| t == "13"), "expected right edge tick '13' in {tick_texts:?}");

    // 3. XY frequency polygon curve and StepOutline modes compile cleanly.
    for style in [HistogramDisplayStyle::XyCurve, HistogramDisplayStyle::StepOutline] {
        let mut g_alt = gs.clone();
        g_alt.spec.histogram.display_style = style;
        let sc = compile_graph(&g_alt, &doc).expect("compile alt histogram style");
        assert!(sc.warnings.is_empty());
        let line_paths = sc
            .primitives
            .iter()
            .filter(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.iter().any(|c| matches!(c, PathCmd::LineTo(_)))))
            .count();
        assert!(line_paths >= 2, "expected line paths for {style:?}");
    }
}

#[test]
fn histogram_direct_from_raw_datasheet_and_dirty_result_guard() {
    let ds = make_two_group_column_sheet();
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone()).unwrap();

    // Direct raw DataSheet binding (e.g., Change Graph Type -> Histogram).
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-raw"), &ds).unwrap();
    gs.initialized = true;
    gs.spec.kind = GraphKind::Histogram;
    gs.spec.histogram.binning_rule = FrequencyBinningRule::AutoSturges;
    gs.spec.histogram.output_mode = FrequencyOutputMode::Percentages;
    gs.spec.histogram.show_gaussian_overlay = true;
    doc.add_graph_sheet(gs.clone()).unwrap();

    let scene = compile_graph(&gs, &doc).expect("compile raw-bound histogram");
    assert!(scene.warnings.is_empty(), "unexpected warnings: {:?}", scene.warnings);
    assert!(!scene.plotted_points.is_empty());

    // Dirty FrequencyDistribution ResultSheet must NOT plot stale bars and must emit pending warning.
    let spec = AnalysisSpec::new("res-dirty", "col-1", AnalysisKind::FrequencyDistribution(FrequencyDistributionSpec::default())).unwrap();
    let mut res = run_analysis(&ds, &spec).unwrap();
    res.status = ResultStatus::Dirty;
    doc.add_result_sheet(res).unwrap();

    let mut gs_dirty = gs;
    gs_dirty.id = SheetId::new("g-dirty");
    gs_dirty.spec.primary_source_sheet_id = SheetId::new("res-dirty");
    doc.add_graph_sheet(gs_dirty.clone()).unwrap();

    let dirty_scene = compile_graph(&gs_dirty, &doc).expect("compile dirty histogram");
    assert!(dirty_scene.plotted_points.is_empty(), "dirty result must not plot stale histogram bars");
    assert!(dirty_scene.warnings.iter().any(|w| w.code == "pending_source_result"));
}

#[test]
fn histogram_preserves_dataset_index_with_empty_leading_column_and_matches_gaussian_legend_dash() {
    // Column 0 ("Empty") has no values; Column 1 ("Active") has finite values.
    let vals = [1.0, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0];
    let mut col = ColumnTable::new(ColumnDesignHint::default(), 2, vals.len()).unwrap();
    col.data_sets[0].title = String::from("Empty");
    col.data_sets[1].title = String::from("Active");
    let mut ds = DataSheet::new("col-sparse", "Sparse Assay", TableArchetype::Column(col)).unwrap();
    for (r, &v) in vals.iter().enumerate() {
        ds.set_cell(CellCoord::new(r, 1, 0), Cell::number(v)).unwrap();
    }

    let color_col0 = Rgba8::rgb(220, 20, 20);
    let color_col1 = Rgba8::rgb(20, 140, 220);

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone()).unwrap();

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-sparse"), &ds).unwrap();
    gs.initialized = true;
    gs.spec.kind = GraphKind::Histogram;
    gs.spec.histogram.binning_rule = FrequencyBinningRule::ExplicitWidth { width: 2.0 };
    gs.spec.histogram.show_gaussian_overlay = true;
    gs.spec.series[0].legend_label = Some(String::from("Series Zero"));
    gs.spec.series[0].gdo = GraphicalDataObject::BarWithReplicates {
        summary: curvebook_doc::SummaryMeasure::Mean,
        fill_color: color_col0,
        border_color: color_col0,
        border_width_pt: 1.0,
        replicate_overlay: None,
    };
    gs.spec.series[1].legend_label = Some(String::from("Series One"));
    gs.spec.series[1].gdo = GraphicalDataObject::BarWithReplicates {
        summary: curvebook_doc::SummaryMeasure::Mean,
        fill_color: color_col1,
        border_color: color_col1,
        border_width_pt: 1.0,
        replicate_overlay: None,
    };
    doc.add_graph_sheet(gs.clone()).unwrap();

    // 1. Raw DataSheet binding preserves data_set_index == 1, custom label, and color_col1.
    let raw_scene = compile_graph(&gs, &doc).expect("compile sparse raw histogram");
    assert!(raw_scene.warnings.is_empty(), "unexpected warnings: {:?}", raw_scene.warnings);
    assert!(!raw_scene.plotted_points.is_empty());
    for pt in &raw_scene.plotted_points {
        assert_eq!(pt.source_cell.col, 1, "must attribute plotted bin to column 1, not empty column 0");
        assert_eq!(pt.series_label, "Series One");
    }
    let bar_strokes: Vec<Rgba8> = raw_scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { fill: Some(_), stroke: Some(st), clip: Some(_), .. } => Some(st.color),
            _ => None,
        })
        .collect();
    assert!(!bar_strokes.is_empty());
    assert!(bar_strokes.iter().all(|&c| c == color_col1), "expected bars to use column 1 color {color_col1:?}, got {bar_strokes:?}");

    // 2. ResultSheet binding also preserves data_set_index == 1 when column 0 is empty.
    let fd = FrequencyDistributionSpec {
        binning_rule: FrequencyBinningRule::ExplicitWidth { width: 2.0 },
        overlay_gaussian: true,
        ..FrequencyDistributionSpec::default()
    };
    let res = run_analysis(&ds, &AnalysisSpec::new("res-sparse", "col-sparse", AnalysisKind::FrequencyDistribution(fd)).unwrap()).unwrap();
    doc.add_result_sheet(res).unwrap();
    let mut gs_res = gs.clone();
    gs_res.id = SheetId::new("g-sparse-res");
    gs_res.spec.primary_source_sheet_id = SheetId::new("res-sparse");
    let res_scene = compile_graph(&gs_res, &doc).expect("compile sparse result histogram");
    assert!(res_scene.warnings.is_empty());
    assert!(res_scene.plotted_points.iter().all(|pt| pt.source_cell.col == 1 && pt.series_label == "Series One"));

    // 3. Gaussian legend line dash matches rendered Gaussian overlay dash in Bars (Solid) and XyCurve/StepOutline (Dashed).
    for (style, expected_dash) in [
        (HistogramDisplayStyle::Bars, LineDashStyle::Solid),
        (HistogramDisplayStyle::XyCurve, LineDashStyle::Dashed),
        (HistogramDisplayStyle::StepOutline, LineDashStyle::Dashed),
    ] {
        let mut g_dash = gs.clone();
        g_dash.spec.histogram.display_style = style;
        let sc = compile_graph(&g_dash, &doc).expect("compile dash-parity histogram");
        let curve_dash = sc
            .primitives
            .iter()
            .find_map(|p| match p {
                ScenePrimitive::Path { path, fill: None, stroke: Some(st), clip: Some(_) } if path.commands.len() > 50 => Some(st.dash),
                _ => None,
            })
            .expect("rendered Gaussian overlay curve");
        assert_eq!(curve_dash, expected_dash, "rendered Gaussian curve dash mismatch for {style:?}");

        // Legend swatch line for the Gaussian entry is an unclipped 2-point horizontal path stroked with `color_col1`.
        let legend_line_dashes: Vec<LineDashStyle> = sc
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Path { path, fill: None, stroke: Some(st), clip: None } if path.commands.len() == 2 && st.color == color_col1 => {
                    Some(st.dash)
                }
                _ => None,
            })
            .collect();
        assert!(!legend_line_dashes.is_empty(), "expected Gaussian legend line swatch for {style:?}");
        assert_eq!(
            *legend_line_dashes.last().unwrap(),
            curve_dash,
            "Gaussian legend dash must match rendered curve dash for {style:?}: legend={legend_line_dashes:?}, curve={curve_dash:?}"
        );
    }
}
