//! Typed result structures returned by the ANOVA, repeated-measures, mixed-model (REML), nested,
//! and rank-based routines in this module.

use curvebook_doc::{RepeatedMeasuresDesign, SumOfSquaresType};
use serde::{Deserialize, Serialize};

/// ANOVA variance-explained effect sizes and noncentral-*F* confidence intervals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnovaEffectSizes {
    /// Unadjusted proportion of total variation explained ($\eta^2 = \mathrm{SS}_{\text{between}} / \mathrm{SS}_{\text{total}} \in [0, 1]$).
    pub eta_squared: f64,
    /// Noncentral-*F* two-sided confidence interval for $\eta^2$ at configured `ConfidenceLevel` (`None` when noncentral inversion unavailable or `df_between == 0`).
    pub eta_squared_ci: Option<(f64, f64)>,
    /// Partial proportion of variance explained ($\eta_p^2 = \mathrm{SS}_{\text{effect}} / (\mathrm{SS}_{\text{effect}} + \mathrm{SS}_{\text{error}}) \in [0, 1]$).
    pub partial_eta_squared: f64,
    /// Noncentral-*F* two-sided confidence interval for $\eta_p^2$ at configured `ConfidenceLevel` (`None` when unavailable).
    pub partial_eta_squared_ci: Option<(f64, f64)>,
    /// Hays bias-adjusted population variance-explained estimator $\omega^2 = (\mathrm{SS}_{\text{effect}} - \mathrm{df}_{\text{effect}} \mathrm{MS}_{\text{error}}) / (\mathrm{SS}_{\text{total}} + \mathrm{MS}_{\text{error}})$ (clamped `>= 0` or signed per Hays/Olejnik & Algina 2003).
    pub omega_squared: f64,
    /// Cohen's *f* standardized effect size $\sqrt{\eta_p^2 / (1 - \eta_p^2)}$ (`f64::INFINITY` if $\eta_p^2 = 1$).
    pub cohens_f: f64,
    /// Two-sided noncentral-*F* CI for Cohen's *f* converted from `partial_eta_squared_ci` (`None` when unavailable).
    pub cohens_f_ci: Option<(f64, f64)>,
}

/// Homogeneity-of-variance diagnostic tests across `k` groups (`available = false` emits `NaN` metrics for summary inputs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VarianceHomogeneityResult {
    /// Bartlett's (1937) normal-theory likelihood-ratio $\chi^2$ test statistic for equal variances (`NaN` when `!available` or zero group variance).
    pub bartlett_chi2: f64,
    /// Bartlett test degrees of freedom `k - 1`.
    pub bartlett_df: f64,
    /// Upper-tail $\chi^2(k - 1)$ *P* value for Bartlett's test (`NaN` when `!available` or zero group variance).
    pub bartlett_p: f64,
    /// Brown–Forsythe (1974) robust ANOVA *F*-statistic on absolute deviations from group medians (`NaN` when `!available`).
    pub brown_forsythe_f: f64,
    /// Numerator degrees of freedom `k - 1` for Brown–Forsythe test (`NaN` when `!available`).
    pub brown_forsythe_df1: f64,
    /// Denominator degrees of freedom `total_n - k` for Brown–Forsythe test (`NaN` when `!available`).
    pub brown_forsythe_df2: f64,
    /// Upper-tail $F(\mathrm{df1}, \mathrm{df2})$ *P* value for Brown–Forsythe homogeneity test (`NaN` when `!available`).
    pub brown_forsythe_p: f64,
    /// True when raw replicate data supported homogeneity tests; false for pre-summarized inputs.
    #[serde(default)]
    pub available: bool,
}

