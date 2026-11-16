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
//! - Lecoutre, B. (1991). A correction for the epsilon-tilde approximate test in repeated measures
//!   designs with two or more independent groups. *Journal of Educational Statistics*
//!   16(4):371–372. doi:10.3102/10769986016004371
//! - Copenhaver, M. D., & Holland, B. S. (1988). Computation of the distribution of the maximum
//!   studentized range statistic with application to multiple significance testing of simple
//!   effects. *Journal of Statistical Computation and Simulation* 30(1):1–15.
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
//! - Pinheiro, J. C., & Bates, D. M. (1996). Unconstrained parametrizations for variance-covariance
//!   matrices. *Statistics and Computing* 6(3):289–296. doi:10.1007/BF00140873
//! - Lindstrom, M. J., & Bates, D. M. (1988). Newton-Raphson and EM algorithms for linear
//!   mixed-effects models for repeated-measures data. *Journal of the American Statistical
//!   Association* 83(404):1014–1022. doi:10.1080/01621459.1988.10478693
//! - Verbeke, G., & Molenberghs, G. (2000). *Linear Mixed Models for Longitudinal Data*. Springer.
//! - Tomczak, M., & Tomczak, E. (2014). The need to report effect size estimates revisited. An
//!   overview of some recommended measures of effect size. *Trends in Sport Sciences* 1(21):19–25.

use curvebook_doc::{
    AnalysisSpec, ChecklistItem, ChecklistStatus, DataSheet, FactorialAnovaSpec, MultiplicityMethod, MultiplicitySpec, NestedAnalysisSpec,
    OneWayAnovaSpec, OneWayMethod, PValueFormat, PairwiseComparisonRecord, RepeatedMeasuresDesign, ResidualCovariance, ResultCell, ResultSheet,
    ResultTab, ResultTabKind, SubcolumnMode, SumOfSquaresType, TableArchetype,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{
    chi2_sf, dunnett_two_sided_crit_loadings, dunnett_two_sided_p_loadings, f_sf, max_studentized_range_isf, max_studentized_range_sf,
    noncentral_f_ci, student_t_two_sided_isf, student_t_two_sided_p,
};
use curvebook_math::linalg::{DenseMatrix, cholesky, qr_col_pivoted, solve_linear_least_squares, sym_eigen};
use curvebook_math::sum::{NeumaierSum, SummaryMoments};
use serde::{Deserialize, Serialize};

use crate::column::shapiro_wilk;
use crate::common::{
    append_ignored_text_checklist_and_footnote, append_listwise_filtered_checklist_and_footnote, build_tab, checklist_item, extract_numeric_columns,
    init_result_sheet, num_cell, opt_ci_cell, opt_num_cell, p_cell, push_scalar, quantile_type6_sorted, rank_with_ties, resolve_selected_indices,
    sanitize_result_sheet_floats, sorted_finite, validate_finite_slice,
};
use crate::error::StatsError;
use crate::multiplicity::{
    GroupComparisonInput, MultiplicityProcedure, PairwiseComparisonDetail, adjust_p_values, build_multiplicity_tab, compare_dunn_ranks,
    compare_groups_pairwise_with_confidence, compare_groups_with_confidence, linear_trend_test,
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

/// Reason an unstructured repeated-measures REML covariance fit engaged a regularized
/// positive-definite estimator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnstructuredFallbackReason {
    /// Only one subject/cluster was present, leaving within-cluster covariance unidentifiable.
    SingleSubject,
    /// Sample degrees of freedom (`n - p` or cluster count relative to active timepoints) were
    /// insufficient to identify all `t(t + 1) / 2` covariance parameters without regularization.
    SmallSampleRankDeficient,
    /// Unconstrained REML estimate approached the boundary of the positive-definite cone; applied
    /// eigenvalue/diagonal shrinkage regularization.
    SingularCovarianceEstimate,
    /// Quasi-Newton Cholesky REML reached its iteration budget before strict gradient tolerance;
    /// returned the monotone positive-definite REML iterate or regularized fallback.
    IterationLimitReached,
}

/// Unstructured repeated-measures REML covariance and Kenward–Roger fixed-effects output
/// (Pinheiro & Bates, 1996; Kenward & Roger, 1997; Fai & Cornelius, 1996).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnstructuredRemlResult {
    /// Labels of active observed timepoints ordered consistently with the matrices below.
    pub timepoint_labels: Vec<String>,
    /// Original 0-based timepoint indices retained (excluding any all-missing timepoints).
    pub active_timepoint_indices: Vec<usize>,
    /// Estimated `t_obs x t_obs` positive-definite covariance matrix `\hat{\Sigma}`.
    pub covariance_matrix: Vec<Vec<f64>>,
    /// Estimated `t_obs x t_obs` correlation matrix derived from `\hat{\Sigma}`.
    pub correlation_matrix: Vec<Vec<f64>>,
    /// Mean diagonal residual variance `(1 / t_obs) tr(\hat{\Sigma})`.
    pub mean_diagonal_variance: f64,
    pub reml_log_lik: f64,
    pub fixed_terms: Vec<FactorialAnovaTerm>,
    pub satterthwaite_df: Vec<f64>,
    pub kenward_roger_df: Vec<f64>,
    pub converged: bool,
    pub iterations: usize,
    pub fallback_reason: Option<UnstructuredFallbackReason>,
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
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: k, actual: row.len() }));
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
                return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: k, actual: row.len() }));
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
/// matrix `S` (`df_e` error df, `n_subj` total subjects across `n_groups` independent groups) via
/// an orthonormal contrast projection `M S M^T`.
///
/// - Mauchly (1940) `W = det(C) / (tr(C)/p)^p` using chi-square approximation on `p(p + 1)/2 - 1` df
///   when `df_e >= p`; when `df_e < p` (singular Wishart), Mauchly's `p` is reported as `NaN` (inestimable).
/// - Greenhouse & Geisser (1959) `epsilon_hat = (sum lambda)^2 / (p sum lambda^2)` bounded in `[1/p, 1]`
///   (falling back to the conservative lower bound `1/p` when `df_e < 1` or contrast variance is zero).
/// - Huynh & Feldt (1976, p. 74) with the split-plot `(N - g + 1)` correction of Lecoutre (1991,
///   *J. Educ. Stat.* 16:371–372): `epsilon_tilde = ((N - g + 1) p epsilon_hat - 2) / (p (df_e - p epsilon_hat))`,
///   clamped to `[epsilon_hat, 1]`.
fn compute_sphericity_from_cov_contrast(
    s_cov: &DenseMatrix,
    contrast: &DenseMatrix,
    df_e: f64,
    n_subj: f64,
    n_groups: f64,
) -> Result<SphericityResult, StatsError> {
    let p = contrast.nrows();
    if p <= 1 {
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
    let lower_bound = 1.0 / pf;
    let m_df = 0.5 * pf * (pf + 1.0) - 1.0;
    if !df_e.is_finite() || df_e < 1.0 {
        return Ok(SphericityResult {
            mauchly_w: f64::NAN,
            mauchly_chi2: f64::NAN,
            mauchly_df: m_df,
            mauchly_p: f64::NAN,
            greenhouse_geisser_eps: lower_bound,
            huynh_feldt_eps: lower_bound,
        });
    }
    let c_cov = symmetrize_dense(&contrast.matmul(s_cov)?.matmul(&contrast.transpose())?)?;
    let eig = sym_eigen(&c_cov)?;
    let eigvals: Vec<f64> = eig.eigenvalues.iter().map(|&x| x.max(0.0)).collect();
    let tr: f64 = eigvals.iter().sum();
    let tr_sq: f64 = eigvals.iter().map(|&l| l * l).sum();
    if tr <= 0.0 || tr_sq <= 0.0 {
        return Ok(SphericityResult {
            mauchly_w: f64::NAN,
            mauchly_chi2: f64::NAN,
            mauchly_df: m_df,
            mauchly_p: f64::NAN,
            greenhouse_geisser_eps: lower_bound,
            huynh_feldt_eps: lower_bound,
        });
    }
    let eps_gg = ((tr * tr) / (pf * tr_sq)).clamp(lower_bound, 1.0);
    let n_eff = n_subj - n_groups + 1.0;
    let hf_num = n_eff * pf * eps_gg - 2.0;
    let hf_denom = pf * (df_e - pf * eps_gg);
    let eps_hf = if df_e <= 1.0 {
        eps_gg
    } else if hf_denom > 0.0 {
        (hf_num / hf_denom).clamp(eps_gg, 1.0)
    } else if hf_num > 0.0 {
        1.0
    } else {
        eps_gg
    };

    if df_e < pf {
        return Ok(SphericityResult {
            mauchly_w: 0.0,
            mauchly_chi2: f64::NAN,
            mauchly_df: m_df,
            mauchly_p: f64::NAN,
            greenhouse_geisser_eps: eps_gg,
            huynh_feldt_eps: eps_hf,
        });
    }

    // Normalize eigenvalues by mean_eig = tr / p before taking logs so Mauchly W is scale-invariant.
    let mean_eig = tr / pf;
    let ln_w: f64 = eigvals.iter().map(|&v| (v / mean_eig).max(1e-15).ln()).sum::<f64>().min(0.0);
    let w = ln_w.exp().clamp(1e-15, 1.0);
    let f_scale = (1.0 - (2.0 * pf * pf + pf + 2.0) / (6.0 * pf * df_e)).max(0.0);
    let chi2 = (-df_e * f_scale * ln_w).max(0.0);
    let mauchly_p = if m_df > 0.0 { chi2_sf(chi2, m_df)? } else { 1.0 };

    Ok(SphericityResult { mauchly_w: w, mauchly_chi2: chi2, mauchly_df: m_df, mauchly_p, greenhouse_geisser_eps: eps_gg, huynh_feldt_eps: eps_hf })
}

fn compute_sphericity(blocks: &[Vec<f64>]) -> Result<SphericityResult, StatsError> {
    let n = blocks.len();
    let k = blocks.first().map_or(0, Vec::len);
    if k <= 2 {
        return Ok(SphericityResult {
            mauchly_w: 1.0,
            mauchly_chi2: 0.0,
            mauchly_df: 0.0,
            mauchly_p: 1.0,
            greenhouse_geisser_eps: 1.0,
            huynh_feldt_eps: 1.0,
        });
    }
    if n < 2 {
        let pf = (k - 1) as f64;
        let lb = 1.0 / pf;
        return Ok(SphericityResult {
            mauchly_w: f64::NAN,
            mauchly_chi2: f64::NAN,
            mauchly_df: 0.5 * pf * (pf + 1.0) - 1.0,
            mauchly_p: f64::NAN,
            greenhouse_geisser_eps: lb,
            huynh_feldt_eps: lb,
        });
    }
    let s_cov = pooled_within_group_cov(&[blocks.to_vec()])?;
    let h_mat = helmert_contrast_matrix(k)?;
    let nf = n as f64;
    compute_sphericity_from_cov_contrast(&s_cov, &h_mat, nf - 1.0, nf, 1.0)
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
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: k, actual: row.len() }));
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

#[derive(Debug, Clone)]
struct CellSampleStats {
    count: usize,
    var_unbiased: f64,
}

#[derive(Debug, Clone)]
struct OlsPostHocModel {
    factor_levels: Vec<Vec<String>>,
    factor_basis: Vec<Vec<Vec<f64>>>,
    subsets: Vec<Vec<usize>>,
    total_cols: usize,
    beta: Vec<f64>,
    cov_unscaled: DenseMatrix,
    row_proj: DenseMatrix,
    ms_resid: f64,
    df_resid: f64,
    cell_stats: std::collections::BTreeMap<Vec<usize>, CellSampleStats>,
}

fn build_xtx_pinv_and_proj(x: &DenseMatrix) -> Result<(DenseMatrix, DenseMatrix), StatsError> {
    let p = x.ncols();
    let xtx = symmetrize_dense(&x.transpose().matmul(x)?)?;
    let eig = sym_eigen(&xtx)?;
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    let tol = max_ev.max(1.0) * (p.max(1) as f64) * 1e-11;
    let mut pinv = vec![0.0_f64; p * p];
    let mut proj = vec![0.0_f64; p * p];
    for (k, &ev) in eig.eigenvalues.iter().enumerate() {
        if ev <= tol {
            continue;
        }
        let inv_ev = 1.0 / ev;
        for i in 0..p {
            let qi = eig.eigenvectors.get(i, k).unwrap_or(0.0);
            for j in i..p {
                let qj = eig.eigenvectors.get(j, k).unwrap_or(0.0);
                let dp = qi * qj;
                let di = dp * inv_ev;
                proj[i * p + j] += dp;
                pinv[i * p + j] += di;
                if i != j {
                    proj[j * p + i] += dp;
                    pinv[j * p + i] += di;
                }
            }
        }
    }
    Ok((DenseMatrix::from_row_major(p, p, pinv)?, DenseMatrix::from_row_major(p, p, proj)?))
}

fn build_cell_stats_map(rows: &[(Vec<usize>, f64)]) -> Result<std::collections::BTreeMap<Vec<usize>, CellSampleStats>, StatsError> {
    let mut grouped: std::collections::BTreeMap<Vec<usize>, Vec<f64>> = std::collections::BTreeMap::new();
    for (lvls, y) in rows {
        grouped.entry(lvls.clone()).or_default().push(*y);
    }
    let mut out = std::collections::BTreeMap::new();
    for (cell, vals) in grouped {
        let count = vals.len();
        let m = SummaryMoments::from_slice(&vals)?;
        let var_unbiased = if count >= 2 && m.css.is_finite() { (m.css / ((count - 1) as f64)).max(0.0) } else { 0.0 };
        out.insert(cell, CellSampleStats { count, var_unbiased });
    }
    Ok(out)
}

/// Fit Type I (sequential) or Type III (partial, sum/effect-coded) general `N`-way factorial ANOVA
/// via `curvebook_math::linalg::solve_linear_least_squares`.
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
    let (res, _) = fit_factorial_anova_with_model(data, max_interaction_order, ss_type)?;
    Ok(res)
}

fn fit_factorial_anova_with_model(
    data: &FactorialObservations,
    max_interaction_order: usize,
    ss_type: SumOfSquaresType,
) -> Result<(FactorialAnovaResult, OlsPostHocModel), StatsError> {
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
    let (cov_unscaled, row_proj) = build_xtx_pinv_and_proj(&full_x)?;
    let cell_stats = build_cell_stats_map(&data.rows)?;
    let model = OlsPostHocModel {
        factor_levels: data.factor_levels.clone(),
        factor_basis,
        subsets,
        total_cols,
        beta: full_sol.coefficients,
        cov_unscaled,
        row_proj,
        ms_resid,
        df_resid,
        cell_stats,
    };
    Ok((res, model))
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
                row: Some(compute_sphericity_from_cov_contrast(&s_cov, &m_row, df_e, sf, 1.0)?),
                column: Some(compute_sphericity_from_cov_contrast(&s_cov, &m_col, df_e, sf, 1.0)?),
                interaction: Some(compute_sphericity_from_cov_contrast(&s_cov, &m_int, df_e, sf, 1.0)?),
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
            let sph_col = compute_sphericity_from_cov_contrast(&s_pool, &m_col, df_e, n_subj, af)?;
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
            let sph_row = compute_sphericity_from_cov_contrast(&s_pool, &m_row, df_e, n_subj, bf)?;
            Ok(RmTwoWaySphericities { row: Some(sph_row), column: None, interaction: None })
        }
        RepeatedMeasuresDesign::None => Ok(RmTwoWaySphericities::default()),
    }
}

#[derive(Debug, Clone)]
struct RmBalancedPostHocModel {
    design: RepeatedMeasuresDesign,
    s_count: usize,
    a_count: usize,
    b_count: usize,
    row_means: Vec<f64>,
    col_means: Vec<f64>,
    cell_means: Vec<Vec<f64>>,
    err_a_ms: f64,
    err_a_df: f64,
    err_b_ms: f64,
    err_b_df: f64,
    err_ab_ms: f64,
    err_ab_df: f64,
    subject_data: Vec<Vec<Vec<f64>>>,
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
    let (res, _) = fit_rm_two_way_anova_with_posthoc(subject_data, row_name, col_name, rm_design, correction)?;
    Ok(res)
}

fn fit_rm_two_way_anova_with_posthoc(
    subject_data: &[Vec<Vec<f64>>],
    row_name: &str,
    col_name: &str,
    rm_design: RepeatedMeasuresDesign,
    correction: RmSphericityCorrection,
) -> Result<(RmTwoWayAnovaResult, RmBalancedPostHocModel), StatsError> {
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
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: a, actual: mat.len() }));
        }
        let mut s_acc = NeumaierSum::new();
        for (i, r_vec) in mat.iter().enumerate() {
            if r_vec.len() != b {
                return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: b, actual: r_vec.len() }));
            }
            for (j, &v) in r_vec.iter().enumerate() {
                if !v.is_finite() {
                    return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "y", value: v }));
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
    let rm_res = RmTwoWayAnovaResult { anova, design: rm_design, sphericities, correction, term_error_dfs: [err_a_df, err_b_df, err_ab_df] };
    let post_hoc = RmBalancedPostHocModel {
        design: rm_design,
        s_count,
        a_count: a,
        b_count: b,
        row_means,
        col_means,
        cell_means,
        err_a_ms: err_a_ss / err_a_df.max(1.0),
        err_a_df,
        err_b_ms: err_b_ss / err_b_df.max(1.0),
        err_b_df,
        err_ab_ms: ms_resid,
        err_ab_df,
        subject_data: subject_data.to_vec(),
    };
    Ok((rm_res, post_hoc))
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
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: nr, actual: nc }));
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
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: a.nrows(), actual: b.nrows() }));
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

