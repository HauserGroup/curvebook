//! XY scatter, line, error-bar, confidence-band, and fitted regression curve scene builder,
//! plus shared multi-segment axis, tick, frame, marker, error-bar, annotation, and legend kernels.

use curvebook_doc::{
    AxisSpec, AxisThreshold, BreakOrnament, CurveSamplePoint, DataSheet, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FittedCurveOverlay,
    FramePreset, GraphAnnotation, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, LineConnectionKind, ProjectDocument,
    RegressionOverlaySpec, ResultCell, ResultStatus, SeriesSpec, SheetId, SubcolumnMode, SummarizedPoint, SummaryMeasure, TableArchetype,
    TickLabelFormat, XColumnMode, XyTable,
};
use curvebook_math::AxisScale;
use curvebook_math::dist::student_t_two_sided_isf;
use curvebook_math::sum::SummaryMoments;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::common::{quantile_sorted, quartiles_hinge, sorted_finite};
use curvebook_text::{LocaleNumberPolicy, RichText, expand_template_hooks, format_axis_tick, measure_text_pt, parse_markup};

use crate::scales::{
    AxisOrientation, AxisRenderOptions, ResolvedAdditionalTick, ResolvedAxis, ResolvedSegment, compute_additional_ticks_for_segments,
    compute_segment_tick_values, emit_additional_tick_gridline_segments, emit_additional_x_tick_primitives, emit_additional_y_tick_primitives,
    format_tick_rich, scale_inverse, scale_transform,
};
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PlottedPointMeta, PointPt, PositionedText, RectPt,
    ScenePrimitive, SourceCellId, StrokePaint, TextAnchor,
};

const KAPPA: f64 = 0.552_284_749_830_793_6;

/// Complete multi-segment horizontal or vertical axis layout backed by `scales::ResolvedAxis`.
#[derive(Debug, Clone)]
pub(crate) struct SegmentedAxis {
    pub segments: Vec<ResolvedSegment>,
    pub break_ornament: BreakOrnament,
    pub tick_format: TickLabelFormat,
    pub tick_angle_deg: f64,
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
            stroke_width_pt: if spec.stroke_width_pt > 0.0 { spec.stroke_width_pt } else { 1.0 },
            warnings,
        }
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
    let expanded = expand_template_hooks(raw, |token| resolve_hook_token(token, doc, primary_info_id));
    parse_markup(&expanded)
}

