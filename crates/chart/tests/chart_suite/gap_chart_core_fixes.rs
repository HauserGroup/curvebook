//! Regression tests for `GAP-01-01` .. `GAP-01-08` (chart core, equation box, scales, XY overlays, fit summary).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use curvebook_chart::equation_box::render_tex_equation_box;
use curvebook_chart::fit_summary::{FitSummaryPlacement, FitSummarySpec, extract_fit_summary_entries};
use curvebook_chart::scales::{
    AxisOrientation, MAX_TICKS_PER_SEGMENT, ResolvedAxis, TickComputeConfig, TickKind, compute_axis_ticks_with_warnings, format_tick_rich,
};
use curvebook_chart::{
    ChartScene, CompileOptions, PathCmd, ScenePrimitive, compile_graph, compile_graph_with_options, compile_layout, resolve_data_sheet,
    resolve_data_sheet_or_upstream,
};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, AxisSpec, Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, ConnectingLineStyle,
    ContingencyTable, CurveSamplePoint, DataSheet, DescriptiveSpec, DiagnosticPlotKind, EmbeddedTableStyle, EquationDisplayMode, ErrorCode,
    ErrorPropagationMode, FittedCurveOverlay, GraphAnnotation, GraphKind, GraphSheet, GraphicalDataObject, GroupedTable, LayoutItem, LayoutSheet,
    LayoutSpec, LineConnectionKind, LogitDomain, MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode, NonlinearFitSpec,
    PartsOfWholeTable, ProjectDocument, RectPt as DocRectPt, RegressionOverlaySpec, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus,
    ResultTab, ResultTabKind, SeriesOffsetSpec, SheetId, SizeChannelMapping, SubcolumnMode, SurvivalTable, TableArchetype, TickLabelFormat,
    TransformAnalysisSpec, VariableRole, XyTable,
};
use curvebook_fit::tex_format::format_model_equation_tex;
use curvebook_math::AxisScale;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_text::LocaleNumberPolicy;

use super::common::scene_texts;

fn build_xy_doc_with_fit(points: &[(f64, f64)]) -> (ProjectDocument, GraphSheet) {
    let mut doc = ProjectDocument::new("GAP01XY").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, points.len()).unwrap();
    xy.data_sets[0].title = String::from("Dose A");
    for (r, &(x, y)) in points.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new("d-xy", "XY", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();

    let mut graph = GraphSheet::new_uninitialized_for_data("g-xy", &ds).unwrap();
    graph.initialized = true;
    doc.add_graph_sheet(graph.clone()).unwrap();
    (doc, graph)
}

#[test]
fn gap_01_01_deep_scripted_chains_do_not_overflow_stack_and_tex_fallback_escapes() {
    let chained_caret_dsl = format!("Y = X{}", "^2".repeat(30_000));
    let tex_caret = format_model_equation_tex("custom_dsl", Some(&chained_caret_dsl), &[], false, 4);
    assert!(tex_caret.contains("\\^"), "fallback must escape excessive chained `^`");
    // Count unescaped `^` characters in fallback output.
    let unescaped_carets =
        tex_caret.as_bytes().iter().enumerate().filter(|&(i, &b)| b == b'^' && (i == 0 || tex_caret.as_bytes()[i - 1] != b'\\')).count();
    assert_eq!(unescaped_carets, 0, "expected zero unescaped `^` in excessive fallback chain, got {unescaped_carets}");

    let chained_star_dsl = format!("Y = X{}", "**2".repeat(30_000));
    let tex_star = format_model_equation_tex("custom_dsl", Some(&chained_star_dsl), &[], false, 4);
    assert!(tex_star.contains("\\^"), "fallback must escape excessive `**` power chains");

    // Directly exercise raw unescaped `^` and `_` chains (100k chars) on a bounded 2 MiB thread stack
    // to verify parser depth cap, iterative Drop, iterative Clone, and bounded render/wrap never stack-overflow.
    let raw_sup = format!("Y = X{}", "^2".repeat(50_000));
    let raw_sub = format!("Y = X{}", "_i".repeat(50_000));
    let handle = std::thread::Builder::new()
        .name(String::from("gap-01-01-stack-guard"))
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            for raw in [&raw_sup, &raw_sub] {
                let mut scene = ChartScene::new("g-eq", "Eq", 400.0, 200.0, None);
                render_tex_equation_box(&mut scene, DocRectPt::new(10.0, 10.0, 240.0, 40.0), raw, 10.0, true);
                assert!(!scene.primitives.is_empty());
            }
        })
        .expect("spawn stack-bounded thread");
    handle.join().expect("deep scripted chain must not overflow stack");

    // End-to-end compile_graph with crafted UpToDate fit + ModelEquation annotation.
    let (mut doc, mut graph) = build_xy_doc_with_fit(&[(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)]);
    let mut nl = NonlinearFitSpec::new("custom_dsl");
    nl.custom_dsl = Some(chained_caret_dsl);
    let spec = AnalysisSpec::new("fit-evil", "d-xy", AnalysisKind::NonlinearRegression(nl)).unwrap();
    let mut res = ResultSheet::new(spec).unwrap();
    res.status = ResultStatus::UpToDate;
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("custom_dsl"),
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 1.0, y: 2.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 3.0, y: 6.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    doc.add_result_sheet(res).unwrap();
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("fit-evil"),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: DocRectPt::new(40.0, 30.0, 200.0, 40.0),
        font_size_pt: 10.0,
        bordered: true,
    });
    let scene = compile_graph(&graph, &doc).expect("compile graph with adversarial custom_dsl");
    assert!(!scene.primitives.is_empty());
}

