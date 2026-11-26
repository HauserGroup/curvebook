//! ANOVA effect sizes with exact noncentral-*F* confidence intervals, variance-homogeneity tests,
//! and the shared sum-of-squares round-off and *F*-ratio helpers.
//!
//! References:
//! - Steiger, J. H. (2004). Beyond the F test: effect size confidence intervals and tests of close
//!   fit in the analysis of variance and contrast analysis. *Psychological Methods* 9(2):164–182.
//!   doi:10.1037/1082-989X.9.2.164
//! - Olejnik, S., & Algina, J. (2003). Generalized eta and omega squared statistics: measures of
//!   effect size for some common research designs. *Psychological Methods* 8(4):434–447.
//!   doi:10.1037/1082-989X.8.4.434
//! - Cohen, J. (1988). *Statistical Power Analysis for the Behavioral Sciences* (2nd ed.).
//!   Lawrence Erlbaum.
//! - Bartlett, M. S. (1937). Properties of sufficiency and statistical tests. *Proceedings of the
//!   Royal Society of London A* 160(901):268–282. doi:10.1098/rspa.1937.0109
//! - Brown, M. B., & Forsythe, A. B. (1974a). Robust tests for the equality of variances. *Journal
//!   of the American Statistical Association* 69(346):364–367. doi:10.1080/01621459.1974.10482955

use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{chi2_sf, f_sf, noncentral_f_ci};
use curvebook_math::sum::{NeumaierSum, SummaryMoments};

use crate::anova::one_way::ordinary_one_way_anova_slices;
use crate::anova::results::{AnovaEffectSizes, VarianceHomogeneityResult};
use crate::common::{quantile_type6_sorted, sorted_finite, validate_finite_slice};
use crate::error::StatsError;

pub(in crate::anova) fn clamp_ss_roundoff(ss: f64, data_max_abs: f64, obs_count: usize) -> f64 {
    let ss_clean = ss.max(0.0);
    if data_max_abs == 0.0 || !data_max_abs.is_finite() {
        return ss_clean;
    }
    let roundoff_amp = 64.0 * f64::EPSILON * data_max_abs;
    let tol = roundoff_amp * roundoff_amp * (obs_count.max(1) as f64);
    if ss_clean <= tol { 0.0 } else { ss_clean }
}

pub(in crate::anova) fn f_ratio_p(ms_num: f64, ms_den: f64, df1: f64, df2: f64) -> Result<(f64, f64), StatsError> {
    if ms_den > 0.0 {
        let f = ms_num / ms_den;
        Ok((f, f_sf(f, df1, df2)?))
    } else if ms_num > 0.0 {
        Ok((f64::INFINITY, 0.0))
    } else {
        Ok((0.0, 1.0))
    }
}

/// Compute ANOVA effect sizes (`eta^2`, `partial eta^2`, `omega^2`, Cohen's `f`) and exact
/// noncentral-*F* confidence intervals for one-way designs.
///
/// - `omega^2 = (SS_effect - df_effect * MS_error) / (SS_total + MS_error)` (Hays estimator per
///   Olejnik & Algina, 2003).
/// - Cohen's `f = sqrt(eta_p^2 / (1 - eta_p^2))` (Cohen, 1988, ch. 8; returns `+inf` when
///   `eta_p^2 = 1.0`).
/// - For one-way designs (`SS_effect + SS_error == SS_total` and `df_effect + df_error + 1 == N`),
///   `N = df_effect + df_error + 1` is exact and `eta_squared == partial_eta_squared`, so Steiger
///   (2004) noncentral-*F* intervals are computed automatically. Multi-way callers **must** call
///   [`compute_anova_effect_sizes_for_n`] with the true `total_n` instead (because omitted factor
///   terms with zero SS can satisfy `SS_effect + SS_error == SS_total` even though
///   `df_effect + df_error + 1 != total_n`, and this 6-argument helper cannot verify `total_n`).
pub fn compute_anova_effect_sizes(
    ss_effect: f64,
    ss_error: f64,
    ss_total: f64,
    df_effect: f64,
    df_error: f64,
    confidence: ConfidenceLevel,
) -> AnovaEffectSizes {
    let is_one_way = ((ss_effect + ss_error) - ss_total).abs() <= 1e-9 * ss_total.max(1e-30);
    if is_one_way {
        let total_n = (df_effect + df_error + 1.0).round().max(1.0) as usize;
        return compute_anova_effect_sizes_for_n(ss_effect, ss_error, ss_total, df_effect, df_error, total_n, confidence);
    }

    let ms_error = if df_error > 0.0 { ss_error / df_error } else { 0.0 };
    let eta2 = if ss_total > 0.0 { (ss_effect / ss_total).clamp(0.0, 1.0) } else { 0.0 };
    let partial_eta2 = if ss_effect + ss_error > 0.0 { (ss_effect / (ss_effect + ss_error)).clamp(0.0, 1.0) } else { 0.0 };
    let omega2_denom = ss_total + ms_error;
    let omega2 = if omega2_denom > 0.0 { ((ss_effect - df_effect * ms_error) / omega2_denom).clamp(0.0, 1.0) } else { 0.0 };
    let cohens_f = if partial_eta2 < 1.0 {
        (partial_eta2 / (1.0 - partial_eta2)).sqrt()
    } else if partial_eta2 > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };

    AnovaEffectSizes {
        eta_squared: eta2,
        eta_squared_ci: None,
        partial_eta_squared: partial_eta2,
        partial_eta_squared_ci: None,
        omega_squared: omega2,
        cohens_f,
        cohens_f_ci: None,
    }
}

