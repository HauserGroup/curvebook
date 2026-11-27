//! Multiple comparisons (FWER) and False Discovery Rate (FDR) adjustment engines.
//!
//! Supports all 14+ procedures: Unadjusted, Tukey–Kramer, Šídák, Bonferroni, Holm–Šídák,
//! Holm–Bonferroni, Dunnett, Dunnett's T3, Games–Howell, Tamhane's T2, Dunn's rank-sum,
//! Fisher's LSD, Student–Newman–Keuls, Benjamini–Hochberg, Benjamini–Yekutieli, and
//! two-stage Benjamini–Krieger–Yekutieli, plus post-hoc linear trend testing.
//!
//! Each routine below cites its source in short `Author (Year)` form; full details are listed
//! here once. Hsu (1996) is the general reference for the familywise procedures.
//!
//! References:
//! - Benjamini, Y., & Hochberg, Y. (1995). Controlling the false discovery rate: a practical and
//!   powerful approach to multiple testing. *Journal of the Royal Statistical Society, Series B*,
//!   57(1), 289-300.
//! - Benjamini, Y., Krieger, A. M., & Yekutieli, D. (2006). Adaptive linear step-up procedures that
//!   control the false discovery rate. *Biometrika*, 93(3), 491-507.
//! - Benjamini, Y., & Yekutieli, D. (2001). The control of the false discovery rate in multiple
//!   testing under dependency. *Annals of Statistics*, 29(4), 1165-1188.
//! - Dunn, O. J. (1961). Multiple comparisons among means. *Journal of the American Statistical
//!   Association*, 56(293), 52-64.
//! - Dunn, O. J. (1964). Multiple comparisons using rank sums. *Technometrics*, 6(3), 241-252.
//! - Dunnett, C. W. (1955). A multiple comparison procedure for comparing several treatments with a
//!   control. *Journal of the American Statistical Association*, 50(272), 1096-1121.
//! - Dunnett, C. W. (1980). Pairwise multiple comparisons in the unequal variance case. *Journal of
//!   the American Statistical Association*, 75(372), 796-800. doi:10.1080/01621459.1980.10477552.
//! - Fisher, R. A. (1935). *The Design of Experiments*. Oliver & Boyd.
//! - Games, P. A., & Howell, J. F. (1976). Pairwise multiple comparison procedures with unequal n's
//!   and/or variances: a Monte Carlo study. *Journal of Educational Statistics*, 1(2), 113-125.
//! - Holland, B. S., & Copenhaver, M. D. (1987). An improved sequentially rejective Bonferroni test
//!   procedure. *Biometrics*, 43(2), 417-423. doi:10.2307/2531823.
//! - Holm, S. (1979). A simple sequentially rejective multiple test procedure. *Scandinavian Journal
//!   of Statistics*, 6(2), 65-70.
//! - Hsu, J. C. (1996). *Multiple Comparisons: Theory and Methods*. Chapman & Hall.
//! - Keuls, M. (1952). The use of the "studentized range" in connection with an analysis of
//!   variance. *Euphytica*, 1(2), 112-122. doi:10.1007/BF01908269.
//! - Kramer, C. Y. (1956). Extension of multiple range tests to group means with unequal numbers of
//!   replications. *Biometrics*, 12(3), 307-310. doi:10.2307/3001469.
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum Associates.
//! - Newman, D. (1939). The distribution of range in samples from a normal population, expressed in
//!   terms of an independent estimate of standard deviation. *Biometrika*, 31(1/2), 20-30.
//! - Šidák, Z. (1967). Rectangular confidence regions for the means of multivariate normal
//!   distributions. *Journal of the American Statistical Association*, 62(318), 626-633.
//! - Tamhane, A. C. (1979). A comparison of procedures for multiple comparisons of means with
//!   unequal variances. *Journal of the American Statistical Association*, 74(366), 471-480.
//!   doi:10.1080/01621459.1979.10482541.
//! - Tukey, J. W. (1953). *The Problem of Multiple Comparisons*. Unpublished manuscript, Princeton
//!   University (reprinted in *The Collected Works of John W. Tukey*, Vol. VIII, 1994).
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.

use std::collections::BTreeSet;

use curvebook_doc::{MultiplicityMethod, MultiplicitySpec, PairwiseComparisonRecord, ResultCell, ResultTab};
use curvebook_math::dist::{
    dunnett_loadings_from_sizes, dunnett_two_sided_crit, dunnett_two_sided_p, f_sf, max_studentized_range_isf, max_studentized_range_sf,
    norm_two_sided_p, student_t_two_sided_isf, student_t_two_sided_p, studentized_range_isf, studentized_range_sf,
};
use curvebook_math::sum::NeumaierSum;
use serde::{Deserialize, Serialize};

use crate::error::StatsError;

/// Unified familywise error-rate and FDR adjustment selector (extends [`MultiplicityMethod`] with
/// standalone Holm–Bonferroni, Fisher LSD, and Newman–Keuls options).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MultiplicityProcedure {
    /// Unadjusted pairwise comparisons without multiplicity control (`adjusted == raw_p`).
    None,
    /// Tukey–Kramer studentized range procedure for unequal or equal sample sizes (FWER; Tukey 1953 / Kramer 1956).
    TukeyKramer,
    /// Dunn–Šidák multiplicative correction `1 - (1 - p)^m` (FWER; Šidák 1967).
    Sidak,
    /// Bonferroni multiplicative correction `min(1, p * m)` (FWER; Dunn 1961).
    Bonferroni,
    /// Holm–Šidák step-down sequential FWER procedure (Holm 1979 / Šidák 1967).
    HolmSidak,
    /// Holm–Bonferroni step-down sequential FWER procedure (Holm 1979).
    HolmBonferroni,
    /// Dunnett many-to-one multivariate *t* comparison against `control_column` (FWER; Dunnett 1955).
    Dunnett {
        /// Zero-based group/dataset index designated as the shared control arm.
        control_column: usize,
    },
    /// Dunnett T3 unequal-variance pairwise comparison using Welch *df* and Šidák-adjusted *t* (FWER; Dunnett 1980).
    DunnettT3,
    /// Games–Howell unequal-variance studentized range comparison with Welch–Satterthwaite *df* (FWER; Games & Howell 1976).
    GamesHowell,
    /// Tamhane T2 unequal-variance conservative Šidák-adjusted pairwise *t* test (FWER; Tamhane 1979).
    TamhaneT2,
    /// Dunn nonparametric rank-sum pairwise multiple comparisons following Kruskal–Wallis (Bonferroni-adjusted *z*; Dunn 1964).
    DunnRankSum,
    /// Single-degree-of-freedom polynomial linear-trend contrast across ordered group means (Maxwell & Delaney 2004).
    LinearTrend,
    /// Unprotected Fisher's Least Significant Difference pairwise *t* tests (`raw_p`; Fisher 1935).
    FisherLsd,
    /// Student–Newman–Keuls step-down studentized range comparison (FWER stepwise; Newman 1939 / Keuls 1952).
    NewmanKeuls,
    /// Benjamini–Hochberg linear step-up FDR procedure producing monotonic *q* values (Benjamini & Hochberg 1995).
    BenjaminiHochbergFdr,
    /// Benjamini–Yekutieli FDR procedure valid under arbitrary dependence via $c(m) = \sum_{i=1}^m 1/i$ (Benjamini & Yekutieli 2001).
    BenjaminiYekutieliFdr,
    /// Two-stage adaptive Benjamini–Krieger–Yekutieli step-up FDR procedure (Benjamini, Krieger & Yekutieli 2006).
    TwoStageBkyFdr,
}

impl MultiplicityProcedure {
    /// Convert a document-layer [`MultiplicityMethod`] specification into a [`MultiplicityProcedure`].
    #[must_use]
    pub fn from_spec(method: MultiplicityMethod) -> Self {
        Self::from(method)
    }
}

impl From<MultiplicityMethod> for MultiplicityProcedure {
    fn from(value: MultiplicityMethod) -> Self {
        match value {
            MultiplicityMethod::None => Self::None,
            MultiplicityMethod::TukeyHsd => Self::TukeyKramer,
            MultiplicityMethod::NewmanKeuls => Self::NewmanKeuls,
            MultiplicityMethod::Dunnett { control_column } => Self::Dunnett { control_column },
            MultiplicityMethod::Sidak => Self::Sidak,
            MultiplicityMethod::Bonferroni => Self::Bonferroni,
            MultiplicityMethod::HolmSidak => Self::HolmSidak,
            MultiplicityMethod::FisherLsd => Self::FisherLsd,
            MultiplicityMethod::DunnettT3 => Self::DunnettT3,
            MultiplicityMethod::GamesHowell => Self::GamesHowell,
            MultiplicityMethod::TamhaneT2 => Self::TamhaneT2,
            MultiplicityMethod::DunnRankSum => Self::DunnRankSum,
            MultiplicityMethod::LinearTrend => Self::LinearTrend,
            MultiplicityMethod::BenjaminiHochbergFdr => Self::BenjaminiHochbergFdr,
            MultiplicityMethod::BenjaminiYekutieliFdr => Self::BenjaminiYekutieliFdr,
            MultiplicityMethod::TwoStageBkyFdr => Self::TwoStageBkyFdr,
        }
    }
}