#[test]
fn gap_01_02_adversarial_unmatched_delims_and_tags_parse_in_linear_time() {
    let cases = [
        ("open_parens", "(".repeat(100_000)),
        ("open_brackets", "[".repeat(100_000)),
        ("pipe_parens", "|(".repeat(50_000)),
        ("bare_lt", "<".repeat(100_000)),
        ("unclosed_b", "<b>".repeat(33_333)),
        ("unclosed_sup", "<sup>".repeat(20_000)),
        ("unclosed_left", "\\left(".repeat(16_000)),
        ("unclosed_sqrt_opt", "\\sqrt[".repeat(16_000)),
    ];

    for (label, payload) in &cases {
        let start = Instant::now();
        let mut scene = ChartScene::new("g-adv", *label, 400.0, 200.0, None);
        render_tex_equation_box(&mut scene, DocRectPt::new(10.0, 10.0, 0.0, 0.0), payload, 10.0, false);
        let elapsed = start.elapsed();
        assert!(elapsed.as_secs_f64() < 5.0, "{label} took too long ({elapsed:?}); suspected quadratic rescan");
        assert!(!scene.primitives.is_empty(), "{label} should emit primitives");
    }

    // Verify tex_format fallback escapes unmatched `(`, `[`, `|`, `<`, `>`.
    let adv_dsl = format!("Y = {} + {} + {} + <b>x", "(".repeat(200), "[".repeat(200), "|(".repeat(200));
    let tex = format_model_equation_tex("custom_dsl", Some(&adv_dsl), &[], false, 4);
    assert!(tex.contains("\\textless{}") && tex.contains("\\textgreater{}"), "HTML brackets must be escaped: {tex}");
    assert!(tex.contains("\\(") && tex.contains("\\[") && tex.contains("\\text{|}"), "unmatched delimiters must be escaped: {tex}");
    assert!(!tex.contains("<b>"), "raw HTML tag must not survive fallback: {tex}");

    // Verify valid balanced expressions still render cleanly (fractions, scaled parens, bold tags, radicals, pipes).
    let mut valid_scene = ChartScene::new("g-ok", "Ok", 400.0, 200.0, None);
    render_tex_equation_box(
        &mut valid_scene,
        DocRectPt::new(10.0, 10.0, 0.0, 0.0),
        r"Y = \left(\frac{V_{\max} \cdot X}{K_m + X}\right) + \sqrt[3]{|X|} + <b>2</b>",
        10.0,
        true,
    );
    let texts = scene_texts(&valid_scene).join(" ");
    assert!(texts.contains("max") && texts.contains("X") && texts.contains('2'), "valid math texts={texts}");
    let path_count = valid_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { .. })).count();
    assert!(path_count >= 5, "expected border + vinculum + scaled parens + radical paths, got {path_count}");
}

#[test]
fn gap_01_03_dirty_embedded_result_tables_and_analyzable_data_omitted_with_warnings() {
    let (mut doc, mut graph) = build_xy_doc_with_fit(&[(1.0, 10.0), (2.0, 20.0), (3.0, 30.0)]);

    // 1. Dirty ResultSheet with cached table cells -> EmbeddedResultTable annotation & LayoutItem::LiveTable must omit + warn.
    let spec = AnalysisSpec::new("res-dirty-tab", "d-xy", AnalysisKind::DescriptiveStats(Default::default())).unwrap();
    let mut dirty_res = ResultSheet::new(spec).unwrap();
    dirty_res.status = ResultStatus::Dirty;
    dirty_res.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("P value")],
        row_labels: vec![String::from("Group A")],
        rows: vec![vec![ResultCell::Text(String::from("STALEP0001"))]],
        footnotes: Vec::new(),
    });
    doc.add_result_sheet(dirty_res).unwrap();

    graph.spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
        result_sheet_id: SheetId::new("res-dirty-tab"),
        tab_id: None,
        bounds_pt: DocRectPt::new(180.0, 20.0, 140.0, 60.0),
        selected_rows: None,
        selected_cols: None,
        style: EmbeddedTableStyle::default(),
    });

    let dirty_scene = compile_graph(&graph, &doc).unwrap();
    assert!(
        dirty_scene.warnings.iter().any(|w| w.code == "pending_embedded_result_table"),
        "expected pending_embedded_result_table warning, got {:?}",
        dirty_scene.warnings
    );
    let dirty_texts = scene_texts(&dirty_scene);
    assert!(!dirty_texts.iter().any(|t| t.contains("STALEP0001")), "stale Dirty embedded table cell leaked: {dirty_texts:?}");

    // Layout LiveTable pointing to the same Dirty ResultSheet also omits stale text and warns.
    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::LiveTable {
        id: String::from("tbl-1"),
        source_sheet_id: SheetId::new("res-dirty-tab"),
        tab_id: None,
        bounds_pt: DocRectPt::new(20.0, 20.0, 200.0, 80.0),
        selected_rows: None,
        selected_cols: None,
        style: EmbeddedTableStyle::default(),
    });
    let layout = LayoutSheet::new("lay-dirty", "Dirty Layout", layout_spec).unwrap();
    let layout_scene = compile_layout(&layout, &doc).unwrap();
    assert!(
        layout_scene.warnings.iter().any(|w| w.code == "pending_embedded_result_table"),
        "expected layout pending_embedded_result_table warning, got {:?}",
        layout_scene.warnings
    );
    assert!(!scene_texts(&layout_scene).iter().any(|t| t.contains("STALEP0001")));

    // Marking UpToDate renders the table with zero pending warnings.
    doc.result_sheet_mut(&SheetId::new("res-dirty-tab")).unwrap().status = ResultStatus::UpToDate;
    let fresh_scene = compile_graph(&graph, &doc).unwrap();
    assert!(!fresh_scene.warnings.iter().any(|w| w.code == "pending_embedded_result_table"));
    assert!(scene_texts(&fresh_scene).iter().any(|t| t.contains("STALEP0001")));

    // 2. Dirty ResultSheet carrying `analyzable_data`: `resolve_data_sheet*` returns None and XY/Column graphs warn `pending_source_result`.
    let mut derived_xy = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    derived_xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    derived_xy.set_x_cell(1, Cell::number(2.0)).unwrap();
    derived_xy.data_sets[0].cells[0][0] = Cell::number(99.0);
    derived_xy.data_sets[0].cells[1][0] = Cell::number(101.0);
    let derived_ds = DataSheet::new("res-tr-data", "Derived XY", TableArchetype::Xy(derived_xy)).unwrap();

    let tr_spec = AnalysisSpec::new(
        "res-tr-dirty",
        "d-xy",
        AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: None, y_transform: None, error_propagation: Default::default() }),
    )
    .unwrap();
    let mut tr_res = ResultSheet::new(tr_spec).unwrap();
    tr_res.status = ResultStatus::Dirty;
    tr_res.analyzable_data = Some(derived_ds.clone());
    doc.add_result_sheet(tr_res).unwrap();

    let tr_id = SheetId::new("res-tr-dirty");
    assert!(resolve_data_sheet(&doc, &tr_id).is_none(), "resolve_data_sheet must reject Dirty ResultSheet");
    assert!(resolve_data_sheet_or_upstream(&doc, &tr_id).is_none(), "resolve_data_sheet_or_upstream must reject Dirty ResultSheet");

    let mut xy_from_dirty = GraphSheet::new_uninitialized_for_data("g-tr-dirty", &derived_ds).unwrap();
    xy_from_dirty.spec.primary_source_sheet_id = tr_id.clone();
    xy_from_dirty.spec.series[0].source_sheet_id = tr_id.clone();
    let tr_scene = compile_graph(&xy_from_dirty, &doc).unwrap();
    assert!(tr_scene.plotted_points.is_empty(), "stale Dirty analyzable_data points must not be plotted");
    assert!(
        tr_scene.warnings.iter().any(|w| w.code == "pending_source_result"),
        "expected pending_source_result warning, got {:?}",
        tr_scene.warnings
    );
    assert!(!tr_scene.warnings.iter().any(|w| w.code == "missing_source_sheet"));

    // Column graph sourced from a Dirty analyzable_data ResultSheet also warns `pending_source_result`.
    let mut col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    col_tbl.data_sets[0].cells[0][0] = Cell::number(5.0);
    col_tbl.data_sets[0].cells[1][0] = Cell::number(7.0);
    let col_ds = DataSheet::new("col-derived", "Col Derived", TableArchetype::Column(col_tbl)).unwrap();
    let col_spec = AnalysisSpec::new("res-col-dirty", "d-xy", AnalysisKind::DescriptiveStats(Default::default())).unwrap();
    let mut col_res = ResultSheet::new(col_spec).unwrap();
    col_res.status = ResultStatus::Dirty;
    col_res.analyzable_data = Some(col_ds.clone());
    doc.add_result_sheet(col_res).unwrap();
    let mut col_graph = GraphSheet::new_uninitialized_for_data("g-col-dirty", &col_ds).unwrap();
    col_graph.spec.primary_source_sheet_id = SheetId::new("res-col-dirty");
    col_graph.spec.series[0].source_sheet_id = SheetId::new("res-col-dirty");
    let col_scene = compile_graph(&col_graph, &doc).unwrap();
    assert!(col_scene.warnings.iter().any(|w| w.code == "pending_source_result"), "col warnings: {:?}", col_scene.warnings);
    assert!(!col_scene.warnings.iter().any(|w| w.code == "missing_source_sheet"));

    // PartsOfWhole graph chained through an UpToDate ResultSheet onto a Dirty upstream ResultSheet omits stale data and warns `pending_analysis_result`.
    let pow_chain_spec = AnalysisSpec::new("res-pow-chain", "res-col-dirty", AnalysisKind::DescriptiveStats(Default::default())).unwrap();
    let mut pow_chain_res = ResultSheet::new(pow_chain_spec).unwrap();
    pow_chain_res.status = ResultStatus::UpToDate;
    doc.add_result_sheet(pow_chain_res).unwrap();
    let mut pow_graph = GraphSheet::new_uninitialized_for_data("g-pow-chain", &col_ds).unwrap();
    pow_graph.spec.kind = curvebook_doc::GraphKind::PartsOfWhole;
    pow_graph.spec.primary_source_sheet_id = SheetId::new("res-pow-chain");
    let pow_scene = compile_graph(&pow_graph, &doc).unwrap();
    assert!(pow_scene.warnings.iter().any(|w| w.code == "pending_analysis_result"), "pow warnings: {:?}", pow_scene.warnings);

    // Once UpToDate, resolve_data_sheet and plotting resume cleanly.
    doc.result_sheet_mut(&tr_id).unwrap().status = ResultStatus::UpToDate;
    assert!(resolve_data_sheet(&doc, &tr_id).is_some());
    assert!(resolve_data_sheet_or_upstream(&doc, &tr_id).is_some());
    let ok_scene = compile_graph(&xy_from_dirty, &doc).unwrap();
    assert_eq!(ok_scene.plotted_points.len(), 2);
}