struct RemlContrastCore {
    beta: Vec<f64>,
    phi: DenseMatrix,
    phi_a: DenseMatrix,
    w_info_inv: DenseMatrix,
    phi_m_phi: Vec<DenseMatrix>,
    n_minus_p: f64,
    y_scale: f64,
    col_map: Vec<Option<usize>>,
}

fn eval_reml_wald_terms_with_core(
    phi: &DenseMatrix,
    beta: &[f64],
    sigma2_e: f64,
    fisher: &OwnedRemlFisherMats,
    terms: &[(&str, usize, usize)],
    n_minus_p: f64,
) -> Result<(RemlWaldTermsOut, RemlContrastCore), StatsError> {
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
    let core = RemlContrastCore {
        beta: beta.to_vec(),
        phi: phi.clone(),
        phi_a,
        w_info_inv,
        phi_m_phi,
        n_minus_p,
        y_scale: 1.0,
        col_map: (0..p).map(Some).collect(),
    };
    Ok(((fixed_terms, satt_dfs, kr_dfs), core))
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
    fit_random_intercept_reml_with_core(x, y, cluster_ids, terms).map(|(res, _)| res)
}

fn fit_random_intercept_reml_with_core(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    terms: &[(&str, usize, usize)],
) -> Result<(MixedRemlResult, RemlContrastCore), StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
    }
    if cluster_ids.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: cluster_ids.len() }));
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
    let ((fixed_terms, satterthwaite_df, kenward_roger_df), core) = eval_reml_wald_terms_with_core(&phi, &beta, sigma2_e, &fisher, terms, n_minus_p)?;

    let res = MixedRemlResult {
        sigma2_random: sigma2_u,
        sigma2_error: sigma2_e,
        icc,
        reml_log_lik,
        lrt_random_chi2: lrt_chi2,
        lrt_random_p: lrt_p,
        fixed_terms,
        satterthwaite_df,
        kenward_roger_df,
    };
    Ok((res, core))
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
    fit_multi_vc_reml_with_core(x, y, random_effects, terms).map(|(res, _)| res)
}

fn fit_multi_vc_reml_with_core(
    x: &DenseMatrix,
    y: &[f64],
    random_effects: &[(&str, DenseMatrix, usize)],
    terms: &[(&str, usize, usize)],
) -> Result<(MultiVcRemlResult, RemlContrastCore), StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
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
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: g_mat.nrows().min(g_mat.ncols()) }));
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
    let ((mut fixed_terms, satterthwaite_df, kenward_roger_df), mut core) =
        eval_reml_wald_terms_with_core(&st.phi, &st.beta, theta[0], &fisher, terms, n_minus_p)?;
    core.y_scale = y_scale;
    for term in &mut fixed_terms {
        if let Some(f_val) = term.f_statistic {
            let ms = f_val * sigma2_error;
            term.ms = ms;
            term.ss = ms * term.df;
        }
    }
    let variance_components = names.into_iter().zip(theta.iter().skip(1).map(|&v| v * y_scale * y_scale)).collect();
    let reml_log_lik = st.ll - n_minus_p * y_scale.ln();

    let res = MultiVcRemlResult { variance_components, sigma2_error, reml_log_lik, fixed_terms, satterthwaite_df, kenward_roger_df };
    Ok((res, core))
}

const LOG_CHOL_DIAG_MIN: f64 = -9.0;
const LOG_CHOL_DIAG_MAX: f64 = 9.0;

fn chol_diag_mask(t: usize) -> Vec<bool> {
    let mut is_diag = Vec::with_capacity(t * (t + 1) / 2);
    for i in 0..t {
        for j in 0..=i {
            is_diag.push(i == j);
        }
    }
    is_diag
}

struct UnstructuredClusterBlock {
    x_c: DenseMatrix,
    y_c: Vec<f64>,
    tps: Vec<usize>,
}

struct UnstructuredEval {
    ll: f64,
    beta: Vec<f64>,
    phi: DenseMatrix,
    g_sigma: DenseMatrix,
}

struct UnstructuredPrepared {
    blocks: Vec<UnstructuredClusterBlock>,
    reduced_terms: Vec<(String, usize, usize)>,
    col_map: Vec<Option<usize>>,
    active_tp_indices: Vec<usize>,
    active_labels: Vec<String>,
    p_eff: usize,
    n_obs: usize,
    y_scale: f64,
}

fn sigma_from_log_chol(eta: &[f64], t: usize) -> Result<(DenseMatrix, DenseMatrix), StatsError> {
    let mut l_data = vec![0.0_f64; t * t];
    let mut idx = 0_usize;
    for i in 0..t {
        for j in 0..i {
            l_data[i * t + j] = eta[idx];
            idx += 1;
        }
        l_data[i * t + i] = eta[idx].clamp(LOG_CHOL_DIAG_MIN, LOG_CHOL_DIAG_MAX).exp();
        idx += 1;
    }
    let l_mat = DenseMatrix::from_row_major(t, t, l_data)?;
    let sigma = symmetrize_dense(&l_mat.matmul(&l_mat.transpose())?)?;
    Ok((l_mat, sigma))
}

fn log_chol_from_sigma(sigma: &DenseMatrix) -> Result<Vec<f64>, StatsError> {
    let chol = cholesky(&symmetrize_dense(sigma)?)?;
    let l_mat = chol.l();
    let t = l_mat.nrows();
    let mut eta = Vec::with_capacity(t * (t + 1) / 2);
    for i in 0..t {
        for j in 0..i {
            eta.push(l_mat.get(i, j).unwrap_or(0.0));
        }
        let d = l_mat.get(i, i).unwrap_or(1.0).max(1e-12);
        eta.push(d.ln().clamp(LOG_CHOL_DIAG_MIN, LOG_CHOL_DIAG_MAX));
    }
    Ok(eta)
}

fn log_chol_neg_grad(g_sigma: &DenseMatrix, l_mat: &DenseMatrix) -> Result<Vec<f64>, StatsError> {
    let gl = g_sigma.matmul(l_mat)?;
    let t = l_mat.nrows();
    let mut g_neg = Vec::with_capacity(t * (t + 1) / 2);
    for i in 0..t {
        for j in 0..i {
            g_neg.push(-2.0 * gl.get(i, j).unwrap_or(0.0));
        }
        g_neg.push(-2.0 * gl.get(i, i).unwrap_or(0.0) * l_mat.get(i, i).unwrap_or(1.0));
    }
    Ok(g_neg)
}

fn unstructured_floor_spd(mat: &DenseMatrix, min_ratio: f64, default_scale: f64) -> Result<(DenseMatrix, bool), StatsError> {
    let sym = symmetrize_dense(mat)?;
    let eig = sym_eigen(&sym)?;
    let t = sym.nrows();
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    let floor = if max_ev > 1e-30 { (min_ratio * max_ev).max(1e-12) } else { default_scale.max(1e-8) };
    let mut adjusted = max_ev <= 1e-30;
    let mut out = vec![0.0_f64; t * t];
    for (k, &ev_raw) in eig.eigenvalues.iter().enumerate() {
        let ev = if ev_raw < floor {
            adjusted = true;
            floor
        } else {
            ev_raw
        };
        for i in 0..t {
            let qi = eig.eigenvectors.get(i, k).unwrap_or(0.0);
            for j in i..t {
                let qj = eig.eigenvectors.get(j, k).unwrap_or(0.0);
                let delta = ev * qi * qj;
                out[i * t + j] += delta;
                if i != j {
                    out[j * t + i] += delta;
                }
            }
        }
    }
    Ok((symmetrize_dense(&DenseMatrix::from_row_major(t, t, out)?)?, adjusted))
}

fn unstructured_eigen_ratio(mat: &DenseMatrix) -> Result<(f64, f64, f64), StatsError> {
    let eig = sym_eigen(&symmetrize_dense(mat)?)?;
    let max_ev = eig.eigenvalues.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min_ev = eig.eigenvalues.iter().copied().fold(f64::INFINITY, f64::min);
    let t = mat.nrows().max(1) as f64;
    let mean_diag: f64 = (0..mat.nrows()).map(|i| mat.get(i, i).unwrap_or(0.0).max(0.0)).sum::<f64>() / t;
    Ok((min_ev, max_ev, mean_diag))
}

fn unstructured_shrink_spd(mat: &DenseMatrix, target_ratio: f64, fallback_var: f64) -> Result<DenseMatrix, StatsError> {
    let eig = sym_eigen(&symmetrize_dense(mat)?)?;
    let t = mat.nrows();
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    if max_ev <= 1e-30 {
        return scale_dense(&DenseMatrix::identity(t)?, fallback_var.max(1e-8));
    }
    let min_ev = eig.eigenvalues.iter().copied().fold(f64::INFINITY, f64::min).max(0.0);
    let mut psd = vec![0.0_f64; t * t];
    for (k, &ev_raw) in eig.eigenvalues.iter().enumerate() {
        let ev = ev_raw.max(0.0);
        if ev == 0.0 {
            continue;
        }
        for i in 0..t {
            let qi = eig.eigenvectors.get(i, k).unwrap_or(0.0);
            for j in i..t {
                let qj = eig.eigenvectors.get(j, k).unwrap_or(0.0);
                let delta = ev * qi * qj;
                psd[i * t + j] += delta;
                if i != j {
                    psd[j * t + i] += delta;
                }
            }
        }
    }
    let mean_diag: f64 = (0..t).map(|i| psd[i * t + i]).sum::<f64>() / (t.max(1) as f64);
    if mean_diag <= 1e-30 {
        return scale_dense(&DenseMatrix::identity(t)?, fallback_var.max(1e-8));
    }
    if min_ev / max_ev >= target_ratio {
        return symmetrize_dense(&DenseMatrix::from_row_major(t, t, psd)?);
    }
    let denom = (mean_diag - min_ev) + target_ratio * (max_ev - mean_diag);
    let delta = if denom > 1e-30 { ((target_ratio * max_ev - min_ev) / denom).clamp(0.005, 0.25) } else { 0.05 };
    for i in 0..t {
        for j in 0..t {
            psd[i * t + j] *= 1.0 - delta;
            if i == j {
                psd[i * t + j] += delta * mean_diag;
            }
        }
    }
    symmetrize_dense(&DenseMatrix::from_row_major(t, t, psd)?)
}

type ReducedDesignOut = (DenseMatrix, Vec<(String, usize, usize)>, Vec<Option<usize>>, usize);

fn unstructured_reduce_design(x: &DenseMatrix, terms: &[(&str, usize, usize)]) -> Result<ReducedDesignOut, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    let qr = qr_col_pivoted(x)?;
    if qr.rank() == p {
        let kept_terms = terms.iter().map(|&(name, s, l)| (name.to_owned(), s, l)).collect();
        let col_map = (0..p).map(Some).collect();
        return Ok((x.clone(), kept_terms, col_map, p));
    }

    // Sequential re-orthogonalized Modified Gram-Schmidt preserves term column ordering while
    // dropping structurally aliased columns (e.g., when a factor level/timepoint is all missing).
    let mut keep = vec![false; p];
    let mut q_cols: Vec<Vec<f64>> = Vec::with_capacity(p);
    for (j, slot) in keep.iter_mut().enumerate() {
        let mut w: Vec<f64> = (0..n).map(|r| x.get(r, j).unwrap_or(0.0)).collect();
        let norm0 = w.iter().map(|&v| v * v).sum::<f64>().sqrt();
        if norm0 <= 1e-14 {
            continue;
        }
        for _ in 0..2 {
            for qk in &q_cols {
                let dot: f64 = w.iter().zip(qk.iter()).map(|(&a, &b)| a * b).sum();
                for (wi, &qi) in w.iter_mut().zip(qk.iter()) {
                    *wi -= dot * qi;
                }
            }
        }
        let norm_rem = w.iter().map(|&v| v * v).sum::<f64>().sqrt();
        if norm_rem > 1e-10 * norm0 {
            *slot = true;
            let inv = 1.0 / norm_rem;
            for wi in &mut w {
                *wi *= inv;
            }
            q_cols.push(w);
        }
    }

    let mut col_map = vec![None; p];
    let mut p_eff = 0_usize;
    for (j, &is_kept) in keep.iter().enumerate() {
        if is_kept {
            col_map[j] = Some(p_eff);
            p_eff += 1;
        }
    }
    if p_eff == 0 {
        return Err(StatsError::EmptyInput);
    }

    let mut x_data = Vec::with_capacity(n * p_eff);
    for r in 0..n {
        for (j, &is_kept) in keep.iter().enumerate() {
            if is_kept {
                x_data.push(x.get(r, j).unwrap_or(0.0));
            }
        }
    }
    let x_red = DenseMatrix::from_row_major(n, p_eff, x_data)?;
    let mut reduced_terms = Vec::with_capacity(terms.len());
    for &(name, start, len) in terms {
        let end = start.saturating_add(len).min(p);
        let mapped: Vec<usize> = col_map[start..end].iter().filter_map(|&m| m).collect();
        let new_start = mapped.first().copied().unwrap_or(0);
        reduced_terms.push((name.to_owned(), new_start, mapped.len()));
    }
    Ok((x_red, reduced_terms, col_map, p_eff))
}

fn unstructured_prepare_clusters(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    timepoint_ids: &[usize],
    timepoint_labels: &[String],
    terms: &[(&str, usize, usize)],
) -> Result<UnstructuredPrepared, StatsError> {
    let n = x.nrows();
    if y.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
    }
    if cluster_ids.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: cluster_ids.len() }));
    }
    if timepoint_ids.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: timepoint_ids.len() }));
    }
    if !x.all_finite() {
        return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "x", value: f64::NAN }));
    }
    validate_finite_slice(y)?;
    let n_declared = timepoint_labels.len();
    if n_declared == 0 {
        return Err(StatsError::EmptyInput);
    }

    let mut tp_seen = vec![false; n_declared];
    for &tp in timepoint_ids {
        if tp >= n_declared {
            return Err(StatsError::InvalidParameter { name: "timepoint_id", value: tp as f64 });
        }
        tp_seen[tp] = true;
    }
    let mut active_tp_indices = Vec::new();
    let mut active_labels = Vec::new();
    let mut tp_remap = vec![None; n_declared];
    for (orig_tp, &seen) in tp_seen.iter().enumerate() {
        if seen {
            tp_remap[orig_tp] = Some(active_tp_indices.len());
            active_tp_indices.push(orig_tp);
            active_labels.push(timepoint_labels[orig_tp].clone());
        }
    }
    if active_tp_indices.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let (x_red, reduced_terms, col_map, p_eff) = unstructured_reduce_design(x, terms)?;
    if n <= p_eff {
        return Err(StatsError::InsufficientSampleSize { context: "unstructured RM REML", required: p_eff + 1, actual: n });
    }

    let y_moments = SummaryMoments::from_slice(y)?;
    let y_scale_raw = (y_moments.css / (n as f64)).sqrt();
    let y_scale = if y_scale_raw.is_finite() && y_scale_raw > 0.0 { y_scale_raw } else { 1.0 };
    let y_scaled: Vec<f64> = y.iter().map(|&v| v / y_scale).collect();

    let mut grouped = std::collections::BTreeMap::<usize, Vec<usize>>::new();
    for (row_idx, &cid) in cluster_ids.iter().enumerate() {
        grouped.entry(cid).or_default().push(row_idx);
    }
    let mut blocks = Vec::with_capacity(grouped.len());
    for mut rows in grouped.into_values() {
        rows.sort_by_key(|&r| timepoint_ids[r]);
        let m = rows.len();
        let mut tps = Vec::with_capacity(m);
        let mut x_c_data = Vec::with_capacity(m * p_eff);
        let mut y_c = Vec::with_capacity(m);
        for &r in &rows {
            let compact = tp_remap[timepoint_ids[r]].ok_or(StatsError::EmptyInput)?;
            if tps.last() == Some(&compact) {
                return Err(StatsError::InvalidParameter { name: "timepoint_id", value: timepoint_ids[r] as f64 });
            }
            tps.push(compact);
            y_c.push(y_scaled[r]);
            for c in 0..p_eff {
                x_c_data.push(x_red.get(r, c).unwrap_or(0.0));
            }
        }
        let x_c = DenseMatrix::from_row_major(m, p_eff, x_c_data)?;
        blocks.push(UnstructuredClusterBlock { x_c, y_c, tps });
    }

    Ok(UnstructuredPrepared { blocks, reduced_terms, col_map, active_tp_indices, active_labels, p_eff, n_obs: n, y_scale })
}

