//! Nonparametric two-group hypothesis tests: Mann–Whitney *U* with Hodges–Lehmann shift and Moses
//! CI, Wilcoxon matched-pairs signed-rank (classic and Pratt zero handling), exact paired sign
//! test, and two-sample Kolmogorov–Smirnov test.
//!
//! # References
//!
//! - Wilcoxon, F. (1945). Individual comparisons by ranking methods. *Biometrics Bulletin* 1(6):80–83.
//!   doi:10.2307/3001968
//! - Mann, H. B., & Whitney, D. R. (1947). On a test of whether one of two random variables is
//!   stochastically larger than the other. *Annals of Mathematical Statistics* 18(1):50–60.
//!   doi:10.1214/aoms/1177730491
//! - Pratt, J. W. (1959). Remarks on zeros and ties in the Wilcoxon signed rank procedures.
//!   *Journal of the American Statistical Association* 54(287):655–667. doi:10.1080/01621459.1959.10501526
//! - Hodges, J. L., & Lehmann, E. L. (1963). Estimates of location based on rank tests.
//!   *Annals of Mathematical Statistics* 34(2):598–611. doi:10.1214/aoms/1177704172
//! - Monahan, J. F. (1984). Algorithm 616: Fast computation of the Hodges–Lehmann location estimator.
//!   *ACM Transactions on Mathematical Software* 10(3):265–270. doi:10.1145/1271.319414
//! - Hollander, M., Wolfe, D. A., & Chicken, E. (2014). *Nonparametric Statistical Methods* (3rd ed.).
//!   Wiley. Ch. 3 (paired/one-sample location: signed-rank test, Walsh-average estimator), ch. 4
//!   (two-sample location: rank-sum test, Hodges–Lehmann shift estimator, Moses interval).
//! - Conover, W. J. (1999). *Practical Nonparametric Statistics* (3rd ed.). Wiley.
//! - Dixon, W. J., & Mood, A. M. (1946). The statistical sign test. *Journal of the American
//!   Statistical Association* 41:557–566.
//! - Cliff, N. (1993). Dominance statistics: Ordinal analyses to answer ordinal questions.
//!   *Psychological Bulletin* 114(3):494–509. doi:10.1037/0033-2909.114.3.494
//! - Wendt, H. W. (1972). Dealing with a common problem in social science: A simplified rank-biserial
//!   coefficient of correlation based on the U statistic. *European Journal of Social Psychology* 2.
//!   doi:10.1002/ejsp.2420020412
//! - Kerby, D. S. (2014). The simple difference formula: An approach to teaching nonparametric
//!   correlation. *Comprehensive Psychology* 3. doi:10.2466/11.IT.3.1
//! - Smirnov, N. (1948). Table for estimating the goodness of fit of empirical distributions.
//!   *Annals of Mathematical Statistics* 19(2):279–281. doi:10.1214/aoms/1177730256
//! - Hodges, J. L. (1958). The significance probability of the Smirnov two-sample test.
//!   *Arkiv för Matematik* 3(5):469–486. doi:10.1007/BF02589501

use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{norm_sf, norm_two_sided_isf};
use curvebook_math::special::inc_beta::reg_beta;
use serde::{Deserialize, Serialize};

use crate::column::{exact_binomial_median_ci, hodges_lehmann_one_sample_with_coverage, ks_asymptotic_p, wilcoxon_exact_p};
use crate::common::{quantile_type6_sorted, quartiles_hinge, rank_with_ties, select_kth_pairwise_sum, sorted_finite, validate_finite_slice};
use crate::error::StatsError;

