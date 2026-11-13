//! Two-group parametric and nonparametric hypothesis tests, standardized effect sizes with
//! exact noncentral-*t* confidence intervals, variance homogeneity tests, and Bland–Altman agreement.

use lumencraft_doc::{
    AnalysisSpec, ChecklistItem, ChecklistStatus, DataSheet, PValueFormat, PairwiseComparisonRecord, ResultCell, ResultSheet, ResultTabKind,
    TwoGroupMethod, TwoGroupSpec,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{f_sf, norm_sf, norm_two_sided_isf, student_t_cdf, student_t_sf, student_t_two_sided_p};
use lumencraft_math::noncentral_t_ci;
use lumencraft_math::sum::{NeumaierSum, SummaryMoments, pearson_r};
use serde::{Deserialize, Serialize};

use crate::column::{hedges_j_factor, hodges_lehmann_one_sample, ks_asymptotic_p, wilcoxon_exact_p};
use crate::common::{
    build_tab, checklist_item, ci_cell, extract_numeric_columns, extract_paired_values, init_result_sheet, num_cell, opt_ci_cell, opt_num_cell,
    p_cell, push_scalar, quantile_type6_sorted, rank_with_ties, resolve_selected_indices, sorted_finite, t_crit_two_sided, validate_finite_slice,
};
use crate::error::StatsError;
use crate::multiplicity::{MultiplicityProcedure, adjust_p_values};

// ============================================================================
// Variance Homogeneity Tests (F-test, Levene, Brown-Forsythe)
// ============================================================================

/// Result of an *F*-test comparing two sample variances.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VarianceFTestResult {
    pub f_statistic: f64,
    pub df_num: f64,
    pub df_den: f64,
    pub p_value: f64,
}

/// Two-sided *F*-test for equality of variances (`s_max^2 / s_min^2`).
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
    pub use_median: bool,
    pub f_statistic: f64,
    pub df_between: f64,
    pub df_within: f64,
    pub p_value: f64,
}

/// Levene (`use_median = false`) or Brown–Forsythe (`use_median = true`) homogeneity-of-variance test.
pub fn levene_variance_test(groups: &[&[f64]], use_median: bool) -> Result<LeveneTestResult, StatsError> {
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Levene test", required: 2, actual: groups.len() });
    }
    let mut z_groups = Vec::with_capacity(groups.len());
    let mut all_z = Vec::new();
    for grp in groups {
        validate_finite_slice(grp)?;
        if grp.len() < 2 {
            return Err(StatsError::InsufficientSampleSize { context: "Levene group", required: 2, actual: grp.len() });
        }
        let center = if use_median {
            let s = sorted_finite(grp);
            quantile_type6_sorted(&s, 0.5).unwrap_or(s[0])
        } else {
            SummaryMoments::from_slice(grp)?.mean
        };
        let z: Vec<f64> = grp.iter().map(|&v| (v - center).abs()).collect();
        all_z.extend_from_slice(&z);
        z_groups.push(z);
    }
    let total_n = all_z.len();
    let k = groups.len();
    let grand = SummaryMoments::from_slice(&all_z)?;
    let mut ss_b = NeumaierSum::new();
    let mut ss_w = NeumaierSum::new();
    for zg in &z_groups {
        let gm = SummaryMoments::from_slice(zg)?;
        let d = gm.mean - grand.mean;
        ss_b.add((gm.count as f64) * d * d);
        ss_w.add(gm.css);
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
    Ok(LeveneTestResult { use_median, f_statistic: f_stat, df_between: df_b, df_within: df_w, p_value: p_val })
}

// ============================================================================
// Parametric Two-Group Comparisons & Standardized Effect Sizes
// ============================================================================

/// Standardized effect sizes and exact noncentral-*t* confidence intervals for two-group tests.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TwoGroupEffectSizes {
    pub cohens_d: f64,
    pub cohens_d_ci: Option<(f64, f64)>,
    pub hedges_g: f64,
    pub hedges_g_ci: Option<(f64, f64)>,
    pub glass_delta: Option<f64>,
    pub glass_delta_ci: Option<(f64, f64)>,
    pub r_squared: f64,
}

