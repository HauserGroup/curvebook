//! Dual-axis Gardner–Altman estimation plot builder for two-group comparisons.
//!
//! Renders individual observations and group means/CIs on a primary left axis alongside
//! an aligned floating difference axis on the right sharing the exact data-unit scale so
//! `0` on the difference axis aligns horizontally with Group A's mean and `Δ = mean(B) - mean(A)`
//! aligns horizontally with Group B's mean. Supports paired slope lines for matched designs.
//!
//! # References
//!
//! - Gardner, M. J., & Altman, D. G. (1986). Confidence intervals rather than *P* values:
//!   estimation rather than hypothesis testing. *BMJ* 292(6522):746–750.
//!   doi:10.1136/bmj.292.6522.746
//! - Ho, J., Tumkaya, T., Aryal, S., Choi, H., & Claridge-Chang, A. (2019). Moving beyond
//!   *P* values: data analysis with estimation graphics. *Nature Methods* 16(7):565–566.
//!   doi:10.1038/s41592-019-0470-3

use curvebook_doc::{AnalysisKind, AxisSpec, ColumnDesignHint, DataSheet, GraphSheet, ProjectDocument, TableArchetype, TwoGroupMethod, VariableRole};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::student_t_two_sided_isf;
use curvebook_math::sum::SummaryMoments;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_text::{LocaleNumberPolicy, RichText, parse_markup};

use super::super::multivariate::{cell_token, column_categorical_levels};
use super::{
    collect_ds_active_values_with_sources, find_matching_result_where_checked, has_blocked_matching_result, resolve_analysis_source_data_sheet,
    resolve_source_data_sheet_checked,
};
use crate::markers::emit_marker;
use crate::scales::{compute_segment_tick_values, format_tick_rich};
use crate::scene::{
    ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive, SourceCellId, StrokePaint,
    TextAnchor,
};
use crate::survival_cat::{
    PlotMarginConfig, SegmentedFrameSpec, draw_empty_open_frame, layout_and_render_segmented_chrome, solid_stroke, text_metrics,
};
use crate::xy::{SegmentedAxis, format_point_value};

struct ObsPoint {
    value: f64,
    source: SourceCellId,
}

struct EstimationGroup {
    label: String,
    obs: Vec<ObsPoint>,
    mean: f64,
    ci_lo: f64,
    ci_hi: f64,
}

struct EstimationData {
    group_a: EstimationGroup,
    group_b: EstimationGroup,
    /// Mean difference `mean(B) - mean(A)` (or paired mean of `y_b - y_a`).
    diff: f64,
    diff_ci_lo: f64,
    diff_ci_hi: f64,
    confidence: ConfidenceLevel,
    paired: bool,
    /// Matched `(y_a, y_b)` pairs when `paired == true`.
    paired_links: Vec<(f64, f64)>,
}

