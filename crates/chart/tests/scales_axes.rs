#![allow(clippy::float_cmp, clippy::cast_lossless)]

use curvebook_chart::scales::{
    AxisFrameMode, AxisMapping, AxisOrientation, AxisRenderOptions, CategoricalTick, LOG_POSITIVE_FLOOR_DEFAULT, LegendEntry, MAX_TICKS_PER_SEGMENT,
    ResolvedAxis, ScaleDiagnostics, TickComputeConfig, TickKind, compute_axis_ticks, compute_plot_rect, format_tick_rich,
    legend_entries_from_graph_spec, render_annotations, render_axes_and_frame, render_graph_chrome, render_legend, resolve_template_hooks,
    scale_inverse, scale_transform, split_polyline_across_segments,
};
use curvebook_chart::{ChartScene, FillPaint, PathCmd, PointPt, RectPt, ScenePrimitive, TextAnchor};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AnnotationShapeKind, AxisSegment, AxisSpec, AxisThreshold, BreakOrnament, CiBound, ConstantValue, DataSheet,
    DescriptiveSpec, EquationDisplayMode, FramePreset, GraphAnnotation, GraphSheet, InfoSheet, LegendSpec, LogitDomain, NonlinearFitSpec,
    ProjectDocument, RectPt as DocRectPt, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SizePt,
    SubcolumnMode, TableArchetype, TickLabelFormat, XyTable,
};
use curvebook_math::{AxisScale, dist::norm_ppf};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, Rgba8};
use curvebook_text::{LocaleNumberPolicy, SpanScript};
use proptest::prelude::*;

#[allow(clippy::expect_used)]
fn build_sample_doc_and_graph() -> (ProjectDocument, GraphSheet) {
    let mut doc = ProjectDocument::default();
    let mut info = InfoSheet::new_preseeded(SheetId::new("info-1"), "Assay Info").expect("info sheet");
    info.set_constant("Experiment ID", ConstantValue::Text(String::from("EXP-42")), None).expect("set constant");
    let mut data =
        DataSheet::new(SheetId::new("data-1"), "Dose Response", TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(3), 2, 6).expect("xy")))
            .expect("data");
    data.info_sheet_id = Some(info.id.clone());

    let mut result = ResultSheet::new(
        AnalysisSpec::new(SheetId::new("fit-1"), data.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("analysis"),
    )
    .expect("result");
    result.status = ResultStatus::UpToDate;
    result.scalars.push(ResultScalarEntry { key: String::from("IC50"), data_set_index: Some(0), value: 2.5, units: Some(String::from("nM")) });
    result.tabs.push(ResultTab {
        id: String::from("main"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Estimate"), String::from("95% CI"), String::from("P Value")],
        row_labels: vec![String::from("IC50"), String::from("Hill")],
        rows: vec![
            vec![
                ResultCell::Number { value: 2.5, prefix: None, units: Some(String::from("nM")) },
                ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.8), upper: CiBound::Finite(3.4) },
                ResultCell::PValue { raw: 0.0004, formatted: String::from("0.0004"), stars: String::from("***") },
            ],
            vec![
                ResultCell::Text(String::from("1.12")),
                ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Finite(2.0) },
                ResultCell::Boolean(true),
            ],
        ],
        footnotes: Vec::new(),
    });

    let graph = GraphSheet::new_uninitialized_for_data(SheetId::new("graph-1"), &data).expect("graph");
    doc.add_info_sheet(info).expect("add info");
    doc.add_data_sheet(data).expect("add data");
    doc.add_result_sheet(result).expect("add result");
    doc.add_graph_sheet(graph.clone()).expect("add graph");
    (doc, graph)
}

fn line_count_in_scene(scene: &ChartScene) -> usize {
    scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { path, stroke: Some(_), .. } if path.commands.len() >= 2)).count()
}

#[test]
fn forward_and_inverse_mapping_roundtrip_all_seven_scales() {
    let cases = [
        (AxisScale::Linear, -25.0, 75.0, 12.5),
        (AxisScale::Log10, 1e-4, 1e4, 0.25),
        (AxisScale::Log2, 0.25, 64.0, 8.0),
        (AxisScale::Ln, 0.1, 100.0, std::f64::consts::E),
        (AxisScale::Logit(LogitDomain::Proportion), 0.02, 0.98, 0.25),
        (AxisScale::Logit(LogitDomain::Percent), 2.0, 98.0, 75.0),
        (AxisScale::Probit, 0.5, 99.5, 84.1344746068543),
        (AxisScale::Reciprocal, 1.0, 10.0, 2.5),
    ];

    for (scale, min, max, sample) in cases {
        let mapping = AxisMapping { min, max, scale };
        // Horizontal span (left -> right)
        let x_pt = mapping.map_value(sample, 40.0, 340.0).expect("valid horizontal map");
        let x_back = mapping.unmap_coord(x_pt, 40.0, 340.0).expect("valid horizontal unmap");
        assert!((x_back - sample).abs() < 1e-8, "{scale:?} horizontal roundtrip: {sample} -> {x_pt} -> {x_back}");

        // Vertical span (bottom -> top)
        let y_pt = mapping.map_value(sample, 220.0, 20.0).expect("valid vertical map");
        let y_back = mapping.unmap_coord(y_pt, 220.0, 20.0).expect("valid vertical unmap");
        assert!((y_back - sample).abs() < 1e-8, "{scale:?} vertical roundtrip: {sample} -> {y_pt} -> {y_back}");
    }

    // Probit explicitly uses curvebook_math inverse normal CDF (Phi^{-1}(0.5) == 0, Phi^{-1}(0.841345) ~ +1)
    let probit_map = AxisMapping { min: 5.0, max: 95.0, scale: AxisScale::Probit };
    let mid_pt = probit_map.map_value(50.0, 0.0, 100.0).expect("probit 50%");
    assert!((mid_pt - 50.0).abs() < 1e-12);
    let expected_z = norm_ppf(0.841344746068543).expect("norm_ppf");
    assert!((scale_transform(AxisScale::Probit, 84.1344746068543).expect("probit") - expected_z).abs() < 1e-12);
    assert!((scale_inverse(AxisScale::Probit, 0.0).expect("probit inv") - 50.0).abs() < 1e-12);

    // Non-finite inputs never produce NaN/Inf
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let m = AxisMapping { min: 0.0, max: 10.0, scale: AxisScale::Linear };
        assert!(m.map_value(bad, 0.0, 100.0).is_none());
        assert!(m.unmap_coord(bad, 0.0, 100.0).is_none());
    }
}

