use curvebook_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AnnotationShapeKind, AreaFillSpec, AxisSegment, AxisThreshold, BracketLabelMode, BracketStyle, BreakOrnament,
    CategoricalTickSource, Cell, ColumnDesignHint, ColumnTable, ConnectingLineStyle, ConstantValue, DataSheet, EquationDisplayMode,
    ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FramePreset, GraphAnnotation, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement,
    GroupedTable, InfoConstant, InfoSheet, LineConnectionKind, LinearRegressionSpec, MultiplicityMethod, MultiplicitySpec, NestedTable,
    NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod, PairwiseBracketSpec, ProjectDocument, RectPt as DocRectPt, RegressionOverlaySpec,
    ReplicateOverlayStyle, ResultScalarEntry, ResultSheet, ResultStatus, SheetId, SubcolumnMode, SummaryMeasure, TableArchetype, TickLabelFormat,
    WhiskerMethod, XColumnMode, XyTable,
};
use curvebook_fit::{HookContext, execute_analysis};
use curvebook_math::{AxisScale, ConfidenceLevel, PValueFormat};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};

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
                if !t.anchor_pt.x.is_finite() || !t.anchor_pt.y.is_finite() || !t.font_size_pt.is_finite() {
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

fn find_texts(scene: &ChartScene) -> Vec<String> {
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
fn slice_0_xy_dose_response_replicates_and_4pl_overlay() {
    let mut doc = ProjectDocument::new("Slice 0 Assay").unwrap();

    // Paired InfoSheet with constant for template hook resolution
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Info").unwrap();
    info.constants.push(InfoConstant::new("Compound", ConstantValue::Text(String::from("LC-401")), None).unwrap());
    doc.add_info_sheet(info).unwrap();

    // XY Table with RawReplicates(3), agonist concentration & 4PL Hill curve
    let concs = [1e-9, 1e-8, 1e-7, 1e-6, 1e-5, 1e-4];
    let true_y = |x: f64| -> f64 { 10.0 + (100.0 - 10.0) / (1.0 + (1e-6 / x).powf(1.0)) };

    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), 1, concs.len()).unwrap();
    xy.x_title = String::from("Agonist (M)");
    xy.data_sets[0].title = String::from("Response (%)");
    for (r, &c) in concs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(c)).unwrap();
        let base = true_y(c);
        xy.data_sets[0].cells[r][0] = Cell::number(base - 1.8);
        xy.data_sets[0].cells[r][1] = Cell::number(base + 0.4);
        xy.data_sets[0].cells[r][2] = Cell::number(base + 1.4);
    }
    // Exclude one extreme corrupted replicate on row 2 to confirm non-destructive masking
    xy.data_sets[0].cells[2].push(Cell::excluded_number(9999.0));
    xy.y_subcolumn_mode = SubcolumnMode::RawReplicates(4);
    for (r, row) in xy.data_sets[0].cells.iter_mut().enumerate() {
        if r != 2 {
            row.push(Cell::empty());
        }
    }

    let mut data_sheet = DataSheet::new("data-1", "Dose Response", TableArchetype::Xy(xy)).unwrap();
    data_sheet.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(data_sheet).unwrap();

    // Run 4PL nonlinear fit producing FittedCurveOverlay with CI and prediction bands
    let mut nl = NonlinearFitSpec::new("agonist_vs_response_4pl");
    nl.confidence_bands = true;
    nl.prediction_bands = true;
    let fit_spec = AnalysisSpec::new("fit-1", "data-1", AnalysisKind::NonlinearRegression(nl)).unwrap();
    let fit_res = curvebook_fit::execute_analysis(&fit_spec, doc.data_sheet(&SheetId::new("data-1")).unwrap(), &HookContext::default()).unwrap();
    assert!(!fit_res.fitted_curves.is_empty());
    doc.add_result_sheet(fit_res).unwrap();

    let mut spec = GraphSpec::default_for_data_sheet(doc.data_sheet(&SheetId::new("data-1")).unwrap());
    spec.title = String::from("Dose Response Curve");
    spec.x_axis.segments[0].scale = AxisScale::Log10;
    spec.x_axis.tick_format = TickLabelFormat::PowerOfTenSuperscript;
    spec.y_axis.title = String::from("Activity ({{info:Compound}}, %)");
    spec.series[0].y_error_bars = Some(ErrorBarSpec {
        metric: ErrorBarMetric::ConfidenceInterval95,
        direction: ErrorBarDirection::Both,
        dash_style: LineDashStyle::Solid,
        stroke_width_pt: 1.0,
        cap_width_pt: 5.0,
        color: None,
    });
    spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("fit-1"),
        sample_segments: 384,
        show_confidence_band: true,
        show_prediction_band: true,
        shaded_ribbon: true,
    });

    let mut graph = GraphSheet::new_uninitialized_for_data("graph-1", doc.data_sheet(&SheetId::new("data-1")).unwrap()).unwrap();
    graph.initialized = true;
    graph.spec = spec;

    let scene = compile_graph(&graph, &doc).unwrap();
    assert!(all_points_finite(&scene));

    // Verify adaptive curve resampling emitted ~384 polyline points (within 256..=512)
    let curve_segments: usize = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: None, stroke: Some(s), .. } if s.width_pt >= 1.7 => Some(path.commands.len()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    assert!((256..=512).contains(&curve_segments), "expected adaptive 256..=512 curve points, got {curve_segments}");

    // Verify shaded CI and prediction ribbons emitted closed filled paths
    let shaded_bands = scene
        .primitives
        .iter()
        .filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(f), stroke: None, .. } if f.color.a > 0 && f.color.a < 255))
        .count();
    assert!(shaded_bands >= 2, "expected CI + prediction shaded polygons");

    // Check expanded InfoSheet template hook & PowerOfTenSuperscript markup
    let texts = find_texts(&scene);
    assert!(texts.iter().any(|t| t == "Activity (LC-401, %)"), "template hook not expanded in {texts:?}");
    let has_superscript = scene.primitives.iter().any(|p| match p {
        ScenePrimitive::Text(t) => t.rich.spans.iter().any(|sp| sp.script == curvebook_text::SpanScript::Superscript),
        _ => false,
    });
    assert!(has_superscript, "expected 10^k superscript spans on log X ticks");
}

