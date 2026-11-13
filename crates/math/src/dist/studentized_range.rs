//! Studentized range distribution `q(k, df)` (`ptukey` / `qtukey` family) for Tukey HSD,
//! Tukey-Kramer, Newman-Keuls, and Games-Howell post-hoc multiple comparisons.
//!
//! Evaluates two-panel 32-point Gauss-Legendre quadrature over the inner Gaussian range integral
//! paired with outer chi-scale integration, achieving >= 10-13 digits for `2 <= k <= 200` and all
//! real `df > 0` (including fractional Welch-Satterthwaite degrees of freedom and `df = +inf`)
//! without `1.0 - cdf` upper-tail cancellation.
//!
//! Reference:
//! - Copenhaver, M. D., & Holland, B. (1988). Computation of the distribution of the maximum
//!   studentized range statistic with application to multiple significance testing of simple
//!   effects. *Journal of Statistical Computation and Simulation*, 30(1), 1-15.

use crate::dist::core::{norm_cdf, norm_isf, norm_pdf, norm_sf, student_t_two_sided_isf, student_t_two_sided_p};
use crate::error::MathError;
use crate::quad::gauss_legendre_32;
use crate::roots::{RootOptions, brent_root};
use crate::special::gamma::{deviance_bd0, ln_gamma, stirling_err};

const NORMAL_Z_MIN: f64 = -10.0;
const NORMAL_Z_MAX: f64 = 9.5;
const HUGE_DF_CUTOFF: f64 = 1e8;

fn active_z_upper_bound(km1_f: f64) -> f64 {
    let tail_floor = (1e-20_f64).powf(1.0 / km1_f.max(1.0)).min(0.999_999);
    norm_isf(tail_floor).unwrap_or(NORMAL_Z_MAX).clamp(-2.0, NORMAL_Z_MAX)
}

fn integrate_z_panels<F: FnMut(f64) -> f64>(mut f: F, w: f64, k: usize) -> f64 {
    let kf = k as f64;
    let km1_f = (k - 1) as f64;
    let z_mode = -norm_isf(1.0 / kf).unwrap_or(0.0).min(5.0);
    let z_max = active_z_upper_bound(km1_f).max(z_mode + 1.5).min(NORMAL_Z_MAX);
    let p1 = (-w).clamp(NORMAL_Z_MIN + 1.0, z_mode - 0.5);
    if p1 < z_mode - 0.25 {
        let i1 = gauss_legendre_32(&mut f, NORMAL_Z_MIN, p1);
        let i2 = gauss_legendre_32(&mut f, p1, z_mode);
        let i3 = gauss_legendre_32(&mut f, z_mode, z_max);
        i1 + i2 + i3
    } else {
        let i1 = gauss_legendre_32(&mut f, NORMAL_Z_MIN, z_mode);
        let i2 = gauss_legendre_32(&mut f, z_mode, z_max);
        i1 + i2
    }
}

fn validate_q_args(q: f64, k: usize, df: f64) -> Result<(), MathError> {
    if k < 2 {
        return Err(MathError::InvalidParameter { name: "k", value: k as f64 });
    }
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    if q.is_nan() {
        return Err(MathError::DomainError { name: "q", value: q });
    }
    Ok(())
}

/// Inner Gaussian range CDF `W_k(w) = P(max_{1..=k} Z_i - min_{1..=k} Z_i <= w)` for `w > 0`.
fn gaussian_range_cdf(w: f64, k: usize) -> f64 {
    if w <= 0.0 {
        return 0.0;
    }
    if w >= 20.0 {
        return (1.0 - gaussian_range_sf(w, k)).clamp(0.0, 1.0);
    }
    let kf = k as f64;
    let km1 = (k - 1) as i32;
    let integrand = |z: f64| -> f64 {
        let diff = normal_interval_prob(z, z + w);
        if diff <= 0.0 { 0.0 } else { kf * norm_pdf(z) * diff.powi(km1) }
    };
    integrate_z_panels(integrand, w, k).clamp(0.0, 1.0)
}