#[test]
fn log_scales_emit_warnings_on_non_positive_and_clamp_error_bars_to_floor() {
    for scale in [AxisScale::Log10, AxisScale::Log2, AxisScale::Ln] {
        let mapping = AxisMapping { min: 0.1, max: 100.0, scale };
        let mut warnings = Vec::new();

        assert!(mapping.map_value_checked(0.0, 200.0, 20.0, "Y", &mut warnings).is_none());
        assert!(mapping.map_value_checked(-5.0, 200.0, 20.0, "Y", &mut warnings).is_none());
        assert_eq!(warnings.iter().filter(|w| w.code == "non_positive_on_log_axis").count(), 2);

        let mut eb_warnings = Vec::new();
        let eb = mapping.map_error_bar_checked(5.0, -3.0, 15.0, (200.0, 20.0), "Y", &mut eb_warnings).expect("clamped error bar");
        assert!(eb.lower_clipped);
        assert!(!eb.upper_clipped);
        assert!((eb.lower_pt - 200.0).abs() < 1e-9, "lower error bar must clamp to floor span_start=200.0");
        assert!(eb.upper_pt < eb.center_pt && eb.center_pt < eb.lower_pt);
        assert!(eb_warnings.iter().any(|w| w.code == "log_error_bar_clamped"));
    }

    // Fallback floor when mapping itself had invalid non-positive min
    let bad_min = AxisMapping { min: -1.0, max: 10.0, scale: AxisScale::Log10 };
    assert_eq!(bad_min.positive_floor(), 10.0);
    let both_bad = AxisMapping { min: -1.0, max: 0.0, scale: AxisScale::Log10 };
    assert_eq!(both_bad.positive_floor(), LOG_POSITIVE_FLOOR_DEFAULT);
}

#[test]
fn logit_probit_and_reciprocal_domain_guards_emit_structured_warnings() {
    let mut warnings = Vec::new();
    let logit_prop = AxisMapping { min: 0.05, max: 0.95, scale: AxisScale::Logit(LogitDomain::Proportion) };
    assert!(logit_prop.map_value_checked(0.0, 0.0, 100.0, "X", &mut warnings).is_none());
    assert!(logit_prop.map_value_checked(1.0, 0.0, 100.0, "X", &mut warnings).is_none());
    assert!(logit_prop.map_value_checked(1.2, 0.0, 100.0, "X", &mut warnings).is_none());
    assert_eq!(warnings.iter().filter(|w| w.code == "out_of_domain_logit").count(), 3);

    let eb_logit = logit_prop.map_error_bar_checked(0.1, -0.2, 1.2, (0.0, 100.0), "X", &mut warnings).expect("clamped logit eb");
    assert!(eb_logit.lower_clipped && eb_logit.upper_clipped);
    assert!(eb_logit.lower_pt.is_finite() && eb_logit.upper_pt.is_finite());

    let probit = AxisMapping { min: 1.0, max: 99.0, scale: AxisScale::Probit };
    assert!(probit.map_value_checked(0.0, 0.0, 100.0, "Y", &mut warnings).is_none());
    assert!(probit.map_value_checked(100.0, 0.0, 100.0, "Y", &mut warnings).is_none());
    assert_eq!(warnings.iter().filter(|w| w.code == "out_of_domain_probit").count(), 2);

    let recip = AxisMapping { min: 1.0, max: 10.0, scale: AxisScale::Reciprocal };
    assert!(recip.map_value_checked(0.0, 0.0, 100.0, "X", &mut warnings).is_none());
    assert!(warnings.iter().any(|w| w.code == "zero_on_reciprocal_axis"));
    let eb_recip = recip.map_error_bar_checked(2.0, -1.0, 4.0, (0.0, 100.0), "X", &mut warnings).expect("guarded reciprocal eb");
    assert!(eb_recip.lower_clipped);
}