#[test]
fn xy_summary_error_bars_student_t_explicit_horizontal_and_directions() {
    let mut doc = ProjectDocument::new("Error Bar Matrix").unwrap();
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::RawReplicates(3), 3, 3).unwrap();

    // Row 0..3
    for r in 0..3 {
        let x = (r + 1) as f64 * 10.0;
        xy.set_x_subcell(r, 0, Cell::number(x)).unwrap();
        xy.set_x_subcell(r, 1, Cell::number(1.5)).unwrap();
        xy.set_x_subcell(r, 2, Cell::number(4.0)).unwrap();

        // Series 0 (95% CI, Both), Series 1 (SD, UpperOnly, Dashed), Series 2 (Range, LowerOnly, Dotted)
        for c in 0..3 {
            xy.data_sets[c].cells[r][0] = Cell::number(20.0 + (r as f64) * 5.0 - 2.0);
            xy.data_sets[c].cells[r][1] = Cell::number(20.0 + (r as f64) * 5.0);
            xy.data_sets[c].cells[r][2] = Cell::number(20.0 + (r as f64) * 5.0 + 4.0);
        }
    }

    doc.add_data_sheet(DataSheet::new("data-eb", "EB", TableArchetype::Xy(xy)).unwrap()).unwrap();
    let mut graph = GraphSheet::new_uninitialized_for_data("g-eb", doc.data_sheet(&SheetId::new("data-eb")).unwrap()).unwrap();
    graph.initialized = true;
    graph.spec.y_axis.segments[0].min = Some(0.0);
    graph.spec.y_axis.segments[0].max = Some(50.0);
    graph.spec.series[0].x_error_bars = Some(ErrorBarSpec {
        metric: ErrorBarMetric::Sem,
        direction: ErrorBarDirection::Both,
        dash_style: LineDashStyle::Solid,
        stroke_width_pt: 1.0,
        cap_width_pt: 4.0,
        color: None,
    });
    graph.spec.series[0].y_error_bars = Some(ErrorBarSpec {
        metric: ErrorBarMetric::ConfidenceInterval95,
        direction: ErrorBarDirection::Both,
        dash_style: LineDashStyle::Solid,
        stroke_width_pt: 1.1,
        cap_width_pt: 6.0,
        color: None,
    });
    graph.spec.series[1].y_error_bars = Some(ErrorBarSpec {
        metric: ErrorBarMetric::Sd,
        direction: ErrorBarDirection::UpperOnly,
        dash_style: LineDashStyle::Dashed,
        stroke_width_pt: 1.1,
        cap_width_pt: 5.0,
        color: None,
    });
    graph.spec.series[2].y_error_bars = Some(ErrorBarSpec {
        metric: ErrorBarMetric::InterquartileRange,
        direction: ErrorBarDirection::LowerOnly,
        dash_style: LineDashStyle::Dotted,
        stroke_width_pt: 1.1,
        cap_width_pt: 4.0,
        color: None,
    });

    let scene = compile_graph(&graph, &doc).unwrap();
    assert!(all_points_finite(&scene));

    // Verify dashed & dotted error bar paths exist
    let has_dashed = scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { stroke: Some(s), .. } if s.dash == LineDashStyle::Dashed));
    let has_dotted = scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { stroke: Some(s), .. } if s.dash == LineDashStyle::Dotted));
    assert!(has_dashed && has_dotted);

    // Explicit subcolumn MeanLowHigh test
    let mut exp_doc = ProjectDocument::new("Explicit").unwrap();
    let mut exp_xy = XyTable::new(SubcolumnMode::MeanLowHigh, 1, 2).unwrap();
    exp_xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    exp_xy.data_sets[0].cells[0] = vec![Cell::number(50.0), Cell::number(5.0), Cell::number(15.0)];
    exp_xy.set_x_cell(1, Cell::number(2.0)).unwrap();
    exp_xy.data_sets[0].cells[1] = vec![Cell::number(60.0), Cell::number(10.0), Cell::number(20.0)];
    exp_doc.add_data_sheet(DataSheet::new("d-exp", "Explicit EB", TableArchetype::Xy(exp_xy)).unwrap()).unwrap();
    let mut g_exp = GraphSheet::new_uninitialized_for_data("g-exp", exp_doc.data_sheet(&SheetId::new("d-exp")).unwrap()).unwrap();
    g_exp.spec.series[0].y_error_bars = Some(ErrorBarSpec { metric: ErrorBarMetric::ExplicitSubcolumns, ..ErrorBarSpec::default() });
    let exp_scene = compile_graph(&g_exp, &exp_doc).unwrap();
    assert!(all_points_finite(&exp_scene));
    assert!(exp_scene.primitives.len() > 10);
}

