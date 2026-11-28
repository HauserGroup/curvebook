//! Lognormal distribution validation, transformation adapters, and back-transformed geometric-mean
//! ratio hypothesis tests across one-sample, two-group, one-way ANOVA (ordinary, Welch/Brown–Forsythe,
//! repeated-measures/REML), and multiple row-wise comparisons.
//!
//! # References
//!
//! - Bland, J. M., & Altman, D. G. (1996). Transformations, means, and confidence intervals.
//!   *BMJ*, 312(7038), 1079. `doi:10.1136/bmj.312.7038.1079`.
//! - Bland, J. M., & Altman, D. G. (1996). Statistics notes: The use of transformation when comparing
//!   two means. *BMJ*, 312(7039), 1153. `doi:10.1136/bmj.312.7039.1153`.
//! - Kirkwood, T. B. L. (1979). Geometric means and measures of dispersion. *Biometrics*, 35,
//!   908–909.
//! - Student (1908). The probable error of a mean. *Biometrika*, 6(1), 1–25.
//!   `doi:10.1093/biomet/6.1.1`.
//! - Welch, B. L. (1947). The generalization of 'Student's' problem when several different population
//!   variances are involved. *Biometrika*, 34(1–2), 28–35. `doi:10.1093/biomet/34.1-2.28`.
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin*, 2(6), 110–114. `doi:10.2307/3002019`.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall. Ch. 7–11.
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - Benjamini, Y., Krieger, A. M., & Yekutieli, D. (2006). Adaptive linear step-up procedures that
//!   control the false discovery rate. *Biometrika*, 93(3), 491–507.
//!   `doi:10.1093/biomet/93.3.491`.

use curvebook_math::ConfidenceLevel;
use curvebook_math::sum::SummaryMoments;
use serde::{Deserialize, Serialize};

use crate::anova::{
    OneWayAnovaResult, RmOneWayAnovaResult, UnequalVarianceAnovaResult, ordinary_one_way_anova, rm_one_way_anova, unequal_variance_one_way_anova,
};
use crate::column::one_sample_t_test;
use crate::common::{t_crit_two_sided, validate_finite_slice};
use crate::error::StatsError;
use crate::multiplicity::{GroupComparisonInput, MultiplicityProcedure, PairwiseComparisonDetail, adjust_p_values};
use crate::two_group::{PairedTResult, UnpairedTResult, lognormal_unpaired_t_test, ratio_paired_t_test, welch_t_test};

/// Count finite values that are impossible under a strictly positive lognormal model (`x <= 0.0`).
#[must_use]
pub fn count_impossible_nonpositive(values: &[f64]) -> usize {
    values.iter().filter(|&&v| v.is_finite() && v <= 0.0).count()
}

/// Validate that every entry in `values` is finite and strictly positive (`> 0.0`), returning
/// the natural-logarithm transformed vector `ln(y_i)` (Bland & Altman 1996).
pub fn validate_and_ln_transform(values: &[f64], param_name: &'static str) -> Result<Vec<f64>, StatsError> {
    validate_finite_slice(values)?;
    if let Some(&bad) = values.iter().find(|&&v| v <= 0.0) {
        return Err(StatsError::InvalidParameter { name: param_name, value: bad });
    }
    Ok(values.iter().map(|v| v.ln()).collect())
}

/// Complete output of a one-sample lognormal / ratio *t*-test comparing a strictly positive sample's
/// geometric mean against a strictly positive hypothetical value `mu0` (Bland & Altman 1996;
/// Kirkwood 1979; Student 1908).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OneSampleRatioTResult {
    /// Sample size `n >= 2`.
    pub n: usize,
    /// Number of non-positive values encountered (zero when test succeeds).
    pub n_impossible_nonpositive: usize,
    /// Strictly positive hypothetical geometric mean `mu0`.
    pub hypothetical_geo_mean: f64,
    /// Sample geometric mean `exp(mean(ln x))`.
    pub geometric_mean: f64,
    /// Multiplicative geometric standard deviation factor `exp(sd(ln x)) >= 1`.
    pub geometric_sd_factor: f64,
    /// Asymmetric confidence interval `exp(mean(ln x) +/- t_crit * sem(ln x))` for the geometric mean.
    pub geometric_mean_ci: (f64, f64),
    /// Ratio of sample geometric mean to hypothetical value `geometric_mean / hypothetical_geo_mean`.
    pub ratio_gm_over_hypothetical: f64,
    /// Asymmetric confidence interval for `ratio_gm_over_hypothetical`.
    pub ratio_ci: (f64, f64),
    /// Arithmetic mean on natural-log scale `mean(ln x)`.
    pub mean_ln: f64,
    /// Sample SD on natural-log scale `sd(ln x)`.
    pub sd_ln: f64,
    /// Standard error of `mean_ln`.
    pub sem_ln: f64,
    /// Discrepancy on natural-log scale `mean(ln x) - ln(mu0)`.
    pub discrepancy_ln: f64,
    /// Symmetric confidence interval for `discrepancy_ln` on natural-log scale.
    pub ci_ln: (f64, f64),
    /// One-sample Student *t* statistic on natural-log scale.
    pub t_statistic: f64,
    /// Degrees of freedom `n - 1`.
    pub df: f64,
    /// Student *t*(df) *P* value (`two_tailed` or directional one-tailed).
    pub p_value: f64,
    /// Whether `p_value` is two-sided (`true`) or one-sided (`false`).
    pub two_tailed: bool,
    /// Bias-corrected Hedges' *g* on natural-log scale (`None` when `n = 2`).
    pub hedges_g: Option<f64>,
}

