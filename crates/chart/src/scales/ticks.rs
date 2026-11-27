//! Auto nice-tick generation, additional ticks, scientific rich-text formatting, and template hooks.

use curvebook_doc::{AdditionalTick, ProjectDocument, ResultStatus, SheetId, TickDirection, TickLabelFormat};
use curvebook_math::{AxisScale, LogitDomain};
use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_text::{LocaleNumberPolicy, RichText, expand_template_hooks, parse_markup};

use super::MAX_TICKS_PER_SEGMENT;
use super::mapping::{AxisMapping, scale_inverse, scale_transform};
use super::segments::{ResolvedAxis, ResolvedSegment};
use crate::scene::{ChartWarning, LineCap, LineJoin, Path2D, PointPt, PositionedText, ScenePrimitive, StrokePaint, TextAnchor};

const SI_PREFIXES: [&str; 17] = ["y", "z", "a", "f", "p", "n", "\u{00B5}", "m", "", "k", "M", "G", "T", "P", "E", "Z", "Y"];

// ============================================================================
// 3. Auto Nice-Tick Computation & Scientific Rich-Text Formatting
// ============================================================================

/// Major vs minor tick mark classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TickKind {
    Major,
    Minor,
}

/// Optional explicit categorical tick entry mapping a data coordinate to a display label.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoricalTick {
    pub value: f64,
    pub label: String,
}

impl CategoricalTick {
    /// Construct a categorical tick at coordinate `value`.
    #[must_use]
    pub fn new(value: f64, label: impl Into<String>) -> Self {
        Self { value, label: label.into() }
    }

    /// Build evenly indexed categorical ticks at `0.0, 1.0, ...` from label strings.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn from_labels(labels: &[String]) -> Vec<Self> {
        labels.iter().enumerate().map(|(i, s)| Self { value: i as f64, label: s.clone() }).collect()
    }
}

/// Single resolved tick mark along an axis segment.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisTick {
    pub segment_index: usize,
    pub value: f64,
    pub coord_pt: f64,
    pub kind: TickKind,
    pub label: Option<RichText>,
}

/// Configuration for generating and formatting ticks on a [`ResolvedAxis`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickComputeConfig<'a> {
    pub tick_format: TickLabelFormat,
    pub policy: &'a LocaleNumberPolicy,
    pub auto_log_minor_ticks: bool,
    pub categorical_ticks: Option<&'a [CategoricalTick]>,
}

/// Compute all major and minor ticks across all segments of `axis`.
#[must_use]
pub fn compute_axis_ticks(axis: &ResolvedAxis, config: TickComputeConfig<'_>) -> Vec<AxisTick> {
    let mut unused = Vec::new();
    compute_axis_ticks_with_warnings(axis, config, &mut unused)
}

/// Compute all major and minor ticks across all segments of `axis`, emitting capping warnings.
#[must_use]
pub fn compute_axis_ticks_with_warnings(axis: &ResolvedAxis, config: TickComputeConfig<'_>, warnings: &mut Vec<ChartWarning>) -> Vec<AxisTick> {
    let mut out = Vec::new();
    for seg in &axis.segments {
        let mut seg_ticks = compute_segment_ticks_with_warnings(seg, config, warnings);
        out.append(&mut seg_ticks);
    }
    out
}

/// Compute major and minor ticks for a single [`ResolvedSegment`].
#[must_use]
pub fn compute_segment_ticks(seg: &ResolvedSegment, config: TickComputeConfig<'_>) -> Vec<AxisTick> {
    let mut unused = Vec::new();
    compute_segment_ticks_with_warnings(seg, config, &mut unused)
}

/// Compute major and minor ticks `(majors, minors)` in data units for a single [`ResolvedSegment`].
pub(crate) fn compute_segment_tick_values(
    seg: &ResolvedSegment,
    target_major_override: Option<usize>,
    auto_log_minor_ticks: bool,
    warnings: &mut Vec<ChartWarning>,
) -> (Vec<f64>, Vec<f64>) {
    let span_pt = (seg.span_end_pt - seg.span_start_pt).abs().max(1.0);
    let target_major = target_major_override.unwrap_or_else(|| ((span_pt / 54.0).round() as usize).clamp(3, 9));
    match seg.scale {
        AxisScale::Linear => linear_major_minor(seg, target_major, warnings),
        AxisScale::Log10 => log10_major_minor(seg, target_major, auto_log_minor_ticks, warnings),
        AxisScale::Log2 => log_base_major_minor(seg, 2.0, target_major, warnings),
        AxisScale::Ln => log_base_major_minor(seg, std::f64::consts::E, target_major, warnings),
        AxisScale::Logit(LogitDomain::Percent) => candidate_prob_ticks(seg, LOGIT_PCT_TICKS, target_major, warnings),
        AxisScale::Logit(LogitDomain::Proportion) => candidate_prob_ticks(seg, LOGIT_PROP_TICKS, target_major, warnings),
        AxisScale::Probit => candidate_prob_ticks(seg, PROBIT_PCT_TICKS, target_major, warnings),
        AxisScale::Reciprocal => {
            if seg.major_tick_interval.is_some_and(|s| s.is_finite() && s > 0.0) {
                linear_major_minor(seg, target_major, warnings)
            } else {
                reciprocal_major_minor(seg, target_major, warnings)
            }
        }
    }
}

/// Compute major and minor ticks for a single [`ResolvedSegment`], recording tick capping warnings.
#[must_use]
pub fn compute_segment_ticks_with_warnings(seg: &ResolvedSegment, config: TickComputeConfig<'_>, warnings: &mut Vec<ChartWarning>) -> Vec<AxisTick> {
    if let TickLabelFormat::Categorical(_) = config.tick_format
        && let Some(cat_ticks) = config.categorical_ticks
    {
        return compute_categorical_ticks(seg, cat_ticks);
    }

    let (majors, minors) = compute_segment_tick_values(seg, None, config.auto_log_minor_ticks, warnings);

    let mut ticks = Vec::with_capacity(majors.len() + minors.len());
    for val in majors {
        if let Some(coord_pt) = seg.mapping().map_value(val, seg.span_start_pt, seg.span_end_pt) {
            let label = format_tick_rich(val, seg.scale, config.tick_format, config.policy);
            ticks.push(AxisTick { segment_index: seg.segment_index, value: val, coord_pt, kind: TickKind::Major, label: Some(label) });
        }
    }
    for val in minors {
        if let Some(coord_pt) = seg.mapping().map_value(val, seg.span_start_pt, seg.span_end_pt) {
            ticks.push(AxisTick { segment_index: seg.segment_index, value: val, coord_pt, kind: TickKind::Minor, label: None });
        }
    }
    ticks
}