/// Result of a Mann–Whitney *U* test (Mann & Whitney 1947; Wilcoxon 1945).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MannWhitneyResult {
    /// Number of observations in group A.
    pub n_a: usize,
    /// Number of observations in group B.
    pub n_b: usize,
    /// Sample median of group A (Hyndman & Fan Type 6).
    pub median_a: f64,
    /// Sample median of group B (Hyndman & Fan Type 6).
    pub median_b: f64,
    /// Mann–Whitney *U* count of `(a > b)` pairs (with 0.5 for exact ties) for group A.
    pub u_a: f64,
    /// Complementary Mann–Whitney *U* count `n_a * n_b - u_a` for group B.
    pub u_b: f64,
    /// Smaller Mann–Whitney statistic `min(u_a, u_b)`.
    pub u_min: f64,
    /// *P* value testing identical distributions (`two_tailed` doubles smaller tail vs directional observed-tail one-sided via exact DP when `n_a + n_b <= 40` and tie-free, else tie-/continuity-corrected normal).
    pub p_value: f64,
    /// True if exact Mann–Whitney combinatorial DP distribution was used; false for normal approximation.
    pub exact_p_used: bool,
    /// True for two-sided *P* value; false for directional one-sided test.
    pub two_tailed: bool,
    /// Hodges–Lehmann (1963) shift location estimator `median(a_i - b_j)` across all pairwise differences.
    pub hodges_lehmann_diff: f64,
    /// Two-sided distribution-free order-statistic CI `(lower, upper)` on pairwise differences `a_i - b_j` at configured `ConfidenceLevel` (`None` if normal/exact rank boundary unavailable).
    pub hodges_lehmann_ci: Option<(f64, f64)>,
    /// Exact achieved confidence percentage when small-sample (`na <= 30 && nb <= 30`, untied) exact Mann–Whitney critical values were used (`None` for asymptotic normal or unavailable CIs).
    #[serde(default)]
    pub ci_actual_confidence_percent: Option<f64>,
    /// Cliff's dominance effect size `delta = (u_a - u_b) / (n_a * n_b)` in `[-1, 1]` (Cliff 1993).
    pub cliffs_delta: f64,
    /// Rank-biserial correlation `r = 1 - 2 * u_min / (n_a * n_b)` / Kerby (2014) signed rank effect size in `[-1, 1]`.
    pub rank_biserial_r: f64,
}

/// Exact DP distribution of Mann–Whitney `U` for `n1 + n2 <= 40` without ties.
///
/// Counts arrangements by the Mann & Whitney (1947) recurrence on `U`; the two-sided *p*-value doubles
/// the smaller tail, and the one-sided *p*-value returns the smaller (observed-direction) tail
/// `min(P(U <= u_obs), P(U >= u_obs))`, matching the sign, Wilcoxon, and *t*-test convention.
fn mann_whitney_exact_p(n1: usize, n2: usize, u_obs: usize, two_tailed: bool) -> f64 {
    let max_u = n1 * n2;
    // dp[i][u] = number of subsets of size i from i+j elements with U = u, iteratively over N = n1+n2.
    let mut dp = vec![vec![0.0_f64; max_u + 1]; n1 + 1];
    dp[0][0] = 1.0;
    for total in 1..=(n1 + n2) {
        let i_max = total.min(n1);
        for i in (1..=i_max).rev() {
            let j = total - i;
            if j > n2 {
                continue;
            }
            for u in j..=max_u {
                dp[i][u] += dp[i - 1][u - j];
            }
        }
    }
    let total_ways: f64 = dp[n1].iter().sum();
    if total_ways <= 0.0 {
        return 1.0;
    }
    let lower_tail: f64 = dp[n1].iter().take(u_obs + 1).sum();
    let upper_tail: f64 = dp[n1].iter().skip(u_obs).sum();
    let min_tail = lower_tail.min(upper_tail);
    if two_tailed { (2.0 * min_tail / total_ways).clamp(0.0, 1.0) } else { (min_tail / total_ways).clamp(0.0, 1.0) }
}

/// Exact conditional permutation DP for Mann–Whitney with tied ranks (`n_a + n_b <= 18`).
/// Computes exact permutation counts over doubled integer ranks `2 * r_i`.
fn mann_whitney_exact_ties_p(ranks: &[f64], na: usize, two_tailed: bool) -> f64 {
    let n = ranks.len();
    let two_ranks: Vec<usize> = ranks.iter().map(|&r| (2.0 * r).round() as usize).collect();
    let max_sum: usize = two_ranks.iter().sum();
    let mut dp = vec![vec![0_u64; max_sum + 1]; na + 1];
    dp[0][0] = 1;
    let mut running_max = 0_usize;
    for (idx, &tr) in two_ranks.iter().enumerate() {
        let i_max = na.min(idx + 1);
        running_max += tr;
        for i in (1..=i_max).rev() {
            for s in tr..=running_max {
                dp[i][s] += dp[i - 1][s - tr];
            }
        }
    }
    let total_ways: u64 = dp[na].iter().sum();
    if total_ways == 0 {
        return 1.0;
    }
    let two_r1_obs: usize = two_ranks.iter().take(na).sum();
    if two_tailed {
        let expected_two_r1 = na * (n + 1);
        let obs_dev = two_r1_obs.abs_diff(expected_two_r1);
        let tail_ways: u64 = dp[na].iter().enumerate().filter(|&(s, _)| s.abs_diff(expected_two_r1) >= obs_dev).map(|(_, &c)| c).sum();
        ((tail_ways as f64) / (total_ways as f64)).clamp(0.0, 1.0)
    } else {
        let lower_tail: u64 = dp[na].iter().take(two_r1_obs + 1).sum();
        let upper_tail: u64 = dp[na].iter().skip(two_r1_obs).sum();
        let min_tail = lower_tail.min(upper_tail);
        ((min_tail as f64) / (total_ways as f64)).clamp(0.0, 1.0)
    }
}