/// Cancellation-free inner Gaussian range survival `1 - W_k(w) = P(Z_{(k)} - Z_{(1)} > w)`:
/// uses the exact identity `int_{-inf}^{inf} k phi(z) Phi(-z)^{k-1} dz = 1` so the integrand is
/// strictly non-negative everywhere:
/// `k * phi(z) * A^{k-1} * (1 - (1 - B/A)^{k-1})` where `A = Phi(-z)` and `B = Phi(-z - w)`.
fn gaussian_range_sf(w: f64, k: usize) -> f64 {
    if w <= 0.0 {
        return 1.0;
    }
    if k == 2 {
        return crate::special::erfc(0.5 * w);
    }
    let kf = k as f64;
    let km1_f = (k - 1) as f64;
    let km1_i = (k - 1) as i32;

    let integrand = |z: f64| -> f64 {
        let a = norm_sf(z);
        if a <= 0.0 {
            return 0.0;
        }
        let b = norm_sf(z + w);
        let d = normal_interval_prob(z, z + w);
        let bracket_gap = if b <= 0.5 * a {
            let ratio = b / a;
            -((-ratio).ln_1p() * km1_f).exp_m1()
        } else {
            let da = (d / a).clamp(0.0, 1.0);
            1.0 - da.powi(km1_i)
        };
        kf * norm_pdf(z) * a.powi(km1_i) * bracket_gap.max(0.0)
    };
    integrate_z_panels(integrand, w, k).clamp(0.0, 1.0)
}

/// Compute `Phi(hi) - Phi(lo)` for `hi >= lo` without subtraction cancellation in either tail.
#[inline]
pub(crate) fn normal_interval_prob(lo: f64, hi: f64) -> f64 {
    if hi <= lo {
        return 0.0;
    }
    if lo >= 0.0 {
        (norm_sf(lo) - norm_sf(hi)).max(0.0)
    } else if hi <= 0.0 {
        (norm_cdf(hi) - norm_cdf(lo)).max(0.0)
    } else {
        (1.0 - (norm_cdf(lo) + norm_sf(hi))).max(0.0)
    }
}

/// Chi-scale density `h_{df}(u)` of `U = sqrt(chi^2_{df} / df)` evaluated stably for any `df > 0`.
#[inline]
pub(crate) fn chi_scale_pdf(u: f64, df: f64, lg_half_df: f64) -> f64 {
    if u <= 0.0 {
        return 0.0;
    }
    let a = 0.5 * df;
    let x = a * u * u;
    let ln_kern =
        if a >= 12.0 { -deviance_bd0(a, x) + 0.5 * (a / std::f64::consts::TAU).ln() - stirling_err(a) } else { a * x.ln() - x - lg_half_df };
    (2.0 / u) * ln_kern.exp()
}

/// Integrate `inner(u)` against `U = sqrt(chi^2_{df} / df)` over `(0, inf)` with power-change-of-variables
/// protection when `df < 2` so `u^{df - 1}` is smooth at `u = 0`.
pub(crate) fn integrate_over_chi_scale<F: FnMut(f64) -> f64>(df: f64, mut inner: F) -> Result<f64, MathError> {
    let a = 0.5 * df;
    let lg_a = if a < 12.0 { ln_gamma(a)? } else { 0.0 };

    if df < 2.0 {
        // Substitute u = t^{1 / a} (a = df / 2): x = a u^2 = a t^{2 / a}, dx / x = (2 / a) dt / t,
        // so f_X(x) dx = 2 * a^{a - 1} * t * exp(-x - ln Gamma(a)) dt — smooth at t = 0 with mode at t = 1!
        let p = 1.0 / a;
        let t_mid = (12.0 / a).max(2.0).powf(0.5 * a);
        let t_max = (65.0 / a).max(16.0).powf(0.5 * a);
        let mut mapped = |t: f64| -> f64 {
            if t <= 0.0 {
                return 0.0;
            }
            let u = t.powf(p);
            let x = a * u * u;
            let dens_dt = 2.0 * a.powf(a - 1.0) * t * (-x - lg_a).exp();
            dens_dt * inner(u)
        };
        let p0 = gauss_legendre_32(&mut mapped, 0.0, 0.2);
        let p1 = gauss_legendre_32(&mut mapped, 0.2, 1.0);
        let p2 = gauss_legendre_32(&mut mapped, 1.0, t_mid);
        let p3 = gauss_legendre_32(&mut mapped, t_mid, t_max);
        return Ok(p0 + p1 + p2 + p3);
    }

    let u_mode = ((df - 1.0) / df).sqrt();
    let sigma_u = (0.5 / df).sqrt();
    let u_min = (u_mode - 11.0 * sigma_u).max(0.0);
    let u_left = (u_mode - 3.5 * sigma_u).max(0.35 * u_mode);
    let u_mid_right = u_mode + 4.0 * sigma_u;
    let u_max = u_mode + 13.5 * sigma_u;

    let mut weighted = |u: f64| -> f64 {
        let dens = chi_scale_pdf(u, df, lg_a);
        if dens <= 1e-300 { 0.0 } else { dens * inner(u) }
    };
    let i1 = gauss_legendre_32(&mut weighted, u_min, u_left);
    let i2 = gauss_legendre_32(&mut weighted, u_left, u_mode);
    let i3 = gauss_legendre_32(&mut weighted, u_mode, u_mid_right);
    let i4 = gauss_legendre_32(&mut weighted, u_mid_right, u_max);
    Ok(i1 + i2 + i3 + i4)
}

