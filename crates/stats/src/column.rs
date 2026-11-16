//! Column descriptive statistics, exact binomial median CIs, normality & lognormality tests,
//! outlier identification (Grubbs & ROUT), and one-sample hypothesis tests.

use lumencraft_doc::{
    AnalysisSpec, ChecklistItem, ChecklistStatus, DataSheet, DescriptiveSpec, OutlierSpec, PValueFormat, ResultCell, ResultSheet, ResultTab,
    ResultTabKind, SubcolumnAggregation, SummarizedPoint, TableArchetype, VariableRole,
};
use lumencraft_math::dist::{
    chi2_isf, chi2_ppf, chi2_sf, lilliefors_d_from_sorted, lilliefors_p_value, norm_cdf, norm_ppf, norm_sf, student_t_cdf, student_t_sf,
    student_t_two_sided_p,
};
use lumencraft_math::noncentral_t_ci;
use lumencraft_math::special::{gamma::gamma, inc_beta::reg_beta};
use lumencraft_math::sum::{NeumaierSum, SummaryMoments, lag1_autocorrelation};
use lumencraft_math::{ConfidenceLevel, MathError};
use serde::{Deserialize, Serialize};

use crate::common::{
    append_ignored_text_checklist_and_footnote, build_tab, checklist_item, init_result_sheet, is_ignored_non_numeric_cell, num_cell, opt_ci_cell,
    opt_num_cell, p_cell, push_scalar, quantile_sorted, quantile_type6_sorted, quartiles_hinge, rank_with_ties, resolve_selected_indices,
    sanitize_result_sheet_floats, select_kth_upper_triangle_half_sum, sorted_finite, t_crit_two_sided, validate_finite_slice,
};
use crate::error::StatsError;

// ============================================================================
// Exact Binomial Median Confidence Interval
// ============================================================================

/// Nonparametric exact order-statistic confidence interval for the median.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MedianCi {
    pub lower: f64,
    pub upper: f64,
    pub lower_rank_1based: usize,
    pub upper_rank_1based: usize,
    pub target_confidence_percent: f64,
    pub actual_confidence_percent: f64,
}

/// Exact binomial `P(Bin(n, 0.5) <= k) = I_{0.5}(n - k, k + 1)`.
fn binomial_half_cdf(k: usize, n: usize) -> Result<f64, StatsError> {
    if k >= n {
        return Ok(1.0);
    }
    let val = reg_beta(0.5, (n - k) as f64, (k + 1) as f64)?;
    Ok(val.clamp(0.0, 1.0))
}

/// Compute the exact nonparametric binomial confidence interval for the median from a finite slice.
pub fn exact_binomial_median_ci(values: &[f64], confidence: ConfidenceLevel) -> Result<MedianCi, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "exact binomial median CI", required: 3, actual: n });
    }
    let sorted = sorted_finite(values);
    let target = confidence.percent() / 100.0;

    let half_n = n / 2;
    let mut best_r = 0_usize;
    let mut best_cov = (1.0 - 2.0 * binomial_half_cdf(0, n)?).clamp(0.0, 1.0);

    for r in 1..half_n {
        let cov = (1.0 - 2.0 * binomial_half_cdf(r, n)?).clamp(0.0, 1.0);
        if cov >= target {
            best_r = r;
            best_cov = cov;
        } else {
            break;
        }
    }

    let upper_idx = n - 1 - best_r;
    Ok(MedianCi {
        lower: sorted[best_r],
        upper: sorted[upper_idx],
        lower_rank_1based: best_r + 1,
        upper_rank_1based: upper_idx + 1,
        target_confidence_percent: confidence.percent(),
        actual_confidence_percent: 100.0 * best_cov,
    })
}

// ============================================================================
// Descriptive Statistics
// ============================================================================

/// Complete descriptive summary for a single numeric column or aggregated subcolumn series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DescriptiveSummary {
    pub label: String,
    pub n: usize,
    pub excluded_count: usize,
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
    pub iqr: f64,
    pub range: f64,
    pub custom_lower_percentile: Option<(f64, f64)>,
    pub custom_upper_percentile: Option<(f64, f64)>,
    pub sum: f64,
    pub mean: f64,
    pub sd: Option<f64>,
    #[serde(default)]
    pub sd_ci: Option<(f64, f64)>,
    pub sem: Option<f64>,
    pub mean_ci: Option<(f64, f64)>,
    pub median_ci: Option<MedianCi>,
    pub cv_percent: Option<f64>,
    pub geometric_mean: Option<f64>,
    pub geometric_sd_factor: Option<f64>,
    pub geometric_mean_ci: Option<(f64, f64)>,
    pub harmonic_mean: Option<f64>,
    pub harmonic_mean_ci: Option<(f64, f64)>,
    pub quadratic_mean: f64,
    pub quadratic_mean_ci: Option<(f64, f64)>,
    pub skewness: Option<f64>,
    pub excess_kurtosis: Option<f64>,
    pub lag1_autocorrelation: Option<f64>,
}

/// Compute comprehensive descriptive statistics on raw finite observations.
pub fn summarize_column(
    label: impl Into<String>,
    values: &[f64],
    excluded_count: usize,
    confidence: ConfidenceLevel,
    custom_upper_percentile: Option<f64>,
) -> Result<DescriptiveSummary, StatsError> {
    validate_finite_slice(values)?;
    let label = label.into();
    let n = values.len();
    let sorted = sorted_finite(values);
    let min = sorted[0];
    let max = sorted[n - 1];
    let range = max - min;
    let (q1, median, q3) = quartiles_hinge(&sorted).ok_or(StatsError::EmptyInput)?;
    let iqr = q3 - q1;

    let (custom_lower_percentile, custom_upper_percentile) = compute_custom_percentiles(&sorted, custom_upper_percentile)?;

    let moments = SummaryMoments::from_slice(values)?;
    let sd = moments.sample_sd().ok();
    let sd_ci = compute_sd_ci(sd, n, confidence)?;
    let sem = moments.sem().ok();
    let mean_ci = compute_mean_ci(moments.mean, sem, n, confidence)?;
    let median_ci = exact_binomial_median_ci(values, confidence).ok();

    let cv_percent = match sd {
        Some(s) if moments.mean != 0.0 => Some(100.0 * s / moments.mean.abs()),
        _ => None,
    };

    let (geometric_mean, geometric_sd_factor, geometric_mean_ci) = compute_geometric_stats(values, confidence)?;
    let (harmonic_mean, harmonic_mean_ci) = compute_harmonic_stats(values, confidence)?;
    let (quadratic_mean, quadratic_mean_ci) = compute_quadratic_stats(values, confidence)?;
    let (skewness, excess_kurtosis) = compute_skewness_kurtosis(values, moments.mean, sd);
    let lag1 = lag1_autocorrelation(values).ok();

    Ok(DescriptiveSummary {
        label,
        n,
        excluded_count,
        min,
        q1,
        median,
        q3,
        max,
        iqr,
        range,
        custom_lower_percentile,
        custom_upper_percentile,
        sum: moments.sum,
        mean: moments.mean,
        sd,
        sd_ci,
        sem,
        mean_ci,
        median_ci,
        cv_percent,
        geometric_mean,
        geometric_sd_factor,
        geometric_mean_ci,
        harmonic_mean,
        harmonic_mean_ci,
        quadratic_mean,
        quadratic_mean_ci,
        skewness,
        excess_kurtosis,
        lag1_autocorrelation: lag1,
    })
}

type OptionalCiPair = (Option<(f64, f64)>, Option<(f64, f64)>);
type OptionalMeanCi = (Option<f64>, Option<(f64, f64)>);
type ExtractedSeriesPair = (Vec<(String, Vec<f64>, usize)>, Vec<(String, Vec<SummarizedPoint>)>);

struct IndexedRawSeries {
    data_set_index: usize,
    label: String,
    values: Vec<f64>,
    excluded_count: usize,
    non_numeric_count: usize,
    /// Original 0-based table row index for each active observation in `values`.
    row_indices: Vec<usize>,
}

struct IndexedPooledSeries {
    data_set_index: usize,
    label: String,
    points: Vec<SummarizedPoint>,
}

struct DescriptiveSeriesExtraction {
    raw: Vec<IndexedRawSeries>,
    pooled: Vec<IndexedPooledSeries>,
    ignored_non_numeric: Vec<(String, usize)>,
}

fn compute_custom_percentiles(sorted: &[f64], custom_upper: Option<f64>) -> Result<OptionalCiPair, StatsError> {
    let Some(p_hi) = custom_upper else {
        return Ok((None, None));
    };
    if !p_hi.is_finite() || !(50.0..100.0).contains(&p_hi) {
        return Err(StatsError::InvalidParameter { name: "custom_upper_percentile", value: p_hi });
    }
    let p_lo = 100.0 - p_hi;
    let v_lo = quantile_sorted(sorted, p_lo / 100.0).map(|v| (p_lo, v));
    let v_hi = quantile_sorted(sorted, p_hi / 100.0).map(|v| (p_hi, v));
    Ok((v_lo, v_hi))
}

fn compute_mean_ci(mean: f64, sem: Option<f64>, n: usize, confidence: ConfidenceLevel) -> Result<Option<(f64, f64)>, StatsError> {
    if n < 2 {
        return Ok(None);
    }
    let Some(se) = sem else {
        return Ok(None);
    };
    let t_crit = t_crit_two_sided(confidence, (n - 1) as f64)?;
    let half = t_crit * se;
    Ok(Some((mean - half, mean + half)))
}

fn compute_sd_ci(sd: Option<f64>, n: usize, confidence: ConfidenceLevel) -> Result<Option<(f64, f64)>, StatsError> {
    if n < 2 {
        return Ok(None);
    }
    compute_sd_ci_df(sd, (n - 1) as f64, confidence)
}

fn compute_sd_ci_df(sd: Option<f64>, df: f64, confidence: ConfidenceLevel) -> Result<Option<(f64, f64)>, StatsError> {
    if !df.is_finite() || df < 1.0 {
        return Ok(None);
    }
    let Some(s) = sd else {
        return Ok(None);
    };
    let half_alpha = 0.5 * confidence.alpha();
    let upper_chi2 = chi2_isf(half_alpha, df)?;
    let lower_chi2 = chi2_ppf(half_alpha, df)?;
    if upper_chi2 > 0.0 && lower_chi2 > 0.0 { Ok(Some((s * (df / upper_chi2).sqrt(), s * (df / lower_chi2).sqrt()))) } else { Ok(None) }
}

type GeomStats = (Option<f64>, Option<f64>, Option<(f64, f64)>);