#[test]
fn gap_01_04_broken_axis_auto_segments_partition_samples_and_bounds_cleanly() {
    let plot = curvebook_chart::RectPt::new(50.0, 20.0, 240.0, 180.0);

    // 1. Bounds path (2 and 3 auto segments over [0, 100]): contiguous partition, zero intermediate data dropped.
    for seg_count in [2usize, 3] {
        let mut spec = AxisSpec::linear("Survival Time");
        spec.segments = vec![AxisSegment::default(); seg_count];
        let mut w = Vec::new();
        let resolved = ResolvedAxis::resolve(&spec, AxisOrientation::Horizontal, plot, Some((0.0, 100.0)), &mut w);
        assert_eq!(resolved.segments.len(), seg_count);
        for i in 0..seg_count {
            assert!(resolved.segments[i].domain_min < resolved.segments[i].domain_max);
            if i + 1 < seg_count {
                assert!(
                    (resolved.segments[i].domain_max - resolved.segments[i + 1].domain_min).abs() < 1e-9,
                    "segments {i} and {} should partition contiguously: {:?}",
                    i + 1,
                    resolved.segments
                );
            }
        }
        // Every intermediate value in [0, 100] must map cleanly (not dropped!).
        for v in [0.0, 15.0, 33.0, 50.0, 67.0, 85.0, 100.0] {
            assert!(resolved.map_value(v).is_some(), "value {v} dropped on {seg_count}-segment bounds axis: {:?}", resolved.segments);
        }
        // Points span multiple distinct segments (not all crammed into seg 0).
        assert_eq!(resolved.map_value(5.0).unwrap().segment_index, 0);
        assert_eq!(resolved.map_value(95.0).unwrap().segment_index, seg_count - 1);
    }

    // 2. Samples path (2 and 3 auto segments over [0, 20, 40, 60, 80, 100]): no duplicate full-domain segments!
    let samples = [0.0, 20.0, 40.0, 60.0, 80.0, 100.0];
    for seg_count in [2usize, 3] {
        let mut spec = AxisSpec::linear("Y");
        spec.segments = vec![AxisSegment::default(); seg_count];
        let mut w = Vec::new();
        let resolved = ResolvedAxis::resolve_from_samples(&spec, AxisOrientation::Vertical, (200.0, 20.0), &samples, false, 0.05, &mut w);
        assert_eq!(resolved.segments.len(), seg_count);
        for i in 0..seg_count {
            assert!(resolved.segments[i].domain_min < resolved.segments[i].domain_max);
            if i + 1 < seg_count {
                assert!(
                    resolved.segments[i].domain_max <= resolved.segments[i + 1].domain_min + 1e-9,
                    "segments must not duplicate/overlap domains: {:?}",
                    resolved.segments
                );
            }
        }
        for &v in &samples {
            assert!(resolved.map_value(v).is_some(), "sample {v} dropped on {seg_count}-segment sample axis");
        }
        assert_eq!(resolved.map_value(0.0).unwrap().segment_index, 0);
        assert_eq!(resolved.map_value(100.0).unwrap().segment_index, seg_count - 1);
    }

    // 3. Log10 2-segment auto axis splits in log space cleanly.
    let mut log_spec = AxisSpec::linear("Conc");
    log_spec.segments =
        vec![AxisSegment { scale: AxisScale::Log10, ..AxisSegment::default() }, AxisSegment { scale: AxisScale::Log10, ..AxisSegment::default() }];
    let mut lw = Vec::new();
    let log_res = ResolvedAxis::resolve_from_samples(
        &log_spec,
        AxisOrientation::Horizontal,
        (50.0, 290.0),
        &[1.0, 10.0, 100.0, 1000.0, 10_000.0],
        false,
        0.0,
        &mut lw,
    );
    assert!((log_res.segments[0].domain_min - 1.0).abs() < 1e-9);
    assert!((log_res.segments[0].domain_max - 100.0).abs() < 1e-6, "expected log midpoint ~100, got {:?}", log_res.segments);
    assert!((log_res.segments[1].domain_min - 100.0).abs() < 1e-6);
    assert!((log_res.segments[1].domain_max - 10_000.0).abs() < 1e-6);

    // 4. Mixed explicit seg 0 `[0, 10]` + auto seg 1 `(None, None)` over `[1, 5, 50, 90]`.
    let mut mixed_spec = AxisSpec::linear("Y");
    mixed_spec.segments = vec![AxisSegment { min: Some(0.0), max: Some(10.0), ..AxisSegment::default() }, AxisSegment::default()];
    let mut mw = Vec::new();
    let mixed =
        ResolvedAxis::resolve_from_samples(&mixed_spec, AxisOrientation::Vertical, (200.0, 20.0), &[1.0, 5.0, 50.0, 90.0], false, 0.05, &mut mw);
    assert_eq!((mixed.segments[0].domain_min, mixed.segments[0].domain_max), (0.0, 10.0));
    assert!((mixed.segments[1].domain_min - 10.0).abs() < 1e-9 && mixed.segments[1].domain_max >= 90.0, "got {:?}", mixed.segments);
    assert_eq!(mixed.map_value(5.0).unwrap().segment_index, 0);
    assert_eq!(mixed.map_value(50.0).unwrap().segment_index, 1);
    assert_eq!(mixed.map_value(90.0).unwrap().segment_index, 1);
}