#[test]
fn xy_all_markers_splines_area_fill_and_okabe_ito() {
    let shapes = [
        MarkerShape::Circle,
        MarkerShape::Square,
        MarkerShape::TriangleUp,
        MarkerShape::TriangleDown,
        MarkerShape::Diamond,
        MarkerShape::Cross,
        MarkerShape::Plus,
        MarkerShape::Star,
        MarkerShape::Hexagon,
        MarkerShape::HorizontalBar,
        MarkerShape::VerticalBar,
    ];
    let fill_modes = [
        MarkerFillMode::Filled,
        MarkerFillMode::Open,
        MarkerFillMode::SemiFilledTop,
        MarkerFillMode::SemiFilledBottom,
        MarkerFillMode::SemiFilledLeft,
        MarkerFillMode::SemiFilledRight,
    ];
    let line_kinds =
        [LineConnectionKind::PointToPoint, LineConnectionKind::Staircase, LineConnectionKind::MonotonicCubicSpline, LineConnectionKind::AkimaSpline];

    let mut doc = ProjectDocument::new("Shapes & Splines").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, shapes.len(), 5).unwrap();
    for r in 0..5 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        for c in 0..shapes.len() {
            xy.data_sets[c].cells[r][0] = Cell::number(10.0 + (c as f64) * 3.0 + ((r as f64) * 0.9).sin() * 4.0);
        }
    }
    doc.add_data_sheet(DataSheet::new("d-shapes", "Markers", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let mut graph = GraphSheet::new_uninitialized_for_data("g-shapes", doc.data_sheet(&SheetId::new("d-shapes")).unwrap()).unwrap();
    graph.spec.palette = PaletteId::OkabeIto;
    for (idx, s) in graph.spec.series.iter_mut().enumerate() {
        let color = PaletteId::OkabeIto.color_at(idx);
        s.gdo = GraphicalDataObject::ScatterAndLine {
            marker: Some(MarkerStyle {
                shape: shapes[idx],
                fill_mode: fill_modes[idx % fill_modes.len()],
                size_pt: 6.0,
                fill_color: color,
                stroke_color: color,
                stroke_width_pt: 1.0,
            }),
            line: Some(ConnectingLineStyle {
                kind: line_kinds[idx % line_kinds.len()],
                dash_style: LineDashStyle::Solid,
                stroke_width_pt: 1.2,
                color,
            }),
            area_fill: (idx == 0).then_some(AreaFillSpec { baseline_y: 0.0, fill_color: color.with_alpha(55) }),
            replicate_lines: curvebook_doc::ReplicateLineMode::default(),
        };
    }

    let scene = compile_graph(&graph, &doc).unwrap();
    assert!(all_points_finite(&scene));

    // Must contain CubicTo commands from MonotonicCubicSpline, AkimaSpline, and Circle markers
    let cubic_count = scene
        .primitives
        .iter()
        .filter(|p| matches!(p, ScenePrimitive::Path { path, .. } if path.commands.iter().any(|c| matches!(c, PathCmd::CubicTo { .. }))))
        .count();
    assert!(cubic_count >= 4, "expected spline and circular Bezier paths, got {cubic_count}");

    // Must contain Okabe-Ito primary blue & area fill
    assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { fill: Some(f), .. } if f.color.a == 55)));
}

#[test]
fn xy_broken_axes_split_curves_and_emit_log_warnings() {
    let mut doc = ProjectDocument::new("Broken & Log").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 7).unwrap();
    let xs = [1.5, 2.0, 5.0, 8.0, 120.0, 150.0, 180.0];
    let ys = [-3.0, 4.0, 8.0, 10.0, 14.0, 16.0, 18.0];
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        if r == 0 {
            // Negative mean (-3.0) inside first X segment triggers non_positive_on_log_axis on log Y
            xy.data_sets[0].cells[r][0] = Cell::number(-5.0);
            xy.data_sets[0].cells[r][1] = Cell::number(-1.0);
        } else {
            // Large spread on r == 1 (mean +4.0, lower error bar < 0) triggers log_axis_floor_clip
            xy.data_sets[0].cells[r][0] = Cell::number(y - 7.0);
            xy.data_sets[0].cells[r][1] = Cell::number(y + 7.0);
        }
    }
    doc.add_data_sheet(DataSheet::new("d-brk", "Broken", TableArchetype::Xy(xy)).unwrap()).unwrap();

    // Add fitted curve overlay spanning both broken segments [1..=10] and [100..=200]
    let fit_spec = AnalysisSpec::new("fit-brk", "d-brk", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("michaelis_menten"))).unwrap();
    let mut fit_res = curvebook_doc::ResultSheet::new(fit_spec).unwrap();
    let pts = (1..=200)
        .map(|i| {
            let x = i as f64;
            curvebook_doc::CurveSamplePoint {
                x,
                y: 2.0 + 0.1 * x,
                ci_lower: Some(1.0 + 0.1 * x),
                ci_upper: Some(3.0 + 0.1 * x),
                pred_lower: None,
                pred_upper: None,
            }
        })
        .collect();
    fit_res.fitted_curves.push(curvebook_doc::FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: true,
        ambiguous: false,
        points: pts,
    });
    doc.add_result_sheet(fit_res).unwrap();

    let mut graph = GraphSheet::new_uninitialized_for_data("g-brk", doc.data_sheet(&SheetId::new("d-brk")).unwrap()).unwrap();
    graph.spec.x_axis.break_ornament = BreakOrnament::Zigzag;
    graph.spec.x_axis.segments = vec![
        AxisSegment {
            min: Some(1.0),
            max: Some(10.0),
            scale: AxisScale::Linear,
            length_share: 0.5,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
            ..AxisSegment::default()
        },
        AxisSegment {
            min: Some(100.0),
            max: Some(200.0),
            scale: AxisScale::Linear,
            length_share: 0.5,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
            ..AxisSegment::default()
        },
    ];
    graph.spec.y_axis.segments = vec![AxisSegment {
        min: Some(0.5),
        max: Some(100.0),
        scale: AxisScale::Log10,
        length_share: 1.0,
        major_tick_interval: None,
        minor_ticks_per_interval: 1,
        ..AxisSegment::default()
    }];
    graph.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
        marker: Some(MarkerStyle::default()),
        line: Some(ConnectingLineStyle {
            kind: LineConnectionKind::PointToPoint,
            dash_style: LineDashStyle::Solid,
            stroke_width_pt: 1.2,
            color: Rgba8::BLACK,
        }),
        area_fill: None,
        replicate_lines: curvebook_doc::ReplicateLineMode::default(),
    };
    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("fit-brk"),
        sample_segments: 280,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: true,
    });

    let scene = compile_graph(&graph, &doc).unwrap();
    assert!(all_points_finite(&scene));

    let codes: Vec<&str> = scene.warnings.iter().map(|w| w.code.as_str()).collect();
    assert!(codes.contains(&"non_positive_on_log_axis"), "expected log omission warning for negative Y replicate/mean, got {codes:?}");
    assert!(codes.contains(&"log_axis_floor_clip"), "expected log floor clip warning for lower error bar <= 0, got {codes:?}");

    // Ensure connecting line / fitted curve paths NEVER span across the X axis break gap midpoint
    let r = scene.plot_rect_pt;
    let gap_mid_x = r.x + 0.5 * r.width;
    let crosses_gap = scene.primitives.iter().any(|p| match p {
        ScenePrimitive::Path { path, fill: None, stroke: Some(s), .. } if s.width_pt >= 1.15 => {
            let xs: Vec<f64> = path
                .commands
                .iter()
                .filter_map(|c| match *c {
                    PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => Some(pt.x),
                    _ => None,
                })
                .collect();
            xs.len() >= 2 && xs.iter().any(|&x| x < gap_mid_x - 2.0) && xs.iter().any(|&x| x > gap_mid_x + 2.0)
        }
        _ => false,
    });
    assert!(!crosses_gap, "fitted curve or connecting line illegally bridged discontinuous axis gap");
}

