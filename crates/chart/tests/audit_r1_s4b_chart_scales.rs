//! Regression tests for Stage 4B `S3` chart scale findings (`R1-S08-09`, `R1-S08-10`, `R1-S08-11`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc;
use std::time::Duration;

use curvebook_chart::scales::{
    AxisMapping, AxisOrientation, AxisRenderOptions, MAX_TICKS_PER_SEGMENT, ResolvedAxis, ScaleDiagnostics, TickComputeConfig, TickKind,
    compute_axis_ticks_with_warnings, render_graph_chrome, resolve_axes_and_ticks,
};
use curvebook_chart::{ChartScene, PathCmd, RectPt, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, AxisSpec, Cell, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, DataSheet,
    DiagnosticPlotKind, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, GraphKind, GraphSheet, LogitDomain, MultipleVariablesTable,
    MultivariateSpec, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SizePt, SubcolumnMode,
    SurvivalTable, TableArchetype, VariableRole, XyTable,
};
use curvebook_math::AxisScale;
use curvebook_text::LocaleNumberPolicy;

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
            ScenePrimitive::RasterImage(img) => {
                assert!(
                    img.bounds_pt.x.is_finite()
                        && img.bounds_pt.y.is_finite()
                        && img.bounds_pt.width.is_finite()
                        && img.bounds_pt.height.is_finite()
                        && img.rotation_deg.is_finite()
                );
            }
            ScenePrimitive::PushClip { rect } => {
                assert!(rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite());
            }
            ScenePrimitive::PopClip => {}
        }
    }
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

fn xy_doc_with_points(points: &[(f64, f64)]) -> (ProjectDocument, GraphSheet) {
    let mut doc = ProjectDocument::new("S4B-XY").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, points.len()).unwrap();
    for (r, &(x, y)) in points.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new("d-xy", "XY", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();
    let mut graph = GraphSheet::new_uninitialized_for_data("g-xy", &ds).unwrap();
    graph.initialized = true;
    (doc, graph)
}

