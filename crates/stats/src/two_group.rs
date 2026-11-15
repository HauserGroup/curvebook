//! Two-group parametric and nonparametric hypothesis tests, standardized effect sizes with
//! exact noncentral-*t* confidence intervals, variance homogeneity tests, and Bland–Altman agreement.

use lumencraft_doc::{
    AnalysisSpec, BlandAltmanSpec, Cell, CellValue, ChecklistItem, ChecklistStatus, CurveSamplePoint, DataSheet, FittedCurveOverlay,
    MultipleRowTestsSpec, MultipleVariablesTable, PValueFormat, PairwiseComparisonRecord, ResultCell, ResultSheet, ResultTabKind, TableArchetype,
    TwoGroupMethod, TwoGroupSpec, VariableRole,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{f_sf, norm_sf, norm_two_sided_isf, student_t_cdf, student_t_sf, student_t_two_sided_p};
use lumencraft_math::noncentral_t_ci;
use lumencraft_math::special::inc_beta::reg_beta;
use lumencraft_math::sum::{NeumaierSum, SummaryMoments, pearson_r};
use serde::{Deserialize, Serialize};

use crate::column::{
    OneSampleTResult, exact_binomial_median_ci, hedges_j_factor, hodges_lehmann_one_sample, ks_asymptotic_p, shapiro_wilk,
    summarize_pre_summarized_points, wilcoxon_exact_p,
};
use crate::common::{
    append_ignored_text_checklist_and_footnote, append_listwise_filtered_checklist_and_footnote, build_tab, checklist_item, ci_cell,
    extract_numeric_columns, extract_paired_values_with_report, init_result_sheet, is_ignored_non_numeric_cell, num_cell, opt_ci_cell, opt_num_cell,
    p_cell, push_scalar, quantile_type6_sorted, quartiles_hinge, rank_with_ties, resolve_selected_indices, sanitize_result_sheet_floats,
    select_kth_pairwise_sum, sorted_finite, t_crit_two_sided, validate_finite_slice,
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

fn compute_unpaired_effect_sizes_f(
    diff: f64,
    sds: (f64, f64, f64),
    n_pair: (f64, f64),
    t_df_conf: (f64, f64, ConfidenceLevel),
) -> Result<TwoGroupEffectSizes, StatsError> {
    let (sd_a, sd_b, sd_pooled) = sds;
    let (na, nb) = n_pair;
    let (t_stat, df_ci, confidence) = t_df_conf;
    let n_eff = ((na * nb) / (na + nb)).sqrt();
    let d = if sd_pooled > 0.0 { diff / sd_pooled } else { 0.0 };
    let df_pool = (na + nb - 2.0).max(1.0);
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
        let g_ci = noncentral_t_ci(t_glass, (na - 1.0).max(1.0), confidence.percent()).ok().map(|(lo, hi)| (lo / n_eff, hi / n_eff));
        (Some(gd), g_ci)
    } else if sd_b > 0.0 {
        let gd = diff / sd_b;
        let t_glass = gd * n_eff;
        let g_ci = noncentral_t_ci(t_glass, (nb - 1.0).max(1.0), confidence.percent()).ok().map(|(lo, hi)| (lo / n_eff, hi / n_eff));
        (Some(gd), g_ci)
    } else {
        (None, None)
    };

    let r_sq = if df_ci > 0.0 { (t_stat * t_stat) / (t_stat * t_stat + df_ci) } else { 0.0 };
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

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn build_unpaired_t_from_moments(
    welch: bool,
    a: (f64, f64, f64),
    b: (f64, f64, f64),
    two_tailed: bool,
    confidence: ConfidenceLevel,
    levene_median: LeveneTestResult,
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
pub fn unpaired_t_test_from_summary(
    group_a: (f64, f64, f64),
    group_b: (f64, f64, f64),
    two_tailed: bool,
    confidence: ConfidenceLevel,
) -> Result<UnpairedTResult, StatsError> {
    let placeholder_levene =
        LeveneTestResult { use_median: true, f_statistic: 0.0, df_between: 1.0, df_within: (group_a.2 + group_b.2 - 2.0).max(1.0), p_value: 1.0 };
    build_unpaired_t_from_moments(false, group_a, group_b, two_tailed, confidence, placeholder_levene)
}

/// Unpaired Welch *t*-test reconstructed directly from pre-summarized `(mean, sd, n)` moments.
pub fn welch_t_test_from_summary(
    group_a: (f64, f64, f64),
    group_b: (f64, f64, f64),
    two_tailed: bool,
    confidence: ConfidenceLevel,
) -> Result<UnpairedTResult, StatsError> {
    let placeholder_levene =
        LeveneTestResult { use_median: true, f_statistic: 0.0, df_between: 1.0, df_within: (group_a.2 + group_b.2 - 2.0).max(1.0), p_value: 1.0 };
    build_unpaired_t_from_moments(true, group_a, group_b, two_tailed, confidence, placeholder_levene)
}

/// One-sample Student *t*-test reconstructed directly from pre-summarized `(mean, sd, n)` moments.
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
    let j = hedges_j_factor(df)?;
    let g = j * d;
    let inv_sqrt_n = 1.0 / n.sqrt();
    let g_ci = noncentral_t_ci(t_stat, df, confidence.percent()).ok().map(|(lo, hi)| (lo * inv_sqrt_n * j, hi * inv_sqrt_n * j));

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
    } else if !ranked.tie_counts.is_empty() && na + nb <= 18 {
        (mann_whitney_exact_ties_p(&ranked.ranks, na, two_tailed), true)
    } else {
        let mean_u = 0.5 * naf * nbf;
        let var_u = ((naf * nbf / 12.0) * (nf + 1.0 - ranked.tie_sum_t3_minus_t / (nf * (nf - 1.0)))).max(1e-30);
        let sd_u = var_u.sqrt();
        let p = if two_tailed {
            let cc = (u_a - mean_u).abs() - 0.5;
            if cc <= 0.0 { 1.0 } else { (2.0 * norm_sf(cc / sd_u)).min(1.0) }
        } else {
            let cc = ((u_a - mean_u).abs() - 0.5).max(0.0);
            norm_sf(cc / sd_u).clamp(0.0, 1.0)
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
    if na == 0 || nb == 0 {
        return Err(StatsError::EmptyInput);
    }
    let (hl, lo_ci_val, hi_ci_val) = if m > 40_000 {
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
        if na + nb < 4 {
            return Ok((hl, None));
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
        (hl, ci_lo, ci_hi)
    } else {
        let mut diffs = Vec::with_capacity(m);
        for &va in a {
            for &vb in b {
                diffs.push(va - vb);
            }
        }
        diffs.sort_by(f64::total_cmp);
        let hl = quantile_type6_sorted(&diffs, 0.5).unwrap_or(diffs[0]);
        if !hl.is_finite() {
            return Err(StatsError::Math(lumencraft_math::MathError::DomainError { name: "hodges_lehmann", value: hl }));
        }
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
        (hl, diffs[lo_idx], diffs[hi_idx])
    };
    let ci = if lo_ci_val.is_finite() && hi_ci_val.is_finite() { Some((lo_ci_val, hi_ci_val)) } else { None };
    Ok((hl, ci))
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

/// Output of an exact binomial paired sign test on matched differences `a_i - b_i`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedSignTestResult {
    pub n_total: usize,
    pub n_positive: usize,
    pub n_negative: usize,
    pub n_ties: usize,
    pub n_nonzero: usize,
    pub median_difference: f64,
    pub ci: Option<(f64, f64)>,
    pub ci_actual_confidence_percent: Option<f64>,
    pub p_value: f64,
    pub two_tailed: bool,
}

/// Exact paired sign test on matched pairs `(a_i, b_i)` evaluating `d_i = a_i - b_i` after
/// dropping exact zero ties (`d_i == 0`), with exact order-statistic binomial CI for median `d`.
pub fn paired_sign_test(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<PairedSignTestResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.len() != b.len() {
        return Err(StatsError::DimensionMismatch { expected: a.len(), actual: b.len() });
    }
    let diffs: Vec<f64> = a.iter().zip(b.iter()).map(|(&x, &y)| x - y).collect();
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
    let median_difference = quartiles_hinge(&sorted).map_or(sorted[0], |(_, med, _)| med);
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
        let avg = if (a + b).is_finite() { 0.5 * (a + b) } else { 0.5 * a + 0.5 * b };
        let d = if percent_difference {
            if avg == 0.0 {
                return Err(StatsError::InvalidParameter { name: "average", value: 0.0 });
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
        if !avg.is_finite() || !d.is_finite() {
            return Err(StatsError::InvalidParameter { name: "bland_altman_overflow", value: if !avg.is_finite() { avg } else { d } });
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

fn extract_pre_summarized_group(
    col_tab: &lumencraft_doc::ColumnTable,
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
                let indices = resolve_selected_indices(&spec.selected_data_sets, col_tab.data_sets.len());
                let &first_idx = indices.first().ok_or(StatsError::EmptyInput)?;
                let (label, mean, sd, n) = extract_pre_summarized_group(col_tab, first_idx, conf)?;
                let out = one_sample_t_test_from_summary(mean, sd, n, hypothetical, tg.two_tailed, conf)?;
                populate_one_sample_t_sheet_impl(&mut res, &label, None, &out, conf, p_fmt);
                return Ok(res);
            }
            TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT => {
                let indices = resolve_selected_indices(&spec.selected_data_sets, col_tab.data_sets.len());
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

    let indices = resolve_selected_indices(&spec.selected_data_sets, sheet.archetype.column_count());
    if indices.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "two-group comparison columns", required: 2, actual: indices.len() });
    }
    let cols = extract_numeric_columns(sheet, &indices[..2])?;
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
            let out = wilcoxon_matched_pairs(&report.xs, &report.ys, tg.two_tailed, conf, false)?;
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
    let row_labels = vec![
        String::from("Difference mode"),
        String::from("N pairs"),
        String::from("Bias"),
        String::from("SD of bias"),
        String::from("SE of bias"),
        format!("{}% CI of bias", conf.percent()),
        format!("Lower {}% LoA", conf.percent()),
        format!("{}% CI of lower LoA", conf.percent()),
        format!("Upper {}% LoA", conf.percent()),
        format!("{}% CI of upper LoA", conf.percent()),
    ];
    let rows = vec![
        vec![ResultCell::Text(mode_text.to_owned())],
        vec![num_cell(out.n as f64)],
        vec![num_cell(out.bias)],
        vec![num_cell(out.sd_bias)],
        vec![num_cell(out.se_bias)],
        vec![ci_cell(out.bias_ci.0, out.bias_ci.1)],
        vec![num_cell(out.loa_lower)],
        vec![ci_cell(out.loa_lower_ci.0, out.loa_lower_ci.1)],
        vec![num_cell(out.loa_upper)],
        vec![ci_cell(out.loa_upper_ci.0, out.loa_upper_ci.1)],
    ];
    res.tabs.push(build_tab(
        "bland-altman-summary",
        "Bland-Altman method comparison",
        ResultTabKind::MainSummary,
        vec![format!("{lbl_a} vs. {lbl_b}")],
        row_labels,
        rows,
    ));

    let diff_header = if out.percent_difference { String::from("Percent Difference (%)") } else { String::from("Difference (A - B)") };
    let coord_headers = vec![String::from("Average (A + B) / 2"), diff_header, lbl_a.clone(), lbl_b.clone()];
    let mut coord_labels = Vec::with_capacity(out.n);
    let mut coord_rows = Vec::with_capacity(out.n);
    let mut overlay_pts = Vec::with_capacity(out.n);
    for (idx, ((&(avg, diff), &a), &b)) in out.points.iter().zip(vals_a.iter()).zip(vals_b.iter()).enumerate() {
        coord_labels.push(format!("Pair {}", idx + 1));
        coord_rows.push(vec![num_cell(avg), num_cell(diff), num_cell(a), num_cell(b)]);
        overlay_pts.push(CurveSamplePoint {
            x: avg,
            y: diff,
            ci_lower: Some(out.loa_lower),
            ci_upper: Some(out.loa_upper),
            pred_lower: None,
            pred_upper: None,
        });
    }
    res.tabs.push(build_tab(
        "bland-altman-coordinates",
        "Bland-Altman Pair Coordinates",
        ResultTabKind::Residuals,
        coord_headers,
        coord_labels,
        coord_rows,
    ));
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

    let (lbl_a, lbl_b, row_labels, group_a_rows, group_b_rows) = extract_multiple_row_inputs(sheet, spec, mr.paired)?;
    let proc_eff = MultiplicityProcedure::from(mr.multiplicity.method);
    let mut raw_p = Vec::new();
    let mut staged = Vec::new();
    let mut skipped_rows = 0_usize;

    for (idx, (ra, rb)) in group_a_rows.iter().zip(group_b_rows.iter()).enumerate() {
        let row_lbl = row_labels.get(idx).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", idx + 1));
        let row_out = run_single_row_comparison(ra, rb, mr, conf);
        match row_out {
            Ok((mean_a, mean_b, diff, stat, df, p_val)) => {
                raw_p.push(p_val);
                staged.push((idx, row_lbl, mean_a, mean_b, diff, stat, df, p_val));
            }
            Err(_) => {
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
            significant: adj <= threshold,
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

    let sum_labels = vec![
        String::from("Rows tested"),
        String::from("Rows skipped (insufficient data)"),
        String::from("Significant discoveries"),
        String::from("Test family"),
        String::from("Multiplicity method"),
        String::from("Significance / FDR threshold"),
        String::from("Minimum raw P"),
        String::from("Minimum adjusted P / q"),
    ];
    let sum_rows = vec![
        vec![num_cell(entries.len() as f64)],
        vec![num_cell(skipped_rows as f64)],
        vec![num_cell(discoveries as f64)],
        vec![ResultCell::Text(String::from(variant_label))],
        vec![ResultCell::Text(format!("{:?}", mr.multiplicity.method))],
        vec![num_cell(threshold)],
        vec![p_cell(min_raw_p, p_fmt)],
        vec![p_cell(min_adj_p, p_fmt)],
    ];
    res.tabs.push(build_tab(
        "multiple-row-summary",
        "Multiple Row t-Tests Summary",
        ResultTabKind::MainSummary,
        vec![format!("{lbl_a} vs. {lbl_b}")],
        sum_labels,
        sum_rows,
    ));

    let cmp_headers = vec![
        format!("Mean ({lbl_a})"),
        format!("Mean ({lbl_b})"),
        String::from("Difference (A - B)"),
        String::from("Statistic"),
        String::from("df"),
        String::from("Raw P"),
        String::from("Adjusted P / q"),
        String::from("Significant"),
    ];
    let mut cmp_labels = Vec::with_capacity(entries.len());
    let mut cmp_rows = Vec::with_capacity(entries.len());
    let pair_desc = format!("{lbl_a} vs. {lbl_b}");
    for e in &entries {
        cmp_labels.push(e.label.clone());
        cmp_rows.push(vec![
            num_cell(e.mean_a),
            num_cell(e.mean_b),
            num_cell(e.difference),
            num_cell(e.statistic),
            num_cell(e.df),
            p_cell(e.raw_p, p_fmt),
            p_cell(e.adjusted_p_or_q, p_fmt),
            ResultCell::Boolean(e.significant),
        ]);
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
    res.tabs.push(build_tab(
        "multiple-row-comparisons",
        "Multiple Row-Wise Comparisons",
        ResultTabKind::MultipleComparisons,
        cmp_headers,
        cmp_labels,
        cmp_rows,
    ));

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
    if let TableArchetype::MultipleVariables(mv) = &sheet.archetype
        && let Some(r_col) = mv_response
    {
        if r_col >= mv.variables.len() {
            return Err(StatsError::MissingColumn { index: r_col });
        }
        return Ok(vec![r_col]);
    }
    Ok(selected.to_vec())
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
    let pool = resolve_selected_indices(selected, mv.variables.len());
    let candidates: Vec<usize> = pool
        .into_iter()
        .filter(|&idx| Some(idx) != response_col && mv.variables.get(idx).is_some_and(|v| v.role == VariableRole::Categorical))
        .collect();
    match candidates.as_slice() {
        [only] => Ok(*only),
        _ => Err(StatsError::InvalidParameter { name: "mv_group_column", value: candidates.len() as f64 }),
    }
}

fn infer_mv_response_column(mv: &MultipleVariablesTable, group_col: usize, selected: &[usize]) -> Result<usize, StatsError> {
    let pool = resolve_selected_indices(selected, mv.variables.len());
    let candidates: Vec<usize> =
        pool.into_iter().filter(|&idx| idx != group_col && mv.variables.get(idx).is_some_and(|v| v.role != VariableRole::Categorical)).collect();
    match candidates.as_slice() {
        [only] => Ok(*only),
        _ => Err(StatsError::InvalidParameter { name: "mv_response_column", value: candidates.len() as f64 }),
    }
}

fn resolve_mv_two_group_inputs(sheet: &DataSheet, spec: &AnalysisSpec, tg: &TwoGroupSpec) -> Result<Option<MvTwoGroupExtraction>, StatsError> {
    let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
        return Ok(None);
    };
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
        let selected = resolve_selected_indices(&spec.selected_data_sets, mv.variables.len());
        let cat_cols: Vec<usize> = selected.iter().copied().filter(|&i| mv.variables[i].role == VariableRole::Categorical).collect();
        let num_cols: Vec<usize> = selected.iter().copied().filter(|&i| mv.variables[i].role != VariableRole::Categorical).collect();
        if cat_cols.len() == 1 && num_cols.len() == 1 {
            (cat_cols[0], num_cols[0])
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
            })
        }
        TableArchetype::Xy(xy) => {
            if xy.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            if xy.data_sets.len() >= 2 && ba.column_a != ba.column_b {
                let dsa = xy.data_sets.get(ba.column_a).ok_or(StatsError::MissingColumn { index: ba.column_a })?;
                let dsb = xy.data_sets.get(ba.column_b).ok_or(StatsError::MissingColumn { index: ba.column_b })?;
                let mut xs = Vec::new();
                let mut ys = Vec::new();
                let mut non_numeric_cells = 0_usize;
                let mut skipped_incomplete_rows = 0_usize;
                for r in 0..xy.row_count() {
                    let ca = dsa.cells.get(r).and_then(|s| s.first());
                    let cb = dsb.cells.get(r).and_then(|s| s.first());
                    if ca.is_some_and(is_ignored_non_numeric_cell) {
                        non_numeric_cells += 1;
                    }
                    if cb.is_some_and(is_ignored_non_numeric_cell) {
                        non_numeric_cells += 1;
                    }
                    let has_entry = ca.is_some_and(|c| !c.is_empty() || c.excluded) || cb.is_some_and(|c| !c.is_empty() || c.excluded);
                    if let (Some(a), Some(b)) = (ca.and_then(Cell::active_number), cb.and_then(Cell::active_number)) {
                        xs.push(a);
                        ys.push(b);
                    } else if has_entry {
                        skipped_incomplete_rows += 1;
                    }
                }
                if xs.is_empty() {
                    return Err(StatsError::EmptyInput);
                }
                Ok(BlandAltmanExtractedPairs {
                    label_a: dsa.title.clone(),
                    label_b: dsb.title.clone(),
                    xs,
                    ys,
                    skipped_incomplete_rows,
                    non_numeric_cells,
                })
            } else {
                let y_idx = if ba.column_a < xy.data_sets.len() { ba.column_a } else { 0 };
                let ds = xy.data_sets.get(y_idx).ok_or(StatsError::MissingColumn { index: y_idx })?;
                let pairs = xy.active_xy_pairs(y_idx);
                if pairs.is_empty() {
                    return Err(StatsError::EmptyInput);
                }
                let (xs, ys) = pairs.into_iter().unzip();
                Ok(BlandAltmanExtractedPairs {
                    label_a: xy.x_title.clone(),
                    label_b: ds.title.clone(),
                    xs,
                    ys,
                    skipped_incomplete_rows: 0,
                    non_numeric_cells: 0,
                })
            }
        }
        other => Err(StatsError::IncompatibleTable { analysis: "Bland-Altman", actual: other.kind() }),
    }
}

type MultipleRowInputs = (String, String, Vec<String>, Vec<Vec<f64>>, Vec<Vec<f64>>);

fn extract_multiple_row_inputs(sheet: &DataSheet, spec: &AnalysisSpec, paired: bool) -> Result<MultipleRowInputs, StatsError> {
    match &sheet.archetype {
        TableArchetype::Grouped(grp) => {
            if grp.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let indices = resolve_selected_indices(&spec.selected_data_sets, grp.data_sets.len());
            if indices.len() < 2 {
                return Err(StatsError::InsufficientSampleSize { context: "multiple row t-tests groups", required: 2, actual: indices.len() });
            }
            let (ia, ib) = (indices[0], indices[1]);
            let dsa = &grp.data_sets[ia];
            let dsb = &grp.data_sets[ib];
            let n_rows = grp.row_count();
            let mut labels = Vec::with_capacity(n_rows);
            let mut rows_a = Vec::with_capacity(n_rows);
            let mut rows_b = Vec::with_capacity(n_rows);
            for r in 0..n_rows {
                let lbl = grp.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1));
                let (va, vb) = extract_paired_or_unpaired_subcolumns(dsa.cells.get(r), dsb.cells.get(r), paired);
                labels.push(lbl);
                rows_a.push(va);
                rows_b.push(vb);
            }
            Ok((dsa.title.clone(), dsb.title.clone(), labels, rows_a, rows_b))
        }
        TableArchetype::Xy(xy) => {
            if xy.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let indices = resolve_selected_indices(&spec.selected_data_sets, xy.data_sets.len());
            if indices.len() < 2 {
                return Err(StatsError::InsufficientSampleSize { context: "multiple row t-tests groups", required: 2, actual: indices.len() });
            }
            let (ia, ib) = (indices[0], indices[1]);
            let dsa = &xy.data_sets[ia];
            let dsb = &xy.data_sets[ib];
            let n_rows = xy.row_count();
            let mut labels = Vec::with_capacity(n_rows);
            let mut rows_a = Vec::with_capacity(n_rows);
            let mut rows_b = Vec::with_capacity(n_rows);
            for r in 0..n_rows {
                let lbl = xy.x_value_at(r).map_or_else(|| format!("Row {}", r + 1), |x| format!("{x}"));
                let (va, vb) = extract_paired_or_unpaired_subcolumns(dsa.cells.get(r), dsb.cells.get(r), paired);
                labels.push(lbl);
                rows_a.push(va);
                rows_b.push(vb);
            }
            Ok((dsa.title.clone(), dsb.title.clone(), labels, rows_a, rows_b))
        }
        TableArchetype::Column(col) => {
            if col.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let indices = resolve_selected_indices(&spec.selected_data_sets, col.data_sets.len());
            if indices.len() < 4 {
                return Err(StatsError::InsufficientSampleSize {
                    context: "multiple row t-tests replicate columns per group",
                    required: 4,
                    actual: indices.len(),
                });
            }
            let mid = indices.len() / 2;
            let (left, right) = (&indices[..mid], &indices[mid..]);
            let n_rows = col.row_count();
            let mut labels = Vec::with_capacity(n_rows);
            let mut rows_a = Vec::with_capacity(n_rows);
            let mut rows_b = Vec::with_capacity(n_rows);
            for r in 0..n_rows {
                let lbl = col.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1));
                let (va, vb) = if paired {
                    let mut pa = Vec::new();
                    let mut pb = Vec::new();
                    for (&la, &rb_idx) in left.iter().zip(right.iter()) {
                        if let (Some(a), Some(b)) = (
                            col.data_sets[la].cell(r, 0).and_then(Cell::active_number),
                            col.data_sets[rb_idx].cell(r, 0).and_then(Cell::active_number),
                        ) {
                            pa.push(a);
                            pb.push(b);
                        }
                    }
                    (pa, pb)
                } else {
                    (
                        left.iter().filter_map(|&c| col.data_sets[c].cell(r, 0).and_then(Cell::active_number)).collect(),
                        right.iter().filter_map(|&c| col.data_sets[c].cell(r, 0).and_then(Cell::active_number)).collect(),
                    )
                };
                labels.push(lbl);
                rows_a.push(va);
                rows_b.push(vb);
            }
            Ok((String::from("Group A"), String::from("Group B"), labels, rows_a, rows_b))
        }
        TableArchetype::MultipleVariables(mv) => {
            let indices: Vec<usize> = resolve_selected_indices(&spec.selected_data_sets, mv.variables.len())
                .into_iter()
                .filter(|&i| mv.variables.get(i).is_some_and(|v| v.role != VariableRole::Categorical))
                .collect();
            if indices.len() < 4 {
                return Err(StatsError::InsufficientSampleSize {
                    context: "multiple row t-tests numeric replicate columns",
                    required: 4,
                    actual: indices.len(),
                });
            }
            let mid = indices.len() / 2;
            let (left, right) = (&indices[..mid], &indices[mid..]);
            let label_col = mv.variables.iter().position(|v| v.role == VariableRole::Categorical);
            let n_rows = mv.row_count();
            let mut labels = Vec::with_capacity(n_rows);
            let mut rows_a = Vec::with_capacity(n_rows);
            let mut rows_b = Vec::with_capacity(n_rows);
            for r in 0..n_rows {
                let lbl = label_col.and_then(|lc| mv.variables[lc].cells.get(r)).and_then(mv_cell_token).unwrap_or_else(|| format!("Row {}", r + 1));
                let (va, vb) = if paired {
                    let mut pa = Vec::new();
                    let mut pb = Vec::new();
                    for (&la, &rb_idx) in left.iter().zip(right.iter()) {
                        if let (Some(a), Some(b)) = (
                            mv.variables[la].cells.get(r).and_then(Cell::active_number),
                            mv.variables[rb_idx].cells.get(r).and_then(Cell::active_number),
                        ) {
                            pa.push(a);
                            pb.push(b);
                        }
                    }
                    (pa, pb)
                } else {
                    (
                        left.iter().filter_map(|&c| mv.variables[c].cells.get(r).and_then(Cell::active_number)).collect(),
                        right.iter().filter_map(|&c| mv.variables[c].cells.get(r).and_then(Cell::active_number)).collect(),
                    )
                };
                labels.push(lbl);
                rows_a.push(va);
                rows_b.push(vb);
            }
            Ok((String::from("Group A"), String::from("Group B"), labels, rows_a, rows_b))
        }
        other => Err(StatsError::IncompatibleTable { analysis: "Multiple row t-tests", actual: other.kind() }),
    }
}

fn extract_paired_or_unpaired_subcolumns(row_a: Option<&Vec<Cell>>, row_b: Option<&Vec<Cell>>, paired: bool) -> (Vec<f64>, Vec<f64>) {
    match (row_a, row_b) {
        (Some(sa), Some(sb)) if paired => {
            let mut va = Vec::new();
            let mut vb = Vec::new();
            for (ca, cb) in sa.iter().zip(sb.iter()) {
                if let (Some(a), Some(b)) = (ca.active_number(), cb.active_number()) {
                    va.push(a);
                    vb.push(b);
                }
            }
            (va, vb)
        }
        (Some(sa), Some(sb)) => (sa.iter().filter_map(Cell::active_number).collect(), sb.iter().filter_map(Cell::active_number).collect()),
        _ => (Vec::new(), Vec::new()),
    }
}

type SingleRowOutcome = (f64, f64, f64, f64, f64, f64);

fn run_single_row_comparison(ra: &[f64], rb: &[f64], mr: &MultipleRowTestsSpec, conf: ConfidenceLevel) -> Result<SingleRowOutcome, StatsError> {
    if mr.paired && mr.nonparametric {
        let out = wilcoxon_matched_pairs(ra, rb, true, conf, false)?;
        let ma = SummaryMoments::from_slice(ra)?.mean;
        let mb = SummaryMoments::from_slice(rb)?.mean;
        return Ok((ma, mb, out.hodges_lehmann_pseudomedian, out.w_stat, out.n_effective as f64, out.p_value));
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
        return Ok((ma, mb, out.hodges_lehmann_diff, out.u_min, (ra.len() + rb.len()) as f64 - 2.0, out.p_value));
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
    append_unpaired_checklist(&mut res.checklist, out, conf.alpha(), has_raw_replicates);
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
    } else if has_raw_replicates {
        checklist.push(checklist_item(
            "Variance homogeneity check",
            if out.method_label == "Welch's t test" && out.levene_median.p_value <= alpha { ChecklistStatus::Info } else { ChecklistStatus::Pass },
            Some(format!("Levene(median) F = {:.3}, p = {:.4}", out.levene_median.f_statistic, out.levene_median.p_value)),
            if out.method_label == "Welch's t test" && out.levene_median.p_value <= alpha {
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

fn populate_one_sample_t_sheet_impl(
    res: &mut ResultSheet,
    label: &str,
    values: Option<&[f64]>,
    out: &OneSampleTResult,
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
    let row_labels = vec![
        String::from("P value"),
        String::from("Median difference (A - B)"),
        format!("{}% CI of median difference", conf.percent()),
        String::from("Median CI actual confidence (%)"),
        String::from("Positive differences (n+)"),
        String::from("Negative differences (n-)"),
        String::from("Zero ties (n0)"),
        String::from("Effective pairs (n+ + n-)"),
    ];
    let rows = vec![
        vec![p_cell(out.p_value, p_fmt)],
        vec![num_cell(out.median_difference)],
        vec![opt_ci_cell(out.ci)],
        vec![opt_num_cell(out.ci_actual_confidence_percent)],
        vec![num_cell(out.n_positive as f64)],
        vec![num_cell(out.n_negative as f64)],
        vec![num_cell(out.n_ties as f64)],
        vec![num_cell(out.n_nonzero as f64)],
    ];
    res.tabs.push(build_tab(
        "paired-sign-test",
        "Paired sign test",
        ResultTabKind::MainSummary,
        vec![format!("{lbl_a} vs. {lbl_b}")],
        row_labels,
        rows,
    ));

    let pos_prop = if out.n_nonzero > 0 { Some((out.n_positive as f64) / (out.n_nonzero as f64)) } else { None };
    let diag_labels = vec![
        String::from("Total matched pairs"),
        String::from("Effective non-tie pairs"),
        String::from("Dropped exact zero ties"),
        String::from("Positive sign proportion"),
        String::from("Null binomial probability (p0)"),
    ];
    let diag_rows = vec![
        vec![num_cell(out.n_total as f64)],
        vec![num_cell(out.n_nonzero as f64)],
        vec![num_cell(out.n_ties as f64)],
        vec![opt_num_cell(pos_prop)],
        vec![num_cell(0.5)],
    ];
    res.tabs.push(build_tab(
        "paired-sign-diagnostics",
        "Paired sign test diagnostics",
        ResultTabKind::Diagnostics,
        vec![format!("{lbl_a} vs. {lbl_b}")],
        diag_labels,
        diag_rows,
    ));

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
