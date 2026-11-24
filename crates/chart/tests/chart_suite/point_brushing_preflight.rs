//! Integration tests for QOL-5 `ChartScene::plotted_points` provenance & `hit_test_point`
//! brushing across XY, Column, Grouped, Nested, Survival, Multivariate, and Layout scenes,
//! plus `preflight_check_scene` publication QA checks.

use curvebook_chart::{
    ChartScene, FillPaint, Path2D, PointPt, PositionedText, RectPt, ScenePrimitive, SourceCellId, StrokePaint, TextAnchor, compile_graph,
    compile_layout, preflight_check_scene,
};
use curvebook_doc::{
    Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, DataSheet, ErrorBarMetric, ErrorBarSpec, GraphSheet, GraphicalDataObject, GroupedTable,
    JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode, NestedTable,
    ProjectDocument, RectPt as DocRectPt, ReplicateOverlayStyle, SheetId, SizeChannelMapping, SubcolumnMode, SummaryMeasure, SurvivalTable,
    TableArchetype, WhiskerMethod, XyTable,
};
use curvebook_palette::{MarkerShape, MarkerStyle, Rgba8};
use curvebook_text::RichText;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn xy_raw_replicates_and_summarized_hit_test() -> TestResult {
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 3)?;
    xy.data_sets[0].title = String::from("DoseResponse");
    for (r, (x, y0, y1)) in [(1.0, 10.0, 14.0), (2.0, 22.0, 26.0), (3.0, 35.0, 41.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y0);
        xy.data_sets[0].cells[r][1] = Cell::number(y1);
    }
    let ds = DataSheet::new(SheetId::new("xy-raw"), "XY Raw", TableArchetype::Xy(xy))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-xy-raw"), &ds)?;
    gs.spec.series[0].y_error_bars = None;
    let raw_scene = compile_graph(&gs, &doc)?;
    assert_eq!(raw_scene.plotted_points.len(), 6);

    let target = raw_scene
        .plotted_points
        .iter()
        .find(|p| p.source_cell == SourceCellId { sheet_id: SheetId::new("xy-raw"), row: 1, col: 0, subcol: 1 })
        .ok_or("missing raw replicate provenance (row 1, col 0, subcol 1)")?;
    assert!(target.value_summary.contains("X=") && target.value_summary.contains("Y="));

    let hit = raw_scene.hit_test_point(PointPt::new(target.center_pt.x + 1.5, target.center_pt.y - 1.5), 3.0).ok_or("expected hit near replicate")?;
    assert_eq!(hit.source_cell, target.source_cell);
    assert_eq!(hit.series_label, "DoseResponse");
    assert!(raw_scene.hit_test_point(PointPt::new(0.5, 0.5), 0.5).is_none());

    // Enabling Y error bars collapses markers to summarized centers (`subcol: 0`).
    gs.spec.series[0].y_error_bars = Some(ErrorBarSpec { metric: ErrorBarMetric::Sd, ..ErrorBarSpec::default() });
    let sum_scene = compile_graph(&gs, &doc)?;
    assert_eq!(sum_scene.plotted_points.len(), 3);
    assert!(sum_scene.plotted_points.iter().all(|p| p.source_cell.subcol == 0));
    let sum_target = sum_scene.plotted_points.iter().find(|p| p.source_cell.row == 2).ok_or("missing summarized XY point row 2")?;
    let sum_hit = sum_scene.hit_test_point(sum_target.center_pt, 2.0).ok_or("expected summarized XY hit")?;
    assert_eq!(sum_hit.source_cell, SourceCellId { sheet_id: SheetId::new("xy-raw"), row: 2, col: 0, subcol: 0 });
    Ok(())
}

