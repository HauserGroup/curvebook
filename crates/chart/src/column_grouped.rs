//! Column, grouped bar, replicate scatter, box-and-whisker, violin, floating bar,
//! before/after paired lines, and auto-stacked collision-free significance brackets.

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, AxisSpec, BracketLabelMode, BracketStyle, CartesianOrientation, CategoricalTickSource, ColumnTable, ErrorBarDirection,
    ErrorBarMetric, ErrorBarSpec, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, GroupedTable, NestedTable, PairwiseBracketSpec,
    PairwiseComparisonRecord, PairwiseDisplayMode, ProjectDocument, ReplicateOverlayStyle, ResultStatus, SheetId, SubcolumnMode, SummaryMeasure,
    TableArchetype, TickLabelFormat, WhiskerMethod,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::sum::SummaryMoments;
use curvebook_palette::{LineDashStyle, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::cld::{CldOptions, compute_cld_from_result_sheet};
use curvebook_stats::column::summarize_pre_summarized_points;
use curvebook_stats::common::{quantile_sorted, quartiles_hinge, sorted_finite};
use curvebook_text::{LocaleNumberPolicy, RichText, format_pvalue_label, measure_text_pt, parse_markup};

use crate::markers::{emit_marker, polygon_path};
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PathCmd, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive,
    SourceCellId, StrokePaint, TextAnchor,
};
use crate::xy::{
    AxisErrorTarget, ChromeAxes, LegendItem, SegmentedAxis, compute_central_tendency, compute_raw_error_bounds, configure_plot_rect,
    emit_directional_error_bar, expand_doc_text, format_point_value, is_log_like, primary_info_sheet_id, render_cartesian_chrome,
    render_cartesian_chrome_full, render_legend_and_annotations, render_legend_and_annotations_with_offset, resolve_data_sheet, stroke_with_dash,
    summarize_subcells,
};

const KDE_SAMPLES: usize = 65;

#[derive(Debug, Clone)]
pub(crate) struct RawObs {
    pub(crate) value: f64,
    pub(crate) source: SourceCellId,
}

#[derive(Debug, Clone)]
enum ExtractedKind {
    Raw(Vec<RawObs>),
    PreSummarized { center: f64, lower: f64, upper: f64, source: SourceCellId },
}

#[derive(Debug, Clone)]
struct CategorySlotItem {
    group_key: usize,
    col_index: usize,
    override_series_index: Option<usize>,
    row_index: Option<usize>,
    match_label: String,
    alt_label: String,
    x_pos: f64,
    slot_width_data: f64,
    gdo: GraphicalDataObject,
    y_error_bars: Option<ErrorBarSpec>,
    default_color: Rgba8,
    bind_to_right_y2: bool,
    payload: ExtractedKind,
    stack_group: Option<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderedSlotBound {
    pub(crate) group_key: usize,
    pub(crate) label: String,
    pub(crate) alt_label: String,
    pub(crate) x_pt: f64,
    pub(crate) top_y_pt: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedCldSlot {
    pub(crate) group_key: usize,
    pub(crate) label: String,
    pub(crate) letters: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedCldLabels {
    pub(crate) font_size_pt: f64,
    pub(crate) offset_pt: f64,
    pub(crate) color: Rgba8,
    pub(crate) labels: Vec<ResolvedCldSlot>,
}

#[derive(Debug, Clone)]
struct ResolvedBracket {
    slot_a_idx: usize,
    slot_b_idx: usize,
    x_lo: f64,
    x_hi: f64,
    style: BracketStyle,
    label: RichText,
    stroke_width_pt: f64,
    font_size_pt: f64,
    step_pt: f64,
}

/// Compile a column, grouped, or nested graph sheet into vector primitives inside `scene`.
pub fn compile_column_or_grouped_graph(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let primary_opt = resolve_data_sheet(doc, &graph.spec.primary_source_sheet_id);
    if primary_opt.is_none() {
        scene
            .warnings
            .push(ChartWarning::new("missing_source_sheet", format!("primary data sheet '{}' not found", graph.spec.primary_source_sheet_id)));
    }
    if matches!(graph.spec.orientation, CartesianOrientation::Horizontal) {
        return compile_horizontal_column_or_grouped(graph, doc, scene, primary_opt);
    }

    let has_y2 = graph.spec.y2_axis.is_some()
        || graph.spec.series.iter().any(|s| s.bind_to_right_y2)
        || matches!(graph.spec.frame_preset, curvebook_doc::FramePreset::DualYSecondaryRight);
    let mut tick_positions: Vec<(f64, String)> = Vec::new();
    let mut items = primary_opt.map(|primary| extract_items(graph, primary, &mut tick_positions, &mut scene.warnings)).unwrap_or_default();

    let draw_brackets =
        matches!(graph.spec.pairwise_display_mode, PairwiseDisplayMode::BracketsOnly | PairwiseDisplayMode::BracketsAndCompactLetterDisplay);
    let active_bracket_est = if draw_brackets { count_active_brackets(graph, doc, &items) } else { 0 };
    let slot_metas: Vec<(usize, &str, &str, Option<f64>, f64)> =
        items.iter().map(|it| (it.group_key, it.match_label.as_str(), it.alt_label.as_str(), item_summary_value(it), it.x_pos)).collect();
    let resolved_cld = resolve_cld_overlay(graph, doc, &slot_metas, &mut scene.warnings);
    let bracket_margin_pt = if active_bracket_est > 0 { ((active_bracket_est.min(4) as f64) * 16.0).min(64.0) } else { 0.0 };
    let cld_margin_pt = if resolved_cld.is_some() { cld_reserved_top_margin_pt(&graph.spec) } else { 0.0 };
    let legend_items = primary_opt.map(|primary| build_legend_items(graph, primary)).unwrap_or_default();
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    let legend_box_w = crate::scales::auto_legend_box_width_pt(&graph.spec.legend, legend_items.iter().map(|it| it.label.as_str()), doc, info_id);
    configure_plot_rect(graph, scene, has_y2, bracket_margin_pt + cld_margin_pt, legend_box_w);

    let Some(primary) = primary_opt else {
        render_empty_axes(graph, doc, scene, has_y2);
        return Ok(());
    };

    let (x_vals, y_vals, y2_vals) = collect_domain_values(graph, &items, &tick_positions, &mut scene.warnings);
    if matches!(graph.spec.grouped_arrangement, GroupedArrangement::Superimposed) {
        prepare_superimposed_items(&mut items);
    }

    let include_zero_y = items.iter().any(|it| matches!(it.gdo, GraphicalDataObject::BarWithReplicates { .. }));
    let bracket_headroom = if active_bracket_est > 0 { (0.14 + 0.04 * (active_bracket_est.saturating_sub(1).min(3) as f64)).min(0.26) } else { 0.05 };
    let headroom =
        if resolved_cld.is_some() { if active_bracket_est > 0 { (bracket_headroom + 0.05).min(0.30) } else { 0.12 } } else { bracket_headroom };
    let r = scene.plot_rect_pt;
    let x_axis = SegmentedAxis::build(&graph.spec.x_axis, r.x, r.right(), &x_vals, false, 0.0);
    let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &y_vals, include_zero_y, headroom);
    let y2_axis = if has_y2 {
        let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| curvebook_doc::AxisSpec::linear("Y2"));
        let source = if y2_vals.is_empty() { &y_vals } else { &y2_vals };
        Some(SegmentedAxis::build(&y2_spec, r.bottom(), r.y, source, include_zero_y, headroom))
    } else {
        None
    };

    let cat_ticks_opt = if tick_positions.is_empty() { None } else { Some(tick_positions.as_slice()) };
    render_cartesian_chrome(scene, graph, doc, &x_axis, &y_axis, y2_axis.as_ref(), cat_ticks_opt);

    let mut bounds: Vec<RenderedSlotBound> = Vec::with_capacity(items.len());
    let connector_axes = ConnectorAxes { x: &x_axis, y: &y_axis, y2: y2_axis.as_ref() };
    render_before_after_connectors(scene, &graph.spec, primary, &items, connector_axes, &mut bounds);

    // Track cumulative positive/negative stack heights per row for Stacked arrangement.
    let mut stacks = StackAccum::default();

    for item in &mut items {
        if matches!(item.gdo, GraphicalDataObject::BeforeAfterLines { .. }) {
            continue;
        }
        let target_y = if item.bind_to_right_y2 { y2_axis.as_ref().unwrap_or(&y_axis) } else { &y_axis };
        let top_y = render_single_item_oriented(scene, &graph.spec, item, &x_axis, target_y, &mut stacks, false);
        if let Some((_, x_pt)) = x_axis.map_value(item.x_pos) {
            bounds.push(RenderedSlotBound {
                group_key: item.group_key,
                label: item.match_label.clone(),
                alt_label: item.alt_label.clone(),
                x_pt,
                top_y_pt: top_y.unwrap_or(r.bottom()),
            });
        }
    }

    if let Some(cld) = &resolved_cld {
        render_cld_labels(scene, graph, cld, &mut bounds);
    }
    if draw_brackets {
        render_pairwise_brackets(scene, graph, doc, &bounds);
    }
    render_legend_and_annotations(scene, graph, doc, &legend_items)?;
    Ok(())
}

fn render_empty_axes(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene, has_y2: bool) {
    let r = scene.plot_rect_pt;
    let x_axis = SegmentedAxis::build(&graph.spec.x_axis, r.x, r.right(), &[0.0, 1.0], false, 0.0);
    let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &[0.0, 1.0], false, 0.0);
    let y2_axis = if has_y2 {
        let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| curvebook_doc::AxisSpec::linear("Y2"));
        Some(SegmentedAxis::build(&y2_spec, r.bottom(), r.y, &[0.0, 1.0], false, 0.0))
    } else {
        None
    };
    render_cartesian_chrome(scene, graph, doc, &x_axis, &y_axis, y2_axis.as_ref(), None);
}

#[derive(Debug, Clone)]
struct HorizontalSlotBound {
    group_key: usize,
    label: String,
    alt_label: String,
    y_pt: f64,
    right_x_pt: f64,
}

fn compile_horizontal_column_or_grouped(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
    primary_opt: Option<&curvebook_doc::DataSheet>,
) -> Result<(), ChartError> {
    let has_y2 = graph.spec.y2_axis.is_some()
        || graph.spec.series.iter().any(|s| s.bind_to_right_y2)
        || matches!(graph.spec.frame_preset, curvebook_doc::FramePreset::DualYSecondaryRight);
    let mut tick_positions: Vec<(f64, String)> = Vec::new();
    let mut items = primary_opt.map(|primary| extract_items(graph, primary, &mut tick_positions, &mut scene.warnings)).unwrap_or_default();

    let draw_brackets =
        matches!(graph.spec.pairwise_display_mode, PairwiseDisplayMode::BracketsOnly | PairwiseDisplayMode::BracketsAndCompactLetterDisplay);
    let active_bracket_est = if draw_brackets { count_active_brackets(graph, doc, &items) } else { 0 };
    let slot_metas: Vec<(usize, &str, &str, Option<f64>, f64)> =
        items.iter().map(|it| (it.group_key, it.match_label.as_str(), it.alt_label.as_str(), item_summary_value(it), it.x_pos)).collect();
    let resolved_cld = resolve_cld_overlay(graph, doc, &slot_metas, &mut scene.warnings);

    let legend_items = primary_opt.map(|primary| build_legend_items(graph, primary)).unwrap_or_default();
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    let legend_box_w = crate::scales::auto_legend_box_width_pt(&graph.spec.legend, legend_items.iter().map(|it| it.label.as_str()), doc, info_id);
    configure_plot_rect(graph, scene, has_y2, 0.0, legend_box_w);
    let has_y_title = !graph.spec.y_axis.title.trim().is_empty();
    let max_cat_w_pt = tick_positions.iter().map(|(_, lbl)| measure_text_pt(&parse_markup(lbl), 8.0).width_pt).fold(0.0_f64, f64::max);
    if max_cat_w_pt > 0.0 {
        let min_left = if has_y_title { 28.0 + max_cat_w_pt + 10.0 } else { 14.0 + max_cat_w_pt + 8.0 };
        let right = scene.plot_rect_pt.right();
        let max_left = (scene.width_pt * 0.42).min((right - 40.0).max(scene.plot_rect_pt.x));
        let new_left = scene.plot_rect_pt.x.max(min_left).min(max_left);
        scene.plot_rect_pt.x = new_left;
        scene.plot_rect_pt.width = (right - new_left).max(40.0);
    }
    let extra_right = (if active_bracket_est > 0 { ((active_bracket_est.min(4) as f64) * 18.0).min(72.0) } else { 0.0 })
        + if resolved_cld.is_some() { 24.0 } else { 0.0 };
    let applied_extra_right = if extra_right > 0.0 {
        let shrink = extra_right.min((scene.plot_rect_pt.width - 40.0).max(0.0));
        scene.plot_rect_pt.width -= shrink;
        shrink
    } else {
        0.0
    };

    let Some(primary) = primary_opt else {
        let r = scene.plot_rect_pt;
        let x_axis = SegmentedAxis::build(&graph.spec.x_axis, r.x, r.right(), &[0.0, 1.0], false, 0.0);
        let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &[0.0, 1.0], false, 0.0);
        let y2_axis = has_y2.then(|| {
            let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2"));
            SegmentedAxis::build(&y2_spec, r.bottom(), r.y, &[0.0, 1.0], false, 0.0)
        });
        render_cartesian_chrome_full(scene, graph, doc, &ChromeAxes { x: &x_axis, y: &y_axis, y2: y2_axis.as_ref(), cat_x: None, cat_y: None });
        return Ok(());
    };

    if items.iter().any(|it| it.bind_to_right_y2) {
        scene.warnings.push(ChartWarning::new(
            "horizontal_y2_unsupported",
            "right Y2 series binding is not supported in horizontal orientation; plotted against horizontal value axis",
        ));
    }

    let (cat_vals, mut val_vals, y2_vals) = collect_domain_values(graph, &items, &tick_positions, &mut scene.warnings);
    val_vals.extend(y2_vals);
    if matches!(graph.spec.grouped_arrangement, GroupedArrangement::Superimposed) {
        prepare_superimposed_items(&mut items);
    }

    let include_zero_x = items.iter().any(|it| matches!(it.gdo, GraphicalDataObject::BarWithReplicates { .. }));
    let headroom = if active_bracket_est > 0 || resolved_cld.is_some() { 0.14 } else { 0.05 };
    let r = scene.plot_rect_pt;
    let x_axis = SegmentedAxis::build(&graph.spec.x_axis, r.x, r.right(), &val_vals, include_zero_x, headroom);
    let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &cat_vals, false, 0.0);
    let y2_axis = has_y2.then(|| {
        let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2"));
        SegmentedAxis::build(&y2_spec, r.bottom(), r.y, &cat_vals, false, 0.0)
    });

    let cat_ticks_opt = if tick_positions.is_empty() { None } else { Some(tick_positions.as_slice()) };
    render_cartesian_chrome_full(scene, graph, doc, &ChromeAxes { x: &x_axis, y: &y_axis, y2: y2_axis.as_ref(), cat_x: None, cat_y: cat_ticks_opt });

    let mut horiz_bounds: Vec<HorizontalSlotBound> = Vec::with_capacity(items.len());
    let connector_axes = ConnectorAxes { x: &y_axis, y: &x_axis, y2: None };
    let mut vert_proxy_bounds: Vec<RenderedSlotBound> = Vec::new();
    render_before_after_connectors_oriented(scene, &graph.spec, primary, &items, connector_axes, true, &mut vert_proxy_bounds, &mut horiz_bounds);

    let mut stacks = StackAccum::default();
    for item in &mut items {
        if matches!(item.gdo, GraphicalDataObject::BeforeAfterLines { .. }) {
            continue;
        }
        let right_x = render_single_item_oriented(scene, &graph.spec, item, &y_axis, &x_axis, &mut stacks, true);
        if let Some((_, y_pt)) = y_axis.map_value(item.x_pos) {
            horiz_bounds.push(HorizontalSlotBound {
                group_key: item.group_key,
                label: item.match_label.clone(),
                alt_label: item.alt_label.clone(),
                y_pt,
                right_x_pt: right_x.unwrap_or(r.x),
            });
        }
    }

    if let Some(cld) = &resolved_cld {
        render_horizontal_cld_labels(scene, cld, &mut horiz_bounds);
    }
    if draw_brackets {
        render_horizontal_pairwise_brackets(scene, graph, doc, &horiz_bounds);
    }

    let _ = render_legend_and_annotations_with_offset(scene, graph, doc, &legend_items, applied_extra_right)?;
    Ok(())
}

