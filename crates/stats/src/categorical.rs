//! Contingency table, categorical proportion, agreement, and goodness-of-fit analyses.
//!
//! Includes Pearson chi-square (with Yates continuity option), likelihood-ratio `G^2`,
//! Fisher's exact test (`2x2` + `RxC` Freeman–Halton / deterministic Monte Carlo), Cochran–Armitage
//! trend test, McNemar paired test, Wilson & Clopper–Pearson binomial CIs, Baptista–Pike OR CI,
//! Koopman score RR CI, Newcombe risk-difference CI, Cohen's kappa (unweighted/weighted),
//! Fleiss' kappa, and phi / Cramér's `V` with exact noncentral chi-square CIs.

use lumencraft_doc::{
    AnalysisSpec, ChecklistStatus, ContingencyMethod, ContingencySpec, DataSheet, PValueFormat, ResultCell, ResultSheet, ResultTabKind,
    TableArchetype,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{chi2_sf, noncentral_chi2_ci, norm_cdf, norm_sf, norm_two_sided_isf};
use lumencraft_math::rng::DeterministicRng;
use lumencraft_math::roots::{RootOptions, brent_root};
use lumencraft_math::special::{ln_factorial, reg_beta_inv};
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
    pub sensitivity: f64,
    pub sensitivity_wilson_ci: (f64, f64),
    pub specificity: f64,
    pub specificity_wilson_ci: (f64, f64),
    pub ppv: f64,
    pub ppv_wilson_ci: (f64, f64),
    pub npv: f64,
    pub npv_wilson_ci: (f64, f64),
}

/// Weighting scheme for Cohen's kappa on square `K x K` confusion matrices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KappaWeighting {
    Unweighted,
    Linear,
    Quadratic,
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
    let alpha = confidence.alpha();
    let half = 0.5 * alpha;
    let lo = if k == 0 { 0.0 } else { reg_beta_inv(k as f64, (n - k + 1) as f64, half)? };
    let hi = if k >= n { 1.0 } else { reg_beta_inv((k + 1) as f64, (n - k) as f64, 1.0 - half)? };
    Ok((lo.clamp(0.0, 1.0), hi.clamp(0.0, 1.0)))
}

