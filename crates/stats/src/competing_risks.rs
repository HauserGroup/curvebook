//! Competing-risks Aalen–Johansen cumulative incidence function (CIF) and Gray's (1988)
//! *K*-sample weighted log-rank test.
//!
//! Computes nonparametric Aalen–Johansen cumulative incidence curves $\hat{I}_k(t)$ for a primary
//! event of interest in the presence of competing events, pointwise Greenwood / Marubini–Valsecchi
//! standard errors and complementary log-log (or symmetric linear) confidence intervals, and
//! Gray's (1988) *K*-sample subdistribution hazard test across $K \ge 2$ cohorts.
//!
//! References:
//! - Aalen, O. O., & Johansen, S. (1978). An empirical transition matrix for non-homogeneous
//!   Markov chains based on censored observations. *Scandinavian Journal of Statistics*, 5(3),
//!   141-150.
//! - Choudhury, J. B. (2002). Non-parametric confidence interval estimation for competing risks
//!   analysis: application to contraceptive data. *Statistics in Medicine*, 21(8), 1129-1144.
//!   `doi:10.1002/sim.1070`.
//! - Fine, J. P., & Gray, R. J. (1999). A proportional hazards model for the subdistribution of a
//!   competing risk. *Journal of the American Statistical Association*, 94(446), 496-509.
//!   `doi:10.1080/01621459.1999.10474144`.
//! - Gray, R. J. (1988). A class of *K*-sample tests for comparing the cumulative incidence of a
//!   competing risk. *Annals of Statistics*, 16(3), 1141-1154.
//!   `doi:10.1214/aos/1176350951`.
//! - Klein, J. P., & Moeschberger, M. L. (2003). *Survival Analysis: Techniques for Censored and
//!   Truncated Data* (2nd ed.). New York: Springer.
//! - Marubini, E., & Valsecchi, M. G. (1995). *Analysing Survival Data from Clinical Trials and
//!   Observational Studies*. Chichester: Wiley.

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisSpec, ChecklistStatus, CompetingRisksSpec, CurveSamplePoint, DataSheet, FittedCurveOverlay, ResultSheet, ResultTabKind, TableArchetype,
    VariableColumn, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{chi2_sf, norm_two_sided_isf};
use curvebook_math::linalg::{DenseMatrix, sym_eigen};
use serde::{Deserialize, Serialize};

use crate::common::{
    TabBuilder, append_listwise_filtered_checklist_and_footnote, checklist_item, ci_cell, init_result_sheet, num_cell, opt_num_cell, p_cell_at_alpha,
    push_scalar, resolve_selected_indices, sanitize_result_sheet_floats, text_cell,
};
use crate::error::StatsError;

/// Event classification for a subject in a competing-risks time-to-event analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompetingRiskStatus {
    /// Subject was right-censored without experiencing any failure event.
    Censored,
    /// Subject experienced the primary event of interest (cause 1).
    PrimaryEvent,
    /// Subject experienced a competing event preventing the primary event (cause 2).
    CompetingEvent,
}

/// Single validated competing-risks observation `(time >= 0, status)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CompetingRiskObservation {
    /// Non-negative elapsed follow-up time (`>= 0.0`, finite).
    pub time: f64,
    /// Observed event or censoring status.
    pub status: CompetingRiskStatus,
}

/// One distinct time step on an Aalen–Johansen cumulative incidence curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CifStep {
    /// Distinct observed time $t \ge 0$.
    pub time: f64,
    /// Subjects at risk immediately prior to `time` ($n_j$).
    pub n_at_risk: usize,
    /// Primary events observed at `time` ($d_{1j}$).
    pub n_primary: usize,
    /// Competing events observed at `time` ($d_{2j}$).
    pub n_competing: usize,
    /// Subjects right-censored at `time`.
    pub n_censored: usize,
    /// Overall event-free Kaplan–Meier survival probability $\hat{S}(t) \in [0, 1]$.
    pub overall_survival: f64,
    /// Aalen–Johansen cumulative incidence $\hat{I}_1(t)$ for the primary event.
    pub cif_primary: f64,
    /// Greenwood / Marubini–Valsecchi (1995) standard error of $\hat{I}_1(t)$.
    pub se_primary: f64,
    /// Pointwise confidence interval lower bound for $\hat{I}_1(t)$ in `[0, 1]`.
    pub ci_primary_lower: f64,
    /// Pointwise confidence interval upper bound for $\hat{I}_1(t)$ in `[0, 1]`.
    pub ci_primary_upper: f64,
    /// Aalen–Johansen cumulative incidence $\hat{I}_2(t)$ for competing events.
    pub cif_competing: f64,
    /// Greenwood / Marubini–Valsecchi (1995) standard error of $\hat{I}_2(t)$.
    pub se_competing: f64,
    /// Pointwise confidence interval lower bound for $\hat{I}_2(t)$ in `[0, 1]`.
    pub ci_competing_lower: f64,
    /// Pointwise confidence interval upper bound for $\hat{I}_2(t)$ in `[0, 1]`.
    pub ci_competing_upper: f64,
}

/// Aalen–Johansen cumulative incidence trajectory for one cohort.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CifCohortResult {
    /// Cohort display label.
    pub label: String,
    /// Total subjects in this cohort.
    pub n_total: usize,
    /// Total primary events in this cohort.
    pub n_primary: usize,
    /// Total competing events in this cohort.
    pub n_competing: usize,
    /// Total right-censored subjects in this cohort.
    pub n_censored: usize,
    /// Final cumulative incidence of the primary event at the end of follow-up.
    pub final_cif_primary: f64,
    /// Confidence interval `(lower, upper)` for `final_cif_primary`.
    pub final_ci_primary: (f64, f64),
    /// Final cumulative incidence of competing events at the end of follow-up.
    pub final_cif_competing: f64,
    /// Confidence interval `(lower, upper)` for `final_cif_competing`.
    pub final_ci_competing: (f64, f64),
    /// Chronologically ordered distinct-time steps.
    pub steps: Vec<CifStep>,
}