fn compute_geometric_stats(values: &[f64], confidence: ConfidenceLevel) -> Result<GeomStats, StatsError> {
    if values.iter().any(|&v| v <= 0.0) {
        return Ok((None, None, None));
    }
    let logs: Vec<f64> = values.iter().map(|v| v.ln()).collect();
    let lm = SummaryMoments::from_slice(&logs)?;
    let gm = lm.mean.exp();
    let gsd = lm.sample_sd().ok().map(f64::exp);
    let g_ci =
        if let Some((lo, hi)) = compute_mean_ci(lm.mean, lm.sem().ok(), values.len(), confidence)? { Some((lo.exp(), hi.exp())) } else { None };
    Ok((Some(gm), gsd, g_ci))
}

fn compute_harmonic_stats(values: &[f64], confidence: ConfidenceLevel) -> Result<OptionalMeanCi, StatsError> {
    if values.iter().any(|&v| v <= 0.0) {
        return Ok((None, None));
    }
    let min_v = values.iter().copied().fold(f64::INFINITY, f64::min);
    let invs: Vec<f64> = values.iter().map(|v| min_v / v).collect();
    let im = SummaryMoments::from_slice(&invs)?;
    let hm = (min_v / im.mean).is_finite().then_some(min_v / im.mean);
    let ci = if let Some((lo_inv, hi_inv)) = compute_mean_ci(im.mean, im.sem().ok(), values.len(), confidence)? {
        if lo_inv > 0.0 && hi_inv > 0.0 {
            let lo = min_v / hi_inv;
            let hi = min_v / lo_inv;
            (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
        } else {
            None
        }
    } else {
        None
    };
    Ok((hm, ci))
}

fn compute_quadratic_stats(values: &[f64], confidence: ConfidenceLevel) -> Result<(f64, Option<(f64, f64)>), StatsError> {
    let n = values.len();
    let max_abs = values.iter().fold(0.0_f64, |acc, &v| acc.max(v.abs()));
    if max_abs == 0.0 {
        return Ok((0.0, (n >= 2).then_some((0.0, 0.0))));
    }
    let sqs: Vec<f64> = values.iter().map(|v| (v / max_abs).powi(2)).collect();
    let sm = SummaryMoments::from_slice(&sqs)?;
    let rms = max_abs * sm.mean.max(0.0).sqrt();
    if !rms.is_finite() {
        return Err(StatsError::Math(lumencraft_math::MathError::DomainError { name: "quadratic_mean", value: rms }));
    }
    let ci = if n >= 2 {
        if let Some((lo_sq, hi_sq)) = compute_mean_ci(sm.mean, sm.sem().ok(), n, confidence)? {
            let lo = max_abs * lo_sq.max(0.0).sqrt();
            let hi = max_abs * hi_sq.max(0.0).sqrt();
            (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
        } else {
            None
        }
    } else {
        None
    };
    Ok((rms, ci))
}

fn compute_skewness_kurtosis(values: &[f64], mean: f64, sd: Option<f64>) -> (Option<f64>, Option<f64>) {
    let n = values.len();
    let Some(s) = sd else {
        return (None, None);
    };
    if s == 0.0 || n < 3 {
        return (None, None);
    }
    let mut sum3 = NeumaierSum::new();
    let mut sum4 = NeumaierSum::new();
    for &x in values {
        let z = (x - mean) / s;
        let z2 = z * z;
        sum3.add(z2 * z);
        sum4.add(z2 * z2);
    }
    let nf = n as f64;
    let skew = (nf / ((nf - 1.0) * (nf - 2.0))) * sum3.total();
    let kurt = if n >= 4 {
        let term1 = (nf * (nf + 1.0)) / ((nf - 1.0) * (nf - 2.0) * (nf - 3.0)) * sum4.total();
        let term2 = 3.0 * (nf - 1.0) * (nf - 1.0) / ((nf - 2.0) * (nf - 3.0));
        Some(term1 - term2)
    } else {
        None
    };
    (Some(skew), kurt)
}

/// Pool pre-summarized `(Mean, SD, N)` points into exact combined `(total_n, mean, sd, sem, ci)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PooledSummaryResult {
    pub label: String,
    pub point_count: usize,
    pub total_n: f64,
    pub mean: f64,
    pub sd: Option<f64>,
    #[serde(default)]
    pub sd_ci: Option<(f64, f64)>,
    pub sem: Option<f64>,
    pub mean_ci: Option<(f64, f64)>,
}

/// Reconstruct pooled descriptive moments from pre-summarized points (`MeanSdN`, `MeanSemN`, `MeanCvN`).
pub fn summarize_pre_summarized_points(
    label: impl Into<String>,
    points: &[SummarizedPoint],
    confidence: ConfidenceLevel,
) -> Result<PooledSummaryResult, StatsError> {
    if points.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let mut total_n_acc = NeumaierSum::new();
    let mut weighted_sum_acc = NeumaierSum::new();
    for pt in points {
        total_n_acc.add(pt.n);
        weighted_sum_acc.add(pt.n * pt.mean);
    }
    let total_n = total_n_acc.total();
    if total_n <= 0.0 {
        return Err(StatsError::EmptyInput);
    }
    let grand_mean = weighted_sum_acc.total() / total_n;
    let mut ss_within_acc = NeumaierSum::new();
    let mut ss_between_acc = NeumaierSum::new();
    for pt in points {
        ss_within_acc.add(pt.ss_within);
        let d = pt.mean - grand_mean;
        ss_between_acc.add(pt.n * d * d);
    }
    let ss_total = (ss_within_acc.total() + ss_between_acc.total()).max(0.0);
    let (sd, sd_ci, sem, mean_ci) = if total_n > 1.0 {
        let s = (ss_total / (total_n - 1.0)).sqrt();
        let se = s / total_n.sqrt();
        let t_crit = t_crit_two_sided(confidence, total_n - 1.0)?;
        let s_ci = compute_sd_ci_df(Some(s), total_n - 1.0, confidence)?;
        (Some(s), s_ci, Some(se), Some((grand_mean - t_crit * se, grand_mean + t_crit * se)))
    } else {
        (None, None, None, None)
    };
    Ok(PooledSummaryResult { label: label.into(), point_count: points.len(), total_n, mean: grand_mean, sd, sd_ci, sem, mean_ci })
}

/// Extract series from `DataSheet` under the configured [`SubcolumnAggregation`] mode.
pub fn extract_series_for_descriptive(
    sheet: &DataSheet,
    selected: &[usize],
    aggregation: SubcolumnAggregation,
) -> Result<ExtractedSeriesPair, StatsError> {
    let extracted = extract_indexed_series_for_descriptive(sheet, selected, aggregation)?;
    let raw_out = extracted.raw.into_iter().map(|s| (s.label, s.values, s.excluded_count)).collect();
    let pooled_out = extracted.pooled.into_iter().map(|p| (p.label, p.points)).collect();
    Ok((raw_out, pooled_out))
}

fn collect_ignored_non_numeric(raw: &[IndexedRawSeries]) -> Vec<(String, usize)> {
    raw.iter().filter(|s| s.non_numeric_count > 0).map(|s| (s.label.clone(), s.non_numeric_count)).collect()
}

fn extract_indexed_series_for_descriptive(
    sheet: &DataSheet,
    selected: &[usize],
    aggregation: SubcolumnAggregation,
) -> Result<DescriptiveSeriesExtraction, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(t) => extract_column_table_for_desc(t, selected),
        TableArchetype::Xy(t) => extract_xy_table_for_desc(t, selected, aggregation),
        TableArchetype::Grouped(t) => extract_grouped_table_for_desc(t, selected, aggregation),
        TableArchetype::MultipleVariables(t) => {
            let indices = resolve_selected_indices(selected, t.variables.len());
            let mut raw = Vec::new();
            let mut ignored = Vec::new();
            for idx in indices {
                let var = &t.variables[idx];
                let mut vals = Vec::new();
                let mut row_indices = Vec::new();
                let mut ex = 0_usize;
                let mut non_num = 0_usize;
                for (r_idx, c) in var.cells.iter().enumerate() {
                    if c.excluded {
                        ex += 1;
                    } else if let Some(v) = c.active_number() {
                        vals.push(v);
                        row_indices.push(r_idx);
                    } else if is_ignored_non_numeric_cell(c) {
                        non_num += 1;
                    }
                }
                if non_num > 0 {
                    ignored.push((var.title.clone(), non_num));
                }
                if !vals.is_empty() {
                    raw.push(IndexedRawSeries {
                        data_set_index: idx,
                        label: var.title.clone(),
                        values: vals,
                        excluded_count: ex,
                        non_numeric_count: non_num,
                        row_indices,
                    });
                }
            }
            Ok(DescriptiveSeriesExtraction { raw, pooled: Vec::new(), ignored_non_numeric: ignored })
        }
        other => Err(StatsError::IncompatibleTable { analysis: "descriptive statistics", actual: other.kind() }),
    }
}

fn extract_column_table_for_desc(t: &lumencraft_doc::ColumnTable, selected: &[usize]) -> Result<DescriptiveSeriesExtraction, StatsError> {
    let indices = resolve_selected_indices(selected, t.data_sets.len());
    if t.subcolumn_mode.supports_parametric_moments() {
        let mut pooled = Vec::new();
        for idx in indices {
            let ds = &t.data_sets[idx];
            let mut pts = Vec::new();
            for r in 0..ds.row_count() {
                if let Some(pt) = t.summarized_point(r, idx)? {
                    pts.push(pt);
                }
            }
            if !pts.is_empty() {
                pooled.push(IndexedPooledSeries { data_set_index: idx, label: ds.title.clone(), points: pts });
            }
        }
        return Ok(DescriptiveSeriesExtraction { raw: Vec::new(), pooled, ignored_non_numeric: Vec::new() });
    }
    if t.subcolumn_mode.is_pre_summarized() {
        return Err(StatsError::RequiresRawReplicates);
    }
    let mut raw = Vec::new();
    let mut ignored = Vec::new();
    for idx in indices {
        let ds = &t.data_sets[idx];
        let mut vals = Vec::new();
        let mut row_indices = Vec::new();
        let mut ex = 0_usize;
        let mut non_num = 0_usize;
        for (r_idx, row) in ds.cells.iter().enumerate() {
            if let Some(c) = row.first() {
                if c.excluded {
                    ex += 1;
                } else if let Some(v) = c.active_number() {
                    vals.push(v);
                    row_indices.push(r_idx);
                } else if is_ignored_non_numeric_cell(c) {
                    non_num += 1;
                }
            }
        }
        if non_num > 0 {
            ignored.push((ds.title.clone(), non_num));
        }
        if !vals.is_empty() {
            raw.push(IndexedRawSeries {
                data_set_index: idx,
                label: ds.title.clone(),
                values: vals,
                excluded_count: ex,
                non_numeric_count: non_num,
                row_indices,
            });
        }
    }
    Ok(DescriptiveSeriesExtraction { raw, pooled: Vec::new(), ignored_non_numeric: ignored })
}