fn count_active_brackets(graph: &GraphSheet, doc: &ProjectDocument, items: &[CategorySlotItem]) -> usize {
    graph
        .spec
        .comparisons
        .iter()
        .filter(|b| {
            let res_opt = doc.result_sheet(&b.result_sheet_id);
            if res_opt.is_some_and(|res| matches!(res.status, ResultStatus::Failed { .. })) {
                return false;
            }
            let rec_opt = res_opt.and_then(|res| find_comparison_record(&res.pairwise_comparisons, b.group_a, b.group_b));
            if b.hide_if_non_significant && !rec_opt.is_some_and(|rec| rec.significant) {
                return false;
            }
            if !items.is_empty() {
                let Some(slot_a) = find_item_for_group(items, b.group_a, rec_opt.map(|r| r.label_a.as_str())) else {
                    return false;
                };
                let Some(slot_b) = find_item_for_group(items, b.group_b, rec_opt.map(|r| r.label_b.as_str())) else {
                    return false;
                };
                if matches!(graph.spec.grouped_arrangement, GroupedArrangement::Stacked) && (slot_a.x_pos - slot_b.x_pos).abs() < 1e-6 {
                    return false;
                }
            }
            true
        })
        .count()
}

fn find_item_for_group<'a>(items: &'a [CategorySlotItem], group_idx: usize, label_hint: Option<&str>) -> Option<&'a CategorySlotItem> {
    let idx = find_matched_index(items, group_idx, label_hint, |it| (it.group_key, it.match_label.as_str(), it.alt_label.as_str()))?;
    items.get(idx)
}

fn find_matched_index<T>(slice: &[T], group_idx: usize, label_hint: Option<&str>, meta: impl Fn(&T) -> (usize, &str, &str)) -> Option<usize> {
    if let Some(hint) = label_hint {
        let needle = hint.trim();
        if !needle.is_empty()
            && let Some((pos, _)) = slice.iter().enumerate().find(|(_, item)| {
                let (_, lbl, alt) = meta(item);
                lbl.trim() == needle || alt.trim() == needle
            })
        {
            return Some(pos);
        }
    }
    slice.iter().enumerate().find(|(_, item)| meta(item).0 == group_idx).map(|(pos, _)| pos)
}

fn item_z_footprint(item: &CategorySlotItem) -> (f64, f64) {
    let val_extent = match &item.payload {
        ExtractedKind::PreSummarized { center, upper, .. } => center.abs().max(upper.abs()),
        ExtractedKind::Raw(obs) => {
            let mut tmp_warnings = Vec::new();
            let vals = raw_values(obs);
            compute_central_tendency(&vals, gdo_summary_measure(&item.gdo), &mut tmp_warnings).map_or(0.0, f64::abs)
        }
    };
    let marker_size = match &item.gdo {
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker.size_pt,
        GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } | GraphicalDataObject::BeforeAfterLines { marker: Some(m), .. } => m.size_pt,
        _ => 0.0,
    };
    if matches!(item.gdo, GraphicalDataObject::AlignedDotPlot { .. } | GraphicalDataObject::ScatterAndLine { .. }) {
        (marker_size, val_extent)
    } else {
        (val_extent, marker_size)
    }
}

fn prepare_superimposed_items(items: &mut [CategorySlotItem]) {
    items.sort_by(|a, b| {
        let za = item_z_footprint(a);
        let zb = item_z_footprint(b);
        zb.0.total_cmp(&za.0).then_with(|| zb.1.total_cmp(&za.1)).then_with(|| a.group_key.cmp(&b.group_key))
    });
    for i in 0..items.len() {
        let x = items[i].x_pos;
        let rank_at_x = items[..i].iter().filter(|prev| (prev.x_pos - x).abs() < 1e-6).count();
        if rank_at_x > 0 && matches!(items[i].gdo, GraphicalDataObject::BarWithReplicates { .. } | GraphicalDataObject::FloatingBar { .. }) {
            items[i].slot_width_data *= (1.0 - 0.12 * (rank_at_x as f64).min(4.0)).max(0.35);
        }
    }
}

fn find_comparison_record(records: &[PairwiseComparisonRecord], a: usize, b: usize) -> Option<&PairwiseComparisonRecord> {
    records.iter().find(|r| (r.group_a == a && r.group_b == b) || (r.group_a == b && r.group_b == a))
}

fn resolve_series_style(graph: &GraphSheet, col_idx: usize) -> (GraphicalDataObject, Option<ErrorBarSpec>, Rgba8, bool) {
    let palette_color = graph.spec.palette.color_at(col_idx);
    if let Some(s) = graph.spec.series.iter().find(|s| s.data_set_index == col_idx) {
        return (s.gdo.clone(), s.y_error_bars, palette_color, s.bind_to_right_y2);
    }
    if let Some(first) = graph.spec.series.first() {
        let mut gdo = first.gdo.clone();
        if graph.spec.series.len() == 1 {
            tint_gdo(&mut gdo, palette_color);
        }
        return (gdo, first.y_error_bars, palette_color, first.bind_to_right_y2);
    }
    let gdo = GraphicalDataObject::BarWithReplicates {
        summary: SummaryMeasure::Mean,
        fill_color: palette_color.with_alpha(210),
        border_color: palette_color,
        border_width_pt: 1.0,
        replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
    };
    (gdo, Some(ErrorBarSpec::default()), palette_color, false)
}

fn tint_gdo(gdo: &mut GraphicalDataObject, swatch: Rgba8) {
    match gdo {
        GraphicalDataObject::BarWithReplicates { fill_color, border_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, border_color, .. } => {
            *fill_color = swatch.with_alpha(fill_color.a);
            *border_color = swatch;
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            marker.fill_color = swatch;
            marker.stroke_color = swatch;
        }
        GraphicalDataObject::BoxAndWhisker { fill_color, stroke_color, .. } | GraphicalDataObject::Violin { fill_color, stroke_color, .. } => {
            *fill_color = swatch.with_alpha(fill_color.a);
            *stroke_color = swatch;
        }
        GraphicalDataObject::BeforeAfterLines { marker, line_color, .. } => {
            *line_color = swatch;
            if let Some(m) = marker {
                m.fill_color = swatch;
                m.stroke_color = swatch;
            }
        }
        _ => {}
    }
}

fn extract_items(
    graph: &GraphSheet,
    sheet: &curvebook_doc::DataSheet,
    tick_positions: &mut Vec<(f64, String)>,
    warnings: &mut Vec<ChartWarning>,
) -> Vec<CategorySlotItem> {
    match &sheet.archetype {
        TableArchetype::Column(col) => extract_column_items(graph, &sheet.id, col, tick_positions, warnings),
        TableArchetype::Nested(nested) => extract_nested_items(graph, &sheet.id, nested, tick_positions),
        TableArchetype::Grouped(grouped) => extract_grouped_items(graph, &sheet.id, grouped, tick_positions, warnings),
        _ => {
            warnings.push(ChartWarning::new("incompatible_table_archetype", "sheet archetype is not a Column, Grouped, or Nested table"));
            Vec::new()
        }
    }
}

fn effective_presum_err_spec<'a>(
    gdo: &GraphicalDataObject,
    y_error_bars: &'a Option<ErrorBarSpec>,
    scratch: &'a mut Option<ErrorBarSpec>,
) -> Option<&'a ErrorBarSpec> {
    if let Some(eb) = y_error_bars {
        return Some(eb);
    }
    if matches!(gdo, GraphicalDataObject::BoxAndWhisker { .. } | GraphicalDataObject::Violin { .. } | GraphicalDataObject::FloatingBar { .. }) {
        *scratch = Some(ErrorBarSpec { metric: ErrorBarMetric::ExplicitSubcolumns, ..ErrorBarSpec::default() });
        return scratch.as_ref();
    }
    None
}

fn category_axis(graph: &GraphSheet) -> &AxisSpec {
    if matches!(graph.spec.orientation, CartesianOrientation::Horizontal) { &graph.spec.y_axis } else { &graph.spec.x_axis }
}

fn extract_column_items(
    graph: &GraphSheet,
    sheet_id: &SheetId,
    table: &ColumnTable,
    tick_positions: &mut Vec<(f64, String)>,
    warnings: &mut Vec<ChartWarning>,
) -> Vec<CategorySlotItem> {
    let mut items = Vec::with_capacity(table.data_sets.len());
    let cat_axis = category_axis(graph);
    for (c, ds) in table.data_sets.iter().enumerate() {
        let (gdo, y_error_bars, default_color, bind_to_right_y2) = resolve_series_style(graph, c);
        let x_pos = (c as f64) + 0.5;
        let label = match cat_axis.tick_format {
            TickLabelFormat::Categorical(CategoricalTickSource::RowNumbers) => format!("{}", c + 1),
            _ => ds.title.clone(),
        };
        tick_positions.push((x_pos, label.clone()));

        let payload = if table.subcolumn_mode.is_pre_summarized() {
            let mut scratch = None;
            let eff_err = effective_presum_err_spec(&gdo, &y_error_bars, &mut scratch);
            extract_column_presummarized(sheet_id, c, table, eff_err, warnings)
        } else {
            let mut obs = Vec::new();
            for (row, subcells) in ds.cells.iter().enumerate() {
                for (subcol, cell) in subcells.iter().enumerate() {
                    if let Some(value) = cell.active_number() {
                        obs.push(RawObs { value, source: SourceCellId { sheet_id: sheet_id.clone(), row, col: c, subcol } });
                    }
                }
            }
            ExtractedKind::Raw(obs)
        };

        items.push(CategorySlotItem {
            group_key: c,
            col_index: c,
            override_series_index: crate::override_series_index_for_col(&graph.spec, &graph.spec.primary_source_sheet_id, c),
            row_index: None,
            match_label: ds.title.clone(),
            alt_label: label,
            x_pos,
            slot_width_data: 0.68,
            gdo,
            y_error_bars,
            default_color,
            bind_to_right_y2,
            payload,
            stack_group: None,
        });
    }
    items
}

fn extract_column_presummarized(
    sheet_id: &SheetId,
    col: usize,
    table: &ColumnTable,
    err_spec: Option<&ErrorBarSpec>,
    warnings: &mut Vec<ChartWarning>,
) -> ExtractedKind {
    let Some(ds) = table.data_sets.get(col) else {
        return ExtractedKind::Raw(Vec::new());
    };
    let mode = table.subcolumn_mode;
    if mode.supports_parametric_moments() {
        let mut points = Vec::new();
        let mut first_row = None;
        for row in 0..ds.cells.len() {
            match table.summarized_point(row, col) {
                Ok(Some(pt)) => {
                    first_row.get_or_insert(row);
                    points.push(pt);
                }
                Ok(None) => {}
                Err(err) => {
                    warnings.push(ChartWarning::new("invalid_pre_summarized_point", err.to_string()));
                }
            }
        }
        let (Some(row0), Ok(pooled)) = (first_row, summarize_pre_summarized_points(&ds.title, &points, ConfidenceLevel::DEFAULT_95)) else {
            return ExtractedKind::Raw(Vec::new());
        };
        if !pooled.mean.is_finite() {
            return ExtractedKind::Raw(Vec::new());
        }
        let center = pooled.mean;
        let (lower, upper) = if let Some(eb) = err_spec {
            match eb.metric {
                ErrorBarMetric::Sd => {
                    let sd = pooled.sd.unwrap_or_else(|| if points.len() == 1 { points[0].sd } else { 0.0 });
                    (center - sd, center + sd)
                }
                ErrorBarMetric::Sem => {
                    let sem = pooled.sem.unwrap_or_else(|| if points.len() == 1 { points[0].sd / points[0].n.sqrt() } else { 0.0 });
                    (center - sem, center + sem)
                }
                ErrorBarMetric::ConfidenceInterval95 => {
                    if let Some((lo, hi)) = pooled.mean_ci {
                        (lo, hi)
                    } else {
                        warnings.push(ChartWarning::new(
                            "insufficient_sample_size_for_ci",
                            format!("95% CI requires N >= 2 (got N = {}); error bar omitted", pooled.total_n),
                        ));
                        (center, center)
                    }
                }
                ErrorBarMetric::ExplicitSubcolumns | ErrorBarMetric::InterquartileRange | ErrorBarMetric::Range => {
                    if points.len() == 1
                        && let Some(b) = ds.cells.get(row0).and_then(|subs| mode.explicit_bounds(subs))
                    {
                        (b.lower, b.upper)
                    } else {
                        let spread = match mode {
                            SubcolumnMode::MeanSemN => pooled.sem.or(pooled.sd).unwrap_or(0.0),
                            _ => pooled.sd.unwrap_or(0.0),
                        };
                        (center - spread, center + spread)
                    }
                }
            }
        } else {
            (center, center)
        };
        if !lower.is_finite() || !upper.is_finite() {
            return ExtractedKind::Raw(Vec::new());
        }
        return ExtractedKind::PreSummarized { center, lower, upper, source: SourceCellId { sheet_id: sheet_id.clone(), row: row0, col, subcol: 0 } };
    }

    for (row, subcells) in ds.cells.iter().enumerate() {
        if let Some((center, lower, upper)) = summarize_subcells(mode, subcells, SummaryMeasure::Mean, err_spec, warnings) {
            return ExtractedKind::PreSummarized { center, lower, upper, source: SourceCellId { sheet_id: sheet_id.clone(), row, col, subcol: 0 } };
        }
    }
    ExtractedKind::Raw(Vec::new())
}

fn extract_nested_items(
    graph: &GraphSheet,
    sheet_id: &SheetId,
    table: &NestedTable,
    tick_positions: &mut Vec<(f64, String)>,
) -> Vec<CategorySlotItem> {
    let mut items = Vec::with_capacity(table.groups.len());
    let cat_axis = category_axis(graph);
    for (g_idx, group) in table.groups.iter().enumerate() {
        let (gdo, y_error_bars, default_color, bind_to_right_y2) = resolve_series_style(graph, g_idx);
        let x_pos = (g_idx as f64) + 0.5;
        let label = match cat_axis.tick_format {
            TickLabelFormat::Categorical(CategoricalTickSource::RowNumbers) => format!("{}", g_idx + 1),
            _ => group.title.clone(),
        };
        tick_positions.push((x_pos, label.clone()));

        let mut unit_means = Vec::new();
        let mut all_tech = Vec::new();
        for b in 0..group.biological_unit_count() {
            let mut unit_vals = Vec::new();
            let mut first_row: Option<usize> = None;
            for (r, row_cells) in group.cells.iter().enumerate() {
                if let Some(v) = row_cells.get(b).and_then(curvebook_doc::Cell::active_number) {
                    first_row.get_or_insert(r);
                    unit_vals.push(v);
                    all_tech.push(RawObs { value: v, source: SourceCellId { sheet_id: sheet_id.clone(), row: r, col: g_idx, subcol: b } });
                }
            }
            if !unit_vals.is_empty()
                && let Ok(m) = SummaryMoments::from_slice(&unit_vals)
            {
                unit_means.push(RawObs {
                    value: m.mean,
                    source: SourceCellId { sheet_id: sheet_id.clone(), row: first_row.unwrap_or(0), col: g_idx, subcol: b },
                });
            }
        }

        // Box/Violin display technical density if biological units are small; bars summarize across bio units when >= 2.
        let use_unit_means = match &gdo {
            GraphicalDataObject::BoxAndWhisker { .. } | GraphicalDataObject::Violin { .. } => unit_means.len() >= 5,
            _ => unit_means.len() >= 2,
        };
        let values = if use_unit_means { unit_means } else { all_tech };

        items.push(CategorySlotItem {
            group_key: g_idx,
            col_index: g_idx,
            override_series_index: crate::override_series_index_for_col(&graph.spec, &graph.spec.primary_source_sheet_id, g_idx),
            row_index: None,
            match_label: group.title.clone(),
            alt_label: label,
            x_pos,
            slot_width_data: 0.68,
            gdo,
            y_error_bars,
            default_color,
            bind_to_right_y2,
            payload: ExtractedKind::Raw(values),
            stack_group: None,
        });
    }
    items
}

fn grouped_row_tick_label(table: &GroupedTable, cat_axis: &AxisSpec, r: usize) -> String {
    match cat_axis.tick_format {
        TickLabelFormat::Categorical(CategoricalTickSource::RowNumbers) => format!("{}", r + 1),
        _ => row_display_label(&table.row_titles, r),
    }
}

