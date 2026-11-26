//! Regression tests for Stage 4A `S2` XY / survival-cat chart findings (`R1-S08-04..06`, `R1-S08-08`, `R1-X2-06`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ChartScene, PathCmd, PointPt, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, Cell, CurveSamplePoint, DataSheet, DemingErrorSpec, EquationDisplayMode, ErrorBarMetric, ErrorBarSpec,
    FittedCurveOverlay, GraphAnnotation, GraphSheet, LinearRegressionSpec, NonlinearFitSpec, ProjectDocument, RectPt, RegressionOverlaySpec,
    ResultSheet, ResultStatus, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, XyTable,
};
use curvebook_fit::{HookContext, execute_analysis};
use curvebook_math::{AxisScale, LogitDomain};

use super::common::all_points_finite;

fn filled_marker_centers(scene: &ChartScene) -> Vec<PointPt> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } => match path.commands.first() {
                Some(PathCmd::MoveTo(pt)) if pt.x < 250.0 && (pt.x - 180.0).abs() > 1.0 => Some(*pt),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn numeric_tick_texts(scene: &ChartScene) -> Vec<(f64, PointPt)> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => t.rich.plain_text().trim().parse::<f64>().ok().map(|v| (v, t.anchor_pt)),
            _ => None,
        })
        .collect()
}

fn y_axis_tick_texts(scene: &ChartScene) -> Vec<(f64, PointPt)> {
    numeric_tick_texts(scene).into_iter().filter(|(_, p)| p.x < scene.plot_rect_pt.x && p.y <= scene.plot_rect_pt.bottom() + 6.0).collect()
}

fn x_axis_tick_texts(scene: &ChartScene) -> Vec<(f64, PointPt)> {
    numeric_tick_texts(scene)
        .into_iter()
        .filter(|(_, p)| p.x >= scene.plot_rect_pt.x - 10.0 && p.y > scene.plot_rect_pt.bottom() && p.y <= scene.plot_rect_pt.bottom() + 18.0)
        .collect()
}

fn build_xy_single_graph(points: &[(f64, f64)], x_scale: AxisScale, y_scale: AxisScale) -> (ProjectDocument, GraphSheet) {
    let mut doc = ProjectDocument::new("XY Audit").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, points.len()).unwrap();
    for (r, &(x, y)) in points.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new("d-xy", "XY", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();
    let mut graph = GraphSheet::new_uninitialized_for_data("g-xy", &ds).unwrap();
    graph.spec.x_axis.segments = vec![AxisSegment { scale: x_scale, ..AxisSegment::default() }];
    graph.spec.y_axis.segments = vec![AxisSegment { scale: y_scale, ..AxisSegment::default() }];
    (doc, graph)
}

