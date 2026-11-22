//! XY scatter, line, error-bar, confidence-band, and fitted regression curve scene builder,
//! plus shared multi-segment axis, tick, frame, marker, error-bar, annotation, and legend kernels.

use curvebook_doc::{
    AxisSpec, AxisThreshold, BreakOrnament, CurveSamplePoint, DataSheet, EmbeddedTableStyle, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec,
    FittedCurveOverlay, FramePreset, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, LineConnectionKind, ProjectDocument,
    RegressionOverlaySpec, ReplicateLineMode, ResultStatus, SeriesSpec, SheetId, SubcolumnMode, SummarizedPoint, SummaryMeasure, TableArchetype,
    TickDirection, TickLabelFormat, XColumnMode, XyTable,
};
use curvebook_math::AxisScale;
use curvebook_math::dist::student_t_two_sided_isf;
use curvebook_math::sum::SummaryMoments;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::common::{quantile_sorted, quartiles_hinge, sorted_finite};
use curvebook_text::{LocaleNumberPolicy, RichText, SpanWeight, format_axis_tick, measure_text_pt, parse_markup};

use crate::scales::{
    AdditionalYTickLayout, AxisOrientation, AxisRenderOptions, ResolvedAdditionalTick, ResolvedAxis, ResolvedSegment,
    compute_additional_ticks_for_segments, compute_segment_tick_values, ellipse_path, emit_additional_tick_gridline_segments,
    emit_additional_x_tick_primitives, emit_additional_y_tick_primitives, format_tick_rich, resolve_exact_domain_with_warnings, scale_inverse,
    scale_transform, tick_span_offsets,
};
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PlottedPointMeta, PointPt, PositionedText, RectPt,
    ScenePrimitive, SourceCellId, StrokePaint, TextAnchor,
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
            let (majors, _) = compute_segment_tick_values(seg, None, true, &mut dummy);
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
    doc.data_sheet(id).or_else(|| {
        doc.result_sheet(id).filter(|res| !matches!(res.status, ResultStatus::Failed { .. })).and_then(|res| res.analyzable_data.as_ref())
    })
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
pub(crate) fn configure_plot_rect(graph: &GraphSheet, scene: &mut ChartScene, has_y2: bool, top_bracket_extra_pt: f64) {
    let w = scene.width_pt;
    let h = scene.height_pt;
    let has_title = !graph.spec.title.trim().is_empty();
    let has_x_title = !graph.spec.x_axis.title.trim().is_empty();
    let has_y_title = !graph.spec.y_axis.title.trim().is_empty();
    let y2_title_len = graph.spec.y2_axis.as_ref().map_or(0, |a| a.title.trim().len());

    let left_base: f64 = if has_y_title { 56.0 } else { 42.0 };
    let right_base: f64 = if has_y2 || y2_title_len > 0 {
        56.0
    } else if graph.spec.legend.visible && graph.spec.legend.position_pt.is_none() && !graph.spec.series.is_empty() {
        86.0
    } else {
        20.0
    };
    let top_base: f64 = if has_title { 32.0 } else { 18.0 } + top_bracket_extra_pt.max(0.0);
    let rot_extra: f64 = if graph.spec.x_axis.tick_angle_deg.abs() > 15.0 { 16.0 } else { 0.0 };
    let bottom_base: f64 = if has_x_title { 44.0 } else { 28.0 } + rot_extra;

    let left = left_base.min(w * 0.28);
    let right = right_base.min(w * 0.30);
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

/// Emit any of the 11 `MarkerShape`s and 6 `MarkerFillMode`s at `center`.
pub(crate) fn emit_marker(primitives: &mut Vec<ScenePrimitive>, center: PointPt, style: &MarkerStyle, clip: Option<RectPt>) {
    let r = (style.size_pt * 0.5).max(0.5);
    let path = build_marker_path(center, r, style.shape);
    let stroke = (style.stroke_width_pt > 0.0).then(|| StrokePaint::solid(style.stroke_color, style.stroke_width_pt));

    if is_open_stroke_shape(style.shape) {
        let line_stroke = Some(StrokePaint::solid(style.stroke_color, style.stroke_width_pt.max(1.0)));
        primitives.push(ScenePrimitive::Path { path, fill: None, stroke: line_stroke, clip });
        return;
    }

    match style.fill_mode {
        MarkerFillMode::Filled => {
            primitives.push(ScenePrimitive::Path { path, fill: Some(FillPaint::solid(style.fill_color)), stroke, clip });
        }
        MarkerFillMode::Open => {
            primitives.push(ScenePrimitive::Path {
                path,
                fill: None,
                stroke: stroke.or_else(|| Some(StrokePaint::solid(style.stroke_color, 1.0))),
                clip,
            });
        }
        MarkerFillMode::SemiFilledTop | MarkerFillMode::SemiFilledBottom | MarkerFillMode::SemiFilledLeft | MarkerFillMode::SemiFilledRight => {
            let half_rect = semi_fill_rect(center, r, style.fill_mode);
            let combined_clip = clip.map_or(half_rect, |c| intersect_rects(c, half_rect));
            primitives.push(ScenePrimitive::Path {
                path: path.clone(),
                fill: Some(FillPaint::solid(style.fill_color)),
                stroke: None,
                clip: Some(combined_clip),
            });
            primitives.push(ScenePrimitive::Path {
                path,
                fill: None,
                stroke: stroke.or_else(|| Some(StrokePaint::solid(style.stroke_color, 1.0))),
                clip,
            });
        }
    }
}

fn is_open_stroke_shape(shape: MarkerShape) -> bool {
    matches!(shape, MarkerShape::Cross | MarkerShape::Plus | MarkerShape::HorizontalBar | MarkerShape::VerticalBar)
}

fn semi_fill_rect(center: PointPt, r: f64, mode: MarkerFillMode) -> RectPt {
    let pad = r + 1.0;
    match mode {
        MarkerFillMode::SemiFilledTop => RectPt::new(center.x - pad, center.y - pad, 2.0 * pad, pad),
        MarkerFillMode::SemiFilledBottom => RectPt::new(center.x - pad, center.y, 2.0 * pad, pad),
        MarkerFillMode::SemiFilledLeft => RectPt::new(center.x - pad, center.y - pad, pad, 2.0 * pad),
        MarkerFillMode::SemiFilledRight => RectPt::new(center.x, center.y - pad, pad, 2.0 * pad),
        MarkerFillMode::Filled | MarkerFillMode::Open => RectPt::new(center.x - pad, center.y - pad, 2.0 * pad, 2.0 * pad),
    }
}

pub(crate) fn intersect_rects(a: RectPt, b: RectPt) -> RectPt {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = a.right().min(b.right());
    let y1 = a.bottom().min(b.bottom());
    RectPt::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

fn build_marker_path(c: PointPt, r: f64, shape: MarkerShape) -> Path2D {
    match shape {
        MarkerShape::Circle => ellipse_path(RectPt::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r)),
        MarkerShape::Square => Path2D::rect(RectPt::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r)),
        MarkerShape::TriangleUp => polygon_path(&[
            PointPt::new(c.x, c.y - r),
            PointPt::new(c.x + 0.866_025 * r, c.y + 0.5 * r),
            PointPt::new(c.x - 0.866_025 * r, c.y + 0.5 * r),
        ]),
        MarkerShape::TriangleDown => polygon_path(&[
            PointPt::new(c.x, c.y + r),
            PointPt::new(c.x + 0.866_025 * r, c.y - 0.5 * r),
            PointPt::new(c.x - 0.866_025 * r, c.y - 0.5 * r),
        ]),
        MarkerShape::Diamond => {
            polygon_path(&[PointPt::new(c.x, c.y - r), PointPt::new(c.x + r, c.y), PointPt::new(c.x, c.y + r), PointPt::new(c.x - r, c.y)])
        }
        MarkerShape::Hexagon => {
            let mut pts = Vec::with_capacity(6);
            for i in 0..6 {
                let ang = (i as f64) * std::f64::consts::FRAC_PI_3 - std::f64::consts::FRAC_PI_6;
                pts.push(PointPt::new(c.x + r * ang.cos(), c.y + r * ang.sin()));
            }
            polygon_path(&pts)
        }
        MarkerShape::Star => {
            let mut pts = Vec::with_capacity(10);
            for i in 0..10 {
                let ang = (i as f64) * std::f64::consts::PI / 5.0 - std::f64::consts::FRAC_PI_2;
                let rad = if i % 2 == 0 { r } else { r * 0.42 };
                pts.push(PointPt::new(c.x + rad * ang.cos(), c.y + rad * ang.sin()));
            }
            polygon_path(&pts)
        }
        MarkerShape::Cross => {
            let d = r * 0.72;
            Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(c.x - d, c.y - d)),
                    PathCmd::LineTo(PointPt::new(c.x + d, c.y + d)),
                    PathCmd::MoveTo(PointPt::new(c.x - d, c.y + d)),
                    PathCmd::LineTo(PointPt::new(c.x + d, c.y - d)),
                ],
            }
        }
        MarkerShape::Plus => Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(c.x - r, c.y)),
                PathCmd::LineTo(PointPt::new(c.x + r, c.y)),
                PathCmd::MoveTo(PointPt::new(c.x, c.y - r)),
                PathCmd::LineTo(PointPt::new(c.x, c.y + r)),
            ],
        },
        MarkerShape::HorizontalBar => Path2D::line(PointPt::new(c.x - r, c.y), PointPt::new(c.x + r, c.y)),
        MarkerShape::VerticalBar => Path2D::line(PointPt::new(c.x, c.y - r), PointPt::new(c.x, c.y + r)),
    }
}

