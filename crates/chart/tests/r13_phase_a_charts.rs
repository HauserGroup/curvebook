#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ChartScene, PathCmd, PointPt, ScenePrimitive, compile_graph, compile_layout};
use curvebook_doc::{
    AdditionalTick, AnalysisKind, AnalysisSpec, AxisThreshold, CartesianOrientation, Cell, ColumnDesignHint, ColumnTable, ConnectingLineStyle,
    DataSheet, DiagnosticPlotKind, EmbeddedTableStyle, ErrorBarMetric, GraphAnnotation, GraphKind, GraphSheet, GraphicalDataObject,
    GroupedArrangement, GroupedTable, HeatmapColumnLabelAngle, HeatmapColumnLabelMode, HeatmapLabelSpec, HeatmapRowLabelMode, LayoutItem,
    LayoutSheet, LayoutSpec, LineConnectionKind, LinearRegressionSpec, MultipleVariablesTable, OneWayAnovaSpec, ProjectDocument, RectPt as DocRectPt,
    RegressionOverlaySpec, ReplicateLineMode, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SeriesOffsetSpec, SheetId,
    SubcolumnMode, SummaryMeasure, SurvivalDisplayMetric, SurvivalTable, TableArchetype, TickDirection, TickLabelFormat, VariableColumn,
    VariableRole, XyTable,
};
use curvebook_fit::{HookContext, execute_analysis};
use curvebook_math::AxisScale;
use curvebook_palette::{LineDashStyle, Rgba8};

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

fn path_segments(path: &curvebook_chart::Path2D) -> Vec<(PointPt, PointPt)> {
    let mut out = Vec::new();
    let mut cur = None;
    for &cmd in &path.commands {
        match cmd {
            PathCmd::MoveTo(p) => cur = Some(p),
            PathCmd::LineTo(p) => {
                if let Some(prev) = cur {
                    out.push((prev, p));
                }
                cur = Some(p);
            }
            PathCmd::QuadTo { to, .. } | PathCmd::CubicTo { to, .. } => cur = Some(to),
            PathCmd::Close => {}
        }
    }
    out
}

#[test]
fn km_survival_metrics_point_to_point_sem_and_hundred_percent_censored() {
    let mut tbl = SurvivalTable::new(2, 6).expect("survival table");
    tbl.cohorts[0].title = String::from("Treated");
    tbl.cohorts[1].title = String::from("CensoredArm");
    for r in 0..6 {
        tbl.times[r] = Cell::number(((r + 1) * 4) as f64);
        // Cohort 0 has events at t = 4, 12, 20; Cohort 1 has zero events (100% censored).
        tbl.cohorts[0].cells[r][0] = Cell::number(if matches!(r, 0 | 2 | 4) { 1.0 } else { 0.0 });
        tbl.cohorts[1].cells[r][0] = Cell::number(0.0);
    }
    let ds = DataSheet::new(SheetId::new("surv-1"), "KM Cohorts", TableArchetype::Survival(tbl)).expect("data sheet");
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone()).expect("add data");

    // 1. PercentCumulativeEvent + PointToPoint + Greenwood SEM error bars + Number at Risk
    let mut gs_ev = GraphSheet::new_uninitialized_for_data(SheetId::new("g-ev"), &ds).expect("graph");
    gs_ev.initialized = true;
    gs_ev.spec.survival_number_at_risk = true;
    for s in &mut gs_ev.spec.series {
        if let GraphicalDataObject::KaplanMeierStep { display_metric, staircase, error_display, show_greenwood_ci, .. } = &mut s.gdo {
            *display_metric = SurvivalDisplayMetric::PercentCumulativeEvent;
            *staircase = false;
            *error_display = Some(ErrorBarMetric::Sem);
            *show_greenwood_ci = false;
        }
    }
    let scene_ev = compile_graph(&gs_ev, &doc).expect("compile event point-to-point KM");
    assert!(all_points_finite(&scene_ev));

    // PointToPoint emits sloped (diagonal) line segments where both X and Y change along the Treated curve.
    let treated_color = match gs_ev.spec.series[0].gdo {
        GraphicalDataObject::KaplanMeierStep { line_color, .. } => Some(line_color),
        _ => None,
    }
    .expect("KM step series");
    let has_sloped_segment = scene_ev.primitives.iter().any(|prim| match prim {
        ScenePrimitive::Path { path, stroke: Some(st), clip: Some(_), .. } if st.color == treated_color => {
            path_segments(path).into_iter().any(|(a, b)| (a.x - b.x).abs() > 2.0 && (a.y - b.y).abs() > 2.0)
        }
        _ => false,
    });
    assert!(has_sloped_segment, "staircase: false should produce diagonal PointToPoint curve segments");

    // PercentCumulativeEvent places Treated's later endpoint ABOVE (smaller screen Y than) its start!
    let treated_curve_pts: Vec<(f64, f64)> = scene_ev
        .primitives
        .iter()
        .filter_map(|prim| match prim {
            ScenePrimitive::Path { path, stroke: Some(st), clip: Some(_), .. } if st.color == treated_color && path.commands.len() >= 3 => {
                Some(path_segments(path).into_iter().flat_map(|(a, b)| vec![(a.x, a.y), (b.x, b.y)]).collect::<Vec<_>>())
            }
            _ => None,
        })
        .flatten()
        .collect();
    let min_x_y = treated_curve_pts.iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap().1;
    let max_x_y = treated_curve_pts.iter().max_by(|a, b| a.0.total_cmp(&b.0)).unwrap().1;
    assert!(max_x_y < min_x_y - 5.0, "PercentCumulativeEvent curve must rise from lower Y to higher percentage ({max_x_y} < {min_x_y})");

    // 100%-censored arm renders flat line across the chart without events and appears in Number at Risk.
    let texts_ev = plain_texts(&scene_ev);
    assert!(texts_ev.iter().any(|t| t.contains("CensoredArm")), "number-at-risk table should list CensoredArm: {texts_ev:?}");

    // 2. FractionSurvival renders [0..1] Y axis ticks and descending curve + ConfidenceInterval95 error display
    let mut gs_frac = gs_ev.clone();
    for s in &mut gs_frac.spec.series {
        if let GraphicalDataObject::KaplanMeierStep { display_metric, staircase, error_display, .. } = &mut s.gdo {
            *display_metric = SurvivalDisplayMetric::FractionSurvival;
            *staircase = true;
            *error_display = Some(ErrorBarMetric::ConfidenceInterval95);
        }
    }
    let scene_frac = compile_graph(&gs_frac, &doc).expect("compile fraction survival");
    assert!(all_points_finite(&scene_frac));
    let frac_texts = plain_texts(&scene_frac);
    assert!(frac_texts.iter().any(|t| t == "0.5" || t == "1" || t == "0.2" || t == "0.8"), "FractionSurvival Y ticks expected: {frac_texts:?}");
    assert!(!frac_texts.iter().any(|t| t == "100" || t == "80"), "FractionSurvival should not show 0..100 ticks: {frac_texts:?}");
}

