//! Survival analysis: Kaplan–Meier curves, Greenwood / log-log confidence bands,
//! Brookmeyer–Crowley median survival CI, Log-rank (Mantel–Cox), Gehan–Breslow–Wilcoxon,
//! log-rank trend, Hazard Ratios, and Cox Proportional Hazards regression (Breslow, Efron, & Exact
//! ties) with Schoenfeld residual proportional-hazards diagnostics.
//!
//! Methods follow Kaplan & Meier (1958), Greenwood (1926), Kalbfleisch & Prentice (2002),
//! Brookmeyer & Crowley (1982), Mantel (1966), Peto & Peto (1972), Gehan (1965), Breslow (1970),
//! Tarone (1975), Berry, Kitchin & Mock (1991), Cox (1972), Breslow (1974), Efron (1977),
//! Gail, Lubin & Rubinstein (1981), Schoenfeld (1982), Therneau, Grambsch & Fleming (1990),
//! Grambsch & Therneau (1994), Harrell et al. (1982), with textbook treatments in Klein &
//! Moeschberger (2003) and Therneau & Grambsch (2000).
//!
//! References:
//! - Kaplan, E. L., & Meier, P. (1958). Nonparametric estimation from incomplete observations.
//!   *Journal of the American Statistical Association*, 53(282), 457-481.
//!   `doi:10.1080/01621459.1958.10501452`.
//! - Greenwood, M. (1926). The natural duration of cancer. *Reports on Public Health and Medical
//!   Subjects*, No. 33. London: HMSO.
//! - Kalbfleisch, J. D., & Prentice, R. L. (2002). *The Statistical Analysis of Failure Time Data*
//!   (2nd ed.). New York: Wiley.
//! - Brookmeyer, R., & Crowley, J. (1982). A confidence interval for the median survival time.
//!   *Biometrics*, 38(1), 29-41. `doi:10.2307/2530286`.
//! - Mantel, N., & Haenszel, W. (1959). Statistical aspects of the analysis of data from
//!   retrospective studies of disease. *Journal of the National Cancer Institute*, 22(4), 719-748.
//! - Mantel, N. (1966). Evaluation of survival data and two new rank order statistics arising in
//!   its consideration. *Cancer Chemotherapy Reports*, 50(3), 163-170.
//! - Greenland, S., & Robins, J. M. (1985). Estimation of a common effect parameter from sparse
//!   follow-up data. *Biometrics*, 41(1), 55-68. `doi:10.2307/2530643`.
//! - Peto, R., & Peto, J. (1972). Asymptotically efficient rank invariant test procedures.
//!   *Journal of the Royal Statistical Society, Series A*, 135(2), 185-207. `doi:10.2307/2344317`.
//! - Gehan, E. A. (1965). A generalized Wilcoxon test for comparing arbitrarily singly-censored
//!   samples. *Biometrika*, 52(1/2). `doi:10.1093/biomet/52.1-2.203`.
//! - Breslow, N. (1970). A generalized Kruskal-Wallis test for comparing K samples subject to
//!   unequal patterns of censorship. *Biometrika*, 57(3), 579-594. `doi:10.1093/biomet/57.3.579`.
//! - Tarone, R. E. (1975). Tests for trend in life table analysis. *Biometrika*, 62(3), 679-682.
//!   `doi:10.1093/biomet/62.3.679`.
//! - Berry, G., Kitchin, R. M., & Mock, P. A. (1991). A comparison of two simple hazard ratio
//!   estimators based on the logrank test. *Statistics in Medicine*, 10(5), 749-755.
//!   `doi:10.1002/sim.4780100510`.
//! - Machin, D., Cheung, Y. B., & Parmar, M. K. B. (2006). *Survival Analysis: A Practical
//!   Approach* (2nd ed.). Chichester: Wiley.
//! - Cox, D. R. (1972). Regression models and life-tables. *Journal of the Royal Statistical
//!   Society, Series B*, 34(2), 187-220. `doi:10.1111/j.2517-6161.1972.tb00899.x`.
//! - Breslow, N. (1974). Covariance analysis of censored survival data. *Biometrics*, 30(1).
//!   `doi:10.2307/2529620`.
//! - Efron, B. (1977). The efficiency of Cox's likelihood function for censored data. *Journal of
//!   the American Statistical Association*, 72(359), 557-565. `doi:10.1080/01621459.1977.10480613`.
//! - Schoenfeld, D. (1982). Partial residuals for the proportional hazards regression model.
//!   *Biometrika*, 69(1), 239-241. `doi:10.1093/biomet/69.1.239`.
//! - Therneau, T. M., Grambsch, P. M., & Fleming, T. R. (1990). Martingale-based residuals for
//!   survival models. *Biometrika*, 77(1), 147-160. `doi:10.1093/biomet/77.1.147`.
//! - Grambsch, P. M., & Therneau, T. M. (1994). Proportional hazards tests and diagnostics based on
//!   weighted residuals. *Biometrika*, 81(3), 515-526. `doi:10.1093/biomet/81.3.515`.
//! - Harrell, F. E., Califf, R. M., Pryor, D. B., Lee, K. L., & Rosati, R. A. (1982). Evaluating
//!   the yield of medical tests. *JAMA*, 247(18), 2543-2546. `doi:10.1001/jama.247.18.2543`.
//! - Klein, J. P., & Moeschberger, M. L. (2003). *Survival Analysis: Techniques for Censored and
//!   Truncated Data* (2nd ed.). New York: Springer.
//! - Therneau, T. M., & Grambsch, P. M. (2000). *Modeling Survival Data: Extending the Cox Model*.
//!   New York: Springer. `doi:10.1007/978-1-4757-3294-8`.

use curvebook_doc::{
    AnalysisSpec, Cell, CellValue, ChecklistStatus, CovariateProfile, CoxTieMethod, CurveSamplePoint, DataSheet, FittedCurveOverlay, FormulaTerm,
    ModelFormulaSpec, MultipleVariablesTable, PValueFormat, PairwiseComparisonRecord, RegressionCiMethod, ResultCell, ResultSheet, ResultTabKind,
    SurvivalAnalysisSpec, SurvivalObservation, SurvivalOtherValuePolicy, SurvivalStatus, TableArchetype, VariableColumn, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{chi2_sf, norm_two_sided_isf, norm_two_sided_p};
use curvebook_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares, sym_eigen};
use curvebook_math::sum::SummaryMoments;
use serde::{Deserialize, Serialize};

use crate::common::{
    KvTabBuilder, TabBuilder, append_ignored_text_checklist_and_footnote, append_listwise_filtered_checklist_and_footnote, checklist_item, ci_cell,
    finite_ci_cell, init_result_sheet, is_ignored_non_numeric_cell, num_cell, opt_ci_cell, opt_num_cell, p_cell_at_alpha, push_scalar,
    resolve_selected_indices, sanitize_result_sheet_floats, text_cell,
};
use crate::cox_ext::{
    compute_cox_snell_rows, compute_cox_vifs, cox_aic_and_aicc_by_events, cox_profile_likelihood_ci, median_survival_ratio_ci,
    parse_survival_status_with_policy, predict_cox_profile_survival,
};
use crate::error::StatsError;
use crate::formula_design::{CategoricalCoding, FormulaPredictorColumn, collect_formula_columns, compile_formula_design, remap_formula_columns};
use crate::multiplicity::{MultiplicityProcedure, adjust_p_values, adjust_sidak};

// ============================================================================
// Typed Outputs
// ============================================================================

/// Single time step on a Kaplan–Meier survival curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KaplanMeierStep {
    /// Distinct observed follow-up time `t >= 0` (`t = 0.0` sentinel step emitted only if present or implied).
    pub time: f64,
    /// Number of subjects still at risk immediately before `time` (`n_i`).
    pub n_at_risk: usize,
    /// Number of uncensored events observed exactly at `time` (`d_i`).
    pub n_events: usize,
    /// Number of subjects right-censored at `time` (removed from risk set after events at `time`).
    pub n_censored: usize,
    /// Kaplan–Meier product-limit survival probability $S(t)$ in `[0, 1]`.
    pub survival: f64,
    /// Greenwood (1926) asymptotic standard error $\mathrm{SE}[S(t)]$ (`0.0` at $S(t) \in \{0, 1\}$).
    pub greenwood_se: f64,
    /// Lower endpoint of pointwise confidence interval in `[0, 1]` at configured `ConfidenceLevel` (log-log Kalbfleisch–Prentice or linear plain).
    pub ci_lower: f64,
    /// Upper endpoint of pointwise confidence interval in `[0, 1]` at configured `ConfidenceLevel`.
    pub ci_upper: f64,
}

/// Complete Kaplan–Meier summary for a single cohort.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KaplanMeierCohortResult {
    /// Cohort / group display label.
    pub label: String,
    /// Total number of subjects in the cohort (`n_events + n_censored + still at risk`).
    pub n_total: usize,
    /// Total number of uncensored failure events observed in this cohort.
    pub n_events: usize,
    /// Total number of right-censored observations in this cohort.
    pub n_censored: usize,
    /// Estimated median survival time where $S(t)$ reaches/crosses `0.5` (`None` if survival never drops to `<= 0.5`).
    pub median_survival: Option<f64>,
    /// Two-sided Brookmeyer–Crowley (1982) confidence interval `(lower, upper)` for median survival (`None` endpoints when median or upper/lower crossing is not reached).
    pub median_ci: (Option<f64>, Option<f64>),
    /// Chronologically ordered event/censoring steps across distinct follow-up times.
    pub steps: Vec<KaplanMeierStep>,
}

/// Two-group hazard ratio summary from observed/expected (log-rank/Pike), Mantel–Haenszel
/// common hazard ratio (Berry, Kitchin & Mock 1991; Greenland & Robins 1985), and Peto's
/// one-step estimator (Berry, Kitchin & Mock 1991).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HazardRatioResult {
    /// Ratio of observed-to-expected rates `(O_A / E_A) / (O_B / E_B)` (`f64::NAN` if `E_A <= 0`,
    /// `E_B <= 0`, or `O_B <= 0`).
    pub log_rank_hr: f64,
    /// Confidence interval for `log_rank_hr` via `SE(ln HR_OE) = sqrt(1 / E_A + 1 / E_B)`; `None`
    /// when `log_rank_hr` is non-estimable or `O_A <= 0`.
    pub log_rank_ci: Option<(f64, f64)>,
    /// Genuine Mantel–Haenszel common hazard/rate ratio `sum(d_Aj * n_Bj / n_j) /
    /// sum(d_Bj * n_Aj / n_j)` (`f64::NAN` if the denominator sum `S <= 0`).
    pub mantel_haenszel_hr: f64,
    /// Confidence interval for `mantel_haenszel_hr` via Greenland & Robins (1985) variance
    /// `sum(d_j * n_Aj * n_Bj / n_j^2) / (R * S)`; `None` when non-estimable (`R <= 0` or `S <= 0`).
    pub mantel_haenszel_ci: Option<(f64, f64)>,
    /// Peto one-step log-rank hazard ratio `exp((O_A - E_A) / V)` (`f64::NAN` when `V <= 0`).
    pub peto_one_step_hr: f64,
    /// Confidence interval for `peto_one_step_hr` with `SE(ln HR) = 1 / sqrt(V)`; `None` when `V <= 0`.
    pub peto_one_step_ci: Option<(f64, f64)>,
}

impl HazardRatioResult {
    /// Returns `Some(hr)` when `log_rank_hr` is finite and estimable, else `None`.
    pub fn log_rank_hr_opt(&self) -> Option<f64> {
        self.log_rank_hr.is_finite().then_some(self.log_rank_hr)
    }

    /// Returns `Some(hr)` when `mantel_haenszel_hr` is finite and estimable, else `None`.
    pub fn mantel_haenszel_hr_opt(&self) -> Option<f64> {
        self.mantel_haenszel_hr.is_finite().then_some(self.mantel_haenszel_hr)
    }

    /// Returns `Some(hr)` when `peto_one_step_hr` is finite and estimable, else `None`.
    pub fn peto_one_step_hr_opt(&self) -> Option<f64> {
        self.peto_one_step_hr.is_finite().then_some(self.peto_one_step_hr)
    }
}

/// Omnibus multi-cohort survival curve comparison test result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurvivalComparisonResult {
    /// Comparison test name (e.g. `"Log-rank (Mantel-Cox)"`, `"Gehan-Breslow-Wilcoxon"`, `"Log-rank test for trend"`).
    pub test_name: String,
    /// Chi-square test statistic ($\ge 0$, or `f64::NAN` when non-estimable due to zero events or zero covariance rank).
    pub chi_square: f64,
    /// Asymptotic chi-square degrees of freedom from covariance matrix rank (`<= k - 1` for homogeneity, `1` for log-rank trend; `f64::NAN` when non-estimable).
    pub df: f64,
    /// Upper-tail $\chi^2(\text{df})$ *P* value testing identical survival curves across cohorts (`f64::NAN` when non-estimable).
    pub p_value: f64,
}

/// Single covariate estimate from Cox Proportional Hazards regression (Cox 1972).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoxCovariateResult {
    /// Predictor column or encoded dummy-level term label.
    pub name: String,
    /// Maximum partial-likelihood log-hazard regression coefficient $\hat{\beta}$.
    pub beta: f64,
    /// Asymptotic standard error $\sqrt{I(\hat{\beta})^{-1}_{jj}}$ from the observed partial-likelihood information matrix.
    pub se: f64,
    /// Wald test statistic $\hat{\beta} / \mathrm{SE}(\hat{\beta})$.
    pub z_statistic: f64,
    /// Two-sided Wald *P* value $2 \Phi(-|z|)$ testing $H_0: \beta_j = 0$.
    pub p_value: f64,
    /// Estimated hazard ratio $\exp(\hat{\beta}) > 0$ per 1-unit increase in the covariate.
    pub hazard_ratio: f64,
    /// Two-sided Wald confidence interval $\exp(\hat{\beta} \pm z_{\alpha/2}\mathrm{SE})$ for `hazard_ratio` at configured `ConfidenceLevel`.
    pub hr_ci: (f64, f64),
    /// Grambsch–Therneau (1994) Pearson correlation `rho` between scaled Schoenfeld residuals and event times (`0.0` if degenerate).
    pub schoenfeld_rho: f64,
    /// Two-sided proportional-hazards diagnostic *P* value testing $H_0: \rho = 0$ (`1.0` when `< 3` events).
    pub schoenfeld_p: f64,
}

/// Single time-indexed baseline cumulative hazard and survival point from a Cox PH fit
/// (evaluated at the sample mean covariates). `cum_hazard` is the Breslow (1974) estimator
/// (Efron-adjusted under Efron ties) and `survival = exp(-cum_hazard)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoxBaselinePoint {
    /// Event follow-up time `t >= 0` (`t = 0.0` initial anchor plus distinct observed event times).
    pub time: f64,
    /// Baseline cumulative hazard $H_0(t)$ evaluated at sample-mean covariates.
    pub cum_hazard: f64,
    /// Baseline survival probability $S_0(t) = \exp(-H_0(t)) \in [0, 1]$ at sample-mean covariates.
    pub survival: f64,
}

/// Complete Cox Proportional Hazards regression output (Cox 1972; Therneau & Grambsch 2000).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoxPhResult {
    /// Number of complete subjects included in the partial likelihood fit.
    pub n_subjects: usize,
    /// Total count of uncensored failure events across `n_subjects`.
    pub n_events: usize,
    /// Partial-likelihood tied-event-time handling method (`Breslow`, `Efron`, or `Exact`).
    pub tie_method: CoxTieMethod,
    /// True if Newton–Raphson step-halving converged within gradient/step tolerances.
    pub converged: bool,
    /// Number of Newton–Raphson iterations executed.
    pub iterations: usize,
    /// Null partial log-likelihood $\ell(0)$ at zero coefficients.
    pub log_lik_null: f64,
    /// Fitted partial log-likelihood $\ell(\hat{\beta})$ at convergence.
    pub log_lik_model: f64,
    /// Harrell's *C*-index concordance probability in `[0, 1]` over comparable subject pairs (Harrell et al. 1982);
    /// `None` when `comparable_pairs == 0` (for example when all events occur at identical times).
    pub concordance: Option<f64>,
    /// Total number of non-tied comparable pairs contributing to Harrell's *C*-index denominator (`0` makes `concordance` `None`).
    #[serde(default)]
    pub comparable_pairs: usize,
    /// Omnibus Likelihood Ratio $\chi^2 = 2(\ell(\hat{\beta}) - \ell(0))$ on `covariates.len()` degrees of freedom.
    pub likelihood_ratio_chi2: f64,
    /// Upper-tail $\chi^2(p)$ *P* value for the omnibus likelihood-ratio test.
    pub likelihood_ratio_p: f64,
    /// Omnibus Wald $\chi^2 = \hat{\beta}^\top I(\hat{\beta}) \hat{\beta}$ on `p` degrees of freedom.
    pub wald_chi2: f64,
    /// Upper-tail $\chi^2(p)$ *P* value for the omnibus Wald test.
    pub wald_p: f64,
    /// Omnibus Score (log-rank multivariate) $\chi^2 = U(0)^\top I(0)^{-1} U(0)$ on `p` degrees of freedom.
    pub score_chi2: f64,
    /// Upper-tail $\chi^2(p)$ *P* value for the omnibus Score test.
    pub score_p: f64,
    /// Global Grambsch–Therneau (1994) scaled Schoenfeld proportional-hazards $\chi^2$ test statistic.
    pub global_schoenfeld_chi2: f64,
    /// Degrees of freedom for the global proportional-hazards Schoenfeld test (`p`).
    pub global_schoenfeld_df: f64,
    /// Upper-tail $\chi^2(p)$ *P* value testing global proportional hazards (`p < 0.05` warns of non-proportionality).
    pub global_schoenfeld_p: f64,
    /// Ordered per-covariate coefficient, hazard ratio, confidence interval, and Schoenfeld check results.
    pub covariates: Vec<CoxCovariateResult>,
    /// Per-subject Martingale residuals $M_i = \delta_i - \hat{H}(t_i | X_i)$ in input subject order.
    pub martingale_residuals: Vec<f64>,
    /// Per-subject symmetric deviance residuals derived from `martingale_residuals` (Therneau & Grambsch 2000).
    pub deviance_residuals: Vec<f64>,
    /// Estimated baseline cumulative hazard and survival trajectory evaluated at sample-mean covariates.
    pub baseline_survival: Vec<CoxBaselinePoint>,
    /// Inverse Fisher information covariance matrix `Cov(beta_hat)` (`p x p`).
    #[serde(default)]
    pub cov_beta: Vec<Vec<f64>>,
    /// Parameter correlation matrix `Corr(beta_hat)` derived from `cov_beta` (`p x p`).
    #[serde(default)]
    pub correlation_matrix: Vec<Vec<f64>>,
}

// ============================================================================
// Kaplan–Meier, Brookmeyer–Crowley Median CI, Log-Rank & Wilcoxon
// ============================================================================

#[inline]
fn normalize_zero_time(t: f64) -> f64 {
    if t == 0.0 { 0.0 } else { t }
}

/// Compute Kaplan–Meier survival curve, Greenwood SE, pointwise CIs (asymmetrical log-log or linear),
/// and Brookmeyer–Crowley median survival CI.
///
/// - Survival: Kaplan & Meier (1958) product-limit estimator `S(t) = prod_{t_i <= t} (1 - d_i / n_i)`
///   over distinct event times. At a tied time, events are counted before censorings (censored
///   subjects remain in the risk set `n_i`), the usual product-limit convention.
/// - Standard error: Greenwood (1926) variance `Var[S(t)] = S(t)^2 * sum d_i / (n_i (n_i - d_i))`.
///   Terms with `n_i = d_i` (where `S` drops to 0) are omitted from the sum.
/// - Pointwise CI: either the linear ("plain") interval `S +/- z * SE`, clamped to `[0, 1]`, or the
///   log-log interval `S^exp(+/- z * se_ll)` with `se_ll = sqrt(greenwood_sum) / |ln S|`
///   (Kalbfleisch & Prentice 2002; Klein & Moeschberger 2003). No arcsine-square-root or plain
///   `log` transform is implemented.
/// - Median: the first time `S(t) < 0.5`. If `S(t)` equals 0.5 exactly over an interval, the
///   midpoint of that interval and the next event time is used.
/// - Median CI: Brookmeyer & Crowley (1982) test inversion; see `brookmeyer_crowley_median_ci`.
///
/// Full references are listed in the module header.
pub fn kaplan_meier_curve(
    label: &str,
    observations: &[SurvivalObservation],
    asymmetrical_log_log_ci: bool,
    confidence: ConfidenceLevel,
) -> Result<KaplanMeierCohortResult, StatsError> {
    if observations.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    for obs in observations {
        if !obs.time.is_finite() || obs.time < 0.0 {
            return Err(StatsError::InvalidParameter { name: "survival_time", value: obs.time });
        }
    }
    let mut sorted: Vec<SurvivalObservation> = observations
        .iter()
        .map(|o| {
            let mut copy = *o;
            copy.time = normalize_zero_time(copy.time);
            copy
        })
        .collect();
    sorted.sort_by(|a, b| a.time.total_cmp(&b.time));

    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let n_total = sorted.len();
    let mut n_at_risk = n_total;
    let mut n_events_total = 0_usize;
    let mut n_censored_total = 0_usize;
    let mut surv = 1.0_f64;
    let mut greenwood_sum = 0.0_f64;
    let mut steps = Vec::new();

    let mut i = 0_usize;
    while i < sorted.len() {
        let t = sorted[i].time;
        let mut d_i = 0_usize;
        let mut c_i = 0_usize;
        while i < sorted.len() && sorted[i].time.total_cmp(&t).is_eq() {
            match sorted[i].status {
                SurvivalStatus::Event => d_i += 1,
                SurvivalStatus::Censored => c_i += 1,
            }
            i += 1;
        }
        n_events_total += d_i;
        n_censored_total += c_i;
        let at_risk_before = n_at_risk;
        if d_i > 0 && at_risk_before > 0 {
            let nf = at_risk_before as f64;
            let df = d_i as f64;
            surv *= 1.0 - df / nf;
            if at_risk_before > d_i {
                greenwood_sum += df / (nf * (nf - df));
            }
        }
        let se = surv * greenwood_sum.sqrt();
        let (ci_lo, ci_hi) = km_pointwise_ci(surv, se, greenwood_sum, z_crit, asymmetrical_log_log_ci);
        steps.push(KaplanMeierStep {
            time: t,
            n_at_risk: at_risk_before,
            n_events: d_i,
            n_censored: c_i,
            survival: surv,
            greenwood_se: se,
            ci_lower: ci_lo,
            ci_upper: ci_hi,
        });
        n_at_risk = n_at_risk.saturating_sub(d_i + c_i);
    }

    let median_survival = km_quantile_time(&steps, 0.5);
    let median_ci = brookmeyer_crowley_median_ci(&steps, z_crit, asymmetrical_log_log_ci);

    Ok(KaplanMeierCohortResult {
        label: label.to_owned(),
        n_total,
        n_events: n_events_total,
        n_censored: n_censored_total,
        median_survival,
        median_ci,
        steps,
    })
}