/// Resolved custom [`AdditionalTick`] projected onto an active axis segment.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAdditionalTick {
    pub segment_index: usize,
    pub value: f64,
    pub coord_pt: f64,
    pub label: Option<RichText>,
    pub draw_gridline: bool,
}

/// Map custom [`AdditionalTick`] entries across active segments of `axis`.
///
/// Values outside every active segment domain (including values inside break gaps or outside the
/// valid scale domain such as `<= 0` on log scales, `0` on reciprocal, or endpoints on
/// logit/probit) are omitted cleanly. When `label` is `None`, the value is formatted using
/// `tick_format` and `policy`.
#[must_use]
pub fn compute_additional_ticks(
    axis: &ResolvedAxis,
    additional: &[AdditionalTick],
    tick_format: TickLabelFormat,
    policy: &LocaleNumberPolicy,
) -> Vec<ResolvedAdditionalTick> {
    let segs: Vec<(usize, AxisMapping, f64, f64)> =
        axis.segments.iter().map(|s| (s.segment_index, s.mapping(), s.span_start_pt, s.span_end_pt)).collect();
    compute_additional_ticks_for_segments(&segs, additional, tick_format, policy, None, None)
}

fn additional_segment_contains_value(min: f64, max: f64, value: f64) -> bool {
    if !min.is_finite() || !max.is_finite() || !value.is_finite() {
        return false;
    }
    let lo = min.min(max);
    let hi = min.max(max);
    let half_span = (0.5 * hi - 0.5 * lo).abs();
    let tol = 2e-12 * half_span.max(lo.abs()).max(hi.abs());
    value >= lo - tol && value <= hi + tol
}

pub(crate) fn compute_additional_ticks_for_segments(
    segments: &[(usize, AxisMapping, f64, f64)],
    additional: &[AdditionalTick],
    tick_format: TickLabelFormat,
    policy: &LocaleNumberPolicy,
    doc: Option<&ProjectDocument>,
    primary_id: Option<&SheetId>,
) -> Vec<ResolvedAdditionalTick> {
    let mut out = Vec::with_capacity(additional.len().min(MAX_TICKS_PER_SEGMENT));
    for tick in additional.iter().take(MAX_TICKS_PER_SEGMENT) {
        if !tick.value.is_finite() {
            continue;
        }
        for &(segment_index, mapping, span_start, span_end) in segments {
            if !additional_segment_contains_value(mapping.min, mapping.max, tick.value) {
                continue;
            }
            if let Some(raw_coord) = mapping.map_value(tick.value, span_start, span_end) {
                let lo_pt = span_start.min(span_end);
                let hi_pt = span_start.max(span_end);
                let coord_pt = crate::clamp_f64_safe(raw_coord, lo_pt, hi_pt);
                let label = match &tick.label {
                    Some(raw) => {
                        let expanded = doc.map_or_else(|| raw.clone(), |d| resolve_template_hooks(raw, d, primary_id));
                        if expanded.trim().is_empty() { None } else { Some(parse_markup(&expanded)) }
                    }
                    None => Some(format_tick_rich(tick.value, mapping.scale, tick_format, policy)),
                };
                out.push(ResolvedAdditionalTick { segment_index, value: tick.value, coord_pt, label, draw_gridline: tick.draw_gridline });
                break;
            }
        }
    }
    out
}

pub(crate) fn gridline_stroke(grid_color: Rgba8) -> StrokePaint {
    StrokePaint { color: grid_color, width_pt: 0.5, dash: LineDashStyle::Dotted, cap: LineCap::Butt, join: LineJoin::Miter }
}

pub(crate) fn emit_additional_tick_gridline_segments(
    ticks: &[ResolvedAdditionalTick],
    is_horizontal: bool,
    perp_spans: &[(f64, f64)],
    grid_color: Rgba8,
    primitives: &mut Vec<ScenePrimitive>,
) {
    let stroke = gridline_stroke(grid_color);
    for tick in ticks {
        if !tick.draw_gridline {
            continue;
        }
        for &(q0, q1) in perp_spans {
            let (p0, p1) = if is_horizontal {
                (PointPt::new(tick.coord_pt, q0), PointPt::new(tick.coord_pt, q1))
            } else {
                (PointPt::new(q0, tick.coord_pt), PointPt::new(q1, tick.coord_pt))
            };
            primitives.push(ScenePrimitive::Path { path: Path2D::line(p0, p1), fill: None, stroke: Some(stroke), clip: None });
        }
    }
}

/// Compute `(offset_start, offset_end)` perpendicular to a cartesian axis spine for `dir`.
#[must_use]
pub(crate) fn tick_span_offsets(dir: TickDirection, len: f64, outward_sign: f64) -> Option<(f64, f64)> {
    if !len.is_finite() || len <= 0.0 {
        return None;
    }
    match dir {
        TickDirection::Outside => Some((0.0, outward_sign * len)),
        TickDirection::Inside => Some((-outward_sign * len, 0.0)),
        TickDirection::Both => Some((-outward_sign * len, outward_sign * len)),
        TickDirection::None => None,
    }
}

