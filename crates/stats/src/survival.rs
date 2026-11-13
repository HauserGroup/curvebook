//! Survival analysis: Kaplan–Meier curves, Greenwood / log-log confidence bands,
//! Brookmeyer–Crowley median survival CI, Log-rank (Mantel–Cox), Gehan–Breslow–Wilcoxon,
//! log-rank trend, Hazard Ratios, and Cox Proportional Hazards regression (Efron & Breslow ties)
//! with Schoenfeld residual proportional-hazards diagnostics.

use lumencraft_doc::{
    AnalysisSpec, ChecklistStatus, CoxTieMethod, CurveSamplePoint, DataSheet, FittedCurveOverlay, PValueFormat, PairwiseComparisonRecord, ResultCell,
    ResultSheet, ResultTabKind, SurvivalAnalysisSpec, SurvivalObservation, SurvivalStatus, TableArchetype,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{chi2_sf, norm_two_sided_isf, norm_two_sided_p};
use lumencraft_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares};
use serde::{Deserialize, Serialize};

use crate::common::{
    build_tab, checklist_item, ci_cell, init_result_sheet, num_cell, opt_ci_cell, opt_num_cell, p_cell, push_scalar, resolve_selected_indices,
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

/// Two-group hazard ratio summary from Mantel–Haenszel and log-rank estimators.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HazardRatioResult {
    pub log_rank_hr: f64,
    pub log_rank_ci: (f64, f64),
    pub mantel_haenszel_hr: f64,
    pub mantel_haenszel_ci: (f64, f64),
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
}

// ============================================================================
// Kaplan–Meier, Brookmeyer–Crowley Median CI, Log-Rank & Wilcoxon
// ============================================================================

/// Compute Kaplan–Meier survival curve, Greenwood SE, pointwise CIs (asymmetrical log-log or linear),
/// and Brookmeyer–Crowley median survival CI.
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
    let median_ci = brookmeyer_crowley_median_ci(&steps, z_crit);

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

/// Brookmeyer–Crowley (1982) confidence interval for median survival using the log-log transformed
/// Greenwood interval crossing `S(t) = 0.5` (matching standard `survfit` default `"log"`/`"log-log"`).
fn brookmeyer_crowley_median_ci(steps: &[KaplanMeierStep], z_crit: f64) -> (Option<f64>, Option<f64>) {
    let lo_time = steps.iter().find(|s| s.ci_lower <= 0.5 + 1e-12).map(|s| s.time);
    let hi_time = steps.iter().find(|s| s.ci_upper < 0.5 - 1e-12).map(|s| s.time);
    let _ = z_crit;
    (lo_time, hi_time)
}

#[derive(Debug, Clone, Copy)]
enum SurvivalWeight {
    LogRank,
    GehanWilcoxon,
}

/// Multi-cohort log-rank (Mantel–Cox) or Gehan–Breslow–Wilcoxon chi-square test using exact
/// hypergeometric variance covariance matrix over distinct event times.
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

