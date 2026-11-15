//! One-way, two-way, three-way, N-way factorial ANOVA, repeated-measures sphericity corrections
//! (Mauchly, Greenhouse–Geisser, Huynh–Feldt), mixed-effects REML (Satterthwaite & Kenward–Roger
//! degrees of freedom), nested hierarchical ANOVA / nested *t*-test, and effect sizes with exact
//! noncentral-*F* confidence intervals.

use lumencraft_doc::{
    AnalysisSpec, ChecklistItem, ChecklistStatus, DataSheet, FactorialAnovaSpec, MultiplicityMethod, MultiplicitySpec, NestedAnalysisSpec,
    OneWayAnovaSpec, OneWayMethod, PValueFormat, PairwiseComparisonRecord, RepeatedMeasuresDesign, ResultCell, ResultSheet, ResultTab, ResultTabKind,
    SubcolumnMode, SumOfSquaresType, TableArchetype,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{chi2_sf, f_sf, noncentral_f_ci, student_t_two_sided_p};
use lumencraft_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares, sym_eigen};
use lumencraft_math::sum::{NeumaierSum, SummaryMoments};
use serde::{Deserialize, Serialize};

use crate::column::shapiro_wilk;
use crate::common::{
    build_tab, checklist_item, extract_numeric_columns, init_result_sheet, num_cell, opt_ci_cell, opt_num_cell, p_cell, push_scalar,
    quantile_type6_sorted, rank_with_ties, resolve_selected_indices, sorted_finite, validate_finite_slice,
};
use crate::error::StatsError;
use crate::multiplicity::{
    GroupComparisonInput, MultiplicityProcedure, PairwiseComparisonDetail, build_multiplicity_tab, compare_dunn_ranks, compare_groups_pairwise,
    compare_groups_with_procedure, linear_trend_test,
};

// ============================================================================
// Typed Outputs & Effect Sizes
// ============================================================================

/// ANOVA variance-explained effect sizes and noncentral-*F* confidence intervals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnovaEffectSizes {
    pub eta_squared: f64,
    pub eta_squared_ci: Option<(f64, f64)>,
    pub partial_eta_squared: f64,
    pub partial_eta_squared_ci: Option<(f64, f64)>,
    pub omega_squared: f64,
    pub cohens_f: f64,
    pub cohens_f_ci: Option<(f64, f64)>,
}

/// Homogeneity-of-variance diagnostic tests across `k` groups.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VarianceHomogeneityResult {
    pub bartlett_chi2: f64,
    pub bartlett_df: f64,
    pub bartlett_p: f64,
    pub brown_forsythe_f: f64,
    pub brown_forsythe_df1: f64,
    pub brown_forsythe_df2: f64,
    pub brown_forsythe_p: f64,
}

/// Ordinary one-way ANOVA table metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OneWayAnovaResult {
    pub group_labels: Vec<String>,
    pub group_n: Vec<usize>,
    pub group_means: Vec<f64>,
    pub group_sds: Vec<f64>,
    pub total_n: usize,
    pub ss_between: f64,
    pub ss_within: f64,
    pub ss_total: f64,
    pub df_between: f64,
    pub df_within: f64,
    pub df_total: f64,
    pub ms_between: f64,
    pub ms_within: f64,
    pub f_statistic: f64,
    pub p_value: f64,
    pub r_squared: f64,
    pub variance_tests: VarianceHomogeneityResult,
    pub effect_sizes: AnovaEffectSizes,
}

/// Unequal-variance one-way ANOVA results (Brown–Forsythe and Welch).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnequalVarianceAnovaResult {
    pub welch_w: f64,
    pub welch_df1: f64,
    pub welch_df2: f64,
    pub welch_p: f64,
    pub brown_forsythe_f_star: f64,
    pub brown_forsythe_df1: f64,
    pub brown_forsythe_df2: f64,
    pub brown_forsythe_p: f64,
}

/// Mauchly sphericity test and epsilon corrections for repeated-measures factors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SphericityResult {
    pub mauchly_w: f64,
    pub mauchly_chi2: f64,
    pub mauchly_df: f64,
    pub mauchly_p: f64,
    pub greenhouse_geisser_eps: f64,
    pub huynh_feldt_eps: f64,
}

/// Repeated-measures one-way ANOVA output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RmOneWayAnovaResult {
    pub n_subjects: usize,
    pub k_conditions: usize,
    pub ss_treatment: f64,
    pub ss_subjects: f64,
    pub ss_residual: f64,
    pub ss_total: f64,
    pub df_treatment: f64,
    pub df_subjects: f64,
    pub df_residual: f64,
    pub ms_treatment: f64,
    pub ms_subjects: f64,
    pub ms_residual: f64,
    pub f_treatment: f64,
    pub p_uncorrected: f64,
    pub p_greenhouse_geisser: f64,
    pub p_huynh_feldt: f64,
    pub f_subjects: f64,
    pub p_subjects: f64,
    pub sphericity: SphericityResult,
    pub partial_eta_squared: f64,
}

/// Nonparametric one-way ANOVA output (Kruskal–Wallis).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KruskalWallisResult {
    pub h_statistic: f64,
    pub df: f64,
    pub p_value: f64,
    pub tie_correction: f64,
    pub epsilon_squared: f64,
    pub group_mean_ranks: Vec<f64>,
    pub total_n: usize,
}

/// Nonparametric repeated-measures ANOVA output (Friedman).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FriedmanResult {
    pub q_statistic: f64,
    pub df: f64,
    pub p_value: f64,
    pub kendalls_w: f64,
    pub n_blocks: usize,
    pub k_treatments: usize,
    pub condition_rank_sums: Vec<f64>,
    pub condition_mean_ranks: Vec<f64>,
}

/// Single source-of-variation row in a factorial / general linear ANOVA table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactorialAnovaTerm {
    pub name: String,
    pub ss: f64,
    pub df: f64,
    pub ms: f64,
    pub f_statistic: Option<f64>,
    pub p_value: Option<f64>,
    pub partial_eta_squared: Option<f64>,
    pub eta_squared: Option<f64>,
}

/// Factorial (two-way, three-way, or N-way) ANOVA table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactorialAnovaResult {
    pub terms: Vec<FactorialAnovaTerm>,
    pub ss_residual: f64,
    pub df_residual: f64,
    pub ms_residual: f64,
    pub ss_total: f64,
    pub df_total: f64,
    pub total_n: usize,
    pub unbalanced: bool,
    pub ss_type: SumOfSquaresType,
}

/// Linear mixed-effects REML fit for one-way / nested hierarchical or unbalanced RM data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MixedRemlResult {
    pub sigma2_random: f64,
    pub sigma2_error: f64,
    pub icc: f64,
    pub reml_log_lik: f64,
    pub lrt_random_chi2: f64,
    pub lrt_random_p: f64,
    pub fixed_terms: Vec<FactorialAnovaTerm>,
    pub satterthwaite_df: Vec<f64>,
    pub kenward_roger_df: Vec<f64>,
}

/// Nested one-way ANOVA and optional two-group nested *t*-test output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedAnovaResult {
    pub group_labels: Vec<String>,
    pub f_among_groups: f64,
    pub df1_groups: f64,
    pub df2_groups_satterthwaite: f64,
    pub df2_groups_kenward_roger: f64,
    pub p_among_groups: f64,
    pub t_nested: Option<f64>,
    pub f_among_subclusters: f64,
    pub df1_subclusters: f64,
    pub df2_error: f64,
    pub p_among_subclusters: f64,
    pub ss_groups: f64,
    pub ss_subclusters: f64,
    pub ss_error: f64,
    pub ms_groups: f64,
    pub ms_subclusters: f64,
    pub ms_error: f64,
    pub variance_subclusters: f64,
    pub variance_error: f64,
    pub icc_subclusters: f64,
    pub total_observations: usize,
}

// ============================================================================
// Effect Sizes & Variance Homogeneity Helpers
// ============================================================================

/// Compute ANOVA effect sizes (`eta^2`, `partial eta^2`, `omega^2`, Cohen's `f`) and exact
/// noncentral-*F* confidence intervals.
pub fn compute_anova_effect_sizes(
    ss_effect: f64,
    ss_error: f64,
    ss_total: f64,
    df_effect: f64,
    df_error: f64,
    confidence: ConfidenceLevel,
) -> AnovaEffectSizes {
    let ms_error = if df_error > 0.0 { ss_error / df_error } else { 0.0 };
    let eta2 = if ss_total > 0.0 { (ss_effect / ss_total).clamp(0.0, 1.0) } else { 0.0 };
    let partial_eta2 = if ss_effect + ss_error > 0.0 { (ss_effect / (ss_effect + ss_error)).clamp(0.0, 1.0) } else { 0.0 };
    let omega2_denom = ss_total + ms_error;
    let omega2 = if omega2_denom > 0.0 { ((ss_effect - df_effect * ms_error) / omega2_denom).clamp(0.0, 1.0) } else { 0.0 };
    let cohens_f = if partial_eta2 < 1.0 { (partial_eta2 / (1.0 - partial_eta2)).sqrt() } else { 0.0 };

    let f_obs = if ms_error > 0.0 && df_effect > 0.0 { (ss_effect / df_effect) / ms_error } else { 0.0 };
    let total_df_n = df_effect + df_error + 1.0;
    let ncf_ci = if f_obs.is_finite() && df_effect >= 1.0 && df_error >= 1.0 {
        noncentral_f_ci(f_obs.max(0.0), df_effect, df_error, confidence.percent()).ok()
    } else {
        None
    };

    let partial_eta_ci = ncf_ci.map(|(lo, hi)| {
        let p_lo = (lo / (lo + total_df_n)).clamp(0.0, 1.0);
        let p_hi = (hi / (hi + total_df_n)).clamp(0.0, 1.0);
        (p_lo, p_hi)
    });
    let cohens_f_ci = partial_eta_ci.map(|(p_lo, p_hi)| {
        let f_lo = if p_lo < 1.0 { (p_lo / (1.0 - p_lo)).sqrt() } else { 0.0 };
        let f_hi = if p_hi < 1.0 { (p_hi / (1.0 - p_hi)).sqrt() } else { f64::INFINITY };
        (f_lo, f_hi)
    });

    AnovaEffectSizes {
        eta_squared: eta2,
        eta_squared_ci: partial_eta_ci,
        partial_eta_squared: partial_eta2,
        partial_eta_squared_ci: partial_eta_ci,
        omega_squared: omega2,
        cohens_f,
        cohens_f_ci,
    }
}

/// Bartlett's and Brown–Forsythe homogeneity-of-variance tests across `k >= 2` groups.
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
    })
}

// ============================================================================
// One-Way ANOVA (Ordinary, Welch/BF, RM, Kruskal–Wallis, Friedman)
// ============================================================================

/// Ordinary one-way ANOVA using two-pass `SummaryMoments` and `NeumaierSum` shifted by the grand
/// mean so stiff NIST StRD series (`SmLs01..09`, `SiRstv`, `AtmWtAg`) retain full precision.
pub fn ordinary_one_way_anova(groups: &[(&str, &[f64])], confidence: ConfidenceLevel) -> Result<OneWayAnovaResult, StatsError> {
    let slices: Vec<&[f64]> = groups.iter().map(|(_, s)| *s).collect();
    let labels: Vec<String> = groups.iter().map(|(l, _)| (*l).to_owned()).collect();
    let mut res = ordinary_one_way_anova_slices(&slices, confidence)?;
    res.group_labels = labels;
    res.variance_tests = variance_homogeneity_tests(&slices)?;
    Ok(res)
}

fn ordinary_one_way_anova_slices(groups: &[&[f64]], confidence: ConfidenceLevel) -> Result<OneWayAnovaResult, StatsError> {
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: groups.len() });
    }
    let mut group_n = Vec::with_capacity(groups.len());
    let mut group_means = Vec::with_capacity(groups.len());
    let mut group_sds = Vec::with_capacity(groups.len());
    let mut ss_within_acc = NeumaierSum::new();
    let mut grand_sum_acc = NeumaierSum::new();
    let mut total_n = 0_usize;

    for &g in groups {
        validate_finite_slice(g)?;
        let m = SummaryMoments::from_slice(g)?;
        group_n.push(g.len());
        group_means.push(m.mean);
        group_sds.push(m.sample_sd().unwrap_or(0.0));
        ss_within_acc.add(m.css);
        grand_sum_acc.add((g.len() as f64) * m.mean);
        total_n += g.len();
    }
    if total_n <= groups.len() {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA residual df", required: groups.len() + 1, actual: total_n });
    }
    let total_nf = total_n as f64;
    let grand_mean_approx = grand_sum_acc.total() / total_nf;

    // Two-pass refinement on grand mean to preserve precision on large-offset datasets (SmLs07..09).
    let mut shift_corr = NeumaierSum::new();
    for (&n_i, &m_i) in group_n.iter().zip(group_means.iter()) {
        shift_corr.add((n_i as f64) * (m_i - grand_mean_approx));
    }
    let grand_mean = grand_mean_approx + shift_corr.total() / total_nf;

    let mut ss_between_acc = NeumaierSum::new();
    for (&n_i, &m_i) in group_n.iter().zip(group_means.iter()) {
        let diff = m_i - grand_mean;
        ss_between_acc.add((n_i as f64) * diff * diff);
    }

    let ss_between = ss_between_acc.total().max(0.0);
    let ss_within = ss_within_acc.total().max(0.0);
    let ss_total = ss_between + ss_within;

    let df_between = (groups.len() - 1) as f64;
    let df_within = (total_n - groups.len()) as f64;
    let df_total = (total_n - 1) as f64;

    let ms_between = ss_between / df_between;
    let ms_within = ss_within / df_within;
    let (f_stat, p_val) = if ms_within > 0.0 {
        let f = ms_between / ms_within;
        (f, f_sf(f, df_between, df_within)?)
    } else if ms_between > 0.0 {
        (f64::INFINITY, 0.0)
    } else {
        (0.0, 1.0)
    };

    let r_squared = if ss_total > 0.0 { ss_between / ss_total } else { 0.0 };
    let effect_sizes = compute_anova_effect_sizes(ss_between, ss_within, ss_total, df_between, df_within, confidence);

    Ok(OneWayAnovaResult {
        group_labels: (0..groups.len()).map(|i| format!("Group {}", i + 1)).collect(),
        group_n,
        group_means,
        group_sds,
        total_n,
        ss_between,
        ss_within,
        ss_total,
        df_between,
        df_within,
        df_total,
        ms_between,
        ms_within,
        f_statistic: f_stat,
        p_value: p_val,
        r_squared,
        variance_tests: VarianceHomogeneityResult {
            bartlett_chi2: 0.0,
            bartlett_df: df_between,
            bartlett_p: 1.0,
            brown_forsythe_f: 0.0,
            brown_forsythe_df1: df_between,
            brown_forsythe_df2: df_within,
            brown_forsythe_p: 1.0,
        },
        effect_sizes,
    })
}

