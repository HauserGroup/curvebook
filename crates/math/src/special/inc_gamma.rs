//! Lower (`P(a, x)`) and upper (`Q(a, x)`) regularized incomplete gamma functions, their natural
//! logarithms (`ln_reg_gamma_p`, `ln_reg_gamma_q`), and tail-safe quantile inverses.
//!
//! Combines three complementary representations so no region suffers subtraction cancellation or
//! slow convergence:
//! 1. Strictly positive power series for `x < a + 1`,
//! 2. Legendre's continued fraction (modified Lentz) for `x >= a + 1`,
//! 3. Temme's (1979) uniform asymptotic `erfc` expansion for large `a` in the transition region `x ~ a`.
//!
//! References:
//! - Temme, N. M. (1979). The asymptotic expansion of the incomplete gamma functions.
//!   *SIAM Journal on Mathematical Analysis*, 10(4), 757-766.
//! - DiDonato, A. R., & Morris, A. H. (1986). Computation of the incomplete gamma function ratios
//!   and their inverse. *ACM TOMS*, 12(4), 377-393.
//! - NIST DLMF §8.7, §8.9, §8.11.

use crate::error::MathError;
use crate::roots::safeguarded_halley_step;
use crate::special::erf::{erfc, erfcx, exp_neg_sq, norm_isf_raw};
use crate::special::gamma::{deviance_bd0, ln_gamma, stirling_err};

const TEMME_A_THRESHOLD: f64 = 4096.0;
const MAX_GAMMA_ITERS: usize = 8192;

fn validate_gamma_args(a: f64, x: f64) -> Result<(), MathError> {
    if !a.is_finite() || a <= 0.0 {
        return Err(MathError::InvalidParameter { name: "a", value: a });
    }
    if x.is_nan() || x < 0.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    Ok(())
}

/// Cancellation-free evaluation of `ln(x^a * exp(-x) / Gamma(a))` for `a > 0, x > 0`.
fn ln_gamma_kernel(a: f64, x: f64) -> Result<f64, MathError> {
    if a >= 12.0 { Ok(-deviance_bd0(a, x) + 0.5 * (a / std::f64::consts::TAU).ln() - stirling_err(a)) } else { Ok(a * x.ln() - x - ln_gamma(a)?) }
}

/// Power series for `P(a, x) = (x^a e^{-x} / Gamma(a)) * sum_{k>=0} x^k / (a (a+1) ... (a+k))`
/// returning `ln(sum_{k>=0} x^k / (a (a+1) ... (a+k)))`.
fn ln_series_sum(a: f64, x: f64) -> Result<f64, MathError> {
    let mut ap = a;
    let mut term = 1.0 / a;
    let mut sum = term;
    for _ in 0..MAX_GAMMA_ITERS {
        ap += 1.0;
        term *= x / ap;
        let next = sum + term;
        if term <= 0.25 * f64::EPSILON * next {
            return Ok(next.ln());
        }
        sum = next;
    }
    Err(MathError::NotConverged { algorithm: "inc_gamma_series", iterations: MAX_GAMMA_ITERS })
}