/// Gray's (1988) *K*-sample weighted log-rank test result for equality of CIFs across cohorts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrayTestResult {
    /// Label of the tested cause (`"Primary event"` or `"Competing event"`).
    pub cause_label: String,
    /// Weight exponent $\rho \ge 0$ (`0.0` = unweighted Gray test).
    pub rho: f64,
    /// Asymptotic $\chi^2$ test statistic (`f64::NAN` if rank is zero).
    pub chi_square: f64,
    /// Asymptotic degrees of freedom (covariance rank $\le K - 1$).
    pub df: f64,
    /// Upper-tail $\chi^2(\text{df})$ *P* value (`f64::NAN` if non-estimable).
    pub p_value: f64,
    /// Per-group weighted subdistribution score contributions `(group_label, S_g)` summing to `0`.
    pub group_scores: Vec<(String, f64)>,
}

/// Complete competing-risks analysis output across one or more cohorts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompetingRisksResult {
    /// Per-cohort Aalen–Johansen CIF results.
    pub cohorts: Vec<CifCohortResult>,
    /// Gray's (1988) test comparing primary-event CIFs across cohorts (`None` when `< 2` cohorts).
    pub gray_primary: Option<GrayTestResult>,
    /// Gray's (1988) test comparing competing-event CIFs across cohorts (`None` when `< 2` cohorts).
    pub gray_competing: Option<GrayTestResult>,
}

#[inline]
fn normalize_time(t: f64) -> f64 {
    if t == 0.0 { 0.0 } else { t }
}

fn cif_pointwise_ci(cif: f64, se: f64, z: f64, log_log: bool) -> (f64, f64) {
    if !cif.is_finite() || !se.is_finite() || se <= 0.0 {
        let clamped = cif.clamp(0.0, 1.0);
        return (clamped, clamped);
    }
    if cif <= 0.0 {
        return (0.0, 0.0);
    }
    if cif >= 1.0 {
        return (1.0, 1.0);
    }
    if !log_log {
        return ((cif - z * se).clamp(0.0, 1.0), (cif + z * se).clamp(0.0, 1.0));
    }
    // Choudhury (2002) complementary log-log transform on `1 - I(t)` (equivalent to log(-log(1 - I(t)))):
    // guarantees bounds strictly within (0, 1) and matches R `survival::survfit(..., conf.type="log-log")` on 1 - I.
    // Specifically: theta = ln(-ln(1 - I)), SE(theta) = SE(I) / ((1 - I) * |ln(1 - I)|),
    // giving I_lo = 1 - (1 - I)^{exp(-z * SE(theta))}, I_hi = 1 - (1 - I)^{exp(+z * SE(theta))}.
    let one_minus = 1.0 - cif;
    let neg_ln = -one_minus.ln();
    if !neg_ln.is_finite() || neg_ln <= 0.0 {
        return ((cif - z * se).clamp(0.0, 1.0), (cif + z * se).clamp(0.0, 1.0));
    }
    let se_ll = se / (one_minus * neg_ln);
    let lo_s = one_minus.powf((z * se_ll).exp());
    let hi_s = one_minus.powf((-z * se_ll).exp());
    let lo = (1.0 - hi_s).clamp(0.0, 1.0);
    let hi = (1.0 - lo_s).clamp(0.0, 1.0);
    (lo.min(hi), lo.max(hi))
}

