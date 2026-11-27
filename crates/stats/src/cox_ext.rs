//! Cox Proportional Hazards and Kaplan–Meier survival extensions: Venzon–Moolgavkar
//! profile-likelihood confidence intervals, covariate-profile predicted survival curves
//! $S(t \mid x_0)$, Cox–Snell and covariate residual diagnostics, Cox covariate VIFs,
//! configurable status-code & reverse-KM follow-up policies, and two-arm median survival
//! ratio with confidence interval.
//!
//! # References
//!
//! - Cox, D. R. (1972). Regression models and life-tables. *Journal of the Royal Statistical
//!   Society, Series B*, 34(2), 187–220. `doi:10.1111/j.2517-6161.1972.tb00899.x`.
//! - Breslow, N. (1974). Covariance analysis of censored survival data. *Biometrics*, 30(1), 89–99.
//!   `doi:10.2307/2529620`.
//! - Efron, B. (1977). The efficiency of Cox's likelihood function for censored data. *Journal of
//!   the American Statistical Association*, 72(359), 557–565.
//!   `doi:10.1080/01621459.1977.10480613`.
//! - Brookmeyer, R., & Crowley, J. (1982). A confidence interval for the median survival time.
//!   *Biometrics*, 38(1), 29–41. `doi:10.2307/2530286`.
//! - Venzon, D. J., & Moolgavkar, S. H. (1988). A method for computing profile-likelihood-based
//!   confidence intervals. *Journal of the Royal Statistical Society, Series C (Applied
//!   Statistics)*, 37(1), 87–94. `doi:10.2307/2347496`.
//! - Schemper, M., & Smith, T. L. (1996). A note on quantifying follow-up in studies of failure
//!   time. *Controlled Clinical Trials*, 17(4), 343–346. `doi:10.1016/0197-2456(96)00075-x`.
//! - Therneau, T. M., & Grambsch, P. M. (2000). *Modeling Survival Data: Extending the Cox Model*.
//!   Springer. `doi:10.1007/978-1-4757-3294-8`.
//! - Burnham, K. P., & Anderson, D. R. (2002). *Model Selection and Multimodel Inference* (2nd
//!   ed.). Springer.
//! - Klein, J. P., & Moeschberger, M. L. (2003). *Survival Analysis: Techniques for Censored and
//!   Truncated Data* (2nd ed.). Springer.

use curvebook_doc::{CoxTieMethod, SurvivalObservation, SurvivalOtherValuePolicy, SurvivalStatus};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::norm_two_sided_isf;
use curvebook_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares};
use curvebook_math::sum::SummaryMoments;
use serde::{Deserialize, Serialize};

use crate::error::StatsError;
use crate::glm_ext::brent_root;
use crate::survival::{
    CoxBaselinePoint, CoxSubject, KaplanMeierCohortResult, OrderedCoxSubject, PreparedCoxOrder, cox_eval_ordered_with_offset, kaplan_meier_curve,
    prepare_cox_ordered,
};

/// Two-arm ratio of Kaplan–Meier median survival times with reciprocal and log-scale CI.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MedianSurvivalRatio {
    /// Ratio `median_A / median_B`.
    pub ratio_a_over_b: f64,
    /// Confidence interval for `median_A / median_B` (propagated on the log scale from the two
    /// arms' independent Brookmeyer–Crowley median CIs when all four bounds are finite and positive).
    pub ci_a_over_b: Option<(f64, f64)>,
    /// Reciprocal ratio `median_B / median_A`.
    pub ratio_b_over_a: f64,
    /// Reciprocal confidence interval `(1 / hi, 1 / lo)` for `median_B / median_A`.
    pub ci_b_over_a: Option<(f64, f64)>,
}

/// Subject-level Cox–Snell, Martingale, and Deviance diagnostic record.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoxSnellRow {
    /// Observed follow-up time `t_i`.
    pub time: f64,
    /// Event indicator (`true` = failure, `false` = right-censored).
    pub event: bool,
    /// Fitted linear predictor `eta_i = x_i^T beta`.
    pub linear_predictor: f64,
    /// Cox–Snell residual `r_CS,i = delta_i - M_i >= 0`.
    pub cox_snell: f64,
    /// Martingale residual `M_i`.
    pub martingale: f64,
    /// Deviance residual `D_i`.
    pub deviance: f64,
    /// Kaplan–Meier cumulative hazard `-ln S_KM(r_CS,i)` evaluated on `(r_CS, event)` for the
    /// 45-degree exponential diagnostic check (`None` when `S_KM == 0`).
    pub km_cum_hazard: Option<f64>,
}