#[test]
fn discontinuous_two_and_three_segment_axes_and_gap_geometry() {
    let mut x_spec = AxisSpec::linear("Time (h)");
    x_spec.gap_width_pt = 12.0;
    x_spec.segments = vec![
        AxisSegment {
            min: Some(0.0),
            max: Some(10.0),
            scale: AxisScale::Linear,
            length_share: 1.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        },
        AxisSegment {
            min: Some(50.0),
            max: Some(100.0),
            scale: AxisScale::Linear,
            length_share: 3.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        },
    ];
    let plot = RectPt::new(50.0, 30.0, 212.0, 160.0);
    let mut warnings = Vec::new();
    let x_ax = ResolvedAxis::resolve(&x_spec, AxisOrientation::Horizontal, plot, None, &mut warnings);
    assert!(warnings.is_empty());
    assert_eq!(x_ax.segments.len(), 2);
    assert_eq!(x_ax.breaks.len(), 1);
    assert!((x_ax.segments[0].length_fraction - 0.25).abs() < 1e-12);
    assert!((x_ax.segments[1].length_fraction - 0.75).abs() < 1e-12);
    // Usable width = 212 - 12 = 200 -> seg0 = 50pt [50..100], gap [100..112], seg1 = 150pt [112..262]
    assert!((x_ax.segments[0].span_start_pt - 50.0).abs() < 1e-9);
    assert!((x_ax.segments[0].span_end_pt - 100.0).abs() < 1e-9);
    assert!((x_ax.breaks[0].gap_start_pt - 100.0).abs() < 1e-9);
    assert!((x_ax.breaks[0].gap_end_pt - 112.0).abs() < 1e-9);
    assert!((x_ax.segments[1].span_start_pt - 112.0).abs() < 1e-9);
    assert!((x_ax.segments[1].span_end_pt - 262.0).abs() < 1e-9);

    assert_eq!(x_ax.map_value(5.0).map(|m| (m.segment_index, m.coord_pt)), Some((0, 75.0)));
    assert!(x_ax.map_value(30.0).is_none()); // Inside break domain!
    assert_eq!(x_ax.map_value(75.0).map(|m| (m.segment_index, m.coord_pt)), Some((1, 187.0)));
    assert!(x_ax.unmap_coord(106.0).is_none()); // Inside physical gap!
    assert!((x_ax.unmap_coord(187.0).expect("seg1 unmap") - 75.0).abs() < 1e-9);

    // 3-segment vertical axis with mixed scales (Linear + Log10 + Linear)
    let mut y_spec = AxisSpec::linear("Response");
    y_spec.gap_width_pt = 10.0;
    y_spec.segments = vec![
        AxisSegment {
            min: Some(0.0),
            max: Some(10.0),
            scale: AxisScale::Linear,
            length_share: 1.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        },
        AxisSegment {
            min: Some(10.0),
            max: Some(1000.0),
            scale: AxisScale::Log10,
            length_share: 2.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        },
        AxisSegment {
            min: Some(2000.0),
            max: Some(4000.0),
            scale: AxisScale::Linear,
            length_share: 1.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        },
    ];
    let y_ax = ResolvedAxis::resolve(&y_spec, AxisOrientation::Vertical, RectPt::new(40.0, 20.0, 200.0, 180.0), None, &mut warnings);
    assert_eq!(y_ax.segments.len(), 3);
    assert_eq!(y_ax.breaks.len(), 2);
    // Usable height = 180 - 20 = 160 -> [40pt, 80pt, 40pt] going bottom->top (200 -> 160, gap 160->150, 150 -> 70, gap 70->60, 60 -> 20)
    assert!((y_ax.segments[0].span_start_pt - 200.0).abs() < 1e-9);
    assert!((y_ax.segments[0].span_end_pt - 160.0).abs() < 1e-9);
    assert!((y_ax.segments[1].span_start_pt - 150.0).abs() < 1e-9);
    assert!((y_ax.segments[1].span_end_pt - 70.0).abs() < 1e-9);
    assert!((y_ax.segments[2].span_start_pt - 60.0).abs() < 1e-9);
    assert!((y_ax.segments[2].span_end_pt - 20.0).abs() < 1e-9);
    // 100 in Log10 [10, 1000] is at midpoint 110.0
    assert!((y_ax.map_value(100.0).expect("log seg1").coord_pt - 110.0).abs() < 1e-9);

    // Invalid 0 or >3 segment specs degrade safely with warnings
    let mut bad_empty = AxisSpec::linear("Empty");
    bad_empty.segments.clear();
    let mut w2 = Vec::new();
    let ax_empty = ResolvedAxis::resolve(&bad_empty, AxisOrientation::Horizontal, plot, None, &mut w2);
    assert_eq!(ax_empty.segments.len(), 1);
    assert!(w2.iter().any(|w| w.code == "empty_axis_segments"));

    let mut bad_many = AxisSpec::linear("Many");
    bad_many.segments = vec![AxisSegment::default(); 5];
    let mut w3 = Vec::new();
    let ax_many = ResolvedAxis::resolve(&bad_many, AxisOrientation::Horizontal, plot, None, &mut w3);
    assert_eq!(ax_many.segments.len(), 3);
    assert!(w3.iter().any(|w| w.code == "too_many_axis_segments"));
}

#[test]
fn break_ornaments_emit_double_slash_zigzag_and_blank_gap() {
    let (doc, mut graph) = build_sample_doc_and_graph();
    graph.spec.x_axis.segments = vec![
        AxisSegment {
            min: Some(0.0),
            max: Some(10.0),
            scale: AxisScale::Linear,
            length_share: 1.0,
            major_tick_interval: Some(5.0),
            minor_ticks_per_interval: 0,
        },
        AxisSegment {
            min: Some(50.0),
            max: Some(100.0),
            scale: AxisScale::Linear,
            length_share: 1.0,
            major_tick_interval: Some(25.0),
            minor_ticks_per_interval: 0,
        },
    ];
    graph.spec.x_axis.gap_width_pt = 10.0;

    let opts = AxisRenderOptions::default();

    graph.spec.x_axis.break_ornament = BreakOrnament::BlankGap;
    let mut s_blank = ChartScene::new("b", "Blank", 360.0, 260.0, None);
    render_axes_and_frame(&graph.spec, &doc, &mut s_blank, &opts);

    graph.spec.x_axis.break_ornament = BreakOrnament::DoubleSlash;
    let mut s_slash = ChartScene::new("s", "Slash", 360.0, 260.0, None);
    render_axes_and_frame(&graph.spec, &doc, &mut s_slash, &opts);

    graph.spec.x_axis.break_ornament = BreakOrnament::Zigzag;
    let mut s_zig = ChartScene::new("z", "Zigzag", 360.0, 260.0, None);
    render_axes_and_frame(&graph.spec, &doc, &mut s_zig, &opts);

    let blank_lines = line_count_in_scene(&s_blank);
    let slash_lines = line_count_in_scene(&s_slash);
    let zig_lines = line_count_in_scene(&s_zig);

    assert_eq!(slash_lines, blank_lines + 2, "DoubleSlash should add 2 diagonal slash strokes");
    assert_eq!(zig_lines, blank_lines + 1, "Zigzag should add 1 multi-segment zigzag path");
    let has_zigzag_cmd = s_zig.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { path, .. } if path.commands.len() == 4));
    assert!(has_zigzag_cmd);
}

