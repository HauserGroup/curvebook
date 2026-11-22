//! End-to-end regression tests for Wave 1 SEC-09 S1 findings (`FA-SEC09-01`..`FA-SEC09-05`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::equation_box::render_tex_equation_box;
use curvebook_chart::{
    compile_graph,
    scene::{ChartScene, PathCmd, PointPt, ScenePrimitive, preflight_check_scene},
};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, AxisSpec, BlandAltmanSpec, Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, DataSheet,
    DiagnosticPlotKind, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, GraphKind, GraphSheet, GraphicalDataObject, GroupedTable,
    LineConnectionKind, MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode, PointStyleOverride, ProjectDocument, RectPt,
    ResultCell, ResultScalarEntry, ResultSheet, ResultTab, ResultTabKind, SheetId, SizeChannelMapping, SubcolumnMode, SummaryMeasure, TableArchetype,
    VariableRole, XyTable,
};
use curvebook_math::ConfidenceLevel;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::two_group::run_bland_altman_analysis;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn make_doc(title: &str) -> TestResult<ProjectDocument> {
    Ok(ProjectDocument::new(title)?)
}

fn log10_axis(title: &str) -> AxisSpec {
    let mut axis = AxisSpec::linear(title);
    axis.segments[0].scale = AxisScale::Log10;
    axis
}

fn mv_encoding(plot_mode: MultivariatePlotMode, x_column: usize, y_column: usize) -> MultivariateGraphEncoding {
    MultivariateGraphEncoding {
        plot_mode,
        x_column,
        y_column,
        color: ColorChannelMapping::Constant(Rgba8::rgb(40, 80, 120)),
        size: SizeChannelMapping::Constant { radius_pt: 4.5 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    }
}

fn plain_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn line_segments_with_color(scene: &ChartScene, color: Rgba8) -> Vec<(PointPt, PointPt, f64)> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, stroke: Some(st), .. } if st.color == color && path.commands.len() >= 2 => {
                if let (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b))) = (path.commands.first(), path.commands.get(1)) {
                    Some((*a, *b, st.width_pt))
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
}

fn extract_y_tick_numbers(scene: &ChartScene) -> Vec<f64> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => {
                let s = t.rich.plain_text().replace('\u{2212}', "-");
                if t.anchor_pt.x < scene.plot_rect_pt.x + 2.0
                    && t.anchor_pt.y >= scene.plot_rect_pt.y - 10.0
                    && t.anchor_pt.y <= scene.plot_rect_pt.bottom() + 10.0
                {
                    s.parse::<f64>().ok()
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
}

fn path_fill_spans_with_color(scene: &ChartScene, color: Rgba8) -> Vec<(f64, f64)> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(f), .. } if f.color == color => {
                let mut min_x = f64::INFINITY;
                let mut max_x = f64::NEG_INFINITY;
                let mut min_y = f64::INFINITY;
                let mut max_y = f64::NEG_INFINITY;
                for cmd in &path.commands {
                    if let PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) = cmd {
                        min_x = min_x.min(pt.x);
                        max_x = max_x.max(pt.x);
                        min_y = min_y.min(pt.y);
                        max_y = max_y.max(pt.y);
                    }
                }
                (min_x.is_finite() && min_y.is_finite()).then_some((max_x - min_x, max_y - min_y))
            }
            _ => None,
        })
        .collect()
}

// ============================================================================
// FA-SEC09-01: equation_box.rs TeX script parser infinite loop & OOM
// ============================================================================

#[test]
fn sec09_01_equation_box_malformed_scripts_terminate_without_oom() {
    let deeply_chained = format!("y = x{}", "^2".repeat(120));
    let deeply_nested = format!("{}x^2{}", "{".repeat(120), "}".repeat(120));
    let cases = ["x^", "x_ ", "y = ^", "y = ^{unclosed", "^}", "_}", "x^1^2^3", deeply_chained.as_str(), deeply_nested.as_str()];

    for tex in cases {
        let mut scene = ChartScene::new("eq", "Eq", 320.0, 200.0, Some(Rgba8::WHITE));
        render_tex_equation_box(&mut scene, RectPt::new(10.0, 10.0, 260.0, 44.0), tex, 10.0, true);
        assert!(!scene.primitives.is_empty(), "expected primitives for {tex:?}");
        assert!(scene.primitives.len() < 5_000, "primitive count exploded for {tex:?}: {}", scene.primitives.len());
    }

    let mut caret_scene = ChartScene::new("eq", "Eq", 320.0, 200.0, Some(Rgba8::WHITE));
    render_tex_equation_box(&mut caret_scene, RectPt::new(10.0, 10.0, 260.0, 44.0), "y = ^", 10.0, false);
    let rendered = plain_texts(&caret_scene).join("");
    assert!(rendered.contains('^'), "dangling ^ should render literally: {rendered:?}");

    let mut brace_scene = ChartScene::new("eq", "Eq", 320.0, 200.0, Some(Rgba8::WHITE));
    render_tex_equation_box(&mut brace_scene, RectPt::new(10.0, 10.0, 260.0, 44.0), "^}", 10.0, false);
    let rendered_brace = plain_texts(&brace_scene).join("");
    assert!(rendered_brace.contains('^'), "unmatched ^}} should consume ^ literally: {rendered_brace:?}");
}

// ============================================================================
// FA-SEC09-02: CategoricalSummary Student-t CI, geometric mean, domain, styling
// ============================================================================