#[test]
fn r1_s08_09_scales_and_compile_graph_share_guards_and_tick_engine() {
    let policy = LocaleNumberPolicy::publication();
    let plot_rect = RectPt::new(50.0, 24.0, 300.0, 200.0);

    // 1. Extreme log ranges (5e-324..=f64::MAX) through `ResolvedAxis`, `compute_axis_ticks_with_warnings`, and `render_graph_chrome`.
    for scale in [AxisScale::Log10, AxisScale::Log2, AxisScale::Ln] {
        let mut spec = AxisSpec::linear("Extreme Log");
        spec.segments = vec![AxisSegment { scale, ..AxisSegment::default() }];
        let mut warnings = Vec::new();
        let resolved = ResolvedAxis::resolve(&spec, AxisOrientation::Vertical, plot_rect, Some((5e-324, f64::MAX)), &mut warnings);
        let ticks = compute_axis_ticks_with_warnings(
            &resolved,
            TickComputeConfig { tick_format: spec.tick_format, policy: &policy, auto_log_minor_ticks: true, categorical_ticks: None },
            &mut warnings,
        );
        assert!(ticks.len() <= MAX_TICKS_PER_SEGMENT * 2, "{scale:?} emitted too many ticks: {}", ticks.len());
        assert!(ticks.iter().all(|t| t.value.is_finite() && t.value > 0.0 && t.coord_pt.is_finite()));

        let (doc, mut graph) = xy_doc_with_points(&[(1.0, 5e-324), (2.0, f64::MAX)]);
        graph.spec.y_axis = spec;
        let mut chrome_scene = ChartScene::new("chrome", "Extreme Log", 420.0, 300.0, None);
        let opts = AxisRenderOptions { y_data_bounds: Some((5e-324, f64::MAX)), ..AxisRenderOptions::default() };
        let _ = render_graph_chrome(&graph.spec, &doc, &mut chrome_scene, &opts);
        assert_all_primitives_finite(&chrome_scene);

        let compiled = compile_guarded(doc, graph);
        assert_all_primitives_finite(&compiled);
    }

    // 2. Tiny `major_tick_interval` emits `major_tick_interval_capped` across BOTH `resolve_axes_and_ticks` AND `compile_graph` (XY, Column, Survival).
    let mut capped_y_spec = AxisSpec::linear("Y Capped");
    capped_y_spec.segments = vec![AxisSegment { min: Some(0.0), max: Some(100.0), major_tick_interval: Some(1e-12), ..AxisSegment::default() }];

    let (xy_doc, mut xy_graph) = xy_doc_with_points(&[(1.0, 10.0), (2.0, 90.0)]);
    xy_graph.spec.y_axis = capped_y_spec.clone();
    let mut chrome_spec = xy_graph.spec.clone();
    chrome_spec.canvas_size_pt = SizePt::new(420.0, 300.0);
    let mut scales_scene = ChartScene::new("scales-cap", "Cap", 420.0, 300.0, None);
    let compiled_axes = resolve_axes_and_ticks(&chrome_spec, &mut scales_scene, &AxisRenderOptions::default());
    assert!(
        scales_scene.warnings.iter().any(|w| w.code == "major_tick_interval_capped"),
        "scales::resolve_axes_and_ticks must warn on tiny major_tick_interval: {:?}",
        scales_scene.warnings
    );
    let scales_y_majors: Vec<f64> = compiled_axes.y_ticks.iter().filter(|t| t.kind == TickKind::Major).map(|t| t.value).collect();
    assert!(!scales_y_majors.is_empty() && scales_y_majors.len() <= MAX_TICKS_PER_SEGMENT);

    let xy_scene = compile_guarded(xy_doc, xy_graph);
    assert!(xy_scene.warnings.iter().any(|w| w.code == "major_tick_interval_capped"));
    assert_all_primitives_finite(&xy_scene);

    let mut col_doc = ProjectDocument::new("S4B-Col").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).unwrap();
    for (r, v) in [10.0, 50.0, 90.0].into_iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let col_ds = DataSheet::new("d-col", "Col", TableArchetype::Column(col)).unwrap();
    col_doc.add_data_sheet(col_ds.clone()).unwrap();
    let mut col_graph = GraphSheet::new_uninitialized_for_data("g-col", &col_ds).unwrap();
    col_graph.initialized = true;
    col_graph.spec.y_axis = capped_y_spec.clone();
    let col_scene = compile_guarded(col_doc, col_graph);
    assert!(col_scene.warnings.iter().any(|w| w.code == "major_tick_interval_capped"));
    assert_all_primitives_finite(&col_scene);

    let mut surv_doc = ProjectDocument::new("S4B-Surv").unwrap();
    let mut surv = SurvivalTable::new(1, 3).unwrap();
    for (r, (t, s)) in [(2.0, 1.0), (5.0, 1.0), (10.0, 1.0)].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(s);
    }
    let surv_ds = DataSheet::new("d-surv", "Surv", TableArchetype::Survival(surv)).unwrap();
    surv_doc.add_data_sheet(surv_ds.clone()).unwrap();
    let mut surv_graph = GraphSheet::new_uninitialized_for_data("g-surv", &surv_ds).unwrap();
    surv_graph.initialized = true;
    surv_graph.spec.y_axis = capped_y_spec;
    let surv_scene = compile_guarded(surv_doc, surv_graph);
    assert!(surv_scene.warnings.iter().any(|w| w.code == "major_tick_interval_capped"));
    assert_all_primitives_finite(&surv_scene);

    // 3. Reversed explicit range `[100.0, 0.0]` preserved on `ResolvedAxis::resolve` and `compile_graph`.
    let mut rev_spec = AxisSpec::linear("Reversed X");
    rev_spec.segments = vec![AxisSegment { min: Some(100.0), max: Some(0.0), ..AxisSegment::default() }];
    let mut rev_warnings = Vec::new();
    let rev_resolved = ResolvedAxis::resolve(&rev_spec, AxisOrientation::Horizontal, plot_rect, None, &mut rev_warnings);
    assert!((rev_resolved.segments[0].domain_min - 100.0).abs() < 1e-12);
    assert!((rev_resolved.segments[0].domain_max - 0.0).abs() < 1e-12);
    let rev_pt_high = rev_resolved.map_value(100.0).unwrap().coord_pt;
    let rev_pt_low = rev_resolved.map_value(0.0).unwrap().coord_pt;
    assert!(rev_pt_high < rev_pt_low, "reversed horizontal axis must map 100.0 left of 0.0");

    // 4. Extreme Probit/Logit tails & mixed-sign Reciprocal guards through `ResolvedAxis::resolve` + `compute_axis_ticks_with_warnings`.
    for prob_scale in [AxisScale::Probit, AxisScale::Logit(LogitDomain::Percent)] {
        let mut pspec = AxisSpec::linear("Prob");
        pspec.segments = vec![AxisSegment { scale: prob_scale, ..AxisSegment::default() }];
        let mut pwarn = Vec::new();
        let pr = ResolvedAxis::resolve(&pspec, AxisOrientation::Vertical, plot_rect, Some((0.005, 99.995)), &mut pwarn);
        assert!(pr.map_value(0.005).is_some() && pr.map_value(99.995).is_some(), "{prob_scale:?} extreme tail points must stay in resolved domain");
        let pticks = compute_axis_ticks_with_warnings(
            &pr,
            TickComputeConfig { tick_format: pspec.tick_format, policy: &policy, auto_log_minor_ticks: false, categorical_ticks: None },
            &mut pwarn,
        );
        assert!(pticks.len() >= 2 && pticks.iter().all(|t| t.coord_pt.is_finite()));
    }

    let mut recip_spec = AxisSpec::linear("Recip Mixed");
    recip_spec.segments = vec![AxisSegment { scale: AxisScale::Reciprocal, min: Some(-5.0), max: Some(5.0), ..AxisSegment::default() }];
    let mut rwarn = Vec::new();
    let rr = ResolvedAxis::resolve(&recip_spec, AxisOrientation::Vertical, plot_rect, None, &mut rwarn);
    assert!(rwarn.iter().any(|w| w.code == "invalid_axis_domain"), "expected invalid_axis_domain for mixed-sign reciprocal: {rwarn:?}");
    let seg = &rr.segments[0];
    assert!(seg.domain_min != 0.0 && seg.domain_max != 0.0 && seg.domain_min.signum() == seg.domain_max.signum());

    // 5. Oversized `gap_width_pt` on multi-segment axis capped at 25% in BOTH `scales::ResolvedAxis::resolve` and `compile_graph`.
    let mut gap_spec = AxisSpec::linear("Broken Gap");
    gap_spec.gap_width_pt = 999.0;
    gap_spec.segments = vec![
        AxisSegment { min: Some(0.0), max: Some(10.0), length_share: 1.0, ..AxisSegment::default() },
        AxisSegment { min: Some(50.0), max: Some(100.0), length_share: 1.0, ..AxisSegment::default() },
    ];
    let mut gwarn = Vec::new();
    let gr = ResolvedAxis::resolve(&gap_spec, AxisOrientation::Horizontal, RectPt::new(40.0, 20.0, 200.0, 120.0), None, &mut gwarn);
    let gap_pt = (gr.breaks[0].gap_end_pt - gr.breaks[0].gap_start_pt).abs();
    assert!(gap_pt <= 50.0 + 1e-9, "gap width must be capped at 25% of span (50pt), got {gap_pt}");
    assert!(gr.segments.iter().all(|s| (s.span_end_pt - s.span_start_pt).abs() > 10.0));

    let (gap_doc, mut gap_graph) = xy_doc_with_points(&[(2.0, 1.0), (80.0, 2.0)]);
    gap_graph.spec.x_axis = gap_spec;
    let gap_scene = compile_guarded(gap_doc, gap_graph);
    assert_all_primitives_finite(&gap_scene);
    assert_eq!(gap_scene.plotted_points.len(), 2);
}