#[test]
fn column_grouped_and_nested_all_gdos_and_arrangements() {
    // 1. Column table across BoxAndWhisker (with Tukey outlier), Violin (full & half), Bar+Beeswarm, AlignedDotPlot, FloatingBar
    let mut doc = ProjectDocument::new("Column & Violin").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 5, 8).unwrap();
    let samples = [
        [10.0, 12.0, 11.0, 13.0, 12.5, 11.5, 10.5, 42.0], // Box Tukey outlier at 42.0
        [18.0, 20.0, 19.0, 22.0, 21.0, 23.0, 20.5, 19.5], // Violin full
        [24.0, 25.0, 26.0, 28.0, 30.0, 27.0, 25.5, 29.0], // Half violin + points
        [15.0, 16.0, 14.0, 17.0, 15.5, 16.5, 15.2, 16.8], // Bar + DeterministicBeeswarm
        [30.0, 32.0, 31.0, 33.0, 29.0, 34.0, 31.5, 32.5], // AlignedDotPlot + FloatingBar check
    ];
    for (c, vals) in samples.iter().enumerate() {
        col.data_sets[c].title = format!("G{}", c + 1);
        for (r, &v) in vals.iter().enumerate() {
            col.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    doc.add_data_sheet(DataSheet::new("d-col", "Column", TableArchetype::Column(col)).unwrap()).unwrap();
    let mut g_col = GraphSheet::new_uninitialized_for_data("g-col", doc.data_sheet(&SheetId::new("d-col")).unwrap()).unwrap();
    let palette = PaletteId::OkabeIto;
    g_col.spec.series[0].gdo = GraphicalDataObject::BoxAndWhisker {
        whiskers: WhiskerMethod::Tukey1_5Iqr,
        fill_color: palette.color_at(0).with_alpha(120),
        stroke_color: palette.color_at(0),
        show_outliers: true,
        show_all_points: false,
    };
    g_col.spec.series[1].gdo = GraphicalDataObject::Violin {
        fill_color: palette.color_at(1).with_alpha(120),
        stroke_color: palette.color_at(1),
        clip_to_observed_range: true,
        show_median_and_quartiles: true,
        half_violin_with_points: false,
    };
    g_col.spec.series[2].gdo = GraphicalDataObject::Violin {
        fill_color: palette.color_at(2).with_alpha(120),
        stroke_color: palette.color_at(2),
        clip_to_observed_range: false,
        show_median_and_quartiles: true,
        half_violin_with_points: true,
    };
    g_col.spec.series[3].gdo = GraphicalDataObject::BarWithReplicates {
        summary: SummaryMeasure::GeometricMean,
        fill_color: palette.color_at(3).with_alpha(180),
        border_color: palette.color_at(3),
        border_width_pt: 1.0,
        replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
    };
    g_col.spec.series[4].gdo = GraphicalDataObject::FloatingBar {
        summary: Some(SummaryMeasure::Median),
        fill_color: palette.color_at(4).with_alpha(180),
        border_color: palette.color_at(4),
        border_width_pt: 1.0,
    };

    let scene1 = compile_graph(&g_col, &doc).unwrap();
    let scene2 = compile_graph(&g_col, &doc).unwrap();
    assert_eq!(scene1.primitives, scene2.primitives, "deterministic beeswarm and KDE violin must compile identically");
    assert!(all_points_finite(&scene1));

    // Verify Paired BeforeAfterLines + AlignedDotPlot + Nested table
    let mut paired = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 4).unwrap();
    for r in 0..4 {
        paired.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        paired.data_sets[1].cells[r][0] = Cell::number(18.0 + (r as f64) * 1.5);
    }
    doc.add_data_sheet(DataSheet::new("d-paired", "Paired", TableArchetype::Column(paired)).unwrap()).unwrap();
    let g_paired = GraphSheet::new_uninitialized_for_data("g-paired", doc.data_sheet(&SheetId::new("d-paired")).unwrap()).unwrap();
    let s_paired = compile_graph(&g_paired, &doc).unwrap();
    assert!(all_points_finite(&s_paired));

    let mut nested = NestedTable::new(2, 3, 3).unwrap();
    for g in 0..2 {
        for b in 0..3 {
            for r in 0..3 {
                nested.groups[g].cells[r][b] = Cell::number(20.0 + (g * 15 + b * 2 + r) as f64);
            }
        }
    }
    doc.add_data_sheet(DataSheet::new("d-nested", "Nested", TableArchetype::Nested(nested)).unwrap()).unwrap();
    let mut g_nested = GraphSheet::new_uninitialized_for_data("g-nested", doc.data_sheet(&SheetId::new("d-nested")).unwrap()).unwrap();
    g_nested.spec.series[0].gdo =
        GraphicalDataObject::AlignedDotPlot { summary: SummaryMeasure::Median, marker: MarkerStyle::default(), show_crossbar: true };
    let s_nested = compile_graph(&g_nested, &doc).unwrap();
    assert!(all_points_finite(&s_nested));

    // Grouped arrangements: Interleaved, Separated, Stacked
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).unwrap();
    grp.row_titles = vec![String::from("Day 1"), String::from("Day 7")];
    grp.data_sets[0].title = String::from("Control");
    grp.data_sets[1].title = String::from("Drug");
    for c in 0..2 {
        for r in 0..2 {
            grp.data_sets[c].cells[r] = vec![Cell::number(10.0 + (c * 12 + r * 5) as f64), Cell::number(12.0 + (c * 12 + r * 5) as f64)];
        }
    }
    doc.add_data_sheet(DataSheet::new("d-grp", "Grouped", TableArchetype::Grouped(grp)).unwrap()).unwrap();
    let mut g_grp = GraphSheet::new_uninitialized_for_data("g-grp", doc.data_sheet(&SheetId::new("d-grp")).unwrap()).unwrap();
    g_grp.spec.grouped_arrangement = GroupedArrangement::Interleaved;
    let s_int = compile_graph(&g_grp, &doc).unwrap();
    g_grp.spec.grouped_arrangement = GroupedArrangement::Separated;
    let s_sep = compile_graph(&g_grp, &doc).unwrap();
    g_grp.spec.grouped_arrangement = GroupedArrangement::Stacked;
    let s_stk = compile_graph(&g_grp, &doc).unwrap();
    assert!(all_points_finite(&s_int) && all_points_finite(&s_sep) && all_points_finite(&s_stk));
    assert_ne!(s_int.primitives, s_sep.primitives);
    assert_ne!(s_int.primitives, s_stk.primitives);
}

