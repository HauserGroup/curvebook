//! Axis scale mapping, guarded domain diagnostics, major/minor nice-tick computation,
//! discontinuous multi-segment axes, frame presets, thresholds, legends, and annotations.

use std::fmt::Write as FmtWrite;

use curvebook_doc::{
    AdditionalTick, AnnotationShapeKind, AxisSegment, AxisSpec, AxisThreshold, BreakOrnament, FramePreset, GraphAnnotation, GraphSpec, LegendSpec,
    ProjectDocument, SheetId, SizePt, TableArchetype, TickDirection, TickLabelFormat,
};
use curvebook_math::dist::{norm_cdf, norm_ppf};
use curvebook_math::{AxisScale, LogitDomain};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, Rgba8};
use curvebook_text::{LocaleNumberPolicy, RichText, expand_template_hooks, measure_text_pt, parse_markup};

use crate::scene::{
    ChartScene, ChartWarning, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor,
};

/// Upper bound on ticks emitted per segment to prevent unbounded allocation on adversarial specs.
pub const MAX_TICKS_PER_SEGMENT: usize = 256;

/// Fallback strictly positive lower floor used when clamping invalid log-scale bounds.
pub const LOG_POSITIVE_FLOOR_DEFAULT: f64 = 1e-12;

/// Default outward offset (in points) for [`AxisFrameMode::OffsetAxes`].
pub const DEFAULT_AXIS_OFFSET_PT: f64 = 6.0;

/// Default major tick length in PostScript points.
pub const DEFAULT_MAJOR_TICK_LEN_PT: f64 = 4.5;

/// Default minor tick length in PostScript points.
pub const DEFAULT_MINOR_TICK_LEN_PT: f64 = 2.5;

const MAX_SEGMENTS_PER_AXIS: usize = 3;
const MIN_FINITE_AXIS_SPAN: f64 = f64::MIN_POSITIVE * 4.0;
const ELLIPSE_KAPPA: f64 = 0.552_284_749_830_793_6;
const SI_PREFIXES: [&str; 17] = ["y", "z", "a", "f", "p", "n", "\u{00B5}", "m", "", "k", "M", "G", "T", "P", "E", "Z", "Y"];

// ============================================================================
// 1. Guarded Scale Mapping, Inverse Mapping, Diagnostics & Error-Bar Clamping
// ============================================================================

/// Aggregated counts of non-fatal domain anomalies encountered while mapping data onto an axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScaleDiagnostics {
    pub non_positive_log_skipped: usize,
    pub log_error_bars_clamped: usize,
    pub out_of_domain_logit_skipped: usize,
    pub out_of_domain_probit_skipped: usize,
    pub zero_reciprocal_skipped: usize,
    pub non_finite_skipped: usize,
    pub boundary_clamped: usize,
}

impl ScaleDiagnostics {
    /// True when zero domain anomalies occurred.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.non_positive_log_skipped == 0
            && self.log_error_bars_clamped == 0
            && self.out_of_domain_logit_skipped == 0
            && self.out_of_domain_probit_skipped == 0
            && self.zero_reciprocal_skipped == 0
            && self.non_finite_skipped == 0
            && self.boundary_clamped == 0
    }

    /// Record a domain rejection matching `scale` when `value` failed forward transformation.
    pub fn record_domain_rejection(&mut self, scale: AxisScale, value: f64) {
        if !value.is_finite() {
            self.non_finite_skipped += 1;
            return;
        }
        match scale {
            AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => self.non_positive_log_skipped += 1,
            AxisScale::Logit(_) => self.out_of_domain_logit_skipped += 1,
            AxisScale::Probit => self.out_of_domain_probit_skipped += 1,
            AxisScale::Reciprocal => self.zero_reciprocal_skipped += 1,
            AxisScale::Linear => {}
        }
    }

    /// Flush non-zero anomaly counters as structured [`ChartWarning`] entries into `warnings`.
    pub fn emit_warnings(&self, axis_label: &str, warnings: &mut Vec<ChartWarning>) {
        if self.non_positive_log_skipped > 0 {
            warnings.push(ChartWarning::new(
                "non_positive_on_log_axis",
                format!("skipped {} non-positive value(s) on log axis '{axis_label}'", self.non_positive_log_skipped),
            ));
        }
        if self.log_error_bars_clamped > 0 {
            warnings.push(ChartWarning::new(
                "log_error_bar_clamped",
                format!("clamped {} lower error-bar bound(s) to positive floor on log axis '{axis_label}'", self.log_error_bars_clamped),
            ));
        }
        if self.out_of_domain_logit_skipped > 0 {
            warnings.push(ChartWarning::new(
                "out_of_domain_logit",
                format!("skipped {} out-of-domain value(s) on logit axis '{axis_label}'", self.out_of_domain_logit_skipped),
            ));
        }
        if self.out_of_domain_probit_skipped > 0 {
            warnings.push(ChartWarning::new(
                "out_of_domain_probit",
                format!("skipped {} out-of-domain value(s) on probit axis '{axis_label}'", self.out_of_domain_probit_skipped),
            ));
        }
        if self.zero_reciprocal_skipped > 0 {
            warnings.push(ChartWarning::new(
                "zero_on_reciprocal_axis",
                format!("skipped {} zero value(s) on reciprocal (1/X) axis '{axis_label}'", self.zero_reciprocal_skipped),
            ));
        }
        if self.non_finite_skipped > 0 {
            warnings.push(ChartWarning::new(
                "non_finite_axis_value",
                format!("skipped {} non-finite value(s) on axis '{axis_label}'", self.non_finite_skipped),
            ));
        }
        if self.boundary_clamped > 0 {
            warnings.push(ChartWarning::new(
                "error_bar_domain_clamped",
                format!("clamped {} error-bar bound(s) to valid scale domain on axis '{axis_label}'", self.boundary_clamped),
            ));
        }
    }
}

/// Result of mapping a point and its `[lower, upper]` error-bar bounds onto a single axis span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ErrorBarMappedPt {
    pub center_pt: f64,
    pub lower_pt: f64,
    pub upper_pt: f64,
    pub lower_clipped: bool,
    pub upper_clipped: bool,
}

/// Guarded forward transform `g(u)` across all 7 [`AxisScale`] variants, explicitly invoking
/// `curvebook_math` inverse normal CDF (`norm_ppf`) for [`AxisScale::Probit`].
#[must_use]
pub fn scale_transform(scale: AxisScale, value: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    match scale {
        AxisScale::Probit => {
            if !(value > 0.0 && value < 100.0) {
                return None;
            }
            norm_ppf(value / 100.0).ok().filter(|t| t.is_finite())
        }
        _ => scale.transform(value),
    }
}

/// Guarded inverse transform `g^{-1}(t)` across all 7 [`AxisScale`] variants, using
/// `curvebook_math::dist::norm_cdf` for [`AxisScale::Probit`].
#[must_use]
pub fn scale_inverse(scale: AxisScale, transformed: f64) -> Option<f64> {
    if !transformed.is_finite() {
        return None;
    }
    match scale {
        AxisScale::Probit => {
            let pct = 100.0 * norm_cdf(transformed);
            pct.is_finite().then_some(pct)
        }
        _ => scale.inverse(transformed),
    }
}

/// Continuous mapping of a single axis domain `[min, max]` under [`AxisScale`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisMapping {
    pub min: f64,
    pub max: f64,
    pub scale: AxisScale,
}

impl AxisMapping {
    /// Project `value` onto `[span_start, span_end]` in scene points, returning `None` if invalid.
    #[must_use]
    pub fn map_value(self, value: f64, span_start: f64, span_end: f64) -> Option<f64> {
        if !span_start.is_finite() || !span_end.is_finite() {
            return None;
        }
        let t_min = scale_transform(self.scale, self.min)?;
        let t_max = scale_transform(self.scale, self.max)?;
        let t_val = scale_transform(self.scale, value)?;
        let denom = t_max - t_min;
        if !denom.is_finite() || denom.abs() <= MIN_FINITE_AXIS_SPAN {
            return None;
        }
        let u = (t_val - t_min) / denom;
        let pt = span_start + u * (span_end - span_start);
        pt.is_finite().then_some(pt)
    }

    /// Invert scene coordinate `coord_pt` along `[span_start, span_end]` back into data space.
    #[must_use]
    pub fn unmap_coord(self, coord_pt: f64, span_start: f64, span_end: f64) -> Option<f64> {
        if !coord_pt.is_finite() || !span_start.is_finite() || !span_end.is_finite() {
            return None;
        }
        let span = span_end - span_start;
        if !span.is_finite() || span.abs() <= MIN_FINITE_AXIS_SPAN {
            return None;
        }
        let t_min = scale_transform(self.scale, self.min)?;
        let t_max = scale_transform(self.scale, self.max)?;
        let denom = t_max - t_min;
        if !denom.is_finite() || denom.abs() <= MIN_FINITE_AXIS_SPAN {
            return None;
        }
        let u = (coord_pt - span_start) / span;
        let t_val = t_min + u * denom;
        scale_inverse(self.scale, t_val)
    }

    /// Map `value` onto `[span_start, span_end]`, recording domain rejections on `diag`.
    #[must_use]
    pub fn map_value_tracked(self, value: f64, span_start: f64, span_end: f64, diag: &mut ScaleDiagnostics) -> Option<f64> {
        if !value.is_finite() {
            diag.non_finite_skipped += 1;
            return None;
        }
        if scale_transform(self.scale, value).is_none() {
            diag.record_domain_rejection(self.scale, value);
            return None;
        }
        self.map_value(value, span_start, span_end)
    }

    /// Map `value` onto `[span_start, span_end]` and immediately append any warning to `warnings`.
    #[must_use]
    pub fn map_value_checked(self, value: f64, span_start: f64, span_end: f64, axis_label: &str, warnings: &mut Vec<ChartWarning>) -> Option<f64> {
        let mut diag = ScaleDiagnostics::default();
        let out = self.map_value_tracked(value, span_start, span_end, &mut diag);
        diag.emit_warnings(axis_label, warnings);
        out
    }

    /// Strictly positive domain floor used when clamping lower error-bar whiskers on log scales.
    #[must_use]
    pub fn positive_floor(self) -> f64 {
        match (self.min.is_finite() && self.min > 0.0, self.max.is_finite() && self.max > 0.0) {
            (true, true) => self.min.min(self.max),
            (true, false) => self.min,
            (false, true) => self.max,
            (false, false) => LOG_POSITIVE_FLOOR_DEFAULT,
        }
    }

    /// Map `(center, lower, upper)` onto `[span_start, span_end]`, clamping non-positive lower
    /// bounds on log axes to the positive axis floor and guarding logit/probit/reciprocal limits.
    #[must_use]
    pub fn map_error_bar(
        self,
        center: f64,
        lower: f64,
        upper: f64,
        span_start: f64,
        span_end: f64,
        diag: &mut ScaleDiagnostics,
    ) -> Option<ErrorBarMappedPt> {
        let center_pt = self.map_value_tracked(center, span_start, span_end, diag)?;
        let (safe_lower, lower_clipped) = self.clamp_error_bound(center, lower, true, diag);
        let (safe_upper, upper_clipped) = self.clamp_error_bound(center, upper, false, diag);
        let lower_pt = self.map_value(safe_lower, span_start, span_end).unwrap_or(center_pt);
        let upper_pt = self.map_value(safe_upper, span_start, span_end).unwrap_or(center_pt);
        Some(ErrorBarMappedPt { center_pt, lower_pt, upper_pt, lower_clipped, upper_clipped })
    }

    /// Map `(center, lower, upper)` onto `[span.0, span.1]` and flush any clamping warnings.
    #[must_use]
    pub fn map_error_bar_checked(
        self,
        center: f64,
        lower: f64,
        upper: f64,
        span: (f64, f64),
        axis_label: &str,
        warnings: &mut Vec<ChartWarning>,
    ) -> Option<ErrorBarMappedPt> {
        let mut diag = ScaleDiagnostics::default();
        let out = self.map_error_bar(center, lower, upper, span.0, span.1, &mut diag);
        diag.emit_warnings(axis_label, warnings);
        out
    }

    /// Closest-to-zero bound within `[self.min, self.max]` sharing `center`'s sign on reciprocal axes.
    pub(crate) fn reciprocal_zero_clamp_bound(self, center: f64) -> f64 {
        let same_sign = |v: f64| v.is_finite() && ((center > 0.0 && v > 0.0) || (center < 0.0 && v < 0.0));
        match (same_sign(self.min), same_sign(self.max)) {
            (true, true) => {
                if center > 0.0 {
                    self.min.min(self.max)
                } else {
                    self.min.max(self.max)
                }
            }
            (true, false) => self.min,
            (false, true) => self.max,
            (false, false) => center,
        }
    }

    fn clamp_error_bound(self, center: f64, bound: f64, is_lower: bool, diag: &mut ScaleDiagnostics) -> (f64, bool) {
        if !bound.is_finite() {
            diag.non_finite_skipped += 1;
            return (center, true);
        }
        match self.scale {
            AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => {
                if bound <= 0.0 {
                    if is_lower {
                        diag.log_error_bars_clamped += 1;
                    } else {
                        diag.non_positive_log_skipped += 1;
                    }
                    return (self.positive_floor(), true);
                }
                (bound, false)
            }
            AxisScale::Logit(LogitDomain::Proportion) => clamp_open_interval(bound, self.min, self.max, 0.0, 1.0, 1e-6, diag),
            AxisScale::Logit(LogitDomain::Percent) | AxisScale::Probit => clamp_open_interval(bound, self.min, self.max, 0.0, 100.0, 1e-4, diag),
            AxisScale::Reciprocal => {
                if bound == 0.0 || (bound.signum() != center.signum() && center != 0.0) {
                    diag.boundary_clamped += 1;
                    return (self.reciprocal_zero_clamp_bound(center), true);
                }
                (bound, false)
            }
            AxisScale::Linear => (bound, false),
        }
    }
}

fn clamp_open_interval(bound: f64, dom_min: f64, dom_max: f64, lo_open: f64, hi_open: f64, eps: f64, diag: &mut ScaleDiagnostics) -> (f64, bool) {
    let floor = if dom_min.is_finite() && dom_min > lo_open && dom_min < hi_open { dom_min } else { lo_open + eps };
    let ceil = if dom_max.is_finite() && dom_max > lo_open && dom_max < hi_open { dom_max } else { hi_open - eps };
    if bound <= lo_open {
        diag.boundary_clamped += 1;
        (floor.min(ceil), true)
    } else if bound >= hi_open {
        diag.boundary_clamped += 1;
        (floor.max(ceil), true)
    } else {
        (bound, false)
    }
}

// ============================================================================
// 2. Discontinuous Multi-Segment Axes & Segment-Aware Splitting Helpers
// ============================================================================

/// Orientation of a plotted axis on the 2D canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisOrientation {
    Horizontal,
    Vertical,
}

/// Physical break gap between two adjacent segments of a discontinuous axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisBreakGap {
    pub after_segment_index: usize,
    pub gap_start_pt: f64,
    pub gap_end_pt: f64,
    pub ornament: BreakOrnament,
}

/// Resolved segment of a 1–3 segment axis with finite validated bounds and point span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedSegment {
    pub segment_index: usize,
    pub domain_min: f64,
    pub domain_max: f64,
    pub scale: AxisScale,
    /// Normalized share of usable axis length in `(0.0, 1.0]`.
    pub length_fraction: f64,
    /// Point coordinate corresponding to `domain_min`.
    pub span_start_pt: f64,
    /// Point coordinate corresponding to `domain_max`.
    pub span_end_pt: f64,
    pub major_tick_interval: Option<f64>,
    pub minor_ticks_per_interval: u8,
}

impl ResolvedSegment {
    /// Build an [`AxisMapping`] for this segment's domain and scale.
    #[must_use]
    pub const fn mapping(&self) -> AxisMapping {
        AxisMapping { min: self.domain_min, max: self.domain_max, scale: self.scale }
    }

    /// True if `value` lies within `[domain_min, domain_max]` (inclusive with tiny boundary eps).
    #[must_use]
    pub fn contains_value(&self, value: f64) -> bool {
        if !value.is_finite() {
            return false;
        }
        let lo = self.domain_min.min(self.domain_max);
        let hi = self.domain_min.max(self.domain_max);
        let tol = 1e-12 * (hi - lo).abs().max(lo.abs()).max(hi.abs());
        value >= lo - tol && value <= hi + tol
    }

    /// True if scene coordinate `coord_pt` lies within `[span_start_pt, span_end_pt]`.
    #[must_use]
    pub fn contains_coord(&self, coord_pt: f64) -> bool {
        if !coord_pt.is_finite() {
            return false;
        }
        let lo = self.span_start_pt.min(self.span_end_pt);
        let hi = self.span_start_pt.max(self.span_end_pt);
        coord_pt >= lo - 1e-6 && coord_pt <= hi + 1e-6
    }

    /// Map `value` onto this segment's scene coordinate span if it belongs to this segment.
    #[must_use]
    pub fn map_value(&self, value: f64) -> Option<f64> {
        if !self.contains_value(value) {
            return None;
        }
        self.mapping().map_value(value, self.span_start_pt, self.span_end_pt)
    }

    /// Map `value` onto `[span_start_pt, span_end_pt]` if within `[domain_min, domain_max]` in transformed space.
    #[must_use]
    pub(crate) fn map_strict(&self, value: f64) -> Option<f64> {
        let g_min = scale_transform(self.scale, self.domain_min)?;
        let g_max = scale_transform(self.scale, self.domain_max)?;
        let g_u = scale_transform(self.scale, value)?;
        let denom = g_max - g_min;
        if !denom.is_finite() || denom.abs() <= MIN_FINITE_AXIS_SPAN {
            return None;
        }
        let g_span = denom.abs();
        let tol = 1e-9 * g_span.max(g_min.abs().max(g_max.abs()) * f64::EPSILON);
        let lo = g_min.min(g_max) - tol;
        let hi = g_min.max(g_max) + tol;
        if g_u < lo || g_u > hi {
            return None;
        }
        let s = ((g_u - g_min) / denom).clamp(0.0, 1.0);
        let pt = self.span_start_pt + s * (self.span_end_pt - self.span_start_pt);
        pt.is_finite().then_some(pt)
    }

