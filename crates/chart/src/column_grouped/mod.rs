//! Column, grouped bar, replicate scatter, box-and-whisker, violin, floating bar,
//! before/after paired lines, and auto-stacked collision-free significance brackets.

mod brackets_cld;
mod horizontal;
mod items;

use curvebook_doc::{
    AxisSpec, CartesianOrientation, CategoricalTickSource, ColumnTable, ErrorBarMetric, ErrorBarSpec, GraphSheet, GraphicalDataObject,
    GroupedArrangement, GroupedTable, NestedTable, PairwiseDisplayMode, ProjectDocument, ReplicateOverlayStyle, SheetId, SubcolumnMode,
    SummaryMeasure, TableArchetype, TickLabelFormat,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::sum::SummaryMoments;
use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_stats::column::summarize_pre_summarized_points;

pub(crate) use brackets_cld::{ResolvedCldLabels, cld_reserved_top_margin_pt, render_cld_labels, resolve_cld_overlay};
pub(crate) use items::{
    BoxWhiskerRenderCfg, FloatingBarRenderCfg, ReplicateDotCtx, ViolinRenderCfg, emit_replicate_dots_oriented, render_box_whisker_obs,
    render_floating_bar_bounds, render_violin_obs,
};

use brackets_cld::{count_active_brackets, item_summary_value, render_pairwise_brackets};
use horizontal::compile_horizontal_column_or_grouped;
use items::{render_before_after_connectors, render_single_item_oriented};

use crate::scene::{ChartError, ChartScene, ChartWarning, SourceCellId};
use crate::xy::{
    LegendItem, SegmentedAxis, compute_central_tendency, compute_raw_error_bounds, configure_plot_rect, primary_info_sheet_id,
    render_cartesian_chrome, render_legend_and_annotations, resolve_data_sheet, summarize_subcells,
};

#[derive(Debug, Clone)]
pub(crate) struct RawObs {
    pub(crate) value: f64,
    pub(crate) source: SourceCellId,
}

#[derive(Debug, Clone)]
enum ExtractedKind {
    Raw(Vec<RawObs>),
    PreSummarized { mode: SubcolumnMode, center: f64, lower: f64, upper: f64, source: SourceCellId },
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

fn item_z_footprint(item: &CategorySlotItem) -> (f64, f64) {
    let val_extent = match &item.payload {
        ExtractedKind::PreSummarized { center, upper, .. } => center.abs().max(upper.abs()),
        ExtractedKind::Raw(obs) => {
            let summary = gdo_summary_measure(&item.gdo);
            if matches!(summary, SummaryMeasure::Mean) {
                let mut sum = 0.0;
                let mut count = 0_usize;
                for o in obs {
                    if o.value.is_finite() {
                        sum += o.value;
                        count += 1;
                    }
                }
                if count > 0 { (sum / (count as f64)).abs() } else { 0.0 }
            } else {
                let mut tmp_warnings = Vec::new();
                let vals = raw_values(obs);
                compute_central_tendency(&vals, summary, &mut tmp_warnings).map_or(0.0, f64::abs)
            }
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

/// Precompute item footprints once in $O(S)$, z-sort in $O(S \log S)$, and narrow co-located
/// superimposed bars in $O(S \log S)$ instead of scanning prior slots quadratically (`GAP-02-07`).
fn prepare_superimposed_items(items: &mut Vec<CategorySlotItem>) {
    if items.len() <= 1 {
        return;
    }
    let mut tagged: Vec<((f64, f64), CategorySlotItem)> = std::mem::take(items).into_iter().map(|it| (item_z_footprint(&it), it)).collect();
    tagged.sort_by(|(za, a), (zb, b)| zb.0.total_cmp(&za.0).then_with(|| zb.1.total_cmp(&za.1)).then_with(|| a.group_key.cmp(&b.group_key)));
    *items = tagged.into_iter().map(|(_, it)| it).collect();

    let mut x_order: Vec<(f64, usize)> = items.iter().enumerate().filter_map(|(pos, it)| it.x_pos.is_finite().then_some((it.x_pos, pos))).collect();
    x_order.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut start = 0_usize;
    while start < x_order.len() {
        let anchor_x = x_order[start].0;
        let mut end = start + 1;
        while end < x_order.len() && (x_order[end].0 - anchor_x).abs() < 1e-6 {
            end += 1;
        }
        if end - start > 1 {
            let cluster = &mut x_order[start..end];
            if cluster.windows(2).any(|w| w[0].1 > w[1].1) {
                cluster.sort_unstable_by_key(|&(_, pos)| pos);
            }
            for (rank_at_x, &(_, pos)) in cluster.iter().enumerate().skip(1) {
                if matches!(items[pos].gdo, GraphicalDataObject::BarWithReplicates { .. } | GraphicalDataObject::FloatingBar { .. }) {
                    items[pos].slot_width_data *= (1.0 - 0.12 * (rank_at_x as f64).min(4.0)).max(0.35);
                }
            }
        }
        start = end;
    }
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
        return ExtractedKind::PreSummarized {
            mode,
            center,
            lower,
            upper,
            source: SourceCellId { sheet_id: sheet_id.clone(), row: row0, col, subcol: 0 },
        };
    }

    let mut first_plottable = None;
    let mut populated_rows = 0_usize;
    for (row, subcells) in ds.cells.iter().enumerate() {
        if subcells.first().and_then(curvebook_doc::Cell::active_number).is_none() {
            continue;
        }
        populated_rows += 1;
        if first_plottable.is_none()
            && let Some((center, lower, upper)) = summarize_subcells(mode, subcells, SummaryMeasure::Mean, err_spec, warnings)
        {
            first_plottable = Some(ExtractedKind::PreSummarized {
                mode,
                center,
                lower,
                upper,
                source: SourceCellId { sheet_id: sheet_id.clone(), row, col, subcol: 0 },
            });
        }
    }
    if populated_rows > 1 {
        warnings.push(ChartWarning::new(
            "multiple_unpoolable_summary_rows",
            format!(
                "column `{}` has {populated_rows} pre-summarized {mode:?} rows without sample size N; plotted first active summary row only",
                ds.title
            ),
        ));
    }
    first_plottable.unwrap_or_else(|| ExtractedKind::Raw(Vec::new()))
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
            ExtractedKind::PreSummarized {
                mode: table.subcolumn_mode,
                center,
                lower,
                upper,
                source: SourceCellId { sheet_id: sheet_id.clone(), row: r, col: c, subcol: 0 },
            }
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

fn data_set_title(archetype: &TableArchetype, data_set_index: usize) -> Option<String> {
    let title = match archetype {
        TableArchetype::Column(t) => t.data_sets.get(data_set_index).map(|ds| ds.title.as_str()),
        TableArchetype::Nested(t) => t.groups.get(data_set_index).map(|g| g.title.as_str()),
        TableArchetype::Contingency(t) => t.data_sets.get(data_set_index).map(|ds| ds.title.as_str()),
        _ => None,
    }?;
    (!title.trim().is_empty()).then(|| title.to_owned())
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
            let label =
                s.legend_label.clone().or_else(|| data_set_title(&sheet.archetype, s.data_set_index)).unwrap_or_else(|| format!("Group {}", idx + 1));
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

    #[test]
    fn column_nested_and_contingency_legends_fall_back_to_data_set_titles() {
        use curvebook_doc::{ContingencyTable, NestedTable};

        // 1. Column graph: actual column titles appear in both build_legend_items and compiled scene,
        // while explicit legend_label overrides and out-of-bounds indices fall back to "Group N".
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2).expect("column table");
        col.data_sets[0].title = String::from("Vehicle");
        col.data_sets[0].cells = vec![vec![Cell::number(10.0)], vec![Cell::number(12.0)]];
        col.data_sets[1].title = String::from("Drug X");
        col.data_sets[1].cells = vec![vec![Cell::number(18.0)], vec![Cell::number(20.0)]];
        col.data_sets[2].title = String::from("Drug Y");
        col.data_sets[2].cells = vec![vec![Cell::number(25.0)], vec![Cell::number(27.0)]];
        let col_ds = DataSheet::new(SheetId::new("col_leg"), "Column", TableArchetype::Column(col)).expect("col ds");
        let mut col_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_col_leg"), &col_ds).expect("col gs");
        col_gs.spec.legend.visible = true;
        col_gs.spec.series[0].legend_label = None;
        col_gs.spec.series[1].legend_label = None;
        col_gs.spec.series[2].legend_label = Some(String::from("Custom Override"));

        let col_items = super::build_legend_items(&col_gs, &col_ds);
        let col_labels: Vec<&str> = col_items.iter().map(|it| it.label.as_str()).collect();
        assert_eq!(col_labels, vec!["Vehicle", "Drug X", "Custom Override"]);

        // Out-of-bounds data_set_index falls back to "Group N".
        let mut oob_gs = col_gs.clone();
        oob_gs.spec.series[1].data_set_index = 99;
        let oob_items = super::build_legend_items(&oob_gs, &col_ds);
        assert_eq!(oob_items[1].label, "Group 2");

        // End-to-end compile_graph check: rendered scene contains column titles, not "Group 1"/"Group 2".
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(col_ds).expect("add col ds");
        doc.add_graph_sheet(col_gs.clone()).expect("add col gs");
        let scene = compile_graph(&col_gs, &doc).expect("compile col graph");
        let texts: Vec<String> = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|t| t == "Vehicle"), "expected Vehicle in legend texts: {texts:?}");
        assert!(texts.iter().any(|t| t == "Drug X"), "expected Drug X in legend texts: {texts:?}");
        assert!(texts.iter().any(|t| t == "Custom Override"), "expected Custom Override in legend texts: {texts:?}");
        assert!(!texts.iter().any(|t| t == "Group 1" || t == "Group 2"), "unexpected Group N fallback in scene texts: {texts:?}");

        // 2. Nested graph: falls back to NestedGroupColumn::title.
        let mut nested = NestedTable::new(2, 2, 2).expect("nested table");
        nested.groups[0].title = String::from("Wildtype");
        nested.groups[1].title = String::from("Knockout");
        let nested_ds = DataSheet::new(SheetId::new("nst_leg"), "Nested", TableArchetype::Nested(nested)).expect("nested ds");
        let mut nested_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_nst_leg"), &nested_ds).expect("nested gs");
        for s in &mut nested_gs.spec.series {
            s.legend_label = None;
        }
        let nested_labels: Vec<String> = super::build_legend_items(&nested_gs, &nested_ds).into_iter().map(|it| it.label).collect();
        assert_eq!(nested_labels, vec!["Wildtype", "Knockout"]);

        // 3. Contingency graph: falls back to ContingencyTable data_sets title.
        let mut cont = ContingencyTable::new(2, 2).expect("contingency table");
        cont.data_sets[0].title = String::from("Responder");
        cont.data_sets[1].title = String::from("Non-responder");
        let cont_ds = DataSheet::new(SheetId::new("ctg_leg"), "Contingency", TableArchetype::Contingency(cont)).expect("cont ds");
        let mut cont_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_ctg_leg"), &cont_ds).expect("cont gs");
        for s in &mut cont_gs.spec.series {
            s.legend_label = None;
        }
        let cont_labels: Vec<String> = super::build_legend_items(&cont_gs, &cont_ds).into_iter().map(|it| it.label).collect();
        assert_eq!(cont_labels, vec!["Responder", "Non-responder"]);
    }
}