#[test]
fn forest_plot_log_scale_coordinate_oracle_warnings_and_ref_dedup() {
    // 1. Grouped table in UpperLowerLimits subcolumn mode ([Estimate, Lower, Upper])
    let mut g = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 1, 3).expect("grouped table");
    g.row_titles = vec![String::from("Trial Alpha"), String::from("Trial Beta"), String::from("Broken Interval")];
    g.data_sets[0].title = String::from("Odds Ratio");
    // Trial Alpha: est = 2.0, lo = 1.0, hi = 3.5 -> valid
    g.data_sets[0].cells[0] = vec![Cell::number(2.0), Cell::number(1.0), Cell::number(3.5)];
    // Trial Beta: est = 0.5, lo = 0.25, hi = 1.0 -> valid
    g.data_sets[0].cells[1] = vec![Cell::number(0.5), Cell::number(0.25), Cell::number(1.0)];
    // Broken Interval: lower > est -> emits "invalid_forest_interval"
    g.data_sets[0].cells[2] = vec![Cell::number(1.2), Cell::number(2.5), Cell::number(3.0)];

    let ds_grp = DataSheet::new(SheetId::new("f-grp"), "Forest Studies", TableArchetype::Grouped(g)).expect("ds");
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-forest"), &ds_grp).expect("gs");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
    assert!(!gs.spec.forest_show_pooled_diamond, "user Grouped forest plot must default forest_show_pooled_diamond = false");
    gs.spec.x_axis.segments[0].scale = AxisScale::Log10;
    gs.spec.x_axis.segments[0].min = Some(0.25);
    gs.spec.x_axis.segments[0].max = Some(4.0);
    // Styled user threshold at 1.0 must deduplicate hardcoded default reference line at x = 1.0.
    gs.spec.x_axis.thresholds = vec![AxisThreshold {
        value: 1.0,
        shade_to: None,
        color: Rgba8::rgb(180, 40, 40),
        dash_style: LineDashStyle::Dotted,
        stroke_width_pt: 1.2,
        label: Some(String::from("Null OR=1")),
    }];

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds_grp).expect("add data");
    let scene = compile_graph(&gs, &doc).expect("compile forest");
    assert!(all_points_finite(&scene));

    // Invalid interval warning triggered
    assert!(scene.warnings.iter().any(|w| w.code == "invalid_forest_interval"), "expected invalid_forest_interval warning, got {:?}", scene.warnings);

    // Pooled diamond suppressed when forest_show_pooled_diamond = false
    let texts = plain_texts(&scene);
    assert!(!texts.iter().any(|t| t.contains("Pooled")), "pooled diamond label must be omitted when flag is false");
    assert!(texts.iter().any(|t| t == "Trial Alpha") && texts.iter().any(|t| t == "Trial Beta"));

    // Log-scale D1 coordinate oracle (<= 0.5 pt): Trial Alpha OR = 2.0 on [0.25, 4.0]
    let frac_alpha = (2.0_f64.log10() - 0.25_f64.log10()) / (4.0_f64.log10() - 0.25_f64.log10());
    let expected_x_alpha = scene.plot_rect_pt.x + frac_alpha * scene.plot_rect_pt.width;
    let y_alpha = scene
        .primitives
        .iter()
        .find_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text() == "Trial Alpha" => Some(t.anchor_pt.y),
            _ => None,
        })
        .expect("Trial Alpha row label");
    let alpha_marker_x = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } => {
                let mut min_x = f64::INFINITY;
                let mut max_x = f64::NEG_INFINITY;
                let mut min_y = f64::INFINITY;
                let mut max_y = f64::NEG_INFINITY;
                for cmd in &path.commands {
                    if let PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) = *cmd {
                        min_x = min_x.min(pt.x);
                        max_x = max_x.max(pt.x);
                        min_y = min_y.min(pt.y);
                        max_y = max_y.max(pt.y);
                    }
                }
                let cy = 0.5 * (min_y + max_y);
                ((cy - y_alpha).abs() < 8.0 && (max_x - min_x) < 16.0).then_some(0.5 * (min_x + max_x))
            }
            _ => None,
        })
        .next()
        .expect("Trial Alpha filled square marker");
    assert!(
        (alpha_marker_x - expected_x_alpha).abs() <= 0.5,
        "forest log-scale X oracle mismatch: actual={alpha_marker_x} expected={expected_x_alpha}"
    );

    // Ref line deduplication: count vertical full-height lines near x = 1.0 pixel coordinate
    let frac_one = (1.0_f64.log10() - 0.25_f64.log10()) / (4.0_f64.log10() - 0.25_f64.log10());
    let x_one_pt = scene.plot_rect_pt.x + frac_one * scene.plot_rect_pt.width;
    let ref_lines_at_one = scene
        .primitives
        .iter()
        .filter(|prim| match prim {
            ScenePrimitive::Path { path, stroke: Some(_), clip: Some(_), .. } => path_segments(path)
                .into_iter()
                .any(|(a, b)| (a.x - x_one_pt).abs() < 0.75 && (b.x - x_one_pt).abs() < 0.75 && (a.y - b.y).abs() > scene.plot_rect_pt.height * 0.8),
            _ => false,
        })
        .count();
    assert_eq!(ref_lines_at_one, 1, "must not draw duplicate vertical ref lines at x=1.0 (count={ref_lines_at_one})");

    // 2. Column table (3 columns: [Est, Lower, Upper]) + Grouped (MeanLowHigh) + pooled diamond enabled
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2).expect("col forest");
    col.row_titles = vec![String::from("Cohort X"), String::from("Cohort Y")];
    col.data_sets[0].cells = vec![vec![Cell::number(1.5)], vec![Cell::number(2.5)]];
    col.data_sets[1].cells = vec![vec![Cell::number(1.1)], vec![Cell::number(1.8)]];
    col.data_sets[2].cells = vec![vec![Cell::number(2.1)], vec![Cell::number(3.4)]];
    let ds_col = DataSheet::new(SheetId::new("f-col"), "Col Forest", TableArchetype::Column(col)).expect("ds");
    let mut gs_col = GraphSheet::new_uninitialized_for_data(SheetId::new("g-fcol"), &ds_col).expect("gs");
    gs_col.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
    gs_col.spec.forest_show_pooled_diamond = true;
    doc.add_data_sheet(ds_col).expect("add col");
    let scene_col = compile_graph(&gs_col, &doc).expect("compile col forest");
    assert!(all_points_finite(&scene_col));
    assert!(plain_texts(&scene_col).iter().any(|t| t.contains("Pooled")), "explicit forest_show_pooled_diamond=true should render Pooled");

    // 3. Grouped MeanLowHigh ([Mean, -Err, +Err])
    let mut g_mlh = GroupedTable::new(SubcolumnMode::MeanLowHigh, 1, 2).expect("mlh");
    g_mlh.row_titles = vec![String::from("Arm 1"), String::from("Arm 2")];
    g_mlh.data_sets[0].cells =
        vec![vec![Cell::number(2.0), Cell::number(0.5), Cell::number(1.0)], vec![Cell::number(3.0), Cell::number(0.6), Cell::number(1.2)]];
    let ds_mlh = DataSheet::new(SheetId::new("f-mlh"), "MLH Forest", TableArchetype::Grouped(g_mlh)).expect("ds mlh");
    let gs_mlh = {
        let mut g = GraphSheet::new_uninitialized_for_data(SheetId::new("g-mlh"), &ds_mlh).expect("gs");
        g.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
        g
    };
    doc.add_data_sheet(ds_mlh).expect("add mlh");
    let scene_mlh = compile_graph(&gs_mlh, &doc).expect("compile mlh forest");
    assert!(scene_mlh.warnings.is_empty(), "valid MeanLowHigh intervals must produce zero warnings: {:?}", scene_mlh.warnings);
    assert!(plain_texts(&scene_mlh).iter().any(|t| t == "Arm 1") && plain_texts(&scene_mlh).iter().any(|t| t == "Arm 2"));
}