#[test]
fn sec09_02_categorical_summary_student_t_ci_and_insufficient_n_warning() -> TestResult {
    // Small bucket: N=2 ([10.0, 20.0]) -> mean=15.0, SEM=5.0.
    // Exact 95% Student-t CI: df=1 => t_{0.975,1} = 12.706204736... => half-width = 63.531 ([-48.531, 78.531]).
    // Wrong hardcoded 1.96 would give half-width = 9.8 ([5.2, 24.8]).
    let mut mv = MultipleVariablesTable::new(2, 3)?;
    mv.variables[0].title = String::from("Group");
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[1].title = String::from("Value");
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[0].cells[0] = Cell::text("Pair");
    mv.variables[1].cells[0] = Cell::number(10.0);
    mv.variables[0].cells[1] = Cell::text("Pair");
    mv.variables[1].cells[1] = Cell::number(20.0);
    mv.variables[0].cells[2] = Cell::text("Solo");
    mv.variables[1].cells[2] = Cell::number(15.0);

    let ds = DataSheet::new(SheetId::new("mv-ci"), "CI Data", TableArchetype::MultipleVariables(mv))?;
    let mut doc = make_doc("CatCI")?;
    doc.add_data_sheet(ds.clone())?;

    let eb_color = Rgba8::rgb(12, 34, 56);
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cat-ci"), &ds)?;
    gs.initialized = true;
    gs.spec.kind = GraphKind::MultipleVariables;
    gs.spec.multivariate = Some(mv_encoding(
        MultivariatePlotMode::CategoricalSummary {
            gdo: GraphicalDataObject::AlignedDotPlot {
                summary: SummaryMeasure::Mean,
                marker: MarkerStyle {
                    shape: MarkerShape::Circle,
                    fill_mode: MarkerFillMode::Filled,
                    size_pt: 4.5,
                    fill_color: Rgba8::rgb(40, 80, 120),
                    stroke_color: Rgba8::WHITE,
                    stroke_width_pt: 0.5,
                },
                show_crossbar: true,
            },
            y_error_bars: Some(ErrorBarSpec {
                metric: ErrorBarMetric::ConfidenceInterval95,
                direction: ErrorBarDirection::Both,
                dash_style: LineDashStyle::Solid,
                stroke_width_pt: 1.4,
                cap_width_pt: 8.0,
                color: Some(eb_color),
            }),
        },
        0,
        1,
    ));

    let scene = compile_graph(&gs, &doc)?;
    assert!(scene.warnings.iter().any(|w| w.code == "insufficient_sample_size_for_ci"), "expected N=1 CI omission warning, got {:?}", scene.warnings);

    let y_ticks = extract_y_tick_numbers(&scene);
    let y_tick_min = y_ticks.iter().copied().fold(f64::INFINITY, f64::min);
    let y_tick_max = y_ticks.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        y_tick_max >= 50.0 && y_tick_min <= -40.0,
        "Student-t 95% CI extents [-48.53, +78.53] must expand axis domain beyond [5.2, 24.8]: ticks={y_ticks:?}"
    );

    let eb_lines = line_segments_with_color(&scene, eb_color);
    let has_tall_stem = eb_lines.iter().any(|(a, b, _)| (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() > 0.65 * scene.plot_rect_pt.height);
    assert!(has_tall_stem, "expected vertical Student-t CI stem spanning >65% of plot height: {eb_lines:?}");
    let cap_count = eb_lines.iter().filter(|(a, b, _)| (a.y - b.y).abs() < 1e-6 && (a.x - b.x).abs() > 7.5).count();
    assert_eq!(cap_count, 2, "expected 2 error caps for N=2 group only (omitted for N=1)");
    Ok(())
}

