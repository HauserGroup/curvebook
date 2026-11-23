//! Motulsky & Brown (2006) ROUT outlier detection core and RSDR robust scale estimator (`D1` verified).
//!
//! Provides the shared pure-numerical ROUT scale and top-30% Benjamini–Hochberg step-up FDR
//! detection kernel used by both column outlier detection (`curvebook-stats`) and robust nonlinear
//! regression diagnostics (`curvebook-fit`).
//!
//! # References
//! - Motulsky, H. J., & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear
//!   regression – a new method based on robust nonlinear regression and the false discovery rate.
//!   *BMC Bioinformatics*, 7, 123. `doi:10.1186/1471-2105-7-123`.
//! - Benjamini, Y., & Hochberg, Y. (1995). Controlling the false discovery rate: A practical and
//!   powerful approach to multiple testing. *Journal of the Royal Statistical Society: Series B*,
//!   57(1), 289–300. `doi:10.1111/j.2517-6161.1995.tb02031.x`.

use crate::dist::student_t_two_sided_p;
use crate::error::MathError;
use crate::quantile::quantile_type7_sorted;

/// Exact 1-sigma central normal probability `erf(1 / sqrt(2))` (`~68.2689492%`) used for the
/// 68.27th percentile of absolute residuals in Motulsky & Brown (2006).
pub const P68_NORMAL_PROB: f64 = 0.682_689_492;

/// Pure numerical output of [`rout_detect_outliers_core`].
#[derive(Debug, Clone, PartialEq)]
pub struct RoutCoreResult {
    /// Robust Standard Deviation of the Residuals (`RSDR = P_68.27 * N / (N - K)`), floored by
    /// `rsdr_floor` when supplied.
    pub rsdr: f64,
    /// Residual degrees of freedom `N - K` (`>= 1`).
    pub df: usize,
    /// Per-observation robust Studentized residual `|r_i| / rsdr` aligned with `residuals`.
    /// When `rsdr` collapses to the zero floor, equals `0.0` for zero residuals and `f64::INFINITY`
    /// for non-zero residuals.
    pub t_stats: Vec<f64>,
    /// Per-observation two-sided Student-*t*(`df`) *p*-value aligned with `residuals`.
    pub p_values: Vec<f64>,
    /// Per-observation Benjamini–Hochberg adjusted *q*-value aligned with `residuals`; equals `1.0`
    /// for the closest ~70% of observations outside the Motulsky & Brown (2006) top-30% test scope.
    pub q_values: Vec<f64>,
    /// Per-observation outlier mask aligned with `residuals`.
    pub is_outlier: Vec<bool>,
    /// Zero-based indices of flagged outliers in ascending index order.
    pub outlier_indices: Vec<usize>,
}

/// Compute the Robust Standard Deviation of the Residuals (`RSDR`) per Motulsky & Brown (2006):
/// `RSDR = P_68.27(|r|) * N / max(N - k_free, 1)`.
///
/// `P_68.27` is evaluated at [`P68_NORMAL_PROB`] (`0.682_689_492`) via Type-7 linear interpolation
/// ([`quantile_type7_sorted`]). Returns `0.0` for an empty slice.
///
/// # Reference
/// - Motulsky, H. J., & Brown, R. E. (2006). *BMC Bioinformatics*, 7, 123.
#[must_use]
pub fn compute_rsdr(abs_residuals: &[f64], k_free: usize) -> f64 {
    let n = abs_residuals.len();
    if n == 0 {
        return 0.0;
    }
    let mut sorted = abs_residuals.to_vec();
    sorted.sort_by(f64::total_cmp);
    let p68 = quantile_type7_sorted(&sorted, P68_NORMAL_PROB).unwrap_or(0.0);
    let df = n.saturating_sub(k_free).max(1);
    p68 * ((n as f64) / (df as f64))
}

fn evaluate_rout_t_and_p(abs_res: &[f64], rsdr: f64, zero_floor: f64, df: usize) -> Result<(Vec<f64>, Vec<f64>), MathError> {
    let n = abs_res.len();
    let df_f = df as f64;
    let mut t_stats = Vec::with_capacity(n);
    let mut p_values = Vec::with_capacity(n);
    for &abs_r in abs_res {
        let (t_stat, p_val) = if rsdr <= zero_floor {
            if abs_r <= zero_floor { (0.0, 1.0) } else { (f64::INFINITY, 0.0) }
        } else {
            let t = abs_r / rsdr;
            let p = student_t_two_sided_p(t, df_f)?;
            (t, p)
        };
        t_stats.push(t_stat);
        p_values.push(p_val);
    }
    Ok((t_stats, p_values))
}