#[test]
fn horizontal_population_pyramid_stacked_abs_ticks_and_labels() {
    let bands = [
        "0-4",
        "5-9",
        "10-14",
        "15-19",
        "20-24",
        "25-29",
        "30-34",
        "35-39",
        "40-44",
        "45-49",
        "50-54",
        "55-59",
        "60-64",
        "65-69",
        "70-74",
        "75-79",
        "80-84",
        "85+ (Elderly)",
    ];
    let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, bands.len()).expect("pyramid table");
    g.row_titles = bands.iter().map(|s| (*s).to_owned()).collect();
    g.data_sets[0].title = String::from("Group Left");
    g.data_sets[1].title = String::from("Group Right");
    for r in 0..bands.len() {
        let mag = 12.0 - (r as f64) * 0.55;
        g.data_sets[0].cells[r][0] = Cell::number(-mag);
        g.data_sets[1].cells[r][0] = Cell::number(mag - 0.3);
    }

    let ds = DataSheet::new(SheetId::new("pyr-1"), "Age Distribution", TableArchetype::Grouped(g)).expect("ds");
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-pyr"), &ds).expect("gs");
    gs.initialized = true;
    gs.spec.orientation = CartesianOrientation::Horizontal;
    gs.spec.grouped_arrangement = GroupedArrangement::Stacked;
    gs.spec.x_axis.tick_format = TickLabelFormat::AbsoluteDecimal;
    gs.spec.x_axis.additional_ticks = vec![AdditionalTick { value: 0.0, label: Some(String::from("Mid")), draw_gridline: true }];
    gs.spec.y_axis.title = String::from("Age Cohort");

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds).expect("add data");
    let scene = compile_graph(&gs, &doc).expect("compile pyramid");
    assert!(all_points_finite(&scene));

    // Filled bar rects should exist on both sides of X = 0.0
    let left_color = match gs.spec.series[0].gdo {
        GraphicalDataObject::BarWithReplicates { fill_color, .. } => Some(fill_color),
        _ => None,
    }
    .expect("left bar series");
    let right_color = match gs.spec.series[1].gdo {
        GraphicalDataObject::BarWithReplicates { fill_color, .. } => Some(fill_color),
        _ => None,
    }
    .expect("right bar series");
    let left_bars = scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(f), .. } if f.color == left_color)).count();
    let right_bars = scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(f), .. } if f.color == right_color)).count();
    assert_eq!(left_bars, bands.len() + 1, "18 left bars + 1 legend swatch expected");
    assert_eq!(right_bars, bands.len() + 1, "18 right bars + 1 legend swatch expected");

    // Zero baseline splits negative vs positive bars cleanly: find bar X bounds in plot area
    let mut neg_min_x = f64::INFINITY;
    let mut neg_max_x = f64::NEG_INFINITY;
    let mut pos_min_x = f64::INFINITY;
    let mut pos_max_x = f64::NEG_INFINITY;
    for prim in &scene.primitives {
        if let ScenePrimitive::Path { path, fill: Some(f), clip: Some(_), .. } = prim
            && (f.color == left_color || f.color == right_color)
        {
            for cmd in &path.commands {
                if let PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) = *cmd {
                    if f.color == left_color {
                        neg_min_x = neg_min_x.min(pt.x);
                        neg_max_x = neg_max_x.max(pt.x);
                    } else {
                        pos_min_x = pos_min_x.min(pt.x);
                        pos_max_x = pos_max_x.max(pt.x);
                    }
                }
            }
        }
    }
    assert!((neg_max_x - pos_min_x).abs() <= 1.0, "negative and positive pyramid bars must meet at X=0 spine: {neg_max_x} vs {pos_min_x}");
    assert!(neg_min_x < neg_max_x - 20.0 && pos_max_x > pos_min_x + 20.0);

    // AbsoluteDecimal X ticks contain no minus signs
    let x_tick_texts: Vec<String> = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) if t.anchor_pt.y > scene.plot_rect_pt.bottom() && t.anchor_pt.y < scene.height_pt - 15.0 => {
                Some(t.rich.plain_text())
            }
            _ => None,
        })
        .collect();
    assert!(!x_tick_texts.is_empty());
    assert!(x_tick_texts.iter().all(|s| !s.starts_with('-')), "AbsoluteDecimal ticks must not have leading minus: {x_tick_texts:?}");
    assert!(plain_texts(&scene).iter().any(|s| s == "Mid"), "AdditionalTick label expected");

    // Wide Y label fits cleanly on canvas & bottom band "0-4" is below top band "85+ (Elderly)"
    let t_0_4 = scene
        .primitives
        .iter()
        .find_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text() == "0-4" => Some(t),
            _ => None,
        })
        .expect("0-4 label");
    let t_85 = scene
        .primitives
        .iter()
        .find_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text() == "85+ (Elderly)" => Some(t),
            _ => None,
        })
        .expect("85+ (Elderly) label");
    assert!(t_85.anchor_pt.x > 0.0 && t_85.anchor_pt.x <= scene.plot_rect_pt.x, "wide Y label clipped or misplaced: {:?}", t_85.anchor_pt);
    assert!(t_0_4.anchor_pt.y > t_85.anchor_pt.y, "Row 0 (0-4) should appear near bottom and 85+ near top");
}