#[test]
fn sec09_02_categorical_summary_geom_mean_capless_log_clamp_before_after_and_overrides() -> TestResult {
    // 1. GeometricMean warns & omits non-positive group while still rendering positive group.
    let mut mv = MultipleVariablesTable::new(2, 5)?;
    mv.variables[0].title = String::from("Arm");
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[1].title = String::from("Signal");
    mv.variables[1].role = VariableRole::Continuous;
    let rows = [("ZeroArm", 0.0), ("ZeroArm", 4.0), ("ZeroArm", 8.0), ("PosArm", 2.0), ("PosArm", 8.0)];
    for (r, (grp, val)) in rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(grp);
        mv.variables[1].cells[r] = Cell::number(val);
    }
    let ds = DataSheet::new(SheetId::new("mv-gm"), "GeomMean", TableArchetype::MultipleVariables(mv))?;
    let mut doc = make_doc("CatGM")?;
    doc.add_data_sheet(ds.clone())?;

    let bar_border = Rgba8::rgb(11, 22, 33);
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-gm"), &ds)?;
    gs.initialized = true;
    gs.spec.kind = GraphKind::MultipleVariables;
    gs.spec.multivariate = Some(mv_encoding(
        MultivariatePlotMode::CategoricalSummary {
            gdo: GraphicalDataObject::BarWithReplicates {
                summary: SummaryMeasure::GeometricMean,
                fill_color: Rgba8::rgb(100, 140, 180),
                border_color: bar_border,
                border_width_pt: 1.0,
                replicate_overlay: None,
            },
            y_error_bars: None,
        },
        0,
        1,
    ));
    let scene_gm = compile_graph(&gs, &doc)?;
    assert!(
        scene_gm.warnings.iter().any(|w| w.code == "non_positive_geometric_mean"),
        "expected non_positive_geometric_mean warning, got {:?}",
        scene_gm.warnings
    );
    let filled_bars = scene_gm
        .primitives
        .iter()
        .filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), stroke: Some(st), .. } if st.color == bar_border))
        .count();
    assert_eq!(filled_bars, 1, "only PosArm should emit geometric mean bar; ZeroArm must be omitted (no arithmetic fallback)");

    // 2. Capless error bars (`cap_width_pt = 0.0`) & Log Y clamping when lower bound <= 0.
    let mut mv_log = MultipleVariablesTable::new(2, 2)?;
    mv_log.variables[0].title = String::from("Arm");
    mv_log.variables[0].role = VariableRole::Categorical;
    mv_log.variables[1].title = String::from("Conc");
    mv_log.variables[1].role = VariableRole::Continuous;
    mv_log.variables[0].cells[0] = Cell::text("LogArm");
    mv_log.variables[1].cells[0] = Cell::number(1.0);
    mv_log.variables[0].cells[1] = Cell::text("LogArm");
    mv_log.variables[1].cells[1] = Cell::number(100.0); // mean=50.5, SD=70.00 -> raw_lo = -19.5 <= 0
    let ds_log = DataSheet::new(SheetId::new("mv-log"), "LogCat", TableArchetype::MultipleVariables(mv_log))?;
    doc.add_data_sheet(ds_log.clone())?;

    let log_eb_color = Rgba8::rgb(177, 44, 55);
    let mut gs_log = GraphSheet::new_uninitialized_for_data(SheetId::new("g-log"), &ds_log)?;
    gs_log.initialized = true;
    gs_log.spec.kind = GraphKind::MultipleVariables;
    gs_log.spec.y_axis = log10_axis("Conc");
    gs_log.spec.multivariate = Some(mv_encoding(
        MultivariatePlotMode::CategoricalSummary {
            gdo: GraphicalDataObject::AlignedDotPlot {
                summary: SummaryMeasure::Mean,
                marker: MarkerStyle {
                    shape: MarkerShape::Circle,
                    fill_mode: MarkerFillMode::Filled,
                    size_pt: 4.0,
                    fill_color: Rgba8::BLACK,
                    stroke_color: Rgba8::WHITE,
                    stroke_width_pt: 0.5,
                },
                show_crossbar: true,
            },
            y_error_bars: Some(ErrorBarSpec {
                metric: ErrorBarMetric::Sd,
                direction: ErrorBarDirection::Both,
                dash_style: LineDashStyle::Solid,
                stroke_width_pt: 1.5,
                cap_width_pt: 0.0,
                color: Some(log_eb_color),
            }),
        },
        0,
        1,
    ));
    let scene_log = compile_graph(&gs_log, &doc)?;
    let log_eb_lines = line_segments_with_color(&scene_log, log_eb_color);
    let vertical_stems = log_eb_lines.iter().filter(|(a, b, _)| (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() > 2.0).count();
    let horizontal_caps = log_eb_lines.iter().filter(|(a, b, _)| (a.y - b.y).abs() < 1e-6 && (a.x - b.x).abs() > 1.0).count();
    assert_eq!(vertical_stems, 1, "log lower bound <= 0 should clamp & still render vertical stem: {log_eb_lines:?}");
    assert_eq!(horizontal_caps, 0, "cap_width_pt = 0.0 should emit capless stem with zero caps");

    // 3. BeforeAfterLines connecting stroke, custom marker, per-cell PointStyleOverride, hit_test_point, plus ScatterAndLine styling.
    let mut mv_ba = MultipleVariablesTable::new(2, 4)?;
    mv_ba.variables[0].title = String::from("Stage");
    mv_ba.variables[0].role = VariableRole::Categorical;
    mv_ba.variables[1].title = String::from("Measure");
    mv_ba.variables[1].role = VariableRole::Continuous;
    for (r, (st, val)) in [("Pre", 10.0), ("Pre", 12.0), ("Post", 20.0), ("Post", 24.0)].into_iter().enumerate() {
        mv_ba.variables[0].cells[r] = Cell::text(st);
        mv_ba.variables[1].cells[r] = Cell::number(val);
    }
    let ds_ba = DataSheet::new(SheetId::new("mv-ba"), "BeforeAfter", TableArchetype::MultipleVariables(mv_ba))?;
    doc.add_data_sheet(ds_ba.clone())?;

    let conn_color = Rgba8::rgb(200, 10, 20);
    let override_color = Rgba8::rgb(0, 220, 110);
    let mut gs_ba = GraphSheet::new_uninitialized_for_data(SheetId::new("g-ba"), &ds_ba)?;
    gs_ba.initialized = true;
    gs_ba.spec.kind = GraphKind::MultipleVariables;
    gs_ba.spec.multivariate = Some(mv_encoding(
        MultivariatePlotMode::CategoricalSummary {
            gdo: GraphicalDataObject::BeforeAfterLines {
                marker: Some(MarkerStyle {
                    shape: MarkerShape::Square,
                    fill_mode: MarkerFillMode::Filled,
                    size_pt: 5.0,
                    fill_color: Rgba8::rgb(50, 60, 70),
                    stroke_color: Rgba8::WHITE,
                    stroke_width_pt: 0.6,
                }),
                line_color: conn_color,
                stroke_width_pt: 2.5,
            },
            y_error_bars: None,
        },
        0,
        1,
    ));
    gs_ba.spec.point_overrides = vec![PointStyleOverride {
        series_index: 1,
        point_index: 2, // row 2 -> Post 20.0
        subcol: None,
        color: Some(override_color),
        symbol: Some(MarkerShape::Diamond),
        size_pt: Some(12.0),
        border_width_pt: None,
        fill_color: Some(override_color),
        fill_mode: Some(MarkerFillMode::Filled),
        hidden: false,
    }];
    let scene_ba = compile_graph(&gs_ba, &doc)?;
    let conn_lines = line_segments_with_color(&scene_ba, conn_color);
    assert!(
        conn_lines.iter().any(|(a, b, w)| (b.x - a.x).abs() > 10.0 && (*w - 2.5).abs() < 1e-6),
        "BeforeAfterLines must honor line_color and stroke_width_pt: {conn_lines:?}"
    );
    assert_eq!(scene_ba.plotted_points.len(), 4, "plotted_points must record all 4 observations for hit testing");
    let row2_pt = scene_ba.plotted_points.iter().find(|p| p.source_cell.row == 2).expect("row 2 in plotted_points");
    let hit = scene_ba.hit_test_point(row2_pt.center_pt, 2.0).expect("hit_test_point should find overridden point");
    assert_eq!(hit.source_cell.row, 2);
    let override_spans = path_fill_spans_with_color(&scene_ba, override_color);
    assert!(
        override_spans.iter().any(|&(w, h)| (w - 12.0).abs() < 2.5 && (h - 12.0).abs() < 2.5),
        "per-cell point override on row 2 must apply custom size_pt=12.0 & fill_color: {override_spans:?}"
    );

    // ScatterAndLine custom stroke and marker color honoring
    let sl_line_color = Rgba8::rgb(14, 150, 210);
    let sl_marker_color = Rgba8::rgb(220, 120, 15);
    let mut gs_sl = GraphSheet::new_uninitialized_for_data(SheetId::new("g-sl"), &ds_ba)?;
    gs_sl.initialized = true;
    gs_sl.spec.kind = GraphKind::MultipleVariables;
    gs_sl.spec.multivariate = Some(mv_encoding(
        MultivariatePlotMode::CategoricalSummary {
            gdo: GraphicalDataObject::ScatterAndLine {
                marker: Some(MarkerStyle {
                    shape: MarkerShape::Circle,
                    fill_mode: MarkerFillMode::Filled,
                    size_pt: 6.0,
                    fill_color: sl_marker_color,
                    stroke_color: Rgba8::WHITE,
                    stroke_width_pt: 0.5,
                }),
                line: Some(curvebook_doc::ConnectingLineStyle {
                    kind: LineConnectionKind::PointToPoint,
                    dash_style: LineDashStyle::Solid,
                    stroke_width_pt: 2.2,
                    color: sl_line_color,
                }),
                area_fill: None,
                replicate_lines: curvebook_doc::ReplicateLineMode::default(),
            },
            y_error_bars: None,
        },
        0,
        1,
    ));
    let scene_sl = compile_graph(&gs_sl, &doc)?;
    let sl_lines = line_segments_with_color(&scene_sl, sl_line_color);
    assert!(
        sl_lines.iter().any(|(a, b, w)| (b.x - a.x).abs() > 10.0 && (*w - 2.2).abs() < 1e-6),
        "ScatterAndLine must honor connecting line stroke: {sl_lines:?}"
    );
    assert!(!path_fill_spans_with_color(&scene_sl, sl_marker_color).is_empty(), "ScatterAndLine must honor custom marker fill_color");
    Ok(())
}