#[test]
fn r1_s08_04_probit_and_logit_extremes_not_silently_dropped() {
    for scale in [AxisScale::Probit, AxisScale::Logit(LogitDomain::Percent)] {
        let (doc, graph) = build_xy_single_graph(
            &[(1.0, 0.005), (2.0, 0.05), (3.0, 20.0), (4.0, 50.0), (5.0, 80.0), (6.0, 99.95), (7.0, 99.995)],
            AxisScale::Linear,
            scale,
        );
        let scene = compile_graph(&graph, &doc).unwrap();
        assert!(all_points_finite(&scene), "{scale:?} scene must have finite coordinates");
        assert_eq!(scene.plotted_points.len(), 7, "all 7 extreme interior {scale:?} points must plot: {:?}", scene.warnings);
        assert!(filled_marker_centers(&scene).len() >= 7, "expected >= 7 marker primitives for {scale:?}");
        assert!(scene.warnings.is_empty(), "unexpected warnings for valid {scale:?} interior points: {:?}", scene.warnings);
        let ticks = numeric_tick_texts(&scene);
        assert!(ticks.iter().any(|(v, _)| *v < 0.1), "expected tail tick < 0.1 for {scale:?}: {ticks:?}");
        assert!(ticks.iter().any(|(v, _)| *v > 99.9), "expected upper tail tick > 99.9 for {scale:?}: {ticks:?}");
    }

    let (doc_prop, graph_prop) = build_xy_single_graph(
        &[(1.0, 0.0005), (2.0, 0.2), (3.0, 0.5), (4.0, 0.8), (5.0, 0.9995)],
        AxisScale::Linear,
        AxisScale::Logit(LogitDomain::Proportion),
    );
    let scene_prop = compile_graph(&graph_prop, &doc_prop).unwrap();
    assert!(all_points_finite(&scene_prop));
    assert_eq!(scene_prop.plotted_points.len(), 5, "all 5 Logit(Proportion) interior points must plot");
    assert!(scene_prop.warnings.is_empty(), "warnings={:?}", scene_prop.warnings);

    // Zoomed-in Probit window `[30, 70]` still emits >= 2 Y ticks via transformed-space fallback.
    let (doc_zoom, mut graph_zoom) = build_xy_single_graph(&[(1.0, 35.0), (2.0, 65.0)], AxisScale::Linear, AxisScale::Probit);
    graph_zoom.spec.y_axis.segments[0].min = Some(30.0);
    graph_zoom.spec.y_axis.segments[0].max = Some(70.0);
    let scene_zoom = compile_graph(&graph_zoom, &doc_zoom).unwrap();
    assert!(all_points_finite(&scene_zoom));
    assert_eq!(scene_zoom.plotted_points.len(), 2);
    let y_ticks_zoom: Vec<f64> = y_axis_tick_texts(&scene_zoom).into_iter().map(|(v, _)| v).collect();
    assert!(y_ticks_zoom.len() >= 2, "expected zoomed Probit Y ticks: {y_ticks_zoom:?}");

    // Survival chart with Probit Y axis repairs 0%/100% poles to valid interior bounds & draws curves cleanly.
    let mut doc_surv = ProjectDocument::new("Surv Probit").unwrap();
    let mut surv = SurvivalTable::new(1, 4).unwrap();
    for (r, (t, status)) in [(2.0, 1.0), (4.0, 0.0), (6.0, 1.0), (10.0, 1.0)].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(status);
    }
    let surv_ds = DataSheet::new("d-surv", "Survival", TableArchetype::Survival(surv)).unwrap();
    doc_surv.add_data_sheet(surv_ds.clone()).unwrap();
    let mut g_surv = GraphSheet::new_uninitialized_for_data("g-surv", &surv_ds).unwrap();
    g_surv.spec.y_axis.segments[0].scale = AxisScale::Probit;
    let scene_surv = compile_graph(&g_surv, &doc_surv).unwrap();
    assert!(all_points_finite(&scene_surv));
    assert!(!scene_surv.plotted_points.is_empty(), "survival Probit chart should plot KM events/censors");
}

