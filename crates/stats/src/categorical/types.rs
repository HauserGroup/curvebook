//! Typed result structures and ordering enums for contingency tables, exact binomial tests,
//! `2 x 2` effect ratios / NNT, Barnard's unconditional test, and inter-rater kappa agreement.
//!
//! References:
//! - Pearson, K. (1900). On the criterion that a given system of deviations from the probable in the
//!   case of a correlated system of variables is such that it can be reasonably supposed to have
//!   arisen from random sampling. *Philosophical Magazine*, 50(302), 157-175.
//!   `doi:10.1080/14786440009463897`.
//! - Wilson, E. B. (1927). Probable inference, the law of succession, and statistical inference.
//!   *Journal of the American Statistical Association*, 22(158), 209-212.
//! - Clopper, C. J., & Pearson, E. S. (1934). The use of confidence or fiducial limits illustrated
//!   in the case of the binomial. *Biometrika*, 26(4), 404-413.
//! - Barnard, G. A. (1947). Significance tests for 2 x 2 tables. *Biometrika*, 34(1-2), 123-138.
//! - Sterne, T. E. (1954). Some remarks on confidence or fiducial limits. *Biometrika*, 41(1-2),
//!   275-278. `doi:10.1093/biomet/41.1-2.275`.
//! - Cohen, J. (1960). A coefficient of agreement for nominal scales. *Educational and Psychological
//!   Measurement*, 20(1), 37-46. `doi:10.1177/001316446002000104`.
//! - Fleiss, J. L. (1971). Measuring nominal scale agreement among many raters. *Psychological
//!   Bulletin*, 76(5), 378-382. `doi:10.1037/h0031619`.
//! - Baptista, J., & Pike, M. C. (1977). Algorithm AS 115: Exact two-sided confidence limits for the
//!   odds ratio in a 2 x 2 table. *Journal of the Royal Statistical Society C*, 26(2), 214-220.
//! - Koopman, P. A. R. (1984). Confidence intervals for the ratio of two binomial proportions.
//!   *Biometrics*, 40(2), 513-517.
//! - Newcombe, R. G. (1998). Interval estimation for the difference between independent proportions:
//!   Comparison of eleven methods. *Statistics in Medicine*, 17(8), 873-890.
//! - Altman, D. G. (1998). Confidence intervals for the number needed to treat. *BMJ*, 317(7168),
//!   1309-1312.

use serde::{Deserialize, Serialize};

/// Contingency association and test output across `R x C` count tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContingencyResult {
    /// Number of contingency rows `R >= 2`.
    pub n_rows: usize,
    /// Number of contingency columns `C >= 2`.
    pub n_cols: usize,
    /// Grand sum of all observed integer cell counts `N`.
    pub total_count: u64,
    /// Pearson chi-square statistic $\sum (O - E)^2 / E$ (Yates continuity-corrected on `2 x 2` when requested).
    pub chi_square: f64,
    /// Chi-square degrees of freedom `(R - 1) * (C - 1)`.
    pub df: f64,
    /// Asymptotic upper-tail $\chi^2(\text{df})$ *P* value (`1.0` when degenerate).
    pub chi_square_p: f64,
    /// Likelihood-ratio $G^2 = 2 \sum O \ln(O / E)$ statistic (Woolf / Williams `G`-test).
    pub g_squared: f64,
    /// Asymptotic upper-tail $\chi^2(\text{df})$ *P* value for $G^2$.
    pub g_squared_p: f64,
    /// Two-sided Fisher–Freeman–Halton exact *P* value (`None` when not evaluated or exact-sum guard suppresses it).
    pub fisher_exact_p: Option<f64>,
    /// Yates continuity-corrected McNemar paired $\chi^2$ on `2 x 2` tables (`None` when `R != 2` or `C != 2`).
    pub mcnemar_chi2: Option<f64>,
    /// Exact binomial two-sided McNemar *P* value on discordant cells on `2 x 2` tables (`None` outside `2 x 2`).
    pub mcnemar_exact_p: Option<f64>,
    /// Cochran–Armitage linear trend standard normal *z* statistic across ordered `2 x C` or `R x 2` proportions (`None` if neither dimension is 2 or variance zero).
    pub cochran_armitage_z: Option<f64>,
    /// Two-sided *P* value for the Cochran–Armitage trend test (`None` when `cochran_armitage_z` is `None`).
    pub cochran_armitage_p: Option<f64>,
    /// Unsigned Phi association coefficient $\sqrt{\chi^2_{\text{uncorr}} / N}$ (`None` outside `2 x 2` tables).
    pub phi: Option<f64>,
    /// Bias-uncorrected Cramér's $V = \sqrt{\chi^2_{\text{uncorr}} / (N \min(R - 1, C - 1))} \in [0, 1]$.
    pub cramers_v: f64,
    /// Two-sided asymptotic confidence interval for Cramér's *V* (`None` when degenerate).
    pub cramers_v_ci: Option<(f64, f64)>,
    /// Minimum expected cell frequency $\min_{i,j} E_{ij}$ under independence (Cochran's rule of 5).
    pub min_expected_count: f64,
    /// Fraction of table cells in `[0, 1]` having expected count `< 5.0`.
    pub cells_lt5_fraction: f64,
    /// Number of empty (all-zero) rows removed prior to testing (`0` when all input rows were active).
    #[serde(default)]
    pub dropped_empty_rows: usize,
    /// Number of empty (all-zero) columns removed prior to testing (`0` when all input columns were active).
    #[serde(default)]
    pub dropped_empty_cols: usize,
}