/// Parse a numeric survival status code honoring optional custom `event_code` / `censored_code`
/// and [`SurvivalOtherValuePolicy`].
///
/// To preserve strict legacy validation, when `event_code` and `censored_code` are both `None`
/// and `treat_other_as == SurvivalOtherValuePolicy::Exclude`, any code other than `0.0` or `1.0`
/// returns [`StatsError::InvalidParameter`]. Whenever a custom code or non-default policy is
/// configured, unrecognized values follow `treat_other_as` (`Exclude` returns `Ok(None)` so the
/// observation is skipped cleanly; `Censor` and `Event` map accordingly).
pub fn parse_survival_status_with_policy(
    st: f64,
    event_code: Option<f64>,
    censored_code: Option<f64>,
    treat_other_as: SurvivalOtherValuePolicy,
) -> Result<Option<SurvivalStatus>, StatsError> {
    if !st.is_finite() {
        return Err(StatsError::InvalidParameter { name: "survival_status", value: st });
    }
    let ev = event_code.unwrap_or(1.0);
    let cens = censored_code.unwrap_or(0.0);
    if !ev.is_finite() || !cens.is_finite() || (ev - cens).abs() <= 1e-12 {
        return Err(StatsError::InvalidParameter { name: "survival_status_codes", value: ev });
    }
    let tol = 1e-12 * (1.0 + ev.abs().max(cens.abs()));
    if (st - ev).abs() <= tol {
        return Ok(Some(SurvivalStatus::Event));
    }
    if (st - cens).abs() <= tol {
        return Ok(Some(SurvivalStatus::Censored));
    }
    if event_code.is_none() && censored_code.is_none() && treat_other_as == SurvivalOtherValuePolicy::Exclude {
        return Err(StatsError::InvalidParameter { name: "survival_status", value: st });
    }
    match treat_other_as {
        SurvivalOtherValuePolicy::Exclude => Ok(None),
        SurvivalOtherValuePolicy::Censor => Ok(Some(SurvivalStatus::Censored)),
        SurvivalOtherValuePolicy::Event => Ok(Some(SurvivalStatus::Event)),
    }
}

/// Compute the ratio of two independent Kaplan–Meier median survival times (`m_A / m_B` and
/// `m_B / m_A`) along with a log-scale confidence interval propagated from each arm's finite
/// Brookmeyer–Crowley (1982) median CI bounds.
#[must_use]
pub fn median_survival_ratio_ci(
    km_a: &KaplanMeierCohortResult,
    km_b: &KaplanMeierCohortResult,
    confidence: ConfidenceLevel,
) -> Option<MedianSurvivalRatio> {
    let (Some(ma), Some(mb)) = (km_a.median_survival, km_b.median_survival) else {
        return None;
    };
    if !ma.is_finite() || !mb.is_finite() || ma <= 0.0 || mb <= 0.0 {
        return None;
    }
    let ratio_ab = ma / mb;
    let ratio_ba = mb / ma;
    let z = norm_two_sided_isf(confidence.alpha()).ok()?;

    let ci_ab = match (km_a.median_ci, km_b.median_ci) {
        ((Some(la), Some(ua)), (Some(lb), Some(ub))) if la > 0.0 && ua > la && lb > 0.0 && ub > lb => {
            let se_log_a = (ua.ln() - la.ln()) / (2.0 * z);
            let se_log_b = (ub.ln() - lb.ln()) / (2.0 * z);
            let se_log_r = se_log_a.hypot(se_log_b);
            if se_log_r.is_finite() && se_log_r > 0.0 {
                let ln_r = ratio_ab.ln();
                let lo = (ln_r - z * se_log_r).exp();
                let hi = (ln_r + z * se_log_r).exp();
                (lo.is_finite() && hi.is_finite() && lo > 0.0).then_some((lo, hi))
            } else {
                None
            }
        }
        _ => None,
    };
    let ci_ba = ci_ab.and_then(|(lo, hi)| {
        let r_lo = 1.0 / hi;
        let r_hi = 1.0 / lo;
        (r_lo.is_finite() && r_hi.is_finite()).then_some((r_lo, r_hi))
    });
    Some(MedianSurvivalRatio { ratio_a_over_b: ratio_ab, ci_a_over_b: ci_ab, ratio_b_over_a: ratio_ba, ci_b_over_a: ci_ba })
}

