//! Regression tests covering FA-SEC09-06..13 and the doc/chart side of FA-X1-03.
#![allow(clippy::float_cmp, clippy::cast_lossless)]

use std::io::Cursor;

use curvebook_chart::scene::{ChartScene, PathCmd, ScenePrimitive};
use curvebook_chart::{compile_graph, compile_layout};
use curvebook_doc::graph::MAX_GRAPH_ANNOTATIONS;
use curvebook_doc::result::ErrorCode;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AnnotationShapeKind, AxisScale, AxisSegment, BlandAltmanSpec, BracketLabelMode, BracketStyle, CategoricalTickSource,
    Cell, ColumnDesignHint, ColumnTable, CompactLetterDisplaySpec, ContingencyTable, CropInsetsPt, CurveSamplePoint, DataSheet, DescriptiveSpec,
    DiagnosticPlotKind, EmbeddedTableStyle, FittedCurveOverlay, FramePreset, GraphAnnotation, GraphKind, GraphSheet, GraphicalDataObject,
    GroupedArrangement, GroupedTable, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, MAX_IMAGE_PIXELS, MultipleRowTestsSpec,
    MultiplicitySpec, NestedTable, NonlinearFitSpec, OneWayAnovaSpec, PairwiseBracketSpec, PairwiseComparisonRecord, PairwiseDisplayMode,
    ProjectDocument, ProjectImageAsset, RectPt as DocRectPt, RegressionOverlaySpec, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus,
    ResultTab, ResultTabKind, RocSpec, SheetId, SubcolumnMode, SummaryMeasure, SurvivalAnalysisSpec, SurvivalTable, TableArchetype, TickLabelFormat,
    WhiskerMethod, XyTable, content_addressed_asset_id,
};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use image::{DynamicImage, GrayImage, ImageFormat, Luma};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn encode_solid_png_rgba_asset(width: u32, height: u32, luma: u8) -> TestResult<ProjectImageAsset> {
    let gray = GrayImage::from_pixel(width, height, Luma([luma]));
    let mut bytes = Vec::new();
    DynamicImage::ImageLuma8(gray).write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)?;
    let asset_id = content_addressed_asset_id(&bytes);
    Ok(ProjectImageAsset { asset_id, media_type: curvebook_doc::ImageMimeKind::Png, pixel_size: (width, height), bytes })
}

fn all_points_finite(scene: &ChartScene) -> bool {
    for prim in &scene.primitives {
        match prim {
            ScenePrimitive::Path { path, .. } => {
                for cmd in &path.commands {
                    let ok = match *cmd {
                        PathCmd::MoveTo(p) | PathCmd::LineTo(p) => p.x.is_finite() && p.y.is_finite(),
                        PathCmd::QuadTo { control, to } => control.x.is_finite() && control.y.is_finite() && to.x.is_finite() && to.y.is_finite(),
                        PathCmd::CubicTo { c1, c2, to } => {
                            c1.x.is_finite() && c1.y.is_finite() && c2.x.is_finite() && c2.y.is_finite() && to.x.is_finite() && to.y.is_finite()
                        }
                        PathCmd::Close => true,
                    };
                    if !ok {
                        return false;
                    }
                }
            }
            ScenePrimitive::Text(t) => {
                if !t.anchor_pt.x.is_finite() || !t.anchor_pt.y.is_finite() || !t.font_size_pt.is_finite() || !t.rotation_deg.is_finite() {
                    return false;
                }
            }
            ScenePrimitive::RasterImage(img) => {
                if !img.bounds_pt.x.is_finite()
                    || !img.bounds_pt.y.is_finite()
                    || !img.bounds_pt.width.is_finite()
                    || !img.bounds_pt.height.is_finite()
                    || !img.rotation_deg.is_finite()
                {
                    return false;
                }
            }
            ScenePrimitive::PushClip { rect } => {
                if !rect.x.is_finite() || !rect.y.is_finite() || !rect.width.is_finite() || !rect.height.is_finite() {
                    return false;
                }
            }
            ScenePrimitive::PopClip => {}
        }
    }
    true
}

