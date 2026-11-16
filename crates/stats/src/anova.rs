//! One-way, two-way, three-way, N-way factorial ANOVA, repeated-measures sphericity corrections
//! (Mauchly, Greenhouse–Geisser, Huynh–Feldt), mixed-effects REML (Satterthwaite & Kenward–Roger
//! degrees of freedom), nested hierarchical ANOVA / nested *t*-test, and effect sizes with exact
//! noncentral-*F* confidence intervals.
//!
//! Classical fixed-effects ANOVA partitions (one-way, factorial, repeated-measures, nested) follow
//! the textbook treatments in Zar (2010), Maxwell & Delaney (2004), and Searle (1987); ordinary
//! one-way results are checked against the NIST StRD ANOVA datasets.
//!
//! References:
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - Searle, S. R. (1987). *Linear Models for Unbalanced Data*. Wiley.
//! - NIST Statistical Reference Datasets (StRD), Analysis of Variance:
//!   <https://www.itl.nist.gov/div898/strd/anova/anova.html>
//! - Bartlett, M. S. (1937). Properties of sufficiency and statistical tests. *Proceedings of the
//!   Royal Society of London A* 160(901):268–282. doi:10.1098/rspa.1937.0109
//! - Brown, M. B., & Forsythe, A. B. (1974a). Robust tests for the equality of variances. *Journal
//!   of the American Statistical Association* 69(346):364–367. doi:10.1080/01621459.1974.10482955
//! - Brown, M. B., & Forsythe, A. B. (1974b). The small sample behavior of some statistics which
//!   test the equality of several means. *Technometrics* 16(1):129–132.
//! - Welch, B. L. (1951). On the comparison of several mean values: an alternative approach.
//!   *Biometrika* 38(3/4):330–336.
//! - Mauchly, J. W. (1940). Significance test for sphericity of a normal *n*-variate distribution.
//!   *Annals of Mathematical Statistics* 11(2):204–209. doi:10.1214/aoms/1177731915
//! - Greenhouse, S. W., & Geisser, S. (1959). On methods in the analysis of profile data.
//!   *Psychometrika* 24(2):95–112. doi:10.1007/BF02289823
//! - Huynh, H., & Feldt, L. S. (1976). Estimation of the Box correction for degrees of freedom from
//!   sample data in randomized block and split-plot designs. *Journal of Educational Statistics*
//!   1(1):69–82. doi:10.3102/10769986001001069
//! - Kruskal, W. H., & Wallis, W. A. (1952). Use of ranks in one-criterion variance analysis.
//!   *Journal of the American Statistical Association* 47(260):583–621.
//!   doi:10.1080/01621459.1952.10483441
//! - Friedman, M. (1937). The use of ranks to avoid the assumption of normality implicit in the
//!   analysis of variance. *Journal of the American Statistical Association* 32(200):675–701.
//!   doi:10.1080/01621459.1937.10503522
//! - Kendall, M. G., & Babington Smith, B. (1939). The problem of *m* rankings. *Annals of
//!   Mathematical Statistics* 10(3):275–287.
//! - Steiger, J. H. (2004). Beyond the F test: effect size confidence intervals and tests of close
//!   fit in the analysis of variance and contrast analysis. *Psychological Methods* 9(2):164–182.
//!   doi:10.1037/1082-989X.9.2.164
//! - Olejnik, S., & Algina, J. (2003). Generalized eta and omega squared statistics: measures of
//!   effect size for some common research designs. *Psychological Methods* 8(4):434–447.
//!   doi:10.1037/1082-989X.8.4.434
//! - Cohen, J. (1988). *Statistical Power Analysis for the Behavioral Sciences* (2nd ed.).
//!   Lawrence Erlbaum.
//! - Patterson, H. D., & Thompson, R. (1971). Recovery of inter-block information when block sizes
//!   are unequal. *Biometrika* 58(3):545–554. doi:10.1093/biomet/58.3.545
//! - Self, S. G., & Liang, K.-Y. (1987). Asymptotic properties of maximum likelihood estimators and
//!   likelihood ratio tests under nonstandard conditions. *Journal of the American Statistical
//!   Association* 82(398):605–610. doi:10.1080/01621459.1987.10478472
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114. doi:10.2307/3002019
//! - Giesbrecht, F. G., & Burns, J. C. (1985). Two-stage analysis based on a mixed model:
//!   large-sample asymptotic theory and small-sample simulation results. *Biometrics*
//!   41(2):477–486. doi:10.2307/2530872
//! - Kenward, M. G., & Roger, J. H. (1997). Small sample inference for fixed effects from
//!   restricted maximum likelihood. *Biometrics* 53(3):983–997. doi:10.2307/2533558
//! - Fai, A. H.-T., & Cornelius, P. L. (1996). Approximate F-tests of multiple degree of freedom
//!   hypotheses in generalized least squares analyses of unbalanced split-plot experiments.
//!   *Journal of Statistical Computation and Simulation* 54(4):363–378. doi:10.1080/00949659608811740
//! - Gilmour, A. R., Thompson, R., & Cullis, B. R. (1995). Average Information REML: an efficient
//!   algorithm for variance parameter estimation in linear mixed models. *Biometrics*
//!   51(4):1440–1450. doi:10.2307/2533274
//! - Tomczak, M., & Tomczak, E. (2014). The need to report effect size estimates revisited. An
//!   overview of some recommended measures of effect size. *Trends in Sport Sciences* 1(21):19–25.

use lumencraft_doc::{
    AnalysisSpec, ChecklistItem, ChecklistStatus, DataSheet, FactorialAnovaSpec, MultiplicityMethod, MultiplicitySpec, NestedAnalysisSpec,
    OneWayAnovaSpec, OneWayMethod, PValueFormat, PairwiseComparisonRecord, RepeatedMeasuresDesign, ResultCell, ResultSheet, ResultTab, ResultTabKind,
    SubcolumnMode, SumOfSquaresType, TableArchetype,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{chi2_sf, f_sf, noncentral_f_ci};
use lumencraft_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares, sym_eigen};
use lumencraft_math::sum::{NeumaierSum, SummaryMoments};
use serde::{Deserialize, Serialize};

use crate::column::shapiro_wilk;
use crate::common::{
    append_ignored_text_checklist_and_footnote, append_listwise_filtered_checklist_and_footnote, build_tab, checklist_item, extract_numeric_columns,
    init_result_sheet, num_cell, opt_ci_cell, opt_num_cell, p_cell, push_scalar, quantile_type6_sorted, rank_with_ties, resolve_selected_indices,
    sanitize_result_sheet_floats, sorted_finite, validate_finite_slice,
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
    #[serde(default)]
    pub available: bool,
}

fn clamp_ss_roundoff(ss: f64, data_max_abs: f64, obs_count: usize) -> f64 {
    let ss_clean = ss.max(0.0);
    if data_max_abs == 0.0 || !data_max_abs.is_finite() {
        return ss_clean;
    }
    let roundoff_amp = 64.0 * f64::EPSILON * data_max_abs;
    let tol = roundoff_amp * roundoff_amp * (obs_count.max(1) as f64);
    if ss_clean <= tol { 0.0 } else { ss_clean }
}

fn f_ratio_p(ms_num: f64, ms_den: f64, df1: f64, df2: f64) -> Result<(f64, f64), StatsError> {
    if ms_den > 0.0 {
        let f = ms_num / ms_den;
        Ok((f, f_sf(f, df1, df2)?))
    } else if ms_num > 0.0 {
        Ok((f64::INFINITY, 0.0))
    } else {
        Ok((0.0, 1.0))
    }
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

/// Sphericity epsilon adjustment applied to repeated-measures two-way *F*-test *p*-values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RmSphericityCorrection {
    #[default]
    None,
    GreenhouseGeisser,
    HuynhFeldt,
}

/// Per-factor sphericity diagnostics for two-way repeated-measures and split-plot ANOVA.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RmTwoWaySphericities {
    pub row: Option<SphericityResult>,
    pub column: Option<SphericityResult>,
    pub interaction: Option<SphericityResult>,
}

/// Two-way repeated-measures / split-plot ANOVA output with factor-specific sphericity diagnostics
/// and sphericity-adjusted *p*-values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RmTwoWayAnovaResult {
    pub anova: FactorialAnovaResult,
    pub design: RepeatedMeasuresDesign,
    pub sphericities: RmTwoWaySphericities,
    pub correction: RmSphericityCorrection,
    pub term_error_dfs: [f64; 3],
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

/// Multi-variance-component linear mixed-effects REML fit (`V = sigma_e^2 I + sum_k sigma_k^2 G_k`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiVcRemlResult {
    pub variance_components: Vec<(String, f64)>,
    pub sigma2_error: f64,
    pub reml_log_lik: f64,
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
/// noncentral-*F* confidence intervals for one-way designs.
///
/// - `omega^2 = (SS_effect - df_effect * MS_error) / (SS_total + MS_error)` (Hays estimator per
///   Olejnik & Algina, 2003).
/// - Cohen's `f = sqrt(eta_p^2 / (1 - eta_p^2))` (Cohen, 1988, ch. 8; returns `+inf` when
///   `eta_p^2 = 1.0`).
/// - For one-way designs (`SS_effect + SS_error == SS_total`), `N = df_effect + df_error + 1` is
///   exact and `eta_squared == partial_eta_squared`, so Steiger (2004) noncentral-*F* intervals are
///   computed automatically. For multi-way designs where `SS_effect + SS_error != SS_total`, the
///   omitted factor degrees of freedom are unknown from `(df_effect, df_error)` alone and
///   `eta_squared != partial_eta_squared`; call [`compute_anova_effect_sizes_for_n`] with the true
///   `total_n` instead (this 6-argument helper returns `None` for the confidence intervals).
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
/// For one-way designs (`SS_effect + SS_error == SS_total`), `eta_squared_ci` coincides with
/// `partial_eta_squared_ci`. For multi-way designs (`SS_effect + SS_error != SS_total`),
/// `partial_eta_squared_ci` uses `lambda / (lambda + total_n)` per Steiger (2004, eq. 16) while
/// `eta_squared_ci` is left `None` because the noncentral-*F* pivot parameterizes `partial eta^2`,
/// not classical `eta^2 = SS_effect / SS_total`.
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

    let f_obs = if ms_error > 0.0 && df_effect > 0.0 { (ss_effect / df_effect) / ms_error } else { 0.0 };
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
    let is_one_way = ((ss_effect + ss_error) - ss_total).abs() <= 1e-9 * ss_total.max(1e-30);

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

// ============================================================================
// One-Way ANOVA (Ordinary, Welch/BF, RM, Kruskal–Wallis, Friedman)
// ============================================================================

/// Ordinary one-way ANOVA using two-pass `SummaryMoments` and `NeumaierSum` shifted by the grand
/// mean so stiff NIST StRD series (`SmLs01..09`, `SiRstv`, `AtmWtAg`) retain full precision.
///
/// Fixed-effects one-way ANOVA partition `SS_total = SS_between + SS_within` with
/// `F = MS_between / MS_within` on `(k - 1, N - k)` df (Zar, 2010, ch. 10), verified against the
/// NIST StRD ANOVA certified values.
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
            bartlett_chi2: f64::NAN,
            bartlett_df: df_between,
            bartlett_p: f64::NAN,
            brown_forsythe_f: f64::NAN,
            brown_forsythe_df1: df_between,
            brown_forsythe_df2: df_within,
            brown_forsythe_p: f64::NAN,
            available: false,
        },
        effect_sizes,
    })
}

/// One-way ANOVA from pre-summarized `(label, n, mean, sd)` records.
///
/// Same partition as [`ordinary_one_way_anova`] with `SS_within = sum (n_i - 1) s_i^2` and
/// `SS_between = sum n_i (mean_i - grand_mean)^2` (Zar, 2010, ch. 10).
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
            bartlett_chi2: f64::NAN,
            bartlett_df: dfb,
            bartlett_p: f64::NAN,
            brown_forsythe_f: f64::NAN,
            brown_forsythe_df1: dfb,
            brown_forsythe_df2: dfw,
            brown_forsythe_p: f64::NAN,
            available: false,
        },
        effect_sizes,
    })
}

