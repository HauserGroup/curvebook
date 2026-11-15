//! Contingency table, categorical proportion, agreement, and goodness-of-fit analyses.
//!
//! Includes Pearson chi-square (with Yates continuity option), likelihood-ratio `G^2`,
//! Fisher's exact test (`2x2` + `RxC` Freeman–Halton / deterministic Monte Carlo), Cochran–Armitage
//! trend test, McNemar paired test, Wilson & Clopper–Pearson binomial CIs, Baptista–Pike OR CI,
//! Koopman score RR CI, Newcombe risk-difference CI, Cohen's kappa (unweighted/weighted),
//! Fleiss' kappa, and phi / Cramér's `V` with exact noncentral chi-square CIs.

pub use lumencraft_doc::KappaWeighting;
use lumencraft_doc::{
    AnalysisSpec, ChecklistStatus, ContingencyMethod, ContingencySpec, DataSheet, PValueFormat, ResultCell, ResultSheet, ResultTabKind,
    SubcolumnMode, TableArchetype,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{chi2_sf, noncentral_chi2_ci, norm_cdf, norm_sf, norm_two_sided_isf};
use lumencraft_math::rng::DeterministicRng;
use lumencraft_math::roots::{RootOptions, brent_root};
use lumencraft_math::special::{ln_choose, ln_factorial, reg_beta_inv};
use serde::{Deserialize, Serialize};

use crate::common::{build_tab, checklist_item, ci_cell, init_result_sheet, num_cell, opt_ci_cell, opt_num_cell, p_cell, push_scalar};
use crate::error::StatsError;

// ============================================================================
// Typed Outputs
// ============================================================================

/// Contingency association and test output across `R x C` count tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContingencyResult {
    pub n_rows: usize,
    pub n_cols: usize,
    pub total_count: u64,
    pub chi_square: f64,
    pub df: f64,
    pub chi_square_p: f64,
    pub g_squared: f64,
    pub g_squared_p: f64,
    pub fisher_exact_p: Option<f64>,
    pub mcnemar_chi2: Option<f64>,
    pub mcnemar_exact_p: Option<f64>,
    pub cochran_armitage_z: Option<f64>,
    pub cochran_armitage_p: Option<f64>,
    pub phi: Option<f64>,
    pub cramers_v: f64,
    pub cramers_v_ci: Option<(f64, f64)>,
    pub min_expected_count: f64,
    pub cells_lt5_fraction: f64,
}

/// Altman (1998) direction for Number Needed to Treat (`NNTB`, `NNTH`, or `Infinite` when `RD = 0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NntKind {
    Benefit,
    Harm,
    #[default]
    Infinite,
}

/// Complete `2 x 2` risk, odds, and diagnostic accuracy ratios with score/exact CIs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TwoByTwoRatiosResult {
    pub odds_ratio: f64,
    pub or_baptista_pike_ci: Option<(f64, f64)>,
    pub relative_risk: f64,
    pub rr_koopman_ci: Option<(f64, f64)>,
    pub risk_difference: f64,
    pub rd_newcombe_ci: Option<(f64, f64)>,
    pub nnt: Option<f64>,
    #[serde(default)]
    pub nnt_kind: NntKind,
    #[serde(default)]
    pub nnt_value: Option<f64>,
    #[serde(default)]
    pub nnt_point_label: String,
    #[serde(default)]
    pub nnt_ci_label: String,
    #[serde(default)]
    pub nnt_ci_bounds: Option<(f64, f64)>,
    pub sensitivity: f64,
    pub sensitivity_wilson_ci: (f64, f64),
    pub specificity: f64,
    pub specificity_wilson_ci: (f64, f64),
    pub ppv: f64,
    pub ppv_wilson_ci: (f64, f64),
    pub npv: f64,
    pub npv_wilson_ci: (f64, f64),
}

/// Detailed McNemar paired-table result including discordant-pair odds ratio and exact Clopper–Pearson CI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McNemarResult {
    pub b: u64,
    pub c: u64,
    pub chi_square_yates: f64,
    pub exact_p_value: f64,
    pub discordant_odds_ratio: Option<f64>,
    pub discordant_or_label: String,
    pub discordant_or_ci: Option<(f64, f64)>,
    pub discordant_p_ci: (f64, f64),
}

/// Exact one-sample binomial test output with Sterne & doubled-tail two-sided *p*-values and
/// Clopper–Pearson, Wilson, and Agresti–Coull confidence intervals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExactBinomialResult {
    pub successes: u64,
    pub total: u64,
    pub null_proportion: f64,
    pub proportion: f64,
    pub p_value_lower: f64,
    pub p_value_upper: f64,
    pub p_value_sterne: f64,
    pub p_value_doubled_tail: f64,
    pub clopper_pearson_ci: (f64, f64),
    pub wilson_ci: (f64, f64),
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

/// Complete output of Barnard's exact unconditional `2 x 2` binomial test (`[[a, b], [c, d]]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarnardExactResult {
    pub statistic: f64,
    pub p_value: f64,
    pub nuisance_pi: f64,
    pub ordering: BarnardOrdering,
    pub used_asymptotic_fallback: bool,
}

/// Inter-rater agreement metrics (Cohen's kappa or Fleiss' kappa).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KappaResult {
    pub kappa: f64,
    pub observed_agreement: f64,
    pub expected_agreement: f64,
    pub se: f64,
    pub ci: (f64, f64),
    pub z_statistic: f64,
    pub p_value: f64,
}

// ============================================================================
// Binomial CIs (Wilson Score & Clopper–Pearson Exact)
// ============================================================================

/// Wilson (1927) score confidence interval for binomial proportion `k / n`.
#[must_use]
pub fn wilson_score_ci(k: u64, n: u64, confidence: ConfidenceLevel) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let nf = n as f64;
    let kf = (k.min(n)) as f64;
    let p = kf / nf;
    let z = norm_two_sided_isf(confidence.alpha()).unwrap_or(1.959_963_984_540_054);
    let z2 = z * z;
    let denom = 1.0 + z2 / nf;
    let center = (p + z2 / (2.0 * nf)) / denom;
    let half = (z / denom) * ((p * (1.0 - p) / nf) + z2 / (4.0 * nf * nf)).sqrt();
    ((center - half).clamp(0.0, 1.0), (center + half).clamp(0.0, 1.0))
}

/// Clopper–Pearson (1934) exact binomial confidence interval via inverse Beta quantiles.
pub fn clopper_pearson_ci(k: u64, n: u64, confidence: ConfidenceLevel) -> Result<(f64, f64), StatsError> {
    if n == 0 {
        return Ok((0.0, 1.0));
    }
    if k > n {
        return Err(StatsError::InvalidParameter { name: "binomial_successes", value: k as f64 });
    }
    let alpha = confidence.alpha();
    let half = 0.5 * alpha;
    let lo = if k == 0 { 0.0 } else { reg_beta_inv(k as f64, (n - k + 1) as f64, half)? };
    let hi = if k >= n { 1.0 } else { reg_beta_inv((k + 1) as f64, (n - k) as f64, 1.0 - half)? };
    Ok((lo.clamp(0.0, 1.0), hi.clamp(0.0, 1.0)))
}

/// Agresti–Coull (1998) confidence interval for binomial proportion `k / n`.
#[must_use]
pub fn agresti_coull_ci(k: u64, n: u64, confidence: ConfidenceLevel) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let nf = n as f64;
    let kf = (k.min(n)) as f64;
    let z = norm_two_sided_isf(confidence.alpha()).unwrap_or(1.959_963_984_540_054);
    let z2 = z * z;
    let n_tilde = nf + z2;
    let p_tilde = (kf + 0.5 * z2) / n_tilde;
    let half = z * (p_tilde * (1.0 - p_tilde) / n_tilde).sqrt();
    ((p_tilde - half).clamp(0.0, 1.0), (p_tilde + half).clamp(0.0, 1.0))
}

/// Exact one-sample binomial test for `k` successes out of `n` trials against `null_proportion in (0, 1)`.
pub fn exact_binomial_test(k: u64, n: u64, null_proportion: f64, confidence: ConfidenceLevel) -> Result<ExactBinomialResult, StatsError> {
    if n == 0 {
        return Err(StatsError::EmptyInput);
    }
    if k > n {
        return Err(StatsError::InvalidParameter { name: "binomial_successes", value: k as f64 });
    }
    if n > EXACT_BINOMIAL_MAX_TOTAL {
        return Err(StatsError::InvalidParameter { name: "binomial_max_total", value: n as f64 });
    }
    if !null_proportion.is_finite() || null_proportion <= 0.0 || null_proportion >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "null_proportion", value: null_proportion });
    }

    let ln_p0 = null_proportion.ln();
    let ln_q0 = (1.0 - null_proportion).ln();
    let pmf_at = |j: u64| -> Result<f64, StatsError> {
        let jf = j as f64;
        let rem = (n - j) as f64;
        let log_pmf = ln_choose(n, j)? + jf * ln_p0 + rem * ln_q0;
        Ok(log_pmf.exp())
    };

    let p_obs = pmf_at(k)?;
    let threshold = p_obs * (1.0 + 1e-12);
    let mut p_lower = 0.0_f64;
    let mut p_upper = 0.0_f64;
    let mut p_sterne = 0.0_f64;

    for j in 0..=n {
        let pj = pmf_at(j)?;
        if j <= k {
            p_lower += pj;
        }
        if j >= k {
            p_upper += pj;
        }
        if pj <= threshold {
            p_sterne += pj;
        }
    }

    let p_lower = p_lower.clamp(0.0, 1.0);
    let p_upper = p_upper.clamp(0.0, 1.0);
    let p_sterne = p_sterne.clamp(0.0, 1.0);
    let p_doubled = (2.0 * p_lower.min(p_upper)).clamp(0.0, 1.0);

    Ok(ExactBinomialResult {
        successes: k,
        total: n,
        null_proportion,
        proportion: (k as f64) / (n as f64),
        p_value_lower: p_lower,
        p_value_upper: p_upper,
        p_value_sterne: p_sterne,
        p_value_doubled_tail: p_doubled,
        clopper_pearson_ci: clopper_pearson_ci(k, n, confidence)?,
        wilson_ci: wilson_score_ci(k, n, confidence),
        agresti_coull_ci: agresti_coull_ci(k, n, confidence),
    })
}

// ============================================================================
// Contingency Tests (Chi-Square, G^2, Fisher, Cochran–Armitage, McNemar)
// ============================================================================

const FISHER_2X2_MAX_TOTAL: u64 = 5_000_000;
const FISHER_RXC_MAX_TOTAL: u64 = 50_000;
const FISHER_RXC_ANALYZE_MAX_TOTAL: u64 = 20_000;
const MCNEMAR_MAX_TOTAL: u64 = 5_000_000;
const EXACT_BINOMIAL_MAX_TOTAL: u64 = 5_000_000;
const NONCENTRAL_HYPERGEOM_MAX_TOTAL: u64 = 5_000_000;
const NONCENTRAL_HYPERGEOM_MAX_SPAN: u64 = 100_000;

fn checked_sum_u64(vals: &[u64], name: &'static str) -> Result<u64, StatsError> {
    vals.iter().try_fold(0_u64, |acc, &v| acc.checked_add(v)).ok_or(StatsError::InvalidParameter { name, value: f64::INFINITY })
}

fn checked_table_margins(counts: &[Vec<u64>]) -> Result<(Vec<u64>, Vec<u64>, u64), StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    let mut r_sums = Vec::with_capacity(n_rows);
    let mut c_sums = vec![0_u64; n_cols];
    let mut total = 0_u64;
    for row in counts {
        if row.len() != n_cols {
            return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n_cols, actual: row.len() }));
        }
        let r_sum = checked_sum_u64(row, "contingency_row_total")?;
        r_sums.push(r_sum);
        total = total.checked_add(r_sum).ok_or(StatsError::InvalidParameter { name: "contingency_table_total", value: f64::INFINITY })?;
        for (c, &v) in row.iter().enumerate() {
            c_sums[c] = c_sums[c].checked_add(v).ok_or(StatsError::InvalidParameter { name: "contingency_col_total", value: f64::INFINITY })?;
        }
    }
    Ok((r_sums, c_sums, total))
}

/// Log-probability of observing integer matrix `counts` given its row sums `r_sums` and column sums
/// `c_sums` under the multiple hypergeometric null distribution.
fn log_hypergeom_table(counts: &[Vec<u64>], r_sums: &[u64], c_sums: &[u64], total: u64) -> Result<f64, StatsError> {
    let mut num = 0.0_f64;
    for &r in r_sums {
        num += ln_factorial(r);
    }
    for &c in c_sums {
        num += ln_factorial(c);
    }
    let mut den = ln_factorial(total);
    for row in counts {
        for &v in row {
            den += ln_factorial(v);
        }
    }
    Ok(num - den)
}

/// Exact two-sided Fisher's test for a `2 x 2` count matrix `[[a, b], [c, d]]`.
pub fn fisher_exact_2x2(a: u64, b: u64, c: u64, d: u64) -> Result<f64, StatsError> {
    let r1 = a.checked_add(b).ok_or(StatsError::InvalidParameter { name: "fisher_exact_row1_total", value: f64::INFINITY })?;
    let r2 = c.checked_add(d).ok_or(StatsError::InvalidParameter { name: "fisher_exact_row2_total", value: f64::INFINITY })?;
    let c1 = a.checked_add(c).ok_or(StatsError::InvalidParameter { name: "fisher_exact_col1_total", value: f64::INFINITY })?;
    let total = r1.checked_add(r2).ok_or(StatsError::InvalidParameter { name: "fisher_exact_table_total", value: f64::INFINITY })?;
    if total == 0 {
        return Ok(1.0);
    }
    if total > FISHER_2X2_MAX_TOTAL {
        return Err(StatsError::InvalidParameter { name: "fisher_exact_max_total", value: total as f64 });
    }
    let lo = c1.saturating_sub(r2);
    let hi = r1.min(c1);
    let log_const = ln_factorial(r1) + ln_factorial(r2) + ln_factorial(c1) + ln_factorial(total - c1) - ln_factorial(total);
    let obs_log = log_const - (ln_factorial(a) + ln_factorial(b) + ln_factorial(c) + ln_factorial(d));

    let mut p_sum = 0.0_f64;
    for x in lo..=hi {
        let bx = r1 - x;
        let cx = c1 - x;
        let dx = r2 - cx;
        let lp = log_const - (ln_factorial(x) + ln_factorial(bx) + ln_factorial(cx) + ln_factorial(dx));
        if lp <= obs_log + 1e-12 {
            p_sum += lp.exp();
        }
    }
    Ok(p_sum.clamp(0.0, 1.0))
}

const BARNARD_MAX_TOTAL: u64 = 2_000;
const BARNARD_MAX_GRID_CELLS: u64 = 250_000;
const BARNARD_MAX_BOSCHLOO_TOTAL: u64 = 600;
const BARNARD_MAX_CSM_CELLS: u64 = 1_600;