/// Complete output of an unpaired Student *t*, Welch *t*, or lognormal unpaired *t*-test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnpairedTResult {
    pub method_label: &'static str,
    pub n_a: usize,
    pub n_b: usize,
    pub mean_a: f64,
    pub mean_b: f64,
    pub sd_a: f64,
    pub sd_b: f64,
    pub difference: f64,
    pub se_difference: f64,
    pub ci: (f64, f64),
    pub t_statistic: f64,
    pub df: f64,
    pub p_value: f64,
    pub two_tailed: bool,
    pub f_test: VarianceFTestResult,
    pub levene_median: LeveneTestResult,
    pub effect_sizes: TwoGroupEffectSizes,
    /// Present for `LognormalUnpairedT`: geometric mean ratio `exp(mean_ln_a - mean_ln_b)` and CI.
    pub geometric_mean_ratio: Option<f64>,
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

fn compute_unpaired_effect_sizes(
    diff: f64,
    sds: (f64, f64, f64),
    n_pair: (usize, usize),
    t_df_conf: (f64, f64, ConfidenceLevel),
) -> Result<TwoGroupEffectSizes, StatsError> {
    let (sd_a, sd_b, sd_pooled) = sds;
    let (n_a, n_b) = n_pair;
    let (t_stat, df_ci, confidence) = t_df_conf;
    let na = n_a as f64;
    let nb = n_b as f64;
    let n_eff = ((na * nb) / (na + nb)).sqrt();
    let d = if sd_pooled > 0.0 { diff / sd_pooled } else { 0.0 };
    let df_pool = (n_a + n_b - 2) as f64;
    let j = if df_pool > 1.0 { hedges_j_factor(df_pool)? } else { 1.0 };
    let g = j * d;

    let (d_ci, g_ci) = if sd_pooled > 0.0 && n_eff > 0.0 {
        let t_for_d = d * n_eff;
        if let Ok((lo, hi)) = noncentral_t_ci(t_for_d, df_ci, confidence.percent()) {
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
        let g_ci = noncentral_t_ci(t_glass, (n_a - 1) as f64, confidence.percent()).ok().map(|(lo, hi)| (lo / n_eff, hi / n_eff));
        (Some(gd), g_ci)
    } else if sd_b > 0.0 {
        let gd = diff / sd_b;
        let t_glass = gd * n_eff;
        let g_ci = noncentral_t_ci(t_glass, (n_b - 1) as f64, confidence.percent()).ok().map(|(lo, hi)| (lo / n_eff, hi / n_eff));
        (Some(gd), g_ci)
    } else {
        (None, None)
    };

    let r_sq = if df_ci > 0.0 { (t_stat * t_stat) / (t_stat * t_stat + df_ci) } else { 0.0 };
    Ok(TwoGroupEffectSizes { cohens_d: d, cohens_d_ci: d_ci, hedges_g: g, hedges_g_ci: g_ci, glass_delta, glass_delta_ci, r_squared: r_sq })
}

/// Unpaired two-sample Student *t*-test (`a - b` or `b - a`; here `mean_a - mean_b`).
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
    let levene_median = levene_variance_test(&[a, b], true)?;
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
    let levene_median = levene_variance_test(&[a, b], true)?;
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

/// Output of a paired *t*-test or ratio paired *t*-test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedTResult {
    pub method_label: &'static str,
    pub n_pairs: usize,
    pub mean_of_differences: f64,
    pub sd_of_differences: f64,
    pub sem_of_differences: f64,
    pub ci: (f64, f64),
    pub t_statistic: f64,
    pub df: f64,
    pub p_value: f64,
    pub two_tailed: bool,
    pub pairing_r: Option<f64>,
    pub pairing_p: Option<f64>,
    pub cohens_dz: f64,
    pub hedges_gz: f64,
    pub hedges_gz_ci: Option<(f64, f64)>,
    pub geometric_mean_ratio: Option<f64>,
    pub geometric_mean_ratio_ci: Option<(f64, f64)>,
}

/// Paired *t*-test on matched pairs `(a_i, b_i)` analyzing `a_i - b_i`.
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
            let p_r = if (1.0 - r * r) > 0.0 {
                let tr = r * (((n - 2) as f64) / (1.0 - r * r)).sqrt();
                student_t_sf(tr, (n - 2) as f64).ok()
            } else {
                Some(0.0)
            };
            (Some(r), p_r)
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let dz = md.mean / sd;
    let j = hedges_j_factor(df)?;
    let gz = j * dz;
    let inv_sqrt_n = 1.0 / (n as f64).sqrt();
    let gz_ci = noncentral_t_ci(t_stat, df, confidence.percent()).ok().map(|(lo, hi)| (lo * inv_sqrt_n * j, hi * inv_sqrt_n * j));

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