#[test]
fn segment_aware_polyline_and_interval_splitting_clips_cleanly_at_gaps() {
    let mut x_spec = AxisSpec::linear("X");
    x_spec.gap_width_pt = 12.0;
    x_spec.segments = vec![
        AxisSegment {
            min: Some(0.0),
            max: Some(10.0),
            scale: AxisScale::Linear,
            length_share: 1.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        },
        AxisSegment {
            min: Some(50.0),
            max: Some(100.0),
            scale: AxisScale::Linear,
            length_share: 1.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        },
    ];
    let mut y_spec = AxisSpec::linear("Y");
    y_spec.gap_width_pt = 10.0;
    y_spec.segments = vec![
        AxisSegment {
            min: Some(0.0),
            max: Some(10.0),
            scale: AxisScale::Linear,
            length_share: 1.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        },
        AxisSegment {
            min: Some(50.0),
            max: Some(100.0),
            scale: AxisScale::Linear,
            length_share: 1.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        },
    ];

    let plot = RectPt::new(40.0, 20.0, 212.0, 170.0);
    let mut warnings = Vec::new();
    let x_ax = ResolvedAxis::resolve(&x_spec, AxisOrientation::Horizontal, plot, None, &mut warnings);
    let y_ax = ResolvedAxis::resolve(&y_spec, AxisOrientation::Vertical, plot, None, &mut warnings);

    let mut diag = ScaleDiagnostics::default();
    let clipped = split_polyline_across_segments(&[(2.0, 2.0), (8.0, 8.0), (60.0, 60.0), (90.0, 90.0)], &x_ax, &y_ax, &mut diag);
    assert!(diag.is_clean());
    assert_eq!(clipped.len(), 2, "diagonal polyline should split cleanly into (0,0) and (1,1) cells");
    assert_eq!((clipped[0].x_segment_index, clipped[0].y_segment_index), (0, 0));
    assert_eq!((clipped[1].x_segment_index, clipped[1].y_segment_index), (1, 1));

    let x_gap_lo = x_ax.breaks[0].gap_start_pt;
    let x_gap_hi = x_ax.breaks[0].gap_end_pt;
    for cell in &clipped {
        for cmd in &cell.path.commands {
            let pt = match *cmd {
                PathCmd::MoveTo(p) | PathCmd::LineTo(p) => p,
                _ => continue,
            };
            assert!(pt.x <= x_gap_lo + 1e-6 || pt.x >= x_gap_hi - 1e-6, "coordinate {} entered X break gap [{x_gap_lo}, {x_gap_hi}]", pt.x);
        }
    }

    // Bar / interval split across Y break
    let bar_spans = y_ax.map_interval(0.0, 80.0);
    assert_eq!(bar_spans.len(), 2);
    assert!(!bar_spans[0].clipped_start && bar_spans[0].clipped_end);
    assert!(bar_spans[1].clipped_start && !bar_spans[1].clipped_end);
}