fn resolve_hook_token(token: &str, doc: &ProjectDocument, primary_info_id: Option<&SheetId>) -> Option<String> {
    let parts: Vec<&str> = token.split(':').map(str::trim).collect();
    match parts.as_slice() {
        ["info", name] if !name.is_empty() => {
            let sheet = primary_info_id.and_then(|id| doc.info_sheet(id)).or_else(|| doc.info_sheets.first())?;
            let constant = sheet.get_constant(name)?;
            let text = constant.value.display_text();
            if text.is_empty() { None } else { Some(text) }
        }
        ["info", sheet_id, name] if !sheet_id.is_empty() && !name.is_empty() => {
            let sheet = doc.info_sheet(&SheetId::new(*sheet_id))?;
            let text = sheet.get_constant(name)?.value.display_text();
            if text.is_empty() { None } else { Some(text) }
        }
        ["result", sheet_id, key] if !sheet_id.is_empty() && !key.is_empty() => {
            let sheet = doc.result_sheet(&SheetId::new(*sheet_id))?;
            let val = sheet.lookup_scalar(key, None)?;
            Some(format_axis_tick(val, AxisScale::Linear, &LocaleNumberPolicy::default()))
        }
        ["result", sheet_id, ds_str, key] if !sheet_id.is_empty() && !key.is_empty() => {
            let ds = ds_str.parse::<usize>().ok()?;
            let sheet = doc.result_sheet(&SheetId::new(*sheet_id))?;
            let val = sheet.lookup_scalar(key, Some(ds)).or_else(|| sheet.lookup_scalar(key, None))?;
            Some(format_axis_tick(val, AxisScale::Linear, &LocaleNumberPolicy::default()))
        }
        _ => None,
    }
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
        MarkerShape::Circle => build_ellipse_path(c, r, r),
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

pub(crate) fn build_ellipse_path(c: PointPt, rx: f64, ry: f64) -> Path2D {
    let kx = KAPPA * rx;
    let ky = KAPPA * ry;
    Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(c.x, c.y - ry)),
            PathCmd::CubicTo { c1: PointPt::new(c.x + kx, c.y - ry), c2: PointPt::new(c.x + rx, c.y - ky), to: PointPt::new(c.x + rx, c.y) },
            PathCmd::CubicTo { c1: PointPt::new(c.x + rx, c.y + ky), c2: PointPt::new(c.x + kx, c.y + ry), to: PointPt::new(c.x, c.y + ry) },
            PathCmd::CubicTo { c1: PointPt::new(c.x - kx, c.y + ry), c2: PointPt::new(c.x - rx, c.y + ky), to: PointPt::new(c.x - rx, c.y) },
            PathCmd::CubicTo { c1: PointPt::new(c.x - rx, c.y - ky), c2: PointPt::new(c.x - kx, c.y - ry), to: PointPt::new(c.x, c.y - ry) },
            PathCmd::Close,
        ],
    }
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

    emit_thresholds(&mut scene.primitives, r, x_axis, &graph.spec.x_axis.thresholds, true);
    emit_thresholds(&mut scene.primitives, r, y_axis, &graph.spec.y_axis.thresholds, false);
    if let (Some(y2), Some(y2_spec)) = (y2_axis, graph.spec.y2_axis.as_ref()) {
        emit_thresholds(&mut scene.primitives, r, y2, &y2_spec.thresholds, false);
    }

    emit_frame_spines(&mut scene.primitives, r, graph.spec.frame_preset, x_axis, y_axis, y2_axis);

    emit_x_ticks(&mut scene.primitives, &mut scene.warnings, r, x_axis, categorical_x_ticks, &policy);
    if !x_add.is_empty() {
        let tick_stroke = StrokePaint::solid(Rgba8::BLACK, x_axis.stroke_width_pt.max(0.75));
        emit_additional_x_tick_primitives(&x_add, r.bottom(), x_axis.tick_angle_deg, tick_stroke, &mut scene.primitives);
    }
    emit_y_ticks(&mut scene.primitives, &mut scene.warnings, r.x, true, y_axis, &policy);
    if !y_add.is_empty() {
        let tick_stroke = StrokePaint::solid(Rgba8::BLACK, y_axis.stroke_width_pt.max(0.75));
        emit_additional_y_tick_primitives(&y_add, r.x, true, y_axis.tick_angle_deg, 6.5, tick_stroke, &mut scene.primitives);
    }
    if let Some(y2) = y2_axis {
        emit_y_ticks(&mut scene.primitives, &mut scene.warnings, r.right(), false, y2, &policy);
        if !y2_add.is_empty() {
            let tick_stroke = StrokePaint::solid(Rgba8::BLACK, y2.stroke_width_pt.max(0.75));
            emit_additional_y_tick_primitives(&y2_add, r.right(), false, y2.tick_angle_deg, 6.5, tick_stroke, &mut scene.primitives);
        }
    }

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
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich,
            anchor_pt: PointPt::new(14.0_f64.min(r.x * 0.35), r.y + 0.5 * r.height),
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

fn emit_thresholds(primitives: &mut Vec<ScenePrimitive>, r: RectPt, axis: &SegmentedAxis, thresholds: &[AxisThreshold], is_x: bool) {
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
                rich: parse_markup(label),
                anchor_pt,
                anchor,
                font_size_pt: 8.0,
                rotation_deg: 0.0,
                color: th.color,
            }));
        }
    }
}

