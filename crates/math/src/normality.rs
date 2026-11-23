//! Univariate normality test kernels and tie-averaged rank helpers (`D1` verified).
//!
//! Provides pure numerical kernels shared across column statistics (`curvebook-stats`) and
//! regression residual diagnostics (`curvebook-fit`):
//! - [`shapiro_wilk`]: Shapiro–Wilk `W` test (`3 <= n <= 5000`) via Royston (1992, 1995 AS R94),
//! - [`dagostino_pearson`]: D'Agostino–Pearson omnibus `K^2` test (`n >= 8`) combining
//!   D'Agostino (1970) skewness and Anscombe & Glynn (1983) kurtosis normalizations,
//! - [`anderson_darling`]: Anderson–Darling `A^{*2}` test (`n >= 8`) with Stephens (1974) /
//!   D'Agostino & Stephens (1986) Case 3 finite-sample correction and monotone tail p-value,
//! - [`average_ranks`]: 1-based mid-ranks with exact tie averaging (Zar 2010).
//!
//! # References
//! - Shapiro, S. S., & Wilk, M. B. (1965). An analysis of variance test for normality (complete
//!   samples). *Biometrika*, 52(3/4), 591–611.
//! - Royston, P. (1992). Approximating the Shapiro–Wilk W-test for non-normality. *Statistics
//!   and Computing*, 2(3), 117–119.
//! - Royston, P. (1995). Remark AS R94: A remark on algorithm AS 181: The W-test for normality.
//!   *Applied Statistics*, 44(4), 547–551.
//! - D'Agostino, R. B. (1970). Transformation to normality of the null distribution of `g_1`.
//!   *Biometrika*, 57(3), 679–681.
//! - D'Agostino, R. B., & Pearson, E. S. (1973). Tests for departure from normality. Empirical
//!   results for the distributions of `b_2` and `sqrt(b_1)`. *Biometrika*, 60(3), 613–622.
//! - Anscombe, F. J., & Glynn, W. J. (1983). Distribution of the kurtosis statistic `b_2` for
//!   normal samples. *Biometrika*, 70(1), 227–234.
//! - D'Agostino, R. B., Belanger, A., & D'Agostino, R. B., Jr. (1990). A suggestion for using
//!   powerful and informative tests of normality. *The American Statistician*, 44(4), 316–321.
//! - Anderson, T. W., & Darling, D. A. (1954). A test of goodness of fit. *Journal of the
//!   American Statistical Association*, 49(268), 765–769.
//! - Stephens, M. A. (1974). EDF statistics for goodness of fit and some comparisons. *Journal
//!   of the American Statistical Association*, 69(347), 730–737.
//! - D'Agostino, R. B., & Stephens, M. A. (Eds.). (1986). *Goodness-of-Fit Techniques* (Table 4.9,
//!   Case 3). Marcel Dekker.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson.

use crate::dist::{chi2_sf, norm_cdf, norm_ppf, norm_sf};
use crate::error::MathError;
use crate::sum::{NeumaierSum, SummaryMoments};

/// Pure numerical output of a univariate normality test kernel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalityResult {
    /// Observed test statistic (`W`, `K^2`, or `A^{*2}`).
    pub statistic: f64,
    /// Upper-tail *p*-value clamped to `[0.0, 1.0]`.
    pub p_value: f64,
}

fn validate_finite_slice(values: &[f64]) -> Result<(), MathError> {
    if values.is_empty() {
        return Err(MathError::EmptyInput);
    }
    for &v in values {
        if !v.is_finite() {
            return Err(MathError::DomainError { name: "x", value: v });
        }
    }
    Ok(())
}

fn poly_eval(coeffs: &[f64], x: f64) -> f64 {
    let mut acc = 0.0_f64;
    for &c in coeffs.iter().rev() {
        acc = acc * x + c;
    }
    acc
}

