//! XY scatter, bar, area, replicate trajectory, spline interpolation, and error-bar series compilation.

use curvebook_doc::{
    AxisSpec, DataSheet, ErrorBarMetric, ErrorBarSpec, FramePreset, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement,
    LineConnectionKind, ProjectDocument, ReplicateLineMode, SeriesSpec, SheetId, SubcolumnMode, SummaryMeasure, TableArchetype, XColumnMode, XyTable,
};
use curvebook_math::AxisScale;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};

use super::fit_overlay::{FittedOverlayData, OverlayDrawTarget, render_fitted_curve_overlay, resolve_fitted_overlay};
use super::{
    AxisErrorTarget, LegendItem, SegmentedAxis, configure_plot_rect, emit_directional_error_bar, format_point_value, is_log_like,
    primary_info_sheet_id, render_cartesian_chrome, render_legend_and_annotations_with_offset, resolve_data_sheet, stroke_with_dash,
    summarize_subcells,
};
use crate::markers::emit_marker;
use crate::scales::scale_transform;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PathCmd, PlottedPointMeta, PointPt, RectPt, ScenePrimitive, SourceCellId, StrokePaint,
};

// ============================================================================
// XY Scene Builder (`compile_xy_graph`)
// ============================================================================

#[derive(Debug, Clone)]
struct XyDatum {
    row: usize,
    x_center: f64,
    x_lo: f64,
    x_hi: f64,
    y_center: f64,
    y_lo: f64,
    y_hi: f64,
}

#[derive(Debug, Clone)]
struct ExtractedXySeries {
    series_index: usize,
    sheet_id: SheetId,
    col_index: usize,
    active_x_rows: Vec<(usize, f64)>,
    points: Vec<XyDatum>,
    replicates: Vec<(f64, f64, usize, usize)>,
    has_x_err: bool,
    has_y_err: bool,
    label: String,
    default_color: Rgba8,
    fitted_overlay: Option<FittedOverlayData>,
    rout_outliers: Vec<RoutOutlier>,
}

#[derive(Debug, Clone)]
struct RoutOutlier {
    source: SourceCellId,
    x: f64,
    y: f64,
}

/// Compile an XY graph sheet into vector primitives inside `scene`.
pub fn compile_xy_graph(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    compile_xy_graph_with_options(graph, doc, &crate::CompileOptions::default(), scene)
}

fn push_unresolved_source_warning(doc: &ProjectDocument, sheet_id: &SheetId, is_primary: bool, warnings: &mut Vec<ChartWarning>) {
    if let Some(res) = doc.result_sheet(sheet_id) {
        match &res.status {
            curvebook_doc::ResultStatus::Dirty => {
                warnings.push(ChartWarning::new(
                    "pending_source_result",
                    format!("source result sheet '{sheet_id}' is pending recalculation; derived data omitted"),
                ));
                return;
            }
            curvebook_doc::ResultStatus::Failed { message, .. } => {
                warnings.push(ChartWarning::new(
                    "failed_source_result",
                    format!("source result sheet '{sheet_id}' failed: {message}; derived data omitted"),
                ));
                return;
            }
            curvebook_doc::ResultStatus::UpToDate | curvebook_doc::ResultStatus::Imported | curvebook_doc::ResultStatus::ImportedStale => {}
        }
    }
    let prefix = if is_primary { "primary data sheet" } else { "series source sheet" };
    warnings.push(ChartWarning::new("missing_source_sheet", format!("{prefix} '{sheet_id}' not found")));
}