    /// Clamp `value` into `[domain_min, domain_max]` in transformed space and return `(coord_pt, was_clipped)`.
    #[must_use]
    pub(crate) fn map_clamped(&self, value: f64) -> (f64, bool) {
        let Some(g_min) = scale_transform(self.scale, self.domain_min) else {
            return (self.span_start_pt, true);
        };
        let Some(g_max) = scale_transform(self.scale, self.domain_max) else {
            return (self.span_end_pt, true);
        };
        let lo = g_min.min(g_max);
        let hi = g_min.max(g_max);
        let num_hi = self.domain_min.max(self.domain_max);
        let (g_num_lo, g_num_hi) = if self.domain_min <= self.domain_max { (g_min, g_max) } else { (g_max, g_min) };
        let effective_val = if self.scale == AxisScale::Reciprocal {
            let anchor = if self.domain_min != 0.0 { self.domain_min } else { self.domain_max };
            if !value.is_finite() || value == 0.0 || (anchor != 0.0 && value.signum() != anchor.signum()) {
                return if value.is_finite() {
                    let safe = self.mapping().reciprocal_zero_clamp_bound(anchor);
                    let g_safe = scale_transform(self.scale, safe).unwrap_or(g_num_lo);
                    let denom = g_max - g_min;
                    let s = if denom.abs() > 0.0 { ((g_safe.clamp(lo, hi) - g_min) / denom).clamp(0.0, 1.0) } else { 0.0 };
                    (self.span_start_pt + s * (self.span_end_pt - self.span_start_pt), true)
                } else {
                    (self.span_start_pt, true)
                };
            }
            value
        } else {
            value
        };
        let g_val = match scale_transform(self.scale, effective_val) {
            Some(v) if v.is_finite() => v,
            _ => {
                let fallback = if effective_val.is_finite() && effective_val >= num_hi { g_num_hi } else { g_num_lo };
                let denom = g_max - g_min;
                let s = if denom.abs() > 0.0 { ((fallback - g_min) / denom).clamp(0.0, 1.0) } else { 0.0 };
                return (self.span_start_pt + s * (self.span_end_pt - self.span_start_pt), true);
            }
        };
        let clipped = g_val < lo || g_val > hi;
        let denom = g_max - g_min;
        let s = if denom.abs() > 0.0 { ((g_val.clamp(lo, hi) - g_min) / denom).clamp(0.0, 1.0) } else { 0.0 };
        (self.span_start_pt + s * (self.span_end_pt - self.span_start_pt), clipped)
    }

    /// Clip a data interval `[v_lo, v_hi]` against this segment and return its mapped point span.
    #[must_use]
    pub fn map_interval(&self, v_lo: f64, v_hi: f64) -> Option<SegmentIntervalPt> {
        if !v_lo.is_finite() || !v_hi.is_finite() {
            return None;
        }
        let seg_lo = self.domain_min.min(self.domain_max);
        let seg_hi = self.domain_min.max(self.domain_max);
        let req_lo = v_lo.min(v_hi);
        let req_hi = v_lo.max(v_hi);
        if req_hi < seg_lo || req_lo > seg_hi {
            return None;
        }
        let clipped_lo = req_lo.max(seg_lo);
        let clipped_hi = req_hi.min(seg_hi);
        let p0 = self.mapping().map_value(clipped_lo, self.span_start_pt, self.span_end_pt)?;
        let p1 = self.mapping().map_value(clipped_hi, self.span_start_pt, self.span_end_pt)?;
        Some(SegmentIntervalPt {
            segment_index: self.segment_index,
            start_pt: p0,
            end_pt: p1,
            clipped_start: req_lo < seg_lo,
            clipped_end: req_hi > seg_hi,
        })
    }
}

/// Result of locating and projecting a data value onto a [`ResolvedAxis`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentedPointMatch {
    pub segment_index: usize,
    pub coord_pt: f64,
}

/// Mapped sub-interval within a single [`ResolvedSegment`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentIntervalPt {
    pub segment_index: usize,
    pub start_pt: f64,
    pub end_pt: f64,
    pub clipped_start: bool,
    pub clipped_end: bool,
}

/// Error bar mapped across one or more segments of a [`ResolvedAxis`].
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentedErrorBar {
    pub center: Option<SegmentedPointMatch>,
    pub spans: Vec<SegmentIntervalPt>,
    pub lower_clipped: bool,
    pub upper_clipped: bool,
}

/// Clipped polyline path confined to a single `(x_segment_index, y_segment_index)` plot cell.
#[derive(Debug, Clone, PartialEq)]
pub struct ClippedSegmentPath {
    pub x_segment_index: usize,
    pub y_segment_index: usize,
    pub clip_rect: RectPt,
    pub path: Path2D,
}

/// Complete resolved axis across 1–3 discontinuous segments and their break gaps.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAxis {
    pub orientation: AxisOrientation,
    pub segments: Vec<ResolvedSegment>,
    pub breaks: Vec<AxisBreakGap>,
    pub axis_start_pt: f64,
    pub axis_end_pt: f64,
}

#[derive(Debug, Clone, Copy)]
enum DomainResolveSource<'a> {
    Bounds(Option<(f64, f64)>),
    Samples { data_values: &'a [f64], include_zero_if_linear: bool, top_headroom_frac: f64 },
}

impl ResolvedAxis {
    /// Resolve an [`AxisSpec`] over `plot_rect` along `orientation`, clamping invalid domains and
    /// normalizing `length_share` weights into `length_fraction`.
    #[must_use]
    pub fn resolve(
        spec: &AxisSpec,
        orientation: AxisOrientation,
        plot_rect: RectPt,
        data_bounds: Option<(f64, f64)>,
        warnings: &mut Vec<ChartWarning>,
    ) -> Self {
        let (axis_start_pt, axis_end_pt) = match orientation {
            AxisOrientation::Horizontal => (plot_rect.x, plot_rect.right()),
            AxisOrientation::Vertical => (plot_rect.bottom(), plot_rect.y),
        };
        Self::resolve_with_source(spec, orientation, axis_start_pt, axis_end_pt, DomainResolveSource::Bounds(data_bounds), warnings)
    }

    /// Resolve an [`AxisSpec`] over an explicit point span `span_pt = (start_pt, end_pt)` using raw data samples.
    #[must_use]
    pub(crate) fn resolve_from_samples(
        spec: &AxisSpec,
        orientation: AxisOrientation,
        span_pt: (f64, f64),
        data_values: &[f64],
        include_zero_if_linear: bool,
        top_headroom_frac: f64,
        warnings: &mut Vec<ChartWarning>,
    ) -> Self {
        Self::resolve_with_source(
            spec,
            orientation,
            span_pt.0,
            span_pt.1,
            DomainResolveSource::Samples { data_values, include_zero_if_linear, top_headroom_frac },
            warnings,
        )
    }

    fn resolve_with_source(
        spec: &AxisSpec,
        orientation: AxisOrientation,
        axis_start_pt: f64,
        axis_end_pt: f64,
        source: DomainResolveSource<'_>,
        warnings: &mut Vec<ChartWarning>,
    ) -> Self {
        let raw_segments = normalize_segment_slice(&spec.segments, &spec.title, warnings);
        let n = raw_segments.len();
        let total_len = (axis_end_pt - axis_start_pt).abs().max(1.0);
        let dir = if axis_end_pt >= axis_start_pt { 1.0 } else { -1.0 };

        let num_gaps = n.saturating_sub(1);
        let raw_gap = if spec.gap_width_pt.is_finite() && spec.gap_width_pt >= 0.0 { spec.gap_width_pt } else { 8.0 };
        let gap_pt = if num_gaps == 0 { 0.0 } else { raw_gap.min((total_len * 0.25) / (num_gaps as f64)) };
        let usable_len = (total_len - gap_pt * (num_gaps as f64)).max(1.0);

        let shares = normalize_length_shares(&raw_segments);
        let mut resolved = Vec::with_capacity(n);
        let mut breaks = Vec::with_capacity(num_gaps);
        let mut cursor = axis_start_pt;
        let mut prev_hi: Option<f64> = None;

        for (idx, seg) in raw_segments.iter().enumerate() {
            let fraction = shares.get(idx).copied().unwrap_or(1.0 / (n as f64));
            let seg_len = usable_len * fraction;
            let span_start_pt = cursor;
            let span_end_pt = cursor + dir * seg_len;
            let (domain_min, domain_max) = match source {
                DomainResolveSource::Bounds(data_bounds) => {
                    let seg_bounds_hint = segment_data_bounds_hint(idx, n, data_bounds);
                    resolve_segment_domain(seg, seg_bounds_hint, prev_hi, &spec.title, warnings)
                }
                DomainResolveSource::Samples { data_values, include_zero_if_linear, top_headroom_frac } => {
                    resolve_sample_segment_domain(seg, data_values, include_zero_if_linear, top_headroom_frac, warnings)
                }
            };
            prev_hi = Some(domain_max);

            resolved.push(ResolvedSegment {
                segment_index: idx,
                domain_min,
                domain_max,
                scale: seg.scale,
                length_fraction: fraction,
                span_start_pt,
                span_end_pt,
                major_tick_interval: seg.major_tick_interval,
                minor_ticks_per_interval: seg.minor_ticks_per_interval,
            });

            if idx + 1 < n {
                let gap_start_pt = span_end_pt;
                let gap_end_pt = gap_start_pt + dir * gap_pt;
                breaks.push(AxisBreakGap { after_segment_index: idx, gap_start_pt, gap_end_pt, ornament: spec.break_ornament });
                cursor = gap_end_pt;
            }
        }

        Self { orientation, segments: resolved, breaks, axis_start_pt, axis_end_pt }
    }

    /// Map `value` onto the first matching segment, returning `None` if it falls in a break gap
    /// or outside every segment.
    #[must_use]
    pub fn map_value(&self, value: f64) -> Option<SegmentedPointMatch> {
        for seg in &self.segments {
            if let Some(coord_pt) = seg.map_value(value) {
                return Some(SegmentedPointMatch { segment_index: seg.segment_index, coord_pt });
            }
        }
        None
    }

    /// Map `value` onto the first matching segment, recording invalid-domain statistics on `diag`.
    #[must_use]
    pub fn map_value_tracked(&self, value: f64, diag: &mut ScaleDiagnostics) -> Option<SegmentedPointMatch> {
        if !value.is_finite() {
            diag.non_finite_skipped += 1;
            return None;
        }
        if let Some(matched) = self.map_value(value) {
            return Some(matched);
        }
        if let Some(first) = self.segments.first()
            && scale_transform(first.scale, value).is_none()
        {
            diag.record_domain_rejection(first.scale, value);
        }
        None
    }

    /// Invert scene coordinate `coord_pt` back to data space using the segment containing `coord_pt`.
    #[must_use]
    pub fn unmap_coord(&self, coord_pt: f64) -> Option<f64> {
        for seg in &self.segments {
            if seg.contains_coord(coord_pt) {
                return seg.mapping().unmap_coord(coord_pt, seg.span_start_pt, seg.span_end_pt);
            }
        }
        None
    }

    /// Split a data-space interval `[v_start, v_end]` across all overlapping segments.
    #[must_use]
    pub fn map_interval(&self, v_start: f64, v_end: f64) -> Vec<SegmentIntervalPt> {
        let mut out = Vec::new();
        for seg in &self.segments {
            if let Some(interval) = seg.map_interval(v_start, v_end) {
                out.push(interval);
            }
        }
        out
    }

    /// Map a data point and error-bar bounds `[lower, upper]` across all matching segments,
    /// clamping non-positive log lower bounds to the positive floor with diagnostics.
    #[must_use]
    pub fn map_error_bar(&self, center: f64, lower: f64, upper: f64, diag: &mut ScaleDiagnostics) -> Option<SegmentedErrorBar> {
        if !center.is_finite() {
            diag.non_finite_skipped += 1;
            return None;
        }
        let center_match = self.map_value_tracked(center, diag);
        let first_seg = self.segments.first()?;
        let mapping = center_match.and_then(|m| self.segments.get(m.segment_index)).unwrap_or(first_seg).mapping();
        let (safe_lower, lower_clipped) = mapping.clamp_error_bound(center, lower, true, diag);
        let (safe_upper, upper_clipped) = mapping.clamp_error_bound(center, upper, false, diag);
        let lo = safe_lower.min(safe_upper);
        let hi = safe_lower.max(safe_upper);
        let spans = self.map_interval(lo, hi);
        if center_match.is_none() && spans.is_empty() {
            return None;
        }
        Some(SegmentedErrorBar { center: center_match, spans, lower_clipped, upper_clipped })
    }
}

/// Build the axis-aligned clip rectangle for a `(x_seg, y_seg)` pair.
#[must_use]
pub fn segment_cell_rect(x_seg: &ResolvedSegment, y_seg: &ResolvedSegment) -> RectPt {
    let x0 = x_seg.span_start_pt.min(x_seg.span_end_pt);
    let x1 = x_seg.span_start_pt.max(x_seg.span_end_pt);
    let y0 = y_seg.span_start_pt.min(y_seg.span_end_pt);
    let y1 = y_seg.span_start_pt.max(y_seg.span_end_pt);
    RectPt::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// Split a data-space polyline into clean per-segment [`ClippedSegmentPath`]s without drawing
/// spurious strokes across axis break gaps.
pub fn split_polyline_across_segments(
    points: &[(f64, f64)],
    x_axis: &ResolvedAxis,
    y_axis: &ResolvedAxis,
    diag: &mut ScaleDiagnostics,
) -> Vec<ClippedSegmentPath> {
    if points.len() < 2 {
        return Vec::new();
    }
    let nx = x_axis.segments.len();
    let ny = y_axis.segments.len();
    let mut cell_paths = vec![Path2D::new(); nx * ny];
    let mut last_pts: Vec<Option<PointPt>> = vec![None; nx * ny];

    for pair in points.windows(2) {
        let [(x0, y0), (x1, y1)] = *pair else { continue };
        if !validate_pair_domains(x0, y0, x1, y1, x_axis, y_axis, diag) {
            last_pts.fill(None);
            continue;
        }
        for (xi, x_seg) in x_axis.segments.iter().enumerate() {
            for (yi, y_seg) in y_axis.segments.iter().enumerate() {
                let cell_idx = xi * ny + yi;
                if let Some((p0, p1)) = clip_edge_to_segment_cell((x0, y0), (x1, y1), x_seg, y_seg) {
                    if let (Some(path), Some(last)) = (cell_paths.get_mut(cell_idx), last_pts.get_mut(cell_idx)) {
                        append_clipped_edge(path, last, p0, p1);
                    }
                } else if let Some(last) = last_pts.get_mut(cell_idx) {
                    *last = None;
                }
            }
        }
    }

    collect_non_empty_cell_paths(x_axis, y_axis, cell_paths)
}

fn validate_pair_domains(x0: f64, y0: f64, x1: f64, y1: f64, x_axis: &ResolvedAxis, y_axis: &ResolvedAxis, diag: &mut ScaleDiagnostics) -> bool {
    let mut ok = true;
    for x in [x0, x1] {
        if x_axis.segments.iter().all(|s| scale_transform(s.scale, x).is_none()) {
            let scale = x_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale);
            diag.record_domain_rejection(scale, x);
            ok = false;
        }
    }
    for y in [y0, y1] {
        if y_axis.segments.iter().all(|s| scale_transform(s.scale, y).is_none()) {
            let scale = y_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale);
            diag.record_domain_rejection(scale, y);
            ok = false;
        }
    }
    ok
}

fn clip_edge_to_segment_cell(p0: (f64, f64), p1: (f64, f64), x_seg: &ResolvedSegment, y_seg: &ResolvedSegment) -> Option<(PointPt, PointPt)> {
    let gx0 = scale_transform(x_seg.scale, p0.0)?;
    let gy0 = scale_transform(y_seg.scale, p0.1)?;
    let gx1 = scale_transform(x_seg.scale, p1.0)?;
    let gy1 = scale_transform(y_seg.scale, p1.1)?;

    let gx_min = scale_transform(x_seg.scale, x_seg.domain_min)?;
    let gx_max = scale_transform(x_seg.scale, x_seg.domain_max)?;
    let gy_min = scale_transform(y_seg.scale, y_seg.domain_min)?;
    let gy_max = scale_transform(y_seg.scale, y_seg.domain_max)?;

    let (t0, t1) = liang_barsky_clip((gx0, gy0), (gx1, gy1), (gx_min.min(gx_max), gy_min.min(gy_max)), (gx_min.max(gx_max), gy_min.max(gy_max)))?;

    let dx = gx1 - gx0;
    let dy = gy1 - gy0;
    let cx0 = map_transformed_to_span(gx0 + t0 * dx, gx_min, gx_max, x_seg.span_start_pt, x_seg.span_end_pt)?;
    let cy0 = map_transformed_to_span(gy0 + t0 * dy, gy_min, gy_max, y_seg.span_start_pt, y_seg.span_end_pt)?;
    let cx1 = map_transformed_to_span(gx0 + t1 * dx, gx_min, gx_max, x_seg.span_start_pt, x_seg.span_end_pt)?;
    let cy1 = map_transformed_to_span(gy0 + t1 * dy, gy_min, gy_max, y_seg.span_start_pt, y_seg.span_end_pt)?;
    Some((PointPt::new(cx0, cy0), PointPt::new(cx1, cy1)))
}