/// Welch's and Brown–Forsythe's unequal-variance one-way ANOVA from group summaries.
///
/// Welch (1951) `W` with weights `w_i = n_i / s_i^2`, denominator
/// `1 + 2 (k - 2) / (k^2 - 1) * sum (1 - w_i / sum w)^2 / (n_i - 1)`, and
/// `df2 = (k^2 - 1) / (3 sum (1 - w_i / sum w)^2 / (n_i - 1))`. Brown & Forsythe (1974b)
/// `F* = sum n_i (mean_i - grand_mean)^2 / sum (1 - n_i / N) s_i^2` on `(k - 1, f)` df, with `f`
/// from the Satterthwaite (1946) approximation.
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
///
/// Subjects × treatments (randomized-block) partition with `F = MS_treatment / MS_residual` on
/// `(k - 1, (n - 1)(k - 1))` df (Zar, 2010, ch. 12; Maxwell & Delaney, 2004, ch. 11). Corrected
/// p-values multiply both df by Greenhouse & Geisser (1959) or Huynh & Feldt (1976) epsilon.
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
    let mut data_max_abs = 0.0_f64;

    for row in blocks {
        let mut rsum = NeumaierSum::new();
        for (c, &v) in row.iter().enumerate() {
            data_max_abs = data_max_abs.max(v.abs());
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

    let obs_count = n * k;
    let sst = clamp_ss_roundoff(ss_treat.total(), data_max_abs, obs_count);
    let sss = clamp_ss_roundoff(ss_subj.total(), data_max_abs, obs_count);
    let ssr = clamp_ss_roundoff(ss_resid.total(), data_max_abs, obs_count);
    let ss_total = sst + sss + ssr;

    let dft = kf - 1.0;
    let dfs = nf - 1.0;
    let dfr = dft * dfs;
    let mst = sst / dft;
    let mss = sss / dfs;
    let msr = ssr / dfr;

    let (f_treat, p_uncorr) = f_ratio_p(mst, msr, dft, dfr)?;
    let (f_subj, p_subj) = f_ratio_p(mss, msr, dfs, dfr)?;

    let sphericity = compute_sphericity(blocks)?;
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

fn helmert_contrast_matrix(k: usize) -> Result<DenseMatrix, StatsError> {
    if k <= 1 {
        return Ok(DenseMatrix::zeros(0, k)?);
    }
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
    Ok(h_mat)
}

fn unit_row_vector(k: usize) -> Result<DenseMatrix, StatsError> {
    let val = if k > 0 { 1.0 / (k as f64).sqrt() } else { 0.0 };
    Ok(DenseMatrix::from_row_major(1, k, vec![val; k])?)
}

fn kronecker_dense(a: &DenseMatrix, b: &DenseMatrix) -> Result<DenseMatrix, StatsError> {
    let ra = a.nrows();
    let ca = a.ncols();
    let rb = b.nrows();
    let cb = b.ncols();
    let rows = ra * rb;
    let cols = ca * cb;
    let mut data = vec![0.0_f64; rows * cols];
    for ia in 0..ra {
        for ja in 0..ca {
            let av = a.get(ia, ja).unwrap_or(0.0);
            for ib in 0..rb {
                let r = ia * rb + ib;
                for jb in 0..cb {
                    let c = ja * cb + jb;
                    data[r * cols + c] = av * b.get(ib, jb).unwrap_or(0.0);
                }
            }
        }
    }
    Ok(DenseMatrix::from_row_major(rows, cols, data)?)
}

fn pooled_within_group_cov(groups: &[Vec<Vec<f64>>]) -> Result<DenseMatrix, StatsError> {
    let k = groups.first().and_then(|g| g.first()).map_or(0, Vec::len);
    if k == 0 {
        return Err(StatsError::EmptyInput);
    }
    let mut total_df = 0.0_f64;
    let mut s_sum = vec![0.0_f64; k * k];
    for grp in groups {
        let n_g = grp.len();
        if n_g < 2 {
            continue;
        }
        let ng_f = n_g as f64;
        let mut means = vec![0.0_f64; k];
        for row in grp {
            if row.len() != k {
                return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: k, actual: row.len() }));
            }
            for (c, &v) in row.iter().enumerate() {
                means[c] += v / ng_f;
            }
        }
        total_df += ng_f - 1.0;
        for r in 0..k {
            for c in r..k {
                let mut acc = NeumaierSum::new();
                for row in grp {
                    acc.add((row[r] - means[r]) * (row[c] - means[c]));
                }
                s_sum[r * k + c] += acc.total();
            }
        }
    }
    if total_df <= 0.0 {
        return Err(StatsError::InsufficientSampleSize { context: "within-group covariance df", required: 2, actual: 0 });
    }
    for r in 0..k {
        for c in r..k {
            let val = s_sum[r * k + c] / total_df;
            s_sum[r * k + c] = val;
            s_sum[c * k + r] = val;
        }
    }
    Ok(DenseMatrix::from_row_major(k, k, s_sum)?)
}

/// Compute Mauchly's `W` and Greenhouse–Geisser / Huynh–Feldt sphericity epsilons from a covariance
/// matrix `S` (`df_e` error df, `n_subj` total subjects) via an orthonormal contrast projection `M S M^T`.
///
/// - Mauchly (1940) `W = det(C) / (tr(C)/p)^p` using chi-square approximation on `p(p + 1)/2 - 1` df.
/// - Greenhouse & Geisser (1959) `epsilon_hat = (sum lambda)^2 / (p sum lambda^2)` bounded in `[1/p, 1]`.
/// - Huynh & Feldt (1976, p. 74) `epsilon_tilde = (N p epsilon_hat - 2) / (p (df_e - p epsilon_hat))`,
///   clamped to `[epsilon_hat, 1]` (returning `1` when `df_e <= p * epsilon_hat`).
fn compute_sphericity_from_cov_contrast(s_cov: &DenseMatrix, contrast: &DenseMatrix, df_e: f64, n_subj: f64) -> Result<SphericityResult, StatsError> {
    let p = contrast.nrows();
    if p <= 1 || df_e <= 1.0 {
        return Ok(SphericityResult {
            mauchly_w: 1.0,
            mauchly_chi2: 0.0,
            mauchly_df: 0.0,
            mauchly_p: 1.0,
            greenhouse_geisser_eps: 1.0,
            huynh_feldt_eps: 1.0,
        });
    }
    let c_cov = symmetrize_dense(&contrast.matmul(s_cov)?.matmul(&contrast.transpose())?)?;
    let eig = sym_eigen(&c_cov)?;
    let eigvals: Vec<f64> = eig.eigenvalues.iter().map(|&x| x.max(0.0)).collect();
    let tr: f64 = eigvals.iter().sum();
    let tr_sq: f64 = eigvals.iter().map(|&l| l * l).sum();
    if tr <= 0.0 || tr_sq <= 0.0 {
        return Ok(SphericityResult {
            mauchly_w: 1.0,
            mauchly_chi2: 0.0,
            mauchly_df: 0.0,
            mauchly_p: 1.0,
            greenhouse_geisser_eps: 1.0,
            huynh_feldt_eps: 1.0,
        });
    }
    let pf = p as f64;
    let eps_gg = ((tr * tr) / (pf * tr_sq)).clamp(1.0 / pf, 1.0);
    let hf_denom = pf * (df_e - pf * eps_gg);
    let eps_hf = if hf_denom > 0.0 { ((n_subj * pf * eps_gg - 2.0) / hf_denom).clamp(eps_gg, 1.0) } else { 1.0 };

    // Normalize eigenvalues by mean_eig = tr / p before taking logs so Mauchly W is scale-invariant.
    let mean_eig = tr / pf;
    let ln_w: f64 = eigvals.iter().map(|&v| (v / mean_eig).max(1e-15).ln()).sum::<f64>().min(0.0);
    let w = ln_w.exp().clamp(1e-15, 1.0);
    let m_df = 0.5 * pf * (pf + 1.0) - 1.0;
    let f_scale = (1.0 - (2.0 * pf * pf + pf + 2.0) / (6.0 * pf * df_e)).max(0.0);
    let chi2 = (-df_e * f_scale * ln_w).max(0.0);
    let mauchly_p = if m_df > 0.0 { chi2_sf(chi2, m_df)? } else { 1.0 };

    Ok(SphericityResult { mauchly_w: w, mauchly_chi2: chi2, mauchly_df: m_df, mauchly_p, greenhouse_geisser_eps: eps_gg, huynh_feldt_eps: eps_hf })
}

fn compute_sphericity(blocks: &[Vec<f64>]) -> Result<SphericityResult, StatsError> {
    let n = blocks.len();
    let k = blocks.first().map_or(0, Vec::len);
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
    let s_cov = pooled_within_group_cov(&[blocks.to_vec()])?;
    let h_mat = helmert_contrast_matrix(k)?;
    let nf = n as f64;
    compute_sphericity_from_cov_contrast(&s_cov, &h_mat, nf - 1.0, nf)
}

/// Kruskal–Wallis one-way ANOVA on ranks with tie correction and rank epsilon-squared effect size.
///
/// `H = 12 / (N (N + 1)) sum R_i^2 / n_i - 3 (N + 1)`, divided by the tie correction
/// `1 - sum (t^3 - t) / (N^3 - N)` and referred to `chi^2(k - 1)` (Kruskal & Wallis, 1952;
/// Zar, 2010, ch. 10). When all values are tied (`tie_correction = 0`), reports `H = 0` and `p = 1`.
///
/// Effect size is rank epsilon-squared `epsilon^2_R = H / (N - 1)` (Tomczak & Tomczak, 2014, eq. 5).
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
    let (h_corr, tie_factor) = if tie_denom > 1e-15 { (h_raw.max(0.0) / tie_denom, tie_denom) } else { (0.0, 0.0) };
    let df = (groups.len() - 1) as f64;
    let p_value = chi2_sf(h_corr, df)?;
    let eps2 = (h_corr / (nf - 1.0)).clamp(0.0, 1.0);

    Ok(KruskalWallisResult { h_statistic: h_corr, df, p_value, tie_correction: tie_factor, epsilon_squared: eps2, group_mean_ranks, total_n })
}