fn z_pooled_2x2(i: u64, n1: u64, j: u64, n2: u64) -> f64 {
    let total = n1 + n2;
    let s = i + j;
    if s == 0 || s == total || n1 == 0 || n2 == 0 {
        return 0.0;
    }
    let p1 = (i as f64) / (n1 as f64);
    let p2 = (j as f64) / (n2 as f64);
    let p_pool = (s as f64) / (total as f64);
    let denom = (p_pool * (1.0 - p_pool) * (1.0 / (n1 as f64) + 1.0 / (n2 as f64))).sqrt();
    if denom > 0.0 { (p1 - p2) / denom } else { 0.0 }
}

fn z_wald_2x2(i: u64, n1: u64, j: u64, n2: u64) -> f64 {
    if n1 == 0 || n2 == 0 {
        return 0.0;
    }
    let p1 = (i as f64) / (n1 as f64);
    let p2 = (j as f64) / (n2 as f64);
    let diff = p1 - p2;
    if diff == 0.0 {
        return 0.0;
    }
    let var = p1 * (1.0 - p1) / (n1 as f64) + p2 * (1.0 - p2) / (n2 as f64);
    if var > 0.0 { diff / var.sqrt() } else { diff.signum() * f64::INFINITY }
}

fn build_ln_factorials(n_max: usize) -> Result<Vec<f64>, StatsError> {
    let mut out = Vec::with_capacity(n_max + 1);
    for k in 0..=n_max {
        out.push(ln_factorial(k as u64));
    }
    Ok(out)
}

fn precompute_binom_terms(f_s: &[f64], ln_fact: &[f64], total: usize) -> Vec<(usize, f64, f64)> {
    let mut terms = Vec::new();
    let ln_n = ln_fact[total];
    for (s, &f_val) in f_s.iter().enumerate() {
        if f_val > 0.0 {
            let ln_binom = ln_n - ln_fact[s] - ln_fact[total - s];
            terms.push((s, f_val, ln_binom));
        }
    }
    terms
}

fn eval_unconditional_prob(terms: &[(usize, f64, f64)], f0: f64, fn_end: f64, total: usize, pi: f64) -> f64 {
    if pi <= 0.0 {
        return f0;
    }
    if pi >= 1.0 {
        return fn_end;
    }
    let ln_pi = pi.ln();
    let ln_1_pi = (1.0 - pi).ln();
    let mut sum = 0.0_f64;
    for &(s, f_val, ln_binom) in terms {
        let lp = ln_binom + (s as f64) * ln_pi + ((total - s) as f64) * ln_1_pi;
        if lp > -745.0 {
            sum += f_val * lp.exp();
        }
    }
    sum.clamp(0.0, 1.0)
}

fn golden_maximize_pi<F: Fn(f64) -> f64>(eval: &F, mut a: f64, mut b: f64) -> (f64, f64) {
    const INV_PHI: f64 = 0.618_033_988_749_894_9;
    let mut c = b - INV_PHI * (b - a);
    let mut d = a + INV_PHI * (b - a);
    let mut fc = eval(c);
    let mut fd = eval(d);
    for _ in 0..55 {
        if fc < fd {
            a = c;
            c = d;
            fc = fd;
            d = a + INV_PHI * (b - a);
            fd = eval(d);
        } else {
            b = d;
            d = c;
            fd = fc;
            c = b - INV_PHI * (b - a);
            fc = eval(c);
        }
    }
    let mid = 0.5 * (a + b);
    let fm = eval(mid);
    if fc >= fd && fc >= fm {
        (fc, c)
    } else if fd >= fm {
        (fd, d)
    } else {
        (fm, mid)
    }
}

fn maximize_nuisance_pi(f_s: &[f64], ln_fact: &[f64], total: usize, pi_hint: f64) -> (f64, f64) {
    let f0 = f_s.first().copied().unwrap_or(0.0);
    let fn_end = f_s.last().copied().unwrap_or(0.0);
    let terms = precompute_binom_terms(f_s, ln_fact, total);
    if terms.is_empty() {
        return (0.0, 0.5);
    }

    let mut pis = Vec::with_capacity(1120 + total.min(400));
    for k in 0..=1000 {
        pis.push((k as f64) / 1000.0);
    }
    if total > 0 {
        let tf = total as f64;
        for k in 1..=32 {
            let eps = (k as f64) / (32.0 * tf);
            if eps > 0.0 && eps < 1.0 {
                pis.push(eps);
                pis.push(1.0 - eps);
            }
        }
        if total <= 400 {
            for s in 1..total {
                pis.push((s as f64) / tf);
            }
        }
    }
    pis.sort_by(f64::total_cmp);
    pis.dedup_by(|a, b| (*a - *b).abs() < 1e-13);

    let eval = |pi: f64| eval_unconditional_prob(&terms, f0, fn_end, total, pi);
    let vals: Vec<f64> = pis.iter().map(|&pi| eval(pi)).collect();

    let mut best_p = -1.0_f64;
    let mut best_pi = 0.5_f64;
    for (idx, (&pi, &val)) in pis.iter().zip(vals.iter()).enumerate() {
        if val > best_p {
            best_p = val;
            best_pi = pi;
        }
        let ge_prev = idx == 0 || val >= vals[idx - 1];
        let ge_next = idx + 1 == vals.len() || val >= vals[idx + 1];
        if ge_prev && ge_next && idx > 0 && idx + 1 < vals.len() {
            let (cand_p, cand_pi) = golden_maximize_pi(&eval, pis[idx - 1], pis[idx + 1]);
            if cand_p > best_p {
                best_p = cand_p;
                best_pi = cand_pi;
            }
        }
    }
    let sym_pi = (1.0 - best_pi).clamp(0.0, 1.0);
    let sym_val = eval(sym_pi);
    if (best_p - sym_val).abs() <= 1.0e-12 * best_p.max(1.0) && (sym_pi - pi_hint).abs() < (best_pi - pi_hint).abs() {
        best_p = best_p.max(sym_val);
        best_pi = sym_pi;
    }
    (best_p.clamp(0.0, 1.0), best_pi)
}

fn fast_grid_max_pi(f_s: &[f64], ln_fact: &[f64], total: usize) -> f64 {
    let f0 = f_s.first().copied().unwrap_or(0.0);
    let fn_end = f_s.last().copied().unwrap_or(0.0);
    let terms = precompute_binom_terms(f_s, ln_fact, total);
    let mut best = f0.max(fn_end);
    for k in 1..200 {
        let pi = (k as f64) / 200.0;
        let v = eval_unconditional_prob(&terms, f0, fn_end, total, pi);
        if v > best {
            best = v;
        }
    }
    best
}

fn accumulate_score_region(a: usize, n1: usize, c: usize, n2: usize, ln_fact: &[f64]) -> Vec<f64> {
    let total = n1 + n2;
    let s_obs = (a + c) as u128;
    let obs_diff = (a * n2).abs_diff(c * n1) as u128;
    let obs_num_sq = obs_diff * obs_diff;
    let obs_s_term = s_obs * ((total as u128) - s_obs);
    let mut f_s = vec![0.0_f64; total + 1];
    let ln_n1 = ln_fact[n1];
    let ln_n2 = ln_fact[n2];
    let ln_total = ln_fact[total];

    for i in 0..=n1 {
        let lp_i = ln_n1 - ln_fact[i] - ln_fact[n1 - i];
        for j in 0..=n2 {
            let s = i + j;
            if s == 0 || s == total {
                continue;
            }
            let d_ij = (i * n2).abs_diff(j * n1) as u128;
            let lhs = (d_ij * d_ij).saturating_mul(obs_s_term);
            let s_term = (s as u128) * ((total - s) as u128);
            let rhs = obs_num_sq.saturating_mul(s_term);
            if lhs >= rhs {
                let lp_j = ln_n2 - ln_fact[j] - ln_fact[n2 - j];
                let ln_hg = lp_i + lp_j + ln_fact[s] + ln_fact[total - s] - ln_total;
                f_s[s] += ln_hg.exp();
            }
        }
    }
    f_s
}

fn accumulate_wald_region(a: usize, n1: usize, c: usize, n2: usize, ln_fact: &[f64]) -> Vec<f64> {
    let total = n1 + n2;
    let z_obs = z_wald_2x2(a as u64, n1 as u64, c as u64, n2 as u64).abs();
    let mut f_s = vec![0.0_f64; total + 1];
    let ln_n1 = ln_fact[n1];
    let ln_n2 = ln_fact[n2];
    let ln_total = ln_fact[total];

    for i in 0..=n1 {
        let lp_i = ln_n1 - ln_fact[i] - ln_fact[n1 - i];
        for j in 0..=n2 {
            let z = z_wald_2x2(i as u64, n1 as u64, j as u64, n2 as u64).abs();
            let in_region = if z_obs.is_infinite() { z.is_infinite() } else { z >= z_obs - 1e-12 };
            if in_region {
                let s = i + j;
                let lp_j = ln_n2 - ln_fact[j] - ln_fact[n2 - j];
                let ln_hg = lp_i + lp_j + ln_fact[s] + ln_fact[total - s] - ln_total;
                f_s[s] += ln_hg.exp();
            }
        }
    }
    f_s
}

fn accumulate_boschloo_region(a: usize, n1: usize, c: usize, n2: usize, ln_fact: &[f64]) -> Vec<f64> {
    let total = n1 + n2;
    let s_obs = a + c;
    let ln_n1 = ln_fact[n1];
    let ln_n2 = ln_fact[n2];
    let ln_total = ln_fact[total];

    let fisher_slice = |s: usize| -> Vec<(usize, f64, f64)> {
        let lo = s.saturating_sub(n2);
        let hi = n1.min(s);
        let log_const = ln_n1 + ln_n2 + ln_fact[s] + ln_fact[total - s] - ln_total;
        let mut probs: Vec<(usize, f64)> = Vec::with_capacity(hi - lo + 1);
        for x in lo..=hi {
            let lp = log_const - (ln_fact[x] + ln_fact[n1 - x] + ln_fact[s - x] + ln_fact[n2 - (s - x)]);
            probs.push((x, lp.exp()));
        }
        let mut sorted = probs.clone();
        sorted.sort_by(|u, v| u.1.total_cmp(&v.1));
        let mut out = Vec::with_capacity(probs.len());
        for (x, px) in probs {
            let p_fisher: f64 = sorted.iter().take_while(|(_, pv)| *pv <= px + 1e-12).map(|(_, pv)| *pv).sum();
            out.push((x, px, p_fisher.clamp(0.0, 1.0)));
        }
        out
    };

    let obs_slice = fisher_slice(s_obs);
    let p_obs = obs_slice.iter().find(|&&(x, _, _)| x == a).map_or(1.0, |&(_, _, pf)| pf);

    let mut f_s = vec![0.0_f64; total + 1];
    for (s, slot) in f_s.iter_mut().enumerate() {
        for (_, hg_p, pf) in fisher_slice(s) {
            if pf <= p_obs + 1e-12 {
                *slot += hg_p;
            }
        }
    }
    f_s
}

fn csm_frontier_pairs(in_region: &[Vec<bool>], n1: usize, n2: usize) -> Vec<Vec<(usize, usize)>> {
    let mut pairs = Vec::new();
    let mut seen = vec![vec![false; n2 + 1]; n1 + 1];
    for i in 0..=n1 {
        for j in 0..=n2 {
            if in_region[i][j] || seen[i][j] {
                continue;
            }
            let lhs = i * n2;
            let rhs = j * n1;
            let eligible = if lhs > rhs {
                let up_ok = i == n1 || in_region[i + 1][j];
                let left_ok = j == 0 || in_region[i][j - 1];
                up_ok && left_ok
            } else if lhs < rhs {
                let down_ok = i == 0 || in_region[i - 1][j];
                let right_ok = j == n2 || in_region[i][j + 1];
                down_ok && right_ok
            } else {
                false
            };
            if eligible {
                let ci = n1 - i;
                let cj = n2 - j;
                seen[i][j] = true;
                seen[ci][cj] = true;
                if i == ci && j == cj {
                    pairs.push(vec![(i, j)]);
                } else {
                    pairs.push(vec![(i, j), (ci, cj)]);
                }
            }
        }
    }
    pairs
}

fn accumulate_csm_region(a: usize, n1: usize, c: usize, n2: usize, ln_fact: &[f64]) -> Vec<f64> {
    let total = n1 + n2;
    let ln_n1 = ln_fact[n1];
    let ln_n2 = ln_fact[n2];
    let ln_total = ln_fact[total];
    let hg_at = |i: usize, j: usize| -> f64 {
        let s = i + j;
        (ln_n1 - ln_fact[i] - ln_fact[n1 - i] + ln_n2 - ln_fact[j] - ln_fact[n2 - j] + ln_fact[s] + ln_fact[total - s] - ln_total).exp()
    };

    let mut in_region = vec![vec![false; n2 + 1]; n1 + 1];
    let mut f_s = vec![0.0_f64; total + 1];
    while !in_region[a][c] {
        let candidates = csm_frontier_pairs(&in_region, n1, n2);
        if candidates.is_empty() {
            break;
        }
        let mut best_val = f64::INFINITY;
        let mut scores = Vec::with_capacity(candidates.len());
        for pair in &candidates {
            let mut prev = [0.0_f64; 2];
            for (idx, &(i, j)) in pair.iter().enumerate() {
                let s = i + j;
                prev[idx] = f_s[s];
                f_s[s] += hg_at(i, j);
            }
            let val = fast_grid_max_pi(&f_s, ln_fact, total);
            if val < best_val {
                best_val = val;
            }
            scores.push(val);
            for (idx, &(i, j)) in pair.iter().enumerate().rev() {
                f_s[i + j] = prev[idx];
            }
        }
        for (pair, &val) in candidates.iter().zip(scores.iter()) {
            if val <= best_val + 1e-12 {
                for &(i, j) in pair {
                    if !in_region[i][j] {
                        in_region[i][j] = true;
                        f_s[i + j] += hg_at(i, j);
                    }
                }
            }
        }
    }
    f_s
}

/// Barnard's exact unconditional two-sided test for a `2 x 2` count matrix `[[a, b], [c, d]]`
/// using the Suissa & Shuster (1985) pooled-variance score statistic (`ScorePooled`).
pub fn barnard_exact_2x2(a: u64, b: u64, c: u64, d: u64) -> Result<BarnardExactResult, StatsError> {
    barnard_exact_2x2_with_ordering(a, b, c, d, BarnardOrdering::ScorePooled)
}