/// Compile a dual-axis Gardner–Altman estimation plot into `scene`.
pub(in crate::multivariate_layout) fn compile_estimation_plot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let Some(data) = extract_estimation_data(graph, doc, &mut scene.warnings) else {
        scene.warnings.push(ChartWarning::new(
            "empty_estimation_plot",
            "insufficient two-group observations (need at least 2 finite values per group) for estimation plot",
        ));
        draw_empty_open_frame(scene);
        return Ok(());
    };

    // Shared vertical data domain must cover all raw points, group CIs, and the shifted difference CI
    // (`mean_a + diff_ci_lo ..= mean_a + diff_ci_hi`) plus `mean_a` itself so the right floating axis
    // sits cleanly inside the same vertical scale.
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;
    let mut include_y = |v: f64| {
        if v.is_finite() {
            y_min = y_min.min(v);
            y_max = y_max.max(v);
        }
    };
    for o in data.group_a.obs.iter().chain(data.group_b.obs.iter()) {
        include_y(o.value);
    }
    for v in [
        data.group_a.mean,
        data.group_a.ci_lo,
        data.group_a.ci_hi,
        data.group_b.mean,
        data.group_b.ci_lo,
        data.group_b.ci_hi,
        data.group_a.mean + data.diff,
        data.group_a.mean + data.diff_ci_lo,
        data.group_a.mean + data.diff_ci_hi,
    ] {
        include_y(v);
    }
    if !y_min.is_finite() || !y_max.is_finite() {
        y_min = 0.0;
        y_max = 1.0;
    }
    let raw_span = y_max - y_min;
    let y_pad = if raw_span > 1e-12 { raw_span * 0.12 } else { y_min.abs().max(1.0) * 0.15 };
    let y_bounds = (y_min - y_pad, y_max + y_pad);

    // Reserve right panel budget for the floating difference axis, effect whisker, tick labels, and title.
    let right_budget_pt = crate::clamp_f64_safe(scene.width_pt * 0.36, 72.0, 165.0);
    let mut frame_graph = graph.clone();
    // Estimation plot has its own right floating axis; suppress standard y2 axis chrome on the left frame.
    frame_graph.spec.y2_axis = None;
    if frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title.clear();
    }
    if frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = String::from("Value");
    }

    let cat_x = vec![(0.0, data.group_a.label.clone()), (1.0, data.group_b.label.clone())];
    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (-0.55, 1.55),
            y_bounds,
            y2_bounds: None,
            cat_x: Some(&cat_x),
            cat_y: None,
            margins: PlotMarginConfig { extra_right_pt: right_budget_pt, ..PlotMarginConfig::default() },
        },
    );

    let left_r = scene.plot_rect_pt;
    let Some((_, xa_pt)) = x_axis.map_value(0.0) else {
        return Ok(());
    };
    let Some((_, xb_pt)) = x_axis.map_value(1.0) else {
        return Ok(());
    };
    let slot_half_w = ((xb_pt - xa_pt).abs() * 0.28).clamp(6.0, 22.0);
    let col_a = graph.spec.palette.color_at(0);
    let col_b = graph.spec.palette.color_at(1);

    // 1. Paired slope lines (behind markers) when paired mode is active.
    if data.paired && !data.paired_links.is_empty() {
        let slope_stroke = solid_stroke(Rgba8::rgba(95, 105, 118, 165), 0.9, LineDashStyle::Solid);
        for &(va, vb) in &data.paired_links {
            if let (Some((_, ya)), Some((_, yb))) = (y_axis.map_value(va), y_axis.map_value(vb)) {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(xa_pt, ya), PointPt::new(xb_pt, yb)),
                    fill: None,
                    stroke: Some(slope_stroke),
                    clip: Some(left_r),
                });
            }
        }
    }

    // 2. Raw observations (aligned at x_a/x_b for paired, deterministic beeswarm for unpaired).
    emit_group_observations(
        scene,
        graph,
        &data.group_a,
        &y_axis,
        GroupStripStyle { cx: xa_pt, max_half_w: slot_half_w, clip: left_r, color: col_a, paired: data.paired },
    );
    emit_group_observations(
        scene,
        graph,
        &data.group_b,
        &y_axis,
        GroupStripStyle { cx: xb_pt, max_half_w: slot_half_w, clip: left_r, color: col_b, paired: data.paired },
    );

    // 3. Per-group mean + (1 - alpha) CI summary overlays.
    let bar_offset = if data.paired {
        (slot_half_w * 0.72).clamp(7.0, 14.0)
    } else if slot_half_w >= 14.0 {
        slot_half_w + 5.0
    } else {
        0.0
    };
    let mean_a_x = if data.paired { (xa_pt - bar_offset).max(left_r.x + 5.0) } else { xa_pt };
    let mean_b_x = if data.paired { (xb_pt + bar_offset).min(left_r.right() - 5.0) } else { xb_pt };
    emit_group_mean_ci(scene, &data.group_a, mean_a_x, &y_axis, left_r, col_a, data.paired);
    emit_group_mean_ci(scene, &data.group_b, mean_b_x, &y_axis, left_r, col_b, data.paired);

    // 4. Right-hand floating difference axis sharing exact `y_axis` scale shifted by `mean_a`.
    let right_avail_x0 = left_r.right() + 10.0;
    let right_avail_x1 = (scene.width_pt - 8.0).max(right_avail_x0 + 24.0);
    let right_w = right_avail_x1 - right_avail_x0;
    let effect_x = right_avail_x0 + (right_w * 0.22).clamp(10.0, 26.0);
    let right_spine_x = (effect_x + (right_w * 0.22).clamp(12.0, 24.0)).min(right_avail_x1 - 22.0);

    // Horizontal dashed alignment lines linking Group A mean -> 0 and Group B mean -> Δ.
    let align_stroke = solid_stroke(Rgba8::rgb(90, 98, 108), 0.9, LineDashStyle::Dashed);
    let full_span_clip = RectPt::new(left_r.x, left_r.y, (right_spine_x - left_r.x).max(1.0) + 2.0, left_r.height);
    if let Some((_, y_zero_pt)) = y_axis.map_value(data.group_a.mean) {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(mean_a_x, y_zero_pt), PointPt::new(right_spine_x, y_zero_pt)),
            fill: None,
            stroke: Some(align_stroke),
            clip: Some(full_span_clip),
        });
    }
    if let Some((_, y_diff_pt)) = y_axis.map_value(data.group_a.mean + data.diff) {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(mean_b_x, y_diff_pt), PointPt::new(right_spine_x, y_diff_pt)),
            fill: None,
            stroke: Some(align_stroke),
            clip: Some(full_span_clip),
        });
    }

    // Floating difference axis spine, ticks, and title.
    render_floating_difference_axis(scene, graph, &data, &y_axis, FloatingAxisLayout { left_r, effect_x, right_spine_x, right_avail_x1 });

    Ok(())
}