#[test]
fn gap_01_05_series_offset_shifts_fitted_curve_bands_and_domain() {
    let (mut doc, mut graph) = build_xy_doc_with_fit(&[(1.0, 10.0), (2.0, 20.0), (3.0, 30.0), (4.0, 40.0)]);
    let spec = AnalysisSpec::new("fit-off", "d-xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).unwrap();
    let mut res = ResultSheet::new(spec).unwrap();
    res.status = ResultStatus::UpToDate;
    res.scalars.push(ResultScalarEntry { key: String::from("R2"), data_set_index: Some(0), value: 0.999, units: None });
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("straight_line"),
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 1.0, y: 10.0, ci_lower: Some(8.0), ci_upper: Some(12.0), pred_lower: Some(5.0), pred_upper: Some(15.0) },
            CurveSamplePoint { x: 4.0, y: 40.0, ci_lower: Some(38.0), ci_upper: Some(42.0), pred_lower: Some(35.0), pred_upper: Some(45.0) },
        ],
    });
    doc.add_result_sheet(res).unwrap();

    let curve_color = Rgba8::rgb(12, 99, 211);
    graph.spec.x_axis.segments[0].min = Some(0.0);
    graph.spec.x_axis.segments[0].max = Some(20.0);
    graph.spec.y_axis.segments[0].min = Some(0.0);
    graph.spec.y_axis.segments[0].max = Some(60.0);
    graph.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
        marker: Some(MarkerStyle {
            shape: MarkerShape::Circle,
            fill_mode: MarkerFillMode::Filled,
            size_pt: 5.0,
            fill_color: curve_color,
            stroke_color: curve_color,
            stroke_width_pt: 1.0,
        }),
        line: None,
        area_fill: None,
        replicate_lines: Default::default(),
    };
    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("fit-off"),
        sample_segments: 256,
        show_confidence_band: true,
        show_prediction_band: true,
        shaded_ribbon: true,
    });

    let base_scene = compile_graph(&graph, &doc).unwrap();
    let extract_curve_bounds = |scene: &ChartScene| -> (f64, f64, f64, f64) {
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for prim in &scene.primitives {
            if let ScenePrimitive::Path { path, fill: None, stroke: Some(s), clip: Some(_) } = prim
                && s.color == curve_color
                && (s.width_pt - 1.75).abs() < 1e-6
            {
                for cmd in &path.commands {
                    if let PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) = *cmd {
                        min_x = min_x.min(pt.x);
                        max_x = max_x.max(pt.x);
                        min_y = min_y.min(pt.y);
                        max_y = max_y.max(pt.y);
                    }
                }
            }
        }
        (min_x, max_x, min_y, max_y)
    };
    let extract_ci_bounds = |scene: &ChartScene| -> (f64, f64, f64, f64) {
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for prim in &scene.primitives {
            if let ScenePrimitive::Path { path, fill: Some(f), stroke: None, clip: Some(_) } = prim
                && f.color == curve_color.with_alpha(68)
            {
                for cmd in &path.commands {
                    if let PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) = *cmd {
                        min_x = min_x.min(pt.x);
                        max_x = max_x.max(pt.x);
                        min_y = min_y.min(pt.y);
                        max_y = max_y.max(pt.y);
                    }
                }
            }
        }
        (min_x, max_x, min_y, max_y)
    };

    let (bx0, bx1, by0, by1) = extract_curve_bounds(&base_scene);
    let (bci_x0, bci_x1, bci_y0, bci_y1) = extract_ci_bounds(&base_scene);
    assert!(bx0.is_finite() && bci_x0.is_finite());

    // Shift by x_data = +10.0, x_pt = +12.0, y_pt = +8.0 (visually upward = -8.0 in page Y).
    graph.spec.series[0].offset = SeriesOffsetSpec { x_data: 10.0, x_pt: 12.0, y_pt: 8.0 };
    let shifted_scene = compile_graph_with_options(
        &graph,
        &doc,
        &CompileOptions { fit_summary: FitSummarySpec { enabled: true, placement: FitSummaryPlacement::AutoInset, ..FitSummarySpec::default() } },
    )
    .unwrap();
    let (sx0, sx1, sy0, sy1) = extract_curve_bounds(&shifted_scene);
    let (sci_x0, sci_x1, sci_y0, sci_y1) = extract_ci_bounds(&shifted_scene);

    let px_per_x = base_scene.plot_rect_pt.width / 20.0;
    let expected_dx = 10.0 * px_per_x + 12.0;
    let expected_dy = -8.0;
    assert!(((sx0 - bx0) - expected_dx).abs() < 1.0, "curve min_x shift: got {}, expected {expected_dx}", sx0 - bx0);
    assert!(((sx1 - bx1) - expected_dx).abs() < 1.0, "curve max_x shift: got {}, expected {expected_dx}", sx1 - bx1);
    assert!(((sy0 - by0) - expected_dy).abs() < 0.5, "curve min_y shift: got {}, expected {expected_dy}", sy0 - by0);
    assert!(((sy1 - by1) - expected_dy).abs() < 0.5, "curve max_y shift: got {}, expected {expected_dy}", sy1 - by1);
    assert!(((sci_x0 - bci_x0) - expected_dx).abs() < 1.0, "CI min_x shift mismatch");
    assert!(((sci_x1 - bci_x1) - expected_dx).abs() < 1.0, "CI max_x shift mismatch");
    assert!(((sci_y0 - bci_y0) - expected_dy).abs() < 0.5, "CI min_y shift mismatch");
    assert!(((sci_y1 - bci_y1) - expected_dy).abs() < 0.5, "CI max_y shift mismatch");

    // Also verify auto-range X axis expands to contain a fit-only (`show_raw_data = false`) offset curve.
    graph.spec.x_axis.segments[0].min = None;
    graph.spec.x_axis.segments[0].max = None;
    graph.spec.series[0].show_raw_data = false;
    graph.spec.series[0].offset = SeriesOffsetSpec { x_data: 100.0, x_pt: 0.0, y_pt: 0.0 };
    let auto_scene = compile_graph(&graph, &doc).unwrap();
    let (ax0, ax1, _, _) = extract_curve_bounds(&auto_scene);
    assert!(ax0.is_finite() && ax1 > ax0, "auto-range X axis must cover offset fitted curve when show_raw_data=false");
}