fn extract_grouped_items(
    graph: &GraphSheet,
    sheet_id: &SheetId,
    table: &GroupedTable,
    tick_positions: &mut Vec<(f64, String)>,
    warnings: &mut Vec<ChartWarning>,
) -> Vec<CategorySlotItem> {
    let nrows = table.row_count();
    let ncols = table.data_sets.len();
    if nrows == 0 || ncols == 0 {
        return Vec::new();
    }
    let arrangement = graph.spec.grouped_arrangement;
    let numeric_rows = grouped_row_x_positions(table, graph);
    let cat_axis = category_axis(graph);
    let mut items = Vec::with_capacity(nrows * ncols);

    match arrangement {
        GroupedArrangement::Interleaved => {
            for (r, &cx) in numeric_rows.iter().enumerate().take(nrows) {
                let step = if r + 1 < nrows { (numeric_rows[r + 1] - cx).abs().max(0.5) } else { 1.0 };
                let cluster_w = step * 0.76;
                let slot_w = cluster_w / (ncols as f64);
                let row_label = grouped_row_tick_label(table, cat_axis, r);
                if matches!(cat_axis.tick_format, TickLabelFormat::Categorical(_)) || table.row_numeric_spacing.is_none() {
                    tick_positions.push((cx, row_label));
                }
                for c in 0..ncols {
                    let slot_x = cx - 0.5 * cluster_w + ((c as f64) + 0.5) * slot_w;
                    let slot = GroupedSlotPlacement { r, c, x_pos: slot_x, slot_w: slot_w * 0.88, stack_group: None };
                    push_grouped_cell(&mut items, graph, sheet_id, table, slot, warnings);
                }
            }
        }
        GroupedArrangement::Separated => {
            let group_stride = (nrows as f64) + 0.9;
            for c in 0..ncols {
                for r in 0..nrows {
                    let slot_x = (c as f64) * group_stride + (r as f64) + 0.5;
                    let row_label = row_display_label(&table.row_titles, r);
                    let col_title = table.data_sets.get(c).map_or("", |d| d.title.as_str());
                    let label = match cat_axis.tick_format {
                        TickLabelFormat::Categorical(CategoricalTickSource::ColumnTitles) => {
                            if col_title.is_empty() {
                                format!("Col {}", c + 1)
                            } else {
                                col_title.to_owned()
                            }
                        }
                        TickLabelFormat::Categorical(CategoricalTickSource::RowTitles) => row_label,
                        TickLabelFormat::Categorical(CategoricalTickSource::RowNumbers) => format!("{}", r + 1),
                        _ => format!("{col_title}:{row_label}"),
                    };
                    tick_positions.push((slot_x, label));
                    let slot = GroupedSlotPlacement { r, c, x_pos: slot_x, slot_w: 0.72, stack_group: None };
                    push_grouped_cell(&mut items, graph, sheet_id, table, slot, warnings);
                }
            }
        }
        GroupedArrangement::Stacked | GroupedArrangement::Superimposed => {
            let stack_for_row = matches!(arrangement, GroupedArrangement::Stacked);
            for (r, &cx) in numeric_rows.iter().enumerate().take(nrows) {
                let row_label = grouped_row_tick_label(table, cat_axis, r);
                if matches!(cat_axis.tick_format, TickLabelFormat::Categorical(_)) || table.row_numeric_spacing.is_none() {
                    tick_positions.push((cx, row_label));
                }
                for c in 0..ncols {
                    let slot = GroupedSlotPlacement { r, c, x_pos: cx, slot_w: 0.62, stack_group: stack_for_row.then_some(r) };
                    push_grouped_cell(&mut items, graph, sheet_id, table, slot, warnings);
                }
            }
        }
    }
    items
}

fn grouped_row_x_positions(table: &GroupedTable, graph: &GraphSheet) -> Vec<f64> {
    let nrows = table.row_count();
    let cat_axis = if matches!(graph.spec.orientation, CartesianOrientation::Horizontal) { &graph.spec.y_axis } else { &graph.spec.x_axis };
    if !matches!(cat_axis.tick_format, TickLabelFormat::Categorical(_))
        && let Some(spacing) = &table.row_numeric_spacing
    {
        let mut out = Vec::with_capacity(nrows);
        let mut all_valid = spacing.len() == nrows;
        for opt in spacing {
            if let Some(v) = opt
                && v.is_finite()
            {
                out.push(*v);
            } else {
                all_valid = false;
                break;
            }
        }
        if all_valid {
            return out;
        }
    }
    (0..nrows).map(|r| (r as f64) + 0.5).collect()
}

fn row_display_label(titles: &[String], r: usize) -> String {
    titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty()).map_or_else(|| format!("Row {}", r + 1), str::to_owned)
}

#[derive(Clone, Copy)]
struct GroupedSlotPlacement {
    r: usize,
    c: usize,
    x_pos: f64,
    slot_w: f64,
    stack_group: Option<usize>,
}

fn raw_values(obs: &[RawObs]) -> Vec<f64> {
    obs.iter().map(|o| o.value).collect()
}

fn sorted_finite_obs(obs: &[RawObs]) -> Vec<RawObs> {
    let mut out: Vec<RawObs> = obs.iter().filter(|o| o.value.is_finite()).cloned().collect();
    out.sort_by(|a, b| a.value.total_cmp(&b.value));
    out
}

fn push_grouped_cell(
    out: &mut Vec<CategorySlotItem>,
    graph: &GraphSheet,
    sheet_id: &SheetId,
    table: &GroupedTable,
    slot: GroupedSlotPlacement,
    warnings: &mut Vec<ChartWarning>,
) {
    let r = slot.r;
    let c = slot.c;
    let (gdo, y_error_bars, default_color, bind_to_right_y2) = resolve_series_style(graph, c);
    let col_title = table.data_sets.get(c).map_or_else(|| format!("Col {}", c + 1), |d| d.title.clone());
    let row_label = row_display_label(&table.row_titles, r);
    let subcells = table.data_sets.get(c).and_then(|d| d.cells.get(r));
    let payload = if table.subcolumn_mode.is_pre_summarized() {
        let cells = subcells.map_or(&[][..], Vec::as_slice);
        let mut scratch = None;
        let eff_err = effective_presum_err_spec(&gdo, &y_error_bars, &mut scratch);
        if let Some((center, lower, upper)) = summarize_subcells(table.subcolumn_mode, cells, SummaryMeasure::Mean, eff_err, warnings) {
            ExtractedKind::PreSummarized { center, lower, upper, source: SourceCellId { sheet_id: sheet_id.clone(), row: r, col: c, subcol: 0 } }
        } else {
            ExtractedKind::Raw(Vec::new())
        }
    } else {
        let mut obs = Vec::new();
        if let Some(cells) = subcells {
            for (subcol, cell) in cells.iter().enumerate() {
                if let Some(value) = cell.active_number() {
                    obs.push(RawObs { value, source: SourceCellId { sheet_id: sheet_id.clone(), row: r, col: c, subcol } });
                }
            }
        }
        ExtractedKind::Raw(obs)
    };

    let idx = out.len();
    out.push(CategorySlotItem {
        group_key: idx,
        col_index: c,
        override_series_index: crate::override_series_index_for_col(&graph.spec, &graph.spec.primary_source_sheet_id, c),
        row_index: Some(r),
        match_label: format!("{col_title}:{row_label}"),
        alt_label: col_title,
        x_pos: slot.x_pos,
        slot_width_data: slot.slot_w,
        gdo,
        y_error_bars,
        default_color,
        bind_to_right_y2,
        payload,
        stack_group: slot.stack_group,
    });
}

fn collect_domain_values(
    graph: &GraphSheet,
    items: &[CategorySlotItem],
    tick_positions: &[(f64, String)],
    warnings: &mut Vec<ChartWarning>,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut x_vals = Vec::new();
    let mut y_vals = Vec::new();
    let mut y2_vals = Vec::new();

    for &(tx, _) in tick_positions {
        x_vals.push(tx - 0.5);
        x_vals.push(tx + 0.5);
    }
    let mut stack_pos: Vec<f64> = Vec::new();
    let mut stack_neg: Vec<f64> = Vec::new();

    for it in items {
        x_vals.push(it.x_pos - 0.6 * it.slot_width_data);
        x_vals.push(it.x_pos + 0.6 * it.slot_width_data);
        let target_y = if it.bind_to_right_y2 { &mut y2_vals } else { &mut y_vals };

        match &it.payload {
            ExtractedKind::PreSummarized { center, lower, upper, .. } => {
                if let Some(sg) = it.stack_group {
                    record_stack_extents(*center, *lower, *upper, sg, &mut stack_pos, &mut stack_neg, target_y);
                } else {
                    target_y.push(*center);
                    if it.y_error_bars.is_some()
                        || matches!(
                            it.gdo,
                            GraphicalDataObject::BoxAndWhisker { .. } | GraphicalDataObject::Violin { .. } | GraphicalDataObject::FloatingBar { .. }
                        )
                    {
                        target_y.push(*lower);
                        target_y.push(*upper);
                    }
                }
            }
            ExtractedKind::Raw(obs) => {
                let vals = raw_values(obs);
                for &v in &vals {
                    target_y.push(v);
                }
                let summary = gdo_summary_measure(&it.gdo);
                if let Some(center) = compute_central_tendency(&vals, summary, warnings) {
                    let (lo, hi) =
                        if let Some(eb) = &it.y_error_bars { compute_raw_error_bounds(&vals, center, eb.metric, 0.05) } else { (center, center) };
                    if let Some(sg) = it.stack_group {
                        record_stack_extents(center, lo, hi, sg, &mut stack_pos, &mut stack_neg, target_y);
                    } else {
                        target_y.push(lo);
                        target_y.push(hi);
                    }
                }
            }
        }
    }

    if x_vals.is_empty() && !items.is_empty() {
        x_vals.push(0.0);
        x_vals.push((items.len() as f64).max(1.0));
    }
    let _ = graph;
    (x_vals, y_vals, y2_vals)
}

fn record_stack_extents(center: f64, lo: f64, hi: f64, sg: usize, pos: &mut Vec<f64>, neg: &mut Vec<f64>, target: &mut Vec<f64>) {
    if pos.len() <= sg {
        pos.resize(sg + 1, 0.0);
        neg.resize(sg + 1, 0.0);
    }
    if center >= 0.0 {
        pos[sg] += center;
        target.push(pos[sg]);
        target.push(pos[sg] + (hi - center).max(0.0));
    } else {
        neg[sg] += center;
        target.push(neg[sg]);
        target.push(neg[sg] - (center - lo).max(0.0));
    }
}

const fn gdo_summary_measure(gdo: &GraphicalDataObject) -> SummaryMeasure {
    match gdo {
        GraphicalDataObject::BarWithReplicates { summary, .. } | GraphicalDataObject::AlignedDotPlot { summary, .. } => *summary,
        GraphicalDataObject::FloatingBar { summary: Some(s), .. } => *s,
        _ => SummaryMeasure::Mean,
    }
}

#[derive(Default)]
struct StackAccum {
    pos: Vec<f64>,
    neg: Vec<f64>,
}

#[derive(Clone, Copy)]
struct SlotGeom {
    cx: f64,
    width_pt: f64,
}

struct ConnectorAxes<'a> {
    x: &'a SegmentedAxis,
    y: &'a SegmentedAxis,
    y2: Option<&'a SegmentedAxis>,
}

fn render_before_after_connectors(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    sheet: &curvebook_doc::DataSheet,
    items: &[CategorySlotItem],
    axes: ConnectorAxes<'_>,
    bounds: &mut Vec<RenderedSlotBound>,
) {
    let mut horiz_unused = Vec::new();
    render_before_after_connectors_oriented(scene, graph_spec, sheet, items, axes, false, bounds, &mut horiz_unused);
}

#[allow(clippy::too_many_arguments)]
fn render_before_after_connectors_oriented(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    sheet: &curvebook_doc::DataSheet,
    items: &[CategorySlotItem],
    axes: ConnectorAxes<'_>,
    is_horizontal: bool,
    bounds: &mut Vec<RenderedSlotBound>,
    horiz_bounds: &mut Vec<HorizontalSlotBound>,
) {
    let mut ba_cols: Vec<(usize, &CategorySlotItem)> =
        items.iter().enumerate().filter(|(_, it)| matches!(it.gdo, GraphicalDataObject::BeforeAfterLines { .. })).collect();
    if ba_cols.is_empty() {
        return;
    }
    if ba_cols.len() < 2 {
        scene.warnings.push(ChartWarning::new("unsupported_before_after_pairing", "Before/After lines require at least two slots to connect"));
        return;
    }

    let (line_color, width_pt, default_marker) = match &ba_cols[0].1.gdo {
        GraphicalDataObject::BeforeAfterLines { line_color, stroke_width_pt, marker } => (*line_color, *stroke_width_pt, *marker),
        _ => return,
    };
    let clip = scene.plot_rect_pt;
    let stroke = StrokePaint::solid(line_color, width_pt.max(0.75));
    let mut ext_per_slot: Vec<f64> = vec![if is_horizontal { clip.x } else { clip.bottom() }; items.len()];

    struct BaObsPoint<'a> {
        col_index: usize,
        row: usize,
        subcol: usize,
        item: &'a CategorySlotItem,
        val: f64,
    }

    let mut emit_trajectory = |traj: &[BaObsPoint<'_>], slot_indices: &[usize]| {
        let mut pts: Vec<(usize, usize, usize, &CategorySlotItem, PointPt, f64)> = Vec::with_capacity(traj.len());
        for (idx_in_traj, obs) in traj.iter().enumerate() {
            let item = obs.item;
            let target_val = if !is_horizontal && item.bind_to_right_y2 { axes.y2.unwrap_or(axes.y) } else { axes.y };
            let mapped = if is_horizontal {
                match (target_val.map_value(obs.val), axes.x.map_value(item.x_pos)) {
                    (Some((_, px)), Some((_, py))) => Some(PointPt::new(px, py)),
                    _ => None,
                }
            } else {
                match (axes.x.map_value(item.x_pos), target_val.map_value(obs.val)) {
                    (Some((_, px)), Some((_, py))) => Some(PointPt::new(px, py)),
                    _ => None,
                }
            };
            if let Some(pt) = mapped {
                let slot_idx = slot_indices[idx_in_traj];
                if let Some(ext) = ext_per_slot.get_mut(slot_idx) {
                    *ext = if is_horizontal { ext.max(pt.x) } else { ext.min(pt.y) };
                }
                pts.push((obs.col_index, obs.row, obs.subcol, item, pt, obs.val));
            }
        }
        if pts.len() >= 2 {
            let mut cmds = Vec::with_capacity(pts.len());
            cmds.push(PathCmd::MoveTo(pts[0].4));
            for &(_, _, _, _, p, _) in &pts[1..] {
                cmds.push(PathCmd::LineTo(p));
            }
            scene.primitives.push(ScenePrimitive::Path { path: Path2D { commands: cmds }, fill: None, stroke: Some(stroke), clip: Some(clip) });
        }
        for &(data_col, row, subcol, item, p, val) in &pts {
            let item_marker = match &item.gdo {
                GraphicalDataObject::BeforeAfterLines { marker, .. } => *marker,
                _ => default_marker,
            };
            let Some(m) = item_marker else {
                continue;
            };
            let resolved = if let Some(series_index) = item.override_series_index {
                let Some(m) = crate::resolve_overridden_marker_for_cell(graph_spec, series_index, row, Some(subcol), m) else {
                    continue;
                };
                m
            } else {
                m
            };
            emit_marker(&mut scene.primitives, p, &resolved, Some(clip));
            scene.plotted_points.push(PlottedPointMeta {
                center_pt: p,
                radius_pt: (resolved.size_pt * 0.5).max(3.0),
                source_cell: SourceCellId { sheet_id: sheet.id.clone(), row, col: data_col, subcol },
                series_label: item.match_label.clone(),
                value_summary: format_point_value(val),
            });
        }
    };

    match &sheet.archetype {
        TableArchetype::Column(col_table) => {
            ba_cols.sort_by(|(_, a), (_, b)| a.x_pos.total_cmp(&b.x_pos).then_with(|| a.col_index.cmp(&b.col_index)));
            let slot_indices: Vec<usize> = ba_cols.iter().map(|&(idx, _)| idx).collect();
            for row in 0..col_table.row_count() {
                let mut traj = Vec::new();
                for &(_, item) in &ba_cols {
                    if let Some(val) =
                        col_table.data_sets.get(item.col_index).and_then(|d| d.cell(row, 0)).and_then(curvebook_doc::Cell::active_number)
                    {
                        traj.push(BaObsPoint { col_index: item.col_index, row, subcol: 0, item, val });
                    }
                }
                emit_trajectory(&traj, &slot_indices);
            }
        }
        TableArchetype::Grouped(grp_table) => {
            if grp_table.data_sets.len() >= 2 {
                let mut by_row: BTreeMap<usize, Vec<(usize, &CategorySlotItem)>> = BTreeMap::new();
                for &(slot_idx, item) in &ba_cols {
                    if let Some(r) = item.row_index {
                        by_row.entry(r).or_default().push((slot_idx, item));
                    }
                }
                let mut connected_any = false;
                for (row, mut row_slots) in by_row {
                    row_slots.sort_by(|(_, a), (_, b)| a.x_pos.total_cmp(&b.x_pos).then_with(|| a.col_index.cmp(&b.col_index)));
                    let distinct_x = row_slots.windows(2).any(|w| (w[0].1.x_pos - w[1].1.x_pos).abs() > 1e-6);
                    if row_slots.len() < 2 || !distinct_x {
                        continue;
                    }
                    connected_any = true;
                    let slot_indices: Vec<usize> = row_slots.iter().map(|&(idx, _)| idx).collect();
                    if grp_table.subcolumn_mode.is_pre_summarized() {
                        let traj: Vec<BaObsPoint<'_>> = row_slots
                            .iter()
                            .filter_map(|&(_, item)| match &item.payload {
                                ExtractedKind::PreSummarized { center, .. } if center.is_finite() => {
                                    Some(BaObsPoint { col_index: item.col_index, row, subcol: 0, item, val: *center })
                                }
                                _ => None,
                            })
                            .collect();
                        emit_trajectory(&traj, &slot_indices);
                    } else {
                        let max_sub = row_slots
                            .iter()
                            .map(|(_, it)| grp_table.data_sets.get(it.col_index).and_then(|d| d.cells.get(row)).map_or(0, Vec::len))
                            .max()
                            .unwrap_or(0);
                        for subcol in 0..max_sub {
                            let mut traj = Vec::new();
                            let mut used_slots = Vec::new();
                            for &(slot_idx, item) in &row_slots {
                                if let Some(val) = grp_table
                                    .data_sets
                                    .get(item.col_index)
                                    .and_then(|d| d.cell(row, subcol))
                                    .and_then(curvebook_doc::Cell::active_number)
                                {
                                    traj.push(BaObsPoint { col_index: item.col_index, row, subcol, item, val });
                                    used_slots.push(slot_idx);
                                }
                            }
                            emit_trajectory(&traj, &used_slots);
                        }
                    }
                }
                if !connected_any {
                    scene.warnings.push(ChartWarning::new(
                        "unsupported_before_after_pairing",
                        "Grouped Before/After lines require distinct category positions across paired columns (e.g. Interleaved or Separated)",
                    ));
                    return;
                }
            } else {
                ba_cols.sort_by(|(_, a), (_, b)| a.x_pos.total_cmp(&b.x_pos).then_with(|| a.row_index.cmp(&b.row_index)));
                let distinct_x = ba_cols.windows(2).any(|w| (w[0].1.x_pos - w[1].1.x_pos).abs() > 1e-6);
                if !distinct_x {
                    scene
                        .warnings
                        .push(ChartWarning::new("unsupported_before_after_pairing", "Before/After pairing requires distinct category positions"));
                    return;
                }
                let max_sub = if grp_table.subcolumn_mode.is_pre_summarized() {
                    1
                } else {
                    ba_cols
                        .iter()
                        .filter_map(|(_, it)| it.row_index.and_then(|r| grp_table.data_sets.get(it.col_index)?.cells.get(r).map(Vec::len)))
                        .max()
                        .unwrap_or(0)
                };
                for subcol in 0..max_sub {
                    let mut traj = Vec::new();
                    let mut used_slots = Vec::new();
                    for &(slot_idx, item) in &ba_cols {
                        let Some(row) = item.row_index else { continue };
                        let val_opt = if grp_table.subcolumn_mode.is_pre_summarized() {
                            match &item.payload {
                                ExtractedKind::PreSummarized { center, .. } if center.is_finite() => Some(*center),
                                _ => None,
                            }
                        } else {
                            grp_table.data_sets.get(item.col_index).and_then(|d| d.cell(row, subcol)).and_then(curvebook_doc::Cell::active_number)
                        };
                        if let Some(val) = val_opt {
                            traj.push(BaObsPoint { col_index: item.col_index, row, subcol, item, val });
                            used_slots.push(slot_idx);
                        }
                    }
                    emit_trajectory(&traj, &used_slots);
                }
            }
        }
        _ => {
            scene.warnings.push(ChartWarning::new("unsupported_before_after_pairing", "Before/After lines require a Column or Grouped table"));
            return;
        }
    }

    for &(slot_idx, item) in &ba_cols {
        if let Some((_, cat_pt)) = axes.x.map_value(item.x_pos) {
            let ext = ext_per_slot.get(slot_idx).copied().unwrap_or(if is_horizontal { clip.x } else { clip.bottom() });
            if is_horizontal {
                horiz_bounds.push(HorizontalSlotBound {
                    group_key: item.group_key,
                    label: item.match_label.clone(),
                    alt_label: item.alt_label.clone(),
                    y_pt: cat_pt,
                    right_x_pt: ext,
                });
            } else {
                bounds.push(RenderedSlotBound {
                    group_key: item.group_key,
                    label: item.match_label.clone(),
                    alt_label: item.alt_label.clone(),
                    x_pt: cat_pt,
                    top_y_pt: ext,
                });
            }
        }
    }
}

