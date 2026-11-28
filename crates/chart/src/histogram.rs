//! Publication-quality histogram scene compiler (`GraphKind::Histogram`).
//!
//! Renders contiguous histogram bars, XY frequency polygon curves, stepped outlines, cumulative
//! frequency curves, optional fitted Gaussian density/CDF overlays, and single- or multi-dataset
//! overlay / side-by-side layouts from either an up-to-date [`AnalysisKind::FrequencyDistribution`]
//! [`ResultSheet`] or directly from a raw [`DataSheet`].

use curvebook_doc::{
    AnalysisKind, DataSheet, FrequencyDistributionSpec, FrequencyOutputMode, GraphSheet, GraphicalDataObject, HistogramBinAlignment,
    HistogramDisplayStyle, HistogramMultiSeriesMode, ProjectDocument, ResultCell, ResultSheet, ResultStatus, SeriesSpec, SheetId,
    SubcolumnAggregation,
};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::frequency::{GAUSSIAN_OVERLAY_POINTS, compute_sheet_frequency_distribution, evaluate_gaussian_overlay};

use crate::markers::emit_marker;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PathCmd, PlottedPointMeta, PointPt, RectPt, ScenePrimitive, SourceCellId, StrokePaint,
};
use crate::xy::{
    LegendItem, SegmentedAxis, configure_plot_rect, format_point_value, primary_info_sheet_id, render_cartesian_chrome,
    render_legend_and_annotations, stroke_with_dash,
};

#[derive(Debug, Clone)]
struct CompiledHistBin {
    edge_low: f64,
    edge_high: f64,
    center: f64,
    value: f64,
}

#[derive(Debug, Clone)]
struct CompiledHistSeries {
    data_set_index: usize,
    label: String,
    color: Rgba8,
    border_color: Rgba8,
    bins: Vec<CompiledHistBin>,
    gaussian: Option<Vec<(f64, f64)>>,
}

#[derive(Debug, Clone)]
struct CompiledHistogramData {
    edges: Vec<f64>,
    centers: Vec<f64>,
    bin_width: f64,
    series: Vec<CompiledHistSeries>,
}