/// Group summary statistics consumed by pairwise comparison engines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupComparisonInput {
    /// Zero-based group or dataset column index.
    pub index: usize,
    /// Human-readable group header label.
    pub label: String,
    /// Group sample size `n_i >= 1` (`f64` to support summarized or fractional effective weight designs).
    pub n: f64,
    /// Sample mean of observations in this group.
    pub mean: f64,
    /// Sample standard deviation (`N - 1` denominator) within this group (`0.0` if `n == 1`).
    pub sd: f64,
}

/// Single detailed pairwise comparison output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairwiseComparisonDetail {
    /// Zero-based index of the first group (`A`).
    pub group_a: usize,
    /// Zero-based index of the second group (`B`).
    pub group_b: usize,
    /// Display label of `group_a`.
    pub label_a: String,
    /// Display label of `group_b`.
    pub label_b: String,
    /// Signed difference `mean_a - mean_b` (or `mean_rank_a - mean_rank_b` for rank procedures).
    pub mean_diff: f64,
    /// Standard error of `mean_diff` (pooled MSE or Welch unpooled depending on `MultiplicityProcedure`).
    pub se_diff: f64,
    /// Test statistic (`t`, `q / sqrt(2)`, or Dunn `|z|` depending on procedure).
    pub statistic: f64,
    /// Denominator degrees of freedom (`f64::INFINITY` for asymptotic normal rank tests like Dunn).
    pub df: f64,
    /// Unadjusted two-sided single-contrast *P* value (`NaN` if contrast variance is zero).
    pub raw_p: f64,
    /// Multiplicity-adjusted two-sided *P* value (or FDR-adjusted *q* value when `is_fdr_q == true`) in `[0, 1]`.
    pub adjusted_p_or_q: f64,
    /// True when `adjusted_p_or_q` represents a false discovery rate *q* value; false for FWER-adjusted or raw *P*.
    pub is_fdr_q: bool,
    /// Simultaneous or individual confidence interval `(lower, upper)` for `mean_diff` at configured `ConfidenceLevel`; `None` for rank tests or step-down/FDR methods without simultaneous CIs.
    pub ci: Option<(f64, f64)>,
    /// True when this contrast passes the family significance rule under the chosen procedure and threshold.
    pub significant: bool,
}

impl PairwiseComparisonDetail {
    /// Convert to [`PairwiseComparisonRecord`] for [`curvebook_doc::ResultSheet`].
    #[must_use]
    pub fn to_record(&self) -> PairwiseComparisonRecord {
        PairwiseComparisonRecord {
            group_a: self.group_a,
            group_b: self.group_b,
            label_a: self.label_a.clone(),
            label_b: self.label_b.clone(),
            estimate_diff: Some(self.mean_diff),
            raw_p_or_q: self.adjusted_p_or_q,
            is_fdr_q: self.is_fdr_q,
            significant: self.significant,
        }
    }
}

/// Post-hoc linear trend test across ordered groups (Maxwell & Delaney 2004).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinearTrendResult {
    /// Single-degree-of-freedom sum of squares explained by the equally spaced linear contrast across group means.
    pub ss_trend: f64,
    /// Numerator degrees of freedom (`1.0`).
    pub df_trend: f64,
    /// Residual denominator degrees of freedom copied from the omnibus ANOVA error term.
    pub df_error: f64,
    /// Contrast *F*-ratio `ss_trend / mse` on `(1, df_error)` degrees of freedom.
    pub f_statistic: f64,
    /// Upper-tail $F(1, \text{df\_error})$ *P* value testing zero linear slope across ordered group means.
    pub p_value: f64,
    /// Least-squares slope of group means per unit integer group step `L / sum(c_i^2)`.
    pub slope_estimate: f64,
    /// Proportion of total variation explained by the linear trend (`SS_trend / (SS_between + SS_error)`), in `[0, 1]`.
    pub r_squared: f64,
}

/// Compute post-hoc linear trend test across ordered groups using pooled `mse` and `df_error`.
///
/// Single-degree-of-freedom linear contrast with equally spaced, centred coefficients
/// `c_i = i - (k - 1) / 2` (unweighted means approach): `L = sum(c_i * mean_i)`,
/// `SS_trend = L^2 / sum(c_i^2 / n_i)`, `F = SS_trend / MSE` on `(1, df_error)` df, as in the trend
/// analysis of Maxwell & Delaney (2004). `slope_estimate = L / sum(c_i^2)` is the least-squares slope
/// of the group means on group index, and `r_squared = SS_trend / (SS_between + MSE * df_error)`
/// lies in `[0, 1]`. When `mse == 0`, returns `F = +inf, p = 0` for any nonzero linear contrast
/// (`ss_trend > 0`) and `F = 0, p = 1` when `ss_trend == 0`.
pub fn linear_trend_test(groups: &[GroupComparisonInput], mse: f64, df_error: f64) -> Result<LinearTrendResult, StatsError> {
    let k = groups.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "linear trend test", required: 2, actual: k });
    }
    if !mse.is_finite() || mse < 0.0 || !df_error.is_finite() || df_error <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "mse_or_df", value: mse });
    }
    let center = 0.5 * ((k - 1) as f64);
    let mut num = NeumaierSum::new();
    let mut denom_ss = NeumaierSum::new();
    let mut denom_slope = NeumaierSum::new();
    let mut total_n = NeumaierSum::new();
    let mut weighted_mean_sum = NeumaierSum::new();
    for (i, g) in groups.iter().enumerate() {
        if !g.n.is_finite() || g.n <= 0.0 {
            return Err(StatsError::InvalidParameter { name: "group_n", value: g.n });
        }
        if !g.mean.is_finite() {
            return Err(StatsError::InvalidParameter { name: "group_mean", value: g.mean });
        }
        let c = (i as f64) - center;
        num.add(c * g.mean);
        denom_ss.add((c * c) / g.n);
        denom_slope.add(c * c);
        total_n.add(g.n);
        weighted_mean_sum.add(g.n * g.mean);
    }
    let l_val = num.total();
    let d_val = denom_ss.total();
    let ss_trend = if d_val > 0.0 { (l_val * l_val) / d_val } else { 0.0 };
    let grand_mean = if total_n.total() > 0.0 { weighted_mean_sum.total() / total_n.total() } else { 0.0 };
    let mut ss_between_acc = NeumaierSum::new();
    for g in groups {
        let diff = g.mean - grand_mean;
        ss_between_acc.add(g.n * diff * diff);
    }
    let ss_total_implied = ss_between_acc.total() + mse * df_error;
    let r_squared = if ss_total_implied > 0.0 { (ss_trend / ss_total_implied).clamp(0.0, 1.0) } else { 0.0 };
    let (f_stat, p_value) = if mse > 0.0 {
        let f = ss_trend / mse;
        (f, f_sf(f, 1.0, df_error)?)
    } else if ss_trend > 0.0 {
        (f64::INFINITY, 0.0)
    } else {
        (0.0, 1.0)
    };
    let slope_estimate = if denom_slope.total() > 0.0 { l_val / denom_slope.total() } else { 0.0 };
    Ok(LinearTrendResult { ss_trend, df_trend: 1.0, df_error, f_statistic: f_stat, p_value, slope_estimate, r_squared })
}

// ============================================================================
// Standalone P-Value & FDR Adjustment Functions
// ============================================================================

/// Partition `raw_p` so non-finite entries (`NaN`, `±Inf`) remain `NaN` in the output without
/// corrupting the family size `m`, rank order, or step-down/step-up running extrema of valid
/// finite *p*-values.
fn with_finite_pvalues(raw_p: &[f64], f: impl FnOnce(&[f64]) -> Vec<f64>) -> Vec<f64> {
    if raw_p.iter().all(|p| p.is_finite()) {
        return f(raw_p);
    }
    let mut valid_indices = Vec::with_capacity(raw_p.len());
    let mut valid_p = Vec::with_capacity(raw_p.len());
    for (idx, &p) in raw_p.iter().enumerate() {
        if p.is_finite() {
            valid_indices.push(idx);
            valid_p.push(p);
        }
    }
    let mut out = vec![f64::NAN; raw_p.len()];
    if !valid_p.is_empty() {
        let adj = f(&valid_p);
        for (&orig_idx, val) in valid_indices.iter().zip(adj) {
            if let Some(slot) = out.get_mut(orig_idx) {
                *slot = val;
            }
        }
    }
    out
}