fn liang_barsky_clip(p0: (f64, f64), p1: (f64, f64), b_min: (f64, f64), b_max: (f64, f64)) -> Option<(f64, f64)> {
    let dx = p1.0 - p0.0;
    let dy = p1.1 - p0.1;
    let mut t_enter = 0.0_f64;
    let mut t_leave = 1.0_f64;
    let edges = [(-dx, p0.0 - b_min.0), (dx, b_max.0 - p0.0), (-dy, p0.1 - b_min.1), (dy, b_max.1 - p0.1)];
    for (p, q) in edges {
        if p.abs() <= 1e-15 {
            if q < -1e-12 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t_leave {
                    return None;
                }
                t_enter = t_enter.max(r);
            } else {
                if r < t_enter {
                    return None;
                }
                t_leave = t_leave.min(r);
            }
        }
    }
    if t_enter <= t_leave { Some((t_enter, t_leave)) } else { None }
}

fn map_transformed_to_span(t_val: f64, t_min: f64, t_max: f64, span_start: f64, span_end: f64) -> Option<f64> {
    let denom = t_max - t_min;
    if !denom.is_finite() || denom.abs() <= MIN_FINITE_AXIS_SPAN {
        return None;
    }
    let u = (t_val - t_min) / denom;
    let out = span_start + u * (span_end - span_start);
    out.is_finite().then_some(out)
}

fn append_clipped_edge(path: &mut Path2D, last_pt: &mut Option<PointPt>, p0: PointPt, p1: PointPt) {
    let same_start = last_pt.is_some_and(|prev| (prev.x - p0.x).abs() <= 1e-4 && (prev.y - p0.y).abs() <= 1e-4);
    if !same_start {
        path.commands.push(PathCmd::MoveTo(p0));
    }
    path.commands.push(PathCmd::LineTo(p1));
    *last_pt = Some(p1);
}

fn collect_non_empty_cell_paths(x_axis: &ResolvedAxis, y_axis: &ResolvedAxis, cell_paths: Vec<Path2D>) -> Vec<ClippedSegmentPath> {
    let ny = y_axis.segments.len();
    let mut out = Vec::new();
    for (xi, x_seg) in x_axis.segments.iter().enumerate() {
        for (yi, y_seg) in y_axis.segments.iter().enumerate() {
            if let Some(path) = cell_paths.get(xi * ny + yi)
                && !path.is_empty()
            {
                out.push(ClippedSegmentPath {
                    x_segment_index: xi,
                    y_segment_index: yi,
                    clip_rect: segment_cell_rect(x_seg, y_seg),
                    path: path.clone(),
                });
            }
        }
    }
    out
}

fn segment_data_bounds_hint(idx: usize, count: usize, data_bounds: Option<(f64, f64)>) -> Option<(f64, f64)> {
    let (lo, hi) = data_bounds?;
    if count <= 1 {
        return Some((lo, hi));
    }
    if idx == 0 {
        Some((lo, lo))
    } else if idx + 1 == count {
        Some((hi, hi))
    } else {
        None
    }
}

fn normalize_segment_slice(segments: &[AxisSegment], title: &str, warnings: &mut Vec<ChartWarning>) -> Vec<AxisSegment> {
    if segments.is_empty() {
        warnings.push(ChartWarning::new("empty_axis_segments", format!("axis '{title}' had 0 segments; defaulted to 1 linear segment")));
        return vec![AxisSegment::default()];
    }
    if segments.len() > MAX_SEGMENTS_PER_AXIS {
        warnings.push(ChartWarning::new(
            "too_many_axis_segments",
            format!("axis '{title}' had {} segments; clamped to {MAX_SEGMENTS_PER_AXIS}", segments.len()),
        ));
        return segments[..MAX_SEGMENTS_PER_AXIS].to_vec();
    }
    segments.to_vec()
}

fn normalize_length_shares(segments: &[AxisSegment]) -> Vec<f64> {
    let valid_shares: Vec<f64> =
        segments.iter().map(|s| if s.length_share.is_finite() && s.length_share > 0.0 { s.length_share } else { 1.0 }).collect();
    let sum: f64 = valid_shares.iter().sum();
    if sum <= 0.0 || !sum.is_finite() {
        return vec![1.0 / (segments.len().max(1) as f64); segments.len()];
    }
    valid_shares.into_iter().map(|w| w / sum).collect()
}

fn default_domain_for_scale(scale: AxisScale, prev_hi: Option<f64>) -> (f64, f64) {
    match scale {
        AxisScale::Linear => {
            let lo = prev_hi.map_or(0.0, |p| p + 10.0);
            (lo, lo + 10.0)
        }
        AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => {
            let lo = prev_hi.filter(|v| *v > 0.0).map_or(1.0, |p| p * 10.0);
            (lo, lo * 10.0)
        }
        AxisScale::Logit(LogitDomain::Proportion) => (0.05, 0.95),
        AxisScale::Logit(LogitDomain::Percent) => (5.0, 95.0),
        AxisScale::Probit => (1.0, 99.0),
        AxisScale::Reciprocal => {
            let lo = prev_hi.filter(|v| *v > 0.0).map_or(1.0, |p| p * 2.0);
            (lo, lo * 10.0)
        }
    }
}

const PERCENT_TAIL_DISTANCES: &[f64] = &[10.0, 5.0, 1.0, 0.5, 0.1, 0.05, 0.01, 0.005, 0.001, 0.0005, 0.0001, 1e-5, 1e-6, 1e-8, 1e-10, 1e-12];

pub(crate) fn is_log_like_scale(scale: AxisScale) -> bool {
    matches!(scale, AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln)
}

pub(crate) fn prob_auto_domain_min(raw_min: f64, upper: f64, scale: AxisScale) -> f64 {
    let mult = upper / 100.0;
    let target = raw_min * 0.5;
    for &tail_pct in PERCENT_TAIL_DISTANCES {
        let cand = tail_pct * mult;
        if cand <= target && cand <= raw_min && scale_transform(scale, cand).is_some() {
            return cand;
        }
    }
    if target > 0.0 && target < upper && scale_transform(scale, target).is_some() { target } else { raw_min }
}

pub(crate) fn prob_auto_domain_max(raw_max: f64, upper: f64, scale: AxisScale) -> f64 {
    let mult = upper / 100.0;
    let target_tail = (upper - raw_max) * 0.5;
    for &tail_pct in PERCENT_TAIL_DISTANCES {
        let tail = tail_pct * mult;
        let cand = upper - tail;
        if tail <= target_tail && cand >= raw_max && scale_transform(scale, cand).is_some() {
            return cand;
        }
    }
    let padded = upper - target_tail;
    if padded > 0.0 && padded < upper && scale_transform(scale, padded).is_some() { padded.max(raw_max) } else { raw_max }
}

fn linear_constant_pad(val: f64) -> f64 {
    if !val.is_finite() || val == 0.0 { 1.0 } else { (val.abs() * 0.1).max(MIN_FINITE_AXIS_SPAN) }
}

fn is_degenerate_domain(dmin: f64, dmax: f64, scale: AxisScale) -> bool {
    if !dmin.is_finite() || !dmax.is_finite() {
        return true;
    }
    if matches!(scale, AxisScale::Linear) {
        let span = dmax - dmin;
        !span.is_finite() || span.abs() <= MIN_FINITE_AXIS_SPAN
    } else {
        match (scale_transform(scale, dmin), scale_transform(scale, dmax)) {
            (Some(t_min), Some(t_max)) => (t_max - t_min).abs() < 1e-12,
            _ => true,
        }
    }
}

fn sample_default_domain(scale: AxisScale) -> (f64, f64) {
    match scale {
        AxisScale::Linear => (0.0, 1.0),
        AxisScale::Log10 | AxisScale::Ln | AxisScale::Reciprocal => (1.0, 100.0),
        AxisScale::Log2 => (1.0, 16.0),
        AxisScale::Logit(LogitDomain::Percent) | AxisScale::Probit => (1.0, 99.0),
        AxisScale::Logit(LogitDomain::Proportion) => (0.01, 0.99),
    }
}

fn expand_log_degenerate(center: f64, lo_mult: f64, hi_mult: f64, scale: AxisScale) -> (f64, f64) {
    if !center.is_finite() || center <= 0.0 || scale_transform(scale, center).is_none() {
        return sample_default_domain(scale);
    }
    let raw_lo = center * lo_mult;
    let lo = if raw_lo.is_finite() && raw_lo > 0.0 && scale_transform(scale, raw_lo).is_some() { raw_lo } else { center };
    let raw_hi = center * hi_mult;
    let hi = if raw_hi.is_finite() && scale_transform(scale, raw_hi).is_some() { raw_hi } else { f64::MAX };
    if lo.is_finite() && hi.is_finite() && lo < hi && scale_transform(scale, lo).is_some() && scale_transform(scale, hi).is_some() {
        (lo, hi)
    } else {
        sample_default_domain(scale)
    }
}

fn expand_degenerate_domain(center: f64, scale: AxisScale) -> (f64, f64) {
    match scale {
        AxisScale::Linear => {
            let safe_center = if center.is_finite() { center } else { 0.0 };
            let pad = linear_constant_pad(safe_center);
            (safe_center - pad, safe_center + pad)
        }
        AxisScale::Log10 => expand_log_degenerate(center, 0.1, 10.0, scale),
        AxisScale::Log2 | AxisScale::Ln => expand_log_degenerate(center, 0.5, 2.0, scale),
        AxisScale::Logit(LogitDomain::Percent) | AxisScale::Probit => {
            if center.is_finite() && center > 0.0 && center < 100.0 {
                let lo = prob_auto_domain_min(center, 100.0, scale);
                let hi = prob_auto_domain_max(center, 100.0, scale);
                if lo < hi && scale_transform(scale, lo).is_some() && scale_transform(scale, hi).is_some() { (lo, hi) } else { (1.0, 99.0) }
            } else {
                (1.0, 99.0)
            }
        }
        AxisScale::Logit(LogitDomain::Proportion) => {
            if center.is_finite() && center > 0.0 && center < 1.0 {
                let lo = prob_auto_domain_min(center, 1.0, scale);
                let hi = prob_auto_domain_max(center, 1.0, scale);
                if lo < hi && scale_transform(scale, lo).is_some() && scale_transform(scale, hi).is_some() { (lo, hi) } else { (0.01, 0.99) }
            } else {
                (0.01, 0.99)
            }
        }
        AxisScale::Reciprocal => {
            if !center.is_finite() || center == 0.0 {
                sample_default_domain(AxisScale::Reciprocal)
            } else if center > 0.0 {
                (center * 0.5, center * 2.0)
            } else {
                (center * 2.0, center * 0.5)
            }
        }
    }
}

fn filter_reciprocal_segment_values(seg: &AxisSegment, valid: &[f64], warnings: &mut Vec<ChartWarning>) -> Vec<f64> {
    if let Some(anchor) = seg.min.filter(|v| v.is_finite() && *v != 0.0).or_else(|| seg.max.filter(|v| v.is_finite() && *v != 0.0)) {
        return valid.iter().copied().filter(|&v| v * anchor > 0.0).collect();
    }
    let has_pos = valid.iter().any(|&v| v > 0.0);
    let has_neg = valid.iter().any(|&v| v < 0.0);
    if has_pos && has_neg {
        warnings.push(ChartWarning::new(
            "reciprocal_domain_crosses_zero",
            "reciprocal axis cannot span zero on a single segment; auto-domain restricted to positive values (use a broken axis to show both signs)",
        ));
        return valid.iter().copied().filter(|&v| v > 0.0).collect();
    }
    valid.to_vec()
}

fn parse_explicit_bound(raw: Option<f64>, scale: AxisScale, label: &str, warnings: &mut Vec<ChartWarning>) -> Option<f64> {
    match raw {
        Some(v) if v.is_finite() && scale_transform(scale, v).is_some() => Some(v),
        Some(v) => {
            warnings.push(ChartWarning::new("invalid_axis_domain", format!("ignored invalid {label} {v} for {scale:?} axis")));
            None
        }
        None => None,
    }
}

fn resolve_segment_domain(
    seg: &AxisSegment,
    data_bounds: Option<(f64, f64)>,
    prev_hi: Option<f64>,
    axis_title: &str,
    warnings: &mut Vec<ChartWarning>,
) -> (f64, f64) {
    let (def_lo, def_hi) = default_domain_for_scale(seg.scale, prev_hi);
    let (auto_lo, auto_hi) = match data_bounds {
        Some((d_lo, d_hi)) if d_lo.is_finite() && d_hi.is_finite() => compute_nice_bounds_for_scale(d_lo.min(d_hi), d_lo.max(d_hi), seg.scale),
        _ => (def_lo, def_hi),
    };
    let explicit_min = parse_explicit_bound(seg.min, seg.scale, "minimum", warnings);
    let explicit_max = parse_explicit_bound(seg.max, seg.scale, "maximum", warnings);
    let mut lo = explicit_min.unwrap_or(auto_lo);
    let mut hi = explicit_max.unwrap_or(auto_hi);
    if explicit_min.is_some() && explicit_max.is_none() && hi <= lo {
        hi = extend_upper_bound(lo, seg.scale);
    }
    if explicit_max.is_some() && explicit_min.is_none() && lo >= hi {
        lo = extend_lower_bound(hi, seg.scale);
    }
    if is_degenerate_domain(lo, hi, seg.scale) {
        if explicit_min.is_some() && explicit_max.is_some() {
            warnings.push(ChartWarning::new(
                "invalid_axis_domain",
                format!("expanded degenerate explicit domain [{lo}, {hi}] for {:?} on axis '{axis_title}'", seg.scale),
            ));
        }
        (lo, hi) = expand_degenerate_domain(lo, seg.scale);
    }

    let valid_lo = scale_transform(seg.scale, lo).is_some();
    let valid_hi = scale_transform(seg.scale, hi).is_some();
    let span = hi - lo;
    let valid_span = span.is_finite() && span.abs() > MIN_FINITE_AXIS_SPAN;
    let valid_reciprocal = !matches!(seg.scale, AxisScale::Reciprocal) || (lo != 0.0 && hi != 0.0 && lo.signum() == hi.signum());

    if valid_lo && valid_hi && valid_span && valid_reciprocal {
        (lo, hi)
    } else {
        warnings.push(ChartWarning::new(
            "invalid_axis_domain",
            format!("repaired invalid domain [{lo}, {hi}] for {:?} on axis '{axis_title}'", seg.scale),
        ));
        (def_lo, def_hi)
    }
}

pub(crate) fn resolve_sample_segment_domain(
    seg: &AxisSegment,
    data_values: &[f64],
    include_zero_if_linear: bool,
    top_headroom_frac: f64,
    warnings: &mut Vec<ChartWarning>,
) -> (f64, f64) {
    let scale = seg.scale;
    let valid: Vec<f64> = data_values.iter().copied().filter(|&v| v.is_finite() && scale_transform(scale, v).is_some()).collect();
    let seg_valid = if scale == AxisScale::Reciprocal { filter_reciprocal_segment_values(seg, &valid, warnings) } else { valid };

    let explicit_min = parse_explicit_bound(seg.min, scale, "minimum", warnings);
    let explicit_max = parse_explicit_bound(seg.max, scale, "maximum", warnings);

    let mut dmin = explicit_min.unwrap_or_else(|| sample_auto_domain_min(&seg_valid, scale, include_zero_if_linear));
    let mut dmax = explicit_max.unwrap_or_else(|| sample_auto_domain_max(&seg_valid, scale, top_headroom_frac));

    if explicit_min.is_none() && matches!(scale, AxisScale::Linear) && include_zero_if_linear && dmin > 0.0 && dmin <= dmax {
        dmin = 0.0;
    }
    if explicit_max.is_none() && matches!(scale, AxisScale::Linear) && include_zero_if_linear && dmax < 0.0 && dmin <= dmax {
        dmax = 0.0;
    }
    if seg_valid.is_empty() {
        if explicit_min.is_some() && explicit_max.is_none() && dmax <= dmin {
            dmax = extend_upper_bound(dmin, scale);
        } else if explicit_max.is_some() && explicit_min.is_none() && dmin >= dmax {
            dmin = extend_lower_bound(dmax, scale);
        }
    }
    if is_degenerate_domain(dmin, dmax, scale) {
        if explicit_min.is_some() && explicit_max.is_some() {
            warnings
                .push(ChartWarning::new("invalid_axis_domain", format!("expanded degenerate explicit domain [{dmin}, {dmax}] for {scale:?} axis")));
        }
        (dmin, dmax) = expand_degenerate_domain(dmin, scale);
    }
    if scale == AxisScale::Reciprocal
        && (!dmin.is_finite()
            || !dmax.is_finite()
            || dmin == 0.0
            || dmax == 0.0
            || dmin.signum() != dmax.signum()
            || is_degenerate_domain(dmin, dmax, scale))
    {
        warnings
            .push(ChartWarning::new("invalid_axis_domain", format!("repaired zero-crossing or invalid domain [{dmin}, {dmax}] for Reciprocal axis")));
        let pos: Vec<f64> = seg_valid.iter().copied().filter(|&v| v > 0.0).collect();
        let neg: Vec<f64> = seg_valid.iter().copied().filter(|&v| v < 0.0).collect();
        let chosen = if !pos.is_empty() { &pos } else { &neg };
        if !chosen.is_empty() {
            let mut rmin = sample_auto_domain_min(chosen, AxisScale::Reciprocal, false);
            let mut rmax = sample_auto_domain_max(chosen, AxisScale::Reciprocal, top_headroom_frac);
            if is_degenerate_domain(rmin, rmax, AxisScale::Reciprocal) {
                (rmin, rmax) = expand_degenerate_domain(rmin, AxisScale::Reciprocal);
            }
            if rmin.is_finite() && rmax.is_finite() && rmin != 0.0 && rmax != 0.0 && rmin.signum() == rmax.signum() {
                return (rmin, rmax);
            }
        }
        return sample_default_domain(scale);
    }
    if is_log_like_scale(scale)
        && (!dmin.is_finite()
            || !dmax.is_finite()
            || dmin <= 0.0
            || dmax <= 0.0
            || is_degenerate_domain(dmin, dmax, scale)
            || scale_transform(scale, dmin).is_none()
            || scale_transform(scale, dmax).is_none())
    {
        warnings.push(ChartWarning::new(
            "invalid_axis_domain",
            format!("repaired non-finite or non-positive domain [{dmin}, {dmax}] for {scale:?} axis"),
        ));
        return sample_default_domain(scale);
    }
    (dmin, dmax)
}