fn km_pointwise_ci(surv: f64, se: f64, greenwood_sum: f64, z_crit: f64, log_log: bool) -> (f64, f64) {
    if surv <= 0.0 {
        return (0.0, 0.0);
    }
    if surv >= 1.0 {
        return (1.0, 1.0);
    }
    if log_log {
        let ln_s = surv.ln();
        if ln_s.abs() <= 1e-15 {
            return (1.0, 1.0);
        }
        let se_ll = greenwood_sum.sqrt() / ln_s.abs();
        let lnl_s = (-ln_s).ln();
        let lo = (-(lnl_s + z_crit * se_ll).exp()).exp().clamp(0.0, 1.0);
        let hi = (-(lnl_s - z_crit * se_ll).exp()).exp().clamp(0.0, 1.0);
        (lo, hi)
    } else {
        ((surv - z_crit * se).clamp(0.0, 1.0), (surv + z_crit * se).clamp(0.0, 1.0))
    }
}

fn km_quantile_time(steps: &[KaplanMeierStep], target_s: f64) -> Option<f64> {
    let mut prev_t = None;
    for s in steps {
        if (s.survival - target_s).abs() <= 1e-12 {
            if prev_t.is_none() {
                prev_t = Some(s.time);
            }
        } else if s.survival < target_s {
            return Some(match prev_t {
                Some(t0) => 0.5 * (t0 + s.time),
                None => s.time,
            });
        }
    }
    prev_t
}

/// Brookmeyer & Crowley (1982) confidence interval for median survival, obtained by inverting the
/// pointwise Greenwood test around `S(t) = 0.5` at critical value `z_crit`: the lower limit is the
/// first finite-variance step whose lower bound reaches `<= 0.5`, and the upper limit is the first
/// finite-variance step whose upper bound falls below `0.5` (open-ended `None` if it never does).
///
/// Steps where `S(t) <= 0.0` have indeterminate Greenwood variance (`0 * inf` due to `n_j = d_j`)
/// and are excluded from finite interval inversion, matching standard Greenwood quantile behavior.
/// With `asymmetrical_log_log_ci == false` this is the linear ("plain") Brookmeyer & Crowley (1982)
/// test inversion; with `asymmetrical_log_log_ci == true` it is the log-log transformed inversion of
/// Klein & Moeschberger (2003). Full references are listed in the module header.
fn brookmeyer_crowley_median_ci(steps: &[KaplanMeierStep], z_crit: f64, asymmetrical_log_log_ci: bool) -> (Option<f64>, Option<f64>) {
    if !z_crit.is_finite() || z_crit <= 0.0 {
        return (None, None);
    }
    let mut lo_time = None;
    let mut hi_time = None;
    for s in steps {
        if s.survival <= 0.0 || s.survival >= 1.0 || !s.greenwood_se.is_finite() {
            continue;
        }
        let greenwood_sum = (s.greenwood_se / s.survival).powi(2);
        let (ci_lo, ci_hi) = km_pointwise_ci(s.survival, s.greenwood_se, greenwood_sum, z_crit, asymmetrical_log_log_ci);
        if lo_time.is_none() && ci_lo <= 0.5 + 1e-12 {
            lo_time = Some(s.time);
        }
        if hi_time.is_none() && ci_hi < 0.5 - 1e-12 {
            hi_time = Some(s.time);
        }
        if lo_time.is_some() && hi_time.is_some() {
            break;
        }
    }
    (lo_time, hi_time)
}

#[derive(Debug, Clone, Copy)]
enum SurvivalWeight {
    LogRank,
    GehanWilcoxon,
}

fn unavailable_survival_comparison(test_name: &str) -> SurvivalComparisonResult {
    SurvivalComparisonResult { test_name: test_name.to_owned(), chi_square: f64::NAN, df: f64::NAN, p_value: f64::NAN }
}

/// Multi-cohort log-rank (Mantel–Cox) or Gehan–Breslow–Wilcoxon chi-square test using exact
/// hypergeometric variance covariance matrix over distinct event times.
///
/// Both tests are members of the weighted log-rank class: `U_g = sum_j w_j (d_gj - n_gj d_j / n_j)`
/// with hypergeometric covariance `V_gh = sum_j w_j^2 d_j (n_j - d_j) / (n_j (n_j - 1)) * n_gj
/// (delta_gh - n_hj / n_j)`, and `chi^2 = U' V^- U` on effective covariance rank `<= K - 1` degrees of freedom
/// (Klein & Moeschberger 2003).
/// - `w_j = 1`: log-rank test (Mantel 1966; Peto & Peto 1972).
/// - `w_j = n_j` (number at risk): Gehan (1965) generalized Wilcoxon test, extended to `K` samples by
///   Breslow (1970). The variance is the conditional hypergeometric weighted log-rank variance,
///   not Gehan's original two-sample permutation variance, so results can differ slightly from
///   Gehan (1965) in small samples.
///
/// Full references are listed in the module header.
pub fn compare_survival_curves(cohorts: &[&[SurvivalObservation]], gehan_wilcoxon: bool) -> Result<SurvivalComparisonResult, StatsError> {
    let k = cohorts.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "survival curve comparison", required: 2, actual: k });
    }
    let test_name = if gehan_wilcoxon { "Gehan-Breslow-Wilcoxon test" } else { "Log-rank (Mantel-Cox) test" };
    let weight_kind = if gehan_wilcoxon { SurvivalWeight::GehanWilcoxon } else { SurvivalWeight::LogRank };
    let (u, cov, obs_tot, exp_tot) = weighted_log_rank_stats(cohorts, weight_kind)?;

    if obs_tot.iter().sum::<f64>() <= 0.0 {
        return Ok(unavailable_survival_comparison(test_name));
    }

    // Drop groups with zero expected events (e.g. completely censored before the first event)
    // before forming the reduced covariance and computing its effective rank df.
    let active: Vec<usize> = (0..k).filter(|&g| exp_tot[g] > 0.0).collect();
    if active.len() < 2 {
        return Ok(unavailable_survival_comparison(test_name));
    }
    let m = active.len() - 1;
    let mut v_red = vec![0.0_f64; m * m];
    let mut u_red = Vec::with_capacity(m);
    for (ri, &gi) in active.iter().take(m).enumerate() {
        u_red.push(u[gi]);
        for (cj, &gj) in active.iter().take(m).enumerate() {
            v_red[ri * m + cj] = cov[gi * k + gj];
        }
    }
    let max_diag = (0..m).map(|i| v_red[i * m + i].abs()).fold(0.0_f64, f64::max);
    if !max_diag.is_finite() || max_diag <= 0.0 {
        return Ok(unavailable_survival_comparison(test_name));
    }

    let v_mat = DenseMatrix::from_row_major(m, m, v_red)?;
    let (chi2, rank) = if let Ok(chol) = cholesky(&v_mat)
        && (0..m).all(|i| {
            let piv = chol.l().get(i, i).unwrap_or(0.0);
            piv.is_finite() && piv * piv > 1.0e-12 * max_diag
        }) {
        let sol = chol.solve(&u_red)?;
        let qf = u_red.iter().zip(sol.iter()).map(|(a, b)| a * b).sum::<f64>().max(0.0);
        (qf, m)
    } else {
        let eig = sym_eigen(&v_mat)?;
        let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
        if !max_ev.is_finite() || max_ev <= 0.0 {
            return Ok(unavailable_survival_comparison(test_name));
        }
        let tol = 1.0e-11 * max_ev;
        let mut qf = 0.0_f64;
        let mut eff_rank = 0_usize;
        for (col, &ev) in eig.eigenvalues.iter().enumerate() {
            if ev > tol {
                let mut proj = 0.0_f64;
                for (row, &ur) in u_red.iter().enumerate() {
                    proj += eig.eigenvectors.get(row, col).unwrap_or(0.0) * ur;
                }
                qf += (proj * proj) / ev;
                eff_rank += 1;
            }
        }
        (qf.max(0.0), eff_rank)
    };

    if rank == 0 || !chi2.is_finite() {
        return Ok(unavailable_survival_comparison(test_name));
    }
    let df = rank as f64;
    let p_value = chi2_sf(chi2, df)?;
    Ok(SurvivalComparisonResult { test_name: test_name.to_owned(), chi_square: chi2, df, p_value })
}

/// Log-rank test for linear trend across `K >= 2` ordered cohorts (Tarone 1975).
///
/// Uses the log-rank score vector `U` and hypergeometric covariance `V` from
/// `compare_survival_curves` with equally spaced scores `s = (1, ..., K)` in cohort order:
/// `chi^2 = (s' U)^2 / (s' V s)` on 1 degree of freedom (Tarone 1975; Klein & Moeschberger 2003).
/// Full references are listed in the module header.
pub fn log_rank_trend_test(cohorts: &[&[SurvivalObservation]]) -> Result<SurvivalComparisonResult, StatsError> {
    let k = cohorts.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "survival curve comparison", required: 2, actual: k });
    }
    let test_name = "Log-rank test for trend";
    let (u, cov, obs_tot, _) = weighted_log_rank_stats(cohorts, SurvivalWeight::LogRank)?;
    if obs_tot.iter().sum::<f64>() <= 0.0 {
        return Ok(unavailable_survival_comparison(test_name));
    }
    let scores: Vec<f64> = (1..=k).map(|i| i as f64).collect();
    let num: f64 = scores.iter().zip(u.iter()).map(|(s, ui)| s * ui).sum();
    let mut denom = 0.0_f64;
    let mut max_cov = 0.0_f64;
    for i in 0..k {
        for j in 0..k {
            let v_ij = cov[i * k + j];
            max_cov = max_cov.max(v_ij.abs());
            denom += scores[i] * v_ij * scores[j];
        }
    }
    let score_norm_sq: f64 = scores.iter().map(|s| s * s).sum();
    if !denom.is_finite() || !max_cov.is_finite() || max_cov <= 0.0 || denom <= 1.0e-12 * max_cov * score_norm_sq {
        return Ok(unavailable_survival_comparison(test_name));
    }
    let chi2 = ((num * num) / denom).max(0.0);
    let p_value = chi2_sf(chi2, 1.0)?;
    Ok(SurvivalComparisonResult { test_name: String::from(test_name), chi_square: chi2, df: 1.0, p_value })
}

/// Observed/expected, genuine Mantel–Haenszel, and Peto one-step Hazard Ratio estimates with
/// `confidence` intervals for two cohorts (cohort A relative to cohort B).
///
/// Over distinct event times `t_j` with risk-set sizes `n_Aj, n_Bj` (`n_j = n_Aj + n_Bj`) and event
/// counts `d_Aj, d_Bj` (`d_j = d_Aj + d_Bj`), log-rank statistics are `O_g = sum d_gj`,
/// `E_g = sum d_j * n_gj / n_j`, and hypergeometric variance `V = sum d_j (n_j - d_j) / (n_j (n_j - 1)) * n_Aj n_Bj / n_j`:
/// - `log_rank_hr`: Pike / observed-to-expected rate ratio `(O_A / E_A) / (O_B / E_B)` with Wald
///   CI `exp(ln HR +/- z * sqrt(1 / E_A + 1 / E_B))` (Machin, Cheung & Parmar 2006; Berry, Kitchin
///   & Mock 1991). Non-estimable (`f64::NAN`, `log_rank_ci: None`) when `E_A <= 0`, `E_B <= 0`, or
///   `O_B <= 0`; `log_rank_ci` is also `None` when `O_A <= 0`.
/// - `mantel_haenszel_hr`: genuine Mantel–Haenszel common hazard/rate ratio `R / S` where
///   `R = sum d_Aj n_Bj / n_j` and `S = sum d_Bj n_Aj / n_j` (Mantel & Haenszel 1959; Berry,
///   Kitchin & Mock 1991, eq. 3), with Greenland & Robins (1985, eq. 5) sparse-data variance
///   `Var(ln HR_MH) = (sum d_j n_Aj n_Bj / n_j^2) / (R * S)` and CI `None` when `R <= 0` or `S <= 0`.
/// - `peto_one_step_hr`: Peto's one-step estimator `exp((O_A - E_A) / V)` with `SE = 1 / sqrt(V)`
///   (Berry, Kitchin & Mock 1991; Machin, Cheung & Parmar 2006); non-estimable when `V <= 0`.
///
/// Full references are listed in the module header.
pub fn two_cohort_hazard_ratio(
    cohort_a: &[SurvivalObservation],
    cohort_b: &[SurvivalObservation],
    confidence: ConfidenceLevel,
) -> Result<HazardRatioResult, StatsError> {
    let (_, cov, obs, exp) = weighted_log_rank_stats(&[cohort_a, cohort_b], SurvivalWeight::LogRank)?;
    let (r_mh, s_mh, gr_num) = mh_strata_sums(cohort_a, cohort_b);
    let o1 = obs[0];
    let o2 = obs[1];
    let e1 = exp[0];
    let e2 = exp[1];
    let v = cov[0];
    let z_crit = norm_two_sided_isf(confidence.alpha())?;

    let (lr_hr, lr_ci) = if e1 > 0.0 && e2 > 0.0 && o2 > 0.0 {
        let hr = (o1 / e1) / (o2 / e2);
        let se_oe = (1.0 / e1 + 1.0 / e2).sqrt();
        (hr, log_scale_hr_ci(hr, se_oe, z_crit))
    } else {
        (f64::NAN, None)
    };

    let (mh_hr, mh_ci) = if s_mh > 0.0 {
        let hr = r_mh / s_mh;
        let se_mh = if r_mh > 0.0 && gr_num > 0.0 { (gr_num / (r_mh * s_mh)).sqrt() } else { f64::NAN };
        (hr, log_scale_hr_ci(hr, se_mh, z_crit))
    } else {
        (f64::NAN, None)
    };

    let (peto_hr, peto_ci) = if v > 0.0 {
        let log_peto = (o1 - e1) / v;
        let se_peto = (1.0 / v).sqrt();
        let hr = log_peto.exp();
        let ci = ((log_peto - z_crit * se_peto).exp(), (log_peto + z_crit * se_peto).exp());
        (hr, (ci.0.is_finite() && ci.1.is_finite()).then_some(ci))
    } else {
        (f64::NAN, None)
    };

    Ok(HazardRatioResult {
        log_rank_hr: lr_hr,
        log_rank_ci: lr_ci,
        mantel_haenszel_hr: mh_hr,
        mantel_haenszel_ci: mh_ci,
        peto_one_step_hr: peto_hr,
        peto_one_step_ci: peto_ci,
    })
}

fn log_scale_hr_ci(hr: f64, se_log: f64, z_crit: f64) -> Option<(f64, f64)> {
    if !hr.is_finite() || hr <= 0.0 || !se_log.is_finite() || se_log <= 0.0 {
        return None;
    }
    let ln_hr = hr.ln();
    let lo = (ln_hr - z_crit * se_log).exp();
    let hi = (ln_hr + z_crit * se_log).exp();
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}

fn mh_strata_sums(cohort_a: &[SurvivalObservation], cohort_b: &[SurvivalObservation]) -> (f64, f64, f64) {
    let mut recs: Vec<(f64, usize, SurvivalStatus)> = Vec::with_capacity(cohort_a.len() + cohort_b.len());
    for o in cohort_a {
        recs.push((normalize_zero_time(o.time), 0, o.status));
    }
    for o in cohort_b {
        recs.push((normalize_zero_time(o.time), 1, o.status));
    }
    recs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut at_risk = [cohort_a.len(), cohort_b.len()];
    let mut r_sum = 0.0_f64;
    let mut s_sum = 0.0_f64;
    let mut gr_num = 0.0_f64;
    let mut idx = 0_usize;
    while idx < recs.len() {
        let t = recs[idx].0;
        let mut d = [0_usize; 2];
        let mut c = [0_usize; 2];
        while idx < recs.len() && recs[idx].0 == t {
            let (_, g, st) = recs[idx];
            match st {
                SurvivalStatus::Event => d[g] += 1,
                SurvivalStatus::Censored => c[g] += 1,
            }
            idx += 1;
        }
        let d_tot = d[0] + d[1];
        let n_tot = at_risk[0] + at_risk[1];
        if d_tot > 0 && n_tot > 0 {
            let nf = n_tot as f64;
            let na = at_risk[0] as f64;
            let nb = at_risk[1] as f64;
            r_sum += (d[0] as f64) * nb / nf;
            s_sum += (d[1] as f64) * na / nf;
            gr_num += (d_tot as f64) * na * nb / (nf * nf);
        }
        at_risk[0] = at_risk[0].saturating_sub(d[0] + c[0]);
        at_risk[1] = at_risk[1].saturating_sub(d[1] + c[1]);
    }
    (r_sum, s_sum, gr_num)
}

type WeightedLogRankStats = (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>);

fn weighted_log_rank_stats(cohorts: &[&[SurvivalObservation]], weight_kind: SurvivalWeight) -> Result<WeightedLogRankStats, StatsError> {
    let k = cohorts.len();
    let mut all_recs: Vec<(f64, usize, SurvivalStatus)> = Vec::new();
    let mut at_risk = vec![0_usize; k];
    for (g, obs) in cohorts.iter().enumerate() {
        if obs.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        at_risk[g] = obs.len();
        for o in *obs {
            if !o.time.is_finite() || o.time < 0.0 {
                return Err(StatsError::InvalidParameter { name: "survival_time", value: o.time });
            }
            all_recs.push((normalize_zero_time(o.time), g, o.status));
        }
    }
    all_recs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut u = vec![0.0_f64; k];
    let mut obs_tot = vec![0.0_f64; k];
    let mut exp_tot = vec![0.0_f64; k];
    let mut cov = vec![0.0_f64; k * k];

    let mut idx = 0_usize;
    while idx < all_recs.len() {
        let t = all_recs[idx].0;
        let mut d_g = vec![0_usize; k];
        let mut c_g = vec![0_usize; k];
        while idx < all_recs.len() && all_recs[idx].0 == t {
            let (_, g, st) = all_recs[idx];
            match st {
                SurvivalStatus::Event => d_g[g] += 1,
                SurvivalStatus::Censored => c_g[g] += 1,
            }
            idx += 1;
        }
        let d_total: usize = d_g.iter().sum();
        let n_total: usize = at_risk.iter().sum();

        if d_total > 0 && n_total > 0 {
            let nf = n_total as f64;
            let df = d_total as f64;
            let w = match weight_kind {
                SurvivalWeight::LogRank => 1.0,
                SurvivalWeight::GehanWilcoxon => nf,
            };
            let var_mult = if n_total > 1 { w * w * df * (nf - df) / (nf * (nf - 1.0)) } else { 0.0 };
            for g in 0..k {
                let ng = at_risk[g] as f64;
                let dg = d_g[g] as f64;
                let eg = (ng / nf) * df;
                obs_tot[g] += dg;
                exp_tot[g] += eg;
                u[g] += w * (dg - eg);
                if var_mult > 0.0 {
                    for h in 0..k {
                        let nh = at_risk[h] as f64;
                        let delta = if g == h { 1.0 } else { 0.0 };
                        cov[g * k + h] += var_mult * ng * (delta - nh / nf);
                    }
                }
            }
        }
        for g in 0..k {
            at_risk[g] = at_risk[g].saturating_sub(d_g[g] + c_g[g]);
        }
    }
    Ok((u, cov, obs_tot, exp_tot))
}

// ============================================================================
// Cox Proportional Hazards Regression (Breslow & Efron Ties + Schoenfeld PH)
// ============================================================================

/// Input observation record for Cox Proportional Hazards regression.
#[derive(Debug, Clone, PartialEq)]
pub struct CoxSubject {
    /// Non-negative follow-up time (`>= 0.0`; non-finite or `< 0.0` subjects are rejected).
    pub time: f64,
    /// Event indicator (`true` = failure event observed at `time`, `false` = right-censored).
    pub event: bool,
    /// Covariate design vector `X_i` of fixed length `p >= 1` across all subjects.
    pub x: Vec<f64>,
}

const COX_EXACT_MAX_DP_ELEMS: usize = 4_000_000;
const COX_EXACT_MAX_OPS_PER_EVAL: usize = 50_000_000;

pub(crate) type OrderedCoxSubject = (f64, bool, Vec<f64>, usize);

pub(crate) struct PreparedCoxOrder {
    pub(crate) ordered: Vec<OrderedCoxSubject>,
    pub(crate) clusters: Vec<CoxTimeCluster>,
    pub(crate) x_sds: Vec<f64>,
}