/// Adjust a slice of raw *p*-values using Bonferroni (`min(1, m * p)`), the Bonferroni-inequality
/// procedure of Dunn (1961). Non-finite entries (`NaN`, `±Inf`) remain `NaN` and are excluded from `m`.
#[must_use]
pub fn adjust_bonferroni(raw_p: &[f64]) -> Vec<f64> {
    with_finite_pvalues(raw_p, |vals| {
        let m = vals.len() as f64;
        vals.iter().map(|&p| (p.clamp(0.0, 1.0) * m).min(1.0)).collect()
    })
}

/// Adjust a slice of raw *p*-values using Šídák (`1 - (1 - p)^m`, cancellation-free); Šidák (1967).
/// Non-finite entries (`NaN`, `±Inf`) remain `NaN` and are excluded from `m`.
#[must_use]
pub fn adjust_sidak(raw_p: &[f64]) -> Vec<f64> {
    with_finite_pvalues(raw_p, |vals| {
        let m = vals.len() as f64;
        vals.iter().map(|&p| sidak_single(p, m)).collect()
    })
}

#[inline]
fn sidak_single(p: f64, m: f64) -> f64 {
    if !p.is_finite() || !m.is_finite() || m <= 0.0 {
        return f64::NAN;
    }
    let pc = p.clamp(0.0, 1.0);
    if pc >= 1.0 {
        return 1.0;
    }
    if pc <= 0.0 {
        return 0.0;
    }
    (-(m * (-pc).ln_1p()).exp_m1()).clamp(pc, 1.0)
}

/// Step-down Holm–Bonferroni adjusted *p*-values (monotone and `>= raw_p`); Holm (1979).
///
/// Sorted ascending, `p_(i)` is multiplied by `m - i + 1` and a running maximum enforces
/// step-down monotonicity. Non-finite entries (`NaN`, `±Inf`) remain `NaN` and are excluded from `m`.
#[must_use]
pub fn adjust_holm_bonferroni(raw_p: &[f64]) -> Vec<f64> {
    with_finite_pvalues(raw_p, |vals| step_down_adjust_finite(vals, false))
}

/// Step-down Holm–Šídák adjusted *p*-values (monotone and `>= raw_p`).
///
/// Holm's (1979) step-down scheme with the Šidák (1967) multiplier `1 - (1 - p_(i))^(m - i + 1)`
/// in place of the Bonferroni one (Holland & Copenhaver 1987), with a running maximum.
/// Non-finite entries (`NaN`, `±Inf`) remain `NaN` and are excluded from `m`.
#[must_use]
pub fn adjust_holm_sidak(raw_p: &[f64]) -> Vec<f64> {
    with_finite_pvalues(raw_p, |vals| step_down_adjust_finite(vals, true))
}

fn step_down_adjust_finite(raw_p: &[f64], sidak: bool) -> Vec<f64> {
    let m = raw_p.len();
    if m == 0 {
        return Vec::new();
    }
    let mut order: Vec<(usize, f64)> = raw_p.iter().copied().enumerate().map(|(i, p)| (i, p.clamp(0.0, 1.0))).collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut out = vec![0.0_f64; m];
    let mut running_max = 0.0_f64;
    for (rank, &(orig_idx, p)) in order.iter().enumerate() {
        let mult = (m - rank) as f64;
        let cand = if sidak { sidak_single(p, mult) } else { (p * mult).min(1.0) };
        running_max = running_max.max(cand).max(p).min(1.0);
        out[orig_idx] = running_max;
    }
    out
}

/// Benjamini–Hochberg (BH) step-up FDR *q*-values (monotone and `>= raw_p`); Benjamini & Hochberg (1995).
///
/// `q_(i) = min_{j >= i} min(1, m * p_(j) / j)` (running minimum from the largest *p*-value down).
/// Non-finite entries (`NaN`, `±Inf`) remain `NaN` and are excluded from `m`.
#[must_use]
pub fn adjust_benjamini_hochberg(raw_p: &[f64]) -> Vec<f64> {
    with_finite_pvalues(raw_p, |vals| step_up_fdr_finite(vals, 1.0))
}

/// Benjamini–Yekutieli (BY) step-up FDR *q*-values under arbitrary dependency (`c(m) = H_m`);
/// Benjamini & Yekutieli (2001): the BH step-up run with `m` replaced by `m * sum_{i<=m} 1/i`.
/// Non-finite entries (`NaN`, `±Inf`) remain `NaN` and are excluded from `m`.
#[must_use]
pub fn adjust_benjamini_yekutieli(raw_p: &[f64]) -> Vec<f64> {
    with_finite_pvalues(raw_p, |vals| {
        let m = vals.len();
        let harmonic: f64 = (1..=m).map(|i| 1.0 / (i as f64)).sum();
        step_up_fdr_finite(vals, harmonic.max(1.0))
    })
}

fn step_up_fdr_finite(raw_p: &[f64], multiplier: f64) -> Vec<f64> {
    let m = raw_p.len();
    if m == 0 {
        return Vec::new();
    }
    let mf = m as f64;
    let mut order: Vec<(usize, f64)> = raw_p.iter().copied().enumerate().map(|(i, p)| (i, p.clamp(0.0, 1.0))).collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut out = vec![1.0_f64; m];
    let mut running_min = 1.0_f64;
    for (rank, &(orig_idx, p)) in order.iter().enumerate().rev() {
        let cand = (p * mf * multiplier / ((rank + 1) as f64)).min(1.0);
        running_min = running_min.min(cand);
        out[orig_idx] = running_min.max(p).min(1.0);
    }
    out
}

/// Two-stage linear step-up Benjamini–Krieger–Yekutieli (2006) FDR *q*-values evaluated at target `fdr_q`.
/// Discoveries satisfy `q_i <= fdr_q` iff rejected by the two-stage BKY procedure at `fdr_q`.
///
/// Follows the two-stage linear step-up procedure of Benjamini, Krieger & Yekutieli (2006): stage 1
/// runs BH at `q' = q / (1 + q)` giving `r1` rejections; if `r1 = 0` nothing is rejected and if
/// `r1 = m` everything is; otherwise `m0_hat = m - r1` and stage 2 runs the linear step-up with
/// thresholds `i * q' / m0_hat`. The returned values depend on `fdr_q` (via `q'` and `m0_hat`), so
/// they are threshold-specific decision values rather than `q`-independent *q*-values.
/// Any finite `fdr_q` strictly in `(0, 1)` is used directly without floor clamping; non-finite or
/// out-of-range `fdr_q` yields `NaN` for every entry. Non-finite *p*-value entries (`NaN`, `±Inf`)
/// remain `NaN` and are excluded from `m`.
#[must_use]
pub fn adjust_two_stage_bky(raw_p: &[f64], fdr_q: f64) -> Vec<f64> {
    if raw_p.is_empty() {
        return Vec::new();
    }
    if !fdr_q.is_finite() || fdr_q <= 0.0 || fdr_q >= 1.0 {
        return vec![f64::NAN; raw_p.len()];
    }
    let q = fdr_q;
    with_finite_pvalues(raw_p, |vals| {
        let m = vals.len();
        let q_stage1 = q / (1.0 + q);
        let bh1 = step_up_fdr_finite(vals, 1.0);
        let r1 = bh1.iter().filter(|&&qv| qv <= q_stage1).count();
        if r1 == 0 || r1 == m {
            return bh1.into_iter().map(|qv| (qv * (1.0 + q)).clamp(0.0, 1.0)).collect();
        }
        let m0_hat = (m - r1) as f64;
        let mut order: Vec<(usize, f64)> = vals.iter().copied().enumerate().map(|(i, p)| (i, p.clamp(0.0, 1.0))).collect();
        order.sort_by(|a, b| a.1.total_cmp(&b.1));

        let mut out = vec![1.0_f64; m];
        let mut running_min = 1.0_f64;
        // BKY (2006) Definition 6 step-up threshold scaling m0_hat * (1 + q) / i may be < 1,
        // so adjusted Stage-2 decision values can be strictly below raw_p without losing FDR control.
        for (rank, &(orig_idx, p)) in order.iter().enumerate().rev() {
            let cand = (p * m0_hat * (1.0 + q) / ((rank + 1) as f64)).min(1.0);
            running_min = running_min.min(cand);
            out[orig_idx] = running_min.clamp(0.0, 1.0);
        }
        out
    })
}