pub(crate) fn sanitize_linear_range(min_v: f64, max_v: f64, fallback: (f64, f64)) -> (f64, f64) {
    if !min_v.is_finite() || !max_v.is_finite() {
        return fallback;
    }
    let (lo, hi) = if min_v <= max_v { (min_v, max_v) } else { (max_v, min_v) };
    let span = hi - lo;
    if !span.is_finite() || span.abs() <= MIN_FINITE_AXIS_SPAN {
        let pad = linear_constant_pad(lo);
        (lo - pad, hi + pad)
    } else {
        (lo, hi)
    }
}

fn repair_prob_bound(
    val: f64,
    is_min: bool,
    is_explicit: bool,
    raw_fallback: f64,
    upper: f64,
    scale: AxisScale,
    warnings: &mut Vec<ChartWarning>,
) -> f64 {
    if val.is_finite() && scale_transform(scale, val).is_some() {
        return val;
    }
    if is_explicit {
        warnings.push(ChartWarning::new("invalid_axis_domain", format!("repaired out-of-domain bound {val} for {scale:?} axis")));
    }
    if !is_explicit && raw_fallback.is_finite() && scale_transform(scale, raw_fallback).is_some() {
        return if is_min { prob_auto_domain_min(raw_fallback, upper, scale) } else { prob_auto_domain_max(raw_fallback, upper, scale) };
    }
    let edge = 0.01 * (upper / 100.0);
    if is_min { edge } else { upper - edge }
}

/// Resolve a single-segment axis domain using pre-padded `(data_min, data_max)` while honoring
/// explicit (including reversed) bounds and guarding log/logit/probit/reciprocal limits.
pub(crate) fn resolve_exact_domain_with_warnings(axis: &AxisSpec, data_min: f64, data_max: f64, warnings: &mut Vec<ChartWarning>) -> (f64, f64) {
    let segs = normalize_segment_slice(&axis.segments, &axis.title, warnings);
    let seg = segs.first();
    let scale = seg.map_or(AxisScale::Linear, |s| s.scale);
    let (raw_lo, raw_hi) = sanitize_linear_range(data_min, data_max, (0.0, 1.0));

    let explicit_min = match seg.and_then(|s| s.min) {
        Some(v) if v.is_finite() => Some(v),
        Some(v) => {
            warnings.push(ChartWarning::new("invalid_axis_domain", format!("ignored non-finite minimum {v} for {scale:?} axis")));
            None
        }
        None => None,
    };
    let explicit_max = match seg.and_then(|s| s.max) {
        Some(v) if v.is_finite() => Some(v),
        Some(v) => {
            warnings.push(ChartWarning::new("invalid_axis_domain", format!("ignored non-finite maximum {v} for {scale:?} axis")));
            None
        }
        None => None,
    };

    let mut lo = explicit_min.unwrap_or(raw_lo);
    let mut hi = explicit_max.unwrap_or(raw_hi);
    let init_span = hi - lo;
    let is_init_degen = if matches!(scale, AxisScale::Linear) {
        !init_span.is_finite() || init_span.abs() <= MIN_FINITE_AXIS_SPAN
    } else {
        !init_span.is_finite() || init_span.abs() <= 1e-12
    };
    if is_init_degen {
        if explicit_min.is_some() && explicit_max.is_some() {
            warnings.push(ChartWarning::new("invalid_axis_domain", format!("expanded degenerate explicit domain [{lo}, {hi}] for {scale:?} axis")));
        }
        (lo, hi) = sanitize_linear_range(lo, hi, (0.0, 1.0));
    }

    match scale {
        AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => {
            if lo <= 0.0 || hi <= 0.0 || scale_transform(scale, lo).is_none() || scale_transform(scale, hi).is_none() {
                if explicit_min.is_some() || explicit_max.is_some() {
                    warnings.push(ChartWarning::new("invalid_axis_domain", format!("repaired non-positive domain [{lo}, {hi}] for {scale:?} axis")));
                }
                if lo <= 0.0 {
                    lo = if raw_lo > 0.0 && scale_transform(scale, raw_lo).is_some() { raw_lo } else { 0.1 };
                }
                if hi <= 0.0 {
                    hi = if raw_hi > 0.0 && scale_transform(scale, raw_hi).is_some() { raw_hi } else { lo * 10.0 };
                }
                if (hi - lo).abs() <= 1e-12 || scale_transform(scale, lo).is_none() || scale_transform(scale, hi).is_none() {
                    return sample_default_domain(scale);
                }
            }
        }
        AxisScale::Logit(domain) => {
            let upper = match domain {
                LogitDomain::Percent => 100.0,
                LogitDomain::Proportion => 1.0,
            };
            lo = repair_prob_bound(lo, true, explicit_min.is_some(), raw_lo, upper, scale, warnings);
            hi = repair_prob_bound(hi, false, explicit_max.is_some(), raw_hi, upper, scale, warnings);
            if (hi - lo).abs() <= 1e-12 {
                let edge = 0.01 * (upper / 100.0);
                lo = edge;
                hi = upper - edge;
            }
        }
        AxisScale::Probit => {
            lo = repair_prob_bound(lo, true, explicit_min.is_some(), raw_lo, 100.0, scale, warnings);
            hi = repair_prob_bound(hi, false, explicit_max.is_some(), raw_hi, 100.0, scale, warnings);
            if (hi - lo).abs() <= 1e-12 {
                lo = 0.01;
                hi = 99.99;
            }
        }
        AxisScale::Reciprocal => {
            if !lo.is_finite() || !hi.is_finite() || lo == 0.0 || hi == 0.0 || lo.signum() != hi.signum() || (hi - lo).abs() <= 1e-12 {
                if explicit_min.is_some() || explicit_max.is_some() || (lo < 0.0 && hi > 0.0) || (lo > 0.0 && hi < 0.0) {
                    warnings.push(ChartWarning::new(
                        "invalid_axis_domain",
                        format!("repaired zero-crossing or invalid domain [{lo}, {hi}] for Reciprocal axis"),
                    ));
                }
                if hi > 0.0 && lo <= 0.0 {
                    lo = if raw_lo > 0.0 && raw_lo < hi { raw_lo } else { (hi * 0.01).clamp(1e-6, hi * 0.5) };
                } else if lo > 0.0 && hi <= 0.0 {
                    hi = if raw_hi > 0.0 && raw_hi < lo { raw_hi } else { (lo * 0.01).clamp(1e-6, lo * 0.5) };
                } else if lo < 0.0 && hi >= 0.0 {
                    hi = if raw_hi < 0.0 && raw_hi > lo { raw_hi } else { (lo * 0.01).clamp(lo * 0.5, -1e-6) };
                } else if hi < 0.0 && lo >= 0.0 {
                    lo = if raw_lo < 0.0 && raw_lo > hi { raw_lo } else { (hi * 0.01).clamp(hi * 0.5, -1e-6) };
                } else {
                    return (1.0, 100.0);
                }
                if !lo.is_finite() || !hi.is_finite() || lo == 0.0 || hi == 0.0 || lo.signum() != hi.signum() || (hi - lo).abs() <= 1e-12 {
                    return (1.0, 100.0);
                }
            }
        }
        AxisScale::Linear => {}
    }
    (lo, hi)
}

fn log_auto_domain_min(raw_min: f64, scale: AxisScale) -> f64 {
    let candidate = match scale {
        AxisScale::Log10 => 10.0_f64.powf(raw_min.log10().floor()),
        AxisScale::Log2 => 2.0_f64.powf(raw_min.log2().floor()),
        AxisScale::Ln => (raw_min.ln() - 0.15).exp(),
        _ => raw_min,
    };
    if candidate.is_finite() && candidate > 0.0 && scale_transform(scale, candidate).is_some() { candidate.min(raw_min) } else { raw_min }
}

fn log_auto_domain_max(raw_max: f64, scale: AxisScale, top_headroom_frac: f64) -> f64 {
    let headroom = if top_headroom_frac.is_finite() { top_headroom_frac.max(0.0) } else { 0.0 };
    let padded = raw_max * (1.0 + headroom);
    let candidate = match scale {
        AxisScale::Log10 if padded.is_finite() => 10.0_f64.powf(padded.log10().ceil()),
        AxisScale::Log2 if padded.is_finite() => 2.0_f64.powf(padded.log2().ceil()),
        AxisScale::Ln => (raw_max.ln() + 0.15 + headroom).exp(),
        _ => f64::INFINITY,
    };
    if candidate.is_finite() && candidate > 0.0 && scale_transform(scale, candidate).is_some() {
        candidate.max(raw_max)
    } else if padded.is_finite() && padded > 0.0 && scale_transform(scale, padded).is_some() {
        padded.max(raw_max)
    } else {
        f64::MAX
    }
}

fn sample_auto_domain_min(valid: &[f64], scale: AxisScale, include_zero_if_linear: bool) -> f64 {
    let raw_min = valid.iter().copied().fold(f64::INFINITY, f64::min);
    let raw_max = valid.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !raw_min.is_finite() {
        return sample_default_domain(scale).0;
    }
    match scale {
        AxisScale::Linear => {
            let mut base = if include_zero_if_linear && raw_min > 0.0 { 0.0 } else { raw_min };
            let raw_span = (raw_max - base).max(base.abs() * 0.1);
            let span = if raw_span.is_finite() && raw_span > MIN_FINITE_AXIS_SPAN { raw_span } else { 1.0 };
            if !include_zero_if_linear || base < 0.0 {
                base -= 0.06 * span;
            }
            let step = nice_linear_step(span / 5.0);
            clean_zero((base / step).floor() * step)
        }
        AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => log_auto_domain_min(raw_min, scale),
        AxisScale::Logit(LogitDomain::Percent) | AxisScale::Probit => prob_auto_domain_min(raw_min, 100.0, scale),
        AxisScale::Logit(LogitDomain::Proportion) => prob_auto_domain_min(raw_min, 1.0, scale),
        AxisScale::Reciprocal => {
            if raw_min > 0.0 {
                raw_min * 0.85
            } else {
                raw_min * 1.15
            }
        }
    }
}

fn sample_auto_domain_max(valid: &[f64], scale: AxisScale, top_headroom_frac: f64) -> f64 {
    let raw_min = valid.iter().copied().fold(f64::INFINITY, f64::min);
    let raw_max = valid.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !raw_max.is_finite() {
        return sample_default_domain(scale).1;
    }
    match scale {
        AxisScale::Linear => {
            let raw_span = (raw_max - raw_min).max(raw_max.abs() * 0.1);
            let span = if raw_span.is_finite() && raw_span > MIN_FINITE_AXIS_SPAN { raw_span } else { 1.0 };
            let padded = raw_max + span * (0.08 + top_headroom_frac.max(0.0));
            let step = nice_linear_step(span / 5.0);
            clean_zero((padded / step).ceil() * step)
        }
        AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => log_auto_domain_max(raw_max, scale, top_headroom_frac),
        AxisScale::Logit(LogitDomain::Percent) | AxisScale::Probit => prob_auto_domain_max(raw_max, 100.0, scale),
        AxisScale::Logit(LogitDomain::Proportion) => prob_auto_domain_max(raw_max, 1.0, scale),
        AxisScale::Reciprocal => {
            if raw_max > 0.0 {
                raw_max * 1.15
            } else {
                raw_max * 0.85
            }
        }
    }
}

fn compute_nice_bounds_for_scale(lo: f64, hi: f64, scale: AxisScale) -> (f64, f64) {
    match scale {
        AxisScale::Linear => nice_linear_bounds(lo, hi),
        AxisScale::Log10 => nice_log_bounds(lo, hi, 10.0, scale),
        AxisScale::Log2 => nice_log_bounds(lo, hi, 2.0, scale),
        AxisScale::Ln => nice_log_bounds(lo, hi, std::f64::consts::E, scale),
        AxisScale::Logit(LogitDomain::Proportion) => {
            let auto_lo = if lo > 0.0 && lo < 1.0 { prob_auto_domain_min(lo, 1.0, scale) } else { 0.05 };
            let auto_hi = if hi > 0.0 && hi < 1.0 { prob_auto_domain_max(hi, 1.0, scale) } else { 0.95 };
            if auto_lo < auto_hi && scale_transform(scale, auto_lo).is_some() && scale_transform(scale, auto_hi).is_some() {
                (auto_lo, auto_hi)
            } else {
                (0.05, 0.95)
            }
        }
        AxisScale::Logit(LogitDomain::Percent) | AxisScale::Probit => {
            let auto_lo = if lo > 0.0 && lo < 100.0 { prob_auto_domain_min(lo, 100.0, scale) } else { 1.0 };
            let auto_hi = if hi > 0.0 && hi < 100.0 { prob_auto_domain_max(hi, 100.0, scale) } else { 99.0 };
            if auto_lo < auto_hi && scale_transform(scale, auto_lo).is_some() && scale_transform(scale, auto_hi).is_some() {
                (auto_lo, auto_hi)
            } else {
                (1.0, 99.0)
            }
        }
        AxisScale::Reciprocal => {
            if lo > 0.0 && hi > 0.0 {
                (lo, if hi > lo { hi } else { lo * 10.0 })
            } else if lo < 0.0 && hi < 0.0 {
                (lo, if hi > lo { hi } else { lo * 0.1 })
            } else {
                (1.0, 10.0)
            }
        }
    }
}

fn nice_linear_bounds(lo: f64, hi: f64) -> (f64, f64) {
    let span = hi - lo;
    if !span.is_finite() || span.abs() <= MIN_FINITE_AXIS_SPAN {
        let pad = linear_constant_pad(lo);
        return (lo - pad, hi + pad);
    }
    let step = nice_linear_step(span / 5.0);
    let nice_lo = if lo >= 0.0 && lo <= step { 0.0 } else { (lo / step).floor() * step };
    let nice_hi = (hi / step).ceil() * step;
    let out_hi = if nice_hi.is_finite() { nice_hi } else { hi };
    let out_lo = if nice_lo.is_finite() { nice_lo } else { lo };
    (clean_zero(out_lo), clean_zero(out_hi))
}

fn nice_log_bounds(lo: f64, hi: f64, base: f64, scale: AxisScale) -> (f64, f64) {
    let safe_lo = if lo > 0.0 && lo.is_finite() && scale_transform(scale, lo).is_some() {
        lo
    } else if hi > 0.0 && hi.is_finite() && scale_transform(scale, hi * 0.01).is_some() {
        hi * 0.01
    } else {
        1.0
    };
    let safe_hi = if hi > safe_lo && hi.is_finite() && scale_transform(scale, hi).is_some() { hi } else { safe_lo * base };
    let out_lo = log_auto_domain_min(safe_lo, scale);
    let out_hi = log_auto_domain_max(safe_hi, scale, 0.0);
    if out_lo.is_finite() && out_hi.is_finite() && out_lo < out_hi { (out_lo, out_hi) } else { (safe_lo, safe_hi.max(safe_lo * base)) }
}

fn extend_upper_bound(lo: f64, scale: AxisScale) -> f64 {
    match scale {
        AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => {
            let scaled = lo.max(LOG_POSITIVE_FLOOR_DEFAULT) * 10.0;
            if scaled.is_finite() && scaled > lo { scaled } else { f64::MAX }
        }
        AxisScale::Logit(LogitDomain::Proportion) => prob_auto_domain_max(lo, 1.0, scale),
        AxisScale::Logit(LogitDomain::Percent) | AxisScale::Probit => prob_auto_domain_max(lo, 100.0, scale),
        AxisScale::Linear => lo + linear_constant_pad(lo),
        AxisScale::Reciprocal => {
            if lo < 0.0 {
                lo * 0.5
            } else {
                lo + lo.abs().max(1.0)
            }
        }
    }
}

fn extend_lower_bound(hi: f64, scale: AxisScale) -> f64 {
    match scale {
        AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => (hi * 0.1).max(LOG_POSITIVE_FLOOR_DEFAULT),
        AxisScale::Logit(LogitDomain::Proportion) => prob_auto_domain_min(hi, 1.0, scale),
        AxisScale::Logit(LogitDomain::Percent) | AxisScale::Probit => prob_auto_domain_min(hi, 100.0, scale),
        AxisScale::Linear => hi - linear_constant_pad(hi),
        AxisScale::Reciprocal => {
            if hi < 0.0 {
                hi * 2.0
            } else {
                (hi * 0.5).max(LOG_POSITIVE_FLOOR_DEFAULT)
            }
        }
    }
}

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
        AxisScale::Log10 => {
            if seg.major_tick_interval.is_some_and(|s| s.is_finite() && s > 0.0) && target_major_override.is_some() {
                linear_major_minor(seg, target_major, warnings)
            } else {
                log10_major_minor(seg, auto_log_minor_ticks, warnings)
            }
        }
        AxisScale::Log2 => log_base_major_minor(seg, 2.0, warnings),
        AxisScale::Ln => log_base_major_minor(seg, std::f64::consts::E, warnings),
        AxisScale::Logit(LogitDomain::Percent) => candidate_prob_ticks(seg, LOGIT_PCT_TICKS, warnings),
        AxisScale::Logit(LogitDomain::Proportion) => candidate_prob_ticks(seg, LOGIT_PROP_TICKS, warnings),
        AxisScale::Probit => candidate_prob_ticks(seg, PROBIT_PCT_TICKS, warnings),
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
    let tol = 1e-12 * (hi - lo).abs().max(lo.abs()).max(hi.abs());
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