pub(crate) fn emit_additional_x_tick_primitives(
    ticks: &[ResolvedAdditionalTick],
    spine_y: f64,
    tick_angle_deg: f64,
    tick_direction: TickDirection,
    tick_stroke: StrokePaint,
    primitives: &mut Vec<ScenePrimitive>,
) {
    let rot = if tick_angle_deg.is_finite() { tick_angle_deg } else { 0.0 };
    let anchor = if rot.abs() > 10.0 { TextAnchor::End } else { TextAnchor::Middle };
    let span = tick_span_offsets(tick_direction, 4.0, 1.0);
    for tick in ticks {
        if let Some((o0, o1)) = span {
            primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(tick.coord_pt, spine_y + o0), PointPt::new(tick.coord_pt, spine_y + o1)),
                fill: None,
                stroke: Some(tick_stroke),
                clip: None,
            });
        }
        if let Some(rich) = &tick.label {
            primitives.push(ScenePrimitive::Text(PositionedText {
                rich: rich.clone(),
                anchor_pt: PointPt::new(tick.coord_pt, spine_y + 13.0),
                anchor,
                font_size_pt: 8.5,
                rotation_deg: rot,
                color: Rgba8::BLACK,
            }));
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AdditionalYTickLayout {
    pub spine_x: f64,
    pub is_left: bool,
    pub tick_angle_deg: f64,
    pub label_offset_pt: f64,
    pub tick_direction: TickDirection,
}

pub(crate) fn emit_additional_y_tick_primitives(
    ticks: &[ResolvedAdditionalTick],
    layout: AdditionalYTickLayout,
    tick_stroke: StrokePaint,
    primitives: &mut Vec<ScenePrimitive>,
) {
    let dir = if layout.is_left { -1.0 } else { 1.0 };
    let anchor = if layout.is_left { TextAnchor::End } else { TextAnchor::Start };
    let rot = if layout.tick_angle_deg.is_finite() { layout.tick_angle_deg } else { 0.0 };
    let span = tick_span_offsets(layout.tick_direction, 4.0, dir);
    for tick in ticks {
        if let Some((o0, o1)) = span {
            primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(layout.spine_x + o0, tick.coord_pt), PointPt::new(layout.spine_x + o1, tick.coord_pt)),
                fill: None,
                stroke: Some(tick_stroke),
                clip: None,
            });
        }
        if let Some(rich) = &tick.label {
            primitives.push(ScenePrimitive::Text(PositionedText {
                rich: rich.clone(),
                anchor_pt: PointPt::new(layout.spine_x + dir * layout.label_offset_pt, tick.coord_pt + 3.0),
                anchor,
                font_size_pt: 8.5,
                rotation_deg: rot,
                color: Rgba8::BLACK,
            }));
        }
    }
}

const LOGIT_PCT_TICKS: &[f64] = &[0.001, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 25.0, 50.0, 75.0, 90.0, 95.0, 99.0, 99.5, 99.9, 99.95, 99.99, 99.999];
const LOGIT_PROP_TICKS: &[f64] =
    &[0.00001, 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95, 0.99, 0.995, 0.999, 0.9995, 0.9999, 0.99999];
const PROBIT_PCT_TICKS: &[f64] = &[0.001, 0.01, 0.05, 0.1, 1.0, 5.0, 10.0, 20.0, 50.0, 80.0, 90.0, 95.0, 99.0, 99.9, 99.95, 99.99, 99.999];

fn compute_categorical_ticks(seg: &ResolvedSegment, cat_ticks: &[CategoricalTick]) -> Vec<AxisTick> {
    let mut out = Vec::new();
    for ct in cat_ticks.iter().take(MAX_TICKS_PER_SEGMENT) {
        if seg.contains_value(ct.value)
            && let Some(coord_pt) = seg.mapping().map_value(ct.value, seg.span_start_pt, seg.span_end_pt)
        {
            out.push(AxisTick {
                segment_index: seg.segment_index,
                value: ct.value,
                coord_pt,
                kind: TickKind::Major,
                label: Some(parse_markup(&ct.label)),
            });
        }
    }
    out
}

pub(super) fn nice_linear_step(rough: f64) -> f64 {
    if !rough.is_finite() || rough <= 0.0 {
        return 1.0;
    }
    let exp = rough.log10().floor();
    let base = 10.0_f64.powf(exp);
    if !base.is_finite() || base <= 0.0 {
        return rough;
    }
    let frac = rough / base;
    let nice_mult = if frac <= 1.5 {
        1.0
    } else if frac <= 3.0 {
        2.0
    } else if frac <= 7.0 {
        5.0
    } else {
        10.0
    };
    let step = nice_mult * base;
    if step.is_finite() && step > 0.0 { step } else { rough }
}

