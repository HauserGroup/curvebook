//! Rank-based one-way tests: Kruskal–Wallis for independent groups and Friedman for matched blocks.
//!
//! References:
//! - Kruskal, W. H., & Wallis, W. A. (1952). Use of ranks in one-criterion variance analysis.
//!   *Journal of the American Statistical Association* 47(260):583–621.
//!   doi:10.1080/01621459.1952.10483441
//! - Iman, R. L., Quade, D., & Alexander, D. A. (1975). Exact probability levels for the
//!   Kruskal–Wallis test. *Selected Tables in Mathematical Statistics* 3:329–384.
//! - Streitberg, B., & Röhmel, J. (1986). Exact distributions for permutation and rank tests:
//!   An introduction to some recently published algorithms. *Statistical Software Newsletter*
//!   12(1):10–17.
//! - Baglivo, J., Olivier, D., & Pagano, M. (1992). Methods for exact goodness-of-fit tests.
//!   *Journal of the American Statistical Association* 87(418):464–469.
//!   doi:10.1080/01621459.1992.10475227
//! - Meyer, J. P., & Seaman, M. A. (2013). A comparison of the exact Kruskal–Wallis distribution
//!   to asymptotic approximations for all sample sizes up to 105. *Journal of the American
//!   Statistical Association* 108(504):1418–1428. doi:10.1080/01621459.2013.800759
//! - Hollander, M., Wolfe, D. A., & Chicken, E. (2014). *Nonparametric Statistical Methods*
//!   (3rd ed.). Wiley.
//! - Friedman, M. (1937). The use of ranks to avoid the assumption of normality implicit in the
//!   analysis of variance. *Journal of the American Statistical Association* 32(200):675–701.
//!   doi:10.1080/01621459.1937.10503522
//! - Kendall, M. G., & Babington Smith, B. (1939). The problem of *m* rankings. *Annals of
//!   Mathematical Statistics* 10(3):275–287.
//! - Tomczak, M., & Tomczak, E. (2014). The need to report effect size estimates revisited. An
//!   overview of some recommended measures of effect size. *Trends in Sport Sciences* 1(21):19–25.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.

use curvebook_math::dist::chi2_sf;
use curvebook_math::sum::NeumaierSum;
use serde::{Deserialize, Serialize};

use crate::anova::results::{FriedmanResult, KruskalWallisResult};
use crate::common::{rank_with_ties, validate_finite_slice};
use crate::error::StatsError;

/// Method used to evaluate the Kruskal–Wallis omnibus *P* value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KruskalWallisPMethod {
    /// Exact conditional permutation distribution (Streitberg & Röhmel 1986 1D shift DP for
    /// `k = 2` up to `N <= 250`; exact 2D rank-sum DP for `k = 3` up to `N <= 25` within
    /// `2_500_000` cells; exact meet-in-the-middle / symmetry-canonicalized branch-and-bound for
    /// `k >= 4` within `100_000` splits), falling back to the tie-corrected $\chi^2(k - 1)$
    /// approximation above that budget (Kruskal & Wallis 1952; Meyer & Seaman 2013).
    #[default]
    Auto,
    /// Tie-corrected asymptotic $\chi^2(k - 1)$ approximation (Kruskal & Wallis 1952; Zar 2010,
    /// ch. 10), matching `R` `stats::kruskal.test`.
    Asymptotic,
}

/// Maximum combined sample size for exact 1D permutation DP when `k == 2`.
const KW_EXACT_K2_MAX_N: usize = 250;

/// Maximum `f64` DP table cells allocated per tail pass when `k == 2` (~32 MiB cap).
const KW_EXACT_K2_MAX_CELLS: usize = 4_000_000;

/// Maximum combined sample size for exact 2D `u64` rank-sum DP when `k == 3`.
///
/// At `N = 25`, `25! / (8! 8! 9!) ~= 1.06e11 << u64::MAX`, so exact integer counts never overflow.
const KW_EXACT_K3_MAX_N: usize = 25;

/// Maximum `u64` DP table cells allocated for `k == 3` (~20 MiB cap).
const KW_EXACT_K3_MAX_CELLS: usize = 2_500_000;

/// Maximum combined sample size for exact `k >= 4` meet-in-the-middle / branch-and-bound.
const KW_EXACT_KGE4_MAX_N: usize = 30;

/// Maximum worst-case meet-in-the-middle split leaves for exact `k >= 4` (~10 ms release budget).
const KW_EXACT_KGE4_MAX_SPLITS: u128 = 100_000;