#[test]
fn r1_s08_04b_reciprocal_mixed_sign_and_out_of_domain_warnings() {
    // Mixed signs on single Reciprocal segment must warn rather than dropping all 4 points silently.
    let (doc_mix, graph_mix) = build_xy_single_graph(&[(1.0, -2.0), (2.0, -1.0), (3.0, 1.0), (4.0, 5.0)], AxisScale::Linear, AxisScale::Reciprocal);
    let scene_mix = compile_graph(&graph_mix, &doc_mix).unwrap();
    assert!(all_points_finite(&scene_mix));
    assert!(
        scene_mix.warnings.iter().any(|w| w.code == "reciprocal_domain_crosses_zero"),
        "expected reciprocal_domain_crosses_zero warning, got {:?}",
        scene_mix.warnings
    );
    assert_eq!(scene_mix.plotted_points.len(), 2, "positive reciprocal points (1, 5) should still plot");

    // Broken Reciprocal axis with negative and positive segments plots all 4 points cleanly.
    let (doc_brk, mut graph_brk) =
        build_xy_single_graph(&[(1.0, -2.0), (2.0, -1.0), (3.0, 1.0), (4.0, 5.0)], AxisScale::Linear, AxisScale::Reciprocal);
    graph_brk.spec.y_axis.segments = vec![
        AxisSegment { min: Some(-5.0), max: Some(-0.5), scale: AxisScale::Reciprocal, length_share: 0.5, ..AxisSegment::default() },
        AxisSegment { min: Some(0.5), max: Some(10.0), scale: AxisScale::Reciprocal, length_share: 0.5, ..AxisSegment::default() },
    ];
    let scene_brk = compile_graph(&graph_brk, &doc_brk).unwrap();
    assert!(all_points_finite(&scene_brk));
    assert_eq!(scene_brk.plotted_points.len(), 4, "broken reciprocal segments must plot all 4 mixed-sign points");

    // All-negative single-point Reciprocal expands cleanly (`dmin < dmax < 0`) with zero warnings.
    let (doc_neg, graph_neg) = build_xy_single_graph(&[(1.0, -2.0)], AxisScale::Linear, AxisScale::Reciprocal);
    let scene_neg = compile_graph(&graph_neg, &doc_neg).unwrap();
    assert!(all_points_finite(&scene_neg));
    assert_eq!(scene_neg.plotted_points.len(), 1);
    assert!(scene_neg.warnings.is_empty(), "warnings={:?}", scene_neg.warnings);

    // Non-domain points emit scale-specific omission warning codes (`zero_on_reciprocal_axis`, `out_of_domain_probit`, `out_of_domain_logit`).
    for (scale, bad_y, expected_code) in [
        (AxisScale::Reciprocal, 0.0, "zero_on_reciprocal_axis"),
        (AxisScale::Probit, 100.0, "out_of_domain_probit"),
        (AxisScale::Logit(LogitDomain::Proportion), 0.0, "out_of_domain_logit"),
    ] {
        let valid_y = if matches!(scale, AxisScale::Logit(LogitDomain::Proportion)) { 0.5 } else { 50.0 };
        let (doc_bad, graph_bad) = build_xy_single_graph(&[(1.0, valid_y), (2.0, bad_y)], AxisScale::Linear, scale);
        let scene_bad = compile_graph(&graph_bad, &doc_bad).unwrap();
        assert!(all_points_finite(&scene_bad));
        assert!(
            scene_bad.warnings.iter().any(|w| w.code == expected_code),
            "expected {expected_code} for {scale:?} with {bad_y}, got {:?}",
            scene_bad.warnings
        );
    }
}