/// Overflow-safe linear tick generator bounded at `MAX_TICKS_PER_SEGMENT` ticks per segment.
pub(crate) fn compute_safe_linear_ticks(
    min_v: f64,
    max_v: f64,
    requested_step: Option<f64>,
    target_ticks: usize,
    warnings: &mut Vec<ChartWarning>,
) -> (Vec<f64>, f64, usize) {
    let lo = min_v.min(max_v);
    let hi = min_v.max(max_v);
    if !lo.is_finite() || !hi.is_finite() || hi <= lo {
        let pt = if lo.is_finite() { vec![clean_zero(lo)] } else { Vec::new() };
        return (pt, 1.0, 0);
    }
    let span = hi - lo;
    let half_span = 0.5 * hi - 0.5 * lo;
    if !half_span.is_finite() || half_span <= 0.0 {
        return (vec![clean_zero(lo)], 1.0, 0);
    }

    let max_intervals = (MAX_TICKS_PER_SEGMENT.saturating_sub(1)).max(1) as f64;
    let fallback_step = || {
        if span.is_finite() {
            nice_linear_step(span / 8.0)
        } else {
            let hs = nice_linear_step(half_span / 8.0);
            if (2.0 * hs).is_finite() { 2.0 * hs } else { half_span }
        }
    };
    let mut step = match requested_step {
        Some(s) if s.is_finite() && s > 0.0 => s,
        _ if span.is_finite() => nice_linear_step(span / (target_ticks.max(2) as f64)),
        _ => {
            let hs = nice_linear_step(half_span / (target_ticks.max(2) as f64));
            if (2.0 * hs).is_finite() { 2.0 * hs } else { half_span }
        }
    };
    let min_safe_step = if span.is_finite() {
        (span / max_intervals).max(lo.abs().max(hi.abs()) * 1e-13)
    } else {
        (2.0 * (half_span / max_intervals)).max(lo.abs().max(hi.abs()) * 1e-13)
    };
    let raw_intervals = if span.is_finite() { span / step } else { 2.0 * (half_span / step) };
    if !raw_intervals.is_finite() || raw_intervals > max_intervals || step < min_safe_step {
        let stride = (raw_intervals / max_intervals).ceil();
        let eff_step = if raw_intervals.is_finite() && stride.is_finite() && stride <= 1_000_000.0 {
            let scaled = step * stride;
            if scaled.is_finite() && scaled >= min_safe_step { scaled } else { fallback_step().max(min_safe_step) }
        } else {
            fallback_step().max(min_safe_step)
        };
        if let Some(req) = requested_step
            && req.is_finite()
            && req > 0.0
        {
            warnings.push(ChartWarning::new(
                "major_tick_interval_capped",
                format!("major_tick_interval ({req:e}) exceeds max {MAX_TICKS_PER_SEGMENT} ticks over span [{lo}, {hi}]; adjusted to {eff_step:e}"),
            ));
        }
        step = eff_step;
    }

    let mut start_k_f = (lo / step - 1e-9).ceil();
    let mut end_k_f = (hi / step + 1e-9).floor();
    if !start_k_f.is_finite()
        || !end_k_f.is_finite()
        || start_k_f.abs() > 4.5e15
        || end_k_f.abs() > 4.5e15
        || (end_k_f - start_k_f) > max_intervals + 2.0
    {
        step = fallback_step().max(min_safe_step);
        start_k_f = (lo / step - 1e-9).ceil();
        end_k_f = (hi / step + 1e-9).floor();
    }
    if !start_k_f.is_finite() || !end_k_f.is_finite() || end_k_f < start_k_f {
        return (vec![clean_zero(lo), clean_zero(hi)], step, 0);
    }

    let count = ((end_k_f - start_k_f).round().max(0.0) as usize).saturating_add(1).min(MAX_TICKS_PER_SEGMENT);
    let tol = 1e-9 * (if span.is_finite() { span } else { half_span }).max(step);
    let mut majors = Vec::with_capacity(count);
    for dk in 0..count {
        let mut v = clean_tick_residual((start_k_f + (dk as f64)) * step, step);
        if !v.is_finite() {
            continue;
        }
        if v < lo && lo - v <= tol {
            v = lo;
        } else if v > hi && v - hi <= tol {
            v = hi;
        }
        if v >= lo - tol && v <= hi + tol {
            majors.push(clean_zero(v));
        }
    }
    if majors.is_empty() {
        majors.push(clean_zero(lo));
        majors.push(clean_zero(hi));
    }
    (majors, step, count)
}

fn linear_major_minor(seg: &ResolvedSegment, target_major: usize, warnings: &mut Vec<ChartWarning>) -> (Vec<f64>, Vec<f64>) {
    let lo = seg.domain_min.min(seg.domain_max);
    let hi = seg.domain_min.max(seg.domain_max);
    let requested = seg.major_tick_interval.filter(|s| s.is_finite() && *s > 0.0);
    let (majors, safe_step, _) = compute_safe_linear_ticks(lo, hi, requested, target_major, warnings);
    let minors = compute_linear_minors(lo, hi, &majors, safe_step, seg.minor_ticks_per_interval);
    (majors, minors)
}

fn compute_linear_minors(lo: f64, hi: f64, majors: &[f64], major_step: f64, minor_per_interval: u8) -> Vec<f64> {
    if minor_per_interval == 0 || majors.len() > 64 || !major_step.is_finite() || major_step <= 0.0 {
        return Vec::new();
    }
    let sub_divs = usize::from(minor_per_interval) + 1;
    let sub_step = major_step / (sub_divs as f64);
    if !sub_step.is_finite() || sub_step <= 0.0 {
        return Vec::new();
    }
    let anchor = majors.first().copied().unwrap_or(lo);
    let start_f = ((lo - anchor) / sub_step).floor() - 1.0;
    let end_f = ((hi - anchor) / sub_step).ceil() + 1.0;
    if !start_f.is_finite() || !end_f.is_finite() || start_f.abs() > 4.5e15 || end_f.abs() > 4.5e15 || end_f < start_f {
        return Vec::new();
    }
    let count = ((end_f - start_f).round().max(0.0) as usize).saturating_add(1).min(MAX_TICKS_PER_SEGMENT * 4);
    let mut minors = Vec::new();
    let eps = 1e-9 * major_step;
    for dk in 0..count {
        if minors.len() >= MAX_TICKS_PER_SEGMENT {
            break;
        }
        let v = clean_tick_residual(anchor + (start_f + (dk as f64)) * sub_step, major_step);
        if !v.is_finite() || v < lo - eps || v > hi + eps {
            continue;
        }
        let is_major = majors.iter().any(|&m| (m - v).abs() <= eps);
        if !is_major {
            minors.push(clean_zero(v));
        }
    }
    minors
}

fn log_in_base(value: f64, base: f64) -> f64 {
    if (base - 10.0).abs() < f64::EPSILON {
        value.log10()
    } else if (base - 2.0).abs() < f64::EPSILON {
        value.log2()
    } else if (base - std::f64::consts::E).abs() < f64::EPSILON {
        value.ln()
    } else {
        value.log(base)
    }
}

fn log_exponent_range(lo: f64, hi: f64, base: f64) -> Option<(i32, i32, i32)> {
    let lo_exp_f = log_in_base(lo, base).floor();
    let hi_exp_f = log_in_base(hi, base).ceil();
    if !lo_exp_f.is_finite() || !hi_exp_f.is_finite() || lo_exp_f > hi_exp_f {
        return None;
    }
    let span_f = (hi_exp_f - lo_exp_f).max(1.0);
    if !span_f.is_finite() {
        return None;
    }
    let max_ticks_f = MAX_TICKS_PER_SEGMENT.saturating_sub(1).max(1) as f64;
    let min_stride_f = (span_f / max_ticks_f).ceil().max(1.0);
    let stride_f = (span_f / 8.0).ceil().max(min_stride_f).max(1.0);
    if !stride_f.is_finite() || lo_exp_f < f64::from(i32::MIN) || hi_exp_f > f64::from(i32::MAX) || stride_f > f64::from(i32::MAX) {
        return None;
    }
    Some((lo_exp_f as i32, hi_exp_f as i32, (stride_f as i32).max(1)))
}