fn extract_xy_table_for_desc(
    t: &lumencraft_doc::XyTable,
    selected: &[usize],
    aggregation: SubcolumnAggregation,
) -> Result<DescriptiveSeriesExtraction, StatsError> {
    let indices = resolve_selected_indices(selected, t.data_sets.len());
    if t.y_subcolumn_mode.supports_parametric_moments() {
        let mut pooled = Vec::new();
        for idx in indices {
            let ds = &t.data_sets[idx];
            let mut pts = Vec::new();
            for r in 0..ds.row_count() {
                if let Some(pt) = t.summarized_y_point(r, idx)? {
                    pts.push(pt);
                }
            }
            if !pts.is_empty() {
                pooled.push(IndexedPooledSeries { data_set_index: idx, label: ds.title.clone(), points: pts });
            }
        }
        return Ok(DescriptiveSeriesExtraction { raw: Vec::new(), pooled, ignored_non_numeric: Vec::new() });
    }
    if t.y_subcolumn_mode.is_pre_summarized() {
        return Err(StatsError::RequiresRawReplicates);
    }
    let raw = aggregate_replicate_datasets(&t.data_sets, &indices, aggregation);
    let ignored_non_numeric = collect_ignored_non_numeric(&raw);
    Ok(DescriptiveSeriesExtraction { raw, pooled: Vec::new(), ignored_non_numeric })
}

fn extract_grouped_table_for_desc(
    t: &lumencraft_doc::GroupedTable,
    selected: &[usize],
    aggregation: SubcolumnAggregation,
) -> Result<DescriptiveSeriesExtraction, StatsError> {
    let indices = resolve_selected_indices(selected, t.data_sets.len());
    if t.subcolumn_mode.supports_parametric_moments() {
        let mut pooled = Vec::new();
        for idx in indices {
            let ds = &t.data_sets[idx];
            let mut pts = Vec::new();
            for row in &ds.cells {
                if let Some(pt) = SummarizedPoint::reconstruct(t.subcolumn_mode, row)? {
                    pts.push(pt);
                }
            }
            if !pts.is_empty() {
                pooled.push(IndexedPooledSeries { data_set_index: idx, label: ds.title.clone(), points: pts });
            }
        }
        return Ok(DescriptiveSeriesExtraction { raw: Vec::new(), pooled, ignored_non_numeric: Vec::new() });
    }
    if t.subcolumn_mode.is_pre_summarized() {
        return Err(StatsError::RequiresRawReplicates);
    }
    let raw = aggregate_replicate_datasets(&t.data_sets, &indices, aggregation);
    let ignored_non_numeric = collect_ignored_non_numeric(&raw);
    Ok(DescriptiveSeriesExtraction { raw, pooled: Vec::new(), ignored_non_numeric })
}

fn aggregate_replicate_datasets(
    data_sets: &[lumencraft_doc::DataSetColumn],
    indices: &[usize],
    aggregation: SubcolumnAggregation,
) -> Vec<IndexedRawSeries> {
    let mut out = Vec::new();
    for &idx in indices {
        let ds = &data_sets[idx];
        let subs = ds.subcolumn_count();
        match aggregation {
            SubcolumnAggregation::RowMeans => {
                let mut vals = Vec::new();
                let mut row_indices = Vec::new();
                let mut ex = 0_usize;
                let mut non_num = 0_usize;
                for (r_idx, row) in ds.cells.iter().enumerate() {
                    let mut row_vals = Vec::new();
                    for c in row {
                        if c.excluded {
                            ex += 1;
                        } else if let Some(v) = c.active_number() {
                            row_vals.push(v);
                        } else if is_ignored_non_numeric_cell(c) {
                            non_num += 1;
                        }
                    }
                    if let Ok(m) = SummaryMoments::from_slice(&row_vals) {
                        vals.push(m.mean);
                        row_indices.push(r_idx);
                    }
                }
                if !vals.is_empty() {
                    out.push(IndexedRawSeries {
                        data_set_index: idx,
                        label: ds.title.clone(),
                        values: vals,
                        excluded_count: ex,
                        non_numeric_count: non_num,
                        row_indices,
                    });
                }
            }
            SubcolumnAggregation::PerSubcolumn => {
                for s in 0..subs {
                    let mut vals = Vec::new();
                    let mut row_indices = Vec::new();
                    let mut ex = 0_usize;
                    let mut non_num = 0_usize;
                    for (r_idx, row) in ds.cells.iter().enumerate() {
                        if let Some(c) = row.get(s) {
                            if c.excluded {
                                ex += 1;
                            } else if let Some(v) = c.active_number() {
                                vals.push(v);
                                row_indices.push(r_idx);
                            } else if is_ignored_non_numeric_cell(c) {
                                non_num += 1;
                            }
                        }
                    }
                    if !vals.is_empty() {
                        let lbl = if subs > 1 { format!("{}:{}", ds.title, s + 1) } else { ds.title.clone() };
                        out.push(IndexedRawSeries {
                            data_set_index: idx,
                            label: lbl,
                            values: vals,
                            excluded_count: ex,
                            non_numeric_count: non_num,
                            row_indices,
                        });
                    }
                }
            }
            SubcolumnAggregation::PooledGrandSample => {
                let mut vals = Vec::new();
                let mut row_indices = Vec::new();
                let mut ex = 0_usize;
                let mut non_num = 0_usize;
                for (r_idx, row) in ds.cells.iter().enumerate() {
                    for c in row {
                        if c.excluded {
                            ex += 1;
                        } else if let Some(v) = c.active_number() {
                            vals.push(v);
                            row_indices.push(r_idx);
                        } else if is_ignored_non_numeric_cell(c) {
                            non_num += 1;
                        }
                    }
                }
                if !vals.is_empty() {
                    out.push(IndexedRawSeries {
                        data_set_index: idx,
                        label: ds.title.clone(),
                        values: vals,
                        excluded_count: ex,
                        non_numeric_count: non_num,
                        row_indices,
                    });
                }
            }
        }
    }
    out
}

// ============================================================================
// Normality & Lognormality Battery
// ============================================================================

/// Result of a single normality test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalityTestOutcome {
    pub test_name: String,
    pub statistic: f64,
    pub p_value: f64,
    pub passed: bool,
}

/// Complete four-test normality battery plus optional lognormal comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalityBatteryResult {
    pub n: usize,
    pub dagostino_pearson: Option<NormalityTestOutcome>,
    pub shapiro_wilk: Option<NormalityTestOutcome>,
    pub anderson_darling: Option<NormalityTestOutcome>,
    pub kolmogorov_smirnov: Option<NormalityTestOutcome>,
    pub lognormal_dagostino_pearson: Option<NormalityTestOutcome>,
    pub lognormal_shapiro_wilk: Option<NormalityTestOutcome>,
    pub lognormal_anderson_darling: Option<NormalityTestOutcome>,
    pub lognormal_kolmogorov_smirnov: Option<NormalityTestOutcome>,
    /// `ln(L_normal / L_lognormal)`: positive favors normal, negative favors lognormal.
    pub normal_vs_lognormal_log_lr: Option<f64>,
}

/// D'Agostino–Pearson omnibus `K^2` normality test (`n >= 8`).
pub fn dagostino_pearson_k2(values: &[f64], alpha: f64) -> Result<NormalityTestOutcome, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n < 8 {
        return Err(StatsError::InsufficientSampleSize { context: "D'Agostino-Pearson K^2", required: 8, actual: n });
    }
    let moments = SummaryMoments::from_slice(values)?;
    if moments.css == 0.0 {
        return Err(StatsError::ZeroVariance { context: "D'Agostino-Pearson K^2" });
    }
    let nf = n as f64;
    let m2 = moments.css / nf;
    let mut s3 = NeumaierSum::new();
    let mut s4 = NeumaierSum::new();
    for &x in values {
        let d = x - moments.mean;
        let d2 = d * d;
        s3.add(d2 * d);
        s4.add(d2 * d2);
    }
    let m3 = s3.total() / nf;
    let m4 = s4.total() / nf;
    let sqrt_b1 = m3 / (m2 * m2.sqrt());
    let b2 = m4 / (m2 * m2);

    // Skewness normal transform (D'Agostino 1970).
    let y = sqrt_b1 * (((nf + 1.0) * (nf + 3.0)) / (6.0 * (nf - 2.0))).sqrt();
    let beta2 = (3.0 * (nf * nf + 27.0 * nf - 70.0) * (nf + 1.0) * (nf + 3.0)) / ((nf - 2.0) * (nf + 5.0) * (nf + 7.0) * (nf + 9.0));
    let w2 = -1.0 + (2.0 * (beta2 - 1.0)).sqrt();
    let delta = 1.0 / (0.5 * w2.ln()).sqrt();
    let alpha_skew = (2.0 / (w2 - 1.0)).sqrt();
    let z1 = delta * (y / alpha_skew).asinh();

    // Kurtosis normal transform (Anscombe & Glynn 1983).
    let e_b2 = 3.0 * (nf - 1.0) / (nf + 1.0);
    let var_b2 = (24.0 * nf * (nf - 2.0) * (nf - 3.0)) / ((nf + 1.0).powi(2) * (nf + 3.0) * (nf + 5.0));
    let x_k = (b2 - e_b2) / var_b2.sqrt();
    let sqrt_beta1 =
        (6.0 * (nf * nf - 5.0 * nf + 2.0) / ((nf + 7.0) * (nf + 9.0))) * ((6.0 * (nf + 3.0) * (nf + 5.0)) / (nf * (nf - 2.0) * (nf - 3.0))).sqrt();
    let a = 6.0 + (8.0 / sqrt_beta1) * (2.0 / sqrt_beta1 + (1.0 + 4.0 / (sqrt_beta1 * sqrt_beta1)).sqrt());
    let term = (1.0 - 2.0 / a) / (1.0 + x_k * (2.0 / (a - 4.0)).sqrt());
    let z2 = ((1.0 - 2.0 / (9.0 * a)) - term.cbrt()) / (2.0 / (9.0 * a)).sqrt();

    let k2 = (z1 * z1 + z2 * z2).max(0.0);
    let p_value = chi2_sf(k2, 2.0)?.clamp(0.0, 1.0);
    Ok(NormalityTestOutcome { test_name: String::from("D'Agostino-Pearson K2"), statistic: k2, p_value, passed: p_value > alpha })
}

/// Shapiro–Wilk `W` normality test (`3 <= n <= 5000`) using Royston (1992/1995) AS R94.
pub fn shapiro_wilk(values: &[f64], alpha: f64) -> Result<NormalityTestOutcome, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Shapiro-Wilk", required: 3, actual: n });
    }
    if n > 5000 {
        return Err(StatsError::InvalidParameter { name: "n", value: n as f64 });
    }
    let sorted = sorted_finite(values);
    let moments = SummaryMoments::from_slice(&sorted)?;
    if moments.css == 0.0 {
        return Err(StatsError::ZeroVariance { context: "Shapiro-Wilk" });
    }

    let weights = shapiro_wilk_weights(n)?;
    let mut num_acc = NeumaierSum::new();
    for i in 0..(n / 2) {
        let w_i = weights[n - 1 - i];
        num_acc.add(w_i * (sorted[n - 1 - i] - sorted[i]));
    }
    let num = num_acc.total();
    let w = ((num * num) / moments.css).clamp(0.0, 1.0);
    let p_value = shapiro_wilk_p_value(w, n);
    Ok(NormalityTestOutcome { test_name: String::from("Shapiro-Wilk W"), statistic: w, p_value, passed: p_value > alpha })
}