#[test]
fn column_grouped_nested_provenance_and_hit_test() -> TestResult {
    let mut doc = ProjectDocument::default();

    // 1. Column Before/After paired connectors
    let mut col_ba = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 3)?;
    col_ba.data_sets[0].title = String::from("Baseline");
    col_ba.data_sets[1].title = String::from("Followup");
    for (r, (v0, v1)) in [(10.0, 16.0), (12.0, 20.0), (15.0, 24.0)].into_iter().enumerate() {
        col_ba.data_sets[0].cells[r][0] = Cell::number(v0);
        col_ba.data_sets[1].cells[r][0] = Cell::number(v1);
    }
    let ds_ba = DataSheet::new(SheetId::new("col-ba"), "Paired Col", TableArchetype::Column(col_ba))?;
    doc.add_data_sheet(ds_ba.clone())?;
    let mut g_ba = GraphSheet::new_uninitialized_for_data(SheetId::new("g-ba"), &ds_ba)?;
    let ba_gdo =
        GraphicalDataObject::BeforeAfterLines { marker: Some(MarkerStyle::default()), line_color: Rgba8::rgb(40, 80, 160), stroke_width_pt: 1.2 };
    g_ba.spec.series[0].gdo = ba_gdo.clone();
    g_ba.spec.series[1].gdo = ba_gdo;
    let ba_scene = compile_graph(&g_ba, &doc)?;
    assert_eq!(ba_scene.plotted_points.len(), 6);
    let ba_target = ba_scene
        .plotted_points
        .iter()
        .find(|p| p.source_cell == SourceCellId { sheet_id: SheetId::new("col-ba"), row: 2, col: 1, subcol: 0 })
        .ok_or("missing Before/After provenance")?;
    assert_eq!(ba_scene.hit_test_point(ba_target.center_pt, 2.0).map(|m| &m.source_cell), Some(&ba_target.source_cell));

    // 2. Column BoxAndWhisker outlier provenance
    let mut col_box = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 8)?;
    col_box.data_sets[0].title = String::from("CohortA");
    for (r, v) in [10.0, 11.0, 11.4, 12.0, 12.2, 12.6, 13.0, 38.0].into_iter().enumerate() {
        col_box.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let ds_box = DataSheet::new(SheetId::new("col-box"), "Box Outliers", TableArchetype::Column(col_box))?;
    doc.add_data_sheet(ds_box.clone())?;
    let mut g_box = GraphSheet::new_uninitialized_for_data(SheetId::new("g-box"), &ds_box)?;
    g_box.spec.series[0].gdo = GraphicalDataObject::BoxAndWhisker {
        whiskers: WhiskerMethod::Tukey1_5Iqr,
        fill_color: Rgba8::rgba(70, 130, 180, 160),
        stroke_color: Rgba8::BLACK,
        show_outliers: true,
        show_all_points: false,
    };
    let box_scene = compile_graph(&g_box, &doc)?;
    assert_eq!(box_scene.plotted_points.len(), 1);
    assert_eq!(box_scene.plotted_points[0].source_cell, SourceCellId { sheet_id: SheetId::new("col-box"), row: 7, col: 0, subcol: 0 });
    let box_hit = box_scene.hit_test_point(box_scene.plotted_points[0].center_pt, 2.0).ok_or("expected outlier dot hit")?;
    assert_eq!(box_hit.source_cell.row, 7);

    // 3. Grouped table with replicate beeswarm dots (`row`, `col`, `subcol`)
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2)?;
    grp.row_titles = vec![String::from("Low"), String::from("High")];
    grp.data_sets[0].title = String::from("Vehicle");
    grp.data_sets[1].title = String::from("Compound");
    for r in 0..2 {
        for c in 0..2 {
            grp.data_sets[c].cells[r][0] = Cell::number(10.0 + (r * 5 + c * 3) as f64);
            grp.data_sets[c].cells[r][1] = Cell::number(12.0 + (r * 5 + c * 3) as f64);
        }
    }
    let ds_grp = DataSheet::new(SheetId::new("grp-raw"), "Grouped", TableArchetype::Grouped(grp))?;
    doc.add_data_sheet(ds_grp.clone())?;
    let mut g_grp = GraphSheet::new_uninitialized_for_data(SheetId::new("g-grp"), &ds_grp)?;
    for s in &mut g_grp.spec.series {
        s.gdo = GraphicalDataObject::BarWithReplicates {
            summary: SummaryMeasure::Mean,
            fill_color: Rgba8::rgba(100, 140, 200, 180),
            border_color: Rgba8::BLACK,
            border_width_pt: 1.0,
            replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
        };
    }
    let grp_scene = compile_graph(&g_grp, &doc)?;
    assert_eq!(grp_scene.plotted_points.len(), 8);
    let grp_target = grp_scene
        .plotted_points
        .iter()
        .find(|p| p.source_cell == SourceCellId { sheet_id: SheetId::new("grp-raw"), row: 1, col: 1, subcol: 1 })
        .ok_or("missing grouped cell provenance")?;
    assert_eq!(grp_scene.hit_test_point(grp_target.center_pt, 2.0).map(|m| &m.source_cell), Some(&grp_target.source_cell));

    // 4. Nested table (`col` = group, `subcol` = biological unit, `row` = technical replicate)
    let mut nested = NestedTable::new(2, 2, 3)?;
    nested.groups[0].title = String::from("WT");
    nested.groups[1].title = String::from("KO");
    for g in 0..2 {
        for b in 0..2 {
            for r in 0..3 {
                nested.groups[g].cells[r][b] = Cell::number(5.0 + (g * 10 + b * 3 + r) as f64);
            }
        }
    }
    let ds_nest = DataSheet::new(SheetId::new("nest"), "Nested", TableArchetype::Nested(nested))?;
    doc.add_data_sheet(ds_nest.clone())?;
    let mut g_nest = GraphSheet::new_uninitialized_for_data(SheetId::new("g-nest"), &ds_nest)?;
    for s in &mut g_nest.spec.series {
        s.gdo = GraphicalDataObject::BoxAndWhisker {
            whiskers: WhiskerMethod::MinToMax,
            fill_color: Rgba8::rgba(180, 120, 80, 160),
            stroke_color: Rgba8::BLACK,
            show_outliers: false,
            show_all_points: true,
        };
    }
    let nest_scene = compile_graph(&g_nest, &doc)?;
    assert_eq!(nest_scene.plotted_points.len(), 12);
    let nest_target = nest_scene
        .plotted_points
        .iter()
        .find(|p| p.source_cell == SourceCellId { sheet_id: SheetId::new("nest"), row: 2, col: 1, subcol: 1 })
        .ok_or("missing nested technical replicate provenance")?;
    assert_eq!(nest_scene.hit_test_point(nest_target.center_pt, 2.0).map(|m| &m.source_cell), Some(&nest_target.source_cell));
    Ok(())
}