/// Compile an XY graph sheet into vector primitives inside `scene` with explicit [`crate::CompileOptions`].
pub fn compile_xy_graph_with_options(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    options: &crate::CompileOptions,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    if resolve_data_sheet(doc, &graph.spec.primary_source_sheet_id).is_none() {
        push_unresolved_source_warning(doc, &graph.spec.primary_source_sheet_id, true, &mut scene.warnings);
    }

    let has_y2_series = graph.spec.series.iter().any(|s| s.bind_to_right_y2);
    let has_y2 = graph.spec.y2_axis.is_some() || has_y2_series || matches!(graph.spec.frame_preset, FramePreset::DualYSecondaryRight);
    let extracted = extract_all_xy_series(graph, doc, &mut scene.warnings);
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    let legend_box_w = crate::scales::auto_legend_box_width_pt(&graph.spec.legend, extracted.iter().map(|d| d.label.as_str()), doc, info_id);
    let fit_summary_w = crate::fit_summary::reserved_adjacent_fit_summary_width_pt(graph, doc, scene.width_pt, has_y2, &options.fit_summary);
    let right_gutter_w = match (legend_box_w, fit_summary_w) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    configure_plot_rect(graph, scene, has_y2, 0.0, right_gutter_w);
    let (x_vals, y_vals, y2_vals) = collect_xy_domain_values(&graph.spec, &extracted);
    let has_bar_y =
        graph.spec.series.iter().any(|s| !s.bind_to_right_y2 && s.show_raw_data && matches!(s.gdo, GraphicalDataObject::BarWithReplicates { .. }));
    let has_bar_y2 =
        graph.spec.series.iter().any(|s| s.bind_to_right_y2 && s.show_raw_data && matches!(s.gdo, GraphicalDataObject::BarWithReplicates { .. }));

    let r = scene.plot_rect_pt;
    let y_headroom = crate::scales::DEFAULT_AUTORANGE_HEADROOM.default_xy_top_headroom_frac;
    let x_axis = SegmentedAxis::build(&graph.spec.x_axis, r.x, r.right(), &x_vals, false, 0.0);
    let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &y_vals, has_bar_y, y_headroom);
    let y2_axis = if has_y2 {
        let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2"));
        let source = if y2_vals.is_empty() { &y_vals } else { &y2_vals };
        Some(SegmentedAxis::build(&y2_spec, r.bottom(), r.y, source, has_bar_y2, y_headroom))
    } else {
        None
    };

    render_cartesian_chrome(scene, graph, doc, &x_axis, &y_axis, y2_axis.as_ref(), None);

    let mut render_order: Vec<usize> = (0..graph.spec.series.len().min(extracted.len())).collect();
    if matches!(graph.spec.grouped_arrangement, GroupedArrangement::Superimposed) {
        render_order.sort_by(|&a, &b| {
            let (ma, la) = series_z_size(&graph.spec.series[a].gdo);
            let (mb, lb) = series_z_size(&graph.spec.series[b].gdo);
            mb.total_cmp(&ma).then_with(|| lb.total_cmp(&la)).then_with(|| a.cmp(&b))
        });
    }
    let mut curve_samples_pt: Vec<PointPt> = Vec::new();
    for idx in render_order {
        let series_spec = &graph.spec.series[idx];
        let data = &extracted[idx];
        let target_y = if series_spec.bind_to_right_y2 { y2_axis.as_ref().unwrap_or(&y_axis) } else { &y_axis };
        render_xy_series(scene, &graph.spec, series_spec, data, &x_axis, target_y);
        if options.fit_summary.enabled
            && let Some(ov) = &data.fitted_overlay
        {
            let dx_data = series_data_x_offset(series_spec);
            let (dx_pt, dy_pt) = series_pt_offset(series_spec);
            for cp in &ov.curve.points {
                if let (Some((_, px)), Some((_, py))) = (x_axis.map_value(cp.x + dx_data), target_y.map_value(cp.y)) {
                    curve_samples_pt.push(PointPt::new(px + dx_pt, py + dy_pt));
                }
            }
        }
    }

    let mut legend_items = Vec::new();
    for (series_spec, data) in graph.spec.series.iter().zip(extracted.iter()) {
        legend_items.push(LegendItem {
            label: data.label.clone(),
            marker: match &series_spec.gdo {
                GraphicalDataObject::ScatterAndLine { marker, .. } => *marker,
                GraphicalDataObject::AlignedDotPlot { marker, .. } => Some(*marker),
                _ => None,
            },
            line_color: match &series_spec.gdo {
                GraphicalDataObject::ScatterAndLine { line: Some(l), .. } => Some(l.color),
                _ if data.fitted_overlay.is_some() => Some(data.default_color),
                _ => None,
            },
            line_dash: match &series_spec.gdo {
                GraphicalDataObject::ScatterAndLine { line: Some(l), .. } => l.dash_style,
                _ => LineDashStyle::Solid,
            },
            swatch_fill: match &series_spec.gdo {
                GraphicalDataObject::ScatterAndLine { area_fill: Some(af), .. } => Some(af.fill_color),
                GraphicalDataObject::BarWithReplicates { fill_color, .. } => Some(*fill_color),
                _ => None,
            },
            swatch_border: match &series_spec.gdo {
                GraphicalDataObject::BarWithReplicates { border_color, .. } => Some(*border_color),
                _ => None,
            },
        });
    }

    let legend_bounds = render_legend_and_annotations_with_offset(scene, graph, doc, &legend_items, 0.0)?;
    if options.fit_summary.enabled {
        let entries = crate::fit_summary::extract_fit_summary_entries(graph, doc, options.fit_summary.max_metrics_per_series);
        let legend_bottom_y = legend_bounds.filter(|lb| lb.x > scene.plot_rect_pt.right()).map(RectPt::bottom);
        let y2_offset = if has_y2 { 48.0 } else { 0.0 };
        if let Some(eval) =
            crate::fit_summary::evaluate_fit_summary_placement(scene, &entries, &curve_samples_pt, legend_bottom_y, y2_offset, &options.fit_summary)
        {
            crate::fit_summary::render_fit_summary_card(
                scene,
                &entries,
                eval.chosen.bounds_pt,
                eval.chosen.font_size_pt,
                options.fit_summary.bordered,
            );
        }
    }
    Ok(())
}