fn render_single_item_oriented(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    item: &CategorySlotItem,
    cat_axis: &SegmentedAxis,
    val_axis: &SegmentedAxis,
    stacks: &mut StackAccum,
    is_horizontal: bool,
) -> Option<f64> {
    let (_, cx) = cat_axis.map_value(item.x_pos)?;
    let (_, cat_lo, _) = cat_axis.map_clamped_to_nearest(item.x_pos - 0.5 * item.slot_width_data);
    let (_, cat_hi, _) = cat_axis.map_clamped_to_nearest(item.x_pos + 0.5 * item.slot_width_data);
    let slot = SlotGeom { cx, width_pt: (cat_hi - cat_lo).abs().max(4.0) };

    match &item.gdo {
        GraphicalDataObject::BeforeAfterLines { .. } => None,
        GraphicalDataObject::BarWithReplicates { .. } => render_bar_item_oriented(scene, graph_spec, item, slot, val_axis, stacks, is_horizontal),
        GraphicalDataObject::AlignedDotPlot { .. } | GraphicalDataObject::ScatterAndLine { .. } => {
            render_aligned_dot_item_oriented(scene, graph_spec, item, slot, val_axis, is_horizontal)
        }
        GraphicalDataObject::BoxAndWhisker { .. } => render_box_whisker_item_oriented(scene, graph_spec, item, slot, val_axis, is_horizontal),
        GraphicalDataObject::Violin { .. } => render_violin_item_oriented(scene, graph_spec, item, slot, val_axis, is_horizontal),
        GraphicalDataObject::FloatingBar { .. } => render_floating_bar_item_oriented(scene, item, slot, val_axis, is_horizontal),
        GraphicalDataObject::KaplanMeierStep { .. } | GraphicalDataObject::PartsOfWhole { .. } => None,
    }
}

fn render_bar_item_oriented(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    item: &CategorySlotItem,
    slot: SlotGeom,
    val_axis: &SegmentedAxis,
    stacks: &mut StackAccum,
    is_horizontal: bool,
) -> Option<f64> {
    let GraphicalDataObject::BarWithReplicates { summary, fill_color, border_color, border_width_pt, replicate_overlay } = item.gdo else {
        return None;
    };
    let cx = slot.cx;
    let slot_w_pt = slot.width_pt;
    let clip = scene.plot_rect_pt;
    let (center, err_lo, err_hi, raw_obs) = match &item.payload {
        ExtractedKind::PreSummarized { center, lower, upper, .. } => (*center, *lower, *upper, Vec::new()),
        ExtractedKind::Raw(obs) => {
            let vals = raw_values(obs);
            let c = compute_central_tendency(&vals, summary, &mut scene.warnings)?;
            let (lo, hi) = if let Some(eb) = &item.y_error_bars { compute_raw_error_bounds(&vals, c, eb.metric, 0.05) } else { (c, c) };
            (c, lo, hi, obs.clone())
        }
    };

    let (base_data, tip_data) = if let Some(sg) = item.stack_group {
        if stacks.pos.len() <= sg {
            stacks.pos.resize(sg + 1, 0.0);
            stacks.neg.resize(sg + 1, 0.0);
        }
        if center >= 0.0 {
            let b = stacks.pos[sg];
            stacks.pos[sg] = b + center;
            (b, b + center)
        } else {
            let b = stacks.neg[sg];
            stacks.neg[sg] = b + center;
            (b, b + center)
        }
    } else {
        let b = if let Some(seg) = val_axis.segments.first() {
            if is_log_like(seg.scale) {
                if center <= 0.0 {
                    scene.warnings.push(ChartWarning::new("non_positive_on_log_axis", format!("bar value {center} omitted on log axis")));
                    return None;
                }
                seg.domain_min
            } else if seg.domain_min <= 0.0 && seg.domain_max >= 0.0 {
                0.0
            } else {
                seg.domain_min
            }
        } else {
            0.0
        };
        (b, center)
    };

    let (_, base_pv, _) = val_axis.map_clamped_to_nearest(base_data);
    let (_, tip_pv, tip_clipped) = val_axis.map_clamped_to_nearest(tip_data);
    if tip_clipped && val_axis.segments.iter().any(|s| is_log_like(s.scale)) && tip_data <= 0.0 {
        scene.warnings.push(ChartWarning::new("non_positive_on_log_axis", format!("bar value {tip_data} omitted on log axis")));
        return None;
    }

    let v0 = base_pv.min(tip_pv);
    let v1 = base_pv.max(tip_pv);
    let bar_rect = if is_horizontal {
        RectPt::new(v0, cx - 0.5 * slot_w_pt, (v1 - v0).max(0.5), slot_w_pt)
    } else {
        RectPt::new(cx - 0.5 * slot_w_pt, v0, slot_w_pt, (v1 - v0).max(0.5))
    };
    let border = (border_width_pt > 0.0).then(|| StrokePaint::solid(border_color, border_width_pt));
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(bar_rect),
        fill: Some(FillPaint::solid(fill_color)),
        stroke: border,
        clip: Some(clip),
    });

    let mut extent_pt = if is_horizontal { v1 } else { v0 };
    if let Some(eb) = &item.y_error_bars {
        let mut eb_use = *eb;
        let shifted_lo = tip_data - (center - err_lo);
        let shifted_hi = tip_data + (err_hi - center);
        if item.stack_group.is_some() {
            eb_use.direction = if center >= 0.0 { ErrorBarDirection::UpperOnly } else { ErrorBarDirection::LowerOnly };
        }
        emit_directional_error_bar(
            scene,
            cx,
            tip_data,
            shifted_lo,
            shifted_hi,
            AxisErrorTarget { axis: val_axis, is_vertical: !is_horizontal, spec: &eb_use, default_color: border_color, clip },
        );
        if is_horizontal || !matches!(eb_use.direction, ErrorBarDirection::LowerOnly) {
            let (_, ehi_pv, _) = val_axis.map_clamped_to_nearest(shifted_hi);
            extent_pt = if is_horizontal { extent_pt.max(ehi_pv) } else { extent_pt.min(ehi_pv) };
        }
    }

    if let Some(overlay) = replicate_overlay
        && !raw_obs.is_empty()
        && item.stack_group.is_none()
    {
        let dot_style = MarkerStyle {
            shape: MarkerShape::Circle,
            fill_mode: curvebook_palette::MarkerFillMode::Filled,
            size_pt: 3.4,
            fill_color: border_color,
            stroke_color: Rgba8::WHITE,
            stroke_width_pt: 0.4,
        };
        let ctx = ReplicateDotCtx {
            spec: graph_spec,
            override_series_index: item.override_series_index,
            cx,
            max_half_w: slot_w_pt * 0.40,
            val_axis,
            style: overlay,
            marker: &dot_style,
            series_label: &item.match_label,
            is_horizontal,
        };
        if let Some(dot_ext) = emit_replicate_dots_oriented(scene, &raw_obs, &ctx) {
            extent_pt = if is_horizontal { extent_pt.max(dot_ext) } else { extent_pt.min(dot_ext) };
        }
    }

    Some(extent_pt)
}

fn render_aligned_dot_item_oriented(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    item: &CategorySlotItem,
    slot: SlotGeom,
    val_axis: &SegmentedAxis,
    is_horizontal: bool,
) -> Option<f64> {
    let (summary, marker, show_crossbar) = match &item.gdo {
        GraphicalDataObject::AlignedDotPlot { summary, marker, show_crossbar } => (*summary, *marker, *show_crossbar),
        GraphicalDataObject::ScatterAndLine { marker, .. } => {
            let m = marker.unwrap_or(MarkerStyle { fill_color: item.default_color, stroke_color: item.default_color, ..MarkerStyle::default() });
            (SummaryMeasure::Mean, m, false)
        }
        _ => return None,
    };
    let cx = slot.cx;
    let slot_w_pt = slot.width_pt;
    let clip = scene.plot_rect_pt;
    let raw_obs = match &item.payload {
        ExtractedKind::Raw(obs) => obs.clone(),
        ExtractedKind::PreSummarized { center, source, .. } => vec![RawObs { value: *center, source: source.clone() }],
    };
    let dot_ctx = ReplicateDotCtx {
        spec: graph_spec,
        override_series_index: item.override_series_index,
        cx,
        max_half_w: slot_w_pt * 0.42,
        val_axis,
        style: ReplicateOverlayStyle::DeterministicBeeswarm,
        marker: &marker,
        series_label: &item.match_label,
        is_horizontal,
    };
    let fallback_ext = if is_horizontal { clip.x } else { clip.bottom() };
    let mut extent_pt = emit_replicate_dots_oriented(scene, &raw_obs, &dot_ctx).unwrap_or(fallback_ext);

    let (center, lo, hi) = match &item.payload {
        ExtractedKind::PreSummarized { center, lower, upper, .. } => (*center, *lower, *upper),
        ExtractedKind::Raw(obs) => {
            let vals = raw_values(obs);
            let c = compute_central_tendency(&vals, summary, &mut scene.warnings)?;
            let (l, h) = if let Some(eb) = &item.y_error_bars { compute_raw_error_bounds(&vals, c, eb.metric, 0.05) } else { (c, c) };
            (c, l, h)
        }
    };
    if let Some((_, c_pv)) = val_axis.map_value(center) {
        extent_pt = if is_horizontal { extent_pt.max(c_pv) } else { extent_pt.min(c_pv) };
        if show_crossbar {
            let half_bar = (slot_w_pt * 0.35).max(5.0);
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(oriented_pt(cx, is_horizontal, -half_bar, c_pv), oriented_pt(cx, is_horizontal, half_bar, c_pv)),
                fill: None,
                stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.5)),
                clip: Some(clip),
            });
        }
    }
    if let Some(eb) = &item.y_error_bars {
        emit_directional_error_bar(
            scene,
            cx,
            center,
            lo,
            hi,
            AxisErrorTarget { axis: val_axis, is_vertical: !is_horizontal, spec: eb, default_color: Rgba8::BLACK, clip },
        );
        let (_, ehi_pv, _) = val_axis.map_clamped_to_nearest(hi);
        extent_pt = if is_horizontal { extent_pt.max(ehi_pv) } else { extent_pt.min(ehi_pv) };
    }
    Some(extent_pt)
}

fn sorted_obs_for_item(payload: &ExtractedKind) -> Vec<RawObs> {
    match payload {
        ExtractedKind::Raw(obs) => sorted_finite_obs(obs),
        ExtractedKind::PreSummarized { center, lower, upper, source } => {
            let mut out: Vec<RawObs> =
                [*lower, *center, *upper].into_iter().filter(|v| v.is_finite()).map(|value| RawObs { value, source: source.clone() }).collect();
            out.sort_by(|a, b| a.value.total_cmp(&b.value));
            out
        }
    }
}

fn emit_clipped_domain_warning(warnings: &mut Vec<ChartWarning>, axis: &SegmentedAxis, vals: &[f64]) {
    let has_log = axis.segments.iter().any(|s| is_log_like(s.scale));
    if has_log && vals.iter().any(|&v| v <= 0.0) {
        warnings.push(ChartWarning::new("log_axis_floor_clip", "clipped non-positive quantile/bound to lower floor on logarithmic axis"));
    } else if vals.iter().any(|&v| axis.map_value(v).is_none()) {
        warnings.push(ChartWarning::new("axis_domain_clip", "clipped quantile/bound to nearest axis segment boundary"));
    }
}

fn oriented_pt(cx: f64, is_horizontal: bool, cat_off: f64, val_pt: f64) -> PointPt {
    if is_horizontal { PointPt::new(val_pt, cx + cat_off) } else { PointPt::new(cx + cat_off, val_pt) }
}

fn oriented_span_rect(cx: f64, is_horizontal: bool, half_cat_w: f64, v0_pt: f64, v1_pt: f64) -> RectPt {
    let lo = v0_pt.min(v1_pt);
    let len = (v1_pt - v0_pt).abs();
    if is_horizontal {
        RectPt::new(lo, cx - half_cat_w, len.max(0.8), 2.0 * half_cat_w)
    } else {
        RectPt::new(cx - half_cat_w, lo, 2.0 * half_cat_w, len.max(0.8))
    }
}

/// Orientation-parameterized box-and-whisker configuration shared across column, grouped, and
/// multivariate categorical summary renderers.
pub(crate) struct BoxWhiskerRenderCfg<'a> {
    pub(crate) spec: &'a GraphSpec,
    pub(crate) override_series_index: Option<usize>,
    pub(crate) cx: f64,
    pub(crate) box_w: f64,
    pub(crate) whiskers: WhiskerMethod,
    pub(crate) fill_color: Rgba8,
    pub(crate) stroke_color: Rgba8,
    pub(crate) show_outliers: bool,
    pub(crate) show_all_points: bool,
    pub(crate) val_axis: &'a SegmentedAxis,
    pub(crate) series_label: &'a str,
    pub(crate) is_horizontal: bool,
}