fn extend_zero_rsdr_boundary_ties(
    order: &[usize],
    abs_res: &[f64],
    p_values: &[f64],
    q_values: &mut [f64],
    j_max: usize,
    m_test: usize,
    zero_floor: f64,
) -> usize {
    if j_max + 1 != m_test {
        return j_max;
    }
    let Some(&cut_idx) = order.get(j_max) else {
        return j_max;
    };
    let cut_p = p_values.get(cut_idx).copied().unwrap_or(1.0);
    let cut_r = abs_res.get(cut_idx).copied().unwrap_or(0.0);
    if cut_p != 0.0 || cut_r <= zero_floor {
        return j_max;
    }
    let mut extended = j_max;
    while let Some(&next_idx) = order.get(extended + 1) {
        let next_p = p_values.get(next_idx).copied().unwrap_or(1.0);
        let next_r = abs_res.get(next_idx).copied().unwrap_or(0.0);
        if next_p == 0.0 && next_r == cut_r {
            extended += 1;
            if let Some(slot) = q_values.get_mut(next_idx) {
                *slot = 0.0;
            }
        } else {
            break;
        }
    }
    extended
}

fn apply_bh_top_30(abs_res: &[f64], p_values: &[f64], fdr_q_percent: f64, rsdr: f64, zero_floor: f64) -> (Vec<f64>, Vec<bool>, Vec<usize>) {
    let n = p_values.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        let pa = p_values.get(a).copied().unwrap_or(1.0);
        let pb = p_values.get(b).copied().unwrap_or(1.0);
        let ra = abs_res.get(a).copied().unwrap_or(0.0);
        let rb = abs_res.get(b).copied().unwrap_or(0.0);
        pa.total_cmp(&pb).then_with(|| rb.total_cmp(&ra)).then_with(|| a.cmp(&b))
    });

    // Motulsky & Brown (2006): test only the top ~30% furthest points (`m_test = N - floor(0.70 * N)`).
    let start = ((0.70 * (n as f64)).floor() as usize).min(n.saturating_sub(1));
    let m_test = n - start;
    let q_alpha = fdr_q_percent / 100.0;

    let mut q_values = vec![1.0_f64; n];
    let mut running_min = 1.0_f64;
    for j in (0..m_test).rev() {
        if let Some(&idx) = order.get(j) {
            let rank = (j + 1) as f64;
            let p_j = p_values.get(idx).copied().unwrap_or(1.0);
            let raw_q = ((n as f64) / rank * p_j).min(1.0);
            running_min = running_min.min(raw_q);
            if let Some(slot) = q_values.get_mut(idx) {
                *slot = running_min;
            }
        }
    }

    let mut last_rejected: Option<usize> = None;
    for j in 0..m_test {
        if let Some(&idx) = order.get(j) {
            let rank = (j + 1) as f64;
            let threshold = q_alpha * rank / (n as f64);
            let p_j = p_values.get(idx).copied().unwrap_or(1.0);
            if p_j <= threshold {
                last_rejected = Some(j);
            }
        }
    }

    let mut is_outlier = vec![false; n];
    if let Some(mut j_max) = last_rejected {
        if rsdr <= zero_floor {
            j_max = extend_zero_rsdr_boundary_ties(&order, abs_res, p_values, &mut q_values, j_max, m_test, zero_floor);
        }
        for &idx in order.iter().take(j_max + 1) {
            if let Some(slot) = is_outlier.get_mut(idx) {
                *slot = true;
            }
        }
    }

    let outlier_indices = is_outlier.iter().enumerate().filter_map(|(idx, &flag)| flag.then_some(idx)).collect();
    (q_values, is_outlier, outlier_indices)
}