fn nice_linear_step(rough: f64) -> f64 {
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
    let span = hi - lo;
    if !lo.is_finite() || !hi.is_finite() || !span.is_finite() || span <= 0.0 {
        let pt = if lo.is_finite() { vec![clean_zero(lo)] } else { Vec::new() };
        return (pt, 1.0, 0);
    }

    let max_intervals = (MAX_TICKS_PER_SEGMENT.saturating_sub(1)).max(1) as f64;
    let mut step = match requested_step {
        Some(s) if s.is_finite() && s > 0.0 => s,
        _ => nice_linear_step(span / (target_ticks.max(2) as f64)),
    };
    let min_safe_step = (span / max_intervals).max(lo.abs().max(hi.abs()) * 1e-13);
    let raw_intervals = span / step;
    if !raw_intervals.is_finite() || raw_intervals > max_intervals || step < min_safe_step {
        let stride = (raw_intervals / max_intervals).ceil();
        let eff_step = if raw_intervals.is_finite() && stride.is_finite() && stride <= 1_000_000.0 {
            let scaled = step * stride;
            if scaled.is_finite() && scaled >= min_safe_step { scaled } else { nice_linear_step(span / 8.0).max(min_safe_step) }
        } else {
            nice_linear_step(span / 8.0).max(min_safe_step)
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
        step = nice_linear_step(span / 8.0).max(min_safe_step);
        start_k_f = (lo / step - 1e-9).ceil();
        end_k_f = (hi / step + 1e-9).floor();
    }
    if !start_k_f.is_finite() || !end_k_f.is_finite() || end_k_f < start_k_f {
        return (vec![clean_zero(lo), clean_zero(hi)], step, 0);
    }

    let count = ((end_k_f - start_k_f).round().max(0.0) as usize).saturating_add(1).min(MAX_TICKS_PER_SEGMENT);
    let tol = 1e-9 * span.max(step);
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

fn log10_major_minor(seg: &ResolvedSegment, auto_log_minor_ticks: bool, warnings: &mut Vec<ChartWarning>) -> (Vec<f64>, Vec<f64>) {
    if !seg.domain_min.is_finite() || !seg.domain_max.is_finite() || seg.domain_min <= 0.0 || seg.domain_max <= 0.0 {
        return (Vec::new(), Vec::new());
    }
    let lo = seg.domain_min.min(seg.domain_max);
    let hi = seg.domain_min.max(seg.domain_max);
    let Some((k_lo, k_hi, stride)) = log_exponent_range(lo, hi, 10.0) else {
        return (Vec::new(), Vec::new());
    };

    let lo_tol = lo * (1.0 - 1e-9);
    let hi_tol = if hi <= f64::MAX / (1.0 + 1e-9) { hi * (1.0 + 1e-9) } else { f64::MAX };
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
    let mut minors = Vec::new();
    let emit_minors = (auto_log_minor_ticks || seg.minor_ticks_per_interval > 0) && decades <= 8 && stride == 1;
    if emit_minors {
        let mut dec = k_lo.saturating_sub(1);
        while dec <= k_hi && minors.len() < MAX_TICKS_PER_SEGMENT {
            let base = 10.0_f64.powi(dec);
            if base.is_finite() && base > 0.0 {
                for mult in 2_u8..=9 {
                    if minors.len() >= MAX_TICKS_PER_SEGMENT {
                        break;
                    }
                    let v = f64::from(mult) * base;
                    if v.is_finite() && v > 0.0 && v >= lo_tol && v <= hi_tol {
                        let is_major = majors.iter().any(|&m| (m - v).abs() <= m * 1e-9);
                        if !is_major {
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
    }
    (majors, minors)
}

fn log_base_major_minor(seg: &ResolvedSegment, base: f64, warnings: &mut Vec<ChartWarning>) -> (Vec<f64>, Vec<f64>) {
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

    let mut minors = Vec::new();
    if seg.minor_ticks_per_interval > 0 && majors.len() >= 2 {
        let divs = usize::from(seg.minor_ticks_per_interval) + 1;
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
    }
    (majors, minors)
}

fn candidate_prob_ticks(seg: &ResolvedSegment, candidates: &[f64], warnings: &mut Vec<ChartWarning>) -> (Vec<f64>, Vec<f64>) {
    let lo = seg.domain_min.min(seg.domain_max);
    let hi = seg.domain_min.max(seg.domain_max);
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
        let mut minors = Vec::new();
        if seg.minor_ticks_per_interval > 0 {
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
        }
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

fn clean_zero(v: f64) -> f64 {
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
    let mut exp3 = if log3.is_finite() { (log3 as i32).clamp(-8, 8) } else { 0 };
    let mut scaled = abs / 10.0_f64.powi(exp3 * 3);
    while scaled >= 1000.0 && exp3 < 8 {
        exp3 += 1;
        scaled = abs / 10.0_f64.powi(exp3 * 3);
    }
    while scaled < 1.0 && exp3 > -8 {
        exp3 -= 1;
        scaled = abs / 10.0_f64.powi(exp3 * 3);
    }
    let mut rounded = format!("{scaled:.3}");
    if exp3 < 8 && rounded.split('.').next().and_then(|int_part| int_part.parse::<u64>().ok()).is_some_and(|n| n >= 1000) {
        exp3 += 1;
        scaled = abs / 10.0_f64.powi(exp3 * 3);
        rounded = format!("{scaled:.3}");
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

/// Resolve `{{info:...}}` and `{{result:...}}` template hooks using `doc`.
#[must_use]
pub fn resolve_template_hooks(template: &str, doc: &ProjectDocument, primary_data_id: Option<&SheetId>) -> String {
    let paired_info =
        primary_data_id.and_then(|id| doc.paired_info_sheet_id(id)).and_then(|info_id| doc.info_sheet(info_id)).or_else(|| doc.info_sheets.first());

    expand_template_hooks(template, |token| resolve_single_hook(token, doc, paired_info))
}

fn resolve_single_hook(token: &str, doc: &ProjectDocument, paired_info: Option<&curvebook_doc::InfoSheet>) -> Option<String> {
    let parts: Vec<&str> = token.split(':').map(str::trim).collect();
    match parts.as_slice() {
        ["info", name] if !name.is_empty() => paired_info.and_then(|i| i.get_constant(name)).map(|c| c.value.display_text()),
        ["info", sheet, name] if !sheet.is_empty() && !name.is_empty() => {
            doc.info_sheet(&SheetId::new(*sheet)).and_then(|i| i.get_constant(name)).map(|c| c.value.display_text())
        }
        ["result", sheet, param] if !sheet.is_empty() && !param.is_empty() => {
            doc.result_sheet(&SheetId::new(*sheet)).and_then(|r| r.lookup_scalar(param, None)).map(format_scalar_clean)
        }
        ["result", sheet, ds_str, param] if !sheet.is_empty() && !param.is_empty() => {
            let ds = ds_str.parse::<usize>().ok()?;
            doc.result_sheet(&SheetId::new(*sheet)).and_then(|r| r.lookup_scalar(param, Some(ds))).map(format_scalar_clean)
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

// ============================================================================
// 4. Frame Presets, Thresholds/Bands, Break Glyphs & Axis Rendering
// ============================================================================

/// Axis frame geometry mode covering all [`FramePreset`] variants plus `TopRightTicks` and `NoAxes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AxisFrameMode {
    #[default]
    OpenL,
    PlainBox,
    TopRightTicks,
    OffsetAxes,
    OriginCrosshair,
    NoAxes,
    DualYSecondaryRight,
}

impl From<FramePreset> for AxisFrameMode {
    fn from(preset: FramePreset) -> Self {
        match preset {
            FramePreset::OpenL => Self::OpenL,
            FramePreset::OffsetRangeTufte => Self::OffsetAxes,
            FramePreset::PlainBoxed => Self::PlainBox,
            FramePreset::DualYSecondaryRight => Self::DualYSecondaryRight,
            FramePreset::OriginCrossing => Self::OriginCrosshair,
        }
    }
}

/// Rendering options for axes, ticks, frames, and margins.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisRenderOptions {
    pub frame_mode: Option<AxisFrameMode>,
    pub policy: LocaleNumberPolicy,
    pub tick_font_size_pt: f64,
    pub title_font_size_pt: f64,
    pub major_tick_len_pt: f64,
    pub minor_tick_len_pt: f64,
    pub offset_gap_pt: f64,
    pub auto_log_minor_ticks: bool,
    pub show_gridlines: bool,
    pub axis_color: Rgba8,
    pub grid_color: Rgba8,
    pub x_categories: Option<Vec<CategoricalTick>>,
    pub y_categories: Option<Vec<CategoricalTick>>,
    pub y2_categories: Option<Vec<CategoricalTick>>,
    pub x_data_bounds: Option<(f64, f64)>,
    pub y_data_bounds: Option<(f64, f64)>,
    pub y2_data_bounds: Option<(f64, f64)>,
}

impl Default for AxisRenderOptions {
    fn default() -> Self {
        Self {
            frame_mode: None,
            policy: LocaleNumberPolicy::publication(),
            tick_font_size_pt: 8.5,
            title_font_size_pt: 10.0,
            major_tick_len_pt: DEFAULT_MAJOR_TICK_LEN_PT,
            minor_tick_len_pt: DEFAULT_MINOR_TICK_LEN_PT,
            offset_gap_pt: DEFAULT_AXIS_OFFSET_PT,
            auto_log_minor_ticks: true,
            show_gridlines: false,
            axis_color: Rgba8::BLACK,
            grid_color: Rgba8::rgba(210, 210, 210, 180),
            x_categories: None,
            y_categories: None,
            y2_categories: None,
            x_data_bounds: None,
            y_data_bounds: None,
            y2_data_bounds: None,
        }
    }
}

/// Compiled axes and ticks returned by [`render_axes_and_frame`] for downstream series builders.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledAxes {
    pub x_axis: ResolvedAxis,
    pub y_axis: ResolvedAxis,
    pub y2_axis: Option<ResolvedAxis>,
    pub x_ticks: Vec<AxisTick>,
    pub y_ticks: Vec<AxisTick>,
    pub y2_ticks: Vec<AxisTick>,
    pub frame_mode: AxisFrameMode,
}

/// Estimate the inner plot rectangle on a canvas of `canvas_size_pt`, leaving clearance for
/// title, rotated tick labels, Y/Y2 labels, and axis titles.
#[must_use]
pub fn compute_plot_rect(canvas_size_pt: SizePt, graph: &GraphSpec, options: &AxisRenderOptions) -> RectPt {
    let w = if canvas_size_pt.width.is_finite() { canvas_size_pt.width.max(40.0) } else { 360.0 };
    let h = if canvas_size_pt.height.is_finite() { canvas_size_pt.height.max(40.0) } else { 260.0 };
    let mode = options.frame_mode.unwrap_or_else(|| AxisFrameMode::from(graph.frame_preset));
    if mode == AxisFrameMode::NoAxes {
        let top = if graph.title.trim().is_empty() { 14.0 } else { 28.0 };
        return RectPt::new(16.0, top, (w - 32.0).max(16.0), (h - top - 16.0).max(16.0));
    }

    let extra_offset = if mode == AxisFrameMode::OffsetAxes { options.offset_gap_pt } else { 0.0 };
    let left_margin = estimate_y_margin(&graph.y_axis, options.y_categories.as_deref(), options, extra_offset);
    let right_margin = if mode == AxisFrameMode::DualYSecondaryRight || graph.y2_axis.is_some() {
        let y2_spec = graph.y2_axis.as_ref().unwrap_or(&graph.y_axis);
        estimate_y_margin(y2_spec, options.y2_categories.as_deref(), options, 0.0).max(40.0)
    } else {
        18.0
    };
    let top_margin = if graph.title.trim().is_empty() { 16.0 } else { 30.0 };
    let bottom_margin = estimate_x_margin(&graph.x_axis, options.x_categories.as_deref(), options, extra_offset);

    let plot_w = (w - left_margin - right_margin).max(24.0);
    let plot_h = (h - top_margin - bottom_margin).max(24.0);
    RectPt::new(left_margin.min(w * 0.45), top_margin.min(h * 0.4), plot_w, plot_h)
}

fn estimate_y_margin(spec: &AxisSpec, cats: Option<&[CategoricalTick]>, options: &AxisRenderOptions, extra_offset: f64) -> f64 {
    let sample_w = if let Some(list) = cats {
        list.iter().map(|c| measure_text_pt(&parse_markup(&c.label), options.tick_font_size_pt).width_pt).fold(18.0_f64, f64::max)
    } else {
        26.0
    };
    let title_space = if spec.title.trim().is_empty() { 6.0 } else { options.title_font_size_pt + 10.0 };
    crate::clamp_f64_safe(sample_w + options.major_tick_len_pt + title_space + extra_offset + 8.0, 32.0, 120.0)
}

fn estimate_x_margin(spec: &AxisSpec, cats: Option<&[CategoricalTick]>, options: &AxisRenderOptions, extra_offset: f64) -> f64 {
    let max_label_w = if let Some(list) = cats {
        list.iter().map(|c| measure_text_pt(&parse_markup(&c.label), options.tick_font_size_pt).width_pt).fold(16.0_f64, f64::max)
    } else {
        24.0
    };
    let angle_rad = spec.tick_angle_deg.to_radians().abs();
    let label_h = options.tick_font_size_pt * angle_rad.cos().abs() + max_label_w * angle_rad.sin().abs();
    let title_space = if spec.title.trim().is_empty() { 6.0 } else { options.title_font_size_pt + 10.0 };
    crate::clamp_f64_safe(label_h + options.major_tick_len_pt + title_space + extra_offset + 10.0, 28.0, 120.0)
}

/// Render background threshold zones (`shade_to: Some(...)`) and optional gridlines behind data.
pub fn render_thresholds_background(axes: &CompiledAxes, graph: &GraphSpec, scene: &mut ChartScene, options: &AxisRenderOptions) {
    let mut drawn_x: Vec<f64> = Vec::new();
    let mut drawn_y: Vec<f64> = Vec::new();
    if options.show_gridlines {
        emit_axis_gridlines(axes, graph, &options.policy, scene, options.grid_color, &mut drawn_x, &mut drawn_y);
    }
    emit_additional_tick_gridlines_dedup(axes, graph, scene, options, &drawn_x, &drawn_y);
    emit_threshold_zones(&graph.x_axis.thresholds, &axes.x_axis, &axes.y_axis, scene);
    emit_threshold_zones(&graph.y_axis.thresholds, &axes.y_axis, &axes.x_axis, scene);
    if let (Some(y2_spec), Some(y2_axis)) = (&graph.y2_axis, &axes.y2_axis) {
        emit_threshold_zones(&y2_spec.thresholds, y2_axis, &axes.x_axis, scene);
    }
}

/// Render foreground threshold lines and optional threshold labels across matching axis segments.
pub fn render_thresholds_lines(axes: &CompiledAxes, graph: &GraphSpec, doc: &ProjectDocument, scene: &mut ChartScene) {
    let primary_id = Some(&graph.primary_source_sheet_id);
    emit_threshold_rules(&graph.x_axis.thresholds, &axes.x_axis, &axes.y_axis, doc, primary_id, scene);
    emit_threshold_rules(&graph.y_axis.thresholds, &axes.y_axis, &axes.x_axis, doc, primary_id, scene);
    if let (Some(y2_spec), Some(y2_axis)) = (&graph.y2_axis, &axes.y2_axis) {
        emit_threshold_rules(&y2_spec.thresholds, y2_axis, &axes.x_axis, doc, primary_id, scene);
    }
}

fn emit_axis_gridlines(
    axes: &CompiledAxes,
    graph: &GraphSpec,
    policy: &LocaleNumberPolicy,
    scene: &mut ChartScene,
    grid_color: Rgba8,
    drawn_x: &mut Vec<f64>,
    drawn_y: &mut Vec<f64>,
) {
    let stroke = gridline_stroke(grid_color);
    let auto_prefix = |ticks: &[AxisTick], ax: &ResolvedAxis, spec: &AxisSpec| -> usize {
        if spec.additional_ticks.is_empty() {
            return ticks.len();
        }
        let add = compute_additional_ticks(ax, &spec.additional_ticks, spec.tick_format, policy);
        let prefix = ticks.len().saturating_sub(add.len());
        let suffix_matches = ticks.len() >= add.len() && ticks[prefix..].iter().zip(&add).all(|(t, a)| (t.coord_pt - a.coord_pt).abs() <= 1e-6);
        if suffix_matches { prefix } else { ticks.len() }
    };
    let x_auto_len = auto_prefix(&axes.x_ticks, &axes.x_axis, &graph.x_axis);
    for tick in &axes.x_ticks[..x_auto_len] {
        if tick.kind != TickKind::Major {
            continue;
        }
        drawn_x.push(tick.coord_pt);
        for y_seg in &axes.y_axis.segments {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(tick.coord_pt, y_seg.span_start_pt), PointPt::new(tick.coord_pt, y_seg.span_end_pt)),
                fill: None,
                stroke: Some(stroke),
                clip: None,
            });
        }
    }
    let y_auto_len = auto_prefix(&axes.y_ticks, &axes.y_axis, &graph.y_axis);
    for tick in &axes.y_ticks[..y_auto_len] {
        if tick.kind != TickKind::Major {
            continue;
        }
        drawn_y.push(tick.coord_pt);
        for x_seg in &axes.x_axis.segments {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(x_seg.span_start_pt, tick.coord_pt), PointPt::new(x_seg.span_end_pt, tick.coord_pt)),
                fill: None,
                stroke: Some(stroke),
                clip: None,
            });
        }
    }
}

fn emit_additional_tick_gridlines_dedup(
    axes: &CompiledAxes,
    graph: &GraphSpec,
    scene: &mut ChartScene,
    options: &AxisRenderOptions,
    skip_x: &[f64],
    skip_y: &[f64],
) {
    let y_spans: Vec<(f64, f64)> = axes.y_axis.segments.iter().map(|s| (s.span_start_pt, s.span_end_pt)).collect();
    let x_spans: Vec<(f64, f64)> = axes.x_axis.segments.iter().map(|s| (s.span_start_pt, s.span_end_pt)).collect();

    if !graph.x_axis.additional_ticks.is_empty() {
        let mut x_add = compute_additional_ticks(&axes.x_axis, &graph.x_axis.additional_ticks, graph.x_axis.tick_format, &options.policy);
        x_add.retain(|t| t.draw_gridline && !skip_x.iter().any(|&c| (c - t.coord_pt).abs() <= 1e-6));
        emit_additional_tick_gridline_segments(&x_add, true, &y_spans, options.grid_color, &mut scene.primitives);
    }
    if !graph.y_axis.additional_ticks.is_empty() {
        let mut y_add = compute_additional_ticks(&axes.y_axis, &graph.y_axis.additional_ticks, graph.y_axis.tick_format, &options.policy);
        y_add.retain(|t| t.draw_gridline && !skip_y.iter().any(|&c| (c - t.coord_pt).abs() <= 1e-6));
        emit_additional_tick_gridline_segments(&y_add, false, &x_spans, options.grid_color, &mut scene.primitives);
    }
    if let (Some(y2_spec), Some(y2_ax)) = (&graph.y2_axis, &axes.y2_axis)
        && !y2_spec.additional_ticks.is_empty()
    {
        let y2_add = compute_additional_ticks(y2_ax, &y2_spec.additional_ticks, y2_spec.tick_format, &options.policy);
        emit_additional_tick_gridline_segments(&y2_add, false, &x_spans, options.grid_color, &mut scene.primitives);
    }
}

fn emit_threshold_zones(thresholds: &[AxisThreshold], primary: &ResolvedAxis, perp: &ResolvedAxis, scene: &mut ChartScene) {
    for thr in thresholds {
        let Some(shade_to) = thr.shade_to else { continue };
        if !thr.value.is_finite() || !shade_to.is_finite() {
            continue;
        }
        let fill_color = if thr.color.a == u8::MAX { thr.color.with_alpha(48) } else { thr.color };
        let fill = FillPaint::solid(fill_color);
        for p_span in primary.map_interval(thr.value, shade_to) {
            for q_seg in &perp.segments {
                let rect = match primary.orientation {
                    AxisOrientation::Horizontal => RectPt::new(
                        p_span.start_pt.min(p_span.end_pt),
                        q_seg.span_start_pt.min(q_seg.span_end_pt),
                        (p_span.end_pt - p_span.start_pt).abs(),
                        (q_seg.span_end_pt - q_seg.span_start_pt).abs(),
                    ),
                    AxisOrientation::Vertical => RectPt::new(
                        q_seg.span_start_pt.min(q_seg.span_end_pt),
                        p_span.start_pt.min(p_span.end_pt),
                        (q_seg.span_end_pt - q_seg.span_start_pt).abs(),
                        (p_span.end_pt - p_span.start_pt).abs(),
                    ),
                };
                if rect.width > 0.0 && rect.height > 0.0 {
                    scene.primitives.push(ScenePrimitive::Path { path: Path2D::rect(rect), fill: Some(fill), stroke: None, clip: None });
                }
            }
        }
    }
}

fn emit_threshold_rules(
    thresholds: &[AxisThreshold],
    primary: &ResolvedAxis,
    perp: &ResolvedAxis,
    doc: &ProjectDocument,
    primary_id: Option<&SheetId>,
    scene: &mut ChartScene,
) {
    for thr in thresholds {
        let Some(matched) = primary.map_value(thr.value) else { continue };
        let width_pt = if thr.stroke_width_pt.is_finite() && thr.stroke_width_pt > 0.0 { thr.stroke_width_pt } else { 1.0 };
        let stroke = StrokePaint { color: thr.color, width_pt, dash: thr.dash_style, cap: LineCap::Butt, join: LineJoin::Miter };
        for q_seg in &perp.segments {
            let (p0, p1) = match primary.orientation {
                AxisOrientation::Horizontal => {
                    (PointPt::new(matched.coord_pt, q_seg.span_start_pt), PointPt::new(matched.coord_pt, q_seg.span_end_pt))
                }
                AxisOrientation::Vertical => (PointPt::new(q_seg.span_start_pt, matched.coord_pt), PointPt::new(q_seg.span_end_pt, matched.coord_pt)),
            };
            scene.primitives.push(ScenePrimitive::Path { path: Path2D::line(p0, p1), fill: None, stroke: Some(stroke), clip: None });
        }

        if let Some(raw_label) = &thr.label
            && !raw_label.trim().is_empty()
        {
            let expanded = resolve_template_hooks(raw_label, doc, primary_id);
            let rich = parse_markup(&expanded);
            let (anchor_pt, anchor) = match primary.orientation {
                AxisOrientation::Horizontal => (PointPt::new(matched.coord_pt + 3.0, scene.plot_rect_pt.y + 10.0), TextAnchor::Start),
                AxisOrientation::Vertical => (PointPt::new(scene.plot_rect_pt.right() - 4.0, matched.coord_pt - 3.0), TextAnchor::End),
            };
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich,
                anchor_pt,
                anchor,
                font_size_pt: 8.0,
                rotation_deg: 0.0,
                color: thr.color,
            }));
        }
    }
}

/// Resolve axes and compute major/minor ticks for `graph` without emitting drawing primitives yet.
pub fn resolve_axes_and_ticks(graph: &GraphSpec, scene: &mut ChartScene, options: &AxisRenderOptions) -> CompiledAxes {
    let mode = options.frame_mode.unwrap_or_else(|| AxisFrameMode::from(graph.frame_preset));
    let r = scene.plot_rect_pt;

    let x_axis = ResolvedAxis::resolve(&graph.x_axis, AxisOrientation::Horizontal, r, options.x_data_bounds, &mut scene.warnings);
    let y_axis = ResolvedAxis::resolve(&graph.y_axis, AxisOrientation::Vertical, r, options.y_data_bounds, &mut scene.warnings);
    let y2_spec_fallback = AxisSpec::linear("");
    let (y2_spec, y2_axis) = if mode == AxisFrameMode::DualYSecondaryRight || graph.y2_axis.is_some() {
        let spec_ref = graph.y2_axis.as_ref().unwrap_or(&y2_spec_fallback);
        (Some(spec_ref), Some(ResolvedAxis::resolve(spec_ref, AxisOrientation::Vertical, r, options.y2_data_bounds, &mut scene.warnings)))
    } else {
        (None, None)
    };

    let append_additional = |ticks: &mut Vec<AxisTick>, ax: &ResolvedAxis, spec: &AxisSpec| {
        if spec.additional_ticks.is_empty() {
            return;
        }
        for add in compute_additional_ticks(ax, &spec.additional_ticks, spec.tick_format, &options.policy) {
            ticks.push(AxisTick {
                segment_index: add.segment_index,
                value: add.value,
                coord_pt: add.coord_pt,
                kind: TickKind::Major,
                label: add.label,
            });
        }
    };

    let mut x_ticks = compute_axis_ticks_with_warnings(
        &x_axis,
        TickComputeConfig {
            tick_format: graph.x_axis.tick_format,
            policy: &options.policy,
            auto_log_minor_ticks: options.auto_log_minor_ticks,
            categorical_ticks: options.x_categories.as_deref(),
        },
        &mut scene.warnings,
    );
    append_additional(&mut x_ticks, &x_axis, &graph.x_axis);

    let mut y_ticks = compute_axis_ticks_with_warnings(
        &y_axis,
        TickComputeConfig {
            tick_format: graph.y_axis.tick_format,
            policy: &options.policy,
            auto_log_minor_ticks: options.auto_log_minor_ticks,
            categorical_ticks: options.y_categories.as_deref(),
        },
        &mut scene.warnings,
    );
    append_additional(&mut y_ticks, &y_axis, &graph.y_axis);

    let y2_ticks = match (y2_spec, &y2_axis) {
        (Some(spec), Some(ax)) => {
            let mut ts = compute_axis_ticks_with_warnings(
                ax,
                TickComputeConfig {
                    tick_format: spec.tick_format,
                    policy: &options.policy,
                    auto_log_minor_ticks: options.auto_log_minor_ticks,
                    categorical_ticks: options.y2_categories.as_deref(),
                },
                &mut scene.warnings,
            );
            append_additional(&mut ts, ax, spec);
            ts
        }
        _ => Vec::new(),
    };

    CompiledAxes { x_axis, y_axis, y2_axis, x_ticks, y_ticks, y2_ticks, frame_mode: mode }
}

/// Emit spines, break ornaments, tick marks, tick labels, and axis titles for pre-resolved `axes`.
pub fn emit_axes_and_frame(axes: &CompiledAxes, graph: &GraphSpec, doc: &ProjectDocument, scene: &mut ChartScene, options: &AxisRenderOptions) {
    if axes.frame_mode != AxisFrameMode::NoAxes {
        emit_frame_geometry(axes, graph, doc, scene, options);
    }
}

/// Resolve axes, compute nice ticks, and emit spines, break ornaments, tick marks, tick labels,
/// and axis titles according to `graph.frame_preset` (or `options.frame_mode`).
pub fn render_axes_and_frame(graph: &GraphSpec, doc: &ProjectDocument, scene: &mut ChartScene, options: &AxisRenderOptions) -> CompiledAxes {
    let compiled = resolve_axes_and_ticks(graph, scene, options);
    if compiled.frame_mode != AxisFrameMode::NoAxes {
        emit_additional_tick_gridlines_dedup(&compiled, graph, scene, options, &[], &[]);
    }
    emit_axes_and_frame(&compiled, graph, doc, scene, options);
    compiled
}

fn ticks_with_expanded_additional<'a>(
    ticks: &'a [AxisTick],
    ax: &ResolvedAxis,
    spec: &AxisSpec,
    doc: &ProjectDocument,
    primary_id: Option<&SheetId>,
    options: &AxisRenderOptions,
) -> std::borrow::Cow<'a, [AxisTick]> {
    if !spec.additional_ticks.iter().any(|t| t.label.as_ref().is_some_and(|s| s.contains("{{"))) {
        return std::borrow::Cow::Borrowed(ticks);
    }
    let segs: Vec<(usize, AxisMapping, f64, f64)> =
        ax.segments.iter().map(|s| (s.segment_index, s.mapping(), s.span_start_pt, s.span_end_pt)).collect();
    let expanded = compute_additional_ticks_for_segments(&segs, &spec.additional_ticks, spec.tick_format, &options.policy, Some(doc), primary_id);
    let prefix = ticks.len().saturating_sub(expanded.len());
    if ticks.len() < expanded.len() || !ticks[prefix..].iter().zip(&expanded).all(|(t, a)| (t.coord_pt - a.coord_pt).abs() <= 1e-6) {
        return std::borrow::Cow::Borrowed(ticks);
    }
    let mut cloned = ticks.to_vec();
    for (slot, add) in cloned[prefix..].iter_mut().zip(expanded) {
        slot.label = add.label;
    }
    std::borrow::Cow::Owned(cloned)
}