#[test]
fn xy_offsets_bars_hide_raw_and_spaghetti_trajectories() {
    // 1. Offset + BarWithReplicates + show_raw_data: false keeping regression overlay
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3).expect("xy");
    xy.data_sets[0].title = String::from("Signal");
    for (r, (x, y)) in [(1.0, 10.0), (2.0, 20.0), (3.0, 30.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds_xy = DataSheet::new(SheetId::new("xy-1"), "XY Base", TableArchetype::Xy(xy)).expect("ds");
    let fit_spec = AnalysisSpec::new("res-lin", "xy-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).expect("spec");
    let fit_res = execute_analysis(&fit_spec, &ds_xy, &HookContext::default()).expect("fit");

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds_xy.clone()).expect("add ds");
    doc.add_result_sheet(fit_res).expect("add fit");

    let mut gs_base = GraphSheet::new_uninitialized_for_data(SheetId::new("g-base"), &ds_xy).expect("gs");
    gs_base.initialized = true;
    gs_base.spec.x_axis.segments[0].min = Some(0.0);
    gs_base.spec.x_axis.segments[0].max = Some(5.0);
    gs_base.spec.y_axis.segments[0].min = Some(0.0);
    gs_base.spec.y_axis.segments[0].max = Some(40.0);
    let scene_base = compile_graph(&gs_base, &doc).expect("compile base");

    let mut gs_off = gs_base.clone();
    gs_off.spec.series[0].offset = SeriesOffsetSpec { x_data: 0.5, x_pt: 12.0, y_pt: -7.0 };
    let scene_off = compile_graph(&gs_off, &doc).expect("compile offset");
    assert_eq!(scene_base.plotted_points.len(), 3);
    assert_eq!(scene_off.plotted_points.len(), 3);
    let dx_expected_data = (0.5 / 5.0) * scene_base.plot_rect_pt.width;
    let dx_total = scene_off.plotted_points[0].center_pt.x - scene_base.plotted_points[0].center_pt.x;
    let dy_total = scene_off.plotted_points[0].center_pt.y - scene_base.plotted_points[0].center_pt.y;
    assert!((dx_total - (dx_expected_data + 12.0)).abs() < 1.0, "expected x shift ~{}, got {dx_total}", dx_expected_data + 12.0);
    // y_pt > 0 shifts upward in plot coords, so y_pt = -7.0 shifts downward (+7.0 screen pt).
    assert!((dy_total - 7.0).abs() < 1e-6, "expected screen y shift +7.0 for y_pt=-7.0, got {dy_total}");

    // BarWithReplicates with explicit bar_width_override_pt: Some(18.0)
    let mut gs_bar = gs_base.clone();
    gs_bar.spec.series[0].bar_width_override_pt = Some(18.0);
    gs_bar.spec.series[0].gdo = GraphicalDataObject::BarWithReplicates {
        summary: SummaryMeasure::Mean,
        fill_color: Rgba8::rgb(40, 120, 200),
        border_color: Rgba8::BLACK,
        border_width_pt: 1.0,
        replicate_overlay: None,
    };
    let scene_bar = compile_graph(&gs_bar, &doc).expect("compile xy bar");
    let mut found_18pt_bar = 0_usize;
    for prim in &scene_bar.primitives {
        if let ScenePrimitive::Path { path, fill: Some(f), clip: Some(_), .. } = prim
            && f.color == Rgba8::rgb(40, 120, 200)
        {
            let xs: Vec<f64> = path
                .commands
                .iter()
                .filter_map(|c| match *c {
                    PathCmd::MoveTo(p) | PathCmd::LineTo(p) => Some(p.x),
                    _ => None,
                })
                .collect();
            let w = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) - xs.iter().copied().fold(f64::INFINITY, f64::min);
            if (w - 18.0).abs() < 1e-6 {
                found_18pt_bar += 1;
            }
        }
    }
    assert_eq!(found_18pt_bar, 3, "expected 3 XY bars of exact width 18.0 pt");

    // show_raw_data: false hides raw points/lines while preserving regression_overlay
    let mut gs_fit_only = gs_base.clone();
    gs_fit_only.spec.series[0].show_raw_data = false;
    gs_fit_only.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-lin"),
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    let scene_fit_only = compile_graph(&gs_fit_only, &doc).expect("compile fit-only");
    assert!(scene_fit_only.plotted_points.is_empty(), "raw points must be hidden when show_raw_data=false");
    let has_fit_polyline = scene_fit_only
        .primitives
        .iter()
        .any(|p| matches!(p, ScenePrimitive::Path { path, stroke: Some(_), clip: Some(_), .. } if path.commands.len() >= 10));
    assert!(has_fit_polyline, "regression overlay must remain visible when show_raw_data=false");

    // 2. Spaghetti IndividualReplicateTrajectories with marker: None and blank middle subcell break
    let mut sp2 = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 5).expect("sp2");
    for (r, x) in [1.0, 2.0, 3.0, 4.0, 5.0].into_iter().enumerate() {
        sp2.x_cells[r][0] = Cell::number(x);
    }
    // Subcol 0 has all 5 values -> 1 contiguous 5-vertex trajectory;
    // Subcol 1 has a gap at row 2 -> 2 disjoint 2-vertex trajectories ([0..=1] and [3..=4]).
    sp2.data_sets[0].cells = vec![
        vec![Cell::number(10.0), Cell::number(20.0)],
        vec![Cell::number(15.0), Cell::number(22.0)],
        vec![Cell::number(18.0), Cell::empty()],
        vec![Cell::number(22.0), Cell::number(28.0)],
        vec![Cell::number(26.0), Cell::number(31.0)],
    ];
    let ds_sp = DataSheet::new(SheetId::new("xy-sp"), "Spaghetti", TableArchetype::Xy(sp2)).expect("ds sp");
    let mut doc_sp = ProjectDocument::default();
    doc_sp.add_data_sheet(ds_sp.clone()).expect("add");
    let mut gs_sp = GraphSheet::new_uninitialized_for_data(SheetId::new("g-sp"), &ds_sp).expect("gs sp");
    gs_sp.initialized = true;
    gs_sp.spec.legend.visible = false;
    gs_sp.spec.series[0].y_error_bars = None;
    gs_sp.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
        marker: None,
        line: Some(ConnectingLineStyle {
            kind: LineConnectionKind::PointToPoint,
            dash_style: LineDashStyle::Solid,
            stroke_width_pt: 1.5,
            color: Rgba8::rgb(0, 114, 178),
        }),
        area_fill: None,
        replicate_lines: ReplicateLineMode::IndividualReplicateTrajectories,
    };
    let scene_sp = compile_graph(&gs_sp, &doc_sp).expect("compile spaghetti");
    assert_eq!(scene_sp.plotted_points.len(), 9, "9 active replicate vertices must emit PlottedPointMeta even with marker: None");
    let mut traj_lens: Vec<usize> = scene_sp
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, stroke: Some(st), clip: Some(_), .. } if st.color == Rgba8::rgb(0, 114, 178) => Some(path.commands.len()),
            _ => None,
        })
        .collect();
    traj_lens.sort_unstable();
    assert_eq!(
        traj_lens,
        vec![2, 2, 5],
        "subcol 0 emits one 5-point trajectory while gapped subcol 1 splits into two 2-point runs without bridging or zero-coercing"
    );
}