fn scene_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn simple_xy_doc() -> TestResult<(ProjectDocument, DataSheet, GraphSheet)> {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    for (r, (x, y)) in [(1.0, 2.0), (2.0, 4.0), (4.0, 8.0), (8.0, 16.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new(SheetId::new("xy-1"), "Calibration", TableArchetype::Xy(xy))?;
    let gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-1"), &ds)?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_graph_sheet(gs.clone())?;
    Ok((doc, ds, gs))
}

#[test]
fn fa_x1_03_max_graph_annotations_enforced_in_validate() -> TestResult {
    let (_doc, _ds, mut gs) = simple_xy_doc()?;
    let shape = GraphAnnotation::Shape {
        kind: AnnotationShapeKind::Rectangle,
        bounds_pt: DocRectPt::new(10.0, 10.0, 20.0, 20.0),
        stroke_color: Rgba8::BLACK,
        stroke_width_pt: 1.0,
        dash: LineDashStyle::Solid,
        fill_color: None,
    };
    gs.spec.annotations = vec![shape.clone(); MAX_GRAPH_ANNOTATIONS];
    assert!(gs.spec.validate().is_ok());

    gs.spec.annotations.push(shape);
    assert!(gs.spec.validate().is_err(), "exceeding MAX_GRAPH_ANNOTATIONS must fail validate()");
    Ok(())
}

#[test]
fn fa_x1_03_graph_and_layout_enforce_pixel_budget_across_clones() -> TestResult {
    let (mut doc, _ds, mut gs) = simple_xy_doc()?;
    let asset = encode_solid_png_rgba_asset(4096, 4096, 96)?;
    let asset_id = asset.asset_id.clone();
    doc.upsert_image_asset(asset)?;

    // 5 identical 4096x4096 references inside one GraphSheet -> 4 fit MAX_IMAGE_PIXELS, 5th hits budget.
    for i in 0..5 {
        gs.spec.annotations.push(GraphAnnotation::EmbeddedImage {
            asset_id: asset_id.clone(),
            bounds_pt: DocRectPt::new(20.0 + (i as f64) * 30.0, 20.0, 24.0, 24.0),
            rotation_deg: 0.0,
            lock_aspect_ratio: true,
            crop: CropInsetsPt::default(),
            border: None,
        });
    }
    let scene = compile_graph(&gs, &doc)?;
    assert!(
        scene.warnings.iter().any(|w| w.code == "embedded_image_budget_exceeded"),
        "expected embedded_image_budget_exceeded warning, got {:?}",
        scene.warnings
    );
    let rasters: Vec<_> = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::RasterImage(img) => Some(img),
            _ => None,
        })
        .collect();
    assert_eq!(rasters.len(), 4, "expected 4 materialized 4096x4096 rasters");
    let total_pixels: u64 = rasters.iter().map(|r| u64::from(r.pixel_size.0) * u64::from(r.pixel_size.1)).sum();
    assert!(total_pixels <= MAX_IMAGE_PIXELS);
    let total_rgba: usize = rasters.iter().map(|r| r.rgba.len()).sum();
    assert!(total_rgba <= (MAX_IMAGE_PIXELS as usize) * 4);

    // Panel cloning amplification guard in compile_layout:
    // Each graph has 1 4096x4096 image; 5 LiveGraph panels would materialize 5 * 16M > MAX_IMAGE_PIXELS.
    gs.spec.annotations.truncate(1);
    if let Some(slot) = doc.graph_sheet_mut(&gs.id) {
        *slot = gs.clone();
    }

    let mut layout = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    for idx in 0..5 {
        layout.items.push(LayoutItem::LiveGraph {
            id: format!("panel-{idx}"),
            graph_sheet_id: gs.id.clone(),
            bounds_pt: DocRectPt::new(10.0 + (idx as f64) * 80.0, 10.0, 75.0, 75.0),
            scale_percent: 100.0,
            badge_override: None,
        });
    }
    let layout_sheet = LayoutSheet::new(SheetId::new("lay-1"), "Budget Layout", layout)?;
    let layout_scene = compile_layout(&layout_sheet, &doc)?;
    assert!(
        layout_scene.warnings.iter().any(|w| w.code == "embedded_image_budget_exceeded"),
        "expected embedded_image_budget_exceeded on multi-panel layout, got {:?}",
        layout_scene.warnings
    );
    let layout_rasters: Vec<_> = layout_scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::RasterImage(img) => Some(img),
            _ => None,
        })
        .collect();
    assert_eq!(layout_rasters.len(), 4, "panel clones must cap total layout rasters at 4");
    let layout_rgba_bytes: usize = layout_rasters.iter().map(|r| r.rgba.len()).sum();
    assert!(layout_rgba_bytes <= 256 * 1024 * 1024);

    // Multiple LayoutItem::EmbeddedImage items must likewise enforce MAX_IMAGE_PIXELS.
    let mut img_layout = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    for idx in 0..5 {
        img_layout.items.push(LayoutItem::EmbeddedImage {
            id: format!("img-{idx}"),
            asset_id: asset_id.clone(),
            bounds_pt: DocRectPt::new(10.0 + (idx as f64) * 80.0, 10.0, 60.0, 60.0),
            rotation_deg: 0.0,
            lock_aspect_ratio: true,
            crop: CropInsetsPt::default(),
            border: None,
            badge_override: None,
        });
    }
    let img_layout_sheet = LayoutSheet::new(SheetId::new("lay-img"), "Image Layout", img_layout)?;
    let img_layout_scene = compile_layout(&img_layout_sheet, &doc)?;
    assert!(img_layout_scene.warnings.iter().any(|w| w.code == "embedded_image_budget_exceeded"));
    assert_eq!(img_layout_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::RasterImage(_))).count(), 4);
    Ok(())
}

