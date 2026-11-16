//! Permanent regression tests for Round 2 (`R0.19`) Slices `S08` (`R2-S08-01..06`)
//! and `S11-16` (`R2-S11-16` single-axis `ModelEquation` overflow clipping).

use lumencraft_chart::equation_box::{ModelEquationParams, render_model_equation_annotation, render_tex_equation_box};
use lumencraft_chart::{
    ChartScene, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, SourceCellId, TextAnchor, compile_graph, preflight_check_scene,
    select_point_override,
};
use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, Cell, ClusterDistance, ClusterLinkage, ColorChannelMapping, ColumnDesignHint, ColumnTable,
    DataSheet, DiagnosticPlotKind, EquationDisplayMode, FittedCurveOverlay, GraphKind, GraphSheet, GraphicalDataObject, GroupedArrangement,
    GroupedTable, LinearRegressionSpec, MultipleRowTestsSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec,
    MultivariateGraphEncoding, MultivariatePlotMode, MultivariateSpec, NonlinearFitSpec, PointStyleOverride, ProjectDocument, RectPt as DocRectPt,
    ResultScalarEntry, ResultSheet, ResultStatus, ResultTabKind, SheetId, SizeChannelMapping, SubcolumnMode, TableArchetype, XyTable,
};
use lumencraft_palette::{MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use lumencraft_stats::multivariate::run_multivariate_analysis;
use lumencraft_stats::two_group::{run_bland_altman_analysis, run_multiple_row_t_tests_analysis};
use lumencraft_text::{RichSpan, RichText, SpanScript, SpanSlope, SpanWeight};

type TestResult = Result<(), Box<dyn std::error::Error>>;

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

fn filled_marker_centers(scene: &ChartScene) -> Vec<PointPt> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } => {
                let mut xs = Vec::new();
                let mut ys = Vec::new();
                for cmd in &path.commands {
                    match *cmd {
                        PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => {
                            xs.push(pt.x);
                            ys.push(pt.y);
                        }
                        PathCmd::CubicTo { c1, c2, to } => {
                            xs.extend([c1.x, c2.x, to.x]);
                            ys.extend([c1.y, c2.y, to.y]);
                        }
                        PathCmd::QuadTo { control, to } => {
                            xs.extend([control.x, to.x]);
                            ys.extend([control.y, to.y]);
                        }
                        PathCmd::Close => {}
                    }
                }
                if xs.is_empty() {
                    return None;
                }
                let x_mid = 0.5 * (xs.iter().copied().fold(f64::INFINITY, f64::min) + xs.iter().copied().fold(f64::NEG_INFINITY, f64::max));
                let y_mid = 0.5 * (ys.iter().copied().fold(f64::INFINITY, f64::min) + ys.iter().copied().fold(f64::NEG_INFINITY, f64::max));
                Some(PointPt::new(x_mid, y_mid))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn r2_s08_01_model_equation_never_fabricates_defaults_or_uses_summary_scalars() -> TestResult {
    let mut doc = ProjectDocument::new("EqSafety")?;
    let ds_id = SheetId::new("xy-1");

    // 1. LinearRegression where dataset 0 converged and dataset 1 has NO scalars (plus shared R2/RSS).
    let lin_spec = AnalysisSpec::new(SheetId::new("res-lin"), ds_id.clone(), AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?;
    let mut lin_res = ResultSheet::new(lin_spec)?;
    lin_res.status = ResultStatus::UpToDate;
    lin_res.scalars = vec![
        ResultScalarEntry { key: String::from("Slope"), data_set_index: Some(0), value: 3.25, units: None },
        ResultScalarEntry { key: String::from("YIntercept"), data_set_index: Some(0), value: 4.5, units: None },
        ResultScalarEntry { key: String::from("R2"), data_set_index: None, value: 0.991, units: None },
        ResultScalarEntry { key: String::from("RSS"), data_set_index: None, value: 0.42, units: None },
    ];
    let lin_id = lin_res.id.clone();
    doc.result_sheets.push(lin_res);

    let mut lin_ds1_scene = ChartScene::new("s-lin-1", "LinDS1", 320.0, 200.0, Some(Rgba8::WHITE));
    render_model_equation_annotation(
        &mut lin_ds1_scene,
        &doc,
        &ModelEquationParams {
            result_sheet_id: &lin_id,
            data_set_index: 1,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: DocRectPt::new(20.0, 20.0, 0.0, 0.0),
            font_size_pt: 10.0,
            bordered: true,
        },
    );
    let lin_texts = scene_texts(&lin_ds1_scene).join(" | ");
    assert!(lin_texts.contains("unavailable"), "expected missing-dataset placeholder, got: {lin_texts}");
    assert!(!lin_texts.contains("1 · X") && !lin_texts.contains("+ 0"), "must never fabricate 1.0/0.0: {lin_texts}");
    assert!(!lin_texts.contains("0.991") && !lin_texts.contains("0.42"), "must never leak R2/RSS: {lin_texts}");

    // 2. Pending (`Dirty`) LinearRegression must render a pending placeholder rather than fabricated 1.0 / 0.0.
    let dirty_spec = AnalysisSpec::new(SheetId::new("res-dirty"), ds_id.clone(), AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?;
    let dirty_res = ResultSheet::new(dirty_spec)?;
    let dirty_id = dirty_res.id.clone();
    doc.result_sheets.push(dirty_res);

    let mut dirty_scene = ChartScene::new("s-dirty", "Dirty", 320.0, 200.0, Some(Rgba8::WHITE));
    render_model_equation_annotation(
        &mut dirty_scene,
        &doc,
        &ModelEquationParams {
            result_sheet_id: &dirty_id,
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: DocRectPt::new(20.0, 20.0, 0.0, 0.0),
            font_size_pt: 10.0,
            bordered: false,
        },
    );
    let dirty_texts = scene_texts(&dirty_scene).join(" | ");
    assert!(dirty_texts.contains("pending"), "expected pending placeholder, got: {dirty_texts}");
    assert!(!dirty_texts.contains("1 · X"), "must not fabricate Slope=1.0 on dirty sheet: {dirty_texts}");

    // 3. Polynomial (`polynomial_2`) with only shared GOF scalars (`R2`, `RSS`) or partial params must NOT substitute R2/RSS or 0.0.
    let poly_spec =
        AnalysisSpec::new(SheetId::new("res-poly"), ds_id.clone(), AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("polynomial_2")))?;
    let mut poly_res = ResultSheet::new(poly_spec)?;
    poly_res.status = ResultStatus::UpToDate;
    poly_res.scalars = vec![
        ResultScalarEntry { key: String::from("R2"), data_set_index: None, value: 0.876, units: None },
        ResultScalarEntry { key: String::from("RSS"), data_set_index: None, value: 12.34, units: None },
    ];
    let poly_id = poly_res.id.clone();
    doc.result_sheets.push(poly_res);

    let mut poly_scene = ChartScene::new("s-poly", "Poly", 320.0, 200.0, Some(Rgba8::WHITE));
    render_model_equation_annotation(
        &mut poly_scene,
        &doc,
        &ModelEquationParams {
            result_sheet_id: &poly_id,
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: DocRectPt::new(20.0, 20.0, 0.0, 0.0),
            font_size_pt: 10.0,
            bordered: true,
        },
    );
    let poly_texts = scene_texts(&poly_scene).join(" | ");
    assert!(poly_texts.contains("unavailable"), "expected unavailable placeholder for GOF-only polynomial: {poly_texts}");
    assert!(!poly_texts.contains("0.876") && !poly_texts.contains("12.34"), "R2/RSS leaked into polynomial: {poly_texts}");

    // 4. Nonlinear multi-dataset with overlay on dataset 0 + global parameter `Plateau` must not bleed onto missing/unconverged dataset 1.
    let nl_spec = AnalysisSpec::new(SheetId::new("res-nl"), ds_id, AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("one_phase_decay")))?;
    let mut nl_res = ResultSheet::new(nl_spec)?;
    nl_res.status = ResultStatus::UpToDate;
    nl_res.scalars = vec![
        ResultScalarEntry { key: String::from("Y0"), data_set_index: Some(0), value: 100.0, units: None },
        ResultScalarEntry { key: String::from("K"), data_set_index: Some(0), value: 0.5, units: None },
        ResultScalarEntry { key: String::from("Plateau"), data_set_index: None, value: 10.0, units: None },
    ];
    nl_res.fitted_curves = vec![FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("one_phase_decay"),
        converged: true,
        ambiguous: false,
        points: Vec::new(),
    }];
    let nl_id = nl_res.id.clone();
    doc.result_sheets.push(nl_res);

    let mut nl_ds1_scene = ChartScene::new("s-nl-1", "NLDS1", 320.0, 200.0, Some(Rgba8::WHITE));
    render_model_equation_annotation(
        &mut nl_ds1_scene,
        &doc,
        &ModelEquationParams {
            result_sheet_id: &nl_id,
            data_set_index: 1,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: DocRectPt::new(20.0, 20.0, 0.0, 0.0),
            font_size_pt: 10.0,
            bordered: true,
        },
    );
    let nl_texts = scene_texts(&nl_ds1_scene).join(" | ");
    assert!(nl_texts.contains("unavailable"), "missing overlay on ds 1 must render placeholder: {nl_texts}");
    assert!(!nl_texts.contains("100") && !nl_texts.contains("10"), "ds 0 / global scalar leaked to ds 1: {nl_texts}");
    Ok(())
}

