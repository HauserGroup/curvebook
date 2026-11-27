//! XY scatter, line, error-bar, confidence-band, and fitted regression curve scene builder,
//! plus shared multi-segment axis, tick, frame, marker, error-bar, annotation, and legend kernels.

mod fit_overlay;
mod series;

use curvebook_doc::{
    AxisSpec, AxisThreshold, BreakOrnament, DataSheet, EmbeddedTableStyle, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FramePreset, GraphSheet,
    ProjectDocument, ResultStatus, SheetId, SubcolumnMode, SummarizedPoint, SummaryMeasure, TickDirection, TickLabelFormat,
};
use curvebook_math::AxisScale;
use curvebook_math::dist::student_t_two_sided_isf;
use curvebook_math::sum::SummaryMoments;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerStyle, Rgba8};
use curvebook_stats::common::{quantile_sorted, quartiles_hinge, sorted_finite};
use curvebook_text::{LocaleNumberPolicy, RichText, SpanWeight, format_axis_tick, measure_text_pt, parse_markup};

pub use series::{compile_xy_graph, compile_xy_graph_with_options};

use crate::scales::{
    AdditionalYTickLayout, AxisOrientation, DEFAULT_GRID_COLOR, ResolvedAdditionalTick, ResolvedAxis, ResolvedSegment,
    compute_additional_ticks_for_segments, compute_segment_tick_values, emit_additional_tick_gridline_segments, emit_additional_x_tick_primitives,
    emit_additional_y_tick_primitives, format_tick_rich, resolve_exact_domain_with_warnings, tick_span_offsets,
};
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive,
    StrokePaint, TextAnchor,
};

/// Complete multi-segment horizontal or vertical axis layout backed by `scales::ResolvedAxis`.
#[derive(Debug, Clone)]
pub(crate) struct SegmentedAxis {
    pub segments: Vec<ResolvedSegment>,
    pub break_ornament: BreakOrnament,
    pub tick_format: TickLabelFormat,
    pub tick_angle_deg: f64,
    pub tick_direction: TickDirection,
    pub show_tick_labels: bool,
    pub stroke_width_pt: f64,
    pub warnings: Vec<ChartWarning>,
}

impl SegmentedAxis {
    pub(crate) fn build(
        spec: &AxisSpec,
        span_start: f64,
        span_end: f64,
        data_values: &[f64],
        include_zero_if_linear: bool,
        top_headroom_frac: f64,
    ) -> Self {
        let mut warnings = Vec::new();
        let orientation = if span_end < span_start { AxisOrientation::Vertical } else { AxisOrientation::Horizontal };
        let resolved = ResolvedAxis::resolve_from_samples(
            spec,
            orientation,
            (span_start, span_end),
            data_values,
            include_zero_if_linear,
            top_headroom_frac,
            &mut warnings,
        );
        Self {
            segments: resolved.segments,
            break_ornament: spec.break_ornament,
            tick_format: spec.tick_format,
            tick_angle_deg: spec.tick_angle_deg,
            tick_direction: spec.tick_direction,
            show_tick_labels: spec.show_tick_labels,
            stroke_width_pt: if spec.stroke_width_pt > 0.0 { spec.stroke_width_pt } else { 1.0 },
            warnings,
        }
    }

    pub(crate) fn build_exact(spec: &AxisSpec, span_start: f64, span_end: f64, exact_bounds: (f64, f64)) -> Self {
        let mut warnings = Vec::new();
        let orientation = if span_end < span_start { AxisOrientation::Vertical } else { AxisOrientation::Horizontal };
        let segments = if spec.segments.len() <= 1 {
            let (domain_min, domain_max) = resolve_exact_domain_with_warnings(spec, exact_bounds.0, exact_bounds.1, &mut warnings);
            let seg = spec.segments.first();
            vec![ResolvedSegment {
                segment_index: 0,
                domain_min,
                domain_max,
                scale: seg.map_or(AxisScale::Linear, |s| s.scale),
                length_fraction: 1.0,
                span_start_pt: span_start,
                span_end_pt: span_end,
                major_tick_interval: seg.and_then(|s| s.major_tick_interval),
                minor_ticks_per_interval: seg.map_or(0, |s| s.minor_ticks_per_interval),
            }]
        } else {
            let plot_rect = match orientation {
                AxisOrientation::Horizontal => RectPt::new(span_start, 0.0, (span_end - span_start).abs(), 100.0),
                AxisOrientation::Vertical => RectPt::new(0.0, span_end, 100.0, (span_start - span_end).abs()),
            };
            ResolvedAxis::resolve(spec, orientation, plot_rect, Some(exact_bounds), &mut warnings).segments
        };
        Self {
            segments,
            break_ornament: spec.break_ornament,
            tick_format: spec.tick_format,
            tick_angle_deg: spec.tick_angle_deg,
            tick_direction: spec.tick_direction,
            show_tick_labels: spec.show_tick_labels,
            stroke_width_pt: if spec.stroke_width_pt > 0.0 { spec.stroke_width_pt } else { 1.0 },
            warnings,
        }
    }

    pub(crate) fn preview_tick_labels(&self, policy: &LocaleNumberPolicy) -> Vec<String> {
        let mut dummy = Vec::new();
        let mut out = Vec::new();
        for seg in &self.segments {
            let (majors, _) = compute_segment_tick_values(seg, None, false, &mut dummy);
            for val in majors {
                out.push(format_tick_rich(val, seg.scale, self.tick_format, policy).plain_text());
            }
        }
        out
    }

    pub(crate) fn map_value(&self, value: f64) -> Option<(usize, f64)> {
        for seg in &self.segments {
            if let Some(pt) = seg.map_strict(value) {
                return Some((seg.segment_index, pt));
            }
        }
        None
    }

    pub(crate) fn map_clamped_to_nearest(&self, value: f64) -> (usize, f64, bool) {
        if let Some((idx, pt)) = self.map_value(value) {
            return (idx, pt, false);
        }
        let mut best_idx = 0;
        let mut best_dist = f64::INFINITY;
        for seg in &self.segments {
            let seg_lo = seg.domain_min.min(seg.domain_max);
            let seg_hi = seg.domain_min.max(seg.domain_max);
            let d = if value < seg_lo {
                (seg_lo - value).abs()
            } else if value > seg_hi {
                (value - seg_hi).abs()
            } else {
                0.0
            };
            if d < best_dist {
                best_dist = d;
                best_idx = seg.segment_index;
            }
        }
        if let Some(seg) = self.segments.get(best_idx) {
            let (pt, clipped) = seg.map_clamped(value);
            (best_idx, pt, clipped)
        } else {
            (0, 0.0, true)
        }
    }
}

/// Resolve a `DataSheet` from either top-level `data_sheets` or an analyzable `ResultSheet`.
pub(crate) fn resolve_data_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Option<&'a DataSheet> {
    crate::resolve_data_sheet(doc, id)
}

/// Resolve inline `{{info:...}}` and `{{result:...}}` template hooks and parse markup into `RichText`.
pub(crate) fn expand_doc_text(raw: &str, doc: &ProjectDocument, primary_info_id: Option<&SheetId>) -> RichText {
    let expanded = crate::scales::resolve_template_hooks_with_info(raw, doc, primary_info_id);
    parse_markup(&expanded)
}

pub(crate) fn primary_info_sheet_id<'a>(doc: &'a ProjectDocument, primary_data_id: &SheetId) -> Option<&'a SheetId> {
    doc.paired_info_sheet_id(primary_data_id)
}

/// Configure `scene.plot_rect_pt` with dynamic margins accounting for title, axis titles, Y2, brackets, and legend.
pub(crate) fn configure_plot_rect(
    graph: &GraphSheet,
    scene: &mut ChartScene,
    has_y2: bool,
    top_bracket_extra_pt: f64,
    auto_legend_box_w_pt: Option<f64>,
) {
    let w = scene.width_pt;
    let h = scene.height_pt;
    let has_title = !graph.spec.title.trim().is_empty();
    let has_x_title = !graph.spec.x_axis.title.trim().is_empty();
    let has_y_title = !graph.spec.y_axis.title.trim().is_empty();
    let y2_title_len = graph.spec.y2_axis.as_ref().map_or(0, |a| a.title.trim().len());

    let left_base: f64 = if has_y_title { 56.0 } else { 42.0 };
    let has_right_y2 = has_y2 || y2_title_len > 0;
    let right_base: f64 = match (has_right_y2, auto_legend_box_w_pt) {
        (true, Some(box_w)) => 48.0 + box_w + 12.0,
        (true, None) => 56.0,
        (false, Some(box_w)) => (box_w + 12.0).max(86.0),
        (false, None) => 20.0,
    };
    let top_base: f64 = if has_title { 32.0 } else { 18.0 } + top_bracket_extra_pt.max(0.0);
    let rot_extra: f64 = if graph.spec.x_axis.tick_angle_deg.abs() > 15.0 { 16.0 } else { 0.0 };
    let bottom_base: f64 = if has_x_title { 44.0 } else { 28.0 } + rot_extra;

    let left = left_base.min(w * 0.28);
    let max_right = match (has_right_y2, auto_legend_box_w_pt.is_some()) {
        (true, true) => w * 0.48,
        (false, true) => w * 0.42,
        _ => w * 0.30,
    };
    let right = right_base.min(max_right);
    let top = top_base.min(h * 0.42);
    let bottom = bottom_base.min(h * 0.30);

    let plot_w = (w - left - right).max(24.0);
    let plot_h = (h - top - bottom).max(24.0);
    scene.plot_rect_pt = RectPt::new(left, top, plot_w, plot_h);
}