/// Apply a multiplicity procedure to a family of raw *p*-values (`None` / `FisherLsd` leave values
/// clamped unadjusted; `Bonferroni`, `Sidak`, `HolmBonferroni`, `HolmSidak`, `BenjaminiHochbergFdr`,
/// `BenjaminiYekutieliFdr`, and `TwoStageBkyFdr` apply their respective adjustments; ANOVA-only
/// variants map conservatively to standalone FWER adjustments — `TamhaneT2` to Šídák, and
/// `TukeyKramer`, `NewmanKeuls`, `Dunnett`, `DunnettT3`, `GamesHowell`, `DunnRankSum`, and
/// `LinearTrend` to Bonferroni). Prefer [`try_adjust_p_values`] when ANOVA-only variants should be
/// rejected with [`StatsError::IncompatibleAnalysisSpec`].
#[must_use]
pub fn adjust_p_values(raw_p: &[f64], procedure: MultiplicityProcedure, threshold: f64) -> (Vec<f64>, bool) {
    match procedure {
        MultiplicityProcedure::None | MultiplicityProcedure::FisherLsd => {
            (with_finite_pvalues(raw_p, |vals| vals.iter().map(|p| p.clamp(0.0, 1.0)).collect()), false)
        }
        MultiplicityProcedure::Bonferroni
        | MultiplicityProcedure::TukeyKramer
        | MultiplicityProcedure::NewmanKeuls
        | MultiplicityProcedure::Dunnett { .. }
        | MultiplicityProcedure::DunnettT3
        | MultiplicityProcedure::GamesHowell
        | MultiplicityProcedure::DunnRankSum
        | MultiplicityProcedure::LinearTrend => (adjust_bonferroni(raw_p), false),
        MultiplicityProcedure::Sidak | MultiplicityProcedure::TamhaneT2 => (adjust_sidak(raw_p), false),
        MultiplicityProcedure::HolmBonferroni => (adjust_holm_bonferroni(raw_p), false),
        MultiplicityProcedure::HolmSidak => (adjust_holm_sidak(raw_p), false),
        MultiplicityProcedure::BenjaminiHochbergFdr => (adjust_benjamini_hochberg(raw_p), true),
        MultiplicityProcedure::BenjaminiYekutieliFdr => (adjust_benjamini_yekutieli(raw_p), true),
        MultiplicityProcedure::TwoStageBkyFdr => (adjust_two_stage_bky(raw_p, threshold), true),
    }
}

/// Fallibly adjust a family of raw *p*-values, rejecting ANOVA-only comparison procedures.
///
/// Rejects `TukeyKramer`, `NewmanKeuls`, `Dunnett`, `DunnettT3`, `GamesHowell`, `DunnRankSum`,
/// `LinearTrend`, and `TamhaneT2` with [`StatsError::IncompatibleAnalysisSpec`] because those
/// procedures depend on group means, sample sizes, variance estimates, or rank sums rather than a
/// standalone family of *p*-values (Hsu 1996; Benjamini, Krieger & Yekutieli 2006).
pub fn try_adjust_p_values(raw_p: &[f64], procedure: MultiplicityProcedure, threshold: f64) -> Result<(Vec<f64>, bool), StatsError> {
    match procedure {
        MultiplicityProcedure::TukeyKramer
        | MultiplicityProcedure::NewmanKeuls
        | MultiplicityProcedure::Dunnett { .. }
        | MultiplicityProcedure::DunnettT3
        | MultiplicityProcedure::GamesHowell
        | MultiplicityProcedure::DunnRankSum
        | MultiplicityProcedure::LinearTrend
        | MultiplicityProcedure::TamhaneT2 => Err(StatsError::IncompatibleAnalysisSpec {
            expected: "raw p-value multiplicity procedure (None, Bonferroni, Sidak, HolmBonferroni, HolmSidak, BenjaminiHochbergFdr, BenjaminiYekutieliFdr, or TwoStageBkyFdr)",
        }),
        _ => Ok(adjust_p_values(raw_p, procedure, threshold)),
    }
}

// ============================================================================
// Pairwise Group Comparison Engines (Pooled-Variance, Unequal-Variance, Rank)
// ============================================================================

/// Validated pairwise comparison family selection extracted from [`MultiplicitySpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MultiplicityFamilySelection<'a> {
    /// All `k * (k - 1) / 2` distinct pairs.
    AllPairs,
    /// Many-to-one comparisons of every non-control group against the zero-based control index.
    ControlVsAll(usize),
    /// Explicit planned subset of `m = pairs.len()` distinct `(group_a, group_b)` pairs (Hsu 1996).
    SelectedPairs(&'a [(usize, usize)]),
}

/// Validate that `spec.control_column` and `spec.selected_pairs` are mutually exclusive,
/// compatible with `spec.method`, non-empty, free of self-pairs, and free of duplicate unordered
/// pairs, returning the active [`MultiplicityFamilySelection`].
pub(crate) fn validate_multiplicity_family_spec(spec: &MultiplicitySpec) -> Result<MultiplicityFamilySelection<'_>, StatsError> {
    if let Some(pairs) = spec.selected_pairs.as_deref() {
        if spec.effective_control_column().is_some() {
            return Err(StatsError::IncompatibleAnalysisSpec { expected: "either control_column (or Dunnett) or selected_pairs, not both" });
        }
        if !MultiplicitySpec::supports_selected_pairs(spec.method) {
            return Err(StatsError::IncompatibleAnalysisSpec {
                expected: "selected-pairs compatible multiplicity method (Bonferroni, Sidak, HolmSidak, DunnRankSum, FisherLsd, DunnettT3, TamhaneT2, or FDR)",
            });
        }
        if pairs.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        let mut seen = BTreeSet::new();
        for &(a, b) in pairs {
            if a == b {
                return Err(StatsError::InvalidParameter { name: "selected_pairs", value: a as f64 });
            }
            let key = (a.min(b), a.max(b));
            if !seen.insert(key) {
                return Err(StatsError::InvalidParameter { name: "selected_pairs", value: a as f64 });
            }
        }
        return Ok(MultiplicityFamilySelection::SelectedPairs(pairs));
    }
    match validate_multiplicity_control_spec(spec)? {
        Some(ctrl) => Ok(MultiplicityFamilySelection::ControlVsAll(ctrl)),
        None => Ok(MultiplicityFamilySelection::AllPairs),
    }
}

/// Map a validated slice of external group-index pairs onto slice positions using `find_pos`,
/// rejecting any member absent from the analysed groups or collapsing onto the same position.
pub(crate) fn resolve_selected_pair_positions<F>(pairs: &[(usize, usize)], mut find_pos: F) -> Result<Vec<(usize, usize)>, StatsError>
where
    F: FnMut(usize) -> Option<usize>,
{
    let mut out = Vec::with_capacity(pairs.len());
    for &(a, b) in pairs {
        let pa = find_pos(a).ok_or(StatsError::InvalidParameter { name: "selected_pairs", value: a as f64 })?;
        let pb = find_pos(b).ok_or(StatsError::InvalidParameter { name: "selected_pairs", value: b as f64 })?;
        if pa == pb {
            return Err(StatsError::InvalidParameter { name: "selected_pairs", value: a as f64 });
        }
        out.push((pa, pb));
    }
    Ok(out)
}

/// Validate that `spec.control_column` is compatible with `spec.method` and return the effective
/// zero-based control index when a many-to-one control-versus-all family is requested.
pub(crate) fn validate_multiplicity_control_spec(spec: &MultiplicitySpec) -> Result<Option<usize>, StatsError> {
    match spec.method {
        MultiplicityMethod::Dunnett { control_column } => {
            if let Some(extra) = spec.control_column
                && extra != control_column
            {
                return Err(StatsError::InvalidParameter { name: "control_column", value: extra as f64 });
            }
            Ok(Some(control_column))
        }
        other => match spec.control_column {
            Some(ctrl) if MultiplicitySpec::supports_control_column(other) => Ok(Some(ctrl)),
            Some(_) => Err(StatsError::IncompatibleAnalysisSpec {
                expected: "control-vs-all compatible multiplicity method (Dunnett, Bonferroni, Sidak, HolmSidak, DunnRankSum, FisherLsd, DunnettT3, TamhaneT2, or FDR)",
            }),
            None => Ok(None),
        },
    }
}

fn resolve_group_pair_indices(groups: &[GroupComparisonInput], family: MultiplicityFamilySelection<'_>) -> Result<Vec<(usize, usize)>, StatsError> {
    let k = groups.len();
    match family {
        MultiplicityFamilySelection::ControlVsAll(ctrl) => {
            let ctrl_pos =
                groups.iter().position(|g| g.index == ctrl).ok_or(StatsError::InvalidParameter { name: "control_column", value: ctrl as f64 })?;
            Ok((0..k).filter(|&i| i != ctrl_pos).map(|i| (i, ctrl_pos)).collect())
        }
        MultiplicityFamilySelection::SelectedPairs(pairs) => resolve_selected_pair_positions(pairs, |idx| groups.iter().position(|g| g.index == idx)),
        MultiplicityFamilySelection::AllPairs => {
            let mut pairs = Vec::with_capacity(k * k.saturating_sub(1) / 2);
            for i in 0..k {
                for j in (i + 1)..k {
                    pairs.push((i, j));
                }
            }
            Ok(pairs)
        }
    }
}

