//! Column descriptive statistics, exact binomial median CIs, normality & lognormality tests,
//! outlier identification (Grubbs & ROUT), and one-sample hypothesis tests.
//!
//! References (D1 citation pass 2026-11-18; other routines cite their sources inline):
//! - Chan, T. F., Golub, G. H., & LeVeque, R. J. (1983). Algorithms for computing the sample
//!   variance: Analysis and recommendations. *The American Statistician*, 37(3), 242-247.
//!   `doi:10.1080/00031305.1983.10483115`.
//! - Student (1908). The probable error of a mean. *Biometrika*, 6(1), 1-25.
//!   `doi:10.1093/biomet/6.1.1`.
//! - Kirkwood, T. B. L. (1979). Geometric means and measures of dispersion. *Biometrics*, 35,
//!   908-909 (letter; JSTOR 2530139).
//! - Bland, J. M., & Altman, D. G. (1996). Transformations, means, and confidence intervals. *BMJ*,
//!   312(7038), 1079. `doi:10.1136/bmj.312.7038.1079`.
//! - Lam, F. C., Hung, C. T., & Perrier, D. G. (1985). Estimation of variance for harmonic mean
//!   half-lives. *Journal of Pharmaceutical Sciences*, 74(2), 229-231.
//!   `doi:10.1002/jps.2600740229`.
//! - Casella, G., & Berger, R. L. (2002). *Statistical Inference* (2nd ed.). Duxbury / Thomson
//!   Learning. §9.2.1 (monotone transformation equivariance of confidence intervals).
//! - Bullen, P. S. (2003). *Handbook of Means and Their Inequalities* (2nd ed.). Kluwer Academic /
//!   Springer. Ch. III (generalized power means: harmonic, geometric, arithmetic, and quadratic/RMS).
//!   `doi:10.1007/978-94-017-0399-4`.
//! - Blom, G. (1958). *Statistical Estimates and Transformed Beta-Variables*. Stockholm: Almqvist &
//!   Wiksell; New York: Wiley.
//! - D'Agostino, R. B. (1970). Transformation to normality of the null distribution of `g1`.
//!   *Biometrika*, 57(3), 679-681. `doi:10.1093/biomet/57.3.679`.
//! - Anscombe, F. J., & Glynn, W. J. (1983). Distribution of the kurtosis statistic `b2` for normal
//!   samples. *Biometrika*, 70(1), 227-234. `doi:10.1093/biomet/70.1.227`.
//! - D'Agostino, R. B., Belanger, A., & D'Agostino, R. B., Jr. (1990). A suggestion for using
//!   powerful and informative tests of normality. *The American Statistician*, 44(4), 316-321.
//!   `doi:10.1080/00031305.1990.10475751`.
//! - Grubbs, F. E. (1969). Procedures for detecting outlying observations in samples.
//!   *Technometrics*, 11(1), 1-21. `doi:10.1080/00401706.1969.10490657`.
//! - Rosner, B. (1983). Percentage points for a generalized ESD many-outlier procedure.
//!   *Technometrics*, 25(2), 165-172. `doi:10.1080/00401706.1983.10487848`.
//! - Motulsky, H. J., & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear
//!   regression – a new method based on robust nonlinear regression and the false discovery rate.
//!   *BMC Bioinformatics*, 7, 123. `doi:10.1186/1471-2105-7-123`.
//! - Benjamini, Y., & Hochberg, Y. (1995). Controlling the false discovery rate: A practical and
//!   powerful approach to multiple testing. *Journal of the Royal Statistical Society, Series B*,
//!   57(1), 289-300. `doi:10.1111/j.2517-6161.1995.tb02031.x`.
//! - Yeh, K. C., & Kwan, K. C. (1978). A comparison of numerical integrating algorithms by
//!   trapezoidal, Lagrange, and spline approximation. *Journal of Pharmacokinetics and
//!   Biopharmaceutics*, 6(1), 79-98. `doi:10.1007/bf01066064`.
//! - Stephens, M. A. (1986). Tests based on EDF statistics. In R. B. D'Agostino & M. A. Stephens
//!   (Eds.), *Goodness-of-Fit Techniques*, Ch. 4, pp. 97-193. Marcel Dekker.
//! - Hollander, M., Wolfe, D. A., & Chicken, E. (2014). *Nonparametric Statistical Methods* (3rd ed.).
//!   Wiley. Ch. 3 (signed-rank Hodges–Lehmann pseudomedian & Walsh-average confidence intervals).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, DataSheet, DescriptiveSpec, OutlierSpec, PValueFormat, ResultCell, ResultSheet,
    ResultTab, ResultTabKind, SubcolumnAggregation, SubcolumnMode, SummarizedPoint, SummaryPointError, TableArchetype, VariableRole,
};
use curvebook_math::dist::{
    chi2_isf, chi2_ppf, lilliefors_d_from_sorted, lilliefors_p_value, norm_cdf, norm_ppf, norm_sf, student_t_cdf, student_t_sf, student_t_two_sided_p,
};
use curvebook_math::noncentral_t_ci;
use curvebook_math::special::{gamma::gamma, inc_beta::reg_beta};
use curvebook_math::sum::{NeumaierSum, SummaryMoments, lag1_autocorrelation};
use curvebook_math::{ConfidenceLevel, MathError};
use serde::{Deserialize, Serialize};

use crate::common::{
    TabBuilder, append_ignored_text_checklist_and_footnote, checklist_item, enforce_summary_mode_matrix, init_result_sheet,
    is_ignored_non_numeric_cell, num_cell, opt_ci_cell, opt_num_cell, p_cell_at_alpha, push_scalar, quantile_sorted, quantile_type6_sorted,
    quartiles_hinge, rank_with_ties, sanitize_result_sheet_floats, select_kth_upper_triangle_half_sum, sorted_finite, t_crit_two_sided, text_cell,
    try_resolve_selected_indices, validate_finite_slice,
};
use crate::error::StatsError;

// ============================================================================
// Exact Binomial Median Confidence Interval
// ============================================================================

/// Nonparametric exact order-statistic confidence interval for the median (Conover 1999).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MedianCi {
    /// Lower confidence endpoint equal to order statistic `x_{(lower_rank_1based)}`.
    pub lower: f64,
    /// Upper confidence endpoint equal to order statistic `x_{(upper_rank_1based)}`.
    pub upper: f64,
    /// One-based rank index of `lower` in the sorted ascending sample (`>= 1`).
    pub lower_rank_1based: usize,
    /// One-based rank index of `upper` in the sorted ascending sample (`<= n`).
    pub upper_rank_1based: usize,
    /// Requested nominal coverage percentage (e.g. `95.0` for a 95% confidence interval).
    pub target_confidence_percent: f64,
    /// Exact discrete binomial achieved coverage percentage `>= target_confidence_percent` (`100 * (1 - 2 * P(Bin(n, 0.5) <= r))`).
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
    /// Column or dataset title label.
    pub label: String,
    /// Count of active finite numeric observations included (`n >= 1`).
    pub n: usize,
    /// Count of explicitly excluded cells in the column not included in `n`.
    pub excluded_count: usize,
    /// Minimum observed value.
    pub min: f64,
    /// Lower quartile (25th percentile) computed with Hyndman & Fan (1996) Type 6 (`p(n + 1)` linear interpolation).
    pub q1: f64,
    /// Sample median (50th percentile).
    pub median: f64,
    /// Upper quartile (75th percentile) computed with Hyndman & Fan (1996) Type 6 (`p(n + 1)` linear interpolation).
    pub q3: f64,
    /// Maximum observed value.
    pub max: f64,
    /// Interquartile range `q3 - q1` in original units.
    pub iqr: f64,
    /// Full sample span `max - min` in original units.
    pub range: f64,
    /// Lower symmetric custom percentile `(percentile_percent, value)` corresponding to `100 - upper`, computed via Hyndman & Fan (1996) Type 7 continuous linear interpolation (`None` when `custom_upper_percentile` was omitted).
    pub custom_lower_percentile: Option<(f64, f64)>,
    /// Upper custom percentile `(percentile_percent, value)`, computed via Hyndman & Fan (1996) Type 7 continuous linear interpolation (`None` when `custom_upper_percentile` was omitted).
    pub custom_upper_percentile: Option<(f64, f64)>,
    /// Arithmetic sum of all active values.
    pub sum: f64,
    /// Arithmetic sample mean $\bar{x} = \frac{1}{n}\sum x_i$.
    pub mean: f64,
    /// Sample standard deviation with Bessel `n - 1` denominator (`None` when `n < 2`).
    pub sd: Option<f64>,
    /// Exact two-sided chi-square confidence interval `(lower, upper)` for population SD on `n - 1` df at configured `ConfidenceLevel` (`None` when `n < 2` or zero variance).
    #[serde(default)]
    pub sd_ci: Option<(f64, f64)>,
    /// Standard error of the mean `sd / sqrt(n)` (`None` when `n < 2`).
    pub sem: Option<f64>,
    /// Two-sided Student *t* confidence interval `mean +/- t_{alpha/2, n-1} * sem` (`None` when `n < 2`).
    pub mean_ci: Option<(f64, f64)>,
    /// Exact binomial order-statistic confidence interval for the median (`None` when `n < 3`).
    pub median_ci: Option<MedianCi>,
    /// Percent coefficient of variation `100 * sd / |mean|` (`None` when `n < 2` or `mean == 0.0`).
    pub cv_percent: Option<f64>,
    /// Geometric mean $\exp(\frac{1}{n}\sum \ln x_i)$ (`None` if any value is `<= 0.0`).
    pub geometric_mean: Option<f64>,
    /// Multiplicative geometric standard deviation factor $\exp(\mathrm{sd}(\ln x)) \ge 1$ (`None` when `n < 2` or any value `<= 0.0`).
    pub geometric_sd_factor: Option<f64>,
    /// Two-sided exponentiated Student *t* confidence interval on log scale for `geometric_mean` (`None` when `n < 2` or any value `<= 0.0`).
    pub geometric_mean_ci: Option<(f64, f64)>,
    /// Harmonic mean $n / \sum (1 / x_i)$ (Bullen 2003; `None` when any value `<= 0.0` or reciprocal sum is zero).
    pub harmonic_mean: Option<f64>,
    /// Two-sided Student *t* confidence interval for the population harmonic mean `1 / E[1/X]`,
    /// obtained by forming the Student (1908) *t* interval `[L, U]` on the reciprocal-transformed
    /// sample `1 / x_i` and monotonically back-transforming via `[1 / U, 1 / L]` (Bland & Altman 1996;
    /// Lam, Hung & Perrier 1985; Casella & Berger 2002).
    ///
    /// Returns `None` when `n < 2`, any observation is `<= 0.0`, the reciprocal-scale interval
    /// touches or crosses zero (`L <= 0.0`, where `u -> 1 / u` is singular), or bounds overflow.
    pub harmonic_mean_ci: Option<(f64, f64)>,
    /// Quadratic mean (root-mean-square) $\sqrt{\frac{1}{n}\sum x_i^2} \ge 0$ (Bullen 2003).
    pub quadratic_mean: f64,
    /// Two-sided monotone back-transformed Student *t* confidence interval for the population
    /// quadratic mean (RMS) `sqrt(E[X^2])`, obtained by forming the Student (1908) *t* interval
    /// `[L_sq, U_sq]` on the squared observations `x_i^2` and back-transforming via
    /// `[sqrt(max(L_sq, 0)), sqrt(U_sq)]` (Bland & Altman 1996; Casella & Berger 2002; Bullen 2003).
    ///
    /// Unlike a linearized delta-method interval (`rms +/- t * SE(x^2) / (2 * rms)`), this monotone
    /// endpoint back-transformation preserves the non-negative domain `sqrt(E[X^2]) >= 0` (clamping
    /// the lower endpoint to `0.0` whenever `L_sq <= 0.0`) and preserves coverage for `sqrt(E[X^2])`
    /// whenever `[L_sq, U_sq]` covers `E[X^2]`. Returns `None` when `n < 2`, all observations are
    /// `0.0` (`quadratic_mean == 0.0`), or bounds overflow.
    pub quadratic_mean_ci: Option<(f64, f64)>,
    /// Sample Fisher–Pearson skewness `G_1` (Joanes & Gill 1998 Type 2 unbiased adjustment; `None` when `n < 3` or zero SD).
    pub skewness: Option<f64>,
    /// Sample excess kurtosis `G_2` (Joanes & Gill 1998 Type 2 unbiased adjustment; `None` when `n < 4` or zero SD).
    pub excess_kurtosis: Option<f64>,
    /// Sample lag-1 serial autocorrelation `r_1` in `[-1, 1]` along row order (`None` when `n < 3` or zero variance).
    pub lag1_autocorrelation: Option<f64>,
}

