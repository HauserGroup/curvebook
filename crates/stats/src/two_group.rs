//! Two-group parametric and nonparametric hypothesis tests, standardized effect sizes with
//! exact noncentral-*t* confidence intervals, variance homogeneity tests, and Bland–Altman agreement.
//!
//! # References
//!
//! Full bibliographic details for the methods implemented in this module; individual items cite
//! them in short `Author (Year)` form.
//!
//! - Student (1908). The probable error of a mean. *Biometrika* 6(1):1–25. doi:10.1093/biomet/6.1.1
//! - Welch, B. L. (1947). The generalization of 'Student's' problem when several different population
//!   variances are involved. *Biometrika* 34(1–2):28–35. doi:10.1093/biomet/34.1-2.28
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114. doi:10.2307/3002019
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall. Ch. 7 (one-sample),
//!   ch. 8 (two-sample, including the variance-ratio *F* test), ch. 9 (paired-sample hypotheses).
//! - Levene, H. (1960). Robust tests for equality of variances. In I. Olkin et al. (Eds.),
//!   *Contributions to Probability and Statistics: Essays in Honor of Harold Hotelling*, pp. 278–292.
//!   Stanford University Press.
//! - Brown, M. B., & Forsythe, A. B. (1974). Robust tests for the equality of variances.
//!   *Journal of the American Statistical Association* 69(346):364–367. doi:10.1080/01621459.1974.10482955
//! - Cohen, J. (1988). *Statistical Power Analysis for the Behavioral Sciences* (2nd ed.). Lawrence Erlbaum.
//! - Hedges, L. V. (1981). Distribution theory for Glass's estimator of effect size and related
//!   estimators. *Journal of Educational Statistics* 6(2):107–128. doi:10.3102/10769986006002107
//! - Glass, G. V. (1976). Primary, secondary, and meta-analysis of research. *Educational Researcher*
//!   5(10):3–8. doi:10.3102/0013189X005010003
//! - Steiger, J. H., & Fouladi, R. T. (1997). Noncentrality interval estimation and the evaluation of
//!   statistical models. In L. L. Harlow, S. A. Mulaik, & J. H. Steiger (Eds.), *What If There Were No
//!   Significance Tests?*, pp. 221–257. Lawrence Erlbaum.
//! - Steiger, J. H. (2004). Beyond the F test: Effect size confidence intervals and tests of close
//!   fit in the analysis of variance and contrast analysis. *Psychological Methods* 9(2):164–182.
//!   doi:10.1037/1082-989X.9.2.164
//! - Cumming, G., & Finch, S. (2001). A primer on the understanding, use, and calculation of confidence
//!   intervals that are based on central and noncentral distributions. *Educational and Psychological
//!   Measurement* 61(4):532–574.
//! - Rosenthal, R. (1991). *Meta-Analytic Procedures for Social Research* (rev. ed.). Sage.
//! - Bland, J. M., & Altman, D. G. (1996). Statistics notes: The use of transformation when comparing
//!   two means. *BMJ* 312(7039):1153. doi:10.1136/bmj.312.7039.1153
//! - Wilcoxon, F. (1945). Individual comparisons by ranking methods. *Biometrics Bulletin* 1(6):80–83.
//!   doi:10.2307/3001968
//! - Mann, H. B., & Whitney, D. R. (1947). On a test of whether one of two random variables is
//!   stochastically larger than the other. *Annals of Mathematical Statistics* 18(1):50–60.
//!   doi:10.1214/aoms/1177730491
//! - Pratt, J. W. (1959). Remarks on zeros and ties in the Wilcoxon signed rank procedures.
//!   *Journal of the American Statistical Association* 54(287):655–667. doi:10.1080/01621459.1959.10501526
//! - Hodges, J. L., & Lehmann, E. L. (1963). Estimates of location based on rank tests.
//!   *Annals of Mathematical Statistics* 34(2):598–611. doi:10.1214/aoms/1177704172
//! - Monahan, J. F. (1984). Algorithm 616: Fast computation of the Hodges–Lehmann location estimator.
//!   *ACM Transactions on Mathematical Software* 10(3):265–270. doi:10.1145/1271.319414
//! - Hollander, M., Wolfe, D. A., & Chicken, E. (2014). *Nonparametric Statistical Methods* (3rd ed.).
//!   Wiley. Ch. 3 (paired/one-sample location: signed-rank test, Walsh-average estimator), ch. 4
//!   (two-sample location: rank-sum test, Hodges–Lehmann shift estimator, Moses interval).
//! - Conover, W. J. (1999). *Practical Nonparametric Statistics* (3rd ed.). Wiley.
//! - Dixon, W. J., & Mood, A. M. (1946). The statistical sign test. *Journal of the American
//!   Statistical Association* 41:557–566.
//! - Cliff, N. (1993). Dominance statistics: Ordinal analyses to answer ordinal questions.
//!   *Psychological Bulletin* 114(3):494–509. doi:10.1037/0033-2909.114.3.494
//! - Wendt, H. W. (1972). Dealing with a common problem in social science: A simplified rank-biserial
//!   coefficient of correlation based on the U statistic. *European Journal of Social Psychology* 2.
//!   doi:10.1002/ejsp.2420020412
//! - Kerby, D. S. (2014). The simple difference formula: An approach to teaching nonparametric
//!   correlation. *Comprehensive Psychology* 3. doi:10.2466/11.IT.3.1
//! - Smirnov, N. (1948). Table for estimating the goodness of fit of empirical distributions.
//!   *Annals of Mathematical Statistics* 19(2):279–281. doi:10.1214/aoms/1177730256
//! - Hodges, J. L. (1958). The significance probability of the Smirnov two-sample test.
//!   *Arkiv för Matematik* 3(5):469–486. doi:10.1007/BF02589501
//! - Bland, J. M., & Altman, D. G. (1986). Statistical methods for assessing agreement between two
//!   methods of clinical measurement. *Lancet* 327(8476):307–310. doi:10.1016/S0140-6736(86)90837-8
//! - Bland, J. M., & Altman, D. G. (1999). Measuring agreement in method comparison studies.
//!   *Statistical Methods in Medical Research* 8(2):135–160. doi:10.1177/096228029900800204

use curvebook_doc::{
    AnalysisSpec, BlandAltmanSpec, Cell, CellValue, ChecklistItem, ChecklistStatus, CurveSamplePoint, DataSheet, FittedCurveOverlay,
    MultipleRowTestsSpec, MultipleVariablesTable, PValueFormat, PairwiseComparisonRecord, ResultCell, ResultSheet, ResultTabKind, TableArchetype,
    TwoGroupMethod, TwoGroupSpec, VariableRole, XColumnMode,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{f_sf, norm_sf, norm_two_sided_isf, student_t_cdf, student_t_sf, student_t_two_sided_p};
use curvebook_math::noncentral_t_ci;
use curvebook_math::special::inc_beta::reg_beta;
use curvebook_math::sum::{NeumaierSum, SummaryMoments, pearson_r};
use serde::{Deserialize, Serialize};

use crate::column::{
    OneSampleTResult, exact_binomial_median_ci, hedges_g_with_ci, hedges_j_factor, hodges_lehmann_one_sample_with_coverage, ks_asymptotic_p,
    shapiro_wilk, summarize_pre_summarized_points, wilcoxon_exact_p, wilcoxon_max_achievable_coverage_percent,
};
use crate::common::{
    KvTabBuilder, TabBuilder, append_ignored_text_checklist_and_footnote, append_listwise_filtered_checklist_and_footnote, checklist_item,
    extract_numeric_columns, extract_paired_values_with_report, init_result_sheet, is_ignored_non_numeric_cell, num_cell, p_cell_at_alpha,
    push_scalar, quantile_type6_sorted, quartiles_hinge, rank_with_ties, sanitize_result_sheet_floats, select_kth_pairwise_sum, sorted_finite,
    t_crit_two_sided, try_resolve_selected_indices, validate_finite_slice,
};
use crate::error::StatsError;
use crate::multiplicity::{MultiplicityProcedure, adjust_p_values};

// ============================================================================
// Variance Homogeneity Tests (F-test, Levene, Brown-Forsythe)
// ============================================================================

/// Result of an *F*-test comparing two sample variances.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VarianceFTestResult {
    /// Variance ratio $s_{\max}^2 / s_{\min}^2 \ge 1.0$ (`f64::INFINITY` when smaller variance is zero; `1.0` when both are zero).
    pub f_statistic: f64,
    /// Numerator degrees of freedom `n_max - 1` associated with the larger sample variance.
    pub df_num: f64,
    /// Denominator degrees of freedom `n_min - 1` associated with the smaller sample variance.
    pub df_den: f64,
    /// Two-sided *F* test *P* value `min(1, 2 * P(F >= f_statistic))` (`0.0` when ratio infinite, `1.0` when variances equal).
    pub p_value: f64,
}

/// Two-sided *F*-test for equality of variances (`s_max^2 / s_min^2`).
///
/// Variance-ratio test with the larger sample variance in the numerator and the two-sided *p*-value
/// taken as `min(1, 2 * P(F >= f))` (Zar 2010, ch. 8). Sensitive to non-normality; see
/// [`levene_variance_test`] for robust alternatives.
pub fn f_test_variances(a: &[f64], b: &[f64]) -> Result<VarianceFTestResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "F-test group A", required: 2, actual: a.len() });
    }
    if b.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "F-test group B", required: 2, actual: b.len() });
    }
    let va = SummaryMoments::from_slice(a)?.sample_variance()?;
    let vb = SummaryMoments::from_slice(b)?.sample_variance()?;
    if va == 0.0 && vb == 0.0 {
        return Ok(VarianceFTestResult { f_statistic: 1.0, df_num: (a.len() - 1) as f64, df_den: (b.len() - 1) as f64, p_value: 1.0 });
    }
    let (f_stat, df_n, df_d) = if va >= vb {
        let f = if vb > 0.0 { va / vb } else { f64::INFINITY };
        (f, (a.len() - 1) as f64, (b.len() - 1) as f64)
    } else {
        let f = if va > 0.0 { vb / va } else { f64::INFINITY };
        (f, (b.len() - 1) as f64, (a.len() - 1) as f64)
    };
    let p_value = if f_stat.is_finite() { (2.0 * f_sf(f_stat, df_n, df_d)?).min(1.0) } else { 0.0 };
    Ok(VarianceFTestResult { f_statistic: f_stat, df_num: df_n, df_den: df_d, p_value })
}

/// Output of Levene's (mean-centered) or Brown–Forsythe's (median-centered) test of equal variances.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LeveneTestResult {
    /// True for Brown–Forsythe deviations from group medians; false for Levene deviations from group means.
    pub use_median: bool,
    /// One-way ANOVA *F*-statistic computed on absolute within-group deviations `|x_ij - c_i|`.
    pub f_statistic: f64,
    /// Numerator degrees of freedom `k - 1` (`1.0` for two groups).
    pub df_between: f64,
    /// Denominator degrees of freedom `sum(n_i) - k`.
    pub df_within: f64,
    /// Upper-tail $F(\text{df\_between}, \text{df\_within})$ *P* value testing equal population variances.
    pub p_value: f64,
}

/// Levene (`use_median = false`) or Brown–Forsythe (`use_median = true`) homogeneity-of-variance test.
///
/// One-way ANOVA *F* on absolute deviations `|x_ij - c_i|`, where `c_i` is the group mean
/// (Levene 1960) or the group median (Brown & Forsythe 1974).
///
/// When every group's absolute deviations are identical (zero within-group spread of `|x_ij - c_i|`)
/// the *F* ratio is undefined; this function keeps its historical convention of returning
/// `F = 0, p = 1` (no between-group spread either) or `F = +Inf, p = 0`. The standalone analysis in
/// [`crate::variance_homogeneity`] instead rejects that case with [`StatsError::ZeroVariance`].
pub fn levene_variance_test(groups: &[&[f64]], use_median: bool) -> Result<LeveneTestResult, StatsError> {
    Ok(levene_variance_core(groups, use_median)?.test)
}

/// Per-group intermediate quantities of the Levene / Brown–Forsythe computation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LeveneCore {
    /// Omnibus test result (`F`, `df`, `p`).
    pub(crate) test: LeveneTestResult,
    /// Per-group centre `c_i` (mean or median).
    pub(crate) centers: Vec<f64>,
    /// Per-group mean absolute deviation `mean_j |x_ij - c_i|`.
    pub(crate) mean_abs_deviations: Vec<f64>,
    /// Within-group sum of squares of the absolute deviations (`SS_within` of the ANOVA on `z_ij`).
    pub(crate) ss_within: f64,
    /// Rounding-noise floor for [`Self::ss_within`]: `N * (8 * eps * max_ij |x_ij|)^2`.
    pub(crate) ss_within_noise_floor: f64,
}

/// Shared Levene / Brown–Forsythe computation (Levene 1960; Brown & Forsythe 1974) returning the
/// omnibus result together with per-group centres and the within-group dispersion of `|x_ij - c_i|`.
pub(crate) fn levene_variance_core(groups: &[&[f64]], use_median: bool) -> Result<LeveneCore, StatsError> {
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Levene test", required: 2, actual: groups.len() });
    }
    let mut z_groups = Vec::with_capacity(groups.len());
    let mut all_z = Vec::new();
    let mut centers = Vec::with_capacity(groups.len());
    let mut max_abs = 0.0_f64;
    for grp in groups {
        validate_finite_slice(grp)?;
        if grp.len() < 2 {
            return Err(StatsError::InsufficientSampleSize { context: "Levene group", required: 2, actual: grp.len() });
        }
        let center = if use_median {
            let s = sorted_finite(grp);
            quantile_type6_sorted(&s, 0.5).or_else(|| s.first().copied()).ok_or(StatsError::EmptyInput)?
        } else {
            SummaryMoments::from_slice(grp)?.mean
        };
        max_abs = grp.iter().fold(max_abs, |m, &v| m.max(v.abs()));
        let z: Vec<f64> = grp.iter().map(|&v| (v - center).abs()).collect();
        all_z.extend_from_slice(&z);
        z_groups.push(z);
        centers.push(center);
    }
    let total_n = all_z.len();
    let k = groups.len();
    let grand = SummaryMoments::from_slice(&all_z)?;
    let mut ss_b = NeumaierSum::new();
    let mut ss_w = NeumaierSum::new();
    let mut mean_abs_deviations = Vec::with_capacity(k);
    for zg in &z_groups {
        let gm = SummaryMoments::from_slice(zg)?;
        let d = gm.mean - grand.mean;
        ss_b.add((gm.count as f64) * d * d);
        ss_w.add(gm.css);
        mean_abs_deviations.push(gm.mean);
    }
    let df_b = (k - 1) as f64;
    let df_w = (total_n - k) as f64;
    let ssb = ss_b.total();
    let ssw = ss_w.total();
    let (f_stat, p_val) = if ssw > 0.0 {
        let f = (ssb / df_b) / (ssw / df_w);
        (f, f_sf(f, df_b, df_w)?)
    } else if ssb == 0.0 {
        (0.0, 1.0)
    } else {
        (f64::INFINITY, 0.0)
    };
    let noise = 8.0 * f64::EPSILON * max_abs;
    Ok(LeveneCore {
        test: LeveneTestResult { use_median, f_statistic: f_stat, df_between: df_b, df_within: df_w, p_value: p_val },
        centers,
        mean_abs_deviations,
        ss_within: ssw,
        ss_within_noise_floor: (total_n as f64) * noise * noise,
    })
}

// ============================================================================
// Parametric Two-Group Comparisons & Standardized Effect Sizes
// ============================================================================

/// Standardized effect sizes and exact noncentral-*t* confidence intervals for two-group tests.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TwoGroupEffectSizes {
    /// Standardized mean difference `(mean_a - mean_b) / s_pooled` (or `s_welch` / unpooled as specified; Cohen 1988).
    pub cohens_d: f64,
    /// Exact noncentral-*t* confidence interval `(lower, upper)` for `cohens_d` at configured `ConfidenceLevel` (Steiger & Fouladi 1997; `None` if inversion fails or Welch summary lacks exact df).
    pub cohens_d_ci: Option<(f64, f64)>,
    /// Small-sample bias-corrected standardized difference `J(df) * cohens_d` (Hedges 1981).
    pub hedges_g: f64,
    /// Exact noncentral-*t* CI for `hedges_g` scaled by `J(df)` at configured `ConfidenceLevel` (`None` when `cohens_d_ci` is `None`).
    pub hedges_g_ci: Option<(f64, f64)>,
    /// Glass's $\Delta = (\bar{x}_a - \bar{x}_b) / s_a$ (`None` when group A SD `s_a` is zero or unavailable; never substitutes `s_b`).
    pub glass_delta: Option<f64>,
    /// Approximate/noncentral CI for Glass's $\Delta$ (`None` when unavailable).
    pub glass_delta_ci: Option<(f64, f64)>,
    /// Variance explained `eta^2 = t^2 / (t^2 + df)` in `[0, 1]`.
    pub r_squared: f64,
}

/// Complete output of an unpaired Student *t*, Welch *t*, or lognormal unpaired *t*-test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnpairedTResult {
    /// Descriptive test method label (`"Unpaired Student t test"`, `"Welch's t test"`, or `"Lognormal unpaired t test"`).
    pub method_label: &'static str,
    /// Sample size in group A (raw count or rounded `n_a` from summary inputs).
    pub n_a: usize,
    /// Sample size in group B.
    pub n_b: usize,
    /// Sample arithmetic mean of group A (`mean(ln A)` when lognormal).
    pub mean_a: f64,
    /// Sample arithmetic mean of group B (`mean(ln B)` when lognormal).
    pub mean_b: f64,
    /// Sample standard deviation (`n - 1` denominator) of group A.
    pub sd_a: f64,
    /// Sample standard deviation (`n - 1` denominator) of group B.
    pub sd_b: f64,
    /// Signed difference `mean_a - mean_b` (on raw scale, or log scale for lognormal *t*).
    pub difference: f64,
    /// Standard error of `difference` (`s_p sqrt(1/n_a + 1/n_b)` for Student; `sqrt(s_a^2/n_a + s_b^2/n_b)` for Welch).
    pub se_difference: f64,
    /// Two-sided `t(df)` confidence interval `(lower, upper)` for `difference` at configured `ConfidenceLevel`.
    pub ci: (f64, f64),
    /// Two-sample *t* statistic `difference / se_difference`.
    pub t_statistic: f64,
    /// Residual degrees of freedom (`n_a + n_b - 2` for Student; Welch–Satterthwaite effective *df* for Welch).
    pub df: f64,
    /// Student *t*(df) *P* value (`two_tailed` two-sided vs directional one-sided matching `t_statistic` sign).
    pub p_value: f64,
    /// True when `p_value` is two-sided; false when directional one-tailed.
    pub two_tailed: bool,
    /// Variance-ratio *F*-test comparing `sd_a^2` and `sd_b^2` (Zar 2010).
    pub f_test: VarianceFTestResult,
    /// Brown–Forsythe (median-based Levene) test when raw replicates are available; `None` for summary inputs.
    pub levene_median: Option<LeveneTestResult>,
    /// Standardized effect sizes (`cohens_d`, `hedges_g`, `glass_delta`, `r_squared`) and exact noncentral-*t* CIs.
    pub effect_sizes: TwoGroupEffectSizes,
    /// Present for `LognormalUnpairedT`: geometric mean ratio `exp(mean_ln_a - mean_ln_b)` and CI.
    pub geometric_mean_ratio: Option<f64>,
    /// Present for `LognormalUnpairedT`: two-sided exponentiated *t*(df) CI `exp(ci)` for `geometric_mean_ratio`.
    pub geometric_mean_ratio_ci: Option<(f64, f64)>,
}

fn t_tail_p(t_stat: f64, df: f64, two_tailed: bool) -> Result<f64, StatsError> {
    if two_tailed {
        Ok(student_t_two_sided_p(t_stat, df)?)
    } else if t_stat >= 0.0 {
        Ok(student_t_sf(t_stat, df)?)
    } else {
        Ok(student_t_cdf(t_stat, df)?)
    }
}

/// Variance-ratio *F* test (Zar 2010, ch. 8) from summary SDs; same convention as [`f_test_variances`].
fn f_test_from_summary(sd_a: f64, n_a: f64, sd_b: f64, n_b: f64) -> Result<VarianceFTestResult, StatsError> {
    let va = sd_a * sd_a;
    let vb = sd_b * sd_b;
    let df_a = (n_a - 1.0).max(1.0);
    let df_b = (n_b - 1.0).max(1.0);
    if va == 0.0 && vb == 0.0 {
        return Ok(VarianceFTestResult { f_statistic: 1.0, df_num: df_a, df_den: df_b, p_value: 1.0 });
    }
    let (f_stat, df_n, df_d) = if va >= vb {
        (if vb > 0.0 { va / vb } else { f64::INFINITY }, df_a, df_b)
    } else {
        (if va > 0.0 { vb / va } else { f64::INFINITY }, df_b, df_a)
    };
    let p_value = if f_stat.is_finite() { (2.0 * f_sf(f_stat, df_n, df_d)?).min(1.0) } else { 0.0 };
    Ok(VarianceFTestResult { f_statistic: f_stat, df_num: df_n, df_den: df_d, p_value })
}

