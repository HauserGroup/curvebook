//! Regression tests for Stage 3B `R1-S08-01` (log-axis tick generation & domain overflow/underflow).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc;
use std::time::Duration;

use curvebook_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph};
use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, GraphSpec, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, TickLabelFormat,
    XyTable,
};
use curvebook_math::AxisScale;

fn xy_doc(rows: &[(f64, f64)]) -> (ProjectDocument, GraphSheet) {
    let mut doc = ProjectDocument::new("S3B-XY").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, rows.len()).unwrap();
    for (r, &(x, y)) in rows.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    doc.add_data_sheet(DataSheet::new("d", "D", TableArchetype::Xy(xy)).unwrap()).unwrap();
    let ds = doc.data_sheet(&SheetId::new("d")).unwrap();
    let mut g = GraphSheet::new_uninitialized_for_data("g", ds).unwrap();
    g.initialized = true;
    g.spec = GraphSpec::default_for_data_sheet(ds);
    (doc, g)
}

fn column_doc(values: &[f64]) -> (ProjectDocument, GraphSheet) {
    let mut doc = ProjectDocument::new("S3B-COL").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, values.len()).unwrap();
    for (r, &v) in values.iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(v);
    }
    doc.add_data_sheet(DataSheet::new("d", "D", TableArchetype::Column(col)).unwrap()).unwrap();
    let ds = doc.data_sheet(&SheetId::new("d")).unwrap();
    let mut g = GraphSheet::new_uninitialized_for_data("g", ds).unwrap();
    g.initialized = true;
    g.spec = GraphSpec::default_for_data_sheet(ds);
    (doc, g)
}

fn compile_guarded(doc: ProjectDocument, graph: GraphSheet) -> ChartScene {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compile_graph(&graph, &doc)));
        let _ = tx.send(res);
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Err(_) => panic!("compile_graph hung beyond 10s"),
        Ok(Err(_)) => panic!("compile_graph panicked"),
        Ok(Ok(res)) => res.expect("compile_graph returned Err"),
    }
}

fn assert_all_primitives_finite(scene: &ChartScene) {
    for prim in &scene.primitives {
        match prim {
            ScenePrimitive::Path { path, clip, .. } => {
                for cmd in &path.commands {
                    match *cmd {
                        PathCmd::MoveTo(p) | PathCmd::LineTo(p) => {
                            assert!(p.x.is_finite() && p.y.is_finite(), "non-finite path point: {p:?}");
                        }
                        PathCmd::QuadTo { control, to } => {
                            assert!(control.x.is_finite() && control.y.is_finite() && to.x.is_finite() && to.y.is_finite());
                        }
                        PathCmd::CubicTo { c1, c2, to } => {
                            assert!(
                                c1.x.is_finite() && c1.y.is_finite() && c2.x.is_finite() && c2.y.is_finite() && to.x.is_finite() && to.y.is_finite()
                            );
                        }
                        PathCmd::Close => {}
                    }
                }
                if let Some(c) = clip {
                    assert!(c.x.is_finite() && c.y.is_finite() && c.width.is_finite() && c.height.is_finite());
                }
            }
            ScenePrimitive::Text(t) => {
                assert!(t.anchor_pt.x.is_finite() && t.anchor_pt.y.is_finite() && t.font_size_pt.is_finite());
            }
            ScenePrimitive::PushClip { rect } => {
                assert!(rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite());
            }
            ScenePrimitive::PopClip => {}
        }
    }
}

#[test]
fn r1_s08_01_log_axis_near_f64_max_does_not_panic_or_hang() {
    for scale in [AxisScale::Log10, AxisScale::Log2, AxisScale::Ln] {
        for y_max in [1.5e308, f64::MAX] {
            let (doc, mut g) = xy_doc(&[(1.0, 1.0), (2.0, y_max)]);
            g.spec.y_axis.segments[0].scale = scale;
            let scene = compile_guarded(doc, g);
            assert_all_primitives_finite(&scene);
            assert!(scene.primitives.len() < 1000, "unexpectedly large scene for {scale:?}: {}", scene.primitives.len());
            assert_eq!(scene.plotted_points.len(), 2, "both finite positive points should be plotted on {scale:?}");
        }
    }
}