/// Friedman nonparametric repeated-measures rank test on complete blocks (`n` rows × `k` conditions).
///
/// Friedman (1937) statistic with within-block tie correction,
/// `Q = 12 sum (R_j - n (k + 1)/2)^2 / (n k (k + 1) - sum (t^3 - t) / (k - 1))`, referred to
/// `chi^2(k - 1)` (Zar, 2010, ch. 12). Kendall's `W = Q / (n (k - 1))` (Kendall & Babington
/// Smith, 1939).
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
///
/// Sums of squares come from full-vs-reduced model comparisons (Maxwell & Delaney, 2004, ch. 7;
/// Searle, 1987). Type I: sequential RSS reductions in term order. Type III: RSS increase from
/// dropping each term's sum-to-zero-coded columns from the full model. With no empty cells this
/// matches the usual Type III hypotheses on unweighted marginal means. With empty cells it tests
/// whatever estimable function remains, which is not the Type IV construction.
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
                let df_t = full_sol.rank.saturating_sub(red_sol.rank) as f64;
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
                let df_t = seq_sol.rank.saturating_sub(prev_rank) as f64;
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
    if df <= 0.0 {
        return Ok(FactorialAnovaTerm {
            name: name.to_owned(),
            ss: 0.0,
            df: 0.0,
            ms: 0.0,
            f_statistic: None,
            p_value: None,
            partial_eta_squared: None,
            eta_squared: None,
        });
    }
    let ms = ss / df;
    let f_val = if ms_resid > 0.0 {
        ms / ms_resid
    } else if ms > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
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
        if t.df > 0.0
            && let Some(f_val) = t.f_statistic
        {
            t.p_value = Some(f_sf(f_val, t.df, res.df_residual.max(1.0))?);
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

fn apply_rm_sphericity_p(
    f_val: f64,
    df_num: f64,
    df_den: f64,
    sph: Option<&SphericityResult>,
    correction: RmSphericityCorrection,
) -> Result<f64, StatsError> {
    let eps = match (correction, sph) {
        (RmSphericityCorrection::None, _) | (_, None) => 1.0,
        (RmSphericityCorrection::GreenhouseGeisser, Some(s)) => s.greenhouse_geisser_eps,
        (RmSphericityCorrection::HuynhFeldt, Some(s)) => s.huynh_feldt_eps,
    };
    let adj_df1 = (df_num * eps).max(1e-12);
    let adj_df2 = (df_den * eps).max(1e-12);
    Ok(f_sf(f_val, adj_df1, adj_df2)?)
}

fn compute_rm_two_way_sphericities(
    subject_data: &[Vec<Vec<f64>>],
    a: usize,
    b: usize,
    rm_design: RepeatedMeasuresDesign,
) -> Result<RmTwoWaySphericities, StatsError> {
    let s_count = subject_data.len();
    let sf = s_count as f64;
    let af = a as f64;
    let bf = b as f64;
    match rm_design {
        RepeatedMeasuresDesign::BothFactorsMatched => {
            let mut y_flat = Vec::with_capacity(s_count);
            for mat in subject_data {
                let mut row = Vec::with_capacity(a * b);
                for r_vec in mat {
                    row.extend_from_slice(r_vec);
                }
                y_flat.push(row);
            }
            let s_cov = pooled_within_group_cov(&[y_flat])?;
            let ha = helmert_contrast_matrix(a)?;
            let hb = helmert_contrast_matrix(b)?;
            let ua = unit_row_vector(a)?;
            let ub = unit_row_vector(b)?;
            let m_row = kronecker_dense(&ha, &ub)?;
            let m_col = kronecker_dense(&ua, &hb)?;
            let m_int = kronecker_dense(&ha, &hb)?;
            let df_e = sf - 1.0;
            Ok(RmTwoWaySphericities {
                row: Some(compute_sphericity_from_cov_contrast(&s_cov, &m_row, df_e, sf)?),
                column: Some(compute_sphericity_from_cov_contrast(&s_cov, &m_col, df_e, sf)?),
                interaction: Some(compute_sphericity_from_cov_contrast(&s_cov, &m_int, df_e, sf)?),
            })
        }
        RepeatedMeasuresDesign::ColumnFactorMatched => {
            let mut groups = vec![Vec::with_capacity(s_count); a];
            for mat in subject_data {
                for i in 0..a {
                    groups[i].push(mat[i].clone());
                }
            }
            let s_pool = pooled_within_group_cov(&groups)?;
            let m_col = helmert_contrast_matrix(b)?;
            let n_subj = af * sf;
            let df_e = n_subj - af;
            let sph_col = compute_sphericity_from_cov_contrast(&s_pool, &m_col, df_e, n_subj)?;
            Ok(RmTwoWaySphericities { row: None, column: Some(sph_col), interaction: None })
        }
        RepeatedMeasuresDesign::RowFactorMatched => {
            let mut groups = vec![Vec::with_capacity(s_count); b];
            for mat in subject_data {
                for j in 0..b {
                    let mut vec_subj = Vec::with_capacity(a);
                    for r_vec in mat.iter().take(a) {
                        vec_subj.push(r_vec[j]);
                    }
                    groups[j].push(vec_subj);
                }
            }
            let s_pool = pooled_within_group_cov(&groups)?;
            let m_row = helmert_contrast_matrix(a)?;
            let n_subj = bf * sf;
            let df_e = n_subj - bf;
            let sph_row = compute_sphericity_from_cov_contrast(&s_pool, &m_row, df_e, n_subj)?;
            Ok(RmTwoWaySphericities { row: Some(sph_row), column: None, interaction: None })
        }
        RepeatedMeasuresDesign::None => Ok(RmTwoWaySphericities::default()),
    }
}

/// Repeated-measures two-way ANOVA (`RowFactorMatched`, `ColumnFactorMatched`, or
/// `BothFactorsMatched`) on complete balanced subject blocks `subject_data[subject][row][col]`,
/// returning separate sphericity diagnostics for all within-subject factors and applying the
/// requested [`RmSphericityCorrection`] (`None`, `GreenhouseGeisser`, or `HuynhFeldt`) directly
/// to the *F*-test *p*-values.
///
/// - When `BothFactorsMatched`, sphericity matrices come from the Kronecker orthonormal Helmert
///   contrasts `H_A x u_B`, `u_A x H_B`, and `H_A x H_B` on the subject covariance matrix (`df_e = S - 1`).
/// - For split-plot designs (`ColumnFactorMatched` / `RowFactorMatched`), sphericity is computed from
///   the pooled within-group covariance matrix on `df_error = N - g` (Huynh & Feldt, 1976, p. 71)
///   and applied to both the within-subject main effect and the interaction term.
pub fn fit_rm_two_way_anova_with_sphericity(
    subject_data: &[Vec<Vec<f64>>],
    row_name: &str,
    col_name: &str,
    rm_design: RepeatedMeasuresDesign,
    correction: RmSphericityCorrection,
) -> Result<RmTwoWayAnovaResult, StatsError> {
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
    let mut data_max_abs = 0.0_f64;

    for (s, mat) in subject_data.iter().enumerate() {
        if mat.len() != a {
            return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: a, actual: mat.len() }));
        }
        let mut s_acc = NeumaierSum::new();
        for (i, r_vec) in mat.iter().enumerate() {
            if r_vec.len() != b {
                return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: b, actual: r_vec.len() }));
            }
            for (j, &v) in r_vec.iter().enumerate() {
                if !v.is_finite() {
                    return Err(StatsError::Math(lumencraft_math::MathError::DomainError { name: "y", value: v }));
                }
                data_max_abs = data_max_abs.max(v.abs());
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

    let ss_a = clamp_ss_roundoff(row_means.iter().map(|&m| sf * bf * (m - gm).powi(2)).sum(), data_max_abs, total_n);
    let ss_b = clamp_ss_roundoff(col_means.iter().map(|&m| sf * af * (m - gm).powi(2)).sum(), data_max_abs, total_n);
    let mut ss_ab = 0.0_f64;
    for i in 0..a {
        for j in 0..b {
            let eff = cell_means[i][j] - row_means[i] - col_means[j] + gm;
            ss_ab += sf * eff * eff;
        }
    }
    let ss_ab = clamp_ss_roundoff(ss_ab, data_max_abs, total_n);
    let ss_s = clamp_ss_roundoff(subj_means.iter().map(|&m| af * bf * (m - gm).powi(2)).sum(), data_max_abs, total_n);
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
    let ss_sa = clamp_ss_roundoff(ss_sa, data_max_abs, total_n);
    let ss_sb = clamp_ss_roundoff(ss_sb, data_max_abs, total_n);
    let ss_sab = clamp_ss_roundoff(ss_sab, data_max_abs, total_n);

    let dfa = af - 1.0;
    let dfb = bf - 1.0;
    let dfab = dfa * dfb;
    let ss_total = ss_a + ss_b + ss_ab + ss_s + ss_sa + ss_sb + ss_sab;

    let sphericities = compute_rm_two_way_sphericities(subject_data, a, b, rm_design)?;
    let (err_a_ss, err_a_df, err_b_ss, err_b_df, err_ab_ss, err_ab_df, term_sphs) = match rm_design {
        RepeatedMeasuresDesign::BothFactorsMatched => (
            ss_sa,
            (sf - 1.0) * dfa,
            ss_sb,
            (sf - 1.0) * dfb,
            ss_sab,
            (sf - 1.0) * dfab,
            [sphericities.row.as_ref(), sphericities.column.as_ref(), sphericities.interaction.as_ref()],
        ),
        RepeatedMeasuresDesign::ColumnFactorMatched => {
            let ss_subj_within_a = ss_s + ss_sa;
            let df_subj_within_a = af * (sf - 1.0);
            let ss_err_within = ss_sb + ss_sab;
            let df_err_within = af * (sf - 1.0) * dfb;
            (
                ss_subj_within_a,
                df_subj_within_a,
                ss_err_within,
                df_err_within,
                ss_err_within,
                df_err_within,
                [None, sphericities.column.as_ref(), sphericities.column.as_ref()],
            )
        }
        RepeatedMeasuresDesign::RowFactorMatched => {
            let ss_subj_within_b = ss_s + ss_sb;
            let df_subj_within_b = bf * (sf - 1.0);
            let ss_err_within = ss_sa + ss_sab;
            let df_err_within = bf * (sf - 1.0) * dfa;
            (
                ss_err_within,
                df_err_within,
                ss_subj_within_b,
                df_subj_within_b,
                ss_err_within,
                df_err_within,
                [sphericities.row.as_ref(), None, sphericities.row.as_ref()],
            )
        }
        RepeatedMeasuresDesign::None => {
            let df_res = (total_n as f64) - af * bf;
            let ss_res = ss_s + ss_sa + ss_sb + ss_sab;
            (ss_res, df_res, ss_res, df_res, ss_res, df_res, [None, None, None])
        }
    };

    let make_term =
        |name: String, ss: f64, df: f64, e_ss: f64, e_df: f64, sph_opt: Option<&SphericityResult>| -> Result<FactorialAnovaTerm, StatsError> {
            let ms = ss / df.max(1.0);
            let ms_e = e_ss / e_df.max(1.0);
            let (f, _) = f_ratio_p(ms, ms_e, df.max(1.0), e_df.max(1.0))?;
            let p = apply_rm_sphericity_p(f, df.max(1.0), e_df.max(1.0), sph_opt, correction)?;
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
        make_term(row_name.to_owned(), ss_a, dfa, err_a_ss, err_a_df, term_sphs[0])?,
        make_term(col_name.to_owned(), ss_b, dfb, err_b_ss, err_b_df, term_sphs[1])?,
        make_term(format!("{row_name} × {col_name}"), ss_ab, dfab, err_ab_ss, err_ab_df, term_sphs[2])?,
    ];

    let df_resid = err_ab_df;
    let ms_resid = err_ab_ss / df_resid.max(1.0);
    let anova = FactorialAnovaResult {
        terms,
        ss_residual: err_ab_ss,
        df_residual: df_resid,
        ms_residual: ms_resid,
        ss_total,
        df_total: (total_n - 1) as f64,
        total_n,
        unbalanced: false,
        ss_type: SumOfSquaresType::TypeIII,
    };
    Ok(RmTwoWayAnovaResult { anova, design: rm_design, sphericities, correction, term_error_dfs: [err_a_df, err_b_df, err_ab_df] })
}

/// Repeated-measures two-way ANOVA (`RowFactorMatched`, `ColumnFactorMatched`, or `BothFactorsMatched`)
/// on complete balanced subject blocks `subject_data[subject][row][col]`, returning uncorrected *p*-values
/// alongside the primary within-subject [`SphericityResult`].
///
/// See [`fit_rm_two_way_anova_with_sphericity`] for full per-factor (`row`, `column`, `interaction`)
/// sphericity results and applied Greenhouse–Geisser / Huynh–Feldt *p*-values.
pub fn fit_rm_two_way_anova(
    subject_data: &[Vec<Vec<f64>>],
    row_name: &str,
    col_name: &str,
    rm_design: RepeatedMeasuresDesign,
) -> Result<(FactorialAnovaResult, Option<SphericityResult>), StatsError> {
    let res = fit_rm_two_way_anova_with_sphericity(subject_data, row_name, col_name, rm_design, RmSphericityCorrection::None)?;
    let primary_sph = match rm_design {
        RepeatedMeasuresDesign::BothFactorsMatched | RepeatedMeasuresDesign::ColumnFactorMatched => res.sphericities.column,
        RepeatedMeasuresDesign::RowFactorMatched => res.sphericities.row,
        RepeatedMeasuresDesign::None => None,
    };
    Ok((res.anova, primary_sph))
}

struct RiClusterStats {
    m_c: f64,
    sum_y: f64,
    yy: f64,
    sum_x: Vec<f64>,
    xy: Vec<f64>,
    xx: Vec<f64>,
}

struct OwnedRemlFisherMats {
    info_e: DenseMatrix,
    m_mats: Vec<DenseMatrix>,
    q_mats: Vec<Vec<DenseMatrix>>,
}

struct KrTermContext<'a> {
    phi: &'a DenseMatrix,
    phi_a: &'a DenseMatrix,
    beta: &'a [f64],
    sigma2_e: f64,
    w_info_inv: &'a DenseMatrix,
    phi_m_phi: &'a [DenseMatrix],
    n_minus_p: f64,
}

struct MultiVcEvalState {
    ll: f64,
    score: Vec<f64>,
    ai: DenseMatrix,
    beta: Vec<f64>,
    phi: DenseMatrix,
    vi: DenseMatrix,
    p_mat: DenseMatrix,
    vir: Vec<f64>,
    quad: f64,
}

fn symmetrize_dense(m: &DenseMatrix) -> Result<DenseMatrix, StatsError> {
    let nr = m.nrows();
    let nc = m.ncols();
    if nr != nc {
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: nr, actual: nc }));
    }
    let mut data = vec![0.0_f64; nr * nr];
    for i in 0..nr {
        data[i * nr + i] = m.get(i, i).unwrap_or(0.0);
        for j in (i + 1)..nr {
            let v = 0.5 * (m.get(i, j).unwrap_or(0.0) + m.get(j, i).unwrap_or(0.0));
            data[i * nr + j] = v;
            data[j * nr + i] = v;
        }
    }
    Ok(DenseMatrix::from_row_major(nr, nr, data)?)
}

fn scale_dense(m: &DenseMatrix, factor: f64) -> Result<DenseMatrix, StatsError> {
    let data: Vec<f64> = m.as_slice().iter().map(|&v| v * factor).collect();
    Ok(DenseMatrix::from_row_major(m.nrows(), m.ncols(), data)?)
}

fn dense_sub(a: &DenseMatrix, b: &DenseMatrix) -> Result<DenseMatrix, StatsError> {
    if a.nrows() != b.nrows() || a.ncols() != b.ncols() {
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: a.nrows(), actual: b.nrows() }));
    }
    let data: Vec<f64> = a.as_slice().iter().zip(b.as_slice().iter()).map(|(&x, &y)| x - y).collect();
    Ok(DenseMatrix::from_row_major(a.nrows(), a.ncols(), data)?)
}

fn trace_product(a: &DenseMatrix, b: &DenseMatrix) -> f64 {
    let n = a.nrows().min(b.ncols());
    let k = a.ncols().min(b.nrows());
    let mut acc = NeumaierSum::new();
    for i in 0..n {
        for j in 0..k {
            acc.add(a.get(i, j).unwrap_or(0.0) * b.get(j, i).unwrap_or(0.0));
        }
    }
    acc.total()
}

fn subblock_dense(m: &DenseMatrix, start: usize, len: usize) -> Result<DenseMatrix, StatsError> {
    let mut data = Vec::with_capacity(len * len);
    for r in 0..len {
        for c in 0..len {
            data.push(m.get(start + r, start + c).unwrap_or(0.0));
        }
    }
    Ok(DenseMatrix::from_row_major(len, len, data)?)
}

fn invert_reml_info(info: &DenseMatrix) -> Result<DenseMatrix, StatsError> {
    let sym = symmetrize_dense(info)?;
    if let Ok(chol) = cholesky(&sym)
        && let Ok(inv) = chol.inverse()
    {
        return Ok(inv);
    }
    // Near-singular or collinear strata: Moore-Penrose pseudoinverse on positive eigenmodes.
    let eig = sym_eigen(&sym)?;
    let r = sym.nrows();
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    let tol = 1e-12 * max_ev.max(1e-30);
    let mut inv_data = vec![0.0_f64; r * r];
    for (k, &ev) in eig.eigenvalues.iter().enumerate() {
        if ev <= tol {
            continue;
        }
        let inv_ev = 1.0 / ev;
        for i in 0..r {
            let qi = eig.eigenvectors.get(i, k).unwrap_or(0.0);
            for j in i..r {
                let qj = eig.eigenvectors.get(j, k).unwrap_or(0.0);
                let delta = qi * qj * inv_ev;
                inv_data[i * r + j] += delta;
                if i != j {
                    inv_data[j * r + i] += delta;
                }
            }
        }
    }
    Ok(DenseMatrix::from_row_major(r, r, inv_data)?)
}

fn ri_precompute_clusters(x: &DenseMatrix, y: &[f64], cluster_ids: &[usize]) -> Vec<RiClusterStats> {
    let p = x.ncols();
    let mut grouped = std::collections::BTreeMap::<usize, Vec<usize>>::new();
    for (row_idx, &cid) in cluster_ids.iter().enumerate() {
        grouped.entry(cid).or_default().push(row_idx);
    }
    let mut clusters = Vec::with_capacity(grouped.len());
    for rows in grouped.into_values() {
        let m_c = rows.len() as f64;
        let mut sum_y = 0.0_f64;
        let mut yy = 0.0_f64;
        let mut sum_x = vec![0.0_f64; p];
        let mut xy = vec![0.0_f64; p];
        let mut xx = vec![0.0_f64; p * p];
        for r in rows {
            let yr = y[r];
            sum_y += yr;
            yy += yr * yr;
            for j in 0..p {
                let xrj = x.get(r, j).unwrap_or(0.0);
                sum_x[j] += xrj;
                xy[j] += xrj * yr;
                for k in j..p {
                    xx[j * p + k] += xrj * x.get(r, k).unwrap_or(0.0);
                }
            }
        }
        for j in 0..p {
            for k in 0..j {
                xx[j * p + k] = xx[k * p + j];
            }
        }
        clusters.push(RiClusterStats { m_c, sum_y, yy, sum_x, xy, xx });
    }
    clusters
}

type RiProfileEval = (f64, f64, DenseMatrix, Vec<f64>);