/// Barnard's exact unconditional two-sided test for `[[a, b], [c, d]]` with configurable
/// rejection-region ordering (`ScorePooled`, `WaldUnpooled`, `Boschloo`, or `Csm`).
pub fn barnard_exact_2x2_with_ordering(a: u64, b: u64, c: u64, d: u64, ordering: BarnardOrdering) -> Result<BarnardExactResult, StatsError> {
    let n1 = a.checked_add(b).ok_or(StatsError::InvalidParameter { name: "barnard_row1_total", value: f64::INFINITY })?;
    let n2 = c.checked_add(d).ok_or(StatsError::InvalidParameter { name: "barnard_row2_total", value: f64::INFINITY })?;
    let total = n1.checked_add(n2).ok_or(StatsError::InvalidParameter { name: "barnard_table_total", value: f64::INFINITY })?;
    if total == 0 || n1 == 0 || n2 == 0 {
        return Ok(BarnardExactResult { statistic: 0.0, p_value: 1.0, nuisance_pi: 0.5, ordering, used_asymptotic_fallback: false });
    }

    let z_stat = match ordering {
        BarnardOrdering::WaldUnpooled => z_wald_2x2(a, n1, c, n2),
        BarnardOrdering::ScorePooled | BarnardOrdering::Boschloo | BarnardOrdering::Csm => z_pooled_2x2(a, n1, c, n2),
    };
    if (a as u128) * (n2 as u128) == (c as u128) * (n1 as u128) {
        return Ok(BarnardExactResult { statistic: 0.0, p_value: 1.0, nuisance_pi: 0.5, ordering, used_asymptotic_fallback: false });
    }

    let grid_cells = n1.saturating_add(1).saturating_mul(n2.saturating_add(1));
    if ordering == BarnardOrdering::Csm && grid_cells > BARNARD_MAX_CSM_CELLS {
        return Err(StatsError::InvalidParameter { name: "barnard_csm_grid_cells", value: grid_cells as f64 });
    }
    let exceeds_exact_cap = total > BARNARD_MAX_TOTAL
        || grid_cells > BARNARD_MAX_GRID_CELLS
        || (ordering == BarnardOrdering::Boschloo && total > BARNARD_MAX_BOSCHLOO_TOTAL);
    if exceeds_exact_cap {
        let z_pool = z_pooled_2x2(a, n1, c, n2);
        let p_asym = if z_pool.is_finite() { (2.0 * norm_sf(z_pool.abs())).clamp(0.0, 1.0) } else { 0.0 };
        let pi_hat = ((a + c) as f64) / (total as f64);
        return Ok(BarnardExactResult { statistic: z_stat, p_value: p_asym, nuisance_pi: pi_hat, ordering, used_asymptotic_fallback: true });
    }

    let n1_u = n1 as usize;
    let n2_u = n2 as usize;
    let total_u = total as usize;
    let a_u = a as usize;
    let c_u = c as usize;
    let ln_fact = build_ln_factorials(total_u)?;

    let f_s = match ordering {
        BarnardOrdering::ScorePooled => accumulate_score_region(a_u, n1_u, c_u, n2_u, &ln_fact),
        BarnardOrdering::WaldUnpooled => accumulate_wald_region(a_u, n1_u, c_u, n2_u, &ln_fact),
        BarnardOrdering::Boschloo => accumulate_boschloo_region(a_u, n1_u, c_u, n2_u, &ln_fact),
        BarnardOrdering::Csm => accumulate_csm_region(a_u, n1_u, c_u, n2_u, &ln_fact),
    };
    let pi_hint = ((a_u + c_u) as f64) / (total_u as f64);
    let (p_value, nuisance_pi) = maximize_nuisance_pi(&f_s, &ln_fact, total_u, pi_hint);

    Ok(BarnardExactResult { statistic: z_stat, p_value, nuisance_pi, ordering, used_asymptotic_fallback: false })
}

fn build_fisher_rxc_pool(c_sums: &[u64], total: u64) -> Result<Vec<usize>, StatsError> {
    let total_usize = usize::try_from(total).map_err(|_| StatsError::InvalidParameter { name: "fisher_exact_max_total", value: total as f64 })?;
    let mut col_pool = Vec::with_capacity(total_usize);
    for (c_idx, &cnt) in c_sums.iter().enumerate() {
        let cnt_usize = usize::try_from(cnt).map_err(|_| StatsError::InvalidParameter { name: "fisher_exact_max_total", value: cnt as f64 })?;
        let new_len = col_pool
            .len()
            .checked_add(cnt_usize)
            .filter(|&len| len <= total_usize)
            .ok_or(StatsError::InvalidParameter { name: "fisher_exact_max_total", value: total as f64 })?;
        col_pool.resize(new_len, c_idx);
    }
    Ok(col_pool)
}

/// Freeman–Halton exact / deterministic Monte Carlo *p*-value for general `R x C` contingency tables.
///
/// Uses deterministic Patefield-style permutation sampling (`DeterministicRng::from_seed`) for
/// `R*C > 4` or large counts where full enumeration explodes combinatorially, while exact `2x2`
/// uses full hypergeometric enumeration.
pub fn fisher_exact_rxc(counts: &[Vec<u64>]) -> Result<f64, StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    if n_rows < 2 || n_cols < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Fisher R x C contingency", required: 2, actual: n_rows.min(n_cols) });
    }
    let (r_sums, c_sums, total) = checked_table_margins(counts)?;
    if n_rows == 2 && n_cols == 2 {
        return fisher_exact_2x2(counts[0][0], counts[0][1], counts[1][0], counts[1][1]);
    }
    if total == 0 {
        return Ok(1.0);
    }

    // Build flat vector of column assignments and permute deterministically across 10,000 draws.
    if total > FISHER_RXC_MAX_TOTAL {
        // Reject tables exceeding the deterministic Monte Carlo permutation cap rather than silently
        // substituting asymptotic chi-square under a Fisher exact label.
        return Err(StatsError::InvalidParameter { name: "fisher_exact_max_total", value: total as f64 });
    }
    let obs_log = log_hypergeom_table(counts, &r_sums, &c_sums, total)?;
    let mut col_pool = build_fisher_rxc_pool(&c_sums, total)?;
    let n_sims = 10_000_usize;
    let mut rng = DeterministicRng::from_seed(0xC07_4F15_8E2B_1001);
    let mut sim_table = vec![vec![0_u64; n_cols]; n_rows];
    let mut extreme = 0_usize;

    for _ in 0..n_sims {
        rng.shuffle(&mut col_pool);
        for row in &mut sim_table {
            row.fill(0);
        }
        let mut cursor = 0_usize;
        for (r_idx, &r_cnt) in r_sums.iter().enumerate() {
            for _ in 0..r_cnt {
                let Some(&c_idx) = col_pool.get(cursor) else {
                    return Err(StatsError::InvalidParameter { name: "fisher_exact_max_total", value: total as f64 });
                };
                sim_table[r_idx][c_idx] += 1;
                cursor += 1;
            }
        }
        let sim_log = log_hypergeom_table(&sim_table, &r_sums, &c_sums, total)?;
        if sim_log <= obs_log + 1e-12 {
            extreme += 1;
        }
    }
    Ok(((extreme + 1) as f64) / ((n_sims + 1) as f64))
}

fn chi_square_stats_with_margins(
    counts: &[Vec<u64>],
    r_sums: &[u64],
    c_sums: &[u64],
    total: u64,
    yates: bool,
) -> Result<(f64, f64, f64), StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    if n_rows < 2 || n_cols < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "chi-square contingency", required: 2, actual: n_rows.min(n_cols) });
    }
    if total == 0 {
        return Err(StatsError::EmptyInput);
    }
    let total_f = total as f64;
    let use_yates = yates && n_rows == 2 && n_cols == 2;
    let mut chi2 = 0.0_f64;
    let mut g2 = 0.0_f64;
    let mut min_exp = f64::INFINITY;

    for (r, row) in counts.iter().enumerate() {
        let rs_f = r_sums[r] as f64;
        for (c, &o_u) in row.iter().enumerate() {
            let obs = o_u as f64;
            let exp = (rs_f * (c_sums[c] as f64)) / total_f;
            min_exp = min_exp.min(exp);
            if exp > 0.0 {
                let diff = (obs - exp).abs();
                let adj_diff = if use_yates { (diff - 0.5).max(0.0) } else { diff };
                chi2 += (adj_diff * adj_diff) / exp;
                if obs > 0.0 {
                    g2 += 2.0 * obs * (obs / exp).ln();
                }
            }
        }
    }
    Ok((chi2, g2.max(0.0), min_exp))
}

/// Cochran–Armitage chi-square test for linear trend across ordered levels (`R x 2` or `2 x C`).
pub fn cochran_armitage_trend(counts: &[Vec<u64>]) -> Result<(f64, f64), StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    if !((n_cols == 2 && n_rows >= 3) || (n_rows == 2 && n_cols >= 3)) {
        let bad = if n_rows == 2 || n_cols == 2 { n_rows.max(n_cols) } else { n_rows.min(n_cols) };
        return Err(StatsError::InvalidParameter { name: "cochran_armitage_dimensions", value: bad as f64 });
    }
    for row in counts {
        if row.len() != n_cols {
            return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n_cols, actual: row.len() }));
        }
    }
    // Orient as k ordered levels x 2 outcomes.
    let oriented: Vec<[f64; 2]> = if n_cols == 2 {
        counts.iter().map(|r| [r[0] as f64, r[1] as f64]).collect()
    } else {
        (0..n_cols).map(|c| [counts[0][c] as f64, counts[1][c] as f64]).collect()
    };

    let k = oriented.len();
    let mut n_total = 0.0_f64;
    let mut r_sum_0 = 0.0_f64;
    let mut n_i = Vec::with_capacity(k);
    for pair in &oriented {
        let ni = pair[0] + pair[1];
        n_i.push(ni);
        n_total += ni;
        r_sum_0 += pair[0];
    }
    if n_total <= 0.0 || r_sum_0 <= 0.0 || r_sum_0 >= n_total {
        return Ok((0.0, 1.0));
    }
    let p_bar = r_sum_0 / n_total;
    let mut w_mean = 0.0_f64;
    for (idx, &ni) in n_i.iter().enumerate() {
        w_mean += ((idx + 1) as f64) * ni / n_total;
    }

    let mut num = 0.0_f64;
    let mut var_num = 0.0_f64;
    for (idx, pair) in oriented.iter().enumerate() {
        let score = (idx + 1) as f64 - w_mean;
        num += score * pair[0];
        var_num += score * score * n_i[idx];
    }
    let denom = (p_bar * (1.0 - p_bar) * var_num).sqrt();
    if denom <= 0.0 {
        return Ok((0.0, 1.0));
    }
    let z = num / denom;
    let p = 2.0 * norm_sf(z.abs());
    Ok((z, p.clamp(0.0, 1.0)))
}

/// McNemar's paired `2 x 2` test on discordant counts `(b = cell[0][1], c = cell[1][0])`,
/// returning `(chi2_with_continuity, exact_binomial_two_sided_p)`.
pub fn mcnemar_test(b: u64, c: u64) -> Result<(f64, f64), StatsError> {
    let n = b.checked_add(c).ok_or(StatsError::InvalidParameter { name: "mcnemar_discordant_total", value: f64::INFINITY })?;
    if n == 0 {
        return Ok((0.0, 1.0));
    }
    if n > MCNEMAR_MAX_TOTAL {
        return Err(StatsError::InvalidParameter { name: "mcnemar_max_total", value: n as f64 });
    }
    let diff = (b as f64 - c as f64).abs();
    let chi2 = ((diff - 1.0).max(0.0)).powi(2) / (n as f64);
    if b == c {
        return Ok((chi2, 1.0));
    }
    let min_bc = b.min(c);
    let ln_two_n = -(n as f64) * std::f64::consts::LN_2;
    let ln_nf = ln_factorial(n);
    let mut tail = 0.0_f64;
    for k in 0..=min_bc {
        let lp = ln_nf - ln_factorial(k) - ln_factorial(n - k) + ln_two_n;
        tail += lp.exp();
    }
    Ok((chi2, (2.0 * tail).clamp(0.0, 1.0)))
}

/// Detailed McNemar paired analysis including discordant-pair odds ratio `b / c` and its
/// exact Clopper–Pearson confidence interval transformed via `[p_L / (1 - p_L), p_U / (1 - p_U)]`.
pub fn mcnemar_detailed(b: u64, c: u64, confidence: ConfidenceLevel) -> Result<McNemarResult, StatsError> {
    let (chi_square_yates, exact_p_value) = mcnemar_test(b, c)?;
    let n = b.checked_add(c).ok_or(StatsError::InvalidParameter { name: "mcnemar_discordant_total", value: f64::INFINITY })?;
    if n == 0 {
        return Ok(McNemarResult {
            b,
            c,
            chi_square_yates,
            exact_p_value,
            discordant_odds_ratio: None,
            discordant_or_label: String::from("Undefined"),
            discordant_or_ci: None,
            discordant_p_ci: (0.0, 1.0),
        });
    }

    let p_ci = clopper_pearson_ci(b, n, confidence)?;
    let p_to_odds = |p: f64| -> f64 {
        if p >= 1.0 {
            f64::INFINITY
        } else if p <= 0.0 {
            0.0
        } else {
            p / (1.0 - p)
        }
    };
    let or_lo = p_to_odds(p_ci.0);
    let or_hi = p_to_odds(p_ci.1);
    let (odds_ratio, label) = if c == 0 {
        (Some(f64::INFINITY), String::from("∞"))
    } else {
        let val = (b as f64) / (c as f64);
        (Some(val), format!("{val:.4}"))
    };

    Ok(McNemarResult {
        b,
        c,
        chi_square_yates,
        exact_p_value,
        discordant_odds_ratio: odds_ratio,
        discordant_or_label: label,
        discordant_or_ci: Some((or_lo, or_hi)),
        discordant_p_ci: p_ci,
    })
}