fn poly_eval(coeffs: &[f64], x: f64) -> f64 {
    let mut acc = 0.0_f64;
    for &c in coeffs.iter().rev() {
        acc = acc * x + c;
    }
    acc
}

fn shapiro_wilk_weights(n: usize) -> Result<Vec<f64>, StatsError> {
    if n == 3 {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        return Ok(vec![-s, 0.0, s]);
    }
    let nf = n as f64;
    let mut m = Vec::with_capacity(n);
    let mut m_sq_sum = 0.0_f64;
    for i in 1..=n {
        let p = ((i as f64) - 0.375) / (nf + 0.25);
        let mi = norm_ppf(p)?;
        m_sq_sum += mi * mi;
        m.push(mi);
    }
    let m_norm = m_sq_sum.sqrt();
    let mut c: Vec<f64> = m.iter().map(|&mi| mi / m_norm).collect();
    let u = 1.0 / nf.sqrt();

    let c1 = [0.0, 0.221_157, -0.147_981, -2.071_19, 4.434_685, -2.706_056];
    let a_n = c[n - 1] + poly_eval(&c1, u);

    let mut a = vec![0.0_f64; n];
    a[n - 1] = a_n;
    a[0] = -a_n;

    if n <= 5 {
        let phi = (m_sq_sum - 2.0 * m[n - 1] * m[n - 1]) / (1.0 - 2.0 * a_n * a_n);
        let scale = 1.0 / phi.sqrt();
        for i in 1..(n - 1) {
            a[i] = m[i] * scale;
        }
    } else {
        let c2 = [0.0, 0.042_981, -0.293_762, -1.752_461, 5.682_633, -3.582_633];
        let a_nm1 = c[n - 2] + poly_eval(&c2, u);
        a[n - 2] = a_nm1;
        a[1] = -a_nm1;
        let phi = (m_sq_sum - 2.0 * m[n - 1] * m[n - 1] - 2.0 * m[n - 2] * m[n - 2]) / (1.0 - 2.0 * a_n * a_n - 2.0 * a_nm1 * a_nm1);
        let scale = 1.0 / phi.sqrt();
        for i in 2..(n - 2) {
            a[i] = m[i] * scale;
        }
    }
    c.clear();
    Ok(a)
}

fn shapiro_wilk_p_value(w: f64, n: usize) -> f64 {
    if w >= 1.0 {
        return 1.0;
    }
    if n == 3 {
        let p = (6.0 / std::f64::consts::PI) * w.max(0.0).sqrt().clamp(0.0, 1.0).acos();
        return p.clamp(0.0, 1.0);
    }
    let one_minus_w = (1.0 - w).max(1e-16);
    if n <= 11 {
        let nf = n as f64;
        let gamma_val = 0.459 * nf - 2.273;
        let lw = one_minus_w.ln();
        if lw >= gamma_val {
            return 0.0;
        }
        let y = -(gamma_val - lw).ln();
        let mu = -0.000_671_4 * nf.powi(3) + 0.025_054 * nf.powi(2) - 0.399_78 * nf + 0.5440;
        let sigma = (-0.002_032_2 * nf.powi(3) + 0.062_767 * nf.powi(2) - 0.778_57 * nf + 1.3822).exp();
        norm_sf((y - mu) / sigma).clamp(0.0, 1.0)
    } else {
        let x = (n as f64).ln();
        let y = one_minus_w.ln();
        let mu = 0.003_891_5 * x.powi(3) - 0.083_751 * x.powi(2) - 0.310_82 * x - 1.5861;
        let sigma = (0.003_030_2 * x.powi(2) - 0.082_676 * x - 0.4803).exp();
        norm_sf((y - mu) / sigma).clamp(0.0, 1.0)
    }
}

/// Anderson–Darling normality test (`n >= 8`) with Stephens small-sample modification `A^{*2}`.
pub fn anderson_darling(values: &[f64], alpha: f64) -> Result<NormalityTestOutcome, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n < 8 {
        return Err(StatsError::InsufficientSampleSize { context: "Anderson-Darling", required: 8, actual: n });
    }
    let sorted = sorted_finite(values);
    let moments = SummaryMoments::from_slice(&sorted)?;
    let sd = moments.sample_sd()?;
    if sd == 0.0 {
        return Err(StatsError::ZeroVariance { context: "Anderson-Darling" });
    }
    let nf = n as f64;
    let mut sum_acc = NeumaierSum::new();
    for i in 0..n {
        let z_lo = (sorted[i] - moments.mean) / sd;
        let z_hi = (sorted[n - 1 - i] - moments.mean) / sd;
        let p_lo = norm_cdf(z_lo).clamp(1e-16, 1.0 - 1e-16);
        let q_hi = norm_sf(z_hi).clamp(1e-16, 1.0 - 1e-16);
        sum_acc.add(((2 * i + 1) as f64) * (p_lo.ln() + q_hi.ln()));
    }
    let a2 = (-nf - sum_acc.total() / nf).max(0.0);
    let a2_star = a2 * (1.0 + 0.75 / nf + 2.25 / (nf * nf));

    let p_value = if a2_star >= 0.6 {
        (1.2937 - 5.709 * a2_star + 0.0186 * a2_star * a2_star).exp()
    } else if a2_star > 0.34 {
        (0.9177 - 4.279 * a2_star - 1.38 * a2_star * a2_star).exp()
    } else if a2_star > 0.20 {
        1.0 - (-8.318 + 42.796 * a2_star - 59.938 * a2_star * a2_star).exp()
    } else {
        1.0 - (-13.436 + 101.14 * a2_star - 223.73 * a2_star * a2_star).exp()
    }
    .clamp(0.0, 1.0);

    Ok(NormalityTestOutcome { test_name: String::from("Anderson-Darling A2*"), statistic: a2_star, p_value, passed: p_value > alpha })
}

/// Kolmogorov–Smirnov normality test (`n >= 5`) using the clean-room Lilliefors (1967) /
/// Stephens (1974) distribution in [`lumencraft_math::dist::lilliefors_p_value`].
pub fn lilliefors_ks(values: &[f64], alpha: f64) -> Result<NormalityTestOutcome, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n < 5 {
        return Err(StatsError::InsufficientSampleSize { context: "Kolmogorov-Smirnov (Lilliefors)", required: 5, actual: n });
    }
    let sorted = sorted_finite(values);
    let moments = SummaryMoments::from_slice(&sorted)?;
    if moments.sample_sd()? == 0.0 {
        return Err(StatsError::ZeroVariance { context: "Kolmogorov-Smirnov (Lilliefors)" });
    }
    let d_max = lilliefors_d_from_sorted(&sorted)?;
    let p_value = lilliefors_p_value(d_max, n)?;

    Ok(NormalityTestOutcome { test_name: String::from("Kolmogorov-Smirnov (Lilliefors)"), statistic: d_max, p_value, passed: p_value > alpha })
}

/// Evaluate all four normality tests on `values` and (if strictly positive) on `ln(values)`.
pub fn normality_battery(values: &[f64], alpha: f64) -> Result<NormalityBatteryResult, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    let dagostino_pearson = dagostino_pearson_k2(values, alpha).ok();
    let shapiro_wilk_res = shapiro_wilk(values, alpha).ok();
    let anderson_darling_res = anderson_darling(values, alpha).ok();
    let ks_res = lilliefors_ks(values, alpha).ok();

    let (ln_dp, ln_sw, ln_ad, ln_ks, log_lr) = if values.iter().all(|&v| v > 0.0) && n >= 3 {
        let logs: Vec<f64> = values.iter().map(|v| v.ln()).collect();
        let m_x = SummaryMoments::from_slice(values)?;
        let m_l = SummaryMoments::from_slice(&logs)?;
        let lr = if m_x.css > 0.0 && m_l.css > 0.0 {
            let nf = n as f64;
            let var_x = m_x.css / nf;
            let var_l = m_l.css / nf;
            let ll_norm = -0.5 * nf * ((std::f64::consts::TAU * var_x).ln() + 1.0);
            let ll_lognorm = -0.5 * nf * ((std::f64::consts::TAU * var_l).ln() + 1.0) - m_l.sum;
            Some(ll_norm - ll_lognorm)
        } else {
            None
        };
        (
            dagostino_pearson_k2(&logs, alpha).ok(),
            shapiro_wilk(&logs, alpha).ok(),
            anderson_darling(&logs, alpha).ok(),
            lilliefors_ks(&logs, alpha).ok(),
            lr,
        )
    } else {
        (None, None, None, None, None)
    };

    Ok(NormalityBatteryResult {
        n,
        dagostino_pearson,
        shapiro_wilk: shapiro_wilk_res,
        anderson_darling: anderson_darling_res,
        kolmogorov_smirnov: ks_res,
        lognormal_dagostino_pearson: ln_dp,
        lognormal_shapiro_wilk: ln_sw,
        lognormal_anderson_darling: ln_ad,
        lognormal_kolmogorov_smirnov: ln_ks,
        normal_vs_lognormal_log_lr: log_lr,
    })
}

/// Generate Q–Q expected normal (or lognormal) quantiles paired with sorted sample values.
pub fn qq_expected_quantiles(values: &[f64], lognormal: bool) -> Result<Vec<(f64, f64)>, StatsError> {
    validate_finite_slice(values)?;
    if lognormal && values.iter().any(|&v| v <= 0.0) {
        return Err(StatsError::InvalidParameter { name: "lognormal_x", value: 0.0 });
    }
    let transformed: Vec<f64> = if lognormal { values.iter().map(|v| v.ln()).collect() } else { values.to_vec() };
    let sorted = sorted_finite(&transformed);
    let n = sorted.len();
    let moments = SummaryMoments::from_slice(&sorted)?;
    let sd = moments.sample_sd().unwrap_or(0.0);
    let nf = n as f64;
    let mut out = Vec::with_capacity(n);
    for (i, &obs) in sorted.iter().enumerate() {
        let p = ((i as f64) + 0.625) / (nf + 0.25);
        let z = norm_ppf(p)?;
        let expected = moments.mean + sd * z;
        out.push((expected, obs));
    }
    Ok(out)
}

// ============================================================================
// Outlier Detection (Grubbs & ROUT)
// ============================================================================

/// Flagged outlier record with original 0-based observation index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlierFlag {
    pub index: usize,
    pub value: f64,
    pub score: f64,
    pub threshold_or_p: f64,
}

/// Output of Grubbs' ESD outlier test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrubbsResult {
    pub n: usize,
    pub alpha: f64,
    pub iterative: bool,
    pub outliers: Vec<OutlierFlag>,
    pub cleaned_values: Vec<f64>,
}