/// Standardized two-group effect sizes.
///
/// - Cohen's `d = diff / s_pooled` (Cohen 1988).
/// - Hedges' `g = J(df) * d` with `df = n_a + n_b - 2` and the exact gamma-function bias correction
///   `J` of Hedges (1981) (large-`df` approximation `1 - 3 / (4 df - 1)`, also Hedges 1981).
/// - Confidence interval for `d` by inverting the noncentral *t* distribution of
///   `d * sqrt(n_a n_b / (n_a + n_b))` on pooled `df = n_a + n_b - 2` (Steiger & Fouladi 1997;
///   Cumming & Finch 2001; Steiger 2004). Because `d` standardizes by `s_pooled`, the pivot
///   follows noncentral `t(df = n_a + n_b - 2)` even when paired with Welch's `t` test. The `g`
///   interval scales the `d` limits by `J(n_a + n_b - 2)`.
/// - Glass's `Delta = diff / s_a` (Glass 1976), with a noncentral-*t* interval on `n_a - 1` df
///   of the standardizing group A (Hedges 1981); returns `None` when `s_a == 0` without substituting `s_b`.
/// - `r^2 = t^2 / (t^2 + df)` using the test's own `t` statistic and `df` (Rosenthal 1991).
fn compute_unpaired_effect_sizes_f(
    diff: f64,
    sds: (f64, f64, f64),
    n_pair: (f64, f64),
    t_df_conf: (f64, f64, ConfidenceLevel),
) -> Result<TwoGroupEffectSizes, StatsError> {
    let (sd_a, _sd_b, sd_pooled) = sds;
    let (na, nb) = n_pair;
    let (t_stat, df_test, confidence) = t_df_conf;
    let n_eff = ((na * nb) / (na + nb)).sqrt();
    let d = if sd_pooled > 0.0 { diff / sd_pooled } else { 0.0 };
    let df_pool = (na + nb - 2.0).max(1.0);
    let j = if df_pool > 1.0 { hedges_j_factor(df_pool)? } else { 1.0 };
    let g = j * d;

    let (d_ci, g_ci) = if sd_pooled > 0.0 && n_eff > 0.0 {
        let t_for_d = d * n_eff;
        if let Ok((lo, hi)) = noncentral_t_ci(t_for_d, df_pool, confidence.percent()) {
            let dl = lo / n_eff;
            let dh = hi / n_eff;
            (Some((dl, dh)), Some((j * dl, j * dh)))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let (glass_delta, glass_delta_ci) = if sd_a > 0.0 {
        let gd = diff / sd_a;
        let t_glass = gd * n_eff;
        let g_ci = noncentral_t_ci(t_glass, (na - 1.0).max(1.0), confidence.percent()).ok().map(|(lo, hi)| (lo / n_eff, hi / n_eff));
        (Some(gd), g_ci)
    } else {
        (None, None)
    };

    let r_sq = if df_test > 0.0 { (t_stat * t_stat) / (t_stat * t_stat + df_test) } else { 0.0 };
    Ok(TwoGroupEffectSizes { cohens_d: d, cohens_d_ci: d_ci, hedges_g: g, hedges_g_ci: g_ci, glass_delta, glass_delta_ci, r_squared: r_sq })
}

fn compute_unpaired_effect_sizes(
    diff: f64,
    sds: (f64, f64, f64),
    n_pair: (usize, usize),
    t_df_conf: (f64, f64, ConfidenceLevel),
) -> Result<TwoGroupEffectSizes, StatsError> {
    compute_unpaired_effect_sizes_f(diff, sds, (n_pair.0 as f64, n_pair.1 as f64), t_df_conf)
}

/// Student pooled-variance *t* (Student 1908; Zar 2010, ch. 8) or Welch *t* with Welch–Satterthwaite
/// `df` (Welch 1947; Satterthwaite 1946) from `(mean, sd, n)` summaries.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn build_unpaired_t_from_moments(
    welch: bool,
    a: (f64, f64, f64),
    b: (f64, f64, f64),
    two_tailed: bool,
    confidence: ConfidenceLevel,
    levene_median: Option<LeveneTestResult>,
) -> Result<UnpairedTResult, StatsError> {
    let (mean_a, sd_a, na) = a;
    let (mean_b, sd_b, nb) = b;
    if na < 2.0 {
        return Err(StatsError::InsufficientSampleSize {
            context: if welch { "Welch t-test group A" } else { "unpaired t-test group A" },
            required: 2,
            actual: na.max(0.0) as usize,
        });
    }
    if nb < 2.0 {
        return Err(StatsError::InsufficientSampleSize {
            context: if welch { "Welch t-test group B" } else { "unpaired t-test group B" },
            required: 2,
            actual: nb.max(0.0) as usize,
        });
    }
    if !mean_a.is_finite() || !sd_a.is_finite() || sd_a < 0.0 || !na.is_finite() {
        return Err(StatsError::InvalidParameter { name: "group_a_summary", value: sd_a });
    }
    if !mean_b.is_finite() || !sd_b.is_finite() || sd_b < 0.0 || !nb.is_finite() {
        return Err(StatsError::InvalidParameter { name: "group_b_summary", value: sd_b });
    }

    let va = sd_a * sd_a;
    let vb = sd_b * sd_b;
    let css_a = (na - 1.0) * va;
    let css_b = (nb - 1.0) * vb;
    let df_pool = na + nb - 2.0;
    let sp2 = (css_a + css_b) / df_pool;
    let sp = sp2.sqrt();

    let (se, df) = if welch {
        let sem2_a = va / na;
        let sem2_b = vb / nb;
        let se2 = sem2_a + sem2_b;
        if se2 == 0.0 {
            return Err(StatsError::ZeroVariance { context: "Welch t-test" });
        }
        let denom = (sem2_a * sem2_a) / (na - 1.0) + (sem2_b * sem2_b) / (nb - 1.0);
        (se2.sqrt(), (se2 * se2) / denom)
    } else {
        if sp2 == 0.0 {
            return Err(StatsError::ZeroVariance { context: "unpaired t-test" });
        }
        (sp * (1.0 / na + 1.0 / nb).sqrt(), df_pool)
    };

    let diff = mean_a - mean_b;
    let t_stat = diff / se;
    let p_value = t_tail_p(t_stat, df, two_tailed)?;
    let t_crit = t_crit_two_sided(confidence, df)?;
    let ci = (diff - t_crit * se, diff + t_crit * se);
    let f_test = f_test_from_summary(sd_a, na, sd_b, nb)?;
    let effect_sizes = compute_unpaired_effect_sizes_f(diff, (sd_a, sd_b, sp), (na, nb), (t_stat, df, confidence))?;

    Ok(UnpairedTResult {
        method_label: if welch { "Welch's t test" } else { "Unpaired Student t test" },
        n_a: na.round() as usize,
        n_b: nb.round() as usize,
        mean_a,
        mean_b,
        sd_a,
        sd_b,
        difference: diff,
        se_difference: se,
        ci,
        t_statistic: t_stat,
        df,
        p_value,
        two_tailed,
        f_test,
        levene_median,
        effect_sizes,
        geometric_mean_ratio: None,
        geometric_mean_ratio_ci: None,
    })
}

/// Unpaired Student *t*-test reconstructed directly from pre-summarized `(mean, sd, n)` moments.
///
/// Pooled-variance *t* (Student 1908; Zar 2010, ch. 8) with the exact two-sample variance-ratio
/// *F* test from summary SDs and sample sizes. Levene/Brown–Forsythe requires raw replicates, so
/// `levene_median` is `None`.
pub fn unpaired_t_test_from_summary(
    group_a: (f64, f64, f64),
    group_b: (f64, f64, f64),
    two_tailed: bool,
    confidence: ConfidenceLevel,
) -> Result<UnpairedTResult, StatsError> {
    build_unpaired_t_from_moments(false, group_a, group_b, two_tailed, confidence, None)
}

/// Unpaired Welch *t*-test reconstructed directly from pre-summarized `(mean, sd, n)` moments.
///
/// Welch (1947) statistic with Welch–Satterthwaite `df` (Satterthwaite 1946) and the exact
/// two-sample variance-ratio *F* test from summary SDs and sample sizes. Levene/Brown–Forsythe
/// requires raw replicates, so `levene_median` is `None`.
pub fn welch_t_test_from_summary(
    group_a: (f64, f64, f64),
    group_b: (f64, f64, f64),
    two_tailed: bool,
    confidence: ConfidenceLevel,
) -> Result<UnpairedTResult, StatsError> {
    build_unpaired_t_from_moments(true, group_a, group_b, two_tailed, confidence, None)
}

/// One-sample Student *t*-test reconstructed directly from pre-summarized `(mean, sd, n)` moments.
///
/// One-sample *t* on `n - 1` df (Student 1908; Zar 2010, ch. 7). Cohen's `d = (mean - mu0) / sd`
/// (Cohen 1988), Hedges' `g = J(n - 1) * d` (Hedges 1981), and a noncentral-*t* interval for the
/// standardized effect (Steiger & Fouladi 1997; Cumming & Finch 2001) whose limits are scaled by `J`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn one_sample_t_test_from_summary(
    mean: f64,
    sd: f64,
    n: f64,
    hypothetical_mean: f64,
    two_tailed: bool,
    confidence: ConfidenceLevel,
) -> Result<OneSampleTResult, StatsError> {
    if !hypothetical_mean.is_finite() {
        return Err(StatsError::InvalidParameter { name: "hypothetical_mean", value: hypothetical_mean });
    }
    if !mean.is_finite() || !sd.is_finite() || sd < 0.0 || !n.is_finite() {
        return Err(StatsError::InvalidParameter { name: "one_sample_summary", value: sd });
    }
    if n < 2.0 {
        return Err(StatsError::InsufficientSampleSize { context: "one-sample t test", required: 2, actual: n.max(0.0) as usize });
    }
    if sd == 0.0 {
        return Err(StatsError::ZeroVariance { context: "one-sample t test" });
    }
    let sem = sd / n.sqrt();
    let discrepancy = mean - hypothetical_mean;
    let t_stat = discrepancy / sem;
    let df = n - 1.0;
    let p_value = t_tail_p(t_stat, df, two_tailed)?;
    let t_crit = t_crit_two_sided(confidence, df)?;
    let ci = (discrepancy - t_crit * sem, discrepancy + t_crit * sem);
    let d = discrepancy / sd;
    let inv_sqrt_n = 1.0 / n.sqrt();
    let (g, g_ci) = hedges_g_with_ci(d, t_stat, df, inv_sqrt_n, confidence.percent())?;

    Ok(OneSampleTResult {
        n: n.round() as usize,
        hypothetical_mean,
        sample_mean: mean,
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

/// Unpaired two-sample Student *t*-test (`a - b` or `b - a`; here `mean_a - mean_b`).
///
/// Pooled-variance *t* on `n_a + n_b - 2` df (Student 1908; Zar 2010, ch. 8), reported together with
/// the variance-ratio *F* test, the Brown & Forsythe (1974) test, Cohen's `d` (Cohen 1988), Hedges'
/// `g` (Hedges 1981), Glass's `Delta` (Glass 1976), noncentral-*t* intervals for them (Steiger &
/// Fouladi 1997; Cumming & Finch 2001), and `r^2 = t^2 / (t^2 + df)` (Rosenthal 1991).
pub fn unpaired_t_test(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<UnpairedTResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "unpaired t-test group A", required: 2, actual: a.len() });
    }
    if b.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "unpaired t-test group B", required: 2, actual: b.len() });
    }
    let ma = SummaryMoments::from_slice(a)?;
    let mb = SummaryMoments::from_slice(b)?;
    let sd_a = ma.sample_sd()?;
    let sd_b = mb.sample_sd()?;
    let df = (a.len() + b.len() - 2) as f64;
    let sp2 = (ma.css + mb.css) / df;
    if sp2 == 0.0 {
        return Err(StatsError::ZeroVariance { context: "unpaired t-test" });
    }
    let sp = sp2.sqrt();
    let se = sp * (1.0 / (a.len() as f64) + 1.0 / (b.len() as f64)).sqrt();
    let diff = ma.mean - mb.mean;
    let t_stat = diff / se;
    let p_value = t_tail_p(t_stat, df, two_tailed)?;
    let t_crit = t_crit_two_sided(confidence, df)?;
    let ci = (diff - t_crit * se, diff + t_crit * se);
    let f_test = f_test_variances(a, b)?;
    let levene_median = Some(levene_variance_test(&[a, b], true)?);
    let effect_sizes = compute_unpaired_effect_sizes(diff, (sd_a, sd_b, sp), (a.len(), b.len()), (t_stat, df, confidence))?;

    Ok(UnpairedTResult {
        method_label: "Unpaired Student t test",
        n_a: a.len(),
        n_b: b.len(),
        mean_a: ma.mean,
        mean_b: mb.mean,
        sd_a,
        sd_b,
        difference: diff,
        se_difference: se,
        ci,
        t_statistic: t_stat,
        df,
        p_value,
        two_tailed,
        f_test,
        levene_median,
        effect_sizes,
        geometric_mean_ratio: None,
        geometric_mean_ratio_ci: None,
    })
}

/// Unpaired Welch *t*-test (unequal variances, Welch–Satterthwaite fractional df).
///
/// `t = diff / sqrt(s_a^2/n_a + s_b^2/n_b)` (Welch 1947) with
/// `df = (s_a^2/n_a + s_b^2/n_b)^2 / [(s_a^2/n_a)^2/(n_a-1) + (s_b^2/n_b)^2/(n_b-1)]` (Satterthwaite 1946).
/// Standardized effect sizes `d` and `g` (Cohen 1988; Hedges 1981) standardize by the pooled SD, so
/// their noncentral-*t* confidence intervals use pooled `df = n_a + n_b - 2` (Steiger 2004), while
/// `r^2 = t^2 / (t^2 + df)` uses the Welch `t` and Welch–Satterthwaite `df`.
pub fn welch_t_test(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<UnpairedTResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Welch t-test group A", required: 2, actual: a.len() });
    }
    if b.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Welch t-test group B", required: 2, actual: b.len() });
    }
    let ma = SummaryMoments::from_slice(a)?;
    let mb = SummaryMoments::from_slice(b)?;
    let va = ma.sample_variance()?;
    let vb = mb.sample_variance()?;
    let sd_a = va.sqrt();
    let sd_b = vb.sqrt();
    let na = a.len() as f64;
    let nb = b.len() as f64;
    let sem2_a = va / na;
    let sem2_b = vb / nb;
    let se2 = sem2_a + sem2_b;
    if se2 == 0.0 {
        return Err(StatsError::ZeroVariance { context: "Welch t-test" });
    }
    let se = se2.sqrt();
    let denom = (sem2_a * sem2_a) / (na - 1.0) + (sem2_b * sem2_b) / (nb - 1.0);
    let df = (se2 * se2) / denom;
    let diff = ma.mean - mb.mean;
    let t_stat = diff / se;
    let p_value = t_tail_p(t_stat, df, two_tailed)?;
    let t_crit = t_crit_two_sided(confidence, df)?;
    let ci = (diff - t_crit * se, diff + t_crit * se);
    let f_test = f_test_variances(a, b)?;
    let levene_median = Some(levene_variance_test(&[a, b], true)?);
    let sd_pooled = ((ma.css + mb.css) / (na + nb - 2.0)).sqrt();
    let effect_sizes = compute_unpaired_effect_sizes(diff, (sd_a, sd_b, sd_pooled), (a.len(), b.len()), (t_stat, df, confidence))?;

    Ok(UnpairedTResult {
        method_label: "Welch's t test",
        n_a: a.len(),
        n_b: b.len(),
        mean_a: ma.mean,
        mean_b: mb.mean,
        sd_a,
        sd_b,
        difference: diff,
        se_difference: se,
        ci,
        t_statistic: t_stat,
        df,
        p_value,
        two_tailed,
        f_test,
        levene_median,
        effect_sizes,
        geometric_mean_ratio: None,
        geometric_mean_ratio_ci: None,
    })
}

/// Unpaired lognormal *t*-test (Student *t*-test performed on `ln(x)` for strictly positive observations).
///
/// The back-transformed difference and CI are the ratio of geometric means and its interval
/// (Bland & Altman 1996).
pub fn lognormal_unpaired_t_test(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<UnpairedTResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.iter().any(|&v| v <= 0.0) || b.iter().any(|&v| v <= 0.0) {
        return Err(StatsError::InvalidParameter { name: "lognormal_x", value: 0.0 });
    }
    let la: Vec<f64> = a.iter().map(|v| v.ln()).collect();
    let lb: Vec<f64> = b.iter().map(|v| v.ln()).collect();
    let mut out = unpaired_t_test(&la, &lb, two_tailed, confidence)?;
    out.method_label = "Lognormal unpaired t test";
    out.geometric_mean_ratio = Some(out.difference.exp());
    out.geometric_mean_ratio_ci = Some((out.ci.0.exp(), out.ci.1.exp()));
    Ok(out)
}

/// Complete output of a paired or ratio-paired Student *t*-test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedTResult {
    /// Descriptive method label (`"Paired t test"` or `"Ratio paired t test"`).
    pub method_label: &'static str,
    /// Number of complete paired observations `(a_i, b_i)` (`n_pairs >= 2`).
    pub n_pairs: usize,
    /// Arithmetic mean of paired differences `mean(a - b)` (or `mean(ln(a / b))` for ratio paired *t*).
    pub mean_of_differences: f64,
    /// Sample standard deviation of paired differences (`n_pairs - 1` denominator).
    pub sd_of_differences: f64,
    /// Standard error of the mean difference `sd_of_differences / sqrt(n_pairs)`.
    pub sem_of_differences: f64,
    /// Two-sided `t(df)` confidence interval `(lower, upper)` for `mean_of_differences` at configured `ConfidenceLevel`.
    pub ci: (f64, f64),
    /// Paired Student *t* statistic `mean_of_differences / sem_of_differences`.
    pub t_statistic: f64,
    /// Degrees of freedom `n_pairs - 1` (`>= 1.0`).
    pub df: f64,
    /// Student *t*(df) *P* value (`two_tailed` two-sided vs directional one-sided matching `t_statistic` sign).
    pub p_value: f64,
    /// True when `p_value` is two-sided; false when directional one-tailed.
    pub two_tailed: bool,
    /// Pearson correlation `r` assessing pairing effectiveness (`None` when `n_pairs < 3` or zero variance in either column).
    pub pairing_r: Option<f64>,
    /// Two-sided *P* value testing $H_0: \rho_{\text{pairing}} = 0$ on `n_pairs - 2` df (`None` when `n_pairs < 3` or `pairing_r` unavailable).
    pub pairing_p: Option<f64>,
    /// Standardized paired effect size `d_z = mean_of_differences / sd_of_differences` (Cohen 1988).
    pub cohens_dz: f64,
    /// Bias-corrected Hedges' `g_z = J(df) * d_z` (Hedges 1981); `None` when `df <= 1.0` (`n_pairs = 2`, gamma pole).
    pub hedges_gz: Option<f64>,
    /// Exact noncentral-*t* two-sided CI for `hedges_gz` (Steiger & Fouladi 1997); `None` when `hedges_gz` is `None` or inversion fails.
    pub hedges_gz_ci: Option<(f64, f64)>,
    /// Present for `ratio_paired_t_test`: geometric mean of ratios `exp(mean(ln(a / b)))`; `None` for additive `paired_t_test`.
    pub geometric_mean_ratio: Option<f64>,
    /// Present for `ratio_paired_t_test`: two-sided exponentiated `t(df)` CI `exp(ci)` for `geometric_mean_ratio`; `None` otherwise.
    pub geometric_mean_ratio_ci: Option<(f64, f64)>,
}