fn unstructured_eval_state(sigma: &DenseMatrix, blocks: &[UnstructuredClusterBlock], p_eff: usize) -> Result<UnstructuredEval, StatsError> {
    let t_obs = sigma.nrows();
    let mut sum_ln_det_v = 0.0_f64;
    let mut xtvx_data = vec![0.0_f64; p_eff * p_eff];
    let mut xtvy = vec![0.0_f64; p_eff];
    let mut block_cache = Vec::with_capacity(blocks.len());

    for block in blocks {
        let m = block.tps.len();
        let mut v_data = Vec::with_capacity(m * m);
        for &ti in &block.tps {
            for &tj in &block.tps {
                v_data.push(sigma.get(ti, tj).unwrap_or(0.0));
            }
        }
        let chol_v = cholesky(&symmetrize_dense(&DenseMatrix::from_row_major(m, m, v_data)?)?)?;
        sum_ln_det_v += chol_v.ln_det();
        let w_c = chol_v.inverse()?;
        let w_c_x_c = w_c.matmul(&block.x_c)?;
        let w_c_y_c = w_c.matvec(&block.y_c)?;
        for a in 0..m {
            for c1 in 0..p_eff {
                let x_a1 = block.x_c.get(a, c1).unwrap_or(0.0);
                xtvy[c1] += x_a1 * w_c_y_c[a];
                for c2 in c1..p_eff {
                    xtvx_data[c1 * p_eff + c2] += x_a1 * w_c_x_c.get(a, c2).unwrap_or(0.0);
                }
            }
        }
        block_cache.push((w_c, w_c_x_c));
    }
    for c1 in 0..p_eff {
        for c2 in 0..c1 {
            xtvx_data[c1 * p_eff + c2] = xtvx_data[c2 * p_eff + c1];
        }
    }

    let chol_x = cholesky(&symmetrize_dense(&DenseMatrix::from_row_major(p_eff, p_eff, xtvx_data)?)?)?;
    let ln_det_xtvx = chol_x.ln_det();
    let phi = chol_x.inverse()?;
    let beta = chol_x.solve(&xtvy)?;

    let mut quad = 0.0_f64;
    let mut s1 = vec![0.0_f64; t_obs * t_obs];
    let mut s2 = vec![0.0_f64; t_obs * t_obs];
    let mut s3 = vec![0.0_f64; t_obs * t_obs];

    for (block, (w_c, w_c_x_c)) in blocks.iter().zip(block_cache.iter()) {
        let m = block.tps.len();
        let x_beta = block.x_c.matvec(&beta)?;
        let r_c: Vec<f64> = block.y_c.iter().zip(x_beta.iter()).map(|(&yv, &xb)| yv - xb).collect();
        let w_r_c = w_c.matvec(&r_c)?;
        quad += r_c.iter().zip(w_r_c.iter()).map(|(&r, &wr)| r * wr).sum::<f64>();
        let u_phi = w_c_x_c.matmul(&phi)?;

        for a in 0..m {
            let ta = block.tps[a];
            for b in a..m {
                let tb = block.tps[b];
                let w_ab = w_c.get(a, b).unwrap_or(0.0);
                let s3_ab = w_r_c[a] * w_r_c[b];
                let mut s2_ab = 0.0_f64;
                for c_idx in 0..p_eff {
                    s2_ab += u_phi.get(a, c_idx).unwrap_or(0.0) * w_c_x_c.get(b, c_idx).unwrap_or(0.0);
                }
                s1[ta * t_obs + tb] += w_ab;
                s2[ta * t_obs + tb] += s2_ab;
                s3[ta * t_obs + tb] += s3_ab;
                if ta != tb {
                    s1[tb * t_obs + ta] += w_ab;
                    s2[tb * t_obs + ta] += s2_ab;
                    s3[tb * t_obs + ta] += s3_ab;
                }
            }
        }
    }

    let g_data: Vec<f64> = (0..(t_obs * t_obs)).map(|idx| 0.5 * (s3[idx] - s1[idx] + s2[idx])).collect();
    let g_sigma = symmetrize_dense(&DenseMatrix::from_row_major(t_obs, t_obs, g_data)?)?;
    let ll = -0.5 * (sum_ln_det_v + ln_det_xtvx + quad);
    Ok(UnstructuredEval { ll, beta, phi, g_sigma })
}

fn unstructured_ols_pairwise_init(
    blocks: &[UnstructuredClusterBlock],
    p_eff: usize,
    t_obs: usize,
    n_obs: usize,
) -> Result<(DenseMatrix, f64, f64), StatsError> {
    let mut xtx_data = vec![0.0_f64; p_eff * p_eff];
    let mut xty = vec![0.0_f64; p_eff];
    for block in blocks {
        let m = block.tps.len();
        for a in 0..m {
            for c1 in 0..p_eff {
                let x_a1 = block.x_c.get(a, c1).unwrap_or(0.0);
                xty[c1] += x_a1 * block.y_c[a];
                for c2 in c1..p_eff {
                    xtx_data[c1 * p_eff + c2] += x_a1 * block.x_c.get(a, c2).unwrap_or(0.0);
                }
            }
        }
    }
    for c1 in 0..p_eff {
        for c2 in 0..c1 {
            xtx_data[c1 * p_eff + c2] = xtx_data[c2 * p_eff + c1];
        }
    }
    let chol = cholesky(&symmetrize_dense(&DenseMatrix::from_row_major(p_eff, p_eff, xtx_data)?)?)?;
    let beta_ols = chol.solve(&xty)?;

    let mut rss = 0.0_f64;
    let mut sum_prod = vec![0.0_f64; t_obs * t_obs];
    let mut counts = vec![0_usize; t_obs * t_obs];
    let df_ratio = (n_obs as f64) / ((n_obs - p_eff).max(1) as f64);

    for block in blocks {
        let xb = block.x_c.matvec(&beta_ols)?;
        let res: Vec<f64> = block.y_c.iter().zip(xb.iter()).map(|(&yv, &bv)| yv - bv).collect();
        rss += res.iter().map(|&r| r * r).sum::<f64>();
        for (a, &ta) in block.tps.iter().enumerate() {
            for (b, &tb) in block.tps.iter().enumerate() {
                sum_prod[ta * t_obs + tb] += res[a] * res[b];
                counts[ta * t_obs + tb] += 1;
            }
        }
    }
    let ols_var = (rss / ((n_obs - p_eff).max(1) as f64)).max(1e-8);
    let mut cov0 = vec![0.0_f64; t_obs * t_obs];
    for i in 0..t_obs {
        let cnt_ii = counts[i * t_obs + i];
        cov0[i * t_obs + i] = if cnt_ii > 0 { (sum_prod[i * t_obs + i] / (cnt_ii as f64) * df_ratio).max(1e-8) } else { ols_var };
    }
    for i in 0..t_obs {
        for j in (i + 1)..t_obs {
            let cnt = counts[i * t_obs + j];
            if cnt > 0 {
                let raw = sum_prod[i * t_obs + j] / (cnt as f64) * df_ratio;
                let max_off = 0.92 * (cov0[i * t_obs + i] * cov0[j * t_obs + j]).sqrt();
                let clamped = raw.clamp(-max_off, max_off);
                cov0[i * t_obs + j] = clamped;
                cov0[j * t_obs + i] = clamped;
            }
        }
    }
    let raw_mat = symmetrize_dense(&DenseMatrix::from_row_major(t_obs, t_obs, cov0)?)?;
    let (min_ev, max_ev, _) = unstructured_eigen_ratio(&raw_mat)?;
    let scatter_ratio = if max_ev > 1e-30 { min_ev / max_ev } else { 0.0 };
    let (spd_init, _) = unstructured_floor_spd(&raw_mat, 0.02, ols_var)?;
    Ok((spd_init, ols_var, scatter_ratio))
}

fn unstructured_em_warm_start(
    blocks: &[UnstructuredClusterBlock],
    sigma_init: DenseMatrix,
    p_eff: usize,
    ols_var: f64,
    max_iters: usize,
) -> Result<(DenseMatrix, usize), StatsError> {
    let mut sigma = sigma_init;
    let mut st = unstructured_eval_state(&sigma, blocks, p_eff)?;
    let c_scale = 2.0 / (blocks.len().max(1) as f64);
    let mut iters = 0_usize;

    for step_idx in 0..max_iters {
        let grad_norm = st.g_sigma.frobenius_norm();
        if grad_norm < 1e-6 {
            iters = step_idx;
            break;
        }
        let step_mat = scale_dense(&sigma.matmul(&st.g_sigma)?.matmul(&sigma)?, c_scale)?;
        let mut alpha = 1.0_f64;
        let mut accepted = false;
        for _ in 0..8 {
            let scaled_step = scale_dense(&step_mat, alpha)?;
            let cand_raw = symmetrize_dense(&DenseMatrix::from_row_major(
                sigma.nrows(),
                sigma.ncols(),
                sigma.as_slice().iter().zip(scaled_step.as_slice().iter()).map(|(&a, &b)| a + b).collect(),
            )?)?;
            if let Ok((cand_spd, _)) = unstructured_floor_spd(&cand_raw, 1e-7, ols_var)
                && let Ok(cand_st) = unstructured_eval_state(&cand_spd, blocks, p_eff)
                && cand_st.ll >= st.ll - 1e-12
            {
                let improve = (cand_st.ll - st.ll).abs();
                sigma = cand_spd;
                st = cand_st;
                accepted = true;
                if improve <= 1e-12 * (1.0 + st.ll.abs()) {
                    return Ok((sigma, step_idx + 1));
                }
                break;
            }
            alpha *= 0.5;
        }
        iters = step_idx + 1;
        if !accepted {
            break;
        }
    }
    Ok((sigma, iters))
}

fn unstructured_bfgs_optimize(
    blocks: &[UnstructuredClusterBlock],
    sigma_start: DenseMatrix,
    p_eff: usize,
    max_iters: usize,
) -> Result<(DenseMatrix, bool, usize), StatsError> {
    let t_obs = sigma_start.nrows();
    let is_diag = chol_diag_mask(t_obs);
    let mut eta = log_chol_from_sigma(&sigma_start)?;
    let k_dim = eta.len();
    let (l_init, mut sigma) = sigma_from_log_chol(&eta, t_obs)?;
    let mut st = unstructured_eval_state(&sigma, blocks, p_eff)?;
    let mut g = log_chol_neg_grad(&st.g_sigma, &l_init)?;
    let mut h_inv: Vec<f64> = (0..(k_dim * k_dim)).map(|idx| if idx / k_dim == idx % k_dim { 1.0 } else { 0.0 }).collect();

    for iter in 0..max_iters {
        let g_inf = g.iter().copied().fold(0.0_f64, |acc, v| acc.max(v.abs()));
        if g_inf <= 1e-8 {
            return Ok((sigma, true, iter));
        }

        let mut dir = vec![0.0_f64; k_dim];
        for i in 0..k_dim {
            let mut sum = 0.0_f64;
            for j in 0..k_dim {
                sum -= h_inv[i * k_dim + j] * g[j];
            }
            dir[i] = sum;
        }
        let mut g_dot_d: f64 = g.iter().zip(dir.iter()).map(|(&gi, &di)| gi * di).sum();
        if !g_dot_d.is_finite() || g_dot_d >= -1e-16 * g_inf {
            for (idx, val) in h_inv.iter_mut().enumerate() {
                *val = if idx / k_dim == idx % k_dim { 1.0 } else { 0.0 };
            }
            dir = g.iter().map(|&gi| -gi).collect();
            g_dot_d = -g.iter().map(|&gi| gi * gi).sum::<f64>();
        }

        let mut alpha = 1.0_f64;
        let mut accepted = None;
        for _ in 0..24 {
            let mut eta_cand = vec![0.0_f64; k_dim];
            for i in 0..k_dim {
                let val = eta[i] + alpha * dir[i];
                eta_cand[i] = if is_diag[i] { val.clamp(LOG_CHOL_DIAG_MIN, LOG_CHOL_DIAG_MAX) } else { val };
            }
            if let Ok((lc, sig_c)) = sigma_from_log_chol(&eta_cand, t_obs)
                && let Ok(st_c) = unstructured_eval_state(&sig_c, blocks, p_eff)
                && st_c.ll.is_finite()
                && st_c.ll >= st.ll - 1e-4 * alpha * g_dot_d
            {
                accepted = Some((eta_cand, lc, sig_c, st_c));
                break;
            }
            alpha *= 0.5;
        }
        let Some((eta_next, l_next, sigma_next, st_next)) = accepted else {
            return Ok((sigma, g_inf <= 1e-4, iter));
        };

        let s: Vec<f64> = eta_next.iter().zip(eta.iter()).map(|(&a, &b)| a - b).collect();
        let step_inf = s.iter().copied().fold(0.0_f64, |acc, v| acc.max(v.abs()));
        let g_next = log_chol_neg_grad(&st_next.g_sigma, &l_next)?;
        let y_vec: Vec<f64> = g_next.iter().zip(g.iter()).map(|(&a, &b)| a - b).collect();
        let sy: f64 = s.iter().zip(y_vec.iter()).map(|(&si, &yi)| si * yi).sum();
        let yy: f64 = y_vec.iter().map(|&yi| yi * yi).sum();
        let ss: f64 = s.iter().map(|&si| si * si).sum();
        if iter == 0 && sy > 1e-14 && yy > 1e-14 {
            let gamma = sy / yy;
            for (idx, val) in h_inv.iter_mut().enumerate() {
                *val = if idx / k_dim == idx % k_dim { gamma } else { 0.0 };
            }
        }
        if sy > 1e-12 * (ss * yy).sqrt() {
            let rho = 1.0 / sy;
            let mut hy = vec![0.0_f64; k_dim];
            for i in 0..k_dim {
                hy[i] = (0..k_dim).map(|j| h_inv[i * k_dim + j] * y_vec[j]).sum();
            }
            let yhy: f64 = y_vec.iter().zip(hy.iter()).map(|(&yi, &hyi)| yi * hyi).sum();
            let coef_ss = rho + rho * rho * yhy;
            for i in 0..k_dim {
                for j in i..k_dim {
                    let next_ij = h_inv[i * k_dim + j] + coef_ss * s[i] * s[j] - rho * (hy[i] * s[j] + s[i] * hy[j]);
                    h_inv[i * k_dim + j] = next_ij;
                    h_inv[j * k_dim + i] = next_ij;
                }
            }
        }

        eta = eta_next;
        sigma = sigma_next;
        st = st_next;
        g = g_next;

        if step_inf <= 1e-10 && g_inf <= 1e-6 {
            return Ok((sigma, true, iter + 1));
        }
        if is_diag.iter().enumerate().any(|(i, &d)| d && eta[i] <= LOG_CHOL_DIAG_MIN + 1.0) {
            let (min_ev, max_ev, _) = unstructured_eigen_ratio(&sigma)?;
            if max_ev <= 0.0 || min_ev / max_ev <= 1e-6 {
                return Ok((sigma, false, iter + 1));
            }
        }
    }

    let g_inf = g.iter().copied().fold(0.0_f64, |acc, v| acc.max(v.abs()));
    Ok((sigma, g_inf <= 1e-4, max_iters))
}

struct UnstructuredBlockFisher<'a> {
    w_tilde: Vec<f64>,
    u_rows: Vec<Vec<f64>>,
    pair_ab: &'a [(usize, usize)],
}

fn unstructured_accumulate_fisher(
    blk: &UnstructuredBlockFisher<'_>,
    t_obs: usize,
    p_eff: usize,
    m_data: &mut [Vec<f64>],
    q_data: &mut [Vec<f64>],
    tr_ww: &mut [f64],
) {
    let k_dim = blk.pair_ab.len();
    let row_contrib = |k_idx: usize| -> [(usize, usize); 2] {
        let (a, b) = blk.pair_ab[k_idx];
        [(a, b), (b, a)]
    };

    for (k_idx, &(a, b)) in blk.pair_ab.iter().enumerate() {
        let ua = &blk.u_rows[a];
        let ub = &blk.u_rows[b];
        let md = &mut m_data[k_idx];
        if a == b {
            for r in 0..p_eff {
                for c in 0..p_eff {
                    md[r * p_eff + c] += ua[r] * ua[c];
                }
            }
        } else {
            for r in 0..p_eff {
                for c in 0..p_eff {
                    md[r * p_eff + c] += ua[r] * ub[c] + ub[r] * ua[c];
                }
            }
        }

        let pairs_k = row_contrib(k_idx);
        let len_k = if a == b { 1 } else { 2 };
        for l_idx in k_idx..k_dim {
            let (cp, dp) = blk.pair_ab[l_idx];
            let pairs_l = row_contrib(l_idx);
            let len_l = if cp == dp { 1 } else { 2 };
            let qd = &mut q_data[k_idx * k_dim + l_idx];
            let mut tr_acc = 0.0_f64;
            for &(r1, s1) in &pairs_k[..len_k] {
                let v1 = &blk.u_rows[s1];
                for &(r2, s2) in &pairs_l[..len_l] {
                    let w12 = blk.w_tilde[r1 * t_obs + r2];
                    if w12 != 0.0 {
                        let v2 = &blk.u_rows[s2];
                        for r in 0..p_eff {
                            let wr = w12 * v1[r];
                            for c in 0..p_eff {
                                qd[r * p_eff + c] += wr * v2[c];
                            }
                        }
                    }
                    tr_acc += blk.w_tilde[r1 * t_obs + s2] * blk.w_tilde[s1 * t_obs + r2];
                }
            }
            tr_ww[k_idx * k_dim + l_idx] += tr_acc;
        }
    }
}