struct GroupStripStyle {
    cx: f64,
    max_half_w: f64,
    clip: RectPt,
    color: Rgba8,
    paired: bool,
}

struct FloatingAxisLayout {
    left_r: RectPt,
    effect_x: f64,
    right_spine_x: f64,
    right_avail_x1: f64,
}

fn emit_group_observations(scene: &mut ChartScene, graph: &GraphSheet, group: &EstimationGroup, y_axis: &SegmentedAxis, style: GroupStripStyle) {
    let mut mapped: Vec<(usize, f64, &ObsPoint)> = Vec::with_capacity(group.obs.len());
    for (idx, obs) in group.obs.iter().enumerate() {
        if let Some((_, py)) = y_axis.map_value(obs.value) {
            mapped.push((idx, py, obs));
        }
    }
    let dot_r = 2.4_f64;
    let offsets = if style.paired { vec![0.0_f64; mapped.len()] } else { compute_beeswarm_x_offsets(&mapped, dot_r * 2.15, style.max_half_w) };
    let base_marker = MarkerStyle {
        shape: MarkerShape::Circle,
        fill_mode: MarkerFillMode::Filled,
        size_pt: dot_r * 2.0,
        fill_color: style.color.with_alpha(205),
        stroke_color: Rgba8::WHITE,
        stroke_width_pt: 0.45,
    };
    for ((_, py, obs), dx) in mapped.into_iter().zip(offsets) {
        let Some(marker) = crate::resolve_overridden_marker_for_col(
            &graph.spec,
            &obs.source.sheet_id,
            obs.source.col,
            obs.source.row,
            Some(obs.source.subcol),
            base_marker,
        ) else {
            continue;
        };
        let center_pt = PointPt::new(style.cx + dx, py);
        emit_marker(&mut scene.primitives, center_pt, &marker, Some(style.clip));
        scene.plotted_points.push(PlottedPointMeta {
            center_pt,
            radius_pt: (marker.size_pt * 0.5).max(3.0),
            source_cell: obs.source.clone(),
            series_label: group.label.clone(),
            value_summary: format!("{} = {}", group.label, format_point_value(obs.value)),
        });
    }
}

fn compute_beeswarm_x_offsets(mapped: &[(usize, f64, &ObsPoint)], min_sep_pt: f64, max_half_w: f64) -> Vec<f64> {
    let n = mapped.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| mapped[a].1.total_cmp(&mapped[b].1).then_with(|| a.cmp(&b)));
    let mut offsets = vec![0.0_f64; n];
    let mut placed: Vec<(f64, f64)> = Vec::with_capacity(n);
    let step = (min_sep_pt * 0.65).max(1.5);
    let sep2 = min_sep_pt * min_sep_pt;

    for idx in order {
        let py = mapped[idx].1;
        let mut chosen = 0.0_f64;
        let mut ring = 0_usize;
        loop {
            let candidates: [f64; 2] = if ring == 0 {
                [0.0, 0.0]
            } else {
                let mag = (ring as f64) * step;
                if mag > max_half_w {
                    chosen = if idx % 2 == 0 { max_half_w } else { -max_half_w };
                    break;
                }
                if idx % 2 == 0 { [mag, -mag] } else { [-mag, mag] }
            };
            let count = if ring == 0 { 1 } else { 2 };
            let mut found = false;
            for &cand in &candidates[..count] {
                let collides = placed.iter().any(|&(ox, oy)| {
                    let dx = cand - ox;
                    let dy = py - oy;
                    dx * dx + dy * dy < sep2
                });
                if !collides {
                    chosen = cand;
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
            ring += 1;
            if ring > 32 {
                chosen = crate::clamp_f64_safe(if idx % 2 == 0 { max_half_w } else { -max_half_w }, -max_half_w, max_half_w);
                break;
            }
        }
        offsets[idx] = chosen;
        placed.push((chosen, py));
    }
    offsets
}

fn emit_group_mean_ci(scene: &mut ChartScene, group: &EstimationGroup, cx: f64, y_axis: &SegmentedAxis, clip: RectPt, color: Rgba8, paired: bool) {
    let ci_stroke = StrokePaint::solid(Rgba8::rgb(25, 28, 34), 1.5);
    if let (Some((_, y_lo_pt)), Some((_, y_hi_pt))) = (y_axis.map_value(group.ci_lo), y_axis.map_value(group.ci_hi)) {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(cx, y_lo_pt), PointPt::new(cx, y_hi_pt)),
            fill: None,
            stroke: Some(ci_stroke),
            clip: Some(clip),
        });
        let cap = 3.2;
        for y_end in [y_lo_pt, y_hi_pt] {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(cx - cap, y_end), PointPt::new(cx + cap, y_end)),
                fill: None,
                stroke: Some(ci_stroke),
                clip: Some(clip),
            });
        }
    }
    if let Some((_, y_mean_pt)) = y_axis.map_value(group.mean) {
        if paired {
            let marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 5.2,
                fill_color: color,
                stroke_color: Rgba8::BLACK,
                stroke_width_pt: 0.9,
            };
            emit_marker(&mut scene.primitives, PointPt::new(cx, y_mean_pt), &marker, Some(clip));
        } else {
            let half_bar = 10.0;
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(cx - half_bar, y_mean_pt), PointPt::new(cx + half_bar, y_mean_pt)),
                fill: None,
                stroke: Some(StrokePaint::solid(Rgba8::BLACK, 2.0)),
                clip: Some(clip),
            });
        }
    }
}