/// Mann–Whitney *U* test comparing `a` and `b`, with exact DP *p*-values for untied samples
/// (`n_a + n_b <= 40`) and exact conditional permutation DP for tied samples (`n_a + n_b <= 18`),
/// exact Hodges–Lehmann median difference `a - b` + Moses CI, Cliff's `delta`, and rank-biserial `r`.
///
/// Wilcoxon (1945) rank-sum / Mann & Whitney (1947) `U_a = R_a - n_a (n_a + 1) / 2`. Above the
/// exact DP bounds (`n_a + n_b > 40` untied, or `> 18` with ties), the *p*-value uses the normal
/// approximation with the tie-corrected variance `(n_a n_b / 12) [N + 1 - sum(t^3 - t) / (N (N - 1))]`
/// and a 0.5 continuity correction (Hollander, Wolfe & Chicken 2014, ch. 4).
/// Cliff's `delta = (U_a - U_b) / (n_a n_b)` (Cliff 1993), equal to the rank-biserial correlation
/// of Wendt (1972) / Kerby (2014).
pub fn mann_whitney_u(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<MannWhitneyResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    let na = a.len();
    let nb = b.len();
    let sa = sorted_finite(a);
    let sb = sorted_finite(b);
    let median_a = quantile_type6_sorted(&sa, 0.5).or_else(|| sa.first().copied()).ok_or(StatsError::EmptyInput)?;
    let median_b = quantile_type6_sorted(&sb, 0.5).or_else(|| sb.first().copied()).ok_or(StatsError::EmptyInput)?;

    let mut combined = Vec::with_capacity(na + nb);
    combined.extend_from_slice(a);
    combined.extend_from_slice(b);
    let ranked = rank_with_ties(&combined)?;
    let r1: f64 = ranked.ranks.iter().take(na).sum();
    let naf = na as f64;
    let nbf = nb as f64;
    let nf = (na + nb) as f64;

    let u_a = r1 - 0.5 * naf * (naf + 1.0);
    let u_b = naf * nbf - u_a;
    let u_min = u_a.min(u_b);

    let (p_value, exact_p_used) = if na + nb <= 40 && ranked.tie_counts.is_empty() {
        let u_int = u_a.round() as usize;
        (mann_whitney_exact_p(na, nb, u_int, two_tailed), true)
    } else if !ranked.tie_counts.is_empty() && na + nb <= 18 {
        (mann_whitney_exact_ties_p(&ranked.ranks, na, two_tailed), true)
    } else {
        let mean_u = 0.5 * naf * nbf;
        let var_u = (naf * nbf / 12.0) * (nf + 1.0 - ranked.tie_sum_t3_minus_t / (nf * (nf - 1.0)));
        if ranked.tie_counts.as_slice() == [na + nb] || var_u <= 0.0 {
            let p = if (u_a - mean_u).abs() <= 1e-12 { 1.0 } else { 0.0 };
            (p, false)
        } else {
            let sd_u = var_u.sqrt();
            let p = if two_tailed {
                let cc = (u_a - mean_u).abs() - 0.5;
                if cc <= 0.0 { 1.0 } else { (2.0 * norm_sf(cc / sd_u)).min(1.0) }
            } else {
                let cc = ((u_a - mean_u).abs() - 0.5).max(0.0);
                norm_sf(cc / sd_u).clamp(0.0, 1.0)
            };
            (p.clamp(0.0, 1.0), false)
        }
    };

    let (hodges_lehmann_diff, hodges_lehmann_ci, ci_actual_confidence_percent) = hodges_lehmann_two_sample(a, b, confidence)?;
    let cliffs_delta = (u_a - u_b) / (naf * nbf);
    let rank_biserial_r = cliffs_delta;

    Ok(MannWhitneyResult {
        n_a: na,
        n_b: nb,
        median_a,
        median_b,
        u_a,
        u_b,
        u_min,
        p_value,
        exact_p_used,
        two_tailed,
        hodges_lehmann_diff,
        hodges_lehmann_ci,
        ci_actual_confidence_percent,
        cliffs_delta,
        rank_biserial_r,
    })
}