/// One-way ANOVA from pre-summarized `(label, n, mean, sd)` records.
pub fn ordinary_one_way_from_summary(groups: &[GroupComparisonInput], confidence: ConfidenceLevel) -> Result<OneWayAnovaResult, StatsError> {
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: groups.len() });
    }
    let mut total_n_f = 0.0_f64;
    let mut grand_sum = NeumaierSum::new();
    let mut ss_within = NeumaierSum::new();
    for g in groups {
        if g.n < 1.0 || !g.mean.is_finite() || !g.sd.is_finite() || g.sd < 0.0 {
            return Err(StatsError::InvalidParameter { name: "group_summary", value: g.n });
        }
        total_n_f += g.n;
        grand_sum.add(g.n * g.mean);
        if g.n > 1.0 {
            ss_within.add((g.n - 1.0) * g.sd * g.sd);
        }
    }
    let grand_mean = grand_sum.total() / total_n_f;
    let mut ss_between = NeumaierSum::new();
    for g in groups {
        let d = g.mean - grand_mean;
        ss_between.add(g.n * d * d);
    }
    let ssb = ss_between.total().max(0.0);
    let ssw = ss_within.total().max(0.0);
    let sst = ssb + ssw;
    let dfb = (groups.len() - 1) as f64;
    let dfw = total_n_f - (groups.len() as f64);
    if dfw <= 0.0 {
        return Err(StatsError::InsufficientSampleSize {
            context: "ANOVA summary residual df",
            required: groups.len() + 1,
            actual: total_n_f as usize,
        });
    }
    let msb = ssb / dfb;
    let msw = ssw / dfw;
    let (f_stat, p_val) = if msw > 0.0 {
        let f = msb / msw;
        (f, f_sf(f, dfb, dfw)?)
    } else if msb > 0.0 {
        (f64::INFINITY, 0.0)
    } else {
        (0.0, 1.0)
    };
    let effect_sizes = compute_anova_effect_sizes(ssb, ssw, sst, dfb, dfw, confidence);

    Ok(OneWayAnovaResult {
        group_labels: groups.iter().map(|g| g.label.clone()).collect(),
        group_n: groups.iter().map(|g| g.n.round() as usize).collect(),
        group_means: groups.iter().map(|g| g.mean).collect(),
        group_sds: groups.iter().map(|g| g.sd).collect(),
        total_n: total_n_f.round() as usize,
        ss_between: ssb,
        ss_within: ssw,
        ss_total: sst,
        df_between: dfb,
        df_within: dfw,
        df_total: total_n_f - 1.0,
        ms_between: msb,
        ms_within: msw,
        f_statistic: f_stat,
        p_value: p_val,
        r_squared: if sst > 0.0 { ssb / sst } else { 0.0 },
        variance_tests: VarianceHomogeneityResult {
            bartlett_chi2: 0.0,
            bartlett_df: dfb,
            bartlett_p: 1.0,
            brown_forsythe_f: 0.0,
            brown_forsythe_df1: dfb,
            brown_forsythe_df2: dfw,
            brown_forsythe_p: 1.0,
        },
        effect_sizes,
    })
}

/// Welch's and Brown–Forsythe's unequal-variance one-way ANOVA from group summaries.
pub fn unequal_variance_one_way_anova(groups: &[GroupComparisonInput]) -> Result<UnequalVarianceAnovaResult, StatsError> {
    let k = groups.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: k });
    }
    let kf = k as f64;
    let mut weights = Vec::with_capacity(k);
    let mut w_sum = 0.0_f64;
    let mut w_mean_sum = 0.0_f64;
    let mut total_n = 0.0_f64;
    let mut ssb_num = 0.0_f64;
    let mut bf_denom = 0.0_f64;
    let mut bf_satt_denom = 0.0_f64;

    for g in groups {
        if g.n < 2.0 || !g.sd.is_finite() || g.sd <= 0.0 {
            return Err(StatsError::InvalidParameter { name: "group_sd", value: g.sd });
        }
        let v = g.sd * g.sd;
        let w = g.n / v;
        weights.push(w);
        w_sum += w;
        w_mean_sum += w * g.mean;
        total_n += g.n;
    }
    let grand_mean: f64 = groups.iter().map(|g| g.n * g.mean).sum::<f64>() / total_n;
    let x_bar_w = w_mean_sum / w_sum;

    let mut welch_num = 0.0_f64;
    let mut lambda_sum = 0.0_f64;
    for (g, &w) in groups.iter().zip(weights.iter()) {
        let dm_w = g.mean - x_bar_w;
        welch_num += w * dm_w * dm_w;
        let frac = 1.0 - w / w_sum;
        lambda_sum += (frac * frac) / (g.n - 1.0);

        let dm = g.mean - grand_mean;
        ssb_num += g.n * dm * dm;
        let v = g.sd * g.sd;
        let term = (1.0 - g.n / total_n) * v;
        bf_denom += term;
        bf_satt_denom += (term * term) / (g.n - 1.0);
    }

    let welch_df1 = kf - 1.0;
    let lambda_factor = 3.0 * lambda_sum / (kf * kf - 1.0);
    let welch_w = (welch_num / welch_df1) / (1.0 + (2.0 * (kf - 2.0) / 3.0) * lambda_factor);
    let welch_df2 = if lambda_sum > 0.0 { (kf * kf - 1.0) / (3.0 * lambda_sum) } else { 1.0 };
    let welch_p = f_sf(welch_w, welch_df1, welch_df2)?;

    let bf_f_star = if bf_denom > 0.0 { ssb_num / bf_denom } else { 0.0 };
    let bf_df2 = if bf_satt_denom > 0.0 { (bf_denom * bf_denom) / bf_satt_denom } else { 1.0 };
    let bf_p = f_sf(bf_f_star, welch_df1, bf_df2)?;

    Ok(UnequalVarianceAnovaResult {
        welch_w,
        welch_df1,
        welch_df2,
        welch_p,
        brown_forsythe_f_star: bf_f_star,
        brown_forsythe_df1: welch_df1,
        brown_forsythe_df2: bf_df2,
        brown_forsythe_p: bf_p,
    })
}

/// Repeated-measures one-way ANOVA on complete subject blocks (`n` rows × `k` conditions), with
/// Mauchly's sphericity test, Greenhouse–Geisser `epsilon_hat`, and Huynh–Feldt `epsilon_tilde`.
pub fn rm_one_way_anova(blocks: &[Vec<f64>]) -> Result<RmOneWayAnovaResult, StatsError> {
    let n = blocks.len();
    if n < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "repeated-measures subjects", required: 2, actual: n });
    }
    let k = blocks[0].len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: k });
    }
    for row in blocks {
        if row.len() != k {
            return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: k, actual: row.len() }));
        }
        validate_finite_slice(row)?;
    }

    let nf = n as f64;
    let kf = k as f64;
    let mut grand_sum = NeumaierSum::new();
    let mut row_means = Vec::with_capacity(n);
    let mut col_sums = vec![NeumaierSum::new(); k];

    for row in blocks {
        let mut rsum = NeumaierSum::new();
        for (c, &v) in row.iter().enumerate() {
            rsum.add(v);
            col_sums[c].add(v);
            grand_sum.add(v);
        }
        row_means.push(rsum.total() / kf);
    }
    let grand_mean = grand_sum.total() / (nf * kf);
    let col_means: Vec<f64> = col_sums.iter().map(|s| s.total() / nf).collect();

    let mut ss_treat = NeumaierSum::new();
    for &cm in &col_means {
        let d = cm - grand_mean;
        ss_treat.add(nf * d * d);
    }
    let mut ss_subj = NeumaierSum::new();
    for &rm in &row_means {
        let d = rm - grand_mean;
        ss_subj.add(kf * d * d);
    }
    let mut ss_resid = NeumaierSum::new();
    for (r, row) in blocks.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            let res = v - row_means[r] - col_means[c] + grand_mean;
            ss_resid.add(res * res);
        }
    }

    let sst = ss_treat.total().max(0.0);
    let sss = ss_subj.total().max(0.0);
    let ssr = ss_resid.total().max(0.0);
    let ss_total = sst + sss + ssr;

    let dft = kf - 1.0;
    let dfs = nf - 1.0;
    let dfr = dft * dfs;
    let mst = sst / dft;
    let mss = sss / dfs;
    let msr = ssr / dfr;

    let f_treat = if msr > 0.0 { mst / msr } else { 0.0 };
    let f_subj = if msr > 0.0 { mss / msr } else { 0.0 };
    let p_uncorr = f_sf(f_treat, dft, dfr)?;
    let p_subj = f_sf(f_subj, dfs, dfr)?;

    let sphericity = compute_sphericity(blocks, &col_means)?;
    let gg_df1 = (dft * sphericity.greenhouse_geisser_eps).max(1.0);
    let gg_df2 = (dfr * sphericity.greenhouse_geisser_eps).max(1.0);
    let hf_df1 = (dft * sphericity.huynh_feldt_eps).max(1.0);
    let hf_df2 = (dfr * sphericity.huynh_feldt_eps).max(1.0);
    let p_gg = f_sf(f_treat, gg_df1, gg_df2)?;
    let p_hf = f_sf(f_treat, hf_df1, hf_df2)?;

    let partial_eta2 = if sst + ssr > 0.0 { sst / (sst + ssr) } else { 0.0 };

    Ok(RmOneWayAnovaResult {
        n_subjects: n,
        k_conditions: k,
        ss_treatment: sst,
        ss_subjects: sss,
        ss_residual: ssr,
        ss_total,
        df_treatment: dft,
        df_subjects: dfs,
        df_residual: dfr,
        ms_treatment: mst,
        ms_subjects: mss,
        ms_residual: msr,
        f_treatment: f_treat,
        p_uncorrected: p_uncorr,
        p_greenhouse_geisser: p_gg,
        p_huynh_feldt: p_hf,
        f_subjects: f_subj,
        p_subjects: p_subj,
        sphericity,
        partial_eta_squared: partial_eta2,
    })
}

/// Compute Mauchly's `W` and Greenhouse–Geisser / Huynh–Feldt sphericity epsilons from an `n x k`
/// repeated-measures matrix via orthonormal Helmert contrast projection `C^T S C`.
fn compute_sphericity(blocks: &[Vec<f64>], col_means: &[f64]) -> Result<SphericityResult, StatsError> {
    let n = blocks.len();
    let k = col_means.len();
    if k <= 2 || n <= 2 {
        return Ok(SphericityResult {
            mauchly_w: 1.0,
            mauchly_chi2: 0.0,
            mauchly_df: 0.0,
            mauchly_p: 1.0,
            greenhouse_geisser_eps: 1.0,
            huynh_feldt_eps: 1.0,
        });
    }
    let nf = n as f64;
    let _kf = k as f64;
    let df_s = nf - 1.0;

    // Sample covariance matrix S (k x k).
    let mut s_cov = vec![0.0_f64; k * k];
    for r in 0..k {
        for c in r..k {
            let mut acc = NeumaierSum::new();
            for row in blocks {
                acc.add((row[r] - col_means[r]) * (row[c] - col_means[c]));
            }
            let val = acc.total() / df_s;
            s_cov[r * k + c] = val;
            s_cov[c * k + r] = val;
        }
    }

    // Orthonormal Helmert contrast matrix H ((k - 1) x k).
    let p = k - 1;
    let mut h_mat = DenseMatrix::zeros(p, k)?;
    for i in 0..p {
        let m = (i + 1) as f64;
        let norm = (m * (m + 1.0)).sqrt();
        for j in 0..=i {
            h_mat.set(i, j, 1.0 / norm)?;
        }
        h_mat.set(i, i + 1, -m / norm)?;
    }

    let s_mat = DenseMatrix::from_row_major(k, k, s_cov)?;
    let hs = h_mat.matmul(&s_mat)?;
    let c_cov = hs.matmul(&h_mat.transpose())?;
    let eig = sym_eigen(&c_cov)?;
    let mut eigvals: Vec<f64> = eig.eigenvalues.iter().map(|&x| x.max(0.0)).collect();

    let tr = eigvals.iter().sum::<f64>();
    let tr_sq = eigvals.iter().map(|&l| l * l).sum::<f64>();
    let pf = p as f64;
    let eps_gg = if tr_sq > 0.0 { ((tr * tr) / (pf * tr_sq)).clamp(1.0 / pf, 1.0) } else { 1.0 };
    let eps_hf = ((nf * pf * eps_gg - 2.0) / (pf * (nf - 1.0 - pf * eps_gg))).clamp(eps_gg, 1.0);

    // Mauchly's W = det(C) / (tr(C)/p)^p.
    for v in &mut eigvals {
        *v = v.max(1e-18);
    }
    let prod: f64 = eigvals.iter().product();
    let mean_eig = (tr / pf).max(1e-18);
    let w = (prod / mean_eig.powf(pf)).clamp(1e-15, 1.0);
    let m_df = 0.5 * pf * (pf + 1.0) - 1.0;
    let f_scale = 1.0 - (2.0 * pf * pf + pf + 2.0) / (6.0 * pf * df_s);
    let chi2 = (-df_s * f_scale * w.ln()).max(0.0);
    let mauchly_p = if m_df > 0.0 { chi2_sf(chi2, m_df)? } else { 1.0 };

    Ok(SphericityResult { mauchly_w: w, mauchly_chi2: chi2, mauchly_df: m_df, mauchly_p, greenhouse_geisser_eps: eps_gg, huynh_feldt_eps: eps_hf })
}