fn render_floating_difference_axis(
    scene: &mut ChartScene,
    graph: &GraphSheet,
    data: &EstimationData,
    y_axis: &SegmentedAxis,
    layout: FloatingAxisLayout,
) {
    let FloatingAxisLayout { left_r, effect_x, right_spine_x, right_avail_x1 } = layout;
    let (dom_lo, dom_hi) = y_axis
        .segments
        .first()
        .map_or((data.group_a.mean - 1.0, data.group_a.mean + 1.0), |s| (s.domain_min.min(s.domain_max), s.domain_min.max(s.domain_max)));
    let max_diff_lo = dom_lo - data.group_a.mean;
    let max_diff_hi = dom_hi - data.group_a.mean;

    // Difference span covers 0, Δ, and the full CI of Δ, bounded within the left axis window.
    let mut d_lo = 0.0_f64.min(data.diff).min(data.diff_ci_lo);
    let mut d_hi = 0.0_f64.max(data.diff).max(data.diff_ci_hi);
    let d_span = (d_hi - d_lo).abs();
    let d_pad = if d_span > 1e-12 { d_span * 0.12 } else { (dom_hi - dom_lo).abs().max(1.0) * 0.12 };
    d_lo = (d_lo - d_pad).max(max_diff_lo);
    d_hi = (d_hi + d_pad).min(max_diff_hi);
    if d_hi <= d_lo + 1e-12 {
        d_lo = max_diff_lo;
        d_hi = max_diff_hi;
    }

    // Build a temporary linear axis strictly over `[d_lo, d_hi]` to generate clean nice tick values.
    let diff_axis_spec = AxisSpec::linear("Mean difference");
    let (_, y_d_lo_pt) = y_axis.map_value(data.group_a.mean + d_lo).unwrap_or((0, left_r.bottom()));
    let (_, y_d_hi_pt) = y_axis.map_value(data.group_a.mean + d_hi).unwrap_or((0, left_r.y));
    let diff_axis = SegmentedAxis::build_exact(&diff_axis_spec, y_d_lo_pt, y_d_hi_pt, (d_lo, d_hi));

    let mut tick_vals: Vec<f64> = Vec::new();
    let mut dummy_w = Vec::new();
    if let Some(seg) = diff_axis.segments.first() {
        let (majors, _) = compute_segment_tick_values(seg, None, false, &mut dummy_w);
        tick_vals.extend(majors.into_iter().filter(|&v| v >= max_diff_lo - 1e-9 && v <= max_diff_hi + 1e-9));
    }
    if !tick_vals.iter().any(|&v| v.abs() <= 1e-9 * d_span.max(1.0)) && (max_diff_lo..=max_diff_hi).contains(&0.0) {
        tick_vals.push(0.0);
    }
    tick_vals.sort_by(f64::total_cmp);
    tick_vals.dedup_by(|a, b| (*a - *b).abs() <= 1e-9 * d_span.max(1.0));

    let spine_d_lo = tick_vals.first().copied().unwrap_or(d_lo).min(d_lo).max(max_diff_lo);
    let spine_d_hi = tick_vals.last().copied().unwrap_or(d_hi).max(d_hi).min(max_diff_hi);
    let (_, spine_bot_pt) = y_axis.map_value(data.group_a.mean + spine_d_lo).unwrap_or((0, left_r.bottom()));
    let (_, spine_top_pt) = y_axis.map_value(data.group_a.mean + spine_d_hi).unwrap_or((0, left_r.y));

    let axis_stroke = StrokePaint::solid(Rgba8::BLACK, graph.spec.y_axis.stroke_width_pt.max(0.9));
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(right_spine_x, spine_top_pt), PointPt::new(right_spine_x, spine_bot_pt)),
        fill: None,
        stroke: Some(axis_stroke),
        clip: None,
    });

    // Right-hand ticks & labels mapped via `y_axis(mean_a + tick_val)` to guarantee exact scale parity.
    let policy = LocaleNumberPolicy::default();
    let mut max_lbl_w = 0.0_f64;
    for tv in &tick_vals {
        let clean_v = if tv.abs() < 1e-12 * d_span.max(1.0) { 0.0 } else { *tv };
        if let Some((_, y_pt)) = y_axis.map_value(data.group_a.mean + clean_v) {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(right_spine_x, y_pt), PointPt::new(right_spine_x + 4.0, y_pt)),
                fill: None,
                stroke: Some(axis_stroke),
                clip: None,
            });
            let rich = format_tick_rich(clean_v, curvebook_math::AxisScale::Linear, graph.spec.y_axis.tick_format, &policy);
            max_lbl_w = max_lbl_w.max(text_metrics(&rich.plain_text(), 8.5).width_pt);
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich,
                anchor_pt: PointPt::new(right_spine_x + 6.5, y_pt + 3.0),
                anchor: TextAnchor::Start,
                font_size_pt: 8.5,
                rotation_deg: 0.0,
                color: Rgba8::BLACK,
            }));
        }
    }

    // Right floating axis title (vertical, rotated +90 deg).
    let right_title = graph.spec.y2_axis.as_ref().map(|a| a.title.trim()).filter(|t| !t.is_empty()).map(String::from).unwrap_or_else(|| {
        if data.paired {
            format!("Paired mean difference ({} \u{2212} {})", data.group_b.label, data.group_a.label)
        } else {
            format!("Mean difference ({} \u{2212} {})", data.group_b.label, data.group_a.label)
        }
    });
    let title_x = (right_spine_x + 12.0 + max_lbl_w + 8.0).min(right_avail_x1 - 4.0);
    let title_y = 0.5 * (spine_top_pt + spine_bot_pt);
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup(&right_title),
        anchor_pt: PointPt::new(title_x, title_y),
        anchor: TextAnchor::Middle,
        font_size_pt: 8.5,
        rotation_deg: 90.0,
        color: Rgba8::BLACK,
    }));

    // Effect estimate (Δ) marker and vertical CI whisker on the floating axis.
    let eff_clip = RectPt::new(effect_x - 14.0, left_r.y - 6.0, 28.0, left_r.height + 12.0);
    if let (Some((_, y_ci_lo)), Some((_, y_ci_hi))) =
        (y_axis.map_value(data.group_a.mean + data.diff_ci_lo), y_axis.map_value(data.group_a.mean + data.diff_ci_hi))
    {
        let ci_stroke = StrokePaint::solid(Rgba8::BLACK, 2.0);
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(effect_x, y_ci_lo), PointPt::new(effect_x, y_ci_hi)),
            fill: None,
            stroke: Some(ci_stroke),
            clip: Some(eff_clip),
        });
        let cap = 3.5;
        for y_end in [y_ci_lo, y_ci_hi] {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(effect_x - cap, y_end), PointPt::new(effect_x + cap, y_end)),
                fill: None,
                stroke: Some(ci_stroke),
                clip: Some(eff_clip),
            });
        }
    }
    if let Some((_, y_eff_pt)) = y_axis.map_value(data.group_a.mean + data.diff) {
        let eff_marker = MarkerStyle {
            shape: MarkerShape::Circle,
            fill_mode: MarkerFillMode::Filled,
            size_pt: 6.8,
            fill_color: Rgba8::BLACK,
            stroke_color: Rgba8::WHITE,
            stroke_width_pt: 0.9,
        };
        emit_marker(&mut scene.primitives, PointPt::new(effect_x, y_eff_pt), &eff_marker, Some(eff_clip));
    }

    // Concise effect-size footer under the difference panel aligned with the X-axis ticks.
    let ci_pct = data.confidence.percent().round();
    let delta_lbl = format!("\u{0394} = {} ({ci_pct:.0}% CI)", format_point_value(data.diff));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain(delta_lbl),
        anchor_pt: PointPt::new(effect_x, left_r.bottom() + 13.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 7.5,
        rotation_deg: 0.0,
        color: Rgba8::rgb(45, 50, 58),
    }));
}