/// Ordinary one-way ANOVA table metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OneWayAnovaResult {
    /// Ordered group display labels (`len == k`).
    pub group_labels: Vec<String>,
    /// Sample sizes per group (`n_i`).
    pub group_n: Vec<usize>,
    /// Sample arithmetic means per group.
    pub group_means: Vec<f64>,
    /// Sample standard deviations (`n_i - 1` denominator) per group (`0.0` when `n_i == 1`).
    pub group_sds: Vec<f64>,
    /// Grand total number of observations $\sum n_i$.
    pub total_n: usize,
    /// Between-groups (treatment) sum of squares $\sum n_i (\bar{x}_i - \bar{x})^2$.
    pub ss_between: f64,
    /// Within-groups (residual error) sum of squares $\sum (n_i - 1) s_i^2$.
    pub ss_within: f64,
    /// Total corrected sum of squares `ss_between + ss_within`.
    pub ss_total: f64,
    /// Between-groups degrees of freedom `k - 1`.
    pub df_between: f64,
    /// Within-groups residual degrees of freedom `total_n - k`.
    pub df_within: f64,
    /// Total corrected degrees of freedom `total_n - 1`.
    pub df_total: f64,
    /// Mean square between groups `ss_between / df_between`.
    pub ms_between: f64,
    /// Mean square within groups `ss_within / df_within` (`MSE`).
    pub ms_within: f64,
    /// Omnibus ANOVA *F*-statistic `ms_between / ms_within` (`f64::INFINITY` on zero within-group error with non-zero between-group separation).
    pub f_statistic: f64,
    /// Upper-tail $F(\mathrm{df\_between}, \mathrm{df\_within})$ *P* value testing equal group means.
    pub p_value: f64,
    /// Unadjusted proportion of variance explained `ss_between / ss_total` ($\eta^2 \in [0, 1]$).
    pub r_squared: f64,
    /// Variance homogeneity diagnostic battery (Bartlett and Brown–Forsythe).
    pub variance_tests: VarianceHomogeneityResult,
    /// Effect-size summary (`eta_squared`, `partial_eta_squared`, `omega_squared`, `cohens_f`, and noncentral-*F* CIs).
    pub effect_sizes: AnovaEffectSizes,
}

/// Unequal-variance one-way ANOVA results (Brown–Forsythe and Welch).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnequalVarianceAnovaResult {
    /// Welch (1951) heteroscedastic one-way *W* *F*-statistic weighting groups by $w_i = n_i / s_i^2$.
    pub welch_w: f64,
    /// Welch numerator degrees of freedom `k - 1`.
    pub welch_df1: f64,
    /// Welch–Satterthwaite heteroscedastic denominator degrees of freedom.
    pub welch_df2: f64,
    /// Upper-tail $F(\text{welch\_df1}, \text{welch\_df2})$ *P* value testing equal group means without assuming homoscedasticity.
    pub welch_p: f64,
    /// Brown–Forsythe (1974) robust mean-comparison $F^*$ statistic (`sum n_i (mean_i - mean)^2 / sum (1 - n_i/N) s_i^2`).
    pub brown_forsythe_f_star: f64,
    /// Brown–Forsythe $F^*$ numerator degrees of freedom `k - 1`.
    pub brown_forsythe_df1: f64,
    /// Brown–Forsythe $F^*$ denominator effective Satterthwaite degrees of freedom.
    pub brown_forsythe_df2: f64,
    /// Upper-tail $F(\text{brown\_forsythe\_df1}, \text{brown\_forsythe\_df2})$ *P* value for equal means.
    pub brown_forsythe_p: f64,
}

/// Mauchly sphericity test and epsilon corrections for repeated-measures factors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SphericityResult {
    /// Mauchly's (1940) sphericity test criterion $W \in (0, 1]$ (`1.0` for `k <= 2` conditions or rank-1 covariance).
    pub mauchly_w: f64,
    /// Bartlett/Box chi-square approximation to $- \ln W$ (`0.0` when `k <= 2` or rank deficient).
    pub mauchly_chi2: f64,
    /// Chi-square degrees of freedom `k * (k - 1) / 2 - 1` (`m(m + 1)/2 - 1`).
    pub mauchly_df: f64,
    /// Upper-tail $\chi^2(\text{mauchly\_df})$ *P* value testing compound symmetry / sphericity (`NaN` when `n <= k` subjects or `k == 2`).
    pub mauchly_p: f64,
    /// Greenhouse–Geisser (1959) conservative sphericity correction $\hat{\varepsilon} \in [1 / (k - 1), 1]$.
    pub greenhouse_geisser_eps: f64,
    /// Huynh–Feldt / Lecoutre (1976 / 1991) less-conservative sphericity correction $\tilde{\varepsilon} \in [1/(k-1), 1]$ clamped to `1.0`.
    pub huynh_feldt_eps: f64,
}

