//! Column, grouped bar, replicate scatter, box-and-whisker, violin, floating bar,
//! before/after paired lines, and auto-stacked collision-free significance brackets.

use std::collections::BTreeMap;

use lumencraft_doc::{
    AnalysisKind, BracketLabelMode, BracketStyle, CategoricalTickSource, ColumnTable, ErrorBarDirection, ErrorBarSpec, GraphSheet, GraphSpec,
    GraphicalDataObject, GroupedArrangement, GroupedTable, NestedTable, PairwiseBracketSpec, PairwiseComparisonRecord, PairwiseDisplayMode,
    ProjectDocument, ReplicateOverlayStyle, ResultStatus, SheetId, SummaryMeasure, TableArchetype, TickLabelFormat, WhiskerMethod,
};
use lumencraft_math::sum::SummaryMoments;
use lumencraft_palette::{LineDashStyle, MarkerShape, MarkerStyle, Rgba8};
use lumencraft_stats::cld::{CldOptions, compute_cld_from_result_sheet};
use lumencraft_stats::common::{quantile_sorted, quartiles_hinge, sorted_finite};
use lumencraft_text::{LocaleNumberPolicy, RichText, format_pvalue_label, measure_text_pt};

use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PathCmd, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive,
    SourceCellId, StrokePaint, TextAnchor,
};
use crate::xy::{
    AxisErrorTarget, LegendItem, SegmentedAxis, compute_central_tendency, compute_raw_error_bounds, configure_plot_rect, emit_directional_error_bar,
    emit_marker, expand_doc_text, format_point_value, is_log_like, polygon_path, primary_info_sheet_id, render_cartesian_chrome,
    render_legend_and_annotations, resolve_data_sheet, stroke_with_dash, summarize_subcells,
};

const KDE_SAMPLES: usize = 65;

#[derive(Debug, Clone)]
struct RawObs {
    value: f64,
    source: SourceCellId,
}

#[derive(Debug, Clone)]
enum ExtractedKind {
    Raw(Vec<RawObs>),
    PreSummarized { center: f64, lower: f64, upper: f64, source: SourceCellId },
}

#[derive(Debug, Clone)]
struct CategorySlotItem {
    group_key: usize,
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

    let has_y2 = graph.spec.y2_axis.is_some()
        || graph.spec.series.iter().any(|s| s.bind_to_right_y2)
        || matches!(graph.spec.frame_preset, lumencraft_doc::FramePreset::DualYSecondaryRight);
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
    configure_plot_rect(graph, scene, has_y2, bracket_margin_pt + cld_margin_pt);

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
        let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| lumencraft_doc::AxisSpec::linear("Y2"));
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
        let top_y = render_single_item(scene, &graph.spec, item, &x_axis, target_y, &mut stacks);
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
    let legend_items = build_legend_items(graph, primary);
    render_legend_and_annotations(scene, graph, doc, &legend_items);
    Ok(())
}

fn render_empty_axes(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene, has_y2: bool) {
    let r = scene.plot_rect_pt;
    let x_axis = SegmentedAxis::build(&graph.spec.x_axis, r.x, r.right(), &[0.0, 1.0], false, 0.0);
    let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &[0.0, 1.0], false, 0.0);
    let y2_axis = if has_y2 {
        let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| lumencraft_doc::AxisSpec::linear("Y2"));
        Some(SegmentedAxis::build(&y2_spec, r.bottom(), r.y, &[0.0, 1.0], false, 0.0))
    } else {
        None
    };
    render_cartesian_chrome(scene, graph, doc, &x_axis, &y_axis, y2_axis.as_ref(), None);
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
    sheet: &lumencraft_doc::DataSheet,
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