pub(crate) fn stroke_with_dash(color: Rgba8, width_pt: f64, dash: LineDashStyle) -> StrokePaint {
    StrokePaint { color, width_pt: if width_pt > 0.0 { width_pt } else { 1.0 }, dash, cap: LineCap::Butt, join: LineJoin::Miter }
}

/// Compute central tendency (`Mean`, `Median`, or `GeometricMean`) over active finite values.
pub(crate) fn compute_central_tendency(values: &[f64], summary: SummaryMeasure, warnings: &mut Vec<ChartWarning>) -> Option<f64> {
    let finite = sorted_finite(values);
    if finite.is_empty() {
        return None;
    }
    match summary {
        SummaryMeasure::Mean => SummaryMoments::from_slice(&finite).ok().map(|m| m.mean),
        SummaryMeasure::Median => quantile_sorted(&finite, 0.5),
        SummaryMeasure::GeometricMean => {
            if finite.iter().any(|&v| v <= 0.0) {
                warnings.push(ChartWarning::new(
                    "non_positive_geometric_mean",
                    "geometric mean is undefined for zero or negative values; series summary omitted",
                ));
                return None;
            }
            let sum_ln: f64 = finite.iter().map(|v| v.ln()).sum();
            let gm = (sum_ln / (finite.len() as f64)).exp();
            gm.is_finite().then_some(gm)
        }
    }
}

/// Summarize either pre-summarized subcolumns or raw replicates into `(center, lower_bound, upper_bound)`.
pub(crate) fn summarize_subcells(
    mode: SubcolumnMode,
    subcells: &[curvebook_doc::Cell],
    summary: SummaryMeasure,
    error_spec: Option<&ErrorBarSpec>,
    warnings: &mut Vec<ChartWarning>,
) -> Option<(f64, f64, f64)> {
    if mode.is_pre_summarized() {
        return summarize_pre_summarized(mode, subcells, error_spec, warnings);
    }
    let raw: Vec<f64> = subcells.iter().filter_map(curvebook_doc::Cell::active_number).collect();
    let center = compute_central_tendency(&raw, summary, warnings)?;
    let (lo, hi) = if let Some(eb) = error_spec { compute_raw_error_bounds(&raw, center, eb.metric, 0.05) } else { (center, center) };
    Some((center, lo, hi))
}

fn summarize_pre_summarized(
    mode: SubcolumnMode,
    subcells: &[curvebook_doc::Cell],
    error_spec: Option<&ErrorBarSpec>,
    warnings: &mut Vec<ChartWarning>,
) -> Option<(f64, f64, f64)> {
    let center_only = subcells.first().and_then(curvebook_doc::Cell::active_number).filter(|v| v.is_finite());
    let Some(eb) = error_spec else {
        return center_only.map(|c| (c, c, c));
    };
    let explicit = mode.explicit_bounds(subcells);
    if mode.supports_parametric_moments() {
        match SummarizedPoint::reconstruct(mode, subcells) {
            Ok(Some(pt)) => {
                let (lo, hi) = match eb.metric {
                    ErrorBarMetric::Sd => (pt.mean - pt.sd, pt.mean + pt.sd),
                    ErrorBarMetric::Sem => {
                        let sem = if pt.n > 0.0 { pt.sd / pt.n.sqrt() } else { 0.0 };
                        (pt.mean - sem, pt.mean + sem)
                    }
                    ErrorBarMetric::ConfidenceInterval95 => {
                        let half = if pt.df_within >= 1.0 && pt.n > 0.0 {
                            let sem = pt.sd / pt.n.sqrt();
                            let t = student_t_two_sided_isf(0.05, pt.df_within).unwrap_or(1.959_963_984_540_054);
                            t * sem
                        } else {
                            warnings.push(ChartWarning::new(
                                "insufficient_sample_size_for_ci",
                                format!("95% CI requires N >= 2 (got N = {}); error bar omitted", pt.n),
                            ));
                            0.0
                        };
                        (pt.mean - half, pt.mean + half)
                    }
                    ErrorBarMetric::ExplicitSubcolumns | ErrorBarMetric::InterquartileRange | ErrorBarMetric::Range => {
                        if let Some(b) = explicit {
                            (b.lower, b.upper)
                        } else {
                            (pt.mean - pt.sd, pt.mean + pt.sd)
                        }
                    }
                };
                return Some((pt.mean, lo, hi));
            }
            Ok(None) => return None,
            Err(err) => {
                warnings.push(ChartWarning::new("invalid_pre_summarized_point", err.to_string()));
                let center = center_only?;
                let can_use_direct_spread = !matches!(
                    err,
                    curvebook_doc::SummaryPointError::NegativeSpread { .. }
                        | curvebook_doc::SummaryPointError::SampleSizeBelowOne { .. }
                        | curvebook_doc::SummaryPointError::MissingSpread
                ) && matches!(
                    (mode, eb.metric),
                    (SubcolumnMode::MeanSdN | SubcolumnMode::MeanCvN, ErrorBarMetric::Sd | ErrorBarMetric::ExplicitSubcolumns)
                        | (SubcolumnMode::MeanSemN, ErrorBarMetric::Sem | ErrorBarMetric::ExplicitSubcolumns)
                );
                if can_use_direct_spread && let Some(b) = explicit {
                    return Some((b.center, b.lower, b.upper));
                }
                return Some((center, center, center));
            }
        }
    }
    if matches!(mode, SubcolumnMode::MeanSd | SubcolumnMode::MeanSem) && eb.metric == ErrorBarMetric::ConfidenceInterval95 {
        return center_only.map(|center| {
            warnings.push(ChartWarning::new(
                "missing_sample_size_for_ci",
                format!("95% CI requires sample size N, which is absent in {mode:?} mode; error bar omitted"),
            ));
            (center, center, center)
        });
    }
    if let Some(b) = explicit {
        return Some((b.center, b.lower, b.upper));
    }
    center_only.map(|center| {
        warnings.push(ChartWarning::new(
            "incomplete_explicit_error_bounds",
            format!("pre-summarized {mode:?} row is missing lower/upper spread cells; error bar omitted"),
        ));
        (center, center, center)
    })
}

/// Compute `(lower_endpoint, upper_endpoint)` around `center` from raw replicate values.
pub(crate) fn compute_raw_error_bounds(values: &[f64], center: f64, metric: ErrorBarMetric, alpha: f64) -> (f64, f64) {
    let sorted = sorted_finite(values);
    if sorted.is_empty() {
        return (center, center);
    }
    match metric {
        ErrorBarMetric::Sd => {
            let sd = SummaryMoments::from_slice(&sorted).ok().and_then(|m| m.sample_sd().ok()).unwrap_or(0.0);
            (center - sd, center + sd)
        }
        ErrorBarMetric::Sem => {
            let sem = SummaryMoments::from_slice(&sorted).ok().and_then(|m| m.sem().ok()).unwrap_or(0.0);
            (center - sem, center + sem)
        }
        ErrorBarMetric::ConfidenceInterval95 => {
            let n = sorted.len();
            if n < 2 {
                return (center, center);
            }
            let sem = SummaryMoments::from_slice(&sorted).ok().and_then(|m| m.sem().ok()).unwrap_or(0.0);
            let t_crit = student_t_two_sided_isf(alpha, (n - 1) as f64).unwrap_or(1.959_963_984_540_054);
            let half = t_crit * sem;
            (center - half, center + half)
        }
        ErrorBarMetric::InterquartileRange => {
            if let Some((q1, _, q3)) = quartiles_hinge(&sorted) {
                (q1.min(center), q3.max(center))
            } else {
                (center, center)
            }
        }
        ErrorBarMetric::Range => {
            let lo = sorted.first().copied().unwrap_or(center);
            let hi = sorted.last().copied().unwrap_or(center);
            (lo.min(center), hi.max(center))
        }
        ErrorBarMetric::ExplicitSubcolumns => (center, center),
    }
}

/// Axis binding & styling context for directional error-bar rendering.
pub(crate) struct AxisErrorTarget<'a> {
    pub axis: &'a SegmentedAxis,
    pub is_vertical: bool,
    pub spec: &'a ErrorBarSpec,
    pub default_color: Rgba8,
    pub clip: RectPt,
}

