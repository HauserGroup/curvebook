//! Parametric two-group and one-sample *t*-tests (unpaired Student, Welch, lognormal, paired,
//! ratio-paired, and pre-summarized) together with standardized effect sizes and exact
//! noncentral-*t* confidence intervals.
//!
//! # References
//!
//! - Student (1908). The probable error of a mean. *Biometrika* 6(1):1–25. doi:10.1093/biomet/6.1.1
//! - Welch, B. L. (1947). The generalization of 'Student's' problem when several different population
//!   variances are involved. *Biometrika* 34(1–2):28–35. doi:10.1093/biomet/34.1-2.28
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114. doi:10.2307/3002019
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall. Ch. 7 (one-sample),
//!   ch. 8 (two-sample), ch. 9 (paired-sample hypotheses).
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

use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{student_t_cdf, student_t_sf, student_t_two_sided_p};
use curvebook_math::noncentral_t_ci;
use curvebook_math::sum::{SummaryMoments, pearson_r};
use serde::{Deserialize, Serialize};

use super::variance::{LeveneTestResult, VarianceFTestResult, f_test_from_summary, f_test_variances, levene_variance_test};
use crate::column::{OneSampleTResult, hedges_g_with_ci, hedges_j_factor};
use crate::common::{t_crit_two_sided, validate_finite_slice};
use crate::error::StatsError;

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