#[test]
fn gap_01_06_fit_summary_swatch_uses_resolved_series_line_or_marker_color() {
    let mut doc = ProjectDocument::new("GAP01Swatch").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 3).unwrap();
    xy.data_sets[0].title = String::from("LineColorSeries");
    xy.data_sets[1].title = String::from("MarkerColorSeries");
    for r in 0..3 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 2.0);
        xy.data_sets[1].cells[r][0] = Cell::number((r + 1) as f64 * 3.0);
    }
    let ds = DataSheet::new("d-sw", "XY", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();

    let spec = AnalysisSpec::new("fit-sw", "d-sw", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).unwrap();
    let mut res = ResultSheet::new(spec).unwrap();
    res.status = ResultStatus::UpToDate;
    for s in 0..2 {
        res.scalars.push(ResultScalarEntry { key: String::from("R2"), data_set_index: Some(s), value: 0.99, units: None });
        res.fitted_curves.push(FittedCurveOverlay {
            data_set_index: s,
            equation_id: String::from("straight_line"),
            converged: true,
            ambiguous: false,
            points: vec![
                CurveSamplePoint { x: 1.0, y: 2.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
                CurveSamplePoint { x: 3.0, y: 6.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            ],
        });
    }
    doc.add_result_sheet(res).unwrap();

    let custom_line = Rgba8::rgb(17, 133, 99);
    let custom_marker = Rgba8::rgb(199, 33, 144);
    let mut graph = GraphSheet::new_uninitialized_for_data("g-sw", &ds).unwrap();
    graph.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
        marker: Some(MarkerStyle { stroke_color: Rgba8::rgb(5, 5, 5), fill_color: Rgba8::rgb(5, 5, 5), ..MarkerStyle::default() }),
        line: Some(ConnectingLineStyle {
            kind: LineConnectionKind::PointToPoint,
            color: custom_line,
            stroke_width_pt: 1.5,
            dash_style: LineDashStyle::Solid,
        }),
        area_fill: None,
        replicate_lines: Default::default(),
    };
    graph.spec.series[1].gdo = GraphicalDataObject::ScatterAndLine {
        marker: Some(MarkerStyle { stroke_color: custom_marker, fill_color: Rgba8::rgb(250, 250, 250), ..MarkerStyle::default() }),
        line: None,
        area_fill: None,
        replicate_lines: Default::default(),
    };
    for s in 0..2 {
        graph.spec.series[s].regression_overlay = Some(RegressionOverlaySpec {
            result_sheet_id: SheetId::new("fit-sw"),
            sample_segments: 64,
            show_confidence_band: false,
            show_prediction_band: false,
            shaded_ribbon: false,
        });
    }

    let entries = extract_fit_summary_entries(&graph, &doc, 2);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].color, custom_line, "swatch 0 must match series line color, not palette.color_at(0)");
    assert_eq!(entries[1].color, custom_marker, "swatch 1 must match series marker stroke color, not palette.color_at(1)");
    assert_ne!(custom_line, graph.spec.palette.color_at(0));
    assert_ne!(custom_marker, graph.spec.palette.color_at(1));
}