fn ri_eval_profile(clusters: &[RiClusterStats], p: usize, n_minus_p: f64, tau_raw: f64) -> Result<RiProfileEval, StatsError> {
    let tau = tau_raw.max(0.0);
    let mut xtvx = vec![0.0_f64; p * p];
    let mut xtvy = vec![0.0_f64; p];
    let mut ytvy = 0.0_f64;
    let mut ln_det_v = 0.0_f64;

    for cl in clusters {
        let gamma = tau / (1.0 + cl.m_c * tau);
        ln_det_v += (1.0 + cl.m_c * tau).ln();
        ytvy += cl.yy - gamma * cl.sum_y * cl.sum_y;
        for r_row in 0..p {
            xtvy[r_row] += cl.xy[r_row] - gamma * cl.sum_x[r_row] * cl.sum_y;
            for c_col in r_row..p {
                xtvx[r_row * p + c_col] += cl.xx[r_row * p + c_col] - gamma * cl.sum_x[r_row] * cl.sum_x[c_col];
            }
        }
    }
    for r_row in 0..p {
        for c_col in 0..r_row {
            xtvx[r_row * p + c_col] = xtvx[c_col * p + r_row];
        }
    }
    let xtvx_mat = DenseMatrix::from_row_major(p, p, xtvx)?;
    let chol_x = cholesky(&xtvx_mat)?;
    let ln_det_xtvx = chol_x.ln_det();
    let xtvx_inv = chol_x.inverse()?;
    let beta = chol_x.solve(&xtvy)?;
    let quad_b: f64 = beta.iter().zip(xtvy.iter()).map(|(&b, &rhs)| b * rhs).sum();
    let rss = (ytvy - quad_b).max(1e-30);
    let sigma2_e = rss / n_minus_p;
    let ll = -0.5 * (n_minus_p * sigma2_e.ln() + ln_det_v + ln_det_xtvx + n_minus_p);
    Ok((ll, sigma2_e, xtvx_inv, beta))
}

fn ri_build_fisher_mats(
    clusters: &[RiClusterStats],
    p: usize,
    n_obs: usize,
    tau: f64,
    sigma2_e: f64,
    phi_rel: &DenseMatrix,
) -> Result<OwnedRemlFisherMats, StatsError> {
    let mut m0 = vec![0.0_f64; p * p];
    let mut m1 = vec![0.0_f64; p * p];
    let mut q00 = vec![0.0_f64; p * p];
    let mut q01 = vec![0.0_f64; p * p];
    let mut q11 = vec![0.0_f64; p * p];
    let mut sum_a2 = 0.0_f64;
    let mut sum_ma2 = 0.0_f64;
    let mut sum_m2a2 = 0.0_f64;

    for cl in clusters {
        let a_c = 1.0 / (1.0 + cl.m_c * tau);
        let a2 = a_c * a_c;
        let a3 = a2 * a_c;
        let c_m0 = (1.0 - a2) / cl.m_c;
        let c_q00 = (1.0 - a3) / cl.m_c;
        sum_a2 += a2;
        sum_ma2 += cl.m_c * a2;
        sum_m2a2 += cl.m_c * cl.m_c * a2;
        for r in 0..p {
            for c in r..p {
                let idx = r * p + c;
                let xx_rc = cl.xx[idx];
                let xplus_rc = cl.sum_x[r] * cl.sum_x[c];
                m0[idx] += xx_rc - c_m0 * xplus_rc;
                m1[idx] += a2 * xplus_rc;
                q00[idx] += xx_rc - c_q00 * xplus_rc;
                q01[idx] += a3 * xplus_rc;
                q11[idx] += cl.m_c * a3 * xplus_rc;
            }
        }
    }
    for mat in [&mut m0, &mut m1, &mut q00, &mut q01, &mut q11] {
        for r in 0..p {
            for c in 0..r {
                mat[r * p + c] = mat[c * p + r];
            }
        }
    }
    let m0_mat = DenseMatrix::from_row_major(p, p, m0)?;
    let m1_mat = DenseMatrix::from_row_major(p, p, m1)?;
    let q00_mat = DenseMatrix::from_row_major(p, p, q00)?;
    let q01_mat = DenseMatrix::from_row_major(p, p, q01)?;
    let q11_mat = DenseMatrix::from_row_major(p, p, q11)?;

    let tr_v2 = ((n_obs - clusters.len()) as f64) + sum_a2;
    let m_rel = [&m0_mat, &m1_mat];
    let q_rel = [[&q00_mat, &q01_mat], [&q01_mat, &q11_mat]];
    let tr_vg_vg = [[tr_v2, sum_ma2], [sum_ma2, sum_m2a2]];
    let mut ie_data = [0.0_f64; 4];
    for r in 0..2 {
        let pm_r = phi_rel.matmul(m_rel[r])?;
        for s in r..2 {
            let pm_s = phi_rel.matmul(m_rel[s])?;
            let ie_rs = 0.5 * (tr_vg_vg[r][s] - 2.0 * trace_product(phi_rel, q_rel[r][s]) + trace_product(&pm_r, &pm_s));
            ie_data[r * 2 + s] = ie_rs;
            ie_data[s * 2 + r] = ie_rs;
        }
    }
    let inv_s2 = 1.0 / sigma2_e;
    let inv_s4 = inv_s2 * inv_s2;
    let inv_s6 = inv_s4 * inv_s2;
    let info_e = scale_dense(&DenseMatrix::from_row_major(2, 2, ie_data.to_vec())?, inv_s4)?;
    let m_mats = vec![scale_dense(&m0_mat, inv_s4)?, scale_dense(&m1_mat, inv_s4)?];
    let q01_s = scale_dense(&q01_mat, inv_s6)?;
    let q_mats = vec![vec![scale_dense(&q00_mat, inv_s6)?, q01_s.clone()], vec![q01_s, scale_dense(&q11_mat, inv_s6)?]];
    Ok(OwnedRemlFisherMats { info_e, m_mats, q_mats })
}

fn kr_scale_and_df(qf: f64, a1: f64, a2: f64, nu_sat: f64, n_minus_p: f64) -> (f64, f64) {
    if a2 <= 1e-14 || 1.0 - a2 / qf <= 1e-6 {
        return (1.0, nu_sat);
    }
    let b_kr = (a1 + 6.0 * a2) / (2.0 * qf);
    let g_kr = ((qf + 1.0) * a1 - (qf + 4.0) * a2) / ((qf + 2.0) * a2);
    let den_c = 3.0 * qf + 2.0 * (1.0 - g_kr);
    if den_c.abs() <= 1e-14 {
        return (1.0, nu_sat);
    }
    let c1 = g_kr / den_c;
    let c2 = (qf - g_kr) / den_c;
    let c3 = (qf + 2.0 - g_kr) / den_c;
    let e_star = 1.0 / (1.0 - a2 / qf);
    let v_num = 1.0 + c1 * b_kr;
    let v_den = (1.0 - c2 * b_kr).powi(2) * (1.0 - c3 * b_kr);
    if v_num <= 0.0 || v_den <= 1e-6 {
        return (1.0, nu_sat);
    }
    let v_star = (2.0 / qf) * (v_num / v_den);
    let rho = v_star / (2.0 * e_star * e_star);
    if qf * rho - 1.0 <= 1e-12 {
        return (1.0 / e_star, n_minus_p);
    }
    let nu_kr = (4.0 + (qf + 2.0) / (qf * rho - 1.0)).clamp(1.0, n_minus_p);
    let lambda = if nu_kr > 2.0 { nu_kr / (e_star * (nu_kr - 2.0)) } else { 1.0 / e_star };
    (lambda.max(0.0), nu_kr)
}

fn kr_single_term_df1(ctx: &KrTermContext<'_>, term_name: &str, col: usize) -> Result<(FactorialAnovaTerm, f64, f64), StatsError> {
    let v = ctx.phi.get(col, col).unwrap_or(0.0).max(0.0);
    let v_a_raw = ctx.phi_a.get(col, col).unwrap_or(v);
    let v_a = if v_a_raw > 1e-12 * v { v_a_raw } else { v };
    let b = ctx.beta[col];
    let f_kr = if v_a > 0.0 {
        (b * b) / v_a
    } else if b != 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let mut grad = Vec::with_capacity(ctx.phi_m_phi.len());
    for pmp in ctx.phi_m_phi {
        grad.push(pmp.get(col, col).unwrap_or(0.0));
    }
    let wg = ctx.w_info_inv.matvec(&grad)?;
    let denom: f64 = grad.iter().zip(wg.iter()).map(|(&a, &c)| a * c).sum();
    let nu = if denom > 1e-30 * v * v { (2.0 * v * v / denom).clamp(1.0, ctx.n_minus_p) } else { ctx.n_minus_p };
    let p_val = f_sf(f_kr, 1.0, nu)?;
    let ms = f_kr * ctx.sigma2_e;
    let part_eta = if f_kr + nu > 0.0 { Some(f_kr / (f_kr + nu)) } else { Some(0.0) };
    let term = FactorialAnovaTerm {
        name: term_name.to_owned(),
        ss: ms,
        df: 1.0,
        ms,
        f_statistic: Some(f_kr),
        p_value: Some(p_val),
        partial_eta_squared: part_eta,
        eta_squared: None,
    };
    Ok((term, nu, nu))
}

fn kr_multi_term_df(ctx: &KrTermContext<'_>, term_name: &str, start: usize, q: usize) -> Result<(FactorialAnovaTerm, f64, f64), StatsError> {
    let qf = q as f64;
    let sigma = symmetrize_dense(&subblock_dense(ctx.phi, start, q)?)?;
    let sigma_a = symmetrize_dense(&subblock_dense(ctx.phi_a, start, q)?)?;
    let beta_sub = &ctx.beta[start..(start + q)];
    let inv_a = match cholesky(&sigma_a).and_then(|c| c.inverse()) {
        Ok(inv) => inv,
        Err(_) => invert_reml_info(&sigma_a)?,
    };
    let inv_b = inv_a.matvec(beta_sub)?;
    let quad: f64 = beta_sub.iter().zip(inv_b.iter()).map(|(&b, &ib)| b * ib).sum();
    let f_raw = (quad / qf).max(0.0);

    // Fai & Cornelius (1996) Satterthwaite df via spectral decomposition of L Phi L^T.
    let eig = sym_eigen(&sigma)?;
    let n_vc = ctx.w_info_inv.nrows();
    let mut nu_modes = Vec::with_capacity(q);
    for m in 0..q {
        let v_m = eig.eigenvalues[m];
        if v_m <= 1e-30 {
            nu_modes.push(ctx.n_minus_p);
            continue;
        }
        let mut u_m = Vec::with_capacity(q);
        for r in 0..q {
            u_m.push(eig.eigenvectors.get(r, m).unwrap_or(0.0));
        }
        let mut grad_m = Vec::with_capacity(n_vc);
        for pmp in ctx.phi_m_phi {
            let sub = subblock_dense(pmp, start, q)?;
            let sv = sub.matvec(&u_m)?;
            grad_m.push(u_m.iter().zip(sv.iter()).map(|(&a, &b)| a * b).sum());
        }
        let wg = ctx.w_info_inv.matvec(&grad_m)?;
        let denom_m: f64 = grad_m.iter().zip(wg.iter()).map(|(&a, &b)| a * b).sum();
        let nu_m = if denom_m > 1e-30 * v_m * v_m { 2.0 * v_m * v_m / denom_m } else { ctx.n_minus_p };
        nu_modes.push(nu_m);
    }
    let nu_sat = if nu_modes.iter().all(|&nm| nm > 2.0) {
        let e_fc: f64 = nu_modes.iter().map(|&nm| nm / (nm - 2.0)).sum();
        if e_fc > qf + 1e-12 { (2.0 * e_fc / (e_fc - qf)).clamp(1.0, ctx.n_minus_p) } else { ctx.n_minus_p }
    } else {
        nu_modes.iter().copied().fold(ctx.n_minus_p, f64::min).clamp(1.0, ctx.n_minus_p)
    };

    // Kenward & Roger (1997, Biometrics 53:985 Eq. (6)) scaled F* and denominator df.
    let sigma_inv = invert_reml_info(&sigma)?;
    let mut t_mats = Vec::with_capacity(n_vc);
    let mut tr_t = Vec::with_capacity(n_vc);
    for pmp in ctx.phi_m_phi {
        let d_sub = subblock_dense(pmp, start, q)?;
        let ti = sigma_inv.matmul(&d_sub)?;
        let mut tr = 0.0_f64;
        for k in 0..q {
            tr += ti.get(k, k).unwrap_or(0.0);
        }
        tr_t.push(tr);
        t_mats.push(ti);
    }
    let mut a1 = 0.0_f64;
    let mut a2 = 0.0_f64;
    for i in 0..n_vc {
        for j in 0..n_vc {
            let wij = ctx.w_info_inv.get(i, j).unwrap_or(0.0);
            a1 += wij * tr_t[i] * tr_t[j];
            a2 += wij * trace_product(&t_mats[i], &t_mats[j]);
        }
    }
    let (lambda_kr, nu_kr) = kr_scale_and_df(qf, a1, a2, nu_sat, ctx.n_minus_p);
    let f_kr = (lambda_kr * f_raw).max(0.0);
    let p_val = f_sf(f_kr, qf, nu_kr)?;
    let ms = f_kr * ctx.sigma2_e;
    let part_eta = if qf * f_kr + nu_kr > 0.0 { Some((qf * f_kr) / (qf * f_kr + nu_kr)) } else { Some(0.0) };
    let term = FactorialAnovaTerm {
        name: term_name.to_owned(),
        ss: ms * qf,
        df: qf,
        ms,
        f_statistic: Some(f_kr),
        p_value: Some(p_val),
        partial_eta_squared: part_eta,
        eta_squared: None,
    };
    Ok((term, nu_sat, nu_kr))
}

type RemlWaldTermsOut = (Vec<FactorialAnovaTerm>, Vec<f64>, Vec<f64>);