#[test]
fn r1_s08_10_negative_reciprocal_error_bar_bounds_stay_in_domain() {
    // Forward negative Reciprocal mapping [-10.0, -1.0]: zero-crossing upper error bound (+1.0 / 0.0) must clamp to -1.0, NOT +1e-12 (~1e14 pt outlier).
    let neg_map = AxisMapping { min: -10.0, max: -1.0, scale: AxisScale::Reciprocal };
    for upper in [0.0, 1.0, 25.0] {
        let mut diag = ScaleDiagnostics::default();
        let mapped = neg_map.map_error_bar(-5.0, -9.0, upper, 0.0, 300.0, &mut diag).expect("valid negative reciprocal center");
        assert!(mapped.upper_clipped, "zero-crossing upper={upper} must report upper_clipped");
        assert!(!mapped.lower_clipped, "in-domain lower=-9.0 should not be clipped");
        assert_eq!(diag.log_error_bars_clamped, 0);
        assert!(diag.boundary_clamped >= 1);
        assert!(
            mapped.center_pt.is_finite()
                && mapped.lower_pt.is_finite()
                && mapped.upper_pt.is_finite()
                && mapped.lower_pt >= 0.0
                && mapped.upper_pt <= 300.0 + 1e-9,
            "negative reciprocal mapped pts must stay inside [0, 300], got {mapped:?}"
        );
        assert!((mapped.upper_pt - 300.0).abs() < 1e-6, "expected clamp to -1.0 boundary at 300pt, got {}", mapped.upper_pt);
    }

    // Reversed negative Reciprocal domain [-1.0, -10.0] also clamps to -1.0 boundary cleanly.
    let rev_neg_map = AxisMapping { min: -1.0, max: -10.0, scale: AxisScale::Reciprocal };
    let mut rev_diag = ScaleDiagnostics::default();
    let rev_mapped = rev_neg_map.map_error_bar(-5.0, -9.0, 2.0, 0.0, 300.0, &mut rev_diag).expect("reversed negative reciprocal");
    assert!(rev_mapped.upper_clipped);
    assert!((rev_mapped.upper_pt - 0.0).abs() < 1e-6, "reversed negative reciprocal must clamp -1.0 to 0pt, got {}", rev_mapped.upper_pt);

    // Positive Reciprocal [1.0, 10.0] crossing into non-positive clamps cleanly to min=1.0, NOT positive_floor=1e-12.
    let pos_map = AxisMapping { min: 1.0, max: 10.0, scale: AxisScale::Reciprocal };
    let mut pos_diag = ScaleDiagnostics::default();
    let pos_mapped = pos_map.map_error_bar(5.0, -2.0, 8.0, 0.0, 300.0, &mut pos_diag).expect("positive reciprocal");
    assert!(pos_mapped.lower_clipped);
    assert!((pos_mapped.lower_pt - 0.0).abs() < 1e-6, "positive reciprocal must clamp to domain bound 1.0 at 0pt, got {}", pos_mapped.lower_pt);

    // Segmented `ResolvedAxis::map_error_bar` on negative Reciprocal [-10.0, -1.0] must keep spans finite and inside plot span.
    let mut ax_spec = AxisSpec::linear("Negative 1/Y");
    ax_spec.segments = vec![AxisSegment { scale: AxisScale::Reciprocal, min: Some(-10.0), max: Some(-1.0), ..AxisSegment::default() }];
    let mut warnings = Vec::new();
    let resolved = ResolvedAxis::resolve(&ax_spec, AxisOrientation::Vertical, RectPt::new(40.0, 20.0, 240.0, 200.0), None, &mut warnings);
    let mut seg_diag = ScaleDiagnostics::default();
    let seg_eb = resolved.map_error_bar(-5.0, -9.0, 3.0, &mut seg_diag).expect("segmented negative reciprocal error bar");
    assert!(seg_eb.upper_clipped && seg_eb.spans.len() == 1);
    let sp = seg_eb.spans[0];
    assert!(sp.start_pt.is_finite() && sp.end_pt.is_finite() && sp.start_pt >= 20.0 && sp.end_pt <= 220.0 && sp.start_pt <= 220.0);

    // End-to-end XY compilation with negative pre-summarized point & zero-crossing SD on negative Reciprocal Y axis.
    let mut doc = ProjectDocument::new("S4B-NegRecip").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::MeanSdN, 1, 2).unwrap();
    xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy.data_sets[0].cells[0] = vec![Cell::number(-5.0), Cell::number(7.0), Cell::number(4.0)];
    xy.set_x_cell(1, Cell::number(2.0)).unwrap();
    xy.data_sets[0].cells[1] = vec![Cell::number(-8.0), Cell::number(1.0), Cell::number(4.0)];
    let ds = DataSheet::new("d-nr", "NegRecip", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data("g-nr", &ds).unwrap();
    gs.initialized = true;
    gs.spec.series[0].y_error_bars = Some(ErrorBarSpec { metric: ErrorBarMetric::Sd, direction: ErrorBarDirection::Both, ..ErrorBarSpec::default() });
    gs.spec.y_axis.segments = vec![AxisSegment { scale: AxisScale::Reciprocal, min: Some(-10.0), max: Some(-1.0), ..AxisSegment::default() }];
    let scene = compile_guarded(doc, gs);
    assert_all_primitives_finite(&scene);
    for prim in &scene.primitives {
        if let ScenePrimitive::Path { path, .. } = prim {
            for cmd in &path.commands {
                if let PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) = cmd {
                    assert!(pt.x.abs() < 5000.0 && pt.y.abs() < 5000.0, "huge outlier coordinate on negative reciprocal error bar: {pt:?}");
                }
            }
        }
    }
}