/// Exact distribution-free two-sided Mann–Whitney order-statistic index `k` (1-based) and achieved
/// coverage fraction for untied samples of sizes `na, nb <= 30` (Hollander, Wolfe & Chicken 2014, Ch. 4.3).
fn mann_whitney_exact_hl_ci_rank(na: usize, nb: usize, confidence: ConfidenceLevel) -> (usize, f64) {
    let (n1, n2) = if na <= nb { (na, nb) } else { (nb, na) };
    let m = n1 * n2;
    let half_m = m / 2;
    let mut dp = vec![vec![0_u128; half_m + 1]; n1 + 1];
    dp[0][0] = 1;
    for total in 1..=(n1 + n2) {
        let i_max = total.min(n1);
        for i in (1..=i_max).rev() {
            let j = total - i;
            if j > n2 {
                continue;
            }
            for u in j..=half_m {
                dp[i][u] += dp[i - 1][u - j];
            }
        }
    }
    let mut total_ways = 1_u128;
    for k in 1..=n1 {
        total_ways = total_ways * ((n2 + k) as u128) / (k as u128);
    }
    let total_f = total_ways as f64;
    let target = confidence.percent() / 100.0;
    let mut cum = dp[n1][0];
    let mut best_k = 1_usize;
    let mut best_cov = (1.0 - 2.0 * (cum as f64) / total_f).clamp(0.0, 1.0);
    if best_cov + 1e-14 < target {
        return (1, best_cov);
    }
    for k in 2..=half_m {
        cum += dp[n1][k - 1];
        let cov = (1.0 - 2.0 * (cum as f64) / total_f).clamp(0.0, 1.0);
        if cov + 1e-14 >= target {
            best_k = k;
            best_cov = cov;
        } else {
            break;
        }
    }
    (best_k, best_cov)
}

/// Upper bound on achievable two-sided Mann–Whitney CI coverage (%) for sample sizes `na, nb <= 30`.
pub(super) fn mann_whitney_max_achievable_coverage_percent(na: usize, nb: usize) -> Option<f64> {
    if na >= 1 && nb >= 1 && na <= 30 && nb <= 30 {
        let (n1, n2) = if na <= nb { (na, nb) } else { (nb, na) };
        let mut total_ways = 1_u128;
        for k in 1..=n1 {
            total_ways = total_ways * ((n2 + k) as u128) / (k as u128);
        }
        Some(100.0 * (1.0 - 2.0 / (total_ways as f64)).clamp(0.0, 1.0))
    } else {
        None
    }
}

/// Point estimate, two-sided Hodges–Lehmann CI, and achieved small-sample confidence percentage.
type HodgesLehmannShiftWithCoverage = (f64, Option<(f64, f64)>, Option<f64>);