fn shapiro_wilk_weights(n: usize) -> Result<Vec<f64>, MathError> {
    if n == 3 {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        return Ok(vec![-s, 0.0, s]);
    }
    let nf = n as f64;
    let mut m = Vec::with_capacity(n);
    let mut m_sq_sum = 0.0_f64;
    for i in 1..=n {
        let p = ((i as f64) - 0.375) / (nf + 0.25);
        let mi = norm_ppf(p)?;
        m_sq_sum += mi * mi;
        m.push(mi);
    }
    let m_norm = m_sq_sum.sqrt();
    let u = 1.0 / nf.sqrt();
    let m_last = m.get(n - 1).copied().ok_or(MathError::InvalidParameter { name: "n", value: nf })?;

    const C1: [f64; 6] = [0.0, 0.221_157, -0.147_981, -2.071_19, 4.434_685, -2.706_056];
    let a_n = m_last / m_norm + poly_eval(&C1, u);

    let mut a = vec![0.0_f64; n];
    if let Some(first) = a.first_mut() {
        *first = -a_n;
    }
    if let Some(last) = a.last_mut() {
        *last = a_n;
    }

    if n <= 5 {
        let num = m_sq_sum - 2.0 * m_last * m_last;
        let den = (1.0 - 2.0 * a_n * a_n).max(1e-12);
        let scale = (den / num).sqrt();
        for (slot, &mi) in a.iter_mut().zip(m.iter()).skip(1).take(n - 2) {
            *slot = mi * scale;
        }
    } else {
        const C2: [f64; 6] = [0.0, 0.042_981, -0.293_762, -1.752_461, 5.682_633, -3.582_633];
        let m_prev = m.get(n - 2).copied().ok_or(MathError::InvalidParameter { name: "n", value: nf })?;
        let a_nm1 = m_prev / m_norm + poly_eval(&C2, u);
        if let Some(slot) = a.get_mut(1) {
            *slot = -a_nm1;
        }
        if let Some(slot) = a.get_mut(n - 2) {
            *slot = a_nm1;
        }
        let num = m_sq_sum - 2.0 * m_last * m_last - 2.0 * m_prev * m_prev;
        let den = (1.0 - 2.0 * a_n * a_n - 2.0 * a_nm1 * a_nm1).max(1e-12);
        let scale = (den / num).sqrt();
        for (slot, &mi) in a.iter_mut().zip(m.iter()).skip(2).take(n - 4) {
            *slot = mi * scale;
        }
    }
    Ok(a)
}

fn shapiro_wilk_p_value(w: f64, n: usize) -> f64 {
    if w >= 1.0 {
        return 1.0;
    }
    if n == 3 {
        // Shapiro & Wilk (1965) / Royston (1995 AS R94): exact null CDF on [3/4, 1].
        let p = (6.0 / std::f64::consts::PI) * (w.max(0.0).sqrt().clamp(0.0, 1.0).asin() - std::f64::consts::FRAC_PI_3);
        return p.clamp(0.0, 1.0);
    }
    let one_minus_w = (1.0 - w).max(1e-16);
    if n <= 11 {
        let nf = n as f64;
        let gamma_val = 0.459 * nf - 2.273;
        let lw = one_minus_w.ln();
        if lw >= gamma_val {
            return 0.0;
        }
        let y = -(gamma_val - lw).ln();
        let mu = -0.000_671_4 * nf.powi(3) + 0.025_054 * nf.powi(2) - 0.399_78 * nf + 0.5440;
        let sigma = (-0.002_032_2 * nf.powi(3) + 0.062_767 * nf.powi(2) - 0.778_57 * nf + 1.3822).exp();
        norm_sf((y - mu) / sigma).clamp(0.0, 1.0)
    } else {
        let x = (n as f64).ln();
        let y = one_minus_w.ln();
        let mu = 0.003_891_5 * x.powi(3) - 0.083_751 * x.powi(2) - 0.310_82 * x - 1.5861;
        let sigma = (0.003_030_2 * x.powi(2) - 0.082_676 * x - 0.4803).exp();
        norm_sf((y - mu) / sigma).clamp(0.0, 1.0)
    }
}