/// Directional vertical or horizontal error-bar rendering with clean log-axis floor clipping.
pub(crate) fn emit_directional_error_bar(
    scene: &mut ChartScene,
    fixed_pt: f64,
    center_data: f64,
    lo_data: f64,
    hi_data: f64,
    target: AxisErrorTarget<'_>,
) {
    let primitives = &mut scene.primitives;
    let warnings = &mut scene.warnings;
    let axis = target.axis;
    let is_vertical = target.is_vertical;
    let spec = target.spec;
    let default_color = target.default_color;
    let clip = target.clip;
    let Some((seg_idx, center_pt)) = axis.map_value(center_data) else {
        return;
    };
    let Some(seg) = axis.segments.get(seg_idx) else {
        return;
    };
    let color = spec.color.unwrap_or(default_color);
    let stroke = stroke_with_dash(color, spec.stroke_width_pt, spec.dash_style);
    let half_cap = 0.5 * spec.cap_width_pt.max(0.0);

    let draw_lo = !matches!(spec.direction, ErrorBarDirection::UpperOnly);
    let draw_hi = !matches!(spec.direction, ErrorBarDirection::LowerOnly);

    let (lo_pt, lo_clipped) = if draw_lo && lo_data < center_data {
        let (pt, clipped) = seg.map_clamped(lo_data);
        if clipped && is_log_like(seg.scale) && lo_data <= 0.0 {
            warnings.push(ChartWarning::new(
                "log_axis_floor_clip",
                format!("lower error-bar endpoint ({lo_data:.4e}) is non-positive on {:?} axis; clipped cleanly to segment floor", seg.scale),
            ));
        }
        (Some(pt), clipped)
    } else {
        (None, false)
    };

    let (hi_pt, hi_clipped) = if draw_hi && hi_data > center_data {
        let (pt, clipped) = seg.map_clamped(hi_data);
        (Some(pt), clipped)
    } else {
        (None, false)
    };

    let start_pt = lo_pt.unwrap_or(center_pt);
    let end_pt = hi_pt.unwrap_or(center_pt);
    if (end_pt - start_pt).abs() < 0.05 {
        return;
    }

    let make_pt = |fixed: f64, along: f64| if is_vertical { PointPt::new(fixed, along) } else { PointPt::new(along, fixed) };
    primitives.push(ScenePrimitive::Path {
        path: Path2D::line(make_pt(fixed_pt, start_pt), make_pt(fixed_pt, end_pt)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(clip),
    });

    if half_cap > 0.0 {
        let cap_stroke = stroke_with_dash(color, spec.stroke_width_pt, LineDashStyle::Solid);
        if let Some(lp) = lo_pt
            && !lo_clipped
        {
            let p0 = if is_vertical { PointPt::new(fixed_pt - half_cap, lp) } else { PointPt::new(lp, fixed_pt - half_cap) };
            let p1 = if is_vertical { PointPt::new(fixed_pt + half_cap, lp) } else { PointPt::new(lp, fixed_pt + half_cap) };
            primitives.push(ScenePrimitive::Path { path: Path2D::line(p0, p1), fill: None, stroke: Some(cap_stroke), clip: Some(clip) });
        }
        if let Some(hp) = hi_pt
            && !hi_clipped
        {
            let p0 = if is_vertical { PointPt::new(fixed_pt - half_cap, hp) } else { PointPt::new(hp, fixed_pt - half_cap) };
            let p1 = if is_vertical { PointPt::new(fixed_pt + half_cap, hp) } else { PointPt::new(hp, fixed_pt + half_cap) };
            primitives.push(ScenePrimitive::Path { path: Path2D::line(p0, p1), fill: None, stroke: Some(cap_stroke), clip: Some(clip) });
        }
    }
}

pub(crate) fn is_log_like(scale: AxisScale) -> bool {
    matches!(scale, AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln)
}

/// Axis bundle with optional categorical X or Y tick lists for shared cartesian chrome emission.
pub(crate) struct ChromeAxes<'a> {
    pub x: &'a SegmentedAxis,
    pub y: &'a SegmentedAxis,
    pub y2: Option<&'a SegmentedAxis>,
    pub cat_x: Option<&'a [(f64, String)]>,
    pub cat_y: Option<&'a [(f64, String)]>,
}

/// Emit axes, frames, ticks, break marks, thresholds, and axis titles shared across cartesian figures.
pub(crate) fn render_cartesian_chrome(
    scene: &mut ChartScene,
    graph: &GraphSheet,
    doc: &ProjectDocument,
    x_axis: &SegmentedAxis,
    y_axis: &SegmentedAxis,
    y2_axis: Option<&SegmentedAxis>,
    categorical_x_ticks: Option<&[(f64, String)]>,
) {
    render_cartesian_chrome_full(scene, graph, doc, &ChromeAxes { x: x_axis, y: y_axis, y2: y2_axis, cat_x: categorical_x_ticks, cat_y: None });
}

/// Emit axes, frames, ticks (with optional X or Y categorical labels), break marks, thresholds, and titles.
pub(crate) fn render_cartesian_chrome_full(scene: &mut ChartScene, graph: &GraphSheet, doc: &ProjectDocument, axes: &ChromeAxes<'_>) {
    let x_axis = axes.x;
    let y_axis = axes.y;
    let y2_axis = axes.y2;
    let r = scene.plot_rect_pt;
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    let primary_opt = Some(&graph.spec.primary_source_sheet_id);
    let policy = LocaleNumberPolicy::default();
    let grid_color = DEFAULT_GRID_COLOR;

    scene.warnings.extend(x_axis.warnings.iter().cloned());
    scene.warnings.extend(y_axis.warnings.iter().cloned());
    if let Some(y2) = y2_axis {
        scene.warnings.extend(y2.warnings.iter().cloned());
    }

    let x_add = resolve_segmented_additional_ticks(x_axis, &graph.spec.x_axis, &policy, doc, primary_opt);
    let y_add = resolve_segmented_additional_ticks(y_axis, &graph.spec.y_axis, &policy, doc, primary_opt);
    let y2_add = match (y2_axis, graph.spec.y2_axis.as_ref()) {
        (Some(y2), Some(y2_spec)) => resolve_segmented_additional_ticks(y2, y2_spec, &policy, doc, primary_opt),
        _ => Vec::new(),
    };
    let x_spans: Vec<(f64, f64)> = x_axis.segments.iter().map(|s| (s.span_start_pt, s.span_end_pt)).collect();
    let y_spans: Vec<(f64, f64)> = y_axis.segments.iter().map(|s| (s.span_start_pt, s.span_end_pt)).collect();
    emit_additional_tick_gridline_segments(&x_add, true, &y_spans, grid_color, &mut scene.primitives);
    emit_additional_tick_gridline_segments(&y_add, false, &x_spans, grid_color, &mut scene.primitives);
    emit_additional_tick_gridline_segments(&y2_add, false, &x_spans, grid_color, &mut scene.primitives);

    emit_thresholds(&mut scene.primitives, r, x_axis, &graph.spec.x_axis.thresholds, true, doc, info_id);
    emit_thresholds(&mut scene.primitives, r, y_axis, &graph.spec.y_axis.thresholds, false, doc, info_id);
    if let (Some(y2), Some(y2_spec)) = (y2_axis, graph.spec.y2_axis.as_ref()) {
        emit_thresholds(&mut scene.primitives, r, y2, &y2_spec.thresholds, false, doc, info_id);
    }

    let offset = if matches!(graph.spec.frame_preset, FramePreset::OffsetRangeTufte) { 4.0 } else { 0.0 };
    let (x_spine_y, y_spine_x) = if matches!(graph.spec.frame_preset, FramePreset::OriginCrossing) {
        let x_pos = match y_axis.map_value(0.0) {
            Some((_, pt)) => pt,
            None => {
                scene.warnings.push(ChartWarning::new("origin_outside_y_domain", "Y=0 lies outside domain; crosshair X spine placed at plot bottom"));
                r.bottom()
            }
        };
        let y_pos = match x_axis.map_value(0.0) {
            Some((_, pt)) => pt,
            None => {
                scene.warnings.push(ChartWarning::new("origin_outside_x_domain", "X=0 lies outside domain; crosshair Y spine placed at plot left"));
                r.x
            }
        };
        (x_pos, y_pos)
    } else {
        (r.bottom() + offset, r.x - offset)
    };

    let x_major_pts = emit_x_ticks(&mut scene.primitives, &mut scene.warnings, x_spine_y, x_axis, axes.cat_x, &policy);
    if !x_add.is_empty() {
        let tick_stroke = StrokePaint::solid(Rgba8::BLACK, x_axis.stroke_width_pt.max(0.75));
        emit_additional_x_tick_primitives(&x_add, x_spine_y, x_axis.tick_angle_deg, x_axis.tick_direction, tick_stroke, &mut scene.primitives);
    }
    let left_y_ctx = YTickAxisCtx { spine_x: y_spine_x, is_left: true, axis: y_axis, categorical_ticks: axes.cat_y, policy: &policy };
    let y_major_pts = emit_y_ticks(&mut scene.primitives, &mut scene.warnings, &left_y_ctx);
    if !y_add.is_empty() {
        let tick_stroke = StrokePaint::solid(Rgba8::BLACK, y_axis.stroke_width_pt.max(0.75));
        emit_additional_y_tick_primitives(
            &y_add,
            AdditionalYTickLayout {
                spine_x: y_spine_x,
                is_left: true,
                tick_angle_deg: y_axis.tick_angle_deg,
                label_offset_pt: 6.5,
                tick_direction: y_axis.tick_direction,
            },
            tick_stroke,
            &mut scene.primitives,
        );
    }
    if let Some(y2) = y2_axis {
        let right_y_ctx = YTickAxisCtx { spine_x: r.right(), is_left: false, axis: y2, categorical_ticks: None, policy: &policy };
        let _ = emit_y_ticks(&mut scene.primitives, &mut scene.warnings, &right_y_ctx);
        if !y2_add.is_empty() {
            let tick_stroke = StrokePaint::solid(Rgba8::BLACK, y2.stroke_width_pt.max(0.75));
            emit_additional_y_tick_primitives(
                &y2_add,
                AdditionalYTickLayout {
                    spine_x: r.right(),
                    is_left: false,
                    tick_angle_deg: y2.tick_angle_deg,
                    label_offset_pt: 6.5,
                    tick_direction: y2.tick_direction,
                },
                tick_stroke,
                &mut scene.primitives,
            );
        }
    }

    emit_frame_spines(
        &mut scene.primitives,
        &FrameSpinesCtx {
            plot_rect: r,
            preset: graph.spec.frame_preset,
            x_axis,
            y_axis,
            y2_axis,
            spine_xy: (x_spine_y, y_spine_x),
            x_major_pts: &x_major_pts,
            y_major_pts: &y_major_pts,
        },
    );

    // Axis titles (expanded + parsed for Greek / sub / superscript)
    if !graph.spec.x_axis.title.trim().is_empty() {
        let rich = expand_doc_text(&graph.spec.x_axis.title, doc, info_id);
        let y_title = r.bottom() + if x_axis.tick_angle_deg.abs() > 15.0 { 36.0 } else { 26.0 };
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich,
            anchor_pt: PointPt::new(r.x + 0.5 * r.width, y_title.min(scene.height_pt - 4.0)),
            anchor: TextAnchor::Middle,
            font_size_pt: 10.0,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
    if !graph.spec.y_axis.title.trim().is_empty() {
        let rich = expand_doc_text(&graph.spec.y_axis.title, doc, info_id);
        let y_title_x = if axes.cat_y.is_some() { 11.0_f64.min(r.x * 0.25) } else { 14.0_f64.min(r.x * 0.35) };
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich,
            anchor_pt: PointPt::new(y_title_x, r.y + 0.5 * r.height),
            anchor: TextAnchor::Middle,
            font_size_pt: 10.0,
            rotation_deg: -90.0,
            color: Rgba8::BLACK,
        }));
    }
    if let Some(y2_spec) = &graph.spec.y2_axis
        && !y2_spec.title.trim().is_empty()
    {
        let rich = expand_doc_text(&y2_spec.title, doc, info_id);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich,
            anchor_pt: PointPt::new((scene.width_pt - 14.0).max(r.right() + 26.0), r.y + 0.5 * r.height),
            anchor: TextAnchor::Middle,
            font_size_pt: 10.0,
            rotation_deg: 90.0,
            color: Rgba8::BLACK,
        }));
    }
}

