//! Standalone frequency distribution and histogram binning engine.
//!
//! Partitions finite numeric samples into uniform right-closed bins `[e_0, e_1]`, `(e_1, e_2]`,
//! ..., `(e_{k-1}, e_k]` using either an automatic bin-width rule or an explicit positive bin width
//! with optional first/last bin centers. Computes raw counts, relative frequencies, percentages,
//! probability densities, cumulative counts/frequencies/percentages (exact empirical CDF evaluated
//! at each upper bin edge `e_{j+1}`), and an optional Gaussian density or CDF overlay.
//!
//! # References
//!
//! - Sturges, H. A. (1926). The choice of a class interval. *Journal of the American Statistical
//!   Association*, 21(153), 65–66. `doi:10.1080/01621459.1926.10502161`.
//! - Scott, D. W. (1979). On optimal and data-based histograms. *Biometrika*, 66(3), 605–610.
//!   `doi:10.1093/biomet/66.3.605`.
//! - Freedman, D., & Diaconis, P. (1981). On the histogram as a density estimator: L2 theory.
//!   *Zeitschrift für Wahrscheinlichkeitstheorie und verwandte Gebiete*, 57(4), 453–476.
//!   `doi:10.1007/BF01025868`.
//! - Hyndman, R. J., & Fan, Y. (1996). Sample quantiles in statistical packages. *The American
//!   Statistician*, 50(4), 361–365. `doi:10.1080/00031305.1996.10473566`.
//!
//! # Edge cases
//!
//! Every failure returns a typed [`StatsError`]; nothing panics.
//!
//! - Empty input or zero active finite values → [`StatsError::EmptyInput`].
//! - Non-finite (`NaN`, `±Inf`) observations in the slice API → [`StatsError::Math`]
//!   (`MathError::DomainError`). Sheet extraction ignores text/blank/excluded cells and reports
//!   ignored text cells in the diagnostic checklist.
//! - Constant data (`min == max`) or single observation (`n == 1`) under an automatic rule →
//!   single unit-width bin `[c - 0.5, c + 0.5]` centered at `c`, with a diagnostic checklist note.
//! - Zero IQR (`IQR == 0`) with positive spread under Freedman–Diaconis → falls back to Scott's
//!   rule (when `s > 0`) or Sturges' rule, with a diagnostic checklist note.
//! - Non-positive or non-finite explicit bin width, `first_bin_center > last_bin_center`, or a
//!   bin configuration producing more than [`MAX_HISTOGRAM_BINS`] bins →
//!   [`StatsError::InvalidParameter`].
//! - Pre-summarized `Column`, `Xy`, or `Grouped` subcolumn modes →
//!   [`StatsError::RequiresRawReplicates`].

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ChecklistStatus, CurveSamplePoint, DataSheet, FittedCurveOverlay, FrequencyBinningRule,
    FrequencyDistributionSpec, FrequencyOutputMode, ResultSheet, ResultTabKind, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_math::dist::{norm_cdf, norm_pdf};
use curvebook_math::sum::SummaryMoments;
use serde::{Deserialize, Serialize};

use crate::column::extract_indexed_series_for_descriptive;
use crate::common::{
    TabBuilder, append_ignored_text_checklist_and_footnote, checklist_item, enforce_summary_mode_matrix, init_result_sheet, num_cell, opt_num_cell,
    push_scalar, quantile_sorted, sanitize_result_sheet_floats, sorted_finite, validate_finite_slice,
};
use crate::error::StatsError;

/// Upper safety ceiling on the number of histogram bins generated from any input or explicit width.
pub const MAX_HISTOGRAM_BINS: usize = 10_000;

/// Number of evenly spaced curve sample points emitted for an optional Gaussian density/CDF overlay.
pub const GAUSSIAN_OVERLAY_POINTS: usize = 201;

/// Per-bin frequency distribution statistics for one histogram bin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistogramBin {
    /// Lower bin edge `e_j`.
    pub edge_low: f64,
    /// Upper bin edge `e_{j+1}`.
    pub edge_high: f64,
    /// Bin midpoint `0.5 * (e_j + e_{j+1})`.
    pub center: f64,
    /// Bin width `e_{j+1} - e_j`.
    pub width: f64,
    /// Raw number of active observations falling into this bin.
    pub count: usize,
    /// Relative frequency `count / N` in `[0, 1]`.
    pub relative_frequency: f64,
    /// Percentage `100 * count / N` in `[0, 100]`.
    pub percentage: f64,
    /// Probability density `count / (N * width)` integrating to `1.0` across all bins when no observations fall outside `[e_0, e_k]`.
    pub density: f64,
    /// Cumulative number of observations `<= e_{j+1}` (including any observations below `e_0`).
    pub cumulative_count: usize,
    /// Cumulative relative frequency `ecdf(e_{j+1})` in `[0, 1]`.
    pub cumulative_frequency: f64,
    /// Cumulative percentage `100 * ecdf(e_{j+1})` in `[0, 100]`.
    pub cumulative_percentage: f64,
}