#[test]
fn heatmap_thirteen_gene_label_modes_and_angles() {
    let mut mv = MultipleVariablesTable::new(6, 13).expect("13x6 gene matrix");
    mv.row_titles = (1..=13).map(|i| format!("Gene-{i:02}")).collect();
    for c in 0..6 {
        mv.variables[c] = VariableColumn::new(format!("SampleCondition{}", (b'A' + c as u8) as char), 13).expect("var");
        mv.variables[c].role = VariableRole::Continuous;
        for r in 0..13 {
            mv.variables[c].cells[r] = Cell::number(((r * 7 + c * 3) % 19) as f64 - 5.0);
        }
    }
    let ds = DataSheet::new(SheetId::new("hm-13"), "Genes", TableArchetype::MultipleVariables(mv)).expect("ds");
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone()).expect("add");

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-hm"), &ds).expect("gs");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);

    // Horizontal0 vs Vertical90 dynamically increases bottom margin
    gs.spec.heatmap_labels = HeatmapLabelSpec {
        row_labels: HeatmapRowLabelMode::RowTitles,
        column_labels: HeatmapColumnLabelMode::ColumnTitles,
        column_angle: HeatmapColumnLabelAngle::Horizontal0,
    };
    let s_h = compile_graph(&gs, &doc).expect("compile 0deg");

    gs.spec.heatmap_labels.column_angle = HeatmapColumnLabelAngle::Vertical90;
    let s_v = compile_graph(&gs, &doc).expect("compile 90deg");
    assert!(s_v.plot_rect_pt.bottom() < s_h.plot_rect_pt.bottom() - 5.0, "Vertical90 must reserve larger bottom margin");
    let has_90 = s_v.primitives.iter().any(
        |p| matches!(p, ScenePrimitive::Text(t) if (t.rotation_deg.abs() - 90.0).abs() < 1e-3 && t.rich.plain_text().starts_with("SampleCondition")),
    );
    assert!(has_90, "expected +-90 deg rotated column titles");

    gs.spec.heatmap_labels.column_angle = HeatmapColumnLabelAngle::Angled45;
    let s_45 = compile_graph(&gs, &doc).expect("compile 45deg");
    assert!(s_45.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if (t.rotation_deg.abs() - 45.0).abs() < 1e-3)));

    // RowNumbers & ColumnLetters
    gs.spec.heatmap_labels = HeatmapLabelSpec {
        row_labels: HeatmapRowLabelMode::RowNumbers,
        column_labels: HeatmapColumnLabelMode::ColumnLetters,
        column_angle: HeatmapColumnLabelAngle::Horizontal0,
    };
    let s_num = compile_graph(&gs, &doc).expect("compile RowNumbers/ColLetters");
    let num_texts = plain_texts(&s_num);
    assert!(num_texts.iter().any(|t| t == "13") && num_texts.iter().any(|t| t == "F") && !num_texts.iter().any(|t| t == "Gene-01"));

    // None hides both row and col labels
    gs.spec.heatmap_labels = HeatmapLabelSpec {
        row_labels: HeatmapRowLabelMode::None,
        column_labels: HeatmapColumnLabelMode::None,
        column_angle: HeatmapColumnLabelAngle::Horizontal0,
    };
    let s_none = compile_graph(&gs, &doc).expect("compile None");
    let none_texts = plain_texts(&s_none);
    assert!(!none_texts.iter().any(|t| t == "Gene-01" || t.starts_with("SampleCondition")));
}