fn extract_all_xy_series(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Vec<ExtractedXySeries> {
    let mut out = Vec::with_capacity(graph.spec.series.len());
    let palette = graph.spec.palette;
    for (idx, series_spec) in graph.spec.series.iter().enumerate() {
        let color = palette.color_at(idx);
        let sheet_opt = resolve_data_sheet(doc, &series_spec.source_sheet_id);
        if sheet_opt.is_none() && series_spec.source_sheet_id != graph.spec.primary_source_sheet_id {
            push_unresolved_source_warning(doc, &series_spec.source_sheet_id, false, warnings);
        }

        let mut active_x_rows = Vec::new();
        let mut points = Vec::new();
        let mut replicates = Vec::new();
        let mut fallback_label = format!("Series {}", idx + 1);
        let has_x_err = series_spec.x_error_bars.is_some();
        let has_y_err = series_spec.y_error_bars.is_some();

        let sheet_id = sheet_opt.map_or_else(|| series_spec.source_sheet_id.clone(), |s| s.id.clone());
        if let Some(sheet) = sheet_opt
            && let TableArchetype::Xy(xy) = &sheet.archetype
        {
            if let Some(ds) = xy.data_sets.get(series_spec.data_set_index) {
                fallback_label = ds.title.clone();
                extract_xy_table_series(xy, series_spec, &mut active_x_rows, &mut points, &mut replicates, warnings);
            } else {
                warnings.push(ChartWarning::new(
                    "missing_data_set",
                    format!("data set index {} out of bounds in '{}'", series_spec.data_set_index, sheet.id),
                ));
            }
        }

        let (fitted_overlay, rout_outliers) = match resolve_fitted_overlay(doc, series_spec, warnings) {
            Some((overlay, outliers)) => {
                let markers = sheet_opt.map_or_else(Vec::new, |sheet| resolve_rout_outliers(sheet, series_spec.data_set_index, outliers));
                (Some(overlay), markers)
            }
            None => (None, Vec::new()),
        };
        let base_label = series_spec.legend_label.clone().unwrap_or(fallback_label);
        let label = match fitted_overlay.as_ref().and_then(|ov| ov.imported_label.as_deref()) {
            Some(tag) => format!("{base_label} [{tag}]"),
            None => base_label,
        };
        out.push(ExtractedXySeries {
            series_index: idx,
            sheet_id,
            col_index: series_spec.data_set_index,
            active_x_rows,
            points,
            replicates,
            has_x_err,
            has_y_err,
            label,
            default_color: color,
            fitted_overlay,
            rout_outliers,
        });
    }
    out
}

fn extract_xy_table_series(
    xy: &XyTable,
    series_spec: &SeriesSpec,
    active_x_rows: &mut Vec<(usize, f64)>,
    points: &mut Vec<XyDatum>,
    replicates: &mut Vec<(f64, f64, usize, usize)>,
    warnings: &mut Vec<ChartWarning>,
) {
    let Some(ds) = xy.data_sets.get(series_spec.data_set_index) else {
        return;
    };
    let y_summary = if xy.y_subcolumn_mode == SubcolumnMode::MedianIqr
        || series_spec.y_error_bars.is_some_and(|e| e.metric == ErrorBarMetric::InterquartileRange)
    {
        SummaryMeasure::Median
    } else {
        SummaryMeasure::Mean
    };
    let x_submode = match xy.x_mode {
        XColumnMode::Numbers { subcolumn_mode } => subcolumn_mode,
        XColumnMode::DateOrElapsedTime { .. } | XColumnMode::GeneratedSequence { .. } => SubcolumnMode::Single,
    };
    let keep_replicates = !xy.y_subcolumn_mode.is_pre_summarized()
        && (series_spec.y_error_bars.is_none()
            || matches!(
                series_spec.gdo,
                GraphicalDataObject::ScatterAndLine { replicate_lines: ReplicateLineMode::IndividualReplicateTrajectories, .. }
                    | GraphicalDataObject::BarWithReplicates { replicate_overlay: Some(_), .. }
            ));

    for row in 0..xy.row_count() {
        let Some((xc, xlo, xhi)) = extract_xy_x_row(xy, row, x_submode, series_spec.x_error_bars.as_ref(), warnings) else {
            continue;
        };
        active_x_rows.push((row, xc));
        let Some(subcells) = ds.cells.get(row) else {
            continue;
        };
        if keep_replicates {
            for (subcol, cell) in subcells.iter().enumerate() {
                if let Some(yv) = cell.active_number() {
                    replicates.push((xc, yv, row, subcol));
                }
            }
        }
        if let Some((yc, ylo, yhi)) = summarize_subcells(xy.y_subcolumn_mode, subcells, y_summary, series_spec.y_error_bars.as_ref(), warnings) {
            points.push(XyDatum { row, x_center: xc, x_lo: xlo, x_hi: xhi, y_center: yc, y_lo: ylo, y_hi: yhi });
        }
    }
}

fn extract_xy_x_row(
    xy: &XyTable,
    row: usize,
    x_submode: SubcolumnMode,
    x_err: Option<&ErrorBarSpec>,
    warnings: &mut Vec<ChartWarning>,
) -> Option<(f64, f64, f64)> {
    if matches!(xy.x_mode, XColumnMode::Numbers { .. }) && (x_submode.is_pre_summarized() || matches!(x_submode, SubcolumnMode::RawReplicates(_))) {
        let cells = xy.x_cells.get(row)?;
        summarize_subcells(x_submode, cells, SummaryMeasure::Mean, x_err, warnings)
    } else {
        let x = xy.x_value_at(row)?;
        Some((x, x, x))
    }
}

fn resolve_rout_outliers(sheet: &DataSheet, col: usize, outliers: &[SourceCellId]) -> Vec<RoutOutlier> {
    let TableArchetype::Xy(xy) = &sheet.archetype else {
        return Vec::new();
    };
    let Some(ds) = xy.data_sets.get(col) else {
        return Vec::new();
    };
    outliers
        .iter()
        .filter(|source| source.sheet_id == sheet.id && source.col == col)
        .filter_map(|source| {
            let x = xy.x_summary_at(source.row).ok().flatten()?;
            let y = ds.cells.get(source.row)?.get(source.subcol)?.active_number()?;
            Some(RoutOutlier { source: source.clone(), x, y })
        })
        .collect()
}

fn series_data_x_offset(spec: &SeriesSpec) -> f64 {
    if spec.offset.x_data.is_finite() { spec.offset.x_data } else { 0.0 }
}

fn series_pt_offset(spec: &SeriesSpec) -> (f64, f64) {
    let dx = if spec.offset.x_pt.is_finite() { spec.offset.x_pt } else { 0.0 };
    // Page-space Y grows downward, so positive `offset.y_pt` nudges visually upward (`-y`).
    let dy = if spec.offset.y_pt.is_finite() { -spec.offset.y_pt } else { 0.0 };
    (dx, dy)
}

fn collect_xy_domain_values(spec: &GraphSpec, extracted: &[ExtractedXySeries]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut x_vals = Vec::new();
    let mut y_vals = Vec::new();
    let mut y2_vals = Vec::new();
    for (s_spec, item) in spec.series.iter().zip(extracted.iter()) {
        let target_y = if s_spec.bind_to_right_y2 { &mut y2_vals } else { &mut y_vals };
        let dx = series_data_x_offset(s_spec);
        if s_spec.show_raw_data {
            for pt in &item.points {
                x_vals.push(pt.x_center + dx);
                if item.has_x_err {
                    x_vals.push(pt.x_lo + dx);
                    x_vals.push(pt.x_hi + dx);
                }
                target_y.push(pt.y_center);
                if item.has_y_err {
                    target_y.push(pt.y_lo);
                    target_y.push(pt.y_hi);
                }
            }
            for &(rx, ry, _, _) in &item.replicates {
                x_vals.push(rx + dx);
                target_y.push(ry);
            }
            for point in &item.rout_outliers {
                x_vals.push(point.x + dx);
                target_y.push(point.y);
            }
        }
        if let Some(ov) = &item.fitted_overlay {
            for cp in &ov.curve.points {
                x_vals.push(cp.x + dx);
                target_y.push(cp.y);
                if ov.spec.show_confidence_band {
                    if let Some(lo) = cp.ci_lower {
                        target_y.push(lo);
                    }
                    if let Some(hi) = cp.ci_upper {
                        target_y.push(hi);
                    }
                }
                if ov.spec.show_prediction_band {
                    if let Some(lo) = cp.pred_lower {
                        target_y.push(lo);
                    }
                    if let Some(hi) = cp.pred_upper {
                        target_y.push(hi);
                    }
                }
            }
        }
    }
    (x_vals, y_vals, y2_vals)
}

fn render_xy_series(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    spec: &SeriesSpec,
    data: &ExtractedXySeries,
    x_axis: &SegmentedAxis,
    y_axis: &SegmentedAxis,
) {
    let clip = scene.plot_rect_pt;
    let is_bar = matches!(spec.gdo, GraphicalDataObject::BarWithReplicates { .. });
    let (marker_opt, line_opt, area_opt, is_spaghetti) = match &spec.gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill, replicate_lines } => {
            (*marker, *line, *area_fill, *replicate_lines == ReplicateLineMode::IndividualReplicateTrajectories)
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => (Some(*marker), None, None, false),
        GraphicalDataObject::BarWithReplicates { .. } => (None, None, None, false),
        _ => (Some(MarkerStyle { fill_color: data.default_color, stroke_color: data.default_color, ..MarkerStyle::default() }), None, None, false),
    };

    let dx_data = series_data_x_offset(spec);
    let (dx_pt, dy_pt) = series_pt_offset(spec);

    // 1. Fitted regression curve overlay & bands (always rendered even when `show_raw_data == false`)
    if let Some(ov) = &data.fitted_overlay {
        let curve_color = crate::fit_summary::resolved_series_curve_color(spec, data.default_color);
        render_fitted_curve_overlay(
            &mut scene.primitives,
            &mut scene.warnings,
            ov,
            OverlayDrawTarget { x_axis, y_axis, color: curve_color, clip, dx_data, dx_pt, dy_pt },
        );
    }

    if !spec.show_raw_data {
        return;
    }

    // 2. Map summary points & emit warnings for off-domain coordinates
    let mut mapped_runs: Vec<Vec<PointPt>> = Vec::new();
    let mut current_run: Vec<PointPt> = Vec::new();
    let mut summary_markers: Vec<(PointPt, usize, f64, f64)> = Vec::new();
    let mut prev_seg_pair: Option<(usize, usize)> = None;
    let mut warned_x_domain = false;
    let mut warned_y_domain = false;

    for pt in &data.points {
        let shifted_x = pt.x_center + dx_data;
        let Some((sx, px)) = x_axis.map_value(shifted_x) else {
            if !warned_x_domain && is_out_of_axis_domain(x_axis, shifted_x) {
                warned_x_domain = true;
                push_domain_omission_warning(&mut scene.warnings, x_axis, "X", shifted_x);
            }
            flush_run(&mut current_run, &mut mapped_runs);
            prev_seg_pair = None;
            continue;
        };
        let Some((sy, py)) = y_axis.map_value(pt.y_center) else {
            if !warned_y_domain && is_out_of_axis_domain(y_axis, pt.y_center) {
                warned_y_domain = true;
                push_domain_omission_warning(&mut scene.warnings, y_axis, "Y", pt.y_center);
            }
            flush_run(&mut current_run, &mut mapped_runs);
            prev_seg_pair = None;
            continue;
        };

        if let Some(prev) = prev_seg_pair
            && prev != (sx, sy)
        {
            flush_run(&mut current_run, &mut mapped_runs);
        }
        prev_seg_pair = Some((sx, sy));
        let mapped_pt = PointPt::new(px + dx_pt, py + dy_pt);
        current_run.push(mapped_pt);
        summary_markers.push((mapped_pt, pt.row, pt.x_center, pt.y_center));
    }
    flush_run(&mut current_run, &mut mapped_runs);

    // 2b. Vertical bars when series GDO is `BarWithReplicates`
    let mut bars_emitted_points = false;
    if is_bar {
        bars_emitted_points = render_xy_bars(scene, data, &summary_markers, &XyBarRenderCtx { graph_spec, spec, y_axis, dy_pt, clip });
    }

    // 3. Area fill under connecting lines (clipped per discontinuous segment)
    if let Some(af) = area_opt {
        let (_, base_py, base_clipped) = y_axis.map_clamped_to_nearest(af.baseline_y);
        if base_clipped && y_axis.segments.iter().any(|s| is_log_like(s.scale)) && af.baseline_y <= 0.0 {
            scene
                .warnings
                .push(ChartWarning::new("log_axis_floor_clip", format!("area fill baseline {} clipped to segment floor on log axis", af.baseline_y)));
        }
        let kind = line_opt.map_or(LineConnectionKind::PointToPoint, |l| l.kind);
        for run in &mapped_runs {
            if let Some(poly) = build_area_polygon(run, kind, base_py + dy_pt) {
                scene.primitives.push(ScenePrimitive::Path {
                    path: poly,
                    fill: Some(FillPaint::solid(af.fill_color)),
                    stroke: None,
                    clip: Some(clip),
                });
            }
        }
    }

    let markers_emit_replicates = marker_opt.is_some() && !data.replicates.is_empty() && spec.y_error_bars.is_none();

    // 4. Connecting lines (either per-replicate longitudinal trajectories or summary-center polyline)
    if is_spaghetti && !data.replicates.is_empty() {
        let line_style = line_opt.unwrap_or(curvebook_doc::ConnectingLineStyle {
            kind: LineConnectionKind::PointToPoint,
            dash_style: LineDashStyle::Solid,
            stroke_width_pt: 1.25,
            color: data.default_color,
        });
        render_replicate_trajectories(
            scene,
            data,
            &ReplicateTrajectoryCtx {
                x_axis,
                y_axis,
                line_style: &line_style,
                offsets: (dx_data, dx_pt, dy_pt),
                emit_meta: !markers_emit_replicates,
                clip,
            },
        );
    } else if let Some(line_style) = line_opt {
        let stroke = stroke_with_dash(line_style.color, line_style.stroke_width_pt, line_style.dash_style);
        for run in &mapped_runs {
            if let Some(path) = build_connecting_path(run, line_style.kind) {
                scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: Some(clip) });
            }
        }
    }

    // 5. Error bars (Y vertical, X horizontal)
    let err_default = match &spec.gdo {
        GraphicalDataObject::BarWithReplicates { border_color, .. } => *border_color,
        _ => marker_opt.map_or(data.default_color, |m| m.stroke_color),
    };
    for pt in &data.points {
        let shifted_x = pt.x_center + dx_data;
        if let (Some(eb), Some((_, px))) = (&spec.y_error_bars, x_axis.map_value(shifted_x)) {
            let start_idx = scene.primitives.len();
            emit_directional_error_bar(
                scene,
                px + dx_pt,
                pt.y_center,
                pt.y_lo,
                pt.y_hi,
                AxisErrorTarget { axis: y_axis, is_vertical: true, spec: eb, default_color: err_default, clip },
            );
            translate_primitives(&mut scene.primitives[start_idx..], 0.0, dy_pt);
        }
        if let (Some(eb), Some((_, py))) = (&spec.x_error_bars, y_axis.map_value(pt.y_center)) {
            let start_idx = scene.primitives.len();
            emit_directional_error_bar(
                scene,
                py + dy_pt,
                shifted_x,
                pt.x_lo + dx_data,
                pt.x_hi + dx_data,
                AxisErrorTarget { axis: x_axis, is_vertical: false, spec: eb, default_color: err_default, clip },
            );
            translate_primitives(&mut scene.primitives[start_idx..], dx_pt, 0.0);
        }
    }

    let first_series_point = scene.plotted_points.len();
    // 6. Markers (either raw individual replicates when y_error_bars is None, or summarized centers)
    if let Some(marker) = marker_opt {
        let ctx = XyMarkerCtx {
            spec: graph_spec,
            x_axis,
            y_axis,
            marker,
            clip,
            use_replicates: !data.replicates.is_empty() && spec.y_error_bars.is_none(),
            dx_data,
            dx_pt,
            dy_pt,
            warned_x_domain,
            warned_y_domain,
        };
        emit_xy_markers(scene, data, &summary_markers, &ctx);
    } else if line_opt.is_some() && !is_spaghetti && !bars_emitted_points {
        for &(pt, row, vx, vy) in &summary_markers {
            push_xy_point_meta(scene, data, pt, 6.0, (row, 0, vx, vy));
        }
    }
    emit_rout_outlier_markers(scene, data, &RoutOutlierCtx { x_axis, y_axis, first_series_point, dx_data, dx_pt, dy_pt });
}