#[test]
fn fa_sec09_12_budgets_finite_guards_rounded_rect_and_curve_segments() -> TestResult {
    let (mut doc, ds, mut gs) = simple_xy_doc()?;

    // 1. Huge 10,000-row result tab inside a small 120x45pt EmbeddedResultTable box must succeed
    // (only visible fitted rows count toward cell budget, not raw tab height).
    let mut big_res =
        ResultSheet::new(AnalysisSpec::new(SheetId::new("res-big"), ds.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    big_res.status = ResultStatus::UpToDate;
    big_res.tabs.push(ResultTab {
        id: String::from("main"),
        title: String::from("Main"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("A"), String::from("B"), String::from("C"), String::from("D")],
        row_labels: (0..10_000).map(|i| format!("R{i}")).collect(),
        rows: vec![
            vec![
                ResultCell::Number { value: 1.0, prefix: None, units: None },
                ResultCell::Number { value: 2.0, prefix: None, units: None },
                ResultCell::Number { value: 3.0, prefix: None, units: None },
                ResultCell::Number { value: 4.0, prefix: None, units: None },
            ];
            10_000
        ],
        footnotes: Vec::new(),
    });
    doc.add_result_sheet(big_res)?;

    let mut fit_res = ResultSheet::new(AnalysisSpec::new(
        SheetId::new("res-fit"),
        ds.id.clone(),
        AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("linear")),
    )?)?;
    fit_res.status = ResultStatus::UpToDate;
    fit_res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: true,
        ambiguous: false,
        points: (0..=20)
            .map(|i| {
                let x = 1.0 + (i as f64) * 0.35;
                CurveSamplePoint { x, y: 2.0 * x, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None }
            })
            .collect(),
    });
    doc.add_result_sheet(fit_res)?;

    gs.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-fit"),
        sample_segments: 900,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    gs.spec.legend.position_pt = Some((f64::INFINITY, 20.0));
    gs.spec.annotations = vec![
        GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("res-big"),
            tab_id: None,
            bounds_pt: DocRectPt::new(260.0, 20.0, 120.0, 45.0),
            selected_rows: None,
            selected_cols: None,
            style: EmbeddedTableStyle::default(),
        },
        GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("missing-res"),
            tab_id: None,
            bounds_pt: DocRectPt::new(260.0, 80.0, 90.0, 40.0),
            selected_rows: None,
            selected_cols: None,
            style: EmbeddedTableStyle::default(),
        },
        GraphAnnotation::Shape {
            kind: AnnotationShapeKind::RoundedRectangle,
            bounds_pt: DocRectPt::new(40.0, 40.0, 50.0, 30.0),
            stroke_color: Rgba8::BLACK,
            stroke_width_pt: 1.0,
            dash: LineDashStyle::Solid,
            fill_color: None,
        },
        GraphAnnotation::LineOrArrow {
            start_pt: (f64::NAN, 15.0),
            end_pt: (50.0, 50.0),
            arrowhead: true,
            dash: LineDashStyle::Solid,
            color: Rgba8::BLACK,
        },
    ];

    let scene = compile_graph(&gs, &doc)?;
    assert!(all_points_finite(&scene), "non-finite coords leaked into scene");
    assert!(scene.warnings.iter().any(|w| w.code == "non_finite_annotation"));
    assert!(scene.warnings.iter().any(|w| w.code == "non_finite_legend_position"));

    let has_quad = scene
        .primitives
        .iter()
        .any(|p| matches!(p, ScenePrimitive::Path { path, .. } if path.commands.iter().any(|c| matches!(c, PathCmd::QuadTo { .. }))));
    assert!(has_quad, "RoundedRectangle annotation should emit QuadTo rounded corners");

    let has_placeholder_rect = scene.primitives.iter().any(|p| match p {
        ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None } if path.commands.len() == 5 => {
            matches!(path.commands.first(), Some(PathCmd::MoveTo(pt)) if (pt.x - 260.0).abs() < 1e-6 && (pt.y - 80.0).abs() < 1e-6)
        }
        _ => false,
    });
    assert!(has_placeholder_rect, "missing embedded table should draw placeholder rect at bounds_pt");

    let has_dense_overlay = scene
        .primitives
        .iter()
        .any(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() == 900));
    assert!(has_dense_overlay, "sample_segments=900 should emit 900-vertex polyline (>512 cap)");

    // Cumulative table cell budget across multiple large tables returns ChartError::ResourceLimit cleanly without OOM.
    let mut gs_many = GraphSheet::new_uninitialized_for_data(SheetId::new("g-many"), &ds)?;
    for idx in 0..12 {
        gs_many.spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("res-big"),
            tab_id: None,
            bounds_pt: DocRectPt::new(10.0 + (idx as f64) * 5.0, 10.0, 400.0, 200_000.0),
            selected_rows: None,
            selected_cols: None,
            style: EmbeddedTableStyle::default(),
        });
    }
    let res_many = compile_graph(&gs_many, &doc);
    assert!(
        matches!(res_many, Err(curvebook_chart::ChartError::ResourceLimit { .. })),
        "cumulative table budget over MAX_LAYOUT_PRIMITIVES must return ResourceLimit, got {res_many:?}"
    );
    Ok(())
}

