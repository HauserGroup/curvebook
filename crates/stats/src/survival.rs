//! Survival analysis: Kaplan–Meier curves, Greenwood / log-log confidence bands,
//! Brookmeyer–Crowley median survival CI, Log-rank (Mantel–Cox), Gehan–Breslow–Wilcoxon,
//! log-rank trend, Hazard Ratios, and Cox Proportional Hazards regression (Efron & Breslow ties)
//! with Schoenfeld residual proportional-hazards diagnostics.
//!
//! Methods follow Kaplan & Meier (1958), Greenwood (1926), Kalbfleisch & Prentice (2002),
//! Brookmeyer & Crowley (1982), Mantel (1966), Peto & Peto (1972), Gehan (1965), Breslow (1970),
//! Tarone (1975), Berry, Kitchin & Mock (1991), Cox (1972), Breslow (1974), Efron (1977),
//! Schoenfeld (1982), Therneau, Grambsch & Fleming (1990), Grambsch & Therneau (1994),
//! Harrell et al. (1982), with textbook treatments in Klein & Moeschberger (2003) and
//! Therneau & Grambsch (2000).
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

use lumencraft_doc::{
    AnalysisSpec, Cell, CellValue, ChecklistStatus, CoxTieMethod, CurveSamplePoint, DataSheet, FittedCurveOverlay, MultipleVariablesTable,
    PValueFormat, PairwiseComparisonRecord, ResultCell, ResultSheet, ResultTabKind, SurvivalAnalysisSpec, SurvivalObservation, SurvivalStatus,
    TableArchetype, VariableColumn, VariableRole,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{chi2_sf, norm_two_sided_isf, norm_two_sided_p};
use lumencraft_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares};
use serde::{Deserialize, Serialize};

use crate::common::{
    append_ignored_text_checklist_and_footnote, append_listwise_filtered_checklist_and_footnote, build_tab, checklist_item, ci_cell,
    init_result_sheet, is_ignored_non_numeric_cell, num_cell, opt_ci_cell, opt_num_cell, p_cell, push_scalar, resolve_selected_indices,
    sanitize_result_sheet_floats,
};
use crate::error::StatsError;
use crate::multiplicity::{MultiplicityProcedure, adjust_p_values};

// ============================================================================
// Typed Outputs
// ============================================================================

/// Single time step on a Kaplan–Meier survival curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KaplanMeierStep {
    pub time: f64,
    pub n_at_risk: usize,
    pub n_events: usize,
    pub n_censored: usize,
    pub survival: f64,
    pub greenwood_se: f64,
    pub ci_lower: f64,
    pub ci_upper: f64,
}

/// Complete Kaplan–Meier summary for a single cohort.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KaplanMeierCohortResult {
    pub label: String,
    pub n_total: usize,
    pub n_events: usize,
    pub n_censored: usize,
    pub median_survival: Option<f64>,
    pub median_ci: (Option<f64>, Option<f64>),
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
    pub test_name: String,
    pub chi_square: f64,
    pub df: f64,
    pub p_value: f64,
}

/// Single covariate estimate from Cox Proportional Hazards regression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoxCovariateResult {
    pub name: String,
    pub beta: f64,
    pub se: f64,
    pub z_statistic: f64,
    pub p_value: f64,
    pub hazard_ratio: f64,
    pub hr_ci: (f64, f64),
    pub schoenfeld_rho: f64,
    pub schoenfeld_p: f64,
}

/// Single time-indexed baseline cumulative hazard and survival point from a Cox PH fit
/// (evaluated at the sample mean covariates). `cum_hazard` is the Breslow (1974) estimator
/// (Efron-adjusted under Efron ties) and `survival = exp(-cum_hazard)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoxBaselinePoint {
    pub time: f64,
    pub cum_hazard: f64,
    pub survival: f64,
}