/// Compute pairwise comparisons across `groups` using pooled `mse` and `df_error` (for parametric
/// equal-variance tests) or group SDs (for `GamesHowell`, `DunnettT3`, `TamhaneT2`).
pub fn compare_groups_pairwise(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    spec: &MultiplicitySpec,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let family = validate_multiplicity_family_spec(spec)?;
    compare_groups_impl(groups, mse, df_error, spec.method.into(), family, spec.threshold, spec.threshold, spec.report_confidence_intervals)
}

/// Compute pairwise comparisons across `groups` using `confidence` (`ci_alpha = 1 - confidence`)
/// to construct confidence intervals while `spec.threshold` governs discovery/significance flags.
pub fn compare_groups_pairwise_with_confidence(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    spec: &MultiplicitySpec,
    confidence: curvebook_math::ConfidenceLevel,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let family = validate_multiplicity_family_spec(spec)?;
    compare_groups_impl(groups, mse, df_error, spec.method.into(), family, spec.threshold, confidence.alpha(), spec.report_confidence_intervals)
}

/// Compute pairwise comparisons with explicit [`MultiplicityProcedure`].
///
/// Pooled-variance procedures (`None`, `FisherLsd`, `Bonferroni`, `Sidak`, Holm variants, and the
/// FDR procedures) start from Student *t* statistics `(mean_a - mean_b) / sqrt(MSE (1/n_a + 1/n_b))`
/// on `df_error` and then adjust the *p*-values. `FisherLsd` is the unadjusted least significant
/// difference of Fisher (1935); this routine does not itself gate it on a significant omnibus *F*.
/// Procedure-specific sources are cited on each comparison engine. Non-pairwise procedures
/// (`DunnRankSum`, `LinearTrend`) return `StatsError::InvalidParameter`; use [`compare_dunn_ranks`]
/// or [`linear_trend_test`] instead.
pub fn compare_groups_with_procedure(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    procedure: MultiplicityProcedure,
    threshold: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let family = match procedure {
        MultiplicityProcedure::Dunnett { control_column } => MultiplicityFamilySelection::ControlVsAll(control_column),
        _ => MultiplicityFamilySelection::AllPairs,
    };
    compare_groups_impl(groups, mse, df_error, procedure, family, threshold, threshold, report_ci)
}

/// Compute pairwise comparisons with explicit [`MultiplicityProcedure`], constructing confidence
/// intervals at `confidence` (`ci_alpha = confidence.alpha()`) while testing significance against
/// `threshold`.
pub fn compare_groups_with_confidence(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    procedure: MultiplicityProcedure,
    threshold: f64,
    report_ci: bool,
    confidence: curvebook_math::ConfidenceLevel,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let family = match procedure {
        MultiplicityProcedure::Dunnett { control_column } => MultiplicityFamilySelection::ControlVsAll(control_column),
        _ => MultiplicityFamilySelection::AllPairs,
    };
    compare_groups_impl(groups, mse, df_error, procedure, family, threshold, confidence.alpha(), report_ci)
}

#[allow(clippy::too_many_arguments)]
fn compare_groups_impl(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    procedure: MultiplicityProcedure,
    family: MultiplicityFamilySelection<'_>,
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    if k < 2 {
        return Ok(Vec::new());
    }
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: threshold });
    }
    if report_ci && (!ci_alpha.is_finite() || ci_alpha <= 0.0 || ci_alpha >= 1.0) {
        return Err(StatsError::InvalidParameter { name: "ci_alpha", value: ci_alpha });
    }
    let requires_group_variance =
        matches!(procedure, MultiplicityProcedure::GamesHowell | MultiplicityProcedure::TamhaneT2 | MultiplicityProcedure::DunnettT3);
    for g in groups {
        if !g.n.is_finite() || g.n <= 0.0 {
            return Err(StatsError::InvalidParameter { name: "group_n", value: g.n });
        }
        if requires_group_variance && g.n < 2.0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let actual = g.n.floor().max(0.0) as usize;
            return Err(StatsError::InsufficientSampleSize { context: "unequal-variance pairwise comparison group", required: 2, actual });
        }
        if !g.mean.is_finite() {
            return Err(StatsError::InvalidParameter { name: "group_mean", value: g.mean });
        }
        if !g.sd.is_finite() || g.sd < 0.0 {
            return Err(StatsError::InvalidParameter { name: "group_sd", value: g.sd });
        }
    }
    if !matches!(
        procedure,
        MultiplicityProcedure::GamesHowell
            | MultiplicityProcedure::TamhaneT2
            | MultiplicityProcedure::DunnettT3
            | MultiplicityProcedure::DunnRankSum
            | MultiplicityProcedure::LinearTrend
    ) {
        if !mse.is_finite() || mse < 0.0 {
            return Err(StatsError::InvalidParameter { name: "mse", value: mse });
        }
        if !df_error.is_finite() || df_error <= 0.0 {
            return Err(StatsError::InvalidParameter { name: "df_error", value: df_error });
        }
    }
    match procedure {
        MultiplicityProcedure::Dunnett { control_column } => {
            if matches!(family, MultiplicityFamilySelection::SelectedPairs(_)) {
                return Err(StatsError::IncompatibleAnalysisSpec { expected: "control-vs-all Dunnett procedure without selected_pairs" });
            }
            compare_dunnett(groups, mse, df_error, control_column, threshold, ci_alpha, report_ci)
        }
        MultiplicityProcedure::TukeyKramer => {
            if !matches!(family, MultiplicityFamilySelection::AllPairs) {
                return Err(StatsError::IncompatibleAnalysisSpec {
                    expected: "all-pairs Tukey-Kramer procedure without control_column or selected_pairs",
                });
            }
            compare_tukey_kramer(groups, mse, df_error, threshold, ci_alpha, report_ci)
        }
        MultiplicityProcedure::NewmanKeuls => {
            if !matches!(family, MultiplicityFamilySelection::AllPairs) {
                return Err(StatsError::IncompatibleAnalysisSpec {
                    expected: "all-pairs Newman-Keuls procedure without control_column or selected_pairs",
                });
            }
            compare_newman_keuls(groups, mse, df_error, threshold)
        }
        MultiplicityProcedure::GamesHowell => {
            if !matches!(family, MultiplicityFamilySelection::AllPairs) {
                return Err(StatsError::IncompatibleAnalysisSpec {
                    expected: "all-pairs Games-Howell procedure without control_column or selected_pairs",
                });
            }
            compare_games_howell(groups, threshold, ci_alpha, report_ci)
        }
        MultiplicityProcedure::TamhaneT2 => compare_tamhane_t2(groups, family, threshold, ci_alpha, report_ci),
        MultiplicityProcedure::DunnettT3 => compare_dunnett_t3(groups, family, threshold, ci_alpha, report_ci),
        MultiplicityProcedure::DunnRankSum => {
            Err(StatsError::InvalidParameter { name: "multiplicity_procedure_dunn_requires_kruskal_wallis", value: 0.0 })
        }
        MultiplicityProcedure::LinearTrend => {
            Err(StatsError::InvalidParameter { name: "multiplicity_procedure_linear_trend_requires_trend_test", value: 0.0 })
        }
        MultiplicityProcedure::None
        | MultiplicityProcedure::Sidak
        | MultiplicityProcedure::Bonferroni
        | MultiplicityProcedure::HolmSidak
        | MultiplicityProcedure::HolmBonferroni
        | MultiplicityProcedure::FisherLsd
        | MultiplicityProcedure::BenjaminiHochbergFdr
        | MultiplicityProcedure::BenjaminiYekutieliFdr
        | MultiplicityProcedure::TwoStageBkyFdr => compare_pooled_or_fdr(groups, mse, df_error, procedure, family, threshold, ci_alpha, report_ci),
    }
}