/// One-sample lognormal / ratio *t*-test comparing `ln(x_i)` against `ln(hypothetical_geo_mean)`
/// (Bland & Altman 1996; Student 1908).
pub fn one_sample_ratio_t_test(
    values: &[f64],
    hypothetical_geo_mean: f64,
    two_tailed: bool,
    confidence: ConfidenceLevel,
) -> Result<OneSampleRatioTResult, StatsError> {
    if !hypothetical_geo_mean.is_finite() || hypothetical_geo_mean <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "hypothetical_geo_mean", value: hypothetical_geo_mean });
    }
    let logs = validate_and_ln_transform(values, "lognormal_x")?;
    let ln_mu0 = hypothetical_geo_mean.ln();
    let res = one_sample_t_test(&logs, ln_mu0, two_tailed, confidence)?;
    let moments = SummaryMoments::from_slice(&logs)?;
    let sd_ln = moments.sample_sd()?;
    let sem_ln = moments.sem()?;
    let t_crit = t_crit_two_sided(confidence, res.df)?;
    let gm_ci = ((res.sample_mean - t_crit * sem_ln).exp(), (res.sample_mean + t_crit * sem_ln).exp());
    let ratio_ci = (res.ci.0.exp(), res.ci.1.exp());

    Ok(OneSampleRatioTResult {
        n: res.n,
        n_impossible_nonpositive: 0,
        hypothetical_geo_mean,
        geometric_mean: res.sample_mean.exp(),
        geometric_sd_factor: sd_ln.exp(),
        geometric_mean_ci: gm_ci,
        ratio_gm_over_hypothetical: res.discrepancy.exp(),
        ratio_ci,
        mean_ln: res.sample_mean,
        sd_ln,
        sem_ln,
        discrepancy_ln: res.discrepancy,
        ci_ln: res.ci,
        t_statistic: res.t_statistic,
        df: res.df,
        p_value: res.p_value,
        two_tailed,
        hedges_g: res.hedges_g,
    })
}

/// Unpaired lognormal Welch *t*-test (Welch–Satterthwaite *t*-test performed on `ln(x)` for
/// strictly positive observations; Welch 1947; Satterthwaite 1946; Bland & Altman 1996).
///
/// Back-transforms `mean(ln A) - mean(ln B)` and its confidence interval into the geometric mean
/// ratio `GM(A) / GM(B)` and its asymmetric confidence interval.
pub fn lognormal_welch_t_test(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<UnpairedTResult, StatsError> {
    let la = validate_and_ln_transform(a, "lognormal_x")?;
    let lb = validate_and_ln_transform(b, "lognormal_x")?;
    let mut out = welch_t_test(&la, &lb, two_tailed, confidence)?;
    out.method_label = "Lognormal Welch's t test";
    out.geometric_mean_ratio = Some(out.difference.exp());
    out.geometric_mean_ratio_ci = Some((out.ci.0.exp(), out.ci.1.exp()));
    Ok(out)
}

/// Back-transformed pairwise contrast from a natural-log-scale ANOVA or mixed-model comparison
/// (Bland & Altman 1996).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LognormalPairwiseContrast {
    /// Zero-based index of group A.
    pub group_a: usize,
    /// Zero-based index of group B.
    pub group_b: usize,
    /// Display label of group A.
    pub label_a: String,
    /// Display label of group B.
    pub label_b: String,
    /// Difference on natural-log scale `mean(ln A) - mean(ln B)`.
    pub log_mean_diff: f64,
    /// Standard error of `log_mean_diff`.
    pub se_log_diff: f64,
    /// Geometric mean ratio `exp(log_mean_diff) = GM(A) / GM(B)`.
    pub geometric_mean_ratio: f64,
    /// Asymmetric confidence interval `exp(ci_log)` for `geometric_mean_ratio`.
    pub geometric_mean_ratio_ci: Option<(f64, f64)>,
    /// Test statistic (`t` or `q`).
    pub statistic: f64,
    /// Degrees of freedom.
    pub df: f64,
    /// Unadjusted *P* value.
    pub raw_p: f64,
    /// Multiplicity-adjusted *P* or FDR *q* value.
    pub adjusted_p_or_q: f64,
    /// True when `adjusted_p_or_q` is an FDR *q* value.
    pub is_fdr_q: bool,
    /// True when the comparison is significant at the configured threshold.
    pub significant: bool,
}