const fn gcd_usize(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

const fn gcd_u128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

fn lcm_u128(a: u128, b: u128) -> Option<u128> {
    if a == 0 || b == 0 {
        return None;
    }
    (a / gcd_u128(a, b)).checked_mul(b)
}

fn binom_f64(n: usize, k: usize) -> f64 {
    let k_eff = k.min(n.saturating_sub(k));
    let mut acc = 1.0_f64;
    for j in 1..=k_eff {
        acc = acc * ((n - k_eff + j) as f64) / (j as f64);
    }
    acc
}

fn binom_u128(n: usize, k: usize) -> Option<u128> {
    if k > n {
        return Some(0);
    }
    let k_eff = k.min(n - k);
    let mut acc = 1_u128;
    for j in 1..=k_eff {
        let num = (n - k_eff + j) as u128;
        let den = j as u128;
        let g = gcd_u128(acc, den);
        acc = (acc / g).checked_mul(num / (den / g))?;
    }
    Some(acc)
}

fn multinomial_u128(ns: &[usize]) -> Option<u128> {
    let mut rem = ns.iter().try_fold(0_usize, |acc, &ni| acc.checked_add(ni))?;
    let mut acc = 1_u128;
    for &ni in ns {
        acc = acc.checked_mul(binom_u128(rem, ni)?)?;
        rem = rem.checked_sub(ni)?;
    }
    Some(acc)
}

/// Count subsets of size `k` from non-negative integer `weights` whose sum is at most `cap`,
/// using the Streitberg & Röhmel (1986) shift algorithm with GCD scaling and suffix-minimum pruning.
fn count_subsets_sum_le_f64(weights: &[usize], k: usize, cap: usize) -> Option<f64> {
    let n = weights.len();
    if k == 0 {
        return Some(1.0);
    }
    if k > n {
        return Some(0.0);
    }
    let mut g = 0_usize;
    for &w in weights {
        if w > 0 {
            g = gcd_usize(g, w);
            if g == 1 {
                break;
            }
        }
    }
    if g == 0 {
        return Some(binom_f64(n, k));
    }
    let mut ws: Vec<usize> = if g > 1 { weights.iter().map(|&w| w / g).collect() } else { weights.to_vec() };
    let scaled_cap = cap / g;
    ws.sort_unstable();

    let min_possible: usize = ws.iter().take(k).sum();
    if scaled_cap < min_possible {
        return Some(0.0);
    }
    let max_possible: usize = ws.iter().rev().take(k).sum();
    if scaled_cap >= max_possible {
        return Some(binom_f64(n, k));
    }

    let stride = scaled_cap.checked_add(1)?;
    let cells = k.checked_add(1)?.checked_mul(stride)?;
    if cells > KW_EXACT_K2_MAX_CELLS {
        return None;
    }

    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0_usize);
    let mut acc_p = 0_usize;
    for &w in &ws {
        acc_p = acc_p.checked_add(w)?;
        prefix.push(acc_p);
    }

    let mut dp = vec![0.0_f64; cells];
    if let Some(first) = dp.first_mut() {
        *first = 1.0;
    }
    let mut row_max: Vec<Option<usize>> = vec![None; k + 1];
    if let Some(slot) = row_max.first_mut() {
        *slot = Some(0);
    }

    for (idx, &w) in ws.iter().enumerate() {
        let rem = n - 1 - idx;
        let i_min = k.saturating_sub(rem).max(1);
        let i_max = k.min(idx + 1);
        for i in (i_min..=i_max).rev() {
            let Some(prev_max) = row_max.get(i - 1).copied().flatten() else {
                continue;
            };
            let need = k - i;
            let suffix_min = prefix.get(idx + 1 + need).copied().unwrap_or(0).saturating_sub(prefix.get(idx + 1).copied().unwrap_or(0));
            if suffix_min > scaled_cap {
                continue;
            }
            let eff_cap = scaled_cap - suffix_min;
            if w > eff_cap {
                continue;
            }
            let hi = eff_cap.min(prev_max.saturating_add(w));
            let split_at = i * stride;
            let (before, after) = dp.split_at_mut(split_at);
            let prev_start = (i - 1) * stride;
            let prev_row = before.get(prev_start..split_at)?;
            let cur_row = after.get_mut(..stride)?;
            if let (Some(dst), Some(src)) = (cur_row.get_mut(w..=hi), prev_row.get(..=hi - w)) {
                for (d, &s) in dst.iter_mut().zip(src.iter()) {
                    *d += s;
                }
            }
            if let Some(slot) = row_max.get_mut(i) {
                *slot = Some(slot.map_or(hi, |m| m.max(hi)));
            }
        }
    }

    let Some(final_max) = row_max.get(k).copied().flatten() else {
        return Some(0.0);
    };
    let mut tail_sum = NeumaierSum::new();
    if let Some(final_row) = dp.get(k * stride..=k * stride + final_max) {
        for &c in final_row {
            tail_sum.add(c);
        }
    }
    Some(tail_sum.total())
}

/// Exact conditional permutation *P* value for `k == 2` Kruskal–Wallis (algebraically identical
/// to the two-sided Mann–Whitney symmetric deviation test `P(|S - E[S]| >= |S_obs - E[S]|)`).
fn kw_exact_k2_p(two_ranks: &[usize], na: usize, nb: usize) -> Option<f64> {
    let n = two_ranks.len();
    if n > KW_EXACT_K2_MAX_N || na == 0 || nb == 0 || na + nb != n {
        return None;
    }
    let (k, obs_sum) = if na <= nb { (na, two_ranks.iter().take(na).sum::<usize>()) } else { (nb, two_ranks.iter().skip(na).sum::<usize>()) };
    let min_r = two_ranks.iter().copied().min()?;
    let max_r = two_ranks.iter().copied().max()?;
    if min_r == max_r {
        return Some(1.0);
    }
    let exp_sum = k.checked_mul(n.checked_add(1)?)?;
    let obs_dev = obs_sum.abs_diff(exp_sum);
    if obs_dev == 0 {
        return Some(1.0);
    }
    let total_ways = binom_f64(n, k);
    if !total_ways.is_finite() || total_ways <= 0.0 {
        return None;
    }
    let k_min_r = k.checked_mul(min_r)?;
    let k_max_r = k.checked_mul(max_r)?;
    let lo_cnt = if let Some(cap_lo) = exp_sum.checked_sub(obs_dev).and_then(|t| t.checked_sub(k_min_r)) {
        let w_lo: Vec<usize> = two_ranks.iter().map(|&r| r - min_r).collect();
        count_subsets_sum_le_f64(&w_lo, k, cap_lo)?
    } else {
        0.0
    };
    let hi_cnt = if let Some(cap_hi) = exp_sum.checked_add(obs_dev).and_then(|t| k_max_r.checked_sub(t)) {
        let w_hi: Vec<usize> = two_ranks.iter().map(|&r| max_r - r).collect();
        count_subsets_sum_le_f64(&w_hi, k, cap_hi)?
    } else {
        0.0
    };
    Some(((lo_cnt + hi_cnt) / total_ways).clamp(0.0, 1.0))
}