impl HistogramBin {
    /// Return the bin value corresponding to `mode`.
    #[must_use]
    pub const fn value_for_mode(&self, mode: FrequencyOutputMode) -> f64 {
        match mode {
            FrequencyOutputMode::Counts => self.count as f64,
            FrequencyOutputMode::RelativeFrequency => self.relative_frequency,
            FrequencyOutputMode::Percentages => self.percentage,
            FrequencyOutputMode::Density => self.density,
            FrequencyOutputMode::CumulativeCounts => self.cumulative_count as f64,
            FrequencyOutputMode::CumulativeFrequency => self.cumulative_frequency,
            FrequencyOutputMode::CumulativePercentages => self.cumulative_percentage,
        }
    }
}

/// Complete histogram & moment summary for one data series over shared bin edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesHistogramResult {
    /// Zero-based column/dataset index on the source table.
    pub data_set_index: usize,
    /// Series display label.
    pub label: String,
    /// Number of active finite observations in this series.
    pub n: usize,
    /// Number of user-excluded cells omitted from this series.
    pub excluded_count: usize,
    /// Observations strictly below the first bin's lower edge `e_0`.
    pub underflow_count: usize,
    /// Observations strictly above the last bin's upper edge `e_k`.
    pub overflow_count: usize,
    /// Sample arithmetic mean of active observations.
    pub mean: f64,
    /// Sample standard deviation (`n - 1` denominator; `None` when `n < 2`).
    pub sd: Option<f64>,
    /// Sample interquartile range `Q3 - Q1` using Hyndman–Fan Type-7 quantiles.
    pub iqr: f64,
    /// Ordered sequence of histogram bins.
    pub bins: Vec<HistogramBin>,
    /// Optional fitted Gaussian density or CDF overlay evaluated across `[e_0, e_k]`.
    pub gaussian_overlay: Option<Vec<(f64, f64)>>,
}

/// Multi-series frequency distribution output sharing common uniform bin edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrequencyDistributionResult {
    /// Requested bin-width selection rule.
    pub binning_rule: FrequencyBinningRule,
    /// Optional diagnostic note explaining any automatic fallback (constant data, zero IQR).
    pub effective_rule_note: Option<String>,
    /// Uniform bin width `w > 0`.
    pub bin_width: f64,
    /// Strictly ascending bin boundary edges of length `k + 1`.
    pub edges: Vec<f64>,
    /// Bin midpoints of length `k`.
    pub centers: Vec<f64>,
    /// Primary output scale requested.
    pub output_mode: FrequencyOutputMode,
    /// Per-series histogram tabulations in input order.
    pub series: Vec<SeriesHistogramResult>,
}

/// Compute Sturges' (1926) bin count `ceil(log2(n)) + 1` (returning `1` for `n <= 1`).
#[must_use]
pub fn nclass_sturges(n: usize) -> usize {
    if n <= 1 {
        return 1;
    }
    let k = (n as f64).log2().ceil();
    (k as usize).saturating_add(1).max(1)
}

/// Compute Scott's (1979) bin count `ceil((max - min) / (3.5 * s * n^(-1/3)))`.
///
/// Returns `None` when `n < 2`, `sd <= 0`, or `range <= 0`.
#[must_use]
pub fn nclass_scott(n: usize, sd: f64, range: f64) -> Option<usize> {
    if n < 2 || !sd.is_finite() || sd <= 0.0 || !range.is_finite() || range <= 0.0 {
        return None;
    }
    let h = 3.5 * sd / (n as f64).cbrt();
    if !h.is_finite() || h <= 0.0 {
        return None;
    }
    let k = (range / h).ceil();
    if !k.is_finite() || k < 1.0 {
        return None;
    }
    Some((k as usize).max(1))
}

/// Compute Freedman & Diaconis' (1981) bin count `ceil((max - min) / (2 * IQR * n^(-1/3)))`.
///
/// Returns `None` when `n < 2`, `iqr <= 0`, or `range <= 0`.
#[must_use]
pub fn nclass_freedman_diaconis(n: usize, iqr: f64, range: f64) -> Option<usize> {
    if n < 2 || !iqr.is_finite() || iqr <= 0.0 || !range.is_finite() || range <= 0.0 {
        return None;
    }
    let h = 2.0 * iqr / (n as f64).cbrt();
    if !h.is_finite() || h <= 0.0 {
        return None;
    }
    let k = (range / h).ceil();
    if !k.is_finite() || k < 1.0 {
        return None;
    }
    Some((k as usize).max(1))
}