/// Hodges & Lehmann (1963) two-sample shift estimator (median of all `a_i - b_j`) with the Moses
/// distribution-free interval `[D_(k), D_(mn + 1 - k)]` (Hollander, Wolfe & Chicken 2014, ch. 4).
///
/// Uses exact Mann–Whitney DP critical values for small untied samples (`na <= 30 && nb <= 30`),
/// exact sorted differences when `m * n <= 40_000`, and exact `O((m + n) log(m + n))`
/// monotone-matrix selection (`select_kth_pairwise_sum`, cf. Monahan 1984) for both the shift
/// median and the large-sample normal-approximated Moses CI (`floor(mn/2 - z sqrt(mn(m+n+1)/12))`)
/// when `m * n > 40_000`.
fn hodges_lehmann_two_sample(a: &[f64], b: &[f64], confidence: ConfidenceLevel) -> Result<HodgesLehmannShiftWithCoverage, StatsError> {
    let na = a.len();
    let nb = b.len();
    let m = na.saturating_mul(nb);
    if na == 0 || nb == 0 {
        return Err(StatsError::EmptyInput);
    }
    let (hl, lo_ci_val, hi_ci_val, actual_conf_pct) = if m > 40_000 {
        let sa = sorted_finite(a);
        let mut neg_b: Vec<f64> = b.iter().map(|&v| -v).collect();
        neg_b.sort_by(f64::total_cmp);
        let m_u64 = (na as u64) * (nb as u64);
        let hl = if m_u64 % 2 == 1 {
            select_kth_pairwise_sum(&sa, &neg_b, m_u64 / 2)?
        } else {
            let mid_lo = select_kth_pairwise_sum(&sa, &neg_b, m_u64 / 2 - 1)?;
            let mid_hi = select_kth_pairwise_sum(&sa, &neg_b, m_u64 / 2)?;
            0.5 * mid_lo + 0.5 * mid_hi
        };
        if !hl.is_finite() {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "hodges_lehmann", value: hl }));
        }
        if na + nb < 4 {
            return Ok((hl, None, None));
        }
        let z = norm_two_sided_isf(confidence.alpha())?;
        let naf = na as f64;
        let nbf = nb as f64;
        let mu = 0.5 * naf * nbf;
        let sd = (naf * nbf * (naf + nbf + 1.0) / 12.0).sqrt();
        let k = (mu - z * sd).floor().max(1.0) as u64;
        let lo_idx = k.saturating_sub(1).min(m_u64 - 1);
        let hi_idx = (m_u64 - k).min(m_u64 - 1);
        let ci_lo = select_kth_pairwise_sum(&sa, &neg_b, lo_idx)?;
        let ci_hi = select_kth_pairwise_sum(&sa, &neg_b, hi_idx)?;
        (hl, ci_lo, ci_hi, None)
    } else {
        let mut diffs = Vec::with_capacity(m);
        for &va in a {
            for &vb in b {
                diffs.push(va - vb);
            }
        }
        diffs.sort_by(f64::total_cmp);
        let hl = quantile_type6_sorted(&diffs, 0.5).or_else(|| diffs.first().copied()).ok_or(StatsError::EmptyInput)?;
        if !hl.is_finite() {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "hodges_lehmann", value: hl }));
        }
        if na + nb < 4 {
            return Ok((hl, None, None));
        }
        let sa = sorted_finite(a);
        let sb = sorted_finite(b);
        let untied = sa.windows(2).all(|w| w[0] < w[1]) && sb.windows(2).all(|w| w[0] < w[1]) && !has_cross_ties(&sa, &sb);
        let (k, actual_conf_pct) = if na <= 30 && nb <= 30 && untied {
            let (exact_k, cov) = mann_whitney_exact_hl_ci_rank(na, nb, confidence);
            (exact_k, Some(100.0 * cov))
        } else {
            let z = norm_two_sided_isf(confidence.alpha())?;
            let naf = na as f64;
            let nbf = nb as f64;
            let mu = 0.5 * naf * nbf;
            let sd = (naf * nbf * (naf + nbf + 1.0) / 12.0).sqrt();
            ((mu - z * sd).floor().max(1.0) as usize, None)
        };
        let lo_idx = k.saturating_sub(1).min(m - 1);
        let hi_idx = (m - k).min(m - 1);
        (hl, diffs[lo_idx], diffs[hi_idx], actual_conf_pct)
    };
    let ci = if lo_ci_val.is_finite() && hi_ci_val.is_finite() { Some((lo_ci_val, hi_ci_val)) } else { None };
    Ok((hl, ci, actual_conf_pct))
}

/// Output of Wilcoxon matched-pairs signed-rank test (Wilcoxon 1945; Pratt 1959).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WilcoxonMatchedResult {
    /// Total number of paired observations supplied before any zero-difference handling.
    pub n_pairs: usize,
    /// Effective non-zero pair count contributing to signed ranks (equal to `n_pairs` minus exact zero differences in classic mode, or total `n_pairs` in Pratt mode).
    pub n_effective: usize,
    /// Sum of signed ranks for positive differences `a_i > b_i` (`W+`).
    pub w_plus: f64,
    /// Sum of signed ranks for negative differences `a_i < b_i` (`W-`).
    pub w_minus: f64,
    /// Wilcoxon test statistic (`W+ - W-` or `min(W+, W-)` depending on mode/tail reporting).
    pub w_stat: f64,
    /// *P* value testing symmetric median difference around zero (`two_tailed` vs observed-direction one-tailed; exact DP when `(!use_pratt_zeros || n_zeros == 0)` with `n_effective <= 25` and no ties, else tie-/continuity-corrected normal).
    pub p_value: f64,
    /// True if exact DP distribution was used; false for asymptotic normal approximation.
    pub exact_p_used: bool,
    /// True for two-sided *P* value; false for directional one-sided test.
    pub two_tailed: bool,
    /// Hodges–Lehmann (1963) Walsh-average pseudomedian computed over all paired differences (including exact zeros).
    pub hodges_lehmann_pseudomedian: f64,
    /// Two-sided order-statistic CI `(lower, upper)` over pairwise Walsh averages `(d_i + d_j)/2` at configured `ConfidenceLevel` (`None` if insufficient pairs).
    pub hodges_lehmann_ci: Option<(f64, f64)>,
    /// Exact achieved confidence percentage when small-sample (`n_pairs <= 30`, untied) exact signed-rank critical values were used (`None` for asymptotic normal or unavailable CIs).
    #[serde(default)]
    pub ci_actual_confidence_percent: Option<f64>,
    /// Kerby (2014) matched-pairs rank-biserial correlation `(W+ - W-) / (W+ + W-)` in `[-1, 1]`.
    pub rank_biserial_r: f64,
}

