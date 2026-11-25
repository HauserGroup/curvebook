//! Discontinuous multi-segment axis resolution, auto-range headroom policy, and segment-aware clipping.

use curvebook_doc::{AxisSegment, AxisSpec, BreakOrnament};
use curvebook_math::{AxisScale, LogitDomain};

use super::mapping::{AxisMapping, ScaleDiagnostics, scale_transform, transformed_fraction};
use super::ticks::{clean_zero, nice_linear_step};
use super::{LOG_POSITIVE_FLOOR_DEFAULT, MIN_FINITE_AXIS_SPAN};
use crate::scene::{ChartWarning, Path2D, PathCmd, PointPt, RectPt};

const MAX_SEGMENTS_PER_AXIS: usize = 3;

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
        let half_span = (0.5 * hi - 0.5 * lo).abs();
        let tol = 2e-12 * half_span.max(lo.abs()).max(hi.abs());
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
        let half_span = (0.5 * g_max - 0.5 * g_min).abs();
        if !half_span.is_finite() || half_span <= 0.5 * MIN_FINITE_AXIS_SPAN {
            return None;
        }
        let tol = 2e-9 * half_span.max(g_min.abs().max(g_max.abs()) * f64::EPSILON);
        let lo = g_min.min(g_max) - tol;
        let hi = g_min.max(g_max) + tol;
        if g_u < lo || g_u > hi {
            return None;
        }
        let s = transformed_fraction(g_u, g_min, g_max)?.clamp(0.0, 1.0);
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
                    let s = transformed_fraction(g_safe.clamp(lo, hi), g_min, g_max).unwrap_or(0.0).clamp(0.0, 1.0);
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
                let s = transformed_fraction(fallback, g_min, g_max).unwrap_or(0.0).clamp(0.0, 1.0);
                return (self.span_start_pt + s * (self.span_end_pt - self.span_start_pt), true);
            }
        };
        let clipped = g_val < lo || g_val > hi;
        let s = transformed_fraction(g_val.clamp(lo, hi), g_min, g_max).unwrap_or(0.0).clamp(0.0, 1.0);
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
    /// Resolve a segmented axis directly from raw data samples with explicit headroom and zero-inclusion rules.
    pub fn resolve_from_samples(
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
    let u = transformed_fraction(t_val, t_min, t_max)?;
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
        // Overflow of `dmax - dmin` only happens for huge opposite-sign finite bounds.
        span.is_finite() && span.abs() <= MIN_FINITE_AXIS_SPAN
    } else {
        match (scale_transform(scale, dmin), scale_transform(scale, dmax)) {
            (Some(t_min), Some(t_max)) => {
                let t_span = t_max - t_min;
                if t_span.is_finite() {
                    let thresh =
                        if matches!(scale, AxisScale::Reciprocal) { (1e-12 * t_min.abs().max(t_max.abs())).max(MIN_FINITE_AXIS_SPAN) } else { 1e-12 };
                    t_span.abs() < thresh
                } else {
                    false
                }
            }
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
            let lo = if (safe_center - pad).is_finite() { safe_center - pad } else { -f64::MAX };
            let hi = if (safe_center + pad).is_finite() { safe_center + pad } else { f64::MAX };
            (lo, hi)
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
    let valid_span = !is_degenerate_domain(lo, hi, seg.scale);
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
    if explicit_min.is_some() && explicit_max.is_none() && dmax <= dmin {
        dmax = extend_upper_bound(dmin, scale);
    } else if explicit_max.is_some() && explicit_min.is_none() && dmin >= dmax {
        dmin = extend_lower_bound(dmax, scale);
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
    if span.is_finite() && span.abs() <= MIN_FINITE_AXIS_SPAN {
        let pad = linear_constant_pad(lo);
        let new_lo = if (lo - pad).is_finite() { lo - pad } else { -f64::MAX };
        let new_hi = if (hi + pad).is_finite() { hi + pad } else { f64::MAX };
        if new_lo < new_hi { (new_lo, new_hi) } else { fallback }
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

fn repair_log_lower_from_upper(hi: f64, scale: AxisScale) -> f64 {
    for factor in [0.01, 0.1, 0.5] {
        let cand = hi * factor;
        if cand.is_finite() && cand > 0.0 && cand < hi && scale_transform(scale, cand).is_some() {
            return cand;
        }
    }
    hi
}

fn repair_log_exact_domain(
    mut lo: f64,
    mut hi: f64,
    raw_lo: f64,
    raw_hi: f64,
    has_explicit: bool,
    scale: AxisScale,
    warnings: &mut Vec<ChartWarning>,
) -> (f64, f64) {
    let lo_ok = lo > 0.0 && scale_transform(scale, lo).is_some();
    let hi_ok = hi > 0.0 && scale_transform(scale, hi).is_some();
    if lo_ok && hi_ok {
        return (lo, hi);
    }
    if has_explicit {
        warnings.push(ChartWarning::new("invalid_axis_domain", format!("repaired non-positive domain [{lo}, {hi}] for {scale:?} axis")));
    }
    if !lo_ok {
        if hi_ok {
            lo =
                if raw_lo > 0.0 && raw_lo < hi && scale_transform(scale, raw_lo).is_some() { raw_lo } else { repair_log_lower_from_upper(hi, scale) };
        } else if raw_lo > 0.0 && scale_transform(scale, raw_lo).is_some() {
            lo = raw_lo;
        } else {
            return sample_default_domain(scale);
        }
    }
    if !hi_ok {
        hi = if raw_hi > lo && scale_transform(scale, raw_hi).is_some() { raw_hi } else { extend_upper_bound(lo, scale) };
    }
    if scale_transform(scale, lo).is_some() && scale_transform(scale, hi).is_some() { (lo, hi) } else { sample_default_domain(scale) }
}

fn repair_reciprocal_exact_domain(
    mut lo: f64,
    mut hi: f64,
    raw_lo: f64,
    raw_hi: f64,
    explicit_min: Option<f64>,
    explicit_max: Option<f64>,
    warnings: &mut Vec<ChartWarning>,
) -> (f64, f64) {
    let valid_explicit_min = explicit_min.is_some_and(|v| v.is_finite() && v != 0.0);
    let valid_explicit_max = explicit_max.is_some_and(|v| v.is_finite() && v != 0.0);
    if valid_explicit_min && !valid_explicit_max && (hi == 0.0 || !hi.is_finite() || hi.signum() != lo.signum()) {
        hi = extend_upper_bound(lo, AxisScale::Reciprocal);
    } else if valid_explicit_max && !valid_explicit_min && (lo == 0.0 || !lo.is_finite() || lo.signum() != hi.signum()) {
        lo = extend_lower_bound(hi, AxisScale::Reciprocal);
    }
    if lo.is_finite() && hi.is_finite() && lo != 0.0 && hi != 0.0 && lo.signum() == hi.signum() {
        return (lo, hi);
    }
    if explicit_min.is_some() || explicit_max.is_some() || (lo < 0.0 && hi > 0.0) || (lo > 0.0 && hi < 0.0) {
        warnings.push(ChartWarning::new("invalid_axis_domain", format!("repaired zero-crossing or invalid domain [{lo}, {hi}] for Reciprocal axis")));
    }
    if hi > 0.0 && lo <= 0.0 {
        lo = if raw_lo > 0.0 && raw_lo < hi {
            raw_lo
        } else if hi * 0.01 > 0.0 {
            hi * 0.01
        } else {
            hi * 0.5
        };
    } else if lo > 0.0 && hi <= 0.0 {
        hi = if raw_hi > 0.0 && raw_hi < lo {
            raw_hi
        } else if lo * 0.01 > 0.0 {
            lo * 0.01
        } else {
            lo * 0.5
        };
    } else if lo < 0.0 && hi >= 0.0 {
        hi = if raw_hi < 0.0 && raw_hi > lo {
            raw_hi
        } else if lo * 0.01 < 0.0 {
            lo * 0.01
        } else {
            lo * 0.5
        };
    } else if hi < 0.0 && lo >= 0.0 {
        lo = if raw_lo < 0.0 && raw_lo > hi {
            raw_lo
        } else if hi * 0.01 < 0.0 {
            hi * 0.01
        } else {
            hi * 0.5
        };
    } else {
        return sample_default_domain(AxisScale::Reciprocal);
    }
    if lo.is_finite() && hi.is_finite() && lo != 0.0 && hi != 0.0 && lo.signum() == hi.signum() {
        (lo, hi)
    } else {
        sample_default_domain(AxisScale::Reciprocal)
    }
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

    match scale {
        AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => {
            (lo, hi) = repair_log_exact_domain(lo, hi, raw_lo, raw_hi, explicit_min.is_some() || explicit_max.is_some(), scale, warnings);
        }
        AxisScale::Logit(domain) => {
            let upper = match domain {
                LogitDomain::Percent => 100.0,
                LogitDomain::Proportion => 1.0,
            };
            lo = repair_prob_bound(lo, true, explicit_min.is_some(), raw_lo, upper, scale, warnings);
            hi = repair_prob_bound(hi, false, explicit_max.is_some(), raw_hi, upper, scale, warnings);
        }
        AxisScale::Probit => {
            lo = repair_prob_bound(lo, true, explicit_min.is_some(), raw_lo, 100.0, scale, warnings);
            hi = repair_prob_bound(hi, false, explicit_max.is_some(), raw_hi, 100.0, scale, warnings);
        }
        AxisScale::Reciprocal => {
            (lo, hi) = repair_reciprocal_exact_domain(lo, hi, raw_lo, raw_hi, explicit_min, explicit_max, warnings);
        }
        AxisScale::Linear => {}
    }

    let valid_explicit_min = explicit_min.is_some_and(|v| scale_transform(scale, v).is_some());
    let valid_explicit_max = explicit_max.is_some_and(|v| scale_transform(scale, v).is_some());
    if valid_explicit_min && !valid_explicit_max && hi <= lo {
        hi = extend_upper_bound(lo, scale);
    } else if valid_explicit_max && !valid_explicit_min && lo >= hi {
        lo = extend_lower_bound(hi, scale);
    }

    if is_degenerate_domain(lo, hi, scale) {
        if explicit_min.is_some() && explicit_max.is_some() {
            warnings.push(ChartWarning::new("invalid_axis_domain", format!("expanded degenerate explicit domain [{lo}, {hi}] for {scale:?} axis")));
        }
        (lo, hi) = expand_degenerate_domain(lo, scale);
        if is_degenerate_domain(lo, hi, scale) {
            return sample_default_domain(scale);
        }
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

/// Explicit autoranging headroom policy (`R20.5`).
///
/// Governs automatic upper-bound headroom on segments whose `max` is `None` (`auto_range == true`).
/// Segments with explicit manual bounds (`AxisSegment::min = Some(_)` or `max = Some(_)`) are
/// **never** modified by this policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoRangeHeadroomPolicy {
    /// Base linear headroom fraction added above `raw_max` before nice-step snapping (`0.08`).
    pub base_linear_headroom_frac: f64,
    /// Default extra top headroom fraction applied to XY Y-axes (`0.05`).
    pub default_xy_top_headroom_frac: f64,
    /// Upper clamp on caller-supplied `top_headroom_frac` to prevent non-finite or extreme blowout (`1.0`).
    pub max_extra_headroom_frac: f64,
    /// When `true`, strictly negative linear data (`raw_max < 0.0`, `raw_min < raw_max`) clamps
    /// its top-headroom padded upper bound at `0.0` instead of overshooting into positive space.
    pub clamp_negative_max_at_zero: bool,
}

/// Canonical `R20.5` autoranging headroom policy for `curvebook-chart`.
pub const DEFAULT_AUTORANGE_HEADROOM: AutoRangeHeadroomPolicy = AutoRangeHeadroomPolicy {
    base_linear_headroom_frac: 0.08,
    default_xy_top_headroom_frac: 0.05,
    max_extra_headroom_frac: 1.0,
    clamp_negative_max_at_zero: true,
};

/// Returns `true` when `seg` has at least one explicit manual bound (`auto_range == false` on
/// lower, upper, or both ends).
#[must_use]
pub const fn segment_has_manual_limits(seg: &AxisSegment) -> bool {
    seg.min.is_some() || seg.max.is_some()
}

/// Returns `true` when both ends of `seg` are automatic (`min.is_none() && max.is_none()`).
#[must_use]
pub const fn segment_is_auto_range(seg: &AxisSegment) -> bool {
    seg.min.is_none() && seg.max.is_none()
}

fn sanitize_top_headroom_frac(top_headroom_frac: f64) -> f64 {
    if top_headroom_frac.is_finite() { top_headroom_frac.clamp(0.0, DEFAULT_AUTORANGE_HEADROOM.max_extra_headroom_frac) } else { 0.0 }
}

fn log_auto_domain_max(raw_max: f64, scale: AxisScale, top_headroom_frac: f64) -> f64 {
    let headroom = sanitize_top_headroom_frac(top_headroom_frac);
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
            let base = if include_zero_if_linear && raw_min > 0.0 { 0.0 } else { raw_min };
            let raw_span = raw_max - base;
            let (step, padded_base) = if raw_span.is_finite() {
                let span = raw_span.max(base.abs() * 0.1);
                let safe_span = if span.is_finite() && span > MIN_FINITE_AXIS_SPAN { span } else { 1.0 };
                let adj = if !include_zero_if_linear || base < 0.0 {
                    let b = base - 0.06 * safe_span;
                    if b.is_finite() { b } else { base }
                } else {
                    base
                };
                (nice_linear_step(safe_span / 5.0), adj)
            } else {
                let half_span = (0.5 * raw_max - 0.5 * base).max(MIN_FINITE_AXIS_SPAN);
                let half_step = nice_linear_step(half_span / 5.0);
                let full_step = if (2.0 * half_step).is_finite() { 2.0 * half_step } else { half_span };
                let adj = if !include_zero_if_linear || base < 0.0 {
                    let b = base - 0.12 * half_span;
                    if b.is_finite() { b } else { base }
                } else {
                    base
                };
                (full_step, adj)
            };
            let snapped = (padded_base / step).floor() * step;
            clean_zero(if snapped.is_finite() { snapped } else { base })
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
            let raw_span = raw_max - raw_min;
            let headroom = DEFAULT_AUTORANGE_HEADROOM.base_linear_headroom_frac + sanitize_top_headroom_frac(top_headroom_frac);
            let clamp_neg_to_zero = DEFAULT_AUTORANGE_HEADROOM.clamp_negative_max_at_zero && raw_max < 0.0 && raw_min < raw_max;
            let (step, padded) = if raw_span.is_finite() {
                let span = raw_span.max(raw_max.abs() * 0.1);
                let safe_span = if span.is_finite() && span > MIN_FINITE_AXIS_SPAN { span } else { 1.0 };
                let p = raw_max + safe_span * headroom;
                let clamped = if clamp_neg_to_zero && p > 0.0 { 0.0 } else { p };
                (nice_linear_step(safe_span / 5.0), if clamped.is_finite() { clamped } else { raw_max })
            } else {
                let half_span = (0.5 * raw_max - 0.5 * raw_min).max(MIN_FINITE_AXIS_SPAN);
                let half_step = nice_linear_step(half_span / 5.0);
                let full_step = if (2.0 * half_step).is_finite() { 2.0 * half_step } else { half_span };
                let p = raw_max + 2.0 * half_span * headroom;
                let clamped = if clamp_neg_to_zero && p > 0.0 { 0.0 } else { p };
                (full_step, if clamped.is_finite() { clamped } else { raw_max })
            };
            let snapped = (padded / step).ceil() * step;
            clean_zero(if snapped.is_finite() { snapped } else { raw_max })
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

pub(super) fn nice_linear_bounds(lo: f64, hi: f64) -> (f64, f64) {
    let span = hi - lo;
    if span.is_finite() {
        if span.abs() <= MIN_FINITE_AXIS_SPAN {
            let pad = linear_constant_pad(lo);
            let new_lo = if (lo - pad).is_finite() { lo - pad } else { -f64::MAX };
            let new_hi = if (hi + pad).is_finite() { hi + pad } else { f64::MAX };
            return (new_lo, new_hi);
        }
        let step = nice_linear_step(span / 5.0);
        let nice_lo = if lo >= 0.0 && lo <= step { 0.0 } else { (lo / step).floor() * step };
        let nice_hi = (hi / step).ceil() * step;
        let out_hi = if nice_hi.is_finite() { nice_hi } else { hi };
        let out_lo = if nice_lo.is_finite() { nice_lo } else { lo };
        (clean_zero(out_lo), clean_zero(out_hi))
    } else {
        let half_span = 0.5 * hi - 0.5 * lo;
        let half_step = nice_linear_step(half_span / 5.0);
        let step = if (2.0 * half_step).is_finite() { 2.0 * half_step } else { half_span };
        let nice_lo = (lo / step).floor() * step;
        let nice_hi = (hi / step).ceil() * step;
        let out_lo = if nice_lo.is_finite() { nice_lo } else { lo };
        let out_hi = if nice_hi.is_finite() { nice_hi } else { hi };
        (clean_zero(out_lo), clean_zero(out_hi))
    }
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
            if lo > 0.0 && lo.is_finite() {
                let mult = if matches!(scale, AxisScale::Log10) { 10.0 } else { 2.0 };
                let scaled = lo * mult;
                if scaled.is_finite() && scaled > lo {
                    return scaled;
                }
                let fallback = lo * 2.0;
                if fallback.is_finite() && fallback > lo { fallback } else { f64::MAX }
            } else {
                LOG_POSITIVE_FLOOR_DEFAULT * 10.0
            }
        }
        AxisScale::Logit(LogitDomain::Proportion) => prob_auto_domain_max(lo, 1.0, scale),
        AxisScale::Logit(LogitDomain::Percent) | AxisScale::Probit => prob_auto_domain_max(lo, 100.0, scale),
        AxisScale::Linear => {
            let cand = lo + linear_constant_pad(lo);
            if cand.is_finite() { cand } else { f64::MAX }
        }
        AxisScale::Reciprocal => {
            if lo < 0.0 {
                lo * 0.5
            } else {
                let cand = lo + lo.abs().max(1.0);
                if cand.is_finite() { cand } else { f64::MAX }
            }
        }
    }
}

fn extend_lower_bound(hi: f64, scale: AxisScale) -> f64 {
    match scale {
        AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => {
            if hi > 0.0 && hi.is_finite() {
                let factor = if matches!(scale, AxisScale::Log10) { 0.1 } else { 0.5 };
                for mult in [factor, 0.5] {
                    let cand = hi * mult;
                    if cand > 0.0 && cand < hi && scale_transform(scale, cand).is_some() {
                        return cand;
                    }
                }
                hi
            } else {
                LOG_POSITIVE_FLOOR_DEFAULT
            }
        }
        AxisScale::Logit(LogitDomain::Proportion) => prob_auto_domain_min(hi, 1.0, scale),
        AxisScale::Logit(LogitDomain::Percent) | AxisScale::Probit => prob_auto_domain_min(hi, 100.0, scale),
        AxisScale::Linear => {
            let cand = hi - linear_constant_pad(hi);
            if cand.is_finite() { cand } else { -f64::MAX }
        }
        AxisScale::Reciprocal => {
            if hi < 0.0 {
                let cand = hi * 2.0;
                if cand.is_finite() { cand } else { -f64::MAX }
            } else {
                let cand = hi * 0.5;
                if cand > 0.0 && cand < hi && scale_transform(scale, cand).is_some() { cand } else { hi }
            }
        }
    }
}