/// Grubbs' extreme studentized deviate test (`n >= 3`, single or iterative two-sided).
pub fn grubbs_outliers(values: &[f64], alpha: f64, iterative: bool) -> Result<GrubbsResult, StatsError> {
    validate_finite_slice(values)?;
    if !alpha.is_finite() || alpha <= 0.0 || alpha >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "alpha", value: alpha });
    }
    if values.len() < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Grubbs outlier test", required: 3, actual: values.len() });
    }
    let mut active: Vec<(usize, f64)> = values.iter().copied().enumerate().collect();
    let mut outliers = Vec::new();

    loop {
        let n = active.len();
        if n < 3 {
            break;
        }
        let slice: Vec<f64> = active.iter().map(|&(_, v)| v).collect();
        let moments = SummaryMoments::from_slice(&slice)?;
        let sd = moments.sample_sd()?;
        if sd == 0.0 {
            break;
        }
        let mut max_pos = 0_usize;
        let mut max_g = -1.0_f64;
        for (pos, &(_, v)) in active.iter().enumerate() {
            let g = (v - moments.mean).abs() / sd;
            if g > max_g {
                max_g = g;
                max_pos = pos;
            }
        }
        let nf = n as f64;
        let t_crit = lumencraft_math::dist::student_t_isf(alpha / (2.0 * nf), nf - 2.0)?;
        let g_crit = ((nf - 1.0) / nf.sqrt()) * (t_crit * t_crit / (nf - 2.0 + t_crit * t_crit)).sqrt();
        if max_g > g_crit {
            let (orig_idx, val) = active.remove(max_pos);
            outliers.push(OutlierFlag { index: orig_idx, value: val, score: max_g, threshold_or_p: g_crit });
            if !iterative {
                break;
            }
        } else {
            break;
        }
    }

    let cleaned_values = active.into_iter().map(|(_, v)| v).collect();
    Ok(GrubbsResult { n: values.len(), alpha, iterative, outliers, cleaned_values })
}

/// Output of the standalone column ROUT outlier test (`K = 1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutResult {
    pub n: usize,
    pub fdr_q_percent: f64,
    pub robust_center: f64,
    pub rsdr: f64,
    pub outliers: Vec<OutlierFlag>,
    pub cleaned_values: Vec<f64>,
}

/// Standalone column ROUT outlier detection (`K = 1` robust Cauchy/Lorentzian IRLS location +
/// `P_68.27` RSDR + Benjamini–Hochberg FDR step-up at `fdr_q_percent`).
pub fn rout_outliers(values: &[f64], fdr_q_percent: f64) -> Result<RoutResult, StatsError> {
    validate_finite_slice(values)?;
    if !fdr_q_percent.is_finite() || fdr_q_percent <= 0.0 || fdr_q_percent > 100.0 {
        return Err(StatsError::InvalidParameter { name: "fdr_q_percent", value: fdr_q_percent });
    }
    let n = values.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "ROUT outlier detection", required: 3, actual: n });
    }
    let sorted = sorted_finite(values);
    let mut center = quantile_type6_sorted(&sorted, 0.5).unwrap_or(sorted[0]);
    let init_abs: Vec<f64> = sorted_finite(&values.iter().map(|v| (v - center).abs()).collect::<Vec<_>>());
    let init_scale = quantile_sorted(&init_abs, 0.6827).unwrap_or(0.0) * (n as f64) / ((n - 1) as f64);

    if init_scale > 0.0 {
        // Cauchy/Lorentzian IRLS refinement of location M (`K = 1`).
        let mut scale = init_scale;
        for _ in 0..25 {
            let mut num = NeumaierSum::new();
            let mut den = NeumaierSum::new();
            for &x in values {
                let r = (x - center) / scale;
                let w = 1.0 / (1.0 + r * r);
                num.add(w * x);
                den.add(w);
            }
            let d = den.total();
            if d <= 0.0 {
                break;
            }
            let next_center = num.total() / d;
            let shift = (next_center - center).abs();
            center = next_center;
            let res_abs = sorted_finite(&values.iter().map(|v| (v - center).abs()).collect::<Vec<_>>());
            let next_rsdr = quantile_sorted(&res_abs, 0.6827).unwrap_or(0.0) * (n as f64) / ((n - 1) as f64);
            if next_rsdr > 0.0 {
                scale = next_rsdr;
            }
            if shift <= 1e-12 * scale.max(1.0) {
                break;
            }
        }
    }

    let mut abs_res: Vec<(usize, f64)> = values.iter().enumerate().map(|(i, &v)| (i, (v - center).abs())).collect();
    abs_res.sort_by(|a, b| a.1.total_cmp(&b.1));
    let only_abs: Vec<f64> = abs_res.iter().map(|&(_, r)| r).collect();
    let rsdr = quantile_sorted(&only_abs, 0.6827).unwrap_or(0.0) * (n as f64) / ((n - 1) as f64);

    let mut outlier_mask = vec![false; n];
    let mut outliers = Vec::new();
    let q = fdr_q_percent / 100.0;
    let df = (n - 1) as f64;
    let start = ((0.70 * (n as f64)).floor() as usize).min(n - 1);
    let rout_stat_p = |r: f64| -> Result<(f64, f64), StatsError> {
        if rsdr > 0.0 {
            let t_stat = r / rsdr;
            Ok((t_stat, student_t_two_sided_p(t_stat, df)?))
        } else if r > 0.0 {
            // Bulk (>=68.27%) residuals are zero so RSDR == 0: any non-zero residual
            // is infinitely many robust SDs from the bulk center with P = 0.
            Ok((f64::INFINITY, 0.0))
        } else {
            Ok((0.0, 1.0))
        }
    };
    let mut first_outlier_pos = None;
    for (pos, &(_, r)) in abs_res.iter().enumerate().skip(start) {
        let (_, p_val) = rout_stat_p(r)?;
        let bh_thresh = q * ((n - pos) as f64) / (n as f64);
        if p_val <= bh_thresh {
            first_outlier_pos = Some(pos);
            break;
        }
    }
    if let Some(mut cut) = first_outlier_pos {
        if rsdr == 0.0 && abs_res[cut].1 > 0.0 {
            while cut > 0 && abs_res[cut - 1].1 == abs_res[cut].1 {
                cut -= 1;
            }
        }
        for &(orig_idx, r) in &abs_res[cut..] {
            outlier_mask[orig_idx] = true;
            let (t_stat, p_val) = rout_stat_p(r)?;
            outliers.push(OutlierFlag { index: orig_idx, value: values[orig_idx], score: t_stat, threshold_or_p: p_val });
        }
    }

    let cleaned_values = values.iter().enumerate().filter_map(|(i, &v)| (!outlier_mask[i]).then_some(v)).collect();
    Ok(RoutResult { n, fdr_q_percent, robust_center: center, rsdr, outliers, cleaned_values })
}

// ============================================================================
// One-Sample Hypothesis Tests
// ============================================================================

/// Result of a one-sample *t*-test against hypothetical mean `mu0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OneSampleTResult {
    pub n: usize,
    pub hypothetical_mean: f64,
    pub sample_mean: f64,
    pub discrepancy: f64,
    pub sd: f64,
    pub sem: f64,
    pub t_statistic: f64,
    pub df: f64,
    pub p_value: f64,
    pub two_tailed: bool,
    pub ci: (f64, f64),
    pub cohens_d: f64,
    pub hedges_g: f64,
    pub hedges_g_ci: Option<(f64, f64)>,
}

/// One-sample Student *t*-test against `hypothetical_mean`.
pub fn one_sample_t_test(
    values: &[f64],
    hypothetical_mean: f64,
    two_tailed: bool,
    confidence: ConfidenceLevel,
) -> Result<OneSampleTResult, StatsError> {
    validate_finite_slice(values)?;
    if !hypothetical_mean.is_finite() {
        return Err(StatsError::InvalidParameter { name: "hypothetical_mean", value: hypothetical_mean });
    }
    let n = values.len();
    if n < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "one-sample t test", required: 2, actual: n });
    }
    let moments = SummaryMoments::from_slice(values)?;
    let sd = moments.sample_sd()?;
    if sd == 0.0 {
        return Err(StatsError::ZeroVariance { context: "one-sample t test" });
    }
    let sem = moments.sem()?;
    let discrepancy = moments.mean - hypothetical_mean;
    let t_stat = discrepancy / sem;
    let df = (n - 1) as f64;
    let p_value = if two_tailed {
        student_t_two_sided_p(t_stat, df)?
    } else if t_stat >= 0.0 {
        student_t_sf(t_stat, df)?
    } else {
        student_t_cdf(t_stat, df)?
    };
    let t_crit = t_crit_two_sided(confidence, df)?;
    let ci = (discrepancy - t_crit * sem, discrepancy + t_crit * sem);

    let d = discrepancy / sd;
    let j = hedges_j_factor(df)?;
    let g = j * d;
    let inv_sqrt_n = 1.0 / (n as f64).sqrt();
    let g_ci = noncentral_t_ci(t_stat, df, confidence.percent()).ok().map(|(lo, hi)| (lo * inv_sqrt_n * j, hi * inv_sqrt_n * j));

    Ok(OneSampleTResult {
        n,
        hypothetical_mean,
        sample_mean: moments.mean,
        discrepancy,
        sd,
        sem,
        t_statistic: t_stat,
        df,
        p_value,
        two_tailed,
        ci,
        cohens_d: d,
        hedges_g: g,
        hedges_g_ci: g_ci,
    })
}

/// Exact Hedges' small-sample correction factor `J(df) = Gamma(df/2) / (sqrt(df/2) * Gamma((df - 1)/2))`.
pub(crate) fn hedges_j_factor(df: f64) -> Result<f64, MathError> {
    if df > 100.0 {
        return Ok(1.0 - 3.0 / (4.0 * df - 1.0));
    }
    let num = gamma(0.5 * df)?;
    let den = (0.5 * df).sqrt() * gamma(0.5 * (df - 1.0))?;
    Ok(num / den)
}

/// Result of a one-sample Wilcoxon signed-rank test against hypothetical median `mu0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OneSampleWilcoxonResult {
    pub n_total: usize,
    pub n_nonzero: usize,
    pub hypothetical_median: f64,
    pub sample_median: f64,
    pub w_plus: f64,
    pub w_minus: f64,
    pub w_stat: f64,
    pub p_value: f64,
    pub exact_p_used: bool,
    pub two_tailed: bool,
    pub pseudomedian_hl: f64,
    pub pseudomedian_ci: Option<(f64, f64)>,
}