/// Wilcoxon matched-pairs signed-rank test (`use_pratt_zeros = false` uses classic zero exclusion;
/// `true` uses Pratt's zero-inclusion tie adjustment).
///
/// Signed-rank statistic `W+` (Wilcoxon 1945). Classic mode drops zero differences and uses the exact
/// null distribution for `n <= 25` without ties, otherwise the normal approximation with tie
/// correction and 0.5 continuity correction (Hollander, Wolfe & Chicken 2014, ch. 3). Pratt mode
/// (Pratt 1959) ranks zeros with the nonzero `|d|` and discards their ranks: when `n_zeros == 0`,
/// it is identical to classic mode (and uses the exact DP for `n <= 25` without ties); when
/// `n_zeros > 0`, it uses the Pratt normal approximation with mean `[n(n+1) - z(z+1)] / 4` and variance
/// `[n(n+1)(2n+1) - z(z+1)(2z+1) - sum_{nonzero}(t^3 - t)/2] / 24`, where the tie correction
/// sum includes only **nonzero** tied groups so zeros are not double-counted. The pseudomedian is
/// the Hodges & Lehmann (1963) Walsh-average estimator computed on **all** differences (zeros
/// included, in both modes); the matched-pairs rank-biserial `r = (W+ - W-) / (W+ + W-)` follows
/// Kerby (2014).
pub fn wilcoxon_matched_pairs(
    a: &[f64],
    b: &[f64],
    two_tailed: bool,
    confidence: ConfidenceLevel,
    use_pratt_zeros: bool,
) -> Result<WilcoxonMatchedResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.len() != b.len() {
        return Err(StatsError::DimensionMismatch { expected: a.len(), actual: b.len() });
    }
    let diffs_all: Vec<f64> = a.iter().zip(b.iter()).map(|(&x, &y)| x - y).collect();
    let diffs_work: Vec<f64> = if use_pratt_zeros { diffs_all.clone() } else { diffs_all.iter().copied().filter(|&d| d != 0.0).collect() };
    let n_eff = diffs_work.len();
    if n_eff == 0 {
        return Ok(WilcoxonMatchedResult {
            n_pairs: a.len(),
            n_effective: 0,
            w_plus: 0.0,
            w_minus: 0.0,
            w_stat: 0.0,
            p_value: 1.0,
            exact_p_used: true,
            two_tailed,
            hodges_lehmann_pseudomedian: 0.0,
            hodges_lehmann_ci: None,
            ci_actual_confidence_percent: None,
            rank_biserial_r: 0.0,
        });
    }

    let abs_diffs: Vec<f64> = diffs_work.iter().map(|d| d.abs()).collect();
    let ranked = rank_with_ties(&abs_diffs)?;
    let mut w_plus = 0.0_f64;
    let mut w_minus = 0.0_f64;
    let mut n_zeros = 0_usize;
    for (&d, &r) in diffs_work.iter().zip(ranked.ranks.iter()) {
        if d > 0.0 {
            w_plus += r;
        } else if d < 0.0 {
            w_minus += r;
        } else {
            n_zeros += 1;
        }
    }

    let (p_value, exact_p_used) = if (!use_pratt_zeros || n_zeros == 0) && n_eff <= 25 && ranked.tie_counts.is_empty() {
        let max_sum = n_eff * (n_eff + 1) / 2;
        (wilcoxon_exact_p(n_eff, w_plus.round() as usize, max_sum, two_tailed), true)
    } else {
        let nf = n_eff as f64;
        let zf = n_zeros as f64;
        let mean_w = 0.25 * (nf * (nf + 1.0) - zf * (zf + 1.0));
        // Subtract any zero-difference tie group (t_0^3 - t_0) so zeros are not double-counted
        // alongside the Pratt -d_0(d_0+1)(2d_0+1) term.
        let zero_tie_corr = if n_zeros > 1 { zf * (zf * zf - 1.0) } else { 0.0 };
        let nonzero_tie_sum = (ranked.tie_sum_t3_minus_t - zero_tie_corr).max(0.0);
        let var_w = ((nf * (nf + 1.0) * (2.0 * nf + 1.0) - zf * (zf + 1.0) * (2.0 * zf + 1.0) - 0.5 * nonzero_tie_sum) / 24.0).max(0.0);
        if n_zeros == n_eff || var_w <= 0.0 {
            let p = if (w_plus - mean_w).abs() <= 1e-12 { 1.0 } else { 0.0 };
            (p, false)
        } else {
            let sd_w = var_w.sqrt();
            let p = if two_tailed {
                let cc = (w_plus - mean_w).abs() - 0.5;
                if cc <= 0.0 { 1.0 } else { (2.0 * norm_sf(cc / sd_w)).min(1.0) }
            } else {
                let cc = ((w_plus - mean_w).abs() - 0.5).max(0.0);
                norm_sf(cc / sd_w).clamp(0.0, 1.0)
            };
            (p.clamp(0.0, 1.0), false)
        }
    };

    let (hl, hl_ci, ci_actual_confidence_percent) = hodges_lehmann_one_sample_with_coverage(&diffs_all, confidence)?;
    let total_rank_sum = w_plus + w_minus;
    let rank_biserial_r = if total_rank_sum > 0.0 { (w_plus - w_minus) / total_rank_sum } else { 0.0 };

    Ok(WilcoxonMatchedResult {
        n_pairs: a.len(),
        n_effective: n_eff,
        w_plus,
        w_minus,
        w_stat: w_plus - w_minus,
        p_value,
        exact_p_used,
        two_tailed,
        hodges_lehmann_pseudomedian: hl,
        hodges_lehmann_ci: hl_ci,
        ci_actual_confidence_percent,
        rank_biserial_r,
    })
}