fn standardize_cox_subjects(subjects: &[CoxSubject], p: usize) -> Result<(Vec<OrderedCoxSubject>, Vec<f64>), StatsError> {
    let n = subjects.len();
    let nf = n as f64;
    let mut x_means = vec![0.0_f64; p];
    let mut x_sds = vec![1.0_f64; p];
    for j in 0..p {
        let col: Vec<f64> = subjects.iter().map(|s| s.x[j]).collect();
        let mom = SummaryMoments::from_slice(&col)?;
        x_means[j] = mom.mean;
        let max_dev = col.iter().map(|&v| (v - mom.mean).abs()).fold(0.0_f64, f64::max);
        if max_dev > 0.0 {
            let ss_scaled: f64 = col
                .iter()
                .map(|&v| {
                    let d = (v - mom.mean) / max_dev;
                    d * d
                })
                .sum();
            let sd = max_dev * (ss_scaled / nf).sqrt();
            if sd.is_finite() && sd > 0.0 {
                x_sds[j] = sd;
            }
        }
    }
    let mut ordered: Vec<OrderedCoxSubject> = subjects
        .iter()
        .enumerate()
        .map(|(orig_idx, s)| {
            let xs: Vec<f64> = (0..p).map(|j| (s.x[j] - x_means[j]) / x_sds[j]).collect();
            (normalize_zero_time(s.time), s.event, xs, orig_idx)
        })
        .collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    Ok((ordered, x_sds))
}

pub(crate) fn prepare_cox_ordered(subjects: &[CoxSubject], p: usize) -> Result<PreparedCoxOrder, StatsError> {
    let (ordered, x_sds) = standardize_cox_subjects(subjects, p)?;
    let clusters = build_cox_clusters(&ordered);
    Ok(PreparedCoxOrder { ordered, clusters, x_sds })
}

fn validate_cox_exact_ties_budget(n: usize, clusters: &[CoxTimeCluster], p: usize) -> Result<(), StatsError> {
    let p_sq = p.checked_mul(p).ok_or(StatsError::InvalidParameter { name: "cox_exact_ties_budget", value: f64::INFINITY })?;
    let p_tri = p
        .checked_mul(p.saturating_add(1))
        .map(|v| v / 2)
        .ok_or(StatsError::InvalidParameter { name: "cox_exact_ties_budget", value: f64::INFINITY })?;
    let mut total_ops = 0_usize;
    for cl in clusters {
        let d = cl.event_end - cl.risk_start;
        let m = n - cl.risk_start;
        if d < 2 || d >= m {
            continue;
        }
        let k_target = d.min(m - d);
        let dp_elems = k_target.checked_add(1).and_then(|kp1| kp1.checked_mul(p_sq));
        if dp_elems.is_none_or(|elems| elems > COX_EXACT_MAX_DP_ELEMS) {
            return Err(StatsError::InvalidParameter { name: "cox_exact_ties_budget", value: ((k_target as f64) + 1.0) * (p as f64) * (p as f64) });
        }
        let cl_ops = m.checked_mul(k_target).and_then(|mk| mk.checked_mul(p_tri));
        let Some(next_ops) = cl_ops.and_then(|ops| total_ops.checked_add(ops)) else {
            return Err(StatsError::InvalidParameter { name: "cox_exact_ties_budget", value: (m as f64) * (k_target as f64) * (p_tri as f64) });
        };
        if next_ops > COX_EXACT_MAX_OPS_PER_EVAL {
            return Err(StatsError::InvalidParameter { name: "cox_exact_ties_budget", value: next_ops as f64 });
        }
        total_ops = next_ops;
    }
    Ok(())
}

/// Fit a Cox (1972) Proportional Hazards regression model using Newton–Raphson with backtracking
/// line search and Breslow (1974), Efron (1977), or Exact discrete (Gail, Lubin & Rubinstein
/// 1981) tie handling.
///
/// Covariates are centered and standardized internally so orthogonal multi-scale covariates do
/// not fail Cholesky conditioning; coefficients and covariance matrices are back-transformed to
/// the original predictor units.
///
/// - Proportional hazards: Schoenfeld (1982) residuals and the Grambsch & Therneau (1994) test.
///
/// Full references are listed in the module header.
pub fn cox_ph_regression(
    subjects: &[CoxSubject],
    covariate_names: &[String],
    tie_method: CoxTieMethod,
    confidence: ConfidenceLevel,
) -> Result<CoxPhResult, StatsError> {
    let n = subjects.len();
    let p = covariate_names.len();
    if n < p + 2 || p == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "Cox PH regression", required: p + 2, actual: n });
    }
    let n_events = subjects.iter().filter(|s| s.event).count();
    if n_events == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "Cox PH events", required: 1, actual: 0 });
    }
    for s in subjects {
        if !s.time.is_finite() || s.time < 0.0 || s.x.len() != p || s.x.iter().any(|v| !v.is_finite()) {
            return Err(StatsError::InvalidParameter { name: "cox_subject", value: s.time });
        }
    }

    // Center and standardize covariates internally so multi-scale predictors never trigger
    // spurious Cholesky failure or Levenberg damping imbalance.
    let (ordered, x_sds) = standardize_cox_subjects(subjects, p)?;
    let clusters = build_cox_clusters(&ordered);
    if tie_method == CoxTieMethod::Exact {
        validate_cox_exact_ties_budget(n, &clusters, p)?;
    }

    let zero_beta = vec![0.0_f64; p];
    let (ll_null, grad_null, info_null) = cox_eval(&ordered, &clusters, &zero_beta, tie_method, p)?;
    let score_chi2 = solve_quad_form(&info_null, &grad_null, p).unwrap_or(0.0);

    let mut beta_std = vec![0.0_f64; p];
    let mut ll_curr = ll_null;
    let mut grad = grad_null;
    let mut info = info_null.clone();
    let mut last_step = vec![0.0_f64; p];
    let mut converged = false;
    let mut iterations = 0_usize;

    for iter in 1..=80 {
        iterations = iter;
        let info_mat = DenseMatrix::from_row_major(p, p, info.clone())?;
        let step = if let Ok(chol) = cholesky(&info_mat) {
            chol.solve(&grad)?
        } else {
            // Mild Levenberg damping on near-singular standardized information matrix.
            let mut damped = info.clone();
            for j in 0..p {
                damped[j * p + j] += 1e-4;
            }
            let dm = DenseMatrix::from_row_major(p, p, damped)?;
            let (ls, _) = solve_linear_least_squares(&dm, &grad)?;
            ls.coefficients
        };
        let max_lp_step = max_abs_linear_predictor(&ordered, &step);
        let mut scale = if max_lp_step > 1e6 { 1e6 / max_lp_step } else { 1.0 };

        let mut accepted = false;
        for _ in 0..32 {
            let cand: Vec<f64> = (0..p).map(|j| beta_std[j] + scale * step[j]).collect();
            if let Ok((ll_cand, g_cand, i_cand)) = cox_eval(&ordered, &clusters, &cand, tie_method, p)
                && ll_cand.is_finite()
                && ll_cand >= ll_curr - 1e-12
            {
                let delta = (ll_cand - ll_curr).abs();
                beta_std = cand;
                ll_curr = ll_cand;
                grad = g_cand;
                info = i_cand;
                last_step = step;
                accepted = true;
                if delta < 1e-10 && max_lp_step * scale < 1e-8 {
                    converged = true;
                }
                break;
            }
            scale *= 0.5;
        }
        if converged || !accepted {
            break;
        }
    }

    let info_null_mat = DenseMatrix::from_row_major(p, p, info_null.clone())?;
    let null_well_conditioned = cholesky(&info_null_mat).ok().is_some_and(|chol| matrix_pivots_well_conditioned(&chol, &info_null, p));
    let info_mat = DenseMatrix::from_row_major(p, p, info.clone())?;
    let chol_opt = cholesky(&info_mat).ok();
    let curr_well_conditioned = chol_opt.as_ref().is_some_and(|chol| matrix_pivots_well_conditioned(chol, &info, p));

    let separated = detect_cox_separation(&ordered, &clusters, &beta_std, &last_step, &info_null, &info, tie_method);
    let diverged = !converged || !ll_curr.is_finite() || beta_std.iter().any(|b| !b.is_finite());

    // Never regularize (info + 1e-6*I) or substitute I when covariance is singular/separated.
    if diverged || separated || !null_well_conditioned || !curr_well_conditioned {
        converged = false;
    }
    let cov_std = if converged { chol_opt.and_then(|chol| chol.inverse().ok()) } else { None };
    if cov_std.is_none() {
        converged = false;
    }

    let lr_chi2 = (2.0 * (ll_curr - ll_null)).max(0.0);
    let df = p as f64;
    let lr_p = chi2_sf(lr_chi2, df)?;
    let (wald_chi2, wald_p) = if cov_std.is_some() {
        let w = (0..p)
            .map(|j| {
                let row_dot: f64 = (0..p).map(|k| info[j * p + k] * beta_std[k]).sum();
                beta_std[j] * row_dot
            })
            .sum::<f64>()
            .max(0.0);
        (w, chi2_sf(w, df)?)
    } else {
        (f64::NAN, f64::NAN)
    };
    let score_p = chi2_sf(score_chi2, df)?;

    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let (schoenfeld_rhos, schoenfeld_ps, global_sch_chi2) = schoenfeld_diagnostics(&ordered, &clusters, &beta_std, cov_std.as_ref(), tie_method, p)?;
    let global_sch_p = if global_sch_chi2.is_finite() { chi2_sf(global_sch_chi2, df)? } else { f64::NAN };

    let mut covariates = Vec::with_capacity(p);
    for j in 0..p {
        let beta_j = beta_std[j] / x_sds[j];
        let var_std_j = cov_std.as_ref().and_then(|cov| cov.get(j, j)).filter(|v| v.is_finite() && *v > 0.0);
        let (se, z_stat, p_val, hr_lo, hr_hi) = if let Some(vj) = var_std_j {
            let s_std = vj.sqrt();
            let s = s_std / x_sds[j];
            let z = beta_std[j] / s_std;
            (s, z, norm_two_sided_p(z), (beta_j - z_crit * s).exp(), (beta_j + z_crit * s).exp())
        } else {
            (f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN)
        };
        let hr = beta_j.exp();
        covariates.push(CoxCovariateResult {
            name: covariate_names[j].clone(),
            beta: beta_j,
            se,
            z_statistic: z_stat,
            p_value: p_val,
            hazard_ratio: hr,
            hr_ci: (hr_lo, hr_hi),
            schoenfeld_rho: schoenfeld_rhos[j],
            schoenfeld_p: schoenfeld_ps[j],
        });
    }

    let (martingale_residuals, deviance_residuals, baseline_survival) = cox_residuals_and_baseline(&ordered, &clusters, &beta_std, tie_method, n);
    let (concordance, comparable_pairs) = harrell_c_index(&ordered, &beta_std);
    let (cov_beta_rows, correlation_matrix) = cox_cov_and_corr_matrices(cov_std.as_ref(), &x_sds, p);

    Ok(CoxPhResult {
        n_subjects: n,
        n_events,
        tie_method,
        converged,
        iterations,
        log_lik_null: ll_null,
        log_lik_model: ll_curr,
        concordance,
        comparable_pairs,
        likelihood_ratio_chi2: lr_chi2,
        likelihood_ratio_p: lr_p,
        wald_chi2,
        wald_p,
        score_chi2,
        score_p,
        global_schoenfeld_chi2: global_sch_chi2,
        global_schoenfeld_df: df,
        global_schoenfeld_p: global_sch_p,
        covariates,
        martingale_residuals,
        deviance_residuals,
        baseline_survival,
        cov_beta: cov_beta_rows,
        correlation_matrix,
    })
}

fn cox_cov_and_corr_matrices(cov_std: Option<&DenseMatrix>, x_sds: &[f64], p: usize) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    let Some(cov) = cov_std else {
        return (vec![vec![f64::NAN; p]; p], vec![vec![f64::NAN; p]; p]);
    };
    let mut cov_rows = vec![vec![0.0_f64; p]; p];
    let mut corr_rows = vec![vec![0.0_f64; p]; p];
    let inv_sds: Vec<f64> = (0..p)
        .map(|j| {
            let var = cov.get(j, j).unwrap_or(0.0).max(0.0);
            if var > 0.0 { 1.0 / var.sqrt() } else { 0.0 }
        })
        .collect();
    for j in 0..p {
        for k in 0..p {
            let c_std = cov.get(j, k).unwrap_or(0.0);
            cov_rows[j][k] = c_std / (x_sds[j] * x_sds[k]);
            corr_rows[j][k] = if j == k { if inv_sds[j] > 0.0 { 1.0 } else { f64::NAN } } else { (c_std * inv_sds[j] * inv_sds[k]).clamp(-1.0, 1.0) };
        }
    }
    (cov_rows, corr_rows)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CoxTimeCluster {
    pub(crate) time: f64,
    pub(crate) risk_start: usize,
    pub(crate) event_end: usize,
    pub(crate) cluster_end: usize,
}

fn build_cox_clusters(ordered: &[(f64, bool, Vec<f64>, usize)]) -> Vec<CoxTimeCluster> {
    let n = ordered.len();
    let mut clusters = Vec::new();
    let mut idx = 0_usize;
    while idx < n {
        let t = ordered[idx].0;
        let risk_start = idx;
        while idx < n && ordered[idx].0 == t && ordered[idx].1 {
            idx += 1;
        }
        let event_end = idx;
        while idx < n && ordered[idx].0 == t {
            idx += 1;
        }
        clusters.push(CoxTimeCluster { time: t, risk_start, event_end, cluster_end: idx });
    }
    clusters
}

#[inline]
fn log_add_exp(a: f64, b: f64) -> f64 {
    if !a.is_finite() {
        return b;
    }
    if !b.is_finite() {
        return a;
    }
    let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
    let diff = lo - hi;
    if diff < -750.0 { hi } else { hi + diff.exp().ln_1p() }
}

fn compute_linear_predictors(ordered: &[(f64, bool, Vec<f64>, usize)], beta: &[f64], offset: Option<&[f64]>) -> Result<Vec<f64>, StatsError> {
    let mut lps = Vec::with_capacity(ordered.len());
    for (i, s) in ordered.iter().enumerate() {
        let mut lp: f64 = s.2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
        if let Some(off) = offset {
            lp += off.get(i).copied().unwrap_or(0.0);
        }
        if !lp.is_finite() {
            return Err(StatsError::NotConverged { algorithm: "cox_eval_non_finite_lp" });
        }
        lps.push(lp);
    }
    Ok(lps)
}

fn max_abs_linear_predictor(ordered: &[(f64, bool, Vec<f64>, usize)], vec: &[f64]) -> f64 {
    ordered.iter().map(|s| s.2.iter().zip(vec.iter()).map(|(x, b)| x * b).sum::<f64>().abs()).fold(0.0_f64, f64::max)
}

fn matrix_pivots_well_conditioned(chol: &curvebook_math::linalg::Cholesky, mat: &[f64], p: usize) -> bool {
    (0..p).all(|j| {
        let diag = mat[j * p + j];
        let piv = chol.l().get(j, j).unwrap_or(0.0);
        let piv_sq = piv * piv;
        diag.is_finite() && diag > 0.0 && piv.is_finite() && piv_sq > 1e-10 * diag
    })
}

fn has_monotone_separation(ordered: &[(f64, bool, Vec<f64>, usize)], clusters: &[CoxTimeCluster], dir: &[f64], tie_method: CoxTieMethod) -> bool {
    let dir_norm = dir.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !dir_norm.is_finite() || dir_norm <= 0.0 {
        return false;
    }
    let lps: Vec<f64> = ordered.iter().map(|s| s.2.iter().zip(dir.iter()).map(|(x, d)| x * d).sum::<f64>()).collect();
    let max_abs = lps.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
    if !max_abs.is_finite() || max_abs == 0.0 {
        return false;
    }
    let tol = 1e-12 * max_abs;
    let n = ordered.len();
    let mut has_strict = false;
    for cl in clusters {
        if cl.event_end == cl.risk_start {
            continue;
        }
        let mut min_evt = f64::INFINITY;
        let mut max_evt = f64::NEG_INFINITY;
        for &lp in &lps[cl.risk_start..cl.event_end] {
            min_evt = min_evt.min(lp);
            max_evt = max_evt.max(lp);
        }
        if cl.event_end == n {
            // Under Breslow/Efron, tied events appear in the risk-set denominator alongside each
            // other, so min_evt < max_evt forces partial log-likelihood -> -inf along `dir`.
            if tie_method != CoxTieMethod::Exact && min_evt < max_evt - tol {
                return false;
            }
            continue;
        }
        let mut max_surv = f64::NEG_INFINITY;
        let mut min_surv = f64::INFINITY;
        for &lp in &lps[cl.event_end..n] {
            max_surv = max_surv.max(lp);
            min_surv = min_surv.min(lp);
        }
        match tie_method {
            CoxTieMethod::Breslow | CoxTieMethod::Efron => {
                // Every tied event must attain the maximum across both tied events and survivors.
                if min_evt < max_evt.max(max_surv) - tol {
                    return false;
                }
                if min_evt > min_surv + tol {
                    has_strict = true;
                }
            }
            CoxTieMethod::Exact => {
                // Exact discrete partial likelihood sums over d-subsets of the risk set; the
                // observed event subset attains the max subset sum iff min_evt >= max_surv and
                // strictly beats another d-subset iff max_evt > min_surv.
                if min_evt < max_surv - tol {
                    return false;
                }
                if max_evt > min_surv + tol {
                    has_strict = true;
                }
            }
        }
    }
    has_strict
}

fn detect_cox_separation(
    ordered: &[OrderedCoxSubject],
    clusters: &[CoxTimeCluster],
    beta: &[f64],
    last_step: &[f64],
    info_null: &[f64],
    info_curr: &[f64],
    tie_method: CoxTieMethod,
) -> bool {
    let p = beta.len();
    let neg_beta: Vec<f64> = beta.iter().map(|b| -b).collect();
    if has_monotone_separation(ordered, clusters, beta, tie_method) || has_monotone_separation(ordered, clusters, &neg_beta, tie_method) {
        return true;
    }
    let neg_step: Vec<f64> = last_step.iter().map(|v| -v).collect();
    if has_monotone_separation(ordered, clusters, last_step, tie_method) || has_monotone_separation(ordered, clusters, &neg_step, tie_method) {
        return true;
    }
    for j in 0..p {
        let dn = info_null[j * p + j];
        let dc = info_curr[j * p + j];
        if dn.is_finite() && dn > 0.0 && (!dc.is_finite() || dc <= 1e-8 * dn) {
            let mut pos_e = vec![0.0_f64; p];
            let mut neg_e = vec![0.0_f64; p];
            pos_e[j] = 1.0;
            neg_e[j] = -1.0;
            if has_monotone_separation(ordered, clusters, &pos_e, tie_method) || has_monotone_separation(ordered, clusters, &neg_e, tie_method) {
                return true;
            }
        }
    }
    false
}

struct WeightedRiskMoments {
    p: usize,
    shift: f64,
    weight_sum: f64,
    mean: Vec<f64>,
    m2: Vec<f64>,
    delta: Vec<f64>,
}

impl WeightedRiskMoments {
    fn new(p: usize) -> Self {
        Self { p, shift: f64::NEG_INFINITY, weight_sum: 0.0, mean: vec![0.0_f64; p], m2: vec![0.0_f64; p * p], delta: vec![0.0_f64; p] }
    }

    fn reset(&mut self, shift: f64) {
        self.shift = shift;
        self.weight_sum = 0.0;
        self.mean.fill(0.0);
        self.m2.fill(0.0);
    }

    fn rescale_to(&mut self, new_shift: f64, need_info: bool) {
        if !self.shift.is_finite() {
            self.shift = new_shift;
            return;
        }
        if new_shift > self.shift {
            let scale = (self.shift - new_shift).exp();
            if scale == 0.0 {
                self.weight_sum = 0.0;
                if need_info {
                    self.m2.fill(0.0);
                }
            } else {
                self.weight_sum *= scale;
                if need_info {
                    for j in 0..self.p {
                        for k in j..self.p {
                            self.m2[j * self.p + k] *= scale;
                        }
                    }
                }
            }
            self.shift = new_shift;
        }
    }

    fn add_subject(&mut self, lp: f64, x: &[f64], need_info: bool) {
        let w = (lp - self.shift).exp();
        if w == 0.0 || !w.is_finite() {
            return;
        }
        if self.weight_sum == 0.0 {
            self.weight_sum = w;
            self.mean.copy_from_slice(x);
            return;
        }
        let w_new = self.weight_sum + w;
        let inv_w_new = 1.0 / w_new;
        for j in 0..self.p {
            let d = x[j] - self.mean[j];
            self.delta[j] = d;
            self.mean[j] += (w * inv_w_new) * d;
        }
        if need_info {
            let factor = w * (self.weight_sum * inv_w_new);
            for j in 0..self.p {
                for k in j..self.p {
                    self.m2[j * self.p + k] += factor * self.delta[j] * self.delta[k];
                }
            }
        }
        self.weight_sum = w_new;
    }

    fn merge_same_shift(&mut self, other: &Self, need_info: bool) {
        if other.weight_sum == 0.0 {
            return;
        }
        if self.weight_sum == 0.0 {
            self.weight_sum = other.weight_sum;
            self.mean.copy_from_slice(&other.mean);
            if need_info {
                self.m2.copy_from_slice(&other.m2);
            }
            return;
        }
        let ws = self.weight_sum;
        let we = other.weight_sum;
        let w_tot = ws + we;
        for j in 0..self.p {
            let d = other.mean[j] - self.mean[j];
            self.delta[j] = d;
            if ws >= we {
                self.mean[j] += (we / w_tot) * d;
            } else {
                self.mean[j] = other.mean[j] - (ws / w_tot) * d;
            }
        }
        if need_info {
            let bridge = (ws * we) / w_tot;
            for j in 0..self.p {
                for k in j..self.p {
                    self.m2[j * self.p + k] += other.m2[j * self.p + k] + bridge * self.delta[j] * self.delta[k];
                }
            }
        }
        self.weight_sum = w_tot;
    }