fn resolve_segmented_additional_ticks(
    axis: &SegmentedAxis,
    spec: &AxisSpec,
    policy: &LocaleNumberPolicy,
    doc: &ProjectDocument,
    primary_id: Option<&SheetId>,
) -> Vec<ResolvedAdditionalTick> {
    if spec.additional_ticks.is_empty() {
        return Vec::new();
    }
    let segs: Vec<(usize, crate::scales::AxisMapping, f64, f64)> =
        axis.segments.iter().map(|s| (s.segment_index, s.mapping(), s.span_start_pt, s.span_end_pt)).collect();
    compute_additional_ticks_for_segments(&segs, &spec.additional_ticks, axis.tick_format, policy, Some(doc), primary_id)
}

fn emit_thresholds(
    primitives: &mut Vec<ScenePrimitive>,
    r: RectPt,
    axis: &SegmentedAxis,
    thresholds: &[AxisThreshold],
    is_x: bool,
    doc: &ProjectDocument,
    info_id: Option<&SheetId>,
) {
    for th in thresholds {
        let Some((seg_idx, pos)) = axis.map_value(th.value) else {
            continue;
        };
        if let Some(end_val) = th.shade_to {
            let (pt_end, _) = axis.segments[seg_idx].map_clamped(end_val);
            let lo = pos.min(pt_end);
            let hi = pos.max(pt_end);
            let rect = if is_x { RectPt::new(lo, r.y, hi - lo, r.height) } else { RectPt::new(r.x, lo, r.width, hi - lo) };
            primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(rect),
                fill: Some(FillPaint::solid(th.color.with_alpha(th.color.a.min(64)))),
                stroke: None,
                clip: Some(r),
            });
        }
        let (p0, p1) =
            if is_x { (PointPt::new(pos, r.y), PointPt::new(pos, r.bottom())) } else { (PointPt::new(r.x, pos), PointPt::new(r.right(), pos)) };
        primitives.push(ScenePrimitive::Path {
            path: Path2D::line(p0, p1),
            fill: None,
            stroke: Some(stroke_with_dash(th.color, th.stroke_width_pt, th.dash_style)),
            clip: Some(r),
        });
        if let Some(label) = &th.label
            && !label.trim().is_empty()
        {
            let anchor_pt = if is_x { PointPt::new(pos + 3.0, r.y + 10.0) } else { PointPt::new(r.right() - 4.0, pos - 3.0) };
            let anchor = if is_x { TextAnchor::Start } else { TextAnchor::End };
            primitives.push(ScenePrimitive::Text(PositionedText {
                rich: expand_doc_text(label, doc, info_id),
                anchor_pt,
                anchor,
                font_size_pt: 8.0,
                rotation_deg: 0.0,
                color: th.color,
            }));
        }
    }
}

struct FrameSpinesCtx<'a> {
    plot_rect: RectPt,
    preset: FramePreset,
    x_axis: &'a SegmentedAxis,
    y_axis: &'a SegmentedAxis,
    y2_axis: Option<&'a SegmentedAxis>,
    spine_xy: (f64, f64),
    x_major_pts: &'a [f64],
    y_major_pts: &'a [f64],
}

fn emit_frame_spines(primitives: &mut Vec<ScenePrimitive>, ctx: &FrameSpinesCtx<'_>) {
    let r = ctx.plot_rect;
    let (x_spine_y, y_spine_x) = ctx.spine_xy;
    let x_stroke = StrokePaint::solid(Rgba8::BLACK, ctx.x_axis.stroke_width_pt);
    let y_stroke = StrokePaint::solid(Rgba8::BLACK, ctx.y_axis.stroke_width_pt);

    match ctx.preset {
        FramePreset::OpenL => {
            draw_segmented_spine(primitives, x_spine_y, true, ctx.x_axis, x_stroke, None);
            draw_segmented_spine(primitives, y_spine_x, false, ctx.y_axis, y_stroke, None);
        }
        FramePreset::OffsetRangeTufte => {
            draw_segmented_spine(primitives, x_spine_y, true, ctx.x_axis, x_stroke, Some(ctx.x_major_pts));
            draw_segmented_spine(primitives, y_spine_x, false, ctx.y_axis, y_stroke, Some(ctx.y_major_pts));
        }
        FramePreset::PlainBoxed => {
            draw_segmented_spine(primitives, r.bottom(), true, ctx.x_axis, x_stroke, None);
            draw_segmented_spine(primitives, r.y, true, ctx.x_axis, x_stroke, None);
            draw_segmented_spine(primitives, r.x, false, ctx.y_axis, y_stroke, None);
            draw_segmented_spine(primitives, r.right(), false, ctx.y2_axis.unwrap_or(ctx.y_axis), y_stroke, None);
        }
        FramePreset::DualYSecondaryRight => {
            draw_segmented_spine(primitives, r.bottom(), true, ctx.x_axis, x_stroke, None);
            draw_segmented_spine(primitives, r.x, false, ctx.y_axis, y_stroke, None);
            draw_segmented_spine(primitives, r.right(), false, ctx.y2_axis.unwrap_or(ctx.y_axis), y_stroke, None);
        }
        FramePreset::OriginCrossing => {
            draw_segmented_spine(primitives, x_spine_y, true, ctx.x_axis, x_stroke, None);
            draw_segmented_spine(primitives, y_spine_x, false, ctx.y_axis, y_stroke, None);
        }
    }
}

fn tufte_single_span(major_pts: &[f64], seg: &ResolvedSegment) -> (f64, f64) {
    let lo = major_pts.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = major_pts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if lo.is_finite() && hi.is_finite() && (hi - lo).abs() > 1.0 {
        if seg.span_end_pt >= seg.span_start_pt { (lo, hi) } else { (hi, lo) }
    } else {
        (seg.span_start_pt, seg.span_end_pt)
    }
}

fn draw_segmented_spine(
    primitives: &mut Vec<ScenePrimitive>,
    fixed: f64,
    is_horizontal: bool,
    axis: &SegmentedAxis,
    stroke: StrokePaint,
    tufte_major_pts: Option<&[f64]>,
) {
    let trim_single = tufte_major_pts.filter(|_| axis.segments.len() == 1);
    for (idx, seg) in axis.segments.iter().enumerate() {
        let (s0, s1) = trim_single.map_or((seg.span_start_pt, seg.span_end_pt), |pts| tufte_single_span(pts, seg));
        let (p0, p1) =
            if is_horizontal { (PointPt::new(s0, fixed), PointPt::new(s1, fixed)) } else { (PointPt::new(fixed, s0), PointPt::new(fixed, s1)) };
        primitives.push(ScenePrimitive::Path { path: Path2D::line(p0, p1), fill: None, stroke: Some(stroke), clip: None });
        if let Some(next) = axis.segments.get(idx + 1) {
            emit_break_ornament(primitives, fixed, is_horizontal, seg.span_end_pt, next.span_start_pt, axis.break_ornament, stroke);
        }
    }
}