/// Compute the Aalen–Johansen (1978) cumulative incidence function and Greenwood / Marubini–Valsecchi
/// (1995) variance and confidence intervals for a single cohort.
///
/// At each distinct observed time $t_j$ (with events preceding right-censorings at ties), let
/// $n_j$ be the number at risk, $d_{1j}$ primary events, $d_{2j}$ competing events, and
/// $d_j = d_{1j} + d_{2j}$. With overall survival $\hat{S}(t_j^-) = \prod_{l < j} (1 - d_l / n_l)$,
/// $\hat{I}_k(t) = \sum_{t_j \le t} \hat{S}(t_j^-) d_{kj} / n_j$, and
/// $$\widehat{\mathrm{Var}}[\hat{I}_k(t)] = \sum_{t_j \le t} \left\{ [\hat{I}_k(t) - \hat{I}_k(t_j)]^2 \frac{d_j}{n_j (n_j - d_j)} + [\hat{S}(t_j^-)]^2 \frac{d_{kj}(n_j - d_{kj})}{n_j^3} - 2 [\hat{I}_k(t) - \hat{I}_k(t_j)] \hat{S}(t_j^-) \frac{d_{kj}}{n_j^2} \right\}.$$
///
/// # Errors
/// Returns [`StatsError`] on empty input or negative/non-finite follow-up times.
pub fn aalen_johansen_cif(
    label: impl Into<String>,
    observations: &[CompetingRiskObservation],
    confidence: ConfidenceLevel,
    log_log_ci: bool,
) -> Result<CifCohortResult, StatsError> {
    if observations.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let mut cleaned: Vec<CompetingRiskObservation> = Vec::with_capacity(observations.len());
    for obs in observations {
        if !obs.time.is_finite() || obs.time < 0.0 {
            return Err(StatsError::InvalidParameter { name: "survival_time", value: obs.time });
        }
        cleaned.push(CompetingRiskObservation { time: normalize_time(obs.time), status: obs.status });
    }
    cleaned.sort_by(|a, b| a.time.total_cmp(&b.time));

    let z = norm_two_sided_isf(confidence.alpha())?;
    let n_total = cleaned.len();
    let mut n_primary_tot = 0_usize;
    let mut n_competing_tot = 0_usize;
    let mut n_censored_tot = 0_usize;

    // Group by distinct time.
    let mut times = Vec::new();
    let mut at_risk = Vec::new();
    let mut d1_vec = Vec::new();
    let mut d2_vec = Vec::new();
    let mut cens_vec = Vec::new();

    let mut pos = 0_usize;
    while pos < n_total {
        let t = cleaned[pos].time;
        let n_j = n_total - pos;
        let mut d1 = 0_usize;
        let mut d2 = 0_usize;
        let mut c_j = 0_usize;
        while pos < n_total && cleaned[pos].time == t {
            match cleaned[pos].status {
                CompetingRiskStatus::PrimaryEvent => d1 += 1,
                CompetingRiskStatus::CompetingEvent => d2 += 1,
                CompetingRiskStatus::Censored => c_j += 1,
            }
            pos += 1;
        }
        n_primary_tot += d1;
        n_competing_tot += d2;
        n_censored_tot += c_j;
        times.push(t);
        at_risk.push(n_j);
        d1_vec.push(d1);
        d2_vec.push(d2);
        cens_vec.push(c_j);
    }

    let m = times.len();
    let mut s_prev = vec![1.0_f64; m];
    let mut s_cur = vec![1.0_f64; m];
    let mut i1 = vec![0.0_f64; m];
    let mut i2 = vec![0.0_f64; m];

    let mut surv = 1.0_f64;
    let mut cif1 = 0.0_f64;
    let mut cif2 = 0.0_f64;
    for j in 0..m {
        s_prev[j] = surv;
        let nj = at_risk[j] as f64;
        let d1j = d1_vec[j] as f64;
        let d2j = d2_vec[j] as f64;
        let dj = d1j + d2j;
        if nj > 0.0 {
            cif1 = (cif1 + surv * d1j / nj).clamp(0.0, 1.0);
            cif2 = (cif2 + surv * d2j / nj).clamp(0.0, 1.0);
            surv = (surv * (1.0 - dj / nj)).clamp(0.0, 1.0);
        }
        s_cur[j] = surv;
        i1[j] = cif1;
        i2[j] = cif2;
    }

    // Marubini–Valsecchi (1995) / Aalen–Johansen exact Greenwood variance at each time step.
    let mut steps = Vec::with_capacity(m);
    for k in 0..m {
        let mut var1 = 0.0_f64;
        let mut var2 = 0.0_f64;
        for j in 0..=k {
            let nj = at_risk[j] as f64;
            let d1j = d1_vec[j] as f64;
            let d2j = d2_vec[j] as f64;
            let dj = d1j + d2j;
            let sp = s_prev[j];
            if nj <= 0.0 || dj <= 0.0 {
                continue;
            }
            let diff1 = i1[k] - i1[j];
            let diff2 = i2[k] - i2[j];
            let greenwood_factor = if nj > dj { dj / (nj * (nj - dj)) } else { 0.0 };

            let t1_1 = diff1 * diff1 * greenwood_factor;
            let t2_1 = sp * sp * d1j * (nj - d1j) / (nj * nj * nj);
            let t3_1 = -2.0 * diff1 * sp * d1j / (nj * nj);
            var1 += t1_1 + t2_1 + t3_1;

            let t1_2 = diff2 * diff2 * greenwood_factor;
            let t2_2 = sp * sp * d2j * (nj - d2j) / (nj * nj * nj);
            let t3_2 = -2.0 * diff2 * sp * d2j / (nj * nj);
            var2 += t1_2 + t2_2 + t3_2;
        }
        let se1 = var1.max(0.0).sqrt();
        let se2 = var2.max(0.0).sqrt();
        let (ci1_lo, ci1_hi) = cif_pointwise_ci(i1[k], se1, z, log_log_ci);
        let (ci2_lo, ci2_hi) = cif_pointwise_ci(i2[k], se2, z, log_log_ci);

        steps.push(CifStep {
            time: times[k],
            n_at_risk: at_risk[k],
            n_primary: d1_vec[k],
            n_competing: d2_vec[k],
            n_censored: cens_vec[k],
            overall_survival: s_cur[k],
            cif_primary: i1[k],
            se_primary: se1,
            ci_primary_lower: ci1_lo,
            ci_primary_upper: ci1_hi,
            cif_competing: i2[k],
            se_competing: se2,
            ci_competing_lower: ci2_lo,
            ci_competing_upper: ci2_hi,
        });
    }

    let (final_cif_primary, final_ci_primary, final_cif_competing, final_ci_competing) =
        steps.last().map_or((0.0, (0.0, 0.0), 0.0, (0.0, 0.0)), |s| {
            (s.cif_primary, (s.ci_primary_lower, s.ci_primary_upper), s.cif_competing, (s.ci_competing_lower, s.ci_competing_upper))
        });

    Ok(CifCohortResult {
        label: label.into(),
        n_total,
        n_primary: n_primary_tot,
        n_competing: n_competing_tot,
        n_censored: n_censored_tot,
        final_cif_primary,
        final_ci_primary,
        final_cif_competing,
        final_ci_competing,
        steps,
    })
}

/// Compute Gray's (1988) *K*-sample weighted log-rank test comparing cumulative incidence functions
/// for `target_cause` (`PrimaryEvent` or `CompetingEvent`) across $K \ge 2$ cohorts.
///
/// Implements the exact subdistribution hazard score vector and martingale variance–covariance
/// matrix of Gray (1988, Section 2) with weight $w(t) = (1 - \hat{F}_{\text{pool}}(t^-))^\rho$
/// ($\rho \ge 0$). Reduces identically to the Mantel–Cox log-rank test when no competing events
/// occur and $\rho = 0$.
///
struct GrayTrajectory {
    k: usize,
    m: usize,
    y_mat: Vec<Vec<f64>>,
    d2_mat: Vec<Vec<f64>>,
    f1_cur_mat: Vec<Vec<f64>>,
    f2_prev_mat: Vec<Vec<f64>>,
    d1_pool: Vec<f64>,
    f1_pool_prev: Vec<f64>,
    r_mat: Vec<Vec<f64>>,
    r_pool: Vec<f64>,
    w: Vec<f64>,
    d_lam: Vec<f64>,
    score: Vec<f64>,
}