fn render_box_whisker_item_oriented(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    item: &CategorySlotItem,
    slot: SlotGeom,
    val_axis: &SegmentedAxis,
    is_horizontal: bool,
) -> Option<f64> {
    let GraphicalDataObject::BoxAndWhisker { whiskers, fill_color, stroke_color, show_outliers, show_all_points } = item.gdo else {
        return None;
    };
    let sorted_obs = sorted_obs_for_item(&item.payload);
    render_box_whisker_obs(
        scene,
        &sorted_obs,
        &BoxWhiskerRenderCfg {
            spec: graph_spec,
            override_series_index: item.override_series_index,
            cx: slot.cx,
            box_w: slot.width_pt * 0.72,
            whiskers,
            fill_color,
            stroke_color,
            show_outliers,
            show_all_points,
            val_axis,
            series_label: &item.match_label,
            is_horizontal,
        },
    )
}

pub(crate) fn render_box_whisker_obs(scene: &mut ChartScene, sorted_obs: &[RawObs], cfg: &BoxWhiskerRenderCfg<'_>) -> Option<f64> {
    if sorted_obs.is_empty() {
        return None;
    }
    let cx = cfg.cx;
    let box_w = cfg.box_w;
    let val_axis = cfg.val_axis;
    let is_horizontal = cfg.is_horizontal;
    let stroke_color = cfg.stroke_color;
    let clip = scene.plot_rect_pt;
    let sorted = raw_values(sorted_obs);
    let (q1, median, q3) = quartiles_hinge(&sorted)?;
    let (w_lo, w_hi, outlier_indices) = compute_box_whiskers(&sorted, q1, q3, cfg.whiskers);

    let (_, py_q1, c_q1) = val_axis.map_clamped_to_nearest(q1);
    let (_, py_med, c_med) = val_axis.map_clamped_to_nearest(median);
    let (_, py_q3, c_q3) = val_axis.map_clamped_to_nearest(q3);
    let (_, py_wlo, c_wlo) = val_axis.map_clamped_to_nearest(w_lo);
    let (_, py_whi, c_whi) = val_axis.map_clamped_to_nearest(w_hi);
    if c_q1 || c_med || c_q3 || c_wlo || c_whi {
        emit_clipped_domain_warning(&mut scene.warnings, val_axis, &[q1, median, q3, w_lo, w_hi]);
    }

    let cap_half = box_w * 0.28;
    let stroke = StrokePaint::solid(stroke_color, 1.1);

    // Lower & upper whisker stems and caps
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(oriented_pt(cx, is_horizontal, 0.0, py_q1), oriented_pt(cx, is_horizontal, 0.0, py_wlo)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(clip),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(oriented_pt(cx, is_horizontal, -cap_half, py_wlo), oriented_pt(cx, is_horizontal, cap_half, py_wlo)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(clip),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(oriented_pt(cx, is_horizontal, 0.0, py_q3), oriented_pt(cx, is_horizontal, 0.0, py_whi)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(clip),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(oriented_pt(cx, is_horizontal, -cap_half, py_whi), oriented_pt(cx, is_horizontal, cap_half, py_whi)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(clip),
    });

    // Interquartile box & median line
    let box_rect = oriented_span_rect(cx, is_horizontal, 0.5 * box_w, py_q1, py_q3);
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(box_rect),
        fill: Some(FillPaint::solid(cfg.fill_color)),
        stroke: Some(stroke),
        clip: Some(clip),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(oriented_pt(cx, is_horizontal, -0.5 * box_w, py_med), oriented_pt(cx, is_horizontal, 0.5 * box_w, py_med)),
        fill: None,
        stroke: Some(StrokePaint::solid(stroke_color, 1.75)),
        clip: Some(clip),
    });

    let mut extent_pt = if is_horizontal { py_whi.max(box_rect.right()) } else { py_whi.min(box_rect.y) };
    let dot_style = MarkerStyle {
        shape: MarkerShape::Circle,
        fill_mode: curvebook_palette::MarkerFillMode::Filled,
        size_pt: 3.4,
        fill_color: stroke_color,
        stroke_color,
        stroke_width_pt: 0.5,
    };
    if cfg.show_all_points {
        let dot_ctx = ReplicateDotCtx {
            spec: cfg.spec,
            override_series_index: cfg.override_series_index,
            cx,
            max_half_w: box_w * 0.42,
            val_axis,
            style: ReplicateOverlayStyle::DeterministicBeeswarm,
            marker: &dot_style,
            series_label: cfg.series_label,
            is_horizontal,
        };
        if let Some(dot_ext) = emit_replicate_dots_oriented(scene, sorted_obs, &dot_ctx) {
            extent_pt = if is_horizontal { extent_pt.max(dot_ext) } else { extent_pt.min(dot_ext) };
        }
    } else if cfg.show_outliers {
        for idx in outlier_indices {
            let obs = &sorted_obs[idx];
            let resolved = if let Some(series_index) = cfg.override_series_index {
                let Some(m) = crate::resolve_overridden_marker_for_cell(cfg.spec, series_index, obs.source.row, Some(obs.source.subcol), dot_style)
                else {
                    continue;
                };
                m
            } else {
                dot_style
            };
            if let Some((_, p_val)) = val_axis.map_value(obs.value) {
                let dot_pt = oriented_pt(cx, is_horizontal, 0.0, p_val);
                emit_marker(&mut scene.primitives, dot_pt, &resolved, Some(clip));
                push_col_point_meta(scene, dot_pt, resolved.size_pt, obs, cfg.series_label);
                extent_pt = if is_horizontal { extent_pt.max(p_val + resolved.size_pt * 0.5) } else { extent_pt.min(p_val) };
            }
        }
    }
    Some(extent_pt)
}

fn compute_box_whiskers(sorted: &[f64], q1: f64, q3: f64, method: WhiskerMethod) -> (f64, f64, Vec<usize>) {
    let min_v = sorted.first().copied().unwrap_or(q1);
    let max_v = sorted.last().copied().unwrap_or(q3);
    match method {
        WhiskerMethod::MinToMax => (min_v, max_v, Vec::new()),
        WhiskerMethod::Tukey1_5Iqr => {
            let iqr = (q3 - q1).max(0.0);
            let lo_fence = q1 - 1.5 * iqr;
            let hi_fence = q3 + 1.5 * iqr;
            let w_lo = sorted.iter().copied().find(|&v| v >= lo_fence).unwrap_or(q1);
            let w_hi = sorted.iter().rev().copied().find(|&v| v <= hi_fence).unwrap_or(q3);
            let outliers = sorted.iter().enumerate().filter_map(|(idx, &v)| (v < w_lo || v > w_hi).then_some(idx)).collect();
            (w_lo, w_hi, outliers)
        }
        WhiskerMethod::Percentile10To90 => percentile_whiskers(sorted, 0.10, 0.90),
        WhiskerMethod::Percentile5To95 => percentile_whiskers(sorted, 0.05, 0.95),
        WhiskerMethod::Percentile2_5To97_5 => percentile_whiskers(sorted, 0.025, 0.975),
        WhiskerMethod::Percentile1To99 => percentile_whiskers(sorted, 0.01, 0.99),
    }
}

fn percentile_whiskers(sorted: &[f64], p_lo: f64, p_hi: f64) -> (f64, f64, Vec<usize>) {
    let w_lo = quantile_sorted(sorted, p_lo).unwrap_or(0.0);
    let w_hi = quantile_sorted(sorted, p_hi).unwrap_or(w_lo);
    let outliers = sorted.iter().enumerate().filter_map(|(idx, &v)| (v < w_lo - 1e-12 || v > w_hi + 1e-12).then_some(idx)).collect();
    (w_lo, w_hi, outliers)
}

/// Orientation-parameterized KDE violin configuration shared across column, grouped, and
/// multivariate categorical summary renderers.
pub(crate) struct ViolinRenderCfg<'a> {
    pub(crate) spec: &'a GraphSpec,
    pub(crate) override_series_index: Option<usize>,
    pub(crate) cx: f64,
    pub(crate) half_max_w: f64,
    pub(crate) fill_color: Rgba8,
    pub(crate) stroke_color: Rgba8,
    pub(crate) clip_to_observed_range: bool,
    pub(crate) show_median_and_quartiles: bool,
    pub(crate) half_violin_with_points: bool,
    pub(crate) val_axis: &'a SegmentedAxis,
    pub(crate) series_label: &'a str,
    pub(crate) is_horizontal: bool,
}

fn render_violin_item_oriented(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    item: &CategorySlotItem,
    slot: SlotGeom,
    val_axis: &SegmentedAxis,
    is_horizontal: bool,
) -> Option<f64> {
    let GraphicalDataObject::Violin { fill_color, stroke_color, clip_to_observed_range, show_median_and_quartiles, half_violin_with_points } =
        item.gdo
    else {
        return None;
    };
    let sorted_obs = sorted_obs_for_item(&item.payload);
    render_violin_obs(
        scene,
        &sorted_obs,
        &ViolinRenderCfg {
            spec: graph_spec,
            override_series_index: item.override_series_index,
            cx: slot.cx,
            half_max_w: slot.width_pt * 0.45,
            fill_color,
            stroke_color,
            clip_to_observed_range,
            show_median_and_quartiles,
            half_violin_with_points,
            val_axis,
            series_label: &item.match_label,
            is_horizontal,
        },
    )
}

pub(crate) fn render_violin_obs(scene: &mut ChartScene, sorted_obs: &[RawObs], cfg: &ViolinRenderCfg<'_>) -> Option<f64> {
    if sorted_obs.is_empty() {
        return None;
    }
    let cx = cfg.cx;
    let half_max_w = cfg.half_max_w;
    let val_axis = cfg.val_axis;
    let is_horizontal = cfg.is_horizontal;
    let stroke_color = cfg.stroke_color;
    let clip = scene.plot_rect_pt;
    let sorted = raw_values(sorted_obs);

    let (q1, median, q3) = quartiles_hinge(&sorted)?;

    let profile = build_kde_profile(&sorted, q1, q3, cfg.clip_to_observed_range, val_axis);
    if profile.len() < 2 {
        let (_, py, clipped) = val_axis.map_clamped_to_nearest(median);
        if clipped {
            emit_clipped_domain_warning(&mut scene.warnings, val_axis, &[median]);
        }
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(oriented_pt(cx, is_horizontal, -0.5 * half_max_w, py), oriented_pt(cx, is_horizontal, 0.5 * half_max_w, py)),
            fill: None,
            stroke: Some(StrokePaint::solid(stroke_color, 1.5)),
            clip: Some(clip),
        });
        return Some(py);
    }

    let mut right_pts: Vec<PointPt> = Vec::with_capacity(profile.len());
    let mut left_pts: Vec<PointPt> = Vec::with_capacity(profile.len());
    let mut extent_pt = if is_horizontal { clip.x } else { clip.bottom() };
    for &(yv, norm_d) in &profile {
        if let Some((_, pv)) = val_axis.map_value(yv) {
            let dx = norm_d * half_max_w;
            right_pts.push(oriented_pt(cx, is_horizontal, dx, pv));
            left_pts.push(oriented_pt(cx, is_horizontal, if cfg.half_violin_with_points { 0.0 } else { -dx }, pv));
            extent_pt = if is_horizontal { extent_pt.max(pv) } else { extent_pt.min(pv) };
        }
    }

    if right_pts.len() >= 2 {
        let mut contour = Vec::with_capacity(right_pts.len() + left_pts.len());
        contour.extend_from_slice(&right_pts);
        for &lp in left_pts.iter().rev() {
            contour.push(lp);
        }
        scene.primitives.push(ScenePrimitive::Path {
            path: polygon_path(&contour),
            fill: Some(FillPaint::solid(cfg.fill_color)),
            stroke: Some(StrokePaint::solid(stroke_color, 1.0)),
            clip: Some(clip),
        });
    }

    if cfg.show_median_and_quartiles {
        for (val, width, dash) in [(median, 1.75, LineDashStyle::Solid), (q1, 0.9, LineDashStyle::Dashed), (q3, 0.9, LineDashStyle::Dashed)] {
            if let Some((_, pv)) = val_axis.map_value(val) {
                let d_at_val = interpolate_profile_width(&profile, val) * half_max_w;
                let off0 = if cfg.half_violin_with_points { 0.0 } else { -d_at_val.max(2.0) };
                let off1 = d_at_val.max(2.0);
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(oriented_pt(cx, is_horizontal, off0, pv), oriented_pt(cx, is_horizontal, off1, pv)),
                    fill: None,
                    stroke: Some(stroke_with_dash(stroke_color, width, dash)),
                    clip: Some(clip),
                });
            }
        }
    }

    if cfg.half_violin_with_points {
        let dot_style = MarkerStyle {
            shape: MarkerShape::Circle,
            fill_mode: curvebook_palette::MarkerFillMode::Filled,
            size_pt: 3.4,
            fill_color: stroke_color,
            stroke_color: Rgba8::WHITE,
            stroke_width_pt: 0.4,
        };
        let half_center = cx - 0.52 * half_max_w;
        let dot_ctx = ReplicateDotCtx {
            spec: cfg.spec,
            override_series_index: cfg.override_series_index,
            cx: half_center,
            max_half_w: 0.40 * half_max_w,
            val_axis,
            style: ReplicateOverlayStyle::DeterministicBeeswarm,
            marker: &dot_style,
            series_label: cfg.series_label,
            is_horizontal,
        };
        if let Some(dot_ext) = emit_replicate_dots_oriented(scene, sorted_obs, &dot_ctx) {
            extent_pt = if is_horizontal { extent_pt.max(dot_ext) } else { extent_pt.min(dot_ext) };
        }
    }

    Some(extent_pt)
}

fn build_kde_profile(sorted: &[f64], q1: f64, q3: f64, clip_to_observed: bool, y_axis: &SegmentedAxis) -> Vec<(f64, f64)> {
    let n = sorted.len();
    let min_v = sorted[0];
    let max_v = sorted[n - 1];
    let sd = SummaryMoments::from_slice(sorted).ok().and_then(|m| m.sample_sd().ok()).unwrap_or(0.0);
    let iqr = (q3 - q1).max(0.0);
    let spread = if sd > 0.0 && iqr > 0.0 {
        sd.min(iqr / 1.34)
    } else if sd > 0.0 {
        sd
    } else {
        (max_v - min_v).abs() * 0.25
    };
    let h = if spread > 0.0 { 0.9 * spread * (n as f64).powf(-0.2) } else { (min_v.abs() * 0.05).max(0.1) };

    let mut lo = if clip_to_observed { min_v } else { min_v - 2.0 * h };
    let mut hi = if clip_to_observed { max_v } else { max_v + 2.0 * h };
    if !y_axis.segments.is_empty() {
        let axis_min = y_axis.segments.iter().map(|s| s.domain_min.min(s.domain_max)).fold(f64::INFINITY, f64::min);
        let axis_max = y_axis.segments.iter().map(|s| s.domain_min.max(s.domain_max)).fold(f64::NEG_INFINITY, f64::max);
        if y_axis.segments.iter().any(|s| is_log_like(s.scale)) {
            let floor_v = if min_v > 0.0 { axis_min.max(min_v * 0.85) } else { axis_min };
            lo = lo.max(floor_v);
        } else {
            lo = lo.max(axis_min);
        }
        hi = hi.min(axis_max);
    }
    if hi <= lo {
        return Vec::new();
    }

    let inv_norm = 1.0 / ((n as f64) * h * (2.0 * std::f64::consts::PI).sqrt());
    let mut raw = Vec::with_capacity(KDE_SAMPLES);
    let mut max_d = 0.0_f64;
    for i in 0..KDE_SAMPLES {
        let y = lo + (hi - lo) * ((i as f64) / ((KDE_SAMPLES - 1) as f64));
        let mut sum_k = 0.0;
        for &obs in sorted {
            let z = (y - obs) / h;
            sum_k += (-0.5 * z * z).exp();
        }
        let d = sum_k * inv_norm;
        max_d = max_d.max(d);
        raw.push((y, d));
    }
    if max_d <= 0.0 {
        return raw.into_iter().map(|(y, _)| (y, 0.2)).collect();
    }
    raw.into_iter().map(|(y, d)| (y, d / max_d)).collect()
}

fn interpolate_profile_width(profile: &[(f64, f64)], target_y: f64) -> f64 {
    let idx = profile.partition_point(|&(y, _)| y <= target_y);
    if idx == 0 {
        return profile.first().map_or(0.5, |p| p.1);
    }
    if idx >= profile.len() {
        return profile.last().map_or(0.5, |p| p.1);
    }
    let (y0, d0) = profile[idx - 1];
    let (y1, d1) = profile[idx];
    let span = y1 - y0;
    let w = if span.abs() > 1e-12 { crate::clamp_f64_safe((target_y - y0) / span, 0.0, 1.0) } else { 0.0 };
    d0 + w * (d1 - d0)
}

/// Orientation-parameterized floating-bar configuration shared across column, grouped, and
/// multivariate categorical summary renderers.
pub(crate) struct FloatingBarRenderCfg<'a> {
    pub(crate) cx: f64,
    pub(crate) slot_w_pt: f64,
    pub(crate) fill_color: Rgba8,
    pub(crate) border_color: Rgba8,
    pub(crate) border_width_pt: f64,
    pub(crate) val_axis: &'a SegmentedAxis,
    pub(crate) is_horizontal: bool,
}