/// Core Motulsky & Brown (2006) ROUT outlier detection on robust residuals:
/// 1. Computes `RSDR = compute_rsdr(|r|, k_free).max(rsdr_floor)` with `df = N - k_free`.
/// 2. Evaluates two-sided Student-*t*(`df`) *p*-values from `t_i = |r_i| / RSDR` (handling exact
///    zero-scale bulk ties cleanly: `|r_i| == 0 -> t = 0, P = 1`; `|r_i| > 0 -> t = +Inf, P = 0`).
/// 3. Applies the Benjamini & Hochberg (1995) step-up FDR procedure at `Q = fdr_q_percent / 100`
///    strictly over the top ~30% furthest observations (`m_test = N - floor(0.70 * N)`), leaving
///    the closest ~70% untested (`q_value = 1.0`, `is_outlier = false`).
///
/// Pass `rsdr_floor = 0.0` to preserve exact `RSDR == 0.0` on heavy-tie / constant column data, or
/// a positive scale floor (e.g. `1e-10 * max_abs_weighted_y`) for nonlinear regression fits.
///
/// # Errors
/// Returns:
/// - [`MathError::EmptyInput`] if `residuals` is empty,
/// - [`MathError::DomainError`] if any residual is non-finite,
/// - [`MathError::InvalidParameter`] if `residuals.len() <= k_free`, `fdr_q_percent` is outside
///   `(0.0, 100.0]`, or `rsdr_floor` is negative or non-finite.
///
/// # References
/// - Motulsky, H. J., & Brown, R. E. (2006). *BMC Bioinformatics*, 7, 123.
/// - Benjamini, Y., & Hochberg, Y. (1995). *JRSS-B*, 57(1), 289–300.
pub fn rout_detect_outliers_core(residuals: &[f64], k_free: usize, fdr_q_percent: f64, rsdr_floor: f64) -> Result<RoutCoreResult, MathError> {
    if residuals.is_empty() {
        return Err(MathError::EmptyInput);
    }
    let n = residuals.len();
    for &r in residuals {
        if !r.is_finite() {
            return Err(MathError::DomainError { name: "residuals", value: r });
        }
    }
    if n <= k_free {
        return Err(MathError::InvalidParameter { name: "n", value: n as f64 });
    }
    if !fdr_q_percent.is_finite() || fdr_q_percent <= 0.0 || fdr_q_percent > 100.0 {
        return Err(MathError::InvalidParameter { name: "fdr_q_percent", value: fdr_q_percent });
    }
    if !rsdr_floor.is_finite() || rsdr_floor < 0.0 {
        return Err(MathError::InvalidParameter { name: "rsdr_floor", value: rsdr_floor });
    }

    let df = n - k_free;
    let abs_res: Vec<f64> = residuals.iter().map(|r| r.abs()).collect();
    let rsdr = compute_rsdr(&abs_res, k_free).max(rsdr_floor);
    if !rsdr.is_finite() {
        return Err(MathError::DomainError { name: "rsdr", value: rsdr });
    }
    let zero_floor = if rsdr_floor > 0.0 { (1e-20 * rsdr_floor).max(f64::MIN_POSITIVE) } else { 0.0 };

    let (t_stats, p_values) = evaluate_rout_t_and_p(&abs_res, rsdr, zero_floor, df)?;
    let (q_values, is_outlier, outlier_indices) = apply_bh_top_30(&abs_res, &p_values, fdr_q_percent, rsdr, zero_floor);

    Ok(RoutCoreResult { rsdr, df, t_stats, p_values, q_values, is_outlier, outlier_indices })
}

#[cfg(test)]
mod tests {
    use super::{P68_NORMAL_PROB, compute_rsdr, rout_detect_outliers_core};
    use crate::dist::student_t_two_sided_p;
    use crate::error::MathError;

    fn assert_close(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, label: &str) {
        assert!(actual.is_finite(), "{label}: expected finite near {expected}, got {actual}");
        let diff = (actual - expected).abs();
        let bound = abs_tol.max(rel_tol * expected.abs());
        assert!(diff <= bound, "{label}: actual={actual:.16e}, expected={expected:.16e}, diff={diff:.3e} > bound={bound:.3e}");
    }