fn eval_reml_wald_terms_from_mats(
    phi: &DenseMatrix,
    beta: &[f64],
    sigma2_e: f64,
    fisher: &OwnedRemlFisherMats,
    terms: &[(&str, usize, usize)],
    n_minus_p: f64,
) -> Result<RemlWaldTermsOut, StatsError> {
    let w_info_inv = invert_reml_info(&fisher.info_e)?;
    let n_vc = fisher.m_mats.len();
    let p = phi.nrows();
    let mut phi_m_phi = Vec::with_capacity(n_vc);
    for m_mat in &fisher.m_mats {
        phi_m_phi.push(symmetrize_dense(&phi.matmul(m_mat)?.matmul(phi)?)?);
    }
    let mut lambda_data = vec![0.0_f64; p * p];
    for r in 0..n_vc {
        for s in 0..n_vc {
            let wrs = w_info_inv.get(r, s).unwrap_or(0.0);
            if wrs == 0.0 {
                continue;
            }
            let m_phi_m = fisher.m_mats[r].matmul(phi)?.matmul(&fisher.m_mats[s])?;
            let diff = dense_sub(&fisher.q_mats[r][s], &m_phi_m)?;
            let contrib = phi.matmul(&diff)?.matmul(phi)?;
            for (dst, &val) in lambda_data.iter_mut().zip(contrib.as_slice().iter()) {
                *dst += wrs * val;
            }
        }
    }
    let phi_a_raw: Vec<f64> = phi.as_slice().iter().zip(lambda_data.iter()).map(|(&a, &l)| a + 2.0 * l).collect();
    let phi_a_sym = symmetrize_dense(&DenseMatrix::from_row_major(p, p, phi_a_raw)?)?;
    let phi_a = if cholesky(&phi_a_sym).is_ok() { phi_a_sym } else { phi.clone() };
    let ctx = KrTermContext { phi, phi_a: &phi_a, beta, sigma2_e, w_info_inv: &w_info_inv, phi_m_phi: &phi_m_phi, n_minus_p };

    let mut fixed_terms = Vec::with_capacity(terms.len());
    let mut satt_dfs = Vec::with_capacity(terms.len());
    let mut kr_dfs = Vec::with_capacity(terms.len());
    for &(name, start, len) in terms {
        if len == 0 || start + len > p {
            continue;
        }
        let (term, nu_sat, nu_kr) = if len == 1 { kr_single_term_df1(&ctx, name, start)? } else { kr_multi_term_df(&ctx, name, start, len)? };
        fixed_terms.push(term);
        satt_dfs.push(nu_sat);
        kr_dfs.push(nu_kr);
    }
    Ok((fixed_terms, satt_dfs, kr_dfs))
}

/// Fit a linear mixed-effects random-intercept model via Restricted Maximum Likelihood (REML)
/// (Patterson & Thompson, 1971) with genuine Fai & Cornelius (1996) / Satterthwaite (1946)
/// and Kenward & Roger (1997) denominator degrees of freedom, adjusted covariance, and scaled *F*.
pub fn fit_random_intercept_reml(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    terms: &[(&str, usize, usize)],
) -> Result<MixedRemlResult, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n {
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
    }
    if cluster_ids.len() != n {
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n, actual: cluster_ids.len() }));
    }
    if n <= p {
        return Err(StatsError::InsufficientSampleSize { context: "REML mixed model", required: p + 1, actual: n });
    }
    validate_finite_slice(y)?;
    let n_minus_p = (n - p) as f64;
    let y_moments = SummaryMoments::from_slice(y)?;
    let y_scale_raw = (y_moments.css / (n as f64)).sqrt();
    let y_scale = if y_scale_raw.is_finite() && y_scale_raw > 0.0 { y_scale_raw } else { 1.0 };
    let y_scaled: Vec<f64> = y.iter().map(|&v| v / y_scale).collect();

    let clusters = ri_precompute_clusters(x, &y_scaled, cluster_ids);
    let eval_u = |u: f64| -> Result<(f64, RiProfileEval), StatsError> {
        let tau = u.exp_m1().max(0.0);
        let ev = ri_eval_profile(&clusters, p, n_minus_p, tau)?;
        Ok((tau, ev))
    };

    let (_, zero_eval) = eval_u(0.0)?;
    let ll_zero = zero_eval.0;

    // Coarse search on u = ln(1 + tau) in [0, 12] + fine search in [0, 0.25].
    let mut best_u = 0.0_f64;
    let mut best_ll = ll_zero;
    let mut best_step = 12.0 / 80.0;
    for i in 1..=80 {
        let u = (i as f64) * (12.0 / 80.0);
        if let Ok((_, (ll, _, _, _))) = eval_u(u)
            && ll > best_ll
        {
            best_ll = ll;
            best_u = u;
        }
    }
    for i in 1..=25 {
        let u = (i as f64) * (0.25 / 25.0);
        if let Ok((_, (ll, _, _, _))) = eval_u(u)
            && ll > best_ll
        {
            best_ll = ll;
            best_u = u;
            best_step = 0.25 / 25.0;
        }
    }

    let mut a_u = (best_u - best_step).max(0.0);
    let mut b_u = (best_u + best_step).min(12.0);
    let phi_gold = 0.5 * (5.0_f64.sqrt() - 1.0);
    let mut c_u = b_u - phi_gold * (b_u - a_u);
    let mut d_u = a_u + phi_gold * (b_u - a_u);
    let mut f_c = eval_u(c_u).map_or(f64::NEG_INFINITY, |(_, ev)| ev.0);
    let mut f_d = eval_u(d_u).map_or(f64::NEG_INFINITY, |(_, ev)| ev.0);
    for _ in 0..65 {
        if f_c < f_d {
            a_u = c_u;
            c_u = d_u;
            f_c = f_d;
            d_u = a_u + phi_gold * (b_u - a_u);
            f_d = eval_u(d_u).map_or(f64::NEG_INFINITY, |(_, ev)| ev.0);
        } else {
            b_u = d_u;
            d_u = c_u;
            f_d = f_c;
            c_u = b_u - phi_gold * (b_u - a_u);
            f_c = eval_u(c_u).map_or(f64::NEG_INFINITY, |(_, ev)| ev.0);
        }
    }
    let cand_u = 0.5 * (a_u + b_u);
    let (cand_tau, cand_eval) = eval_u(cand_u)?;
    let (tau_hat, (opt_ll, sigma2_e_scaled, xtvx_rel_inv, beta_scaled)) = if cand_eval.0 > best_ll && cand_eval.0 > ll_zero + 1e-12 {
        (cand_tau, cand_eval)
    } else if best_ll > ll_zero + 1e-12 {
        eval_u(best_u)?
    } else {
        (0.0, zero_eval)
    };

    let lrt_chi2 = (2.0 * (opt_ll - ll_zero)).max(0.0);
    let lrt_p = if tau_hat > 0.0 && lrt_chi2 > 0.0 { 0.5 * chi2_sf(lrt_chi2, 1.0)? } else { 1.0 };

    let sigma2_e = sigma2_e_scaled * y_scale * y_scale;
    let sigma2_u = tau_hat * sigma2_e;
    let icc = if sigma2_u + sigma2_e > 0.0 { sigma2_u / (sigma2_u + sigma2_e) } else { 0.0 };
    let reml_log_lik = opt_ll - n_minus_p * y_scale.ln();
    let beta: Vec<f64> = beta_scaled.into_iter().map(|b| b * y_scale).collect();
    let phi = scale_dense(&xtvx_rel_inv, sigma2_e)?;

    let fisher = ri_build_fisher_mats(&clusters, p, n, tau_hat, sigma2_e, &xtvx_rel_inv)?;
    let (fixed_terms, satterthwaite_df, kenward_roger_df) = eval_reml_wald_terms_from_mats(&phi, &beta, sigma2_e, &fisher, terms, n_minus_p)?;

    Ok(MixedRemlResult {
        sigma2_random: sigma2_u,
        sigma2_error: sigma2_e,
        icc,
        reml_log_lik,
        lrt_random_chi2: lrt_chi2,
        lrt_random_p: lrt_p,
        fixed_terms,
        satterthwaite_df,
        kenward_roger_df,
    })
}

fn cluster_ids_to_g_matrix(cluster_ids: &[usize]) -> Result<(DenseMatrix, usize), StatsError> {
    let n = cluster_ids.len();
    let mut remap = std::collections::BTreeMap::<usize, usize>::new();
    let mut mapped = Vec::with_capacity(n);
    for &cid in cluster_ids {
        let next = remap.len();
        let idx = *remap.entry(cid).or_insert(next);
        mapped.push(idx);
    }
    let c_count = remap.len();
    let mut g = vec![0.0_f64; n * n];
    for i in 0..n {
        g[i * n + i] = 1.0;
        for j in (i + 1)..n {
            if mapped[i] == mapped[j] {
                g[i * n + j] = 1.0;
                g[j * n + i] = 1.0;
            }
        }
    }
    Ok((DenseMatrix::from_row_major(n, n, g)?, c_count))
}

fn multi_vc_eval_state(x: &DenseMatrix, y: &[f64], gs: &[DenseMatrix], theta: &[f64]) -> Result<MultiVcEvalState, StatsError> {
    let n = x.nrows();
    let n_vc = gs.len();
    let mut v_data = vec![0.0_f64; n * n];
    for (&th, g) in theta.iter().zip(gs.iter()) {
        for (dst, &gv) in v_data.iter_mut().zip(g.as_slice().iter()) {
            *dst += th * gv;
        }
    }
    let v_mat = symmetrize_dense(&DenseMatrix::from_row_major(n, n, v_data)?)?;
    let chol_v = cholesky(&v_mat)?;
    let ld_v = chol_v.ln_det();
    let vi = chol_v.inverse()?;

    let xt_vi = x.transpose().matmul(&vi)?;
    let xtvix = symmetrize_dense(&xt_vi.matmul(x)?)?;
    let chol_m = cholesky(&xtvix)?;
    let ld_m = chol_m.ln_det();
    let phi = chol_m.inverse()?;
    let xtvi_y = xt_vi.matvec(y)?;
    let beta = chol_m.solve(&xtvi_y)?;
    let x_beta = x.matvec(&beta)?;
    let resid: Vec<f64> = y.iter().zip(x_beta.iter()).map(|(&yv, &xb)| yv - xb).collect();
    let vir = vi.matvec(&resid)?;
    let quad: f64 = resid.iter().zip(vir.iter()).map(|(&r, &vr)| r * vr).sum();
    let ll = -0.5 * (ld_v + ld_m + quad);

    let proj = xt_vi.transpose().matmul(&phi)?.matmul(&xt_vi)?;
    let p_mat = symmetrize_dense(&dense_sub(&vi, &proj)?)?;

    let mut g_vir = Vec::with_capacity(n_vc);
    let mut score = Vec::with_capacity(n_vc);
    for g in gs {
        let gv = g.matvec(&vir)?;
        let tr_pg = trace_product(&p_mat, g);
        let q_g: f64 = vir.iter().zip(gv.iter()).map(|(&a, &b)| a * b).sum();
        score.push(-0.5 * (tr_pg - q_g));
        g_vir.push(gv);
    }
    let mut ai_data = vec![0.0_f64; n_vc * n_vc];
    for i in 0..n_vc {
        let p_g_vir_i = p_mat.matvec(&g_vir[i])?;
        for j in i..n_vc {
            let val: f64 = g_vir[j].iter().zip(p_g_vir_i.iter()).map(|(&a, &b)| a * b).sum();
            let ai_ij = 0.5 * val;
            ai_data[i * n_vc + j] = ai_ij;
            ai_data[j * n_vc + i] = ai_ij;
        }
    }
    let ai = DenseMatrix::from_row_major(n_vc, n_vc, ai_data)?;
    Ok(MultiVcEvalState { ll, score, ai, beta, phi, vi, p_mat, vir, quad })
}

fn multi_vc_em_step(st: &MultiVcEvalState, gs: &[DenseMatrix], dofs: &[f64], theta: &[f64], floor_positive: bool) -> Vec<f64> {
    let mut next = Vec::with_capacity(theta.len());
    for k in 0..theta.len() {
        let th = theta[k];
        let g_vir = gs[k].matvec(&st.vir).unwrap_or_default();
        let uhat_sq = th * th * st.vir.iter().zip(g_vir.iter()).map(|(&a, &b)| a * b).sum::<f64>();
        let cov_trace = dofs[k] * th - th * th * trace_product(&st.p_mat, &gs[k]);
        let floor = if k == 0 || floor_positive { 1e-12 } else { 0.0 };
        next.push(((uhat_sq + cov_trace) / dofs[k]).max(floor));
    }
    next
}

