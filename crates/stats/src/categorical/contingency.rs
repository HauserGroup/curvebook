//! Contingency table hypothesis tests: Pearson chi-square (with Yates continuity option),
//! likelihood-ratio `G^2`, Fisher's exact test (`2 x 2` and `R x C` Freeman–Halton Monte Carlo),
//! Barnard's exact unconditional test, Cochran–Armitage trend test, McNemar paired test, and
//! phi / Cramér's `V` with exact noncentral chi-square CIs.
//!
//! References:
//! - Pearson, K. (1900). On the criterion that a given system of deviations from the probable in the
//!   case of a correlated system of variables is such that it can be reasonably supposed to have
//!   arisen from random sampling. *Philosophical Magazine*, 50(302), 157-175.
//!   `doi:10.1080/14786440009463897`.
//! - Yates, F. (1934). Contingency tables involving small numbers and the chi-squared test.
//!   *Supplement to the Journal of the Royal Statistical Society*, 1(2), 217-235. `doi:10.2307/2983604`.
//! - Wilks, S. S. (1938). The large-sample distribution of the likelihood ratio for testing composite
//!   hypotheses. *Annals of Mathematical Statistics*, 9(1), 60-62. `doi:10.1214/aoms/1177732360`.
//! - Barnard, G. A. (1947). Significance tests for 2 x 2 tables. *Biometrika*, 34(1-2), 123-138.
//! - Suissa, S., & Shuster, J. J. (1985). Exact unconditional sample sizes for the 2 x 2 binomial
//!   trial. *Journal of the Royal Statistical Society A*, 148(4), 317-327.
//! - Boschloo, R. D. (1970). Raised conditional level of significance for the 2 x 2-table when
//!   testing the equality of two probabilities. *Stat. Neerl.*, 24(1), 1-9.
//! - Cochran, W. G. (1954). Some methods for strengthening the common chi-squared tests.
//!   *Biometrics*, 10(4), 417-451. `doi:10.2307/3001616`.
//! - Armitage, P. (1955). Tests for linear trends in proportions and frequencies. *Biometrics*,
//!   11(3), 375-386. `doi:10.2307/3001775`.
//! - McNemar, Q. (1947). Note on the sampling error of the difference between correlated proportions
//!   or percentages. *Psychometrika*, 12(2), 153-157. `doi:10.1007/bf02295996`.
//! - Edwards, A. L. (1948). Note on the "correction for continuity" in testing the significance of the
//!   difference between correlated proportions. *Psychometrika*, 13(3), 185-187.
//!   `doi:10.1007/bf02289261`.
//! - Sterne, T. E. (1954). Some remarks on confidence or fiducial limits. *Biometrika*, 41(1-2),
//!   275-278. `doi:10.1093/biomet/41.1-2.275`.
//! - Agresti, A. (2013). *Categorical Data Analysis* (3rd ed.). Wiley.

use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{chi2_sf, noncentral_chi2_ci, norm_sf};
use curvebook_math::rng::DeterministicRng;
use curvebook_math::special::ln_factorial;
use curvebook_math::sum::NeumaierSum;

use super::binomial::clopper_pearson_ci;
use super::types::{BarnardExactResult, BarnardOrdering, ContingencyResult, McNemarResult};
use crate::error::StatsError;

const FISHER_2X2_MAX_TOTAL: u64 = 5_000_000;
const FISHER_RXC_MAX_TOTAL: u64 = 50_000;
const FISHER_RXC_ANALYZE_MAX_TOTAL: u64 = 20_000;
const MCNEMAR_MAX_TOTAL: u64 = 5_000_000;

pub(super) const BARNARD_MAX_TOTAL: u64 = 2_000;
pub(super) const BARNARD_MAX_GRID_CELLS: u64 = 250_000;
const BARNARD_MAX_BOSCHLOO_TOTAL: u64 = 600;
const BARNARD_MAX_CSM_CELLS: u64 = 1_600;
const BOSCHLOO_LOG_REL_TOL: f64 = 1.0e-10;