fn unstructured_build_fisher_mats(
    blocks: &[UnstructuredClusterBlock],
    sigma: &DenseMatrix,
    phi: &DenseMatrix,
    p_eff: usize,
) -> Result<OwnedRemlFisherMats, StatsError> {
    let t_obs = sigma.nrows();
    let k_dim = t_obs * (t_obs + 1) / 2;
    let mut pair_ab = Vec::with_capacity(k_dim);
    for a in 0..t_obs {
        for b in 0..=a {
            pair_ab.push((a, b));
        }
    }
    let mut m_data = vec![vec![0.0_f64; p_eff * p_eff]; k_dim];
    let mut q_data = vec![vec![0.0_f64; p_eff * p_eff]; k_dim * k_dim];
    let mut tr_ww = vec![0.0_f64; k_dim * k_dim];

    for block in blocks {
        let m = block.tps.len();
        let mut v_data = Vec::with_capacity(m * m);
        for &ti in &block.tps {
            for &tj in &block.tps {
                v_data.push(sigma.get(ti, tj).unwrap_or(0.0));
            }
        }
        let chol_v = cholesky(&symmetrize_dense(&DenseMatrix::from_row_major(m, m, v_data)?)?)?;
        let w_c = chol_v.inverse()?;
        let w_c_x_c = w_c.matmul(&block.x_c)?;

        let mut w_tilde = vec![0.0_f64; t_obs * t_obs];
        let mut u_rows = vec![vec![0.0_f64; p_eff]; t_obs];
        for (a, &ta) in block.tps.iter().enumerate() {
            for c in 0..p_eff {
                u_rows[ta][c] = w_c_x_c.get(a, c).unwrap_or(0.0);
            }
            for (b, &tb) in block.tps.iter().enumerate() {
                w_tilde[ta * t_obs + tb] = w_c.get(a, b).unwrap_or(0.0);
            }
        }
        let blk = UnstructuredBlockFisher { w_tilde, u_rows, pair_ab: &pair_ab };
        unstructured_accumulate_fisher(&blk, t_obs, p_eff, &mut m_data, &mut q_data, &mut tr_ww);
    }

    let mut m_mats = Vec::with_capacity(k_dim);
    let mut phi_m = Vec::with_capacity(k_dim);
    for data in m_data {
        let m_mat = symmetrize_dense(&DenseMatrix::from_row_major(p_eff, p_eff, data)?)?;
        phi_m.push(phi.matmul(&m_mat)?);
        m_mats.push(m_mat);
    }

    let mut q_mats = vec![vec![DenseMatrix::zeros(p_eff, p_eff)?; k_dim]; k_dim];
    let mut ie_data = vec![0.0_f64; k_dim * k_dim];
    for r in 0..k_dim {
        for s in r..k_dim {
            let q_rs = DenseMatrix::from_row_major(p_eff, p_eff, std::mem::take(&mut q_data[r * k_dim + s]))?;
            let ie_rs = 0.5 * (tr_ww[r * k_dim + s] - 2.0 * trace_product(phi, &q_rs) + trace_product(&phi_m[r], &phi_m[s]));
            ie_data[r * k_dim + s] = ie_rs;
            ie_data[s * k_dim + r] = ie_rs;
            if r == s {
                q_mats[r][s] = symmetrize_dense(&q_rs)?;
            } else {
                q_mats[s][r] = q_rs.transpose();
                q_mats[r][s] = q_rs;
            }
        }
    }
    let info_e = symmetrize_dense(&DenseMatrix::from_row_major(k_dim, k_dim, ie_data)?)?;
    Ok(OwnedRemlFisherMats { info_e, m_mats, q_mats })
}

fn unstructured_corr_from_cov(cov: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let t = cov.len();
    let mut corr = vec![vec![0.0_f64; t]; t];
    for i in 0..t {
        let sd_i = cov[i][i].max(0.0).sqrt();
        corr[i][i] = if cov[i][i] > 0.0 { 1.0 } else { 0.0 };
        for j in (i + 1)..t {
            let sd_j = cov[j][j].max(0.0).sqrt();
            let val = if sd_i > 0.0 && sd_j > 0.0 { (cov[i][j] / (sd_i * sd_j)).clamp(-1.0, 1.0) } else { 0.0 };
            corr[i][j] = val;
            corr[j][i] = val;
        }
    }
    corr
}

/// Fit an unstructured `t x t` repeated-measures residual covariance matrix
/// (`\Sigma = L L^T`, `t(t + 1)/2` variance-covariance parameters) via log-Cholesky REML
/// (Pinheiro & Bates, 1996; Lindstrom & Bates, 1988) with EM warm-start, BFGS quasi-Newton
/// refinement, and exact Satterthwaite (1946) / Fai & Cornelius (1996) and Kenward & Roger (1997)
/// inference using the linear `vech(\Sigma)` parameterization (`\partial^2 V / \partial\theta_r\partial\theta_s = 0`).
///
/// Engages documented positive-definite diagonal/shrinkage fallbacks for single-subject designs,
/// rank-deficient small samples, all-missing timepoints (compacted from active timepoints alongside
/// rank-reduced fixed effects), or singular covariance boundaries.
pub fn fit_unstructured_rm_reml(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    timepoint_ids: &[usize],
    timepoint_labels: &[String],
    terms: &[(&str, usize, usize)],
) -> Result<UnstructuredRemlResult, StatsError> {
    fit_unstructured_rm_reml_with_core(x, y, cluster_ids, timepoint_ids, timepoint_labels, terms).map(|(res, _)| res)
}