/// Sphericity epsilon adjustment applied to repeated-measures two-way *F*-test *p*-values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RmSphericityCorrection {
    /// Uncorrected standard ANOVA *F*-test degrees of freedom assuming exact sphericity ($\varepsilon = 1$).
    #[default]
    None,
    /// Multiply numerator and denominator degrees of freedom by Greenhouse–Geisser $\hat{\varepsilon}$ (Greenhouse & Geisser 1959).
    GreenhouseGeisser,
    /// Multiply numerator and denominator degrees of freedom by Huynh–Feldt / Lecoutre $\tilde{\varepsilon}$ (Huynh & Feldt 1976; Lecoutre 1991).
    HuynhFeldt,
}

/// Per-factor sphericity diagnostics for two-way repeated-measures and split-plot ANOVA.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RmTwoWaySphericities {
    /// Sphericity result for the within-subject Row factor (`None` when between-subject or `< 3` levels).
    pub row: Option<SphericityResult>,
    /// Sphericity result for the within-subject Column factor (`None` when between-subject or `< 3` levels).
    pub column: Option<SphericityResult>,
    /// Sphericity result for the two-factor interaction (`None` when `< 3` combined within-subject contrasts).
    pub interaction: Option<SphericityResult>,
}

/// Two-way repeated-measures / split-plot ANOVA output with factor-specific sphericity diagnostics
/// and sphericity-adjusted *p*-values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RmTwoWayAnovaResult {
    /// Underlying two-way ANOVA table (`terms` ordered `[Row, Column, Interaction]` with *P* values already adjusted by `correction`).
    pub anova: FactorialAnovaResult,
    /// Repeated-measures experimental design (`BothWithin` or split-plot `OneWithinOneBetween`).
    pub design: RepeatedMeasuresDesign,
    /// Per-factor Mauchly, Greenhouse–Geisser, and Huynh–Feldt sphericity diagnostics.
    pub sphericities: RmTwoWaySphericities,
    /// Sphericity correction rule applied to `anova.terms` *P* values.
    pub correction: RmSphericityCorrection,
    /// Uncorrected denominator error degrees of freedom `[df_err_row, df_err_col, df_err_interaction]` before epsilon scaling.
    pub term_error_dfs: [f64; 3],
}

/// Repeated-measures one-way ANOVA output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RmOneWayAnovaResult {
    /// Number of complete matched subjects/blocks (`n_subjects >= 2`).
    pub n_subjects: usize,
    /// Number of matched repeated-measures treatment conditions (`k_conditions >= 2`).
    pub k_conditions: usize,
    /// Treatment (within-subject factor) sum of squares.
    pub ss_treatment: f64,
    /// Inter-subject matching block sum of squares.
    pub ss_subjects: f64,
    /// Residual interaction (subject-by-treatment) error sum of squares.
    pub ss_residual: f64,
    /// Total corrected sum of squares `ss_treatment + ss_subjects + ss_residual`.
    pub ss_total: f64,
    /// Treatment numerator degrees of freedom `k_conditions - 1`.
    pub df_treatment: f64,
    /// Subjects block degrees of freedom `n_subjects - 1`.
    pub df_subjects: f64,
    /// Residual error degrees of freedom `(n_subjects - 1) * (k_conditions - 1)`.
    pub df_residual: f64,
    /// Treatment mean square `ss_treatment / df_treatment`.
    pub ms_treatment: f64,
    /// Subjects mean square `ss_subjects / df_subjects`.
    pub ms_subjects: f64,
    /// Residual error mean square `ss_residual / df_residual`.
    pub ms_residual: f64,
    /// Treatment *F*-statistic `ms_treatment / ms_residual`.
    pub f_treatment: f64,
    /// Uncorrected $F(\mathrm{df\_treatment}, \mathrm{df\_residual})$ *P* value assuming sphericity.
    pub p_uncorrected: f64,
    /// Greenhouse–Geisser $\hat{\varepsilon}$-adjusted treatment *P* value (`f_sf(f_treatment, eps * df1, eps * df2)`).
    pub p_greenhouse_geisser: f64,
    /// Huynh–Feldt $\tilde{\varepsilon}$-adjusted treatment *P* value.
    pub p_huynh_feldt: f64,
    /// Subject matching effectiveness *F*-ratio `ms_subjects / ms_residual`.
    pub f_subjects: f64,
    /// Upper-tail *P* value testing whether subject-level matching explained systematic variance.
    pub p_subjects: f64,
    /// Mauchly sphericity test and $\hat{\varepsilon} / \tilde{\varepsilon}$ estimates.
    pub sphericity: SphericityResult,
    /// Partial eta-squared $\eta_p^2 = \mathrm{SS}_{\text{treatment}} / (\mathrm{SS}_{\text{treatment}} + \mathrm{SS}_{\text{residual}}) \in [0, 1]$.
    pub partial_eta_squared: f64,
}