#[test]
fn r1_s08_05_reversed_axis_ranges_honoured_on_xy_and_survival_cat() {
    // XY linear reversed Y (`min = 100, max = 0`): larger values map lower on screen (larger y_pt).
    let (doc, mut graph) = build_xy_single_graph(&[(1.0, 10.0), (2.0, 30.0), (3.0, 60.0), (4.0, 90.0)], AxisScale::Linear, AxisScale::Linear);
    graph.spec.y_axis.segments[0].min = Some(100.0);
    graph.spec.y_axis.segments[0].max = Some(0.0);
    let scene = compile_graph(&graph, &doc).unwrap();
    assert!(all_points_finite(&scene));
    assert_eq!(scene.plotted_points.len(), 4, "reversed Y must render all 4 points");
    assert!(scene.warnings.is_empty(), "explicit reversed range must not warn invalid: {:?}", scene.warnings);
    for w in scene.plotted_points.windows(2) {
        assert!(
            w[1].center_pt.y > w[0].center_pt.y,
            "on reversed Y [100 -> 0], larger Y values must sit lower on canvas: {:?}",
            scene.plotted_points
        );
    }
    let y_ticks: Vec<(f64, PointPt)> = y_axis_tick_texts(&scene);
    let top_zero = y_ticks.iter().find(|(v, _)| (*v - 0.0).abs() < 1e-6).expect("0 tick missing").1.y;
    let bot_100 = y_ticks.iter().find(|(v, _)| (*v - 100.0).abs() < 1e-6).expect("100 tick missing").1.y;
    assert!(top_zero < bot_100, "0 tick ({top_zero}) must appear above 100 tick ({bot_100}) on reversed [100..0] Y axis");

    // Reversed Log10 X (`min = 1000, max = 1`) + Reciprocal reversed Y (`min = 10, max = 1`).
    let (doc_lr, mut graph_lr) = build_xy_single_graph(&[(10.0, 2.0), (100.0, 5.0), (500.0, 8.0)], AxisScale::Log10, AxisScale::Reciprocal);
    graph_lr.spec.x_axis.segments[0].min = Some(1000.0);
    graph_lr.spec.x_axis.segments[0].max = Some(1.0);
    graph_lr.spec.y_axis.segments[0].min = Some(10.0);
    graph_lr.spec.y_axis.segments[0].max = Some(1.0);
    let scene_lr = compile_graph(&graph_lr, &doc_lr).unwrap();
    assert!(all_points_finite(&scene_lr));
    assert_eq!(scene_lr.plotted_points.len(), 3);
    assert!(scene_lr.warnings.is_empty(), "warnings={:?}", scene_lr.warnings);
    assert!(scene_lr.plotted_points[0].center_pt.x > scene_lr.plotted_points[2].center_pt.x, "reversed Log10 X order must invert left-to-right");

    // Survival chart reversed Y (`min = 100, max = 0`): 100% survival maps below 0%.
    let mut doc_surv = ProjectDocument::new("Surv Reversed").unwrap();
    let mut surv = SurvivalTable::new(1, 3).unwrap();
    for (r, (t, s)) in [(2.0, 1.0), (5.0, 1.0), (8.0, 1.0)].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(s);
    }
    let surv_ds = DataSheet::new("d-surv-rev", "Survival", TableArchetype::Survival(surv)).unwrap();
    doc_surv.add_data_sheet(surv_ds.clone()).unwrap();
    let mut g_surv = GraphSheet::new_uninitialized_for_data("g-surv-rev", &surv_ds).unwrap();
    g_surv.spec.y_axis.title = String::from("Survival (%)");
    g_surv.spec.y_axis.segments[0].min = Some(100.0);
    g_surv.spec.y_axis.segments[0].max = Some(0.0);
    let s_surv = compile_graph(&g_surv, &doc_surv).unwrap();
    assert!(all_points_finite(&s_surv));
    assert_eq!(s_surv.plotted_points.len(), 3);
    let surv_y_ticks: Vec<(f64, PointPt)> = y_axis_tick_texts(&s_surv);
    let s_zero = surv_y_ticks.iter().find(|(v, _)| (*v - 0.0).abs() < 1e-6).expect("0 tick").1.y;
    let s_100 = surv_y_ticks.iter().find(|(v, _)| (*v - 100.0).abs() < 1e-6).expect("100 tick").1.y;
    assert!(s_zero < s_100, "reversed survival Y must put 0% above 100%");
}