/// Full contingency table analysis combining Pearson chi-square, `G^2`, Fisher/Freeman–Halton,
/// Cochran–Armitage (when `R=2` or `C=2`), McNemar (when `2x2`), phi, and Cramér's `V`.
pub fn analyze_contingency(counts: &[Vec<u64>], yates_continuity: bool, confidence: ConfidenceLevel) -> Result<ContingencyResult, StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    if n_rows == 0 || n_cols == 0 {
        return Err(StatsError::EmptyInput);
    }
    let (r_sums, c_sums, total_count) = checked_table_margins(counts)?;
    if total_count == 0 {
        return Err(StatsError::EmptyInput);
    }

    let (chi2, g2, min_exp) = chi_square_stats_with_margins(counts, &r_sums, &c_sums, total_count, yates_continuity)?;
    let df = ((n_rows - 1) * (n_cols - 1)) as f64;
    let chi2_p = chi2_sf(chi2, df)?;
    let g2_p = chi2_sf(g2, df)?;

    // Count fraction of cells with expected count < 5.
    let total_f = total_count as f64;
    let mut lt5 = 0_usize;
    for &rs in &r_sums {
        for &cs in &c_sums {
            if ((rs as f64) * (cs as f64)) / total_f < 5.0 {
                lt5 += 1;
            }
        }
    }
    let cells_lt5_fraction = (lt5 as f64) / ((n_rows * n_cols) as f64);

    let fisher_exact_p = if n_rows == 2 && n_cols == 2 {
        if total_count <= FISHER_2X2_MAX_TOTAL { Some(fisher_exact_2x2(counts[0][0], counts[0][1], counts[1][0], counts[1][1])?) } else { None }
    } else if n_rows <= 6 && n_cols <= 6 && total_count <= FISHER_RXC_ANALYZE_MAX_TOTAL {
        Some(fisher_exact_rxc(counts)?)
    } else {
        None
    };

    let (mcnemar_chi2, mcnemar_exact_p) = if n_rows == 2 && n_cols == 2 {
        let discordant_total =
            counts[0][1].checked_add(counts[1][0]).ok_or(StatsError::InvalidParameter { name: "mcnemar_discordant_total", value: f64::INFINITY })?;
        if discordant_total <= MCNEMAR_MAX_TOTAL {
            let (mc, mp) = mcnemar_test(counts[0][1], counts[1][0])?;
            (Some(mc), Some(mp))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let (cochran_armitage_z, cochran_armitage_p) = if (n_rows == 2 && n_cols >= 3) || (n_cols == 2 && n_rows >= 3) {
        let (z, p) = cochran_armitage_trend(counts)?;
        (Some(z), Some(p))
    } else {
        (None, None)
    };

    // Uncorrected Pearson chi-square for phi and Cramer's V.
    let (chi2_uncorr, _, _) =
        if yates_continuity { chi_square_stats_with_margins(counts, &r_sums, &c_sums, total_count, false)? } else { (chi2, g2, min_exp) };
    let phi = if n_rows == 2 && n_cols == 2 { Some((chi2_uncorr / total_f).sqrt()) } else { None };
    let k_min = (n_rows.min(n_cols) - 1) as f64;
    let cramers_v = (chi2_uncorr / (total_f * k_min)).sqrt().clamp(0.0, 1.0);
    let cramers_v_ci = noncentral_chi2_ci(chi2_uncorr, df, confidence.percent()).ok().map(|(lo, hi)| {
        let v_lo = (lo / (total_f * k_min)).sqrt().clamp(0.0, 1.0);
        let v_hi = (hi / (total_f * k_min)).sqrt().clamp(0.0, 1.0);
        (v_lo, v_hi)
    });

    Ok(ContingencyResult {
        n_rows,
        n_cols,
        total_count,
        chi_square: chi2,
        df,
        chi_square_p: chi2_p,
        g_squared: g2,
        g_squared_p: g2_p,
        fisher_exact_p,
        mcnemar_chi2,
        mcnemar_exact_p,
        cochran_armitage_z,
        cochran_armitage_p,
        phi,
        cramers_v,
        cramers_v_ci,
        min_expected_count: min_exp,
        cells_lt5_fraction,
    })
}

// ============================================================================
// 2 x 2 Effect Ratios & Score / Exact CIs (Baptista-Pike, Koopman, Newcombe)
// ============================================================================

/// Noncentral hypergeometric exact two-sided *p*-value as a function of log-odds ratio `theta`.
fn noncentral_hypergeom_p(a_obs: u64, r1: u64, r2: u64, c1: u64, or_val: f64) -> Result<f64, StatsError> {
    let total = r1.checked_add(r2).ok_or(StatsError::InvalidParameter { name: "noncentral_hypergeom_total", value: f64::INFINITY })?;
    if total > NONCENTRAL_HYPERGEOM_MAX_TOTAL {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_max_total", value: total as f64 });
    }
    if c1 > total {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_col1", value: c1 as f64 });
    }
    let lo = c1.saturating_sub(r2);
    let hi = r1.min(c1);
    if lo > hi || a_obs < lo || a_obs > hi {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_cell", value: a_obs as f64 });
    }
    if !or_val.is_finite() || or_val < 0.0 {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_or", value: or_val });
    }
    let span = hi - lo + 1;
    if span > NONCENTRAL_HYPERGEOM_MAX_SPAN {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_max_span", value: span as f64 });
    }
    let span_usize = usize::try_from(span).map_err(|_| StatsError::InvalidParameter { name: "noncentral_hypergeom_max_span", value: span as f64 })?;

    let log_or = or_val.max(1e-18).ln();
    let mut log_weights = Vec::with_capacity(span_usize);
    let mut max_lw = f64::NEG_INFINITY;
    for x in lo..=hi {
        let bx = r1 - x;
        let cx = c1 - x;
        let dx = r2 - cx;
        let lw = (x as f64) * log_or - (ln_factorial(x) + ln_factorial(bx) + ln_factorial(cx) + ln_factorial(dx));
        max_lw = max_lw.max(lw);
        log_weights.push((x, lw));
    }
    let mut norm = 0.0_f64;
    let mut probs = Vec::with_capacity(log_weights.len());
    for (x, lw) in log_weights {
        let w = (lw - max_lw).exp();
        norm += w;
        probs.push((x, w));
    }
    if !norm.is_finite() || norm <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_norm", value: norm });
    }
    let mut obs_p = 0.0_f64;
    for &(x, w) in &probs {
        if x == a_obs {
            obs_p = w / norm;
            break;
        }
    }
    let p_sum: f64 = probs.iter().map(|(_, w)| w / norm).filter(|&p| p <= obs_p + 1e-12).sum();
    Ok(p_sum.clamp(0.0, 1.0))
}

/// Baptista–Pike (1977) exact conditional odds-ratio confidence interval for `2 x 2` table `[[a, b], [c, d]]`.
pub fn baptista_pike_or_ci(a: u64, b: u64, c: u64, d: u64, confidence: ConfidenceLevel) -> Option<(f64, f64)> {
    let r1 = a.checked_add(b)?;
    let r2 = c.checked_add(d)?;
    let c1 = a.checked_add(c)?;
    let total = r1.checked_add(r2)?;
    let lo_x = c1.saturating_sub(r2);
    let hi_x = r1.min(c1);
    let span = hi_x.checked_sub(lo_x).and_then(|v| v.checked_add(1))?;
    if r1 == 0 || r2 == 0 || lo_x >= hi_x || total > NONCENTRAL_HYPERGEOM_MAX_TOTAL || span > NONCENTRAL_HYPERGEOM_MAX_SPAN {
        return None;
    }
    let alpha = confidence.alpha();
    // Point OR estimate with Haldane correction only for locating search bracket.
    let or_pt = if b == 0 || c == 0 {
        ((a as f64 + 0.5) * (d as f64 + 0.5)) / ((b as f64 + 0.5) * (c as f64 + 0.5))
    } else if a == 0 || d == 0 {
        0.5 / (((b as f64) * (c as f64)).max(1.0))
    } else {
        ((a as f64) * (d as f64)) / ((b as f64) * (c as f64))
    };

    let lo_bound = if a <= lo_x {
        0.0
    } else {
        let mut l = or_pt.max(1e-6);
        for _ in 0..40 {
            if noncentral_hypergeom_p(a, r1, r2, c1, l).ok()? < alpha {
                break;
            }
            l *= 0.5;
        }
        brent_root(
            |theta| noncentral_hypergeom_p(a, r1, r2, c1, theta).unwrap_or(0.0) - alpha,
            l,
            or_pt.max(l * 1.1),
            RootOptions { rel_tol: 1e-7, abs_tol: 1e-8, max_iters: 64 },
        )
        .unwrap_or(l)
    };

    let hi_bound = if a >= hi_x {
        f64::INFINITY
    } else {
        let mut h = or_pt.max(1.0);
        for _ in 0..40 {
            if noncentral_hypergeom_p(a, r1, r2, c1, h).ok()? < alpha {
                break;
            }
            h *= 2.0;
        }
        brent_root(
            |theta| noncentral_hypergeom_p(a, r1, r2, c1, theta).unwrap_or(0.0) - alpha,
            or_pt.min(h * 0.9),
            h,
            RootOptions { rel_tol: 1e-7, abs_tol: 1e-8, max_iters: 64 },
        )
        .unwrap_or(h)
    };

    Some((lo_bound, hi_bound))
}

/// Koopman (1984) asymptotic score confidence interval for relative risk `RR = (a/n1) / (c/n2)`.
pub fn koopman_rr_ci(a: u64, b: u64, c: u64, d: u64, confidence: ConfidenceLevel) -> Option<(f64, f64)> {
    let n1 = (u128::from(a) + u128::from(b)) as f64;
    let n2 = (u128::from(c) + u128::from(d)) as f64;
    if n1 <= 0.0 || n2 <= 0.0 || (a == 0 && c == 0) {
        return None;
    }
    let z2 = norm_two_sided_isf(confidence.alpha()).ok()?.powi(2);
    let score = |phi: f64| koopman_chi2_score(a, b, c, d, z2, phi);

    let rr_pt = if c == 0 { f64::INFINITY } else { ((a as f64) / n1) / ((c as f64) / n2) };

    let lo = if a == 0 {
        0.0
    } else if c == 0 {
        koopman_solve_lower_c_zero(&score)?
    } else {
        koopman_solve_lower(&score, rr_pt)?
    };

    let hi = if c == 0 {
        f64::INFINITY
    } else if a == 0 {
        koopman_solve_upper_a_zero(&score)?
    } else {
        koopman_solve_upper(&score, rr_pt)?
    };

    Some((lo, hi))
}

fn koopman_chi2_score(a: u64, b: u64, c: u64, d: u64, z2: f64, phi: f64) -> f64 {
    if !phi.is_finite() || phi <= 0.0 {
        return f64::INFINITY;
    }
    let x1 = a as f64;
    let x2 = c as f64;
    let n1 = (u128::from(a) + u128::from(b)) as f64;
    let n2 = (u128::from(c) + u128::from(d)) as f64;
    let aq = n1 + n2;
    let abs_b = phi * (n1 + x2) + (x1 + n2); // = -b_q > 0
    let cq = phi * (x1 + x2);
    let disc = if abs_b > 1e150 {
        let r1 = (4.0 * aq) / abs_b;
        let r2 = cq / abs_b;
        abs_b * (1.0 - r1 * r2).max(0.0).sqrt()
    } else {
        (abs_b * abs_b - 4.0 * aq * cq).max(0.0).sqrt()
    };
    // Quadratic solves for constrained MLE p1_tilde (= phi * p2_tilde); Citardauq's formula avoids cancellation.
    let denom = abs_b + disc;
    let p1_raw = (2.0 * cq) / denom;
    let p2_raw = (2.0 * (x1 + x2)) / denom;

    koopman_cell_term(a, b, n1, x1, p1_raw) + koopman_cell_term(c, d, n2, x2, p2_raw) - z2
}

fn koopman_cell_term(events: u64, non_events: u64, n: f64, x: f64, p_raw: f64) -> f64 {
    if events == 0 {
        n * p_raw / (1.0 - p_raw).max(1e-14)
    } else if non_events == 0 {
        n * (1.0 - p_raw).max(0.0) / p_raw.max(1e-100)
    } else {
        let p = p_raw.clamp(1e-100, 1.0 - 1e-15);
        (x - n * p).powi(2) / (n * p * (1.0 - p))
    }
}

const KOOPMAN_BRACKET_STEPS: usize = 120;
const KOOPMAN_ROOT_OPTIONS: RootOptions = RootOptions { rel_tol: 1e-10, abs_tol: 1e-30, max_iters: 128 };

fn koopman_find_negative_score_point<F: Fn(f64) -> f64>(score: &F, scale: f64) -> Option<(f64, f64)> {
    let mut pt = 1.0_f64;
    for _ in 0..KOOPMAN_BRACKET_STEPS {
        if !pt.is_finite() || pt <= 0.0 {
            return None;
        }
        let val = score(pt);
        if !val.is_finite() {
            return None;
        }
        if val <= 0.0 {
            return Some((pt, val));
        }
        pt *= scale;
    }
    None
}

fn koopman_bracket_and_solve<F: Fn(f64) -> f64>(score: &F, neg_start: f64, s_neg_start: f64, scale: f64, lower_branch: bool) -> Option<f64> {
    if !neg_start.is_finite() || neg_start <= 0.0 || !s_neg_start.is_finite() || s_neg_start > 0.0 {
        return None;
    }
    if s_neg_start == 0.0 {
        return Some(neg_start);
    }
    let mut neg_pt = neg_start;
    let mut pos_pt = neg_start * scale;
    let mut bracketed = false;
    for _ in 0..KOOPMAN_BRACKET_STEPS {
        if !pos_pt.is_finite() || pos_pt <= 0.0 {
            return None;
        }
        let s_pos = score(pos_pt);
        if !s_pos.is_finite() {
            return None;
        }
        if s_pos >= 0.0 {
            bracketed = true;
            break;
        }
        neg_pt = pos_pt;
        pos_pt *= scale;
    }
    if !bracketed {
        return None;
    }
    let (lo, hi) = if lower_branch { (pos_pt, neg_pt) } else { (neg_pt, pos_pt) };
    brent_root(score, lo, hi, KOOPMAN_ROOT_OPTIONS).ok()
}

fn koopman_solve_lower<F: Fn(f64) -> f64>(score: &F, rr_pt: f64) -> Option<f64> {
    if !rr_pt.is_finite() || rr_pt <= 0.0 {
        return None;
    }
    let s_neg = score(rr_pt);
    koopman_bracket_and_solve(score, rr_pt, s_neg, 0.5, true)
}

fn koopman_solve_upper<F: Fn(f64) -> f64>(score: &F, rr_pt: f64) -> Option<f64> {
    if !rr_pt.is_finite() || rr_pt <= 0.0 {
        return None;
    }
    let s_neg = score(rr_pt);
    koopman_bracket_and_solve(score, rr_pt, s_neg, 2.0, false)
}

fn koopman_solve_lower_c_zero<F: Fn(f64) -> f64>(score: &F) -> Option<f64> {
    let (neg_pt, s_neg) = koopman_find_negative_score_point(score, 2.0)?;
    koopman_bracket_and_solve(score, neg_pt, s_neg, 0.5, true)
}

fn koopman_solve_upper_a_zero<F: Fn(f64) -> f64>(score: &F) -> Option<f64> {
    let (neg_pt, s_neg) = koopman_find_negative_score_point(score, 0.5)?;
    koopman_bracket_and_solve(score, neg_pt, s_neg, 2.0, false)
}