fn emit_frame_spines(
    primitives: &mut Vec<ScenePrimitive>,
    r: RectPt,
    preset: FramePreset,
    x_axis: &SegmentedAxis,
    y_axis: &SegmentedAxis,
    y2_axis: Option<&SegmentedAxis>,
) {
    let x_stroke = StrokePaint::solid(Rgba8::BLACK, x_axis.stroke_width_pt);
    let y_stroke = StrokePaint::solid(Rgba8::BLACK, y_axis.stroke_width_pt);
    let offset = if matches!(preset, FramePreset::OffsetRangeTufte) { 4.0 } else { 0.0 };

    match preset {
        FramePreset::OpenL | FramePreset::OffsetRangeTufte => {
            draw_segmented_spine(primitives, r.bottom() + offset, true, x_axis, x_stroke);
            draw_segmented_spine(primitives, r.x - offset, false, y_axis, y_stroke);
        }
        FramePreset::PlainBoxed => {
            draw_segmented_spine(primitives, r.bottom(), true, x_axis, x_stroke);
            draw_segmented_spine(primitives, r.y, true, x_axis, x_stroke);
            draw_segmented_spine(primitives, r.x, false, y_axis, y_stroke);
            draw_segmented_spine(primitives, r.right(), false, y2_axis.unwrap_or(y_axis), y_stroke);
        }
        FramePreset::DualYSecondaryRight => {
            draw_segmented_spine(primitives, r.bottom(), true, x_axis, x_stroke);
            draw_segmented_spine(primitives, r.x, false, y_axis, y_stroke);
            draw_segmented_spine(primitives, r.right(), false, y2_axis.unwrap_or(y_axis), y_stroke);
        }
        FramePreset::OriginCrossing => {
            let x_pos = y_axis.map_value(0.0).map_or(r.bottom(), |(_, pt)| pt);
            let y_pos = x_axis.map_value(0.0).map_or(r.x, |(_, pt)| pt);
            draw_segmented_spine(primitives, x_pos, true, x_axis, x_stroke);
            draw_segmented_spine(primitives, y_pos, false, y_axis, y_stroke);
        }
    }
}

fn draw_segmented_spine(primitives: &mut Vec<ScenePrimitive>, fixed: f64, is_horizontal: bool, axis: &SegmentedAxis, stroke: StrokePaint) {
    for (idx, seg) in axis.segments.iter().enumerate() {
        let (p0, p1) = if is_horizontal {
            (PointPt::new(seg.span_start_pt, fixed), PointPt::new(seg.span_end_pt, fixed))
        } else {
            (PointPt::new(fixed, seg.span_start_pt), PointPt::new(fixed, seg.span_end_pt))
        };
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
    r: RectPt,
    axis: &SegmentedAxis,
    categorical_ticks: Option<&[(f64, String)]>,
    policy: &LocaleNumberPolicy,
) {
    let tick_stroke = StrokePaint::solid(Rgba8::BLACK, axis.stroke_width_pt.max(0.75));
    if let Some(ticks) = categorical_ticks {
        for (pos_val, label) in ticks {
            if let Some((_, x_pt)) = axis.map_value(*pos_val) {
                primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(x_pt, r.bottom()), PointPt::new(x_pt, r.bottom() + 4.0)),
                    fill: None,
                    stroke: Some(tick_stroke),
                    clip: None,
                });
                let anchor = if axis.tick_angle_deg.abs() > 15.0 { TextAnchor::End } else { TextAnchor::Middle };
                primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: parse_markup(label),
                    anchor_pt: PointPt::new(x_pt, r.bottom() + 13.0),
                    anchor,
                    font_size_pt: 8.5,
                    rotation_deg: axis.tick_angle_deg,
                    color: Rgba8::BLACK,
                }));
            }
        }
        return;
    }

    for seg in &axis.segments {
        let (majors, minors) = compute_segment_tick_values(seg, None, true, warnings);
        for m in minors {
            if let Some(x_pt) = seg.map_strict(m) {
                primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(x_pt, r.bottom()), PointPt::new(x_pt, r.bottom() + 2.2)),
                    fill: None,
                    stroke: Some(tick_stroke),
                    clip: None,
                });
            }
        }
        for val in majors {
            if let Some(x_pt) = seg.map_strict(val) {
                primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(x_pt, r.bottom()), PointPt::new(x_pt, r.bottom() + 4.0)),
                    fill: None,
                    stroke: Some(tick_stroke),
                    clip: None,
                });
                let rich = format_tick_rich(val, seg.scale, axis.tick_format, policy);
                let anchor = if axis.tick_angle_deg.abs() > 15.0 { TextAnchor::End } else { TextAnchor::Middle };
                primitives.push(ScenePrimitive::Text(PositionedText {
                    rich,
                    anchor_pt: PointPt::new(x_pt, r.bottom() + 13.0),
                    anchor,
                    font_size_pt: 8.5,
                    rotation_deg: axis.tick_angle_deg,
                    color: Rgba8::BLACK,
                }));
            }
        }
    }
}