#[test]
fn auto_nice_ticks_and_log10_minor_cadence_2_to_9() {
    let plot = RectPt::new(40.0, 20.0, 280.0, 200.0);
    let policy = LocaleNumberPolicy::publication();
    let mut warnings = Vec::new();

    // Linear nice ticks + interior minor ticks
    let mut lin_spec = AxisSpec::linear("Linear");
    lin_spec.segments[0].min = Some(0.0);
    lin_spec.segments[0].max = Some(10.0);
    lin_spec.segments[0].minor_ticks_per_interval = 1;
    let lin_ax = ResolvedAxis::resolve(&lin_spec, AxisOrientation::Horizontal, plot, None, &mut warnings);
    let lin_ticks = compute_axis_ticks(
        &lin_ax,
        TickComputeConfig { tick_format: TickLabelFormat::Decimal, policy: &policy, auto_log_minor_ticks: true, categorical_ticks: None },
    );
    let lin_majors: Vec<f64> = lin_ticks.iter().filter(|t| t.kind == TickKind::Major).map(|t| t.value).collect();
    let lin_minors: Vec<f64> = lin_ticks.iter().filter(|t| t.kind == TickKind::Minor).map(|t| t.value).collect();
    assert_eq!(lin_majors, vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
    assert_eq!(lin_minors, vec![1.0, 3.0, 5.0, 7.0, 9.0]);

    // Log10 major powers of 10 + 2..9 * 10^k minor ticks across 3 decades [0.1, 100.0]
    let mut log10_spec = AxisSpec::linear("Conc");
    log10_spec.segments[0] = AxisSegment {
        min: Some(0.1),
        max: Some(100.0),
        scale: AxisScale::Log10,
        length_share: 1.0,
        major_tick_interval: None,
        minor_ticks_per_interval: 0,
    };
    let log10_ax = ResolvedAxis::resolve(&log10_spec, AxisOrientation::Horizontal, plot, None, &mut warnings);
    let log10_ticks = compute_axis_ticks(
        &log10_ax,
        TickComputeConfig {
            tick_format: TickLabelFormat::PowerOfTenSuperscript,
            policy: &policy,
            auto_log_minor_ticks: true,
            categorical_ticks: None,
        },
    );
    let log10_majors: Vec<f64> = log10_ticks.iter().filter(|t| t.kind == TickKind::Major).map(|t| t.value).collect();
    let log10_minors: Vec<f64> = log10_ticks.iter().filter(|t| t.kind == TickKind::Minor).map(|t| t.value).collect();
    assert_eq!(log10_majors.len(), 4); // 0.1, 1, 10, 100
    assert_eq!(log10_minors.len(), 24); // 3 decades * 8 minor ticks (2..=9)
    assert!((log10_minors[0] - 0.2).abs() < 1e-12);
    assert!((log10_minors[8] - 2.0).abs() < 1e-12);
    assert!((log10_minors[23] - 90.0).abs() < 1e-12);

    // Log2, Probit, Logit, Reciprocal ticks
    let mut log2_spec = AxisSpec::linear("Log2");
    log2_spec.segments[0] = AxisSegment {
        min: Some(1.0),
        max: Some(16.0),
        scale: AxisScale::Log2,
        length_share: 1.0,
        major_tick_interval: None,
        minor_ticks_per_interval: 0,
    };
    let log2_ax = ResolvedAxis::resolve(&log2_spec, AxisOrientation::Horizontal, plot, None, &mut warnings);
    let log2_ticks = compute_axis_ticks(
        &log2_ax,
        TickComputeConfig { tick_format: TickLabelFormat::Decimal, policy: &policy, auto_log_minor_ticks: true, categorical_ticks: None },
    );
    assert_eq!(log2_ticks.iter().map(|t| t.value).collect::<Vec<_>>(), vec![1.0, 2.0, 4.0, 8.0, 16.0]);

    let mut probit_spec = AxisSpec::linear("Probability (%)");
    probit_spec.segments[0] = AxisSegment {
        min: Some(1.0),
        max: Some(99.0),
        scale: AxisScale::Probit,
        length_share: 1.0,
        major_tick_interval: None,
        minor_ticks_per_interval: 0,
    };
    let probit_ax = ResolvedAxis::resolve(&probit_spec, AxisOrientation::Vertical, plot, None, &mut warnings);
    let probit_ticks = compute_axis_ticks(
        &probit_ax,
        TickComputeConfig { tick_format: TickLabelFormat::Decimal, policy: &policy, auto_log_minor_ticks: true, categorical_ticks: None },
    );
    assert!(probit_ticks.iter().any(|t| (t.value - 50.0).abs() < 1e-12));
    assert!(probit_ticks.iter().any(|t| (t.value - 95.0).abs() < 1e-12));

    let mut recip_spec = AxisSpec::linear("1/S");
    recip_spec.segments[0] = AxisSegment {
        min: Some(1.0),
        max: Some(10.0),
        scale: AxisScale::Reciprocal,
        length_share: 1.0,
        major_tick_interval: None,
        minor_ticks_per_interval: 0,
    };
    let recip_ax = ResolvedAxis::resolve(&recip_spec, AxisOrientation::Horizontal, plot, None, &mut warnings);
    let recip_ticks = compute_axis_ticks(
        &recip_ax,
        TickComputeConfig { tick_format: TickLabelFormat::Decimal, policy: &policy, auto_log_minor_ticks: true, categorical_ticks: None },
    );
    assert!(recip_ticks.len() >= 3);

    // Adversarial tiny major_tick_interval caps safely
    let mut adv_spec = AxisSpec::linear("Tiny Step");
    adv_spec.segments[0] = AxisSegment {
        min: Some(0.0),
        max: Some(1000.0),
        scale: AxisScale::Linear,
        length_share: 1.0,
        major_tick_interval: Some(1e-12),
        minor_ticks_per_interval: 5,
    };
    let adv_ax = ResolvedAxis::resolve(&adv_spec, AxisOrientation::Horizontal, plot, None, &mut warnings);
    let adv_ticks = compute_axis_ticks(
        &adv_ax,
        TickComputeConfig { tick_format: TickLabelFormat::Decimal, policy: &policy, auto_log_minor_ticks: false, categorical_ticks: None },
    );
    assert!(adv_ticks.len() <= MAX_TICKS_PER_SEGMENT * 2);
}

#[test]
fn tick_formatting_modes_and_rotated_labels_use_curvebook_text() {
    let policy = LocaleNumberPolicy::publication();
    let sup = format_tick_rich(1e-6, AxisScale::Log10, TickLabelFormat::PowerOfTenSuperscript, &policy);
    assert_eq!(sup.spans.len(), 2);
    assert_eq!(sup.spans[0].text, "10");
    assert_eq!(sup.spans[1].script, SpanScript::Superscript);
    assert_eq!(sup.spans[1].text, "\u{2212}6");

    let sci = format_tick_rich(0.0025, AxisScale::Linear, TickLabelFormat::ScientificE, &policy);
    assert_eq!(sci.plain_text(), "2.5e\u{2212}3");

    let antilog = format_tick_rich(-2.0, AxisScale::Linear, TickLabelFormat::Antilog, &policy);
    assert_eq!(antilog.plain_text(), "0.01");

    let si_micro = format_tick_rich(1e-6, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy);
    assert_eq!(si_micro.plain_text(), "1 \u{00B5}");
    let si_kilo = format_tick_rich(1500.0, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy);
    assert_eq!(si_kilo.plain_text(), "1.5 k");

    // Rotated categorical labels expand plot bottom margin and attach rotation_deg + TextAnchor
    let (doc, mut graph) = build_sample_doc_and_graph();
    graph.spec.x_axis.tick_format = TickLabelFormat::Categorical(curvebook_doc::CategoricalTickSource::ColumnTitles);
    graph.spec.x_axis.tick_angle_deg = 45.0;
    graph.spec.x_axis.segments[0].min = Some(-0.5);
    graph.spec.x_axis.segments[0].max = Some(2.5);

    let cats = CategoricalTick::from_labels(&[String::from("Vehicle Control"), String::from("Low Dose"), String::from("High Dose")]);
    let opts_flat = AxisRenderOptions { x_categories: Some(cats.clone()), ..AxisRenderOptions::default() };
    let mut opts_rot = AxisRenderOptions { x_categories: Some(cats), ..AxisRenderOptions::default() };
    let mut graph_flat = graph.spec.clone();
    graph_flat.x_axis.tick_angle_deg = 0.0;

    let rect_flat = compute_plot_rect(SizePt::new(360.0, 260.0), &graph_flat, &opts_flat);
    let rect_rot = compute_plot_rect(SizePt::new(360.0, 260.0), &graph.spec, &opts_rot);
    assert!(rect_rot.height < rect_flat.height, "45-deg rotated long labels should reserve taller bottom margin");

    opts_rot.frame_mode = None;
    let mut scene = ChartScene::new("rot", "Rotated", 360.0, 260.0, None);
    scene.plot_rect_pt = rect_rot;
    render_axes_and_frame(&graph.spec, &doc, &mut scene, &opts_rot);
    let rot_texts: Vec<&curvebook_chart::PositionedText> = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) if (t.rotation_deg - 45.0).abs() < 1e-6 => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(rot_texts.len(), 3);
    assert_eq!(rot_texts[0].anchor, TextAnchor::Start);
}

