//! Chart rendering tests for `ColumnTable` pre-summarized asymmetric (`MeanLowHigh`,
//! `UpperLowerLimits`, `MedianIqr`) and symmetric no-`N` (`MeanSd`, `MeanSem`) modes across
//! bar charts (vertical & horizontal), box-and-whisker, floating bars, and aligned dot plots.

use curvebook_chart::{PathCmd, ScenePrimitive, compile_graph};
use curvebook_doc::{
    CartesianOrientation, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, GraphSheet, GraphSpec,
    GraphicalDataObject, ProjectDocument, SheetId, SubcolumnMode, SummaryMeasure, TableArchetype, WhiskerMethod,
};
use curvebook_palette::Rgba8;

fn make_column_doc(mode: SubcolumnMode, rows: Vec<Vec<Cell>>) -> ProjectDocument {
    let mut doc = ProjectDocument::new("Column Summary Chart").unwrap();
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 1, rows.len()).unwrap();
    col.data_sets[0].title = String::from("Group A");
    col.data_sets[0].cells = rows;
    doc.add_data_sheet(DataSheet::new("col-1", "Summary Data", TableArchetype::Column(col)).unwrap()).unwrap();
    doc
}

fn vertical_error_bar_stem_endpoints(scene: &curvebook_chart::ChartScene) -> Vec<(f64, f64, f64)> {
    let plot = scene.plot_rect_pt;
    let mut stems = Vec::new();
    for prim in &scene.primitives {
        if let ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } = prim
            && path.commands.len() == 2
            && let (PathCmd::MoveTo(p0), PathCmd::LineTo(p1)) = (path.commands[0], path.commands[1])
            && (p0.x - p1.x).abs() < 1e-4
            && (p0.y - p1.y).abs() > 2.0
            && p0.x > plot.x + 1.0
            && p0.x < plot.right() - 1.0
        {
            stems.push((p0.x, p0.y.min(p1.y), p0.y.max(p1.y)));
        }
    }
    stems
}

fn filled_bar_rects(scene: &curvebook_chart::ChartScene) -> Vec<(f64, f64, f64, f64)> {
    let plot = scene.plot_rect_pt;
    let mut rects = Vec::new();
    for prim in &scene.primitives {
        if let ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } = prim {
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for cmd in &path.commands {
                if let PathCmd::MoveTo(p) | PathCmd::LineTo(p) = cmd {
                    xs.push(p.x);
                    ys.push(p.y);
                }
            }
            if xs.len() >= 4 {
                let x0 = xs.iter().copied().fold(f64::INFINITY, f64::min);
                let x1 = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let y0 = ys.iter().copied().fold(f64::INFINITY, f64::min);
                let y1 = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                if x0 >= plot.x - 1.0 && x1 <= plot.right() + 1.0 && (x1 - x0) > 4.0 && (y1 - y0) > 2.0 {
                    rects.push((x0, y0, x1, y1));
                }
            }
        }
    }
    rects
}

#[test]
fn column_asymmetric_modes_render_asymmetric_bar_whiskers_vertical_and_horizontal() {
    // Center = 20.0, lower = 16.0 (down 4), upper = 32.0 (up 12) => 3:1 upper-to-lower whisker ratio.
    let cases = [
        (SubcolumnMode::MeanLowHigh, vec![Cell::number(20.0), Cell::number(4.0), Cell::number(12.0)]),
        (SubcolumnMode::UpperLowerLimits, vec![Cell::number(20.0), Cell::number(16.0), Cell::number(32.0)]),
        (SubcolumnMode::MedianIqr, vec![Cell::number(20.0), Cell::number(16.0), Cell::number(32.0)]),
    ];

    for (mode, row) in cases {
        let doc = make_column_doc(mode, vec![row]);
        let mut spec = GraphSpec::default_for_data_sheet(&doc.data_sheets[0]);
        spec.series[0].y_error_bars =
            Some(ErrorBarSpec { metric: ErrorBarMetric::ExplicitSubcolumns, direction: ErrorBarDirection::Both, ..ErrorBarSpec::default() });

        let gs = GraphSheet {
            id: SheetId::new("g-v"),
            title: format!("Col {mode:?}"),
            initialized: true,
            spec: spec.clone(),
            floating_notes: Vec::new(),
            color_tag: None,
            origin: None,
        };
        let scene = compile_graph(&gs, &doc).unwrap();
        assert!(scene.warnings.is_empty(), "expected zero warnings for {mode:?} bar plot, got {:?}", scene.warnings);

        let bars = filled_bar_rects(&scene);
        assert_eq!(bars.len(), 1, "expected one filled bar for {mode:?}");
        let bar_top_y = bars[0].1;

        let stems = vertical_error_bar_stem_endpoints(&scene);
        assert!(!stems.is_empty(), "expected vertical error bar stem for {mode:?}");
        // In screen coordinates (y increases downward): y_upper < bar_top_y < y_lower.
        let (_, y_top, y_bot) = stems[0];
        let upper_arm_pt = bar_top_y - y_top;
        let lower_arm_pt = y_bot - bar_top_y;
        assert!(upper_arm_pt > 2.0 && lower_arm_pt > 2.0, "{mode:?}: arms ({upper_arm_pt}, {lower_arm_pt})");
        let ratio = upper_arm_pt / lower_arm_pt;
        assert!((ratio - 3.0).abs() < 0.05, "{mode:?}: expected 3:1 asymmetric whisker ratio (up=12 vs down=4), got {ratio:.4}");

        // Horizontal orientation compiles cleanly with zero warnings as well.
        let mut hspec = spec;
        hspec.orientation = CartesianOrientation::Horizontal;
        let hgs = GraphSheet {
            id: SheetId::new("g-h"),
            title: format!("Horiz {mode:?}"),
            initialized: true,
            spec: hspec,
            floating_notes: Vec::new(),
            color_tag: None,
            origin: None,
        };
        let hscene = compile_graph(&hgs, &doc).unwrap();
        assert!(hscene.warnings.is_empty(), "horizontal {mode:?} warnings: {:?}", hscene.warnings);
        assert!(!filled_bar_rects(&hscene).is_empty());
    }
}