/// Add shifted counts along the first rank-sum axis `(c1 - 1, c2) -> (c1, c2)` in `k == 3` 2D DP.
fn kw_k3_add_axis1(src_plane: &[u64], dst_plane: &mut [u64], pm: (usize, usize), w: usize, max_s1: usize, stride2: usize) -> Option<(usize, usize)> {
    let (pm1, pm2) = pm;
    if w > max_s1 {
        return None;
    }
    let s1_hi = pm1.min(max_s1 - w);
    for s1 in 0..=s1_hi {
        let src_off = s1 * stride2;
        let dst_off = (s1 + w) * stride2;
        let src_row = src_plane.get(src_off..=src_off + pm2)?;
        let dst_row = dst_plane.get_mut(dst_off..=dst_off + pm2)?;
        for (d, &s) in dst_row.iter_mut().zip(src_row.iter()) {
            *d = d.checked_add(s)?;
        }
    }
    Some((s1_hi + w, pm2))
}

/// Add shifted counts along the second rank-sum axis `(c1, c2 - 1) -> (c1, c2)` in `k == 3` 2D DP.
fn kw_k3_add_axis2(src_plane: &[u64], dst_plane: &mut [u64], pm: (usize, usize), w: usize, max_s2: usize, stride2: usize) -> Option<(usize, usize)> {
    let (pm1, pm2) = pm;
    if w > max_s2 {
        return None;
    }
    let s2_hi = pm2.min(max_s2 - w);
    for s1 in 0..=pm1 {
        let row_off = s1 * stride2;
        let src_row = src_plane.get(row_off..=row_off + s2_hi)?;
        let dst_row = dst_plane.get_mut(row_off + w..=row_off + w + s2_hi)?;
        for (d, &s) in dst_row.iter_mut().zip(src_row.iter()) {
            *d = d.checked_add(s)?;
        }
    }
    Some((pm1, s2_hi + w))
}

/// Scoring context for the final `k == 3` DP plane.
struct K3ScoreCtx {
    fm: (usize, usize),
    stride2: usize,
    ns: [usize; 3],
    lcm: u128,
    min_r: usize,
    g: usize,
    total_two_rank: u128,
    q_obs: u128,
}

/// Sum exact tail counts `Q(S_1, S_2, S_3) >= Q_obs` over the final `k == 3` DP plane.
fn kw_k3_tail_prob(final_plane: &[u64], ctx: &K3ScoreCtx) -> Option<f64> {
    let [n1, n2, n3] = ctx.ns;
    let (m1, m2, m3) = (ctx.lcm / (n1 as u128), ctx.lcm / (n2 as u128), ctx.lcm / (n3 as u128));
    let g128 = ctx.g as u128;
    let base1 = (n1 as u128).checked_mul(ctx.min_r as u128)?;
    let base2 = (n2 as u128).checked_mul(ctx.min_r as u128)?;
    let mut total_cnt = 0_u128;
    let mut hits = 0_u128;
    for s1 in 0..=ctx.fm.0 {
        let row_off = s1 * ctx.stride2;
        let row = final_plane.get(row_off..=row_off + ctx.fm.1)?;
        let s1_orig = base1.checked_add((s1 as u128).checked_mul(g128)?)?;
        let q1 = m1.checked_mul(s1_orig.checked_mul(s1_orig)?)?;
        for (s2, &cnt) in row.iter().enumerate() {
            if cnt == 0 {
                continue;
            }
            let s2_orig = base2.checked_add((s2 as u128).checked_mul(g128)?)?;
            let s3_orig = ctx.total_two_rank.checked_sub(s1_orig)?.checked_sub(s2_orig)?;
            let q = q1.checked_add(m2.checked_mul(s2_orig.checked_mul(s2_orig)?)?)?.checked_add(m3.checked_mul(s3_orig.checked_mul(s3_orig)?)?)?;
            let c128 = u128::from(cnt);
            total_cnt = total_cnt.checked_add(c128)?;
            if q >= ctx.q_obs {
                hits = hits.checked_add(c128)?;
            }
        }
    }
    if total_cnt == 0 {
        return None;
    }
    Some(((hits as f64) / (total_cnt as f64)).clamp(0.0, 1.0))
}