    fn combine_mean_and_cov(&self, other: &Self, other_scale: f64, mean_out: &mut [f64], mut info_acc: Option<&mut [f64]>) -> f64 {
        let ws = self.weight_sum;
        let we = other_scale * other.weight_sum;
        let denom = ws + we;
        if denom <= 0.0 {
            return 0.0;
        }
        if ws == 0.0 {
            mean_out.copy_from_slice(&other.mean);
            if let Some(info) = info_acc {
                let scale = other_scale / denom;
                for j in 0..self.p {
                    for k in j..self.p {
                        let v = scale * other.m2[j * self.p + k];
                        info[j * self.p + k] += v;
                        if j != k {
                            info[k * self.p + j] += v;
                        }
                    }
                }
            }
            return denom;
        }
        if we == 0.0 {
            mean_out.copy_from_slice(&self.mean);
            if let Some(info) = info_acc {
                let scale = 1.0 / denom;
                for j in 0..self.p {
                    for k in j..self.p {
                        let v = scale * self.m2[j * self.p + k];
                        info[j * self.p + k] += v;
                        if j != k {
                            info[k * self.p + j] += v;
                        }
                    }
                }
            }
            return denom;
        }
        let bridge = (ws * we) / denom;
        for j in 0..self.p {
            let d = other.mean[j] - self.mean[j];
            mean_out[j] = if ws >= we { self.mean[j] + (we / denom) * d } else { other.mean[j] - (ws / denom) * d };
        }
        if let Some(ref mut info) = info_acc {
            for j in 0..self.p {
                let dj = other.mean[j] - self.mean[j];
                for k in j..self.p {
                    let dk = other.mean[k] - self.mean[k];
                    let v = (self.m2[j * self.p + k] + other_scale * other.m2[j * self.p + k] + bridge * dj * dk) / denom;
                    info[j * self.p + k] += v;
                    if j != k {
                        info[k * self.p + j] += v;
                    }
                }
            }
        }
        denom
    }
}

/// Log-space Gail–Lubin–Rubinstein (1981) symmetric-polynomial dynamic programming for a tied
/// event cluster `(risk_start, event_end)` with `d >= 2` events out of `m = n - risk_start` at-risk
/// subjects.
///
/// Returns `(ll_contrib, mu_d, cov_d_upper)` where `mu_d = E[sum_{S} x]` and
/// `cov_d = Cov(sum_{S} x)` over all `d`-subsets `S` of the risk set with probability proportional
/// to `exp(sum_{i in S} lp[i])`. Uses complement symmetry `d_eff = d.min(m - d)` when `d > m / 2`.
fn glr_exact_cluster_dp(
    ordered: &[(f64, bool, Vec<f64>, usize)],
    raw_lp: &[f64],
    risk_start: usize,
    event_end: usize,
    p: usize,
    need_info: bool,
) -> (f64, Vec<f64>, Vec<f64>) {
    let n = ordered.len();
    let d = event_end - risk_start;
    let m = n - risk_start;
    let p2 = p * p;
    if d == m {
        let mut mu_tot = vec![0.0_f64; p];
        for row in &ordered[risk_start..n] {
            for j in 0..p {
                mu_tot[j] += row.2[j];
            }
        }
        return (0.0, mu_tot, vec![0.0_f64; p2]);
    }

    let complement = d > m / 2;
    let k_target = if complement { m - d } else { d };
    let sign = if complement { -1.0_f64 } else { 1.0_f64 };
    let lp_ref = if complement {
        raw_lp[risk_start..n].iter().copied().fold(f64::INFINITY, f64::min)
    } else {
        raw_lp[risk_start..n].iter().copied().fold(f64::NEG_INFINITY, f64::max)
    };

    let mut dp_l = vec![f64::NEG_INFINITY; k_target + 1];
    dp_l[0] = 0.0;
    let mut dp_mu = vec![0.0_f64; (k_target + 1) * p];
    let mut dp_c = if need_info { vec![0.0_f64; (k_target + 1) * p2] } else { Vec::new() };
    let mut delta = vec![0.0_f64; p];

    for (step_idx, i) in (risk_start..n).enumerate() {
        let eta_item = sign * (raw_lp[i] - lp_ref);
        let x_item = &ordered[i].2;
        let k_max = (step_idx + 1).min(k_target);
        for k in (1..=k_max).rev() {
            let l_prev = dp_l[k - 1];
            if !l_prev.is_finite() {
                continue;
            }
            let l_term = eta_item + l_prev;
            let l_old = dp_l[k];
            if !l_old.is_finite() {
                dp_l[k] = l_term;
                for j in 0..p {
                    dp_mu[k * p + j] = dp_mu[(k - 1) * p + j] + x_item[j];
                }
                if need_info {
                    let src = (k - 1) * p2;
                    let dst = k * p2;
                    dp_c.copy_within(src..src + p2, dst);
                }
            } else {
                let l_new = log_add_exp(l_old, l_term);
                let alpha = (l_old - l_new).exp();
                let gamma = (l_term - l_new).exp();
                dp_l[k] = l_new;
                for j in 0..p {
                    delta[j] = (dp_mu[(k - 1) * p + j] + x_item[j]) - dp_mu[k * p + j];
                }
                if need_info {
                    let bridge = alpha * gamma;
                    let cur_off = k * p2;
                    let prv_off = (k - 1) * p2;
                    for j in 0..p {
                        for col in j..p {
                            let idx_mat = j * p + col;
                            dp_c[cur_off + idx_mat] =
                                alpha * dp_c[cur_off + idx_mat] + gamma * dp_c[prv_off + idx_mat] + bridge * delta[j] * delta[col];
                        }
                    }
                }
                for j in 0..p {
                    if alpha >= gamma {
                        dp_mu[k * p + j] += gamma * delta[j];
                    } else {
                        dp_mu[k * p + j] = (dp_mu[(k - 1) * p + j] + x_item[j]) - alpha * delta[j];
                    }
                }
            }
        }
    }

    let c_out = if need_info { dp_c[k_target * p2..(k_target + 1) * p2].to_vec() } else { Vec::new() };
    let mu_res = &dp_mu[k_target * p..(k_target + 1) * p];
    if complement {
        let mut surv_eta = 0.0_f64;
        for &lp in &raw_lp[event_end..n] {
            surv_eta += -(lp - lp_ref);
        }
        let ll_contrib = surv_eta - dp_l[k_target];
        let mut mu_d = vec![0.0_f64; p];
        for row in &ordered[risk_start..n] {
            for j in 0..p {
                mu_d[j] += row.2[j];
            }
        }
        for j in 0..p {
            mu_d[j] -= mu_res[j];
        }
        (ll_contrib, mu_d, c_out)
    } else {
        let mut evt_eta = 0.0_f64;
        for &lp in &raw_lp[risk_start..event_end] {
            evt_eta += lp - lp_ref;
        }
        let ll_contrib = evt_eta - dp_l[k_target];
        (ll_contrib, mu_res.to_vec(), c_out)
    }
}

/// Log partial likelihood, score vector, and observed information of the Cox (1972) model at
/// `beta` with optional per-subject linear-predictor `offset`, Breslow (1974), Efron (1977), or
/// Exact discrete (Kalbfleisch & Prentice 2002; Gail, Lubin & Rubinstein 1981) tie handling,
/// stabilized via per-risk-set log-sum-exp. Supports `p == 0` for offset-only partial likelihoods.
pub(crate) fn cox_eval_ordered_with_offset(
    ordered: &[(f64, bool, Vec<f64>, usize)],
    clusters: &[CoxTimeCluster],
    beta: &[f64],
    offset: Option<&[f64]>,
    tie_method: CoxTieMethod,
    p: usize,
) -> Result<(f64, Vec<f64>, Vec<f64>), StatsError> {
    let raw_lp = compute_linear_predictors(ordered, beta, offset)?;
    let need_info = p > 0;
    let mut ll = 0.0_f64;
    let mut grad = vec![0.0_f64; p];
    let mut info = vec![0.0_f64; p * p];

    let mut after = WeightedRiskMoments::new(p);
    let mut evt_moments = WeightedRiskMoments::new(p);
    let mut mean_buf = vec![0.0_f64; p];

    for cl in clusters.iter().rev() {
        let lp_cluster_max = raw_lp[cl.risk_start..cl.cluster_end].iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let lp_risk_max = if after.shift.is_finite() { after.shift.max(lp_cluster_max) } else { lp_cluster_max };
        after.rescale_to(lp_risk_max, need_info);
        for i in cl.event_end..cl.cluster_end {
            after.add_subject(raw_lp[i], &ordered[i].2, need_info);
        }

        evt_moments.reset(lp_risk_max);
        for i in cl.risk_start..cl.event_end {
            evt_moments.add_subject(raw_lp[i], &ordered[i].2, need_info);
        }

        let d = cl.event_end - cl.risk_start;
        if d > 0 {
            if tie_method == CoxTieMethod::Exact && d >= 2 {
                let (ll_cl, mu_d, c_d) = glr_exact_cluster_dp(ordered, &raw_lp, cl.risk_start, cl.event_end, p, need_info);
                ll += ll_cl;
                for i in cl.risk_start..cl.event_end {
                    for j in 0..p {
                        grad[j] += ordered[i].2[j];
                    }
                }
                for j in 0..p {
                    grad[j] -= mu_d[j];
                    for k in j..p {
                        let v = c_d[j * p + k];
                        info[j * p + k] += v;
                        if j != k {
                            info[k * p + j] += v;
                        }
                    }
                }
            } else {
                for i in cl.risk_start..cl.event_end {
                    ll += raw_lp[i] - lp_risk_max;
                    for j in 0..p {
                        grad[j] += ordered[i].2[j];
                    }
                }
                let df_f = d as f64;
                for m in 0..d {
                    let frac = if tie_method == CoxTieMethod::Efron { (m as f64) / df_f } else { 0.0 };
                    let info_opt = if need_info { Some(info.as_mut_slice()) } else { None };
                    let denom = after.combine_mean_and_cov(&evt_moments, 1.0 - frac, &mut mean_buf, info_opt);
                    if !denom.is_finite() || denom <= 0.0 {
                        return Err(StatsError::NotConverged { algorithm: "cox_eval_risk_underflow" });
                    }
                    ll -= denom.ln();
                    for j in 0..p {
                        grad[j] -= mean_buf[j];
                    }
                }
            }
        }

        after.merge_same_shift(&evt_moments, need_info);
    }

    Ok((ll, grad, info))
}

fn cox_eval(
    ordered: &[(f64, bool, Vec<f64>, usize)],
    clusters: &[CoxTimeCluster],
    beta: &[f64],
    tie_method: CoxTieMethod,
    p: usize,
) -> Result<(f64, Vec<f64>, Vec<f64>), StatsError> {
    cox_eval_ordered_with_offset(ordered, clusters, beta, None, tie_method, p)
}

fn solve_quad_form(mat_data: &[f64], vec: &[f64], p: usize) -> Result<f64, StatsError> {
    let m = DenseMatrix::from_row_major(p, p, mat_data.to_vec())?;
    let sol = if let Ok(chol) = cholesky(&m) {
        chol.solve(vec)?
    } else {
        let (ls, _) = solve_linear_least_squares(&m, vec)?;
        ls.coefficients
    };
    Ok(vec.iter().zip(sol.iter()).map(|(a, b)| a * b).sum::<f64>().max(0.0))
}

/// Schoenfeld (1982) residuals `r_k = x_k - xbar(beta, t_k)` at each event (Breslow risk-set mean,
/// Efron-weighted mean, or Exact conditional mean `mu_d / d`) and Grambsch & Therneau (1994)
/// proportional-hazards tests based on scaled residuals `r*_k = d * Cov(beta) * r_k`, stabilized
/// per risk set without linear-predictor clamping.
fn schoenfeld_diagnostics(
    ordered: &[(f64, bool, Vec<f64>, usize)],
    clusters: &[CoxTimeCluster],
    beta: &[f64],
    cov_beta: Option<&DenseMatrix>,
    tie_method: CoxTieMethod,
    p: usize,
) -> Result<(Vec<f64>, Vec<f64>, f64), StatsError> {
    let Some(cov_beta) = cov_beta else {
        return Ok((vec![f64::NAN; p], vec![f64::NAN; p], f64::NAN));
    };
    let raw_lp = compute_linear_predictors(ordered, beta, None)?;

    let mut cluster_e_bars: Vec<Option<Vec<f64>>> = vec![None; clusters.len()];
    let mut after = WeightedRiskMoments::new(p);
    let mut evt_moments = WeightedRiskMoments::new(p);
    let mut mean_buf = vec![0.0_f64; p];

    for (c_idx, cl) in clusters.iter().enumerate().rev() {
        let lp_cluster_max = raw_lp[cl.risk_start..cl.cluster_end].iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let lp_risk_max = if after.shift.is_finite() { after.shift.max(lp_cluster_max) } else { lp_cluster_max };
        after.rescale_to(lp_risk_max, false);
        for i in cl.event_end..cl.cluster_end {
            after.add_subject(raw_lp[i], &ordered[i].2, false);
        }
        evt_moments.reset(lp_risk_max);
        for i in cl.risk_start..cl.event_end {
            evt_moments.add_subject(raw_lp[i], &ordered[i].2, false);
        }

        let d = cl.event_end - cl.risk_start;
        if d > 0 {
            let mut e_bar = vec![0.0_f64; p];
            let df_f = d as f64;
            if tie_method == CoxTieMethod::Exact && d >= 2 {
                let (_, mu_d, _) = glr_exact_cluster_dp(ordered, &raw_lp, cl.risk_start, cl.event_end, p, false);
                for j in 0..p {
                    e_bar[j] = mu_d[j] / df_f;
                }
            } else {
                for m in 0..d {
                    let frac = if tie_method == CoxTieMethod::Efron { (m as f64) / df_f } else { 0.0 };
                    let _ = after.combine_mean_and_cov(&evt_moments, 1.0 - frac, &mut mean_buf, None);
                    for j in 0..p {
                        e_bar[j] += mean_buf[j] / df_f;
                    }
                }
            }
            cluster_e_bars[c_idx] = Some(e_bar);
        }

        after.merge_same_shift(&evt_moments, false);
    }

    let mut resid_rows: Vec<(f64, Vec<f64>)> = Vec::new();
    for (c_idx, cl) in clusters.iter().enumerate() {
        let Some(e_bar) = &cluster_e_bars[c_idx] else {
            continue;
        };
        for i in cl.risk_start..cl.event_end {
            let r_vec: Vec<f64> = (0..p).map(|j| ordered[i].2[j] - e_bar[j]).collect();
            resid_rows.push((cl.time, r_vec));
        }
    }

    let n_evt = resid_rows.len();
    if n_evt < 3 {
        return Ok((vec![f64::NAN; p], vec![f64::NAN; p], f64::NAN));
    }

    // Grambsch-Therneau (1994) scaled Schoenfeld correlation test against tied average event-rank time.
    let mut g_times = vec![0.0_f64; n_evt];
    let denom_rank = (n_evt + 1) as f64;
    let mut r_idx = 0_usize;
    while r_idx < n_evt {
        let t_curr = resid_rows[r_idx].0;
        let start = r_idx;
        while r_idx < n_evt && resid_rows[r_idx].0 == t_curr {
            r_idx += 1;
        }
        let avg_rank = 0.5 * ((start + 1 + r_idx) as f64) / denom_rank;
        for g in &mut g_times[start..r_idx] {
            *g = avg_rank;
        }
    }
    let g_mean: f64 = g_times.iter().sum::<f64>() / (n_evt as f64);
    let g_ss: f64 = g_times.iter().map(|&g| (g - g_mean).powi(2)).sum();
    if !g_ss.is_finite() || g_ss <= 0.0 {
        return Ok((vec![f64::NAN; p], vec![f64::NAN; p], f64::NAN));
    }
    let n_ef = n_evt as f64;

    let mut scaled_cols = vec![vec![0.0_f64; n_evt]; p];
    for (i, (_, r)) in resid_rows.iter().enumerate() {
        for (j, col) in scaled_cols.iter_mut().enumerate() {
            let mut sum_jr = 0.0_f64;
            for (k, &rk) in r.iter().enumerate() {
                sum_jr += n_ef * cov_beta.get(j, k).unwrap_or(0.0) * rk;
            }
            col[i] = sum_jr;
        }
    }

    let mut rhos = vec![0.0_f64; p];
    let mut ps = vec![1.0_f64; p];
    let mut sum_scaled_t = vec![0.0_f64; p];

    for j in 0..p {
        let m_s = scaled_cols[j].iter().sum::<f64>() / n_ef;
        let ss_s: f64 = scaled_cols[j].iter().map(|&s| (s - m_s).powi(2)).sum();
        let mut cross = 0.0_f64;
        for i in 0..n_evt {
            let gt = g_times[i] - g_mean;
            cross += gt * (scaled_cols[j][i] - m_s);
            sum_scaled_t[j] += gt * resid_rows[i].1[j];
        }
        let rho = if g_ss > 0.0 && ss_s > 0.0 { (cross / (g_ss * ss_s).sqrt()).clamp(-1.0, 1.0) } else { 0.0 };
        rhos[j] = rho;
        let v_jj = cov_beta.get(j, j).unwrap_or(0.0);
        let z2 = if v_jj > 0.0 && g_ss > 0.0 { (cross * cross) / (n_ef * v_jj * g_ss) } else { 0.0 };
        ps[j] = chi2_sf(z2, 1.0)?;
    }

    // Global test: (sum_i g_i r_i)^T (n_evt * cov_beta / g_ss) (sum_i g_i r_i) (Grambsch & Therneau eq. 6).
    let mut global_chi2 = 0.0_f64;
    if g_ss > 0.0 {
        for j in 0..p {
            for k in 0..p {
                global_chi2 += sum_scaled_t[j] * (n_ef * cov_beta.get(j, k).unwrap_or(0.0) / g_ss) * sum_scaled_t[k];
            }
        }
    }
    Ok((rhos, ps, global_chi2.max(0.0)))
}

/// Martingale residuals `M_i = delta_i - exp(x_i' beta) H0(t_i)` and deviance residuals
/// `sign(M_i) sqrt(-2 (M_i + delta_i ln(delta_i - M_i)))` (Therneau, Grambsch & Fleming 1990), and
/// the cumulative baseline hazard at the mean covariates, stabilized in log-space per risk set.
fn cox_residuals_and_baseline(
    ordered: &[(f64, bool, Vec<f64>, usize)],
    clusters: &[CoxTimeCluster],
    beta: &[f64],
    tie_method: CoxTieMethod,
    n: usize,
) -> (Vec<f64>, Vec<f64>, Vec<CoxBaselinePoint>) {
    let raw_lp: Vec<f64> = ordered.iter().map(|s| s.2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum()).collect();
    let mut cluster_risk: Vec<(f64, f64, f64)> = vec![(0.0, 0.0, 0.0); clusters.len()];
    let mut shift = f64::NEG_INFINITY;
    let mut w_after = 0.0_f64;

    for (c_idx, cl) in clusters.iter().enumerate().rev() {
        let lp_cluster_max = raw_lp[cl.risk_start..cl.cluster_end].iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let lp_risk_max = if shift.is_finite() { shift.max(lp_cluster_max) } else { lp_cluster_max };
        if shift.is_finite() && lp_risk_max > shift {
            w_after *= (shift - lp_risk_max).exp();
        }
        shift = lp_risk_max;
        for &lp in &raw_lp[cl.event_end..cl.cluster_end] {
            w_after += (lp - shift).exp();
        }
        let mut w_evt = 0.0_f64;
        for &lp in &raw_lp[cl.risk_start..cl.event_end] {
            w_evt += (lp - shift).exp();
        }
        cluster_risk[c_idx] = (lp_risk_max, w_after, w_evt);
        w_after += w_evt;
    }

    let mut log_h0 = f64::NEG_INFINITY;
    let mut mart = vec![0.0_f64; n];
    let mut dev = vec![0.0_f64; n];
    let mut baseline = Vec::new();

    for (c_idx, cl) in clusters.iter().enumerate() {
        let d = cl.event_end - cl.risk_start;
        let mut log_h0_evt = log_h0;
        if d > 0 {
            let (lp_risk_max, ws, we) = cluster_risk[c_idx];
            let df_f = d as f64;
            let mut dh_shifted = 0.0_f64;
            let mut dh_evt_shifted = 0.0_f64;
            for m in 0..d {
                let frac = if tie_method == CoxTieMethod::Efron { (m as f64) / df_f } else { 0.0 };
                let step = 1.0 / (ws + (1.0 - frac) * we).max(1e-300);
                dh_shifted += step;
                dh_evt_shifted += (1.0 - frac) * step;
            }
            let log_dh = -lp_risk_max + dh_shifted.ln();
            let log_dh_evt = if dh_evt_shifted > 0.0 { -lp_risk_max + dh_evt_shifted.ln() } else { f64::NEG_INFINITY };
            log_h0_evt = log_add_exp(log_h0, log_dh_evt);
            log_h0 = log_add_exp(log_h0, log_dh);
            let cum_h0 = log_h0.exp();
            let survival = if cum_h0.is_finite() { (-cum_h0).exp().clamp(0.0, 1.0) } else { 0.0 };
            baseline.push(CoxBaselinePoint { time: cl.time, cum_hazard: cum_h0, survival });
        }
        for i in cl.risk_start..cl.cluster_end {
            let delta = if ordered[i].1 { 1.0 } else { 0.0 };
            let log_h_subj = if ordered[i].1 { log_h0_evt } else { log_h0 };
            let expected = if log_h_subj.is_finite() { (raw_lp[i] + log_h_subj).exp() } else { 0.0 };
            let m = delta - expected;
            let orig = ordered[i].3;
            mart[orig] = m;
            let sign = if m >= 0.0 { 1.0 } else { -1.0 };
            let d_sq = (-2.0 * (m + delta * (delta - m).max(1e-18).ln())).max(0.0);
            dev[orig] = sign * d_sq.sqrt();
        }
    }
    (mart, dev, baseline)
}