/// Compute comprehensive descriptive statistics on raw finite observations.
///
/// Moments use the compensated pairwise algorithm of Chan, Golub & LeVeque (1983); quartiles
/// (`q1`, `median`, `q3`) use Hyndman & Fan (1996) Definition 6, optional custom symmetric
/// percentiles use Hyndman & Fan (1996) Definition 7, the mean CI is the Student (1908) *t*
/// interval, the SD CI is the chi-square interval on `n - 1` df, and the generalized power-mean
/// CIs (`geometric_mean_ci`, `harmonic_mean_ci`, `quadratic_mean_ci`; Bullen 2003) are monotone
/// back-transformations of the Student (1908) *t* interval on `ln x`, `1 / x`, and `x^2`
/// respectively (Kirkwood 1979; Lam, Hung & Perrier 1985; Bland & Altman 1996; Casella & Berger 2002).
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

struct IndexedSpreadOnlySeries {
    data_set_index: usize,
    label: String,
    mode: SubcolumnMode,
    /// `(entered_mean, entered_spread)` pairs from `MeanSd` or `MeanSem` (no sample size `N`).
    rows: Vec<(f64, Option<f64>)>,
}

struct IndexedIntervalSummarySeries {
    data_set_index: usize,
    label: String,
    mode: SubcolumnMode,
    /// `(center, sub1, sub2)` tuples from `MeanLowHigh` (`[Mean, -Error, +Error]`),
    /// `UpperLowerLimits` (`[Mean, Lower, Upper]`), or `MedianIqr` (`[Median, Q1, Q3]`).
    rows: Vec<(f64, Option<f64>, Option<f64>)>,
}

struct DescriptiveSeriesExtraction {
    raw: Vec<IndexedRawSeries>,
    pooled: Vec<IndexedPooledSeries>,
    no_n_summary: Vec<IndexedSpreadOnlySeries>,
    interval_summary: Vec<IndexedIntervalSummarySeries>,
    ignored_non_numeric: Vec<(String, usize)>,
}

fn extract_spread_only_series(
    data_sets: &[curvebook_doc::DataSetColumn],
    indices: &[usize],
    mode: SubcolumnMode,
) -> Result<Vec<IndexedSpreadOnlySeries>, StatsError> {
    let mut out = Vec::new();
    for &idx in indices {
        let ds = &data_sets[idx];
        let mut rows = Vec::new();
        for row in &ds.cells {
            let Some(mean) = row.first().and_then(|c| c.active_number()) else {
                continue;
            };
            let spread = row.get(1).and_then(|c| c.active_number());
            if let Some(s) = spread
                && s < 0.0
            {
                return Err(StatsError::SummaryPoint(SummaryPointError::NegativeSpread { spread: s }));
            }
            rows.push((mean, spread));
        }
        if !rows.is_empty() {
            out.push(IndexedSpreadOnlySeries { data_set_index: idx, label: ds.title.clone(), mode, rows });
        }
    }
    Ok(out)
}

fn extract_interval_summary_series(
    data_sets: &[curvebook_doc::DataSetColumn],
    indices: &[usize],
    mode: SubcolumnMode,
) -> Result<Vec<IndexedIntervalSummarySeries>, StatsError> {
    let mut out = Vec::new();
    for &idx in indices {
        let ds = &data_sets[idx];
        let mut rows = Vec::new();
        for row in &ds.cells {
            let Some(center) = row.first().and_then(|c| c.active_number()) else {
                continue;
            };
            let sub1 = row.get(1).and_then(|c| c.active_number());
            let sub2 = row.get(2).and_then(|c| c.active_number());
            if mode == SubcolumnMode::MeanLowHigh {
                if let Some(down) = sub1
                    && down < 0.0
                {
                    return Err(StatsError::SummaryPoint(SummaryPointError::NegativeSpread { spread: down }));
                }
                if let Some(up) = sub2
                    && up < 0.0
                {
                    return Err(StatsError::SummaryPoint(SummaryPointError::NegativeSpread { spread: up }));
                }
            }
            rows.push((center, sub1, sub2));
        }
        if !rows.is_empty() {
            out.push(IndexedIntervalSummarySeries { data_set_index: idx, label: ds.title.clone(), mode, rows });
        }
    }
    Ok(out)
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

/// Student (1908) *t* interval `mean +/- t_{alpha/2, n-1} * SEM`.
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
    if !s.is_finite() || s <= 0.0 {
        return Ok(None);
    }
    let half_alpha = 0.5 * confidence.alpha();
    let upper_chi2 = chi2_isf(half_alpha, df)?;
    let lower_chi2 = chi2_ppf(half_alpha, df)?;
    if upper_chi2 > 0.0 && lower_chi2 > 0.0 { Ok(Some((s * (df / upper_chi2).sqrt(), s * (df / lower_chi2).sqrt()))) } else { Ok(None) }
}

type GeomStats = (Option<f64>, Option<f64>, Option<(f64, f64)>);

/// Geometric mean, geometric SD factor `exp(sd(ln x))` (Kirkwood 1979), and the geometric-mean CI
/// obtained by back-transforming the Student *t* interval of `ln x`.
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

/// Harmonic mean `n / sum(1 / x_i)` (Bullen 2003) and its reciprocal-scale back-transformed
/// Student *t* CI `[1 / U_inv, 1 / L_inv]` for `1 / E[1 / X]` (Bland & Altman 1996; Lam, Hung &
/// Perrier 1985; Casella & Berger 2002).
///
/// Reciprocals are scaled by `min_v = min(x_i) > 0` (`u_i = min_v / x_i in (0, 1]`) so extreme
/// magnitudes never underflow or overflow. Returns `(None, None)` if any observation is `<= 0.0`,
/// and `ci = None` when `n < 2` or the reciprocal-scale CI lower endpoint `L_inv <= 0.0`.
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

/// Quadratic mean (RMS) `sqrt(mean(x_i^2))` (Bullen 2003) and its exact monotone back-transformed
/// Student *t* CI `[sqrt(max(L_sq, 0)), sqrt(U_sq)]` for `sqrt(E[X^2])` (Bland & Altman 1996;
/// Casella & Berger 2002).
///
/// Observations are scaled by `max_abs = max(|x_i|)` (`w_i = (x_i / max_abs)^2 in [0, 1]`) before
/// squaring to avoid overflow/underflow. Because `E[X^2] >= 0`, any negative lower endpoint
/// `L_sq < 0` on the squared scale is clamped at `0.0` before taking the square root, preserving
/// non-negativity and nominal coverage. Returns `(0.0, None)` when all values are `0.0`, and
/// `ci = None` when `n < 2`.
fn compute_quadratic_stats(values: &[f64], confidence: ConfidenceLevel) -> Result<(f64, Option<(f64, f64)>), StatsError> {
    let n = values.len();
    let max_abs = values.iter().fold(0.0_f64, |acc, &v| acc.max(v.abs()));
    if max_abs == 0.0 {
        return Ok((0.0, None));
    }
    let sqs: Vec<f64> = values.iter().map(|v| (v / max_abs).powi(2)).collect();
    let sm = SummaryMoments::from_slice(&sqs)?;
    let rms = max_abs * sm.mean.max(0.0).sqrt();
    if !rms.is_finite() {
        return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "quadratic_mean", value: rms }));
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
    /// Column or dataset title label.
    pub label: String,
    /// Number of input summarized rows (`points`) pooled together.
    pub point_count: usize,
    /// Exact grand replicate count $\sum_i n_i$ reconstructed across all summarized rows.
    pub total_n: f64,
    /// Pooled weighted grand mean $\sum_i n_i \bar{x}_i / \sum_i n_i$.
    pub mean: f64,
    /// Exact pooled sample SD combining within-point (`(n_i - 1) SD_i^2`) and between-point (`n_i (mean_i - grand)^2`) sums of squares (`None` when `total_n <= 1.0`).
    pub sd: Option<f64>,
    /// Two-sided chi-square confidence interval for population SD on `total_n - 1` df at configured `ConfidenceLevel` (`None` when `total_n <= 1.0` or zero variance).
    #[serde(default)]
    pub sd_ci: Option<(f64, f64)>,
    /// Standard error of the pooled mean `sd / sqrt(total_n)` (`None` when `total_n <= 1.0`).
    pub sem: Option<f64>,
    /// Two-sided Student *t* confidence interval on `total_n - 1` df (`None` when `total_n <= 1.0`).
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
            let mut indices = try_resolve_selected_indices(selected, t.variables.len())?;
            if selected.is_empty() {
                indices.retain(|&idx| !matches!(t.variables[idx].role, VariableRole::Categorical | VariableRole::Label));
            }
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
            Ok(DescriptiveSeriesExtraction {
                raw,
                pooled: Vec::new(),
                no_n_summary: Vec::new(),
                interval_summary: Vec::new(),
                ignored_non_numeric: ignored,
            })
        }
        other => Err(StatsError::IncompatibleTable { analysis: "descriptive statistics", actual: other.kind() }),
    }
}