/// Evaluate a Gaussian density or CDF overlay across `x_domain = (x_min, x_max)` scaled to `output_mode`.
///
/// Returns `None` when `sd <= 0`, `n == 0`, `bin_width <= 0`, or any parameter is non-finite.
#[must_use]
pub fn evaluate_gaussian_overlay(
    mean: f64,
    sd: f64,
    n: usize,
    bin_width: f64,
    output_mode: FrequencyOutputMode,
    x_domain: (f64, f64),
    num_points: usize,
) -> Option<Vec<(f64, f64)>> {
    let (x_min, x_max) = x_domain;
    if n == 0
        || num_points < 2
        || !mean.is_finite()
        || !sd.is_finite()
        || sd <= 0.0
        || !bin_width.is_finite()
        || bin_width <= 0.0
        || !x_min.is_finite()
        || !x_max.is_finite()
        || x_max <= x_min
    {
        return None;
    }
    let nf = n as f64;
    let span = x_max - x_min;
    let denom = (num_points - 1) as f64;
    let mut pts = Vec::with_capacity(num_points);
    for i in 0..num_points {
        let frac = (i as f64) / denom;
        let x = x_min + frac * span;
        let z = (x - mean) / sd;
        let y = match output_mode {
            FrequencyOutputMode::Counts => nf * bin_width * (norm_pdf(z) / sd),
            FrequencyOutputMode::RelativeFrequency => bin_width * (norm_pdf(z) / sd),
            FrequencyOutputMode::Percentages => 100.0 * bin_width * (norm_pdf(z) / sd),
            FrequencyOutputMode::Density => norm_pdf(z) / sd,
            FrequencyOutputMode::CumulativeCounts => nf * norm_cdf(z),
            FrequencyOutputMode::CumulativeFrequency => norm_cdf(z),
            FrequencyOutputMode::CumulativePercentages => 100.0 * norm_cdf(z),
        };
        if y.is_finite() {
            pts.push((x, y));
        }
    }
    if pts.len() >= 2 { Some(pts) } else { None }
}

fn validate_spec_parameters(spec: &FrequencyDistributionSpec) -> Result<(), StatsError> {
    if let FrequencyBinningRule::ExplicitWidth { width } = spec.binning_rule
        && (!width.is_finite() || width <= 0.0)
    {
        return Err(StatsError::InvalidParameter { name: "binning_rule.width", value: width });
    }
    if let Some(c0) = spec.first_bin_center
        && !c0.is_finite()
    {
        return Err(StatsError::InvalidParameter { name: "first_bin_center", value: c0 });
    }
    if let Some(c1) = spec.last_bin_center
        && !c1.is_finite()
    {
        return Err(StatsError::InvalidParameter { name: "last_bin_center", value: c1 });
    }
    if let (Some(c0), Some(c1)) = (spec.first_bin_center, spec.last_bin_center)
        && c0 > c1
    {
        return Err(StatsError::InvalidParameter { name: "first_bin_center", value: c0 });
    }
    Ok(())
}