/// Output of an exact binomial paired sign test on matched differences `a_i - b_i`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedSignTestResult {
    /// Total number of paired observations supplied.
    pub n_total: usize,
    /// Number of strictly positive differences (`a_i - b_i > 0`).
    pub n_positive: usize,
    /// Number of strictly negative differences (`a_i - b_i < 0`).
    pub n_negative: usize,
    /// Number of exact zero ties (`a_i == b_i`) excluded from the binomial sign count.
    pub n_ties: usize,
    /// Total non-zero differences `n_positive + n_negative`.
    pub n_nonzero: usize,
    /// Sample median across all `n_total` differences `a_i - b_i` (zeros included).
    pub median_difference: f64,
    /// Exact binomial order-statistic confidence interval `(lower, upper)` for the median difference (`None` when `n_total < 3`).
    pub ci: Option<(f64, f64)>,
    /// Actual discrete achieved coverage percentage `>= nominal` for `ci` (`None` when `ci` is `None`).
    pub ci_actual_confidence_percent: Option<f64>,
    /// Exact `Binomial(n_nonzero, 0.5)` sign-test *P* value (`two_tailed` doubles smaller tail vs observed-direction smaller tail).
    pub p_value: f64,
    /// True for two-sided *P* value; false for directional one-sided test.
    pub two_tailed: bool,
}

/// Exact paired sign test on matched pairs `(a_i, b_i)` evaluating `d_i = a_i - b_i` after
/// dropping exact zero ties (`d_i == 0`), with exact order-statistic binomial CI for median `d`.
///
/// Exact `Binomial(n_nonzero, 1/2)` sign test (Dixon & Mood 1946; Conover 1999) and the
/// distribution-free order-statistic interval for the median (Conover 1999). The two-sided *p*-value
/// doubles the smaller tail; the one-sided *p*-value is the smaller tail, i.e. it is taken in the
/// observed direction rather than a pre-specified one. The median CI is computed over all
/// differences, zeros included.
pub fn paired_sign_test(a: &[f64], b: &[f64], two_tailed: bool, confidence: ConfidenceLevel) -> Result<PairedSignTestResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    if a.len() != b.len() {
        return Err(StatsError::DimensionMismatch { expected: a.len(), actual: b.len() });
    }
    let diffs: Vec<f64> = a.iter().zip(b.iter()).map(|(&x, &y)| x - y).collect();
    validate_finite_slice(&diffs)?;
    let n_total = diffs.len();
    let n_positive = diffs.iter().filter(|&&d| d > 0.0).count();
    let n_negative = diffs.iter().filter(|&&d| d < 0.0).count();
    let n_ties = n_total - n_positive - n_negative;
    let n_nonzero = n_positive + n_negative;

    let p_value = if n_nonzero == 0 {
        1.0
    } else {
        let k_min = n_positive.min(n_negative);
        let tail = reg_beta(0.5, (n_nonzero - k_min) as f64, (k_min + 1) as f64)?.clamp(0.0, 1.0);
        if two_tailed { if k_min * 2 == n_nonzero { 1.0 } else { (2.0 * tail).clamp(0.0, 1.0) } } else { tail }
    };

    let sorted = sorted_finite(&diffs);
    let (_, median_difference, _) = quartiles_hinge(&sorted).ok_or(StatsError::EmptyInput)?;
    let (ci, ci_actual_confidence_percent) = match exact_binomial_median_ci(&diffs, confidence) {
        Ok(mci) => (Some((mci.lower, mci.upper)), Some(mci.actual_confidence_percent)),
        Err(_) => (None, None),
    };

    Ok(PairedSignTestResult {
        n_total,
        n_positive,
        n_negative,
        n_ties,
        n_nonzero,
        median_difference,
        ci,
        ci_actual_confidence_percent,
        p_value,
        two_tailed,
    })
}