fn extract_column_table_for_desc(t: &curvebook_doc::ColumnTable, selected: &[usize]) -> Result<DescriptiveSeriesExtraction, StatsError> {
    let indices = try_resolve_selected_indices(selected, t.data_sets.len())?;
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
        return Ok(DescriptiveSeriesExtraction {
            raw: Vec::new(),
            pooled,
            no_n_summary: Vec::new(),
            interval_summary: Vec::new(),
            ignored_non_numeric: Vec::new(),
        });
    }
    if matches!(t.subcolumn_mode, SubcolumnMode::MeanSd | SubcolumnMode::MeanSem) {
        let no_n_summary = extract_spread_only_series(&t.data_sets, &indices, t.subcolumn_mode)?;
        return Ok(DescriptiveSeriesExtraction {
            raw: Vec::new(),
            pooled: Vec::new(),
            no_n_summary,
            interval_summary: Vec::new(),
            ignored_non_numeric: Vec::new(),
        });
    }
    if matches!(t.subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr) {
        let interval_summary = extract_interval_summary_series(&t.data_sets, &indices, t.subcolumn_mode)?;
        return Ok(DescriptiveSeriesExtraction {
            raw: Vec::new(),
            pooled: Vec::new(),
            no_n_summary: Vec::new(),
            interval_summary,
            ignored_non_numeric: Vec::new(),
        });
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
    Ok(DescriptiveSeriesExtraction { raw, pooled: Vec::new(), no_n_summary: Vec::new(), interval_summary: Vec::new(), ignored_non_numeric: ignored })
}

fn extract_xy_table_for_desc(
    t: &curvebook_doc::XyTable,
    selected: &[usize],
    aggregation: SubcolumnAggregation,
) -> Result<DescriptiveSeriesExtraction, StatsError> {
    let indices = try_resolve_selected_indices(selected, t.data_sets.len())?;
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
        return Ok(DescriptiveSeriesExtraction {
            raw: Vec::new(),
            pooled,
            no_n_summary: Vec::new(),
            interval_summary: Vec::new(),
            ignored_non_numeric: Vec::new(),
        });
    }
    if matches!(t.y_subcolumn_mode, SubcolumnMode::MeanSd | SubcolumnMode::MeanSem) {
        let no_n_summary = extract_spread_only_series(&t.data_sets, &indices, t.y_subcolumn_mode)?;
        return Ok(DescriptiveSeriesExtraction {
            raw: Vec::new(),
            pooled: Vec::new(),
            no_n_summary,
            interval_summary: Vec::new(),
            ignored_non_numeric: Vec::new(),
        });
    }
    if matches!(t.y_subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr) {
        let interval_summary = extract_interval_summary_series(&t.data_sets, &indices, t.y_subcolumn_mode)?;
        return Ok(DescriptiveSeriesExtraction {
            raw: Vec::new(),
            pooled: Vec::new(),
            no_n_summary: Vec::new(),
            interval_summary,
            ignored_non_numeric: Vec::new(),
        });
    }
    let raw = aggregate_replicate_datasets(&t.data_sets, &indices, aggregation);
    let ignored_non_numeric = collect_ignored_non_numeric(&raw);
    Ok(DescriptiveSeriesExtraction { raw, pooled: Vec::new(), no_n_summary: Vec::new(), interval_summary: Vec::new(), ignored_non_numeric })
}

fn extract_grouped_table_for_desc(
    t: &curvebook_doc::GroupedTable,
    selected: &[usize],
    aggregation: SubcolumnAggregation,
) -> Result<DescriptiveSeriesExtraction, StatsError> {
    let indices = try_resolve_selected_indices(selected, t.data_sets.len())?;
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
        return Ok(DescriptiveSeriesExtraction {
            raw: Vec::new(),
            pooled,
            no_n_summary: Vec::new(),
            interval_summary: Vec::new(),
            ignored_non_numeric: Vec::new(),
        });
    }
    if matches!(t.subcolumn_mode, SubcolumnMode::MeanSd | SubcolumnMode::MeanSem) {
        let no_n_summary = extract_spread_only_series(&t.data_sets, &indices, t.subcolumn_mode)?;
        return Ok(DescriptiveSeriesExtraction {
            raw: Vec::new(),
            pooled: Vec::new(),
            no_n_summary,
            interval_summary: Vec::new(),
            ignored_non_numeric: Vec::new(),
        });
    }
    if matches!(t.subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr) {
        let interval_summary = extract_interval_summary_series(&t.data_sets, &indices, t.subcolumn_mode)?;
        return Ok(DescriptiveSeriesExtraction {
            raw: Vec::new(),
            pooled: Vec::new(),
            no_n_summary: Vec::new(),
            interval_summary,
            ignored_non_numeric: Vec::new(),
        });
    }
    let raw = aggregate_replicate_datasets(&t.data_sets, &indices, aggregation);
    let ignored_non_numeric = collect_ignored_non_numeric(&raw);
    Ok(DescriptiveSeriesExtraction { raw, pooled: Vec::new(), no_n_summary: Vec::new(), interval_summary: Vec::new(), ignored_non_numeric })
}

fn aggregate_replicate_datasets(
    data_sets: &[curvebook_doc::DataSetColumn],
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
    /// Human-readable test name (e.g. `"D'Agostino-Pearson"`, `"Shapiro-Wilk"`, `"Anderson-Darling"`, `"Kolmogorov-Smirnov (Lilliefors)"`).
    pub test_name: String,
    /// Computed test statistic (`K^2`, `W`, adjusted `A^2*`, or `D`).
    pub statistic: f64,
    /// Test *P* value testing the null hypothesis of Gaussian (or lognormal when on log data) distribution.
    pub p_value: f64,
    /// True when `p_value > alpha` at the caller-supplied significance level `alpha` (does not reject normality).
    pub passed: bool,
}

/// Complete four-test normality battery plus optional lognormal comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalityBatteryResult {
    /// Number of finite active values evaluated (`n`).
    pub n: usize,
    /// D'Agostino–Pearson omnibus $K^2$ test on raw values (`None` when `n < 8`).
    pub dagostino_pearson: Option<NormalityTestOutcome>,
    /// Royston (1992/1995) Shapiro–Wilk *W* test on raw values (`None` when `n < 3` or `n > 5000`).
    pub shapiro_wilk: Option<NormalityTestOutcome>,
    /// Stephens (1986) Case 3 sample-size-adjusted Anderson–Darling $A^{2*}$ test on raw values (`None` when `n < 8`).
    pub anderson_darling: Option<NormalityTestOutcome>,
    /// Dallal–Wilkinson (1986) / Lilliefors (1967) Kolmogorov–Smirnov *D* test with composite parameters on raw values (`None` when `n < 5`).
    pub kolmogorov_smirnov: Option<NormalityTestOutcome>,
    /// D'Agostino–Pearson $K^2$ test evaluated on $\ln(x_i)$ (`None` when `n < 8` or any value `<= 0.0`).
    pub lognormal_dagostino_pearson: Option<NormalityTestOutcome>,
    /// Shapiro–Wilk *W* test evaluated on $\ln(x_i)$ (`None` when `n < 3`, `n > 5000`, or any value `<= 0.0`).
    pub lognormal_shapiro_wilk: Option<NormalityTestOutcome>,
    /// Anderson–Darling $A^{2*}$ test evaluated on $\ln(x_i)$ (`None` when `n < 8` or any value `<= 0.0`).
    pub lognormal_anderson_darling: Option<NormalityTestOutcome>,
    /// Lilliefors KS *D* test evaluated on $\ln(x_i)$ (`None` when `n < 5` or any value `<= 0.0`).
    pub lognormal_kolmogorov_smirnov: Option<NormalityTestOutcome>,
    /// `ln(L_normal / L_lognormal)`: positive favors normal, negative favors lognormal (`None` when `n < 3`, non-positive data, or degenerate variance).
    pub normal_vs_lognormal_log_lr: Option<f64>,
}

/// D'Agostino–Pearson omnibus `K^2` normality test (`n >= 8`).
///
/// `K^2 = Z(sqrt b1)^2 + Z(b2)^2` referred to chi-square on 2 df (D'Agostino, Belanger &
/// D'Agostino 1990), using the skewness transform of D'Agostino (1970) and the kurtosis transform of
/// Anscombe & Glynn (1983).
pub fn dagostino_pearson_k2(values: &[f64], alpha: f64) -> Result<NormalityTestOutcome, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n < 8 {
        return Err(StatsError::InsufficientSampleSize { context: "D'Agostino-Pearson K^2", required: 8, actual: n });
    }
    let res = curvebook_math::normality::dagostino_pearson(values).map_err(|err| match err {
        MathError::DomainError { name: "variance", .. } => StatsError::ZeroVariance { context: "D'Agostino-Pearson K^2" },
        other => StatsError::Math(other),
    })?;
    Ok(NormalityTestOutcome {
        test_name: String::from("D'Agostino-Pearson K2"),
        statistic: res.statistic,
        p_value: res.p_value,
        passed: res.p_value > alpha,
    })
}