/// Compute complete `2 x 2` risk/odds ratios (`[[a, b], [c, d]]`), Baptista-Pike OR CI, Koopman
/// Compute Altman (1998) `NNTB` / `NNTH` point summary, CI label, and finite CI bounds (when CI
/// excludes zero) from risk difference `rd` and its CI `(rd_low, rd_high)`.
#[must_use]
pub fn altman_nnt_from_rd(rd: f64, rd_low: f64, rd_high: f64) -> (NntKind, Option<f64>, String, String, Option<(f64, f64)>) {
    const EPS: f64 = 1e-15;
    let (kind, value, point_label) = if rd > EPS {
        let v = 1.0 / rd;
        (NntKind::Benefit, Some(v), format!("NNTB {v:.2}"))
    } else if rd < -EPS {
        let v = 1.0 / rd.abs();
        (NntKind::Harm, Some(v), format!("NNTH {v:.2}"))
    } else {
        (NntKind::Infinite, None, String::from("∞"))
    };

    let (ci_label, ci_bounds) = if rd_low > EPS {
        let lo = 1.0 / rd_high;
        let hi = 1.0 / rd_low;
        (format!("NNTB {lo:.2} to {hi:.2}"), Some((lo, hi)))
    } else if rd_high < -EPS {
        let lo = 1.0 / rd_lo_abs(rd_low);
        let hi = 1.0 / rd_lo_abs(rd_high);
        (format!("NNTH {lo:.2} to {hi:.2}"), Some((lo, hi)))
    } else {
        let harm_str = if rd_low < -EPS { format!("{:.2}", 1.0 / rd_low.abs()) } else { String::from("∞") };
        let benefit_str = if rd_high > EPS { format!("{:.2}", 1.0 / rd_high) } else { String::from("∞") };
        (format!("NNTH {harm_str} to ∞ to NNTB {benefit_str}"), None)
    };

    (kind, value, point_label, ci_label, ci_bounds)
}

#[inline]
fn rd_lo_abs(v: f64) -> f64 {
    v.abs().max(1e-300)
}

/// RR CI, Newcombe risk-difference CI, NNT, and Wilson CIs for sensitivity/specificity/PPV/NPV.
pub fn analyze_2x2_ratios(a: u64, b: u64, c: u64, d: u64, confidence: ConfidenceLevel) -> Result<TwoByTwoRatiosResult, StatsError> {
    let r1 = a.checked_add(b).ok_or(StatsError::InvalidParameter { name: "contingency_row_sum", value: f64::INFINITY })?;
    let r2 = c.checked_add(d).ok_or(StatsError::InvalidParameter { name: "contingency_row_sum", value: f64::INFINITY })?;
    let c1 = a.checked_add(c).ok_or(StatsError::InvalidParameter { name: "contingency_col_sum", value: f64::INFINITY })?;
    let c2 = b.checked_add(d).ok_or(StatsError::InvalidParameter { name: "contingency_col_sum", value: f64::INFINITY })?;
    let _total = r1.checked_add(r2).ok_or(StatsError::InvalidParameter { name: "contingency_total", value: f64::INFINITY })?;
    if r1 == 0 || r2 == 0 {
        return Err(StatsError::EmptyInput);
    }
    let p1 = (a as f64) / (r1 as f64);
    let p2 = (c as f64) / (r2 as f64);

    let or_val =
        if b == 0 || c == 0 { if a == 0 && d == 0 { 1.0 } else { f64::INFINITY } } else { ((a as f64) * (d as f64)) / ((b as f64) * (c as f64)) };
    let rr_val = if p2 > 0.0 {
        p1 / p2
    } else if p1 > 0.0 {
        f64::INFINITY
    } else {
        1.0
    };
    let rd = p1 - p2;
    let nnt = if rd.abs() > 1e-12 { Some(1.0 / rd) } else { None };

    let (l1, u1) = wilson_score_ci(a, r1, confidence);
    let (l2, u2) = wilson_score_ci(c, r2, confidence);
    let rd_lo = (rd - ((p1 - l1).powi(2) + (u2 - p2).powi(2)).sqrt()).clamp(-1.0, 1.0);
    let rd_hi = (rd + ((u1 - p1).powi(2) + (p2 - l2).powi(2)).sqrt()).clamp(-1.0, 1.0);
    let (nnt_kind, nnt_value, nnt_point_label, nnt_ci_label, nnt_ci_bounds) = altman_nnt_from_rd(rd, rd_lo, rd_hi);

    // Standard diagnostic 2x2 orientation: Rows = Test +/-, Cols = Disease +/- (TP=a, FP=b, FN=c, TN=d).
    let sens = if c1 > 0 { (a as f64) / (c1 as f64) } else { 0.0 };
    let spec = if c2 > 0 { (d as f64) / (c2 as f64) } else { 0.0 };
    let ppv = p1;
    let npv = (d as f64) / (r2 as f64);

    Ok(TwoByTwoRatiosResult {
        odds_ratio: or_val,
        or_baptista_pike_ci: baptista_pike_or_ci(a, b, c, d, confidence),
        relative_risk: rr_val,
        rr_koopman_ci: koopman_rr_ci(a, b, c, d, confidence),
        risk_difference: rd,
        rd_newcombe_ci: Some((rd_lo, rd_hi)),
        nnt,
        nnt_kind,
        nnt_value,
        nnt_point_label,
        nnt_ci_label,
        nnt_ci_bounds,
        sensitivity: sens,
        sensitivity_wilson_ci: wilson_score_ci(a, c1, confidence),
        specificity: spec,
        specificity_wilson_ci: wilson_score_ci(d, c2, confidence),
        ppv,
        ppv_wilson_ci: wilson_score_ci(a, r1, confidence),
        npv,
        npv_wilson_ci: wilson_score_ci(d, r2, confidence),
    })
}

// ============================================================================
// Cohen's & Fleiss' Kappa Agreement
// ============================================================================

/// Cohen's kappa (unweighted, linear-weighted, or quadratic-weighted) on a square `K x K` matrix.
pub fn cohens_kappa(counts: &[Vec<u64>], weighting: KappaWeighting, confidence: ConfidenceLevel) -> Result<KappaResult, StatsError> {
    let k = counts.len();
    if k < 2 || counts.iter().any(|r| r.len() != k) {
        return Err(StatsError::InvalidParameter { name: "square_matrix_k", value: k as f64 });
    }
    let (row_sums, _col_sums, total) = checked_table_margins(counts)?;
    if total == 0 {
        return Err(StatsError::EmptyInput);
    }
    let nf = total as f64;
    let row_p: Vec<f64> = row_sums.into_iter().map(|s| (s as f64) / nf).collect();
    let mut col_p = vec![0.0_f64; k];
    for row in counts {
        for (j, &v) in row.iter().enumerate() {
            col_p[j] += (v as f64) / nf;
        }
    }

    let weight = |i: usize, j: usize| -> f64 {
        let diff = i.abs_diff(j) as f64;
        let max_d = (k - 1) as f64;
        match weighting {
            KappaWeighting::Unweighted => {
                if i == j {
                    1.0
                } else {
                    0.0
                }
            }
            KappaWeighting::Linear => 1.0 - diff / max_d,
            KappaWeighting::Quadratic => 1.0 - (diff / max_d).powi(2),
        }
    };

    let mut po = 0.0_f64;
    let mut pe = 0.0_f64;
    let mut w_i_bar = vec![0.0_f64; k];
    let mut w_j_bar = vec![0.0_f64; k];
    for i in 0..k {
        for j in 0..k {
            let w = weight(i, j);
            let pij = (counts[i][j] as f64) / nf;
            po += w * pij;
            pe += w * row_p[i] * col_p[j];
            w_i_bar[i] += w * col_p[j];
            w_j_bar[j] += w * row_p[i];
        }
    }

    let kappa = if (1.0 - pe).abs() > 1e-12 { (po - pe) / (1.0 - pe) } else { 1.0 };

    // Asymptotic SE (Fleiss, Cohen & Everitt 1969).
    let mut var_sum = 0.0_f64;
    for i in 0..k {
        for j in 0..k {
            let pij = (counts[i][j] as f64) / nf;
            let term = weight(i, j) - (w_i_bar[i] + w_j_bar[j]) * (1.0 - kappa);
            var_sum += pij * term * term;
        }
    }
    let c_const = po * pe - 2.0 * pe + po;
    let _ = c_const;
    let denom_sq = (1.0 - pe).powi(2).max(1e-18);
    let se = ((var_sum - (kappa * (1.0 - pe) - (po - pe)).powi(2)).max(0.0) / (nf * denom_sq)).sqrt();

    let z_crit = norm_two_sided_isf(confidence.alpha()).unwrap_or(1.959_963_984_540_054);
    let ci = ((kappa - z_crit * se).clamp(-1.0, 1.0), (kappa + z_crit * se).clamp(-1.0, 1.0));

    // SE under null kappa = 0 for H0: kappa = 0 test.
    let mut var0 = 0.0_f64;
    for i in 0..k {
        for j in 0..k {
            let term = weight(i, j) - (w_i_bar[i] + w_j_bar[j]);
            var0 += row_p[i] * col_p[j] * term * term;
        }
    }
    let se0 = ((var0 - pe * pe).max(0.0) / (nf * denom_sq)).sqrt();
    let z_stat = if se0 > 0.0 { kappa / se0 } else { 0.0 };
    let p_val = 2.0 * (1.0 - norm_cdf(z_stat.abs()));

    Ok(KappaResult { kappa, observed_agreement: po, expected_agreement: pe, se, ci, z_statistic: z_stat, p_value: p_val.clamp(0.0, 1.0) })
}

/// Fleiss' multi-rater kappa across `N` subjects (`rows`) and `K` categories (`cols`), where each
/// row contains the integer counts of `m` raters assigning the subject into each category.
pub fn fleiss_kappa(rater_counts: &[Vec<u64>], confidence: ConfidenceLevel) -> Result<KappaResult, StatsError> {
    let n = rater_counts.len();
    let k = rater_counts.first().map_or(0, Vec::len);
    if n < 2 || k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Fleiss kappa", required: 2, actual: n.min(k) });
    }
    let m = checked_sum_u64(&rater_counts[0], "fleiss_raters_per_subject")?;
    if m < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Fleiss raters per subject", required: 2, actual: m as usize });
    }
    for row in rater_counts {
        if row.len() != k || checked_sum_u64(row, "fleiss_raters_per_subject")? != m {
            return Err(StatsError::InvalidParameter { name: "raters_per_subject", value: m as f64 });
        }
    }

    let nf = n as f64;
    let mf = m as f64;
    let mut col_sums = vec![0.0_f64; k];
    let mut p_bar = 0.0_f64;
    for row in rater_counts {
        let mut sq_sum = 0.0_f64;
        for (j, &c) in row.iter().enumerate() {
            let cf = c as f64;
            col_sums[j] += cf;
            sq_sum += cf * cf;
        }
        p_bar += (sq_sum - mf) / (mf * (mf - 1.0));
    }
    p_bar /= nf;

    let total_assignments = nf * mf;
    let p_j: Vec<f64> = col_sums.iter().map(|&s| s / total_assignments).collect();
    let pe: f64 = p_j.iter().map(|&pj| pj * pj).sum();
    let kappa = if (1.0 - pe).abs() > 1e-12 { (p_bar - pe) / (1.0 - pe) } else { 1.0 };

    // Fleiss exact asymptotic variance under H0.
    let sum_pj3: f64 = p_j.iter().map(|&pj| pj * pj * pj).sum();
    let denom = nf * mf * (mf - 1.0) * (1.0 - pe).powi(2);
    let var0 = if denom > 0.0 { 2.0 * (pe - (2.0 * mf - 3.0) * pe * pe + 2.0 * (mf - 2.0) * sum_pj3) / denom } else { 0.0 };
    let se = var0.max(0.0).sqrt();
    let z_crit = norm_two_sided_isf(confidence.alpha()).unwrap_or(1.959_963_984_540_054);
    let ci = ((kappa - z_crit * se).clamp(-1.0, 1.0), (kappa + z_crit * se).clamp(-1.0, 1.0));
    let z_stat = if se > 0.0 { kappa / se } else { 0.0 };
    let p_val = 2.0 * norm_sf(z_stat.abs());

    Ok(KappaResult { kappa, observed_agreement: p_bar, expected_agreement: pe, se, ci, z_statistic: z_stat, p_value: p_val.clamp(0.0, 1.0) })
}

// ============================================================================
// ResultSheet Builder
// ============================================================================

fn validate_and_run_barnard_if_requested(counts: &[Vec<u64>], method: ContingencyMethod) -> Result<Option<BarnardExactResult>, StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    match method {
        ContingencyMethod::McNemarPaired if n_rows != 2 || n_cols != 2 => {
            Err(StatsError::InvalidParameter { name: "mcnemar_dimensions", value: n_rows.max(n_cols) as f64 })
        }
        ContingencyMethod::CochranArmitageTrend if !((n_rows == 2 && n_cols >= 3) || (n_cols == 2 && n_rows >= 3)) => {
            let bad = if n_rows == 2 || n_cols == 2 { n_rows.max(n_cols) } else { n_rows.min(n_cols) };
            Err(StatsError::InvalidParameter { name: "cochran_armitage_dimensions", value: bad as f64 })
        }
        ContingencyMethod::BarnardExact => {
            if n_rows != 2 || n_cols != 2 || counts.iter().any(|r| r.len() != 2) {
                return Err(StatsError::InvalidParameter { name: "barnard_exact_dimensions", value: n_rows.max(n_cols) as f64 });
            }
            let n1 = counts[0][0]
                .checked_add(counts[0][1])
                .ok_or(StatsError::InvalidParameter { name: "barnard_exact_max_total", value: f64::INFINITY })?;
            let n2 = counts[1][0]
                .checked_add(counts[1][1])
                .ok_or(StatsError::InvalidParameter { name: "barnard_exact_max_total", value: f64::INFINITY })?;
            let total = n1.checked_add(n2).ok_or(StatsError::InvalidParameter { name: "barnard_exact_max_total", value: f64::INFINITY })?;
            let grid_cells = n1.saturating_add(1).saturating_mul(n2.saturating_add(1));
            if total > BARNARD_MAX_TOTAL || grid_cells > BARNARD_MAX_GRID_CELLS {
                return Err(StatsError::InvalidParameter { name: "barnard_exact_max_total", value: total as f64 });
            }
            let br = barnard_exact_2x2(counts[0][0], counts[0][1], counts[1][0], counts[1][1])?;
            if br.used_asymptotic_fallback {
                return Err(StatsError::InvalidParameter { name: "barnard_exact_max_total", value: total as f64 });
            }
            Ok(Some(br))
        }
        _ => Ok(None),
    }
}

struct ContingencyPrimaryOutcome {
    name: &'static str,
    stat: Option<f64>,
    p_value: f64,
    footnote: Option<&'static str>,
}

impl ContingencyPrimaryOutcome {
    fn plain(name: &'static str, stat: Option<f64>, p_value: f64) -> Self {
        Self { name, stat, p_value, footnote: None }
    }
}