#[test]
fn fa_sec09_06_failed_results_and_strict_template_hooks() -> TestResult {
    let (mut doc, ds, mut gs) = simple_xy_doc()?;

    // 1. Failed result sheet blocked from XY regression overlay, EmbeddedResultTable, and scalar hooks.
    let mut failed_fit = ResultSheet::new(AnalysisSpec::new(
        SheetId::new("res-fail"),
        ds.id.clone(),
        AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("linear")),
    )?)?;
    failed_fit.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("convergence failed") };
    failed_fit.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: false,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 1.0, y: 2.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 8.0, y: 16.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    failed_fit.scalars.push(ResultScalarEntry { key: String::from("IC50"), data_set_index: Some(0), value: 99.0, units: None });
    failed_fit.tabs.push(ResultTab {
        id: String::from("main"),
        title: String::from("Main"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Val")],
        row_labels: vec![String::from("Stale")],
        rows: vec![vec![ResultCell::Text(String::from("STALE_VALUE"))]],
        footnotes: Vec::new(),
    });
    doc.add_result_sheet(failed_fit)?;

    // 2. UpToDate result with scalar ONLY on dataset 1 must NOT match dataset 0 hook {{result:res-ds1:0:IC50}}.
    let mut res_ds1 =
        ResultSheet::new(AnalysisSpec::new(SheetId::new("res-ds1"), ds.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    res_ds1.status = ResultStatus::UpToDate;
    res_ds1.scalars.push(ResultScalarEntry { key: String::from("IC50"), data_set_index: Some(1), value: 42.0, units: None });
    doc.add_result_sheet(res_ds1)?;

    // 3. Dirty result with matching dataset 0 scalar must NOT resolve.
    let mut res_dirty =
        ResultSheet::new(AnalysisSpec::new(SheetId::new("res-dirty"), ds.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    res_dirty.status = ResultStatus::Dirty;
    res_dirty.scalars.push(ResultScalarEntry { key: String::from("IC50"), data_set_index: Some(0), value: 77.0, units: None });
    doc.add_result_sheet(res_dirty)?;

    gs.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-fail"),
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    gs.spec.title = String::from("{{result:res-dirty:0:IC50}} | {{result:res-ds1:0:IC50}}");
    gs.spec.annotations = vec![GraphAnnotation::EmbeddedResultTable {
        result_sheet_id: SheetId::new("res-fail"),
        tab_id: None,
        bounds_pt: DocRectPt::new(220.0, 20.0, 100.0, 50.0),
        selected_rows: None,
        selected_cols: None,
        style: EmbeddedTableStyle::default(),
    }];

    let scene = compile_graph(&gs, &doc)?;
    assert!(scene.warnings.iter().any(|w| w.code == "failed_regression_result"));
    assert!(scene.warnings.iter().any(|w| w.code == "failed_embedded_result_table"));
    let texts = scene_texts(&scene);
    assert!(!texts.iter().any(|t| t.contains("STALE_VALUE")), "failed table data leaked into scene: {texts:?}");
    assert!(!texts.iter().any(|t| t.contains("77") || t.contains("42")), "dirty or wrong-dataset scalar resolved: {texts:?}");
    assert!(texts.iter().any(|t| t.contains("{{result:res-dirty:0:IC50}}")));
    assert!(texts.iter().any(|t| t.contains("{{result:res-ds1:0:IC50}}")));

    // 4. Failed survival result as primary source emits failed_result_sheet and plots zero curves.
    let mut failed_km = ResultSheet::new(AnalysisSpec::new(
        SheetId::new("km-fail"),
        ds.id.clone(),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            asymmetrical_greenwood_ci: false,
            log_rank_mantel_cox: false,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        }),
    )?)?;
    failed_km.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("failed km") };
    failed_km.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("km"),
        converged: false,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 0.0, y: 1.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 10.0, y: 0.5, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    doc.add_result_sheet(failed_km)?;
    let mut g_km = GraphSheet::new_uninitialized_for_data(SheetId::new("g-km-fail"), &ds)?;
    g_km.spec.kind = GraphKind::Survival;
    g_km.spec.primary_source_sheet_id = SheetId::new("km-fail");
    let km_scene = compile_graph(&g_km, &doc)?;
    assert!(km_scene.warnings.iter().any(|w| w.code == "failed_result_sheet"));

    // 5. Failed diagnostic results (Volcano, ROC, BlandAltman) emit failed_result_sheet and no plotted points.
    for (rid, dkind, akind) in [
        ("res-volc-fail", DiagnosticPlotKind::Volcano, AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default())),
        ("res-roc-fail", DiagnosticPlotKind::RocCurve, AnalysisKind::RocCurve(RocSpec::default())),
        ("res-ba-fail", DiagnosticPlotKind::BlandAltman, AnalysisKind::BlandAltman(BlandAltmanSpec::default())),
    ] {
        let mut r = ResultSheet::new(AnalysisSpec::new(SheetId::new(rid), ds.id.clone(), akind)?)?;
        r.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("failed diag") };
        doc.add_result_sheet(r)?;
        let mut g = GraphSheet::new_uninitialized_for_data(SheetId::new(format!("g-{rid}")), &ds)?;
        g.spec.kind = GraphKind::Diagnostic(dkind);
        g.spec.primary_source_sheet_id = SheetId::new(rid);
        let d_scene = compile_graph(&g, &doc)?;
        assert!(
            d_scene.warnings.iter().any(|w| w.code == "failed_result_sheet"),
            "missing failed_result_sheet warning for {dkind:?}: {:?}",
            d_scene.warnings
        );
    }
    Ok(())
}