fn validate_and_sort_gray_groups(
    groups: &[(String, Vec<CompetingRiskObservation>)],
) -> Result<(Vec<Vec<CompetingRiskObservation>>, Vec<f64>), StatsError> {
    let mut ev_times: Vec<f64> = Vec::new();
    let mut sorted_groups: Vec<Vec<CompetingRiskObservation>> = Vec::with_capacity(groups.len());
    for (_, obs_list) in groups {
        if obs_list.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        let mut clean = Vec::with_capacity(obs_list.len());
        for o in obs_list {
            if !o.time.is_finite() || o.time < 0.0 {
                return Err(StatsError::InvalidParameter { name: "survival_time", value: o.time });
            }
            let t = normalize_time(o.time);
            if !matches!(o.status, CompetingRiskStatus::Censored) {
                ev_times.push(t);
            }
            clean.push(CompetingRiskObservation { time: t, status: o.status });
        }
        clean.sort_by(|a, b| a.time.total_cmp(&b.time));
        sorted_groups.push(clean);
    }
    ev_times.sort_by(f64::total_cmp);
    ev_times.dedup();
    Ok((sorted_groups, ev_times))
}

fn build_gray_trajectory(
    sorted_groups: &[Vec<CompetingRiskObservation>],
    ev_times: &[f64],
    target_cause: CompetingRiskStatus,
    rho: f64,
) -> GrayTrajectory {
    let k = sorted_groups.len();
    let m = ev_times.len();
    let mut y_mat = vec![vec![0.0_f64; m]; k];
    let mut d1_mat = vec![vec![0.0_f64; m]; k];
    let mut d2_mat = vec![vec![0.0_f64; m]; k];
    let mut s_prev_mat = vec![vec![1.0_f64; m]; k];
    let mut f1_prev_mat = vec![vec![0.0_f64; m]; k];
    let mut f1_cur_mat = vec![vec![0.0_f64; m]; k];
    let mut f2_prev_mat = vec![vec![0.0_f64; m]; k];

    for g in 0..k {
        let obs = &sorted_groups[g];
        let n_g = obs.len();
        let mut ptr = 0_usize;
        let mut s_val = 1.0_f64;
        let mut f1_val = 0.0_f64;
        let mut f2_val = 0.0_f64;
        for (j, &tj) in ev_times.iter().enumerate() {
            while ptr < n_g && obs[ptr].time < tj {
                ptr += 1;
            }
            let at_risk = (n_g - ptr) as f64;
            let mut d1 = 0.0_f64;
            let mut d2 = 0.0_f64;
            let mut scan = ptr;
            while scan < n_g && obs[scan].time == tj {
                if obs[scan].status == target_cause {
                    d1 += 1.0;
                } else if !matches!(obs[scan].status, CompetingRiskStatus::Censored) {
                    d2 += 1.0;
                }
                scan += 1;
            }
            y_mat[g][j] = at_risk;
            d1_mat[g][j] = d1;
            d2_mat[g][j] = d2;
            s_prev_mat[g][j] = s_val;
            f1_prev_mat[g][j] = f1_val;
            f2_prev_mat[g][j] = f2_val;
            if at_risk > 0.0 {
                f1_val = (f1_val + s_val * d1 / at_risk).clamp(0.0, 1.0);
                f2_val = (f2_val + s_val * d2 / at_risk).clamp(0.0, 1.0);
                s_val = (s_val * (1.0 - (d1 + d2) / at_risk)).clamp(0.0, 1.0);
            }
            f1_cur_mat[g][j] = f1_val;
        }
    }

    let mut d1_pool = vec![0.0_f64; m];
    let mut f1_pool_prev = vec![0.0_f64; m];
    let mut sp = 1.0_f64;
    let mut fp = 0.0_f64;
    let mut r_mat = vec![vec![0.0_f64; m]; k];
    let mut r_pool = vec![0.0_f64; m];
    for j in 0..m {
        let mut yp = 0.0_f64;
        let mut d1p = 0.0_f64;
        let mut d2p = 0.0_f64;
        let mut rp = 0.0_f64;
        for g in 0..k {
            yp += y_mat[g][j];
            d1p += d1_mat[g][j];
            d2p += d2_mat[g][j];
            if y_mat[g][j] > 0.0 && s_prev_mat[g][j] > 0.0 {
                let rg = y_mat[g][j] * (1.0 - f1_prev_mat[g][j]) / s_prev_mat[g][j];
                r_mat[g][j] = rg;
                rp += rg;
            }
        }
        d1_pool[j] = d1p;
        f1_pool_prev[j] = fp;
        r_pool[j] = rp;
        if yp > 0.0 {
            fp = (fp + sp * d1p / yp).clamp(0.0, 1.0);
            sp = (sp * (1.0 - (d1p + d2p) / yp)).clamp(0.0, 1.0);
        }
    }

    let w: Vec<f64> = f1_pool_prev.iter().map(|&f| (1.0 - f).max(0.0).powf(rho)).collect();
    let d_lam: Vec<f64> = (0..m).map(|j| if r_pool[j] > 0.0 { d1_pool[j] / r_pool[j] } else { 0.0 }).collect();

    let mut score = vec![0.0_f64; k];
    for g in 0..k {
        let mut acc = 0.0_f64;
        for j in 0..m {
            if r_pool[j] > 0.0 && d1_pool[j] > 0.0 {
                acc += w[j] * (d1_mat[g][j] - r_mat[g][j] * d_lam[j]);
            }
        }
        score[g] = acc;
    }

    GrayTrajectory { k, m, y_mat, d2_mat, f1_cur_mat, f2_prev_mat, d1_pool, f1_pool_prev, r_mat, r_pool, w, d_lam, score }
}