fn render_floating_bar_item_oriented(
    scene: &mut ChartScene,
    item: &CategorySlotItem,
    slot: SlotGeom,
    val_axis: &SegmentedAxis,
    is_horizontal: bool,
) -> Option<f64> {
    let GraphicalDataObject::FloatingBar { summary, fill_color, border_color, border_width_pt } = item.gdo else {
        return None;
    };
    let (lo, hi, mid) = match &item.payload {
        ExtractedKind::PreSummarized { center, lower, upper, .. } => (lower.min(*upper), lower.max(*upper), summary.map(|_| *center)),
        ExtractedKind::Raw(obs) => {
            let vals = raw_values(obs);
            let sorted = sorted_finite(&vals);
            if sorted.is_empty() {
                return None;
            }
            let min_v = sorted[0];
            let max_v = sorted[sorted.len() - 1];
            let mid_v = summary.and_then(|s| compute_central_tendency(&sorted, s, &mut scene.warnings));
            (min_v, max_v, mid_v)
        }
    };
    render_floating_bar_bounds(
        scene,
        lo,
        hi,
        mid,
        &FloatingBarRenderCfg { cx: slot.cx, slot_w_pt: slot.width_pt, fill_color, border_color, border_width_pt, val_axis, is_horizontal },
    )
}

pub(crate) fn render_floating_bar_bounds(scene: &mut ChartScene, lo: f64, hi: f64, mid: Option<f64>, cfg: &FloatingBarRenderCfg<'_>) -> Option<f64> {
    let clip = scene.plot_rect_pt;
    let (_, py_lo, c_lo) = cfg.val_axis.map_clamped_to_nearest(lo);
    let (_, py_hi, c_hi) = cfg.val_axis.map_clamped_to_nearest(hi);
    if c_lo || c_hi {
        emit_clipped_domain_warning(&mut scene.warnings, cfg.val_axis, &[lo, hi]);
    }
    let rect = oriented_span_rect(cfg.cx, cfg.is_horizontal, 0.5 * cfg.slot_w_pt, py_lo, py_hi);
    let border = (cfg.border_width_pt > 0.0).then(|| StrokePaint::solid(cfg.border_color, cfg.border_width_pt));
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(rect),
        fill: Some(FillPaint::solid(cfg.fill_color)),
        stroke: border,
        clip: Some(clip),
    });

    if let Some(m) = mid
        && let Some((_, pv_m)) = cfg.val_axis.map_value(m)
    {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(
                oriented_pt(cfg.cx, cfg.is_horizontal, -0.5 * cfg.slot_w_pt, pv_m),
                oriented_pt(cfg.cx, cfg.is_horizontal, 0.5 * cfg.slot_w_pt, pv_m),
            ),
            fill: None,
            stroke: Some(StrokePaint::solid(cfg.border_color, cfg.border_width_pt.max(1.2))),
            clip: Some(clip),
        });
    }
    Some(if cfg.is_horizontal { rect.right() } else { rect.y })
}

pub(crate) struct ReplicateDotCtx<'a> {
    pub(crate) spec: &'a GraphSpec,
    pub(crate) override_series_index: Option<usize>,
    pub(crate) cx: f64,
    pub(crate) max_half_w: f64,
    pub(crate) val_axis: &'a SegmentedAxis,
    pub(crate) style: ReplicateOverlayStyle,
    pub(crate) marker: &'a MarkerStyle,
    pub(crate) series_label: &'a str,
    pub(crate) is_horizontal: bool,
}

fn push_col_point_meta(scene: &mut ChartScene, center_pt: PointPt, marker_size_pt: f64, obs: &RawObs, series_label: &str) {
    scene.plotted_points.push(PlottedPointMeta {
        center_pt,
        radius_pt: (marker_size_pt * 0.5).max(3.0),
        source_cell: obs.source.clone(),
        series_label: series_label.to_owned(),
        value_summary: format_point_value(obs.value),
    });
}

const MAX_REPLICATE_MARKERS: usize = 4_096;

fn subsample_sorted_indices(pts_v: &[f64], cap: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..pts_v.len()).collect();
    order.sort_by(|&a, &b| pts_v[a].total_cmp(&pts_v[b]).then_with(|| a.cmp(&b)));
    if order.len() <= cap || cap < 2 {
        return order.into_iter().take(cap).collect();
    }
    let last = (order.len() - 1) as u64;
    let denom = (cap - 1) as u64;
    (0..cap).map(|i| order[((i as u64 * last) / denom) as usize]).collect()
}

pub(crate) fn emit_replicate_dots_oriented(scene: &mut ChartScene, obs: &[RawObs], ctx: &ReplicateDotCtx<'_>) -> Option<f64> {
    let clip = scene.plot_rect_pt;
    let mut mapped: Vec<(f64, &RawObs, MarkerStyle)> = Vec::with_capacity(obs.len());
    let mut pts_v: Vec<f64> = Vec::with_capacity(obs.len());
    let mut max_dot_r = (ctx.marker.size_pt * 0.5).max(1.2);
    for o in obs {
        let resolved = if let Some(series_index) = ctx.override_series_index {
            let Some(m) = crate::resolve_overridden_marker_for_cell(ctx.spec, series_index, o.source.row, Some(o.source.subcol), *ctx.marker) else {
                continue;
            };
            m
        } else {
            *ctx.marker
        };
        if let Some((_, pv)) = ctx.val_axis.map_value(o.value) {
            max_dot_r = max_dot_r.max(resolved.size_pt * 0.5);
            mapped.push((pv, o, resolved));
            pts_v.push(pv);
        }
    }
    if mapped.is_empty() {
        return None;
    }

    if mapped.len() > MAX_REPLICATE_MARKERS {
        let total = mapped.len();
        let keep = subsample_sorted_indices(&pts_v, MAX_REPLICATE_MARKERS);
        mapped = keep.iter().map(|&idx| mapped[idx]).collect();
        pts_v = keep.iter().map(|&idx| pts_v[idx]).collect();
        if !scene.warnings.iter().any(|w| w.code == "replicate_markers_capped") {
            scene.warnings.push(ChartWarning::new(
                "replicate_markers_capped",
                format!("replicate overlay capped to {MAX_REPLICATE_MARKERS} representative markers (from {total} points)"),
            ));
        }
    }

    let offsets = match ctx.style {
        ReplicateOverlayStyle::AlignedColumn => vec![0.0; mapped.len()],
        ReplicateOverlayStyle::DeterministicBeeswarm => deterministic_beeswarm_offsets(&pts_v, ctx.max_half_w, max_dot_r),
    };
    let mut extent_pt = if ctx.is_horizontal { clip.x } else { clip.bottom() };
    for (&(pv, raw, resolved), off) in mapped.iter().zip(offsets.iter()) {
        let dot_pt = oriented_pt(ctx.cx, ctx.is_horizontal, *off, pv);
        emit_marker(&mut scene.primitives, dot_pt, &resolved, Some(clip));
        push_col_point_meta(scene, dot_pt, resolved.size_pt, raw, ctx.series_label);
        extent_pt = if ctx.is_horizontal { extent_pt.max(pv + resolved.size_pt * 0.5) } else { extent_pt.min(pv - max_dot_r) };
    }
    Some(extent_pt)
}

struct BeeswarmState {
    lane_last_y: Vec<f64>,
    fallback_last_y: [f64; 2],
    fallback_x: [f64; 2],
    max_steps: isize,
    neighbor_span: isize,
    step: f64,
    min_dist: f64,
}

impl BeeswarmState {
    fn hits(&self, dx: f64, y: f64, lane: isize) -> bool {
        let min_dist_sq = self.min_dist * self.min_dist;
        let lo = (lane - self.neighbor_span).max(-self.max_steps);
        let hi = (lane + self.neighbor_span).min(self.max_steps);
        for m in lo..=hi {
            let py = self.lane_last_y[(m + self.max_steps) as usize];
            let dy = y - py;
            if py.is_finite() && dy < self.min_dist {
                let ddx = dx - (m as f64) * self.step;
                if ddx * ddx + dy * dy < min_dist_sq {
                    return true;
                }
            }
        }
        self.fallback_x.iter().zip(self.fallback_last_y.iter()).any(|(&fx, &py)| {
            let dy = y - py;
            py.is_finite() && dy < self.min_dist && {
                let ddx = dx - fx;
                ddx * ddx + dy * dy < min_dist_sq
            }
        })
    }
}

fn deterministic_beeswarm_offsets(ys: &[f64], max_half_w: f64, dot_r: f64) -> Vec<f64> {
    if ys.is_empty() {
        return Vec::new();
    }
    if !max_half_w.is_finite() || max_half_w <= 0.0 {
        return vec![0.0; ys.len()];
    }

    let mut order: Vec<usize> = (0..ys.len()).filter(|&i| ys[i].is_finite()).collect();
    order.sort_by(|&a, &b| ys[a].total_cmp(&ys[b]).then_with(|| a.cmp(&b)));

    let safe_r = if dot_r.is_finite() && dot_r > 0.0 { dot_r } else { 1.0 };
    let min_dist = (2.15 * safe_r).max(2.0);
    let step = min_dist * 0.85;
    let max_steps = ((max_half_w / step).floor() as isize).clamp(1, 128);
    let mut state = BeeswarmState {
        lane_last_y: vec![f64::NEG_INFINITY; (max_steps * 2 + 1) as usize],
        fallback_last_y: [f64::NEG_INFINITY; 2],
        fallback_x: [max_half_w * 0.75, -max_half_w * 0.75],
        max_steps,
        neighbor_span: (min_dist / step).ceil() as isize + 1,
        step,
        min_dist,
    };

    let mut out = vec![0.0; ys.len()];
    let mut last_saturated_y: Option<f64> = None;

    for (placed_count, idx) in order.into_iter().enumerate() {
        let y = ys[idx];
        let mut chosen_lane: Option<(isize, f64)> = None;

        if last_saturated_y != Some(y) {
            'search: for k in 0..=state.max_steps {
                let candidates: [isize; 2] = if k == 0 { [0, 0] } else { [-k, k] };
                let limit = if k == 0 { 1 } else { 2 };
                for &lane in &candidates[..limit] {
                    let dx = (lane as f64) * state.step;
                    if dx.abs() <= max_half_w + 1e-6 && !state.hits(dx, y, lane) {
                        chosen_lane = Some((lane, dx));
                        break 'search;
                    }
                }
            }
        }

        let chosen_dx = if let Some((lane, dx)) = chosen_lane {
            state.lane_last_y[(lane + state.max_steps) as usize] = y;
            dx
        } else {
            last_saturated_y = Some(y);
            let fi = if placed_count.is_multiple_of(2) { 0 } else { 1 };
            state.fallback_last_y[fi] = y;
            state.fallback_x[fi]
        };
        out[idx] = chosen_dx;
    }
    out
}

fn item_summary_value(item: &CategorySlotItem) -> Option<f64> {
    match &item.payload {
        ExtractedKind::PreSummarized { center, .. } => center.is_finite().then_some(*center),
        ExtractedKind::Raw(obs) => {
            let vals = raw_values(obs);
            let mut unused = Vec::new();
            compute_central_tendency(&vals, gdo_summary_measure(&item.gdo), &mut unused)
        }
    }
}

pub(crate) fn cld_reserved_top_margin_pt(spec: &GraphSpec) -> f64 {
    if !matches!(spec.pairwise_display_mode, PairwiseDisplayMode::CompactLetterDisplayOnly | PairwiseDisplayMode::BracketsAndCompactLetterDisplay) {
        return 0.0;
    }
    let Some(cld) = &spec.compact_letter_display else {
        return 0.0;
    };
    let font = if cld.font_size_pt.is_finite() && cld.font_size_pt > 0.0 { cld.font_size_pt } else { 9.5 };
    let offset = if cld.offset_pt.is_finite() && cld.offset_pt >= 0.0 { cld.offset_pt } else { 6.0 };
    (font + offset + 6.0).clamp(16.0, 34.0)
}

fn collect_cld_means_override(res: &curvebook_doc::ResultSheet, slot_metas: &[(usize, &str, &str, Option<f64>, f64)]) -> Option<Vec<(usize, f64)>> {
    let needs_override = res.pairwise_comparisons.iter().any(|r| r.estimate_diff.is_none()) || matches!(res.analysis.kind, AnalysisKind::Survival(_));
    if !needs_override {
        return None;
    }
    let mut seen: BTreeMap<usize, String> = BTreeMap::new();
    for rec in &res.pairwise_comparisons {
        seen.entry(rec.group_a).or_insert_with(|| rec.label_a.clone());
        seen.entry(rec.group_b).or_insert_with(|| rec.label_b.clone());
    }
    let mut out = Vec::new();
    for (idx, lbl) in seen {
        if let Some(pos) = find_matched_index(slot_metas, idx, Some(lbl.as_str()), |m| (m.0, m.1, m.2))
            && let Some(mean) = slot_metas[pos].3
            && mean.is_finite()
        {
            out.push((idx, mean));
        }
    }
    (!out.is_empty()).then_some(out)
}

fn match_cld_assignments_to_slots(
    assignments: &[curvebook_stats::cld::CldGroupAssignment],
    slot_metas: &[(usize, &str, &str, Option<f64>, f64)],
    grouped_arrangement: GroupedArrangement,
    warnings: &mut Vec<ChartWarning>,
) -> Option<Vec<ResolvedCldSlot>> {
    let mut matched_slots: Vec<(usize, f64)> = Vec::new();
    let mut labels = Vec::new();

    for assign in assignments {
        let Some(pos) = find_matched_index(slot_metas, assign.group_index, Some(assign.label.as_str()), |m| (m.0, m.1, m.2)) else {
            continue;
        };
        let x_pos = slot_metas[pos].4;
        if let Some(&(_, prev_x)) = matched_slots.iter().find(|&&(prev_pos, prev_x)| prev_pos == pos || (prev_x - x_pos).abs() < 1e-6) {
            let reason = if matches!(grouped_arrangement, GroupedArrangement::Stacked | GroupedArrangement::Superimposed) {
                "stacked_cld_guarded"
            } else {
                "overlapping_cld_x_guarded"
            };
            warnings.push(ChartWarning::new(reason, format!("skipped compact letter display for overlapping groups at X={prev_x:.2}")));
            return None;
        }
        matched_slots.push((pos, x_pos));
        labels.push(ResolvedCldSlot { group_key: slot_metas[pos].0, label: assign.label.clone(), letters: assign.letters.clone() });
    }

    (!labels.is_empty()).then_some(labels)
}

pub(crate) fn resolve_cld_overlay(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    slot_metas: &[(usize, &str, &str, Option<f64>, f64)],
    warnings: &mut Vec<ChartWarning>,
) -> Option<ResolvedCldLabels> {
    if !matches!(
        graph.spec.pairwise_display_mode,
        PairwiseDisplayMode::CompactLetterDisplayOnly | PairwiseDisplayMode::BracketsAndCompactLetterDisplay
    ) {
        return None;
    }
    let cld_spec = graph.spec.compact_letter_display.as_ref()?;
    let Some(res) = doc.result_sheet(&cld_spec.result_sheet_id) else {
        warnings.push(ChartWarning::new("missing_cld_result", format!("CLD skipped: result sheet '{}' not found", cld_spec.result_sheet_id)));
        return None;
    };
    if let ResultStatus::Failed { message, .. } = &res.status {
        let detail = if message.trim().is_empty() { "result sheet failed" } else { message.trim() };
        warnings.push(ChartWarning::new("failed_cld_result", format!("CLD skipped: {detail}")));
        return None;
    }

    let options = CldOptions { ordering: cld_spec.ordering, uppercase: cld_spec.uppercase, alpha_override: cld_spec.alpha_override };
    let means_override = collect_cld_means_override(res, slot_metas);
    let cld_res = match compute_cld_from_result_sheet(res, means_override.as_deref(), options) {
        Ok(r) => r,
        Err(err) => {
            warnings.push(ChartWarning::new("incompatible_cld_result", format!("CLD skipped: {err}")));
            return None;
        }
    };
    let labels = match_cld_assignments_to_slots(&cld_res.sorted_assignments, slot_metas, graph.spec.grouped_arrangement, warnings)?;
    let font_size_pt = if cld_spec.font_size_pt.is_finite() && cld_spec.font_size_pt > 0.0 { cld_spec.font_size_pt } else { 9.5 };
    let offset_pt = if cld_spec.offset_pt.is_finite() && cld_spec.offset_pt >= 0.0 { cld_spec.offset_pt } else { 6.0 };
    Some(ResolvedCldLabels { font_size_pt, offset_pt, color: cld_spec.color, labels })
}