fn extract_estimation_data(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<EstimationData> {
    let mut confidence = ConfidenceLevel::DEFAULT_95;
    let mut paired = graph.spec.estimation_paired_lines;
    let mut welch = false;
    let mut preferred_cols: Option<(usize, usize)> = None;
    let mut mv_group_col: Option<usize> = None;
    let mut mv_resp_col: Option<usize> = None;
    let mut source_ds: Option<&DataSheet> = None;

    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, |r| matches!(r.analysis.kind, AnalysisKind::TwoGroupComparison(_)))
        && let AnalysisKind::TwoGroupComparison(tg) = &res.analysis.kind
    {
        confidence = res.analysis.confidence_level;
        match tg.method {
            TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest => {
                paired = true;
            }
            TwoGroupMethod::WelchT | TwoGroupMethod::LognormalWelchT => {
                welch = true;
            }
            _ => {}
        }
        mv_group_col = tg.mv_group_column;
        mv_resp_col = tg.mv_response_column;
        if res.analysis.selected_data_sets.len() >= 2 {
            preferred_cols = Some((res.analysis.selected_data_sets[0], res.analysis.selected_data_sets[1]));
        }
        source_ds = resolve_analysis_source_data_sheet(res, graph, doc);
    } else if has_blocked_matching_result(graph, doc, |r| matches!(r.analysis.kind, AnalysisKind::TwoGroupComparison(_))) {
        return None;
    }

    let ds = source_ds.or_else(|| resolve_source_data_sheet_checked(graph, doc, warnings))?;
    let (label_a, obs_a, label_b, obs_b, paired_links, auto_paired) =
        extract_two_groups_from_sheet(ds, graph, preferred_cols, mv_group_col, mv_resp_col, paired)?;
    let is_paired = paired || auto_paired;

    if is_paired {
        if paired_links.len() < 2 {
            return None;
        }
        let vals_a: Vec<f64> = paired_links.iter().map(|&(a, _)| a).collect();
        let vals_b: Vec<f64> = paired_links.iter().map(|&(_, b)| b).collect();
        let diffs: Vec<f64> = paired_links.iter().map(|&(a, b)| b - a).collect();
        let (mean_a, ci_a_lo, ci_a_hi) = group_mean_ci(&vals_a, confidence)?;
        let (mean_b, ci_b_lo, ci_b_hi) = group_mean_ci(&vals_b, confidence)?;
        let (diff, diff_ci_lo, diff_ci_hi) = group_mean_ci(&diffs, confidence)?;
        Some(EstimationData {
            group_a: EstimationGroup { label: label_a, obs: obs_a, mean: mean_a, ci_lo: ci_a_lo, ci_hi: ci_a_hi },
            group_b: EstimationGroup { label: label_b, obs: obs_b, mean: mean_b, ci_lo: ci_b_lo, ci_hi: ci_b_hi },
            diff,
            diff_ci_lo,
            diff_ci_hi,
            confidence,
            paired: true,
            paired_links,
        })
    } else {
        if obs_a.len() < 2 || obs_b.len() < 2 {
            return None;
        }
        let vals_a: Vec<f64> = obs_a.iter().map(|o| o.value).collect();
        let vals_b: Vec<f64> = obs_b.iter().map(|o| o.value).collect();
        let (mean_a, ci_a_lo, ci_a_hi) = group_mean_ci(&vals_a, confidence)?;
        let (mean_b, ci_b_lo, ci_b_hi) = group_mean_ci(&vals_b, confidence)?;
        let (diff, diff_ci_lo, diff_ci_hi) = unpaired_diff_ci(&vals_a, &vals_b, confidence, welch)?;
        Some(EstimationData {
            group_a: EstimationGroup { label: label_a, obs: obs_a, mean: mean_a, ci_lo: ci_a_lo, ci_hi: ci_a_hi },
            group_b: EstimationGroup { label: label_b, obs: obs_b, mean: mean_b, ci_lo: ci_b_lo, ci_hi: ci_b_hi },
            diff,
            diff_ci_lo,
            diff_ci_hi,
            confidence,
            paired: false,
            paired_links: Vec::new(),
        })
    }
}