/// Exact conditional permutation *P* value for `k == 3` Kruskal–Wallis via 2D `u64` rank-sum DP
/// over shifted, GCD-scaled doubled mid-ranks (Baglivo, Olivier & Pagano 1992; Meyer & Seaman 2013).
fn kw_exact_k3_p(two_ranks: &[usize], group_sizes: &[usize], lcm: u128, q_obs: u128) -> Option<f64> {
    let n = two_ranks.len();
    if n > KW_EXACT_K3_MAX_N || group_sizes.len() != 3 {
        return None;
    }
    let mut ns = [group_sizes[0], group_sizes[1], group_sizes[2]];
    ns.sort_unstable();
    let [n1, n2, n3] = ns;
    if n1 == 0 || n1 + n2 + n3 != n {
        return None;
    }
    let min_r = two_ranks.iter().copied().min()?;
    let g = two_ranks.iter().try_fold(0_usize, |acc, &r| Some(gcd_usize(acc, r.checked_sub(min_r)?)))?;
    if g == 0 {
        return Some(1.0);
    }
    let mut ws: Vec<usize> = two_ranks.iter().map(|&r| (r - min_r) / g).collect();
    ws.sort_unstable();

    let max_s1: usize = ws.iter().rev().take(n1).sum();
    let max_s2: usize = ws.iter().rev().take(n2).sum();
    let stride1 = max_s1.checked_add(1)?;
    let stride2 = max_s2.checked_add(1)?;
    let plane = stride1.checked_mul(stride2)?;
    let c2_len = n2.checked_add(1)?;
    let total_slots = n1.checked_add(1)?.checked_mul(c2_len)?;
    let cells = total_slots.checked_mul(plane)?;
    if cells > KW_EXACT_K3_MAX_CELLS {
        return None;
    }

    let mut dp = vec![0_u64; cells];
    if let Some(first) = dp.first_mut() {
        *first = 1;
    }
    let mut box_max: Vec<Option<(usize, usize)>> = vec![None; total_slots];
    if let Some(slot) = box_max.first_mut() {
        *slot = Some((0, 0));
    }

    let merge_box =
        |cur: Option<(usize, usize)>, b: (usize, usize)| -> Option<(usize, usize)> { Some(cur.map_or(b, |(a1, a2)| (a1.max(b.0), a2.max(b.1)))) };

    for (idx, &w) in ws.iter().enumerate() {
        let min_c12 = (idx + 1).saturating_sub(n3);
        let c1_max = n1.min(idx + 1);
        for c1 in (0..=c1_max).rev() {
            let c2_min = min_c12.saturating_sub(c1);
            let c2_max = n2.min(idx + 1 - c1);
            for c2 in (c2_min..=c2_max).rev() {
                let dst_slot = c1 * c2_len + c2;
                let dst_off = dst_slot * plane;
                let mut cur_box = box_max.get(dst_slot).copied().flatten();

                if c1 > 0
                    && let Some(pm) = box_max.get((c1 - 1) * c2_len + c2).copied().flatten()
                {
                    let src_off = ((c1 - 1) * c2_len + c2) * plane;
                    let (before, after) = dp.split_at_mut(dst_off);
                    if let Some(b) = kw_k3_add_axis1(before.get(src_off..src_off + plane)?, after.get_mut(..plane)?, pm, w, max_s1, stride2) {
                        cur_box = merge_box(cur_box, b);
                    }
                }
                if c2 > 0
                    && let Some(pm) = box_max.get(c1 * c2_len + (c2 - 1)).copied().flatten()
                {
                    let src_off = (c1 * c2_len + (c2 - 1)) * plane;
                    let (before, after) = dp.split_at_mut(dst_off);
                    if let Some(b) = kw_k3_add_axis2(before.get(src_off..src_off + plane)?, after.get_mut(..plane)?, pm, w, max_s2, stride2) {
                        cur_box = merge_box(cur_box, b);
                    }
                }
                if let Some(slot) = box_max.get_mut(dst_slot) {
                    *slot = cur_box;
                }
            }
        }
    }

    let final_slot = n1 * c2_len + n2;
    let final_plane = dp.get(final_slot * plane..(final_slot + 1) * plane)?;
    let fm = box_max.get(final_slot).copied().flatten()?;
    let total_two_rank: u128 = two_ranks.iter().map(|&r| r as u128).sum();
    kw_k3_tail_prob(final_plane, &K3ScoreCtx { fm, stride2, ns, lcm, min_r, g, total_two_rank, q_obs })
}

/// Reusable 1D subset-sum DP buffers for meet-in-the-middle halves.
struct HalfDpScratch {
    ws_shift: Vec<usize>,
    dp: Vec<u64>,
    row_max: Vec<Option<usize>>,
}

/// Reusable scratch buffers for zero-allocation `k >= 4` meet-in-the-middle leaf evaluation.
struct MitmScratch {
    a_vals: Vec<u128>,
    b_vals: Vec<u128>,
    half_dp: HalfDpScratch,
    qa_hist: Vec<(u128, u64)>,
    qb_hist: Vec<(u128, u64)>,
}

impl MitmScratch {
    fn new(max_n: usize) -> Self {
        Self {
            a_vals: Vec::with_capacity(max_n),
            b_vals: Vec::with_capacity(max_n),
            half_dp: HalfDpScratch { ws_shift: Vec::with_capacity(max_n), dp: Vec::new(), row_max: Vec::with_capacity(max_n + 1) },
            qa_hist: Vec::with_capacity(128),
            qb_hist: Vec::with_capacity(128),
        }
    }
}

/// Advance `idx` (strictly increasing indices in `0..n`) to the next lexicographic combination.
fn next_combination(idx: &mut [usize], n: usize) -> bool {
    let k = idx.len();
    for i in (0..k).rev() {
        let Some(&val) = idx.get(i) else {
            return false;
        };
        if val < n - k + i {
            let next_val = val + 1;
            if let Some(slot) = idx.get_mut(i) {
                *slot = next_val;
            }
            for j in (i + 1)..k {
                let prev = idx.get(j - 1).copied().unwrap_or(0);
                if let Some(slot) = idx.get_mut(j) {
                    *slot = prev + 1;
                }
            }
            return true;
        }
    }
    false
}