/// Shapiro–Wilk `W` normality test (`3 <= n <= 5000`) using Shapiro & Wilk (1965) and
/// Royston (1992; 1995 AS R94).
pub fn shapiro_wilk(values: &[f64], alpha: f64) -> Result<NormalityTestOutcome, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Shapiro-Wilk", required: 3, actual: n });
    }
    if n > 5000 {
        return Err(StatsError::InvalidParameter { name: "n", value: n as f64 });
    }
    let res = curvebook_math::normality::shapiro_wilk(values).map_err(|err| match err {
        MathError::DomainError { name: "variance", .. } => StatsError::ZeroVariance { context: "Shapiro-Wilk" },
        other => StatsError::Math(other),
    })?;
    Ok(NormalityTestOutcome {
        test_name: String::from("Shapiro-Wilk W"),
        statistic: res.statistic,
        p_value: res.p_value,
        passed: res.p_value > alpha,
    })
}

/// Anderson–Darling normality test (`n >= 8`) with Stephens small-sample modification `A^{*2}`.
pub fn anderson_darling(values: &[f64], alpha: f64) -> Result<NormalityTestOutcome, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n < 8 {
        return Err(StatsError::InsufficientSampleSize { context: "Anderson-Darling", required: 8, actual: n });
    }
    let res = curvebook_math::normality::anderson_darling(values).map_err(|err| match err {
        MathError::DomainError { name: "sd", .. } => StatsError::ZeroVariance { context: "Anderson-Darling" },
        other => StatsError::Math(other),
    })?;
    Ok(NormalityTestOutcome {
        test_name: String::from("Anderson-Darling A2*"),
        statistic: res.statistic,
        p_value: res.p_value,
        passed: res.p_value > alpha,
    })
}

/// Kolmogorov–Smirnov normality test (`n >= 5`) using the clean-room Lilliefors (1967) /
/// Stephens (1974) distribution in [`curvebook_math::dist::lilliefors_p_value`].
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
///
/// Plotting positions are Blom's (1958) `(i - 3/8) / (n + 1/4)` for 1-based rank `i`.
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
/// Single outlier record flagged by Grubbs' ESD or column ROUT detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlierFlag {
    /// Zero-based index of the flagged observation in the input `values` slice.
    pub index: usize,
    /// Numeric value of the flagged observation.
    pub value: f64,
    /// Outlier test score (`G` for Grubbs' test; Student *t* ratio `|r_i| / RSDR` for ROUT).
    pub score: f64,
    /// Critical score threshold (`G_crit` for Grubbs) or raw two-sided *P* value (for ROUT).
    pub threshold_or_p: f64,
}

/// Output of Grubbs' ESD outlier test (Grubbs 1969).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrubbsResult {
    /// Total number of finite input observations originally examined (`n >= 3`).
    pub n: usize,
    /// Two-sided significance threshold `alpha` in `(0, 1)` used to compute Grubbs critical values.
    pub alpha: f64,
    /// True for sequential iterative outlier removal; false for single-outlier detection.
    pub iterative: bool,
    /// Observations flagged as statistical outliers in detection order.
    pub outliers: Vec<OutlierFlag>,
    /// Remaining non-outlier observations preserving original relative order.
    pub cleaned_values: Vec<f64>,
}

/// Grubbs' extreme studentized deviate test (`n >= 3`, single or iterative two-sided).
///
/// Grubbs (1969) statistic `G = max |x_i - mean| / s` against the two-sided critical value
/// `((n - 1) / sqrt(n)) sqrt(t^2 / (n - 2 + t^2))`, `t = t_{alpha/(2n), n-2}`. Iterative mode removes
/// the flagged value and repeats on the reduced sample until a test is not significant; it is the
/// sequential Grubbs procedure, not Rosner's (1983) generalized ESD.
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
        let t_crit = curvebook_math::dist::student_t_isf(alpha / (2.0 * nf), nf - 2.0)?;
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

/// Output of the standalone column ROUT outlier test (`K = 1`; Motulsky & Brown 2006).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutResult {
    /// Number of input observations analyzed (`n >= 3`).
    pub n: usize,
    /// Target Benjamini–Hochberg false discovery rate `Q` in percent (`(0, 100]`, e.g. `1.0` for 1%).
    pub fdr_q_percent: f64,
    /// Robust Cauchy/Lorentzian IRLS location estimate (`mu_robust`).
    pub robust_center: f64,
    /// Robust Standard Deviation of Residuals (`RSDR`, 68.27th percentile scaled by `n / (n - 1)`).
    pub rsdr: f64,
    /// Observations rejected by the BH step-up procedure at FDR `Q`.
    pub outliers: Vec<OutlierFlag>,
    /// Non-outlier observations preserving original input order.
    pub cleaned_values: Vec<f64>,
}

/// Standalone column ROUT outlier detection (`K = 1` robust Cauchy/Lorentzian IRLS location +
/// `P_68.27` RSDR + Benjamini–Hochberg FDR step-up at `fdr_q_percent`).
///
/// Motulsky & Brown (2006) ROUT with the Benjamini & Hochberg (1995) step-up rule.
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
    let mut center = quantile_type6_sorted(&sorted, 0.5).or_else(|| sorted.first().copied()).ok_or(StatsError::EmptyInput)?;
    let abs_from_center = |c: f64| -> Vec<f64> { values.iter().map(|&v| (v - c).abs()).collect() };
    let init_scale = curvebook_math::rout::compute_rsdr(&abs_from_center(center), 1);

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
            let next_rsdr = curvebook_math::rout::compute_rsdr(&abs_from_center(center), 1);
            if next_rsdr > 0.0 {
                scale = next_rsdr;
            }
            if shift <= 1e-12 * scale.max(1.0) {
                break;
            }
        }
    }

    let residuals: Vec<f64> = values.iter().map(|&v| v - center).collect();
    let core = curvebook_math::rout::rout_detect_outliers_core(&residuals, 1, fdr_q_percent, 0.0)?;
    let mut flagged = core.outlier_indices;
    flagged.sort_by(|&a, &b| residuals[a].abs().total_cmp(&residuals[b].abs()));
    let outliers = flagged
        .into_iter()
        .map(|orig_idx| OutlierFlag {
            index: orig_idx,
            value: values[orig_idx],
            score: core.t_stats[orig_idx],
            threshold_or_p: core.p_values[orig_idx],
        })
        .collect();
    let cleaned_values = values.iter().enumerate().filter_map(|(i, &v)| (!core.is_outlier[i]).then_some(v)).collect();
    Ok(RoutResult { n, fdr_q_percent, robust_center: center, rsdr: core.rsdr, outliers, cleaned_values })
}

// ============================================================================
// One-Sample Hypothesis Tests
// ============================================================================

/// Output of a one-sample Student *t*-test against `hypothetical_mean` (Student 1908).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OneSampleTResult {
    /// Sample size `n` (may be rounded from effective `n_total` for summary inputs; `n >= 2`).
    pub n: usize,
    /// Null-hypothesis reference value $\mu_0$.
    pub hypothetical_mean: f64,
    /// Observed sample arithmetic mean $\bar{x}$.
    pub sample_mean: f64,
    /// Mean discrepancy $\bar{x} - \mu_0$ in sample units.
    pub discrepancy: f64,
    /// Sample standard deviation (`n - 1` denominator) in sample units.
    pub sd: f64,
    /// Standard error of the mean `sd / sqrt(n)` in sample units.
    pub sem: f64,
    /// Student *t* statistic `discrepancy / sem`.
    pub t_statistic: f64,
    /// Degrees of freedom `n - 1` (`>= 1.0`).
    pub df: f64,
    /// Student *t*(df) *P* value (`two_tailed` two-sided vs directional one-sided matching `t_statistic` sign).
    pub p_value: f64,
    /// True when `p_value` is two-sided; false when directional one-tailed.
    pub two_tailed: bool,
    /// Two-sided `t(df)` confidence interval `(lower, upper)` for `discrepancy = mean - mu0` at configured `ConfidenceLevel`.
    pub ci: (f64, f64),
    /// Standardized Cohen's $d = (\bar{x} - \mu_0) / \mathrm{sd}$ (Cohen 1988).
    pub cohens_d: f64,
    /// Bias-corrected Hedges' $g = J(\mathrm{df}) \cdot d$ (Hedges 1981); `None` when `df <= 1.0` (`n = 2`, where `Gamma((df-1)/2)` has a pole).
    pub hedges_g: Option<f64>,
    /// Exact noncentral-*t* two-sided CI for `hedges_g` scaled by $J(\mathrm{df})$ (Steiger & Fouladi 1997); `None` when `hedges_g` is `None` or noncentral-*t* inversion fails.
    pub hedges_g_ci: Option<(f64, f64)>,
}

/// One-sample Student *t*-test against `hypothetical_mean` (Student 1908).
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
    let inv_sqrt_n = 1.0 / (n as f64).sqrt();
    let (g, g_ci) = hedges_g_with_ci(d, t_stat, df, inv_sqrt_n, confidence.percent())?;

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

/// Compute Hedges' small-sample corrected `g = J(df) * d` and its noncentral-*t* CI when `df > 1.0`.
pub(crate) type HedgesEstimate = (Option<f64>, Option<(f64, f64)>);

/// Returns `(None, None)` when `df <= 1.0` because `Gamma((df - 1)/2)` is undefined at its `<= 0` poles.
pub(crate) fn hedges_g_with_ci(d: f64, t_stat: f64, df: f64, inv_sqrt_n: f64, conf_percent: f64) -> Result<HedgesEstimate, MathError> {
    if df <= 1.0 {
        return Ok((None, None));
    }
    let j = hedges_j_factor(df)?;
    let g = j * d;
    let g_ci = noncentral_t_ci(t_stat, df, conf_percent).ok().map(|(lo, hi)| (lo * inv_sqrt_n * j, hi * inv_sqrt_n * j));
    Ok((Some(g), g_ci))
}