#[test]
fn fa_sec09_07_horizontal_column_and_grouped_before_after() -> TestResult {
    // 1. Horizontal ColumnTable with BoxAndWhisker, Violin, BeforeAfterLines, CLD labels, brackets, and Y2 warning.
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 5)?;
    col.data_sets[0].title = String::from("Ctrl");
    col.data_sets[1].title = String::from("Treated");
    for r in 0..5 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + (r as f64) * 2.0);
        col.data_sets[1].cells[r][0] = Cell::number(20.0 + (r as f64) * 2.5);
    }
    let ds = DataSheet::new(SheetId::new("col-h"), "HCol", TableArchetype::Column(col))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;

    let mut anova_res =
        ResultSheet::new(AnalysisSpec::new(SheetId::new("res-anova"), ds.id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))?)?;
    anova_res.status = ResultStatus::UpToDate;
    anova_res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: 0,
        group_b: 1,
        label_a: String::from("Ctrl"),
        label_b: String::from("Treated"),
        estimate_diff: Some(-10.0),
        raw_p_or_q: 0.002,
        is_fdr_q: false,
        significant: true,
    });
    doc.add_result_sheet(anova_res.clone())?;

    for (gdo, expect_y2_warn) in [
        (
            GraphicalDataObject::BoxAndWhisker {
                whiskers: WhiskerMethod::MinToMax,
                fill_color: Rgba8::rgb(100, 150, 200),
                stroke_color: Rgba8::BLACK,
                show_outliers: true,
                show_all_points: false,
            },
            true,
        ),
        (
            GraphicalDataObject::Violin {
                fill_color: Rgba8::rgb(100, 200, 150),
                stroke_color: Rgba8::BLACK,
                clip_to_observed_range: false,
                show_median_and_quartiles: true,
                half_violin_with_points: false,
            },
            false,
        ),
        (
            GraphicalDataObject::BeforeAfterLines {
                marker: Some(MarkerStyle {
                    shape: MarkerShape::Circle,
                    size_pt: 5.0,
                    fill_mode: MarkerFillMode::Filled,
                    fill_color: Rgba8::BLACK,
                    stroke_color: Rgba8::WHITE,
                    stroke_width_pt: 0.75,
                }),
                line_color: Rgba8::BLACK,
                stroke_width_pt: 1.0,
            },
            false,
        ),
    ] {
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-h"), &ds)?;
        gs.spec.orientation = curvebook_doc::CartesianOrientation::Horizontal;
        gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(anova_res.id.clone()));
        gs.spec.pairwise_display_mode = PairwiseDisplayMode::BracketsAndCompactLetterDisplay;
        gs.spec.comparisons = vec![PairwiseBracketSpec {
            result_sheet_id: anova_res.id.clone(),
            group_a: 0,
            group_b: 1,
            style: BracketStyle::Square,
            label_mode: BracketLabelMode::Custom(String::from("*")),
            hide_if_non_significant: false,
            stroke_width_pt: 1.0,
            font_size_pt: 9.0,
            bracket_step_pt: 14.0,
        }];
        for (idx, s) in gs.spec.series.iter_mut().enumerate() {
            s.gdo = gdo.clone();
            if expect_y2_warn && idx == 1 {
                s.bind_to_right_y2 = true;
            }
        }
        let scene = compile_graph(&gs, &doc)?;
        assert!(all_points_finite(&scene));
        assert!(
            scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { clip: Some(_), .. })),
            "horizontal {gdo:?} produced no clipped geometry"
        );
        let texts = scene_texts(&scene);
        assert!(texts.iter().any(|t| t == "*"), "horizontal bracket label missing: {texts:?}");
        assert!(texts.iter().any(|t| t == "a") && texts.iter().any(|t| t == "b"), "horizontal CLD labels missing: {texts:?}");
        if expect_y2_warn {
            assert!(scene.warnings.iter().any(|w| w.code == "horizontal_y2_unsupported"));
        }
    }

    // 2. Grouped BeforeAfterLines across Interleaved & Separated (Vertical & Horizontal), plus invalid Nested warning.
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2)?;
    grp.row_titles = vec![String::from("Week 1"), String::from("Week 2")];
    grp.data_sets[0].title = String::from("Vehicle");
    grp.data_sets[1].title = String::from("Drug");
    for r in 0..2 {
        for c in 0..2 {
            for k in 0..3 {
                grp.data_sets[c].cells[r][k] = Cell::number(10.0 + (r * 10 + c * 5 + k) as f64);
            }
        }
    }
    let grp_ds = DataSheet::new(SheetId::new("grp-ba"), "Grouped BA", TableArchetype::Grouped(grp))?;
    doc.add_data_sheet(grp_ds.clone())?;

    for arr in [GroupedArrangement::Interleaved, GroupedArrangement::Separated] {
        for orient in [curvebook_doc::CartesianOrientation::Vertical, curvebook_doc::CartesianOrientation::Horizontal] {
            let mut g_ba = GraphSheet::new_uninitialized_for_data(SheetId::new("g-gba"), &grp_ds)?;
            g_ba.spec.grouped_arrangement = arr;
            g_ba.spec.orientation = orient;
            for s in &mut g_ba.spec.series {
                s.gdo = GraphicalDataObject::BeforeAfterLines { marker: None, line_color: Rgba8::BLACK, stroke_width_pt: 1.2 };
            }
            let scene = compile_graph(&g_ba, &doc)?;
            let connector_lines = scene
                .primitives
                .iter()
                .filter(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() == 2))
                .count();
            assert!(connector_lines >= 3, "expected before-after lines for {arr:?}/{orient:?}, got {connector_lines}");
        }
    }

    let mut nested = NestedTable::new(2, 2, 2)?;
    for g in 0..2 {
        for s in 0..2 {
            for r in 0..2 {
                nested.groups[g].cells[r][s] = Cell::number(5.0 + (g * 4 + s * 2 + r) as f64);
            }
        }
    }
    let nest_ds = DataSheet::new(SheetId::new("nest-ba"), "Nested BA", TableArchetype::Nested(nested))?;
    doc.add_data_sheet(nest_ds.clone())?;
    let mut g_nest = GraphSheet::new_uninitialized_for_data(SheetId::new("g-nest"), &nest_ds)?;
    for s in &mut g_nest.spec.series {
        s.gdo = GraphicalDataObject::BeforeAfterLines { marker: None, line_color: Rgba8::BLACK, stroke_width_pt: 1.0 };
    }
    let nest_scene = compile_graph(&g_nest, &doc)?;
    assert!(
        nest_scene.warnings.iter().any(|w| w.code == "unsupported_before_after_pairing"),
        "expected unsupported_before_after_pairing for Nested table, got {:?}",
        nest_scene.warnings
    );
    Ok(())
}