/// One-sample Wilcoxon signed-rank test against `hypothetical_median`.
/// Uses exact dynamic-programming distribution for `n_nonzero <= 25` without ties, and
/// tie-corrected + continuity-corrected normal approximation otherwise.
pub fn one_sample_wilcoxon(
    values: &[f64],
    hypothetical_median: f64,
    two_tailed: bool,
    confidence: ConfidenceLevel,
) -> Result<OneSampleWilcoxonResult, StatsError> {
    validate_finite_slice(values)?;
    if !hypothetical_median.is_finite() {
        return Err(StatsError::InvalidParameter { name: "hypothetical_median", value: hypothetical_median });
    }
    let sorted = sorted_finite(values);
    let sample_median = quantile_type6_sorted(&sorted, 0.5).unwrap_or(sorted[0]);
    let diffs: Vec<f64> = values.iter().map(|&v| v - hypothetical_median).filter(|&d| d != 0.0).collect();
    let n_nz = diffs.len();
    if n_nz == 0 {
        return Ok(OneSampleWilcoxonResult {
            n_total: values.len(),
            n_nonzero: 0,
            hypothetical_median,
            sample_median,
            w_plus: 0.0,
            w_minus: 0.0,
            w_stat: 0.0,
            p_value: 1.0,
            exact_p_used: true,
            two_tailed,
            pseudomedian_hl: sample_median,
            pseudomedian_ci: None,
        });
    }

    let abs_diffs: Vec<f64> = diffs.iter().map(|d| d.abs()).collect();
    let ranked = rank_with_ties(&abs_diffs)?;
    let mut w_plus = 0.0_f64;
    let mut w_minus = 0.0_f64;
    for (&d, &r) in diffs.iter().zip(ranked.ranks.iter()) {
        if d > 0.0 {
            w_plus += r;
        } else {
            w_minus += r;
        }
    }
    let w_signed = w_plus - w_minus;
    let (p_value, exact_p_used) = if n_nz <= 25 && ranked.tie_counts.is_empty() {
        let w_pos_int = w_plus.round() as usize;
        let max_sum = n_nz * (n_nz + 1) / 2;
        let p = wilcoxon_exact_p(n_nz, w_pos_int, max_sum, two_tailed);
        (p, true)
    } else {
        let nf = n_nz as f64;
        let mean_w = nf * (nf + 1.0) / 4.0;
        let var_w = ((nf * (nf + 1.0) * (2.0 * nf + 1.0) - 0.5 * ranked.tie_sum_t3_minus_t) / 24.0).max(1e-30);
        let sd_w = var_w.sqrt();
        let p = if two_tailed {
            let cc = (w_plus - mean_w).abs() - 0.5;
            if cc <= 0.0 { 1.0 } else { (2.0 * norm_sf(cc / sd_w)).min(1.0) }
        } else {
            let cc = ((w_plus - mean_w).abs() - 0.5).max(0.0);
            norm_sf(cc / sd_w).clamp(0.0, 1.0)
        };
        (p.clamp(0.0, 1.0), false)
    };

    let (pseudomedian_hl, pseudomedian_ci) = hodges_lehmann_one_sample(values, confidence)?;
    Ok(OneSampleWilcoxonResult {
        n_total: values.len(),
        n_nonzero: n_nz,
        hypothetical_median,
        sample_median,
        w_plus,
        w_minus,
        w_stat: w_signed,
        p_value,
        exact_p_used,
        two_tailed,
        pseudomedian_hl,
        pseudomedian_ci,
    })
}

/// Exact DP distribution of `W+` under `H_0` for `n <= 25` without ties.
pub(crate) fn wilcoxon_exact_p(n: usize, w_plus: usize, max_sum: usize, two_tailed: bool) -> f64 {
    let mut ways = vec![0_u64; max_sum + 1];
    ways[0] = 1;
    for rank in 1..=n {
        for s in (rank..=max_sum).rev() {
            ways[s] += ways[s - rank];
        }
    }
    let total = 2.0_f64.powi(n as i32);
    let upper_tail: u64 = ways.iter().skip(w_plus).sum();
    let lower_tail: u64 = ways.iter().take(w_plus + 1).sum();
    let min_tail = (upper_tail as f64).min(lower_tail as f64);
    if two_tailed { (2.0 * min_tail / total).clamp(0.0, 1.0) } else { (min_tail / total).clamp(0.0, 1.0) }
}

/// Hodges–Lehmann one-sample pseudomedian (median of Walsh averages `(x_i + x_j) / 2, i <= j`)
/// and exact/normal confidence interval.
pub(crate) fn hodges_lehmann_one_sample(values: &[f64], confidence: ConfidenceLevel) -> Result<(f64, Option<(f64, f64)>), StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    let (hl, m_u64, lo_ci_val, hi_ci_val) = if n > 2_000 {
        let sorted = sorted_finite(values);
        let m_u64 = (n as u64) * ((n as u64) + 1) / 2;
        let hl = if m_u64 % 2 == 1 {
            select_kth_upper_triangle_half_sum(&sorted, m_u64 / 2)?
        } else {
            let mid_lo = select_kth_upper_triangle_half_sum(&sorted, m_u64 / 2 - 1)?;
            let mid_hi = select_kth_upper_triangle_half_sum(&sorted, m_u64 / 2)?;
            0.5 * mid_lo + 0.5 * mid_hi
        };
        if n < 4 {
            return Ok((hl, None));
        }
        let z = lumencraft_math::dist::norm_two_sided_isf(confidence.alpha())?;
        let nf = n as f64;
        let mu = nf * (nf + 1.0) / 4.0;
        let sd = (nf * (nf + 1.0) * (2.0 * nf + 1.0) / 24.0).sqrt();
        let k = (mu - z * sd).floor().max(1.0) as u64;
        let lo_idx = k.saturating_sub(1).min(m_u64 - 1);
        let hi_idx = (m_u64 - k).min(m_u64 - 1);
        let ci_lo = select_kth_upper_triangle_half_sum(&sorted, lo_idx)?;
        let ci_hi = select_kth_upper_triangle_half_sum(&sorted, hi_idx)?;
        (hl, m_u64, ci_lo, ci_hi)
    } else {
        let m = n * (n + 1) / 2;
        let mut walsh = Vec::with_capacity(m);
        for i in 0..n {
            for j in i..n {
                walsh.push(0.5 * values[i] + 0.5 * values[j]);
            }
        }
        walsh.sort_by(f64::total_cmp);
        let hl = quantile_type6_sorted(&walsh, 0.5).unwrap_or(walsh[0]);
        if n < 4 {
            return Ok((hl, None));
        }
        let z = lumencraft_math::dist::norm_two_sided_isf(confidence.alpha())?;
        let nf = n as f64;
        let mu = nf * (nf + 1.0) / 4.0;
        let sd = (nf * (nf + 1.0) * (2.0 * nf + 1.0) / 24.0).sqrt();
        let k = (mu - z * sd).floor().max(1.0) as usize;
        let lo_idx = k.saturating_sub(1).min(m - 1);
        let hi_idx = (m - k).min(m - 1);
        (hl, m as u64, walsh[lo_idx], walsh[hi_idx])
    };
    let _ = m_u64;
    Ok((hl, Some((lo_ci_val, hi_ci_val))))
}

/// Kolmogorov–Smirnov one-sample test against a known theoretical `Normal(mean, sd^2)`.
pub fn one_sample_ks_normal(values: &[f64], mean: f64, sd: f64) -> Result<NormalityTestOutcome, StatsError> {
    validate_finite_slice(values)?;
    if !mean.is_finite() || !sd.is_finite() || sd <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "sd", value: sd });
    }
    let sorted = sorted_finite(values);
    let n = sorted.len();
    let nf = n as f64;
    let mut d = 0.0_f64;
    for (i, &x) in sorted.iter().enumerate() {
        let f_z = norm_cdf((x - mean) / sd);
        d = d.max(((i + 1) as f64) / nf - f_z).max(f_z - (i as f64) / nf);
    }
    let p_value = ks_asymptotic_p(d * nf.sqrt());
    Ok(NormalityTestOutcome { test_name: String::from("One-sample KS (Normal)"), statistic: d, p_value, passed: p_value > 0.05 })
}

/// Asymptotic Kolmogorov–Smirnov survival probability `P(K >= lambda)`.
pub(crate) fn ks_asymptotic_p(lambda: f64) -> f64 {
    if !lambda.is_finite() || lambda <= 0.0 {
        return 1.0;
    }
    if lambda < 0.27 {
        return 1.0;
    }
    let l2 = lambda * lambda;
    let mut sum = 0.0_f64;
    for k in 1..=100 {
        let kf = k as f64;
        let term = (-2.0 * kf * kf * l2).exp();
        if k % 2 == 1 {
            sum += term;
        } else {
            sum -= term;
        }
        if term <= 1e-16 * sum.abs().max(1e-300) {
            break;
        }
    }
    (2.0 * sum).clamp(0.0, 1.0)
}

// ============================================================================
// ResultSheet Builders
// ============================================================================

/// Execute [`DescriptiveSpec`] on a [`DataSheet`] and emit a complete [`ResultSheet`].
pub fn run_descriptive_analysis(sheet: &DataSheet, spec: &AnalysisSpec, desc: &DescriptiveSpec) -> Result<ResultSheet, StatsError> {
    let mut res = init_result_sheet(spec)?;
    let extracted = extract_indexed_series_for_descriptive(sheet, &spec.selected_data_sets, desc.aggregation)?;
    let raw_series = extracted.raw;
    let pooled_series = extracted.pooled;
    if raw_series.is_empty() && pooled_series.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    append_xy_trapezoidal_auc(&mut res, sheet, &spec.selected_data_sets)?;

    let p_fmt = spec.p_value_format;
    let conf = spec.confidence_level;
    if !pooled_series.is_empty() {
        populate_pooled_descriptive_sheet(&mut res, &pooled_series, conf)?;
        sanitize_result_sheet_floats(&mut res)?;
        return Ok(res);
    }

    let mut summaries = Vec::with_capacity(raw_series.len());
    for series in &raw_series {
        summaries.push(summarize_column(&series.label, &series.values, series.excluded_count, conf, desc.custom_upper_percentile)?);
    }
    res.tabs.push(build_descriptive_tab(&summaries, conf));

    for (series, s) in raw_series.iter().zip(&summaries) {
        let ds_idx = Some(series.data_set_index);
        push_scalar(&mut res, "N", ds_idx, s.n as f64);
        push_scalar(&mut res, "Mean", ds_idx, s.mean);
        push_scalar(&mut res, "Median", ds_idx, s.median);
        if let Some(sd) = s.sd {
            push_scalar(&mut res, "SD", ds_idx, sd);
        }
        if let Some(sem) = s.sem {
            push_scalar(&mut res, "SEM", ds_idx, sem);
        }
    }

    if desc.run_normality_battery {
        let mut batteries = Vec::with_capacity(raw_series.len());
        for series in &raw_series {
            batteries.push(normality_battery(&series.values, conf.alpha())?);
        }
        res.tabs.push(build_normality_tab(&summaries, &batteries, p_fmt));
        append_normality_checklist(&mut res.checklist, &summaries, &batteries, conf.alpha());
    }

    if let Some(mu0) = desc.hypothetical_value {
        append_one_sample_tab(&mut res, &raw_series, mu0, conf, p_fmt)?;
    }

    if let Some(qq_tab) = build_normal_qq_tab(&raw_series)? {
        res.tabs.push(qq_tab);
    }

    append_row_descriptive_stats(&mut res, sheet, &spec.selected_data_sets, desc.aggregation)?;

    if summaries.iter().all(|s| s.n < 3) {
        res.checklist.push(checklist_item(
            "Sample size adequacy",
            ChecklistStatus::Warn,
            Some(format!("min n = {}", summaries.iter().map(|s| s.n).min().unwrap_or(0))),
            "Columns with fewer than 3 values cannot support exact median CIs or normality tests.",
        ));
    } else {
        res.checklist.push(checklist_item(
            "Descriptive summary completed",
            ChecklistStatus::Pass,
            Some(format!("{} column(s)", summaries.len())),
            "Compensated moments, quantiles, and exact binomial median confidence intervals computed.",
        ));
    }

    append_ignored_text_checklist_and_footnote(&mut res, "descriptive statistics", &extracted.ignored_non_numeric);
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}