fn build_explicit_edges(
    data_min: f64,
    data_max: f64,
    width: f64,
    first_bin_center: Option<f64>,
    last_bin_center: Option<f64>,
    param_name: &'static str,
) -> Result<(Vec<f64>, f64), StatsError> {
    if !width.is_finite() || width <= 0.0 {
        return Err(StatsError::InvalidParameter { name: param_name, value: width });
    }
    let ulp_tol_low = 1e-9 * width.max(data_min.abs()).max(1.0);
    let ulp_tol_high = 1e-9 * width.max(data_max.abs()).max(1.0);
    let (edge0, k_f, exact_upper, cover_low, cover_high) = match (first_bin_center, last_bin_center) {
        (Some(c0), Some(c1)) => {
            let e0 = c0 - 0.5 * width;
            let steps = ((c1 - c0) / width + 1e-12).floor().max(0.0);
            let k = steps + 1.0;
            let exact_end = if ((c1 - c0) / width - steps).abs() <= 1e-9 { Some(c1 + 0.5 * width) } else { None };
            (e0, k, exact_end, false, false)
        }
        (Some(c0), None) => {
            let e0 = c0 - 0.5 * width;
            let mut k = if data_max <= e0 + width { 1.0 } else { ((data_max - e0) / width).ceil().max(1.0) };
            if data_max > e0 && e0 + k * width + ulp_tol_high < data_max {
                k += 1.0;
            }
            (e0, k, None, false, data_max >= e0)
        }
        (None, Some(c1)) => {
            let e_end = c1 + 0.5 * width;
            let mut k = if data_min >= e_end - width { 1.0 } else { ((e_end - data_min) / width).ceil().max(1.0) };
            let mut e0 = e_end - k * width;
            if data_min <= e_end {
                if e0 > data_min + ulp_tol_low {
                    k += 1.0;
                    e0 = e_end - k * width;
                }
                if e0 > data_min && e0 - data_min <= ulp_tol_low {
                    e0 = data_min;
                }
            }
            (e0, k, Some(e_end), data_min <= e_end, false)
        }
        (None, None) => {
            let mut e0 = (data_min / width).floor() * width;
            let mut e_end = (data_max / width).ceil() * width;
            if !e0.is_finite() || !e_end.is_finite() {
                return Err(StatsError::InvalidParameter { name: param_name, value: width });
            }
            if e0 > data_min {
                if e0 - data_min <= ulp_tol_low {
                    e0 = data_min;
                } else {
                    e0 -= width;
                }
            }
            if e_end < data_max {
                if data_max - e_end <= ulp_tol_high {
                    e_end = data_max;
                } else {
                    e_end += width;
                }
            }
            if e_end <= e0 {
                e_end = e0 + width;
            }
            let mut k = ((e_end - e0) / width).round().max(1.0);
            if e0 + k * width + ulp_tol_high < data_max {
                k += 1.0;
            }
            (e0, k, None, true, true)
        }
    };

    if !edge0.is_finite() || !k_f.is_finite() || k_f < 1.0 || k_f > (MAX_HISTOGRAM_BINS as f64) {
        return Err(StatsError::InvalidParameter { name: param_name, value: width });
    }
    let k = k_f as usize;
    let mut edges = Vec::with_capacity(k + 1);
    for j in 0..=k {
        let e = edge0 + (j as f64) * width;
        if !e.is_finite() {
            return Err(StatsError::InvalidParameter { name: param_name, value: width });
        }
        edges.push(e);
    }
    if let Some(e_end) = exact_upper
        && e_end.is_finite()
        && let Some(last) = edges.last_mut()
    {
        *last = e_end;
    }
    if cover_low
        && let Some(first) = edges.first_mut()
        && *first > data_min
    {
        *first = data_min;
    }
    if cover_high
        && let Some(last) = edges.last_mut()
        && *last < data_max
    {
        *last = data_max;
    }
    if !edges.windows(2).all(|w| w[0] < w[1]) {
        return Err(StatsError::InvalidParameter { name: param_name, value: width });
    }
    Ok((edges, width))
}

fn resolve_bin_edges(all_values: &[f64], spec: &FrequencyDistributionSpec) -> Result<(Vec<f64>, f64, Option<String>), StatsError> {
    validate_spec_parameters(spec)?;
    validate_finite_slice(all_values)?;

    let sorted = sorted_finite(all_values);
    let n = sorted.len();
    let data_min = sorted[0];
    let data_max = sorted[n - 1];
    let range = data_max - data_min;
    if !range.is_finite() {
        return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "data_range", value: range }));
    }

    if let FrequencyBinningRule::ExplicitWidth { width } = spec.binning_rule {
        let (edges, w) = build_explicit_edges(data_min, data_max, width, spec.first_bin_center, spec.last_bin_center, "binning_rule.width")?;
        return Ok((edges, w, None));
    }

    // Automatic binning rule.
    if n == 1 || range == 0.0 {
        let note = String::from("All active observations are identical (or n = 1); using a single unit-width bin centered on the data value.");
        if spec.first_bin_center.is_some() || spec.last_bin_center.is_some() {
            let (edges, w) = build_explicit_edges(data_min, data_max, 1.0, spec.first_bin_center, spec.last_bin_center, "first_bin_center")?;
            return Ok((edges, w, Some(note)));
        }
        let e0 = data_min - 0.5;
        let e1 = data_min + 0.5;
        if !e0.is_finite() || !e1.is_finite() || e1 <= e0 {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "constant_bin_edges", value: data_min }));
        }
        return Ok((vec![e0, e1], e1 - e0, Some(note)));
    }

    let moments = SummaryMoments::from_slice(&sorted)?;
    let sd = moments.sample_sd().ok();
    let q1 = quantile_sorted(&sorted, 0.25).unwrap_or(data_min);
    let q3 = quantile_sorted(&sorted, 0.75).unwrap_or(data_max);
    let iqr = (q3 - q1).max(0.0);

    let (raw_k, note) = match spec.binning_rule {
        FrequencyBinningRule::AutoSturges => (nclass_sturges(n), None),
        FrequencyBinningRule::AutoScott => match sd.and_then(|s| nclass_scott(n, s, range)) {
            Some(k) => (k, None),
            None => (nclass_sturges(n), Some(String::from("Sample SD unavailable for Scott's rule; fell back to Sturges' rule."))),
        },
        FrequencyBinningRule::AutoFreedmanDiaconis => match nclass_freedman_diaconis(n, iqr, range) {
            Some(k) => (k, None),
            None => match sd.and_then(|s| nclass_scott(n, s, range)) {
                Some(k) => (k, Some(String::from("Interquartile range is zero; fell back from Freedman–Diaconis to Scott's rule."))),
                None => (nclass_sturges(n), Some(String::from("Interquartile range is zero; fell back from Freedman–Diaconis to Sturges' rule."))),
            },
        },
        FrequencyBinningRule::ExplicitWidth { .. } => (1, None),
    };

    let (k, cap_note) = if raw_k > MAX_HISTOGRAM_BINS {
        (MAX_HISTOGRAM_BINS, Some(format!("Automatic bin count ({raw_k}) exceeded the safety ceiling; capped at {MAX_HISTOGRAM_BINS} bins.")))
    } else {
        (raw_k.max(1), None)
    };
    let effective_note = match (note, cap_note) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };

    let auto_width = range / (k as f64);
    if !auto_width.is_finite() || auto_width <= 0.0 {
        return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "bin_width", value: auto_width }));
    }

    if spec.first_bin_center.is_some() || spec.last_bin_center.is_some() {
        let (edges, w) = build_explicit_edges(data_min, data_max, auto_width, spec.first_bin_center, spec.last_bin_center, "first_bin_center")?;
        return Ok((edges, w, effective_note));
    }

    let mut edges = Vec::with_capacity(k + 1);
    for j in 0..k {
        edges.push(data_min + (j as f64) * auto_width);
    }
    edges.push(data_max);
    Ok((edges, auto_width, effective_note))
}