#[test]
fn survival_censored_and_event_hit_test() -> TestResult {
    let mut surv = SurvivalTable::new(2, 6)?;
    surv.cohorts[0].title = String::from("Control");
    surv.cohorts[1].title = String::from("Treated");
    let rows = [(2.0, 1, 0), (4.0, 0, 1), (6.0, 1, 0), (9.0, 1, 1), (12.0, 0, 1), (15.0, 1, 0)];
    for (r, (t, c0, c1)) in rows.into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(c0 as f64);
        surv.cohorts[1].cells[r][0] = Cell::number(c1 as f64);
    }
    let ds = DataSheet::new(SheetId::new("surv"), "Survival", TableArchetype::Survival(surv))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-surv"), &ds)?;
    for s in &mut gs.spec.series {
        if let GraphicalDataObject::KaplanMeierStep { show_censored_ticks, .. } = &mut s.gdo {
            *show_censored_ticks = true;
        }
    }
    let scene = compile_graph(&gs, &doc)?;
    assert_eq!(scene.plotted_points.len(), 12);

    // Censored observation at row 1 (`t=4.0`) in cohort 0 and row 0 (`t=2.0`) in cohort 1.
    let cens_c0 = scene
        .plotted_points
        .iter()
        .find(|p| p.source_cell == SourceCellId { sheet_id: SheetId::new("surv"), row: 1, col: 0, subcol: 0 })
        .ok_or("missing cohort 0 censored tick provenance")?;
    let event_c1 = scene
        .plotted_points
        .iter()
        .find(|p| p.source_cell == SourceCellId { sheet_id: SheetId::new("surv"), row: 3, col: 1, subcol: 0 })
        .ok_or("missing cohort 1 event provenance")?;
    assert_eq!(scene.hit_test_point(cens_c0.center_pt, 2.0).map(|m| &m.source_cell), Some(&cens_c0.source_cell));
    assert_eq!(scene.hit_test_point(event_c1.center_pt, 2.0).map(|m| &m.source_cell), Some(&event_c1.source_cell));
    Ok(())
}