#[test]
fn live_pairwise_brackets_auto_stacked_and_formatted_from_anova_results() {
    let mut doc = ProjectDocument::new("Brackets").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 4, 5).unwrap();
    let groups = [
        [10.0, 11.0, 10.5, 9.8, 10.2],  // group 0
        [10.3, 10.8, 10.4, 10.1, 10.5], // group 1 (not significant vs 0)
        [18.0, 19.2, 17.8, 18.6, 18.9], // group 2 (highly significant vs 0)
        [27.0, 28.5, 26.8, 27.9, 28.1], // group 3 (highly significant vs 0 and 2)
    ];
    for (c, vals) in groups.iter().enumerate() {
        col.data_sets[c].title = format!("Arm {}", c + 1);
        for (r, &v) in vals.iter().enumerate() {
            col.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    doc.add_data_sheet(DataSheet::new("d-anova", "Arms", TableArchetype::Column(col)).unwrap()).unwrap();

    let mut anova_spec = AnalysisSpec::new(
        "res-anova",
        "d-anova",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::TukeyHsd,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
            },
            ..OneWayAnovaSpec::default()
        }),
    )
    .unwrap();
    anova_spec.p_value_format = PValueFormat::Apa;
    anova_spec.confidence_level = ConfidenceLevel::DEFAULT_95;

    let anova_res = curvebook_stats::run_analysis(doc.data_sheet(&SheetId::new("d-anova")).unwrap(), &anova_spec).unwrap();
    assert!(!anova_res.pairwise_comparisons.is_empty());
    doc.add_result_sheet(anova_res).unwrap();

    let mut graph = GraphSheet::new_uninitialized_for_data("g-anova", doc.data_sheet(&SheetId::new("d-anova")).unwrap()).unwrap();
    graph.spec.frame_preset = FramePreset::OpenL;
    graph.spec.comparisons = vec![
        // ns pair with hide_if_non_significant = true (should be omitted!)
        PairwiseBracketSpec {
            result_sheet_id: SheetId::new("res-anova"),
            group_a: 0,
            group_b: 1,
            style: BracketStyle::FlatLine,
            label_mode: BracketLabelMode::SummaryStars,
            hide_if_non_significant: true,
            stroke_width_pt: 1.0,
            font_size_pt: 8.5,
            bracket_step_pt: 14.0,
        },
        // Significant overlapping comparisons spanning [0..2], [2..3], and outer [0..3]
        PairwiseBracketSpec {
            result_sheet_id: SheetId::new("res-anova"),
            group_a: 0,
            group_b: 2,
            style: BracketStyle::Square,
            label_mode: BracketLabelMode::NumericPOrQ,
            hide_if_non_significant: true,
            stroke_width_pt: 1.0,
            font_size_pt: 8.5,
            bracket_step_pt: 14.0,
        },
        PairwiseBracketSpec {
            result_sheet_id: SheetId::new("res-anova"),
            group_a: 2,
            group_b: 3,
            style: BracketStyle::HalfTick,
            label_mode: BracketLabelMode::SummaryStars,
            hide_if_non_significant: true,
            stroke_width_pt: 1.0,
            font_size_pt: 8.5,
            bracket_step_pt: 14.0,
        },
        PairwiseBracketSpec {
            result_sheet_id: SheetId::new("res-anova"),
            group_a: 0,
            group_b: 3,
            style: BracketStyle::CurlyBrace,
            label_mode: BracketLabelMode::Custom(String::from("Overall **")),
            hide_if_non_significant: false,
            stroke_width_pt: 1.1,
            font_size_pt: 8.5,
            bracket_step_pt: 14.0,
        },
    ];
    graph.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: curvebook_doc::RectPt::new(60.0, 38.0, 90.0, 20.0),
        markdown: String::from("<b>Tukey</b> HSD"),
        bordered: true,
    });

    let scene = compile_graph(&graph, &doc).unwrap();
    assert!(all_points_finite(&scene));

    // Hidden ns bracket should NOT emit "ns"
    let texts: Vec<(String, f64)> = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some((t.rich.plain_text(), t.anchor_pt.y)),
            _ => None,
        })
        .collect();
    assert!(!texts.iter().any(|(s, _)| s == "ns"), "non-significant bracket should have been hidden: {texts:?}");

    // Outer span [0..3] ("Overall **") must be stacked strictly higher (smaller y_pt) than inner [0..2] & [2..3]
    let y_0_2 = texts.iter().find(|(s, _)| s.starts_with("P ")).map(|(_, y)| *y).expect("expected APA P-value bracket label");
    let y_0_3 = texts.iter().find(|(s, _)| s == "Overall **").map(|(_, y)| *y).expect("expected Overall ** bracket label");
    assert!(y_0_3 < y_0_2 - 8.0, "outer bracket [0..3] (y={y_0_3}) must stack above overlapping bracket [0..2] (y={y_0_2})");
}