/// Paired *t*-test on matched pairs `(a_i, b_i)` analyzing `a_i - b_i`.
///
/// One-sample *t* on the differences with `n - 1` df (Student 1908; Zar 2010, ch. 9). Effect size
/// `d_z = mean(d) / sd(d)` (Cohen 1988), `g_z = J(n - 1) * d_z` (Hedges 1981), with a noncentral-*t*
/// interval (Steiger & Fouladi 1997) whose limits are scaled by `J`. The pairing check reports
/// Pearson `r` with a **two-sided** *p*-value for `H0: rho = 0` from `t = r sqrt((n-2)/(1-r^2))` on
/// `n - 2` df (returning `0` continuously at both `r = -1` and `r = +1` when `n >= 3`).
pub fn paired_t_test(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<PairedTResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.len() != b.len() {
        return Err(StatsError::DimensionMismatch { expected: a.len(), actual: b.len() });
    }
    let n = a.len();
    if n < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "paired t-test", required: 2, actual: n });
    }
    let diffs: Vec<f64> = a.iter().zip(b.iter()).map(|(&x, &y)| x - y).collect();
    let md = SummaryMoments::from_slice(&diffs)?;
    let sd = md.sample_sd()?;
    if sd == 0.0 {
        return Err(StatsError::ZeroVariance { context: "paired t-test differences" });
    }
    let sem = md.sem()?;
    let df = (n - 1) as f64;
    let t_stat = md.mean / sem;
    let p_value = t_tail_p(t_stat, df, two_tailed)?;
    let t_crit = t_crit_two_sided(confidence, df)?;
    let ci = (md.mean - t_crit * sem, md.mean + t_crit * sem);

    let (pairing_r, pairing_p) = if n >= 3 {
        if let Ok(r) = pearson_r(a, b) {
            let r_clamped = r.clamp(-1.0, 1.0);
            let one_minus_r2 = 1.0 - r_clamped * r_clamped;
            let p_r = if one_minus_r2 > 0.0 {
                let tr = r_clamped * (((n - 2) as f64) / one_minus_r2).sqrt();
                student_t_two_sided_p(tr, (n - 2) as f64).ok()
            } else {
                Some(0.0)
            };
            (Some(r_clamped), p_r)
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let dz = md.mean / sd;
    let inv_sqrt_n = 1.0 / (n as f64).sqrt();
    let (gz, gz_ci) = hedges_g_with_ci(dz, t_stat, df, inv_sqrt_n, confidence.percent())?;

    Ok(PairedTResult {
        method_label: "Paired t test",
        n_pairs: n,
        mean_of_differences: md.mean,
        sd_of_differences: sd,
        sem_of_differences: sem,
        ci,
        t_statistic: t_stat,
        df,
        p_value,
        two_tailed,
        pairing_r,
        pairing_p,
        cohens_dz: dz,
        hedges_gz: gz,
        hedges_gz_ci: gz_ci,
        geometric_mean_ratio: None,
        geometric_mean_ratio_ci: None,
    })
}

/// Ratio paired *t*-test on strictly positive pairs `(a_i, b_i)` analyzing `ln(a_i / b_i)`.
///
/// Paired *t* on log-transformed values; the back-transformed mean difference and CI are the
/// geometric mean ratio and its interval (Bland & Altman 1996).
pub fn ratio_paired_t_test(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<PairedTResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.len() != b.len() {
        return Err(StatsError::DimensionMismatch { expected: a.len(), actual: b.len() });
    }
    if a.iter().any(|&v| v <= 0.0) || b.iter().any(|&v| v <= 0.0) {
        return Err(StatsError::InvalidParameter { name: "ratio_paired_x", value: 0.0 });
    }
    let la: Vec<f64> = a.iter().map(|v| v.ln()).collect();
    let lb: Vec<f64> = b.iter().map(|v| v.ln()).collect();
    let mut out = paired_t_test(&la, &lb, two_tailed, confidence)?;
    out.method_label = "Ratio paired t test";
    out.geometric_mean_ratio = Some(out.mean_of_differences.exp());
    out.geometric_mean_ratio_ci = Some((out.ci.0.exp(), out.ci.1.exp()));
    Ok(out)
}

// ============================================================================
// Nonparametric Two-Group Tests
// ============================================================================

/// Result of a Mann–Whitney *U* test (Mann & Whitney 1947; Wilcoxon 1945).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MannWhitneyResult {
    /// Number of observations in group A.
    pub n_a: usize,
    /// Number of observations in group B.
    pub n_b: usize,
    /// Sample median of group A (Hyndman & Fan Type 6).
    pub median_a: f64,
    /// Sample median of group B (Hyndman & Fan Type 6).
    pub median_b: f64,
    /// Mann–Whitney *U* count of `(a > b)` pairs (with 0.5 for exact ties) for group A.
    pub u_a: f64,
    /// Complementary Mann–Whitney *U* count `n_a * n_b - u_a` for group B.
    pub u_b: f64,
    /// Smaller Mann–Whitney statistic `min(u_a, u_b)`.
    pub u_min: f64,
    /// *P* value testing identical distributions (`two_tailed` doubles smaller tail vs directional observed-tail one-sided via exact DP when `n_a + n_b <= 40` and tie-free, else tie-/continuity-corrected normal).
    pub p_value: f64,
    /// True if exact Mann–Whitney combinatorial DP distribution was used; false for normal approximation.
    pub exact_p_used: bool,
    /// True for two-sided *P* value; false for directional one-sided test.
    pub two_tailed: bool,
    /// Hodges–Lehmann (1963) shift location estimator `median(a_i - b_j)` across all pairwise differences.
    pub hodges_lehmann_diff: f64,
    /// Two-sided distribution-free order-statistic CI `(lower, upper)` on pairwise differences `a_i - b_j` at configured `ConfidenceLevel` (`None` if normal/exact rank boundary unavailable).
    pub hodges_lehmann_ci: Option<(f64, f64)>,
    /// Exact achieved confidence percentage when small-sample (`na <= 30 && nb <= 30`, untied) exact Mann–Whitney critical values were used (`None` for asymptotic normal or unavailable CIs).
    #[serde(default)]
    pub ci_actual_confidence_percent: Option<f64>,
    /// Cliff's dominance effect size `delta = (u_a - u_b) / (n_a * n_b)` in `[-1, 1]` (Cliff 1993).
    pub cliffs_delta: f64,
    /// Rank-biserial correlation `r = 1 - 2 * u_min / (n_a * n_b)` / Kerby (2014) signed rank effect size in `[-1, 1]`.
    pub rank_biserial_r: f64,
}

/// Exact DP distribution of Mann–Whitney `U` for `n1 + n2 <= 40` without ties.
///
/// Counts arrangements by the Mann & Whitney (1947) recurrence on `U`; the two-sided *p*-value doubles
/// the smaller tail, and the one-sided *p*-value returns the smaller (observed-direction) tail
/// `min(P(U <= u_obs), P(U >= u_obs))`, matching the sign, Wilcoxon, and *t*-test convention.
fn mann_whitney_exact_p(n1: usize, n2: usize, u_obs: usize, two_tailed: bool) -> f64 {
    let max_u = n1 * n2;
    // dp[i][u] = number of subsets of size i from i+j elements with U = u, iteratively over N = n1+n2.
    let mut dp = vec![vec![0.0_f64; max_u + 1]; n1 + 1];
    dp[0][0] = 1.0;
    for total in 1..=(n1 + n2) {
        let i_max = total.min(n1);
        for i in (1..=i_max).rev() {
            let j = total - i;
            if j > n2 {
                continue;
            }
            for u in j..=max_u {
                dp[i][u] += dp[i - 1][u - j];
            }
        }
    }
    let total_ways: f64 = dp[n1].iter().sum();
    if total_ways <= 0.0 {
        return 1.0;
    }
    let lower_tail: f64 = dp[n1].iter().take(u_obs + 1).sum();
    let upper_tail: f64 = dp[n1].iter().skip(u_obs).sum();
    let min_tail = lower_tail.min(upper_tail);
    if two_tailed { (2.0 * min_tail / total_ways).clamp(0.0, 1.0) } else { (min_tail / total_ways).clamp(0.0, 1.0) }
}

/// Exact conditional permutation DP for Mann–Whitney with tied ranks (`n_a + n_b <= 18`).
/// Computes exact permutation counts over doubled integer ranks `2 * r_i`.
fn mann_whitney_exact_ties_p(ranks: &[f64], na: usize, two_tailed: bool) -> f64 {
    let n = ranks.len();
    let two_ranks: Vec<usize> = ranks.iter().map(|&r| (2.0 * r).round() as usize).collect();
    let max_sum: usize = two_ranks.iter().sum();
    let mut dp = vec![vec![0_u64; max_sum + 1]; na + 1];
    dp[0][0] = 1;
    let mut running_max = 0_usize;
    for (idx, &tr) in two_ranks.iter().enumerate() {
        let i_max = na.min(idx + 1);
        running_max += tr;
        for i in (1..=i_max).rev() {
            for s in tr..=running_max {
                dp[i][s] += dp[i - 1][s - tr];
            }
        }
    }
    let total_ways: u64 = dp[na].iter().sum();
    if total_ways == 0 {
        return 1.0;
    }
    let two_r1_obs: usize = two_ranks.iter().take(na).sum();
    if two_tailed {
        let expected_two_r1 = na * (n + 1);
        let obs_dev = two_r1_obs.abs_diff(expected_two_r1);
        let tail_ways: u64 = dp[na].iter().enumerate().filter(|&(s, _)| s.abs_diff(expected_two_r1) >= obs_dev).map(|(_, &c)| c).sum();
        ((tail_ways as f64) / (total_ways as f64)).clamp(0.0, 1.0)
    } else {
        let lower_tail: u64 = dp[na].iter().take(two_r1_obs + 1).sum();
        let upper_tail: u64 = dp[na].iter().skip(two_r1_obs).sum();
        let min_tail = lower_tail.min(upper_tail);
        ((min_tail as f64) / (total_ways as f64)).clamp(0.0, 1.0)
    }
}

/// Mann–Whitney *U* test comparing `a` and `b`, with exact DP *p*-values for untied samples
/// (`n_a + n_b <= 40`) and exact conditional permutation DP for tied samples (`n_a + n_b <= 18`),
/// exact Hodges–Lehmann median difference `a - b` + Moses CI, Cliff's `delta`, and rank-biserial `r`.
///
/// Wilcoxon (1945) rank-sum / Mann & Whitney (1947) `U_a = R_a - n_a (n_a + 1) / 2`. Above the
/// exact DP bounds (`n_a + n_b > 40` untied, or `> 18` with ties), the *p*-value uses the normal
/// approximation with the tie-corrected variance `(n_a n_b / 12) [N + 1 - sum(t^3 - t) / (N (N - 1))]`
/// and a 0.5 continuity correction (Hollander, Wolfe & Chicken 2014, ch. 4).
/// Cliff's `delta = (U_a - U_b) / (n_a n_b)` (Cliff 1993), equal to the rank-biserial correlation
/// of Wendt (1972) / Kerby (2014).
pub fn mann_whitney_u(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<MannWhitneyResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    let na = a.len();
    let nb = b.len();
    let sa = sorted_finite(a);
    let sb = sorted_finite(b);
    let median_a = quantile_type6_sorted(&sa, 0.5).or_else(|| sa.first().copied()).ok_or(StatsError::EmptyInput)?;
    let median_b = quantile_type6_sorted(&sb, 0.5).or_else(|| sb.first().copied()).ok_or(StatsError::EmptyInput)?;

    let mut combined = Vec::with_capacity(na + nb);
    combined.extend_from_slice(a);
    combined.extend_from_slice(b);
    let ranked = rank_with_ties(&combined)?;
    let r1: f64 = ranked.ranks.iter().take(na).sum();
    let naf = na as f64;
    let nbf = nb as f64;
    let nf = (na + nb) as f64;

    let u_a = r1 - 0.5 * naf * (naf + 1.0);
    let u_b = naf * nbf - u_a;
    let u_min = u_a.min(u_b);

    let (p_value, exact_p_used) = if na + nb <= 40 && ranked.tie_counts.is_empty() {
        let u_int = u_a.round() as usize;
        (mann_whitney_exact_p(na, nb, u_int, two_tailed), true)
    } else if !ranked.tie_counts.is_empty() && na + nb <= 18 {
        (mann_whitney_exact_ties_p(&ranked.ranks, na, two_tailed), true)
    } else {
        let mean_u = 0.5 * naf * nbf;
        let var_u = (naf * nbf / 12.0) * (nf + 1.0 - ranked.tie_sum_t3_minus_t / (nf * (nf - 1.0)));
        if ranked.tie_counts.as_slice() == [na + nb] || var_u <= 0.0 {
            let p = if (u_a - mean_u).abs() <= 1e-12 { 1.0 } else { 0.0 };
            (p, false)
        } else {
            let sd_u = var_u.sqrt();
            let p = if two_tailed {
                let cc = (u_a - mean_u).abs() - 0.5;
                if cc <= 0.0 { 1.0 } else { (2.0 * norm_sf(cc / sd_u)).min(1.0) }
            } else {
                let cc = ((u_a - mean_u).abs() - 0.5).max(0.0);
                norm_sf(cc / sd_u).clamp(0.0, 1.0)
            };
            (p.clamp(0.0, 1.0), false)
        }
    };

    let (hodges_lehmann_diff, hodges_lehmann_ci, ci_actual_confidence_percent) = hodges_lehmann_two_sample(a, b, confidence)?;
    let cliffs_delta = (u_a - u_b) / (naf * nbf);
    let rank_biserial_r = cliffs_delta;

    Ok(MannWhitneyResult {
        n_a: na,
        n_b: nb,
        median_a,
        median_b,
        u_a,
        u_b,
        u_min,
        p_value,
        exact_p_used,
        two_tailed,
        hodges_lehmann_diff,
        hodges_lehmann_ci,
        ci_actual_confidence_percent,
        cliffs_delta,
        rank_biserial_r,
    })
}

/// Exact distribution-free two-sided Mann–Whitney order-statistic index `k` (1-based) and achieved
/// coverage fraction for untied samples of sizes `na, nb <= 30` (Hollander, Wolfe & Chicken 2014, Ch. 4.3).
fn mann_whitney_exact_hl_ci_rank(na: usize, nb: usize, confidence: ConfidenceLevel) -> (usize, f64) {
    let (n1, n2) = if na <= nb { (na, nb) } else { (nb, na) };
    let m = n1 * n2;
    let half_m = m / 2;
    let mut dp = vec![vec![0_u128; half_m + 1]; n1 + 1];
    dp[0][0] = 1;
    for total in 1..=(n1 + n2) {
        let i_max = total.min(n1);
        for i in (1..=i_max).rev() {
            let j = total - i;
            if j > n2 {
                continue;
            }
            for u in j..=half_m {
                dp[i][u] += dp[i - 1][u - j];
            }
        }
    }
    let mut total_ways = 1_u128;
    for k in 1..=n1 {
        total_ways = total_ways * ((n2 + k) as u128) / (k as u128);
    }
    let total_f = total_ways as f64;
    let target = confidence.percent() / 100.0;
    let mut cum = dp[n1][0];
    let mut best_k = 1_usize;
    let mut best_cov = (1.0 - 2.0 * (cum as f64) / total_f).clamp(0.0, 1.0);
    if best_cov + 1e-14 < target {
        return (1, best_cov);
    }
    for k in 2..=half_m {
        cum += dp[n1][k - 1];
        let cov = (1.0 - 2.0 * (cum as f64) / total_f).clamp(0.0, 1.0);
        if cov + 1e-14 >= target {
            best_k = k;
            best_cov = cov;
        } else {
            break;
        }
    }
    (best_k, best_cov)
}

/// Upper bound on achievable two-sided Mann–Whitney CI coverage (%) for sample sizes `na, nb <= 30`.
fn mann_whitney_max_achievable_coverage_percent(na: usize, nb: usize) -> Option<f64> {
    if na >= 1 && nb >= 1 && na <= 30 && nb <= 30 {
        let (n1, n2) = if na <= nb { (na, nb) } else { (nb, na) };
        let mut total_ways = 1_u128;
        for k in 1..=n1 {
            total_ways = total_ways * ((n2 + k) as u128) / (k as u128);
        }
        Some(100.0 * (1.0 - 2.0 / (total_ways as f64)).clamp(0.0, 1.0))
    } else {
        None
    }
}

/// Point estimate, two-sided Hodges–Lehmann CI, and achieved small-sample confidence percentage.
type HodgesLehmannShiftWithCoverage = (f64, Option<(f64, f64)>, Option<f64>);

/// Hodges & Lehmann (1963) two-sample shift estimator (median of all `a_i - b_j`) with the Moses
/// distribution-free interval `[D_(k), D_(mn + 1 - k)]` (Hollander, Wolfe & Chicken 2014, ch. 4).
///
/// Uses exact Mann–Whitney DP critical values for small untied samples (`na <= 30 && nb <= 30`),
/// exact sorted differences when `m * n <= 40_000`, and exact `O((m + n) log(m + n))`
/// monotone-matrix selection (`select_kth_pairwise_sum`, cf. Monahan 1984) for both the shift
/// median and the large-sample normal-approximated Moses CI (`floor(mn/2 - z sqrt(mn(m+n+1)/12))`)
/// when `m * n > 40_000`.
fn hodges_lehmann_two_sample(a: &[f64], b: &[f64], confidence: ConfidenceLevel) -> Result<HodgesLehmannShiftWithCoverage, StatsError> {
    let na = a.len();
    let nb = b.len();
    let m = na.saturating_mul(nb);
    if na == 0 || nb == 0 {
        return Err(StatsError::EmptyInput);
    }
    let (hl, lo_ci_val, hi_ci_val, actual_conf_pct) = if m > 40_000 {
        let sa = sorted_finite(a);
        let mut neg_b: Vec<f64> = b.iter().map(|&v| -v).collect();
        neg_b.sort_by(f64::total_cmp);
        let m_u64 = (na as u64) * (nb as u64);
        let hl = if m_u64 % 2 == 1 {
            select_kth_pairwise_sum(&sa, &neg_b, m_u64 / 2)?
        } else {
            let mid_lo = select_kth_pairwise_sum(&sa, &neg_b, m_u64 / 2 - 1)?;
            let mid_hi = select_kth_pairwise_sum(&sa, &neg_b, m_u64 / 2)?;
            0.5 * mid_lo + 0.5 * mid_hi
        };
        if !hl.is_finite() {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "hodges_lehmann", value: hl }));
        }
        if na + nb < 4 {
            return Ok((hl, None, None));
        }
        let z = norm_two_sided_isf(confidence.alpha())?;
        let naf = na as f64;
        let nbf = nb as f64;
        let mu = 0.5 * naf * nbf;
        let sd = (naf * nbf * (naf + nbf + 1.0) / 12.0).sqrt();
        let k = (mu - z * sd).floor().max(1.0) as u64;
        let lo_idx = k.saturating_sub(1).min(m_u64 - 1);
        let hi_idx = (m_u64 - k).min(m_u64 - 1);
        let ci_lo = select_kth_pairwise_sum(&sa, &neg_b, lo_idx)?;
        let ci_hi = select_kth_pairwise_sum(&sa, &neg_b, hi_idx)?;
        (hl, ci_lo, ci_hi, None)
    } else {
        let mut diffs = Vec::with_capacity(m);
        for &va in a {
            for &vb in b {
                diffs.push(va - vb);
            }
        }
        diffs.sort_by(f64::total_cmp);
        let hl = quantile_type6_sorted(&diffs, 0.5).or_else(|| diffs.first().copied()).ok_or(StatsError::EmptyInput)?;
        if !hl.is_finite() {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "hodges_lehmann", value: hl }));
        }
        if na + nb < 4 {
            return Ok((hl, None, None));
        }
        let sa = sorted_finite(a);
        let sb = sorted_finite(b);
        let untied = sa.windows(2).all(|w| w[0] < w[1]) && sb.windows(2).all(|w| w[0] < w[1]) && !has_cross_ties(&sa, &sb);
        let (k, actual_conf_pct) = if na <= 30 && nb <= 30 && untied {
            let (exact_k, cov) = mann_whitney_exact_hl_ci_rank(na, nb, confidence);
            (exact_k, Some(100.0 * cov))
        } else {
            let z = norm_two_sided_isf(confidence.alpha())?;
            let naf = na as f64;
            let nbf = nb as f64;
            let mu = 0.5 * naf * nbf;
            let sd = (naf * nbf * (naf + nbf + 1.0) / 12.0).sqrt();
            ((mu - z * sd).floor().max(1.0) as usize, None)
        };
        let lo_idx = k.saturating_sub(1).min(m - 1);
        let hi_idx = (m - k).min(m - 1);
        (hl, diffs[lo_idx], diffs[hi_idx], actual_conf_pct)
    };
    let ci = if lo_ci_val.is_finite() && hi_ci_val.is_finite() { Some((lo_ci_val, hi_ci_val)) } else { None };
    Ok((hl, ci, actual_conf_pct))
}

/// Output of Wilcoxon matched-pairs signed-rank test (Wilcoxon 1945; Pratt 1959).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WilcoxonMatchedResult {
    /// Total number of paired observations supplied before any zero-difference handling.
    pub n_pairs: usize,
    /// Effective non-zero pair count contributing to signed ranks (equal to `n_pairs` minus exact zero differences in classic mode, or total `n_pairs` in Pratt mode).
    pub n_effective: usize,
    /// Sum of signed ranks for positive differences `a_i > b_i` (`W+`).
    pub w_plus: f64,
    /// Sum of signed ranks for negative differences `a_i < b_i` (`W-`).
    pub w_minus: f64,
    /// Wilcoxon test statistic (`W+ - W-` or `min(W+, W-)` depending on mode/tail reporting).
    pub w_stat: f64,
    /// *P* value testing symmetric median difference around zero (`two_tailed` vs observed-direction one-tailed; exact DP when `(!use_pratt_zeros || n_zeros == 0)` with `n_effective <= 25` and no ties, else tie-/continuity-corrected normal).
    pub p_value: f64,
    /// True if exact DP distribution was used; false for asymptotic normal approximation.
    pub exact_p_used: bool,
    /// True for two-sided *P* value; false for directional one-sided test.
    pub two_tailed: bool,
    /// Hodges–Lehmann (1963) Walsh-average pseudomedian computed over all paired differences (including exact zeros).
    pub hodges_lehmann_pseudomedian: f64,
    /// Two-sided order-statistic CI `(lower, upper)` over pairwise Walsh averages `(d_i + d_j)/2` at configured `ConfidenceLevel` (`None` if insufficient pairs).
    pub hodges_lehmann_ci: Option<(f64, f64)>,
    /// Exact achieved confidence percentage when small-sample (`n_pairs <= 30`, untied) exact signed-rank critical values were used (`None` for asymptotic normal or unavailable CIs).
    #[serde(default)]
    pub ci_actual_confidence_percent: Option<f64>,
    /// Kerby (2014) matched-pairs rank-biserial correlation `(W+ - W-) / (W+ + W-)` in `[-1, 1]`.
    pub rank_biserial_r: f64,
}