/// Build sorted `(q_half, count)` histogram over all `C(m, n_sub)` splits of sorted `ws` into
/// groups of sizes `(n_sub, ws.len() - n_sub)` with multipliers `(m_first, m_second)`.
fn fill_half_q_hist(ws: &[u128], n_sub: usize, m_pair: (u128, u128), buf: &mut HalfDpScratch, out_hist: &mut Vec<(u128, u64)>) -> Option<()> {
    let (m_first, m_second) = m_pair;
    let m = ws.len();
    let min_w = *ws.first()?;
    let total_w: u128 = ws.iter().copied().try_fold(0_u128, u128::checked_add)?;
    buf.ws_shift.clear();
    for &w in ws {
        buf.ws_shift.push(usize::try_from(w.checked_sub(min_w)?).ok()?);
    }
    let max_sum: usize = buf.ws_shift.iter().rev().take(n_sub).sum();
    let stride = max_sum.checked_add(1)?;
    let cells = n_sub.checked_add(1)?.checked_mul(stride)?;
    buf.dp.clear();
    buf.dp.resize(cells, 0_u64);
    if let Some(first) = buf.dp.first_mut() {
        *first = 1;
    }
    buf.row_max.clear();
    buf.row_max.resize(n_sub + 1, None);
    if let Some(slot) = buf.row_max.first_mut() {
        *slot = Some(0);
    }

    for (idx, &w) in buf.ws_shift.iter().enumerate() {
        let rem = m - 1 - idx;
        let i_min = n_sub.saturating_sub(rem).max(1);
        let i_max = n_sub.min(idx + 1);
        for i in (i_min..=i_max).rev() {
            let Some(prev_max) = buf.row_max.get(i - 1).copied().flatten() else {
                continue;
            };
            let hi = (prev_max + w).min(max_sum);
            let split_at = i * stride;
            let (before, after) = buf.dp.split_at_mut(split_at);
            let prev_start = (i - 1) * stride;
            let prev_row = before.get(prev_start..split_at)?;
            let cur_row = after.get_mut(..stride)?;
            if let (Some(dst), Some(src)) = (cur_row.get_mut(w..=hi), prev_row.get(..=hi - w)) {
                for (d, &s) in dst.iter_mut().zip(src.iter()) {
                    *d = d.checked_add(s)?;
                }
            }
            if let Some(slot) = buf.row_max.get_mut(i) {
                *slot = Some(slot.map_or(hi, |cur| cur.max(hi)));
            }
        }
    }

    out_hist.clear();
    let Some(final_max) = buf.row_max.get(n_sub).copied().flatten() else {
        return Some(());
    };
    let final_row = buf.dp.get(n_sub * stride..=n_sub * stride + final_max)?;
    let base = (n_sub as u128).checked_mul(min_w)?;
    for (s_sh, &cnt) in final_row.iter().enumerate() {
        if cnt == 0 {
            continue;
        }
        let s1 = base.checked_add(s_sh as u128)?;
        let s2 = total_w.checked_sub(s1)?;
        let q = m_first.checked_mul(s1.checked_mul(s1)?)?.checked_add(m_second.checked_mul(s2.checked_mul(s2)?)?)?;
        out_hist.push((q, cnt));
    }
    out_hist.sort_unstable_by_key(|&(q, _)| q);
    Some(())
}

/// Exact convex endpoint maximum of `m_first * S_1^2 + m_second * (S_tot - S_1)^2` over subsets
/// of size `n_first` from sorted `vals`.
fn max_pair_q(vals: &[u128], n_first: usize, m_first: u128, m_second: u128, total: u128) -> Option<u128> {
    let lo: u128 = vals.iter().take(n_first).copied().sum();
    let hi: u128 = vals.iter().rev().take(n_first).copied().sum();
    let q_lo = m_first.checked_mul(lo * lo)?.checked_add(m_second.checked_mul((total - lo) * (total - lo))?)?;
    let q_hi = m_first.checked_mul(hi * hi)?.checked_add(m_second.checked_mul((total - hi) * (total - hi))?)?;
    Some(q_lo.max(q_hi))
}

/// Two-pointer count of pairs `(qa, qb)` from ascending histograms achieving `qa + qb >= q_target`.
fn convolve_hist_ge(qa_hist: &[(u128, u64)], qb_hist: &[(u128, u64)], q_target: u128) -> Option<u128> {
    let mut j = qb_hist.len();
    let mut suf = 0_u128;
    let mut hits = 0_u128;
    for &(qa, ca) in qa_hist {
        while let Some(&(qb, cb)) = j.checked_sub(1).and_then(|idx| qb_hist.get(idx)) {
            if qa.saturating_add(qb) >= q_target {
                j -= 1;
                suf = suf.checked_add(u128::from(cb))?;
            } else {
                break;
            }
        }
        hits = hits.checked_add(u128::from(ca).checked_mul(suf)?)?;
    }
    Some(hits)
}