fn bin_series_against_edges(
    data_set_index: usize,
    label: String,
    values: &[f64],
    excluded_count: usize,
    edges: &[f64],
    bin_width: f64,
    spec: &FrequencyDistributionSpec,
) -> Result<SeriesHistogramResult, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    let k = edges.len().saturating_sub(1);
    if k == 0 {
        return Err(StatsError::InvalidParameter { name: "bin_count", value: 0.0 });
    }

    let e_first = edges[0];
    let e_second = edges[1];
    let e_last = edges[k];

    let mut counts = vec![0_usize; k];
    let mut underflow_count = 0_usize;
    let mut overflow_count = 0_usize;

    for &x in values {
        if x < e_first {
            underflow_count += 1;
        } else if x > e_last {
            overflow_count += 1;
        } else if x <= e_second {
            counts[0] += 1;
        } else {
            let idx = edges.partition_point(|&e| e < x);
            let bin_idx = idx.saturating_sub(1).min(k - 1);
            counts[bin_idx] += 1;
        }
    }

    let moments = SummaryMoments::from_slice(values)?;
    let mean = moments.mean;
    let sd = if n >= 2 { moments.sample_sd().ok().filter(|s| s.is_finite()) } else { None };
    let sorted = sorted_finite(values);
    let q1 = quantile_sorted(&sorted, 0.25).unwrap_or(sorted[0]);
    let q3 = quantile_sorted(&sorted, 0.75).unwrap_or(sorted[n - 1]);
    let iqr = (q3 - q1).max(0.0);

    let nf = n as f64;
    let mut bins = Vec::with_capacity(k);
    let mut running_cum = underflow_count;
    for j in 0..k {
        let edge_low = edges[j];
        let edge_high = edges[j + 1];
        let w = edge_high - edge_low;
        let center = 0.5 * (edge_low + edge_high);
        let c = counts[j];
        running_cum += c;
        let rel = (c as f64) / nf;
        let pct = 100.0 * rel;
        let dens = if w > 0.0 { (c as f64) / (nf * w) } else { 0.0 };
        let cum_freq = (running_cum as f64) / nf;
        let cum_pct = 100.0 * cum_freq;
        bins.push(HistogramBin {
            edge_low,
            edge_high,
            center,
            width: w,
            count: c,
            relative_frequency: rel,
            percentage: pct,
            density: dens,
            cumulative_count: running_cum,
            cumulative_frequency: cum_freq,
            cumulative_percentage: cum_pct,
        });
    }

    let gaussian_overlay = if spec.overlay_gaussian {
        sd.and_then(|s| evaluate_gaussian_overlay(mean, s, n, bin_width, spec.output_mode, (e_first, e_last), GAUSSIAN_OVERLAY_POINTS))
    } else {
        None
    };

    Ok(SeriesHistogramResult { data_set_index, label, n, excluded_count, underflow_count, overflow_count, mean, sd, iqr, bins, gaussian_overlay })
}