type ExtractedTwoGroups = (String, Vec<ObsPoint>, String, Vec<ObsPoint>, Vec<(f64, f64)>, bool);

fn extract_two_groups_from_sheet(
    ds: &DataSheet,
    graph: &GraphSheet,
    preferred_cols: Option<(usize, usize)>,
    mv_group_col: Option<usize>,
    mv_resp_col: Option<usize>,
    force_paired: bool,
) -> Option<ExtractedTwoGroups> {
    match &ds.archetype {
        TableArchetype::Column(col) if col.data_sets.len() >= 2 => {
            let (ca, cb) = preferred_cols
                .filter(|&(a, b)| a < col.data_sets.len() && b < col.data_sets.len() && a != b)
                .or_else(|| {
                    if graph.spec.series.len() >= 2 {
                        let a = graph.spec.series[0].data_set_index;
                        let b = graph.spec.series[1].data_set_index;
                        (a < col.data_sets.len() && b < col.data_sets.len() && a != b).then_some((a, b))
                    } else {
                        None
                    }
                })
                .unwrap_or((0, 1));
            let auto_paired = matches!(col.design_hint, ColumnDesignHint::PairedRepeatedMeasures);
            let is_paired = force_paired || auto_paired;
            let label_a = col.data_sets[ca].title.clone();
            let label_b = col.data_sets[cb].title.clone();
            if is_paired {
                let mut obs_a = Vec::new();
                let mut obs_b = Vec::new();
                let mut links = Vec::new();
                for r in 0..col.row_count() {
                    if let (Some(va), Some(vb)) = (
                        col.data_sets[ca].cells.get(r).and_then(|s| s.first()).and_then(|c| c.active_number()).filter(|v| v.is_finite()),
                        col.data_sets[cb].cells.get(r).and_then(|s| s.first()).and_then(|c| c.active_number()).filter(|v| v.is_finite()),
                    ) {
                        obs_a.push(ObsPoint { value: va, source: SourceCellId { sheet_id: ds.id.clone(), row: r, col: ca, subcol: 0 } });
                        obs_b.push(ObsPoint { value: vb, source: SourceCellId { sheet_id: ds.id.clone(), row: r, col: cb, subcol: 0 } });
                        links.push((va, vb));
                    }
                }
                Some((label_a, obs_a, label_b, obs_b, links, auto_paired))
            } else {
                let (raw_a, la) = collect_ds_active_values_with_sources(ds, ca);
                let (raw_b, lb) = collect_ds_active_values_with_sources(ds, cb);
                let obs_a = raw_a.into_iter().map(|(value, source)| ObsPoint { value, source }).collect();
                let obs_b = raw_b.into_iter().map(|(value, source)| ObsPoint { value, source }).collect();
                Some((la, obs_a, lb, obs_b, Vec::new(), false))
            }
        }
        TableArchetype::Grouped(g) if g.data_sets.len() >= 2 => {
            let (ca, cb) = preferred_cols.filter(|&(a, b)| a < g.data_sets.len() && b < g.data_sets.len() && a != b).unwrap_or((0, 1));
            if force_paired {
                let label_a = g.data_sets[ca].title.clone();
                let label_b = g.data_sets[cb].title.clone();
                let mut obs_a = Vec::new();
                let mut obs_b = Vec::new();
                let mut links = Vec::new();
                let pre_sum = g.subcolumn_mode.is_pre_summarized();
                for r in 0..g.row_count() {
                    let (Some(row_a), Some(row_b)) = (g.data_sets[ca].cells.get(r), g.data_sets[cb].cells.get(r)) else {
                        continue;
                    };
                    let sub_count = if pre_sum { 1.min(row_a.len()).min(row_b.len()) } else { row_a.len().min(row_b.len()) };
                    for sub in 0..sub_count {
                        if let (Some(va), Some(vb)) = (
                            row_a.get(sub).and_then(|c| c.active_number()).filter(|v| v.is_finite()),
                            row_b.get(sub).and_then(|c| c.active_number()).filter(|v| v.is_finite()),
                        ) {
                            obs_a.push(ObsPoint { value: va, source: SourceCellId { sheet_id: ds.id.clone(), row: r, col: ca, subcol: sub } });
                            obs_b.push(ObsPoint { value: vb, source: SourceCellId { sheet_id: ds.id.clone(), row: r, col: cb, subcol: sub } });
                            links.push((va, vb));
                        }
                    }
                }
                Some((label_a, obs_a, label_b, obs_b, links, false))
            } else {
                let (raw_a, la) = collect_ds_active_values_with_sources(ds, ca);
                let (raw_b, lb) = collect_ds_active_values_with_sources(ds, cb);
                let obs_a: Vec<ObsPoint> = raw_a.into_iter().map(|(value, source)| ObsPoint { value, source }).collect();
                let obs_b: Vec<ObsPoint> = raw_b.into_iter().map(|(value, source)| ObsPoint { value, source }).collect();
                Some((la, obs_a, lb, obs_b, Vec::new(), false))
            }
        }
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
            let cat_idx = mv_group_col
                .filter(|&i| i < mv.variables.len())
                .or_else(|| {
                    graph
                        .spec
                        .multivariate
                        .as_ref()
                        .map(|m| m.x_column)
                        .filter(|&i| mv.variables.get(i).is_some_and(|v| v.role == VariableRole::Categorical))
                })
                .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Categorical));
            if let Some(ci) = cat_idx {
                let ri = mv_resp_col
                    .filter(|&i| i < mv.variables.len() && i != ci)
                    .or_else(|| graph.spec.multivariate.as_ref().map(|m| m.y_column).filter(|&i| i < mv.variables.len() && i != ci))
                    .or_else(|| {
                        mv.variables
                            .iter()
                            .enumerate()
                            .position(|(i, v)| i != ci && matches!(v.role, VariableRole::Continuous | VariableRole::Calculated))
                    })?;
                let cat_col = &mv.variables[ci];
                let resp_col = &mv.variables[ri];
                let levels = column_categorical_levels(cat_col, None);
                if levels.len() < 2 {
                    return None;
                }
                let la = levels[0].clone();
                let lb = levels[1].clone();
                let mut obs_a = Vec::new();
                let mut obs_b = Vec::new();
                for r in 0..mv.row_count() {
                    if let (Some(tok), Some(val)) =
                        (cell_token(cat_col, r), resp_col.cells.get(r).and_then(|c| c.active_number()).filter(|v| v.is_finite()))
                    {
                        let pt = ObsPoint { value: val, source: SourceCellId { sheet_id: ds.id.clone(), row: r, col: ri, subcol: 0 } };
                        if tok == la {
                            obs_a.push(pt);
                        } else if tok == lb {
                            obs_b.push(pt);
                        }
                    }
                }
                let links = if force_paired { obs_a.iter().zip(obs_b.iter()).map(|(a, b)| (a.value, b.value)).collect() } else { Vec::new() };
                return Some((la, obs_a, lb, obs_b, links, false));
            }
            // Fallback: two numeric columns treated directly as Group A and Group B.
            let (ca, cb) = preferred_cols.filter(|&(a, b)| a < mv.variables.len() && b < mv.variables.len() && a != b).unwrap_or((0, 1));
            let label_a = mv.variables[ca].title.clone();
            let label_b = mv.variables[cb].title.clone();
            if force_paired {
                let mut obs_a = Vec::new();
                let mut obs_b = Vec::new();
                let mut links = Vec::new();
                for r in 0..mv.row_count() {
                    if let (Some(va), Some(vb)) = (
                        mv.variables[ca].cells.get(r).and_then(|c| c.active_number()).filter(|v| v.is_finite()),
                        mv.variables[cb].cells.get(r).and_then(|c| c.active_number()).filter(|v| v.is_finite()),
                    ) {
                        obs_a.push(ObsPoint { value: va, source: SourceCellId { sheet_id: ds.id.clone(), row: r, col: ca, subcol: 0 } });
                        obs_b.push(ObsPoint { value: vb, source: SourceCellId { sheet_id: ds.id.clone(), row: r, col: cb, subcol: 0 } });
                        links.push((va, vb));
                    }
                }
                Some((label_a, obs_a, label_b, obs_b, links, false))
            } else {
                let (raw_a, la) = collect_ds_active_values_with_sources(ds, ca);
                let (raw_b, lb) = collect_ds_active_values_with_sources(ds, cb);
                let obs_a = raw_a.into_iter().map(|(value, source)| ObsPoint { value, source }).collect();
                let obs_b = raw_b.into_iter().map(|(value, source)| ObsPoint { value, source }).collect();
                Some((la, obs_a, lb, obs_b, Vec::new(), false))
            }
        }
        _ => None,
    }
}