fn emit_frame_geometry(axes: &CompiledAxes, graph: &GraphSpec, doc: &ProjectDocument, scene: &mut ChartScene, options: &AxisRenderOptions) {
    let r = scene.plot_rect_pt;
    let x_stroke = axis_stroke(&graph.x_axis, options.axis_color);
    let y_stroke = axis_stroke(&graph.y_axis, options.axis_color);
    let offset = if axes.frame_mode == AxisFrameMode::OffsetAxes { options.offset_gap_pt } else { 0.0 };

    let (x_spine_y, y_spine_x) = match axes.frame_mode {
        AxisFrameMode::OriginCrosshair => resolve_origin_crosshair_coords(axes, r, &mut scene.warnings),
        AxisFrameMode::OffsetAxes => (r.bottom() + offset, r.x - offset),
        _ => (r.bottom(), r.x),
    };

    let primary_id = Some(&graph.primary_source_sheet_id);
    let x_ticks = ticks_with_expanded_additional(&axes.x_ticks, &axes.x_axis, &graph.x_axis, doc, primary_id, options);
    let y_ticks = ticks_with_expanded_additional(&axes.y_ticks, &axes.y_axis, &graph.y_axis, doc, primary_id, options);

    let trim_tufte = axes.frame_mode == AxisFrameMode::OffsetAxes;
    emit_horizontal_spine(&axes.x_axis, &x_ticks, x_spine_y, trim_tufte, x_stroke, scene);
    emit_vertical_spine(&axes.y_axis, &y_ticks, y_spine_x, trim_tufte, y_stroke, scene);

    if matches!(axes.frame_mode, AxisFrameMode::PlainBox | AxisFrameMode::TopRightTicks) {
        emit_horizontal_spine(&axes.x_axis, &x_ticks, r.y, false, x_stroke, scene);
        emit_vertical_spine(&axes.y_axis, &y_ticks, r.right(), false, y_stroke, scene);
    }
    if axes.frame_mode == AxisFrameMode::TopRightTicks {
        emit_ticks_only_horizontal(&x_ticks, r.y, true, x_stroke, options, scene);
        emit_ticks_only_vertical(&y_ticks, r.right(), false, y_stroke, options, scene);
    }

    emit_x_ticks_and_labels(&x_ticks, x_spine_y, &graph.x_axis, x_stroke, options, scene);
    emit_y_ticks_and_labels(&y_ticks, y_spine_x, false, &graph.y_axis, y_stroke, options, scene);

    emit_axis_titles(graph, doc, primary_id, x_spine_y, y_spine_x, options, scene);

    if let Some(y2_ax) = &axes.y2_axis {
        let y2_spec = graph.y2_axis.as_ref().unwrap_or(&graph.y_axis);
        let y2_stroke = axis_stroke(y2_spec, options.axis_color);
        let y2_ticks = ticks_with_expanded_additional(&axes.y2_ticks, y2_ax, y2_spec, doc, primary_id, options);
        emit_vertical_spine(y2_ax, &y2_ticks, r.right(), false, y2_stroke, scene);
        emit_y_ticks_and_labels(&y2_ticks, r.right(), true, y2_spec, y2_stroke, options, scene);
        emit_y2_title(y2_spec, doc, primary_id, r, options, scene);
    }
}

fn axis_stroke(spec: &AxisSpec, color: Rgba8) -> StrokePaint {
    let w = if spec.stroke_width_pt.is_finite() && spec.stroke_width_pt > 0.0 { spec.stroke_width_pt } else { 1.0 };
    StrokePaint::solid(color, w)
}

fn resolve_origin_crosshair_coords(axes: &CompiledAxes, r: RectPt, warnings: &mut Vec<ChartWarning>) -> (f64, f64) {
    let x_spine_y = match axes.y_axis.map_value(0.0) {
        Some(m) => m.coord_pt,
        None => {
            warnings.push(ChartWarning::new("origin_outside_y_domain", "Y=0 lies outside domain; crosshair X spine placed at plot bottom"));
            r.bottom()
        }
    };
    let y_spine_x = match axes.x_axis.map_value(0.0) {
        Some(m) => m.coord_pt,
        None => {
            warnings.push(ChartWarning::new("origin_outside_x_domain", "X=0 lies outside domain; crosshair Y spine placed at plot left"));
            r.x
        }
    };
    (x_spine_y, y_spine_x)
}

fn emit_horizontal_spine(axis: &ResolvedAxis, ticks: &[AxisTick], spine_y: f64, trim_tufte: bool, stroke: StrokePaint, scene: &mut ChartScene) {
    for seg in &axis.segments {
        let (x0, x1) = if trim_tufte && axis.segments.len() == 1 {
            tufte_trimmed_span(ticks, seg.segment_index, seg.span_start_pt, seg.span_end_pt)
        } else {
            (seg.span_start_pt, seg.span_end_pt)
        };
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x0, spine_y), PointPt::new(x1, spine_y)),
            fill: None,
            stroke: Some(stroke),
            clip: None,
        });
    }
    for br in &axis.breaks {
        emit_horizontal_break_ornament(*br, spine_y, stroke, scene);
    }
}

fn emit_vertical_spine(axis: &ResolvedAxis, ticks: &[AxisTick], spine_x: f64, trim_tufte: bool, stroke: StrokePaint, scene: &mut ChartScene) {
    for seg in &axis.segments {
        let (y0, y1) = if trim_tufte && axis.segments.len() == 1 {
            tufte_trimmed_span(ticks, seg.segment_index, seg.span_start_pt, seg.span_end_pt)
        } else {
            (seg.span_start_pt, seg.span_end_pt)
        };
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(spine_x, y0), PointPt::new(spine_x, y1)),
            fill: None,
            stroke: Some(stroke),
            clip: None,
        });
    }
    for br in &axis.breaks {
        emit_vertical_break_ornament(*br, spine_x, stroke, scene);
    }
}

fn tufte_trimmed_span(ticks: &[AxisTick], seg_idx: usize, fallback_start: f64, fallback_end: f64) -> (f64, f64) {
    let mut coords: Vec<f64> = ticks.iter().filter(|t| t.segment_index == seg_idx && t.kind == TickKind::Major).map(|t| t.coord_pt).collect();
    if coords.len() < 2 {
        return (fallback_start, fallback_end);
    }
    coords.sort_by(f64::total_cmp);
    match (coords.first().copied(), coords.last().copied()) {
        (Some(a), Some(b)) if (b - a).abs() > 1.0 => (a, b),
        _ => (fallback_start, fallback_end),
    }
}