fn extract_column_items(
    graph: &GraphSheet,
    sheet_id: &SheetId,
    table: &ColumnTable,
    tick_positions: &mut Vec<(f64, String)>,
    warnings: &mut Vec<ChartWarning>,
) -> Vec<CategorySlotItem> {
    let mut items = Vec::with_capacity(table.data_sets.len());
    for (c, ds) in table.data_sets.iter().enumerate() {
        let (gdo, y_error_bars, default_color, bind_to_right_y2) = resolve_series_style(graph, c);
        let x_pos = (c as f64) + 0.5;
        let label = match graph.spec.x_axis.tick_format {
            TickLabelFormat::Categorical(CategoricalTickSource::RowNumbers) => format!("{}", c + 1),
            _ => ds.title.clone(),
        };
        tick_positions.push((x_pos, label.clone()));

        let payload = if table.subcolumn_mode.is_pre_summarized() {
            extract_column_presummarized(sheet_id, c, table.subcolumn_mode, &ds.cells, y_error_bars.as_ref(), warnings)
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
    mode: lumencraft_doc::SubcolumnMode,
    rows: &[Vec<lumencraft_doc::Cell>],
    err_spec: Option<&ErrorBarSpec>,
    warnings: &mut Vec<ChartWarning>,
) -> ExtractedKind {
    for (row, subcells) in rows.iter().enumerate() {
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
    for (g_idx, group) in table.groups.iter().enumerate() {
        let (gdo, y_error_bars, default_color, bind_to_right_y2) = resolve_series_style(graph, g_idx);
        let x_pos = (g_idx as f64) + 0.5;
        tick_positions.push((x_pos, group.title.clone()));

        let mut unit_means = Vec::new();
        let mut all_tech = Vec::new();
        for b in 0..group.biological_unit_count() {
            let mut unit_vals = Vec::new();
            let mut first_row: Option<usize> = None;
            for (r, row_cells) in group.cells.iter().enumerate() {
                if let Some(v) = row_cells.get(b).and_then(lumencraft_doc::Cell::active_number) {
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
            match_label: group.title.clone(),
            alt_label: group.title.clone(),
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
    let mut items = Vec::with_capacity(nrows * ncols);

    match arrangement {
        GroupedArrangement::Interleaved => {
            for (r, &cx) in numeric_rows.iter().enumerate().take(nrows) {
                let step = if r + 1 < nrows { (numeric_rows[r + 1] - cx).abs().max(0.5) } else { 1.0 };
                let cluster_w = step * 0.76;
                let slot_w = cluster_w / (ncols as f64);
                let row_label = row_display_label(&table.row_titles, r);
                if matches!(graph.spec.x_axis.tick_format, TickLabelFormat::Categorical(_)) || table.row_numeric_spacing.is_none() {
                    tick_positions.push((cx, row_label.clone()));
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
                    tick_positions.push((slot_x, format!("{col_title}:{row_label}")));
                    let slot = GroupedSlotPlacement { r, c, x_pos: slot_x, slot_w: 0.72, stack_group: None };
                    push_grouped_cell(&mut items, graph, sheet_id, table, slot, warnings);
                }
            }
        }
        GroupedArrangement::Stacked | GroupedArrangement::Superimposed => {
            let stack_for_row = matches!(arrangement, GroupedArrangement::Stacked);
            for (r, &cx) in numeric_rows.iter().enumerate().take(nrows) {
                let row_label = row_display_label(&table.row_titles, r);
                if matches!(graph.spec.x_axis.tick_format, TickLabelFormat::Categorical(_)) || table.row_numeric_spacing.is_none() {
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
    if !matches!(graph.spec.x_axis.tick_format, TickLabelFormat::Categorical(_))
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
        if let Some((center, lower, upper)) = summarize_subcells(table.subcolumn_mode, cells, SummaryMeasure::Mean, y_error_bars.as_ref(), warnings) {
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
                    if it.y_error_bars.is_some() {
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
    sheet: &lumencraft_doc::DataSheet,
    items: &[CategorySlotItem],
    axes: ConnectorAxes<'_>,
    bounds: &mut Vec<RenderedSlotBound>,
) {
    let mut ba_cols: Vec<(usize, &CategorySlotItem)> =
        items.iter().enumerate().filter(|(_, it)| matches!(it.gdo, GraphicalDataObject::BeforeAfterLines { .. })).collect();
    if ba_cols.len() < 2 {
        return;
    }
    let TableArchetype::Column(col_table) = &sheet.archetype else {
        return;
    };
    ba_cols.sort_by(|(_, a), (_, b)| a.x_pos.total_cmp(&b.x_pos).then_with(|| a.group_key.cmp(&b.group_key)));
    let (line_color, width_pt, default_marker) = match &ba_cols[0].1.gdo {
        GraphicalDataObject::BeforeAfterLines { line_color, stroke_width_pt, marker } => (*line_color, *stroke_width_pt, *marker),
        _ => return,
    };

    let clip = scene.plot_rect_pt;
    let stroke = StrokePaint::solid(line_color, width_pt.max(0.75));
    let mut top_per_col: Vec<f64> = vec![clip.bottom(); items.len()];

    for row in 0..col_table.row_count() {
        let mut pts: Vec<(usize, &CategorySlotItem, PointPt, f64)> = Vec::new();
        for &(slot_idx, item) in &ba_cols {
            let data_col = item.group_key;
            let Some(val) = col_table.data_sets.get(data_col).and_then(|d| d.cell(row, 0)).and_then(lumencraft_doc::Cell::active_number) else {
                continue;
            };
            let target_y = if item.bind_to_right_y2 { axes.y2.unwrap_or(axes.y) } else { axes.y };
            if let (Some((_, px)), Some((_, py))) = (axes.x.map_value(item.x_pos), target_y.map_value(val)) {
                pts.push((data_col, item, PointPt::new(px, py), val));
                if let Some(slot_top) = top_per_col.get_mut(slot_idx) {
                    *slot_top = slot_top.min(py);
                }
            }
        }
        if pts.len() >= 2 {
            let mut cmds = Vec::with_capacity(pts.len());
            cmds.push(PathCmd::MoveTo(pts[0].2));
            for &(_, _, p, _) in &pts[1..] {
                cmds.push(PathCmd::LineTo(p));
            }
            scene.primitives.push(ScenePrimitive::Path { path: Path2D { commands: cmds }, fill: None, stroke: Some(stroke), clip: Some(clip) });
        }
        for &(data_col, item, p, val) in &pts {
            let item_marker = match &item.gdo {
                GraphicalDataObject::BeforeAfterLines { marker, .. } => *marker,
                _ => default_marker,
            };
            let Some(m) = item_marker else {
                continue;
            };
            let Some(resolved) = crate::resolve_overridden_marker_for_cell(graph_spec, data_col, row, Some(0), m) else {
                continue;
            };
            emit_marker(&mut scene.primitives, p, &resolved, Some(clip));
            scene.plotted_points.push(PlottedPointMeta {
                center_pt: p,
                radius_pt: (resolved.size_pt * 0.5).max(3.0),
                source_cell: SourceCellId { sheet_id: sheet.id.clone(), row, col: data_col, subcol: 0 },
                series_label: item.match_label.clone(),
                value_summary: format_point_value(val),
            });
        }
    }

    for &(slot_idx, item) in &ba_cols {
        if let Some((_, x_pt)) = axes.x.map_value(item.x_pos) {
            bounds.push(RenderedSlotBound {
                group_key: item.group_key,
                label: item.match_label.clone(),
                alt_label: item.alt_label.clone(),
                x_pt,
                top_y_pt: top_per_col.get(slot_idx).copied().unwrap_or(clip.bottom()),
            });
        }
    }
}

fn render_single_item(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    item: &CategorySlotItem,
    x_axis: &SegmentedAxis,
    y_axis: &SegmentedAxis,
    stacks: &mut StackAccum,
) -> Option<f64> {
    let (_, cx) = x_axis.map_value(item.x_pos)?;
    let (_, x_left, _) = x_axis.map_clamped_to_nearest(item.x_pos - 0.5 * item.slot_width_data);
    let (_, x_right, _) = x_axis.map_clamped_to_nearest(item.x_pos + 0.5 * item.slot_width_data);
    let slot = SlotGeom { cx, width_pt: (x_right - x_left).abs().max(4.0) };

    match &item.gdo {
        GraphicalDataObject::BeforeAfterLines { .. } => None,
        GraphicalDataObject::BarWithReplicates { .. } => render_bar_item(scene, graph_spec, item, slot, y_axis, stacks),
        GraphicalDataObject::AlignedDotPlot { .. } | GraphicalDataObject::ScatterAndLine { .. } => {
            render_aligned_dot_item(scene, graph_spec, item, slot, y_axis)
        }
        GraphicalDataObject::BoxAndWhisker { .. } => render_box_whisker_item(scene, graph_spec, item, slot, y_axis),
        GraphicalDataObject::Violin { .. } => render_violin_item(scene, graph_spec, item, slot, y_axis),
        GraphicalDataObject::FloatingBar { .. } => render_floating_bar_item(scene, item, slot, y_axis),
        GraphicalDataObject::KaplanMeierStep { .. } | GraphicalDataObject::PartsOfWhole { .. } => None,
    }
}

fn render_bar_item(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    item: &CategorySlotItem,
    slot: SlotGeom,
    y_axis: &SegmentedAxis,
    stacks: &mut StackAccum,
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

    let (base_data, top_data) = if let Some(sg) = item.stack_group {
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
        let b = if let Some(seg) = y_axis.segments.first() {
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

    let (_, base_py, _) = y_axis.map_clamped_to_nearest(base_data);
    let (_, top_py, top_clipped) = y_axis.map_clamped_to_nearest(top_data);
    if top_clipped && y_axis.segments.iter().any(|s| is_log_like(s.scale)) && top_data <= 0.0 {
        scene.warnings.push(ChartWarning::new("non_positive_on_log_axis", format!("bar value {top_data} omitted on log axis")));
        return None;
    }

    let y0 = base_py.min(top_py);
    let y1 = base_py.max(top_py);
    let bar_rect = RectPt::new(cx - 0.5 * slot_w_pt, y0, slot_w_pt, (y1 - y0).max(0.5));
    let border = (border_width_pt > 0.0).then(|| StrokePaint::solid(border_color, border_width_pt));
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(bar_rect),
        fill: Some(FillPaint::solid(fill_color)),
        stroke: border,
        clip: Some(clip),
    });

    let mut highest_pt = y0;
    if let Some(eb) = &item.y_error_bars {
        let mut eb_use = *eb;
        let shifted_lo = top_data - (center - err_lo);
        let shifted_hi = top_data + (err_hi - center);
        if item.stack_group.is_some() {
            eb_use.direction = if center >= 0.0 { ErrorBarDirection::UpperOnly } else { ErrorBarDirection::LowerOnly };
        }
        emit_directional_error_bar(
            scene,
            cx,
            top_data,
            shifted_lo,
            shifted_hi,
            AxisErrorTarget { axis: y_axis, is_vertical: true, spec: &eb_use, default_color: border_color, clip },
        );
        if !matches!(eb_use.direction, ErrorBarDirection::LowerOnly) {
            let (_, ehi_py, _) = y_axis.map_clamped_to_nearest(shifted_hi);
            highest_pt = highest_pt.min(ehi_py);
        }
    }

    if let Some(overlay) = replicate_overlay
        && !raw_obs.is_empty()
        && item.stack_group.is_none()
    {
        let dot_style = MarkerStyle {
            shape: MarkerShape::Circle,
            fill_mode: lumencraft_palette::MarkerFillMode::Filled,
            size_pt: 3.4,
            fill_color: border_color,
            stroke_color: Rgba8::WHITE,
            stroke_width_pt: 0.4,
        };
        let ctx = ReplicateDotCtx {
            spec: graph_spec,
            cx,
            max_half_w: slot_w_pt * 0.40,
            y_axis,
            style: overlay,
            marker: &dot_style,
            series_label: &item.match_label,
        };
        let top_dot = emit_replicate_dots(scene, &raw_obs, &ctx);
        highest_pt = highest_pt.min(top_dot);
    }

    Some(highest_pt)
}

fn render_aligned_dot_item(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    item: &CategorySlotItem,
    slot: SlotGeom,
    y_axis: &SegmentedAxis,
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
        cx,
        max_half_w: slot_w_pt * 0.42,
        y_axis,
        style: ReplicateOverlayStyle::DeterministicBeeswarm,
        marker: &marker,
        series_label: &item.match_label,
    };
    let mut highest_pt = emit_replicate_dots(scene, &raw_obs, &dot_ctx);

    let (center, lo, hi) = match &item.payload {
        ExtractedKind::PreSummarized { center, lower, upper, .. } => (*center, *lower, *upper),
        ExtractedKind::Raw(obs) => {
            let vals = raw_values(obs);
            let c = compute_central_tendency(&vals, summary, &mut scene.warnings)?;
            let (l, h) = if let Some(eb) = &item.y_error_bars { compute_raw_error_bounds(&vals, c, eb.metric, 0.05) } else { (c, c) };
            (c, l, h)
        }
    };
    if let Some((_, c_py)) = y_axis.map_value(center) {
        highest_pt = highest_pt.min(c_py);
        if show_crossbar {
            let half_bar = (slot_w_pt * 0.35).max(5.0);
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(cx - half_bar, c_py), PointPt::new(cx + half_bar, c_py)),
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
            AxisErrorTarget { axis: y_axis, is_vertical: true, spec: eb, default_color: Rgba8::BLACK, clip },
        );
        let (_, ehi_py, _) = y_axis.map_clamped_to_nearest(hi);
        highest_pt = highest_pt.min(ehi_py);
    }
    Some(highest_pt)
}

fn render_box_whisker_item(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    item: &CategorySlotItem,
    slot: SlotGeom,
    y_axis: &SegmentedAxis,
) -> Option<f64> {
    let GraphicalDataObject::BoxAndWhisker { whiskers, fill_color, stroke_color, show_outliers, show_all_points } = item.gdo else {
        return None;
    };
    let cx = slot.cx;
    let slot_w_pt = slot.width_pt;
    let clip = scene.plot_rect_pt;
    let sorted_obs = match &item.payload {
        ExtractedKind::Raw(obs) => sorted_finite_obs(obs),
        ExtractedKind::PreSummarized { center, lower, upper, source } => vec![
            RawObs { value: *lower, source: source.clone() },
            RawObs { value: *center, source: source.clone() },
            RawObs { value: *upper, source: source.clone() },
        ],
    };
    if sorted_obs.is_empty() {
        return None;
    }
    let sorted = raw_values(&sorted_obs);
    let (q1, median, q3) = quartiles_hinge(&sorted)?;
    let (w_lo, w_hi, outlier_indices) = compute_box_whiskers(&sorted, q1, q3, whiskers);

    let (_, py_q1, _) = y_axis.map_clamped_to_nearest(q1);
    let (_, py_med, _) = y_axis.map_clamped_to_nearest(median);
    let (_, py_q3, _) = y_axis.map_clamped_to_nearest(q3);
    let (_, py_wlo, _) = y_axis.map_clamped_to_nearest(w_lo);
    let (_, py_whi, _) = y_axis.map_clamped_to_nearest(w_hi);

    let box_w = slot_w_pt * 0.72;
    let cap_half = box_w * 0.28;
    let stroke = StrokePaint::solid(stroke_color, 1.1);

    // Lower & upper whisker stems and caps
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(cx, py_q1), PointPt::new(cx, py_wlo)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(clip),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(cx - cap_half, py_wlo), PointPt::new(cx + cap_half, py_wlo)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(clip),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(cx, py_q3), PointPt::new(cx, py_whi)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(clip),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(cx - cap_half, py_whi), PointPt::new(cx + cap_half, py_whi)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(clip),
    });

    // Interquartile box & median line
    let top_box = py_q3.min(py_q1);
    let bot_box = py_q3.max(py_q1);
    let box_rect = RectPt::new(cx - 0.5 * box_w, top_box, box_w, (bot_box - top_box).max(0.8));
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(box_rect),
        fill: Some(FillPaint::solid(fill_color)),
        stroke: Some(stroke),
        clip: Some(clip),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(box_rect.x, py_med), PointPt::new(box_rect.right(), py_med)),
        fill: None,
        stroke: Some(StrokePaint::solid(stroke_color, 1.75)),
        clip: Some(clip),
    });

    let mut highest_pt = py_whi.min(top_box);
    let dot_style = MarkerStyle {
        shape: MarkerShape::Circle,
        fill_mode: lumencraft_palette::MarkerFillMode::Filled,
        size_pt: 3.4,
        fill_color: stroke_color,
        stroke_color,
        stroke_width_pt: 0.5,
    };
    if show_all_points {
        let dot_ctx = ReplicateDotCtx {
            spec: graph_spec,
            cx,
            max_half_w: box_w * 0.42,
            y_axis,
            style: ReplicateOverlayStyle::DeterministicBeeswarm,
            marker: &dot_style,
            series_label: &item.match_label,
        };
        let top_dot = emit_replicate_dots(scene, &sorted_obs, &dot_ctx);
        highest_pt = highest_pt.min(top_dot);
    } else if show_outliers {
        for idx in outlier_indices {
            let obs = &sorted_obs[idx];
            let Some(resolved) =
                crate::resolve_overridden_marker_for_cell(graph_spec, obs.source.col, obs.source.row, Some(obs.source.subcol), dot_style)
            else {
                continue;
            };
            if let Some((_, py)) = y_axis.map_value(obs.value) {
                let dot_pt = PointPt::new(cx, py);
                emit_marker(&mut scene.primitives, dot_pt, &resolved, Some(clip));
                push_col_point_meta(scene, dot_pt, resolved.size_pt, obs, &item.match_label);
                highest_pt = highest_pt.min(py);
            }
        }
    }
    Some(highest_pt)
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

fn render_violin_item(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    item: &CategorySlotItem,
    slot: SlotGeom,
    y_axis: &SegmentedAxis,
) -> Option<f64> {
    let GraphicalDataObject::Violin { fill_color, stroke_color, clip_to_observed_range, show_median_and_quartiles, half_violin_with_points } =
        item.gdo
    else {
        return None;
    };
    let cx = slot.cx;
    let slot_w_pt = slot.width_pt;
    let clip = scene.plot_rect_pt;
    let sorted_obs = match &item.payload {
        ExtractedKind::Raw(obs) => sorted_finite_obs(obs),
        ExtractedKind::PreSummarized { center, lower, upper, source } => vec![
            RawObs { value: *lower, source: source.clone() },
            RawObs { value: *center, source: source.clone() },
            RawObs { value: *upper, source: source.clone() },
        ],
    };
    if sorted_obs.is_empty() {
        return None;
    }
    let sorted = raw_values(&sorted_obs);

    let (q1, median, q3) = quartiles_hinge(&sorted)?;
    let half_max_w = slot_w_pt * 0.45;

    let profile = build_kde_profile(&sorted, q1, q3, clip_to_observed_range, y_axis);
    if profile.len() < 2 {
        let (_, py, _) = y_axis.map_clamped_to_nearest(median);
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(cx - 0.5 * half_max_w, py), PointPt::new(cx + 0.5 * half_max_w, py)),
            fill: None,
            stroke: Some(StrokePaint::solid(stroke_color, 1.5)),
            clip: Some(clip),
        });
        return Some(py);
    }

    let mut right_pts: Vec<PointPt> = Vec::with_capacity(profile.len());
    let mut left_pts: Vec<PointPt> = Vec::with_capacity(profile.len());
    let mut highest_pt = clip.bottom();
    for &(yv, norm_d) in &profile {
        if let Some((_, py)) = y_axis.map_value(yv) {
            let dx = norm_d * half_max_w;
            right_pts.push(PointPt::new(cx + dx, py));
            left_pts.push(PointPt::new(if half_violin_with_points { cx } else { cx - dx }, py));
            highest_pt = highest_pt.min(py);
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
            fill: Some(FillPaint::solid(fill_color)),
            stroke: Some(StrokePaint::solid(stroke_color, 1.0)),
            clip: Some(clip),
        });
    }

    if show_median_and_quartiles {
        for (val, width, dash) in [(median, 1.75, LineDashStyle::Solid), (q1, 0.9, LineDashStyle::Dashed), (q3, 0.9, LineDashStyle::Dashed)] {
            if let Some((_, py)) = y_axis.map_value(val) {
                let d_at_val = interpolate_profile_width(&profile, val) * half_max_w;
                let x0 = if half_violin_with_points { cx } else { cx - d_at_val.max(2.0) };
                let x1 = cx + d_at_val.max(2.0);
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(x0, py), PointPt::new(x1, py)),
                    fill: None,
                    stroke: Some(stroke_with_dash(stroke_color, width, dash)),
                    clip: Some(clip),
                });
            }
        }
    }

    if half_violin_with_points {
        let dot_style = MarkerStyle {
            shape: MarkerShape::Circle,
            fill_mode: lumencraft_palette::MarkerFillMode::Filled,
            size_pt: 3.4,
            fill_color: stroke_color,
            stroke_color: Rgba8::WHITE,
            stroke_width_pt: 0.4,
        };
        let half_center = cx - 0.52 * half_max_w;
        let dot_ctx = ReplicateDotCtx {
            spec: graph_spec,
            cx: half_center,
            max_half_w: 0.40 * half_max_w,
            y_axis,
            style: ReplicateOverlayStyle::DeterministicBeeswarm,
            marker: &dot_style,
            series_label: &item.match_label,
        };
        let top_dot = emit_replicate_dots(scene, &sorted_obs, &dot_ctx);
        highest_pt = highest_pt.min(top_dot);
    }

    Some(highest_pt)
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
    if let Some(seg0) = y_axis.segments.first() {
        if is_log_like(seg0.scale) {
            lo = lo.max(seg0.domain_min.max(min_v * 0.85));
        }
        hi = hi.min(y_axis.segments.last().map_or(hi, |s| s.domain_max));
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

fn render_floating_bar_item(scene: &mut ChartScene, item: &CategorySlotItem, slot: SlotGeom, y_axis: &SegmentedAxis) -> Option<f64> {
    let GraphicalDataObject::FloatingBar { summary, fill_color, border_color, border_width_pt } = item.gdo else {
        return None;
    };
    let cx = slot.cx;
    let slot_w_pt = slot.width_pt;
    let clip = scene.plot_rect_pt;
    let (lo, hi, mid) = match &item.payload {
        ExtractedKind::PreSummarized { center, lower, upper, .. } => (*lower, *upper, summary.map(|_| *center)),
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

    let (_, py_lo, _) = y_axis.map_clamped_to_nearest(lo);
    let (_, py_hi, _) = y_axis.map_clamped_to_nearest(hi);
    let top_y = py_lo.min(py_hi);
    let bot_y = py_lo.max(py_hi);
    let rect = RectPt::new(cx - 0.5 * slot_w_pt, top_y, slot_w_pt, (bot_y - top_y).max(1.0));
    let border = (border_width_pt > 0.0).then(|| StrokePaint::solid(border_color, border_width_pt));
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(rect),
        fill: Some(FillPaint::solid(fill_color)),
        stroke: border,
        clip: Some(clip),
    });

    if let Some(m) = mid
        && let Some((_, py_m)) = y_axis.map_value(m)
    {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(rect.x, py_m), PointPt::new(rect.right(), py_m)),
            fill: None,
            stroke: Some(StrokePaint::solid(border_color, border_width_pt.max(1.2))),
            clip: Some(clip),
        });
    }
    Some(top_y)
}