#[test]
fn all_frame_presets_render_expected_spines_ticks_and_origin_warnings() {
    let (doc, mut graph) = build_sample_doc_and_graph();
    graph.spec.x_axis.segments[0].min = Some(-5.0);
    graph.spec.x_axis.segments[0].max = Some(5.0);
    graph.spec.y_axis.segments[0].min = Some(-10.0);
    graph.spec.y_axis.segments[0].max = Some(10.0);

    let opts = AxisRenderOptions::default();

    // OpenL
    graph.spec.frame_preset = FramePreset::OpenL;
    let mut s_open = ChartScene::new("open", "OpenL", 360.0, 260.0, None);
    render_axes_and_frame(&graph.spec, &doc, &mut s_open, &opts);

    // PlainBoxed
    graph.spec.frame_preset = FramePreset::PlainBoxed;
    let mut s_box = ChartScene::new("box", "Box", 360.0, 260.0, None);
    render_axes_and_frame(&graph.spec, &doc, &mut s_box, &opts);
    assert_eq!(line_count_in_scene(&s_box), line_count_in_scene(&s_open) + 2);

    // TopRightTicks
    let opts_tr = AxisRenderOptions { frame_mode: Some(AxisFrameMode::TopRightTicks), ..AxisRenderOptions::default() };
    let mut s_tr = ChartScene::new("tr", "TopRightTicks", 360.0, 260.0, None);
    let c_tr = render_axes_and_frame(&graph.spec, &doc, &mut s_tr, &opts_tr);
    assert_eq!(line_count_in_scene(&s_tr), line_count_in_scene(&s_box) + c_tr.x_ticks.len() + c_tr.y_ticks.len());

    // OffsetRangeTufte
    graph.spec.frame_preset = FramePreset::OffsetRangeTufte;
    let mut s_off = ChartScene::new("off", "Offset", 360.0, 260.0, None);
    let c_off = render_axes_and_frame(&graph.spec, &doc, &mut s_off, &opts);
    assert_eq!(c_off.frame_mode, AxisFrameMode::OffsetAxes);
    assert!(line_count_in_scene(&s_off) > 2);

    // DualYSecondaryRight
    graph.spec.frame_preset = FramePreset::DualYSecondaryRight;
    let mut y2 = AxisSpec::linear("Secondary Y2");
    y2.segments[0].min = Some(0.0);
    y2.segments[0].max = Some(100.0);
    graph.spec.y2_axis = Some(y2);
    let mut s_y2 = ChartScene::new("y2", "DualY", 360.0, 260.0, None);
    let c_y2 = render_axes_and_frame(&graph.spec, &doc, &mut s_y2, &opts);
    assert!(c_y2.y2_axis.is_some());
    assert!(!c_y2.y2_ticks.is_empty());
    assert!(s_y2.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "Secondary Y2")));

    // OriginCrossing inside domain [-5, 5] x [-10, 10]
    graph.spec.frame_preset = FramePreset::OriginCrossing;
    let mut s_orig = ChartScene::new("orig", "Origin", 360.0, 260.0, None);
    render_axes_and_frame(&graph.spec, &doc, &mut s_orig, &opts);
    assert!(s_orig.warnings.is_empty());
    let expected_ox = s_orig.plot_rect_pt.x + 0.5 * s_orig.plot_rect_pt.width;
    assert!(s_orig.primitives.iter().any(|p| matches!(
        p,
        ScenePrimitive::Path { path, .. }
            if path.commands.first() == Some(&PathCmd::MoveTo(PointPt::new(expected_ox, s_orig.plot_rect_pt.bottom())))
    )));

    // OriginCrossing outside domain emits warning cleanly
    graph.spec.x_axis.segments[0].min = Some(1.0);
    graph.spec.x_axis.segments[0].max = Some(5.0);
    let mut s_orig_out = ChartScene::new("orig_out", "OriginOut", 360.0, 260.0, None);
    render_axes_and_frame(&graph.spec, &doc, &mut s_orig_out, &opts);
    assert!(s_orig_out.warnings.iter().any(|w| w.code == "origin_outside_x_domain"));

    // NoAxes emits zero axis primitives
    let opts_none = AxisRenderOptions { frame_mode: Some(AxisFrameMode::NoAxes), ..AxisRenderOptions::default() };
    let mut s_none = ChartScene::new("none", "NoAxes", 360.0, 260.0, None);
    render_axes_and_frame(&graph.spec, &doc, &mut s_none, &opts_none);
    assert!(s_none.primitives.is_empty());
}