fn emit_y_ticks(
    primitives: &mut Vec<ScenePrimitive>,
    warnings: &mut Vec<ChartWarning>,
    spine_x: f64,
    is_left: bool,
    axis: &SegmentedAxis,
    policy: &LocaleNumberPolicy,
) {
    let tick_stroke = StrokePaint::solid(Rgba8::BLACK, axis.stroke_width_pt.max(0.75));
    let dir = if is_left { -1.0 } else { 1.0 };
    let anchor = if is_left { TextAnchor::End } else { TextAnchor::Start };

    for seg in &axis.segments {
        let (majors, minors) = compute_segment_tick_values(seg, None, true, warnings);
        for m in minors {
            if let Some(y_pt) = seg.map_strict(m) {
                primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(spine_x, y_pt), PointPt::new(spine_x + dir * 2.2, y_pt)),
                    fill: None,
                    stroke: Some(tick_stroke),
                    clip: None,
                });
            }
        }
        for val in majors {
            if let Some(y_pt) = seg.map_strict(val) {
                primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(spine_x, y_pt), PointPt::new(spine_x + dir * 4.0, y_pt)),
                    fill: None,
                    stroke: Some(tick_stroke),
                    clip: None,
                });
                let rich = format_tick_rich(val, seg.scale, axis.tick_format, policy);
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

/// Emit `LegendSpec` and `GraphAnnotation` items cleanly.
pub(crate) fn render_legend_and_annotations(scene: &mut ChartScene, graph: &GraphSheet, doc: &ProjectDocument, legend_items: &[LegendItem]) {
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    for ann in &graph.spec.annotations {
        render_annotation(scene, doc, info_id, ann);
    }
    if graph.spec.legend.visible && !legend_items.is_empty() {
        render_legend(scene, &graph.spec, legend_items);
    }
}