#[test]
fn gap_01_07_major_tick_interval_honored_on_log_prob_scales_and_log10_minor_ticks_disable() {
    let policy = LocaleNumberPolicy::publication();
    let plot = curvebook_chart::RectPt::new(50.0, 20.0, 280.0, 200.0);

    let majors_for = |scale: AxisScale, min: f64, max: f64, step: f64| -> (Vec<f64>, Vec<curvebook_chart::ChartWarning>) {
        let mut spec = AxisSpec::linear("Axis");
        spec.segments = vec![AxisSegment {
            min: Some(min),
            max: Some(max),
            scale,
            length_share: 1.0,
            major_tick_interval: Some(step),
            minor_ticks_per_interval: 0,
        }];
        let mut w = Vec::new();
        let resolved = ResolvedAxis::resolve(&spec, AxisOrientation::Horizontal, plot, None, &mut w);
        let ticks = compute_axis_ticks_with_warnings(
            &resolved,
            TickComputeConfig { tick_format: TickLabelFormat::Decimal, policy: &policy, auto_log_minor_ticks: false, categorical_ticks: None },
            &mut w,
        );
        (ticks.into_iter().filter(|t| t.kind == TickKind::Major).map(|t| t.value).collect(), w)
    };

    // Log10 decade step = 2 -> 1, 100, 10000, 1e6
    let (log10_m, _) = majors_for(AxisScale::Log10, 1.0, 1e6, 2.0);
    assert_eq!(log10_m.len(), 4, "Log10 step=2 got {log10_m:?}");
    for (got, exp) in log10_m.iter().zip([1.0, 100.0, 10_000.0, 1e6]) {
        assert!((got - exp).abs() / exp < 1e-12, "got {got}, exp {exp}");
    }

    // Log2 exponent step = 2 -> 1, 4, 16, 64
    let (log2_m, _) = majors_for(AxisScale::Log2, 1.0, 64.0, 2.0);
    assert_eq!(log2_m, vec![1.0, 4.0, 16.0, 64.0]);

    // Ln exponent step = 2 -> e^0, e^2, e^4, e^6
    let e = std::f64::consts::E;
    let (ln_m, _) = majors_for(AxisScale::Ln, 1.0, e.powi(6), 2.0);
    assert_eq!(ln_m.len(), 4, "Ln step=2 got {ln_m:?}");
    for (got, exp) in ln_m.iter().zip([1.0, e.powi(2), e.powi(4), e.powi(6)]) {
        assert!((got - exp).abs() / exp < 1e-12, "got {got}, exp {exp}");
    }

    // Logit Percent step = 20 -> 20, 40, 60, 80
    let (logit_pct_m, _) = majors_for(AxisScale::Logit(LogitDomain::Percent), 10.0, 90.0, 20.0);
    assert_eq!(logit_pct_m, vec![20.0, 40.0, 60.0, 80.0]);

    // Logit Proportion step = 0.2 -> 0.2, 0.4, 0.6, 0.8
    let (logit_prop_m, _) = majors_for(AxisScale::Logit(LogitDomain::Proportion), 0.1, 0.9, 0.2);
    assert_eq!(logit_prop_m.len(), 4, "Logit(Prop) got {logit_prop_m:?}");
    for (got, exp) in logit_prop_m.iter().zip([0.2, 0.4, 0.6, 0.8]) {
        assert!((got - exp).abs() < 1e-12);
    }

    // Probit step = 20 -> 20, 40, 60, 80
    let (probit_m, _) = majors_for(AxisScale::Probit, 10.0, 90.0, 20.0);
    assert_eq!(probit_m, vec![20.0, 40.0, 60.0, 80.0]);

    // Adversarial tiny interval on Log10 and Probit caps cleanly with `major_tick_interval_capped`.
    for (scale, lo, hi) in [(AxisScale::Log10, 1.0, 1000.0), (AxisScale::Probit, 1.0, 99.0)] {
        let (ticks, w) = majors_for(scale, lo, hi, 1e-12);
        assert!(ticks.len() <= MAX_TICKS_PER_SEGMENT);
        assert!(w.iter().any(|x| x.code == "major_tick_interval_capped"), "{scale:?} warnings: {w:?}");
    }

    // Live XY compile_graph: setting `minor_ticks_per_interval = 0` on Log10 disables minor ticks; `8` enables them.
    let (doc, mut graph) = build_xy_doc_with_fit(&[(1.0, 1.0), (2.0, 10.0), (3.0, 100.0)]);
    graph.spec.y_axis.segments = vec![AxisSegment {
        min: Some(1.0),
        max: Some(1000.0),
        scale: AxisScale::Log10,
        length_share: 1.0,
        major_tick_interval: None,
        minor_ticks_per_interval: 0,
    }];
    let scene_no_minor = compile_graph(&graph, &doc).unwrap();
    graph.spec.y_axis.segments[0].minor_ticks_per_interval = 8;
    let scene_with_minor = compile_graph(&graph, &doc).unwrap();

    // Count horizontal tick strokes protruding left of `plot_rect_pt.x` (minor length = 2.2 pt, major length = 4.0 pt).
    let count_left_y_minor_ticks = |scene: &ChartScene| -> usize {
        let x_spine = scene.plot_rect_pt.x;
        scene
            .primitives
            .iter()
            .filter(|p| match p {
                ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None } if path.commands.len() == 2 => {
                    if let (PathCmd::MoveTo(a), PathCmd::LineTo(b)) = (path.commands[0], path.commands[1]) {
                        let horiz = (a.y - b.y).abs() < 1e-6;
                        let at_spine = (a.x - x_spine).abs() < 1e-6 || (b.x - x_spine).abs() < 1e-6;
                        let len = (a.x - b.x).abs();
                        horiz && at_spine && (len - 2.2).abs() < 0.2
                    } else {
                        false
                    }
                }
                _ => false,
            })
            .count()
    };
    assert_eq!(count_left_y_minor_ticks(&scene_no_minor), 0, "minor_ticks_per_interval=0 must turn off Log10 minor ticks");
    assert_eq!(count_left_y_minor_ticks(&scene_with_minor), 24, "minor_ticks_per_interval=8 over 3 decades must draw 24 minor ticks");
}