/// Altman (1998) direction for Number Needed to Treat (`NNTB`, `NNTH`, or `Infinite` when `RD = 0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NntKind {
    /// Positive risk reduction (`NNTB = 1 / |RD|`, Number Needed to Treat to Benefit).
    Benefit,
    /// Negative risk difference / increased adverse event risk (`NNTH = 1 / |RD|`, Number Needed to Treat to Harm).
    Harm,
    /// Zero risk difference (`RD == 0.0` or undefined/non-finite NNT).
    #[default]
    Infinite,
}

/// Complete `2 x 2` risk, odds, and diagnostic accuracy ratios with score/exact CIs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TwoByTwoRatiosResult {
    /// Sample odds ratio `(a * d) / (b * c)` (`None` when `0 / 0`, `Some(f64::INFINITY)` when `a * d > 0` and `b * c == 0`).
    pub odds_ratio: Option<f64>,
    /// Two-sided Baptista–Pike (1977) conditional/exact mid-P / Fisher odds-ratio CI at configured `ConfidenceLevel` (`None` if both ends undefined).
    pub or_baptista_pike_ci: Option<(f64, f64)>,
    /// Sample relative risk / risk ratio `(a / (a + b)) / (c / (c + d))` (`None` when `a == 0 && c == 0`, `Some(f64::INFINITY)` when `a > 0 && c == 0`).
    pub relative_risk: Option<f64>,
    /// Two-sided Koopman (1984) asymptotic score CI for relative risk (`None` if non-estimable).
    pub rr_koopman_ci: Option<(f64, f64)>,
    /// Signed difference of proportions `p1 - p2` in `[-1, 1]`.
    pub risk_difference: f64,
    /// Two-sided Newcombe (1998) Method 10 hybrid Wilson score CI for `risk_difference` (`None` if zero row marginals).
    pub rd_newcombe_ci: Option<(f64, f64)>,
    /// Legacy signed reciprocal `1 / risk_difference` (positive for benefit, negative for harm;
    /// `None` when `|risk_difference| <= 1e-15`; prefer `nnt_kind` and `nnt_value` per Altman 1998).
    pub nnt: Option<f64>,
    /// Altman (1998) treatment direction (`Benefit`, `Harm`, or `Infinite`).
    #[serde(default)]
    pub nnt_kind: NntKind,
    /// Unsigned NNT magnitude `1 / |risk_difference|` when `nnt_kind != NntKind::Infinite`, else `None`.
    #[serde(default)]
    pub nnt_value: Option<f64>,
    /// Formatted point summary (`"NNTB 5.00"`, `"NNTH 4.20"`, or `"Infinity"` per Altman 1998).
    #[serde(default)]
    pub nnt_point_label: String,
    /// Formatted confidence interval string spanning `NNTB`/`Infinity`/`NNTH` when the RD interval crosses zero (Altman 1998).
    #[serde(default)]
    pub nnt_ci_label: String,
    /// Raw ordered numeric interval endpoints mapped from `rd_newcombe_ci` (`None` if RD CI crosses zero or unavailable).
    #[serde(default)]
    pub nnt_ci_bounds: Option<(f64, f64)>,
    /// Diagnostic sensitivity `a / (a + c)` in `[0, 1]` (`None` when column 0 total `a + c == 0`).
    pub sensitivity: Option<f64>,
    /// Two-sided Wilson (1927) score CI for `sensitivity` in `[0, 1]` (`None` when `a + c == 0`).
    pub sensitivity_wilson_ci: Option<(f64, f64)>,
    /// Diagnostic specificity `d / (b + d)` in `[0, 1]` (`None` when column 1 total `b + d == 0`).
    pub specificity: Option<f64>,
    /// Two-sided Wilson (1927) score CI for `specificity` in `[0, 1]` (`None` when `b + d == 0`).
    pub specificity_wilson_ci: Option<(f64, f64)>,
    /// Positive predictive value `a / (a + b)` in `[0, 1]`.
    pub ppv: f64,
    /// Two-sided Wilson (1927) score CI for `ppv` in `[0, 1]`.
    pub ppv_wilson_ci: (f64, f64),
    /// Negative predictive value `d / (c + d)` in `[0, 1]`.
    pub npv: f64,
    /// Two-sided Wilson (1927) score CI for `npv` in `[0, 1]`.
    pub npv_wilson_ci: (f64, f64),
}