// ============================================================================
// FA-SEC09-03: Sub-picomolar log10 and extreme [-1e308, 1e308] XY scenes
// ============================================================================

#[test]
fn sec09_03_xy_chart_subpicomolar_log_and_extreme_linear_domains() -> TestResult {
    // 1. Sub-picomolar log10 XY chart
    let mut xy_sub = XyTable::new(SubcolumnMode::Single, 1, 3)?;
    for (r, (xv, yv)) in [(1e-14, 2e-14), (1e-13, 3e-13), (1e-12, 5e-12)].into_iter().enumerate() {
        xy_sub.x_cells[r][0] = Cell::number(xv);
        xy_sub.data_sets[0].cells[r][0] = Cell::number(yv);
    }
    let ds_sub = DataSheet::new(SheetId::new("xy-sub"), "SubpM", TableArchetype::Xy(xy_sub))?;
    let mut doc = make_doc("ScalesXY")?;
    doc.add_data_sheet(ds_sub.clone())?;

    let mut gs_sub = GraphSheet::new_uninitialized_for_data(SheetId::new("g-sub"), &ds_sub)?;
    gs_sub.initialized = true;
    gs_sub.spec.kind = GraphKind::Xy;
    gs_sub.spec.x_axis = log10_axis("Conc X (M)");
    gs_sub.spec.y_axis = log10_axis("Conc Y (M)");
    let scene_sub = compile_graph(&gs_sub, &doc)?;
    assert!(
        !scene_sub.warnings.iter().any(|w| w.code == "degenerate_domain_fallback"),
        "sub-picomolar log domain triggered bogus fallback: {:?}",
        scene_sub.warnings
    );
    assert_eq!(scene_sub.plotted_points.len(), 3, "expected 3 rendered markers for sub-picomolar log XY");
    assert!(
        (scene_sub.plotted_points[2].center_pt.x - scene_sub.plotted_points[0].center_pt.x).abs() > 20.0,
        "sub-picomolar markers must span plot width: {:?}",
        scene_sub.plotted_points
    );

    // 2. Extreme [-1e308, 1e308] linear XY chart
    let mut xy_ext = XyTable::new(SubcolumnMode::Single, 1, 3)?;
    for (r, (xv, yv)) in [(-1e308, -1e308), (0.0, 0.0), (1e308, 1e308)].into_iter().enumerate() {
        xy_ext.x_cells[r][0] = Cell::number(xv);
        xy_ext.data_sets[0].cells[r][0] = Cell::number(yv);
    }
    let ds_ext = DataSheet::new(SheetId::new("xy-ext"), "Extreme", TableArchetype::Xy(xy_ext))?;
    doc.add_data_sheet(ds_ext.clone())?;

    let mut gs_ext = GraphSheet::new_uninitialized_for_data(SheetId::new("g-ext"), &ds_ext)?;
    gs_ext.initialized = true;
    gs_ext.spec.kind = GraphKind::Xy;
    let scene_ext = compile_graph(&gs_ext, &doc)?;
    let issues = preflight_check_scene(&scene_ext);
    assert!(issues.is_empty(), "extreme range produced preflight issues: {issues:?}");
    assert_eq!(scene_ext.plotted_points.len(), 3, "expected 3 finite markers on [-1e308, 1e308] XY plot");
    assert!(
        (scene_ext.plotted_points[2].center_pt.x - scene_ext.plotted_points[0].center_pt.x).abs() > 50.0,
        "extreme markers collapsed: {:?}",
        scene_ext.plotted_points
    );
    Ok(())
}

