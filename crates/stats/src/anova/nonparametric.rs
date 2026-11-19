//! Rank-based one-way tests: Kruskal–Wallis for independent groups and Friedman for matched blocks.
//!
//! References:
//! - Kruskal, W. H., & Wallis, W. A. (1952). Use of ranks in one-criterion variance analysis.
//!   *Journal of the American Statistical Association* 47(260):583–621.
//!   doi:10.1080/01621459.1952.10483441
//! - Friedman, M. (1937). The use of ranks to avoid the assumption of normality implicit in the
//!   analysis of variance. *Journal of the American Statistical Association* 32(200):675–701.
//!   doi:10.1080/01621459.1937.10503522
//! - Kendall, M. G., & Babington Smith, B. (1939). The problem of *m* rankings. *Annals of
//!   Mathematical Statistics* 10(3):275–287.
//! - Tomczak, M., & Tomczak, E. (2014). The need to report effect size estimates revisited. An
//!   overview of some recommended measures of effect size. *Trends in Sport Sciences* 1(21):19–25.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.

use curvebook_math::dist::chi2_sf;

use crate::anova::results::{FriedmanResult, KruskalWallisResult};
use crate::common::{rank_with_ties, validate_finite_slice};
use crate::error::StatsError;

/// Kruskal–Wallis one-way ANOVA on ranks with tie correction and rank epsilon-squared effect size.
///
/// `H = 12 / (N (N + 1)) sum R_i^2 / n_i - 3 (N + 1)`, divided by the tie correction
/// `1 - sum (t^3 - t) / (N^3 - N)` and referred to `chi^2(k - 1)` (Kruskal & Wallis, 1952;
/// Zar, 2010, ch. 10). When all values are tied (`tie_correction = 0`), reports `H = 0` and `p = 1`.
///
/// Effect size is rank epsilon-squared `epsilon^2_R = H / (N - 1)` (Tomczak & Tomczak, 2014, eq. 5).
pub fn kruskal_wallis_test(groups: &[&[f64]]) -> Result<KruskalWallisResult, StatsError> {
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
        let r_sum: f64 = ranked.ranks[offset..(offset + n_i)].iter().sum();
        let mean_rank = r_sum / (n_i as f64);
        group_mean_ranks.push(mean_rank);
        h_sum += (r_sum * r_sum) / (n_i as f64);
        offset += n_i;
    }

    let h_raw = (12.0 / (nf * (nf + 1.0))) * h_sum - 3.0 * (nf + 1.0);
    let tie_denom = 1.0 - ranked.tie_sum_t3_minus_t / (nf * nf * nf - nf);
    let (h_corr, tie_factor) = if tie_denom > 1e-15 { (h_raw.max(0.0) / tie_denom, tie_denom) } else { (0.0, 0.0) };
    let df = (groups.len() - 1) as f64;
    let p_value = chi2_sf(h_corr, df)?;
    let eps2 = (h_corr / (nf - 1.0)).clamp(0.0, 1.0);

    Ok(KruskalWallisResult { h_statistic: h_corr, df, p_value, tie_correction: tie_factor, epsilon_squared: eps2, group_mean_ranks, total_n })
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
    let k = blocks[0].len();
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
        for (c, &r) in ranked.ranks.iter().enumerate() {
            rank_sums[c] += r;
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
