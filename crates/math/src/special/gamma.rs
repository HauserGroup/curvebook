//! Gamma, log-gamma, Stirling remainder, beta, digamma, factorial, binomial, and Loader deviance
//! primitives in pure IEEE-754 `f64`.
//!
//! References:
//! - NIST Digital Library of Mathematical Functions, Ch. 5 (Gamma Function).
//! - Loader, C. (2000). Fast and accurate computation of binomial probabilities.
//! - DiDonato, A. R., & Morris, A. H. (1992). Algorithm 708: Significant digit computation of the
//!   incomplete beta function ratios. *ACM TOMS*, 18(3), 360-373.

use std::sync::OnceLock;

use crate::error::MathError;

const HALF_LN_2PI: f64 = 0.918_938_533_204_672_7;
const EULER_GAMMA: f64 = std::f64::consts::EULER_GAMMA;
const STIRLING_CUTOFF: f64 = 12.0;

/// Exact Bernoulli coefficients `B_{2k} / (2k * (2k - 1))` for `k = 1..=8` in Stirling's series.
const STIRLING_COEFFS: [f64; 8] =
    [1.0 / 12.0, -1.0 / 360.0, 1.0 / 1260.0, -1.0 / 1680.0, 1.0 / 1188.0, -691.0 / 360_360.0, 1.0 / 156.0, -3617.0 / 122_400.0];

/// Exact Bernoulli coefficients `B_{2k} / (2k)` for `k = 1..=8` in the asymptotic digamma series.
const DIGAMMA_COEFFS: [f64; 8] = [1.0 / 12.0, -1.0 / 120.0, 1.0 / 252.0, -1.0 / 240.0, 1.0 / 132.0, -691.0 / 32_760.0, 1.0 / 12.0, -3617.0 / 8160.0];

static ZETA_MINUS_ONE: OnceLock<[f64; 32]> = OnceLock::new();
static FACTORIAL_TABLE: OnceLock<[f64; 171]> = OnceLock::new();
static LN_FACTORIAL_TABLE: OnceLock<[f64; 171]> = OnceLock::new();

fn zeta_minus_one_table() -> &'static [f64; 32] {
    ZETA_MINUS_ONE.get_or_init(|| {
        let mut table = [0.0_f64; 32];
        for (idx, slot) in table.iter_mut().enumerate() {
            let s = (idx + 2) as i32;
            let sf = f64::from(s);
            // Direct sum n = 2..127 + Euler-Maclaurin tail at N = 128 (truncation < 1e-20).
            let mut sum = 0.0_f64;
            for n in (2..128_u32).rev() {
                sum += f64::from(n).powi(-s);
            }
            let n_tail = 128.0_f64;
            let n_pow = n_tail.powi(-s);
            let inv_n2 = 1.0 / (n_tail * n_tail);
            let em0 = n_tail * n_pow / (sf - 1.0) + 0.5 * n_pow;
            let em1 = (sf / 12.0) * (n_pow / n_tail);
            let em2 = -(sf * (sf + 1.0) * (sf + 2.0) / 720.0) * (n_pow / n_tail) * inv_n2;
            let em3 = (sf * (sf + 1.0) * (sf + 2.0) * (sf + 3.0) * (sf + 4.0) / 30_240.0) * (n_pow / n_tail) * inv_n2 * inv_n2;
            *slot = sum + em0 + em1 + em2 + em3;
        }
        table
    })
}

fn factorial_table() -> &'static [f64; 171] {
    FACTORIAL_TABLE.get_or_init(|| {
        let mut table = [1.0_f64; 171];
        let mut hi = 1.0_f64;
        let mut lo = 0.0_f64;
        for (n, slot) in table.iter_mut().enumerate().skip(2) {
            let kf = n as f64;
            let p = hi * kf;
            let err = hi.mul_add(kf, -p) + lo * kf;
            hi = p + err;
            lo = err - (hi - p);
            *slot = hi;
        }
        table
    })
}

fn ln_factorial_table() -> &'static [f64; 171] {
    LN_FACTORIAL_TABLE.get_or_init(|| {
        let mut table = [0.0_f64; 171];
        let mut hi = 1.0_f64;
        let mut lo = 0.0_f64;
        for (n, slot) in table.iter_mut().enumerate().skip(2) {
            let kf = n as f64;
            let p = hi * kf;
            let err = hi.mul_add(kf, -p) + lo * kf;
            hi = p + err;
            lo = err - (hi - p);
            *slot = hi.ln() + lo / hi;
        }
        table
    })
}