#[test]
fn gap_01_08_engineering_si_extreme_values_fallback_to_scientific_notation() {
    let policy = LocaleNumberPolicy::publication();
    let fmt = |v: f64| format_tick_rich(v, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy).plain_text();

    // Non-zero values below yocto (1e-24) must never collapse to "0 y".
    for (v, expected) in [
        (1e-28, "1e\u{2212}28"),
        (-1e-28, "\u{2212}1e\u{2212}28"),
        (5e-28, "5e\u{2212}28"),
        (1e-30, "1e\u{2212}30"),
        (5e-25, "5e\u{2212}25"),
        (1e27, "1e27"),
        (-1e30, "\u{2212}1e30"),
        (999.9996e24, "1e27"),
    ] {
        let got = fmt(v);
        assert_eq!(got, expected, "value {v:e}");
        assert!(!got.contains("0 y"), "value {v:e} collapsed to {got:?}");
    }
    let subnormal = fmt(5e-324);
    assert!(subnormal.contains("e\u{2212}324") && !subnormal.contains("0 y"), "subnormal got {subnormal:?}");

    // Boundary and in-range SI values remain exact.
    assert_eq!(fmt(0.0), "0");
    assert_eq!(fmt(1e-24), "1 y");
    assert_eq!(fmt(999.9996e-27), "1 y");
    assert_eq!(fmt(999.99), "999.99");
    assert_eq!(fmt(999.9996), "1 k");
    assert_eq!(fmt(999.999e24), "999.999 Y");
}