/// Exact count of partitions of sorted `ws` into 4 labeled groups of sorted sizes `ns` achieving
/// `sum_{g=0..4} ms[g] * S_g^2 >= q_target`, via meet-in-the-middle `(n0, n1)` vs `(n2, n3)`.
fn count_k4_mitm(ws: &[u128], ns: [usize; 4], ms: [u128; 4], lcm: u128, q_target: u128, scratch: &mut MitmScratch) -> Option<u128> {
    let n = ws.len();
    let [n0, n1, n2, n3] = ns;
    let [m0, m1, m2, m3] = ms;
    let (m_a, m_b) = (n0 + n1, n2 + n3);
    if m_a + m_b != n {
        return None;
    }
    if q_target == 0 {
        return multinomial_u128(&ns);
    }
    let ways_ab = binom_u128(m_a, n0)?.checked_mul(binom_u128(m_b, n2)?)?;

    let sym_halves = n0 == n2 && n1 == n3;
    let sym_factor: u128 = if sym_halves { 2 } else { 1 };
    let (pick_count, pool_len, shift) = if sym_halves { (m_a - 1, n - 1, 1) } else { (m_a, n, 0) };
    let mut comb: Vec<usize> = (0..pick_count).collect();
    let mut in_a = vec![false; n];
    let mut hits = 0_u128;

    loop {
        in_a.fill(false);
        if sym_halves && let Some(first) = in_a.first_mut() {
            *first = true;
        }
        for &c in &comb {
            if let Some(slot) = in_a.get_mut(c + shift) {
                *slot = true;
            }
        }

        scratch.a_vals.clear();
        scratch.b_vals.clear();
        for (&w, &flag) in ws.iter().zip(in_a.iter()) {
            if flag {
                scratch.a_vals.push(w);
            } else {
                scratch.b_vals.push(w);
            }
        }

        let sa: u128 = scratch.a_vals.iter().copied().try_fold(0_u128, u128::checked_add)?;
        let sb: u128 = scratch.b_vals.iter().copied().try_fold(0_u128, u128::checked_add)?;
        let max_qa = max_pair_q(&scratch.a_vals, n0, m0, m1, sa)?;
        let max_qb = max_pair_q(&scratch.b_vals, n2, m2, m3, sb)?;

        if max_qa.checked_add(max_qb)? >= q_target {
            let min_qa = lcm.checked_mul(sa.checked_mul(sa)?)?.checked_add((m_a as u128) - 1)? / (m_a as u128);
            let min_qb = lcm.checked_mul(sb.checked_mul(sb)?)?.checked_add((m_b as u128) - 1)? / (m_b as u128);
            if min_qa.checked_add(min_qb)? >= q_target {
                hits = hits.checked_add(ways_ab)?;
            } else {
                fill_half_q_hist(&scratch.a_vals, n0, (m0, m1), &mut scratch.half_dp, &mut scratch.qa_hist)?;
                fill_half_q_hist(&scratch.b_vals, n2, (m2, m3), &mut scratch.half_dp, &mut scratch.qb_hist)?;
                hits = hits.checked_add(convolve_hist_ge(&scratch.qa_hist, &scratch.qb_hist, q_target)?)?;
            }
        }

        if !next_combination(&mut comb, pool_len) {
            break;
        }
    }

    hits.checked_mul(sym_factor)
}

/// Worst-case number of `k == 4` meet-in-the-middle split leaves for sorted `ns` (`len >= 4`).
fn estimate_kge4_splits(ns: &[usize]) -> Option<u128> {
    let k = ns.len();
    let n: usize = ns.iter().try_fold(0_usize, |acc, &x| acc.checked_add(x))?;
    if k == 4 {
        let [n0, n1, n2, n3] = [ns[0], ns[1], ns[2], ns[3]];
        let c = binom_u128(n, n0 + n1)?;
        return Some(if n0 == n2 && n1 == n3 { c / 2 } else { c });
    }
    let all_equal = ns.first() == ns.last();
    let branch = if all_equal { binom_u128(n - 1, ns[0] - 1)? } else { binom_u128(n, ns[0])? };
    let sub = estimate_kge4_splits(ns.get(1..)?)?;
    branch.checked_mul(sub)
}

/// Recursive symmetry-canonicalized branch-and-bound for `k >= 5` delegating to `count_k4_mitm`.
fn count_kge4_recursive(ws: &[u128], ns: &[usize], ms: &[u128], lcm: u128, cur_q: u128, q_obs: u128, scratch: &mut MitmScratch) -> Option<u128> {
    let k = ns.len();
    let r = ws.len();
    if k == 4 {
        let q_target = q_obs.saturating_sub(cur_q);
        return count_k4_mitm(ws, [ns[0], ns[1], ns[2], ns[3]], [ms[0], ms[1], ms[2], ms[3]], lcm, q_target, scratch);
    }
    if cur_q >= q_obs {
        return multinomial_u128(ns);
    }
    let s_rem: u128 = ws.iter().copied().try_fold(0_u128, u128::checked_add)?;
    let r128 = r as u128;
    let min_q_rem = lcm.checked_mul(s_rem.checked_mul(s_rem)?)?.checked_add(r128 - 1)? / r128;
    if cur_q.checked_add(min_q_rem)? >= q_obs {
        return multinomial_u128(ns);
    }
    // Exact O(k) integer upper bound: r^2 * Q_rem <= r * L * S_rem^2 + sum_j m_j * M_j^2,
    // where M_j = max(r * top_{n_j} - n_j * S_rem, n_j * S_rem - r * bot_{n_j}).
    let q_need = q_obs - cur_q;
    if let Some(lhs) = r128.checked_mul(r128).and_then(|r2| r2.checked_mul(q_need)) {
        let mut rhs_opt = r128.checked_mul(lcm).and_then(|rl| rl.checked_mul(s_rem)).and_then(|v| v.checked_mul(s_rem));
        for (&nj, &mj) in ns.iter().zip(ms.iter()) {
            let bot: u128 = ws.iter().take(nj).copied().sum();
            let top: u128 = ws.iter().rev().take(nj).copied().sum();
            let nj_s = (nj as u128).checked_mul(s_rem)?;
            let d_hi = r128.checked_mul(top)?.saturating_sub(nj_s);
            let d_lo = nj_s.saturating_sub(r128.checked_mul(bot)?);
            let dev = d_hi.max(d_lo);
            rhs_opt = rhs_opt.and_then(|acc| dev.checked_mul(dev).and_then(|d2| mj.checked_mul(d2)).and_then(|t| acc.checked_add(t)));
        }
        if let Some(rhs) = rhs_opt
            && lhs > rhs
        {
            return Some(0);
        }
    }

    let ng = *ns.first()?;
    let mg = *ms.first()?;
    let all_equal = ns.first() == ns.last();
    let sym_factor = if all_equal { k as u128 } else { 1 };
    let pick_count = if all_equal { ng - 1 } else { ng };
    let pool_len = if all_equal { r - 1 } else { r };
    let mut comb: Vec<usize> = (0..pick_count).collect();
    let mut chosen = vec![false; r];
    let mut next_ws = Vec::with_capacity(r - ng);
    let mut hits = 0_u128;

    loop {
        chosen.fill(false);
        if all_equal {
            if let Some(first) = chosen.first_mut() {
                *first = true;
            }
            for &c in &comb {
                if let Some(slot) = chosen.get_mut(c + 1) {
                    *slot = true;
                }
            }
        } else {
            for &c in &comb {
                if let Some(slot) = chosen.get_mut(c) {
                    *slot = true;
                }
            }
        }

        let mut s_g = 0_u128;
        next_ws.clear();
        for (&w, &flag) in ws.iter().zip(chosen.iter()) {
            if flag {
                s_g = s_g.checked_add(w)?;
            } else {
                next_ws.push(w);
            }
        }
        let next_q = cur_q.checked_add(mg.checked_mul(s_g.checked_mul(s_g)?)?)?;
        let sub_hits = count_kge4_recursive(&next_ws, ns.get(1..)?, ms.get(1..)?, lcm, next_q, q_obs, scratch)?;
        hits = hits.checked_add(sub_hits)?;

        if !next_combination(&mut comb, pool_len) {
            break;
        }
    }

    hits.checked_mul(sym_factor)
}