/// Complete Cox Proportional Hazards regression output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoxPhResult {
    pub n_subjects: usize,
    pub n_events: usize,
    pub tie_method: CoxTieMethod,
    pub converged: bool,
    pub iterations: usize,
    pub log_lik_null: f64,
    pub log_lik_model: f64,
    pub concordance: f64,
    pub likelihood_ratio_chi2: f64,
    pub likelihood_ratio_p: f64,
    pub wald_chi2: f64,
    pub wald_p: f64,
    pub score_chi2: f64,
    pub score_p: f64,
    pub global_schoenfeld_chi2: f64,
    pub global_schoenfeld_df: f64,
    pub global_schoenfeld_p: f64,
    pub covariates: Vec<CoxCovariateResult>,
    pub martingale_residuals: Vec<f64>,
    pub deviance_residuals: Vec<f64>,
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
    let mut sorted = observations.to_vec();
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
        while i < sorted.len() && sorted[i].time == t {
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

/// Multi-cohort log-rank (Mantel–Cox) or Gehan–Breslow–Wilcoxon chi-square test using exact
/// hypergeometric variance covariance matrix over distinct event times.
///
/// Both tests are members of the weighted log-rank class: `U_g = sum_j w_j (d_gj - n_gj d_j / n_j)`
/// with hypergeometric covariance `V_gh = sum_j w_j^2 d_j (n_j - d_j) / (n_j (n_j - 1)) * n_gj
/// (delta_gh - n_hj / n_j)`, and `chi^2 = U' V^- U` on `K - 1` degrees of freedom computed from the
/// first `K - 1` groups (Klein & Moeschberger 2003).
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
    let weight_kind = if gehan_wilcoxon { SurvivalWeight::GehanWilcoxon } else { SurvivalWeight::LogRank };
    let (u, cov, _, _) = weighted_log_rank_stats(cohorts, weight_kind)?;

    // Solve generalized quadratic form u_{k-1}^T V_{k-1}^{-1} u_{k-1}.
    let m = k - 1;
    let mut v_red = vec![0.0_f64; m * m];
    for i in 0..m {
        for j in 0..m {
            v_red[i * m + j] = cov[i * k + j];
        }
    }
    let u_red = &u[0..m];
    let v_mat = DenseMatrix::from_row_major(m, m, v_red)?;
    let chi2 = if let Ok(chol) = cholesky(&v_mat) {
        let sol = chol.solve(u_red)?;
        u_red.iter().zip(sol.iter()).map(|(a, b)| a * b).sum::<f64>().max(0.0)
    } else {
        let (ls, _) = solve_linear_least_squares(&v_mat, u_red)?;
        u_red.iter().zip(ls.coefficients.iter()).map(|(a, b)| a * b).sum::<f64>().max(0.0)
    };
    let df = m as f64;
    let p_value = chi2_sf(chi2, df)?;
    let test_name = if gehan_wilcoxon { "Gehan-Breslow-Wilcoxon test" } else { "Log-rank (Mantel-Cox) test" };
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
    let (u, cov, _, _) = weighted_log_rank_stats(cohorts, SurvivalWeight::LogRank)?;
    let scores: Vec<f64> = (1..=k).map(|i| i as f64).collect();
    let num: f64 = scores.iter().zip(u.iter()).map(|(s, ui)| s * ui).sum();
    let mut denom = 0.0_f64;
    for i in 0..k {
        for j in 0..k {
            denom += scores[i] * cov[i * k + j] * scores[j];
        }
    }
    let chi2 = if denom > 0.0 { (num * num) / denom } else { 0.0 };
    let p_value = chi2_sf(chi2, 1.0)?;
    Ok(SurvivalComparisonResult { test_name: String::from("Log-rank test for trend"), chi_square: chi2, df: 1.0, p_value })
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
        recs.push((o.time, 0, o.status));
    }
    for o in cohort_b {
        recs.push((o.time, 1, o.status));
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
            all_recs.push((o.time, g, o.status));
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

/// Subject record for Cox Proportional Hazards regression.
#[derive(Debug, Clone)]
pub struct CoxSubject {
    pub time: f64,
    pub event: bool,
    pub x: Vec<f64>,
}

/// Fit Cox Proportional Hazards model via Newton–Raphson with backtracking step-halving,
/// supporting Breslow and Efron tie handling, omnibus LR/Wald/Score tests, Harrell's C-index,
/// and Schoenfeld residual proportional-hazards checks.
///
/// - Model and estimation: maximizes the Cox (1972) partial likelihood (covariates are centred at
///   their sample means first; `beta` is invariant to the shift). Linear predictors are clamped to
///   `[-50, 50]` before exponentiation as an overflow guard.
/// - Ties: Breslow (1974) approximation (every tied event sees the full risk set) or Efron (1977)
///   approximation (the `m`-th of `d` tied events sees the risk set minus `m / d` of the tied
///   events' weight), with the matching score and observed information (Therneau & Grambsch 2000).
/// - Inference: `Cov(beta)` is the inverse observed information at convergence; Wald `z` and HR CIs
///   `exp(beta +/- z * SE)`; omnibus likelihood-ratio, Wald, and score (at `beta = 0`) chi-square
///   tests on `p` degrees of freedom.
/// - Residuals and baseline hazard: martingale and deviance residuals (Therneau, Grambsch & Fleming
///   1990) and the Breslow-type cumulative baseline hazard (Breslow 1974), Efron-adjusted when
///   Efron ties are selected (Therneau & Grambsch 2000).
/// - Discrimination: Harrell et al. (1982) concordance index.
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

    // Center covariates for numerical stability (partial likelihood and beta are shift-invariant!).
    let mut x_means = vec![0.0_f64; p];
    for s in subjects {
        for (xm, &xj) in x_means.iter_mut().zip(&s.x) {
            *xm += xj / (n as f64);
        }
    }
    let mut ordered: Vec<(f64, bool, Vec<f64>, usize)> = subjects
        .iter()
        .enumerate()
        .map(|(orig_idx, s)| {
            let xc: Vec<f64> = (0..p).map(|j| s.x[j] - x_means[j]).collect();
            (s.time, s.event, xc, orig_idx)
        })
        .collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.1.cmp(&a.1)));

    let zero_beta = vec![0.0_f64; p];
    let (ll_null, grad_null, info_null) = cox_eval(&ordered, &zero_beta, tie_method, p)?;
    let score_chi2 = solve_quad_form(&info_null, &grad_null, p).unwrap_or(0.0);

    let mut beta = vec![0.0_f64; p];
    let mut ll_curr = ll_null;
    let mut grad = grad_null;
    let mut info = info_null.clone();
    let mut converged = false;
    let mut iterations = 0_usize;

    for iter in 1..=50 {
        iterations = iter;
        let info_mat = DenseMatrix::from_row_major(p, p, info.clone())?;
        let step = if let Ok(chol) = cholesky(&info_mat) {
            chol.solve(&grad)?
        } else {
            // Mild Levenberg damping on near-singular information matrix.
            let mut damped = info.clone();
            for j in 0..p {
                damped[j * p + j] += 1e-4;
            }
            let dm = DenseMatrix::from_row_major(p, p, damped)?;
            let (ls, _) = solve_linear_least_squares(&dm, &grad)?;
            ls.coefficients
        };
        let max_step = step.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
        let mut scale = if max_step > 2.0 { 2.0 / max_step } else { 1.0 };

        let mut accepted = false;
        for _ in 0..20 {
            let cand: Vec<f64> = (0..p).map(|j| beta[j] + scale * step[j]).collect();
            if let Ok((ll_cand, g_cand, i_cand)) = cox_eval(&ordered, &cand, tie_method, p)
                && ll_cand.is_finite()
                && ll_cand >= ll_curr - 1e-12
            {
                let delta = (ll_cand - ll_curr).abs();
                beta = cand;
                ll_curr = ll_cand;
                grad = g_cand;
                info = i_cand;
                accepted = true;
                if delta < 1e-10 && max_step * scale < 1e-8 {
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

    let (lp_min, lp_max) = ordered.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), s| {
        let lp: f64 = s.2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
        (mn.min(lp), mx.max(lp))
    });
    let lp_span = lp_max - lp_min;
    let separation_or_diverged = !converged || !lp_span.is_finite() || lp_span > 35.0 || beta.iter().any(|b| !b.is_finite());

    let info_mat = DenseMatrix::from_row_major(p, p, info.clone())?;
    let chol_opt = cholesky(&info_mat).ok();
    let well_conditioned = chol_opt.as_ref().is_some_and(|chol| {
        (0..p).all(|j| {
            let diag_null = info_null[j * p + j];
            let diag_curr = info[j * p + j];
            let piv = chol.l().get(j, j).unwrap_or(0.0);
            let piv_sq = piv * piv;
            diag_null.is_finite()
                && diag_null > 0.0
                && diag_curr.is_finite()
                && diag_curr > 1e-8 * diag_null
                && piv.is_finite()
                && piv_sq > 1e-10 * diag_curr
        })
    });

    // Never regularize (info + 1e-6*I) or substitute I when covariance is singular/separated.
    if separation_or_diverged || !well_conditioned {
        converged = false;
    }
    let cov_beta = if converged { chol_opt.and_then(|chol| chol.inverse().ok()) } else { None };
    if cov_beta.is_none() {
        converged = false;
    }

    let lr_chi2 = (2.0 * (ll_curr - ll_null)).max(0.0);
    let df = p as f64;
    let lr_p = chi2_sf(lr_chi2, df)?;
    let (wald_chi2, wald_p) = if cov_beta.is_some() {
        let w = (0..p)
            .map(|j| {
                let row_dot: f64 = (0..p).map(|k| info[j * p + k] * beta[k]).sum();
                beta[j] * row_dot
            })
            .sum::<f64>()
            .max(0.0);
        (w, chi2_sf(w, df)?)
    } else {
        (f64::NAN, f64::NAN)
    };
    let score_p = chi2_sf(score_chi2, df)?;

    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let (schoenfeld_rhos, schoenfeld_ps, global_sch_chi2) = schoenfeld_diagnostics(&ordered, &beta, cov_beta.as_ref(), tie_method, p)?;
    let global_sch_p = if global_sch_chi2.is_finite() { chi2_sf(global_sch_chi2, df)? } else { f64::NAN };

    let mut covariates = Vec::with_capacity(p);
    for j in 0..p {
        let var_j = cov_beta.as_ref().and_then(|cov| cov.get(j, j)).filter(|v| v.is_finite() && *v > 0.0);
        let (se, z_stat, p_val, hr_lo, hr_hi) = if let Some(vj) = var_j {
            let s = vj.sqrt();
            let z = beta[j] / s;
            (s, z, norm_two_sided_p(z), (beta[j] - z_crit * s).exp(), (beta[j] + z_crit * s).exp())
        } else {
            (f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN)
        };
        let hr = beta[j].exp();
        covariates.push(CoxCovariateResult {
            name: covariate_names[j].clone(),
            beta: beta[j],
            se,
            z_statistic: z_stat,
            p_value: p_val,
            hazard_ratio: hr,
            hr_ci: (hr_lo, hr_hi),
            schoenfeld_rho: schoenfeld_rhos[j],
            schoenfeld_p: schoenfeld_ps[j],
        });
    }

    let (martingale_residuals, deviance_residuals, baseline_survival) = cox_residuals_and_baseline(&ordered, &beta, tie_method, n);
    let concordance = harrell_c_index(&ordered, &beta);
    let (cov_beta_rows, correlation_matrix) = cox_cov_and_corr_matrices(cov_beta.as_ref(), p);

    Ok(CoxPhResult {
        n_subjects: n,
        n_events,
        tie_method,
        converged,
        iterations,
        log_lik_null: ll_null,
        log_lik_model: ll_curr,
        concordance,
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

fn cox_cov_and_corr_matrices(cov_beta: Option<&DenseMatrix>, p: usize) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    let Some(cov) = cov_beta else {
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
            let c = cov.get(j, k).unwrap_or(0.0);
            cov_rows[j][k] = c;
            corr_rows[j][k] = if j == k { if inv_sds[j] > 0.0 { 1.0 } else { f64::NAN } } else { (c * inv_sds[j] * inv_sds[k]).clamp(-1.0, 1.0) };
        }
    }
    (cov_rows, corr_rows)
}

/// Log partial likelihood, score vector, and observed information of the Cox (1972) model at
/// `beta`, with Breslow (1974) or Efron (1977) tie handling (Therneau & Grambsch 2000).
fn cox_eval(
    ordered: &[(f64, bool, Vec<f64>, usize)],
    beta: &[f64],
    tie_method: CoxTieMethod,
    p: usize,
) -> Result<(f64, Vec<f64>, Vec<f64>), StatsError> {
    let n = ordered.len();
    let mut raw_lp = vec![0.0_f64; n];
    let mut lp_max = f64::NEG_INFINITY;
    for i in 0..n {
        let lp: f64 = ordered[i].2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
        if !lp.is_finite() {
            return Err(StatsError::NotConverged { algorithm: "cox_eval_non_finite_lp" });
        }
        raw_lp[i] = lp;
        if lp > lp_max {
            lp_max = lp;
        }
    }
    let mut eta = vec![0.0_f64; n];
    let mut w = vec![0.0_f64; n];
    for i in 0..n {
        eta[i] = raw_lp[i] - lp_max;
        w[i] = eta[i].exp();
    }

    // Cumulative risk-set sums from largest time down to smallest.
    let mut s0_rev = vec![0.0_f64; n + 1];
    let mut s1_rev = vec![vec![0.0_f64; p]; n + 1];
    let mut s2_rev = vec![vec![0.0_f64; p * p]; n + 1];

    for i in (0..n).rev() {
        let wi = w[i];
        s0_rev[i] = s0_rev[i + 1] + wi;
        for j in 0..p {
            s1_rev[i][j] = s1_rev[i + 1][j] + wi * ordered[i].2[j];
            for k in j..p {
                let val = s2_rev[i + 1][j * p + k] + wi * ordered[i].2[j] * ordered[i].2[k];
                s2_rev[i][j * p + k] = val;
                s2_rev[i][k * p + j] = val;
            }
        }
    }

    let mut ll = 0.0_f64;
    let mut grad = vec![0.0_f64; p];
    let mut info = vec![0.0_f64; p * p];

    let mut idx = 0_usize;
    while idx < n {
        let t = ordered[idx].0;
        let risk_start = idx;
        let mut event_indices = Vec::new();
        while idx < n && ordered[idx].0 == t {
            if ordered[idx].1 {
                event_indices.push(idx);
            }
            idx += 1;
        }
        let d = event_indices.len();
        if d == 0 {
            continue;
        }
        let s0_risk = s0_rev[risk_start];
        let s1_risk = &s1_rev[risk_start];
        let s2_risk = &s2_rev[risk_start];

        // Sum over tied event subjects.
        let mut s0_evt = 0.0_f64;
        let mut s1_evt = vec![0.0_f64; p];
        let mut s2_evt = vec![0.0_f64; p * p];
        for &ei in &event_indices {
            let wei = w[ei];
            s0_evt += wei;
            ll += eta[ei];
            for j in 0..p {
                grad[j] += ordered[ei].2[j];
                s1_evt[j] += wei * ordered[ei].2[j];
                for k in j..p {
                    let v = s2_evt[j * p + k] + wei * ordered[ei].2[j] * ordered[ei].2[k];
                    s2_evt[j * p + k] = v;
                    s2_evt[k * p + j] = v;
                }
            }
        }

        let df_f = d as f64;
        for m in 0..d {
            let frac = match tie_method {
                CoxTieMethod::Breslow => 0.0,
                CoxTieMethod::Efron => (m as f64) / df_f,
            };
            let raw_denom = s0_risk - frac * s0_evt;
            if !raw_denom.is_finite() || raw_denom <= 0.0 {
                return Err(StatsError::NotConverged { algorithm: "cox_eval_risk_underflow" });
            }
            let denom = raw_denom;
            ll -= denom.ln();
            for j in 0..p {
                let s1_eff = s1_risk[j] - frac * s1_evt[j];
                let mean_j = s1_eff / denom;
                grad[j] -= mean_j;
            }
            for j in 0..p {
                let mean_j = (s1_risk[j] - frac * s1_evt[j]) / denom;
                for k in j..p {
                    let mean_k = (s1_risk[k] - frac * s1_evt[k]) / denom;
                    let s2_eff = s2_risk[j * p + k] - frac * s2_evt[j * p + k];
                    let v = s2_eff / denom - mean_j * mean_k;
                    info[j * p + k] += v;
                    if j != k {
                        info[k * p + j] += v;
                    }
                }
            }
        }
    }

    Ok((ll, grad, info))
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

/// Schoenfeld (1982) residuals `r_k = x_k - xbar(beta, t_k)` at each event (with the Efron-weighted
/// risk-set mean when Efron ties are used) and the Grambsch & Therneau (1994) proportional-hazards
/// tests based on scaled residuals `r*_k = d * Cov(beta) * r_k`.
///
/// The time transform is the event rank divided by `d + 1` (tied event times share their average
/// rank). Per covariate, `rho` is the Pearson correlation of `r*_kj` with the transformed time, and
/// the test statistic is the approximate form
/// `(sum_k (g_k - gbar) r*_kj)^2 / (d * Cov(beta)_jj * sum_k (g_k - gbar)^2)` on 1 df. The global
/// statistic is the matching approximate form using the full `Cov(beta)` on `p` df. The `+ beta`
/// offset of the scaled residuals is omitted because it does not affect either statistic.
/// Full references are listed in the module header.
fn schoenfeld_diagnostics(
    ordered: &[(f64, bool, Vec<f64>, usize)],
    beta: &[f64],
    cov_beta: Option<&DenseMatrix>,
    tie_method: CoxTieMethod,
    p: usize,
) -> Result<(Vec<f64>, Vec<f64>, f64), StatsError> {
    let Some(cov_beta) = cov_beta else {
        return Ok((vec![f64::NAN; p], vec![f64::NAN; p], f64::NAN));
    };
    let n = ordered.len();
    let w: Vec<f64> = ordered.iter().map(|s| s.2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum::<f64>().clamp(-50.0, 50.0).exp()).collect();
    let mut s0_rev = vec![0.0_f64; n + 1];
    let mut s1_rev = vec![vec![0.0_f64; p]; n + 1];
    for i in (0..n).rev() {
        s0_rev[i] = s0_rev[i + 1] + w[i];
        let next_s1 = s1_rev[i + 1].clone();
        for ((dst, &nxt), &xv) in s1_rev[i].iter_mut().zip(&next_s1).zip(&ordered[i].2) {
            *dst = nxt + w[i] * xv;
        }
    }

    let mut resid_rows: Vec<(f64, Vec<f64>)> = Vec::new();
    let mut idx = 0_usize;
    while idx < n {
        let t = ordered[idx].0;
        let r_start = idx;
        let mut evts = Vec::new();
        while idx < n && ordered[idx].0 == t {
            if ordered[idx].1 {
                evts.push(idx);
            }
            idx += 1;
        }
        if evts.is_empty() {
            continue;
        }
        let d = evts.len() as f64;
        let mut s0_e = 0.0_f64;
        let mut s1_e = vec![0.0_f64; p];
        for &ei in &evts {
            s0_e += w[ei];
            for (dst, &xv) in s1_e.iter_mut().zip(&ordered[ei].2) {
                *dst += w[ei] * xv;
            }
        }
        let mut e_bar = vec![0.0_f64; p];
        for m in 0..evts.len() {
            let frac = if tie_method == CoxTieMethod::Efron { (m as f64) / d } else { 0.0 };
            let denom = (s0_rev[r_start] - frac * s0_e).max(1e-18);
            for j in 0..p {
                e_bar[j] += (s1_rev[r_start][j] - frac * s1_e[j]) / (denom * d);
            }
        }
        for &ei in &evts {
            let r_vec: Vec<f64> = (0..p).map(|j| ordered[ei].2[j] - e_bar[j]).collect();
            resid_rows.push((t, r_vec));
        }
    }

    let n_evt = resid_rows.len();
    if n_evt < 3 {
        return Ok((vec![0.0; p], vec![1.0; p], 0.0));
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
/// the cumulative baseline hazard at the mean covariates.
///
/// The baseline hazard increment at each event time is the Breslow (1974) estimator
/// `d_j / sum_{risk set} exp(x' beta)`, or with Efron ties the Efron-adjusted increment
/// `sum_m 1 / (S0 - (m / d) S0_events)`; a subject dying at a tied time is charged the
/// `(1 - m / d)`-weighted share of that increment (Therneau & Grambsch 2000).
fn cox_residuals_and_baseline(
    ordered: &[(f64, bool, Vec<f64>, usize)],
    beta: &[f64],
    tie_method: CoxTieMethod,
    n: usize,
) -> (Vec<f64>, Vec<f64>, Vec<CoxBaselinePoint>) {
    let w: Vec<f64> = ordered.iter().map(|s| s.2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum::<f64>().clamp(-50.0, 50.0).exp()).collect();
    let mut s0_rev = vec![0.0_f64; n + 1];
    for i in (0..n).rev() {
        s0_rev[i] = s0_rev[i + 1] + w[i];
    }

    let mut cum_h0 = 0.0_f64;
    let mut mart = vec![0.0_f64; n];
    let mut dev = vec![0.0_f64; n];
    let mut baseline = Vec::new();

    let mut idx = 0_usize;
    while idx < n {
        let t = ordered[idx].0;
        let r_start = idx;
        let mut evts = Vec::new();
        let mut grp = Vec::new();
        while idx < n && ordered[idx].0 == t {
            grp.push(idx);
            if ordered[idx].1 {
                evts.push(idx);
            }
            idx += 1;
        }
        let mut cum_h0_evt = cum_h0;
        if !evts.is_empty() {
            let cum_h0_before = cum_h0;
            let s0_e: f64 = evts.iter().map(|&ei| w[ei]).sum();
            let d = evts.len() as f64;
            let mut dh_evt = 0.0_f64;
            for m in 0..evts.len() {
                let frac = if tie_method == CoxTieMethod::Efron { (m as f64) / d } else { 0.0 };
                let step = 1.0 / (s0_rev[r_start] - frac * s0_e).max(1e-18);
                cum_h0 += step;
                dh_evt += (1.0 - frac) * step;
            }
            cum_h0_evt = cum_h0_before + dh_evt;
            baseline.push(CoxBaselinePoint { time: t, cum_hazard: cum_h0, survival: (-cum_h0).exp().clamp(0.0, 1.0) });
        }
        for &i in &grp {
            let delta = if ordered[i].1 { 1.0 } else { 0.0 };
            let h_subj = if ordered[i].1 { cum_h0_evt } else { cum_h0 };
            let m = delta - w[i] * h_subj;
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
fn harrell_c_index(ordered: &[(f64, bool, Vec<f64>, usize)], beta: &[f64]) -> f64 {
    let lp: Vec<f64> = ordered.iter().map(|s| s.2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum()).collect();
    let mut conc = 0.0_f64;
    let mut pairs = 0.0_f64;
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
                pairs += 1.0;
                if lp[i] > lp[j] {
                    conc += 1.0;
                } else if (lp[i] - lp[j]).abs() <= 1e-12 {
                    conc += 0.5;
                }
            }
        }
    }
    if pairs > 0.0 { conc / pairs } else { 0.5 }
}

// ============================================================================
// ResultSheet Builder
// ============================================================================

/// Execute a [`SurvivalAnalysisSpec`] against a `Survival` or `MultipleVariables` [`DataSheet`].
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
        } => {
            let (cohorts, skipped_mv_rows, ignored_non_numeric) = match &sheet.archetype {
                TableArchetype::Survival(table) => {
                    let indices = resolve_selected_indices(&spec.selected_data_sets, table.cohorts.len());
                    if indices.is_empty() {
                        return Err(StatsError::EmptyInput);
                    }
                    let mut list = Vec::with_capacity(indices.len());
                    for c_idx in indices {
                        let obs = table.cohort_observations(c_idx)?;
                        list.push((c_idx, table.cohorts[c_idx].title.clone(), obs));
                    }
                    (list, 0_usize, Vec::new())
                }
                TableArchetype::MultipleVariables(mv) => {
                    let mv_ext = extract_mv_km_cohorts(mv, &spec.selected_data_sets, *mv_time_column, *mv_status_column, *mv_group_column)?;
                    (mv_ext.cohorts, mv_ext.skipped_rows, mv_ext.ignored_non_numeric)
                }
                other => {
                    return Err(StatsError::IncompatibleTable { analysis: "Kaplan-Meier survival", actual: other.kind() });
                }
            };

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
            let m_headers = vec![
                String::from("N total"),
                String::from("Events"),
                String::from("Censored"),
                String::from("Median survival"),
                format!("{}% CI (median)", confidence.percent()),
            ];
            let mut m_labels = Vec::with_capacity(km_results.len());
            let mut m_rows = Vec::with_capacity(km_results.len());
            for km in &km_results {
                m_labels.push(km.label.clone());
                let med_ci = match km.median_ci {
                    (Some(lo), Some(hi)) => ci_cell(lo, hi),
                    (Some(lo), None) => {
                        ResultCell::ConfidenceInterval { lower: lumencraft_doc::CiBound::Finite(lo), upper: lumencraft_doc::CiBound::Open }
                    }
                    _ => ResultCell::Empty,
                };
                m_rows.push(vec![
                    num_cell(km.n_total as f64),
                    num_cell(km.n_events as f64),
                    num_cell(km.n_censored as f64),
                    opt_num_cell(km.median_survival),
                    med_ci,
                ]);
            }
            out.tabs.push(build_tab("km-summary", "Kaplan-Meier cohort summary", ResultTabKind::MainSummary, m_headers, m_labels, m_rows));

            // Number at risk tab.
            let nar_headers = vec![
                String::from("Cohort"),
                String::from("Time"),
                String::from("At risk"),
                String::from("Events"),
                String::from("Survival"),
                format!("{}% CI", confidence.percent()),
            ];
            let mut nar_labels = Vec::new();
            let mut nar_rows = Vec::new();
            for km in &km_results {
                for s in &km.steps {
                    nar_labels.push(format!("{} @ {:.3}", km.label, s.time));
                    nar_rows.push(vec![
                        ResultCell::Text(km.label.clone()),
                        num_cell(s.time),
                        num_cell(s.n_at_risk as f64),
                        num_cell(s.n_events as f64),
                        num_cell(s.survival),
                        ci_cell(s.ci_lower, s.ci_upper),
                    ]);
                }
            }
            out.tabs.push(build_tab(
                "number-at-risk",
                "Survival schedule & number at risk",
                ResultTabKind::NumberAtRisk,
                nar_headers,
                nar_labels,
                nar_rows,
            ));

            // Curve comparisons if >= 2 cohorts.
            if cohort_obs.len() >= 2 {
                let refs: Vec<&[SurvivalObservation]> = cohort_obs.iter().map(Vec::as_slice).collect();
                let mut cmp_rows = Vec::new();
                let mut cmp_labels = Vec::new();
                if *log_rank_mantel_cox {
                    let lr = compare_survival_curves(&refs, false)?;
                    push_scalar(&mut out, "Log-rank chi-square", None, lr.chi_square);
                    push_scalar(&mut out, "Log-rank P value", None, lr.p_value);
                    cmp_labels.push(lr.test_name);
                    cmp_rows.push(vec![num_cell(lr.chi_square), num_cell(lr.df), p_cell(lr.p_value, p_fmt)]);
                }
                if *gehan_breslow_wilcoxon {
                    let gbw = compare_survival_curves(&refs, true)?;
                    push_scalar(&mut out, "Gehan-Breslow-Wilcoxon chi-square", None, gbw.chi_square);
                    push_scalar(&mut out, "Gehan-Breslow-Wilcoxon P", None, gbw.p_value);
                    cmp_labels.push(gbw.test_name);
                    cmp_rows.push(vec![num_cell(gbw.chi_square), num_cell(gbw.df), p_cell(gbw.p_value, p_fmt)]);
                }
                if *trend_test && refs.len() >= 3 {
                    let tr = log_rank_trend_test(&refs)?;
                    cmp_labels.push(tr.test_name);
                    cmp_rows.push(vec![num_cell(tr.chi_square), num_cell(tr.df), p_cell(tr.p_value, p_fmt)]);
                }
                if !cmp_rows.is_empty() {
                    out.tabs.push(build_tab(
                        "curve-comparison",
                        "Comparison of survival curves",
                        ResultTabKind::ModelComparison,
                        vec![String::from("Chi-square"), String::from("df"), String::from("P value")],
                        cmp_labels,
                        cmp_rows,
                    ));
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
            append_listwise_filtered_checklist_and_footnote(&mut out, "Kaplan-Meier survival", skipped_mv_rows, total_kept);
            append_ignored_text_checklist_and_footnote(&mut out, "Kaplan-Meier survival", &ignored_non_numeric);
        }
        SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, reference_levels, tie_method } => {
            let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
                return Err(StatsError::IncompatibleTable { analysis: "Cox proportional hazards", actual: sheet.archetype.kind() });
            };
            let extraction = build_cox_design_subjects(mv, *time_column, *status_column, predictor_columns, reference_levels)?;
            let pred_names = extraction.pred_names;
            let subjects = extraction.subjects;

            let cox = cox_ph_regression(&subjects, &pred_names, *tie_method, confidence)?;
            let aic = -2.0 * cox.log_lik_model + 2.0 * (cox.covariates.len() as f64);
            let has_non_estimable = cox.covariates.iter().any(|c| !c.se.is_finite());
            push_scalar(&mut out, "Converged", None, if cox.converged { 1.0 } else { 0.0 });
            push_scalar(&mut out, "Iterations", None, cox.iterations as f64);
            push_scalar(&mut out, "Concordance (C-index)", None, cox.concordance);
            push_scalar(&mut out, "Likelihood ratio chi2", None, cox.likelihood_ratio_chi2);
            push_scalar(&mut out, "Likelihood ratio P", None, cox.likelihood_ratio_p);
            push_scalar(&mut out, "Global Schoenfeld P", None, cox.global_schoenfeld_p);
            push_scalar(&mut out, "AIC", None, aic);

            let headers = vec![
                String::from("beta"),
                String::from("SE"),
                String::from("Wald z"),
                String::from("P value"),
                String::from("Hazard Ratio"),
                format!("{}% CI (HR)", confidence.percent()),
                String::from("Schoenfeld P"),
            ];
            let mut labels = Vec::with_capacity(cox.covariates.len());
            let mut rows = Vec::with_capacity(cox.covariates.len());
            for cov in &cox.covariates {
                labels.push(cov.name.clone());
                rows.push(vec![
                    num_cell(cov.beta),
                    num_cell(cov.se),
                    num_cell(cov.z_statistic),
                    p_cell(cov.p_value, p_fmt),
                    num_cell(cov.hazard_ratio),
                    ci_cell(cov.hr_ci.0, cov.hr_ci.1),
                    p_cell(cov.schoenfeld_p, p_fmt),
                ]);
            }
            let mut cox_tab = build_tab(
                "cox-ph",
                "Cox proportional hazards estimates",
                ResultTabKind::ParametersAndConfidenceIntervals,
                headers,
                labels.clone(),
                rows,
            );
            if !cox.converged || has_non_estimable {
                cox_tab.footnotes.push(String::from(
                    "Model did not converge or exhibited separation/collinearity; non-estimable SE, Wald Z/P, and HR CI bounds are omitted. Refer to Likelihood ratio P for the omnibus test.",
                ));
            }
            out.tabs.push(cox_tab);

            append_cox_baseline_tabs(&mut out, &cox.baseline_survival);

            let corr_rows: Vec<Vec<ResultCell>> = cox.correlation_matrix.iter().map(|r| r.iter().map(|&v| num_cell(v)).collect()).collect();
            out.tabs.push(build_tab(
                "cox-covariance",
                "Cox parameter correlation matrix",
                ResultTabKind::CovarianceMatrix,
                labels.clone(),
                labels,
                corr_rows,
            ));

            append_cox_curve_overlays(&mut out, &cox);

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
                    "Global Schoenfeld proportional-hazards test cannot be computed when the covariate covariance matrix is non-estimable.",
                )
            };
            out.checklist.push(checklist_item("Proportional hazards assumption (Schoenfeld)", ph_status, ph_metric, ph_expl));
            append_listwise_filtered_checklist_and_footnote(&mut out, "Cox proportional hazards", extraction.skipped_rows, subjects.len());
            append_ignored_text_checklist_and_footnote(&mut out, "Cox proportional hazards", &extraction.ignored_non_numeric);
        }
    }

    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

fn append_cox_baseline_tabs(out: &mut ResultSheet, baseline: &[CoxBaselinePoint]) {
    let mut base_labels = Vec::with_capacity(baseline.len() + 1);
    let mut base_rows = Vec::with_capacity(baseline.len() + 1);
    if baseline.first().is_some_and(|p| p.time > 0.0) {
        base_labels.push(String::from("t = 0.0000"));
        base_rows.push(vec![num_cell(0.0), num_cell(0.0), num_cell(1.0)]);
    }
    for pt in baseline {
        base_labels.push(format!("t = {:.4}", pt.time));
        base_rows.push(vec![num_cell(pt.time), num_cell(pt.cum_hazard), num_cell(pt.survival)]);
    }
    let mut base_tab = build_tab(
        "cox-baseline-survival",
        "Cox baseline survival curve (at mean covariates)",
        ResultTabKind::BaselineSurvival,
        vec![String::from("Time"), String::from("Cumulative Hazard H0(t)"), String::from("Baseline Survival S0(t)")],
        base_labels,
        base_rows,
    );
    base_tab.footnotes.push(String::from("Evaluated at sample covariate means using the fitted Breslow/Efron cumulative hazard step function."));
    out.tabs.push(base_tab);

    let mut lml_labels = Vec::with_capacity(baseline.len());
    let mut lml_rows = Vec::with_capacity(baseline.len());
    for pt in baseline {
        if pt.cum_hazard > 0.0 && pt.survival > 0.0 && pt.survival < 1.0 {
            let log_t = (pt.time > 0.0).then(|| pt.time.ln());
            let lml = pt.cum_hazard.ln();
            lml_labels.push(format!("t = {:.4}", pt.time));
            lml_rows.push(vec![num_cell(pt.time), opt_num_cell(log_t), num_cell(pt.survival), num_cell(lml)]);
        }
    }
    out.tabs.push(build_tab(
        "cox-log-minus-log",
        "Cox log-minus-log survival coordinates",
        ResultTabKind::LogMinusLog,
        vec![String::from("Time"), String::from("ln(Time)"), String::from("Baseline Survival S0(t)"), String::from("ln(-ln S0(t))")],
        lml_labels,
        lml_rows,
    ));
}

/// Build Cox PH predictor names and complete-case subjects, expanding categorical predictors
/// into `"{title}[{level} vs {ref_level}]"` reference-coded dummy indicators.
struct CoxDesignExtraction {
    pred_names: Vec<String>,
    subjects: Vec<CoxSubject>,
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
    time: f64,
    event: bool,
    preds: Vec<RetainedCoxPred>,
}

fn build_cox_design_subjects(
    mv: &MultipleVariablesTable,
    time_column: usize,
    status_column: usize,
    predictor_columns: &[usize],
    reference_levels: &[(usize, String)],
) -> Result<CoxDesignExtraction, StatsError> {
    let time_var = mv.variables.get(time_column).ok_or(StatsError::MissingColumn { index: time_column })?;
    let status_var = mv.variables.get(status_column).ok_or(StatsError::MissingColumn { index: status_column })?;
    let mut is_cat = Vec::with_capacity(predictor_columns.len());
    for &pc in predictor_columns {
        let pv = mv.variables.get(pc).ok_or(StatsError::MissingColumn { index: pc })?;
        is_cat.push(pv.role == VariableRole::Categorical);
    }

    let active_end = (0..mv.row_count()).rfind(|&r| {
        cell_has_entry(time_var.cells.get(r))
            || cell_has_entry(status_var.cells.get(r))
            || predictor_columns.iter().any(|&pc| cell_has_entry(mv.variables[pc].cells.get(r)))
    });
    let mut retained_rows = Vec::new();
    let mut skipped_rows = 0_usize;
    let mut time_non_num = 0_usize;
    let mut status_non_num = 0_usize;
    let mut pred_non_num = vec![0_usize; predictor_columns.len()];

    if let Some(end_row) = active_end {
        for r in 0..=end_row {
            let tc = time_var.cells.get(r);
            let sc = status_var.cells.get(r);
            let has_entry =
                cell_has_entry(tc) || cell_has_entry(sc) || predictor_columns.iter().any(|&pc| cell_has_entry(mv.variables[pc].cells.get(r)));
            if !has_entry {
                continue;
            }
            if tc.is_some_and(is_ignored_non_numeric_cell) {
                time_non_num += 1;
            }
            if sc.is_some_and(is_ignored_non_numeric_cell) {
                status_non_num += 1;
            }
            for (pos, &pc) in predictor_columns.iter().enumerate() {
                if !is_cat[pos] && mv.variables[pc].cells.get(r).is_some_and(is_ignored_non_numeric_cell) {
                    pred_non_num[pos] += 1;
                }
            }
            let (Some(t), Some(st)) = (tc.and_then(Cell::active_number), sc.and_then(Cell::active_number)) else {
                skipped_rows += 1;
                continue;
            };
            let status = parse_survival_status(st)?;
            let mut row_preds = Vec::with_capacity(predictor_columns.len());
            let mut ok = true;
            for (pos, &pc) in predictor_columns.iter().enumerate() {
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
                    row_preds.push(RetainedCoxPred::Categorical(tok));
                } else if let Some(xv) = cell.active_number() {
                    row_preds.push(RetainedCoxPred::Numeric(xv));
                } else {
                    ok = false;
                    break;
                }
            }
            if ok {
                retained_rows.push(RetainedCoxRow { time: t, event: matches!(status, SurvivalStatus::Event), preds: row_preds });
            } else {
                skipped_rows += 1;
            }
        }
    }

    // Derive observed categorical levels and reference levels strictly from complete-case retained rows
    // so incomplete/excluded rows never introduce all-zero phantom dummy columns.
    let mut cat_levels: Vec<Option<(String, Vec<String>)>> = Vec::with_capacity(predictor_columns.len());
    for (pos, &pc) in predictor_columns.iter().enumerate() {
        let pv = &mv.variables[pc];
        if is_cat[pos] {
            let cells: Vec<Cell> = retained_rows
                .iter()
                .filter_map(|row| match &row.preds[pos] {
                    RetainedCoxPred::Categorical(tok) => Some(Cell::text(tok.as_str())),
                    RetainedCoxPred::Numeric(_) => None,
                })
                .collect();
            let retained_col = VariableColumn {
                title: pv.title.clone(),
                role: pv.role,
                role_inferred: pv.role_inferred,
                source: pv.source.clone(),
                level_order: pv.level_order.clone(),
                cells,
            };
            let mut lvls: Vec<String> =
                retained_col.ordered_levels_with(&pv.level_order).into_iter().filter(|s| s.included_count > 0).map(|s| s.level).collect();
            let ref_lvl = reference_levels
                .iter()
                .find(|(idx, _)| *idx == pc)
                .map(|(_, l)| l.clone())
                .filter(|l| lvls.iter().any(|existing| existing == l))
                .or_else(|| lvls.first().cloned());
            if let Some(rl) = ref_lvl {
                lvls.retain(|l| l != &rl);
                cat_levels.push(Some((rl, lvls)));
            } else {
                cat_levels.push(None);
            }
        } else {
            cat_levels.push(None);
        }
    }

    let mut pred_names = Vec::new();
    for (pos, &pc) in predictor_columns.iter().enumerate() {
        let title = &mv.variables[pc].title;
        if let Some((ref_lvl, non_refs)) = &cat_levels[pos] {
            for lv in non_refs {
                pred_names.push(format!("{title}[{lv} vs {ref_lvl}]"));
            }
        } else {
            pred_names.push(title.clone());
        }
    }

    if !retained_rows.is_empty() && pred_names.is_empty() {
        return Err(StatsError::ZeroVariance { context: "Cox PH predictor has fewer than 2 levels after complete-case filtering" });
    }

    let mut subjects = Vec::with_capacity(retained_rows.len());
    for row in retained_rows {
        let mut x_vals = Vec::with_capacity(pred_names.len());
        for (pos, pred) in row.preds.into_iter().enumerate() {
            match (pred, &cat_levels[pos]) {
                (RetainedCoxPred::Categorical(tok), Some((_, non_refs))) => {
                    for lv in non_refs {
                        x_vals.push(if &tok == lv { 1.0 } else { 0.0 });
                    }
                }
                (RetainedCoxPred::Numeric(xv), None) => x_vals.push(xv),
                _ => {}
            }
        }
        subjects.push(CoxSubject { time: row.time, event: row.event, x: x_vals });
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
            ignored_non_numeric.push((mv.variables[predictor_columns[pos]].title.clone(), count));
        }
    }

    Ok(CoxDesignExtraction { pred_names, subjects, skipped_rows, ignored_non_numeric })
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

