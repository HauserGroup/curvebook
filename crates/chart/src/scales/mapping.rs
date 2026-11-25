//! Guarded axis-scale transformations, inverse mappings, diagnostics, and error-bar clamping.

use curvebook_math::dist::{norm_cdf, norm_ppf};
use curvebook_math::{AxisScale, LogitDomain};

use super::{LOG_POSITIVE_FLOOR_DEFAULT, MIN_FINITE_AXIS_SPAN};
use crate::scene::ChartWarning;

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

/// Compute `(t_val - t_min) / (t_max - t_min)` without overflowing intermediate differences
/// when endpoints span extreme finite magnitudes (e.g. `[-1e308, 1e308]`).
pub(super) fn transformed_fraction(t_val: f64, t_min: f64, t_max: f64) -> Option<f64> {
    if !t_val.is_finite() || !t_min.is_finite() || !t_max.is_finite() {
        return None;
    }
    let denom = t_max - t_min;
    let num = t_val - t_min;
    if denom.is_finite() && num.is_finite() {
        if denom.abs() <= MIN_FINITE_AXIS_SPAN {
            return None;
        }
        let u = num / denom;
        return u.is_finite().then_some(u);
    }
    let half_denom = 0.5 * t_max - 0.5 * t_min;
    if !half_denom.is_finite() || half_denom.abs() <= MIN_FINITE_AXIS_SPAN {
        return None;
    }
    let half_num = 0.5 * t_val - 0.5 * t_min;
    if !half_num.is_finite() {
        return None;
    }
    let u = half_num / half_denom;
    u.is_finite().then_some(u)
}

/// Interpolate `t_min + u * (t_max - t_min)` without intermediate overflow on extreme spans.
pub(super) fn interpolate_transformed(u: f64, t_min: f64, t_max: f64) -> Option<f64> {
    if !u.is_finite() || !t_min.is_finite() || !t_max.is_finite() {
        return None;
    }
    let denom = t_max - t_min;
    if denom.is_finite() {
        if denom.abs() <= MIN_FINITE_AXIS_SPAN {
            return None;
        }
        let t_val = t_min + u * denom;
        return t_val.is_finite().then_some(t_val);
    }
    let half_denom = 0.5 * t_max - 0.5 * t_min;
    if !half_denom.is_finite() || half_denom.abs() <= MIN_FINITE_AXIS_SPAN {
        return None;
    }
    let half_t = 0.5 * t_min + u * half_denom;
    let t_val = 2.0 * half_t;
    t_val.is_finite().then_some(t_val)
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
        let u = transformed_fraction(t_val, t_min, t_max)?;
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
        let u = (coord_pt - span_start) / span;
        let t_val = interpolate_transformed(u, t_min, t_max)?;
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

    pub(super) fn clamp_error_bound(self, center: f64, bound: f64, is_lower: bool, diag: &mut ScaleDiagnostics) -> (f64, bool) {
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