/// Wilcoxon matched-pairs signed-rank test (`use_pratt_zeros = false` uses classic zero exclusion;
/// `true` uses Pratt's zero-inclusion tie adjustment).
///
/// Signed-rank statistic `W+` (Wilcoxon 1945). Classic mode drops zero differences and uses the exact
/// null distribution for `n <= 25` without ties, otherwise the normal approximation with tie
/// correction and 0.5 continuity correction (Hollander, Wolfe & Chicken 2014, ch. 3). Pratt mode
/// (Pratt 1959) ranks zeros with the nonzero `|d|` and discards their ranks: when `n_zeros == 0`,
/// it is identical to classic mode (and uses the exact DP for `n <= 25` without ties); when
/// `n_zeros > 0`, it uses the Pratt normal approximation with mean `[n(n+1) - z(z+1)] / 4` and variance
/// `[n(n+1)(2n+1) - z(z+1)(2z+1) - sum_{nonzero}(t^3 - t)/2] / 24`, where the tie correction
/// sum includes only **nonzero** tied groups so zeros are not double-counted. The pseudomedian is
/// the Hodges & Lehmann (1963) Walsh-average estimator computed on **all** differences (zeros
/// included, in both modes); the matched-pairs rank-biserial `r = (W+ - W-) / (W+ + W-)` follows
/// Kerby (2014).
pub fn wilcoxon_matched_pairs(
    a: &[f64],
    b: &[f64],
    two_tailed: bool,
    confidence: ConfidenceLevel,
    use_pratt_zeros: bool,
) -> Result<WilcoxonMatchedResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.len() != b.len() {
        return Err(StatsError::DimensionMismatch { expected: a.len(), actual: b.len() });
    }
    let diffs_all: Vec<f64> = a.iter().zip(b.iter()).map(|(&x, &y)| x - y).collect();
    let diffs_work: Vec<f64> = if use_pratt_zeros { diffs_all.clone() } else { diffs_all.iter().copied().filter(|&d| d != 0.0).collect() };
    let n_eff = diffs_work.len();
    if n_eff == 0 {
        return Ok(WilcoxonMatchedResult {
            n_pairs: a.len(),
            n_effective: 0,
            w_plus: 0.0,
            w_minus: 0.0,
            w_stat: 0.0,
            p_value: 1.0,
            exact_p_used: true,
            two_tailed,
            hodges_lehmann_pseudomedian: 0.0,
            hodges_lehmann_ci: None,
            ci_actual_confidence_percent: None,
            rank_biserial_r: 0.0,
        });
    }

    let abs_diffs: Vec<f64> = diffs_work.iter().map(|d| d.abs()).collect();
    let ranked = rank_with_ties(&abs_diffs)?;
    let mut w_plus = 0.0_f64;
    let mut w_minus = 0.0_f64;
    let mut n_zeros = 0_usize;
    for (&d, &r) in diffs_work.iter().zip(ranked.ranks.iter()) {
        if d > 0.0 {
            w_plus += r;
        } else if d < 0.0 {
            w_minus += r;
        } else {
            n_zeros += 1;
        }
    }

    let (p_value, exact_p_used) = if (!use_pratt_zeros || n_zeros == 0) && n_eff <= 25 && ranked.tie_counts.is_empty() {
        let max_sum = n_eff * (n_eff + 1) / 2;
        (wilcoxon_exact_p(n_eff, w_plus.round() as usize, max_sum, two_tailed), true)
    } else {
        let nf = n_eff as f64;
        let zf = n_zeros as f64;
        let mean_w = 0.25 * (nf * (nf + 1.0) - zf * (zf + 1.0));
        // Subtract any zero-difference tie group (t_0^3 - t_0) so zeros are not double-counted
        // alongside the Pratt -d_0(d_0+1)(2d_0+1) term.
        let zero_tie_corr = if n_zeros > 1 { zf * (zf * zf - 1.0) } else { 0.0 };
        let nonzero_tie_sum = (ranked.tie_sum_t3_minus_t - zero_tie_corr).max(0.0);
        let var_w = ((nf * (nf + 1.0) * (2.0 * nf + 1.0) - zf * (zf + 1.0) * (2.0 * zf + 1.0) - 0.5 * nonzero_tie_sum) / 24.0).max(0.0);
        if n_zeros == n_eff || var_w <= 0.0 {
            let p = if (w_plus - mean_w).abs() <= 1e-12 { 1.0 } else { 0.0 };
            (p, false)
        } else {
            let sd_w = var_w.sqrt();
            let p = if two_tailed {
                let cc = (w_plus - mean_w).abs() - 0.5;
                if cc <= 0.0 { 1.0 } else { (2.0 * norm_sf(cc / sd_w)).min(1.0) }
            } else {
                let cc = ((w_plus - mean_w).abs() - 0.5).max(0.0);
                norm_sf(cc / sd_w).clamp(0.0, 1.0)
            };
            (p.clamp(0.0, 1.0), false)
        }
    };

    let (hl, hl_ci, ci_actual_confidence_percent) = hodges_lehmann_one_sample_with_coverage(&diffs_all, confidence)?;
    let total_rank_sum = w_plus + w_minus;
    let rank_biserial_r = if total_rank_sum > 0.0 { (w_plus - w_minus) / total_rank_sum } else { 0.0 };

    Ok(WilcoxonMatchedResult {
        n_pairs: a.len(),
        n_effective: n_eff,
        w_plus,
        w_minus,
        w_stat: w_plus - w_minus,
        p_value,
        exact_p_used,
        two_tailed,
        hodges_lehmann_pseudomedian: hl,
        hodges_lehmann_ci: hl_ci,
        ci_actual_confidence_percent,
        rank_biserial_r,
    })
}

/// Output of an exact binomial paired sign test on matched differences `a_i - b_i`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedSignTestResult {
    /// Total number of paired observations supplied.
    pub n_total: usize,
    /// Number of strictly positive differences (`a_i - b_i > 0`).
    pub n_positive: usize,
    /// Number of strictly negative differences (`a_i - b_i < 0`).
    pub n_negative: usize,
    /// Number of exact zero ties (`a_i == b_i`) excluded from the binomial sign count.
    pub n_ties: usize,
    /// Total non-zero differences `n_positive + n_negative`.
    pub n_nonzero: usize,
    /// Sample median across all `n_total` differences `a_i - b_i` (zeros included).
    pub median_difference: f64,
    /// Exact binomial order-statistic confidence interval `(lower, upper)` for the median difference (`None` when `n_total < 3`).
    pub ci: Option<(f64, f64)>,
    /// Actual discrete achieved coverage percentage `>= nominal` for `ci` (`None` when `ci` is `None`).
    pub ci_actual_confidence_percent: Option<f64>,
    /// Exact `Binomial(n_nonzero, 0.5)` sign-test *P* value (`two_tailed` doubles smaller tail vs observed-direction smaller tail).
    pub p_value: f64,
    /// True for two-sided *P* value; false for directional one-sided test.
    pub two_tailed: bool,
}

/// Exact paired sign test on matched pairs `(a_i, b_i)` evaluating `d_i = a_i - b_i` after
/// dropping exact zero ties (`d_i == 0`), with exact order-statistic binomial CI for median `d`.
///
/// Exact `Binomial(n_nonzero, 1/2)` sign test (Dixon & Mood 1946; Conover 1999) and the
/// distribution-free order-statistic interval for the median (Conover 1999). The two-sided *p*-value
/// doubles the smaller tail; the one-sided *p*-value is the smaller tail, i.e. it is taken in the
/// observed direction rather than a pre-specified one. The median CI is computed over all
/// differences, zeros included.
pub fn paired_sign_test(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<PairedSignTestResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.len() != b.len() {
        return Err(StatsError::DimensionMismatch { expected: a.len(), actual: b.len() });
    }
    let diffs: Vec<f64> = a.iter().zip(b.iter()).map(|(&x, &y)| x - y).collect();
    validate_finite_slice(&diffs)?;
    let n_total = diffs.len();
    let n_positive = diffs.iter().filter(|&&d| d > 0.0).count();
    let n_negative = diffs.iter().filter(|&&d| d < 0.0).count();
    let n_ties = n_total - n_positive - n_negative;
    let n_nonzero = n_positive + n_negative;

    let p_value = if n_nonzero == 0 {
        1.0
    } else {
        let k_min = n_positive.min(n_negative);
        let tail = reg_beta(0.5, (n_nonzero - k_min) as f64, (k_min + 1) as f64)?.clamp(0.0, 1.0);
        if two_tailed { if k_min * 2 == n_nonzero { 1.0 } else { (2.0 * tail).clamp(0.0, 1.0) } } else { tail }
    };

    let sorted = sorted_finite(&diffs);
    let (_, median_difference, _) = quartiles_hinge(&sorted).ok_or(StatsError::EmptyInput)?;
    let (ci, ci_actual_confidence_percent) = match exact_binomial_median_ci(&diffs, confidence) {
        Ok(mci) => (Some((mci.lower, mci.upper)), Some(mci.actual_confidence_percent)),
        Err(_) => (None, None),
    };

    Ok(PairedSignTestResult {
        n_total,
        n_positive,
        n_negative,
        n_ties,
        n_nonzero,
        median_difference,
        ci,
        ci_actual_confidence_percent,
        p_value,
        two_tailed,
    })
}

/// Output of a two-sample Kolmogorov–Smirnov test.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TwoSampleKsResult {
    /// Sample size in group A.
    pub n_a: usize,
    /// Sample size in group B.
    pub n_b: usize,
    /// Kolmogorov–Smirnov supremum empirical CDF discrepancy `D = sup_x |F_a(x) - F_b(x)|` in `[0, 1]`.
    pub d_statistic: f64,
    /// Two-sided *P* value testing equality of distributions (exact Smirnov lattice path DP when `n_a <= 25`, `n_b <= 25`, and cross-tie-free; else Smirnov asymptotic series).
    pub p_value: f64,
    /// True when exact Smirnov combinatorial DP was used; false when asymptotic Smirnov series was used.
    pub exact_p_used: bool,
}

/// Two-sample Kolmogorov–Smirnov test (exact Smirnov lattice path DP when `n_a <= 25` and `n_b <= 25`
/// with no ties between the samples, Smirnov asymptotic series otherwise).
///
/// `D = sup |F_a(x) - F_b(x)|`. The exact two-sided *p*-value `P(D >= d)` counts lattice paths
/// (Hodges 1958); the asymptotic *p*-value evaluates the limiting Kolmogorov–Smirnov series at
/// `lambda = D sqrt(n_a n_b / (n_a + n_b))` (Smirnov 1948) without a finite-sample correction.
pub fn ks_two_sample(a: &[f64], b: &[f64]) -> Result<TwoSampleKsResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    let sa = sorted_finite(a);
    let sb = sorted_finite(b);
    let na = sa.len();
    let nb = sb.len();
    let d_stat = compute_ks_d(&sa, &sb);

    let (p_value, exact_p_used) = if na <= 25 && nb <= 25 && !has_cross_ties(&sa, &sb) {
        (ks_exact_smirnov_p(na, nb, d_stat), true)
    } else {
        let naf = na as f64;
        let nbf = nb as f64;
        let eff = (naf * nbf / (naf + nbf)).sqrt();
        (ks_asymptotic_p(d_stat * eff), false)
    };

    Ok(TwoSampleKsResult { n_a: na, n_b: nb, d_statistic: d_stat, p_value, exact_p_used })
}

fn compute_ks_d(sa: &[f64], sb: &[f64]) -> f64 {
    let na = sa.len() as f64;
    let nb = sb.len() as f64;
    let mut i = 0_usize;
    let mut j = 0_usize;
    let mut d_max = 0.0_f64;
    while i < sa.len() && j < sb.len() {
        let v = sa[i].min(sb[j]);
        while i < sa.len() && sa[i] == v {
            i += 1;
        }
        while j < sb.len() && sb[j] == v {
            j += 1;
        }
        let diff = ((i as f64) / na - (j as f64) / nb).abs();
        d_max = d_max.max(diff);
    }
    d_max
}

fn has_cross_ties(sa: &[f64], sb: &[f64]) -> bool {
    let mut i = 0;
    let mut j = 0;
    while i < sa.len() && j < sb.len() {
        if sa[i] == sb[j] {
            return true;
        }
        if sa[i] < sb[j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    false
}

/// Exact two-sided Smirnov `P(D >= d_obs)` by counting monotone lattice paths from `(0, 0)` to
/// `(n_a, n_b)` that stay strictly inside `|i/n_a - j/n_b| < d_obs` (Hodges 1958).
fn ks_exact_smirnov_p(na: usize, nb: usize, d_obs: f64) -> f64 {
    let naf = na as f64;
    let nbf = nb as f64;
    let eps = 1e-12;
    let mut dp = vec![vec![0.0_f64; nb + 1]; na + 1];
    dp[0][0] = 1.0;
    for i in 0..=na {
        for j in 0..=nb {
            if i == 0 && j == 0 {
                continue;
            }
            let diff = ((i as f64) / naf - (j as f64) / nbf).abs();
            if diff + eps >= d_obs {
                dp[i][j] = 0.0;
            } else {
                let from_i = if i > 0 { dp[i - 1][j] } else { 0.0 };
                let from_j = if j > 0 { dp[i][j - 1] } else { 0.0 };
                dp[i][j] = from_i + from_j;
            }
        }
    }
    let total = (1..=na).fold(1.0_f64, |acc, k| acc * ((nb + k) as f64) / (k as f64));
    let paths_strictly_below = dp[na][nb];
    (1.0 - paths_strictly_below / total).clamp(0.0, 1.0)
}

// ============================================================================
// Bland-Altman Method Comparison
// ============================================================================

/// Bland–Altman method-comparison bias and Limits of Agreement (Bland & Altman 1986, 1999).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlandAltmanResult {
    /// Number of paired method measurements included.
    pub n: usize,
    /// True if differences are expressed as percentages `100 * (a - b) / ((a + b) / 2)`; false for raw difference `a - b`.
    pub percent_difference: bool,
    /// Mean bias `mean(d)` across paired differences (in raw difference units or percent).
    pub bias: f64,
    /// Sample standard deviation of paired differences `sd(d)`.
    pub sd_bias: f64,
    /// Standard error of mean bias `sd(d) / sqrt(n)`.
    pub se_bias: f64,
    /// Two-sided `t(n - 1)` confidence interval `(lower, upper)` for mean `bias` at configured `ConfidenceLevel`.
    pub bias_ci: (f64, f64),
    /// Lower Limit of Agreement `bias - z_{alpha/2} * sd_bias`.
    pub loa_lower: f64,
    /// Upper Limit of Agreement `bias + z_{alpha/2} * sd_bias`.
    pub loa_upper: f64,
    /// Two-sided Bland–Altman (1999) `t(n - 1)` confidence interval for `loa_lower` using `SE_LoA = sd_bias * sqrt(1/n + z^2 / (2(n - 1)))`.
    pub loa_lower_ci: (f64, f64),
    /// Two-sided Bland–Altman (1999) `t(n - 1)` confidence interval for `loa_upper` using `SE_LoA`.
    pub loa_upper_ci: (f64, f64),
    /// `(average, difference_or_percent_diff)` points for Bland–Altman plotting.
    pub points: Vec<(f64, f64)>,
}

/// Compute Bland–Altman agreement (`method_a - method_b` vs `(method_a + method_b) / 2`).
///
/// Bias `mean(d)` with a *t* interval and limits of agreement `mean(d) ± z * sd(d)` (Bland & Altman
/// 1986), where `z` is the normal quantile for the requested confidence (1.96 at 95%). The limits'
/// intervals use the approximate standard error `sd(d) * sqrt(1/n + z^2 / (2(n - 1)))` with a
/// *t*(`n - 1`) multiplier (Bland & Altman 1999; the 1986 paper's `sqrt(3 sd^2 / n)` is the
/// `z ≈ 2` simplification). The optional percent difference `100 (a - b) / mean(a, b)` is discussed in
/// Bland & Altman (1999) and requires strictly positive paired measurements (`a > 0` and `b > 0`).
pub fn bland_altman(
    method_a: &[f64],
    method_b: &[f64],
    percent_difference: bool,
    confidence: ConfidenceLevel,
) -> Result<BlandAltmanResult, StatsError> {
    validate_finite_slice(method_a)?;
    validate_finite_slice(method_b)?;
    if method_a.len() != method_b.len() {
        return Err(StatsError::DimensionMismatch { expected: method_a.len(), actual: method_b.len() });
    }
    let n = method_a.len();
    if n < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Bland-Altman", required: 2, actual: n });
    }
    let mut points = Vec::with_capacity(n);
    let mut diffs = Vec::with_capacity(n);
    for (&a, &b) in method_a.iter().zip(method_b.iter()) {
        let avg = if (a + b).is_finite() { 0.5 * (a + b) } else { 0.5 * a + 0.5 * b };
        if !avg.is_finite() {
            return Err(StatsError::InvalidParameter { name: "bland_altman_overflow", value: avg });
        }
        let d = if percent_difference {
            if avg <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "average", value: avg });
            }
            if a <= 0.0 || b <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "percent_difference_measurement", value: a.min(b) });
            }
            if (a - b).is_finite() {
                (100.0 / avg) * (a - b)
            } else {
                let scale = a.abs().max(b.abs());
                100.0 * ((a / scale - b / scale) / (avg / scale))
            }
        } else {
            a - b
        };
        if !d.is_finite() {
            return Err(StatsError::InvalidParameter { name: "bland_altman_overflow", value: d });
        }
        points.push((avg, d));
        diffs.push(d);
    }

    let max_abs = diffs.iter().fold(0.0_f64, |acc, &v| acc.max(v.abs()));
    let (bias, sd_bias, se_bias) = if max_abs == 0.0 {
        (0.0, 0.0, 0.0)
    } else {
        let scaled: Vec<f64> = diffs.iter().map(|&d| d / max_abs).collect();
        let md = SummaryMoments::from_slice(&scaled)?;
        (md.mean * max_abs, md.sample_sd()? * max_abs, md.sem()? * max_abs)
    };
    let nf = n as f64;
    let df = nf - 1.0;
    let z_loa = norm_two_sided_isf(confidence.alpha())?;
    let t_crit = t_crit_two_sided(confidence, df)?;

    let loa_lower = bias - z_loa * sd_bias;
    let loa_upper = bias + z_loa * sd_bias;
    let se_loa = sd_bias * (1.0 / nf + (z_loa * z_loa) / (2.0 * df)).sqrt();
    let bias_ci = (bias - t_crit * se_bias, bias + t_crit * se_bias);
    let loa_lower_ci = (loa_lower - t_crit * se_loa, loa_lower + t_crit * se_loa);
    let loa_upper_ci = (loa_upper - t_crit * se_loa, loa_upper + t_crit * se_loa);
    let outputs_finite =
        [bias, sd_bias, se_bias, loa_lower, loa_upper, bias_ci.0, bias_ci.1, loa_lower_ci.0, loa_lower_ci.1, loa_upper_ci.0, loa_upper_ci.1]
            .iter()
            .all(|v| v.is_finite());
    if !outputs_finite {
        return Err(StatsError::InvalidParameter { name: "bland_altman_overflow", value: loa_upper });
    }

    Ok(BlandAltmanResult { n, percent_difference, bias, sd_bias, se_bias, bias_ci, loa_lower, loa_upper, loa_lower_ci, loa_upper_ci, points })
}

// ============================================================================
// Multiple Row-wise Tests (Volcano Plot Source)
// ============================================================================

/// Single row comparison entry for multiple *t*-tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleRowTestEntry {
    /// Zero-based table row index tested.
    pub row_index: usize,
    /// Human-readable row feature label.
    pub label: String,
    /// Sample mean (or median in nonparametric rows) of group A on this row.
    pub mean_a: f64,
    /// Sample mean (or median in nonparametric rows) of group B on this row.
    pub mean_b: f64,
    /// Signed shift/difference `mean_a - mean_b` (or HL shift / difference of paired medians).
    pub difference: f64,
    /// Row-level test statistic (*t*, Mann–Whitney *U*/*U_min*, or Wilcoxon *W*/*stat*).
    pub statistic: f64,
    /// Degrees of freedom (`n_a + n_b - 2`, Welch *df*, `n_pairs - 1`, or `f64::INFINITY` for rank tests).
    pub df: f64,
    /// Unadjusted two-sided single-row *P* value.
    pub raw_p: f64,
    /// Familywise-error adjusted *P* or FDR-adjusted *q* value across all tested rows.
    pub adjusted_p_or_q: f64,
    /// True when the row passes the multiplicity threshold (discovery).
    pub significant: bool,
}

/// Reject ANOVA-specific pairwise/post-hoc procedures that are undefined for raw-P row families.
fn validate_raw_p_multiplicity_procedure(procedure: MultiplicityProcedure) -> Result<(), StatsError> {
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
        MultiplicityProcedure::None
        | MultiplicityProcedure::FisherLsd
        | MultiplicityProcedure::Sidak
        | MultiplicityProcedure::Bonferroni
        | MultiplicityProcedure::HolmSidak
        | MultiplicityProcedure::HolmBonferroni
        | MultiplicityProcedure::BenjaminiHochbergFdr
        | MultiplicityProcedure::BenjaminiYekutieliFdr
        | MultiplicityProcedure::TwoStageBkyFdr => Ok(()),
    }
}