/// Result of a one-sample Wilcoxon signed-rank test against hypothetical median `mu0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OneSampleWilcoxonResult {
    /// Total number of active observations supplied before dropping exact zero differences `x_i == mu0`.
    pub n_total: usize,
    /// Number of non-zero differences `|x_i - mu0| > 0` retained in signed-rank ranking.
    pub n_nonzero: usize,
    /// Hypothesized null median $\mu_0$.
    pub hypothetical_median: f64,
    /// Sample median of all `n_total` observations.
    pub sample_median: f64,
    /// Sum of positive signed ranks `W+`.
    pub w_plus: f64,
    /// Sum of negative signed ranks `W-`.
    pub w_minus: f64,
    /// Signed-rank summary statistic (`W+` vs `W-` directional/minimum per exact DP or normal approximation).
    pub w_stat: f64,
    /// *P* value testing $H_0: \text{median} = \mu_0$ (two-sided when `two_tailed == true`, else directional one-tailed via exact DP for `n_nonzero <= 25` without ties or tie-/continuity-corrected normal).
    pub p_value: f64,
    /// True if exact dynamic-programming signed-rank tail distribution was used (`n_nonzero <= 25` without ties); false for asymptotic normal.
    pub exact_p_used: bool,
    /// True for two-sided *P* value; false for directional one-sided test.
    pub two_tailed: bool,
    /// One-sample Hodges–Lehmann pseudomedian estimator `median_{(i <= j)}((x_i + x_j) / 2)`.
    pub pseudomedian_hl: f64,
    /// Two-sided Walsh-average confidence interval `(lower, upper)` for the pseudomedian at configured `ConfidenceLevel` (`None` if insufficient non-zero pairs).
    pub pseudomedian_ci: Option<(f64, f64)>,
    /// Exact achieved confidence percentage when small-sample (`n <= 30`, untied) exact signed-rank critical values were used (`None` for asymptotic normal or unavailable CIs).
    #[serde(default)]
    pub ci_actual_confidence_percent: Option<f64>,
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
    let sample_median = quantile_type6_sorted(&sorted, 0.5).or_else(|| sorted.first().copied()).ok_or(StatsError::EmptyInput)?;
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
            ci_actual_confidence_percent: None,
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

    let (pseudomedian_hl, pseudomedian_ci, ci_actual_confidence_percent) = hodges_lehmann_one_sample_with_coverage(values, confidence)?;
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
        ci_actual_confidence_percent,
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

/// Exact distribution-free two-sided Wilcoxon signed-rank order-statistic index `k` (1-based)
/// and achieved coverage fraction for an untied sample of size `n <= 30` (Hollander, Wolfe & Chicken 2014, Ch. 3.3).
pub(crate) fn wilcoxon_exact_hl_ci_rank(n: usize, confidence: ConfidenceLevel) -> (usize, f64) {
    let m = n * (n + 1) / 2;
    let half_m = m / 2;
    let mut ways = vec![0_u64; half_m + 1];
    ways[0] = 1;
    for rank in 1..=n {
        for s in (rank..=half_m).rev() {
            ways[s] += ways[s - rank];
        }
    }
    let total = (1_u64 << n) as f64;
    let target = confidence.percent() / 100.0;
    let mut cum = ways[0];
    let mut best_k = 1_usize;
    let mut best_cov = (1.0 - 2.0 * (cum as f64) / total).clamp(0.0, 1.0);
    if best_cov + 1e-14 < target {
        return (1, best_cov);
    }
    for k in 2..=half_m {
        cum += ways[k - 1];
        let cov = (1.0 - 2.0 * (cum as f64) / total).clamp(0.0, 1.0);
        if cov + 1e-14 >= target {
            best_k = k;
            best_cov = cov;
        } else {
            break;
        }
    }
    (best_k, best_cov)
}

/// Upper bound on achievable two-sided signed-rank CI coverage (%) for sample size `n <= 30`.
pub(crate) fn wilcoxon_max_achievable_coverage_percent(n: usize) -> Option<f64> {
    if (1..=30).contains(&n) {
        let total = (1_u64 << n) as f64;
        Some(100.0 * (1.0 - 2.0 / total).clamp(0.0, 1.0))
    } else {
        None
    }
}

/// Point estimate, two-sided Hodges–Lehmann CI, and achieved small-sample confidence percentage.
pub(crate) type HodgesLehmannWithCoverage = (f64, Option<(f64, f64)>, Option<f64>);

/// Hodges–Lehmann one-sample pseudomedian (median of Walsh averages `(x_i + x_j) / 2, i <= j`),
/// confidence interval, and exact achieved confidence percentage when `n <= 30` without ties.
pub(crate) fn hodges_lehmann_one_sample_with_coverage(values: &[f64], confidence: ConfidenceLevel) -> Result<HodgesLehmannWithCoverage, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n > 2_000 {
        let sorted = sorted_finite(values);
        let m_u64 = (n as u64) * ((n as u64) + 1) / 2;
        let hl = if m_u64 % 2 == 1 {
            select_kth_upper_triangle_half_sum(&sorted, m_u64 / 2)?
        } else {
            let mid_lo = select_kth_upper_triangle_half_sum(&sorted, m_u64 / 2 - 1)?;
            let mid_hi = select_kth_upper_triangle_half_sum(&sorted, m_u64 / 2)?;
            0.5 * mid_lo + 0.5 * mid_hi
        };
        let z = curvebook_math::dist::norm_two_sided_isf(confidence.alpha())?;
        let nf = n as f64;
        let mu = nf * (nf + 1.0) / 4.0;
        let sd = (nf * (nf + 1.0) * (2.0 * nf + 1.0) / 24.0).sqrt();
        let k = (mu - z * sd).floor().max(1.0) as u64;
        let lo_idx = k.saturating_sub(1).min(m_u64 - 1);
        let hi_idx = (m_u64 - k).min(m_u64 - 1);
        let ci_lo = select_kth_upper_triangle_half_sum(&sorted, lo_idx)?;
        let ci_hi = select_kth_upper_triangle_half_sum(&sorted, hi_idx)?;
        return Ok((hl, Some((ci_lo, ci_hi)), None));
    }

    let m = n * (n + 1) / 2;
    let mut walsh = Vec::with_capacity(m);
    for i in 0..n {
        for j in i..n {
            walsh.push(0.5 * values[i] + 0.5 * values[j]);
        }
    }
    walsh.sort_by(f64::total_cmp);
    let hl = quantile_type6_sorted(&walsh, 0.5).or_else(|| walsh.first().copied()).ok_or(StatsError::EmptyInput)?;
    if n < 4 {
        return Ok((hl, None, None));
    }
    let sorted_vals = sorted_finite(values);
    let untied = sorted_vals.windows(2).all(|w| w[0] < w[1]);
    let (k, actual_conf_pct) = if n <= 30 && untied {
        let (exact_k, cov) = wilcoxon_exact_hl_ci_rank(n, confidence);
        (exact_k, Some(100.0 * cov))
    } else {
        let z = curvebook_math::dist::norm_two_sided_isf(confidence.alpha())?;
        let nf = n as f64;
        let mu = nf * (nf + 1.0) / 4.0;
        let sd = (nf * (nf + 1.0) * (2.0 * nf + 1.0) / 24.0).sqrt();
        ((mu - z * sd).floor().max(1.0) as usize, None)
    };
    let lo_idx = k.saturating_sub(1).min(m - 1);
    let hi_idx = (m - k).min(m - 1);
    Ok((hl, Some((walsh[lo_idx], walsh[hi_idx])), actual_conf_pct))
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn hodges_lehmann_one_sample(values: &[f64], confidence: ConfidenceLevel) -> Result<(f64, Option<(f64, f64)>), StatsError> {
    let (hl, ci, _) = hodges_lehmann_one_sample_with_coverage(values, confidence)?;
    Ok((hl, ci))
}

/// Kolmogorov–Smirnov one-sample test against a known theoretical `Normal(mean, sd^2)` at `alpha = 0.05`.
///
/// See [`one_sample_ks_normal_at_alpha`] to specify a custom significance threshold `alpha`.
pub fn one_sample_ks_normal(values: &[f64], mean: f64, sd: f64) -> Result<NormalityTestOutcome, StatsError> {
    one_sample_ks_normal_at_alpha(values, mean, sd, 0.05)
}