#[test]
fn r1_s08_06_blank_sd_and_missing_n_pre_summarized_rules() {
    // 1. Blank SD with no error bars must preserve the mean point with zero warnings.
    let mut doc = ProjectDocument::new("S08-06 Blank SD").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::MeanSdN, 1, 3).unwrap();
    let rows = [(1.0, 50.0, Some(10.0), Some(100.0)), (2.0, 60.0, Some(10.0), Some(100.0)), (3.0, 70.0, None, Some(100.0))];
    for (r, (x, m, sd_opt, n_opt)) in rows.into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(m);
        xy.data_sets[0].cells[r][1] = sd_opt.map_or(Cell::empty(), Cell::number);
        xy.data_sets[0].cells[r][2] = n_opt.map_or(Cell::empty(), Cell::number);
    }
    let ds = DataSheet::new("d-s06", "Summary", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();
    let mut g = GraphSheet::new_uninitialized_for_data("g-s06", &ds).unwrap();
    g.spec.series[0].y_error_bars = None;
    g.spec.y_axis.segments[0].min = Some(0.0);
    g.spec.y_axis.segments[0].max = Some(100.0);
    let scene_no_eb = compile_graph(&g, &doc).unwrap();
    assert!(all_points_finite(&scene_no_eb));
    assert_eq!(scene_no_eb.plotted_points.len(), 3, "blank SD row with no error bars must still draw mean point");
    assert!(scene_no_eb.warnings.is_empty(), "warnings={:?}", scene_no_eb.warnings);

    // 2. Missing N on Mean/SD/N with SEM or CI95 must NEVER substitute +/-SD.
    let mut doc_n = ProjectDocument::new("S08-06 Missing N").unwrap();
    let mut xy_n = XyTable::new(SubcolumnMode::MeanSdN, 1, 2).unwrap();
    xy_n.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy_n.data_sets[0].cells[0] = vec![Cell::number(50.0), Cell::number(15.0), Cell::empty()];
    xy_n.set_x_cell(1, Cell::number(2.0)).unwrap();
    xy_n.data_sets[0].cells[1] = vec![Cell::number(60.0), Cell::number(-5.0), Cell::number(10.0)]; // negative SD
    let ds_n = DataSheet::new("d-n", "Missing N", TableArchetype::Xy(xy_n)).unwrap();
    doc_n.add_data_sheet(ds_n.clone()).unwrap();

    let mut g_base = GraphSheet::new_uninitialized_for_data("g-base", &ds_n).unwrap();
    g_base.spec.y_axis.segments[0].min = Some(0.0);
    g_base.spec.y_axis.segments[0].max = Some(100.0);
    g_base.spec.series[0].y_error_bars = None;
    let scene_base = compile_graph(&g_base, &doc_n).unwrap();
    assert_eq!(scene_base.plotted_points.len(), 2);

    for metric in [ErrorBarMetric::Sem, ErrorBarMetric::ConfidenceInterval95] {
        let mut g_m = g_base.clone();
        g_m.spec.series[0].y_error_bars = Some(ErrorBarSpec { metric, ..ErrorBarSpec::default() });
        let scene_m = compile_graph(&g_m, &doc_n).unwrap();
        assert!(all_points_finite(&scene_m));
        assert_eq!(scene_m.plotted_points.len(), 2, "mean points remain plotted when error bar cannot be reconstructed");
        assert!(
            scene_m.warnings.iter().any(|w| w.code == "invalid_pre_summarized_point"),
            "expected invalid_pre_summarized_point warning for {metric:?}, got {:?}",
            scene_m.warnings
        );
        assert_eq!(
            scene_m.primitives, scene_base.primitives,
            "missing N (and negative SD) with {metric:?} must emit NO error bar stems/caps (+/-SD fallback forbidden)"
        );
    }

    // 3. MeanSemN with missing N and ErrorBarMetric::Sd must NEVER substitute +/-SEM as +/-SD.
    let mut doc_sem = ProjectDocument::new("MeanSemN").unwrap();
    let mut xy_sem = XyTable::new(SubcolumnMode::MeanSemN, 1, 1).unwrap();
    xy_sem.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy_sem.data_sets[0].cells[0] = vec![Cell::number(50.0), Cell::number(5.0), Cell::empty()];
    let ds_sem = DataSheet::new("d-sem", "MeanSemN", TableArchetype::Xy(xy_sem)).unwrap();
    doc_sem.add_data_sheet(ds_sem.clone()).unwrap();
    let mut g_sem_none = GraphSheet::new_uninitialized_for_data("g-sem", &ds_sem).unwrap();
    g_sem_none.spec.y_axis.segments[0].min = Some(0.0);
    g_sem_none.spec.y_axis.segments[0].max = Some(100.0);
    g_sem_none.spec.series[0].y_error_bars = None;
    let scene_sem_none = compile_graph(&g_sem_none, &doc_sem).unwrap();
    let mut g_sem_sd = g_sem_none.clone();
    g_sem_sd.spec.series[0].y_error_bars = Some(ErrorBarSpec { metric: ErrorBarMetric::Sd, ..ErrorBarSpec::default() });
    let scene_sem_sd = compile_graph(&g_sem_sd, &doc_sem).unwrap();
    assert_eq!(scene_sem_sd.primitives, scene_sem_none.primitives, "MeanSemN missing N must not draw +/-SEM when Sd is requested");
    assert!(scene_sem_sd.warnings.iter().any(|w| w.code == "invalid_pre_summarized_point"));

    // 4. MeanSdN with N=1 and ConfidenceInterval95 warns cleanly without error bar.
    let mut doc_ci = ProjectDocument::new("N=1 CI").unwrap();
    let mut xy_ci = XyTable::new(SubcolumnMode::MeanSdN, 1, 1).unwrap();
    xy_ci.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy_ci.data_sets[0].cells[0] = vec![Cell::number(50.0), Cell::number(10.0), Cell::number(1.0)];
    let ds_ci = DataSheet::new("d-ci", "CI", TableArchetype::Xy(xy_ci)).unwrap();
    doc_ci.add_data_sheet(ds_ci.clone()).unwrap();
    let mut g_ci = GraphSheet::new_uninitialized_for_data("g-ci", &ds_ci).unwrap();
    g_ci.spec.y_axis.segments[0].min = Some(0.0);
    g_ci.spec.y_axis.segments[0].max = Some(100.0);
    g_ci.spec.series[0].y_error_bars = Some(ErrorBarSpec { metric: ErrorBarMetric::ConfidenceInterval95, ..ErrorBarSpec::default() });
    let scene_ci = compile_graph(&g_ci, &doc_ci).unwrap();
    assert!(scene_ci.warnings.iter().any(|w| w.code == "insufficient_sample_size_for_ci"));
}