#[test]
fn frames_thresholds_dual_y2_tick_formats_and_annotations() {
    let mut doc = ProjectDocument::new("Chrome").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4).unwrap();
    for r in 0..4 {
        xy.set_x_cell(r, Cell::number(-2.0 + r as f64)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(1000.0 * (r + 1) as f64);
        xy.data_sets[1].cells[r][0] = Cell::number(0.15 * (r + 1) as f64);
    }
    doc.add_data_sheet(DataSheet::new("d-ch", "Chrome Data", TableArchetype::Xy(xy)).unwrap()).unwrap();

    for preset in [FramePreset::PlainBoxed, FramePreset::OffsetRangeTufte, FramePreset::DualYSecondaryRight, FramePreset::OriginCrossing] {
        let mut g = GraphSheet::new_uninitialized_for_data("g-ch", doc.data_sheet(&SheetId::new("d-ch")).unwrap()).unwrap();
        g.spec.frame_preset = preset;
        g.spec.x_axis.tick_format = TickLabelFormat::Antilog;
        g.spec.y_axis.tick_format = TickLabelFormat::EngineeringSi;
        g.spec.y_axis.thresholds.push(AxisThreshold {
            value: 2000.0,
            shade_to: Some(3000.0),
            color: Rgba8::rgb(200, 50, 50),
            dash_style: LineDashStyle::Dashed,
            stroke_width_pt: 1.0,
            label: Some(String::from("Cutoff")),
        });
        g.spec.series[1].bind_to_right_y2 = true;
        g.spec.annotations = vec![
            GraphAnnotation::Shape {
                kind: AnnotationShapeKind::RoundedRectangle,
                bounds_pt: curvebook_doc::RectPt::new(80.0, 50.0, 40.0, 20.0),
                stroke_color: Rgba8::BLACK,
                stroke_width_pt: 0.8,
                dash: LineDashStyle::Solid,
                fill_color: None,
            },
            GraphAnnotation::LineOrArrow {
                start_pt: (90.0, 70.0),
                end_pt: (120.0, 95.0),
                arrowhead: true,
                dash: LineDashStyle::Solid,
                color: Rgba8::BLACK,
            },
        ];
        let scene = compile_graph(&g, &doc).unwrap();
        assert!(all_points_finite(&scene));
    }

    // Verify RowNumbers tick source on Column graph & ScientificE Y format
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
    for c in 0..2 {
        for r in 0..3 {
            col.data_sets[c].cells[r][0] = Cell::number(5.0 + (c * 3 + r) as f64);
        }
    }
    doc.add_data_sheet(DataSheet::new("d-rn", "RowNums", TableArchetype::Column(col)).unwrap()).unwrap();
    let mut g_rn = GraphSheet::new_uninitialized_for_data("g-rn", doc.data_sheet(&SheetId::new("d-rn")).unwrap()).unwrap();
    g_rn.spec.x_axis.tick_format = TickLabelFormat::Categorical(CategoricalTickSource::RowNumbers);
    g_rn.spec.y_axis.tick_format = TickLabelFormat::ScientificE;
    let s_rn = compile_graph(&g_rn, &doc).unwrap();
    let texts = find_texts(&s_rn);
    assert!(texts.iter().any(|t| t == "1") && texts.iter().any(|t| t == "2"), "expected RowNumbers ticks 1, 2 in {texts:?}");
}

#[test]
fn model_equation_box_live_refresh_across_xy_and_column_graphs() {
    let mut doc = ProjectDocument::new("LiveEq").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
    for (r, (x, y)) in [(-8.0, 8.0), (-7.0, 20.0), (-6.0, 52.0), (-5.0, 84.0), (-4.0, 96.0), (-3.0, 99.0)].into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let xy_ds = DataSheet::new("ds-xy", "Dose", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(xy_ds.clone()).unwrap();

    let nl_spec =
        AnalysisSpec::new("res-nl", "ds-xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).unwrap();
    let nl_res = execute_analysis(&nl_spec, &xy_ds, &HookContext::default()).unwrap();
    assert_eq!(nl_res.status, ResultStatus::UpToDate);
    doc.add_result_sheet(nl_res).unwrap();

    let lin_spec = AnalysisSpec::new("res-lin", "ds-xy", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
    let mut lin_res = ResultSheet::new(lin_spec).unwrap();
    lin_res.status = ResultStatus::UpToDate;
    lin_res.scalars = vec![
        ResultScalarEntry { key: String::from("Slope"), data_set_index: Some(0), value: 14.5, units: None },
        ResultScalarEntry { key: String::from("YIntercept"), data_set_index: Some(0), value: 3.25, units: None },
    ];
    doc.add_result_sheet(lin_res).unwrap();

    let mut g = GraphSheet::new_uninitialized_for_data("g-eq", doc.data_sheet(&SheetId::new("ds-xy")).unwrap()).unwrap();
    g.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-nl"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(90.0, 40.0, 230.0, 54.0),
        font_size_pt: 9.0,
        bordered: true,
    });
    g.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-lin"),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: DocRectPt::new(90.0, 105.0, 160.0, 24.0),
        font_size_pt: 8.5,
        bordered: false,
    });

    let scene_before = compile_graph(&g, &doc).unwrap();
    assert!(all_points_finite(&scene_before));
    let texts_before = find_texts(&scene_before).join(" | ");
    assert!(texts_before.contains("Slope") && texts_before.contains("Intercept"), "symbolic linear missing: {texts_before}");
    assert!(!texts_before.contains("777"), "sentinel should not exist before mutation: {texts_before}");

    // Mutate Top scalar directly without touching GraphSpec and re-compile -> live refresh!
    let top_entry =
        doc.result_sheets.iter_mut().find(|r| r.id == SheetId::new("res-nl")).unwrap().scalars.iter_mut().find(|s| s.key == "Top").unwrap();
    top_entry.value = 777.0;

    let scene_after = compile_graph(&g, &doc).unwrap();
    assert!(all_points_finite(&scene_after));
    let texts_after = find_texts(&scene_after).join(" | ");
    assert!(texts_after.contains("777"), "live scalar refresh failed: {texts_after}");

    // Column family also compiles ModelEquation annotations via xy::render_legend_and_annotations.
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
    for c in 0..2 {
        for r in 0..3 {
            col.data_sets[c].cells[r][0] = Cell::number((c * 3 + r + 1) as f64);
        }
    }
    doc.add_data_sheet(DataSheet::new("ds-col", "Groups", TableArchetype::Column(col)).unwrap()).unwrap();
    let mut g_col = GraphSheet::new_uninitialized_for_data("g-col", doc.data_sheet(&SheetId::new("ds-col")).unwrap()).unwrap();
    g_col.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-lin"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(80.0, 40.0, 160.0, 26.0),
        font_size_pt: 9.0,
        bordered: true,
    });
    let col_scene = compile_graph(&g_col, &doc).unwrap();
    assert!(all_points_finite(&col_scene));
    let col_texts = find_texts(&col_scene).join(" | ");
    assert!(col_texts.contains("14.5"), "col_texts={col_texts}");
    assert!(col_texts.contains("3.25"), "col_texts={col_texts}");
}

