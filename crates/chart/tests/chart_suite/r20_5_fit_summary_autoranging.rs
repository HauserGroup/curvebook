//! Regression and prototype-evaluation tests for `R20.5`:
//! - Fit-summary readability & adjacent vs. automatic inside-plot inset placement across
//!   sparse dose-response curves, dense multi-series scatter plots, and narrow viewports.
//! - Explicit backward-compatibility rules: saved figures via `compile_graph` remain
//!   bit-identical; decoupled figure typography (`FigureTypography`) satisfies preflight;
//!   manual axis limits (`auto_range == false`, i.e., explicit `AxisSegment::min`/`max`)
//!   are never overridden by autoranging headroom.
//! - Autoranging edge cases: zero-range constants, strictly negative linear data,
//!   log axes with non-positive/decade values, and `NaN`/`±Inf` values and headroom.

use curvebook_chart::fit_summary::{
    DEFAULT_FIGURE_TYPOGRAPHY, FitSummaryPlacement, FitSummarySpec, MIN_USABLE_PLOT_WIDTH_PT, PlacementStrategy, evaluate_fit_summary_placement,
    extract_fit_summary_entries, measure_fit_summary_card,
};
use curvebook_chart::scales::{AxisOrientation, DEFAULT_AUTORANGE_HEADROOM, ResolvedAxis, segment_has_manual_limits, segment_is_auto_range};
use curvebook_chart::{
    ChartScene, CompileOptions, PathCmd, PointPt, RectPt, ScenePrimitive, compile_graph, compile_graph_with_options, preflight_check_scene,
};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, AxisSpec, Cell, CurveSamplePoint, DataSheet, FittedCurveOverlay, GraphSheet, NonlinearFitSpec,
    ProjectDocument, RegressionOverlaySpec, ResultScalarEntry, ResultSheet, ResultStatus, SheetId, SizePt, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_fit::{HookContext, execute_analysis};