/// Result of a Mann–Whitney *U* test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MannWhitneyResult {
    pub n_a: usize,
    pub n_b: usize,
    pub median_a: f64,
    pub median_b: f64,
    pub u_a: f64,
    pub u_b: f64,
    pub u_min: f64,
    pub p_value: f64,
    pub exact_p_used: bool,
    pub two_tailed: bool,
    pub hodges_lehmann_diff: f64,
    pub hodges_lehmann_ci: Option<(f64, f64)>,
    pub cliffs_delta: f64,
    pub rank_biserial_r: f64,
}

/// Exact DP distribution of Mann–Whitney `U` for `n1 + n2 <= 40` without ties.
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
    if two_tailed {
        let min_tail = lower_tail.min(upper_tail);
        (2.0 * min_tail / total_ways).clamp(0.0, 1.0)
    } else {
        (upper_tail / total_ways).clamp(0.0, 1.0)
    }
}

/// Mann–Whitney *U* test comparing `a` and `b`, with exact DP *p*-values for `n_a + n_b <= 40`
/// without ties, Hodges–Lehmann median difference `a - b` + Moses CI, Cliff's `delta`, and
/// rank-biserial `r`.
pub fn mann_whitney_u(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<MannWhitneyResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    let na = a.len();
    let nb = b.len();
    let sa = sorted_finite(a);
    let sb = sorted_finite(b);
    let median_a = quantile_type6_sorted(&sa, 0.5).unwrap_or(sa[0]);
    let median_b = quantile_type6_sorted(&sb, 0.5).unwrap_or(sb[0]);

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
    } else {
        let mean_u = 0.5 * naf * nbf;
        let var_u = ((naf * nbf / 12.0) * (nf + 1.0 - ranked.tie_sum_t3_minus_t / (nf * (nf - 1.0)))).max(1e-30);
        let sd_u = var_u.sqrt();
        let p = if two_tailed {
            let cc = (u_a - mean_u).abs() - 0.5;
            if cc <= 0.0 { 1.0 } else { (2.0 * norm_sf(cc / sd_u)).min(1.0) }
        } else {
            norm_sf((u_a - mean_u - 0.5) / sd_u)
        };
        (p.clamp(0.0, 1.0), false)
    };

    let (hodges_lehmann_diff, hodges_lehmann_ci) = hodges_lehmann_two_sample(a, b, confidence)?;
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
        cliffs_delta,
        rank_biserial_r,
    })
}

fn hodges_lehmann_two_sample(a: &[f64], b: &[f64], confidence: ConfidenceLevel) -> Result<(f64, Option<(f64, f64)>), StatsError> {
    let na = a.len();
    let nb = b.len();
    let m = na.saturating_mul(nb);
    if m == 0 {
        return Err(StatsError::EmptyInput);
    }
    if m > 40_000 {
        let sa = sorted_finite(a);
        let sb = sorted_finite(b);
        let ma = quantile_type6_sorted(&sa, 0.5).unwrap_or(sa[0]);
        let mb = quantile_type6_sorted(&sb, 0.5).unwrap_or(sb[0]);
        return Ok((ma - mb, None));
    }
    let mut diffs = Vec::with_capacity(m);
    for &va in a {
        for &vb in b {
            diffs.push(va - vb);
        }
    }
    diffs.sort_by(f64::total_cmp);
    let hl = quantile_type6_sorted(&diffs, 0.5).unwrap_or(diffs[0]);
    if na + nb < 4 {
        return Ok((hl, None));
    }
    let z = norm_two_sided_isf(confidence.alpha())?;
    let naf = na as f64;
    let nbf = nb as f64;
    let mu = 0.5 * naf * nbf;
    let sd = (naf * nbf * (naf + nbf + 1.0) / 12.0).sqrt();
    let k = (mu - z * sd).floor().max(1.0) as usize;
    let lo_idx = k.saturating_sub(1).min(m - 1);
    let hi_idx = (m - k).min(m - 1);
    Ok((hl, Some((diffs[lo_idx], diffs[hi_idx]))))
}