pub(crate) fn render_cld_labels(scene: &mut ChartScene, graph: &GraphSheet, cld: &ResolvedCldLabels, bounds: &mut [RenderedSlotBound]) {
    let title_floor_y = if graph.spec.title.trim().is_empty() { 4.0 } else { 24.0 };
    for entry in &cld.labels {
        let Some(pos) =
            find_matched_index(bounds, entry.group_key, Some(entry.label.as_str()), |b| (b.group_key, b.label.as_str(), b.alt_label.as_str()))
        else {
            continue;
        };
        let bound = &mut bounds[pos];
        let rich = RichText::plain(entry.letters.clone());
        let metrics = measure_text_pt(&rich, cld.font_size_pt);
        let label_h = metrics.height_pt().max(cld.font_size_pt);
        let min_baseline_y = title_floor_y + label_h + 2.0;
        let max_baseline_y = (scene.plot_rect_pt.bottom() - 6.0).max(min_baseline_y);
        let desired_y = bound.top_y_pt - cld.offset_pt - 0.15 * cld.font_size_pt;
        let cld_y = crate::clamp_f64_safe(desired_y, min_baseline_y, max_baseline_y);

        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich,
            anchor_pt: PointPt::new(bound.x_pt, cld_y),
            anchor: TextAnchor::Middle,
            font_size_pt: cld.font_size_pt,
            rotation_deg: 0.0,
            color: cld.color,
        }));
        bound.top_y_pt = bound.top_y_pt.min(cld_y - label_h - 1.5);
    }
}

fn render_pairwise_brackets(scene: &mut ChartScene, graph: &GraphSheet, doc: &ProjectDocument, bounds: &[RenderedSlotBound]) {
    if graph.spec.comparisons.is_empty() {
        return;
    }
    let mut resolved = resolve_brackets(graph, doc, bounds, &mut scene.warnings);
    if resolved.is_empty() {
        return;
    }

    // Sort shortest spans first so wider comparison brackets auto-stack cleanly above inner ones.
    resolved.sort_by(|a, b| {
        let span_a = a.slot_a_idx.abs_diff(a.slot_b_idx);
        let span_b = b.slot_a_idx.abs_diff(b.slot_b_idx);
        span_a.cmp(&span_b).then_with(|| a.x_lo.total_cmp(&b.x_lo))
    });

    let mut placed: Vec<(f64, f64, f64)> = Vec::with_capacity(resolved.len());
    let title_floor_y = if graph.spec.title.trim().is_empty() { 4.0 } else { 24.0 };

    for br in resolved {
        let data_top_y = bounds
            .iter()
            .filter(|b| b.x_pt >= br.x_lo - 1.0 && b.x_pt <= br.x_hi + 1.0)
            .map(|b| b.top_y_pt)
            .fold(f64::INFINITY, f64::min)
            .min(scene.plot_rect_pt.bottom());
        let metrics = measure_text_pt(&br.label, br.font_size_pt);
        let label_h = metrics.height_pt().max(br.font_size_pt);
        let min_step = br.step_pt.max(label_h + 7.0);
        let min_bar_y = title_floor_y + label_h + 4.0;
        let max_bar_y = (scene.plot_rect_pt.bottom() - 8.0).max(min_bar_y);

        let mut bar_y = (data_top_y - 8.0).min(scene.plot_rect_pt.bottom() - 12.0);
        for &(px0, px1, p_top) in &placed {
            if px1 >= br.x_lo - 2.0 && px0 <= br.x_hi + 2.0 {
                bar_y = bar_y.min(p_top - min_step);
            }
        }
        bar_y = crate::clamp_f64_safe(bar_y, min_bar_y, max_bar_y);

        emit_bracket_geometry(&mut scene.primitives, &br, bar_y);
        let label_y = bar_y - 3.5;
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: br.label,
            anchor_pt: PointPt::new(0.5 * (br.x_lo + br.x_hi), label_y),
            anchor: TextAnchor::Middle,
            font_size_pt: br.font_size_pt,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
        placed.push((br.x_lo, br.x_hi, label_y - label_h));
    }
}

fn render_horizontal_cld_labels(scene: &mut ChartScene, cld: &ResolvedCldLabels, bounds: &mut [HorizontalSlotBound]) {
    let max_x_floor = (scene.width_pt - 8.0).max(scene.plot_rect_pt.x + 12.0);
    for entry in &cld.labels {
        let Some(pos) =
            find_matched_index(bounds, entry.group_key, Some(entry.label.as_str()), |b| (b.group_key, b.label.as_str(), b.alt_label.as_str()))
        else {
            continue;
        };
        let bound = &mut bounds[pos];
        let rich = RichText::plain(entry.letters.clone());
        let metrics = measure_text_pt(&rich, cld.font_size_pt);
        let label_w = metrics.width_pt.max(cld.font_size_pt * 0.6);
        let desired_x = (bound.right_x_pt + cld.offset_pt).min((max_x_floor - label_w).max(scene.plot_rect_pt.x + 4.0));
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich,
            anchor_pt: PointPt::new(desired_x, bound.y_pt + cld.font_size_pt * 0.35),
            anchor: TextAnchor::Start,
            font_size_pt: cld.font_size_pt,
            rotation_deg: 0.0,
            color: cld.color,
        }));
        bound.right_x_pt = bound.right_x_pt.max(desired_x + label_w + 2.0);
    }
}

fn render_horizontal_pairwise_brackets(scene: &mut ChartScene, graph: &GraphSheet, doc: &ProjectDocument, bounds: &[HorizontalSlotBound]) {
    if graph.spec.comparisons.is_empty() || bounds.is_empty() {
        return;
    }
    let proxy: Vec<RenderedSlotBound> = bounds
        .iter()
        .map(|b| RenderedSlotBound {
            group_key: b.group_key,
            label: b.label.clone(),
            alt_label: b.alt_label.clone(),
            x_pt: b.y_pt,
            top_y_pt: b.right_x_pt,
        })
        .collect();
    let mut resolved = resolve_brackets(graph, doc, &proxy, &mut scene.warnings);
    if resolved.is_empty() {
        return;
    }
    resolved.sort_by(|a, b| {
        let span_a = a.slot_a_idx.abs_diff(a.slot_b_idx);
        let span_b = b.slot_a_idx.abs_diff(b.slot_b_idx);
        span_a.cmp(&span_b).then_with(|| a.x_lo.total_cmp(&b.x_lo))
    });

    let mut placed: Vec<(f64, f64, f64)> = Vec::with_capacity(resolved.len());
    let max_bar_x = (scene.width_pt - 14.0).max(scene.plot_rect_pt.x + 12.0);
    for br in resolved {
        let data_right_x =
            bounds.iter().filter(|b| b.y_pt >= br.x_lo - 1.0 && b.y_pt <= br.x_hi + 1.0).map(|b| b.right_x_pt).fold(scene.plot_rect_pt.x, f64::max);
        let metrics = measure_text_pt(&br.label, br.font_size_pt);
        let label_w = metrics.width_pt.max(br.font_size_pt);
        let min_step = br.step_pt.max(label_w + 8.0);
        let mut bar_x = (data_right_x + 8.0).max(scene.plot_rect_pt.x + 8.0);
        for &(py0, py1, p_right) in &placed {
            if py1 >= br.x_lo - 2.0 && py0 <= br.x_hi + 2.0 {
                bar_x = bar_x.max(p_right + min_step);
            }
        }
        bar_x = crate::clamp_f64_safe(bar_x, scene.plot_rect_pt.x + 8.0, (max_bar_x - label_w).max(scene.plot_rect_pt.x + 8.0));
        let stroke = StrokePaint::solid(Rgba8::BLACK, br.stroke_width_pt);
        let path = Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(bar_x - 4.0, br.x_lo)),
                PathCmd::LineTo(PointPt::new(bar_x, br.x_lo)),
                PathCmd::LineTo(PointPt::new(bar_x, br.x_hi)),
                PathCmd::LineTo(PointPt::new(bar_x - 4.0, br.x_hi)),
            ],
        };
        scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: None });
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: br.label,
            anchor_pt: PointPt::new(bar_x + 4.0, 0.5 * (br.x_lo + br.x_hi) + br.font_size_pt * 0.35),
            anchor: TextAnchor::Start,
            font_size_pt: br.font_size_pt,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
        placed.push((br.x_lo, br.x_hi, bar_x + label_w + 4.0));
    }
}

fn resolve_brackets(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    bounds: &[RenderedSlotBound],
    warnings: &mut Vec<ChartWarning>,
) -> Vec<ResolvedBracket> {
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    let mut out = Vec::with_capacity(graph.spec.comparisons.len());
    for spec in &graph.spec.comparisons {
        let res_opt = doc.result_sheet(&spec.result_sheet_id);
        if let Some(res) = res_opt
            && let ResultStatus::Failed { message, .. } = &res.status
        {
            let detail = if message.trim().is_empty() { "result sheet failed" } else { message.trim() };
            warnings.push(ChartWarning::new("failed_comparison_result", format!("comparison bracket skipped: {detail}")));
            continue;
        }
        let rec_opt = res_opt.and_then(|res| find_comparison_record(&res.pairwise_comparisons, spec.group_a, spec.group_b));

        if spec.hide_if_non_significant {
            let Some(rec) = rec_opt else {
                continue;
            };
            if !rec.significant {
                continue;
            }
        }

        let Some(label) = build_bracket_label(spec, res_opt, rec_opt, doc, info_id, warnings) else {
            continue;
        };
        let slot_a_opt = find_slot_for_group(bounds, spec.group_a, rec_opt.map(|r| r.label_a.as_str()));
        let slot_b_opt = find_slot_for_group(bounds, spec.group_b, rec_opt.map(|r| r.label_b.as_str()));
        let (Some((idx_a, slot_a)), Some((idx_b, slot_b))) = (slot_a_opt, slot_b_opt) else {
            warnings.push(ChartWarning::new(
                "missing_comparison_group",
                format!("comparison bracket {} vs {} skipped: target group not plotted", spec.group_a, spec.group_b),
            ));
            continue;
        };
        if (slot_a.x_pt - slot_b.x_pt).abs() < 1e-6 {
            if matches!(graph.spec.grouped_arrangement, GroupedArrangement::Stacked) {
                warnings.push(ChartWarning::new(
                    "stacked_bracket_guarded",
                    format!("skipped intra-stack comparison bracket ({}, {}) at X={:.1}", spec.group_a, spec.group_b, slot_a.x_pt),
                ));
            }
            continue;
        }
        out.push(ResolvedBracket {
            slot_a_idx: idx_a,
            slot_b_idx: idx_b,
            x_lo: slot_a.x_pt.min(slot_b.x_pt),
            x_hi: slot_a.x_pt.max(slot_b.x_pt),
            style: spec.style,
            label,
            stroke_width_pt: if spec.stroke_width_pt > 0.0 { spec.stroke_width_pt } else { 1.0 },
            font_size_pt: if spec.font_size_pt > 0.0 { spec.font_size_pt } else { 8.5 },
            step_pt: if spec.bracket_step_pt > 0.0 { spec.bracket_step_pt } else { 14.0 },
        });
    }
    out
}

fn build_bracket_label(
    spec: &PairwiseBracketSpec,
    res_opt: Option<&curvebook_doc::ResultSheet>,
    rec_opt: Option<&PairwiseComparisonRecord>,
    doc: &ProjectDocument,
    info_id: Option<&curvebook_doc::SheetId>,
    warnings: &mut Vec<ChartWarning>,
) -> Option<RichText> {
    match &spec.label_mode {
        BracketLabelMode::Custom(raw) => Some(expand_doc_text(raw, doc, info_id)),
        BracketLabelMode::SummaryStars => {
            let (Some(res), Some(rec)) = (res_opt, rec_opt) else {
                warnings.push(ChartWarning::new(
                    "missing_comparison_record",
                    format!("pairwise comparison ({}, {}) not found in '{}'", spec.group_a, spec.group_b, spec.result_sheet_id),
                ));
                return None;
            };
            Some(RichText::plain(res.analysis.p_value_format.stars(rec.raw_p_or_q)))
        }
        BracketLabelMode::NumericPOrQ => {
            let (Some(res), Some(rec)) = (res_opt, rec_opt) else {
                warnings.push(ChartWarning::new(
                    "missing_comparison_record",
                    format!("pairwise comparison ({}, {}) not found in '{}'", spec.group_a, spec.group_b, spec.result_sheet_id),
                ));
                return None;
            };
            let policy = LocaleNumberPolicy::default();
            let mut text = format_pvalue_label(rec.raw_p_or_q, res.analysis.p_value_format, true, false, &policy);
            if rec.is_fdr_q
                && let Some(rest) = text.strip_prefix('P')
            {
                text = format!("q{rest}");
            }
            Some(RichText::plain(text))
        }
    }
}

fn find_slot_for_group<'a>(bounds: &'a [RenderedSlotBound], group_idx: usize, label_hint: Option<&str>) -> Option<(usize, &'a RenderedSlotBound)> {
    let pos = find_matched_index(bounds, group_idx, label_hint, |b| (b.group_key, b.label.as_str(), b.alt_label.as_str()))?;
    bounds.get(pos).map(|b| (pos, b))
}

fn emit_bracket_geometry(primitives: &mut Vec<ScenePrimitive>, br: &ResolvedBracket, bar_y: f64) {
    let stroke = StrokePaint::solid(Rgba8::BLACK, br.stroke_width_pt);
    let path = match br.style {
        BracketStyle::FlatLine => Path2D::line(PointPt::new(br.x_lo, bar_y), PointPt::new(br.x_hi, bar_y)),
        BracketStyle::HalfTick => bracket_legs_path(br.x_lo, br.x_hi, bar_y, 2.5),
        BracketStyle::Square => bracket_legs_path(br.x_lo, br.x_hi, bar_y, 5.0),
        BracketStyle::CurlyBrace => curly_brace_path(br.x_lo, br.x_hi, bar_y, 4.5),
    };
    primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: None });
}

fn bracket_legs_path(x0: f64, x1: f64, y: f64, tick_len: f64) -> Path2D {
    Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(x0, y + tick_len)),
            PathCmd::LineTo(PointPt::new(x0, y)),
            PathCmd::LineTo(PointPt::new(x1, y)),
            PathCmd::LineTo(PointPt::new(x1, y + tick_len)),
        ],
    }
}

fn curly_brace_path(x0: f64, x1: f64, y: f64, amp: f64) -> Path2D {
    let mid = 0.5 * (x0 + x1);
    let bot = y + amp;
    Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(x0, bot)),
            PathCmd::CubicTo { c1: PointPt::new(x0, y), c2: PointPt::new(mid, bot), to: PointPt::new(mid, y - 1.5) },
            PathCmd::CubicTo { c1: PointPt::new(mid, bot), c2: PointPt::new(x1, y), to: PointPt::new(x1, bot) },
        ],
    }
}

fn build_legend_items(graph: &GraphSheet, sheet: &curvebook_doc::DataSheet) -> Vec<LegendItem> {
    if !matches!(sheet.archetype, TableArchetype::Grouped(_)) && graph.spec.series.len() <= 1 {
        return Vec::new();
    }
    if let TableArchetype::Grouped(g) = &sheet.archetype {
        return g
            .data_sets
            .iter()
            .enumerate()
            .map(|(c, ds)| {
                let series_spec = graph.spec.series.iter().find(|s| s.data_set_index == c);
                let label = series_spec.and_then(|s| s.legend_label.clone()).unwrap_or_else(|| ds.title.clone());
                let (gdo, _, _, _) = resolve_series_style(graph, c);
                legend_item_from_gdo(label, &gdo)
            })
            .collect();
    }
    graph
        .spec
        .series
        .iter()
        .enumerate()
        .map(|(idx, s)| {
            let label = s.legend_label.clone().unwrap_or_else(|| format!("Group {}", idx + 1));
            legend_item_from_gdo(label, &s.gdo)
        })
        .collect()
}