/// Compute Variance Inflation Factors `VIF_j = 1 / (1 - R_j^2)` across the `p` covariates of a
/// Cox PH design matrix (regressing each covariate on an intercept plus the remaining `p - 1`
/// covariates). Returns `None` for all entries when `p <= 1`.
#[must_use]
pub fn compute_cox_vifs(subjects: &[CoxSubject], p: usize) -> Vec<Option<f64>> {
    let n = subjects.len();
    let mut vifs = vec![None; p];
    if p <= 1 || n <= p || subjects.iter().any(|s| s.x.len() != p) {
        return vifs;
    }
    for (j, slot) in vifs.iter_mut().enumerate() {
        let target: Vec<f64> = subjects.iter().map(|s| s.x[j]).collect();
        let Ok(tm) = SummaryMoments::from_slice(&target) else { continue };
        if tm.css <= 0.0 {
            continue;
        }
        let mut pred_data = Vec::with_capacity(n * p);
        for s in subjects {
            pred_data.push(1.0);
            for (k, &xk) in s.x.iter().enumerate() {
                if k != j {
                    pred_data.push(xk);
                }
            }
        }
        let Ok(xm) = DenseMatrix::from_row_major(n, p, pred_data) else { continue };
        if let Ok((sol, _)) = solve_linear_least_squares(&xm, &target) {
            let r2 = (1.0 - sol.rss / tm.css).clamp(0.0, 0.999_999_999);
            *slot = Some(1.0 / (1.0 - r2));
        }
    }
    vifs
}

/// Evaluate predicted cumulative hazard `H(t | x_0)` and survival `S(t | x_0) = exp(-H(t | x_0))`
/// for a covariate profile `profile_x` from the mean-centered Cox baseline survival schedule
/// (`survival::survfit.coxph` convention; Breslow 1974; Efron 1977; Therneau & Grambsch 2000).
pub fn predict_cox_profile_survival(
    baseline_centered: &[CoxBaselinePoint],
    x_means: &[f64],
    beta: &[f64],
    profile_x: &[f64],
) -> Result<Vec<CoxBaselinePoint>, StatsError> {
    let p = beta.len();
    if x_means.len() != p || profile_x.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: profile_x.len() });
    }
    let mut eta_shift = 0.0_f64;
    for j in 0..p {
        if !beta[j].is_finite() || !x_means[j].is_finite() || !profile_x[j].is_finite() {
            return Err(StatsError::InvalidParameter { name: "cox_profile_x", value: profile_x[j] });
        }
        eta_shift += beta[j] * (profile_x[j] - x_means[j]);
    }
    let mult = eta_shift.clamp(-500.0, 500.0).exp();
    let mut out = Vec::with_capacity(baseline_centered.len());
    for pt in baseline_centered {
        let cum_h = pt.cum_hazard * mult;
        let surv = if cum_h.is_finite() { (-cum_h).exp().clamp(0.0, 1.0) } else { 0.0 };
        out.push(CoxBaselinePoint { time: pt.time, cum_hazard: cum_h, survival: surv });
    }
    Ok(out)
}

/// Compute subject-level Cox–Snell residuals `r_CS,i = delta_i - M_i` paired with the Kaplan–Meier
/// cumulative hazard `-ln S_KM(r_CS,i)` on `(r_CS, event)` for the 45-degree model-adequacy plot
/// (Therneau & Grambsch 2000; Klein & Moeschberger 2003).
pub fn compute_cox_snell_rows(
    subjects: &[CoxSubject],
    beta: &[f64],
    martingale_residuals: &[f64],
    deviance_residuals: &[f64],
) -> Result<Vec<CoxSnellRow>, StatsError> {
    let n = subjects.len();
    if martingale_residuals.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: martingale_residuals.len() });
    }
    if deviance_residuals.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: deviance_residuals.len() });
    }
    if let Some(bad) = subjects.iter().find(|s| s.x.len() != beta.len()) {
        return Err(StatsError::DimensionMismatch { expected: beta.len(), actual: bad.x.len() });
    }
    let mut r_cs = Vec::with_capacity(n);
    let mut lps = Vec::with_capacity(n);
    let mut obs = Vec::with_capacity(n);
    for (i, s) in subjects.iter().enumerate() {
        let lp: f64 = s.x.iter().zip(beta.iter()).map(|(&x, &b)| x * b).sum();
        let delta = if s.event { 1.0 } else { 0.0 };
        let cs = (delta - martingale_residuals[i]).max(0.0);
        lps.push(lp);
        r_cs.push(cs);
        obs.push(SurvivalObservation { row: i, time: cs, status: if s.event { SurvivalStatus::Event } else { SurvivalStatus::Censored } });
    }
    let km = kaplan_meier_curve("Cox-Snell", &obs, true, ConfidenceLevel::DEFAULT_95).ok();
    let lookup_km_h = |val: f64| -> Option<f64> {
        let km_res = km.as_ref()?;
        let mut s_val = 1.0_f64;
        for step in &km_res.steps {
            if step.time <= val + 1e-12 {
                s_val = step.survival;
            } else {
                break;
            }
        }
        (s_val > 0.0 && s_val <= 1.0).then(|| (-s_val.ln()).max(0.0))
    };

    let mut rows = Vec::with_capacity(n);
    for (i, s) in subjects.iter().enumerate() {
        rows.push(CoxSnellRow {
            time: s.time,
            event: s.event,
            linear_predictor: lps[i],
            cox_snell: r_cs[i],
            martingale: martingale_residuals[i],
            deviance: deviance_residuals[i],
            km_cum_hazard: lookup_km_h(r_cs[i]),
        });
    }
    Ok(rows)
}