/// Exact conditional permutation *P* value for `k >= 4` Kruskal–Wallis within `KW_EXACT_KGE4_MAX_SPLITS`.
fn kw_exact_kge4_p(two_ranks: &[usize], group_sizes: &[usize], lcm: u128, q_obs: u128) -> Option<f64> {
    let n = two_ranks.len();
    if n > KW_EXACT_KGE4_MAX_N || group_sizes.len() < 4 {
        return None;
    }
    let mut ns: Vec<usize> = group_sizes.to_vec();
    ns.sort_unstable();
    if *ns.first()? == 0 {
        return None;
    }
    let splits = estimate_kge4_splits(&ns)?;
    if splits > KW_EXACT_KGE4_MAX_SPLITS {
        return None;
    }
    let total_ways = multinomial_u128(&ns)?;
    if total_ways == 0 {
        return None;
    }
    let mut ws: Vec<u128> = two_ranks.iter().map(|&r| r as u128).collect();
    ws.sort_unstable();
    let ms: Vec<u128> = ns.iter().map(|&ni| lcm / (ni as u128)).collect();
    let mut scratch = MitmScratch::new(n);
    let hits = count_kge4_recursive(&ws, &ns, &ms, lcm, 0, q_obs, &mut scratch)?;
    Some(((hits as f64) / (total_ways as f64)).clamp(0.0, 1.0))
}

/// Attempt exact conditional permutation Kruskal–Wallis *P* value within bounded memory and time.
fn kruskal_wallis_exact_p(ranks: &[f64], group_sizes: &[usize]) -> Option<f64> {
    let k = group_sizes.len();
    let n = ranks.len();
    if k < 2 || n < 3 {
        return None;
    }
    let two_ranks: Vec<usize> = ranks.iter().map(|&r| (2.0 * r).round() as usize).collect();
    let min_r = two_ranks.iter().copied().min()?;
    let max_r = two_ranks.iter().copied().max()?;
    if min_r == max_r {
        return Some(1.0);
    }

    if k == 2 {
        return kw_exact_k2_p(&two_ranks, group_sizes[0], group_sizes[1]);
    }

    let mut lcm = 1_u128;
    for &ni in group_sizes {
        lcm = lcm_u128(lcm, ni as u128)?;
    }
    let mut offset = 0_usize;
    let mut q_obs = 0_u128;
    let mut total_two_rank = 0_u128;
    for &ni in group_sizes {
        let s: u128 = two_ranks.get(offset..offset + ni)?.iter().map(|&r| r as u128).sum();
        total_two_rank = total_two_rank.checked_add(s)?;
        let m_i = lcm / (ni as u128);
        q_obs = q_obs.checked_add(m_i.checked_mul(s.checked_mul(s)?)?)?;
        offset += ni;
    }
    // Cauchy–Schwarz minimum: if observed Q equals the lower bound (H == 0), every permutation has Q >= Q_obs.
    if q_obs.checked_mul(n as u128)? <= lcm.checked_mul(total_two_rank.checked_mul(total_two_rank)?)? {
        return Some(1.0);
    }

    if k == 3 { kw_exact_k3_p(&two_ranks, group_sizes, lcm, q_obs) } else { kw_exact_kge4_p(&two_ranks, group_sizes, lcm, q_obs) }
}

/// Kruskal–Wallis one-way ANOVA on ranks with tie correction, exact small-sample conditional
/// permutation *P* values, and rank epsilon-squared effect size.
///
/// `H = 12 / (N (N + 1)) sum R_i^2 / n_i - 3 (N + 1)`, divided by the tie correction
/// `1 - sum (t^3 - t) / (N^3 - N)` (Kruskal & Wallis, 1952; Zar, 2010, ch. 10). Within bounded
/// exact-enumeration limits (`k = 2` up to `N <= 250`; `k = 3` up to `N <= 25`, `<= 2_500_000`
/// cells; `k >= 4` up to `100_000` meet-in-the-middle splits), evaluates the exact conditional
/// permutation *P* value (Iman, Quade & Alexander 1975; Streitberg & Röhmel 1986; Baglivo,
/// Olivier & Pagano 1992; Meyer & Seaman 2013); above that budget falls back to $\chi^2(k - 1)$.
/// When all values are tied (`tie_correction = 0`), reports `H = 0` and `p = 1`.
///
/// Effect size is rank epsilon-squared `epsilon^2_R = H / (N - 1)` (Tomczak & Tomczak, 2014, eq. 5).
pub fn kruskal_wallis_test(groups: &[&[f64]]) -> Result<KruskalWallisResult, StatsError> {
    kruskal_wallis_test_with_method(groups, KruskalWallisPMethod::Auto)
}