fn render_annotation(scene: &mut ChartScene, doc: &ProjectDocument, info_id: Option<&SheetId>, ann: &GraphAnnotation) {
    match ann {
        GraphAnnotation::Shape { kind, bounds_pt, stroke_color, stroke_width_pt, dash, fill_color } => {
            let r = RectPt::from(*bounds_pt);
            let path = match kind {
                curvebook_doc::AnnotationShapeKind::Rectangle => Path2D::rect(r),
                curvebook_doc::AnnotationShapeKind::RoundedRectangle => rounded_rect_path(r, 4.0),
                curvebook_doc::AnnotationShapeKind::Ellipse => build_ellipse_path(r.center(), 0.5 * r.width, 0.5 * r.height),
            };
            scene.primitives.push(ScenePrimitive::Path {
                path,
                fill: fill_color.map(FillPaint::solid),
                stroke: Some(stroke_with_dash(*stroke_color, *stroke_width_pt, *dash)),
                clip: None,
            });
        }
        GraphAnnotation::LineOrArrow { start_pt, end_pt, arrowhead, dash, color } => {
            let p0 = PointPt::new(start_pt.0, start_pt.1);
            let p1 = PointPt::new(end_pt.0, end_pt.1);
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(p0, p1),
                fill: None,
                stroke: Some(stroke_with_dash(*color, 1.25, *dash)),
                clip: None,
            });
            if *arrowhead {
                emit_arrowhead(&mut scene.primitives, p0, p1, *color);
            }
        }
        GraphAnnotation::TextBox { bounds_pt, markdown, bordered } => {
            let r = RectPt::from(*bounds_pt);
            if *bordered {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(r),
                    fill: Some(FillPaint::solid(Rgba8::WHITE)),
                    stroke: Some(StrokePaint::solid(Rgba8::BLACK, 0.8)),
                    clip: None,
                });
            }
            let rich = expand_doc_text(markdown, doc, info_id);
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich,
                anchor_pt: PointPt::new(r.x + 4.0, r.y + 11.0),
                anchor: TextAnchor::Start,
                font_size_pt: 8.5,
                rotation_deg: 0.0,
                color: Rgba8::BLACK,
            }));
        }
        GraphAnnotation::EmbeddedResultTable { result_sheet_id, tab_id, bounds_pt } => {
            render_embedded_result_table(scene, doc, result_sheet_id, tab_id.as_deref(), RectPt::from(*bounds_pt));
        }
        GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bounds_pt, font_size_pt, bordered } => {
            crate::equation_box::render_model_equation_annotation(
                scene,
                doc,
                &crate::equation_box::ModelEquationParams {
                    result_sheet_id,
                    data_set_index: *data_set_index,
                    mode: *mode,
                    bounds_pt: *bounds_pt,
                    font_size_pt: *font_size_pt,
                    bordered: *bordered,
                },
            );
        }
    }
}

fn rounded_rect_path(r: RectPt, radius: f64) -> Path2D {
    let rad = radius.min(0.5 * r.width).min(0.5 * r.height).max(0.0);
    Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(r.x + rad, r.y)),
            PathCmd::LineTo(PointPt::new(r.right() - rad, r.y)),
            PathCmd::QuadTo { control: PointPt::new(r.right(), r.y), to: PointPt::new(r.right(), r.y + rad) },
            PathCmd::LineTo(PointPt::new(r.right(), r.bottom() - rad)),
            PathCmd::QuadTo { control: PointPt::new(r.right(), r.bottom()), to: PointPt::new(r.right() - rad, r.bottom()) },
            PathCmd::LineTo(PointPt::new(r.x + rad, r.bottom())),
            PathCmd::QuadTo { control: PointPt::new(r.x, r.bottom()), to: PointPt::new(r.x, r.bottom() - rad) },
            PathCmd::LineTo(PointPt::new(r.x, r.y + rad)),
            PathCmd::QuadTo { control: PointPt::new(r.x, r.y), to: PointPt::new(r.x + rad, r.y) },
            PathCmd::Close,
        ],
    }
}

fn emit_arrowhead(primitives: &mut Vec<ScenePrimitive>, from: PointPt, to: PointPt, color: Rgba8) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = dx.hypot(dy);
    if len < 1e-6 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let px = -uy;
    let py = ux;
    let head_len = 6.0;
    let head_w = 2.5;
    let base = PointPt::new(to.x - head_len * ux, to.y - head_len * uy);
    let left = PointPt::new(base.x + head_w * px, base.y + head_w * py);
    let right = PointPt::new(base.x - head_w * px, base.y - head_w * py);
    primitives.push(ScenePrimitive::Path { path: polygon_path(&[to, left, right]), fill: Some(FillPaint::solid(color)), stroke: None, clip: None });
}