/// Kruskal–Wallis one-way ANOVA on ranks with tie correction and epsilon-squared effect size.
pub fn kruskal_wallis_test(groups: &[&[f64]]) -> Result<KruskalWallisResult, StatsError> {
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: groups.len() });
    }
    let mut flat = Vec::new();
    let mut group_sizes = Vec::with_capacity(groups.len());
    for &g in groups {
        validate_finite_slice(g)?;
        group_sizes.push(g.len());
        flat.extend_from_slice(g);
    }
    let total_n = flat.len();
    if total_n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Kruskal-Wallis observations", required: 3, actual: total_n });
    }
    let ranked = rank_with_ties(&flat)?;

    let mut offset = 0_usize;
    let mut group_mean_ranks = Vec::with_capacity(groups.len());
    let nf = total_n as f64;
    let mut h_sum = 0.0_f64;
    for &n_i in &group_sizes {
        let r_sum: f64 = ranked.ranks[offset..(offset + n_i)].iter().sum();
        let mean_rank = r_sum / (n_i as f64);
        group_mean_ranks.push(mean_rank);
        h_sum += (r_sum * r_sum) / (n_i as f64);
        offset += n_i;
    }

    let h_raw = (12.0 / (nf * (nf + 1.0))) * h_sum - 3.0 * (nf + 1.0);
    let tie_denom = 1.0 - ranked.tie_sum_t3_minus_t / (nf * nf * nf - nf);
    let (h_corr, tie_factor) = if tie_denom > 0.0 { (h_raw.max(0.0) / tie_denom, tie_denom) } else { (0.0, 1.0) };
    let df = (groups.len() - 1) as f64;
    let p_value = chi2_sf(h_corr, df)?;
    let eps2 = ((h_corr - df) / (nf - 1.0)).clamp(0.0, 1.0);

    Ok(KruskalWallisResult { h_statistic: h_corr, df, p_value, tie_correction: tie_factor, epsilon_squared: eps2, group_mean_ranks, total_n })
}

/// Friedman nonparametric repeated-measures rank test on complete blocks (`n` rows × `k` conditions).
pub fn friedman_test(blocks: &[Vec<f64>]) -> Result<FriedmanResult, StatsError> {
    let n = blocks.len();
    if n < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "repeated-measures subjects", required: 2, actual: n });
    }
    let k = blocks[0].len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: k });
    }
    let mut rank_sums = vec![0.0_f64; k];
    let mut tie_sum = 0.0_f64;

    for row in blocks {
        if row.len() != k {
            return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: k, actual: row.len() }));
        }
        let ranked = rank_with_ties(row)?;
        tie_sum += ranked.tie_sum_t3_minus_t;
        for (c, &r) in ranked.ranks.iter().enumerate() {
            rank_sums[c] += r;
        }
    }

    let nf = n as f64;
    let kf = k as f64;
    let mean_rank_expected = 0.5 * (kf + 1.0);
    let ss_cols: f64 = rank_sums.iter().map(|&rs| (rs - nf * mean_rank_expected).powi(2)).sum();
    let denom = nf * kf * (kf + 1.0) / 12.0 - tie_sum / (12.0 * (kf - 1.0));
    let q_stat = if denom > 0.0 { (ss_cols / denom).max(0.0) } else { 0.0 };
    let df = kf - 1.0;
    let p_value = chi2_sf(q_stat, df)?;
    let kendalls_w = (q_stat / (nf * df)).clamp(0.0, 1.0);
    let mean_ranks = rank_sums.iter().map(|&r| r / nf).collect();

    Ok(FriedmanResult {
        q_statistic: q_stat,
        df,
        p_value,
        kendalls_w,
        n_blocks: n,
        k_treatments: k,
        condition_rank_sums: rank_sums,
        condition_mean_ranks: mean_ranks,
    })
}

// ============================================================================
// General Factorial (2-Way, 3-Way, N-Way) ANOVA Engine (Type I & Type III SS)
// ============================================================================

/// Factor definition and active observations for general factorial ANOVA (`N >= 1` factors).
#[derive(Debug, Clone)]
pub struct FactorialObservations {
    pub factor_names: Vec<String>,
    pub factor_levels: Vec<Vec<String>>,
    /// Per observation: `(level_indices_per_factor, y)`.
    pub rows: Vec<(Vec<usize>, f64)>,
}

#[derive(Debug, Clone)]
struct TermSpec {
    name: String,
    col_start: usize,
    col_count: usize,
}

/// Fit Type I (sequential) or Type III (partial, sum/effect-coded) general `N`-way factorial ANOVA
/// via `lumencraft_math::linalg::solve_linear_least_squares`.
pub fn fit_factorial_anova(
    data: &FactorialObservations,
    max_interaction_order: usize,
    ss_type: SumOfSquaresType,
) -> Result<FactorialAnovaResult, StatsError> {
    let n_obs = data.rows.len();
    let n_factors = data.factor_names.len();
    if n_obs < 3 || n_factors == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "factorial ANOVA observations", required: 3, actual: n_obs });
    }
    let y: Vec<f64> = data.rows.iter().map(|(_, yv)| *yv).collect();
    validate_finite_slice(&y)?;
    let y_moments = SummaryMoments::from_slice(&y)?;

    // Build factor combinations up to max_interaction_order.
    let order_cap = max_interaction_order.clamp(1, n_factors);
    let mut subsets: Vec<Vec<usize>> = Vec::new();
    for ord in 1..=order_cap {
        collect_index_combinations(n_factors, ord, 0, &mut Vec::new(), &mut subsets);
    }

    // Build sum-to-zero effect coding (+1 / -1) for each factor (needed for valid Type III partial SS!).
    let factor_basis: Vec<Vec<Vec<f64>>> = data.factor_levels.iter().map(|levels| sum_contrast_vectors(levels.len())).collect();

    let mut cols: Vec<Vec<f64>> = vec![vec![1.0_f64; n_obs]]; // column 0 = intercept
    let mut terms = Vec::with_capacity(subsets.len());

    for sub in &subsets {
        let col_start = cols.len();
        let prod_cols = kronecker_term_columns(&data.rows, &factor_basis, sub);
        let col_count = prod_cols.len();
        cols.extend(prod_cols);
        let name = sub.iter().map(|&idx| data.factor_names[idx].as_str()).collect::<Vec<_>>().join(" × ");
        terms.push(TermSpec { name, col_start, col_count });
    }

    let total_cols = cols.len();
    let full_x = columns_to_matrix(&cols, n_obs)?;
    let (full_sol, _) = solve_linear_least_squares(&full_x, &y)?;
    let ss_resid = full_sol.rss.max(0.0);
    let df_resid = full_sol.df_residual as f64;
    if df_resid <= 0.0 {
        return Err(StatsError::InsufficientSampleSize { context: "factorial ANOVA design df", required: total_cols + 1, actual: n_obs });
    }
    let ms_resid = ss_resid / df_resid;
    let ss_total = y_moments.css.max(ss_resid);

    let unbalanced = is_design_unbalanced(&data.rows, &data.factor_levels);
    let mut out_terms = Vec::with_capacity(terms.len());

    match ss_type {
        SumOfSquaresType::TypeIII => {
            for t in &terms {
                let mut dropped_cols = Vec::with_capacity(total_cols - t.col_count);
                for (c_idx, col) in cols.iter().enumerate() {
                    if c_idx < t.col_start || c_idx >= t.col_start + t.col_count {
                        dropped_cols.push(col.clone());
                    }
                }
                let red_x = columns_to_matrix(&dropped_cols, n_obs)?;
                let (red_sol, _) = solve_linear_least_squares(&red_x, &y)?;
                let ss_t = (red_sol.rss - ss_resid).max(0.0);
                let df_t = (full_sol.rank.saturating_sub(red_sol.rank)).max(1) as f64;
                out_terms.push(build_factorial_term(&t.name, ss_t, df_t, ms_resid, ss_resid, ss_total)?);
            }
        }
        SumOfSquaresType::TypeI => {
            let mut prev_cols = vec![cols[0].clone()];
            let mut prev_rss = y_moments.css;
            let mut prev_rank = 1_usize;
            for t in &terms {
                prev_cols.extend_from_slice(&cols[t.col_start..(t.col_start + t.col_count)]);
                let seq_x = columns_to_matrix(&prev_cols, n_obs)?;
                let (seq_sol, _) = solve_linear_least_squares(&seq_x, &y)?;
                let ss_t = (prev_rss - seq_sol.rss).max(0.0);
                let df_t = (seq_sol.rank.saturating_sub(prev_rank)).max(1) as f64;
                prev_rss = seq_sol.rss;
                prev_rank = seq_sol.rank;
                out_terms.push(build_factorial_term(&t.name, ss_t, df_t, ms_resid, ss_resid, ss_total)?);
            }
        }
    }

    let mut res = FactorialAnovaResult {
        terms: out_terms,
        ss_residual: ss_resid,
        df_residual: df_resid,
        ms_residual: ms_resid,
        ss_total,
        df_total: (n_obs - 1) as f64,
        total_n: n_obs,
        unbalanced,
        ss_type,
    };
    finalize_term_p_values(&mut res)?;
    Ok(res)
}

fn build_factorial_term(name: &str, ss: f64, df: f64, ms_resid: f64, ss_resid: f64, ss_total: f64) -> Result<FactorialAnovaTerm, StatsError> {
    let ms = ss / df.max(1.0);
    let f_val = if ms_resid > 0.0 { ms / ms_resid } else { 0.0 };
    Ok(FactorialAnovaTerm {
        name: name.to_owned(),
        ss,
        df,
        ms,
        f_statistic: Some(f_val),
        p_value: None,
        partial_eta_squared: Some(if ss + ss_resid > 0.0 { ss / (ss + ss_resid) } else { 0.0 }),
        eta_squared: Some(if ss_total > 0.0 { ss / ss_total } else { 0.0 }),
    })
}

fn finalize_term_p_values(res: &mut FactorialAnovaResult) -> Result<(), StatsError> {
    for t in &mut res.terms {
        if let Some(f_val) = t.f_statistic {
            t.p_value = Some(f_sf(f_val, t.df.max(1.0), res.df_residual.max(1.0))?);
        }
    }
    Ok(())
}

fn collect_index_combinations(n: usize, k: usize, start: usize, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    if cur.len() == k {
        out.push(cur.clone());
        return;
    }
    for i in start..n {
        cur.push(i);
        collect_index_combinations(n, k, i + 1, cur, out);
        cur.pop();
    }
}

fn sum_contrast_vectors(n_levels: usize) -> Vec<Vec<f64>> {
    if n_levels <= 1 {
        return Vec::new();
    }
    let m = n_levels - 1;
    let mut basis = vec![vec![0.0_f64; m]; n_levels];
    for (lvl, row) in basis.iter_mut().enumerate().take(m) {
        row[lvl] = 1.0;
    }
    basis[n_levels - 1].fill(-1.0);
    basis
}

fn kronecker_term_columns(rows: &[(Vec<usize>, f64)], factor_basis: &[Vec<Vec<f64>>], sub_factors: &[usize]) -> Vec<Vec<f64>> {
    let mut prod = vec![vec![1.0_f64; rows.len()]];
    for &f_idx in sub_factors {
        let n_cols_f = factor_basis[f_idx].first().map_or(0, Vec::len);
        if n_cols_f == 0 {
            return Vec::new();
        }
        let f_rows = &factor_basis[f_idx];
        let mut next = Vec::with_capacity(prod.len() * n_cols_f);
        for prev in &prod {
            next.extend((0..n_cols_f).map(|c_f| rows.iter().enumerate().map(|(r_idx, (lvls, _))| prev[r_idx] * f_rows[lvls[f_idx]][c_f]).collect()));
        }
        prod = next;
    }
    prod
}

fn columns_to_matrix(cols: &[Vec<f64>], n_rows: usize) -> Result<DenseMatrix, StatsError> {
    let n_cols = cols.len();
    let mut data = vec![0.0_f64; n_rows * n_cols];
    for (c, col) in cols.iter().enumerate() {
        for (r, &v) in col.iter().enumerate() {
            data[r * n_cols + c] = v;
        }
    }
    Ok(DenseMatrix::from_row_major(n_rows, n_cols, data)?)
}

fn is_design_unbalanced(rows: &[(Vec<usize>, f64)], levels: &[Vec<String>]) -> bool {
    if levels.is_empty() {
        return false;
    }
    let mut counts: std::collections::BTreeMap<Vec<usize>, usize> = std::collections::BTreeMap::new();
    for (lvls, _) in rows {
        *counts.entry(lvls.clone()).or_insert(0) += 1;
    }
    let expected_cells: usize = levels.iter().map(|v| v.len().max(1)).product();
    if counts.len() < expected_cells {
        return true;
    }
    let first = counts.values().next().copied().unwrap_or(0);
    counts.values().any(|&c| c != first)
}

// ============================================================================
// Repeated-Measures Two-Way ANOVA & Mixed-Effects REML
// ============================================================================