fn gray_martingale_covariance(t: &GrayTrajectory) -> Vec<Vec<f64>> {
    let (k, m) = (t.k, t.m);
    let mut cov = vec![vec![0.0_f64; k]; k];
    for l in 0..k {
        let mut d_mat = vec![vec![0.0_f64; m]; k];
        let mut running = vec![0.0_f64; k];
        for j in (0..m).rev() {
            for i in 0..k {
                d_mat[i][j] = running[i];
            }
            let one_minus_fp = 1.0 - t.f1_pool_prev[j];
            if t.r_pool[j] > 0.0 && t.d1_pool[j] > 0.0 && one_minus_fp > 0.0 {
                let factor_u = t.w[j] * t.r_mat[l][j] * t.d_lam[j] / one_minus_fp;
                for i in 0..k {
                    let delta_il = if i == l { 1.0 } else { 0.0 };
                    running[i] += factor_u * (delta_il - t.r_mat[i][j] / t.r_pool[j]);
                }
            }
        }

        let mut a_vec = vec![0.0_f64; k];
        let mut b_vec = vec![0.0_f64; k];
        for j in 0..m {
            let y_lj = t.y_mat[l][j];
            if y_lj <= 0.0 {
                continue;
            }
            let f2_lj = t.f2_prev_mat[l][j];
            let one_minus_f1_lj = 1.0 - t.f1_cur_mat[l][j];
            for i in 0..k {
                let delta_il = if i == l { 1.0 } else { 0.0 };
                let direct = if t.r_pool[j] > 0.0 { t.w[j] * (delta_il - t.r_mat[i][j] / t.r_pool[j]) } else { 0.0 };
                a_vec[i] = direct - (f2_lj / y_lj) * d_mat[i][j];
                b_vec[i] = -(one_minus_f1_lj / y_lj) * d_mat[i][j];
            }
            let v1 = if t.r_pool[j] > 1.0 && t.d1_pool[j] > 0.0 {
                t.r_mat[l][j] * t.d_lam[j] * ((t.r_pool[j] - t.d1_pool[j]) / (t.r_pool[j] - 1.0)).max(0.0)
            } else if t.r_pool[j] > 0.0 && t.d1_pool[j] > 0.0 {
                t.r_mat[l][j] * t.d_lam[j]
            } else {
                0.0
            };
            let d2_lj = t.d2_mat[l][j];
            let v2 = if y_lj > 1.0 && d2_lj > 0.0 { d2_lj * ((y_lj - d2_lj) / (y_lj - 1.0)).max(0.0) } else { d2_lj };

            for r_i in 0..k {
                for c_i in 0..k {
                    cov[r_i][c_i] += v1 * a_vec[r_i] * a_vec[c_i] + v2 * b_vec[r_i] * b_vec[c_i];
                }
            }
        }
    }

    for r_i in 0..k {
        for c_i in (r_i + 1)..k {
            let avg = 0.5 * (cov[r_i][c_i] + cov[c_i][r_i]);
            cov[r_i][c_i] = avg;
            cov[c_i][r_i] = avg;
        }
    }
    cov
}

fn gray_quadratic_form(score: &[f64], cov: &[Vec<f64>]) -> Result<(f64, f64, f64), StatsError> {
    let k = score.len();
    let cov_mat = DenseMatrix::from_fn(k, k, |r_i, c_i| cov[r_i][c_i])?;
    let eig = sym_eigen(&cov_mat)?;
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    if max_ev <= 0.0 || !max_ev.is_finite() {
        return Ok((f64::NAN, f64::NAN, f64::NAN));
    }
    let tol = 1e-10 * max_ev;
    let mut stat = 0.0_f64;
    let mut rank = 0_usize;
    for (col_idx, &ev) in eig.eigenvalues.iter().enumerate() {
        if ev > tol {
            let mut proj = 0.0_f64;
            for (row_idx, &sc) in score.iter().enumerate() {
                proj += eig.eigenvectors.get(row_idx, col_idx).unwrap_or(0.0) * sc;
            }
            stat += (proj * proj) / ev;
            rank += 1;
        }
    }
    if rank == 0 {
        return Ok((f64::NAN, f64::NAN, f64::NAN));
    }
    let df_f = rank as f64;
    Ok((stat.max(0.0), df_f, chi2_sf(stat.max(0.0), df_f)?))
}

/// # Errors
/// Returns [`StatsError`] when fewer than 2 groups are provided, any observation has an invalid
/// time, `rho` is negative or non-finite, or `target_cause` is `Censored`.
pub fn gray_k_sample_test(
    groups: &[(String, Vec<CompetingRiskObservation>)],
    target_cause: CompetingRiskStatus,
    rho: f64,
) -> Result<GrayTestResult, StatsError> {
    if matches!(target_cause, CompetingRiskStatus::Censored) {
        return Err(StatsError::InvalidParameter { name: "target_cause", value: 0.0 });
    }
    if !rho.is_finite() || rho < 0.0 {
        return Err(StatsError::InvalidParameter { name: "gray_rho", value: rho });
    }
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Gray's K-sample competing-risks test", required: 2, actual: groups.len() });
    }

    let cause_label = match target_cause {
        CompetingRiskStatus::PrimaryEvent => String::from("Primary event"),
        CompetingRiskStatus::CompetingEvent => String::from("Competing event"),
        CompetingRiskStatus::Censored => String::from("Censored"),
    };

    let (sorted_groups, ev_times) = validate_and_sort_gray_groups(groups)?;
    if ev_times.is_empty() {
        return Ok(GrayTestResult {
            cause_label,
            rho,
            chi_square: f64::NAN,
            df: f64::NAN,
            p_value: f64::NAN,
            group_scores: groups.iter().map(|(lbl, _)| (lbl.clone(), 0.0)).collect(),
        });
    }

    let traj = build_gray_trajectory(&sorted_groups, &ev_times, target_cause, rho);
    let cov = gray_martingale_covariance(&traj);
    let (chi_square, df, p_value) = gray_quadratic_form(&traj.score, &cov)?;
    let group_scores = groups.iter().enumerate().map(|(i, (lbl, _))| (lbl.clone(), traj.score[i])).collect();

    Ok(GrayTestResult { cause_label, rho, chi_square, df, p_value, group_scores })
}