    #[test]
    fn compute_rsdr_matches_motulsky_brown_definition() {
        assert_eq!(compute_rsdr(&[], 1), 0.0);
        // Constant 0.5 residuals across n=10, k=2 => P68 = 0.5, RSDR = 0.5 * 10 / 8 = 0.625.
        assert_close(compute_rsdr(&[0.5; 10], 2), 0.625, 1e-15, 1e-15, "constant rsdr");
        // Linear ramp 0..10 (n=11, k=1 => df=10): pos = P68_NORMAL_PROB * 10 = 6.82689492.
        let ramp: Vec<f64> = (0..=10).map(|i| i as f64).collect();
        let expected = (P68_NORMAL_PROB * 10.0) * 11.0 / 10.0;
        assert_close(compute_rsdr(&ramp, 1), expected, 1e-14, 1e-14, "ramp rsdr");
        // Degenerate k_free >= n clamps df to 1 without dividing by zero.
        assert_close(compute_rsdr(&[2.0, 2.0], 5), 4.0, 1e-15, 1e-15, "df clamped to 1");
    }

    #[test]
    fn rout_top_30_percent_and_bh_q_values_match_motulsky_brown_2006() {
        // N = 100, k_free = 2 (df = 98): 69 core points (|r|=0.01), 1 borderline 31st-furthest
        // point (|r|=1.0, outside top 30%), and 30 extreme outliers (|r|=10.0).
        let mut res = Vec::with_capacity(100);
        for i in 0..100_usize {
            let r = if i < 69 {
                if i % 2 == 0 { 0.01 } else { -0.01 }
            } else if i == 69 {
                1.0
            } else {
                10.0
            };
            res.push(r);
        }
        let out = rout_detect_outliers_core(&res, 2, 5.0, 1e-10 * 100.0).unwrap();
        assert_eq!(out.df, 98);
        assert_eq!(out.outlier_indices.len(), 30);
        assert!(!out.is_outlier[69]);
        assert_close(out.q_values[69], 1.0, 1e-15, 0.0, "31st furthest untested");
        for i in 70..100 {
            assert!(out.is_outlier[i]);
        }

        // Exact BH step-up q-value verification on N = 20, k_free = 2 (m_test = 6).
        let mut res20 = vec![0.1_f64; 20];
        for (i, slot) in res20.iter_mut().enumerate() {
            if i % 2 == 1 {
                *slot = -0.1;
            }
        }
        res20[2] = 0.14;
        res20[5] = -0.17;
        res20[9] = 0.20;
        res20[11] = -0.25;
        res20[14] = 4.0;
        res20[18] = -5.5;
        let out20 = rout_detect_outliers_core(&res20, 2, 1.0, 1e-10 * 50.0).unwrap();
        assert_eq!(out20.outlier_indices, vec![14, 18]);

        let expected_p18 = student_t_two_sided_p(5.5 / out20.rsdr, 18.0).unwrap();
        let expected_p14 = student_t_two_sided_p(4.0 / out20.rsdr, 18.0).unwrap();
        let expected_p11 = student_t_two_sided_p(0.25 / out20.rsdr, 18.0).unwrap();
        assert_close(out20.p_values[18], expected_p18, 1e-15, 1e-12, "p rank 1");
        assert_close(out20.p_values[14], expected_p14, 1e-15, 1e-12, "p rank 2");
        assert_close(out20.p_values[11], expected_p11, 1e-15, 1e-12, "p rank 3");
        assert_close(out20.q_values[14], (expected_p14 * 20.0 / 2.0).min(1.0), 1e-15, 1e-12, "q rank 2");
        assert_close(out20.q_values[11], (expected_p11 * 20.0 / 3.0).min(1.0), 1e-15, 1e-12, "q rank 3");
    }

    #[test]
    fn rout_handles_zero_rsdr_heavy_ties_and_constant_series() {
        let mut heavy_ties = vec![0.0; 8];
        heavy_ties.push(95.0);
        let out = rout_detect_outliers_core(&heavy_ties, 1, 1.0, 0.0).unwrap();
        assert_eq!(out.rsdr, 0.0);
        assert_eq!(out.outlier_indices, vec![8]);
        assert!(out.t_stats[8].is_infinite());
        assert_eq!(out.p_values[8], 0.0);
        assert_eq!(out.t_stats[0], 0.0);
        assert_eq!(out.p_values[0], 1.0);

        let constant = rout_detect_outliers_core(&[0.0; 10], 1, 1.0, 0.0).unwrap();
        assert_eq!(constant.rsdr, 0.0);
        assert!(constant.outlier_indices.is_empty());
    }