/// Output of Wilcoxon matched-pairs signed-rank test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WilcoxonMatchedResult {
    pub n_pairs: usize,
    pub n_effective: usize,
    pub w_plus: f64,
    pub w_minus: f64,
    pub w_stat: f64,
    pub p_value: f64,
    pub exact_p_used: bool,
    pub two_tailed: bool,
    pub hodges_lehmann_pseudomedian: f64,
    pub hodges_lehmann_ci: Option<(f64, f64)>,
    pub rank_biserial_r: f64,
}

/// Wilcoxon matched-pairs signed-rank test (`use_pratt_zeros = false` uses classic zero exclusion;
/// `true` uses Pratt's zero-inclusion tie adjustment).
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

    let (p_value, exact_p_used) = if !use_pratt_zeros && n_eff <= 25 && ranked.tie_counts.is_empty() {
        let max_sum = n_eff * (n_eff + 1) / 2;
        (wilcoxon_exact_p(n_eff, w_plus.round() as usize, max_sum, two_tailed), true)
    } else {
        let nf = n_eff as f64;
        let zf = n_zeros as f64;
        let mean_w = 0.25 * (nf * (nf + 1.0) - zf * (zf + 1.0));
        let var_w = ((nf * (nf + 1.0) * (2.0 * nf + 1.0) - zf * (zf + 1.0) * (2.0 * zf + 1.0) - 0.5 * ranked.tie_sum_t3_minus_t) / 24.0).max(1e-30);
        let sd_w = var_w.sqrt();
        let p = if two_tailed {
            let cc = (w_plus - mean_w).abs() - 0.5;
            if cc <= 0.0 { 1.0 } else { (2.0 * norm_sf(cc / sd_w)).min(1.0) }
        } else {
            norm_sf((w_plus - mean_w - 0.5) / sd_w)
        };
        (p.clamp(0.0, 1.0), false)
    };

    let (hl, hl_ci) = hodges_lehmann_one_sample(&diffs_all, confidence)?;
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
        rank_biserial_r,
    })
}

/// Output of a two-sample Kolmogorov–Smirnov test.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TwoSampleKsResult {
    pub n_a: usize,
    pub n_b: usize,
    pub d_statistic: f64,
    pub p_value: f64,
    pub exact_p_used: bool,
}

/// Two-sample Kolmogorov–Smirnov test (exact Smirnov lattice path DP for `n_a * n_b <= 625` without
/// ties, Smirnov asymptotic series otherwise).
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

/// Bland–Altman method-comparison bias and Limits of Agreement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlandAltmanResult {
    pub n: usize,
    pub percent_difference: bool,
    pub bias: f64,
    pub sd_bias: f64,
    pub se_bias: f64,
    pub bias_ci: (f64, f64),
    pub loa_lower: f64,
    pub loa_upper: f64,
    pub loa_lower_ci: (f64, f64),
    pub loa_upper_ci: (f64, f64),
    /// `(average, difference_or_percent_diff)` points for Bland–Altman plotting.
    pub points: Vec<(f64, f64)>,
}

/// Compute Bland–Altman agreement (`method_a - method_b` vs `(method_a + method_b) / 2`).
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
        let avg = 0.5 * (a + b);
        let d = if percent_difference {
            if avg == 0.0 {
                return Err(StatsError::InvalidParameter { name: "average", value: 0.0 });
            }
            100.0 * (a - b) / avg
        } else {
            a - b
        };
        points.push((avg, d));
        diffs.push(d);
    }

    let md = SummaryMoments::from_slice(&diffs)?;
    let sd_bias = md.sample_sd()?;
    let se_bias = md.sem()?;
    let nf = n as f64;
    let df = nf - 1.0;
    let z_loa = norm_two_sided_isf(confidence.alpha())?;
    let t_crit = t_crit_two_sided(confidence, df)?;

    let loa_lower = md.mean - z_loa * sd_bias;
    let loa_upper = md.mean + z_loa * sd_bias;
    let se_loa = sd_bias * (1.0 / nf + (z_loa * z_loa) / (2.0 * df)).sqrt();

    Ok(BlandAltmanResult {
        n,
        percent_difference,
        bias: md.mean,
        sd_bias,
        se_bias,
        bias_ci: (md.mean - t_crit * se_bias, md.mean + t_crit * se_bias),
        loa_lower,
        loa_upper,
        loa_lower_ci: (loa_lower - t_crit * se_loa, loa_lower + t_crit * se_loa),
        loa_upper_ci: (loa_upper - t_crit * se_loa, loa_upper + t_crit * se_loa),
        points,
    })
}