/// Compute ANOVA effect sizes (`eta^2`, `partial eta^2`, `omega^2`, Cohen's `f`) and exact
/// Steiger (2004) noncentral-*F* confidence intervals for `partial_eta_squared` and `cohens_f`
/// using an explicit total sample size `total_n`.
///
/// For one-way designs (`SS_effect + SS_error == SS_total` and `df_effect + df_error + 1 == total_n`),
/// `eta_squared_ci` coincides with `partial_eta_squared_ci`. For multi-way designs,
/// `partial_eta_squared_ci` uses `lambda / (lambda + total_n)` per Steiger (2004, eq. 16) while
/// `eta_squared_ci` is left `None` because the noncentral-*F* pivot parameterizes `partial eta^2`,
/// not classical `eta^2 = SS_effect / SS_total`. When `ms_error == 0` with `ss_effect > 0`, `F`
/// diverges to `+inf` (`eta^2 = 1`, `cohens_f = +inf`) and all noncentral-*F* CIs return `None`.
pub fn compute_anova_effect_sizes_for_n(
    ss_effect: f64,
    ss_error: f64,
    ss_total: f64,
    df_effect: f64,
    df_error: f64,
    total_n: usize,
    confidence: ConfidenceLevel,
) -> AnovaEffectSizes {
    let ms_error = if df_error > 0.0 { ss_error / df_error } else { 0.0 };
    let eta2 = if ss_total > 0.0 { (ss_effect / ss_total).clamp(0.0, 1.0) } else { 0.0 };
    let partial_eta2 = if ss_effect + ss_error > 0.0 { (ss_effect / (ss_effect + ss_error)).clamp(0.0, 1.0) } else { 0.0 };
    let omega2_denom = ss_total + ms_error;
    let omega2 = if omega2_denom > 0.0 { ((ss_effect - df_effect * ms_error) / omega2_denom).clamp(0.0, 1.0) } else { 0.0 };
    let cohens_f = if partial_eta2 < 1.0 {
        (partial_eta2 / (1.0 - partial_eta2)).sqrt()
    } else if partial_eta2 > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };

    let f_obs = if ms_error > 0.0 && df_effect > 0.0 {
        (ss_effect / df_effect) / ms_error
    } else if ss_effect > 0.0 && df_effect > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let total_n_f = total_n as f64;
    let ncf_ci = if total_n > 0 && f_obs.is_finite() && df_effect >= 1.0 && df_error >= 1.0 {
        noncentral_f_ci(f_obs.max(0.0), df_effect, df_error, confidence.percent()).ok()
    } else {
        None
    };

    let partial_eta_ci = ncf_ci.map(|(lo, hi)| {
        let p_lo = (lo / (lo + total_n_f)).clamp(0.0, 1.0);
        let p_hi = (hi / (hi + total_n_f)).clamp(0.0, 1.0);
        (p_lo, p_hi)
    });
    let cohens_f_ci = partial_eta_ci.map(|(p_lo, p_hi)| {
        let f_lo = if p_lo < 1.0 { (p_lo / (1.0 - p_lo)).sqrt() } else { f64::INFINITY };
        let f_hi = if p_hi < 1.0 { (p_hi / (1.0 - p_hi)).sqrt() } else { f64::INFINITY };
        (f_lo, f_hi)
    });
    let is_one_way =
        ((ss_effect + ss_error) - ss_total).abs() <= 1e-9 * ss_total.max(1e-30) && (df_effect + df_error + 1.0 - total_n_f).abs() <= 1e-9;

    AnovaEffectSizes {
        eta_squared: eta2,
        eta_squared_ci: if is_one_way { partial_eta_ci } else { None },
        partial_eta_squared: partial_eta2,
        partial_eta_squared_ci: partial_eta_ci,
        omega_squared: omega2,
        cohens_f,
        cohens_f_ci,
    }
}