/// Back-transform natural-log-scale pairwise contrasts into geometric mean ratios (`A / B`) and
/// asymmetric confidence intervals (Bland & Altman 1996).
#[must_use]
pub fn back_transform_pairwise_contrasts(details: &[PairwiseComparisonDetail]) -> Vec<LognormalPairwiseContrast> {
    details
        .iter()
        .map(|d| LognormalPairwiseContrast {
            group_a: d.group_a,
            group_b: d.group_b,
            label_a: d.label_a.clone(),
            label_b: d.label_b.clone(),
            log_mean_diff: d.mean_diff,
            se_log_diff: d.se_diff,
            geometric_mean_ratio: d.mean_diff.exp(),
            geometric_mean_ratio_ci: d.ci.map(|(lo, hi)| (lo.exp(), hi.exp())),
            statistic: d.statistic,
            df: d.df,
            raw_p: d.raw_p,
            adjusted_p_or_q: d.adjusted_p_or_q,
            is_fdr_q: d.is_fdr_q,
            significant: d.significant,
        })
        .collect()
}

/// Complete output of ordinary lognormal one-way ANOVA evaluated on `ln(y)` (Zar 2010; Bland & Altman 1996).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LognormalOneWayResult {
    /// Ordinary one-way ANOVA result evaluated on natural-log-transformed observations.
    pub log_anova: OneWayAnovaResult,
    /// Per-group geometric means `exp(mean(ln y_g))`.
    pub group_geometric_means: Vec<f64>,
    /// Per-group geometric SD factors `exp(sd(ln y_g))`.
    pub group_geometric_sd_factors: Vec<f64>,
}

/// Ordinary lognormal one-way ANOVA on strictly positive groups (Zar 2010; Bland & Altman 1996).
pub fn lognormal_one_way_ordinary(groups: &[(&str, &[f64])], confidence: ConfidenceLevel) -> Result<LognormalOneWayResult, StatsError> {
    let mut log_owned: Vec<Vec<f64>> = Vec::with_capacity(groups.len());
    for &(_, vals) in groups {
        log_owned.push(validate_and_ln_transform(vals, "lognormal_x")?);
    }
    let log_pairs: Vec<(&str, &[f64])> = groups.iter().zip(log_owned.iter()).map(|((lbl, _), v)| (*lbl, v.as_slice())).collect();
    let log_anova = ordinary_one_way_anova(&log_pairs, confidence)?;
    let group_geometric_means = log_anova.group_means.iter().map(|&m| m.exp()).collect();
    let group_geometric_sd_factors = log_anova.group_sds.iter().map(|&s| s.exp()).collect();
    Ok(LognormalOneWayResult { log_anova, group_geometric_means, group_geometric_sd_factors })
}

/// Complete output of unequal-variance (Welch & Brown–Forsythe) lognormal one-way ANOVA evaluated on `ln(y)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LognormalWelchOneWayResult {
    /// Base log-scale one-way ANOVA statistics (means, SDs, variance homogeneity tests).
    pub log_base: OneWayAnovaResult,
    /// Welch and Brown–Forsythe unequal-variance one-way ANOVA evaluated on `ln(y)`.
    pub log_unequal_var: UnequalVarianceAnovaResult,
    /// Per-group geometric means `exp(mean(ln y_g))`.
    pub group_geometric_means: Vec<f64>,
    /// Per-group geometric SD factors `exp(sd(ln y_g))`.
    pub group_geometric_sd_factors: Vec<f64>,
}