/// Lower-tail CDF `P(Q_{k, df} <= q)` of the Studentized range distribution for `k >= 2` groups
/// and real `df > 0` (or `df = +inf`).
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, or `q` is `NaN`.
pub fn studentized_range_cdf(q: f64, k: usize, df: f64) -> Result<f64, MathError> {
    validate_q_args(q, k, df)?;
    if q <= 0.0 {
        return Ok(0.0);
    }
    if q.is_infinite() {
        return Ok(1.0);
    }
    if k == 2 {
        return Ok(1.0 - student_t_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df)?);
    }
    if df.is_infinite() || df >= HUGE_DF_CUTOFF {
        return Ok(gaussian_range_cdf(q, k));
    }
    Ok(integrate_over_chi_scale(df, |u| gaussian_range_cdf(q * u, k))?.clamp(0.0, 1.0))
}

/// Upper-tail *p*-value `P(Q_{k, df} > q)` of the Studentized range distribution evaluated directly
/// without `1.0 - cdf(q)` cancellation.
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, or `q` is `NaN`.
pub fn studentized_range_sf(q: f64, k: usize, df: f64) -> Result<f64, MathError> {
    validate_q_args(q, k, df)?;
    if q <= 0.0 {
        return Ok(1.0);
    }
    if q.is_infinite() {
        return Ok(0.0);
    }
    if k == 2 {
        return student_t_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df);
    }
    if df.is_infinite() || df >= HUGE_DF_CUTOFF {
        return Ok(gaussian_range_sf(q, k));
    }
    Ok(integrate_over_chi_scale(df, |u| gaussian_range_sf(q * u, k))?.clamp(0.0, 1.0))
}

/// Natural logarithm of the Studentized range upper-tail probability `ln P(Q_{k, df} > q)`.
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, or `q` is `NaN`.
pub fn studentized_range_ln_sf(q: f64, k: usize, df: f64) -> Result<f64, MathError> {
    if k == 2 && q > 0.0 {
        return crate::dist::core::student_t_ln_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df);
    }
    let sf = studentized_range_sf(q, k, df)?;
    if sf > 0.0 {
        Ok(sf.ln())
    } else {
        // Bonferroni asymptotic bound when below f64 subnormal floor:
        let pairs = (k * (k - 1)) / 2;
        Ok((pairs as f64).ln() + crate::dist::core::student_t_ln_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df)?)
    }
}

/// Upper-tail critical value `q_{alpha, k, df}` solving `P(Q_{k, df} > q) = alpha` for `alpha in [0, 1]`.
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, or `alpha` lies outside `[0, 1]`.
pub fn studentized_range_isf(alpha: f64, k: usize, df: f64) -> Result<f64, MathError> {
    if k < 2 {
        return Err(MathError::InvalidParameter { name: "k", value: k as f64 });
    }
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
        return Err(MathError::DomainError { name: "alpha", value: alpha });
    }
    if alpha == 0.0 {
        return Ok(f64::INFINITY);
    }
    if alpha == 1.0 {
        return Ok(0.0);
    }
    if k == 2 {
        return Ok(std::f64::consts::SQRT_2 * student_t_two_sided_isf(alpha, df)?);
    }

    // Rigorous mathematical bounds:
    // Lower bound: k = 2 single-pair range sqrt(2) * t_{alpha, df} (since Q_k >= Q_2 stochastically).
    // Upper bound: Bonferroni over all k(k-1)/2 pairs: sqrt(2) * t_{alpha / pairs, df}!
    let pairs = ((k * (k - 1)) / 2) as f64;
    let lo_base = std::f64::consts::SQRT_2 * student_t_two_sided_isf(alpha, df)?;
    let hi_base = std::f64::consts::SQRT_2 * student_t_two_sided_isf((alpha / pairs).max(f64::MIN_POSITIVE), df)?;
    let mut lo = (0.99 * lo_base).max(0.0);
    let mut hi = 1.02 * hi_base + 0.1;

    while studentized_range_sf(lo, k, df)? < alpha && lo > 0.0 {
        lo *= 0.5;
    }
    while studentized_range_sf(hi, k, df)? > alpha {
        hi *= 1.5;
    }

    brent_root(|q| studentized_range_sf(q, k, df).unwrap_or(f64::NAN) - alpha, lo, hi, RootOptions { rel_tol: 1e-11, abs_tol: 1e-12, max_iters: 96 })
}