/// Compute Cox PH small-sample corrected AIC by number of events (`Burnham & Anderson 2002`):
/// `AIC = -2 * ll_model + 2 * k`, `AICc = AIC + 2 * k * (k + 1) / (n_events - k - 1)`.
#[must_use]
pub fn cox_aic_and_aicc_by_events(log_lik_model: f64, k_params: usize, n_events: usize) -> (f64, f64) {
    let kf = k_params as f64;
    let aic = -2.0 * log_lik_model + 2.0 * kf;
    let denom = (n_events as f64) - kf - 1.0;
    let aicc = if denom > 0.0 { aic + (2.0 * kf * (kf + 1.0)) / denom } else { f64::INFINITY };
    (aic, aicc)
}

/// Solve the nuisance partial likelihood `max_{beta_{-j}} ell(beta_{-j}; offset)` using warm-started
/// Newton–Raphson with step-halving on standardized nuisance covariates.
fn solve_cox_nuisance_ll(
    ordered_rest: &[(f64, bool, Vec<f64>, usize)],
    clusters: &[crate::survival::CoxTimeCluster],
    offset: &[f64],
    tie_method: CoxTieMethod,
    beta_warm: &mut [f64],
) -> Option<f64> {
    let q = beta_warm.len();
    if q == 0 {
        return cox_eval_ordered_with_offset(ordered_rest, clusters, &[], Some(offset), tie_method, 0).ok().map(|(ll, _, _)| ll);
    }
    let (mut ll_curr, mut grad, mut info) = cox_eval_ordered_with_offset(ordered_rest, clusters, beta_warm, Some(offset), tie_method, q).ok()?;
    if !ll_curr.is_finite() {
        beta_warm.fill(0.0);
        let reset = cox_eval_ordered_with_offset(ordered_rest, clusters, beta_warm, Some(offset), tie_method, q).ok()?;
        ll_curr = reset.0;
        grad = reset.1;
        info = reset.2;
    }
    for _ in 0..50 {
        let info_mat = DenseMatrix::from_row_major(q, q, info.clone()).ok()?;
        let step = if let Ok(chol) = cholesky(&info_mat) {
            chol.solve(&grad).ok()?
        } else {
            let mut damped = info.clone();
            for j in 0..q {
                damped[j * q + j] += 1e-4;
            }
            let dm = DenseMatrix::from_row_major(q, q, damped).ok()?;
            solve_linear_least_squares(&dm, &grad).ok()?.0.coefficients
        };
        let max_step = step.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
        if !max_step.is_finite() {
            return None;
        }
        if max_step < 1e-11 {
            return Some(ll_curr);
        }
        let max_lp = ordered_rest.iter().map(|s| s.2.iter().zip(step.iter()).map(|(&x, &d)| x * d).sum::<f64>().abs()).fold(0.0_f64, f64::max);
        let mut scale = if max_lp > 8.0 { 8.0 / max_lp } else { 1.0 };
        let mut accepted = false;
        for _ in 0..24 {
            let cand: Vec<f64> = beta_warm.iter().zip(step.iter()).map(|(&b, &s)| b + scale * s).collect();
            if let Ok((ll_c, g_c, i_c)) = cox_eval_ordered_with_offset(ordered_rest, clusters, &cand, Some(offset), tie_method, q)
                && ll_c.is_finite()
                && ll_c >= ll_curr - 1e-12
            {
                let delta = (ll_c - ll_curr).abs();
                beta_warm.copy_from_slice(&cand);
                ll_curr = ll_c;
                grad = g_c;
                info = i_c;
                accepted = true;
                if delta < 1e-12 && max_step * scale < 1e-9 {
                    return Some(ll_curr);
                }
                break;
            }
            scale *= 0.5;
        }
        if !accepted {
            break;
        }
    }
    Some(ll_curr)
}