#[test]
fn r2_s08_02_superimposed_before_after_connectors_never_swap_columns() -> TestResult {
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 2)?;
    col.data_sets[0].title = String::from("Before");
    col.data_sets[1].title = String::from("After");
    // Before has tiny values (~2..4); After has large values (~50..60) so z-footprint sorting puts After before Before!
    for (r, (b, a)) in [(2.0, 50.0), (4.0, 60.0)].into_iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(b);
        col.data_sets[1].cells[r][0] = Cell::number(a);
    }
    let ds = DataSheet::new(SheetId::new("ba-col"), "BeforeAfter", TableArchetype::Column(col))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;

    let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new("g-ba"), &ds)?;
    graph.spec.grouped_arrangement = GroupedArrangement::Superimposed;
    for (c, s) in graph.spec.series.iter_mut().enumerate() {
        s.gdo = GraphicalDataObject::BeforeAfterLines {
            line_color: Rgba8::BLACK,
            stroke_width_pt: 1.0,
            marker: Some(MarkerStyle {
                shape: MarkerShape::Circle,
                size_pt: 6.0,
                fill_mode: MarkerFillMode::Filled,
                fill_color: graph.spec.palette.color_at(c),
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.5,
            }),
        };
    }

    let scene = compile_graph(&graph, &doc)?;
    // Verify two connector lines connect left-to-right going UPWARD (Before small value -> lower pixel Y in Y-down -> py_before > py_after).
    let connectors: Vec<(PointPt, PointPt)> = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() == 2 => {
                match (path.commands[0], path.commands[1]) {
                    (PathCmd::MoveTo(p0), PathCmd::LineTo(p1)) if (p1.x - p0.x).abs() > 10.0 => Some((p0, p1)),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect();
    assert_eq!(connectors.len(), 2, "expected 2 Before-After connector lines: {connectors:?}");
    for (p0, p1) in &connectors {
        assert!(p0.x < p1.x, "connector must start at Before x ({}) < After x ({})", p0.x, p1.x);
        assert!(p0.y > p1.y, "Before (2/4) must sit below After (50/60) in Y-down coords: p0={p0:?}, p1={p1:?}");
    }

    // Verify plotted_points provenance maps lower-value points to col=0 (Before) and higher-value points to col=1 (After).
    let before_pts: Vec<&lumencraft_chart::PlottedPointMeta> = scene.plotted_points.iter().filter(|m| m.source_cell.col == 0).collect();
    let after_pts: Vec<&lumencraft_chart::PlottedPointMeta> = scene.plotted_points.iter().filter(|m| m.source_cell.col == 1).collect();
    assert_eq!(before_pts.len(), 2);
    assert_eq!(after_pts.len(), 2);
    assert!(before_pts.iter().all(|b| after_pts.iter().all(|a| b.center_pt.y > a.center_pt.y && b.center_pt.x < a.center_pt.x)));

    // Hiding point (series_index=0, point_index=0) must hide ONLY the col=0 row=0 marker (`Before` row 0).
    let mut g_hidden = graph;
    g_hidden.spec.point_overrides.push(PointStyleOverride { series_index: 0, point_index: 0, hidden: true, ..Default::default() });
    let scene_hidden = compile_graph(&g_hidden, &doc)?;
    assert_eq!(scene_hidden.plotted_points.len(), 3);
    assert!(!scene_hidden.plotted_points.iter().any(|m| m.source_cell.col == 0 && m.source_cell.row == 0));
    Ok(())
}

#[test]
fn r2_s08_03_bubble_and_diagnostic_plots_honor_point_overrides_brushing_and_subcol_helper() -> TestResult {
    // 1. Bubble Scatter (`y_column = 2`): PointStyleOverride must match `series_index = 2`, NOT `0`.
    let mut mv = MultipleVariablesTable::new(4, 4)?;
    for (r, (x, unused, y, sz)) in
        [(1.0, 99.0, 10.0, 5.0), (2.0, 88.0, 20.0, 8.0), (3.0, 77.0, 30.0, 12.0), (4.0, 66.0, 40.0, 15.0)].into_iter().enumerate()
    {
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(unused);
        mv.variables[2].cells[r] = Cell::number(y);
        mv.variables[3].cells[r] = Cell::number(sz);
    }
    let ds_mv = DataSheet::new(SheetId::new("mv-bubble"), "Bubble", TableArchetype::MultipleVariables(mv))?;
    let mut doc_bubble = ProjectDocument::default();
    doc_bubble.add_data_sheet(ds_mv.clone())?;
    let mut g_bubble = GraphSheet::new_uninitialized_for_data(SheetId::new("g-bubble"), &ds_mv)?;
    g_bubble.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 2,
        color: ColorChannelMapping::Constant(Rgba8::rgb(40, 120, 200)),
        size: SizeChannelMapping::Variable { column: 3, min_radius_pt: 3.0, max_radius_pt: 8.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    // Series 0 override must NOT affect y_column=2; Series 2 row 1 override MUST hide row 1.
    g_bubble.spec.point_overrides = vec![
        PointStyleOverride { series_index: 0, point_index: 0, hidden: true, ..Default::default() },
        PointStyleOverride { series_index: 2, point_index: 1, hidden: true, ..Default::default() },
    ];
    let sc_bubble = compile_graph(&g_bubble, &doc_bubble)?;
    assert_eq!(sc_bubble.plotted_points.len(), 3, "expected 3 visible bubbles after hiding (col=2, row=1)");
    assert!(sc_bubble.plotted_points.iter().all(|p| p.source_cell.col == 2));
    assert!(!sc_bubble.plotted_points.iter().any(|p| p.source_cell.row == 1));
    assert!(sc_bubble.plotted_points.iter().any(|p| p.source_cell.row == 0));

    // 2. Normal Q-Q Plot: sorting + blank row `1` in dataset `1` must preserve true `(col=1, row)` provenance.
    let mut qq_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    qq_col.data_sets[0].title = String::from("Ctrl");
    qq_col.data_sets[1].title = String::from("Treated");
    for (r, v) in [(0, Some(40.0)), (1, None), (2, Some(10.0)), (3, Some(20.0))] {
        qq_col.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
        if let Some(val) = v {
            qq_col.data_sets[1].cells[r][0] = Cell::number(val);
        }
    }
    let ds_qq = DataSheet::new(SheetId::new("qq-col"), "QQ", TableArchetype::Column(qq_col))?;
    let mut doc_qq = ProjectDocument::default();
    doc_qq.add_data_sheet(ds_qq.clone())?;
    let mut g_qq = GraphSheet::new_uninitialized_for_data(SheetId::new("g-qq"), &ds_qq)?;
    g_qq.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
    g_qq.spec.series = vec![g_qq.spec.series[1].clone()];
    let sc_qq_all = compile_graph(&g_qq, &doc_qq)?;
    assert_eq!(sc_qq_all.plotted_points.len(), 3);
    // Smallest Y (10.0) came from row 2 -> highest pixel Y on screen!
    let lowest_y_pt = sc_qq_all.plotted_points.iter().max_by(|a, b| a.center_pt.y.total_cmp(&b.center_pt.y)).ok_or("missing qq min point")?;
    assert_eq!(lowest_y_pt.source_cell, SourceCellId { sheet_id: SheetId::new("qq-col"), row: 2, col: 1, subcol: 0 });
    assert_eq!(
        sc_qq_all.hit_test_point(lowest_y_pt.center_pt, 2.0).map(|m| &m.source_cell),
        Some(&lowest_y_pt.source_cell),
        "Normal Q-Q hit_test_point must resolve brushed cell"
    );

    // Hiding (series=1, row=2) hides ONLY the y=10.0 point (`lowest_y_pt`), NOT row=0 or rank=2!
    let mut g_qq_hide = g_qq;
    g_qq_hide.spec.point_overrides.push(PointStyleOverride { series_index: 1, point_index: 2, hidden: true, ..Default::default() });
    let sc_qq_hide = compile_graph(&g_qq_hide, &doc_qq)?;
    assert_eq!(sc_qq_hide.plotted_points.len(), 2);
    assert!(!sc_qq_hide.plotted_points.iter().any(|p| p.source_cell.row == 2));

    // 3. ResidualPlot with blank row `1` keeps true uncompacted row `2` and populates `plotted_points`.
    let mut xy_res = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    for (r, opt) in [(0, Some((1.0, 2.0))), (1, None), (2, Some((3.0, 8.0))), (3, Some((4.0, 9.0)))] {
        if let Some((x, y)) = opt {
            xy_res.set_x_cell(r, Cell::number(x))?;
            xy_res.data_sets[0].cells[r][0] = Cell::number(y);
        }
    }
    let ds_res = DataSheet::new(SheetId::new("xy-res"), "Res", TableArchetype::Xy(xy_res))?;
    let mut doc_res = ProjectDocument::default();
    doc_res.add_data_sheet(ds_res.clone())?;
    let mut g_res = GraphSheet::new_uninitialized_for_data(SheetId::new("g-res"), &ds_res)?;
    g_res.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot);
    g_res.spec.point_overrides.push(PointStyleOverride { series_index: 0, point_index: 2, hidden: true, ..Default::default() });
    let sc_res = compile_graph(&g_res, &doc_res)?;
    assert_eq!(sc_res.plotted_points.len(), 2);
    assert!(!sc_res.plotted_points.iter().any(|p| p.source_cell.row == 2));
    assert!(sc_res.plotted_points.iter().any(|p| p.source_cell.row == 3));

    // 4. BlandAltman (`column_a=1`, `column_b=2`) with missing row `0` populates `plotted_points` and respects `(col=1, row=2)`.
    let mut col_ba = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 4)?;
    for (r, vals) in [(1, (10.0, 11.0)), (2, (20.0, 22.0)), (3, (30.0, 31.0))] {
        col_ba.data_sets[1].cells[r][0] = Cell::number(vals.0);
        col_ba.data_sets[2].cells[r][0] = Cell::number(vals.1);
    }
    let ds_ba = DataSheet::new(SheetId::new("ba-col-src"), "BA", TableArchetype::Column(col_ba))?;
    let ba_spec = BlandAltmanSpec { column_a: 1, column_b: 2, percent_difference: false };
    let an_ba = AnalysisSpec::new(SheetId::new("res-ba"), ds_ba.id.clone(), AnalysisKind::BlandAltman(ba_spec))?;
    let res_ba = run_bland_altman_analysis(&ds_ba, &an_ba, &ba_spec)?;
    let mut doc_ba = ProjectDocument::default();
    doc_ba.add_data_sheet(ds_ba.clone())?;
    doc_ba.add_result_sheet(res_ba.clone())?;
    let mut g_ba = GraphSheet::new_uninitialized_for_data(SheetId::new("g-ba-diag"), &ds_ba)?;
    g_ba.spec.primary_source_sheet_id = res_ba.id;
    g_ba.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman);
    g_ba.spec.point_overrides.push(PointStyleOverride { series_index: 1, point_index: 2, hidden: true, ..Default::default() });
    let sc_ba = compile_graph(&g_ba, &doc_ba)?;
    assert_eq!(sc_ba.plotted_points.len(), 2);
    assert!(!sc_ba.plotted_points.iter().any(|p| p.source_cell.row == 2));
    assert!(sc_ba.plotted_points.iter().all(|p| p.source_cell.col == 1));

    // 5. Volcano from MultipleRowTTests where row `1` was skipped keeps `rec.group_a` row `2`.
    let mut grp_vol = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3)?;
    grp_vol.row_titles = vec![String::from("G0"), String::from("GSkipped"), String::from("G2")];
    for (r, a_vals, b_vals) in [(0, (1.0, 2.0), (5.0, 6.0)), (2, (2.0, 3.0), (9.0, 10.0))] {
        grp_vol.data_sets[0].cells[r][0] = Cell::number(a_vals.0);
        grp_vol.data_sets[0].cells[r][1] = Cell::number(a_vals.1);
        grp_vol.data_sets[1].cells[r][0] = Cell::number(b_vals.0);
        grp_vol.data_sets[1].cells[r][1] = Cell::number(b_vals.1);
    }
    let ds_vol = DataSheet::new(SheetId::new("vol-ds"), "Vol", TableArchetype::Grouped(grp_vol))?;
    let mr_spec = MultipleRowTestsSpec {
        paired: false,
        nonparametric: false,
        assume_equal_sd: false,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::BenjaminiHochbergFdr, threshold: 0.05, report_confidence_intervals: false },
    };
    let an_mr = AnalysisSpec::new(SheetId::new("res-mr"), ds_vol.id.clone(), AnalysisKind::MultipleRowTTests(mr_spec))?;
    let res_mr = run_multiple_row_t_tests_analysis(&ds_vol, &an_mr, &mr_spec)?;
    let mut doc_vol = ProjectDocument::default();
    doc_vol.add_data_sheet(ds_vol.clone())?;
    doc_vol.add_result_sheet(res_mr.clone())?;
    let mut g_vol = GraphSheet::new_uninitialized_for_data(SheetId::new("g-vol"), &ds_vol)?;
    g_vol.spec.primary_source_sheet_id = res_mr.id;
    g_vol.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Volcano);
    g_vol.spec.point_overrides.push(PointStyleOverride { series_index: 0, point_index: 2, hidden: true, ..Default::default() });
    let sc_vol = compile_graph(&g_vol, &doc_vol)?;
    assert_eq!(sc_vol.plotted_points.len(), 1);
    assert_eq!(sc_vol.plotted_points[0].source_cell.row, 0);

    // 6. `select_point_override` multi-replicate subcol lookup prefers exact subcol match and falls back to None.
    let overrides = [
        PointStyleOverride { series_index: 0, point_index: 1, hidden: false, color: Some(Rgba8::rgb(0, 0, 255)), ..Default::default() },
        PointStyleOverride { series_index: 0, point_index: 1, hidden: true, ..Default::default() },
    ];
    let subcol_tags = [None, Some(2)];
    let sub_of =
        |ov: &PointStyleOverride| -> Option<usize> { overrides.iter().position(|item| std::ptr::eq(item, ov)).and_then(|idx| subcol_tags[idx]) };
    let rep2 = select_point_override(&overrides, 0, 1, Some(2), sub_of).ok_or("expected exact subcol=2 match")?;
    assert!(rep2.hidden, "replicate subcol=2 must match subcol-scoped override");
    let rep0 = select_point_override(&overrides, 0, 1, Some(0), sub_of).ok_or("expected wildcard fallback for subcol=0")?;
    assert!(!rep0.hidden && rep0.color == Some(Rgba8::rgb(0, 0, 255)), "sibling subcol=0 must fall back to row wildcard, never subcol=2");
    Ok(())
}