/// Bartlett's and Brown–Forsythe homogeneity-of-variance tests across `k >= 2` groups.
///
/// Bartlett (1937) statistic with the usual correction
/// `C = 1 + (sum 1/nu_i - 1/sum nu_i) / (3 (k - 1))`, referred to `chi^2(k - 1)`. The
/// Brown & Forsythe (1974a) test is the median-centred Levene variant: ordinary one-way ANOVA on
/// `|x_ij - median_i|`.
pub fn variance_homogeneity_tests(groups: &[&[f64]]) -> Result<VarianceHomogeneityResult, StatsError> {
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: groups.len() });
    }
    let mut group_vars = Vec::with_capacity(groups.len());
    let mut group_df = Vec::with_capacity(groups.len());
    let mut abs_dev_groups: Vec<Vec<f64>> = Vec::with_capacity(groups.len());

    let mut total_df = 0.0_f64;
    let mut ss_within = NeumaierSum::new();
    let mut sum_inv_df = NeumaierSum::new();

    for &g in groups {
        validate_finite_slice(g)?;
        if g.len() < 2 {
            return Err(StatsError::InsufficientSampleSize { context: "ANOVA group variance", required: 2, actual: g.len() });
        }
        let m = SummaryMoments::from_slice(g)?;
        let df_i = (g.len() - 1) as f64;
        let var_i = m.sample_variance()?;
        group_vars.push(var_i);
        group_df.push(df_i);
        total_df += df_i;
        ss_within.add(m.css);
        sum_inv_df.add(1.0 / df_i);

        let sorted = sorted_finite(g);
        let med = quantile_type6_sorted(&sorted, 0.50).unwrap_or(m.mean);
        abs_dev_groups.push(g.iter().map(|&x| (x - med).abs()).collect());
    }

    let k = groups.len() as f64;
    let pooled_var = ss_within.total() / total_df;
    let (bartlett_chi2, bartlett_p) = if pooled_var > 0.0 && group_vars.iter().all(|&v| v > 0.0) {
        let mut sum_df_ln_v = NeumaierSum::new();
        for (&df_i, &v_i) in group_df.iter().zip(group_vars.iter()) {
            sum_df_ln_v.add(df_i * v_i.ln());
        }
        let num = total_df * pooled_var.ln() - sum_df_ln_v.total();
        let c = 1.0 + (sum_inv_df.total() - 1.0 / total_df) / (3.0 * (k - 1.0));
        let stat = (num / c).max(0.0);
        (stat, chi2_sf(stat, k - 1.0)?)
    } else if pooled_var > 0.0 {
        (f64::INFINITY, 0.0)
    } else {
        (0.0, 1.0)
    };

    let dev_refs: Vec<&[f64]> = abs_dev_groups.iter().map(Vec::as_slice).collect();
    let bf = ordinary_one_way_anova_slices(&dev_refs, ConfidenceLevel::DEFAULT_95)?;

    Ok(VarianceHomogeneityResult {
        bartlett_chi2,
        bartlett_df: k - 1.0,
        bartlett_p,
        brown_forsythe_f: bf.f_statistic,
        brown_forsythe_df1: bf.df_between,
        brown_forsythe_df2: bf.df_within,
        brown_forsythe_p: bf.p_value,
        available: true,
    })
}