struct XyBarRenderCtx<'a> {
    graph_spec: &'a GraphSpec,
    spec: &'a SeriesSpec,
    y_axis: &'a SegmentedAxis,
    dy_pt: f64,
    clip: RectPt,
}

fn render_xy_bars(
    scene: &mut ChartScene,
    data: &ExtractedXySeries,
    summary_markers: &[(PointPt, usize, f64, f64)],
    ctx: &XyBarRenderCtx<'_>,
) -> bool {
    let &XyBarRenderCtx { graph_spec, spec, y_axis, dy_pt, clip } = ctx;
    let GraphicalDataObject::BarWithReplicates { fill_color, border_color, border_width_pt, replicate_overlay, .. } = spec.gdo else {
        return false;
    };
    let bar_w_pt = if let Some(w) = spec.bar_width_override_pt.filter(|v| v.is_finite() && *v > 0.0) {
        w
    } else {
        let mut xs: Vec<f64> = summary_markers.iter().map(|m| m.0.x).collect();
        xs.sort_by(f64::total_cmp);
        let min_dx = xs.windows(2).map(|w| (w[1] - w[0]).abs()).filter(|d| *d > 1e-3).fold(f64::INFINITY, f64::min);
        if min_dx.is_finite() { crate::clamp_f64_safe(min_dx * 0.65, 4.0, 28.0) } else { 14.0 }
    };

    let base_data = y_axis.segments.first().map_or(0.0, |seg| {
        if is_log_like(seg.scale) {
            seg.domain_min
        } else if seg.domain_min <= 0.0 && seg.domain_max >= 0.0 {
            0.0
        } else {
            seg.domain_min
        }
    });
    let (_, raw_base_py, _) = y_axis.map_clamped_to_nearest(base_data);
    let base_py = raw_base_py + dy_pt;
    let border = (border_width_pt.is_finite() && border_width_pt > 0.0).then(|| StrokePaint::solid(border_color, border_width_pt));

    for &(pt, row, vx, vy) in summary_markers {
        let y0 = base_py.min(pt.y);
        let y1 = base_py.max(pt.y);
        let bar_rect = RectPt::new(pt.x - 0.5 * bar_w_pt, y0, bar_w_pt, (y1 - y0).max(0.5));
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(bar_rect),
            fill: Some(FillPaint::solid(fill_color)),
            stroke: border,
            clip: Some(clip),
        });
        push_xy_point_meta(scene, data, pt, bar_w_pt.min(8.0), (row, 0, vx, vy));
    }

    if replicate_overlay.is_some() && !data.replicates.is_empty() {
        let dot_style = MarkerStyle {
            shape: MarkerShape::Circle,
            fill_mode: MarkerFillMode::Filled,
            size_pt: 3.4,
            fill_color: border_color,
            stroke_color: Rgba8::WHITE,
            stroke_width_pt: 0.4,
        };
        let x_ax_pt = |rx: f64, ry: f64| -> Option<PointPt> {
            let (_, py) = y_axis.map_value(ry)?;
            let mapped = summary_markers.iter().find(|m| (m.2 - rx).abs() <= 1e-12).map(|m| m.0.x);
            mapped.map(|px| PointPt::new(px, py + dy_pt))
        };
        for &(rx, ry, row, subcol) in &data.replicates {
            let Some(marker) = crate::resolve_overridden_marker_for_cell(graph_spec, data.series_index, row, Some(subcol), dot_style) else {
                continue;
            };
            let pt = if let Some(p) = x_ax_pt(rx, ry) {
                p
            } else {
                continue;
            };
            emit_marker(&mut scene.primitives, pt, &marker, Some(clip));
            push_xy_point_meta(scene, data, pt, marker.size_pt, (row, subcol, rx, ry));
        }
    }
    true
}