/// Mantel–Haenszel and Log-rank Hazard Ratio estimates with `confidence` intervals for two cohorts.
pub fn two_cohort_hazard_ratio(
    cohort_a: &[SurvivalObservation],
    cohort_b: &[SurvivalObservation],
    confidence: ConfidenceLevel,
) -> Result<HazardRatioResult, StatsError> {
    let (_, cov, obs, exp) = weighted_log_rank_stats(&[cohort_a, cohort_b], SurvivalWeight::LogRank)?;
    let o1 = obs[0];
    let o2 = obs[1];
    let e1 = exp[0];
    let e2 = exp[1];
    let v = cov[0].max(1e-18);
    let z_crit = norm_two_sided_isf(confidence.alpha())?;

    // Log-rank HR = exp((O1 - E1) / V) (or ratio of relative event rates (O1/E1)/(O2/E2)).
    let lr_hr = if e1 > 0.0 && e2 > 0.0 && o2 > 0.0 { (o1 / e1) / (o2 / e2) } else { ((o1 - e1) / v).exp() };
    let se_log = (1.0 / v).sqrt();
    let lr_lo = (lr_hr.max(1e-15).ln() - z_crit * se_log).exp();
    let lr_hi = (lr_hr.max(1e-15).ln() + z_crit * se_log).exp();

    let mh_hr = ((o1 - e1) / v).exp();
    let mh_lo = ((o1 - e1) / v - z_crit * se_log).exp();
    let mh_hi = ((o1 - e1) / v + z_crit * se_log).exp();

    Ok(HazardRatioResult { log_rank_hr: lr_hr, log_rank_ci: (lr_lo, lr_hi), mantel_haenszel_hr: mh_hr, mantel_haenszel_ci: (mh_lo, mh_hi) })
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

        if d_total > 0 && n_total > 1 {
            let nf = n_total as f64;
            let df = d_total as f64;
            let w = match weight_kind {
                SurvivalWeight::LogRank => 1.0,
                SurvivalWeight::GehanWilcoxon => nf,
            };
            let var_mult = w * w * df * (nf - df) / (nf * nf * (nf - 1.0));
            for g in 0..k {
                let ng = at_risk[g] as f64;
                let dg = d_g[g] as f64;
                let eg = (ng / nf) * df;
                obs_tot[g] += dg;
                exp_tot[g] += eg;
                u[g] += w * (dg - eg);
                for h in 0..k {
                    let nh = at_risk[h] as f64;
                    let delta = if g == h { 1.0 } else { 0.0 };
                    cov[g * k + h] += var_mult * ng * (delta - nh / nf);
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
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));

    let zero_beta = vec![0.0_f64; p];
    let (ll_null, grad_null, info_null) = cox_eval(&ordered, &zero_beta, tie_method, p)?;
    let score_chi2 = solve_quad_form(&info_null, &grad_null, p).unwrap_or(0.0);

    let mut beta = vec![0.0_f64; p];
    let mut ll_curr = ll_null;
    let mut grad = grad_null;
    let mut info = info_null;
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

    let info_mat = DenseMatrix::from_row_major(p, p, info.clone())?;
    let cov_beta = if let Ok(chol) = cholesky(&info_mat) {
        chol.inverse()?
    } else {
        let mut damped = info.clone();
        for j in 0..p {
            damped[j * p + j] += 1e-6;
        }
        let dm = DenseMatrix::from_row_major(p, p, damped)?;
        match cholesky(&dm).and_then(|c| c.inverse()) {
            Ok(inv) => inv,
            Err(_) => DenseMatrix::identity(p)?,
        }
    };

    let lr_chi2 = (2.0 * (ll_curr - ll_null)).max(0.0);
    let wald_chi2 = solve_quad_form(&info, &beta, p).unwrap_or(0.0);
    let df = p as f64;
    let lr_p = chi2_sf(lr_chi2, df)?;
    let wald_p = chi2_sf(wald_chi2, df)?;
    let score_p = chi2_sf(score_chi2, df)?;

    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let (schoenfeld_rhos, schoenfeld_ps, global_sch_chi2) = schoenfeld_diagnostics(&ordered, &beta, &cov_beta, tie_method, p)?;
    let global_sch_p = chi2_sf(global_sch_chi2, df)?;

    let mut covariates = Vec::with_capacity(p);
    for j in 0..p {
        let se = cov_beta.get(j, j).unwrap_or(0.0).max(0.0).sqrt();
        let z_stat = if se > 0.0 { beta[j] / se } else { 0.0 };
        let p_val = norm_two_sided_p(z_stat);
        let hr = beta[j].exp();
        let hr_lo = (beta[j] - z_crit * se).exp();
        let hr_hi = (beta[j] + z_crit * se).exp();
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

    let (martingale_residuals, deviance_residuals) = cox_residuals(&ordered, &beta, tie_method, n);
    let concordance = harrell_c_index(&ordered, &beta);

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
    })
}