fn legend_item_from_gdo(label: String, gdo: &GraphicalDataObject) -> LegendItem {
    match gdo {
        GraphicalDataObject::BarWithReplicates { fill_color, border_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, border_color, .. } => LegendItem {
            label,
            marker: None,
            line_color: None,
            line_dash: LineDashStyle::Solid,
            swatch_fill: Some(*fill_color),
            swatch_border: Some(*border_color),
        },
        GraphicalDataObject::BoxAndWhisker { fill_color, stroke_color, .. } | GraphicalDataObject::Violin { fill_color, stroke_color, .. } => {
            LegendItem {
                label,
                marker: None,
                line_color: None,
                line_dash: LineDashStyle::Solid,
                swatch_fill: Some(*fill_color),
                swatch_border: Some(*stroke_color),
            }
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            LegendItem { label, marker: Some(*marker), line_color: None, line_dash: LineDashStyle::Solid, swatch_fill: None, swatch_border: None }
        }
        GraphicalDataObject::BeforeAfterLines { marker, line_color, .. } => LegendItem {
            label,
            marker: *marker,
            line_color: Some(*line_color),
            line_dash: LineDashStyle::Solid,
            swatch_fill: None,
            swatch_border: None,
        },
        _ => LegendItem { label, marker: None, line_color: None, line_dash: LineDashStyle::Solid, swatch_fill: None, swatch_border: None },
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        BracketLabelMode, BracketStyle, Cell, ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, GraphicalDataObject, GroupedArrangement,
        GroupedTable, PairwiseBracketSpec, PointStyleOverride, ProjectDocument, ReplicateOverlayStyle, SheetId, SubcolumnMode, SummaryMeasure,
        TableArchetype,
    };
    use curvebook_palette::{MarkerShape, Rgba8};

    use crate::{ScenePrimitive, compile_graph};

    #[test]
    fn column_point_overrides_restyle_and_hide_individual_points() {
        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).expect("column table");
        table.data_sets[0].cells = vec![vec![Cell::number(4.0)], vec![Cell::number(7.0)], vec![Cell::number(11.0)]];
        let ds = DataSheet::new(SheetId::new("col_1"), "Samples", TableArchetype::Column(table)).expect("data sheet");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_1"), &ds).expect("graph sheet");
        gs.spec.series[0].gdo = GraphicalDataObject::AlignedDotPlot {
            summary: SummaryMeasure::Mean,
            marker: curvebook_palette::MarkerStyle::default(),
            show_crossbar: false,
        };
        let custom_color = Rgba8::rgb(204, 121, 167);
        gs.spec.point_overrides = vec![
            PointStyleOverride {
                series_index: 0,
                point_index: 1,
                color: Some(custom_color),
                symbol: Some(MarkerShape::Diamond),
                size_pt: Some(9.0),
                ..PointStyleOverride::default()
            },
            PointStyleOverride { series_index: 0, point_index: 2, hidden: true, ..PointStyleOverride::default() },
        ];

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add data");
        doc.add_graph_sheet(gs.clone()).expect("add graph");
        let scene = compile_graph(&gs, &doc).expect("compile graph");

        assert_eq!(scene.plotted_points.len(), 2, "hidden row should not emit plotted point metadata");
        assert!(scene.plotted_points.iter().all(|p| p.source_cell.row != 2));
        let overridden_meta = scene.plotted_points.iter().find(|p| p.source_cell.row == 1).expect("row 1 plotted");
        assert!((overridden_meta.radius_pt - 4.5).abs() < 1e-6);
        assert!(scene.primitives.iter().any(|prim| { matches!(prim, ScenePrimitive::Path { fill: Some(f), .. } if f.color == custom_color) }));
    }

    #[test]
    fn stacked_grouped_guards_intra_stack_brackets_and_clamps_inter_stack_brackets() {
        let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, 2).expect("grouped");
        g.row_titles = vec![String::from("Day 1"), String::from("Day 2")];
        g.data_sets[0].title = String::from("Drug A");
        g.data_sets[0].cells = vec![vec![Cell::number(10.0)], vec![Cell::number(20.0)]];
        g.data_sets[1].title = String::from("Drug B");
        g.data_sets[1].cells = vec![vec![Cell::number(15.0)], vec![Cell::number(25.0)]];
        let ds = DataSheet::new(SheetId::new("grp_1"), "Grouped", TableArchetype::Grouped(g)).expect("sheet");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_1"), &ds).expect("graph");
        gs.spec.grouped_arrangement = GroupedArrangement::Stacked;
        gs.spec.comparisons = vec![
            // Intra-stack (Day 1 Drug A vs Day 1 Drug B at same X): must be guarded cleanly.
            PairwiseBracketSpec {
                result_sheet_id: SheetId::new("res_missing"),
                group_a: 0,
                group_b: 1,
                style: BracketStyle::Square,
                label_mode: BracketLabelMode::Custom(String::from("intra")),
                hide_if_non_significant: false,
                stroke_width_pt: 1.0,
                font_size_pt: 8.5,
                bracket_step_pt: 14.0,
            },
            // Inter-stack (Day 1 vs Day 2 across distinct X positions): placed cleanly above stack summit inside canvas.
            PairwiseBracketSpec {
                result_sheet_id: SheetId::new("res_missing"),
                group_a: 0,
                group_b: 2,
                style: BracketStyle::Square,
                label_mode: BracketLabelMode::Custom(String::from("p=0.01")),
                hide_if_non_significant: false,
                stroke_width_pt: 1.0,
                font_size_pt: 8.5,
                bracket_step_pt: 14.0,
            },
        ];

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add data");
        doc.add_graph_sheet(gs.clone()).expect("add graph");
        let scene = compile_graph(&gs, &doc).expect("compile stacked");

        assert!(scene.warnings.iter().any(|w| w.code == "stacked_bracket_guarded"));
        assert!(!scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "intra")));
        let inter_label = scene
            .primitives
            .iter()
            .find_map(|p| match p {
                ScenePrimitive::Text(t) if t.rich.plain_text() == "p=0.01" => Some(t.anchor_pt.y),
                _ => None,
            })
            .expect("inter-stack bracket label rendered");
        assert!(inter_label >= 24.0 && inter_label < scene.plot_rect_pt.bottom(), "bracket y={inter_label} out of panel frame");
    }

    #[test]
    fn superimposed_grouped_draws_larger_bars_behind_smaller_bars() {
        let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, 1).expect("grouped");
        g.row_titles = vec![String::from("Baseline")];
        // Series 0 is shorter (10.0) than Series 1 (30.0); in Superimposed order Series 1 renders BEFORE Series 0 so Series 0 is in front.
        g.data_sets[0].title = String::from("Short");
        g.data_sets[0].cells = vec![vec![Cell::number(10.0)]];
        g.data_sets[1].title = String::from("Tall");
        g.data_sets[1].cells = vec![vec![Cell::number(30.0)]];
        let ds = DataSheet::new(SheetId::new("grp_sup"), "Superimposed", TableArchetype::Grouped(g)).expect("sheet");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_sup"), &ds).expect("graph");
        gs.spec.grouped_arrangement = GroupedArrangement::Superimposed;
        gs.spec.series[0].gdo = GraphicalDataObject::BarWithReplicates {
            summary: SummaryMeasure::Mean,
            fill_color: Rgba8::rgb(0, 114, 178),
            border_color: Rgba8::rgb(0, 114, 178),
            border_width_pt: 1.0,
            replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
        };
        gs.spec.series[1].gdo = GraphicalDataObject::BarWithReplicates {
            summary: SummaryMeasure::Mean,
            fill_color: Rgba8::rgb(213, 94, 0),
            border_color: Rgba8::rgb(213, 94, 0),
            border_width_pt: 1.0,
            replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
        };

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add data");
        doc.add_graph_sheet(gs.clone()).expect("add graph");
        let scene = compile_graph(&gs, &doc).expect("compile superimposed");

        let bar_colors: Vec<Rgba8> = scene
            .primitives
            .iter()
            .filter_map(|prim| match prim {
                ScenePrimitive::Path { fill: Some(f), clip: Some(_), .. }
                    if f.color == Rgba8::rgb(0, 114, 178) || f.color == Rgba8::rgb(213, 94, 0) =>
                {
                    Some(f.color)
                }
                _ => None,
            })
            .collect();
        assert_eq!(bar_colors.first().copied(), Some(Rgba8::rgb(213, 94, 0)));
        assert_eq!(bar_colors.last().copied(), Some(Rgba8::rgb(0, 114, 178)));
    }

    #[test]
    fn cld_display_modes_avoid_bracket_and_title_collisions_across_bar_and_box() {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, CompactLetterDisplaySpec, OneWayAnovaSpec, PairwiseComparisonRecord, PairwiseDisplayMode, ResultSheet,
            ResultStatus,
        };

        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).expect("column table");
        table.data_sets[0].title = String::from("Ctrl");
        table.data_sets[1].title = String::from("Mid");
        table.data_sets[2].title = String::from("High");
        for (c, base) in [(0, 10.0), (1, 14.0), (2, 22.0)] {
            table.data_sets[c].cells =
                vec![vec![Cell::number(base - 1.0)], vec![Cell::number(base)], vec![Cell::number(base + 1.0)], vec![Cell::number(base)]];
        }
        let ds = DataSheet::new(SheetId::new("col_cld"), "Treatment", TableArchetype::Column(table)).expect("ds");

        let spec = AnalysisSpec::new(SheetId::new("res_anova"), SheetId::new("col_cld"), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))
            .expect("spec");
        let mut res = ResultSheet::new(spec).expect("res");
        res.status = ResultStatus::UpToDate;
        res.pairwise_comparisons = vec![
            PairwiseComparisonRecord {
                group_a: 0,
                group_b: 1,
                label_a: String::from("Ctrl"),
                label_b: String::from("Mid"),
                estimate_diff: Some(-4.0),
                raw_p_or_q: 0.14,
                is_fdr_q: false,
                significant: false,
            },
            PairwiseComparisonRecord {
                group_a: 0,
                group_b: 2,
                label_a: String::from("Ctrl"),
                label_b: String::from("High"),
                estimate_diff: Some(-12.0),
                raw_p_or_q: 0.001,
                is_fdr_q: false,
                significant: true,
            },
            PairwiseComparisonRecord {
                group_a: 1,
                group_b: 2,
                label_a: String::from("Mid"),
                label_b: String::from("High"),
                estimate_diff: Some(-8.0),
                raw_p_or_q: 0.12,
                is_fdr_q: false,
                significant: false,
            },
        ];

        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_cld"), &ds).expect("gs");
        gs.spec.title = String::from("CLD Height Title");
        gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(SheetId::new("res_anova")));
        gs.spec.pairwise_display_mode = PairwiseDisplayMode::CompactLetterDisplayOnly;
        gs.spec.comparisons = vec![PairwiseBracketSpec {
            result_sheet_id: SheetId::new("res_anova"),
            group_a: 0,
            group_b: 2,
            style: BracketStyle::Square,
            label_mode: BracketLabelMode::Custom(String::from("**sig**")),
            hide_if_non_significant: false,
            stroke_width_pt: 1.0,
            font_size_pt: 8.5,
            bracket_step_pt: 14.0,
        }];

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add ds");
        doc.add_result_sheet(res).expect("add res");
        doc.add_graph_sheet(gs.clone()).expect("add gs");

        // 1. CompactLetterDisplayOnly: letters appear (`a` for High, `ab` for Mid, `b` for Ctrl), brackets suppressed.
        let scene_only = compile_graph(&gs, &doc).expect("compile cld only");
        assert!(scene_only.warnings.is_empty(), "unexpected warnings: {:?}", scene_only.warnings);
        let texts: Vec<(f64, f64, String)> = scene_only
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) => Some((t.anchor_pt.x, t.anchor_pt.y, t.rich.plain_text())),
                _ => None,
            })
            .collect();
        assert!(!texts.iter().any(|(_, _, s)| s == "sig"), "CompactLetterDisplayOnly must suppress brackets");
        let a_lbl = texts.iter().find(|(_, _, s)| s == "a").expect("label a");
        let ab_lbl = texts.iter().find(|(_, _, s)| s == "ab").expect("label ab");
        let b_lbl = texts.iter().find(|(_, _, s)| s == "b").expect("label b");
        assert!(b_lbl.0 < ab_lbl.0 && ab_lbl.0 < a_lbl.0, "High should sit rightmost as 'a', Ctrl leftmost as 'b'");
        assert!(a_lbl.1 >= 24.0 && ab_lbl.1 >= 24.0 && b_lbl.1 >= 24.0, "CLD letters must sit below title");

        // 2. BracketsAndCompactLetterDisplay: brackets render ABOVE CLD letters with zero overlap.
        let mut gs_both = gs.clone();
        gs_both.spec.pairwise_display_mode = PairwiseDisplayMode::BracketsAndCompactLetterDisplay;
        let scene_both = compile_graph(&gs_both, &doc).expect("compile cld + brackets");
        let cld_top_a = scene_both
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) if matches!(t.rich.plain_text().as_str(), "a" | "ab" | "b") => Some(t.anchor_pt.y - t.font_size_pt),
                _ => None,
            })
            .fold(f64::INFINITY, f64::min);
        let sig_y = scene_both
            .primitives
            .iter()
            .find_map(|p| match p {
                ScenePrimitive::Text(t) if t.rich.plain_text() == "sig" => Some(t.anchor_pt.y),
                _ => None,
            })
            .expect("bracket text rendered");
        assert!(sig_y + 3.5 < cld_top_a, "bracket baseline+leg ({sig_y}+3.5) must clear top of CLD labels ({cld_top_a})");
        assert!(sig_y >= 24.0, "bracket text must clear title floor");

        // 3. BoxAndWhisker variant preserves CLD letters cleanly.
        let mut gs_box = gs;
        for s in &mut gs_box.spec.series {
            s.gdo = GraphicalDataObject::BoxAndWhisker {
                whiskers: curvebook_doc::WhiskerMethod::MinToMax,
                fill_color: Rgba8::rgb(86, 180, 233),
                stroke_color: Rgba8::BLACK,
                show_outliers: false,
                show_all_points: false,
            };
        }
        let scene_box = compile_graph(&gs_box, &doc).expect("compile box cld");
        assert!(scene_box.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "ab")));
    }

    #[test]
    fn cld_handles_missing_failed_incompatible_and_stacked_gracefully() {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, CompactLetterDisplaySpec, ErrorCode, OneWayAnovaSpec, PairwiseComparisonRecord, PairwiseDisplayMode,
            ResultSheet, ResultStatus,
        };

        let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, 1).expect("grouped");
        g.row_titles = vec![String::from("Day 1")];
        g.data_sets[0].title = String::from("Drug A");
        g.data_sets[0].cells = vec![vec![Cell::number(10.0)]];
        g.data_sets[1].title = String::from("Drug B");
        g.data_sets[1].cells = vec![vec![Cell::number(20.0)]];
        let ds = DataSheet::new(SheetId::new("grp_cld"), "Grouped", TableArchetype::Grouped(g)).expect("ds");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_grp"), &ds).expect("gs");
        gs.spec.grouped_arrangement = GroupedArrangement::Stacked;
        gs.spec.pairwise_display_mode = PairwiseDisplayMode::CompactLetterDisplayOnly;
        gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(SheetId::new("res_missing")));

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add ds");
        doc.add_graph_sheet(gs.clone()).expect("add gs");

        let s_missing = compile_graph(&gs, &doc).expect("missing result");
        assert!(s_missing.warnings.iter().any(|w| w.code == "missing_cld_result"));

        let spec_fail = AnalysisSpec::new(SheetId::new("res_fail"), SheetId::new("grp_cld"), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))
            .expect("spec_fail");
        let mut res_fail = ResultSheet::new(spec_fail).expect("res_fail");
        res_fail.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("boom") };
        doc.add_result_sheet(res_fail).expect("add failed");
        gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(SheetId::new("res_fail")));
        let s_failed = compile_graph(&gs, &doc).expect("failed result");
        assert!(s_failed.warnings.iter().any(|w| w.code == "failed_cld_result"));

        // 3 groups missing pair (0,2) -> incomplete matrix (e.g. Dunnett) -> incompatible_cld_result warning.
        let spec_inc = AnalysisSpec::new(SheetId::new("res_inc"), SheetId::new("grp_cld"), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))
            .expect("spec_inc");
        let mut res_inc = ResultSheet::new(spec_inc).expect("res_inc");
        res_inc.status = ResultStatus::UpToDate;
        res_inc.pairwise_comparisons = vec![
            PairwiseComparisonRecord {
                group_a: 0,
                group_b: 1,
                label_a: String::from("A"),
                label_b: String::from("B"),
                estimate_diff: Some(-5.0),
                raw_p_or_q: 0.01,
                is_fdr_q: false,
                significant: true,
            },
            PairwiseComparisonRecord {
                group_a: 0,
                group_b: 2,
                label_a: String::from("A"),
                label_b: String::from("C"),
                estimate_diff: Some(-8.0),
                raw_p_or_q: 0.01,
                is_fdr_q: false,
                significant: true,
            },
        ];
        doc.add_result_sheet(res_inc).expect("add incomplete");
        gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(SheetId::new("res_inc")));
        let s_inc = compile_graph(&gs, &doc).expect("incomplete result");
        assert!(s_inc.warnings.iter().any(|w| w.code == "incompatible_cld_result"));

        // Stacked coinciding X -> stacked_cld_guarded warning.
        let spec_stk = AnalysisSpec::new(SheetId::new("res_stk"), SheetId::new("grp_cld"), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))
            .expect("spec_stk");
        let mut res_stk = ResultSheet::new(spec_stk).expect("res_stk");
        res_stk.status = ResultStatus::UpToDate;
        res_stk.pairwise_comparisons = vec![PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: String::from("Drug A:Day 1"),
            label_b: String::from("Drug B:Day 1"),
            estimate_diff: Some(-10.0),
            raw_p_or_q: 0.01,
            is_fdr_q: false,
            significant: true,
        }];
        doc.add_result_sheet(res_stk).expect("add stacked");
        gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(SheetId::new("res_stk")));
        let s_stk = compile_graph(&gs, &doc).expect("stacked cld");
        assert!(s_stk.warnings.iter().any(|w| w.code == "stacked_cld_guarded"));
    }
}