/// Nonparametric one-way ANOVA output (Kruskal–Wallis; Kruskal & Wallis 1952).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KruskalWallisResult {
    /// Tie-corrected Kruskal–Wallis $H$ rank statistic.
    pub h_statistic: f64,
    /// Chi-square degrees of freedom `k - 1`.
    pub df: f64,
    /// Asymptotic upper-tail $\chi^2(k - 1)$ *P* value testing identical group rank distributions.
    pub p_value: f64,
    /// Multiplicative tie adjustment denominator $1 - \sum (t_r^3 - t_r) / (N^3 - N) \in (0, 1]$.
    pub tie_correction: f64,
    /// Rank-based variance-explained epsilon-squared `H / (N - 1)` clamped to `[0, 1]`.
    pub epsilon_squared: f64,
    /// Sample mean of 1-based mid-ranks within each group (`len == k`).
    pub group_mean_ranks: Vec<f64>,
    /// Total number of observations across all `k` groups.
    pub total_n: usize,
}

/// Nonparametric repeated-measures ANOVA output (Friedman; Friedman 1937).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FriedmanResult {
    /// Tie-corrected Friedman $\chi^2_r$ ($Q$) rank statistic across `n_blocks` matched blocks.
    pub q_statistic: f64,
    /// Asymptotic chi-square degrees of freedom `k_treatments - 1`.
    pub df: f64,
    /// Asymptotic upper-tail $\chi^2(k - 1)$ *P* value testing identical treatment rank distributions across blocks.
    pub p_value: f64,
    /// Kendall's coefficient of concordance $W = Q / (n (k - 1)) \in [0, 1]$.
    pub kendalls_w: f64,
    /// Number of complete matched blocks (subjects/rows).
    pub n_blocks: usize,
    /// Number of repeated treatments/conditions (`k >= 2`).
    pub k_treatments: usize,
    /// Sum of within-block mid-ranks per condition (`len == k_treatments`).
    pub condition_rank_sums: Vec<f64>,
    /// Average within-block mid-rank per condition (`rank_sum / n_blocks`; `len == k_treatments`).
    pub condition_mean_ranks: Vec<f64>,
}

/// Single source-of-variation row in a factorial / general linear ANOVA table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactorialAnovaTerm {
    /// Model term name (e.g. `"Factor A"`, `"Factor B"`, or `"Factor A x Factor B"`).
    pub name: String,
    /// Term sum of squares (Type I sequential or Type III partial according to `SumOfSquaresType`).
    pub ss: f64,
    /// Numerator degrees of freedom associated with this main effect or interaction.
    pub df: f64,
    /// Term mean square `ss / df`.
    pub ms: f64,
    /// *F*-statistic `ms / ms_error` (`None` if denominator unavailable or zero df).
    pub f_statistic: Option<f64>,
    /// Upper-tail *F* *P* value testing this term's marginal/interaction contrasts equal zero (`None` if unestimable).
    pub p_value: Option<f64>,
    /// Partial eta-squared $\eta_p^2 = \mathrm{SS}_{\text{term}} / (\mathrm{SS}_{\text{term}} + \mathrm{SS}_{\text{error}})$ (`None` if unestimable).
    pub partial_eta_squared: Option<f64>,
    /// Unadjusted classical eta-squared $\eta^2 = \mathrm{SS}_{\text{term}} / \mathrm{SS}_{\text{total}}$ (`None` if unestimable).
    pub eta_squared: Option<f64>,
}