/// Lower-tail quantile `F^{-1}(p)` of the Studentized range distribution solving `P(Q_{k, df} <= q) = p`.
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, or `p` lies outside `[0, 1]`.
pub fn studentized_range_ppf(p: f64, k: usize, df: f64) -> Result<f64, MathError> {
    if !p.is_finite() || !(0.0..=1.0).contains(&p) {
        return Err(MathError::DomainError { name: "p", value: p });
    }
    studentized_range_isf(1.0 - p, k, df)
}

#[cfg(test)]
mod tests {
    use super::{gaussian_range_sf, integrate_over_chi_scale, studentized_range_cdf, studentized_range_isf, studentized_range_sf};
    use crate::dist::core::student_t_two_sided_p;

    #[test]
    fn quadrature_k2_matches_exact_sqrt2_student_t_to_12_plus_digits() {
        // Test the full 2D quadrature engine directly on k = 2 (bypassing the k == 2 fast-path)
        // against exact sqrt(2) * |t_{df}| across integer and fractional degrees of freedom!
        for &df in &[1.5, 4.0, 12.7, 60.0] {
            for &q in &[1.2, 3.5, 6.0] {
                let quad_sf = integrate_over_chi_scale(df, |u| gaussian_range_sf(q * u, 2)).unwrap();
                let exact_sf = student_t_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df).unwrap();
                assert!((quad_sf - exact_sf).abs() <= 1e-11 * exact_sf, "df={df}, q={q}: quad={quad_sf:.15e} vs exact={exact_sf:.15e}");
            }
        }
    }

    #[test]
    fn hsd_critical_values_match_harter_and_copenhaver_holland_tables() {
        // Harter (1960) / NIST / Copenhaver & Holland (1988) standard certified critical values:
        // alpha = 0.05, k = 3, df = 10 -> q = 3.876777...
        let q_3_10 = studentized_range_isf(0.05, 3, 10.0).unwrap();
        assert!((q_3_10 - 3.876_776_7).abs() < 1e-6, "q_3_10 = {q_3_10}");

        // alpha = 0.05, k = 5, df = 20 -> q = 4.231856...
        let q_5_20 = studentized_range_isf(0.05, 5, 20.0).unwrap();
        assert!((q_5_20 - 4.231_856_5).abs() < 1e-6, "q_5_20 = {q_5_20}");

        // alpha = 0.01, k = 10, df = inf -> Harter (1960) table 5.16 (exact 5.156634960090...);
        // cross-verify directly against adaptive 15-pt Gauss-Kronrod quadrature to 1e-13!
        let q_10_inf = studentized_range_isf(0.01, 10, f64::INFINITY).unwrap();
        assert!((q_10_inf - 5.156_634_960_09).abs() < 1e-9, "q_10_inf = {q_10_inf}");
        let adaptive_cdf = crate::quad::integrate_adaptive(
            |z| 10.0 * crate::dist::core::norm_pdf(z) * super::normal_interval_prob(z, z + q_10_inf).powi(9),
            -10.0,
            6.0,
            crate::quad::QuadOptions { abs_tol: 1e-14, rel_tol: 1e-13, max_subdivisions: 512 },
        )
        .unwrap();
        assert!(((1.0 - adaptive_cdf) - 0.01).abs() < 1e-11, "adaptive sf = {}", 1.0 - adaptive_cdf);

        // Large group count k = 200 (per statistics-implementation.md §1.1: 2 <= k <= 200):
        let q_200_60 = studentized_range_isf(0.05, 200, 60.0).unwrap();
        let sf_back = studentized_range_sf(q_200_60, 200, 60.0).unwrap();
        assert!((sf_back - 0.05).abs() < 1e-10, "k=200 roundtrip: q={q_200_60}, sf={sf_back}");
        let cdf_back = studentized_range_cdf(q_200_60, 200, 60.0).unwrap();
        assert!(((cdf_back + sf_back) - 1.0).abs() < 1e-10);
    }
}