fn classify_code(code: f64, spec: &CompetingRisksSpec) -> Option<CompetingRiskStatus> {
    const TOL: f64 = 1e-9;
    if !code.is_finite() {
        return None;
    }
    if (code - spec.censored_code).abs() <= TOL {
        Some(CompetingRiskStatus::Censored)
    } else if (code - spec.primary_event_code).abs() <= TOL {
        Some(CompetingRiskStatus::PrimaryEvent)
    } else if let Some(comp) = spec.competing_event_code {
        ((code - comp).abs() <= TOL).then_some(CompetingRiskStatus::CompetingEvent)
    } else {
        Some(CompetingRiskStatus::CompetingEvent)
    }
}

fn mv_column_has_numbers(col: &VariableColumn) -> bool {
    matches!(col.role, VariableRole::Continuous | VariableRole::Calculated) || col.cells.iter().any(|c| c.active_number().is_some())
}

fn mv_cell_group_token(col: &VariableColumn, row: usize) -> Option<String> {
    let cell = col.cells.get(row)?;
    if cell.excluded || cell.is_empty() {
        return None;
    }
    match &cell.value {
        curvebook_doc::CellValue::Text(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_owned())
        }
        curvebook_doc::CellValue::Number(v) if v.is_finite() => {
            if v.fract().abs() < 1e-12 {
                Some(format!("{v:.0}"))
            } else {
                Some(format!("{v}"))
            }
        }
        _ => None,
    }
}

type CrGroupVec = Vec<(String, Vec<CompetingRiskObservation>)>;

fn extract_cr_survival_groups(
    table: &curvebook_doc::SurvivalTable,
    selected_data_sets: &[usize],
    cr_spec: &CompetingRisksSpec,
) -> Result<(CrGroupVec, usize), StatsError> {
    let selected = resolve_selected_indices(selected_data_sets, table.cohorts.len());
    let mut groups = Vec::new();
    let mut omitted_rows = 0_usize;
    for &c_idx in &selected {
        let cohort = &table.cohorts[c_idx];
        let mut obs = Vec::new();
        for (row, subs) in cohort.cells.iter().enumerate() {
            let Some(scell) = subs.first() else { continue };
            if scell.excluded || scell.is_empty() {
                continue;
            }
            let Some(tcell) = table.times.get(row) else { continue };
            if tcell.excluded || tcell.is_empty() {
                omitted_rows += 1;
                continue;
            }
            let (Some(t), Some(code)) = (tcell.active_number(), scell.active_number()) else {
                omitted_rows += 1;
                continue;
            };
            if !t.is_finite() || t < 0.0 {
                return Err(StatsError::InvalidParameter { name: "survival_time", value: t });
            }
            if let Some(status) = classify_code(code, cr_spec) {
                obs.push(CompetingRiskObservation { time: normalize_time(t), status });
            } else {
                omitted_rows += 1;
            }
        }
        if !obs.is_empty() {
            let lbl = if cohort.title.trim().is_empty() { format!("Cohort {}", c_idx + 1) } else { cohort.title.clone() };
            groups.push((lbl, obs));
        }
    }
    Ok((groups, omitted_rows))
}