/// Detailed McNemar paired-table result including discordant-pair odds ratio and exact Clopper–Pearson CI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McNemarResult {
    /// Count of discordant pairs `(Yes, No)` in upper-right cell.
    pub b: u64,
    /// Count of discordant pairs `(No, Yes)` in lower-left cell.
    pub c: u64,
    /// Yates continuity-corrected McNemar chi-square `(|b - c| - 1)^2 / (b + c)` (`0.0` if `b + c == 0`).
    pub chi_square_yates: f64,
    /// Exact two-sided binomial *P* value testing $H_0: p_{\text{discordant}} = 0.5$ over `b + c` discordant pairs.
    pub exact_p_value: f64,
    /// Discordant odds ratio `b / c` (`None` when `c == 0`).
    pub discordant_odds_ratio: Option<f64>,
    /// Formatted discordant odds ratio string (`"Infinity"` or `"Undefined"` when `c == 0` / `b + c == 0`).
    pub discordant_or_label: String,
    /// Exact two-sided odds-ratio confidence interval `(lower, upper)` inverted from Clopper–Pearson binomial CI on `b / (b + c)` (`None` when unbounded/degenerate).
    pub discordant_or_ci: Option<(f64, f64)>,
    /// Exact two-sided Clopper–Pearson (1934) CI for the discordant proportion `b / (b + c)` in `[0, 1]`.
    pub discordant_p_ci: (f64, f64),
}