fn emit_break_ornament(
    primitives: &mut Vec<ScenePrimitive>,
    fixed: f64,
    is_horizontal: bool,
    gap_a: f64,
    gap_b: f64,
    ornament: BreakOrnament,
    stroke: StrokePaint,
) {
    match ornament {
        BreakOrnament::BlankGap => {}
        BreakOrnament::DoubleSlash => {
            for edge in [gap_a, gap_b] {
                let (p0, p1) = if is_horizontal {
                    (PointPt::new(edge - 2.0, fixed + 4.0), PointPt::new(edge + 2.0, fixed - 4.0))
                } else {
                    (PointPt::new(fixed - 4.0, edge + 2.0), PointPt::new(fixed + 4.0, edge - 2.0))
                };
                primitives.push(ScenePrimitive::Path { path: Path2D::line(p0, p1), fill: None, stroke: Some(stroke), clip: None });
            }
        }
        BreakOrnament::Zigzag => {
            let mid = 0.5 * (gap_a + gap_b);
            let q1 = gap_a + 0.25 * (gap_b - gap_a);
            let q3 = gap_a + 0.75 * (gap_b - gap_a);
            let pts = if is_horizontal {
                [
                    PointPt::new(gap_a, fixed),
                    PointPt::new(q1, fixed - 3.5),
                    PointPt::new(mid, fixed),
                    PointPt::new(q3, fixed + 3.5),
                    PointPt::new(gap_b, fixed),
                ]
            } else {
                [
                    PointPt::new(fixed, gap_a),
                    PointPt::new(fixed - 3.5, q1),
                    PointPt::new(fixed, mid),
                    PointPt::new(fixed + 3.5, q3),
                    PointPt::new(fixed, gap_b),
                ]
            };
            let cmds =
                vec![PathCmd::MoveTo(pts[0]), PathCmd::LineTo(pts[1]), PathCmd::LineTo(pts[2]), PathCmd::LineTo(pts[3]), PathCmd::LineTo(pts[4])];
            primitives.push(ScenePrimitive::Path { path: Path2D { commands: cmds }, fill: None, stroke: Some(stroke), clip: None });
        }
    }
}

fn emit_x_ticks(
    primitives: &mut Vec<ScenePrimitive>,
    warnings: &mut Vec<ChartWarning>,
    spine_y: f64,
    axis: &SegmentedAxis,
    categorical_ticks: Option<&[(f64, String)]>,
    policy: &LocaleNumberPolicy,
) -> Vec<f64> {
    let tick_stroke = StrokePaint::solid(Rgba8::BLACK, axis.stroke_width_pt.max(0.75));
    let major_span = tick_span_offsets(axis.tick_direction, 4.0, 1.0);
    let minor_span = tick_span_offsets(axis.tick_direction, 2.2, 1.0);
    let mut major_pts = Vec::new();
    if let Some(ticks) = categorical_ticks {
        for (pos_val, label) in ticks {
            if let Some((_, x_pt)) = axis.map_value(*pos_val) {
                major_pts.push(x_pt);
                if let Some((o0, o1)) = major_span {
                    primitives.push(ScenePrimitive::Path {
                        path: Path2D::line(PointPt::new(x_pt, spine_y + o0), PointPt::new(x_pt, spine_y + o1)),
                        fill: None,
                        stroke: Some(tick_stroke),
                        clip: None,
                    });
                }
                if axis.show_tick_labels {
                    let anchor = if axis.tick_angle_deg.abs() > 15.0 { TextAnchor::End } else { TextAnchor::Middle };
                    primitives.push(ScenePrimitive::Text(PositionedText {
                        rich: parse_markup(label),
                        anchor_pt: PointPt::new(x_pt, spine_y + 13.0),
                        anchor,
                        font_size_pt: 8.5,
                        rotation_deg: axis.tick_angle_deg,
                        color: Rgba8::BLACK,
                    }));
                }
            }
        }
        return major_pts;
    }

    for seg in &axis.segments {
        let (majors, minors) = compute_segment_tick_values(seg, None, true, warnings);
        if let Some((o0, o1)) = minor_span {
            for m in minors {
                if let Some(x_pt) = seg.map_strict(m) {
                    primitives.push(ScenePrimitive::Path {
                        path: Path2D::line(PointPt::new(x_pt, spine_y + o0), PointPt::new(x_pt, spine_y + o1)),
                        fill: None,
                        stroke: Some(tick_stroke),
                        clip: None,
                    });
                }
            }
        }
        for val in majors {
            if let Some(x_pt) = seg.map_strict(val) {
                major_pts.push(x_pt);
                if let Some((o0, o1)) = major_span {
                    primitives.push(ScenePrimitive::Path {
                        path: Path2D::line(PointPt::new(x_pt, spine_y + o0), PointPt::new(x_pt, spine_y + o1)),
                        fill: None,
                        stroke: Some(tick_stroke),
                        clip: None,
                    });
                }
                if axis.show_tick_labels {
                    let rich = format_tick_rich(val, seg.scale, axis.tick_format, policy);
                    let anchor = if axis.tick_angle_deg.abs() > 15.0 { TextAnchor::End } else { TextAnchor::Middle };
                    primitives.push(ScenePrimitive::Text(PositionedText {
                        rich,
                        anchor_pt: PointPt::new(x_pt, spine_y + 13.0),
                        anchor,
                        font_size_pt: 8.5,
                        rotation_deg: axis.tick_angle_deg,
                        color: Rgba8::BLACK,
                    }));
                }
            }
        }
    }
    major_pts
}

struct YTickAxisCtx<'a> {
    spine_x: f64,
    is_left: bool,
    axis: &'a SegmentedAxis,
    categorical_ticks: Option<&'a [(f64, String)]>,
    policy: &'a LocaleNumberPolicy,
}

fn emit_y_ticks(primitives: &mut Vec<ScenePrimitive>, warnings: &mut Vec<ChartWarning>, ctx: &YTickAxisCtx<'_>) -> Vec<f64> {
    let spine_x = ctx.spine_x;
    let is_left = ctx.is_left;
    let axis = ctx.axis;
    let tick_stroke = StrokePaint::solid(Rgba8::BLACK, axis.stroke_width_pt.max(0.75));
    let dir = if is_left { -1.0 } else { 1.0 };
    let anchor = if is_left { TextAnchor::End } else { TextAnchor::Start };
    let major_span = tick_span_offsets(axis.tick_direction, 4.0, dir);
    let minor_span = tick_span_offsets(axis.tick_direction, 2.2, dir);
    let mut major_pts = Vec::new();

    if let Some(ticks) = ctx.categorical_ticks {
        for (pos_val, label) in ticks {
            if let Some((_, y_pt)) = axis.map_value(*pos_val) {
                major_pts.push(y_pt);
                if let Some((o0, o1)) = major_span {
                    primitives.push(ScenePrimitive::Path {
                        path: Path2D::line(PointPt::new(spine_x + o0, y_pt), PointPt::new(spine_x + o1, y_pt)),
                        fill: None,
                        stroke: Some(tick_stroke),
                        clip: None,
                    });
                }
                if axis.show_tick_labels {
                    primitives.push(ScenePrimitive::Text(PositionedText {
                        rich: parse_markup(label),
                        anchor_pt: PointPt::new(spine_x + dir * 6.5, y_pt + 3.0),
                        anchor,
                        font_size_pt: 8.5,
                        rotation_deg: axis.tick_angle_deg,
                        color: Rgba8::BLACK,
                    }));
                }
            }
        }
        return major_pts;
    }

    for seg in &axis.segments {
        let (majors, minors) = compute_segment_tick_values(seg, None, true, warnings);
        if let Some((o0, o1)) = minor_span {
            for m in minors {
                if let Some(y_pt) = seg.map_strict(m) {
                    primitives.push(ScenePrimitive::Path {
                        path: Path2D::line(PointPt::new(spine_x + o0, y_pt), PointPt::new(spine_x + o1, y_pt)),
                        fill: None,
                        stroke: Some(tick_stroke),
                        clip: None,
                    });
                }
            }
        }
        for val in majors {
            if let Some(y_pt) = seg.map_strict(val) {
                major_pts.push(y_pt);
                if let Some((o0, o1)) = major_span {
                    primitives.push(ScenePrimitive::Path {
                        path: Path2D::line(PointPt::new(spine_x + o0, y_pt), PointPt::new(spine_x + o1, y_pt)),
                        fill: None,
                        stroke: Some(tick_stroke),
                        clip: None,
                    });
                }
                if axis.show_tick_labels {
                    let rich = format_tick_rich(val, seg.scale, axis.tick_format, ctx.policy);
                    primitives.push(ScenePrimitive::Text(PositionedText {
                        rich,
                        anchor_pt: PointPt::new(spine_x + dir * 6.5, y_pt + 3.0),
                        anchor,
                        font_size_pt: 8.5,
                        rotation_deg: axis.tick_angle_deg,
                        color: Rgba8::BLACK,
                    }));
                }
            }
        }
    }
    major_pts
}

pub(crate) fn format_point_value(value: f64) -> String {
    format_axis_tick(value, AxisScale::Linear, &LocaleNumberPolicy::default())
}

/// Legend entry rendered by shared legend emitter.
#[derive(Debug, Clone)]
pub(crate) struct LegendItem {
    pub label: String,
    pub marker: Option<MarkerStyle>,
    pub line_color: Option<Rgba8>,
    pub line_dash: LineDashStyle,
    pub swatch_fill: Option<Rgba8>,
    pub swatch_border: Option<Rgba8>,
}