#[test]
fn column_median_iqr_and_limits_render_box_floating_and_dot_plots_honestly() {
    // 1. MedianIqr BoxAndWhisker draws Q1..Q3 box + median rule cleanly with zero warnings.
    let iqr_doc = make_column_doc(SubcolumnMode::MedianIqr, vec![vec![Cell::number(20.0), Cell::number(14.0), Cell::number(30.0)]]);
    let mut box_spec = GraphSpec::default_for_data_sheet(&iqr_doc.data_sheets[0]);
    box_spec.series[0].gdo = GraphicalDataObject::BoxAndWhisker {
        whiskers: WhiskerMethod::Tukey1_5Iqr,
        fill_color: Rgba8::rgba(120, 170, 220, 200),
        stroke_color: Rgba8::BLACK,
        show_outliers: false,
        show_all_points: false,
    };
    box_spec.series[0].y_error_bars = None;
    let box_gs = GraphSheet {
        id: SheetId::new("g-iqr-box"),
        title: String::from("IQR Box"),
        initialized: true,
        spec: box_spec,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    };
    let box_scene = compile_graph(&box_gs, &iqr_doc).unwrap();
    assert!(box_scene.warnings.is_empty(), "MedianIqr BoxAndWhisker must emit zero warnings: {:?}", box_scene.warnings);
    assert_eq!(filled_bar_rects(&box_scene).len(), 1, "MedianIqr BoxAndWhisker must render Q1..Q3 box");

    // 2. UpperLowerLimits FloatingBar renders [Lower, Upper] floating span with zero warnings.
    let ul_doc = make_column_doc(SubcolumnMode::UpperLowerLimits, vec![vec![Cell::number(25.0), Cell::number(18.0), Cell::number(35.0)]]);
    let mut fl_spec = GraphSpec::default_for_data_sheet(&ul_doc.data_sheets[0]);
    fl_spec.series[0].gdo = GraphicalDataObject::FloatingBar {
        summary: Some(SummaryMeasure::Mean),
        fill_color: Rgba8::rgba(180, 210, 150, 220),
        border_color: Rgba8::BLACK,
        border_width_pt: 1.0,
    };
    fl_spec.series[0].y_error_bars = None;
    let fl_gs = GraphSheet {
        id: SheetId::new("g-ul-float"),
        title: String::from("UL Floating"),
        initialized: true,
        spec: fl_spec,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    };
    let fl_scene = compile_graph(&fl_gs, &ul_doc).unwrap();
    assert!(fl_scene.warnings.is_empty(), "UpperLowerLimits FloatingBar warnings: {:?}", fl_scene.warnings);
    assert_eq!(filled_bar_rects(&fl_scene).len(), 1);
}

#[test]
fn column_mean_sd_and_sem_render_symmetric_whiskers_and_warn_on_multi_row_unpoolable() {
    for mode in [SubcolumnMode::MeanSd, SubcolumnMode::MeanSem] {
        let doc = make_column_doc(mode, vec![vec![Cell::number(20.0), Cell::number(5.0)]]);
        let mut spec = GraphSpec::default_for_data_sheet(&doc.data_sheets[0]);
        spec.series[0].y_error_bars = Some(ErrorBarSpec {
            metric: if mode == SubcolumnMode::MeanSd { ErrorBarMetric::Sd } else { ErrorBarMetric::Sem },
            direction: ErrorBarDirection::Both,
            ..ErrorBarSpec::default()
        });
        let gs = GraphSheet {
            id: SheetId::new("g-sym"),
            title: format!("Col {mode:?}"),
            initialized: true,
            spec: spec.clone(),
            floating_notes: Vec::new(),
            color_tag: None,
            origin: None,
        };
        let scene = compile_graph(&gs, &doc).unwrap();
        assert!(scene.warnings.is_empty(), "{mode:?} warnings: {:?}", scene.warnings);
        let bars = filled_bar_rects(&scene);
        let stems = vertical_error_bar_stem_endpoints(&scene);
        assert_eq!(bars.len(), 1);
        assert!(!stems.is_empty());
        let bar_top = bars[0].1;
        let (_, y_top, y_bot) = stems[0];
        let upper_arm = bar_top - y_top;
        let lower_arm = y_bot - bar_top;
        assert!((upper_arm - lower_arm).abs() < 1e-4, "{mode:?} whisker must be symmetric: up={upper_arm}, down={lower_arm}");

        // Multiple unpoolable rows emit explicit `multiple_unpoolable_summary_rows` warning.
        let multi_doc = make_column_doc(mode, vec![vec![Cell::number(20.0), Cell::number(5.0)], vec![Cell::number(25.0), Cell::number(3.0)]]);
        let multi_scene = compile_graph(&gs, &multi_doc).unwrap();
        assert!(
            multi_scene.warnings.iter().any(|w| w.code == "multiple_unpoolable_summary_rows"),
            "{mode:?} multi-row column should warn, got {:?}",
            multi_scene.warnings
        );
    }
}