#[test]
fn fa_sec09_08_presummarized_bounds_hinges_log_clip_and_reversed_violin() -> TestResult {
    // 1. Pre-summarized BoxAndWhisker, Violin, FloatingBar with y_error_bars: None AND swapped [upper, center, lower].
    let mut grp = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 2, 1)?;
    grp.row_titles = vec![String::from("Group1")];
    // Series 0: normal [center=30, lower=10, upper=50]
    grp.data_sets[0].cells[0] = vec![Cell::number(30.0), Cell::number(10.0), Cell::number(50.0)];
    // Series 1: swapped [center=30, upper=50, lower=10] — must sort bounds without degenerate/inverted box
    grp.data_sets[1].cells[0] = vec![Cell::number(30.0), Cell::number(50.0), Cell::number(10.0)];
    let ds = DataSheet::new(SheetId::new("grp-pre"), "PreSum", TableArchetype::Grouped(grp))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;

    for gdo in [
        GraphicalDataObject::BoxAndWhisker {
            whiskers: WhiskerMethod::MinToMax,
            fill_color: Rgba8::rgb(120, 120, 220),
            stroke_color: Rgba8::BLACK,
            show_outliers: false,
            show_all_points: false,
        },
        GraphicalDataObject::Violin {
            fill_color: Rgba8::rgb(120, 220, 120),
            stroke_color: Rgba8::BLACK,
            clip_to_observed_range: false,
            show_median_and_quartiles: true,
            half_violin_with_points: false,
        },
        GraphicalDataObject::FloatingBar {
            summary: Some(SummaryMeasure::Mean),
            fill_color: Rgba8::rgb(220, 140, 120),
            border_color: Rgba8::BLACK,
            border_width_pt: 1.0,
        },
    ] {
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-pre"), &ds)?;
        for s in &mut gs.spec.series {
            s.gdo = gdo.clone();
            s.y_error_bars = None;
        }
        let scene = compile_graph(&gs, &doc)?;
        // Boxes / violins / floating bars must span measurable Y height (> 20pt), not 0pt line!
        let max_poly_height = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } => {
                    let ys: Vec<f64> = path
                        .commands
                        .iter()
                        .filter_map(|c| match c {
                            PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => Some(pt.y),
                            _ => None,
                        })
                        .collect();
                    let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
                    let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    Some((max_y - min_y).max(0.0))
                }
                _ => None,
            })
            .fold(0.0_f64, f64::max);
        assert!(max_poly_height > 20.0, "pre-summarized bounds collapsed to height={max_poly_height} for {gdo:?}");
    }

    // 2. Log10 BoxAndWhisker & FloatingBar with non-positive lower bound emit log_axis_floor_clip.
    let mut neg_grp = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 1, 1)?;
    neg_grp.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(-2.0), Cell::number(50.0)];
    let neg_ds = DataSheet::new(SheetId::new("grp-neg"), "NegBounds", TableArchetype::Grouped(neg_grp))?;
    doc.add_data_sheet(neg_ds.clone())?;
    for gdo in [
        GraphicalDataObject::BoxAndWhisker {
            whiskers: WhiskerMethod::MinToMax,
            fill_color: Rgba8::rgb(80, 120, 200),
            stroke_color: Rgba8::BLACK,
            show_outliers: false,
            show_all_points: false,
        },
        GraphicalDataObject::FloatingBar {
            summary: Some(SummaryMeasure::Mean),
            fill_color: Rgba8::rgb(80, 120, 200),
            border_color: Rgba8::BLACK,
            border_width_pt: 1.0,
        },
    ] {
        let mut gs_log = GraphSheet::new_uninitialized_for_data(SheetId::new("g-log-clip"), &neg_ds)?;
        gs_log.spec.y_axis.segments[0].scale = AxisScale::Log10;
        gs_log.spec.y_axis.segments[0].min = Some(1.0);
        gs_log.spec.y_axis.segments[0].max = Some(100.0);
        gs_log.spec.series[0].gdo = gdo;
        gs_log.spec.series[0].y_error_bars = None;
        let scene = compile_graph(&gs_log, &doc)?;
        assert!(scene.warnings.iter().any(|w| w.code == "log_axis_floor_clip"), "expected log_axis_floor_clip warning, got {:?}", scene.warnings);
    }

    // 3. Reversed Y-axis Violin renders full KDE polygon (Closed path with > 10 vertices), on both Linear & Log10.
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 6)?;
    for (r, v) in [2.0, 5.0, 10.0, 20.0, 40.0, 80.0].into_iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let col_ds = DataSheet::new(SheetId::new("col-vio"), "ViolinRev", TableArchetype::Column(col))?;
    doc.add_data_sheet(col_ds.clone())?;
    for (scale, min_v, max_v) in [(AxisScale::Linear, 100.0, 0.0), (AxisScale::Log10, 100.0, 1.0)] {
        let mut gs_rev = GraphSheet::new_uninitialized_for_data(SheetId::new("g-rev"), &col_ds)?;
        gs_rev.spec.y_axis.segments[0].scale = scale;
        gs_rev.spec.y_axis.segments[0].min = Some(min_v);
        gs_rev.spec.y_axis.segments[0].max = Some(max_v);
        gs_rev.spec.series[0].gdo = GraphicalDataObject::Violin {
            fill_color: Rgba8::rgb(0, 158, 115),
            stroke_color: Rgba8::BLACK,
            clip_to_observed_range: false,
            show_median_and_quartiles: true,
            half_violin_with_points: false,
        };
        let scene = compile_graph(&gs_rev, &doc)?;
        let has_kde_polygon = scene.primitives.iter().any(|p| {
            matches!(
                p,
                ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. }
                    if path.commands.len() > 10 && matches!(path.commands.last(), Some(PathCmd::Close))
            )
        });
        assert!(has_kde_polygon, "reversed {scale:?} violin failed to emit multi-vertex closed KDE contour");
    }
    Ok(())
}