fn emit_horizontal_break_ornament(br: AxisBreakGap, spine_y: f64, stroke: StrokePaint, scene: &mut ChartScene) {
    let dx = 2.0_f64;
    let dy = 4.0_f64;
    match br.ornament {
        BreakOrnament::BlankGap => {}
        BreakOrnament::DoubleSlash => {
            for gx in [br.gap_start_pt, br.gap_end_pt] {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(gx - dx, spine_y + dy), PointPt::new(gx + dx, spine_y - dy)),
                    fill: None,
                    stroke: Some(stroke),
                    clip: None,
                });
            }
        }
        BreakOrnament::Zigzag => {
            let mid = 0.5 * (br.gap_start_pt + br.gap_end_pt);
            let q1 = 0.5 * (br.gap_start_pt + mid);
            let q3 = 0.5 * (mid + br.gap_end_pt);
            let path = Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(br.gap_start_pt, spine_y)),
                    PathCmd::LineTo(PointPt::new(q1, spine_y - dy)),
                    PathCmd::LineTo(PointPt::new(q3, spine_y + dy)),
                    PathCmd::LineTo(PointPt::new(br.gap_end_pt, spine_y)),
                ],
            };
            scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: None });
        }
    }
}

fn emit_vertical_break_ornament(br: AxisBreakGap, spine_x: f64, stroke: StrokePaint, scene: &mut ChartScene) {
    let dx = 4.0_f64;
    let dy = 2.0_f64;
    match br.ornament {
        BreakOrnament::BlankGap => {}
        BreakOrnament::DoubleSlash => {
            for gy in [br.gap_start_pt, br.gap_end_pt] {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(spine_x - dx, gy + dy), PointPt::new(spine_x + dx, gy - dy)),
                    fill: None,
                    stroke: Some(stroke),
                    clip: None,
                });
            }
        }
        BreakOrnament::Zigzag => {
            let mid = 0.5 * (br.gap_start_pt + br.gap_end_pt);
            let q1 = 0.5 * (br.gap_start_pt + mid);
            let q3 = 0.5 * (mid + br.gap_end_pt);
            let path = Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(spine_x, br.gap_start_pt)),
                    PathCmd::LineTo(PointPt::new(spine_x - dx, q1)),
                    PathCmd::LineTo(PointPt::new(spine_x + dx, q3)),
                    PathCmd::LineTo(PointPt::new(spine_x, br.gap_end_pt)),
                ],
            };
            scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: None });
        }
    }
}

fn emit_ticks_only_horizontal(
    ticks: &[AxisTick],
    spine_y: f64,
    inward_down: bool,
    stroke: StrokePaint,
    options: &AxisRenderOptions,
    scene: &mut ChartScene,
) {
    let dir = if inward_down { 1.0 } else { -1.0 };
    for tick in ticks {
        let len = match tick.kind {
            TickKind::Major => options.major_tick_len_pt,
            TickKind::Minor => options.minor_tick_len_pt,
        };
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(tick.coord_pt, spine_y), PointPt::new(tick.coord_pt, spine_y + dir * len)),
            fill: None,
            stroke: Some(stroke),
            clip: None,
        });
    }
}

fn emit_ticks_only_vertical(
    ticks: &[AxisTick],
    spine_x: f64,
    outward_left: bool,
    stroke: StrokePaint,
    options: &AxisRenderOptions,
    scene: &mut ChartScene,
) {
    let dir = if outward_left { -1.0 } else { 1.0 };
    for tick in ticks {
        let len = match tick.kind {
            TickKind::Major => options.major_tick_len_pt,
            TickKind::Minor => options.minor_tick_len_pt,
        };
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(spine_x, tick.coord_pt), PointPt::new(spine_x + dir * len, tick.coord_pt)),
            fill: None,
            stroke: Some(stroke),
            clip: None,
        });
    }
}

fn should_emit_tick_label(spec: &AxisSpec, value: f64) -> bool {
    spec.show_tick_labels
        || spec.additional_ticks.iter().any(|t| (t.value - value).abs() <= 1e-12 && t.label.as_ref().is_some_and(|s| !s.trim().is_empty()))
}

fn emit_x_ticks_and_labels(
    ticks: &[AxisTick],
    spine_y: f64,
    spec: &AxisSpec,
    stroke: StrokePaint,
    options: &AxisRenderOptions,
    scene: &mut ChartScene,
) {
    let rot = if spec.tick_angle_deg.is_finite() { spec.tick_angle_deg } else { 0.0 };
    let anchor = if rot > 5.0 {
        TextAnchor::Start
    } else if rot < -5.0 {
        TextAnchor::End
    } else {
        TextAnchor::Middle
    };

    for tick in ticks {
        let len = match tick.kind {
            TickKind::Major => options.major_tick_len_pt,
            TickKind::Minor => options.minor_tick_len_pt,
        };
        if let Some((o0, o1)) = tick_span_offsets(spec.tick_direction, len, 1.0) {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(tick.coord_pt, spine_y + o0), PointPt::new(tick.coord_pt, spine_y + o1)),
                fill: None,
                stroke: Some(stroke),
                clip: None,
            });
        }

        if let Some(rich) = &tick.label
            && should_emit_tick_label(spec, tick.value)
        {
            let y_offset = len + options.tick_font_size_pt * 1.05;
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: rich.clone(),
                anchor_pt: PointPt::new(tick.coord_pt, spine_y + y_offset),
                anchor,
                font_size_pt: options.tick_font_size_pt,
                rotation_deg: rot,
                color: options.axis_color,
            }));
        }
    }
}

fn emit_y_ticks_and_labels(
    ticks: &[AxisTick],
    spine_x: f64,
    is_right_axis: bool,
    spec: &AxisSpec,
    stroke: StrokePaint,
    options: &AxisRenderOptions,
    scene: &mut ChartScene,
) {
    let dir = if is_right_axis { 1.0 } else { -1.0 };
    let anchor = if is_right_axis { TextAnchor::Start } else { TextAnchor::End };

    for tick in ticks {
        let len = match tick.kind {
            TickKind::Major => options.major_tick_len_pt,
            TickKind::Minor => options.minor_tick_len_pt,
        };
        if let Some((o0, o1)) = tick_span_offsets(spec.tick_direction, len, dir) {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(spine_x + o0, tick.coord_pt), PointPt::new(spine_x + o1, tick.coord_pt)),
                fill: None,
                stroke: Some(stroke),
                clip: None,
            });
        }

        if let Some(rich) = &tick.label
            && should_emit_tick_label(spec, tick.value)
        {
            let x_pos = spine_x + dir * (len + 3.0);
            let y_pos = tick.coord_pt + options.tick_font_size_pt * 0.32;
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: rich.clone(),
                anchor_pt: PointPt::new(x_pos, y_pos),
                anchor,
                font_size_pt: options.tick_font_size_pt,
                rotation_deg: 0.0,
                color: options.axis_color,
            }));
        }
    }
}

fn emit_axis_titles(
    graph: &GraphSpec,
    doc: &ProjectDocument,
    primary_id: Option<&SheetId>,
    x_spine_y: f64,
    y_spine_x: f64,
    options: &AxisRenderOptions,
    scene: &mut ChartScene,
) {
    let r = scene.plot_rect_pt;
    if !graph.x_axis.title.trim().is_empty() {
        let expanded = resolve_template_hooks(&graph.x_axis.title, doc, primary_id);
        let y_pos = (x_spine_y + options.major_tick_len_pt + options.tick_font_size_pt * 2.2 + 4.0).min(scene.height_pt - 4.0);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&expanded),
            anchor_pt: PointPt::new(r.center().x, y_pos),
            anchor: TextAnchor::Middle,
            font_size_pt: options.title_font_size_pt,
            rotation_deg: 0.0,
            color: options.axis_color,
        }));
    }
    if !graph.y_axis.title.trim().is_empty() {
        let expanded = resolve_template_hooks(&graph.y_axis.title, doc, primary_id);
        let x_pos = (y_spine_x - 30.0).max(options.title_font_size_pt + 2.0);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&expanded),
            anchor_pt: PointPt::new(x_pos, r.center().y),
            anchor: TextAnchor::Middle,
            font_size_pt: options.title_font_size_pt,
            rotation_deg: -90.0,
            color: options.axis_color,
        }));
    }
}

fn emit_y2_title(
    y2_spec: &AxisSpec,
    doc: &ProjectDocument,
    primary_id: Option<&SheetId>,
    r: RectPt,
    options: &AxisRenderOptions,
    scene: &mut ChartScene,
) {
    if y2_spec.title.trim().is_empty() {
        return;
    }
    let expanded = resolve_template_hooks(&y2_spec.title, doc, primary_id);
    let x_pos = (r.right() + 30.0).min(scene.width_pt - options.title_font_size_pt - 2.0);
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup(&expanded),
        anchor_pt: PointPt::new(x_pos, r.center().y),
        anchor: TextAnchor::Middle,
        font_size_pt: options.title_font_size_pt,
        rotation_deg: 90.0,
        color: options.axis_color,
    }));
}

// ============================================================================
// 5. Legends & Canvas GraphAnnotations
// ============================================================================

/// Visual swatch + label entry rendered inside a figure legend.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendEntry {
    pub label: String,
    pub swatch_color: Rgba8,
    pub stroke_color: Option<Rgba8>,
    pub marker_shape: Option<MarkerShape>,
    pub marker_fill_mode: MarkerFillMode,
    pub dash_style: Option<LineDashStyle>,
    pub is_box_swatch: bool,
}

/// Extract default [`LegendEntry`] rows from [`GraphSpec`] series bindings and dataset titles.
#[must_use]
pub fn legend_entries_from_graph_spec(graph: &GraphSpec, doc: &ProjectDocument) -> Vec<LegendEntry> {
    let mut entries = Vec::with_capacity(graph.series.len());
    for (idx, series) in graph.series.iter().enumerate() {
        let label = series
            .legend_label
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                doc.data_sheet(&series.source_sheet_id)
                    .or_else(|| doc.result_sheet(&series.source_sheet_id).and_then(|res| res.analyzable_data.as_ref()))
                    .and_then(|ds| data_set_title(&ds.archetype, series.data_set_index))
                    .map(str::to_owned)
                    .filter(|s| !s.trim().is_empty())
            })
            .unwrap_or_else(|| format!("Series {}", idx + 1));

        let entry = entry_from_gdo(label, &series.gdo, graph.palette.color_at(idx));
        entries.push(entry);
    }
    entries
}

fn data_set_title(archetype: &TableArchetype, idx: usize) -> Option<&str> {
    match archetype {
        TableArchetype::Xy(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        TableArchetype::Column(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        TableArchetype::Grouped(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        TableArchetype::Contingency(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        TableArchetype::Survival(t) => t.cohorts.get(idx).map(|c| c.title.as_str()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        TableArchetype::MultipleVariables(t) => t.variables.get(idx).map(|v| v.title.as_str()),
        TableArchetype::Nested(t) => t.groups.get(idx).map(|g| g.title.as_str()),
    }
}

fn entry_from_gdo(label: String, gdo: &curvebook_doc::GraphicalDataObject, fallback: Rgba8) -> LegendEntry {
    use curvebook_doc::GraphicalDataObject as Gdo;
    match gdo {
        Gdo::ScatterAndLine { marker, line, .. } => LegendEntry {
            label,
            swatch_color: marker.map_or_else(|| line.map_or(fallback, |l| l.color), |m| m.fill_color),
            stroke_color: marker.map(|m| m.stroke_color).or_else(|| line.map(|l| l.color)),
            marker_shape: marker.map(|m| m.shape),
            marker_fill_mode: marker.map_or(MarkerFillMode::Filled, |m| m.fill_mode),
            dash_style: line.map(|l| l.dash_style),
            is_box_swatch: false,
        },
        Gdo::BarWithReplicates { fill_color, border_color, .. } | Gdo::FloatingBar { fill_color, border_color, .. } => LegendEntry {
            label,
            swatch_color: *fill_color,
            stroke_color: Some(*border_color),
            marker_shape: None,
            marker_fill_mode: MarkerFillMode::Filled,
            dash_style: None,
            is_box_swatch: true,
        },
        Gdo::BoxAndWhisker { fill_color, stroke_color, .. } | Gdo::Violin { fill_color, stroke_color, .. } => LegendEntry {
            label,
            swatch_color: *fill_color,
            stroke_color: Some(*stroke_color),
            marker_shape: None,
            marker_fill_mode: MarkerFillMode::Filled,
            dash_style: None,
            is_box_swatch: true,
        },
        Gdo::AlignedDotPlot { marker, .. } => LegendEntry {
            label,
            swatch_color: marker.fill_color,
            stroke_color: Some(marker.stroke_color),
            marker_shape: Some(marker.shape),
            marker_fill_mode: marker.fill_mode,
            dash_style: None,
            is_box_swatch: false,
        },
        Gdo::BeforeAfterLines { marker, line_color, .. } => LegendEntry {
            label,
            swatch_color: *line_color,
            stroke_color: Some(*line_color),
            marker_shape: marker.map(|m| m.shape),
            marker_fill_mode: marker.map_or(MarkerFillMode::Filled, |m| m.fill_mode),
            dash_style: Some(LineDashStyle::Solid),
            is_box_swatch: false,
        },
        Gdo::KaplanMeierStep { line_color, .. } => LegendEntry {
            label,
            swatch_color: *line_color,
            stroke_color: Some(*line_color),
            marker_shape: None,
            marker_fill_mode: MarkerFillMode::Filled,
            dash_style: Some(LineDashStyle::Solid),
            is_box_swatch: false,
        },
        Gdo::PartsOfWhole { .. } => LegendEntry {
            label,
            swatch_color: fallback,
            stroke_color: Some(Rgba8::BLACK),
            marker_shape: None,
            marker_fill_mode: MarkerFillMode::Filled,
            dash_style: None,
            is_box_swatch: true,
        },
    }
}

/// Render a figure legend from `legend` and `entries` into `scene`.
pub fn render_legend(legend: &LegendSpec, entries: &[LegendEntry], doc: &ProjectDocument, primary_id: Option<&SheetId>, scene: &mut ChartScene) {
    if !legend.visible || entries.is_empty() {
        return;
    }
    let font_size = 8.5_f64;
    let row_h = 13.0_f64;
    let swatch_w = 18.0_f64;
    let pad = 6.0_f64;

    let mut parsed_labels = Vec::with_capacity(entries.len());
    let mut max_text_w = 12.0_f64;
    for entry in entries {
        let expanded = resolve_template_hooks(&entry.label, doc, primary_id);
        let rich = parse_markup(&expanded);
        max_text_w = max_text_w.max(measure_text_pt(&rich, font_size).width_pt);
        parsed_labels.push(rich);
    }

    let box_w = pad * 2.0 + swatch_w + 6.0 + max_text_w;
    let box_h = pad * 2.0 + (entries.len() as f64) * row_h;
    let (bx, by) = match legend.position_pt {
        Some((px, py)) if px.is_finite() && py.is_finite() => (px, py),
        _ => ((scene.plot_rect_pt.right() - box_w - 6.0).max(scene.plot_rect_pt.x + 4.0), scene.plot_rect_pt.y + 6.0),
    };
    let box_rect = RectPt::new(bx, by, box_w, box_h);

    if legend.bordered {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(box_rect),
            fill: Some(FillPaint::solid(Rgba8::rgba(255, 255, 255, 235))),
            stroke: Some(StrokePaint::solid(Rgba8::BLACK, 0.8)),
            clip: None,
        });
    }

    for (idx, (entry, rich)) in entries.iter().zip(parsed_labels).enumerate() {
        let row_cy = by + pad + ((idx as f64) + 0.5) * row_h;
        let sx0 = bx + pad;
        emit_legend_swatch(entry, sx0, row_cy, swatch_w, scene);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich,
            anchor_pt: PointPt::new(sx0 + swatch_w + 6.0, row_cy + font_size * 0.34),
            anchor: TextAnchor::Start,
            font_size_pt: font_size,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
}

fn emit_legend_swatch(entry: &LegendEntry, x0: f64, cy: f64, width: f64, scene: &mut ChartScene) {
    if entry.is_box_swatch {
        let swatch_rect = RectPt::new(x0 + 2.0, cy - 4.0, (width - 4.0).max(6.0), 8.0);
        let stroke = entry.stroke_color.map(|c| StrokePaint::solid(c, 0.8));
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(swatch_rect),
            fill: Some(FillPaint::solid(entry.swatch_color)),
            stroke,
            clip: None,
        });
        return;
    }

    if let Some(dash) = entry.dash_style {
        let col = entry.stroke_color.unwrap_or(entry.swatch_color);
        let stroke = StrokePaint { color: col, width_pt: 1.4, dash, cap: LineCap::Butt, join: LineJoin::Miter };
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x0, cy), PointPt::new(x0 + width, cy)),
            fill: None,
            stroke: Some(stroke),
            clip: None,
        });
    }

    let cx = x0 + 0.5 * width;
    let shape = entry.marker_shape.unwrap_or(MarkerShape::Circle);
    let path = swatch_marker_path(PointPt::new(cx, cy), 3.0, shape);
    let fill = match entry.marker_fill_mode {
        MarkerFillMode::Open => None,
        _ => Some(FillPaint::solid(entry.swatch_color)),
    };
    let stroke = entry.stroke_color.map(|c| StrokePaint::solid(c, 0.9));
    if entry.marker_shape.is_some() || entry.dash_style.is_none() {
        scene.primitives.push(ScenePrimitive::Path { path, fill, stroke, clip: None });
    }
}