/// Shapiro–Wilk `W` normality test (`3 <= n <= 5000`) using Shapiro & Wilk (1965) and
/// Royston (1992; 1995 AS R94).
///
/// Uses exact `a = [-1/sqrt(2), 0, 1/sqrt(2)]` and exact arcsine null distribution at `n = 3`,
/// Royston AS R94 polynomial corrections for `a_n` (`n >= 4`) and `a_{n-1}` (`n >= 6`), and
/// Royston's normalizing transformations for `4 <= n <= 5000`.
///
/// # Errors
/// Returns:
/// - [`MathError::EmptyInput`] if `values` is empty,
/// - [`MathError::DomainError`] if any element is non-finite or sample variance is zero,
/// - [`MathError::InvalidParameter`] if `values.len() < 3` or `values.len() > 5000`.
///
/// # References
/// - Shapiro, S. S., & Wilk, M. B. (1965). *Biometrika*, 52(3/4), 591–611.
/// - Royston, P. (1992). *Statistics and Computing*, 2(3), 117–119.
/// - Royston, P. (1995). Remark AS R94. *Applied Statistics*, 44(4), 547–551.
pub fn shapiro_wilk(values: &[f64]) -> Result<NormalityResult, MathError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if !(3..=5000).contains(&n) {
        return Err(MathError::InvalidParameter { name: "n", value: n as f64 });
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let moments = SummaryMoments::from_slice(&sorted)?;
    let sd = moments.population_sd();
    let (scale, css_s) = moments.scaled_m2();
    if !sd.is_finite() || sd <= 0.0 || !scale.is_finite() || scale <= 0.0 || !css_s.is_finite() || css_s <= 0.0 {
        return Err(MathError::DomainError { name: "variance", value: sd });
    }

    let inv_scale = 1.0 / scale;
    let weights = shapiro_wilk_weights(n)?;
    let mut num_acc = NeumaierSum::new();
    for ((&x_lo, &x_hi), &w_hi) in sorted.iter().zip(sorted.iter().rev()).zip(weights.iter().rev()).take(n / 2) {
        let diff = x_hi - x_lo;
        let scaled_diff = if diff.is_finite() { diff * inv_scale } else { x_hi * inv_scale - x_lo * inv_scale };
        num_acc.add(w_hi * scaled_diff);
    }
    let num = num_acc.total();
    let raw_w = (num * num) / css_s;
    if !raw_w.is_finite() {
        return Err(MathError::DomainError { name: "w", value: raw_w });
    }
    let w = raw_w.clamp(0.0, 1.0);
    let p_value = shapiro_wilk_p_value(w, n);
    Ok(NormalityResult { statistic: w, p_value })
}