#[test]
fn r2_s08_04_residual_and_qq_never_consume_bland_altman_pair_coords() -> TestResult {
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 6)?;
    // Values A ~ [10..15] (centered near 12.5, small OLS residuals), B = A - 1000 -> BlandAltman Diff = +1000!
    for r in 0..6 {
        let a = 10.0 + (r as f64);
        let b = a - 1000.0;
        col.data_sets[0].cells[r][0] = Cell::number(a);
        col.data_sets[1].cells[r][0] = Cell::number(b);
    }
    let ds = DataSheet::new(SheetId::new("ba-trap"), "Trap", TableArchetype::Column(col))?;
    let ba_spec = BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false };
    let an_ba = AnalysisSpec::new(SheetId::new("res-ba-trap"), ds.id.clone(), AnalysisKind::BlandAltman(ba_spec))?;
    let res_ba = run_bland_altman_analysis(&ds, &an_ba, &ba_spec)?;
    assert!(res_ba.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals), "BlandAltman emits Residuals-kind coord tab");

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res_ba.clone())?;

    for src in [ds.id.clone(), res_ba.id.clone()] {
        let mut g_res = GraphSheet::new_uninitialized_for_data(SheetId::new("g-res-trap"), &ds)?;
        g_res.spec.primary_source_sheet_id = src.clone();
        g_res.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot);
        let sc_res = compile_graph(&g_res, &doc)?;
        let res_texts = scene_texts(&sc_res);
        assert!(!res_texts.iter().any(|t| t == "1000"), "ResidualPlot ({src}) must NOT plot Bland-Altman Diff=1000: {res_texts:?}");
        assert_eq!(filled_marker_centers(&sc_res).len(), 6);

        let mut g_qq = GraphSheet::new_uninitialized_for_data(SheetId::new("g-qq-trap"), &ds)?;
        g_qq.spec.primary_source_sheet_id = src.clone();
        g_qq.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
        let sc_qq = compile_graph(&g_qq, &doc)?;
        let qq_texts = scene_texts(&sc_qq);
        assert!(!qq_texts.iter().any(|t| t == "1000"), "NormalQqPlot ({src}) must NOT plot Bland-Altman Diff=1000: {qq_texts:?}");
        assert_eq!(filled_marker_centers(&sc_qq).len(), 6);
    }
    Ok(())
}