fn resolve_contingency_primary(
    out: &mut ResultSheet,
    sheet: &DataSheet,
    counts: &[Vec<u64>],
    res: &ContingencyResult,
    barnard_res: Option<&BarnardExactResult>,
    method: ContingencyMethod,
) -> Result<ContingencyPrimaryOutcome, StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    match method {
        ContingencyMethod::PearsonChiSquare { yates_continuity } => {
            let lbl = if yates_continuity { "Chi-square (Yates)" } else { "Pearson chi-square" };
            Ok(ContingencyPrimaryOutcome::plain(lbl, Some(res.chi_square), res.chi_square_p))
        }
        ContingencyMethod::LikelihoodRatioG2 => Ok(ContingencyPrimaryOutcome::plain("Likelihood-ratio G2", Some(res.g_squared), res.g_squared_p)),
        ContingencyMethod::FisherExact => {
            if n_rows == 2 && n_cols == 2 {
                let fp = res.fisher_exact_p.ok_or(StatsError::InvalidParameter { name: "fisher_exact_max_total", value: res.total_count as f64 })?;
                Ok(ContingencyPrimaryOutcome::plain("Fisher's exact test", None, fp))
            } else {
                let mc_p = match res.fisher_exact_p {
                    Some(fp) => fp,
                    None => {
                        let p = fisher_exact_rxc(counts)?;
                        push_scalar(out, "Fisher's exact P", None, p);
                        p
                    }
                };
                push_scalar(out, "Fisher's exact Monte Carlo draws", None, 10_000.0);
                let note = "R x C Fisher's exact test uses a deterministic Monte Carlo permutation test (10,000 draws; max N = 50,000).";
                Ok(ContingencyPrimaryOutcome { name: "Fisher's exact test (Monte Carlo)", stat: None, p_value: mc_p, footnote: Some(note) })
            }
        }
        ContingencyMethod::BarnardExact => {
            let br = barnard_res.ok_or(StatsError::InvalidParameter { name: "barnard_exact_dimensions", value: n_rows.max(n_cols) as f64 })?;
            Ok(ContingencyPrimaryOutcome::plain("Barnard's exact test", Some(br.statistic), br.p_value))
        }
        ContingencyMethod::CochranArmitageTrend => {
            let ca_p = res
                .cochran_armitage_p
                .ok_or(StatsError::InvalidParameter { name: "cochran_armitage_dimensions", value: n_rows.max(n_cols) as f64 })?;
            Ok(ContingencyPrimaryOutcome::plain("Cochran-Armitage trend", res.cochran_armitage_z, ca_p))
        }
        ContingencyMethod::McNemarPaired => {
            let err = if n_rows == 2 && n_cols == 2 {
                StatsError::InvalidParameter { name: "mcnemar_max_total", value: res.total_count as f64 }
            } else {
                StatsError::InvalidParameter { name: "mcnemar_dimensions", value: n_rows.max(n_cols) as f64 }
            };
            let mc_p = res.mcnemar_exact_p.ok_or(err)?;
            Ok(ContingencyPrimaryOutcome::plain("McNemar's test", res.mcnemar_chi2, mc_p))
        }
        ContingencyMethod::CohenKappa { .. } | ContingencyMethod::FleissKappa | ContingencyMethod::ExactBinomial => {
            Err(StatsError::IncompatibleTable { analysis: "contingency analysis", actual: sheet.archetype.kind() })
        }
    }
}

/// Execute a [`ContingencySpec`] against a `Contingency`, `PartsOfWhole`, or binary `Column` [`DataSheet`].
pub fn run_contingency_analysis(sheet: &DataSheet, spec: &AnalysisSpec, cont_spec: &ContingencySpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let confidence = spec.confidence_level;
    let p_fmt = spec.p_value_format;

    if matches!(cont_spec.method, ContingencyMethod::ExactBinomial) {
        return run_exact_binomial_analysis(sheet, spec, cont_spec, out, confidence, p_fmt);
    }

    if let TableArchetype::PartsOfWhole(pow) = &sheet.archetype {
        return run_parts_of_whole_gof(pow, cont_spec, &mut out, p_fmt);
    }

    let TableArchetype::Contingency(table) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "contingency analysis", actual: sheet.archetype.kind() });
    };
    table.validate_counts()?;
    let n_rows = table.row_count();
    let n_cols = table.data_sets.len();
    let mut counts = vec![vec![0_u64; n_cols]; n_rows];
    for (r, row_vec) in counts.iter_mut().enumerate() {
        for (c, slot) in row_vec.iter_mut().enumerate() {
            *slot = table.count_at(r, c)?.unwrap_or(0);
        }
    }

    match cont_spec.method {
        ContingencyMethod::CohenKappa { weighting } => {
            return run_cohen_kappa_sheet(&counts, weighting, confidence, p_fmt, out);
        }
        ContingencyMethod::FleissKappa => {
            return run_fleiss_kappa_sheet(&counts, confidence, p_fmt, out);
        }
        _ => {}
    }

    let yates = matches!(cont_spec.method, ContingencyMethod::PearsonChiSquare { yates_continuity: true });
    let barnard_res = validate_and_run_barnard_if_requested(&counts, cont_spec.method)?;

    let res = analyze_contingency(&counts, yates, confidence)?;
    push_scalar(&mut out, "Chi-square", None, res.chi_square);
    push_scalar(&mut out, "Chi-square P value", None, res.chi_square_p);
    push_scalar(&mut out, "G-squared", None, res.g_squared);
    push_scalar(&mut out, "Cramer's V", None, res.cramers_v);
    if let Some(fp) = res.fisher_exact_p {
        push_scalar(&mut out, "Fisher's exact P", None, fp);
    }
    if let Some(br) = &barnard_res {
        push_scalar(&mut out, "Barnard's exact P", None, br.p_value);
        push_scalar(&mut out, "Barnard nuisance pi", None, br.nuisance_pi);
        push_scalar(&mut out, "Barnard Z-pooled", None, br.statistic);
    }

    let mcnemar_res = if matches!(cont_spec.method, ContingencyMethod::McNemarPaired) {
        let mc = mcnemar_detailed(counts[0][1], counts[1][0], confidence)?;
        push_scalar(&mut out, "McNemar chi-square", None, mc.chi_square_yates);
        push_scalar(&mut out, "McNemar exact P", None, mc.exact_p_value);
        if let Some(or_val) = mc.discordant_odds_ratio {
            push_scalar(&mut out, "Discordant odds ratio", None, or_val);
        }
        if let Some((or_lo, or_hi)) = mc.discordant_or_ci {
            push_scalar(&mut out, "Discordant OR CI lower", None, or_lo);
            push_scalar(&mut out, "Discordant OR CI upper", None, or_hi);
        }
        Some(mc)
    } else {
        None
    };

    let primary = resolve_contingency_primary(&mut out, sheet, &counts, &res, barnard_res.as_ref(), cont_spec.method)?;

    let mut headers = vec![String::from("Statistic"), String::from("df"), String::from("P value")];
    if cont_spec.compute_phi_and_cramers_v {
        headers.push(String::from("Cramer's V / Phi"));
        headers.push(format!("{}% CI", confidence.percent()));
    }
    let mut row_main = vec![opt_num_cell(primary.stat), num_cell(res.df), p_cell(primary.p_value, p_fmt)];
    if cont_spec.compute_phi_and_cramers_v {
        row_main.push(num_cell(res.phi.unwrap_or(res.cramers_v)));
        row_main.push(opt_ci_cell(res.cramers_v_ci));
    }
    out.tabs.push(build_tab(
        "contingency-main",
        "Contingency analysis",
        ResultTabKind::MainSummary,
        headers,
        vec![primary.name.to_owned()],
        vec![row_main],
    ));
    if let Some(note) = primary.footnote
        && let Some(tab) = out.tabs.last_mut()
    {
        tab.footnotes.push(String::from(note));
    }

    if let Some(mc) = &mcnemar_res {
        let or_cell = match mc.discordant_odds_ratio {
            Some(v) if v.is_finite() => num_cell(v),
            Some(_) => ResultCell::Text(String::from("∞")),
            None => ResultCell::Empty,
        };
        let disc_total = mc.b.checked_add(mc.c).ok_or(StatsError::InvalidParameter { name: "mcnemar_discordant_total", value: f64::INFINITY })?;
        let disc_prop_cell = if disc_total > 0 { num_cell((mc.b as f64) / (disc_total as f64)) } else { ResultCell::Empty };
        out.tabs.push(build_tab(
            "contingency-mcnemar",
            "McNemar discordant pairs & exact odds ratio",
            ResultTabKind::ParametersAndConfidenceIntervals,
            vec![String::from("Estimate"), format!("{}% exact CI", confidence.percent())],
            vec![
                String::from("Discordant pair ratio (b / c)"),
                String::from("Discordant odds ratio (Clopper-Pearson)"),
                String::from("Discordant proportion b / (b + c)"),
                String::from("McNemar chi-square (Yates)"),
                String::from("McNemar exact P value"),
            ],
            vec![
                vec![ResultCell::Text(format!("{} / {}", mc.b, mc.c)), ResultCell::Empty],
                vec![or_cell, opt_ci_cell(mc.discordant_or_ci)],
                vec![disc_prop_cell, ci_cell(mc.discordant_p_ci.0, mc.discordant_p_ci.1)],
                vec![num_cell(mc.chi_square_yates), ResultCell::Empty],
                vec![p_cell(mc.exact_p_value, p_fmt), ResultCell::Empty],
            ],
        ));
    }

    if n_rows == 2 && n_cols == 2 && (cont_spec.compute_odds_and_risk_ratios || cont_spec.compute_sensitivity_specificity) {
        let r2x2 = analyze_2x2_ratios(counts[0][0], counts[0][1], counts[1][0], counts[1][1], confidence)?;
        push_scalar(&mut out, "Odds ratio", None, r2x2.odds_ratio);
        push_scalar(&mut out, "Relative risk", None, r2x2.relative_risk);
        push_scalar(&mut out, "Risk difference", None, r2x2.risk_difference);
        if let Some(nnt_v) = r2x2.nnt_value {
            push_scalar(&mut out, "NNT", None, nnt_v);
        }

        let r_headers = vec![String::from("Estimate"), format!("{}% CI", confidence.percent())];
        let mut r_labels = Vec::new();
        let mut r_rows = Vec::new();
        if cont_spec.compute_odds_and_risk_ratios {
            r_labels.extend([
                String::from("Odds ratio (Baptista-Pike CI)"),
                String::from("Relative risk (Koopman CI)"),
                String::from("Risk difference (Newcombe CI)"),
                String::from("Number needed to treat (NNT)"),
                String::from("NNT magnitude (1 / |RD|)"),
            ]);
            r_rows.push(vec![num_cell(r2x2.odds_ratio), opt_ci_cell(r2x2.or_baptista_pike_ci)]);
            r_rows.push(vec![num_cell(r2x2.relative_risk), opt_ci_cell(r2x2.rr_koopman_ci)]);
            r_rows.push(vec![num_cell(r2x2.risk_difference), opt_ci_cell(r2x2.rd_newcombe_ci)]);
            r_rows.push(vec![ResultCell::Text(r2x2.nnt_point_label.clone()), ResultCell::Text(r2x2.nnt_ci_label.clone())]);
            r_rows.push(vec![opt_num_cell(r2x2.nnt_value), opt_ci_cell(r2x2.nnt_ci_bounds)]);
        }
        if cont_spec.compute_sensitivity_specificity {
            r_labels.extend([
                String::from("Sensitivity (Wilson CI)"),
                String::from("Specificity (Wilson CI)"),
                String::from("PPV (Wilson CI)"),
                String::from("NPV (Wilson CI)"),
            ]);
            r_rows.push(vec![num_cell(r2x2.sensitivity), ci_cell(r2x2.sensitivity_wilson_ci.0, r2x2.sensitivity_wilson_ci.1)]);
            r_rows.push(vec![num_cell(r2x2.specificity), ci_cell(r2x2.specificity_wilson_ci.0, r2x2.specificity_wilson_ci.1)]);
            r_rows.push(vec![num_cell(r2x2.ppv), ci_cell(r2x2.ppv_wilson_ci.0, r2x2.ppv_wilson_ci.1)]);
            r_rows.push(vec![num_cell(r2x2.npv), ci_cell(r2x2.npv_wilson_ci.0, r2x2.npv_wilson_ci.1)]);
        }
        out.tabs.push(build_tab(
            "contingency-ratios",
            "Effect sizes & diagnostic proportions",
            ResultTabKind::ParametersAndConfidenceIntervals,
            r_headers,
            r_labels,
            r_rows,
        ));
    }

    if let Some(br) = &barnard_res {
        let status = if br.used_asymptotic_fallback { ChecklistStatus::Info } else { ChecklistStatus::Pass };
        out.checklist.push(checklist_item(
            "Barnard unconditional nuisance maximization",
            status,
            Some(format!("max P = {:.4} at pi = {:.4}, Z_pooled = {:.4}", br.p_value, br.nuisance_pi, br.statistic)),
            if br.used_asymptotic_fallback {
                "Table total exceeded the exact unconditional grid cap; asymptotic pooled score P value was reported."
            } else {
                "Unconditional exact test maximized the two-binomial null probability across nuisance pi in [0, 1] without fixing column margins."
            },
        ));
    }

    let expected_ok = res.min_expected_count >= 5.0 || (res.n_rows > 2 && res.cells_lt5_fraction <= 0.2 && res.min_expected_count >= 1.0);
    out.checklist.push(checklist_item(
        "Minimum expected cell count",
        if expected_ok { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(format!("min E = {:.2}, {:.1}% cells < 5", res.min_expected_count, 100.0 * res.cells_lt5_fraction)),
        if expected_ok {
            "Expected cell counts satisfy Cochran's guidelines for asymptotic chi-square approximation."
        } else {
            "Small expected counts detected (< 5); large-sample chi-square asymptotic accuracy is reduced."
        },
    ));

    Ok(out)
}

fn push_kappa_outputs(
    out: &mut ResultSheet,
    res: &KappaResult,
    row_label: &str,
    diag_labels: Vec<String>,
    diag_rows: Vec<Vec<ResultCell>>,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    push_scalar(out, "kappa", None, res.kappa);
    push_scalar(out, "se", None, res.se);
    push_scalar(out, "z", None, res.z_statistic);
    push_scalar(out, "p_value", None, res.p_value);
    push_scalar(out, "ci_lower", None, res.ci.0);
    push_scalar(out, "ci_upper", None, res.ci.1);
    push_scalar(out, "observed_agreement", None, res.observed_agreement);
    push_scalar(out, "expected_agreement", None, res.expected_agreement);

    out.tabs.push(build_tab(
        "contingency-kappa",
        "Inter-rater agreement (kappa)",
        ResultTabKind::MainSummary,
        vec![String::from("Kappa"), String::from("ASE (SE)"), format!("{}% CI", confidence.percent()), String::from("Z"), String::from("P value")],
        vec![row_label.to_owned()],
        vec![vec![num_cell(res.kappa), num_cell(res.se), ci_cell(res.ci.0, res.ci.1), num_cell(res.z_statistic), p_cell(res.p_value, p_fmt)]],
    ));

    out.tabs.push(build_tab(
        "contingency-kappa-diagnostics",
        "Agreement decomposition diagnostics",
        ResultTabKind::Diagnostics,
        vec![String::from("Value")],
        diag_labels,
        diag_rows,
    ));
}