    #[test]
    fn rout_rejects_invalid_inputs() {
        assert_eq!(rout_detect_outliers_core(&[], 1, 1.0, 0.0), Err(MathError::EmptyInput));
        assert!(matches!(rout_detect_outliers_core(&[1.0, 2.0], 2, 1.0, 0.0), Err(MathError::InvalidParameter { name: "n", .. })));
        assert!(matches!(rout_detect_outliers_core(&[1.0, 2.0, 3.0], 1, 0.0, 0.0), Err(MathError::InvalidParameter { name: "fdr_q_percent", .. })));
        assert!(matches!(rout_detect_outliers_core(&[1.0, 2.0, 3.0], 1, 101.0, 0.0), Err(MathError::InvalidParameter { name: "fdr_q_percent", .. })));
        assert!(matches!(rout_detect_outliers_core(&[1.0, 2.0, 3.0], 1, 1.0, -1.0), Err(MathError::InvalidParameter { name: "rsdr_floor", .. })));
        assert!(matches!(rout_detect_outliers_core(&[1.0, f64::NAN, 3.0], 1, 1.0, 0.0), Err(MathError::DomainError { name: "residuals", .. })));
    }

    #[test]
    fn compute_rsdr_and_rout_guard_large_scale_overflow() {
        // DA-01-04 regression: p68 * (n / df) stays finite even when p68 * n overflows f64::MAX.
        let mut huge = vec![2e306_f64; 100];
        huge[99] = 1e308;
        let rsdr = compute_rsdr(&huge, 1);
        let expected_rsdr = 2e306_f64 * (100.0 / 99.0);
        assert_close(rsdr, expected_rsdr, 0.0, 1e-14, "large-scale finite rsdr");

        let out = rout_detect_outliers_core(&huge, 1, 1.0, 0.0).unwrap();
        assert!(out.rsdr.is_finite());
        assert_eq!(out.outlier_indices, vec![99]);

        // When p68 * (n / df) genuinely exceeds f64::MAX, rout_detect_outliers_core returns DomainError
        // instead of silently flagging 0 outliers via abs_r / +Inf = 0.0.
        let overflow_res = vec![1e308_f64; 10];
        assert!(matches!(rout_detect_outliers_core(&overflow_res, 8, 1.0, 0.0), Err(MathError::DomainError { name: "rsdr", .. })));
    }

    #[test]
    fn rout_zero_rsdr_boundary_ties_are_permutation_invariant() {
        // DA-01-05 regression: n = 100 with 69 zeros (rsdr == 0.0) and 31 tied 10.0 residuals.
        // All 31 tied points must be flagged regardless of input permutation.
        let mut forward = vec![0.0_f64; 69];
        forward.extend(std::iter::repeat_n(10.0_f64, 31));
        let out_fwd = rout_detect_outliers_core(&forward, 1, 1.0, 0.0).unwrap();
        assert_eq!(out_fwd.rsdr, 0.0);
        assert_eq!(out_fwd.outlier_indices.len(), 31);
        for i in 69..100 {
            assert!(out_fwd.is_outlier[i]);
            assert_eq!(out_fwd.q_values[i], 0.0);
        }

        let mut reversed = vec![10.0_f64; 31];
        reversed.extend(std::iter::repeat_n(0.0_f64, 69));
        let out_rev = rout_detect_outliers_core(&reversed, 1, 1.0, 0.0).unwrap();
        assert_eq!(out_rev.outlier_indices.len(), 31);
        for i in 0..31 {
            assert!(out_rev.is_outlier[i]);
            assert_eq!(out_rev.q_values[i], 0.0);
        }

        // Distinct non-zero residual just outside top 30% is NOT pulled in by tie expansion.
        let mut distinct = vec![0.0_f64; 69];
        distinct.push(5.0);
        distinct.extend(std::iter::repeat_n(10.0_f64, 30));
        let out_dist = rout_detect_outliers_core(&distinct, 1, 1.0, 0.0).unwrap();
        assert_eq!(out_dist.outlier_indices.len(), 30);
        assert!(!out_dist.is_outlier[69]);
    }
}