struct ReplicateDotCtx<'a> {
    spec: &'a GraphSpec,
    cx: f64,
    max_half_w: f64,
    y_axis: &'a SegmentedAxis,
    style: ReplicateOverlayStyle,
    marker: &'a MarkerStyle,
    series_label: &'a str,
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

fn subsample_sorted_indices(pts_y: &[f64], cap: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..pts_y.len()).collect();
    order.sort_by(|&a, &b| pts_y[a].total_cmp(&pts_y[b]).then_with(|| a.cmp(&b)));
    if order.len() <= cap || cap < 2 {
        return order.into_iter().take(cap).collect();
    }
    let last = (order.len() - 1) as u64;
    let denom = (cap - 1) as u64;
    (0..cap).map(|i| order[((i as u64 * last) / denom) as usize]).collect()
}

fn emit_replicate_dots(scene: &mut ChartScene, obs: &[RawObs], ctx: &ReplicateDotCtx<'_>) -> f64 {
    let clip = scene.plot_rect_pt;
    let mut top_pt = clip.bottom();
    let mut mapped: Vec<(f64, &RawObs, MarkerStyle)> = Vec::with_capacity(obs.len());
    let mut pts_y: Vec<f64> = Vec::with_capacity(obs.len());
    let mut max_dot_r = (ctx.marker.size_pt * 0.5).max(1.2);
    for o in obs {
        let Some(resolved) = crate::resolve_overridden_marker_for_cell(ctx.spec, o.source.col, o.source.row, Some(o.source.subcol), *ctx.marker)
        else {
            continue;
        };
        if let Some((_, py)) = ctx.y_axis.map_value(o.value) {
            max_dot_r = max_dot_r.max(resolved.size_pt * 0.5);
            mapped.push((py, o, resolved));
            pts_y.push(py);
            top_pt = top_pt.min(py);
        }
    }
    if mapped.is_empty() {
        return top_pt;
    }

    if mapped.len() > MAX_REPLICATE_MARKERS {
        let total = mapped.len();
        let keep = subsample_sorted_indices(&pts_y, MAX_REPLICATE_MARKERS);
        mapped = keep.iter().map(|&idx| mapped[idx]).collect();
        pts_y = keep.iter().map(|&idx| pts_y[idx]).collect();
        if !scene.warnings.iter().any(|w| w.code == "replicate_markers_capped") {
            scene.warnings.push(ChartWarning::new(
                "replicate_markers_capped",
                format!("replicate overlay capped to {MAX_REPLICATE_MARKERS} representative markers (from {total} points)"),
            ));
        }
    }

    let offsets = match ctx.style {
        ReplicateOverlayStyle::AlignedColumn => vec![0.0; mapped.len()],
        ReplicateOverlayStyle::DeterministicBeeswarm => deterministic_beeswarm_offsets(&pts_y, ctx.max_half_w, max_dot_r),
    };
    for (&(py, raw, resolved), dx) in mapped.iter().zip(offsets.iter()) {
        let dot_pt = PointPt::new(ctx.cx + dx, py);
        emit_marker(&mut scene.primitives, dot_pt, &resolved, Some(clip));
        push_col_point_meta(scene, dot_pt, resolved.size_pt, raw, ctx.series_label);
    }
    top_pt - max_dot_r
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

fn collect_cld_means_override(res: &lumencraft_doc::ResultSheet, slot_metas: &[(usize, &str, &str, Option<f64>, f64)]) -> Option<Vec<(usize, f64)>> {
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
    assignments: &[lumencraft_stats::cld::CldGroupAssignment],
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
    res_opt: Option<&lumencraft_doc::ResultSheet>,
    rec_opt: Option<&PairwiseComparisonRecord>,
    doc: &ProjectDocument,
    info_id: Option<&lumencraft_doc::SheetId>,
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

fn build_legend_items(graph: &GraphSheet, sheet: &lumencraft_doc::DataSheet) -> Vec<LegendItem> {
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
    use lumencraft_doc::{
        BracketLabelMode, BracketStyle, Cell, ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, GraphicalDataObject, GroupedArrangement,
        GroupedTable, PairwiseBracketSpec, PointStyleOverride, ProjectDocument, ReplicateOverlayStyle, SheetId, SubcolumnMode, SummaryMeasure,
        TableArchetype,
    };
    use lumencraft_palette::{MarkerShape, Rgba8};

    use crate::{ScenePrimitive, compile_graph};

    #[test]
    fn column_point_overrides_restyle_and_hide_individual_points() {
        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).expect("column table");
        table.data_sets[0].cells = vec![vec![Cell::number(4.0)], vec![Cell::number(7.0)], vec![Cell::number(11.0)]];
        let ds = DataSheet::new(SheetId::new("col_1"), "Samples", TableArchetype::Column(table)).expect("data sheet");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_1"), &ds).expect("graph sheet");
        gs.spec.series[0].gdo = GraphicalDataObject::AlignedDotPlot {
            summary: SummaryMeasure::Mean,
            marker: lumencraft_palette::MarkerStyle::default(),
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
        use lumencraft_doc::{
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
                whiskers: lumencraft_doc::WhiskerMethod::MinToMax,
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
        use lumencraft_doc::{
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