fn legend_item_to_entry(it: &LegendItem) -> crate::scales::LegendEntry {
    if let Some(fill) = it.swatch_fill {
        crate::scales::LegendEntry {
            label: it.label.clone(),
            swatch_color: fill,
            stroke_color: it.swatch_border.or(it.line_color),
            marker_shape: it.marker.map(|m| m.shape),
            marker_fill_mode: it.marker.map_or(MarkerFillMode::Filled, |m| m.fill_mode),
            dash_style: None,
            is_box_swatch: true,
        }
    } else {
        let swatch_color = it.marker.map_or_else(|| it.line_color.unwrap_or(Rgba8::BLACK), |m| m.fill_color);
        let stroke_color = it.line_color.or_else(|| it.marker.map(|m| m.stroke_color));
        crate::scales::LegendEntry {
            label: it.label.clone(),
            swatch_color,
            stroke_color,
            marker_shape: it.marker.map(|m| m.shape),
            marker_fill_mode: it.marker.map_or(MarkerFillMode::Filled, |m| m.fill_mode),
            dash_style: it.line_color.map(|_| it.line_dash),
            is_box_swatch: false,
        }
    }
}

/// Emit `LegendSpec` and `GraphAnnotation` items via shared `scales` helpers.
pub(crate) fn render_legend_and_annotations(
    scene: &mut ChartScene,
    graph: &GraphSheet,
    doc: &ProjectDocument,
    legend_items: &[LegendItem],
) -> Result<(), ChartError> {
    let _ = render_legend_and_annotations_with_offset(scene, graph, doc, legend_items, 0.0)?;
    Ok(())
}

pub(crate) fn render_legend_and_annotations_with_offset(
    scene: &mut ChartScene,
    graph: &GraphSheet,
    doc: &ProjectDocument,
    legend_items: &[LegendItem],
    outside_right_offset_pt: f64,
) -> Result<Option<RectPt>, ChartError> {
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    let mut budget = crate::scales::AnnotationBudget::default();
    crate::scales::render_annotations_with_budget(&graph.spec.annotations, doc, info_id, &mut budget, scene)?;
    let mut legend_bounds = None;
    if graph.spec.legend.visible && !legend_items.is_empty() {
        let entries: Vec<crate::scales::LegendEntry> = legend_items.iter().map(legend_item_to_entry).collect();
        let has_right_y2 = graph.spec.y2_axis.is_some()
            || graph.spec.series.iter().any(|s| s.bind_to_right_y2)
            || matches!(graph.spec.frame_preset, FramePreset::DualYSecondaryRight);
        let y2_offset = if has_right_y2 { 48.0 } else { 0.0 };
        legend_bounds = crate::scales::render_legend_with_placement(
            &graph.spec.legend,
            &entries,
            doc,
            info_id,
            crate::scales::LegendPlacement { outside_right: true, outside_right_offset_pt: outside_right_offset_pt.max(0.0) + y2_offset },
            scene,
        );
    }
    Ok(legend_bounds)
}

/// Parameters for rendering an embedded live result or data table onto a graph or layout scene.
pub(crate) struct EmbeddedTableDrawSpec<'a> {
    pub sheet_id: &'a SheetId,
    pub tab_id: Option<&'a str>,
    pub bounds: RectPt,
    pub selected_rows: Option<&'a [usize]>,
    pub selected_cols: Option<&'a [usize]>,
    pub style: &'a EmbeddedTableStyle,
    pub missing_warning_code: &'static str,
}

fn embedded_table_rich(text: &str, bold: bool) -> RichText {
    let mut rich = if text.contains('<')
        && !text.contains("</")
        && !text.contains("<sub>")
        && !text.contains("<sup>")
        && !text.contains("<b>")
        && !text.contains("<i>")
    {
        RichText::plain(text)
    } else {
        parse_markup(text)
    };
    if bold {
        for span in &mut rich.spans {
            span.weight = SpanWeight::Bold;
        }
    }
    rich
}

fn resolve_embedded_grid(scene: &mut ChartScene, doc: &ProjectDocument, spec: &EmbeddedTableDrawSpec<'_>) -> Option<crate::survival_cat::SheetGrid> {
    if let Some(res) = doc.result_sheet(spec.sheet_id) {
        if matches!(res.status, ResultStatus::Failed { .. }) {
            scene
                .warnings
                .push(ChartWarning::new("failed_embedded_result_table", format!("result sheet '{}' failed; omitted embedded table", spec.sheet_id)));
            return None;
        }
        if !res.has_displayable_output() {
            scene.warnings.push(ChartWarning::new(
                "pending_embedded_result_table",
                format!("result sheet '{}' is pending recalculation; embedded table omitted", spec.sheet_id),
            ));
            return None;
        }
        if res.tabs.is_empty() && res.scalars.is_empty() {
            scene.warnings.push(ChartWarning::new("empty_embedded_result_table", format!("result sheet '{}' has no tabs", spec.sheet_id)));
            return None;
        }
        let tab = if let Some(tid) = spec.tab_id {
            if let Some(found) = res.tabs.iter().find(|t| t.id == tid || t.title.eq_ignore_ascii_case(tid)) {
                Some(found)
            } else {
                scene.warnings.push(ChartWarning::new(
                    "missing_result_tab",
                    format!("tab '{tid}' not found on result sheet '{}'; fell back to first tab", spec.sheet_id),
                ));
                res.tabs.first()
            }
        } else {
            res.tabs.first()
        };
        if let Some(t) = tab {
            let sig_figs = match &res.analysis.kind {
                curvebook_doc::AnalysisKind::NonlinearRegression(nl) => nl.output_significant_digits,
                _ => None,
            };
            let rows =
                t.rows.iter().map(|r| r.iter().map(|cell| crate::survival_cat::format_result_cell_with_sig_figs(cell, sig_figs)).collect()).collect();
            return Some(crate::survival_cat::SheetGrid { headers: t.column_headers.clone(), row_labels: t.row_labels.clone(), rows });
        }
        if !res.scalars.is_empty() {
            return crate::survival_cat::extract_sheet_grid(doc, spec.sheet_id, spec.tab_id);
        }
    }
    if let Some(grid) = crate::survival_cat::extract_sheet_grid(doc, spec.sheet_id, spec.tab_id) {
        return Some(grid);
    }
    scene.warnings.push(ChartWarning::new(spec.missing_warning_code, format!("referenced table sheet '{}' not found", spec.sheet_id)));
    None
}

struct PreparedEmbeddedGrid {
    headers: Vec<String>,
    row_labels: Vec<String>,
    rows: Vec<Vec<String>>,
    data_col_count: usize,
    show_hdr_row: bool,
    show_row_col: bool,
    total_vis_cols: usize,
}

fn prepare_embedded_table_grid(grid: &crate::survival_cat::SheetGrid, spec: &EmbeddedTableDrawSpec<'_>) -> PreparedEmbeddedGrid {
    let row_indices: Vec<usize> = match spec.selected_rows {
        Some(rs) => rs.iter().copied().filter(|&r| r < grid.rows.len()).collect(),
        None => (0..grid.rows.len()).collect(),
    };
    let max_raw_cols = grid.headers.len().max(grid.rows.iter().map(Vec::len).max().unwrap_or(0));
    let col_indices: Vec<usize> = match spec.selected_cols {
        Some(cs) => cs.iter().copied().filter(|&c| c < max_raw_cols).collect(),
        None => (0..max_raw_cols).collect(),
    };

    let mut headers: Vec<String> = col_indices.iter().map(|&c| grid.headers.get(c).cloned().unwrap_or_default()).collect();
    let mut row_labels: Vec<String> = row_indices.iter().map(|&r| grid.row_labels.get(r).cloned().unwrap_or_default()).collect();
    let mut rows: Vec<Vec<String>> = row_indices
        .iter()
        .map(|&r| col_indices.iter().map(|&c| grid.rows.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default()).collect())
        .collect();

    if spec.style.transpose {
        let old_hdrs = std::mem::take(&mut headers);
        let old_rlbls = std::mem::take(&mut row_labels);
        let old_rows = std::mem::take(&mut rows);
        let n_old_rows = old_rows.len();
        let n_old_cols = old_hdrs.len().max(old_rows.iter().map(Vec::len).max().unwrap_or(0));
        headers = (0..n_old_rows).map(|r| old_rlbls.get(r).cloned().unwrap_or_default()).collect();
        row_labels = (0..n_old_cols).map(|c| old_hdrs.get(c).cloned().unwrap_or_default()).collect();
        rows = (0..n_old_cols)
            .map(|c| (0..n_old_rows).map(|r| old_rows.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default()).collect())
            .collect();
    }

    let data_col_count = headers.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let show_hdr_row = spec.style.show_column_titles && headers.iter().any(|h| !h.trim().is_empty());
    let show_row_col = spec.style.show_row_titles && row_labels.iter().any(|l| !l.trim().is_empty());
    let total_vis_cols = (usize::from(show_row_col) + data_col_count).max(1);
    PreparedEmbeddedGrid { headers, row_labels, rows, data_col_count, show_hdr_row, show_row_col, total_vis_cols }
}