/// Factorial (two-way, three-way, or N-way) ANOVA table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactorialAnovaResult {
    /// Main-effect and interaction rows in standard hierarchical order.
    pub terms: Vec<FactorialAnovaTerm>,
    /// Residual error sum of squares.
    pub ss_residual: f64,
    /// Residual error degrees of freedom.
    pub df_residual: f64,
    /// Residual mean square error `ss_residual / df_residual`.
    pub ms_residual: f64,
    /// Total corrected sum of squares around the grand mean.
    pub ss_total: f64,
    /// Total corrected degrees of freedom `total_n - 1`.
    pub df_total: f64,
    /// Total number of non-missing active observations analyzed.
    pub total_n: usize,
    /// True if design cell sample sizes are unequal (unbalanced factorial design).
    pub unbalanced: bool,
    /// Sums-of-squares decomposition convention applied (`TypeI` sequential vs `TypeIII` partial orthogonal contrasts).
    pub ss_type: SumOfSquaresType,
}

const fn default_true() -> bool {
    true
}

/// Linear mixed-effects REML fit for one-way / nested hierarchical or unbalanced RM data
/// (Laird & Ware 1982; Kenward & Roger 1997).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MixedRemlResult {
    /// REML estimate of the random-intercept cluster/subject variance component $\hat{\sigma}_{\text{random}}^2 \ge 0$.
    pub sigma2_random: f64,
    /// REML estimate of the within-cluster residual error variance component $\hat{\sigma}_{\text{error}}^2 > 0$.
    pub sigma2_error: f64,
    /// Intra-class correlation $\mathrm{ICC} = \hat{\sigma}_{\text{random}}^2 / (\hat{\sigma}_{\text{random}}^2 + \hat{\sigma}_{\text{error}}^2) \in [0, 1]$.
    pub icc: f64,
    /// Maximized restricted log-likelihood $\ell_{\text{REML}}$.
    pub reml_log_lik: f64,
    /// Likelihood-ratio chi-square $2(\ell_{\text{REML}} - \ell_{\text{REML,ols}}) \ge 0$ testing significance of the random intercept ($\sigma_{\text{random}}^2 > 0$).
    pub lrt_random_chi2: f64,
    /// Boundary-adjusted / chi-square(1) *P* value testing zero random variance (`lrt_random_chi2`).
    pub lrt_random_p: f64,
    /// Fixed-effect ANOVA table (`*F*`-statistics and *P* values using Kenward–Roger denominator df).
    pub fixed_terms: Vec<FactorialAnovaTerm>,
    /// Satterthwaite (1946) / Fai & Cornelius (1996) denominator df aligned 1-to-1 with `fixed_terms`.
    pub satterthwaite_df: Vec<f64>,
    /// Kenward & Roger (1997) small-sample bias-corrected denominator df aligned 1-to-1 with `fixed_terms`.
    pub kenward_roger_df: Vec<f64>,
    /// Whether the profile likelihood search located a converged REML optimum (false if the variance-ratio optimization hit the upper search ceiling or failed to evaluate).
    #[serde(default = "default_true")]
    pub converged: bool,
    /// Whether the variance-ratio estimate lies on a constraint boundary ($\hat{\sigma}_{\text{random}}^2 = 0$ or upper search ceiling).
    #[serde(default)]
    pub at_boundary: bool,
}