struct ReplicateTrajectoryCtx<'a> {
    x_axis: &'a SegmentedAxis,
    y_axis: &'a SegmentedAxis,
    line_style: &'a curvebook_doc::ConnectingLineStyle,
    offsets: (f64, f64, f64),
    emit_meta: bool,
    clip: RectPt,
}

fn render_replicate_trajectories(scene: &mut ChartScene, data: &ExtractedXySeries, ctx: &ReplicateTrajectoryCtx<'_>) {
    let &ReplicateTrajectoryCtx { x_axis, y_axis, line_style, offsets, emit_meta, clip } = ctx;
    let (dx_data, dx_pt, dy_pt) = offsets;
    let mut subcols: Vec<usize> = data.replicates.iter().map(|&(_, _, _, sc)| sc).collect();
    subcols.sort_unstable();
    subcols.dedup();
    if subcols.is_empty() {
        return;
    }

    let mut cell_lookup: std::collections::BTreeMap<(usize, usize), (f64, f64)> = std::collections::BTreeMap::new();
    for &(rx, ry, row, subcol) in &data.replicates {
        cell_lookup.insert((row, subcol), (rx, ry));
    }

    let mut ordered_rows = data.active_x_rows.clone();
    ordered_rows.sort_by(|a, b| (a.1 + dx_data).total_cmp(&(b.1 + dx_data)).then_with(|| a.0.cmp(&b.0)));

    let stroke = stroke_with_dash(line_style.color, line_style.stroke_width_pt.max(0.75), line_style.dash_style);
    for sc in subcols {
        let mut runs: Vec<Vec<PointPt>> = Vec::new();
        let mut current: Vec<PointPt> = Vec::new();
        let mut prev_seg_pair: Option<(usize, usize)> = None;
        for &(row, _) in &ordered_rows {
            let Some(&(rx, ry)) = cell_lookup.get(&(row, sc)) else {
                flush_run(&mut current, &mut runs);
                prev_seg_pair = None;
                continue;
            };
            let shifted_x = rx + dx_data;
            let (Some((sx, px)), Some((sy, py))) = (x_axis.map_value(shifted_x), y_axis.map_value(ry)) else {
                flush_run(&mut current, &mut runs);
                prev_seg_pair = None;
                continue;
            };
            if let Some(prev) = prev_seg_pair
                && prev != (sx, sy)
            {
                flush_run(&mut current, &mut runs);
            }
            prev_seg_pair = Some((sx, sy));
            let pt = PointPt::new(px + dx_pt, py + dy_pt);
            current.push(pt);
            if emit_meta {
                push_xy_point_meta(scene, data, pt, 6.0, (row, sc, rx, ry));
            }
        }
        flush_run(&mut current, &mut runs);
        for run in &runs {
            if let Some(path) = build_connecting_path(run, line_style.kind) {
                scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: Some(clip) });
            }
        }
    }
}