// ============================================================================
// FA-SEC09-04: Bland-Altman Xy Y0 vs Y1, MV column lookup, dynamic LoA captions
// ============================================================================

#[test]
fn sec09_04_bland_altman_xy_y0_vs_y1_and_x_vs_y0_modes() -> TestResult {
    // X is ~[1000..4000], Y0=[10..40], Y1=[11,19,32,38] -> (Y0 - Y1) bias = 0.0, avg ~ [10.5..39].
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4)?;
    xy.x_title = String::from("SampleOrder");
    xy.data_sets[0].title = String::from("Y0");
    xy.data_sets[1].title = String::from("Y1");
    for (r, (x, a, b)) in [(1000.0, 10.0, 11.0), (2000.0, 20.0, 19.0), (3000.0, 30.0, 32.0), (4000.0, 40.0, 38.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(a);
        xy.data_sets[1].cells[r][0] = Cell::number(b);
    }
    let ds = DataSheet::new(SheetId::new("ba-xy"), "XY BA", TableArchetype::Xy(xy))?;

    // Mode A: default None with 2 Y cols -> Y0 vs Y1 (bias = 0.0).
    let ba_y01 = BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: None };
    let an_y01 = AnalysisSpec::new(SheetId::new("res-y01"), ds.id.clone(), AnalysisKind::BlandAltman(ba_y01))?;
    let mut res_y01 = run_bland_altman_analysis(&ds, &an_y01, &ba_y01)?;
    res_y01.fitted_curves.clear();
    res_y01.tabs.retain(|t| t.kind != ResultTabKind::Residuals);

    let mut doc = make_doc("BlandAltmanXY")?;
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res_y01.clone())?;
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-y01"), &ds)?;
    gs.initialized = true;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman);
    gs.spec.primary_source_sheet_id = res_y01.id;
    let scene_y01 = compile_graph(&gs, &doc)?;
    let texts_y01 = plain_texts(&scene_y01);
    assert!(texts_y01.iter().any(|t| t == "Bias: 0.00"), "expected Y0 vs Y1 bias 0.00, got {texts_y01:?}");
    let x_ticks_y01 = plain_texts(&scene_y01).into_iter().filter_map(|t| t.parse::<f64>().ok()).fold(f64::NEG_INFINITY, f64::max);
    assert!(x_ticks_y01 < 100.0, "expected averages near 10..40 (Y0 vs Y1), not ~1000..4000 (X vs Y0): max_num={x_ticks_y01}");

    // Mode B: explicit xy_compare_x_vs_y = Some(true) -> X vs Y0 (mean diff ~ +2475.0).
    let ba_xy0 = BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: Some(true) };
    let an_xy0 = AnalysisSpec::new(SheetId::new("res-xy0"), ds.id.clone(), AnalysisKind::BlandAltman(ba_xy0))?;
    let mut res_xy0 = run_bland_altman_analysis(&ds, &an_xy0, &ba_xy0)?;
    res_xy0.fitted_curves.clear();
    res_xy0.tabs.retain(|t| t.kind != ResultTabKind::Residuals);
    doc.add_result_sheet(res_xy0.clone())?;
    let mut gs2 = GraphSheet::new_uninitialized_for_data(SheetId::new("g-xy0"), &ds)?;
    gs2.initialized = true;
    gs2.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman);
    gs2.spec.primary_source_sheet_id = res_xy0.id;
    let scene_xy0 = compile_graph(&gs2, &doc)?;
    let texts_xy0 = plain_texts(&scene_xy0);
    assert!(texts_xy0.iter().any(|t| t == "Bias: 2475.00"), "expected X vs Y0 bias 2475.00, got {texts_xy0:?}");
    Ok(())
}