/// Stirling's remainder `delta(z) = ln Gamma(z) - (z - 0.5) ln(z) + z - 0.5 ln(2 pi)` for `z > 0`.
///
/// For `z >= 12`, evaluates the 8-term Bernoulli series directly with `< 1e-19` error, avoiding
/// catastrophic cancellation when forming ratios of large gamma functions.
#[must_use]
pub fn stirling_err(z: f64) -> f64 {
    if z >= STIRLING_CUTOFF {
        let inv_z = 1.0 / z;
        let inv_z2 = inv_z * inv_z;
        let mut poly = STIRLING_COEFFS[7];
        for &c in STIRLING_COEFFS[..7].iter().rev() {
            poly = poly.mul_add(inv_z2, c);
        }
        return poly * inv_z;
    }
    if let Ok(lg) = ln_gamma(z) { lg - (z - 0.5) * z.ln() + z - HALF_LN_2PI } else { f64::NAN }
}

/// Loader's deviance `bd0(x, np) = x * ln(x / np) + np - x >= 0` evaluated without cancellation
/// when `x` is close to `np`.
#[must_use]
pub fn deviance_bd0(x: f64, np: f64) -> f64 {
    if x == 0.0 {
        return np;
    }
    if (x - np).abs() < 0.1 * (x + np) {
        let v = (x - np) / (x + np);
        let v2 = v * v;
        let mut s = (x - np) * v;
        let mut ej = 2.0 * x * v * v2;
        for j in 1..40_u32 {
            let term = ej / f64::from(2 * j + 1);
            let next = s + term;
            if (next - s).abs() <= 0.5 * f64::EPSILON * s.abs() {
                return next.max(0.0);
            }
            s = next;
            ej *= v2;
        }
        return s.max(0.0);
    }
    (x * (x / np).ln() + np - x).max(0.0)
}

/// Evaluates `a * ln(1 + b / a) - b` without cancellation when `|b / a|` is small.
#[must_use]
pub fn aln1p_minus_arg(a: f64, b: f64) -> f64 {
    let u = b / a;
    if u.abs() < 0.2 {
        let mut sum = 0.0_f64;
        let mut pow = -u;
        for k in 1..36_u32 {
            let term = b * pow / f64::from(k + 1);
            let next = sum + term;
            if (next - sum).abs() <= 0.5 * f64::EPSILON * sum.abs().max(1e-300) {
                return next;
            }
            sum = next;
            pow *= -u;
        }
        return sum;
    }
    a * u.ln_1p() - b
}

fn ln_gamma_two_plus_u(u: f64) -> f64 {
    let zm1 = zeta_minus_one_table();
    let mut sum = (1.0 - EULER_GAMMA) * u;
    let mut pow = u * u;
    let mut sign = 1.0_f64;
    for (idx, &z_val) in zm1.iter().enumerate() {
        let k = (idx + 2) as f64;
        let term = sign * (z_val / k) * pow;
        sum += term;
        if term.abs() <= 0.25 * f64::EPSILON * sum.abs() {
            break;
        }
        pow *= u;
        sign = -sign;
    }
    sum
}