// ============================================================================
// Multiple Row-wise Tests (Volcano Plot Source)
// ============================================================================

/// Single row comparison entry for multiple *t*-tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleRowTestEntry {
    pub row_index: usize,
    pub label: String,
    pub mean_a: f64,
    pub mean_b: f64,
    pub difference: f64,
    pub statistic: f64,
    pub df: f64,
    pub raw_p: f64,
    pub adjusted_p_or_q: f64,
    pub significant: bool,
}

/// Run one unpaired or Welch *t*-test per row across two replicate groups and apply multiplicity.
pub fn multiple_row_t_tests(
    row_labels: &[String],
    group_a_rows: &[Vec<f64>],
    group_b_rows: &[Vec<f64>],
    welch: bool,
    procedure: MultiplicityProcedure,
    threshold: f64,
) -> Result<Vec<MultipleRowTestEntry>, StatsError> {
    if group_a_rows.len() != group_b_rows.len() {
        return Err(StatsError::DimensionMismatch { expected: group_a_rows.len(), actual: group_b_rows.len() });
    }
    let mut raw_p = Vec::new();
    let mut staged = Vec::new();
    for (idx, (ra, rb)) in group_a_rows.iter().zip(group_b_rows.iter()).enumerate() {
        let res =
            if welch { welch_t_test(ra, rb, true, ConfidenceLevel::DEFAULT_95) } else { unpaired_t_test(ra, rb, true, ConfidenceLevel::DEFAULT_95) };
        if let Ok(r) = res {
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
            significant: adj_p <= threshold,
        })
        .collect())
}

// ============================================================================
// ResultSheet Builder
// ============================================================================