fn translate_primitives(prims: &mut [ScenePrimitive], dx: f64, dy: f64) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    let shift = |p: &mut PointPt| {
        p.x += dx;
        p.y += dy;
    };
    for prim in prims {
        match prim {
            ScenePrimitive::Path { path, .. } => {
                for cmd in &mut path.commands {
                    match cmd {
                        PathCmd::MoveTo(p) | PathCmd::LineTo(p) => shift(p),
                        PathCmd::QuadTo { control, to } => {
                            shift(control);
                            shift(to);
                        }
                        PathCmd::CubicTo { c1, c2, to } => {
                            shift(c1);
                            shift(c2);
                            shift(to);
                        }
                        PathCmd::Close => {}
                    }
                }
            }
            ScenePrimitive::Text(t) => shift(&mut t.anchor_pt),
            ScenePrimitive::RasterImage(img) => {
                img.bounds_pt.x += dx;
                img.bounds_pt.y += dy;
                if let Some(c) = &mut img.clip {
                    c.x += dx;
                    c.y += dy;
                }
            }
            ScenePrimitive::PushClip { rect } => {
                rect.x += dx;
                rect.y += dy;
            }
            ScenePrimitive::PopClip => {}
        }
    }
}

struct RoutOutlierCtx<'a> {
    x_axis: &'a SegmentedAxis,
    y_axis: &'a SegmentedAxis,
    first_series_point: usize,
    dx_data: f64,
    dx_pt: f64,
    dy_pt: f64,
}

fn emit_rout_outlier_markers(scene: &mut ChartScene, data: &ExtractedXySeries, ctx: &RoutOutlierCtx<'_>) {
    let &RoutOutlierCtx { x_axis, y_axis, first_series_point, dx_data, dx_pt, dy_pt } = ctx;
    let marker = MarkerStyle { shape: MarkerShape::Cross, size_pt: 10.0, stroke_color: Rgba8::BLACK, stroke_width_pt: 1.5, ..MarkerStyle::default() };
    let first_outlier = scene.plotted_points.len();
    for point in &data.rout_outliers {
        let (Some((_, px)), Some((_, py))) = (x_axis.map_value(point.x + dx_data), y_axis.map_value(point.y)) else {
            continue;
        };
        let center_pt = PointPt::new(px + dx_pt, py + dy_pt);
        emit_marker(&mut scene.primitives, center_pt, &marker, Some(scene.plot_rect_pt));
        scene.plotted_points.push(PlottedPointMeta {
            center_pt,
            radius_pt: marker.size_pt * 0.5,
            source_cell: point.source.clone(),
            series_label: data.label.clone(),
            value_summary: format!("ROUT outlier — X={}, Y={}", format_point_value(point.x), format_point_value(point.y)),
        });
    }
    // Outlier badges take hover precedence over coincident markers in the same series.
    if let Some(series_points) = scene.plotted_points.get_mut(first_series_point..) {
        series_points.rotate_left(first_outlier - first_series_point);
    }
}

fn series_z_size(gdo: &GraphicalDataObject) -> (f64, f64) {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            (marker.as_ref().map_or(0.0, |m| m.size_pt), line.as_ref().map_or(0.0, |l| l.stroke_width_pt))
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => (marker.size_pt, 0.0),
        GraphicalDataObject::BeforeAfterLines { marker, stroke_width_pt, .. } => (marker.as_ref().map_or(0.0, |m| m.size_pt), *stroke_width_pt),
        _ => (0.0, 0.0),
    }
}