fn run_cohen_kappa_sheet(
    counts: &[Vec<u64>],
    weighting: KappaWeighting,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
    mut out: ResultSheet,
) -> Result<ResultSheet, StatsError> {
    let res = cohens_kappa(counts, weighting, confidence)?;
    let k = counts.len();
    let (_, _, total) = checked_table_margins(counts)?;
    let label = match weighting {
        KappaWeighting::Unweighted => "Cohen's kappa (unweighted)",
        KappaWeighting::Linear => "Cohen's kappa (linear weighted)",
        KappaWeighting::Quadratic => "Cohen's kappa (quadratic weighted)",
    };
    push_kappa_outputs(
        &mut out,
        &res,
        label,
        vec![
            String::from("Observed agreement (P_o)"),
            String::from("Chance-expected agreement (P_e)"),
            String::from("Chance-corrected denominator (1 - P_e)"),
            String::from("Subjects / total paired ratings (N)"),
            String::from("Categories (K)"),
        ],
        vec![
            vec![num_cell(res.observed_agreement)],
            vec![num_cell(res.expected_agreement)],
            vec![num_cell(1.0 - res.expected_agreement)],
            vec![num_cell(total as f64)],
            vec![num_cell(k as f64)],
        ],
        confidence,
        p_fmt,
    );
    let excess_ok = (1.0 - res.expected_agreement) > 0.05;
    out.checklist.push(checklist_item(
        "Marginal agreement & category balance",
        if excess_ok && total >= 10 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("N = {total}, K = {k}, P_o = {:.4}, P_e = {:.4}", res.observed_agreement, res.expected_agreement)),
        if excess_ok {
            "Cohen's kappa compares observed concordance against marginal chance agreement on square K x K classifications."
        } else {
            "High marginal concentration yields P_e near 1.0 (kappa paradox regime), limiting sensitivity to discordant pairs."
        },
    ));
    Ok(out)
}

fn run_fleiss_kappa_sheet(
    counts: &[Vec<u64>],
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
    mut out: ResultSheet,
) -> Result<ResultSheet, StatsError> {
    let res = fleiss_kappa(counts, confidence)?;
    let n = counts.len();
    let k = counts.first().map_or(0, Vec::len);
    let m = counts.first().map_or(Ok(0), |r| checked_sum_u64(r, "fleiss_raters_per_subject"))?;
    push_kappa_outputs(
        &mut out,
        &res,
        "Fleiss' multi-rater kappa",
        vec![
            String::from("Mean observed agreement (P_bar)"),
            String::from("Chance-expected agreement (P_e)"),
            String::from("Chance-corrected denominator (1 - P_e)"),
            String::from("Subjects (N)"),
            String::from("Categories (K)"),
            String::from("Raters per subject (m)"),
        ],
        vec![
            vec![num_cell(res.observed_agreement)],
            vec![num_cell(res.expected_agreement)],
            vec![num_cell(1.0 - res.expected_agreement)],
            vec![num_cell(n as f64)],
            vec![num_cell(k as f64)],
            vec![num_cell(m as f64)],
        ],
        confidence,
        p_fmt,
    );
    let excess_ok = (1.0 - res.expected_agreement) > 0.05;
    out.checklist.push(checklist_item(
        "Multi-rater design & chance agreement",
        if excess_ok && n >= 5 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("N = {n} subjects, m = {m} raters, K = {k}, P_bar = {:.4}, P_e = {:.4}", res.observed_agreement, res.expected_agreement)),
        if excess_ok {
            "Fleiss' kappa evaluates chance-corrected nominal agreement with equal rater counts per subject."
        } else {
            "Category prevalence concentration drives P_e near 1.0, amplifying sensitivity to small disagreement shifts."
        },
    ));
    Ok(out)
}

fn extract_exact_binomial_counts(sheet: &DataSheet, spec: &AnalysisSpec) -> Result<(u64, u64), StatsError> {
    match &sheet.archetype {
        TableArchetype::PartsOfWhole(pow) => {
            let slices = pow.fractions_for_series(0)?;
            if slices.len() != 2 {
                return Err(StatsError::DimensionMismatch { expected: 2, actual: slices.len() });
            }
            let as_count = |v: f64| -> Result<u64, StatsError> {
                if !v.is_finite() || v < 0.0 || v.fract() != 0.0 || v > (u64::MAX as f64) {
                    return Err(StatsError::InvalidParameter { name: "binomial_count", value: v });
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Ok(v as u64)
            };
            let k = as_count(slices[0].value)?;
            let f = as_count(slices[1].value)?;
            let n = k.checked_add(f).ok_or(StatsError::InvalidParameter { name: "exact_binomial_total", value: f64::INFINITY })?;
            if n == 0 {
                return Err(StatsError::EmptyInput);
            }
            Ok((k, n))
        }
        TableArchetype::Contingency(table) => {
            table.validate_counts()?;
            let n_rows = table.row_count();
            let n_cols = table.data_sets.len();
            let (k, f) = match (n_rows, n_cols) {
                (1, 2) => (table.count_at(0, 0)?.unwrap_or(0), table.count_at(0, 1)?.unwrap_or(0)),
                (2, 1) => (table.count_at(0, 0)?.unwrap_or(0), table.count_at(1, 0)?.unwrap_or(0)),
                _ => return Err(StatsError::DimensionMismatch { expected: 2, actual: n_rows * n_cols }),
            };
            let n = k.checked_add(f).ok_or(StatsError::InvalidParameter { name: "exact_binomial_total", value: f64::INFINITY })?;
            if n == 0 {
                return Err(StatsError::EmptyInput);
            }
            Ok((k, n))
        }
        TableArchetype::Column(col_table) => {
            if !matches!(col_table.subcolumn_mode, SubcolumnMode::Single | SubcolumnMode::RawReplicates(1)) {
                return Err(StatsError::RequiresRawReplicates);
            }
            let col_idx = spec.selected_data_sets.first().copied().unwrap_or(0);
            let Some(ds) = col_table.data_sets.get(col_idx) else {
                return Err(StatsError::MissingColumn { index: col_idx });
            };
            let values = col_table.active_column_values(col_idx);
            if values.is_empty() {
                return Err(StatsError::EmptyInput);
            }
            let _ = ds;
            let mut k = 0_u64;
            for v in &values {
                if *v == 1.0 {
                    k += 1;
                } else if *v != 0.0 {
                    return Err(StatsError::InvalidParameter { name: "binary_value", value: *v });
                }
            }
            Ok((k, values.len() as u64))
        }
        _ => Err(StatsError::IncompatibleTable { analysis: "exact binomial test", actual: sheet.archetype.kind() }),
    }
}

fn run_exact_binomial_analysis(
    sheet: &DataSheet,
    spec: &AnalysisSpec,
    cont_spec: &ContingencySpec,
    mut out: ResultSheet,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> Result<ResultSheet, StatsError> {
    let (k, n) = extract_exact_binomial_counts(sheet, spec)?;
    let p0 = cont_spec.null_proportion.unwrap_or(0.5);
    let res = exact_binomial_test(k, n, p0, confidence)?;

    push_scalar(&mut out, "proportion", None, res.proportion);
    push_scalar(&mut out, "p_value", None, res.p_value_sterne);
    push_scalar(&mut out, "p_value_doubled_tail", None, res.p_value_doubled_tail);
    push_scalar(&mut out, "p_value_lower", None, res.p_value_lower);
    push_scalar(&mut out, "p_value_upper", None, res.p_value_upper);
    push_scalar(&mut out, "null_proportion", None, res.null_proportion);
    push_scalar(&mut out, "clopper_pearson_lower", None, res.clopper_pearson_ci.0);
    push_scalar(&mut out, "clopper_pearson_upper", None, res.clopper_pearson_ci.1);
    push_scalar(&mut out, "wilson_lower", None, res.wilson_ci.0);
    push_scalar(&mut out, "wilson_upper", None, res.wilson_ci.1);
    push_scalar(&mut out, "agresti_coull_lower", None, res.agresti_coull_ci.0);
    push_scalar(&mut out, "agresti_coull_upper", None, res.agresti_coull_ci.1);
    push_scalar(&mut out, "successes", None, res.successes as f64);
    push_scalar(&mut out, "total", None, res.total as f64);

    out.tabs.push(build_tab(
        "exact-binomial-main",
        "Exact binomial test",
        ResultTabKind::MainSummary,
        vec![
            String::from("Successes (k)"),
            String::from("Trials (n)"),
            String::from("Observed p"),
            String::from("Null p0"),
            String::from("Exact P (Sterne)"),
            String::from("Exact P (doubled tail)"),
            String::from("Lower P(X <= k)"),
            String::from("Upper P(X >= k)"),
        ],
        vec![String::from("Binomial proportion vs null")],
        vec![vec![
            num_cell(res.successes as f64),
            num_cell(res.total as f64),
            num_cell(res.proportion),
            num_cell(res.null_proportion),
            p_cell(res.p_value_sterne, p_fmt),
            p_cell(res.p_value_doubled_tail, p_fmt),
            p_cell(res.p_value_lower, p_fmt),
            p_cell(res.p_value_upper, p_fmt),
        ]],
    ));

    out.tabs.push(build_tab(
        "exact-binomial-cis",
        "Binomial proportion confidence intervals",
        ResultTabKind::ParametersAndConfidenceIntervals,
        vec![String::from("Observed p"), format!("{}% CI", confidence.percent()), String::from("Lower bound"), String::from("Upper bound")],
        vec![String::from("Clopper-Pearson (exact)"), String::from("Wilson score"), String::from("Agresti-Coull")],
        vec![
            vec![
                num_cell(res.proportion),
                ci_cell(res.clopper_pearson_ci.0, res.clopper_pearson_ci.1),
                num_cell(res.clopper_pearson_ci.0),
                num_cell(res.clopper_pearson_ci.1),
            ],
            vec![num_cell(res.proportion), ci_cell(res.wilson_ci.0, res.wilson_ci.1), num_cell(res.wilson_ci.0), num_cell(res.wilson_ci.1)],
            vec![
                num_cell(res.proportion),
                ci_cell(res.agresti_coull_ci.0, res.agresti_coull_ci.1),
                num_cell(res.agresti_coull_ci.0),
                num_cell(res.agresti_coull_ci.1),
            ],
        ],
    ));

    let np0 = (n as f64) * p0;
    let nq0 = (n as f64) * (1.0 - p0);
    let min_expected = np0.min(nq0);
    out.tabs.push(build_tab(
        "exact-binomial-diagnostics",
        "Binomial model diagnostics",
        ResultTabKind::Diagnostics,
        vec![String::from("Value")],
        vec![
            String::from("Expected successes under H0 (n * p0)"),
            String::from("Expected failures under H0 (n * (1 - p0))"),
            String::from("Minimum expected cell count min(n*p0, n*(1-p0))"),
        ],
        vec![vec![num_cell(np0)], vec![num_cell(nq0)], vec![num_cell(min_expected)]],
    ));

    out.checklist.push(checklist_item(
        "Exact discrete binomial enumeration",
        if min_expected >= 5.0 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("k = {k}, n = {n}, p0 = {p0:.4}, min(np0, nq0) = {min_expected:.2}")),
        if min_expected >= 5.0 {
            "Exact binomial probabilities were summed directly under Binomial(n, p0); normal-approximation conditions are also met."
        } else {
            "Small expected count min(np0, nq0) < 5 causes normal approximations to diverge from discrete binomial tails; exact binomial enumeration evaluates the discrete distribution directly."
        },
    ));

    Ok(out)
}

fn run_parts_of_whole_gof(
    pow: &lumencraft_doc::PartsOfWholeTable,
    cont_spec: &ContingencySpec,
    out: &mut ResultSheet,
    p_fmt: PValueFormat,
) -> Result<ResultSheet, StatsError> {
    let slices = pow.fractions_for_series(0)?;
    let k = slices.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "parts-of-whole goodness-of-fit", required: 2, actual: k });
    }
    let total: f64 = slices.iter().map(|s| s.value).sum();
    if total <= 0.0 {
        return Err(StatsError::EmptyInput);
    }

    let (proportions, is_custom) = if let Some(weights) = &cont_spec.expected_proportions {
        if weights.len() != k {
            return Err(StatsError::DimensionMismatch { expected: k, actual: weights.len() });
        }
        if let Some(&bad) = weights.iter().find(|&&w| !w.is_finite() || w < 0.0) {
            return Err(StatsError::InvalidParameter { name: "expected_proportions", value: bad });
        }
        let sum: f64 = weights.iter().sum();
        if !sum.is_finite() || sum <= 0.0 {
            return Err(StatsError::InvalidParameter { name: "expected_proportions_sum", value: sum });
        }
        (weights.iter().map(|&w| w / sum).collect::<Vec<_>>(), true)
    } else {
        (vec![1.0 / (k as f64); k], false)
    };

    let mut chi2 = 0.0_f64;
    let mut g2 = 0.0_f64;
    let mut min_expected = f64::INFINITY;
    let mut lt5_count = 0_usize;
    let mut cat_labels = Vec::with_capacity(k);
    let mut cat_rows = Vec::with_capacity(k);

    for (idx, (slice, &p_exp)) in slices.iter().zip(proportions.iter()).enumerate() {
        let obs = slice.value;
        let exp = total * p_exp;
        if exp <= 0.0 && obs > 0.0 {
            return Err(StatsError::InvalidParameter { name: "expected_proportions_zero_for_nonzero_observed", value: obs });
        }
        min_expected = min_expected.min(exp);
        if exp < 5.0 {
            lt5_count += 1;
        }
        let pearson_resid = if exp > 0.0 {
            let diff = obs - exp;
            chi2 += (diff * diff) / exp;
            if obs > 0.0 {
                g2 += 2.0 * obs * (obs / exp).ln();
            }
            diff / exp.sqrt()
        } else {
            0.0
        };
        let label = if slice.label.trim().is_empty() { format!("Category {}", idx + 1) } else { slice.label.clone() };
        cat_labels.push(label);
        cat_rows.push(vec![num_cell(obs), num_cell(obs / total), num_cell(p_exp), num_cell(exp), num_cell(pearson_resid)]);
    }

    let df = (k - 1) as f64;
    let p_val = chi2_sf(chi2, df)?;
    let g2_p = chi2_sf(g2, df)?;
    push_scalar(out, "Goodness-of-fit chi-square", None, chi2);
    push_scalar(out, "Likelihood-ratio G-squared", None, g2);
    push_scalar(out, "P value", None, p_val);
    push_scalar(out, "G-squared P value", None, g2_p);

    let title = if is_custom { "Chi-square goodness of fit (custom expected proportions)" } else { "Chi-square goodness of fit (uniform expected)" };
    let headers = vec![String::from("Statistic"), String::from("df"), String::from("P value")];
    let rows = vec![vec![num_cell(chi2), num_cell(df), p_cell(p_val, p_fmt)], vec![num_cell(g2), num_cell(df), p_cell(g2_p, p_fmt)]];
    out.tabs.push(build_tab(
        "parts-gof",
        title,
        ResultTabKind::MainSummary,
        headers,
        vec![String::from("Pearson chi-square"), String::from("Likelihood-ratio G2")],
        rows,
    ));

    out.tabs.push(build_tab(
        "parts-gof-categories",
        "Category observed vs expected proportions",
        ResultTabKind::ParametersAndConfidenceIntervals,
        vec![
            String::from("Observed"),
            String::from("Observed proportion"),
            String::from("Expected proportion"),
            String::from("Expected count"),
            String::from("Pearson residual"),
        ],
        cat_labels,
        cat_rows,
    ));

    let expected_ok = min_expected >= 5.0 || (k > 2 && (lt5_count as f64) / (k as f64) <= 0.2 && min_expected >= 1.0);
    out.checklist.push(checklist_item(
        "Minimum expected category count",
        if expected_ok { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(format!("min E = {min_expected:.2}, {lt5_count}/{k} categories < 5")),
        if expected_ok {
            "Expected category counts satisfy guidelines for asymptotic chi-square goodness-of-fit approximation."
        } else {
            "Small expected category counts (< 5) reduce asymptotic accuracy of Pearson chi-square and G2 tests."
        },
    ));

    Ok(out.clone())
}