fn group_mean_ci(vals: &[f64], confidence: ConfidenceLevel) -> Option<(f64, f64, f64)> {
    if vals.len() < 2 {
        return None;
    }
    let mom = SummaryMoments::from_slice(vals).ok()?;
    let sd = mom.sample_sd().unwrap_or(0.0);
    let n = vals.len() as f64;
    let se = sd / n.sqrt();
    let half = if se > 0.0 {
        let t_crit = student_t_two_sided_isf(confidence.alpha(), n - 1.0).ok()?;
        t_crit * se
    } else {
        0.0
    };
    Some((mom.mean, mom.mean - half, mom.mean + half))
}

fn unpaired_diff_ci(a: &[f64], b: &[f64], confidence: ConfidenceLevel, welch: bool) -> Option<(f64, f64, f64)> {
    if a.len() < 2 || b.len() < 2 {
        return None;
    }
    let ma = SummaryMoments::from_slice(a).ok()?;
    let mb = SummaryMoments::from_slice(b).ok()?;
    let na = a.len() as f64;
    let nb = b.len() as f64;
    let va = ma.sample_variance().unwrap_or(0.0);
    let vb = mb.sample_variance().unwrap_or(0.0);
    let diff = mb.mean - ma.mean;
    let (se, df) = if welch {
        let se2 = va / na + vb / nb;
        if se2 <= 0.0 {
            return Some((diff, diff, diff));
        }
        let qa = va / na;
        let qb = vb / nb;
        let denom = (qa * qa) / (na - 1.0) + (qb * qb) / (nb - 1.0);
        let df_w = if denom > 0.0 { (se2 * se2) / denom } else { na + nb - 2.0 };
        (se2.sqrt(), df_w.max(1.0))
    } else {
        let df_p = na + nb - 2.0;
        let sp2 = ((na - 1.0) * va + (nb - 1.0) * vb) / df_p;
        let se2 = sp2 * (1.0 / na + 1.0 / nb);
        if se2 <= 0.0 {
            return Some((diff, diff, diff));
        }
        (se2.sqrt(), df_p)
    };
    let t_crit = student_t_two_sided_isf(confidence.alpha(), df).ok()?;
    let half = t_crit * se;
    Some((diff, diff - half, diff + half))
}