fn extract_cr_mv_groups(mv: &curvebook_doc::MultipleVariablesTable, cr_spec: &CompetingRisksSpec) -> Result<(CrGroupVec, usize), StatsError> {
    let n_vars = mv.variables.len();
    if n_vars < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "competing-risks columns (time + status)", required: 2, actual: n_vars });
    }
    let numeric_cols: Vec<usize> = (0..n_vars).filter(|&i| mv_column_has_numbers(&mv.variables[i])).collect();
    let time_col = match cr_spec.time_column {
        Some(idx) if idx < n_vars => idx,
        Some(idx) => return Err(StatsError::InvalidParameter { name: "time_column", value: idx as f64 }),
        None => *numeric_cols.first().ok_or(StatsError::EmptyInput)?,
    };
    let status_col = match cr_spec.status_column {
        Some(idx) if idx < n_vars && idx != time_col => idx,
        Some(idx) => return Err(StatsError::InvalidParameter { name: "status_column", value: idx as f64 }),
        None => *numeric_cols.iter().find(|&&i| i != time_col).ok_or(StatsError::InvalidParameter { name: "status_column", value: 1.0 })?,
    };
    let group_col = match cr_spec.group_column {
        Some(idx) if idx < n_vars && idx != time_col && idx != status_col => Some(idx),
        Some(idx) => return Err(StatsError::InvalidParameter { name: "group_column", value: idx as f64 }),
        None => {
            mv.variables.iter().enumerate().find(|&(i, v)| i != time_col && i != status_col && v.role == VariableRole::Categorical).map(|(i, _)| i)
        }
    };

    let mut map: BTreeMap<String, Vec<CompetingRiskObservation>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut omitted_rows = 0_usize;
    for row in 0..mv.row_count() {
        let t_opt = mv.variables[time_col].cells.get(row).and_then(|c| c.active_number());
        let s_opt = mv.variables[status_col].cells.get(row).and_then(|c| c.active_number());
        let g_opt = match group_col {
            Some(gc) => mv_cell_group_token(&mv.variables[gc], row),
            None => Some(String::from("All subjects")),
        };
        if let (Some(t), Some(code), Some(glbl)) = (t_opt, s_opt, g_opt) {
            if !t.is_finite() || t < 0.0 {
                return Err(StatsError::InvalidParameter { name: "survival_time", value: t });
            }
            if let Some(status) = classify_code(code, cr_spec) {
                let entry = map.entry(glbl.clone()).or_insert_with(|| {
                    order.push(glbl);
                    Vec::new()
                });
                entry.push(CompetingRiskObservation { time: normalize_time(t), status });
            } else {
                omitted_rows += 1;
            }
        } else if mv.variables[time_col].cells.get(row).is_some_and(|c| !c.is_empty())
            || mv.variables[status_col].cells.get(row).is_some_and(|c| !c.is_empty())
        {
            omitted_rows += 1;
        }
    }
    if let Some(gc) = group_col {
        let col_order: Vec<String> = mv.variables[gc].ordered_levels().into_iter().map(|s| s.level).collect();
        let mut sorted_labels = Vec::with_capacity(order.len());
        for lbl in col_order {
            if map.contains_key(&lbl) && !sorted_labels.contains(&lbl) {
                sorted_labels.push(lbl);
            }
        }
        for lbl in order {
            if !sorted_labels.contains(&lbl) {
                sorted_labels.push(lbl);
            }
        }
        order = sorted_labels;
    }
    let groups = order.into_iter().filter_map(|lbl| map.remove(&lbl).map(|obs| (lbl, obs))).collect();
    Ok((groups, omitted_rows))
}

fn append_gray_results(out: &mut ResultSheet, gp: &GrayTestResult, gc: &GrayTestResult, spec: &AnalysisSpec) {
    let alpha = spec.confidence_level.alpha();
    let p_fmt = spec.p_value_format;
    if gp.chi_square.is_finite() {
        push_scalar(out, "Gray test chi-square", None, gp.chi_square);
        push_scalar(out, "Gray test df", None, gp.df);
        push_scalar(out, "Gray test P", None, gp.p_value);
    }
    if gc.chi_square.is_finite() {
        push_scalar(out, "Gray test chi-square (competing)", None, gc.chi_square);
        push_scalar(out, "Gray test df (competing)", None, gc.df);
        push_scalar(out, "Gray test P (competing)", None, gc.p_value);
    }
    let mut gray_tab = TabBuilder::new("cr-gray-test", "Gray's K-sample test", ResultTabKind::MainSummary).headers([
        "Weight rho",
        "Chi-square",
        "df",
        "P value",
        "Interpretation",
    ]);
    for gres in [gp, gc] {
        let interp = if !gres.p_value.is_finite() {
            "Non-estimable (zero events or rank)"
        } else if gres.p_value < alpha {
            "CIFs differ across groups"
        } else {
            "No significant CIF difference"
        };
        gray_tab.push_row(
            gres.cause_label.clone(),
            [
                num_cell(gres.rho),
                opt_num_cell(gres.chi_square.is_finite().then_some(gres.chi_square)),
                opt_num_cell(gres.df.is_finite().then_some(gres.df)),
                p_cell_at_alpha(gres.p_value, p_fmt, alpha),
                text_cell(interp),
            ],
        );
    }
    out.tabs.push(gray_tab.build());

    out.checklist.push(checklist_item(
        "Gray's K-sample test (primary event CIF)",
        ChecklistStatus::Info,
        Some(if gp.chi_square.is_finite() {
            let p_str = if gp.p_value < 1e-4 { format!("{:.3e}", gp.p_value) } else { format!("{:.4}", gp.p_value) };
            format!("chi2 = {:.4}, df = {:.0}, P = {p_str}", gp.chi_square, gp.df)
        } else {
            String::from("Non-estimable")
        }),
        "Gray's (1988) subdistribution test compares cumulative incidence directly across cohorts without censoring competing events as independent.",
    ));
}