/// Execute [`TwoGroupSpec`] on a [`DataSheet`] and emit a complete [`ResultSheet`].
pub fn run_two_group_analysis(sheet: &DataSheet, spec: &AnalysisSpec, tg: &TwoGroupSpec) -> Result<ResultSheet, StatsError> {
    let mut res = init_result_sheet(spec)?;
    let conf = spec.confidence_level;
    let p_fmt = spec.p_value_format;

    match tg.method {
        TwoGroupMethod::OneSampleT { hypothetical } => {
            let cols = extract_numeric_columns(sheet, &spec.selected_data_sets)?;
            let first = cols.first().ok_or(StatsError::EmptyInput)?;
            let out = crate::column::one_sample_t_test(&first.values, hypothetical, tg.two_tailed, conf)?;
            populate_one_sample_t_sheet(&mut res, &first.label, &out, conf, p_fmt);
            return Ok(res);
        }
        TwoGroupMethod::OneSampleWilcoxon { hypothetical } => {
            let cols = extract_numeric_columns(sheet, &spec.selected_data_sets)?;
            let first = cols.first().ok_or(StatsError::EmptyInput)?;
            let out = crate::column::one_sample_wilcoxon(&first.values, hypothetical, tg.two_tailed, conf)?;
            populate_one_sample_w_sheet(&mut res, &first.label, &out, conf, p_fmt);
            return Ok(res);
        }
        _ => {}
    }

    let indices = resolve_selected_indices(&spec.selected_data_sets, sheet.archetype.column_count());
    if indices.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "two-group comparison columns", required: 2, actual: indices.len() });
    }
    let cols = extract_numeric_columns(sheet, &indices[..2])?;
    let (ca, cb) = (&cols[0], &cols[1]);

    match tg.method {
        TwoGroupMethod::UnpairedT => {
            let out = unpaired_t_test(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_unpaired_t_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, tg.compute_effect_sizes, conf, p_fmt);
        }
        TwoGroupMethod::WelchT => {
            let out = welch_t_test(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_unpaired_t_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, tg.compute_effect_sizes, conf, p_fmt);
        }
        TwoGroupMethod::LognormalUnpairedT => {
            let out = lognormal_unpaired_t_test(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_unpaired_t_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, tg.compute_effect_sizes, conf, p_fmt);
        }
        TwoGroupMethod::PairedT => {
            let (xa, xb) = extract_paired_values(sheet, ca.index, cb.index)?;
            let out = paired_t_test(&xa, &xb, tg.two_tailed, conf)?;
            populate_paired_t_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, tg.compute_effect_sizes, conf, p_fmt);
        }
        TwoGroupMethod::RatioPairedT => {
            let (xa, xb) = extract_paired_values(sheet, ca.index, cb.index)?;
            let out = ratio_paired_t_test(&xa, &xb, tg.two_tailed, conf)?;
            populate_paired_t_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, tg.compute_effect_sizes, conf, p_fmt);
        }
        TwoGroupMethod::MannWhitneyU => {
            let out = mann_whitney_u(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_mann_whitney_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, conf, p_fmt);
        }
        TwoGroupMethod::WilcoxonSignedRank => {
            let (xa, xb) = extract_paired_values(sheet, ca.index, cb.index)?;
            let out = wilcoxon_matched_pairs(&xa, &xb, tg.two_tailed, conf, false)?;
            populate_wilcoxon_paired_sheet(&mut res, ca.index, cb.index, &ca.label, &cb.label, &out, conf, p_fmt);
        }
        TwoGroupMethod::KolmogorovSmirnov => {
            let out = ks_two_sample(&ca.values, &cb.values)?;
            populate_ks_sheet(&mut res, (ca.index, cb.index), (&ca.label, &cb.label), &out, conf, p_fmt);
        }
        TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. } => {}
    }
    Ok(res)
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
    let mut row_labels = vec![
        String::from("P value"),
        String::from("t statistic"),
        String::from("df"),
        format!("Mean ({lbl_a})"),
        format!("Mean ({lbl_b})"),
        String::from("Difference between means (A - B)"),
        String::from("SE of difference"),
        format!("{}% CI of difference", conf.percent()),
        String::from("F-test to compare variances (F)"),
        String::from("F-test P value"),
    ];
    let mut rows = vec![
        vec![p_cell(out.p_value, p_fmt)],
        vec![num_cell(out.t_statistic)],
        vec![num_cell(out.df)],
        vec![num_cell(out.mean_a)],
        vec![num_cell(out.mean_b)],
        vec![num_cell(out.difference)],
        vec![num_cell(out.se_difference)],
        vec![ci_cell(out.ci.0, out.ci.1)],
        vec![num_cell(out.f_test.f_statistic)],
        vec![p_cell(out.f_test.p_value, p_fmt)],
    ];
    if let Some(gmr) = out.geometric_mean_ratio {
        row_labels.push(String::from("Geometric mean ratio (A / B)"));
        rows.push(vec![num_cell(gmr)]);
        row_labels.push(format!("{}% CI of geometric ratio", conf.percent()));
        rows.push(vec![opt_ci_cell(out.geometric_mean_ratio_ci)]);
    }
    if include_es {
        row_labels.extend([
            String::from("Cohen's d"),
            format!("{}% CI of Cohen's d", conf.percent()),
            String::from("Hedges' g"),
            format!("{}% CI of Hedges' g", conf.percent()),
            String::from("Glass's Delta"),
            String::from("R squared"),
        ]);
        rows.extend([
            vec![num_cell(out.effect_sizes.cohens_d)],
            vec![opt_ci_cell(out.effect_sizes.cohens_d_ci)],
            vec![num_cell(out.effect_sizes.hedges_g)],
            vec![opt_ci_cell(out.effect_sizes.hedges_g_ci)],
            vec![opt_num_cell(out.effect_sizes.glass_delta)],
            vec![num_cell(out.effect_sizes.r_squared)],
        ]);
    }
    res.tabs.push(build_tab("two-group", out.method_label, ResultTabKind::MainSummary, vec![format!("{lbl_a} vs. {lbl_b}")], row_labels, rows));
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
        significant: out.p_value <= conf.alpha(),
    });
    append_unpaired_checklist(&mut res.checklist, out, conf.alpha());
}