use curvebook_math::AxisScale;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn build_sparse_dose_response_4pl() -> Result<(ProjectDocument, GraphSheet), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("R205Sparse4PL")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8)?;
    xy.data_sets[0].title = String::from("Agonist A");
    let pts: [(f64, f64); 8] = [(-9.0, 10.0), (-8.0, 14.0), (-7.0, 28.0), (-6.0, 58.0), (-5.0, 86.0), (-4.0, 95.0), (-3.0, 99.0), (-2.0, 100.0)];
    for (r, (x, y)) in pts.into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x))?;
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new("xy-4pl", "Dose Response", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(ds.clone())?;

    let nl_spec = AnalysisSpec::new("res-4pl", "xy-4pl", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl")))?;
    let nl_res = execute_analysis(&nl_spec, &ds, &HookContext::default())?;
    assert_eq!(nl_res.status, ResultStatus::UpToDate);
    doc.add_result_sheet(nl_res)?;

    let mut graph = GraphSheet::new_uninitialized_for_data("g-4pl", &ds)?;
    graph.spec.title = String::from("Dose-Response 4PL");
    graph.spec.x_axis.title = String::from("log[Agonist] (M)");
    graph.spec.y_axis.title = String::from("Response (%)");
    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-4pl"),
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    doc.add_graph_sheet(graph.clone())?;
    Ok((doc, graph))
}

fn build_dense_four_quadrant_scatter() -> Result<(ProjectDocument, GraphSheet), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("R205DenseScatter")?;
    let n_rows = 16;
    let mut xy = XyTable::new(SubcolumnMode::Single, 4, n_rows)?;
    xy.data_sets[0].title = String::from("Ascending");
    xy.data_sets[1].title = String::from("Descending");
    xy.data_sets[2].title = String::from("Upper Band");
    xy.data_sets[3].title = String::from("Lower Band");
    for r in 0..n_rows {
        let x = (r as f64) * 6.5;
        xy.set_x_cell(r, Cell::number(x))?;
        xy.data_sets[0].cells[r][0] = Cell::number(5.0 + 0.9 * x);
        xy.data_sets[1].cells[r][0] = Cell::number(95.0 - 0.9 * x);
        xy.data_sets[2].cells[r][0] = Cell::number(82.0 + ((r % 3) as f64) * 6.0);
        xy.data_sets[3].cells[r][0] = Cell::number(6.0 + ((r % 3) as f64) * 6.0);
    }
    let ds = DataSheet::new("xy-dense", "Dense Multi-Series", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(ds.clone())?;

    let spec = AnalysisSpec::new("res-dense", "xy-dense", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line")))?;
    let mut res = ResultSheet::new(spec)?;
    res.status = ResultStatus::UpToDate;
    for s in 0..2 {
        let slope = if s == 0 { 0.9 } else { -0.9 };
        let intercept = if s == 0 { 5.0 } else { 95.0 };
        res.scalars.push(ResultScalarEntry { key: String::from("R2"), data_set_index: Some(s), value: 0.994, units: None });
        res.scalars.push(ResultScalarEntry { key: String::from("Slope"), data_set_index: Some(s), value: slope, units: None });
        let pts: Vec<CurveSamplePoint> = (0..=20)
            .map(|i| {
                let x = (i as f64) * 5.0;
                CurveSamplePoint { x, y: intercept + slope * x, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None }
            })
            .collect();
        res.fitted_curves.push(FittedCurveOverlay {
            data_set_index: s,
            equation_id: String::from("straight_line"),
            converged: true,
            ambiguous: false,
            points: pts,
        });
    }
    doc.add_result_sheet(res)?;

    let mut graph = GraphSheet::new_uninitialized_for_data("g-dense", &ds)?;
    graph.spec.title = String::from("Dense Multi-Series");
    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-dense"),
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    graph.spec.series[1].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-dense"),
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    doc.add_graph_sheet(graph.clone())?;
    Ok((doc, graph))
}

fn find_fit_summary_card_rect(scene: &ChartScene) -> Option<RectPt> {
    scene.primitives.iter().find_map(|p| match p {
        ScenePrimitive::Path { path, fill: Some(fill), stroke: Some(_), clip: None }
            if fill.color == curvebook_palette::Rgba8::rgba(255, 255, 255, 245) =>
        {
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for cmd in &path.commands {
                if let PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) = *cmd {
                    min_x = min_x.min(pt.x);
                    min_y = min_y.min(pt.y);
                    max_x = max_x.max(pt.x);
                    max_y = max_y.max(pt.y);
                }
            }
            (min_x.is_finite() && min_y.is_finite()).then(|| RectPt::new(min_x, min_y, max_x - min_x, max_y - min_y))
        }
        _ => None,
    })
}

#[test]
fn prototype_evaluation_adjacent_vs_auto_inset_sparse_and_dense_plots() -> TestResult {
    // 1. Sparse 4PL dose-response curve (`360 x 260 pt`).
    let (sparse_doc, sparse_graph) = build_sparse_dose_response_4pl()?;
    let adj_opts = CompileOptions { fit_summary: FitSummarySpec { enabled: true, ..FitSummarySpec::default() } };
    let sparse_scene = compile_graph_with_options(&sparse_graph, &sparse_doc, &adj_opts)?;
    let sparse_entries = extract_fit_summary_entries(&sparse_graph, &sparse_doc, 3);
    assert_eq!(sparse_entries.len(), 1);
    assert!(!sparse_entries[0].metrics.is_empty(), "expected extracted 4PL metrics");

    let card_rect = find_fit_summary_card_rect(&sparse_scene).ok_or("expected fit-summary card on sparse 4PL scene")?;
    assert!(
        card_rect.x > sparse_scene.plot_rect_pt.right(),
        "adjacent fit summary must sit outside plot_rect_pt: card_x={:.1}, plot_right={:.1}",
        card_rect.x,
        sparse_scene.plot_rect_pt.right()
    );
    assert!(card_rect.right() <= sparse_scene.width_pt + 0.5, "adjacent fit summary overflowed canvas right edge");
    assert!(sparse_scene.plot_rect_pt.width >= MIN_USABLE_PLOT_WIDTH_PT, "plot width must remain usable");
    assert!(preflight_check_scene(&sparse_scene).is_empty(), "sparse fit-summary scene must pass preflight cleanly");

    // Verify evaluation report directly on sparse plot: adjacent has 0 occlusions & is chosen.
    let sparse_eval =
        evaluate_fit_summary_placement(&sparse_scene, &sparse_entries, &[], None, 0.0, &adj_opts.fit_summary).ok_or("expected sparse evaluation")?;
    assert_eq!(sparse_eval.adjacent.occluded_points, 0);
    assert_eq!(sparse_eval.adjacent.occluded_curve_samples, 0);
    assert_eq!(sparse_eval.chosen.strategy, PlacementStrategy::AdjacentRight);
    assert!(sparse_eval.chosen.font_size_pt >= DEFAULT_FIGURE_TYPOGRAPHY.fit_summary_min_pt);

    // 2. Dense 4-quadrant scatter plot (`360 x 260 pt`).
    let (dense_doc, dense_graph) = build_dense_four_quadrant_scatter()?;
    let dense_scene = compile_graph_with_options(&dense_graph, &dense_doc, &adj_opts)?;
    let dense_entries = extract_fit_summary_entries(&dense_graph, &dense_doc, 2);
    assert_eq!(dense_entries.len(), 2);

    let dense_eval =
        evaluate_fit_summary_placement(&dense_scene, &dense_entries, &[], None, 0.0, &adj_opts.fit_summary).ok_or("expected dense evaluation")?;
    assert_eq!(dense_eval.adjacent.occluded_points, 0, "adjacent placement must have zero point occlusions on dense scatter");
    assert!(
        dense_eval.inset_candidates.iter().all(|c| c.occluded_points > 0),
        "all 4 inside-plot corners should occlude points on dense 4-quadrant scatter: {:?}",
        dense_eval.inset_candidates
    );
    assert!(dense_eval.best_inset.occluded_points > 0);
    assert_eq!(dense_eval.chosen.strategy, PlacementStrategy::AdjacentRight);

    // 3. Small / narrow viewport (`220 x 160 pt`): adjacent cannot retain MIN_USABLE_PLOT_WIDTH_PT
    //    without clipping; AdjacentPreferred gracefully falls back to AutoInset on sparse plot.
    let mut narrow_graph = sparse_graph;
    narrow_graph.spec.canvas_size_pt = SizePt::new(220.0, 160.0);
    narrow_graph.spec.legend.visible = false;
    let narrow_scene = compile_graph_with_options(&narrow_graph, &sparse_doc, &adj_opts)?;
    assert!(narrow_scene.plot_rect_pt.width >= MIN_USABLE_PLOT_WIDTH_PT, "narrow viewport must not crush plot width");
    let narrow_eval =
        evaluate_fit_summary_placement(&narrow_scene, &sparse_entries, &[], None, 0.0, &adj_opts.fit_summary).ok_or("expected narrow evaluation")?;
    assert!(!narrow_eval.adjacent.readable, "adjacent should report unreadable on 220 pt narrow canvas");
    assert_eq!(narrow_eval.chosen.strategy, PlacementStrategy::AutoInset, "narrow sparse plot should fall back to auto_inset");
    Ok(())
}

#[test]
fn backward_compatibility_saved_figures_and_decoupled_typography() -> TestResult {
    let (doc, graph) = build_sparse_dose_response_4pl()?;
    let legacy_scene = compile_graph(&graph, &doc)?;
    let default_opt_scene = compile_graph_with_options(&graph, &doc, &CompileOptions::default())?;
    assert_eq!(legacy_scene, default_opt_scene, "default CompileOptions must be bit-identical to compile_graph");
    assert!(find_fit_summary_card_rect(&legacy_scene).is_none(), "saved figures must not inject an unsolicited fit-summary card unless opted in");

    // Decoupled figure typography remains independent of UI chrome (13 pt / 11.5 pt) and >= 5 pt preflight floor.
    let typo = DEFAULT_FIGURE_TYPOGRAPHY;
    assert!((typo.title_pt - 12.0).abs() < 1e-12);
    assert!((typo.axis_title_pt - 10.0).abs() < 1e-12);
    assert!((typo.tick_label_pt - 8.5).abs() < 1e-12);
    assert!((typo.legend_pt - 8.5).abs() < 1e-12);
    assert!(typo.fit_summary_pt >= typo.fit_summary_min_pt);
    assert!(typo.fit_summary_min_pt > typo.preflight_floor_pt);

    let (w, h) = measure_fit_summary_card(&extract_fit_summary_entries(&graph, &doc, 3), typo.fit_summary_pt);
    assert!(w > 20.0 && h > 15.0);
    Ok(())
}

#[test]
fn manual_axis_limits_auto_range_false_are_never_overridden_by_headroom() {
    struct ManualLimitCase {
        scale: AxisScale,
        min: Option<f64>,
        max: Option<f64>,
        samples: &'static [f64],
    }

    let cases = [
        ManualLimitCase { scale: AxisScale::Linear, min: Some(0.0), max: Some(100.0), samples: &[12.0, 50.0, 99.0] },
        ManualLimitCase { scale: AxisScale::Linear, min: Some(-50.0), max: Some(-5.0), samples: &[-40.0, -10.0] },
        ManualLimitCase { scale: AxisScale::Linear, min: Some(100.0), max: Some(0.0), samples: &[10.0, 90.0] },
        ManualLimitCase { scale: AxisScale::Log10, min: Some(0.1), max: Some(100.0), samples: &[1.0, 10.0, 95.0] },
        ManualLimitCase { scale: AxisScale::Log2, min: Some(1.0), max: Some(64.0), samples: &[2.0, 32.0] },
        ManualLimitCase { scale: AxisScale::Ln, min: Some(0.5), max: Some(50.0), samples: &[1.0, 20.0] },
    ];
    for case in &cases {
        let mut spec = AxisSpec::linear("Y");
        spec.segments = vec![AxisSegment { min: case.min, max: case.max, scale: case.scale, ..AxisSegment::default() }];
        assert!(segment_has_manual_limits(&spec.segments[0]));
        assert!(!segment_is_auto_range(&spec.segments[0]));

        let mut warnings = Vec::new();
        let resolved = ResolvedAxis::resolve_from_samples(&spec, AxisOrientation::Vertical, (200.0, 20.0), case.samples, true, 0.25, &mut warnings);
        assert_eq!(resolved.segments.len(), 1);
        let seg = &resolved.segments[0];
        if let Some(lo) = case.min {
            assert_eq!(seg.domain_min.to_bits(), lo.to_bits(), "manual min overridden for {:?}", case.scale);
        }
        if let Some(hi) = case.max {
            assert_eq!(seg.domain_max.to_bits(), hi.to_bits(), "manual max overridden for {:?}", case.scale);
        }
    }

    // Partial manual limits: explicit max stays exact while auto min adapts; explicit min stays exact while auto max adds headroom.
    let mut partial_max = AxisSpec::linear("Y");
    partial_max.segments[0].max = Some(100.0);
    let mut w = Vec::new();
    let res_pmax = ResolvedAxis::resolve_from_samples(&partial_max, AxisOrientation::Vertical, (200.0, 20.0), &[10.0, 98.0], false, 0.20, &mut w);
    assert_eq!(res_pmax.segments[0].domain_max.to_bits(), 100.0_f64.to_bits());
    assert!(res_pmax.segments[0].domain_min <= 10.0);

    let mut partial_min = AxisSpec::linear("Y");
    partial_min.segments[0].min = Some(5.0);
    let res_pmin = ResolvedAxis::resolve_from_samples(&partial_min, AxisOrientation::Vertical, (200.0, 20.0), &[10.0, 98.0], false, 0.20, &mut w);
    assert_eq!(res_pmin.segments[0].domain_min.to_bits(), 5.0_f64.to_bits());
    assert!(res_pmin.segments[0].domain_max > 98.0);
}

#[test]
fn autoranging_edge_cases_zero_range_negative_log_nan_inf() {
    let auto_spec = |scale: AxisScale| {
        let mut s = AxisSpec::linear("Y");
        s.segments[0].scale = scale;
        assert!(segment_is_auto_range(&s.segments[0]));
        s
    };

    // 1. Strictly negative linear data `[-100.0, -10.0]` must NOT overshoot above 0.0 due to top headroom.
    const {
        assert!(DEFAULT_AUTORANGE_HEADROOM.clamp_negative_max_at_zero);
    }
    let mut w = Vec::new();
    let neg = ResolvedAxis::resolve_from_samples(
        &auto_spec(AxisScale::Linear),
        AxisOrientation::Vertical,
        (200.0, 20.0),
        &[-100.0, -50.0, -10.0],
        false,
        0.05,
        &mut w,
    );
    assert!(neg.segments[0].domain_min <= -100.0);
    assert!(
        neg.segments[0].domain_max <= 0.0 && neg.segments[0].domain_max >= -10.0,
        "all-negative linear data overshot past zero: [{}, {}]",
        neg.segments[0].domain_min,
        neg.segments[0].domain_max
    );

    // 2. Zero-range constant data (positive, zero, negative).
    for constant in [50.0, 0.0, -25.0] {
        let res = ResolvedAxis::resolve_from_samples(
            &auto_spec(AxisScale::Linear),
            AxisOrientation::Vertical,
            (200.0, 20.0),
            &[constant, constant, constant],
            false,
            0.05,
            &mut w,
        );
        let seg = &res.segments[0];
        assert!(seg.domain_min.is_finite() && seg.domain_max.is_finite() && seg.domain_min < constant && seg.domain_max > constant);
    }

    // 3. NaN / ±Inf in samples and non-finite / negative top_headroom_frac never produce NaN/Inf domains.
    for bad_headroom in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.5] {
        let res = ResolvedAxis::resolve_from_samples(
            &auto_spec(AxisScale::Linear),
            AxisOrientation::Vertical,
            (200.0, 20.0),
            &[f64::NAN, 10.0, f64::INFINITY, 80.0, f64::NEG_INFINITY],
            false,
            bad_headroom,
            &mut w,
        );
        let seg = &res.segments[0];
        assert!(seg.domain_min.is_finite() && seg.domain_max.is_finite());
        assert!(seg.domain_min <= 10.0 && seg.domain_max >= 80.0);
    }

    // 4. Log10 axis with non-positive values filtered cleanly & finite positive bounds preserved.
    let log_res = ResolvedAxis::resolve_from_samples(
        &auto_spec(AxisScale::Log10),
        AxisOrientation::Vertical,
        (200.0, 20.0),
        &[-5.0, 0.0, f64::NAN, 1.0, 100.0, f64::INFINITY],
        false,
        0.05,
        &mut w,
    );
    let lseg = &log_res.segments[0];
    assert!(lseg.domain_min > 0.0 && lseg.domain_min <= 1.0);
    assert!(lseg.domain_max.is_finite() && lseg.domain_max >= 100.0);
}

#[test]
fn fit_summary_compilation_is_bit_exact_deterministic() -> TestResult {
    let (doc, graph) = build_sparse_dose_response_4pl()?;
    for placement in [
        FitSummaryPlacement::AdjacentPreferred,
        FitSummaryPlacement::AdjacentRight,
        FitSummaryPlacement::AutoInset,
        FitSummaryPlacement::Explicit(PointPt::new(80.0, 40.0)),
    ] {
        let opts = CompileOptions { fit_summary: FitSummarySpec { enabled: true, placement, ..FitSummarySpec::default() } };
        let s1 = compile_graph_with_options(&graph, &doc, &opts)?;
        let s2 = compile_graph_with_options(&graph, &doc, &opts)?;
        assert_eq!(s1, s2, "fit-summary scene must be bit-exact deterministic for {placement:?}");
    }
    Ok(())
}