#[test]
fn r1_s08_01_log_axis_tiny_subnormal_values_preserve_points_and_finish() {
    let subnormal_cases = [[(1.0, 5e-324), (2.0, 1.0)], [(1.0, 5e-324), (2.0, 1e-320)], [(1.0, f64::MIN_POSITIVE), (2.0, 1.0)]];
    for scale in [AxisScale::Log10, AxisScale::Log2, AxisScale::Ln] {
        for rows in subnormal_cases {
            let (doc, mut g) = xy_doc(&rows);
            g.spec.y_axis.segments[0].scale = scale;
            let scene = compile_guarded(doc, g);
            assert_all_primitives_finite(&scene);
            assert!(scene.primitives.len() < 1000, "unexpectedly large scene for {scale:?}: {}", scene.primitives.len());
            assert_eq!(scene.plotted_points.len(), 2, "subnormal range {rows:?} dropped points on {scale:?}");
        }
    }
}

#[test]
fn r1_s08_01_log_axis_full_f64_span_on_x_and_y() {
    for scale in [AxisScale::Log10, AxisScale::Log2, AxisScale::Ln] {
        let (doc, mut g) = xy_doc(&[(5e-324, 5e-324), (f64::MAX, f64::MAX)]);
        g.spec.x_axis.segments[0].scale = scale;
        g.spec.y_axis.segments[0].scale = scale;
        let scene = compile_guarded(doc, g);
        assert_all_primitives_finite(&scene);
        assert!(scene.primitives.len() < 1000);
        assert_eq!(scene.plotted_points.len(), 2);
    }
}

#[test]
fn r1_s08_01_single_point_and_explicit_extreme_bounds_and_formats() {
    for scale in [AxisScale::Log10, AxisScale::Log2, AxisScale::Ln] {
        for single in [f64::MAX, 1.0e308, 2.0_f64.powi(1023), 5e-324] {
            let (doc, mut g) = xy_doc(&[(single, single)]);
            g.spec.x_axis.segments[0].scale = scale;
            g.spec.y_axis.segments[0].scale = scale;
            let scene = compile_guarded(doc, g);
            assert_all_primitives_finite(&scene);
            assert!(scene.primitives.len() < 1000);
            assert_eq!(scene.plotted_points.len(), 1);
        }
    }

    let explicit_pairs = [
        (Some(5e-324), Some(f64::MAX)),
        (Some(1e307), Some(f64::MAX)),
        (Some(1e308), Some(1e308)),
        (Some(5e-324), Some(5e-324)),
        (Some(0.0), Some(1.5e308)),
        (Some(f64::NAN), Some(f64::INFINITY)),
    ];
    let formats = [
        TickLabelFormat::Decimal,
        TickLabelFormat::PowerOfTenSuperscript,
        TickLabelFormat::ScientificE,
        TickLabelFormat::Antilog,
        TickLabelFormat::EngineeringSi,
    ];
    for (min_b, max_b) in explicit_pairs {
        for fmt in formats {
            let (doc, mut g) = xy_doc(&[(1.0, 1.0), (2.0, 100.0)]);
            g.spec.y_axis.segments[0].scale = AxisScale::Log10;
            g.spec.y_axis.segments[0].min = min_b;
            g.spec.y_axis.segments[0].max = max_b;
            g.spec.y_axis.tick_format = fmt;
            let scene = compile_guarded(doc, g);
            assert_all_primitives_finite(&scene);
            assert!(scene.primitives.len() < 1000);
        }
    }
}

#[test]
fn r1_s08_01_mixed_invalid_cells_and_column_log_axis() {
    let (doc, mut g) = xy_doc(&[(1.0, 0.0), (2.0, -5.0), (3.0, f64::NAN), (4.0, f64::INFINITY), (5.0, 1.5e308)]);
    g.spec.y_axis.segments[0].scale = AxisScale::Log10;
    let scene = compile_guarded(doc, g);
    assert_all_primitives_finite(&scene);
    assert!(scene.primitives.len() < 1000);
    assert_eq!(scene.plotted_points.len(), 1);

    for values in [[1.0, 1.5e308], [5e-324, 1.0], [5e-324, f64::MAX]] {
        let (doc, mut g) = column_doc(&values);
        g.spec.y_axis.segments[0].scale = AxisScale::Log10;
        let scene = compile_guarded(doc, g);
        assert_all_primitives_finite(&scene);
        assert!(scene.primitives.len() < 1000);
    }
}