/// Kolmogorov–Smirnov one-sample test against a known theoretical `Normal(mean, sd^2)` at threshold `alpha`.
pub fn one_sample_ks_normal_at_alpha(values: &[f64], mean: f64, sd: f64, alpha: f64) -> Result<NormalityTestOutcome, StatsError> {
    validate_finite_slice(values)?;
    if !mean.is_finite() {
        return Err(StatsError::InvalidParameter { name: "mean", value: mean });
    }
    if !sd.is_finite() || sd <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "sd", value: sd });
    }
    if !alpha.is_finite() || alpha <= 0.0 || alpha >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "alpha", value: alpha });
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
    Ok(NormalityTestOutcome { test_name: String::from("One-sample KS (Normal)"), statistic: d, p_value, passed: p_value > alpha })
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
    enforce_summary_mode_matrix(sheet, &AnalysisKind::DescriptiveStats(desc.clone()))?;
    if let Some(mu0) = desc.hypothetical_value
        && !mu0.is_finite()
    {
        return Err(StatsError::InvalidParameter { name: "hypothetical_value", value: mu0 });
    }
    let mut res = init_result_sheet(spec)?;
    let extracted = extract_indexed_series_for_descriptive(sheet, &spec.selected_data_sets, desc.aggregation)?;
    let raw_series = extracted.raw;
    let pooled_series = extracted.pooled;
    let no_n_series = extracted.no_n_summary;
    let interval_series = extracted.interval_summary;
    if raw_series.is_empty() && pooled_series.is_empty() && no_n_series.is_empty() && interval_series.is_empty() {
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
    if !no_n_series.is_empty() {
        populate_spread_only_descriptive_sheet(&mut res, &no_n_series)?;
        sanitize_result_sheet_floats(&mut res)?;
        return Ok(res);
    }
    if !interval_series.is_empty() {
        populate_interval_summary_descriptive_sheet(&mut res, &interval_series)?;
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
        res.tabs.push(build_normality_tab(&summaries, &batteries, p_fmt, conf.alpha()));
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

/// Compute per-dataset linear trapezoidal area under curve (Yeh & Kwan 1978)
/// (`sum_i 0.5 * (x_{i+1} - x_i) * (y_i + y_{i+1})`)
/// for [`TableArchetype::Xy`] when at least 2 distinct sorted finite `(x, mean_y)` knots exist.
/// Tied `X` rows are collapsed to their replicate-weighted mean `Y` before integration.
fn append_xy_trapezoidal_auc(res: &mut ResultSheet, sheet: &DataSheet, selected: &[usize]) -> Result<(), StatsError> {
    let TableArchetype::Xy(xy) = &sheet.archetype else {
        return Ok(());
    };
    let ds_indices = try_resolve_selected_indices(selected, xy.data_sets.len())?;
    for ds_idx in ds_indices {
        let ds = &xy.data_sets[ds_idx];
        let mut raw_pts: Vec<(f64, f64, f64)> = Vec::new();
        for r in 0..xy.row_count() {
            let Some(x_raw) = xy.x_summary_at(r)? else { continue };
            if !x_raw.is_finite() {
                continue;
            }
            let x = if x_raw == 0.0 { 0.0 } else { x_raw };
            if xy.y_subcolumn_mode.supports_parametric_moments() {
                if let Some(pt) = xy.summarized_y_point(r, ds_idx)?
                    && pt.mean.is_finite()
                    && pt.n.is_finite()
                    && pt.n > 0.0
                {
                    raw_pts.push((x, pt.mean, pt.n));
                }
            } else if xy.y_subcolumn_mode.is_pre_summarized() {
                if let Some(center) = ds.cells.get(r).and_then(|row| row.first()).and_then(|c| c.active_number())
                    && center.is_finite()
                {
                    raw_pts.push((x, center, 1.0));
                }
            } else if let Some(row) = ds.cells.get(r) {
                for y in row.iter().filter_map(|c| c.active_number()) {
                    raw_pts.push((x, y, 1.0));
                }
            }
        }
        raw_pts.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut knots: Vec<(f64, f64)> = Vec::new();
        let mut i = 0_usize;
        while i < raw_pts.len() {
            let x_k = raw_pts[i].0;
            let mut j = i + 1;
            while j < raw_pts.len() && raw_pts[j].0 == x_k {
                j += 1;
            }
            let mut w_acc = NeumaierSum::new();
            for &(_, _, w) in &raw_pts[i..j] {
                w_acc.add(w);
            }
            let tot_w = w_acc.total();
            let mut y_acc = NeumaierSum::new();
            if tot_w > 0.0 && tot_w.is_finite() {
                for &(_, y, w) in &raw_pts[i..j] {
                    y_acc.add((w / tot_w) * y);
                }
            }
            knots.push((x_k, y_acc.total()));
            i = j;
        }

        if knots.len() >= 2 {
            let mut auc_acc = NeumaierSum::new();
            for w in knots.windows(2) {
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
            let indices = try_resolve_selected_indices(selected, mv.variables.len())?;
            let active_cols: Vec<usize> = indices
                .into_iter()
                .filter(|&idx| {
                    let var = &mv.variables[idx];
                    !matches!(var.role, VariableRole::Categorical | VariableRole::Label) && var.cells.iter().any(|c| c.active_number().is_some())
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

    let mut tab = TabBuilder::new("row-descriptive-stats", "Row-wise descriptive statistics", ResultTabKind::Diagnostics)
        .headers(["N", "Mean", "SD", "Min", "Max"]);
    let mut row_count = 0_usize;
    for (label, vals) in row_entries {
        if vals.is_empty() {
            continue;
        }
        let moments = SummaryMoments::from_slice(&vals)?;
        let sd = if vals.len() >= 2 { moments.sample_sd().ok() } else { None };
        let min_v = vals.iter().copied().fold(f64::INFINITY, f64::min);
        let max_v = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        tab.push_row(label, [num_cell(vals.len() as f64), num_cell(moments.mean), opt_num_cell(sd), num_cell(min_v), num_cell(max_v)]);
        row_count += 1;
    }

    if row_count > 0 {
        res.tabs.push(tab.build());
    }
    Ok(())
}

fn collect_dataset_rows_for_desc(
    data_sets: &[curvebook_doc::DataSetColumn],
    row_titles: &[String],
    selected: &[usize],
    aggregation: SubcolumnAggregation,
) -> Result<Vec<(String, Vec<f64>)>, StatsError> {
    let indices = try_resolve_selected_indices(selected, data_sets.len())?;
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
    let mut tab = TabBuilder::new("normal-qq", "Normal & Lognormal Q-Q Quantiles", ResultTabKind::NormalQq).headers([
        "Data Set",
        "Order Index",
        "Normal Expected",
        "Normal Observed",
        "Lognormal Expected (ln)",
        "Lognormal Observed ln(Y)",
    ]);
    let mut row_count = 0_usize;

    for series in raw_series {
        let label = &series.label;
        let vals = &series.values;
        if vals.is_empty() {
            continue;
        }
        let normal_pairs = qq_expected_quantiles(vals, false)?;
        let lognormal_pairs = qq_expected_quantiles(vals, true).ok();
        if lognormal_pairs.is_none() {
            tab.push_footnote(format!("{label}: lognormal Q-Q coordinates omitted because one or more values are <= 0."));
        }
        for (idx, &(exp_n, obs_n)) in normal_pairs.iter().enumerate() {
            let (exp_ln, obs_ln) = match lognormal_pairs.as_ref().and_then(|v| v.get(idx)) {
                Some(&(e, o)) => (Some(e), Some(o)),
                None => (None, None),
            };
            tab.push_row(
                format!("{label} #{}", idx + 1),
                [text_cell(label.clone()), num_cell((idx + 1) as f64), num_cell(exp_n), num_cell(obs_n), opt_num_cell(exp_ln), opt_num_cell(obs_ln)],
            );
            row_count += 1;
        }
    }

    Ok((row_count > 0).then(|| tab.build()))
}

fn populate_pooled_descriptive_sheet(res: &mut ResultSheet, pooled_series: &[IndexedPooledSeries], conf: ConfidenceLevel) -> Result<(), StatsError> {
    let mut pooled = Vec::with_capacity(pooled_series.len());
    for ps in pooled_series {
        let p = summarize_pre_summarized_points(&ps.label, &ps.points, conf)?;
        let ds_idx = Some(ps.data_set_index);
        push_scalar(res, "Mean", ds_idx, p.mean);
        if let Some(sd) = p.sd {
            push_scalar(res, "SD", ds_idx, sd);
        }
        pooled.push(p);
    }
    res.tabs.push(
        TabBuilder::new("desc-pooled", "Descriptive Statistics (Pooled)", ResultTabKind::DescriptiveSummary)
            .headers(pooled.iter().map(|p| p.label.as_str()))
            .row("Entered summary rows", pooled.iter().map(|p| num_cell(p.point_count as f64)))
            .row("Total N", pooled.iter().map(|p| num_cell(p.total_n)))
            .row("Pooled Mean", pooled.iter().map(|p| num_cell(p.mean)))
            .row("Pooled SD", pooled.iter().map(|p| opt_num_cell(p.sd)))
            .row("Pooled SEM", pooled.iter().map(|p| opt_num_cell(p.sem)))
            .row(format!("{}% CI of Mean", conf.percent()), pooled.iter().map(|p| opt_ci_cell(p.mean_ci)))
            .build(),
    );
    res.checklist.push(checklist_item(
        "Pre-summarized input mode",
        ChecklistStatus::Info,
        None,
        "Moments were reconstructed from entered Mean, Spread, and N; quantiles and normality tests require raw replicates.",
    ));
    Ok(())
}

fn populate_spread_only_descriptive_sheet(res: &mut ResultSheet, series: &[IndexedSpreadOnlySeries]) -> Result<(), StatsError> {
    let mode = series.first().map_or(SubcolumnMode::MeanSd, |s| s.mode);
    let spread_row_label = match mode {
        SubcolumnMode::MeanSem => "Entered SEM (single row)",
        _ => "Entered SD (single row)",
    };
    let spread_scalar_key = match mode {
        SubcolumnMode::MeanSem => "SEM",
        _ => "SD",
    };

    struct SpreadOnlySummary {
        label: String,
        row_count: usize,
        min_mean: f64,
        median_mean: f64,
        max_mean: f64,
        mean_of_means: f64,
        single_row_spread: Option<f64>,
    }

    let mut summaries = Vec::with_capacity(series.len());
    for s in series {
        let means: Vec<f64> = s.rows.iter().map(|(m, _)| *m).collect();
        validate_finite_slice(&means)?;
        let sorted = sorted_finite(&means);
        let (_, median_mean, _) = quartiles_hinge(&sorted).ok_or(StatsError::EmptyInput)?;
        let moments = SummaryMoments::from_slice(&means)?;
        let single_row_spread = if s.rows.len() == 1 { s.rows[0].1 } else { None };
        let ds_idx = Some(s.data_set_index);
        push_scalar(res, "Mean", ds_idx, moments.mean);
        if let Some(sp) = single_row_spread {
            push_scalar(res, spread_scalar_key, ds_idx, sp);
        }
        summaries.push(SpreadOnlySummary {
            label: s.label.clone(),
            row_count: s.rows.len(),
            min_mean: sorted[0],
            median_mean,
            max_mean: sorted[sorted.len() - 1],
            mean_of_means: moments.mean,
            single_row_spread,
        });
    }

    res.tabs.push(
        TabBuilder::new("desc-summary-no-n", "Descriptive Statistics (No N)", ResultTabKind::DescriptiveSummary)
            .headers(summaries.iter().map(|s| s.label.as_str()))
            .row("Entered summary rows", summaries.iter().map(|s| num_cell(s.row_count as f64)))
            .row("Minimum (entered means)", summaries.iter().map(|s| num_cell(s.min_mean)))
            .row("Median (entered means)", summaries.iter().map(|s| num_cell(s.median_mean)))
            .row("Maximum (entered means)", summaries.iter().map(|s| num_cell(s.max_mean)))
            .row("Mean (entered / unweighted)", summaries.iter().map(|s| num_cell(s.mean_of_means)))
            .row(spread_row_label, summaries.iter().map(|s| opt_num_cell(s.single_row_spread)))
            .footnote(
                "Sample size N is not recorded in MeanSd/MeanSem subcolumns; pooled variance across multiple rows, SD/SEM conversion, confidence intervals, quantiles, and normality tests require N or raw replicates.",
            )
            .build(),
    );
    res.checklist.push(checklist_item(
        "Pre-summarized input without N",
        ChecklistStatus::Info,
        Some(format!("{mode:?}")),
        "Reported entered means and single-row spread without fabricating N; pooled variance, CIs, and inferential tests require N or raw replicates.",
    ));
    Ok(())
}

/// Populate descriptive statistics for pre-summarized interval/limit/quartile columns
/// (`MeanLowHigh`, `UpperLowerLimits`, and `MedianIqr`; Zar 2010, Ch. 3–4; Tukey 1977).
///
/// Never fabricates sample size `N`, `SD`, `SEM`, or parametric confidence intervals; for
/// `MedianIqr` the entered central value is reported strictly as a median rather than a mean.
fn populate_interval_summary_descriptive_sheet(res: &mut ResultSheet, series: &[IndexedIntervalSummarySeries]) -> Result<(), StatsError> {
    let mode = series.first().map_or(SubcolumnMode::MeanLowHigh, |s| s.mode);

    struct IntervalSummaryRow {
        label: String,
        row_count: usize,
        min_center: f64,
        median_center: f64,
        max_center: f64,
        mean_center: f64,
        single_center: Option<f64>,
        single_sub1: Option<f64>,
        single_sub2: Option<f64>,
        single_lower: Option<f64>,
        single_upper: Option<f64>,
        single_width: Option<f64>,
    }

    let mut summaries = Vec::with_capacity(series.len());
    for s in series {
        let centers: Vec<f64> = s.rows.iter().map(|(c, _, _)| *c).collect();
        validate_finite_slice(&centers)?;
        let sorted = sorted_finite(&centers);
        let (_, median_center, _) = quartiles_hinge(&sorted).ok_or(StatsError::EmptyInput)?;
        let moments = SummaryMoments::from_slice(&centers)?;
        let ds_idx = Some(s.data_set_index);

        let (single_center, single_sub1, single_sub2, single_lower, single_upper, single_width) = if s.rows.len() == 1 {
            let (c, a, b) = s.rows[0];
            match mode {
                SubcolumnMode::MeanLowHigh => {
                    let lo = a.map(|down| c - down).filter(|v| v.is_finite());
                    let hi = b.map(|up| c + up).filter(|v| v.is_finite());
                    let width = match (a, b) {
                        (Some(down), Some(up)) if (down + up).is_finite() => Some(down + up),
                        _ => None,
                    };
                    (Some(c), a, b, lo, hi, width)
                }
                SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
                    let (lo, hi) = match (a, b) {
                        (Some(v1), Some(v2)) => (Some(v1.min(v2)), Some(v1.max(v2))),
                        other => other,
                    };
                    let width = match (lo, hi) {
                        (Some(v1), Some(v2)) if (v2 - v1).is_finite() => Some(v2 - v1),
                        _ => None,
                    };
                    (Some(c), lo, hi, lo, hi, width)
                }
                _ => (Some(c), a, b, None, None, None),
            }
        } else {
            (None, None, None, None, None, None)
        };

        match mode {
            SubcolumnMode::MedianIqr => {
                push_scalar(res, "Median", ds_idx, single_center.unwrap_or(median_center));
                if let Some(q1) = single_lower {
                    push_scalar(res, "Q1", ds_idx, q1);
                }
                if let Some(q3) = single_upper {
                    push_scalar(res, "Q3", ds_idx, q3);
                }
                if let Some(iqr) = single_width {
                    push_scalar(res, "IQR", ds_idx, iqr);
                }
            }
            SubcolumnMode::UpperLowerLimits => {
                push_scalar(res, "Mean", ds_idx, moments.mean);
                if let Some(lo) = single_lower {
                    push_scalar(res, "LowerLimit", ds_idx, lo);
                }
                if let Some(hi) = single_upper {
                    push_scalar(res, "UpperLimit", ds_idx, hi);
                }
                if let Some(width) = single_width {
                    push_scalar(res, "IntervalWidth", ds_idx, width);
                }
            }
            _ => {
                push_scalar(res, "Mean", ds_idx, moments.mean);
                if let Some(lo) = single_lower {
                    push_scalar(res, "LowerBound", ds_idx, lo);
                }
                if let Some(hi) = single_upper {
                    push_scalar(res, "UpperBound", ds_idx, hi);
                }
                if let Some(width) = single_width {
                    push_scalar(res, "IntervalWidth", ds_idx, width);
                }
            }
        }

        summaries.push(IntervalSummaryRow {
            label: s.label.clone(),
            row_count: s.rows.len(),
            min_center: sorted[0],
            median_center,
            max_center: sorted[sorted.len() - 1],
            mean_center: moments.mean,
            single_center,
            single_sub1,
            single_sub2,
            single_lower,
            single_upper,
            single_width,
        });
    }

    match mode {
        SubcolumnMode::MedianIqr => {
            res.tabs.push(
                TabBuilder::new("desc-summary-median-iqr", "Descriptive Statistics (Median & IQR)", ResultTabKind::DescriptiveSummary)
                    .headers(summaries.iter().map(|s| s.label.as_str()))
                    .row("Entered summary rows", summaries.iter().map(|s| num_cell(s.row_count as f64)))
                    .row("Minimum (entered medians)", summaries.iter().map(|s| num_cell(s.min_center)))
                    .row("Median (entered medians)", summaries.iter().map(|s| num_cell(s.median_center)))
                    .row("Maximum (entered medians)", summaries.iter().map(|s| num_cell(s.max_center)))
                    .row("Entered Median (single row)", summaries.iter().map(|s| opt_num_cell(s.single_center)))
                    .row("Entered 25% Percentile / Q1 (single row)", summaries.iter().map(|s| opt_num_cell(s.single_lower)))
                    .row("Entered 75% Percentile / Q3 (single row)", summaries.iter().map(|s| opt_num_cell(s.single_upper)))
                    .row("Interquartile Range (IQR, single row)", summaries.iter().map(|s| opt_num_cell(s.single_width)))
                    .footnote(
                        "MedianIqr subcolumns store nonparametric quartiles [Median, Q1, Q3] without sample size N; arithmetic mean, SD, SEM, confidence intervals, normality tests, and inferential hypothesis tests require raw replicates or parametric Mean/SD/N summaries.",
                    )
                    .build(),
            );
        }
        SubcolumnMode::UpperLowerLimits => {
            res.tabs.push(
                TabBuilder::new("desc-summary-limits", "Descriptive Statistics (Upper / Lower Limits)", ResultTabKind::DescriptiveSummary)
                    .headers(summaries.iter().map(|s| s.label.as_str()))
                    .row("Entered summary rows", summaries.iter().map(|s| num_cell(s.row_count as f64)))
                    .row("Minimum (entered means)", summaries.iter().map(|s| num_cell(s.min_center)))
                    .row("Median (entered means)", summaries.iter().map(|s| num_cell(s.median_center)))
                    .row("Maximum (entered means)", summaries.iter().map(|s| num_cell(s.max_center)))
                    .row("Mean (entered / unweighted)", summaries.iter().map(|s| num_cell(s.mean_center)))
                    .row("Entered Lower limit (single row)", summaries.iter().map(|s| opt_num_cell(s.single_lower)))
                    .row("Entered Upper limit (single row)", summaries.iter().map(|s| opt_num_cell(s.single_upper)))
                    .row("Interval width (single row)", summaries.iter().map(|s| opt_num_cell(s.single_width)))
                    .footnote(
                        "UpperLowerLimits subcolumns store explicit bounds [Mean, Lower, Upper] without sample size N or parametric SD; pooled variance, CIs, and inferential tests require N and SD/SEM/%CV or raw replicates.",
                    )
                    .build(),
            );
        }
        _ => {
            res.tabs.push(
                TabBuilder::new("desc-summary-interval", "Descriptive Statistics (Asymmetric Error Bounds)", ResultTabKind::DescriptiveSummary)
                    .headers(summaries.iter().map(|s| s.label.as_str()))
                    .row("Entered summary rows", summaries.iter().map(|s| num_cell(s.row_count as f64)))
                    .row("Minimum (entered means)", summaries.iter().map(|s| num_cell(s.min_center)))
                    .row("Median (entered means)", summaries.iter().map(|s| num_cell(s.median_center)))
                    .row("Maximum (entered means)", summaries.iter().map(|s| num_cell(s.max_center)))
                    .row("Mean (entered / unweighted)", summaries.iter().map(|s| num_cell(s.mean_center)))
                    .row("Entered -Error (single row)", summaries.iter().map(|s| opt_num_cell(s.single_sub1)))
                    .row("Entered +Error (single row)", summaries.iter().map(|s| opt_num_cell(s.single_sub2)))
                    .row("Lower bound (single row)", summaries.iter().map(|s| opt_num_cell(s.single_lower)))
                    .row("Upper bound (single row)", summaries.iter().map(|s| opt_num_cell(s.single_upper)))
                    .row("Interval width (single row)", summaries.iter().map(|s| opt_num_cell(s.single_width)))
                    .footnote(
                        "MeanLowHigh subcolumns store asymmetric offsets [Mean, -Error, +Error] without sample size N or parametric SD; pooled variance, CIs, and inferential tests require N and SD/SEM/%CV or raw replicates.",
                    )
                    .build(),
            );
        }
    }

    res.checklist.push(checklist_item(
        "Pre-summarized interval/quartile input",
        ChecklistStatus::Info,
        Some(format!("{mode:?}")),
        "Reported entered central location and explicit interval/quartile bounds without fabricating N, SD, SEM, or inferential tests.",
    ));
    Ok(())
}

fn build_descriptive_tab(summaries: &[DescriptiveSummary], conf: ConfidenceLevel) -> ResultTab {
    let custom_pair = summaries.iter().find_map(|s| match (s.custom_lower_percentile, s.custom_upper_percentile) {
        (Some((p_lo, _)), Some((p_hi, _))) => Some((p_lo, p_hi)),
        _ => None,
    });
    let pct = conf.percent();
    let mut tab = TabBuilder::new("desc-summary", "Descriptive Statistics", ResultTabKind::DescriptiveSummary)
        .headers(summaries.iter().map(|s| s.label.as_str()))
        .row("Number of values", summaries.iter().map(|s| num_cell(s.n as f64)))
        .row("Excluded values", summaries.iter().map(|s| num_cell(s.excluded_count as f64)))
        .row("Minimum", summaries.iter().map(|s| num_cell(s.min)))
        .row("25% Percentile", summaries.iter().map(|s| num_cell(s.q1)))
        .row("Median", summaries.iter().map(|s| num_cell(s.median)))
        .row("75% Percentile", summaries.iter().map(|s| num_cell(s.q3)))
        .row("Maximum", summaries.iter().map(|s| num_cell(s.max)))
        .row("Range", summaries.iter().map(|s| num_cell(s.range)))
        .row("IQR", summaries.iter().map(|s| num_cell(s.iqr)))
        .row("Mean", summaries.iter().map(|s| num_cell(s.mean)))
        .row("Std. Deviation", summaries.iter().map(|s| opt_num_cell(s.sd)))
        .row("CI of SD", summaries.iter().map(|s| opt_ci_cell(s.sd_ci)))
        .row("Std. Error of Mean", summaries.iter().map(|s| opt_num_cell(s.sem)))
        .row(format!("{pct}% CI of Mean"), summaries.iter().map(|s| opt_ci_cell(s.mean_ci)))
        .row(format!("{pct}% CI of Median"), summaries.iter().map(|s| opt_ci_cell(s.median_ci.map(|m| (m.lower, m.upper)))))
        .row("Median CI actual confidence (%)", summaries.iter().map(|s| opt_num_cell(s.median_ci.map(|m| m.actual_confidence_percent))))
        .row("Coefficient of variation (%)", summaries.iter().map(|s| opt_num_cell(s.cv_percent)))
        .row("Geometric mean", summaries.iter().map(|s| opt_num_cell(s.geometric_mean)))
        .row("Geometric SD factor", summaries.iter().map(|s| opt_num_cell(s.geometric_sd_factor)))
        .row(format!("{pct}% CI of Geometric mean"), summaries.iter().map(|s| opt_ci_cell(s.geometric_mean_ci)))
        .row("Harmonic mean", summaries.iter().map(|s| opt_num_cell(s.harmonic_mean)))
        .row(format!("{pct}% CI of Harmonic mean"), summaries.iter().map(|s| opt_ci_cell(s.harmonic_mean_ci)))
        .row("Quadratic mean (RMS)", summaries.iter().map(|s| num_cell(s.quadratic_mean)))
        .row(format!("{pct}% CI of Quadratic mean (RMS)"), summaries.iter().map(|s| opt_ci_cell(s.quadratic_mean_ci)))
        .row("Skewness", summaries.iter().map(|s| opt_num_cell(s.skewness)))
        .row("Excess kurtosis", summaries.iter().map(|s| opt_num_cell(s.excess_kurtosis)))
        .row("Sum", summaries.iter().map(|s| num_cell(s.sum)));
    if let Some((p_lo, p_hi)) = custom_pair {
        tab.push_row(format!("{p_lo}% Percentile (Type 7)"), summaries.iter().map(|s| opt_num_cell(s.custom_lower_percentile.map(|(_, v)| v))))
            .push_row(format!("{p_hi}% Percentile (Type 7)"), summaries.iter().map(|s| opt_num_cell(s.custom_upper_percentile.map(|(_, v)| v))))
            .push_footnote(format!(
                "Standard quartiles (25%, Median, 75%) use Hyndman & Fan (1996) Type 6; custom percentiles ({p_lo}%, {p_hi}%) use Hyndman & Fan (1996) Type 7 continuous linear interpolation."
            ));
    }
    tab.footnote(format!(
        "{pct}% CI of SD uses exact chi-square variance bounds under a normal-theory Gaussian assumption; skewed or heavy-tailed distributions are sensitive to non-normality."
    ))
    .build()
}

fn build_normality_tab(summaries: &[DescriptiveSummary], batteries: &[NormalityBatteryResult], p_fmt: PValueFormat, alpha: f64) -> ResultTab {
    let stat_cells = |f: fn(&NormalityBatteryResult) -> Option<&NormalityTestOutcome>| {
        batteries.iter().map(move |b| f(b).map_or(ResultCell::Empty, |o| num_cell(o.statistic)))
    };
    let p_cells = |f: fn(&NormalityBatteryResult) -> Option<&NormalityTestOutcome>| {
        batteries.iter().map(move |b| f(b).map_or(ResultCell::Empty, |o| p_cell_at_alpha(o.p_value, p_fmt, alpha)))
    };
    TabBuilder::new("normality", "Normality & Lognormality Tests", ResultTabKind::Diagnostics)
        .headers(summaries.iter().map(|s| s.label.as_str()))
        .row("D'Agostino-Pearson K2", stat_cells(|b| b.dagostino_pearson.as_ref()))
        .row("D'Agostino-Pearson P", p_cells(|b| b.dagostino_pearson.as_ref()))
        .row("Shapiro-Wilk W", stat_cells(|b| b.shapiro_wilk.as_ref()))
        .row("Shapiro-Wilk P", p_cells(|b| b.shapiro_wilk.as_ref()))
        .row("Anderson-Darling A2*", stat_cells(|b| b.anderson_darling.as_ref()))
        .row("Anderson-Darling P", p_cells(|b| b.anderson_darling.as_ref()))
        .row("Kolmogorov-Smirnov D", stat_cells(|b| b.kolmogorov_smirnov.as_ref()))
        .row("Kolmogorov-Smirnov P", p_cells(|b| b.kolmogorov_smirnov.as_ref()))
        .row("Normal vs. Lognormal log-LR", batteries.iter().map(|b| opt_num_cell(b.normal_vs_lognormal_log_lr)))
        .build()
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
    if !mu0.is_finite() {
        return Err(StatsError::InvalidParameter { name: "hypothetical_value", value: mu0 });
    }
    let t_results: Vec<_> = raw_series.iter().map(|s| one_sample_t_test(&s.values, mu0, true, conf).ok()).collect();
    let w_results: Vec<_> = raw_series.iter().map(|s| one_sample_wilcoxon(&s.values, mu0, true, conf).ok()).collect();
    let alpha = conf.alpha();
    res.tabs.push(
        TabBuilder::new("one-sample", "One-Sample Tests", ResultTabKind::MainSummary)
            .headers(raw_series.iter().map(|s| s.label.as_str()))
            .row("Hypothetical value", raw_series.iter().map(|_| num_cell(mu0)))
            .row("t statistic", t_results.iter().map(|r| r.as_ref().map_or(ResultCell::Empty, |t| num_cell(t.t_statistic))))
            .row("df", t_results.iter().map(|r| r.as_ref().map_or(ResultCell::Empty, |t| num_cell(t.df))))
            .row("One-sample t P", t_results.iter().map(|r| r.as_ref().map_or(ResultCell::Empty, |t| p_cell_at_alpha(t.p_value, p_fmt, alpha))))
            .row("Wilcoxon signed-rank W", w_results.iter().map(|r| r.as_ref().map_or(ResultCell::Empty, |w| num_cell(w.w_stat))))
            .row("Wilcoxon P", w_results.iter().map(|r| r.as_ref().map_or(ResultCell::Empty, |w| p_cell_at_alpha(w.p_value, p_fmt, alpha))))
            .build(),
    );
    Ok(())
}

/// Execute [`OutlierSpec`] on a [`DataSheet`] and emit a [`ResultSheet`].
pub fn run_outlier_analysis(sheet: &DataSheet, spec: &AnalysisSpec, outlier_spec: OutlierSpec) -> Result<ResultSheet, StatsError> {
    let mut res = init_result_sheet(spec)?;
    let extracted = extract_indexed_series_for_descriptive(sheet, &spec.selected_data_sets, SubcolumnAggregation::PooledGrandSample)?;
    let raw_series = extracted.raw;
    if raw_series.is_empty() {
        if !extracted.pooled.is_empty() || !extracted.no_n_summary.is_empty() || !extracted.interval_summary.is_empty() {
            enforce_summary_mode_matrix(sheet, &AnalysisKind::OutlierDetection(outlier_spec))?;
            return Err(StatsError::RequiresRawReplicates);
        }
        return Err(StatsError::EmptyInput);
    }
    enforce_summary_mode_matrix(sheet, &AnalysisKind::OutlierDetection(outlier_spec))?;
    let mut tab = TabBuilder::new("outliers", "Identified Outliers", ResultTabKind::MainSummary).headers([
        "Column",
        "Row Index (1-based)",
        "Value",
        "Statistic",
        "Threshold / P",
    ]);
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
        for f in flags {
            total_flagged += 1;
            let table_row_1based = series.row_indices.get(f.index).copied().unwrap_or(f.index) + 1;
            let stat_cell = if f.score.is_infinite() { text_cell("∞") } else { num_cell(f.score) };
            tab.push_row(
                format!("Outlier {total_flagged}"),
                [text_cell(series.label.clone()), num_cell(table_row_1based as f64), num_cell(f.value), stat_cell, num_cell(f.threshold_or_p)],
            );
        }
    }

    res.tabs.push(tab.build());
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