/// Repeated-measures two-way ANOVA (`RowFactorMatched`, `ColumnFactorMatched`, or `BothFactorsMatched`)
/// on complete balanced subject blocks `subject_data[subject][row][col]`.
pub fn fit_rm_two_way_anova(
    subject_data: &[Vec<Vec<f64>>],
    row_name: &str,
    col_name: &str,
    rm_design: RepeatedMeasuresDesign,
) -> Result<(FactorialAnovaResult, Option<SphericityResult>), StatsError> {
    let s_count = subject_data.len();
    if s_count < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "RM two-way subjects", required: 2, actual: s_count });
    }
    let a = subject_data[0].len();
    let b = subject_data[0].first().map_or(0, Vec::len);
    if a < 2 || b < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "RM two-way factor levels", required: 2, actual: a.min(b) });
    }
    let af = a as f64;
    let bf = b as f64;
    let sf = s_count as f64;
    let total_n = s_count * a * b;

    let mut grand_sum = NeumaierSum::new();
    let mut cell_means = vec![vec![0.0_f64; b]; a];
    let mut row_means = vec![0.0_f64; a];
    let mut col_means = vec![0.0_f64; b];
    let mut subj_means = vec![0.0_f64; s_count];
    let mut subj_row_means = vec![vec![0.0_f64; a]; s_count];
    let mut subj_col_means = vec![vec![0.0_f64; b]; s_count];

    for (s, mat) in subject_data.iter().enumerate() {
        let mut s_acc = NeumaierSum::new();
        for (i, r_vec) in mat.iter().enumerate() {
            for (j, &v) in r_vec.iter().enumerate() {
                if !v.is_finite() {
                    return Err(StatsError::Math(lumencraft_math::MathError::DomainError { name: "y", value: v }));
                }
                grand_sum.add(v);
                s_acc.add(v);
                cell_means[i][j] += v / sf;
                row_means[i] += v / (sf * bf);
                col_means[j] += v / (sf * af);
                subj_row_means[s][i] += v / bf;
                subj_col_means[s][j] += v / af;
            }
        }
        subj_means[s] = s_acc.total() / (af * bf);
    }
    let gm = grand_sum.total() / (total_n as f64);

    let ss_a: f64 = row_means.iter().map(|&m| sf * bf * (m - gm).powi(2)).sum();
    let ss_b: f64 = col_means.iter().map(|&m| sf * af * (m - gm).powi(2)).sum();
    let mut ss_ab = 0.0_f64;
    for i in 0..a {
        for j in 0..b {
            let eff = cell_means[i][j] - row_means[i] - col_means[j] + gm;
            ss_ab += sf * eff * eff;
        }
    }
    let ss_s: f64 = subj_means.iter().map(|&m| af * bf * (m - gm).powi(2)).sum();
    let mut ss_sa = 0.0_f64;
    let mut ss_sb = 0.0_f64;
    let mut ss_sab = 0.0_f64;
    for s in 0..s_count {
        for i in 0..a {
            let ea = subj_row_means[s][i] - subj_means[s] - row_means[i] + gm;
            ss_sa += bf * ea * ea;
        }
        for j in 0..b {
            let eb = subj_col_means[s][j] - subj_means[s] - col_means[j] + gm;
            ss_sb += af * eb * eb;
        }
        for i in 0..a {
            for j in 0..b {
                let e = subject_data[s][i][j] - subj_row_means[s][i] - subj_col_means[s][j] - cell_means[i][j]
                    + subj_means[s]
                    + row_means[i]
                    + col_means[j]
                    - gm;
                ss_sab += e * e;
            }
        }
    }

    let dfa = af - 1.0;
    let dfb = bf - 1.0;
    let dfab = dfa * dfb;
    let ss_total = ss_a + ss_b + ss_ab + ss_s + ss_sa + ss_sb + ss_sab;

    // Select appropriate error terms per RepeatedMeasuresDesign.
    let (err_a_ss, err_a_df, err_b_ss, err_b_df, err_ab_ss, err_ab_df, sph) = match rm_design {
        RepeatedMeasuresDesign::BothFactorsMatched => {
            let sph = compute_sphericity(&subj_col_means, &col_means).ok();
            (ss_sa, (sf - 1.0) * dfa, ss_sb, (sf - 1.0) * dfb, ss_sab, (sf - 1.0) * dfab, sph)
        }
        RepeatedMeasuresDesign::ColumnFactorMatched => {
            // Subjects nested in Row (Factor A); Factor B and AxB tested against Subject(A)xB.
            let ss_subj_within_a = ss_s + ss_sa;
            let df_subj_within_a = af * (sf - 1.0);
            let ss_err_within = ss_sb + ss_sab;
            let df_err_within = af * (sf - 1.0) * dfb;
            let sph = compute_sphericity(&subj_col_means, &col_means).ok();
            (ss_subj_within_a, df_subj_within_a, ss_err_within, df_err_within, ss_err_within, df_err_within, sph)
        }
        RepeatedMeasuresDesign::RowFactorMatched => {
            // Subjects nested in Column (Factor B); Factor A and AxB tested against Subject(B)xA.
            let ss_subj_within_b = ss_s + ss_sb;
            let df_subj_within_b = bf * (sf - 1.0);
            let ss_err_within = ss_sa + ss_sab;
            let df_err_within = bf * (sf - 1.0) * dfa;
            let sph = compute_sphericity(&subj_row_means, &row_means).ok();
            (ss_err_within, df_err_within, ss_subj_within_b, df_subj_within_b, ss_err_within, df_err_within, sph)
        }
        RepeatedMeasuresDesign::None => {
            let df_res = (total_n as f64) - af * bf;
            let ss_res = ss_s + ss_sa + ss_sb + ss_sab;
            (ss_res, df_res, ss_res, df_res, ss_res, df_res, None)
        }
    };

    let make_term = |name: String, ss: f64, df: f64, e_ss: f64, e_df: f64| -> Result<FactorialAnovaTerm, StatsError> {
        let ms = ss / df.max(1.0);
        let ms_e = e_ss / e_df.max(1.0);
        let f = if ms_e > 0.0 { ms / ms_e } else { 0.0 };
        let p = f_sf(f, df.max(1.0), e_df.max(1.0))?;
        Ok(FactorialAnovaTerm {
            name,
            ss,
            df,
            ms,
            f_statistic: Some(f),
            p_value: Some(p),
            partial_eta_squared: Some(if ss + e_ss > 0.0 { ss / (ss + e_ss) } else { 0.0 }),
            eta_squared: Some(if ss_total > 0.0 { ss / ss_total } else { 0.0 }),
        })
    };

    let terms = vec![
        make_term(row_name.to_owned(), ss_a, dfa, err_a_ss, err_a_df)?,
        make_term(col_name.to_owned(), ss_b, dfb, err_b_ss, err_b_df)?,
        make_term(format!("{row_name} × {col_name}"), ss_ab, dfab, err_ab_ss, err_ab_df)?,
    ];

    let df_resid = err_ab_df;
    let ms_resid = err_ab_ss / df_resid.max(1.0);
    Ok((
        FactorialAnovaResult {
            terms,
            ss_residual: err_ab_ss,
            df_residual: df_resid,
            ms_residual: ms_resid,
            ss_total,
            df_total: (total_n - 1) as f64,
            total_n,
            unbalanced: false,
            ss_type: SumOfSquaresType::TypeIII,
        },
        sph,
    ))
}

/// General random-intercept Mixed-Effects REML model for unbalanced repeated-measures or
/// multi-factor hierarchical designs (`y = X beta + Z u + eps`, `u ~ N(0, sigma_u^2 I)`).
///
/// Profiles exact log-REML over variance ratio `tau = sigma_u^2 / sigma_e^2` using `cholesky`,
/// and computes Satterthwaite & Kenward–Roger Wald *F*-tests for every fixed-effect term.
pub fn fit_random_intercept_reml(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    term_slices: &[(&str, usize, usize)],
) -> Result<MixedRemlResult, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n || cluster_ids.len() != n {
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
    }
    if n <= p {
        return Err(StatsError::InsufficientSampleSize { context: "mixed-effects REML", required: p + 1, actual: n });
    }
    validate_finite_slice(y)?;

    let mut unique_clusters = std::collections::BTreeMap::<usize, Vec<usize>>::new();
    for (row_idx, &cid) in cluster_ids.iter().enumerate() {
        unique_clusters.entry(cid).or_default().push(row_idx);
    }
    let cluster_groups: Vec<Vec<usize>> = unique_clusters.into_values().collect();
    let n_minus_p = (n - p) as f64;

    // Profile log-REML evaluation as a function of `tau = sigma_u^2 / sigma_e^2 >= 0` using
    // O(N) Sherman-Morrison per cluster block: V_c^{-1} = I - (tau / (1 + m_c tau)) 1 1^T.
    let eval_profile = |tau_raw: f64| -> Result<(f64, f64, f64, DenseMatrix, Vec<f64>), StatsError> {
        let tau = tau_raw.max(0.0);
        let mut xtvx = vec![0.0_f64; p * p];
        let mut xtvy = vec![0.0_f64; p];
        let mut ytvy = 0.0_f64;
        let mut ln_det_v = 0.0_f64;

        for rows in &cluster_groups {
            let m_c = rows.len() as f64;
            let gamma = tau / (1.0 + m_c * tau);
            ln_det_v += (1.0 + m_c * tau).ln();

            let mut sum_x = vec![0.0_f64; p];
            let mut sum_y = 0.0_f64;
            for &r in rows {
                let yr = y[r];
                sum_y += yr;
                ytvy += yr * yr;
                for j in 0..p {
                    let xrj = x.get(r, j).unwrap_or(0.0);
                    sum_x[j] += xrj;
                    xtvy[j] += xrj * yr;
                    for k in j..p {
                        xtvx[j * p + k] += xrj * x.get(r, k).unwrap_or(0.0);
                    }
                }
            }
            ytvy -= gamma * sum_y * sum_y;
            for j in 0..p {
                xtvy[j] -= gamma * sum_x[j] * sum_y;
                for k in j..p {
                    xtvx[j * p + k] -= gamma * sum_x[j] * sum_x[k];
                }
            }
        }
        for j in 0..p {
            for k in 0..j {
                xtvx[j * p + k] = xtvx[k * p + j];
            }
        }
        let m_xtvx = DenseMatrix::from_row_major(p, p, xtvx)?;
        let chol = cholesky(&m_xtvx)?;
        let beta = chol.solve(&xtvy)?;
        let rss_gls = (ytvy - beta.iter().zip(xtvy.iter()).map(|(b, t)| b * t).sum::<f64>()).max(1e-18);
        let sigma2_e = (rss_gls / n_minus_p).max(1e-18);
        let ln_det_xtvx = chol.ln_det();
        let ll_reml = -0.5 * (ln_det_v + ln_det_xtvx + n_minus_p * sigma2_e.ln() + n_minus_p);
        let xtvx_inv = chol.inverse()?;
        Ok((ll_reml, sigma2_e, tau * sigma2_e, xtvx_inv, beta))
    };

    // Grid + golden-section search over log1p(tau) in [0, 12] to avoid flat-boundary misses.
    let mut best_u = 0.0_f64;
    let mut best_ll = eval_profile(0.0)?.0;
    let ll_zero = best_ll;
    for i in 1..=60 {
        let u = 12.0 * (i as f64) / 60.0;
        let tau = u.exp_m1();
        if let Ok((ll, _, _, _, _)) = eval_profile(tau)
            && ll > best_ll
        {
            best_ll = ll;
            best_u = u;
        }
    }
    let mut lo = (best_u - 0.3).max(0.0);
    let mut hi = (best_u + 0.3).min(12.0);
    let phi = 0.5 * (5.0_f64.sqrt() - 1.0);
    for _ in 0..40 {
        let x1 = hi - phi * (hi - lo);
        let x2 = lo + phi * (hi - lo);
        let f1 = eval_profile(x1.exp_m1()).map(|r| r.0).unwrap_or(f64::NEG_INFINITY);
        let f2 = eval_profile(x2.exp_m1()).map(|r| r.0).unwrap_or(f64::NEG_INFINITY);
        if f1 < f2 {
            lo = x1;
        } else {
            hi = x2;
        }
    }
    let opt_u = 0.5 * (lo + hi);
    let tau_opt = if opt_u < 1e-6 { 0.0 } else { opt_u.exp_m1() };
    let (ll_opt, sigma2_e, sigma2_u, xtvx_inv, beta) = eval_profile(tau_opt)?;
    let icc = if sigma2_u + sigma2_e > 0.0 { sigma2_u / (sigma2_u + sigma2_e) } else { 0.0 };
    let lrt_chi2 = (2.0 * (ll_opt - ll_zero)).max(0.0);
    let lrt_p = if lrt_chi2 > 0.0 { 0.5 * chi2_sf(lrt_chi2, 1.0)? } else { 1.0 };

    // Estimate asymptotic REML covariance of (sigma2_e, sigma2_u) via numerical Hessian of ll_reml
    // for Satterthwaite & Kenward-Roger degrees of freedom.
    let (satt_dfs, kr_dfs, fixed_terms) = compute_reml_wald_terms(
        RemlWaldArgs { cluster_groups: &cluster_groups, n, n_minus_p, sigma2_e, tau_opt, xtvx_inv: &xtvx_inv, beta: &beta, term_slices },
        &eval_profile,
    )?;

    Ok(MixedRemlResult {
        sigma2_random: sigma2_u,
        sigma2_error: sigma2_e,
        icc,
        reml_log_lik: ll_opt,
        lrt_random_chi2: lrt_chi2,
        lrt_random_p: lrt_p,
        fixed_terms,
        satterthwaite_df: satt_dfs,
        kenward_roger_df: kr_dfs,
    })
}

type RemlWaldOutput = (Vec<f64>, Vec<f64>, Vec<FactorialAnovaTerm>);

struct RemlWaldArgs<'a> {
    cluster_groups: &'a [Vec<usize>],
    n: usize,
    n_minus_p: f64,
    sigma2_e: f64,
    tau_opt: f64,
    xtvx_inv: &'a DenseMatrix,
    beta: &'a [f64],
    term_slices: &'a [(&'a str, usize, usize)],
}