/// Multi-variance-component linear mixed-effects REML fit (`V = sigma_e^2 I + sum_k sigma_k^2 G_k`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiVcRemlResult {
    /// Named random variance component estimates `[("Subject", sigma_s^2), ...]` ($\ge 0$).
    pub variance_components: Vec<(String, f64)>,
    /// REML residual error variance $\hat{\sigma}_e^2 > 0$.
    pub sigma2_error: f64,
    /// Maximized restricted log-likelihood $\ell_{\text{REML}}$.
    pub reml_log_lik: f64,
    /// Fixed-effect Wald/F terms aligned with `satterthwaite_df` and `kenward_roger_df`.
    pub fixed_terms: Vec<FactorialAnovaTerm>,
    /// Fai–Cornelius Satterthwaite denominator df per fixed effect.
    pub satterthwaite_df: Vec<f64>,
    /// Kenward & Roger (1997) adjusted denominator df per fixed effect.
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
    /// Maximized restricted log-likelihood $\ell_{\text{REML}}$ under the unstructured positive-definite $\hat{\Sigma}$.
    pub reml_log_lik: f64,
    /// Fixed-effect ANOVA *F*-terms with Kenward–Roger bias-adjusted *P* values and effect sizes.
    pub fixed_terms: Vec<FactorialAnovaTerm>,
    /// Fai–Cornelius Satterthwaite denominator df aligned with `fixed_terms`.
    pub satterthwaite_df: Vec<f64>,
    /// Kenward & Roger (1997) adjusted denominator df aligned with `fixed_terms`.
    pub kenward_roger_df: Vec<f64>,
    /// True when the Cholesky REML optimizer satisfied gradient/step tolerance without truncation.
    pub converged: bool,
    /// Number of quasi-Newton optimization iterations performed.
    pub iterations: usize,
    /// Optional reason why covariance regularization/shrinkage fallback was engaged (`None` on unregularized interior convergence).
    pub fallback_reason: Option<UnstructuredFallbackReason>,
}

/// Nested one-way ANOVA and optional two-group nested *t*-test output (REML/Satterthwaite/KR).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedAnovaResult {
    /// Ordered top-level group labels (`len == k_groups`).
    pub group_labels: Vec<String>,
    /// Among-groups *F*-statistic accounting for biological subcluster variation.
    pub f_among_groups: f64,
    /// Top-level group numerator degrees of freedom `k_groups - 1`.
    pub df1_groups: f64,
    /// Satterthwaite (1946) effective denominator degrees of freedom for the top-level group comparison.
    pub df2_groups_satterthwaite: f64,
    /// Kenward & Roger (1997) bias-adjusted REML denominator degrees of freedom for top-level groups.
    pub df2_groups_kenward_roger: f64,
    /// Upper-tail *F* *P* value testing equal top-level group means using Kenward–Roger denominator df.
    pub p_among_groups: f64,
    /// Nested *t*-statistic $\sqrt{F}$ signed by group difference when `k_groups == 2` (`None` when `k_groups > 2`).
    pub t_nested: Option<f64>,
    /// Among-subclusters (nested random factor) *F*-statistic `ms_subclusters / ms_error`.
    pub f_among_subclusters: f64,
    /// Numerator degrees of freedom for subclusters nested within groups.
    pub df1_subclusters: f64,
    /// Within-subcluster pure technical replicate error degrees of freedom.
    pub df2_error: f64,
    /// Upper-tail $F(\text{df1\_subclusters}, \text{df2\_error})$ *P* value testing zero subcluster heterogeneity.
    pub p_among_subclusters: f64,
    /// Top-level among-groups sum of squares.
    pub ss_groups: f64,
    /// Among-subclusters nested-within-groups sum of squares.
    pub ss_subclusters: f64,
    /// Pure within-subcluster residual error sum of squares.
    pub ss_error: f64,
    /// Among-groups mean square `ss_groups / df1_groups`.
    pub ms_groups: f64,
    /// Among-subclusters mean square `ss_subclusters / df1_subclusters`.
    pub ms_subclusters: f64,
    /// Pure within-subcluster residual mean square `ss_error / df2_error`.
    pub ms_error: f64,
    /// REML variance component for biological subclusters $\hat{\sigma}_{\text{subcluster}}^2 \ge 0$.
    pub variance_subclusters: f64,
    /// REML pure residual technical error variance component $\hat{\sigma}_{\text{error}}^2 > 0$.
    pub variance_error: f64,
    /// Subcluster intra-class correlation $\hat{\sigma}_{\text{subcluster}}^2 / (\hat{\sigma}_{\text{subcluster}}^2 + \hat{\sigma}_{\text{error}}^2) \in [0, 1]$.
    pub icc_subclusters: f64,
    /// Grand total number of individual observations across all groups and subclusters.
    pub total_observations: usize,
}