/// D'Agostino–Pearson omnibus `K^2` normality test (`n >= 8`).
///
/// Computes `K^2 = Z(sqrt(b_1))^2 + Z(b_2)^2` referred to chi-square on 2 degrees of freedom
/// (D'Agostino & Pearson 1973; D'Agostino, Belanger & D'Agostino 1990), combining the skewness
/// transformation of D'Agostino (1970) with the kurtosis transformation of Anscombe & Glynn (1983).
///
/// Edge cases: clamps the Anscombe–Glynn Wilson–Hilferty denominator at `1e-12` so extreme
/// platykurtic large-`n` samples preserve the negative sign of `Z(b_2)` and stay finite.
///
/// # Errors
/// Returns:
/// - [`MathError::EmptyInput`] if `values` is empty,
/// - [`MathError::DomainError`] if any element is non-finite or sample variance is zero,
/// - [`MathError::InvalidParameter`] if `values.len() < 8`.
///
/// # References
/// - D'Agostino, R. B. (1970). *Biometrika*, 57(3), 679–681.
/// - D'Agostino, R. B., & Pearson, E. S. (1973). *Biometrika*, 60(3), 613–622.
/// - Anscombe, F. J., & Glynn, W. J. (1983). *Biometrika*, 70(1), 227–234.
/// - D'Agostino, R. B., Belanger, A., & D'Agostino, R. B., Jr. (1990). *The American Statistician*,
///   44(4), 316–321.
pub fn dagostino_pearson(values: &[f64]) -> Result<NormalityResult, MathError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n < 8 {
        return Err(MathError::InvalidParameter { name: "n", value: n as f64 });
    }
    let moments = SummaryMoments::from_slice(values)?;
    let sd = moments.population_sd();
    if !sd.is_finite() || sd <= 0.0 {
        return Err(MathError::DomainError { name: "variance", value: sd });
    }

    let nf = n as f64;
    let mut s3 = NeumaierSum::new();
    let mut s4 = NeumaierSum::new();
    for &x in values {
        let diff = x - moments.mean;
        let z = if diff.is_finite() { diff / sd } else { x / sd - moments.mean / sd };
        let z2 = z * z;
        s3.add(z2 * z);
        s4.add(z2 * z2);
    }
    let sqrt_b1 = s3.total() / nf;
    let b2 = s4.total() / nf;
    if !sqrt_b1.is_finite() || !b2.is_finite() {
        return Err(MathError::DomainError { name: "moments", value: if !sqrt_b1.is_finite() { sqrt_b1 } else { b2 } });
    }

    // Skewness normal transform (D'Agostino 1970).
    let y = sqrt_b1 * (((nf + 1.0) * (nf + 3.0)) / (6.0 * (nf - 2.0))).sqrt();
    let beta2 = (3.0 * (nf * nf + 27.0 * nf - 70.0) * (nf + 1.0) * (nf + 3.0)) / ((nf - 2.0) * (nf + 5.0) * (nf + 7.0) * (nf + 9.0));
    let w2 = -1.0 + (2.0 * (beta2 - 1.0)).sqrt();
    let delta = 1.0 / (0.5 * w2.ln()).sqrt();
    let alpha_skew = (2.0 / (w2 - 1.0)).sqrt();
    let z1 = delta * (y / alpha_skew).asinh();

    // Kurtosis normal transform (Anscombe & Glynn 1983).
    let e_b2 = 3.0 * (nf - 1.0) / (nf + 1.0);
    let var_b2 = (24.0 * nf * (nf - 2.0) * (nf - 3.0)) / ((nf + 1.0).powi(2) * (nf + 3.0) * (nf + 5.0));
    let x_k = (b2 - e_b2) / var_b2.sqrt();
    let sqrt_beta1 =
        (6.0 * (nf * nf - 5.0 * nf + 2.0) / ((nf + 7.0) * (nf + 9.0))) * ((6.0 * (nf + 3.0) * (nf + 5.0)) / (nf * (nf - 2.0) * (nf - 3.0))).sqrt();
    let a = 6.0 + (8.0 / sqrt_beta1) * (2.0 / sqrt_beta1 + (1.0 + 4.0 / (sqrt_beta1 * sqrt_beta1)).sqrt());
    let denom_raw = 1.0 + x_k * (2.0 / (a - 4.0)).sqrt();
    if !denom_raw.is_finite() {
        return Err(MathError::DomainError { name: "kurtosis", value: denom_raw });
    }
    let denom = denom_raw.max(1e-12);
    let term = (1.0 - 2.0 / a) / denom;
    let z2 = ((1.0 - 2.0 / (9.0 * a)) - term.cbrt()) / (2.0 / (9.0 * a)).sqrt();

    let raw_k2 = z1 * z1 + z2 * z2;
    if !raw_k2.is_finite() {
        return Err(MathError::DomainError { name: "k2", value: raw_k2 });
    }
    let k2 = raw_k2.max(0.0);
    let p_value = chi2_sf(k2, 2.0)?.clamp(0.0, 1.0);
    Ok(NormalityResult { statistic: k2, p_value })
}

fn anderson_darling_p_value(a2_star: f64) -> f64 {
    // Stephens (1974) / D'Agostino & Stephens (1986) Table 4.9 Case 3 piecewise exponential
    // approximation; clamp at the upper quadratic's vertex `5.709 / (2 * 0.0186) ~ 153.4677`
    // (`P ~ 2.02e-190`) so P stays strictly non-increasing for arbitrarily large A*^2.
    const AD_QUADRATIC_VERTEX: f64 = 5.709 / (2.0 * 0.0186);
    if a2_star >= 0.6 {
        let a_eff = a2_star.min(AD_QUADRATIC_VERTEX);
        (1.2937 - 5.709 * a_eff + 0.0186 * a_eff * a_eff).exp()
    } else if a2_star > 0.34 {
        (0.9177 - 4.279 * a2_star - 1.38 * a2_star * a2_star).exp()
    } else if a2_star > 0.20 {
        1.0 - (-8.318 + 42.796 * a2_star - 59.938 * a2_star * a2_star).exp()
    } else {
        1.0 - (-13.436 + 101.14 * a2_star - 223.73 * a2_star * a2_star).exp()
    }
    .clamp(0.0, 1.0)
}