#[test]
fn r2_s08_05_clustered_heatmap_respects_selected_data_sets() -> TestResult {
    let mut mv = MultipleVariablesTable::new(4, 4)?;
    mv.variables[0].title = String::from("FeatA");
    mv.variables[1].title = String::from("UnselectedMissing");
    mv.variables[2].title = String::from("FeatC");
    mv.variables[3].title = String::from("UnselectedD");
    for r in 0..4 {
        mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
        if r != 1 {
            // Unselected column has a missing value that would poison has_missing if included.
            mv.variables[1].cells[r] = Cell::number(50.0 + r as f64);
        }
        mv.variables[2].cells[r] = Cell::number(10.0 * ((r + 1) as f64));
        mv.variables[3].cells[r] = Cell::number(200.0 + r as f64);
    }
    let ds = DataSheet::new(SheetId::new("hm-sel-ds"), "Subset", TableArchetype::MultipleVariables(mv))?;

    let clust_spec =
        MultivariateSpec::Clustering { k_means_k: None, distance: ClusterDistance::Euclidean, linkage: ClusterLinkage::Ward, k_sweep_max: None };
    let mut an = AnalysisSpec::new(SheetId::new("res-clust-sel"), ds.id.clone(), AnalysisKind::Multivariate(clust_spec.clone()))?;
    an.selected_data_sets = vec![0, 2];
    let clust_res = run_multivariate_analysis(&ds, &an, &clust_spec)?;

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(clust_res)?;

    let mut g = GraphSheet::new_uninitialized_for_data(SheetId::new("g-hm-sel"), &ds)?;
    g.initialized = true;
    g.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    let scene = compile_graph(&g, &doc)?;
    let texts = scene_texts(&scene);
    assert!(texts.iter().any(|t| t == "FeatA"), "missing FeatA: {texts:?}");
    assert!(texts.iter().any(|t| t == "FeatC"), "missing FeatC: {texts:?}");
    assert!(!texts.iter().any(|t| t == "UnselectedMissing"), "unselected missing col appeared: {texts:?}");
    assert!(!texts.iter().any(|t| t == "UnselectedD"), "unselected col appeared: {texts:?}");

    // Because UnselectedMissing was excluded, dendrogram branches attach cleanly.
    let dendro_segments = scene
        .primitives
        .iter()
        .filter(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None } if path.commands.len() == 4))
        .count();
    assert!(dendro_segments >= 3, "expected dendrogram merges across selected cols, got {dendro_segments}");
    Ok(())
}