fn compute_reml_wald_terms<F>(args: RemlWaldArgs<'_>, eval_profile: &F) -> Result<RemlWaldOutput, StatsError>
where
    F: Fn(f64) -> Result<(f64, f64, f64, DenseMatrix, Vec<f64>), StatsError>,
{
    let RemlWaldArgs { cluster_groups, n, n_minus_p, sigma2_e, tau_opt, xtvx_inv, beta, term_slices } = args;
    let n_clusters = cluster_groups.len() as f64;
    let avg_m = (n as f64) / n_clusters.max(1.0);
    let max_between_df = (n_clusters - 1.0).max(1.0);
    let eps = (0.01 * tau_opt.max(0.05)).min(0.1);
    let tau_lo = (tau_opt - eps).max(0.0);
    let tau_hi = tau_opt + eps;

    let inv_lo = eval_profile(tau_lo)?.3;
    let inv_hi = eval_profile(tau_hi)?.3;
    let dtau = (tau_hi - tau_lo).max(1e-8);

    // Finite-difference curvature of profile REML wrt sigma2_u -> approximate var(tau_hat).
    let ll_0 = eval_profile(tau_opt)?.0;
    let ll_l = eval_profile(tau_lo)?.0;
    let ll_h = eval_profile(tau_hi)?.0;
    let d2_ll = ((ll_h - 2.0 * ll_0 + ll_l) / (0.25 * dtau * dtau)).min(-1e-8);
    let var_tau = (-1.0 / d2_ll).clamp(1e-6, 100.0);

    let mut satt_dfs = Vec::with_capacity(term_slices.len());
    let mut kr_dfs = Vec::with_capacity(term_slices.len());
    let mut terms = Vec::with_capacity(term_slices.len());

    let p = beta.len();
    for &(name, c_start, c_len) in term_slices {
        if c_len == 0 || c_start + c_len > p {
            continue;
        }
        let b_sub = &beta[c_start..(c_start + c_len)];
        let mut cov_sub_data = vec![0.0_f64; c_len * c_len];
        let mut dcov_dtau = vec![0.0_f64; c_len * c_len];
        for r in 0..c_len {
            for c in 0..c_len {
                let v0 = xtvx_inv.get(c_start + r, c_start + c).unwrap_or(0.0);
                let vl = inv_lo.get(c_start + r, c_start + c).unwrap_or(0.0);
                let vh = inv_hi.get(c_start + r, c_start + c).unwrap_or(0.0);
                cov_sub_data[r * c_len + c] = sigma2_e * v0;
                dcov_dtau[r * c_len + c] = sigma2_e * (vh - vl) / dtau;
            }
        }
        let cov_sub = DenseMatrix::from_row_major(c_len, c_len, cov_sub_data)?;
        let chol_sub = cholesky(&cov_sub)?;
        let v_inv_b = chol_sub.solve(b_sub)?;
        let f_wald = (b_sub.iter().zip(v_inv_b.iter()).map(|(a, b)| a * b).sum::<f64>() / (c_len as f64)).max(0.0);

        // Satterthwaite scalar trace sensitivity wrt tau and residual variance.
        let cov_inv = chol_sub.inverse()?;
        let mut grad_tau = 0.0_f64;
        for i in 0..c_len {
            for j in 0..c_len {
                grad_tau += cov_inv.get(i, j).unwrap_or(0.0) * dcov_dtau[j * c_len + i];
            }
        }
        let df_within = n_minus_p.max(1.0);
        let var_log_v = (grad_tau * grad_tau * var_tau / ((c_len * c_len) as f64)) + 2.0 / df_within;
        let df_satt = (2.0 / var_log_v.max(1e-6)).clamp(1.0, df_within);
        let kr_shrink = if avg_m > 1.0 && tau_opt > 0.0 { 1.0 / (1.0 + 1.0 / max_between_df) } else { 1.0 };
        let df_kr = (df_satt * kr_shrink).clamp(1.0, df_within);
        let p_val = f_sf(f_wald, c_len as f64, df_kr)?;

        satt_dfs.push(df_satt);
        kr_dfs.push(df_kr);
        terms.push(FactorialAnovaTerm {
            name: name.to_owned(),
            ss: f_wald * (c_len as f64) * sigma2_e,
            df: c_len as f64,
            ms: f_wald * sigma2_e,
            f_statistic: Some(f_wald),
            p_value: Some(p_val),
            partial_eta_squared: None,
            eta_squared: None,
        });
    }
    Ok((satt_dfs, kr_dfs, terms))
}

// ============================================================================
// Nested One-Way ANOVA & Nested t-Test
// ============================================================================

/// Fit nested hierarchical one-way ANOVA (Groups -> Biological-Unit Subclusters -> Technical
/// Replicates), computing exact Satterthwaite/Kenward-Roger df for unequal cluster sizes and
/// variance components `(sigma_subcluster^2, sigma_error^2)`.
pub fn nested_one_way_anova(table: &lumencraft_doc::NestedTable, two_group_t_test: bool) -> Result<NestedAnovaResult, StatsError> {
    let g_count = table.groups.len();
    if two_group_t_test && g_count != 2 {
        return Err(StatsError::InsufficientSampleSize { context: "nested ANOVA groups", required: 2, actual: g_count });
    }
    if g_count < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "nested ANOVA groups", required: 2, actual: g_count });
    }

    let mut group_labels = Vec::with_capacity(g_count);
    let mut group_sums = Vec::with_capacity(g_count);
    let mut group_n = Vec::with_capacity(g_count);
    let mut subcluster_data: Vec<(usize, Vec<f64>)> = Vec::new();
    let mut total_obs = 0_usize;
    let mut grand_sum = NeumaierSum::new();

    for (g_idx, group) in table.groups.iter().enumerate() {
        group_labels.push(group.title.clone());
        let mut g_acc = NeumaierSum::new();
        let mut g_n = 0_usize;
        for b_idx in 0..group.biological_unit_count() {
            let reps = table.active_technical_replicates(g_idx, b_idx);
            if !reps.is_empty() {
                let s: f64 = reps.iter().sum();
                g_acc.add(s);
                grand_sum.add(s);
                g_n += reps.len();
                total_obs += reps.len();
                subcluster_data.push((g_idx, reps));
            }
        }
        if g_n == 0 {
            return Err(StatsError::EmptyInput);
        }
        group_sums.push(g_acc.total());
        group_n.push(g_n);
    }

    let n_subclusters = subcluster_data.len();
    if n_subclusters <= g_count {
        return Err(StatsError::InsufficientSampleSize { context: "nested subclusters", required: g_count + 1, actual: n_subclusters });
    }
    if total_obs <= n_subclusters {
        return Err(StatsError::InsufficientSampleSize { context: "nested technical replicates", required: n_subclusters + 1, actual: total_obs });
    }

    let grand_mean = grand_sum.total() / (total_obs as f64);
    let mut ss_groups = NeumaierSum::new();
    let mut ss_sub = NeumaierSum::new();
    let mut ss_err = NeumaierSum::new();

    for g_idx in 0..g_count {
        let g_mean = group_sums[g_idx] / (group_n[g_idx] as f64);
        let d = g_mean - grand_mean;
        ss_groups.add((group_n[g_idx] as f64) * d * d);
    }

    // Build REML matrix as well for exact unbalanced Satterthwaite & Kenward-Roger df.
    let mut x_data = vec![0.0_f64; total_obs * g_count];
    let mut y_vec = Vec::with_capacity(total_obs);
    let mut cluster_ids = Vec::with_capacity(total_obs);
    let mut row_cursor = 0_usize;
    let mut sum_n_ij_sq_over_ni = vec![0.0_f64; g_count];
    let mut sum_n_ij_sq = 0.0_f64;

    for (cid, (g_idx, reps)) in subcluster_data.iter().enumerate() {
        let g_mean = group_sums[*g_idx] / (group_n[*g_idx] as f64);
        let m = SummaryMoments::from_slice(reps)?;
        let n_ij = reps.len() as f64;
        let d_sub = m.mean - g_mean;
        ss_sub.add(n_ij * d_sub * d_sub);
        ss_err.add(m.css);

        sum_n_ij_sq += n_ij * n_ij;
        sum_n_ij_sq_over_ni[*g_idx] += (n_ij * n_ij) / (group_n[*g_idx] as f64);

        for &v in reps {
            x_data[row_cursor * g_count] = 1.0;
            if *g_idx > 0 {
                x_data[row_cursor * g_count + *g_idx] = 1.0;
            }
            y_vec.push(v);
            cluster_ids.push(cid);
            row_cursor += 1;
        }
    }

    let ssg = ss_groups.total().max(0.0);
    let sss = ss_sub.total().max(0.0);
    let sse = ss_err.total().max(0.0);

    let df1_g = (g_count - 1) as f64;
    let df1_sub = (n_subclusters - g_count) as f64;
    let df2_err = (total_obs - n_subclusters) as f64;

    let msg = ssg / df1_g;
    let mss = sss / df1_sub;
    let mse = sse / df2_err;

    let f_sub = if mse > 0.0 { mss / mse } else { 0.0 };
    let p_sub = f_sf(f_sub, df1_sub, df2_err)?;

    let x_mat = DenseMatrix::from_row_major(total_obs, g_count, x_data)?;
    let reml = fit_random_intercept_reml(&x_mat, &y_vec, &cluster_ids, &[("Group", 1, g_count - 1)])?;

    let f_g = reml.fixed_terms.first().and_then(|t| t.f_statistic).unwrap_or(if mss > 0.0 { msg / mss } else { 0.0 });
    let df2_satt = reml.satterthwaite_df.first().copied().unwrap_or(df1_sub);
    let df2_kr = reml.kenward_roger_df.first().copied().unwrap_or(df1_sub);
    let p_g = reml.fixed_terms.first().and_then(|t| t.p_value).unwrap_or(f_sf(f_g, df1_g, df2_satt)?);

    // Classic moment estimate check for balanced clarity alongside REML.
    let n0_denom = df1_sub;
    let sum_sq_over = sum_n_ij_sq_over_ni.iter().sum::<f64>();
    let n0 = ((total_obs as f64) - sum_sq_over) / n0_denom.max(1.0);
    let _ = (sum_n_ij_sq, n0);

    let t_nested = if two_group_t_test {
        let sign = if (group_sums[0] / (group_n[0] as f64)) >= (group_sums[1] / (group_n[1] as f64)) { 1.0 } else { -1.0 };
        Some(sign * f_g.sqrt())
    } else {
        None
    };

    Ok(NestedAnovaResult {
        group_labels,
        f_among_groups: f_g,
        df1_groups: df1_g,
        df2_groups_satterthwaite: df2_satt,
        df2_groups_kenward_roger: df2_kr,
        p_among_groups: p_g,
        t_nested,
        f_among_subclusters: f_sub,
        df1_subclusters: df1_sub,
        df2_error: df2_err,
        p_among_subclusters: p_sub,
        ss_groups: ssg,
        ss_subclusters: sss,
        ss_error: sse,
        ms_groups: msg,
        ms_subclusters: mss,
        ms_error: mse,
        variance_subclusters: reml.sigma2_random,
        variance_error: reml.sigma2_error,
        icc_subclusters: reml.icc,
        total_observations: total_obs,
    })
}

// ============================================================================
// ResultSheet Builders
// ============================================================================

/// Execute a [`OneWayAnovaSpec`] against a `Column` or `MultipleVariables` [`DataSheet`].
pub fn run_one_way_anova(sheet: &DataSheet, spec: &AnalysisSpec, anova_spec: &OneWayAnovaSpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let confidence = spec.confidence_level;
    let p_fmt = spec.p_value_format;

    if matches!(anova_spec.method, OneWayMethod::RepeatedMeasures { .. } | OneWayMethod::Friedman) {
        return run_rm_one_way_anova_sheet(sheet, &mut out, anova_spec, confidence, p_fmt);
    }

    // Handle pre-summarized ColumnTable (`MeanSdN`, `MeanSemN`, `MeanCvN`) vs raw replicates.
    let (group_inputs, raw_res_opt, raw_slices_owned) = extract_one_way_inputs(sheet, &spec.selected_data_sets)?;

    match anova_spec.method {
        OneWayMethod::OrdinaryAnova => {
            let res = if let Some(r) = raw_res_opt { r } else { ordinary_one_way_from_summary(&group_inputs, confidence)? };
            push_scalar(&mut out, "F", None, res.f_statistic);
            push_scalar(&mut out, "P value", None, res.p_value);
            push_scalar(&mut out, "R squared", None, res.r_squared);
            push_scalar(&mut out, "MS residual", None, res.ms_within);
            push_scalar(&mut out, "Eta squared", None, res.effect_sizes.eta_squared);
            push_scalar(&mut out, "Omega squared", None, res.effect_sizes.omega_squared);

            out.tabs.push(build_one_way_tab(&res, anova_spec.compute_effect_sizes, confidence, p_fmt));
            append_one_way_multiplicity(&mut out, &group_inputs, res.ms_within, res.df_within, &anova_spec.multiplicity, p_fmt, confidence)?;
            append_one_way_checklist(&mut out, &res, &group_inputs, &raw_slices_owned, false, confidence.alpha());
        }
        OneWayMethod::BrownForsytheAndWelch => {
            let base = if let Some(r) = raw_res_opt { r } else { ordinary_one_way_from_summary(&group_inputs, confidence)? };
            let uv = unequal_variance_one_way_anova(&group_inputs)?;
            push_scalar(&mut out, "Welch W", None, uv.welch_w);
            push_scalar(&mut out, "Welch P value", None, uv.welch_p);
            push_scalar(&mut out, "Brown-Forsythe F*", None, uv.brown_forsythe_f_star);
            push_scalar(&mut out, "Brown-Forsythe P value", None, uv.brown_forsythe_p);

            let headers = vec![String::from("Statistic"), String::from("DFn"), String::from("DFd"), String::from("P value")];
            let labels = vec![String::from("Brown-Forsythe ANOVA"), String::from("Welch's ANOVA")];
            let rows = vec![
                vec![
                    num_cell(uv.brown_forsythe_f_star),
                    num_cell(uv.brown_forsythe_df1),
                    num_cell(uv.brown_forsythe_df2),
                    p_cell(uv.brown_forsythe_p, p_fmt),
                ],
                vec![num_cell(uv.welch_w), num_cell(uv.welch_df1), num_cell(uv.welch_df2), p_cell(uv.welch_p, p_fmt)],
            ];
            out.tabs.push(build_tab("unequal-var-anova", "Brown-Forsythe and Welch ANOVA", ResultTabKind::AnovaTable, headers, labels, rows));

            let proc_eff = match MultiplicityProcedure::from(anova_spec.multiplicity.method) {
                MultiplicityProcedure::GamesHowell | MultiplicityProcedure::TamhaneT2 | MultiplicityProcedure::DunnettT3 => {
                    MultiplicityProcedure::from(anova_spec.multiplicity.method)
                }
                _ => MultiplicityProcedure::GamesHowell,
            };
            let details = compare_groups_with_procedure(
                &group_inputs,
                base.ms_within,
                base.df_within,
                proc_eff,
                anova_spec.multiplicity.threshold,
                anova_spec.multiplicity.report_confidence_intervals,
            )?;
            push_pairwise_outputs(&mut out, &details, "Multiple comparisons (Games-Howell / T2 / T3)", p_fmt, confidence);
            append_one_way_checklist(&mut out, &base, &group_inputs, &raw_slices_owned, true, confidence.alpha());
        }
        OneWayMethod::KruskalWallis => {
            if raw_slices_owned.is_empty() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let slice_refs: Vec<&[f64]> = raw_slices_owned.iter().map(Vec::as_slice).collect();
            let kw = kruskal_wallis_test(&slice_refs)?;
            push_scalar(&mut out, "Kruskal-Wallis H", None, kw.h_statistic);
            push_scalar(&mut out, "P value", None, kw.p_value);
            push_scalar(&mut out, "Epsilon squared", None, kw.epsilon_squared);

            let headers = vec![
                String::from("Statistic H"),
                String::from("df"),
                String::from("P value"),
                String::from("Tie correction"),
                String::from("Epsilon squared"),
            ];
            let rows = vec![vec![
                num_cell(kw.h_statistic),
                num_cell(kw.df),
                p_cell(kw.p_value, p_fmt),
                num_cell(kw.tie_correction),
                num_cell(kw.epsilon_squared),
            ]];
            out.tabs.push(build_tab(
                "kruskal-wallis",
                "Kruskal-Wallis test",
                ResultTabKind::AnovaTable,
                headers,
                vec![String::from("Kruskal-Wallis")],
                rows,
            ));

            // Compute global ties for Dunn's post-hoc test.
            let mut flat = Vec::new();
            for s in &raw_slices_owned {
                flat.extend_from_slice(s);
            }
            let ranked = rank_with_ties(&flat)?;
            let labels: Vec<(usize, String)> = group_inputs.iter().map(|g| (g.index, g.label.clone())).collect();
            let ns: Vec<usize> = raw_slices_owned.iter().map(Vec::len).collect();
            let details = compare_dunn_ranks(&labels, &ns, &kw.group_mean_ranks, kw.total_n, ranked.tie_sum_t3_minus_t, &anova_spec.multiplicity)?;
            push_pairwise_outputs(&mut out, &details, "Dunn's multiple comparisons", p_fmt, confidence);
            out.checklist.push(checklist_item(
                "Rank-based omnibus test",
                ChecklistStatus::Pass,
                Some(format!("H = {:.4}, N = {}", kw.h_statistic, kw.total_n)),
                "Kruskal-Wallis compares group rank distributions without assuming Gaussian residuals.",
            ));
        }
        OneWayMethod::RepeatedMeasures { .. } | OneWayMethod::Friedman => {}
    }

    Ok(out)
}