#[test]
fn fa_sec09_09_survival_censoring_nan_and_contingency_log() -> TestResult {
    let (mut doc, ds, _gs) = simple_xy_doc()?;

    // 1. NAR table with events == 0 and unchanged n_at_risk must emit ZERO censoring ticks;
    //    NaN/Inf time/survival points in fitted_curves must be filtered before max_time & drawing.
    let mut km_res = ResultSheet::new(AnalysisSpec::new(
        SheetId::new("km-nar"),
        ds.id.clone(),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            asymmetrical_greenwood_ci: false,
            log_rank_mantel_cox: false,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        }),
    )?)?;
    km_res.status = ResultStatus::UpToDate;
    km_res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("km"),
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 0.0, y: 1.0, ci_lower: Some(1.0), ci_upper: Some(1.0), pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 5.0, y: 0.8, ci_lower: Some(0.5), ci_upper: Some(0.95), pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: f64::NAN, y: 0.5, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: f64::INFINITY, y: 0.1, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    km_res.tabs.push(ResultTab {
        id: String::from("nar"),
        title: String::from("Number at Risk"),
        kind: ResultTabKind::NumberAtRisk,
        column_headers: vec![String::from("Cohort"), String::from("Time"), String::from("AtRisk"), String::from("Events")],
        row_labels: vec![String::from("t0"), String::from("t5")],
        rows: vec![
            vec![
                ResultCell::Text(String::from("Arm A")),
                ResultCell::Number { value: 0.0, prefix: None, units: None },
                ResultCell::Number { value: 10.0, prefix: None, units: None },
                ResultCell::Number { value: 0.0, prefix: None, units: None },
            ],
            vec![
                ResultCell::Text(String::from("Arm A")),
                ResultCell::Number { value: 5.0, prefix: None, units: None },
                ResultCell::Number { value: 10.0, prefix: None, units: None },
                ResultCell::Number { value: 0.0, prefix: None, units: None },
            ],
        ],
        footnotes: Vec::new(),
    });
    doc.add_result_sheet(km_res)?;

    let mut g_km = GraphSheet::new_uninitialized_for_data(SheetId::new("g-km-nar"), &ds)?;
    g_km.spec.kind = GraphKind::Survival;
    g_km.spec.primary_source_sheet_id = SheetId::new("km-nar");
    let km_scene = compile_graph(&g_km, &doc)?;
    assert!(all_points_finite(&km_scene));
    let censor_ticks = km_scene
        .primitives
        .iter()
        .filter(|p| match p {
            ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() == 2 => {
                matches!((path.commands.first(), path.commands.get(1)), (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b))) if (a.x - b.x).abs() < 1e-6 && ((b.y - a.y).abs() - 7.0).abs() < 0.5)
            }
            _ => false,
        })
        .count();
    assert_eq!(censor_ticks, 0, "no spurious censor ticks expected when events=0 and n_at_risk unchanged");

    // 2. Stacked Contingency on Log10 must still draw bottom slice (acc=0 -> top) and warn log_axis_floor_clip.
    let mut cont = ContingencyTable::new(2, 2)?;
    cont.row_titles = vec![String::from("Cat A"), String::from("Cat B")];
    cont.data_sets[0].cells[0][0] = Cell::number(15.0);
    cont.data_sets[1].cells[0][0] = Cell::number(25.0);
    cont.data_sets[0].cells[1][0] = Cell::number(10.0);
    cont.data_sets[1].cells[1][0] = Cell::number(30.0);
    let cont_ds = DataSheet::new(SheetId::new("cont-log"), "ContLog", TableArchetype::Contingency(cont))?;
    doc.add_data_sheet(cont_ds.clone())?;

    let mut g_lin = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cont-lin"), &cont_ds)?;
    g_lin.spec.kind = GraphKind::Contingency;
    g_lin.spec.grouped_arrangement = GroupedArrangement::Stacked;
    let lin_scene = compile_graph(&g_lin, &doc)?;
    let lin_bars = lin_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: Some(_), .. })).count();

    let mut g_log = g_lin;
    g_log.spec.y_axis.segments[0].scale = AxisScale::Log10;
    g_log.spec.y_axis.segments[0].min = Some(1.0);
    g_log.spec.y_axis.segments[0].max = Some(100.0);
    let log_scene = compile_graph(&g_log, &doc)?;
    let log_bars = log_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: Some(_), .. })).count();
    assert_eq!(log_bars, lin_bars, "stacked Log10 contingency dropped bottom slices");
    assert!(log_scene.warnings.iter().any(|w| w.code == "log_axis_floor_clip"));
    Ok(())
}

#[test]
fn fa_sec09_10_forest_margin_log_floor_and_reciprocal_overlay() -> TestResult {
    // 1. Forest plot with legend.visible = false + wide CI strings reserves right margin so text stays within width_pt,
    //    clamps non-positive lower bound on Log10, and places missing estimate at geometric midpoint.
    let mut g_tbl = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 1, 2)?;
    g_tbl.row_titles = vec![String::from("Study Alpha"), String::from("Study Beta")];
    // Row 0: est=1.5, lower=-0.2 (non-positive -> log_axis_floor_clip), upper=3.0
    g_tbl.data_sets[0].cells[0] = vec![Cell::number(1.5), Cell::number(-0.2), Cell::number(3.0)];
    // Row 1: missing est (empty), lower=0.25, upper=4.0 -> geometric midpoint sqrt(0.25*4) = 1.0
    g_tbl.data_sets[0].cells[1] = vec![Cell::empty(), Cell::number(0.25), Cell::number(4.0)];
    let f_ds = DataSheet::new(SheetId::new("f-ds"), "Forest", TableArchetype::Grouped(g_tbl))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(f_ds.clone())?;

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-f"), &f_ds)?;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
    gs.spec.legend.visible = false;
    gs.spec.x_axis.segments[0].scale = AxisScale::Log10;
    gs.spec.x_axis.segments[0].min = Some(0.1);
    gs.spec.x_axis.segments[0].max = Some(10.0);
    let scene = compile_graph(&gs, &doc)?;
    assert!(all_points_finite(&scene));
    assert!(scene.warnings.iter().any(|w| w.code == "log_axis_floor_clip"));
    assert!(scene.plot_rect_pt.right() < scene.width_pt - 50.0, "ForestPlot must reserve right margin for CI text even when legend is hidden");
    let ci_texts: Vec<_> = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text().contains('[') && t.rich.plain_text().contains(']') => Some(t),
            _ => None,
        })
        .collect();
    assert!(!ci_texts.is_empty());
    for t in &ci_texts {
        assert!(t.anchor_pt.x <= scene.width_pt, "CI text anchor {} exceeded width {}", t.anchor_pt.x, scene.width_pt);
    }

    // 2. XY Reciprocal scale FittedCurveOverlay spanning zero (-5..=5) resamples positive [1..=5] cleanly.
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3)?;
    for (r, (x, y)) in [(1.0, 2.0), (2.5, 4.0), (5.0, 8.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let xy_ds = DataSheet::new(SheetId::new("xy-rec"), "Recip", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(xy_ds.clone())?;
    let mut fit_res = ResultSheet::new(AnalysisSpec::new(
        SheetId::new("res-rec"),
        xy_ds.id.clone(),
        AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("linear")),
    )?)?;
    fit_res.status = ResultStatus::UpToDate;
    fit_res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("line"),
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: -5.0, y: 2.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 5.0, y: 8.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    doc.add_result_sheet(fit_res)?;

    let mut g_rec = GraphSheet::new_uninitialized_for_data(SheetId::new("g-rec"), &xy_ds)?;
    g_rec.spec.x_axis.segments[0].scale = AxisScale::Reciprocal;
    g_rec.spec.x_axis.segments[0].min = Some(1.0);
    g_rec.spec.x_axis.segments[0].max = Some(5.0);
    g_rec.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-rec"),
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    let rec_scene = compile_graph(&g_rec, &doc)?;
    let has_overlay = rec_scene
        .primitives
        .iter()
        .any(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() == 256));
    assert!(has_overlay, "zero-spanning curve overlay should still render positive [1, 5] Reciprocal domain");
    Ok(())
}