struct XyMarkerCtx<'a> {
    spec: &'a GraphSpec,
    x_axis: &'a SegmentedAxis,
    y_axis: &'a SegmentedAxis,
    marker: MarkerStyle,
    clip: RectPt,
    use_replicates: bool,
    dx_data: f64,
    dx_pt: f64,
    dy_pt: f64,
    warned_x_domain: bool,
    warned_y_domain: bool,
}

fn push_xy_point_meta(
    scene: &mut ChartScene,
    data: &ExtractedXySeries,
    center_pt: PointPt,
    marker_size_pt: f64,
    cell_info: (usize, usize, f64, f64),
) {
    let (row, subcol, vx, vy) = cell_info;
    scene.plotted_points.push(PlottedPointMeta {
        center_pt,
        radius_pt: (marker_size_pt * 0.5).max(3.0),
        source_cell: SourceCellId { sheet_id: data.sheet_id.clone(), row, col: data.col_index, subcol },
        series_label: data.label.clone(),
        value_summary: format!("X={}, Y={}", format_point_value(vx), format_point_value(vy)),
    });
}

fn emit_xy_markers(scene: &mut ChartScene, data: &ExtractedXySeries, summary_markers: &[(PointPt, usize, f64, f64)], ctx: &XyMarkerCtx<'_>) {
    let mut warned_x = ctx.warned_x_domain;
    let mut warned_y = ctx.warned_y_domain;
    if ctx.use_replicates {
        for &(rx, ry, row, subcol) in &data.replicates {
            let Some(marker) = crate::resolve_overridden_marker_for_cell(ctx.spec, data.series_index, row, Some(subcol), ctx.marker) else {
                continue;
            };
            let shifted_x = rx + ctx.dx_data;
            let Some((_, px)) = ctx.x_axis.map_value(shifted_x) else {
                if !warned_x && is_out_of_axis_domain(ctx.x_axis, shifted_x) {
                    warned_x = true;
                    push_domain_omission_warning(&mut scene.warnings, ctx.x_axis, "X", shifted_x);
                }
                continue;
            };
            let Some((_, py)) = ctx.y_axis.map_value(ry) else {
                if !warned_y && is_out_of_axis_domain(ctx.y_axis, ry) {
                    warned_y = true;
                    push_domain_omission_warning(&mut scene.warnings, ctx.y_axis, "Y", ry);
                }
                continue;
            };
            let pt = PointPt::new(px + ctx.dx_pt, py + ctx.dy_pt);
            emit_marker(&mut scene.primitives, pt, &marker, Some(ctx.clip));
            push_xy_point_meta(scene, data, pt, marker.size_pt, (row, subcol, rx, ry));
        }
    } else {
        for &(pt, row, vx, vy) in summary_markers {
            let Some(marker) = crate::resolve_overridden_marker_for_cell(ctx.spec, data.series_index, row, Some(0), ctx.marker) else {
                continue;
            };
            emit_marker(&mut scene.primitives, pt, &marker, Some(ctx.clip));
            push_xy_point_meta(scene, data, pt, marker.size_pt, (row, 0, vx, vy));
        }
    }
}

fn is_out_of_axis_domain(axis: &SegmentedAxis, val: f64) -> bool {
    axis.segments.iter().all(|s| scale_transform(s.scale, val).is_none())
}

fn push_domain_omission_warning(warnings: &mut Vec<ChartWarning>, axis: &SegmentedAxis, axis_label: &str, val: f64) {
    let scale = axis.segments.first().map_or(AxisScale::Linear, |s| s.scale);
    let code = match scale {
        AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => "non_positive_on_log_axis",
        AxisScale::Probit => "out_of_domain_probit",
        AxisScale::Logit(_) => "out_of_domain_logit",
        AxisScale::Reciprocal => "zero_on_reciprocal_axis",
        AxisScale::Linear => "invalid_axis_coordinate",
    };
    warnings.push(ChartWarning::new(code, format!("omitted {axis_label} coordinate {val} outside {scale:?} axis domain")));
}

fn flush_run(current: &mut Vec<PointPt>, runs: &mut Vec<Vec<PointPt>>) {
    if !current.is_empty() {
        runs.push(std::mem::take(current));
    }
}

pub(super) fn build_connecting_path(run: &[PointPt], kind: LineConnectionKind) -> Option<Path2D> {
    if run.len() < 2 {
        return None;
    }
    let mut commands = Vec::with_capacity(run.len() * 2);
    match kind {
        LineConnectionKind::PointToPoint => {
            commands.push(PathCmd::MoveTo(run[0]));
            for &p in &run[1..] {
                commands.push(PathCmd::LineTo(p));
            }
        }
        LineConnectionKind::Staircase => {
            commands.push(PathCmd::MoveTo(run[0]));
            for win in run.windows(2) {
                commands.push(PathCmd::LineTo(PointPt::new(win[1].x, win[0].y)));
                commands.push(PathCmd::LineTo(win[1]));
            }
        }
        LineConnectionKind::MonotonicCubicSpline => {
            append_hermite_path(&mut commands, run, false);
        }
        LineConnectionKind::AkimaSpline => {
            append_hermite_path(&mut commands, run, true);
        }
    }
    Some(Path2D { commands })
}

fn path_start_and_end(path: &Path2D) -> Option<(PointPt, PointPt)> {
    let first = match path.commands.first()? {
        PathCmd::MoveTo(p) | PathCmd::LineTo(p) | PathCmd::QuadTo { to: p, .. } | PathCmd::CubicTo { to: p, .. } => *p,
        PathCmd::Close => return None,
    };
    let last = path.commands.iter().rev().find_map(|cmd| match cmd {
        PathCmd::MoveTo(p) | PathCmd::LineTo(p) | PathCmd::QuadTo { to: p, .. } | PathCmd::CubicTo { to: p, .. } => Some(*p),
        PathCmd::Close => None,
    })?;
    Some((first, last))
}