#[test]
fn embedded_result_table_and_live_table_unified_features() {
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).expect("col");
    let ds = DataSheet::new(SheetId::new("d-1"), "Data", TableArchetype::Column(col)).expect("ds");
    let mut res =
        ResultSheet::new(AnalysisSpec::new("r-1", "d-1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).expect("spec")).expect("res");
    res.status = ResultStatus::UpToDate;
    res.tabs = vec![ResultTab {
        id: String::from("anova"),
        title: String::from("ANOVA Table"),
        kind: ResultTabKind::AnovaTable,
        column_headers: vec![String::from("SS"), String::from("DF"), String::from("P value")],
        row_labels: vec![String::from("Between"), String::from("Residual"), String::from("Total")],
        rows: vec![
            vec![
                ResultCell::Number { value: 42.5, prefix: None, units: None },
                ResultCell::Number { value: 2.0, prefix: None, units: None },
                ResultCell::PValue { raw: 0.0008, formatted: String::from("0.0008"), stars: String::from("***") },
            ],
            vec![
                ResultCell::Number { value: 11.0, prefix: None, units: None },
                ResultCell::Number { value: 12.0, prefix: None, units: None },
                ResultCell::Empty,
            ],
            vec![
                ResultCell::Number { value: 53.5, prefix: None, units: None },
                ResultCell::Number { value: 14.0, prefix: None, units: None },
                ResultCell::Empty,
            ],
        ],
        footnotes: Vec::new(),
    }];

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-1"), &ds).expect("gs");
    gs.initialized = true;
    gs.spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
        result_sheet_id: SheetId::new("r-1"),
        tab_id: Some(String::from("anova")),
        bounds_pt: DocRectPt::new(80.0, 40.0, 260.0, 120.0),
        selected_rows: Some(vec![0, 2]),
        selected_cols: Some(vec![0, 2]),
        style: EmbeddedTableStyle {
            transpose: true,
            show_column_titles: true,
            show_row_titles: true,
            bold_headers: true,
            font_size_pt: 8.0,
            outer_border_width_pt: 1.0,
            gridline_width_pt: 0.5,
            border_color: Rgba8::BLACK,
            fill_color: Some(Rgba8::WHITE),
        },
    });

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds).expect("add data");
    doc.add_result_sheet(res.clone()).expect("add res");
    doc.add_graph_sheet(gs.clone()).expect("add graph");

    let g_scene = compile_graph(&gs, &doc).expect("compile graph w/ transposed table");
    let g_texts = plain_texts(&g_scene);
    assert!(g_texts.iter().any(|t| t == "SS") && g_texts.iter().any(|t| t == "P value"), "selected cols expected: {g_texts:?}");
    assert!(!g_texts.iter().any(|t| t == "DF") && !g_texts.iter().any(|t| t == "Residual"), "unselected slice should be excluded");
    assert!(g_texts.iter().any(|t| t.contains("0.0008 ***")), "multi-column cell + p stars must render: {g_texts:?}");

    // Transposed layout: "P value" row appears below "SS" row (`y_pval > y_ss`)
    let y_ss = g_scene
        .primitives
        .iter()
        .find_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text() == "SS" => Some(t.anchor_pt.y),
            _ => None,
        })
        .unwrap();
    let y_p = g_scene
        .primitives
        .iter()
        .find_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text() == "P value" => Some(t.anchor_pt.y),
            _ => None,
        })
        .unwrap();
    assert!(y_p > y_ss, "transposed columns become table rows: SS y={y_ss}, P value y={y_p}");

    // Layout LiveTable mirrors same rendering pipeline
    let mut l_spec = LayoutSpec::default();
    l_spec
        .add_item(LayoutItem::LiveTable {
            id: String::from("tbl-1"),
            source_sheet_id: SheetId::new("r-1"),
            tab_id: Some(String::from("anova")),
            bounds_pt: DocRectPt::new(30.0, 30.0, 280.0, 120.0),
            selected_rows: Some(vec![0]),
            selected_cols: Some(vec![0, 1, 2]),
            style: EmbeddedTableStyle::default(),
        })
        .expect("add live table");
    let layout = LayoutSheet::new(SheetId::new("lay-1"), "Layout", l_spec).expect("layout");
    let l_scene = compile_layout(&layout, &doc).expect("compile layout live table");
    assert!(plain_texts(&l_scene).iter().any(|t| t == "42.5" || t.contains("42.5")));

    // Missing tab emits warning; ResultStatus::Failed omits cell text safely without panic
    let mut gs_missing_tab = gs.clone();
    if let GraphAnnotation::EmbeddedResultTable { tab_id, .. } = &mut gs_missing_tab.spec.annotations[0] {
        *tab_id = Some(String::from("nonexistent-tab"));
    }
    let s_miss = compile_graph(&gs_missing_tab, &doc).expect("compile missing tab");
    assert!(s_miss.warnings.iter().any(|w| w.code == "missing_result_tab"));

    doc.result_sheet_mut(&SheetId::new("r-1")).unwrap().status =
        ResultStatus::Failed { code: curvebook_doc::ErrorCode::SolverFailed, message: String::from("simulated failure") };
    let s_fail = compile_graph(&gs, &doc).expect("compile failed res");
    assert!(!plain_texts(&s_fail).iter().any(|t| t.contains("0.0008")), "failed result must render zero table text");
}