#[test]
fn r1_s08_08_major_tick_interval_covers_full_span_and_handles_tiny_steps() {
    // 101 ticks (`0..=1000` step `10`) must NOT truncate at 64 (640).
    let (doc, mut graph) = build_xy_single_graph(&[(1.0, 0.0), (2.0, 1000.0)], AxisScale::Linear, AxisScale::Linear);
    graph.spec.y_axis.segments[0].min = Some(0.0);
    graph.spec.y_axis.segments[0].max = Some(1000.0);
    graph.spec.y_axis.segments[0].major_tick_interval = Some(10.0);
    let scene = compile_graph(&graph, &doc).unwrap();
    assert!(all_points_finite(&scene));
    assert!(scene.warnings.is_empty(), "101 ticks is below 256 cap; unexpected warning {:?}", scene.warnings);
    let y_ticks: Vec<f64> = y_axis_tick_texts(&scene).into_iter().map(|(v, _)| v).collect();
    assert_eq!(y_ticks.len(), 101, "expected all 101 Y ticks [0, 10, ..., 1000], got {}", y_ticks.len());
    let max_tick = y_ticks.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!((max_tick - 1000.0).abs() < 1e-6, "max Y tick was {max_tick}, expected 1000");

    // Dense interval > 256 (`0..=1000` step `1.0`) must stride across entire axis and emit warning.
    graph.spec.y_axis.segments[0].major_tick_interval = Some(1.0);
    let scene_dense = compile_graph(&graph, &doc).unwrap();
    assert!(all_points_finite(&scene_dense));
    assert!(
        scene_dense.warnings.iter().any(|w| w.code == "major_tick_interval_capped"),
        "expected major_tick_interval_capped warning, got {:?}",
        scene_dense.warnings
    );
    let y_dense: Vec<f64> = y_axis_tick_texts(&scene_dense).into_iter().map(|(v, _)| v).collect();
    let max_dense = y_dense.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!((max_dense - 1000.0).abs() < 1e-6, "strided ticks must still reach 1000, got {max_dense}");

    // Tiny adversarial interval `1e-300` over `[-10, 10]` must not panic on i64 overflow.
    let (doc_tiny, mut graph_tiny) = build_xy_single_graph(&[(1.0, -10.0), (2.0, 10.0)], AxisScale::Linear, AxisScale::Linear);
    graph_tiny.spec.y_axis.segments[0].min = Some(-10.0);
    graph_tiny.spec.y_axis.segments[0].max = Some(10.0);
    graph_tiny.spec.y_axis.segments[0].major_tick_interval = Some(1e-300);
    graph_tiny.spec.y_axis.segments[0].minor_ticks_per_interval = 2;
    let scene_tiny = compile_graph(&graph_tiny, &doc_tiny).unwrap();
    assert!(all_points_finite(&scene_tiny));
    assert!(scene_tiny.warnings.iter().any(|w| w.code == "major_tick_interval_capped"));

    // Survival chart `0..=1000` step `20` (51 ticks > old cap 32) covers full 1000 without silent stop at 640.
    let mut doc_s = ProjectDocument::new("Surv Ticks").unwrap();
    let mut surv = SurvivalTable::new(1, 2).unwrap();
    surv.times[0] = Cell::number(500.0);
    surv.cohorts[0].cells[0][0] = Cell::number(1.0);
    surv.times[1] = Cell::number(900.0);
    surv.cohorts[0].cells[1][0] = Cell::number(1.0);
    let ds_s = DataSheet::new("d-surv-t", "Surv", TableArchetype::Survival(surv)).unwrap();
    doc_s.add_data_sheet(ds_s.clone()).unwrap();
    let mut g_s = GraphSheet::new_uninitialized_for_data("g-surv-t", &ds_s).unwrap();
    g_s.spec.x_axis.segments[0].min = Some(0.0);
    g_s.spec.x_axis.segments[0].max = Some(1000.0);
    g_s.spec.x_axis.segments[0].major_tick_interval = Some(20.0);
    let scene_s = compile_graph(&g_s, &doc_s).unwrap();
    assert!(all_points_finite(&scene_s));
    let surv_x_max = x_axis_tick_texts(&scene_s).into_iter().map(|(v, _)| v).fold(f64::NEG_INFINITY, f64::max);
    assert!((surv_x_max - 1000.0).abs() < 1e-6, "survival X ticks stopped at {surv_x_max}, expected 1000");
}