fn build_area_polygon(run: &[PointPt], kind: LineConnectionKind, baseline_py: f64) -> Option<Path2D> {
    let mut top_path = build_connecting_path(run, kind)?;
    let (first, last) = path_start_and_end(&top_path)?;
    top_path.commands.push(PathCmd::LineTo(PointPt::new(last.x, baseline_py)));
    top_path.commands.push(PathCmd::LineTo(PointPt::new(first.x, baseline_py)));
    top_path.commands.push(PathCmd::Close);
    Some(top_path)
}

/// Append a `MoveTo` + cubic Hermite spline segments (Fritsch & Carlson, *SIAM J. Numer. Anal.*
/// 17(2):238–246, 1980; Akima, *J. ACM* 17(4):589–602, 1970).
///
/// Supports strictly increasing scene `x`, strictly decreasing scene `x` (reversed X axes), and
/// unsorted runs with distinct `x` coordinates. Falls back deterministically to straight `LineTo`
/// segments when `n == 2` or when any two points have tied scene `x` (`|dx| <= 1e-9`).
fn append_hermite_path(commands: &mut Vec<PathCmd>, pts: &[PointPt], use_akima: bool) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    if n == 2 {
        commands.push(PathCmd::MoveTo(pts[0]));
        commands.push(PathCmd::LineTo(pts[1]));
        return;
    }

    if pts.windows(2).all(|w| w[1].x > w[0].x + 1e-9) {
        let tangents = compute_spline_slopes_increasing(pts, use_akima);
        emit_cubic_hermite_run(commands, pts, &tangents);
        return;
    }

    if pts.windows(2).all(|w| w[1].x < w[0].x - 1e-9) {
        let rev_pts: Vec<PointPt> = pts.iter().copied().rev().collect();
        let mut tangents = compute_spline_slopes_increasing(&rev_pts, use_akima);
        tangents.reverse();
        emit_cubic_hermite_run(commands, pts, &tangents);
        return;
    }

    let mut sorted = pts.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x));
    if sorted.windows(2).all(|w| w[1].x > w[0].x + 1e-9) {
        let tangents = compute_spline_slopes_increasing(&sorted, use_akima);
        emit_cubic_hermite_run(commands, &sorted, &tangents);
    } else {
        commands.push(PathCmd::MoveTo(pts[0]));
        for &p in &pts[1..] {
            commands.push(PathCmd::LineTo(p));
        }
    }
}

fn compute_spline_slopes_increasing(pts: &[PointPt], use_akima: bool) -> Vec<f64> {
    if use_akima { akima_slopes(pts) } else { fritsch_carlson_slopes(pts) }
}

fn emit_cubic_hermite_run(commands: &mut Vec<PathCmd>, pts: &[PointPt], tangents: &[f64]) {
    let Some(&first) = pts.first() else {
        return;
    };
    commands.push(PathCmd::MoveTo(first));
    for i in 0..(pts.len().saturating_sub(1)) {
        let (Some(&p0), Some(&p1), Some(&m0), Some(&m1)) = (pts.get(i), pts.get(i + 1), tangents.get(i), tangents.get(i + 1)) else {
            continue;
        };
        let dx = p1.x - p0.x;
        let c1 = PointPt::new(p0.x + dx / 3.0, p0.y + m0 * dx / 3.0);
        let c2 = PointPt::new(p1.x - dx / 3.0, p1.y - m1 * dx / 3.0);
        commands.push(PathCmd::CubicTo { c1, c2, to: p1 });
    }
}

fn fritsch_carlson_slopes(pts: &[PointPt]) -> Vec<f64> {
    let n = pts.len();
    let mut delta = Vec::with_capacity(n - 1);
    for i in 0..(n - 1) {
        let dx = (pts[i + 1].x - pts[i].x).max(1e-9);
        delta.push((pts[i + 1].y - pts[i].y) / dx);
    }
    let mut m = vec![0.0; n];
    m[0] = delta[0];
    m[n - 1] = delta[n - 2];
    for i in 1..(n - 1) {
        if delta[i - 1] * delta[i] <= 0.0 {
            m[i] = 0.0;
        } else {
            let h0 = pts[i].x - pts[i - 1].x;
            let h1 = pts[i + 1].x - pts[i].x;
            let w1 = 2.0 * h1 + h0;
            let w2 = h1 + 2.0 * h0;
            m[i] = (w1 + w2) / (w1 / delta[i - 1] + w2 / delta[i]);
        }
    }
    // Enforce Fritsch-Carlson overshoot cap
    for (i, &d) in delta.iter().enumerate() {
        if d.abs() < 1e-12 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
        } else {
            let alpha = m[i] / d;
            let beta = m[i + 1] / d;
            let sq = alpha * alpha + beta * beta;
            if sq > 9.0 {
                let tau = 3.0 / sq.sqrt();
                m[i] = tau * alpha * d;
                m[i + 1] = tau * beta * d;
            }
        }
    }
    m
}

fn akima_slopes(pts: &[PointPt]) -> Vec<f64> {
    let n = pts.len();
    let m_len = n - 1;
    let mut s = Vec::with_capacity(m_len + 4);
    s.push(0.0);
    s.push(0.0);
    for i in 0..m_len {
        let dx = (pts[i + 1].x - pts[i].x).max(1e-9);
        s.push((pts[i + 1].y - pts[i].y) / dx);
    }
    s.push(0.0);
    s.push(0.0);
    // Boundary chord extensions per Akima (1970)
    s[1] = 2.0 * s[2] - s[3.min(m_len + 1)];
    s[0] = 2.0 * s[1] - s[2];
    s[m_len + 2] = 2.0 * s[m_len + 1] - s[m_len.max(2)];
    s[m_len + 3] = 2.0 * s[m_len + 2] - s[m_len + 1];

    let mut out = vec![0.0; n];
    for (i, slot) in out.iter_mut().enumerate() {
        let idx = i + 2;
        let w1 = (s[idx + 1] - s[idx]).abs();
        let w2 = (s[idx - 1] - s[idx - 2]).abs();
        let denom = w1 + w2;
        *slot = if denom > 1e-12 { (w1 * s[idx - 1] + w2 * s[idx]) / denom } else { 0.5 * (s[idx - 1] + s[idx]) };
    }
    out
}