/// Unequal-variance Welch and Brown–Forsythe lognormal one-way ANOVA on strictly positive groups
/// (Welch 1947; Bland & Altman 1996; Zar 2010).
pub fn lognormal_one_way_welch_bf(groups: &[(&str, &[f64])], confidence: ConfidenceLevel) -> Result<LognormalWelchOneWayResult, StatsError> {
    let ord = lognormal_one_way_ordinary(groups, confidence)?;
    let inputs: Vec<GroupComparisonInput> = ord
        .log_anova
        .group_labels
        .iter()
        .enumerate()
        .map(|(i, lbl)| GroupComparisonInput {
            index: i,
            label: lbl.clone(),
            n: ord.log_anova.group_n[i] as f64,
            mean: ord.log_anova.group_means[i],
            sd: ord.log_anova.group_sds[i],
        })
        .collect();
    let log_unequal_var = unequal_variance_one_way_anova(&inputs)?;
    Ok(LognormalWelchOneWayResult {
        log_base: ord.log_anova,
        log_unequal_var,
        group_geometric_means: ord.group_geometric_means,
        group_geometric_sd_factors: ord.group_geometric_sd_factors,
    })
}

/// Complete output of balanced repeated-measures lognormal one-way ANOVA evaluated on `ln(y)`
/// (Maxwell & Delaney 2004; Bland & Altman 1996).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LognormalRmOneWayResult {
    /// Repeated-measures one-way ANOVA with sphericity metrics evaluated on `ln(y)`.
    pub log_rm: RmOneWayAnovaResult,
    /// Per-condition geometric means `exp(mean(ln y_c))`.
    pub condition_geometric_means: Vec<f64>,
    /// Per-condition geometric SD factors `exp(sd(ln y_c))`.
    pub condition_geometric_sd_factors: Vec<f64>,
}

/// Balanced repeated-measures lognormal one-way ANOVA on strictly positive subject blocks
/// (Maxwell & Delaney 2004; Bland & Altman 1996).
pub fn lognormal_one_way_rm(blocks: &[Vec<f64>]) -> Result<LognormalRmOneWayResult, StatsError> {
    if blocks.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let mut log_blocks = Vec::with_capacity(blocks.len());
    for row in blocks {
        log_blocks.push(validate_and_ln_transform(row, "lognormal_x")?);
    }
    let log_rm = rm_one_way_anova(&log_blocks)?;
    let k = log_blocks[0].len();
    let mut condition_geometric_means = Vec::with_capacity(k);
    let mut condition_geometric_sd_factors = Vec::with_capacity(k);
    for c in 0..k {
        let col: Vec<f64> = log_blocks.iter().map(|b| b[c]).collect();
        let m = SummaryMoments::from_slice(&col)?;
        condition_geometric_means.push(m.mean.exp());
        condition_geometric_sd_factors.push(m.sample_sd().unwrap_or(0.0).exp());
    }
    Ok(LognormalRmOneWayResult { log_rm, condition_geometric_means, condition_geometric_sd_factors })
}

/// Single row comparison entry for multiple lognormal or ratio-paired *t*-tests
/// (Bland & Altman 1996; Benjamini, Krieger & Yekutieli 2006).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LognormalMultipleRowEntry {
    /// Zero-based table row index tested.
    pub row_index: usize,
    /// Human-readable row label.
    pub label: String,
    /// Geometric mean of group A (`exp(mean(ln A))`).
    pub geometric_mean_a: f64,
    /// Geometric mean of group B (`exp(mean(ln B))`).
    pub geometric_mean_b: f64,
    /// Mean difference on natural-log scale `mean(ln A) - mean(ln B)`.
    pub log_difference: f64,
    /// Geometric mean ratio `exp(log_difference) = GM(A) / GM(B)`.
    pub geometric_mean_ratio: f64,
    /// Asymmetric confidence interval `exp(ci_log)` for `geometric_mean_ratio`.
    pub geometric_mean_ratio_ci: (f64, f64),
    /// Student or Welch *t* statistic on natural-log scale.
    pub statistic: f64,
    /// Degrees of freedom.
    pub df: f64,
    /// Unadjusted two-sided *P* value.
    pub raw_p: f64,
    /// Familywise-error adjusted *P* or FDR-adjusted *q* value across all tested rows.
    pub adjusted_p_or_q: f64,
    /// True when the row passes the multiplicity threshold.
    pub significant: bool,
}