fn cox_eval(
    ordered: &[(f64, bool, Vec<f64>, usize)],
    beta: &[f64],
    tie_method: CoxTieMethod,
    p: usize,
) -> Result<(f64, Vec<f64>, Vec<f64>), StatsError> {
    let n = ordered.len();
    let mut eta = vec![0.0_f64; n];
    let mut w = vec![0.0_f64; n];
    for i in 0..n {
        let lp: f64 = ordered[i].2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
        eta[i] = lp.clamp(-50.0, 50.0);
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
            let denom = (s0_risk - frac * s0_evt).max(1e-18);
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

fn schoenfeld_diagnostics(
    ordered: &[(f64, bool, Vec<f64>, usize)],
    beta: &[f64],
    cov_beta: &DenseMatrix,
    tie_method: CoxTieMethod,
    p: usize,
) -> Result<(Vec<f64>, Vec<f64>, f64), StatsError> {
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

    // Grambsch-Therneau (1994) scaled Schoenfeld correlation test against event-rank time.
    let g_times: Vec<f64> = (1..=n_evt).map(|r| (r as f64) / ((n_evt + 1) as f64)).collect();
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
        let z2 = if cov_beta.get(j, j).unwrap_or(0.0) > 0.0 && g_ss > 0.0 {
            (n_ef * cov_beta.get(j, j).unwrap_or(0.0) * sum_scaled_t[j] * sum_scaled_t[j]) / g_ss
        } else {
            0.0
        };
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

fn cox_residuals(ordered: &[(f64, bool, Vec<f64>, usize)], beta: &[f64], tie_method: CoxTieMethod, n: usize) -> (Vec<f64>, Vec<f64>) {
    let w: Vec<f64> = ordered.iter().map(|s| s.2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum::<f64>().clamp(-50.0, 50.0).exp()).collect();
    let mut s0_rev = vec![0.0_f64; n + 1];
    for i in (0..n).rev() {
        s0_rev[i] = s0_rev[i + 1] + w[i];
    }

    let mut cum_h0 = 0.0_f64;
    let mut mart = vec![0.0_f64; n];
    let mut dev = vec![0.0_f64; n];

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
        if !evts.is_empty() {
            let s0_e: f64 = evts.iter().map(|&ei| w[ei]).sum();
            let d = evts.len() as f64;
            for m in 0..evts.len() {
                let frac = if tie_method == CoxTieMethod::Efron { (m as f64) / d } else { 0.0 };
                cum_h0 += 1.0 / (s0_rev[r_start] - frac * s0_e).max(1e-18);
            }
        }
        for &i in &grp {
            let delta = if ordered[i].1 { 1.0 } else { 0.0 };
            let m = delta - w[i] * cum_h0;
            let orig = ordered[i].3;
            mart[orig] = m;
            let sign = if m >= 0.0 { 1.0 } else { -1.0 };
            let d_sq = (-2.0 * (m + delta * (delta - m).max(1e-18).ln())).max(0.0);
            dev[orig] = sign * d_sq.sqrt();
        }
    }
    (mart, dev)
}

fn harrell_c_index(ordered: &[(f64, bool, Vec<f64>, usize)], beta: &[f64]) -> f64 {
    let lp: Vec<f64> = ordered.iter().map(|s| s.2.iter().zip(beta.iter()).map(|(x, b)| x * b).sum()).collect();
    let mut conc = 0.0_f64;
    let mut pairs = 0.0_f64;
    for i in 0..ordered.len() {
        if !ordered[i].1 {
            continue;
        }
        for j in (i + 1)..ordered.len() {
            if ordered[j].0 <= ordered[i].0 && !ordered[j].1 {
                continue;
            }
            if ordered[j].0 > ordered[i].0 {
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
        SurvivalAnalysisSpec::KaplanMeier { log_rank_mantel_cox, gehan_breslow_wilcoxon, trend_test, asymmetrical_greenwood_ci, multiplicity } => {
            let TableArchetype::Survival(table) = &sheet.archetype else {
                return Err(StatsError::IncompatibleTable { analysis: "Kaplan-Meier survival", actual: sheet.archetype.kind() });
            };
            let indices = resolve_selected_indices(&spec.selected_data_sets, table.cohorts.len());
            if indices.is_empty() {
                return Err(StatsError::EmptyInput);
            }

            let mut cohort_obs = Vec::with_capacity(indices.len());
            let mut km_results = Vec::with_capacity(indices.len());
            for (pos, &c_idx) in indices.iter().enumerate() {
                let obs = table.cohort_observations(c_idx)?;
                let km = kaplan_meier_curve(&table.cohorts[c_idx].title, &obs, *asymmetrical_greenwood_ci, confidence)?;
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
                    push_scalar(&mut out, "Hazard ratio (log-rank)", None, hr.log_rank_hr);
                    push_scalar(&mut out, "Hazard ratio (Mantel-Haenszel)", None, hr.mantel_haenszel_hr);
                } else if refs.len() >= 3 {
                    append_survival_pairwise(&mut out, &indices, &km_results, &cohort_obs, multiplicity, p_fmt, confidence)?;
                }
            }

            let min_events = km_results.iter().map(|k| k.n_events).min().unwrap_or(0);
            out.checklist.push(checklist_item(
                "Event counts & censoring",
                if min_events >= 5 { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
                Some(format!("min cohort events = {min_events}")),
                "Kaplan-Meier assumes non-informative censoring; medians cannot be estimated if survival remains above 0.5.",
            ));
        }
        SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, reference_levels, tie_method } => {
            let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
                return Err(StatsError::IncompatibleTable { analysis: "Cox proportional hazards", actual: sheet.archetype.kind() });
            };
            let _ = reference_levels;
            let time_var = mv.variables.get(*time_column).ok_or(StatsError::MissingColumn { index: *time_column })?;
            let status_var = mv.variables.get(*status_column).ok_or(StatsError::MissingColumn { index: *status_column })?;
            let mut pred_names = Vec::with_capacity(predictor_columns.len());
            for &pc in predictor_columns {
                let pv = mv.variables.get(pc).ok_or(StatsError::MissingColumn { index: pc })?;
                pred_names.push(pv.title.clone());
            }

            let mut subjects = Vec::new();
            for r in 0..mv.row_count() {
                let Some(t) = time_var.cells.get(r).and_then(|c| c.active_number()) else { continue };
                let Some(st) = status_var.cells.get(r).and_then(|c| c.active_number()) else { continue };
                let mut x_vals = Vec::with_capacity(predictor_columns.len());
                let mut ok = true;
                for &pc in predictor_columns {
                    if let Some(xv) = mv.variables[pc].cells.get(r).and_then(|c| c.active_number()) {
                        x_vals.push(xv);
                    } else {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    subjects.push(CoxSubject { time: t, event: st != 0.0, x: x_vals });
                }
            }

            let cox = cox_ph_regression(&subjects, &pred_names, *tie_method, confidence)?;
            push_scalar(&mut out, "Concordance (C-index)", None, cox.concordance);
            push_scalar(&mut out, "Likelihood ratio chi2", None, cox.likelihood_ratio_chi2);
            push_scalar(&mut out, "Likelihood ratio P", None, cox.likelihood_ratio_p);
            push_scalar(&mut out, "Global Schoenfeld P", None, cox.global_schoenfeld_p);

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
            out.tabs.push(build_tab(
                "cox-ph",
                "Cox proportional hazards estimates",
                ResultTabKind::ParametersAndConfidenceIntervals,
                headers,
                labels,
                rows,
            ));

            let ph_ok = cox.global_schoenfeld_p >= 0.05;
            out.checklist.push(checklist_item(
                "Proportional hazards assumption (Schoenfeld)",
                if ph_ok { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
                Some(format!("Global chi2 = {:.3}, P = {:.4}", cox.global_schoenfeld_chi2, cox.global_schoenfeld_p)),
                if ph_ok {
                    "Scaled Schoenfeld residuals show no significant time-varying slope (PH assumption satisfied)."
                } else {
                    "Schoenfeld residual test suggests time-varying hazard ratios (P < 0.05); inspect stratified or time-dependent models."
                },
            ));
        }
    }

    Ok(out)
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
        MultiplicityProcedure::None => MultiplicityProcedure::None,
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
            num_cell(hr.log_rank_hr),
            opt_ci_cell(Some(hr.log_rank_ci)),
            ResultCell::Boolean(sig),
        ]);
        out.pairwise_comparisons.push(PairwiseComparisonRecord {
            group_a: indices[i],
            group_b: indices[j],
            label_a: lbl_a,
            label_b: lbl_b,
            estimate_diff: Some(hr.log_rank_hr),
            raw_p_or_q: ap,
            is_fdr_q: is_fdr,
            significant: sig,
        });
    }
    out.tabs.push(build_tab("survival-pairwise", "Pairwise survival comparisons", ResultTabKind::MultipleComparisons, headers, row_labels, rows));
    Ok(())
}