fn pow_in_base(base: f64, exp: f64) -> f64 {
    let rounded = exp.round();
    if (exp - rounded).abs() <= 1e-12 && rounded >= f64::from(i32::MIN) && rounded <= f64::from(i32::MAX) {
        #[allow(clippy::cast_possible_truncation)]
        let k = rounded as i32;
        base.powi(k)
    } else {
        base.powf(exp)
    }
}

/// Records that an explicit major tick interval produced fewer than two major ticks on a non-linear scale
/// (typically a linear-axis interval left over after switching to a log/probit/logit scale) and was ignored.
fn warn_interval_ignored(warnings: &mut Vec<ChartWarning>, step: f64, scale: AxisScale) {
    warnings.push(ChartWarning::new(
        "major_tick_interval_ignored",
        format!("major_tick_interval ({step:e}) yields fewer than 2 major ticks on {scale:?} axis; using the scale's default ticks"),
    ));
}

fn explicit_log_major_ticks(lo: f64, hi: f64, base: f64, exp_step: f64, target_major: usize, warnings: &mut Vec<ChartWarning>) -> (Vec<f64>, f64) {
    let t_lo = log_in_base(lo, base);
    let t_hi = log_in_base(hi, base);
    if !t_lo.is_finite() || !t_hi.is_finite() || t_hi <= t_lo {
        return (Vec::new(), exp_step);
    }
    let (exp_ticks, eff_step, _) = compute_safe_linear_ticks(t_lo, t_hi, Some(exp_step), target_major, warnings);
    let lo_tol = lo * (1.0 - 1e-9);
    let hi_tol = if hi <= f64::MAX / (1.0 + 1e-9) { hi * (1.0 + 1e-9) } else { f64::MAX };
    let majors = exp_ticks
        .into_iter()
        .map(|e| pow_in_base(base, e))
        .filter(|&v| v.is_finite() && v > 0.0 && v >= lo_tol && v <= hi_tol)
        .take(MAX_TICKS_PER_SEGMENT)
        .collect();
    (majors, eff_step)
}

fn compute_log_spaced_minors(majors: &[f64], minor_per_interval: u8) -> Vec<f64> {
    let mut minors = Vec::new();
    if minor_per_interval == 0 || majors.len() < 2 {
        return minors;
    }
    let divs = usize::from(minor_per_interval) + 1;
    for pair in majors.windows(2) {
        let [a, b] = *pair else { continue };
        let ta = a.ln();
        let tb = b.ln();
        for j in 1..divs {
            if minors.len() >= MAX_TICKS_PER_SEGMENT {
                break;
            }
            let t = ta + (tb - ta) * ((j as f64) / (divs as f64));
            let v = t.exp();
            if v.is_finite() && v > 0.0 {
                minors.push(v);
            }
        }
    }
    minors
}

fn compute_log10_decade_minors(k_lo: i32, k_hi: i32, lo_tol: f64, hi_tol: f64, majors: &[f64], minor_per_interval: u8) -> Vec<f64> {
    let mut minors = Vec::new();
    let use_standard_2_to_9 = minor_per_interval == 0 || minor_per_interval == 8;
    let divs = f64::from(minor_per_interval) + 1.0;
    let mut dec = k_lo.saturating_sub(1);
    while dec <= k_hi && minors.len() < MAX_TICKS_PER_SEGMENT {
        let base = 10.0_f64.powi(dec);
        if base.is_finite() && base > 0.0 {
            if use_standard_2_to_9 {
                for mult in 2_u8..=9 {
                    if minors.len() >= MAX_TICKS_PER_SEGMENT {
                        break;
                    }
                    let v = f64::from(mult) * base;
                    if v.is_finite() && v > 0.0 && v >= lo_tol && v <= hi_tol && !majors.iter().any(|&m| (m - v).abs() <= m * 1e-9) {
                        minors.push(v);
                    }
                }
            } else {
                for j in 1..=minor_per_interval {
                    if minors.len() >= MAX_TICKS_PER_SEGMENT {
                        break;
                    }
                    let v = base + (9.0 * base) * (f64::from(j) / divs);
                    if v.is_finite() && v > 0.0 && v >= lo_tol && v <= hi_tol && !majors.iter().any(|&m| (m - v).abs() <= m * 1e-9) {
                        minors.push(v);
                    }
                }
            }
        }
        let Some(next_dec) = dec.checked_add(1) else {
            break;
        };
        dec = next_dec;
    }
    minors
}

fn log10_major_minor(
    seg: &ResolvedSegment,
    target_major: usize,
    auto_log_minor_ticks: bool,
    warnings: &mut Vec<ChartWarning>,
) -> (Vec<f64>, Vec<f64>) {
    if !seg.domain_min.is_finite() || !seg.domain_max.is_finite() || seg.domain_min <= 0.0 || seg.domain_max <= 0.0 {
        return (Vec::new(), Vec::new());
    }
    let lo = seg.domain_min.min(seg.domain_max);
    let hi = seg.domain_min.max(seg.domain_max);
    let lo_tol = lo * (1.0 - 1e-9);
    let hi_tol = if hi <= f64::MAX / (1.0 + 1e-9) { hi * (1.0 + 1e-9) } else { f64::MAX };

    if let Some(exp_step) = seg.major_tick_interval.filter(|s| s.is_finite() && *s > 0.0) {
        let (majors, eff_step) = explicit_log_major_ticks(lo, hi, 10.0, exp_step, target_major, warnings);
        if majors.len() < 2 {
            warn_interval_ignored(warnings, exp_step, seg.scale);
        } else {
            let minors = if (eff_step - 1.0).abs() <= 1e-9
                && let Some((k_lo, k_hi, _)) = log_exponent_range(lo, hi, 10.0)
                && (i64::from(k_hi) - i64::from(k_lo)) <= 8
                && (auto_log_minor_ticks || seg.minor_ticks_per_interval > 0)
            {
                compute_log10_decade_minors(k_lo, k_hi, lo_tol, hi_tol, &majors, seg.minor_ticks_per_interval)
            } else {
                compute_log_spaced_minors(&majors, seg.minor_ticks_per_interval)
            };
            return (majors, minors);
        }
    }

    let Some((k_lo, k_hi, stride)) = log_exponent_range(lo, hi, 10.0) else {
        return (Vec::new(), Vec::new());
    };

    let mut majors = Vec::new();
    let mut k = k_lo;
    while k <= k_hi && majors.len() < MAX_TICKS_PER_SEGMENT {
        let v = 10.0_f64.powi(k);
        if v.is_finite() && v > 0.0 && v >= lo_tol && v <= hi_tol {
            majors.push(v);
        }
        let Some(next_k) = k.checked_add(stride) else {
            break;
        };
        k = next_k;
    }

    if majors.len() < 2 {
        let (lin_maj, _) = linear_major_minor(seg, 4, warnings);
        if lin_maj.len() >= 2 {
            majors = lin_maj;
        }
    }

    let decades = i64::from(k_hi) - i64::from(k_lo);
    let emit_minors = (auto_log_minor_ticks || seg.minor_ticks_per_interval > 0) && decades <= 8 && stride == 1;
    let minors =
        if emit_minors { compute_log10_decade_minors(k_lo, k_hi, lo_tol, hi_tol, &majors, seg.minor_ticks_per_interval) } else { Vec::new() };
    (majors, minors)
}