/// Output of a two-sample Kolmogorov–Smirnov test.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TwoSampleKsResult {
    /// Sample size in group A.
    pub n_a: usize,
    /// Sample size in group B.
    pub n_b: usize,
    /// Kolmogorov–Smirnov supremum empirical CDF discrepancy `D = sup_x |F_a(x) - F_b(x)|` in `[0, 1]`.
    pub d_statistic: f64,
    /// Two-sided *P* value testing equality of distributions (exact Smirnov lattice path DP when `n_a <= 25`, `n_b <= 25`, and cross-tie-free; else Smirnov asymptotic series).
    pub p_value: f64,
    /// True when exact Smirnov combinatorial DP was used; false when asymptotic Smirnov series was used.
    pub exact_p_used: bool,
}

/// Two-sample Kolmogorov–Smirnov test (exact Smirnov lattice path DP when `n_a <= 25` and `n_b <= 25`
/// with no ties between the samples, Smirnov asymptotic series otherwise).
///
/// `D = sup |F_a(x) - F_b(x)|`. The exact two-sided *p*-value `P(D >= d)` counts lattice paths
/// (Hodges 1958); the asymptotic *p*-value evaluates the limiting Kolmogorov–Smirnov series at
/// `lambda = D sqrt(n_a n_b / (n_a + n_b))` (Smirnov 1948) without a finite-sample correction.
pub fn ks_two_sample(a: &[f64], b: &[f64]) -> Result<TwoSampleKsResult, StatsError> {
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    let sa = sorted_finite(a);
    let sb = sorted_finite(b);
    let na = sa.len();
    let nb = sb.len();
    let d_stat = compute_ks_d(&sa, &sb);

    let (p_value, exact_p_used) = if na <= 25 && nb <= 25 && !has_cross_ties(&sa, &sb) {
        (ks_exact_smirnov_p(na, nb, d_stat), true)
    } else {
        let naf = na as f64;
        let nbf = nb as f64;
        let eff = (naf * nbf / (naf + nbf)).sqrt();
        (ks_asymptotic_p(d_stat * eff), false)
    };

    Ok(TwoSampleKsResult { n_a: na, n_b: nb, d_statistic: d_stat, p_value, exact_p_used })
}

fn compute_ks_d(sa: &[f64], sb: &[f64]) -> f64 {
    let na = sa.len() as f64;
    let nb = sb.len() as f64;
    let mut i = 0_usize;
    let mut j = 0_usize;
    let mut d_max = 0.0_f64;
    while i < sa.len() && j < sb.len() {
        let v = sa[i].min(sb[j]);
        while i < sa.len() && sa[i] == v {
            i += 1;
        }
        while j < sb.len() && sb[j] == v {
            j += 1;
        }
        let diff = ((i as f64) / na - (j as f64) / nb).abs();
        d_max = d_max.max(diff);
    }
    d_max
}

fn has_cross_ties(sa: &[f64], sb: &[f64]) -> bool {
    let mut i = 0;
    let mut j = 0;
    while i < sa.len() && j < sb.len() {
        if sa[i] == sb[j] {
            return true;
        }
        if sa[i] < sb[j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    false
}

/// Exact two-sided Smirnov `P(D >= d_obs)` by counting monotone lattice paths from `(0, 0)` to
/// `(n_a, n_b)` that stay strictly inside `|i/n_a - j/n_b| < d_obs` (Hodges 1958).
fn ks_exact_smirnov_p(na: usize, nb: usize, d_obs: f64) -> f64 {
    let naf = na as f64;
    let nbf = nb as f64;
    let eps = 1e-12;
    let mut dp = vec![vec![0.0_f64; nb + 1]; na + 1];
    dp[0][0] = 1.0;
    for i in 0..=na {
        for j in 0..=nb {
            if i == 0 && j == 0 {
                continue;
            }
            let diff = ((i as f64) / naf - (j as f64) / nbf).abs();
            if diff + eps >= d_obs {
                dp[i][j] = 0.0;
            } else {
                let from_i = if i > 0 { dp[i - 1][j] } else { 0.0 };
                let from_j = if j > 0 { dp[i][j - 1] } else { 0.0 };
                dp[i][j] = from_i + from_j;
            }
        }
    }
    let total = (1..=na).fold(1.0_f64, |acc, k| acc * ((nb + k) as f64) / (k as f64));
    let paths_strictly_below = dp[na][nb];
    (1.0 - paths_strictly_below / total).clamp(0.0, 1.0)
}