#[test]
fn r1_x2_06_fitted_overlay_never_borrows_wrong_dataset_or_failed_result() {
    let mut doc = ProjectDocument::new("Overlay Match").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4).unwrap();
    for (r, (x, y0, y1)) in [(1.0, 10.0, 90.0), (2.0, 20.0, 80.0), (3.0, 30.0, 70.0), (4.0, 40.0, 60.0)].into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y0);
        xy.data_sets[1].cells[r][0] = Cell::number(y1);
    }
    let ds = DataSheet::new("d-ov", "Two Sets", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();

    // Result sheet containing ONLY a fitted curve for data_set_index = 0 (up to date).
    let fit_spec = AnalysisSpec::new("res-only-0", "d-ov", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("linear"))).unwrap();
    let mut res = ResultSheet::new(fit_spec.clone()).unwrap();
    res.status = ResultStatus::UpToDate;
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 1.0, y: 10.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 4.0, y: 40.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    doc.add_result_sheet(res).unwrap();

    let mut g_none = GraphSheet::new_uninitialized_for_data("g-ov", &ds).unwrap();
    g_none.spec.series[0].regression_overlay = None;
    g_none.spec.series[1].regression_overlay = None;
    let scene_none = compile_graph(&g_none, &doc).unwrap();

    // Attach res-only-0 overlay to series[1] (data_set_index = 1): must NOT borrow curve 0!
    let mut g_wrong = g_none.clone();
    g_wrong.spec.series[1].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-only-0"),
        sample_segments: 50,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    let scene_wrong = compile_graph(&g_wrong, &doc).unwrap();
    assert!(all_points_finite(&scene_wrong));
    assert!(
        scene_wrong.warnings.iter().any(|w| w.code == "missing_fitted_curve"),
        "expected missing_fitted_curve warning, got {:?}",
        scene_wrong.warnings
    );
    assert_eq!(scene_wrong.primitives, scene_none.primitives, "unfitted data set must NOT draw dataset 0's curve or legend line");

    // Failed ResultSheet must NEVER draw overlay even if data_set_index matches.
    let mut failed_res = ResultSheet::new(AnalysisSpec::new("res-fail", "d-ov", fit_spec.kind).unwrap()).unwrap();
    failed_res.status = ResultStatus::Failed { code: curvebook_doc::ErrorCode::SolverFailed, message: String::from("failed") };
    failed_res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 1.0, y: 10.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 4.0, y: 40.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    doc.add_result_sheet(failed_res).unwrap();

    let mut g_fail = g_none.clone();
    g_fail.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-fail"),
        sample_segments: 50,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    let scene_fail = compile_graph(&g_fail, &doc).unwrap();
    assert!(scene_fail.warnings.iter().any(|w| w.code == "failed_regression_result"));
    assert_eq!(scene_fail.primitives, scene_none.primitives, "failed result must not render stale fitted curves");
}