fn compute_embedded_col_xs(
    bounds: RectPt,
    prepared: &PreparedEmbeddedGrid,
    bold_headers: bool,
    max_data_rows: usize,
    font_pt: f64,
) -> (Vec<f64>, Vec<f64>) {
    let mut desired_widths = vec![24.0_f64; prepared.total_vis_cols];
    if prepared.show_row_col {
        let w = prepared
            .row_labels
            .iter()
            .take(max_data_rows)
            .map(|l| measure_text_pt(&embedded_table_rich(l, bold_headers), font_pt).width_pt + 10.0)
            .fold(24.0_f64, f64::max);
        desired_widths[0] = w;
    }
    let dc_offset = usize::from(prepared.show_row_col);
    for c in 0..prepared.data_col_count {
        let mut w = if prepared.show_hdr_row {
            prepared.headers.get(c).map_or(24.0, |h| measure_text_pt(&embedded_table_rich(h, bold_headers), font_pt).width_pt + 10.0)
        } else {
            24.0
        };
        for r in 0..max_data_rows {
            if let Some(cell) = prepared.rows.get(r).and_then(|row| row.get(c)) {
                w = w.max(measure_text_pt(&embedded_table_rich(cell, false), font_pt).width_pt + 10.0);
            }
        }
        if let Some(slot) = desired_widths.get_mut(dc_offset + c) {
            *slot = w;
        }
    }
    let total_desired: f64 = desired_widths.iter().sum::<f64>().max(1.0);
    let col_widths: Vec<f64> = desired_widths.into_iter().map(|w| w * bounds.width / total_desired).collect();
    let mut col_xs = Vec::with_capacity(col_widths.len() + 1);
    let mut cur_x = bounds.x;
    col_xs.push(cur_x);
    for &cw in &col_widths {
        cur_x += cw;
        col_xs.push(cur_x);
    }
    (col_widths, col_xs)
}