fn swatch_marker_path(c: PointPt, r: f64, shape: MarkerShape) -> Path2D {
    match shape {
        MarkerShape::Square => Path2D::rect(RectPt::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r)),
        MarkerShape::Diamond => Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(c.x, c.y - r)),
                PathCmd::LineTo(PointPt::new(c.x + r, c.y)),
                PathCmd::LineTo(PointPt::new(c.x, c.y + r)),
                PathCmd::LineTo(PointPt::new(c.x - r, c.y)),
                PathCmd::Close,
            ],
        },
        MarkerShape::TriangleUp => Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(c.x, c.y - r)),
                PathCmd::LineTo(PointPt::new(c.x + r, c.y + r)),
                PathCmd::LineTo(PointPt::new(c.x - r, c.y + r)),
                PathCmd::Close,
            ],
        },
        MarkerShape::TriangleDown => Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(c.x - r, c.y - r)),
                PathCmd::LineTo(PointPt::new(c.x + r, c.y - r)),
                PathCmd::LineTo(PointPt::new(c.x, c.y + r)),
                PathCmd::Close,
            ],
        },
        _ => ellipse_path(RectPt::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r)),
    }
}

/// Render all [`GraphAnnotation`] items (`TextBox`, `LineOrArrow`, `Shape`, `EmbeddedResultTable`,
/// `ModelEquation`) onto `scene`.
pub fn render_annotations(annotations: &[GraphAnnotation], doc: &ProjectDocument, primary_id: Option<&SheetId>, scene: &mut ChartScene) {
    for ann in annotations {
        match ann {
            GraphAnnotation::TextBox { bounds_pt, markdown, bordered } => {
                render_textbox_annotation(RectPt::from(*bounds_pt), markdown, *bordered, doc, primary_id, scene);
            }
            GraphAnnotation::LineOrArrow { start_pt, end_pt, arrowhead, dash, color } => {
                render_line_or_arrow_annotation(*start_pt, *end_pt, *arrowhead, *dash, *color, scene);
            }
            GraphAnnotation::Shape { kind, bounds_pt, stroke_color, stroke_width_pt, dash, fill_color } => {
                render_shape_annotation(*kind, RectPt::from(*bounds_pt), *stroke_color, *stroke_width_pt, *dash, *fill_color, scene);
            }
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, tab_id, bounds_pt, selected_rows, selected_cols, style } => {
                let _ = crate::xy::render_styled_embedded_table(
                    scene,
                    doc,
                    &crate::xy::EmbeddedTableDrawSpec {
                        sheet_id: result_sheet_id,
                        tab_id: tab_id.as_deref(),
                        bounds: RectPt::from(*bounds_pt),
                        selected_rows: selected_rows.as_deref(),
                        selected_cols: selected_cols.as_deref(),
                        style,
                        missing_warning_code: "missing_embedded_result_table",
                    },
                    None,
                );
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
            GraphAnnotation::EmbeddedImage { asset_id, bounds_pt, rotation_deg, crop, border, .. } => {
                crate::image_decode::emit_embedded_image_primitive(
                    scene,
                    doc,
                    &crate::image_decode::EmbeddedImageDrawSpec {
                        asset_id,
                        bounds_pt: *bounds_pt,
                        rotation_deg: *rotation_deg,
                        crop: *crop,
                        border: *border,
                        clip: None,
                        badge_override: None,
                    },
                    None,
                );
            }
        }
    }
}

fn render_textbox_annotation(
    rect: RectPt,
    markdown: &str,
    bordered: bool,
    doc: &ProjectDocument,
    primary_id: Option<&SheetId>,
    scene: &mut ChartScene,
) {
    if bordered {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(rect),
            fill: Some(FillPaint::solid(Rgba8::rgba(255, 255, 255, 240))),
            stroke: Some(StrokePaint::solid(Rgba8::BLACK, 0.8)),
            clip: None,
        });
    }
    let expanded = resolve_template_hooks(markdown, doc, primary_id);
    let font_size = 8.5_f64;
    let line_h = font_size * 1.3;
    scene.primitives.push(ScenePrimitive::PushClip { rect });
    for (idx, line) in expanded.lines().enumerate() {
        let y_pt = rect.y + 4.0 + font_size + (idx as f64) * line_h;
        if y_pt > rect.bottom() + line_h {
            break;
        }
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(line),
            anchor_pt: PointPt::new(rect.x + 4.0, y_pt),
            anchor: TextAnchor::Start,
            font_size_pt: font_size,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
    scene.primitives.push(ScenePrimitive::PopClip);
}

fn render_line_or_arrow_annotation(
    start_pt: (f64, f64),
    end_pt: (f64, f64),
    arrowhead: bool,
    dash: LineDashStyle,
    color: Rgba8,
    scene: &mut ChartScene,
) {
    if !start_pt.0.is_finite() || !start_pt.1.is_finite() || !end_pt.0.is_finite() || !end_pt.1.is_finite() {
        scene.warnings.push(ChartWarning::new("non_finite_annotation", "skipped LineOrArrow annotation with non-finite coordinates"));
        return;
    }
    let p0 = PointPt::new(start_pt.0, start_pt.1);
    let p1 = PointPt::new(end_pt.0, end_pt.1);
    let stroke = StrokePaint { color, width_pt: 1.25, dash, cap: LineCap::Round, join: LineJoin::Round };
    scene.primitives.push(ScenePrimitive::Path { path: Path2D::line(p0, p1), fill: None, stroke: Some(stroke), clip: None });

    if arrowhead {
        let dx = p1.x - p0.x;
        let dy = p1.y - p0.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len > 1e-6 {
            let ux = dx / len;
            let uy = dy / len;
            let nx = -uy;
            let ny = ux;
            let head_len = 7.0_f64.min(len * 0.6);
            let half_w = 3.0_f64;
            let base_x = p1.x - ux * head_len;
            let base_y = p1.y - uy * head_len;
            let head = Path2D {
                commands: vec![
                    PathCmd::MoveTo(p1),
                    PathCmd::LineTo(PointPt::new(base_x + nx * half_w, base_y + ny * half_w)),
                    PathCmd::LineTo(PointPt::new(base_x - nx * half_w, base_y - ny * half_w)),
                    PathCmd::Close,
                ],
            };
            scene.primitives.push(ScenePrimitive::Path { path: head, fill: Some(FillPaint::solid(color)), stroke: None, clip: None });
        }
    }
}

fn render_shape_annotation(
    kind: AnnotationShapeKind,
    rect: RectPt,
    stroke_color: Rgba8,
    stroke_width_pt: f64,
    dash: LineDashStyle,
    fill_color: Option<Rgba8>,
    scene: &mut ChartScene,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let path = match kind {
        AnnotationShapeKind::Rectangle => Path2D::rect(rect),
        AnnotationShapeKind::RoundedRectangle => rounded_rect_path(rect),
        AnnotationShapeKind::Ellipse => ellipse_path(rect),
    };
    let fill = fill_color.map(FillPaint::solid);
    let stroke = (stroke_width_pt.is_finite() && stroke_width_pt > 0.0).then_some(StrokePaint {
        color: stroke_color,
        width_pt: stroke_width_pt,
        dash,
        cap: LineCap::Butt,
        join: LineJoin::Miter,
    });
    scene.primitives.push(ScenePrimitive::Path { path, fill, stroke, clip: None });
}

/// Construct a smooth closed cubic-Bezier ellipse inside `r`.
#[must_use]
pub fn ellipse_path(r: RectPt) -> Path2D {
    let cx = r.x + 0.5 * r.width;
    let cy = r.y + 0.5 * r.height;
    let rx = 0.5 * r.width;
    let ry = 0.5 * r.height;
    let ox = rx * ELLIPSE_KAPPA;
    let oy = ry * ELLIPSE_KAPPA;
    Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(cx, r.y)),
            PathCmd::CubicTo { c1: PointPt::new(cx + ox, r.y), c2: PointPt::new(r.right(), cy - oy), to: PointPt::new(r.right(), cy) },
            PathCmd::CubicTo { c1: PointPt::new(r.right(), cy + oy), c2: PointPt::new(cx + ox, r.bottom()), to: PointPt::new(cx, r.bottom()) },
            PathCmd::CubicTo { c1: PointPt::new(cx - ox, r.bottom()), c2: PointPt::new(r.x, cy + oy), to: PointPt::new(r.x, cy) },
            PathCmd::CubicTo { c1: PointPt::new(r.x, cy - oy), c2: PointPt::new(cx - ox, r.y), to: PointPt::new(cx, r.y) },
            PathCmd::Close,
        ],
    }
}

fn rounded_rect_path(r: RectPt) -> Path2D {
    let rad = crate::clamp_f64_safe(r.width.min(r.height) * 0.15, 1.0, 8.0);
    let x0 = r.x;
    let x1 = r.right();
    let y0 = r.y;
    let y1 = r.bottom();
    Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(x0 + rad, y0)),
            PathCmd::LineTo(PointPt::new(x1 - rad, y0)),
            PathCmd::QuadTo { control: PointPt::new(x1, y0), to: PointPt::new(x1, y0 + rad) },
            PathCmd::LineTo(PointPt::new(x1, y1 - rad)),
            PathCmd::QuadTo { control: PointPt::new(x1, y1), to: PointPt::new(x1 - rad, y1) },
            PathCmd::LineTo(PointPt::new(x0 + rad, y1)),
            PathCmd::QuadTo { control: PointPt::new(x0, y1), to: PointPt::new(x0, y1 - rad) },
            PathCmd::LineTo(PointPt::new(x0, y0 + rad)),
            PathCmd::QuadTo { control: PointPt::new(x0, y0), to: PointPt::new(x0 + rad, y0) },
            PathCmd::Close,
        ],
    }
}

/// Convenience helper compiling all chrome (background thresholds, axes/frame/ticks/labels,
/// foreground threshold rules, legend, and annotations) around `scene.plot_rect_pt`.
pub fn render_graph_chrome(graph: &GraphSpec, doc: &ProjectDocument, scene: &mut ChartScene, options: &AxisRenderOptions) -> CompiledAxes {
    scene.plot_rect_pt = compute_plot_rect(SizePt::new(scene.width_pt, scene.height_pt), graph, options);
    let axes = resolve_axes_and_ticks(graph, scene, options);
    render_thresholds_background(&axes, graph, scene, options);
    emit_axes_and_frame(&axes, graph, doc, scene, options);
    render_thresholds_lines(&axes, graph, doc, scene);
    let entries = legend_entries_from_graph_spec(graph, doc);
    render_legend(&graph.legend, &entries, doc, Some(&graph.primary_source_sheet_id), scene);
    render_annotations(&graph.annotations, doc, Some(&graph.primary_source_sheet_id), scene);
    axes
}

/// Format an arbitrary slice of [`ChartWarning`] entries into a deterministic diagnostic report line.
#[must_use]
pub fn summarize_warnings(warnings: &[ChartWarning]) -> String {
    let mut out = String::new();
    for (idx, w) in warnings.iter().enumerate() {
        if idx > 0 {
            out.push_str("; ");
        }
        let _ = write!(out, "[{}] {}", w.code, w.message);
    }
    out
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{AxisSegment, AxisSpec, TickLabelFormat};
    use curvebook_math::AxisScale;
    use curvebook_text::{LocaleNumberPolicy, SpanScript};

    use super::{
        AxisMapping, AxisOrientation, ResolvedAxis, format_tick_rich, nice_linear_bounds, resolve_sample_segment_domain, sanitize_linear_range,
    };
    use crate::scene::RectPt;

    #[test]
    fn rv20_tiny_linear_domains_and_constant_padding_preserve_scale() {
        // 1. Explicit `[0.0, 1e-16]` maps cleanly without returning `None` or expanding to `[-0.5, 0.5]`.
        let tiny_map = AxisMapping { min: 0.0, max: 1e-16, scale: AxisScale::Linear };
        let p0 = tiny_map.map_value(0.0, 0.0, 200.0).expect("map 0.0 on [0, 1e-16]");
        let p_mid = tiny_map.map_value(0.5e-16, 0.0, 200.0).expect("map midpoint on [0, 1e-16]");
        let p1 = tiny_map.map_value(1e-16, 0.0, 200.0).expect("map 1e-16 on [0, 1e-16]");
        assert!((p0 - 0.0).abs() < 1e-12);
        assert!((p_mid - 100.0).abs() < 1e-12);
        assert!((p1 - 200.0).abs() < 1e-12);
        let roundtrip = tiny_map.unmap_coord(p_mid, 0.0, 200.0).expect("unmap midpoint on [0, 1e-16]");
        assert!((roundtrip - 0.5e-16).abs() < 1e-30);

        let mut explicit_spec = AxisSpec::linear("Tiny");
        explicit_spec.segments[0].min = Some(0.0);
        explicit_spec.segments[0].max = Some(1e-16);
        let mut warnings = Vec::new();
        let resolved_tiny =
            ResolvedAxis::resolve(&explicit_spec, AxisOrientation::Horizontal, RectPt::new(0.0, 0.0, 200.0, 100.0), None, &mut warnings);
        assert!(warnings.is_empty(), "explicit [0, 1e-16] should not warn: {warnings:?}");
        assert_eq!((resolved_tiny.segments[0].domain_min, resolved_tiny.segments[0].domain_max), (0.0, 1e-16));
        assert!((resolved_tiny.map_value(0.5e-16).expect("segment map").coord_pt - 100.0).abs() < 1e-12);

        // 2. Auto-domain on picomolar/femtomolar `[1e-13, 6e-13]` (span `5e-13`) is NOT treated as degenerate or padded by +/-1.0.
        let (n_lo, n_hi) = nice_linear_bounds(1e-13, 6e-13);
        assert!((0.0..=1e-13).contains(&n_lo), "unexpected nice_lo: {n_lo:e}");
        assert!((6e-13..=1e-12).contains(&n_hi), "unexpected nice_hi: {n_hi:e}");

        let (san_lo, san_hi) = sanitize_linear_range(1e-13, 6e-13, (0.0, 1.0));
        assert_eq!((san_lo, san_hi), (1e-13, 6e-13));

        let seg_auto = AxisSegment::default();
        let mut sample_warnings = Vec::new();
        let (s_lo, s_hi) = resolve_sample_segment_domain(&seg_auto, &[1e-13, 6e-13], false, 0.05, &mut sample_warnings);
        assert!(sample_warnings.is_empty());
        assert!(s_lo <= 1e-13 && s_hi >= 6e-13 && (s_hi - s_lo) < 1e-11, "picomolar sample auto-domain collapsed: [{s_lo:e}, {s_hi:e}]");

        // 3. Single-value `1e-12` pads proportionally by `|lo| * 0.1` (`1e-13`), and `1.0` ONLY when `lo == 0.0`.
        let (single_lo, single_hi) = nice_linear_bounds(1e-12, 1e-12);
        assert!((single_lo - 0.9e-12).abs() < 1e-24, "expected 0.9e-12, got {single_lo:e}");
        assert!((single_hi - 1.1e-12).abs() < 1e-24, "expected 1.1e-12, got {single_hi:e}");

        let (san_single_lo, san_single_hi) = sanitize_linear_range(1e-12, 1e-12, (0.0, 1.0));
        assert!((san_single_lo - 0.9e-12).abs() < 1e-24);
        assert!((san_single_hi - 1.1e-12).abs() < 1e-24);

        let (single_samp_lo, single_samp_hi) = resolve_sample_segment_domain(&seg_auto, &[1e-12], false, 0.0, &mut sample_warnings);
        assert!(single_samp_lo > 0.0 && single_samp_lo < 1e-12 && single_samp_hi > 1e-12 && single_samp_hi < 2e-12);

        let (zero_lo, zero_hi) = nice_linear_bounds(0.0, 0.0);
        assert_eq!((zero_lo, zero_hi), (-1.0, 1.0));
    }

    #[test]
    fn rv21_scientific_and_engineering_si_ticks_carry_after_rounding() {
        let policy = LocaleNumberPolicy::publication();

        // Scientific superscript: `9.9996e5` rounds to `1.000e6` -> `10^6` (never `10×10^5`).
        let pos_sup = format_tick_rich(9.9996e5, AxisScale::Log10, TickLabelFormat::PowerOfTenSuperscript, &policy);
        assert_eq!(pos_sup.spans.len(), 2, "expected 10^6 spans, got {:?}", pos_sup.spans);
        assert_eq!(pos_sup.spans[0].text, "10");
        assert_eq!(pos_sup.spans[1].script, SpanScript::Superscript);
        assert_eq!(pos_sup.spans[1].text, "6");

        let neg_sup = format_tick_rich(-9.9996e5, AxisScale::Linear, TickLabelFormat::PowerOfTenSuperscript, &policy);
        assert_eq!(neg_sup.spans.len(), 2, "expected -10^6 spans, got {:?}", neg_sup.spans);
        assert_eq!(neg_sup.spans[0].text, "\u{2212}10");
        assert_eq!(neg_sup.spans[1].script, SpanScript::Superscript);
        assert_eq!(neg_sup.spans[1].text, "6");

        // Scientific E: `9.9996e5` -> `1e6`, `-9.9996e5` -> `−1e6`.
        let pos_sci = format_tick_rich(9.9996e5, AxisScale::Linear, TickLabelFormat::ScientificE, &policy);
        assert_eq!(pos_sci.plain_text(), "1e6");
        let neg_sci = format_tick_rich(-9.9996e5, AxisScale::Linear, TickLabelFormat::ScientificE, &policy);
        assert_eq!(neg_sci.plain_text(), "\u{2212}1e6");

        // Engineering SI: `999.9996` -> `1 k` (not `1000`), `999_999.6` -> `1 M` (not `1000 k`), plus negative counterparts.
        let pos_si = format_tick_rich(999.9996, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy);
        assert_eq!(pos_si.plain_text(), "1 k");
        let neg_si = format_tick_rich(-999.9996, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy);
        assert_eq!(neg_si.plain_text(), "\u{2212}1 k");

        let pos_si_mega = format_tick_rich(999_999.6, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy);
        assert_eq!(pos_si_mega.plain_text(), "1 M");
        let neg_si_mega = format_tick_rich(-999_999.6, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy);
        assert_eq!(neg_si_mega.plain_text(), "\u{2212}1 M");
    }
}