#[test]
fn fa_sec09_11_and_13_axes_frames_broken_segments_and_cat_tick_sources() -> TestResult {
    // 1. XY OriginCrossing ticks + OffsetRangeTufte + broken multi-segment is_out_of_axis_domain
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2)?;
    xy.x_cells[0][0] = Cell::number(2.0);
    xy.data_sets[0].cells[0][0] = Cell::number(2.0);
    xy.x_cells[1][0] = Cell::number(150.0);
    xy.data_sets[0].cells[1][0] = Cell::number(8.0);
    let xy_ds = DataSheet::new(SheetId::new("xy-axes"), "Axes", TableArchetype::Xy(xy))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(xy_ds.clone())?;

    let mut g_orig = GraphSheet::new_uninitialized_for_data(SheetId::new("g-orig"), &xy_ds)?;
    g_orig.spec.frame_preset = FramePreset::OriginCrossing;
    g_orig.spec.x_axis.segments[0].min = Some(1.0);
    g_orig.spec.x_axis.segments[0].max = Some(10.0);
    g_orig.spec.y_axis.segments[0].min = Some(1.0);
    g_orig.spec.y_axis.segments[0].max = Some(10.0);
    let orig_scene = compile_graph(&g_orig, &doc)?;
    assert!(orig_scene.warnings.iter().any(|w| w.code == "origin_outside_x_domain"));
    assert!(orig_scene.warnings.iter().any(|w| w.code == "origin_outside_y_domain"));

    // Multi-segment broken X axis [0..10, 100..200]: x=150 is inside segments[1] -> no out_of_axis_domain!
    let mut g_brk = GraphSheet::new_uninitialized_for_data(SheetId::new("g-brk"), &xy_ds)?;
    g_brk.spec.x_axis.segments = vec![
        AxisSegment { min: Some(0.0), max: Some(10.0), scale: AxisScale::Linear, ..AxisSegment::default() },
        AxisSegment { min: Some(100.0), max: Some(200.0), scale: AxisScale::Linear, ..AxisSegment::default() },
    ];
    let brk_scene = compile_graph(&g_brk, &doc)?;
    assert!(
        !brk_scene.warnings.iter().any(|w| w.code == "out_of_axis_domain"),
        "point in segments[1] flagged out_of_axis_domain: {:?}",
        brk_scene.warnings
    );

    // 2. Survival chart with 2-segment broken X axis + OffsetRangeTufte + rotated Y tick angle
    let mut surv = SurvivalTable::new(1, 4)?;
    for (r, (t, st)) in [(2.0, 1.0), (5.0, 0.0), (120.0, 1.0), (180.0, 1.0)].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(st);
    }
    let s_ds = DataSheet::new(SheetId::new("surv-brk"), "SurvBrk", TableArchetype::Survival(surv))?;
    doc.add_data_sheet(s_ds.clone())?;
    let mut g_surv = GraphSheet::new_uninitialized_for_data(SheetId::new("g-sbrk"), &s_ds)?;
    g_surv.spec.frame_preset = FramePreset::OffsetRangeTufte;
    g_surv.spec.y_axis.tick_angle_deg = 45.0;
    g_surv.spec.x_axis.segments = vec![
        AxisSegment { min: Some(0.0), max: Some(10.0), scale: AxisScale::Linear, ..AxisSegment::default() },
        AxisSegment { min: Some(100.0), max: Some(200.0), scale: AxisScale::Linear, ..AxisSegment::default() },
    ];
    let s_scene = compile_graph(&g_surv, &doc)?;
    assert!(all_points_finite(&s_scene));
    assert!(
        s_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if (t.rotation_deg - 45.0).abs() < 1e-6)),
        "expected rotated Y tick label in survival_cat"
    );

    // 3. CategoricalTickSource across Grouped arrangements
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, 2)?;
    grp.row_titles = vec![String::from("Day0"), String::from("Day7")];
    grp.data_sets[0].title = String::from("Placebo");
    grp.data_sets[1].title = String::from("Active");
    for r in 0..2 {
        for c in 0..2 {
            grp.data_sets[c].cells[r][0] = Cell::number(10.0 + (r + c) as f64);
        }
    }
    let grp_ds = DataSheet::new(SheetId::new("grp-cat"), "GrpCat", TableArchetype::Grouped(grp))?;
    doc.add_data_sheet(grp_ds.clone())?;

    for (src, arr, expected) in [
        (CategoricalTickSource::RowTitles, GroupedArrangement::Interleaved, "Day0"),
        (CategoricalTickSource::ColumnTitles, GroupedArrangement::Interleaved, "Placebo"),
        (CategoricalTickSource::RowNumbers, GroupedArrangement::Stacked, "2"),
        (CategoricalTickSource::ColumnTitles, GroupedArrangement::Separated, "Active"),
        (CategoricalTickSource::RowTitles, GroupedArrangement::Superimposed, "Day7"),
    ] {
        let mut g_cat = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cat"), &grp_ds)?;
        g_cat.spec.grouped_arrangement = arr;
        g_cat.spec.x_axis.tick_format = TickLabelFormat::Categorical(src);
        let c_scene = compile_graph(&g_cat, &doc)?;
        let texts = scene_texts(&c_scene);
        assert!(texts.iter().any(|t| t == expected), "missing `{expected}` for {src:?}/{arr:?}: {texts:?}");
    }
    Ok(())
}