/// Legendre continued fraction for `Q(a, x) = (x^a e^{-x} / Gamma(a)) / CF(a, x)` on `x >= a + 1`,
/// returning `ln(1 / CF(a, x)) = -ln(CF(a, x))`.
fn ln_legendre_cf_reciprocal(a: f64, x: f64) -> Result<f64, MathError> {
    let tiny = 1e-300_f64;
    let mut b = x + 1.0 - a;
    let mut f = b.max(tiny);
    let mut c = f;
    let mut d = 0.0_f64;

    for k in 1..=MAX_GAMMA_ITERS {
        let kf = k as f64;
        let an = -kf * (kf - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < tiny {
            d = tiny.copysign(d);
        }
        c = b + an / c;
        if c.abs() < tiny {
            c = tiny.copysign(c);
        }
        d = 1.0 / d;
        let delta = d * c;
        f *= delta;
        if (delta - 1.0).abs() <= 0.25 * f64::EPSILON {
            return Ok(-f.ln());
        }
    }
    Err(MathError::NotConverged { algorithm: "inc_gamma_cf", iterations: MAX_GAMMA_ITERS })
}

/// Temme's (1979) uniform asymptotic expansion `(P(a, x), Q(a, x))` for large `a` near `x ~ a`.
fn temme_uniform_pq(a: f64, x: f64) -> (f64, f64) {
    let lambda = x / a;
    let mu = lambda - 1.0;
    let half_eta2 = deviance_bd0(1.0, lambda);
    let eta = (2.0 * half_eta2).sqrt().copysign(mu);

    let (c0, c1, c2) = if eta.abs() < 0.02 {
        let e = eta;
        let c0 = -1.0 / 3.0 + e * (1.0 / 12.0 + e * (-2.0 / 135.0 + e * (1.0 / 864.0 + e / 2835.0)));
        let c1 = -1.0 / 540.0 + e * (-1.0 / 288.0 + e * (25.0 / 12_096.0));
        let c2 = 25.0 / 6048.0;
        (c0, c1, c2)
    } else {
        let inv_mu = 1.0 / mu;
        let inv_mu3 = inv_mu * inv_mu * inv_mu;
        let inv_mu5 = inv_mu3 * inv_mu * inv_mu;
        let inv_eta = 1.0 / eta;
        let inv_eta3 = inv_eta * inv_eta * inv_eta;
        let inv_eta5 = inv_eta3 * inv_eta * inv_eta;
        let c0 = inv_mu - inv_eta;
        let c1 = inv_eta3 - lambda * inv_mu3 - inv_mu / 12.0;
        let c2 = -3.0 * inv_eta5 + lambda * (2.0 * lambda + 1.0) * inv_mu5 + lambda * inv_mu3 / 12.0 + inv_mu / 288.0;
        (c0, c1, c2)
    };

    let inv_a = 1.0 / a;
    let series = c0 + inv_a * (c1 + inv_a * c2);
    let z = eta * (0.5 * a).sqrt();
    let r_a = exp_neg_sq(z) * series / (std::f64::consts::TAU * a).sqrt();
    let q = (0.5 * erfc(z) + r_a).clamp(0.0, 1.0);
    let p = (0.5 * erfc(-z) - r_a).clamp(0.0, 1.0);
    (p, q)
}

fn use_temme(a: f64, x: f64) -> bool {
    a >= TEMME_A_THRESHOLD && (x - a).abs() <= 6.0 * a.sqrt()
}

/// Lower regularized incomplete gamma function `P(a, x) = gamma(a, x) / Gamma(a)` in `[0, 1]`.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `a <= 0`, [`MathError::DomainError`] if `x < 0` or
/// `NaN`, or [`MathError::NotConverged`] if an expansion exceeds its iteration cap.
pub fn reg_gamma_p(a: f64, x: f64) -> Result<f64, MathError> {
    validate_gamma_args(a, x)?;
    if x == 0.0 {
        return Ok(0.0);
    }
    if x.is_infinite() {
        return Ok(1.0);
    }
    if use_temme(a, x) {
        return Ok(temme_uniform_pq(a, x).0);
    }
    if x < a + 1.0 {
        let ln_p = ln_gamma_kernel(a, x)? + ln_series_sum(a, x)?;
        Ok(ln_p.exp().clamp(0.0, 1.0))
    } else {
        let ln_q = ln_gamma_kernel(a, x)? + ln_legendre_cf_reciprocal(a, x)?;
        Ok((1.0 - ln_q.exp()).clamp(0.0, 1.0))
    }
}

/// Upper regularized incomplete gamma function `Q(a, x) = Gamma(a, x) / Gamma(a)` in `[0, 1]`,
/// evaluated directly in the upper tail without `1.0 - P(a, x)` cancellation.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `a <= 0`, [`MathError::DomainError`] if `x < 0` or
/// `NaN`, or [`MathError::NotConverged`] if an expansion exceeds its iteration cap.
pub fn reg_gamma_q(a: f64, x: f64) -> Result<f64, MathError> {
    validate_gamma_args(a, x)?;
    if x == 0.0 {
        return Ok(1.0);
    }
    if x.is_infinite() {
        return Ok(0.0);
    }
    if use_temme(a, x) {
        return Ok(temme_uniform_pq(a, x).1);
    }
    if x >= a + 1.0 {
        let ln_q = ln_gamma_kernel(a, x)? + ln_legendre_cf_reciprocal(a, x)?;
        Ok(ln_q.exp().clamp(0.0, 1.0))
    } else {
        let ln_p = ln_gamma_kernel(a, x)? + ln_series_sum(a, x)?;
        Ok((1.0 - ln_p.exp()).clamp(0.0, 1.0))
    }
}

/// Natural logarithm of the lower regularized incomplete gamma `ln P(a, x)`, remaining finite even
/// when `P(a, x)` underflows `f64` for very small `x` or huge `a`.
///
/// # Errors
/// Returns an error on invalid `a` or `x`.
pub fn ln_reg_gamma_p(a: f64, x: f64) -> Result<f64, MathError> {
    validate_gamma_args(a, x)?;
    if x == 0.0 {
        return Ok(f64::NEG_INFINITY);
    }
    if x.is_infinite() {
        return Ok(0.0);
    }
    if use_temme(a, x) {
        let (p, q) = temme_uniform_pq(a, x);
        return Ok(if p >= 0.5 { (-q).ln_1p() } else { p.ln() });
    }
    if x < a + 1.0 {
        Ok((ln_gamma_kernel(a, x)? + ln_series_sum(a, x)?).min(0.0))
    } else {
        let ln_q = ln_gamma_kernel(a, x)? + ln_legendre_cf_reciprocal(a, x)?;
        Ok((-ln_q.exp()).ln_1p())
    }
}

/// Natural logarithm of the upper regularized incomplete gamma `ln Q(a, x)`, remaining finite and
/// accurate far past `f64` probability underflow (`x >> a`).
///
/// # Errors
/// Returns an error on invalid `a` or `x`.
pub fn ln_reg_gamma_q(a: f64, x: f64) -> Result<f64, MathError> {
    validate_gamma_args(a, x)?;
    if x == 0.0 {
        return Ok(0.0);
    }
    if x.is_infinite() {
        return Ok(f64::NEG_INFINITY);
    }
    if use_temme(a, x) {
        let (p, q) = temme_uniform_pq(a, x);
        if q > 0.0 && q < 0.5 {
            return Ok(q.ln());
        }
        if q >= 0.5 {
            return Ok((-p).ln_1p());
        }
        let lambda = x / a;
        let z = (2.0 * deviance_bd0(1.0, lambda)).sqrt().copysign(lambda - 1.0) * (0.5 * a).sqrt();
        return Ok(-std::f64::consts::LN_2 - z * z + erfcx(z).ln());
    }
    if x >= a + 1.0 {
        Ok((ln_gamma_kernel(a, x)? + ln_legendre_cf_reciprocal(a, x)?).min(0.0))
    } else {
        let ln_p = ln_gamma_kernel(a, x)? + ln_series_sum(a, x)?;
        Ok((-ln_p.exp()).ln_1p())
    }
}

/// Solve `P(a, x) = p` for `x >= 0` given `a > 0` and `p in [0, 1]`.
///
/// # Errors
/// Returns an error if `a <= 0` or `p` is outside `[0, 1]`.
pub fn reg_gamma_p_inv(a: f64, p: f64) -> Result<f64, MathError> {
    if !a.is_finite() || a <= 0.0 {
        return Err(MathError::InvalidParameter { name: "a", value: a });
    }
    if !p.is_finite() || !(0.0..=1.0).contains(&p) {
        return Err(MathError::DomainError { name: "p", value: p });
    }
    invert_reg_gamma_pair(a, p, 1.0 - p)
}

/// Solve `Q(a, x) = q` for `x >= 0` directly from upper-tail probability `q in [0, 1]` without
/// `1.0 - q` subtraction cancellation.
///
/// # Errors
/// Returns an error if `a <= 0` or `q` is outside `[0, 1]`.
pub fn reg_gamma_q_inv(a: f64, q: f64) -> Result<f64, MathError> {
    if !a.is_finite() || a <= 0.0 {
        return Err(MathError::InvalidParameter { name: "a", value: a });
    }
    if !q.is_finite() || !(0.0..=1.0).contains(&q) {
        return Err(MathError::DomainError { name: "q", value: q });
    }
    invert_reg_gamma_pair(a, 1.0 - q, q)
}

fn initial_gamma_quantile_guess(a: f64, p: f64, q: f64) -> Result<f64, MathError> {
    let lg_a1 = ln_gamma(a + 1.0)?;
    // Small-x asymptotic: P(a, x) ~ x^a / Gamma(a + 1).
    if p < 0.2 {
        let x_small = ((p.ln() + lg_a1) / a).exp();
        if x_small > 0.0 && x_small < 0.35 * (a + 1.0) {
            return Ok(x_small * (1.0 + x_small / (a + 1.0)));
        }
    }
    // Large-x asymptotic when a <= 1 and q is very small: Q(a, x) ~ x^{a-1} e^{-x} / Gamma(a).
    if a <= 1.2 && q < 1e-3 {
        let lg_a = lg_a1 - a.ln();
        let u = -q.ln() - lg_a;
        let x0 = (u + (a - 1.0) * u.max(1.0).ln()).max(1e-12);
        return Ok(x0);
    }
    // Wilson-Hilferty cube-root normal approximation using tail-safe z.
    let z = if q <= p { norm_isf_raw(q)? } else { -norm_isf_raw(p)? };
    let w = 1.0 - 1.0 / (9.0 * a) + z / (3.0 * a.sqrt());
    if w > 0.15 {
        return Ok((a * w * w * w).max(f64::MIN_POSITIVE));
    }
    if p <= q {
        Ok((((p.ln() + lg_a1) / a).exp()).max(f64::MIN_POSITIVE))
    } else {
        let lg_a = lg_a1 - a.ln();
        let u = (-q.ln() - lg_a).max(1.0);
        Ok(a + u + (a - 1.0) * u.ln())
    }
}

fn invert_reg_gamma_pair(a: f64, p: f64, q: f64) -> Result<f64, MathError> {
    if p == 0.0 {
        return Ok(0.0);
    }
    if q == 0.0 {
        return Ok(f64::INFINITY);
    }
    if p < 1e-8 {
        return invert_reg_gamma_deep_lower(a, p);
    }
    if q < 1e-8 {
        return invert_reg_gamma_deep_upper(a, q);
    }

    let mut x = initial_gamma_quantile_guess(a, p, q)?;
    let mut lo = 0.0_f64;
    let mut hi = (x * 8.0 + a + 64.0).max(16.0);
    for _ in 0..32 {
        if reg_gamma_q(a, hi)? < q {
            break;
        }
        lo = hi;
        hi *= 4.0;
    }

    let solve_upper = q < p;
    for _ in 0..32 {
        let fx = if solve_upper { reg_gamma_q(a, x)? - q } else { reg_gamma_p(a, x)? - p };
        if fx == 0.0 {
            return Ok(x);
        }
        let x_below_root = if solve_upper { fx > 0.0 } else { fx < 0.0 };
        if x_below_root {
            lo = lo.max(x);
        } else {
            hi = hi.min(x);
        }
        if (hi - lo) <= 2.0 * f64::EPSILON * x.max(f64::MIN_POSITIVE) {
            return Ok(0.5 * (lo + hi));
        }

        let ln_pdf = ln_gamma_kernel(a, x)? - x.ln();
        let pdf = ln_pdf.exp();
        let dfx = if solve_upper { -pdf } else { pdf };
        let d2_over_d1 = (a - 1.0) / x - 1.0;
        let next = safeguarded_halley_step(x, fx, dfx, d2_over_d1, lo, hi);
        if (next - x).abs() <= 1.5 * f64::EPSILON * next.max(f64::MIN_POSITIVE) {
            return Ok(next);
        }
        x = next;
    }
    Ok(0.5 * (lo + hi))
}

fn invert_reg_gamma_deep_upper(a: f64, q: f64) -> Result<f64, MathError> {
    let target_ln = q.ln();
    let mut x = initial_gamma_quantile_guess(a, 1.0 - q, q)?;
    let mut lo = 0.0_f64;
    let mut hi = (x * 4.0 + a + 64.0).max(16.0);
    for _ in 0..32 {
        if ln_reg_gamma_q(a, hi)? < target_ln {
            break;
        }
        lo = hi;
        hi *= 4.0;
    }

    for _ in 0..40 {
        let ln_q = ln_reg_gamma_q(a, x)?;
        let g = ln_q - target_ln;
        if g == 0.0 {
            return Ok(x);
        }
        if g > 0.0 {
            lo = lo.max(x);
        } else {
            hi = hi.min(x);
        }
        if (hi - lo) <= 2.0 * f64::EPSILON * x.max(f64::MIN_POSITIVE) {
            return Ok(0.5 * (lo + hi));
        }
        let ln_pdf = ln_gamma_kernel(a, x)? - x.ln();
        let g1 = -(ln_pdf - ln_q).exp();
        let d2_over_d1 = (a - 1.0) / x - 1.0 - g1;
        let next = safeguarded_halley_step(x, g, g1, d2_over_d1, lo, hi);
        if (next - x).abs() <= 1.5 * f64::EPSILON * next.max(f64::MIN_POSITIVE) {
            return Ok(next);
        }
        x = next;
    }
    Ok(0.5 * (lo + hi))
}

fn invert_reg_gamma_deep_lower(a: f64, p: f64) -> Result<f64, MathError> {
    let target_ln = p.ln();
    let x0 = initial_gamma_quantile_guess(a, p, 1.0 - p)?;
    let mut u = x0.ln();
    let mut u_lo = u - 16.0;
    let mut u_hi = (u + 16.0).max((a + 16.0).ln());
    while ln_reg_gamma_p(a, u_lo.exp())? > target_ln && u_lo > -740.0 {
        u_hi = u_lo;
        u_lo -= 16.0;
    }
    while ln_reg_gamma_p(a, u_hi.exp())? < target_ln {
        u_lo = u_hi;
        u_hi += 16.0;
    }

    for _ in 0..40 {
        let x = u.exp();
        if x == 0.0 {
            return Ok(0.0);
        }
        let ln_p = ln_reg_gamma_p(a, x)?;
        let h = ln_p - target_ln;
        if h == 0.0 {
            return Ok(x);
        }
        if h < 0.0 {
            u_lo = u_lo.max(u);
        } else {
            u_hi = u_hi.min(u);
        }
        if (u_hi - u_lo) <= 2.0 * f64::EPSILON * u.abs().max(1.0) {
            return Ok((0.5 * (u_lo + u_hi)).exp());
        }
        let h1 = (ln_gamma_kernel(a, x)? - ln_p).exp();
        let d2_over_d1 = a - x - h1;
        let next_u = safeguarded_halley_step(u, h, h1, d2_over_d1, u_lo, u_hi);
        if (next_u - u).abs() <= 1.5 * f64::EPSILON * next_u.abs().max(1.0) {
            return Ok(next_u.exp());
        }
        u = next_u;
    }
    Ok((0.5 * (u_lo + u_hi)).exp())
}

#[cfg(test)]
mod tests {
    use super::{ln_reg_gamma_q, reg_gamma_p, reg_gamma_p_inv, reg_gamma_q, reg_gamma_q_inv, temme_uniform_pq};

    #[test]
    fn exponential_and_half_integer_analytical_identities() {
        // For a = 1: P(1, x) = 1 - e^{-x}, Q(1, x) = e^{-x}.
        for &x in &[1e-10, 0.25, 1.0, 5.0, 50.0, 500.0] {
            let q = reg_gamma_q(1.0, x).unwrap();
            let expected = (-x).exp();
            assert!((q - expected).abs() <= 2e-15 * expected, "x={x}: {q} vs {expected}");
            let lq = ln_reg_gamma_q(1.0, x).unwrap();
            assert!((lq - (-x)).abs() <= 2e-14 * x);
        }
        // For a = 0.5: Q(0.5, x) = erfc(sqrt(x)).
        for &x in &[0.01, 0.5, 2.0, 20.0] {
            let q = reg_gamma_q(0.5, x).unwrap();
            let expected = crate::special::erf::erfc(x.sqrt());
            assert!((q - expected).abs() <= 5e-15 * expected.max(1e-300));
        }
    }

    #[test]
    fn temme_matches_series_and_cf_across_seam_and_ramanujan_at_x_eq_a() {
        // At a = 4096, compare Temme directly against series/CF at a = 4095.5!
        let (p_temme, q_temme) = temme_uniform_pq(4095.0, 4120.0);
        let q_cf = reg_gamma_q(4095.0, 4120.0).unwrap();
        let p_cf = reg_gamma_p(4095.0, 4120.0).unwrap();
        assert!((q_temme - q_cf).abs() < 5e-14, "q_temme={q_temme} vs q_cf={q_cf}");
        assert!((p_temme - p_cf).abs() < 5e-14, "p_temme={p_temme} vs p_cf={p_cf}");

        // Ramanujan DLMF 8.11.12 at x = a = 1e6: Q(a, a) = 0.5 - (1/sqrt(2 pi a))*(1/3 + 1/(540 a)).
        let a = 1e6;
        let q_diag = reg_gamma_q(a, a).unwrap();
        let ramanujan = 0.5 - (1.0 / (std::f64::consts::TAU * a).sqrt()) * (1.0 / 3.0 + 1.0 / (540.0 * a));
        assert!((q_diag - ramanujan).abs() < 1e-15, "q_diag={q_diag} vs {ramanujan}");
    }

    #[test]
    fn inverse_incomplete_gamma_roundtrips_lower_and_upper_tails() {
        for &a in &[0.5, 1.0, 2.5, 10.0, 150.0, 5000.0] {
            for &q in &[0.99, 0.5, 0.05, 1e-6, 1e-25, 1e-100] {
                let x = reg_gamma_q_inv(a, q).unwrap();
                let q_back = reg_gamma_q(a, x).unwrap();
                assert!((q_back - q).abs() <= 1e-12 * q, "a={a}, q={q}: x={x}, q_back={q_back}");
            }
            for &p in &[1e-12, 0.01, 0.5, 0.95] {
                let x = reg_gamma_p_inv(a, p).unwrap();
                let p_back = reg_gamma_p(a, x).unwrap();
                assert!((p_back - p).abs() <= 1e-12 * p, "a={a}, p={p}: x={x}, p_back={p_back}");
            }
        }
    }
}