#[test]
fn default_model_ii_deming_overlay_renders_cleanly() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8).expect("xy");
    xy.data_sets[0].title = String::from("Method B");
    for (r, (x, y)) in [(1.0, 2.2), (2.0, 4.1), (3.0, 5.9), (4.0, 8.3), (5.0, 10.0), (6.0, 12.1), (7.0, 13.8), (8.0, 16.2)].into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).expect("x");
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new("xy-deming", "Method Comparison", TableArchetype::Xy(xy)).expect("ds");

    for (tag, err_spec) in [
        ("equal", DemingErrorSpec::EqualError),
        ("sds", DemingErrorSpec::StandardDeviations { sd_x: 0.4, sd_y: 0.8 }),
        ("ratio", DemingErrorSpec::VarianceRatio { lambda: 2.0 }),
    ] {
        let lin_spec = LinearRegressionSpec { deming_model_ii: true, deming_error: err_spec, ..LinearRegressionSpec::default() };
        let res_id = SheetId::new(format!("res-deming-{tag}"));
        let res = execute_analysis(
            &AnalysisSpec::new(res_id.clone(), "xy-deming", AnalysisKind::LinearRegression(lin_spec)).expect("spec"),
            &ds,
            &HookContext::default(),
        )
        .expect("deming execute");
        assert_eq!(res.status, ResultStatus::UpToDate);
        assert_eq!(res.fitted_curves.len(), 1, "expected default Deming overlay for {tag}");

        let mut doc = ProjectDocument::new("DemingOverlay").expect("doc");
        doc.add_data_sheet(ds.clone()).expect("add ds");
        doc.add_result_sheet(res).expect("add res");

        let mut graph = GraphSheet::new_uninitialized_for_data(format!("g-deming-{tag}"), &ds).expect("graph");
        graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
            result_sheet_id: res_id.clone(),
            sample_segments: 256,
            show_confidence_band: false,
            show_prediction_band: false,
            shaded_ribbon: true,
        });
        graph.spec.annotations.push(GraphAnnotation::ModelEquation {
            result_sheet_id: res_id,
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(80.0, 30.0, 0.0, 0.0),
            font_size_pt: 10.0,
            bordered: true,
        });

        let scene = compile_graph(&graph, &doc).expect("compile deming chart");
        assert!(scene.warnings.is_empty(), "warnings for {tag}: {:?}", scene.warnings);
        let overlay_strokes = scene
            .primitives
            .iter()
            .filter(|p| {
                matches!(
                    p,
                    ScenePrimitive::Path { path, fill: None, stroke: Some(st), clip: Some(_) }
                        if (st.width_pt - 1.75).abs() < 1e-6 && path.commands.len() >= 16
                )
            })
            .count();
        assert!(overlay_strokes >= 1, "expected rendered Deming fitted curve path for {tag}");
    }
}