/// Compute a single-series histogram from a raw finite slice using `spec`.
pub fn compute_histogram(values: &[f64], spec: &FrequencyDistributionSpec) -> Result<FrequencyDistributionResult, StatsError> {
    compute_frequency_distribution(&[(0, "Data", values, 0)], spec)
}

/// Compute a multi-series frequency distribution sharing common uniform bin edges across all
/// labeled input series `(data_set_index, label, values, excluded_count)`.
pub fn compute_frequency_distribution(
    series: &[(usize, &str, &[f64], usize)],
    spec: &FrequencyDistributionSpec,
) -> Result<FrequencyDistributionResult, StatsError> {
    if series.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let total_len: usize = series.iter().map(|(_, _, vals, _)| vals.len()).sum();
    if total_len == 0 {
        return Err(StatsError::EmptyInput);
    }
    let mut pooled = Vec::with_capacity(total_len);
    for (_, _, vals, _) in series {
        validate_finite_slice(vals)?;
        pooled.extend_from_slice(vals);
    }

    let (edges, bin_width, effective_rule_note) = resolve_bin_edges(&pooled, spec)?;
    let centers: Vec<f64> = edges.windows(2).map(|w| 0.5 * (w[0] + w[1])).collect();

    let mut out_series = Vec::with_capacity(series.len());
    for &(idx, label, vals, excluded) in series {
        out_series.push(bin_series_against_edges(idx, label.to_owned(), vals, excluded, &edges, bin_width, spec)?);
    }

    Ok(FrequencyDistributionResult {
        binning_rule: spec.binning_rule,
        effective_rule_note,
        bin_width,
        edges,
        centers,
        output_mode: spec.output_mode,
        series: out_series,
    })
}

fn format_rule_description(rule: FrequencyBinningRule) -> String {
    match rule {
        FrequencyBinningRule::AutoSturges => String::from("Sturges (1926)"),
        FrequencyBinningRule::AutoScott => String::from("Scott (1979)"),
        FrequencyBinningRule::AutoFreedmanDiaconis => String::from("Freedman–Diaconis (1981)"),
        FrequencyBinningRule::ExplicitWidth { width } => format!("Explicit width ({width})"),
    }
}

fn build_analyzable_xy_sheet(spec: &AnalysisSpec, dist: &FrequencyDistributionResult) -> Result<DataSheet, StatsError> {
    let k = dist.centers.len();
    let ds_count = dist.series.len();
    let mut xy = XyTable::new(SubcolumnMode::Single, ds_count, k)?;
    xy.x_title = String::from("Bin center");
    for (s_idx, s) in dist.series.iter().enumerate() {
        xy.data_sets[s_idx].title = s.label.clone();
    }
    for (r_idx, &center) in dist.centers.iter().enumerate() {
        xy.set_x_cell(r_idx, Cell::number(center))?;
    }
    let sheet_id = format!("{}_bins", spec.id.as_str());
    let mut ds = DataSheet::new(sheet_id, format!("{} (binned)", spec.effective_title()), TableArchetype::Xy(xy))?;
    for (r_idx, _) in dist.centers.iter().enumerate() {
        for (s_idx, s) in dist.series.iter().enumerate() {
            let v = s.bins[r_idx].value_for_mode(dist.output_mode);
            ds.set_cell(CellCoord::new(r_idx, s_idx, 0), Cell::number(v))?;
        }
    }
    Ok(ds)
}

fn compute_sheet_frequency_distribution_with_ignored(
    sheet: &DataSheet,
    selected_data_sets: &[usize],
    fd: &FrequencyDistributionSpec,
) -> Result<(FrequencyDistributionResult, Vec<(String, usize)>), StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::FrequencyDistribution(fd.clone()))?;
    if !matches!(
        sheet.archetype,
        TableArchetype::Column(_) | TableArchetype::Xy(_) | TableArchetype::Grouped(_) | TableArchetype::MultipleVariables(_)
    ) {
        return Err(StatsError::IncompatibleTable { analysis: "frequency distribution", actual: sheet.archetype.kind() });
    }

    let extracted = extract_indexed_series_for_descriptive(sheet, selected_data_sets, fd.aggregation)?;
    if extracted.raw.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let inputs: Vec<(usize, &str, &[f64], usize)> =
        extracted.raw.iter().map(|s| (s.data_set_index, s.label.as_str(), s.values.as_slice(), s.excluded_count)).collect();
    let dist = compute_frequency_distribution(&inputs, fd)?;
    Ok((dist, extracted.ignored_non_numeric))
}