pub(super) fn checked_sum_u64(vals: &[u64], name: &'static str) -> Result<u64, StatsError> {
    vals.iter().try_fold(0_u64, |acc, &v| acc.checked_add(v)).ok_or(StatsError::InvalidParameter { name, value: f64::INFINITY })
}

pub(super) fn checked_table_margins(counts: &[Vec<u64>]) -> Result<(Vec<u64>, Vec<u64>, u64), StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    let mut r_sums = Vec::with_capacity(n_rows);
    let mut c_sums = vec![0_u64; n_cols];
    let mut total = 0_u64;
    for row in counts {
        if row.len() != n_cols {
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n_cols, actual: row.len() }));
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
///
/// Sums hypergeometric point probabilities satisfying `P(X = x) <= P(X = a) * (1 + 10^-7)`
/// (Agresti, 2013, *Categorical Data Analysis*, 3rd ed., §3.5.1), using a relative probability
/// tolerance in log space so symmetric mirror tables at large `N` are not excluded by floating-point
/// rounding of `ln(n!)`.
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
    let log_rel_tol = (1.0_f64 + 1e-7).ln();

    let mut tail_sum = NeumaierSum::new();
    let mut total_sum = NeumaierSum::new();
    let mut all_included = true;
    for x in lo..=hi {
        let bx = r1 - x;
        let cx = c1 - x;
        let dx = r2 - cx;
        let lp = log_const - (ln_factorial(x) + ln_factorial(bx) + ln_factorial(cx) + ln_factorial(dx));
        let px = lp.exp();
        total_sum.add(px);
        if lp <= obs_log + log_rel_tol {
            tail_sum.add(px);
        } else {
            all_included = false;
        }
    }
    if all_included {
        return Ok(1.0);
    }
    let denom = total_sum.total();
    let p_val = if denom > 0.0 { tail_sum.total() / denom } else { tail_sum.total() };
    Ok(p_val.clamp(0.0, 1.0))
}

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
    if (best_p - sym_val).abs() <= 1.0e-12 * best_p && (sym_pi - pi_hint).abs() < (best_pi - pi_hint).abs() {
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

fn boschloo_fisher_slice(s: usize, n1: usize, n2: usize, total: usize, ln_fact: &[f64]) -> Vec<(usize, f64, f64)> {
    let lo = s.saturating_sub(n2);
    let hi = n1.min(s);
    let log_const = ln_fact[n1] + ln_fact[n2] + ln_fact[s] + ln_fact[total - s] - ln_fact[total];
    let mut entries: Vec<(usize, f64, f64)> = Vec::with_capacity(hi - lo + 1);
    for x in lo..=hi {
        let lp = log_const - (ln_fact[x] + ln_fact[n1 - x] + ln_fact[s - x] + ln_fact[n2 - (s - x)]);
        entries.push((x, lp, lp.exp()));
    }
    let mut sorted = entries.clone();
    sorted.sort_by(|u, v| u.1.total_cmp(&v.1));

    let mut out = Vec::with_capacity(entries.len());
    for (x, lpx, px) in entries {
        let cutoff = lpx + BOSCHLOO_LOG_REL_TOL;
        let end = sorted.partition_point(|&(_, lpv, _)| lpv <= cutoff);
        let ln_pf = if end == 0 {
            0.0
        } else {
            let max_lp = sorted[end - 1].1;
            let mut sum = NeumaierSum::new();
            for &(_, lpv, _) in &sorted[..end] {
                sum.add((lpv - max_lp).exp());
            }
            (max_lp + sum.total().ln()).min(0.0)
        };
        out.push((x, px, ln_pf));
    }
    out
}

fn accumulate_boschloo_region(a: usize, n1: usize, c: usize, n2: usize, ln_fact: &[f64]) -> Vec<f64> {
    let total = n1 + n2;
    let s_obs = a + c;
    let obs_slice = boschloo_fisher_slice(s_obs, n1, n2, total, ln_fact);
    let ln_p_obs = obs_slice.iter().find(|&&(x, _, _)| x == a).map_or(0.0, |&(_, _, ln_pf)| ln_pf);
    let ln_thresh = ln_p_obs + BOSCHLOO_LOG_REL_TOL;

    let mut f_s = vec![0.0_f64; total + 1];
    for (s, slot) in f_s.iter_mut().enumerate() {
        let mut slice_sum = NeumaierSum::new();
        for (_, hg_p, ln_pf) in boschloo_fisher_slice(s, n1, n2, total, ln_fact) {
            if ln_pf <= ln_thresh {
                slice_sum.add(hg_p);
            }
        }
        *slot = slice_sum.total();
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
        let val_thresh = best_val * (1.0 + 1.0e-12);
        for (pair, &val) in candidates.iter().zip(scores.iter()) {
            if val <= val_thresh {
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
    let log_rel_tol = (1.0_f64 + 1e-7).ln();
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
        if sim_log <= obs_log + log_rel_tol {
            extreme += 1;
        }
    }
    Ok(((extreme + 1) as f64) / ((n_sims + 1) as f64))
}

pub(super) struct ReducedContingencyTable {
    pub(super) counts: Vec<Vec<u64>>,
    pub(super) r_sums: Vec<u64>,
    pub(super) c_sums: Vec<u64>,
    pub(super) total_count: u64,
    pub(super) n_rows: usize,
    pub(super) n_cols: usize,
    pub(super) dropped_rows: usize,
    pub(super) dropped_cols: usize,
}

pub(super) fn reduce_zero_margin_table(counts: &[Vec<u64>]) -> Result<ReducedContingencyTable, StatsError> {
    let orig_rows = counts.len();
    let orig_cols = counts.first().map_or(0, Vec::len);
    if orig_rows == 0 || orig_cols == 0 {
        return Err(StatsError::EmptyInput);
    }
    let (r_sums_all, c_sums_all, total_count) = checked_table_margins(counts)?;
    if total_count == 0 {
        return Err(StatsError::EmptyInput);
    }

    let active_rows: Vec<usize> = (0..orig_rows).filter(|&r| r_sums_all[r] > 0).collect();
    let active_cols: Vec<usize> = (0..orig_cols).filter(|&c| c_sums_all[c] > 0).collect();
    let n_rows = active_rows.len();
    let n_cols = active_cols.len();
    if n_rows < 2 || n_cols < 2 {
        return Err(StatsError::InsufficientSampleSize {
            context: "contingency table (after dropping all-zero rows/columns)",
            required: 2,
            actual: n_rows.min(n_cols),
        });
    }

    let mut reduced = Vec::with_capacity(n_rows);
    for &r in &active_rows {
        let mut row = Vec::with_capacity(n_cols);
        for &c in &active_cols {
            row.push(counts[r][c]);
        }
        reduced.push(row);
    }
    let r_sums: Vec<u64> = active_rows.iter().map(|&r| r_sums_all[r]).collect();
    let c_sums: Vec<u64> = active_cols.iter().map(|&c| c_sums_all[c]).collect();
    Ok(ReducedContingencyTable {
        counts: reduced,
        r_sums,
        c_sums,
        total_count,
        n_rows,
        n_cols,
        dropped_rows: orig_rows - n_rows,
        dropped_cols: orig_cols - n_cols,
    })
}

/// Pearson (1900) `X^2`, optionally with the Yates (1934) `2 x 2` continuity correction applied as
/// `max(|O - E| - 0.5, 0)`, plus the likelihood-ratio `G^2 = 2 sum O ln(O / E)` (Wilks 1938), and the
/// minimum expected count.
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
    if r_sums.contains(&0) || c_sums.contains(&0) {
        return Err(StatsError::ZeroVariance { context: "chi-square contingency zero margin" });
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
            let diff = (obs - exp).abs();
            let adj_diff = if use_yates { (diff - 0.5).max(0.0) } else { diff };
            chi2 += (adj_diff * adj_diff) / exp;
            if obs > 0.0 {
                g2 += 2.0 * obs * (obs / exp).ln();
            }
        }
    }
    Ok((chi2, g2.max(0.0), min_exp))
}

/// Cochran–Armitage chi-square test for linear trend across ordered levels (`R x 2` or `2 x C`).
///
/// Cochran (1954); Armitage (1955). Uses equally spaced scores `1..k` and the unconditional
/// variance `p_bar (1 - p_bar) sum n_i (s_i - s_bar)^2`, returning the two-sided normal *P* of `z`.
pub fn cochran_armitage_trend(counts: &[Vec<u64>]) -> Result<(f64, f64), StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    for row in counts {
        if row.len() != n_cols {
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n_cols, actual: row.len() }));
        }
    }
    if !((n_cols == 2 && n_rows >= 3) || (n_rows == 2 && n_cols >= 3)) {
        let bad = if n_rows == 2 || n_cols == 2 { n_rows.max(n_cols) } else { n_rows.min(n_cols) };
        return Err(StatsError::InvalidParameter { name: "cochran_armitage_dimensions", value: bad as f64 });
    }
    let reduced = reduce_zero_margin_table(counts).map_err(|err| match err {
        StatsError::InsufficientSampleSize { actual, .. } => {
            StatsError::InvalidParameter { name: "cochran_armitage_dimensions", value: actual as f64 }
        }
        other => other,
    })?;
    if !((reduced.n_cols == 2 && reduced.n_rows >= 3) || (reduced.n_rows == 2 && reduced.n_cols >= 3)) {
        let bad = if reduced.n_rows == 2 || reduced.n_cols == 2 { reduced.n_rows.max(reduced.n_cols) } else { reduced.n_rows.min(reduced.n_cols) };
        return Err(StatsError::InvalidParameter { name: "cochran_armitage_dimensions", value: bad as f64 });
    }
    // Orient as k ordered levels x 2 outcomes.
    let oriented: Vec<[f64; 2]> = if reduced.n_cols == 2 {
        reduced.counts.iter().map(|r| [r[0] as f64, r[1] as f64]).collect()
    } else {
        (0..reduced.n_cols).map(|c| [reduced.counts[0][c] as f64, reduced.counts[1][c] as f64]).collect()
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
///
/// McNemar (1947) test with the Edwards (1948) continuity correction `(|b - c| - 1)^2 / (b + c)`.
/// The exact *P* doubles the smaller `Binomial(b + c, 1/2)` tail, which equals the Sterne (1954)
/// two-sided *P* because the null distribution is symmetric.
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
///
/// Empty (all-zero) rows and columns are dropped prior to computing degrees of freedom, test
/// statistics, Cramér's *V*, and confidence intervals; tables falling below `2 x 2` are rejected
/// (`InsufficientSampleSize`).
pub fn analyze_contingency(counts: &[Vec<u64>], yates_continuity: bool, confidence: ConfidenceLevel) -> Result<ContingencyResult, StatsError> {
    let reduced = reduce_zero_margin_table(counts)?;
    analyze_reduced_contingency(&reduced, yates_continuity, confidence)
}

pub(super) fn analyze_reduced_contingency(
    reduced: &ReducedContingencyTable,
    yates_continuity: bool,
    confidence: ConfidenceLevel,
) -> Result<ContingencyResult, StatsError> {
    let counts = &reduced.counts;
    let r_sums = &reduced.r_sums;
    let c_sums = &reduced.c_sums;
    let total_count = reduced.total_count;
    let n_rows = reduced.n_rows;
    let n_cols = reduced.n_cols;

    let (chi2, g2, min_exp) = chi_square_stats_with_margins(counts, r_sums, c_sums, total_count, yates_continuity)?;
    let df = ((n_rows - 1) * (n_cols - 1)) as f64;
    let chi2_p = chi2_sf(chi2, df)?;
    let g2_p = chi2_sf(g2, df)?;

    // Count fraction of cells with expected count < 5.
    let total_f = total_count as f64;
    let mut lt5 = 0_usize;
    for &rs in r_sums {
        for &cs in c_sums {
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
    let (chi2_uncorr, _, _) = if yates_continuity && n_rows == 2 && n_cols == 2 {
        chi_square_stats_with_margins(counts, r_sums, c_sums, total_count, false)?
    } else {
        (chi2, g2, min_exp)
    };
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
        dropped_empty_rows: reduced.dropped_rows,
        dropped_empty_cols: reduced.dropped_cols,
    })
}