fn cell_number(cell: Option<&ResultCell>) -> Option<f64> {
    match cell {
        Some(ResultCell::Number { value, .. }) if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn mode_detail_offset(mode: FrequencyOutputMode) -> usize {
    match mode {
        FrequencyOutputMode::Counts => 0,
        FrequencyOutputMode::RelativeFrequency => 1,
        FrequencyOutputMode::Percentages => 2,
        FrequencyOutputMode::Density => 3,
        FrequencyOutputMode::CumulativeCounts => 4,
        FrequencyOutputMode::CumulativeFrequency => 5,
        FrequencyOutputMode::CumulativePercentages => 6,
    }
}

const fn gaussian_overlay_dash(style: HistogramDisplayStyle) -> LineDashStyle {
    match style {
        HistogramDisplayStyle::Bars => LineDashStyle::Solid,
        HistogramDisplayStyle::XyCurve | HistogramDisplayStyle::StepOutline => LineDashStyle::Dashed,
    }
}

fn series_spec_for_dataset(graph: &GraphSheet, data_set_index: usize) -> Option<&SeriesSpec> {
    graph
        .spec
        .series
        .iter()
        .find(|s| s.source_sheet_id == graph.spec.primary_source_sheet_id && s.data_set_index == data_set_index)
        .or_else(|| graph.spec.series.iter().find(|s| s.data_set_index == data_set_index))
}

fn series_colors_for_dataset(graph: &GraphSheet, data_set_index: usize, fallback_color: Rgba8) -> (Rgba8, Rgba8) {
    let Some(spec) = series_spec_for_dataset(graph, data_set_index) else {
        return (fallback_color, fallback_color);
    };
    match &spec.gdo {
        GraphicalDataObject::BarWithReplicates { fill_color, border_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, border_color, .. } => (*fill_color, *border_color),
        GraphicalDataObject::BoxAndWhisker { fill_color, stroke_color, .. } | GraphicalDataObject::Violin { fill_color, stroke_color, .. } => {
            (*fill_color, *stroke_color)
        }
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            let c = line.as_ref().map(|l| l.color).or_else(|| marker.map(|m| m.fill_color)).unwrap_or(fallback_color);
            (c, c)
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => (marker.fill_color, marker.stroke_color),
        GraphicalDataObject::BeforeAfterLines { line_color, .. } | GraphicalDataObject::KaplanMeierStep { line_color, .. } => {
            (*line_color, *line_color)
        }
        GraphicalDataObject::PartsOfWhole { .. } => (fallback_color, fallback_color),
    }
}

fn extract_from_frequency_result(graph: &GraphSheet, res: &ResultSheet, fd: &FrequencyDistributionSpec) -> Option<CompiledHistogramData> {
    let detail_tab = res.tabs.iter().find(|t| t.id == "freq-detail")?;
    if detail_tab.rows.is_empty() || detail_tab.column_headers.len() < 10 {
        return None;
    }
    let summary_tab = res.tabs.iter().find(|t| t.id == "freq-summary");
    let series_count = (detail_tab.column_headers.len().saturating_sub(3)) / 7;
    if series_count == 0 {
        return None;
    }

    let effective_mode = if graph.spec.histogram.cumulative { fd.output_mode.to_cumulative() } else { fd.output_mode };
    let val_offset = mode_detail_offset(effective_mode);

    let k = detail_tab.rows.len();
    let mut edges = Vec::with_capacity(k + 1);
    let mut centers = Vec::with_capacity(k);
    for (r_idx, row) in detail_tab.rows.iter().enumerate() {
        let lo = cell_number(row.first())?;
        let hi = cell_number(row.get(1))?;
        let center = cell_number(row.get(2)).unwrap_or(0.5 * (lo + hi));
        if r_idx == 0 {
            edges.push(lo);
        }
        edges.push(hi);
        centers.push(center);
    }
    let bin_width = res
        .scalars
        .iter()
        .find(|s| s.key == "Bin width" && s.data_set_index.is_none())
        .map(|s| s.value)
        .unwrap_or_else(|| if edges.len() >= 2 { (edges[1] - edges[0]).abs().max(1e-12) } else { 1.0 });
    let e_first = *edges.first()?;
    let e_last = *edges.last()?;

    let show_gauss = graph.spec.histogram.show_gaussian_overlay || fd.overlay_gaussian;
    let result_dsi_order: Vec<usize> = res.scalars.iter().filter(|sc| sc.key == "N").filter_map(|sc| sc.data_set_index).collect();
    let mut out_series = Vec::with_capacity(series_count);

    for s_idx in 0..series_count {
        let base_col = 3 + 7 * s_idx;
        let mut bins = Vec::with_capacity(k);
        for (r_idx, row) in detail_tab.rows.iter().enumerate() {
            let edge_low = edges[r_idx];
            let edge_high = edges[r_idx + 1];
            let center = centers[r_idx];
            let value = cell_number(row.get(base_col + val_offset)).unwrap_or(0.0);
            bins.push(CompiledHistBin { edge_low, edge_high, center, value });
        }

        let dsi = result_dsi_order.get(s_idx).copied().or_else(|| graph.spec.series.get(s_idx).map(|s| s.data_set_index)).unwrap_or(s_idx);
        let summary_label = summary_tab.and_then(|st| st.row_labels.get(s_idx)).cloned();
        let spec_label = series_spec_for_dataset(graph, dsi).and_then(|s| s.legend_label.clone());
        let label = spec_label.or(summary_label).unwrap_or_else(|| format!("Series {}", dsi + 1));
        let fallback = graph.spec.palette.color_at(dsi);
        let (color, border_color) = series_colors_for_dataset(graph, dsi, fallback);

        let gaussian = if show_gauss {
            let cached = if effective_mode == fd.output_mode {
                res.fitted_curves
                    .iter()
                    .find(|fc| fc.data_set_index == dsi && fc.equation_id.starts_with("histogram_gaussian"))
                    .or_else(|| res.fitted_curves.get(s_idx).filter(|fc| fc.equation_id.starts_with("histogram_gaussian")))
                    .map(|fc| fc.points.iter().map(|p| (p.x, p.y)).collect::<Vec<(f64, f64)>>())
            } else {
                None
            };
            cached.or_else(|| {
                let row = summary_tab.and_then(|st| st.rows.get(s_idx))?;
                let n = cell_number(row.first())? as usize;
                let mean = cell_number(row.get(6))?;
                let sd = cell_number(row.get(7))?;
                evaluate_gaussian_overlay(mean, sd, n, bin_width, effective_mode, (e_first, e_last), GAUSSIAN_OVERLAY_POINTS)
            })
        } else {
            None
        };

        out_series.push(CompiledHistSeries { data_set_index: dsi, label, color, border_color, bins, gaussian });
    }

    Some(CompiledHistogramData { edges, centers, bin_width, series: out_series })
}

fn extract_from_raw_data_sheet(graph: &GraphSheet, sheet: &DataSheet, warnings: &mut Vec<ChartWarning>) -> Option<CompiledHistogramData> {
    let selected: Vec<usize> =
        graph.spec.series.iter().filter(|s| s.source_sheet_id == graph.spec.primary_source_sheet_id).map(|s| s.data_set_index).collect();
    let effective_mode =
        if graph.spec.histogram.cumulative { graph.spec.histogram.output_mode.to_cumulative() } else { graph.spec.histogram.output_mode };
    let fd_spec = FrequencyDistributionSpec {
        binning_rule: graph.spec.histogram.binning_rule,
        first_bin_center: None,
        last_bin_center: None,
        output_mode: effective_mode,
        overlay_gaussian: graph.spec.histogram.show_gaussian_overlay,
        aggregation: SubcolumnAggregation::PooledGrandSample,
        create_histogram_graph: false,
    };

    let dist = match compute_sheet_frequency_distribution(sheet, &selected, &fd_spec) {
        Ok(d) => d,
        Err(curvebook_stats::StatsError::EmptyInput) => {
            warnings.push(ChartWarning::new("empty_histogram_data", format!("no finite values available to bin on '{}'", sheet.id)));
            return None;
        }
        Err(err) => {
            warnings.push(ChartWarning::new("histogram_binning_error", format!("histogram binning failed on '{}': {err}", sheet.id)));
            return None;
        }
    };

    let mut out_series = Vec::with_capacity(dist.series.len());
    for s in dist.series {
        let dsi = s.data_set_index;
        let fallback = graph.spec.palette.color_at(dsi);
        let (color, border_color) = series_colors_for_dataset(graph, dsi, fallback);
        let label = series_spec_for_dataset(graph, dsi).and_then(|sp| sp.legend_label.clone()).unwrap_or(s.label);
        let bins = s
            .bins
            .into_iter()
            .map(|b| CompiledHistBin { edge_low: b.edge_low, edge_high: b.edge_high, center: b.center, value: b.value_for_mode(effective_mode) })
            .collect();
        out_series.push(CompiledHistSeries { data_set_index: dsi, label, color, border_color, bins, gaussian: s.gaussian_overlay });
    }

    Some(CompiledHistogramData { edges: dist.edges, centers: dist.centers, bin_width: dist.bin_width, series: out_series })
}

fn resolve_histogram_data(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<CompiledHistogramData> {
    let primary_id = &graph.spec.primary_source_sheet_id;
    if let Some(res) = doc.result_sheet(primary_id) {
        match &res.status {
            ResultStatus::Dirty => {
                warnings.push(ChartWarning::new(
                    "pending_source_result",
                    format!("source result sheet '{primary_id}' is pending recalculation; derived data omitted"),
                ));
                return None;
            }
            ResultStatus::Failed { message, .. } => {
                warnings.push(ChartWarning::new(
                    "failed_source_result",
                    format!("source result sheet '{primary_id}' failed: {message}; derived data omitted"),
                ));
                return None;
            }
            ResultStatus::UpToDate | ResultStatus::Imported | ResultStatus::ImportedStale => {}
        }
        if !res.is_computed_up_to_date() {
            warnings
                .push(ChartWarning::new("stale_source_result", format!("source result sheet '{primary_id}' is not up to date; histogram omitted")));
            return None;
        }
        if let AnalysisKind::FrequencyDistribution(fd) = &res.analysis.kind
            && let Some(compiled) = extract_from_frequency_result(graph, res, fd)
        {
            return Some(compiled);
        }
        if let Some(ds) = res.analyzable_data.as_ref() {
            return extract_from_raw_data_sheet(graph, ds, warnings);
        }
        warnings.push(ChartWarning::new("missing_histogram_data", format!("result sheet '{primary_id}' has no frequency distribution data")));
        return None;
    }

    if let Some(ds) = doc.data_sheet(primary_id) {
        return extract_from_raw_data_sheet(graph, ds, warnings);
    }

    warnings.push(ChartWarning::new("missing_source_sheet", format!("primary data sheet '{primary_id}' not found")));
    None
}

fn build_aligned_x_ticks(graph: &GraphSheet, data: &CompiledHistogramData) -> Option<Vec<(f64, String)>> {
    if graph.spec.x_axis.segments.len() > 1 || graph.spec.x_axis.segments.first().and_then(|s| s.major_tick_interval).is_some() {
        return None;
    }
    let coords: &[f64] = match graph.spec.histogram.bin_alignment {
        HistogramBinAlignment::BinCenters => &data.centers,
        HistogramBinAlignment::BinEdges => &data.edges,
    };
    if coords.is_empty() {
        return None;
    }
    let max_labels = match graph.spec.histogram.bin_alignment {
        HistogramBinAlignment::BinCenters => 10_usize,
        HistogramBinAlignment::BinEdges => 11_usize,
    };
    let stride = coords.len().div_ceil(max_labels).max(1);
    let mut ticks = Vec::new();
    for (i, &val) in coords.iter().enumerate() {
        if i % stride == 0 || i + 1 == coords.len() {
            // Avoid crowding the penultimate tick when the final tick is forced.
            if i + 1 == coords.len() && stride > 1 && i % stride != 0 && coords.len().saturating_sub(1) - (i - (i % stride)) < stride / 2 {
                ticks.pop();
            }
            ticks.push((val, format_point_value(val)));
        }
    }
    Some(ticks)
}

fn build_histogram_legend_items(graph: &GraphSheet, data: &CompiledHistogramData) -> Vec<LegendItem> {
    let style = graph.spec.histogram.display_style;
    let alpha_u8 = effective_bar_alpha_u8(graph.spec.histogram.bar_alpha, data.series.len(), graph.spec.histogram.multi_series_mode);
    let mut items = Vec::new();
    for s in &data.series {
        match style {
            HistogramDisplayStyle::Bars => {
                items.push(LegendItem {
                    label: s.label.clone(),
                    marker: None,
                    line_color: None,
                    line_dash: LineDashStyle::Solid,
                    swatch_fill: Some(s.color.with_alpha(alpha_u8)),
                    swatch_border: Some(s.border_color),
                });
            }
            HistogramDisplayStyle::XyCurve | HistogramDisplayStyle::StepOutline => {
                items.push(LegendItem {
                    label: s.label.clone(),
                    marker: None,
                    line_color: Some(s.border_color),
                    line_dash: LineDashStyle::Solid,
                    swatch_fill: None,
                    swatch_border: None,
                });
            }
        }
        if s.gaussian.is_some() {
            items.push(LegendItem {
                label: format!("{} (Gaussian)", s.label),
                marker: None,
                line_color: Some(s.border_color),
                line_dash: gaussian_overlay_dash(style),
                swatch_fill: None,
                swatch_border: None,
            });
        }
    }
    items
}

fn effective_bar_alpha_u8(bar_alpha: f32, series_count: usize, mode: HistogramMultiSeriesMode) -> u8 {
    let clamped = bar_alpha.clamp(0.05, 1.0);
    let eff = if series_count >= 2 && matches!(mode, HistogramMultiSeriesMode::Overlay) { clamped.min(0.55) } else { clamped };
    (eff * 255.0).round().clamp(12.0, 255.0) as u8
}

fn emit_clipped_polyline(scene: &mut ChartScene, points: &[PointPt], stroke: StrokePaint, clip: RectPt) {
    let Some(&first) = points.first() else {
        return;
    };
    if points.len() < 2 {
        return;
    }
    let mut cmds = Vec::with_capacity(points.len());
    cmds.push(PathCmd::MoveTo(first));
    for &pt in points.iter().skip(1) {
        cmds.push(PathCmd::LineTo(pt));
    }
    scene.primitives.push(ScenePrimitive::Path { path: Path2D { commands: cmds }, fill: None, stroke: Some(stroke), clip: Some(clip) });
}

/// Compile a [`curvebook_doc::GraphKind::Histogram`] sheet into vector primitives inside `scene`.
pub fn compile_histogram_graph(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let hist_opt = resolve_histogram_data(graph, doc, &mut scene.warnings);
    let legend_items = hist_opt.as_ref().map(|d| build_histogram_legend_items(graph, d)).unwrap_or_default();
    let show_legend = graph.spec.legend.visible
        && (legend_items.len() > 1 || (legend_items.len() == 1 && hist_opt.as_ref().is_some_and(|d| d.series.iter().any(|s| s.gaussian.is_some()))));
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    let legend_box_w = if show_legend {
        crate::scales::auto_legend_box_width_pt(&graph.spec.legend, legend_items.iter().map(|it| it.label.as_str()), doc, info_id)
    } else {
        None
    };

    configure_plot_rect(graph, scene, false, 0.0, legend_box_w);
    let r = scene.plot_rect_pt;

    let Some(data) = hist_opt else {
        let x_axis = SegmentedAxis::build_exact(&graph.spec.x_axis, r.x, r.right(), (0.0, 1.0));
        let y_axis = SegmentedAxis::build_exact(&graph.spec.y_axis, r.bottom(), r.y, (0.0, 1.0));
        render_cartesian_chrome(scene, graph, doc, &x_axis, &y_axis, None, None);
        render_legend_and_annotations(scene, graph, doc, &[])?;
        return Ok(());
    };

    let (e_first, e_last) = match (data.edges.first().copied(), data.edges.last().copied()) {
        (Some(lo), Some(hi)) if hi > lo => (lo, hi),
        _ => (0.0, 1.0),
    };
    let x_pad = 0.15 * data.bin_width.max((e_last - e_first) * 0.02);
    let x_axis = SegmentedAxis::build_exact(&graph.spec.x_axis, r.x, r.right(), (e_first - x_pad, e_last + x_pad));

    let mut y_samples = vec![0.0_f64];
    for s in &data.series {
        for b in &s.bins {
            if b.value.is_finite() {
                y_samples.push(b.value);
            }
        }
        if let Some(pts) = &s.gaussian {
            for &(_, y) in pts {
                if y.is_finite() {
                    y_samples.push(y);
                }
            }
        }
    }
    let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &y_samples, true, 0.08);
    let (_, y_zero_pt, _) = y_axis.map_clamped_to_nearest(0.0);

    let x_ticks = build_aligned_x_ticks(graph, &data);
    render_cartesian_chrome(scene, graph, doc, &x_axis, &y_axis, None, x_ticks.as_deref());

    let series_count = data.series.len();
    let alpha_u8 = effective_bar_alpha_u8(graph.spec.histogram.bar_alpha, series_count, graph.spec.histogram.multi_series_mode);
    let source_id: SheetId = graph.spec.primary_source_sheet_id.clone();

    match graph.spec.histogram.display_style {
        HistogramDisplayStyle::Bars => {
            let dodged = series_count >= 2 && matches!(graph.spec.histogram.multi_series_mode, HistogramMultiSeriesMode::SideBySide);
            for (s_pos, s) in data.series.iter().enumerate() {
                let fill = FillPaint::solid(s.color.with_alpha(alpha_u8));
                let stroke = StrokePaint::solid(s.border_color, 0.9);
                for (b_idx, bin) in s.bins.iter().enumerate() {
                    let (_, x0_raw, _) = x_axis.map_clamped_to_nearest(bin.edge_low);
                    let (_, x1_raw, _) = x_axis.map_clamped_to_nearest(bin.edge_high);
                    let (mut bx0, mut bx1) = if x0_raw <= x1_raw { (x0_raw, x1_raw) } else { (x1_raw, x0_raw) };
                    if dodged {
                        let span = (bx1 - bx0).max(0.5);
                        let pad = 0.04 * span;
                        let usable = (span - 2.0 * pad).max(0.5);
                        let sub_w = usable / (series_count as f64);
                        bx0 += pad + (s_pos as f64) * sub_w;
                        bx1 = bx0 + sub_w;
                    }
                    let (_, y_val_pt, _) = y_axis.map_clamped_to_nearest(bin.value);
                    let top = y_val_pt.min(y_zero_pt);
                    let height = (y_zero_pt - y_val_pt).abs();
                    let width = (bx1 - bx0).max(0.5);
                    if height > 0.05 {
                        let rect = RectPt::new(bx0, top, width, height);
                        scene.primitives.push(ScenePrimitive::Path {
                            path: Path2D::rect(rect),
                            fill: Some(fill),
                            stroke: Some(stroke),
                            clip: Some(r),
                        });
                    }
                    scene.plotted_points.push(PlottedPointMeta {
                        source_cell: SourceCellId { sheet_id: source_id.clone(), row: b_idx, col: s.data_set_index, subcol: 0 },
                        center_pt: PointPt::new(0.5 * (bx0 + bx1), y_val_pt),
                        radius_pt: 4.0,
                        series_label: s.label.clone(),
                        value_summary: format!("bin {} = {}", format_point_value(bin.center), format_point_value(bin.value)),
                    });
                }
            }
        }
        HistogramDisplayStyle::XyCurve => {
            for s in &data.series {
                let mut poly = Vec::with_capacity(s.bins.len());
                let marker = MarkerStyle {
                    shape: MarkerShape::Circle,
                    fill_mode: MarkerFillMode::Filled,
                    size_pt: 2.6,
                    fill_color: s.color,
                    stroke_color: s.border_color,
                    stroke_width_pt: 0.6,
                };
                for (b_idx, bin) in s.bins.iter().enumerate() {
                    let (_, cx, _) = x_axis.map_clamped_to_nearest(bin.center);
                    let (_, cy, _) = y_axis.map_clamped_to_nearest(bin.value);
                    let pt = PointPt::new(cx, cy);
                    poly.push(pt);
                    scene.plotted_points.push(PlottedPointMeta {
                        source_cell: SourceCellId { sheet_id: source_id.clone(), row: b_idx, col: s.data_set_index, subcol: 0 },
                        center_pt: pt,
                        radius_pt: 4.0,
                        series_label: s.label.clone(),
                        value_summary: format!("bin {} = {}", format_point_value(bin.center), format_point_value(bin.value)),
                    });
                }
                emit_clipped_polyline(scene, &poly, StrokePaint::solid(s.border_color, 1.8), r);
                for &pt in &poly {
                    emit_marker(&mut scene.primitives, pt, &marker, Some(r));
                }
            }
        }
        HistogramDisplayStyle::StepOutline => {
            for s in &data.series {
                let mut poly = Vec::with_capacity(s.bins.len() * 2 + 2);
                if let Some(first) = s.bins.first() {
                    let (_, x0, _) = x_axis.map_clamped_to_nearest(first.edge_low);
                    poly.push(PointPt::new(x0, y_zero_pt));
                }
                for (b_idx, bin) in s.bins.iter().enumerate() {
                    let (_, x0, _) = x_axis.map_clamped_to_nearest(bin.edge_low);
                    let (_, x1, _) = x_axis.map_clamped_to_nearest(bin.edge_high);
                    let (_, cy, _) = y_axis.map_clamped_to_nearest(bin.value);
                    poly.push(PointPt::new(x0, cy));
                    poly.push(PointPt::new(x1, cy));
                    scene.plotted_points.push(PlottedPointMeta {
                        source_cell: SourceCellId { sheet_id: source_id.clone(), row: b_idx, col: s.data_set_index, subcol: 0 },
                        center_pt: PointPt::new(0.5 * (x0 + x1), cy),
                        radius_pt: 4.0,
                        series_label: s.label.clone(),
                        value_summary: format!("bin {} = {}", format_point_value(bin.center), format_point_value(bin.value)),
                    });
                }
                if let Some(last) = s.bins.last() {
                    let (_, x1, _) = x_axis.map_clamped_to_nearest(last.edge_high);
                    poly.push(PointPt::new(x1, y_zero_pt));
                }
                emit_clipped_polyline(scene, &poly, StrokePaint::solid(s.border_color, 1.6), r);
            }
        }
    }

    // Optional Gaussian density / CDF overlay curves drawn on top of bars/curves.
    for s in &data.series {
        if let Some(pts) = &s.gaussian {
            let mapped: Vec<PointPt> = pts
                .iter()
                .filter_map(|&(x, y)| {
                    if !x.is_finite() || !y.is_finite() {
                        return None;
                    }
                    let (_, px, _) = x_axis.map_clamped_to_nearest(x);
                    let (_, py, _) = y_axis.map_clamped_to_nearest(y);
                    Some(PointPt::new(px, py))
                })
                .collect();
            let dash = gaussian_overlay_dash(graph.spec.histogram.display_style);
            let stroke = stroke_with_dash(s.border_color, 1.7, dash);
            emit_clipped_polyline(scene, &mapped, stroke, r);
        }
    }

    let final_legend = if show_legend { legend_items.as_slice() } else { &[] };
    render_legend_and_annotations(scene, graph, doc, final_legend)?;
    Ok(())
}