/// Compute per-dataset trapezoidal area under curve (`sum_i 0.5 * (x_{i+1} - x_i) * (y_i + y_{i+1})`)
/// for [`TableArchetype::Xy`] when at least 2 sorted finite `(x, mean_y)` points exist.
fn append_xy_trapezoidal_auc(res: &mut ResultSheet, sheet: &DataSheet, selected: &[usize]) -> Result<(), StatsError> {
    let TableArchetype::Xy(xy) = &sheet.archetype else {
        return Ok(());
    };
    let ds_indices = resolve_selected_indices(selected, xy.data_sets.len());
    for ds_idx in ds_indices {
        let ds = &xy.data_sets[ds_idx];
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for r in 0..xy.row_count() {
            let Some(x) = xy.x_value_at(r) else { continue };
            if !x.is_finite() {
                continue;
            }
            let mean_y = if xy.y_subcolumn_mode.supports_parametric_moments() {
                xy.summarized_y_point(r, ds_idx)?.map(|p| p.mean).filter(|v| v.is_finite())
            } else if !xy.y_subcolumn_mode.is_pre_summarized() {
                let y_vals: Vec<f64> = ds.cells.get(r).map(|row| row.iter().filter_map(|c| c.active_number()).collect()).unwrap_or_default();
                if y_vals.is_empty() { None } else { Some(SummaryMoments::from_slice(&y_vals)?.mean) }
            } else {
                None
            };
            if let Some(y) = mean_y {
                pts.push((x, y));
            }
        }
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        if pts.len() >= 2 {
            let mut auc_acc = NeumaierSum::new();
            for w in pts.windows(2) {
                auc_acc.add(0.5 * (w[1].0 - w[0].0) * (w[0].1 + w[1].1));
            }
            push_scalar(res, "Trapezoidal AUC", Some(ds_idx), auc_acc.total());
        }
    }
    Ok(())
}

/// Emit per-row `N`, `Mean`, `SD`, `Min`, `Max` in `"row-descriptive-stats"` when `>= 2`
/// active numeric columns exist on `MultipleVariables`, `Column`, or `Grouped` tables.
fn append_row_descriptive_stats(
    res: &mut ResultSheet,
    sheet: &DataSheet,
    selected: &[usize],
    aggregation: SubcolumnAggregation,
) -> Result<(), StatsError> {
    let row_entries: Vec<(String, Vec<f64>)> = match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let indices = resolve_selected_indices(selected, mv.variables.len());
            let active_cols: Vec<usize> = indices
                .into_iter()
                .filter(|&idx| {
                    let var = &mv.variables[idx];
                    var.role != VariableRole::Categorical && var.cells.iter().any(|c| c.active_number().is_some())
                })
                .collect();
            if active_cols.len() < 2 {
                return Ok(());
            }
            (0..mv.row_count())
                .map(|r| {
                    let label = mv
                        .row_titles
                        .get(r)
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("Row {}", r + 1));
                    let vals: Vec<f64> =
                        active_cols.iter().filter_map(|&c_idx| mv.variables[c_idx].cells.get(r).and_then(|c| c.active_number())).collect();
                    (label, vals)
                })
                .collect()
        }
        TableArchetype::Column(col_tbl) => {
            if col_tbl.subcolumn_mode.is_pre_summarized() {
                return Ok(());
            }
            collect_dataset_rows_for_desc(&col_tbl.data_sets, &col_tbl.row_titles, selected, aggregation)?
        }
        TableArchetype::Grouped(grp_tbl) => {
            if grp_tbl.subcolumn_mode.is_pre_summarized() {
                return Ok(());
            }
            collect_dataset_rows_for_desc(&grp_tbl.data_sets, &grp_tbl.row_titles, selected, aggregation)?
        }
        _ => return Ok(()),
    };

    let mut row_labels = Vec::new();
    let mut rows = Vec::new();
    for (label, vals) in row_entries {
        if vals.is_empty() {
            continue;
        }
        let moments = SummaryMoments::from_slice(&vals)?;
        let sd = if vals.len() >= 2 { moments.sample_sd().ok() } else { None };
        let min_v = vals.iter().copied().fold(f64::INFINITY, f64::min);
        let max_v = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        row_labels.push(label);
        rows.push(vec![num_cell(vals.len() as f64), num_cell(moments.mean), opt_num_cell(sd), num_cell(min_v), num_cell(max_v)]);
    }

    if !rows.is_empty() {
        res.tabs.push(build_tab(
            "row-descriptive-stats",
            "Row-wise descriptive statistics",
            ResultTabKind::Diagnostics,
            vec![String::from("N"), String::from("Mean"), String::from("SD"), String::from("Min"), String::from("Max")],
            row_labels,
            rows,
        ));
    }
    Ok(())
}

fn collect_dataset_rows_for_desc(
    data_sets: &[lumencraft_doc::DataSetColumn],
    row_titles: &[String],
    selected: &[usize],
    aggregation: SubcolumnAggregation,
) -> Result<Vec<(String, Vec<f64>)>, StatsError> {
    let indices = resolve_selected_indices(selected, data_sets.len());
    let active_cols: Vec<usize> =
        indices.into_iter().filter(|&idx| data_sets[idx].cells.iter().any(|row| row.iter().any(|c| c.active_number().is_some()))).collect();
    if active_cols.len() < 2 {
        return Ok(Vec::new());
    }
    let n_rows = active_cols.iter().map(|&idx| data_sets[idx].row_count()).max().unwrap_or(0);
    let mut out = Vec::with_capacity(n_rows);
    for r in 0..n_rows {
        let label = row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| format!("Row {}", r + 1));
        let mut vals = Vec::new();
        for &c_idx in &active_cols {
            let Some(cell_row) = data_sets[c_idx].cells.get(r) else { continue };
            let nums: Vec<f64> = cell_row.iter().filter_map(|c| c.active_number()).collect();
            if nums.is_empty() {
                continue;
            }
            if matches!(aggregation, SubcolumnAggregation::RowMeans) {
                vals.push(SummaryMoments::from_slice(&nums)?.mean);
            } else {
                vals.extend_from_slice(&nums);
            }
        }
        out.push((label, vals));
    }
    Ok(out)
}

fn build_normal_qq_tab(raw_series: &[IndexedRawSeries]) -> Result<Option<ResultTab>, StatsError> {
    if raw_series.is_empty() {
        return Ok(None);
    }
    let headers = vec![
        String::from("Data Set"),
        String::from("Order Index"),
        String::from("Normal Expected"),
        String::from("Normal Observed"),
        String::from("Lognormal Expected (ln)"),
        String::from("Lognormal Observed ln(Y)"),
    ];
    let mut row_labels = Vec::new();
    let mut rows = Vec::new();
    let mut footnotes = Vec::new();

    for series in raw_series {
        let label = &series.label;
        let vals = &series.values;
        if vals.is_empty() {
            continue;
        }
        let normal_pairs = qq_expected_quantiles(vals, false)?;
        let lognormal_pairs = qq_expected_quantiles(vals, true).ok();
        if lognormal_pairs.is_none() {
            footnotes.push(format!("{label}: lognormal Q-Q coordinates omitted because one or more values are <= 0."));
        }
        for (idx, &(exp_n, obs_n)) in normal_pairs.iter().enumerate() {
            let (exp_ln, obs_ln) = match lognormal_pairs.as_ref().and_then(|v| v.get(idx)) {
                Some(&(e, o)) => (Some(e), Some(o)),
                None => (None, None),
            };
            row_labels.push(format!("{label} #{}", idx + 1));
            rows.push(vec![
                ResultCell::Text(label.clone()),
                num_cell((idx + 1) as f64),
                num_cell(exp_n),
                num_cell(obs_n),
                opt_num_cell(exp_ln),
                opt_num_cell(obs_ln),
            ]);
        }
    }

    if rows.is_empty() {
        return Ok(None);
    }
    let mut tab = build_tab("normal-qq", "Normal & Lognormal Q-Q Quantiles", ResultTabKind::NormalQq, headers, row_labels, rows);
    tab.footnotes = footnotes;
    Ok(Some(tab))
}

fn populate_pooled_descriptive_sheet(res: &mut ResultSheet, pooled_series: &[IndexedPooledSeries], conf: ConfidenceLevel) -> Result<(), StatsError> {
    let mut pooled = Vec::with_capacity(pooled_series.len());
    for ps in pooled_series {
        pooled.push(summarize_pre_summarized_points(&ps.label, &ps.points, conf)?);
    }
    let headers: Vec<String> = pooled.iter().map(|p| p.label.clone()).collect();
    let row_labels = vec![
        String::from("Entered summary rows"),
        String::from("Total N"),
        String::from("Pooled Mean"),
        String::from("Pooled SD"),
        String::from("Pooled SEM"),
        format!("{}% CI of Mean", conf.percent()),
    ];
    let mut rows = vec![Vec::with_capacity(pooled.len()); row_labels.len()];
    for (ps, p) in pooled_series.iter().zip(&pooled) {
        rows[0].push(num_cell(p.point_count as f64));
        rows[1].push(num_cell(p.total_n));
        rows[2].push(num_cell(p.mean));
        rows[3].push(opt_num_cell(p.sd));
        rows[4].push(opt_num_cell(p.sem));
        rows[5].push(opt_ci_cell(p.mean_ci));
        let ds_idx = Some(ps.data_set_index);
        push_scalar(res, "Mean", ds_idx, p.mean);
        if let Some(sd) = p.sd {
            push_scalar(res, "SD", ds_idx, sd);
        }
    }
    res.tabs.push(build_tab("desc-pooled", "Descriptive Statistics (Pooled)", ResultTabKind::DescriptiveSummary, headers, row_labels, rows));
    res.checklist.push(checklist_item(
        "Pre-summarized input mode",
        ChecklistStatus::Info,
        None,
        "Moments were reconstructed from entered Mean, Spread, and N; quantiles and normality tests require raw replicates.",
    ));
    Ok(())
}