fn zero_se_stat_p(diff: f64, signed: bool) -> (f64, f64) {
    if diff != 0.0 {
        let stat = if signed { diff.signum() * f64::INFINITY } else { f64::INFINITY };
        (stat, 0.0)
    } else {
        (0.0, 1.0)
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_pooled_or_fdr(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    procedure: MultiplicityProcedure,
    family: MultiplicityFamilySelection<'_>,
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let pair_indices = resolve_group_pair_indices(groups, family)?;
    let m = pair_indices.len();
    if m == 0 {
        return Ok(Vec::new());
    }
    let mut raw_p = Vec::with_capacity(m);
    let mut entries = Vec::with_capacity(m);

    let per_comp_alpha = match procedure {
        MultiplicityProcedure::Bonferroni => Some(ci_alpha / (m as f64)),
        MultiplicityProcedure::Sidak => Some(1.0 - (1.0 - ci_alpha).powf(1.0 / (m as f64))),
        MultiplicityProcedure::None | MultiplicityProcedure::FisherLsd => Some(ci_alpha),
        _ => None,
    };
    let t_crit_opt = match (report_ci, per_comp_alpha) {
        (true, Some(a)) => Some(student_t_two_sided_isf(a.clamp(1e-12, 0.9999), df_error)?),
        _ => None,
    };

    for (i, j) in pair_indices {
        let ga = &groups[i];
        let gb = &groups[j];
        let diff = ga.mean - gb.mean;
        let se = (mse * (1.0 / ga.n + 1.0 / gb.n)).max(0.0).sqrt();
        let (t_stat, p_val) = if se > 0.0 {
            let t = diff / se;
            (t, student_t_two_sided_p(t, df_error)?)
        } else {
            zero_se_stat_p(diff, true)
        };
        raw_p.push(p_val);
        let ci = t_crit_opt.map(|tc| (diff - tc * se, diff + tc * se));
        entries.push((ga, gb, diff, se, t_stat, p_val, ci));
    }

    let (adj, is_fdr) = adjust_p_values(&raw_p, procedure, threshold);
    let out = entries
        .into_iter()
        .zip(adj)
        .map(|((ga, gb, diff, se, t_stat, p_val, ci), adj_p)| PairwiseComparisonDetail {
            group_a: ga.index,
            group_b: gb.index,
            label_a: ga.label.clone(),
            label_b: gb.label.clone(),
            mean_diff: diff,
            se_diff: se,
            statistic: t_stat,
            df: df_error,
            raw_p: p_val,
            adjusted_p_or_q: adj_p,
            is_fdr_q: is_fdr,
            ci,
            significant: adj_p.is_finite() && adj_p <= threshold,
        })
        .collect();
    Ok(out)
}

/// Tukey (1953) honestly-significant-difference comparisons with the Kramer (1956) unequal-`n`
/// standard error: `q = |mean_a - mean_b| / sqrt((MSE / 2) (1/n_a + 1/n_b))` referred to the
/// studentized range distribution with `k` means and `df_error` df.
fn compare_tukey_kramer(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    let q_crit = if report_ci { Some(studentized_range_isf(ci_alpha, k, df_error)?) } else { None };
    let mut out = Vec::with_capacity(k * (k - 1) / 2);
    for i in 0..k {
        for j in (i + 1)..k {
            let ga = &groups[i];
            let gb = &groups[j];
            let diff = ga.mean - gb.mean;
            let se_tukey = (0.5 * mse * (1.0 / ga.n + 1.0 / gb.n)).max(0.0).sqrt();
            let se_diff = se_tukey * std::f64::consts::SQRT_2;
            let (q_stat, raw_p, adj_p) = if se_tukey > 0.0 {
                let q = diff.abs() / se_tukey;
                let rp = student_t_two_sided_p(diff / se_diff, df_error)?;
                (q, rp, studentized_range_sf(q, k, df_error)?)
            } else {
                let (stat, p) = zero_se_stat_p(diff, false);
                (stat, p, p)
            };
            let ci = q_crit.map(|qc| (diff - qc * se_tukey, diff + qc * se_tukey));
            out.push(PairwiseComparisonDetail {
                group_a: ga.index,
                group_b: gb.index,
                label_a: ga.label.clone(),
                label_b: gb.label.clone(),
                mean_diff: diff,
                se_diff,
                statistic: q_stat,
                df: df_error,
                raw_p,
                adjusted_p_or_q: adj_p,
                is_fdr_q: false,
                ci,
                significant: adj_p.is_finite() && adj_p <= threshold,
            });
        }
    }
    Ok(out)
}

/// Student–Newman–Keuls multiple-range comparisons (Newman 1939; Keuls 1952), with Kramer (1956)
/// unequal-`n` standard errors.
///
/// Each pair of means spanning `p` ordered means is referred to the studentized range with `p`
/// means and `df_error` df. Adjusted *p*-values are made coherent with the step-down rule (a range is
/// declared different only if every range containing it is) by taking the maximum with the
/// enclosing ranges. Newman–Keuls does not control the familywise error rate when `k > 3`
/// (Hsu 1996).
fn compare_newman_keuls(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    threshold: f64,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    let mut sorted_idx: Vec<usize> = (0..k).collect();
    sorted_idx.sort_by(|&a, &b| groups[a].mean.total_cmp(&groups[b].mean));

    // Compute p-values in sorted-mean matrix `[p_lo][p_hi]` for `p_lo < p_hi`, enforcing superset
    // step-down monotonicity: `adj_p[i][j] = max(raw_p[i][j], adj_p[i-1][j], adj_p[i][j+1])`.
    let mut p_mat = vec![vec![1.0_f64; k]; k];
    for span in (2..=k).rev() {
        for lo in 0..=(k - span) {
            let hi = lo + span - 1;
            let ga = &groups[sorted_idx[lo]];
            let gb = &groups[sorted_idx[hi]];
            let diff_span = gb.mean - ga.mean;
            let se_q = (0.5 * mse * (1.0 / ga.n + 1.0 / gb.n)).max(0.0).sqrt();
            let raw_p = if se_q > 0.0 {
                let q = diff_span.abs() / se_q;
                studentized_range_sf(q, span, df_error)?
            } else {
                zero_se_stat_p(diff_span, false).1
            };
            let mut adj = raw_p;
            if lo > 0 {
                adj = adj.max(p_mat[lo - 1][hi]);
            }
            if hi + 1 < k {
                adj = adj.max(p_mat[lo][hi + 1]);
            }
            p_mat[lo][hi] = adj.clamp(0.0, 1.0);
        }
    }

    let mut pos_of = vec![0_usize; k];
    for (pos, &orig) in sorted_idx.iter().enumerate() {
        pos_of[orig] = pos;
    }

    let mut out = Vec::with_capacity(k * (k - 1) / 2);
    for i in 0..k {
        for j in (i + 1)..k {
            let ga = &groups[i];
            let gb = &groups[j];
            let diff = ga.mean - gb.mean;
            let se_q = (0.5 * mse * (1.0 / ga.n + 1.0 / gb.n)).max(0.0).sqrt();
            let (q_stat, raw_p) = if se_q > 0.0 {
                let q = diff.abs() / se_q;
                (q, studentized_range_sf(q, 2, df_error)?)
            } else {
                zero_se_stat_p(diff, false)
            };
            let (p_lo, p_hi) = if pos_of[i] < pos_of[j] { (pos_of[i], pos_of[j]) } else { (pos_of[j], pos_of[i]) };
            let adj_p = p_mat[p_lo][p_hi];
            out.push(PairwiseComparisonDetail {
                group_a: ga.index,
                group_b: gb.index,
                label_a: ga.label.clone(),
                label_b: gb.label.clone(),
                mean_diff: diff,
                se_diff: se_q * std::f64::consts::SQRT_2,
                statistic: q_stat,
                df: df_error,
                raw_p,
                adjusted_p_or_q: adj_p,
                is_fdr_q: false,
                ci: None,
                significant: adj_p.is_finite() && adj_p <= threshold,
            });
        }
    }
    Ok(out)
}

/// Dunnett (1955) many-to-one comparisons of each treatment against a control: pooled-variance
/// *t* statistics referred to the exact multivariate *t* distribution with correlations
/// `rho_ij = lambda_i lambda_j`, `lambda_i = sqrt(n_i / (n_i + n_0))` (unequal `n` supported).
/// Returns `StatsError::InvalidParameter` when `control_column` matches no group.
fn compare_dunnett(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    control_column: usize,
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let ctrl_pos = groups
        .iter()
        .position(|g| g.index == control_column)
        .ok_or(StatsError::InvalidParameter { name: "control_column", value: control_column as f64 })?;
    let ctrl = &groups[ctrl_pos];
    let treatments: Vec<&GroupComparisonInput> = groups.iter().enumerate().filter(|&(i, _)| i != ctrl_pos).map(|(_, g)| g).collect();
    if treatments.is_empty() {
        return Ok(Vec::new());
    }
    let treat_sizes: Vec<f64> = treatments.iter().map(|g| g.n).collect();
    let loadings = dunnett_loadings_from_sizes(ctrl.n, &treat_sizes)?;
    let d_crit = if report_ci { Some(dunnett_two_sided_crit(ci_alpha, df_error, ctrl.n, &treat_sizes)?) } else { None };

    let mut out = Vec::with_capacity(treatments.len());
    for (idx, tr) in treatments.iter().enumerate() {
        let diff = tr.mean - ctrl.mean;
        let se = (mse * (1.0 / tr.n + 1.0 / ctrl.n)).max(0.0).sqrt();
        let (t_stat, raw_p, adj_p) = if se > 0.0 {
            let t = diff / se;
            let rp = student_t_two_sided_p(t, df_error)?;
            // Exact Dunnett multivariate-t p-value for this comparison against all m loadings.
            let ap = if loadings.len() == 1 { rp } else { dunnett_two_sided_p(t.abs(), df_error, ctrl.n, &treat_sizes)? };
            (t, rp, ap)
        } else {
            let (stat, p) = zero_se_stat_p(diff, true);
            (stat, p, p)
        };
        let _ = idx;
        let ci = d_crit.map(|dc| (diff - dc * se, diff + dc * se));
        out.push(PairwiseComparisonDetail {
            group_a: tr.index,
            group_b: ctrl.index,
            label_a: tr.label.clone(),
            label_b: ctrl.label.clone(),
            mean_diff: diff,
            se_diff: se,
            statistic: t_stat,
            df: df_error,
            raw_p,
            adjusted_p_or_q: adj_p,
            is_fdr_q: false,
            ci,
            significant: adj_p.is_finite() && adj_p <= threshold,
        });
    }
    Ok(out)
}

/// Welch standard error `sqrt(s_a^2/n_a + s_b^2/n_b)` and Welch–Satterthwaite df for one pair,
/// as used by the Games & Howell (1976), Tamhane (1979) T2 and Dunnett (1980) T3 procedures.
///
/// Scales group standard deviations by `s_max = max(s_a, s_b)` (and variance terms by their
/// maximum) before squaring so neither `se` nor `df` can overflow or underflow to `NaN` across
/// the full finite `f64` range. Returns `(NaN, NaN)` if either group has `n <= 1.0` or invalid SD.
pub(crate) fn welch_pair_se_df(ga: &GroupComparisonInput, gb: &GroupComparisonInput) -> (f64, f64) {
    if !ga.n.is_finite() || !gb.n.is_finite() || ga.n <= 1.0 || gb.n <= 1.0 || !ga.sd.is_finite() || !gb.sd.is_finite() || ga.sd < 0.0 || gb.sd < 0.0
    {
        return (f64::NAN, f64::NAN);
    }
    let s_max = ga.sd.max(gb.sd);
    if s_max <= 0.0 {
        return (0.0, (ga.n + gb.n - 2.0).max(1.0));
    }
    let sa = ga.sd / s_max;
    let sb = gb.sd / s_max;
    let ra = (sa * sa) / ga.n;
    let rb = (sb * sb) / gb.n;
    let r_sum = ra + rb;
    if r_sum <= 0.0 {
        return (0.0, (ga.n + gb.n - 2.0).max(1.0));
    }
    let se = s_max * r_sum.sqrt();
    let r_max = ra.max(rb);
    let ua = ra / r_max;
    let ub = rb / r_max;
    let u_sum = ua + ub;
    let denom = (ua * ua) / (ga.n - 1.0) + (ub * ub) / (gb.n - 1.0);
    let df = if denom > 0.0 && denom.is_finite() { (u_sum * u_sum) / denom } else { (ga.n + gb.n - 2.0).max(1.0) };
    (se, df.max(1.0))
}

/// Games & Howell (1976) unequal-variance pairwise comparisons: per-pair Welch standard error and
/// df, with `q = |diff| / (SE / sqrt(2))` referred to the studentized range with `k` means.
fn compare_games_howell(
    groups: &[GroupComparisonInput],
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    let mut out = Vec::with_capacity(k * (k - 1) / 2);
    for i in 0..k {
        for j in (i + 1)..k {
            let ga = &groups[i];
            let gb = &groups[j];
            let diff = ga.mean - gb.mean;
            let (se, df) = welch_pair_se_df(ga, gb);
            let se_q = se * std::f64::consts::FRAC_1_SQRT_2;
            let (q_stat, raw_p, adj_p) = if se_q > 0.0 {
                let q = diff.abs() / se_q;
                let rp = student_t_two_sided_p(diff / se, df)?;
                (q, rp, studentized_range_sf(q, k, df)?)
            } else {
                let (stat, p) = zero_se_stat_p(diff, false);
                (stat, p, p)
            };
            let ci = if report_ci {
                let qc = studentized_range_isf(ci_alpha, k, df)?;
                Some((diff - qc * se_q, diff + qc * se_q))
            } else {
                None
            };
            out.push(PairwiseComparisonDetail {
                group_a: ga.index,
                group_b: gb.index,
                label_a: ga.label.clone(),
                label_b: gb.label.clone(),
                mean_diff: diff,
                se_diff: se,
                statistic: q_stat,
                df,
                raw_p,
                adjusted_p_or_q: adj_p,
                is_fdr_q: false,
                ci,
                significant: adj_p.is_finite() && adj_p <= threshold,
            });
        }
    }
    Ok(out)
}

/// Tamhane (1979) T2 unequal-variance pairwise comparisons: per-pair Welch *t* tests with a Šidák
/// adjustment over the `m` comparisons (`m = k(k-1)/2` for all-pairs, `m = k - 1` when
/// `control_column` is specified, or `m = selected_pairs.len()` for planned comparisons;
/// Šidák-adjusted Welch critical values for CIs).
fn compare_tamhane_t2(
    groups: &[GroupComparisonInput],
    family: MultiplicityFamilySelection<'_>,
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let pair_indices = resolve_group_pair_indices(groups, family)?;
    if pair_indices.is_empty() {
        return Ok(Vec::new());
    }
    let m = pair_indices.len() as f64;
    let alpha_sidak = 1.0 - (1.0 - ci_alpha).powf(1.0 / m);
    let mut out = Vec::with_capacity(pair_indices.len());
    for (i, j) in pair_indices {
        let ga = &groups[i];
        let gb = &groups[j];
        let diff = ga.mean - gb.mean;
        let (se, df) = welch_pair_se_df(ga, gb);
        let (t_stat, raw_p) = if se > 0.0 {
            let t = diff / se;
            (t, student_t_two_sided_p(t, df)?)
        } else {
            zero_se_stat_p(diff, true)
        };
        let adj_p = sidak_single(raw_p, m);
        let ci = if report_ci {
            let tc = student_t_two_sided_isf(alpha_sidak.clamp(1e-12, 0.9999), df)?;
            Some((diff - tc * se, diff + tc * se))
        } else {
            None
        };
        out.push(PairwiseComparisonDetail {
            group_a: ga.index,
            group_b: gb.index,
            label_a: ga.label.clone(),
            label_b: gb.label.clone(),
            mean_diff: diff,
            se_diff: se,
            statistic: t_stat,
            df,
            raw_p,
            adjusted_p_or_q: adj_p,
            is_fdr_q: false,
            ci,
            significant: adj_p.is_finite() && adj_p <= threshold,
        });
    }
    Ok(out)
}

/// Studentized Maximum Modulus survival probability `P(max_{1..m} |T_l| > t)` with Welch df `nu`,
/// used for Dunnett's T3 unequal-variance pairwise comparisons.
///
/// Uses the identity between Studentized Maximum Modulus `M(m, nu)` and the maximum of `m`
/// independent 2-group Studentized ranges scaled by `1 / sqrt(2)`.
fn studentized_max_modulus_sf(t_abs: f64, m: usize, df: f64) -> Result<f64, StatsError> {
    if t_abs <= 0.0 {
        return Ok(1.0);
    }
    Ok(max_studentized_range_sf(std::f64::consts::SQRT_2 * t_abs, m.max(1), 2, df)?)
}

fn studentized_max_modulus_crit(alpha: f64, m: usize, df: f64) -> Result<f64, StatsError> {
    Ok(max_studentized_range_isf(alpha, m.max(1), 2, df)? * std::f64::consts::FRAC_1_SQRT_2)
}

/// Dunnett (1980) T3 unequal-variance pairwise comparisons: per-pair Welch *t* statistics referred
/// to the studentized maximum modulus with `m` comparisons (`m = k(k-1)/2` for all-pairs,
/// `m = k - 1` when `control_column` is specified, or `m = selected_pairs.len()` for planned
/// comparisons) and the pair's Welch df. `raw_p` reports the unadjusted two-sided Welch *t*
/// *p*-value on Welch df, and `adjusted_p_or_q` reports the Dunnett T3 adjusted *p*-value.
fn compare_dunnett_t3(
    groups: &[GroupComparisonInput],
    family: MultiplicityFamilySelection<'_>,
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let pair_indices = resolve_group_pair_indices(groups, family)?;
    let m = pair_indices.len();
    if m == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(m);
    for (i, j) in pair_indices {
        let ga = &groups[i];
        let gb = &groups[j];
        let diff = ga.mean - gb.mean;
        let (se, df) = welch_pair_se_df(ga, gb);
        let (t_stat, raw_p, adj_p) = if se > 0.0 {
            let t = diff / se;
            let rp = student_t_two_sided_p(t, df)?;
            let ap = if m == 1 { rp } else { studentized_max_modulus_sf(t.abs(), m, df)? };
            (t, rp, ap)
        } else {
            let (stat, p) = zero_se_stat_p(diff, true);
            (stat, p, p)
        };
        let ci = if report_ci {
            let crit =
                if m == 1 { student_t_two_sided_isf(ci_alpha.clamp(1e-12, 0.9999), df)? } else { studentized_max_modulus_crit(ci_alpha, m, df)? };
            Some((diff - crit * se, diff + crit * se))
        } else {
            None
        };
        out.push(PairwiseComparisonDetail {
            group_a: ga.index,
            group_b: gb.index,
            label_a: ga.label.clone(),
            label_b: gb.label.clone(),
            mean_diff: diff,
            se_diff: se,
            statistic: t_stat,
            df,
            raw_p,
            adjusted_p_or_q: adj_p,
            is_fdr_q: false,
            ci,
            significant: adj_p.is_finite() && adj_p <= threshold,
        });
    }
    Ok(out)
}

/// Dunn's nonparametric rank-sum pairwise comparison with tie correction and configurable
/// post-adjustment (Bonferroni, Šídák, Holm, or BH/BY/BKY FDR; `DunnRankSum` and parametric FWER
/// defaults map to Dunn–Bonferroni or Šídák, while explicit `None` or `FisherLsd` stays unadjusted).
///
/// Dunn (1964): `z = (Rbar_i - Rbar_j) / sqrt([N(N+1)/12 - sum(t^3 - t) / (12(N - 1))] (1/n_i + 1/n_j))`
/// on mean ranks from the pooled ranking, with the Kruskal–Wallis tie correction as presented in
/// Zar (2010); Dunn's original proposal uses the Bonferroni adjustment (selected here by `DunnRankSum`).
pub fn compare_dunn_ranks(
    group_labels: &[(usize, String)],
    group_n: &[usize],
    mean_ranks: &[f64],
    total_n: usize,
    tie_sum_t3_minus_t: f64,
    spec: &MultiplicitySpec,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = group_labels.len();
    if k < 2 || total_n < 2 {
        return Ok(Vec::new());
    }
    if group_n.len() != k || mean_ranks.len() != k {
        return Err(StatsError::DimensionMismatch { expected: k, actual: group_n.len().min(mean_ranks.len()) });
    }
    if !spec.threshold.is_finite() || spec.threshold <= 0.0 || spec.threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: spec.threshold });
    }
    if !tie_sum_t3_minus_t.is_finite() || tie_sum_t3_minus_t < 0.0 {
        return Err(StatsError::InvalidParameter { name: "tie_sum_t3_minus_t", value: tie_sum_t3_minus_t });
    }
    for (&n_i, &mr) in group_n.iter().zip(mean_ranks.iter()) {
        if n_i == 0 {
            return Err(StatsError::InvalidParameter { name: "group_n", value: 0.0 });
        }
        if !mr.is_finite() {
            return Err(StatsError::InvalidParameter { name: "mean_rank", value: mr });
        }
    }
    let family = validate_multiplicity_family_spec(spec)?;
    let proc_req = MultiplicityProcedure::from(spec.method);
    let pair_indices: Vec<(usize, usize)> = match family {
        MultiplicityFamilySelection::ControlVsAll(control_column) => {
            let ctrl_pos = group_labels
                .iter()
                .position(|(idx, _)| *idx == control_column)
                .ok_or(StatsError::InvalidParameter { name: "control_column", value: control_column as f64 })?;
            (0..k).filter(|&i| i != ctrl_pos).map(|i| (i, ctrl_pos)).collect()
        }
        MultiplicityFamilySelection::SelectedPairs(pairs) => {
            resolve_selected_pair_positions(pairs, |target| group_labels.iter().position(|(idx, _)| *idx == target))?
        }
        MultiplicityFamilySelection::AllPairs => {
            let mut pairs = Vec::with_capacity(k * (k - 1) / 2);
            for i in 0..k {
                for j in (i + 1)..k {
                    pairs.push((i, j));
                }
            }
            pairs
        }
    };

    let nf = total_n as f64;
    let tie_factor = (nf * (nf + 1.0) / 12.0 - tie_sum_t3_minus_t / (12.0 * (nf - 1.0))).max(0.0);
    let m = pair_indices.len();
    let mut raw_p = Vec::with_capacity(m);
    let mut entries = Vec::with_capacity(m);

    for &(i, j) in &pair_indices {
        let ni = group_n[i] as f64;
        let nj = group_n[j] as f64;
        let diff = mean_ranks[i] - mean_ranks[j];
        let se = (tie_factor * (1.0 / ni + 1.0 / nj)).sqrt();
        let (z_stat, p_val) = if se > 0.0 {
            let z = diff / se;
            (z, norm_two_sided_p(z.abs()))
        } else {
            zero_se_stat_p(diff, true)
        };
        raw_p.push(p_val);
        entries.push((&group_labels[i], &group_labels[j], diff, se, z_stat, p_val));
    }

    let proc_eff = match proc_req {
        MultiplicityProcedure::None | MultiplicityProcedure::FisherLsd => MultiplicityProcedure::None,
        MultiplicityProcedure::DunnRankSum
        | MultiplicityProcedure::LinearTrend
        | MultiplicityProcedure::TukeyKramer
        | MultiplicityProcedure::NewmanKeuls
        | MultiplicityProcedure::GamesHowell
        | MultiplicityProcedure::DunnettT3
        | MultiplicityProcedure::Dunnett { .. } => MultiplicityProcedure::Bonferroni,
        MultiplicityProcedure::TamhaneT2 => MultiplicityProcedure::Sidak,
        MultiplicityProcedure::Bonferroni
        | MultiplicityProcedure::Sidak
        | MultiplicityProcedure::HolmBonferroni
        | MultiplicityProcedure::HolmSidak
        | MultiplicityProcedure::BenjaminiHochbergFdr
        | MultiplicityProcedure::BenjaminiYekutieliFdr
        | MultiplicityProcedure::TwoStageBkyFdr => proc_req,
    };
    let (adj, is_fdr) = adjust_p_values(&raw_p, proc_eff, spec.threshold);
    let out = entries
        .into_iter()
        .zip(adj)
        .map(|(((idx_a, lbl_a), (idx_b, lbl_b), diff, se, z_stat, p_val), adj_p)| PairwiseComparisonDetail {
            group_a: *idx_a,
            group_b: *idx_b,
            label_a: lbl_a.clone(),
            label_b: lbl_b.clone(),
            mean_diff: diff,
            se_diff: se,
            statistic: z_stat,
            df: f64::INFINITY,
            raw_p: p_val,
            adjusted_p_or_q: adj_p,
            is_fdr_q: is_fdr,
            ci: None,
            significant: spec.threshold.is_finite() && adj_p.is_finite() && adj_p <= spec.threshold,
        })
        .collect();
    Ok(out)
}