/// Run one unpaired or Welch *t*-test per row across two replicate groups and apply multiplicity.
///
/// Each row uses [`unpaired_t_test`] (Student 1908) or [`welch_t_test`] (Welch 1947); the adjustment
/// is delegated to `crate::multiplicity`, which carries its own references. Rows whose test fails
/// (e.g. fewer than two values or zero variance) are omitted from the output and from the adjustment.
/// ANOVA-only procedures (`TukeyKramer`, `NewmanKeuls`, `Dunnett`, `DunnettT3`, `GamesHowell`,
/// `DunnRankSum`, `LinearTrend`, `TamhaneT2`) are rejected with [`StatsError::IncompatibleAnalysisSpec`].
pub fn multiple_row_t_tests(
    row_labels: &[String],
    group_a_rows: &[Vec<f64>],
    group_b_rows: &[Vec<f64>],
    welch: bool,
    procedure: MultiplicityProcedure,
    threshold: f64,
) -> Result<Vec<MultipleRowTestEntry>, StatsError> {
    validate_raw_p_multiplicity_procedure(procedure)?;
    if group_a_rows.len() != group_b_rows.len() {
        return Err(StatsError::DimensionMismatch { expected: group_a_rows.len(), actual: group_b_rows.len() });
    }
    let mut raw_p = Vec::new();
    let mut staged = Vec::new();
    for (idx, (ra, rb)) in group_a_rows.iter().zip(group_b_rows.iter()).enumerate() {
        let res =
            if welch { welch_t_test(ra, rb, true, ConfidenceLevel::DEFAULT_95) } else { unpaired_t_test(ra, rb, true, ConfidenceLevel::DEFAULT_95) };
        if let Ok(r) = res
            && r.p_value.is_finite()
        {
            let lbl = row_labels.get(idx).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", idx + 1));
            raw_p.push(r.p_value);
            staged.push((idx, lbl, r.mean_a, r.mean_b, r.difference, r.t_statistic, r.df, r.p_value));
        }
    }
    let (adj, _) = adjust_p_values(&raw_p, procedure, threshold);
    Ok(staged
        .into_iter()
        .zip(adj)
        .map(|((row_index, label, mean_a, mean_b, difference, statistic, df, raw_p), adj_p)| MultipleRowTestEntry {
            row_index,
            label,
            mean_a,
            mean_b,
            difference,
            statistic,
            df,
            raw_p,
            adjusted_p_or_q: adj_p,
            significant: threshold.is_finite() && adj_p.is_finite() && adj_p <= threshold,
        })
        .collect())
}

// ============================================================================
// ResultSheet Builder
// ============================================================================

fn extract_pre_summarized_group(
    col_tab: &curvebook_doc::ColumnTable,
    col_idx: usize,
    conf: ConfidenceLevel,
) -> Result<(String, f64, f64, f64), StatsError> {
    let ds = col_tab.data_sets.get(col_idx).ok_or(StatsError::MissingColumn { index: col_idx })?;
    let mut pts = Vec::new();
    for r in 0..ds.row_count() {
        if let Some(pt) = col_tab.summarized_point(r, col_idx)? {
            pts.push(pt);
        }
    }
    if pts.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let pooled = summarize_pre_summarized_points(&ds.title, &pts, conf)?;
    if pooled.total_n < 2.0 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Err(StatsError::InsufficientSampleSize {
            context: "pre-summarized t-test group",
            required: 2,
            actual: pooled.total_n.max(0.0) as usize,
        });
    }
    let sd = pooled.sd.ok_or(StatsError::InsufficientSampleSize { context: "pre-summarized t-test SD", required: 2, actual: 1 })?;
    Ok((pooled.label, pooled.mean, sd, pooled.total_n))
}

/// Execute [`TwoGroupSpec`] on a [`DataSheet`] and emit a complete [`ResultSheet`].
pub fn run_two_group_analysis(sheet: &DataSheet, spec: &AnalysisSpec, tg: &TwoGroupSpec) -> Result<ResultSheet, StatsError> {
    let mut res = run_two_group_analysis_inner(sheet, spec, tg)?;
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}

fn run_two_group_analysis_inner(sheet: &DataSheet, spec: &AnalysisSpec, tg: &TwoGroupSpec) -> Result<ResultSheet, StatsError> {
    let mut res = init_result_sheet(spec)?;
    let conf = spec.confidence_level;
    let p_fmt = spec.p_value_format;

    if let TableArchetype::Column(col_tab) = &sheet.archetype
        && col_tab.subcolumn_mode.is_pre_summarized()
    {
        if !col_tab.subcolumn_mode.supports_parametric_moments() {
            return Err(StatsError::RequiresRawReplicates);
        }
        match tg.method {
            TwoGroupMethod::OneSampleT { hypothetical } => {
                let indices = try_resolve_selected_indices(&spec.selected_data_sets, col_tab.data_sets.len())?;
                if !spec.selected_data_sets.is_empty() && indices.len() != 1 {
                    return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
                }
                let &first_idx = indices.first().ok_or(StatsError::EmptyInput)?;
                let (label, mean, sd, n) = extract_pre_summarized_group(col_tab, first_idx, conf)?;
                let out = one_sample_t_test_from_summary(mean, sd, n, hypothetical, tg.two_tailed, conf)?;
                populate_one_sample_t_sheet_impl(&mut res, &label, None, &out, conf, p_fmt);
                return Ok(res);
            }
            TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT => {
                let indices = try_resolve_selected_indices(&spec.selected_data_sets, col_tab.data_sets.len())?;
                if !spec.selected_data_sets.is_empty() && indices.len() != 2 {
                    return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
                }
                if indices.len() < 2 {
                    return Err(StatsError::InsufficientSampleSize { context: "two-group comparison columns", required: 2, actual: indices.len() });
                }
                let (idx_a, idx_b) = (indices[0], indices[1]);
                let (lbl_a, mean_a, sd_a, n_a) = extract_pre_summarized_group(col_tab, idx_a, conf)?;
                let (lbl_b, mean_b, sd_b, n_b) = extract_pre_summarized_group(col_tab, idx_b, conf)?;
                let out = if matches!(tg.method, TwoGroupMethod::WelchT) {
                    welch_t_test_from_summary((mean_a, sd_a, n_a), (mean_b, sd_b, n_b), tg.two_tailed, conf)?
                } else {
                    unpaired_t_test_from_summary((mean_a, sd_a, n_a), (mean_b, sd_b, n_b), tg.two_tailed, conf)?
                };
                populate_unpaired_t_sheet_impl(&mut res, idx_a, idx_b, &lbl_a, &lbl_b, &out, tg.compute_effect_sizes, conf, p_fmt, false);
                return Ok(res);
            }
            _ => return Err(StatsError::RequiresRawReplicates),
        }
    }

    match tg.method {
        TwoGroupMethod::OneSampleT { hypothetical } => {
            let sel = resolve_mv_one_sample_selection(sheet, &spec.selected_data_sets, tg.mv_response_column)?;
            let cols = extract_numeric_columns(sheet, &sel)?;
            let first = cols.first().ok_or(StatsError::EmptyInput)?;
            let out = crate::column::one_sample_t_test(&first.values, hypothetical, tg.two_tailed, conf)?;
            populate_one_sample_t_sheet_impl(&mut res, &first.label, Some(&first.values), &out, conf, p_fmt);
            if first.non_numeric_count > 0 {
                append_ignored_text_checklist_and_footnote(&mut res, "one-sample t test", &[(first.label.clone(), first.non_numeric_count)]);
            }
            return Ok(res);
        }
        TwoGroupMethod::OneSampleWilcoxon { hypothetical } => {
            let sel = resolve_mv_one_sample_selection(sheet, &spec.selected_data_sets, tg.mv_response_column)?;
            let cols = extract_numeric_columns(sheet, &sel)?;
            let first = cols.first().ok_or(StatsError::EmptyInput)?;
            let out = crate::column::one_sample_wilcoxon(&first.values, hypothetical, tg.two_tailed, conf)?;
            populate_one_sample_w_sheet(&mut res, &first.label, &out, conf, p_fmt);
            if first.non_numeric_count > 0 {
                append_ignored_text_checklist_and_footnote(&mut res, "one-sample Wilcoxon test", &[(first.label.clone(), first.non_numeric_count)]);
            }
            return Ok(res);
        }
        _ => {}
    }

    if let Some(mv_in) = resolve_mv_two_group_inputs(sheet, spec, tg)? {
        let MvTwoGroupExtraction { group_a: lvl_a, group_b: lvl_b, skipped_missing, ignored_non_numeric } = mv_in;
        if matches!(
            tg.method,
            TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest
        ) {
            return Err(StatsError::InvalidParameter { name: "paired_method_on_mv_categorical_grouping", value: 0.0 });
        }
        match tg.method {
            TwoGroupMethod::UnpairedT => {
                let out = unpaired_t_test(&lvl_a.values, &lvl_b.values, tg.two_tailed, conf)?;
                populate_unpaired_t_sheet(&mut res, 0, 1, &lvl_a.label, &lvl_b.label, &out, tg.compute_effect_sizes, conf, p_fmt);
                append_unpaired_normality_checklist(&mut res.checklist, &lvl_a.values, &lvl_b.values, false, conf.alpha());
            }
            TwoGroupMethod::WelchT => {
                let out = welch_t_test(&lvl_a.values, &lvl_b.values, tg.two_tailed, conf)?;
                populate_unpaired_t_sheet(&mut res, 0, 1, &lvl_a.label, &lvl_b.label, &out, tg.compute_effect_sizes, conf, p_fmt);
                append_unpaired_normality_checklist(&mut res.checklist, &lvl_a.values, &lvl_b.values, false, conf.alpha());
            }
            TwoGroupMethod::LognormalUnpairedT => {
                let out = lognormal_unpaired_t_test(&lvl_a.values, &lvl_b.values, tg.two_tailed, conf)?;
                populate_unpaired_t_sheet(&mut res, 0, 1, &lvl_a.label, &lvl_b.label, &out, tg.compute_effect_sizes, conf, p_fmt);
                append_unpaired_normality_checklist(&mut res.checklist, &lvl_a.values, &lvl_b.values, true, conf.alpha());
            }
            TwoGroupMethod::MannWhitneyU => {
                let out = mann_whitney_u(&lvl_a.values, &lvl_b.values, tg.two_tailed, conf)?;
                populate_mann_whitney_sheet(&mut res, 0, 1, &lvl_a.label, &lvl_b.label, &out, conf, p_fmt);
            }
            TwoGroupMethod::KolmogorovSmirnov => {
                let out = ks_two_sample(&lvl_a.values, &lvl_b.values)?;
                populate_ks_sheet(&mut res, (0, 1), (&lvl_a.label, &lvl_b.label), &out, conf, p_fmt);
            }
            TwoGroupMethod::PairedT
            | TwoGroupMethod::RatioPairedT
            | TwoGroupMethod::WilcoxonSignedRank
            | TwoGroupMethod::PairedSignTest
            | TwoGroupMethod::OneSampleT { .. }
            | TwoGroupMethod::OneSampleWilcoxon { .. } => {}
        }
        append_listwise_filtered_checklist_and_footnote(&mut res, "Two-group comparison", skipped_missing, lvl_a.values.len() + lvl_b.values.len());
        append_ignored_text_checklist_and_footnote(&mut res, "two-group comparison", &ignored_non_numeric);
        return Ok(res);
    }

    let cols = extract_two_group_wide_columns(sheet, &spec.selected_data_sets)?;
    if cols.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "two-group comparison columns", required: 2, actual: cols.len() });
    }
    let (ca, cb) = (&cols[0], &cols[1]);
    let unpaired_ignored = |cols: &[crate::common::ExtractedColumn]| -> Vec<(String, usize)> {
        cols.iter().filter(|c| c.non_numeric_count > 0).map(|c| (c.label.clone(), c.non_numeric_count)).collect()
    };

    match tg.method {
        TwoGroupMethod::UnpairedT => {
            let out = unpaired_t_test(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_unpaired_t_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, tg.compute_effect_sizes, conf, p_fmt);
            append_unpaired_normality_checklist(&mut res.checklist, &ca.values, &cb.values, false, conf.alpha());
            append_ignored_text_checklist_and_footnote(&mut res, "unpaired t test", &unpaired_ignored(&cols));
        }
        TwoGroupMethod::WelchT => {
            let out = welch_t_test(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_unpaired_t_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, tg.compute_effect_sizes, conf, p_fmt);
            append_unpaired_normality_checklist(&mut res.checklist, &ca.values, &cb.values, false, conf.alpha());
            append_ignored_text_checklist_and_footnote(&mut res, "Welch t test", &unpaired_ignored(&cols));
        }
        TwoGroupMethod::LognormalUnpairedT => {
            let out = lognormal_unpaired_t_test(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_unpaired_t_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, tg.compute_effect_sizes, conf, p_fmt);
            append_unpaired_normality_checklist(&mut res.checklist, &ca.values, &cb.values, true, conf.alpha());
            append_ignored_text_checklist_and_footnote(&mut res, "lognormal unpaired t test", &unpaired_ignored(&cols));
        }
        TwoGroupMethod::PairedT => {
            let report = extract_paired_values_with_report(sheet, ca.index, cb.index)?;
            let out = paired_t_test(&report.xs, &report.ys, tg.two_tailed, conf)?;
            populate_paired_t_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, tg.compute_effect_sizes, conf, p_fmt);
            append_paired_normality_checklist(&mut res.checklist, &report.xs, &report.ys, false, conf.alpha());
            report_paired_extraction(&mut res, "Paired t test", &ca.label, &cb.label, &report);
        }
        TwoGroupMethod::RatioPairedT => {
            let report = extract_paired_values_with_report(sheet, ca.index, cb.index)?;
            let out = ratio_paired_t_test(&report.xs, &report.ys, tg.two_tailed, conf)?;
            populate_paired_t_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, tg.compute_effect_sizes, conf, p_fmt);
            append_paired_normality_checklist(&mut res.checklist, &report.xs, &report.ys, true, conf.alpha());
            report_paired_extraction(&mut res, "Ratio paired t test", &ca.label, &cb.label, &report);
        }
        TwoGroupMethod::MannWhitneyU => {
            let out = mann_whitney_u(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_mann_whitney_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, conf, p_fmt);
            append_ignored_text_checklist_and_footnote(&mut res, "Mann-Whitney test", &unpaired_ignored(&cols));
        }
        TwoGroupMethod::WilcoxonSignedRank => {
            let report = extract_paired_values_with_report(sheet, ca.index, cb.index)?;
            let out = wilcoxon_matched_pairs(&report.xs, &report.ys, tg.two_tailed, conf, true)?;
            populate_wilcoxon_paired_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, conf, p_fmt);
            report_paired_extraction(&mut res, "Wilcoxon signed-rank test", &ca.label, &cb.label, &report);
        }
        TwoGroupMethod::PairedSignTest => {
            let report = extract_paired_values_with_report(sheet, ca.index, cb.index)?;
            let out = paired_sign_test(&report.xs, &report.ys, tg.two_tailed, conf)?;
            populate_paired_sign_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, conf, p_fmt);
            report_paired_extraction(&mut res, "Paired sign test", &ca.label, &cb.label, &report);
        }
        TwoGroupMethod::KolmogorovSmirnov => {
            let out = ks_two_sample(&ca.values, &cb.values)?;
            populate_ks_sheet(&mut res, (ca.index, cb.index), (&ca.label, &cb.label), &out, conf, p_fmt);
            append_ignored_text_checklist_and_footnote(&mut res, "Kolmogorov-Smirnov test", &unpaired_ignored(&cols));
        }
        TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. } => {}
    }
    Ok(res)
}

fn report_paired_extraction(res: &mut ResultSheet, analysis: &str, lbl_a: &str, lbl_b: &str, report: &crate::common::PairedValuesReport) {
    report.accounting.push_scalars(res);
    append_listwise_filtered_checklist_and_footnote(res, analysis, report.skipped_incomplete_rows, report.xs.len());
    if report.non_numeric_cells > 0 {
        append_ignored_text_checklist_and_footnote(res, analysis, &[(format!("{lbl_a} / {lbl_b}"), report.non_numeric_cells)]);
    }
}

/// Execute [`BlandAltmanSpec`] on a `Column`, `MultipleVariables`, or `Xy` [`DataSheet`] and emit a
/// complete [`ResultSheet`] (`MainSummary`, pair coordinate table, `scalars`, and overlay curve).
pub fn run_bland_altman_analysis(sheet: &DataSheet, spec: &AnalysisSpec, ba: &BlandAltmanSpec) -> Result<ResultSheet, StatsError> {
    let mut res = init_result_sheet(spec)?;
    let conf = spec.confidence_level;
    let ba_report = extract_bland_altman_pairs(sheet, ba)?;
    let (lbl_a, lbl_b, vals_a, vals_b) = (ba_report.label_a, ba_report.label_b, ba_report.xs, ba_report.ys);
    let out = bland_altman(&vals_a, &vals_b, ba.percent_difference, conf)?;

    let mode_text = if out.percent_difference { "Percent difference (100 * (A - B) / Mean)" } else { "Absolute difference (A - B)" };
    let pct = conf.percent();
    res.tabs.push(
        KvTabBuilder::new("bland-altman-summary", "Bland-Altman method comparison", ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
            .text("Difference mode", mode_text)
            .num("N pairs", out.n as f64)
            .num("Bias", out.bias)
            .num("SD of bias", out.sd_bias)
            .num("SE of bias", out.se_bias)
            .ci(format!("{pct}% CI of bias"), out.bias_ci.0, out.bias_ci.1)
            .num(format!("Lower {pct}% LoA"), out.loa_lower)
            .ci(format!("{pct}% CI of lower LoA"), out.loa_lower_ci.0, out.loa_lower_ci.1)
            .num(format!("Upper {pct}% LoA"), out.loa_upper)
            .ci(format!("{pct}% CI of upper LoA"), out.loa_upper_ci.0, out.loa_upper_ci.1)
            .build(),
    );

    let diff_header = if out.percent_difference { "Percent Difference (%)" } else { "Difference (A - B)" };
    let mut coord_tab = TabBuilder::new("bland-altman-coordinates", "Bland-Altman Pair Coordinates", ResultTabKind::Residuals).headers([
        String::from("Average (A + B) / 2"),
        String::from(diff_header),
        lbl_a.clone(),
        lbl_b.clone(),
    ]);
    let mut overlay_pts = Vec::with_capacity(out.n);
    for (idx, ((&(avg, diff), &a), &b)) in out.points.iter().zip(vals_a.iter()).zip(vals_b.iter()).enumerate() {
        coord_tab.push_row(format!("Pair {}", idx + 1), [num_cell(avg), num_cell(diff), num_cell(a), num_cell(b)]);
        overlay_pts.push(CurveSamplePoint {
            x: avg,
            y: diff,
            ci_lower: Some(out.loa_lower),
            ci_upper: Some(out.loa_upper),
            pred_lower: None,
            pred_upper: None,
        });
    }
    res.tabs.push(coord_tab.build());
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: ba.column_a,
        equation_id: String::from("BlandAltman"),
        converged: true,
        ambiguous: false,
        points: overlay_pts,
    });

    push_scalar(&mut res, "N", None, out.n as f64);
    push_scalar(&mut res, "Bias", None, out.bias);
    push_scalar(&mut res, "SDBias", None, out.sd_bias);
    push_scalar(&mut res, "SEBias", None, out.se_bias);
    push_scalar(&mut res, "LoALower", None, out.loa_lower);
    push_scalar(&mut res, "LoAUpper", None, out.loa_upper);
    res.checklist.push(checklist_item(
        "Bland-Altman sample size & normality of differences",
        if out.n >= 10 { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(format!("n = {} pairs", out.n)),
        "Limits of Agreement assume differences follow an approximately normal distribution with constant variance across the measurement range.",
    ));
    ba_report.accounting.push_scalars(&mut res);
    append_listwise_filtered_checklist_and_footnote(&mut res, "Bland-Altman agreement", ba_report.skipped_incomplete_rows, out.n);
    if ba_report.non_numeric_cells > 0 {
        append_ignored_text_checklist_and_footnote(
            &mut res,
            "Bland-Altman agreement",
            &[(format!("{lbl_a} / {lbl_b}"), ba_report.non_numeric_cells)],
        );
    }
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}