/// Kruskal–Wallis one-way ANOVA on ranks with an explicit [`KruskalWallisPMethod`] selecting either
/// automatic exact conditional permutation evaluation with $\chi^2(k - 1)$ fallback
/// ([`KruskalWallisPMethod::Auto`]) or the asymptotic $\chi^2(k - 1)$ approximation
/// ([`KruskalWallisPMethod::Asymptotic`]).
pub fn kruskal_wallis_test_with_method(groups: &[&[f64]], p_method: KruskalWallisPMethod) -> Result<KruskalWallisResult, StatsError> {
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: groups.len() });
    }
    let mut flat = Vec::new();
    let mut group_sizes = Vec::with_capacity(groups.len());
    for &g in groups {
        validate_finite_slice(g)?;
        group_sizes.push(g.len());
        flat.extend_from_slice(g);
    }
    let total_n = flat.len();
    if total_n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Kruskal-Wallis observations", required: 3, actual: total_n });
    }
    let ranked = rank_with_ties(&flat)?;

    let mut offset = 0_usize;
    let mut group_mean_ranks = Vec::with_capacity(groups.len());
    let nf = total_n as f64;
    let mut h_sum = 0.0_f64;
    for &n_i in &group_sizes {
        let r_sum: f64 = ranked.ranks.get(offset..(offset + n_i)).unwrap_or(&[]).iter().sum();
        let mean_rank = r_sum / (n_i as f64);
        group_mean_ranks.push(mean_rank);
        h_sum += (r_sum * r_sum) / (n_i as f64);
        offset += n_i;
    }

    let h_raw = (12.0 / (nf * (nf + 1.0))) * h_sum - 3.0 * (nf + 1.0);
    let tie_denom = 1.0 - ranked.tie_sum_t3_minus_t / (nf * nf * nf - nf);
    let (h_corr, tie_factor) = if tie_denom > 1e-15 { (h_raw.max(0.0) / tie_denom, tie_denom) } else { (0.0, 0.0) };
    let df = (groups.len() - 1) as f64;
    let (p_value, exact_p_used) = match p_method {
        KruskalWallisPMethod::Auto => match kruskal_wallis_exact_p(&ranked.ranks, &group_sizes) {
            Some(p) => (p, true),
            None => (chi2_sf(h_corr, df)?, false),
        },
        KruskalWallisPMethod::Asymptotic => (chi2_sf(h_corr, df)?, false),
    };
    let eps2 = (h_corr / (nf - 1.0)).clamp(0.0, 1.0);

    Ok(KruskalWallisResult {
        h_statistic: h_corr,
        df,
        p_value,
        exact_p_used,
        tie_correction: tie_factor,
        epsilon_squared: eps2,
        group_mean_ranks,
        total_n,
    })
}

/// Friedman nonparametric repeated-measures rank test on complete blocks (`n` rows × `k` conditions).
///
/// Friedman (1937) statistic with within-block tie correction,
/// `Q = 12 sum (R_j - n (k + 1)/2)^2 / (n k (k + 1) - sum (t^3 - t) / (k - 1))`, referred to
/// `chi^2(k - 1)` (Zar, 2010, ch. 12). Kendall's `W = Q / (n (k - 1))` (Kendall & Babington
/// Smith, 1939).
pub fn friedman_test(blocks: &[Vec<f64>]) -> Result<FriedmanResult, StatsError> {
    let n = blocks.len();
    if n < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "repeated-measures subjects", required: 2, actual: n });
    }
    let k = blocks.first().map_or(0, Vec::len);
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: k });
    }
    let mut rank_sums = vec![0.0_f64; k];
    let mut tie_sum = 0.0_f64;

    for row in blocks {
        if row.len() != k {
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: k, actual: row.len() }));
        }
        let ranked = rank_with_ties(row)?;
        tie_sum += ranked.tie_sum_t3_minus_t;
        for (slot, &r) in rank_sums.iter_mut().zip(ranked.ranks.iter()) {
            *slot += r;
        }
    }

    let nf = n as f64;
    let kf = k as f64;
    let mean_rank_expected = 0.5 * (kf + 1.0);
    let ss_cols: f64 = rank_sums.iter().map(|&rs| (rs - nf * mean_rank_expected).powi(2)).sum();
    let denom = nf * kf * (kf + 1.0) / 12.0 - tie_sum / (12.0 * (kf - 1.0));
    let q_stat = if denom > 0.0 { (ss_cols / denom).max(0.0) } else { 0.0 };
    let df = kf - 1.0;
    let p_value = chi2_sf(q_stat, df)?;
    let kendalls_w = (q_stat / (nf * df)).clamp(0.0, 1.0);
    let mean_ranks = rank_sums.iter().map(|&r| r / nf).collect();

    Ok(FriedmanResult {
        q_statistic: q_stat,
        df,
        p_value,
        kendalls_w,
        n_blocks: n,
        k_treatments: k,
        condition_rank_sums: rank_sums,
        condition_mean_ranks: mean_ranks,
    })
}