pub(crate) fn polygon_path(pts: &[PointPt]) -> Path2D {
    let Some(&first) = pts.first() else {
        return Path2D::new();
    };
    let mut commands = Vec::with_capacity(pts.len() + 1);
    commands.push(PathCmd::MoveTo(first));
    for &p in pts.iter().skip(1) {
        commands.push(PathCmd::LineTo(p));
    }
    commands.push(PathCmd::Close);
    Path2D { commands }
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
    let grid_color = AxisRenderOptions::default().grid_color;

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

    emit_frame_spines(&mut scene.primitives, r, graph.spec.frame_preset, x_axis, y_axis, y2_axis, (x_spine_y, y_spine_x), &x_major_pts, &y_major_pts);

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

#[allow(clippy::too_many_arguments)]
fn emit_frame_spines(
    primitives: &mut Vec<ScenePrimitive>,
    r: RectPt,
    preset: FramePreset,
    x_axis: &SegmentedAxis,
    y_axis: &SegmentedAxis,
    y2_axis: Option<&SegmentedAxis>,
    spine_xy: (f64, f64),
    x_major_pts: &[f64],
    y_major_pts: &[f64],
) {
    let (x_spine_y, y_spine_x) = spine_xy;
    let x_stroke = StrokePaint::solid(Rgba8::BLACK, x_axis.stroke_width_pt);
    let y_stroke = StrokePaint::solid(Rgba8::BLACK, y_axis.stroke_width_pt);

    match preset {
        FramePreset::OpenL => {
            draw_segmented_spine(primitives, x_spine_y, true, x_axis, x_stroke, None);
            draw_segmented_spine(primitives, y_spine_x, false, y_axis, y_stroke, None);
        }
        FramePreset::OffsetRangeTufte => {
            draw_segmented_spine(primitives, x_spine_y, true, x_axis, x_stroke, Some(x_major_pts));
            draw_segmented_spine(primitives, y_spine_x, false, y_axis, y_stroke, Some(y_major_pts));
        }
        FramePreset::PlainBoxed => {
            draw_segmented_spine(primitives, r.bottom(), true, x_axis, x_stroke, None);
            draw_segmented_spine(primitives, r.y, true, x_axis, x_stroke, None);
            draw_segmented_spine(primitives, r.x, false, y_axis, y_stroke, None);
            draw_segmented_spine(primitives, r.right(), false, y2_axis.unwrap_or(y_axis), y_stroke, None);
        }
        FramePreset::DualYSecondaryRight => {
            draw_segmented_spine(primitives, r.bottom(), true, x_axis, x_stroke, None);
            draw_segmented_spine(primitives, r.x, false, y_axis, y_stroke, None);
            draw_segmented_spine(primitives, r.right(), false, y2_axis.unwrap_or(y_axis), y_stroke, None);
        }
        FramePreset::OriginCrossing => {
            draw_segmented_spine(primitives, x_spine_y, true, x_axis, x_stroke, None);
            draw_segmented_spine(primitives, y_spine_x, false, y_axis, y_stroke, None);
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
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    let mut budget = crate::scales::AnnotationBudget::default();
    crate::scales::render_annotations_with_budget(&graph.spec.annotations, doc, info_id, &mut budget, scene)?;
    if graph.spec.legend.visible && !legend_items.is_empty() {
        let entries: Vec<crate::scales::LegendEntry> = legend_items.iter().map(legend_item_to_entry).collect();
        crate::scales::render_legend_with_placement(&graph.spec.legend, &entries, doc, info_id, true, scene);
    }
    Ok(())
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
            let rows = t.rows.iter().map(|r| r.iter().map(crate::survival_cat::format_result_cell).collect()).collect();
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

#[derive(Debug, Clone)]
struct FittedOverlayData {
    spec: RegressionOverlaySpec,
    curve: FittedCurveOverlay,
}

/// Compile an XY graph sheet into vector primitives inside `scene`.
pub fn compile_xy_graph(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    if resolve_data_sheet(doc, &graph.spec.primary_source_sheet_id).is_none() {
        scene
            .warnings
            .push(ChartWarning::new("missing_source_sheet", format!("primary data sheet '{}' not found", graph.spec.primary_source_sheet_id)));
    }

    let has_y2_series = graph.spec.series.iter().any(|s| s.bind_to_right_y2);
    let has_y2 = graph.spec.y2_axis.is_some() || has_y2_series || matches!(graph.spec.frame_preset, FramePreset::DualYSecondaryRight);
    configure_plot_rect(graph, scene, has_y2, 0.0);

    let extracted = extract_all_xy_series(graph, doc, &mut scene.warnings);
    let (x_vals, y_vals, y2_vals) = collect_xy_domain_values(&graph.spec, &extracted);
    let has_bar_y =
        graph.spec.series.iter().any(|s| !s.bind_to_right_y2 && s.show_raw_data && matches!(s.gdo, GraphicalDataObject::BarWithReplicates { .. }));
    let has_bar_y2 =
        graph.spec.series.iter().any(|s| s.bind_to_right_y2 && s.show_raw_data && matches!(s.gdo, GraphicalDataObject::BarWithReplicates { .. }));

    let r = scene.plot_rect_pt;
    let x_axis = SegmentedAxis::build(&graph.spec.x_axis, r.x, r.right(), &x_vals, false, 0.0);
    let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &y_vals, has_bar_y, 0.05);
    let y2_axis = if has_y2 {
        let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2"));
        let source = if y2_vals.is_empty() { &y_vals } else { &y2_vals };
        Some(SegmentedAxis::build(&y2_spec, r.bottom(), r.y, source, has_bar_y2, 0.05))
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
    for idx in render_order {
        let series_spec = &graph.spec.series[idx];
        let data = &extracted[idx];
        let target_y = if series_spec.bind_to_right_y2 { y2_axis.as_ref().unwrap_or(&y_axis) } else { &y_axis };
        render_xy_series(scene, &graph.spec, series_spec, data, &x_axis, target_y);
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

    render_legend_and_annotations(scene, graph, doc, &legend_items)?;
    Ok(())
}

fn extract_all_xy_series(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Vec<ExtractedXySeries> {
    let mut out = Vec::with_capacity(graph.spec.series.len());
    let palette = graph.spec.palette;
    for (idx, series_spec) in graph.spec.series.iter().enumerate() {
        let color = palette.color_at(idx);
        let sheet_opt = resolve_data_sheet(doc, &series_spec.source_sheet_id);
        if sheet_opt.is_none() && series_spec.source_sheet_id != graph.spec.primary_source_sheet_id {
            warnings.push(ChartWarning::new("missing_source_sheet", format!("series source sheet '{}' not found", series_spec.source_sheet_id)));
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
        out.push(ExtractedXySeries {
            sheet_id,
            col_index: series_spec.data_set_index,
            active_x_rows,
            points,
            replicates,
            has_x_err,
            has_y_err,
            label: series_spec.legend_label.clone().unwrap_or(fallback_label),
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

fn resolve_fitted_overlay<'a>(
    doc: &'a ProjectDocument,
    series: &SeriesSpec,
    warnings: &mut Vec<ChartWarning>,
) -> Option<(FittedOverlayData, &'a [SourceCellId])> {
    let spec = series.regression_overlay.as_ref()?;
    let Some(res) = doc.result_sheet(&spec.result_sheet_id) else {
        warnings.push(ChartWarning::new("missing_regression_result", format!("regression result sheet '{}' not found", spec.result_sheet_id)));
        return None;
    };
    if let ResultStatus::Failed { message, .. } = &res.status {
        warnings.push(ChartWarning::new("failed_regression_result", format!("regression result sheet '{}' failed: {message}", spec.result_sheet_id)));
        return None;
    }
    let Some(curve) = res.fitted_curves.iter().find(|fc| fc.data_set_index == series.data_set_index) else {
        warnings.push(ChartWarning::new(
            "missing_fitted_curve",
            format!("regression result sheet '{}' has no fitted curve for data set {}", spec.result_sheet_id, series.data_set_index),
        ));
        return None;
    };
    if !curve.converged {
        warnings.push(ChartWarning::new("unconverged_fitted_curve", format!("fitted curve '{}' did not converge", curve.equation_id)));
    } else if curve.ambiguous {
        warnings.push(ChartWarning::new("ambiguous_fitted_curve", format!("fitted curve '{}' has ambiguous parameters", curve.equation_id)));
    }
    let mut effective_spec = spec.clone();
    let mut effective_curve = curve.clone();
    if matches!(&res.analysis.kind, curvebook_doc::AnalysisKind::NonlinearRegression(nl) if nl.method == curvebook_doc::FitMethod::RobustCauchy) {
        if effective_spec.show_confidence_band || effective_spec.show_prediction_band {
            warnings.push(ChartWarning::new(
                "robust_bands_suppressed",
                format!(
                    "confidence and prediction bands suppressed for Robust (Cauchy) regression result '{}' (NotApplicableInRobustMode)",
                    spec.result_sheet_id
                ),
            ));
        }
        effective_spec.show_confidence_band = false;
        effective_spec.show_prediction_band = false;
        for pt in &mut effective_curve.points {
            pt.ci_lower = None;
            pt.ci_upper = None;
            pt.pred_lower = None;
            pt.pred_upper = None;
        }
    }
    let outliers = if matches!(res.status, ResultStatus::UpToDate) { res.eliminated_outliers.as_slice() } else { &[] };
    Some((FittedOverlayData { spec: effective_spec, curve: effective_curve }, outliers))
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
            let x = xy.x_value_at(source.row)?;
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
        if s_spec.show_raw_data {
            let dx = series_data_x_offset(s_spec);
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
                x_vals.push(cp.x);
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

    // 1. Fitted regression curve overlay & bands (always rendered even when `show_raw_data == false`)
    if let Some(ov) = &data.fitted_overlay {
        let curve_color = line_opt.map_or_else(|| marker_opt.map_or(data.default_color, |m| m.stroke_color), |l| l.color);
        render_fitted_curve_overlay(&mut scene.primitives, &mut scene.warnings, ov, x_axis, y_axis, curve_color, clip);
    }

    if !spec.show_raw_data {
        return;
    }

    let dx_data = series_data_x_offset(spec);
    let (dx_pt, dy_pt) = series_pt_offset(spec);

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
        bars_emitted_points = render_xy_bars(scene, graph_spec, spec, data, y_axis, &summary_markers, dy_pt, clip);
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
        render_replicate_trajectories(scene, data, x_axis, y_axis, &line_style, (dx_data, dx_pt, dy_pt), !markers_emit_replicates, clip);
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
    emit_rout_outlier_markers(scene, data, x_axis, y_axis, first_series_point, dx_data, dx_pt, dy_pt);
}

#[allow(clippy::too_many_arguments)]
fn render_xy_bars(
    scene: &mut ChartScene,
    graph_spec: &GraphSpec,
    spec: &SeriesSpec,
    data: &ExtractedXySeries,
    y_axis: &SegmentedAxis,
    summary_markers: &[(PointPt, usize, f64, f64)],
    dy_pt: f64,
    clip: RectPt,
) -> bool {
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
            let Some(marker) = crate::resolve_overridden_marker_for_cell(graph_spec, data.col_index, row, Some(subcol), dot_style) else {
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

#[allow(clippy::too_many_arguments)]
fn render_replicate_trajectories(
    scene: &mut ChartScene,
    data: &ExtractedXySeries,
    x_axis: &SegmentedAxis,
    y_axis: &SegmentedAxis,
    line_style: &curvebook_doc::ConnectingLineStyle,
    offsets: (f64, f64, f64),
    emit_meta: bool,
    clip: RectPt,
) {
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

#[allow(clippy::too_many_arguments)]
fn emit_rout_outlier_markers(
    scene: &mut ChartScene,
    data: &ExtractedXySeries,
    x_axis: &SegmentedAxis,
    y_axis: &SegmentedAxis,
    first_series_point: usize,
    dx_data: f64,
    dx_pt: f64,
    dy_pt: f64,
) {
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
            let Some(marker) = crate::resolve_overridden_marker_for_cell(ctx.spec, data.col_index, row, Some(subcol), ctx.marker) else {
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
            let Some(marker) = crate::resolve_overridden_marker_for_cell(ctx.spec, data.col_index, row, Some(0), ctx.marker) else {
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

fn build_connecting_path(run: &[PointPt], kind: LineConnectionKind) -> Option<Path2D> {
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

fn render_fitted_curve_overlay(
    primitives: &mut Vec<ScenePrimitive>,
    warnings: &mut Vec<ChartWarning>,
    overlay: &FittedOverlayData,
    x_axis: &SegmentedAxis,
    y_axis: &SegmentedAxis,
    color: Rgba8,
    clip: RectPt,
) {
    let mut src: Vec<CurveSamplePoint> = overlay.curve.points.iter().copied().filter(|p| p.x.is_finite() && p.y.is_finite()).collect();
    if src.len() < 2 {
        return;
    }
    src.sort_by(|a, b| a.x.total_cmp(&b.x));
    let total_samples = usize::from(overlay.spec.sample_segments.clamp(256, 1024));
    let total_x_span: f64 = x_axis.segments.iter().map(|s| (s.span_end_pt - s.span_start_pt).abs()).sum::<f64>().max(1.0);

    for seg_x in &x_axis.segments {
        let seg_weight = ((seg_x.span_end_pt - seg_x.span_start_pt).abs() / total_x_span).clamp(0.1, 1.0);
        let seg_samples = ((total_samples as f64) * seg_weight).round().max(16.0) as usize;
        let resampled = resample_curve_for_x_segment(&src, seg_x, seg_samples);
        if resampled.len() < 2 {
            continue;
        }
        let ctx = OverlayRenderCtx { spec: &overlay.spec, ambiguous: overlay.curve.ambiguous, color, clip };
        emit_overlay_bands_and_curve(primitives, warnings, &resampled, seg_x, y_axis, ctx);
    }
}

fn resample_curve_for_x_segment(src: &[CurveSamplePoint], seg_x: &ResolvedSegment, target_n: usize) -> Vec<CurveSamplePoint> {
    let Some(first_x) = src.first().map(|p| p.x) else {
        return Vec::new();
    };
    let Some(last_x) = src.last().map(|p| p.x) else {
        return Vec::new();
    };
    let Some(g_seg_min) = scale_transform(seg_x.scale, seg_x.domain_min) else {
        return Vec::new();
    };
    let Some(g_seg_max) = scale_transform(seg_x.scale, seg_x.domain_max) else {
        return Vec::new();
    };

    let seg_lo = g_seg_min.min(g_seg_max);
    let seg_hi = g_seg_min.max(g_seg_max);
    let (lo_t, hi_t) = if matches!(seg_x.scale, AxisScale::Reciprocal) && first_x * last_x <= 0.0 {
        if seg_x.domain_min > 0.0 {
            if last_x <= 0.0 {
                return Vec::new();
            }
            let Some(gx1) = scale_transform(seg_x.scale, last_x) else {
                return Vec::new();
            };
            (gx1.max(seg_lo), seg_hi)
        } else {
            if first_x >= 0.0 {
                return Vec::new();
            }
            let Some(gx0) = scale_transform(seg_x.scale, first_x) else {
                return Vec::new();
            };
            (seg_lo, gx0.min(seg_hi))
        }
    } else if matches!(seg_x.scale, AxisScale::Reciprocal) {
        if first_x * seg_x.domain_min <= 0.0 {
            return Vec::new();
        }
        match (scale_transform(seg_x.scale, first_x), scale_transform(seg_x.scale, last_x)) {
            (Some(gx0), Some(gx1)) => (gx0.min(gx1).max(seg_lo), gx0.max(gx1).min(seg_hi)),
            _ => return Vec::new(),
        }
    } else {
        match (scale_transform(seg_x.scale, first_x), scale_transform(seg_x.scale, last_x)) {
            (Some(gx0), Some(gx1)) => (gx0.min(gx1).max(seg_lo), gx0.max(gx1).min(seg_hi)),
            (None, Some(gx1)) => (seg_lo, gx1.min(seg_hi)),
            (Some(gx0), None) => (gx0.max(seg_lo), seg_hi),
            (None, None) => (seg_lo, seg_hi),
        }
    };
    if hi_t <= lo_t {
        return Vec::new();
    }

    let n = target_n.max(2);
    let use_log_interp = is_log_like(seg_x.scale) && src.iter().all(|p| p.x > 0.0);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let frac = (i as f64) / ((n - 1) as f64);
        let t = lo_t + frac * (hi_t - lo_t);
        let Some(x) = scale_inverse(seg_x.scale, t) else {
            continue;
        };
        if let Some(pt) = interpolate_curve_sample(src, x, use_log_interp) {
            out.push(pt);
        }
    }
    out
}

fn interpolate_curve_sample(src: &[CurveSamplePoint], x: f64, use_log_x: bool) -> Option<CurveSamplePoint> {
    let first = src.first()?;
    let last = src.last()?;
    if x < first.x - 1e-12 || x > last.x + 1e-12 {
        return None;
    }
    let idx = src.partition_point(|p| p.x <= x);
    if idx == 0 {
        return Some(*first);
    }
    if idx >= src.len() {
        return Some(*last);
    }
    let a = src[idx - 1];
    let b = src[idx];
    let w = if use_log_x && a.x > 0.0 && b.x > 0.0 && x > 0.0 {
        let span = b.x.ln() - a.x.ln();
        if span.abs() > 1e-15 { ((x.ln() - a.x.ln()) / span).clamp(0.0, 1.0) } else { 0.0 }
    } else {
        let span = b.x - a.x;
        if span.abs() > 1e-15 { ((x - a.x) / span).clamp(0.0, 1.0) } else { 0.0 }
    };
    let lerp = |va: f64, vb: f64| va + w * (vb - va);
    let lerp_opt = |oa: Option<f64>, ob: Option<f64>| match (oa, ob) {
        (Some(va), Some(vb)) => Some(lerp(va, vb)),
        _ => None,
    };
    Some(CurveSamplePoint {
        x,
        y: lerp(a.y, b.y),
        ci_lower: lerp_opt(a.ci_lower, b.ci_lower),
        ci_upper: lerp_opt(a.ci_upper, b.ci_upper),
        pred_lower: lerp_opt(a.pred_lower, b.pred_lower),
        pred_upper: lerp_opt(a.pred_upper, b.pred_upper),
    })
}

#[derive(Clone, Copy)]
struct OverlayRenderCtx<'a> {
    spec: &'a RegressionOverlaySpec,
    ambiguous: bool,
    color: Rgba8,
    clip: RectPt,
}

#[derive(Default)]
struct OverlayBandBuffers {
    pred: Vec<(PointPt, PointPt)>,
    ci: Vec<(PointPt, PointPt)>,
    curve: Vec<PointPt>,
}

fn emit_overlay_bands_and_curve(
    primitives: &mut Vec<ScenePrimitive>,
    warnings: &mut Vec<ChartWarning>,
    samples: &[CurveSamplePoint],
    seg_x: &ResolvedSegment,
    y_axis: &SegmentedAxis,
    ctx: OverlayRenderCtx<'_>,
) {
    // Split runs cleanly per Y AxisSegment so discontinuous Y segments never join across gaps.
    let mut bufs = OverlayBandBuffers::default();
    let mut active_y_seg: Option<usize> = None;
    let mut warned_clip = false;

    for sp in samples {
        let Some(px) = seg_x.map_strict(sp.x) else {
            continue;
        };
        let mapped_y = y_axis.map_value(sp.y);
        if let Some((sy, py)) = mapped_y {
            if active_y_seg.is_some() && active_y_seg != Some(sy) {
                flush_overlay_run(primitives, &mut bufs, ctx);
            }
            active_y_seg = Some(sy);
            bufs.curve.push(PointPt::new(px, py));
            if let Some(y_seg) = y_axis.segments.get(sy) {
                if ctx.spec.show_prediction_band
                    && let (Some(lo), Some(hi)) = (sp.pred_lower, sp.pred_upper)
                {
                    let (py_lo, c1) = y_seg.map_clamped(lo);
                    let (py_hi, c2) = y_seg.map_clamped(hi);
                    if (c1 || c2) && !warned_clip && is_log_like(y_seg.scale) && (lo <= 0.0 || hi <= 0.0) {
                        warned_clip = true;
                        warnings.push(ChartWarning::new("log_axis_floor_clip", "prediction band clipped cleanly to log-axis floor"));
                    }
                    bufs.pred.push((PointPt::new(px, py_lo), PointPt::new(px, py_hi)));
                }
                if ctx.spec.show_confidence_band
                    && let (Some(lo), Some(hi)) = (sp.ci_lower, sp.ci_upper)
                {
                    let (py_lo, c1) = y_seg.map_clamped(lo);
                    let (py_hi, c2) = y_seg.map_clamped(hi);
                    if (c1 || c2) && !warned_clip && is_log_like(y_seg.scale) && (lo <= 0.0 || hi <= 0.0) {
                        warned_clip = true;
                        warnings.push(ChartWarning::new("log_axis_floor_clip", "confidence band clipped cleanly to log-axis floor"));
                    }
                    bufs.ci.push((PointPt::new(px, py_lo), PointPt::new(px, py_hi)));
                }
            }
        } else {
            flush_overlay_run(primitives, &mut bufs, ctx);
            active_y_seg = None;
        }
    }
    flush_overlay_run(primitives, &mut bufs, ctx);
}

fn flush_overlay_run(primitives: &mut Vec<ScenePrimitive>, bufs: &mut OverlayBandBuffers, ctx: OverlayRenderCtx<'_>) {
    emit_band_geometry(primitives, &bufs.pred, ctx.spec.shaded_ribbon, ctx.color.with_alpha(38), LineDashStyle::Dotted, ctx.clip);
    emit_band_geometry(primitives, &bufs.ci, ctx.spec.shaded_ribbon, ctx.color.with_alpha(68), LineDashStyle::Dashed, ctx.clip);
    if bufs.curve.len() >= 2
        && let Some(path) = build_connecting_path(&bufs.curve, LineConnectionKind::PointToPoint)
    {
        let dash = if ctx.ambiguous { LineDashStyle::Dashed } else { LineDashStyle::Solid };
        primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke_with_dash(ctx.color, 1.75, dash)), clip: Some(ctx.clip) });
    }
    bufs.pred.clear();
    bufs.ci.clear();
    bufs.curve.clear();
}

fn emit_band_geometry(
    primitives: &mut Vec<ScenePrimitive>,
    pairs: &[(PointPt, PointPt)],
    shaded: bool,
    fill_color: Rgba8,
    dash: LineDashStyle,
    clip: RectPt,
) {
    if pairs.len() < 2 {
        return;
    }
    if shaded {
        let mut cmds = Vec::with_capacity(pairs.len() * 2 + 1);
        cmds.push(PathCmd::MoveTo(pairs[0].1));
        for &(_, upper) in &pairs[1..] {
            cmds.push(PathCmd::LineTo(upper));
        }
        for &(lower, _) in pairs.iter().rev() {
            cmds.push(PathCmd::LineTo(lower));
        }
        cmds.push(PathCmd::Close);
        primitives.push(ScenePrimitive::Path {
            path: Path2D { commands: cmds },
            fill: Some(FillPaint::solid(fill_color)),
            stroke: None,
            clip: Some(clip),
        });
    } else {
        let stroke = Some(stroke_with_dash(fill_color.with_alpha(180), 0.9, dash));
        let lo_pts: Vec<PointPt> = pairs.iter().map(|p| p.0).collect();
        let hi_pts: Vec<PointPt> = pairs.iter().map(|p| p.1).collect();
        for pts in [lo_pts, hi_pts] {
            if let Some(path) = build_connecting_path(&pts, LineConnectionKind::PointToPoint) {
                primitives.push(ScenePrimitive::Path { path, fill: None, stroke, clip: Some(clip) });
            }
        }
    }
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