#[test]
fn thresholds_shaded_zones_legends_and_all_graph_annotations_compile() {
    let (doc, mut graph) = build_sample_doc_and_graph();
    graph.spec.x_axis.title = String::from("Dose (EXP: {{info:Experiment ID}})");
    graph.spec.y_axis.title = String::from("IC<sub>50</sub> = {{result:fit-1:0:IC50}} nM");
    graph.spec.x_axis.segments[0].min = Some(0.0);
    graph.spec.x_axis.segments[0].max = Some(10.0);
    graph.spec.y_axis.segments[0].min = Some(0.0);
    graph.spec.y_axis.segments[0].max = Some(100.0);

    // Threshold line + shaded reference zone
    graph.spec.y_axis.thresholds.push(AxisThreshold {
        value: 50.0,
        shade_to: Some(80.0),
        color: Rgba8::rgb(213, 94, 0),
        dash_style: LineDashStyle::Dashed,
        stroke_width_pt: 1.2,
        label: Some(String::from("Target Window")),
    });

    // Legend
    graph.spec.legend = LegendSpec { visible: true, position_pt: None, bordered: true };

    // All GraphAnnotation variants
    graph.spec.annotations = vec![
        GraphAnnotation::TextBox {
            bounds_pt: DocRectPt::new(70.0, 40.0, 120.0, 34.0),
            markdown: String::from("<b>ID:</b> {{info:Experiment ID}}\n<i>IC<sub>50</sub></i> = {{result:fit-1:IC50}}"),
            bordered: true,
        },
        GraphAnnotation::LineOrArrow {
            start_pt: (90.0, 100.0),
            end_pt: (150.0, 75.0),
            arrowhead: true,
            dash: LineDashStyle::Solid,
            color: Rgba8::BLACK,
        },
        GraphAnnotation::Shape {
            kind: AnnotationShapeKind::Rectangle,
            bounds_pt: DocRectPt::new(100.0, 120.0, 40.0, 20.0),
            stroke_color: Rgba8::BLACK,
            stroke_width_pt: 1.0,
            dash: LineDashStyle::Dashed,
            fill_color: Some(Rgba8::rgba(0, 114, 178, 60)),
        },
        GraphAnnotation::Shape {
            kind: AnnotationShapeKind::RoundedRectangle,
            bounds_pt: DocRectPt::new(150.0, 120.0, 40.0, 20.0),
            stroke_color: Rgba8::BLACK,
            stroke_width_pt: 1.0,
            dash: LineDashStyle::Solid,
            fill_color: None,
        },
        GraphAnnotation::Shape {
            kind: AnnotationShapeKind::Ellipse,
            bounds_pt: DocRectPt::new(200.0, 120.0, 40.0, 24.0),
            stroke_color: Rgba8::BLACK,
            stroke_width_pt: 1.0,
            dash: LineDashStyle::Dotted,
            fill_color: Some(Rgba8::rgba(0, 158, 115, 70)),
        },
        GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("fit-1"),
            tab_id: Some(String::from("main")),
            bounds_pt: DocRectPt::new(180.0, 155.0, 145.0, 55.0),
            selected_rows: None,
            selected_cols: None,
            style: curvebook_doc::EmbeddedTableStyle::default(),
        },
    ];

    let mut scene = ChartScene::new("chrome", "Full Chrome", 360.0, 260.0, Some(Rgba8::WHITE));
    let opts = AxisRenderOptions { show_gridlines: true, ..AxisRenderOptions::default() };
    let _axes = render_graph_chrome(&graph.spec, &doc, &mut scene, &opts);

    // Hook expansion check
    assert_eq!(
        resolve_template_hooks("{{info:Experiment ID}} / {{result:fit-1:IC50}}", &doc, Some(&graph.spec.primary_source_sheet_id)),
        "EXP-42 / 2.5"
    );
    assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("EXP-42"))));
    assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "Target Window")));
    // Shaded zone filled semi-transparently
    assert!(
        scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { fill: Some(FillPaint { color }), stroke: None, .. } if color.a == 48))
    );
    // Oval/ellipse emits CubicTo commands
    assert!(
        scene
            .primitives
            .iter()
            .any(|p| matches!(p, ScenePrimitive::Path { path, .. } if path.commands.iter().any(|c| matches!(c, PathCmd::CubicTo { .. }))))
    );
    // Embedded result table emits table values
    assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("[1.8, 3.4]"))));
    assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("***"))));

    // Missing ResultSheet in EmbeddedResultTable + non-finite LineOrArrow emit structured warnings
    let mut bad_scene = ChartScene::new("bad_ann", "Bad Ann", 360.0, 260.0, None);
    render_annotations(
        &[
            GraphAnnotation::LineOrArrow {
                start_pt: (f64::NAN, 0.0),
                end_pt: (10.0, 10.0),
                arrowhead: true,
                dash: LineDashStyle::Solid,
                color: Rgba8::BLACK,
            },
            GraphAnnotation::EmbeddedResultTable {
                result_sheet_id: SheetId::new("missing-fit"),
                tab_id: None,
                bounds_pt: DocRectPt::new(10.0, 10.0, 80.0, 40.0),
                selected_rows: None,
                selected_cols: None,
                style: curvebook_doc::EmbeddedTableStyle::default(),
            },
        ],
        &doc,
        None,
        &mut bad_scene,
    );
    assert!(bad_scene.warnings.iter().any(|w| w.code == "non_finite_annotation"));
    assert!(bad_scene.warnings.iter().any(|w| w.code == "missing_embedded_result_table"));

    // Hidden legend emits nothing
    let entries = legend_entries_from_graph_spec(&graph.spec, &doc);
    assert_eq!(entries.len(), 2);
    let mut s_noleg = ChartScene::new("noleg", "NoLeg", 360.0, 260.0, None);
    render_legend(&LegendSpec { visible: false, position_pt: None, bordered: true }, &entries, &doc, None, &mut s_noleg);
    assert!(s_noleg.primitives.is_empty());

    // Custom LegendEntry variants
    let mut s_leg = ChartScene::new("leg", "Leg", 360.0, 260.0, None);
    render_legend(
        &LegendSpec { visible: true, position_pt: Some((20.0, 20.0)), bordered: true },
        &[LegendEntry {
            label: String::from("Group A"),
            swatch_color: Rgba8::rgb(0x00, 0x72, 0xB2),
            stroke_color: Some(Rgba8::BLACK),
            marker_shape: Some(MarkerShape::Diamond),
            marker_fill_mode: MarkerFillMode::Open,
            dash_style: Some(LineDashStyle::Dashed),
            is_box_swatch: false,
        }],
        &doc,
        None,
        &mut s_leg,
    );
    assert!(!s_leg.primitives.is_empty());
}