#[test]
fn axis_tick_directions_labels_and_transparent_paper_fill() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3).expect("xy");
    for (r, (x, y)) in [(0.0, 0.0), (5.0, 5.0), (10.0, 10.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new(SheetId::new("xy-t"), "Ticks", TableArchetype::Xy(xy)).expect("ds");
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone()).expect("add");

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-ticks"), &ds).expect("gs");
    gs.initialized = true;
    gs.spec.paper_fill = None;
    gs.spec.x_axis.show_tick_labels = false;
    gs.spec.x_axis.tick_direction = TickDirection::None;
    gs.spec.y_axis.tick_direction = TickDirection::Inside;

    let s_inside_none = compile_graph(&gs, &doc).expect("compile");
    assert_eq!(s_inside_none.background, None, "transparent paper_fill: None must set scene.background = None");

    // X tick_direction: None & show_tick_labels: false -> zero short vertical tick stubs and zero X numbers below plot_rect_pt.bottom()
    let x_bottom = s_inside_none.plot_rect_pt.bottom();
    let x_ticks = s_inside_none
        .primitives
        .iter()
        .filter(|p| match p {
            ScenePrimitive::Path { path, stroke: Some(_), clip: None, .. } => path_segments(path)
                .into_iter()
                .any(|(a, b)| (a.x - b.x).abs() < 0.1 && (a.y - b.y).abs() <= 8.0 && (a.y.min(b.y) - x_bottom).abs() <= 5.0),
            _ => false,
        })
        .count();
    assert_eq!(x_ticks, 0, "TickDirection::None must suppress X ticks");
    let x_nums: Vec<String> = s_inside_none
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) if t.anchor_pt.y > x_bottom && t.anchor_pt.y < s_inside_none.height_pt - 15.0 && t.rich.plain_text() != "X" => {
                Some(t.rich.plain_text())
            }
            _ => None,
        })
        .collect();
    assert!(x_nums.is_empty(), "show_tick_labels=false must suppress X tick numbers: {x_nums:?}");

    // Y Inside ticks lie strictly to the right of plot_rect_pt.x (x >= plot_rect_pt.x)
    let y_spine_x = s_inside_none.plot_rect_pt.x;
    let y_tick_segs: Vec<(PointPt, PointPt)> = s_inside_none
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, stroke: Some(_), clip: None, .. } => Some(path_segments(path)),
            _ => None,
        })
        .flatten()
        .filter(|(a, b)| {
            (a.y - b.y).abs() < 0.1 && (a.x - b.x).abs() > 0.5 && (a.x - b.x).abs() <= 8.0 && a.y >= s_inside_none.plot_rect_pt.y && a.y <= x_bottom
        })
        .collect();
    assert!(!y_tick_segs.is_empty(), "expected Y Inside tick marks");
    assert!(
        y_tick_segs.iter().all(|(a, b)| a.x.min(b.x) >= y_spine_x - 0.5 && a.x.max(b.x) > y_spine_x + 1.0),
        "Y Inside ticks must point rightward into plot frame: {y_tick_segs:?}"
    );

    // TickDirection::Both spans across spine on both sides
    gs.spec.x_axis.tick_direction = TickDirection::Both;
    let s_both = compile_graph(&gs, &doc).expect("compile both");
    let bot = s_both.plot_rect_pt.bottom();
    let x_both_segs: Vec<(PointPt, PointPt)> = s_both
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, stroke: Some(_), clip: None, .. } => Some(path_segments(path)),
            _ => None,
        })
        .flatten()
        .filter(|(a, b)| {
            (a.x - b.x).abs() < 0.1
                && (a.y - b.y).abs() > 2.0
                && (a.y - b.y).abs() <= 12.0
                && a.x > s_both.plot_rect_pt.x
                && a.x < s_both.plot_rect_pt.right()
        })
        .collect();
    assert!(!x_both_segs.is_empty());
    assert!(x_both_segs.iter().any(|(a, b)| a.y.min(b.y) < bot - 1.0 && a.y.max(b.y) > bot + 1.0), "Both ticks must cross X spine");
}