#[test]
fn r1_s08_11_malformed_dendrogram_tabs_do_not_panic_under_compile_graph() {
    let poisoned_rows = vec![
        vec![
            ResultCell::Number { value: -1e300, prefix: None, units: None },
            ResultCell::Number { value: 1e300, prefix: None, units: None },
            ResultCell::Number { value: 0.4, prefix: None, units: None },
            ResultCell::Number { value: 2.0, prefix: None, units: None },
        ],
        vec![
            ResultCell::Number { value: -1.5, prefix: None, units: None },
            ResultCell::Number { value: isize::MIN as f64, prefix: None, units: None },
            ResultCell::Number { value: f64::NAN, prefix: None, units: None },
            ResultCell::Number { value: 3.0, prefix: None, units: None },
        ],
    ];

    // 1. CorrelationMatrix heatmap with poisoned `hierarchical-dendrogram` tab.
    let mut mv_corr = MultipleVariablesTable::new(3, 3).unwrap();
    for c in 0..3 {
        mv_corr.variables[c].title = format!("V{}", c + 1);
        mv_corr.variables[c].role = VariableRole::Continuous;
        for r in 0..3 {
            mv_corr.variables[c].cells[r] = Cell::number(((r * 3 + c) as f64) + 1.0);
        }
    }
    let ds_corr = DataSheet::new("d-corr", "Corr", TableArchetype::MultipleVariables(mv_corr)).unwrap();
    let corr_spec = AnalysisSpec::new(
        SheetId::new("res-corr"),
        SheetId::new("d-corr"),
        AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }),
    )
    .unwrap();
    let mut corr_res = ResultSheet::new(corr_spec).unwrap();
    corr_res.status = ResultStatus::UpToDate;
    corr_res.tabs.push(ResultTab {
        id: String::from("corr"),
        title: String::from("Correlation"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("V1"), String::from("V2"), String::from("V3")],
        row_labels: vec![String::from("V1"), String::from("V2"), String::from("V3")],
        rows: vec![
            vec![
                ResultCell::Number { value: 1.0, prefix: None, units: None },
                ResultCell::Number { value: 0.4, prefix: None, units: None },
                ResultCell::Number { value: -0.2, prefix: None, units: None },
            ],
            vec![
                ResultCell::Number { value: 0.4, prefix: None, units: None },
                ResultCell::Number { value: 1.0, prefix: None, units: None },
                ResultCell::Number { value: 0.7, prefix: None, units: None },
            ],
            vec![
                ResultCell::Number { value: -0.2, prefix: None, units: None },
                ResultCell::Number { value: 0.7, prefix: None, units: None },
                ResultCell::Number { value: 1.0, prefix: None, units: None },
            ],
        ],
        footnotes: Vec::new(),
    });
    corr_res.tabs.push(ResultTab {
        id: String::from("hierarchical-dendrogram"),
        title: String::from("Dendrogram"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Left"), String::from("Right"), String::from("Distance"), String::from("Size")],
        row_labels: vec![String::from("Step 1"), String::from("Step 2")],
        rows: poisoned_rows.clone(),
        footnotes: Vec::new(),
    });
    let mut corr_doc = ProjectDocument::new("Corr Poisoned Dendro").unwrap();
    corr_doc.add_data_sheet(ds_corr.clone()).unwrap();
    corr_doc.add_result_sheet(corr_res).unwrap();
    let mut corr_graph = GraphSheet::new_uninitialized_for_data("g-corr", &ds_corr).unwrap();
    corr_graph.initialized = true;
    corr_graph.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    corr_graph.spec.primary_source_sheet_id = SheetId::new("res-corr");
    let corr_scene = compile_guarded(corr_doc, corr_graph);
    assert_all_primitives_finite(&corr_scene);

    // 2. Clustering heatmap with 130 rows (> 128 internal clustering cap) and poisoned 129-row `hierarchical-dendrogram` tab.
    let n_rows = 130usize;
    let mut mv_clust = MultipleVariablesTable::new(3, n_rows).unwrap();
    mv_clust.variables[0].role = VariableRole::Label;
    mv_clust.variables[1].role = VariableRole::Continuous;
    mv_clust.variables[2].role = VariableRole::Continuous;
    for r in 0..n_rows {
        mv_clust.variables[0].cells[r] = Cell::text(format!("R{r}"));
        mv_clust.variables[1].cells[r] = Cell::number(r as f64);
        mv_clust.variables[2].cells[r] = Cell::number((r as f64) * 2.0 + 1.0);
    }
    let ds_clust = DataSheet::new("d-cl", "Cluster", TableArchetype::MultipleVariables(mv_clust)).unwrap();
    let cl_spec = AnalysisSpec::new(
        SheetId::new("res-cl"),
        SheetId::new("d-cl"),
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: None,
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: None,
        }),
    )
    .unwrap();
    let mut cl_res = ResultSheet::new(cl_spec).unwrap();
    cl_res.status = ResultStatus::UpToDate;
    let mut bad_rows = Vec::with_capacity(n_rows - 1);
    bad_rows.push(poisoned_rows[0].clone());
    for step in 1..(n_rows - 1) {
        bad_rows.push(vec![
            ResultCell::Number { value: -((step + 1) as f64), prefix: None, units: None },
            ResultCell::Number { value: (step - 1) as f64, prefix: None, units: None },
            ResultCell::Number { value: 1.0, prefix: None, units: None },
            ResultCell::Number { value: (step + 2) as f64, prefix: None, units: None },
        ]);
    }
    cl_res.tabs.push(ResultTab {
        id: String::from("hierarchical-dendrogram"),
        title: String::from("Dendrogram"),
        kind: ResultTabKind::MainSummary,
        column_headers: Vec::new(),
        row_labels: Vec::new(),
        rows: bad_rows,
        footnotes: Vec::new(),
    });
    let mut cl_doc = ProjectDocument::new("Cluster Poisoned Dendro").unwrap();
    cl_doc.add_data_sheet(ds_clust.clone()).unwrap();
    cl_doc.add_result_sheet(cl_res).unwrap();
    let mut cl_graph = GraphSheet::new_uninitialized_for_data("g-cl", &ds_clust).unwrap();
    cl_graph.initialized = true;
    cl_graph.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    let cl_scene = compile_guarded(cl_doc, cl_graph);
    assert_all_primitives_finite(&cl_scene);
}