#[test]
fn analyzable_data_chaining_across_mv_survival_contingency_pow_and_diagnostics() {
    let mut doc = ProjectDocument::new("AnalyzableChaining").expect("doc");
    let wrap_tx = |id: &str, src: &str, ds: DataSheet| {
        let mut r = ResultSheet::new(
            AnalysisSpec::new(
                id,
                src,
                AnalysisKind::Transform(TransformAnalysisSpec::Math {
                    x_transform: None,
                    y_transform: None,
                    error_propagation: ErrorPropagationMode::default(),
                }),
            )
            .expect("spec"),
        )
        .expect("res");
        r.status = ResultStatus::UpToDate;
        r.analyzable_data = Some(ds);
        r
    };
    let build_mv = |id: &str, title: &str, rows: &[(f64, f64, &str)]| {
        let mut mv = MultipleVariablesTable::new(3, rows.len()).expect("mv");
        for (i, t) in ["Age", "Response", "Group"].into_iter().enumerate() {
            mv.variables[i].title = String::from(t);
        }
        mv.variables[2].lock_role(VariableRole::Categorical);
        for (r, &(age, resp, grp)) in rows.iter().enumerate() {
            mv.variables[0].cells[r] = Cell::number(age);
            mv.variables[1].cells[r] = Cell::number(resp);
            mv.variables[2].cells[r] = Cell::text(grp);
        }
        DataSheet::new(id, title, TableArchetype::MultipleVariables(mv)).expect("mv ds")
    };

    // 1. MultipleVariables: 6 raw rows vs 3 filtered adult rows in analyzable_data.
    let raw_mv = build_mv(
        "mv-raw",
        "Raw Patients",
        &[(5.0, 1.0, "Ped"), (8.0, 2.0, "Ped"), (12.0, 3.0, "Ped"), (25.0, 40.0, "Adult"), (35.0, 55.0, "Adult"), (45.0, 70.0, "Adult")],
    );
    let filt_mv = build_mv("mv-filt-data", "Adult Only", &[(25.0, 40.0, "Adult"), (35.0, 55.0, "Adult"), (45.0, 70.0, "Adult")]);
    doc.add_data_sheet(raw_mv).expect("add raw mv");
    doc.add_result_sheet(wrap_tx("res-mv-filt", "mv-raw", filt_mv.clone())).expect("add res mv filt");

    // 1a. BubbleScatter plots 3 filtered points.
    let mut g_mv = GraphSheet::new_uninitialized_for_data("g-mv-filt", &filt_mv).expect("g mv");
    g_mv.spec.primary_source_sheet_id = SheetId::new("res-mv-filt");
    g_mv.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Constant(Rgba8::BLACK),
        size: SizeChannelMapping::Constant { radius_pt: 4.5 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    assert_eq!(compile_graph(&g_mv, &doc).expect("compile mv").plotted_points.len(), 3);

    // 1b. NormalQqPlot, ResidualPlot, Volcano, BlandAltman on transformed ResultSheet plot 3 filtered points.
    for dkind in [DiagnosticPlotKind::NormalQqPlot, DiagnosticPlotKind::ResidualPlot, DiagnosticPlotKind::Volcano, DiagnosticPlotKind::BlandAltman] {
        let mut g_diag = GraphSheet::new_uninitialized_for_data(format!("g-diag-{dkind:?}"), &filt_mv).expect("g diag");
        g_diag.spec.kind = GraphKind::Diagnostic(dkind);
        g_diag.spec.primary_source_sheet_id = SheetId::new("res-mv-filt");
        g_diag.spec.multivariate = None;
        let d_scene = compile_graph(&g_diag, &doc).expect("compile diag");
        assert_eq!(d_scene.plotted_points.len(), 3, "{dkind:?} plotted wrong count, warnings={:?}", d_scene.warnings);
    }

    // 1c. 2-hop chain Heatmap (raw -> res-mv-filt [analyzable] -> res-mv-hop2 [analyzable_data=None]) plots 3 rows.
    let mut hop2 =
        ResultSheet::new(AnalysisSpec::new("res-mv-hop2", "res-mv-filt", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("spec"))
            .expect("hop2");
    hop2.status = ResultStatus::UpToDate;
    doc.add_result_sheet(hop2).expect("add hop2");
    let mut g_hm = GraphSheet::new_uninitialized_for_data("g-hm-hop2", &filt_mv).expect("g hm");
    g_hm.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    g_hm.spec.primary_source_sheet_id = SheetId::new("res-mv-hop2");
    g_hm.spec.heatmap_labels.annotate_cells = true;
    let hm_scene = compile_graph(&g_hm, &doc).expect("compile hm");
    assert!(hm_scene.warnings.is_empty(), "warnings={:?}", hm_scene.warnings);
    let hm_texts = scene_texts(&hm_scene);
    assert!(hm_texts.iter().any(|t| t.contains("40")) && hm_texts.iter().all(|t| t != "5"));

    // 1d. ForestPlot on transformed Grouped(UpperLowerLimits).
    let mk_forest = |id: &str, title: &str, labels: [&str; 2], rows: [[f64; 3]; 2]| {
        let mut g = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 1, 2).expect("for");
        g.row_titles = labels.into_iter().map(String::from).collect();
        for (r, vals) in rows.into_iter().enumerate() {
            g.data_sets[0].cells[r] = vals.into_iter().map(Cell::number).collect();
        }
        DataSheet::new(id, title, TableArchetype::Grouped(g)).expect("for ds")
    };
    let raw_for = mk_forest("for-raw", "RawForest", ["RawStudyA", "RawStudyB"], [[1.0, 0.8, 1.2], [2.0, 1.5, 2.5]]);
    let tx_for = mk_forest("for-tx-data", "CleanForest", ["CleanTrialX", "CleanTrialY"], [[0.6, 0.4, 0.9], [0.7, 0.5, 0.95]]);
    doc.add_data_sheet(raw_for).expect("add raw for");
    doc.add_result_sheet(wrap_tx("res-for-tx", "for-raw", tx_for.clone())).expect("add res for");
    let mut g_for = GraphSheet::new_uninitialized_for_data("g-for-tx", &tx_for).expect("g for");
    g_for.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
    g_for.spec.primary_source_sheet_id = SheetId::new("res-for-tx");
    let for_joined = scene_texts(&compile_graph(&g_for, &doc).expect("compile forest")).join(" | ");
    assert!(for_joined.contains("CleanTrialX") && !for_joined.contains("RawStudyA"), "for_joined={for_joined}");

    // 1e. ROC on transformed Column(Controls, Cases) uses transformed values (AUC=1.000, not raw 0.000).
    let mk_roc = |id: &str, title: &str, pairs: [(f64, f64); 4]| {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).expect("roc");
        for (r, (c, k)) in pairs.into_iter().enumerate() {
            col.data_sets[0].cells[r][0] = Cell::number(c);
            col.data_sets[1].cells[r][0] = Cell::number(k);
        }
        DataSheet::new(id, title, TableArchetype::Column(col)).expect("roc ds")
    };
    let raw_roc = mk_roc("roc-raw", "RawROC", [(10.0, 1.0), (11.0, 2.0), (12.0, 3.0), (13.0, 4.0)]);
    let tx_roc = mk_roc("roc-tx-data", "TransformedROC", [(1.0, 10.0), (2.0, 11.0), (3.0, 12.0), (4.0, 13.0)]);
    doc.add_data_sheet(raw_roc).expect("add raw roc");
    doc.add_result_sheet(wrap_tx("res-roc-tx", "roc-raw", tx_roc.clone())).expect("add res roc");
    let mut g_roc = GraphSheet::new_uninitialized_for_data("g-roc-tx", &tx_roc).expect("g roc");
    g_roc.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve);
    g_roc.spec.primary_source_sheet_id = SheetId::new("res-roc-tx");
    assert!(scene_texts(&compile_graph(&g_roc, &doc).expect("compile roc")).iter().any(|t| t.contains("AUC = 1.000")));

    // 2. Survival, 3. Contingency, and 4. PartsOfWhole on transformed ResultSheets.
    let mut surv = SurvivalTable::new(2, 5).expect("surv");
    surv.cohorts[0].title = String::from("FiltVehicle");
    surv.cohorts[1].title = String::from("FiltDrug");
    for (r, (t, s0, s1)) in [(2.0, 1.0, 0.0), (4.0, 1.0, 1.0), (6.0, 0.0, 0.0), (8.0, 1.0, 1.0), (10.0, 1.0, 0.0)].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(s0);
        surv.cohorts[1].cells[r][0] = Cell::number(s1);
    }
    let mut cont = ContingencyTable::new(2, 2).expect("cont");
    cont.row_titles = vec![String::from("FilteredPos"), String::from("FilteredNeg")];
    cont.data_sets[0].title = String::from("Placebo");
    cont.data_sets[1].title = String::from("Active");
    cont.data_sets[0].cells = vec![vec![Cell::number(12.0)], vec![Cell::number(28.0)]];
    cont.data_sets[1].cells = vec![vec![Cell::number(30.0)], vec![Cell::number(10.0)]];
    let mut pow = PartsOfWholeTable::new(1, 3).expect("pow");
    pow.slice_labels = vec![String::from("AlphaPart"), String::from("BetaPart"), String::from("GammaPart")];
    pow.data_sets[0].cells = vec![vec![Cell::number(50.0)], vec![Cell::number(30.0)], vec![Cell::number(20.0)]];

    for (res_id, ds, needle) in [
        ("res-surv-tx", DataSheet::new("surv-tx-data", "Filtered OS", TableArchetype::Survival(surv)).expect("surv ds"), "FiltDrug"),
        ("res-cont-tx", DataSheet::new("cont-tx-data", "Filtered Cont", TableArchetype::Contingency(cont)).expect("cont ds"), "FilteredPos"),
        ("res-pow-tx", DataSheet::new("pow-tx-data", "Filtered PoW", TableArchetype::PartsOfWhole(pow)).expect("pow ds"), "AlphaPart (50.0%)"),
    ] {
        doc.add_result_sheet(wrap_tx(res_id, "mv-raw", ds.clone())).expect("add tx res");
        let mut g = GraphSheet::new_uninitialized_for_data(format!("g-{res_id}"), &ds).expect("g");
        g.spec.primary_source_sheet_id = SheetId::new(res_id);
        let sc = compile_graph(&g, &doc).expect("compile tx graph");
        assert!(sc.warnings.is_empty(), "{res_id} warnings={:?}", sc.warnings);
        assert!(scene_texts(&sc).iter().any(|t| t.contains(needle)), "{res_id} missing `{needle}`");
    }

    // 5. Failed transform ResultSheet must NEVER leak raw upstream data.
    let mut failed_res = wrap_tx("res-mv-failed", "mv-raw", filt_mv.clone());
    failed_res.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("filter failed") };
    doc.add_result_sheet(failed_res).expect("add failed res");
    let mut g_fail = GraphSheet::new_uninitialized_for_data("g-diag-fail", &filt_mv).expect("g fail");
    g_fail.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
    g_fail.spec.primary_source_sheet_id = SheetId::new("res-mv-failed");
    assert!(compile_graph(&g_fail, &doc).expect("compile fail").plotted_points.is_empty());
}