/// Execute [`MultipleRowTestsSpec`] row-by-row across two groups and emit a complete [`ResultSheet`]
/// (`MainSummary`, `MultipleComparisons`, `scalars`, and `PairwiseComparisonRecord`s for Volcano plots).
pub fn run_multiple_row_t_tests_analysis(sheet: &DataSheet, spec: &AnalysisSpec, mr: &MultipleRowTestsSpec) -> Result<ResultSheet, StatsError> {
    let mut res = init_result_sheet(spec)?;
    let conf = spec.confidence_level;
    let p_fmt = spec.p_value_format;
    let threshold = mr.multiplicity.threshold;
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "multiplicity_threshold", value: threshold });
    }
    let proc_eff = MultiplicityProcedure::from(mr.multiplicity.method);
    validate_raw_p_multiplicity_procedure(proc_eff)?;

    let inputs = extract_multiple_row_inputs(sheet, spec, mr.paired)?;
    let (lbl_a, lbl_b, row_labels, group_a_rows, group_b_rows) = (inputs.label_a, inputs.label_b, inputs.row_labels, inputs.rows_a, inputs.rows_b);
    let mut raw_p = Vec::new();
    let mut staged = Vec::new();
    let mut skipped_rows = 0_usize;

    for (idx, (ra, rb)) in group_a_rows.iter().zip(group_b_rows.iter()).enumerate() {
        let row_lbl = row_labels.get(idx).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", idx + 1));
        let row_out = run_single_row_comparison(ra, rb, mr, conf);
        match row_out {
            Ok((mean_a, mean_b, diff, stat, df, p_val)) if p_val.is_finite() => {
                raw_p.push(p_val);
                staged.push((idx, row_lbl, mean_a, mean_b, diff, stat, df, p_val));
            }
            Ok(_) | Err(_) => {
                skipped_rows += 1;
            }
        }
    }

    if staged.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let (adj_p, is_fdr) = adjust_p_values(&raw_p, proc_eff, threshold);
    let mut entries = Vec::with_capacity(staged.len());
    for ((row_index, label, mean_a, mean_b, difference, statistic, df, rp), adj) in staged.into_iter().zip(adj_p) {
        entries.push(MultipleRowTestEntry {
            row_index,
            label,
            mean_a,
            mean_b,
            difference,
            statistic,
            df,
            raw_p: rp,
            adjusted_p_or_q: adj,
            significant: adj.is_finite() && adj <= threshold,
        });
    }

    let discoveries = entries.iter().filter(|e| e.significant).count();
    let min_raw_p = entries.iter().map(|e| e.raw_p).fold(f64::INFINITY, f64::min);
    let min_adj_p = entries.iter().map(|e| e.adjusted_p_or_q).fold(f64::INFINITY, f64::min);
    let variant_label = match (mr.paired, mr.nonparametric, mr.assume_equal_sd) {
        (true, true, _) => "Wilcoxon matched-pairs signed-rank",
        (true, false, _) => "Paired Student t test",
        (false, true, _) => "Mann-Whitney U test",
        (false, false, true) => "Unpaired Student t test",
        (false, false, false) => "Welch unpaired t test",
    };

    let pair_desc = format!("{lbl_a} vs. {lbl_b}");
    res.tabs.push(
        KvTabBuilder::new("multiple-row-summary", "Multiple Row t-Tests Summary", ResultTabKind::MainSummary, pair_desc.clone())
            .num("Rows tested", entries.len() as f64)
            .num("Rows skipped (insufficient data)", skipped_rows as f64)
            .num("Significant discoveries", discoveries as f64)
            .text("Test family", variant_label)
            .text("Multiplicity method", format!("{:?}", mr.multiplicity.method))
            .num("Significance / FDR threshold", threshold)
            .p("Minimum raw P", min_raw_p, p_fmt, conf.alpha())
            .p("Minimum adjusted P / q", min_adj_p, p_fmt, threshold)
            .build(),
    );

    let mut cmp_tab = TabBuilder::new("multiple-row-comparisons", "Multiple Row-Wise Comparisons", ResultTabKind::MultipleComparisons).headers([
        format!("Mean ({lbl_a})"),
        format!("Mean ({lbl_b})"),
        String::from("Difference (A - B)"),
        String::from("Statistic"),
        String::from("df"),
        String::from("Raw P"),
        String::from("Adjusted P / q"),
        String::from("Significant"),
    ]);
    for e in &entries {
        cmp_tab.push_row(
            e.label.clone(),
            [
                num_cell(e.mean_a),
                num_cell(e.mean_b),
                num_cell(e.difference),
                num_cell(e.statistic),
                num_cell(e.df),
                p_cell_at_alpha(e.raw_p, p_fmt, conf.alpha()),
                p_cell_at_alpha(e.adjusted_p_or_q, p_fmt, threshold),
                ResultCell::Boolean(e.significant),
            ],
        );
        res.pairwise_comparisons.push(PairwiseComparisonRecord {
            group_a: e.row_index,
            group_b: 1,
            label_a: e.label.clone(),
            label_b: pair_desc.clone(),
            estimate_diff: Some(e.difference),
            raw_p_or_q: e.adjusted_p_or_q,
            is_fdr_q: is_fdr,
            significant: e.significant,
        });
    }
    if mr.paired && mr.nonparametric {
        cmp_tab.push_footnote(
            "Paired nonparametric rows use Wilcoxon matched-pairs signed-rank with Pratt (1959) zero-difference handling; df is omitted (not applicable) for rank tests.",
        );
    } else if mr.nonparametric {
        cmp_tab.push_footnote("Unpaired nonparametric rows use the Mann-Whitney U test; df is omitted (not applicable) for rank tests.");
    }
    res.tabs.push(cmp_tab.build());

    push_scalar(&mut res, "RowsTested", None, entries.len() as f64);
    push_scalar(&mut res, "Discoveries", None, discoveries as f64);
    push_scalar(&mut res, "Threshold", None, threshold);
    push_scalar(&mut res, "MinRawP", None, min_raw_p);
    push_scalar(&mut res, "MinAdjustedP", None, min_adj_p);
    res.checklist.push(checklist_item(
        "Multiple row hypothesis testing",
        ChecklistStatus::Pass,
        Some(format!("{discoveries} / {} rows significant at threshold {threshold}", entries.len())),
        "Per-row test statistics and multiplicity-adjusted P/q values are available for Volcano plotting.",
    ));
    if mr.paired {
        append_listwise_filtered_checklist_and_footnote(&mut res, "Multiple row paired t-tests", inputs.skipped_incomplete_pairs, inputs.kept_pairs);
    }
    append_ignored_text_checklist_and_footnote(&mut res, "Multiple row t-tests", &inputs.ignored_non_numeric);
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}

#[derive(Debug, Clone)]
struct LabeledGroupSample {
    label: String,
    values: Vec<f64>,
}

struct MvTwoGroupExtraction {
    group_a: LabeledGroupSample,
    group_b: LabeledGroupSample,
    skipped_missing: usize,
    ignored_non_numeric: Vec<(String, usize)>,
}

fn resolve_mv_one_sample_selection(sheet: &DataSheet, selected: &[usize], mv_response: Option<usize>) -> Result<Vec<usize>, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(col_tab) => {
            let indices = try_resolve_selected_indices(selected, col_tab.data_sets.len())?;
            if !selected.is_empty() && indices.len() != 1 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
            }
            let &first_idx = indices.first().ok_or(StatsError::EmptyInput)?;
            Ok(vec![first_idx])
        }
        TableArchetype::MultipleVariables(mv) => {
            let indices = try_resolve_selected_indices(selected, mv.variables.len())?;
            if let Some(r_col) = mv_response {
                if r_col >= mv.variables.len() {
                    return Err(StatsError::MissingColumn { index: r_col });
                }
                return Ok(vec![r_col]);
            }
            let numeric_indices: Vec<usize> = indices.into_iter().filter(|&i| is_mv_numeric_response_candidate(&mv.variables[i])).collect();
            if !selected.is_empty() && numeric_indices.len() != 1 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: numeric_indices.len() as f64 });
            }
            let &first_idx = numeric_indices.first().ok_or(StatsError::EmptyInput)?;
            Ok(vec![first_idx])
        }
        _ => Ok(selected.to_vec()),
    }
}

fn extract_two_group_wide_columns(sheet: &DataSheet, selected: &[usize]) -> Result<Vec<crate::common::ExtractedColumn>, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(col_tab) => {
            if col_tab.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let indices = try_resolve_selected_indices(selected, col_tab.data_sets.len())?;
            if !selected.is_empty() && indices.len() != 2 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
            }
            if indices.len() < 2 {
                return Err(StatsError::InsufficientSampleSize { context: "two-group comparison columns", required: 2, actual: indices.len() });
            }
            extract_numeric_columns(sheet, &indices[..2])
        }
        TableArchetype::MultipleVariables(mv) => {
            let indices = try_resolve_selected_indices(selected, mv.variables.len())?;
            let numeric_indices: Vec<usize> = indices.into_iter().filter(|&i| is_mv_numeric_response_candidate(&mv.variables[i])).collect();
            if !selected.is_empty() && numeric_indices.len() != 2 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: numeric_indices.len() as f64 });
            }
            if numeric_indices.len() < 2 {
                return Err(StatsError::InsufficientSampleSize {
                    context: "two-group comparison columns",
                    required: 2,
                    actual: numeric_indices.len(),
                });
            }
            extract_numeric_columns(sheet, &numeric_indices[..2])
        }
        _ => extract_numeric_columns(sheet, selected),
    }
}

fn mv_cell_token(cell: &Cell) -> Option<String> {
    if cell.excluded {
        return None;
    }
    match &cell.value {
        CellValue::Empty => None,
        CellValue::Text(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        CellValue::Number(v) if v.is_finite() => Some(format!("{v}")),
        CellValue::Number(_) => None,
    }
}

fn infer_mv_group_column(mv: &MultipleVariablesTable, response_col: Option<usize>, selected: &[usize]) -> Result<usize, StatsError> {
    let pool = try_resolve_selected_indices(selected, mv.variables.len())?;
    let candidates: Vec<usize> = pool
        .into_iter()
        .filter(|&idx| Some(idx) != response_col && mv.variables.get(idx).is_some_and(|v| v.role == VariableRole::Categorical))
        .collect();
    match candidates.as_slice() {
        [only] => Ok(*only),
        _ => Err(StatsError::InvalidParameter { name: "mv_group_column", value: candidates.len() as f64 }),
    }
}

fn is_mv_numeric_response_candidate(var: &curvebook_doc::VariableColumn) -> bool {
    !matches!(var.role, VariableRole::Categorical | VariableRole::Label)
}

fn select_default_mv_response_column(mv: &MultipleVariablesTable, candidates: &[usize]) -> Option<usize> {
    if candidates.len() <= 1 {
        return candidates.first().copied();
    }
    let active: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|&idx| mv.variables.get(idx).is_some_and(|v| v.cells.iter().any(|c| c.active_number().is_some())))
        .collect();
    // On multi-column MV tables (such as qPCR pipelines with derived columns), prefer the last
    // active calculated response column, then the last active numeric column, then the last candidate.
    active
        .iter()
        .copied()
        .rfind(|&idx| mv.variables.get(idx).is_some_and(|v| v.role == VariableRole::Calculated))
        .or_else(|| active.last().copied())
        .or_else(|| candidates.last().copied())
}

fn infer_mv_response_column(mv: &MultipleVariablesTable, group_col: usize, selected: &[usize]) -> Result<usize, StatsError> {
    let pool = try_resolve_selected_indices(selected, mv.variables.len())?;
    let candidates: Vec<usize> =
        pool.into_iter().filter(|&idx| idx != group_col && mv.variables.get(idx).is_some_and(is_mv_numeric_response_candidate)).collect();
    select_default_mv_response_column(mv, &candidates)
        .ok_or(StatsError::InvalidParameter { name: "mv_response_column", value: candidates.len() as f64 })
}

fn resolve_mv_two_group_inputs(sheet: &DataSheet, spec: &AnalysisSpec, tg: &TwoGroupSpec) -> Result<Option<MvTwoGroupExtraction>, StatsError> {
    let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
        return Ok(None);
    };
    let selected = try_resolve_selected_indices(&spec.selected_data_sets, mv.variables.len())?;
    let explicit_mode = tg.mv_group_column.is_some() || tg.mv_response_column.is_some();
    let (g_col, r_col) = if explicit_mode {
        let g = match tg.mv_group_column {
            Some(g) => g,
            None => infer_mv_group_column(mv, tg.mv_response_column, &spec.selected_data_sets)?,
        };
        let r = match tg.mv_response_column {
            Some(r) => r,
            None => infer_mv_response_column(mv, g, &spec.selected_data_sets)?,
        };
        (g, r)
    } else {
        let cat_cols: Vec<usize> = selected.iter().copied().filter(|&i| mv.variables[i].role == VariableRole::Categorical).collect();
        let num_cols: Vec<usize> = selected.iter().copied().filter(|&i| mv.variables.get(i).is_some_and(is_mv_numeric_response_candidate)).collect();
        if cat_cols.len() >= 2 {
            return Err(StatsError::InvalidParameter { name: "mv_group_column", value: cat_cols.len() as f64 });
        }
        if cat_cols.len() == 1 {
            let r_col = select_default_mv_response_column(mv, &num_cols)
                .ok_or(StatsError::InvalidParameter { name: "mv_response_column", value: num_cols.len() as f64 })?;
            (cat_cols[0], r_col)
        } else {
            return Ok(None);
        }
    };

    let g_var = mv.variables.get(g_col).ok_or(StatsError::MissingColumn { index: g_col })?;
    let r_var = mv.variables.get(r_col).ok_or(StatsError::MissingColumn { index: r_col })?;
    if g_col == r_col {
        return Err(StatsError::InvalidParameter { name: "mv_group_column", value: g_col as f64 });
    }

    let levels: Vec<String> = g_var.ordered_levels().into_iter().filter(|l| l.included_count > 0).map(|l| l.level).collect();
    if levels.len() != 2 {
        return Err(StatsError::DimensionMismatch { expected: 2, actual: levels.len() });
    }

    let mut vals_a = Vec::new();
    let mut vals_b = Vec::new();
    let mut skipped_missing = 0_usize;
    let mut non_numeric_response = 0_usize;
    for r in 0..mv.row_count() {
        let Some(gc) = g_var.cells.get(r) else {
            continue;
        };
        let Some(rc) = r_var.cells.get(r) else {
            continue;
        };
        if is_ignored_non_numeric_cell(rc) {
            non_numeric_response += 1;
        }
        let tok = mv_cell_token(gc);
        let num = rc.active_number();
        match (tok, num) {
            (Some(t), Some(y)) if t == levels[0] => vals_a.push(y),
            (Some(t), Some(y)) if t == levels[1] => vals_b.push(y),
            _ => {
                if !gc.is_empty() || !rc.is_empty() || gc.excluded || rc.excluded {
                    skipped_missing += 1;
                }
            }
        }
    }

    if vals_a.is_empty() || vals_b.is_empty() {
        return Err(StatsError::InsufficientSampleSize {
            context: "two-group MV level observations",
            required: 1,
            actual: vals_a.len().min(vals_b.len()),
        });
    }

    let ignored = if non_numeric_response > 0 { vec![(r_var.title.clone(), non_numeric_response)] } else { Vec::new() };
    Ok(Some(MvTwoGroupExtraction {
        group_a: LabeledGroupSample { label: levels[0].clone(), values: vals_a },
        group_b: LabeledGroupSample { label: levels[1].clone(), values: vals_b },
        skipped_missing,
        ignored_non_numeric: ignored,
    }))
}

struct BlandAltmanExtractedPairs {
    label_a: String,
    label_b: String,
    xs: Vec<f64>,
    ys: Vec<f64>,
    skipped_incomplete_rows: usize,
    non_numeric_cells: usize,
    /// Pairs analyzed and omitted rows by reason.
    accounting: curvebook_doc::PairedObservationAccounting,
}

fn extract_bland_altman_pairs(sheet: &DataSheet, ba: &BlandAltmanSpec) -> Result<BlandAltmanExtractedPairs, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(col) => {
            if col.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            if ba.column_a == ba.column_b {
                return Err(StatsError::InvalidParameter { name: "column_b", value: ba.column_b as f64 });
            }
            let report = extract_paired_values_with_report(sheet, ba.column_a, ba.column_b)?;
            let la = col.data_sets[ba.column_a].title.clone();
            let lb = col.data_sets[ba.column_b].title.clone();
            Ok(BlandAltmanExtractedPairs {
                label_a: la,
                label_b: lb,
                xs: report.xs,
                ys: report.ys,
                skipped_incomplete_rows: report.skipped_incomplete_rows,
                non_numeric_cells: report.non_numeric_cells,
                accounting: report.accounting,
            })
        }
        TableArchetype::MultipleVariables(mv) => {
            if ba.column_a == ba.column_b {
                return Err(StatsError::InvalidParameter { name: "column_b", value: ba.column_b as f64 });
            }
            let report = extract_paired_values_with_report(sheet, ba.column_a, ba.column_b)?;
            let la = mv.variables[ba.column_a].title.clone();
            let lb = mv.variables[ba.column_b].title.clone();
            Ok(BlandAltmanExtractedPairs {
                label_a: la,
                label_b: lb,
                xs: report.xs,
                ys: report.ys,
                skipped_incomplete_rows: report.skipped_incomplete_rows,
                non_numeric_cells: report.non_numeric_cells,
                accounting: report.accounting,
            })
        }
        TableArchetype::Xy(xy) => {
            if xy.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            if xy.data_sets.is_empty() {
                return Err(StatsError::EmptyInput);
            }
            if ba.column_a == ba.column_b {
                return Err(StatsError::InvalidParameter { name: "column_b", value: ba.column_b as f64 });
            }
            let compare_x_vs_y = ba.xy_compare_x_vs_y.unwrap_or(xy.data_sets.len() == 1);
            if compare_x_vs_y {
                // X vs Y[0] mode: require canonical indices (0, 1) regardless of additional Y datasets.
                if ba.column_a != 0 {
                    return Err(StatsError::MissingColumn { index: ba.column_a });
                }
                if ba.column_b != 1 {
                    return Err(StatsError::MissingColumn { index: ba.column_b });
                }
                let ds = xy.data_sets.first().ok_or(StatsError::EmptyInput)?;
                let is_gen = matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. });
                let active_end = (0..xy.row_count()).rfind(|&r| {
                    let x_cell = xy.materialized_x_cell_at(r);
                    let x_active = if is_gen { x_cell.excluded } else { !x_cell.is_empty() || x_cell.excluded };
                    x_active || ds.cells.get(r).is_some_and(|subs| subs.iter().any(|c| !c.is_empty() || c.excluded))
                });
                let Some(end_row) = active_end else {
                    return Err(StatsError::EmptyInput);
                };
                let mut xs = Vec::new();
                let mut ys = Vec::new();
                let mut non_numeric_cells = 0_usize;
                let mut skipped_incomplete_rows = 0_usize;
                let mut accounting = curvebook_doc::PairedObservationAccounting::default();
                for r in 0..=end_row {
                    let x_cell = xy.materialized_x_cell_at(r);
                    if !is_gen && is_ignored_non_numeric_cell(&x_cell) {
                        non_numeric_cells += 1;
                    }
                    let x_opt = xy.x_summary_at(r)?;
                    let x_has_entry = is_gen || !x_cell.is_empty() || x_cell.excluded;
                    for cy in ds.cells.get(r).map(Vec::as_slice).unwrap_or(&[]) {
                        if is_ignored_non_numeric_cell(cy) {
                            non_numeric_cells += 1;
                        }
                        if let (Some(x), Some(y)) = (x_opt, cy.active_number()) {
                            xs.push(x);
                            ys.push(y);
                        } else if x_has_entry || !cy.is_empty() || cy.excluded {
                            skipped_incomplete_rows += 1;
                            crate::common::record_omitted_pair(&mut accounting, (!is_gen).then_some(&x_cell), Some(cy));
                        }
                    }
                }
                if xs.is_empty() {
                    return Err(StatsError::EmptyInput);
                }
                Ok(BlandAltmanExtractedPairs {
                    label_a: xy.x_title.clone(),
                    label_b: ds.title.clone(),
                    accounting: curvebook_doc::PairedObservationAccounting { pairs_analyzed: xs.len(), ..accounting },
                    xs,
                    ys,
                    skipped_incomplete_rows,
                    non_numeric_cells,
                })
            } else {
                let dsa = xy.data_sets.get(ba.column_a).ok_or(StatsError::MissingColumn { index: ba.column_a })?;
                let dsb = xy.data_sets.get(ba.column_b).ok_or(StatsError::MissingColumn { index: ba.column_b })?;
                if dsa.subcolumn_count() != dsb.subcolumn_count() {
                    return Err(StatsError::DimensionMismatch { expected: dsa.subcolumn_count(), actual: dsb.subcolumn_count() });
                }
                let mut xs = Vec::new();
                let mut ys = Vec::new();
                let mut non_numeric_cells = 0_usize;
                let mut skipped_incomplete_rows = 0_usize;
                let mut accounting = curvebook_doc::PairedObservationAccounting::default();
                for r in 0..xy.row_count() {
                    let sa = dsa.cells.get(r).map(Vec::as_slice).unwrap_or(&[]);
                    let sb = dsb.cells.get(r).map(Vec::as_slice).unwrap_or(&[]);
                    if sa.len() != sb.len() {
                        return Err(StatsError::DimensionMismatch { expected: sa.len(), actual: sb.len() });
                    }
                    for (ca, cb) in sa.iter().zip(sb.iter()) {
                        if is_ignored_non_numeric_cell(ca) {
                            non_numeric_cells += 1;
                        }
                        if is_ignored_non_numeric_cell(cb) {
                            non_numeric_cells += 1;
                        }
                        let has_entry = (!ca.is_empty() || ca.excluded) || (!cb.is_empty() || cb.excluded);
                        if let (Some(a), Some(b)) = (ca.active_number(), cb.active_number()) {
                            xs.push(a);
                            ys.push(b);
                        } else if has_entry {
                            skipped_incomplete_rows += 1;
                            crate::common::record_omitted_pair(&mut accounting, Some(ca), Some(cb));
                        }
                    }
                }
                if xs.is_empty() {
                    return Err(StatsError::EmptyInput);
                }
                Ok(BlandAltmanExtractedPairs {
                    label_a: dsa.title.clone(),
                    label_b: dsb.title.clone(),
                    accounting: curvebook_doc::PairedObservationAccounting { pairs_analyzed: xs.len(), ..accounting },
                    xs,
                    ys,
                    skipped_incomplete_rows,
                    non_numeric_cells,
                })
            }
        }
        other => Err(StatsError::IncompatibleTable { analysis: "Bland-Altman", actual: other.kind() }),
    }
}

struct MultipleRowInputs {
    label_a: String,
    label_b: String,
    row_labels: Vec<String>,
    rows_a: Vec<Vec<f64>>,
    rows_b: Vec<Vec<f64>>,
    skipped_incomplete_pairs: usize,
    kept_pairs: usize,
    ignored_non_numeric: Vec<(String, usize)>,
}