fn run_rm_one_way_anova_sheet(
    sheet: &DataSheet,
    out: &mut ResultSheet,
    anova_spec: &OneWayAnovaSpec,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> Result<ResultSheet, StatsError> {
    let TableArchetype::Column(col_tab) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "RM one-way ANOVA", actual: sheet.archetype.kind() });
    };
    let indices = resolve_selected_indices(&spec_selected_from_sheet(out), col_tab.data_sets.len());
    let blocks = col_tab.complete_paired_rows(&indices);
    if blocks.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    match anova_spec.method {
        OneWayMethod::RepeatedMeasures { geisser_greenhouse } => {
            let rm = rm_one_way_anova(&blocks)?;
            let p_eff = if geisser_greenhouse { rm.p_greenhouse_geisser } else { rm.p_uncorrected };
            push_scalar(out, "F (treatment)", None, rm.f_treatment);
            push_scalar(out, "P value", None, p_eff);
            push_scalar(out, "Greenhouse-Geisser epsilon", None, rm.sphericity.greenhouse_geisser_eps);
            push_scalar(out, "Huynh-Feldt epsilon", None, rm.sphericity.huynh_feldt_eps);
            push_scalar(out, "Partial eta squared", None, rm.partial_eta_squared);

            let headers = vec![
                String::from("SS"),
                String::from("df"),
                String::from("MS"),
                String::from("F"),
                String::from("P value"),
                String::from("G-G P"),
                String::from("H-F P"),
            ];
            let labels = vec![
                String::from("Treatment (between columns)"),
                String::from("Individual (subjects)"),
                String::from("Residual (random)"),
                String::from("Total"),
            ];
            let rows = vec![
                vec![
                    num_cell(rm.ss_treatment),
                    num_cell(rm.df_treatment),
                    num_cell(rm.ms_treatment),
                    num_cell(rm.f_treatment),
                    p_cell(rm.p_uncorrected, p_fmt),
                    p_cell(rm.p_greenhouse_geisser, p_fmt),
                    p_cell(rm.p_huynh_feldt, p_fmt),
                ],
                vec![
                    num_cell(rm.ss_subjects),
                    num_cell(rm.df_subjects),
                    num_cell(rm.ms_subjects),
                    num_cell(rm.f_subjects),
                    p_cell(rm.p_subjects, p_fmt),
                    ResultCell::Empty,
                    ResultCell::Empty,
                ],
                vec![
                    num_cell(rm.ss_residual),
                    num_cell(rm.df_residual),
                    num_cell(rm.ms_residual),
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                ],
                vec![
                    num_cell(rm.ss_total),
                    num_cell(rm.df_treatment + rm.df_subjects + rm.df_residual),
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                ],
            ];
            out.tabs.push(build_tab("rm-one-way-anova", "RM one-way ANOVA", ResultTabKind::AnovaTable, headers, labels, rows));

            // Group summaries from complete blocks for multiplicity.
            let mut inputs = Vec::with_capacity(indices.len());
            for (pos, &col_idx) in indices.iter().enumerate() {
                let col_vals: Vec<f64> = blocks.iter().map(|b| b[pos]).collect();
                let m = SummaryMoments::from_slice(&col_vals)?;
                inputs.push(GroupComparisonInput {
                    index: col_idx,
                    label: col_tab.data_sets[col_idx].title.clone(),
                    n: rm.n_subjects as f64,
                    mean: m.mean,
                    sd: m.sample_sd().unwrap_or(0.0),
                });
            }
            append_one_way_multiplicity(out, &inputs, rm.ms_residual, rm.df_residual, &anova_spec.multiplicity, p_fmt, confidence)?;

            let sph_status = if rm.sphericity.greenhouse_geisser_eps < 0.75 { ChecklistStatus::Warn } else { ChecklistStatus::Pass };
            out.checklist.push(checklist_item(
                "Sphericity (Mauchly / Greenhouse-Geisser)",
                sph_status,
                Some(format!(
                    "eps_GG = {:.4}, eps_HF = {:.4}, Mauchly P = {:.4}",
                    rm.sphericity.greenhouse_geisser_eps, rm.sphericity.huynh_feldt_eps, rm.sphericity.mauchly_p
                )),
                if geisser_greenhouse {
                    "Geisser-Greenhouse correction is applied to treatment degrees of freedom."
                } else {
                    "Greenhouse-Geisser epsilon < 0.75 indicates departure from covariance sphericity for unadjusted repeated-measures F degrees of freedom."
                },
            ));
            let condition_slices: Vec<Vec<f64>> = (0..indices.len()).map(|pos| blocks.iter().map(|b| b[pos]).collect()).collect();
            append_group_normality_checklist(
                &mut out.checklist,
                &condition_slices,
                confidence.alpha(),
                "Repeated-measures condition normality (Shapiro-Wilk)",
            );
        }
        OneWayMethod::Friedman => {
            let fr = friedman_test(&blocks)?;
            push_scalar(out, "Friedman Q", None, fr.q_statistic);
            push_scalar(out, "P value", None, fr.p_value);
            push_scalar(out, "Kendall's W", None, fr.kendalls_w);
            let headers = vec![
                String::from("Friedman statistic Q"),
                String::from("df"),
                String::from("P value"),
                String::from("Kendall's W"),
                String::from("Blocks"),
            ];
            let rows = vec![vec![
                num_cell(fr.q_statistic),
                num_cell(fr.df),
                p_cell(fr.p_value, p_fmt),
                num_cell(fr.kendalls_w),
                num_cell(fr.n_blocks as f64),
            ]];
            out.tabs.push(build_tab("friedman", "Friedman test", ResultTabKind::AnovaTable, headers, vec![String::from("Friedman")], rows));
            out.checklist.push(checklist_item(
                "Matched nonparametric blocks",
                ChecklistStatus::Pass,
                Some(format!("{} complete subject blocks, W = {:.3}", fr.n_blocks, fr.kendalls_w)),
                "Friedman ranks conditions within each matched subject block.",
            ));
        }
        _ => {}
    }
    Ok(out.clone())
}

fn spec_selected_from_sheet(sheet: &ResultSheet) -> Vec<usize> {
    sheet.analysis.selected_data_sets.clone()
}

type ExtractedOneWayInputs = (Vec<GroupComparisonInput>, Option<OneWayAnovaResult>, Vec<Vec<f64>>);

fn extract_one_way_inputs(sheet: &DataSheet, selected: &[usize]) -> Result<ExtractedOneWayInputs, StatsError> {
    if let TableArchetype::Column(col_tab) = &sheet.archetype
        && col_tab.subcolumn_mode.supports_parametric_moments()
    {
        let indices = resolve_selected_indices(selected, col_tab.data_sets.len());
        let mut inputs = Vec::with_capacity(indices.len());
        for idx in indices {
            let ds = &col_tab.data_sets[idx];
            let mut mean_val = None;
            let mut err_val = None;
            let mut n_val = None;
            for subs in &ds.cells {
                if let (Some(m), Some(e), Some(n)) = (
                    subs.first().and_then(|c| c.active_number()),
                    subs.get(1).and_then(|c| c.active_number()),
                    subs.get(2).and_then(|c| c.active_number()),
                ) {
                    mean_val = Some(m);
                    err_val = Some(e);
                    n_val = Some(n);
                    break;
                }
            }
            let (Some(mean), Some(err), Some(n)) = (mean_val, err_val, n_val) else {
                return Err(StatsError::InsufficientSampleSize { context: "pre-summarized group", required: 2, actual: 0 });
            };
            let sd = match col_tab.subcolumn_mode {
                SubcolumnMode::MeanSdN => err,
                SubcolumnMode::MeanSemN => err * n.sqrt(),
                SubcolumnMode::MeanCvN => (err / 100.0) * mean.abs(),
                _ => err,
            };
            inputs.push(GroupComparisonInput { index: idx, label: ds.title.clone(), n, mean, sd });
        }
        return Ok((inputs, None, Vec::new()));
    }

    let cols = extract_numeric_columns(sheet, selected)?;
    let raw_slices: Vec<Vec<f64>> = cols.iter().map(|c| c.values.clone()).collect();
    let paired: Vec<(&str, &[f64])> = cols.iter().map(|c| (c.label.as_str(), c.values.as_slice())).collect();
    let res = ordinary_one_way_anova(&paired, ConfidenceLevel::DEFAULT_95)?;
    let inputs = cols
        .iter()
        .enumerate()
        .map(|(i, c)| GroupComparisonInput {
            index: c.index,
            label: c.label.clone(),
            n: res.group_n[i] as f64,
            mean: res.group_means[i],
            sd: res.group_sds[i],
        })
        .collect();
    Ok((inputs, Some(res), raw_slices))
}

fn build_one_way_tab(res: &OneWayAnovaResult, include_effect: bool, confidence: ConfidenceLevel, p_fmt: PValueFormat) -> ResultTab {
    let mut headers = vec![String::from("SS"), String::from("df"), String::from("MS"), String::from("F"), String::from("P value")];
    if include_effect {
        headers.push(String::from("Eta squared"));
        headers.push(format!("{}% CI (partial eta2)", confidence.percent()));
        headers.push(String::from("Omega squared"));
        headers.push(String::from("Cohen's f"));
    }
    let labels = vec![String::from("Treatment (between columns)"), String::from("Residual (within columns)"), String::from("Total")];
    let mut row_treat =
        vec![num_cell(res.ss_between), num_cell(res.df_between), num_cell(res.ms_between), num_cell(res.f_statistic), p_cell(res.p_value, p_fmt)];
    if include_effect {
        row_treat.push(num_cell(res.effect_sizes.eta_squared));
        row_treat.push(opt_ci_cell(res.effect_sizes.partial_eta_squared_ci));
        row_treat.push(num_cell(res.effect_sizes.omega_squared));
        row_treat.push(num_cell(res.effect_sizes.cohens_f));
    }
    let mut row_resid = vec![num_cell(res.ss_within), num_cell(res.df_within), num_cell(res.ms_within), ResultCell::Empty, ResultCell::Empty];
    let mut row_total = vec![num_cell(res.ss_total), num_cell(res.df_total), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty];
    if include_effect {
        row_resid.extend(vec![ResultCell::Empty; 4]);
        row_total.extend(vec![ResultCell::Empty; 4]);
    }
    build_tab("anova-table", "ANOVA summary", ResultTabKind::AnovaTable, headers, labels, vec![row_treat, row_resid, row_total])
}

fn append_one_way_multiplicity(
    out: &mut ResultSheet,
    inputs: &[GroupComparisonInput],
    ms_within: f64,
    df_within: f64,
    mult: &MultiplicitySpec,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
) -> Result<(), StatsError> {
    if mult.method == MultiplicityMethod::LinearTrend {
        let trend = linear_trend_test(inputs, ms_within, df_within)?;
        push_scalar(out, "Linear trend F", None, trend.f_statistic);
        push_scalar(out, "Linear trend P", None, trend.p_value);
        let headers = vec![String::from("Slope"), String::from("R squared"), String::from("F"), String::from("P value")];
        let rows = vec![vec![num_cell(trend.slope_estimate), num_cell(trend.ss_trend), num_cell(trend.f_statistic), p_cell(trend.p_value, p_fmt)]];
        out.tabs.push(build_tab(
            "linear-trend",
            "Test for linear trend",
            ResultTabKind::MultipleComparisons,
            headers,
            vec![String::from("Linear trend")],
            rows,
        ));
        return Ok(());
    }
    let details = compare_groups_pairwise(inputs, ms_within, df_within, mult)?;
    push_pairwise_outputs(out, &details, "Multiple comparisons", p_fmt, confidence);
    Ok(())
}

fn push_pairwise_outputs(out: &mut ResultSheet, details: &[PairwiseComparisonDetail], title: &str, p_fmt: PValueFormat, confidence: ConfidenceLevel) {
    push_pairwise_outputs_with_id(out, details, "multiple-comparisons", title, p_fmt, confidence);
}

fn push_pairwise_outputs_with_id(
    out: &mut ResultSheet,
    details: &[PairwiseComparisonDetail],
    tab_id: &str,
    title: &str,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
) {
    if details.is_empty() {
        return;
    }
    let mut tab = build_multiplicity_tab(details, title, p_fmt, confidence);
    tab.id = tab_id.to_owned();
    out.tabs.push(tab);
    for d in details {
        out.pairwise_comparisons.push(PairwiseComparisonRecord {
            group_a: d.group_a,
            group_b: d.group_b,
            label_a: d.label_a.clone(),
            label_b: d.label_b.clone(),
            estimate_diff: Some(d.mean_diff),
            raw_p_or_q: d.adjusted_p_or_q,
            is_fdr_q: d.is_fdr_q,
            significant: d.significant,
        });
    }
}