fn render_embedded_result_table(scene: &mut ChartScene, doc: &ProjectDocument, sheet_id: &SheetId, tab_id: Option<&str>, r: RectPt) {
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(r),
        fill: Some(FillPaint::solid(Rgba8::WHITE)),
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 0.75)),
        clip: None,
    });
    let Some(res) = doc.result_sheet(sheet_id) else {
        return;
    };
    if matches!(res.status, ResultStatus::Failed { .. }) {
        return;
    }
    let Some(tab) = tab_id.and_then(|tid| res.tabs.iter().find(|t| t.id == tid)).or_else(|| res.tabs.first()) else {
        return;
    };
    let mut y = r.y + 10.0;
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup(&format!("<b>{}</b>", tab.title)),
        anchor_pt: PointPt::new(r.x + 4.0, y),
        anchor: TextAnchor::Start,
        font_size_pt: 7.5,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    y += 10.0;
    let max_rows = ((r.height - 20.0) / 9.5).floor().max(0.0) as usize;
    for (row_idx, row_cells) in tab.rows.iter().take(max_rows).enumerate() {
        let label = tab.row_labels.get(row_idx).map_or("", String::as_str);
        let first_val = row_cells.first().map_or_else(String::new, format_result_cell);
        let line = if label.is_empty() { first_val } else { format!("{label}: {first_val}") };
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(line),
            anchor_pt: PointPt::new(r.x + 4.0, y),
            anchor: TextAnchor::Start,
            font_size_pt: 7.0,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
        y += 9.5;
    }
}

fn format_result_cell(cell: &ResultCell) -> String {
    let policy = LocaleNumberPolicy::default();
    match cell {
        ResultCell::Empty => String::new(),
        ResultCell::Number { value, prefix, .. } => {
            let num = format_axis_tick(*value, AxisScale::Linear, &policy);
            prefix.as_ref().map_or(num.clone(), |p| format!("{p}{num}"))
        }
        ResultCell::ConfidenceInterval { lower, upper } => format!("{lower:?}..{upper:?}"),
        ResultCell::PValue { formatted, .. } => formatted.clone(),
        ResultCell::Text(s) => s.clone(),
        ResultCell::Boolean(b) => b.to_string(),
    }
}

fn render_legend(scene: &mut ChartScene, spec: &GraphSpec, items: &[LegendItem]) {
    let font_size = 8.0;
    let row_h = 12.0;
    let max_label_w = items.iter().map(|it| measure_text_pt(&parse_markup(&it.label), font_size).width_pt).fold(24.0_f64, f64::max);
    let box_w = 24.0 + max_label_w;
    let box_h = 8.0 + (items.len() as f64) * row_h;
    let (lx, ly) = spec.legend.position_pt.unwrap_or_else(|| {
        let r = scene.plot_rect_pt;
        ((r.right() + 8.0).min(scene.width_pt - box_w - 4.0).max(r.x + r.width - box_w), r.y + 4.0)
    });
    let box_rect = RectPt::new(lx, ly, box_w, box_h);
    if spec.legend.bordered {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(box_rect),
            fill: Some(FillPaint::solid(Rgba8::WHITE)),
            stroke: Some(StrokePaint::solid(Rgba8::BLACK, 0.75)),
            clip: None,
        });
    }
    for (idx, item) in items.iter().enumerate() {
        let cy = ly + 6.0 + (idx as f64 + 0.5) * row_h;
        let sx = lx + 11.0;
        if let Some(fill) = item.swatch_fill {
            let border = item.swatch_border.map(|c| StrokePaint::solid(c, 0.8));
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(RectPt::new(sx - 5.0, cy - 3.5, 10.0, 7.0)),
                fill: Some(FillPaint::solid(fill)),
                stroke: border,
                clip: None,
            });
        }
        if let Some(lc) = item.line_color {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(sx - 7.0, cy), PointPt::new(sx + 7.0, cy)),
                fill: None,
                stroke: Some(stroke_with_dash(lc, 1.3, item.line_dash)),
                clip: None,
            });
        }
        if let Some(ms) = &item.marker {
            let mut small = *ms;
            small.size_pt = small.size_pt.min(5.0);
            emit_marker(&mut scene.primitives, PointPt::new(sx, cy), &small, None);
        }
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&item.label),
            anchor_pt: PointPt::new(sx + 10.0, cy + 3.0),
            anchor: TextAnchor::Start,
            font_size_pt: font_size,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
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
    points: Vec<XyDatum>,
    replicates: Vec<(f64, f64, usize, usize)>,
    has_x_err: bool,
    has_y_err: bool,
    label: String,
    default_color: Rgba8,
    fitted_overlay: Option<FittedOverlayData>,
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

    let r = scene.plot_rect_pt;
    let x_axis = SegmentedAxis::build(&graph.spec.x_axis, r.x, r.right(), &x_vals, false, 0.0);
    let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &y_vals, false, 0.05);
    let y2_axis = if has_y2 {
        let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2"));
        let source = if y2_vals.is_empty() { &y_vals } else { &y2_vals };
        Some(SegmentedAxis::build(&y2_spec, r.bottom(), r.y, source, false, 0.05))
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
                _ => None,
            },
            swatch_border: None,
        });
    }

    render_legend_and_annotations(scene, graph, doc, &legend_items);
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
                extract_xy_table_series(xy, series_spec, &mut points, &mut replicates, warnings);
            } else {
                warnings.push(ChartWarning::new(
                    "missing_data_set",
                    format!("data set index {} out of bounds in '{}'", series_spec.data_set_index, sheet.id),
                ));
            }
        }

        let fitted_overlay = resolve_fitted_overlay(doc, series_spec, warnings);
        out.push(ExtractedXySeries {
            sheet_id,
            col_index: series_spec.data_set_index,
            points,
            replicates,
            has_x_err,
            has_y_err,
            label: series_spec.legend_label.clone().unwrap_or(fallback_label),
            default_color: color,
            fitted_overlay,
        });
    }
    out
}