fn extract_dataset_pair_multiple_row_inputs<F>(
    data_sets: &[curvebook_doc::DataSetColumn],
    spec: &AnalysisSpec,
    paired: bool,
    n_rows: usize,
    row_label: F,
) -> Result<MultipleRowInputs, StatsError>
where
    F: Fn(usize) -> String,
{
    let indices = try_resolve_selected_indices(&spec.selected_data_sets, data_sets.len())?;
    if paired && indices.len() != 2 {
        return Err(StatsError::InvalidParameter { name: "paired_selected_columns", value: indices.len() as f64 });
    }
    if !spec.selected_data_sets.is_empty() && indices.len() != 2 {
        return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
    }
    if indices.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "multiple row t-tests groups", required: 2, actual: indices.len() });
    }
    let (ia, ib) = (indices[0], indices[1]);
    let dsa = &data_sets[ia];
    let dsb = &data_sets[ib];
    if paired && dsa.subcolumn_count() != dsb.subcolumn_count() {
        return Err(StatsError::DimensionMismatch { expected: dsa.subcolumn_count(), actual: dsb.subcolumn_count() });
    }
    let mut labels = Vec::with_capacity(n_rows);
    let mut rows_a = Vec::with_capacity(n_rows);
    let mut rows_b = Vec::with_capacity(n_rows);
    let mut non_numeric_a = 0_usize;
    let mut non_numeric_b = 0_usize;
    let mut skipped_incomplete_pairs = 0_usize;
    let mut kept_pairs = 0_usize;
    for r in 0..n_rows {
        let (ra, rb) = (dsa.cells.get(r), dsb.cells.get(r));
        if paired
            && let (Some(sa), Some(sb)) = (ra, rb)
            && sa.len() != sb.len()
        {
            return Err(StatsError::DimensionMismatch { expected: sa.len(), actual: sb.len() });
        }
        if let Some(sa) = ra {
            non_numeric_a += sa.iter().filter(|c| is_ignored_non_numeric_cell(c)).count();
        }
        if let Some(sb) = rb {
            non_numeric_b += sb.iter().filter(|c| is_ignored_non_numeric_cell(c)).count();
        }
        let (va, vb, skipped) = extract_paired_or_unpaired_subcolumns(ra, rb, paired);
        if paired {
            kept_pairs += va.len();
            skipped_incomplete_pairs += skipped;
        }
        labels.push(row_label(r));
        rows_a.push(va);
        rows_b.push(vb);
    }
    let mut ignored_non_numeric = Vec::new();
    if non_numeric_a > 0 {
        ignored_non_numeric.push((dsa.title.clone(), non_numeric_a));
    }
    if non_numeric_b > 0 {
        ignored_non_numeric.push((dsb.title.clone(), non_numeric_b));
    }
    Ok(MultipleRowInputs {
        label_a: dsa.title.clone(),
        label_b: dsb.title.clone(),
        row_labels: labels,
        rows_a,
        rows_b,
        skipped_incomplete_pairs,
        kept_pairs,
        ignored_non_numeric,
    })
}

fn extract_split_column_multiple_row_inputs<'a, L, C, T>(
    indices: &[usize],
    paired: bool,
    n_rows: usize,
    row_label: L,
    cell_at: C,
    col_title: T,
) -> MultipleRowInputs
where
    L: Fn(usize) -> String,
    C: Fn(usize, usize) -> Option<&'a Cell>,
    T: Fn(usize) -> String,
{
    let mid = indices.len() / 2;
    let (left, right) = (&indices[..mid], &indices[mid..]);
    let mut labels = Vec::with_capacity(n_rows);
    let mut rows_a = Vec::with_capacity(n_rows);
    let mut rows_b = Vec::with_capacity(n_rows);
    let mut non_numeric_counts = vec![0_usize; indices.len()];
    let mut skipped_incomplete_pairs = 0_usize;
    let mut kept_pairs = 0_usize;
    for r in 0..n_rows {
        let (va, vb) = if paired {
            let mut pa = Vec::new();
            let mut pb = Vec::new();
            for (pos, (&la, &rb_idx)) in left.iter().zip(right.iter()).enumerate() {
                let ca = cell_at(la, r);
                let cb = cell_at(rb_idx, r);
                if ca.is_some_and(is_ignored_non_numeric_cell) {
                    non_numeric_counts[pos] += 1;
                }
                if cb.is_some_and(is_ignored_non_numeric_cell) {
                    non_numeric_counts[mid + pos] += 1;
                }
                if let (Some(a), Some(b)) = (ca.and_then(Cell::active_number), cb.and_then(Cell::active_number)) {
                    pa.push(a);
                    pb.push(b);
                    kept_pairs += 1;
                } else if ca.is_some_and(|c| !c.is_empty() || c.excluded) || cb.is_some_and(|c| !c.is_empty() || c.excluded) {
                    skipped_incomplete_pairs += 1;
                }
            }
            (pa, pb)
        } else {
            let mut va = Vec::new();
            let mut vb = Vec::new();
            for (pos, &col_idx) in indices.iter().enumerate() {
                let cell = cell_at(col_idx, r);
                if cell.is_some_and(is_ignored_non_numeric_cell) {
                    non_numeric_counts[pos] += 1;
                }
                if let Some(v) = cell.and_then(Cell::active_number) {
                    if pos < mid {
                        va.push(v);
                    } else {
                        vb.push(v);
                    }
                }
            }
            (va, vb)
        };
        labels.push(row_label(r));
        rows_a.push(va);
        rows_b.push(vb);
    }
    let ignored_non_numeric =
        indices.iter().zip(non_numeric_counts).filter(|(_, count)| *count > 0).map(|(&col_idx, count)| (col_title(col_idx), count)).collect();
    MultipleRowInputs {
        label_a: String::from("Group A"),
        label_b: String::from("Group B"),
        row_labels: labels,
        rows_a,
        rows_b,
        skipped_incomplete_pairs,
        kept_pairs,
        ignored_non_numeric,
    }
}

fn extract_multiple_row_inputs(sheet: &DataSheet, spec: &AnalysisSpec, paired: bool) -> Result<MultipleRowInputs, StatsError> {
    match &sheet.archetype {
        TableArchetype::Grouped(grp) => {
            if grp.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            extract_dataset_pair_multiple_row_inputs(&grp.data_sets, spec, paired, grp.row_count(), |r| {
                grp.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1))
            })
        }
        TableArchetype::Xy(xy) => {
            if xy.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            extract_dataset_pair_multiple_row_inputs(&xy.data_sets, spec, paired, xy.row_count(), |r| {
                xy.x_value_at(r).map_or_else(|| format!("Row {}", r + 1), |x| format!("{x}"))
            })
        }
        TableArchetype::Column(col) => {
            if col.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let indices = try_resolve_selected_indices(&spec.selected_data_sets, col.data_sets.len())?;
            if paired && !indices.len().is_multiple_of(2) {
                return Err(StatsError::InvalidParameter { name: "paired_selected_columns", value: indices.len() as f64 });
            }
            if indices.len() < 4 {
                return Err(StatsError::InsufficientSampleSize {
                    context: "multiple row t-tests replicate columns per group",
                    required: 4,
                    actual: indices.len(),
                });
            }
            Ok(extract_split_column_multiple_row_inputs(
                &indices,
                paired,
                col.row_count(),
                |r| col.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1)),
                |c, r| col.data_sets[c].cell(r, 0),
                |c| col.data_sets[c].title.clone(),
            ))
        }
        TableArchetype::MultipleVariables(mv) => {
            let indices: Vec<usize> = try_resolve_selected_indices(&spec.selected_data_sets, mv.variables.len())?
                .into_iter()
                .filter(|&i| mv.variables.get(i).is_some_and(is_mv_numeric_response_candidate))
                .collect();
            if paired && !indices.len().is_multiple_of(2) {
                return Err(StatsError::InvalidParameter { name: "paired_selected_columns", value: indices.len() as f64 });
            }
            if indices.len() < 4 {
                return Err(StatsError::InsufficientSampleSize {
                    context: "multiple row t-tests numeric replicate columns",
                    required: 4,
                    actual: indices.len(),
                });
            }
            let label_col = mv
                .variables
                .iter()
                .position(|v| v.role == VariableRole::Label)
                .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Categorical));
            Ok(extract_split_column_multiple_row_inputs(
                &indices,
                paired,
                mv.row_count(),
                |r| label_col.and_then(|lc| mv.variables[lc].cells.get(r)).and_then(mv_cell_token).unwrap_or_else(|| format!("Row {}", r + 1)),
                |c, r| mv.variables[c].cells.get(r),
                |c| mv.variables[c].title.clone(),
            ))
        }
        other => Err(StatsError::IncompatibleTable { analysis: "Multiple row t-tests", actual: other.kind() }),
    }
}

fn extract_paired_or_unpaired_subcolumns(row_a: Option<&Vec<Cell>>, row_b: Option<&Vec<Cell>>, paired: bool) -> (Vec<f64>, Vec<f64>, usize) {
    match (row_a, row_b) {
        (Some(sa), Some(sb)) if paired => {
            let mut va = Vec::new();
            let mut vb = Vec::new();
            let mut skipped = 0_usize;
            for (ca, cb) in sa.iter().zip(sb.iter()) {
                if let (Some(a), Some(b)) = (ca.active_number(), cb.active_number()) {
                    va.push(a);
                    vb.push(b);
                } else if (!ca.is_empty() || ca.excluded) || (!cb.is_empty() || cb.excluded) {
                    skipped += 1;
                }
            }
            (va, vb, skipped)
        }
        (Some(sa), Some(sb)) => (sa.iter().filter_map(Cell::active_number).collect(), sb.iter().filter_map(Cell::active_number).collect(), 0),
        _ => (Vec::new(), Vec::new(), 0),
    }
}

type SingleRowOutcome = (f64, f64, f64, f64, f64, f64);

fn run_single_row_comparison(ra: &[f64], rb: &[f64], mr: &MultipleRowTestsSpec, conf: ConfidenceLevel) -> Result<SingleRowOutcome, StatsError> {
    if mr.paired && mr.nonparametric {
        let out = wilcoxon_matched_pairs(ra, rb, true, conf, true)?;
        let ma = SummaryMoments::from_slice(ra)?.mean;
        let mb = SummaryMoments::from_slice(rb)?.mean;
        return Ok((ma, mb, out.hodges_lehmann_pseudomedian, out.w_stat, f64::INFINITY, out.p_value));
    }
    if mr.paired {
        let out = paired_t_test(ra, rb, true, conf)?;
        let ma = SummaryMoments::from_slice(ra)?.mean;
        let mb = SummaryMoments::from_slice(rb)?.mean;
        return Ok((ma, mb, out.mean_of_differences, out.t_statistic, out.df, out.p_value));
    }
    if mr.nonparametric {
        let out = mann_whitney_u(ra, rb, true, conf)?;
        let ma = SummaryMoments::from_slice(ra)?.mean;
        let mb = SummaryMoments::from_slice(rb)?.mean;
        return Ok((ma, mb, out.hodges_lehmann_diff, out.u_min, f64::INFINITY, out.p_value));
    }
    let out = if mr.assume_equal_sd { unpaired_t_test(ra, rb, true, conf)? } else { welch_t_test(ra, rb, true, conf)? };
    Ok((out.mean_a, out.mean_b, out.difference, out.t_statistic, out.df, out.p_value))
}

#[allow(clippy::too_many_arguments)]
fn populate_unpaired_t_sheet(
    res: &mut ResultSheet,
    idx_a: usize,
    idx_b: usize,
    lbl_a: &str,
    lbl_b: &str,
    out: &UnpairedTResult,
    include_es: bool,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    populate_unpaired_t_sheet_impl(res, idx_a, idx_b, lbl_a, lbl_b, out, include_es, conf, p_fmt, true);
}

#[allow(clippy::too_many_arguments)]
fn populate_unpaired_t_sheet_impl(
    res: &mut ResultSheet,
    idx_a: usize,
    idx_b: usize,
    lbl_a: &str,
    lbl_b: &str,
    out: &UnpairedTResult,
    include_es: bool,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
    has_raw_replicates: bool,
) {
    let is_lognormal = out.geometric_mean_ratio.is_some();
    let alpha = conf.alpha();
    let pct = conf.percent();
    let suffix = if is_lognormal { " (ln scale)" } else { "" };
    let mut tab = KvTabBuilder::new("two-group", out.method_label, ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
        .p("P value", out.p_value, p_fmt, alpha)
        .num("t statistic", out.t_statistic)
        .num("df", out.df)
        .num(if is_lognormal { format!("Mean of ln({lbl_a})") } else { format!("Mean ({lbl_a})") }, out.mean_a)
        .num(if is_lognormal { format!("Mean of ln({lbl_b})") } else { format!("Mean ({lbl_b})") }, out.mean_b)
        .num(if is_lognormal { "Difference of ln means (ln A - ln B)" } else { "Difference between means (A - B)" }, out.difference)
        .num(if is_lognormal { "SE of ln difference" } else { "SE of difference" }, out.se_difference)
        .ci(if is_lognormal { format!("{pct}% CI of ln difference") } else { format!("{pct}% CI of difference") }, out.ci.0, out.ci.1)
        .num(if is_lognormal { "F-test to compare ln variances (F)" } else { "F-test to compare variances (F)" }, out.f_test.f_statistic)
        .p(format!("F-test P value{suffix}"), out.f_test.p_value, p_fmt, alpha);
    if let Some(gmr) = out.geometric_mean_ratio {
        tab.push_num("Geometric mean ratio (A / B)", gmr).push_opt_ci(format!("{pct}% CI of geometric ratio"), out.geometric_mean_ratio_ci);
    }
    if include_es {
        let std_grp = if is_lognormal { format!("ln({lbl_a})") } else { lbl_a.to_owned() };
        tab.push_num(format!("Cohen's d{suffix}"), out.effect_sizes.cohens_d)
            .push_opt_ci(format!("{pct}% CI of Cohen's d{suffix}"), out.effect_sizes.cohens_d_ci)
            .push_num(format!("Hedges' g{suffix}"), out.effect_sizes.hedges_g)
            .push_opt_ci(format!("{pct}% CI of Hedges' g{suffix}"), out.effect_sizes.hedges_g_ci)
            .push_opt_num(format!("Glass's Delta{suffix}"), out.effect_sizes.glass_delta)
            .push_text("Glass's Delta standardizing group", std_grp)
            .push_num(format!("R squared{suffix}"), out.effect_sizes.r_squared);
    }
    if is_lognormal {
        tab.push_footnote(
            "Lognormal unpaired t-test evaluates Student's t on ln(Y); log-scale means, differences, CIs, and effect sizes are on the natural-log scale alongside the back-transformed geometric mean ratio.",
        );
    }
    if include_es {
        if out.effect_sizes.glass_delta.is_none() {
            let msg = format!(
                "Glass's Delta standardizes by group \"{lbl_a}\" SD and is unavailable because \"{lbl_a}\" has zero SD (group \"{lbl_b}\" SD is not substituted)."
            );
            tab.push_footnote(msg.clone());
            res.checklist.push(checklist_item("Glass's Delta standardizing SD", ChecklistStatus::Info, Some(format!("SD({lbl_a}) = 0")), msg));
        } else {
            tab.push_footnote(format!("Glass's Delta standardizes by group \"{lbl_a}\" SD."));
        }
    }
    res.tabs.push(tab.build());
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "t", None, out.t_statistic);
    push_scalar(res, "CohensD", None, out.effect_sizes.cohens_d);
    push_scalar(res, "HedgesG", None, out.effect_sizes.hedges_g);
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.difference),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= alpha,
    });
    append_unpaired_checklist(&mut res.checklist, out, alpha, has_raw_replicates);
}

fn append_unpaired_checklist(checklist: &mut Vec<ChecklistItem>, out: &UnpairedTResult, alpha: f64, has_raw_replicates: bool) {
    if out.n_a < 5 || out.n_b < 5 {
        checklist.push(checklist_item(
            "Sample size adequacy",
            ChecklistStatus::Warn,
            Some(format!("n_A = {}, n_B = {}", out.n_a, out.n_b)),
            "Small group sample sizes (n < 5) limit statistical power and make normality and variance homogeneity tests inconclusive.",
        ));
    } else {
        checklist.push(checklist_item(
            "Sample size adequacy",
            ChecklistStatus::Pass,
            Some(format!("n_A = {}, n_B = {}", out.n_a, out.n_b)),
            "Both groups contain at least 5 observations.",
        ));
    }
    if out.method_label == "Unpaired Student t test" && out.f_test.p_value <= alpha {
        checklist.push(checklist_item(
            "Homogeneity of variances",
            ChecklistStatus::Violated,
            Some(format!("F = {:.3}, p = {:.4}", out.f_test.f_statistic, out.f_test.p_value)),
            "F-test indicates unequal group variances (p <= alpha), violating the equal-variance assumption of pooled-variance Student t tests.",
        ));
    } else if has_raw_replicates && let Some(ref levene) = out.levene_median {
        checklist.push(checklist_item(
            "Variance homogeneity check",
            if out.method_label == "Welch's t test" && levene.p_value <= alpha { ChecklistStatus::Info } else { ChecklistStatus::Pass },
            Some(format!("Levene(median) F = {:.3}, p = {:.4}", levene.f_statistic, levene.p_value)),
            if out.method_label == "Welch's t test" && levene.p_value <= alpha {
                "Levene test indicates heterogeneous variances; Welch's t-test does not assume equal variances."
            } else {
                "No severe departure from equal group variances was detected."
            },
        ));
    } else {
        checklist.push(checklist_item(
            "Variance homogeneity check",
            if out.f_test.p_value <= alpha { ChecklistStatus::Info } else { ChecklistStatus::Pass },
            Some(format!("F = {:.3}, p = {:.4}", out.f_test.f_statistic, out.f_test.p_value)),
            if out.f_test.p_value <= alpha {
                "Variance F-test indicates unequal group variances (p <= alpha); Welch's t-test does not assume equal variances."
            } else {
                "Variance F-test shows no significant departure from equal group variances."
            },
        ));
    }
    if !has_raw_replicates {
        checklist.push(checklist_item(
            "Pre-summarized input mode",
            ChecklistStatus::Info,
            None,
            "Moments were reconstructed from entered Mean, Spread, and N; normality and Levene median tests require raw replicates.",
        ));
    }
}