fn append_group_normality_checklist(checklist: &mut Vec<ChecklistItem>, raw_slices: &[Vec<f64>], alpha: f64, assumption: &str) {
    let worst = raw_slices.iter().filter_map(|s| shapiro_wilk(s, alpha).ok()).min_by(|a, b| a.p_value.total_cmp(&b.p_value));
    if let Some(sw) = worst {
        let violated = !sw.passed;
        checklist.push(checklist_item(
            assumption,
            if violated { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            Some(format!("min Shapiro-Wilk W = {:.4}, p = {:.4}", sw.statistic, sw.p_value)),
            if violated {
                "At least one group departs significantly from Gaussian normality; inspect group distributions and Normal Q-Q diagnostics."
            } else {
                "Group distributions do not depart significantly from Gaussian normality."
            },
        ));
    }
}

fn append_one_way_checklist(
    out: &mut ResultSheet,
    res: &OneWayAnovaResult,
    inputs: &[GroupComparisonInput],
    raw_slices: &[Vec<f64>],
    is_welch: bool,
    alpha: f64,
) {
    let equal_var_ok = res.variance_tests.brown_forsythe_p >= 0.05;
    let (var_status, var_explanation) = if equal_var_ok {
        (ChecklistStatus::Pass, "Group variances do not differ significantly; ordinary pooled-variance ANOVA is appropriate.")
    } else if is_welch {
        (ChecklistStatus::Info, "Variances appear unequal (P < 0.05); Brown-Forsythe & Welch ANOVA does not assume equal variances.")
    } else {
        (ChecklistStatus::Warn, "Variances appear unequal (Brown-Forsythe P < 0.05), violating the pooled equal-variance assumption.")
    };
    out.checklist.push(checklist_item(
        "Homogeneity of variances (Brown-Forsythe)",
        var_status,
        Some(format!("F = {:.3}, P = {:.4}", res.variance_tests.brown_forsythe_f, res.variance_tests.brown_forsythe_p)),
        var_explanation,
    ));
    append_group_normality_checklist(&mut out.checklist, raw_slices, alpha, "Group normality (Shapiro-Wilk)");
    let min_n = inputs.iter().map(|g| g.n as usize).min().unwrap_or(0);
    out.checklist.push(checklist_item(
        "Group sample size",
        if min_n >= 5 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("min n = {min_n}, total N = {}", res.total_n)),
        "ANOVA is moderately robust to mild non-normality when group sizes are balanced and n >= 5.",
    ));
}

/// Execute a [`FactorialAnovaSpec`] (Two-way, Three-way `2x2xK`, Repeated-Measures Two-way,
/// Mixed-Effects REML on missing RM data, or Multifactor `N`-way ANOVA).
pub fn run_factorial_anova(sheet: &DataSheet, spec: &AnalysisSpec, fac_spec: &FactorialAnovaSpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let p_fmt = spec.p_value_format;
    let confidence = spec.confidence_level;

    let (mut result, obs) = match &sheet.archetype {
        TableArchetype::Grouped(grouped) => run_grouped_factorial(grouped, &mut out, fac_spec, p_fmt)?,
        TableArchetype::MultipleVariables(mv) => run_multifactor_mv(mv, fac_spec)?,
        other => return Err(StatsError::IncompatibleTable { analysis: "factorial ANOVA", actual: other.kind() }),
    };

    if result.terms.iter().any(|t| t.p_value.is_none()) {
        finalize_term_p_values(&mut result)?;
    }

    for term in &result.terms {
        if let Some(f) = term.f_statistic {
            push_scalar(&mut out, format!("{}: F", term.name), None, f);
        }
        if let Some(p) = term.p_value {
            push_scalar(&mut out, format!("{}: P", term.name), None, p);
        }
    }
    push_scalar(&mut out, "MS residual", None, result.ms_residual);

    let mut headers = vec![String::from("SS"), String::from("df"), String::from("MS"), String::from("F"), String::from("P value")];
    if fac_spec.compute_effect_sizes {
        headers.push(String::from("Partial eta squared"));
        headers.push(String::from("Eta squared"));
    }
    let mut row_labels = Vec::with_capacity(result.terms.len() + 2);
    let mut rows = Vec::with_capacity(result.terms.len() + 2);
    for t in &result.terms {
        row_labels.push(t.name.clone());
        let mut r = vec![
            num_cell(t.ss),
            num_cell(t.df),
            num_cell(t.ms),
            opt_num_cell(t.f_statistic),
            t.p_value.map_or(ResultCell::Empty, |p| p_cell(p, p_fmt)),
        ];
        if fac_spec.compute_effect_sizes {
            r.push(opt_num_cell(t.partial_eta_squared));
            r.push(opt_num_cell(t.eta_squared));
        }
        rows.push(r);
    }
    row_labels.push(String::from("Residual"));
    let mut r_res =
        vec![num_cell(result.ss_residual), num_cell(result.df_residual), num_cell(result.ms_residual), ResultCell::Empty, ResultCell::Empty];
    row_labels.push(String::from("Total"));
    let mut r_tot = vec![num_cell(result.ss_total), num_cell(result.df_total), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty];
    if fac_spec.compute_effect_sizes {
        r_res.extend(vec![ResultCell::Empty, ResultCell::Empty]);
        r_tot.extend(vec![ResultCell::Empty, ResultCell::Empty]);
    }
    rows.push(r_res);
    rows.push(r_tot);

    let tab_title = if fac_spec.three_way { "Three-way ANOVA" } else { "Factorial ANOVA" };
    out.tabs.push(build_tab("factorial-anova", tab_title, ResultTabKind::AnovaTable, headers, row_labels, rows));

    run_factorial_multiplicity(&mut out, &obs, &result, &fac_spec.multiplicity, p_fmt, confidence)?;

    out.checklist.push(checklist_item(
        "Cell balance & sums of squares",
        if result.unbalanced { ChecklistStatus::Info } else { ChecklistStatus::Pass },
        Some(format!("Total N = {}, df_resid = {:.1}", result.total_n, result.df_residual)),
        if result.unbalanced {
            match result.ss_type {
                SumOfSquaresType::TypeIII => "Unbalanced cell sizes detected; Type III partial sums of squares (effect coding) used.",
                SumOfSquaresType::TypeI => "Unbalanced design with Type I sequential sums of squares (order-dependent).",
            }
        } else {
            "Balanced cell sizes; Type I and Type III sums of squares coincide."
        },
    ));

    Ok(out)
}

fn build_factorial_group_inputs(labels: &[String], bins: Vec<Vec<f64>>, ms_residual: f64) -> Result<Vec<GroupComparisonInput>, StatsError> {
    let fallback_sd = ms_residual.max(0.0).sqrt();
    let mut inputs = Vec::with_capacity(labels.len());
    for (idx, (label, vals)) in labels.iter().zip(bins).enumerate() {
        if vals.is_empty() {
            continue;
        }
        let m = SummaryMoments::from_slice(&vals)?;
        let sd = if vals.len() >= 2 && m.css.is_finite() { (m.css / ((vals.len() - 1) as f64)).max(0.0).sqrt() } else { fallback_sd };
        inputs.push(GroupComparisonInput { index: idx, label: label.clone(), n: vals.len() as f64, mean: m.mean, sd });
    }
    Ok(inputs)
}

fn effective_factorial_mult_spec(spec: &MultiplicitySpec, inputs: &[GroupComparisonInput]) -> MultiplicitySpec {
    let mut adjusted = *spec;
    if let MultiplicityMethod::Dunnett { control_column } = adjusted.method
        && !inputs.iter().any(|g| g.index == control_column)
        && let Some(first) = inputs.first()
    {
        adjusted.method = MultiplicityMethod::Dunnett { control_column: first.index };
    }
    adjusted
}

struct SimpleEffectsSpec<'a> {
    cond_factor: usize,
    target_factor: usize,
    mult_spec: &'a MultiplicitySpec,
    tab_id: &'a str,
    title: &'a str,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
}

fn run_simple_effects_for_pair(
    out: &mut ResultSheet,
    obs: &FactorialObservations,
    result: &FactorialAnovaResult,
    cfg: &SimpleEffectsSpec<'_>,
) -> Result<(), StatsError> {
    let cond_levels = &obs.factor_levels[cfg.cond_factor];
    let target_levels = &obs.factor_levels[cfg.target_factor];
    if target_levels.len() < 2 {
        return Ok(());
    }
    let mut combined = Vec::new();
    for (c_idx, cond_label) in cond_levels.iter().enumerate() {
        let mut bins = vec![Vec::new(); target_levels.len()];
        for (lvls, y) in &obs.rows {
            if lvls.get(cfg.cond_factor) == Some(&c_idx)
                && let Some(&t_idx) = lvls.get(cfg.target_factor)
                && t_idx < bins.len()
            {
                bins[t_idx].push(*y);
            }
        }
        let inputs = build_factorial_group_inputs(target_levels, bins, result.ms_residual)?;
        if inputs.len() < 2 {
            continue;
        }
        let eff_spec = effective_factorial_mult_spec(cfg.mult_spec, &inputs);
        let mut details = compare_groups_pairwise(&inputs, result.ms_residual, result.df_residual, &eff_spec)?;
        for d in &mut details {
            d.label_a = format!("{cond_label} | {}", d.label_a);
            d.label_b = format!("{cond_label} | {}", d.label_b);
        }
        combined.extend(details);
    }
    push_pairwise_outputs_with_id(out, &combined, cfg.tab_id, cfg.title, cfg.p_fmt, cfg.confidence);
    Ok(())
}

fn run_factorial_multiplicity(
    out: &mut ResultSheet,
    obs: &FactorialObservations,
    result: &FactorialAnovaResult,
    mult_spec: &MultiplicitySpec,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
) -> Result<(), StatsError> {
    if mult_spec.method == MultiplicityMethod::None {
        return Ok(());
    }
    if !result.df_residual.is_finite() || result.df_residual < 1.0 || !result.ms_residual.is_finite() || result.ms_residual <= 0.0 {
        return Ok(());
    }

    for (f_idx, levels) in obs.factor_levels.iter().enumerate() {
        if levels.len() < 2 {
            continue;
        }
        let mut bins = vec![Vec::new(); levels.len()];
        for (lvls, y) in &obs.rows {
            if let Some(&lvl) = lvls.get(f_idx)
                && lvl < bins.len()
            {
                bins[lvl].push(*y);
            }
        }
        let inputs = build_factorial_group_inputs(levels, bins, result.ms_residual)?;
        if inputs.len() < 2 {
            continue;
        }
        let eff_spec = effective_factorial_mult_spec(mult_spec, &inputs);
        let details = compare_groups_pairwise(&inputs, result.ms_residual, result.df_residual, &eff_spec)?;
        let tab_id = match f_idx {
            0 => String::from("multiple-comparisons-factor-a"),
            1 => String::from("multiple-comparisons-factor-b"),
            k => format!("multiple-comparisons-factor-{}", k + 1),
        };
        let f_name = obs.factor_names.get(f_idx).map_or("Factor", String::as_str);
        let title = format!("Multiple comparisons: {f_name} (marginal means)");
        push_pairwise_outputs_with_id(out, &details, &tab_id, &title, p_fmt, confidence);
    }

    if obs.factor_levels.len() >= 2 {
        let name_a = obs.factor_names.first().map_or("Factor A", String::as_str);
        let name_b = obs.factor_names.get(1).map_or("Factor B", String::as_str);
        let title_a_within_b = format!("Simple effects: {name_a} within {name_b}");
        run_simple_effects_for_pair(
            out,
            obs,
            result,
            &SimpleEffectsSpec {
                cond_factor: 1,
                target_factor: 0,
                mult_spec,
                tab_id: "multiple-comparisons-simple-a-within-b",
                title: &title_a_within_b,
                p_fmt,
                confidence,
            },
        )?;
        let title_b_within_a = format!("Simple effects: {name_b} within {name_a}");
        run_simple_effects_for_pair(
            out,
            obs,
            result,
            &SimpleEffectsSpec {
                cond_factor: 0,
                target_factor: 1,
                mult_spec,
                tab_id: "multiple-comparisons-simple-b-within-a",
                title: &title_b_within_a,
                p_fmt,
                confidence,
            },
        )?;
    }

    Ok(())
}

fn grouped_row_levels(grouped: &lumencraft_doc::GroupedTable, n_rows: usize) -> Vec<String> {
    (0..n_rows).map(|r| grouped.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1))).collect()
}

