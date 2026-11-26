//! Column and grouped per-slot graphical item builders: bars, replicate beeswarm/jitter dots,
//! box-and-whisker, violin KDE profiles, floating bars, and paired before/after connectors.

use std::collections::BTreeMap;

use curvebook_doc::{
    ErrorBarDirection, GraphSpec, GraphicalDataObject, ReplicateOverlayStyle, SubcolumnMode, SummaryMeasure, TableArchetype, WhiskerMethod,
};
use curvebook_math::sum::SummaryMoments;
use curvebook_palette::{LineDashStyle, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::common::{quantile_sorted, quartiles_hinge, sorted_finite};

use super::horizontal::HorizontalSlotBound;
use super::{CategorySlotItem, ConnectorAxes, ExtractedKind, RawObs, RenderedSlotBound, SlotGeom, StackAccum, raw_values, sorted_finite_obs};
use crate::markers::{emit_marker, polygon_path};
use crate::scene::{
    ChartScene, ChartWarning, FillPaint, Path2D, PathCmd, PlottedPointMeta, PointPt, RectPt, ScenePrimitive, SourceCellId, StrokePaint,
};
use crate::xy::{
    AxisErrorTarget, SegmentedAxis, compute_central_tendency, compute_raw_error_bounds, emit_directional_error_bar, format_point_value, is_log_like,
    stroke_with_dash,
};

const KDE_SAMPLES: usize = 65;

pub(super) enum BeforeAfterBounds<'a> {
    Vertical(&'a mut Vec<RenderedSlotBound>),
    Horizontal(&'a mut Vec<HorizontalSlotBound>),
}

pub(super) fn render_before_after_connectors(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    sheet: &curvebook_doc::DataSheet,
    items: &[CategorySlotItem],
    axes: ConnectorAxes<'_>,
    bounds: &mut Vec<RenderedSlotBound>,
) {
    render_before_after_connectors_oriented(scene, graph_spec, sheet, items, axes, BeforeAfterBounds::Vertical(bounds));
}

pub(super) fn render_before_after_connectors_oriented(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    sheet: &curvebook_doc::DataSheet,
    items: &[CategorySlotItem],
    axes: ConnectorAxes<'_>,
    mut out_bounds: BeforeAfterBounds<'_>,
) {
    let is_horizontal = matches!(out_bounds, BeforeAfterBounds::Horizontal(_));
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
        slot_idx: usize,
        col_index: usize,
        row: usize,
        subcol: usize,
        item: &'a CategorySlotItem,
        val: f64,
    }

    let mut emit_trajectory = |traj: &[BaObsPoint<'_>]| {
        let mut pts: Vec<(usize, usize, usize, &CategorySlotItem, PointPt, f64)> = Vec::with_capacity(traj.len());
        for obs in traj {
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
                if let Some(ext) = ext_per_slot.get_mut(obs.slot_idx) {
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
            for row in 0..col_table.row_count() {
                let mut traj = Vec::new();
                for &(slot_idx, item) in &ba_cols {
                    if let Some(val) =
                        col_table.data_sets.get(item.col_index).and_then(|d| d.cell(row, 0)).and_then(curvebook_doc::Cell::active_number)
                    {
                        traj.push(BaObsPoint { slot_idx, col_index: item.col_index, row, subcol: 0, item, val });
                    }
                }
                emit_trajectory(&traj);
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
                    if grp_table.subcolumn_mode.is_pre_summarized() {
                        let traj: Vec<BaObsPoint<'_>> = row_slots
                            .iter()
                            .filter_map(|&(slot_idx, item)| match &item.payload {
                                ExtractedKind::PreSummarized { center, .. } if center.is_finite() => {
                                    Some(BaObsPoint { slot_idx, col_index: item.col_index, row, subcol: 0, item, val: *center })
                                }
                                _ => None,
                            })
                            .collect();
                        emit_trajectory(&traj);
                    } else {
                        let max_sub = row_slots
                            .iter()
                            .map(|(_, it)| grp_table.data_sets.get(it.col_index).and_then(|d| d.cells.get(row)).map_or(0, Vec::len))
                            .max()
                            .unwrap_or(0);
                        for subcol in 0..max_sub {
                            let mut traj = Vec::new();
                            for &(slot_idx, item) in &row_slots {
                                if let Some(val) = grp_table
                                    .data_sets
                                    .get(item.col_index)
                                    .and_then(|d| d.cell(row, subcol))
                                    .and_then(curvebook_doc::Cell::active_number)
                                {
                                    traj.push(BaObsPoint { slot_idx, col_index: item.col_index, row, subcol, item, val });
                                }
                            }
                            emit_trajectory(&traj);
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
                            traj.push(BaObsPoint { slot_idx, col_index: item.col_index, row, subcol, item, val });
                        }
                    }
                    emit_trajectory(&traj);
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
            match &mut out_bounds {
                BeforeAfterBounds::Horizontal(horiz_bounds) => {
                    horiz_bounds.push(HorizontalSlotBound {
                        group_key: item.group_key,
                        label: item.match_label.clone(),
                        alt_label: item.alt_label.clone(),
                        y_pt: cat_pt,
                        right_x_pt: ext,
                    });
                }
                BeforeAfterBounds::Vertical(bounds) => {
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
}

pub(super) fn render_single_item_oriented(
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

struct PresumIntervalCfg<'a> {
    spec: &'a GraphSpec,
    override_series_index: Option<usize>,
    slot: SlotGeom,
    val_axis: &'a SegmentedAxis,
    series_label: &'a str,
    is_horizontal: bool,
    fill_color: Rgba8,
    stroke_color: Rgba8,
}

/// Render an honest summary marker + error interval for pre-summarized data whenever a raw
/// distribution plot (KDE violin or non-IQR box-and-whisker) cannot be legitimately synthesized
/// from summary columns (`GAP-02-09`).
fn render_presummarized_summary_interval(
    scene: &mut ChartScene,
    center: f64,
    lower: f64,
    upper: f64,
    source: &SourceCellId,
    cfg: &PresumIntervalCfg<'_>,
) -> Option<f64> {
    if !center.is_finite() {
        return None;
    }
    let clip = scene.plot_rect_pt;
    let cx = cfg.slot.cx;
    let is_horizontal = cfg.is_horizontal;
    let lo = if lower.is_finite() && upper.is_finite() { lower.min(upper) } else { center };
    let hi = if lower.is_finite() && upper.is_finite() { lower.max(upper) } else { center };

    let (_, py_lo, c_lo) = cfg.val_axis.map_clamped_to_nearest(lo);
    let (_, py_center, c_center) = cfg.val_axis.map_clamped_to_nearest(center);
    let (_, py_hi, c_hi) = cfg.val_axis.map_clamped_to_nearest(hi);
    if c_lo || c_center || c_hi {
        emit_clipped_domain_warning(&mut scene.warnings, cfg.val_axis, &[lo, center, hi]);
    }

    let stroke = StrokePaint::solid(cfg.stroke_color, 1.2);
    let cap_half = (cfg.slot.width_pt * 0.22).max(2.5);
    if (py_hi - py_lo).abs() > 0.25 {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(oriented_pt(cx, is_horizontal, 0.0, py_lo), oriented_pt(cx, is_horizontal, 0.0, py_hi)),
            fill: None,
            stroke: Some(stroke),
            clip: Some(clip),
        });
        for end_pv in [py_lo, py_hi] {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(oriented_pt(cx, is_horizontal, -cap_half, end_pv), oriented_pt(cx, is_horizontal, cap_half, end_pv)),
                fill: None,
                stroke: Some(stroke),
                clip: Some(clip),
            });
        }
    }

    let base_marker = MarkerStyle {
        shape: MarkerShape::Circle,
        fill_mode: curvebook_palette::MarkerFillMode::Filled,
        size_pt: 5.5,
        fill_color: cfg.fill_color,
        stroke_color: cfg.stroke_color,
        stroke_width_pt: 0.85,
    };
    let resolved = if let Some(series_index) = cfg.override_series_index {
        crate::resolve_overridden_marker_for_cell(cfg.spec, series_index, source.row, Some(source.subcol), base_marker)
    } else {
        Some(base_marker)
    };
    if let Some(marker) = resolved {
        let pt = oriented_pt(cx, is_horizontal, 0.0, py_center);
        emit_marker(&mut scene.primitives, pt, &marker, Some(clip));
        let obs = RawObs { value: center, source: source.clone() };
        push_col_point_meta(scene, pt, marker.size_pt, &obs, cfg.series_label);
    }

    Some(if is_horizontal { py_hi.max(py_center) } else { py_hi.min(py_center) })
}

/// Render an honest `[lower, upper]` filled range bar with a center rule for non-parametric
/// pre-summarized range columns (`MedianIqr` quartiles or explicit `UpperLowerLimits`).
fn render_presummarized_range_box(scene: &mut ChartScene, lower: f64, center: f64, upper: f64, cfg: &PresumIntervalCfg<'_>) -> Option<f64> {
    if !lower.is_finite() || !center.is_finite() || !upper.is_finite() {
        return None;
    }
    let clip = scene.plot_rect_pt;
    let cx = cfg.slot.cx;
    let box_w = cfg.slot.width_pt * 0.72;
    let is_horizontal = cfg.is_horizontal;
    let (lo, hi) = (lower.min(upper), lower.max(upper));
    let (_, py_lo, c_lo) = cfg.val_axis.map_clamped_to_nearest(lo);
    let (_, py_center, c_center) = cfg.val_axis.map_clamped_to_nearest(center);
    let (_, py_hi, c_hi) = cfg.val_axis.map_clamped_to_nearest(hi);
    if c_lo || c_center || c_hi {
        emit_clipped_domain_warning(&mut scene.warnings, cfg.val_axis, &[lo, center, hi]);
    }
    let stroke = StrokePaint::solid(cfg.stroke_color, 1.1);
    let box_rect = oriented_span_rect(cx, is_horizontal, 0.5 * box_w, py_lo, py_hi);
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(box_rect),
        fill: Some(FillPaint::solid(cfg.fill_color)),
        stroke: Some(stroke),
        clip: Some(clip),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(oriented_pt(cx, is_horizontal, -0.5 * box_w, py_center), oriented_pt(cx, is_horizontal, 0.5 * box_w, py_center)),
        fill: None,
        stroke: Some(StrokePaint::solid(cfg.stroke_color, 1.75)),
        clip: Some(clip),
    });
    Some(if is_horizontal { box_rect.right() } else { box_rect.y })
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
    match &item.payload {
        ExtractedKind::Raw(obs) => {
            let sorted_obs = sorted_finite_obs(obs);
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
        ExtractedKind::PreSummarized { mode, center, lower, upper, source } => {
            let cfg = PresumIntervalCfg {
                spec: graph_spec,
                override_series_index: item.override_series_index,
                slot,
                val_axis,
                series_label: &item.match_label,
                is_horizontal,
                fill_color,
                stroke_color,
            };
            if *mode == SubcolumnMode::MedianIqr {
                render_presummarized_range_box(scene, *lower, *center, *upper, &cfg)
            } else if *mode == SubcolumnMode::UpperLowerLimits {
                if !scene.warnings.iter().any(|w| w.code == "boxplot_unsupported_pre_summarized") {
                    scene.warnings.push(ChartWarning::new(
                        "boxplot_unsupported_pre_summarized",
                        "box-and-whisker quartiles require raw replicates or Median/IQR columns; rendered explicit lower/upper range bar instead for UpperLowerLimits data",
                    ));
                }
                render_presummarized_range_box(scene, *lower, *center, *upper, &cfg)
                    .or_else(|| render_presummarized_summary_interval(scene, *center, *lower, *upper, source, &cfg))
            } else {
                if !scene.warnings.iter().any(|w| w.code == "boxplot_unsupported_pre_summarized") {
                    scene.warnings.push(ChartWarning::new(
                        "boxplot_unsupported_pre_summarized",
                        format!(
                            "box-and-whisker quartiles require raw replicates or Median/IQR columns; rendered summary point and interval instead for pre-summarized ({mode:?}) data"
                        ),
                    ));
                }
                render_presummarized_summary_interval(scene, *center, *lower, *upper, source, &cfg)
            }
        }
    }
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
    match &item.payload {
        ExtractedKind::Raw(obs) => {
            let sorted_obs = sorted_finite_obs(obs);
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
        ExtractedKind::PreSummarized { mode, center, lower, upper, source } => {
            if !scene.warnings.iter().any(|w| w.code == "violin_unsupported_pre_summarized") {
                scene.warnings.push(ChartWarning::new(
                    "violin_unsupported_pre_summarized",
                    format!(
                        "KDE violin density requires raw replicate observations; rendered summary point and interval instead for pre-summarized ({mode:?}) data"
                    ),
                ));
            }
            let cfg = PresumIntervalCfg {
                spec: graph_spec,
                override_series_index: item.override_series_index,
                slot,
                val_axis,
                series_label: &item.match_label,
                is_horizontal,
                fill_color,
                stroke_color,
            };
            if matches!(*mode, SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr) {
                render_presummarized_range_box(scene, *lower, *center, *upper, &cfg)
                    .or_else(|| render_presummarized_summary_interval(scene, *center, *lower, *upper, source, &cfg))
            } else {
                render_presummarized_summary_interval(scene, *center, *lower, *upper, source, &cfg)
            }
        }
    }
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