fn extract_xy_table_series(
    xy: &XyTable,
    series_spec: &SeriesSpec,
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

    for row in 0..xy.row_count() {
        let Some((xc, xlo, xhi)) = extract_xy_x_row(xy, row, x_submode, series_spec.x_error_bars.as_ref(), warnings) else {
            continue;
        };
        let Some(subcells) = ds.cells.get(row) else {
            continue;
        };
        if !xy.y_subcolumn_mode.is_pre_summarized() && series_spec.y_error_bars.is_none() {
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

fn resolve_fitted_overlay(doc: &ProjectDocument, series: &SeriesSpec, warnings: &mut Vec<ChartWarning>) -> Option<FittedOverlayData> {
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
    Some(FittedOverlayData { spec: effective_spec, curve: effective_curve })
}

fn collect_xy_domain_values(spec: &GraphSpec, extracted: &[ExtractedXySeries]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut x_vals = Vec::new();
    let mut y_vals = Vec::new();
    let mut y2_vals = Vec::new();
    for (s_spec, item) in spec.series.iter().zip(extracted.iter()) {
        let target_y = if s_spec.bind_to_right_y2 { &mut y2_vals } else { &mut y_vals };
        for pt in &item.points {
            x_vals.push(pt.x_center);
            if item.has_x_err {
                x_vals.push(pt.x_lo);
                x_vals.push(pt.x_hi);
            }
            target_y.push(pt.y_center);
            if item.has_y_err {
                target_y.push(pt.y_lo);
                target_y.push(pt.y_hi);
            }
        }
        for &(rx, ry, _, _) in &item.replicates {
            x_vals.push(rx);
            target_y.push(ry);
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
    let (marker_opt, line_opt, area_opt) = match &spec.gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill } => (*marker, *line, *area_fill),
        GraphicalDataObject::AlignedDotPlot { marker, .. } => (Some(*marker), None, None),
        _ => (Some(MarkerStyle { fill_color: data.default_color, stroke_color: data.default_color, ..MarkerStyle::default() }), None, None),
    };

    // 1. Fitted regression curve overlay & bands (under points)
    if let Some(ov) = &data.fitted_overlay {
        let curve_color = line_opt.map_or_else(|| marker_opt.map_or(data.default_color, |m| m.stroke_color), |l| l.color);
        render_fitted_curve_overlay(&mut scene.primitives, &mut scene.warnings, ov, x_axis, y_axis, curve_color, clip);
    }

    // 2. Map summary points & emit warnings for off-domain coordinates
    let mut mapped_runs: Vec<Vec<PointPt>> = Vec::new();
    let mut current_run: Vec<PointPt> = Vec::new();
    let mut summary_markers: Vec<(PointPt, usize, f64, f64)> = Vec::new();
    let mut prev_seg_pair: Option<(usize, usize)> = None;
    let mut warned_x_domain = false;
    let mut warned_y_domain = false;

    for pt in &data.points {
        let Some((sx, px)) = x_axis.map_value(pt.x_center) else {
            if !warned_x_domain && is_out_of_axis_domain(x_axis, pt.x_center) {
                warned_x_domain = true;
                push_domain_omission_warning(&mut scene.warnings, x_axis, "X", pt.x_center);
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
        let mapped_pt = PointPt::new(px, py);
        current_run.push(mapped_pt);
        summary_markers.push((mapped_pt, pt.row, pt.x_center, pt.y_center));
    }
    flush_run(&mut current_run, &mut mapped_runs);

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
            if let Some(poly) = build_area_polygon(run, kind, base_py) {
                scene.primitives.push(ScenePrimitive::Path {
                    path: poly,
                    fill: Some(FillPaint::solid(af.fill_color)),
                    stroke: None,
                    clip: Some(clip),
                });
            }
        }
    }

    // 4. Connecting lines (PointToPoint / Staircase / MonotonicCubicSpline / AkimaSpline)
    if let Some(line_style) = line_opt {
        let stroke = stroke_with_dash(line_style.color, line_style.stroke_width_pt, line_style.dash_style);
        for run in &mapped_runs {
            if let Some(path) = build_connecting_path(run, line_style.kind) {
                scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: Some(clip) });
            }
        }
    }

    // 5. Error bars (Y vertical, X horizontal)
    let err_default = marker_opt.map_or(data.default_color, |m| m.stroke_color);
    for pt in &data.points {
        if let (Some(eb), Some((_, px))) = (&spec.y_error_bars, x_axis.map_value(pt.x_center)) {
            emit_directional_error_bar(
                scene,
                px,
                pt.y_center,
                pt.y_lo,
                pt.y_hi,
                AxisErrorTarget { axis: y_axis, is_vertical: true, spec: eb, default_color: err_default, clip },
            );
        }
        if let (Some(eb), Some((_, py))) = (&spec.x_error_bars, y_axis.map_value(pt.y_center)) {
            emit_directional_error_bar(
                scene,
                py,
                pt.x_center,
                pt.x_lo,
                pt.x_hi,
                AxisErrorTarget { axis: x_axis, is_vertical: false, spec: eb, default_color: err_default, clip },
            );
        }
    }

    // 6. Markers (either raw individual replicates when y_error_bars is None, or summarized centers)
    if let Some(marker) = marker_opt {
        let ctx = XyMarkerCtx {
            spec: graph_spec,
            x_axis,
            y_axis,
            marker,
            clip,
            use_replicates: !data.replicates.is_empty() && spec.y_error_bars.is_none(),
            warned_x_domain,
            warned_y_domain,
        };
        emit_xy_markers(scene, data, &summary_markers, &ctx);
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
            let Some((_, px)) = ctx.x_axis.map_value(rx) else {
                if !warned_x && is_out_of_axis_domain(ctx.x_axis, rx) {
                    warned_x = true;
                    push_domain_omission_warning(&mut scene.warnings, ctx.x_axis, "X", rx);
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
            let pt = PointPt::new(px, py);
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
    axis.segments.iter().any(|s| scale_transform(s.scale, val).is_none())
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
    let total_samples = usize::from(overlay.spec.sample_segments.clamp(256, 512));
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

    let (lo_t, hi_t) = match (scale_transform(seg_x.scale, first_x), scale_transform(seg_x.scale, last_x)) {
        (Some(gx0), Some(gx1)) => {
            let g_curve_lo = gx0.min(gx1);
            let g_curve_hi = gx0.max(gx1);
            (g_curve_lo.max(g_seg_min.min(g_seg_max)), g_curve_hi.min(g_seg_min.max(g_seg_max)))
        }
        _ => (g_seg_min.min(g_seg_max), g_seg_min.max(g_seg_max)),
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