#[test]
fn multivariate_bubble_scatter_and_layout_hit_test() -> TestResult {
    let mut doc = ProjectDocument::default();

    // Multivariate bubble scatter sheet (`x_col=0`, `y_col=1`, `size_col=2`)
    let mut mv = MultipleVariablesTable::new(3, 4)?;
    mv.variables[0].title = String::from("Dose");
    mv.variables[1].title = String::from("Effect");
    mv.variables[2].title = String::from("Weight");
    for (r, (x, y, w)) in [(1.0, 10.0, 4.0), (3.0, 25.0, 9.0), (6.0, 48.0, 16.0), (9.0, 72.0, 25.0)].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(y);
        mv.variables[2].cells[r] = Cell::number(w);
    }
    let ds_mv = DataSheet::new(SheetId::new("mv"), "MV", TableArchetype::MultipleVariables(mv))?;
    doc.add_data_sheet(ds_mv.clone())?;
    let mut g_mv = GraphSheet::new_uninitialized_for_data(SheetId::new("g-mv"), &ds_mv)?;
    g_mv.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Constant(Rgba8::rgb(40, 120, 200)),
        size: SizeChannelMapping::Variable { column: 2, min_radius_pt: 3.0, max_radius_pt: 10.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    doc.add_graph_sheet(g_mv.clone())?;
    let mv_scene = compile_graph(&g_mv, &doc)?;
    assert_eq!(mv_scene.plotted_points.len(), 4);
    let mv_target = mv_scene
        .plotted_points
        .iter()
        .find(|p| p.source_cell == SourceCellId { sheet_id: SheetId::new("mv"), row: 2, col: 1, subcol: 0 })
        .ok_or("missing bubble row 2 provenance")?;
    assert!(mv_target.radius_pt >= 3.0);
    assert_eq!(
        mv_scene.hit_test_point(PointPt::new(mv_target.center_pt.x + 2.0, mv_target.center_pt.y), 1.0).map(|m| &m.source_cell),
        Some(&mv_target.source_cell)
    );

    // Second graph (XY) combined into a two-panel publication Layout
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3)?;
    for r in 0..3 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(((r + 1) * 15) as f64);
    }
    let ds_xy = DataSheet::new(SheetId::new("xy-lay"), "XY Panel", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(ds_xy.clone())?;
    let mut g_xy = GraphSheet::new_uninitialized_for_data(SheetId::new("g-xy"), &ds_xy)?;
    g_xy.spec.series[0].y_error_bars = None;
    if let GraphicalDataObject::ScatterAndLine { marker, .. } = &mut g_xy.spec.series[0].gdo {
        *marker = Some(MarkerStyle { shape: MarkerShape::Circle, ..MarkerStyle::default() });
    }
    doc.add_graph_sheet(g_xy)?;

    let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    lspec.align_plot_origins = false;
    lspec.items = vec![
        LayoutItem::LiveGraph {
            id: String::from("p1"),
            graph_sheet_id: SheetId::new("g-mv"),
            bounds_pt: DocRectPt::new(15.0, 20.0, 200.0, 140.0),
            scale_percent: 100.0,
            badge_override: None,
        },
        LayoutItem::LiveGraph {
            id: String::from("p2"),
            graph_sheet_id: SheetId::new("g-xy"),
            bounds_pt: DocRectPt::new(250.0, 20.0, 200.0, 140.0),
            scale_percent: 100.0,
            badge_override: None,
        },
    ];
    let layout = LayoutSheet::new(SheetId::new("lay"), "Fig", lspec)?;
    doc.add_layout_sheet(layout.clone())?;

    let page = compile_layout(&layout, &doc)?;
    assert_eq!(page.plotted_points.len(), 7);

    let p1_pt = page
        .plotted_points
        .iter()
        .find(|p| p.source_cell.sheet_id == SheetId::new("mv") && p.source_cell.row == 2)
        .ok_or("missing layout panel 1 point")?;
    assert!(p1_pt.center_pt.x >= 15.0 && p1_pt.center_pt.x <= 215.0);
    assert!(p1_pt.radius_pt >= 2.0);
    assert_eq!(page.hit_test_point(p1_pt.center_pt, 2.0).map(|m| &m.source_cell), Some(&p1_pt.source_cell));

    let p2_pt = page
        .plotted_points
        .iter()
        .find(|p| p.source_cell.sheet_id == SheetId::new("xy-lay") && p.source_cell.row == 1)
        .ok_or("missing layout panel 2 point")?;
    assert!(p2_pt.center_pt.x >= 250.0 && p2_pt.center_pt.x <= 450.0);
    assert_eq!(page.hit_test_point(p2_pt.center_pt, 2.0).map(|m| &m.source_cell), Some(&p2_pt.source_cell));
    Ok(())
}

#[test]
fn preflight_check_scene_detects_all_issues_and_passes_clean_figure() {
    let mut clean = ChartScene::new("preflight", "Preflight Figure", 300.0, 200.0, Some(Rgba8::WHITE));
    clean.plot_rect_pt = RectPt::new(40.0, 20.0, 220.0, 140.0);
    clean.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(40.0, 160.0), PointPt::new(260.0, 160.0)),
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: None,
    });
    clean.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Clean Title"),
        anchor_pt: PointPt::new(150.0, 15.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 9.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    assert!(preflight_check_scene(&clean).is_empty());

    let mut noisy = clean;
    noisy.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Tiny label"),
        anchor_pt: PointPt::new(100.0, 50.0),
        anchor: TextAnchor::Start,
        font_size_pt: 4.2,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    noisy.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(RectPt::new(50.0, 50.0, 20.0, 20.0)),
        fill: Some(FillPaint::solid(Rgba8::rgb(220, 220, 220))),
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 0.15)),
        clip: None,
    });
    noisy.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Clipped off-canvas"),
        anchor_pt: PointPt::new(-12.0, 80.0),
        anchor: TextAnchor::Start,
        font_size_pt: 8.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let issues = preflight_check_scene(&noisy);
    assert_eq!(issues.len(), 3);
    assert!(issues.iter().all(|i| i.severity == "warn"));
    assert!(issues.iter().any(|i| i.code == "min_font_size"));
    assert!(issues.iter().any(|i| i.code == "hairline_stroke"));
    assert!(issues.iter().any(|i| i.code == "text_outside_canvas"));
}