/// Convert pairwise details into a [`curvebook_doc::ResultTab`] and attach
/// [`PairwiseComparisonRecord`] entries (emitting an informational note if `m == 1`).
pub fn build_multiplicity_tab(
    details: &[PairwiseComparisonDetail],
    title: &str,
    p_fmt: curvebook_doc::PValueFormat,
    confidence: curvebook_math::ConfidenceLevel,
) -> ResultTab {
    let headers = vec![
        String::from("Mean Diff."),
        String::from("SE of Diff."),
        String::from("Statistic"),
        String::from("df"),
        String::from("Raw P"),
        String::from("Adjusted P / q"),
        format!("{}% CI of Diff.", confidence.percent()),
        String::from("Significant"),
    ];
    let mut row_labels = Vec::with_capacity(details.len());
    let mut rows = Vec::with_capacity(details.len());
    for d in details {
        row_labels.push(format!("{} vs. {}", d.label_a, d.label_b));
        let ci_cell_val = match d.ci {
            Some((lo, hi)) => crate::common::ci_cell(lo, hi),
            None => ResultCell::Empty,
        };
        let mut adj_cell = crate::common::p_cell_at_alpha(d.adjusted_p_or_q, p_fmt, confidence.alpha());
        if let ResultCell::PValue { stars, .. } = &mut adj_cell {
            if !d.significant {
                *stars = String::from("ns");
            } else if stars == "ns" {
                *stars = String::from("*");
            }
        }
        rows.push(vec![
            crate::common::num_cell(d.mean_diff),
            crate::common::num_cell(d.se_diff),
            crate::common::num_cell(d.statistic),
            crate::common::num_cell(d.df),
            crate::common::p_cell_at_alpha(d.raw_p, p_fmt, confidence.alpha()),
            adj_cell,
            ci_cell_val,
            ResultCell::Boolean(d.significant),
        ]);
    }
    let mut tab =
        crate::common::build_tab("multiple-comparisons", title, curvebook_doc::ResultTabKind::MultipleComparisons, headers, row_labels, rows);
    if details.len() == 1 {
        tab.footnotes.push(String::from("Only m = 1 comparison in family; multiplicity adjustment leaves the unadjusted P value unchanged."));
    }
    tab
}