/// Harrell et al. (1982) concordance index for the fitted linear predictor. A pair is usable when
/// the subject with the shorter time had an event (an event tied in time with a censored subject
/// counts as shorter); tied linear predictors score 1/2. Pairs of tied event times are excluded.
/// Returns `(None, 0)` when zero comparable pairs exist.
fn harrell_c_index(ordered: &[(f64, bool, Vec<f64>, usize)], beta: &[f64]) -> (Option<f64>, usize) {
    let lp: Vec<f64> = ordered.iter().map(|s| s.2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum()).collect();
    let mut conc = 0.0_f64;
    let mut pairs = 0_usize;
    for i in 0..ordered.len() {
        if !ordered[i].1 {
            continue;
        }
        for j in 0..ordered.len() {
            if j == i {
                continue;
            }
            let comparable = ordered[j].0 > ordered[i].0 || (ordered[j].0 == ordered[i].0 && !ordered[j].1);
            if comparable {
                pairs += 1;
                let tie_tol = 1e-12 * lp[i].abs().max(lp[j].abs()).max(1.0);
                if (lp[i] - lp[j]).abs() <= tie_tol {
                    conc += 0.5;
                } else if lp[i] > lp[j] {
                    conc += 1.0;
                }
            }
        }
    }
    let c_index = if pairs > 0 { Some(conc / (pairs as f64)) } else { None };
    (c_index, pairs)
}

// ============================================================================
// ResultSheet Builder
// ============================================================================

fn validate_selected_data_sets(selected: &[usize], total: usize) -> Result<(), StatsError> {
    let mut seen = Vec::with_capacity(selected.len());
    for &idx in selected {
        if idx >= total {
            return Err(StatsError::MissingColumn { index: idx });
        }
        if seen.contains(&idx) {
            return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: idx as f64 });
        }
        seen.push(idx);
    }
    Ok(())
}

fn extract_survival_table_cohort_with_policy(
    table: &curvebook_doc::SurvivalTable,
    cohort_idx: usize,
    event_code: Option<f64>,
    censored_code: Option<f64>,
    treat_other_as: SurvivalOtherValuePolicy,
) -> Result<Vec<SurvivalObservation>, StatsError> {
    if event_code.is_none() && censored_code.is_none() && treat_other_as == SurvivalOtherValuePolicy::Exclude {
        return Ok(table.cohort_observations(cohort_idx)?);
    }
    let cohort = table.cohorts.get(cohort_idx).ok_or(StatsError::MissingColumn { index: cohort_idx })?;
    let mut out = Vec::new();
    for (row, status_subs) in cohort.cells.iter().enumerate() {
        let Some(status_cell) = status_subs.first() else {
            continue;
        };
        if status_cell.excluded || status_cell.is_empty() {
            continue;
        }
        let Some(time_cell) = table.times.get(row) else {
            continue;
        };
        if time_cell.excluded || time_cell.is_empty() {
            continue;
        }
        let Some(t) = time_cell.active_number() else {
            return Err(StatsError::InvalidParameter { name: "survival_time", value: f64::NAN });
        };
        if !t.is_finite() || t < 0.0 {
            return Err(StatsError::InvalidParameter { name: "survival_time", value: t });
        }
        let Some(st) = status_cell.active_number() else {
            return Err(StatsError::InvalidParameter { name: "survival_status", value: f64::NAN });
        };
        if let Some(status) = parse_survival_status_with_policy(st, event_code, censored_code, treat_other_as)? {
            out.push(SurvivalObservation { row, time: t, status });
        }
    }
    Ok(out)
}

/// Execute a [`SurvivalAnalysisSpec`] against a `Survival` or `MultipleVariables` [`DataSheet`].
///
/// # Edge cases (Kaplan–Meier)
///
/// A selected cohort with no included observations (every cell blank or excluded) is left out of
/// the analysis and listed in an `Info` checklist item and footnote; the remaining cohorts are
/// analysed as if it were not selected. Excluding one arm of a three-arm table therefore yields the
/// two-arm comparison and hazard ratios rather than an error. If no cohort has any included
/// observation the analysis returns [`StatsError::EmptyInput`].
pub fn run_survival_analysis(sheet: &DataSheet, spec: &AnalysisSpec, surv_spec: &SurvivalAnalysisSpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let confidence = spec.confidence_level;
    let p_fmt = spec.p_value_format;

    match surv_spec {
        SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox,
            gehan_breslow_wilcoxon,
            trend_test,
            asymmetrical_greenwood_ci,
            multiplicity,
            mv_time_column,
            mv_status_column,
            mv_group_column,
            event_code,
            censored_code,
            treat_other_as,
            ratio_median_ci,
            mv_status_columns,
            mv_group_columns,
        } => {
            let (mut cohorts, skipped_mv_rows, ignored_non_numeric) = match &sheet.archetype {
                TableArchetype::Survival(table) => {
                    validate_selected_data_sets(&spec.selected_data_sets, table.cohorts.len())?;
                    let indices = resolve_selected_indices(&spec.selected_data_sets, table.cohorts.len());
                    if indices.is_empty() {
                        return Err(StatsError::EmptyInput);
                    }
                    let mut list = Vec::with_capacity(indices.len());
                    for c_idx in indices {
                        let obs = extract_survival_table_cohort_with_policy(table, c_idx, *event_code, *censored_code, *treat_other_as)?;
                        list.push((c_idx, table.cohorts[c_idx].title.clone(), obs));
                    }
                    (list, 0_usize, Vec::new())
                }
                TableArchetype::MultipleVariables(mv) => {
                    let mv_ext = extract_mv_km_cohorts(
                        mv,
                        &spec.selected_data_sets,
                        *mv_time_column,
                        *mv_status_column,
                        *mv_group_column,
                        mv_status_columns,
                        mv_group_columns,
                        *event_code,
                        *censored_code,
                        *treat_other_as,
                    )?;
                    (mv_ext.cohorts, mv_ext.skipped_rows, mv_ext.ignored_non_numeric)
                }
                other => {
                    return Err(StatsError::IncompatibleTable { analysis: "Kaplan-Meier survival", actual: other.kind() });
                }
            };

            // Cohorts whose cells are all blank or excluded carry no information; drop them so a
            // fully excluded arm reduces the comparison instead of failing the whole analysis.
            let mut omitted_cohorts: Vec<String> = Vec::new();
            cohorts.retain(|(_, label, obs)| {
                if obs.is_empty() {
                    omitted_cohorts.push(label.clone());
                    false
                } else {
                    true
                }
            });
            if cohorts.is_empty() {
                return Err(StatsError::EmptyInput);
            }

            let indices: Vec<usize> = cohorts.iter().map(|(idx, _, _)| *idx).collect();
            let mut cohort_obs = Vec::with_capacity(cohorts.len());
            let mut km_results = Vec::with_capacity(cohorts.len());
            for (pos, (c_idx, label, obs)) in cohorts.into_iter().enumerate() {
                let km = kaplan_meier_curve(&label, &obs, *asymmetrical_greenwood_ci, confidence)?;
                if let Some(med) = km.median_survival {
                    push_scalar(&mut out, format!("Median survival ({})", km.label), Some(c_idx), med);
                }
                let mut curve_pts = Vec::with_capacity(km.steps.len() + 1);
                curve_pts.push(CurveSamplePoint { x: 0.0, y: 1.0, ci_lower: Some(1.0), ci_upper: Some(1.0), pred_lower: None, pred_upper: None });
                for s in &km.steps {
                    curve_pts.push(CurveSamplePoint {
                        x: s.time,
                        y: s.survival,
                        ci_lower: Some(s.ci_lower),
                        ci_upper: Some(s.ci_upper),
                        pred_lower: None,
                        pred_upper: None,
                    });
                }
                out.fitted_curves.push(FittedCurveOverlay {
                    data_set_index: c_idx,
                    equation_id: format!("Kaplan-Meier:{}", pos),
                    converged: true,
                    ambiguous: false,
                    points: curve_pts,
                });
                cohort_obs.push(obs);
                km_results.push(km);
            }

            // Median survival summary tab.
            let mut m_tab = TabBuilder::new("km-summary", "Kaplan-Meier cohort summary", ResultTabKind::MainSummary).headers([
                String::from("N total"),
                String::from("Events"),
                String::from("Censored"),
                String::from("Median survival"),
                format!("{}% CI (median)", confidence.percent()),
            ]);
            for km in &km_results {
                let med_ci = match km.median_ci {
                    (Some(lo), Some(hi)) => ci_cell(lo, hi),
                    (Some(lo), None) => {
                        ResultCell::ConfidenceInterval { lower: curvebook_doc::CiBound::Finite(lo), upper: curvebook_doc::CiBound::Open }
                    }
                    _ => ResultCell::Empty,
                };
                m_tab.push_row(
                    km.label.clone(),
                    [
                        num_cell(km.n_total as f64),
                        num_cell(km.n_events as f64),
                        num_cell(km.n_censored as f64),
                        opt_num_cell(km.median_survival),
                        med_ci,
                    ],
                );
            }
            out.tabs.push(m_tab.build());

            // Number at risk tab.
            let mut nar_tab = TabBuilder::new("number-at-risk", "Survival schedule & number at risk", ResultTabKind::NumberAtRisk).headers([
                String::from("Cohort"),
                String::from("Time"),
                String::from("At risk"),
                String::from("Events"),
                String::from("Survival"),
                format!("{}% CI", confidence.percent()),
            ]);
            for km in &km_results {
                for s in &km.steps {
                    nar_tab.push_row(
                        format!("{} @ {:.3}", km.label, s.time),
                        [
                            text_cell(km.label.clone()),
                            num_cell(s.time),
                            num_cell(s.n_at_risk as f64),
                            num_cell(s.n_events as f64),
                            num_cell(s.survival),
                            ci_cell(s.ci_lower, s.ci_upper),
                        ],
                    );
                }
            }
            out.tabs.push(nar_tab.build());

            // Curve comparisons if >= 2 cohorts.
            if cohort_obs.len() >= 2 {
                let refs: Vec<&[SurvivalObservation]> = cohort_obs.iter().map(Vec::as_slice).collect();
                let mut cmp_tab = TabBuilder::new("curve-comparison", "Comparison of survival curves", ResultTabKind::ModelComparison).headers([
                    "Chi-square",
                    "df",
                    "P value",
                ]);
                let mut cmp_count = 0_usize;
                let mut non_estimable_tests = Vec::new();
                let mut record_cmp = |cmp: SurvivalComparisonResult| {
                    if !cmp.chi_square.is_finite() {
                        non_estimable_tests.push(cmp.test_name.clone());
                    }
                    cmp_tab.push_row(
                        cmp.test_name,
                        [num_cell(cmp.chi_square), num_cell(cmp.df), p_cell_at_alpha(cmp.p_value, p_fmt, confidence.alpha())],
                    );
                    cmp_count += 1;
                };
                if *log_rank_mantel_cox {
                    let lr = compare_survival_curves(&refs, false)?;
                    push_scalar(&mut out, "Log-rank chi-square", None, lr.chi_square);
                    push_scalar(&mut out, "Log-rank P value", None, lr.p_value);
                    record_cmp(lr);
                }
                if *gehan_breslow_wilcoxon {
                    let gbw = compare_survival_curves(&refs, true)?;
                    push_scalar(&mut out, "Gehan-Breslow-Wilcoxon chi-square", None, gbw.chi_square);
                    push_scalar(&mut out, "Gehan-Breslow-Wilcoxon P", None, gbw.p_value);
                    record_cmp(gbw);
                }
                if *trend_test && refs.len() >= 3 {
                    record_cmp(log_rank_trend_test(&refs)?);
                }
                if cmp_count > 0 {
                    if !non_estimable_tests.is_empty() {
                        let msg = format!(
                            "Survival comparison statistic unavailable for {} because no events were observed across overlapping risk sets (zero covariance rank).",
                            non_estimable_tests.join(", ")
                        );
                        cmp_tab.push_footnote(msg.clone());
                        out.checklist.push(checklist_item(
                            "Survival curve comparison estimability",
                            ChecklistStatus::Info,
                            Some(format!("{} test(s) unavailable", non_estimable_tests.len())),
                            msg,
                        ));
                    }
                    out.tabs.push(cmp_tab.build());
                }

                if refs.len() == 2 {
                    let hr = two_cohort_hazard_ratio(refs[0], refs[1], confidence)?;
                    if let Some(v) = hr.log_rank_hr_opt() {
                        push_scalar(&mut out, "Hazard ratio (log-rank)", None, v);
                    }
                    if let Some(v) = hr.mantel_haenszel_hr_opt() {
                        push_scalar(&mut out, "Hazard ratio (Mantel-Haenszel)", None, v);
                    }
                    if let Some(v) = hr.peto_one_step_hr_opt() {
                        push_scalar(&mut out, "Hazard ratio (Peto one-step)", None, v);
                    }
                    out.tabs.push(
                        TabBuilder::new("hazard-ratio", "Hazard ratio", ResultTabKind::ParametersAndConfidenceIntervals)
                            .headers([String::from("Hazard ratio"), format!("{}% CI", confidence.percent())])
                            .row("Log-rank", [opt_num_cell(hr.log_rank_hr_opt()), opt_ci_cell(hr.log_rank_ci)])
                            .row("Mantel-Haenszel", [opt_num_cell(hr.mantel_haenszel_hr_opt()), opt_ci_cell(hr.mantel_haenszel_ci)])
                            .row("Peto one-step", [opt_num_cell(hr.peto_one_step_hr_opt()), opt_ci_cell(hr.peto_one_step_ci)])
                            .build(),
                    );

                    if *ratio_median_ci {
                        let mut mr_tab =
                            TabBuilder::new("km-median-ratio", "Ratio of median survival times", ResultTabKind::ParametersAndConfidenceIntervals)
                                .headers([String::from("Ratio"), format!("{}% CI", confidence.percent())]);
                        if let Some(mr) = median_survival_ratio_ci(&km_results[0], &km_results[1], confidence) {
                            push_scalar(&mut out, "Median survival ratio (A/B)", None, mr.ratio_a_over_b);
                            push_scalar(&mut out, "Median survival ratio (B/A)", None, mr.ratio_b_over_a);
                            mr_tab.push_row(
                                format!("{} / {}", km_results[0].label, km_results[1].label),
                                [num_cell(mr.ratio_a_over_b), opt_ci_cell(mr.ci_a_over_b)],
                            );
                            mr_tab.push_row(
                                format!("{} / {}", km_results[1].label, km_results[0].label),
                                [num_cell(mr.ratio_b_over_a), opt_ci_cell(mr.ci_b_over_a)],
                            );
                            mr_tab.push_footnote(
                                "Ratio CI propagates each cohort's independent Brookmeyer-Crowley median survival CI on the natural-log scale (omitted when either cohort's median CI is open-ended).",
                            );
                        } else {
                            mr_tab.push_row(format!("{} / {}", km_results[0].label, km_results[1].label), [ResultCell::Empty, ResultCell::Empty]);
                            mr_tab.push_footnote("Median survival ratio is unavailable because at least one cohort did not reach 50% survival.");
                        }
                        out.tabs.push(mr_tab.build());
                    }
                } else if refs.len() >= 3 {
                    append_survival_pairwise(&mut out, &indices, &km_results, &cohort_obs, multiplicity, p_fmt, confidence)?;
                }
            }

            let min_events = km_results.iter().map(|k| k.n_events).min().unwrap_or(0);
            let total_kept: usize = cohort_obs.iter().map(Vec::len).sum();
            out.checklist.push(checklist_item(
                "Event counts & censoring",
                if min_events >= 5 { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
                Some(format!("min cohort events = {min_events}")),
                "Kaplan-Meier assumes non-informative censoring; medians cannot be estimated if survival remains above 0.5.",
            ));
            append_omitted_cohorts_checklist_and_footnote(&mut out, &omitted_cohorts);
            append_listwise_filtered_checklist_and_footnote(&mut out, "Kaplan-Meier survival", skipped_mv_rows, total_kept);
            append_ignored_text_checklist_and_footnote(&mut out, "Kaplan-Meier survival", &ignored_non_numeric);
        }
        SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column,
            status_column,
            predictor_columns,
            reference_levels,
            tie_method,
            formula,
            covariate_profiles,
            nested_comparison,
            emit_cox_snell_residuals,
            ci_method,
            event_code,
            censored_code,
            treat_other_as,
        } => {
            let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
                return Err(StatsError::IncompatibleTable { analysis: "Cox proportional hazards", actual: sheet.archetype.kind() });
            };
            validate_selected_data_sets(&spec.selected_data_sets, mv.variables.len())?;
            let extraction = build_cox_design_subjects(
                mv,
                *time_column,
                *status_column,
                predictor_columns,
                reference_levels,
                formula.as_ref(),
                nested_comparison.as_ref(),
                covariate_profiles,
                *event_code,
                *censored_code,
                *treat_other_as,
            )?;
            let mut cox = cox_ph_regression(&extraction.subjects, &extraction.pred_names, *tie_method, confidence)?;
            let p = cox.covariates.len();
            let vifs = compute_cox_vifs(&extraction.subjects, p);
            let beta_hat: Vec<f64> = cox.covariates.iter().map(|c| c.beta).collect();
            let se_hat: Vec<f64> = cox.covariates.iter().map(|c| c.se).collect();
            let z_crit = norm_two_sided_isf(confidence.alpha())?;

            let (beta_cis, used_profile_ci) = if *ci_method == RegressionCiMethod::ProfileLikelihood && cox.converged {
                let prof = cox_profile_likelihood_ci(&extraction.subjects, &beta_hat, &se_hat, cox.log_lik_model, *tie_method, confidence)?;
                for (cov, &(lo, hi)) in cox.covariates.iter_mut().zip(prof.iter()) {
                    cov.hr_ci = (lo.exp(), hi.exp());
                }
                (prof, true)
            } else {
                let wald: Vec<(f64, f64)> = cox
                    .covariates
                    .iter()
                    .map(|c| if c.se.is_finite() { (c.beta - z_crit * c.se, c.beta + z_crit * c.se) } else { (f64::NAN, f64::NAN) })
                    .collect();
                (wald, false)
            };

            append_cox_ph_scalars_and_tabs(&mut out, &cox, &beta_cis, &vifs, *ci_method, used_profile_ci, confidence, p_fmt);
            append_cox_profile_curves(&mut out, &cox, &extraction.x_means, &beta_hat, &extraction.profile_vectors)?;
            if let Some((red_names, red_subjects)) = &extraction.nested_subjects {
                let red_cox = cox_ph_regression(red_subjects, red_names, *tie_method, confidence)?;
                append_cox_nested_comparison(&mut out, &cox, &red_cox, p_fmt, confidence.alpha())?;
            }
            if *emit_cox_snell_residuals {
                append_cox_snell_and_covariate_tabs(&mut out, &extraction.subjects, &extraction.row_labels, &extraction.pred_names, &beta_hat, &cox)?;
            }
            append_cox_ph_diagnostics(&mut out, &cox);
            append_listwise_filtered_checklist_and_footnote(&mut out, "Cox proportional hazards", extraction.skipped_rows, extraction.subjects.len());
            append_ignored_text_checklist_and_footnote(&mut out, "Cox proportional hazards", &extraction.ignored_non_numeric);
        }
    }

    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

/// Disclose Kaplan–Meier cohorts left out because none of their cells are included.
fn append_omitted_cohorts_checklist_and_footnote(out: &mut ResultSheet, omitted: &[String]) {
    if omitted.is_empty() {
        return;
    }
    let names = omitted.iter().map(|l| if l.is_empty() { "(untitled)" } else { l.as_str() }).collect::<Vec<_>>().join(", ");
    let msg = format!("{} selected group(s) left out of the analysis because every value is blank or excluded: {names}.", omitted.len());
    out.checklist.push(checklist_item(
        "Groups without data",
        ChecklistStatus::Info,
        Some(format!("{} group(s) left out", omitted.len())),
        msg.clone(),
    ));
    if let Some(tab) = out.tabs.first_mut() {
        tab.footnotes.push(msg);
    }
}