/// Compute Venzon–Moolgavkar (1988) profile-likelihood confidence intervals on the log-hazard
/// scale (`beta`) for every covariate in a converged Cox PH model via warm-started offset partial
/// likelihood maximization and bounded Brent root-finding.
pub fn cox_profile_likelihood_ci(
    subjects: &[CoxSubject],
    beta_hat: &[f64],
    se_hat: &[f64],
    ll_max: f64,
    tie_method: CoxTieMethod,
    confidence: ConfidenceLevel,
) -> Result<Vec<(f64, f64)>, StatsError> {
    let p = beta_hat.len();
    if p == 0 {
        return Err(StatsError::EmptyInput);
    }
    if se_hat.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: se_hat.len() });
    }
    if let Some(bad) = subjects.iter().find(|s| s.x.len() != p) {
        return Err(StatsError::DimensionMismatch { expected: p, actual: bad.x.len() });
    }
    let z = norm_two_sided_isf(confidence.alpha())?;
    let thresh = ll_max - 0.5 * z * z;
    let mut out = Vec::with_capacity(p);

    // Prepare canonical time ordering once so offset vectors align with `ordered_rest`.
    let PreparedCoxOrder { ordered: ordered_full, clusters, .. } = prepare_cox_ordered(subjects, p)?;

    for j in 0..p {
        let b0 = beta_hat[j];
        let se = se_hat[j];
        if !b0.is_finite() || !se.is_finite() || se <= 0.0 {
            out.push((f64::NAN, f64::NAN));
            continue;
        }
        // Raw covariate column j aligned with ordered_full (using orig_idx).
        let xj_ordered: Vec<f64> = ordered_full.iter().map(|s| subjects[s.3].x[j]).collect();

        // Build reduced subjects with column j removed and standardize them for conditioning.
        let q = p - 1;
        let (ordered_rest, rest_sds): (Vec<OrderedCoxSubject>, Vec<f64>) = if q == 0 {
            let empty_ord = ordered_full.iter().map(|s| (s.0, s.1, Vec::new(), s.3)).collect();
            (empty_ord, Vec::new())
        } else {
            let sub_rest: Vec<CoxSubject> = subjects
                .iter()
                .map(|s| {
                    let xr: Vec<f64> = s.x.iter().enumerate().filter(|&(k, _)| k != j).map(|(_, &v)| v).collect();
                    CoxSubject { time: s.time, event: s.event, x: xr }
                })
                .collect();
            let prep_r = prepare_cox_ordered(&sub_rest, q)?;
            (prep_r.ordered, prep_r.x_sds)
        };
        let b_rest_mle_std: Vec<f64> =
            beta_hat.iter().enumerate().filter(|&(k, _)| k != j).zip(rest_sds.iter()).map(|((_, &b), &sd)| b * sd).collect();

        let solve_side = |dir: f64| -> f64 {
            let mut b_warm = b_rest_mle_std.clone();
            let mut eval_diff = |bj: f64| -> Option<f64> {
                let offset: Vec<f64> = xj_ordered.iter().map(|&xj| bj * xj).collect();
                let ll = solve_cox_nuisance_ll(&ordered_rest, &clusters, &offset, tie_method, &mut b_warm)?;
                Some(ll - thresh)
            };
            let mut prev_x = b0;
            let mut prev_f = ll_max - thresh;
            let mut cur_step = (z * se).max(1e-4);
            let max_dist = (40.0 * se).max(50.0);
            for _ in 0..24 {
                let cand = prev_x + dir * cur_step;
                if (cand - b0).abs() > max_dist {
                    return if dir < 0.0 { f64::NEG_INFINITY } else { f64::INFINITY };
                }
                let Some(fc) = eval_diff(cand) else {
                    cur_step *= 0.5;
                    continue;
                };
                if fc <= 0.0 {
                    let (a, fa, b, fb) = if dir < 0.0 { (cand, fc, prev_x, prev_f) } else { (prev_x, prev_f, cand, fc) };
                    return brent_root(eval_diff, a, fa, b, fb, 1e-10 * (1.0 + b0.abs())).unwrap_or(cand);
                }
                prev_x = cand;
                prev_f = fc;
                cur_step *= 1.6;
            }
            if dir < 0.0 { f64::NEG_INFINITY } else { f64::INFINITY }
        };

        out.push((solve_side(-1.0), solve_side(1.0)));
    }
    Ok(out)
}