fn log_base_major_minor(seg: &ResolvedSegment, base: f64, target_major: usize, warnings: &mut Vec<ChartWarning>) -> (Vec<f64>, Vec<f64>) {
    if !seg.domain_min.is_finite()
        || !seg.domain_max.is_finite()
        || seg.domain_min <= 0.0
        || seg.domain_max <= 0.0
        || !base.is_finite()
        || base <= 1.0
    {
        return (Vec::new(), Vec::new());
    }
    let lo = seg.domain_min.min(seg.domain_max);
    let hi = seg.domain_min.max(seg.domain_max);

    if let Some(exp_step) = seg.major_tick_interval.filter(|s| s.is_finite() && *s > 0.0) {
        let (majors, _) = explicit_log_major_ticks(lo, hi, base, exp_step, target_major, warnings);
        if majors.len() < 2 {
            warn_interval_ignored(warnings, exp_step, seg.scale);
        } else {
            let minors = compute_log_spaced_minors(&majors, seg.minor_ticks_per_interval);
            return (majors, minors);
        }
    }

    let Some((k_lo, k_hi, stride)) = log_exponent_range(lo, hi, base) else {
        return (Vec::new(), Vec::new());
    };

    let lo_tol = lo * (1.0 - 1e-9);
    let hi_tol = if hi <= f64::MAX / (1.0 + 1e-9) { hi * (1.0 + 1e-9) } else { f64::MAX };
    let mut majors = Vec::new();
    let mut k = k_lo;
    while k <= k_hi && majors.len() < MAX_TICKS_PER_SEGMENT {
        let v = base.powi(k);
        if v.is_finite() && v > 0.0 && v >= lo_tol && v <= hi_tol {
            majors.push(v);
        }
        let Some(next_k) = k.checked_add(stride) else {
            break;
        };
        k = next_k;
    }
    if majors.len() < 2 {
        return linear_major_minor(seg, 4, warnings);
    }

    let minors = compute_log_spaced_minors(&majors, seg.minor_ticks_per_interval);
    (majors, minors)
}

fn compute_transformed_prob_minors(seg: &ResolvedSegment, majors: &[f64]) -> Vec<f64> {
    let mut minors = Vec::new();
    if seg.minor_ticks_per_interval == 0 || majors.len() < 2 {
        return minors;
    }
    let divs = usize::from(seg.minor_ticks_per_interval) + 1;
    for pair in majors.windows(2) {
        let [a, b] = *pair else { continue };
        if let (Some(ta), Some(tb)) = (scale_transform(seg.scale, a), scale_transform(seg.scale, b)) {
            for j in 1..divs {
                if minors.len() >= MAX_TICKS_PER_SEGMENT {
                    break;
                }
                let t = ta + (tb - ta) * ((j as f64) / (divs as f64));
                if let Some(v) = scale_inverse(seg.scale, t)
                    && v.is_finite()
                    && scale_transform(seg.scale, v).is_some()
                {
                    minors.push(v);
                }
            }
        }
    }
    minors
}

fn candidate_prob_ticks(seg: &ResolvedSegment, candidates: &[f64], target_major: usize, warnings: &mut Vec<ChartWarning>) -> (Vec<f64>, Vec<f64>) {
    let lo = seg.domain_min.min(seg.domain_max);
    let hi = seg.domain_min.max(seg.domain_max);
    if let Some(step) = seg.major_tick_interval.filter(|s| s.is_finite() && *s > 0.0) {
        let (raw_majors, _, _) = compute_safe_linear_ticks(lo, hi, Some(step), target_major, warnings);
        let majors: Vec<f64> = raw_majors.into_iter().filter(|&v| v.is_finite() && scale_transform(seg.scale, v).is_some()).collect();
        if majors.len() >= 2 {
            let minors = compute_transformed_prob_minors(seg, &majors);
            return (majors, minors);
        }
        warn_interval_ignored(warnings, step, seg.scale);
    }

    let eps = 1e-12 * hi.abs().max(1.0);
    let mut majors: Vec<f64> = candidates.iter().copied().filter(|&v| v >= lo - eps && v <= hi + eps).collect();
    if majors.len() < 2
        && let (Some(t0), Some(t1)) = (scale_transform(seg.scale, lo), scale_transform(seg.scale, hi))
        && (t1 - t0).abs() > 1e-12
    {
        majors.clear();
        for i in 0..=4 {
            let t = t0 + ((i as f64) / 4.0) * (t1 - t0);
            if let Some(v) = scale_inverse(seg.scale, t)
                && v.is_finite()
                && scale_transform(seg.scale, v).is_some()
            {
                majors.push(v);
            }
        }
    }
    if majors.len() >= 2 {
        let minors = compute_transformed_prob_minors(seg, &majors);
        (majors, minors)
    } else {
        linear_major_minor(seg, 4, warnings)
    }
}