/// Extract raw indexed series from `sheet` (preserving each non-empty column's true
/// `data_set_index` even when earlier columns are empty) and compute a shared-edge frequency
/// distribution using `fd`.
pub fn compute_sheet_frequency_distribution(
    sheet: &DataSheet,
    selected_data_sets: &[usize],
    fd: &FrequencyDistributionSpec,
) -> Result<FrequencyDistributionResult, StatsError> {
    compute_sheet_frequency_distribution_with_ignored(sheet, selected_data_sets, fd).map(|(dist, _)| dist)
}

/// Execute a standalone [`FrequencyDistributionSpec`] against a raw `Column`, `Xy`, `Grouped`, or
/// `MultipleVariables` [`DataSheet`] and emit a complete [`ResultSheet`].
pub fn run_frequency_distribution_analysis(
    sheet: &DataSheet,
    spec: &AnalysisSpec,
    fd: &FrequencyDistributionSpec,
) -> Result<ResultSheet, StatsError> {
    let (dist, ignored_non_numeric) = compute_sheet_frequency_distribution_with_ignored(sheet, &spec.selected_data_sets, fd)?;

    let mut res = init_result_sheet(spec)?;
    let k = dist.centers.len();
    let rule_desc = format_rule_description(fd.binning_rule);

    // Tab 1: Primary frequency distribution table at the configured output scale.
    let mut main_headers = vec![String::from("Bin lower"), String::from("Bin upper"), String::from("Bin center")];
    if dist.series.len() == 1 {
        main_headers.push(fd.output_mode.label().to_owned());
    } else {
        for s in &dist.series {
            main_headers.push(format!("{} ({})", s.label, fd.output_mode.label()));
        }
    }
    let mut main_tab = TabBuilder::new("freq-distribution", "Frequency distribution", ResultTabKind::MainSummary).headers(main_headers);
    for j in 0..k {
        let b0 = &dist.series[0].bins[j];
        let mut row = Vec::with_capacity(3 + dist.series.len());
        row.push(num_cell(b0.edge_low));
        row.push(num_cell(b0.edge_high));
        row.push(num_cell(b0.center));
        for s in &dist.series {
            row.push(num_cell(s.bins[j].value_for_mode(fd.output_mode)));
        }
        main_tab.push_row(format!("Bin {}", j + 1), row);
    }
    main_tab.push_footnote(format!(
        "Binning rule: {rule_desc}; {k} uniform bins of width {:.6}; right-closed intervals (e_j, e_{{j+1}}] with leftmost bin [e_0, e_1].",
        dist.bin_width
    ));
    res.tabs.push(main_tab.build());

    // Tab 2: Detailed multi-scale table (counts, relative frequency, density, cumulative frequency).
    let mut detail_headers = vec![String::from("Bin lower"), String::from("Bin upper"), String::from("Bin center")];
    for s in &dist.series {
        let prefix = if dist.series.len() == 1 { String::new() } else { format!("{}: ", s.label) };
        detail_headers.push(format!("{prefix}Count"));
        detail_headers.push(format!("{prefix}Relative frequency"));
        detail_headers.push(format!("{prefix}Percentage (%)"));
        detail_headers.push(format!("{prefix}Density"));
        detail_headers.push(format!("{prefix}Cumulative count"));
        detail_headers.push(format!("{prefix}Cumulative frequency"));
        detail_headers.push(format!("{prefix}Cumulative (%)"));
    }
    let mut detail_tab =
        TabBuilder::new("freq-detail", "Bin counts, densities & cumulative distribution", ResultTabKind::DescriptiveSummary).headers(detail_headers);
    for j in 0..k {
        let b0 = &dist.series[0].bins[j];
        let mut row = Vec::with_capacity(3 + 7 * dist.series.len());
        row.push(num_cell(b0.edge_low));
        row.push(num_cell(b0.edge_high));
        row.push(num_cell(b0.center));
        for s in &dist.series {
            let b = &s.bins[j];
            row.push(num_cell(b.count as f64));
            row.push(num_cell(b.relative_frequency));
            row.push(num_cell(b.percentage));
            row.push(num_cell(b.density));
            row.push(num_cell(b.cumulative_count as f64));
            row.push(num_cell(b.cumulative_frequency));
            row.push(num_cell(b.cumulative_percentage));
        }
        detail_tab.push_row(format!("Bin {}", j + 1), row);
    }
    res.tabs.push(detail_tab.build());

    // Tab 3: Per-series summary moments & bin accounting.
    let mut summary_tab = TabBuilder::new("freq-summary", "Distribution summary", ResultTabKind::DescriptiveSummary).headers([
        "N",
        "Excluded",
        "Below first bin",
        "Above last bin",
        "Bins",
        "Bin width",
        "Mean",
        "SD",
        "IQR",
    ]);
    for s in &dist.series {
        summary_tab.push_row(
            s.label.clone(),
            [
                num_cell(s.n as f64),
                num_cell(s.excluded_count as f64),
                num_cell(s.underflow_count as f64),
                num_cell(s.overflow_count as f64),
                num_cell(k as f64),
                num_cell(dist.bin_width),
                num_cell(s.mean),
                opt_num_cell(s.sd),
                num_cell(s.iqr),
            ],
        );
    }
    res.tabs.push(summary_tab.build());

    // Key hookable scalars.
    push_scalar(&mut res, "Bin count", None, k as f64);
    push_scalar(&mut res, "Bin width", None, dist.bin_width);
    if let (Some(&c_first), Some(&c_last)) = (dist.centers.first(), dist.centers.last()) {
        push_scalar(&mut res, "First bin center", None, c_first);
        push_scalar(&mut res, "Last bin center", None, c_last);
    }
    if let (Some(&e_first), Some(&e_last)) = (dist.edges.first(), dist.edges.last()) {
        push_scalar(&mut res, "First bin edge", None, e_first);
        push_scalar(&mut res, "Last bin edge", None, e_last);
    }
    for s in &dist.series {
        let dsi = Some(s.data_set_index);
        push_scalar(&mut res, "N", dsi, s.n as f64);
        push_scalar(&mut res, "Mean", dsi, s.mean);
        if let Some(sd_val) = s.sd {
            push_scalar(&mut res, "SD", dsi, sd_val);
        }
        push_scalar(&mut res, "IQR", dsi, s.iqr);
        push_scalar(&mut res, "Underflow", dsi, s.underflow_count as f64);
        push_scalar(&mut res, "Overflow", dsi, s.overflow_count as f64);
    }

    // Optional fitted Gaussian overlay curves.
    let eq_id = if fd.output_mode.is_cumulative() { "histogram_gaussian_cdf" } else { "histogram_gaussian_pdf" };
    let mut skipped_gaussian = 0_usize;
    for s in &dist.series {
        if let Some(pts) = &s.gaussian_overlay {
            res.fitted_curves.push(FittedCurveOverlay {
                data_set_index: s.data_set_index,
                equation_id: String::from(eq_id),
                converged: true,
                ambiguous: false,
                points: pts
                    .iter()
                    .map(|&(x, y)| CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None })
                    .collect(),
            });
        } else if fd.overlay_gaussian {
            skipped_gaussian += 1;
        }
    }

    // Analyzable XY table (X = bin centers, Y = active output values) for downstream fits/graphing.
    res.analyzable_data = Some(build_analyzable_xy_sheet(spec, &dist)?);

    // Diagnostic checklist.
    if let Some(note) = &dist.effective_rule_note {
        res.checklist.push(checklist_item(
            "Binning rule",
            ChecklistStatus::Warn,
            Some(format!("{k} bins, width = {:.4}", dist.bin_width)),
            note.clone(),
        ));
    } else {
        res.checklist.push(checklist_item(
            "Binning rule",
            ChecklistStatus::Pass,
            Some(format!("{rule_desc}: {k} bins, width = {:.4}", dist.bin_width)),
            format!("Partitioned {} series into {k} uniform bins ({}).", dist.series.len(), fd.output_mode.label()),
        ));
    }

    let total_underflow: usize = dist.series.iter().map(|s| s.underflow_count).sum();
    let total_overflow: usize = dist.series.iter().map(|s| s.overflow_count).sum();
    if total_underflow > 0 || total_overflow > 0 {
        res.checklist.push(checklist_item(
            "Bin range coverage",
            ChecklistStatus::Warn,
            Some(format!("{total_underflow} below, {total_overflow} above")),
            "Some active observations fell outside the explicit first/last bin boundaries; cumulative counts still include values below the first bin.",
        ));
    }

    if fd.overlay_gaussian && skipped_gaussian > 0 {
        res.checklist.push(checklist_item(
            "Gaussian density overlay",
            ChecklistStatus::Warn,
            Some(format!("{skipped_gaussian} series omitted")),
            "Gaussian overlay requires n >= 2 and positive sample standard deviation.",
        ));
    }

    let excluded_total: usize = dist.series.iter().map(|s| s.excluded_count).sum();
    if excluded_total > 0 {
        res.checklist.push(checklist_item(
            "Excluded cells",
            ChecklistStatus::Info,
            Some(format!("{excluded_total} excluded")),
            "Excluded cells were omitted from histogram binning and moment calculations.",
        ));
    }

    append_ignored_text_checklist_and_footnote(&mut res, "Frequency distribution", &ignored_non_numeric);
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}
