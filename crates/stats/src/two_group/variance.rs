//! Variance homogeneity tests (*F*-test, Levene, and Brown–Forsythe).
//!
//! # References
//!
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall. Ch. 8 (two-sample
//!   variance-ratio *F* test).
//! - Levene, H. (1960). Robust tests for equality of variances. In I. Olkin et al. (Eds.),
//!   *Contributions to Probability and Statistics: Essays in Honor of Harold Hotelling*, pp. 278–292.
//!   Stanford University Press.
//! - Brown, M. B., & Forsythe, A. B. (1974). Robust tests for the equality of variances.
//!   *Journal of the American Statistical Association* 69(346):364–367. doi:10.1080/01621459.1974.10482955

use curvebook_math::dist::f_sf;
use curvebook_math::sum::{NeumaierSum, SummaryMoments};
use serde::{Deserialize, Serialize};

use crate::common::{quantile_type6_sorted, sorted_finite, validate_finite_slice};
use crate::error::StatsError;

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

/// Variance-ratio *F* test (Zar 2010, ch. 8) from summary SDs; same convention as [`f_test_variances`].
pub(super) fn f_test_from_summary(sd_a: f64, n_a: f64, sd_b: f64, n_b: f64) -> Result<VarianceFTestResult, StatsError> {
    let df_a = (n_a - 1.0).max(1.0);
    let df_b = (n_b - 1.0).max(1.0);
    if sd_a == 0.0 && sd_b == 0.0 {
        return Ok(VarianceFTestResult { f_statistic: 1.0, df_num: df_a, df_den: df_b, p_value: 1.0 });
    }
    let (f_stat, df_n, df_d) = if sd_a >= sd_b {
        let f = if sd_b > 0.0 {
            let ratio = sd_a / sd_b;
            ratio * ratio
        } else {
            f64::INFINITY
        };
        (f, df_a, df_b)
    } else {
        let f = if sd_a > 0.0 {
            let ratio = sd_b / sd_a;
            ratio * ratio
        } else {
            f64::INFINITY
        };
        (f, df_b, df_a)
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