fn reciprocal_major_minor(seg: &ResolvedSegment, target_major: usize, warnings: &mut Vec<ChartWarning>) -> (Vec<f64>, Vec<f64>) {
    let (Some(t0), Some(t1)) = (scale_transform(seg.scale, seg.domain_min), scale_transform(seg.scale, seg.domain_max)) else {
        return (Vec::new(), Vec::new());
    };
    let t_lo = t0.min(t1);
    let t_hi = t0.max(t1);
    let temp_seg = ResolvedSegment {
        segment_index: seg.segment_index,
        domain_min: t_lo,
        domain_max: t_hi,
        scale: AxisScale::Linear,
        length_fraction: seg.length_fraction,
        span_start_pt: seg.span_start_pt,
        span_end_pt: seg.span_end_pt,
        major_tick_interval: None,
        minor_ticks_per_interval: seg.minor_ticks_per_interval,
    };
    let (t_majors, t_minors) = linear_major_minor(&temp_seg, target_major, warnings);
    let majors: Vec<f64> = t_majors.into_iter().filter_map(|t| scale_inverse(AxisScale::Reciprocal, t)).collect();
    let minors: Vec<f64> = t_minors.into_iter().filter_map(|t| scale_inverse(AxisScale::Reciprocal, t)).collect();
    if majors.len() >= 2 { (majors, minors) } else { linear_major_minor(seg, target_major, warnings) }
}

pub(super) fn clean_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

fn clean_tick_residual(v: f64, step: f64) -> f64 {
    if v.abs() <= step.abs() * 1e-11 { 0.0 } else { clean_zero(v) }
}

/// Format a numeric tick value into markup/plain string honoring `format`, `scale`, and `policy`.
#[must_use]
pub(crate) fn format_tick_markup(value: f64, scale: AxisScale, format: TickLabelFormat, policy: &LocaleNumberPolicy) -> String {
    if !value.is_finite() {
        return String::from("NaN");
    }
    let clean = clean_zero(value);
    match format {
        TickLabelFormat::Decimal | TickLabelFormat::Categorical(_) => curvebook_text::format_axis_tick(clean, scale, policy),
        TickLabelFormat::PowerOfTenSuperscript => format_power_of_ten_markup(clean, policy),
        TickLabelFormat::ScientificE => format_scientific_e(clean, policy),
        TickLabelFormat::Antilog => {
            let antilog_val = match scale {
                AxisScale::Linear if clean.abs() <= 300.0 => 10.0_f64.powf(clean),
                _ => clean,
            };
            curvebook_text::format_axis_tick(antilog_val, AxisScale::Log10, policy)
        }
        TickLabelFormat::EngineeringSi => format_engineering_si(clean, policy),
        TickLabelFormat::AbsoluteDecimal => curvebook_text::format_axis_tick(clean.abs(), scale, policy),
    }
}

/// Format a numeric tick value into [`RichText`] honoring `format`, `scale`, and `policy`.
#[must_use]
pub fn format_tick_rich(value: f64, scale: AxisScale, format: TickLabelFormat, policy: &LocaleNumberPolicy) -> RichText {
    let text = format_tick_markup(value, scale, format, policy);
    if matches!(format, TickLabelFormat::PowerOfTenSuperscript) && value.is_finite() { parse_markup(&text) } else { RichText::plain(text) }
}

fn decompose_scientific_3dp(abs: f64) -> Option<(String, i32)> {
    let sci = format!("{abs:.3e}");
    let (mantissa, exp_str) = sci.split_once('e')?;
    let exp = exp_str.parse::<i32>().ok()?;
    Some((trim_decimal_str(mantissa.to_owned()), exp))
}

fn format_power_of_ten_markup(value: f64, policy: &LocaleNumberPolicy) -> String {
    if value == 0.0 {
        return String::from("0");
    }
    let Some((mant_trimmed, exp)) = decompose_scientific_3dp(value.abs()) else {
        return String::from("NaN");
    };
    let sign = if value.is_sign_negative() { policy.minus_sign.as_str() } else { "" };
    let exp_str = format_signed_int(exp, policy);
    if mant_trimmed == "1" {
        format!("{sign}10<sup>{exp_str}</sup>")
    } else {
        let mant_str = apply_locale_chars(&mant_trimmed, policy);
        format!("{sign}{mant_str}\u{00D7}10<sup>{exp_str}</sup>")
    }
}

fn format_scientific_e(value: f64, policy: &LocaleNumberPolicy) -> String {
    if value == 0.0 {
        return String::from("0");
    }
    let Some((mant_trimmed, exp)) = decompose_scientific_3dp(value.abs()) else {
        return String::from("NaN");
    };
    let sign = if value.is_sign_negative() { policy.minus_sign.as_str() } else { "" };
    let mant_str = apply_locale_chars(&mant_trimmed, policy);
    let exp_str = format_signed_int(exp, policy);
    format!("{sign}{mant_str}e{exp_str}")
}

#[allow(clippy::cast_possible_truncation)]
fn format_engineering_si(value: f64, policy: &LocaleNumberPolicy) -> String {
    if value == 0.0 {
        return String::from("0");
    }
    let abs = value.abs();
    let log3 = (abs.log10() / 3.0).floor();
    if !log3.is_finite() {
        return format_scientific_e(value, policy);
    }
    let mut exp3 = log3 as i32;
    if !(-9..=8).contains(&exp3) {
        return format_scientific_e(value, policy);
    }
    let mut scaled = abs / 10.0_f64.powi(exp3 * 3);
    while scaled >= 1000.0 && exp3 < 9 {
        exp3 += 1;
        scaled = abs / 10.0_f64.powi(exp3 * 3);
    }
    while scaled < 1.0 && exp3 > -9 {
        exp3 -= 1;
        scaled = abs / 10.0_f64.powi(exp3 * 3);
    }
    let mut rounded = format!("{scaled:.3}");
    if rounded.split('.').next().and_then(|int_part| int_part.parse::<u64>().ok()).is_some_and(|n| n >= 1000) {
        exp3 += 1;
        scaled = abs / 10.0_f64.powi(exp3 * 3);
        rounded = format!("{scaled:.3}");
    }
    if !(-8..=8).contains(&exp3) {
        return format_scientific_e(value, policy);
    }
    let prefix = SI_PREFIXES.get((exp3 + 8) as usize).copied().unwrap_or("");
    let sign = if value.is_sign_negative() { policy.minus_sign.as_str() } else { "" };
    let num_str = apply_locale_chars(&trim_decimal_str(rounded), policy);
    if prefix.is_empty() { format!("{sign}{num_str}") } else { format!("{sign}{num_str} {prefix}") }
}