fn append_unpaired_checklist(checklist: &mut Vec<ChecklistItem>, out: &UnpairedTResult, alpha: f64) {
    if out.n_a < 5 || out.n_b < 5 {
        checklist.push(checklist_item(
            "Sample size adequacy",
            ChecklistStatus::Warn,
            Some(format!("n_A = {}, n_B = {}", out.n_a, out.n_b)),
            "Small group sample sizes limit statistical power and make normality checks unreliable.",
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
            "Group variances differ significantly; Welch's t-test is recommended.",
        ));
    } else {
        checklist.push(checklist_item(
            "Variance homogeneity check",
            ChecklistStatus::Pass,
            Some(format!("Levene(median) F = {:.3}, p = {:.4}", out.levene_median.f_statistic, out.levene_median.p_value)),
            "No severe departure from equal group variances was flagged.",
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
    let mut row_labels = vec![
        String::from("P value"),
        String::from("t statistic"),
        String::from("df"),
        String::from("Number of pairs"),
        String::from("Mean of differences (A - B)"),
        String::from("SD of differences"),
        format!("{}% CI of difference", conf.percent()),
        String::from("Pairing correlation (r)"),
        String::from("Pairing effectiveness P"),
    ];
    let mut rows = vec![
        vec![p_cell(out.p_value, p_fmt)],
        vec![num_cell(out.t_statistic)],
        vec![num_cell(out.df)],
        vec![num_cell(out.n_pairs as f64)],
        vec![num_cell(out.mean_of_differences)],
        vec![num_cell(out.sd_of_differences)],
        vec![ci_cell(out.ci.0, out.ci.1)],
        vec![opt_num_cell(out.pairing_r)],
        vec![out.pairing_p.map_or(ResultCell::Empty, |p| p_cell(p, p_fmt))],
    ];
    if let Some(gmr) = out.geometric_mean_ratio {
        row_labels.push(String::from("Geometric mean of ratios (A / B)"));
        rows.push(vec![num_cell(gmr)]);
        row_labels.push(format!("{}% CI of ratio", conf.percent()));
        rows.push(vec![opt_ci_cell(out.geometric_mean_ratio_ci)]);
    }
    if include_es {
        row_labels.extend([String::from("Cohen's dz"), String::from("Hedges' gz"), format!("{}% CI of Hedges' gz", conf.percent())]);
        rows.extend([vec![num_cell(out.cohens_dz)], vec![num_cell(out.hedges_gz)], vec![opt_ci_cell(out.hedges_gz_ci)]]);
    }
    res.tabs.push(build_tab("paired-t", out.method_label, ResultTabKind::MainSummary, vec![format!("{lbl_a} vs. {lbl_b}")], row_labels, rows));
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
    let pairing_status = match out.pairing_p {
        Some(p) if p <= conf.alpha() => ChecklistStatus::Pass,
        Some(_) => ChecklistStatus::Warn,
        None => ChecklistStatus::Info,
    };
    res.checklist.push(checklist_item(
        "Pairing effectiveness",
        pairing_status,
        out.pairing_r.map(|r| format!("r = {r:.4}")),
        "Evaluates whether matched pairs covary positively across subjects.",
    ));
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
    let row_labels = vec![
        String::from("P value"),
        String::from("Exact P used"),
        String::from("Mann-Whitney U"),
        String::from("Hodges-Lehmann median diff."),
        format!("{}% CI of median diff.", conf.percent()),
        String::from("Cliff's delta"),
        String::from("Rank-biserial r"),
    ];
    let rows = vec![
        vec![p_cell(out.p_value, p_fmt)],
        vec![ResultCell::Boolean(out.exact_p_used)],
        vec![num_cell(out.u_min)],
        vec![num_cell(out.hodges_lehmann_diff)],
        vec![opt_ci_cell(out.hodges_lehmann_ci)],
        vec![num_cell(out.cliffs_delta)],
        vec![num_cell(out.rank_biserial_r)],
    ];
    res.tabs.push(build_tab("mann-whitney", "Mann-Whitney test", ResultTabKind::MainSummary, vec![format!("{lbl_a} vs. {lbl_b}")], row_labels, rows));
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
        significant: out.p_value <= conf.alpha(),
    });
    res.checklist.push(checklist_item(
        "Nonparametric rank comparison",
        ChecklistStatus::Pass,
        Some(format!("n_A = {}, n_B = {}", out.n_a, out.n_b)),
        "Does not assume Gaussian normality; checks distributional shift under independence.",
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
    let row_labels = vec![
        String::from("P value"),
        String::from("Exact P used"),
        String::from("Sum of positive ranks (W+)"),
        String::from("Sum of negative ranks (W-)"),
        String::from("Hodges-Lehmann pseudomedian"),
        format!("{}% CI of pseudomedian", conf.percent()),
        String::from("Rank-biserial r"),
    ];
    let rows = vec![
        vec![p_cell(out.p_value, p_fmt)],
        vec![ResultCell::Boolean(out.exact_p_used)],
        vec![num_cell(out.w_plus)],
        vec![num_cell(out.w_minus)],
        vec![num_cell(out.hodges_lehmann_pseudomedian)],
        vec![opt_ci_cell(out.hodges_lehmann_ci)],
        vec![num_cell(out.rank_biserial_r)],
    ];
    res.tabs.push(build_tab(
        "wilcoxon-paired",
        "Wilcoxon matched-pairs test",
        ResultTabKind::MainSummary,
        vec![format!("{lbl_a} vs. {lbl_b}")],
        row_labels,
        rows,
    ));
    push_scalar(res, "PValue", None, out.p_value);
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.hodges_lehmann_pseudomedian),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= conf.alpha(),
    });
    res.checklist.push(checklist_item(
        "Symmetry of paired differences",
        ChecklistStatus::Info,
        Some(format!("effective pairs = {}", out.n_effective)),
        "Wilcoxon signed-rank assumes paired differences are symmetrically distributed around the median.",
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
    let row_labels = vec![String::from("P value"), String::from("Kolmogorov-Smirnov D"), String::from("Exact P used")];
    let rows = vec![vec![p_cell(out.p_value, p_fmt)], vec![num_cell(out.d_statistic)], vec![ResultCell::Boolean(out.exact_p_used)]];
    res.tabs.push(build_tab(
        "ks-two-sample",
        "Kolmogorov-Smirnov test",
        ResultTabKind::MainSummary,
        vec![format!("{lbl_a} vs. {lbl_b}")],
        row_labels,
        rows,
    ));
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

fn populate_one_sample_t_sheet(
    res: &mut ResultSheet,
    label: &str,
    out: &crate::column::OneSampleTResult,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    let row_labels = vec![
        String::from("Hypothetical mean"),
        String::from("Sample mean"),
        String::from("Discrepancy"),
        String::from("t statistic"),
        String::from("df"),
        String::from("P value"),
        format!("{}% CI of discrepancy", conf.percent()),
        String::from("Hedges' g"),
    ];
    let rows = vec![
        vec![num_cell(out.hypothetical_mean)],
        vec![num_cell(out.sample_mean)],
        vec![num_cell(out.discrepancy)],
        vec![num_cell(out.t_statistic)],
        vec![num_cell(out.df)],
        vec![p_cell(out.p_value, p_fmt)],
        vec![ci_cell(out.ci.0, out.ci.1)],
        vec![num_cell(out.hedges_g)],
    ];
    res.tabs.push(build_tab("one-sample-t", "One-sample t test", ResultTabKind::MainSummary, vec![label.to_owned()], row_labels, rows));
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "t", None, out.t_statistic);
    res.checklist.push(checklist_item(
        "One-sample Gaussian assumption",
        if out.n < 5 { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
        Some(format!("n = {}", out.n)),
        "One-sample t test assumes independent observations from an approximately Gaussian population.",
    ));
}

fn populate_one_sample_w_sheet(
    res: &mut ResultSheet,
    label: &str,
    out: &crate::column::OneSampleWilcoxonResult,
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    let row_labels = vec![
        String::from("Hypothetical median"),
        String::from("Sample median"),
        String::from("W statistic (W+ - W-)"),
        String::from("P value"),
        String::from("Hodges-Lehmann pseudomedian"),
        format!("{}% CI of pseudomedian", conf.percent()),
    ];
    let rows = vec![
        vec![num_cell(out.hypothetical_median)],
        vec![num_cell(out.sample_median)],
        vec![num_cell(out.w_stat)],
        vec![p_cell(out.p_value, p_fmt)],
        vec![num_cell(out.pseudomedian_hl)],
        vec![opt_ci_cell(out.pseudomedian_ci)],
    ];
    res.tabs.push(build_tab("one-sample-w", "One-sample Wilcoxon test", ResultTabKind::MainSummary, vec![label.to_owned()], row_labels, rows));
    push_scalar(res, "PValue", None, out.p_value);
    res.checklist.push(checklist_item(
        "One-sample symmetry",
        ChecklistStatus::Info,
        Some(format!("n_nonzero = {}", out.n_nonzero)),
        "One-sample Wilcoxon test assumes observations are symmetrically distributed around the population median.",
    ));
}