#[test]
fn render_annotations_supports_model_equation_boxes_with_stacked_fractions() {
    let mut doc = ProjectDocument::default();
    let nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    let mut res =
        ResultSheet::new(AnalysisSpec::new(SheetId::new("fit-eq"), SheetId::new("d1"), AnalysisKind::NonlinearRegression(nl)).unwrap()).unwrap();
    res.status = ResultStatus::UpToDate;
    res.scalars = vec![
        ResultScalarEntry { key: String::from("Bottom"), data_set_index: Some(0), value: 5.0, units: None },
        ResultScalarEntry { key: String::from("Top"), data_set_index: Some(0), value: 95.0, units: None },
        ResultScalarEntry { key: String::from("LogEC50"), data_set_index: Some(0), value: -7.0, units: None },
        ResultScalarEntry { key: String::from("HillSlope"), data_set_index: Some(0), value: 1.2, units: None },
    ];
    doc.result_sheets.push(res);

    let anns = [GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("fit-eq"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(50.0, 30.0, 240.0, 56.0),
        font_size_pt: 9.5,
        bordered: true,
    }];
    let mut scene = ChartScene::new("s-eq", "Equation Annotation", 400.0, 300.0, Some(Rgba8::WHITE));
    render_annotations(&anns, &doc, None, &mut scene);

    let texts: Vec<String> = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect();
    let joined = texts.join(" ");
    assert!(joined.contains("95"), "expected substituted Top=95 in {joined}");
    assert!(joined.contains("1.2"), "expected substituted HillSlope=1.2 in {joined}");

    // Verify at least one horizontal vinculum line path inside the box
    let vinculum_y = scene.primitives.iter().find_map(|p| match p {
        ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None } if path.commands.len() == 2 => {
            match (path.commands.first(), path.commands.get(1)) {
                (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b))) if (a.y - b.y).abs() < 1e-6 && b.x > a.x + 5.0 => Some(a.y),
                _ => None,
            }
        }
        _ => None,
    });
    let vin_y = vinculum_y.expect("expected horizontal vinculum rule");
    let has_num_above = scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.anchor_pt.y < vin_y));
    let has_den_below = scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.anchor_pt.y > vin_y));
    assert!(has_num_above && has_den_below, "numerator and denominator must bracket vinculum at y={vin_y}");
}

proptest! {
    #[test]
    fn prop_all_axis_scales_roundtrip_monotonically(u_frac in 0.05f64..0.95) {
        let cases = [
            (AxisScale::Linear, -100.0, 100.0),
            (AxisScale::Log10, 1e-4, 1e4),
            (AxisScale::Log2, 0.25, 256.0),
            (AxisScale::Ln, 0.05, 50.0),
            (AxisScale::Logit(LogitDomain::Proportion), 0.01, 0.99),
            (AxisScale::Logit(LogitDomain::Percent), 1.0, 99.0),
            (AxisScale::Probit, 0.5, 99.5),
            (AxisScale::Reciprocal, 1.0, 50.0),
        ];
        for (scale, min, max) in cases {
            let val = min + u_frac * (max - min);
            let m = AxisMapping { min, max, scale };
            let pt = m.map_value(val, 20.0, 320.0).expect("map finite");
            prop_assert!(pt.is_finite());
            prop_assert!((20.0 - 1e-9..=320.0 + 1e-9).contains(&pt));
            let back = m.unmap_coord(pt, 20.0, 320.0).expect("unmap finite");
            prop_assert!((back - val).abs() <= 1e-7 * val.abs().max(1.0));
        }
    }
}