fn format_signed_int(n: i32, policy: &LocaleNumberPolicy) -> String {
    if n < 0 { format!("{}{}", policy.minus_sign, n.unsigned_abs()) } else { format!("{n}") }
}

fn trim_decimal_str(mut s: String) -> String {
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" { String::from("0") } else { s }
}

fn apply_locale_chars(raw: &str, policy: &LocaleNumberPolicy) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    for ch in raw.chars() {
        match ch {
            '-' => out.push_str(&policy.minus_sign),
            '.' => out.push(policy.decimal_separator),
            _ => out.push(ch),
        }
    }
    out
}

pub(super) fn resolve_paired_info_id<'a>(doc: &'a ProjectDocument, primary_or_info_id: Option<&'a SheetId>) -> Option<&'a SheetId> {
    primary_or_info_id.and_then(|id| doc.paired_info_sheet_id(id).or_else(|| doc.info_sheet(id).map(|s| &s.id)))
}

/// Resolve `{{info:...}}` and `{{result:...}}` template hooks using `doc` and an optional primary
/// `DataSheet` (or direct `InfoSheet`) identifier.
#[must_use]
pub fn resolve_template_hooks(template: &str, doc: &ProjectDocument, primary_data_id: Option<&SheetId>) -> String {
    let paired_info_id = resolve_paired_info_id(doc, primary_data_id);
    resolve_template_hooks_with_info(template, doc, paired_info_id)
}

/// Resolve `{{info:...}}` and `{{result:...}}` template hooks using `doc` and an already-resolved
/// `paired_info_id` (`InfoSheet` identifier).
pub(crate) fn resolve_template_hooks_with_info(template: &str, doc: &ProjectDocument, paired_info_id: Option<&SheetId>) -> String {
    if !template.contains("{{") {
        return template.to_owned();
    }
    let paired_info = paired_info_id.and_then(|id| doc.info_sheet(id)).or_else(|| doc.info_sheets.first());
    expand_template_hooks(template, |token| resolve_single_hook(token, doc, paired_info))
}

fn resolve_single_hook(token: &str, doc: &ProjectDocument, paired_info: Option<&curvebook_doc::InfoSheet>) -> Option<String> {
    let parts: Vec<&str> = token.split(':').map(str::trim).collect();
    match parts.as_slice() {
        ["info", name] if !name.is_empty() => {
            let text = paired_info.and_then(|i| i.get_constant(name)).map(|c| c.value.display_text())?;
            (!text.is_empty()).then_some(text)
        }
        ["info", sheet, name] if !sheet.is_empty() && !name.is_empty() => {
            let text = doc.info_sheet(&SheetId::new(*sheet)).and_then(|i| i.get_constant(name)).map(|c| c.value.display_text())?;
            (!text.is_empty()).then_some(text)
        }
        ["result", sheet, param] if !sheet.is_empty() && !param.is_empty() => {
            let res = doc.result_sheet(&SheetId::new(*sheet))?;
            if !matches!(res.status, ResultStatus::UpToDate) {
                return None;
            }
            let needle = param.trim();
            let val = res
                .scalars
                .iter()
                .find(|s| {
                    s.key.trim().eq_ignore_ascii_case(needle)
                        && s.value.is_finite()
                        && !crate::equation_box::is_unstable_masked_scalar(res, &s.key, s.data_set_index, Some(doc))
                })
                .or_else(|| {
                    if !matches!(res.analysis.kind, curvebook_doc::AnalysisKind::SimpleLogisticRegression(_)) {
                        return None;
                    }
                    let ds_title = crate::equation_box::dataset_title_for_result(doc, res, 0)?;
                    let prefixed = format!("{ds_title}.{needle}");
                    res.scalars.iter().find(|s| s.data_set_index.is_none() && s.key.trim().eq_ignore_ascii_case(&prefixed) && s.value.is_finite())
                })
                .map(|s| s.value)?;
            Some(format_scalar_clean(val))
        }
        ["result", sheet, ds_str, param] if !sheet.is_empty() && !param.is_empty() => {
            let ds = ds_str.parse::<usize>().ok()?;
            let res = doc.result_sheet(&SheetId::new(*sheet))?;
            if !matches!(res.status, ResultStatus::UpToDate) {
                return None;
            }
            let needle = param.trim();
            // Match the exact dataset index first; fall back to sheet-global scalars
            // (`data_set_index == None`) or legacy pre-Wave-1 SimpleLogisticRegression
            // `"{ds_title}.{param}"` scalars, never to a different dataset or an unstable scalar.
            let val = res
                .scalars
                .iter()
                .find(|s| {
                    s.data_set_index == Some(ds)
                        && s.key.trim().eq_ignore_ascii_case(needle)
                        && s.value.is_finite()
                        && !crate::equation_box::is_unstable_masked_scalar(res, &s.key, Some(ds), Some(doc))
                })
                .or_else(|| {
                    res.scalars.iter().find(|s| {
                        s.data_set_index.is_none()
                            && s.key.trim().eq_ignore_ascii_case(needle)
                            && s.value.is_finite()
                            && !crate::equation_box::is_unstable_masked_scalar(res, &s.key, Some(ds), Some(doc))
                    })
                })
                .or_else(|| {
                    if !matches!(res.analysis.kind, curvebook_doc::AnalysisKind::SimpleLogisticRegression(_)) {
                        return None;
                    }
                    let ds_title = crate::equation_box::dataset_title_for_result(doc, res, ds)?;
                    let prefixed = format!("{ds_title}.{needle}");
                    res.scalars.iter().find(|s| s.data_set_index.is_none() && s.key.trim().eq_ignore_ascii_case(&prefixed) && s.value.is_finite())
                })
                .map(|s| s.value)?;
            Some(format_scalar_clean(val))
        }
        _ => None,
    }
}

pub(crate) fn format_scalar_clean(v: f64) -> String {
    if !v.is_finite() {
        return String::from("NaN");
    }
    let abs = v.abs();
    if abs > 0.0 && !(1e-3..1e5).contains(&abs) { format!("{v:.3e}") } else { trim_decimal_str(format!("{v:.4}")) }
}