// ============================================================================
// Contingency Tests (Chi-Square, G^2, Fisher, Cochran–Armitage, McNemar)
// ============================================================================

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
    let r1 = a + b;
    let r2 = c + d;
    let c1 = a + c;
    let total = r1 + r2;
    if total == 0 {
        return Ok(1.0);
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
    if n_rows == 2 && n_cols == 2 {
        return fisher_exact_2x2(counts[0][0], counts[0][1], counts[1][0], counts[1][1]);
    }
    let r_sums: Vec<u64> = counts.iter().map(|r| r.iter().sum()).collect();
    let mut c_sums = vec![0_u64; n_cols];
    for row in counts {
        for (c, &v) in row.iter().enumerate() {
            c_sums[c] += v;
        }
    }
    let total: u64 = r_sums.iter().sum();
    if total == 0 {
        return Ok(1.0);
    }
    let obs_log = log_hypergeom_table(counts, &r_sums, &c_sums, total)?;

    // Build flat vector of column assignments and permute deterministically across 10,000 draws.
    if total > 50_000 {
        // Fallback asymptotic chi-square if total count exceeds reasonable permutation vector size.
        let (chi2, _, _) = chi_square_stats(counts, false)?;
        let df = ((n_rows - 1) * (n_cols - 1)) as f64;
        return Ok(chi2_sf(chi2, df)?);
    }
    let mut col_pool = Vec::with_capacity(total as usize);
    for (c_idx, &cnt) in c_sums.iter().enumerate() {
        col_pool.resize(col_pool.len() + (cnt as usize), c_idx);
    }
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
                let c_idx = col_pool[cursor];
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

fn chi_square_stats(counts: &[Vec<u64>], yates: bool) -> Result<(f64, f64, f64), StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    if n_rows < 2 || n_cols < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "chi-square contingency", required: 2, actual: n_rows.min(n_cols) });
    }
    let r_sums: Vec<f64> = counts.iter().map(|r| r.iter().sum::<u64>() as f64).collect();
    let mut c_sums = vec![0.0_f64; n_cols];
    for row in counts {
        if row.len() != n_cols {
            return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n_cols, actual: row.len() }));
        }
        for (c, &v) in row.iter().enumerate() {
            c_sums[c] += v as f64;
        }
    }
    let total: f64 = r_sums.iter().sum();
    if total <= 0.0 {
        return Err(StatsError::EmptyInput);
    }

    let use_yates = yates && n_rows == 2 && n_cols == 2;
    let mut chi2 = 0.0_f64;
    let mut g2 = 0.0_f64;
    let mut min_exp = f64::INFINITY;
    let mut lt5_count = 0_usize;

    for (r, row) in counts.iter().enumerate() {
        for (c, &o_u) in row.iter().enumerate() {
            let obs = o_u as f64;
            let exp = (r_sums[r] * c_sums[c]) / total;
            min_exp = min_exp.min(exp);
            if exp < 5.0 {
                lt5_count += 1;
            }
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
    let frac_lt5 = (lt5_count as f64) / ((n_rows * n_cols) as f64);
    Ok((chi2, g2.max(0.0), min_exp)).map(|(x, g, me)| {
        let _ = frac_lt5;
        (x, g, me)
    })
}

/// Cochran–Armitage chi-square test for linear trend across ordered levels (`R x 2` or `2 x C`).
pub fn cochran_armitage_trend(counts: &[Vec<u64>]) -> Result<(f64, f64), StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    if !((n_cols == 2 && n_rows >= 2) || (n_rows == 2 && n_cols >= 2)) {
        return Err(StatsError::InvalidParameter { name: "dimensions", value: n_rows.min(n_cols) as f64 });
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
    let n = b + c;
    if n == 0 {
        return Ok((0.0, 1.0));
    }
    let diff = (b as f64 - c as f64).abs();
    let chi2 = ((diff - 1.0).max(0.0)).powi(2) / (n as f64);
    let min_bc = b.min(c);
    let ln_two_n = -(n as f64) * std::f64::consts::LN_2;
    let ln_nf = ln_factorial(n);
    let mut tail = 0.0_f64;
    for k in 0..=min_bc {
        let lp = ln_nf - ln_factorial(k) - ln_factorial(n - k) + ln_two_n;
        tail += lp.exp();
    }
    let exact_p = if min_bc * 2 == n { 1.0 } else { (2.0 * tail).clamp(0.0, 1.0) };
    Ok((chi2, exact_p))
}

/// Full contingency table analysis combining Pearson chi-square, `G^2`, Fisher/Freeman–Halton,
/// Cochran–Armitage (when `R=2` or `C=2`), McNemar (when `2x2`), phi, and Cramér's `V`.
pub fn analyze_contingency(counts: &[Vec<u64>], yates_continuity: bool, confidence: ConfidenceLevel) -> Result<ContingencyResult, StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    let total_count: u64 = counts.iter().flat_map(|r| r.iter()).sum();
    if total_count == 0 {
        return Err(StatsError::EmptyInput);
    }

    let (chi2, g2, min_exp) = chi_square_stats(counts, yates_continuity)?;
    let df = ((n_rows - 1) * (n_cols - 1)) as f64;
    let chi2_p = chi2_sf(chi2, df)?;
    let g2_p = chi2_sf(g2, df)?;

    // Count fraction of cells with expected count < 5.
    let total_f = total_count as f64;
    let r_sums: Vec<f64> = counts.iter().map(|r| r.iter().sum::<u64>() as f64).collect();
    let mut c_sums = vec![0.0_f64; n_cols];
    for row in counts {
        for (c, &v) in row.iter().enumerate() {
            c_sums[c] += v as f64;
        }
    }
    let mut lt5 = 0_usize;
    for &rs in &r_sums {
        for &cs in &c_sums {
            if (rs * cs) / total_f < 5.0 {
                lt5 += 1;
            }
        }
    }
    let cells_lt5_fraction = (lt5 as f64) / ((n_rows * n_cols) as f64);

    let fisher_exact_p = if n_rows == 2 && n_cols == 2 {
        Some(fisher_exact_2x2(counts[0][0], counts[0][1], counts[1][0], counts[1][1])?)
    } else if n_rows <= 6 && n_cols <= 6 && total_count <= 20_000 {
        Some(fisher_exact_rxc(counts)?)
    } else {
        None
    };

    let (mcnemar_chi2, mcnemar_exact_p) = if n_rows == 2 && n_cols == 2 {
        let (mc, mp) = mcnemar_test(counts[0][1], counts[1][0])?;
        (Some(mc), Some(mp))
    } else {
        (None, None)
    };

    let (cochran_armitage_z, cochran_armitage_p) = if (n_rows == 2 && n_cols >= 2) || (n_cols == 2 && n_rows >= 2) {
        let (z, p) = cochran_armitage_trend(counts)?;
        (Some(z), Some(p))
    } else {
        (None, None)
    };

    // Uncorrected Pearson chi-square for phi and Cramer's V.
    let (chi2_uncorr, _, _) = if yates_continuity { chi_square_stats(counts, false)? } else { (chi2, g2, min_exp) };
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
fn noncentral_hypergeom_p(a_obs: u64, r1: u64, r2: u64, c1: u64, or_val: f64) -> f64 {
    let lo = c1.saturating_sub(r2);
    let hi = r1.min(c1);
    let log_or = or_val.max(1e-18).ln();
    let mut log_weights = Vec::with_capacity((hi - lo + 1) as usize);
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
    let mut obs_p = 0.0_f64;
    for &(x, w) in &probs {
        if x == a_obs {
            obs_p = w / norm;
            break;
        }
    }
    let p_sum: f64 = probs.iter().map(|(_, w)| w / norm).filter(|&p| p <= obs_p + 1e-12).sum();
    p_sum.clamp(0.0, 1.0)
}

/// Baptista–Pike (1977) exact conditional odds-ratio confidence interval for `2 x 2` table `[[a, b], [c, d]]`.
pub fn baptista_pike_or_ci(a: u64, b: u64, c: u64, d: u64, confidence: ConfidenceLevel) -> Option<(f64, f64)> {
    let r1 = a + b;
    let r2 = c + d;
    let c1 = a + c;
    let lo_x = c1.saturating_sub(r2);
    let hi_x = r1.min(c1);
    if r1 == 0 || r2 == 0 || lo_x >= hi_x {
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
            if noncentral_hypergeom_p(a, r1, r2, c1, l) < alpha {
                break;
            }
            l *= 0.5;
        }
        brent_root(
            |theta| noncentral_hypergeom_p(a, r1, r2, c1, theta) - alpha,
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
            if noncentral_hypergeom_p(a, r1, r2, c1, h) < alpha {
                break;
            }
            h *= 2.0;
        }
        brent_root(
            |theta| noncentral_hypergeom_p(a, r1, r2, c1, theta) - alpha,
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
    let n1 = (a + b) as f64;
    let n2 = (c + d) as f64;
    if n1 <= 0.0 || n2 <= 0.0 || (a == 0 && c == 0) {
        return None;
    }
    let z2 = norm_two_sided_isf(confidence.alpha()).ok()?.powi(2);
    let score = |phi: f64| koopman_chi2_score(a, b, c, d, z2, phi);

    let rr_pt = if c == 0 { f64::INFINITY } else { ((a as f64) / n1) / ((c as f64) / n2) };

    let lo = if a == 0 {
        0.0
    } else if c == 0 {
        koopman_solve_lower_c_zero(&score)
    } else {
        koopman_solve_lower(&score, rr_pt)
    };

    let hi = if c == 0 {
        f64::INFINITY
    } else if a == 0 {
        koopman_solve_upper_a_zero(&score)
    } else {
        koopman_solve_upper(&score, rr_pt)
    };

    Some((lo, hi))
}

fn koopman_chi2_score(a: u64, b: u64, c: u64, d: u64, z2: f64, phi: f64) -> f64 {
    if !phi.is_finite() || phi <= 0.0 {
        return f64::INFINITY;
    }
    let x1 = a as f64;
    let x2 = c as f64;
    let n1 = (a + b) as f64;
    let n2 = (c + d) as f64;
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

fn koopman_solve_lower<F: Fn(f64) -> f64>(score: &F, rr_pt: f64) -> f64 {
    let mut hi_neg = rr_pt;
    let mut lo_pos = rr_pt * 0.5;
    for _ in 0..120 {
        if score(lo_pos) > 0.0 {
            break;
        }
        hi_neg = lo_pos;
        lo_pos *= 0.5;
    }
    brent_root(score, lo_pos, hi_neg, RootOptions { rel_tol: 1e-10, abs_tol: 1e-30, max_iters: 128 }).unwrap_or(lo_pos)
}

fn koopman_solve_upper<F: Fn(f64) -> f64>(score: &F, rr_pt: f64) -> f64 {
    let mut lo_neg = rr_pt;
    let mut hi_pos = rr_pt * 2.0;
    for _ in 0..120 {
        if score(hi_pos) > 0.0 {
            break;
        }
        lo_neg = hi_pos;
        hi_pos *= 2.0;
    }
    brent_root(score, lo_neg, hi_pos, RootOptions { rel_tol: 1e-10, abs_tol: 1e-30, max_iters: 128 }).unwrap_or(hi_pos)
}

fn koopman_solve_lower_c_zero<F: Fn(f64) -> f64>(score: &F) -> f64 {
    let mut neg_pt = 1.0_f64;
    for _ in 0..120 {
        if score(neg_pt) < 0.0 {
            break;
        }
        neg_pt *= 2.0;
    }
    let mut pos_pt = neg_pt * 0.5;
    for _ in 0..120 {
        if score(pos_pt) > 0.0 {
            break;
        }
        neg_pt = pos_pt;
        pos_pt *= 0.5;
    }
    brent_root(score, pos_pt, neg_pt, RootOptions { rel_tol: 1e-10, abs_tol: 1e-30, max_iters: 128 }).unwrap_or(pos_pt)
}

fn koopman_solve_upper_a_zero<F: Fn(f64) -> f64>(score: &F) -> f64 {
    let mut neg_pt = 1.0_f64;
    for _ in 0..120 {
        if score(neg_pt) < 0.0 {
            break;
        }
        neg_pt *= 0.5;
    }
    let mut pos_pt = neg_pt * 2.0;
    for _ in 0..120 {
        if score(pos_pt) > 0.0 {
            break;
        }
        neg_pt = pos_pt;
        pos_pt *= 2.0;
    }
    brent_root(score, neg_pt, pos_pt, RootOptions { rel_tol: 1e-10, abs_tol: 1e-30, max_iters: 128 }).unwrap_or(pos_pt)
}

/// Compute complete `2 x 2` risk/odds ratios (`[[a, b], [c, d]]`), Baptista-Pike OR CI, Koopman
/// RR CI, Newcombe risk-difference CI, NNT, and Wilson CIs for sensitivity/specificity/PPV/NPV.
pub fn analyze_2x2_ratios(a: u64, b: u64, c: u64, d: u64, confidence: ConfidenceLevel) -> Result<TwoByTwoRatiosResult, StatsError> {
    let r1 = a + b;
    let r2 = c + d;
    let c1 = a + c;
    let c2 = b + d;
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
    let rd_lo = rd - ((p1 - l1).powi(2) + (u2 - p2).powi(2)).sqrt();
    let rd_hi = rd + ((u1 - p1).powi(2) + (p2 - l2).powi(2)).sqrt();

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
        rd_newcombe_ci: Some((rd_lo.clamp(-1.0, 1.0), rd_hi.clamp(-1.0, 1.0))),
        nnt,
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
    let total: u64 = counts.iter().flat_map(|r| r.iter()).sum();
    if total == 0 {
        return Err(StatsError::EmptyInput);
    }
    let nf = total as f64;
    let row_p: Vec<f64> = counts.iter().map(|r| (r.iter().sum::<u64>() as f64) / nf).collect();
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
    let m: u64 = rater_counts[0].iter().sum();
    if m < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Fleiss raters per subject", required: 2, actual: m as usize });
    }
    for row in rater_counts {
        if row.len() != k || row.iter().sum::<u64>() != m {
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

/// Execute a [`ContingencySpec`] against a `Contingency` or `PartsOfWhole` [`DataSheet`].
pub fn run_contingency_analysis(sheet: &DataSheet, spec: &AnalysisSpec, cont_spec: &ContingencySpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let confidence = spec.confidence_level;
    let p_fmt = spec.p_value_format;

    if let TableArchetype::PartsOfWhole(pow) = &sheet.archetype {
        return run_parts_of_whole_gof(pow, &mut out, p_fmt);
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

    let yates = matches!(cont_spec.method, ContingencyMethod::PearsonChiSquare { yates_continuity: true });
    let res = analyze_contingency(&counts, yates, confidence)?;
    push_scalar(&mut out, "Chi-square", None, res.chi_square);
    push_scalar(&mut out, "Chi-square P value", None, res.chi_square_p);
    push_scalar(&mut out, "G-squared", None, res.g_squared);
    push_scalar(&mut out, "Cramer's V", None, res.cramers_v);
    if let Some(fp) = res.fisher_exact_p {
        push_scalar(&mut out, "Fisher's exact P", None, fp);
    }

    let (primary_name, primary_stat, primary_p) = match cont_spec.method {
        ContingencyMethod::PearsonChiSquare { yates_continuity } => {
            let lbl = if yates_continuity { "Chi-square (Yates)" } else { "Pearson chi-square" };
            (lbl, Some(res.chi_square), res.chi_square_p)
        }
        ContingencyMethod::LikelihoodRatioG2 => ("Likelihood-ratio G2", Some(res.g_squared), res.g_squared_p),
        ContingencyMethod::FisherExact => ("Fisher's exact test", None, res.fisher_exact_p.unwrap_or(res.chi_square_p)),
        ContingencyMethod::CochranArmitageTrend => {
            ("Cochran-Armitage trend", res.cochran_armitage_z, res.cochran_armitage_p.unwrap_or(res.chi_square_p))
        }
        ContingencyMethod::McNemarPaired => ("McNemar's test", res.mcnemar_chi2, res.mcnemar_exact_p.unwrap_or(res.chi_square_p)),
    };

    let mut headers = vec![String::from("Statistic"), String::from("df"), String::from("P value")];
    if cont_spec.compute_phi_and_cramers_v {
        headers.push(String::from("Cramer's V / Phi"));
        headers.push(format!("{}% CI", confidence.percent()));
    }
    let mut row_main = vec![opt_num_cell(primary_stat), num_cell(res.df), p_cell(primary_p, p_fmt)];
    if cont_spec.compute_phi_and_cramers_v {
        row_main.push(num_cell(res.phi.unwrap_or(res.cramers_v)));
        row_main.push(opt_ci_cell(res.cramers_v_ci));
    }
    out.tabs.push(build_tab(
        "contingency-main",
        "Contingency analysis",
        ResultTabKind::MainSummary,
        headers,
        vec![primary_name.to_owned()],
        vec![row_main],
    ));

    if n_rows == 2 && n_cols == 2 && (cont_spec.compute_odds_and_risk_ratios || cont_spec.compute_sensitivity_specificity) {
        let r2x2 = analyze_2x2_ratios(counts[0][0], counts[0][1], counts[1][0], counts[1][1], confidence)?;
        push_scalar(&mut out, "Odds ratio", None, r2x2.odds_ratio);
        push_scalar(&mut out, "Relative risk", None, r2x2.relative_risk);
        push_scalar(&mut out, "Risk difference", None, r2x2.risk_difference);

        let r_headers = vec![String::from("Estimate"), format!("{}% CI", confidence.percent())];
        let mut r_labels = Vec::new();
        let mut r_rows = Vec::new();
        if cont_spec.compute_odds_and_risk_ratios {
            r_labels.extend([
                String::from("Odds ratio (Baptista-Pike CI)"),
                String::from("Relative risk (Koopman CI)"),
                String::from("Risk difference (Newcombe CI)"),
                String::from("Number needed to treat (NNT)"),
            ]);
            r_rows.push(vec![num_cell(r2x2.odds_ratio), opt_ci_cell(r2x2.or_baptista_pike_ci)]);
            r_rows.push(vec![num_cell(r2x2.relative_risk), opt_ci_cell(r2x2.rr_koopman_ci)]);
            r_rows.push(vec![num_cell(r2x2.risk_difference), opt_ci_cell(r2x2.rd_newcombe_ci)]);
            r_rows.push(vec![opt_num_cell(r2x2.nnt), ResultCell::Empty]);
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

    let expected_ok = res.min_expected_count >= 5.0 || (res.n_rows > 2 && res.cells_lt5_fraction <= 0.2 && res.min_expected_count >= 1.0);
    out.checklist.push(checklist_item(
        "Minimum expected cell count",
        if expected_ok { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(format!("min E = {:.2}, {:.1}% cells < 5", res.min_expected_count, 100.0 * res.cells_lt5_fraction)),
        if expected_ok {
            "Expected cell counts satisfy Cochran's guidelines for asymptotic chi-square approximation."
        } else {
            "Small expected counts detected (< 5); prefer Fisher's exact test or exact conditional intervals."
        },
    ));

    Ok(out)
}

fn run_parts_of_whole_gof(pow: &lumencraft_doc::PartsOfWholeTable, out: &mut ResultSheet, p_fmt: PValueFormat) -> Result<ResultSheet, StatsError> {
    let slices = pow.fractions_for_series(0)?;
    let k = slices.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "parts-of-whole goodness-of-fit", required: 2, actual: k });
    }
    let total: f64 = slices.iter().map(|s| s.value).sum();
    if total <= 0.0 {
        return Err(StatsError::EmptyInput);
    }
    let exp = total / (k as f64);
    let chi2: f64 = slices.iter().map(|s| (s.value - exp).powi(2) / exp).sum();
    let df = (k - 1) as f64;
    let p_val = chi2_sf(chi2, df)?;
    push_scalar(out, "Goodness-of-fit chi-square", None, chi2);
    push_scalar(out, "P value", None, p_val);

    let headers = vec![String::from("Chi-square"), String::from("df"), String::from("P value")];
    let rows = vec![vec![num_cell(chi2), num_cell(df), p_cell(p_val, p_fmt)]];
    out.tabs.push(build_tab(
        "parts-gof",
        "Chi-square goodness of fit (uniform expected)",
        ResultTabKind::MainSummary,
        headers,
        vec![String::from("Goodness of fit")],
        rows,
    ));
    Ok(out.clone())
}

#[cfg(test)]
mod tests {
    use lumencraft_math::ConfidenceLevel;
    use lumencraft_math::dist::norm_two_sided_isf;

    use super::{koopman_chi2_score, koopman_rr_ci};

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
}