fn build_descriptive_tab(summaries: &[DescriptiveSummary], conf: ConfidenceLevel) -> ResultTab {
    let headers: Vec<String> = summaries.iter().map(|s| s.label.clone()).collect();
    let row_labels = vec![
        String::from("Number of values"),
        String::from("Excluded values"),
        String::from("Minimum"),
        String::from("25% Percentile"),
        String::from("Median"),
        String::from("75% Percentile"),
        String::from("Maximum"),
        String::from("Range"),
        String::from("IQR"),
        String::from("Mean"),
        String::from("Std. Deviation"),
        String::from("CI of SD"),
        String::from("Std. Error of Mean"),
        format!("{}% CI of Mean", conf.percent()),
        format!("{}% CI of Median", conf.percent()),
        String::from("Median CI actual confidence (%)"),
        String::from("Coefficient of variation (%)"),
        String::from("Geometric mean"),
        String::from("Geometric SD factor"),
        String::from("Harmonic mean"),
        String::from("Quadratic mean (RMS)"),
        String::from("Skewness"),
        String::from("Excess kurtosis"),
        String::from("Sum"),
    ];
    let mut rows = vec![Vec::with_capacity(summaries.len()); row_labels.len()];
    for s in summaries {
        rows[0].push(num_cell(s.n as f64));
        rows[1].push(num_cell(s.excluded_count as f64));
        rows[2].push(num_cell(s.min));
        rows[3].push(num_cell(s.q1));
        rows[4].push(num_cell(s.median));
        rows[5].push(num_cell(s.q3));
        rows[6].push(num_cell(s.max));
        rows[7].push(num_cell(s.range));
        rows[8].push(num_cell(s.iqr));
        rows[9].push(num_cell(s.mean));
        rows[10].push(opt_num_cell(s.sd));
        rows[11].push(opt_ci_cell(s.sd_ci));
        rows[12].push(opt_num_cell(s.sem));
        rows[13].push(opt_ci_cell(s.mean_ci));
        rows[14].push(opt_ci_cell(s.median_ci.map(|m| (m.lower, m.upper))));
        rows[15].push(opt_num_cell(s.median_ci.map(|m| m.actual_confidence_percent)));
        rows[16].push(opt_num_cell(s.cv_percent));
        rows[17].push(opt_num_cell(s.geometric_mean));
        rows[18].push(opt_num_cell(s.geometric_sd_factor));
        rows[19].push(opt_num_cell(s.harmonic_mean));
        rows[20].push(num_cell(s.quadratic_mean));
        rows[21].push(opt_num_cell(s.skewness));
        rows[22].push(opt_num_cell(s.excess_kurtosis));
        rows[23].push(num_cell(s.sum));
    }
    let mut tab = build_tab("desc-summary", "Descriptive Statistics", ResultTabKind::DescriptiveSummary, headers, row_labels, rows);
    tab.footnotes.push(format!(
        "{}% CI of SD uses exact chi-square variance bounds under a normal-theory Gaussian assumption; skewed or heavy-tailed distributions are sensitive to non-normality.",
        conf.percent()
    ));
    tab
}

fn build_normality_tab(summaries: &[DescriptiveSummary], batteries: &[NormalityBatteryResult], p_fmt: PValueFormat) -> ResultTab {
    let headers: Vec<String> = summaries.iter().map(|s| s.label.clone()).collect();
    let row_labels = vec![
        String::from("D'Agostino-Pearson K2"),
        String::from("D'Agostino-Pearson P"),
        String::from("Shapiro-Wilk W"),
        String::from("Shapiro-Wilk P"),
        String::from("Anderson-Darling A2*"),
        String::from("Anderson-Darling P"),
        String::from("Kolmogorov-Smirnov D"),
        String::from("Kolmogorov-Smirnov P"),
        String::from("Normal vs. Lognormal log-LR"),
    ];
    let mut rows = vec![Vec::with_capacity(batteries.len()); row_labels.len()];
    if let [r0, r1, r2, r3, r4, r5, r6, r7, r8] = rows.as_mut_slice() {
        for b in batteries {
            push_norm_pair(r0, r1, b.dagostino_pearson.as_ref(), p_fmt);
            push_norm_pair(r2, r3, b.shapiro_wilk.as_ref(), p_fmt);
            push_norm_pair(r4, r5, b.anderson_darling.as_ref(), p_fmt);
            push_norm_pair(r6, r7, b.kolmogorov_smirnov.as_ref(), p_fmt);
            r8.push(opt_num_cell(b.normal_vs_lognormal_log_lr));
        }
    }
    build_tab("normality", "Normality & Lognormality Tests", ResultTabKind::Diagnostics, headers, row_labels, rows)
}

fn push_norm_pair(stat_row: &mut Vec<ResultCell>, p_row: &mut Vec<ResultCell>, out: Option<&NormalityTestOutcome>, p_fmt: PValueFormat) {
    match out {
        Some(o) => {
            stat_row.push(num_cell(o.statistic));
            p_row.push(p_cell(o.p_value, p_fmt));
        }
        None => {
            stat_row.push(ResultCell::Empty);
            p_row.push(ResultCell::Empty);
        }
    }
}

fn append_normality_checklist(
    checklist: &mut Vec<ChecklistItem>,
    summaries: &[DescriptiveSummary],
    batteries: &[NormalityBatteryResult],
    alpha: f64,
) {
    for (s, b) in summaries.iter().zip(batteries.iter()) {
        if let Some(sw) = &b.shapiro_wilk {
            if sw.p_value <= alpha {
                checklist.push(checklist_item(
                    format!("Normality ({})", s.label),
                    ChecklistStatus::Warn,
                    Some(format!("Shapiro-Wilk W = {:.4}, p = {:.4}", sw.statistic, sw.p_value)),
                    format!(
                        "Shapiro-Wilk W = {:.4}, p = {:.4} indicates significant departure from Gaussian normality; inspect the Normal Q-Q and Normality & Lognormality Tests tabs.",
                        sw.statistic, sw.p_value
                    ),
                ));
            } else {
                checklist.push(checklist_item(
                    format!("Normality ({})", s.label),
                    ChecklistStatus::Pass,
                    Some(format!("Shapiro-Wilk W = {:.4}, p = {:.4}", sw.statistic, sw.p_value)),
                    "No significant departure from Gaussian normality was detected.",
                ));
            }
        }
    }
}

fn append_one_sample_tab(
    res: &mut ResultSheet,
    raw_series: &[IndexedRawSeries],
    mu0: f64,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> Result<(), StatsError> {
    let headers: Vec<String> = raw_series.iter().map(|s| s.label.clone()).collect();
    let row_labels = vec![
        String::from("Hypothetical value"),
        String::from("t statistic"),
        String::from("df"),
        String::from("One-sample t P"),
        String::from("Wilcoxon signed-rank W"),
        String::from("Wilcoxon P"),
    ];
    let mut rows = vec![Vec::with_capacity(raw_series.len()); row_labels.len()];
    for series in raw_series {
        let vals = &series.values;
        rows[0].push(num_cell(mu0));
        if let Ok(t_res) = one_sample_t_test(vals, mu0, true, conf) {
            rows[1].push(num_cell(t_res.t_statistic));
            rows[2].push(num_cell(t_res.df));
            rows[3].push(p_cell(t_res.p_value, p_fmt));
        } else {
            rows[1].push(ResultCell::Empty);
            rows[2].push(ResultCell::Empty);
            rows[3].push(ResultCell::Empty);
        }
        if let Ok(w_res) = one_sample_wilcoxon(vals, mu0, true, conf) {
            rows[4].push(num_cell(w_res.w_stat));
            rows[5].push(p_cell(w_res.p_value, p_fmt));
        } else {
            rows[4].push(ResultCell::Empty);
            rows[5].push(ResultCell::Empty);
        }
    }
    res.tabs.push(build_tab("one-sample", "One-Sample Tests", ResultTabKind::MainSummary, headers, row_labels, rows));
    Ok(())
}

/// Execute [`OutlierSpec`] on a [`DataSheet`] and emit a [`ResultSheet`].
pub fn run_outlier_analysis(sheet: &DataSheet, spec: &AnalysisSpec, outlier_spec: OutlierSpec) -> Result<ResultSheet, StatsError> {
    let mut res = init_result_sheet(spec)?;
    let extracted = extract_indexed_series_for_descriptive(sheet, &spec.selected_data_sets, SubcolumnAggregation::PooledGrandSample)?;
    let raw_series = extracted.raw;
    if raw_series.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let headers = vec![
        String::from("Column"),
        String::from("Row Index (1-based)"),
        String::from("Value"),
        String::from("Statistic"),
        String::from("Threshold / P"),
    ];
    let mut row_labels = Vec::new();
    let mut rows = Vec::new();
    let mut total_flagged = 0_usize;

    for series in &raw_series {
        let ds_idx = Some(series.data_set_index);
        let vals = &series.values;
        let flags = match outlier_spec {
            OutlierSpec::Grubbs { alpha, iterative } => grubbs_outliers(vals, alpha, iterative)?.outliers,
            OutlierSpec::Rout { fdr_q_percent } => {
                let r = rout_outliers(vals, fdr_q_percent)?;
                push_scalar(&mut res, "RobustCenter", ds_idx, r.robust_center);
                push_scalar(&mut res, "RSDR", ds_idx, r.rsdr);
                r.outliers
            }
        };
        push_scalar(&mut res, "OutlierCount", ds_idx, flags.len() as f64);
        total_flagged += flags.len();
        for f in flags {
            let table_row_1based = series.row_indices.get(f.index).copied().unwrap_or(f.index) + 1;
            let stat_cell = if f.score.is_infinite() { ResultCell::Text(String::from("∞")) } else { num_cell(f.score) };
            row_labels.push(format!("Outlier {}", rows.len() + 1));
            rows.push(vec![
                ResultCell::Text(series.label.clone()),
                num_cell(table_row_1based as f64),
                num_cell(f.value),
                stat_cell,
                num_cell(f.threshold_or_p),
            ]);
        }
    }

    res.tabs.push(build_tab("outliers", "Identified Outliers", ResultTabKind::MainSummary, headers, row_labels, rows));
    res.checklist.push(checklist_item(
        "Outlier scan",
        if total_flagged > 0 { ChecklistStatus::Info } else { ChecklistStatus::Pass },
        Some(format!("{total_flagged} flagged")),
        if total_flagged > 0 {
            "Observations exceeded the configured outlier threshold and are listed in the Identified Outliers tab; experimental provenance determines data validity."
        } else {
            "No observations exceeded the configured outlier threshold."
        },
    ));
    append_ignored_text_checklist_and_footnote(&mut res, "outlier analysis", &extracted.ignored_non_numeric);
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}