#[test]
fn r2_s08_06_preflight_checks_clipped_text_and_sub_5pt_scripts() {
    // 1. Clean 10 pt with Superscript (effective 7.0 pt >= 5.0 pt) well within canvas -> 0 issues.
    let mut clean = ChartScene::new("clean", "Clean", 300.0, 200.0, Some(Rgba8::WHITE));
    clean.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::new(vec![
            RichSpan::plain("10"),
            RichSpan {
                text: String::from("-3"),
                script: SpanScript::Superscript,
                weight: SpanWeight::Regular,
                slope: SpanSlope::Upright,
                color: None,
                font_family: None,
                scale: 0.7,
            },
        ]),
        anchor_pt: PointPt::new(120.0, 80.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 10.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    assert!(preflight_check_scene(&clean).is_empty(), "clean scene issues: {:?}", preflight_check_scene(&clean));

    // 2. Anchor inside canvas (285, 80) yet long text extends past right edge (300 pt) -> `text_outside_canvas`.
    let mut canvas_overflow = clean.clone();
    canvas_overflow.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Long overflow label past right canvas edge"),
        anchor_pt: PointPt::new(285.0, 80.0),
        anchor: TextAnchor::Start,
        font_size_pt: 10.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let overflow_issues = preflight_check_scene(&canvas_overflow);
    assert!(overflow_issues.iter().any(|i| i.code == "text_outside_canvas"), "expected text_outside_canvas, got {overflow_issues:?}");

    // 3. Text overflowing an active PushClip box -> `text_outside_canvas`.
    let mut box_clipped = clean.clone();
    box_clipped.primitives.push(ScenePrimitive::PushClip { rect: RectPt::new(40.0, 40.0, 30.0, 20.0) });
    box_clipped.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Clipped inside tight box"),
        anchor_pt: PointPt::new(44.0, 54.0),
        anchor: TextAnchor::Start,
        font_size_pt: 10.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    box_clipped.primitives.push(ScenePrimitive::PopClip);
    let clip_issues = preflight_check_scene(&box_clipped);
    assert!(clip_issues.iter().any(|i| i.code == "text_outside_canvas"), "expected clipped run issue, got {clip_issues:?}");

    // 4. Scaled/scripted span < 5.0 pt (base 6.0 pt * script 0.7 = 4.2 pt, or explicit scale 0.4 * 10 = 4.0 pt) -> `min_font_size`.
    let mut small_script = clean;
    small_script.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::new(vec![
            RichSpan::plain("EC"),
            RichSpan {
                text: String::from("50"),
                script: SpanScript::Subscript,
                weight: SpanWeight::Regular,
                slope: SpanSlope::Upright,
                color: None,
                font_family: None,
                scale: 1.0, // effective 0.7 * 6.0 = 4.2 pt
            },
        ]),
        anchor_pt: PointPt::new(120.0, 120.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 6.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let font_issues = preflight_check_scene(&small_script);
    assert!(font_issues.iter().any(|i| i.code == "min_font_size"), "expected min_font_size for 4.2pt subscript, got {font_issues:?}");
}

#[test]
fn r2_s11_16_equation_box_single_axis_overflow_pushes_clip() {
    let eq = r"$Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{1 + 10^{X}}$";

    // Auto-sized (`0 x 0`): must NOT push clip.
    let mut sc_auto = ChartScene::new("auto", "Auto", 400.0, 260.0, Some(Rgba8::WHITE));
    render_tex_equation_box(&mut sc_auto, DocRectPt::new(30.0, 30.0, 0.0, 0.0), eq, 10.0, true);
    assert!(!sc_auto.primitives.iter().any(|p| matches!(p, ScenePrimitive::PushClip { .. })), "0x0 auto box should not clip");

    // Width-constrained (`44 x 0`): fraction wider than 44 pt must push PushClip + PopClip.
    let mut sc_x_clip = ChartScene::new("xclip", "XClip", 400.0, 260.0, Some(Rgba8::WHITE));
    render_tex_equation_box(&mut sc_x_clip, DocRectPt::new(30.0, 30.0, 44.0, 0.0), eq, 10.0, true);
    let x_pushes = sc_x_clip.primitives.iter().filter(|p| matches!(p, ScenePrimitive::PushClip { .. })).count();
    let x_pops = sc_x_clip.primitives.iter().filter(|p| matches!(p, ScenePrimitive::PopClip)).count();
    assert!(x_pushes >= 1 && x_pushes == x_pops, "expected balanced PushClip/PopClip for narrow w=44 x 0 box");

    // Height-constrained (`0 x 14`): stacked fraction taller than 14 pt must push PushClip + PopClip.
    let mut sc_y_clip = ChartScene::new("yclip", "YClip", 400.0, 260.0, Some(Rgba8::WHITE));
    render_tex_equation_box(&mut sc_y_clip, DocRectPt::new(30.0, 30.0, 0.0, 14.0), eq, 10.0, true);
    let y_pushes = sc_y_clip.primitives.iter().filter(|p| matches!(p, ScenePrimitive::PushClip { .. })).count();
    let y_pops = sc_y_clip.primitives.iter().filter(|p| matches!(p, ScenePrimitive::PopClip)).count();
    assert!(y_pushes >= 1 && y_pushes == y_pops, "expected balanced PushClip/PopClip for shallow 0 x h=14 box");
}