/// Execute competing-risks cumulative incidence analysis against a `Survival` or `MultipleVariables`
/// [`DataSheet`].
pub fn run_competing_risks_analysis(sheet: &DataSheet, spec: &AnalysisSpec, cr_spec: &CompetingRisksSpec) -> Result<ResultSheet, StatsError> {
    if !cr_spec.censored_code.is_finite() || !cr_spec.primary_event_code.is_finite() {
        return Err(StatsError::InvalidParameter { name: "event_or_censored_code", value: cr_spec.primary_event_code });
    }
    if (cr_spec.censored_code - cr_spec.primary_event_code).abs() <= 1e-9 {
        return Err(StatsError::InvalidParameter { name: "primary_event_code", value: cr_spec.primary_event_code });
    }
    if let Some(cc) = cr_spec.competing_event_code
        && (!cc.is_finite() || (cc - cr_spec.censored_code).abs() <= 1e-9 || (cc - cr_spec.primary_event_code).abs() <= 1e-9)
    {
        return Err(StatsError::InvalidParameter { name: "competing_event_code", value: cc });
    }
    if !cr_spec.gray_rho.is_finite() || cr_spec.gray_rho < 0.0 {
        return Err(StatsError::InvalidParameter { name: "gray_rho", value: cr_spec.gray_rho });
    }

    let (groups, omitted_rows) = match &sheet.archetype {
        TableArchetype::Survival(table) => extract_cr_survival_groups(table, &spec.selected_data_sets, cr_spec)?,
        TableArchetype::MultipleVariables(mv) => extract_cr_mv_groups(mv, cr_spec)?,
        other => {
            return Err(StatsError::IncompatibleTable { analysis: "competing-risks cumulative incidence", actual: other.kind() });
        }
    };

    if groups.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let confidence = spec.confidence_level;
    let mut cohorts = Vec::with_capacity(groups.len());
    for (lbl, obs) in &groups {
        cohorts.push(aalen_johansen_cif(lbl.clone(), obs, confidence, cr_spec.log_log_ci)?);
    }

    let (gray_primary, gray_competing) = if groups.len() >= 2 {
        (
            Some(gray_k_sample_test(&groups, CompetingRiskStatus::PrimaryEvent, cr_spec.gray_rho)?),
            Some(gray_k_sample_test(&groups, CompetingRiskStatus::CompetingEvent, cr_spec.gray_rho)?),
        )
    } else {
        (None, None)
    };

    let mut out = init_result_sheet(spec)?;
    let total_subjects: usize = cohorts.iter().map(|c| c.n_total).sum();
    push_scalar(&mut out, "Total subjects", None, total_subjects as f64);
    push_scalar(&mut out, "Groups", None, cohorts.len() as f64);

    // Tab 1: Cohort CIF summary.
    let ci_pct = confidence.percent();
    let mut summary_tab = TabBuilder::new("cr-summary", "Competing-risks CIF summary", ResultTabKind::MainSummary).headers([
        String::from("N"),
        String::from("Primary events"),
        String::from("Competing events"),
        String::from("Censored"),
        String::from("Final primary CIF"),
        format!("Primary {ci_pct:.0}% CI"),
        String::from("Final competing CIF"),
        format!("Competing {ci_pct:.0}% CI"),
    ]);
    for (idx, c) in cohorts.iter().enumerate() {
        push_scalar(&mut out, "Final CIF (primary)", Some(idx), c.final_cif_primary);
        push_scalar(&mut out, "Final CIF (competing)", Some(idx), c.final_cif_competing);
        push_scalar(&mut out, "Primary events", Some(idx), c.n_primary as f64);
        push_scalar(&mut out, "Competing events", Some(idx), c.n_competing as f64);
        push_scalar(&mut out, "Censored", Some(idx), c.n_censored as f64);
        summary_tab.push_row(
            c.label.clone(),
            [
                num_cell(c.n_total as f64),
                num_cell(c.n_primary as f64),
                num_cell(c.n_competing as f64),
                num_cell(c.n_censored as f64),
                num_cell(c.final_cif_primary),
                ci_cell(c.final_ci_primary.0, c.final_ci_primary.1),
                num_cell(c.final_cif_competing),
                ci_cell(c.final_ci_competing.0, c.final_ci_competing.1),
            ],
        );
    }
    let mut summary_tab = summary_tab.build();
    summary_tab.footnotes.push(format!(
        "Aalen-Johansen nonparametric CIF with Marubini-Valsecchi / Greenwood SE and {} {}% pointwise confidence intervals.",
        if cr_spec.log_log_ci { "complementary log-log" } else { "symmetric linear" },
        ci_pct
    ));
    out.tabs.push(summary_tab);

    // Tab 2: Gray's K-sample test table (when >= 2 groups).
    if let (Some(gp), Some(gc)) = (&gray_primary, &gray_competing) {
        append_gray_results(&mut out, gp, gc, spec);
    }

    // Per-cohort CIF curve tabs + FittedCurveOverlay for primary CIF.
    for (idx, c) in cohorts.iter().enumerate() {
        let mut tab = TabBuilder::new(format!("cr-cif-{idx}"), format!("CIF schedule: {}", c.label), ResultTabKind::CurveCoordinates).headers([
            String::from("Time"),
            String::from("At Risk"),
            String::from("Primary Events"),
            String::from("Competing Events"),
            String::from("Censored"),
            String::from("Event-Free S(t)"),
            String::from("Primary CIF"),
            String::from("Primary SE"),
            format!("Primary {ci_pct:.0}% CI"),
            String::from("Competing CIF"),
            String::from("Competing SE"),
            format!("Competing {ci_pct:.0}% CI"),
        ]);
        let mut pts = Vec::with_capacity(c.steps.len() + 1);
        pts.push(CurveSamplePoint { x: 0.0, y: 0.0, ci_lower: Some(0.0), ci_upper: Some(0.0), pred_lower: None, pred_upper: None });
        for (s_idx, s) in c.steps.iter().enumerate() {
            tab.push_row(
                format!("Step {}", s_idx + 1),
                [
                    num_cell(s.time),
                    num_cell(s.n_at_risk as f64),
                    num_cell(s.n_primary as f64),
                    num_cell(s.n_competing as f64),
                    num_cell(s.n_censored as f64),
                    num_cell(s.overall_survival),
                    num_cell(s.cif_primary),
                    num_cell(s.se_primary),
                    ci_cell(s.ci_primary_lower, s.ci_primary_upper),
                    num_cell(s.cif_competing),
                    num_cell(s.se_competing),
                    ci_cell(s.ci_competing_lower, s.ci_competing_upper),
                ],
            );
            pts.push(CurveSamplePoint {
                x: s.time,
                y: s.cif_primary,
                ci_lower: Some(s.ci_primary_lower),
                ci_upper: Some(s.ci_primary_upper),
                pred_lower: None,
                pred_upper: None,
            });
        }
        out.tabs.push(tab.build());
        out.fitted_curves.push(FittedCurveOverlay {
            data_set_index: idx,
            equation_id: String::from("aalen_johansen_cif"),
            converged: true,
            ambiguous: false,
            points: pts,
        });
    }

    append_listwise_filtered_checklist_and_footnote(&mut out, "Competing-risks CIF", omitted_rows, total_subjects);

    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}