#[allow(clippy::too_many_arguments)]
fn append_cox_ph_scalars_and_tabs(
    out: &mut ResultSheet,
    cox: &CoxPhResult,
    beta_cis: &[(f64, f64)],
    vifs: &[Option<f64>],
    requested_ci: RegressionCiMethod,
    used_profile_ci: bool,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    let (aic, aicc) = cox_aic_and_aicc_by_events(cox.log_lik_model, cox.covariates.len(), cox.n_events);
    let has_non_estimable = cox.covariates.iter().any(|c| !c.se.is_finite());
    push_scalar(out, "Converged", None, if cox.converged { 1.0 } else { 0.0 });
    push_scalar(out, "Iterations", None, cox.iterations as f64);
    push_scalar(out, "N", None, cox.n_subjects as f64);
    push_scalar(out, "Events", None, cox.n_events as f64);
    push_scalar(out, "Log-likelihood (null)", None, cox.log_lik_null);
    push_scalar(out, "Log-likelihood (model)", None, cox.log_lik_model);
    if let Some(c_index) = cox.concordance {
        push_scalar(out, "Concordance (C-index)", None, c_index);
    }
    push_scalar(out, "Comparable pairs", None, cox.comparable_pairs as f64);
    push_scalar(out, "Likelihood ratio chi2", None, cox.likelihood_ratio_chi2);
    push_scalar(out, "Likelihood ratio P", None, cox.likelihood_ratio_p);
    push_scalar(out, "Global Schoenfeld P", None, cox.global_schoenfeld_p);
    push_scalar(out, "AIC", None, aic);
    push_scalar(out, "AICc", None, aicc);

    let mut cox_tab = TabBuilder::new("cox-ph", "Cox proportional hazards estimates", ResultTabKind::ParametersAndConfidenceIntervals).headers([
        String::from("beta"),
        String::from("SE"),
        String::from("Wald z"),
        String::from("P value"),
        String::from("Hazard Ratio"),
        format!("{}% CI (HR)", confidence.percent()),
        String::from("Schoenfeld P"),
    ]);
    let mut labels = Vec::with_capacity(cox.covariates.len());
    for cov in &cox.covariates {
        labels.push(cov.name.clone());
        cox_tab.push_row(
            cov.name.clone(),
            [
                num_cell(cov.beta),
                num_cell(cov.se),
                num_cell(cov.z_statistic),
                p_cell_at_alpha(cov.p_value, p_fmt, confidence.alpha()),
                num_cell(cov.hazard_ratio),
                ci_cell(cov.hr_ci.0, cov.hr_ci.1),
                p_cell_at_alpha(cov.schoenfeld_p, p_fmt, confidence.alpha()),
            ],
        );
    }
    if !cox.converged || has_non_estimable {
        cox_tab.push_footnote(
            "Model did not converge or exhibited separation/collinearity; non-estimable SE, Wald Z/P, and HR CI bounds are omitted. Refer to Likelihood ratio P for the omnibus test.",
        );
    } else if !cox.global_schoenfeld_p.is_finite() {
        cox_tab.push_footnote(
            "Schoenfeld proportional-hazards residual test is non-estimable when fewer than 3 events (or fewer than 2 distinct event times) are observed.",
        );
    }
    if requested_ci == RegressionCiMethod::ProfileLikelihood && !used_profile_ci {
        cox_tab.push_footnote("Profile-likelihood CIs require a converged Cox PH fit; asymptotic Wald CIs reported instead.");
    }
    if cox.concordance.is_none() {
        cox_tab.push_footnote("Harrell's concordance index (C-index) is unavailable (\"—\") because 0 comparable subject pairs exist.");
    }
    out.tabs.push(cox_tab.build());

    let pct = confidence.percent();
    let ci_lbl = if used_profile_ci { "Profile likelihood" } else { "Wald" };
    let mut beta_ci_tab =
        TabBuilder::new("cox-beta-cis", "Cox parameter CIs, delta-method SE(HR), and covariate VIF", ResultTabKind::ParametersAndConfidenceIntervals)
            .headers([
                String::from("beta"),
                String::from("SE (beta)"),
                format!("{pct}% CI (beta)"),
                String::from("Hazard Ratio"),
                String::from("SE (HR)"),
                format!("{pct}% CI (HR)"),
                String::from("VIF"),
                String::from("CI method"),
            ]);
    for ((cov, &(b_lo, b_hi)), &vif) in cox.covariates.iter().zip(beta_cis.iter()).zip(vifs.iter()) {
        let se_hr = if cov.hazard_ratio.is_finite() && cov.se.is_finite() { cov.hazard_ratio * cov.se } else { f64::NAN };
        beta_ci_tab.push_row(
            cov.name.clone(),
            [
                num_cell(cov.beta),
                num_cell(cov.se),
                finite_ci_cell(b_lo, b_hi),
                num_cell(cov.hazard_ratio),
                num_cell(se_hr),
                finite_ci_cell(cov.hr_ci.0, cov.hr_ci.1),
                opt_num_cell(vif),
                text_cell(ci_lbl),
            ],
        );
    }
    out.tabs.push(beta_ci_tab.build());

    append_cox_baseline_tabs(out, &cox.baseline_survival);

    let mut corr_tab = TabBuilder::new("cox-covariance", "Cox parameter correlation matrix", ResultTabKind::CovarianceMatrix).headers(labels.clone());
    for (lbl, row) in labels.into_iter().zip(cox.correlation_matrix.iter()) {
        corr_tab.push_row(lbl, row.iter().map(|&v| num_cell(v)));
    }
    out.tabs.push(corr_tab.build());

    append_cox_curve_overlays(out, cox);
}

fn append_cox_profile_curves(
    out: &mut ResultSheet,
    cox: &CoxPhResult,
    x_means: &[f64],
    beta_hat: &[f64],
    profile_vectors: &[(String, Vec<f64>)],
) -> Result<(), StatsError> {
    if profile_vectors.is_empty() || cox.baseline_survival.is_empty() {
        return Ok(());
    }
    let mut schedules = Vec::with_capacity(profile_vectors.len());
    let mut headers = vec![String::from("Time")];
    for (idx, (label, vec_x)) in profile_vectors.iter().enumerate() {
        let sched = predict_cox_profile_survival(&cox.baseline_survival, x_means, beta_hat, vec_x)?;
        headers.push(format!("{label} H(t)"));
        headers.push(format!("{label} S(t)"));
        let prepend_zero = sched.first().is_some_and(|p| p.time > 0.0);
        let mut pts = Vec::with_capacity(sched.len() + usize::from(prepend_zero));
        if prepend_zero {
            pts.push(CurveSamplePoint { x: 0.0, y: 1.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None });
        }
        for pt in &sched {
            pts.push(CurveSamplePoint { x: pt.time, y: pt.survival, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None });
        }
        out.fitted_curves.push(FittedCurveOverlay {
            data_set_index: idx,
            equation_id: format!("cox_profile:{idx}"),
            converged: cox.converged,
            ambiguous: false,
            points: pts,
        });
        schedules.push(sched);
    }

    let mut tab =
        TabBuilder::new("cox-profile-survival", "Cox covariate-profile predicted survival curves", ResultTabKind::BaselineSurvival).headers(headers);
    if cox.baseline_survival.first().is_some_and(|p| p.time > 0.0) {
        let mut row0 = vec![num_cell(0.0)];
        for _ in &schedules {
            row0.push(num_cell(0.0));
            row0.push(num_cell(1.0));
        }
        tab.push_row("t = 0.0000", row0);
    }
    for (step_idx, base_pt) in cox.baseline_survival.iter().enumerate() {
        let mut row = vec![num_cell(base_pt.time)];
        for sched in &schedules {
            row.push(num_cell(sched[step_idx].cum_hazard));
            row.push(num_cell(sched[step_idx].survival));
        }
        tab.push_row(format!("t = {:.4}", base_pt.time), row);
    }
    out.tabs.push(tab.build());
    Ok(())
}

fn append_cox_nested_comparison(
    out: &mut ResultSheet,
    primary: &CoxPhResult,
    nested: &CoxPhResult,
    p_fmt: PValueFormat,
    alpha: f64,
) -> Result<(), StatsError> {
    let (full, red) = if primary.covariates.len() >= nested.covariates.len() { (primary, nested) } else { (nested, primary) };
    let df = (full.covariates.len() as f64) - (red.covariates.len() as f64);
    let (_, full_aicc) = cox_aic_and_aicc_by_events(full.log_lik_model, full.covariates.len(), full.n_events);
    let (_, red_aicc) = cox_aic_and_aicc_by_events(red.log_lik_model, red.covariates.len(), red.n_events);
    let delta_aicc = full_aicc - red_aicc;
    let (chi2, p_val) = if df > 0.0 {
        let c = (2.0 * (full.log_lik_model - red.log_lik_model)).max(0.0);
        (c, chi2_sf(c, df)?)
    } else {
        (f64::NAN, f64::NAN)
    };
    push_scalar(out, "Cox nested LRT chi2", None, chi2);
    push_scalar(out, "Cox nested LRT df", None, df);
    push_scalar(out, "Cox nested LRT P", None, p_val);
    push_scalar(out, "Cox Delta AICc (full - reduced)", None, delta_aicc);

    let mut kv = KvTabBuilder::new("cox-nested-comparison", "Nested Cox PH likelihood-ratio test", ResultTabKind::ModelComparison, "Value")
        .num("Full model log-likelihood", full.log_lik_model)
        .num("Reduced model log-likelihood", red.log_lik_model)
        .num("Full model parameters", full.covariates.len() as f64)
        .num("Reduced model parameters", red.covariates.len() as f64)
        .num("LRT chi-square", chi2)
        .num("LRT df", df)
        .p("LRT P value", p_val, p_fmt, alpha)
        .num("Full model AICc (by events)", full_aicc)
        .num("Reduced model AICc (by events)", red_aicc)
        .num("Delta AICc (full - reduced)", delta_aicc);
    if df <= 0.0 {
        kv = kv.footnote("Nested Cox LRT requires strictly different parameter counts between the full and reduced models.");
    }
    out.tabs.push(kv.build());
    Ok(())
}

fn append_cox_snell_and_covariate_tabs(
    out: &mut ResultSheet,
    subjects: &[CoxSubject],
    row_labels: &[String],
    pred_names: &[String],
    beta_hat: &[f64],
    cox: &CoxPhResult,
) -> Result<(), StatsError> {
    let cs_rows = compute_cox_snell_rows(subjects, beta_hat, &cox.martingale_residuals, &cox.deviance_residuals)?;
    let mut cs_tab = TabBuilder::new("cox-snell-residuals", "Cox-Snell, Martingale, and Deviance residuals", ResultTabKind::Residuals).headers([
        "Time",
        "Event",
        "Linear predictor",
        "Cox-Snell residual",
        "Martingale residual",
        "Deviance residual",
        "KM Cumulative Hazard H(r_CS)",
    ]);
    for (lbl, row) in row_labels.iter().zip(cs_rows.iter()) {
        cs_tab.push_row(
            lbl.clone(),
            [
                num_cell(row.time),
                num_cell(if row.event { 1.0 } else { 0.0 }),
                num_cell(row.linear_predictor),
                num_cell(row.cox_snell),
                num_cell(row.martingale),
                num_cell(row.deviance),
                opt_num_cell(row.km_cum_hazard),
            ],
        );
    }
    out.tabs.push(cs_tab.build());

    let mut cov_headers = vec![String::from("Time"), String::from("Event"), String::from("Martingale residual")];
    cov_headers.extend(pred_names.iter().cloned());
    let mut cov_tab =
        TabBuilder::new("cox-covariate-residuals", "Cox Martingale residuals vs covariates", ResultTabKind::Residuals).headers(cov_headers);
    for (i, (lbl, subj)) in row_labels.iter().zip(subjects.iter()).enumerate() {
        let mut row = vec![num_cell(subj.time), num_cell(if subj.event { 1.0 } else { 0.0 }), num_cell(cox.martingale_residuals[i])];
        for &xv in &subj.x {
            row.push(num_cell(xv));
        }
        cov_tab.push_row(lbl.clone(), row);
    }
    out.tabs.push(cov_tab.build());
    Ok(())
}