fn multi_vc_ai_step(
    x: &DenseMatrix,
    y: &[f64],
    gs: &[DenseMatrix],
    dofs: &[f64],
    theta: &mut [f64],
    n_minus_p: f64,
) -> Result<(bool, bool), StatsError> {
    let st = multi_vc_eval_state(x, y, gs, theta)?;
    let n_vc = theta.len();
    let free: Vec<usize> = (0..n_vc).filter(|&k| theta[k] > 1e-11 || st.score[k] > 1e-9).collect();
    if free.is_empty() {
        return Ok((true, true));
    }
    let max_active_grad = free.iter().map(|&k| st.score[k].abs()).fold(0.0_f64, f64::max);
    if max_active_grad < 1e-9 {
        return Ok((true, true));
    }

    let nf = free.len();
    let g_f: Vec<f64> = free.iter().map(|&k| st.score[k]).collect();
    let mut ai_f_data = Vec::with_capacity(nf * nf);
    for &i in &free {
        for &j in &free {
            ai_f_data.push(st.ai.get(i, j).unwrap_or(0.0));
        }
    }
    let ai_f = symmetrize_dense(&DenseMatrix::from_row_major(nf, nf, ai_f_data)?)?;
    let mut step_f = if let Ok(chol) = cholesky(&ai_f)
        && let Ok(sol) = chol.solve(&g_f)
    {
        sol
    } else {
        let eig = sym_eigen(&ai_f)?;
        let max_ev = eig.eigenvalues.iter().map(|&v| v.abs()).fold(0.0_f64, f64::max).max(1e-12);
        let qtg = eig.eigenvectors.t_matvec(&g_f)?;
        let scaled: Vec<f64> = qtg.iter().zip(eig.eigenvalues.iter()).map(|(&c, &ev)| c / ev.max(1e-8 * max_ev)).collect();
        eig.eigenvectors.matvec(&scaled)?
    };
    let directional: f64 = step_f.iter().zip(g_f.iter()).map(|(&d, &g)| d * g).sum();
    if directional <= 0.0 || !directional.is_finite() {
        step_f = g_f;
    }

    let mut step = vec![0.0_f64; n_vc];
    for (&k, &df) in free.iter().zip(step_f.iter()) {
        step[k] = df;
    }

    let mut alpha = 1.0_f64;
    for _ in 0..25 {
        let mut cand: Vec<f64> = theta.iter().zip(step.iter()).map(|(&t, &d)| (t + alpha * d).max(0.0)).collect();
        cand[0] = cand[0].max(1e-12);
        if let Ok(st_c) = multi_vc_eval_state(x, y, gs, &cand)
            && st_c.ll >= st.ll - 1e-13
        {
            let c_scale = st_c.quad / n_minus_p;
            let mut chosen = cand.clone();
            let mut chosen_ll = st_c.ll;
            if c_scale.is_finite() && c_scale > 1e-12 {
                let mut cand_r: Vec<f64> = cand.iter().map(|&v| v * c_scale).collect();
                cand_r[0] = cand_r[0].max(1e-12);
                if let Ok(st_r) = multi_vc_eval_state(x, y, gs, &cand_r)
                    && st_r.ll > chosen_ll
                {
                    chosen = cand_r;
                    chosen_ll = st_r.ll;
                }
            }
            let max_delta = theta.iter().zip(chosen.iter()).map(|(&a, &b)| (a - b).abs()).fold(0.0_f64, f64::max);
            let improved = chosen_ll > st.ll + 1e-13;
            theta.copy_from_slice(&chosen);
            return Ok((max_delta < 1e-11 && !improved, true));
        }
        alpha *= 0.5;
    }

    // Backtracking failed: take one guaranteed-monotone EM step.
    let em_next = multi_vc_em_step(&st, gs, dofs, theta, false);
    let max_delta = theta.iter().zip(em_next.iter()).map(|(&a, &b)| (a - b).abs()).fold(0.0_f64, f64::max);
    theta.copy_from_slice(&em_next);
    Ok((max_delta < 1e-11, false))
}

/// Fit a multi-variance-component linear mixed model `V = sigma_e^2 I + sum_k sigma_k^2 G_k`
/// via EM warm-start followed by projected Average-Information REML (Gilmour, Thompson & Cullis,
/// 1995), with genuine Fai & Cornelius (1996) Satterthwaite and Kenward & Roger (1997) fixed-effect
/// inference across all `(r + 1)` variance components.
pub fn fit_multi_vc_reml(
    x: &DenseMatrix,
    y: &[f64],
    random_effects: &[(&str, DenseMatrix, usize)],
    terms: &[(&str, usize, usize)],
) -> Result<MultiVcRemlResult, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n {
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
    }
    if n <= p {
        return Err(StatsError::InsufficientSampleSize { context: "multi-VC REML model", required: p + 1, actual: n });
    }
    validate_finite_slice(y)?;
    let n_minus_p = (n - p) as f64;
    let y_moments = SummaryMoments::from_slice(y)?;
    let y_scale_raw = (y_moments.css / (n as f64)).sqrt();
    let y_scale = if y_scale_raw.is_finite() && y_scale_raw > 0.0 { y_scale_raw } else { 1.0 };
    let y_scaled: Vec<f64> = y.iter().map(|&v| v / y_scale).collect();

    let n_vc = 1 + random_effects.len();
    let mut gs = Vec::with_capacity(n_vc);
    let mut dofs = Vec::with_capacity(n_vc);
    let mut names = Vec::with_capacity(random_effects.len());
    gs.push(DenseMatrix::identity(n)?);
    dofs.push(n as f64);
    for (name, g_mat, rank_c) in random_effects {
        if g_mat.nrows() != n || g_mat.ncols() != n {
            return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n, actual: g_mat.nrows().min(g_mat.ncols()) }));
        }
        gs.push(g_mat.clone());
        dofs.push((*rank_c).max(1) as f64);
        names.push((*name).to_owned());
    }

    let var0 = solve_linear_least_squares(x, &y_scaled).map_or(1.0, |(sol, _)| (sol.rss / n_minus_p).max(1e-6));
    let mut theta = vec![(var0 / (n_vc as f64)).max(1e-8); n_vc];
    for _ in 0..25 {
        if let Ok(st) = multi_vc_eval_state(x, &y_scaled, &gs, &theta) {
            theta = multi_vc_em_step(&st, &gs, &dofs, &theta, true);
        } else {
            break;
        }
    }
    for _ in 0..80 {
        let (converged, _) = multi_vc_ai_step(x, &y_scaled, &gs, &dofs, &mut theta, n_minus_p)?;
        if converged {
            break;
        }
    }

    let st = multi_vc_eval_state(x, &y_scaled, &gs, &theta)?;
    let mut ie_data = vec![0.0_f64; n_vc * n_vc];
    let mut m_mats = Vec::with_capacity(n_vc);
    let xt_vi = x.transpose().matmul(&st.vi)?;
    let vi_x = st.vi.matmul(x)?;
    for g in &gs {
        m_mats.push(symmetrize_dense(&xt_vi.matmul(g)?.matmul(&vi_x)?)?);
    }
    let mut q_mats = vec![Vec::with_capacity(n_vc); n_vc];
    let mut pg_mats = Vec::with_capacity(n_vc);
    for (r, gr) in gs.iter().enumerate() {
        pg_mats.push(st.p_mat.matmul(gr)?);
        let xt_vi_gr = xt_vi.matmul(gr)?;
        for gs_mat in &gs {
            q_mats[r].push(xt_vi_gr.matmul(&st.vi)?.matmul(gs_mat)?.matmul(&vi_x)?);
        }
    }
    for r in 0..n_vc {
        for s in r..n_vc {
            let ie_rs = 0.5 * trace_product(&pg_mats[r], &pg_mats[s]);
            ie_data[r * n_vc + s] = ie_rs;
            ie_data[s * n_vc + r] = ie_rs;
        }
    }
    let fisher = OwnedRemlFisherMats { info_e: DenseMatrix::from_row_major(n_vc, n_vc, ie_data)?, m_mats, q_mats };
    let sigma2_error = theta[0] * y_scale * y_scale;
    let (mut fixed_terms, satterthwaite_df, kenward_roger_df) =
        eval_reml_wald_terms_from_mats(&st.phi, &st.beta, theta[0], &fisher, terms, n_minus_p)?;
    for term in &mut fixed_terms {
        if let Some(f_val) = term.f_statistic {
            let ms = f_val * sigma2_error;
            term.ms = ms;
            term.ss = ms * term.df;
        }
    }
    let variance_components = names.into_iter().zip(theta.iter().skip(1).map(|&v| v * y_scale * y_scale)).collect();
    let reml_log_lik = st.ll - n_minus_p * y_scale.ln();

    Ok(MultiVcRemlResult { variance_components, sigma2_error, reml_log_lik, fixed_terms, satterthwaite_df, kenward_roger_df })
}

// ============================================================================
// Nested One-Way ANOVA & Nested t-Test
// ============================================================================

/// Fit nested hierarchical one-way ANOVA (Groups -> Biological-Unit Subclusters -> Technical
/// Replicates), computing exact Satterthwaite / Fai & Cornelius (1996) and Kenward & Roger (1997)
/// denominator degrees of freedom for unequal cluster sizes and variance components
/// `(sigma_subcluster^2, sigma_error^2)`.
///
/// The two-level nested partition (groups, subclusters within groups, error) and the subcluster
/// *F*-test follow Zar (2010, ch. 15). The among-groups test is the REML small-sample
/// Kenward–Roger Wald *F*-test from [`fit_random_intercept_reml`] (Patterson & Thompson, 1971;
/// Kenward & Roger, 1997). For two groups, the nested `t` is the signed square root of that
/// adjusted Wald *F*.
pub fn nested_one_way_anova(table: &lumencraft_doc::NestedTable, two_group_t_test: bool) -> Result<NestedAnovaResult, StatsError> {
    let selected: Vec<usize> = (0..table.groups.len()).collect();
    nested_one_way_anova_selected(table, &selected, two_group_t_test)
}