#[cfg(test)]
mod tests {
    use lumencraft_math::ConfidenceLevel;
    use lumencraft_math::dist::norm_two_sided_isf;

    use super::{
        StatsError, checked_table_margins, fisher_exact_2x2, koopman_chi2_score, koopman_rr_ci, koopman_solve_lower, koopman_solve_lower_c_zero,
        koopman_solve_upper, koopman_solve_upper_a_zero, noncentral_hypergeom_p,
    };

    #[test]
    fn koopman_extreme_boundary_cases_bracket_and_solve_score_to_zero() {
        let conf = ConfidenceLevel::DEFAULT_95;
        let z2 = norm_two_sided_isf(conf.alpha()).unwrap().powi(2);

        assert!(koopman_rr_ci(0, 10, 0, 10, conf).is_none());
        assert!(koopman_rr_ci(0, 0, 5, 5, conf).is_none());
        assert!(koopman_rr_ci(5, 5, 0, 0, conf).is_none());

        let cases = [
            (14_u64, 6_u64, 5_u64, 15_u64),
            (10, 0, 5, 5),
            (5, 5, 10, 0),
            (10, 0, 10, 0),
            (1, 0, 1, 0),
            (0, 10, 5, 5),
            (0, 10_000_000, 5, 5),
            (0, 10, 10, 0),
            (9, 1, 0, 10),
            (10, 0, 0, 10),
            (1, 999, 0, 10),
            (1, 10_000_000_000, 1, 1),
            (1, 1, 1, 10_000_000_000),
            (1, 10_000_000_000, 1, 10_000_000_000),
        ];

        for (a, b, c, d) in cases {
            let (lo, hi) = koopman_rr_ci(a, b, c, d, conf).unwrap();
            assert!(lo >= 0.0 && lo < hi, "invalid interval ({lo}, {hi}) for ({a},{b},{c},{d})");
            if a == 0 {
                assert_eq!(lo, 0.0);
            } else {
                assert!(lo.is_finite() && lo > 0.0);
                let err_lo = koopman_chi2_score(a, b, c, d, z2, lo).abs();
                assert!(err_lo < 1e-7, "non-zero score at lo={lo}: {err_lo} for ({a},{b},{c},{d})");
            }
            if c == 0 {
                assert!(hi.is_infinite());
            } else {
                assert!(hi.is_finite());
                let err_hi = koopman_chi2_score(a, b, c, d, z2, hi).abs();
                assert!(err_hi < 1e-7, "non-zero score at hi={hi}: {err_hi} for ({a},{b},{c},{d})");
            }
            if a > 0 && c > 0 {
                let n1 = (a + b) as f64;
                let n2 = (c + d) as f64;
                let rr_pt = ((a as f64) / n1) / ((c as f64) / n2);
                assert!(lo < rr_pt && rr_pt < hi, "point estimate {rr_pt} outside ({lo}, {hi})");
            }
        }

        let (lo_all, hi_all) = koopman_rr_ci(10, 0, 10, 0, conf).unwrap();
        assert!((lo_all - 10.0 / (10.0 + z2)).abs() < 1e-9);
        assert!((hi_all - (10.0 + z2) / 10.0).abs() < 1e-9);

        let (lo_sep, hi_sep) = koopman_rr_ci(10, 0, 0, 10, conf).unwrap();
        assert!((lo_sep - (1.0 + 10.0 / z2)).abs() < 1e-9);
        assert!(hi_sep.is_infinite());
    }

    #[test]
    fn koopman_published_reference_and_zero_cell_values() {
        let conf_95 = ConfidenceLevel::DEFAULT_95;
        let conf_99 = ConfidenceLevel::PERCENT_99;
        let conf_9999 = ConfidenceLevel::from_percent(99.99).unwrap();

        // Koopman (1984), Biometrics 40:513-517, p. 516 example: x1=36/40, x2=16/80 (RR = 4.5).
        let (lo_36_95, hi_36_95) = koopman_rr_ci(36, 4, 16, 64, conf_95).unwrap();
        assert!((lo_36_95 - 2.939_569_192_214_994).abs() < 1e-9, "lo={lo_36_95}");
        assert!((hi_36_95 - 7.152_223_434_866_03).abs() < 1e-9, "hi={hi_36_95}");

        let (lo_36_99, hi_36_99) = koopman_rr_ci(36, 4, 16, 64, conf_99).unwrap();
        assert!((lo_36_99 - 2.598_049_751_056_122).abs() < 1e-9, "lo99={lo_36_99}");
        assert!((hi_36_99 - 8.284_221_357_270_2).abs() < 1e-9, "hi99={hi_36_99}");

        let (lo_16, hi_16) = koopman_rr_ci(16, 4, 2, 18, conf_95).unwrap();
        assert!((lo_16 - 2.565_778_683_398_19).abs() < 1e-9, "lo16={lo_16}");
        assert!((hi_16 - 29.093_901_260_350_407).abs() < 1e-8, "hi16={hi_16}");

        // Zero-cell edge cases: a = 0 gives [0, upper], c = 0 gives [lower, +inf), symmetric under group swap.
        let a_zero_cases =
            [(0_u64, 10_u64, 5_u64, 5_u64, 0.612_860_577_425_767), (0, 15, 4, 11, 0.841_877_258_244_93), (0, 10, 10, 0, 0.277_532_799_862_889)];
        for (a, b, c, d, expected_hi) in a_zero_cases {
            let (lo, hi) = koopman_rr_ci(a, b, c, d, conf_95).unwrap();
            assert_eq!(lo, 0.0);
            assert!((hi - expected_hi).abs() < 1e-9, "a=0 hi={hi} expected={expected_hi}");

            let (swap_lo, swap_hi) = koopman_rr_ci(c, d, a, b, conf_95).unwrap();
            assert!((swap_lo - 1.0 / expected_hi).abs() < 1e-9, "swap_lo={swap_lo} expected={}", 1.0 / expected_hi);
            assert!(swap_hi.is_infinite());
        }

        let (lo_9999, hi_9999) = koopman_rr_ci(0, 10, 10, 0, conf_9999).unwrap();
        assert_eq!(lo_9999, 0.0);
        assert!((hi_9999 - 0.756_835_261_331_17).abs() < 1e-9, "hi9999={hi_9999}");

        let (lo_c0_a, hi_c0_a) = koopman_rr_ci(5, 5, 0, 10, conf_95).unwrap();
        assert!((lo_c0_a - 1.631_692_487_384_906).abs() < 1e-9);
        assert!(hi_c0_a.is_infinite());

        let (lo_c0_b, hi_c0_b) = koopman_rr_ci(9, 1, 0, 10, conf_95).unwrap();
        assert!((lo_c0_b - 3.186_579_700_647_302).abs() < 1e-9);
        assert!(hi_c0_b.is_infinite());
    }

    #[test]
    fn koopman_solvers_return_none_on_non_bracketing_scores() {
        assert!((koopman_solve_lower(&|phi| 0.5 - phi, 1.0).unwrap() - 0.5).abs() < 1e-9);
        assert!((koopman_solve_upper(&|phi| phi - 2.5, 1.0).unwrap() - 2.5).abs() < 1e-9);
        assert!((koopman_solve_lower_c_zero(&|phi| 0.25 - phi).unwrap() - 0.25).abs() < 1e-9);
        assert!((koopman_solve_upper_a_zero(&|phi| phi - 4.0).unwrap() - 4.0).abs() < 1e-9);

        for rr_bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(koopman_solve_lower(&|phi| 0.5 - phi, rr_bad).is_none());
            assert!(koopman_solve_upper(&|phi| phi - 2.5, rr_bad).is_none());
        }

        let always_neg = |_: f64| -1.0;
        let always_pos = |_: f64| 1.0;
        let unreachable_lo = |phi: f64| if phi < 1e-45 { 1.0 } else { -1.0 };
        let unreachable_hi = |phi: f64| if phi > 1e45 { 1.0 } else { -1.0 };

        assert!(koopman_solve_lower(&always_neg, 1.0).is_none());
        assert!(koopman_solve_lower(&always_pos, 1.0).is_none());
        assert!(koopman_solve_lower(&unreachable_lo, 1.0).is_none());

        assert!(koopman_solve_upper(&always_neg, 1.0).is_none());
        assert!(koopman_solve_upper(&always_pos, 1.0).is_none());
        assert!(koopman_solve_upper(&unreachable_hi, 1.0).is_none());

        assert!(koopman_solve_lower_c_zero(&always_neg).is_none());
        assert!(koopman_solve_lower_c_zero(&always_pos).is_none());
        assert!(koopman_solve_lower_c_zero(&unreachable_lo).is_none());

        assert!(koopman_solve_upper_a_zero(&always_neg).is_none());
        assert!(koopman_solve_upper_a_zero(&always_pos).is_none());
        assert!(koopman_solve_upper_a_zero(&unreachable_hi).is_none());
    }

    #[test]
    fn koopman_solvers_return_none_on_non_finite_and_non_convergent_scores() {
        let lower_nan_mid = |phi: f64| {
            if phi < 0.4 {
                1.0
            } else if phi < 0.6 {
                f64::NAN
            } else {
                -1.0
            }
        };
        let upper_neginf_mid = |phi: f64| {
            if phi > 3.0 {
                1.0
            } else if phi > 1.5 {
                f64::NEG_INFINITY
            } else {
                -1.0
            }
        };
        let nan_inside_lo_bracket = |phi: f64| {
            if phi <= 0.5 {
                1.0
            } else if phi >= 1.0 {
                -1.0
            } else {
                f64::NAN
            }
        };
        let nan_inside_hi_bracket = |phi: f64| {
            if phi <= 1.0 {
                -1.0
            } else if phi >= 2.0 {
                1.0
            } else {
                f64::NAN
            }
        };

        assert!(koopman_solve_lower(&|_| f64::NAN, 1.0).is_none());
        assert!(koopman_solve_lower(&|phi| if phi < 0.6 { f64::INFINITY } else { -1.0 }, 1.0).is_none());
        assert!(koopman_solve_lower(&lower_nan_mid, 1.0).is_none());

        assert!(koopman_solve_upper(&|_| f64::NAN, 1.0).is_none());
        assert!(koopman_solve_upper(&|phi| if phi > 1.5 { f64::INFINITY } else { -1.0 }, 1.0).is_none());
        assert!(koopman_solve_upper(&upper_neginf_mid, 1.0).is_none());

        assert!(koopman_solve_lower_c_zero(&|phi| if phi >= 1.0 { f64::NAN } else { -1.0 }).is_none());
        assert!(koopman_solve_lower_c_zero(&|phi| if phi <= 0.6 { f64::INFINITY } else { -1.0 }).is_none());

        assert!(koopman_solve_upper_a_zero(&|phi| if phi <= 1.0 { f64::NAN } else { -1.0 }).is_none());
        assert!(koopman_solve_upper_a_zero(&|phi| if phi >= 1.5 { f64::INFINITY } else { -1.0 }).is_none());

        assert!(koopman_solve_lower(&nan_inside_lo_bracket, 1.0).is_none());
        assert!(koopman_solve_upper(&nan_inside_hi_bracket, 1.0).is_none());
        assert!(koopman_solve_lower_c_zero(&nan_inside_lo_bracket).is_none());
        assert!(koopman_solve_upper_a_zero(&nan_inside_hi_bracket).is_none());
    }

    #[test]
    fn noncentral_hypergeom_and_table_margin_overflow_guards() {
        // Total overflow, total cap, span cap, invalid col sum, out-of-range cell, and bad OR.
        assert!(matches!(
            noncentral_hypergeom_p(1, u64::MAX, 1, 1, 1.0),
            Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_total", .. })
        ));
        assert!(matches!(
            noncentral_hypergeom_p(1, 3_000_000, 3_000_000, 2, 1.0),
            Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_max_total", .. })
        ));
        assert!(matches!(
            noncentral_hypergeom_p(50_000, 200_000, 200_000, 200_000, 1.0),
            Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_max_span", .. })
        ));
        assert!(matches!(noncentral_hypergeom_p(1, 5, 5, 11, 1.0), Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_col1", .. })));
        assert!(matches!(noncentral_hypergeom_p(6, 5, 5, 6, 1.0), Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_cell", .. })));
        assert!(matches!(noncentral_hypergeom_p(2, 5, 5, 5, f64::NAN), Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_or", .. })));
        assert!(matches!(noncentral_hypergeom_p(2, 5, 5, 5, -1.0), Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_or", .. })));
        // Valid small case stays close to central hypergeometric p at OR = 1.0.
        let p_unit = noncentral_hypergeom_p(8, 10, 10, 10, 1.0).unwrap();
        let p_fisher = fisher_exact_2x2(8, 2, 2, 8).unwrap();
        assert!((p_unit - p_fisher).abs() < 1e-12);

        // Checked table margins reject row overflow, col overflow, table total overflow, and ragged rows.
        assert!(matches!(
            checked_table_margins(&[vec![u64::MAX, 1], vec![0, 0]]),
            Err(StatsError::InvalidParameter { name: "contingency_row_total", .. })
        ));
        assert!(matches!(
            checked_table_margins(&[vec![1_u64 << 63, 0], vec![1_u64 << 63, 0]]),
            Err(StatsError::InvalidParameter { name: "contingency_table_total", .. })
        ));
        assert!(matches!(
            checked_table_margins(&[vec![1_u64 << 63, 0], vec![0, 1_u64 << 63]]),
            Err(StatsError::InvalidParameter { name: "contingency_table_total", .. })
        ));
        assert!(matches!(
            checked_table_margins(&[vec![1, 2], vec![3]]),
            Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: 2, actual: 1 }))
        ));
    }
}