#[test]
fn sec09_04_bland_altman_mv_ignores_graph_multivariate_and_dynamic_loa_captions() -> TestResult {
    // 1. MultipleVariables: cols (0, 3) have diff +500; cols (1, 2) have diff +2.0.
    // Conflicting `graph.spec.multivariate = Some((0, 3))` must NOT override `BlandAltmanSpec { column_a: 1, column_b: 2 }`.
    let mut mv = MultipleVariablesTable::new(4, 5)?;
    mv.variables[0].title = String::from("TrapA");
    mv.variables[1].title = String::from("TrueA");
    mv.variables[2].title = String::from("TrueB");
    mv.variables[3].title = String::from("TrapB");
    for (r, diff) in [1.5, 2.0, 2.5, 1.8, 2.2].into_iter().enumerate() {
        let base = 10.0 + (r as f64) * 5.0;
        mv.variables[0].cells[r] = Cell::number(base + 500.0);
        mv.variables[1].cells[r] = Cell::number(base + diff);
        mv.variables[2].cells[r] = Cell::number(base);
        mv.variables[3].cells[r] = Cell::number(base);
    }
    let ds = DataSheet::new(SheetId::new("mv-ba"), "MV BA", TableArchetype::MultipleVariables(mv))?;
    let ba_spec = BlandAltmanSpec { column_a: 1, column_b: 2, percent_difference: false, xy_compare_x_vs_y: None };
    let mut an_mv = AnalysisSpec::new(SheetId::new("res-mv-ba"), ds.id.clone(), AnalysisKind::BlandAltman(ba_spec))?;
    an_mv.confidence_level = ConfidenceLevel::new(99.0)?;
    let mut res_mv = run_bland_altman_analysis(&ds, &an_mv, &ba_spec)?;
    res_mv.fitted_curves.clear();
    res_mv.tabs.retain(|t| t.kind != ResultTabKind::Residuals);

    let mut doc = make_doc("MVBA")?;
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res_mv.clone())?;
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-mv-ba"), &ds)?;
    gs.initialized = true;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman);
    gs.spec.primary_source_sheet_id = res_mv.id;
    gs.spec.multivariate = Some(mv_encoding(MultivariatePlotMode::BubbleScatter, 0, 3));
    let scene_mv = compile_graph(&gs, &doc)?;
    let texts_mv = plain_texts(&scene_mv);
    assert!(texts_mv.iter().any(|t| t == "Bias: 2.00"), "BlandAltman MV must honor column_a=1, column_b=2 over multivariate (0,3): {texts_mv:?}");
    assert!(scene_mv.plotted_points.iter().all(|p| p.source_cell.col == 1), "plotted points must map to column_a=1");
    assert!(texts_mv.iter().any(|t| t.starts_with("+2.58 SD:")), "99% confidence should label +2.58 SD: {texts_mv:?}");
    assert!(texts_mv.iter().any(|t| t.starts_with("-2.58 SD:")), "99% confidence should label -2.58 SD: {texts_mv:?}");

    // 2. 90% confidence level -> z ≈ 1.64 SD
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 5)?;
    for (r, (a, b)) in [(10.0, 9.0), (14.0, 12.0), (18.0, 17.5), (22.0, 20.0), (26.0, 25.0)].into_iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(a);
        col.data_sets[1].cells[r][0] = Cell::number(b);
    }
    let ds_col = DataSheet::new(SheetId::new("col-90"), "Col90", TableArchetype::Column(col))?;
    let ba_90 = BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: None };
    let mut an_90 = AnalysisSpec::new(SheetId::new("res-90"), ds_col.id.clone(), AnalysisKind::BlandAltman(ba_90))?;
    an_90.confidence_level = ConfidenceLevel::new(90.0)?;
    let res_90 = run_bland_altman_analysis(&ds_col, &an_90, &ba_90)?;
    doc.add_data_sheet(ds_col.clone())?;
    doc.add_result_sheet(res_90.clone())?;
    let mut gs_90 = GraphSheet::new_uninitialized_for_data(SheetId::new("g-90"), &ds_col)?;
    gs_90.initialized = true;
    gs_90.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman);
    gs_90.spec.primary_source_sheet_id = res_90.id;
    let scene_90 = compile_graph(&gs_90, &doc)?;
    let texts_90 = plain_texts(&scene_90);
    assert!(texts_90.iter().any(|t| t.starts_with("+1.64 SD:")), "90% confidence should label +1.64 SD: {texts_90:?}");
    assert!(texts_90.iter().any(|t| t.starts_with("-1.64 SD:")), "90% confidence should label -1.64 SD: {texts_90:?}");

    // 3. Custom Student-t ResultSheet LoA (k = 2.26 symmetric around bias=1.0 with sd_bias=2.0 -> [-3.52, 5.52])
    let t_spec = AnalysisSpec::new(SheetId::new("res-t"), ds_col.id.clone(), AnalysisKind::BlandAltman(ba_90))?;
    let mut res_t = ResultSheet::new(t_spec)?;
    res_t.scalars = vec![
        ResultScalarEntry { key: String::from("bias"), data_set_index: None, value: 1.0, units: None },
        ResultScalarEntry { key: String::from("sd_bias"), data_set_index: None, value: 2.0, units: None },
        ResultScalarEntry { key: String::from("loa_lower"), data_set_index: None, value: -3.52, units: None },
        ResultScalarEntry { key: String::from("loa_upper"), data_set_index: None, value: 5.52, units: None },
    ];
    let coord_rows = vec![
        vec![ResultCell::Number { value: 10.0, prefix: None, units: None }, ResultCell::Number { value: -1.0, prefix: None, units: None }],
        vec![ResultCell::Number { value: 15.0, prefix: None, units: None }, ResultCell::Number { value: 1.0, prefix: None, units: None }],
        vec![ResultCell::Number { value: 20.0, prefix: None, units: None }, ResultCell::Number { value: 3.0, prefix: None, units: None }],
    ];
    res_t.tabs.push(ResultTab {
        id: String::from("bland-altman-coords"),
        title: String::from("Coords"),
        kind: ResultTabKind::Residuals,
        column_headers: vec![String::from("Average"), String::from("Difference")],
        row_labels: vec![String::from("1"), String::from("2"), String::from("3")],
        rows: coord_rows.clone(),
        footnotes: Vec::new(),
    });
    doc.add_result_sheet(res_t.clone())?;
    let mut gs_t = GraphSheet::new_uninitialized_for_data(SheetId::new("g-t"), &ds_col)?;
    gs_t.initialized = true;
    gs_t.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman);
    gs_t.spec.primary_source_sheet_id = res_t.id;
    let scene_t = compile_graph(&gs_t, &doc)?;
    let texts_t = plain_texts(&scene_t);
    assert!(texts_t.iter().any(|t| t == "+2.26 SD: 5.52"), "Student-t LoA should show +2.26 SD: 5.52: {texts_t:?}");
    assert!(texts_t.iter().any(|t| t == "-2.26 SD: -3.52"), "Student-t LoA should show -2.26 SD: -3.52: {texts_t:?}");

    // 4. Nonparametric / asymmetric LoA -> Upper LoA / Lower LoA (never ±1.96 SD)
    let np_spec = AnalysisSpec::new(SheetId::new("res-np"), ds_col.id.clone(), AnalysisKind::BlandAltman(ba_90))?;
    let mut res_np = ResultSheet::new(np_spec)?;
    res_np.scalars = vec![
        ResultScalarEntry { key: String::from("bias"), data_set_index: None, value: 1.0, units: None },
        ResultScalarEntry { key: String::from("sd_bias"), data_set_index: None, value: 2.0, units: None },
        ResultScalarEntry { key: String::from("loa_lower"), data_set_index: None, value: -1.8, units: None },
        ResultScalarEntry { key: String::from("loa_upper"), data_set_index: None, value: 5.4, units: None },
    ];
    res_np.tabs.push(ResultTab {
        id: String::from("bland-altman-coords"),
        title: String::from("Quantile Bland-Altman"),
        kind: ResultTabKind::Residuals,
        column_headers: vec![String::from("Average"), String::from("Difference")],
        row_labels: vec![String::from("1"), String::from("2"), String::from("3")],
        rows: coord_rows,
        footnotes: Vec::new(),
    });
    doc.add_result_sheet(res_np.clone())?;
    let mut gs_np = GraphSheet::new_uninitialized_for_data(SheetId::new("g-np"), &ds_col)?;
    gs_np.initialized = true;
    gs_np.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman);
    gs_np.spec.primary_source_sheet_id = res_np.id;
    let scene_np = compile_graph(&gs_np, &doc)?;
    let texts_np = plain_texts(&scene_np);
    assert!(texts_np.iter().any(|t| t == "Upper LoA: 5.40"), "nonparametric LoA should label Upper LoA: {texts_np:?}");
    assert!(texts_np.iter().any(|t| t == "Lower LoA: -1.80"), "nonparametric LoA should label Lower LoA: {texts_np:?}");
    assert!(!texts_np.iter().any(|t| t.contains("1.96 SD")), "must never label 1.96 SD on quantile LoA: {texts_np:?}");
    Ok(())
}