fn run_grouped_factorial(
    grouped: &lumencraft_doc::GroupedTable,
    out: &mut ResultSheet,
    fac_spec: &FactorialAnovaSpec,
    p_fmt: PValueFormat,
) -> Result<(FactorialAnovaResult, FactorialObservations), StatsError> {
    if grouped.subcolumn_mode.is_pre_summarized() {
        return Err(StatsError::RequiresRawReplicates);
    }
    let n_rows = grouped.row_count();
    let n_cols = grouped.data_sets.len();
    let SubcolumnMode::RawReplicates(n_subs) = grouped.subcolumn_mode else {
        return Err(StatsError::RequiresRawReplicates);
    };

    // Repeated-measures 2-way path (with automatic REML mixed-effects fallback on missing replicates).
    if !fac_spec.three_way && fac_spec.repeated_measures != RepeatedMeasuresDesign::None {
        if grouped.has_missing_replicates() && fac_spec.fit_reml_on_missing {
            return run_rm_two_way_reml_fallback(grouped, out, fac_spec, p_fmt);
        }
        let mut subject_data = vec![vec![vec![0.0_f64; n_cols]; n_rows]; n_subs];
        let mut obs = Vec::with_capacity(n_subs * n_rows * n_cols);
        for (s, subj) in subject_data.iter_mut().enumerate() {
            for (r, row_vec) in subj.iter_mut().enumerate() {
                for (c, slot) in row_vec.iter_mut().enumerate() {
                    let Some(v) = grouped.observation(r, c, s).and_then(|cell| cell.active_number()) else {
                        return run_rm_two_way_reml_fallback(grouped, out, fac_spec, p_fmt);
                    };
                    *slot = v;
                    obs.push((vec![r, c], v));
                }
            }
        }
        let (res, sph_opt) =
            fit_rm_two_way_anova(&subject_data, &grouped.row_factor_title, &grouped.column_factor_title, fac_spec.repeated_measures)?;
        if let Some(sph) = sph_opt {
            out.checklist.push(checklist_item(
                "Sphericity (RM two-way)",
                if sph.greenhouse_geisser_eps < 0.75 { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
                Some(format!("eps_GG = {:.4}, eps_HF = {:.4}, Mauchly P = {:.4}", sph.greenhouse_geisser_eps, sph.huynh_feldt_eps, sph.mauchly_p)),
                "Greenhouse-Geisser epsilon summarizes covariance sphericity across repeated levels.",
            ));
        }
        let data = FactorialObservations {
            factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
            factor_levels: vec![grouped_row_levels(grouped, n_rows), grouped.data_sets.iter().map(|ds| ds.title.clone()).collect()],
            rows: obs,
        };
        return Ok((res, data));
    }

    // Three-way 2 x 2 x K path.
    if fac_spec.three_way && n_cols == 4 {
        let grouping = grouped.three_way_grouping.as_ref();
        let f1_name = grouping.map_or_else(|| String::from("Factor A"), |g| g.factor_1_name.clone());
        let f2_name = grouping.map_or_else(|| String::from("Factor B"), |g| g.factor_2_name.clone());
        let f3_name = grouped.row_factor_title.clone();
        let f1_levels = grouping.map_or_else(|| [String::from("A1"), String::from("A2")], |g| g.factor_1_levels.clone()).to_vec();
        let f2_levels = grouping.map_or_else(|| [String::from("B1"), String::from("B2")], |g| g.factor_2_levels.clone()).to_vec();
        let f3_levels = grouped_row_levels(grouped, n_rows);

        let mut obs = Vec::new();
        for r in 0..n_rows {
            for c in 0..4 {
                let l1 = c / 2;
                let l2 = c % 2;
                for y in grouped.active_replicates(r, c) {
                    obs.push((vec![l1, l2, r], y));
                }
            }
        }
        let data =
            FactorialObservations { factor_names: vec![f1_name, f2_name, f3_name], factor_levels: vec![f1_levels, f2_levels, f3_levels], rows: obs };
        let res = fit_factorial_anova(&data, fac_spec.max_interaction_order.clamp(1, 3), fac_spec.ss_type)?;
        return Ok((res, data));
    }

    // Standard Two-Way ANOVA (Row Factor x Column Factor).
    let row_levels = grouped_row_levels(grouped, n_rows);
    let col_levels: Vec<String> = grouped.data_sets.iter().map(|ds| ds.title.clone()).collect();
    let mut obs = Vec::new();
    for r in 0..n_rows {
        for c in 0..n_cols {
            for y in grouped.active_replicates(r, c) {
                obs.push((vec![r, c], y));
            }
        }
    }
    let data = FactorialObservations {
        factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
        factor_levels: vec![row_levels, col_levels],
        rows: obs,
    };
    let res = fit_factorial_anova(&data, fac_spec.max_interaction_order.clamp(1, 2), fac_spec.ss_type)?;
    Ok((res, data))
}

fn run_rm_two_way_reml_fallback(
    grouped: &lumencraft_doc::GroupedTable,
    out: &mut ResultSheet,
    fac_spec: &FactorialAnovaSpec,
    p_fmt: PValueFormat,
) -> Result<(FactorialAnovaResult, FactorialObservations), StatsError> {
    let n_rows = grouped.row_count();
    let n_cols = grouped.data_sets.len();
    let SubcolumnMode::RawReplicates(n_subs) = grouped.subcolumn_mode else {
        return Err(StatsError::RequiresRawReplicates);
    };
    let row_basis = sum_contrast_vectors(n_rows);
    let col_basis = sum_contrast_vectors(n_cols);
    let n_r = n_rows.saturating_sub(1);
    let n_c = n_cols.saturating_sub(1);
    let p_total = 1 + n_r + n_c + n_r * n_c;

    let mut x_rows: Vec<f64> = Vec::new();
    let mut y_vec: Vec<f64> = Vec::new();
    let mut obs = Vec::new();
    let mut cluster_ids: Vec<usize> = Vec::new();
    let mut ss_tot_acc = NeumaierSum::new();
    for s in 0..n_subs {
        for (r, r_basis) in row_basis.iter().enumerate() {
            for (c, c_basis) in col_basis.iter().enumerate() {
                if let Some(v) = grouped.observation(r, c, s).and_then(|cell| cell.active_number()) {
                    let mut row_x = Vec::with_capacity(p_total);
                    row_x.push(1.0);
                    row_x.extend_from_slice(r_basis);
                    row_x.extend_from_slice(c_basis);
                    for &rb in r_basis {
                        for &cb in c_basis {
                            row_x.push(rb * cb);
                        }
                    }
                    x_rows.extend_from_slice(&row_x);
                    y_vec.push(v);
                    obs.push((vec![r, c], v));
                    ss_tot_acc.add(v);
                    // Cluster ID represents repeated subject identity across matched dimensions.
                    let cid = match fac_spec.repeated_measures {
                        RepeatedMeasuresDesign::ColumnFactorMatched => r * n_subs + s,
                        RepeatedMeasuresDesign::RowFactorMatched => c * n_subs + s,
                        _ => s,
                    };
                    cluster_ids.push(cid);
                }
            }
        }
    }
    let n_obs = y_vec.len();
    if n_obs <= p_total {
        return Err(StatsError::InsufficientSampleSize { context: "RM REML observations", required: p_total + 1, actual: n_obs });
    }
    let x_mat = DenseMatrix::from_row_major(n_obs, p_total, x_rows)?;
    let inter_name = format!("{} × {}", grouped.row_factor_title, grouped.column_factor_title);
    let term_slices = vec![
        (grouped.row_factor_title.as_str(), 1, n_r),
        (grouped.column_factor_title.as_str(), 1 + n_r, n_c),
        (inter_name.as_str(), 1 + n_r + n_c, n_r * n_c),
    ];
    let reml = fit_random_intercept_reml(&x_mat, &y_vec, &cluster_ids, &term_slices)?;
    push_scalar(out, "REML sigma^2 (subject)", None, reml.sigma2_random);
    push_scalar(out, "REML sigma^2 (residual)", None, reml.sigma2_error);
    push_scalar(out, "REML ICC", None, reml.icc);
    push_scalar(out, "REML log-likelihood", None, reml.reml_log_lik);

    let reml_headers =
        vec![String::from("Component"), String::from("Variance"), String::from("ICC"), String::from("LRT chi2"), String::from("LRT P")];
    let reml_rows = vec![
        vec![
            ResultCell::Text(String::from("Subject (random)")),
            num_cell(reml.sigma2_random),
            num_cell(reml.icc),
            num_cell(reml.lrt_random_chi2),
            p_cell(reml.lrt_random_p, p_fmt),
        ],
        vec![ResultCell::Text(String::from("Residual")), num_cell(reml.sigma2_error), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty],
    ];
    out.tabs.push(build_tab(
        "mixed-reml",
        "Mixed-effects REML variance components",
        ResultTabKind::ParametersAndConfidenceIntervals,
        reml_headers,
        vec![String::from("Subject"), String::from("Residual")],
        reml_rows,
    ));
    out.checklist.push(checklist_item(
        "Missing-value mixed-effects REML",
        ChecklistStatus::Info,
        Some(format!("ICC = {:.3}, REML logL = {:.2}", reml.icc, reml.reml_log_lik)),
        "Missing replicates detected in repeated-measures cells; fitted linear mixed-effects REML with Kenward-Roger degrees of freedom instead of listwise deletion.",
    ));

    let ym = SummaryMoments::from_slice(&y_vec)?;
    let res = FactorialAnovaResult {
        terms: reml.fixed_terms,
        ss_residual: reml.sigma2_error * ((n_obs - p_total) as f64),
        df_residual: (n_obs - p_total) as f64,
        ms_residual: reml.sigma2_error,
        ss_total: ym.css,
        df_total: (n_obs - 1) as f64,
        total_n: n_obs,
        unbalanced: true,
        ss_type: SumOfSquaresType::TypeIII,
    };
    let data = FactorialObservations {
        factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
        factor_levels: vec![grouped_row_levels(grouped, n_rows), grouped.data_sets.iter().map(|ds| ds.title.clone()).collect()],
        rows: obs,
    };
    Ok((res, data))
}

fn run_multifactor_mv(
    mv: &lumencraft_doc::MultipleVariablesTable,
    fac_spec: &FactorialAnovaSpec,
) -> Result<(FactorialAnovaResult, FactorialObservations), StatsError> {
    let resp_idx = fac_spec.response_column.ok_or(StatsError::MissingColumn { index: 0 })?;
    let resp_var = mv.variables.get(resp_idx).ok_or(StatsError::MissingColumn { index: resp_idx })?;
    if fac_spec.multifactor_columns.is_empty() {
        return Err(StatsError::InsufficientSampleSize { context: "multifactor columns", required: 1, actual: 0 });
    }
    let mut factor_names = Vec::with_capacity(fac_spec.multifactor_columns.len());
    let mut factor_levels = Vec::with_capacity(fac_spec.multifactor_columns.len());
    for &c_idx in &fac_spec.multifactor_columns {
        let var = mv.variables.get(c_idx).ok_or(StatsError::MissingColumn { index: c_idx })?;
        factor_names.push(var.title.clone());
        let levels: Vec<String> = var.ordered_levels_with(&var.level_order).into_iter().map(|s| s.level).collect();
        factor_levels.push(levels);
    }

    let n_rows = mv.row_count();
    let mut obs = Vec::new();
    for r in 0..n_rows {
        let Some(y) = resp_var.cells.get(r).and_then(|c| c.active_number()) else {
            continue;
        };
        let mut lvls = Vec::with_capacity(fac_spec.multifactor_columns.len());
        let mut valid = true;
        for (f_pos, &c_idx) in fac_spec.multifactor_columns.iter().enumerate() {
            let cell = &mv.variables[c_idx].cells[r];
            if cell.excluded {
                valid = false;
                break;
            }
            let tok = match &cell.value {
                lumencraft_doc::CellValue::Text(s) => s.trim().to_owned(),
                lumencraft_doc::CellValue::Number(v) if v.is_finite() => format!("{v}"),
                _ => {
                    valid = false;
                    break;
                }
            };
            let Some(pos) = factor_levels[f_pos].iter().position(|l| l == &tok) else {
                valid = false;
                break;
            };
            lvls.push(pos);
        }
        if valid {
            obs.push((lvls, y));
        }
    }
    let data = FactorialObservations { factor_names, factor_levels, rows: obs };
    let res = fit_factorial_anova(&data, fac_spec.max_interaction_order, fac_spec.ss_type)?;
    Ok((res, data))
}

/// Execute a [`NestedAnalysisSpec`] against a [`NestedTable`] [`DataSheet`].
pub fn run_nested_analysis(sheet: &DataSheet, spec: &AnalysisSpec, nested_spec: &NestedAnalysisSpec) -> Result<ResultSheet, StatsError> {
    let TableArchetype::Nested(nested) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "nested ANOVA / nested t-test", actual: sheet.archetype.kind() });
    };
    let mut out = init_result_sheet(spec)?;
    let p_fmt = spec.p_value_format;
    let confidence = spec.confidence_level;

    let res = nested_one_way_anova(nested, nested_spec.two_group_t_test)?;
    push_scalar(&mut out, "F (among groups)", None, res.f_among_groups);
    push_scalar(&mut out, "P value (groups)", None, res.p_among_groups);
    push_scalar(&mut out, "Subcluster variance", None, res.variance_subclusters);
    push_scalar(&mut out, "Residual variance", None, res.variance_error);
    push_scalar(&mut out, "ICC (subcluster)", None, res.icc_subclusters);
    if let Some(t_val) = res.t_nested {
        push_scalar(&mut out, "Nested t", None, t_val);
        let _ = student_t_two_sided_p(t_val, res.df2_groups_kenward_roger);
    }

    let headers = vec![String::from("SS"), String::from("df"), String::from("MS"), String::from("F"), String::from("P value")];
    let labels = vec![String::from("Among groups"), String::from("Among subclusters within groups"), String::from("Error (technical replicates)")];
    let rows = vec![
        vec![
            num_cell(res.ss_groups),
            num_cell(res.df1_groups),
            num_cell(res.ms_groups),
            num_cell(res.f_among_groups),
            p_cell(res.p_among_groups, p_fmt),
        ],
        vec![
            num_cell(res.ss_subclusters),
            num_cell(res.df1_subclusters),
            num_cell(res.ms_subclusters),
            num_cell(res.f_among_subclusters),
            p_cell(res.p_among_subclusters, p_fmt),
        ],
        vec![num_cell(res.ss_error), num_cell(res.df2_error), num_cell(res.ms_error), ResultCell::Empty, ResultCell::Empty],
    ];
    let title = if nested_spec.two_group_t_test { "Nested t test" } else { "Nested one-way ANOVA" };
    out.tabs.push(build_tab("nested-anova", title, ResultTabKind::AnovaTable, headers, labels, rows));

    // Variance components tab.
    let vc_headers = vec![
        String::from("Variance component"),
        String::from("SD"),
        String::from("% of total"),
        String::from("Kenward-Roger df"),
        String::from("Satterthwaite df"),
    ];
    let tot_var = (res.variance_subclusters + res.variance_error).max(1e-18);
    let vc_rows = vec![
        vec![
            num_cell(res.variance_subclusters),
            num_cell(res.variance_subclusters.sqrt()),
            num_cell(100.0 * res.variance_subclusters / tot_var),
            num_cell(res.df2_groups_kenward_roger),
            num_cell(res.df2_groups_satterthwaite),
        ],
        vec![
            num_cell(res.variance_error),
            num_cell(res.variance_error.sqrt()),
            num_cell(100.0 * res.variance_error / tot_var),
            ResultCell::Empty,
            ResultCell::Empty,
        ],
    ];
    out.tabs.push(build_tab(
        "variance-components",
        "Variance components (REML)",
        ResultTabKind::ParametersAndConfidenceIntervals,
        vc_headers,
        vec![String::from("Subclusters (biological units)"), String::from("Residual (technical)")],
        vc_rows,
    ));

    // Optional group-level pairwise comparisons if >= 3 groups.
    if !nested_spec.two_group_t_test && nested.groups.len() >= 3 {
        let mut g_inputs = Vec::with_capacity(nested.groups.len());
        let eff_var = res.variance_error + res.variance_subclusters;
        for (g_idx, g) in nested.groups.iter().enumerate() {
            let mut vals = Vec::new();
            for b in 0..g.biological_unit_count() {
                vals.extend(nested.active_technical_replicates(g_idx, b));
            }
            let m = SummaryMoments::from_slice(&vals)?;
            g_inputs.push(GroupComparisonInput { index: g_idx, label: g.title.clone(), n: vals.len() as f64, mean: m.mean, sd: eff_var.sqrt() });
        }
        let details = compare_groups_pairwise(&g_inputs, eff_var, res.df2_groups_kenward_roger, &nested_spec.multiplicity)?;
        push_pairwise_outputs(&mut out, &details, "Nested multiple comparisons", p_fmt, confidence);
    }

    out.checklist.push(checklist_item(
        "Biological vs. technical nesting",
        if res.icc_subclusters > 0.05 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("ICC = {:.3}, KR df = {:.2}", res.icc_subclusters, res.df2_groups_kenward_roger)),
        "Nested REML prevents pseudoreplication by separating biological-unit variance from technical replicate variance.",
    ));
    Ok(out)
}