#[test]
fn auto_legend_stays_outside_plot_rect_for_dose_response_and_column_labels() {
    fn legend_box_x_bounds(scene: &curvebook_chart::ChartScene) -> Option<(f64, f64)> {
        scene.primitives.iter().find_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(fill), stroke: Some(_), clip: None }
                if fill.color == curvebook_palette::Rgba8::rgba(255, 255, 255, 235) =>
            {
                let mut min_x = f64::INFINITY;
                let mut max_x = f64::NEG_INFINITY;
                for cmd in &path.commands {
                    match *cmd {
                        PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => {
                            min_x = min_x.min(pt.x);
                            max_x = max_x.max(pt.x);
                        }
                        _ => {}
                    }
                }
                (min_x.is_finite() && max_x.is_finite()).then_some((min_x, max_x))
            }
            _ => None,
        })
    }

    // 1. XY dose-response curve with realistic series titles (>8 chars) on default 360x260 canvas.
    let mut xy = XyTable::new(SubcolumnMode::Single, 3, 5).unwrap();
    xy.data_sets[0].title = String::from("Wild-Type");
    xy.data_sets[1].title = String::from("Variant R1");
    xy.data_sets[2].title = String::from("Compound A (10 uM)");
    for (row, x) in [-9.0, -8.0, -7.0, -6.0, -5.0].into_iter().enumerate() {
        xy.x_cells[row] = vec![Cell::number(x)];
        xy.data_sets[0].cells[row] = vec![Cell::number(10.0 + (row as f64) * 20.0)];
        xy.data_sets[1].cells[row] = vec![Cell::number(12.0 + (row as f64) * 16.0)];
        xy.data_sets[2].cells[row] = vec![Cell::number(8.0 + (row as f64) * 12.0)];
    }
    let ds = DataSheet::new("ds-dose", "Dose-Response", TableArchetype::Xy(xy)).unwrap();
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone()).unwrap();
    let mut graph = GraphSheet::new_uninitialized_for_data("g-dose", &ds).unwrap();
    graph.spec.legend = curvebook_doc::LegendSpec { visible: true, position_pt: None, bordered: true };

    let scene = compile_graph(&graph, &doc).unwrap();
    let (bx0, bx1) = legend_box_x_bounds(&scene).expect("bordered legend rect");
    assert!(
        bx0 > scene.plot_rect_pt.right(),
        "XY dose-response legend overlapped plot: legend=[{bx0:.1}, {bx1:.1}], plot_right={:.1}",
        scene.plot_rect_pt.right()
    );
    assert!(bx1 <= scene.width_pt, "XY dose-response legend overflowed canvas: bx1={bx1:.1}, width={:.1}", scene.width_pt);
    for expected in ["Wild-Type", "Variant R1", "Compound A (10 uM)"] {
        let label_x = scene
            .primitives
            .iter()
            .find_map(|p| match p {
                ScenePrimitive::Text(t) if t.rich.plain_text() == expected => Some(t.anchor_pt.x),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing legend label {expected}"));
        assert!(label_x > scene.plot_rect_pt.right(), "legend label {expected} overlapped plot: x={label_x:.1}");
    }

    // 2. Very long label still scales gently to stay outside-right without overlapping or truncating.
    let long_label = "Compound A + 100 nM Inhibitor B";
    graph.spec.series[2].legend_label = Some(String::from(long_label));
    let long_scene = compile_graph(&graph, &doc).unwrap();
    let (lbx0, lbx1) = legend_box_x_bounds(&long_scene).expect("bordered long legend rect");
    assert!(
        lbx0 > long_scene.plot_rect_pt.right(),
        "long legend overlapped plot: legend=[{lbx0:.1}, {lbx1:.1}], plot_right={:.1}",
        long_scene.plot_rect_pt.right()
    );
    assert!(lbx1 <= long_scene.width_pt, "long legend overflowed canvas: lbx1={lbx1:.1}, width={:.1}", long_scene.width_pt);
    assert!(find_texts(&long_scene).iter().any(|t| t == long_label), "long legend label must not be truncated");

    // 3. Column graph with multi-word group titles also keeps auto legend outside-right.
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
    col.data_sets[0].title = String::from("Vehicle Control");
    col.data_sets[1].title = String::from("Inhibitor 100 nM");
    for c in 0..2 {
        for r in 0..3 {
            col.data_sets[c].cells[r][0] = Cell::number((c * 10 + r + 5) as f64);
        }
    }
    let ds_col = DataSheet::new("ds-col-leg", "Groups", TableArchetype::Column(col)).unwrap();
    doc.add_data_sheet(ds_col.clone()).unwrap();
    let mut g_col = GraphSheet::new_uninitialized_for_data("g-col-leg", &ds_col).unwrap();
    g_col.spec.legend = curvebook_doc::LegendSpec { visible: true, position_pt: None, bordered: true };
    let col_scene = compile_graph(&g_col, &doc).unwrap();
    let (cbx0, cbx1) = legend_box_x_bounds(&col_scene).expect("bordered column legend rect");
    assert!(
        cbx0 > col_scene.plot_rect_pt.right(),
        "column legend overlapped plot: legend=[{cbx0:.1}, {cbx1:.1}], plot_right={:.1}",
        col_scene.plot_rect_pt.right()
    );
    assert!(cbx1 <= col_scene.width_pt, "column legend overflowed canvas: cbx1={cbx1:.1}, width={:.1}", col_scene.width_pt);
}

#[test]
fn auto_legend_respects_parts_of_whole_horizontal_overlays_grouped_and_y2() {
    use curvebook_doc::{CartesianOrientation, GroupedTable, PartsOfWholeTable};

    fn legend_box_x_bounds(scene: &curvebook_chart::ChartScene) -> Option<(f64, f64)> {
        scene.primitives.iter().find_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(fill), stroke: Some(_), clip: None }
                if fill.color == curvebook_palette::Rgba8::rgba(255, 255, 255, 235) =>
            {
                let mut min_x = f64::INFINITY;
                let mut max_x = f64::NEG_INFINITY;
                for cmd in &path.commands {
                    match *cmd {
                        PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => {
                            min_x = min_x.min(pt.x);
                            max_x = max_x.max(pt.x);
                        }
                        _ => {}
                    }
                }
                (min_x.is_finite() && max_x.is_finite()).then_some((min_x, max_x))
            }
            _ => None,
        })
    }

    let mut doc = ProjectDocument::default();

    // 1. Parts-of-Whole: formatted "Name (XX.X%)" legend must fit outside-right at full font size.
    let mut pow = PartsOfWholeTable::new(1, 2).unwrap();
    pow.slice_labels = vec![String::from("Responder"), String::from("Non-responder")];
    pow.data_sets[0].cells = vec![vec![Cell::number(65.0)], vec![Cell::number(35.0)]];
    let ds_pow = DataSheet::new("ds-pow", "Response", TableArchetype::PartsOfWhole(pow)).unwrap();
    doc.add_data_sheet(ds_pow.clone()).unwrap();
    let mut g_pow = GraphSheet::new_uninitialized_for_data("g-pow", &ds_pow).unwrap();
    g_pow.spec.legend = curvebook_doc::LegendSpec { visible: true, position_pt: None, bordered: true };
    let pow_scene = compile_graph(&g_pow, &doc).unwrap();
    let (px0, px1) = legend_box_x_bounds(&pow_scene).expect("bordered parts-of-whole legend");
    assert!(px0 > pow_scene.plot_rect_pt.right(), "PoW legend overlapped plot: [{px0:.1}, {px1:.1}] vs {:.1}", pow_scene.plot_rect_pt.right());
    assert!(px1 <= pow_scene.width_pt, "PoW legend overflowed canvas: {px1:.1} > {:.1}", pow_scene.width_pt);

    // 2. Grouped table with empty `spec.series` still reserves right margin for auto-built legend items.
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, 2).unwrap();
    grp.data_sets[0].title = String::from("Wild-Type");
    grp.data_sets[1].title = String::from("Mutant K1");
    for c in 0..2 {
        for r in 0..2 {
            grp.data_sets[c].cells[r][0] = Cell::number((c * 5 + r + 10) as f64);
        }
    }
    let ds_grp = DataSheet::new("ds-grp-empty-series", "Grouped", TableArchetype::Grouped(grp)).unwrap();
    doc.add_data_sheet(ds_grp.clone()).unwrap();
    let mut g_grp = GraphSheet::new_uninitialized_for_data("g-grp-empty-series", &ds_grp).unwrap();
    g_grp.spec.series.clear();
    g_grp.spec.legend = curvebook_doc::LegendSpec { visible: true, position_pt: None, bordered: true };
    let grp_scene = compile_graph(&g_grp, &doc).unwrap();
    let (gx0, gx1) = legend_box_x_bounds(&grp_scene).expect("bordered grouped legend");
    assert!(gx0 > grp_scene.plot_rect_pt.right(), "grouped legend overlapped plot: [{gx0:.1}, {gx1:.1}] vs {:.1}", grp_scene.plot_rect_pt.right());
    assert!(gx1 <= grp_scene.width_pt, "grouped legend overflowed canvas: {gx1:.1} > {:.1}", grp_scene.width_pt);

    // 3. Horizontal grouped chart with brackets/CLD offsets auto legend past the overlay band.
    let mut g_horiz = GraphSheet::new_uninitialized_for_data("g-grp-horiz", &ds_grp).unwrap();
    g_horiz.spec.orientation = CartesianOrientation::Horizontal;
    g_horiz.spec.legend = curvebook_doc::LegendSpec { visible: true, position_pt: None, bordered: true };
    let horiz_scene = compile_graph(&g_horiz, &doc).unwrap();
    let (hx0, hx1) = legend_box_x_bounds(&horiz_scene).expect("bordered horizontal legend");
    assert!(
        hx0 > horiz_scene.plot_rect_pt.right(),
        "horizontal legend overlapped plot: [{hx0:.1}, {hx1:.1}] vs {:.1}",
        horiz_scene.plot_rect_pt.right()
    );
    assert!(hx1 <= horiz_scene.width_pt, "horizontal legend overflowed canvas: {hx1:.1} > {:.1}", horiz_scene.width_pt);

    // 4. Dual-Y2 XY chart places auto legend outside-right past the Y2 axis band.
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 3).unwrap();
    xy.data_sets[0].title = String::from("Signal A");
    xy.data_sets[1].title = String::from("Signal B");
    for r in 0..3 {
        xy.x_cells[r] = vec![Cell::number((r + 1) as f64)];
        xy.data_sets[0].cells[r] = vec![Cell::number((r + 1) as f64 * 10.0)];
        xy.data_sets[1].cells[r] = vec![Cell::number((r + 1) as f64 * 2.0)];
    }
    let ds_y2 = DataSheet::new("ds-y2-leg", "Dual Y2", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(ds_y2.clone()).unwrap();
    let mut g_y2 = GraphSheet::new_uninitialized_for_data("g-y2-leg", &ds_y2).unwrap();
    g_y2.spec.y2_axis = Some(curvebook_doc::AxisSpec::linear("Y2"));
    g_y2.spec.series[1].bind_to_right_y2 = true;
    g_y2.spec.legend = curvebook_doc::LegendSpec { visible: true, position_pt: None, bordered: true };
    let y2_scene = compile_graph(&g_y2, &doc).unwrap();
    let (yx0, yx1) = legend_box_x_bounds(&y2_scene).expect("bordered dual-Y2 legend");
    assert!(
        yx0 >= y2_scene.plot_rect_pt.right() + 48.0,
        "dual-Y2 legend overlapped plot or Y2 band: [{yx0:.1}, {yx1:.1}], plot_right={:.1}",
        y2_scene.plot_rect_pt.right()
    );
    assert!(yx1 <= y2_scene.width_pt, "dual-Y2 legend overflowed canvas: {yx1:.1} > {:.1}", y2_scene.width_pt);
}