/// Natural logarithm of the absolute value of the Gamma function `ln |Gamma(x)|` for real `x > 0`.
///
/// # Errors
/// Returns [`MathError::DomainError`] if `x <= 0.0` or `x` is `NaN`.
pub fn ln_gamma(x: f64) -> Result<f64, MathError> {
    if !x.is_finite() {
        if x.is_infinite() && x.is_sign_positive() {
            return Ok(f64::INFINITY);
        }
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if x <= 0.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if x < 0.5 {
        // Gamma(x) = Gamma(2 + x) / (x * (1 + x)); all log terms avoid cancellation.
        return Ok(ln_gamma_two_plus_u(x) - x.ln_1p() - x.ln());
    }
    if x < 1.5 {
        // Near the zero at x = 1: Gamma(x) = Gamma(2 + (x - 1)) / (1 + (x - 1)).
        let u = x - 1.0;
        return Ok(ln_gamma_two_plus_u(u) - u.ln_1p());
    }
    if x <= 2.5 {
        return Ok(ln_gamma_two_plus_u(x - 2.0));
    }
    if x < STIRLING_CUTOFF {
        // Shift DOWN into [1.5, 2.5) by multiplying factors >= 1.5 so both terms are >= 0 (no cancellation!).
        let mut z = x;
        let mut prod = 1.0_f64;
        while z > 2.5 {
            z -= 1.0;
            prod *= z;
        }
        return Ok(ln_gamma_two_plus_u(z - 2.0) + prod.ln());
    }

    Ok((x - 0.5) * x.ln() - x + HALF_LN_2PI + stirling_err(x))
}

/// Euler's Gamma function `Gamma(x)` supporting all real `x` except non-positive integer poles.
///
/// # Errors
/// Returns [`MathError::DomainError`] at poles `x in {0, -1, -2, ...}`, `-inf`, or `NaN`.
pub fn gamma(x: f64) -> Result<f64, MathError> {
    if x.is_nan() || (!x.is_finite() && x < 0.0) {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if x <= 0.0 {
        if x.fract() == 0.0 {
            return Err(MathError::DomainError { name: "x", value: x });
        }
        // Euler reflection: Gamma(x) = pi / (sin(pi * x) * Gamma(1 - x)).
        let sin_pix = (std::f64::consts::PI * (x - 2.0 * (0.5 * x).round())).sin();
        if sin_pix == 0.0 {
            return Err(MathError::DomainError { name: "x", value: x });
        }
        let lg = ln_gamma(1.0 - x)?;
        return Ok(std::f64::consts::PI / (sin_pix * lg.exp()));
    }
    if x <= 171.0 && x.fract() == 0.0 {
        let n = (x as usize).saturating_sub(1);
        if let Some(&val) = factorial_table().get(n) {
            return Ok(val);
        }
    }
    Ok(ln_gamma(x)?.exp())
}

/// Natural logarithm of the Beta function `ln B(a, b) = ln Gamma(a) + ln Gamma(b) - ln Gamma(a + b)`
/// for `a > 0, b > 0`, using asymptotic Stirling corrections for large arguments to avoid cancellation.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `a <= 0` or `b <= 0` or either is non-finite.
pub fn ln_beta(a: f64, b: f64) -> Result<f64, MathError> {
    if !a.is_finite() || a <= 0.0 {
        return Err(MathError::InvalidParameter { name: "a", value: a });
    }
    if !b.is_finite() || b <= 0.0 {
        return Err(MathError::InvalidParameter { name: "b", value: b });
    }
    let (small, large) = if a <= b { (a, b) } else { (b, a) };
    if small >= STIRLING_CUTOFF {
        // Both large: exact Stirling decomposition with ln_1p.
        // (small - 0.5) * ln(small / (small + large)) + (large - 0.5) * ln(large) - large * ln(small + large)
        //   = (small - 0.5) * (ln(small / large) - ln1p(small / large)) - large * ln1p(small / large) - 0.5 * ln(large).
        let sum = small + large;
        let u = small / large;
        let corr = stirling_err(small) + stirling_err(large) - stirling_err(sum);
        let main = (small - 0.5) * (u.ln() - u.ln_1p()) - large * u.ln_1p() + HALF_LN_2PI - 0.5 * large.ln();
        return Ok(main + corr);
    }
    if large >= STIRLING_CUTOFF {
        // One small, one large: ln Gamma(small) - (ln Gamma(large + small) - ln Gamma(large)).
        let lg_small = ln_gamma(small)?;
        let ratio = small * large.ln() + aln1p_minus_arg(large, small) + (small - 0.5) * (small / large).ln_1p() + stirling_err(large + small)
            - stirling_err(large);
        return Ok(lg_small - ratio);
    }
    Ok(ln_gamma(a)? + ln_gamma(b)? - ln_gamma(a + b)?)
}

/// Euler's Beta function `B(a, b)` for `a > 0, b > 0`.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `a <= 0` or `b <= 0`.
pub fn beta(a: f64, b: f64) -> Result<f64, MathError> {
    Ok(ln_beta(a, b)?.exp())
}

/// Digamma (psi) function `psi(x) = d/dx ln Gamma(x)` for `x > 0` (and negative non-integers).
///
/// # Errors
/// Returns [`MathError::DomainError`] at poles `x in {0, -1, -2, ...}` or non-finite `x`.
pub fn digamma(x: f64) -> Result<f64, MathError> {
    if !x.is_finite() {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if x <= 0.0 {
        if x.fract() == 0.0 {
            return Err(MathError::DomainError { name: "x", value: x });
        }
        // Reflection: psi(1 - x) - psi(x) = pi * cot(pi * x).
        let reduced = x - (x).round();
        let tan_pix = (std::f64::consts::PI * reduced).tan();
        if tan_pix == 0.0 {
            return Err(MathError::DomainError { name: "x", value: x });
        }
        return Ok(digamma(1.0 - x)? - std::f64::consts::PI / tan_pix);
    }
    let mut z = x;
    let mut shift = 0.0_f64;
    while z < STIRLING_CUTOFF {
        shift -= 1.0 / z;
        z += 1.0;
    }
    let inv_z = 1.0 / z;
    let inv_z2 = inv_z * inv_z;
    let mut poly = DIGAMMA_COEFFS[7];
    for &c in DIGAMMA_COEFFS[..7].iter().rev() {
        poly = poly.mul_add(inv_z2, c);
    }
    Ok(shift + z.ln() - 0.5 * inv_z - poly * inv_z2)
}

/// Natural logarithm of `n!` for non-negative integer `n`.
#[must_use]
pub fn ln_factorial(n: u64) -> f64 {
    if n <= 1 {
        return 0.0;
    }
    if let Ok(idx) = usize::try_from(n)
        && let Some(&cached) = ln_factorial_table().get(idx)
    {
        return cached;
    }
    let z = (n as f64) + 1.0;
    (z - 0.5) * z.ln() - z + HALF_LN_2PI + stirling_err(z)
}

/// Natural logarithm of binomial coefficient `ln C(n, k)` for `0 <= k <= n`.
///
/// # Errors
/// Returns [`MathError::DomainError`] if `k > n`.
pub fn ln_choose(n: u64, k: u64) -> Result<f64, MathError> {
    if k > n {
        return Err(MathError::DomainError { name: "k", value: k as f64 });
    }
    let k_sym = k.min(n - k);
    if k_sym == 0 {
        return Ok(0.0);
    }
    if n < 171 {
        return Ok(ln_factorial(n) - ln_factorial(k_sym) - ln_factorial(n - k_sym));
    }
    // Loader deviance formulation for large n avoids subtracting O(n ln n) terms.
    let nf = n as f64;
    let kf = k_sym as f64;
    let m = n - k_sym;
    let mf = m as f64;
    if k_sym < 32 {
        let mut acc = 0.0_f64;
        for i in 0..k_sym {
            acc += ((n - i) as f64).ln() - ((i + 1) as f64).ln();
        }
        return Ok(acc);
    }
    // Because ln(r!) = ln(r * Gamma(r)) = (r + 0.5) * ln(r) - r + 0.5 * ln(2 pi) + stirling_err(r),
    // Loader's decomposition at (n, k, m = n - k) evaluates stirling_err at (nf, kf, mf), not +1.
    let corr = stirling_err(nf) - stirling_err(kf) - stirling_err(mf);
    let main = -HALF_LN_2PI + 0.5 * (nf / (kf * mf)).ln() - kf * (kf / nf).ln() - mf * (-kf / nf).ln_1p();
    Ok(main + corr)
}

#[cfg(test)]
mod tests {
    use super::{beta, digamma, gamma, ln_beta, ln_choose, ln_factorial, ln_gamma};

    #[test]
    fn ln_gamma_exact_at_half_and_integers_and_near_zeros() {
        assert_eq!(ln_gamma(1.0).unwrap(), 0.0);
        assert_eq!(ln_gamma(2.0).unwrap(), 0.0);
        let half = gamma(0.5).unwrap();
        assert!((half - std::f64::consts::PI.sqrt()).abs() < 1e-15);

        // Derivative at x = 1 is -gamma_euler; use binary-exact offset u = 2^-40 so 1.0 + u has zero rounding.
        let u = 2.0_f64.powi(-40);
        let tiny = ln_gamma(1.0 + u).unwrap();
        let expected_tiny = -std::f64::consts::EULER_GAMMA * u + (std::f64::consts::PI * std::f64::consts::PI / 12.0) * u * u;
        assert!((tiny - expected_tiny).abs() < 1e-27, "tiny={tiny:.17e} vs {expected_tiny:.17e}");

        let g10 = gamma(10.0).unwrap();
        assert!((g10 - 362_880.0).abs() < 1e-9);
        assert!((gamma(-0.5).unwrap() - (-2.0 * std::f64::consts::PI.sqrt())).abs() < 1e-14);
    }

    #[test]
    fn ln_beta_large_arguments_avoid_cancellation() {
        // B(0.5, 0.5) = pi
        assert!((beta(0.5, 0.5).unwrap() - std::f64::consts::PI).abs() < 1e-15);
        // Large imbalanced: ln B(1e8, 0.5) = ln Gamma(0.5) - (ln Gamma(1e8 + 0.5) - ln Gamma(1e8))
        // ~ 0.5 ln(pi) - 0.5 ln(1e8) = 0.5 * ln(pi / 1e8).
        let lb = ln_beta(1e8, 0.5).unwrap();
        let expected = 0.5 * (std::f64::consts::PI / 1e8).ln() + 1.0 / (8.0 * 1e8);
        assert!((lb - expected).abs() < 1e-14, "lb={lb}, expected={expected}");
    }

    #[test]
    fn digamma_and_ln_choose_identities() {
        // psi(1) = -Euler_gamma, psi(0.5) = -Euler_gamma - 2 ln 2
        assert!((digamma(1.0).unwrap() + std::f64::consts::EULER_GAMMA).abs() < 1e-15);
        assert!((digamma(0.5).unwrap() - (-std::f64::consts::EULER_GAMMA - 2.0 * 2.0_f64.ln())).abs() < 1e-15);
        assert!((ln_choose(10, 3).unwrap().exp() - 120.0).abs() < 1e-12);
        assert!((ln_factorial(20).exp() - 2_432_902_008_176_640_000.0).abs() / 2.43e18 < 1e-15);
    }
}