/// Exact one-sample binomial test output with Sterne & doubled-tail two-sided *p*-values and
/// Clopper–Pearson, Wilson, and Agresti–Coull confidence intervals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExactBinomialResult {
    /// Observed count of successes `k` (`0 <= k <= total`).
    pub successes: u64,
    /// Total number of Bernoulli trials `n`.
    pub total: u64,
    /// Hypothesized null proportion `p_0` in `(0, 1)` (or `[0, 1]`).
    pub null_proportion: f64,
    /// Observed sample proportion `k / n` in `[0, 1]`.
    pub proportion: f64,
    /// One-sided lower-tail exact binomial *P* value $P(K \le k \mid n, p_0)$.
    pub p_value_lower: f64,
    /// One-sided upper-tail exact binomial *P* value $P(K \ge k \mid n, p_0)$.
    pub p_value_upper: f64,
    /// Sterne (1954) exact two-sided *P* value summing all outcomes with probability `<= P(K = k | n, p_0)`.
    pub p_value_sterne: f64,
    /// Doubled one-sided exact two-sided *P* value `min(1, 2 * min(p_value_lower, p_value_upper))`.
    pub p_value_doubled_tail: f64,
    /// Exact two-sided Clopper–Pearson (1934) binomial CI `(lower, upper)` in `[0, 1]` at configured `ConfidenceLevel`.
    pub clopper_pearson_ci: (f64, f64),
    /// Two-sided Wilson (1927) score binomial CI `(lower, upper)` in `[0, 1]`.
    pub wilson_ci: (f64, f64),
    /// Two-sided Agresti–Coull (1998) adjusted score CI `(lower, upper)` clamped to `[0, 1]`.
    pub agresti_coull_ci: (f64, f64),
}

/// Test-statistic ordering rule used to construct the two-sided rejection region for Barnard's
/// exact unconditional `2 x 2` binomial test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BarnardOrdering {
    /// Suissa & Shuster (1985) pooled-variance score statistic `|Z_pooled|`.
    #[default]
    ScorePooled,
    /// Unpooled Wald statistic `|Z_wald|`.
    WaldUnpooled,
    /// Boschloo (1970) ordering by two-sided Fisher exact *p*-value (`<= p_obs`).
    Boschloo,
    /// Barnard's (1945/1947) Convexity–Symmetry–Maximum (CSM) iterative rejection-region ordering.
    Csm,
}

/// Complete output of Barnard's exact unconditional `2 x 2` binomial test (`[[a, b], [c, d]]`; Barnard 1945/1947).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarnardExactResult {
    /// Observed signed *Z* statistic (`z_wald` for `BarnardOrdering::WaldUnpooled`, or pooled score `z_pooled` for `ScorePooled`, `Boschloo`, and `Csm`; `0.0` on degenerate zero margins or equal proportions).
    pub statistic: f64,
    /// Exact unconditional two-sided *P* value maximized over nuisance parameter $\pi \in [0, 1]$.
    pub p_value: f64,
    /// Maximizing nuisance common baseline proportion $\hat{\pi} \in [0, 1]$ achieving `p_value`.
    pub nuisance_pi: f64,
    /// Test-statistic ordering rule used to rank candidate `2 x 2` tables in the rejection region.
    pub ordering: BarnardOrdering,
    /// True if large cell counts exceeded exact enumeration budget and fell back to pooled score asymptotic *P* value.
    pub used_asymptotic_fallback: bool,
}

/// Inter-rater agreement metrics (Cohen's kappa or Fleiss' kappa; Cohen 1960/1968; Fleiss 1971).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KappaResult {
    /// Chance-corrected inter-rater agreement $\kappa = (P_o - P_e) / (1 - P_e) \le 1$ (`1.0` if $P_o = P_e = 1.0$).
    pub kappa: f64,
    /// Observed proportion (or weighted proportion) of agreement $P_o \in [0, 1]$.
    pub observed_agreement: f64,
    /// Chance-expected agreement $P_e \in [0, 1]$ from rater marginal distributions.
    pub expected_agreement: f64,
    /// Asymptotic standard error $\mathrm{SE}(\hat{\kappa})$ (Fleiss, Cohen & Everitt 1969 for Cohen's kappa; Fleiss et al. 2003 for Fleiss' kappa).
    pub se: f64,
    /// Two-sided normal asymptotic confidence interval $\hat{\kappa} \pm z_{\alpha/2}\mathrm{SE}(\hat{\kappa})$ at configured `ConfidenceLevel`.
    pub ci: (f64, f64),
    /// Null-hypothesis test statistic $\hat{\kappa} / \mathrm{SE}_0$ testing $H_0: \kappa = 0$ (`0.0` when variance under null is zero).
    pub z_statistic: f64,
    /// Two-sided normal *P* value $2 \Phi(-|z|)$ testing no agreement beyond chance ($H_0: \kappa = 0$).
    pub p_value: f64,
}