fn append_cox_ph_diagnostics(out: &mut ResultSheet, cox: &CoxPhResult) {
    let has_non_estimable = cox.covariates.iter().any(|c| !c.se.is_finite());
    out.checklist.push(checklist_item(
        "Cox PH convergence & estimability",
        if cox.converged && !has_non_estimable { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(format!("Converged = {}, iterations = {}", cox.converged, cox.iterations)),
        if cox.converged && !has_non_estimable {
            "Partial-likelihood Newton-Raphson converged with a positive-definite covariate information matrix."
        } else {
            "Partial-likelihood Newton-Raphson failed to converge or encountered monotone-likelihood separation / covariate collinearity; non-estimable Wald SE, Z, P, and HR CI entries are omitted, while Likelihood ratio P tests the omnibus fit."
        },
    ));

    let (ph_status, ph_metric, ph_expl) = if cox.global_schoenfeld_p.is_finite() {
        let ph_ok = cox.global_schoenfeld_p >= 0.05;
        (
            if ph_ok { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
            Some(format!("Global chi2 = {:.3}, P = {:.4}", cox.global_schoenfeld_chi2, cox.global_schoenfeld_p)),
            if ph_ok {
                "Scaled Schoenfeld residuals show no significant time-varying slope (PH assumption satisfied)."
            } else {
                "Schoenfeld residual test indicates time-varying hazard ratios (P < 0.05), violating the proportional hazards assumption."
            },
        )
    } else {
        (
            ChecklistStatus::Warn,
            Some(String::from("Global Schoenfeld test unavailable")),
            if cox.n_events < 3 {
                "Global Schoenfeld proportional-hazards test is non-estimable with fewer than 3 events."
            } else {
                "Global Schoenfeld proportional-hazards test cannot be computed when the covariate covariance matrix or distinct event-time variance is non-estimable."
            },
        )
    };
    out.checklist.push(checklist_item("Proportional hazards assumption (Schoenfeld)", ph_status, ph_metric, ph_expl));
    let (c_metric, c_expl) = match cox.concordance {
        Some(c_idx) => (
            format!("C = {c_idx:.4} ({} comparable pairs)", cox.comparable_pairs),
            "Harrell's C-index measures pairwise risk discrimination over all comparable subject pairs (excluding tied event times).",
        ),
        None => (
            format!("— ({} comparable pairs)", cox.comparable_pairs),
            "Harrell's C-index is unavailable because 0 comparable pairs remain (e.g. all events occurred at tied times or no event precedes a longer follow-up).",
        ),
    };
    out.checklist.push(checklist_item("Model discrimination (Harrell's C-index)", ChecklistStatus::Info, Some(c_metric), c_expl));
}

fn append_cox_baseline_tabs(out: &mut ResultSheet, baseline: &[CoxBaselinePoint]) {
    let mut base_tab = TabBuilder::new("cox-baseline-survival", "Cox baseline survival curve (at mean covariates)", ResultTabKind::BaselineSurvival)
        .headers(["Time", "Cumulative Hazard H0(t)", "Baseline Survival S0(t)"])
        .footnote("Evaluated at sample covariate means using the fitted cumulative hazard step function.");
    if baseline.first().is_some_and(|p| p.time > 0.0) {
        base_tab.push_row("t = 0.0000", [num_cell(0.0), num_cell(0.0), num_cell(1.0)]);
    }
    for pt in baseline {
        base_tab.push_row(format!("t = {:.4}", pt.time), [num_cell(pt.time), num_cell(pt.cum_hazard), num_cell(pt.survival)]);
    }
    out.tabs.push(base_tab.build());

    let mut lml_tab = TabBuilder::new("cox-log-minus-log", "Cox log-minus-log survival coordinates", ResultTabKind::LogMinusLog).headers([
        "Time",
        "ln(Time)",
        "Baseline Survival S0(t)",
        "ln(-ln S0(t))",
    ]);
    for pt in baseline {
        if pt.cum_hazard > 0.0 && pt.survival > 0.0 && pt.survival < 1.0 {
            let log_t = (pt.time > 0.0).then(|| pt.time.ln());
            lml_tab.push_row(
                format!("t = {:.4}", pt.time),
                [num_cell(pt.time), opt_num_cell(log_t), num_cell(pt.survival), num_cell(pt.cum_hazard.ln())],
            );
        }
    }
    out.tabs.push(lml_tab.build());
}

/// Build Cox PH predictor names, complete-case subjects, optional reduced nested subjects,
/// covariate sample means, and compiled covariate-profile vectors.
struct CoxDesignExtraction {
    pred_names: Vec<String>,
    subjects: Vec<CoxSubject>,
    row_labels: Vec<String>,
    x_means: Vec<f64>,
    nested_subjects: Option<(Vec<String>, Vec<CoxSubject>)>,
    profile_vectors: Vec<(String, Vec<f64>)>,
    skipped_rows: usize,
    ignored_non_numeric: Vec<(String, usize)>,
}

fn cell_has_entry(cell: Option<&Cell>) -> bool {
    cell.is_some_and(|c| !c.is_empty() || c.excluded)
}

enum RetainedCoxPred {
    Categorical(String),
    Numeric(f64),
}

struct RetainedCoxRow {
    row_num: usize,
    time: f64,
    event: bool,
    preds: Vec<Option<RetainedCoxPred>>,
}

type CoxCatLevels = Vec<Option<Vec<String>>>;

fn resolve_cox_cat_levels(
    mv: &MultipleVariablesTable,
    predictor_columns: &[usize],
    required_mask: &[bool],
    is_cat: &[bool],
    retained_rows: &[RetainedCoxRow],
    reference_levels: &[(usize, String)],
) -> Result<CoxCatLevels, StatsError> {
    let mut cat_levels = Vec::with_capacity(predictor_columns.len());
    for (pos, &pc) in predictor_columns.iter().enumerate() {
        let pv = &mv.variables[pc];
        if !is_cat[pos] || !required_mask[pos] {
            cat_levels.push(None);
            continue;
        }
        let cells: Vec<Cell> = retained_rows
            .iter()
            .filter_map(|row| match row.preds.get(pos).and_then(Option::as_ref) {
                Some(RetainedCoxPred::Categorical(tok)) => Some(Cell::text(tok.as_str())),
                _ => None,
            })
            .collect();
        let retained_col = VariableColumn {
            title: pv.title.clone(),
            role: pv.role,
            role_inferred: pv.role_inferred,
            source: pv.source.clone(),
            level_order: pv.level_order.clone(),
            cells,
            display_decimals: pv.display_decimals,
        };
        let mut lvls: Vec<String> =
            retained_col.ordered_levels_with(&pv.level_order).into_iter().filter(|s| s.included_count > 0).map(|s| s.level).collect();
        let explicit_ref = reference_levels.iter().rev().find(|(idx, _)| *idx == pc).map(|(_, l)| l.clone());
        let ref_lvl = match explicit_ref {
            Some(req) => {
                if !lvls.iter().any(|existing| existing == &req) {
                    return Err(StatsError::InvalidParameter { name: "reference_levels", value: pc as f64 });
                }
                Some(req)
            }
            None => lvls.first().cloned(),
        };
        if let Some(rl) = ref_lvl {
            lvls.retain(|l| l != &rl);
            let mut ordered = Vec::with_capacity(lvls.len() + 1);
            ordered.push(rl);
            ordered.extend(lvls);
            cat_levels.push(Some(ordered));
        } else {
            cat_levels.push(None);
        }
    }
    Ok(cat_levels)
}

enum OwnedCoxFormulaPred {
    Continuous { name: String, values: Vec<f64> },
    Categorical { name: String, levels: Vec<String>, level_indices: Vec<usize> },
}

impl OwnedCoxFormulaPred {
    fn as_col(&self) -> FormulaPredictorColumn<'_> {
        match self {
            Self::Continuous { name, values } => FormulaPredictorColumn::Continuous { name: name.as_str(), values: values.as_slice() },
            Self::Categorical { name, levels, level_indices } => {
                FormulaPredictorColumn::Categorical { name: name.as_str(), levels: levels.as_slice(), level_indices: level_indices.as_slice() }
            }
        }
    }
}

fn compile_cox_profiles(
    mv: &MultipleVariablesTable,
    pred_cols: &[usize],
    required_mask: &[bool],
    cat_levels: &[Option<Vec<String>>],
    cont_means: &[f64],
    primary_local: &ModelFormulaSpec,
    profiles: &[CovariateProfile],
) -> Result<Vec<(String, Vec<f64>)>, StatsError> {
    let mut out = Vec::with_capacity(profiles.len());
    for (p_idx, prof) in profiles.iter().enumerate() {
        let mut assigned_num: Vec<Option<f64>> = vec![None; pred_cols.len()];
        for &(col_idx, val) in &prof.numeric_values {
            let Some(pos) = pred_cols.iter().position(|&pc| pc == col_idx) else {
                return Err(StatsError::InvalidParameter { name: "covariate_profile_column", value: col_idx as f64 });
            };
            if !required_mask[pos] {
                return Err(StatsError::InvalidParameter { name: "covariate_profile_column", value: col_idx as f64 });
            }
            if cat_levels[pos].is_some() {
                return Err(StatsError::IncompatibleAnalysisSpec {
                    expected: "numeric covariate profile entry must target a continuous predictor column",
                });
            }
            if !val.is_finite() {
                return Err(StatsError::InvalidParameter { name: "covariate_profile_value", value: val });
            }
            assigned_num[pos] = Some(val);
        }

        let mut assigned_cat: Vec<Option<usize>> = vec![None; pred_cols.len()];
        for (col_idx, lbl) in &prof.categorical_values {
            let Some(pos) = pred_cols.iter().position(|&pc| pc == *col_idx) else {
                return Err(StatsError::InvalidParameter { name: "covariate_profile_column", value: *col_idx as f64 });
            };
            if !required_mask[pos] {
                return Err(StatsError::InvalidParameter { name: "covariate_profile_column", value: *col_idx as f64 });
            }
            let Some(levels) = &cat_levels[pos] else {
                return Err(StatsError::IncompatibleAnalysisSpec {
                    expected: "categorical covariate profile entry must target a categorical predictor column",
                });
            };
            let lvl_idx = levels
                .iter()
                .position(|l| l == lbl)
                .ok_or(StatsError::InvalidParameter { name: "covariate_profile_level", value: *col_idx as f64 })?;
            assigned_cat[pos] = Some(lvl_idx);
        }

        let mut owned = Vec::with_capacity(pred_cols.len());
        for (pos, &pc) in pred_cols.iter().enumerate() {
            let title = mv.variables[pc].title.clone();
            if !required_mask[pos] {
                owned.push(OwnedCoxFormulaPred::Continuous { name: title, values: vec![0.0] });
                continue;
            }
            if let Some(levels) = &cat_levels[pos] {
                let lvl_idx = assigned_cat[pos].unwrap_or(0);
                owned.push(OwnedCoxFormulaPred::Categorical { name: title, levels: levels.clone(), level_indices: vec![lvl_idx] });
            } else {
                let val = assigned_num[pos].unwrap_or(cont_means[pos]);
                if !val.is_finite() {
                    return Err(StatsError::InvalidParameter { name: "covariate_profile_value", value: val });
                }
                owned.push(OwnedCoxFormulaPred::Continuous { name: title, values: vec![val] });
            }
        }
        let refs: Vec<FormulaPredictorColumn<'_>> = owned.iter().map(OwnedCoxFormulaPred::as_col).collect();
        let compiled = compile_formula_design(&refs, primary_local, CategoricalCoding::ReferenceFirst)?;
        let p = compiled.matrix.ncols();
        let x_vec: Vec<f64> = (0..p).map(|j| compiled.matrix.get(0, j).unwrap_or(0.0)).collect();
        let label = if prof.label.trim().is_empty() { format!("Profile {}", p_idx + 1) } else { prof.label.clone() };
        out.push((label, x_vec));
    }
    Ok(out)
}

struct CoxCollectedRows {
    retained_rows: Vec<RetainedCoxRow>,
    skipped_rows: usize,
    ignored_non_numeric: Vec<(String, usize)>,
}

#[allow(clippy::too_many_arguments)]
fn collect_cox_complete_rows(
    mv: &MultipleVariablesTable,
    time_column: usize,
    status_column: usize,
    pred_cols: &[usize],
    required_mask: &[bool],
    is_cat: &[bool],
    event_code: Option<f64>,
    censored_code: Option<f64>,
    treat_other_as: SurvivalOtherValuePolicy,
) -> Result<CoxCollectedRows, StatsError> {
    let time_var = &mv.variables[time_column];
    let status_var = &mv.variables[status_column];
    let active_end = (0..mv.row_count()).rfind(|&r| {
        cell_has_entry(time_var.cells.get(r))
            || cell_has_entry(status_var.cells.get(r))
            || pred_cols.iter().enumerate().any(|(pos, &pc)| required_mask[pos] && cell_has_entry(mv.variables[pc].cells.get(r)))
    });
    let mut retained_rows = Vec::new();
    let mut skipped_rows = 0_usize;
    let mut time_non_num = 0_usize;
    let mut status_non_num = 0_usize;
    let mut pred_non_num = vec![0_usize; pred_cols.len()];

    if let Some(end_row) = active_end {
        for r in 0..=end_row {
            let tc = time_var.cells.get(r);
            let sc = status_var.cells.get(r);
            let has_entry = cell_has_entry(tc)
                || cell_has_entry(sc)
                || pred_cols.iter().enumerate().any(|(pos, &pc)| required_mask[pos] && cell_has_entry(mv.variables[pc].cells.get(r)));
            if !has_entry {
                continue;
            }
            if tc.is_some_and(is_ignored_non_numeric_cell) {
                time_non_num += 1;
            }
            if sc.is_some_and(is_ignored_non_numeric_cell) {
                status_non_num += 1;
            }
            for (pos, &pc) in pred_cols.iter().enumerate() {
                if required_mask[pos] && !is_cat[pos] && mv.variables[pc].cells.get(r).is_some_and(is_ignored_non_numeric_cell) {
                    pred_non_num[pos] += 1;
                }
            }
            let (Some(t), Some(st)) = (tc.and_then(Cell::active_number), sc.and_then(Cell::active_number)) else {
                skipped_rows += 1;
                continue;
            };
            let Some(status) = parse_survival_status_with_policy(st, event_code, censored_code, treat_other_as)? else {
                skipped_rows += 1;
                continue;
            };
            let mut row_preds = Vec::with_capacity(pred_cols.len());
            let mut ok = true;
            for (pos, &pc) in pred_cols.iter().enumerate() {
                if !required_mask[pos] {
                    row_preds.push(None);
                    continue;
                }
                let Some(cell) = mv.variables[pc].cells.get(r) else {
                    ok = false;
                    break;
                };
                if cell.excluded {
                    ok = false;
                    break;
                }
                if is_cat[pos] {
                    let Some(tok) = mv_survival_token(cell) else {
                        ok = false;
                        break;
                    };
                    row_preds.push(Some(RetainedCoxPred::Categorical(tok)));
                } else if let Some(xv) = cell.active_number() {
                    row_preds.push(Some(RetainedCoxPred::Numeric(xv)));
                } else {
                    ok = false;
                    break;
                }
            }
            if ok {
                retained_rows.push(RetainedCoxRow { row_num: r + 1, time: t, event: matches!(status, SurvivalStatus::Event), preds: row_preds });
            } else {
                skipped_rows += 1;
            }
        }
    }

    let mut ignored_non_numeric = Vec::new();
    if time_non_num > 0 {
        ignored_non_numeric.push((time_var.title.clone(), time_non_num));
    }
    if status_non_num > 0 {
        ignored_non_numeric.push((status_var.title.clone(), status_non_num));
    }
    for (pos, &count) in pred_non_num.iter().enumerate() {
        if count > 0 {
            ignored_non_numeric.push((mv.variables[pred_cols[pos]].title.clone(), count));
        }
    }
    Ok(CoxCollectedRows { retained_rows, skipped_rows, ignored_non_numeric })
}

fn assemble_cox_owned_predictors(
    mv: &MultipleVariablesTable,
    pred_cols: &[usize],
    required_mask: &[bool],
    cat_levels: &[Option<Vec<String>>],
    retained_rows: &[RetainedCoxRow],
) -> (Vec<OwnedCoxFormulaPred>, Vec<f64>) {
    let n_obs = retained_rows.len();
    let mut owned_preds = Vec::with_capacity(pred_cols.len());
    let mut cont_means = vec![0.0_f64; pred_cols.len()];
    for (pos, &pc) in pred_cols.iter().enumerate() {
        let title = mv.variables[pc].title.clone();
        if !required_mask[pos] {
            owned_preds.push(OwnedCoxFormulaPred::Continuous { name: title, values: vec![0.0; n_obs] });
            continue;
        }
        if let Some(levels) = &cat_levels[pos] {
            let mut lidx = Vec::with_capacity(n_obs);
            for row in retained_rows {
                let tok = match row.preds[pos].as_ref() {
                    Some(RetainedCoxPred::Categorical(s)) => s.as_str(),
                    _ => "",
                };
                lidx.push(levels.iter().position(|l| l == tok).unwrap_or(0));
            }
            owned_preds.push(OwnedCoxFormulaPred::Categorical { name: title, levels: levels.clone(), level_indices: lidx });
        } else {
            let mut vals = Vec::with_capacity(n_obs);
            let mut sum = 0.0_f64;
            for row in retained_rows {
                let v = match row.preds[pos].as_ref() {
                    Some(RetainedCoxPred::Numeric(x)) => *x,
                    _ => 0.0,
                };
                sum += v;
                vals.push(v);
            }
            cont_means[pos] = sum / (n_obs as f64);
            owned_preds.push(OwnedCoxFormulaPred::Continuous { name: title, values: vals });
        }
    }
    (owned_preds, cont_means)
}

#[allow(clippy::too_many_arguments)]
fn build_cox_design_subjects(
    mv: &MultipleVariablesTable,
    time_column: usize,
    status_column: usize,
    predictor_columns: &[usize],
    reference_levels: &[(usize, String)],
    formula: Option<&ModelFormulaSpec>,
    nested_comparison: Option<&ModelFormulaSpec>,
    covariate_profiles: &[CovariateProfile],
    event_code: Option<f64>,
    censored_code: Option<f64>,
    treat_other_as: SurvivalOtherValuePolicy,
) -> Result<CoxDesignExtraction, StatsError> {
    if time_column >= mv.variables.len() {
        return Err(StatsError::MissingColumn { index: time_column });
    }
    if status_column >= mv.variables.len() {
        return Err(StatsError::MissingColumn { index: status_column });
    }
    if time_column == status_column {
        return Err(StatsError::InvalidParameter { name: "status_column", value: status_column as f64 });
    }

    let mut pred_cols = predictor_columns.to_vec();
    if pred_cols.is_empty()
        && let Some(f) = formula
    {
        for c in collect_formula_columns(f).into_iter().chain(nested_comparison.into_iter().flat_map(collect_formula_columns)) {
            if !pred_cols.contains(&c) {
                pred_cols.push(c);
            }
        }
    }
    let mut is_cat = Vec::with_capacity(pred_cols.len());
    let mut seen_preds = Vec::with_capacity(pred_cols.len());
    for &pc in &pred_cols {
        let pv = mv.variables.get(pc).ok_or(StatsError::MissingColumn { index: pc })?;
        if pc == time_column || pc == status_column || seen_preds.contains(&pc) {
            return Err(StatsError::InvalidParameter { name: "predictor_columns", value: pc as f64 });
        }
        seen_preds.push(pc);
        is_cat.push(pv.role == VariableRole::Categorical);
    }
    for &(ref_col, _) in reference_levels {
        if ref_col >= mv.variables.len() {
            return Err(StatsError::MissingColumn { index: ref_col });
        }
    }
    if pred_cols.is_empty() {
        return Err(StatsError::InsufficientSampleSize { context: "Cox PH regression", required: 1, actual: 0 });
    }

    let mut primary_local = match formula {
        Some(f) => remap_formula_columns(f, &pred_cols)?,
        None => ModelFormulaSpec { include_intercept: false, terms: (0..pred_cols.len()).map(FormulaTerm::Main).collect() },
    };
    primary_local.include_intercept = false;

    let nested_local = nested_comparison
        .map(|nf| {
            let mut m = remap_formula_columns(nf, &pred_cols)?;
            m.include_intercept = false;
            Ok::<ModelFormulaSpec, StatsError>(m)
        })
        .transpose()?;

    let mut required_mask = vec![false; pred_cols.len()];
    for c in collect_formula_columns(&primary_local).into_iter().chain(nested_local.iter().flat_map(collect_formula_columns)) {
        if let Some(slot) = required_mask.get_mut(c) {
            *slot = true;
        }
    }

    let CoxCollectedRows { retained_rows, skipped_rows, ignored_non_numeric } =
        collect_cox_complete_rows(mv, time_column, status_column, &pred_cols, &required_mask, &is_cat, event_code, censored_code, treat_other_as)?;

    // Derive observed categorical levels and reference levels strictly from complete-case retained rows
    // so incomplete/excluded rows never introduce all-zero phantom dummy columns.
    let cat_levels = resolve_cox_cat_levels(mv, &pred_cols, &required_mask, &is_cat, &retained_rows, reference_levels)?;
    for (pos, lvl_opt) in cat_levels.iter().enumerate() {
        if required_mask[pos] && is_cat[pos] && !retained_rows.is_empty() && lvl_opt.as_ref().is_none_or(|l| l.len() < 2) {
            return Err(StatsError::ZeroVariance { context: "Cox PH predictor has fewer than 2 levels after complete-case filtering" });
        }
    }

    let n_obs = retained_rows.len();
    if n_obs == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "Cox PH regression", required: 3, actual: 0 });
    }

    let (owned_preds, cont_means) = assemble_cox_owned_predictors(mv, &pred_cols, &required_mask, &cat_levels, &retained_rows);
    let pred_refs: Vec<FormulaPredictorColumn<'_>> = owned_preds.iter().map(OwnedCoxFormulaPred::as_col).collect();
    let compiled = compile_formula_design(&pred_refs, &primary_local, CategoricalCoding::ReferenceFirst)?;
    let p_cols = compiled.matrix.ncols();
    if p_cols == 0 {
        return Err(StatsError::ZeroVariance { context: "Cox PH predictor has fewer than 2 levels after complete-case filtering" });
    }

    let mut subjects = Vec::with_capacity(n_obs);
    let mut row_labels = Vec::with_capacity(n_obs);
    let mut x_means = vec![0.0_f64; p_cols];
    for (r, row) in retained_rows.iter().enumerate() {
        let mut x_vals = Vec::with_capacity(p_cols);
        for j in 0..p_cols {
            let v = compiled.matrix.get(r, j).unwrap_or(0.0);
            x_means[j] += v;
            x_vals.push(v);
        }
        row_labels.push(format!("Row {}", row.row_num));
        subjects.push(CoxSubject { time: row.time, event: row.event, x: x_vals });
    }
    for m in &mut x_means {
        *m /= n_obs as f64;
    }

    let nested_subjects = if let Some(nf) = &nested_local {
        let red_c = compile_formula_design(&pred_refs, nf, CategoricalCoding::ReferenceFirst)?;
        let red_p = red_c.matrix.ncols();
        let red_subs: Vec<CoxSubject> = retained_rows
            .iter()
            .enumerate()
            .map(|(r, row)| CoxSubject { time: row.time, event: row.event, x: (0..red_p).map(|j| red_c.matrix.get(r, j).unwrap_or(0.0)).collect() })
            .collect();
        Some((red_c.column_names, red_subs))
    } else {
        None
    };

    let profile_vectors = compile_cox_profiles(mv, &pred_cols, &required_mask, &cat_levels, &cont_means, &primary_local, covariate_profiles)?;

    Ok(CoxDesignExtraction {
        pred_names: compiled.column_names,
        subjects,
        row_labels,
        x_means,
        nested_subjects,
        profile_vectors,
        skipped_rows,
        ignored_non_numeric,
    })
}

/// Push Breslow/Efron baseline survival `S0(t) = exp(-H0(t))` and per-covariate unit-shift
/// overlays `S0(t)^exp(beta_j)` into `out.fitted_curves`.
fn append_cox_curve_overlays(out: &mut ResultSheet, cox: &CoxPhResult) {
    if cox.baseline_survival.is_empty() {
        return;
    }
    let prepend_zero = cox.baseline_survival.first().is_some_and(|p| p.time > 0.0);
    let mut base_pts = Vec::with_capacity(cox.baseline_survival.len() + usize::from(prepend_zero));
    if prepend_zero {
        base_pts.push(CurveSamplePoint { x: 0.0, y: 1.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None });
    }
    for pt in &cox.baseline_survival {
        if !pt.time.is_finite() || !pt.cum_hazard.is_finite() {
            continue;
        }
        let s0 = (-pt.cum_hazard).exp().clamp(0.0, 1.0);
        if s0.is_finite() {
            base_pts.push(CurveSamplePoint { x: pt.time, y: s0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None });
        }
    }
    if !base_pts.is_empty() {
        out.fitted_curves.push(FittedCurveOverlay {
            data_set_index: 0,
            equation_id: String::from("cox_baseline"),
            converged: cox.converged,
            ambiguous: false,
            points: base_pts,
        });
    }

    for (j, cov) in cox.covariates.iter().enumerate() {
        if !cov.beta.is_finite() {
            continue;
        }
        let hr_shift = cov.beta.clamp(-50.0, 50.0).exp();
        let mut cov_pts = Vec::with_capacity(cox.baseline_survival.len() + usize::from(prepend_zero));
        if prepend_zero {
            cov_pts.push(CurveSamplePoint { x: 0.0, y: 1.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None });
        }
        for pt in &cox.baseline_survival {
            if !pt.time.is_finite() || !pt.cum_hazard.is_finite() {
                continue;
            }
            let s_shifted = (-pt.cum_hazard * hr_shift).exp().clamp(0.0, 1.0);
            if s_shifted.is_finite() {
                cov_pts.push(CurveSamplePoint { x: pt.time, y: s_shifted, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None });
            }
        }
        if !cov_pts.is_empty() {
            out.fitted_curves.push(FittedCurveOverlay {
                data_set_index: j + 1,
                equation_id: String::from("cox_covariate"),
                converged: cox.converged,
                ambiguous: false,
                points: cov_pts,
            });
        }
    }
}