// ============================================================================
// FA-SEC09-05: Pre-summarized Grouped Heatmap, Normal Q-Q, and Residual Plot
// ============================================================================

#[test]
fn sec09_05_pre_summarized_grouped_heatmap_normal_qq_and_residual_extract_subcol_zero_only() -> TestResult {
    let grouped_modes = [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MedianIqr, SubcolumnMode::UpperLowerLimits];

    for (idx, mode) in grouped_modes.into_iter().enumerate() {
        let mut gt = GroupedTable::new(mode, 2, 4)?;
        let expected_means = [10.0, 20.0, 30.0, 40.0];
        let trap_subs: &[f64] = match mode {
            SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN => &[9000.0, 3.0],
            SubcolumnMode::MedianIqr | SubcolumnMode::UpperLowerLimits => &[9000.0, 1.0],
            _ => &[],
        };
        for r in 0..4 {
            for c in 0..2 {
                gt.data_sets[c].cells[r][0] = Cell::number(expected_means[r]);
                for (s_off, &trap) in trap_subs.iter().enumerate() {
                    if let Some(cell) = gt.data_sets[c].cells[r].get_mut(1 + s_off) {
                        *cell = Cell::number(trap);
                    }
                }
            }
        }
        let ds_g = DataSheet::new(SheetId::new(format!("g-pre-{idx}")), "GroupedPre", TableArchetype::Grouped(gt))?;
        let mut doc = make_doc(&format!("PreSymG-{idx}"))?;
        doc.add_data_sheet(ds_g.clone())?;

        // 1. Grouped Heatmap colorbar bounds stay in [10, 40] (never polluted by 9000 or 3).
        let mut gs_hm = GraphSheet::new_uninitialized_for_data(SheetId::new("g-hm"), &ds_g)?;
        gs_hm.initialized = true;
        gs_hm.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
        let scene_hm = compile_graph(&gs_hm, &doc)?;
        let cb_nums: Vec<f64> = plain_texts(&scene_hm).into_iter().filter_map(|t| t.parse::<f64>().ok()).collect();
        let hm_max = cb_nums.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!((hm_max - 40.0).abs() < 1e-6, "{mode:?} Heatmap max must equal 40.0 (subcol 0), got {cb_nums:?}");

        // 2. Grouped Normal Q-Q raw fallback plots ONLY 4 points (subcol 0) + emits warning.
        let mut gs_qq = GraphSheet::new_uninitialized_for_data(SheetId::new("g-qq"), &ds_g)?;
        gs_qq.initialized = true;
        gs_qq.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
        let scene_qq = compile_graph(&gs_qq, &doc)?;
        assert_eq!(scene_qq.plotted_points.len(), 4, "{mode:?} Grouped Q-Q must plot only 4 subcol-0 summaries");
        assert!(scene_qq.plotted_points.iter().all(|p| p.source_cell.subcol == 0));
        assert!(
            scene_qq.warnings.iter().any(|w| w.code == "pre_summarized_summary_only"),
            "{mode:?} Grouped Q-Q should warn about pre_summarized_summary_only: {:?}",
            scene_qq.warnings
        );

        // 3. Grouped Residual Plot raw fallback plots ONLY 4 points (residuals in [-15, 15], never 9000) + emits warning.
        let mut gs_res = GraphSheet::new_uninitialized_for_data(SheetId::new("g-res"), &ds_g)?;
        gs_res.initialized = true;
        gs_res.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot);
        let scene_res = compile_graph(&gs_res, &doc)?;
        assert_eq!(scene_res.plotted_points.len(), 4, "{mode:?} Grouped Residual must plot only 4 subcol-0 summaries");
        assert!(scene_res.plotted_points.iter().all(|p| p.source_cell.subcol == 0));
        assert!(
            scene_res.warnings.iter().any(|w| w.code == "pre_summarized_summary_only"),
            "{mode:?} Grouped Residual should warn about pre_summarized_summary_only"
        );
        let res_max_tick = extract_y_tick_numbers(&scene_res).into_iter().fold(f64::NEG_INFINITY, f64::max);
        assert!(res_max_tick < 100.0, "{mode:?} Grouped Residual y-axis corrupted by subcol traps: max={res_max_tick}");
    }

    // ColumnTable pre-summarized modes (MeanSdN, MeanSemN) in Q-Q & Residual plots (including N=2 insufficient Q-Q!).
    for (idx, mode) in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN].into_iter().enumerate() {
        let mut col4 = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 1, 4)?;
        for (r, &m) in [10.0, 20.0, 30.0, 40.0].iter().enumerate() {
            col4.data_sets[0].cells[r][0] = Cell::number(m);
            col4.data_sets[0].cells[r][1] = Cell::number(9000.0);
            col4.data_sets[0].cells[r][2] = Cell::number(5.0);
        }
        let ds_c4 = DataSheet::new(SheetId::new(format!("c4-{idx}")), "ColPre4", TableArchetype::Column(col4))?;
        let mut doc = make_doc(&format!("PreCol-{idx}"))?;
        doc.add_data_sheet(ds_c4.clone())?;

        let mut gs_qq = GraphSheet::new_uninitialized_for_data(SheetId::new("g-c-qq"), &ds_c4)?;
        gs_qq.initialized = true;
        gs_qq.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
        let scene_qq = compile_graph(&gs_qq, &doc)?;
        assert_eq!(scene_qq.plotted_points.len(), 4, "{mode:?} Column Q-Q must plot only 4 subcol-0 values");
        assert!(scene_qq.warnings.iter().any(|w| w.code == "pre_summarized_summary_only"));

        let mut gs_res = GraphSheet::new_uninitialized_for_data(SheetId::new("g-c-res"), &ds_c4)?;
        gs_res.initialized = true;
        gs_res.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot);
        let scene_res = compile_graph(&gs_res, &doc)?;
        assert_eq!(scene_res.plotted_points.len(), 4, "{mode:?} Column Residual must plot only 4 subcol-0 values");
        assert!(scene_res.warnings.iter().any(|w| w.code == "pre_summarized_summary_only"));

        // 2 rows x 3 subcols = 2 real observations (< 3 required for Q-Q), previously spoofed 6 pseudo-observations!
        let mut col2 = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 1, 2)?;
        for (r, &m) in [10.0, 20.0].iter().enumerate() {
            col2.data_sets[0].cells[r][0] = Cell::number(m);
            col2.data_sets[0].cells[r][1] = Cell::number(9000.0);
            col2.data_sets[0].cells[r][2] = Cell::number(5.0);
        }
        let ds_c2 = DataSheet::new(SheetId::new(format!("c2-{idx}")), "ColPre2", TableArchetype::Column(col2))?;
        doc.add_data_sheet(ds_c2.clone())?;
        let mut gs_qq2 = GraphSheet::new_uninitialized_for_data(SheetId::new("g-c2-qq"), &ds_c2)?;
        gs_qq2.initialized = true;
        gs_qq2.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
        let scene_qq2 = compile_graph(&gs_qq2, &doc)?;
        assert!(scene_qq2.plotted_points.is_empty(), "2 pre-summarized rows must NOT render fake 6-point Q-Q");
        assert!(scene_qq2.warnings.iter().any(|w| w.code == "insufficient_qq_sample"));
    }

    Ok(())
}