/// Shared multi-column embedded table renderer used by all graph annotations and layout live tables.
pub(crate) fn render_styled_embedded_table(
    scene: &mut ChartScene,
    doc: &ProjectDocument,
    spec: &EmbeddedTableDrawSpec<'_>,
    extra_table_work: Option<&mut usize>,
) -> Result<(), ChartError> {
    let bounds = spec.bounds;
    if !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.width < 0.0
        || bounds.height < 0.0
    {
        scene.warnings.push(ChartWarning::new("non_finite_annotation", "skipped embedded table with invalid bounds"));
        return Ok(());
    }
    let Some(grid) = resolve_embedded_grid(scene, doc, spec) else {
        if let Some(extra_work) = extra_table_work.as_deref() {
            crate::multivariate_layout::check_layout_primitive_budget(
                scene.primitives.len().saturating_add(*extra_work),
                1,
                "layout missing table placeholder",
            )?;
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(bounds),
                fill: None,
                stroke: Some(StrokePaint::solid(Rgba8::rgb(160, 160, 160), 0.75)),
                clip: None,
            });
        }
        return Ok(());
    };

    let prepared = prepare_embedded_table_grid(&grid, spec);
    let PreparedEmbeddedGrid { headers, row_labels, rows, data_col_count, show_hdr_row, show_row_col, total_vis_cols } = &prepared;
    let (data_col_count, show_hdr_row, show_row_col, total_vis_cols) = (*data_col_count, *show_hdr_row, *show_row_col, *total_vis_cols);
    let style = spec.style;

    let prims_before = scene.primitives.len();
    let font_pt = if style.font_size_pt.is_finite() && style.font_size_pt > 0.0 { style.font_size_pt } else { 7.5 };
    let row_h = (font_pt * 1.65).max(10.5);
    let max_data_rows = ((bounds.height / row_h).floor() as usize).saturating_sub(usize::from(show_hdr_row)).min(rows.len());
    let est_cells = (max_data_rows.saturating_add(usize::from(show_hdr_row))).saturating_mul(total_vis_cols);
    if let Some(extra_work) = extra_table_work.as_deref() {
        crate::multivariate_layout::check_layout_primitive_budget(scene.primitives.len().saturating_add(*extra_work), est_cells, "layout table")?;
    }

    if bounds.width <= 4.0 || bounds.height <= 4.0 {
        if let Some(extra_work) = extra_table_work {
            *extra_work = extra_work.saturating_add(est_cells);
        }
        return Ok(());
    }

    let dc_offset = usize::from(show_row_col);
    let (col_widths, col_xs) = compute_embedded_col_xs(bounds, &prepared, style.bold_headers, max_data_rows, font_pt);

    scene.primitives.push(ScenePrimitive::PushClip { rect: bounds });
    if let Some(bg) = style.fill_color {
        scene.primitives.push(ScenePrimitive::Path { path: Path2D::rect(bounds), fill: Some(FillPaint::solid(bg)), stroke: None, clip: None });
        if show_hdr_row {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(RectPt::new(bounds.x, bounds.y, bounds.width, row_h)),
                fill: Some(FillPaint::solid(Rgba8::rgb(238, 241, 246))),
                stroke: None,
                clip: None,
            });
        }
    }

    let draw_grid = style.gridline_width_pt.is_finite() && style.gridline_width_pt > 0.0;
    let grid_stroke = if draw_grid {
        let ga = ((u16::from(style.border_color.a) * 140) / 255).max(40) as u8;
        Some(StrokePaint::solid(style.border_color.with_alpha(ga), style.gridline_width_pt))
    } else {
        None
    };

    if show_hdr_row {
        for c in 0..data_col_count {
            let x = col_xs[dc_offset + c] + 4.0;
            let hdr = headers.get(c).map(String::as_str).unwrap_or("");
            if hdr.trim().is_empty() {
                continue;
            }
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: embedded_table_rich(hdr, style.bold_headers),
                anchor_pt: PointPt::new(x, bounds.y + font_pt + 2.5),
                anchor: TextAnchor::Start,
                font_size_pt: font_pt,
                rotation_deg: 0.0,
                color: Rgba8::BLACK,
            }));
        }
    }

    for r in 0..max_data_rows {
        let row_idx_visual = r + usize::from(show_hdr_row);
        let y_top = bounds.y + (row_idx_visual as f64) * row_h;
        if let Some(gs) = grid_stroke
            && row_idx_visual > 0
        {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(bounds.x, y_top), PointPt::new(bounds.right(), y_top)),
                fill: None,
                stroke: Some(gs),
                clip: None,
            });
        }
        let baseline_y = y_top + font_pt + 2.0;
        if show_row_col {
            let lbl = row_labels.get(r).map(String::as_str).unwrap_or("");
            if !lbl.trim().is_empty() {
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: embedded_table_rich(lbl, style.bold_headers),
                    anchor_pt: PointPt::new(col_xs[0] + 4.0, baseline_y),
                    anchor: TextAnchor::Start,
                    font_size_pt: font_pt,
                    rotation_deg: 0.0,
                    color: Rgba8::BLACK,
                }));
            }
        }
        if let Some(row_cells) = rows.get(r) {
            for c in 0..data_col_count {
                let cell_txt = row_cells.get(c).map(String::as_str).unwrap_or("");
                if cell_txt.is_empty() {
                    continue;
                }
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: embedded_table_rich(cell_txt, false),
                    anchor_pt: PointPt::new(col_xs[dc_offset + c] + 4.0, baseline_y),
                    anchor: TextAnchor::Start,
                    font_size_pt: font_pt,
                    rotation_deg: 0.0,
                    color: Rgba8::rgb(25, 25, 25),
                }));
            }
        }
    }

    if let Some(gs) = grid_stroke {
        let vis_rows_total = max_data_rows + usize::from(show_hdr_row);
        let table_h = ((vis_rows_total as f64) * row_h).min(bounds.height);
        for &vx in col_xs.iter().skip(1).take(col_widths.len().saturating_sub(1)) {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(vx, bounds.y), PointPt::new(vx, bounds.y + table_h)),
                fill: None,
                stroke: Some(gs),
                clip: None,
            });
        }
    }
    scene.primitives.push(ScenePrimitive::PopClip);

    if style.outer_border_width_pt.is_finite() && style.outer_border_width_pt > 0.0 {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(bounds),
            fill: None,
            stroke: Some(StrokePaint::solid(style.border_color, style.outer_border_width_pt)),
            clip: None,
        });
    }

    if let Some(extra_work) = extra_table_work {
        let pushed = scene.primitives.len().saturating_sub(prims_before);
        *extra_work = extra_work.saturating_add(est_cells.saturating_sub(pushed));
        crate::multivariate_layout::check_layout_primitive_budget(scene.primitives.len().saturating_add(*extra_work), 0, "layout table")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        Cell, DataSheet, GraphSheet, GraphicalDataObject, GroupedArrangement, PointStyleOverride, ProjectDocument, SheetId, SubcolumnMode,
        TableArchetype, XColumnMode, XyTable,
    };
    use curvebook_palette::{MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};

    use crate::{ScenePrimitive, compile_graph};

    #[test]
    fn xy_point_overrides_and_superimposed_z_order() {
        let mut xy = XyTable::new(SubcolumnMode::Single, 2, 3).expect("xy");
        xy.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single };
        xy.x_cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)], vec![Cell::number(3.0)]];
        xy.data_sets[0].title = String::from("SmallMarkers");
        xy.data_sets[0].cells = vec![vec![Cell::number(5.0)], vec![Cell::number(6.0)], vec![Cell::number(7.0)]];
        xy.data_sets[1].title = String::from("BigMarkers");
        xy.data_sets[1].cells = vec![vec![Cell::number(5.0)], vec![Cell::number(6.0)], vec![Cell::number(7.0)]];

        let ds = DataSheet::new(SheetId::new("xy_1"), "XY", TableArchetype::Xy(xy)).expect("sheet");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_xy"), &ds).expect("graph");
        gs.spec.grouped_arrangement = GroupedArrangement::Superimposed;

        let small_c = Rgba8::rgb(0, 114, 178);
        let big_c = Rgba8::rgb(213, 94, 0);
        gs.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
            marker: Some(MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 4.0,
                fill_color: small_c,
                stroke_color: small_c,
                stroke_width_pt: 0.5,
            }),
            line: None,
            area_fill: None,
            replicate_lines: curvebook_doc::ReplicateLineMode::default(),
        };
        gs.spec.series[1].gdo = GraphicalDataObject::ScatterAndLine {
            marker: Some(MarkerStyle {
                shape: MarkerShape::Square,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 9.0,
                fill_color: big_c,
                stroke_color: big_c,
                stroke_width_pt: 0.5,
            }),
            line: None,
            area_fill: None,
            replicate_lines: curvebook_doc::ReplicateLineMode::default(),
        };

        let highlight = Rgba8::rgb(0, 158, 115);
        gs.spec.point_overrides = vec![
            PointStyleOverride { series_index: 0, point_index: 1, color: Some(highlight), ..PointStyleOverride::default() },
            PointStyleOverride { series_index: 0, point_index: 2, hidden: true, ..PointStyleOverride::default() },
        ];

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add data");
        doc.add_graph_sheet(gs.clone()).expect("add graph");
        let scene = compile_graph(&gs, &doc).expect("compile xy");

        // BigMarkers (size 9.0) must precede SmallMarkers (size 4.0) in plotted_points z-order.
        let cols_in_draw_order: Vec<usize> = scene.plotted_points.iter().map(|p| p.source_cell.col).collect();
        assert_eq!(cols_in_draw_order, vec![1, 1, 1, 0, 0]);
        assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { fill: Some(f), .. } if f.color == highlight)));
    }

    #[test]
    fn log_tick_generator_handles_invalid_and_extreme_bounds() {
        use crate::scales::{MAX_TICKS_PER_SEGMENT, ResolvedSegment, compute_segment_tick_values, resolve_sample_segment_domain, scale_transform};
        use curvebook_doc::AxisSegment;
        use curvebook_math::AxisScale;

        let mk_seg = |domain_min: f64, domain_max: f64, scale: AxisScale| ResolvedSegment {
            segment_index: 0,
            domain_min,
            domain_max,
            scale,
            length_fraction: 1.0,
            span_start_pt: 0.0,
            span_end_pt: 200.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        };

        for (min, max) in [(0.0, 100.0), (-5.0, 100.0), (1.0, f64::INFINITY), (f64::NAN, 100.0)] {
            let mut w = Vec::new();
            let (majors, minors) = compute_segment_tick_values(&mk_seg(min, max, AxisScale::Log10), None, true, &mut w);
            assert!(majors.is_empty() && minors.is_empty());
        }

        for (min, max, scale) in [
            (5e-324, f64::MAX, AxisScale::Log10),
            (5e-324, f64::MAX, AxisScale::Log2),
            (5e-324, f64::MAX, AxisScale::Ln),
            (1e307, f64::MAX, AxisScale::Log10),
            (5e-324, 1e-320, AxisScale::Log10),
        ] {
            let mut w = Vec::new();
            let (majors, minors) = compute_segment_tick_values(&mk_seg(min, max, scale), None, true, &mut w);
            assert!(majors.len() <= MAX_TICKS_PER_SEGMENT);
            assert!(minors.len() <= MAX_TICKS_PER_SEGMENT);
            assert!(majors.iter().all(|v| v.is_finite() && *v > 0.0));
            assert!(minors.iter().all(|v| v.is_finite() && *v > 0.0));
        }

        for scale in [AxisScale::Log10, AxisScale::Log2, AxisScale::Ln] {
            let seg_spec = AxisSegment { scale, ..AxisSegment::default() };
            let mut warnings = Vec::new();
            let (dmin, dmax) = resolve_sample_segment_domain(&seg_spec, &[5e-324, f64::MAX], false, 0.05, &mut warnings);
            assert!(dmin.is_finite() && dmax.is_finite() && dmin > 0.0 && dmin < dmax);
            assert!(scale_transform(scale, dmin).is_some() && scale_transform(scale, dmax).is_some());
        }
    }

    #[test]
    fn rv22_monotone_cubic_and_akima_splines_render_smooth_curves_on_reversed_and_unsorted_x() {
        use curvebook_doc::{ConnectingLineStyle, LineConnectionKind};
        use curvebook_palette::LineDashStyle;

        use crate::PathCmd;

        let make_spline_graph = |rows: &[(f64, f64)], kind: LineConnectionKind, reversed_x: bool| -> crate::ChartScene {
            let mut xy = XyTable::new(SubcolumnMode::Single, 1, rows.len()).expect("xy");
            for (r, &(x, y)) in rows.iter().enumerate() {
                xy.set_x_cell(r, Cell::number(x)).expect("set x");
                xy.data_sets[0].cells[r][0] = Cell::number(y);
            }
            let ds = DataSheet::new(SheetId::new("xy_rv22"), "Spline", TableArchetype::Xy(xy)).expect("ds");
            let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g_rv22"), &ds).expect("gs");
            gs.initialized = true;
            if reversed_x {
                gs.spec.x_axis.segments[0].min = Some(10.0);
                gs.spec.x_axis.segments[0].max = Some(0.0);
            } else {
                gs.spec.x_axis.segments[0].min = Some(0.0);
                gs.spec.x_axis.segments[0].max = Some(10.0);
            }
            gs.spec.y_axis.segments[0].min = Some(0.0);
            gs.spec.y_axis.segments[0].max = Some(100.0);
            gs.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
                marker: None,
                line: Some(ConnectingLineStyle { kind, color: Rgba8::BLACK, stroke_width_pt: 1.5, dash_style: LineDashStyle::Solid }),
                area_fill: None,
                replicate_lines: curvebook_doc::ReplicateLineMode::default(),
            };
            let mut doc = ProjectDocument::default();
            doc.add_data_sheet(ds).expect("add data");
            doc.add_graph_sheet(gs.clone()).expect("add graph");
            compile_graph(&gs, &doc).expect("compile spline graph")
        };

        let sample_pts = [(1.0, 10.0), (3.0, 25.0), (6.0, 75.0), (9.0, 90.0)];
        let unsorted_pts = [(6.0, 75.0), (1.0, 10.0), (9.0, 90.0), (3.0, 25.0)];
        let tied_pts = [(1.0, 10.0), (3.0, 25.0), (3.0, 60.0), (9.0, 90.0)];

        for kind in [LineConnectionKind::MonotonicCubicSpline, LineConnectionKind::AkimaSpline] {
            // Reversed X axis (`[10.0 -> 0.0]`, strictly decreasing scene `x`) must emit 3 `CubicTo` segments (not `LineTo` polyline).
            let rev_scene = make_spline_graph(&sample_pts, kind, true);
            let rev_cubic_paths: Vec<_> = rev_scene
                .primitives
                .iter()
                .filter_map(|p| match p {
                    ScenePrimitive::Path { path, clip: Some(_), stroke: Some(_), fill: None } => Some(path),
                    _ => None,
                })
                .collect();
            assert_eq!(rev_cubic_paths.len(), 1, "expected 1 clipped spline path for {kind:?} on reversed X");
            let cubic_count = rev_cubic_paths[0].commands.iter().filter(|c| matches!(c, PathCmd::CubicTo { .. })).count();
            assert_eq!(cubic_count, 3, "{kind:?} on reversed X fell back to straight segments: {:?}", rev_cubic_paths[0].commands);

            // Unsorted distinct-X rows must sort cleanly and match the forward sorted spline path.
            let fwd_scene = make_spline_graph(&sample_pts, kind, false);
            let uns_scene = make_spline_graph(&unsorted_pts, kind, false);
            let fwd_path = fwd_scene
                .primitives
                .iter()
                .find_map(|p| match p {
                    ScenePrimitive::Path { path, clip: Some(_), stroke: Some(_), fill: None } => Some(path),
                    _ => None,
                })
                .expect("fwd path");
            let uns_path = uns_scene
                .primitives
                .iter()
                .find_map(|p| match p {
                    ScenePrimitive::Path { path, clip: Some(_), stroke: Some(_), fill: None } => Some(path),
                    _ => None,
                })
                .expect("unsorted path");
            assert_eq!(fwd_path.commands, uns_path.commands, "{kind:?} unsorted X rows must match sorted forward spline");

            // Tied X coordinates fall back deterministically to straight `LineTo` segments without NaN/Inf.
            let tied_scene = make_spline_graph(&tied_pts, kind, false);
            let tied_path = tied_scene
                .primitives
                .iter()
                .find_map(|p| match p {
                    ScenePrimitive::Path { path, clip: Some(_), stroke: Some(_), fill: None } => Some(path),
                    _ => None,
                })
                .expect("tied path");
            assert!(tied_path.commands.iter().all(|c| matches!(c, PathCmd::MoveTo(_) | PathCmd::LineTo(_))));
        }
    }
}