/// Run one lognormal unpaired (Student or Welch) or ratio-paired *t*-test per row across two groups
/// and apply raw-*P* multiplicity adjustment (Holm–Šídák, BH, BY, two-stage BKY, etc.; Bland &
/// Altman 1996; Benjamini, Krieger & Yekutieli 2006).
#[allow(clippy::too_many_arguments)]
pub fn multiple_row_lognormal_t_tests(
    row_labels: &[String],
    group_a_rows: &[Vec<f64>],
    group_b_rows: &[Vec<f64>],
    paired: bool,
    welch: bool,
    procedure: MultiplicityProcedure,
    threshold: f64,
    confidence: ConfidenceLevel,
) -> Result<Vec<LognormalMultipleRowEntry>, StatsError> {
    if matches!(
        procedure,
        MultiplicityProcedure::TukeyKramer
            | MultiplicityProcedure::NewmanKeuls
            | MultiplicityProcedure::Duncan
            | MultiplicityProcedure::Dunnett { .. }
            | MultiplicityProcedure::DunnettT3
            | MultiplicityProcedure::GamesHowell
            | MultiplicityProcedure::DunnRankSum
            | MultiplicityProcedure::LinearTrend
            | MultiplicityProcedure::TamhaneT2
    ) {
        return Err(StatsError::IncompatibleAnalysisSpec {
            expected: "raw p-value multiplicity procedure (None, Bonferroni, Sidak, HolmBonferroni, HolmSidak, BenjaminiHochbergFdr, BenjaminiYekutieliFdr, or TwoStageBkyFdr)",
        });
    }
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: threshold });
    }
    if group_a_rows.len() != group_b_rows.len() {
        return Err(StatsError::DimensionMismatch { expected: group_a_rows.len(), actual: group_b_rows.len() });
    }

    struct LognormalRowEval {
        gm_a: f64,
        gm_b: f64,
        log_diff: f64,
        gmr: f64,
        gmr_ci: (f64, f64),
        t_stat: f64,
        df: f64,
        p_val: f64,
    }

    let mut raw_p = Vec::new();
    let mut staged = Vec::new();
    for (idx, (ra, rb)) in group_a_rows.iter().zip(group_b_rows.iter()).enumerate() {
        let row_res: Result<LognormalRowEval, StatsError> = if paired {
            ratio_paired_t_test(ra, rb, true, confidence).and_then(|r: PairedTResult| {
                let la = validate_and_ln_transform(ra, "lognormal_x")?;
                let lb = validate_and_ln_transform(rb, "lognormal_x")?;
                let gm_a = SummaryMoments::from_slice(&la)?.mean.exp();
                let gm_b = SummaryMoments::from_slice(&lb)?.mean.exp();
                let gmr = r.geometric_mean_ratio.unwrap_or_else(|| r.mean_of_differences.exp());
                let gmr_ci = r.geometric_mean_ratio_ci.unwrap_or_else(|| (r.ci.0.exp(), r.ci.1.exp()));
                Ok(LognormalRowEval { gm_a, gm_b, log_diff: r.mean_of_differences, gmr, gmr_ci, t_stat: r.t_statistic, df: r.df, p_val: r.p_value })
            })
        } else {
            let res = if welch { lognormal_welch_t_test(ra, rb, true, confidence) } else { lognormal_unpaired_t_test(ra, rb, true, confidence) };
            res.map(|r| {
                let gmr = r.geometric_mean_ratio.unwrap_or_else(|| r.difference.exp());
                let gmr_ci = r.geometric_mean_ratio_ci.unwrap_or_else(|| (r.ci.0.exp(), r.ci.1.exp()));
                LognormalRowEval {
                    gm_a: r.mean_a.exp(),
                    gm_b: r.mean_b.exp(),
                    log_diff: r.difference,
                    gmr,
                    gmr_ci,
                    t_stat: r.t_statistic,
                    df: r.df,
                    p_val: r.p_value,
                }
            })
        };

        if let Ok(ev) = row_res
            && ev.p_val.is_finite()
        {
            let lbl = row_labels.get(idx).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", idx + 1));
            raw_p.push(ev.p_val);
            staged.push((idx, lbl, ev.gm_a, ev.gm_b, ev.log_diff, ev.gmr, ev.gmr_ci, ev.t_stat, ev.df, ev.p_val));
        }
    }

    let (adj, _) = adjust_p_values(&raw_p, procedure, threshold);
    Ok(staged
        .into_iter()
        .zip(adj)
        .map(
            |(
                (
                    row_index,
                    label,
                    geometric_mean_a,
                    geometric_mean_b,
                    log_difference,
                    geometric_mean_ratio,
                    geometric_mean_ratio_ci,
                    statistic,
                    df,
                    raw_p,
                ),
                adjusted_p_or_q,
            )| {
                LognormalMultipleRowEntry {
                    row_index,
                    label,
                    geometric_mean_a,
                    geometric_mean_b,
                    log_difference,
                    geometric_mean_ratio,
                    geometric_mean_ratio_ci,
                    statistic,
                    df,
                    raw_p,
                    adjusted_p_or_q,
                    significant: adjusted_p_or_q.is_finite() && adjusted_p_or_q <= threshold,
                }
            },
        )
        .collect())
}