fn append_unpaired_normality_checklist(checklist: &mut Vec<ChecklistItem>, vals_a: &[f64], vals_b: &[f64], is_lognormal: bool, alpha: f64) {
    let prep =
        |slice: &[f64]| -> Vec<f64> { if is_lognormal { slice.iter().copied().filter(|&x| x > 0.0).map(f64::ln).collect() } else { slice.to_vec() } };
    let a = prep(vals_a);
    let b = prep(vals_b);
    let sw_a = shapiro_wilk(&a, alpha).ok();
    let sw_b = shapiro_wilk(&b, alpha).ok();
    if sw_a.is_none() && sw_b.is_none() {
        return;
    }
    let min_sw = [sw_a.as_ref(), sw_b.as_ref()].into_iter().flatten().min_by(|x, y| x.p_value.total_cmp(&y.p_value));
    if let Some(worst) = min_sw {
        let violated = !worst.passed;
        checklist.push(checklist_item(
            if is_lognormal { "Lognormality of group values" } else { "Normality of group values" },
            if violated { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            Some(format!("min Shapiro-Wilk W = {:.4}, p = {:.4}", worst.statistic, worst.p_value)),
            if violated {
                "Shapiro-Wilk test indicates at least one group departs significantly from Gaussian normality."
            } else {
                "Neither group shows a significant departure from Gaussian normality."
            },
        ));
    }
}

fn append_paired_normality_checklist(checklist: &mut Vec<ChecklistItem>, vals_a: &[f64], vals_b: &[f64], is_ratio: bool, alpha: f64) {
    let diffs: Vec<f64> = vals_a
        .iter()
        .zip(vals_b.iter())
        .filter_map(|(&a, &b)| if is_ratio { (a > 0.0 && b > 0.0).then(|| (a / b).ln()) } else { Some(a - b) })
        .collect();
    if diffs.len() < 5 {
        checklist.push(checklist_item(
            "Normality of paired differences",
            ChecklistStatus::Warn,
            Some(format!("n = {} pairs", diffs.len())),
            format!(
                "Sample size (n = {} pairs) is too small to assess normality of paired differences reliably; assumption test is inconclusive.",
                diffs.len()
            ),
        ));
    } else if let Ok(sw) = shapiro_wilk(&diffs, alpha) {
        let violated = !sw.passed;
        checklist.push(checklist_item(
            "Normality of paired differences",
            if violated { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            Some(format!("Shapiro-Wilk W = {:.4}, p = {:.4}", sw.statistic, sw.p_value)),
            if violated {
                "Shapiro-Wilk test indicates paired differences depart significantly from Gaussian normality."
            } else {
                "Paired differences do not depart significantly from Gaussian normality."
            },
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn populate_paired_t_sheet(
    res: &mut ResultSheet,
    idx_a: usize,
    idx_b: usize,
    lbl_a: &str,
    lbl_b: &str,
    out: &PairedTResult,
    include_es: bool,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    let alpha = conf.alpha();
    let pct = conf.percent();
    let mut tab = KvTabBuilder::new("paired-t", out.method_label, ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
        .p("P value", out.p_value, p_fmt, alpha)
        .num("t statistic", out.t_statistic)
        .num("df", out.df)
        .num("Number of pairs", out.n_pairs as f64)
        .num("Mean of differences (A - B)", out.mean_of_differences)
        .num("SD of differences", out.sd_of_differences)
        .ci(format!("{pct}% CI of difference"), out.ci.0, out.ci.1)
        .opt_num("Pairing correlation (r)", out.pairing_r)
        .cell("Pairing effectiveness P", out.pairing_p.map_or(ResultCell::Empty, |p| p_cell_at_alpha(p, p_fmt, alpha)));
    if let Some(gmr) = out.geometric_mean_ratio {
        tab.push_num("Geometric mean of ratios (A / B)", gmr).push_opt_ci(format!("{pct}% CI of ratio"), out.geometric_mean_ratio_ci);
    }
    if include_es {
        tab.push_num("Cohen's dz", out.cohens_dz)
            .push_opt_num("Hedges' gz", out.hedges_gz)
            .push_opt_ci(format!("{pct}% CI of Hedges' gz"), out.hedges_gz_ci);
    }
    res.tabs.push(tab.build());
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "t", None, out.t_statistic);
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.mean_of_differences),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= conf.alpha(),
    });
    let pairing_status = match (out.pairing_r, out.pairing_p) {
        (Some(r), Some(p)) if r > 0.0 && p <= conf.alpha() => ChecklistStatus::Pass,
        (Some(_), Some(_)) => ChecklistStatus::Warn,
        _ => ChecklistStatus::Info,
    };
    res.checklist.push(checklist_item(
        "Pairing effectiveness",
        pairing_status,
        out.pairing_r.map(|r| format!("r = {r:.4}")),
        "Evaluates two-sided Pearson correlation r; positive significant covariance reduces paired-difference variance.",
    ));
}

fn format_hl_ci_row_and_warn(
    tab: &mut KvTabBuilder,
    checklist: &mut Vec<ChecklistItem>,
    target_desc: &str,
    ci: Option<(f64, f64)>,
    actual_conf_pct: Option<f64>,
    max_bound_pct: Option<f64>,
    conf: ConfidenceLevel,
) -> Option<String> {
    let nominal = conf.percent();
    let unattainable =
        actual_conf_pct.is_some_and(|a| a + 1e-9 < nominal) || (actual_conf_pct.is_none() && max_bound_pct.is_some_and(|m| m + 1e-9 < nominal));
    let ci_label = match actual_conf_pct {
        Some(actual) if actual + 1e-9 < nominal => format!("{actual:.2}% CI of {target_desc} (requested {nominal}%)"),
        _ => format!("{nominal}% CI of {target_desc}"),
    };
    tab.push_opt_ci(ci_label, ci).push_opt_num("HL CI actual confidence (%)", actual_conf_pct);

    if unattainable {
        let (metric, msg) = if let Some(actual) = actual_conf_pct {
            (
                Some(format!("achieved = {actual:.2}% (requested {nominal}%)")),
                format!(
                    "Requested {nominal}% Hodges-Lehmann confidence level is unattainable at this sample size; exact achieved coverage is {actual:.2}%."
                ),
            )
        } else if let Some(bound) = max_bound_pct {
            (
                Some(format!("max achievable <= {bound:.2}% (requested {nominal}%)")),
                format!(
                    "Requested {nominal}% Hodges-Lehmann confidence level is unattainable at this sample size (maximum distribution-free coverage <= {bound:.2}%)."
                ),
            )
        } else {
            (None, format!("Requested {nominal}% Hodges-Lehmann confidence level is unattainable at this sample size."))
        };
        checklist.push(checklist_item("Hodges-Lehmann CI achieved coverage", ChecklistStatus::Warn, metric, msg.clone()));
        Some(msg)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn populate_mann_whitney_sheet(
    res: &mut ResultSheet,
    idx_a: usize,
    idx_b: usize,
    lbl_a: &str,
    lbl_b: &str,
    out: &MannWhitneyResult,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    let alpha = conf.alpha();
    let mut tab = KvTabBuilder::new("mann-whitney", "Mann-Whitney test", ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
        .p("P value", out.p_value, p_fmt, alpha)
        .boolean("Exact P used", out.exact_p_used)
        .num("Mann-Whitney U", out.u_min)
        .num("Hodges-Lehmann median diff.", out.hodges_lehmann_diff);
    let hl_warn = format_hl_ci_row_and_warn(
        &mut tab,
        &mut res.checklist,
        "median diff.",
        out.hodges_lehmann_ci,
        out.ci_actual_confidence_percent,
        mann_whitney_max_achievable_coverage_percent(out.n_a, out.n_b),
        conf,
    );
    tab.push_num("Cliff's delta", out.cliffs_delta).push_num("Rank-biserial r", out.rank_biserial_r);
    if let Some(msg) = hl_warn {
        tab.push_footnote(msg);
    }
    res.tabs.push(tab.build());
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "MannWhitneyU", None, out.u_min);
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.hodges_lehmann_diff),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= alpha,
    });
    res.checklist.push(checklist_item(
        "Nonparametric rank comparison",
        ChecklistStatus::Pass,
        Some(format!("n_A = {}, n_B = {}, exact P = {}", out.n_a, out.n_b, out.exact_p_used)),
        if out.exact_p_used {
            "Exact permutation DP computed under the conditional null rank distribution; does not assume Gaussian normality."
        } else {
            "Asymptotic continuity-corrected tie-adjusted normal approximation used; does not assume Gaussian normality."
        },
    ));
}

#[allow(clippy::too_many_arguments)]
fn populate_wilcoxon_paired_sheet(
    res: &mut ResultSheet,
    idx_a: usize,
    idx_b: usize,
    lbl_a: &str,
    lbl_b: &str,
    out: &WilcoxonMatchedResult,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    let alpha = conf.alpha();
    let mut tab = KvTabBuilder::new("wilcoxon-paired", "Wilcoxon matched-pairs test", ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
        .p("P value", out.p_value, p_fmt, alpha)
        .boolean("Exact P used", out.exact_p_used)
        .num("Sum of positive ranks (W+)", out.w_plus)
        .num("Sum of negative ranks (W-)", out.w_minus)
        .num("Hodges-Lehmann pseudomedian", out.hodges_lehmann_pseudomedian);
    let hl_warn = format_hl_ci_row_and_warn(
        &mut tab,
        &mut res.checklist,
        "pseudomedian",
        out.hodges_lehmann_ci,
        out.ci_actual_confidence_percent,
        wilcoxon_max_achievable_coverage_percent(out.n_pairs),
        conf,
    );
    tab.push_num("Rank-biserial r", out.rank_biserial_r);
    if let Some(msg) = hl_warn {
        tab.push_footnote(msg);
    }
    tab.push_footnote(
        "Zero differences follow Pratt's (1959) method: all pairs (including exact zeros) are ranked by |d|, zero-difference ranks are excluded from W+ and W-, and exact small-sample P values are used when no zero differences or ties occur (n <= 25).",
    );
    res.tabs.push(tab.build());
    push_scalar(res, "PValue", None, out.p_value);
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.hodges_lehmann_pseudomedian),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= alpha,
    });
    res.checklist.push(checklist_item(
        "Symmetry of paired differences",
        ChecklistStatus::Info,
        Some(format!("effective pairs = {} (Pratt zero handling)", out.n_effective)),
        "Wilcoxon signed-rank assumes paired differences are symmetrically distributed around the median; exact zero differences use Pratt (1959) zero-inclusion ranking.",
    ));
}

fn populate_ks_sheet(
    res: &mut ResultSheet,
    indices: (usize, usize),
    labels: (&str, &str),
    out: &TwoSampleKsResult,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    let (idx_a, idx_b) = indices;
    let (lbl_a, lbl_b) = labels;
    let alpha = conf.alpha();
    res.tabs.push(
        KvTabBuilder::new("ks-two-sample", "Kolmogorov-Smirnov test", ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
            .p("P value", out.p_value, p_fmt, alpha)
            .num("Kolmogorov-Smirnov D", out.d_statistic)
            .boolean("Exact P used", out.exact_p_used)
            .build(),
    );
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "KSD", None, out.d_statistic);
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: None,
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= conf.alpha(),
    });
    res.checklist.push(checklist_item(
        "Two-sample empirical CDF test",
        ChecklistStatus::Pass,
        Some(format!("D = {:.4}", out.d_statistic)),
        "Sensitive to differences in location, spread, and distributional shape.",
    ));
}

fn populate_one_sample_t_sheet_impl(
    res: &mut ResultSheet,
    label: &str,
    values: Option<&[f64]>,
    out: &OneSampleTResult,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    res.tabs.push(
        KvTabBuilder::new("one-sample-t", "One-sample t test", ResultTabKind::MainSummary, label)
            .num("Hypothetical mean", out.hypothetical_mean)
            .num("Sample mean", out.sample_mean)
            .num("Discrepancy", out.discrepancy)
            .num("t statistic", out.t_statistic)
            .num("df", out.df)
            .p("P value", out.p_value, p_fmt, conf.alpha())
            .ci(format!("{}% CI of discrepancy", conf.percent()), out.ci.0, out.ci.1)
            .opt_num("Hedges' g", out.hedges_g)
            .build(),
    );
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "t", None, out.t_statistic);

    if let Some(raw_vals) = values {
        let (status, metric, explanation) = if out.n < 5 {
            (
                ChecklistStatus::Warn,
                Some(format!("n = {}", out.n)),
                format!("Sample size (n = {}) is too small to assess Gaussian normality reliably; assumption test is inconclusive.", out.n),
            )
        } else if let Ok(sw) = shapiro_wilk(raw_vals, conf.alpha())
            && !sw.passed
        {
            (
                ChecklistStatus::Warn,
                Some(format!("n = {}, Shapiro-Wilk W = {:.4}, p = {:.4}", out.n, sw.statistic, sw.p_value)),
                String::from("Shapiro-Wilk test indicates significant departure from Gaussian normality."),
            )
        } else {
            (
                ChecklistStatus::Pass,
                Some(format!("n = {}", out.n)),
                String::from("One-sample t test assumes independent observations from an approximately Gaussian population."),
            )
        };
        res.checklist.push(checklist_item("One-sample Gaussian assumption", status, metric, explanation));
    } else {
        res.checklist.push(checklist_item(
            "Pre-summarized input mode",
            ChecklistStatus::Info,
            Some(format!("n = {}", out.n)),
            "Moments were reconstructed from entered Mean, Spread, and N; Gaussian normality tests require raw replicates.",
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn populate_paired_sign_sheet(
    res: &mut ResultSheet,
    idx_a: usize,
    idx_b: usize,
    lbl_a: &str,
    lbl_b: &str,
    out: &PairedSignTestResult,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    let pair_hdr = format!("{lbl_a} vs. {lbl_b}");
    res.tabs.push(
        KvTabBuilder::new("paired-sign-test", "Paired sign test", ResultTabKind::MainSummary, pair_hdr.clone())
            .p("P value", out.p_value, p_fmt, conf.alpha())
            .num("Median difference (A - B)", out.median_difference)
            .opt_ci(format!("{}% CI of median difference", conf.percent()), out.ci)
            .opt_num("Median CI actual confidence (%)", out.ci_actual_confidence_percent)
            .num("Positive differences (n+)", out.n_positive as f64)
            .num("Negative differences (n-)", out.n_negative as f64)
            .num("Zero ties (n0)", out.n_ties as f64)
            .num("Effective pairs (n+ + n-)", out.n_nonzero as f64)
            .build(),
    );

    let pos_prop = if out.n_nonzero > 0 { Some((out.n_positive as f64) / (out.n_nonzero as f64)) } else { None };
    res.tabs.push(
        KvTabBuilder::new("paired-sign-diagnostics", "Paired sign test diagnostics", ResultTabKind::Diagnostics, pair_hdr)
            .num("Total matched pairs", out.n_total as f64)
            .num("Effective non-tie pairs", out.n_nonzero as f64)
            .num("Dropped exact zero ties", out.n_ties as f64)
            .opt_num("Positive sign proportion", pos_prop)
            .num("Null binomial probability (p0)", 0.5)
            .build(),
    );

    push_scalar(res, "median_difference", None, out.median_difference);
    push_scalar(res, "p_value", None, out.p_value);
    push_scalar(res, "n_positive", None, out.n_positive as f64);
    push_scalar(res, "n_negative", None, out.n_negative as f64);
    push_scalar(res, "n_ties", None, out.n_ties as f64);

    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.median_difference),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= conf.alpha(),
    });

    let status = if out.n_nonzero < 6 || out.n_ties > 0 { ChecklistStatus::Info } else { ChecklistStatus::Pass };
    res.checklist.push(checklist_item(
        "Paired sign test assumptions",
        status,
        Some(format!("n+ = {}, n- = {}, ties = {}", out.n_positive, out.n_negative, out.n_ties)),
        "Exact binomial sign test evaluates sign concordance without assuming normality or symmetry; exact zero ties are excluded from the binomial denominator.",
    ));
}

fn populate_one_sample_w_sheet(
    res: &mut ResultSheet,
    label: &str,
    out: &crate::column::OneSampleWilcoxonResult,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    let mut tab = KvTabBuilder::new("one-sample-w", "One-sample Wilcoxon test", ResultTabKind::MainSummary, label)
        .num("Hypothetical median", out.hypothetical_median)
        .num("Sample median", out.sample_median)
        .num("W statistic (W+ - W-)", out.w_stat)
        .p("P value", out.p_value, p_fmt, conf.alpha())
        .num("Hodges-Lehmann pseudomedian", out.pseudomedian_hl);
    let hl_warn = format_hl_ci_row_and_warn(
        &mut tab,
        &mut res.checklist,
        "pseudomedian",
        out.pseudomedian_ci,
        out.ci_actual_confidence_percent,
        wilcoxon_max_achievable_coverage_percent(out.n_total),
        conf,
    );
    if let Some(msg) = hl_warn {
        tab.push_footnote(msg);
    }
    res.tabs.push(tab.build());
    push_scalar(res, "PValue", None, out.p_value);
    res.checklist.push(checklist_item(
        "One-sample symmetry",
        ChecklistStatus::Info,
        Some(format!("n_nonzero = {}", out.n_nonzero)),
        "One-sample Wilcoxon test assumes observations are symmetrically distributed around the population median.",
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use curvebook_doc::{AnalysisKind, Cell, DataSheet, MultipleVariablesTable, SheetId, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole};

    fn run_mv_tg(mv: MultipleVariablesTable, tg: TwoGroupSpec) -> Result<ResultSheet, StatsError> {
        let sheet = DataSheet::new(SheetId::new("mv-tg"), "MV TwoGroup", TableArchetype::MultipleVariables(mv)).unwrap();
        let spec = AnalysisSpec::new(SheetId::new("res-tg"), sheet.id.clone(), AnalysisKind::TwoGroupComparison(tg)).unwrap();
        run_two_group_analysis(&sheet, &spec, &tg)
    }

    fn scalar_val(res: &ResultSheet, key: &str) -> f64 {
        res.scalars.iter().find(|s| s.key == key).map(|s| s.value).unwrap()
    }

    #[test]
    fn mv_two_group_excludes_label_columns_and_resolves_single_response() {
        let mut mv = MultipleVariablesTable::new(3, 8).unwrap();
        mv.variables[0].title = String::from("SampleID");
        mv.variables[0].lock_role(VariableRole::Label);
        mv.variables[1].title = String::from("Treatment");
        mv.variables[1].lock_role(VariableRole::Categorical);
        mv.variables[2].title = String::from("Biomarker");
        mv.variables[2].lock_role(VariableRole::Continuous);

        let g_a = [18.5, 20.2, 21.0, 22.4];
        let g_b = [31.0, 34.5, 42.0, 38.2];
        for (i, &v) in g_a.iter().enumerate() {
            mv.variables[0].cells[i] = Cell::text(format!("V{}", i + 1));
            mv.variables[1].cells[i] = Cell::text("Vehicle");
            mv.variables[2].cells[i] = Cell::number(v);
        }
        for (i, &v) in g_b.iter().enumerate() {
            mv.variables[0].cells[4 + i] = Cell::text(format!("T{}", i + 1));
            mv.variables[1].cells[4 + i] = Cell::text("Treated");
            mv.variables[2].cells[4 + i] = Cell::number(v);
        }
        let expected = welch_t_test(&g_a, &g_b, true, ConfidenceLevel::DEFAULT_95).unwrap();

        for mv_group_column in [None, Some(1)] {
            let res = run_mv_tg(
                mv.clone(),
                TwoGroupSpec { method: TwoGroupMethod::WelchT, mv_group_column, mv_response_column: None, ..TwoGroupSpec::default() },
            )
            .unwrap();
            assert!((scalar_val(&res, "PValue") - expected.p_value).abs() < 1e-12);
            assert!((scalar_val(&res, "t") - expected.t_statistic).abs() < 1e-12);
        }

        // Numeric Label IDs locked to VariableRole::Label must still be excluded.
        let mut mv_num_lbl = mv;
        for i in 0..8 {
            mv_num_lbl.variables[0].cells[i] = Cell::number((101 + i) as f64);
        }
        let res_num_lbl = run_mv_tg(
            mv_num_lbl,
            TwoGroupSpec { method: TwoGroupMethod::WelchT, mv_group_column: None, mv_response_column: None, ..TwoGroupSpec::default() },
        )
        .unwrap();
        assert!((scalar_val(&res_num_lbl, "PValue") - expected.p_value).abs() < 1e-12);
    }

    #[test]
    fn mv_two_group_derived_calculated_response_fallback_picks_last_active_column() {
        let mut mv = MultipleVariablesTable::new(7, 8).unwrap();
        mv.variables[0].title = String::from("SampleID");
        mv.variables[0].lock_role(VariableRole::Label);
        mv.variables[1].title = String::from("Condition");
        mv.variables[1].lock_role(VariableRole::Categorical);
        mv.variables[2].title = String::from("TargetCt");
        mv.variables[2].lock_role(VariableRole::Continuous);
        mv.variables[3].title = String::from("RefCt");
        mv.variables[3].lock_role(VariableRole::Continuous);
        mv.variables[4].title = String::from("DeltaCt");
        mv.variables[4].set_calculated_formula("[TargetCt] - [RefCt]");
        mv.variables[5].title = String::from("Log2FC");
        mv.variables[5].set_calculated_formula("-([DeltaCt] - 4.0)");
        mv.variables[6].title = String::from("TrailingEmpty");
        mv.variables[6].lock_role(VariableRole::Continuous);

        let ctrl_log2fc = [0.0, -0.1, 0.0, 0.1];
        let pert_log2fc = [3.0, 3.1, 3.0, 2.9];
        for r in 0..8 {
            let is_ctrl = r < 4;
            let idx = r % 4;
            let l2fc = if is_ctrl { ctrl_log2fc[idx] } else { pert_log2fc[idx] };
            mv.variables[0].cells[r] = Cell::text(format!("S{}", r + 1));
            mv.variables[1].cells[r] = Cell::text(if is_ctrl { "Control" } else { "Perturbed" });
            mv.variables[2].cells[r] = Cell::number(if is_ctrl { 22.0 } else { 19.0 });
            mv.variables[3].cells[r] = Cell::number(18.0);
            mv.variables[4].cells[r] = Cell::number(4.0 - l2fc);
            mv.variables[5].cells[r] = Cell::number(l2fc);
        }
        let expected = welch_t_test(&ctrl_log2fc, &pert_log2fc, true, ConfidenceLevel::DEFAULT_95).unwrap();

        for mv_group_column in [None, Some(1)] {
            let res = run_mv_tg(
                mv.clone(),
                TwoGroupSpec { method: TwoGroupMethod::WelchT, mv_group_column, mv_response_column: None, ..TwoGroupSpec::default() },
            )
            .unwrap();
            assert!((scalar_val(&res, "PValue") - expected.p_value).abs() < 1e-12);
            assert!((scalar_val(&res, "t") - expected.t_statistic).abs() < 1e-12);
        }
    }

    #[test]
    fn mv_two_group_continuous_fallback_picks_last_active_non_empty_column() {
        let mut mv = MultipleVariablesTable::new(5, 6).unwrap();
        mv.variables[0].title = String::from("SampleID");
        mv.variables[0].lock_role(VariableRole::Label);
        mv.variables[1].title = String::from("Arm");
        mv.variables[1].lock_role(VariableRole::Categorical);
        mv.variables[2].title = String::from("Baseline");
        mv.variables[3].title = String::from("Outcome");
        mv.variables[4].title = String::from("EmptyCol");

        let g_a = [10.0, 11.0, 12.0];
        let g_b = [20.0, 22.0, 24.0];
        for i in 0..3 {
            mv.variables[0].cells[i] = Cell::text(format!("A{i}"));
            mv.variables[1].cells[i] = Cell::text("Placebo");
            mv.variables[2].cells[i] = Cell::number(1.0);
            mv.variables[3].cells[i] = Cell::number(g_a[i]);

            mv.variables[0].cells[3 + i] = Cell::text(format!("B{i}"));
            mv.variables[1].cells[3 + i] = Cell::text("Active");
            mv.variables[2].cells[3 + i] = Cell::number(1.0);
            mv.variables[3].cells[3 + i] = Cell::number(g_b[i]);
        }
        let expected = unpaired_t_test(&g_a, &g_b, true, ConfidenceLevel::DEFAULT_95).unwrap();
        let res = run_mv_tg(mv, TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }).unwrap();
        assert!((scalar_val(&res, "PValue") - expected.p_value).abs() < 1e-12);
        assert!((scalar_val(&res, "t") - expected.t_statistic).abs() < 1e-12);
    }
}