fn nested_one_way_anova_selected(
    table: &lumencraft_doc::NestedTable,
    selected_groups: &[usize],
    two_group_t_test: bool,
) -> Result<NestedAnovaResult, StatsError> {
    let g_count = selected_groups.len();
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

    for (local_g_idx, &orig_g_idx) in selected_groups.iter().enumerate() {
        let group = table.groups.get(orig_g_idx).ok_or(StatsError::MissingColumn { index: orig_g_idx })?;
        group_labels.push(group.title.clone());
        let mut g_acc = NeumaierSum::new();
        let mut g_n = 0_usize;
        for b_idx in 0..group.biological_unit_count() {
            let reps = table.active_technical_replicates(orig_g_idx, b_idx);
            if !reps.is_empty() {
                let s: f64 = reps.iter().sum();
                g_acc.add(s);
                grand_sum.add(s);
                g_n += reps.len();
                total_obs += reps.len();
                subcluster_data.push((local_g_idx, reps));
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

    for (cid, (g_idx, reps)) in subcluster_data.iter().enumerate() {
        let g_mean = group_sums[*g_idx] / (group_n[*g_idx] as f64);
        let m = SummaryMoments::from_slice(reps)?;
        let n_ij = reps.len() as f64;
        let d_sub = m.mean - g_mean;
        ss_sub.add(n_ij * d_sub * d_sub);
        ss_err.add(m.css);

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

    let (f_sub, p_sub) = f_ratio_p(mss, mse, df1_sub, df2_err)?;

    let x_mat = DenseMatrix::from_row_major(total_obs, g_count, x_data)?;
    let reml = fit_random_intercept_reml(&x_mat, &y_vec, &cluster_ids, &[("Group", 1, g_count - 1)])?;

    let f_g = reml.fixed_terms.first().and_then(|t| t.f_statistic).unwrap_or(if mss > 0.0 {
        msg / mss
    } else if msg > 0.0 {
        f64::INFINITY
    } else {
        0.0
    });
    let df2_satt = reml.satterthwaite_df.first().copied().unwrap_or(df1_sub);
    let df2_kr = reml.kenward_roger_df.first().copied().unwrap_or(df1_sub);
    let p_g = reml.fixed_terms.first().and_then(|t| t.p_value).unwrap_or(f_sf(f_g, df1_g, df2_kr)?);

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
    let (group_inputs, raw_res_opt, raw_slices_owned, ignored_non_numeric) = extract_one_way_inputs(sheet, &spec.selected_data_sets)?;

    match anova_spec.method {
        OneWayMethod::OrdinaryAnova => {
            let res = if let Some(r) = raw_res_opt { r } else { ordinary_one_way_from_summary(&group_inputs, confidence)? };
            push_scalar(&mut out, "F", None, res.f_statistic);
            push_scalar(&mut out, "P value", None, res.p_value);
            push_scalar(&mut out, "R squared", None, res.r_squared);
            push_scalar(&mut out, "MS residual", None, res.ms_within);
            push_scalar(&mut out, "Eta squared", None, res.effect_sizes.eta_squared);
            push_scalar(&mut out, "Omega squared", None, res.effect_sizes.omega_squared);

            let mut main_tab = build_one_way_tab(&res, anova_spec.compute_effect_sizes, confidence, p_fmt);
            if !res.variance_tests.available {
                main_tab.footnotes.push(String::from(
                    "Brown-Forsythe and Bartlett variance homogeneity tests are not computable from pre-summarized Mean/SD/N data.",
                ));
            }
            out.tabs.push(main_tab);
            out.tabs.push(build_variance_homogeneity_tab(&res.variance_tests, p_fmt));
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
            let mut uv_tab = build_tab("unequal-var-anova", "Brown-Forsythe and Welch ANOVA", ResultTabKind::AnovaTable, headers, labels, rows);
            if !base.variance_tests.available {
                uv_tab.footnotes.push(String::from(
                    "Brown-Forsythe and Bartlett variance homogeneity tests are not computable from pre-summarized Mean/SD/N data.",
                ));
            }
            out.tabs.push(uv_tab);
            out.tabs.push(build_variance_homogeneity_tab(&base.variance_tests, p_fmt));

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
            let (dunn_title, dunn_footnote) = match MultiplicityProcedure::from(anova_spec.multiplicity.method) {
                MultiplicityProcedure::TukeyKramer
                | MultiplicityProcedure::NewmanKeuls
                | MultiplicityProcedure::GamesHowell
                | MultiplicityProcedure::DunnettT3 => (
                    "Dunn's multiple comparisons (Bonferroni fallback)",
                    Some("Parametric FWER post-hoc method mapped to Dunn's rank-sum comparisons with Bonferroni adjustment for Kruskal-Wallis."),
                ),
                MultiplicityProcedure::TamhaneT2 => (
                    "Dunn's multiple comparisons (Sidak fallback)",
                    Some("Tamhane's T2 mapped to Dunn's rank-sum comparisons with Sidak adjustment for Kruskal-Wallis."),
                ),
                MultiplicityProcedure::Dunnett { .. } => (
                    "Dunn's vs-control comparisons (Bonferroni fallback)",
                    Some(
                        "Dunnett vs-control selection mapped to Dunn's rank-sum comparisons vs control with Bonferroni adjustment over k - 1 pairs.",
                    ),
                ),
                MultiplicityProcedure::None | MultiplicityProcedure::FisherLsd => (
                    "Dunn's multiple comparisons (uncorrected)",
                    Some("Unadjusted Dunn's rank-sum pairwise z tests (no multiplicity correction applied)."),
                ),
                _ => ("Dunn's multiple comparisons", None),
            };
            push_pairwise_outputs(&mut out, &details, dunn_title, p_fmt, confidence);
            if let Some(note) = dunn_footnote
                && let Some(tab) = out.tabs.last_mut()
            {
                tab.footnotes.push(String::from(note));
            }
            out.checklist.push(checklist_item(
                "Rank-based omnibus test",
                ChecklistStatus::Pass,
                Some(format!("H = {:.4}, N = {}", kw.h_statistic, kw.total_n)),
                "Kruskal-Wallis compares group rank distributions without assuming Gaussian residuals.",
            ));
        }
        OneWayMethod::RepeatedMeasures { .. } | OneWayMethod::Friedman => {}
    }

    append_ignored_text_checklist_and_footnote(&mut out, "one-way ANOVA", &ignored_non_numeric);
    sanitize_result_sheet_floats(&mut out)?;
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
    let report = col_tab.complete_paired_rows_with_report(&indices);
    if report.blocks.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let blocks = &report.blocks;

    let rm_analysis_name = match anova_spec.method {
        OneWayMethod::RepeatedMeasures { .. } => "Repeated-measures one-way ANOVA",
        OneWayMethod::Friedman => "Friedman test",
        _ => "Repeated-measures analysis",
    };

    match anova_spec.method {
        OneWayMethod::RepeatedMeasures { geisser_greenhouse } => {
            let rm = rm_one_way_anova(blocks)?;
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
            let fr = friedman_test(blocks)?;
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
    append_listwise_filtered_checklist_and_footnote(out, rm_analysis_name, report.skipped_incomplete_rows, report.blocks.len());
    if report.non_numeric_cells > 0 {
        append_ignored_text_checklist_and_footnote(out, rm_analysis_name, &[(String::from("Selected columns"), report.non_numeric_cells)]);
    }
    sanitize_result_sheet_floats(out)?;
    Ok(out.clone())
}

fn spec_selected_from_sheet(sheet: &ResultSheet) -> Vec<usize> {
    sheet.analysis.selected_data_sets.clone()
}

type ExtractedOneWayInputs = (Vec<GroupComparisonInput>, Option<OneWayAnovaResult>, Vec<Vec<f64>>, Vec<(String, usize)>);

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
        return Ok((inputs, None, Vec::new(), Vec::new()));
    }

    let cols = extract_numeric_columns(sheet, selected)?;
    let ignored_non_numeric: Vec<(String, usize)> =
        cols.iter().filter(|c| c.non_numeric_count > 0).map(|c| (c.label.clone(), c.non_numeric_count)).collect();
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
    Ok((inputs, Some(res), raw_slices, ignored_non_numeric))
}

fn build_variance_homogeneity_tab(vt: &VarianceHomogeneityResult, p_fmt: PValueFormat) -> ResultTab {
    let headers = vec![String::from("Statistic"), String::from("DFn / df"), String::from("DFd"), String::from("P value"), String::from("Status")];
    let labels = vec![String::from("Brown-Forsythe"), String::from("Bartlett")];
    let mut tab = if vt.available && vt.brown_forsythe_p.is_finite() {
        let rows = vec![
            vec![
                num_cell(vt.brown_forsythe_f),
                num_cell(vt.brown_forsythe_df1),
                num_cell(vt.brown_forsythe_df2),
                p_cell(vt.brown_forsythe_p, p_fmt),
                ResultCell::Text(if vt.brown_forsythe_p >= 0.05 {
                    String::from("Equal variances not rejected")
                } else {
                    String::from("Variances differ")
                }),
            ],
            vec![
                num_cell(vt.bartlett_chi2),
                num_cell(vt.bartlett_df),
                ResultCell::Empty,
                p_cell(vt.bartlett_p, p_fmt),
                ResultCell::Text(if vt.bartlett_p >= 0.05 { String::from("Equal variances not rejected") } else { String::from("Variances differ") }),
            ],
        ];
        build_tab("variance-homogeneity", "Test for homogeneity of variances", ResultTabKind::Diagnostics, headers, labels, rows)
    } else {
        let rows = vec![
            vec![
                ResultCell::Empty,
                num_cell(vt.brown_forsythe_df1),
                num_cell(vt.brown_forsythe_df2),
                ResultCell::Empty,
                ResultCell::Text(String::from("Not computable from summary statistics")),
            ],
            vec![
                ResultCell::Empty,
                num_cell(vt.bartlett_df),
                ResultCell::Empty,
                ResultCell::Empty,
                ResultCell::Text(String::from("Not computable from summary statistics")),
            ],
        ];
        build_tab("variance-homogeneity", "Test for homogeneity of variances", ResultTabKind::Diagnostics, headers, labels, rows)
    };
    if !vt.available {
        tab.footnotes.push(String::from(
            "Brown-Forsythe and Bartlett variance homogeneity tests cannot be computed from pre-summarized Mean/SD/N data; raw replicate observations are required.",
        ));
    }
    tab
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
    if res.variance_tests.available && res.variance_tests.brown_forsythe_p.is_finite() {
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
    } else {
        out.checklist.push(checklist_item(
            "Homogeneity of variances (Brown-Forsythe / Bartlett)",
            ChecklistStatus::Info,
            Some(String::from("Not computable from summary statistics")),
            "Brown-Forsythe and Bartlett variance homogeneity tests are not computable from pre-summarized Mean/SD/N data; raw replicate observations are required.",
        ));
    }
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

    let (mut result, obs, aux_tabs) = match &sheet.archetype {
        TableArchetype::Grouped(grouped) => {
            let outcome = run_grouped_factorial(grouped, &mut out, &spec.selected_data_sets, fac_spec, p_fmt)?;
            (outcome.result, outcome.observations, outcome.aux_tabs)
        }
        TableArchetype::MultipleVariables(mv) => {
            let (res, obs) = run_multifactor_mv(mv, fac_spec)?;
            (res, obs, Vec::new())
        }
        other => return Err(StatsError::IncompatibleTable { analysis: "factorial ANOVA", actual: other.kind() }),
    };

    if result.terms.iter().any(|t| t.df > 0.0 && t.p_value.is_none()) {
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
    out.tabs.extend(aux_tabs);

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

    sanitize_result_sheet_floats(&mut out)?;
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
    if !result.df_residual.is_finite() || result.df_residual < 1.0 || !result.ms_residual.is_finite() || result.ms_residual < 0.0 {
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

struct GroupedFactorialOutcome {
    result: FactorialAnovaResult,
    observations: FactorialObservations,
    aux_tabs: Vec<ResultTab>,
}

fn build_rm_sphericity_tab(
    row_name: &str,
    col_name: &str,
    rm_res: &RmTwoWayAnovaResult,
    p_fmt: PValueFormat,
) -> Result<Option<ResultTab>, StatsError> {
    let design = rm_res.design;
    let res = &rm_res.anova;
    let sph = &rm_res.sphericities;
    let inter_name = format!("{row_name} × {col_name}");
    let entries: [(&str, Option<&SphericityResult>, usize); 3] = match design {
        RepeatedMeasuresDesign::BothFactorsMatched => {
            [(row_name, sph.row.as_ref(), 0), (col_name, sph.column.as_ref(), 1), (inter_name.as_str(), sph.interaction.as_ref(), 2)]
        }
        RepeatedMeasuresDesign::ColumnFactorMatched => {
            [(col_name, sph.column.as_ref(), 1), (inter_name.as_str(), sph.column.as_ref(), 2), ("", None, usize::MAX)]
        }
        RepeatedMeasuresDesign::RowFactorMatched => {
            [(row_name, sph.row.as_ref(), 0), (inter_name.as_str(), sph.row.as_ref(), 2), ("", None, usize::MAX)]
        }
        RepeatedMeasuresDesign::None => return Ok(None),
    };

    let headers = vec![
        String::from("Mauchly W"),
        String::from("Mauchly chi2"),
        String::from("df"),
        String::from("Mauchly P"),
        String::from("G-G epsilon"),
        String::from("H-F epsilon"),
        String::from("G-G P"),
        String::from("H-F P"),
    ];
    let mut row_labels = Vec::new();
    let mut rows = Vec::new();
    for (label, s_opt, term_idx) in entries {
        let (Some(s), Some(term), Some(&df_err)) = (s_opt, res.terms.get(term_idx), rm_res.term_error_dfs.get(term_idx)) else {
            continue;
        };
        let f_val = term.f_statistic.unwrap_or(0.0);
        let p_gg = apply_rm_sphericity_p(f_val, term.df, df_err, Some(s), RmSphericityCorrection::GreenhouseGeisser)?;
        let p_hf = apply_rm_sphericity_p(f_val, term.df, df_err, Some(s), RmSphericityCorrection::HuynhFeldt)?;
        row_labels.push(label.to_owned());
        rows.push(vec![
            num_cell(s.mauchly_w),
            num_cell(s.mauchly_chi2),
            num_cell(s.mauchly_df),
            p_cell(s.mauchly_p, p_fmt),
            num_cell(s.greenhouse_geisser_eps),
            num_cell(s.huynh_feldt_eps),
            p_cell(p_gg, p_fmt),
            p_cell(p_hf, p_fmt),
        ]);
    }
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(build_tab("rm-sphericity", "Repeated-measures sphericity", ResultTabKind::Diagnostics, headers, row_labels, rows)))
}

fn push_rm_sphericity_outputs(
    out: &mut ResultSheet,
    row_name: &str,
    col_name: &str,
    rm_res: &RmTwoWayAnovaResult,
    p_fmt: PValueFormat,
) -> Result<Option<ResultTab>, StatsError> {
    let sph = &rm_res.sphericities;
    let inter_name = format!("{row_name} × {col_name}");
    let items: [(&str, Option<&SphericityResult>); 3] =
        [(row_name, sph.row.as_ref()), (col_name, sph.column.as_ref()), (&inter_name, sph.interaction.as_ref())];
    let mut worst_gg = 1.0_f64;
    let mut summary_parts = Vec::new();
    for (name, s_opt) in items {
        let Some(s) = s_opt else {
            continue;
        };
        worst_gg = worst_gg.min(s.greenhouse_geisser_eps);
        push_scalar(out, format!("{name}: G-G epsilon"), None, s.greenhouse_geisser_eps);
        push_scalar(out, format!("{name}: H-F epsilon"), None, s.huynh_feldt_eps);
        summary_parts.push(format!("{name}: eps_GG={:.4}, eps_HF={:.4}, P={:.4}", s.greenhouse_geisser_eps, s.huynh_feldt_eps, s.mauchly_p));
    }
    if !summary_parts.is_empty() {
        out.checklist.push(checklist_item(
            "Sphericity (RM two-way)",
            if worst_gg < 0.75 { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            Some(summary_parts.join("; ")),
            "Greenhouse-Geisser and Huynh-Feldt epsilons summarize covariance sphericity across within-subject factors.",
        ));
    }
    build_rm_sphericity_tab(row_name, col_name, rm_res, p_fmt)
}

fn run_grouped_factorial(
    grouped: &lumencraft_doc::GroupedTable,
    out: &mut ResultSheet,
    selected: &[usize],
    fac_spec: &FactorialAnovaSpec,
    p_fmt: PValueFormat,
) -> Result<GroupedFactorialOutcome, StatsError> {
    if grouped.subcolumn_mode.is_pre_summarized() {
        return Err(StatsError::RequiresRawReplicates);
    }
    let n_rows = grouped.row_count();
    let selected_cols = resolve_selected_indices(selected, grouped.data_sets.len());
    let n_cols = selected_cols.len();
    if n_cols < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "factorial column levels", required: 2, actual: n_cols });
    }
    let SubcolumnMode::RawReplicates(n_subs) = grouped.subcolumn_mode else {
        return Err(StatsError::RequiresRawReplicates);
    };

    // Repeated-measures 2-way path (with automatic REML mixed-effects fallback on missing replicates).
    if !fac_spec.three_way && fac_spec.repeated_measures != RepeatedMeasuresDesign::None {
        let has_missing_selected = (0..n_rows).any(|r| {
            selected_cols.iter().any(|&c| (0..n_subs).any(|s| grouped.observation(r, c, s).and_then(|cell| cell.active_number()).is_none()))
        });
        if has_missing_selected && fac_spec.fit_reml_on_missing {
            return run_rm_two_way_reml_fallback(grouped, out, &selected_cols, fac_spec, p_fmt);
        }
        let mut subject_data = vec![vec![vec![0.0_f64; n_cols]; n_rows]; n_subs];
        let mut obs = Vec::with_capacity(n_subs * n_rows * n_cols);
        for (s, subj) in subject_data.iter_mut().enumerate() {
            for (r, row_vec) in subj.iter_mut().enumerate() {
                for (c_pos, (&orig_c, slot)) in selected_cols.iter().zip(row_vec.iter_mut()).enumerate() {
                    let Some(v) = grouped.observation(r, orig_c, s).and_then(|cell| cell.active_number()) else {
                        return run_rm_two_way_reml_fallback(grouped, out, &selected_cols, fac_spec, p_fmt);
                    };
                    *slot = v;
                    obs.push((vec![r, c_pos], v));
                }
            }
        }
        let rm_res = fit_rm_two_way_anova_with_sphericity(
            &subject_data,
            &grouped.row_factor_title,
            &grouped.column_factor_title,
            fac_spec.repeated_measures,
            RmSphericityCorrection::None,
        )?;
        let sph_tab = push_rm_sphericity_outputs(out, &grouped.row_factor_title, &grouped.column_factor_title, &rm_res, p_fmt)?;
        let data = FactorialObservations {
            factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
            factor_levels: vec![grouped_row_levels(grouped, n_rows), selected_cols.iter().map(|&c| grouped.data_sets[c].title.clone()).collect()],
            rows: obs,
        };
        return Ok(GroupedFactorialOutcome { result: rm_res.anova, observations: data, aux_tabs: sph_tab.into_iter().collect() });
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
            for (c_pos, &orig_c) in selected_cols.iter().enumerate() {
                let l1 = c_pos / 2;
                let l2 = c_pos % 2;
                for y in grouped.active_replicates(r, orig_c) {
                    obs.push((vec![l1, l2, r], y));
                }
            }
        }
        let data =
            FactorialObservations { factor_names: vec![f1_name, f2_name, f3_name], factor_levels: vec![f1_levels, f2_levels, f3_levels], rows: obs };
        let res = fit_factorial_anova(&data, fac_spec.max_interaction_order.clamp(1, 3), fac_spec.ss_type)?;
        return Ok(GroupedFactorialOutcome { result: res, observations: data, aux_tabs: Vec::new() });
    }

    // Standard Two-Way ANOVA (Row Factor x Column Factor).
    let row_levels = grouped_row_levels(grouped, n_rows);
    let col_levels: Vec<String> = selected_cols.iter().map(|&c| grouped.data_sets[c].title.clone()).collect();
    let mut obs = Vec::new();
    for r in 0..n_rows {
        for (c_pos, &orig_c) in selected_cols.iter().enumerate() {
            for y in grouped.active_replicates(r, orig_c) {
                obs.push((vec![r, c_pos], y));
            }
        }
    }
    let data = FactorialObservations {
        factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
        factor_levels: vec![row_levels, col_levels],
        rows: obs,
    };
    let res = fit_factorial_anova(&data, fac_spec.max_interaction_order.clamp(1, 2), fac_spec.ss_type)?;
    Ok(GroupedFactorialOutcome { result: res, observations: data, aux_tabs: Vec::new() })
}

fn push_reml_df_scalars(out: &mut ResultSheet, terms: &[FactorialAnovaTerm], satt: &[f64], kr: &[f64]) {
    for (idx, term) in terms.iter().enumerate() {
        if let Some(&df_s) = satt.get(idx) {
            push_scalar(out, format!("{}: Satterthwaite df", term.name), None, df_s);
        }
        if let Some(&df_k) = kr.get(idx) {
            push_scalar(out, format!("{}: Kenward-Roger df", term.name), None, df_k);
        }
    }
}

fn run_rm_two_way_reml_fallback(
    grouped: &lumencraft_doc::GroupedTable,
    out: &mut ResultSheet,
    selected_cols: &[usize],
    fac_spec: &FactorialAnovaSpec,
    p_fmt: PValueFormat,
) -> Result<GroupedFactorialOutcome, StatsError> {
    let n_rows = grouped.row_count();
    let n_cols = selected_cols.len();
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
    let mut subj_ids: Vec<usize> = Vec::new();
    let mut subj_row_ids: Vec<usize> = Vec::new();
    let mut subj_col_ids: Vec<usize> = Vec::new();
    for s in 0..n_subs {
        for (r, r_basis) in row_basis.iter().enumerate() {
            for (c_pos, (&orig_c, c_basis)) in selected_cols.iter().zip(col_basis.iter()).enumerate() {
                if let Some(v) = grouped.observation(r, orig_c, s).and_then(|cell| cell.active_number()) {
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
                    obs.push((vec![r, c_pos], v));
                    subj_ids.push(s);
                    subj_row_ids.push(s * n_rows + r);
                    subj_col_ids.push(s * n_cols + c_pos);
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

    let (fixed_terms, sigma2_e, reml_ll, icc_subj, vc_tab) = match fac_spec.repeated_measures {
        RepeatedMeasuresDesign::BothFactorsMatched => {
            let (g_s, c_s) = cluster_ids_to_g_matrix(&subj_ids)?;
            let (g_sr, c_sr) = cluster_ids_to_g_matrix(&subj_row_ids)?;
            let (g_sc, c_sc) = cluster_ids_to_g_matrix(&subj_col_ids)?;
            let sr_name = format!("subject × {}", grouped.row_factor_title);
            let sc_name = format!("subject × {}", grouped.column_factor_title);
            let mres = fit_multi_vc_reml(
                &x_mat,
                &y_vec,
                &[("subject", g_s, c_s), (sr_name.as_str(), g_sr, c_sr), (sc_name.as_str(), g_sc, c_sc)],
                &term_slices,
            )?;
            let s2_s = mres.variance_components.first().map_or(0.0, |(_, v)| *v);
            let s2_sr = mres.variance_components.get(1).map_or(0.0, |(_, v)| *v);
            let s2_sc = mres.variance_components.get(2).map_or(0.0, |(_, v)| *v);
            let tot = (s2_s + s2_sr + s2_sc + mres.sigma2_error).max(1e-18);
            let icc = s2_s / tot;

            push_scalar(out, "REML sigma^2 (subject)", None, s2_s);
            push_scalar(out, format!("REML sigma^2 ({sr_name})"), None, s2_sr);
            push_scalar(out, format!("REML sigma^2 ({sc_name})"), None, s2_sc);
            push_scalar(out, "REML sigma^2 (residual)", None, mres.sigma2_error);
            push_scalar(out, "REML ICC", None, icc);
            push_scalar(out, "REML log-likelihood", None, mres.reml_log_lik);
            push_reml_df_scalars(out, &mres.fixed_terms, &mres.satterthwaite_df, &mres.kenward_roger_df);

            let sr_title = format!("Subject × {}", grouped.row_factor_title);
            let sc_title = format!("Subject × {}", grouped.column_factor_title);
            let vc_headers = vec![String::from("Component"), String::from("Variance"), String::from("SD"), String::from("% of total")];
            let vc_rows = vec![
                vec![ResultCell::Text(String::from("Subject (random)")), num_cell(s2_s), num_cell(s2_s.sqrt()), num_cell(100.0 * s2_s / tot)],
                vec![ResultCell::Text(sr_title.clone()), num_cell(s2_sr), num_cell(s2_sr.sqrt()), num_cell(100.0 * s2_sr / tot)],
                vec![ResultCell::Text(sc_title.clone()), num_cell(s2_sc), num_cell(s2_sc.sqrt()), num_cell(100.0 * s2_sc / tot)],
                vec![
                    ResultCell::Text(String::from("Residual")),
                    num_cell(mres.sigma2_error),
                    num_cell(mres.sigma2_error.sqrt()),
                    num_cell(100.0 * mres.sigma2_error / tot),
                ],
            ];
            let tab = build_tab(
                "mixed-reml",
                "Mixed-effects REML variance components",
                ResultTabKind::ParametersAndConfidenceIntervals,
                vc_headers,
                vec![String::from("Subject"), sr_title, sc_title, String::from("Residual")],
                vc_rows,
            );
            (mres.fixed_terms, mres.sigma2_error, mres.reml_log_lik, icc, tab)
        }
        design => {
            let cluster_ids: Vec<usize> = match design {
                RepeatedMeasuresDesign::ColumnFactorMatched => subj_row_ids.clone(),
                RepeatedMeasuresDesign::RowFactorMatched => subj_col_ids.clone(),
                _ => subj_ids,
            };
            let reml = fit_random_intercept_reml(&x_mat, &y_vec, &cluster_ids, &term_slices)?;
            push_scalar(out, "REML sigma^2 (subject)", None, reml.sigma2_random);
            push_scalar(out, "REML sigma^2 (residual)", None, reml.sigma2_error);
            push_scalar(out, "REML ICC", None, reml.icc);
            push_scalar(out, "REML log-likelihood", None, reml.reml_log_lik);
            push_reml_df_scalars(out, &reml.fixed_terms, &reml.satterthwaite_df, &reml.kenward_roger_df);

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
                vec![
                    ResultCell::Text(String::from("Residual")),
                    num_cell(reml.sigma2_error),
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                ],
            ];
            let tab = build_tab(
                "mixed-reml",
                "Mixed-effects REML variance components",
                ResultTabKind::ParametersAndConfidenceIntervals,
                reml_headers,
                vec![String::from("Subject"), String::from("Residual")],
                reml_rows,
            );
            (reml.fixed_terms, reml.sigma2_error, reml.reml_log_lik, reml.icc, tab)
        }
    };

    out.tabs.push(vc_tab);
    out.checklist.push(checklist_item(
        "Missing-value mixed-effects REML",
        ChecklistStatus::Info,
        Some(format!("ICC = {icc_subj:.3}, REML logL = {reml_ll:.2}")),
        "Missing replicates detected in repeated-measures cells; fitted linear mixed-effects REML with Kenward-Roger degrees of freedom instead of listwise deletion.",
    ));

    let ym = SummaryMoments::from_slice(&y_vec)?;
    let res = FactorialAnovaResult {
        terms: fixed_terms,
        ss_residual: sigma2_e * ((n_obs - p_total) as f64),
        df_residual: (n_obs - p_total) as f64,
        ms_residual: sigma2_e,
        ss_total: ym.css,
        df_total: (n_obs - 1) as f64,
        total_n: n_obs,
        unbalanced: true,
        ss_type: SumOfSquaresType::TypeIII,
    };
    let data = FactorialObservations {
        factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
        factor_levels: vec![grouped_row_levels(grouped, n_rows), selected_cols.iter().map(|&c| grouped.data_sets[c].title.clone()).collect()],
        rows: obs,
    };
    Ok(GroupedFactorialOutcome { result: res, observations: data, aux_tabs: Vec::new() })
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
            let factor_var = mv.variables.get(c_idx).ok_or(StatsError::MissingColumn { index: c_idx })?;
            let cell = factor_var.cells.get(r).ok_or(StatsError::DimensionMismatch { expected: n_rows, actual: factor_var.cells.len() })?;
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

/// Execute a [`NestedAnalysisSpec`] against a `Nested` [`DataSheet`].
pub fn run_nested_analysis(sheet: &DataSheet, spec: &AnalysisSpec, nested_spec: &NestedAnalysisSpec) -> Result<ResultSheet, StatsError> {
    let TableArchetype::Nested(nested) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "nested ANOVA / nested t-test", actual: sheet.archetype.kind() });
    };
    let mut out = init_result_sheet(spec)?;
    let p_fmt = spec.p_value_format;

    let selected_groups = resolve_selected_indices(&spec.selected_data_sets, nested.groups.len());
    let res = nested_one_way_anova_selected(nested, &selected_groups, nested_spec.two_group_t_test)?;
    push_scalar(&mut out, "F (among groups)", None, res.f_among_groups);
    push_scalar(&mut out, "P value (groups)", None, res.p_among_groups);
    push_scalar(&mut out, "Subcluster variance", None, res.variance_subclusters);
    push_scalar(&mut out, "Residual variance", None, res.variance_error);
    push_scalar(&mut out, "ICC (subcluster)", None, res.icc_subclusters);
    if let Some(t_val) = res.t_nested {
        push_scalar(&mut out, "Nested t", None, t_val);
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

    // Optional group-level pairwise comparisons using hierarchical group-mean variances:
    // Var(mean_g) = sigma_u^2 * sum_j(n_{gj}^2) / N_g^2 + sigma_e^2 / N_g (Sokal & Rohlf / Montgomery).
    if !nested_spec.two_group_t_test && selected_groups.len() >= 3 {
        append_nested_multiplicity(&mut out, nested, &selected_groups, &res, spec, nested_spec)?;
    }

    out.checklist.push(checklist_item(
        "Biological vs. technical nesting",
        if res.icc_subclusters > 0.05 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("ICC = {:.3}, KR df = {:.2}", res.icc_subclusters, res.df2_groups_kenward_roger)),
        "Nested REML prevents pseudoreplication by separating biological-unit variance from technical replicate variance.",
    ));
    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

fn append_nested_multiplicity(
    out: &mut ResultSheet,
    nested: &lumencraft_doc::NestedTable,
    selected_groups: &[usize],
    res: &NestedAnovaResult,
    spec: &AnalysisSpec,
    nested_spec: &NestedAnalysisSpec,
) -> Result<(), StatsError> {
    let p_fmt = spec.p_value_format;
    let confidence = spec.confidence_level;
    let is_welch = matches!(
        MultiplicityProcedure::from(nested_spec.multiplicity.method),
        MultiplicityProcedure::GamesHowell | MultiplicityProcedure::TamhaneT2 | MultiplicityProcedure::DunnettT3
    );
    let mut group_stats = Vec::with_capacity(selected_groups.len());
    for &g_idx in selected_groups {
        let g = nested.groups.get(g_idx).ok_or(StatsError::MissingColumn { index: g_idx })?;
        let mut vals = Vec::new();
        let mut m_g = 0_usize;
        let mut sum_n_sq = 0.0_f64;
        for b in 0..g.biological_unit_count() {
            let reps = nested.active_technical_replicates(g_idx, b);
            if !reps.is_empty() {
                m_g += 1;
                let n_b = reps.len() as f64;
                sum_n_sq += n_b * n_b;
                vals.extend(reps);
            }
        }
        let m = SummaryMoments::from_slice(&vals)?;
        let total_n_g = vals.len() as f64;
        let var_mean_g = res.variance_subclusters * (sum_n_sq / (total_n_g * total_n_g)) + res.variance_error / total_n_g;
        group_stats.push((g_idx, g.title.clone(), m.mean, m_g as f64, var_mean_g.max(0.0)));
    }
    let all_pos = group_stats.iter().all(|(_, _, _, _, v)| *v > 0.0);
    let mse_eff = if is_welch || !all_pos { 0.0 } else { 1.0 };
    let mut g_inputs = Vec::with_capacity(group_stats.len());
    for (g_idx, label, mean, m_g, var_mean_g) in group_stats {
        let n_eff = if !is_welch && all_pos { 1.0 / var_mean_g } else { m_g.max(1.0) };
        let sd_eff = (m_g.max(1.0) * var_mean_g).sqrt();
        g_inputs.push(GroupComparisonInput { index: g_idx, label, n: n_eff, mean, sd: sd_eff });
    }
    if nested_spec.multiplicity.method == MultiplicityMethod::LinearTrend {
        let trend = linear_trend_test(&g_inputs, mse_eff, res.df2_groups_kenward_roger)?;
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
    } else {
        let details = compare_groups_pairwise(&g_inputs, mse_eff, res.df2_groups_kenward_roger, &nested_spec.multiplicity)?;
        push_pairwise_outputs(out, &details, "Nested multiple comparisons", p_fmt, confidence);
    }
    Ok(())
}