/// Anderson–Darling normality test (`n >= 8`) with Stephens small-sample adjustment `A^{*2}`.
///
/// Computes `A^2` (Anderson & Darling 1954) with estimated mean and sample standard deviation
/// (`n - 1` denominator), applies the Stephens (1974) / D'Agostino & Stephens (1986) Case 3
/// modification `A^{*2} = A^2 (1 + 0.75/n + 2.25/n^2)`, and evaluates the piecewise exponential
/// *p*-value approximation with quadratic-vertex clamping to preserve monotonicity for extreme
/// `A^{*2}`.
///
/// # Errors
/// Returns:
/// - [`MathError::EmptyInput`] if `values` is empty,
/// - [`MathError::DomainError`] if any element is non-finite or sample standard deviation is zero,
/// - [`MathError::InvalidParameter`] if `values.len() < 8`.
///
/// # References
/// - Anderson, T. W., & Darling, D. A. (1954). *JASA*, 49(268), 765–769.
/// - Stephens, M. A. (1974). *JASA*, 69(347), 730–737.
/// - D'Agostino, R. B., & Stephens, M. A. (Eds.). (1986). *Goodness-of-Fit Techniques* (Table 4.9,
///   Case 3). Marcel Dekker.
pub fn anderson_darling(values: &[f64]) -> Result<NormalityResult, MathError> {
    validate_finite_slice(values)?;
    let n = values.len();
    if n < 8 {
        return Err(MathError::InvalidParameter { name: "n", value: n as f64 });
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let moments = SummaryMoments::from_slice(&sorted)?;
    let sd = moments.sample_sd()?;
    if !sd.is_finite() || sd <= 0.0 {
        return Err(MathError::DomainError { name: "sd", value: sd });
    }

    let nf = n as f64;
    let mut sum_acc = NeumaierSum::new();
    for (i, (&x_lo, &x_hi)) in sorted.iter().zip(sorted.iter().rev()).enumerate() {
        let z_lo = (x_lo - moments.mean) / sd;
        let z_hi = (x_hi - moments.mean) / sd;
        let p_lo = norm_cdf(z_lo).clamp(1e-16, 1.0 - 1e-16);
        let q_hi = norm_sf(z_hi).clamp(1e-16, 1.0 - 1e-16);
        sum_acc.add(((2 * i + 1) as f64) * (p_lo.ln() + q_hi.ln()));
    }
    let a2 = (-nf - sum_acc.total() / nf).max(0.0);
    let a2_star = a2 * (1.0 + 0.75 / nf + 2.25 / (nf * nf));
    let p_value = anderson_darling_p_value(a2_star);
    Ok(NormalityResult { statistic: a2_star, p_value })
}

/// Assign 1-based average mid-ranks in `O(n log n)` with tie averaging.
///
/// Returns a vector aligned 1-to-1 with `values`, where tied observations (`a == b`) share the
/// arithmetic mean of the 1-based rank positions they span. Empty inputs return an empty vector.
/// Sorting uses IEEE-754 [`f64::total_cmp`] so `-0.0` and `+0.0` are grouped as ties and
/// non-finite values (`NaN`, `±Inf`) never panic.
///
/// # Reference
/// - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.), §8.10 / §19.9. Pearson.
#[must_use]
pub fn average_ranks(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    let mut indexed: Vec<(usize, f64)> = values.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut ranks = vec![0.0_f64; n];
    let mut i = 0_usize;
    while let Some(&(_, base_val)) = indexed.get(i) {
        let mut j = i + 1;
        while indexed.get(j).is_some_and(|&(_, v)| v == base_val) {
            j += 1;
        }
        let avg_rank = 0.5 * ((i + 1 + j) as f64);
        for &(orig_idx, _) in indexed.get(i..j).unwrap_or(&[]) {
            if let Some(slot) = ranks.get_mut(orig_idx) {
                *slot = avg_rank;
            }
        }
        i = j;
    }
    ranks
}

#[cfg(test)]
mod tests {
    use super::{anderson_darling, average_ranks, dagostino_pearson, shapiro_wilk};
    use crate::error::MathError;

    fn assert_close(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, label: &str) {
        assert!(actual.is_finite(), "{label}: expected finite near {expected}, got {actual}");
        let diff = (actual - expected).abs();
        let bound = abs_tol.max(rel_tol * expected.abs());
        assert!(diff <= bound, "{label}: actual={actual:.16e}, expected={expected:.16e}, diff={diff:.3e} > bound={bound:.3e}");
    }

    #[test]
    fn shapiro_wilk_matches_royston_as_r94_oracles() {
        // D1 Provenance: Shapiro & Wilk (1965) / Royston (1992, 1995 AS R94); cross-checked
        // against published AS R94 reference values on n = 3..6 and n = 12+.
        let oracles: &[(&[f64], f64, f64, &str)] = &[
            (&[1.0, 2.0, 3.0], 1.0, 1.0, "n=3 symmetric"),
            (&[1.0, 1.0, 2.0], 0.75, 0.0, "n=3 extreme"),
            (&[1.0, 1.5, 10.0], 0.791_530_944_625_407_1, 0.094_436_484_202_600_3, "n=3 skewed"),
            (&[1.0, 2.0, 3.0, 4.0], 0.992_912_006_800_619_5, 0.971_877_057_620_898_6, "n=4 uniform"),
            (&[1.0, 2.0, 3.0, 10.0], 0.806_885_664_567_700_2, 0.115_152_981_754_536_6, "n=4 skewed"),
            (&[1.0, 1.0, 1.0, 10.0], 0.629_776_264_913_707_5, 0.001_240_725_931_977_3, "n=4 outlier"),
            (&[1.0, 2.0, 3.0, 4.0, 5.0], 0.986_762_155_447_719_4, 0.967_173_935_968_039_8, "n=5 uniform"),
            (&[1.0, 2.0, 3.0, 4.0, 20.0], 0.687_579_225_298_494, 0.007_068_598_540_515_0, "n=5 skewed"),
            (&[1.0, 2.0, 3.0, 4.0, 5.0, 15.0], 0.758_802_835_602_816_8, 0.024_258_247_776_970_1, "n=6 skewed"),
        ];
        for &(sample, exp_w, exp_p, label) in oracles {
            let res = shapiro_wilk(sample).unwrap();
            assert_close(res.statistic, exp_w, 1e-11, 1e-11, &format!("{label} W"));
            assert_close(res.p_value, exp_p, 1e-9, 1e-9, &format!("{label} p"));
        }

        // n >= 12 branch: symmetric normal quantile sample should pass cleanly.
        let normal_15 = [-1.96, -1.45, -1.12, -0.84, -0.58, -0.35, -0.15, 0.0, 0.15, 0.35, 0.58, 0.84, 1.12, 1.45, 1.96];
        let res15 = shapiro_wilk(&normal_15).unwrap();
        assert!(res15.statistic > 0.99 && res15.p_value > 0.9);
    }

    #[test]
    fn dagostino_pearson_normal_skewed_and_platykurtic_edge_cases() {
        let normal_like = [-1.8, -1.3, -0.9, -0.6, -0.3, -0.1, 0.1, 0.3, 0.6, 0.9, 1.3, 1.8];
        let dp_norm = dagostino_pearson(&normal_like).unwrap();
        assert!(dp_norm.statistic >= 0.0 && dp_norm.statistic < 1.0);
        assert!(dp_norm.p_value > 0.5);

        let heavily_skewed = [1.0, 1.0, 1.1, 1.2, 1.2, 1.3, 1.4, 1.5, 1.6, 50.0];
        let dp_skew = dagostino_pearson(&heavily_skewed).unwrap();
        assert!(dp_skew.statistic > 15.0);
        assert!(dp_skew.p_value < 1e-3);

        // Extreme platykurtic large-n sample where unclamped Anscombe-Glynn denominator <= 0.
        let mut bimodal = vec![-1.0_f64; 500];
        bimodal.extend(std::iter::repeat_n(1.0_f64, 500));
        let dp_bimodal = dagostino_pearson(&bimodal).unwrap();
        assert!(dp_bimodal.statistic.is_finite() && dp_bimodal.statistic > 100.0);
        assert!(dp_bimodal.p_value < 1e-10);
    }

    #[test]
    fn anderson_darling_normal_skewed_and_monotone_large_a2_star() {
        let normal_like = [-1.8, -1.3, -0.9, -0.6, -0.3, -0.1, 0.1, 0.3, 0.6, 0.9, 1.3, 1.8];
        let ad_norm = anderson_darling(&normal_like).unwrap();
        assert!(ad_norm.statistic < 0.25);
        assert!(ad_norm.p_value > 0.7);

        // Extreme non-normal sample with A*^2 > quadratic vertex (~153.5) stays monotone near 0.
        let mut sparse = vec![0.0_f64; 1000];
        sparse[0] = 1000.0;
        let ad_sparse = anderson_darling(&sparse).unwrap();
        assert!(ad_sparse.statistic > 200.0);
        assert!(ad_sparse.p_value >= 0.0 && ad_sparse.p_value < 1e-100);
    }

    #[test]
    fn normality_tests_reject_degenerate_and_out_of_domain_inputs() {
        assert_eq!(shapiro_wilk(&[]), Err(MathError::EmptyInput));
        assert!(matches!(shapiro_wilk(&[1.0, 2.0]), Err(MathError::InvalidParameter { name: "n", .. })));
        assert!(matches!(shapiro_wilk(&[5.0; 4]), Err(MathError::DomainError { name: "variance", .. })));
        assert!(matches!(shapiro_wilk(&[1.0, f64::NAN, 3.0]), Err(MathError::DomainError { name: "x", .. })));
        assert!(matches!(shapiro_wilk(&[1.0, f64::INFINITY, 3.0]), Err(MathError::DomainError { name: "x", .. })));

        assert_eq!(dagostino_pearson(&[]), Err(MathError::EmptyInput));
        assert!(matches!(dagostino_pearson(&[1.0; 7]), Err(MathError::InvalidParameter { name: "n", .. })));
        assert!(matches!(dagostino_pearson(&[3.0; 8]), Err(MathError::DomainError { name: "variance", .. })));
        assert!(matches!(dagostino_pearson(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, f64::NAN]), Err(MathError::DomainError { name: "x", .. })));

        assert_eq!(anderson_darling(&[]), Err(MathError::EmptyInput));
        assert!(matches!(anderson_darling(&[1.0; 7]), Err(MathError::InvalidParameter { name: "n", .. })));
        assert!(matches!(anderson_darling(&[3.0; 8]), Err(MathError::DomainError { name: "sd", .. })));
        assert!(matches!(anderson_darling(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, f64::NEG_INFINITY]), Err(MathError::DomainError { name: "x", .. })));
    }

    #[test]
    fn average_ranks_handles_empty_ties_signed_zero_and_permutation() {
        assert!(average_ranks(&[]).is_empty());
        assert_eq!(average_ranks(&[42.0]), vec![1.0]);
        assert_eq!(average_ranks(&[30.0, 10.0, 20.0]), vec![3.0, 1.0, 2.0]);
        // Ties: 10 (rank 1), 20,20 (avg 2.5), 30,30,30 (avg 5.0)
        assert_eq!(average_ranks(&[20.0, 30.0, 10.0, 30.0, 20.0, 30.0]), vec![2.5, 5.0, 1.0, 5.0, 2.5, 5.0]);
        // Signed zeros are numerically equal and share mid-rank 1.5.
        assert_eq!(average_ranks(&[0.0, -0.0, 1.0]), vec![1.5, 1.5, 3.0]);
    }

    #[test]
    fn dagostino_pearson_is_scale_invariant_across_extreme_exponents() {
        // DA-01-01 regression: standardized moments stay accurate and finite across 1e-200..1e200.
        let normal_base = [-1.8, -1.3, -0.9, -0.6, -0.3, -0.1, 0.1, 0.3, 0.6, 0.9, 1.3, 1.8];
        let ref_norm = dagostino_pearson(&normal_base).unwrap();
        for scale in [1e-200_f64, 1e-90, 1e80, 1e160, 1e200] {
            let scaled: Vec<f64> = normal_base.iter().map(|&x| x * scale).collect();
            let res = dagostino_pearson(&scaled).unwrap();
            assert_close(res.statistic, ref_norm.statistic, 1e-10, 1e-10, &format!("DP normal K^2 @ {scale:e}"));
            assert_close(res.p_value, ref_norm.p_value, 1e-10, 1e-10, &format!("DP normal p @ {scale:e}"));
        }

        // Extreme non-normal outlier at 1e160 must reject normality instead of masking NaN to K^2=0, p=1.
        let outlier_base = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let ref_out = dagostino_pearson(&outlier_base).unwrap();
        let outlier_huge = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1e160];
        let res_huge = dagostino_pearson(&outlier_huge).unwrap();
        assert_close(res_huge.statistic, ref_out.statistic, 1e-10, 1e-10, "DP outlier 1e160 K^2");
        assert_close(res_huge.p_value, ref_out.p_value, 1e-10, 1e-10, "DP outlier 1e160 p");
        assert!(res_huge.statistic > 10.0 && res_huge.p_value < 0.01);

        // Overflow-safe diff fallback when x - mean overflows f64::MAX.
        let extreme_span = [-1e308, -1e308, 1e308, 1e308, 1e308, 1e308, 1e308, 1e308];
        let unit_span = [-1.0, -1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let res_ext = dagostino_pearson(&extreme_span).unwrap();
        let res_unit = dagostino_pearson(&unit_span).unwrap();
        assert_close(res_ext.statistic, res_unit.statistic, 1e-10, 1e-10, "DP extreme span K^2");
        assert_close(res_ext.p_value, res_unit.p_value, 1e-10, 1e-10, "DP extreme span p");
    }

    #[test]
    fn shapiro_wilk_is_scale_invariant_across_extreme_exponents() {
        // DA-01-02 regression: standardized numerator avoids css overflow/underflow and NaN masking.
        let skewed_base = [0.0, 0.0, 0.0, 1.0];
        let ref_skew = shapiro_wilk(&skewed_base).unwrap();
        for scale in [1e-200_f64, 1e-160, 1e160, 1e200] {
            let scaled: Vec<f64> = skewed_base.iter().map(|&x| x * scale).collect();
            let res = shapiro_wilk(&scaled).unwrap();
            assert_close(res.statistic, ref_skew.statistic, 1e-11, 1e-11, &format!("SW skewed W @ {scale:e}"));
            assert_close(res.p_value, ref_skew.p_value, 1e-9, 1e-9, &format!("SW skewed p @ {scale:e}"));
        }

        // Tiny scale where unscaled css underflows to 0.0 must still compute valid W and p.
        let tiny_sym = [1e-200, 2e-200, 3e-200];
        let res_tiny = shapiro_wilk(&tiny_sym).unwrap();
        assert_close(res_tiny.statistic, 1.0, 1e-12, 1e-12, "SW tiny symmetric W");
        assert_close(res_tiny.p_value, 1.0, 1e-12, 1e-12, "SW tiny symmetric p");

        // Overflow-safe x_hi - x_lo fallback when difference overflows f64::MAX.
        let huge_span = [-1e308, 0.0, 1e308];
        let res_span = shapiro_wilk(&huge_span).unwrap();
        assert_close(res_span.statistic, 1.0, 1e-12, 1e-12, "SW huge span W");
        assert_close(res_span.p_value, 1.0, 1e-12, 1e-12, "SW huge span p");
    }
}