fn is_binary_status_column(mv: &MultipleVariablesTable, col_idx: usize) -> bool {
    let Some(var) = mv.variables.get(col_idx) else {
        return false;
    };
    let mut count = 0_usize;
    for cell in &var.cells {
        if cell.excluded || cell.is_empty() {
            continue;
        }
        match cell.value {
            CellValue::Number(v) if v.is_finite() && (v == 0.0 || v == 1.0) => {
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

fn parse_survival_status(st: f64) -> Result<SurvivalStatus, StatsError> {
    if st == 1.0 {
        Ok(SurvivalStatus::Event)
    } else if st == 0.0 {
        Ok(SurvivalStatus::Censored)
    } else {
        Err(StatsError::InvalidParameter { name: "survival_status", value: st })
    }
}

struct MvKmExtraction {
    cohorts: Vec<(usize, String, Vec<SurvivalObservation>)>,
    skipped_rows: usize,
    ignored_non_numeric: Vec<(String, usize)>,
}

fn extract_mv_km_cohorts(
    mv: &MultipleVariablesTable,
    selected: &[usize],
    mv_time_column: Option<usize>,
    mv_status_column: Option<usize>,
    mv_group_column: Option<usize>,
) -> Result<MvKmExtraction, StatsError> {
    if let Some(t) = mv_time_column
        && t >= mv.variables.len()
    {
        return Err(StatsError::MissingColumn { index: t });
    }
    if let Some(s) = mv_status_column
        && s >= mv.variables.len()
    {
        return Err(StatsError::MissingColumn { index: s });
    }
    if let Some(g) = mv_group_column
        && g >= mv.variables.len()
    {
        return Err(StatsError::MissingColumn { index: g });
    }

    let pool = resolve_selected_indices(selected, mv.variables.len());
    let status_col = match mv_status_column {
        Some(s) => s,
        None => {
            let candidates: Vec<usize> =
                pool.iter().copied().filter(|&i| Some(i) != mv_time_column && Some(i) != mv_group_column && is_binary_status_column(mv, i)).collect();
            let preferred = candidates.iter().copied().find(|&i| title_has_any(&mv.variables[i].title, &["status", "event", "censor", "death"]));
            preferred.or_else(|| candidates.first().copied()).ok_or(StatsError::InvalidParameter { name: "mv_status_column", value: -1.0 })?
        }
    };

    let time_col = match mv_time_column {
        Some(t) => t,
        None => {
            let candidates: Vec<usize> =
                pool.iter()
                    .copied()
                    .filter(|&i| {
                        i != status_col
                            && Some(i) != mv_group_column
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

    if time_col == status_col {
        return Err(StatsError::InvalidParameter { name: "mv_status_column", value: status_col as f64 });
    }

    let group_col = match mv_group_column {
        Some(g) => {
            if g == time_col || g == status_col {
                return Err(StatsError::InvalidParameter { name: "mv_group_column", value: g as f64 });
            }
            Some(g)
        }
        None => {
            if mv_time_column.is_none() || mv_status_column.is_none() || !selected.is_empty() {
                let cat_candidates: Vec<usize> = pool
                    .iter()
                    .copied()
                    .filter(|&i| i != time_col && i != status_col && mv.variables.get(i).is_some_and(|v| v.role == VariableRole::Categorical))
                    .collect();
                cat_candidates.first().copied()
            } else {
                None
            }
        }
    };

    let time_var = &mv.variables[time_col];
    let status_var = &mv.variables[status_col];
    let active_end = (0..mv.row_count()).rfind(|&r| {
        cell_has_entry(time_var.cells.get(r))
            || cell_has_entry(status_var.cells.get(r))
            || group_col.is_some_and(|gc| cell_has_entry(mv.variables[gc].cells.get(r)))
    });
    let mut skipped_rows = 0_usize;
    let mut time_non_num = 0_usize;
    let mut status_non_num = 0_usize;

    let cohorts = if let Some(g_col) = group_col {
        let g_var = &mv.variables[g_col];
        let levels: Vec<String> = g_var.ordered_levels().into_iter().filter(|l| l.included_count > 0).map(|l| l.level).collect();
        if levels.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        let mut buckets: Vec<Vec<SurvivalObservation>> = vec![Vec::new(); levels.len()];
        if let Some(end_row) = active_end {
            for r in 0..=end_row {
                let tc = time_var.cells.get(r);
                let sc = status_var.cells.get(r);
                let gc = g_var.cells.get(r);
                if !(cell_has_entry(tc) || cell_has_entry(sc) || cell_has_entry(gc)) {
                    continue;
                }
                if tc.is_some_and(is_ignored_non_numeric_cell) {
                    time_non_num += 1;
                }
                if sc.is_some_and(is_ignored_non_numeric_cell) {
                    status_non_num += 1;
                }
                let (Some(t), Some(st)) = (tc.and_then(Cell::active_number), sc.and_then(Cell::active_number)) else {
                    skipped_rows += 1;
                    continue;
                };
                if !t.is_finite() || t < 0.0 {
                    return Err(StatsError::InvalidParameter { name: "survival_time", value: t });
                }
                let status = parse_survival_status(st)?;
                let Some(tok) = gc.and_then(mv_survival_token) else {
                    skipped_rows += 1;
                    continue;
                };
                if let Some(pos) = levels.iter().position(|lvl| lvl == &tok) {
                    buckets[pos].push(SurvivalObservation { row: r, time: t, status });
                } else {
                    skipped_rows += 1;
                }
            }
        }
        let cohorts: Vec<(usize, String, Vec<SurvivalObservation>)> =
            levels.into_iter().zip(buckets).enumerate().filter(|(_, (_, obs))| !obs.is_empty()).map(|(idx, (lbl, obs))| (idx, lbl, obs)).collect();
        if cohorts.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        cohorts
    } else {
        let mut obs = Vec::new();
        if let Some(end_row) = active_end {
            for r in 0..=end_row {
                let tc = time_var.cells.get(r);
                let sc = status_var.cells.get(r);
                if !(cell_has_entry(tc) || cell_has_entry(sc)) {
                    continue;
                }
                if tc.is_some_and(is_ignored_non_numeric_cell) {
                    time_non_num += 1;
                }
                if sc.is_some_and(is_ignored_non_numeric_cell) {
                    status_non_num += 1;
                }
                let (Some(t), Some(st)) = (tc.and_then(Cell::active_number), sc.and_then(Cell::active_number)) else {
                    skipped_rows += 1;
                    continue;
                };
                if !t.is_finite() || t < 0.0 {
                    return Err(StatsError::InvalidParameter { name: "survival_time", value: t });
                }
                let status = parse_survival_status(st)?;
                obs.push(SurvivalObservation { row: r, time: t, status });
            }
        }
        if obs.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        let label = if time_var.title.trim().is_empty() { String::from("Overall") } else { time_var.title.clone() };
        vec![(0, label, obs)]
    };

    let mut ignored_non_numeric = Vec::new();
    if time_non_num > 0 {
        ignored_non_numeric.push((time_var.title.clone(), time_non_num));
    }
    if status_non_num > 0 {
        ignored_non_numeric.push((status_var.title.clone(), status_non_num));
    }
    Ok(MvKmExtraction { cohorts, skipped_rows, ignored_non_numeric })
}

fn append_survival_pairwise(
    out: &mut ResultSheet,
    indices: &[usize],
    km_results: &[KaplanMeierCohortResult],
    cohort_obs: &[Vec<SurvivalObservation>],
    mult: &lumencraft_doc::MultiplicitySpec,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
) -> Result<(), StatsError> {
    let k = cohort_obs.len();
    let mut pairs = Vec::new();
    let mut raw_p = Vec::new();
    for i in 0..k {
        for j in (i + 1)..k {
            let cmp = compare_survival_curves(&[&cohort_obs[i], &cohort_obs[j]], false)?;
            let hr = two_cohort_hazard_ratio(&cohort_obs[i], &cohort_obs[j], confidence)?;
            raw_p.push(cmp.p_value);
            pairs.push((i, j, cmp.chi_square, cmp.p_value, hr));
        }
    }
    let proc_eff = match MultiplicityProcedure::from(mult.method) {
        MultiplicityProcedure::None | MultiplicityProcedure::FisherLsd => MultiplicityProcedure::None,
        MultiplicityProcedure::BenjaminiHochbergFdr | MultiplicityProcedure::BenjaminiYekutieliFdr | MultiplicityProcedure::TwoStageBkyFdr => {
            MultiplicityProcedure::from(mult.method)
        }
        MultiplicityProcedure::HolmBonferroni => MultiplicityProcedure::HolmBonferroni,
        MultiplicityProcedure::Sidak | MultiplicityProcedure::HolmSidak => MultiplicityProcedure::HolmSidak,
        _ => MultiplicityProcedure::Bonferroni,
    };
    let (adj, is_fdr) = adjust_p_values(&raw_p, proc_eff, mult.threshold);

    let headers = vec![
        String::from("Chi-square"),
        String::from("Raw P"),
        String::from("Adjusted P / q"),
        String::from("Hazard Ratio"),
        format!("{}% CI (HR)", confidence.percent()),
        String::from("Significant"),
    ];
    let mut row_labels = Vec::with_capacity(pairs.len());
    let mut rows = Vec::with_capacity(pairs.len());
    for ((i, j, chi2, rp, hr), ap) in pairs.into_iter().zip(adj) {
        let lbl_a = km_results[i].label.clone();
        let lbl_b = km_results[j].label.clone();
        row_labels.push(format!("{lbl_a} vs. {lbl_b}"));
        let sig = ap <= mult.threshold;
        rows.push(vec![
            num_cell(chi2),
            p_cell(rp, p_fmt),
            p_cell(ap, p_fmt),
            opt_num_cell(hr.log_rank_hr_opt()),
            opt_ci_cell(hr.log_rank_ci),
            ResultCell::Boolean(sig),
        ]);
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
    }
    out.tabs.push(build_tab("survival-pairwise", "Pairwise survival comparisons", ResultTabKind::MultipleComparisons, headers, row_labels, rows));
    Ok(())
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::{SurvivalObservation, SurvivalStatus};

    use super::{SurvivalWeight, compare_survival_curves, weighted_log_rank_stats};

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
}