fn mv_survival_token(cell: &Cell) -> Option<String> {
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

fn is_binary_status_column(mv: &MultipleVariablesTable, col_idx: usize, event_code: Option<f64>, censored_code: Option<f64>) -> bool {
    let Some(var) = mv.variables.get(col_idx) else {
        return false;
    };
    let ev = event_code.unwrap_or(1.0);
    let cens = censored_code.unwrap_or(0.0);
    let tol = 1e-12 * (1.0 + ev.abs().max(cens.abs()));
    let mut count = 0_usize;
    for cell in &var.cells {
        if cell.excluded || cell.is_empty() {
            continue;
        }
        match cell.value {
            CellValue::Number(v) if v.is_finite() && ((v - ev).abs() <= tol || (v - cens).abs() <= tol) => {
                count += 1;
            }
            _ => return false,
        }
    }
    count > 0
}

fn title_has_any(title: &str, hints: &[&str]) -> bool {
    let lower = title.to_ascii_lowercase();
    hints.iter().any(|h| lower.contains(h))
}

struct MvKmExtraction {
    cohorts: Vec<(usize, String, Vec<SurvivalObservation>)>,
    skipped_rows: usize,
    ignored_non_numeric: Vec<(String, usize)>,
}

fn build_composite_group_levels(mv: &MultipleVariablesTable, group_cols: &[usize]) -> Vec<String> {
    let mut acc: Vec<String> = Vec::new();
    for (idx, &gc) in group_cols.iter().enumerate() {
        let lvls: Vec<String> = mv.variables[gc].ordered_levels().into_iter().filter(|l| l.included_count > 0).map(|l| l.level).collect();
        if lvls.is_empty() {
            return Vec::new();
        }
        if idx == 0 {
            acc = lvls;
        } else {
            let mut next = Vec::with_capacity(acc.len() * lvls.len());
            for a in &acc {
                for b in &lvls {
                    next.push(format!("{a} / {b}"));
                }
            }
            acc = next;
        }
    }
    acc
}

struct SingleStatusKmResult {
    cohorts: Vec<(String, Vec<SurvivalObservation>)>,
    skipped: usize,
    time_non_num: usize,
    status_non_num: usize,
}

#[allow(clippy::too_many_arguments)]
fn collect_mv_km_for_status(
    mv: &MultipleVariablesTable,
    time_col: usize,
    status_col: usize,
    group_cols: &[usize],
    levels: &[String],
    multi_status: bool,
    event_code: Option<f64>,
    censored_code: Option<f64>,
    treat_other_as: SurvivalOtherValuePolicy,
) -> Result<SingleStatusKmResult, StatsError> {
    let time_var = &mv.variables[time_col];
    let status_var = &mv.variables[status_col];
    let active_end = (0..mv.row_count()).rfind(|&r| {
        cell_has_entry(time_var.cells.get(r))
            || cell_has_entry(status_var.cells.get(r))
            || group_cols.iter().any(|&gc| cell_has_entry(mv.variables[gc].cells.get(r)))
    });
    let mut skipped = 0_usize;
    let mut time_non_num = 0_usize;
    let mut status_non_num = 0_usize;
    let mut cohorts = Vec::new();

    if !group_cols.is_empty() {
        let mut buckets: Vec<Vec<SurvivalObservation>> = vec![Vec::new(); levels.len()];
        if let Some(end_row) = active_end {
            for r in 0..=end_row {
                let tc = time_var.cells.get(r);
                let sc = status_var.cells.get(r);
                let any_g = group_cols.iter().any(|&gc| cell_has_entry(mv.variables[gc].cells.get(r)));
                if !(cell_has_entry(tc) || cell_has_entry(sc) || any_g) {
                    continue;
                }
                time_non_num += usize::from(tc.is_some_and(is_ignored_non_numeric_cell));
                status_non_num += usize::from(sc.is_some_and(is_ignored_non_numeric_cell));
                let (Some(t), Some(st)) = (tc.and_then(Cell::active_number), sc.and_then(Cell::active_number)) else {
                    skipped += 1;
                    continue;
                };
                if !t.is_finite() || t < 0.0 {
                    return Err(StatsError::InvalidParameter { name: "survival_time", value: t });
                }
                let Some(status) = parse_survival_status_with_policy(st, event_code, censored_code, treat_other_as)? else {
                    skipped += 1;
                    continue;
                };
                let mut toks = Vec::with_capacity(group_cols.len());
                for &gc in group_cols {
                    if let Some(tok) = mv.variables[gc].cells.get(r).and_then(mv_survival_token) {
                        toks.push(tok);
                    }
                }
                if toks.len() != group_cols.len() {
                    skipped += 1;
                    continue;
                }
                let key = if toks.len() == 1 { toks[0].clone() } else { toks.join(" / ") };
                if let Some(pos) = levels.iter().position(|lvl| lvl == &key) {
                    buckets[pos].push(SurvivalObservation { row: r, time: t, status });
                } else {
                    skipped += 1;
                }
            }
        }
        for (lvl, obs) in levels.iter().zip(buckets) {
            if !obs.is_empty() {
                let lbl = if multi_status { format!("{}: {lvl}", status_var.title) } else { lvl.clone() };
                cohorts.push((lbl, obs));
            }
        }
    } else {
        let mut obs = Vec::new();
        if let Some(end_row) = active_end {
            for r in 0..=end_row {
                let tc = time_var.cells.get(r);
                let sc = status_var.cells.get(r);
                if !(cell_has_entry(tc) || cell_has_entry(sc)) {
                    continue;
                }
                time_non_num += usize::from(tc.is_some_and(is_ignored_non_numeric_cell));
                status_non_num += usize::from(sc.is_some_and(is_ignored_non_numeric_cell));
                let (Some(t), Some(st)) = (tc.and_then(Cell::active_number), sc.and_then(Cell::active_number)) else {
                    skipped += 1;
                    continue;
                };
                if !t.is_finite() || t < 0.0 {
                    return Err(StatsError::InvalidParameter { name: "survival_time", value: t });
                }
                let Some(status) = parse_survival_status_with_policy(st, event_code, censored_code, treat_other_as)? else {
                    skipped += 1;
                    continue;
                };
                obs.push(SurvivalObservation { row: r, time: t, status });
            }
        }
        if !obs.is_empty() {
            let lbl = if multi_status {
                status_var.title.clone()
            } else if time_var.title.trim().is_empty() {
                String::from("Overall")
            } else {
                time_var.title.clone()
            };
            cohorts.push((lbl, obs));
        }
    }
    Ok(SingleStatusKmResult { cohorts, skipped, time_non_num, status_non_num })
}

#[allow(clippy::too_many_arguments)]
fn extract_mv_km_cohorts(
    mv: &MultipleVariablesTable,
    selected: &[usize],
    mv_time_column: Option<usize>,
    mv_status_column: Option<usize>,
    mv_group_column: Option<usize>,
    mv_status_columns: &[usize],
    mv_group_columns: &[usize],
    event_code: Option<f64>,
    censored_code: Option<f64>,
    treat_other_as: SurvivalOtherValuePolicy,
) -> Result<MvKmExtraction, StatsError> {
    for &idx in mv_time_column.iter().chain(mv_status_column.iter()).chain(mv_group_column.iter()).chain(mv_status_columns).chain(mv_group_columns) {
        if idx >= mv.variables.len() {
            return Err(StatsError::MissingColumn { index: idx });
        }
    }

    validate_selected_data_sets(selected, mv.variables.len())?;
    let pool = resolve_selected_indices(selected, mv.variables.len());

    let explicit_group_set: Vec<usize> = if !mv_group_columns.is_empty() {
        mv_group_columns.to_vec()
    } else if let Some(g) = mv_group_column {
        vec![g]
    } else {
        Vec::new()
    };

    let status_cols: Vec<usize> = if !mv_status_columns.is_empty() {
        let mut uniq = Vec::with_capacity(mv_status_columns.len());
        for &s in mv_status_columns {
            if !uniq.contains(&s) {
                uniq.push(s);
            }
        }
        uniq
    } else {
        let s = match mv_status_column {
            Some(s) => s,
            None => {
                let candidates: Vec<usize> = pool
                    .iter()
                    .copied()
                    .filter(|&i| {
                        Some(i) != mv_time_column && !explicit_group_set.contains(&i) && is_binary_status_column(mv, i, event_code, censored_code)
                    })
                    .collect();
                let preferred = candidates.iter().copied().find(|&i| title_has_any(&mv.variables[i].title, &["status", "event", "censor", "death"]));
                preferred.or_else(|| candidates.first().copied()).ok_or(StatsError::InvalidParameter { name: "mv_status_column", value: -1.0 })?
            }
        };
        vec![s]
    };

    let time_col = match mv_time_column {
        Some(t) => t,
        None => {
            let candidates: Vec<usize> =
                pool.iter()
                    .copied()
                    .filter(|&i| {
                        !status_cols.contains(&i)
                            && !explicit_group_set.contains(&i)
                            && mv.variables.get(i).is_some_and(|v| {
                                v.role != VariableRole::Categorical && v.cells.iter().filter_map(Cell::active_number).any(|x| x >= 0.0)
                            })
                    })
                    .collect();
            let preferred = candidates
                .iter()
                .copied()
                .find(|&i| title_has_any(&mv.variables[i].title, &["time", "surv", "follow", "dur", "day", "month", "year"]));
            preferred.or_else(|| candidates.first().copied()).ok_or(StatsError::InvalidParameter { name: "mv_time_column", value: -1.0 })?
        }
    };

    if status_cols.contains(&time_col) {
        return Err(StatsError::InvalidParameter { name: "mv_status_column", value: time_col as f64 });
    }

    let group_cols: Vec<usize> = if !explicit_group_set.is_empty() {
        let mut uniq = Vec::with_capacity(explicit_group_set.len());
        for &g in &explicit_group_set {
            if g == time_col || status_cols.contains(&g) {
                return Err(StatsError::InvalidParameter { name: "mv_group_column", value: g as f64 });
            }
            if !uniq.contains(&g) {
                uniq.push(g);
            }
        }
        uniq
    } else if mv_time_column.is_none() || (mv_status_column.is_none() && mv_status_columns.is_empty()) || !selected.is_empty() {
        pool.iter()
            .copied()
            .find(|&i| i != time_col && !status_cols.contains(&i) && mv.variables.get(i).is_some_and(|v| v.role == VariableRole::Categorical))
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };

    let levels = if group_cols.is_empty() { Vec::new() } else { build_composite_group_levels(mv, &group_cols) };
    if !group_cols.is_empty() && levels.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let multi_status = status_cols.len() > 1;
    let mut all_cohorts: Vec<(usize, String, Vec<SurvivalObservation>)> = Vec::new();
    let mut total_skipped = 0_usize;
    let mut time_non_num_counted = false;
    let mut ignored_non_numeric: Vec<(String, usize)> = Vec::new();

    for &status_col in &status_cols {
        let res = collect_mv_km_for_status(mv, time_col, status_col, &group_cols, &levels, multi_status, event_code, censored_code, treat_other_as)?;
        total_skipped += res.skipped;
        for (lbl, obs) in res.cohorts {
            let idx = all_cohorts.len();
            all_cohorts.push((idx, lbl, obs));
        }
        if !time_non_num_counted && res.time_non_num > 0 {
            ignored_non_numeric.push((mv.variables[time_col].title.clone(), res.time_non_num));
            time_non_num_counted = true;
        }
        if res.status_non_num > 0 {
            ignored_non_numeric.push((mv.variables[status_col].title.clone(), res.status_non_num));
        }
    }

    if all_cohorts.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    Ok(MvKmExtraction { cohorts: all_cohorts, skipped_rows: total_skipped, ignored_non_numeric })
}

struct SurvivalPairwisePlan {
    pair_indices: Vec<(usize, usize)>,
    procedure: MultiplicityProcedure,
    note: Option<String>,
}

fn resolve_survival_pairwise_plan(
    indices: &[usize],
    km_results: &[KaplanMeierCohortResult],
    mult: &curvebook_doc::MultiplicitySpec,
) -> Result<SurvivalPairwisePlan, StatsError> {
    let k = indices.len();
    let all_pairs = || -> Vec<(usize, usize)> { (0..k).flat_map(|i| ((i + 1)..k).map(move |j| (i, j))).collect() };
    let control_column = crate::multiplicity::validate_multiplicity_control_spec(mult)?;

    let requested = MultiplicityProcedure::from(mult.method);
    if let Some(ctrl) = control_column {
        let ctrl_pos =
            indices.iter().position(|&idx| idx == ctrl).ok_or(StatsError::InvalidParameter { name: "control_column", value: ctrl as f64 })?;
        let pair_indices: Vec<(usize, usize)> = (0..k).filter(|&i| i != ctrl_pos).map(|i| (i, ctrl_pos)).collect();
        let ctrl_lbl = km_results.get(ctrl_pos).map_or("control", |r| r.label.as_str());
        let (procedure, note) = match requested {
            MultiplicityProcedure::Dunnett { .. } => (
                MultiplicityProcedure::Bonferroni,
                Some(format!(
                    "Dunnett control-comparison mode compared each non-control cohort against '{ctrl_lbl}' ({} log-rank comparisons) with Bonferroni FWER adjustment; exact Dunnett multivariate-t quantiles apply only to Gaussian ANOVA contrasts.",
                    pair_indices.len()
                )),
            ),
            MultiplicityProcedure::FisherLsd => (
                MultiplicityProcedure::None,
                Some(format!(
                    "Fisher's LSD reports unadjusted pairwise log-rank P values against control cohort '{ctrl_lbl}' ({} comparisons) without familywise error control.",
                    pair_indices.len()
                )),
            ),
            MultiplicityProcedure::Bonferroni
            | MultiplicityProcedure::Sidak
            | MultiplicityProcedure::HolmBonferroni
            | MultiplicityProcedure::HolmSidak
            | MultiplicityProcedure::BenjaminiHochbergFdr
            | MultiplicityProcedure::BenjaminiYekutieliFdr
            | MultiplicityProcedure::TwoStageBkyFdr => (
                requested,
                Some(format!(
                    "Control-comparison mode compared each non-control cohort against '{ctrl_lbl}' ({} log-rank comparisons).",
                    pair_indices.len()
                )),
            ),
            MultiplicityProcedure::TamhaneT2 => (
                MultiplicityProcedure::Sidak,
                Some(format!(
                    "Tamhane's T2 is defined for parametric ANOVA contrasts rather than censored survival curves; {} log-rank comparisons against '{ctrl_lbl}' were adjusted via Sidak.",
                    pair_indices.len()
                )),
            ),
            _ => (
                MultiplicityProcedure::Bonferroni,
                Some(format!(
                    "Requested ANOVA post-hoc procedure is defined for parametric or rank contrasts rather than censored survival curves; {} log-rank comparisons against '{ctrl_lbl}' were adjusted via Bonferroni.",
                    pair_indices.len()
                )),
            ),
        };
        return Ok(SurvivalPairwisePlan { pair_indices, procedure, note });
    }
    match requested {
        MultiplicityProcedure::None => Ok(SurvivalPairwisePlan { pair_indices: all_pairs(), procedure: MultiplicityProcedure::None, note: None }),
        MultiplicityProcedure::FisherLsd => Ok(SurvivalPairwisePlan {
            pair_indices: all_pairs(),
            procedure: MultiplicityProcedure::None,
            note: Some(String::from("Fisher's LSD reports unadjusted pairwise log-rank P values without familywise error control.")),
        }),
        MultiplicityProcedure::Bonferroni
        | MultiplicityProcedure::Sidak
        | MultiplicityProcedure::HolmBonferroni
        | MultiplicityProcedure::HolmSidak
        | MultiplicityProcedure::BenjaminiHochbergFdr
        | MultiplicityProcedure::BenjaminiYekutieliFdr
        | MultiplicityProcedure::TwoStageBkyFdr => Ok(SurvivalPairwisePlan { pair_indices: all_pairs(), procedure: requested, note: None }),
        other => {
            let name = match other {
                MultiplicityProcedure::TukeyKramer => "Tukey-Kramer",
                MultiplicityProcedure::NewmanKeuls => "Student-Newman-Keuls",
                MultiplicityProcedure::GamesHowell => "Games-Howell",
                MultiplicityProcedure::TamhaneT2 => "Tamhane's T2",
                MultiplicityProcedure::DunnettT3 => "Dunnett's T3",
                MultiplicityProcedure::DunnRankSum => "Dunn's rank-sum",
                MultiplicityProcedure::LinearTrend => "Linear trend",
                _ => "Requested ANOVA post-hoc procedure",
            };
            let pairs = all_pairs();
            let note = format!(
                "{name} is defined for parametric ANOVA or rank contrasts rather than censored survival curves; pairwise log-rank P values across all {} cohort pairs were adjusted via Bonferroni.",
                pairs.len()
            );
            Ok(SurvivalPairwisePlan { pair_indices: pairs, procedure: MultiplicityProcedure::Bonferroni, note: Some(note) })
        }
    }
}

fn append_survival_pairwise(
    out: &mut ResultSheet,
    indices: &[usize],
    km_results: &[KaplanMeierCohortResult],
    cohort_obs: &[Vec<SurvivalObservation>],
    mult: &curvebook_doc::MultiplicitySpec,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
) -> Result<(), StatsError> {
    let plan = resolve_survival_pairwise_plan(indices, km_results, mult)?;
    let mut pairs = Vec::with_capacity(plan.pair_indices.len());
    let mut raw_p = Vec::with_capacity(plan.pair_indices.len());
    for (i, j) in plan.pair_indices {
        let cmp = compare_survival_curves(&[&cohort_obs[i], &cohort_obs[j]], false)?;
        let hr = two_cohort_hazard_ratio(&cohort_obs[i], &cohort_obs[j], confidence)?;
        raw_p.push(cmp.p_value);
        pairs.push((i, j, cmp.chi_square, cmp.p_value, hr));
    }
    let (adj, is_fdr) = if plan.procedure == MultiplicityProcedure::Sidak {
        (adjust_sidak(&raw_p), false)
    } else {
        adjust_p_values(&raw_p, plan.procedure, mult.threshold)
    };

    let mut pw_tab = TabBuilder::new("survival-pairwise", "Pairwise survival comparisons", ResultTabKind::MultipleComparisons).headers([
        String::from("Chi-square"),
        String::from("Raw P"),
        String::from("Adjusted P / q"),
        String::from("Hazard Ratio"),
        format!("{}% CI (HR)", confidence.percent()),
        String::from("Significant"),
    ]);
    let mut non_estimable_pairs = Vec::new();
    for ((i, j, chi2, rp, hr), ap) in pairs.into_iter().zip(adj) {
        let lbl_a = km_results[i].label.clone();
        let lbl_b = km_results[j].label.clone();
        let pair_label = format!("{lbl_a} vs. {lbl_b}");
        let sig_cell = if ap.is_finite() {
            let sig = ap <= mult.threshold;
            out.pairwise_comparisons.push(PairwiseComparisonRecord {
                group_a: indices[i],
                group_b: indices[j],
                label_a: lbl_a,
                label_b: lbl_b,
                estimate_diff: hr.log_rank_hr_opt(),
                raw_p_or_q: ap,
                is_fdr_q: is_fdr,
                significant: sig,
            });
            ResultCell::Boolean(sig)
        } else {
            non_estimable_pairs.push(pair_label.clone());
            ResultCell::Empty
        };
        pw_tab.push_row(
            pair_label,
            [
                num_cell(chi2),
                p_cell_at_alpha(rp, p_fmt, confidence.alpha()),
                p_cell_at_alpha(ap, p_fmt, mult.threshold),
                opt_num_cell(hr.log_rank_hr_opt()),
                opt_ci_cell(hr.log_rank_ci),
                sig_cell,
            ],
        );
    }
    if !non_estimable_pairs.is_empty() {
        pw_tab.push_footnote(format!(
            "Pairwise log-rank comparisons unavailable and excluded from multiplicity adjustment for {} due to zero observed events.",
            non_estimable_pairs.join(", ")
        ));
    }
    if let Some(note) = plan.note {
        pw_tab.push_footnote(note.clone());
        out.checklist.push(checklist_item("Pairwise survival multiplicity adjustment", ChecklistStatus::Info, None, note));
    }
    out.tabs.push(pw_tab.build());
    Ok(())
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{CoxTieMethod, SurvivalObservation, SurvivalStatus};
    use curvebook_math::ConfidenceLevel;

    use super::{CoxSubject, SurvivalWeight, compare_survival_curves, cox_ph_regression, glr_exact_cluster_dp, log_add_exp, weighted_log_rank_stats};

    #[test]
    fn weighted_log_rank_exact_hypergeometric_covariance() {
        let obs_a = [
            SurvivalObservation { row: 0, time: 5.0, status: SurvivalStatus::Event },
            SurvivalObservation { row: 1, time: 10.0, status: SurvivalStatus::Censored },
            SurvivalObservation { row: 2, time: 10.0, status: SurvivalStatus::Censored },
        ];
        let obs_b = [
            SurvivalObservation { row: 0, time: 5.0, status: SurvivalStatus::Event },
            SurvivalObservation { row: 1, time: 5.0, status: SurvivalStatus::Censored },
            SurvivalObservation { row: 2, time: 10.0, status: SurvivalStatus::Censored },
            SurvivalObservation { row: 3, time: 10.0, status: SurvivalStatus::Censored },
        ];

        let (u, cov, _, _) = weighted_log_rank_stats(&[&obs_a, &obs_b], SurvivalWeight::LogRank).unwrap();
        assert!((u[0] - (1.0 / 7.0)).abs() < 1e-12);
        assert!((cov[0] - (20.0 / 49.0)).abs() < 1e-12);
        assert!((cov[0] + cov[1]).abs() < 1e-12);
        assert!((cov[1] - cov[2]).abs() < 1e-12);

        let lr = compare_survival_curves(&[&obs_a, &obs_b], false).unwrap();
        assert!((lr.chi_square - 0.05).abs() < 1e-12);
        let gbw = compare_survival_curves(&[&obs_a, &obs_b], true).unwrap();
        assert!((gbw.chi_square - 0.05).abs() < 1e-12);

        let obs_c = [
            SurvivalObservation { row: 0, time: 4.0, status: SurvivalStatus::Event },
            SurvivalObservation { row: 1, time: 8.0, status: SurvivalStatus::Event },
        ];
        let (_, cov3, _, _) = weighted_log_rank_stats(&[&obs_a, &obs_b, &obs_c], SurvivalWeight::LogRank).unwrap();
        for r in 0..3 {
            let row_sum: f64 = (0..3).map(|c| cov3[r * 3 + c]).sum();
            assert!(row_sum.abs() < 1e-12, "row {r} covariance sum {row_sum} != 0");
            for c in 0..3 {
                assert!((cov3[r * 3 + c] - cov3[c * 3 + r]).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn glr_exact_dp_matches_bruteforce_enumeration_and_complement_symmetry() {
        // Check d = 3 (d <= m/2) and d = 6 (d > m/2 complement branch) on m = 8 risk set
        // with extreme linear-predictor differences spanning [-900, +900].
        let raw_lp = [900.0, 2.5, -1.2, 0.4, -50.0, -900.0, 1.1, 0.0];
        let x_rows =
            [vec![1.5, -0.4], vec![-0.7, 1.2], vec![0.3, 0.8], vec![2.1, -1.5], vec![-1.1, 0.5], vec![0.8, 2.0], vec![-0.2, -0.9], vec![0.9, 0.1]];
        for d in [2_usize, 3, 5, 6, 8] {
            let ordered: Vec<(f64, bool, Vec<f64>, usize)> = (0..8).map(|i| (1.0, i < d, x_rows[i].clone(), i)).collect();
            let (dp_ll, dp_mu, dp_c) = glr_exact_cluster_dp(&ordered, &raw_lp, 0, d, 2, true);

            let mut subset_logs = Vec::new();
            let mut subset_xs = Vec::new();
            let mut log_denom = f64::NEG_INFINITY;
            for mask in 0_u32..256 {
                if mask.count_ones() as usize != d {
                    continue;
                }
                let mut lp_s = 0.0_f64;
                let mut sx = [0.0_f64; 2];
                for i in 0..8 {
                    if (mask >> i) & 1 == 1 {
                        lp_s += raw_lp[i];
                        sx[0] += x_rows[i][0];
                        sx[1] += x_rows[i][1];
                    }
                }
                log_denom = log_add_exp(log_denom, lp_s);
                subset_logs.push(lp_s);
                subset_xs.push(sx);
            }
            let evt_lp: f64 = raw_lp[..d].iter().sum();
            let bf_ll = evt_lp - log_denom;
            let mut bf_mu = [0.0_f64; 2];
            for (&ls, sx) in subset_logs.iter().zip(&subset_xs) {
                let pr = (ls - log_denom).exp();
                bf_mu[0] += pr * sx[0];
                bf_mu[1] += pr * sx[1];
            }
            let mut bf_c = [[0.0_f64; 2]; 2];
            for (&ls, sx) in subset_logs.iter().zip(&subset_xs) {
                let pr = (ls - log_denom).exp();
                let d0 = sx[0] - bf_mu[0];
                let d1 = sx[1] - bf_mu[1];
                bf_c[0][0] += pr * d0 * d0;
                bf_c[0][1] += pr * d0 * d1;
                bf_c[1][1] += pr * d1 * d1;
            }
            assert!((dp_ll - bf_ll).abs() < 1e-11, "d={d} ll dp={dp_ll} bf={bf_ll}");
            assert!((dp_mu[0] - bf_mu[0]).abs() < 1e-11, "d={d} mu0");
            assert!((dp_mu[1] - bf_mu[1]).abs() < 1e-11, "d={d} mu1");
            assert!((dp_c[0] - bf_c[0][0]).abs() < 1e-11, "d={d} c00");
            assert!((dp_c[1] - bf_c[0][1]).abs() < 1e-11, "d={d} c01");
            assert!((dp_c[3] - bf_c[1][1]).abs() < 1e-11, "d={d} c11");
        }
    }

    #[test]
    fn cox_ph_separates_genuine_partial_and_combo_separation_from_large_span_convergence() {
        // 1. Two-covariate partial separation: x1 separates early vs late events, x2 varies freely.
        let partial_sep = vec![
            CoxSubject { time: 1.0, event: true, x: vec![1.0, 0.0] },
            CoxSubject { time: 2.0, event: true, x: vec![1.0, -10.0] },
            CoxSubject { time: 3.0, event: true, x: vec![0.0, 1.0] },
            CoxSubject { time: 4.0, event: false, x: vec![0.0, 5.0] },
            CoxSubject { time: 5.0, event: false, x: vec![0.0, 2.0] },
        ];
        let res_part = cox_ph_regression(&partial_sep, &["x1".into(), "x2".into()], CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95).unwrap();
        assert!(!res_part.converged, "partial covariate separation must report converged=false");
        assert!(res_part.covariates[0].se.is_nan());

        // 2. Linear-combination separation along v = (1, 1).
        let combo_sep = vec![
            CoxSubject { time: 1.0, event: true, x: vec![1.0, 0.0] },
            CoxSubject { time: 2.0, event: true, x: vec![0.0, 1.0] },
            CoxSubject { time: 3.0, event: true, x: vec![1.0, -1.0] },
            CoxSubject { time: 4.0, event: false, x: vec![0.0, 0.0] },
            CoxSubject { time: 5.0, event: false, x: vec![-1.0, 1.0] },
            CoxSubject { time: 6.0, event: false, x: vec![-1.0, -1.0] },
        ];
        let res_combo = cox_ph_regression(&combo_sep, &["x1".into(), "x2".into()], CoxTieMethod::Exact, ConfidenceLevel::DEFAULT_95).unwrap();
        assert!(!res_combo.converged, "linear-combination separation must report converged=false");
        assert!(res_combo.covariates[0].se.is_nan() && res_combo.covariates[1].se.is_nan());
    }
}