fn fit_unstructured_rm_reml_with_core(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    timepoint_ids: &[usize],
    timepoint_labels: &[String],
    terms: &[(&str, usize, usize)],
) -> Result<(UnstructuredRemlResult, RemlContrastCore), StatsError> {
    let prep = unstructured_prepare_clusters(x, y, cluster_ids, timepoint_ids, timepoint_labels, terms)?;
    let t_obs = prep.active_tp_indices.len();
    let n_clusters = prep.blocks.len();
    let n_minus_p = (prep.n_obs - prep.p_eff) as f64;
    let k_params = t_obs * (t_obs + 1) / 2;

    let (sigma_init, ols_var, scatter_ratio) = unstructured_ols_pairwise_init(&prep.blocks, prep.p_eff, t_obs, prep.n_obs)?;
    let (sigma_scaled, converged, iterations, fallback_reason) = if n_clusters == 1 {
        let diag = scale_dense(&DenseMatrix::identity(t_obs)?, ols_var.max(1e-8))?;
        (diag, true, 0, Some(UnstructuredFallbackReason::SingleSubject))
    } else if n_clusters < t_obs || (prep.n_obs - prep.p_eff) < k_params {
        let shr = unstructured_shrink_spd(&sigma_init, 1e-3, ols_var)?;
        (shr, true, 0, Some(UnstructuredFallbackReason::SmallSampleRankDeficient))
    } else {
        let (em_sig, em_iters) = unstructured_em_warm_start(&prep.blocks, sigma_init, prep.p_eff, ols_var, 20)?;
        let (bfgs_sig, bfgs_conv, bfgs_iters) = unstructured_bfgs_optimize(&prep.blocks, em_sig, prep.p_eff, 220)?;
        let total_iters = em_iters + bfgs_iters;
        let (min_ev, max_ev, _) = unstructured_eigen_ratio(&bfgs_sig)?;
        let cov_ratio = if max_ev > 1e-30 { min_ev / max_ev } else { 0.0 };
        if cov_ratio <= 1e-6 || scatter_ratio <= 1e-8 {
            let reason = if n_clusters <= t_obs || (prep.n_obs - prep.p_eff) <= k_params {
                UnstructuredFallbackReason::SmallSampleRankDeficient
            } else {
                UnstructuredFallbackReason::SingularCovarianceEstimate
            };
            let shr = unstructured_shrink_spd(&bfgs_sig, 1e-3, ols_var)?;
            (shr, bfgs_conv, total_iters, Some(reason))
        } else if !bfgs_conv {
            (bfgs_sig, false, total_iters, Some(UnstructuredFallbackReason::IterationLimitReached))
        } else {
            (bfgs_sig, true, total_iters, None)
        }
    };

    let st_final = unstructured_eval_state(&sigma_scaled, &prep.blocks, prep.p_eff)?;
    let fisher = unstructured_build_fisher_mats(&prep.blocks, &sigma_scaled, &st_final.phi, prep.p_eff)?;
    let term_refs: Vec<(&str, usize, usize)> = prep.reduced_terms.iter().map(|(n, s, l)| (n.as_str(), *s, *l)).collect();
    let mean_diag_scaled: f64 = (0..t_obs).map(|i| sigma_scaled.get(i, i).unwrap_or(1.0)).sum::<f64>() / (t_obs as f64);

    let ((mut fixed_terms, satterthwaite_df, kenward_roger_df), mut core) =
        eval_reml_wald_terms_with_core(&st_final.phi, &st_final.beta, mean_diag_scaled, &fisher, &term_refs, n_minus_p)?;
    core.y_scale = prep.y_scale;
    core.col_map = prep.col_map;

    let var_scale = prep.y_scale * prep.y_scale;
    let mean_diagonal_variance = mean_diag_scaled * var_scale;
    for term in &mut fixed_terms {
        if let Some(f_val) = term.f_statistic {
            let ms = f_val * mean_diagonal_variance;
            term.ms = ms;
            term.ss = ms * term.df;
        }
    }

    let mut covariance_matrix = vec![vec![0.0_f64; t_obs]; t_obs];
    for (i, row) in covariance_matrix.iter_mut().enumerate() {
        for (j, slot) in row.iter_mut().enumerate() {
            *slot = sigma_scaled.get(i, j).unwrap_or(0.0) * var_scale;
        }
    }
    let correlation_matrix = unstructured_corr_from_cov(&covariance_matrix);
    let reml_log_lik = st_final.ll - n_minus_p * prep.y_scale.ln();

    let res = UnstructuredRemlResult {
        timepoint_labels: prep.active_labels,
        active_timepoint_indices: prep.active_tp_indices,
        covariance_matrix,
        correlation_matrix,
        mean_diagonal_variance,
        reml_log_lik,
        fixed_terms,
        satterthwaite_df,
        kenward_roger_df,
        converged,
        iterations,
        fallback_reason,
    };
    Ok((res, core))
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
pub fn nested_one_way_anova(table: &curvebook_doc::NestedTable, two_group_t_test: bool) -> Result<NestedAnovaResult, StatsError> {
    let selected: Vec<usize> = (0..table.groups.len()).collect();
    nested_one_way_anova_selected(table, &selected, two_group_t_test)
}

fn nested_one_way_anova_selected(
    table: &curvebook_doc::NestedTable,
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
            let details = compare_groups_with_confidence(
                &group_inputs,
                base.ms_within,
                base.df_within,
                proc_eff,
                anova_spec.multiplicity.threshold,
                anova_spec.multiplicity.report_confidence_intervals,
                confidence,
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
            let mauchly_p_lbl = if rm.sphericity.mauchly_p.is_finite() { format!("{:.4}", rm.sphericity.mauchly_p) } else { String::from("n/a") };
            out.checklist.push(checklist_item(
                "Sphericity (Mauchly / Greenhouse-Geisser)",
                sph_status,
                Some(format!(
                    "eps_GG = {:.4}, eps_HF = {:.4}, Mauchly P = {mauchly_p_lbl}",
                    rm.sphericity.greenhouse_geisser_eps, rm.sphericity.huynh_feldt_eps
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
    let details = compare_groups_pairwise_with_confidence(inputs, ms_within, df_within, mult, confidence)?;
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
    push_pairwise_outputs_with_footnote(out, details, tab_id, title, p_fmt, confidence, None);
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

    let (mut result, obs, post_hoc, aux_tabs) = match &sheet.archetype {
        TableArchetype::Grouped(grouped) => {
            let outcome = run_grouped_factorial(grouped, &mut out, &spec.selected_data_sets, fac_spec, p_fmt)?;
            (outcome.result, outcome.observations, outcome.post_hoc, outcome.aux_tabs)
        }
        TableArchetype::MultipleVariables(mv) => {
            let (res, obs, model) = run_multifactor_mv(mv, fac_spec)?;
            (res, obs, FactorialPostHocContext::Ols(model), Vec::new())
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

    run_factorial_multiplicity(&mut out, &obs, &post_hoc, &result, &fac_spec.multiplicity, p_fmt, confidence)?;

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

#[derive(Debug, Clone)]
struct RemlContrastModel {
    n_rows: usize,
    n_cols: usize,
    include_interaction: bool,
    row_basis: Vec<Vec<f64>>,
    col_basis: Vec<Vec<f64>>,
    row_proj: DenseMatrix,
    col_map: Vec<Option<usize>>,
    beta: Vec<f64>,
    phi: DenseMatrix,
    phi_a: DenseMatrix,
    w_info_inv: DenseMatrix,
    phi_m_phi: Vec<DenseMatrix>,
    n_minus_p: f64,
    y_scale: f64,
    observed_cells: Vec<Vec<bool>>,
}

#[derive(Debug, Clone)]
enum FactorialPostHocContext {
    Ols(OlsPostHocModel),
    RmBalanced(RmBalancedPostHocModel),
    Reml(RemlContrastModel),
}

struct ContrastEstimate {
    diff: f64,
    se: f64,
    df: f64,
    ctrl_var_pair: Option<(f64, f64)>,
    cov_vectors: Option<(Vec<f64>, Vec<f64>)>,
}

#[derive(Clone, Copy)]
struct NamedSliceContrast<'a> {
    id_a: usize,
    id_b: usize,
    label_a: &'a str,
    label_b: &'a str,
    diff: f64,
    se: f64,
    df: f64,
}

#[inline]
fn dot_slice(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum()
}

fn quad_form(v: &[f64], mat: &DenseMatrix) -> f64 {
    let n = v.len();
    let mut acc = 0.0_f64;
    for (r, &vr) in v.iter().enumerate() {
        if vr == 0.0 {
            continue;
        }
        let mut row_dot = 0.0_f64;
        for (c, &vc) in v.iter().enumerate() {
            if vc != 0.0 {
                row_dot += mat.get(r, c).unwrap_or(0.0) * vc;
            }
        }
        acc += vr * row_dot;
    }
    let _ = n;
    acc
}

fn is_vec_estimable(row_proj: &DenseMatrix, v: &[f64]) -> Result<bool, StatsError> {
    let norm_sq = dot_slice(v, v);
    if norm_sq <= 0.0 {
        return Ok(true);
    }
    let pv = row_proj.matvec(v)?;
    let err_sq: f64 = v.iter().zip(pv.iter()).map(|(&a, &b)| (a - b) * (a - b)).sum();
    Ok(err_sq <= 1e-12 * norm_sq.max(1.0))
}

fn satterthwaite_two_term_df(t1_ms: f64, df1: f64, t2_ms: f64, df2: f64) -> f64 {
    let num = t1_ms + t2_ms;
    let den = (t1_ms * t1_ms) / df1.max(1.0) + (t2_ms * t2_ms) / df2.max(1.0);
    if den > 0.0 { (num * num / den).max(1.0) } else { df1.max(df2).max(1.0) }
}

fn welch_stats_from_weighted_terms(terms: &[(f64, f64)]) -> (f64, f64) {
    let v_sum: f64 = terms.iter().map(|&(v, _)| v.max(0.0)).sum();
    if v_sum <= 0.0 {
        let df_fallback = terms.iter().map(|&(_, df)| df).fold(0.0_f64, f64::max).max(1.0);
        return (0.0, df_fallback);
    }
    let denom: f64 = terms.iter().filter(|&&(v, _)| v > 0.0).map(|&(v, df)| (v * v) / df.max(1.0)).sum();
    let df = if denom > 0.0 { (v_sum * v_sum / denom).max(1.0) } else { 1.0 };
    (v_sum.sqrt(), df)
}

fn welch_two_slice_stats(a: &[f64], b: &[f64]) -> Result<(f64, f64, f64), StatsError> {
    let ma = SummaryMoments::from_slice(a)?;
    let mb = SummaryMoments::from_slice(b)?;
    let va = if a.len() >= 2 { (ma.css / ((a.len() - 1) as f64)).max(0.0) / (a.len() as f64) } else { 0.0 };
    let vb = if b.len() >= 2 { (mb.css / ((b.len() - 1) as f64)).max(0.0) / (b.len() as f64) } else { 0.0 };
    let dfa = (a.len().saturating_sub(1)).max(1) as f64;
    let dfb = (b.len().saturating_sub(1)).max(1) as f64;
    let (se, df) = welch_stats_from_weighted_terms(&[(va, dfa), (vb, dfb)]);
    Ok((ma.mean - mb.mean, se, df))
}

fn paired_diff_stats(diffs: &[f64]) -> Result<(f64, f64, f64), StatsError> {
    let m = SummaryMoments::from_slice(diffs)?;
    let n = diffs.len();
    let df = (n.saturating_sub(1)).max(1) as f64;
    let se = if n >= 2 { ((m.css / df).max(0.0) / (n as f64)).sqrt() } else { 0.0 };
    Ok((m.mean, se, df))
}

fn enumerate_cartesian(radices: &[usize]) -> Result<Vec<Vec<usize>>, StatsError> {
    let mut total = 1usize;
    for &r in radices {
        total = total.saturating_mul(r.max(1));
        if total > 8192 {
            return Err(StatsError::InvalidParameter { name: "factorial_levels", value: total as f64 });
        }
    }
    let mut out = vec![Vec::with_capacity(radices.len())];
    for &r in radices {
        let mut next = Vec::with_capacity(out.len() * r.max(1));
        for prefix in &out {
            for lvl in 0..r.max(1) {
                let mut row = prefix.clone();
                row.push(lvl);
                next.push(row);
            }
        }
        out = next;
    }
    Ok(out)
}

impl OlsPostHocModel {
    fn design_vector(&self, cell: &[usize]) -> Vec<f64> {
        let mut row = Vec::with_capacity(self.total_cols);
        row.push(1.0);
        for sub in &self.subsets {
            let mut prod = vec![1.0_f64];
            for &f_idx in sub {
                let b_row = &self.factor_basis[f_idx][cell[f_idx]];
                if b_row.is_empty() {
                    prod.clear();
                    break;
                }
                let mut next = Vec::with_capacity(prod.len() * b_row.len());
                for &pv in &prod {
                    for &bv in b_row {
                        next.push(pv * bv);
                    }
                }
                prod = next;
            }
            row.extend(prod);
        }
        row
    }

    fn avg_design_over(&self, other_factors: &[usize], tuples: &[Vec<usize>], fixed: &[(usize, usize)]) -> Vec<f64> {
        let n_factors = self.factor_levels.len();
        let mut sum_vec = vec![0.0_f64; self.total_cols];
        let mut cell = vec![0_usize; n_factors];
        for &(f, lvl) in fixed {
            cell[f] = lvl;
        }
        for tup in tuples {
            for (&f, &v) in other_factors.iter().zip(tup.iter()) {
                cell[f] = v;
            }
            let dv = self.design_vector(&cell);
            for (dst, &val) in sum_vec.iter_mut().zip(dv.iter()) {
                *dst += val;
            }
        }
        let inv = 1.0 / (tuples.len().max(1) as f64);
        for dst in &mut sum_vec {
            *dst *= inv;
        }
        sum_vec
    }

    fn pair_cells_exist(&self, other_factors: &[usize], tup: &[usize], fixed_a: &[(usize, usize)], fixed_b: &[(usize, usize)]) -> bool {
        let mut cell = vec![0_usize; self.factor_levels.len()];
        for (&f, &v) in other_factors.iter().zip(tup.iter()) {
            cell[f] = v;
        }
        for &(f, lvl) in fixed_a {
            cell[f] = lvl;
        }
        if !self.cell_stats.contains_key(&cell) {
            return false;
        }
        for &(f, lvl) in fixed_b {
            cell[f] = lvl;
        }
        self.cell_stats.contains_key(&cell)
    }

    fn eval_contrast_from_tuples(
        &self,
        other_factors: &[usize],
        tuples: &[Vec<usize>],
        fixed_a: &[(usize, usize)],
        fixed_b: &[(usize, usize)],
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        let shared: Vec<Vec<usize>> = tuples.iter().filter(|t| self.pair_cells_exist(other_factors, t, fixed_a, fixed_b)).cloned().collect();
        let all_a = self.avg_design_over(other_factors, tuples, fixed_a);
        let all_b = self.avg_design_over(other_factors, tuples, fixed_b);
        let all_l: Vec<f64> = all_a.iter().zip(all_b.iter()).map(|(&a, &b)| a - b).collect();

        let (chosen, w_a, w_b, l) = if is_vec_estimable(&self.row_proj, &all_l)? {
            let use_tups = if shared.len() == tuples.len() {
                tuples
            } else if !shared.is_empty() && unequal_var {
                shared.as_slice()
            } else {
                tuples
            };
            let wa = self.avg_design_over(other_factors, use_tups, fixed_a);
            let wb = self.avg_design_over(other_factors, use_tups, fixed_b);
            let lv: Vec<f64> = wa.iter().zip(wb.iter()).map(|(&a, &b)| a - b).collect();
            (use_tups, wa, wb, lv)
        } else if !shared.is_empty() {
            let wa = self.avg_design_over(other_factors, &shared, fixed_a);
            let wb = self.avg_design_over(other_factors, &shared, fixed_b);
            let lv: Vec<f64> = wa.iter().zip(wb.iter()).map(|(&a, &b)| a - b).collect();
            if !is_vec_estimable(&self.row_proj, &lv)? {
                return Ok(None);
            }
            (shared.as_slice(), wa, wb, lv)
        } else {
            return Ok(None);
        };

        let diff = dot_slice(&l, &self.beta);
        if unequal_var && chosen.iter().all(|t| self.pair_cells_exist(other_factors, t, fixed_a, fixed_b)) {
            let w = 1.0 / (chosen.len() as f64);
            let w2 = w * w;
            let mut terms = Vec::with_capacity(chosen.len() * 2);
            let mut cell = vec![0_usize; self.factor_levels.len()];
            for tup in chosen {
                for (&f, &v) in other_factors.iter().zip(tup.iter()) {
                    cell[f] = v;
                }
                for fixed in [fixed_a, fixed_b] {
                    for &(f, lvl) in fixed {
                        cell[f] = lvl;
                    }
                    if let Some(st) = self.cell_stats.get(&cell) {
                        let (v_c, df_c) =
                            if st.count >= 2 { (st.var_unbiased / (st.count as f64), (st.count - 1) as f64) } else { (self.ms_resid, self.df_resid) };
                        terms.push((w2 * v_c, df_c));
                    }
                }
            }
            let (se, df) = welch_stats_from_weighted_terms(&terms);
            return Ok(Some(ContrastEstimate { diff, se, df, ctrl_var_pair: None, cov_vectors: None }));
        }

        let cov_l_raw = self.cov_unscaled.matvec(&l)?;
        let cov_l: Vec<f64> = cov_l_raw.into_iter().map(|v| v * self.ms_resid).collect();
        let se = dot_slice(&l, &cov_l).max(0.0).sqrt();
        let ctrl_var_pair = if is_vec_estimable(&self.row_proj, &w_a)? && is_vec_estimable(&self.row_proj, &w_b)? {
            Some(((self.ms_resid * quad_form(&w_a, &self.cov_unscaled)).max(0.0), (self.ms_resid * quad_form(&w_b, &self.cov_unscaled)).max(0.0)))
        } else {
            None
        };
        Ok(Some(ContrastEstimate { diff, se, df: self.df_resid, ctrl_var_pair, cov_vectors: Some((l, cov_l)) }))
    }

    fn eval_marginal_pair(&self, f_idx: usize, lvl_a: usize, lvl_b: usize, unequal_var: bool) -> Result<Option<ContrastEstimate>, StatsError> {
        let other_factors: Vec<usize> = (0..self.factor_levels.len()).filter(|&f| f != f_idx).collect();
        let radices: Vec<usize> = other_factors.iter().map(|&f| self.factor_levels[f].len()).collect();
        let tuples = enumerate_cartesian(&radices)?;
        self.eval_contrast_from_tuples(&other_factors, &tuples, &[(f_idx, lvl_a)], &[(f_idx, lvl_b)], unequal_var)
    }

    fn eval_simple_pair(
        &self,
        cond_f: usize,
        cond_lvl: usize,
        tgt_f: usize,
        t_a: usize,
        t_b: usize,
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        let other_factors: Vec<usize> = (0..self.factor_levels.len()).filter(|&f| f != cond_f && f != tgt_f).collect();
        let radices: Vec<usize> = other_factors.iter().map(|&f| self.factor_levels[f].len()).collect();
        let tuples = enumerate_cartesian(&radices)?;
        let has_shared =
            tuples.iter().any(|t| self.pair_cells_exist(&other_factors, t, &[(cond_f, cond_lvl), (tgt_f, t_a)], &[(cond_f, cond_lvl), (tgt_f, t_b)]));
        if !has_shared {
            return Ok(None);
        }
        self.eval_contrast_from_tuples(&other_factors, &tuples, &[(cond_f, cond_lvl), (tgt_f, t_a)], &[(cond_f, cond_lvl), (tgt_f, t_b)], unequal_var)
    }

    fn level_mean_estimate(&self, f_idx: usize, lvl: usize) -> Result<Option<f64>, StatsError> {
        let other_factors: Vec<usize> = (0..self.factor_levels.len()).filter(|&f| f != f_idx).collect();
        let radices: Vec<usize> = other_factors.iter().map(|&f| self.factor_levels[f].len()).collect();
        let tuples = enumerate_cartesian(&radices)?;
        let w = self.avg_design_over(&other_factors, &tuples, &[(f_idx, lvl)]);
        if is_vec_estimable(&self.row_proj, &w)? { Ok(Some(dot_slice(&w, &self.beta))) } else { Ok(None) }
    }
}

impl RmBalancedPostHocModel {
    fn subject_marginal_values(&self, factor_idx: usize, lvl: usize) -> Vec<f64> {
        let s = self.s_count;
        if factor_idx == 0 {
            let inv_b = 1.0 / (self.b_count as f64);
            (0..s).map(|si| self.subject_data[si][lvl].iter().sum::<f64>() * inv_b).collect()
        } else {
            let inv_a = 1.0 / (self.a_count as f64);
            (0..s).map(|si| (0..self.a_count).map(|ai| self.subject_data[si][ai][lvl]).sum::<f64>() * inv_a).collect()
        }
    }

    fn eval_marginal_pair(&self, f_idx: usize, lvl_a: usize, lvl_b: usize, unequal_var: bool) -> Result<Option<ContrastEstimate>, StatsError> {
        let sf = self.s_count as f64;
        let diff = if f_idx == 0 { self.row_means[lvl_a] - self.row_means[lvl_b] } else { self.col_means[lvl_a] - self.col_means[lvl_b] };
        if !unequal_var {
            let (ms, df, n_eff) = if f_idx == 0 {
                (self.err_a_ms, self.err_a_df, sf * (self.b_count as f64))
            } else {
                (self.err_b_ms, self.err_b_df, sf * (self.a_count as f64))
            };
            let v_lvl = (ms / n_eff).max(0.0);
            return Ok(Some(ContrastEstimate { diff, se: (2.0 * v_lvl).sqrt(), df, ctrl_var_pair: Some((v_lvl, v_lvl)), cov_vectors: None }));
        }

        let is_between =
            matches!((self.design, f_idx), (RepeatedMeasuresDesign::ColumnFactorMatched, 0) | (RepeatedMeasuresDesign::RowFactorMatched, 1));
        if is_between {
            let va = self.subject_marginal_values(f_idx, lvl_a);
            let vb = self.subject_marginal_values(f_idx, lvl_b);
            let (d, se, df) = welch_two_slice_stats(&va, &vb)?;
            return Ok(Some(ContrastEstimate { diff: d, se, df, ctrl_var_pair: None, cov_vectors: None }));
        }

        if self.design == RepeatedMeasuresDesign::BothFactorsMatched {
            let va = self.subject_marginal_values(f_idx, lvl_a);
            let vb = self.subject_marginal_values(f_idx, lvl_b);
            let diffs: Vec<f64> = va.into_iter().zip(vb).map(|(x, y)| x - y).collect();
            let (d, se, df) = paired_diff_stats(&diffs)?;
            return Ok(Some(ContrastEstimate { diff: d, se, df, ctrl_var_pair: None, cov_vectors: None }));
        }

        // Mixed design within-subject factor: paired diffs stratified by the between-subject factor.
        let n_between = if f_idx == 1 { self.a_count } else { self.b_count };
        let inv_g = 1.0 / (n_between as f64);
        let w2 = inv_g * inv_g;
        let mut terms = Vec::with_capacity(n_between);
        for g in 0..n_between {
            let diffs: Vec<f64> = (0..self.s_count)
                .map(|si| {
                    if f_idx == 1 {
                        self.subject_data[si][g][lvl_a] - self.subject_data[si][g][lvl_b]
                    } else {
                        self.subject_data[si][lvl_a][g] - self.subject_data[si][lvl_b][g]
                    }
                })
                .collect();
            let (_, g_se, g_df) = paired_diff_stats(&diffs)?;
            terms.push((w2 * g_se * g_se, g_df));
        }
        let (se, df) = welch_stats_from_weighted_terms(&terms);
        Ok(Some(ContrastEstimate { diff, se, df, ctrl_var_pair: None, cov_vectors: None }))
    }

    fn eval_simple_pair(
        &self,
        cond_f: usize,
        cond_lvl: usize,
        tgt_f: usize,
        t_a: usize,
        t_b: usize,
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        let _ = cond_f;
        let cell = |tgt: usize| -> (usize, usize) { if tgt_f == 0 { (tgt, cond_lvl) } else { (cond_lvl, tgt) } };
        let (ra, ca) = cell(t_a);
        let (rb, cb) = cell(t_b);
        let diff = self.cell_means[ra][ca] - self.cell_means[rb][cb];
        let sf = self.s_count as f64;

        if !unequal_var {
            let is_pure_within =
                matches!((self.design, tgt_f), (RepeatedMeasuresDesign::ColumnFactorMatched, 1) | (RepeatedMeasuresDesign::RowFactorMatched, 0));
            let (ms_pool, df_pool) = if is_pure_within {
                (self.err_ab_ms, self.err_ab_df)
            } else {
                let (err_t_ms, err_t_df, n_other) = if tgt_f == 0 {
                    (self.err_a_ms, self.err_a_df, self.b_count as f64)
                } else {
                    (self.err_b_ms, self.err_b_df, self.a_count as f64)
                };
                let ms_num = err_t_ms + (n_other - 1.0) * self.err_ab_ms;
                let df = satterthwaite_two_term_df(err_t_ms, err_t_df, (n_other - 1.0) * self.err_ab_ms, self.err_ab_df);
                (ms_num / n_other, df)
            };
            let v_lvl = (ms_pool / sf).max(0.0);
            return Ok(Some(ContrastEstimate {
                diff,
                se: (2.0 * v_lvl).sqrt(),
                df: df_pool,
                ctrl_var_pair: Some((v_lvl, v_lvl)),
                cov_vectors: None,
            }));
        }

        let is_between =
            matches!((self.design, tgt_f), (RepeatedMeasuresDesign::ColumnFactorMatched, 0) | (RepeatedMeasuresDesign::RowFactorMatched, 1));
        let va: Vec<f64> = (0..self.s_count).map(|si| self.subject_data[si][ra][ca]).collect();
        let vb: Vec<f64> = (0..self.s_count).map(|si| self.subject_data[si][rb][cb]).collect();
        if is_between {
            let (d, se, df) = welch_two_slice_stats(&va, &vb)?;
            Ok(Some(ContrastEstimate { diff: d, se, df, ctrl_var_pair: None, cov_vectors: None }))
        } else {
            let diffs: Vec<f64> = va.into_iter().zip(vb).map(|(x, y)| x - y).collect();
            let (d, se, df) = paired_diff_stats(&diffs)?;
            Ok(Some(ContrastEstimate { diff: d, se, df, ctrl_var_pair: None, cov_vectors: None }))
        }
    }
}

impl RemlContrastModel {
    fn cell_design_vector(&self, r: usize, c: usize) -> Vec<f64> {
        let n_r = self.n_rows.saturating_sub(1);
        let n_c = self.n_cols.saturating_sub(1);
        let inter_df = if self.include_interaction { n_r * n_c } else { 0 };
        let mut v = Vec::with_capacity(1 + n_r + n_c + inter_df);
        v.push(1.0);
        v.extend_from_slice(&self.row_basis[r]);
        v.extend_from_slice(&self.col_basis[c]);
        if self.include_interaction {
            for &rb in &self.row_basis[r] {
                for &cb in &self.col_basis[c] {
                    v.push(rb * cb);
                }
            }
        }
        v
    }

    fn map_to_eff(&self, v_orig: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0_f64; self.beta.len()];
        for (&src, opt_dst) in v_orig.iter().zip(self.col_map.iter()) {
            if let Some(dst) = *opt_dst {
                out[dst] = src;
            }
        }
        out
    }

    fn eval_weight_vecs(&self, wa_orig: &[f64], wb_orig: &[f64]) -> Result<Option<ContrastEstimate>, StatsError> {
        let l_orig: Vec<f64> = wa_orig.iter().zip(wb_orig.iter()).map(|(&a, &b)| a - b).collect();
        if !is_vec_estimable(&self.row_proj, &l_orig)? {
            return Ok(None);
        }
        let l = self.map_to_eff(&l_orig);
        let diff = dot_slice(&l, &self.beta) * self.y_scale;
        let v = quad_form(&l, &self.phi).max(0.0);
        let v_a = quad_form(&l, &self.phi_a).max(0.0);
        let se = v_a.sqrt() * self.y_scale;
        let grad: Vec<f64> = self.phi_m_phi.iter().map(|pmp| quad_form(&l, pmp)).collect();
        let wg = self.w_info_inv.matvec(&grad)?;
        let denom = dot_slice(&grad, &wg);
        let df = if v > 0.0 && denom > 1e-30 * v * v { (2.0 * v * v / denom).clamp(1.0, self.n_minus_p) } else { self.n_minus_p };

        let y2 = self.y_scale * self.y_scale;
        let phi_a_l = self.phi_a.matvec(&l)?;
        let cov_l: Vec<f64> = phi_a_l.into_iter().map(|val| val * y2).collect();
        let ctrl_var_pair = if is_vec_estimable(&self.row_proj, wa_orig)? && is_vec_estimable(&self.row_proj, wb_orig)? {
            let wa = self.map_to_eff(wa_orig);
            let wb = self.map_to_eff(wb_orig);
            Some(((quad_form(&wa, &self.phi_a) * y2).max(0.0), (quad_form(&wb, &self.phi_a) * y2).max(0.0)))
        } else {
            None
        };
        Ok(Some(ContrastEstimate { diff, se, df, ctrl_var_pair, cov_vectors: Some((l, cov_l)) }))
    }

    fn avg_cell_vec(&self, f_idx: usize, lvl: usize, other_indices: &[usize]) -> Vec<f64> {
        let p = self.row_proj.nrows();
        let mut acc = vec![0.0_f64; p];
        for &o in other_indices {
            let (r, c) = if f_idx == 0 { (lvl, o) } else { (o, lvl) };
            let dv = self.cell_design_vector(r, c);
            for (dst, &val) in acc.iter_mut().zip(dv.iter()) {
                *dst += val;
            }
        }
        let inv = 1.0 / (other_indices.len().max(1) as f64);
        for dst in &mut acc {
            *dst *= inv;
        }
        acc
    }

    fn eval_marginal_pair(&self, f_idx: usize, lvl_a: usize, lvl_b: usize) -> Result<Option<ContrastEstimate>, StatsError> {
        let n_other = if f_idx == 0 { self.n_cols } else { self.n_rows };
        let all_other: Vec<usize> = (0..n_other).collect();
        let wa_all = self.avg_cell_vec(f_idx, lvl_a, &all_other);
        let wb_all = self.avg_cell_vec(f_idx, lvl_b, &all_other);
        if let Some(est) = self.eval_weight_vecs(&wa_all, &wb_all)? {
            return Ok(Some(est));
        }
        let shared: Vec<usize> = all_other
            .into_iter()
            .filter(|&o| {
                let (ra, ca) = if f_idx == 0 { (lvl_a, o) } else { (o, lvl_a) };
                let (rb, cb) = if f_idx == 0 { (lvl_b, o) } else { (o, lvl_b) };
                self.observed_cells[ra][ca] && self.observed_cells[rb][cb]
            })
            .collect();
        if shared.is_empty() {
            return Ok(None);
        }
        let wa = self.avg_cell_vec(f_idx, lvl_a, &shared);
        let wb = self.avg_cell_vec(f_idx, lvl_b, &shared);
        self.eval_weight_vecs(&wa, &wb)
    }

    fn eval_simple_pair(&self, cond_lvl: usize, tgt_f: usize, t_a: usize, t_b: usize) -> Result<Option<ContrastEstimate>, StatsError> {
        let (ra, ca) = if tgt_f == 0 { (t_a, cond_lvl) } else { (cond_lvl, t_a) };
        let (rb, cb) = if tgt_f == 0 { (t_b, cond_lvl) } else { (cond_lvl, t_b) };
        if !self.observed_cells[ra][ca] || !self.observed_cells[rb][cb] {
            return Ok(None);
        }
        let wa = self.cell_design_vector(ra, ca);
        let wb = self.cell_design_vector(rb, cb);
        self.eval_weight_vecs(&wa, &wb)
    }
}

impl FactorialPostHocContext {
    fn eval_marginal_pair(&self, f_idx: usize, lvl_a: usize, lvl_b: usize, unequal_var: bool) -> Result<Option<ContrastEstimate>, StatsError> {
        match self {
            Self::Ols(m) => m.eval_marginal_pair(f_idx, lvl_a, lvl_b, unequal_var),
            Self::RmBalanced(m) => m.eval_marginal_pair(f_idx, lvl_a, lvl_b, unequal_var),
            Self::Reml(m) => m.eval_marginal_pair(f_idx, lvl_a, lvl_b),
        }
    }

    fn eval_simple_pair(
        &self,
        cond_f: usize,
        cond_lvl: usize,
        tgt_f: usize,
        t_a: usize,
        t_b: usize,
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        match self {
            Self::Ols(m) => m.eval_simple_pair(cond_f, cond_lvl, tgt_f, t_a, t_b, unequal_var),
            Self::RmBalanced(m) => m.eval_simple_pair(cond_f, cond_lvl, tgt_f, t_a, t_b, unequal_var),
            Self::Reml(m) => m.eval_simple_pair(cond_lvl, tgt_f, t_a, t_b),
        }
    }

    fn level_mean_estimate(&self, f_idx: usize, lvl: usize) -> Result<Option<f64>, StatsError> {
        match self {
            Self::Ols(m) => m.level_mean_estimate(f_idx, lvl),
            Self::RmBalanced(m) => Ok(Some(if f_idx == 0 { m.row_means[lvl] } else { m.col_means[lvl] })),
            Self::Reml(m) => {
                let n_other = if f_idx == 0 { m.n_cols } else { m.n_rows };
                let other: Vec<usize> = (0..n_other).collect();
                let w_orig = m.avg_cell_vec(f_idx, lvl, &other);
                if is_vec_estimable(&m.row_proj, &w_orig)? {
                    let w = m.map_to_eff(&w_orig);
                    Ok(Some(dot_slice(&w, &m.beta) * m.y_scale))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

fn push_pairwise_outputs_with_footnote(
    out: &mut ResultSheet,
    details: &[PairwiseComparisonDetail],
    tab_id: &str,
    title: &str,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
    extra_footnote: Option<String>,
) {
    if details.is_empty() {
        return;
    }
    let mut tab = build_multiplicity_tab(details, title, p_fmt, confidence);
    tab.id = tab_id.to_owned();
    if let Some(note) = extra_footnote {
        tab.footnotes.push(note);
    }
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

#[allow(clippy::too_many_arguments)]
fn build_pair_detail(
    group_a: usize,
    group_b: usize,
    label_a: String,
    label_b: String,
    diff: f64,
    se_diff: f64,
    df: f64,
    statistic: f64,
    raw_p: f64,
    adjusted_p_or_q: f64,
    is_fdr_q: bool,
    ci: Option<(f64, f64)>,
    threshold: f64,
) -> PairwiseComparisonDetail {
    PairwiseComparisonDetail {
        group_a,
        group_b,
        label_a,
        label_b,
        mean_diff: diff,
        se_diff,
        statistic,
        df,
        raw_p,
        adjusted_p_or_q,
        is_fdr_q,
        ci,
        significant: threshold.is_finite() && adjusted_p_or_q.is_finite() && adjusted_p_or_q <= threshold,
    }
}

fn zero_se_pair(diff: f64, unsigned: bool) -> (f64, f64) {
    if diff != 0.0 { (if unsigned { f64::INFINITY } else { diff.signum() * f64::INFINITY }, 0.0) } else { (0.0, 1.0) }
}

fn compare_independent_newman_keuls(
    entries: &[NamedSliceContrast<'_>],
    stats: &[f64],
    raw_p: &[f64],
    threshold: f64,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let m = entries.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|&a, &b| raw_p[a].total_cmp(&raw_p[b]).then_with(|| stats[b].abs().total_cmp(&stats[a].abs())));

    let mut adj_vec = vec![1.0_f64; m];
    let mut running_max = 0.0_f64;
    for (rank, &orig_idx) in order.iter().enumerate() {
        let e = &entries[orig_idx];
        let m_rem = m - rank;
        let cand = if e.se > 0.0 {
            let t_abs = (e.diff / e.se).abs();
            max_studentized_range_sf(std::f64::consts::SQRT_2 * t_abs, m_rem, 2, e.df)?
        } else {
            raw_p[orig_idx]
        };
        running_max = running_max.max(cand).max(raw_p[orig_idx]).clamp(0.0, 1.0);
        adj_vec[orig_idx] = running_max;
    }

    let mut out = Vec::with_capacity(m);
    for (idx, e) in entries.iter().enumerate() {
        out.push(build_pair_detail(
            e.id_a,
            e.id_b,
            e.label_a.to_owned(),
            e.label_b.to_owned(),
            e.diff,
            e.se,
            e.df,
            stats[idx],
            raw_p[idx],
            adj_vec[idx],
            false,
            None,
            threshold,
        ));
    }
    Ok(out)
}

fn compare_independent_contrast_family(
    entries: &[NamedSliceContrast<'_>],
    proc_req: MultiplicityProcedure,
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let m = entries.len();
    if m == 0 {
        return Ok(Vec::new());
    }
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: threshold });
    }
    if report_ci && (!ci_alpha.is_finite() || ci_alpha <= 0.0 || ci_alpha >= 1.0) {
        return Err(StatsError::InvalidParameter { name: "ci_alpha", value: ci_alpha });
    }
    let uses_q_stat =
        matches!(proc_req, MultiplicityProcedure::TukeyKramer | MultiplicityProcedure::NewmanKeuls | MultiplicityProcedure::GamesHowell);
    let mut raw_p = Vec::with_capacity(m);
    let mut stats = Vec::with_capacity(m);
    for e in entries {
        if !e.diff.is_finite() || !e.se.is_finite() || e.se < 0.0 || !e.df.is_finite() || e.df <= 0.0 {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "contrast_estimate", value: f64::NAN }));
        }
        if e.se > 0.0 {
            let t = e.diff / e.se;
            let rp = student_t_two_sided_p(t, e.df)?;
            if !rp.is_finite() {
                return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "contrast_p_value", value: rp }));
            }
            raw_p.push(rp);
            stats.push(if uses_q_stat { t.abs() * std::f64::consts::SQRT_2 } else { t });
        } else {
            let (st, rp) = zero_se_pair(e.diff, uses_q_stat);
            raw_p.push(rp);
            stats.push(st);
        }
    }

    if m == 1 || matches!(proc_req, MultiplicityProcedure::None | MultiplicityProcedure::FisherLsd) {
        let mut out = Vec::with_capacity(m);
        for (idx, e) in entries.iter().enumerate() {
            let ci = if report_ci && !matches!(proc_req, MultiplicityProcedure::NewmanKeuls) {
                let tc = student_t_two_sided_isf(ci_alpha.clamp(1e-12, 0.9999), e.df)?;
                Some((e.diff - tc * e.se, e.diff + tc * e.se))
            } else {
                None
            };
            out.push(build_pair_detail(
                e.id_a,
                e.id_b,
                e.label_a.to_owned(),
                e.label_b.to_owned(),
                e.diff,
                e.se,
                e.df,
                stats[idx],
                raw_p[idx],
                raw_p[idx],
                false,
                ci,
                threshold,
            ));
        }
        return Ok(out);
    }

    match proc_req {
        MultiplicityProcedure::NewmanKeuls => compare_independent_newman_keuls(entries, &stats, &raw_p, threshold),
        MultiplicityProcedure::TukeyKramer
        | MultiplicityProcedure::GamesHowell
        | MultiplicityProcedure::DunnettT3
        | MultiplicityProcedure::Dunnett { .. } => {
            let mut out = Vec::with_capacity(m);
            for (idx, e) in entries.iter().enumerate() {
                let adj_p = if e.se > 0.0 {
                    let t_abs = (e.diff / e.se).abs();
                    max_studentized_range_sf(std::f64::consts::SQRT_2 * t_abs, m, 2, e.df)?
                } else {
                    raw_p[idx]
                };
                let ci = if report_ci {
                    let crit = max_studentized_range_isf(ci_alpha, m, 2, e.df)? * std::f64::consts::FRAC_1_SQRT_2;
                    Some((e.diff - crit * e.se, e.diff + crit * e.se))
                } else {
                    None
                };
                out.push(build_pair_detail(
                    e.id_a,
                    e.id_b,
                    e.label_a.to_owned(),
                    e.label_b.to_owned(),
                    e.diff,
                    e.se,
                    e.df,
                    stats[idx],
                    raw_p[idx],
                    adj_p,
                    false,
                    ci,
                    threshold,
                ));
            }
            Ok(out)
        }
        _ => {
            let eff_proc = if matches!(proc_req, MultiplicityProcedure::TamhaneT2) { MultiplicityProcedure::Sidak } else { proc_req };
            let (adj_p, is_fdr) = adjust_p_values(&raw_p, eff_proc, threshold);
            let per_alpha = match eff_proc {
                MultiplicityProcedure::Bonferroni => Some(ci_alpha / (m as f64)),
                MultiplicityProcedure::Sidak => Some(1.0 - (1.0 - ci_alpha).powf(1.0 / (m as f64))),
                _ => None,
            };
            let mut out = Vec::with_capacity(m);
            for (idx, e) in entries.iter().enumerate() {
                let ci = match (report_ci, per_alpha) {
                    (true, Some(a)) => {
                        let tc = student_t_two_sided_isf(a.clamp(1e-12, 0.9999), e.df)?;
                        Some((e.diff - tc * e.se, e.diff + tc * e.se))
                    }
                    _ => None,
                };
                out.push(build_pair_detail(
                    e.id_a,
                    e.id_b,
                    e.label_a.to_owned(),
                    e.label_b.to_owned(),
                    e.diff,
                    e.se,
                    e.df,
                    stats[idx],
                    raw_p[idx],
                    adj_p[idx],
                    is_fdr,
                    ci,
                    threshold,
                ));
            }
            Ok(out)
        }
    }
}

struct ContrastFamilyEval<'a> {
    levels: &'a [(usize, String)],
    level_means: &'a [Option<f64>],
    mult_spec: &'a MultiplicitySpec,
    confidence: ConfidenceLevel,
    n_slices: usize,
    pairs: Vec<(usize, usize, ContrastEstimate)>,
}

type ContrastFamilyOutcome = (Vec<PairwiseComparisonDetail>, Option<String>);

fn compare_contrast_family(eval: ContrastFamilyEval<'_>) -> Result<ContrastFamilyOutcome, StatsError> {
    let k = eval.levels.len();
    let proc_req = MultiplicityProcedure::from(eval.mult_spec.method);
    let threshold = eval.mult_spec.threshold;
    let ci_alpha = eval.confidence.alpha();
    let report_ci = eval.mult_spec.report_confidence_intervals;
    let n_slices = eval.n_slices.max(1);
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: threshold });
    }
    if eval.pairs.is_empty() {
        return Ok((Vec::new(), None));
    }

    let mut used = vec![false; k];
    for &(i, j, _) in &eval.pairs {
        used[i] = true;
        used[j] = true;
    }
    let k_eff = used.iter().filter(|&&u| u).count().max(2);

    match proc_req {
        MultiplicityProcedure::Dunnett { .. } => compare_contrast_dunnett(&eval, threshold, ci_alpha, report_ci),
        MultiplicityProcedure::TukeyKramer | MultiplicityProcedure::GamesHowell => {
            let mut out = Vec::with_capacity(eval.pairs.len());
            for &(i, j, ref est) in &eval.pairs {
                let se_q = est.se * std::f64::consts::FRAC_1_SQRT_2;
                let (q_stat, raw_p, adj_p) = if se_q > 0.0 {
                    let q = est.diff.abs() / se_q;
                    let rp = student_t_two_sided_p(est.diff / est.se, est.df)?;
                    (q, rp, max_studentized_range_sf(q, n_slices, k_eff, est.df)?)
                } else {
                    let (st, rp) = zero_se_pair(est.diff, true);
                    (st, rp, rp)
                };
                let ci = if report_ci {
                    let qc = max_studentized_range_isf(ci_alpha, n_slices, k_eff, est.df)?;
                    Some((est.diff - qc * se_q, est.diff + qc * se_q))
                } else {
                    None
                };
                out.push(build_pair_detail(
                    eval.levels[i].0,
                    eval.levels[j].0,
                    eval.levels[i].1.clone(),
                    eval.levels[j].1.clone(),
                    est.diff,
                    est.se,
                    est.df,
                    q_stat,
                    raw_p,
                    adj_p,
                    false,
                    ci,
                    threshold,
                ));
            }
            Ok((out, None))
        }
        MultiplicityProcedure::NewmanKeuls => Ok((compare_contrast_newman_keuls(&eval, threshold)?, None)),
        _ => {
            let entries: Vec<NamedSliceContrast<'_>> = eval
                .pairs
                .iter()
                .map(|&(i, j, ref est)| NamedSliceContrast {
                    id_a: eval.levels[i].0,
                    id_b: eval.levels[j].0,
                    label_a: eval.levels[i].1.as_str(),
                    label_b: eval.levels[j].1.as_str(),
                    diff: est.diff,
                    se: est.se,
                    df: est.df,
                })
                .collect();
            let details = compare_independent_contrast_family(&entries, proc_req, threshold, ci_alpha, report_ci)?;
            Ok((details, None))
        }
    }
}

fn factor_loadings_from_cov_vectors(pairs: &[(usize, usize, ContrastEstimate)]) -> Option<Vec<f64>> {
    let m = pairs.len();
    if m <= 1 {
        return Some(vec![std::f64::consts::FRAC_1_SQRT_2; m]);
    }
    if pairs.iter().all(|(_, _, e)| e.se <= 0.0) {
        return Some(vec![std::f64::consts::FRAC_1_SQRT_2; m]);
    }
    if !pairs.iter().all(|(_, _, e)| e.se > 0.0 && e.cov_vectors.is_some()) {
        return None;
    }
    let mut rhos = vec![vec![0.0_f64; m]; m];
    let mut max_abs_rho = 0.0_f64;
    for a in 0..m {
        let (la, _) = pairs[a].2.cov_vectors.as_ref()?;
        for b in (a + 1)..m {
            let (_, cov_lb) = pairs[b].2.cov_vectors.as_ref()?;
            let denom = pairs[a].2.se * pairs[b].2.se;
            if denom <= 0.0 {
                return None;
            }
            let r = dot_slice(la, cov_lb) / denom;
            if !r.is_finite() {
                return None;
            }
            max_abs_rho = max_abs_rho.max(r.abs());
            rhos[a][b] = r;
            rhos[b][a] = r;
        }
    }
    if max_abs_rho <= 1e-12 {
        return Some(vec![0.0; m]);
    }
    if m == 2 {
        let r01 = rhos[0][1];
        if (0.0..0.999_999).contains(&r01) {
            let lam = r01.sqrt();
            return Some(vec![lam, lam]);
        }
        return None;
    }
    let mut log_row_sums = vec![0.0_f64; m];
    for a in 0..m {
        for (b, &r) in rhos[a].iter().enumerate() {
            if a == b {
                continue;
            }
            if !(1e-12..0.999_999).contains(&r) {
                return None;
            }
            log_row_sums[a] += r.ln();
        }
    }
    let sum_s: f64 = log_row_sums.iter().sum();
    let offset = sum_s / (2.0 * ((m - 1) as f64));
    let denom_m = (m - 2) as f64;
    let loadings: Vec<f64> = log_row_sums.iter().map(|&sa| ((sa - offset) / denom_m).exp().clamp(0.0, 0.999_999)).collect();
    for a in 0..m {
        for b in (a + 1)..m {
            if (rhos[a][b] - loadings[a] * loadings[b]).abs() > 1e-4 {
                return None;
            }
        }
    }
    Some(loadings)
}

fn resolve_dunnett_loadings(pairs: &[(usize, usize, ContrastEstimate)]) -> Option<Vec<f64>> {
    if pairs.len() > 1 && pairs.iter().all(|(_, _, e)| e.se > 0.0 && e.cov_vectors.is_some()) {
        return factor_loadings_from_cov_vectors(pairs);
    }
    if let Some(loadings) = factor_loadings_from_cov_vectors(pairs) {
        return Some(loadings);
    }
    let first_vc = pairs.first().and_then(|(_, _, e)| e.ctrl_var_pair.map(|(_, vc)| vc))?;
    if pairs.iter().all(|(_, _, e)| matches!(e.ctrl_var_pair, Some((vt, vc)) if vt + vc > 0.0 && (vc - first_vc).abs() <= 1e-9 * first_vc.max(1.0))) {
        return Some(
            pairs
                .iter()
                .map(|(_, _, e)| match e.ctrl_var_pair {
                    Some((v_tr, v_ctrl)) if v_tr + v_ctrl > 0.0 => (v_ctrl / (v_tr + v_ctrl)).sqrt().clamp(0.0, 0.999_999),
                    _ => 0.0,
                })
                .collect(),
        );
    }
    None
}

fn compare_contrast_dunnett(
    eval: &ContrastFamilyEval<'_>,
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<ContrastFamilyOutcome, StatsError> {
    let m = eval.pairs.len();
    let n_slices = eval.n_slices.max(1);
    let Some(loadings) = resolve_dunnett_loadings(&eval.pairs) else {
        let m_tot = (m * n_slices).max(1);
        let mut out = Vec::with_capacity(m);
        for &(i, j, ref est) in &eval.pairs {
            let (t_stat, raw_p, adj_p) = if est.se > 0.0 {
                let t = est.diff / est.se;
                let rp = student_t_two_sided_p(t, est.df)?;
                let ap = max_studentized_range_sf(std::f64::consts::SQRT_2 * t.abs(), m_tot, 2, est.df)?;
                (t, rp, ap)
            } else {
                let (st, rp) = zero_se_pair(est.diff, false);
                (st, rp, rp)
            };
            let ci = if report_ci {
                let crit = max_studentized_range_isf(ci_alpha, m_tot, 2, est.df)? * std::f64::consts::FRAC_1_SQRT_2;
                Some((est.diff - crit * est.se, est.diff + crit * est.se))
            } else {
                None
            };
            out.push(build_pair_detail(
                eval.levels[i].0,
                eval.levels[j].0,
                eval.levels[i].1.clone(),
                eval.levels[j].1.clone(),
                est.diff,
                est.se,
                est.df,
                t_stat,
                raw_p,
                adj_p,
                false,
                ci,
                threshold,
            ));
        }
        let note = String::from(
            "Contrast correlation structure is non-estimable in single-factor Dunnett form; conservative Studentized Maximum Modulus adjustment applied.",
        );
        return Ok((out, Some(note)));
    };

    let df_joint = eval.pairs.iter().map(|(_, _, e)| e.df).fold(f64::INFINITY, f64::min).max(1.0);
    let slice_ci_alpha = if n_slices > 1 { -((-ci_alpha.clamp(1e-12, 0.9999)).ln_1p() / (n_slices as f64)).exp_m1() } else { ci_alpha };
    let d_crit = if report_ci {
        Some(if m == 1 {
            student_t_two_sided_isf(slice_ci_alpha.clamp(1e-12, 0.9999), df_joint)?
        } else {
            dunnett_two_sided_crit_loadings(slice_ci_alpha.clamp(1e-12, 0.9999), df_joint, &loadings)?
        })
    } else {
        None
    };

    let mut out = Vec::with_capacity(m);
    for &(i, j, ref est) in &eval.pairs {
        let (t_stat, raw_p, adj_p) = if est.se > 0.0 {
            let t = est.diff / est.se;
            let rp = student_t_two_sided_p(t, est.df)?;
            let ap_slice = if m == 1 { rp } else { dunnett_two_sided_p_loadings(t.abs(), df_joint, &loadings)? };
            let ap = if n_slices > 1 {
                let pc = ap_slice.clamp(0.0, 1.0);
                if pc >= 1.0 { 1.0 } else { (-((n_slices as f64) * (-pc).ln_1p()).exp_m1()).clamp(pc, 1.0) }
            } else {
                ap_slice
            };
            (t, rp, ap)
        } else {
            let (st, rp) = zero_se_pair(est.diff, false);
            (st, rp, rp)
        };
        let ci = d_crit.map(|dc| (est.diff - dc * est.se, est.diff + dc * est.se));
        out.push(build_pair_detail(
            eval.levels[i].0,
            eval.levels[j].0,
            eval.levels[i].1.clone(),
            eval.levels[j].1.clone(),
            est.diff,
            est.se,
            est.df,
            t_stat,
            raw_p,
            adj_p,
            false,
            ci,
            threshold,
        ));
    }
    Ok((out, None))
}

fn resolve_level_order_for_nk(k: usize, eval: &ContrastFamilyEval<'_>) -> Vec<usize> {
    let mut means = vec![None; k];
    for (idx, m) in means.iter_mut().enumerate() {
        *m = eval.level_means.get(idx).copied().flatten();
    }
    let mut part: Vec<usize> = Vec::new();
    for i in 0..k {
        if eval.pairs.iter().any(|&(a, b, _)| a == i || b == i) {
            part.push(i);
        }
    }
    if part.iter().any(|&idx| means[idx].is_none()) && !part.is_empty() {
        means[part[0]] = Some(0.0);
        for _ in 0..part.len() {
            for &(a, b, ref est) in &eval.pairs {
                if let (Some(ma), None) = (means[a], means[b]) {
                    means[b] = Some(ma - est.diff);
                } else if let (None, Some(mb)) = (means[a], means[b]) {
                    means[a] = Some(mb + est.diff);
                }
            }
        }
    }
    part.sort_by(|&a, &b| means[a].unwrap_or(0.0).total_cmp(&means[b].unwrap_or(0.0)));
    part
}

fn compare_contrast_newman_keuls(eval: &ContrastFamilyEval<'_>, threshold: f64) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k_total = eval.levels.len();
    let n_slices = eval.n_slices.max(1);
    let sorted_idx = resolve_level_order_for_nk(k_total, eval);
    let k = sorted_idx.len();
    if k < 2 {
        return Ok(Vec::new());
    }
    let mut pair_map: std::collections::BTreeMap<(usize, usize), &ContrastEstimate> = std::collections::BTreeMap::new();
    for &(i, j, ref est) in &eval.pairs {
        pair_map.insert((i.min(j), i.max(j)), est);
    }

    let mut p_mat = vec![vec![1.0_f64; k]; k];
    for span in (2..=k).rev() {
        for lo in 0..=(k - span) {
            let hi = lo + span - 1;
            let u = sorted_idx[lo];
            let v = sorted_idx[hi];
            let raw_p = if let Some(&est) = pair_map.get(&(u.min(v), u.max(v))) {
                let se_q = est.se * std::f64::consts::FRAC_1_SQRT_2;
                if se_q > 0.0 { max_studentized_range_sf(est.diff.abs() / se_q, n_slices, span, est.df)? } else { zero_se_pair(est.diff, true).1 }
            } else {
                0.0
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

    let mut pos_of = vec![0_usize; k_total];
    for (pos, &orig) in sorted_idx.iter().enumerate() {
        pos_of[orig] = pos;
    }

    let mut out = Vec::with_capacity(eval.pairs.len());
    for &(i, j, ref est) in &eval.pairs {
        let se_q = est.se * std::f64::consts::FRAC_1_SQRT_2;
        let (q_stat, raw_p) =
            if se_q > 0.0 { (est.diff.abs() / se_q, student_t_two_sided_p(est.diff / est.se, est.df)?) } else { zero_se_pair(est.diff, true) };
        let (p_lo, p_hi) = (pos_of[i].min(pos_of[j]), pos_of[i].max(pos_of[j]));
        let adj_p = p_mat[p_lo][p_hi];
        out.push(build_pair_detail(
            eval.levels[i].0,
            eval.levels[j].0,
            eval.levels[i].1.clone(),
            eval.levels[j].1.clone(),
            est.diff,
            est.se,
            est.df,
            q_stat,
            raw_p,
            adj_p,
            false,
            None,
            threshold,
        ));
    }
    Ok(out)
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

fn validate_dunnett_ctrl(mult_spec: &MultiplicitySpec, n_levels: usize) -> Result<(), StatsError> {
    if let MultiplicityMethod::Dunnett { control_column } = mult_spec.method
        && control_column >= n_levels
    {
        return Err(StatsError::InvalidParameter { name: "control_column", value: control_column as f64 });
    }
    Ok(())
}

/// Resolve the single factorial factor targeted by `MultiplicityMethod::Dunnett { control_column }`.
///
/// Prefers primary factor `0` when `control_column` is in bounds there, otherwise falls back to the
/// first factor with at least 2 levels where `control_column < levels.len()`, returning
/// `StatsError::InvalidParameter` when `control_column` is out of bounds for every factor.
fn resolve_factorial_dunnett_target(obs: &FactorialObservations, control_column: usize) -> Result<usize, StatsError> {
    for (f_idx, levels) in obs.factor_levels.iter().enumerate() {
        if levels.len() >= 2 && control_column < levels.len() {
            return Ok(f_idx);
        }
    }
    Err(StatsError::InvalidParameter { name: "control_column", value: control_column as f64 })
}

fn is_unequal_variance_method(method: MultiplicityMethod) -> bool {
    matches!(method, MultiplicityMethod::GamesHowell | MultiplicityMethod::TamhaneT2 | MultiplicityMethod::DunnettT3)
}

fn is_contrast_count_multiplicity_method(method: MultiplicityMethod) -> bool {
    matches!(
        method,
        MultiplicityMethod::None
            | MultiplicityMethod::FisherLsd
            | MultiplicityMethod::Bonferroni
            | MultiplicityMethod::Sidak
            | MultiplicityMethod::HolmSidak
            | MultiplicityMethod::TamhaneT2
            | MultiplicityMethod::DunnettT3
            | MultiplicityMethod::BenjaminiHochbergFdr
            | MultiplicityMethod::BenjaminiYekutieliFdr
            | MultiplicityMethod::TwoStageBkyFdr
    )
}

fn collect_factor_pairs<F>(n_levels: usize, method: MultiplicityMethod, mut eval_pair: F) -> Result<Vec<(usize, usize, ContrastEstimate)>, StatsError>
where
    F: FnMut(usize, usize) -> Result<Option<ContrastEstimate>, StatsError>,
{
    let mut pairs = Vec::new();
    if let MultiplicityMethod::Dunnett { control_column } = method {
        for tr in 0..n_levels {
            if tr == control_column {
                continue;
            }
            if let Some(est) = eval_pair(tr, control_column)? {
                pairs.push((tr, control_column, est));
            }
        }
    } else {
        for i in 0..n_levels {
            for j in (i + 1)..n_levels {
                if let Some(est) = eval_pair(i, j)? {
                    pairs.push((i, j, est));
                }
            }
        }
    }
    Ok(pairs)
}

fn run_simple_effects_for_pair(
    out: &mut ResultSheet,
    obs: &FactorialObservations,
    post_hoc: &FactorialPostHocContext,
    cfg: &SimpleEffectsSpec<'_>,
    next_group_id: &mut usize,
) -> Result<(), StatsError> {
    let cond_levels = &obs.factor_levels[cfg.cond_factor];
    let target_levels = &obs.factor_levels[cfg.target_factor];
    let k_t = target_levels.len();
    if k_t < 2 {
        return Ok(());
    }
    validate_dunnett_ctrl(cfg.mult_spec, k_t)?;
    let unequal = is_unequal_variance_method(cfg.mult_spec.method);
    let ci_alpha = cfg.confidence.alpha();

    if k_t == 2 {
        let (idx_a, idx_b) = match cfg.mult_spec.method {
            MultiplicityMethod::Dunnett { control_column: 0 } => (1, 0),
            _ => (0, 1),
        };
        struct TwoLvlItem {
            id_a: usize,
            id_b: usize,
            lbl_a: String,
            lbl_b: String,
            est: ContrastEstimate,
        }
        let mut items = Vec::with_capacity(cond_levels.len());
        for (c_idx, cond_label) in cond_levels.iter().enumerate() {
            if let Some(est) = post_hoc.eval_simple_pair(cfg.cond_factor, c_idx, cfg.target_factor, idx_a, idx_b, unequal)? {
                let id_a = *next_group_id;
                let id_b = *next_group_id + 1;
                *next_group_id += 2;
                items.push(TwoLvlItem {
                    id_a,
                    id_b,
                    lbl_a: format!("{cond_label} | {}", target_levels[idx_a]),
                    lbl_b: format!("{cond_label} | {}", target_levels[idx_b]),
                    est,
                });
            }
        }
        let slice_refs: Vec<NamedSliceContrast<'_>> = items
            .iter()
            .map(|it| NamedSliceContrast {
                id_a: it.id_a,
                id_b: it.id_b,
                label_a: it.lbl_a.as_str(),
                label_b: it.lbl_b.as_str(),
                diff: it.est.diff,
                se: it.est.se,
                df: it.est.df,
            })
            .collect();
        let combined = compare_independent_contrast_family(
            &slice_refs,
            cfg.mult_spec.method.into(),
            cfg.mult_spec.threshold,
            ci_alpha,
            cfg.mult_spec.report_confidence_intervals,
        )?;
        let note = if combined.len() >= 2 && !matches!(cfg.mult_spec.method, MultiplicityMethod::None | MultiplicityMethod::FisherLsd) {
            Some(format!("Multiplicity adjusted across all {} simple-effect comparisons.", combined.len()))
        } else {
            None
        };
        push_pairwise_outputs_with_footnote(out, &combined, cfg.tab_id, cfg.title, cfg.p_fmt, cfg.confidence, note);
        return Ok(());
    }

    let mut slice_groups = Vec::with_capacity(cond_levels.len());
    for (c_idx, cond_label) in cond_levels.iter().enumerate() {
        let base_id = *next_group_id;
        *next_group_id += k_t;
        let slice_levels: Vec<(usize, String)> =
            target_levels.iter().enumerate().map(|(i, l)| (base_id + i, format!("{cond_label} | {l}"))).collect();
        let pairs = collect_factor_pairs(k_t, cfg.mult_spec.method, |i, j| {
            post_hoc.eval_simple_pair(cfg.cond_factor, c_idx, cfg.target_factor, i, j, unequal)
        })?;
        if !pairs.is_empty() {
            slice_groups.push((slice_levels, pairs));
        }
    }
    if slice_groups.is_empty() {
        return Ok(());
    }

    let n_slices = slice_groups.len();
    let mut dunnett_fallback_note: Option<String> = None;
    let combined = if is_contrast_count_multiplicity_method(cfg.mult_spec.method) {
        let mut flat_refs = Vec::new();
        for (slice_levels, pairs) in &slice_groups {
            for &(i, j, ref est) in pairs {
                flat_refs.push(NamedSliceContrast {
                    id_a: slice_levels[i].0,
                    id_b: slice_levels[j].0,
                    label_a: slice_levels[i].1.as_str(),
                    label_b: slice_levels[j].1.as_str(),
                    diff: est.diff,
                    se: est.se,
                    df: est.df,
                });
            }
        }
        compare_independent_contrast_family(
            &flat_refs,
            cfg.mult_spec.method.into(),
            cfg.mult_spec.threshold,
            ci_alpha,
            cfg.mult_spec.report_confidence_intervals,
        )?
    } else {
        let level_means: Vec<Option<f64>> = vec![None; k_t];
        let mut all_details = Vec::new();
        for (slice_levels, pairs) in slice_groups {
            let (details, note_opt) = compare_contrast_family(ContrastFamilyEval {
                levels: &slice_levels,
                level_means: &level_means,
                mult_spec: cfg.mult_spec,
                confidence: cfg.confidence,
                n_slices,
                pairs,
            })?;
            if dunnett_fallback_note.is_none() {
                dunnett_fallback_note = note_opt;
            }
            all_details.extend(details);
        }
        all_details
    };

    let mut notes = Vec::new();
    if combined.len() >= 2 && !matches!(cfg.mult_spec.method, MultiplicityMethod::None | MultiplicityMethod::FisherLsd) {
        notes.push(format!("Multiplicity adjusted across all {} simple-effect comparisons.", combined.len()));
    }
    if let Some(dn) = dunnett_fallback_note {
        notes.push(dn);
    }
    let merged_note = if notes.is_empty() { None } else { Some(notes.join(" ")) };
    push_pairwise_outputs_with_footnote(out, &combined, cfg.tab_id, cfg.title, cfg.p_fmt, cfg.confidence, merged_note);
    Ok(())
}

fn run_factorial_multiplicity(
    out: &mut ResultSheet,
    obs: &FactorialObservations,
    post_hoc: &FactorialPostHocContext,
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
    let dunnett_target = match mult_spec.method {
        MultiplicityMethod::Dunnett { control_column } => Some((resolve_factorial_dunnett_target(obs, control_column)?, control_column)),
        _ => None,
    };
    let unequal = is_unequal_variance_method(mult_spec.method);
    let mut next_group_id = 0_usize;

    for (f_idx, levels) in obs.factor_levels.iter().enumerate() {
        let k = levels.len();
        if k < 2 {
            continue;
        }
        if let Some((target_f, _)) = dunnett_target
            && f_idx != target_f
        {
            continue;
        }
        validate_dunnett_ctrl(mult_spec, k)?;
        let base_id = next_group_id;
        next_group_id += k;
        let numbered_levels: Vec<(usize, String)> = levels.iter().enumerate().map(|(i, l)| (base_id + i, l.clone())).collect();
        let pairs = collect_factor_pairs(k, mult_spec.method, |i, j| post_hoc.eval_marginal_pair(f_idx, i, j, unequal))?;
        if pairs.is_empty() {
            continue;
        }
        let mut level_means = Vec::with_capacity(k);
        for lvl in 0..k {
            level_means.push(post_hoc.level_mean_estimate(f_idx, lvl)?);
        }
        let (details, fam_note) = compare_contrast_family(ContrastFamilyEval {
            levels: &numbered_levels,
            level_means: &level_means,
            mult_spec,
            confidence,
            n_slices: 1,
            pairs,
        })?;
        let tab_id = match f_idx {
            0 => String::from("multiple-comparisons-factor-a"),
            1 => String::from("multiple-comparisons-factor-b"),
            n => format!("multiple-comparisons-factor-{}", n + 1),
        };
        let f_name = obs.factor_names.get(f_idx).map_or("Factor", String::as_str);
        let title = format!("Multiple comparisons: {f_name} (marginal means)");
        let mut notes = Vec::new();
        if let Some((_, ctrl_idx)) = dunnett_target
            && obs.factor_levels.len() >= 2
        {
            let ctrl_lbl = levels.get(ctrl_idx).map_or("control", String::as_str);
            notes.push(format!("Dunnett many-to-one comparisons target {f_name} against control level `{ctrl_lbl}` (index {ctrl_idx})."));
        }
        if let Some(fn_str) = fam_note {
            notes.push(fn_str);
        }
        let extra_note = if notes.is_empty() { None } else { Some(notes.join(" ")) };
        push_pairwise_outputs_with_footnote(out, &details, &tab_id, &title, p_fmt, confidence, extra_note);
    }

    if obs.factor_levels.len() >= 2 {
        let name_a = obs.factor_names.first().map_or("Factor A", String::as_str);
        let name_b = obs.factor_names.get(1).map_or("Factor B", String::as_str);
        if dunnett_target.is_none_or(|(target_f, _)| target_f == 0) {
            let title_a_within_b = format!("Simple effects: {name_a} within {name_b}");
            run_simple_effects_for_pair(
                out,
                obs,
                post_hoc,
                &SimpleEffectsSpec {
                    cond_factor: 1,
                    target_factor: 0,
                    mult_spec,
                    tab_id: "multiple-comparisons-simple-a-within-b",
                    title: &title_a_within_b,
                    p_fmt,
                    confidence,
                },
                &mut next_group_id,
            )?;
        }
        if dunnett_target.is_none_or(|(target_f, _)| target_f == 1) {
            let title_b_within_a = format!("Simple effects: {name_b} within {name_a}");
            run_simple_effects_for_pair(
                out,
                obs,
                post_hoc,
                &SimpleEffectsSpec {
                    cond_factor: 0,
                    target_factor: 1,
                    mult_spec,
                    tab_id: "multiple-comparisons-simple-b-within-a",
                    title: &title_b_within_a,
                    p_fmt,
                    confidence,
                },
                &mut next_group_id,
            )?;
        }
    }

    Ok(())
}

fn grouped_row_levels(grouped: &curvebook_doc::GroupedTable, n_rows: usize) -> Vec<String> {
    (0..n_rows).map(|r| grouped.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1))).collect()
}

struct GroupedFactorialOutcome {
    result: FactorialAnovaResult,
    observations: FactorialObservations,
    post_hoc: FactorialPostHocContext,
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
        let mauchly_lbl = if s.mauchly_p.is_finite() { format!("{:.4}", s.mauchly_p) } else { String::from("n/a") };
        summary_parts.push(format!("{name}: eps_GG={:.4}, eps_HF={:.4}, P={mauchly_lbl}", s.greenhouse_geisser_eps, s.huynh_feldt_eps));
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
    grouped: &curvebook_doc::GroupedTable,
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

    // Repeated-measures 2-way path (REML mixed-effects fallback on missing replicates or explicit unstructured covariance).
    if !fac_spec.three_way && fac_spec.repeated_measures != RepeatedMeasuresDesign::None {
        let has_missing_selected = (0..n_rows).any(|r| {
            selected_cols.iter().any(|&c| (0..n_subs).any(|s| grouped.observation(r, c, s).and_then(|cell| cell.active_number()).is_none()))
        });
        if (has_missing_selected && fac_spec.fit_reml_on_missing) || fac_spec.residual_covariance == ResidualCovariance::Unstructured {
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
        let (rm_res, rm_post_hoc) = fit_rm_two_way_anova_with_posthoc(
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
        return Ok(GroupedFactorialOutcome {
            result: rm_res.anova,
            observations: data,
            post_hoc: FactorialPostHocContext::RmBalanced(rm_post_hoc),
            aux_tabs: sph_tab.into_iter().collect(),
        });
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
        let (res, model) = fit_factorial_anova_with_model(&data, fac_spec.max_interaction_order.clamp(1, 3), fac_spec.ss_type)?;
        return Ok(GroupedFactorialOutcome { result: res, observations: data, post_hoc: FactorialPostHocContext::Ols(model), aux_tabs: Vec::new() });
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
    let (res, model) = fit_factorial_anova_with_model(&data, fac_spec.max_interaction_order.clamp(1, 2), fac_spec.ss_type)?;
    Ok(GroupedFactorialOutcome { result: res, observations: data, post_hoc: FactorialPostHocContext::Ols(model), aux_tabs: Vec::new() })
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

struct RmUnstructuredDesign<'a> {
    x_mat: &'a DenseMatrix,
    y_vec: &'a [f64],
    subj_ids: &'a [usize],
    subj_row_ids: &'a [usize],
    subj_col_ids: &'a [usize],
    row_ids: &'a [usize],
    col_ids: &'a [usize],
    row_levels: &'a [String],
    col_levels: &'a [String],
    term_slices: &'a [(&'a str, usize, usize)],
    design: RepeatedMeasuresDesign,
}

fn unstructured_fallback_label(reason: UnstructuredFallbackReason) -> &'static str {
    match reason {
        UnstructuredFallbackReason::SingleSubject => "single-subject isotropic covariance fallback",
        UnstructuredFallbackReason::SmallSampleRankDeficient => "small-sample rank-deficient shrinkage fallback",
        UnstructuredFallbackReason::SingularCovarianceEstimate => "singular covariance boundary shrinkage fallback",
        UnstructuredFallbackReason::IterationLimitReached => "iteration-budget regularized iterate fallback",
    }
}

fn build_matrix_tab(id: &str, title: &str, labels: &[String], matrix: &[Vec<f64>], footnotes: Vec<String>) -> ResultTab {
    let rows: Vec<Vec<ResultCell>> = matrix.iter().map(|row| row.iter().map(|&v| num_cell(v)).collect()).collect();
    let mut tab = build_tab(id, title, ResultTabKind::CovarianceMatrix, labels.to_vec(), labels.to_vec(), rows);
    tab.footnotes = footnotes;
    tab
}

fn run_rm_two_way_unstructured_reml(
    out: &mut ResultSheet,
    ctx: &RmUnstructuredDesign<'_>,
) -> Result<(Vec<FactorialAnovaTerm>, f64, f64, RemlContrastCore), StatsError> {
    let n_cols = ctx.col_levels.len();
    let (cluster_ids, tp_ids, tp_labels): (Vec<usize>, Vec<usize>, Vec<String>) = match ctx.design {
        RepeatedMeasuresDesign::ColumnFactorMatched => (ctx.subj_row_ids.to_vec(), ctx.col_ids.to_vec(), ctx.col_levels.to_vec()),
        RepeatedMeasuresDesign::RowFactorMatched => (ctx.subj_col_ids.to_vec(), ctx.row_ids.to_vec(), ctx.row_levels.to_vec()),
        _ => {
            let tps: Vec<usize> = ctx.row_ids.iter().zip(ctx.col_ids.iter()).map(|(&r, &c)| r * n_cols + c).collect();
            let mut labels = Vec::with_capacity(ctx.row_levels.len() * n_cols);
            for r_lbl in ctx.row_levels {
                for c_lbl in ctx.col_levels {
                    labels.push(format!("{r_lbl} × {c_lbl}"));
                }
            }
            (ctx.subj_ids.to_vec(), tps, labels)
        }
    };

    let (ures, core) = fit_unstructured_rm_reml_with_core(ctx.x_mat, ctx.y_vec, &cluster_ids, &tp_ids, &tp_labels, ctx.term_slices)?;
    push_scalar(out, "REML mean diagonal variance", None, ures.mean_diagonal_variance);
    push_scalar(out, "REML log-likelihood", None, ures.reml_log_lik);
    push_reml_df_scalars(out, &ures.fixed_terms, &ures.satterthwaite_df, &ures.kenward_roger_df);

    let mut dropped_labels = Vec::new();
    for (idx, lbl) in tp_labels.iter().enumerate() {
        if !ures.active_timepoint_indices.contains(&idx) {
            dropped_labels.push(lbl.clone());
        }
    }

    let mut footnotes = Vec::new();
    if !dropped_labels.is_empty() {
        footnotes.push(format!("Excluded all-missing timepoint(s): {}.", dropped_labels.join(", ")));
    }
    if let Some(reason) = ures.fallback_reason {
        footnotes.push(format!("Regularized SPD covariance used ({}).", unstructured_fallback_label(reason)));
    }

    out.tabs.push(build_matrix_tab(
        "unstructured-reml-covariance",
        "Estimated residual covariance matrix (REML)",
        &ures.timepoint_labels,
        &ures.covariance_matrix,
        footnotes.clone(),
    ));
    out.tabs.push(build_matrix_tab(
        "unstructured-reml-correlation",
        "Estimated residual correlation matrix (REML)",
        &ures.timepoint_labels,
        &ures.correlation_matrix,
        footnotes,
    ));

    let status = if ures.fallback_reason.is_some() || !dropped_labels.is_empty() { ChecklistStatus::Warn } else { ChecklistStatus::Info };
    let mut explanation = String::from(
        "Fitted unstructured t x t repeated-measures residual covariance by Cholesky REML with Fai-Cornelius Satterthwaite and Kenward-Roger degrees of freedom.",
    );
    if !dropped_labels.is_empty() {
        explanation.push_str(&format!(" Excluded all-missing timepoint(s): {}.", dropped_labels.join(", ")));
    }
    if let Some(reason) = ures.fallback_reason {
        explanation.push_str(&format!(" Engaged {}.", unstructured_fallback_label(reason)));
    }
    out.checklist.push(checklist_item(
        "Unstructured covariance REML",
        status,
        Some(format!(
            "t = {}, REML logL = {:.2}, mean diag var = {:.4}",
            ures.timepoint_labels.len(),
            ures.reml_log_lik,
            ures.mean_diagonal_variance
        )),
        explanation,
    ));

    let df_sum: f64 = ures.fixed_terms.iter().map(|t| t.df).sum();
    let df_resid = ((ctx.y_vec.len() as f64) - 1.0 - df_sum).max(1.0);
    Ok((ures.fixed_terms, ures.mean_diagonal_variance, df_resid, core))
}

fn run_rm_two_way_reml_fallback(
    grouped: &curvebook_doc::GroupedTable,
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
    let include_interaction = fac_spec.max_interaction_order >= 2;
    let inter_df = if include_interaction { n_r * n_c } else { 0 };
    let p_total = 1 + n_r + n_c + inter_df;

    let mut x_rows: Vec<f64> = Vec::new();
    let mut y_vec: Vec<f64> = Vec::new();
    let mut obs = Vec::new();
    let mut subj_ids: Vec<usize> = Vec::new();
    let mut subj_row_ids: Vec<usize> = Vec::new();
    let mut subj_col_ids: Vec<usize> = Vec::new();
    let mut row_ids: Vec<usize> = Vec::new();
    let mut col_ids: Vec<usize> = Vec::new();
    let mut observed_cells = vec![vec![false; n_cols]; n_rows];
    for s in 0..n_subs {
        for (r, r_basis) in row_basis.iter().enumerate() {
            for (c_pos, (&orig_c, c_basis)) in selected_cols.iter().zip(col_basis.iter()).enumerate() {
                if let Some(v) = grouped.observation(r, orig_c, s).and_then(|cell| cell.active_number()) {
                    let mut row_x = Vec::with_capacity(p_total);
                    row_x.push(1.0);
                    row_x.extend_from_slice(r_basis);
                    row_x.extend_from_slice(c_basis);
                    if include_interaction {
                        for &rb in r_basis {
                            for &cb in c_basis {
                                row_x.push(rb * cb);
                            }
                        }
                    }
                    x_rows.extend_from_slice(&row_x);
                    y_vec.push(v);
                    obs.push((vec![r, c_pos], v));
                    observed_cells[r][c_pos] = true;
                    subj_ids.push(s);
                    subj_row_ids.push(s * n_rows + r);
                    subj_col_ids.push(s * n_cols + c_pos);
                    row_ids.push(r);
                    col_ids.push(c_pos);
                }
            }
        }
    }
    let n_obs = y_vec.len();
    if n_obs == 0 {
        return Err(StatsError::EmptyInput);
    }
    if fac_spec.residual_covariance == ResidualCovariance::CompoundSymmetry && n_obs <= p_total {
        return Err(StatsError::InsufficientSampleSize { context: "RM REML observations", required: p_total + 1, actual: n_obs });
    }
    let x_mat = DenseMatrix::from_row_major(n_obs, p_total, x_rows)?;
    let inter_name = format!("{} × {}", grouped.row_factor_title, grouped.column_factor_title);
    let mut term_slices = vec![(grouped.row_factor_title.as_str(), 1, n_r), (grouped.column_factor_title.as_str(), 1 + n_r, n_c)];
    if include_interaction {
        term_slices.push((inter_name.as_str(), 1 + n_r + n_c, inter_df));
    }

    let row_levels = grouped_row_levels(grouped, n_rows);
    let col_levels: Vec<String> = selected_cols.iter().map(|&c| grouped.data_sets[c].title.clone()).collect();

    let (fixed_terms, sigma2_e, df_residual, reml_core) = if fac_spec.residual_covariance == ResidualCovariance::Unstructured {
        run_rm_two_way_unstructured_reml(
            out,
            &RmUnstructuredDesign {
                x_mat: &x_mat,
                y_vec: &y_vec,
                subj_ids: &subj_ids,
                subj_row_ids: &subj_row_ids,
                subj_col_ids: &subj_col_ids,
                row_ids: &row_ids,
                col_ids: &col_ids,
                row_levels: &row_levels,
                col_levels: &col_levels,
                term_slices: &term_slices,
                design: fac_spec.repeated_measures,
            },
        )?
    } else {
        let (fixed_terms, sigma2_e, reml_ll, icc_subj, vc_tab, reml_core) = match fac_spec.repeated_measures {
            RepeatedMeasuresDesign::BothFactorsMatched => {
                let (g_s, c_s) = cluster_ids_to_g_matrix(&subj_ids)?;
                let (g_sr, c_sr) = cluster_ids_to_g_matrix(&subj_row_ids)?;
                let (g_sc, c_sc) = cluster_ids_to_g_matrix(&subj_col_ids)?;
                let sr_name = format!("subject × {}", grouped.row_factor_title);
                let sc_name = format!("subject × {}", grouped.column_factor_title);
                let (mres, core) = fit_multi_vc_reml_with_core(
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
                (mres.fixed_terms, mres.sigma2_error, mres.reml_log_lik, icc, tab, core)
            }
            design => {
                let cluster_ids: Vec<usize> = match design {
                    RepeatedMeasuresDesign::ColumnFactorMatched => subj_row_ids.clone(),
                    RepeatedMeasuresDesign::RowFactorMatched => subj_col_ids.clone(),
                    _ => subj_ids,
                };
                let (reml, core) = fit_random_intercept_reml_with_core(&x_mat, &y_vec, &cluster_ids, &term_slices)?;
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
                (reml.fixed_terms, reml.sigma2_error, reml.reml_log_lik, reml.icc, tab, core)
            }
        };

        out.tabs.push(vc_tab);
        out.checklist.push(checklist_item(
            "Missing-value mixed-effects REML",
            ChecklistStatus::Info,
            Some(format!("ICC = {icc_subj:.3}, REML logL = {reml_ll:.2}")),
            "Missing replicates detected in repeated-measures cells; fitted linear mixed-effects REML with Kenward-Roger degrees of freedom instead of listwise deletion.",
        ));
        (fixed_terms, sigma2_e, (n_obs - p_total) as f64, reml_core)
    };

    let ym = SummaryMoments::from_slice(&y_vec)?;
    let res = FactorialAnovaResult {
        terms: fixed_terms,
        ss_residual: sigma2_e * df_residual,
        df_residual,
        ms_residual: sigma2_e,
        ss_total: ym.css,
        df_total: (n_obs - 1) as f64,
        total_n: n_obs,
        unbalanced: true,
        ss_type: SumOfSquaresType::TypeIII,
    };
    let data = FactorialObservations {
        factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
        factor_levels: vec![row_levels, col_levels],
        rows: obs,
    };
    let (_, row_proj) = build_xtx_pinv_and_proj(&x_mat)?;
    let reml_model = RemlContrastModel {
        n_rows,
        n_cols,
        include_interaction,
        row_basis,
        col_basis,
        row_proj,
        col_map: reml_core.col_map,
        beta: reml_core.beta,
        phi: reml_core.phi,
        phi_a: reml_core.phi_a,
        w_info_inv: reml_core.w_info_inv,
        phi_m_phi: reml_core.phi_m_phi,
        n_minus_p: reml_core.n_minus_p,
        y_scale: reml_core.y_scale,
        observed_cells,
    };
    Ok(GroupedFactorialOutcome { result: res, observations: data, post_hoc: FactorialPostHocContext::Reml(reml_model), aux_tabs: Vec::new() })
}

fn run_multifactor_mv(
    mv: &curvebook_doc::MultipleVariablesTable,
    fac_spec: &FactorialAnovaSpec,
) -> Result<(FactorialAnovaResult, FactorialObservations, OlsPostHocModel), StatsError> {
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
                curvebook_doc::CellValue::Text(s) => s.trim().to_owned(),
                curvebook_doc::CellValue::Number(v) if v.is_finite() => format!("{v}"),
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
    let (res, model) = fit_factorial_anova_with_model(&data, fac_spec.max_interaction_order, fac_spec.ss_type)?;
    Ok((res, data, model))
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
    nested: &curvebook_doc::NestedTable,
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
        let details =
            compare_groups_pairwise_with_confidence(&g_inputs, mse_eff, res.df2_groups_kenward_roger, &nested_spec.multiplicity, confidence)?;
        push_pairwise_outputs(out, &details, "Nested multiple comparisons", p_fmt, confidence);
    }
    Ok(())
}
